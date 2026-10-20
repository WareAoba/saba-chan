//! GUI가 띄운 데몬은 stderr가 GUI의 파이프다. GUI가 먼저 죽어 파이프가 닫혀도 데몬은 로그를 쓰다 죽지 않아야 한다.

use std::io::{Read, Write};
use std::process::{Command, Stdio};
use std::time::{Duration, Instant};

fn free_port() -> u16 {
    std::net::TcpListener::bind("127.0.0.1:0").unwrap().local_addr().unwrap().port()
}

/// HTTP/1.1 요청 하나를 보내고 상태 줄을 돌려준다. 연결이 응답 없이 끊기면 None
fn status_line(port: u16, request: &str) -> Option<String> {
    let mut stream = std::net::TcpStream::connect(("127.0.0.1", port)).ok()?;
    stream.set_read_timeout(Some(Duration::from_secs(10))).ok()?;
    stream.write_all(request.as_bytes()).ok()?;
    let mut out = String::new();
    let _ = stream.read_to_string(&mut out);
    out.lines().next().filter(|l| l.starts_with("HTTP/")).map(str::to_string)
}

/// 단언이 실패해도 데몬을 남기지 않는다
struct KillOnDrop(std::process::Child);

impl Drop for KillOnDrop {
    fn drop(&mut self) {
        let _ = self.0.kill();
        let _ = self.0.wait();
    }
}

#[test]
fn a_daemon_keeps_serving_after_its_parent_closes_the_log_pipe() {
    let data_dir = tempfile::tempdir().unwrap();
    let port = free_port();
    let mut daemon = KillOnDrop(Command::new(env!("CARGO_BIN_EXE_saba-core"))
        .arg("--spawned")
        .env("SABA_DATA_DIR", data_dir.path())
        .env("SABA_IPC_PORT", port.to_string())
        .env("SABA_MODULES_PATH", data_dir.path().join("modules"))
        .env("SABA_INSTANCES_PATH", data_dir.path().join("instances"))
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .expect("spawn saba-core"));

    let token_path = data_dir.path().join(".ipc_token");
    let deadline = Instant::now() + Duration::from_secs(30);
    while !token_path.exists() || status_line(port, "GET /api/version HTTP/1.1\r\nHost: 127.0.0.1\r\nConnection: close\r\n\r\n").is_none() {
        assert!(Instant::now() < deadline, "daemon never came up");
        assert!(daemon.0.try_wait().unwrap().is_none(), "daemon exited early");
        std::thread::sleep(Duration::from_millis(100));
    }
    let token = std::fs::read_to_string(&token_path).unwrap();

    // 부모(GUI)가 사라진 것처럼 파이프의 읽는 쪽을 닫는다
    drop(daemon.0.stdout.take());
    drop(daemon.0.stderr.take());

    // 경고를 남기는 요청(토큰 없음)과 정보 로그를 남기는 요청(클라이언트 등록) 모두 답해야 한다
    let unauthorized = status_line(port, "GET /api/servers HTTP/1.1\r\nHost: 127.0.0.1\r\nConnection: close\r\n\r\n");
    assert_eq!(unauthorized.as_deref().map(|l| l.contains("401")), Some(true), "{unauthorized:?}");
    let body = r#"{"kind":"gui"}"#;
    let register = format!(
        "POST /api/client/register HTTP/1.1\r\nHost: 127.0.0.1\r\nX-Saba-Token: {}\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}",
        token.trim(),
        body.len(),
        body
    );
    for _ in 0..3 {
        let registered = status_line(port, &register);
        assert_eq!(registered.as_deref().map(|l| l.contains("200")), Some(true), "{registered:?}");
    }
    std::thread::sleep(Duration::from_millis(300));
    assert!(daemon.0.try_wait().unwrap().is_none(), "daemon must keep running");

    // 로그는 데몬 자신의 파일에 남는다
    let log = std::fs::read_to_string(data_dir.path().join("logs").join("daemon.log")).unwrap_or_default();
    assert!(log.contains("[Heartbeat] Active clients"), "the daemon log file should keep the lines: {log}");
}
