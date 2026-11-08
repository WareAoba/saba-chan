//! 새 PC 붙이기: 다른 PC(새로 설치한 리눅스 등)가 낸 페어링 코드를 이 PC 주인의 것으로 받고, 그 PC의 원격 데몬과 짝을 맺는다.
//!
//!   claim_and_connect   릴레이에 코드를 받고(`relay::claim`) 그 PC가 원격 포트를 올릴 때까지 기다려 `peer::connect`
//!   Install             이 PC의 ssh로 원격 PC에 붙어 한 줄 설치(install.sh `--pair`)를 돌린다. ssh는 의사 터미널에서 돌려
//!                       처음 접속 확인·비밀번호·sudo 물음을 앱에 넘긴다 (사바쨩은 열쇠·비밀번호를 받아 두지 않는다).
//!                       설치가 찍은 `SABA_PAIR_CODE=<코드>` 줄을 보고 `claim_and_connect`를 한다

use std::collections::HashMap;
use std::io::{Read, Write};
use std::sync::{Arc, LazyLock, Mutex};
use std::time::Duration;

use axum::http::{Request, StatusCode};
use saba_chan_updater_lib::config_file::ConfigFile;
use serde::Serialize;
use serde_json::{json, Value};
use tokio_util::sync::CancellationToken;

use super::config::Daemon;
use super::peer::{self, ConnectError};
use super::relay::{self, ClaimFailure, Target};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ClaimError {
    CodeNotFound,
    CodeUsed,
    CodeExpired,
    RelayNotPaired,
    RelayUnreachable,
    /// 코드는 받았지만 그 PC가 시간 안에 원격 포트를 올리지 않았다 (방화벽, 꺼진 원격 포트)
    RemoteNotReady,
    Connect(ConnectError),
}

impl ClaimError {
    pub fn code(self) -> &'static str {
        match self {
            Self::CodeNotFound => "pair_code_not_found",
            Self::CodeUsed => "pair_code_used",
            Self::CodeExpired => "pair_code_expired",
            Self::RelayNotPaired => "relay_not_paired",
            Self::RelayUnreachable => "relay_unreachable",
            Self::RemoteNotReady => "remote_not_ready",
            Self::Connect(e) => e.code(),
        }
    }

    pub fn status(self) -> StatusCode {
        match self {
            Self::CodeNotFound => StatusCode::NOT_FOUND,
            Self::CodeUsed => StatusCode::CONFLICT,
            Self::CodeExpired => StatusCode::GONE,
            Self::RelayNotPaired => StatusCode::SERVICE_UNAVAILABLE,
            Self::RelayUnreachable => StatusCode::BAD_GATEWAY,
            Self::RemoteNotReady => StatusCode::GATEWAY_TIMEOUT,
            Self::Connect(e) => e.status(),
        }
    }
}

/// 페어링 코드 모양 (릴레이 `remotePairClaimSchema`와 같다)
pub fn valid_code(code: &str) -> bool {
    (8..=64).contains(&code.len()) && code.bytes().all(|b| b.is_ascii_alphanumeric() || b == b'_' || b == b'-')
}

/// 코드를 받은 PC가 데몬을 켜고 원격 포트를 올릴 때까지 기다리는 시간과 간격
const READY_WAIT: Duration = if cfg!(test) { Duration::from_secs(2) } else { Duration::from_secs(120) };
const READY_POLL: Duration = if cfg!(test) { Duration::from_millis(100) } else { Duration::from_secs(3) };

