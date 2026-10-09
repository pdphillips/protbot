#[derive(Clone, Debug, PartialEq, Eq)]
pub struct CalEvent {
    pub uid: String,
    pub summary: String,
    pub start: String,
    pub end: String,
    pub location: String,
}

pub fn parse_ics(text: &str) -> Vec<CalEvent> {
    let unfolded = unfold(text);
    let mut events = Vec::new();
    let mut current: Option<CalEvent> = None;
    for line in unfolded.lines() {
        let line = line.trim_end();
        if line.eq_ignore_ascii_case("BEGIN:VEVENT") {
            current = Some(CalEvent {
                uid: String::new(),
                summary: String::new(),
                start: String::new(),
                end: String::new(),
                location: String::new(),
            });
            continue;
        }
        if line.eq_ignore_ascii_case("END:VEVENT") {
            if let Some(event) = current.take() {
                if !event.uid.is_empty() || !event.summary.is_empty() {
                    events.push(event);
                }
            }
            continue;
        }
        let Some(event) = current.as_mut() else {
            continue;
        };
        let Some((name, value)) = split_prop(line) else {
            continue;
        };
        match name.as_str() {
            "UID" => event.uid = value,
            "SUMMARY" => event.summary = value,
            "DTSTART" => event.start = value,
            "DTEND" => event.end = value,
            "LOCATION" => event.location = value,
            _ => {}
        }
    }
    events.truncate(100);
    events
}

fn unfold(text: &str) -> String {
    let mut out = String::new();
    for line in text.lines() {
        if line.starts_with(' ') || line.starts_with('\t') {
            out.push_str(line.trim_start());
        } else {
            if !out.is_empty() {
                out.push('\n');
            }
            out.push_str(line.trim_end_matches('\r'));
        }
    }
    out
}

fn split_prop(line: &str) -> Option<(String, String)> {
    let (left, value) = line.split_once(':')?;
    let name = left.split(';').next()?.trim().to_ascii_uppercase();
    if name.is_empty() {
        return None;
    }
    Some((name, unescape(value.trim())))
}

fn unescape(value: &str) -> String {
    let mut out = String::new();
    let mut chars = value.chars().peekable();
    while let Some(ch) = chars.next() {
        if ch == '\\' {
            match chars.next() {
                Some('n') | Some('N') => out.push(' '),
                Some(other) => out.push(other),
                None => {}
            }
        } else {
            out.push(ch);
        }
    }
    out
}

pub fn validate_ics_url(url: &str) -> Result<(), String> {
    if !url.starts_with("https://") || url.len() > 2048 || url.chars().any(|ch| ch.is_control() || ch == '"') {
        return Err("calendar feed must be an https URL".into());
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_unfolded_event() {
        let text = "BEGIN:VCALENDAR\r\nBEGIN:VEVENT\r\nUID:evt-1\r\nDTSTART:20261009T150000Z\r\nSUMMARY:Stand\r\n up\r\nLOCATION:Lab\r\nEND:VEVENT\r\nEND:VCALENDAR\r\n";
        let events = parse_ics(text);
        assert_eq!(events.len(), 1);
        assert_eq!(events[0].uid, "evt-1");
        assert_eq!(events[0].summary, "Standup");
        assert_eq!(events[0].start, "20261009T150000Z");
        assert_eq!(events[0].location, "Lab");
        assert!(validate_ics_url("http://insecure.example/a").is_err());
    }
}
