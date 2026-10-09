use std::io::{BufRead, BufReader, Read, Write};
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

fn fake_imap() -> u16 {
    let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
    let port = listener.local_addr().unwrap().port();
    std::thread::spawn(move || {
        for stream in listener.incoming().take(4) {
            let mut stream = match stream {
                Ok(stream) => stream,
                Err(_) => continue,
            };
            let _ = stream.write_all(b"* OK fixture\r\n");
            let mut buf = Vec::new();
            let mut tmp = [0u8; 2048];
            loop {
                let n = stream.read(&mut tmp).unwrap_or(0);
                if n == 0 {
                    break;
                }
                buf.extend_from_slice(&tmp[..n]);
                while let Some(pos) = buf.windows(1).position(|byte| byte[0] == b'\n') {
                    let line = String::from_utf8_lossy(&buf[..=pos]).into_owned();
                    buf.drain(..=pos);
                    let tag = line.split_whitespace().next().unwrap_or("t1");
                    let reply = if line.contains("FETCH") {
                        let body = "From: a@b.co\r\nSubject: Hi\r\n";
                        format!(
                            "* 1 FETCH (UID 1 FLAGS () BODY[HEADER.FIELDS (FROM SUBJECT)] {{{}}}\r\n{body})\r\n{tag} OK\r\n",
                            body.len()
                        )
                    } else if line.contains("CAPABILITY") {
                        format!("* CAPABILITY IMAP4rev1\r\n{tag} OK\r\n")
                    } else {
                        format!("{tag} OK\r\n")
                    };
                    if stream.write_all(reply.as_bytes()).is_err() {
                        break;
                    }
                }
            }
        }
    });
    port
}

#[test]
fn stdio_lists_mail_and_pass_without_leaking_the_token() {
    let port = fake_imap().to_string();
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
        ("PROTON_BRIDGE_PASSWORD", "bridge-fixture"),
        ("PROTBOT_BRIDGE_USER", "bot@proton.me"),
        ("PROTBOT_BRIDGE_PROBE", "0"),
        ("PROTBOT_IMAP_PORT", &port),
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
    assert!(mail.contains("inbox:1"), "{mail}");
    assert!(!mail.contains("NOTREAL"), "{mail}");
    assert!(!mail.contains("bridge-fixture"), "{mail}");
    server.send(r#"{"jsonrpc":"2.0","id":4,"method":"tools/call","params":{"name":"pass_vaults","arguments":{}}}"#);
    let vaults = server.recv();
    assert!(vaults.contains("v1"), "{vaults}");
    assert!(!vaults.contains("NOTREAL"));
    drop(server);
    let logged = std::fs::read_to_string(&log).unwrap_or_default();
    assert!(logged.contains("vault"), "{logged}");
    assert!(!logged.contains("NOTREAL"), "{logged}");
    assert!(!logged.contains("bridge-fixture"), "{logged}");
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
        ("PROTON_BRIDGE_PASSWORD", "bridge-fixture"),
        ("PROTBOT_BRIDGE_USER", "bot@proton.me"),
        ("PROTBOT_BRIDGE_PROBE", "0"),
        ("PROTBOT_PASS_BIN", &script),
        ("PROTBOT_FAKE_SLEEP", "1"),
        ("PROTBOT_FAKE_PID", pid_file.to_str().unwrap()),
    ]);
    server.send(r#"{"jsonrpc":"2.0","id":1,"method":"initialize","params":{"protocolVersion":"2025-06-18","capabilities":{}}}"#);
    let _ = server.recv();
    server.send(r#"{"jsonrpc":"2.0","method":"notifications/initialized"}"#);
    let started = std::time::Instant::now();
    server.send(r#"{"jsonrpc":"2.0","id":2,"method":"tools/call","params":{"name":"pass_vaults","arguments":{}}}"#);
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

#[test]
fn non_loopback_bridge_host_exits() {
    let dir = temp_dir("host");
    let output = std::process::Command::new(env!("CARGO_BIN_EXE_protbot"))
        .env("PROTON_PASS_AGENT_TOKEN", "pst_host_fixture::NOTREAL")
        .env("PROTON_BRIDGE_PASSWORD", "bridge-fixture")
        .env("PROTBOT_BRIDGE_USER", "bot@proton.me")
        .env("PROTBOT_BRIDGE_HOST", "192.0.2.1")
        .env("PROTBOT_SETTINGS", dir.join("settings.toml"))
        .env("XDG_RUNTIME_DIR", &dir)
        .env("PROTBOT_DISABLE_WATCHER", "1")
        .output()
        .unwrap();
    assert!(!output.status.success());
    let err = String::from_utf8_lossy(&output.stderr);
    assert!(err.contains("loopback"), "{err}");
    assert!(!err.contains("bridge-fixture"), "{err}");
    assert!(!err.contains("NOTREAL"), "{err}");
    let _ = std::fs::remove_dir_all(&dir);
}
