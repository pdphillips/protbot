use crate::policy::{one_line, valid_folder, valid_item_id};
use serde_json::Value;

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct MailHeader {
    pub id: String,
    pub from: String,
    pub subject: String,
    pub summary: String,
    pub unread: bool,
}

pub fn parse_message_list(text: &str) -> Result<Vec<MailHeader>, String> {
    let value: Value = serde_json::from_str(text).map_err(|_| "mail list was not json".to_string())?;
    let items: Vec<Value> = if let Some(items) = value.as_array() {
        items.clone()
    } else {
        value
            .get("messages")
            .or_else(|| value.get("Messages"))
            .and_then(Value::as_array)
            .cloned()
            .ok_or_else(|| "mail list missing messages".to_string())?
    };
    let mut out = Vec::new();
    for item in items {
        let Some(id) = first_str(&item, &["id", "ID", "message_id", "MessageID"]) else {
            continue;
        };
        if !valid_item_id(&id) {
            continue;
        }
        let from = sender(&item);
        let subject = one_line(&first_str(&item, &["subject", "Subject"]).unwrap_or_default(), 200);
        let snippet = first_str(&item, &["snippet", "summary", "preview", "Snippet"]);
        let summary = one_line(&snippet.unwrap_or_else(|| subject.clone()), 160);
        out.push(MailHeader {
            id,
            from,
            subject,
            summary,
            unread: flag(&item, &["unread", "Unread"]),
        });
    }
    Ok(out)
}

fn sender(item: &Value) -> String {
    if let Some(value) = first_str(item, &["from_address", "fromAddress", "senderAddress"]) {
        return value.to_ascii_lowercase();
    }
    for pointer in ["/from/address", "/sender/address", "/Sender/Address", "/from/Address"] {
        if let Some(value) = item.pointer(pointer).and_then(Value::as_str) {
            return value.trim().to_ascii_lowercase();
        }
    }
    first_str(item, &["from", "sender"])
        .unwrap_or_default()
        .to_ascii_lowercase()
}

fn first_str(item: &Value, keys: &[&str]) -> Option<String> {
    for key in keys {
        if let Some(value) = item.get(*key).and_then(Value::as_str) {
            let value = value.trim();
            if !value.is_empty() {
                return Some(value.to_string());
            }
        }
    }
    None
}

fn flag(item: &Value, keys: &[&str]) -> bool {
    for key in keys {
        match item.get(*key) {
            Some(Value::Bool(value)) => return *value,
            Some(Value::Number(value)) => return value.as_i64().unwrap_or(0) != 0,
            _ => {}
        }
    }
    false
}

pub fn check_folder(folder: &str) -> Result<(), String> {
    if valid_folder(folder) {
        Ok(())
    } else {
        Err("folder name is invalid".into())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_wrapped_and_bare_lists() {
        let wrapped = r#"{"messages":[{"id":"m1","from":"Owner@Proton.Me","subject":"Hi","snippet":"line one\nline two","unread":1}]}"#;
        let headers = parse_message_list(wrapped).unwrap();
        assert_eq!(headers[0].from, "owner@proton.me");
        assert_eq!(headers[0].summary, "line one line two");
        assert!(headers[0].unread);
        let bare = r#"[{"ID":"m2","Sender":{"Address":"a@b.co"},"Subject":"S"}]"#;
        let headers = parse_message_list(bare).unwrap();
        assert_eq!(headers[0].id, "m2");
        assert_eq!(headers[0].from, "a@b.co");
        assert!(parse_message_list("not-json").is_err());
    }
}
