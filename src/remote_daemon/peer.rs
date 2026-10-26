//! 붙는 쪽 (D1b): 이 PC의 데몬이 같은 주인의 원격 데몬에 짝을 맺고, GUI·CLI의 요청을 그 데몬에 중계한다.
//!
//!   connect   릴레이에서 그 PC 앞 연결표를 받아, 받은 지문을 고정하고 TLS로 붙어 `/remote/pair`로 토큰을 받는다
//!   forward   저장한 원격 데몬에 요청 하나를 그대로 보낸다. 닿지 못하면 릴레이에서 새 주소·지문을 받아 한 번 더 해 본다
//!
//! 직접 붙지 못하거나 주소가 없는 PC(NAT 뒤)는 릴레이 터널로 붙는다 (T1, `tunnel.rs`)

use std::sync::Arc;
use std::time::Duration;

use futures_util::StreamExt;
use tokio_util::sync::CancellationToken;

use axum::{
    body::Body,
    http::{header, HeaderMap, HeaderName, Method, Request, StatusCode},
    response::{IntoResponse, Response},
    Json,
};
use saba_chan_updater_lib::config_file::ConfigFile;
use serde_json::{json, Value};

use super::config::{self, Daemon};
use super::relay::{self, Endpoint, Target, TicketFailure};
use super::{tls, tunnel};

const CONNECT_TIMEOUT: Duration = Duration::from_secs(10);
/// 원격 데몬이 답의 머리를 보낼 때까지. 정지는 인스턴스 `stop_timeout`(3600초까지)을 기다린 뒤 답하므로 CLI의 정지 대기와 같게 둔다.
/// 본문은 스트림이라 제한하지 않는다 (이벤트 스트림)
const RESPONSE_TIMEOUT: Duration = if cfg!(test) { Duration::from_secs(1) } else { Duration::from_secs(3600 + 600) };
/// 중계하는 요청 본문 상한 (다시 보내려고 모아 둔다)
const BODY_LIMIT: usize = 16 * 1024 * 1024;
/// 원격 데몬에 넘기는 요청 머리. 이 PC의 IPC 토큰·쿠키 같은 것은 넘기지 않는다
const FORWARDED: &[HeaderName] = &[header::CONTENT_TYPE, header::ACCEPT, header::ACCEPT_LANGUAGE];
const FORWARDED_NAMED: &[&str] = &["last-event-id", "x-saba-client"];

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ConnectError {
    /// 같은 주인의 다른 PC가 아니다
    NotFound,
    /// 그 PC가 원격 포트를 열고 주소를 올리지 않았다
    NoEndpoint,
    /// 이 PC가 릴레이에 연결돼 있지 않다
    RelayNotPaired,
    RelayUnreachable,
    /// 원격 데몬에 닿지 못했다 (주소·방화벽·지문)
    RemoteUnreachable,
    /// 원격 데몬이 연결표를 받지 않았다
    Rejected,
    SaveFailed,
}

impl ConnectError {
    pub fn code(self) -> &'static str {
        match self {
            Self::NotFound => "remote_not_found",
            Self::NoEndpoint => "remote_no_endpoint",
            Self::RelayNotPaired => "relay_not_paired",
            Self::RelayUnreachable => "relay_unreachable",
            Self::RemoteUnreachable => "remote_unreachable",
            Self::Rejected => "remote_rejected",
            Self::SaveFailed => "save_failed",
        }
    }

    pub fn status(self) -> StatusCode {
        match self {
            Self::NotFound => StatusCode::NOT_FOUND,
            Self::NoEndpoint => StatusCode::CONFLICT,
            Self::RelayNotPaired => StatusCode::SERVICE_UNAVAILABLE,
            Self::SaveFailed => StatusCode::INTERNAL_SERVER_ERROR,
            _ => StatusCode::BAD_GATEWAY,
        }
    }
}

pub fn error(status: StatusCode, code: &str) -> Response {
    (status, Json(json!({ "error": code, "error_code": code }))).into_response()
}

type Sender = hyper::client::conn::http1::SendRequest<Body>;

/// 지문을 고정해 원격 데몬에 붙는다
async fn open(endpoint: &Endpoint) -> Option<Sender> {
    let connect = async {
        let tcp = tokio::net::TcpStream::connect((endpoint.address.as_str(), endpoint.port)).await.ok()?;
        let tls_config = tls::pinned_client_config(&endpoint.cert_sha256).ok()?;
        let name = rustls::pki_types::ServerName::try_from(tls::SERVER_NAME).ok()?;
        let stream = tokio_rustls::TlsConnector::from(Arc::new(tls_config)).connect(name, tcp).await.ok()?;
        let (send, conn) = hyper::client::conn::http1::handshake(hyper_util::rt::TokioIo::new(stream)).await.ok()?;
        tokio::spawn(conn.with_upgrades());
        Some(send)
    };
    tokio::time::timeout(CONNECT_TIMEOUT, connect).await.ok().flatten()
}

