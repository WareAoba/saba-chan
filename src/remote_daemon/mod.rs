//! 원격 데몬 (D1, 05 §4.4 (j)): 같은 주인의 다른 PC가 이 데몬에 TLS로 직접 붙는 원격 포트.
//!
//! 설정(config.json `remote`)이 켜져 있으면 자체 서명 인증서로 원격 포트를 열고, 공인 주소와 인증서 지문을 릴레이에 올린다.
//! 붙는 PC는 릴레이에서 받은 연결표로 한 번 짝을 맺고(`/remote/pair`), 그 뒤로는 받은 토큰으로 데몬 API를 부른다.
//! 지금은 공인 주소가 있는 PC만 된다 (`Transport::Direct`). 붙는 쪽(내 PC에서 원격 데몬을 고르고 중계하기)은 다음 단계다

pub mod config;
pub mod relay;
pub mod server;
pub mod tls;

use std::sync::{Arc, OnceLock, RwLock};

use saba_chan_updater_lib::config_file::ConfigFile;
use serde::Serialize;
use tokio_util::sync::CancellationToken;

use crate::ipc::IPCServer;

#[derive(Debug, Clone, Default, Serialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct Status {
    pub listening: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub cert_sha256: Option<String>,
    /// 릴레이에 지금 주소를 올렸는지 (주소가 없거나 릴레이에 연결하지 않았으면 false)
    pub published: bool,
    /// `certificate_failed` | `port_in_use` | `bind_failed`
    #[serde(skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,
}

#[derive(Default)]
struct Runtime {
    cancel: Option<(CancellationToken, tokio::task::JoinHandle<()>)>,
    published: bool,
    status: Status,
}

static RUNTIME: tokio::sync::Mutex<Runtime> = tokio::sync::Mutex::const_new(Runtime { cancel: None, published: false, status: Status { listening: false, cert_sha256: None, published: false, error: None } });

/// 짝 맺은 PC 목록 (원격 포트의 인증층이 요청마다 본다). 설정 파일에서 채우고, 짝 맺기·풀기가 고친다
pub(crate) fn clients() -> Arc<RwLock<Vec<config::Client>>> {
    static CLIENTS: OnceLock<Arc<RwLock<Vec<config::Client>>>> = OnceLock::new();
    CLIENTS.get_or_init(Arc::default).clone()
}

pub(crate) fn reload_clients(file: &ConfigFile) {
    *clients().write().unwrap_or_else(|e| e.into_inner()) = config::clients_in(file);
}

pub async fn status() -> Status {
    RUNTIME.lock().await.status.clone()
}

/// 릴레이에 올릴 주소: 호스트 이름(RFC 1123) 또는 IP 리터럴 (릴레이 `isValidRemoteAddress`와 같은 규칙)
pub fn valid_address(s: &str) -> bool {
    if s.parse::<std::net::IpAddr>().is_ok() {
        return true;
    }
    s.len() <= 253
        && s.split('.').all(|label| {
            (1..=63).contains(&label.len())
                && !label.starts_with('-')
                && !label.ends_with('-')
                && label.bytes().all(|b| b.is_ascii_alphanumeric() || b == b'-')
        })
}

fn listen_on(addr: std::net::SocketAddr, dual: bool) -> std::io::Result<std::net::TcpListener> {
    let socket = socket2::Socket::new(socket2::Domain::for_address(addr), socket2::Type::STREAM, Some(socket2::Protocol::TCP))?;
    if dual {
        socket.set_only_v6(false)?;
    }
    // IPC와 같이 Unix에서만: 다시 열 때 TIME_WAIT 연결이 남아 있어도 묶이게 (Windows의 SO_REUSEADDR는 남의 리슨 포트도 허용한다)
    #[cfg(not(windows))]
    socket.set_reuse_address(true)?;
    socket.bind(&addr.into())?;
    socket.listen(128)?;
    socket.set_nonblocking(true)?;
    Ok(socket.into())
}

/// 두 겹(IPv6·IPv4)으로 받는다. IPv6가 없으면 IPv4만
fn bind(port: u16) -> std::io::Result<tokio::net::TcpListener> {
    let std_listener = match listen_on((std::net::Ipv6Addr::UNSPECIFIED, port).into(), true) {
        Ok(l) => l,
        Err(e) if e.kind() == std::io::ErrorKind::AddrInUse => return Err(e),
        Err(_) => listen_on((std::net::Ipv4Addr::UNSPECIFIED, port).into(), false)?,
    };
    tokio::net::TcpListener::from_std(std_listener)
}

