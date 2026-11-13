use super::*;
use super::heartbeat::*;

/// 서버 주소(`server.address`)는 릴레이가 하트비트 답에 실어 준 접속 주소를 쓴다. IP가 아닌 값·없는 값은 모른다로 둔다
/// F8: 화면이 보일 상태 하나를 데몬이 정한다 (클라이언트가 오류 코드를 따로 해석하지 않는다)
#[test]
fn agent_state_is_one_value_for_every_client() {
    let state = |s: AgentStatus| agent_state(&s);
    assert_eq!(state(AgentStatus { running: true, ..Default::default() }), "running");
    assert_eq!(state(AgentStatus { running: true, last_error_code: Some(TOO_OLD), ..Default::default() }), "too_old");
    let rejected = HeartbeatReject { code: "TOO_BIG".into(), field: "x".into(), max: None, actual: None };
    assert_eq!(state(AgentStatus { running: true, heartbeat_rejected: Some(rejected), ..Default::default() }), "rejected");
    assert_eq!(state(AgentStatus { last_error_code: Some("token_replaced"), ..Default::default() }), "replaced");
    assert_eq!(state(AgentStatus { last_error_code: Some("invalid_relay_url"), ..Default::default() }), "invalid_relay_url");
    assert_eq!(state(AgentStatus::default()), "stopped");
}

#[test]
fn the_address_the_relay_saw_is_read_from_the_heartbeat_reply() {
    assert_eq!(relay_seen_address(&json!({ "status": "ok", "address": "203.0.113.5" })).as_deref(), Some("203.0.113.5"));
    assert_eq!(relay_seen_address(&json!({ "address": "2001:db8::1" })).as_deref(), Some("2001:db8::1"));
    assert_eq!(relay_seen_address(&json!({ "address": "evil.example:1" })), None);
    assert_eq!(relay_seen_address(&json!({ "status": "ok" })), None);
}

/// 배포판은 os-release의 ID·VERSION_ID·PRETTY_NAME으로 싣는다 (앱 카드의 배포판 로고). 릴레이가 받지 않는 ID면 싣지 않는다
#[test]
fn the_linux_distro_comes_from_os_release() {
    let ubuntu = "PRETTY_NAME=\"Ubuntu 24.04.1 LTS\"\nNAME=\"Ubuntu\"\nVERSION_ID=\"24.04\"\nID=ubuntu\nID_LIKE=debian\n";
    assert_eq!(parse_os_release(ubuntu), Some(json!({ "id": "ubuntu", "version": "24.04", "name": "Ubuntu 24.04.1 LTS" })));
    assert_eq!(parse_os_release("ID=arch\nPRETTY_NAME=\"Arch Linux\"\n"), Some(json!({ "id": "arch", "name": "Arch Linux" })));
    assert_eq!(parse_os_release("ID=\"<svg>\"\n"), None);
    assert_eq!(parse_os_release("NAME=x\n"), None);
}

/// 13절: 업데이트로 내려가기 직전 릴레이에 "업데이트 중"을 알린다 (세션의 주소·토큰으로 서명). 연결돼 있지 않으면 보내지 않는다
#[tokio::test]
async fn the_relay_is_told_before_the_daemon_goes_down_for_an_update() {
    use axum::routing::post;
    let seen: Arc<Mutex<Vec<(String, bool)>>> = Arc::default();
    let log = seen.clone();
    let app = axum::Router::new().route(
        "/v1/host/updating",
        post(move |headers: axum::http::HeaderMap, body: String| {
            let log = log.clone();
            async move {
                let signed = headers.keys().any(|k| k.as_str().to_ascii_lowercase().contains("signature"));
                log.lock().unwrap().push((body, signed));
                axum::Json(json!({ "ok": true, "untilSeconds": 210 }))
            }
        }),
    );
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let base = format!("http://{}", listener.local_addr().unwrap());
    tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });

    let agent = RelayAgent::new();
    assert!(agent.notify_updating(90).await.is_err(), "not connected");
    {
        let mut inner = agent.lock();
        inner.cancel = Some(CancellationToken::new());
        inner.endpoint = Some((base, "sbn_host.secret".into()));
    }
    agent.notify_updating(90).await.unwrap();
    let seen = seen.lock().unwrap().clone();
    assert_eq!(seen.len(), 1);
    assert_eq!(serde_json::from_str::<Value>(&seen[0].0).unwrap(), json!({ "etaSeconds": 90 }));
    assert!(seen[0].1, "signed like the heartbeat");
    agent.stop();
    assert!(agent.notify_updating(90).await.is_err(), "a stopped session sends nothing");
}

/// 사용자가 사바쨩을 끄면 내려가기 직전 릴레이에 알린다 (`POST /v1/host/stopping`, 본문 없음): 릴레이가 "PC 연결 끊김"으로 보지 않는다.
/// 업데이트로 내려가는 중이면 이미 알렸으므로 보내지 않는다
#[tokio::test]
async fn the_relay_is_told_when_the_daemon_is_shut_down_on_purpose() {
    use axum::routing::post;
    let seen: Arc<Mutex<Vec<(String, bool)>>> = Arc::default();
    let log = seen.clone();
    let app = axum::Router::new()
        .route(
            "/v1/host/stopping",
            post(move |headers: axum::http::HeaderMap, body: String| {
                let log = log.clone();
                async move {
                    let signed = headers.keys().any(|k| k.as_str().to_ascii_lowercase().contains("signature"));
                    log.lock().unwrap().push((body, signed));
                    axum::Json(json!({ "ok": true }))
                }
            }),
        )
        .route("/v1/host/updating", post(|| async { axum::Json(json!({ "ok": true, "untilSeconds": 210 })) }));
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let base = format!("http://{}", listener.local_addr().unwrap());
    tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });

    let connected = || {
        let agent = RelayAgent::new();
        let mut inner = agent.lock();
        inner.cancel = Some(CancellationToken::new());
        inner.endpoint = Some((base.clone(), "sbn_host.secret".into()));
        drop(inner);
        agent
    };
    let agent = RelayAgent::new();
    agent.notify_stopping().await;
    assert!(seen.lock().unwrap().is_empty(), "not connected");

    let agent = connected();
    agent.notify_stopping().await;
    assert_eq!(seen.lock().unwrap().clone(), vec![(String::new(), true)], "no body, signed like the heartbeat");
    assert!(agent.going_down.load(std::sync::atomic::Ordering::SeqCst), "no heartbeat brings it back online afterwards");

    let agent = connected();
    agent.notify_updating(90).await.unwrap();
    agent.notify_stopping().await;
    assert_eq!(seen.lock().unwrap().len(), 1, "going down for an update: the relay keeps its update window");
}

/// 릴레이가 이 PC의 버전을 받지 않으면(최소 버전 미달) 상태에 `agent_too_old`와 요구 버전을 남겨
/// GUI·CLI가 업데이트를 안내하게 하고, 받기는 업데이트 전에는 같은 답이므로 heartbeat 주기로만 다시 묻는다
#[tokio::test]
async fn a_relay_that_needs_a_newer_agent_is_reported_and_asked_slowly() {
    use axum::routing::{get, post};
    async fn too_old() -> (axum::http::StatusCode, axum::Json<Value>) {
        let body = json!({ "error": "AGENT_TOO_OLD", "minVersion": "99.0.0", "message": "Update Saba-chan to 99.0.0 or later." });
        (axum::http::StatusCode::UPGRADE_REQUIRED, axum::Json(body))
    }
    let app = axum::Router::new()
        .route("/info", get(|| async { axum::Json(json!({ "minAgentVersion": "99.0.0" })) }))
        .route("/heartbeat", post(too_old))
        .route("/poll", get(too_old));
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let base = format!("http://{}", listener.local_addr().unwrap());
    tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });

    let tmp = tempfile::tempdir().unwrap();
    let mut sup = crate::supervisor::Supervisor::new_with_instances_dir(
        tmp.path().join("modules").to_str().unwrap(),
        tmp.path().join("instances").to_str().unwrap(),
    );
    sup.initialize().await.unwrap();
    let server = crate::ipc::IPCServer::new(std::sync::Arc::new(tokio::sync::RwLock::new(sup)), "127.0.0.1:0", crate::daemon_log::DaemonLogBuffer::new());
    let agent = RelayAgent::new();
    let cancel = CancellationToken::new();
    {
        let mut inner = agent.lock();
        inner.cancel = Some(cancel.clone());
        inner.generation = 1;
        inner.status.running = true;
    }
    let session = Session {
        agent: agent.clone(),
        server,
        client: relay_client_with_timeout(POLL_TIMEOUT).unwrap(),
        base,
        token: "sbn_host.secret".into(),
        cancel,
        generation: 1,
        seen: Arc::default(),
        metadata_sent: Mutex::new(None),
        linked_acks: Mutex::default(),
        chatbot_sync_due: Default::default(),
        seen_v4: Mutex::default(),
    };
    let reset = || agent.update(1, |s| *s = AgentStatus { running: true, ..Default::default() });
    let reported = |what: &str| {
        let status = agent.status();
        assert_eq!(status.last_error_code, Some("agent_too_old"), "{what}: {status:?}");
        assert_eq!(status.min_version.as_deref(), Some("99.0.0"), "{what}");
        let json = serde_json::to_value(&status).unwrap();
        assert_eq!((json["lastErrorCode"].clone(), json["minVersion"].clone()), (json!("agent_too_old"), json!("99.0.0")));
    };

    assert!(session.check_version().await.is_err());
    reported("/info");
    reset();
    session.heartbeat().await;
    reported("heartbeat");
    reset();
    let polled = session.poll_once().await.unwrap_err();
    reported("poll");
    assert_eq!(poll_retry(&polled, 1), HEARTBEAT_INTERVAL);
    assert_eq!(poll_retry("poll 502 Bad Gateway: ", 1), backoff(1), "other failures keep the short backoff");
}

/// 릴레이가 상태 보고(heartbeat)를 검증에서 거절하면 어느 값 때문인지(칸·상한·보낸 크기)를 상태에 남겨
/// 앱이 연결 상태에 이유를 보이게 하고, 다시 받아 주면 지운다
#[tokio::test]
async fn a_heartbeat_the_relay_refuses_reports_which_value_was_refused() {
    use axum::routing::post;
    use std::sync::atomic::{AtomicBool, Ordering};
    static REFUSE: AtomicBool = AtomicBool::new(true);
    async fn heartbeat() -> (axum::http::StatusCode, axum::Json<Value>) {
        if REFUSE.load(Ordering::SeqCst) {
            let reason = json!({ "code": "TOO_BIG", "field": "metadata.servers", "max": 200, "actual": 201 });
            return (axum::http::StatusCode::BAD_REQUEST, axum::Json(json!({ "error": "VALIDATION_ERROR", "details": [], "reason": reason })));
        }
        (axum::http::StatusCode::OK, axum::Json(json!({ "ok": true })))
    }
    let app = axum::Router::new().route("/heartbeat", post(heartbeat));
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let base = format!("http://{}", listener.local_addr().unwrap());
    tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });

    let tmp = tempfile::tempdir().unwrap();
    let mut sup = crate::supervisor::Supervisor::new_with_instances_dir(
        tmp.path().join("modules").to_str().unwrap(),
        tmp.path().join("instances").to_str().unwrap(),
    );
    sup.initialize().await.unwrap();
    let server = crate::ipc::IPCServer::new(std::sync::Arc::new(tokio::sync::RwLock::new(sup)), "127.0.0.1:0", crate::daemon_log::DaemonLogBuffer::new());
    let agent = RelayAgent::new();
    let cancel = CancellationToken::new();
    {
        let mut inner = agent.lock();
        inner.cancel = Some(cancel.clone());
        inner.generation = 1;
        inner.status.running = true;
    }
    let session = Session {
        agent: agent.clone(),
        server,
        client: relay_client_with_timeout(POLL_TIMEOUT).unwrap(),
        base,
        token: "sbn_host.secret".into(),
        cancel,
        generation: 1,
        seen: Arc::default(),
        metadata_sent: Mutex::new(None),
        linked_acks: Mutex::default(),
        chatbot_sync_due: Default::default(),
        seen_v4: Mutex::default(),
    };

    session.heartbeat().await;
    let json = serde_json::to_value(agent.status()).unwrap();
    assert_eq!(json["heartbeatRejected"], json!({ "code": "TOO_BIG", "field": "metadata.servers", "max": 200, "actual": 201 }), "{json}");
    assert_eq!(json["lastError"], "heartbeat 400 Bad Request: VALIDATION_ERROR (metadata.servers)");

    REFUSE.store(false, Ordering::SeqCst);
    session.heartbeat().await;
    let json = serde_json::to_value(agent.status()).unwrap();
    assert!(json.get("heartbeatRejected").is_none() && json["lastError"].is_null(), "{json}");
}

