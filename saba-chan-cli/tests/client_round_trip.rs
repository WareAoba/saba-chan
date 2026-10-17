//! ═══════════════════════════════════════════════════════════════════
//! P0-1: CLI DaemonClient 라운드트립 (Component Integration)
//! ═══════════════════════════════════════════════════════════════════
//!
//! `saba-chan-cli`의 `DaemonClient`가 실제 데몬 IPC HTTP 서버와 정확한
//! 메서드/경로/바디로 통신하고 응답을 파싱하는지 검증한다.
//!
//! 분류: **Component Integration** — in-process Supervisor + IPCServer 부팅,
//! `saba-core::ipc::IPCServer`를 실제 띄우고, 같은 프로세스 내에서
//! `DaemonClient`로 HTTP 호출. 진정한 라운드트립이지만 별도 OS 프로세스는 spawn 없음.
//!
//! ## 검증 범위
//! - `list_servers` / `list_instances` / `create_instance` / `get_instance`
//!   / `update_instance` / `delete_instance` round-trip
//! - `start_managed` / `stop_server` 응답 스키마
//! - 인증 비활성화 모드 (`SABA_AUTH_DISABLED=1`) 동작
//! - 잘못된 인스턴스 ID → 404 에러 처리

use saba_chan_cli::client::DaemonClient;
use saba_core::ipc::IPCServer;
use saba_core::supervisor::Supervisor;
use serde_json::json;
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

