use crate::app::App;
use crate::bridge::Outbound;
use crate::calendar::{parse_ics, validate_ics_url};
use crate::cli::{CliError, CliOutput, CommandSpec};
use crate::mail::{self, MailHeader};
use crate::policy::{ensure_within, normalize_abs, normalize_email, valid_item_id};
use crate::redact::{audit, redact};
use crate::settings::SettingsPatch;
use crate::watcher::{self, Notice, WatchState};
use serde::Deserialize;
use serde_json::{json, Value};
use std::time::{Duration, Instant};
use tokio::sync::mpsc;

#[derive(Clone, Debug)]
pub struct ToolOutput {
    pub text: String,
    pub is_error: bool,
}

pub async fn call_tool(app: &App, name: &str, arguments: Value) -> ToolOutput {
    let id = log_id(name, &arguments);
    audit(name, &id);
    match dispatch(app, name, arguments).await {
        Ok(text) => ToolOutput { text, is_error: false },
        Err(err) => ToolOutput {
            text: public_error(&err),
            is_error: true,
        },
    }
}

fn public_error(err: &str) -> String {
    let mut text = strip_urls(&redact(err));
    if text.len() > 500 {
        text.truncate(500);
    }
    text
}

fn strip_urls(input: &str) -> String {
    let mut out = String::with_capacity(input.len());
    let bytes = input.as_bytes();
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i..].starts_with(b"https://") || bytes[i..].starts_with(b"http://") {
            out.push_str("[url]");
            i += if bytes[i..].starts_with(b"https://") { 8 } else { 7 };
            while i < bytes.len() && !bytes[i].is_ascii_whitespace() {
                i += 1;
            }
            continue;
        }
        out.push(bytes[i] as char);
        i += 1;
    }
    out
}

fn log_id(tool: &str, arguments: &Value) -> String {
    for key in ["id", "item_id", "share_id"] {
        if let Some(value) = arguments.get(key).and_then(Value::as_str) {
            if valid_item_id(value) {
                return value.to_string();
            }
        }
    }
    let _ = tool;
    "-".into()
}

async fn dispatch(app: &App, name: &str, arguments: Value) -> Result<String, String> {
    let arguments = if arguments.is_null() { json!({}) } else { arguments };
    match name {
        "mail_list" => mail_list(app, &arguments).await,
        "mail_read" => mail_read(app, &arguments).await,
        "mail_send" => mail_send(app, &arguments).await,
        "mail_trash" => mail_mutate(app, &arguments, "trash").await,
        "mail_delete" => mail_mutate(app, &arguments, "delete").await,
        "mail_move" => mail_move(app, &arguments).await,
        "mail_folders" => mail_folders(app).await,
        "mail_notifications" => mail_notifications(app, &arguments),
        "pass_vaults" => pass_vaults(app).await,
        "pass_items" => pass_items(app, &arguments).await,
        "pass_item_view" => pass_item_view(app, &arguments).await,
        "pass_item_create_login" => pass_item_create(app, &arguments).await,
        "pass_item_delete" => pass_item_delete(app, &arguments).await,
        "pass_totp" => pass_totp(app, &arguments).await,
        "drive_list" => drive_list(app, &arguments).await,
        "drive_info" => drive_info(app, &arguments).await,
        "drive_upload" => drive_upload(app, &arguments).await,
        "drive_download" => drive_download(app, &arguments).await,
        "drive_trash" => drive_trash(app, &arguments).await,
        "calendar_events" => calendar_events(app).await,
        "settings_get" => settings_get(app),
        "settings_update" => settings_update(app, &arguments),
        "whoami" => whoami(app).await,
        _ => Err(format!("unknown tool {name}")),
    }
}