/// 화면은 릴레이에 따로 묻지 않고 이 값으로 붙었는지 본다 (예전에는 5초마다 릴레이에 다녀왔다)
#[test]
fn connected_means_a_recent_heartbeat_with_no_failure_since() {
    let now = now_ms();
    let heard = |ago: u64| AgentStatus { running: true, last_heartbeat_at: Some(now - ago), ..Default::default() };
    assert!(connected(&heard(1_000), now));
    assert!(!connected(&AgentStatus { running: true, ..Default::default() }, now), "never heard back");
    assert!(!connected(&AgentStatus { last_error: Some("poll 502".into()), ..heard(1_000) }, now), "failed since");
    assert!(!connected(&heard(CONNECTED_WITHIN_MS + 1), now), "the heartbeat stopped answering");
    assert!(!connected(&AgentStatus { running: false, ..heard(1_000) }, now), "stopped");
}

/// 붙고 끊길 때만 `relay.status`를 낸다. 매분 바뀌는 하트비트 시각만으로는 내지 않는다
#[tokio::test]
async fn a_change_in_the_connection_is_announced_once() {
    let mut rx = crate::events::bus().subscribe(None).rx;
    let agent = RelayAgent::new();
    {
        let mut inner = agent.lock();
        inner.cancel = Some(CancellationToken::new());
        inner.generation = 1;
        inner.status.running = true;
    }
    let mut announced = || {
        let mut n = 0;
        while let Ok(e) = rx.try_recv() {
            n += usize::from(e.kind == "relay.status");
        }
        n
    };
    agent.update(1, |s| s.last_heartbeat_at = Some(1));
    assert_eq!(announced(), 1, "first heartbeat");
    agent.update(1, |s| s.last_heartbeat_at = Some(2));
    assert_eq!(announced(), 0, "only the time moved");
    agent.update(1, |s| s.last_error = Some("poll 502".into()));
    agent.update(1, |s| s.last_error = Some("poll 502".into()));
    assert_eq!(announced(), 1, "the same failure again is not news");
    agent.stop();
    assert_eq!(announced(), 1);
    let json = serde_json::to_value(agent.status()).unwrap();
    assert_eq!(json["connected"], json!(false));
}

/// 외부 감사: ack한 명령의 결과 전송이 한 번 실패하면(503 등) 릴레이가 다시 보내 주지 않아 결과가 사라졌다.
/// 일시적인 실패는 몇 번 다시 보내고, 릴레이가 그 명령을 모르면(4xx) 그만둔다
#[tokio::test]
async fn a_result_the_relay_did_not_take_is_sent_again() {
    use axum::routing::post;
    use std::sync::atomic::{AtomicUsize, Ordering};
    static CALLS: AtomicUsize = AtomicUsize::new(0);
    async fn result(axum::extract::Path(id): axum::extract::Path<String>) -> axum::http::StatusCode {
        let n = CALLS.fetch_add(1, Ordering::SeqCst);
        match id.as_str() {
            "gone" => axum::http::StatusCode::NOT_FOUND,
            _ if n < 2 => axum::http::StatusCode::SERVICE_UNAVAILABLE,
            _ => axum::http::StatusCode::OK,
        }
    }
    let app = axum::Router::new().route("/result/{id}", post(result));
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let base = format!("http://{}", listener.local_addr().unwrap());
    tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });

    let tmp = tempfile::tempdir().unwrap();
    let mut sup = crate::supervisor::Supervisor::new_with_instances_dir(
        tmp.path().join("modules").to_str().unwrap(),
        tmp.path().join("instances").to_str().unwrap(),
    );
    sup.initialize().await.unwrap();
    let server = crate::ipc::IPCServer::new(std::sync::Arc::new(tokio::sync::RwLock::new(sup)), "127.0.0.1:0", crate::daemon_log::DaemonLogBuffer::new());
    let session = Session {
        agent: RelayAgent::new(),
        server,
        client: relay_client_with_timeout(POLL_TIMEOUT).unwrap(),
        base,
        token: "sbn_host.secret".into(),
        cancel: CancellationToken::new(),
        generation: 1,
        seen: Arc::default(),
        metadata_sent: Mutex::new(None),
        linked_acks: Mutex::default(),
        chatbot_sync_due: Default::default(),
        seen_v4: Mutex::default(),
    };
    session.post_result("c1", &json!({ "success": true })).await;
    assert_eq!(CALLS.swap(0, Ordering::SeqCst), 3, "two 503s, then taken");
    session.post_result("gone", &json!({ "success": true })).await;
    assert_eq!(CALLS.load(Ordering::SeqCst), 1, "a command the relay no longer has is not sent again");
    assert!(session.agent.unsent.lock().unwrap().is_empty(), "a refused result is not kept");
}

/// 사용자 11-02: 게임 채팅이 디스코드에 늦게 올라간다. 이벤트는 모으지 않고 곧 보낸다.
/// 이어서 오면 relay 한도(노드당 초당 5건) 안의 간격으로 묶어 보낸다
#[tokio::test]
async fn a_game_chat_line_is_sent_soon_and_following_lines_keep_under_the_relay_limit() {
    use axum::routing::post;
    static GOT: std::sync::OnceLock<tokio::sync::mpsc::UnboundedSender<Value>> = std::sync::OnceLock::new();
    let (tx, mut rx) = tokio::sync::mpsc::unbounded_channel();
    GOT.set(tx).unwrap();
    async fn events(axum::Json(body): axum::Json<Value>) -> axum::http::StatusCode {
        let _ = GOT.get().unwrap().send(body);
        axum::http::StatusCode::OK
    }
    let app = axum::Router::new().route("/events", post(events));
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let base = format!("http://{}", listener.local_addr().unwrap());
    tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });
    let tmp = tempfile::tempdir().unwrap();
    let session = test_session(&RelayAgent::new(), &base, "sbn_host.secret", tmp.path()).await;

    let chat = |instance: &str| crate::events::emit("instance.chat", json!({ "instanceId": instance, "instance": "s", "text": "\\[s\\] **Steve**: hi" }));
    // 그 서버의 줄이 relay에 닿을 때까지 걸린 시간 (다른 시험의 이벤트는 건너뛴다)
    let arrived = |rx: &mut tokio::sync::mpsc::UnboundedReceiver<Value>, instance: &'static str| {
        let started = tokio::time::Instant::now();
        let mut rx = std::mem::replace(rx, tokio::sync::mpsc::unbounded_channel().1);
        async move {
            loop {
                match tokio::time::timeout(Duration::from_secs(5), rx.recv()).await {
                    Ok(Some(body)) if body["events"].as_array().is_some_and(|a| a.iter().any(|e| e["data"]["instanceId"] == instance)) => return (started.elapsed(), rx),
                    Ok(Some(_)) => {}
                    _ => return (Duration::MAX, rx),
                }
            }
        }
    };
    tokio::join!(session.events_loop(), async {
        tokio::time::sleep(Duration::from_millis(100)).await;
        chat("chat-latency-first");
        let (first, mut rx) = arrived(&mut rx, "chat-latency-first").await;
        chat("chat-latency-second");
        let (second, _) = arrived(&mut rx, "chat-latency-second").await;
        session.cancel.cancel();
        assert!(first < Duration::from_millis(300), "the first line reached the relay after {first:?}");
        assert!(second >= Duration::from_millis(150), "the next send waits for the gap ({second:?})");
        assert!(second < Duration::from_millis(600), "but only for the gap ({second:?})");
    });
}

/// 업데이트로 내려간다고 알린 뒤에는 heartbeat를 보내지 않는다 (릴레이의 "업데이트 중" 창을 옛 데몬이 닫지 않게)
#[tokio::test]
async fn no_heartbeat_is_sent_after_announcing_the_update() {
    use std::sync::atomic::{AtomicUsize, Ordering};
    static BEATS: AtomicUsize = AtomicUsize::new(0);
    async fn beat() -> axum::Json<Value> {
        BEATS.fetch_add(1, Ordering::SeqCst);
        axum::Json(json!({ "status": "ok" }))
    }
    let app = axum::Router::new().route("/heartbeat", axum::routing::post(beat));
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let base = format!("http://{}", listener.local_addr().unwrap());
    tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });
    let tmp = tempfile::tempdir().unwrap();
    let agent = RelayAgent::new();
    let session = test_session(&agent, &base, "sbn_host.secret", tmp.path()).await;
    session.heartbeat().await;
    assert_eq!(BEATS.load(Ordering::SeqCst), 1);
    let _ = agent.notify_updating(90).await;
    session.heartbeat().await;
    assert_eq!(BEATS.load(Ordering::SeqCst), 1);
}

/// 시험용 세션: 주어진 에이전트·주소·토큰으로
async fn test_session(agent: &RelayAgent, base: &str, token: &str, tmp: &std::path::Path) -> Session {
    let mut sup = crate::supervisor::Supervisor::new_with_instances_dir(
        tmp.join("modules").to_str().unwrap(),
        tmp.join("instances").to_str().unwrap(),
    );
    sup.initialize().await.unwrap();
    let server = crate::ipc::IPCServer::new(std::sync::Arc::new(tokio::sync::RwLock::new(sup)), "127.0.0.1:0", crate::daemon_log::DaemonLogBuffer::new());
    Session {
        agent: agent.clone(),
        server,
        client: relay_client_with_timeout(POLL_TIMEOUT).unwrap(),
        base: base.into(),
        token: token.into(),
        cancel: CancellationToken::new(),
        generation: 1,
        seen: Arc::default(),
        metadata_sent: Mutex::new(None),
        linked_acks: Mutex::default(),
        chatbot_sync_due: Default::default(),
        seen_v4: Mutex::default(),
    }
}

/// `unsent_loop`를 돌리다가 `done`이 참이 되거나 `wait`이 지나면 세션을 끝낸다
async fn run_unsent(session: &Session, wait: Duration, done: impl Fn() -> bool) {
    tokio::join!(session.unsent_loop(), async {
        let until = tokio::time::Instant::now() + wait;
        while !done() && tokio::time::Instant::now() < until {
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
        session.cancel.cancel();
    });
}

/// 외부 감사 10-23: 결과 전송이 여섯 번 다 실패하면 결과를 버렸다. ack한 명령이라 릴레이가 다시 보내 주지 않는다.
/// 받지 않은 결과는 남겨 두고, 릴레이가 살아나면 명령을 다시 실행하지 않고 같은 결과를 보낸다
#[tokio::test]
async fn a_result_kept_after_every_retry_failed_is_sent_once_the_relay_recovers() {
    use axum::routing::post;
    use std::sync::atomic::{AtomicUsize, Ordering};
    static CALLS: AtomicUsize = AtomicUsize::new(0);
    static BODY: std::sync::Mutex<String> = std::sync::Mutex::new(String::new());
    async fn result(body: String) -> axum::http::StatusCode {
        if CALLS.fetch_add(1, Ordering::SeqCst) < 7 {
            return axum::http::StatusCode::SERVICE_UNAVAILABLE;
        }
        *BODY.lock().unwrap() = body;
        axum::http::StatusCode::OK
    }
    let app = axum::Router::new().route("/result/{id}", post(result));
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let base = format!("http://{}", listener.local_addr().unwrap());
    tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });

    let tmp = tempfile::tempdir().unwrap();
    let agent = RelayAgent::new();
    let session = test_session(&agent, &base, "sbn_host.secret", tmp.path()).await;
    session.post_result("c1", &json!({ "success": true, "n": 1 })).await;
    assert_eq!(CALLS.load(Ordering::SeqCst), (RESULT_RETRIES + 1) as usize);
    assert_eq!(agent.unsent.lock().unwrap().len(), 1, "kept after the last retry");
    run_unsent(&session, Duration::from_secs(5), || agent.unsent.lock().unwrap().is_empty()).await;
    assert!(agent.unsent.lock().unwrap().is_empty(), "sent once the relay took it");
    assert_eq!(CALLS.load(Ordering::SeqCst), 8);
    assert_eq!(serde_json::from_str::<Value>(&BODY.lock().unwrap()).unwrap(), json!({ "success": true, "n": 1 }));
}

