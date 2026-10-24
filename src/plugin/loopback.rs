//! 격리된 실행의 루프백 중계. AppContainer는 127.0.0.1에 닿지 못하므로(능력은 internetClient뿐),
//! 미확인 모듈이 자기 서버(REST·상태 핑)에 묻지 못해 상태 질의와 정지가 늘 시간 초과였다.
//!
//! 모듈(공용 `saba_loopback.py`)이 stderr에 `LOOPBACK:{id, port, data, timeout, idle_ms}` 한 줄을 쓰면
//! 데몬이 `127.0.0.1:port`에 TCP로 `data`(base64)를 보내고 받은 바이트를 `SABA_LOOPBACK_DIR/<id>.json`에
//! `{ok, data}` 또는 `{ok:false, error}`로 둔다. 모듈은 그 파일을 기다린다(stdin은 지금처럼 config 뒤에 닫힌다).
//! 포트는 데몬이 만든 config에 있는 그 인스턴스의 포트뿐이고, 그 포트에서 듣는 프로세스가 데몬이 추적하는
//! 그 인스턴스의 서버(또는 그 자식)일 때만 연결한다. 요청·응답 크기와 시간에 상한이 있다.

use base64::engine::general_purpose::STANDARD as BASE64;
use base64::Engine;
use serde_json::{json, Value};
use std::path::{Path, PathBuf};
use std::time::Duration;
use tokio::io::{AsyncReadExt, AsyncWriteExt};

/// stderr 줄 머리
pub const PREFIX: &str = "LOOPBACK:";
/// 보내는 바이트 상한
pub const MAX_REQUEST: usize = 64 * 1024;
/// 받는 바이트 상한 (넘으면 거기서 끊고 받은 만큼 돌려준다)
pub const MAX_RESPONSE: usize = 1024 * 1024;
/// 한 요청의 시간 상한 (연결부터 마지막 바이트까지)
pub const MAX_TIMEOUT: Duration = Duration::from_secs(30);
/// 응답이 끊기지 않는 프로토콜(마인크래프트 상태 핑 등)을 위한 "조용해지면 끝" 상한
const MAX_IDLE: Duration = Duration::from_secs(2);
/// 한 실행이 보낼 수 있는 요청 수
pub const MAX_REQUESTS: usize = 64;

#[derive(Clone, Debug)]
pub struct Proxy {
    /// 응답 파일을 두는 폴더 (격리 실행이 읽을 수 있는 임시 폴더 아래)
    pub dir: PathBuf,
    /// 이 실행이 닿아도 되는 루프백 포트
    pub ports: Vec<u16>,
    /// 데몬이 추적하는 그 인스턴스의 서버 PID. 없으면(꺼져 있음) 아무 데도 잇지 않는다
    pub server: Option<u32>,
}

/// 데몬이 만든 config에서 그 인스턴스의 포트를 모은다: 게임 포트 `port`와 `rest_port`.
/// 다른 `*_port` 모듈 설정은 모듈이 `import_settings`로 아무 값이나 넣을 수 있어 넣지 않는다
pub fn instance_ports(config: &Value) -> Vec<u16> {
    let Some(map) = config.as_object() else {
        return Vec::new();
    };
    let mut ports: Vec<u16> = map
        .iter()
        .filter(|(key, _)| matches!(key.as_str(), "port" | "rest_port"))
        .filter_map(|(_, value)| match value {
            Value::Number(n) => n.as_u64(),
            Value::String(s) => s.trim().parse().ok(),
            _ => None,
        })
        .filter_map(|p| u16::try_from(p).ok())
        .filter(|p| *p > 0)
        .collect();
    ports.sort_unstable();
    ports.dedup();
    ports
}

/// 요청 id: 응답 파일 이름이 되므로 경로 문자가 없어야 한다
fn plain_id(id: &str) -> bool {
    !id.is_empty() && id.len() <= 64 && id.bytes().all(|b| b.is_ascii_alphanumeric() || b == b'-' || b == b'_')
}

/// 한 요청을 처리하고 응답 파일을 쓴다. 파일을 쓸 수 없는 요청(id가 이상함)은 로그만 남긴다
pub async fn serve(proxy: &Proxy, line: &str) {
    let request: Value = match serde_json::from_str(line) {
        Ok(v) => v,
        Err(e) => {
            tracing::warn!("[Loopback] unreadable request: {}", e);
            return;
        }
    };
    let Some(id) = request.get("id").and_then(Value::as_str).filter(|id| plain_id(id)) else {
        tracing::warn!("[Loopback] request without a usable id");
        return;
    };
    let reply = match exchange(proxy, &request).await {
        Ok(bytes) => json!({ "ok": true, "data": BASE64.encode(bytes) }),
        Err(e) => json!({ "ok": false, "error": e }),
    };
    if let Err(e) = write_reply(&proxy.dir, id, &reply) {
        tracing::warn!("[Loopback] could not write the reply for {}: {}", id, e);
    }
}

