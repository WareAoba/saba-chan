//! 붙는 쪽 (D1b): 이 PC의 데몬이 같은 주인의 원격 데몬에 짝을 맺고, GUI·CLI의 요청을 그 데몬에 중계한다.
//!
//!   connect   릴레이에서 그 PC 앞 연결표를 받아, 받은 지문을 고정하고 TLS로 붙어 `/remote/pair`로 토큰을 받는다
//!   forward   저장한 원격 데몬에 요청 하나를 그대로 보낸다. 닿지 못하면 릴레이에서 새 주소·지문을 받아 한 번 더 해 본다

use std::sync::Arc;
use std::time::Duration;

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
use super::tls;

const CONNECT_TIMEOUT: Duration = Duration::from_secs(10);
/// 원격 데몬이 답의 머리를 보낼 때까지 (본문은 스트림이라 제한하지 않는다: 이벤트 스트림)
const RESPONSE_TIMEOUT: Duration = Duration::from_secs(120);
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

/// 요청 하나를 보낸다. 붙지 못했거나 답 머리가 오지 않으면 None
async fn send(endpoint: &Endpoint, req: Request<Body>) -> Option<hyper::Response<hyper::body::Incoming>> {
    let mut sender = open(endpoint).await?;
    tokio::time::timeout(RESPONSE_TIMEOUT, sender.send_request(req)).await.ok()?.ok()
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
    let req = Request::post("/remote/pair")
        .header(header::HOST, tls::SERVER_NAME)
        .header(header::CONTENT_TYPE, "application/json")
        .body(Body::from(body))
        .expect("pair request");
    let res = send(&ticket.endpoint, req).await.ok_or(ConnectError::RemoteUnreachable)?;
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

/// 저장한 원격 데몬 `host_id`에 `path`(쿼리 포함, `/api/...`)를 보낸다
pub async fn forward(file: &ConfigFile, relay: Option<Target>, host_id: &str, path: &str, req: Request<Body>) -> Response {
    let Some(daemon) = config::daemons_in(file).into_iter().find(|d| d.host_id == host_id) else {
        return error(StatusCode::NOT_FOUND, "remote_unknown");
    };
    let (parts, body) = req.into_parts();
    let Ok(body) = axum::body::to_bytes(body, BODY_LIMIT).await else {
        return error(StatusCode::PAYLOAD_TOO_LARGE, "body_too_large");
    };
    let request = || build(&parts.method, path, &parts.headers, &daemon.token, &body);
    let Some(first) = request() else {
        return error(StatusCode::BAD_REQUEST, "bad_request");
    };
    let mut endpoint = endpoint_of(&daemon);
    let mut res = send(&endpoint, first).await;
    if res.is_none() {
        // 주소가 바뀌었거나 원격 데몬이 인증서를 새로 만들었을 수 있다: 릴레이가 아는 지금 값으로 한 번 더
        if let Some(fresh) = fresh_endpoint(relay.as_ref(), host_id).await.filter(|e| *e != endpoint) {
            let _ = config::update_endpoint_in(file, host_id, &fresh.address, fresh.port, &fresh.cert_sha256);
            endpoint = fresh;
            if let Some(again) = request() {
                res = send(&endpoint, again).await;
            }
        }
    }
    let Some(res) = res else {
        return error(StatusCode::BAD_GATEWAY, "remote_unreachable");
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
                "/api/remote/tickets/:id/redeem",
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
            .with_state(state);
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let base = format!("http://{}", listener.local_addr().unwrap());
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
        let clients = Arc::new(std::sync::RwLock::new(config::clients_in(&file)));
        let ctx = Ctx::new(file, clients, RelaySource::Fixed(relay.clone()));
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let port = listener.local_addr().unwrap().port();
        *state.endpoint.lock().unwrap() = json!({ "address": "127.0.0.1", "port": port, "certSha256": id.sha256 });
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
        let (status, body) = json_of(forward(&file, Some(relay.clone()), "host-b", "/api/echo", req).await).await;
        assert_eq!(status, 200, "{body}");
        assert_eq!(body, json!({ "lang": "ko", "ipcToken": null, "cookie": null }));

        let post = Request::post("/ignored").header("content-type", "text/plain").body(Body::from("hello")).unwrap();
        let res = forward(&file, Some(relay.clone()), "host-b", "/api/echo", post).await;
        assert_eq!(axum::body::to_bytes(res.into_body(), 1024).await.unwrap(), "hello");

        let (status, body) = json_of(forward(&file, Some(relay), "host-x", "/api/echo", get_req(&[])).await).await;
        assert_eq!((status, body["error_code"].as_str()), (404, Some("remote_unknown")));
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
        let (status, body) = json_of(forward(&file, Some(relay), "host-b", "/api/echo", get_req(&[])).await).await;
        assert_eq!(status, 200, "{body}");
        let after = config::daemons_in(&file).remove(0);
        assert_ne!((after.port, &after.cert_sha256), (before.port, &before.cert_sha256), "the saved endpoint follows the relay");
        assert_eq!(after.token, before.token);
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
        let res = forward(&file, Some(relay), "host-b", "/api/stream", get_req(&[])).await;
        assert_eq!(res.status(), StatusCode::OK);
        let mut stream = res.into_body().into_data_stream();
        let first = tokio::time::timeout(Duration::from_secs(5), stream.next()).await.expect("the first chunk arrives before the stream ends");
        assert_eq!(first.unwrap().unwrap(), "first\n");
    }
}
