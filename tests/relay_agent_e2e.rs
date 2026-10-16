//! 릴레이 에이전트 E2E — 가짜 릴레이 서버를 띄워 데몬 에이전트의 왕복을 검증합니다.
//! 하트비트 서명 → 롱폴로 명령 수신 → 채팅 처리 → 결과 반환까지 한 번에 확인합니다.

use axum::{
    extract::{Path, State},
    http::{HeaderMap, StatusCode},
    response::IntoResponse,
    routing::{get, post},
    Json, Router,
};
use hmac::{Hmac, Mac};
use saba_core::ipc::IPCServer;
use saba_core::supervisor::Supervisor;
use serde_json::{json, Value};
use sha2::Sha256;
use std::sync::{Arc, Mutex};
use std::time::Duration;
use tokio::sync::RwLock;

const NODE_ID: &str = "node1";
const SECRET: &str = "s3cret";

#[derive(Default)]
struct Relay {
    heartbeats: Vec<Value>,
    bad_signatures: u32,
    delivered: bool,
    results: Vec<(String, Value)>,
}

type Shared = Arc<Mutex<Relay>>;

fn verify(headers: &HeaderMap, method: &str, path: &str, body: &str) -> bool {
    let h = |k: &str| headers.get(k).and_then(|v| v.to_str().ok()).unwrap_or("").to_string();
    if h("authorization") != format!("Bearer sbn_{}.{}", NODE_ID, SECRET) || h("x-saba-client") != "1" {
        return false;
    }
    let payload = format!("{}\n{}\n{}\n{}\n{}", method, path, h("x-request-timestamp"), h("x-request-nonce"), body);
    let mut mac = Hmac::<Sha256>::new_from_slice(SECRET.as_bytes()).unwrap();
    mac.update(payload.as_bytes());
    hex::encode(mac.finalize().into_bytes()) == h("x-request-signature")
}

async fn info() -> impl IntoResponse {
    Json(json!({ "version": "test", "minAgentVersion": "0.1.0" }))
}

async fn heartbeat(State(s): State<Shared>, headers: HeaderMap, body: String) -> impl IntoResponse {
    let mut s = s.lock().unwrap();
    if !verify(&headers, "POST", "/heartbeat", &body) {
        s.bad_signatures += 1;
        return (StatusCode::UNAUTHORIZED, Json(json!({"error": "bad signature"})));
    }
    s.heartbeats.push(serde_json::from_str(&body).unwrap());
    (StatusCode::OK, Json(json!({ "status": "ok", "sleeping": false })))
}

async fn poll(State(s): State<Shared>, headers: HeaderMap) -> axum::response::Response {
    {
        let mut s = s.lock().unwrap();
        if !verify(&headers, "GET", "/poll", "") {
            s.bad_signatures += 1;
            return StatusCode::UNAUTHORIZED.into_response();
        }
        if !s.delivered {
            s.delivered = true;
            return Json(json!({ "commands": [{
                "id": "req 1",
                "payload": { "action": "raw_command", "text": "list" },
                "requestedBy": "user-1",
                "guildId": "guild-1",
                "channelId": "chan-1",
            }]}))
            .into_response();
        }
    }
    tokio::time::sleep(Duration::from_millis(200)).await;
    StatusCode::NO_CONTENT.into_response()
}

async fn result(State(s): State<Shared>, Path(id): Path<String>, headers: HeaderMap, body: String) -> impl IntoResponse {
    let mut s = s.lock().unwrap();
    // 서명 경로는 인코딩된 그대로여야 한다
    if !verify(&headers, "POST", "/result/req%201", &body) {
        s.bad_signatures += 1;
        return StatusCode::UNAUTHORIZED;
    }
    s.results.push((id, serde_json::from_str(&body).unwrap()));
    StatusCode::OK
}

async fn start_mock_relay(state: Shared) -> String {
    let app = Router::new()
        .route("/info", get(info))
        .route("/heartbeat", post(heartbeat))
        .route("/poll", get(poll))
        .route("/result/:id", post(result))
        .with_state(state);
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });
    format!("http://{}", addr)
}

