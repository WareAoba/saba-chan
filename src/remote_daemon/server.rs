//! 원격 포트의 HTTP (D1). 로컬 IPC와 같은 라우트에, 인증층만 짝 맺은 PC 토큰으로 바꿔 씌운다.
//!
//!   POST /remote/pair {ticketId, secret}   토큰 없이 받는다. 릴레이에 연결표를 한 번 확인하고 그 PC 앞으로 토큰을 낸다
//!   그 밖                                  `Authorization: Bearer sbr_…`. 노드 토큰·원격 설정 라우트는 원격에서 막는다

use std::sync::{Arc, RwLock};
use std::time::{Duration, Instant};

use axum::{
    body::Body,
    extract::State,
    http::{header, Request, StatusCode},
    middleware::Next,
    response::{IntoResponse, Response},
    routing::post,
    Json, Router,
};
use saba_chan_updater_lib::config_file::ConfigFile;
use serde::Deserialize;
use serde_json::json;
use tokio_util::sync::CancellationToken;

use super::config::{self, Client};
use super::relay::{self, Failure, Target};

/// 원격에서 부르지 못하는 라우트: 이 PC의 릴레이 토큰, 개인정보 지우기, 원격 포트 설정과 짝 관리
const LOCAL_ONLY: &[&str] = &["/api/config/node-token", "/api/relay/privacy/erase", "/api/remote/"];
const HANDSHAKE_TIMEOUT: Duration = Duration::from_secs(10);
const MAX_CONNECTIONS: usize = 64;
/// 한 곳(IP)이 쥘 수 있는 연결 (인증 전에 잡으므로 한 곳이 짝 맺은 PC의 자리를 다 쥐지 못하게)
const MAX_PER_SOURCE: usize = 8;
/// 릴레이 터널(이 PC 안 루프백에서 붙는다) 몫은 따로 둔다
const MAX_TUNNELS: usize = 16;
/// 짝 맺기 시도는 1분에 한 곳(IP)에서 이만큼, 모두 합쳐 이만큼 (릴레이에도 그만큼만 묻는다)
const PAIR_PER_SOURCE: usize = 5;
const PAIR_PER_MINUTE: usize = 20;
const PAIR_BODY_LIMIT: usize = 1024;
/// 요청 머리를 이 안에 다 보내지 않으면 끊는다 (TLS만 맺고 가만히 있는 연결이 자리를 쥐지 못하게)
const HEADER_TIMEOUT: Duration = if cfg!(test) { Duration::from_secs(1) } else { Duration::from_secs(15) };

/// 연결한 곳 (요청 확장값, `serve`가 넣는다)
#[derive(Clone, Copy)]
struct Peer(std::net::IpAddr);

/// 릴레이 주소·노드 토큰은 부를 때마다 읽는다 (그동안 다시 연결했을 수 있다)
#[derive(Clone)]
pub(crate) enum RelaySource {
    Configured(crate::ipc::IPCServer),
    #[cfg(test)]
    Fixed(Target),
}

impl RelaySource {
    pub(crate) async fn target(&self) -> Option<Target> {
        match self {
            Self::Configured(server) => {
                let configured = server.config_store.get_bot_config().await.cloud.relay_url;
                let token = crate::ipc::handlers::relay::load_node_token()?;
                Some(Target { base: crate::ipc::handlers::relay::signed_relay_url(&configured, None), token })
            }
            #[cfg(test)]
            Self::Fixed(t) => Some(t.clone()),
        }
    }
}

#[derive(Clone)]
pub(crate) struct Ctx {
    pub file: ConfigFile,
    pub clients: Arc<RwLock<Vec<Client>>>,
    pub relay: RelaySource,
    pairs: Arc<std::sync::Mutex<Vec<(Instant, std::net::IpAddr)>>>,
}

impl Ctx {
    pub(crate) fn new(file: ConfigFile, clients: Arc<RwLock<Vec<Client>>>, relay: RelaySource) -> Self {
        Self { file, clients, relay, pairs: Arc::default() }
    }

    fn pair_allowed(&self, from: std::net::IpAddr) -> bool {
        let mut pairs = self.pairs.lock().unwrap_or_else(|e| e.into_inner());
        pairs.retain(|(t, _)| t.elapsed() < Duration::from_secs(60));
        if pairs.len() >= PAIR_PER_MINUTE || pairs.iter().filter(|(_, ip)| *ip == from).count() >= PAIR_PER_SOURCE {
            return false;
        }
        pairs.push((Instant::now(), from));
        true
    }
}

