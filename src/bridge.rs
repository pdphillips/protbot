use crate::mail::MailHeader;
use crate::policy::{normalize_email, one_line};
use std::net::{IpAddr, Ipv4Addr, Ipv6Addr};
use std::sync::{Arc, Mutex};
use std::time::Duration;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::TcpStream;
use tokio::time::timeout;
use tokio_rustls::client::TlsStream;
use tokio_rustls::TlsConnector;

const NO_PROMPT: &str = "Protbot will not prompt and will not invent a password.";

pub fn check_loopback(host: &str) -> Result<(), String> {
    match host.trim().to_ascii_lowercase().as_str() {
        "127.0.0.1" | "localhost" | "::1" => Ok(()),
        _ => Err(
            "bridge host is not loopback; Protbot only connects to 127.0.0.1, localhost, or ::1"
                .into(),
        ),
    }
}

pub fn missing_secret(what: &str, pass_name: &str) -> String {
    format!(
        "{what} is unset. Store it with pass at {pass_name} and start through scripts/spawn.sh. A restored Grok Bot computer wipes that session. {NO_PROMPT}"
    )
}

#[derive(Clone)]
pub struct RecordingMailer {
    inner: Arc<Mutex<Rec>>,
}

struct Rec {
    lists: u64,
    sends: Vec<String>,
}

impl Default for RecordingMailer {
    fn default() -> Self {
        Self {
            inner: Arc::new(Mutex::new(Rec {
                lists: 0,
                sends: Vec::new(),
            })),
        }
    }
}

impl RecordingMailer {
    pub fn list_calls(&self) -> u64 {
        self.inner.lock().unwrap_or_else(|err| err.into_inner()).lists
    }

    pub fn sends(&self) -> Vec<String> {
        self.inner.lock().unwrap_or_else(|err| err.into_inner()).sends.clone()
    }

    async fn list(&self, folder: &str, _unread_only: bool) -> Result<Vec<MailHeader>, String> {
        self.inner.lock().unwrap_or_else(|err| err.into_inner()).lists += 1;
        Ok(vec![MailHeader {
            id: format!("{folder}:1"),
            from: "a@b.co".into(),
            subject: "Hi".into(),
            summary: "line".into(),
            unread: true,
        }])
    }

    async fn read(&self, _id: &str) -> Result<String, String> {
        Ok("hello".into())
    }