pub fn tool_defs() -> Vec<Value> {
    let empty = obj(json!({}), &[]);
    vec![
        tool("mail_list", "List mail headers through Proton Mail Bridge. Cached for 30 seconds.", obj(json!({"folder": {"type": "string"}, "unread_only": {"type": "boolean"}}), &[])),
        tool("mail_read", "Read one message. The body is returned to the caller and is not logged.", obj(json!({"id": {"type": "string"}}), &["id"])),
        tool("mail_send", "Send mail. Requires ALLOW_WRITES=true and an allowlisted recipient.", obj(json!({"to": {"type": "array", "items": {"type": "string"}}, "cc": {"type": "array", "items": {"type": "string"}}, "bcc": {"type": "array", "items": {"type": "string"}}, "subject": {"type": "string"}, "body": {"type": "string"}}), &["to", "subject", "body"])),
        tool("mail_trash", "Move a message to trash. Requires ALLOW_WRITES=true.", obj(json!({"id": {"type": "string"}}), &["id"])),
        tool("mail_delete", "Delete a message. Requires ALLOW_WRITES=true.", obj(json!({"id": {"type": "string"}}), &["id"])),
        tool("mail_move", "Move a message to a folder. Requires ALLOW_WRITES=true.", obj(json!({"id": {"type": "string"}, "dest": {"type": "string"}}), &["id", "dest"])),
        tool("mail_folders", "List mail folders.", empty.clone()),
        tool("mail_notifications", "Return watcher notices (owner mail and keyword hits immediately, everything else in the hourly batch).", obj(json!({"clear": {"type": "boolean"}, "limit": {"type": "integer"}}), &[])),
        tool("pass_vaults", "List Proton Pass vaults the agent token can see.", empty.clone()),
        tool("pass_items", "List items in a vault. Titles and ids only in the CLI output; this server does not log them.", obj(json!({"vault": {"type": "string"}}), &["vault"])),
        tool("pass_item_view", "View a Pass item. A reason is sent to the Pass audit log.", obj(json!({"vault": {"type": "string"}, "item_id": {"type": "string"}, "item_title": {"type": "string"}, "field": {"type": "string"}}), &["vault"])),
        tool("pass_item_create_login", "Create a login item. The password is sent on stdin, not argv. Requires ALLOW_WRITES=true.", obj(json!({"vault": {"type": "string"}, "title": {"type": "string"}, "username": {"type": "string"}, "email": {"type": "string"}, "password": {"type": "string"}, "url": {"type": "string"}}), &["vault", "title", "password"])),
        tool("pass_item_delete", "Delete a Pass item by share id and item id. Requires ALLOW_WRITES=true.", obj(json!({"share_id": {"type": "string"}, "item_id": {"type": "string"}}), &["share_id", "item_id"])),
        tool("pass_totp", "Read a TOTP code for an item.", obj(json!({"vault": {"type": "string"}, "item_id": {"type": "string"}, "item_title": {"type": "string"}}), &["vault"])),
        tool("drive_list", "List a Drive folder. Absolute paths only; '..' is rejected.", obj(json!({"path": {"type": "string"}}), &["path"])),
        tool("drive_info", "Info for a Drive path.", obj(json!({"path": {"type": "string"}}), &["path"])),
        tool("drive_upload", "Upload a local file into Drive. Requires ALLOW_WRITES=true and PROTBOT_UPLOAD_DIR.", obj(json!({"local_path": {"type": "string"}, "remote_path": {"type": "string"}}), &["local_path", "remote_path"])),
        tool("drive_download", "Download a Drive file into PROTBOT_DOWNLOAD_DIR.", obj(json!({"remote_path": {"type": "string"}, "dest_dir": {"type": "string"}}), &["remote_path", "dest_dir"])),
        tool("drive_trash", "Trash a Drive path. Requires ALLOW_WRITES=true.", obj(json!({"path": {"type": "string"}}), &["path"])),
        tool("calendar_events", "Read ICS feeds configured in settings or PROTBOT_CALENDAR_ICS. Feed URLs are not logged.", empty.clone()),
        tool("settings_get", "Show the clamped settings and whether writes are enabled.", empty.clone()),
        tool("settings_update", "Update settings.toml. Intervals are clamped.", obj(json!({"poll_interval_secs": {"type": "integer"}, "summary_cadence_secs": {"type": "integer"}, "settings_reload_secs": {"type": "integer"}, "sender_allowlist": {"type": "array", "items": {"type": "string"}}, "importance_keywords": {"type": "array", "items": {"type": "string"}}, "calendar_feeds": {"type": "array", "items": {"type": "string"}}}), &[])),
        tool("whoami", "Show the mailbox address signed in to Proton Mail Bridge.", empty),
    ]
}

