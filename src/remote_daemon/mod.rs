//! 원격 데몬 (D1, 05 §4.4 (j)): 같은 주인의 다른 PC가 이 데몬에 TLS로 직접 붙는 원격 포트.
//!
//! 설정(config.json `remote`)이 켜져 있으면 자체 서명 인증서로 원격 포트를 열고, 공인 주소와 인증서 지문을 릴레이에 올린다.
//! 붙는 PC는 릴레이에서 받은 연결표로 한 번 짝을 맺고(`/remote/pair`), 그 뒤로는 받은 토큰으로 데몬 API를 부른다.
//! 공인 주소가 없으면 주소 없이 올리고, 붙는 PC는 릴레이 터널로 붙는다 (T1, `tunnel.rs`)

pub mod config;
pub mod onboard;
pub mod peer;
pub mod relay;
pub mod server;
pub mod target;
pub mod tls;
pub mod tunnel;

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, OnceLock};

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
    /// 실패한 주소 올리기를 다시 하는 작업: 그 작업만 멈추는 취소(리스너 취소의 자식), 작업, 그 작업이 올렸는지
    retry: Option<(CancellationToken, tokio::task::JoinHandle<()>, Arc<AtomicBool>)>,
    published: bool,
    /// 릴레이에 올린 주소: 설정한 주소, 비었으면 릴레이가 본 이 PC의 공인 IP (`None`: 주소 없이, 터널로만)
    address: Option<String>,
    status: Status,
}

static RUNTIME: tokio::sync::Mutex<Runtime> = tokio::sync::Mutex::const_new(Runtime {
    cancel: None,
    retry: None,
    published: false,
    address: None,
    status: Status { listening: false, cert_sha256: None, published: false, error: None },
});

/// 릴레이에 올릴 직접 연결 주소. 설정한 주소가 먼저이고, 비었으면 릴레이가 하트비트 답으로 알려 준 이 PC의 공인 IP다
/// (사용자 결정 2026-10-25·2026-11-08: 릴레이 터널은 처음 짝 맺을 때와 직접 닿지 않을 때만 쓴다).
/// 직접 닿는지는 붙는 PC가 붙어 보고 정한다 (`peer::send_to`)
fn published_address(settings: &config::Settings, server: &IPCServer) -> Option<String> {
    public_address(settings, server.relay_agent.seen_address()).0
}

/// 이 PC의 공개 주소: 정한 주소(도메인·IP)가 먼저이고, 비었으면 `seen`(릴레이가 본 공인 IP). 둘째 값은 정한 주소인가.
/// 원격 데몬 직접 연결과 게임 서버 주소(`/api/instance/:id/address`, 포트 확인)가 함께 쓴다 (사용자 결정 2026-11-10)
pub fn public_address(settings: &config::Settings, seen: Option<String>) -> (Option<String>, bool) {
    match configured_address(settings) {
        Some(a) => (Some(a), true),
        None => (seen, false),
    }
}

/// 정한 공개 주소 (비었으면 없다)
pub fn configured_address(settings: &config::Settings) -> Option<String> {
    Some(settings.address.trim().to_string()).filter(|a| !a.is_empty())
}

/// 릴레이가 본 공인 IP가 바뀌었을 때(첫 하트비트 포함, `relay_agent`): 설정한 주소가 없고 원격 포트가 열려 있으면 새 IP를 올린다
pub async fn on_seen_address(server: &IPCServer) {
    let settings = config::settings_in(&ConfigFile::default_location());
    let mut rt = RUNTIME.lock().await;
    let address = published_address(&settings, server);
    publish_changed(&mut rt, &server::RelaySource::Configured(server.clone()), address, settings.port, RETRY_FIRST).await;
}

