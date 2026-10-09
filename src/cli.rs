use crate::redact::redact;
use std::future::Future;
use std::pin::Pin;
use std::process::Stdio;
use std::time::Duration;
use tokio::io::AsyncReadExt;
use tokio::process::Command;
use tokio::time::timeout;

const OUTPUT_CAP: usize = 8 * 1024 * 1024;

#[derive(Clone)]
pub struct CommandSpec {
    pub bin: String,
    pub args: Vec<String>,
    pub stdin_data: Vec<u8>,
    pub env_overrides: Vec<(String, String)>,
    /// When true, secret Proton Pass variables are stripped before overrides are applied.
    pub scrub_secrets: bool,
    pub timeout: Duration,
}

impl std::fmt::Debug for CommandSpec {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("CommandSpec")
            .field("bin", &self.bin)
            .field("args", &self.args)
            .field("stdin_len", &self.stdin_data.len())
            .field("scrub_secrets", &self.scrub_secrets)
            .field("timeout", &self.timeout)
            .finish()
    }
}

#[derive(Clone, Debug)]
pub struct CliOutput {
    pub stdout: String,
    pub stderr: String,
    pub code: i32,
}

#[derive(Debug)]
pub enum CliError {
    Timeout { bin: String },
    Spawn { bin: String, message: String },
    Exit { bin: String, code: i32, stderr: String },
    Message(String),
}

impl std::fmt::Display for CliError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Timeout { bin } => write!(f, "{bin} timed out"),
            Self::Spawn { bin, message } => write!(f, "{bin} failed to start: {message}"),
            Self::Exit { bin, code, stderr } => {
                let stderr = redact(stderr);
                if stderr.is_empty() {
                    write!(f, "{bin} exited {code}")
                } else {
                    write!(f, "{bin} exited {code}: {stderr}")
                }
            }
            Self::Message(message) => write!(f, "{message}"),
        }
    }
}

pub type RunFut<'a> = Pin<Box<dyn Future<Output = Result<CliOutput, CliError>> + Send + 'a>>;

pub trait Runner: Send + Sync {
    fn run<'a>(&'a self, spec: CommandSpec) -> RunFut<'a>;
}

pub struct ProcessRunner;

impl Runner for ProcessRunner {
    fn run<'a>(&'a self, spec: CommandSpec) -> RunFut<'a> {
        Box::pin(run_process(spec))
    }
}

fn secret_env_key(key: &str) -> bool {
    matches!(
        key,
        "PROTON_PASS_AGENT_TOKEN"
            | "PROTON_PASS_PERSONAL_ACCESS_TOKEN"
            | "PROTON_PASS_AGENT_REASON"
            | "PROTON_PASS_PASSWORD"
            | "PROTON_PASS_TOTP"
            | "PROTBOT_CALENDAR_ICS"
    )
}

async fn run_process(spec: CommandSpec) -> Result<CliOutput, CliError> {
    let mut cmd = Command::new(&spec.bin);
    cmd.args(&spec.args)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .kill_on_drop(true);
    #[cfg(unix)]
    {
        cmd.process_group(0);
    }
    if spec.scrub_secrets {
        cmd.env_clear();
        for (key, value) in std::env::vars() {
            if !secret_env_key(&key) {
                cmd.env(key, value);
            }
        }
    }
    for (key, value) in &spec.env_overrides {
        cmd.env(key, value);
    }

    let mut child = cmd.spawn().map_err(|err| CliError::Spawn {
        bin: spec.bin.clone(),
        message: err.to_string(),
    })?;
    if let Some(mut stdin) = child.stdin.take() {
        use tokio::io::AsyncWriteExt;
        let _ = stdin.write_all(&spec.stdin_data).await;
        let _ = stdin.shutdown().await;
    }
    let mut stdout = child
        .stdout
        .take()
        .ok_or_else(|| CliError::Message("missing stdout".into()))?;
    let mut stderr = child
        .stderr
        .take()
        .ok_or_else(|| CliError::Message("missing stderr".into()))?;
    let pid = child.id();
    let joined = timeout(spec.timeout, async move {
        let status_fut = child.wait();
        let out_fut = read_capped(&mut stdout);
        let err_fut = read_capped(&mut stderr);
        tokio::join!(status_fut, out_fut, err_fut)
    })
    .await;
    let (status, stdout, stderr) = match joined {
        Ok((Ok(status), stdout, stderr)) => (status, stdout, stderr),
        Ok((Err(err), _, _)) => {
            return Err(CliError::Spawn {
                bin: spec.bin,
                message: err.to_string(),
            });
        }
        Err(_) => {
            if let Some(pid) = pid {
                kill_group(pid);
            }
            return Err(CliError::Timeout { bin: spec.bin });
        }
    };
    let stdout = String::from_utf8_lossy(&stdout).to_string();
    let stderr = redact(&String::from_utf8_lossy(&stderr));
    let code = status.code().unwrap_or(1);
    if !status.success() {
        return Err(CliError::Exit {
            bin: spec.bin,
            code,
            stderr,
        });
    }
    Ok(CliOutput { stdout, stderr, code })
}