/// 받지 않을 요청에 바로 거절을 돌려준다 (모듈이 시간 초과까지 기다리지 않게)
pub fn refuse(proxy: &Proxy, line: &str, error: &str) {
    let id = serde_json::from_str::<Value>(line).ok().and_then(|r| r.get("id").and_then(Value::as_str).map(str::to_owned));
    if let Some(id) = id.filter(|id| plain_id(id)) {
        let _ = write_reply(&proxy.dir, &id, &json!({ "ok": false, "error": error }));
    }
}

/// 다 쓴 뒤에 이름을 바꿔 둔다: 모듈이 반쯤 쓴 파일을 읽지 않게. 폴더는 격리 쪽도 쓸 수 있으므로
/// 임시 파일은 늘 새로 만든다 (미리 놓인 링크를 따라 쓰지 않는다)
fn write_reply(dir: &Path, id: &str, reply: &Value) -> std::io::Result<()> {
    saba_atomic_file::write(&dir.join(format!("{id}.json")), &serde_json::to_vec(reply)?, saba_atomic_file::Mode::Plain)
}

async fn exchange(proxy: &Proxy, request: &Value) -> Result<Vec<u8>, String> {
    let port = request
        .get("port")
        .and_then(Value::as_u64)
        .and_then(|p| u16::try_from(p).ok())
        .ok_or("port_missing")?;
    if !proxy.ports.contains(&port) {
        tracing::warn!("[Loopback] refused port {} (allowed: {:?})", port, proxy.ports);
        return Err("port_not_allowed".into());
    }
    let data = BASE64
        .decode(request.get("data").and_then(Value::as_str).unwrap_or(""))
        .map_err(|_| "data_not_base64".to_string())?;
    if data.len() > MAX_REQUEST {
        return Err("request_too_large".into());
    }
    let server = proxy.server;
    if !tokio::task::spawn_blocking(move || served_by(port, server)).await.unwrap_or(false) {
        tracing::warn!("[Loopback] refused port {}: not held by the instance's server (PID {:?})", port, server);
        return Err("port_not_served".into());
    }
    let timeout = request
        .get("timeout")
        .and_then(Value::as_f64)
        .filter(|t| t.is_finite() && *t > 0.0)
        .map(Duration::from_secs_f64)
        .unwrap_or(Duration::from_secs(5))
        .min(MAX_TIMEOUT);
    let idle = request
        .get("idle_ms")
        .and_then(Value::as_u64)
        .map(Duration::from_millis)
        .map(|d| d.min(MAX_IDLE));
    match tokio::time::timeout(timeout, talk(port, &data, idle)).await {
        Ok(result) => result.map_err(|e| format!("io: {e}")),
        Err(_) => Err("timed_out".into()),
    }
}

/// 그 포트에서 듣는 프로세스가 `server`이거나 그 자식인가
fn served_by(port: u16, server: Option<u32>) -> bool {
    let Some(root) = server else {
        return false;
    };
    let Some(owners) = listener_pids(port) else {
        return false;
    };
    let mut system = sysinfo::System::new();
    system.refresh_processes();
    let parent = |pid: u32| system.process(sysinfo::Pid::from_u32(pid)).and_then(|p| p.parent()).map(|p| p.as_u32());
    owners.into_iter().any(|pid| descends_from(pid, root, parent))
}

/// `pid`에서 부모를 따라 올라가 `root`를 만나는가 (서버 런처 아래의 실제 서버 프로세스도 센다)
fn descends_from(mut pid: u32, root: u32, parent: impl Fn(u32) -> Option<u32>) -> bool {
    for _ in 0..64 {
        if pid == root {
            return true;
        }
        match parent(pid) {
            Some(p) if p != pid => pid = p,
            _ => return false,
        }
    }
    false
}

/// TCP로 그 포트를 듣는 프로세스들. 표를 못 읽으면 None (그때는 잇지 않는다)
#[cfg(windows)]
fn listener_pids(port: u16) -> Option<Vec<u32>> {
    crate::validator::tcp_listener_pids(port)
}

