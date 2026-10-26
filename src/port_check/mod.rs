//! 포트포워딩 확인 (사용자 결정 2026-10-26): 친구가 밖에서 이 PC의 게임 서버에 닿는지 릴레이가 대신 접속해 본다
//!
//! 확인할 포트는 모듈이 `[[settings.fields]] public = "tcp" | "udp" | "both"`로 선언한 것뿐이다 (게임 이름으로 나누지 않는다).
//! 1. 서버가 꺼져 있으면 그 포트에 잠깐 시험 리스너를 연다. 켜져 있으면 게임이 연 포트를 그대로 쓴다.
//! 2. 릴레이 `POST /api/port-check`에 일회용 확인값(nonce)과 포트를 보낸다. 릴레이는 요청이 온 공인 주소의 그 포트로만 보낸다.
//! 3. 시험 리스너가 확인값을 받았으면 열림. 켜진 서버의 TCP는 릴레이의 접속 결과로, 켜진 서버의 UDP는 알 수 없음(서버를 끄면 확인된다).
//!
//! 공유기가 공인 주소를 받지 못한 경우(CGNAT)는 UPnP로 공유기가 아는 바깥 주소를 물어 릴레이가 본 주소와 견준다.
//! 시험 리스너를 여는 동안 같은 인스턴스의 시작은 기다리고, 시작하는 중인 인스턴스의 포트는 잡지 않는다 (`begin_start`). 결과는 인스턴스마다 마지막 한 번을 메모리에 둔다.

use crate::instance::ServerInstance;
use crate::supervisor::module_loader::ModuleSettings;
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use std::collections::{HashMap, HashSet};
use std::net::{IpAddr, Ipv4Addr, SocketAddr};
use std::sync::{Arc, Mutex};
use std::time::Duration;
use tokio::io::AsyncReadExt;
use tokio_util::sync::CancellationToken;

/// 확인값을 보낸 뒤 시험 리스너가 받기를 기다리는 시간 (릴레이는 TCP를 다 보낸 뒤에 답한다)
const RECEIVE_GRACE: Duration = if cfg!(test) { Duration::from_millis(300) } else { Duration::from_millis(1500) };
/// 릴레이 요청 시간 (릴레이의 TCP 접속 한도 3초 + 여유)
const RELAY_TIMEOUT: Duration = Duration::from_secs(10);
/// 한 번에 확인하는 포트 수 (릴레이 `MAX_PORTS`와 같다)
pub const MAX_PORTS: usize = 16;
/// 데몬이 켜진 뒤 자동 확인까지 (자동 시작하는 서버가 먼저 포트를 열게)
const BOOT_DELAY: Duration = Duration::from_secs(90);

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Protocol {
    Tcp,
    Udp,
}

/// module.toml 설정 필드의 `public` 값 → 프로토콜
pub fn protocols_of(public: &str) -> Option<&'static [Protocol]> {
    match public {
        "tcp" => Some(&[Protocol::Tcp]),
        "udp" => Some(&[Protocol::Udp]),
        "both" => Some(&[Protocol::Tcp, Protocol::Udp]),
        _ => None,
    }
}

/// 밖에 여는 포트 하나
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PublicPort {
    /// 설정 필드 이름
    pub field: String,
    pub port: u16,
    pub protocol: Protocol,
}

fn as_port(v: &Value) -> Option<u16> {
    match v {
        Value::Number(n) => n.as_u64(),
        Value::String(s) => s.trim().parse().ok(),
        _ => None,
    }
    .and_then(|p| u16::try_from(p).ok())
    .filter(|p| *p > 0)
}

/// 이 인스턴스가 밖에 여는 포트: `public`을 선언한 설정 필드의 값 (인스턴스 값, 없으면 모듈 기본값)
pub fn public_ports(instance: &ServerInstance, settings: Option<&ModuleSettings>) -> Vec<PublicPort> {
    let mut out = Vec::new();
    for field in settings.map(|s| s.fields.as_slice()).unwrap_or_default() {
        let Some(protocols) = field.public.as_deref().and_then(protocols_of) else { continue };
        let core = match field.name.as_str() {
            "port" => instance.port,
            "rest_port" => instance.rest_port,
            _ => None,
        };
        let value = core
            .or_else(|| instance.module_settings.get(&field.name).and_then(as_port))
            .or_else(|| field.default.as_ref().and_then(|d| serde_json::to_value(d).ok()).as_ref().and_then(as_port));
        let Some(port) = value else { continue };
        for &protocol in protocols {
            out.push(PublicPort { field: field.name.clone(), port, protocol });
        }
    }
    out
}