fn tool(name: &str, description: &str, schema: Value) -> Value {
    json!({ "name": name, "description": description, "inputSchema": schema })
}

fn obj(properties: Value, required: &[&str]) -> Value {
    json!({
        "type": "object",
        "properties": properties,
        "additionalProperties": false,
        "required": required,
    })
}

pub async fn watch_loop(app: App, tx: mpsc::Sender<Notice>) {
    let mut state = WatchState::default();
    loop {
        let _ = app.settings().reload_if_stale();
        let settings = app.settings().get();
        if let Ok(headers) = list_headers(&app, "inbox", false).await {
            for notice in watcher::poll_once(&mut state, &headers, &settings, Instant::now()) {
                app.push_notice(notice.clone());
                if tx.send(notice).await.is_err() {
                    return;
                }
            }
        } else {
            audit("mail_watch", "-");
        }
        tokio::time::sleep(Duration::from_secs(settings.poll_interval_secs)).await;
    }
}

async fn list_headers(app: &App, folder: &str, unread_only: bool) -> Result<Vec<MailHeader>, String> {
    mail::check_folder(folder)?;
    let key = format!("mail:list:{folder}:{unread_only}");
    if let Some(hit) = app.cache_get(&key) {
        return serde_json::from_str(&hit).map_err(|_| "cached mail list was not json".to_string());
    }
    let headers = timed(app, app.mail().list(folder, unread_only)).await?;
    if let Ok(stored) = serde_json::to_string(&headers) {
        app.cache_insert(&key, stored);
    }
    Ok(headers)
}

async fn timed<T>(app: &App, fut: impl std::future::Future<Output = Result<T, String>>) -> Result<T, String> {
    tokio::time::timeout(app.cfg().tool_timeout, fut)
        .await
        .map_err(|_| "bridge timed out".to_string())?
}

async fn mail_list(app: &App, arguments: &Value) -> Result<String, String> {
    #[derive(Deserialize)]
    #[serde(deny_unknown_fields)]
    struct Input {
        #[serde(default = "inbox")]
        folder: String,
        #[serde(default)]
        unread_only: bool,
    }
    fn inbox() -> String {
        "inbox".into()
    }
    let input: Input = parse(arguments)?;
    let headers = list_headers(app, &input.folder, input.unread_only).await?;
    Ok(serde_json::to_string(&headers_json(&headers)).unwrap_or_else(|_| "[]".into()))
}

fn headers_json(headers: &[MailHeader]) -> Value {
    Value::Array(
        headers
            .iter()
            .map(|header| {
                json!({
                    "id": header.id,
                    "from": header.from,
                    "subject": header.subject,
                    "summary": header.summary,
                    "unread": header.unread,
                })
            })
            .collect(),
    )
}

async fn mail_read(app: &App, arguments: &Value) -> Result<String, String> {
    #[derive(Deserialize)]
    #[serde(deny_unknown_fields)]
    struct Input {
        id: String,
    }
    let input: Input = parse(arguments)?;
    require_id(&input.id)?;
    timed(app, app.mail().read(&input.id)).await
}

