use crate::policy::AllowList;
use serde::{Deserialize, Serialize};
use std::fs::{self, OpenOptions};
use std::io::Write;
use std::os::unix::fs::{OpenOptionsExt, PermissionsExt};
use std::path::{Path, PathBuf};
use std::sync::Mutex;
use std::time::{Duration, Instant};

pub const POLL_MIN: u64 = 30;
pub const POLL_MAX: u64 = 86_400;
pub const SUMMARY_MIN: u64 = 300;
pub const SUMMARY_MAX: u64 = 604_800;
pub const RELOAD_MIN: u64 = 30;
pub const RELOAD_MAX: u64 = 3_600;

fn default_poll() -> u64 {
    300
}
fn default_summary() -> u64 {
    3600
}
fn default_reload() -> u64 {
    180
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct Settings {
    #[serde(default = "default_poll")]
    pub poll_interval_secs: u64,
    #[serde(default = "default_summary")]
    pub summary_cadence_secs: u64,
    #[serde(default = "default_reload")]
    pub settings_reload_secs: u64,
    #[serde(default)]
    pub sender_allowlist: Vec<String>,
    #[serde(default)]
    pub importance_keywords: Vec<String>,
    #[serde(default)]
    pub calendar_feeds: Vec<String>,
}

impl Default for Settings {
    fn default() -> Self {
        Self {
            poll_interval_secs: default_poll(),
            summary_cadence_secs: default_summary(),
            settings_reload_secs: default_reload(),
            sender_allowlist: Vec::new(),
            importance_keywords: Vec::new(),
            calendar_feeds: Vec::new(),
        }
    }
}

#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SettingsPatch {
    pub poll_interval_secs: Option<u64>,
    pub summary_cadence_secs: Option<u64>,
    pub settings_reload_secs: Option<u64>,
    pub sender_allowlist: Option<Vec<String>>,
    pub importance_keywords: Option<Vec<String>>,
    pub calendar_feeds: Option<Vec<String>>,
}

impl Settings {
    pub fn clamp(mut self) -> Result<Self, String> {
        self.poll_interval_secs = self.poll_interval_secs.clamp(POLL_MIN, POLL_MAX);
        self.summary_cadence_secs = self.summary_cadence_secs.clamp(SUMMARY_MIN, SUMMARY_MAX);
        self.settings_reload_secs = self.settings_reload_secs.clamp(RELOAD_MIN, RELOAD_MAX);
        self.sender_allowlist = AllowList::from_list(&self.sender_allowlist)?.as_strings();
        if self.sender_allowlist.len() > 64 {
            self.sender_allowlist.truncate(64);
        }
        self.importance_keywords = self
            .importance_keywords
            .into_iter()
            .map(|word| word.trim().to_string())
            .filter(|word| !word.is_empty())
            .map(|word| word.chars().take(64).collect::<String>())
            .take(32)
            .collect();
        for feed in &self.calendar_feeds {
            validate_feed_ref(feed)?;
        }
        if self.calendar_feeds.len() > 16 {
            self.calendar_feeds.truncate(16);
        }
        Ok(self)
    }

    pub fn from_toml(text: &str) -> Result<Self, String> {
        let parsed: Self = toml::from_str(text).map_err(|err| format!("settings: {err}"))?;
        parsed.clamp()
    }
}

pub fn validate_feed_ref(feed: &str) -> Result<(), String> {
    let feed = feed.trim();
    if feed.len() > 200 || feed.contains('\n') || feed.contains('\0') {
        return Err("calendar feed ref is invalid".into());
    }
    if feed.starts_with("http://") || feed.starts_with("https://") {
        return Err("calendar feed ref must be a vault/item name, not a URL".into());
    }
    let Some((vault, item)) = feed.split_once('/') else {
        return Err("calendar feed ref must look like vault/item".into());
    };
    if vault.is_empty()
        || item.is_empty()
        || vault.contains('/')
        || item.contains('/')
        || vault.contains("..")
        || item.contains("..")
    {
        return Err("calendar feed ref is invalid".into());
    }
    Ok(())
}

pub struct SettingsStore {
    path: PathBuf,
    inner: Mutex<Settings>,
    last_check: Mutex<Instant>,
}

impl SettingsStore {
    pub fn open(path: PathBuf) -> Result<Self, String> {
        let settings = if path.exists() {
            let text = fs::read_to_string(&path).map_err(|err| format!("settings read: {err}"))?;
            Settings::from_toml(&text)?
        } else {
            let settings = Settings::default();
            save_atomic(&path, &settings)?;
            settings
        };
        Ok(Self {
            path,
            inner: Mutex::new(settings),
            last_check: Mutex::new(Instant::now()),
        })
    }