/// 확인할 인스턴스 하나
#[derive(Debug, Clone)]
pub struct Target {
    pub instance_id: String,
    pub running: bool,
    pub ports: Vec<PublicPort>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum PortStatus {
    Open,
    Closed,
    /// 켜진 서버의 UDP: 게임이 답하지 않아 알 수 없다 (서버를 끄면 확인된다). 릴레이가 공인 주소를 보지 못했을 때도
    Unknown,
    /// 서버가 꺼져 있는데 다른 프로그램이 그 포트를 쓰고 있다
    InUse,
    /// 닿지 않았고, 이 PC 방화벽(Windows)에 사바쨩을 허용하는 규칙이 없다
    Firewall,
}

#[derive(Debug, Clone, Serialize)]
pub struct PortResult {
    pub field: String,
    pub port: u16,
    pub protocol: Protocol,
    pub status: PortStatus,
}

/// 공유기와 공인 주소
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum Nat {
    /// 이 PC에 공인 주소가 바로 있다
    Direct,
    /// 공유기가 공인 주소를 받았다 (포트포워딩하면 된다)
    Router,
    /// 공유기가 받은 바깥 주소가 공인 주소가 아니다: 통신사 공유 주소(CGNAT)라 포트포워딩으로 열 수 없다
    Cgnat,
    /// 공유기가 UPnP에 답하지 않아 모른다
    Unknown,
}

/// 인터넷에서 닿는 IPv4 유니캐스트 주소인가 (CGNAT 공유 대역 100.64.0.0/10도 아니다)
fn is_public_v4(ip: Ipv4Addr) -> bool {
    let o = ip.octets();
    !(ip.is_private()
        || ip.is_loopback()
        || ip.is_link_local()
        || ip.is_broadcast()
        || ip.is_documentation()
        || ip.is_unspecified()
        || ip.is_multicast()
        || o[0] == 0
        || o[0] >= 240
        || (o[0] == 100 && (o[1] & 0xc0) == 64)
        || (o[0] == 198 && (o[1] & 0xfe) == 18))
}

/// 릴레이가 본 주소(`observed`), 그 주소로 가는 길의 이 PC 주소(`route_local`), 공유기가 UPnP로 알려 준 바깥 주소(`gateway`)
pub fn classify_nat(observed: IpAddr, route_local: Option<IpAddr>, gateway: Option<IpAddr>) -> Nat {
    if route_local == Some(observed) {
        return Nat::Direct;
    }
    match gateway {
        Some(IpAddr::V4(g)) if !is_public_v4(g) => Nat::Cgnat,
        Some(g) if g != observed => Nat::Cgnat,
        Some(_) => Nat::Router,
        None => Nat::Unknown,
    }
}

/// 인스턴스 하나의 결과
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Report {
    pub instance_id: String,
    /// RFC3339
    pub checked_at: String,
    /// 릴레이가 본 이 PC의 공인 주소
    pub address: Option<String>,
    pub public_address: bool,
    pub nat: Nat,
    pub running: bool,
    /// open: 모두 열림, firewall: 이 PC 방화벽이 막는 포트가 있다, closed: 닫힌 포트가 있다, cgnat: 공유기에 공인 주소가 없다, unknown: 닫힌 것은 없지만 모두 확인하지 못했다,
    /// no_ports: 모듈이 밖에 여는 포트를 선언하지 않았다
    pub verdict: &'static str,
    pub ports: Vec<PortResult>,
}

fn verdict(nat: Nat, ports: &[PortResult]) -> &'static str {
    if ports.is_empty() {
        "no_ports"
    } else if nat == Nat::Cgnat {
        "cgnat"
    } else if ports.iter().any(|p| p.status == PortStatus::Firewall) {
        "firewall"
    } else if ports.iter().any(|p| p.status == PortStatus::Closed) {
        "closed"
    } else if ports.iter().all(|p| p.status == PortStatus::Open) {
        "open"
    } else {
        "unknown"
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum CheckError {
    /// 릴레이에 연결하지 않았다 (노드 토큰 없음)
    NotPaired,
    /// 같은 인스턴스(또는 같은 공인 주소)를 이미 확인하고 있다
    Busy,
    RateLimited,
    /// 릴레이에 닿지 못했거나 알 수 없는 답
    Relay(String),
}

impl CheckError {
    pub fn code(&self) -> &'static str {
        match self {
            CheckError::NotPaired => "node_token_missing",
            CheckError::Busy => "port_check_busy",
            CheckError::RateLimited => "rate_limited",
            CheckError::Relay(_) => "relay_unreachable",
        }
    }
}

/// 릴레이 주소와 서명할 노드 토큰
#[derive(Debug, Clone)]
pub struct Relay {
    pub base: String,
    pub token: String,
}

// ── 시험 리스너를 여는 동안: 같은 인스턴스의 시작을 기다리게 한다 ──

#[derive(Default)]
struct Holds {
    /// 시험 리스너로 포트를 잡고 있는 인스턴스
    checking: HashSet<String>,
    /// 시작하는 중인 인스턴스 (같은 인스턴스의 시작이 둘이면 둘 다 센다)
    starting: HashMap<String, usize>,
}

static HOLDS: Mutex<Option<Holds>> = Mutex::new(None);
static RELEASED: tokio::sync::Notify = tokio::sync::Notify::const_new();

fn with_holds<R>(f: impl FnOnce(&mut Holds) -> R) -> R {
    f(HOLDS.lock().unwrap_or_else(|e| e.into_inner()).get_or_insert_with(Holds::default))
}

/// 확인 중인 인스턴스 묶음. 놓으면(drop) 기다리던 시작이 이어진다
struct Hold(Vec<String>);

impl Hold {
    /// 하나라도 이미 확인 중이면 None. 시작하는 중인 인스턴스를 함께 돌려준다 (그 포트에는 리스너를 열지 않는다)
    fn take(ids: &[String]) -> Option<(Hold, HashSet<String>)> {
        with_holds(|h| {
            if ids.iter().any(|id| h.checking.contains(id)) {
                return None;
            }
            h.checking.extend(ids.iter().cloned());
            let starting = ids.iter().filter(|id| h.starting.contains_key(*id)).cloned().collect();
            Some((Hold(ids.to_vec()), starting))
        })
    }
}

impl Drop for Hold {
    fn drop(&mut self) {
        with_holds(|h| {
            for id in &self.0 {
                h.checking.remove(id);
            }
        });
        RELEASED.notify_waiters();
    }
}

#[cfg(test)]
fn is_held(instance_id: &str) -> bool {
    with_holds(|h| h.checking.contains(instance_id))
}

/// 시작하는 동안 쥔다. 놓기 전에 시작한 확인은 이 인스턴스의 포트에 리스너를 열지 않는다
pub struct Starting(String);

impl Drop for Starting {
    fn drop(&mut self) {
        with_holds(|h| {
            if let Some(n) = h.starting.get_mut(&self.0) {
                *n -= 1;
                if *n == 0 {
                    h.starting.remove(&self.0);
                }
            }
        });
    }
}

/// 시작 경로가 부른다: 이 인스턴스의 포트를 확인 중이면 끝날 때까지(길어야 몇 초) 기다린 뒤 시작한다고 적는다
pub async fn begin_start(instance_id: &str) -> Starting {
    let deadline = tokio::time::Instant::now() + RELAY_TIMEOUT + Duration::from_secs(5);
    loop {
        let notified = RELEASED.notified();
        let entered = with_holds(|h| {
            // 기다림이 너무 길면 그냥 시작한다 (확인보다 시작이 먼저다)
            if h.checking.contains(instance_id) && tokio::time::Instant::now() < deadline {
                return false;
            }
            *h.starting.entry(instance_id.to_string()).or_default() += 1;
            true
        });
        if entered {
            return Starting(instance_id.to_string());
        }
        let _ = tokio::time::timeout_at(deadline, notified).await;
    }
}

// ── 시험 리스너 ──

type Seen = Arc<Mutex<HashSet<(u16, Protocol)>>>;

fn expected_line(nonce: &str) -> Vec<u8> {
    format!("saba-chan-port-check {}\n", nonce).into_bytes()
}

/// 포트마다 리스너를 열고, 확인값을 받으면 `seen`에 적는다. 열지 못한 포트(다른 프로그램이 쓴다)를 돌려준다
async fn open_listeners(
    ports: &[(u16, Protocol)],
    bind_ip: IpAddr,
    nonce: &str,
    seen: &Seen,
    stop: &CancellationToken,
) -> (HashSet<(u16, Protocol)>, Vec<tokio::task::JoinHandle<()>>) {
    let mut busy = HashSet::new();
    let mut tasks = Vec::new();
    let line = Arc::new(expected_line(nonce));
    for &(port, protocol) in ports {
        let addr = SocketAddr::new(bind_ip, port);
        let (seen, stop, line) = (seen.clone(), stop.clone(), line.clone());
        match protocol {
            Protocol::Tcp => match tokio::net::TcpListener::bind(addr).await {
                Ok(listener) => {
                    tasks.push(tokio::spawn(async move {
                        // 받은 연결도 확인이 끝나면 닫는다 (남은 연결이 포트를 잡아 이어지는 서버 시작이 실패하지 않게)
                        let mut conns = tokio::task::JoinSet::new();
                        loop {
                            let (mut sock, _) = tokio::select! {
                                _ = stop.cancelled() => {
                                    conns.shutdown().await;
                                    return;
                                }
                                accepted = listener.accept() => match accepted { Ok(a) => a, Err(_) => continue },
                            };
                            let (seen, line) = (seen.clone(), line.clone());
                            conns.spawn(async move {
                                let mut buf = Vec::with_capacity(64);
                                let read = tokio::time::timeout(Duration::from_secs(3), async {
                                    let mut chunk = [0u8; 64];
                                    while buf.len() < 128 {
                                        match sock.read(&mut chunk).await {
                                            Ok(0) | Err(_) => break,
                                            Ok(n) => buf.extend_from_slice(&chunk[..n]),
                                        }
                                        if buf.ends_with(b"\n") {
                                            break;
                                        }
                                    }
                                });
                                let _ = read.await;
                                if buf == *line {
                                    seen.lock().unwrap_or_else(|e| e.into_inner()).insert((port, Protocol::Tcp));
                                }
                            });
                        }
                    }));
                }
                Err(_) => {
                    busy.insert((port, protocol));
                }
            },
            Protocol::Udp => match tokio::net::UdpSocket::bind(addr).await {
                Ok(socket) => {
                    tasks.push(tokio::spawn(async move {
                        let mut buf = [0u8; 128];
                        loop {
                            let n = tokio::select! {
                                _ = stop.cancelled() => return,
                                got = socket.recv_from(&mut buf) => match got { Ok((n, _)) => n, Err(_) => continue },
                            };
                            if buf[..n] == line[..] {
                                seen.lock().unwrap_or_else(|e| e.into_inner()).insert((port, Protocol::Udp));
                            }
                        }
                    }));
                }
                Err(_) => {
                    busy.insert((port, protocol));
                }
            },
        }
    }
    (busy, tasks)
}

// ── 릴레이 ──

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct RelayAnswer {
    address: String,
    public_address: bool,
    probes: Vec<RelayProbe>,
}

#[derive(Debug, Deserialize)]
struct RelayProbe {
    port: u16,
    protocol: Protocol,
    result: String,
}

/// 공인 IPv4 주소로 나가도록 IPv4에서만 보낸다 (친구는 대개 IPv4로 붙는다). 프록시를 거치면 프록시 주소를 확인하게 되므로 쓰지 않는다
fn relay_client() -> Result<reqwest::Client, CheckError> {
    reqwest::Client::builder()
        .timeout(RELAY_TIMEOUT)
        .no_proxy()
        .local_address(IpAddr::V4(Ipv4Addr::UNSPECIFIED))
        .build()
        .map_err(|e| CheckError::Relay(e.to_string()))
}

async fn ask_relay(relay: &Relay, nonce: &str, ports: &[(u16, Protocol)]) -> Result<RelayAnswer, CheckError> {
    let path = "/api/port-check";
    let body = json!({
        "nonce": nonce,
        "ports": ports.iter().map(|(port, protocol)| json!({ "port": port, "protocol": protocol })).collect::<Vec<_>>(),
    })
    .to_string();
    let mut req = relay_client()?.post(format!("{}{}", relay.base, path)).header("X-Saba-Client", "1");
    let headers = crate::ipc::handlers::relay::signed_headers_with(&relay.token, "POST", path, Some(&body)).ok_or(CheckError::NotPaired)?;
    for (k, v) in headers {
        req = req.header(k, v);
    }
    let res = req.header("Content-Type", "application/json").body(body).send().await.map_err(|e| CheckError::Relay(e.to_string()))?;
    match res.status().as_u16() {
        200 => res.json::<RelayAnswer>().await.map_err(|_| CheckError::Relay("invalid relay response".into())),
        409 => Err(CheckError::Busy),
        429 => Err(CheckError::RateLimited),
        401 => Err(CheckError::NotPaired),
        code => Err(CheckError::Relay(format!("relay answered {}", code))),
    }
}

/// 이 주소로 가는 길에서 쓰는 이 PC의 주소 (보내지는 않는다: UDP connect는 길만 고른다)
fn route_local(observed: IpAddr) -> Option<IpAddr> {
    let socket = std::net::UdpSocket::bind((bind_ip(), 0)).ok()?;
    socket.connect((observed, 9)).ok()?;
    socket.local_addr().ok().map(|a| a.ip())
}

/// 공유기가 UPnP로 알려 주는 바깥 주소
async fn gateway_external_ip() -> Option<IpAddr> {
    if cfg!(test) {
        return None;
    }
    let mut options = igd_next::SearchOptions::default();
    options.timeout = Some(Duration::from_secs(2));
    let gateway = igd_next::aio::tokio::search_gateway(options).await.ok()?;
    tokio::time::timeout(Duration::from_secs(3), gateway.get_external_ip()).await.ok()?.ok()
}

/// 인스턴스들의 포트를 한 번에 확인한다. `bind_ip`는 시험 리스너 주소 (데몬은 모든 주소, 시험은 루프백)
pub async fn run(relay: &Relay, targets: &[Target], bind_ip: IpAddr) -> Result<Vec<Report>, CheckError> {
    run_with(relay, targets, bind_ip, firewall::blocked_ports).await
}

/// 이 PC 방화벽이 막는 포트를 고르는 함수 (시험에서는 가짜로 바꾼다)
type BlockedPorts = fn(&[(u16, Protocol)]) -> Vec<(u16, Protocol)>;

/// `firewall`: 이 PC 방화벽이 막는 시험 리스너 포트 (닿지 않은 포트를 닫힘 대신 방화벽으로 알린다)
async fn run_with(
    relay: &Relay,
    targets: &[Target],
    bind_ip: IpAddr,
    firewall: BlockedPorts,
) -> Result<Vec<Report>, CheckError> {
    let ids: Vec<String> = targets.iter().map(|t| t.instance_id.clone()).collect();
    let (_hold, starting) = Hold::take(&ids).ok_or(CheckError::Busy)?;
    // 시작하는 중인 서버는 켜진 것으로 본다: 포트를 잡지 않고, 아직 열리지 않았을 수 있어 접속 결과도 쓰지 않는다
    let targets: Vec<Target> =
        targets.iter().map(|t| Target { running: t.running || starting.contains(&t.instance_id), ..t.clone() }).collect();
    let targets = targets.as_slice();
    let nonce = uuid::Uuid::new_v4().simple().to_string();
    let seen: Seen = Arc::default();
    let stop = CancellationToken::new();
    let _stop_listeners = stop.clone().drop_guard();

    // 꺼진 서버는 시험 리스너로, 켜진 서버는 TCP만 릴레이 접속 결과로 본다
    let listen: Vec<(u16, Protocol)> = unique(targets.iter().filter(|t| !t.running).flat_map(|t| t.ports.iter()));
    let (busy, listeners) = open_listeners(&listen, bind_ip, &nonce, &seen, &stop).await;
    let firewalled = firewall(&listen);
    let live_tcp = unique(targets.iter().filter(|t| t.running && !starting.contains(&t.instance_id)).flat_map(|t| t.ports.iter()).filter(|p| p.protocol == Protocol::Tcp));
    let mut ask: Vec<(u16, Protocol)> = listen.iter().filter(|p| !busy.contains(p)).copied().collect();
    for p in live_tcp {
        if !ask.contains(&p) {
            ask.push(p);
        }
    }
    ask.truncate(MAX_PORTS);
    // 한 번에 실을 수 없어 묻지 않은 포트는 닫힘이 아니라 모름이다
    let asked: HashSet<(u16, Protocol)> = ask.iter().copied().collect();

    let (answer, gateway) = if ask.is_empty() {
        (None, None)
    } else {
        let (answer, gateway) = tokio::join!(ask_relay(relay, &nonce, &ask), gateway_external_ip());
        if answer.is_ok() {
            tokio::time::sleep(RECEIVE_GRACE).await;
        }
        (Some(answer), gateway)
    };
    // 포트를 실제로 닫은 뒤에 붙잡기를 푼다 (기다리던 서버 시작이 그 포트를 연다)
    stop.cancel();
    for task in listeners {
        let _ = task.await;
    }
    let answer = answer.transpose()?;
    let seen = seen.lock().unwrap_or_else(|e| e.into_inner()).clone();
    let tcp_results: HashMap<(u16, Protocol), &str> =
        answer.iter().flat_map(|a| &a.probes).map(|p| ((p.port, p.protocol), p.result.as_str())).collect();
    let observed = answer.as_ref().and_then(|a| a.address.parse::<IpAddr>().ok());
    let public_address = answer.as_ref().is_some_and(|a| a.public_address);
    let nat = observed.filter(|_| public_address).map(|o| classify_nat(o, route_local(o), gateway)).unwrap_or(Nat::Unknown);

    let checked_at = chrono::Utc::now().to_rfc3339();
    Ok(targets
        .iter()
        .map(|t| {
            let ports: Vec<PortResult> = t
                .ports
                .iter()
                .map(|p| {
                    let key = (p.port, p.protocol);
                    let status = if !t.running && busy.contains(&key) {
                        PortStatus::InUse
                    } else if !public_address || !asked.contains(&key) {
                        PortStatus::Unknown
                    } else if !t.running {
                        if seen.contains(&key) {
                            PortStatus::Open
                        } else if firewalled.contains(&key) {
                            PortStatus::Firewall
                        } else {
                            PortStatus::Closed
                        }
                    } else if p.protocol == Protocol::Udp {
                        PortStatus::Unknown
                    } else {
                        match tcp_results.get(&key) {
                            Some(&"connected") => PortStatus::Open,
                            Some(_) => PortStatus::Closed,
                            None => PortStatus::Unknown,
                        }
                    };
                    PortResult { field: p.field.clone(), port: p.port, protocol: p.protocol, status }
                })
                .collect();
            Report {
                instance_id: t.instance_id.clone(),
                checked_at: checked_at.clone(),
                address: answer.as_ref().map(|a| a.address.clone()),
                public_address,
                nat,
                running: t.running,
                verdict: verdict(nat, &ports),
                ports,
            }
        })
        .collect())
}

fn unique<'a>(ports: impl Iterator<Item = &'a PublicPort>) -> Vec<(u16, Protocol)> {
    let mut out: Vec<(u16, Protocol)> = Vec::new();
    for p in ports {
        if !out.contains(&(p.port, p.protocol)) {
            out.push((p.port, p.protocol));
        }
    }
    out
}

