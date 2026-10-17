//! ═══════════════════════════════════════════════════════════════════
//! Cloud Agent E2E — Mock Relay 서버와 데몬 cloud_agent 사이 전수검증
//! ═══════════════════════════════════════════════════════════════════
//!
//! 검증 항목:
//! 1. **Heartbeat**: agent가 `POST /heartbeat`를 보내며 HMAC 헤더 4종이 검증된다.
//! 2. **Long-poll**: agent가 `GET /poll`을 보내고, mock이 명령을 돌려주면
//!    agent가 dispatch한 뒤 `POST /result/:id`로 결과를 반환한다.
//! 3. **종료**: `CancellationToken`을 cancel하면 백그라운드 task들이 빠져나간다.
//!
//! Mock relay는 axum 라우터로 띄우며, 실제 릴레이 서버 코드는 사용하지 않는다.
//! 서명 검증 알고리즘은 [`server-chan/relay-server/src/middleware/auth.ts`]와
//! 동일한 로직을 테스트 내부에 포팅하여, 데몬이 보내는 헤더가 서버 검증을
//! 통과하는지 직접 확인한다.

use axum::{
    extract::{Path, State},
    http::{HeaderMap, StatusCode},
    response::IntoResponse,
    routing::{get, post},
    Json, Router,
};
use hmac::{Hmac, Mac};
use saba_core::cloud_agent::{self, auth::ParsedToken, CloudAgentConfig};
use saba_core::supervisor::Supervisor;
use serde_json::{json, Value};
use sha2::Sha256;
use std::sync::Arc;
use std::time::Duration;
use tokio::sync::{Mutex, RwLock};
use tokio_util::sync::CancellationToken;

type HmacSha256 = Hmac<Sha256>;

#[derive(Default)]
struct MockState {
    expected_secret: String,
    heartbeats: Mutex<Vec<Value>>,
    polls_served: Mutex<u32>,
    results: Mutex<Vec<(String, Value)>>,
    /// 한 번만 응답할 명령 큐
    pending_commands: Mutex<Vec<Value>>,
    sig_failures: Mutex<u32>,
}

type SharedState = Arc<MockState>;

fn verify_signature(
    headers: &HeaderMap,
    method: &str,
    path: &str,
    body: &str,
    secret: &str,
) -> bool {
    let ts = headers.get("x-request-timestamp").and_then(|v| v.to_str().ok());
    let nonce = headers.get("x-request-nonce").and_then(|v| v.to_str().ok());
    let sig = headers.get("x-request-signature").and_then(|v| v.to_str().ok());
    let auth = headers.get("authorization").and_then(|v| v.to_str().ok());
    let client = headers.get("x-saba-client").and_then(|v| v.to_str().ok());
    let (Some(ts), Some(nonce), Some(sig), Some(auth), Some(client)) =
        (ts, nonce, sig, auth, client)
    else {
        return false;
    };
    if !auth.starts_with("Bearer sbn_") {
        return false;
    }
    if client != "1" {
        return false;
    }
    let payload = format!("{}\n{}\n{}\n{}\n{}", method, path, ts, nonce, body);
    let mut mac = HmacSha256::new_from_slice(secret.as_bytes()).unwrap();
    mac.update(payload.as_bytes());
    let expected = hex::encode(mac.finalize().into_bytes());
    expected == sig
}

async fn heartbeat_handler(
    State(state): State<SharedState>,
    headers: HeaderMap,
    body: String,
) -> impl IntoResponse {
    if !verify_signature(&headers, "POST", "/heartbeat", &body, &state.expected_secret) {
        *state.sig_failures.lock().await += 1;
        return (StatusCode::UNAUTHORIZED, "bad sig").into_response();
    }
    let parsed: Value = serde_json::from_str(&body).unwrap_or(Value::Null);
    state.heartbeats.lock().await.push(parsed);
    (StatusCode::OK, Json(json!({ "status": "ok" }))).into_response()
}