#[cfg(target_os = "linux")]
fn listener_pids(port: u16) -> Option<Vec<u32>> {
    // 듣는 소켓의 inode를 찾고(st 0A = LISTEN), 그 소켓을 쥔 프로세스를 /proc/<pid>/fd에서 찾는다
    let mut sockets = Vec::new();
    for table in ["/proc/net/tcp", "/proc/net/tcp6"] {
        let Ok(text) = std::fs::read_to_string(table) else { continue };
        for cols in text.lines().skip(1).map(|l| l.split_whitespace().collect::<Vec<_>>()) {
            let local = cols.get(1).and_then(|a| a.rsplit(':').next()).and_then(|p| u16::from_str_radix(p, 16).ok());
            if cols.get(3) == Some(&"0A") && local == Some(port) {
                if let Some(inode) = cols.get(9) {
                    sockets.push(std::path::PathBuf::from(format!("socket:[{inode}]")));
                }
            }
        }
    }
    let mut pids = Vec::new();
    for entry in std::fs::read_dir("/proc").ok()?.flatten() {
        let Some(pid) = entry.file_name().to_str().and_then(|n| n.parse::<u32>().ok()) else { continue };
        let Ok(fds) = std::fs::read_dir(entry.path().join("fd")) else { continue };
        if fds.flatten().any(|fd| std::fs::read_link(fd.path()).is_ok_and(|t| sockets.contains(&t))) {
            pids.push(pid);
        }
    }
    Some(pids)
}

#[cfg(not(any(windows, target_os = "linux")))]
fn listener_pids(_port: u16) -> Option<Vec<u32>> {
    None
}