// ── 마지막 결과 ──

static LAST: Mutex<Option<HashMap<String, Report>>> = Mutex::new(None);

fn remember(reports: &[Report]) {
    let mut last = LAST.lock().unwrap_or_else(|e| e.into_inner());
    let map = last.get_or_insert_with(HashMap::new);
    for r in reports {
        map.insert(r.instance_id.clone(), r.clone());
    }
}

/// 이 인스턴스의 마지막 확인 결과
pub fn last(instance_id: &str) -> Option<Report> {
    LAST.lock().unwrap_or_else(|e| e.into_inner()).as_ref().and_then(|m| m.get(instance_id).cloned())
}

/// 지운 인스턴스의 결과를 버린다
pub fn forget(instance_id: &str) {
    if let Some(map) = LAST.lock().unwrap_or_else(|e| e.into_inner()).as_mut() {
        map.remove(instance_id);
    }
}

// ── 데몬에서 부르기 ──

/// 확인할 인스턴스 (없는 id는 뺀다). 밖에 여는 포트를 선언하지 않은 인스턴스도 `ports`가 빈 채로 넣는다
pub async fn targets(state: &crate::ipc::IPCServer, ids: &[String]) -> Vec<Target> {
    let sup = state.supervisor.read().await;
    let mut out = Vec::new();
    for id in ids {
        let Some(instance) = sup.instance_store.get(id).cloned() else { continue };
        let module = sup.module_loader.get_module(&instance.module_name).ok();
        let ports = public_ports(&instance, module.as_ref().and_then(|m| m.metadata.settings.as_ref()));
        out.push(Target { running: sup.is_running(id).await, instance_id: instance.id, ports });
    }
    out
}