fn error(status: StatusCode, code: &str) -> Response {
    (status, Json(json!({ "error": code, "error_code": code }))).into_response()
}

pub(crate) fn router(routes: Router, ctx: Ctx) -> Router {
    routes
        .merge(Router::new().route("/remote/pair", post(pair)).with_state(ctx.clone()))
        .layer(axum::middleware::from_fn_with_state(ctx, remote_auth))
}

async fn remote_auth(State(ctx): State<Ctx>, mut req: Request<Body>, next: Next) -> Response {
    let path = req.uri().path();
    if path == "/remote/pair" {
        // 토큰 없이 받는 본문은 크기와 시간을 묶는다 (머리만 보내고 본문을 미루는 연결이 자리를 쥐지 못하게)
        let (parts, body) = req.into_parts();
        return match tokio::time::timeout(HEADER_TIMEOUT, axum::body::to_bytes(body, PAIR_BODY_LIMIT)).await {
            Ok(Ok(bytes)) => next.run(Request::from_parts(parts, Body::from(bytes))).await,
            Ok(Err(_)) => error(StatusCode::PAYLOAD_TOO_LARGE, "body_too_large"),
            Err(_) => error(StatusCode::REQUEST_TIMEOUT, "request_timeout"),
        };
    }
    if path == "/health" {
        return next.run(req).await;
    }
    // 노드 토큰, 그리고 고른 데몬과 상관없이 이 PC에서만 처리하는 경로(연결 관리, 이 PC의 인터페이스·앱 설정·업데이트,
    // 로그인 자동 실행, 데몬 종료: `target::stays_local`)는 짝 맺은 PC도 부르지 못한다
    if LOCAL_ONLY.iter().any(|p| path == p.trim_end_matches('/') || path.starts_with(p))
        || (path.starts_with("/api/") && super::target::stays_local(path))
    {
        return error(StatusCode::FORBIDDEN, "remote_local_only");
    }
    let token = req
        .headers()
        .get(header::AUTHORIZATION)
        .and_then(|v| v.to_str().ok())
        .and_then(|v| v.strip_prefix("Bearer "))
        .unwrap_or("")
        .trim();
    let known = config::verify(&ctx.clients.read().unwrap_or_else(|e| e.into_inner()), token);
    let Some(client) = known else {
        return error(StatusCode::UNAUTHORIZED, "remote_unauthorized");
    };
    let mut origin = crate::audit::Origin::from_headers(req.headers());
    origin.source = "remote".into();
    origin.user_name = Some(client.name);
    req.extensions_mut().insert(origin);
    next.run(req).await
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct PairBody {
    ticket_id: String,
    secret: String,
}

async fn pair(State(ctx): State<Ctx>, peer: Option<axum::Extension<Peer>>, Json(body): Json<PairBody>) -> Response {
    if !relay::valid_ticket_id(&body.ticket_id) || body.secret.is_empty() || body.secret.len() > 64 {
        return error(StatusCode::UNAUTHORIZED, "ticket_invalid");
    }
    let from = peer.map_or(std::net::IpAddr::from([0, 0, 0, 0]), |p| p.0 .0);
    if !ctx.pair_allowed(from) {
        return error(StatusCode::TOO_MANY_REQUESTS, "too_many_attempts");
    }
    let Some(target) = ctx.relay.target().await else {
        return error(StatusCode::SERVICE_UNAVAILABLE, "relay_not_paired");
    };
    let from = match relay::redeem(&target, &body.ticket_id, &body.secret).await {
        Ok(from) => from,
        Err(Failure::Rejected) => return error(StatusCode::UNAUTHORIZED, "ticket_invalid"),
        Err(Failure::Unreachable) => return error(StatusCode::BAD_GATEWAY, "relay_unreachable"),
    };
    let token = match config::issue_in(&ctx.file, &from.from_host_id, &from.from_host_name) {
        Ok(t) => t,
        Err(e) => {
            tracing::warn!("Failed to save a remote pairing: {}", e);
            return error(StatusCode::INTERNAL_SERVER_ERROR, "save_failed");
        }
    };
    *ctx.clients.write().unwrap_or_else(|e| e.into_inner()) = config::clients_in(&ctx.file);
    tracing::info!("Remote daemon paired with host {}", from.from_host_id);
    Json(json!({ "token": token })).into_response()
}

/// TLS로 받아 `app`에 넘긴다. `cancel`이면 새 연결을 받지 않고 열린 연결도 닫는다
pub(crate) async fn serve(
    listener: tokio::net::TcpListener,
    tls: Arc<rustls::ServerConfig>,
    app: Router,
    cancel: CancellationToken,
) {
    let acceptor = tokio_rustls::TlsAcceptor::from(tls);
    let slots = Arc::new(tokio::sync::Semaphore::new(MAX_CONNECTIONS));
    let tunnel_slots = Arc::new(tokio::sync::Semaphore::new(MAX_TUNNELS));
    let per_source = Sources::default();
    loop {
        let (tcp, peer) = tokio::select! {
            _ = cancel.cancelled() => return,
            accepted = listener.accept() => match accepted {
                Ok(accepted) => accepted,
                Err(e) => {
                    tracing::warn!("Remote daemon accept failed: {}", e);
                    tokio::time::sleep(Duration::from_millis(200)).await;
                    continue;
                }
            },
        };
        // 자리가 없으면 바로 끊는다 (핸드셰이크하지 않는 연결이 자리를 다 쥐지 못하게 핸드셰이크에도 시간 제한)
        let source = peer.ip().to_canonical();
        let pool = if source.is_loopback() { &tunnel_slots } else { &slots };
        let Ok(slot) = pool.clone().try_acquire_owned() else { continue };
        let Some(held) = per_source.take(source, if source.is_loopback() { MAX_TUNNELS } else { MAX_PER_SOURCE }) else {
            continue;
        };
        let (acceptor, app, cancel) = (acceptor.clone(), app.clone(), cancel.clone());
        tokio::spawn(async move {
            let _slot = (slot, held);
            let Ok(Ok(stream)) = tokio::time::timeout(HANDSHAKE_TIMEOUT, acceptor.accept(tcp)).await else { return };
            let ip = Peer(peer.ip().to_canonical());
            let app = tower::ServiceExt::map_request(app, move |mut req: Request<hyper::body::Incoming>| {
                req.extensions_mut().insert(ip);
                req
            });
            let service = hyper_util::service::TowerToHyperService::new(app);
            // 릴레이 터널(T1)은 이 PC 안에서 붙고 ALPN h2로 HTTP/2 연결 하나에 요청을 여러 개 싣는다.
            // 그 밖은 HTTP/1.1만: 요청 머리 시간 제한이 HTTP/2에는 없다
            let h2 = ip.0.is_loopback() && stream.get_ref().1.alpn_protocol() == Some(b"h2");
            let io = hyper_util::rt::TokioIo::new(stream);
            if h2 {
                let mut builder = hyper::server::conn::http2::Builder::new(hyper_util::rt::TokioExecutor::new());
                builder
                    .timer(hyper_util::rt::TokioTimer::new())
                    .keep_alive_interval(Duration::from_secs(25))
                    .keep_alive_timeout(Duration::from_secs(20));
                tokio::select! {
                    _ = builder.serve_connection(io, service) => {}
                    _ = cancel.cancelled() => {}
                }
            } else {
                let mut builder = hyper::server::conn::http1::Builder::new();
                builder.timer(hyper_util::rt::TokioTimer::new()).header_read_timeout(HEADER_TIMEOUT);
                tokio::select! {
                    _ = builder.serve_connection(io, service).with_upgrades() => {}
                    _ = cancel.cancelled() => {}
                }
            }
        });
    }
}

/// 곳(IP)마다 열린 연결 수
#[derive(Clone, Default)]
struct Sources(Arc<std::sync::Mutex<std::collections::HashMap<std::net::IpAddr, usize>>>);

/// 연결이 끝나면 그 곳의 수를 돌려놓는다
struct Held(Sources, std::net::IpAddr);

impl Sources {
    fn take(&self, ip: std::net::IpAddr, max: usize) -> Option<Held> {
        let mut map = self.0.lock().unwrap_or_else(|e| e.into_inner());
        let n = map.entry(ip).or_default();
        if *n >= max {
            return None;
        }
        *n += 1;
        Some(Held(self.clone(), ip))
    }
}

impl Drop for Held {
    fn drop(&mut self) {
        let mut map = self.0 .0.lock().unwrap_or_else(|e| e.into_inner());
        if let Some(n) = map.get_mut(&self.1) {
            *n -= 1;
            if *n == 0 {
                map.remove(&self.1);
            }
        }
    }
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;
    use crate::remote_daemon::tls::{pinned_client_config, Identity, SERVER_NAME};
    use http_body_util::BodyExt;
    use serde_json::Value;

    /// 지문을 고정해 원격 포트에 요청 하나를 보낸다
    pub(crate) async fn call(
        addr: std::net::SocketAddr,
        pin: &str,
        method: &str,
        path: &str,
        token: Option<&str>,
        body: Option<Value>,
    ) -> std::io::Result<(u16, Value)> {
        let tcp = tokio::net::TcpStream::connect(addr).await?;
        let connector = tokio_rustls::TlsConnector::from(Arc::new(pinned_client_config(pin).unwrap()));
        let tls = connector.connect(rustls::pki_types::ServerName::try_from(SERVER_NAME).unwrap(), tcp).await?;
        let (mut send, conn) = hyper::client::conn::http1::handshake(hyper_util::rt::TokioIo::new(tls))
            .await
            .map_err(std::io::Error::other)?;
        tokio::spawn(conn);
        let mut req = Request::builder().method(method).uri(path).header(header::HOST, "remote");
        if let Some(t) = token {
            req = req.header(header::AUTHORIZATION, format!("Bearer {t}"));
        }
        let req = match body {
            Some(b) => req.header(header::CONTENT_TYPE, "application/json").body(http_body_util::Full::new(bytes(b))),
            None => req.body(http_body_util::Full::new(hyper::body::Bytes::new())),
        }
        .unwrap();
        let res = send.send_request(req).await.map_err(std::io::Error::other)?;
        let status = res.status().as_u16();
        let raw = res.into_body().collect().await.map_err(std::io::Error::other)?.to_bytes();
        Ok((status, serde_json::from_slice(&raw).unwrap_or(Value::Null)))
    }

    fn bytes(v: Value) -> hyper::body::Bytes {
        hyper::body::Bytes::from(v.to_string())
    }

    /// 릴레이 흉내: 연결표 `good`/`s3cret`만 받는다
    async fn fake_relay() -> String {
        use axum::extract::Path;
        let app = Router::new().route(
            "/api/remote/tickets/{id}/redeem",
            post(|Path(id): Path<String>, Json(b): Json<Value>| async move {
                if id == "good" && b["secret"] == "s3cret" {
                    (StatusCode::OK, Json(json!({ "fromHostId": "host-a", "fromHostName": "집 PC" })))
                } else {
                    (StatusCode::GONE, Json(json!({ "error": "USED" })))
                }
            }),
        );
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let base = format!("http://{}", listener.local_addr().unwrap());
        tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });
        base
    }

    struct Remote {
        addr: std::net::SocketAddr,
        pin: String,
        _dir: tempfile::TempDir,
        cancel: CancellationToken,
    }

    async fn start(relay_base: Option<String>) -> Remote {
        let dir = tempfile::tempdir().unwrap();
        let id = Identity::load_or_create(dir.path()).unwrap();
        let file = ConfigFile::at(dir.path());
        let relay = RelaySource::Fixed(Target { base: relay_base.unwrap_or_else(|| "http://127.0.0.1:9".into()), token: "sbn_host-b.secret".into() });
        let ctx = Ctx::new(file, Arc::default(), relay);
        let routes = Router::new()
            .route("/health", axum::routing::get(|| async { Json(json!({ "ok": true })) }))
            .route("/api/instances", axum::routing::get(|| async { Json(json!([])) }))
            .route("/api/config/node-token", axum::routing::get(|| async { Json(json!({ "token": "leak" })) }))
            .route("/api/remote/clients", axum::routing::get(|| async { Json(json!([])) }));
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();
        let cancel = CancellationToken::new();
        tokio::spawn(serve(listener, Arc::new(id.server_config().unwrap()), router(routes, ctx), cancel.clone()));
        Remote { addr, pin: id.sha256.clone(), _dir: dir, cancel }
    }

    #[tokio::test]
    async fn a_pc_pairs_with_a_ticket_and_then_uses_the_api_with_its_token() {
        let r = start(Some(fake_relay().await)).await;
        let (s, _) = call(r.addr, &r.pin, "GET", "/api/instances", None, None).await.unwrap();
        assert_eq!(s, 401);
        let (s, body) =
            call(r.addr, &r.pin, "POST", "/remote/pair", None, Some(json!({ "ticketId": "good", "secret": "s3cret" }))).await.unwrap();
        assert_eq!(s, 200, "{body}");
        let token = body["token"].as_str().unwrap().to_string();
        let (s, body) = call(r.addr, &r.pin, "GET", "/api/instances", Some(&token), None).await.unwrap();
        assert_eq!((s, body), (200, json!([])));
        let (s, _) = call(r.addr, &r.pin, "GET", "/api/instances", Some("sbr_guess"), None).await.unwrap();
        assert_eq!(s, 401);
        r.cancel.cancel();
    }

    #[tokio::test]
    async fn a_rejected_ticket_gives_no_token() {
        let r = start(Some(fake_relay().await)).await;
        for body in [json!({ "ticketId": "good", "secret": "wrong" }), json!({ "ticketId": "../x", "secret": "s3cret" })] {
            let (s, res) = call(r.addr, &r.pin, "POST", "/remote/pair", None, Some(body)).await.unwrap();
            assert_eq!((s, res["error"].as_str()), (401, Some("ticket_invalid")));
            assert!(res.get("token").is_none());
        }
    }

    #[tokio::test]
    async fn an_unreachable_relay_is_not_a_pairing() {
        let r = start(None).await;
        let (s, res) =
            call(r.addr, &r.pin, "POST", "/remote/pair", None, Some(json!({ "ticketId": "good", "secret": "s3cret" }))).await.unwrap();
        assert_eq!((s, res["error"].as_str()), (502, Some("relay_unreachable")));
    }

    #[tokio::test]
    async fn local_only_routes_are_refused_even_with_a_token() {
        let r = start(Some(fake_relay().await)).await;
        let (_, body) =
            call(r.addr, &r.pin, "POST", "/remote/pair", None, Some(json!({ "ticketId": "good", "secret": "s3cret" }))).await.unwrap();
        let token = body["token"].as_str().unwrap().to_string();
        for path in [
            "/api/config/node-token",
            "/api/relay/privacy/erase",
            "/api/remote/clients",
            "/api/system/autostart",
            "/api/daemon/shutdown",
            "/api/updates/check",
            "/api/clients/launch",
            "/api/config/gui",
        ] {
            let (s, _) = call(r.addr, &r.pin, "GET", path, Some(&token), None).await.unwrap();
            assert_eq!(s, 403, "{path}");
        }
        let (s, _) = call(r.addr, &r.pin, "GET", "/health", None, None).await.unwrap();
        assert_eq!(s, 200);
    }

    #[test]
    fn one_source_cannot_hold_every_connection() {
        let sources = Sources::default();
        let a: std::net::IpAddr = "203.0.113.5".parse().unwrap();
        let b: std::net::IpAddr = "198.51.100.7".parse().unwrap();
        let held: Vec<_> = (0..MAX_PER_SOURCE).map(|_| sources.take(a, MAX_PER_SOURCE).unwrap()).collect();
        assert!(sources.take(a, MAX_PER_SOURCE).is_none());
        assert!(sources.take(b, MAX_PER_SOURCE).is_some(), "another source still connects");
        drop(held);
        assert!(sources.take(a, MAX_PER_SOURCE).is_some());
    }

    #[tokio::test]
    async fn pairing_attempts_are_limited() {
        let r = start(Some(fake_relay().await)).await;
        let mut codes = Vec::new();
        for _ in 0..PAIR_PER_SOURCE + 1 {
            let body = json!({ "ticketId": "bad", "secret": "x" });
            codes.push(call(r.addr, &r.pin, "POST", "/remote/pair", None, Some(body)).await.unwrap().0);
        }
        assert_eq!(codes.last(), Some(&429));
    }

    /// 실제 데몬 라우트에 씌워도 IPC 토큰·루프백 검사 없이 짝 맺은 PC 토큰으로 통하고, 노드 토큰은 막힌다
    #[tokio::test]
    async fn the_daemon_routes_answer_a_paired_pc() {
        let tmp = tempfile::tempdir().unwrap();
        let mut sup = crate::supervisor::Supervisor::new_with_instances_dir(
            tmp.path().join("modules").to_str().unwrap(),
            tmp.path().join("instances").to_str().unwrap(),
        );
        sup.initialize().await.unwrap();
        let server = crate::ipc::IPCServer::new(
            Arc::new(tokio::sync::RwLock::new(sup)),
            "127.0.0.1:0",
            crate::daemon_log::DaemonLogBuffer::new(),
        );
        let id = Identity::load_or_create(tmp.path()).unwrap();
        let file = ConfigFile::at(tmp.path());
        let token = config::issue_in(&file, "host-a", "집 PC").unwrap();
        let clients = Arc::new(RwLock::new(config::clients_in(&file)));
        let ctx = Ctx::new(file, clients, RelaySource::Fixed(Target { base: "http://127.0.0.1:9".into(), token: "sbn_b.s".into() }));
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();
        let cancel = CancellationToken::new();
        tokio::spawn(serve(listener, Arc::new(id.server_config().unwrap()), router(server.routes(), ctx), cancel.clone()));

        let (s, body) = call(addr, &id.sha256, "GET", "/api/instances", Some(&token), None).await.unwrap();
        assert_eq!(s, 200, "{body}");
        assert!(body.is_array());
        let (s, _) = call(addr, &id.sha256, "GET", "/api/config/node-token", Some(&token), None).await.unwrap();
        assert_eq!(s, 403);
        let (s, _) = call(addr, &id.sha256, "GET", "/api/instances", None, None).await.unwrap();
        assert_eq!(s, 401);
        cancel.cancel();
    }

    #[test]
    fn one_source_cannot_use_up_everyone_elses_pairing_attempts() {
        let ctx = Ctx::new(ConfigFile::at(std::path::Path::new(".")), Arc::default(), RelaySource::Fixed(Target { base: String::new(), token: String::new() }));
        let (a, b) = (std::net::IpAddr::from([198, 51, 100, 1]), std::net::IpAddr::from([198, 51, 100, 2]));
        for _ in 0..PAIR_PER_SOURCE {
            assert!(ctx.pair_allowed(a));
        }
        assert!(!ctx.pair_allowed(a));
        assert!(ctx.pair_allowed(b), "another source still pairs");
    }

    /// TLS만 맺고 아무것도 보내지 않는 연결은 머리 시간 제한 뒤 닫힌다 (자리를 돌려준다)
    #[tokio::test]
    async fn a_connection_that_sends_nothing_is_closed() {
        use tokio::io::AsyncReadExt;
        let r = start(None).await;
        let tcp = tokio::net::TcpStream::connect(r.addr).await.unwrap();
        let connector = tokio_rustls::TlsConnector::from(Arc::new(pinned_client_config(&r.pin).unwrap()));
        let mut tls = connector.connect(rustls::pki_types::ServerName::try_from(SERVER_NAME).unwrap(), tcp).await.unwrap();
        let mut buf = [0u8; 64];
        let read = tokio::time::timeout(HEADER_TIMEOUT * 3, tls.read(&mut buf)).await;
        assert!(matches!(read, Ok(Ok(0)) | Ok(Err(_))), "the idle connection must be closed, got {read:?}");
    }

    /// 머리만 보내고 본문을 보내지 않는 요청도 자리를 오래 쥐지 못한다 (토큰 없는 짝 맺기와 거절될 요청 모두)
    #[tokio::test]
    async fn a_request_that_never_sends_its_body_is_closed() {
        use tokio::io::{AsyncReadExt, AsyncWriteExt};
        let r = start(None).await;
        for path in ["/remote/pair", "/api/instances"] {
            let tcp = tokio::net::TcpStream::connect(r.addr).await.unwrap();
            let connector = tokio_rustls::TlsConnector::from(Arc::new(pinned_client_config(&r.pin).unwrap()));
            let mut tls = connector.connect(rustls::pki_types::ServerName::try_from(SERVER_NAME).unwrap(), tcp).await.unwrap();
            let head = format!("POST {path} HTTP/1.1\r\nHost: remote\r\nContent-Type: application/json\r\nContent-Length: 100\r\n\r\n");
            tls.write_all(head.as_bytes()).await.unwrap();
            tls.flush().await.unwrap();
            let closed = tokio::time::timeout(HEADER_TIMEOUT * 3, async {
                let mut buf = [0u8; 1024];
                loop {
                    match tls.read(&mut buf).await {
                        Ok(0) | Err(_) => return,
                        Ok(_) => {}
                    }
                }
            })
            .await;
            assert!(closed.is_ok(), "{path}: a request without its body must not hold the connection");
        }
    }

    #[tokio::test]
    async fn a_client_pinned_to_another_certificate_does_not_connect() {
        let r = start(None).await;
        assert!(call(r.addr, &"00".repeat(32), "GET", "/health", None, None).await.is_err());
    }
}
