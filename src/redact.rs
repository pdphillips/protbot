/// Replace token-shaped substrings. Safe to apply to CLI stderr before it is shown.
pub fn redact(input: &str) -> String {
    let mut out = String::with_capacity(input.len());
    let bytes = input.as_bytes();
    let mut i = 0;
    while i < bytes.len() {
        if let Some(len) = secret_span(bytes, i) {
            out.push_str("[redacted]");
            i += len;
            continue;
        }
        let ch = input[i..].chars().next().unwrap();
        out.push(ch);
        i += ch.len_utf8();
    }
    out
}

fn secret_span(bytes: &[u8], i: usize) -> Option<usize> {
    const KEYS: &[&[u8]] = &[
        b"PROTON_PASS_AGENT_TOKEN=",
        b"PROTON_PASS_PERSONAL_ACCESS_TOKEN=",
        b"PROTON_PASS_PASSWORD=",
        b"PROTON_PASS_TOTP=",
    ];
    for key in KEYS {
        if bytes[i..].starts_with(key) {
            let mut j = i + key.len();
            while j < bytes.len() && !bytes[j].is_ascii_whitespace() {
                j += 1;
            }
            return Some(j - i);
        }
    }
    if bytes[i..].starts_with(b"pst_") {
        let mut j = i + 4;
        let mut saw_sep = false;
        while j < bytes.len() && !bytes[j].is_ascii_whitespace() {
            if bytes[j] == b':' {
                saw_sep = true;
            }
            j += 1;
        }
        if saw_sep && j - i > 8 {
            return Some(j - i);
        }
    }
    None
}

pub fn format_audit(tool: &str, id: &str) -> String {
    let tool = sanitize(tool);
    let id = sanitize(id);
    let id = if id.is_empty() { "-".to_string() } else { id };
    format!("protbot tool={tool} id={id}")
}

fn sanitize(value: &str) -> String {
    value
        .chars()
        .filter(|c| c.is_ascii_alphanumeric() || matches!(*c, '_' | '-' | '.'))
        .take(128)
        .collect()
}

pub fn audit(tool: &str, id: &str) {
    eprintln!("{}", format_audit(tool, id));
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn redacts_agent_token_shapes() {
        let raw = "boom pst_abc123::TOKENKEY end PROTON_PASS_AGENT_TOKEN=pst_zz::yy";
        let clean = redact(raw);
        assert!(!clean.contains("pst_"));
        assert!(!clean.contains("TOKENKEY"));
        assert!(clean.contains("[redacted]"));
    }

    #[test]
    fn audit_line_keeps_tool_and_id_only() {
        let line = format_audit("mail_read", "msg/1 body secret");
        assert_eq!(line, "protbot tool=mail_read id=msg1bodysecret");
        assert!(!line.contains('/'));
        assert!(!line.contains('\n'));
    }
}