/// Codex 리뷰 #553: 원격 업데이트 뒤 데몬이 0.5초 만에 꺼져, 첫 전송이 실패한 결과(재시작 안내)를 다시 보내기 전에 잃었다.
/// 종료 전에 보내는 중인 결과가 끝나기를 기다린다
#[tokio::test]
async fn shutdown_waits_for_a_result_still_being_sent() {
    use axum::routing::post;
    use std::sync::atomic::{AtomicUsize, Ordering};
    static CALLS: AtomicUsize = AtomicUsize::new(0);
    async fn result() -> axum::http::StatusCode {
        if CALLS.fetch_add(1, Ordering::SeqCst) == 0 {
            return axum::http::StatusCode::SERVICE_UNAVAILABLE;
        }
        axum::http::StatusCode::OK
    }
    let app = axum::Router::new().route("/result/{id}", post(result));
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let base = format!("http://{}", listener.local_addr().unwrap());
    tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });

    let tmp = tempfile::tempdir().unwrap();
    let agent = RelayAgent::new();
    let session = Arc::new(test_session(&agent, &base, "sbn_host.secret", tmp.path()).await);
    assert!(agent.settle(Duration::from_millis(10)).await, "nothing to wait for");
    let sending = session.clone();
    let task = tokio::spawn(async move { sending.post_result("c1", &json!({ "success": true })).await });
    tokio::time::sleep(Duration::from_millis(5)).await;
    assert!(agent.settle(Duration::from_secs(5)).await);
    assert_eq!(CALLS.load(Ordering::SeqCst), 2, "the retry went out before settle returned");
    task.await.unwrap();
}

/// Codex 리뷰 #549: 대기열 맨 앞 결과만 계속 5xx를 받으면 매 회차 거기서 멈춰 뒤 결과를 보내지 않았다
#[tokio::test]
async fn one_result_the_relay_keeps_refusing_does_not_block_the_others() {
    use axum::routing::post;
    use std::sync::atomic::{AtomicUsize, Ordering};
    static OTHER: AtomicUsize = AtomicUsize::new(0);
    async fn result(axum::extract::Path(id): axum::extract::Path<String>) -> axum::http::StatusCode {
        if id == "c1" {
            return axum::http::StatusCode::INTERNAL_SERVER_ERROR;
        }
        OTHER.fetch_add(1, Ordering::SeqCst);
        axum::http::StatusCode::OK
    }
    let app = axum::Router::new().route("/result/{id}", post(result));
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let base = format!("http://{}", listener.local_addr().unwrap());
    tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });

    let tmp = tempfile::tempdir().unwrap();
    let agent = RelayAgent::new();
    let session = test_session(&agent, &base, "sbn_host.secret", tmp.path()).await;
    session.keep_unsent("c1", "{}".into());
    session.keep_unsent("c2", "{}".into());
    run_unsent(&session, Duration::from_secs(5), || OTHER.load(Ordering::SeqCst) > 0).await;
    assert_eq!(OTHER.load(Ordering::SeqCst), 1, "the result behind the refused one is sent");
    assert!(session.has_unsent("c1") && !session.has_unsent("c2"));
}

/// 받지 않은 결과는 그 결과를 만든 주소·토큰의 다음 세션만 보낸다. 다른 토큰(다른 노드)의 세션은 버린다
#[tokio::test]
async fn a_kept_result_goes_only_to_the_node_that_ran_the_command() {
    use axum::routing::post;
    use std::sync::atomic::{AtomicUsize, Ordering};
    static CALLS: AtomicUsize = AtomicUsize::new(0);
    let app = axum::Router::new().route(
        "/result/{id}",
        post(|| async {
            CALLS.fetch_add(1, Ordering::SeqCst);
            axum::http::StatusCode::OK
        }),
    );
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let base = format!("http://{}", listener.local_addr().unwrap());
    tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });

    let tmp = tempfile::tempdir().unwrap();
    let agent = RelayAgent::new();
    let first = test_session(&agent, &base, "sbn_host.secret", tmp.path()).await;
    first.keep_unsent("c1", "{}".into());
    first.cancel.cancel();
    // 같은 주소·토큰으로 다시 연결: 보낸다
    let again = test_session(&agent, &base, "sbn_host.secret", tmp.path()).await;
    run_unsent(&again, Duration::from_secs(5), || CALLS.load(Ordering::SeqCst) > 0).await;
    assert_eq!(CALLS.swap(0, Ordering::SeqCst), 1);
    // 다른 토큰·다른 주소: 보내지 않고 버린다
    first.keep_unsent("c2", "{}".into());
    let other_node = test_session(&agent, &base, "sbn_other.secret", tmp.path()).await;
    run_unsent(&other_node, result_retry_delay(RESULT_RETRIES) * 3, || false).await;
    let other_relay = test_session(&agent, "http://127.0.0.1:9", "sbn_host.secret", tmp.path()).await;
    run_unsent(&other_relay, result_retry_delay(RESULT_RETRIES) * 3, || false).await;
    assert_eq!(CALLS.load(Ordering::SeqCst), 0);
    assert!(agent.unsent.lock().unwrap().is_empty(), "another node's result is dropped");
}

/// 외부 감사: 끝난 세션의 `unsent_loop`가 기다림에서 깨어나 대기열을 정리하면서, 그 사이 시작한 새 세션(다른 토큰)의 결과를 지웠다
#[tokio::test]
async fn an_ended_session_does_not_drop_the_next_sessions_kept_results() {
    let tmp = tempfile::tempdir().unwrap();
    let agent = RelayAgent::new();
    let old = test_session(&agent, "http://127.0.0.1:9", "sbn_host.secret", tmp.path()).await;
    let new = test_session(&agent, "http://127.0.0.1:9", "sbn_other.secret", tmp.path()).await;
    // 옛 세션은 첫 차례에 기다림에 들어간다. 새 세션이 결과를 남긴 뒤 옛 세션이 끝난다
    tokio::join!(old.unsent_loop(), async {
        new.keep_unsent("c1", "{}".into());
        old.cancel.cancel();
    });
    assert!(new.has_unsent("c1"), "the next session's result is kept");
}

/// 결과를 아직 보내지 못한 명령을 릴레이가 다시 보내도(기억한 id가 지난 뒤에도) 다시 실행하지 않고 ack만 한다
#[tokio::test]
async fn a_command_whose_result_is_kept_is_not_run_again() {
    use axum::routing::{get, post};
    use std::sync::atomic::{AtomicUsize, Ordering};
    static RESULTS: AtomicUsize = AtomicUsize::new(0);
    static ACKS: AtomicUsize = AtomicUsize::new(0);
    let app = axum::Router::new()
        .route(
            "/poll",
            get(|| async {
                tokio::time::sleep(Duration::from_millis(20)).await;
                axum::Json(json!({ "commands": [{ "id": "c1", "payload": {} }] }))
            }),
        )
        .route(
            "/ack",
            post(|| async {
                ACKS.fetch_add(1, Ordering::SeqCst);
                axum::http::StatusCode::OK
            }),
        )
        .route(
            "/result/{id}",
            post(|| async {
                RESULTS.fetch_add(1, Ordering::SeqCst);
                axum::http::StatusCode::OK
            }),
        );
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let base = format!("http://{}", listener.local_addr().unwrap());
    tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });

    let tmp = tempfile::tempdir().unwrap();
    let agent = RelayAgent::new();
    let session = test_session(&agent, &base, "sbn_host.secret", tmp.path()).await;
    session.keep_unsent("c1", "{}".into());
    tokio::join!(session.poll_loop(), async {
        let until = tokio::time::Instant::now() + Duration::from_secs(5);
        while ACKS.load(Ordering::SeqCst) == 0 && tokio::time::Instant::now() < until {
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
        tokio::time::sleep(Duration::from_millis(100)).await;
        session.cancel.cancel();
    });
    assert!(ACKS.load(Ordering::SeqCst) > 0, "the redelivered command is acked again");
    assert_eq!(RESULTS.load(Ordering::SeqCst), 0, "not run again (running would post a new result)");
    assert_eq!(agent.unsent.lock().unwrap().len(), 1);
}

/// 디스코드에서 바꾼 권한의 응답과 heartbeat는 같은 PC 시계의 `permissionsAt`을 싣는다.
/// 릴레이는 바꾼 뒤보다 먼저 읽은 heartbeat 권한을 버린다 (늦게 와서 옛 레벨로 되돌리지 않게)
#[tokio::test]
async fn a_permission_change_reports_when_it_was_saved() {
    let tmp = tempfile::tempdir().unwrap();
    let mut sup = crate::supervisor::Supervisor::new_with_instances_dir(
        tmp.path().join("modules").to_str().unwrap(),
        tmp.path().join("instances").to_str().unwrap(),
    );
    sup.initialize().await.unwrap();
    let server = crate::ipc::IPCServer::new(std::sync::Arc::new(tokio::sync::RwLock::new(sup)), "127.0.0.1:0", crate::daemon_log::DaemonLogBuffer::new());
    let who = crate::chat::permissions::Requester {
        guild_id: Some("r8-permissions-at".into()),
        user_id: Some("owner".into()),
        owner_id: Some("owner".into()),
        via_chatbot: false,
        confirmed_by: None,
        guild_owner_id: None,
    };
    let before = now_ms();
    let res = permission_action(&server, &who, &json!({ "op": "default_level", "level": 1 })).await;
    assert_eq!(res["success"], true, "{res}");
    let at = res["data"]["permissionsAt"].as_u64().unwrap_or_else(|| panic!("no permissionsAt: {res}"));
    assert!(at >= before && at <= now_ms());
    // 응답은 heartbeat `permissions[guild]`와 같은 모양이다
    for key in ["defaultLevel", "members", "chatbotCap", "levels", "commandLevels", "ladder", "required"] {
        assert!(res["data"].get(key).is_some(), "no {key}: {res}");
    }
}

/// `/설정` 명령별 레벨: `command_level {key, level|null}`로 그 명령의 덮어쓰기를 두고 지운다 (주인만)
#[tokio::test]
async fn the_owner_sets_and_clears_one_command_level() {
    let tmp = tempfile::tempdir().unwrap();
    let mut sup = crate::supervisor::Supervisor::new_with_instances_dir(
        tmp.path().join("modules").to_str().unwrap(),
        tmp.path().join("instances").to_str().unwrap(),
    );
    sup.initialize().await.unwrap();
    let server = crate::ipc::IPCServer::new(std::sync::Arc::new(tokio::sync::RwLock::new(sup)), "127.0.0.1:0", crate::daemon_log::DaemonLogBuffer::new());
    let who = |user: &str| crate::chat::permissions::Requester {
        guild_id: Some("r8-command-level".into()),
        user_id: Some(user.into()),
        owner_id: Some("owner".into()),
        via_chatbot: false,
        confirmed_by: None,
        guild_owner_id: None,
    };
    let set = permission_action(&server, &who("owner"), &json!({ "op": "command_level", "key": "i1/kick", "level": 2 })).await;
    assert_eq!(set["data"]["commandLevels"]["i1/kick"], 2, "{set}");
    let cleared = permission_action(&server, &who("owner"), &json!({ "op": "command_level", "key": "i1/kick", "level": null })).await;
    assert!(cleared["data"]["commandLevels"].get("i1/kick").is_none(), "{cleared}");
    let bad = permission_action(&server, &who("owner"), &json!({ "op": "command_level", "key": "kick", "level": 1 })).await;
    assert_eq!(bad["data"]["code"], "INVALID_COMMAND", "{bad}");
    let other = permission_action(&server, &who("u2"), &json!({ "op": "command_level", "key": "i1/kick", "level": 1 })).await;
    assert_eq!(other["data"]["code"], "NOT_OWNER", "{other}");
}

/// 노드 토큰이 평문으로 나가지 않게: 이미 저장된 http 주소(루프백 제외)로는 세션을 열지 않는다
#[test]
fn a_session_only_opens_to_an_https_or_loopback_relay() {
    assert_eq!(session_relay_url("").unwrap(), "https://saba-chan.online");
    assert_eq!(session_relay_url(" https://my.relay/ ").unwrap(), "https://my.relay");
    assert_eq!(session_relay_url("http://127.0.0.1:9000").unwrap(), "http://127.0.0.1:9000");
    let err = session_relay_url("http://old.example").unwrap_err();
    assert_eq!(err.code, "invalid_relay_url");
}