    pub fn memory(settings: Settings) -> Self {
        Self {
            path: PathBuf::new(),
            inner: Mutex::new(settings),
            last_check: Mutex::new(Instant::now()),
        }
    }

    pub fn get(&self) -> Settings {
        self.inner.lock().unwrap_or_else(|err| err.into_inner()).clone()
    }

    pub fn reload_if_stale(&self) -> Result<(), String> {
        if self.path.as_os_str().is_empty() {
            return Ok(());
        }
        let interval = Duration::from_secs(self.get().settings_reload_secs);
        let mut last = self.last_check.lock().unwrap_or_else(|err| err.into_inner());
        if last.elapsed() < interval {
            return Ok(());
        }
        *last = Instant::now();
        drop(last);
        if !self.path.exists() {
            return Ok(());
        }
        let text = fs::read_to_string(&self.path).map_err(|err| format!("settings read: {err}"))?;
        let settings = Settings::from_toml(&text)?;
        *self.inner.lock().unwrap_or_else(|err| err.into_inner()) = settings;
        Ok(())
    }

    pub fn apply_patch(&self, patch: SettingsPatch) -> Result<Settings, String> {
        let mut next = self.get();
        if let Some(value) = patch.poll_interval_secs {
            next.poll_interval_secs = value;
        }
        if let Some(value) = patch.summary_cadence_secs {
            next.summary_cadence_secs = value;
        }
        if let Some(value) = patch.settings_reload_secs {
            next.settings_reload_secs = value;
        }
        if let Some(value) = patch.sender_allowlist {
            next.sender_allowlist = value;
        }
        if let Some(value) = patch.importance_keywords {
            next.importance_keywords = value;
        }
        if let Some(value) = patch.calendar_feeds {
            next.calendar_feeds = value;
        }
        let next = next.clamp()?;
        if !self.path.as_os_str().is_empty() {
            save_atomic(&self.path, &next)?;
        }
        *self.inner.lock().unwrap_or_else(|err| err.into_inner()) = next.clone();
        Ok(next)
    }
}

pub fn save_atomic(path: &Path, settings: &Settings) -> Result<(), String> {
    if let Some(parent) = path.parent() {
        if !parent.as_os_str().is_empty() {
            fs::create_dir_all(parent).map_err(|err| format!("settings dir: {err}"))?;
        }
    }
    let text = toml::to_string_pretty(settings).map_err(|err| format!("settings encode: {err}"))?;
    let tmp = path.with_extension("toml.tmp");
    {
        let mut file = OpenOptions::new()
            .write(true)
            .create(true)
            .truncate(true)
            .mode(0o600)
            .open(&tmp)
            .map_err(|err| format!("settings write: {err}"))?;
        file.write_all(text.as_bytes())
            .map_err(|err| format!("settings write: {err}"))?;
    }
    fs::rename(&tmp, path).map_err(|err| format!("settings rename: {err}"))?;
    let _ = fs::set_permissions(path, fs::Permissions::from_mode(0o600));
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn clamps_runaway_intervals() {
        let settings = Settings {
            poll_interval_secs: 1,
            summary_cadence_secs: 10,
            settings_reload_secs: 1,
            sender_allowlist: vec!["Owner@Proton.Me".into()],
            importance_keywords: vec!["  OTP  ".into(), "".into()],
            calendar_feeds: vec![],
        }
        .clamp()
        .unwrap();
        assert_eq!(settings.poll_interval_secs, POLL_MIN);
        assert_eq!(settings.summary_cadence_secs, SUMMARY_MIN);
        assert_eq!(settings.settings_reload_secs, RELOAD_MIN);
        assert_eq!(settings.sender_allowlist, vec!["owner@proton.me"]);
        assert_eq!(settings.importance_keywords, vec!["OTP"]);

        let high = Settings {
            poll_interval_secs: u64::MAX,
            summary_cadence_secs: u64::MAX,
            settings_reload_secs: u64::MAX,
            ..Settings::default()
        }
        .clamp()
        .unwrap();
        assert_eq!(high.poll_interval_secs, POLL_MAX);
        assert_eq!(high.summary_cadence_secs, SUMMARY_MAX);
        assert_eq!(high.settings_reload_secs, RELOAD_MAX);
    }

    #[test]
    fn rejects_url_in_feed_ref_and_unknown_field() {
        assert!(validate_feed_ref("https://cal.example/feed.ics").is_err());
        assert!(Settings::from_toml("poll_interval_secs = 1\nnope = true\n").is_err());
        let settings = Settings::from_toml("poll_interval_secs = 1\n").unwrap();
        assert_eq!(settings.poll_interval_secs, POLL_MIN);
    }
}
