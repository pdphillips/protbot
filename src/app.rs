use crate::cache::TtlCache;
use crate::cli::Runner;
use crate::policy::AllowList;
use crate::settings::{Settings, SettingsStore};
use crate::watcher::Notice;
use std::collections::VecDeque;
use std::fs;
use std::os::unix::fs::PermissionsExt;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};
use std::time::Duration;

pub struct Config {
    token: Option<String>,
    pub allow_writes: bool,
    pub recipients: AllowList,
    pub mail_bin: String,
    pub pass_bin: String,
    pub drive_bin: String,
    pub curl_bin: String,
    pub settings_path: PathBuf,
    pub tool_timeout: Duration,
    pub upload_dir: Option<PathBuf>,
    pub download_dir: Option<PathBuf>,
    pub calendar_ics: Vec<String>,
    pub session_dir: PathBuf,
    pub disable_watcher: bool,
}

impl std::fmt::Debug for Config {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("Config")
            .field("token_present", &self.token.is_some())
            .field("allow_writes", &self.allow_writes)
            .field("recipient_count", &self.recipients.as_strings().len())
            .field("calendar_feed_count", &self.calendar_ics.len())
            .field("disable_watcher", &self.disable_watcher)
            .finish()
    }
}

impl Drop for Config {
    fn drop(&mut self) {
        if let Some(token) = self.token.as_mut() {
            unsafe {
                for byte in token.as_bytes_mut() {
                    *byte = 0;
                }
            }
        }
        if !self.session_dir.as_os_str().is_empty() {
            let _ = fs::remove_dir_all(&self.session_dir);
        }
    }
}

impl Config {
    pub fn token(&self) -> Option<&str> {
        self.token.as_deref()
    }

    pub fn for_test(root: &Path) -> Self {
        Self {
            token: Some("pst_test::fixture".into()),
            allow_writes: false,
            recipients: AllowList::default(),
            mail_bin: "proton-mail".into(),
            pass_bin: "pass-cli".into(),
            drive_bin: "proton-drive".into(),
            curl_bin: "curl".into(),
            settings_path: root.join("settings.toml"),
            tool_timeout: Duration::from_secs(5),
            upload_dir: None,
            download_dir: None,
            calendar_ics: Vec::new(),
            session_dir: PathBuf::new(),
            disable_watcher: true,
        }
    }

    pub fn from_env() -> Result<Self, String> {
        if std::env::args().len() > 1 {
            return Err("this program takes no arguments".into());
        }
        let token = std::env::var("PROTON_PASS_AGENT_TOKEN")
            .ok()
            .map(|value| value.trim().to_string())
            .filter(|value| !value.is_empty());
        let recipients = AllowList::parse_csv(
            &std::env::var("PROTBOT_RECIPIENT_ALLOWLIST").unwrap_or_default(),
        )?;
        let mut calendar_ics = Vec::new();
        for part in std::env::var("PROTBOT_CALENDAR_ICS").unwrap_or_default().split(',') {
            let part = part.trim();
            if part.is_empty() {
                continue;
            }
            crate::calendar::validate_ics_url(part)?;
            calendar_ics.push(part.to_string());
        }
        let settings_path = std::env::var("PROTBOT_SETTINGS")
            .map(PathBuf::from)
            .unwrap_or_else(|_| default_settings_path());
        let session_dir = session_dir()?;
        Ok(Self {
            token,
            allow_writes: env_flag("ALLOW_WRITES"),
            recipients,
            mail_bin: env_or("PROTBOT_MAIL_BIN", "proton-mail"),
            pass_bin: env_or("PROTBOT_PASS_BIN", "pass-cli"),
            drive_bin: env_or("PROTBOT_DRIVE_BIN", "proton-drive"),
            curl_bin: env_or("PROTBOT_CURL_BIN", "curl"),
            settings_path,
            tool_timeout: Duration::from_secs(env_u64("PROTBOT_TOOL_TIMEOUT_SECS", 30, 5, 120)),
            upload_dir: env_path("PROTBOT_UPLOAD_DIR")?,
            download_dir: env_path("PROTBOT_DOWNLOAD_DIR")?,
            calendar_ics,
            session_dir,
            disable_watcher: env_flag("PROTBOT_DISABLE_WATCHER"),
        })
    }
}

fn env_or(name: &str, default: &str) -> String {
    std::env::var(name)
        .ok()
        .filter(|value| !value.trim().is_empty())
        .unwrap_or_else(|| default.to_string())
}

fn env_flag(name: &str) -> bool {
    matches!(
        std::env::var(name).unwrap_or_default().trim().to_ascii_lowercase().as_str(),
        "1" | "true" | "yes"
    )
}

