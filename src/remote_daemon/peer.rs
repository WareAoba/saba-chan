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
use super::{h2, tls, tunnel};

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

/// 직접 연결. 새 원격 데몬은 HTTP/2(ALPN `tls::DIRECT_H2`)로 받아 원격 PC마다 하나를 열어 두고 요청을 모두 싣는다(`DIRECT`).
/// 이전 릴리스는 HTTP/1.1이라 요청마다 연결 하나를 쓰고, 답을 마친 것을 잠깐 남겨 다시 쓴다(`IDLE`)
enum Conn {
    H1(H1Sender),
    H2(Direct),
}

/// HTTP/2 직접 연결과 그 위에 연 요청 수 (답 본문이 끝날 때까지). 원격 데몬은 연 요청이 없이 15초(`server::HEADER_TIMEOUT`)가
/// 지나면 연결을 닫는다: 그 닫힘이 닿지 않은 연결(원격 PC의 전원·망이 조용히 끊김)에 요청을 싣지 않게, 연 요청 없이
/// `REUSE_WINDOW`가 지난 연결은 쓰지 않는다
#[derive(Clone)]
struct Direct {
    sender: h2::Sender,
    opened: tokio::sync::watch::Sender<usize>,
}

impl Direct {
    fn new(sender: h2::Sender) -> Self {
        let (opened, streams) = tokio::sync::watch::channel(0);
        // 쉬는 시간이 지나면 받는 쪽을 놓고 끝난다: 그 뒤로 `opened.is_closed()`다
        tokio::spawn(h2::idle(streams, REUSE_WINDOW));
        Self { sender, opened }
    }

    /// 잡은 자리에 아직 열려 있고 쉰 지 오래되지 않은 연결
    fn live(held: &h2::Held<Direct>) -> Option<Direct> {
        held.as_ref().filter(|d| !d.sender.is_closed() && !d.opened.is_closed()).cloned()
    }

    /// 싣는다. 결과는 `send_h2`와 같다
    #[allow(clippy::result_large_err)] // 오류는 다시 보낼 요청을 그대로 돌려준다
    async fn send(self, req: Request<Body>) -> Result<Sent, Option<Request<Body>>> {
        let open = h2::Open::new(&self.opened);
        Ok(match send_h2(self.sender, req).await? {
            Sent::Answered(res) => Sent::Answered(res.map(|body| h2::counted(body, open))),
            other => other,
        })
    }
}

type H1Sender = hyper::client::conn::http1::SendRequest<Body>;

/// 직접 붙지 못한 까닭
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum NotOpened {
    /// 주소에 닿지 않거나 지문이 다르다
    Unreachable,
    /// 닿았지만 원격 데몬이 TLS를 맺기 전에 끊었다: 연결 자리가 없다 (`server::MAX_PER_SOURCE`). 주소는 맞다
    Full,
    /// 이번 요청은 터널로: 직접 연결이 `DIRECT_HEAD_START` 안에 열리지 않아 함께 연 터널이 먼저 열렸거나,
    /// 같은 PC의 다른 요청이 여는 연결을 기다리는 동안 그 요청이 직접 붙지 못했다
    Slow,
}

/// 직접 연결의 TCP keepalive: 이만큼 조용하면 살아 있는지 묻기 시작하고, 이 간격으로 다시 묻는다.
/// 긴 요청(정지 대기)의 답과 이벤트 스트림은 시간 제한 없이 기다리므로, 원격 PC의 망이 조용히 끊긴 것을 여기서 알아챈다
const KEEPALIVE_IDLE: Duration = Duration::from_secs(30);
const KEEPALIVE_INTERVAL: Duration = Duration::from_secs(5);

async fn dial(endpoint: &Endpoint) -> std::io::Result<tokio::net::TcpStream> {
    let tcp = tokio::net::TcpStream::connect((endpoint.address.as_str(), endpoint.port)).await?;
    socket2::SockRef::from(&tcp).set_tcp_keepalive(&socket2::TcpKeepalive::new().with_time(KEEPALIVE_IDLE).with_interval(KEEPALIVE_INTERVAL))?;
    Ok(tcp)
}

/// 지문을 고정해 원격 데몬에 붙는다
async fn open(endpoint: &Endpoint) -> Result<Conn, NotOpened> {
    use std::io::ErrorKind;
    let connect = async {
        let tcp = dial(endpoint).await.map_err(|_| NotOpened::Unreachable)?;
        let mut tls_config = tls::pinned_client_config(&endpoint.cert_sha256).map_err(|_| NotOpened::Unreachable)?;
        tls_config.alpn_protocols = vec![tls::DIRECT_H2.to_vec(), b"http/1.1".to_vec()];
        let name = rustls::pki_types::ServerName::try_from(tls::SERVER_NAME).map_err(|_| NotOpened::Unreachable)?;
        // 지문이 다르면 `InvalidData`, 상대가 그냥 끊으면 EOF·reset이다
        let stream = tokio_rustls::TlsConnector::from(Arc::new(tls_config)).connect(name, tcp).await.map_err(|e| match e.kind() {
            ErrorKind::UnexpectedEof | ErrorKind::ConnectionReset | ErrorKind::ConnectionAborted | ErrorKind::BrokenPipe => NotOpened::Full,
            _ => NotOpened::Unreachable,
        })?;
        let h2 = stream.get_ref().1.alpn_protocol() == Some(tls::DIRECT_H2);
        mark_http1(endpoint, !h2);
        if h2 {
            return h2::handshake(stream).await.map(|s| Conn::H2(Direct::new(s))).ok_or(NotOpened::Unreachable);
        }
        let (send, conn) = hyper::client::conn::http1::handshake(hyper_util::rt::TokioIo::new(stream)).await.map_err(|_| NotOpened::Unreachable)?;
        tokio::spawn(conn.with_upgrades());
        Ok(Conn::H1(send))
    };
    tokio::time::timeout(CONNECT_TIMEOUT, connect).await.unwrap_or(Err(NotOpened::Unreachable))
}

enum Sent {
    Answered(hyper::Response<Body>),
    /// 붙지 못했다: 요청은 원격 데몬에 닿지 않았다 (다른 주소로 다시 보내도 된다)
    NotConnected,
    /// 붙은 뒤 답이 오지 않았다: 원격 데몬이 이미 처리하고 있을 수 있다 (다시 보내지 않는다)
    NoAnswer,
}

/// 답을 마친 직접 연결을 다음 요청에 다시 쓰는 시간. 원격 데몬은 다음 요청 머리를 15초(`server::HEADER_TIMEOUT`)
/// 기다린 뒤 연결을 닫으므로, 그보다 넉넉히 짧게 두어 닫히는 연결에 요청을 싣지 않는다 (GUI 폴링 0.5~2초는 이어 쓴다)
const REUSE_WINDOW: Duration = if cfg!(test) { Duration::from_millis(500) } else { Duration::from_secs(8) };
/// 다시 쓴 연결에 보낸 읽기 요청이 답 머리를 기다리는 시간. 넘기면 끊긴 연결로 보고 새 연결로 다시 보낸다
const REUSED_ANSWER_TIMEOUT: Duration = if cfg!(test) { Duration::from_millis(300) } else { Duration::from_secs(10) };
/// 원격 데몬마다 남겨 두는 직접 연결 수 (HTTP/1.1은 한 연결에 한 요청씩이라 동시 요청만큼 연다)
const IDLE_PER_ENDPOINT: usize = 4;

type IdleKey = (String, u16, String);
/// 원격 데몬에 직접 붙은 HTTP/1.1 연결 중 다시 쓸 수 있는 것: (연결, 마지막 요청의 답을 받은 시각)
type Idle = std::collections::HashMap<IdleKey, Vec<(H1Sender, std::time::Instant)>>;
static IDLE: std::sync::LazyLock<std::sync::Mutex<Idle>> =
    std::sync::LazyLock::new(Default::default);