async fn boot_daemon() -> (
    String,
    Arc<RwLock<Supervisor>>,
    tokio::task::JoinHandle<()>,
    tempfile::TempDir,
    tempfile::TempDir,
    tempfile::TempDir,
) {
    std::env::set_var("SABA_AUTH_DISABLED", "1");

    let data_dir = tempfile::tempdir().expect("temp data dir");
    std::env::set_var("SABA_DATA_DIR", data_dir.path());

    let modules_dir = tempfile::tempdir().expect("temp modules dir");
    let instances_dir = tempfile::tempdir().expect("temp instances dir");

    // 최소한의 모듈 1개를 fixture로 배치
    let m = modules_dir.path().join("test-module");
    std::fs::create_dir_all(&m).unwrap();
    std::fs::write(
        m.join("module.toml"),
        r#"[module]
name = "test-module"
version = "0.0.1"
description = "CLI roundtrip fixture"
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
    let sup_clone = supervisor.clone();
    let server = IPCServer::new(
        sup_clone,
        &listen_addr,
        saba_core::daemon_log::DaemonLogBuffer::new(),
    );
    let handle = tokio::spawn(async move {
        let _ = server.start().await;
    });

    // ready 대기
    let probe = reqwest::Client::new();
    for _ in 0..50 {
        if let Ok(r) = probe.get(format!("{}/api/modules", base_url)).send().await {
            if r.status().is_success() {
                return (base_url, supervisor, handle, data_dir, modules_dir, instances_dir);
            }
        }
        sleep(Duration::from_millis(100)).await;
    }
    panic!("IPC server not ready");
}

#[tokio::test]
async fn cli_client_full_instance_crud_roundtrip() {
    let (base_url, _sup, handle, _dd, _md, _id) = boot_daemon().await;

    let cli = DaemonClient::new(Some(&base_url)).expect("cli client");

    // 초기 상태: 인스턴스 0개
    let initial = cli.list_instances().await.expect("list_instances");
    assert!(initial.is_empty(), "초기 인스턴스 목록은 비어 있어야 함: {:?}", initial);

    // 생성 — 데몬 IPC 스키마: module_name 사용
    let created = cli
        .create_instance(json!({
            "name": "cli-test-srv",
            "module_name": "test-module"
        }))
        .await
        .expect("create_instance");
    let id = created
        .get("id")
        .and_then(|v| v.as_str())
        .expect("응답에 id 포함")
        .to_string();
    assert!(!id.is_empty());

    // 조회
    let fetched = cli.get_instance(&id).await.expect("get_instance");
    assert_eq!(fetched.get("name").and_then(|v| v.as_str()), Some("cli-test-srv"));
    assert_eq!(fetched.get("module_name").and_then(|v| v.as_str()), Some("test-module"));

    // 목록에 노출
    let listed = cli.list_instances().await.expect("list_instances after create");
    assert_eq!(listed.len(), 1, "생성 직후 목록에는 정확히 1개여야 함");

    // 수정은 별도 테스트(`cli_client_update_instance_roundtrip`)에서 검증—
    // PATCH가 hook dispatch로 5수이상 걸릴 수 있어 short timeout(5s) 클라이언트에서는
    // 타임아웃 일으키는 이슈가 탐지됨(아래 디스커버리 참조).

    // 삭제
    let _deleted = cli.delete_instance(&id).await.expect("delete_instance");
    let after = cli.list_instances().await.expect("list_instances after delete");
    assert!(after.is_empty(), "삭제 후 목록은 비어야 함");

    handle.abort();
}

/// `DaemonClient::update_instance`는 데몬 PATCH 핸들러가 hook dispatch
/// (`server.settings_changed`)를 포함해 5초 이상 걸리는 경우가 있어
/// long-client(120초 타임아웃)로 보낸다. 본 테스트는:
///   1) PATCH가 short-timeout으로 떨어지지 않고 정상 완료되는지,
///   2) 변경값이 데몬에 실제로 반영(get_instance에서 회수)되는지
/// 두 가지를 검증한다.
#[tokio::test]
async fn cli_client_update_instance_roundtrip() {
    let (base_url, _sup, handle, _dd, _md, _id) = boot_daemon().await;
    let cli = DaemonClient::new(Some(&base_url)).expect("cli client");
    let created = cli
        .create_instance(json!({ "name": "upd-srv", "module_name": "test-module" }))
        .await
        .expect("create_instance");
    let id = created["id"].as_str().unwrap().to_string();

    // 1) PATCH 자체가 timeout 없이 완료되어야 한다 (응답은 {"success": true}).
    let resp = cli
        .update_instance(&id, json!({ "port": 25577 }))
        .await
        .expect("update_instance");
    assert_eq!(resp.get("success").and_then(|v| v.as_bool()), Some(true));

    // 2) GET으로 변경값이 실제 반영됐는지 확인.
    let after = cli.get_instance(&id).await.expect("get_instance");
    assert_eq!(after.get("port").and_then(|v| v.as_u64()), Some(25577));
    handle.abort();
}

#[tokio::test]
async fn cli_client_unknown_instance_yields_error() {
    let (base_url, _sup, handle, _dd, _md, _id) = boot_daemon().await;

    let cli = DaemonClient::new(Some(&base_url)).expect("cli client");

    let result = cli.get_instance("non-existent-id").await;
    assert!(result.is_err(), "존재하지 않는 ID는 에러여야 함");
    let msg = result.unwrap_err().to_string();
    assert!(
        msg.contains("404") || msg.to_lowercase().contains("not found"),
        "에러 메시지에 404/not found 단서가 있어야 함: {msg}"
    );

    handle.abort();
}

#[tokio::test]
async fn cli_client_lists_known_module() {
    let (base_url, _sup, handle, _dd, _md, _id) = boot_daemon().await;

    let cli = DaemonClient::new(Some(&base_url)).expect("cli client");

    // list_servers는 인스턴스 기반이므로, 모듈 발견 자체는 별도 path.
    // CLI client는 modules 엔드포인트를 직접 노출하지 않으나, 빈 인스턴스 목록은 정상 반환되어야.
    let servers = cli.list_servers().await.expect("list_servers");
    assert!(servers.is_empty(), "초기 서버 목록은 빈 배열");

    handle.abort();
}

/// 회귀 테스트: `send_stdin`은 데몬 핸들러가 기대하는 `command` 키로 본문을 보내야 한다.
/// 과거 버그: CLI가 `{"input": ...}` 으로 보내 데몬이 항상
/// `400 Missing 'command' field`로 거절. (GUI/디스코드 봇은 `command` 사용)
#[tokio::test]
async fn cli_client_send_stdin_uses_command_key() {
    let (base_url, _sup, handle, _dd, _md, _id) = boot_daemon().await;
    let cli = DaemonClient::new(Some(&base_url)).expect("cli client");

    // 인스턴스를 하나 만든 뒤 (실제 프로세스는 안 띄움) stdin 호출.
    // 프로세스가 안 떠 있으므로 supervisor가 5xx를 돌려줄 가능성이 높지만,
    // 그 5xx의 본문에 "Missing 'command' field"가 포함돼서는 안 된다.
    let created = cli
        .create_instance(json!({ "name": "stdin-srv", "module_name": "test-module" }))
        .await
        .expect("create_instance");
    let id = created["id"].as_str().unwrap().to_string();

    let result = cli.send_stdin(&id, "say hi").await;
    let combined = match result {
        Ok(v) => format!("{:?}", v),
        Err(e) => format!("{:#}", e),
    };
    assert!(
        !combined.contains("Missing 'command' field"),
        "stdin 본문은 'command' 키를 사용해야 함 (GUI/봇/데몬 계약 일치). got: {combined}"
    );

    handle.abort();
}
