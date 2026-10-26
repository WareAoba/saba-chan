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
    let proxy = proxy_for(&url(&relay.base, query), |k| std::env::var(k).ok());
    open_socket_via(relay, query, proxy).await
}

async fn open_socket_via(relay: &Target, query: &str, proxy: Option<Proxy>) -> Option<Socket> {
    let mut request = url(&relay.base, query).into_client_request().ok()?;
    request
        .headers_mut()
        .insert("Authorization", format!("Bearer {}", relay.token).parse().ok()?);
    request
        .headers_mut()
        .insert("X-Saba-Client", HeaderValue::from_static("1"));
    let opened = match proxy {
        None => tokio_tungstenite::connect_async(request).await,
        Some(proxy) => {
            let uri = request.uri();
            let host = uri.host()?.to_string();
            let port = uri.port_u16().unwrap_or(if uri.scheme_str() == Some("wss") { 443 } else { 80 });
            let tcp = connect_through(&proxy, &host, port).await?;
            tokio_tungstenite::client_async_tls(request, tcp).await
        }
    };
    match opened {
        Ok((socket, _)) => Some(socket),
        Err(e) => {
            tracing::warn!("Relay tunnel socket failed: {}", e);
            None
        }
    }
}

/// 프록시(`http://[사용자:암호@]호스트:포트`). reqwest는 프록시 환경을 따르지만 웹소켓은 reqwest를 거치지 않는다
#[derive(Debug)]
struct Proxy {
    addr: String,
    /// `Proxy-Authorization: Basic`의 값
    auth: Option<String>,
}

/// reqwest(`system-proxy`)와 같은 환경 변수: wss는 `HTTPS_PROXY`, ws는 `HTTP_PROXY`, 그다음 `ALL_PROXY`.
/// `NO_PROXY`의 호스트·도메인(`.corp`, `corp`)과 `*`, 루프백은 직접 붙는다. http 프록시만 쓴다
fn proxy_for(target: &str, env: impl Fn(&str) -> Option<String>) -> Option<Proxy> {
    use base64::Engine;
    let uri: tokio_tungstenite::tungstenite::http::Uri = target.parse().ok()?;
    let host = uri.host()?.trim_start_matches('[').trim_end_matches(']').to_ascii_lowercase();
    if host == "localhost" || host.parse::<std::net::IpAddr>().is_ok_and(|ip| ip.is_loopback()) {
        return None;
    }
    let get = |names: &[&str]| names.iter().find_map(|n| env(n).filter(|v| !v.trim().is_empty()));
    let no_proxy = get(&["NO_PROXY", "no_proxy"]).unwrap_or_default();
    let bypass = no_proxy.split(',').map(str::trim).filter(|e| !e.is_empty()).any(|e| {
        let domain = e.trim_start_matches('.').to_ascii_lowercase();
        e == "*" || host == domain || host.ends_with(&format!(".{domain}"))
    });
    if bypass {
        return None;
    }
    let value = match uri.scheme_str() {
        Some("wss") => get(&["HTTPS_PROXY", "https_proxy", "ALL_PROXY", "all_proxy"]),
        _ => get(&["HTTP_PROXY", "http_proxy", "ALL_PROXY", "all_proxy"]),
    }?;
    let rest = value.trim().strip_prefix("http://")?.trim_end_matches('/');
    let (auth, addr) = match rest.rsplit_once('@') {
        Some((user, addr)) => (Some(base64::engine::general_purpose::STANDARD.encode(user)), addr),
        None => (None, rest),
    };
    let addr = if addr.rsplit_once(':').is_some_and(|(_, p)| p.parse::<u16>().is_ok()) { addr.to_string() } else { format!("{addr}:80") };
    Some(Proxy { addr, auth })
}