/// 원격 데몬(주소·포트·지문)마다 열어 둔 HTTP/2 직접 연결 하나
static DIRECT: h2::Connections<IdleKey, Direct> = h2::Connections::new();
/// 지난번에 HTTP/1.1로 답한 원격 데몬(이전 릴리스): 연결을 같이 쓸 수 없으므로 여는 동안 다른 요청을 기다리게 하지 않는다
static HTTP1: std::sync::LazyLock<std::sync::Mutex<std::collections::HashSet<IdleKey>>> = std::sync::LazyLock::new(Default::default);

fn mark_http1(endpoint: &Endpoint, http1: bool) {
    let mut earlier = HTTP1.lock().unwrap_or_else(|e| e.into_inner());
    if http1 {
        earlier.insert(idle_key(endpoint));
    } else {
        earlier.remove(&idle_key(endpoint));
    }
}

fn idle_key(endpoint: &Endpoint) -> IdleKey {
    (endpoint.address.clone(), endpoint.port, endpoint.cert_sha256.clone())
}

/// 비어 있고 아직 열려 있는, 최근에 쓴 연결 하나. 답 본문을 아직 받는 연결은 준비되지 않아 건너뛴다
fn take_idle(endpoint: &Endpoint) -> Option<H1Sender> {
    let mut idle = IDLE.lock().unwrap_or_else(|e| e.into_inner());
    let list = idle.get_mut(&idle_key(endpoint))?;
    list.retain(|(s, at)| !s.is_closed() && at.elapsed() < REUSE_WINDOW);
    let i = list.iter().position(|(s, _)| s.is_ready())?;
    Some(list.swap_remove(i).0)
}

fn keep_idle(endpoint: &Endpoint, sender: H1Sender) {
    let mut idle = IDLE.lock().unwrap_or_else(|e| e.into_inner());
    let list = idle.entry(idle_key(endpoint)).or_default();
    list.retain(|(s, at)| !s.is_closed() && at.elapsed() < REUSE_WINDOW);
    if list.len() < IDLE_PER_ENDPOINT {
        list.push((sender, std::time::Instant::now()));
    }
}

/// 뒤에서 연 직접 연결을 다음 요청이 쓰게 둔다
async fn keep(endpoint: &Endpoint, conn: Conn) {
    match conn {
        Conn::H1(sender) => keep_idle(endpoint, sender),
        Conn::H2(direct) => *DIRECT.hold(idle_key(endpoint)).await = Some(direct),
    }
}

/// 남겨 둔 연결로 보낸 결과
enum Reused {
    Sent(Sent),
    /// 남겨 둔 연결이 없거나 요청이 나가지 않았다: 이 요청을 새 연결로 보낸다
    Unsent(Request<Body>),
    /// 보내다 연결이 끊겼거나 답 머리가 `REUSED_ANSWER_TIMEOUT` 안에 오지 않았다: 요청을 다시 만들어 새 연결로 보낸다
    Lost,
}

/// 남겨 둔 연결로 보내 본다. 읽기 요청(GET·HEAD)만: 원격 데몬이 막 다시 켜져 연결이 끊긴 것을 보내고 나서야 알면
/// 원격 데몬이 받았는지 모르므로, 다시 보내도 되는 요청만 싣는다
async fn send_reused(endpoint: &Endpoint, req: Request<Body>) -> Reused {
    if !matches!(*req.method(), Method::GET | Method::HEAD) {
        return Reused::Unsent(req);
    }
    let Some(mut sender) = take_idle(endpoint) else { return Reused::Unsent(req) };
    // 그새 원격 PC의 망이 조용히 끊겼으면 OS가 포기할 때까지 기다리게 된다: 머리는 짧게만 기다리고 새 연결로 다시 보낸다
    match tokio::time::timeout(REUSED_ANSWER_TIMEOUT, sender.try_send_request(req)).await {
        Ok(Ok(res)) => {
            keep_idle(endpoint, sender);
            Reused::Sent(Sent::Answered(res.map(Body::new)))
        }
        Ok(Err(mut e)) => e.take_message().map_or(Reused::Lost, Reused::Unsent),
        Err(_) => Reused::Lost,
    }
}

/// `request`는 남겨 둔 연결이 끊겨 있었을 때 한 번 더 만든다. 직접 붙지 못했으면 그렇게 적고 `Sent::NotConnected`.
/// 원격 데몬이 자리가 없어 끊었거나 터널이 먼저 열렸으면 `Err`: 요청은 나가지 않았다
async fn send(relay: Option<&Target>, host_id: &str, endpoint: &Endpoint, request: &(dyn Fn() -> Option<Request<Body>> + Sync)) -> Result<Sent, NotOpened> {
    let Some(req) = request() else { return Ok(Sent::NotConnected) };
    let key = idle_key(endpoint);
    // 열어 둔 HTTP/2 연결이 있으면 싣는다. 그새 끊겼으면 놓고 새로 연다: 나가지 못한 요청은 그대로, 나간 뒤 끊긴 것은
    // 원격 데몬이 받았는지 모르므로 다시 보내도 되는 읽기(GET·HEAD)만 다시 만든다 (원격 데몬이 막 다시 켜진 것)
    let read = matches!(*req.method(), Method::GET | Method::HEAD);
    let shared = Direct::live(&DIRECT.hold(key.clone()).await);
    let req = match shared {
        Some(direct) => match direct.send(req).await {
            Ok(sent) => return Ok(sent),
            Err(unsent) => {
                *DIRECT.hold(key.clone()).await = None;
                match unsent.or_else(|| read.then(request).flatten()) {
                    Some(req) => req,
                    None => return Ok(Sent::NoAnswer),
                }
            }
        },
        None => req,
    };
    // 이전 릴리스(HTTP/1.1)면 방금 쓴 연결이 있을 때 TCP·TLS를 새로 열지 않는다
    let req = match send_reused(endpoint, req).await {
        Reused::Sent(sent) => return Ok(sent),
        Reused::Unsent(unsent) => unsent,
        Reused::Lost => match request() {
            Some(req) => req,
            None => return Ok(Sent::NotConnected),
        },
    };
    // 여는 동안 같은 PC의 다른 요청은 기다렸다가 HTTP/2 연결이면 같이 쓴다 (한꺼번에 온 요청이 연결을 하나씩 열지 않게)
    let held = DIRECT.hold(key.clone()).await;
    if let Some(direct) = Direct::live(&held) {
        drop(held);
        return Ok(sent_h2(direct.send(req).await));
    }
    if direct_failed().contains_key(&key) {
        return Err(NotOpened::Slow);
    }
    let earlier = HTTP1.lock().unwrap_or_else(|e| e.into_inner()).contains(&key);
    let mut held = (!earlier).then_some(held);
    let conn = match open_or_tunnel(relay, host_id, endpoint).await {
        Ok(conn) => conn,
        Err(NotOpened::Unreachable) => {
            direct_failed().insert(key, DirectFailed { at: std::time::Instant::now(), wait: DIRECT_RETRY, probing: false });
            return Ok(Sent::NotConnected);
        }
        Err(other) => return Err(other),
    };
    let mut sender = match conn {
        Conn::H2(direct) => {
            match held.as_mut() {
                Some(held) => **held = Some(direct.clone()),
                // 그새 새 릴리스로 바뀌었다
                None => *DIRECT.hold(key).await = Some(direct.clone()),
            }
            drop(held);
            return Ok(sent_h2(direct.send(req).await));
        }
        Conn::H1(sender) => sender,
    };
    drop(held);
    Ok(match tokio::time::timeout(RESPONSE_TIMEOUT, sender.send_request(req)).await {
        Ok(Ok(res)) => {
            keep_idle(endpoint, sender);
            Sent::Answered(res.map(Body::new))
        }
        _ => Sent::NoAnswer,
    })
}

/// HTTP/2 연결에 싣는다. 연결이 끊겼으면 `Err`: 요청이 나가지 못하고 돌아왔으면 그 요청(새 연결로 다시 보내도 된다),
/// 나간 뒤 끊겼으면 `None`
#[allow(clippy::result_large_err)] // 오류는 다시 보낼 요청을 그대로 돌려준다
async fn send_h2(mut sender: h2::Sender, mut req: Request<Body>) -> Result<Sent, Option<Request<Body>>> {
    h2::absolute(&mut req);
    if sender.ready().await.is_err() {
        return Err(Some(req));
    }
    match tokio::time::timeout(RESPONSE_TIMEOUT, sender.try_send_request(req)).await {
        Ok(Ok(res)) => Ok(Sent::Answered(res.map(Body::new))),
        Ok(Err(mut e)) => Err(e.take_message()),
        Err(_) => Ok(Sent::NoAnswer),
    }
}

