//! 원격 포트의 HTTP (D1). 로컬 IPC와 같은 라우트에, 인증층만 짝 맺은 PC 토큰으로 바꿔 씌운다.
//!
//!   POST /remote/pair {ticketId, secret}   토큰 없이 받는다. 릴레이에 연결표를 한 번 확인하고 그 PC 앞으로 토큰을 낸다
//!   그 밖                                  `Authorization: Bearer sbr_…`. 노드 토큰·원격 설정 라우트는 원격에서 막는다

use std::collections::HashMap;
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
use serde_json::{json, Value};
use tokio_util::sync::CancellationToken;

use super::config::{self, Client};
use super::relay::{self, Failure, Target};

/// 원격에서 부르지 못하는 라우트: 이 PC의 릴레이 토큰, 개인정보 지우기, 원격 포트 설정과 짝 관리,
/// 이 PC의 아무 경로에 쓰거나 읽는 옮기기(설정 내보내기·들이기, 저장 폴더 옮기기)
const LOCAL_ONLY: &[&str] =
    &["/api/config/node-token", "/api/relay/privacy/erase", "/api/remote/", "/api/migration/", "/api/system/paths/move"];
/// PC 전용 칸을 보려고 받아 두는 본문 (axum `Json`의 기본 한도와 같다)
const FIELDS_BODY_LIMIT: usize = 2 * 1024 * 1024;
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

/// 릴레이 주소·노드 토큰은 부를 때마다 본다 (그동안 다시 연결했을 수 있다). 설정 파일은 바뀌었을 때만 다시 읽는다
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
                let token = crate::ipc::handlers::relay::peek_node_token()?;
                Some(Target { base: crate::ipc::handlers::relay::signed_relay_url(&configured, None), token })
            }
            #[cfg(test)]
            Self::Fixed(t) => Some(t.clone()),
        }
    }
}

/// 짝 맺은 PC 목록 (인증층이 요청마다 본다)과, 토큰마다 그 토큰으로 연 응답을 끝낼 표.
/// 목록에서 빠진 토큰(짝을 풀었거나 다시 맺어 바뀐 토큰)으로 열어 둔 스트림은 목록을 바꿀 때 끝낸다
#[derive(Clone, Default)]
pub(crate) struct Clients(Arc<RwLock<Paired>>);

#[derive(Default)]
struct Paired {
    list: Vec<Client>,
    /// 토큰 해시 → 그 토큰으로 연 응답
    open: HashMap<String, CancellationToken>,
}

impl Clients {
    pub(crate) fn set(&self, list: Vec<Client>) {
        let mut paired = self.0.write().unwrap_or_else(|e| e.into_inner());
        paired.open.retain(|hash, cancel| {
            let kept = list.iter().any(|c| &c.token_hash == hash);
            if !kept {
                cancel.cancel();
            }
            kept
        });
        paired.list = list;
    }

    /// 토큰의 주인 PC와, 짝이 풀리면 끝날 표지
    fn verify(&self, token: &str) -> Option<(Client, CancellationToken)> {
        let mut paired = self.0.write().unwrap_or_else(|e| e.into_inner());
        let client = config::verify(&paired.list, token)?;
        let ended = paired.open.entry(client.token_hash.clone()).or_default().clone();
        Some((client, ended))
    }
}

#[derive(Clone)]
pub(crate) struct Ctx {
    pub file: ConfigFile,
    pub clients: Clients,
    pub relay: RelaySource,
    /// PC 전용 칸 판정에 쓰는 이 데몬 (모듈의 PC 전용 설정, 저장된 값)
    server: Option<crate::ipc::IPCServer>,
    pairs: Arc<std::sync::Mutex<Vec<(Instant, std::net::IpAddr)>>>,
}

impl Ctx {
    pub(crate) fn new(file: ConfigFile, clients: Clients, relay: RelaySource) -> Self {
        Self { file, clients, relay, server: None, pairs: Arc::default() }
    }