/// 프록시에 `CONNECT host:port`로 터널을 연다 (응답 머리는 8KB까지)
async fn connect_through(proxy: &Proxy, host: &str, port: u16) -> Option<tokio::net::TcpStream> {
    let mut tcp = match tokio::net::TcpStream::connect(&proxy.addr).await {
        Ok(tcp) => tcp,
        Err(e) => {
            tracing::warn!("Relay tunnel could not reach the proxy {}: {}", proxy.addr, e);
            return None;
        }
    };
    let target = if host.contains(':') { format!("[{host}]:{port}") } else { format!("{host}:{port}") };
    let mut head = format!("CONNECT {target} HTTP/1.1\r\nHost: {target}\r\n");
    if let Some(auth) = &proxy.auth {
        head.push_str(&format!("Proxy-Authorization: Basic {auth}\r\n"));
    }
    head.push_str("\r\n");
    tcp.write_all(head.as_bytes()).await.ok()?;
    let mut reply = Vec::new();
    let mut byte = [0u8; 1];
    while !reply.ends_with(b"\r\n\r\n") {
        if reply.len() > 8192 {
            return None;
        }
        tcp.read_exact(&mut byte).await.ok()?;
        reply.push(byte[0]);
    }
    let status = String::from_utf8_lossy(&reply).split_whitespace().nth(1).map(str::to_string);
    if status.as_deref() != Some("200") {
        tracing::warn!("Relay tunnel proxy refused CONNECT: {}", status.unwrap_or_default());
        return None;
    }
    Some(tcp)
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
    let sender = tokio::time::timeout(DIAL_TIMEOUT, connect).await.ok().flatten();
    if sender.is_none() {
        tracing::warn!("Could not open a relay tunnel to remote daemon {}", host_id);
    }
    sender
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
    use super::*;

    fn env<'a>(pairs: &'a [(&'a str, &'a str)]) -> impl Fn(&str) -> Option<String> + 'a {
        move |k| pairs.iter().find(|(n, _)| *n == k).map(|(_, v)| v.to_string())
    }

    /// 웹소켓은 reqwest를 거치지 않으므로 프록시 환경을 직접 따른다 (프록시로만 나가는 회사망·클라우드)
    #[test]
    fn the_tunnel_follows_the_proxy_environment() {
        let vars = [("HTTPS_PROXY", "http://user:pw@proxy.lan:3128"), ("NO_PROXY", "localhost,.corp,relay.direct")];
        let proxy = proxy_for("wss://saba-chan.online/node/tunnel?to=a", env(&vars)).expect("a proxy");
        assert_eq!(proxy.addr, "proxy.lan:3128");
        assert_eq!(proxy.auth.as_deref(), Some("dXNlcjpwdw=="));
        assert!(proxy_for("wss://relay.corp/x", env(&vars)).is_none(), "NO_PROXY suffix");
        assert!(proxy_for("wss://relay.direct/x", env(&vars)).is_none(), "NO_PROXY host");
        assert!(proxy_for("ws://127.0.0.1:9/x", env(&vars)).is_none(), "loopback is never proxied");
        assert!(proxy_for("wss://saba-chan.online/x", env(&[("https_proxy", "http://p:8080"), ("no_proxy", "*")])).is_none());
        assert_eq!(proxy_for("wss://saba-chan.online/x", env(&[("https_proxy", "http://p:8080")])).unwrap().addr, "p:8080");
        assert!(proxy_for("wss://saba-chan.online/x", env(&[])).is_none());
    }

    /// 프록시에 CONNECT로 터널을 열고 그 위에서 웹소켓을 맺는다
    #[tokio::test]
    async fn the_tunnel_socket_goes_through_a_connect_proxy() {
        let relay = tokio::net::TcpListener::bind((std::net::Ipv4Addr::LOCALHOST, 0)).await.unwrap();
        let relay_port = relay.local_addr().unwrap().port();
        tokio::spawn(async move {
            let (tcp, _) = relay.accept().await.unwrap();
            let mut ws = tokio_tungstenite::accept_async(tcp).await.unwrap();
            if let Some(Ok(msg)) = ws.next().await {
                ws.send(msg).await.unwrap();
            }
        });
        let proxy = tokio::net::TcpListener::bind((std::net::Ipv4Addr::LOCALHOST, 0)).await.unwrap();
        let proxy_addr = proxy.local_addr().unwrap().to_string();
        let seen = Arc::new(Mutex::new(String::new()));
        let seen_by_proxy = seen.clone();
        tokio::spawn(async move {
            let (mut client, _) = proxy.accept().await.unwrap();
            let mut head = Vec::new();
            let mut byte = [0u8; 1];
            while !head.ends_with(b"\r\n\r\n") {
                client.read_exact(&mut byte).await.unwrap();
                head.push(byte[0]);
            }
            let head = String::from_utf8(head).unwrap();
            *seen_by_proxy.lock().unwrap() = head.lines().next().unwrap_or_default().to_string();
            let target = head.split_whitespace().nth(1).unwrap().to_string();
            let mut upstream = tokio::net::TcpStream::connect(target).await.unwrap();
            client.write_all(b"HTTP/1.1 200 Connection Established\r\n\r\n").await.unwrap();
            let _ = tokio::io::copy_bidirectional(&mut client, &mut upstream).await;
        });
        let target = Target { base: format!("http://127.0.0.1:{relay_port}"), token: "sbn_x.y".into() };
        let proxy = Proxy { addr: proxy_addr, auth: None };
        let mut socket = open_socket_via(&target, "accept=t", Some(proxy)).await.expect("opened through the proxy");
        socket.send(Message::Binary(b"ping".to_vec().into())).await.unwrap();
        let echoed = socket.next().await.unwrap().unwrap();
        assert_eq!(echoed.into_data().as_ref(), b"ping");
        assert_eq!(*seen.lock().unwrap(), format!("CONNECT 127.0.0.1:{relay_port} HTTP/1.1"));
    }

    #[test]
    fn the_tunnel_url_follows_the_relay_scheme() {
        assert_eq!(
            url("https://relay.example/", "to=a"),
            "wss://relay.example/node/tunnel?to=a"
        );
        assert_eq!(
            url("http://127.0.0.1:9", "accept=x"),
            "ws://127.0.0.1:9/node/tunnel?accept=x"
        );
    }
}
