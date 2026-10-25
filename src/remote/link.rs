//! 릴레이 웹소켓 연결 (`<relay>/node/ws`, 노드 토큰)
//!
//! 릴레이가 명령 대기열로 `web_session`을 보내면 붙고, 웹 세션이 하나도 없는 채로 `IDLE`이 지나면 닫는다.
//! 평소에는 열어 두지 않는다. 주고받는 것은 한 줄 JSON이다:
//! - 릴레이 → 데몬: `{t:"open", sid, user}` · `{t:"close", sid}` · `{t:"req", sid, id, method, path, body?}`
//! - 데몬 → 릴레이: `{t:"hello", version, protocol}` · `{t:"res", sid, id, status, body}` · `{t:"event", id, kind, data}`
//!
//! 이벤트는 세션을 가리지 않고 한 번 보내고, 릴레이가 그 PC의 세션들에 나눠 준다.

use super::exec;
use crate::ipc::IPCServer;
use futures_util::{SinkExt, StreamExt};
use serde_json::{json, Value};
use std::collections::HashMap;
use std::sync::{Arc, Mutex};
use std::time::Duration;
use tokio::sync::{mpsc, Semaphore};
use tokio_tungstenite::tungstenite::{client::IntoClientRequest, protocol::WebSocketConfig, Message};
use tokio_util::sync::CancellationToken;

/// 브라우저가 이 번호를 보고 지원하는 데몬인지 판단한다
pub const PROTOCOL: u32 = 1;
/// 세션이 없는 채로 이만큼 지나면 닫는다
const IDLE: Duration = if cfg!(test) {
    Duration::from_millis(300)
} else {
    Duration::from_secs(60)
};
/// 연결을 살려 두는 ping 간격 (nginx·릴레이 유휴 끊김보다 짧게)
const PING: Duration = Duration::from_secs(25);
/// 동시에 실행하는 원격 요청 수
const IN_FLIGHT: usize = 8;
/// 실행 중과 기다리는 요청을 합한 상한. 넘치면 쌓지 않고 바로 `remote_busy`로 답한다
const QUEUED: usize = 32;
/// 한 메시지 상한: 로그 내보내기(본문 8 MiB 남짓)가 들어간다
const MAX_MESSAGE: usize = 16 << 20;
/// 세션이 남은 채 끊기면 다시 붙어 보는 횟수 (1·2·4초 뒤)
const RECONNECTS: u32 = 3;

static RUNNING: Mutex<Option<CancellationToken>> = Mutex::new(None);

/// 연결은 프로세스에 하나라서, 그것을 여는 시험은 하나씩 돈다
#[cfg(test)]
pub(crate) static TEST_LOCK: tokio::sync::Mutex<()> = tokio::sync::Mutex::const_new(());

/// 연결이 없으면 연다. 이미 있으면 그대로 둔다 (릴레이는 새 세션마다 `web_session`을 보낸다)
pub fn ensure(server: IPCServer, relay_base: &str, token: &str) {
    let mut running = RUNNING.lock().unwrap_or_else(|e| e.into_inner());
    if running.as_ref().is_some_and(|c| !c.is_cancelled()) {
        return;
    }
    let cancel = CancellationToken::new();
    *running = Some(cancel.clone());
    let url = ws_url(relay_base);
    let token = token.to_string();
    tokio::spawn(async move {
        let mut failures = 0;
        while !cancel.is_cancelled() {
            match connect_once(&server, &url, &token, &cancel, &mut failures).await {
                Ok(Ended::Idle) | Ok(Ended::Cancelled) => break,
                Ok(Ended::Dropped) | Err(_) if failures < RECONNECTS => {
                    failures += 1;
                    // 기다리는 중에 멈추면 다시 붙지 않는다
                    tokio::select! {
                        _ = cancel.cancelled() => break,
                        _ = tokio::time::sleep(Duration::from_secs(1 << (failures - 1))) => {}
                    }
                }
                Ok(Ended::Dropped) | Err(_) => break,
            }
        }
        cancel.cancel();
    });
}

/// 연결을 닫는다 (릴레이 연결 해제·토큰 교체 때)
pub fn stop() {
    if let Some(cancel) = RUNNING.lock().unwrap_or_else(|e| e.into_inner()).take() {
        cancel.cancel();
    }
}

