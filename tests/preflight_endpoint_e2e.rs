//! Integration test for `GET /api/runtime/preflight`.
//!
//! Boots an in-process IPC server, mutates the preflight tracker
//! directly, and asserts the endpoint surfaces the state correctly.
//!
//! ## Why this exists
//!
//! The preflight tracker has unit tests (saba-core lib), but those
//! don't exercise the HTTP endpoint. A wiring mistake — wrong route,
//! missing state extractor, broken JSON serialization — would be
//! invisible to the unit tests yet break the GUI's preparing ring.

use std::sync::Arc;
use std::time::Duration;

use saba_core::ipc::IPCServer;
use saba_core::supervisor::Supervisor;
use tokio::sync::RwLock;

fn pick_free_port() -> u16 {
    let l = std::net::TcpListener::bind("127.0.0.1:0").expect("bind ephemeral port");
    l.local_addr().expect("local_addr").port()
}

async fn boot() -> (String, IPCServer) {
    std::env::set_var("SABA_AUTH_DISABLED", "1");
    let supervisor = Arc::new(RwLock::new(Supervisor::new("./modules")));
    let port = pick_free_port();
    let addr = format!("127.0.0.1:{}", port);
    let server = IPCServer::new(
        supervisor,
        &addr,
        saba_core::daemon_log::DaemonLogBuffer::new(),
    );
    let server_clone = server.clone();
    tokio::spawn(async move {
        let _ = server_clone.start().await;
    });

    // Wait for /health to respond.
    let client = reqwest::Client::new();
    let base = format!("http://{}", addr);
    for _ in 0..50 {
        if client
            .get(format!("{}/health", base))
            .timeout(Duration::from_millis(200))
            .send()
            .await
            .map(|r| r.status().is_success())
            .unwrap_or(false)
        {
            return (base, server);
        }
        tokio::time::sleep(Duration::from_millis(50)).await;
    }
    panic!("IPC server did not become ready");
}

#[tokio::test]
async fn preflight_endpoint_returns_idle_state_when_untouched() {
    let (base, _server) = boot().await;
    let resp = reqwest::get(format!("{}/api/runtime/preflight", base))
        .await
        .expect("request");
    assert!(resp.status().is_success(), "status: {}", resp.status());
    let body: serde_json::Value = resp.json().await.expect("json");
    assert_eq!(body["phase"], "preflight-idle");
    assert_eq!(body["percent"], 0);
    assert_eq!(body["complete"], false);
    assert!(body["error"].is_null());
    // Components should be present and all "pending".
    let components = &body["components"];
    assert_eq!(components["node"], "pending");
    assert_eq!(components["python"], "pending");
}

#[tokio::test]
async fn preflight_endpoint_reflects_tracker_mutations() {
    let (base, server) = boot().await;

    server
        .preflight_tracker
        .set_phase(
            saba_core::runtime_preflight::Phase::Python,
            "Installing python deps...",
        )
        .await;
    server.preflight_tracker.set_component("node", "ok").await;
    server
        .preflight_tracker
        .set_component("python", "running")
        .await;

    let resp = reqwest::get(format!("{}/api/runtime/preflight", base))
        .await
        .expect("request");
    let body: serde_json::Value = resp.json().await.expect("json");
    assert_eq!(body["phase"], "preflight-python");
    assert_eq!(body["message"], "Installing python deps...");
    assert_eq!(body["components"]["node"], "ok");
    assert_eq!(body["components"]["python"], "running");
    assert_eq!(body["complete"], false);
}

#[tokio::test]
async fn preflight_endpoint_reports_completion() {
    let (base, server) = boot().await;
    server.preflight_tracker.finish_ok().await;

    let resp = reqwest::get(format!("{}/api/runtime/preflight", base))
        .await
        .expect("request");
    let body: serde_json::Value = resp.json().await.expect("json");
    assert_eq!(body["phase"], "preflight-done");
    assert_eq!(body["complete"], true);
    assert_eq!(body["percent"], 100);
}

#[tokio::test]
async fn preflight_endpoint_reports_failure_with_error() {
    let (base, server) = boot().await;
    server
        .preflight_tracker
        .finish_failed("npm install crashed: ENOSPC")
        .await;

    let resp = reqwest::get(format!("{}/api/runtime/preflight", base))
        .await
        .expect("request");
    let body: serde_json::Value = resp.json().await.expect("json");
    assert_eq!(body["phase"], "preflight-failed");
    assert_eq!(body["complete"], true);
    // ★ The error string must reach the GUI verbatim — without it the
    //   "no error message" production complaint resurfaces.
    assert_eq!(body["error"], "npm install crashed: ENOSPC");
}