/// 방금 연 연결에 보낸 결과
fn sent_h2(sent: Result<Sent, Option<Request<Body>>>) -> Sent {
    match sent {
        Ok(sent) => sent,
        Err(Some(_)) => Sent::NotConnected,
        Err(None) => Sent::NoAnswer,
    }
}

/// 직접 연결이 이만큼 안에 열리지 않으면 터널도 같이 열기 시작한다. 먼저 열린 쪽으로 보낸다
/// (말없이 버려지는 주소에 직접 연결이 포기할 때(`CONNECT_TIMEOUT`)까지 요청을 붙잡지 않게)
const DIRECT_HEAD_START: Duration = Duration::from_millis(1500);

/// 직접 연결을 먼저 열어 보고, `DIRECT_HEAD_START` 안에 안 되면 터널도 함께 연다.
/// 터널이 먼저 열리면 `Slow`: 직접 연결이 못 붙은 것으로 적고(다음 요청부터 터널로), 직접 연결은 뒤에서 마저 기다린다.
/// 열리면 남겨 두고 적은 것을 지운다 (다음 요청은 직접)
async fn open_or_tunnel(relay: Option<&Target>, host_id: &str, endpoint: &Endpoint) -> Result<Conn, NotOpened> {
    let ep = endpoint.clone();
    let mut direct = Box::pin(async move { open(&ep).await });
    if let Ok(first) = tokio::time::timeout(DIRECT_HEAD_START, &mut direct).await {
        return first;
    }
    let Some(relay) = relay else { return direct.await };
    let tunnel = tunnel::sender(relay, host_id, &endpoint.cert_sha256);
    tokio::pin!(tunnel);
    tokio::select! {
        d = &mut direct => d,
        t = &mut tunnel => {
            if t.is_none() {
                return direct.await;
            }
            direct_failed().insert(idle_key(endpoint), DirectFailed { at: std::time::Instant::now(), wait: DIRECT_RETRY, probing: true });
            let endpoint = endpoint.clone();
            tokio::spawn(async move {
                match direct.await {
                    Ok(conn) => {
                        keep(&endpoint, conn).await;
                        direct_failed().remove(&idle_key(&endpoint));
                    }
                    // 닿았지만 자리가 없었다: 주소는 맞다
                    Err(NotOpened::Full) => {
                        direct_failed().remove(&idle_key(&endpoint));
                    }
                    Err(_) => {
                        direct_failed().insert(idle_key(&endpoint), DirectFailed { at: std::time::Instant::now(), wait: DIRECT_RETRY, probing: false });
                    }
                }
            });
            Err(NotOpened::Slow)
        }
    }
}

async fn through_tunnel(relay: Option<&Target>, host_id: &str, endpoint: &Endpoint, mut req: Request<Body>) -> Sent {
    let Some(relay) = relay else { return Sent::NotConnected };
    // 열어 둔 터널이 그새 끊겼으면 요청은 나가지 않고 돌아온다: 새 터널로 한 번 더 보낸다
    for _ in 0..2 {
        let Some(sender) = tunnel::sender(relay, host_id, &endpoint.cert_sha256).await else { return Sent::NotConnected };
        match send_h2(sender, req).await {
            Ok(sent) => return sent,
            Err(Some(unsent)) => req = unsent,
            Err(None) => return Sent::NoAnswer,
        }
    }
    Sent::NotConnected
}

/// 직접 붙지 못한 주소를 다시 직접 붙어 보기까지 터널만 쓰는 시간. 그동안 요청은 기다리지 않고 터널로 가고,
/// 시간이 지나면 다음 요청이 뒤에서 한 번 직접 붙어 본다 (NAT 뒤 PC에 직접 붙기를 요청마다 기다리지 않게).
/// 다시 붙어 봐도 안 되면 `DIRECT_RETRY_MAX`까지 두 배씩 늘린다: 업데이트로 잠깐 내려간 원격 데몬은 금방 다시 직접 붙는다
const DIRECT_RETRY: Duration = if cfg!(test) { Duration::from_millis(200) } else { Duration::from_secs(15) };
const DIRECT_RETRY_MAX: Duration = if cfg!(test) { Duration::from_millis(800) } else { Duration::from_secs(300) };

/// 직접 붙지 못한 주소(주소·포트·지문)와 그때, 다시 붙어 보기까지 기다릴 시간. 뒤에서 다시 붙어 보는 중이면 `probing`
#[derive(Clone, Copy)]
struct DirectFailed {
    at: std::time::Instant,
    wait: Duration,
    probing: bool,
}
static DIRECT_FAILED: std::sync::LazyLock<std::sync::Mutex<std::collections::HashMap<IdleKey, DirectFailed>>> =
    std::sync::LazyLock::new(Default::default);

fn direct_failed() -> std::sync::MutexGuard<'static, std::collections::HashMap<IdleKey, DirectFailed>> {
    DIRECT_FAILED.lock().unwrap_or_else(|e| e.into_inner())
}

/// 원격 데몬이 자리가 없다며 끊은 것을 이만큼 잇달아 보면 닿지 못하는 주소로 적는다: 그 주소에서 원격 데몬이 아닌 것이
/// 받고 끊는 것일 수 있다 (포트 포워딩이 다른 곳을 가리킴). 적지 않으면 요청마다 직접 붙어 보고 PC 카드는 계속 `direct`다
const FULL_LIMIT: u32 = 3;
static FULL_STREAK: std::sync::LazyLock<std::sync::Mutex<std::collections::HashMap<IdleKey, u32>>> = std::sync::LazyLock::new(Default::default);

fn full_streak() -> std::sync::MutexGuard<'static, std::collections::HashMap<IdleKey, u32>> {
    FULL_STREAK.lock().unwrap_or_else(|e| e.into_inner())
}

/// 이 주소로 지금 직접 붙어 봐도 되는가. 얼마 전에 못 붙었으면 아니다. 그때로부터 기다릴 시간이 지났으면
/// 뒤에서 한 번 붙어 보고(되면 다음 요청부터 직접) 이번 요청은 터널로 보낸다
fn try_direct(endpoint: &Endpoint) -> bool {
    let key = idle_key(endpoint);
    let mut failed = direct_failed();
    let Some(f) = failed.get_mut(&key) else { return true };
    if !f.probing && f.at.elapsed() >= f.wait {
        f.probing = true;
        let wait = (f.wait * 2).min(DIRECT_RETRY_MAX);
        let endpoint = endpoint.clone();
        tokio::spawn(async move {
            let ok = match open(&endpoint).await {
                Ok(conn) => {
                    keep(&endpoint, conn).await;
                    true
                }
                Err(_) => false,
            };
            let mut failed = direct_failed();
            if ok {
                failed.remove(&idle_key(&endpoint));
            } else {
                failed.insert(idle_key(&endpoint), DirectFailed { at: std::time::Instant::now(), wait, probing: false });
            }
        });
    }
    false
}

/// 저장한 원격 데몬에 지금 붙는 길 (앱의 PC 카드): 주소가 있고 얼마 전 직접 붙기에 실패하지 않았으면 `direct`, 아니면 `relay`
pub fn route_of(d: &Daemon) -> &'static str {
    let endpoint = endpoint_of(d);
    if endpoint.address.is_empty() || direct_failed().contains_key(&idle_key(&endpoint)) { "relay" } else { "direct" }
}