/// `https://relay` → `wss://relay/node/ws` (모의 릴레이는 http → ws)
pub fn ws_url(relay_base: &str) -> String {
    let base = relay_base.trim_end_matches('/');
    let base = base
        .strip_prefix("https://")
        .map(|r| format!("wss://{r}"))
        .or_else(|| base.strip_prefix("http://").map(|r| format!("ws://{r}")));
    format!("{}/node/ws", base.unwrap_or_else(|| relay_base.to_string()))
}

/// 웹으로 보내는 이벤트. 화면은 목록을 다시 읽을 때만 쓰므로 종류와 인스턴스 id만 담는다
/// (경로·콘솔 줄·오류 원문은 PC 밖으로 내보내지 않는다, I3)
fn web_event(event: &crate::events::Event) -> Option<Value> {
    let kind = event.kind.as_str();
    if !(kind.starts_with("instance.") || kind.starts_with("module.") || kind == "action.run") {
        return None;
    }
    let mut data = serde_json::Map::new();
    if let Some(id) = event.data.get("instanceId").filter(|v| v.is_string()) {
        data.insert("instanceId".into(), id.clone());
    }
    Some(json!({ "t": "event", "id": event.id, "kind": kind, "data": data }))
}

#[derive(Debug, PartialEq)]
enum Ended {
    Idle,
    Cancelled,
    /// 세션이 남은 채 끊겼다
    Dropped,
}

async fn connect_once(
    server: &IPCServer,
    url: &str,
    token: &str,
    cancel: &CancellationToken,
    failures: &mut u32,
) -> Result<Ended, String> {
    let mut request = url.into_client_request().map_err(|e| e.to_string())?;
    let auth = format!("Bearer {token}").parse().map_err(|_| "bad token".to_string())?;
    request.headers_mut().insert("Authorization", auth);
    request.headers_mut().insert(
        "X-Saba-Client",
        tokio_tungstenite::tungstenite::http::HeaderValue::from_static("1"),
    );
    let config = WebSocketConfig {
        max_message_size: Some(MAX_MESSAGE),
        max_frame_size: Some(MAX_MESSAGE),
        ..Default::default()
    };
    let (socket, _) = tokio_tungstenite::connect_async_with_config(request, Some(config), false)
        .await
        .map_err(|e| e.to_string())?;
    let (mut sink, mut stream) = socket.split();

    let hello = json!({ "t": "hello", "version": crate::relay_agent::AGENT_VERSION, "protocol": PROTOCOL });
    sink.send(Message::Text(hello.to_string()))
        .await
        .map_err(|e| e.to_string())?;
    // 붙었으니 다시 붙기 횟수를 새로 센다 (몇 시간 붙어 있던 연결이 예전 끊김 때문에 포기하지 않게)
    *failures = 0;

    let (out_tx, mut out_rx) = mpsc::unbounded_channel::<Value>();
    let sessions: Arc<Mutex<HashMap<String, Option<String>>>> = Arc::default();
    let queue = Arc::new(Semaphore::new(QUEUED));
    let slots = Arc::new(Semaphore::new(IN_FLIGHT));
    let mut events = crate::events::bus().subscribe(None).rx;
    let mut ping = tokio::time::interval(PING);
    ping.tick().await;
    let idle = tokio::time::sleep(IDLE);
    tokio::pin!(idle);

    loop {
        tokio::select! {
            _ = cancel.cancelled() => {
                let _ = sink.send(Message::Close(None)).await;
                return Ok(Ended::Cancelled);
            }
            () = &mut idle, if sessions.lock().unwrap_or_else(|e| e.into_inner()).is_empty() => {
                let _ = sink.send(Message::Close(None)).await;
                return Ok(Ended::Idle);
            }
            _ = ping.tick() => {
                sink.send(Message::Ping(Vec::new())).await.map_err(|e| e.to_string())?;
            }
            Some(out) = out_rx.recv() => {
                sink.send(Message::Text(out.to_string())).await.map_err(|e| e.to_string())?;
            }
            event = events.recv() => {
                let frame = match event {
                    Ok(e) => match web_event(&e) {
                        Some(frame) => frame,
                        None => continue,
                    },
                    // 뒤처졌다: 브라우저가 목록을 다시 읽게 한다
                    Err(tokio::sync::broadcast::error::RecvError::Lagged(_)) => json!({ "t": "event", "kind": "resync", "data": {} }),
                    Err(tokio::sync::broadcast::error::RecvError::Closed) => continue,
                };
                sink.send(Message::Text(frame.to_string())).await.map_err(|e| e.to_string())?;
            }
            msg = stream.next() => {
                let text = match msg {
                    Some(Ok(Message::Text(t))) => t,
                    Some(Ok(Message::Close(_))) | None => {
                        let live = !sessions.lock().unwrap_or_else(|e| e.into_inner()).is_empty();
                        return Ok(if live { Ended::Dropped } else { Ended::Idle });
                    }
                    Some(Ok(_)) => continue,
                    Some(Err(e)) => return Err(e.to_string()),
                };
                let Ok(frame) = serde_json::from_str::<Value>(&text) else { continue };
                let sid = frame.get("sid").and_then(Value::as_str).unwrap_or_default().to_string();
                match frame.get("t").and_then(Value::as_str) {
                    Some("open") if !sid.is_empty() => {
                        let user = frame.get("user").and_then(Value::as_str).map(str::to_string);
                        sessions.lock().unwrap_or_else(|e| e.into_inner()).insert(sid, user);
                    }
                    Some("close") => {
                        let mut s = sessions.lock().unwrap_or_else(|e| e.into_inner());
                        s.remove(&sid);
                        if s.is_empty() {
                            idle.as_mut().reset(tokio::time::Instant::now() + IDLE);
                        }
                    }
                    Some("req") => {
                        // 열지 않은 세션의 요청은 받지 않는다 (릴레이가 세션을 연 뒤에만 보낸다)
                        let user = match sessions.lock().unwrap_or_else(|e| e.into_inner()).get(&sid) {
                            Some(user) => user.clone(),
                            None => continue,
                        };
                        dispatch(server, &frame, sid, user, &out_tx, &queue, &slots);
                    }
                    _ => {}
                }
            }
        }
    }
}