enum Sent {
    Answered(hyper::Response<hyper::body::Incoming>),
    /// 붙지 못했다: 요청은 원격 데몬에 닿지 않았다 (다른 주소로 다시 보내도 된다)
    NotConnected,
    /// 붙은 뒤 답이 오지 않았다: 원격 데몬이 이미 처리하고 있을 수 있다 (다시 보내지 않는다)
    NoAnswer,
}

async fn send(endpoint: &Endpoint, req: Request<Body>) -> Sent {
    let Some(mut sender) = open(endpoint).await else { return Sent::NotConnected };
    match tokio::time::timeout(RESPONSE_TIMEOUT, sender.send_request(req)).await {
        Ok(Ok(res)) => Sent::Answered(res),
        _ => Sent::NoAnswer,
    }
}

async fn through_tunnel(relay: Option<&Target>, host_id: &str, endpoint: &Endpoint, mut req: Request<Body>) -> Sent {
    let Some(relay) = relay else { return Sent::NotConnected };
    tunnel::absolute(&mut req);
    // 열어 둔 터널이 그새 끊겼으면 요청은 나가지 않고 돌아온다: 새 터널로 한 번 더 보낸다
    for _ in 0..2 {
        let Some(mut sender) = tunnel::sender(relay, host_id, &endpoint.cert_sha256).await else { return Sent::NotConnected };
        if sender.ready().await.is_err() {
            continue;
        }
        match tokio::time::timeout(RESPONSE_TIMEOUT, sender.try_send_request(req)).await {
            Ok(Ok(res)) => return Sent::Answered(res),
            Ok(Err(mut e)) => match e.take_message() {
                Some(unsent) => req = unsent,
                None => return Sent::NoAnswer,
            },
            Err(_) => return Sent::NoAnswer,
        }
    }
    Sent::NotConnected
}

/// 직접 붙어 보고, 주소가 없거나 붙지 못하면 릴레이 터널로. 이 PC까지 터널이 열려 있으면 터널부터 쓴다
/// (NAT 뒤 PC에 직접 붙기를 요청마다 기다리지 않게). `request`는 보낼 때마다 새로 만든다
async fn send_to(relay: Option<&Target>, host_id: &str, endpoint: &Endpoint, request: &(dyn Fn() -> Option<Request<Body>> + Sync)) -> Sent {
    if !endpoint.address.is_empty() && !tunnel::is_open(relay, host_id) {
        let Some(req) = request() else { return Sent::NotConnected };
        match send(endpoint, req).await {
            Sent::NotConnected => {}
            sent => return sent,
        }
    }
    let Some(req) = request() else { return Sent::NotConnected };
    through_tunnel(relay, host_id, endpoint, req).await
}

/// `host_id` 원격 데몬과 짝을 맺고 저장한다
pub async fn connect(file: &ConfigFile, relay: Option<Target>, host_id: &str) -> Result<Daemon, ConnectError> {
    let relay = relay.ok_or(ConnectError::RelayNotPaired)?;
    let ticket = relay::ticket(&relay, host_id).await.map_err(|e| match e {
        TicketFailure::NotFound => ConnectError::NotFound,
        TicketFailure::NoEndpoint => ConnectError::NoEndpoint,
        TicketFailure::Unreachable => ConnectError::RelayUnreachable,
    })?;
    let body = json!({ "ticketId": ticket.ticket_id, "secret": ticket.secret }).to_string();
    let req = || {
        Request::post("/remote/pair")
            .header(header::HOST, tls::SERVER_NAME)
            .header(header::CONTENT_TYPE, "application/json")
            .body(Body::from(body.clone()))
            .ok()
    };
    let Sent::Answered(res) = send_to(Some(&relay), host_id, &ticket.endpoint, &req).await else {
        return Err(ConnectError::RemoteUnreachable);
    };
    if !res.status().is_success() {
        return Err(if res.status() == StatusCode::UNAUTHORIZED { ConnectError::Rejected } else { ConnectError::RemoteUnreachable });
    }
    let raw = http_body_util::BodyExt::collect(http_body_util::Limited::new(res.into_body(), 64 * 1024))
        .await
        .map_err(|_| ConnectError::RemoteUnreachable)?
        .to_bytes();
    let token = serde_json::from_slice::<Value>(&raw)
        .ok()
        .and_then(|v| v.get("token").and_then(Value::as_str).map(str::to_string))
        .filter(|t| t.starts_with("sbr_"))
        .ok_or(ConnectError::RemoteUnreachable)?;
    let daemon = Daemon {
        host_id: host_id.to_string(),
        name: ticket.name,
        address: ticket.endpoint.address,
        port: ticket.endpoint.port,
        cert_sha256: ticket.endpoint.cert_sha256,
        token,
        paired_at: chrono::Utc::now().to_rfc3339(),
    };
    config::save_daemon_in(file, daemon.clone()).map_err(|_| ConnectError::SaveFailed)?;
    tracing::info!("Paired with remote daemon {}", host_id);
    Ok(daemon)
}