    async fn send(&self, body: &str) -> Result<String, String> {
        self.inner
            .lock()
            .unwrap_or_else(|err| err.into_inner())
            .sends
            .push(body.to_string());
        Ok(r#"{"ok":true}"#.into())
    }

    async fn ok(&self) -> Result<String, String> {
        Ok(r#"{"ok":true}"#.into())
    }
}

pub enum Mailer {
    Record(RecordingMailer),
    Bridge(BridgeMailer),
}

impl Mailer {
    pub async fn list(&self, folder: &str, unread_only: bool) -> Result<Vec<MailHeader>, String> {
        match self {
            Self::Record(mail) => mail.list(folder, unread_only).await,
            Self::Bridge(mail) => mail.list(folder, unread_only).await,
        }
    }

    pub async fn read(&self, id: &str) -> Result<String, String> {
        match self {
            Self::Record(mail) => mail.read(id).await,
            Self::Bridge(mail) => mail.read(id).await,
        }
    }

    pub async fn send(&self, message: &Outbound) -> Result<String, String> {
        match self {
            Self::Record(mail) => mail.send(&message.body).await,
            Self::Bridge(mail) => mail.send(message).await,
        }
    }

    pub async fn trash(&self, id: &str) -> Result<String, String> {
        match self {
            Self::Record(mail) => mail.ok().await,
            Self::Bridge(mail) => mail.store_flag(id, "Trash").await,
        }
    }

    pub async fn delete(&self, id: &str) -> Result<String, String> {
        match self {
            Self::Record(mail) => mail.ok().await,
            Self::Bridge(mail) => mail.delete(id).await,
        }
    }

    pub async fn move_msg(&self, id: &str, dest: &str) -> Result<String, String> {
        match self {
            Self::Record(mail) => mail.ok().await,
            Self::Bridge(mail) => mail.move_msg(id, dest).await,
        }
    }

    pub async fn folders(&self) -> Result<String, String> {
        match self {
            Self::Record(_) => Ok(r#"{"folders":["INBOX"]}"#.into()),
            Self::Bridge(mail) => mail.folders().await,
        }
    }

    pub async fn whoami(&self) -> Result<String, String> {
        match self {
            Self::Record(_) => Ok(r#"{"address":"bot@proton.me"}"#.into()),
            Self::Bridge(mail) => mail.whoami().await,
        }
    }

    pub fn recording(&self) -> Option<RecordingMailer> {
        match self {
            Self::Record(mail) => Some(mail.clone()),
            Self::Bridge(_) => None,
        }
    }
}

pub struct Outbound {
    pub to: Vec<String>,
    pub cc: Vec<String>,
    pub bcc: Vec<String>,
    pub subject: String,
    pub body: String,
}

pub struct BridgeMailer {
    host: String,
    imap_port: u16,
    smtp_port: u16,
    user: String,
    password: String,
    timeout: Duration,
}

impl std::fmt::Debug for BridgeMailer {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("BridgeMailer")
            .field("host", &self.host)
            .field("imap_port", &self.imap_port)
            .field("smtp_port", &self.smtp_port)
            .field("user", &self.user)
            .finish()
    }
}

impl BridgeMailer {
    pub fn new(
        host: String,
        imap_port: u16,
        smtp_port: u16,
        user: String,
        password: String,
        timeout: Duration,
    ) -> Result<Self, String> {
        check_loopback(&host)?;
        if password.is_empty() {
            return Err(missing_secret(
                "PROTON_BRIDGE_PASSWORD",
                "protbot/bridge-password",
            ));
        }
        Ok(Self {
            host,
            imap_port,
            smtp_port,
            user,
            password,
            timeout,
        })
    }

    pub async fn probe(&self) -> Result<(), String> {
        let mut imap = self.imap().await?;
        imap.logout().await;
        Ok(())
    }

    async fn list(&self, folder: &str, unread_only: bool) -> Result<Vec<MailHeader>, String> {
        let mailbox = mailbox_name(folder)?;
        let mut imap = self.imap().await?;
        imap.cmd(&format!("SELECT {mailbox}")).await?;
        let blob = imap
            .cmd("UID FETCH 1:* (UID FLAGS BODY.PEEK[HEADER.FIELDS (FROM SUBJECT)])")
            .await?;
        imap.logout().await;
        let mut headers = parse_fetches(&blob, folder);
        if unread_only {
            headers.retain(|header| header.unread);
        }
        if headers.len() > 200 {
            let skip = headers.len() - 200;
            headers = headers.split_off(skip);
        }
        Ok(headers)
    }

    async fn read(&self, id: &str) -> Result<String, String> {
        let (folder, uid) = split_id(id)?;
        let mailbox = mailbox_name(&folder)?;
        let mut imap = self.imap().await?;
        imap.cmd(&format!("SELECT {mailbox}")).await?;
        let blob = imap.cmd(&format!("UID FETCH {uid} (BODY.PEEK[])")).await?;
        imap.logout().await;
        Ok(extract_body(&blob))
    }

    async fn store_flag(&self, id: &str, dest: &str) -> Result<String, String> {
        self.transfer(id, dest).await
    }

    async fn delete(&self, id: &str) -> Result<String, String> {
        let (folder, uid) = split_id(id)?;
        let mailbox = mailbox_name(&folder)?;
        let mut imap = self.imap().await?;
        imap.cmd(&format!("SELECT {mailbox}")).await?;
        imap.cmd(&format!("UID STORE {uid} +FLAGS.SILENT (\\Deleted)"))
            .await?;
        imap.cmd(&format!("UID EXPUNGE {uid}")).await?;
        imap.logout().await;
        Ok(r#"{"ok":true}"#.into())
    }

    async fn move_msg(&self, id: &str, dest: &str) -> Result<String, String> {
        self.transfer(id, dest).await
    }

    async fn transfer(&self, id: &str, dest: &str) -> Result<String, String> {
        let (folder, uid) = split_id(id)?;
        let mailbox = mailbox_name(&folder)?;
        let dest_box = mailbox_name(dest)?;
        let mut imap = self.imap().await?;
        imap.cmd(&format!("SELECT {mailbox}")).await?;
        let moved = imap.cmd(&format!("UID MOVE {uid} {dest_box}")).await;
        if moved.is_err() {
            imap.cmd(&format!("UID COPY {uid} {dest_box}")).await?;
            imap.cmd(&format!("UID STORE {uid} +FLAGS.SILENT (\\Deleted)"))
                .await?;
            imap.cmd(&format!("UID EXPUNGE {uid}")).await?;
        }
        imap.logout().await;
        Ok(r#"{"ok":true}"#.into())
    }

    async fn folders(&self) -> Result<String, String> {
        let mut imap = self.imap().await?;
        let blob = imap.cmd(r#"LIST "" "*""#).await?;
        imap.logout().await;
        let names = parse_list(&blob);
        serde_json::to_string(&serde_json::json!({"folders": names}))
            .map_err(|err| err.to_string())
    }

    async fn whoami(&self) -> Result<String, String> {
        let mut imap = self.imap().await?;
        imap.logout().await;
        Ok(serde_json::json!({"address": self.user, "bridge": self.host}).to_string())
    }

    async fn send(&self, message: &Outbound) -> Result<String, String> {
        let mut smtp = Smtp::connect(self).await?;
        smtp.send(self, message).await?;
        Ok(r#"{"ok":true}"#.into())
    }

    async fn imap(&self) -> Result<Imap, String> {
        let host = self.host.clone();
        let port = self.imap_port;
        timeout(self.timeout, Imap::connect(self))
            .await
            .map_err(|_| bridge_down(&host, port))?
    }
}

fn bridge_down(host: &str, port: u16) -> String {
    format!(
        "Proton Mail Bridge is not signed in on {host}:{port}. Sign in on this machine with the bot account password and 2FA. {NO_PROMPT} A restored Grok Bot computer does not keep that session."
    )
}

fn mailbox_name(folder: &str) -> Result<String, String> {
    crate::mail::check_folder(folder)?;
    let name = if folder.eq_ignore_ascii_case("inbox") {
        "INBOX".to_string()
    } else {
        folder.to_string()
    };
    imap_quote(&name)
}

fn split_id(id: &str) -> Result<(String, String), String> {
    let Some((folder, uid)) = id.rsplit_once(':') else {
        return Err("id is invalid".into());
    };
    if folder.is_empty() || !uid.chars().all(|ch| ch.is_ascii_digit()) || uid.is_empty() {
        return Err("id is invalid".into());
    }
    crate::mail::check_folder(folder)?;
    Ok((folder.to_string(), uid.to_string()))
}

fn imap_quote(value: &str) -> Result<String, String> {
    if value.chars().any(|ch| ch == '\0' || ch == '\r' || ch == '\n') {
        return Err("value is invalid".into());
    }
    Ok(format!(
        "\"{}\"",
        value.replace('\\', "\\\\").replace('"', "\\\"")
    ))
}

enum Sock {
    Empty,
    Tcp(TcpStream),
    Tls(TlsStream<TcpStream>),
}

impl Sock {
    async fn write_all(&mut self, bytes: &[u8]) -> Result<(), String> {
        let result = match self {
            Self::Empty => return Err("bridge connection failed".into()),
            Self::Tcp(stream) => stream.write_all(bytes).await,
            Self::Tls(stream) => stream.write_all(bytes).await,
        };
        result.map_err(|_| "bridge connection failed".to_string())
    }

    async fn read(&mut self, buf: &mut [u8]) -> Result<usize, String> {
        let result = match self {
            Self::Empty => return Err("bridge connection failed".into()),
            Self::Tcp(stream) => stream.read(buf).await,
            Self::Tls(stream) => stream.read(buf).await,
        };
        result.map_err(|_| "bridge connection failed".to_string())
    }
}

struct Imap {
    sock: Sock,
    buf: Vec<u8>,
    tag: u32,
    password: String,
}

impl Imap {
    async fn connect(mail: &BridgeMailer) -> Result<Self, String> {
        check_loopback(&mail.host)?;
        let addr = format!("{}:{}", mail.host, mail.imap_port);
        let stream = TcpStream::connect(&addr)
            .await
            .map_err(|_| bridge_down(&mail.host, mail.imap_port))?;
        stream
            .set_nodelay(true)
            .map_err(|_| "bridge connection failed".to_string())?;
        let mut imap = Self {
            sock: Sock::Tcp(stream),
            buf: Vec::new(),
            tag: 0,
            password: mail.password.clone(),
        };
        let greeting = imap.read_line().await?;
        if !greeting.to_ascii_uppercase().contains("OK") {
            return Err(bridge_down(&mail.host, mail.imap_port));
        }
        let caps = imap.cmd("CAPABILITY").await?;
        if caps.to_ascii_uppercase().contains("STARTTLS") {
            imap.cmd("STARTTLS").await?;
            imap.upgrade(&mail.host).await?;
        }
        let user = imap_quote(&mail.user)?;
        let pass = imap_quote(&mail.password)?;
        let login = imap.cmd(&format!("LOGIN {user} {pass}")).await;
        drop(pass);
        login.map_err(|err| hide_secret(&err, &mail.password))?;
        Ok(imap)
    }

    async fn upgrade(&mut self, host: &str) -> Result<(), String> {
        let stream = match std::mem::replace(&mut self.sock, Sock::Empty) {
            Sock::Tcp(stream) => stream,
            _ => return Err("bridge connection failed".into()),
        };
        if !self.buf.is_empty() {
            return Err("bridge connection failed".into());
        }
        self.upgrade_with(host, stream).await
    }

    async fn cmd(&mut self, command: &str) -> Result<String, String> {
        self.tag += 1;
        let tag = format!("t{}", self.tag);
        let line = format!("{tag} {command}\r\n");
        self.sock.write_all(line.as_bytes()).await?;
        let blob = self.read_tagged(&tag).await?;
        Ok(hide_secret(&blob, &self.password))
    }

    async fn logout(&mut self) {
        let _ = self.cmd("LOGOUT").await;
    }

    async fn read_line(&mut self) -> Result<String, String> {
        loop {
            if let Some(pos) = self.buf.iter().position(|byte| *byte == b'\n') {
                let mut line: Vec<u8> = self.buf.drain(..=pos).collect();
                if line.last() == Some(&b'\n') {
                    line.pop();
                }
                if line.last() == Some(&b'\r') {
                    line.pop();
                }
                return Ok(String::from_utf8_lossy(&line).into_owned());
            }
            let mut tmp = [0u8; 4096];
            let n = self.sock.read(&mut tmp).await?;
            if n == 0 {
                return Err("bridge closed the connection".into());
            }
            self.buf.extend_from_slice(&tmp[..n]);
            if self.buf.len() > 8 * 1024 * 1024 {
                return Err("bridge response is too large".into());
            }
        }
    }

    async fn read_exact(&mut self, count: usize) -> Result<Vec<u8>, String> {
        while self.buf.len() < count {
            let mut tmp = [0u8; 4096];
            let n = self.sock.read(&mut tmp).await?;
            if n == 0 {
                return Err("bridge closed the connection".into());
            }
            self.buf.extend_from_slice(&tmp[..n]);
            if self.buf.len() > 8 * 1024 * 1024 {
                return Err("bridge response is too large".into());
            }
        }
        Ok(self.buf.drain(..count).collect())
    }

    async fn read_tagged(&mut self, tag: &str) -> Result<String, String> {
        let mut all = String::new();
        loop {
            let line = self.read_line().await?;
            let literal = literal_len(&line);
            all.push_str(&line);
            all.push('\n');
            if let Some(count) = literal {
                let bytes = self.read_exact(count).await?;
                all.push_str(&String::from_utf8_lossy(&bytes));
            }
            if line.starts_with(tag) && line.as_bytes().get(tag.len()) == Some(&b' ') {
                if line.to_ascii_uppercase().contains(" OK") {
                    return Ok(all);
                }
                return Err("bridge rejected the command".into());
            }
        }
    }

    async fn upgrade_with(&mut self, host: &str, stream: TcpStream) -> Result<(), String> {
        let name = server_name(host)?;
        let connector = TlsConnector::from(tls_config()?);
        let tls = connector
            .connect(name, stream)
            .await
            .map_err(|_| "bridge connection failed".to_string())?;
        self.sock = Sock::Tls(tls);
        self.buf.clear();
        Ok(())
    }
}

fn literal_len(line: &str) -> Option<usize> {
    let start = line.rfind('{')?;
    let end = line.rfind('}')?;
    if end <= start || end + 1 != line.len() {
        return None;
    }
    line[start + 1..end].parse().ok()
}

fn hide_secret(text: &str, secret: &str) -> String {
    if secret.is_empty() {
        return text.to_string();
    }
    text.replace(secret, "[redacted]")
}

fn server_name(host: &str) -> Result<rustls::pki_types::ServerName<'static>, String> {
    let name = match host {
        "127.0.0.1" => rustls::pki_types::ServerName::IpAddress(IpAddr::V4(Ipv4Addr::LOCALHOST).into()),
        "::1" => rustls::pki_types::ServerName::IpAddress(IpAddr::V6(Ipv6Addr::LOCALHOST).into()),
        "localhost" => rustls::pki_types::ServerName::try_from("localhost")
            .map_err(|_| "bridge host is not loopback".to_string())?
            .to_owned(),
        _ => return Err("bridge host is not loopback".into()),
    };
    Ok(name)
}

fn tls_config() -> Result<Arc<rustls::ClientConfig>, String> {
    let provider = Arc::new(rustls::crypto::ring::default_provider());
    let config = rustls::ClientConfig::builder_with_provider(provider)
        .with_protocol_versions(&[&rustls::version::TLS12, &rustls::version::TLS13])
        .map_err(|_| "bridge connection failed".to_string())?
        .dangerous()
        .with_custom_certificate_verifier(Arc::new(LoopbackCerts))
        .with_no_client_auth();
    Ok(Arc::new(config))
}

#[derive(Debug)]
struct LoopbackCerts;

impl rustls::client::danger::ServerCertVerifier for LoopbackCerts {
    fn verify_server_cert(
        &self,
        _end_entity: &rustls::pki_types::CertificateDer<'_>,
        _intermediates: &[rustls::pki_types::CertificateDer<'_>],
        _server_name: &rustls::pki_types::ServerName<'_>,
        _ocsp_response: &[u8],
        _now: rustls::pki_types::UnixTime,
    ) -> Result<rustls::client::danger::ServerCertVerified, rustls::Error> {
        Ok(rustls::client::danger::ServerCertVerified::assertion())
    }

    fn verify_tls12_signature(
        &self,
        message: &[u8],
        cert: &rustls::pki_types::CertificateDer<'_>,
        dss: &rustls::DigitallySignedStruct,
    ) -> Result<rustls::client::danger::HandshakeSignatureValid, rustls::Error> {
        rustls::crypto::verify_tls12_signature(
            message,
            cert,
            dss,
            &rustls::crypto::ring::default_provider().signature_verification_algorithms,
        )
    }

    fn verify_tls13_signature(
        &self,
        message: &[u8],
        cert: &rustls::pki_types::CertificateDer<'_>,
        dss: &rustls::DigitallySignedStruct,
    ) -> Result<rustls::client::danger::HandshakeSignatureValid, rustls::Error> {
        rustls::crypto::verify_tls13_signature(
            message,
            cert,
            dss,
            &rustls::crypto::ring::default_provider().signature_verification_algorithms,
        )
    }

    fn supported_verify_schemes(&self) -> Vec<rustls::SignatureScheme> {
        rustls::crypto::ring::default_provider()
            .signature_verification_algorithms
            .supported_schemes()
    }
}

struct Smtp {
    sock: Sock,
    buf: Vec<u8>,
}

impl Smtp {
    async fn connect(mail: &BridgeMailer) -> Result<Self, String> {
        check_loopback(&mail.host)?;
        let addr = format!("{}:{}", mail.host, mail.smtp_port);
        let stream = timeout(mail.timeout, TcpStream::connect(&addr))
            .await
            .map_err(|_| bridge_down(&mail.host, mail.smtp_port))?
            .map_err(|_| bridge_down(&mail.host, mail.smtp_port))?;
        let mut smtp = Self {
            sock: Sock::Tcp(stream),
            buf: Vec::new(),
        };
        smtp.read_reply().await?;
        let ehlo = smtp.cmd("EHLO protbot").await?;
        if ehlo.to_ascii_uppercase().contains("STARTTLS") {
            smtp.cmd("STARTTLS").await?;
            let stream = match std::mem::replace(&mut smtp.sock, Sock::Empty) {
                Sock::Tcp(stream) => stream,
                _ => return Err("bridge connection failed".into()),
            };
            let connector = TlsConnector::from(tls_config()?);
            let tls = connector
                .connect(server_name(&mail.host)?, stream)
                .await
                .map_err(|_| "bridge connection failed".to_string())?;
            smtp.sock = Sock::Tls(tls);
            smtp.buf.clear();
            smtp.cmd("EHLO protbot").await?;
        }
        let token = base64_encode(format!("\0{}\0{}", mail.user, mail.password).as_bytes());
        let auth = smtp.cmd(&format!("AUTH PLAIN {token}")).await;
        drop(token);
        auth.map_err(|err| hide_secret(&err, &mail.password))?;
        Ok(smtp)
    }

    async fn send(&mut self, mail: &BridgeMailer, message: &Outbound) -> Result<(), String> {
        let from = normalize_email(&mail.user)?;
        smtp_ok(self.cmd(&format!("MAIL FROM:<{from}>")).await?)?;
        for addr in message.to.iter().chain(message.cc.iter()).chain(message.bcc.iter()) {
            smtp_ok(self.cmd(&format!("RCPT TO:<{addr}>")).await?)?;
        }
        smtp_ok(self.cmd("DATA").await?)?;
        let mut data = format!(
            "From: <{from}>\r\nTo: {}\r\n",
            message.to.join(", ")
        );
        if !message.cc.is_empty() {
            data.push_str(&format!("Cc: {}\r\n", message.cc.join(", ")));
        }
        data.push_str(&format!(
            "Subject: {}\r\nMIME-Version: 1.0\r\nContent-Type: text/plain; charset=utf-8\r\n\r\n",
            message.subject
        ));
        for line in message.body.split('\n') {
            if line.starts_with('.') {
                data.push('.');
            }
            data.push_str(line.trim_end_matches('\r'));
            data.push_str("\r\n");
        }
        data.push_str(".\r\n");
        self.sock.write_all(data.as_bytes()).await?;
        let reply = self.read_reply().await?;
        smtp_ok(reply)?;
        let _ = self.cmd("QUIT").await;
        Ok(())
    }

    async fn cmd(&mut self, command: &str) -> Result<String, String> {
        self.sock.write_all(format!("{command}\r\n").as_bytes()).await?;
        self.read_reply().await
    }

    async fn read_reply(&mut self) -> Result<String, String> {
        let mut all = String::new();
        loop {
            let line = self.read_line().await?;
            let done = line.len() < 4 || line.as_bytes()[3] == b' ';
            all.push_str(&line);
            all.push('\n');
            if done {
                return Ok(all);
            }
        }
    }

    async fn read_line(&mut self) -> Result<String, String> {
        loop {
            if let Some(pos) = self.buf.iter().position(|byte| *byte == b'\n') {
                let mut line: Vec<u8> = self.buf.drain(..=pos).collect();
                if line.last() == Some(&b'\n') {
                    line.pop();
                }
                if line.last() == Some(&b'\r') {
                    line.pop();
                }
                return Ok(String::from_utf8_lossy(&line).into_owned());
            }
            let mut tmp = [0u8; 2048];
            let n = self.sock.read(&mut tmp).await?;
            if n == 0 {
                return Err("bridge closed the connection".into());
            }
            self.buf.extend_from_slice(&tmp[..n]);
        }
    }
}

fn smtp_ok(line: String) -> Result<String, String> {
    if line.starts_with('2') || line.starts_with('3') {
        Ok(line)
    } else {
        Err("bridge rejected the message".into())
    }
}

fn base64_encode(bytes: &[u8]) -> String {
    const TABLE: &[u8] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
    let mut out = String::new();
    let mut i = 0;
    while i + 3 <= bytes.len() {
        let n = ((bytes[i] as u32) << 16) | ((bytes[i + 1] as u32) << 8) | bytes[i + 2] as u32;
        out.push(TABLE[((n >> 18) & 63) as usize] as char);
        out.push(TABLE[((n >> 12) & 63) as usize] as char);
        out.push(TABLE[((n >> 6) & 63) as usize] as char);
        out.push(TABLE[(n & 63) as usize] as char);
        i += 3;
    }
    if i < bytes.len() {
        let mut n = (bytes[i] as u32) << 16;
        out.push(TABLE[((n >> 18) & 63) as usize] as char);
        if i + 1 < bytes.len() {
            n |= (bytes[i + 1] as u32) << 8;
            out.push(TABLE[((n >> 12) & 63) as usize] as char);
            out.push(TABLE[((n >> 6) & 63) as usize] as char);
            out.push('=');
        } else {
            out.push(TABLE[((n >> 12) & 63) as usize] as char);
            out.push('=');
            out.push('=');
        }
    }
    out
}

fn parse_fetches(blob: &str, folder: &str) -> Vec<MailHeader> {
    let mut headers = Vec::new();
    for chunk in blob.split("\n* ").filter(|chunk| chunk.contains("FETCH")) {
        let Some(uid) = token_after(chunk, "UID ") else {
            continue;
        };
        if !uid.chars().all(|ch| ch.is_ascii_digit()) {
            continue;
        }
        let unread = !chunk.contains("\\Seen");
        let from = header_value(chunk, "From")
            .map(|value| address_of(&value))
            .unwrap_or_default();
        let subject = one_line(&header_value(chunk, "Subject").unwrap_or_default(), 200);
        let summary = one_line(&subject, 160);
        headers.push(MailHeader {
            id: format!("{folder}:{uid}"),
            from,
            subject,
            summary,
            unread,
        });
    }
    headers
}

fn token_after(text: &str, key: &str) -> Option<String> {
    let rest = text.split(key).nth(1)?;
    let token = rest.split_whitespace().next()?;
    Some(token.trim_matches(|ch: char| ch == ')' || ch == '(').to_string())
}

fn header_value(chunk: &str, name: &str) -> Option<String> {
    let prefix = format!("{name}:");
    for line in chunk.lines() {
        if line.to_ascii_lowercase().starts_with(&prefix.to_ascii_lowercase()) {
            return Some(line[prefix.len()..].trim().to_string());
        }
    }
    None
}

fn address_of(value: &str) -> String {
    if let Some(start) = value.rfind('<') {
        if let Some(end) = value[start + 1..].find('>') {
            return normalize_email(&value[start + 1..start + 1 + end]).unwrap_or_default();
        }
    }
    normalize_email(value).unwrap_or_default()
}

fn extract_body(blob: &str) -> String {
    if let Some(start) = blob.find("\r\n\r\n") {
        let mut body = blob[start + 4..].to_string();
        if let Some(end) = body.rfind("\nt") {
            body.truncate(end);
        }
        return body;
    }
    blob.to_string()
}

fn parse_list(blob: &str) -> Vec<String> {
    let mut names = Vec::new();
    for line in blob.lines() {
        if !line.contains("LIST") {
            continue;
        }
        if let Some(start) = line.rfind('"') {
            let head = &line[..start];
            if let Some(open) = head.rfind('"') {
                let name = head[open + 1..start].replace("\\\"", "\"");
                if !name.is_empty() {
                    names.push(name);
                }
            }
        }
    }
    names
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rejects_non_loopback_hosts() {
        assert!(check_loopback("127.0.0.1").is_ok());
        assert!(check_loopback("localhost").is_ok());
        assert!(check_loopback("::1").is_ok());
        for host in ["10.0.0.1", "192.0.2.1", "proton.me", "127.0.0.1.evil", "", "0.0.0.0"] {
            let err = check_loopback(host).unwrap_err();
            assert!(err.contains("loopback"), "{host}: {err}");
        }
    }

    #[test]
    fn parses_fetch_without_logging_shape() {
        let blob = "* 1 FETCH (UID 7 FLAGS (\\Recent) BODY[HEADER.FIELDS (FROM SUBJECT)] {48}\nFrom: Owner <Owner@Proton.me>\nSubject: Hi there\n)\nt2 OK\n";
        let headers = parse_fetches(blob, "inbox");
        assert_eq!(headers.len(), 1);
        assert_eq!(headers[0].id, "inbox:7");
        assert_eq!(headers[0].from, "owner@proton.me");
        assert!(headers[0].unread);
        assert_eq!(headers[0].subject, "Hi there");
    }

    #[tokio::test]
    async fn plaintext_loopback_list_and_bad_login_hides_password() {
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let port = listener.local_addr().unwrap().port();
        tokio::spawn(async move {
            loop {
                let Ok((mut sock, _)) = listener.accept().await else {
                    break;
                };
                tokio::spawn(async move {
                    let _ = sock.write_all(b"* OK fixture\r\n").await;
                    let mut buf = Vec::new();
                    let _ = tokio::time::timeout(Duration::from_secs(2), async {
                        loop {
                            let mut tmp = [0u8; 1024];
                            let n = sock.read(&mut tmp).await.unwrap_or(0);
                            if n == 0 {
                                break;
                            }
                            buf.extend_from_slice(&tmp[..n]);
                            if buf.windows(2).any(|pair| pair == b"\r\n") {
                                let text = String::from_utf8_lossy(&buf).to_string();
                                buf.clear();
                                let tag = text.split_whitespace().next().unwrap_or("t1");
                                if text.contains("LOGIN") {
                                    if text.contains("wrong-secret") {
                                        let _ = sock.write_all(format!("{tag} NO\r\n").as_bytes()).await;
                                    } else {
                                        let _ = sock.write_all(format!("{tag} OK\r\n").as_bytes()).await;
                                    }
                                } else if text.contains("FETCH") {
                                    let body = "From: a@b.co\r\nSubject: Hi\r\n";
                                    let msg = format!(
                                        "* 1 FETCH (UID 4 FLAGS () BODY[HEADER.FIELDS (FROM SUBJECT)] {{{}}}\r\n{body})\r\n{tag} OK\r\n",
                                        body.len()
                                    );
                                    let _ = sock.write_all(msg.as_bytes()).await;
                                } else if text.contains("CAPABILITY") {
                                    let _ = sock
                                        .write_all(format!("* CAPABILITY IMAP4rev1\r\n{tag} OK\r\n").as_bytes())
                                        .await;
                                } else {
                                    let _ = sock.write_all(format!("{tag} OK\r\n").as_bytes()).await;
                                }
                            }
                        }
                    })
                    .await;
                });
            }
        });
        let bad = BridgeMailer::new(
            "127.0.0.1".into(),
            port,
            port,
            "bot@proton.me".into(),
            "wrong-secret".into(),
            Duration::from_secs(2),
        )
        .unwrap();
        let err = bad.probe().await.unwrap_err();
        assert!(!err.contains("wrong-secret"), "{err}");
        assert!(err.contains("will not prompt") || err.contains("rejected") || err.contains("Bridge"), "{err}");

        let good = BridgeMailer::new(
            "127.0.0.1".into(),
            port,
            port,
            "bot@proton.me".into(),
            "bridge-secret".into(),
            Duration::from_secs(2),
        )
        .unwrap();
        let headers = good.list("inbox", false).await.unwrap();
        assert_eq!(headers[0].id, "inbox:4");
        assert_eq!(headers[0].from, "a@b.co");
        assert!(BridgeMailer::new(
            "10.1.1.1".into(),
            1143,
            1025,
            "bot@proton.me".into(),
            "x".into(),
            Duration::from_secs(1),
        )
        .is_err());
    }
}
