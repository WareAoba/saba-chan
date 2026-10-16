//! 데몬 단일 인스턴스 보장 (instance_lock) — 실제 바이너리 두 개를 띄워 확인한다.

use std::process::{Command, Stdio};
use std::time::{Duration, Instant};

fn spawn_daemon(data_dir: &std::path::Path, port: u16) -> std::process::Child {
    Command::new(env!("CARGO_BIN_EXE_saba-core"))
        .arg("--spawned")
        .env("SABA_DATA_DIR", data_dir)
        .env("SABA_IPC_PORT", port.to_string())
        .env("SABA_MODULES_PATH", data_dir.join("modules"))
        .env("SABA_INSTANCES_PATH", data_dir.join("instances"))
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::piped())
        .spawn()
        .expect("spawn saba-core")
}

fn free_port() -> u16 {
    std::net::TcpListener::bind("127.0.0.1:0").unwrap().local_addr().unwrap().port()
}

#[test]
fn second_daemon_exits_without_touching_running_one() {
    let data_dir = tempfile::tempdir().unwrap();
    let lock_path = data_dir.path().join("daemon.lock");

    let mut first = spawn_daemon(data_dir.path(), free_port());

    // 첫 데몬이 잠금과 토큰을 모두 기록할 때까지 대기
    let token_path = data_dir.path().join(".ipc_token");
    let deadline = Instant::now() + Duration::from_secs(30);
    while saba_core::instance_lock::read_owner(&lock_path).is_none() || !token_path.exists() {
        assert!(Instant::now() < deadline, "first daemon never took the lock");
        assert!(first.try_wait().unwrap().is_none(), "first daemon exited early");
        std::thread::sleep(Duration::from_millis(100));
    }
    let token_before = std::fs::read_to_string(&token_path).unwrap();

    let second = spawn_daemon(data_dir.path(), free_port())
        .wait_with_output()
        .expect("wait second daemon");
    assert_eq!(second.status.code(), Some(2), "second daemon must exit with code 2");
    assert!(
        String::from_utf8_lossy(&second.stderr).contains("already running"),
        "stderr should explain why: {}",
        String::from_utf8_lossy(&second.stderr)
    );

    // 두 번째 데몬이 토큰을 덮어쓰지 않았어야 한다
    let token_after = std::fs::read_to_string(&token_path).unwrap();
    assert_eq!(token_before, token_after);
    assert!(first.try_wait().unwrap().is_none(), "first daemon must keep running");

    let _ = first.kill();
    let _ = first.wait();
}