/// 직접 붙어 보고, 주소가 없거나 붙지 못하면 릴레이 터널로 (사용자 결정 2026-10-25: 릴레이는 연결될 때까지만 거든다).
/// 직접 답을 받으면 열어 둔 터널을 놓는다 (그 위에서 받던 이벤트 스트림은 끝날 때까지 간다). 못 붙은 주소는 한동안 터널로만 보낸다(`try_direct`).
/// 원격 데몬이 자리가 없어 끊은 것은 그 요청만 터널로 보내고 주소는 그대로 믿는다 (잇달아 `FULL_LIMIT`번이면 못 붙은 주소로 적는다). `request`는 보낼 때마다 새로 만든다
async fn send_to(relay: Option<&Target>, host_id: &str, endpoint: &Endpoint, request: &(dyn Fn() -> Option<Request<Body>> + Sync)) -> Sent {
    if !endpoint.address.is_empty() && try_direct(endpoint) {
        match send(relay, host_id, endpoint, request).await {
            // 직접 붙지 못했다 (`send`가 적었다)
            Ok(Sent::NotConnected) => {}
            Ok(sent) => {
                if matches!(sent, Sent::Answered(_)) {
                    full_streak().remove(&idle_key(endpoint));
                    if let Some(relay) = relay {
                        tunnel::release(relay, host_id);
                    }
                }
                return sent;
            }
            // 터널이 먼저 열렸다: 이번 요청은 터널로 (직접 연결은 뒤에서 마저 열려 보고, 열리면 다음 요청부터 쓴다)
            Err(NotOpened::Slow) => {}
            Err(_) => {
                let mut streak = full_streak();
                let seen = streak.entry(idle_key(endpoint)).or_default();
                *seen += 1;
                if *seen >= FULL_LIMIT {
                    streak.remove(&idle_key(endpoint));
                    direct_failed().insert(idle_key(endpoint), DirectFailed { at: std::time::Instant::now(), wait: DIRECT_RETRY, probing: false });
                }
            }
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
        owner_name: ticket.owner_name,
        address: ticket.endpoint.address,
        port: ticket.endpoint.port,
        cert_sha256: ticket.endpoint.cert_sha256,
        token,
        paired_at: chrono::Utc::now().to_rfc3339(),
    };
    config::save_daemon_in(file, daemon.clone()).map_err(|_| ConnectError::SaveFailed)?;
    note_reached(&relay, host_id);
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

/// 원격 데몬에 넘길 경로: 데몬 API와 상태 확인만 (짝 맺기 같은 원격 포트 자체의 길은 넘기지 않는다).
/// 넘기는 경로는 풀지 않은 그대로지만 판정은 푼 경로로 한다
pub fn forward_path(rest: &str, query: Option<&str>) -> Option<String> {
    let rest = rest.trim_start_matches('/');
    let segments = crate::remote::gate::path_segments(&format!("/{rest}"))?;
    if !(segments == ["health"] || (segments.len() > 1 && segments[0] == "api")) {
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
    let Some(line) = line.as_mut() else { return res };
    line.status = res.status().as_u16();
    // 시작 실패처럼 200에 `success:false`(또는 `ok:false`)로 답하는 라우트가 있다: 길이를 아는 작은 본문만 열어 보고 그대로 넘긴다
    let small = axum::body::HttpBody::size_hint(res.body()).exact().is_some_and(|n| n <= DECLINE_PEEK_LIMIT as u64);
    if !res.status().is_success() || !small {
        return res;
    }
    let (head, body) = res.into_parts();
    let Ok(bytes) = axum::body::to_bytes(body, DECLINE_PEEK_LIMIT).await else {
        line.status = 0;
        return error(StatusCode::BAD_GATEWAY, "remote_unreachable");
    };
    if let Ok(v) = serde_json::from_slice::<Value>(&bytes) {
        line.declined = ["success", "ok"].iter().any(|k| v.get(*k).and_then(Value::as_bool) == Some(false));
    }
    Response::from_parts(head, Body::from(bytes))
}

/// 감사 기록에 실패 여부를 보려고 열어 보는 답 본문의 상한
const DECLINE_PEEK_LIMIT: usize = 1024 * 1024;

/// 원격 데몬에 보낸 조작 한 줄 (이 PC 감사 로그). 쿼리·본문은 비밀을 가릴 모듈 정보가 원격에 있어 남기지 않는다.
/// 답을 받기 전에 끝나도(요청이 끊기거나 이 데몬이 멈춤) 이미 원격에 닿았을 수 있으므로 결과 모름(`status` 0)으로 남긴다
struct AuditLine {
    origin: crate::audit::Origin,
    daemon: String,
    command: String,
    status: u16,
    /// 2xx이지만 본문이 실패라고 했다
    declined: bool,
}

impl AuditLine {
    fn new(method: &Method, path: &str, daemon: &str) -> Self {
        let command = format!("{method} {}", path.split('?').next().unwrap_or(path));
        Self { origin: crate::audit::current(), daemon: daemon.to_string(), command, status: 0, declined: false }
    }
}

impl Drop for AuditLine {
    fn drop(&mut self) {
        let event = crate::audit::Event {
            action: "remote",
            daemon: Some(&self.daemon),
            command: Some(&self.command),
            ok: (200..300).contains(&self.status) && !self.declined,
            status: self.status,
            ..Default::default()
        };
        crate::audit::log().record(&self.origin, &event);
    }
}

/// 닿지 않는 원격 PC에 쉬는 동안: 요청마다 붙어 보지 않고 바로 닿지 않음으로 답한다 (꺼진 PC에 GUI 폴링마다 터널을 열고
/// 경고를 남기지 않게). 릴레이가 꺼졌다고 하면 바로, 켜졌다는데 닿지 않으면 `UNREACHABLE_LIMIT`번 잇달아 실패한 뒤 쉰다.
/// 때가 지나면 다음 요청이 뒤에서 릴레이 PC 목록을 한 번 읽어, 꺼져 있으면 `OFFLINE_RECHECK` 더 쉬고 아니면 푼다.
/// 다시 연결(`connect`)하면 바로 푼다
const OFFLINE_RECHECK: Duration = if cfg!(test) { Duration::from_millis(200) } else { Duration::from_secs(60) };
const UNREACHABLE_LIMIT: u32 = 3;

#[derive(Default)]
struct Unreachable {
    /// 잇달아 닿지 않은 횟수 (닿으면 지운다)
    streak: u32,
    /// 쉬기 시작한 때와 쉴 시간. 뒤에서 릴레이에 묻는 중이면 `checking`
    paused: Option<(std::time::Instant, Duration)>,
    checking: bool,
}

/// 릴레이·원격 PC마다 (같은 PC도 릴레이가 다르면 따로)
static UNREACHABLE: std::sync::LazyLock<std::sync::Mutex<std::collections::HashMap<(String, String), Unreachable>>> =
    std::sync::LazyLock::new(Default::default);

fn unreachable() -> std::sync::MutexGuard<'static, std::collections::HashMap<(String, String), Unreachable>> {
    UNREACHABLE.lock().unwrap_or_else(|e| e.into_inner())
}

/// 쉬는 중인가. 쉴 시간이 지났으면 뒤에서 릴레이에 그 PC가 켜졌는지 묻는다 (이번 요청은 기다리지 않는다)
fn resting(relay: &Target, host_id: &str) -> bool {
    let key = (relay.base.clone(), host_id.to_string());
    let mut all = unreachable();
    let Some(u) = all.get_mut(&key) else { return false };
    let Some((at, wait)) = u.paused else { return false };
    if !u.checking && at.elapsed() >= wait {
        u.checking = true;
        let relay = relay.clone();
        tokio::spawn(async move {
            let offline = matches!(relay::peers(&relay).await, Ok(peers) if peers.iter().any(|p| p.host_id == key.1 && p.status == "offline"));
            let mut all = unreachable();
            let Some(u) = all.get_mut(&key) else { return };
            u.checking = false;
            if offline {
                u.paused = Some((std::time::Instant::now(), OFFLINE_RECHECK));
            } else {
                // 다음 요청이 다시 붙어 본다. 또 닿지 않으면 잇달은 횟수가 이어져 더 오래 쉰다
                u.paused = None;
                tracing::info!("Trying remote daemon {} again", key.1);
            }
        });
    }
    true
}

/// 닿지 않았다. 릴레이가 꺼졌다고 했으면 바로, 아니면 잇달아 `UNREACHABLE_LIMIT`번째부터 쉰다 (`DIRECT_RETRY`부터 두 배씩)
fn note_unreachable(relay: &Target, host_id: &str, offline: bool) {
    let mut all = unreachable();
    let u = all.entry((relay.base.clone(), host_id.to_string())).or_default();
    u.streak += 1;
    let wait = if offline {
        OFFLINE_RECHECK
    } else if u.streak >= UNREACHABLE_LIMIT {
        DIRECT_RETRY.saturating_mul(1 << (u.streak - UNREACHABLE_LIMIT).min(8)).min(DIRECT_RETRY_MAX)
    } else {
        return;
    };
    u.paused = Some((std::time::Instant::now(), wait));
    if offline {
        tracing::warn!("Remote daemon {} is offline; not connecting until it is back", host_id);
    } else {
        tracing::warn!("Remote daemon {} is unreachable {} times in a row; retrying in {:?}", host_id, u.streak, wait);
    }
}

fn note_reached(relay: &Target, host_id: &str) {
    unreachable().remove(&(relay.base.clone(), host_id.to_string()));
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
    if relay.as_ref().is_some_and(|r| resting(r, host_id)) {
        return error(StatusCode::BAD_GATEWAY, "remote_unreachable");
    }
    let endpoint = endpoint_of(daemon);
    let mut res = send_to(relay.as_ref(), host_id, &endpoint, &request).await;
    if let (Sent::NotConnected, Some(relay)) = (&res, relay.as_ref()) {
        // 주소가 바뀌었거나 원격 데몬이 인증서를 새로 만들었을 수 있다: 릴레이가 아는 지금 값으로 한 번 더
        let peer = relay::peers(relay).await.ok().and_then(|peers| peers.into_iter().find(|p| p.host_id == host_id));
        let offline = peer.as_ref().is_some_and(|p| p.status == "offline");
        if let Some(fresh) = peer.and_then(|p| p.endpoint).filter(|e| !offline && *e != endpoint) {
            let _ = config::update_endpoint_in(file, host_id, &fresh.address, fresh.port, &fresh.cert_sha256);
            res = send_to(Some(relay), host_id, &fresh, &request).await;
        }
        if matches!(res, Sent::NotConnected) {
            note_unreachable(relay, host_id, offline);
        }
    }
    if let (Sent::Answered(_), Some(relay)) = (&res, relay.as_ref()) {
        note_reached(relay, host_id);
    }
    let res = match res {
        Sent::Answered(res) => res,
        Sent::NotConnected => return error(StatusCode::BAD_GATEWAY, "remote_unreachable"),
        Sent::NoAnswer => return error(StatusCode::GATEWAY_TIMEOUT, "remote_timeout"),
    };
    let (mut head, body) = res.into_parts();
    for hop in [header::CONNECTION, header::TRANSFER_ENCODING, HeaderName::from_static("keep-alive")] {
        head.headers.remove(hop);
    }
    Response::from_parts(head, body)
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
        /// `proxy` 앞의 연결을 끊는 표시: 취소하면 그때 이어 둔 연결이 모두 끊긴다 (`/api/cut-once`)
        cut: Arc<Mutex<CancellationToken>>,
        /// 켜 두면 끊긴 연결을 닫지 않고 말없이 버린다 (원격 PC의 전원·망이 조용히 끊긴 것)
        silent: Arc<std::sync::atomic::AtomicBool>,
        /// PC 목록이 알리는 host-b 상태 (비면 online). online이 아니거나 `refuse`면 터널을 열자마자 닫는다 (릴레이가 명령을 넘기지 못함)
        status: Arc<Mutex<String>>,
        refuse: Arc<std::sync::atomic::AtomicBool>,
    }

    impl Relay {
        fn status(&self) -> String {
            let s = self.status.lock().unwrap().clone();
            if s.is_empty() { "online".into() } else { s }
        }
    }

    /// 원격 데몬 앞의 TCP 중계. 릴레이가 알리는 주소를 이것으로 바꾼다 (터널은 그대로 원격 데몬에 붙는다).
    /// 새 연결은 `delay` 뒤에 잇고, `state.cut`이 취소되면 그때 이어 둔 연결을 끊는다
    async fn proxy(state: &Relay, delay: Duration) {
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let upstream = *state.port.lock().unwrap();
        state.endpoint.lock().unwrap()["port"] = json!(listener.local_addr().unwrap().port());
        let (cut, silent) = (state.cut.clone(), state.silent.clone());
        tokio::spawn(async move {
            while let Ok((mut near, _)) = listener.accept().await {
                let (cut, silent) = (cut.lock().unwrap().clone(), silent.clone());
                tokio::spawn(async move {
                    tokio::time::sleep(delay).await;
                    let Ok(mut far) = tokio::net::TcpStream::connect(("127.0.0.1", upstream)).await else { return };
                    tokio::select! {
                        _ = tokio::io::copy_bidirectional(&mut near, &mut far) => {}
                        () = cut.cancelled() => {
                            if silent.load(std::sync::atomic::Ordering::SeqCst) {
                                std::future::pending::<()>().await;
                            }
                        }
                    }
                });
            }
        });
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
            if r.status() != "online" || r.refuse.load(std::sync::atomic::Ordering::SeqCst) {
                return;
            }
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
                    Json(json!({ "peers": [{ "hostId": "host-b", "name": "원격 PC", "status": r.status(), "endpoint": endpoint }] }))
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
        remote_daemon_with(relay, state, paired, None).await
    }

    /// 이전 릴리스(0.1.14)의 원격 데몬: ALPN에 `h2`를 두지만 HTTP/2로는 루프백(터널)에만 답하고 직접 연결은 HTTP/1.1이다
    async fn earlier_remote_daemon(relay: &Target, state: &Relay) -> (tempfile::TempDir, CancellationToken) {
        remote_daemon_with(relay, state, None, Some(vec![b"h2".to_vec(), b"http/1.1".to_vec()])).await
    }

    async fn remote_daemon_with(relay: &Target, state: &Relay, paired: Option<&ConfigFile>, alpn: Option<Vec<Vec<u8>>>) -> (tempfile::TempDir, CancellationToken) {
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
            .route("/api/declined", axum::routing::post(|| async { Json(json!({ "ok": false, "error_code": "not_found" })) }))
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
                "/api/slow-once",
                get({
                    let calls = Arc::new(std::sync::atomic::AtomicUsize::new(0));
                    move || async move {
                        if calls.fetch_add(1, std::sync::atomic::Ordering::SeqCst) == 0 {
                            tokio::time::sleep(RESPONSE_TIMEOUT * 3).await;
                        }
                        Json(json!({ "ok": true }))
                    }
                }),
            )
            .route(
                "/api/cut-once",
                get(cut_once(state.cut.clone())).post(cut_once(state.cut.clone())),
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
        let mut tls = id.server_config().unwrap();
        if let Some(alpn) = alpn {
            tls.alpn_protocols = alpn;
        }
        tokio::spawn(server::serve(listener, Arc::new(tls), server::router(routes, ctx), cancel.clone()));
        (dir, cancel)
    }

    /// 처음 받은 요청은 원격 데몬이 받은 뒤 연결이 끊긴다(`proxy`). 다시 오면 답한다
    fn cut_once(cut: Arc<Mutex<CancellationToken>>) -> impl Fn() -> std::pin::Pin<Box<dyn std::future::Future<Output = Json<Value>> + Send>> + Clone + Send + Sync + 'static {
        let calls = Arc::new(std::sync::atomic::AtomicUsize::new(0));
        move || {
            let (calls, cut) = (calls.clone(), cut.clone());
            Box::pin(async move {
                if calls.fetch_add(1, std::sync::atomic::Ordering::SeqCst) == 0 {
                    std::mem::take(&mut *cut.lock().unwrap()).cancel();
                    tokio::time::sleep(RESPONSE_TIMEOUT * 3).await;
                }
                Json(json!({ "ok": true }))
            })
        }
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

    /// 새 원격 데몬에는 HTTP/2 직접 연결 하나에 요청을 모두 싣는다: 콘솔·이벤트 스트림이 여럿 열려 있어도 다른 요청이 막히거나
    /// 원격 데몬의 연결 자리(한 곳 상한)를 다 써서 터널로 밀려나지 않는다. 스트림이 열린 연결은 쉬는 것으로 보지 않고,
    /// 쉬다가 원격 데몬이 닫은 연결 뒤의 요청은 새 연결로 바로 간다
    #[tokio::test]
    async fn many_open_streams_do_not_push_requests_off_the_direct_connection() {
        let state = Relay::default();
        let relay = fake_relay(state.clone()).await;
        let (_b, _stop) = remote_daemon(&relay, &state, None).await;
        let dir = tempfile::tempdir().unwrap();
        let file = ConfigFile::at(dir.path());
        connect(&file, Some(relay.clone()), "host-b").await.unwrap();
        let mut streams = Vec::new();
        for _ in 0..20 {
            let res = forward(&file, Some(relay.clone()), "host-b", "/api/stream", get_req(&[]), CancellationToken::new()).await;
            let mut stream = res.into_body().into_data_stream();
            assert_eq!(stream.next().await.unwrap().unwrap(), "first\n");
            streams.push(stream);
        }
        // 원격 데몬이 쉬는 연결을 닫는 시간(시험에서 1초)보다 오래 지나도 스트림이 열린 연결은 그대로다
        tokio::time::sleep(Duration::from_millis(2500)).await;
        let (status, _) = json_of(forward(&file, Some(relay.clone()), "host-b", "/api/echo", get_req(&[]), CancellationToken::new()).await).await;
        assert_eq!(status, 200);
        let post = Request::post("/ignored").header("content-type", "text/plain").body(Body::from("hello")).unwrap();
        let res = forward(&file, Some(relay.clone()), "host-b", "/api/echo", post, CancellationToken::new()).await;
        assert_eq!(axum::body::to_bytes(res.into_body(), 1024).await.unwrap(), "hello");
        for stream in &mut streams {
            assert!(tokio::time::timeout(Duration::from_millis(20), stream.next()).await.is_err(), "the stream is still open");
        }
        assert_eq!(state.tunnels.load(std::sync::atomic::Ordering::SeqCst), 0, "every request went straight to the remote daemon");

        drop(streams);
        tokio::time::sleep(Duration::from_millis(2500)).await;
        let (status, _) = json_of(forward(&file, Some(relay), "host-b", "/api/echo", get_req(&[]), CancellationToken::new()).await).await;
        assert_eq!(status, 200);
        assert_eq!(state.tunnels.load(std::sync::atomic::Ordering::SeqCst), 0);
    }

    /// 열어 둔 HTTP/2 연결이 요청이 나간 뒤 끊기면, 원격 데몬이 받았는지 모르므로 읽기만 새 연결로 다시 보내고
    /// 바꾸는 요청은 다시 보내지 않는다
    #[tokio::test]
    async fn after_the_shared_connection_drops_only_a_read_is_sent_again() {
        let state = Relay::default();
        let relay = fake_relay(state.clone()).await;
        let (_b, _stop) = remote_daemon(&relay, &state, None).await;
        proxy(&state, Duration::ZERO).await;
        let dir = tempfile::tempdir().unwrap();
        let file = ConfigFile::at(dir.path());
        connect(&file, Some(relay.clone()), "host-b").await.unwrap();
        let send = |req| forward(&file, Some(relay.clone()), "host-b", "/api/cut-once", req, CancellationToken::new());

        let (status, body) = json_of(send(get_req(&[])).await).await;
        assert_eq!((status, body), (200, json!({ "ok": true })));
        let (status, body) = json_of(send(Request::post("/ignored").body(Body::empty()).unwrap()).await).await;
        assert_eq!((status, body["error_code"].as_str()), (504, Some("remote_timeout")));
        assert_eq!(state.tunnels.load(std::sync::atomic::Ordering::SeqCst), 0);
    }

    /// 원격 데몬은 쉬는 HTTP/2 연결을 닫는다. 그 닫힘이 닿지 않은 연결(원격 PC의 망이 조용히 끊김)에는 쉬고 난 뒤의 요청을
    /// 싣지 않고 새로 연다: 바꾸는 요청도 답을 기다리다 504로 끝나지 않는다
    #[tokio::test]
    async fn a_connection_that_went_quiet_while_idle_is_not_used_again() {
        let state = Relay::default();
        let relay = fake_relay(state.clone()).await;
        let (_b, _stop) = remote_daemon(&relay, &state, None).await;
        proxy(&state, Duration::ZERO).await;
        let dir = tempfile::tempdir().unwrap();
        let file = ConfigFile::at(dir.path());
        connect(&file, Some(relay.clone()), "host-b").await.unwrap();
        let (status, _) = json_of(forward(&file, Some(relay.clone()), "host-b", "/api/echo", get_req(&[]), CancellationToken::new()).await).await;
        assert_eq!(status, 200);

        state.silent.store(true, std::sync::atomic::Ordering::SeqCst);
        std::mem::take(&mut *state.cut.lock().unwrap()).cancel();
        tokio::time::sleep(REUSE_WINDOW + Duration::from_millis(200)).await;
        let post = Request::post("/ignored").header("content-type", "text/plain").body(Body::from("hello")).unwrap();
        let res = forward(&file, Some(relay), "host-b", "/api/echo", post, CancellationToken::new()).await;
        assert_eq!(res.status(), StatusCode::OK);
        assert_eq!(axum::body::to_bytes(res.into_body(), 1024).await.unwrap(), "hello");
    }

    /// 이전 릴리스의 원격 데몬(HTTP/1.1)에도 직접 붙는다. 잇달아 보내는 요청(GUI 폴링)은 방금 쓴 직접 연결을 다시 쓰고,
    /// 원격 데몬이 닫았을 오래된 연결은 쓰지 않는다
    #[tokio::test]
    async fn back_to_back_requests_reuse_the_direct_connection() {
        let state = Relay::default();
        let relay = fake_relay(state.clone()).await;
        let (_b, _stop) = earlier_remote_daemon(&relay, &state).await;
        let dir = tempfile::tempdir().unwrap();
        let file = ConfigFile::at(dir.path());
        let daemon = connect(&file, Some(relay.clone()), "host-b").await.unwrap();
        let key = idle_key(&endpoint_of(&daemon));
        let idle = || IDLE.lock().unwrap().get(&key).map_or(0, Vec::len);

        for _ in 0..3 {
            let (status, _) = json_of(forward(&file, Some(relay.clone()), "host-b", "/api/echo", get_req(&[]), CancellationToken::new()).await).await;
            assert_eq!(status, 200);
            assert_eq!(idle(), 1, "one connection, used again");
        }
        // 원격 데몬이 다음 요청을 기다리다 닫을 만큼 지난 뒤에도 요청은 새 연결로 간다
        tokio::time::sleep(Duration::from_millis(1200)).await;
        let (status, _) = json_of(forward(&file, Some(relay), "host-b", "/api/echo", get_req(&[]), CancellationToken::new()).await).await;
        assert_eq!(status, 200);
        assert_eq!(idle(), 1);
    }

    /// 이전 릴리스(HTTP/1.1)로 답한 원격 데몬에는 연결 하나를 같이 쓸 수 없으므로, 동시에 온 요청이 연결을 하나씩 차례로 열지 않는다
    #[tokio::test]
    async fn concurrent_requests_to_an_earlier_remote_daemon_open_their_connections_together() {
        const OPENING: Duration = Duration::from_millis(300);
        let state = Relay::default();
        let relay = fake_relay(state.clone()).await;
        let (_b, _stop) = earlier_remote_daemon(&relay, &state).await;
        proxy(&state, OPENING).await;
        let dir = tempfile::tempdir().unwrap();
        let file = ConfigFile::at(dir.path());
        connect(&file, Some(relay.clone()), "host-b").await.unwrap();

        let started = std::time::Instant::now();
        let posts = (0..4).map(|_| {
            let post = Request::post("/ignored").header("content-type", "text/plain").body(Body::from("hello")).unwrap();
            forward(&file, Some(relay.clone()), "host-b", "/api/echo", post, CancellationToken::new())
        });
        for res in futures_util::future::join_all(posts).await {
            assert_eq!(res.status(), StatusCode::OK);
        }
        assert!(started.elapsed() < OPENING * 3, "4 connections took {:?} to open", started.elapsed());
        assert_eq!(state.tunnels.load(std::sync::atomic::Ordering::SeqCst), 0);
    }

    /// HTTP/1.1(이전 릴리스)에서 다시 쓴 연결에 보낸 읽기 요청은 답 머리를 짧게만 기다린다. 조용히 끊긴 연결이면 새 연결로 다시 보낸다
    #[tokio::test]
    async fn a_read_on_a_reused_connection_that_goes_quiet_is_sent_again_on_a_new_one() {
        let state = Relay::default();
        let relay = fake_relay(state.clone()).await;
        let (_b, _stop) = earlier_remote_daemon(&relay, &state).await;
        let dir = tempfile::tempdir().unwrap();
        let file = ConfigFile::at(dir.path());
        connect(&file, Some(relay.clone()), "host-b").await.unwrap();
        let (status, _) = json_of(forward(&file, Some(relay.clone()), "host-b", "/api/echo", get_req(&[]), CancellationToken::new()).await).await;
        assert_eq!(status, 200);
        // 남겨 둔 연결로 간 첫 요청은 답이 오지 않는다
        let (status, body) = json_of(forward(&file, Some(relay), "host-b", "/api/slow-once", get_req(&[]), CancellationToken::new()).await).await;
        assert_eq!((status, body), (200, json!({ "ok": true })));
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

        // 200이어도 본문이 실패(`success`·`ok`가 false)라고 하면 실패로 남기고, 본문은 그대로 넘긴다
        let declined = forward(&file, Some(relay_b.clone()), "host-b", "/api/declined", Request::post("/ignored").body(Body::empty()).unwrap(), CancellationToken::new());
        let declined_origin = Origin { user_name: Some(format!("{marker}-declined")), ..Origin::new("gui") };
        let (status, body) = json_of(with_origin(declined_origin, declined).await).await;
        assert_eq!((status, body["error_code"].as_str()), (200, Some("not_found")));
        let declined: Vec<Value> = crate::audit::log().recent(1000, None).into_iter().filter(|e| e["userName"] == format!("{marker}-declined").as_str()).collect();
        assert_eq!((declined.len(), declined[0]["ok"].as_bool(), declined[0]["status"].as_u64()), (1, Some(false), Some(200)));

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
        assert!(tunnel::is_open(Some(&relay), "host-b").await);
        tunnel::close("host-b");
        assert!(!tunnel::is_open(Some(&relay), "host-b").await);
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

    /// 주소가 말없이 붙잡히면(방화벽이 버림) 직접 연결이 포기할 때(10초)까지 기다리지 않는다.
    /// 잠깐 기다려도 안 열리면 터널도 같이 열고 먼저 열린 쪽으로 보낸다 (사용자 11-12: 연결을 누르면 결과까지 멈춤)
    #[tokio::test]
    async fn a_silent_address_does_not_hold_the_request_until_the_direct_timeout() {
        let state = Relay::default();
        let relay = fake_relay(state.clone()).await;
        let (_b, _stop) = remote_daemon(&relay, &state, None).await;
        // TCP는 받지만 TLS에는 답하지 않는 주소
        let silent = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let silent_port = silent.local_addr().unwrap().port();
        let _hold = tokio::spawn(async move {
            let mut held = Vec::new();
            while let Ok((sock, _)) = silent.accept().await {
                held.push(sock);
            }
        });
        state.endpoint.lock().unwrap()["port"] = json!(silent_port);
        let dir = tempfile::tempdir().unwrap();
        let file = ConfigFile::at(dir.path());
        let started = std::time::Instant::now();
        connect(&file, Some(relay.clone()), "host-b").await.unwrap();
        assert!(started.elapsed() < CONNECT_TIMEOUT / 2, "paired through the tunnel in {:?}", started.elapsed());
        assert_eq!(state.tunnels.load(std::sync::atomic::Ordering::SeqCst), 1);
    }

    /// 원격 데몬이 연결 자리가 없어 받자마자 끊은 것은 닿지 못하는 주소가 아니다: 자리가 나면 다음 요청은 바로 직접 간다
    #[tokio::test]
    async fn a_remote_daemon_with_no_free_connection_is_still_reached_directly_afterwards() {
        let state = Relay::default();
        let relay = fake_relay(state.clone()).await;
        let (_b, _stop) = remote_daemon(&relay, &state, None).await;
        let dir = tempfile::tempdir().unwrap();
        let file = ConfigFile::at(dir.path());
        let daemon = connect(&file, Some(relay.clone()), "host-b").await.unwrap();
        DIRECT.remove(&idle_key(&endpoint_of(&daemon)));
        // 이 PC(루프백) 몫의 자리를 다 쥔다
        let mut held = Vec::new();
        for _ in 0..32 {
            held.push(tokio::net::TcpStream::connect(("127.0.0.1", daemon.port)).await.unwrap());
        }
        tokio::time::sleep(Duration::from_millis(100)).await;
        let res = forward(&file, Some(relay.clone()), "host-b", "/api/echo", get_req(&[]), CancellationToken::new()).await;
        assert_eq!(res.status(), StatusCode::BAD_GATEWAY, "nothing gets in while every slot is taken");
        assert_eq!(route_of(&daemon), "direct");

        drop(held);
        tokio::time::sleep(Duration::from_millis(100)).await;
        let tunnels = state.tunnels.load(std::sync::atomic::Ordering::SeqCst);
        let (status, body) = json_of(forward(&file, Some(relay), "host-b", "/api/echo", get_req(&[]), CancellationToken::new()).await).await;
        assert_eq!(status, 200, "{body}");
        assert_eq!(state.tunnels.load(std::sync::atomic::Ordering::SeqCst), tunnels, "the request went straight to the remote daemon");
    }

    /// 저장한 주소에서 원격 데몬이 아닌 것이 받자마자 끊으면(포트 포워딩이 다른 곳을 가리킴) 몇 번 뒤에는 닿지 못하는 주소로 본다:
    /// PC 카드가 릴레이로 보이고 요청마다 직접 붙어 보지 않는다
    #[tokio::test]
    async fn an_address_that_keeps_hanging_up_is_shown_as_reached_through_the_relay() {
        let state = Relay::default();
        let relay = fake_relay(state.clone()).await;
        let (_b, _stop) = remote_daemon(&relay, &state, None).await;
        let other = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        state.endpoint.lock().unwrap()["port"] = json!(other.local_addr().unwrap().port());
        let tries = Arc::new(std::sync::atomic::AtomicUsize::new(0));
        tokio::spawn({
            let tries = tries.clone();
            async move {
                while other.accept().await.is_ok() {
                    tries.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
                }
            }
        });
        let dir = tempfile::tempdir().unwrap();
        let file = ConfigFile::at(dir.path());
        let daemon = connect(&file, Some(relay.clone()), "host-b").await.unwrap();
        for _ in 0..4 {
            let (status, _) = json_of(forward(&file, Some(relay.clone()), "host-b", "/api/echo", get_req(&[]), CancellationToken::new()).await).await;
            assert_eq!(status, 200);
        }
        assert_eq!(route_of(&daemon), "relay");
        let tries = tries.load(std::sync::atomic::Ordering::SeqCst);
        assert!(tries <= 3, "kept trying the address directly: {tries} times");
    }

    /// 릴레이가 꺼졌다고 하는 PC에는 요청마다 터널을 열지 않는다 (경고가 3초마다 쌓였다).
    /// 다시 켜지면 뒤에서 릴레이 PC 목록으로 알아채고 다시 붙는다
    #[tokio::test]
    async fn a_pc_the_relay_reports_offline_is_not_dialed_for_every_request() {
        let state = Relay::default();
        let relay = fake_relay(state.clone()).await;
        let (_b, _stop) = remote_daemon(&relay, &state, None).await;
        state.endpoint.lock().unwrap()["address"] = Value::Null;
        let dir = tempfile::tempdir().unwrap();
        let file = ConfigFile::at(dir.path());
        connect(&file, Some(relay.clone()), "host-b").await.unwrap();
        tunnel::release(&relay, "host-b");
        *state.status.lock().unwrap() = "offline".into();
        let dials = || state.tunnels.load(std::sync::atomic::Ordering::SeqCst);
        let before = dials();
        for _ in 0..5 {
            let res = forward(&file, Some(relay.clone()), "host-b", "/api/echo", get_req(&[]), CancellationToken::new()).await;
            assert_eq!(res.status(), StatusCode::BAD_GATEWAY);
        }
        assert_eq!(dials(), before + 1, "only the first request opens a tunnel");

        *state.status.lock().unwrap() = "online".into();
        tokio::time::sleep(OFFLINE_RECHECK * 2).await;
        // 쉬는 동안의 요청은 바로 닿지 않음으로 답하고, 뒤에서 릴레이에 다시 물어본다
        let _ = forward(&file, Some(relay.clone()), "host-b", "/api/echo", get_req(&[]), CancellationToken::new()).await;
        tokio::time::sleep(Duration::from_millis(200)).await;
        let (status, body) = json_of(forward(&file, Some(relay), "host-b", "/api/echo", get_req(&[]), CancellationToken::new()).await).await;
        assert_eq!(status, 200, "{body}");
    }

    /// 릴레이는 켜졌다는데 터널이 잇달아 닿지 않으면 간격을 늘려 가며 쉰다
    #[tokio::test]
    async fn a_tunnel_that_keeps_failing_is_retried_with_growing_gaps() {
        let state = Relay::default();
        let relay = fake_relay(state.clone()).await;
        let (_b, _stop) = remote_daemon(&relay, &state, None).await;
        state.endpoint.lock().unwrap()["address"] = Value::Null;
        let dir = tempfile::tempdir().unwrap();
        let file = ConfigFile::at(dir.path());
        connect(&file, Some(relay.clone()), "host-b").await.unwrap();
        tunnel::release(&relay, "host-b");
        state.refuse.store(true, std::sync::atomic::Ordering::SeqCst);
        let dials = || state.tunnels.load(std::sync::atomic::Ordering::SeqCst);
        let before = dials();
        for _ in 0..8 {
            let res = forward(&file, Some(relay.clone()), "host-b", "/api/echo", get_req(&[]), CancellationToken::new()).await;
            assert_eq!(res.status(), StatusCode::BAD_GATEWAY);
        }
        assert_eq!(dials(), before + UNREACHABLE_LIMIT as usize, "stops dialing after {UNREACHABLE_LIMIT} failures in a row");

        state.refuse.store(false, std::sync::atomic::Ordering::SeqCst);
        tokio::time::sleep(DIRECT_RETRY * 2).await;
        let _ = forward(&file, Some(relay.clone()), "host-b", "/api/echo", get_req(&[]), CancellationToken::new()).await;
        tokio::time::sleep(Duration::from_millis(200)).await;
        let (status, body) = json_of(forward(&file, Some(relay), "host-b", "/api/echo", get_req(&[]), CancellationToken::new()).await).await;
        assert_eq!(status, 200, "{body}");
    }

    /// 직접 닿지 않는 주소는 다시 붙어 보는 간격을 늘려 간다 (잠깐 내려간 원격 데몬은 금방 다시 직접 붙고, NAT 뒤 PC는 자주 두드리지 않는다)
    #[tokio::test]
    async fn an_unreachable_address_is_tried_again_soon_and_then_less_and_less_often() {
        let state = Relay::default();
        let relay = fake_relay(state.clone()).await;
        let (_b, _stop) = remote_daemon(&relay, &state, None).await;
        // 그 주소에는 원격 데몬이 아닌 것이 있다
        let other = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        state.endpoint.lock().unwrap()["port"] = json!(other.local_addr().unwrap().port());
        let tries = Arc::new(std::sync::atomic::AtomicUsize::new(0));
        tokio::spawn({
            let tries = tries.clone();
            async move {
                while let Ok((mut tcp, _)) = other.accept().await {
                    tries.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
                    // 보낸 것을 읽고 답한다: 읽지 않고 닫으면 Windows는 reset을 보내 '자리가 없어 끊음'으로 읽힌다
                    let _ = tokio::io::AsyncReadExt::read(&mut tcp, &mut [0u8; 4096]).await;
                    let _ = tokio::io::AsyncWriteExt::write_all(&mut tcp, b"not a remote daemon\n").await;
                }
            }
        });
        let dir = tempfile::tempdir().unwrap();
        let file = ConfigFile::at(dir.path());
        connect(&file, Some(relay.clone()), "host-b").await.unwrap();
        assert_eq!(tries.load(std::sync::atomic::Ordering::SeqCst), 1);

        let started = std::time::Instant::now();
        while started.elapsed() < DIRECT_RETRY * 6 {
            let (status, _) = json_of(forward(&file, Some(relay.clone()), "host-b", "/api/echo", get_req(&[]), CancellationToken::new()).await).await;
            assert_eq!(status, 200);
            tokio::time::sleep(DIRECT_RETRY / 8).await;
        }
        tokio::time::sleep(Duration::from_millis(50)).await;
        let again = tries.load(std::sync::atomic::Ordering::SeqCst) - 1;
        assert!((1..=3).contains(&again), "tried again {again} times in six first intervals");
    }

    /// 터널로 짝 맺은 PC에 직접 닿게 되면 열어 둔 터널을 놓는다. 그 위에서 받던 스트림은 끊기지 않는다
    #[tokio::test]
    async fn the_tunnel_is_let_go_once_the_remote_daemon_answers_directly() {
        let state = Relay::default();
        let relay = fake_relay(state.clone()).await;
        let (_b, _stop) = remote_daemon(&relay, &state, None).await;
        let reachable = state.endpoint.lock().unwrap().clone();
        state.endpoint.lock().unwrap()["address"] = Value::Null;
        let dir = tempfile::tempdir().unwrap();
        let file = ConfigFile::at(dir.path());
        let daemon = connect(&file, Some(relay.clone()), "host-b").await.unwrap();
        let res = forward(&file, Some(relay.clone()), "host-b", "/api/stream", get_req(&[]), CancellationToken::new()).await;
        let mut stream = res.into_body().into_data_stream();
        assert_eq!(stream.next().await.unwrap().unwrap(), "first\n");
        assert!(tunnel::is_open(Some(&relay), "host-b").await);

        // 원격 PC가 공인 주소를 올렸다
        config::update_endpoint_in(&file, "host-b", reachable["address"].as_str().unwrap(), daemon.port, &daemon.cert_sha256).unwrap();
        let (status, _) = json_of(forward(&file, Some(relay.clone()), "host-b", "/api/echo", get_req(&[]), CancellationToken::new()).await).await;
        assert_eq!(status, 200);
        assert!(!tunnel::is_open(Some(&relay), "host-b").await);
        assert!(tokio::time::timeout(Duration::from_millis(500), stream.next()).await.is_err(), "the stream on the tunnel is still open");
    }

    /// 직접 연결은 TCP keepalive를 켠다 (원격 PC의 망이 조용히 끊기면 기다리던 답과 이벤트 스트림이 끝나게).
    /// 끊긴 연결이 실제로 끝나는 것은 OS 몫이라 여기서 보지 않는다
    #[tokio::test]
    async fn a_direct_connection_has_tcp_keepalive_on() {
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let endpoint = Endpoint { address: "127.0.0.1".into(), port: listener.local_addr().unwrap().port(), cert_sha256: String::new() };
        let tcp = dial(&endpoint).await.unwrap();
        assert!(socket2::SockRef::from(&tcp).keepalive().unwrap());
        #[cfg(target_os = "linux")]
        assert_eq!((socket2::SockRef::from(&tcp).tcp_keepalive_time().unwrap(), socket2::SockRef::from(&tcp).tcp_keepalive_interval().unwrap()), (KEEPALIVE_IDLE, KEEPALIVE_INTERVAL));
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