/// 요청 한 건을 대기열에 넣는다. 자리가 없으면 바로 바쁘다고 답한다
fn dispatch(
    server: &IPCServer,
    frame: &Value,
    sid: String,
    user: Option<String>,
    out: &mpsc::UnboundedSender<Value>,
    queue: &Arc<Semaphore>,
    slots: &Arc<Semaphore>,
) {
    let id = frame.get("id").cloned().unwrap_or(Value::Null);
    let Ok(place) = queue.clone().try_acquire_owned() else {
        let body = json!({ "ok": false, "success": false, "error": "Too many requests", "error_code": "remote_busy" });
        let _ = out.send(json!({ "t": "res", "sid": sid, "id": id, "status": 429, "body": body }));
        return;
    };
    let req = exec::Request {
        method: frame
            .get("method")
            .and_then(Value::as_str)
            .unwrap_or_default()
            .to_string(),
        path: frame
            .get("path")
            .and_then(Value::as_str)
            .unwrap_or_default()
            .to_string(),
        body: frame.get("body").cloned().filter(|b| !b.is_null()),
    };
    let (server, out, slots) = (server.clone(), out.clone(), slots.clone());
    tokio::spawn(async move {
        let _place = place;
        let Ok(_slot) = slots.acquire_owned().await else { return };
        let res = exec::run(&server, req, user).await;
        let _ = out.send(json!({ "t": "res", "sid": sid, "id": id, "status": res.status, "body": res.body }));
    });
}

#[cfg(test)]
mod tests {
    use super::*;
    use tokio_tungstenite::tungstenite::handshake::server::{Request as HsRequest, Response as HsResponse};

    async fn server(tmp: &std::path::Path) -> IPCServer {
        crate::ipc::auth::ensure_test_token();
        let mut sup = crate::supervisor::Supervisor::new_with_instances_dir(
            tmp.join("modules").to_str().unwrap(),
            tmp.join("instances").to_str().unwrap(),
        );
        sup.initialize().await.unwrap();
        IPCServer::new(
            std::sync::Arc::new(tokio::sync::RwLock::new(sup)),
            "127.0.0.1:0",
            crate::daemon_log::DaemonLogBuffer::new(),
        )
    }