#[test]
fn heartbeat_commands_leave_templates_on_the_pc() {
    let field: crate::module_loader::CommandField = serde_json::from_value(json!({
        "name": "kick", "label": "Kick", "method": "console", "console_template": "kick {player}",
        "endpoint_template": "/v1/kick", "risk": "caution",
        "inputs": [{ "name": "player", "type": "text", "required": true }]
    }))
    .unwrap();
    let entry = relay_command_entry(&field, &["추방".to_string()], None);
    assert_eq!(entry["name"], "kick");
    assert_eq!(entry["aliases"], json!(["추방"]));
    assert!(relay_command_entry(&field, &[], None).get("aliases").is_none());
    assert_eq!(entry["inputs"][0]["name"], "player");
    assert!(entry.get("console_template").is_none() && entry.get("endpoint_template").is_none() && entry.get("method").is_none());
}

/// 번역한 명령 이름이 켜져 있으면 모듈 locales의 이름·설명·칸 이름을 언어별로 싣는다 (슬래시 명령의 언어별 이름)
#[test]
fn heartbeat_commands_carry_translated_names_when_asked() {
    let field: crate::module_loader::CommandField = serde_json::from_value(json!({
        "name": "save-all", "label": "Save World", "description": "Save the game world",
        "inputs": [{ "name": "flush", "type": "text" }]
    }))
    .unwrap();
    let texts = |lang: &str| -> std::sync::Arc<Value> {
        std::sync::Arc::new(match lang {
            "ko" => json!({ "commands": { "save-all": { "label": "월드 저장", "description": "게임 월드를 저장합니다", "inputs": { "flush": { "label": "바로 쓰기" } } } } }),
            _ => Value::Null,
        })
    };
    let entry = relay_command_entry(&field, &[], Some(&texts));
    assert_eq!(entry["titles"]["ko"], "월드 저장");
    assert_eq!(entry["descriptions"]["ko"], "게임 월드를 저장합니다");
    assert_eq!(entry["inputs"][0]["titles"]["ko"], "바로 쓰기");
    assert!(entry["titles"].get("en").is_none(), "no text for a language → nothing (the relay shows the plain name)");
    assert!(relay_command_entry(&field, &[], None).get("titles").is_none(), "off → plain names only");
}

/// 서버가 켜지고 꺼진 것만 하트비트를 앞당긴다 (콘솔 줄·백업 같은 잦은 이벤트는 아니다)
#[test]
fn instance_state_events_kick_the_heartbeat() {
    for kind in ["instance.started", "instance.stopped", "instance.ready", "instance.exit"] {
        assert!(kicks_heartbeat(kind), "{kind}");
    }
    for kind in ["instance.chat", "backup.done", "plan.done", "update.available"] {
        assert!(!kicks_heartbeat(kind), "{kind}");
    }
}

/// 게임 이름: module.toml `display_name`과 언어마다 모듈 locales의 `display_name` (상황판 채널에 켜진 게임을 보인다)
#[test]
fn heartbeat_modules_carry_game_names() {
    let texts = |lang: &str| -> std::sync::Arc<Value> {
        std::sync::Arc::new(match lang {
            "ko" => json!({ "display_name": "프로젝트 좀보이드" }),
            "ja" => json!({ "display_name": " " }),
            _ => Value::Null,
        })
    };
    let mut entry = json!({});
    relay_module_names(&mut entry, Some("Project Zomboid"), &texts);
    assert_eq!(entry["display_name"], "Project Zomboid");
    assert_eq!(entry["display_names"], json!({ "ko": "프로젝트 좀보이드" }), "blank or missing → left out");

    let mut bare = json!({});
    relay_module_names(&mut bare, None, &|_: &str| std::sync::Arc::new(Value::Null));
    assert_eq!(bare, json!({}), "no names → no keys (the relay falls back to the module name)");

    // 모듈 locales의 지금 모양은 `module.display_name`이다
    let nested = |lang: &str| std::sync::Arc::new(if lang == "ko" { json!({ "module": { "display_name": "팰월드" } }) } else { Value::Null });
    let mut entry = json!({});
    relay_module_names(&mut entry, None, &nested);
    assert_eq!(entry["display_names"], json!({ "ko": "팰월드" }));
}