fn endpoint_of(d: &Daemon) -> Endpoint {
    Endpoint { address: d.address.clone(), port: d.port, cert_sha256: d.cert_sha256.clone() }
}

fn build(method: &Method, path: &str, headers: &HeaderMap, token: &str, body: &axum::body::Bytes) -> Option<Request<Body>> {
    let mut req = Request::builder().method(method.clone()).uri(path).header(header::HOST, tls::SERVER_NAME);
    for name in FORWARDED.iter().cloned().chain(FORWARDED_NAMED.iter().map(|n| HeaderName::from_static(n))) {
        for value in headers.get_all(&name) {
            req = req.header(&name, value);
        }
    }
    req.header(header::AUTHORIZATION, format!("Bearer {token}")).body(Body::from(body.clone())).ok()
}

/// 원격 데몬에 넘길 경로: 데몬 API와 상태 확인만 (짝 맺기 같은 원격 포트 자체의 길은 넘기지 않는다)
pub fn forward_path(rest: &str, query: Option<&str>) -> Option<String> {
    let rest = rest.trim_start_matches('/');
    if !(rest == "health" || rest.starts_with("api/")) || rest.split('/').any(|s| s == ".." || s == ".") {
        return None;
    }
    Some(match query {
        Some(q) => format!("/{rest}?{q}"),
        None => format!("/{rest}"),
    })
}

/// 저장한 원격 데몬 `host_id`에 `path`(쿼리 포함, `/api/...`)를 보낸다.
/// `stop`(이 데몬 종료)이 취소되면 기다리던 답과 넘기던 스트림을 끝낸다.
pub async fn forward(file: &ConfigFile, relay: Option<Target>, host_id: &str, path: &str, req: Request<Body>, stop: CancellationToken) -> Response {
    tokio::select! {
        _ = stop.cancelled() => error(StatusCode::SERVICE_UNAVAILABLE, "shutting_down"),
        res = forward_once(file, relay, host_id, path, req) => {
            let (head, body) = res.into_parts();
            let body = body.into_data_stream().take_until(stop.cancelled_owned());
            Response::from_parts(head, Body::from_stream(body))
        }
    }
}

async fn forward_once(file: &ConfigFile, relay: Option<Target>, host_id: &str, path: &str, req: Request<Body>) -> Response {
    let Some(daemon) = config::daemons_in(file).into_iter().find(|d| d.host_id == host_id) else {
        return error(StatusCode::NOT_FOUND, "remote_unknown");
    };
    // 원격 데몬도 출처 `remote`로 남기지만, 이 PC에서 누가 시켰는지는 이 PC에만 있다. 읽기는 남기지 않는다
    let mut line = (!matches!(*req.method(), Method::GET | Method::HEAD | Method::OPTIONS) && !crate::audit::is_quiet())
        .then(|| AuditLine::new(req.method(), path, &daemon.name));
    let res = send_saved(file, relay, &daemon, path, req).await;
    if let Some(line) = line.as_mut() {
        line.status = res.status().as_u16();
    }
    res
}

/// 원격 데몬에 보낸 조작 한 줄 (이 PC 감사 로그). 쿼리·본문은 비밀을 가릴 모듈 정보가 원격에 있어 남기지 않는다.
/// 답을 받기 전에 끝나도(요청이 끊기거나 이 데몬이 멈춤) 이미 원격에 닿았을 수 있으므로 결과 모름(`status` 0)으로 남긴다
struct AuditLine {
    origin: crate::audit::Origin,
    daemon: String,
    command: String,
    status: u16,
}

impl AuditLine {
    fn new(method: &Method, path: &str, daemon: &str) -> Self {
        let command = format!("{method} {}", path.split('?').next().unwrap_or(path));
        Self { origin: crate::audit::current(), daemon: daemon.to_string(), command, status: 0 }
    }
}

impl Drop for AuditLine {
    fn drop(&mut self) {
        let event = crate::audit::Event {
            action: "remote",
            daemon: Some(&self.daemon),
            command: Some(&self.command),
            ok: (200..300).contains(&self.status),
            status: self.status,
            ..Default::default()
        };
        crate::audit::log().record(&self.origin, &event);
    }
}

