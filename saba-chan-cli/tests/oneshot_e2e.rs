//! ═══════════════════════════════════════════════════════════════════
//! P0-1 / one-shot dispatcher: end-to-end coverage
//! ═══════════════════════════════════════════════════════════════════
//!
//! Spins up an in-process IPCServer (same harness as `client_round_trip`),
//! then drives the one-shot dispatcher (`oneshot::runner::dispatch`) with
//! representative subcommands. Verifies that:
//!
//!   * read commands hit the daemon and return EXIT_OK
//!   * write commands persist via the real handler chain
//!   * destructive commands without `--yes` exit non-zero
//!   * `daemon offline` paths return EXIT_DAEMON_OFFLINE without panicking
//!
//! These tests intentionally do **not** spawn the saba-core binary or the
//! Discord bot — those flows are covered by manual QA and are out of scope
//! for hermetic CI.

use clap::Parser;
use saba_chan_cli::client::DaemonClient;
use saba_chan_cli::oneshot::{self, Cli};
use saba_core::ipc::IPCServer;
use saba_core::supervisor::Supervisor;
use std::net::TcpListener;
use std::sync::Arc;
use tokio::sync::RwLock;
use tokio::time::{sleep, Duration};

fn pick_free_port() -> u16 {
    let l = TcpListener::bind("127.0.0.1:0").expect("ephemeral port");
    let p = l.local_addr().unwrap().port();
    drop(l);
    p
}

struct Harness {
    base_url: String,
    _data_dir: tempfile::TempDir,
    _modules_dir: tempfile::TempDir,
    _instances_dir: tempfile::TempDir,
    _supervisor: Arc<RwLock<Supervisor>>,
    handle: tokio::task::JoinHandle<()>,
}

impl Drop for Harness {
    fn drop(&mut self) {
        self.handle.abort();
    }
}

async fn boot() -> Harness {
    std::env::set_var("SABA_AUTH_DISABLED", "1");
    let data_dir = tempfile::tempdir().expect("temp data");
    std::env::set_var("SABA_DATA_DIR", data_dir.path());
    let modules_dir = tempfile::tempdir().expect("temp modules");
    let instances_dir = tempfile::tempdir().expect("temp instances");

    // Minimal module fixture
    let m = modules_dir.path().join("test-module");
    std::fs::create_dir_all(&m).unwrap();
    std::fs::write(
        m.join("module.toml"),
        r#"[module]
name = "test-module"
version = "0.0.1"
description = "oneshot fixture"
author = "ci"
game_name = "TestGame"
display_name = "Test Module"
entry = "lifecycle.py"
"#,
    )
    .unwrap();
    std::fs::write(m.join("lifecycle.py"), "# stub\n").unwrap();

    let supervisor = Arc::new(RwLock::new(Supervisor::new_with_instances_dir(
        modules_dir.path().to_str().unwrap(),
        instances_dir.path().to_str().unwrap(),
    )));
    {
        let mut sup = supervisor.write().await;
        sup.initialize().await.expect("supervisor init");
    }

    let port = pick_free_port();
    let listen_addr = format!("127.0.0.1:{}", port);
    let base_url = format!("http://{}", listen_addr);
    let server = IPCServer::new(
        supervisor.clone(),
        &listen_addr,
        saba_core::daemon_log::DaemonLogBuffer::new(),
    );
    let handle = tokio::spawn(async move {
        let _ = server.start().await;
    });

    let probe = reqwest::Client::new();
    for _ in 0..50 {
        if let Ok(r) = probe.get(format!("{}/api/modules", base_url)).send().await {
            if r.status().is_success() {
                return Harness {
                    base_url,
                    _data_dir: data_dir,
                    _modules_dir: modules_dir,
                    _instances_dir: instances_dir,
                    _supervisor: supervisor,
                    handle,
                };
            }
        }
        sleep(Duration::from_millis(100)).await;
    }
    panic!("daemon not ready");
}

fn parse_cli(args: &[&str]) -> Cli {
    // Always include --json so output is stable; quiet stdout doesn't matter.
    let mut full = vec!["saba-chan-cli", "--json"];
    full.extend_from_slice(args);
    Cli::try_parse_from(full).expect("clap parse")
}

#[tokio::test]
async fn oneshot_module_list_returns_ok() {
    let h = boot().await;
    let client = DaemonClient::new(Some(&h.base_url)).expect("client");
    let cli = parse_cli(&["module", "list"]);
    let code = oneshot::runner::dispatch(cli, client).await;
    assert_eq!(code, oneshot::EXIT_OK);
}

#[tokio::test]
async fn oneshot_server_list_returns_ok_when_empty() {
    let h = boot().await;
    let client = DaemonClient::new(Some(&h.base_url)).expect("client");
    let cli = parse_cli(&["server", "list"]);
    let code = oneshot::runner::dispatch(cli, client).await;
    assert_eq!(code, oneshot::EXIT_OK);
}

#[tokio::test]
async fn oneshot_unknown_server_returns_not_found() {
    let h = boot().await;
    let client = DaemonClient::new(Some(&h.base_url)).expect("client");
    let cli = parse_cli(&["server", "info", "does-not-exist"]);
    let code = oneshot::runner::dispatch(cli, client).await;
    assert_eq!(code, oneshot::EXIT_NOT_FOUND);
}