async fn mail_send(app: &App, arguments: &Value) -> Result<String, String> {
    #[derive(Deserialize)]
    #[serde(deny_unknown_fields)]
    struct Input {
        to: Vec<String>,
        #[serde(default)]
        cc: Vec<String>,
        #[serde(default)]
        bcc: Vec<String>,
        subject: String,
        body: String,
    }
    let input: Input = parse(arguments)?;
    require_writes(app)?;
    if input.to.is_empty() {
        return Err("at least one recipient is required".into());
    }
    if input.subject.is_empty() || input.subject.len() > 998 || input.subject.chars().any(|ch| ch == '\n' || ch == '\r' || ch == '\0') {
        return Err("subject is invalid".into());
    }
    if input.body.len() > 200_000 {
        return Err("body is too large".into());
    }
    let mut recipients = Vec::new();
    for addr in input.to.iter().chain(input.cc.iter()).chain(input.bcc.iter()) {
        let email = normalize_email(addr)?;
        if !app.cfg().recipients.permits(&email) {
            return Err(format!("recipient is not allowlisted: {email}"));
        }
        recipients.push(email);
    }
    let message = Outbound {
        to: input.to.iter().map(|addr| normalize_email(addr)).collect::<Result<Vec<_>, _>>()?,
        cc: input.cc.iter().map(|addr| normalize_email(addr)).collect::<Result<Vec<_>, _>>()?,
        bcc: input.bcc.iter().map(|addr| normalize_email(addr)).collect::<Result<Vec<_>, _>>()?,
        subject: input.subject,
        body: input.body,
    };
    let _ = recipients;
    let out = timed(app, app.mail().send(&message)).await?;
    app.cache_invalidate();
    Ok(out)
}

async fn mail_mutate(app: &App, arguments: &Value, verb: &str) -> Result<String, String> {
    #[derive(Deserialize)]
    #[serde(deny_unknown_fields)]
    struct Input {
        id: String,
    }
    let input: Input = parse(arguments)?;
    require_writes(app)?;
    require_id(&input.id)?;
    let out = if verb == "delete" {
        timed(app, app.mail().delete(&input.id)).await?
    } else {
        timed(app, app.mail().trash(&input.id)).await?
    };
    app.cache_invalidate();
    Ok(out)
}

async fn mail_move(app: &App, arguments: &Value) -> Result<String, String> {
    #[derive(Deserialize)]
    #[serde(deny_unknown_fields)]
    struct Input {
        id: String,
        dest: String,
    }
    let input: Input = parse(arguments)?;
    require_writes(app)?;
    require_id(&input.id)?;
    mail::check_folder(&input.dest)?;
    let out = timed(app, app.mail().move_msg(&input.id, &input.dest)).await?;
    app.cache_invalidate();
    Ok(out)
}

async fn mail_folders(app: &App) -> Result<String, String> {
    const KEY: &str = "mail:folders";
    if let Some(hit) = app.cache_get(KEY) {
        return Ok(hit);
    }
    let out = timed(app, app.mail().folders()).await?;
    serde_json::from_str::<Value>(&out).map_err(|_| "folder list was not json".to_string())?;
    app.cache_insert(KEY, out.clone());
    Ok(out)
}

fn mail_notifications(app: &App, arguments: &Value) -> Result<String, String> {
    #[derive(Deserialize)]
    #[serde(deny_unknown_fields)]
    struct Input {
        #[serde(default)]
        clear: bool,
        #[serde(default = "twenty")]
        limit: usize,
    }
    fn twenty() -> usize {
        20
    }
    let input: Input = parse(arguments)?;
    let limit = input.limit.clamp(1, 100);
    let notices = app.notices(limit, input.clear);
    serde_json::to_string(&notices).map_err(|err| err.to_string())
}

async fn pass_vaults(app: &App) -> Result<String, String> {
    let out = pass_cmd(app, "pass_vaults", "-", vec!["vault".into(), "list".into(), "--output".into(), "json".into()], Vec::new()).await?;
    Ok(out.stdout)
}

async fn pass_items(app: &App, arguments: &Value) -> Result<String, String> {
    #[derive(Deserialize)]
    #[serde(deny_unknown_fields)]
    struct Input {
        vault: String,
    }
    let input: Input = parse(arguments)?;
    require_label(&input.vault)?;
    let out = pass_cmd(
        app,
        "pass_items",
        "-",
        vec!["item".into(), "list".into(), "--vault-name".into(), input.vault, "--output".into(), "json".into()],
        Vec::new(),
    )
    .await?;
    Ok(out.stdout)
}

