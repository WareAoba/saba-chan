//! 공인 주소 없는 PC 터널 (T1, 05 §4.4 (k)): 직접 붙지 못하는 원격 데몬에 릴레이가 바이트를 이어 준다.
//!
//!   붙는 쪽(A)   `dial`    릴레이 `/node/tunnel?to=<B>`를 열고, 그 위에 지문 고정 TLS + HTTP/2 연결 하나를 맺어 원격 PC마다 다시 쓴다
//!   원격 쪽(B)   `accept`  릴레이 명령 `remote_tunnel {tunnelId}`을 받으면 `/node/tunnel?accept=<id>`를 열고 자기 원격 포트에 잇는다
//!
//! 릴레이는 두 데몬의 TLS 암호문만 넘긴다. 원격 포트의 인증(짝 맺은 PC 토큰)은 직접 연결과 같다

use std::collections::HashMap;
use std::sync::{Arc, LazyLock, Mutex};
use std::time::Duration;

use axum::body::Body;
use futures_util::{SinkExt, StreamExt};
use tokio::io::{AsyncRead, AsyncReadExt, AsyncWrite, AsyncWriteExt};
use tokio_tungstenite::tungstenite::{client::IntoClientRequest, http::HeaderValue, Message};

use super::relay::Target;
use super::tls;

/// 릴레이에 붙고, 원격 PC가 받고, TLS·HTTP/2를 맺을 때까지 (릴레이는 15초 안에 받지 않으면 닫는다)
const DIAL_TIMEOUT: Duration = Duration::from_secs(20);
/// 웹소켓 프레임 하나에 싣는 바이트
const CHUNK: usize = 64 * 1024;

pub type Sender = hyper::client::conn::http2::SendRequest<Body>;

/// `https://relay` → `wss://relay/node/tunnel?<query>` (모의 릴레이는 http → ws)
fn url(relay_base: &str, query: &str) -> String {
    let base = relay_base.trim_end_matches('/');
    let base = base
        .strip_prefix("https://")
        .map(|r| format!("wss://{r}"))
        .or_else(|| base.strip_prefix("http://").map(|r| format!("ws://{r}")))
        .unwrap_or_else(|| base.to_string());
    format!("{base}/node/tunnel?{query}")
}

type Socket = tokio_tungstenite::WebSocketStream<tokio_tungstenite::MaybeTlsStream<tokio::net::TcpStream>>;

async fn open_socket(relay: &Target, query: &str) -> Option<Socket> {
    let mut request = url(&relay.base, query).into_client_request().ok()?;
    request
        .headers_mut()
        .insert("Authorization", format!("Bearer {}", relay.token).parse().ok()?);
    request
        .headers_mut()
        .insert("X-Saba-Client", HeaderValue::from_static("1"));
    let (socket, _) = tokio_tungstenite::connect_async(request).await.ok()?;
    Some(socket)
}

/// 웹소켓의 바이너리 프레임과 `io`의 바이트를 잇는다. 한쪽이 끝나면 둘 다 닫는다
async fn pump<S: AsyncRead + AsyncWrite>(socket: Socket, io: S) {
    let (mut sink, mut stream) = socket.split();
    let (mut rd, mut wr) = tokio::io::split(io);
    let up = async {
        let mut buf = vec![0u8; CHUNK];
        while let Ok(n @ 1..) = rd.read(&mut buf).await {
            if sink.send(Message::Binary(buf[..n].to_vec().into())).await.is_err() {
                return;
            }
        }
        let _ = sink.close().await;
    };
    let down = async {
        while let Some(Ok(msg)) = stream.next().await {
            match msg {
                Message::Binary(bytes) if wr.write_all(&bytes).await.is_ok() => {}
                Message::Ping(_) | Message::Pong(_) => {}
                _ => break,
            }
        }
        let _ = wr.shutdown().await;
    };
    tokio::select! {
        () = up => {}
        () = down => {}
    }
}

/// 원격 쪽: 릴레이가 연 터널 `tunnel_id`를 받아 이 PC의 원격 포트에 잇는다
pub async fn accept(relay: Target, tunnel_id: String, port: u16) {
    let Some(socket) = tokio::time::timeout(DIAL_TIMEOUT, open_socket(&relay, &format!("accept={tunnel_id}")))
        .await
        .ok()
        .flatten()
    else {
        tracing::warn!("Could not accept a remote daemon tunnel from the relay");
        return;
    };
    // 원격 포트는 두 겹(IPv6·IPv4)이거나 IPv4만, 시험에서는 루프백 하나다
    let local = match tokio::net::TcpStream::connect((std::net::Ipv4Addr::LOCALHOST, port)).await {
        Ok(tcp) => tcp,
        Err(_) => match tokio::net::TcpStream::connect((std::net::Ipv6Addr::LOCALHOST, port)).await {
            Ok(tcp) => tcp,
            Err(e) => {
                tracing::warn!("Remote daemon tunnel could not reach the remote port: {}", e);
                return;
            }
        },
    };
    pump(socket, local).await;
}