    pub(crate) fn with_server(mut self, server: crate::ipc::IPCServer) -> Self {
        self.server = Some(server);
        self
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

/// 짝 맺은 PC가 부르지 못하는 경로면 상태와 오류 코드. 경로는 라우터가 푸는 대로 푼 값으로 본다.
/// 노드 토큰, 그리고 고른 데몬과 상관없이 이 PC에서만 처리하는 경로(연결 관리, 이 PC의 인터페이스·앱 설정·업데이트,
/// 로그인 자동 실행, 데몬 종료: `target::stays_local`)는 짝 맺은 PC도 부르지 못한다
pub(crate) fn refused_path(path: &str) -> Option<(StatusCode, &'static str)> {
    let Some(path) = crate::remote::gate::normalized_path(path) else {
        return Some((StatusCode::BAD_REQUEST, "remote_bad_path"));
    };
    let path = path.as_str();
    (LOCAL_ONLY.iter().any(|p| path == p.trim_end_matches('/') || path.starts_with(p))
        || (path.starts_with("/api/") && super::target::stays_local(path)))
    .then_some((StatusCode::FORBIDDEN, "remote_local_only"))
}

/// 짝 맺은 PC가 바꾸지 못하는 본문 칸. 웹 원격과 같은 PC 전용 칸 규칙(`gate::blocked_fields`)에서
/// 저장된 값과 같은 칸(바꾸지 않는 칸)은 뺀다: GUI는 설정 창의 칸을 바꾸지 않아도 함께 보낸다.
/// 플러그인 설치는 해시를 함께 줄 때만 막는다 (주소만 주면 데몬이 공식 목록의 주소인지 본다)
pub(crate) async fn blocked_fields(server: Option<&crate::ipc::IPCServer>, method: &str, path: &str, body: Option<&Value>) -> Vec<String> {
    let Ok(pattern) = crate::remote::gate::route_of(path) else { return Vec::new() };
    if !crate::remote::gate::checks_fields(method, pattern) {
        return Vec::new();
    }
    let pc_only = match server {
        Some(server) => crate::remote::exec::pc_only_fields(server, pattern, path, body).await,
        None => Vec::new(),
    };
    let mut blocked = crate::remote::gate::blocked_fields(method, pattern, body, &pc_only);
    if blocked.is_empty() {
        return blocked;
    }
    if pattern == "/api/extensions/{id}/install" {
        let sha = body.and_then(|b| b.get("sha256")).and_then(Value::as_str).is_some_and(|s| !s.is_empty());
        return if sha { blocked } else { Vec::new() };
    }
    let (Some(server), Some(body), "/api/instance/{id}" | "/api/instance/{id}/properties") = (server, body, pattern) else {
        return blocked;
    };
    // 라우터처럼 id를 풀어서 찾는다
    let id = path.split('?').next().and_then(|p| p.split('/').nth(3)).and_then(crate::remote::gate::percent_decode);
    let supervisor = server.supervisor.read().await;
    let Some(instance) = id.and_then(|id| supervisor.instance_store.get(&id)) else { return blocked };
    let view = serde_json::to_value(instance).unwrap_or_default();
    blocked.retain(|key| {
        let (sent, key) = match key.strip_prefix("settings.") {
            Some(k) => (body.get("settings").and_then(|s| s.get(k)), k),
            None => (body.get(key.as_str()), key.as_str()),
        };
        let Some(sent) = sent else { return true };
        // 익스텐션 값은 키마다 합친다
        if key == "extension_data" {
            return sent.as_object().is_none_or(|m| m.iter().any(|(k, v)| !same_value(v, instance.extension_data.get(k))));
        }
        !same_value(sent, crate::ipc::handlers::instance::stored_setting(&view, instance, key).as_ref())
    });
    blocked
}

/// 보낸 값이 저장된 값과 같은지. 비운 값(null·빈 글자·빈 목록)은 같고, 숫자·참거짓은 글자로 보낸 것과 같다
fn same_value(sent: &Value, stored: Option<&Value>) -> bool {
    fn norm(v: &Value) -> Value {
        match v {
            Value::String(s) if s.is_empty() => Value::Null,
            Value::Array(a) if a.is_empty() => Value::Null,
            Value::Number(n) => Value::String(n.to_string()),
            Value::Bool(b) => Value::String(b.to_string()),
            Value::Array(a) => Value::Array(a.iter().map(norm).collect()),
            Value::Object(o) => Value::Object(o.iter().map(|(k, v)| (k.clone(), norm(v))).collect()),
            v => v.clone(),
        }
    }
    norm(sent) == stored.map_or(Value::Null, norm)
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
    if let Some((status, code)) = refused_path(path) {
        return error(status, code);
    }
    let token = req
        .headers()
        .get(header::AUTHORIZATION)
        .and_then(|v| v.to_str().ok())
        .and_then(|v| v.strip_prefix("Bearer "))
        .unwrap_or("")
        .trim();
    let Some((client, revoked)) = ctx.clients.verify(token) else {
        return error(StatusCode::UNAUTHORIZED, "remote_unauthorized");
    };
    let mut origin = crate::audit::Origin::from_headers(req.headers());
    origin.source = "remote".into();
    origin.user = Some(client.host_id);
    origin.user_name = Some(client.name);
    req.extensions_mut().insert(origin);
    // 웹 원격과 같이 PC 전용 칸은 바꾸지 못한다. 하나라도 있으면 요청 전체를 거절한다 (일부만 쓰지 않게)
    let method = req.method().as_str().to_string();
    if crate::remote::gate::route_of(req.uri().path()).is_ok_and(|p| crate::remote::gate::checks_fields(&method, p)) {
        let (parts, body) = req.into_parts();
        let Ok(bytes) = axum::body::to_bytes(body, FIELDS_BODY_LIMIT).await else {
            return error(StatusCode::PAYLOAD_TOO_LARGE, "body_too_large");
        };
        let json = serde_json::from_slice::<Value>(&bytes).ok();
        let blocked = blocked_fields(ctx.server.as_ref(), &method, parts.uri.path(), json.as_ref()).await;
        if !blocked.is_empty() {
            return (StatusCode::FORBIDDEN, Json(crate::remote::exec::field_refusal(blocked))).into_response();
        }
        req = Request::from_parts(parts, Body::from(bytes));
    }
    // 짝이 풀리면 이 응답의 본문(이벤트 스트림, 내려받기)도 오류로 끊는다(받던 쪽이 다 받은 것으로 알지 않게). 새 요청은 위에서 바로 막힌다.
    // 길이를 아는 본문은 그대로 둔다
    let res = next.run(req).await;
    if axum::body::HttpBody::size_hint(res.body()).exact().is_some() {
        return res;
    }
    let (head, body) = res.into_parts();
    let cut = futures_util::stream::unfold((body.into_data_stream(), Some(revoked)), |(mut body, revoked)| async move {
        let revoked = revoked?;
        tokio::select! {
            biased;
            () = revoked.cancelled() => Some((Err(axum::Error::new(std::io::Error::other("remote client revoked"))), (body, None))),
            next = futures_util::StreamExt::next(&mut body) => next.map(|item| (item, (body, Some(revoked)))),
        }
    });
    Response::from_parts(head, Body::from_stream(cut))
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
    ctx.clients.set(config::clients_in(&ctx.file));
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
        file: ConfigFile,
        clients: Clients,
    }

    async fn start(relay_base: Option<String>) -> Remote {
        let dir = tempfile::tempdir().unwrap();
        let id = Identity::load_or_create(dir.path()).unwrap();
        let file = ConfigFile::at(dir.path());
        let relay = RelaySource::Fixed(Target { base: relay_base.unwrap_or_else(|| "http://127.0.0.1:9".into()), token: "sbn_host-b.secret".into() });
        let clients = Clients::default();
        let ctx = Ctx::new(file.clone(), clients.clone(), relay);
        let routes = Router::new()
            .route("/health", axum::routing::get(|| async { Json(json!({ "ok": true })) }))
            .route(
                "/api/events",
                axum::routing::get(|| async {
                    let first = futures_util::stream::once(async { Ok::<_, std::io::Error>(axum::body::Bytes::from("first\n")) });
                    Body::from_stream(futures_util::StreamExt::chain(first, futures_util::stream::pending()))
                }),
            )
            .route("/api/instances", axum::routing::get(|| async { Json(json!([])) }))
            .route("/api/config/node-token", axum::routing::get(|| async { Json(json!({ "token": "leak" })) }))
            .route("/api/remote/clients", axum::routing::get(|| async { Json(json!([])) }))
            .route("/api/config/{kind}/reset", post(|axum::extract::Path(kind): axum::extract::Path<String>| async move { Json(json!({ "reset": kind })) }));
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();
        let cancel = CancellationToken::new();
        tokio::spawn(serve(listener, Arc::new(id.server_config().unwrap()), router(routes, ctx), cancel.clone()));
        Remote { addr, pin: id.sha256.clone(), _dir: dir, cancel, file, clients }
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

    /// 짝을 풀면(다시 맺어 토큰이 바뀌어도) 그 토큰으로 열어 둔 이벤트 스트림도 끝난다. 다른 PC의 스트림은 그대로다
    #[tokio::test]
    async fn revoking_a_pc_ends_the_streams_it_has_open() {
        use tokio::io::{AsyncBufReadExt, AsyncWriteExt};
        let r = start(Some(fake_relay().await)).await;
        let open = |token: String| {
            let (addr, pin) = (r.addr, r.pin.clone());
            async move {
                let tcp = tokio::net::TcpStream::connect(addr).await.unwrap();
                let connector = tokio_rustls::TlsConnector::from(Arc::new(pinned_client_config(&pin).unwrap()));
                let mut tls = connector.connect(rustls::pki_types::ServerName::try_from(SERVER_NAME).unwrap(), tcp).await.unwrap();
                let head = format!("GET /api/events HTTP/1.1\r\nHost: remote\r\nAuthorization: Bearer {token}\r\n\r\n");
                tls.write_all(head.as_bytes()).await.unwrap();
                let mut lines = tokio::io::BufReader::new(tls).lines();
                while let Some(line) = lines.next_line().await.unwrap() {
                    if line.ends_with("first") {
                        return lines;
                    }
                }
                panic!("the stream did not start");
            }
        };
        let ended = |mut lines: tokio::io::Lines<tokio::io::BufReader<tokio_rustls::client::TlsStream<tokio::net::TcpStream>>>| async move {
            tokio::time::timeout(Duration::from_secs(3), async {
                loop {
                    match lines.next_line().await {
                        // 청크 끝(`0`)이나 연결 끊김
                        Ok(Some(l)) if l.trim() == "0" => return,
                        Ok(Some(_)) => {}
                        Ok(None) | Err(_) => return,
                    }
                }
            })
            .await
            .is_ok()
        };
        let a = config::issue_in(&r.file, "host-a", "집 PC").unwrap();
        let c = config::issue_in(&r.file, "host-c", "노트북").unwrap();
        r.clients.set(config::clients_in(&r.file));
        let (stream_a, stream_c) = (open(a.clone()).await, open(c).await);

        config::revoke_in(&r.file, "host-a").unwrap();
        r.clients.set(config::clients_in(&r.file));
        assert!(ended(stream_a).await, "the revoked PC's stream ends");
        assert!(!ended(stream_c).await, "another PC's stream stays open");
        let (s, _) = call(r.addr, &r.pin, "GET", "/api/instances", Some(&a), None).await.unwrap();
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

    /// 이 PC 전용 경로는 퍼센트 인코딩해도 같은 판정을 받는다 (라우터가 푸는 대로 풀어서 본다)
    #[tokio::test]
    async fn local_only_routes_are_judged_on_the_decoded_path() {
        let r = start(None).await;
        let token = config::issue_in(&r.file, "host-a", "집 PC").unwrap();
        r.clients.set(config::clients_in(&r.file));
        for path in ["/api/config/gui/reset", "/api/config/%67ui/reset", "/api/config/%6Eode-token", "/api/%72emote/clients"] {
            let (s, body) = call(r.addr, &r.pin, "POST", path, Some(&token), None).await.unwrap();
            assert_eq!((s, body["error_code"].as_str()), (403, Some("remote_local_only")), "{path}");
        }
        for path in ["/api/config/%2e%2e/reset", "/api/config/a%2Fb/reset", "/api/config/%FF/reset"] {
            let (s, body) = call(r.addr, &r.pin, "POST", path, Some(&token), None).await.unwrap();
            assert_eq!((s, body["error_code"].as_str()), (400, Some("remote_bad_path")), "{path}");
        }
        let (s, body) = call(r.addr, &r.pin, "POST", "/api/config/bot/reset", Some(&token), None).await.unwrap();
        assert_eq!((s, body), (200, json!({ "reset": "bot" })));
        r.cancel.cancel();
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
        let clients = Clients::default();
        clients.set(config::clients_in(&file));
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

    /// 짝 맺은 PC도 웹 원격과 같이 PC 전용 칸을 바꾸지 못한다. 바꾸지 않는 칸(저장된 값과 같은 칸)은 GUI가 함께 보내도 받는다.
    /// 바뀐 PC 전용 칸이 하나라도 있으면 요청 전체를 거절한다 (나머지 칸도 쓰지 않는다)
    #[tokio::test]
    async fn a_paired_pc_cannot_change_pc_only_values() {
        let tmp = tempfile::tempdir().unwrap();
        crate::ipc::auth::ensure_test_token();
        let mut sup = crate::supervisor::Supervisor::new_with_instances_dir(
            tmp.path().join("modules").to_str().unwrap(),
            tmp.path().join("instances").to_str().unwrap(),
        );
        sup.initialize().await.unwrap();
        let mut inst = crate::instance::ServerInstance::new("srv", "plain");
        inst.executable_path = Some("/srv/game/run".into());
        inst.port = Some(25565);
        let id = inst.id.clone();
        sup.instance_store.add(inst).unwrap();
        let server = crate::ipc::IPCServer::new(
            Arc::new(tokio::sync::RwLock::new(sup)),
            "127.0.0.1:0",
            crate::daemon_log::DaemonLogBuffer::new(),
        );
        let ident = Identity::load_or_create(tmp.path()).unwrap();
        let file = ConfigFile::at(tmp.path());
        let token = config::issue_in(&file, "host-a", "집 PC").unwrap();
        let clients = Clients::default();
        clients.set(config::clients_in(&file));
        let ctx = Ctx::new(file, clients, RelaySource::Fixed(Target { base: "http://127.0.0.1:9".into(), token: "sbn_b.s".into() }))
            .with_server(server.clone());
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();
        let cancel = CancellationToken::new();
        tokio::spawn(serve(listener, Arc::new(ident.server_config().unwrap()), router(server.routes(), ctx), cancel.clone()));
        let send = |method: &'static str, path: String, body: Option<Value>| {
            let (pin, token) = (ident.sha256.clone(), token.clone());
            async move { call(addr, &pin, method, &path, Some(&token), body).await.unwrap() }
        };
        let port = || async { server.supervisor.read().await.instance_store.get(&id).unwrap().port };

        let (s, body) = send("PATCH", format!("/api/instance/{id}"), Some(json!({ "port": 1234, "executable_path": "/bin/sh" }))).await;
        assert_eq!((s, body["error_code"].as_str()), (403, Some("remote_field_blocked")), "{body}");
        assert_eq!(body["fields"], json!(["executable_path"]));
        assert_eq!(port().await, Some(25565), "a refused request changes nothing");
        // 인코딩한 id로도 같은 판정
        let encoded: String = id.chars().map(|c| format!("%{:02X}", c as u32)).collect();
        let (s, body) = send("PATCH", format!("/api/instance/{encoded}"), Some(json!({ "executable_path": "/bin/sh" }))).await;
        assert_eq!((s, body["error_code"].as_str()), (403, Some("remote_field_blocked")), "{body}");
        // 저장된 값과 같은 PC 전용 칸은 받는다
        let (s, body) = send("PATCH", format!("/api/instance/{id}"), Some(json!({ "port": 1234, "executable_path": "/srv/game/run", "extra_args": [] }))).await;
        assert_ne!(body["error_code"].as_str(), Some("remote_field_blocked"), "{s} {body}");
        assert_eq!(port().await, Some(1234));

        let (s, body) = send("POST", "/api/extensions/x/install".into(), Some(json!({ "download_url": "https://h.example/x.zip", "sha256": "00" }))).await;
        assert_eq!((s, body["error_code"].as_str()), (403, Some("remote_field_blocked")), "{body}");
        // 주소만 주면 데몬이 카탈로그 주소인지 본다
        let (_, body) = send("POST", "/api/extensions/x/install".into(), Some(json!({ "download_url": "https://h.example/x.zip" }))).await;
        assert_ne!(body["error_code"].as_str(), Some("remote_field_blocked"), "{body}");
        let (s, body) = send("POST", "/api/instances".into(), Some(json!({ "name": "b", "module_name": "plain", "migration_source": "/home" }))).await;
        assert_eq!((s, body["fields"].clone()), (403, json!(["migration_source"])), "{body}");

        for path in ["/api/migration/export", "/api/migration/import", "/api/system/paths/move"] {
            let (s, body) = send("POST", path.into(), Some(json!({}))).await;
            assert_eq!((s, body["error_code"].as_str()), (403, Some("remote_local_only")), "{path}");
        }
        cancel.cancel();
    }

    #[test]
    fn one_source_cannot_use_up_everyone_elses_pairing_attempts() {
        let ctx = Ctx::new(ConfigFile::at(std::path::Path::new(".")), Clients::default(), RelaySource::Fixed(Target { base: String::new(), token: String::new() }));
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