async fn pass_item_view(app: &App, arguments: &Value) -> Result<String, String> {
    let (vault, item_id, item_title, field) = pass_ref(arguments)?;
    let mut args = vec!["item".into(), "view".into(), "--vault-name".into(), vault, "--output".into(), "json".into()];
    push_item_ref(&mut args, &item_id, &item_title)?;
    if !field.is_empty() {
        require_label(&field)?;
        args.push("--field".into());
        args.push(field);
    }
    let id = if item_id.is_empty() { "-" } else { &item_id };
    let out = pass_cmd(app, "pass_item_view", id, args, Vec::new()).await?;
    Ok(out.stdout)
}

async fn pass_item_create(app: &App, arguments: &Value) -> Result<String, String> {
    #[derive(Deserialize)]
    #[serde(deny_unknown_fields)]
    struct Input {
        vault: String,
        title: String,
        #[serde(default)]
        username: String,
        #[serde(default)]
        email: String,
        password: String,
        #[serde(default)]
        url: String,
    }
    let input: Input = parse(arguments)?;
    require_writes(app)?;
    require_label(&input.vault)?;
    require_label(&input.title)?;
    if input.password.is_empty() || input.password.len() > 1024 || input.password.contains('\0') {
        return Err("password is invalid".into());
    }
    if !input.email.is_empty() {
        normalize_email(&input.email)?;
    }
    if !input.url.is_empty() {
        validate_ics_url(&input.url)?;
    }
    let mut template = serde_json::Map::new();
    template.insert("title".into(), json!(input.title));
    template.insert("password".into(), json!(input.password));
    if !input.username.is_empty() {
        template.insert("username".into(), json!(input.username));
    }
    if !input.email.is_empty() {
        template.insert("email".into(), json!(input.email));
    }
    if !input.url.is_empty() {
        template.insert("urls".into(), json!([input.url]));
    }
    let stdin = serde_json::to_vec(&Value::Object(template)).map_err(|err| err.to_string())?;
    let out = pass_cmd(
        app,
        "pass_item_create_login",
        "-",
        vec!["item".into(), "create".into(), "login".into(), "--vault-name".into(), input.vault, "--from-template".into(), "-".into()],
        stdin,
    )
    .await?;
    Ok(out.stdout)
}

async fn pass_item_delete(app: &App, arguments: &Value) -> Result<String, String> {
    #[derive(Deserialize)]
    #[serde(deny_unknown_fields)]
    struct Input {
        share_id: String,
        item_id: String,
    }
    let input: Input = parse(arguments)?;
    require_writes(app)?;
    require_id(&input.share_id)?;
    require_id(&input.item_id)?;
    let out = pass_cmd(
        app,
        "pass_item_delete",
        &input.item_id,
        vec!["item".into(), "delete".into(), "--share-id".into(), input.share_id, "--item-id".into(), input.item_id.clone()],
        Vec::new(),
    )
    .await?;
    Ok(out.stdout)
}

async fn pass_totp(app: &App, arguments: &Value) -> Result<String, String> {
    let (vault, item_id, item_title, _) = pass_ref(arguments)?;
    let mut args = vec!["item".into(), "totp".into(), "--vault-name".into(), vault, "--output".into(), "json".into()];
    push_item_ref(&mut args, &item_id, &item_title)?;
    let id = if item_id.is_empty() { "-" } else { &item_id };
    let out = pass_cmd(app, "pass_totp", id, args, Vec::new()).await?;
    Ok(out.stdout)
}

fn pass_ref(arguments: &Value) -> Result<(String, String, String, String), String> {
    #[derive(Deserialize)]
    #[serde(deny_unknown_fields)]
    struct Input {
        vault: String,
        #[serde(default)]
        item_id: String,
        #[serde(default)]
        item_title: String,
        #[serde(default)]
        field: String,
    }
    let input: Input = parse(arguments)?;
    require_label(&input.vault)?;
    match (input.item_id.is_empty(), input.item_title.is_empty()) {
        (false, true) => require_id(&input.item_id)?,
        (true, false) => require_label(&input.item_title)?,
        _ => return Err("provide item_id or item_title, not both".into()),
    }
    Ok((input.vault, input.item_id, input.item_title, input.field))
}

fn push_item_ref(args: &mut Vec<String>, item_id: &str, item_title: &str) -> Result<(), String> {
    if !item_id.is_empty() {
        args.push("--item-id".into());
        args.push(item_id.into());
    } else {
        args.push("--item-title".into());
        args.push(item_title.into());
    }
    Ok(())
}