async fn send_saved(file: &ConfigFile, relay: Option<Target>, daemon: &Daemon, path: &str, req: Request<Body>) -> Response {
    let host_id = daemon.host_id.as_str();
    let (parts, body) = req.into_parts();
    let Ok(body) = axum::body::to_bytes(body, BODY_LIMIT).await else {
        return error(StatusCode::PAYLOAD_TOO_LARGE, "body_too_large");
    };
    let request = || build(&parts.method, path, &parts.headers, &daemon.token, &body);
    if request().is_none() {
        return error(StatusCode::BAD_REQUEST, "bad_request");
    }
    let endpoint = endpoint_of(daemon);
    let mut res = send_to(relay.as_ref(), host_id, &endpoint, &request).await;
    if matches!(res, Sent::NotConnected) {
        // 주소가 바뀌었거나 원격 데몬이 인증서를 새로 만들었을 수 있다: 릴레이가 아는 지금 값으로 한 번 더
        if let Some(fresh) = fresh_endpoint(relay.as_ref(), host_id).await.filter(|e| *e != endpoint) {
            let _ = config::update_endpoint_in(file, host_id, &fresh.address, fresh.port, &fresh.cert_sha256);
            res = send_to(relay.as_ref(), host_id, &fresh, &request).await;
        }
    }
    let res = match res {
        Sent::Answered(res) => res,
        Sent::NotConnected => return error(StatusCode::BAD_GATEWAY, "remote_unreachable"),
        Sent::NoAnswer => return error(StatusCode::GATEWAY_TIMEOUT, "remote_timeout"),
    };
    let (mut head, incoming) = res.into_parts();
    for hop in [header::CONNECTION, header::TRANSFER_ENCODING, HeaderName::from_static("keep-alive")] {
        head.headers.remove(hop);
    }
    Response::from_parts(head, Body::new(incoming))
}

