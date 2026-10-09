use crate::mail::MailHeader;
use crate::settings::Settings;
use serde::Serialize;
use std::collections::{HashSet, VecDeque};
use std::time::{Duration, Instant};

const SEEN_CAP: usize = 10_000;

#[derive(Clone, Debug, Serialize, PartialEq, Eq)]
pub struct NoticeMsg {
    pub id: String,
    pub from: String,
    pub subject: String,
    pub summary: String,
}

#[derive(Clone, Debug, Serialize, PartialEq, Eq)]
pub struct Notice {
    pub kind: String,
    pub messages: Vec<NoticeMsg>,
}

#[derive(Default)]
pub struct WatchState {
    seen: HashSet<String>,
    order: VecDeque<String>,
    pub baselined: bool,
    pub batch: Vec<MailHeader>,
    pub last_summary: Option<Instant>,
}

impl WatchState {
    fn mark(&mut self, id: &str) {
        if self.seen.insert(id.to_string()) {
            self.order.push_back(id.to_string());
            while self.order.len() > SEEN_CAP {
                if let Some(old) = self.order.pop_front() {
                    self.seen.remove(&old);
                }
            }
        }
    }
}

pub fn poll_once(state: &mut WatchState, headers: &[MailHeader], settings: &Settings, now: Instant) -> Vec<Notice> {
    if !state.baselined {
        for header in headers {
            state.mark(&header.id);
        }
        state.baselined = true;
        state.last_summary = Some(now);
        return Vec::new();
    }

    let mut immediate = Vec::new();
    for header in headers {
        if state.seen.contains(&header.id) {
            continue;
        }
        state.mark(&header.id);
        if is_immediate(header, settings) {
            immediate.push(header.clone());
        } else {
            state.batch.push(header.clone());
        }
    }

    let mut events = Vec::new();
    if !immediate.is_empty() {
        events.push(Notice {
            kind: "immediate".into(),
            messages: immediate.iter().map(notice_msg).collect(),
        });
    }

    let due = state
        .last_summary
        .map(|then| now.saturating_duration_since(then) >= Duration::from_secs(settings.summary_cadence_secs))
        .unwrap_or(true);
    if due {
        state.last_summary = Some(now);
        if !state.batch.is_empty() {
            let messages = std::mem::take(&mut state.batch);
            events.push(Notice {
                kind: "batch".into(),
                messages: messages.iter().map(notice_msg).collect(),
            });
        }
    }
    events
}

fn is_immediate(header: &MailHeader, settings: &Settings) -> bool {
    let from = header.from.to_ascii_lowercase();
    if settings.sender_allowlist.iter().any(|entry| {
        if let Some(domain) = entry.strip_prefix('@') {
            from.split_once('@').is_some_and(|(_, host)| host == domain)
        } else {
            entry == &from
        }
    }) {
        return true;
    }
    let hay = format!("{} {}", header.subject, header.summary).to_ascii_lowercase();
    settings
        .importance_keywords
        .iter()
        .any(|word| !word.is_empty() && hay.contains(&word.to_ascii_lowercase()))
}

fn notice_msg(header: &MailHeader) -> NoticeMsg {
    NoticeMsg {
        id: header.id.clone(),
        from: header.from.clone(),
        subject: header.subject.clone(),
        summary: header.summary.clone(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::settings::Settings;

    fn msg(id: &str, from: &str, subject: &str, summary: &str) -> MailHeader {
        MailHeader {
            id: id.into(),
            from: from.into(),
            subject: subject.into(),
            summary: summary.into(),
            unread: true,
        }
    }

    fn settings() -> Settings {
        Settings {
            poll_interval_secs: 30,
            summary_cadence_secs: 300,
            settings_reload_secs: 30,
            sender_allowlist: vec!["owner@proton.me".into()],
            importance_keywords: vec!["otp".into()],
            calendar_feeds: vec![],
        }
    }

    #[test]
    fn dedupes_immediate_and_batch() {
        let mut state = WatchState::default();
        let settings = settings();
        let t0 = Instant::now();
        let old = msg("old", "news@lists.example", "Digest", "weekly");
        assert!(poll_once(&mut state, &[old.clone()], &settings, t0).is_empty());

        let owner = msg("own", "owner@proton.me", "Ping", "need you");
        let events = poll_once(&mut state, &[old.clone(), owner.clone()], &settings, t0 + Duration::from_secs(1));
        assert_eq!(events.len(), 1);
        assert_eq!(events[0].kind, "immediate");
        assert_eq!(events[0].messages[0].id, "own");
        assert!(state.batch.is_empty());

        let again = poll_once(&mut state, &[old, owner], &settings, t0 + Duration::from_secs(2));
        assert!(again.is_empty());

        let other = msg("n1", "vendor@shop.example", "Invoice", "attached");
        let keyword = msg("n2", "bot@shop.example", "Your OTP", "code inside");
        let events = poll_once(
            &mut state,
            &[other.clone(), keyword.clone()],
            &settings,
            t0 + Duration::from_secs(3),
        );
        assert_eq!(events.len(), 1);
        assert_eq!(events[0].messages[0].id, "n2");
        assert_eq!(state.batch.len(), 1);
        assert_eq!(state.batch[0].id, "n1");

        let events = poll_once(&mut state, &[], &settings, t0 + Duration::from_secs(303));
        assert_eq!(events.len(), 1);
        assert_eq!(events[0].kind, "batch");
        assert_eq!(events[0].messages[0].id, "n1");
        assert!(state.batch.is_empty());

        let quiet = poll_once(&mut state, &[], &settings, t0 + Duration::from_secs(700));
        assert!(quiet.is_empty());
    }
}
