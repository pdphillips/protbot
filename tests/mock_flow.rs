use std::sync::Arc;

use protbot::{AllowList, App, CliError, CliOutput, Config, MapRunner, Settings, call_tool};
use serde_json::json;

fn runner() -> Arc<MapRunner> {
    Arc::new(MapRunner::new(|spec| {
        let args = spec.args.join(" ");
        let stdout = if args.contains("messages") && args.contains("list") {
            r#"{"messages":[{"id":"m1","from":"a@b.co","subject":"Hi","snippet":"line"}]}"#.to_string()
        } else if spec.args.first().is_some_and(|arg| arg == "info") {
            "{}".to_string()
        } else {
            r#"{"ok":true,"id":"item1"}"#.to_string()
        };
        Ok(CliOutput {
            stdout,
            stderr: String::new(),
            code: 0,
        })
    }))
}

fn app(cfg: Config, mock: Arc<MapRunner>) -> App {
    App::for_test(cfg, Settings::default(), mock)
}

#[tokio::test]
async fn send_requires_writes_and_allowlist() {
    let mock = runner();
    let mut cfg = Config::for_test(std::path::Path::new("/tmp"));
    cfg.recipients = AllowList::parse_csv("friend@example.com").unwrap();
    let running = app(cfg, mock.clone());
    let denied = call_tool(
        &running,
        "mail_send",
        json!({"to":["friend@example.com"],"subject":"Hi","body":"secret-body"}),
    )
    .await;
    assert!(denied.is_error);
    assert!(denied.text.contains("ALLOW_WRITES"));
    assert!(mock.calls().is_empty());
    assert!(running.recording().unwrap().sends().is_empty());

    let mock = runner();
    let mut cfg = Config::for_test(std::path::Path::new("/tmp"));
    cfg.allow_writes = true;
    cfg.recipients = AllowList::parse_csv("friend@example.com").unwrap();
    let running = app(cfg, mock.clone());
    let blocked = call_tool(
        &running,
        "mail_send",
        json!({"to":["other@example.com"],"subject":"Hi","body":"secret-body"}),
    )
    .await;
    assert!(blocked.is_error);
    assert!(blocked.text.contains("not allowlisted"));
    assert!(mock.calls().is_empty());
    assert!(running.recording().unwrap().sends().is_empty());

    let mock = runner();
    let mut cfg = Config::for_test(std::path::Path::new("/tmp"));
    cfg.allow_writes = true;
    cfg.recipients = AllowList::parse_csv("friend@example.com").unwrap();
    let running = app(cfg, mock.clone());
    let sent = call_tool(
        &running,
        "mail_send",
        json!({"to":["Friend@Example.com"],"subject":"Hi","body":"secret-body"}),
    )
    .await;
    assert!(!sent.is_error, "{}", sent.text);
    assert!(mock.calls().is_empty());
    assert_eq!(running.recording().unwrap().sends(), vec!["secret-body".to_string()]);
}

#[tokio::test]
async fn list_cache_invalidates_on_write() {
    let mock = runner();
    let mut cfg = Config::for_test(std::path::Path::new("/tmp"));
    cfg.allow_writes = true;
    let app = app(cfg, mock.clone());
    let first = call_tool(&app, "mail_list", json!({})).await;
    assert!(!first.is_error, "{}", first.text);
    let second = call_tool(&app, "mail_list", json!({})).await;
    assert!(!second.is_error);
    let trashed = call_tool(&app, "mail_trash", json!({"id":"m1"})).await;
    assert!(!trashed.is_error, "{}", trashed.text);
    let third = call_tool(&app, "mail_list", json!({})).await;
    assert!(!third.is_error);
    assert_eq!(app.recording().unwrap().list_calls(), 2);
    assert!(mock.calls().is_empty());
}

#[tokio::test]
async fn drive_rejects_traversal_before_cli() {
    let mock = runner();
    let cfg = Config::for_test(std::path::Path::new("/tmp"));
    let app = app(cfg, mock.clone());
    let out = call_tool(&app, "drive_list", json!({"path":"/my-files/../../etc/passwd"})).await;
    assert!(out.is_error);
    assert!(out.text.contains("traversal"));
    assert!(mock.calls().is_empty());
}