async fn fresh_endpoint(relay: Option<&Target>, host_id: &str) -> Option<Endpoint> {
    relay::peers(relay?).await.ok()?.into_iter().find(|p| p.host_id == host_id)?.endpoint
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::remote_daemon::server::{self, Ctx, RelaySource};
    use axum::{extract::State, routing::get, Router};
    use std::sync::Mutex;
    use tokio_util::sync::CancellationToken;

    #[derive(Clone, Default)]
    struct Relay {
        /// 원격 PC(host-b)가 올린 지금 주소
        endpoint: Arc<Mutex<Value>>,
        /// 터널 흉내: 원격 PC의 실제 원격 포트, 릴레이 주소, 받기를 기다리는 A 소켓, 연 터널 수
        port: Arc<Mutex<u16>>,
        base: Arc<Mutex<String>>,
        waiting: Arc<Mutex<std::collections::HashMap<String, axum::extract::ws::WebSocket>>>,
        tunnels: Arc<std::sync::atomic::AtomicUsize>,
    }

    /// 두 웹소켓의 프레임을 잇는다 (릴레이 `TunnelHub` 흉내)
    async fn splice(a: axum::extract::ws::WebSocket, b: axum::extract::ws::WebSocket) {
        let (mut a_tx, mut a_rx) = a.split();
        let (mut b_tx, mut b_rx) = b.split();
        let ab = async { while let Some(Ok(m)) = a_rx.next().await { if futures_util::SinkExt::send(&mut b_tx, m).await.is_err() { break } } };
        let ba = async { while let Some(Ok(m)) = b_rx.next().await { if futures_util::SinkExt::send(&mut a_tx, m).await.is_err() { break } } };
        tokio::select! { () = ab => {} () = ba => {} }
    }

    /// `/node/tunnel`: A가 `to=host-b`로 열면 B 에이전트가 명령을 받은 것처럼 `tunnel::accept`를 띄우고, B가 `accept=`로 붙으면 잇는다
    async fn tunnel_ws(
        State(r): State<Relay>,
        axum::extract::Query(q): axum::extract::Query<std::collections::HashMap<String, String>>,
        ws: axum::extract::ws::WebSocketUpgrade,
    ) -> Response {
        if let Some(id) = q.get("accept").cloned() {
            return ws.on_upgrade(move |b| async move {
                let a = r.waiting.lock().unwrap().remove(&id);
                if let Some(a) = a {
                    splice(a, b).await;
                }
            });
        }
        if q.get("to").map(String::as_str) != Some("host-b") {
            return StatusCode::NOT_FOUND.into_response();
        }
        ws.on_upgrade(move |a| async move {
            let id = format!("t{}", r.tunnels.fetch_add(1, std::sync::atomic::Ordering::SeqCst));
            r.waiting.lock().unwrap().insert(id.clone(), a);
            let b = Target { base: r.base.lock().unwrap().clone(), token: "sbn_host-b.secret".into() };
            let port = *r.port.lock().unwrap();
            tokio::spawn(tunnel::accept(b, id, port));
        })
    }

    /// 릴레이 흉내: host-b 앞 연결표·확인·PC 목록. host-off는 주소가 없고 그 밖은 남의 PC다
    async fn fake_relay(state: Relay) -> Target {
        let app = Router::new()
            .route(
                "/api/remote/tickets",
                axum::routing::post(|State(r): State<Relay>, Json(b): Json<Value>| async move {
                    match b["toHostId"].as_str() {
                        Some("host-b") => {
                            let mut t = r.endpoint.lock().unwrap().clone();
                            t["ticketId"] = json!("good");
                            t["secret"] = json!("s3cret");
                            t["expiresAt"] = json!(0);
                            t["name"] = json!("원격 PC");
                            (StatusCode::CREATED, Json(t))
                        }
                        Some("host-off") => (StatusCode::CONFLICT, Json(json!({ "error": "NO_ENDPOINT" }))),
                        _ => (StatusCode::NOT_FOUND, Json(json!({ "error": "NOT_FOUND" }))),
                    }
                }),
            )
            .route(
                "/api/remote/tickets/{id}/redeem",
                axum::routing::post(|axum::extract::Path(id): axum::extract::Path<String>, Json(b): Json<Value>| async move {
                    if id == "good" && b["secret"] == "s3cret" {
                        (StatusCode::OK, Json(json!({ "fromHostId": "host-a", "fromHostName": "집 PC" })))
                    } else {
                        (StatusCode::GONE, Json(json!({ "error": "USED" })))
                    }
                }),
            )
            .route(
                "/api/remote/peers",
                get(|State(r): State<Relay>| async move {
                    let endpoint = r.endpoint.lock().unwrap().clone();
                    Json(json!({ "peers": [{ "hostId": "host-b", "name": "원격 PC", "status": "online", "endpoint": endpoint }] }))
                }),
            )
            .route("/node/tunnel", get(tunnel_ws))
            .with_state(state.clone());
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let base = format!("http://{}", listener.local_addr().unwrap());
        *state.base.lock().unwrap() = base.clone();
        tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });
        Target { base, token: "sbn_host-a.secret".into() }
    }

    /// 원격 데몬(host-b): 받은 머리를 그대로 돌려주는 라우트와 첫 조각 뒤 멈추는 스트림
    /// `paired`가 있으면 그 설정 파일(짝 맺은 PC)을 쓰고 인증서만 새로 만든다 (같은 PC가 다시 뜬 것)
    async fn remote_daemon(relay: &Target, state: &Relay, paired: Option<&ConfigFile>) -> (tempfile::TempDir, CancellationToken) {
        let dir = tempfile::tempdir().unwrap();
        let id = tls::Identity::load_or_create(dir.path()).unwrap();
        let file = paired.cloned().unwrap_or_else(|| ConfigFile::at(dir.path()));
        let routes = Router::new()
            .route(
                "/api/echo",
                get(|headers: HeaderMap| async move {
                    let h = |n: &str| headers.get(n).and_then(|v| v.to_str().ok()).map(str::to_string);
                    Json(json!({ "lang": h("accept-language"), "ipcToken": h("x-saba-token"), "cookie": h("cookie") }))
                })
                .post(|body: String| async move { body }),
            )
            .route(
                "/api/slow",
                get(|| async {
                    tokio::time::sleep(RESPONSE_TIMEOUT * 3).await;
                    "late"
                })
                .post(|| async {
                    tokio::time::sleep(RESPONSE_TIMEOUT * 3).await;
                    "late"
                }),
            )
            .route(
                "/api/stream",
                get(|| async {
                    let chunks = futures_util::stream::once(async { Ok::<_, std::io::Error>(axum::body::Bytes::from("first\n")) })
                        .chain(futures_util::stream::once(async {
                            tokio::time::sleep(Duration::from_secs(30)).await;
                            Ok(axum::body::Bytes::from("late\n"))
                        }));
                    Body::from_stream(chunks)
                }),
            );
        let clients = server::Clients::default();
        clients.set(config::clients_in(&file));
        let ctx = Ctx::new(file, clients, RelaySource::Fixed(relay.clone()));
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let port = listener.local_addr().unwrap().port();
        *state.endpoint.lock().unwrap() = json!({ "address": "127.0.0.1", "port": port, "certSha256": id.sha256 });
        *state.port.lock().unwrap() = port;
        let cancel = CancellationToken::new();
        tokio::spawn(server::serve(listener, Arc::new(id.server_config().unwrap()), server::router(routes, ctx), cancel.clone()));
        (dir, cancel)
    }

    fn get_req(headers: &[(&str, &str)]) -> Request<Body> {
        let mut req = Request::get("/ignored");
        for (k, v) in headers {
            req = req.header(*k, *v);
        }
        req.body(Body::empty()).unwrap()
    }

    async fn json_of(res: Response) -> (u16, Value) {
        let status = res.status().as_u16();
        let raw = axum::body::to_bytes(res.into_body(), 1 << 20).await.unwrap();
        (status, serde_json::from_slice(&raw).unwrap_or(Value::Null))
    }

    use futures_util::StreamExt;

    #[tokio::test]
    async fn a_pc_pairs_with_its_remote_daemon_and_forwards_requests_without_its_local_secrets() {
        let state = Relay::default();
        let relay = fake_relay(state.clone()).await;
        let (_b, _stop) = remote_daemon(&relay, &state, None).await;
        let dir = tempfile::tempdir().unwrap();
        let file = ConfigFile::at(dir.path());

        let daemon = connect(&file, Some(relay.clone()), "host-b").await.unwrap();
        assert_eq!(daemon.name, "원격 PC");
        assert!(daemon.token.starts_with("sbr_"));
        assert_eq!(config::daemons_in(&file), vec![daemon]);

        let req = get_req(&[("accept-language", "ko"), ("x-saba-token", "local-ipc-secret"), ("cookie", "c=1")]);
        let (status, body) = json_of(forward(&file, Some(relay.clone()), "host-b", "/api/echo", req, CancellationToken::new()).await).await;
        assert_eq!(status, 200, "{body}");
        assert_eq!(body, json!({ "lang": "ko", "ipcToken": null, "cookie": null }));

        let post = Request::post("/ignored").header("content-type", "text/plain").body(Body::from("hello")).unwrap();
        let res = forward(&file, Some(relay.clone()), "host-b", "/api/echo", post, CancellationToken::new()).await;
        assert_eq!(axum::body::to_bytes(res.into_body(), 1024).await.unwrap(), "hello");

        let (status, body) = json_of(forward(&file, Some(relay), "host-x", "/api/echo", get_req(&[]), CancellationToken::new()).await).await;
        assert_eq!((status, body["error_code"].as_str()), (404, Some("remote_unknown")));
    }

    /// 원격 데몬에 보낸 조작(읽기가 아닌 요청)은 이 PC의 감사 로그에도 시킨 쪽·데몬·경로로 남는다. 쿼리와 본문은 남기지 않는다
    #[tokio::test]
    async fn a_forwarded_change_is_also_in_this_pcs_audit_log() {
        use crate::audit::{with_origin, Origin};
        let state = Relay::default();
        let relay = fake_relay(state.clone()).await;
        let (_b, _stop) = remote_daemon(&relay, &state, None).await;
        let dir = tempfile::tempdir().unwrap();
        let file = ConfigFile::at(dir.path());
        connect(&file, Some(relay.clone()), "host-b").await.unwrap();

        let marker = uuid::Uuid::new_v4().to_string();
        let origin = Origin { user_name: Some(marker.clone()), ..Origin::new("cli") };
        let origin_b = Origin { user_name: Some(format!("{marker}-cut")), ..Origin::new("gui") };
        let relay_b = relay.clone();
        let post = Request::post("/ignored").body(Body::from("secret-body")).unwrap();
        let forwarded = forward(&file, Some(relay.clone()), "host-b", "/api/echo?token=secret-query", post, CancellationToken::new());
        assert_eq!(with_origin(origin.clone(), forwarded).await.status(), StatusCode::OK);
        let read = forward(&file, Some(relay.clone()), "host-b", "/api/echo", get_req(&[]), CancellationToken::new());
        assert_eq!(with_origin(origin.clone(), read).await.status(), StatusCode::OK);
        let missing = forward(&file, Some(relay), "host-b", "/api/missing", Request::delete("/ignored").body(Body::empty()).unwrap(), CancellationToken::new());
        assert_eq!(with_origin(origin, missing).await.status(), StatusCode::NOT_FOUND);

        let mine: Vec<Value> = crate::audit::log().recent(1000, None).into_iter().filter(|e| e["userName"] == marker.as_str()).collect();
        assert_eq!(mine.len(), 2, "only the changes are recorded: {mine:?}");
        assert_eq!(mine[1]["source"], "cli");
        assert_eq!(mine[1]["action"], "remote");
        assert_eq!(mine[1]["daemon"], "원격 PC");
        assert_eq!(mine[1]["command"], "POST /api/echo");
        assert_eq!((mine[1]["ok"].as_bool(), mine[1]["status"].as_u64()), (Some(true), Some(200)));
        assert_eq!(mine[0]["command"], "DELETE /api/missing");
        assert_eq!((mine[0]["ok"].as_bool(), mine[0]["status"].as_u64()), (Some(false), Some(404)));
        assert!(!mine.iter().any(|e| e.to_string().contains("secret")), "{mine:?}");

        // 답을 기다리다 이 데몬이 멈춰도 원격에 닿았을 수 있으므로 결과 모름으로 남는다
        let stop = CancellationToken::new();
        let slow = forward(&file, Some(relay_b.clone()), "host-b", "/api/slow", Request::post("/ignored").body(Body::empty()).unwrap(), stop.clone());
        let (res, ()) = tokio::join!(with_origin(origin_b, slow), async {
            tokio::time::sleep(Duration::from_millis(300)).await;
            stop.cancel();
        });
        assert_eq!(res.status(), StatusCode::SERVICE_UNAVAILABLE);
        let cut: Vec<Value> = crate::audit::log().recent(1000, None).into_iter().filter(|e| e["userName"] == format!("{marker}-cut").as_str()).collect();
        assert_eq!(cut.len(), 1, "{cut:?}");
        assert_eq!((cut[0]["command"].as_str(), cut[0]["ok"].as_bool(), cut[0]["status"].as_u64()), (Some("POST /api/slow"), Some(false), Some(0)));
    }

    /// 공인 주소가 없는 PC(NAT 뒤)는 릴레이 터널로 짝을 맺고 요청을 보낸다. 터널 하나를 계속 쓴다
    #[tokio::test]
    async fn a_pc_without_a_public_address_is_reached_through_the_relay_tunnel() {
        let state = Relay::default();
        let relay = fake_relay(state.clone()).await;
        let (_b, _stop) = remote_daemon(&relay, &state, None).await;
        state.endpoint.lock().unwrap()["address"] = Value::Null;
        let dir = tempfile::tempdir().unwrap();
        let file = ConfigFile::at(dir.path());

        let daemon = connect(&file, Some(relay.clone()), "host-b").await.unwrap();
        assert_eq!(daemon.address, "");
        let req = get_req(&[("accept-language", "ko")]);
        let (status, body) = json_of(forward(&file, Some(relay.clone()), "host-b", "/api/echo", req, CancellationToken::new()).await).await;
        assert_eq!((status, body["lang"].as_str()), (200, Some("ko")), "{body}");
        let post = Request::post("/ignored").header("content-type", "text/plain").body(Body::from("hello")).unwrap();
        let res = forward(&file, Some(relay.clone()), "host-b", "/api/echo", post, CancellationToken::new()).await;
        assert_eq!(axum::body::to_bytes(res.into_body(), 1024).await.unwrap(), "hello");
        // 이벤트 스트림이 열려 있어도 다른 요청은 같은 터널로 간다
        let res = forward(&file, Some(relay.clone()), "host-b", "/api/stream", get_req(&[]), CancellationToken::new()).await;
        let mut stream = res.into_body().into_data_stream();
        assert_eq!(stream.next().await.unwrap().unwrap(), "first\n");
        let (status, _) = json_of(forward(&file, Some(relay.clone()), "host-b", "/api/echo", get_req(&[]), CancellationToken::new()).await).await;
        assert_eq!(status, 200);
        assert_eq!(state.tunnels.load(std::sync::atomic::Ordering::SeqCst), 1, "one tunnel carries every request");
        // 원격 데몬을 잊으면 열어 둔 터널을 놓는다 (릴레이의 터널 수 한도를 쥐고 있지 않게)
        assert!(tunnel::is_open(Some(&relay), "host-b"));
        tunnel::close("host-b");
        assert!(!tunnel::is_open(Some(&relay), "host-b"));
    }

    /// 주소가 있어도 직접 닿지 않으면(공유기가 포트를 넘기지 않음) 터널로 붙는다
    #[tokio::test]
    async fn an_unreachable_address_falls_back_to_the_tunnel() {
        let state = Relay::default();
        let relay = fake_relay(state.clone()).await;
        let (_b, _stop) = remote_daemon(&relay, &state, None).await;
        state.endpoint.lock().unwrap()["port"] = json!(9);
        let dir = tempfile::tempdir().unwrap();
        let file = ConfigFile::at(dir.path());
        connect(&file, Some(relay.clone()), "host-b").await.unwrap();
        let (status, body) = json_of(forward(&file, Some(relay), "host-b", "/api/echo", get_req(&[]), CancellationToken::new()).await).await;
        assert_eq!(status, 200, "{body}");
        assert_eq!(state.tunnels.load(std::sync::atomic::Ordering::SeqCst), 1);
    }

    #[tokio::test]
    async fn connecting_says_why_it_could_not() {
        let state = Relay::default();
        let relay = fake_relay(state.clone()).await;
        let dir = tempfile::tempdir().unwrap();
        let file = ConfigFile::at(dir.path());
        assert_eq!(connect(&file, None, "host-b").await, Err(ConnectError::RelayNotPaired));
        assert_eq!(connect(&file, Some(relay.clone()), "host-x").await, Err(ConnectError::NotFound));
        assert_eq!(connect(&file, Some(relay.clone()), "host-off").await, Err(ConnectError::NoEndpoint));
        // 릴레이가 알려 준 주소에 아무도 없다
        *state.endpoint.lock().unwrap() = json!({ "address": "127.0.0.1", "port": 9, "certSha256": "ab".repeat(32) });
        assert_eq!(connect(&file, Some(relay), "host-b").await, Err(ConnectError::RemoteUnreachable));
        assert!(config::daemons_in(&file).is_empty());
    }

    /// 원격 PC가 다른 포트·새 인증서로 다시 떴으면, 릴레이의 지금 값으로 다시 붙고 저장한 주소를 고친다
    #[tokio::test]
    async fn a_moved_remote_daemon_is_found_again_through_the_relay() {
        let state = Relay::default();
        let relay = fake_relay(state.clone()).await;
        let (b, stop) = remote_daemon(&relay, &state, None).await;
        let dir = tempfile::tempdir().unwrap();
        let file = ConfigFile::at(dir.path());
        let before = connect(&file, Some(relay.clone()), "host-b").await.unwrap();
        stop.cancel();

        let (_b2, _stop2) = remote_daemon(&relay, &state, Some(&ConfigFile::at(b.path()))).await;
        let (status, body) = json_of(forward(&file, Some(relay), "host-b", "/api/echo", get_req(&[]), CancellationToken::new()).await).await;
        assert_eq!(status, 200, "{body}");
        let after = config::daemons_in(&file).remove(0);
        assert_ne!((after.port, &after.cert_sha256), (before.port, &before.cert_sha256), "the saved endpoint follows the relay");
        assert_eq!(after.token, before.token);
    }

    /// 원격 데몬에 닿아 처리 중인 요청(정지 같은 긴 요청)은 답이 늦어도 다른 주소로 다시 보내지 않는다
    #[tokio::test]
    async fn a_slow_answer_is_not_sent_again_elsewhere() {
        let state = Relay::default();
        let relay = fake_relay(state.clone()).await;
        let (_b, _stop) = remote_daemon(&relay, &state, None).await;
        let dir = tempfile::tempdir().unwrap();
        let file = ConfigFile::at(dir.path());
        let before = connect(&file, Some(relay.clone()), "host-b").await.unwrap();
        // 릴레이가 다른 주소를 알게 됐다
        *state.endpoint.lock().unwrap() = json!({ "address": "127.0.0.1", "port": 9, "certSha256": "cd".repeat(32) });
        let (status, body) = json_of(forward(&file, Some(relay), "host-b", "/api/slow", get_req(&[]), CancellationToken::new()).await).await;
        assert_eq!((status, body["error_code"].as_str()), (504, Some("remote_timeout")));
        assert_eq!(config::daemons_in(&file).remove(0), before, "the saved endpoint stays");
    }

    /// 이벤트 스트림처럼 끝나지 않는 답도 오는 대로 넘긴다
    #[tokio::test]
    async fn a_streamed_answer_is_passed_on_as_it_comes() {
        let state = Relay::default();
        let relay = fake_relay(state.clone()).await;
        let (_b, _stop) = remote_daemon(&relay, &state, None).await;
        let dir = tempfile::tempdir().unwrap();
        let file = ConfigFile::at(dir.path());
        connect(&file, Some(relay.clone()), "host-b").await.unwrap();
        let res = forward(&file, Some(relay), "host-b", "/api/stream", get_req(&[]), CancellationToken::new()).await;
        assert_eq!(res.status(), StatusCode::OK);
        let mut stream = res.into_body().into_data_stream();
        let first = tokio::time::timeout(Duration::from_secs(5), stream.next()).await.expect("the first chunk arrives before the stream ends");
        assert_eq!(first.unwrap().unwrap(), "first\n");
    }

    #[tokio::test]
    async fn stopping_this_daemon_ends_a_relayed_stream_and_a_waiting_request() {
        let state = Relay::default();
        let relay = fake_relay(state.clone()).await;
        let (_b, _stop) = remote_daemon(&relay, &state, None).await;
        let dir = tempfile::tempdir().unwrap();
        let file = ConfigFile::at(dir.path());
        connect(&file, Some(relay.clone()), "host-b").await.unwrap();

        let stop = CancellationToken::new();
        let res = forward(&file, Some(relay.clone()), "host-b", "/api/stream", get_req(&[]), stop.clone()).await;
        let mut stream = res.into_body().into_data_stream();
        stream.next().await.unwrap().unwrap();
        stop.cancel();
        let end = tokio::time::timeout(Duration::from_secs(5), stream.next()).await.expect("the stream ends when this daemon stops");
        assert!(end.is_none());

        let stop = CancellationToken::new();
        let slow = tokio::spawn({
            let (file, relay, stop) = (file.clone(), relay.clone(), stop.clone());
            async move { forward(&file, Some(relay), "host-b", "/api/slow", get_req(&[]), stop).await }
        });
        tokio::time::sleep(Duration::from_millis(100)).await;
        stop.cancel();
        let res = tokio::time::timeout(Duration::from_secs(5), slow).await.expect("a waiting request gives up when this daemon stops").unwrap();
        assert_eq!(res.status(), StatusCode::SERVICE_UNAVAILABLE);
    }
}