async fn poll_handler(
    State(state): State<SharedState>,
    headers: HeaderMap,
) -> impl IntoResponse {
    if !verify_signature(&headers, "GET", "/poll", "", &state.expected_secret) {
        *state.sig_failures.lock().await += 1;
        return (StatusCode::UNAUTHORIZED, "bad sig").into_response();
    }
    *state.polls_served.lock().await += 1;
    let pending = std::mem::take(&mut *state.pending_commands.lock().await);
    if pending.is_empty() {
        // 빠르게 빈 응답을 돌려 long-poll 사이클을 짧게
        (StatusCode::OK, Json(json!({ "commands": [] }))).into_response()
    } else {
        (StatusCode::OK, Json(json!({ "commands": pending }))).into_response()
    }
}

async fn result_handler(
    State(state): State<SharedState>,
    Path(req_id): Path<String>,
    headers: HeaderMap,
    body: String,
) -> impl IntoResponse {
    let path = format!("/result/{}", req_id);
    if !verify_signature(&headers, "POST", &path, &body, &state.expected_secret) {
        *state.sig_failures.lock().await += 1;
        return (StatusCode::UNAUTHORIZED, "bad sig").into_response();
    }
    let parsed: Value = serde_json::from_str(&body).unwrap_or(Value::Null);
    state.results.lock().await.push((req_id, parsed));
    (StatusCode::OK, Json(json!({"ok": true}))).into_response()
}

async fn spawn_mock_relay(secret: String) -> (String, SharedState, tokio::task::JoinHandle<()>) {
    let state = Arc::new(MockState {
        expected_secret: secret,
        ..Default::default()
    });
    let app = Router::new()
        .route("/heartbeat", post(heartbeat_handler))
        .route("/poll", get(poll_handler))
        .route("/result/:id", post(result_handler))
        .with_state(state.clone());
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let port = listener.local_addr().unwrap().port();
    let handle = tokio::spawn(async move {
        axum::serve(listener, app).await.ok();
    });
    let url = format!("http://127.0.0.1:{}", port);
    (url, state, handle)
}

fn make_supervisor() -> Arc<RwLock<Supervisor>> {
    let tmp = tempfile::tempdir().unwrap();
    let modules_dir = tmp.path().to_string_lossy().to_string();
    // tempdir을 Box::leak으로 Static으로 만들지는 않고, supervisor만 반환.
    // tmp가 drop되면 디렉토리는 사라지나, Supervisor는 modules_dir 경로를
    // String으로만 보유하므로 테스트 동안 안전하다 (실제 모듈 디스커버는
    // 대상 디렉토리가 비어있어도 빈 Vec를 반환).
    std::mem::forget(tmp);
    Arc::new(RwLock::new(Supervisor::new(&modules_dir)))
}

fn make_config_store() -> Arc<saba_core::config_store::ConfigStore> {
    let tmp = tempfile::tempdir().unwrap();
    let data_dir = tmp.path().to_path_buf();
    // make_supervisor와 동일한 이유로 tempdir을 leak — 테스트 동안 디렉토리 유지.
    std::mem::forget(tmp);
    Arc::new(saba_core::config_store::ConfigStore::new(&data_dir))
}