fn env_u64(name: &str, default: u64, min: u64, max: u64) -> u64 {
    std::env::var(name)
        .ok()
        .and_then(|value| value.parse().ok())
        .unwrap_or(default)
        .clamp(min, max)
}

fn env_path(name: &str) -> Result<Option<PathBuf>, String> {
    let Ok(value) = std::env::var(name) else {
        return Ok(None);
    };
    if value.trim().is_empty() {
        return Ok(None);
    }
    Ok(Some(crate::policy::normalize_abs(value.trim())?))
}

fn default_settings_path() -> PathBuf {
    let home = std::env::var("HOME").unwrap_or_else(|_| "/tmp".into());
    let base = std::env::var("XDG_CONFIG_HOME").unwrap_or_else(|_| format!("{home}/.config"));
    PathBuf::from(base).join("protbot/settings.toml")
}

fn session_dir() -> Result<PathBuf, String> {
    let base = std::env::var("XDG_RUNTIME_DIR").unwrap_or_else(|_| "/dev/shm".into());
    let dir = PathBuf::from(base).join(format!("protbot-{}", std::process::id()));
    fs::create_dir_all(&dir).map_err(|err| format!("session dir: {err}"))?;
    fs::set_permissions(&dir, fs::Permissions::from_mode(0o700))
        .map_err(|err| format!("session dir: {err}"))?;
    Ok(dir)
}

struct Inner {
    cfg: Config,
    settings: SettingsStore,
    cache: Mutex<TtlCache>,
    runner: Arc<dyn Runner>,
    cli_lock: tokio::sync::Mutex<()>,
    pass_ready: Mutex<bool>,
    notices: Mutex<VecDeque<Notice>>,
}

#[derive(Clone)]
pub struct App {
    inner: Arc<Inner>,
}

impl App {
    pub fn from_env() -> Result<Self, String> {
        let cfg = Config::from_env()?;
        let settings = SettingsStore::open(cfg.settings_path.clone())?;
        Ok(Self::new(cfg, settings, Arc::new(crate::cli::ProcessRunner)))
    }

    pub fn for_test(cfg: Config, settings: Settings, runner: Arc<dyn Runner>) -> Self {
        Self::new(cfg, SettingsStore::memory(settings), runner)
    }

    fn new(cfg: Config, settings: SettingsStore, runner: Arc<dyn Runner>) -> Self {
        Self {
            inner: Arc::new(Inner {
                cfg,
                settings,
                cache: Mutex::new(TtlCache::new(Duration::from_secs(30))),
                runner,
                cli_lock: tokio::sync::Mutex::new(()),
                pass_ready: Mutex::new(false),
                notices: Mutex::new(VecDeque::new()),
            }),
        }
    }

    pub fn cfg(&self) -> &Config {
        &self.inner.cfg
    }

    pub fn settings(&self) -> &SettingsStore {
        &self.inner.settings
    }

    pub fn runner(&self) -> &dyn Runner {
        self.inner.runner.as_ref()
    }

    pub fn cli_lock(&self) -> &tokio::sync::Mutex<()> {
        &self.inner.cli_lock
    }

    pub fn pass_ready(&self) -> bool {
        *self.inner.pass_ready.lock().unwrap_or_else(|err| err.into_inner())
    }

    pub fn set_pass_ready(&self, ready: bool) {
        *self.inner.pass_ready.lock().unwrap_or_else(|err| err.into_inner()) = ready;
    }

    pub fn cache_get(&self, key: &str) -> Option<String> {
        self.inner
            .cache
            .lock()
            .unwrap_or_else(|err| err.into_inner())
            .get(key)
            .map(str::to_string)
    }

    pub fn cache_insert(&self, key: &str, value: String) {
        self.inner
            .cache
            .lock()
            .unwrap_or_else(|err| err.into_inner())
            .insert(key, value);
    }

    pub fn cache_invalidate(&self) {
        self.inner
            .cache
            .lock()
            .unwrap_or_else(|err| err.into_inner())
            .invalidate_all();
    }

    pub fn push_notice(&self, notice: Notice) {
        let mut queue = self.inner.notices.lock().unwrap_or_else(|err| err.into_inner());
        queue.push_back(notice);
        while queue.len() > 200 {
            queue.pop_front();
        }
    }

    pub fn notices(&self, limit: usize, clear: bool) -> Vec<Notice> {
        let mut queue = self.inner.notices.lock().unwrap_or_else(|err| err.into_inner());
        let start = queue.len().saturating_sub(limit);
        let out: Vec<Notice> = queue.iter().skip(start).cloned().collect();
        if clear {
            queue.clear();
        }
        out
    }
}
