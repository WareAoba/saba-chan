//! 릴레이 에이전트가 오래 걸리는 명령 하나 때문에 뒤의 명령을 붙잡지 않는지 (가짜 릴레이)
//!
//! 환경 변수(`SABA_DATA_DIR`)를 바꾸므로 다른 E2E와 다른 바이너리로 둔다.

use axum::{
    extract::{Path, State},
    http::StatusCode,
    response::IntoResponse,
    routing::{get, post},
    Json, Router,
};
use saba_core::ipc::IPCServer;
use saba_core::supervisor::Supervisor;
use serde_json::{json, Value};
use std::sync::{Arc, Mutex};
use std::time::Duration;
use tokio::sync::RwLock;

const MODULE: &str = "[module]\nschema_version = 1\nname = \"slowq\"\nversion = \"1.0.0\"\nentry = \"lifecycle.py\"\n\n[[queries]]\nname = \"slow\"\nlabel = \"Slow\"\nfunction = \"slow_query\"\n";
const LIFECYCLE: &str = "import json, sys, time\njson.load(sys.stdin)\nif sys.argv[1] == 'slow_query':\n    time.sleep(4)\nprint(json.dumps({'success': True, 'data': {'ok': 1}}))\n";

#[derive(Default)]
struct Relay {
    instance: String,
    delivered: bool,
    results: Vec<String>,
}

type Shared = Arc<Mutex<Relay>>;

async fn poll(State(s): State<Shared>) -> axum::response::Response {
    {
        let mut s = s.lock().unwrap();
        if !s.delivered {
            s.delivered = true;
            let who = |id: &str, payload: Value| json!({ "id": id, "payload": payload, "requestedBy": "user-1", "guildId": "guild-1" });
            return Json(json!({ "commands": [
                who("slow", json!({ "action": "query", "id": "server.query:slow", "instance": s.instance, "input": {} })),
                who("fast", json!({ "action": "plan", "text": "list" })),
            ]}))
            .into_response();
        }
    }
    tokio::time::sleep(Duration::from_millis(200)).await;
    StatusCode::NO_CONTENT.into_response()
}

async fn result(State(s): State<Shared>, Path(id): Path<String>) -> impl IntoResponse {
    s.lock().unwrap().results.push(id);
    StatusCode::OK
}

#[tokio::test]
async fn a_slow_command_does_not_hold_up_the_ones_behind_it() {
    // 실패하면 데몬 로그(격리 준비·검사에 걸린 시간 등)를 함께 보인다
    let _ = tracing_subscriber::fmt().with_test_writer().with_env_filter("saba_core=info").try_init();
    std::env::set_var("SABA_AUTH_DISABLED", "1");
    let data_dir = std::env::temp_dir().join(format!("saba-relay-conc-{}", std::process::id()));
    std::fs::create_dir_all(&data_dir).unwrap();
    std::env::set_var("SABA_DATA_DIR", &data_dir);
    std::fs::write(data_dir.join(".node_token"), "sbn_node1.s3cret").unwrap();

    let relay: Shared = Arc::default();
    let app = Router::new()
        .route("/info", get(|| async { Json(json!({ "version": "test", "minAgentVersion": "0.1.0" })) }))
        .route("/heartbeat", post(|| async { Json(json!({ "status": "ok" })) }))
        .route("/poll", get(poll))
        .route("/result/{id}", post(result))
        .with_state(relay.clone());
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let relay_url = format!("http://{}", listener.local_addr().unwrap());
    tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });
    std::fs::write(data_dir.join("bot-config.json"), json!({ "prefix": "!saba", "cloud": { "relayUrl": relay_url } }).to_string()).unwrap();

    let modules_dir = data_dir.join("modules");
    std::fs::create_dir_all(modules_dir.join("slowq")).unwrap();
    std::fs::write(modules_dir.join("slowq").join("module.toml"), MODULE).unwrap();
    std::fs::write(modules_dir.join("slowq").join("lifecycle.py"), LIFECYCLE).unwrap();
    std::env::set_var("SABA_MODULES_PATH", &modules_dir);
    saba_core::sandbox::set_consent(saba_core::sandbox::Consent::Unisolated, "module-slowq", true).unwrap();
    let instances_dir = data_dir.join("instances");
    std::fs::create_dir_all(&instances_dir).unwrap();
    let mut sup = Supervisor::new_with_instances_dir(modules_dir.to_str().unwrap(), &instances_dir.to_string_lossy());
    sup.initialize().await.unwrap();
    let inst = saba_core::instance::ServerInstance::new("srv", "slowq");
    relay.lock().unwrap().instance = inst.id.clone();
    sup.instance_store.add(inst).unwrap();

    let port = std::net::TcpListener::bind("127.0.0.1:0").unwrap().local_addr().unwrap().port();
    let server = IPCServer::new(Arc::new(RwLock::new(sup)), &format!("127.0.0.1:{}", port), saba_core::daemon_log::DaemonLogBuffer::new());
    tokio::spawn(async move {
        let _ = server.start().await;
    });

    // 확인되지 않은 모듈이라 첫 호출이 격리용 venv를 만든다(Windows CI에서 15초 넘게 걸림).
    // 순서만 보므로 느린 명령이 끝날 때까지 넉넉히 기다린다.
    for _ in 0..900 {
        if relay.lock().unwrap().results.len() >= 2 {
            break;
        }
        tokio::time::sleep(Duration::from_millis(100)).await;
    }
    let results = relay.lock().unwrap().results.clone();
    assert_eq!(results, vec!["fast".to_string(), "slow".to_string()], "the quick command waited for the slow one");
    let _ = std::fs::remove_dir_all(&data_dir);
}