/// 코드를 이 PC 주인의 것으로 받고 그 PC와 짝을 맺는다. 짝을 맺으면 그 데몬의 자동 업데이트를 켠다 (사용자 결정 2026-11-08)
pub async fn claim_and_connect(file: &ConfigFile, relay: Option<Target>, code: &str, stop: &CancellationToken) -> Result<Daemon, ClaimError> {
    let target = relay.clone().ok_or(ClaimError::RelayNotPaired)?;
    let claimed = relay::claim(&target, code).await.map_err(|e| match e {
        ClaimFailure::NotFound => ClaimError::CodeNotFound,
        ClaimFailure::AlreadyClaimed => ClaimError::CodeUsed,
        ClaimFailure::Expired => ClaimError::CodeExpired,
        ClaimFailure::Unreachable => ClaimError::RelayUnreachable,
    })?;
    tracing::info!("Claimed pairing code for PC {}", claimed.host_id);
    let deadline = tokio::time::Instant::now() + READY_WAIT;
    let daemon = loop {
        match peer::connect(file, relay.clone(), &claimed.host_id).await {
            Ok(d) => break d,
            // 막 받은 PC는 노드 토큰을 받고 원격 포트를 올리기까지 몇 초 걸린다
            Err(ConnectError::NotFound | ConnectError::NoEndpoint | ConnectError::RemoteUnreachable) if tokio::time::Instant::now() < deadline => {}
            Err(ConnectError::NotFound | ConnectError::NoEndpoint) => return Err(ClaimError::RemoteNotReady),
            Err(e) => return Err(ClaimError::Connect(e)),
        }
        tokio::select! {
            _ = stop.cancelled() => return Err(ClaimError::RemoteNotReady),
            _ = tokio::time::sleep(READY_POLL) => {}
        }
    };
    set_auto_update(file, relay, &daemon.host_id, true, stop.clone()).await;
    Ok(daemon)
}

/// 원격 데몬의 자동 업데이트를 켜거나 끈다 (`PUT /api/daemon/update`). 옛 데몬은 이 경로가 없어 조용히 넘어간다
pub async fn set_auto_update(file: &ConfigFile, relay: Option<Target>, host_id: &str, enabled: bool, stop: CancellationToken) -> bool {
    let body = json!({ "auto": enabled }).to_string();
    let req = Request::put("/api/daemon/update")
        .header(axum::http::header::CONTENT_TYPE, "application/json")
        .body(axum::body::Body::from(body))
        .unwrap_or_default();
    let res = peer::forward(file, relay, host_id, "/api/daemon/update", req, stop).await;
    let ok = res.status().is_success();
    if !ok {
        tracing::info!("Remote daemon {} did not take the auto-update setting ({})", host_id, res.status());
    }
    ok
}

// ── ssh로 설치 ──

/// 설치를 돌리는 한 줄. 사용자 입력은 넣지 않는다 (옵션은 정해진 글자만)
const INSTALL_URL: &str = "https://raw.githubusercontent.com/WareAoba/saba-chan/main/install.sh";
/// 설치가 찍는 페어링 코드 줄 (install.sh `--pair` → `saba-chan-cli bot pair`)
pub const PAIR_MARKER: &str = "SABA_PAIR_CODE=";
/// 앱에 보여 줄 출력의 상한 (앞을 버린다)
const OUTPUT_LIMIT: usize = 256 * 1024;

#[derive(Debug, Clone, Default)]
pub struct InstallOptions {
    pub host: String,
    pub user: String,
    pub port: u16,
    /// 로그아웃해도 데몬이 돌게 (`loginctl enable-linger`, sudo)
    pub linger: bool,
    /// SteamCMD가 쓰는 32비트 라이브러리 (sudo)
    pub steam_libs: bool,
}

/// ssh 계정 이름: 옵션으로 읽히지 않게 `-`로 시작하지 않는다
pub fn valid_user(user: &str) -> bool {
    (1..=64).contains(&user.len()) && !user.starts_with('-') && user.bytes().all(|b| b.is_ascii_alphanumeric() || matches!(b, b'.' | b'_' | b'-'))
}

