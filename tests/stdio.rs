use std::io::{BufRead, BufReader, Write};
use std::process::{Command, Stdio};
use std::time::Duration;

fn temp_dir(name: &str) -> std::path::PathBuf {
    let dir = std::env::temp_dir().join(format!("protbot-{name}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    dir
}

struct Server {
    child: std::process::Child,
    lines: std::sync::mpsc::Receiver<String>,
    stderr: std::sync::mpsc::Receiver<String>,
}

fn spawn(env: &[(&str, &str)]) -> Server {
    let mut cmd = Command::new(env!("CARGO_BIN_EXE_protbot"));
    cmd.stdin(Stdio::piped()).stdout(Stdio::piped()).stderr(Stdio::piped()).env_remove("PROTON_PASS_AGENT_TOKEN");
    for (key, value) in env {
        cmd.env(key, value);
    }
    let mut child = cmd.spawn().unwrap();
    let stdout = child.stdout.take().unwrap();
    let stderr = child.stderr.take().unwrap();
    let (out_tx, lines) = std::sync::mpsc::channel();
    std::thread::spawn(move || {
        for line in BufReader::new(stdout).lines() {
            match line {
                Ok(line) => {
                    if out_tx.send(line).is_err() {
                        break;
                    }
                }
                Err(_) => break,
            }
        }
    });
    let (err_tx, stderr_rx) = std::sync::mpsc::channel();
    std::thread::spawn(move || {
        for line in BufReader::new(stderr).lines() {
            match line {
                Ok(line) => {
                    if err_tx.send(line).is_err() {
                        break;
                    }
                }
                Err(_) => break,
            }
        }
    });
    Server {
        child,
        lines,
        stderr: stderr_rx,
    }
}

impl Server {
    fn send(&mut self, line: &str) {
        let stdin = self.child.stdin.as_mut().unwrap();
        stdin.write_all(line.as_bytes()).unwrap();
        stdin.write_all(b"\n").unwrap();
        stdin.flush().unwrap();
    }

    fn recv(&self) -> String {
        self.lines.recv_timeout(Duration::from_secs(8)).expect("stdout line")
    }
}

impl Drop for Server {
    fn drop(&mut self) {
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
}

#[test]
fn stdio_lists_mail_and_pass_without_leaking_the_token() {
    let dir = temp_dir("stdio");
    let log = dir.join("cli.log");
    let script = format!("{}/tests/fixtures/fake-cli.sh", env!("CARGO_MANIFEST_DIR"));
    std::fs::set_permissions(&script, std::os::unix::fs::PermissionsExt::from_mode(0o755)).unwrap();
    let token = "pst_stdio_fixture::NOTREAL";
    let mut server = spawn(&[
        ("PROTON_PASS_AGENT_TOKEN", token),
        ("PROTBOT_DISABLE_WATCHER", "1"),
        ("PROTBOT_SETTINGS", dir.join("settings.toml").to_str().unwrap()),
        ("XDG_RUNTIME_DIR", dir.to_str().unwrap()),
        ("PROTBOT_MAIL_BIN", &script),
        ("PROTBOT_PASS_BIN", &script),
        ("PROTBOT_DRIVE_BIN", &script),
        ("PROTBOT_CURL_BIN", &script),
        ("PROTBOT_FAKE_LOG", log.to_str().unwrap()),
    ]);
    server.send(r#"{"jsonrpc":"2.0","id":1,"method":"initialize","params":{"protocolVersion":"2025-06-18","capabilities":{},"clientInfo":{"name":"test","version":"0"}}}"#);
    let init = server.recv();
    assert!(init.contains("protbot"), "{init}");
    server.send(r#"{"jsonrpc":"2.0","method":"notifications/initialized"}"#);
    server.send(r#"{"jsonrpc":"2.0","id":2,"method":"tools/list"}"#);
    let tools = server.recv();
    assert!(tools.contains("mail_list"), "{tools}");
    assert!(tools.contains("pass_vaults"));
    server.send(r#"{"jsonrpc":"2.0","id":3,"method":"tools/call","params":{"name":"mail_list","arguments":{}}}"#);
    let mail = server.recv();
    assert!(mail.contains("m1"), "{mail}");
    assert!(!mail.contains("NOTREAL"), "{mail}");
    server.send(r#"{"jsonrpc":"2.0","id":4,"method":"tools/call","params":{"name":"pass_vaults","arguments":{}}}"#);
    let vaults = server.recv();
    assert!(vaults.contains("v1"), "{vaults}");
    assert!(!vaults.contains("NOTREAL"));
    drop(server);
    let logged = std::fs::read_to_string(&log).unwrap_or_default();
    assert!(logged.contains("messages list"), "{logged}");
    assert!(!logged.contains("NOTREAL"), "{logged}");
    assert!(!logged.contains("pst_stdio_fixture"), "{logged}");
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn hung_cli_is_killed() {
    let dir = temp_dir("timeout");
    let pid_file = dir.join("sleep.pid");
    let script = format!("{}/tests/fixtures/fake-cli.sh", env!("CARGO_MANIFEST_DIR"));
    std::fs::set_permissions(&script, std::os::unix::fs::PermissionsExt::from_mode(0o755)).unwrap();
    let mut server = spawn(&[
        ("PROTON_PASS_AGENT_TOKEN", "pst_timeout_fixture::NOTREAL"),
        ("PROTBOT_DISABLE_WATCHER", "1"),
        ("PROTBOT_TOOL_TIMEOUT_SECS", "1"),
        ("PROTBOT_SETTINGS", dir.join("settings.toml").to_str().unwrap()),
        ("XDG_RUNTIME_DIR", dir.to_str().unwrap()),
        ("PROTBOT_MAIL_BIN", &script),
        ("PROTBOT_FAKE_SLEEP", "1"),
        ("PROTBOT_FAKE_PID", pid_file.to_str().unwrap()),
    ]);
    server.send(r#"{"jsonrpc":"2.0","id":1,"method":"initialize","params":{"protocolVersion":"2025-06-18","capabilities":{}}}"#);
    let _ = server.recv();
    server.send(r#"{"jsonrpc":"2.0","method":"notifications/initialized"}"#);
    let started = std::time::Instant::now();
    server.send(r#"{"jsonrpc":"2.0","id":2,"method":"tools/call","params":{"name":"mail_list","arguments":{}}}"#);
    let response = server.recv();
    assert!(started.elapsed() < Duration::from_secs(8), "kill did not return");
    assert!(response.contains("timed out") || response.contains("true"), "{response}");
    let pid = std::fs::read_to_string(&pid_file).unwrap_or_default();
    let pid = pid.trim();
    if !pid.is_empty() {
        std::thread::sleep(Duration::from_millis(200));
        let alive = std::path::Path::new(&format!("/proc/{pid}")).exists();
        assert!(!alive, "sleeping cli still alive: {pid}");
    }
    let mut leaked = false;
    while let Ok(line) = server.stderr.try_recv() {
        if line.contains("NOTREAL") {
            leaked = true;
        }
    }
    assert!(!leaked);
    drop(server);
    let _ = std::fs::remove_dir_all(&dir);
}