/// 언어마다 더하는 별명은 언어별로 따로 싣는다: 릴레이가 길드 언어의 것만 더한다
#[test]
fn heartbeat_modules_carry_aliases_by_language() {
    let tmp = tempfile::tempdir().unwrap();
    std::fs::create_dir_all(tmp.path().join("locales")).unwrap();
    std::fs::write(
        tmp.path().join("locales/ko.json"),
        r#"{"module": {"display_name": "팰월드"}, "aliases": {"module": ["팰"], "commands": {"kick": ["추방"], "gone": ["없음"]}}}"#,
    )
    .unwrap();
    std::fs::write(tmp.path().join("locales/ja.json"), r#"{"aliases": {"module": ["パル"]}}"#).unwrap();
    std::fs::write(tmp.path().join("locales/en.json"), r#"{"commands": {"kick": {"label": "Kick"}}}"#).unwrap();
    let mut entry = json!({ "name": "palworld", "module_aliases": ["pw"], "commands": { "fields": [{ "name": "kick" }, { "name": "save" }] } });
    relay_language_aliases(&mut entry, tmp.path());
    assert_eq!(entry["module_aliases"], json!(["pw"]), "the language-free words stay as they are");
    assert_eq!(entry["module_aliases_by_language"], json!({ "ko": ["팰월드", "팰"], "ja": ["パル"] }));
    assert_eq!(entry["commands"]["fields"][0]["aliases_by_language"], json!({ "ko": ["추방"] }));
    assert!(entry["commands"]["fields"][1].get("aliases_by_language").is_none());
}

/// 사바쨩 웹: 릴레이가 세션을 열었다고 알리면 이 세션의 릴레이 주소·토큰으로 웹소켓을 연다
#[tokio::test]
async fn a_web_session_opens_the_relay_websocket() {
    let _one = crate::remote::link::TEST_LOCK.lock().await;
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let base = format!("http://{}", listener.local_addr().unwrap());
    let tmp = tempfile::tempdir().unwrap();
    let session = test_session(&RelayAgent::new(), &base, "sbn_host.secret", tmp.path()).await;
    let result = session.handle_payload(&json!({ "id": "c1", "payload": { "action": "web_session", "sessionId": "s1" } })).await;
    assert_eq!(result["success"], true, "{result:?}");
    let (tcp, _) = tokio::time::timeout(Duration::from_secs(5), listener.accept()).await.expect("the daemon dials the relay").unwrap();
    let mut path = String::new();
    let _ = tokio_tungstenite::accept_hdr_async(tcp, |req: &tokio_tungstenite::tungstenite::handshake::server::Request, res| {
        path = req.uri().path().to_string();
        Ok(res)
    })
    .await;
    assert_eq!(path, "/node/ws");
    crate::remote::link::stop();
}

/// 원격 MCP: 길드 없이 오고, 요청자가 이 PC의 주인(`hostOwnerDiscordId`)일 때만 앱과 같은 액션을 쓴다
#[tokio::test]
async fn mcp_commands_run_only_for_the_host_owner() {
    let tmp = tempfile::tempdir().unwrap();
    let session = test_session(&RelayAgent::new(), "http://127.0.0.1:9", "sbn_host.secret", tmp.path()).await;
    let plan = json!({ "action": "plan", "via": "mcp", "id": "extension.enable", "input": { "extension": "x" } });
    let ok = session.handle_payload(&json!({ "id": "c1", "requestedBy": "u1", "hostOwnerDiscordId": "u1", "payload": plan })).await;
    assert_eq!(ok["success"], true, "{ok:?}");
    let denied = |r: &Value| r["success"] == false && r["data"]["code"] == "PERMISSION_DENIED";
    for cmd in [
        json!({ "id": "c2", "requestedBy": "u2", "hostOwnerDiscordId": "u1", "payload": plan }),
        json!({ "id": "c3", "requestedBy": "u1", "payload": plan }),
        json!({ "id": "c4", "requestedBy": "u1", "hostOwnerDiscordId": "u1", "guildId": "g1", "payload": plan }),
        json!({ "id": "c5", "hostOwnerDiscordId": "u1", "payload": plan }),
    ] {
        let r = session.handle_payload(&cmd).await;
        assert!(denied(&r), "{cmd} → {r:?}");
    }
    // 액션 목록은 MCP 주인에게만 (앱 표면 전체)
    let catalog = json!({ "action": "catalog", "via": "mcp" });
    let r = session.handle_payload(&json!({ "id": "c6", "requestedBy": "u1", "hostOwnerDiscordId": "u1", "payload": catalog })).await;
    let ids: Vec<&str> = r["data"]["actions"].as_array().unwrap().iter().filter_map(|a| a["id"].as_str()).collect();
    assert!(ids.contains(&"extension.enable") && ids.contains(&"server.stop"), "{r:?}");
    let r = session.handle_payload(&json!({ "id": "c7", "requestedBy": "u2", "hostOwnerDiscordId": "u1", "payload": catalog })).await;
    assert!(denied(&r), "{r:?}");
    let r = session.handle_payload(&json!({ "id": "c8", "requestedBy": "u1", "guildId": "g1", "payload": { "action": "catalog" } })).await;
    assert_eq!(r["success"], false, "{r:?}");
    // 문장 계획은 MCP로 받지 않는다
    let text = json!({ "action": "plan", "via": "mcp", "text": "srv start" });
    let r = session.handle_payload(&json!({ "id": "c9", "requestedBy": "u1", "hostOwnerDiscordId": "u1", "payload": text })).await;
    assert_eq!((r["success"].as_bool(), r["data"]["code"].as_str()), (Some(false), Some("invalid_input")), "{r:?}");
}

/// 확인 전 권한 검사(`check`)는 디스코드 요청으로 받고, 계획처럼 요청자 권한으로 판정한다
#[tokio::test]
async fn a_check_is_judged_like_a_plan() {
    let tmp = tempfile::tempdir().unwrap();
    let session = test_session(&RelayAgent::new(), "http://127.0.0.1:9", "sbn_host.secret", tmp.path()).await;
    let check = json!({ "action": "check", "via": "direct", "id": "chatbot.common.set", "input": { "model": "m1" } });
    let ok = session.handle_payload(&json!({ "id": "c1", "requestedBy": "u1", "hostOwnerDiscordId": "u1", "payload": check })).await;
    assert_eq!((ok["success"].as_bool(), ok["data"]["actionId"].as_str()), (Some(true), Some("chatbot.common.set")), "{ok:?}");
    assert!(ok["data"].get("planId").is_none(), "{ok:?}");
    let r = session.handle_payload(&json!({ "id": "c2", "requestedBy": "u2", "hostOwnerDiscordId": "u1", "payload": check })).await;
    assert_eq!(r["data"]["code"], "PERMISSION_DENIED", "{r:?}");
}

/// 봇 DM `/설정`: 길드 없는 디스코드 직접 요청은 요청자가 이 PC의 주인(`hostOwnerDiscordId`)일 때만 주인이다
#[tokio::test]
async fn guildless_discord_requests_are_the_owner_only_when_the_relay_says_so() {
    let tmp = tempfile::tempdir().unwrap();
    let session = test_session(&RelayAgent::new(), "http://127.0.0.1:9", "sbn_host.secret", tmp.path()).await;
    let plan = json!({ "action": "plan", "via": "direct", "id": "chatbot.common.set", "input": { "model": "m1" } });
    let ok = session.handle_payload(&json!({ "id": "c1", "requestedBy": "u1", "hostOwnerDiscordId": "u1", "payload": plan })).await;
    assert_eq!((ok["success"].as_bool(), ok["data"]["actionId"].as_str()), (Some(true), Some("chatbot.common.set")), "{ok:?}");
    for cmd in [
        json!({ "id": "c2", "requestedBy": "u2", "hostOwnerDiscordId": "u1", "payload": plan }),
        json!({ "id": "c3", "requestedBy": "u1", "payload": plan }),
    ] {
        let r = session.handle_payload(&cmd).await;
        assert_eq!(r["data"]["code"], "PERMISSION_DENIED", "{cmd} → {r:?}");
    }
}

/// 계획 뒤 바로 실행 (`autoRun`): 확인이 필요 없으면 실행 결과를, 확인이 필요하거나 `except`에 든 액션이면 계획을 준다
#[tokio::test]
async fn auto_run_plans_run_unless_they_need_a_confirmation_or_are_excepted() {
    let tmp = tempfile::tempdir().unwrap();
    let session = test_session(&RelayAgent::new(), "http://127.0.0.1:9", "sbn_host.secret", tmp.path()).await;
    let send = |payload: Value| {
        let cmd = json!({ "id": "c", "requestedBy": "u1", "hostOwnerDiscordId": "u1", "payload": payload });
        let session = &session;
        async move { session.handle_payload(&cmd).await }
    };
    let plan = |extra: Value| {
        let mut p = json!({ "action": "plan", "via": "mcp", "id": "extension.enable", "input": { "extension": "x" } });
        p.as_object_mut().unwrap().extend(extra.as_object().unwrap().clone());
        p
    };
    // 없는 익스텐션이라 실행은 실패한다: 실패도 실행의 답이다
    let ran = send(plan(json!({ "autoRun": {} }))).await;
    assert_eq!(ran["data"]["autoRun"], true, "{ran}");
    assert_eq!(ran["data"]["actionId"], "extension.enable", "{ran}");
    assert!(ran["data"]["planId"].is_string(), "{ran}");
    // 계획이 실행에 쓰였다: 같은 planId는 다시 실행되지 않는다
    let pid = ran["data"]["planId"].as_str().unwrap().to_string();
    let again = send(json!({ "action": "run_plan", "via": "mcp", "planId": pid })).await;
    assert_eq!(again["data"]["code"], "plan_expired", "{again}");
    for kept in [plan(json!({})), plan(json!({ "autoRun": { "except": ["extension.enable"] } }))] {
        let r = send(kept.clone()).await;
        assert_eq!((r["success"].as_bool(), r["data"].get("autoRun")), (Some(true), None), "{kept} → {r}");
        assert!(r["data"]["planId"].is_string(), "{r}");
    }
    // 확인이 필요한 계획·답만 있는 문장 계획은 이어서 실행하지 않는다
    let auto = json!({ "autoRun": {} });
    let view = |extra: Value| {
        let mut v = json!({ "planId": "p", "actionId": "server.status", "needsConfirm": false });
        v.as_object_mut().unwrap().extend(extra.as_object().unwrap().clone());
        v
    };
    assert_eq!(auto_run_plan(&auto, &view(json!({}))), Some("p"));
    assert_eq!(auto_run_plan(&auto, &view(json!({ "needsConfirm": true }))), None);
    assert_eq!(auto_run_plan(&auto, &json!({ "text": "도움말", "needsConfirm": false })), None);
    assert_eq!(auto_run_plan(&json!({ "autoRun": true }), &view(json!({}))), None);
}

/// 원격 데몬 터널: 원격 포트가 꺼져 있거나 터널 id가 이상하면 받지 않는다
#[tokio::test]
async fn a_tunnel_is_refused_while_the_remote_port_is_off() {
    let tmp = tempfile::tempdir().unwrap();
    let session = test_session(&RelayAgent::new(), "http://127.0.0.1:9", "sbn_host.secret", tmp.path()).await;
    let bad = session.handle_payload(&json!({ "id": "c1", "payload": { "action": "remote_tunnel", "tunnelId": "../x" } })).await;
    assert_eq!(bad["success"], false);
    let off = session.handle_payload(&json!({ "id": "c2", "payload": { "action": "remote_tunnel", "tunnelId": "t1" } })).await;
    assert_eq!((off["success"].as_bool(), off["data"]["error"].as_str()), (Some(false), Some("remote_off")));
}

#[test]
fn heartbeat_servers_carry_only_what_the_relay_reads() {
    let server = json!({
        "id": "i1", "name": "마크", "module": "minecraft", "status": "running",
        "executable_path": "C:/srv/java.exe", "working_dir": "C:/srv",
        "module_settings": { "rcon_secret": "x" }, "rest_password": "y"
    });
    assert_eq!(relay_server_entry(&server, None), json!({ "id": "i1", "name": "마크", "module": "minecraft", "status": "running" }));
}

/// 전용 멤버 서버만 모드와 명단의 디스코드 ID를 올린다 (게임 닉네임은 PC에만). 서버 공용은 명단을 쓰지 않는다
#[test]
fn heartbeat_servers_carry_only_the_discord_ids_of_members_servers() {
    let mut server = json!({
        "id": "i1", "name": "마크", "module": "minecraft", "status": "stopped",
        "controllers": [{ "name": "Steve", "discord_id": "332007306403250178" }, { "name": "Alex" }]
    });
    let shared = relay_server_entry(&server, None);
    assert!(shared.get("controllers").is_none() && shared.get("access").is_none());
    server["access"] = json!("members");
    let entry = relay_server_entry(&server, None);
    assert_eq!((entry["access"].clone(), entry["controllers"].clone()), (json!("members"), json!(["332007306403250178"])));
    assert!(!entry.to_string().contains("Steve"));
}

#[test]
fn heartbeat_servers_add_usage_and_an_extension_status_line() {
    let server = json!({
        "id": "i1", "name": "마크", "module": "minecraft", "status": "running",
        "extension_status": { "zz": { "status_line": "later" }, "docker": { "status_line": "  컨테이너 실행 중\n", "path": "C:/x" } },
        "players": { "online": 3, "max": 20 }, "start_time": 1_793_000_000u64
    });
    let usage = crate::system_monitor::InstanceMetrics {
        id: "i1".into(), name: "마크".into(), running: true, process_count: 1,
        cpu_percent: 12.345, memory_bytes: 3 * 1024 * 1024 * 1024, disk_read_bps: 0, disk_write_bps: 0,
        net_rx_bps: None, net_tx_bps: None,
    };
    let entry = relay_server_entry(&server, Some(&usage));
    assert_eq!(entry["usage"], json!({ "cpu": 12.3, "memMb": 3072 }));
    assert_eq!(entry["statusLine"], "컨테이너 실행 중");
    assert_eq!(entry["players"], json!({ "online": 3, "max": 20 }));
    assert_eq!(entry["startedAt"], 1_793_000_000u64, "상황판 가동 시간");
    let mut off = server.clone();
    off["status"] = json!("stopped");
    assert!(relay_server_entry(&off, None).get("players").is_none(), "a stopped server has no players");
    assert!(relay_server_entry(&off, None).get("startedAt").is_none());
    assert!(entry.get("extension_status").is_none());
    assert!(entry.get("chatLinks").is_none());
    let linked = json!({ "id": "hb-linked", "status": "running" });
    crate::supervisor::chat_link::toggle("hb-linked", "100000000000000001", "200000000000000001", None, |_| Ok::<(), ()>(())).unwrap();
    assert_eq!(relay_server_entry(&linked, None)["chatLinks"], json!([{ "guild": "100000000000000001", "channel": "200000000000000001", "direction": "both" }]));
    let stopped = crate::system_monitor::InstanceMetrics { running: false, ..usage };
    assert!(relay_server_entry(&server, Some(&stopped)).get("usage").is_none());
}

/// 메타데이터는 바뀔 때만 전체를 보낸다: 릴레이가 받아 둔 지문과 같으면 서버 목록·권한만
#[test]
fn heartbeat_metadata_is_sent_whole_only_when_it_changed() {
    let meta = json!({ "servers": [{ "id": "a" }], "permissions": { "g": {} }, "permissionsAt": 1, "actions": [1, 2], "language": "ko" });
    let (first, hash, unchanged) = heartbeat_metadata(meta.clone(), None);
    assert!(!unchanged);
    assert_eq!(first, meta);
    // 자주 바뀌는 서버 목록·권한 시각은 지문에 들지 않는다
    let mut busy = meta.clone();
    busy["servers"] = json!([{ "id": "a", "usage": { "cpu": 3.0, "memMb": 10 } }]);
    busy["permissionsAt"] = json!(2);
    // PC 부하와 예약 목록도 지문에 들지 않고 늘 간다
    busy["host"] = json!({ "cpu": 91, "mem": 40, "diskFree": 7 });
    busy["schedules"] = json!([{ "instanceId": "a", "action": "restart", "next": 1 }]);
    let (light, same, unchanged) = heartbeat_metadata(busy.clone(), Some(&hash));
    assert!(unchanged);
    assert_eq!(same, hash);
    assert_eq!(
        light,
        json!({ "servers": busy["servers"], "permissions": { "g": {} }, "permissionsAt": 2, "host": busy["host"], "schedules": busy["schedules"] })
    );
    // 권한이나 큰 값이 바뀌면 지문이 바뀌어 전체를 보낸다
    for (key, value) in [("permissions", json!({ "g": { "defaultLevel": 2 } })), ("actions", json!([1]))] {
        let mut changed = meta.clone();
        changed[key] = value;
        let (whole, other, unchanged) = heartbeat_metadata(changed.clone(), Some(&hash));
        assert!(!unchanged && other != hash, "{key}");
        assert_eq!(whole, changed);
    }
}

/// PC 부하는 정수 %로, 디스크는 가장 덜 남은 드라이브의 남은 비율만 (드라이브 이름은 싣지 않는다)
#[test]
fn host_load_is_whole_percents_and_the_fullest_disk() {
    use crate::system_monitor::*;
    let disk = |total: u64, avail: u64| DiskMetrics { mount: "D:\\".into(), total_bytes: total, available_bytes: avail, others_bytes: 0, instances: vec![], sizes_at_ms: None };
    let snap = MetricsSnapshot {
        sampled_at_ms: 0,
        window_ms: 0,
        cpu: CpuMetrics { cores: 8, total_percent: 81.6, others_percent: 0.0, clock_mhz: None, temp_c: None },
        memory: MemoryMetrics { total_bytes: 1000, used_bytes: 795, others_bytes: 0, clock_mhz: None },
        disk_io: DiskIoMetrics { read_bps: 0, write_bps: 0, source: "physical", per_instance: false },
        network: NetworkMetrics { rx_bps: 0, tx_bps: 0, per_instance: false },
        disks: vec![disk(100, 40), disk(200, 14), disk(0, 0)],
        instances: vec![],
        net_trace: crate::system_monitor::net_trace::NetTraceMetrics::status("off"),
        volumes: vec![],
        adapters: vec![],
        gpus: None,
    };
    assert_eq!(host_load(&snap), json!({ "cpu": 82, "mem": 80, "diskFree": 7 }));
    let none = MetricsSnapshot { disks: vec![], ..snap };
    assert!(host_load(&none).get("diskFree").is_none());
}

/// 하트비트가 IPv6로 나가 릴레이가 IPv6를 봤으면, 친구에게 안내할 주소는 포트 확인처럼 IPv4로 다시 물어 얻은 공인 IPv4다.
/// 같은 IPv6에는 다시 묻지 않고, 그 답을 모르는 옛 릴레이(빈 포트 목록을 400으로 거절)면 IPv6를 그대로 쓴다.
/// 일시 실패(한도·시간 초과)면 그동안 IPv6를 쓰고 간격을 두고 다시 묻는다
#[tokio::test]
async fn a_relay_that_saw_ipv6_is_asked_once_for_the_ipv4_friends_reach() {
    use axum::{extract::State as AxState, routing::post};
    use std::sync::atomic::{AtomicUsize, Ordering};
    #[derive(Clone, Copy, PartialEq)]
    enum PortCheck {
        KnowsAddressOnly,
        Old,
        LimitedOnce,
    }
    #[derive(Clone)]
    struct Relay {
        heartbeat_address: &'static str,
        mode: PortCheck,
        asked: Arc<AtomicUsize>,
    }
    async fn heartbeat(AxState(r): AxState<Relay>) -> axum::Json<Value> {
        axum::Json(json!({ "status": "ok", "address": r.heartbeat_address }))
    }
    async fn port_check(AxState(r): AxState<Relay>, axum::Json(body): axum::Json<Value>) -> axum::response::Response {
        use axum::response::IntoResponse;
        let asked = r.asked.fetch_add(1, Ordering::SeqCst);
        if r.mode == PortCheck::Old || body["ports"] != json!([]) {
            return (axum::http::StatusCode::BAD_REQUEST, axum::Json(json!({ "error": "VALIDATION_ERROR" }))).into_response();
        }
        if r.mode == PortCheck::LimitedOnce && asked == 0 {
            return axum::http::StatusCode::TOO_MANY_REQUESTS.into_response();
        }
        axum::Json(json!({ "address": "203.0.113.7", "publicAddress": true, "probes": [] })).into_response()
    }
    for (heartbeat_address, mode, shown, asked) in [
        ("2001:db8::1", PortCheck::KnowsAddressOnly, "203.0.113.7", 1),
        ("2001:db8::1", PortCheck::Old, "2001:db8::1", 1),
        ("2001:db8::1", PortCheck::LimitedOnce, "203.0.113.7", 2),
        ("198.51.100.4", PortCheck::KnowsAddressOnly, "198.51.100.4", 0),
    ] {
        let relay = Relay { heartbeat_address, mode, asked: Arc::default() };
        let app = axum::Router::new().route("/heartbeat", post(heartbeat)).route("/api/port-check", post(port_check)).with_state(relay.clone());
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let base = format!("http://{}", listener.local_addr().unwrap());
        tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });

        let tmp = tempfile::tempdir().unwrap();
        let agent = RelayAgent::new();
        let session = test_session(&agent, &base, "sbn_h1.secret", tmp.path()).await;
        {
            let mut inner = agent.lock();
            inner.cancel = Some(session.cancel.clone());
            inner.generation = 1;
            inner.status.running = true;
        }
        session.heartbeat_once().await;
        if mode == PortCheck::LimitedOnce {
            assert_eq!(agent.seen_address().as_deref(), Some(heartbeat_address));
            tokio::time::sleep(IPV4_RETRY).await;
        }
        session.heartbeat_once().await;
        session.heartbeat_once().await;
        assert_eq!(agent.seen_address().as_deref(), Some(shown), "{heartbeat_address}");
        assert_eq!(relay.asked.load(Ordering::SeqCst), asked, "{heartbeat_address}");
        session.cancel.cancel();
    }
}