/// ssh에 넘길 인자. 호스트·계정은 걸러 받은 것만
pub fn ssh_args(o: &InstallOptions) -> Vec<String> {
    let mut flags = String::from(" --pair --yes --no-gui");
    if o.linger {
        flags.push_str(" --linger");
    }
    if o.steam_libs {
        flags.push_str(" --steam-libs");
    }
    vec![
        "-tt".into(),
        "-o".into(),
        "ServerAliveInterval=15".into(),
        "-p".into(),
        o.port.to_string(),
        format!("{}@{}", o.user, o.host),
        format!("curl -fsSL {INSTALL_URL} | sh -s --{flags}"),
    ]
}

/// ssh 프로그램. Windows는 OS에 든 OpenSSH 클라이언트를 먼저 찾는다
fn ssh_program() -> Option<std::path::PathBuf> {
    #[cfg(windows)]
    {
        if let Some(root) = std::env::var_os("SystemRoot") {
            let p = std::path::Path::new(&root).join("System32").join("OpenSSH").join("ssh.exe");
            if p.is_file() {
                return Some(p);
            }
        }
    }
    let name = if cfg!(windows) { "ssh.exe" } else { "ssh" };
    std::env::split_paths(&std::env::var_os("PATH")?).map(|d| d.join(name)).find(|p| p.is_file())
}

/// 앱에 넘기는 물음
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Prompt {
    /// `host_key` | `password` | `passphrase` | `sudo` (그 밖의 줄 입력은 `text`)
    pub kind: &'static str,
    /// 물음 줄과, 처음 접속 확인이면 지문이 든 앞줄들
    pub text: String,
}

/// 끝나지 않은 마지막 줄이 입력을 기다리는 물음인가
pub fn detect_prompt(output: &str) -> Option<Prompt> {
    let last = output.rsplit('\n').next().unwrap_or("").trim_end();
    if last.is_empty() {
        return None;
    }
    let lower = last.to_ascii_lowercase();
    let kind = if lower.contains("continue connecting") && lower.contains("yes/no") {
        "host_key"
    } else if lower.starts_with("[sudo]") || (lower.contains("sudo") && lower.ends_with(':')) {
        "sudo"
    } else if lower.contains("passphrase for") && lower.ends_with(':') {
        "passphrase"
    } else if lower.ends_with("password:") {
        "password"
    } else {
        return None;
    };
    let text = if kind == "host_key" {
        // 처음 접속 확인은 지문이 든 앞줄들과 함께 보인다
        let lines: Vec<&str> = output.lines().collect();
        lines[lines.len().saturating_sub(4)..].join("\n")
    } else {
        last.to_string()
    };
    Some(Prompt { kind, text })
}

/// 터미널 제어 글자를 지운다 (CSI·OSC, 줄 앞으로 돌아가기). 남은 글자만 앱 콘솔에 보인다
pub fn strip_ansi(raw: &str) -> String {
    let mut out = String::with_capacity(raw.len());
    let mut chars = raw.chars().peekable();
    while let Some(c) = chars.next() {
        match c {
            '\u{1b}' => match chars.next() {
                Some('[') => {
                    for c in chars.by_ref() {
                        if ('@'..='~').contains(&c) {
                            break;
                        }
                    }
                }
                Some(']') => {
                    while let Some(c) = chars.next() {
                        if c == '\u{7}' || (c == '\u{1b}' && chars.next_if_eq(&'\\').is_some()) {
                            break;
                        }
                    }
                }
                _ => {}
            },
            '\r' => {
                if chars.peek() != Some(&'\n') {
                    // 같은 줄을 다시 쓰는 진행 표시: 줄을 나눠 보인다
                    out.push('\n');
                }
            }
            c if c.is_control() && c != '\n' && c != '\t' => {}
            c => out.push(c),
        }
    }
    out
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum Phase {
    Connecting,
    Installing,
    /// 설치가 페어링 코드를 냈다: 받고 짝을 맺는 중
    Pairing,
    Done,
    Failed,
    Cancelled,
}

struct Inner {
    phase: Phase,
    /// 지금까지의 출력 (앞을 버렸으면 `dropped`만큼)
    output: String,
    dropped: usize,
    prompt: Option<Prompt>,
    error: Option<String>,
    daemon: Option<(String, String)>,
    code_seen: bool,
}

pub struct Install {
    inner: Mutex<Inner>,
    writer: Mutex<Option<Box<dyn Write + Send>>>,
    child: Mutex<Option<Box<dyn portable_pty::ChildKiller + Send + Sync>>>,
    cancel: CancellationToken,
}

static INSTALLS: LazyLock<Mutex<HashMap<String, Arc<Install>>>> = LazyLock::new(Default::default);

fn installs() -> std::sync::MutexGuard<'static, HashMap<String, Arc<Install>>> {
    INSTALLS.lock().unwrap_or_else(|e| e.into_inner())
}

