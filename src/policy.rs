use std::path::{Component, Path, PathBuf};

#[derive(Clone, Debug, PartialEq, Eq)]
enum Entry {
    Address(String),
    Domain(String),
}

/// Recipient or sender allowlist. Empty permits nobody.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct AllowList {
    entries: Vec<Entry>,
}

impl AllowList {
    pub fn parse_csv(raw: &str) -> Result<Self, String> {
        let mut entries = Vec::new();
        for part in raw.split([',', '\n']) {
            let part = part.trim();
            if part.is_empty() {
                continue;
            }
            entries.push(parse_entry(part)?);
        }
        Ok(Self { entries })
    }

    pub fn from_list(raw: &[String]) -> Result<Self, String> {
        let mut entries = Vec::new();
        for part in raw {
            let part = part.trim();
            if part.is_empty() {
                continue;
            }
            entries.push(parse_entry(part)?);
        }
        Ok(Self { entries })
    }

    pub fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }

    pub fn permits(&self, email: &str) -> bool {
        let Ok(email) = normalize_email(email) else {
            return false;
        };
        self.entries.iter().any(|entry| match entry {
            Entry::Address(addr) => addr == &email,
            Entry::Domain(domain) => email.split_once('@').is_some_and(|(_, host)| host == domain),
        })
    }

    pub fn as_strings(&self) -> Vec<String> {
        self.entries
            .iter()
            .map(|entry| match entry {
                Entry::Address(addr) => addr.clone(),
                Entry::Domain(domain) => format!("@{domain}"),
            })
            .collect()
    }
}

fn parse_entry(raw: &str) -> Result<Entry, String> {
    if let Some(domain) = raw.strip_prefix('@') {
        let domain = domain.trim().to_ascii_lowercase();
        if !valid_domain(&domain) {
            return Err("allowlist domain is invalid".into());
        }
        return Ok(Entry::Domain(domain));
    }
    Ok(Entry::Address(normalize_email(raw)?))
}

pub fn normalize_email(raw: &str) -> Result<String, String> {
    let email = raw.trim().to_ascii_lowercase();
    let Some((local, domain)) = email.split_once('@') else {
        return Err("email is missing @".into());
    };
    if local.is_empty()
        || local.len() > 64
        || domain.len() > 253
        || local.chars().any(|c| c.is_whitespace() || c == '/')
        || !valid_domain(domain)
    {
        return Err("email is invalid".into());
    }
    Ok(email)
}

fn valid_domain(domain: &str) -> bool {
    let mut labels = domain.split('.');
    let Some(first) = labels.next() else {
        return false;
    };
    if first.is_empty() {
        return false;
    }
    let mut saw_dot = false;
    let mut ok = label_ok(first);
    for label in labels {
        saw_dot = true;
        ok = ok && label_ok(label);
    }
    ok && saw_dot && !domain.ends_with('.')
}

fn label_ok(label: &str) -> bool {
    !label.is_empty()
        && label.len() <= 63
        && !label.starts_with('-')
        && !label.ends_with('-')
        && label
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || c == '-')
}

pub fn normalize_abs(path: &str) -> Result<PathBuf, String> {
    if path.is_empty() || path.contains('\0') || path.contains('\\') {
        return Err("path is empty or contains a forbidden character".into());
    }
    let raw = Path::new(path);
    if !raw.is_absolute() {
        return Err("path must be absolute".into());
    }
    let mut out = PathBuf::new();
    for component in raw.components() {
        match component {
            Component::RootDir => out.push("/"),
            Component::Normal(seg) => {
                if seg.to_string_lossy().contains("..") {
                    return Err("path traversal rejected".into());
                }
                out.push(seg);
            }
            Component::CurDir => {}
            Component::ParentDir => return Err("path traversal rejected".into()),
            Component::Prefix(_) => return Err("path prefix rejected".into()),
        }
    }
    if out.as_os_str().is_empty() {
        return Err("path is empty".into());
    }
    Ok(out)
}

pub fn ensure_within(path: &Path, root: &Path) -> Result<(), String> {
    let root = normalize_abs(&root.to_string_lossy())?;
    let path = normalize_abs(&path.to_string_lossy())?;
    if path.starts_with(&root) && path != root {
        return Ok(());
    }
    if path == root {
        return Ok(());
    }
    Err("path is outside the configured directory".into())
}

pub fn valid_item_id(id: &str) -> bool {
    !id.is_empty()
        && id.len() <= 512
        && !id.contains("..")
        && !id.contains('/')
        && !id.contains('\\')
        && id
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || matches!(c, '_' | '.' | ':' | '@' | '+' | '-' | '='))
}

pub fn valid_folder(name: &str) -> bool {
    !name.is_empty()
        && name.len() <= 128
        && !name.contains("..")
        && !name.contains('\0')
        && name
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || matches!(c, ' ' | '_' | '.' | '/' | '-'))
}

pub fn one_line(value: &str, max_chars: usize) -> String {
    let mut out = String::new();
    for ch in value.chars() {
        if out.chars().count() >= max_chars {
            break;
        }
        if ch == '\n' || ch == '\r' || ch == '\t' {
            if !out.ends_with(' ') {
                out.push(' ');
            }
            continue;
        }
        out.push(ch);
    }
    out.trim().to_string()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn allowlist_exact_and_domain() {
        let list = AllowList::parse_csv("Owner@Proton.Me, @Example.com").unwrap();
        assert!(list.permits("owner@proton.me"));
        assert!(list.permits("a@example.com"));
        assert!(!list.permits("a@notexample.com"));
        assert!(!list.permits("owner@proton.me.evil.com"));
        assert!(!AllowList::default().permits("owner@proton.me"));
    }

    #[test]
    fn allowlist_rejects_garbage() {
        assert!(AllowList::parse_csv("not-an-email").is_err());
        assert!(AllowList::parse_csv("@").is_err());
    }

    #[test]
    fn path_traversal_rejected() {
        assert!(normalize_abs("/my-files/docs").is_ok());
        assert!(normalize_abs("/my-files/../../etc/passwd").is_err());
        assert!(normalize_abs("/my-files/foo/../../../etc").is_err());
        assert!(normalize_abs("my-files/docs").is_err());
        assert!(normalize_abs("/my-files/./docs").unwrap().ends_with("docs"));
        let root = Path::new("/data/drive");
        assert!(ensure_within(Path::new("/data/drive/a"), root).is_ok());
        assert!(ensure_within(Path::new("/data/other"), root).is_err());
    }
}