/// heartbeat 계약 표본 (`tests/fixtures/relay-heartbeat-sample.json`, 릴레이 저장소의 같은 파일을 그대로 복사한 것).
/// 릴레이 시험은 이 표본을 스키마·처리기·카탈로그로 지나게 하고, 여기서는 데몬이 실제로 보내는 본문과 받는 답을 대조한다
fn heartbeat_sample() -> Value {
    serde_json::from_str(include_str!("../../tests/fixtures/relay-heartbeat-sample.json")).unwrap()
}

/// JSON 모양: 경로 → 값 형식들. 배열 원소는 `[]`, 키가 id·언어·칸 이름인 묶음은 `*`로 모은다.
/// 액션 `input`은 릴레이가 넘기기만 하는 JSON 스키마라 그 자체의 형식만 본다
fn shape(v: &Value, path: &str, out: &mut std::collections::BTreeMap<String, std::collections::BTreeSet<&'static str>>) {
    const MAPS: &[&str] = &[
        "metadata.allowedInstances", "metadata.fullNotice", "metadata.startNotice", "metadata.serverWords", "metadata.serverWords.*.commands",
        "metadata.permissions", "metadata.permissions.*.members", "metadata.permissions.*.commandLevels",
        "metadata.permissions.*.required.actions", "metadata.permissions.*.required.commands", "metadata.permissions.*.required.commands.*",
        "metadata.permissions.*.required.servers", "metadata.permissions.*.required.servers.*.actions", "metadata.permissions.*.required.servers.*.commands",
        "metadata.actions[].title", "metadata.actions[].name", "metadata.actions[].description",
        "metadata.modules[].display_names", "metadata.modules[].module_aliases_by_language",
        "metadata.modules[].commands.fields[].titles", "metadata.modules[].commands.fields[].descriptions",
        "metadata.modules[].commands.fields[].aliases_by_language", "metadata.modules[].commands.fields[].inputs[].titles",
        "latestReleases",
    ];
    let kind = match v {
        Value::Null => "null",
        Value::Bool(_) => "bool",
        Value::Number(_) => "number",
        Value::String(_) => "string",
        Value::Array(_) => "array",
        Value::Object(_) => "object",
    };
    out.entry(path.to_string()).or_default().insert(kind);
    if path.ends_with("].input") {
        return;
    }
    let join = |key: &str| if path.is_empty() { key.to_string() } else { format!("{path}.{key}") };
    match v {
        Value::Array(items) => items.iter().for_each(|i| shape(i, &format!("{path}[]"), out)),
        Value::Object(map) if MAPS.contains(&path) => map.values().for_each(|i| shape(i, &join("*"), out)),
        Value::Object(map) => map.iter().for_each(|(k, i)| shape(i, &join(k), out)),
        _ => {}
    }
}

/// 데몬이 보낸 값의 경로가 모두 표본에 같은 형식으로 있는가. 어긋난 (경로, 형식)을 돌려준다.
/// 어느 쪽이 키 이름을 바꿔도 여기서 드러난다. 표본의 null은 "값이 없을 수도 있다"라 어느 형식이든 받는다
fn off_contract(sent: &Value, samples: &[&Value], except: &[&str]) -> Vec<(String, &'static str)> {
    let (mut ours, mut theirs) = (Default::default(), Default::default());
    shape(sent, "", &mut ours);
    samples.iter().for_each(|sample| shape(sample, "", &mut theirs));
    ours.into_iter()
        .filter(|(p, _)| !except.iter().any(|e| p == e || p.starts_with(&format!("{e}."))))
        .flat_map(|(p, kinds)| {
            let known = theirs.get(&p).cloned().unwrap_or_default();
            kinds.into_iter().filter(move |k| !(known.contains(k) || (known.contains("null") || *k == "null") && !known.is_empty())).map(move |k| (p.clone(), k)).collect::<Vec<_>>()
        })
        .collect()
}

/// 실제 하트비트 본문(첫 전체·줄인 것)과 서버·모듈 한 줄이 표본과 같은 키·형식이고, 표본의 답(공인 주소·새로 묶인 길드·
/// 메타데이터 다시 달라기)을 데몬이 읽는다. 어느 저장소에서 키 이름을 바꾸면 이 시험이나 릴레이 시험이 깨진다
#[tokio::test]
async fn the_heartbeat_matches_the_contract_sample_shared_with_the_relay() {
    use axum::{extract::State as AxState, routing::post};
    use std::collections::VecDeque;
    let sample = heartbeat_sample();
    // 표본은 공개 저장소에 들어간다: 가짜 값만
    let text = sample.to_string();
    assert!(!text.contains("sbn_") && !text.contains("sk-"), "the sample holds no tokens or keys");

    type Shared = Arc<Mutex<(Vec<Value>, VecDeque<Value>)>>;
    async fn heartbeat(AxState(shared): AxState<Shared>, axum::Json(body): axum::Json<Value>) -> axum::Json<Value> {
        let mut s = shared.lock().unwrap();
        s.0.push(body);
        axum::Json(s.1.pop_front().unwrap_or(Value::Null))
    }
    // 업데이트 알림(latestReleases·updateHint)은 이 시험이 보지 않는다: 실으면 실제 업데이트 확인이 시작된다
    let mut response = sample["response"].clone();
    for key in ["latestReleases", "updateHint"] {
        response.as_object_mut().unwrap().remove(key);
    }
    let shared: Shared = Arc::new(Mutex::new((Vec::new(), VecDeque::from([response.clone(), response, sample["needMetadataResponse"].clone()]))));
    let app = axum::Router::new().route("/heartbeat", post(heartbeat)).with_state(shared.clone());
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let base = format!("http://{}", listener.local_addr().unwrap());
    tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });

    let tmp = tempfile::tempdir().unwrap();
    let agent = RelayAgent::new();
    let session = test_session(&agent, &base, "sbn_h1.secret", tmp.path()).await;
    {
        let mut inner = agent.lock();
        inner.cancel = Some(session.cancel.clone());
        inner.generation = 1;
        inner.status.running = true;
    }
    // 권한·허용 서버가 실리게 길드 하나를 둔다
    let guild = "600000000000000001";
    session.server.config_store.update_node_settings(guild, |node| {
        node.insert("allowedInstances".into(), json!(["inst-0001"]));
        ((), true)
    }).await.unwrap();

    assert!(session.heartbeat_once().await, "a newly linked guild is acknowledged at once");
    session.heartbeat_once().await;
    let sent = shared.lock().unwrap().0.clone();
    // moduleDetails: 새 릴레이는 받지 않지만 운영 중인 릴레이 스키마가 요구한다 (`metadata`의 주석)
    assert_eq!(off_contract(&sent[0], &[&sample["request"]], &["metadata.moduleDetails"]), vec![], "{}", sent[0]);
    assert!(sent[0]["metadata"]["permissions"][guild].is_object() && sent[0]["metadata"]["actions"].as_array().is_some_and(|a| !a.is_empty()));
    // 답의 공인 주소와 새로 묶인 길드를 읽었다: 다음 하트비트가 그 연결 id로 확인 응답을 싣는다
    assert_eq!(agent.lock().status.seen_address.as_deref(), sample["response"]["address"].as_str());
    assert_eq!(sent[1]["linkedGuildsAckIds"], json!([sample["response"]["linkedGuilds"][0]["linkId"]]));

    // 릴레이가 지문을 받아 두었으면 줄여 보내고, 모른다고 하면(needMetadata) 곧바로 전체를 다시 보낸다
    *session.metadata_sent.lock().unwrap() = sent[1]["metadataHash"].as_str().map(str::to_string);
    assert!(session.heartbeat_once().await, "needMetadata asks for the whole metadata again");
    let reduced = shared.lock().unwrap().0[2].clone();
    assert_eq!(reduced["metadataUnchanged"], json!(true));
    // 줄인 본문은 전체 본문의 일부 키다 (표본의 줄인 본문은 예시라 PC 부하·예약·확인 응답이 없다)
    assert_eq!(off_contract(&reduced, &[&sample["reducedRequest"], &sample["request"]], &[]), vec![], "{reduced}");

    // 서버·모듈 한 줄 (빈 데몬에는 없다): 표본 줄을 데몬 값으로 만들어 대조한다
    let server = json!({
        "id": "inst-0001", "name": "friends", "module": "minecraft", "status": "running", "access": "members",
        "controllers": [{ "name": "Steve", "discord_id": "300000000000000002" }], "players": { "online": 2, "max": 20 },
        "start_time": 1_731_400_000u64, "extension_status": { "docker": { "status_line": "TPS 20.0" } },
    });
    let usage = crate::system_monitor::InstanceMetrics {
        id: "inst-0001".into(), name: "friends".into(), running: true, process_count: 1,
        cpu_percent: 12.5, memory_bytes: 2048 * 1024 * 1024, disk_read_bps: 0, disk_write_bps: 0, net_rx_bps: None, net_tx_bps: None,
    };
    crate::supervisor::chat_link::toggle("inst-0001", "600000000000000001", "700000000000000001", None, |_| Ok::<(), ()>(())).unwrap();
    let entry = relay_server_entry(&server, Some(&usage));
    assert_eq!(off_contract(&entry, &[&sample["request"]["metadata"]["servers"][0]], &[]), vec![], "{entry}");
    assert_eq!(entry.as_object().unwrap().len(), sample["request"]["metadata"]["servers"][0].as_object().unwrap().len(), "{entry}");

    let module_dir = tmp.path().join("mc");
    std::fs::create_dir_all(module_dir.join("locales")).unwrap();
    std::fs::write(
        module_dir.join("locales").join("ko.json"),
        r#"{"module": {"display_name": "마인크래프트"}, "aliases": {"module": ["마크"], "commands": {"say": ["공지"]}}, "commands": {"say": {"label": "말하기", "description": "모두에게 말해요", "inputs": {"message": {"label": "메시지"}}}}}"#,
    )
    .unwrap();
    let say: crate::module_loader::CommandField = serde_json::from_value(json!({
        "name": "say", "label": "Say", "description": "Broadcast a message", "risk": "safe",
        "inputs": [{ "name": "message", "label": "Message", "type": "text", "required": true }],
    }))
    .unwrap();
    let view = crate::chat::api::ModuleView {
        name: "minecraft".into(),
        game_names: vec!["Minecraft".into()],
        module_aliases: vec!["mc".into()],
        display_name: Some("Minecraft".into()),
        command_aliases: vec![("say".into(), vec!["broadcast".into()])],
        commands: vec![say],
        presets: vec![serde_json::from_value(json!({ "id": "vanilla", "label": "Vanilla" })).unwrap()],
        locale_dir: Some(module_dir),
        ..Default::default()
    };
    let module = relay_module_entry(&view);
    let sample_module = &sample["request"]["metadata"]["modules"][0];
    assert_eq!(off_contract(&module, &[sample_module], &[]), vec![], "{module}");
    let keys = |v: &Value| v.as_object().unwrap().keys().cloned().collect::<std::collections::BTreeSet<_>>();
    assert_eq!(keys(&module), keys(sample_module), "{module}");
    assert_eq!(keys(&module["commands"]["fields"][0]), keys(&sample_module["commands"]["fields"][0]), "{module}");
    session.cancel.cancel();
}