/// 원격 포트가 열려 있고 올릴 주소가 바뀌었으면 새 주소를 올린다. 실패하면 `retry_after` 뒤부터 다시 올린다
async fn publish_changed(rt: &mut Runtime, relay: &server::RelaySource, address: Option<String>, port: u16, retry_after: std::time::Duration) {
    let (true, Some(sha)) = (rt.status.listening, rt.status.cert_sha256.clone()) else { return };
    if address == rt.address {
        return;
    }
    let Some(target) = relay.target().await else { return };
    // 다시 올리던 작업은 예전 주소를 쥐고 있다: 멈추고, 보내던 요청이 끝난 뒤에 새 주소를 올린다 (늦게 닿은 예전 주소가 덮지 않게)
    if let Some((cancel, task, done)) = rt.retry.take() {
        cancel.cancel();
        let _ = task.await;
        if done.load(Ordering::SeqCst) {
            rt.published = true;
            rt.status.published = true;
        }
    }
    match relay::publish(&target, address.as_deref(), port, &sha).await {
        relay::Published::Done => {
            tracing::info!("Published the remote daemon address the relay sees");
            rt.address = address;
            rt.published = true;
            rt.status.published = true;
        }
        relay::Published::Failed => {
            rt.address = address.clone();
            retry_publishing(rt, relay.clone(), address, port, sha, retry_after);
        }
        relay::Published::Refused | relay::Published::Unauthorized => {}
    }
}

/// 실패한 주소 올리기를 포트가 열려 있는 동안 다시 하는 작업을 띄운다
fn retry_publishing(rt: &mut Runtime, relay: server::RelaySource, address: Option<String>, port: u16, sha: String, first: std::time::Duration) {
    let Some((listener, _)) = rt.cancel.as_ref() else { return };
    let cancel = listener.child_token();
    let done = Arc::new(AtomicBool::new(false));
    let (stop, flag) = (cancel.clone(), done.clone());
    let task = tokio::spawn(async move {
        if publish_with_retry(&relay, address.as_deref(), port, &sha, &stop, first).await {
            flag.store(true, Ordering::SeqCst);
            tracing::info!("Published the remote daemon address to the relay");
        }
    });
    rt.retry = Some((cancel, task, done));
}

/// 릴레이의 내 PC 목록에 맞춘다 (`GET /api/remote/peers`를 읽을 때와 10분마다):
/// - 저장한 이름·주인 이름을 릴레이 것으로
/// - 서버쨩에서 빠진 PC(디스코드에서 지웠거나 개인정보를 지운 PC)는 짝을 푼다. 그 PC가 가진 토큰은 더는 통하지 않는다 (사용자 결정 2026-11-08)
/// - 저장한 원격 데몬의 주소·지문이 바뀌었으면 따른다 (터널로 짝 맺은 PC가 공인 주소를 올린 뒤 직접 붙게)
/// - 이 PC 이름을 받아 둔다 (`config::this_name_in`, 옛 릴레이는 주지 않는다)
pub async fn sync_with_relay(file: &ConfigFile, target: &relay::Target) -> Result<relay::PeerList, relay::Failure> {
    let list = relay::peer_list(target).await?;
    if let Some(me) = &list.this_pc {
        if let Err(e) = config::save_this_name_in(file, &me.name) {
            tracing::warn!("Failed to remember this PC's name: {}", e);
        }
    }
    let peers = &list.peers;
    let known: Vec<_> = peers.iter().map(|p| (p.host_id.clone(), p.name.clone(), p.owner_name.clone())).collect();
    if let Err(e) = config::refresh_names_in(file, &known) {
        tracing::warn!("Failed to refresh remote PC names: {}", e);
    }
    for gone in config::clients_in(file).into_iter().filter(|c| !peers.iter().any(|p| p.host_id == c.host_id)) {
        match config::revoke_in(file, &gone.host_id) {
            Ok(_) => tracing::info!("Unpaired remote PC {}: it is no longer one of the owner's PCs", gone.host_id),
            Err(e) => tracing::warn!("Failed to unpair remote PC {}: {}", gone.host_id, e),
        }
    }
    for d in config::daemons_in(file) {
        let Some(ep) = peers.iter().find(|p| p.host_id == d.host_id).and_then(|p| p.endpoint.as_ref()) else { continue };
        if (ep.address.as_str(), ep.port, ep.cert_sha256.as_str()) != (d.address.as_str(), d.port, d.cert_sha256.as_str()) {
            let _ = config::update_endpoint_in(file, &d.host_id, &ep.address, ep.port, &ep.cert_sha256);
        }
    }
    // 원격 포트가 들고 있는 짝 맺은 PC도 (새 이름, 푼 짝)
    reload_clients(file);
    Ok(list)
}

