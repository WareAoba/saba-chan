//! 릴레이 연결을 다시 맺을 때(주소·토큰·설정 변경) 받은 명령이 사라지지 않는지 (가짜 릴레이)
//!
//! 시작하지 않은 명령은 ack하지 않아 릴레이가 새 연결로 다시 보내고,
//! 이미 실행 중이던 명령은 연결이 끝난 뒤에도 결과를 올린다.
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
use std::collections::HashMap;
use std::sync::{Arc, Mutex};
use std::time::Duration;
use tokio::sync::RwLock;

const MODULE: &str = "[module]\nschema_version = 1\nname = \"slowq\"\nversion = \"1.0.0\"\nentry = \"lifecycle.py\"\n\n[[queries]]\nname = \"slow\"\nlabel = \"Slow\"\nfunction = \"slow_query\"\n";
const LIFECYCLE: &str = "import json, sys, time\njson.load(sys.stdin)\nif sys.argv[1] == 'slow_query':\n    time.sleep(3)\nprint(json.dumps({'success': True, 'data': {'ok': 1}}))\n";

/// 계약대로: 전달한 뒤 ack도 결과도 없으면 다시 보낸다 (최대 2번, 간격은 테스트용으로 짧게)
#[derive(Default)]
struct Relay {
    instance: String,
    delivered: bool,
    commands: HashMap<String, Value>,
    redelivered: HashMap<String, u32>,
    acked: Vec<String>,
    results: Vec<String>,
}

type Shared = Arc<Mutex<Relay>>;

async fn poll(State(s): State<Shared>) -> axum::response::Response {
    {
        let mut s = s.lock().unwrap();
        if !s.delivered {
            s.delivered = true;
            let who = |id: &str, payload: Value| json!({ "id": id, "payload": payload, "requestedBy": "user-1", "guildId": "guild-1" });
            let slow = json!({ "action": "query", "id": "server.query:slow", "instance": s.instance, "input": {} });
            let mut batch: Vec<Value> = (1..=4).map(|i| who(&format!("s{i}"), slow.clone())).collect();
            batch.push(who("w", json!({ "action": "raw_command", "text": "list" })));
            for c in &batch {
                s.commands.insert(c["id"].as_str().unwrap().to_string(), c.clone());
            }
            return Json(json!({ "commands": batch })).into_response();
        }
        let again: Vec<String> = s
            .commands
            .keys()
            .filter(|id| !s.acked.contains(id) && !s.results.contains(id) && s.redelivered.get(*id).copied().unwrap_or(0) < 2)
            .cloned()
            .collect();
        if !again.is_empty() {
            let batch: Vec<Value> = again.iter().map(|id| s.commands[id].clone()).collect();
            for id in again {
                *s.redelivered.entry(id).or_default() += 1;
            }
            return Json(json!({ "commands": batch })).into_response();
        }
    }
    tokio::time::sleep(Duration::from_millis(200)).await;
    StatusCode::NO_CONTENT.into_response()
}

async fn ack(State(s): State<Shared>, Json(body): Json<Value>) -> impl IntoResponse {
    let ids: Vec<String> = body["ids"].as_array().unwrap().iter().map(|v| v.as_str().unwrap().to_string()).collect();
    s.lock().unwrap().acked.extend(ids);
    Json(json!({ "acked": 1 }))
}

async fn result(State(s): State<Shared>, Path(id): Path<String>) -> impl IntoResponse {
    s.lock().unwrap().results.push(id);
    StatusCode::OK
}

#[tokio::test]
async fn a_reconnect_neither_drops_waiting_commands_nor_the_results_of_running_ones() {
    std::env::set_var("SABA_AUTH_DISABLED", "1");
    let data_dir = std::env::temp_dir().join(format!("saba-relay-handover-{}", std::process::id()));
    std::fs::create_dir_all(&data_dir).unwrap();
    std::env::set_var("SABA_DATA_DIR", &data_dir);
    std::fs::write(data_dir.join(".node_token"), "sbn_node1.s3cret").unwrap();

    let relay: Shared = Arc::default();
    let app = Router::new()
        .route("/info", get(|| async { Json(json!({ "version": "test", "minAgentVersion": "0.1.0" })) }))
        .route("/heartbeat", post(|| async { Json(json!({ "status": "ok" })) }))
        .route("/poll", get(poll))
        .route("/ack", post(ack))
        .route("/result/:id", post(result))
        .with_state(relay.clone());
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let relay_url = format!("http://{}", listener.local_addr().unwrap());
    tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });
    std::fs::write(data_dir.join("bot-config.json"), json!({ "prefix": "!saba", "cloud": { "relayUrl": relay_url } }).to_string()).unwrap();

    let modules_dir = data_dir.join("modules");
    std::fs::create_dir_all(modules_dir.join("slowq")).unwrap();
    std::fs::write(modules_dir.join("slowq").join("module.toml"), MODULE).unwrap();
    std::fs::write(modules_dir.join("slowq").join("lifecycle.py"), LIFECYCLE).unwrap();
    let instances_dir = data_dir.join("instances");
    std::fs::create_dir_all(&instances_dir).unwrap();
    let mut sup = Supervisor::new_with_instances_dir(modules_dir.to_str().unwrap(), &instances_dir.to_string_lossy());
    sup.initialize().await.unwrap();
    let inst = saba_core::instance::ServerInstance::new("srv", "slowq");
    relay.lock().unwrap().instance = inst.id.clone();
    sup.instance_store.add(inst).unwrap();

    let port = std::net::TcpListener::bind("127.0.0.1:0").unwrap().local_addr().unwrap().port();
    let server = IPCServer::new(Arc::new(RwLock::new(sup)), &format!("127.0.0.1:{}", port), saba_core::daemon_log::DaemonLogBuffer::new());
    let handle = server.clone();
    tokio::spawn(async move {
        let _ = server.start().await;
    });

    // 느린 명령 넷이 자리를 다 차지해 다섯째(w)는 기다린다
    for _ in 0..100 {
        if relay.lock().unwrap().acked.len() >= 4 {
            break;
        }
        tokio::time::sleep(Duration::from_millis(50)).await;
    }
    tokio::time::sleep(Duration::from_millis(300)).await;
    {
        let s = relay.lock().unwrap();
        let mut acked = s.acked.clone();
        acked.sort();
        assert_eq!(acked, vec!["s1", "s2", "s3", "s4"], "only commands that started are acked");
    }

    // 설정이 바뀌어 연결을 다시 맺는다
    saba_core::relay_agent::restart(&handle).await;

    for _ in 0..200 {
        if relay.lock().unwrap().results.len() >= 5 {
            break;
        }
        tokio::time::sleep(Duration::from_millis(100)).await;
    }
    let mut results = relay.lock().unwrap().results.clone();
    results.sort();
    // 결과가 ack보다 먼저 닿으면 다시 온 명령에 결과를 한 번 더 올린다 (릴레이는 처음 것만 쓴다)
    results.dedup();
    assert_eq!(results, vec!["s1", "s2", "s3", "s4", "w"], "a command was lost across the reconnect");
    let _ = std::fs::remove_dir_all(&data_dir);
}