async fn relay_target(state: &crate::ipc::IPCServer) -> Result<Relay, CheckError> {
    let token = crate::ipc::handlers::relay::load_node_token().ok_or(CheckError::NotPaired)?;
    let configured = state.config_store.get_bot_config().await.cloud.relay_url;
    Ok(Relay { base: crate::ipc::handlers::relay::signed_relay_url(&configured, None), token })
}

fn bind_ip() -> IpAddr {
    // 시험은 루프백에만 묶는다 (Windows가 시험 바이너리마다 방화벽 허용 창을 띄우지 않게)
    if cfg!(test) { IpAddr::V4(Ipv4Addr::LOCALHOST) } else { IpAddr::V4(Ipv4Addr::UNSPECIFIED) }
}

/// 확인하고 결과를 남긴다. 결과마다 `instance.port_check` 이벤트를 낸다
pub async fn check(state: &crate::ipc::IPCServer, ids: &[String]) -> Result<Vec<Report>, CheckError> {
    let targets = targets(state, ids).await;
    // 포트를 선언하지 않은 인스턴스는 릴레이에 묻지 않는다
    let (with_ports, without): (Vec<Target>, Vec<Target>) = targets.into_iter().partition(|t| !t.ports.is_empty());
    let mut reports = if with_ports.is_empty() { Vec::new() } else { run(&relay_target(state).await?, &with_ports, bind_ip()).await? };
    let checked_at = chrono::Utc::now().to_rfc3339();
    reports.extend(without.into_iter().map(|t| Report {
        instance_id: t.instance_id,
        checked_at: checked_at.clone(),
        address: None,
        public_address: false,
        nat: Nat::Unknown,
        running: t.running,
        verdict: "no_ports",
        ports: Vec::new(),
    }));
    remember(&reports);
    for r in &reports {
        crate::events::emit("instance.port_check", json!({ "instanceId": r.instance_id, "verdict": r.verdict }));
    }
    Ok(reports)
}

