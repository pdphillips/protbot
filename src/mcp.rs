use crate::app::App;
use crate::tools::{self, ToolOutput};
use crate::watcher::Notice;
use serde_json::{json, Value};
use std::future::pending;
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};
use tokio::sync::mpsc;

pub async fn serve(app: App, mut notices: Option<mpsc::Receiver<Notice>>) -> Result<(), String> {
    let (tx, mut rx) = mpsc::unbounded_channel::<Vec<u8>>();
    tokio::spawn(async move {
        let mut out = tokio::io::stdout();
        while let Some(line) = rx.recv().await {
            if out.write_all(&line).await.is_err() || out.write_all(b"\n").await.is_err() || out.flush().await.is_err() {
                break;
            }
        }
    });

    let mut reader = BufReader::new(tokio::io::stdin());
    let (incoming_tx, mut incoming_rx) = mpsc::unbounded_channel();
    tokio::spawn(async move {
        loop {
            match read_message(&mut reader).await {
                Ok(None) => {
                    let _ = incoming_tx.send(Err("eof".into()));
                    break;
                }
                Ok(Some(message)) => {
                    if incoming_tx.send(Ok(message)).is_err() {
                        break;
                    }
                }
                Err(err) => {
                    if incoming_tx.send(Err(err)).is_err() {
                        break;
                    }
                }
            }
        }
    });

    let mut ready = false;
    let mut min_rank = rank("info");
    let mut pending_notices = Vec::new();

    loop {
        tokio::select! {
            incoming = incoming_rx.recv() => {
                match incoming {
                    None => break,
                    Some(Err(err)) if err == "eof" => break,
                    Some(Err(err)) => {
                        let _ = tx.send(error_line(Value::Null, -32700, &err));
                    }
                    Some(Ok(message)) => {
                        if let Some(response) = handle(&app, &message, &mut ready, &mut min_rank).await {
                            let _ = tx.send(response);
                        }
                        if ready {
                            for notice in pending_notices.drain(..) {
                                emit(&tx, &notice, min_rank);
                            }
                        }
                    }
                }
            }
            notice = next_notice(&mut notices) => {
                match notice {
                    Some(notice) => {
                        if ready {
                            emit(&tx, &notice, min_rank);
                        } else if pending_notices.len() < 50 {
                            pending_notices.push(notice);
                        }
                    }
                    None => {
                        notices = None;
                    }
                }
            }
        }
    }
    Ok(())
}

async fn next_notice(notices: &mut Option<mpsc::Receiver<Notice>>) -> Option<Notice> {
    match notices.as_mut() {
        Some(rx) => rx.recv().await,
        None => pending().await,
    }
}

fn emit(tx: &mpsc::UnboundedSender<Vec<u8>>, notice: &Notice, min_rank: u8) {
    if rank("info") < min_rank {
        return;
    }
    let line = json!({
        "jsonrpc": "2.0",
        "method": "notifications/message",
        "params": {
            "level": "info",
            "logger": "protbot",
            "data": notice,
        }
    });
    if let Ok(bytes) = serde_json::to_vec(&line) {
        let _ = tx.send(bytes);
    }
}

async fn handle(app: &App, message: &Value, ready: &mut bool, min_rank: &mut u8) -> Option<Vec<u8>> {
    let method = message.get("method").and_then(Value::as_str).unwrap_or("");
    let id = message.get("id").cloned();
    let has_id = id.as_ref().is_some_and(|value| !value.is_null());
    if method == "notifications/initialized" || method == "notifications/cancelled" {
        return None;
    }
    let Some(id) = id.filter(|value| !value.is_null()) else {
        if method.is_empty() && has_id {
            return Some(error_line(Value::Null, -32600, "invalid request"));
        }
        return None;
    };
    if method != "initialize" && !*ready {
        return Some(error_line(id, -32600, "server not initialized"));
    }
    match method {
        "initialize" => {
            *ready = true;
            let version = message
                .pointer("/params/protocolVersion")
                .and_then(Value::as_str)
                .filter(|version| {
                    matches!(
                        *version,
                        "2024-11-05" | "2025-03-26" | "2025-06-18" | "2025-11-25"
                    )
                })
                .unwrap_or("2025-06-18");
            Some(result_line(id, json!({
                "protocolVersion": version,
                "capabilities": { "tools": {}, "logging": {} },
                "serverInfo": { "name": "protbot", "version": env!("CARGO_PKG_VERSION") },
                "instructions": "Protbot reads the bot's own Proton account through proton-mail, pass-cli, proton-drive, and ICS feeds. Writes need ALLOW_WRITES=true. Sends only go to the recipient allowlist. Do not expect secrets in logs."
            })))
        }
        "ping" => Some(result_line(id, json!({}))),
        "logging/setLevel" => {
            if let Some(level) = message.pointer("/params/level").and_then(Value::as_str) {
                *min_rank = rank(level);
            }
            Some(result_line(id, json!({})))
        }
        "tools/list" => Some(result_line(id, json!({ "tools": tools::tool_defs() }))),
        "tools/call" => {
            let name = message.pointer("/params/name").and_then(Value::as_str).unwrap_or("");
            let arguments = message.pointer("/params/arguments").cloned().unwrap_or_else(|| json!({}));
            let output = match tokio::time::timeout(app.cfg().tool_timeout, tools::call_tool(app, name, arguments)).await {
                Ok(output) => output,
                Err(_) => ToolOutput { text: "timed out".into(), is_error: true },
            };
            Some(result_line(id, json!({
                "content": [{ "type": "text", "text": output.text }],
                "isError": output.is_error,
            })))
        }
        "" => Some(error_line(id, -32600, "invalid request")),
        _ => Some(error_line(id, -32601, "method not found")),
    }
}

fn rank(level: &str) -> u8 {
    match level {
        "debug" => 0,
        "info" => 1,
        "notice" => 2,
        "warning" => 3,
        "error" => 4,
        "critical" | "alert" | "emergency" => 5,
        _ => 1,
    }
}

fn result_line(id: Value, result: Value) -> Vec<u8> {
    serde_json::to_vec(&json!({ "jsonrpc": "2.0", "id": id, "result": result })).unwrap_or_default()
}

fn error_line(id: Value, code: i32, message: &str) -> Vec<u8> {
    serde_json::to_vec(&json!({
        "jsonrpc": "2.0",
        "id": id,
        "error": { "code": code, "message": message }
    }))
    .unwrap_or_default()
}

async fn read_message(reader: &mut BufReader<tokio::io::Stdin>) -> Result<Option<Value>, String> {
    let mut buf = Vec::new();
    loop {
        let available = reader.fill_buf().await.map_err(|err| err.to_string())?;
        if available.is_empty() {
            if buf.is_empty() {
                return Ok(None);
            }
            break;
        }
        if let Some(index) = available.iter().position(|byte| *byte == b'\n') {
            if buf.len() + index > 4 * 1024 * 1024 {
                reader.consume(index + 1);
                return Err("message too large".into());
            }
            buf.extend_from_slice(&available[..index]);
            reader.consume(index + 1);
            break;
        }
        if buf.len() + available.len() > 4 * 1024 * 1024 {
            return Err("message too large".into());
        }
        let count = available.len();
        buf.extend_from_slice(available);
        reader.consume(count);
    }
    if buf.last() == Some(&b'\r') {
        buf.pop();
    }
    if buf.is_empty() {
        return Ok(Some(Value::Null));
    }
    serde_json::from_slice(&buf).map(Some).map_err(|_| "invalid json".to_string())
}