async fn pass_cmd(app: &App, tool: &str, item_id: &str, args: Vec<String>, stdin: Vec<u8>) -> Result<CliOutput, String> {
    ensure_pass(app).await?;
    let token = app.cfg().token().ok_or("pass token is not set")?.to_string();
    let reason = agent_reason(tool, item_id);
    let session = app.cfg().session_dir.display().to_string();
    run_cli(
        app,
        &app.cfg().pass_bin,
        args,
        stdin,
        Some(PassEnv { token, reason, session }),
    )
    .await
}

async fn ensure_pass(app: &App) -> Result<(), String> {
    if app.pass_ready() {
        return Ok(());
    }
    let token = app.cfg().token().ok_or("pass token is not set")?.to_string();
    let session = app.cfg().session_dir.display().to_string();
    let info = run_cli(
        app,
        &app.cfg().pass_bin,
        vec!["info".into(), "--output".into(), "json".into()],
        Vec::new(),
        Some(PassEnv {
            token: token.clone(),
            reason: agent_reason("pass_session", "-"),
            session: session.clone(),
        }),
    )
    .await;
    if let Err(err) = info {
        if err.contains("timed out") {
            return Err(err);
        }
        run_cli(
            app,
            &app.cfg().pass_bin,
            vec!["login".into()],
            Vec::new(),
            Some(PassEnv {
                token,
                reason: agent_reason("pass_login", "-"),
                session,
            }),
        )
        .await?;
    }
    app.set_pass_ready(true);
    Ok(())
}

struct PassEnv {
    token: String,
    reason: String,
    session: String,
}

fn agent_reason(tool: &str, item_id: &str) -> String {
    let reason = if item_id.is_empty() || item_id == "-" {
        format!("protbot {tool}")
    } else {
        format!("protbot {tool} item {item_id}")
    };
    reason.chars().take(300).collect()
}

async fn drive_list(app: &App, arguments: &Value) -> Result<String, String> {
    let path = remote_path(arguments)?;
    let key = format!("drive:list:{path}");
    if let Some(hit) = app.cache_get(&key) {
        return Ok(hit);
    }
    let out = run_cli(
        app,
        &app.cfg().drive_bin,
        vec!["--json".into(), "filesystem".into(), "list".into(), path.clone()],
        Vec::new(),
        None,
    )
    .await?;
    serde_json::from_str::<Value>(&out.stdout).map_err(|_| "drive list was not json".to_string())?;
    app.cache_insert(&key, out.stdout.clone());
    Ok(out.stdout)
}

async fn drive_info(app: &App, arguments: &Value) -> Result<String, String> {
    let path = remote_path(arguments)?;
    let key = format!("drive:info:{path}");
    if let Some(hit) = app.cache_get(&key) {
        return Ok(hit);
    }
    let out = run_cli(
        app,
        &app.cfg().drive_bin,
        vec!["--json".into(), "filesystem".into(), "info".into(), path.clone()],
        Vec::new(),
        None,
    )
    .await?;
    app.cache_insert(&key, out.stdout.clone());
    Ok(out.stdout)
}

async fn drive_upload(app: &App, arguments: &Value) -> Result<String, String> {
    #[derive(Deserialize)]
    #[serde(deny_unknown_fields)]
    struct Input {
        local_path: String,
        remote_path: String,
    }
    let input: Input = parse(arguments)?;
    require_writes(app)?;
    let local = normalize_abs(&input.local_path)?;
    let remote = normalize_abs(&input.remote_path)?;
    let root = app.cfg().upload_dir.as_deref().ok_or("PROTBOT_UPLOAD_DIR is not set")?;
    ensure_within(&local, root)?;
    let out = run_cli(
        app,
        &app.cfg().drive_bin,
        vec!["filesystem".into(), "upload".into(), local.display().to_string(), remote.display().to_string()],
        Vec::new(),
        None,
    )
    .await?;
    app.cache_invalidate();
    Ok(if out.stdout.is_empty() { "uploaded".into() } else { out.stdout })
}