    #[test]
    fn web_events_carry_only_the_kind_and_instance() {
        let event = |kind: &str, data: Value| crate::events::Event { id: 9, kind: kind.into(), data, replay: false };
        let exit = event(
            "instance.exit",
            json!({ "at": 1, "instanceId": "i1", "evidence": "console: C:/Users/me/crash.txt" }),
        );
        assert_eq!(
            web_event(&exit),
            Some(json!({ "t": "event", "id": 9, "kind": "instance.exit", "data": { "instanceId": "i1" } }))
        );
        assert!(web_event(&event("module.installed", json!({ "path": "C:/x" }))).is_some());
        // 화면이 쓰지 않는 이벤트는 보내지 않는다 (백업 오류 원문, 디스코드 연결 요청 등)
        assert_eq!(web_event(&event("backup.done", json!({ "error": "C:/x" }))), None);
        assert_eq!(web_event(&event("relay.link_request", json!({}))), None);
    }

    #[test]
    fn relay_urls_become_websocket_urls() {
        assert_eq!(ws_url("https://saba.example/"), "wss://saba.example/node/ws");
        assert_eq!(ws_url("http://127.0.0.1:9000"), "ws://127.0.0.1:9000/node/ws");
    }

    async fn next_frame<S>(stream: &mut S) -> Value
    where
        S: futures_util::Stream<Item = Result<Message, tokio_tungstenite::tungstenite::Error>> + Unpin,
    {
        loop {
            match tokio::time::timeout(Duration::from_secs(5), stream.next())
                .await
                .expect("frame in time")
            {
                Some(Ok(Message::Text(t))) => return serde_json::from_str(&t).unwrap(),
                Some(Ok(Message::Close(_))) | None => return json!({ "t": "closed" }),
                Some(Ok(_)) => continue,
                Some(Err(e)) => panic!("{e}"),
            }
        }
    }

    /// 모의 릴레이: 노드 토큰을 확인하고, 세션을 열어 요청 하나를 보내고, 이벤트를 받고, 닫으면 데몬이 끊는다
    #[tokio::test]
    async fn serves_a_web_session_through_the_relay() {
        let _one = TEST_LOCK.lock().await;
        let tmp = tempfile::tempdir().unwrap();
        let server = server(tmp.path()).await;
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let base = format!("http://{}", listener.local_addr().unwrap());
        let relay = tokio::spawn(async move {
            let (tcp, _) = listener.accept().await.unwrap();
            let mut auth = None;
            let ws = tokio_tungstenite::accept_hdr_async(tcp, |req: &HsRequest, res: HsResponse| {
                auth = req
                    .headers()
                    .get("Authorization")
                    .map(|v| v.to_str().unwrap().to_string());
                assert_eq!(req.uri().path(), "/node/ws");
                // 운영 nginx 는 데몬 경로에 X-Saba-Client 를 요구한다
                assert_eq!(
                    req.headers().get("X-Saba-Client").map(|v| v.as_bytes()),
                    Some(&b"1"[..])
                );
                Ok(res)
            })
            .await
            .unwrap();
            let (mut sink, mut stream) = ws.split();
            let hello = next_frame(&mut stream).await;
            sink.send(Message::Text(
                json!({ "t": "open", "sid": "s1", "user": "u1" }).to_string(),
            ))
            .await
            .unwrap();
            // 열지 않은 세션의 요청은 답하지 않는다
            sink.send(Message::Text(
                json!({ "t": "req", "sid": "nope", "id": 1, "method": "GET", "path": "/api/instances" }).to_string(),
            ))
            .await
            .unwrap();
            sink.send(Message::Text(
                json!({ "t": "req", "sid": "s1", "id": 2, "method": "GET", "path": "/api/config/node-token" })
                    .to_string(),
            ))
            .await
            .unwrap();
            sink.send(Message::Text(
                json!({ "t": "req", "sid": "s1", "id": 3, "method": "GET", "path": "/api/instances" }).to_string(),
            ))
            .await
            .unwrap();
            let mut answers = HashMap::new();
            let mut saw_event = false;
            while answers.len() < 2 || !saw_event {
                let frame = next_frame(&mut stream).await;
                match frame["t"].as_str() {
                    Some("res") => {
                        answers.insert(frame["id"].as_i64().unwrap(), frame);
                        if answers.len() == 2 {
                            crate::events::emit("instance.remote_test", json!({ "instanceId": "i1", "path": "C:/x" }));
                        }
                    }
                    Some("event") if frame["kind"] == "instance.remote_test" => {
                        assert_eq!(frame["data"], json!({ "instanceId": "i1" }), "no paths in web events");
                        saw_event = true;
                    }
                    _ => {}
                }
            }
            sink.send(Message::Text(json!({ "t": "close", "sid": "s1" }).to_string()))
                .await
                .unwrap();
            // 다른 시험이 낸 이벤트가 끼어들 수 있다 (버스는 프로세스에 하나)
            let after_close = loop {
                let frame = next_frame(&mut stream).await;
                if frame["t"] != "event" {
                    break frame;
                }
            };
            (auth, hello, answers, after_close)
        });

        ensure(server.clone(), &base, "sbn_host.secret");
        let (auth, hello, answers, after_close) = tokio::time::timeout(Duration::from_secs(10), relay)
            .await
            .unwrap()
            .unwrap();
        assert_eq!(auth.as_deref(), Some("Bearer sbn_host.secret"));
        assert_eq!(hello["t"], "hello");
        assert_eq!(hello["protocol"], PROTOCOL);
        assert!(!answers.contains_key(&1), "request for an unopened session is dropped");
        assert_eq!(answers[&2]["status"], 403);
        assert_eq!(answers[&2]["body"]["error_code"], "remote_not_allowed");
        assert_eq!(answers[&3]["status"], 200);
        assert_eq!(answers[&3]["sid"], "s1");
        assert_eq!(
            after_close["t"], "closed",
            "the daemon hangs up once no session is left"
        );
        stop();
    }