/// 다시 올리기 간격의 상한
const RETRY_MAX: std::time::Duration = std::time::Duration::from_secs(300);

/// `first`만큼 기다렸다가 주소를 올린다. 실패하면 간격을 두 배씩 늘려(상한 5분) 올릴 때까지 다시 한다.
/// `cancel`(포트를 닫거나 설정을 다시 적용함)이면 멈추고 false
async fn publish_with_retry(
    relay: &server::RelaySource,
    address: &str,
    port: u16,
    sha: &str,
    cancel: &CancellationToken,
    first: std::time::Duration,
) -> bool {
    let mut wait = first;
    loop {
        tokio::select! {
            _ = cancel.cancelled() => return false,
            _ = tokio::time::sleep(wait) => {}
        }
        if let Some(t) = relay.target().await {
            if relay::publish(&t, address, port, sha).await {
                return !cancel.is_cancelled();
            }
        }
        wait = (wait * 2).min(RETRY_MAX);
    }
}

/// 설정대로 원격 포트를 연다·닫는다·다시 연다. 설정을 바꾼 뒤, 데몬이 뜰 때, 노드 토큰이 바뀔 때 부른다
pub async fn apply(server: &IPCServer) -> Status {
    let file = ConfigFile::default_location();
    let settings = config::settings_in(&file);
    let relay = server::RelaySource::Configured(server.clone());
    let mut rt = RUNTIME.lock().await;
    // 예전 리스너가 포트를 놓을 때까지 기다린다 (같은 포트로 다시 열 때)
    if let Some((cancel, task)) = rt.cancel.take() {
        cancel.cancel();
        let _ = task.await;
    }
    reload_clients(&file);
    let mut status = Status::default();

    if settings.enabled {
        match tls::Identity::load_or_create(&saba_chan_updater_lib::constants::resolve_data_dir()) {
            Err(e) => {
                tracing::warn!("Remote daemon certificate failed: {}", e);
                status.error = Some("certificate_failed".into());
            }
            Ok(identity) => {
                status.cert_sha256 = Some(identity.sha256.clone());
                match (identity.server_config(), bind(settings.port)) {
                    (Ok(tls), Ok(listener)) => {
                        let cancel = server.shutdown_token.child_token();
                        let ctx = server::Ctx::new(file.clone(), clients(), relay.clone());
                        let app = server::router(server.routes(), ctx);
                        let task = tokio::spawn(server::serve(listener, Arc::new(tls), app, cancel.clone()));
                        rt.cancel = Some((cancel, task));
                        status.listening = true;
                        tracing::info!("Remote daemon listening on port {}", settings.port);
                    }
                    (Err(e), _) => {
                        tracing::warn!("Remote daemon TLS setup failed: {}", e);
                        status.error = Some("certificate_failed".into());
                    }
                    (_, Err(e)) => {
                        tracing::warn!("Remote daemon could not bind port {}: {}", settings.port, e);
                        let code = if e.kind() == std::io::ErrorKind::AddrInUse { "port_in_use" } else { "bind_failed" };
                        status.error = Some(code.into());
                    }
                }
            }
        }
    }

    // 릴레이의 주소: 열려 있고 공인 주소가 있으면 올리고, 아니면 (전에 올렸을 때) 내린다
    let target = relay.target().await;
    let address = settings.address.trim();
    if let (true, Some(sha), Some(t)) = (status.listening && !address.is_empty(), status.cert_sha256.as_deref(), target.as_ref()) {
        status.published = relay::publish(t, address, settings.port, sha).await;
        if let (false, Some((cancel, _))) = (status.published, rt.cancel.as_ref()) {
            // 부팅 직후 네트워크가 아직이거나 릴레이가 잠시 받지 않으면, 포트가 열려 있는 동안 다시 올린다
            tracing::warn!("Failed to publish the remote daemon address to the relay; retrying");
            let (cancel, address, sha) = (cancel.clone(), address.to_string(), sha.to_string());
            let port = settings.port;
            tokio::spawn(async move {
                if publish_with_retry(&relay, &address, port, &sha, &cancel, std::time::Duration::from_secs(5)).await {
                    let mut rt = RUNTIME.lock().await;
                    // 그사이 설정을 다시 적용했으면 그쪽 상태를 덮지 않는다 (다시 적용은 잠근 채 예전 것을 취소한다)
                    if !cancel.is_cancelled() {
                        rt.published = true;
                        rt.status.published = true;
                        tracing::info!("Published the remote daemon address to the relay");
                    }
                }
            });
        }
    } else if let (true, Some(t)) = (rt.published || !address.is_empty(), target.as_ref()) {
        // 데몬이 다시 뜬 뒤에는 전에 올렸는지 모르므로, 주소를 적어 둔 PC면 내린다
        if !relay::withdraw(t).await {
            tracing::warn!("Failed to withdraw the remote daemon address from the relay");
        }
    }
    rt.published = status.published;
    rt.status = status.clone();
    status
}