/// 자동 확인이 켜져 있고 릴레이에 연결돼 있으면 조용히 확인한다 (실패는 로그만)
pub async fn auto_check(state: &crate::ipc::IPCServer, ids: Vec<String>) {
    if !state.config_store.get_gui_settings().await.auto_port_check || crate::ipc::handlers::relay::load_node_token().is_none() {
        return;
    }
    // 릴레이 한 번에 실을 수 있는 만큼씩 나눈다
    let targets = targets(state, &ids).await;
    let mut batch: Vec<String> = Vec::new();
    let mut count = 0;
    for t in targets.iter().filter(|t| !t.ports.is_empty()) {
        if count + t.ports.len() > MAX_PORTS && !batch.is_empty() {
            log_auto(check(state, &std::mem::take(&mut batch)).await);
            count = 0;
        }
        count += t.ports.len();
        batch.push(t.instance_id.clone());
    }
    if !batch.is_empty() {
        log_auto(check(state, &batch).await);
    }
}

fn log_auto(result: Result<Vec<Report>, CheckError>) {
    match result {
        Ok(reports) => {
            for r in reports.iter().filter(|r| r.verdict != "open") {
                tracing::info!("Port check for {}: {}", r.instance_id, r.verdict);
            }
        }
        Err(e) => tracing::info!("Automatic port check skipped: {}", e.code()),
    }
}

/// 데몬이 켜질 때: 잠시 뒤 모든 인스턴스를 자동 확인한다
pub async fn auto_on_start(state: crate::ipc::IPCServer) {
    tokio::select! {
        _ = state.shutdown_token.cancelled() => return,
        _ = tokio::time::sleep(BOOT_DELAY) => {}
    }
    let ids: Vec<String> = state.supervisor.read().await.instance_store.list().iter().map(|i| i.id.clone()).collect();
    auto_check(&state, ids).await;
}

mod firewall;

#[cfg(test)]
mod tests;