#[tokio::test]
async fn agent_round_trips_a_relay_command() {
    std::env::set_var("SABA_AUTH_DISABLED", "1");
    let data_dir = std::env::temp_dir().join(format!("saba-relay-e2e-{}", std::process::id()));
    std::fs::create_dir_all(&data_dir).unwrap();
    std::env::set_var("SABA_DATA_DIR", &data_dir);
    std::fs::write(data_dir.join(".node_token"), format!("sbn_{}.{}", NODE_ID, SECRET)).unwrap();

    let relay_state: Shared = Arc::default();
    let relay_url = start_mock_relay(relay_state.clone()).await;
    // 페어링된 노드: 데몬이 뜨자마자 에이전트가 이 릴레이로 붙어야 한다
    std::fs::write(
        data_dir.join("bot-config.json"),
        json!({ "prefix": "!saba", "cloud": { "relayUrl": relay_url } }).to_string(),
    )
    .unwrap();

    let modules_dir = data_dir.join("modules");
    let instances_dir = data_dir.join("instances");
    std::fs::create_dir_all(&modules_dir).unwrap();
    std::fs::create_dir_all(&instances_dir).unwrap();
    let supervisor = Arc::new(RwLock::new(Supervisor::new_with_instances_dir(
        modules_dir.to_str().unwrap(),
        &instances_dir.to_string_lossy(),
    )));
    supervisor.write().await.initialize().await.unwrap();

    let port = std::net::TcpListener::bind("127.0.0.1:0").unwrap().local_addr().unwrap().port();
    let base = format!("http://127.0.0.1:{}", port);
    let server = IPCServer::new(supervisor, &format!("127.0.0.1:{}", port), saba_core::daemon_log::DaemonLogBuffer::new());
    tokio::spawn(async move {
        let _ = server.start().await;
    });

    let client = reqwest::Client::new();
    for _ in 0..50 {
        if client.get(format!("{}/health", base)).send().await.is_ok() {
            break;
        }
        tokio::time::sleep(Duration::from_millis(100)).await;
    }

    // 이미 자동 시작됐으므로 start는 그대로 성공해야 한다
    let started: Value = client.post(format!("{}/api/relay/agent/start", base)).send().await.unwrap().json().await.unwrap();
    assert_eq!(started["success"], true, "{}", started);

    let mut posted = None;
    for _ in 0..100 {
        if let Some(r) = relay_state.lock().unwrap().results.first().cloned() {
            posted = Some(r);
            break;
        }
        tokio::time::sleep(Duration::from_millis(100)).await;
    }
    let (id, body) = posted.expect("agent never posted a result");
    assert_eq!(id, "req 1");
    assert_eq!(body["success"], true);
    let en: Value = serde_json::from_str(include_str!("../locales/en/bot.json")).unwrap();
    assert_eq!(body["data"]["text"], en["list"]["empty"], "{}", body);

    {
        let s = relay_state.lock().unwrap();
        assert_eq!(s.bad_signatures, 0);
        let hb = s.heartbeats.first().expect("no heartbeat");
        assert_eq!(hb["metadata"]["botConfig"]["prefix"], "!saba");
        assert!(hb["agentVersion"].is_string());
    }

    let status: Value = client.get(format!("{}/api/relay/agent", base)).send().await.unwrap().json().await.unwrap();
    assert_eq!(status["running"], true, "{}", status);
    assert!(status["commandsHandled"].as_u64().unwrap() >= 1);

    client.post(format!("{}/api/relay/agent/stop", base)).send().await.unwrap();
    let mut stopped = false;
    for _ in 0..50 {
        let status: Value = client.get(format!("{}/api/relay/agent", base)).send().await.unwrap().json().await.unwrap();
        if status["running"] == false {
            stopped = true;
            break;
        }
        tokio::time::sleep(Duration::from_millis(100)).await;
    }
    assert!(stopped, "agent did not stop");
    let _ = std::fs::remove_dir_all(&data_dir);
}