impl Install {
    fn new(cancel: CancellationToken) -> Self {
        Self {
            inner: Mutex::new(Inner { phase: Phase::Connecting, output: String::new(), dropped: 0, prompt: None, error: None, daemon: None, code_seen: false }),
            writer: Mutex::new(None),
            child: Mutex::new(None),
            cancel,
        }
    }

    fn lock(&self) -> std::sync::MutexGuard<'_, Inner> {
        self.inner.lock().unwrap_or_else(|e| e.into_inner())
    }

    /// 출력 조각을 붙이고, 물음과 페어링 코드 줄을 찾는다. 새로 찾은 코드를 돌려준다
    fn push(&self, text: &str) -> Option<String> {
        let mut inner = self.lock();
        let start_of_new = inner.output.len();
        inner.output.push_str(text);
        if inner.phase == Phase::Connecting && !text.trim().is_empty() {
            inner.phase = Phase::Installing;
        }
        let mut code = None;
        if !inner.code_seen {
            // 줄 하나가 조각에 걸쳐 올 수 있어 끝난 줄에서만 찾는다
            let from = inner.output[..start_of_new].rfind('\n').map_or(0, |i| i + 1);
            for line in inner.output[from..].split_inclusive('\n').filter(|l| l.ends_with('\n')) {
                if let Some(c) = line.trim().strip_prefix(PAIR_MARKER).map(str::trim).filter(|c| valid_code(c)) {
                    code = Some(c.to_string());
                    break;
                }
            }
            if code.is_some() {
                inner.code_seen = true;
                inner.phase = Phase::Pairing;
            }
        }
        inner.prompt = detect_prompt(&inner.output);
        if inner.output.len() > OUTPUT_LIMIT {
            let mut cut = inner.output.len() - OUTPUT_LIMIT;
            while !inner.output.is_char_boundary(cut) {
                cut += 1;
            }
            inner.output.drain(..cut);
            inner.dropped += cut;
        }
        code
    }

    fn finish(&self, phase: Phase, error: Option<&str>) {
        let mut inner = self.lock();
        if matches!(inner.phase, Phase::Done | Phase::Failed | Phase::Cancelled) {
            return;
        }
        inner.phase = phase;
        inner.error = error.map(str::to_string);
        inner.prompt = None;
    }

    /// `since`(지금까지 받은 출력 길이) 뒤의 출력과 지금 상태
    pub fn view(&self, since: usize) -> Value {
        let inner = self.lock();
        let total = inner.dropped + inner.output.len();
        let mut from = since.saturating_sub(inner.dropped).min(inner.output.len());
        while !inner.output.is_char_boundary(from) {
            from -= 1;
        }
        json!({
            "phase": inner.phase,
            "output": &inner.output[from..],
            "offset": total,
            "prompt": inner.prompt,
            "error_code": inner.error,
            "hostId": inner.daemon.as_ref().map(|d| d.0.clone()),
            "name": inner.daemon.as_ref().map(|d| d.1.clone()),
        })
    }

    /// 물음에 답한다. 답은 ssh·sudo에만 넘기고 남기지 않는다
    pub fn answer(&self, text: &str) -> bool {
        let mut writer = self.writer.lock().unwrap_or_else(|e| e.into_inner());
        let Some(w) = writer.as_mut() else { return false };
        let ok = w.write_all(text.as_bytes()).and_then(|()| w.write_all(b"\r")).and_then(|()| w.flush()).is_ok();
        if ok {
            self.lock().prompt = None;
        }
        ok
    }

    pub fn cancel(&self) {
        self.cancel.cancel();
        if let Some(mut child) = self.child.lock().unwrap_or_else(|e| e.into_inner()).take() {
            let _ = child.kill();
        }
        self.finish(Phase::Cancelled, None);
    }
}