/// 보내고, 닫힐 때까지(또는 상한까지, 또는 `idle` 동안 조용해질 때까지) 받는다
async fn talk(port: u16, data: &[u8], idle: Option<Duration>) -> std::io::Result<Vec<u8>> {
    let mut stream = tokio::net::TcpStream::connect(("127.0.0.1", port)).await?;
    stream.write_all(data).await?;
    let mut received = Vec::new();
    let mut buf = [0u8; 8192];
    loop {
        let read = match (idle, received.is_empty()) {
            (Some(gap), false) => match tokio::time::timeout(gap, stream.read(&mut buf)).await {
                Ok(r) => r?,
                Err(_) => break,
            },
            _ => stream.read(&mut buf).await?,
        };
        if read == 0 {
            break;
        }
        let room = MAX_RESPONSE - received.len();
        received.extend_from_slice(&buf[..read.min(room)]);
        if received.len() >= MAX_RESPONSE {
            break;
        }
    }
    Ok(received)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn proxy(ports: Vec<u16>) -> (tempfile::TempDir, Proxy) {
        let dir = tempfile::tempdir().unwrap();
        // 테스트 서버는 이 시험 프로세스 안에서 듣는다
        let proxy = Proxy { dir: dir.path().to_path_buf(), ports, server: Some(std::process::id()) };
        (dir, proxy)
    }

    fn reply(proxy: &Proxy, id: &str) -> Value {
        serde_json::from_slice(&std::fs::read(proxy.dir.join(format!("{id}.json"))).unwrap()).unwrap()
    }

    /// 한 번 받고 답한 뒤 닫는 서버 (HTTP `Connection: close`처럼)
    async fn answering(answer: &'static [u8]) -> (u16, tokio::task::JoinHandle<Vec<u8>>) {
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let port = listener.local_addr().unwrap().port();
        let server = tokio::spawn(async move {
            let (mut sock, _) = listener.accept().await.unwrap();
            let mut got = vec![0u8; 1024];
            let n = sock.read(&mut got).await.unwrap();
            sock.write_all(answer).await.unwrap();
            got.truncate(n);
            got
        });
        (port, server)
    }

    /// 격리된 모듈이 자기 서버의 REST에 보낸 요청을 데몬이 대신 보내고 응답을 파일로 돌려준다
    #[tokio::test]
    async fn a_request_to_the_instances_own_port_is_relayed() {
        let (port, server) = answering(b"HTTP/1.1 200 OK\r\nContent-Length: 2\r\n\r\n{}").await;
        let (_dir, proxy) = proxy(vec![port]);
        let request = json!({ "id": "a1", "port": port, "data": BASE64.encode(b"GET /v1/api/metrics HTTP/1.1\r\n\r\n") });
        serve(&proxy, &request.to_string()).await;
        assert_eq!(server.await.unwrap(), b"GET /v1/api/metrics HTTP/1.1\r\n\r\n");
        let reply = reply(&proxy, "a1");
        assert_eq!(reply["ok"], true);
        assert_eq!(BASE64.decode(reply["data"].as_str().unwrap()).unwrap(), b"HTTP/1.1 200 OK\r\nContent-Length: 2\r\n\r\n{}");
    }

    /// 그 인스턴스의 포트가 아니면 연결하지 않고 거절한다 (데몬 IPC 같은 다른 루프백 서비스에 닿지 않게)
    #[tokio::test]
    async fn a_port_outside_the_instance_is_refused() {
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let other = listener.local_addr().unwrap().port();
        let (_dir, proxy) = proxy(vec![other.wrapping_add(1).max(1)]);
        serve(&proxy, &json!({ "id": "b", "port": other, "data": "" }).to_string()).await;
        assert_eq!(reply(&proxy, "b")["error"], "port_not_allowed");
        assert!(tokio::time::timeout(Duration::from_millis(100), listener.accept()).await.is_err(), "no connection was made");
    }

    #[tokio::test]
    async fn oversized_requests_and_path_like_ids_are_refused() {
        let (_dir, proxy) = proxy(vec![1]);
        let big = BASE64.encode(vec![0u8; MAX_REQUEST + 1]);
        serve(&proxy, &json!({ "id": "c", "port": 1, "data": big }).to_string()).await;
        assert_eq!(reply(&proxy, "c")["error"], "request_too_large");
        serve(&proxy, &json!({ "id": "../x", "port": 1, "data": "" }).to_string()).await;
        assert_eq!(std::fs::read_dir(&proxy.dir).unwrap().count(), 1, "no file is written for a path-like id");
    }

    /// 닫지 않는 프로토콜(마인크래프트 상태 핑)은 `idle_ms` 동안 조용해지면 받은 만큼 돌려준다
    #[tokio::test]
    async fn an_answer_that_never_closes_ends_after_the_idle_gap() {
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let port = listener.local_addr().unwrap().port();
        let _server = tokio::spawn(async move {
            let (mut sock, _) = listener.accept().await.unwrap();
            let mut got = [0u8; 64];
            let _ = sock.read(&mut got).await;
            sock.write_all(b"pong").await.unwrap();
            tokio::time::sleep(Duration::from_secs(10)).await;
        });
        let (_dir, proxy) = proxy(vec![port]);
        let started = std::time::Instant::now();
        serve(&proxy, &json!({ "id": "d", "port": port, "data": BASE64.encode(b"ping"), "timeout": 5, "idle_ms": 200 }).to_string()).await;
        assert_eq!(BASE64.decode(reply(&proxy, "d")["data"].as_str().unwrap()).unwrap(), b"pong");
        assert!(started.elapsed() < Duration::from_secs(3));
    }

    /// 리뷰 310: 허용 포트는 모듈이 고칠 수 있는 config에서 온다. 그 포트에서 듣는 프로세스가 이 인스턴스의
    /// 서버(또는 그 자식)가 아니면 연결하지 않는다 (PC의 다른 로컬 서비스에 대신 닿지 않게)
    #[tokio::test]
    async fn a_port_another_process_listens_on_is_refused() {
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let port = listener.local_addr().unwrap().port();
        let (_dir, mut proxy) = proxy(vec![port]);
        proxy.server = Some(u32::MAX);
        serve(&proxy, &json!({ "id": "e", "port": port, "data": "" }).to_string()).await;
        assert_eq!(reply(&proxy, "e")["error"], "port_not_served");
        proxy.server = None;
        serve(&proxy, &json!({ "id": "f", "port": port, "data": "" }).to_string()).await;
        assert_eq!(reply(&proxy, "f")["error"], "port_not_served", "no running server, nothing to reach");
        assert!(tokio::time::timeout(Duration::from_millis(100), listener.accept()).await.is_err(), "no connection was made");
    }

    #[test]
    fn a_listener_counts_when_it_is_the_server_or_one_of_its_children() {
        let parents = [(30, 20), (20, 10), (10, 1)];
        let parent = |pid| parents.iter().find(|(p, _)| *p == pid).map(|(_, pp)| *pp);
        assert!(descends_from(20, 20, parent));
        assert!(descends_from(30, 20, parent));
        assert!(!descends_from(10, 20, parent), "the server's parent is not the server");
        assert!(!descends_from(5, 20, parent));
    }

    /// 모듈 설정의 `*_port` 값은 모듈이 정할 수 있으므로 넣지 않는다: 데몬 인스턴스 값 `port`·`rest_port`만
    #[test]
    fn the_instance_ports_are_the_game_and_rest_ports() {
        let config = json!({ "port": 8211, "rest_port": "8212", "query_port": 27015, "x_port": 22, "max_players": 32, "pid": 4 });
        assert_eq!(instance_ports(&config), vec![8211, 8212]);
    }
}
