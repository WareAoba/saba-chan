//! 원격 데몬 (D1, 05 §4.4 (j)): 같은 주인의 다른 PC가 이 데몬에 TLS로 직접 붙는 원격 포트.
//!
//! 설정(config.json `remote`)이 켜져 있으면 자체 서명 인증서로 원격 포트를 열고, 공인 주소와 인증서 지문을 릴레이에 올린다.
//! 붙는 PC는 릴레이에서 받은 연결표로 한 번 짝을 맺고(`/remote/pair`), 그 뒤로는 받은 토큰으로 데몬 API를 부른다.
//! 공인 주소가 없으면 주소 없이 올리고, 붙는 PC는 릴레이 터널로 붙는다 (T1, `tunnel.rs`)

pub mod config;
mod h2;
pub mod onboard;
pub mod peer;
mod publisher;
pub mod relay;
pub mod server;
pub mod target;
pub mod tls;
pub mod tunnel;

use std::sync::Arc;

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

/// 원격 데몬의 실행 상태 (`IPCServer`가 하나 쥔다): 원격 포트, 릴레이에 주소 올리기, 짝 맺은 PC, 붙는 쪽 연결.
/// 복제는 같은 상태를 가리킨다
#[derive(Clone, Default)]
pub struct RemoteDaemon {
    port: Arc<tokio::sync::Mutex<Port>>,
    publisher: publisher::Publisher,
    /// 짝 맺은 PC 목록 (원격 포트의 인증층이 요청마다 본다). 설정 파일에서 채우고(`Clients::reload`), 짝 맺기·풀기가 고친다.
    /// 짝을 푼 PC의 열린 스트림도 다시 읽을 때 끝난다
    pub(crate) clients: server::Clients,
    /// 붙는 쪽: 원격 데몬마다의 연결과 터널
    pub peers: peer::Pool,
}

/// 열어 둔 원격 포트: 리스너를 멈추는 취소와 그 작업, 마지막으로 적용한 결과 (`published`는 `Publisher`가 안다)
#[derive(Default)]
struct Port {
    listener: Option<(CancellationToken, tokio::task::JoinHandle<()>)>,
    status: Status,
}

/// 릴레이에 올릴 직접 연결 주소. 설정한 주소가 먼저이고, 비었으면 릴레이가 하트비트 답으로 알려 준 이 PC의 공인 IP다
/// (사용자 결정 2026-10-25·2026-11-08: 릴레이 터널은 처음 짝 맺을 때와 직접 닿지 않을 때만 쓴다).
/// 직접 닿는지는 붙는 PC가 붙어 보고 정한다 (`peer::Pool::send_to`)
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

/// 릴레이가 본 공인 IP나 정한 공개 주소가 바뀌었을 때(첫 하트비트 포함): 원격 포트가 열려 있으면 새 주소를 올린다.
/// 올리기는 뒤에서 한다 (`publisher`)
pub fn on_seen_address(server: &IPCServer) {
    let settings = config::settings_in(&ConfigFile::default_location());
    server.remote_daemon.publisher.readdress(published_address(&settings, server));
}

impl RemoteDaemon {
    pub async fn status(&self) -> Status {
        Status { published: self.publisher.published(), ..self.port.lock().await.status.clone() }
    }

    /// 릴레이의 내 PC 목록에 맞춘다 (`GET /api/remote/peers`를 읽을 때와 10분마다):
    /// - 저장한 이름·주인 이름을 릴레이 것으로
    /// - 서버쨩에서 빠진 PC(디스코드에서 지웠거나 개인정보를 지운 PC)는 짝을 푼다. 그 PC가 가진 토큰은 더는 통하지 않는다 (사용자 결정 2026-11-08)
    /// - 저장한 원격 데몬의 주소·지문이 바뀌었으면 따른다 (터널로 짝 맺은 PC가 공인 주소를 올린 뒤 직접 붙게)
    /// - 이 PC 이름을 받아 둔다 (`config::this_name_in`, 옛 릴레이는 주지 않는다)
    pub async fn sync_with_relay(&self, file: &ConfigFile, target: &relay::Target) -> Result<relay::PeerList, relay::Failure> {
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
        self.clients.reload(file);
        Ok(list)
    }
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
                let _ = server.remote_daemon.sync_with_relay(&ConfigFile::default_location(), &target).await;
            }
        }
    });
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

/// 설정대로 원격 포트를 연다·닫는다·다시 연다. 설정을 바꾼 뒤, 데몬이 뜰 때, 노드 토큰이 바뀔 때 부른다
pub async fn apply(server: &IPCServer) -> Status {
    let remote = &server.remote_daemon;
    let file = ConfigFile::default_location();
    let settings = config::settings_in(&file);
    let relay = server::RelaySource::Configured(server.clone());
    let mut port = remote.port.lock().await;
    // 예전 리스너가 포트를 놓을 때까지 기다린다 (같은 포트로 다시 열 때)
    if let Some((cancel, task)) = port.listener.take() {
        cancel.cancel();
        let _ = task.await;
    }
    remote.clients.reload(&file);
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
                        let ctx = server::Ctx::new(file.clone(), remote.clients.clone(), relay.clone());
                        let app = server::router(server.routes(), ctx);
                        let task = tokio::spawn(server::serve(listener, Arc::new(tls), app, cancel.clone()));
                        port.listener = Some((cancel, task));
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

    // 릴레이의 주소: 열려 있으면 올리고(공인 주소가 없으면 주소 없이, 터널로만), 아니면 (전에 올렸을 때) 내린다.
    // 데몬이 다시 뜬 뒤에는 전에 올렸는지 모르므로, 원격 포트를 연 적이 있는(인증서가 있는) PC면 내린다
    let endpoint = match (status.listening, &status.cert_sha256) {
        (true, Some(sha)) => Some(publisher::Endpoint { address: published_address(&settings, server), port: settings.port, cert_sha256: sha.clone() }),
        _ => None,
    };
    let withdraw = tls::exists(&saba_chan_updater_lib::constants::resolve_data_dir());
    status.published = remote.publisher.apply(&relay, &server.shutdown_token, endpoint, withdraw).await;
    port.status = status.clone();
    status
}

#[cfg(test)]
mod tests {
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