/// 릴레이가 읽지 않는 값(기능 목록, 옛 전역 별명)은 하트비트에 싣지 않는다. 릴레이 스키마가 버리는 값이다
#[tokio::test]
async fn heartbeat_leaves_out_what_the_relay_drops() {
    let tmp = tempfile::tempdir().unwrap();
    let session = test_session(&RelayAgent::new(), "http://127.0.0.1:9", "sbn_h1.secret", tmp.path()).await;
    let meta = session.metadata().await;
    assert!(meta.get("features").is_none());
    assert_eq!(meta["botConfig"].as_object().unwrap().keys().collect::<Vec<_>>(), vec!["prefixes"]);
    // 2026-11-12 전 릴레이 스키마는 줄이지 않은 하트비트에 이 둘을 요구한다
    assert!(meta["modules"].is_array() && meta["moduleDetails"].is_object());
}

/// 새로 묶인 길드를 적용했으면 다음 하트비트가 릴레이가 준 연결 id를 확인 응답(`linkedGuildsAckIds`)으로 싣는다.
/// 없으면 싣지 않는다
#[test]
fn heartbeat_acks_the_linked_guilds_it_applied() {
    let acks = ["a".to_string(), "b".to_string()];
    let body = heartbeat_body(json!({}), "h", false, &acks);
    assert_eq!(body["linkedGuildsAckIds"], json!(["a", "b"]));
    assert!(body.get("linkedGuildsAck").is_none());
    assert_eq!(body["metadataHash"], "h");
    assert!(body.get("metadataUnchanged").is_none());
    let body = heartbeat_body(json!({}), "h", true, &[]);
    assert!(body.get("linkedGuildsAck").is_none());
    assert!(body.get("linkedGuildsAckIds").is_none());
    assert_eq!(body["metadataUnchanged"], true);
}

/// 코드 없이 묶인 길드는 서버를 나중에 허용하고, 코드로 묶인 길드는 PC 설정을 따른다. 이미 정한 길드는 건드리지 않는다
#[test]
fn linked_guilds_open_servers_by_how_they_were_linked() {
    use crate::config_store::NewGuildInstances::{Auto, Manual};
    let node = |v: Value| v.as_object().unwrap().clone();
    let mut n = node(json!({}));
    assert!(close_linked_guild(&mut n, "link", Auto));
    assert_eq!(n["allowedInstances"], json!([]));
    let mut n = node(json!({}));
    assert!(!close_linked_guild(&mut n, "code", Auto), "auto opens every server");
    assert!(n.get("allowedInstances").is_none());
    let mut n = node(json!({}));
    assert!(close_linked_guild(&mut n, "code", Manual));
    assert_eq!(n["allowedInstances"], json!([]));
    let mut n = node(json!({ "allowedInstances": ["i-mc"] }));
    assert!(!close_linked_guild(&mut n, "link", Manual), "a list chosen before stays");
    assert_eq!(n["allowedInstances"], json!(["i-mc"]));
    let mut n = node(json!({}));
    assert!(!close_linked_guild(&mut n, "other", Manual), "unknown ways are ignored");
}

#[test]
fn heartbeat_carries_permissions_for_every_known_guild() {
    let nodes = json!({
        "g1": { "members": { "a": 2, "bad": 7 }, "ownerDiscordId": "o", "chatbotCap": 1, "commandLevels": { "i/stop": 3 } },
        "g2": { "ownerDiscordId": "o", "allowedInstances": ["x"], "preset": "strict" },
        "g3": { "defaultLevel": -1 }
    });
    // 릴레이가 요구 레벨을 미리 셀 수 있게 프리셋을 푼 risk별 레벨, 챗봇 상한, 명령별 덮어쓰기도 싣는다
    let default_levels = json!({ "safe": 0, "caution": 1, "danger": 2, "admin": 2 });
    let ladder = json!([{ "id": 3 }, { "id": 2 }, { "id": 1 }, { "id": 0 }, { "id": -1 }]);
    let nothing = json!({ "actions": {}, "commands": {}, "servers": {} });
    assert_eq!(
        permissions_by_guild(&nodes, &Default::default()),
        json!({
            "g1": { "defaultLevel": 1, "members": { "a": 2 }, "chatbotCap": 1, "levels": default_levels, "commandLevels": { "i/stop": 3 }, "ladder": ladder, "required": nothing },
            "g2": { "defaultLevel": 1, "members": {}, "chatbotCap": 0, "levels": { "safe": 1, "caution": 2, "danger": 3, "admin": 3 }, "commandLevels": {}, "ladder": ladder, "required": nothing },
            "g3": { "defaultLevel": -1, "members": {}, "chatbotCap": 0, "levels": default_levels, "commandLevels": {}, "ladder": ladder, "required": nothing }
        })
    );
    // 요구 레벨은 그 길드가 볼 수 있는 서버만, 권한 판정과 같은 값으로 싣는다 (`actions::required_snapshot`)
    let targets = crate::actions::RemoteTargets {
        actions: vec![json!({ "id": "server.stop", "risk": "safe", "scope": "instance" })],
        commands: Vec::new(),
        instances: vec![("i".into(), "mc".into()), ("x".into(), "mc".into())],
    };
    let listed = permissions_by_guild(&nodes, &targets);
    assert_eq!(listed["g1"]["required"], json!({ "actions": { "server.stop": 1 }, "commands": {}, "servers": { "i": { "actions": { "server.stop": 3 } } } }));
    assert_eq!(listed["g2"]["required"]["servers"], json!({}));
    let many: serde_json::Map<String, Value> = (0..COMMAND_LEVELS_PER_GUILD + 5).map(|i| (format!("i/c{i:04}"), json!(2))).collect();
    let big = permissions_by_guild(&json!({ "g": { "commandLevels": many } }), &Default::default());
    assert_eq!(big["g"]["commandLevels"].as_object().unwrap().len(), COMMAND_LEVELS_PER_GUILD);
    assert_eq!(big["g"]["commandLevelsTruncated"], true);
    assert!(big["g"].get("required").is_none(), "left to the daemon when the overrides do not fit");
}

#[test]
fn a_half_emoji_does_not_drop_the_whole_poll_batch() {
    let body = r#"{"commands":[{"id":"1","payload":{"text":"say \ud83d"}},{"id":"2","payload":{"text":"ok 😀 \\ud83d"}}]}"#;
    let v = decode_poll(body);
    let cmds = v["commands"].as_array().unwrap();
    assert_eq!(cmds.len(), 2);
    assert_eq!(cmds[0]["payload"]["text"], "say \u{fffd}");
    // 짝이 맞는 이모지와 이스케이프된 역슬래시 뒤의 글자는 그대로
    assert_eq!(cmds[1]["payload"]["text"], "ok 😀 \\ud83d");
    assert_eq!(decode_poll("not json"), Value::Null);
}

#[test]
fn allowed_instances_are_sent_only_for_guilds_that_list_them() {
    let settings = json!({
        "g1": { "allowedInstances": ["i-mc", 3, "i-pal"] },
        "g2": { "memberPermissions": {} },
        "g3": { "allowedInstances": [] },
    });
    assert_eq!(
        allowed_instances_by_guild(&settings),
        Some(json!({ "g1": ["i-mc", "i-pal"], "g3": [] }))
    );
    assert_eq!(allowed_instances_by_guild(&json!({ "g2": {} })), None);
    assert_eq!(allowed_instances_by_guild(&json!(null)), None);
}

// PC 주인 DM 알림: 백업 범위와 끈 종류 (사용자 2026-11-08)
#[test]
fn dm_notice_is_sent_only_when_something_is_off() {
    use crate::config_store::{BackupNotice, BotConfig};
    let mut meta = json!({});
    dm_notice_meta(&mut meta, &BotConfig::default());
    assert_eq!(meta, json!({}));

    let mut bot: BotConfig = serde_json::from_value(json!({ "backupNotice": "off", "dmNotice": { "crash": false, "unresponsive": false, "chatbotUsage": false } })).unwrap();
    assert_eq!(bot.backup_notice, BackupNotice::Off);
    assert!(bot.dm_notice.moderation, "빠진 칸은 켜짐");
    dm_notice_meta(&mut meta, &bot);
    assert_eq!(meta, json!({ "backupNotice": "off", "dmOff": ["crash", "unresponsive", "chatbotUsage"] }));

    bot.backup_notice = BackupNotice::Failures;
    let mut meta = json!({});
    dm_notice_meta(&mut meta, &bot);
    assert_eq!(meta["backupNotice"], "failures");

    // 켜야 보내는 종류(PC 연결 끊김)는 켰을 때만 `dmOn`으로 싣고, 끈 종류에는 넣지 않는다
    let bot: BotConfig = serde_json::from_value(json!({ "dmNotice": { "offline": true } })).unwrap();
    let mut meta = json!({});
    dm_notice_meta(&mut meta, &bot);
    assert_eq!(meta, json!({ "dmOn": ["offline"] }));
}

// 켜짐 알림을 켠 서버는 길드마다 싣는다. 켠 서버가 없는 길드는 싣지 않는다 (릴레이는 없으면 알리지 않는다)
#[test]
fn start_notice_is_sent_per_guild() {
    let settings = json!({
        "g1": { "startNotice": ["i-mc", 3] },
        "g2": { "allowedInstances": ["i-mc"] },
    });
    assert_eq!(ids_by_guild(&settings, "startNotice"), Some(json!({ "g1": ["i-mc"] })));
    assert_eq!(ids_by_guild(&json!({ "g2": {} }), "startNotice"), None);
}

#[test]
fn finished_old_session_leaves_new_session_alone() {
    let agent = RelayAgent::new();
    {
        let mut inner = agent.lock();
        inner.generation = 2;
        inner.cancel = Some(CancellationToken::new());
        inner.status.running = true;
    }
    agent.finish(1, Some("old".into()));
    assert!(agent.lock().cancel.is_some() && agent.lock().status.running);
    agent.finish(2, None);
    assert!(agent.lock().cancel.is_none() && !agent.lock().status.running);
}

#[test]
fn remote_action_module_errors_hide_diagnostics_and_sensitive_text() {
    let out = action_result(Err(crate::actions::ActionError {
        status: 500, code: "module_exception", message: "peer 203.0.113.7 token=hunter2".into(),
        data: json!({ "success": false, "module": "plain", "function": "start", "path": "/home/me/server", "stderr_tail": ["private traceback"] }),
    }));
    assert!(out["data"].get("stderr_tail").is_none());
    assert_eq!(out["data"]["error"], "peer x.x.x.x token=***");
    assert_eq!(out["data"]["path"], "<path>");
}

#[test]
fn game_chat_is_dropped_first_when_the_queue_is_full() {
    let ev = |id: u64, kind: &str| crate::events::Event { id, kind: kind.into(), data: json!({"instanceId": "i", "kind": "crash", "text": "x"}), replay: true };
    let mut pending = std::collections::VecDeque::new();
    queue_event(&mut pending, &ev(1, "instance.exit"));
    for id in 2..=(EVENT_PENDING as u64 + 5) {
        queue_event(&mut pending, &ev(id, "instance.chat"));
    }
    assert_eq!(pending.len(), EVENT_PENDING);
    assert_eq!(pending[0]["type"], "instance.exit", "a crash outlives a flood of chat");
    assert_eq!(pending.back().unwrap()["id"], EVENT_PENDING as u64 + 5, "the newest chat stays");
}