async fn read_capped(reader: &mut (impl AsyncReadExt + Unpin)) -> Vec<u8> {
    let mut buf = Vec::new();
    let mut tmp = [0u8; 8192];
    loop {
        let n = match reader.read(&mut tmp).await {
            Ok(0) | Err(_) => break,
            Ok(n) => n,
        };
        if buf.len() < OUTPUT_CAP {
            let room = OUTPUT_CAP - buf.len();
            buf.extend_from_slice(&tmp[..n.min(room)]);
        }
    }
    buf
}

fn kill_group(pid: u32) {
    #[cfg(unix)]
    unsafe {
        let _ = kill(-(pid as i32), 9);
    }
    #[cfg(not(unix))]
    {
        let _ = pid;
    }
}

#[cfg(unix)]
unsafe extern "C" {
    fn kill(pid: i32, sig: i32) -> i32;
}

#[derive(Clone, Debug)]
pub struct CallRecord {
    pub bin: String,
    pub args: Vec<String>,
    pub stdin_data: Vec<u8>,
    pub reason: Option<String>,
    pub pass_token_set: bool,
    pub secret_in_args: bool,
}

pub struct MapRunner {
    handler: Box<dyn Fn(&CommandSpec) -> Result<CliOutput, CliError> + Send + Sync>,
    pub calls: std::sync::Mutex<Vec<CallRecord>>,
}

impl MapRunner {
    pub fn new(
        handler: impl Fn(&CommandSpec) -> Result<CliOutput, CliError> + Send + Sync + 'static,
    ) -> Self {
        Self {
            handler: Box::new(handler),
            calls: std::sync::Mutex::new(Vec::new()),
        }
    }

    pub fn calls(&self) -> Vec<CallRecord> {
        self.calls.lock().unwrap_or_else(|err| err.into_inner()).clone()
    }
}

impl Runner for MapRunner {
    fn run<'a>(&'a self, spec: CommandSpec) -> RunFut<'a> {
        let secret_in_args = spec.args.iter().any(|arg| {
            arg.contains("pst_")
                || arg.contains("PROTON_PASS_")
                || arg.contains("PROTON_PASS_AGENT_TOKEN")
        });
        let reason = spec
            .env_overrides
            .iter()
            .find(|(key, _)| key == "PROTON_PASS_AGENT_REASON")
            .map(|(_, value)| value.clone());
        let pass_token_set = spec
            .env_overrides
            .iter()
            .any(|(key, value)| key == "PROTON_PASS_PERSONAL_ACCESS_TOKEN" && !value.is_empty());
        self.calls
            .lock()
            .unwrap_or_else(|err| err.into_inner())
            .push(CallRecord {
                bin: spec.bin.clone(),
                args: spec.args.clone(),
                stdin_data: spec.stdin_data.clone(),
                reason,
                pass_token_set,
                secret_in_args,
            });
        let result = (self.handler)(&spec);
        Box::pin(async move { result })
    }
}