#[cfg(test)]
mod tests {
    use std::sync::atomic::{AtomicUsize, Ordering};
    use std::time::Duration;

    /// 릴레이 흉내: 처음 `fail`번은 503, 그 뒤로 204
    async fn flaky_relay(fail: usize) -> (super::server::RelaySource, super::Arc<AtomicUsize>) {
        let calls = super::Arc::new(AtomicUsize::new(0));
        let seen = calls.clone();
        let app = axum::Router::new().route(
            "/api/remote/endpoint",
            axum::routing::put(move || {
                let n = seen.fetch_add(1, Ordering::SeqCst);
                async move { if n < fail { axum::http::StatusCode::SERVICE_UNAVAILABLE } else { axum::http::StatusCode::NO_CONTENT } }
            }),
        );
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let base = format!("http://{}", listener.local_addr().unwrap());
        tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });
        (super::server::RelaySource::Fixed(super::relay::Target { base, token: "sbn_host-b.secret".into() }), calls)
    }

    /// 부팅 직후처럼 릴레이가 잠시 받지 않아도, 포트가 열려 있는 동안 다시 올려 결국 올린다
    #[tokio::test]
    async fn a_failed_publish_is_retried_until_the_relay_takes_it() {
        let (relay, calls) = flaky_relay(2).await;
        let cancel = super::CancellationToken::new();
        let published = super::publish_with_retry(&relay, "saba.example.com", 57480, &"ab".repeat(32), &cancel, Duration::from_millis(10)).await;
        assert!(published);
        assert_eq!(calls.load(Ordering::SeqCst), 3);
    }

    /// 포트를 닫거나 설정을 다시 적용하면 다시 올리기를 멈춘다
    #[tokio::test]
    async fn retrying_stops_once_the_port_closes() {
        let (relay, calls) = flaky_relay(usize::MAX).await;
        let cancel = super::CancellationToken::new();
        let stop = cancel.clone();
        tokio::spawn(async move {
            tokio::time::sleep(Duration::from_millis(100)).await;
            stop.cancel();
        });
        let published = super::publish_with_retry(&relay, "saba.example.com", 57480, &"ab".repeat(32), &cancel, Duration::from_millis(10)).await;
        assert!(!published);
        let after = calls.load(Ordering::SeqCst);
        tokio::time::sleep(Duration::from_millis(100)).await;
        assert_eq!(calls.load(Ordering::SeqCst), after, "no more attempts after the port closed");
    }

    /// 설정을 바꿔 같은 포트로 다시 열 때: 예전 리스너가 끝나면 포트가 바로 비어 있다
    #[tokio::test]
    async fn the_port_is_free_again_once_the_listener_stops() {
        let dir = tempfile::tempdir().unwrap();
        let tls = super::tls::Identity::load_or_create(dir.path()).unwrap().server_config().unwrap();
        let listener = super::bind(0).unwrap();
        let port = listener.local_addr().unwrap().port();
        assert!(super::bind(port).is_err(), "the port is taken while listening");
        let cancel = super::CancellationToken::new();
        let task = tokio::spawn(super::server::serve(listener, super::Arc::new(tls), axum::Router::new(), cancel.clone()));
        cancel.cancel();
        task.await.unwrap();
        super::bind(port).expect("rebind after stop");
    }

    #[test]
    fn addresses_follow_the_relay_rule() {
        for ok in ["saba.example.com", "203.0.113.7", "2001:db8::1", "localhost", "a-b.c"] {
            assert!(super::valid_address(ok), "{ok}");
        }
        for bad in ["", "evil.com/x", "user@host", "a b", "-bad.example.com", "bad-.x", "a..b", "h:7443"] {
            assert!(!super::valid_address(bad), "{bad}");
        }
    }
}