/// 데몬이 떠 있는 동안 10분마다 릴레이의 내 PC 목록에 맞춘다 (`sync_with_relay`)
pub fn spawn_sync(server: IPCServer) {
    tokio::spawn(async move {
        let mut wait = std::time::Duration::from_secs(60);
        loop {
            tokio::select! {
                _ = server.shutdown_token.cancelled() => return,
                _ = tokio::time::sleep(wait) => {}
            }
            wait = std::time::Duration::from_secs(600);
            if let Some(target) = server::RelaySource::Configured(server.clone()).target().await {
                let _ = sync_with_relay(&ConfigFile::default_location(), &target).await;
            }
        }
    });
}

/// 짝 맺은 PC 목록 (원격 포트의 인증층이 요청마다 본다). 설정 파일에서 채우고, 짝 맺기·풀기가 고친다
pub(crate) fn clients() -> server::Clients {
    static CLIENTS: OnceLock<server::Clients> = OnceLock::new();
    CLIENTS.get_or_init(server::Clients::default).clone()
}

/// 짝을 푼 PC의 열린 스트림도 여기서 끝난다 (`Clients::set`)
pub(crate) fn reload_clients(file: &ConfigFile) {
    clients().reload(file);
}

pub async fn status() -> Status {
    let rt = RUNTIME.lock().await;
    let mut status = rt.status.clone();
    status.published |= rt.retry.as_ref().is_some_and(|(_, _, done)| done.load(Ordering::SeqCst));
    status
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

/// 두 겹(IPv6·IPv4)으로 받는다. IPv6가 없으면 IPv4만.
/// 시험은 루프백에만 묶는다: 모든 주소에 묶으면 Windows가 빌드마다 이름이 바뀌는 시험 바이너리마다 방화벽 허용 창을 띄운다
fn bind(port: u16) -> std::io::Result<tokio::net::TcpListener> {
    use std::net::{Ipv4Addr, Ipv6Addr};
    let (v6, v4) = if cfg!(test) { (Ipv6Addr::LOCALHOST, Ipv4Addr::LOCALHOST) } else { (Ipv6Addr::UNSPECIFIED, Ipv4Addr::UNSPECIFIED) };
    let std_listener = match listen_on((v6, port).into(), true) {
        Ok(l) => l,
        Err(e) if e.kind() == std::io::ErrorKind::AddrInUse => return Err(e),
        Err(_) => listen_on((v4, port).into(), false)?,
    };
    tokio::net::TcpListener::from_std(std_listener)
}

/// 올리기에 실패한 뒤 처음 다시 올릴 때까지의 간격과, 간격의 상한
const RETRY_FIRST: std::time::Duration = std::time::Duration::from_secs(5);
const RETRY_MAX: std::time::Duration = std::time::Duration::from_secs(300);

/// `first`만큼 기다렸다가 주소를 올린다. 실패하면 간격을 두 배씩 늘려(상한 5분) 올릴 때까지 다시 한다.
/// 올렸으면 true. `cancel`(포트를 닫거나 설정을 다시 적용함)이면 기다림 중에 멈추고 false. 릴레이가 받지 않거나(`Refused`)
/// 노드 토큰을 거절하면(`Unauthorized`, 토큰이 바뀌면 `apply`가 다시 올린다) 그만두고 false.
/// 보내는 중인 요청은 끝까지 기다린다: 다시 적용하는 쪽이 이 작업이 끝난 뒤에 새 주소를 올리도록 (늦게 닿은 예전 주소가 덮지 않게)
async fn publish_with_retry(
    relay: &server::RelaySource,
    address: Option<&str>,
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
            match relay::publish(&t, address, port, sha).await {
                relay::Published::Done => return true,
                relay::Published::Refused | relay::Published::Unauthorized => return false,
                relay::Published::Failed => {}
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
    // 다시 올리기도 같은 취소로 멈춘다. 보내던 요청이 끝나야 새 주소를 올린다
    let mut was_published = rt.published;
    if let Some((_, task, done)) = rt.retry.take() {
        let _ = task.await;
        was_published |= done.load(Ordering::SeqCst);
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

    // 릴레이의 주소: 열려 있으면 올리고(공인 주소가 없으면 주소 없이, 터널로만), 아니면 (전에 올렸을 때) 내린다
    let target = relay.target().await;
    let address = published_address(&settings, server);
    let address = address.as_deref();
    if let (true, Some(sha), Some(t)) = (status.listening, status.cert_sha256.as_deref(), target.as_ref()) {
        let published = relay::publish(t, address, settings.port, sha).await;
        status.published = published == relay::Published::Done;
        if published == relay::Published::Refused {
            tracing::warn!("The relay refused the remote daemon endpoint");
        }
        if published == relay::Published::Unauthorized {
            tracing::warn!("The relay rejected the node token; not publishing the remote daemon address");
        }
        if published == relay::Published::Failed {
            // 부팅 직후 네트워크가 아직이거나 릴레이가 잠시 받지 않으면, 포트가 열려 있는 동안 다시 올린다
            tracing::warn!("Failed to publish the remote daemon address to the relay; retrying");
            retry_publishing(&mut rt, relay, address.map(str::to_string), settings.port, sha.to_string(), RETRY_FIRST);
        }
    } else if let (true, Some(t)) = (was_published || tls::exists(&saba_chan_updater_lib::constants::resolve_data_dir()), target.as_ref()) {
        // 데몬이 다시 뜬 뒤에는 전에 올렸는지 모르므로, 원격 포트를 연 적이 있는(인증서가 있는) PC면 내린다
        if !relay::withdraw(t).await {
            tracing::warn!("Failed to withdraw the remote daemon address from the relay");
        }
    }
    rt.published = status.published;
    rt.address = address.map(str::to_string);
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
        let published = super::publish_with_retry(&relay, Some("saba.example.com"), 57480, &"ab".repeat(32), &cancel, Duration::from_millis(10)).await;
        assert!(published);
        assert_eq!(calls.load(Ordering::SeqCst), 3);
    }

    /// 릴레이가 본문을 받지 않으면(400) 같은 본문을 다시 보내지 않는다
    #[tokio::test]
    async fn a_relay_that_refuses_the_endpoint_is_not_asked_again() {
        let calls = super::Arc::new(AtomicUsize::new(0));
        let seen = calls.clone();
        let app = axum::Router::new().route(
            "/api/remote/endpoint",
            axum::routing::put(move |axum::Json(body): axum::Json<serde_json::Value>| {
                seen.fetch_add(1, Ordering::SeqCst);
                async move { if body["address"].is_null() { axum::http::StatusCode::BAD_REQUEST } else { axum::http::StatusCode::NO_CONTENT } }
            }),
        );
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let base = format!("http://{}", listener.local_addr().unwrap());
        tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });
        let relay = super::server::RelaySource::Fixed(super::relay::Target { base, token: "sbn_host-b.secret".into() });
        let cancel = super::CancellationToken::new();
        let published = super::publish_with_retry(&relay, None, 57480, &"ab".repeat(32), &cancel, Duration::from_millis(10)).await;
        assert!(!published);
        assert_eq!(calls.load(Ordering::SeqCst), 1);
    }

    /// 릴레이가 이 노드 토큰을 받지 않으면(401, 같은 계정의 다른 PC가 페어링해 토큰이 대체됨 등) 다시 보내지 않는다.
    /// 같은 토큰으로는 계속 거절되고, 토큰이 바뀌면 apply가 다시 올린다
    #[tokio::test]
    async fn a_relay_that_rejects_the_node_token_is_not_asked_again() {
        let calls = super::Arc::new(AtomicUsize::new(0));
        let seen = calls.clone();
        let app = axum::Router::new().route(
            "/api/remote/endpoint",
            axum::routing::put(move || {
                seen.fetch_add(1, Ordering::SeqCst);
                async { (axum::http::StatusCode::UNAUTHORIZED, axum::Json(serde_json::json!({"error": "TOKEN_REPLACED"}))) }
            }),
        );
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let base = format!("http://{}", listener.local_addr().unwrap());
        tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });
        let relay = super::server::RelaySource::Fixed(super::relay::Target { base, token: "sbn_host-b.secret".into() });
        let cancel = super::CancellationToken::new();
        let published = super::publish_with_retry(&relay, Some("saba.example.com"), 57480, &"ab".repeat(32), &cancel, Duration::from_millis(10)).await;
        assert!(!published);
        assert_eq!(calls.load(Ordering::SeqCst), 1);
    }

    /// 부팅 직후 주소 없이 올리려다 실패해 다시 올리는 중에 릴레이가 본 공인 IP를 알게 되면, 나중 주소가 남는다:
    /// 다시 올리던 작업이 쥐고 있던 예전 주소(없음)가 새 주소를 덮지 않는다
    #[tokio::test]
    async fn an_address_learned_while_retrying_is_not_overwritten_by_the_stale_retry() {
        let up = super::Arc::new(std::sync::atomic::AtomicBool::new(false));
        let last = super::Arc::new(std::sync::Mutex::new(None));
        let (online, record) = (up.clone(), last.clone());
        let app = axum::Router::new().route(
            "/api/remote/endpoint",
            axum::routing::put(move |axum::Json(body): axum::Json<serde_json::Value>| {
                let ok = online.load(Ordering::SeqCst);
                if ok {
                    *record.lock().unwrap() = Some(body["address"].clone());
                }
                async move { if ok { axum::http::StatusCode::NO_CONTENT } else { axum::http::StatusCode::SERVICE_UNAVAILABLE } }
            }),
        );
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let base = format!("http://{}", listener.local_addr().unwrap());
        tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });
        let relay = super::server::RelaySource::Fixed(super::relay::Target { base, token: "sbn_host-b.secret".into() });

        let mut rt = super::Runtime {
            cancel: Some((super::CancellationToken::new(), tokio::spawn(async {}))),
            status: super::Status { listening: true, cert_sha256: Some("ab".repeat(32)), ..Default::default() },
            ..Default::default()
        };
        super::retry_publishing(&mut rt, relay.clone(), None, 57480, "ab".repeat(32), Duration::from_millis(150));
        up.store(true, Ordering::SeqCst);
        super::publish_changed(&mut rt, &relay, Some("203.0.113.7".into()), 57480, Duration::from_millis(150)).await;
        tokio::time::sleep(Duration::from_millis(400)).await;
        assert_eq!(*last.lock().unwrap(), Some(serde_json::json!("203.0.113.7")), "the later address is what the relay keeps");
        assert_eq!(rt.address.as_deref(), Some("203.0.113.7"));
        assert!(rt.status.published);
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
        let published = super::publish_with_retry(&relay, Some("saba.example.com"), 57480, &"ab".repeat(32), &cancel, Duration::from_millis(10)).await;
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

    /// 다시 적용(취소)할 때 보내던 요청은 끝까지 기다린다: 뒤에 올리는 새 주소를 늦게 닿은 예전 주소가 덮지 않게
    #[tokio::test]
    async fn a_cancelled_retry_finishes_the_request_it_was_sending() {
        let handled = super::Arc::new(std::sync::atomic::AtomicBool::new(false));
        let seen = handled.clone();
        let app = axum::Router::new().route(
            "/api/remote/endpoint",
            axum::routing::put(move || {
                let seen = seen.clone();
                async move {
                    tokio::time::sleep(Duration::from_millis(300)).await;
                    seen.store(true, Ordering::SeqCst);
                    axum::http::StatusCode::NO_CONTENT
                }
            }),
        );
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let base = format!("http://{}", listener.local_addr().unwrap());
        tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });
        let relay = super::server::RelaySource::Fixed(super::relay::Target { base, token: "sbn_host-b.secret".into() });
        let cancel = super::CancellationToken::new();
        let stop = cancel.clone();
        tokio::spawn(async move {
            tokio::time::sleep(Duration::from_millis(100)).await;
            stop.cancel();
        });
        let published = super::publish_with_retry(&relay, Some("saba.example.com"), 57480, &"ab".repeat(32), &cancel, Duration::from_millis(10)).await;
        assert!(handled.load(Ordering::SeqCst), "returns only after the relay handled the request");
        assert!(published);
    }

    /// 시험의 원격 포트는 루프백에만 묶는다 (Windows가 시험 바이너리마다 방화벽 허용 창을 띄우지 않게)
    #[tokio::test]
    async fn tests_bind_the_remote_port_to_loopback_only() {
        let listener = super::bind(0).unwrap();
        assert!(listener.local_addr().unwrap().ip().is_loopback());
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