async fn drive_download(app: &App, arguments: &Value) -> Result<String, String> {
    #[derive(Deserialize)]
    #[serde(deny_unknown_fields)]
    struct Input {
        remote_path: String,
        dest_dir: String,
    }
    let input: Input = parse(arguments)?;
    let remote = normalize_abs(&input.remote_path)?;
    let dest = normalize_abs(&input.dest_dir)?;
    let root = app.cfg().download_dir.as_deref().ok_or("PROTBOT_DOWNLOAD_DIR is not set")?;
    ensure_within(&dest, root)?;
    let out = run_cli(
        app,
        &app.cfg().drive_bin,
        vec!["filesystem".into(), "download".into(), remote.display().to_string(), dest.display().to_string()],
        Vec::new(),
        None,
    )
    .await?;
    Ok(if out.stdout.is_empty() { "downloaded".into() } else { out.stdout })
}

async fn drive_trash(app: &App, arguments: &Value) -> Result<String, String> {
    require_writes(app)?;
    let path = remote_path(arguments)?;
    let out = run_cli(
        app,
        &app.cfg().drive_bin,
        vec!["filesystem".into(), "trash".into(), path],
        Vec::new(),
        None,
    )
    .await?;
    app.cache_invalidate();
    Ok(if out.stdout.is_empty() { "trashed".into() } else { out.stdout })
}

fn remote_path(arguments: &Value) -> Result<String, String> {
    #[derive(Deserialize)]
    #[serde(deny_unknown_fields)]
    struct Input {
        path: String,
    }
    let input: Input = parse(arguments)?;
    Ok(normalize_abs(&input.path)?.display().to_string())
}

async fn calendar_events(app: &App) -> Result<String, String> {
    let _ = app.settings().reload_if_stale();
    let settings = app.settings().get();
    let mut events = Vec::new();
    let mut failed = 0u32;
    for (index, url) in app.cfg().calendar_ics.iter().enumerate() {
        let key = format!("calendar:env:{index}");
        match cached_ics(app, &key, url).await {
            Ok(text) => events.extend(parse_ics(&text)),
            Err(_) => failed += 1,
        }
    }
    for (index, feed) in settings.calendar_feeds.iter().enumerate() {
        let key = format!("calendar:ref:{index}");
        if let Some(hit) = app.cache_get(&key) {
            events.extend(parse_ics(&hit));
            continue;
        }
        let url = match feed_url(app, feed).await {
            Ok(url) => url,
            Err(_) => {
                failed += 1;
                continue;
            }
        };
        match fetch_ics(app, &url).await {
            Ok(text) => {
                app.cache_insert(&key, text.clone());
                events.extend(parse_ics(&text));
            }
            Err(_) => failed += 1,
        }
    }
    events.truncate(100);
    Ok(serde_json::to_string(&json!({
        "failed_feeds": failed,
        "events": events.iter().map(|event| json!({
            "uid": event.uid,
            "summary": event.summary,
            "start": event.start,
            "end": event.end,
            "location": event.location,
        })).collect::<Vec<_>>(),
    })).unwrap_or_else(|_| "{}".into()))
}

async fn cached_ics(app: &App, key: &str, url: &str) -> Result<String, String> {
    if let Some(hit) = app.cache_get(key) {
        return Ok(hit);
    }
    let text = fetch_ics(app, url).await?;
    app.cache_insert(key, text.clone());
    Ok(text)
}

async fn feed_url(app: &App, feed: &str) -> Result<String, String> {
    let Some((vault, item)) = feed.split_once('/') else {
        return Err("calendar feed ref is invalid".into());
    };
    let out = pass_cmd(
        app,
        "calendar_feed",
        "-",
        vec!["item".into(), "view".into(), "--vault-name".into(), vault.into(), "--item-title".into(), item.into(), "--output".into(), "json".into()],
        Vec::new(),
    )
    .await?;
    let value: Value = serde_json::from_str(&out.stdout).map_err(|_| "pass item was not json".to_string())?;
    let mut found = Vec::new();
    collect_https(&value, &mut found);
    let url = found
        .iter()
        .find(|url| {
            let lower = url.to_ascii_lowercase();
            lower.contains("ics") || lower.contains("calendar")
        })
        .cloned()
        .or_else(|| found.into_iter().next())
        .ok_or_else(|| "pass item has no https feed".to_string())?;
    validate_ics_url(&url)?;
    Ok(url)
}