    /// 대기열이 차면 요청을 쌓지 않고 바로 바쁘다고 답한다
    #[tokio::test]
    async fn requests_beyond_the_queue_are_answered_busy() {
        let tmp = tempfile::tempdir().unwrap();
        let server = server(tmp.path()).await;
        let (out_tx, mut out_rx) = mpsc::unbounded_channel::<Value>();
        let queue = Arc::new(Semaphore::new(0));
        let slots = Arc::new(Semaphore::new(IN_FLIGHT));
        let frame = json!({ "t": "req", "sid": "s1", "id": 7, "method": "GET", "path": "/health" });
        dispatch(&server, &frame, "s1".into(), None, &out_tx, &queue, &slots);
        let res = tokio::time::timeout(Duration::from_secs(1), out_rx.recv())
            .await
            .unwrap()
            .unwrap();
        assert_eq!(res["id"], 7);
        assert_eq!(res["status"], 429);
        assert_eq!(res["body"]["error_code"], "remote_busy");
    }

    /// 다시 붙기를 기다리는 중에 멈추면 더 붙지 않는다
    #[tokio::test]
    async fn stop_during_backoff_does_not_reconnect() {
        let _one = TEST_LOCK.lock().await;
        let tmp = tempfile::tempdir().unwrap();
        let server = server(tmp.path()).await;
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let base = format!("http://{}", listener.local_addr().unwrap());
        let accepted = Arc::new(std::sync::atomic::AtomicUsize::new(0));
        let count = accepted.clone();
        tokio::spawn(async move {
            // 웹소켓이 아닌 상대: 받자마자 끊어서 연결 실패로 만든다
            while let Ok((tcp, _)) = listener.accept().await {
                count.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
                drop(tcp);
            }
        });
        ensure(server, &base, "sbn_host.secret");
        while accepted.load(std::sync::atomic::Ordering::SeqCst) == 0 {
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
        tokio::time::sleep(Duration::from_millis(100)).await;
        stop();
        tokio::time::sleep(Duration::from_millis(1500)).await;
        assert_eq!(accepted.load(std::sync::atomic::Ordering::SeqCst), 1);
    }
}