pub fn install(id: &str) -> Option<Arc<Install>> {
    installs().get(id).cloned()
}

pub fn remove_install(id: &str) -> Option<Arc<Install>> {
    installs().remove(id)
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum StartError {
    SshMissing,
    SpawnFailed,
}

/// ssh로 설치를 시작한다. 설치 하나의 id를 돌려준다
pub fn start_install(file: ConfigFile, relay: Option<Target>, options: &InstallOptions, stop: CancellationToken) -> Result<String, StartError> {
    let program = ssh_program().ok_or(StartError::SshMissing)?;
    let mut cmd = portable_pty::CommandBuilder::new(program);
    cmd.args(ssh_args(options));
    let size = portable_pty::PtySize { rows: 40, cols: 160, pixel_width: 0, pixel_height: 0 };
    let pair = portable_pty::native_pty_system().openpty(size).map_err(|_| StartError::SpawnFailed)?;
    let mut child = pair.slave.spawn_command(cmd).map_err(|e| {
        tracing::warn!("Could not start ssh: {}", e);
        StartError::SpawnFailed
    })?;
    drop(pair.slave);
    let mut reader = pair.master.try_clone_reader().map_err(|_| StartError::SpawnFailed)?;
    let writer = pair.master.take_writer().map_err(|_| StartError::SpawnFailed)?;

    let id = uuid::Uuid::new_v4().simple().to_string();
    let session = Arc::new(Install::new(stop.child_token()));
    *session.writer.lock().unwrap_or_else(|e| e.into_inner()) = Some(writer);
    *session.child.lock().unwrap_or_else(|e| e.into_inner()) = Some(child.clone_killer());
    installs().insert(id.clone(), session.clone());
    tracing::info!("Installing saba-chan over ssh on {}:{}", options.host, options.port);

    let rt = tokio::runtime::Handle::current();
    let reading = session.clone();
    std::thread::spawn(move || {
        let _master = pair.master;
        let mut buf = [0u8; 4096];
        let mut pending = Vec::new();
        loop {
            let n = match reader.read(&mut buf) {
                Ok(0) | Err(_) => break,
                Ok(n) => n,
            };
            pending.extend_from_slice(&buf[..n]);
            // 여러 바이트 글자가 조각에 걸치면 다음 조각과 합쳐 읽는다
            let valid = match std::str::from_utf8(&pending) {
                Ok(_) => pending.len(),
                Err(e) if e.error_len().is_none() => e.valid_up_to(),
                Err(_) => pending.len(),
            };
            let text = String::from_utf8_lossy(&pending[..valid]).into_owned();
            pending.drain(..valid);
            if let Some(code) = reading.push(&strip_ansi(&text)) {
                let (file, relay, session) = (file.clone(), relay.clone(), reading.clone());
                rt.spawn(async move {
                    match claim_and_connect(&file, relay, &code, &session.cancel).await {
                        Ok(d) => {
                            session.lock().daemon = Some((d.host_id.clone(), d.label()));
                            session.finish(Phase::Done, None);
                        }
                        Err(e) => session.finish(Phase::Failed, Some(e.code())),
                    }
                });
            }
        }
    });
    let waiting = session;
    std::thread::spawn(move || {
        let status = child.wait();
        let (code_seen, cancelled) = {
            let inner = waiting.lock();
            (inner.code_seen, inner.phase == Phase::Cancelled)
        };
        // 짝 맺기는 따로 끝난다 (설치 쪽 CLI는 코드가 받아지면 끝난다)
        if code_seen || cancelled {
            return;
        }
        let error = match status {
            // ssh 자체의 실패 (접속·인증)
            Ok(s) if s.exit_code() == 255 => "ssh_failed",
            Ok(s) if s.success() => "pair_code_missing",
            _ => "install_failed",
        };
        waiting.finish(Phase::Failed, Some(error));
    });
    Ok(id)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_install_command_carries_only_fixed_flags() {
        let o = InstallOptions { host: "nas.local".into(), user: "saba".into(), port: 2222, linger: true, steam_libs: false };
        let args = ssh_args(&o);
        assert_eq!(&args[..5], ["-tt", "-o", "ServerAliveInterval=15", "-p", "2222"]);
        assert_eq!(args[5], "saba@nas.local");
        assert_eq!(args[6], format!("curl -fsSL {INSTALL_URL} | sh -s -- --pair --yes --no-gui --linger"));
        assert!(valid_user("saba_1.x"));
        for bad in ["", "-oProxyCommand=x", "a b", "a;b", "a@b"] {
            assert!(!valid_user(bad), "{bad}");
        }
    }

    #[test]
    fn prompts_are_told_apart() {
        let host_key = "The authenticity of host 'nas (10.0.0.2)' can't be established.\nED25519 key fingerprint is SHA256:abc.\nAre you sure you want to continue connecting (yes/no/[fingerprint])? ";
        let p = detect_prompt(host_key).unwrap();
        assert_eq!(p.kind, "host_key");
        assert!(p.text.contains("SHA256:abc"));
        assert_eq!(detect_prompt("saba@nas's password: ").unwrap().kind, "password");
        assert_eq!(detect_prompt("ok\n[sudo] password for saba: ").unwrap().kind, "sudo");
        assert_eq!(detect_prompt("Enter passphrase for key '/home/a/.ssh/id_ed25519': ").unwrap().kind, "passphrase");
        assert_eq!(detect_prompt("Downloading...\n"), None);
        assert_eq!(detect_prompt("saba@nas's password: \nWelcome\n"), None);
    }

    #[test]
    fn terminal_control_is_stripped() {
        assert_eq!(strip_ansi("\u{1b}[1;36mcode\u{1b}[0m\r\nnext\u{1b}]0;title\u{7}"), "code\nnext");
        assert_eq!(strip_ansi("10%\r20%\r\n"), "10%\n20%\n");
    }

    /// 설치가 찍은 코드 줄은 줄이 끝난 뒤 한 번만 찾고, 조각에 걸쳐 와도 찾는다
    #[test]
    fn the_pairing_code_line_is_found_once_even_when_split() {
        let s = Install::new(CancellationToken::new());
        assert_eq!(s.push("Installing\nSABA_PAIR_CO"), None);
        assert_eq!(s.push("DE=Abc123_xyz-0"), None);
        assert_eq!(s.push("\n").as_deref(), Some("Abc123_xyz-0"));
        assert_eq!(s.push("SABA_PAIR_CODE=Other12345\n"), None);
        let v = s.view(0);
        assert_eq!(v["phase"], "pairing");
        // 이어 읽기: 받은 만큼 뒤만
        let offset = v["offset"].as_u64().unwrap() as usize;
        s.push("more\n");
        assert_eq!(s.view(offset)["output"], "more\n");
    }

    #[test]
    fn a_bad_code_line_is_not_taken() {
        let s = Install::new(CancellationToken::new());
        assert_eq!(s.push("SABA_PAIR_CODE=short\n"), None);
        assert_eq!(s.push("SABA_PAIR_CODE=has space here\n"), None);
    }
}