fn collect_https(value: &Value, out: &mut Vec<String>) {
    match value {
        Value::String(text) if text.starts_with("https://") => out.push(text.clone()),
        Value::Array(items) => {
            for item in items {
                collect_https(item, out);
            }
        }
        Value::Object(map) => {
            for item in map.values() {
                collect_https(item, out);
            }
        }
        _ => {}
    }
}

async fn fetch_ics(app: &App, url: &str) -> Result<String, String> {
    validate_ics_url(url)?;
    let stdin = format!("silent\nshow-error\nfail\nmax-time = 20\nurl = \"{url}\"\n");
    let out = run_cli(app, &app.cfg().curl_bin, vec!["--config".into(), "-".into()], stdin.into_bytes(), None).await?;
    Ok(out.stdout)
}

fn settings_get(app: &App) -> Result<String, String> {
    let _ = app.settings().reload_if_stale();
    let settings = app.settings().get();
    Ok(serde_json::to_string(&json!({
        "settings": settings,
        "writes_enabled": app.cfg().allow_writes,
        "recipient_allowlist": app.cfg().recipients.as_strings(),
        "pass_token_present": app.cfg().token().is_some(),
        "bridge_password_present": app.cfg().bridge_password_present,
    })).unwrap_or_else(|_| "{}".into()))
}

fn settings_update(app: &App, arguments: &Value) -> Result<String, String> {
    let patch: SettingsPatch = parse(arguments)?;
    let settings = app.settings().apply_patch(patch)?;
    Ok(serde_json::to_string(&settings).map_err(|err| err.to_string())?)
}

async fn whoami(app: &App) -> Result<String, String> {
    timed(app, app.mail().whoami()).await
}

fn require_writes(app: &App) -> Result<(), String> {
    if app.cfg().allow_writes {
        Ok(())
    } else {
        Err("writes are disabled; set ALLOW_WRITES=true".into())
    }
}

fn require_id(id: &str) -> Result<(), String> {
    if valid_item_id(id) {
        Ok(())
    } else {
        Err("id is invalid".into())
    }
}

fn require_label(value: &str) -> Result<(), String> {
    if value.is_empty()
        || value.len() > 200
        || value.chars().any(|ch| ch.is_control())
        || value.contains("..")
    {
        Err("label is invalid".into())
    } else {
        Ok(())
    }
}

fn parse<T: for<'de> Deserialize<'de>>(arguments: &Value) -> Result<T, String> {
    serde_json::from_value(arguments.clone()).map_err(|_| "invalid arguments".to_string())
}

async fn run_cli(
    app: &App,
    bin: &str,
    args: Vec<String>,
    stdin_data: Vec<u8>,
    pass: Option<PassEnv>,
) -> Result<CliOutput, String> {
    let _guard = app.cli_lock().lock().await;
    let mut env_overrides = Vec::new();
    if let Some(pass) = pass {
        env_overrides.push(("PROTON_PASS_PERSONAL_ACCESS_TOKEN".into(), pass.token));
        env_overrides.push(("PROTON_PASS_AGENT_REASON".into(), pass.reason));
        if !pass.session.is_empty() {
            env_overrides.push(("PROTON_PASS_SESSION_DIR".into(), pass.session));
        }
    }
    let spec = CommandSpec {
        bin: bin.to_string(),
        args,
        stdin_data,
        env_overrides,
        scrub_secrets: true,
        timeout: app.cfg().tool_timeout,
    };
    app.runner().run(spec).await.map_err(|err| match err {
        CliError::Exit { bin, code, stderr } => {
            let stderr = public_error(&stderr);
            if stderr.is_empty() {
                format!("{bin} exited {code}")
            } else {
                stderr
            }
        }
        other => public_error(&other.to_string()),
    })
}