#[test]
fn only_notifications_go_to_the_relay_without_paths_or_console_lines() {
    let ev = |kind: &str, data: Value| crate::events::Event { id: 7, kind: kind.into(), data, replay: true };
    let crash = ev(
        "instance.exit",
        json!({"at": 1, "instanceId": "i", "instance": "마크", "kind": "crash", "exitCode": 1, "signal": null, "evidence": "crash file C:\\Users\\me\\crash.txt", "restartInSecs": 10, "note": null}),
    );
    let sent = relay_event(&crash).unwrap();
    assert_eq!(sent["type"], "instance.exit");
    assert_eq!(sent["id"], 7);
    assert_eq!(sent["data"]["restartInSecs"], 10);
    assert!(sent["data"].get("evidence").is_none());

    let coded = ev("instance.exit", json!({"kind": "unknown_exit", "evidence": "exit code 3"}));
    assert_eq!(relay_event(&coded).unwrap()["data"]["evidence"], "exit code 3");
    let console = ev("instance.exit", json!({"kind": "crash", "evidence": "console: joined from 10.0.0.2"}));
    assert!(relay_event(&console).unwrap()["data"].get("evidence").is_none());
    assert!(relay_event(&ev("instance.exit", json!({"kind": "user_stop"}))).is_none());

    let failed = ev("backup.done", json!({"instanceId": "i", "instance": "mc1", "kind": "manual", "ok": false, "error": "D:\\saves locked"}));
    assert_eq!(relay_event(&failed).unwrap()["data"], json!({"instanceId": "i", "instance": "mc1", "kind": "manual", "ok": false}));
    assert!(relay_event(&ev("action.run", json!({"action": "server.start"}))).is_none());
    assert!(relay_event(&ev("provision.progress", json!({}))).is_none());
    let silent = ev("instance.unresponsive", json!({"at": 1, "instanceId": "i", "instance": "mc1", "minutes": 10, "restarting": false, "reason": "connection refused"}));
    assert_eq!(relay_event(&silent).unwrap()["data"], json!({"at": 1, "instanceId": "i", "instance": "mc1", "minutes": 10, "restarting": false}));
    // R39 준비 알림은 계약 필드만 (디스코드 "다 켜졌어요")
    let ready = ev("instance.ready", json!({"at": 1, "instanceId": "i", "instance": "mc1", "ok": false, "code": "start_failed", "line": "C:\\x"}));
    assert_eq!(relay_event(&ready).unwrap()["data"], json!({"at": 1, "instanceId": "i", "instance": "mc1", "ok": false, "code": "start_failed"}));
    // 켠 서버의 업데이트 알림은 종류·모듈·새 버전만
    assert_eq!(relay_event(&ev("instance.updated", json!({"at": 1, "instanceId": "i", "instance": "pal"}))).unwrap()["data"], json!({"at": 1, "instanceId": "i", "instance": "pal"}));
    let notice = ev("instance.update_notice", json!({"at": 1, "instanceId": "i", "instance": "pal", "kind": "module", "module": "palworld", "latest": "1.1.0", "path": "C:\\x"}));
    assert_eq!(relay_event(&notice).unwrap()["data"], json!({"at": 1, "instanceId": "i", "instance": "pal", "kind": "module", "module": "palworld", "latest": "1.1.0"}));
    // 자동 업데이트 대기는 접속자 수만, 실패는 오류 원문(로컬 경로가 섞인다) 없이
    let waiting = ev("instance.update_waiting", json!({"at": 1, "instanceId": "i", "instance": "pal", "players": 2}));
    assert_eq!(relay_event(&waiting).unwrap()["data"], json!({"at": 1, "instanceId": "i", "instance": "pal", "players": 2}));
    let failed_update = ev("instance.update_failed", json!({"at": 1, "instanceId": "i", "instance": "pal", "error": "C:\\x locked"}));
    assert_eq!(relay_event(&failed_update).unwrap()["data"], json!({"at": 1, "instanceId": "i", "instance": "pal"}));
    // 게임 채팅은 데몬이 만든 문장만 (채팅 원문·이름 칸은 따로 싣지 않는다)
    let chat = ev("instance.chat", json!({"at": 1, "instanceId": "i", "instance": "mc1", "text": "\\[mc1\\] **Steve**: hi", "player": "Steve", "message": "hi"}));
    assert_eq!(relay_event(&chat).unwrap()["data"], json!({"at": 1, "instanceId": "i", "instance": "mc1", "text": "\\[mc1\\] **Steve**: hi"}));
    // 채팅 연동: 연동 채널과 그 줄, 저절로 꺼진 연동
    let linked = ev("instance.chat", json!({"at": 1, "instanceId": "i", "instance": "mc1", "links": [{"guild": "1", "channel": "2"}], "linkText": "l"}));
    assert_eq!(relay_event(&linked).unwrap()["data"], json!({"at": 1, "instanceId": "i", "instance": "mc1", "links": [{"guild": "1", "channel": "2"}], "linkText": "l"}));
    let ended = ev("instance.chat_link", json!({"at": 1, "instanceId": "i", "instance": "mc1", "guild": "1", "channel": "2", "active": false, "reason": "empty"}));
    assert_eq!(relay_event(&ended).unwrap()["data"]["reason"], "empty");
    // 시작·정상 정지는 누가 시켰는지만 (사바쨩 밖의 시작은 unexpected)
    let started = ev("instance.started", json!({"at": 1, "instanceId": "i", "instance": "mc1", "source": "external", "unexpected": true, "path": "C:\\x"}));
    assert_eq!(relay_event(&started).unwrap()["data"], json!({"at": 1, "instanceId": "i", "instance": "mc1", "source": "external", "unexpected": true}));
    let stopped = ev("instance.stopped", json!({"at": 1, "instanceId": "i", "instance": "mc1", "source": "gui", "userName": "집 PC", "kind": "user_stop"}));
    assert_eq!(relay_event(&stopped).unwrap()["data"], json!({"at": 1, "instanceId": "i", "instance": "mc1", "source": "gui", "userName": "집 PC", "kind": "user_stop"}));
    // 접속자가 없어 자동 정지: 비어 있던 분
    let idle = ev("instance.stopped", json!({"at": 1, "instanceId": "i", "instance": "mc1", "source": "idle", "kind": "user_stop", "idleMinutes": 30}));
    assert_eq!(relay_event(&idle).unwrap()["data"]["idleMinutes"], 30);
    // 여러 단계 실행은 디스코드에서 시작한 것만
    assert!(relay_event(&ev("plan.done", json!({"runId": "r", "ok": true}))).is_none());
    assert_eq!(relay_event(&ev("plan.done", json!({"runId": "r", "guildId": "g", "ok": true}))).unwrap()["data"]["guildId"], "g");
}

#[test]
fn a_redelivered_command_runs_once_and_gets_its_result_again() {
    let mut seen = SeenCommands::default();
    let t = std::time::Instant::now();
    assert_eq!(seen.receive("c1", t), Received::New);
    seen.start("c1");
    // 처리 중에 다시 오면 실행하지 않는다 (끝날 때 결과가 한 번 나간다)
    assert_eq!(seen.receive("c1", t + Duration::from_secs(15)), Received::Running);
    seen.finish("c1", &json!({"ok": true}));
    assert_eq!(seen.receive("c1", t + Duration::from_secs(30)), Received::Done(json!({"ok": true})));
    assert_eq!(seen.receive("c2", t + Duration::from_secs(30)), Received::New);
}

/// 연결을 다시 맺어도 끝난 명령과 실행 중인 명령은 기억한다. 받아 두고 시작하지 않은 명령은 잊어서 다시 오면 실행한다
#[test]
fn a_reconnect_keeps_finished_commands_and_forgets_unfinished_ones() {
    let mut seen = SeenCommands::default();
    let t = std::time::Instant::now();
    seen.receive("done", t);
    seen.finish("done", &json!({"ok": true}));
    seen.receive("dropped", t);
    // 실행 중인 명령은 남는다
    seen.receive("running", t);
    seen.start("running");
    seen.forget(&["dropped"]);
    assert_eq!(seen.receive("done", t), Received::Done(json!({"ok": true})));
    assert_eq!(seen.receive("dropped", t), Received::New);
    assert_eq!(seen.receive("running", t), Received::Running);
}

/// 실행 중인 명령이 다시 오면(ack가 닿지 않았다) 다시 ack한다. 아직 시작하지 않은 명령은 시작할 때 ack한다
#[test]
fn a_redelivered_command_is_told_apart_as_waiting_or_running() {
    let mut seen = SeenCommands::default();
    let t = std::time::Instant::now();
    assert_eq!(seen.receive("c1", t), Received::New);
    assert_eq!(seen.receive("c1", t), Received::Waiting);
    seen.start("c1");
    assert_eq!(seen.receive("c1", t), Received::Running);
}

#[test]
fn finished_commands_are_forgotten_after_ten_minutes_or_past_the_cap() {
    let mut seen = SeenCommands::default();
    let t = std::time::Instant::now();
    seen.receive("old", t);
    seen.finish("old", &json!({"ok": true}));
    assert_eq!(seen.receive("old", std::time::Instant::now() + SEEN_FOR), Received::New);

    let mut seen = SeenCommands::default();
    for i in 0..=SEEN_MAX {
        seen.receive(&format!("c{i}"), t);
        seen.finish(&format!("c{i}"), &json!({"ok": true}));
    }
    assert_eq!(seen.entries.len(), SEEN_MAX);
    assert_eq!(seen.receive("c0", t), Received::New);
    assert_eq!(seen.receive(&format!("c{SEEN_MAX}"), t), Received::Done(json!({"ok": true})));
}

/// 외부 감사: 대기열이 밀려 10분 넘게 기다린 명령을 시작한 뒤 다시 받으면, 시간 정리가 그 명령까지 잊어 또 실행했다
#[test]
fn waiting_or_running_commands_are_not_forgotten_by_time_or_cap() {
    let mut seen = SeenCommands::default();
    let t = std::time::Instant::now();
    seen.receive("waiting", t);
    seen.receive("running", t);
    seen.start("running");
    seen.receive("done", t);
    seen.finish("done", &json!({"ok": true}));
    let later = t + SEEN_FOR + Duration::from_secs(1);
    assert_eq!(seen.receive("waiting", later), Received::Waiting);
    assert_eq!(seen.receive("running", later), Received::Running);
    assert_eq!(seen.receive("done", later), Received::New, "a finished one is still forgotten");
    // 개수를 넘으면 끝난 명령부터 잊는다
    for i in 0..SEEN_MAX {
        seen.receive(&format!("c{i}"), later);
        seen.finish(&format!("c{i}"), &json!({"ok": true}));
    }
    assert_eq!(seen.receive("waiting", later), Received::Waiting);
    assert_eq!(seen.receive("running", later), Received::Running);
}

/// 독립 리뷰: 10분 넘게 기다린 명령이 끝나자마자 다시 오면, 받은 시각 기준으로 잊혀 또 실행했다
#[test]
fn a_long_waiting_command_is_kept_for_ten_minutes_after_it_finishes() {
    let mut seen = SeenCommands::default();
    let now = std::time::Instant::now();
    assert_eq!(seen.receive("c1", now - SEEN_FOR - Duration::from_secs(1)), Received::New);
    seen.start("c1");
    seen.finish("c1", &json!({"ok": true}));
    assert_eq!(seen.receive("c1", now), Received::Done(json!({"ok": true})));
}

#[test]
fn acks_carry_every_started_id_in_batches_the_relay_accepts() {
    let commands: Vec<Value> = (0..120).map(|i| json!({"id": format!("c{i}")})).chain([json!({"payload": {}})]).collect();
    let bodies = ack_bodies(&commands);
    let ids: Vec<Vec<String>> = bodies.iter().map(|b| serde_json::from_str::<Value>(b).unwrap()["ids"].as_array().unwrap().iter().map(|v| v.as_str().unwrap().to_string()).collect()).collect();
    assert_eq!(ids.iter().map(Vec::len).collect::<Vec<_>>(), vec![50, 50, 20]);
    assert_eq!(ids.concat(), (0..120).map(|i| format!("c{i}")).collect::<Vec<_>>());
    assert!(ack_bodies(&[]).is_empty());
}

#[test]
fn semver_compare() {
    assert!(semver_lt("0.1.0", "0.1.1"));
    assert!(!semver_lt("0.1.1", "0.1.0"));
    assert!(!semver_lt("1.0", "1.0.0"));
    assert!(semver_lt("0.9.9", "1"));
}

#[test]
fn prerelease_agent_is_not_older_than_previous_release() {
    assert!(!semver_lt("0.1.11-beta.1", "0.1.10"));
    assert!(semver_lt("0.1.11-beta.1", "0.1.11"));
    assert!(semver_lt("0.1.11-beta.1", "0.1.11-beta.2"));
    assert!(!semver_lt("0.1.11-rc.1", "0.1.11-beta.2"));
}

#[test]
fn backoff_doubles_up_to_cap() {
    assert_eq!(backoff(1), Duration::from_secs(3));
    assert_eq!(backoff(2), Duration::from_secs(6));
    assert_eq!(backoff(10), RETRY_MAX);
    assert_eq!(backoff(100), RETRY_MAX);
}

#[test]
fn encodes_like_encode_uri_component() {
    assert_eq!(encode_segment("abc-123"), "abc-123");
    assert_eq!(encode_segment("a b/c"), "a%20b%2Fc");
}