#[tokio::test]
async fn pass_create_keeps_password_off_argv_and_sets_reason() {
    let mock = runner();
    let mut cfg = Config::for_test(std::path::Path::new("/tmp"));
    cfg.allow_writes = true;
    let app = app(cfg, mock.clone());
    let out = call_tool(
        &app,
        "pass_item_create_login",
        json!({
            "vault": "Bot",
            "title": "Example",
            "password": "s3cret-password",
            "url": "https://example.com/login"
        }),
    )
    .await;
    assert!(!out.is_error, "{}", out.text);
    let create = mock
        .calls()
        .into_iter()
        .find(|call| call.args.iter().any(|arg| arg == "create"))
        .expect("create call");
    assert!(create.pass_token_set);
    assert!(!create.secret_in_args);
    let reason = create.reason.expect("reason");
    assert!(reason.contains("pass_item_create_login"));
    assert!(!reason.contains("s3cret-password"));
    let stdin = String::from_utf8(create.stdin_data).unwrap();
    assert!(stdin.contains("s3cret-password"));
    assert!(!create.args.iter().any(|arg| arg.contains("s3cret-password")));
}

#[tokio::test]
async fn settings_update_clamps_and_rejects_unknown_fields() {
    let mock = runner();
    let cfg = Config::for_test(std::path::Path::new("/tmp"));
    let app = app(cfg, mock);
    let out = call_tool(&app, "settings_update", json!({"poll_interval_secs": 1})).await;
    assert!(!out.is_error, "{}", out.text);
    assert!(out.text.contains("poll_interval_secs = 30") || out.text.contains("\"poll_interval_secs\":30"));
    let bad = call_tool(&app, "mail_list", json!({"folder":"inbox","extra":true})).await;
    assert!(bad.is_error);
    assert!(bad.text.contains("invalid arguments"));
}

fn drive_runner(accept_legacy: bool, accept_current: bool) -> Arc<MapRunner> {
    Arc::new(MapRunner::new(move |spec| {
        let legacy = spec.args.first().is_some_and(|arg| arg == "--json");
        if (legacy && accept_legacy) || (!legacy && accept_current) {
            Ok(CliOutput {
                stdout: r#"[{"name":"a.txt"}]"#.to_string(),
                stderr: String::new(),
                code: 0,
            })
        } else {
            Err(CliError::Exit {
                bin: spec.bin.clone(),
                code: 1,
                stderr: if legacy {
                    "Command not found: --json filesystem".into()
                } else {
                    "unknown option --json".into()
                },
            })
        }
    }))
}

fn args(list: &[&str]) -> Vec<String> {
    list.iter().map(|arg| arg.to_string()).collect()
}

#[tokio::test]
async fn drive_json_goes_after_subcommand() {
    let mock = drive_runner(false, true);
    let app = app(Config::for_test(std::path::Path::new("/tmp")), mock.clone());
    let listed = call_tool(&app, "drive_list", json!({"path":"/my-files"})).await;
    assert!(!listed.is_error, "{}", listed.text);
    let info = call_tool(&app, "drive_info", json!({"path":"/my-files/a.txt"})).await;
    assert!(!info.is_error, "{}", info.text);
    let calls: Vec<Vec<String>> = mock.calls().into_iter().map(|call| call.args).collect();
    assert_eq!(
        calls,
        vec![
            args(&["filesystem", "list", "--json", "/my-files"]),
            args(&["filesystem", "info", "--json", "/my-files/a.txt"]),
        ]
    );
}

#[tokio::test]
async fn drive_falls_back_to_legacy_json_order_and_remembers_it() {
    let mock = drive_runner(true, false);
    let app = app(Config::for_test(std::path::Path::new("/tmp")), mock.clone());
    let listed = call_tool(&app, "drive_list", json!({"path":"/my-files"})).await;
    assert!(!listed.is_error, "{}", listed.text);
    assert!(listed.text.contains("a.txt"));
    let info = call_tool(&app, "drive_info", json!({"path":"/my-files/a.txt"})).await;
    assert!(!info.is_error, "{}", info.text);
    let calls: Vec<Vec<String>> = mock.calls().into_iter().map(|call| call.args).collect();
    assert_eq!(
        calls,
        vec![
            args(&["filesystem", "list", "--json", "/my-files"]),
            args(&["--json", "filesystem", "list", "/my-files"]),
            args(&["--json", "filesystem", "info", "/my-files/a.txt"]),
        ]
    );
}

#[tokio::test]
async fn drive_reports_first_error_when_both_orders_fail() {
    let mock = drive_runner(false, false);
    let app = app(Config::for_test(std::path::Path::new("/tmp")), mock.clone());
    let out = call_tool(&app, "drive_list", json!({"path":"/my-files"})).await;
    assert!(out.is_error);
    assert!(out.text.contains("unknown option"), "{}", out.text);
    assert_eq!(mock.calls().len(), 2);
    assert!(!app.drive_json_legacy());
}