/// 붙는 쪽: 원격 PC `host_id`까지 터널을 열고 지문 `pin`을 고정해 HTTP/2 연결을 맺는다
async fn dial(relay: &Target, host_id: &str, pin: &str) -> Option<Sender> {
    // 호스트 id는 nanoid 문자만 (쿼리에 그대로 넣는다)
    if !super::relay::valid_ticket_id(host_id) {
        return None;
    }
    let connect = async {
        let socket = open_socket(relay, &format!("to={host_id}")).await?;
        let (near, far) = tokio::io::duplex(4 * CHUNK);
        tokio::spawn(pump(socket, far));
        let mut tls_config = tls::pinned_client_config(pin).ok()?;
        tls_config.alpn_protocols = vec![b"h2".to_vec()];
        let name = rustls::pki_types::ServerName::try_from(tls::SERVER_NAME).ok()?;
        let stream = tokio_rustls::TlsConnector::from(Arc::new(tls_config))
            .connect(name, near)
            .await
            .ok()?;
        let (send, conn) = hyper::client::conn::http2::Builder::new(hyper_util::rt::TokioExecutor::new())
            .timer(hyper_util::rt::TokioTimer::new())
            .keep_alive_interval(Duration::from_secs(25))
            .keep_alive_timeout(Duration::from_secs(20))
            .handshake(hyper_util::rt::TokioIo::new(stream))
            .await
            .ok()?;
        tokio::spawn(conn);
        Some(send)
    };
    tokio::time::timeout(DIAL_TIMEOUT, connect).await.ok().flatten()
}

type Slot = Arc<tokio::sync::Mutex<Option<Sender>>>;

/// 릴레이·원격 PC마다 연결 하나. 여는 동안 같은 PC의 다른 요청은 그 연결을 기다린다
static OPEN: LazyLock<Mutex<HashMap<(String, String), Slot>>> = LazyLock::new(Mutex::default);

fn slot(relay: &Target, host_id: &str) -> Slot {
    let key = (relay.base.clone(), host_id.to_string());
    OPEN.lock()
        .unwrap_or_else(|e| e.into_inner())
        .entry(key)
        .or_default()
        .clone()
}

/// 이 PC까지 열린 터널이 있는가 (있으면 직접 연결보다 먼저 쓴다: NAT 뒤 PC에 직접 붙기를 매번 기다리지 않게)
pub fn is_open(relay: Option<&Target>, host_id: &str) -> bool {
    relay.is_some_and(|r| {
        slot(r, host_id)
            .try_lock()
            .is_ok_and(|s| s.as_ref().is_some_and(|s| !s.is_closed()))
    })
}

/// 원격 데몬을 잊으면 그 PC까지 열어 둔 터널을 놓는다 (연결을 쥔 것이 없어지면 HTTP/2 연결과 터널이 닫힌다.
/// 릴레이는 한 PC가 여는 터널 수를 묶는다)
pub fn close(host_id: &str) {
    OPEN.lock().unwrap_or_else(|e| e.into_inner()).retain(|(_, h), _| h != host_id);
}

/// 열린 터널을 쓰고, 없거나 끊겼으면 새로 연다
pub async fn sender(relay: &Target, host_id: &str, pin: &str) -> Option<Sender> {
    let slot = slot(relay, host_id);
    let mut open = slot.lock().await;
    if let Some(s) = open.as_ref().filter(|s| !s.is_closed()) {
        return Some(s.clone());
    }
    *open = dial(relay, host_id, pin).await;
    if open.is_some() {
        tracing::info!("Opened a relay tunnel to remote daemon {}", host_id);
    }
    open.clone()
}

/// HTTP/2 요청은 절대 주소를 쓴다 (`:authority`)
pub fn absolute(req: &mut hyper::Request<Body>) {
    let path = req
        .uri()
        .path_and_query()
        .map(|p| p.as_str().to_string())
        .unwrap_or_else(|| "/".into());
    if let Ok(uri) = format!("https://{}{}", tls::SERVER_NAME, path).parse() {
        *req.uri_mut() = uri;
    }
    req.headers_mut().remove(hyper::header::HOST);
}

#[cfg(test)]
mod tests {
    #[test]
    fn the_tunnel_url_follows_the_relay_scheme() {
        assert_eq!(
            super::url("https://relay.example/", "to=a"),
            "wss://relay.example/node/tunnel?to=a"
        );
        assert_eq!(
            super::url("http://127.0.0.1:9", "accept=x"),
            "ws://127.0.0.1:9/node/tunnel?accept=x"
        );
    }
}
