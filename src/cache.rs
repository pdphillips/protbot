use std::collections::HashMap;
use std::time::{Duration, Instant};

#[derive(Debug)]
pub struct TtlCache {
    ttl: Duration,
    map: HashMap<String, (Instant, String)>,
}

impl TtlCache {
    pub fn new(ttl: Duration) -> Self {
        Self {
            ttl,
            map: HashMap::new(),
        }
    }

    pub fn get_at(&self, key: &str, now: Instant) -> Option<&str> {
        let (stored, value) = self.map.get(key)?;
        if now.saturating_duration_since(*stored) >= self.ttl {
            return None;
        }
        Some(value.as_str())
    }

    pub fn get(&self, key: &str) -> Option<&str> {
        self.get_at(key, Instant::now())
    }

    pub fn insert_at(&mut self, key: impl Into<String>, value: String, now: Instant) {
        self.map.insert(key.into(), (now, value));
    }

    pub fn insert(&mut self, key: impl Into<String>, value: String) {
        self.insert_at(key, value, Instant::now());
    }

    pub fn invalidate_all(&mut self) {
        self.map.clear();
    }

    #[cfg(test)]
    pub fn len(&self) -> usize {
        self.map.len()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn expires_and_invalidates() {
        let mut cache = TtlCache::new(Duration::from_secs(30));
        let t0 = Instant::now();
        cache.insert_at("mail:list:inbox", "headers".into(), t0);
        assert_eq!(cache.get_at("mail:list:inbox", t0).unwrap(), "headers");
        assert!(cache
            .get_at("mail:list:inbox", t0 + Duration::from_secs(30))
            .is_none());
        cache.insert_at("mail:folders", "folders".into(), t0);
        cache.insert_at("calendar:0", "events".into(), t0);
        assert_eq!(cache.len(), 3);
        cache.invalidate_all();
        assert_eq!(cache.len(), 0);
        assert!(cache.get_at("mail:folders", t0).is_none());
    }
}
