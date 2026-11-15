//! 통합 테스트가 함께 쓰는 준비 코드 (`mod common;`)
#![allow(dead_code)]

use saba_core::ipc::IPCServer;
use saba_core::supervisor::Supervisor;
use serde_json::Value;
use std::sync::Arc;
use tokio::sync::RwLock;
use tokio::time::{sleep, Duration};

/// 빈 포트 번호. 리스너를 바로 놓으므로 그사이 다른 테스트의 데몬이 같은 번호를 잡을 수 있다:
/// `serve_ipc`는 답한 데몬이 자기 것인지 확인한다
pub fn free_port() -> u16 {
    std::net::TcpListener::bind("127.0.0.1:0").unwrap().local_addr().unwrap().port()
}

/// 이 포트에서 답하는 데몬이 `epoch`(이 테스트가 만든 로그 버퍼의 세대)를 가진 데몬인지.
/// 다른 데몬이 답하거나(포트를 빼앗김) 이 데몬이 바인드에 실패해 끝나면 false
async fn wait_for_own_ipc(base_url: &str, client: &reqwest::Client, epoch: u64, task: &tokio::task::JoinHandle<()>) -> bool {
    for _ in 0..50 {
        if task.is_finished() {
            return false;
        }
        if let Ok(resp) = client.get(format!("{}/api/daemon/console?count=0", base_url)).send().await {
            if resp.status().is_success() {
                let body: Value = resp.json().await.unwrap_or_default();
                return body["epoch"].as_u64() == Some(epoch);
            }
        }
        sleep(Duration::from_millis(300)).await;
    }
    panic!("IPC server did not become ready: {}", base_url);
}

/// 인증을 끈 IPC 서버를 띄워 (base_url, 서버 작업)을 돌려준다. `keep`은 서버 작업이 끝날 때(abort)까지 쥔다.
/// 고른 포트를 그사이 다른 테스트의 데몬이 잡으면 그 데몬에 붙지 않고 다른 포트로 다시 띄운다
pub async fn serve_ipc<K: Send + Sync + 'static>(supervisor: Arc<RwLock<Supervisor>>, keep: Arc<K>) -> (String, tokio::task::JoinHandle<()>) {
    // edition 2024: 환경 변수 쓰기는 unsafe다. 테스트 준비에서만 쓴다
    unsafe { std::env::set_var("SABA_AUTH_DISABLED", "1") };
    let client = saba_chan_updater_lib::tls::http_client_builder().build().unwrap();
    for attempt in 1..=5 {
        let listen_addr = format!("127.0.0.1:{}", free_port());
        let base_url = format!("http://{}", listen_addr);
        let logs = saba_core::daemon_log::DaemonLogBuffer::new();
        let epoch = logs.epoch();
        let server = IPCServer::new(supervisor.clone(), &listen_addr, logs);
        let keep = keep.clone();
        let addr = listen_addr.clone();
        let server_task = tokio::spawn(async move {
            let _keep = keep;
            if let Err(e) = server.start().await {
                panic!("IPC server on {} failed: {:#}", addr, e);
            }
        });
        if wait_for_own_ipc(&base_url, &client, epoch, &server_task).await {
            return (base_url, server_task);
        }
        server_task.abort();
        eprintln!("IPC port {} was taken by another daemon (attempt {}), retrying on another port", listen_addr, attempt);
    }
    panic!("IPC server could not bind a port of its own after 5 attempts");
}