fn test_token() -> ParsedToken {
    ParsedToken {
        raw: "sbn_test-node.test-secret".to_string(),
        node_id: "test-node".to_string(),
        secret: "test-secret".to_string(),
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn cloud_agent_sends_signed_heartbeat() {
    let token = test_token();
    let (relay_url, state, _server) = spawn_mock_relay(token.secret.clone()).await;
    let supervisor = make_supervisor();
    let shutdown = CancellationToken::new();

    let config = CloudAgentConfig {
        relay_url: relay_url.clone(),
        agent_version: "test-version".to_string(),
        // heartbeat을 즉시 + 매우 짧게 반복하여 빠른 검증
        heartbeat_interval: Duration::from_millis(200),
        poll_timeout: Duration::from_secs(2),
    };
    let handles = cloud_agent::start_with_token(config, token, supervisor, make_config_store(), shutdown.clone());
    assert_eq!(handles.len(), 2, "heartbeat + poll task 2개 spawn");

    // heartbeat 1개 이상 도착할 때까지 대기 (최대 3초)
    let deadline = std::time::Instant::now() + Duration::from_secs(3);
    while state.heartbeats.lock().await.is_empty() {
        if std::time::Instant::now() > deadline {
            break;
        }
        tokio::time::sleep(Duration::from_millis(50)).await;
    }

    shutdown.cancel();
    for h in handles {
        let _ = tokio::time::timeout(Duration::from_secs(2), h).await;
    }

    let beats = state.heartbeats.lock().await;
    assert!(!beats.is_empty(), "최소 1개의 heartbeat이 도착해야 함");
    let first = &beats[0];
    assert_eq!(first["agentVersion"], "test-version");
    assert!(first["metadata"]["modules"].is_array());
    assert!(first["metadata"]["servers"].is_array());

    assert_eq!(*state.sig_failures.lock().await, 0, "서명 검증은 모두 통과해야 함");
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn cloud_agent_dispatches_command_and_posts_result() {
    let token = test_token();
    let (relay_url, state, _server) = spawn_mock_relay(token.secret.clone()).await;
    let supervisor = make_supervisor();
    let shutdown = CancellationToken::new();

    // 미리 명령 1개를 큐에 넣어둔다 — agent의 첫 poll에서 받아갈 것
    state.pending_commands.lock().await.push(json!({
        "id": "req-001",
        "payload": { "action": "ping" },
        "requestedBy": "tester",
    }));

    let config = CloudAgentConfig {
        relay_url,
        agent_version: "test-version".to_string(),
        heartbeat_interval: Duration::from_secs(60), // 이번 테스트에선 heartbeat 비활성화 수준
        poll_timeout: Duration::from_secs(2),
    };
    let handles = cloud_agent::start_with_token(config, token, supervisor, make_config_store(), shutdown.clone());

    // 결과 도착 대기 (최대 5초)
    let deadline = std::time::Instant::now() + Duration::from_secs(5);
    while state.results.lock().await.is_empty() {
        if std::time::Instant::now() > deadline {
            break;
        }
        tokio::time::sleep(Duration::from_millis(50)).await;
    }

    shutdown.cancel();
    for h in handles {
        let _ = tokio::time::timeout(Duration::from_secs(2), h).await;
    }

    let results = state.results.lock().await;
    assert_eq!(results.len(), 1, "ping action 결과 1건이 도착해야 함");
    let (req_id, body) = &results[0];
    assert_eq!(req_id, "req-001");
    assert_eq!(body["success"], true);
    assert_eq!(body["data"]["pong"], true);
    assert_eq!(*state.sig_failures.lock().await, 0);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn cloud_agent_rejects_unknown_action_with_failure() {
    let token = test_token();
    let (relay_url, state, _server) = spawn_mock_relay(token.secret.clone()).await;
    let supervisor = make_supervisor();
    let shutdown = CancellationToken::new();

    state.pending_commands.lock().await.push(json!({
        "id": "req-bad",
        "payload": { "action": "totally_made_up" },
    }));

    let config = CloudAgentConfig {
        relay_url,
        agent_version: "v".to_string(),
        heartbeat_interval: Duration::from_secs(60),
        poll_timeout: Duration::from_secs(2),
    };
    let handles = cloud_agent::start_with_token(config, token, supervisor, make_config_store(), shutdown.clone());

    let deadline = std::time::Instant::now() + Duration::from_secs(5);
    while state.results.lock().await.is_empty() {
        if std::time::Instant::now() > deadline {
            break;
        }
        tokio::time::sleep(Duration::from_millis(50)).await;
    }
    shutdown.cancel();
    for h in handles {
        let _ = tokio::time::timeout(Duration::from_secs(2), h).await;
    }

    let results = state.results.lock().await;
    assert_eq!(results.len(), 1);
    let (id, body) = &results[0];
    assert_eq!(id, "req-bad");
    assert_eq!(body["success"], false);
    assert!(body["data"]["error"].as_str().unwrap_or("").contains("unsupported"));
}

/// 회귀 방지: 서버가 빈 commands 를 즉시 반환하더라도 데몬이 tight-loop 에 빠지지
/// 않고 최소 1s cooldown 을 유지하는지 확인.
///
/// poll_timeout=10s 로 설정하지만, mock 서버는 즉시 빈 응답을 돌려준다.
/// cooldown 없으면 1초당 수십~수백회 폴이 발생하고, cooldown=1s 이면 ~1회/s.
/// jitter(0~2s) 까지 고려해 3초 동안 최대 5회 정도여야 한다.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn cloud_agent_throttles_on_empty_poll_response() {
    let token = test_token();
    let (relay_url, state, _server) = spawn_mock_relay(token.secret.clone()).await;
    let supervisor = make_supervisor();
    let shutdown = CancellationToken::new();

    let config = CloudAgentConfig {
        relay_url,
        agent_version: "v".to_string(),
        // heartbeat 은 이번 테스트와 무관하게 길게
        heartbeat_interval: Duration::from_secs(60),
        // poll_timeout 은 의도적으로 길게 — mock 서버가 즉시 빈 응답을 돌려주므로
        // 데몬이 cooldown 없으면 tight-loop 에 빠진다.
        poll_timeout: Duration::from_secs(10),
    };
    let handles = cloud_agent::start_with_token(config, token, supervisor, make_config_store(), shutdown.clone());

    // 3.5초 대기 — startup jitter(최대 2s) + 약 1초 폴링 cooldown 1~2회 발생 예상
    tokio::time::sleep(Duration::from_millis(3_500)).await;

    shutdown.cancel();
    for h in handles {
        let _ = tokio::time::timeout(Duration::from_secs(2), h).await;
    }

    let polls = *state.polls_served.lock().await;
    // cooldown 없으면 수십~수백 회. 정상은 0~3회 (jitter 영향).
    assert!(
        polls <= 5,
        "tight-loop 회귀: 3.5초 동안 {} 회 폴 (cooldown 미적용 의심)",
        polls
    );
    assert_eq!(*state.sig_failures.lock().await, 0);
}

/// 페어링 직후 데몬 재시작 없이 에이전트가 활성화되는지 검증 (핫 리로드).
///
/// 회귀 배경: 페어링으로 `.node_token`이 저장되어도 에이전트는 데몬 재시작
/// 전까지 시작되지 않아, 하트비트 미전송 → 릴레이가 HOST_OFFLINE을 반환했다.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn cloud_agent_runtime_hot_starts_after_token_saved() {
    let token = test_token();
    let (relay_url, state, _server) = spawn_mock_relay(token.secret.clone()).await;
    let supervisor = make_supervisor();
    let shutdown = CancellationToken::new();

    // 토큰 파일이 저장될 data_dir (아직 토큰 없음)
    let tmp = tempfile::tempdir().unwrap();
    let data_dir = tmp.path().to_path_buf();

    let config = CloudAgentConfig {
        relay_url,
        agent_version: "hot-reload-test".to_string(),
        heartbeat_interval: Duration::from_millis(200),
        poll_timeout: Duration::from_secs(2),
    };
    let runtime = cloud_agent::CloudAgentRuntime::new(
        config,
        supervisor,
        make_config_store(),
        data_dir.clone(),
        shutdown.clone(),
    );

    // 1) 토큰 없음 → 시작 실패 (idle)
    assert!(!runtime.restart_from_file().await, "토큰 없이는 시작되지 않아야 함");
    assert!(!runtime.is_running().await);

    // 2) 페어링 완료 시뮬레이션: 토큰 파일 저장 → 핫 스타트
    std::fs::write(data_dir.join(".node_token"), &token.raw).unwrap();
    assert!(runtime.restart_from_file().await, "토큰 저장 후 핫 스타트되어야 함");
    assert!(runtime.is_running().await);

    // 하트비트가 실제로 도착하는지 확인 (최대 3초)
    let deadline = std::time::Instant::now() + Duration::from_secs(3);
    while state.heartbeats.lock().await.is_empty() {
        if std::time::Instant::now() > deadline {
            break;
        }
        tokio::time::sleep(Duration::from_millis(50)).await;
    }
    assert!(
        !state.heartbeats.lock().await.is_empty(),
        "핫 스타트 후 하트비트가 도착해야 함"
    );

    // 3) 재시작(re-pair) — 기존 태스크가 정지되고 새 인스턴스로 교체
    assert!(runtime.restart_from_file().await);
    assert!(runtime.is_running().await);

    // 4) 토큰 삭제 후 restart → 정지 상태
    std::fs::remove_file(data_dir.join(".node_token")).unwrap();
    assert!(!runtime.restart_from_file().await, "토큰 삭제 후에는 정지되어야 함");
    assert!(!runtime.is_running().await);

    // 5) stop은 멱등
    runtime.stop().await;
    assert!(!runtime.is_running().await);

    shutdown.cancel();
    assert_eq!(*state.sig_failures.lock().await, 0);
}



/// 인스턴스별 디스코드 설정(instanceSettings) 집행 E2E —
/// 릴레이 poll 응답의 guildId/requestedBy가 디스패처 컨텍스트로 전달되어
/// 데몬이 노드/유저 권한을 실제로 판정하는지 전체 배선을 검증한다.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn cloud_agent_enforces_instance_permissions() {
    use saba_core::config_store::InstanceDiscordSettings;
    use saba_core::instance::ServerInstance;
    use std::collections::HashMap;

    let token = test_token();
    let (relay_url, state, _server) = spawn_mock_relay(token.secret.clone()).await;

    // 인스턴스 1개("pal-1")를 가진 supervisor (tempdir 격리)
    let modules_tmp = tempfile::tempdir().unwrap();
    let instances_tmp = tempfile::tempdir().unwrap();
    let mut sup = Supervisor::new_with_instances_dir(
        &modules_tmp.path().to_string_lossy(),
        &instances_tmp.path().to_string_lossy(),
    );
    let inst: ServerInstance = serde_json::from_value(json!({
        "id": "inst-pal-1",
        "name": "pal-1",
        "module_name": "palworld",
        "executable_path": null,
        "working_dir": null,
        "auto_detect": false,
        "process_name": null,
        "port": null,
        "rcon_port": null,
        "rcon_password": null,
    }))
    .unwrap();
    sup.instance_store.add(inst).unwrap();
    let supervisor = Arc::new(RwLock::new(sup));
    std::mem::forget(modules_tmp);
    std::mem::forget(instances_tmp);

    // bot-config: pal-1은 guild-a에서만 사용 가능, stop은 user-bad 차단
    let config_store = make_config_store();
    let mut bot_cfg = saba_core::config_store::BotConfig::default();
    let mut blocked = HashMap::new();
    blocked.insert("stop".to_string(), vec!["user-bad".to_string()]);
    bot_cfg.instance_settings.insert(
        "inst-pal-1".to_string(),
        InstanceDiscordSettings {
            allowed_nodes: Some(vec!["guild-a".to_string()]),
            blocked_users: blocked,
            ..Default::default()
        },
    );
    config_store.set_bot_config(bot_cfg).await.unwrap();

    // 명령 2건: (1) 비허용 노드에서 접근, (2) 허용 노드에서 차단 유저가 stop
    state.pending_commands.lock().await.push(json!({
        "id": "req-node-blocked",
        "payload": { "action": "raw_command", "text": "pal-1 status" },
        "guildId": "guild-b",
        "requestedBy": "user-good",
    }));
    state.pending_commands.lock().await.push(json!({
        "id": "req-user-blocked",
        "payload": { "action": "raw_command", "text": "pal-1 stop" },
        "guildId": "guild-a",
        "requestedBy": "user-bad",
    }));

    let shutdown = CancellationToken::new();
    let config = CloudAgentConfig {
        relay_url,
        agent_version: "perm-test".to_string(),
        heartbeat_interval: Duration::from_secs(60),
        poll_timeout: Duration::from_secs(2),
    };
    let handles =
        cloud_agent::start_with_token(config, token, supervisor, config_store, shutdown.clone());

    // 결과 2건 도착 대기 (최대 5초)
    let deadline = std::time::Instant::now() + Duration::from_secs(5);
    while state.results.lock().await.len() < 2 {
        if std::time::Instant::now() > deadline {
            break;
        }
        tokio::time::sleep(Duration::from_millis(50)).await;
    }
    shutdown.cancel();
    for h in handles {
        let _ = tokio::time::timeout(Duration::from_secs(2), h).await;
    }

    let results = state.results.lock().await;
    assert_eq!(results.len(), 2, "권한 거부 결과 2건이 도착해야 함");

    let find = |id: &str| -> &Value {
        &results.iter().find(|(rid, _)| rid == id).expect("result missing").1
    };

    let node_blocked = find("req-node-blocked");
    assert_eq!(node_blocked["success"], false);
    assert_eq!(node_blocked["data"]["code"], "NODE_NOT_ALLOWED");
    assert_eq!(node_blocked["data"]["args"]["name"], "pal-1");

    let user_blocked = find("req-user-blocked");
    assert_eq!(user_blocked["success"], false);
    assert_eq!(user_blocked["data"]["code"], "USER_BLOCKED");
    assert_eq!(user_blocked["data"]["args"]["command"], "stop");
}