#[tokio::test]
async fn oneshot_delete_without_yes_is_blocked() {
    let h = boot().await;
    // First create an instance via the API directly (one-shot create flow is
    // intentionally low-priority — we test the destructive guardrail).
    let client = DaemonClient::new(Some(&h.base_url)).expect("client");
    let created = client
        .create_instance(serde_json::json!({
            "name": "delete-me",
            "module_name": "test-module",
        }))
        .await
        .expect("create");
    let _id = created["id"].as_str().unwrap().to_string();

    // Without --yes, delete must be refused with EXIT_ERR (require_yes maps to EXIT_ERR).
    let cli = parse_cli(&["server", "delete", "delete-me"]);
    let code = oneshot::runner::dispatch(cli, client).await;
    assert_eq!(
        code,
        oneshot::EXIT_ERR,
        "delete without --yes must fail with generic error"
    );
}

#[tokio::test]
async fn oneshot_status_returns_ok_when_online() {
    let h = boot().await;
    let client = DaemonClient::new(Some(&h.base_url)).expect("client");
    let cli = parse_cli(&["status"]);
    let code = oneshot::runner::dispatch(cli, client).await;
    assert_eq!(code, oneshot::EXIT_OK);
}

#[tokio::test]
async fn oneshot_server_create_returns_ok() {
    // server create는 신규 명령. 데몬 오프라인일 때 거짓 성공이 아니라 실제로 인스턴스를 만들어야 한다.
    let h = boot().await;
    let client = DaemonClient::new(Some(&h.base_url)).expect("client");
    let cli = parse_cli(&["server", "create", "from-cli", "--module", "test-module"]);
    let code = oneshot::runner::dispatch(cli, client.clone()).await;
    assert_eq!(code, oneshot::EXIT_OK);

    // 그리고 실제로 인스턴스 목록에 잡혀야 한다.
    let listed = client.list_instances().await.expect("list");
    assert!(listed.iter().any(|v| v.get("name").and_then(|n| n.as_str()) == Some("from-cli")));
}

#[tokio::test]
async fn oneshot_server_set_via_pairs_persists() {
    let h = boot().await;
    let client = DaemonClient::new(Some(&h.base_url)).expect("client");
    // 사전 인스턴스 생성
    let _ = client
        .create_instance(serde_json::json!({
            "name": "set-target",
            "module_name": "test-module",
        }))
        .await
        .expect("create");

    let cli = parse_cli(&["server", "set", "set-target", "port=25577"]);
    let code = oneshot::runner::dispatch(cli, client.clone()).await;
    assert_eq!(code, oneshot::EXIT_OK);

    // 데몬에 다시 물어 확인 (long_client로 PATCH가 정상 회수돼야 함).
    let inst_list = client.list_instances().await.expect("list");
    let inst = inst_list
        .iter()
        .find(|v| v.get("name").and_then(|n| n.as_str()) == Some("set-target"))
        .cloned()
        .expect("instance present");
    assert_eq!(inst.get("port").and_then(|v| v.as_u64()), Some(25577));
}

#[tokio::test]
async fn oneshot_status_offline_returns_daemon_offline_exit() {
    // 죽은 포트 → status text 모드도 EXIT_DAEMON_OFFLINE이어야 한다.
    let dead_port = pick_free_port();
    let base_url = format!("http://127.0.0.1:{}", dead_port);
    let client = DaemonClient::new(Some(&base_url)).expect("client");
    let cli = parse_cli(&["status"]);
    let code = oneshot::runner::dispatch(cli, client).await;
    assert_eq!(
        code,
        oneshot::EXIT_DAEMON_OFFLINE,
        "status 명령은 데몬 오프라인 시 무조건 EXIT_DAEMON_OFFLINE(2)여야 한다"
    );
}

#[tokio::test]
async fn oneshot_alias_list_propagates_offline_error() {
    let dead_port = pick_free_port();
    let base_url = format!("http://127.0.0.1:{}", dead_port);
    let client = DaemonClient::new(Some(&base_url)).expect("client");
    let cli = parse_cli(&["alias", "list"]);
    let code = oneshot::runner::dispatch(cli, client).await;
    assert_eq!(code, oneshot::EXIT_DAEMON_OFFLINE);
}

#[tokio::test]
async fn oneshot_offline_daemon_yields_offline_exit_code() {
    // Point the client at an unused port — connection will be refused.
    let dead_port = pick_free_port();
    // Drop any potential listener and immediately reuse the port number.
    let base_url = format!("http://127.0.0.1:{}", dead_port);

    let client = DaemonClient::new(Some(&base_url)).expect("client");
    // `module list` requires a live daemon. With --json we still get exit 2.
    let cli = parse_cli(&["module", "list"]);
    let code = oneshot::runner::dispatch(cli, client).await;
    assert_eq!(
        code,
        oneshot::EXIT_DAEMON_OFFLINE,
        "expected EXIT_DAEMON_OFFLINE (2), got {}",
        code
    );
}
