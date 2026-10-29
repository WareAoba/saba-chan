use super::*;
use crate::supervisor::module_loader::SettingField;
use axum::{extract::State, http::StatusCode, routing::post, Json, Router};
use tokio::io::{AsyncReadExt, AsyncWriteExt};

fn field(name: &str, public: Option<&str>, default: Option<i64>) -> SettingField {
    SettingField {
        name: name.into(),
        field_type: "number".into(),
        port: true,
        public: public.map(str::to_string),
        default: default.map(toml::Value::Integer),
        ..Default::default()
    }
}

#[test]
fn public_ports_come_from_the_fields_the_module_declares() {
    let settings = ModuleSettings {
        fields: vec![
            field("port", Some("udp"), Some(8211)),
            field("query_port", Some("both"), Some(27015)),
            field("rest_port", None, Some(8212)),
            field("extra_port", Some("tcp"), Some(9000)),
        ],
    };
    let mut inst = ServerInstance::new("p", "palworld");
    inst.port = Some(8300);
    inst.module_settings.insert("query_port".into(), json!("27100"));
    let ports = public_ports(&inst, Some(&settings));
    let got: Vec<(&str, u16, Protocol)> = ports.iter().map(|p| (p.field.as_str(), p.port, p.protocol)).collect();
    assert_eq!(
        got,
        vec![
            ("port", 8300, Protocol::Udp),
            ("query_port", 27100, Protocol::Tcp),
            ("query_port", 27100, Protocol::Udp),
            // 인스턴스에 값이 없으면 모듈 기본값
            ("extra_port", 9000, Protocol::Tcp),
        ]
    );
    assert!(public_ports(&inst, None).is_empty());
}

#[test]
fn nat_is_read_from_the_gateway_and_the_route() {
    let observed: IpAddr = "203.0.114.7".parse().unwrap();
    let lan: IpAddr = "192.168.0.10".parse().unwrap();
    assert_eq!(classify_nat(observed, Some(observed), None), Nat::Direct);
    assert_eq!(classify_nat(observed, Some(lan), Some(observed)), Nat::Router);
    // 공유기가 받은 바깥 주소가 통신사 공유 대역이면 CGNAT
    assert_eq!(classify_nat(observed, Some(lan), Some("100.72.1.2".parse().unwrap())), Nat::Cgnat);
    // 사설 주소면 공유기 아래 공유기(이중 NAT)일 수 있어 단정하지 않는다: 바깥 공유기에서 포트포워딩하면 열린다
    assert_eq!(classify_nat(observed, Some(lan), Some("10.1.2.3".parse().unwrap())), Nat::Unknown);
    assert_eq!(classify_nat(observed, Some(lan), Some("192.168.1.2".parse().unwrap())), Nat::Unknown);
    // 공인 주소지만 릴레이가 본 주소와 다르면 한 번 더 바깥에서 묶였다
    assert_eq!(classify_nat(observed, Some(lan), Some("198.51.101.1".parse().unwrap())), Nat::Cgnat);
    assert_eq!(classify_nat(observed, Some(lan), None), Nat::Unknown);
}

#[test]
fn the_verdict_puts_cgnat_and_closed_first() {
    let p = |status| PortResult { field: "port".into(), port: 1, protocol: Protocol::Tcp, status };
    assert_eq!(verdict(Nat::Router, &[]), "no_ports");
    // 확인값을 실제로 받았으면 NAT 추정보다 앞선다
    assert_eq!(verdict(Nat::Cgnat, &[p(PortStatus::Open)]), "open");
    assert_eq!(verdict(Nat::Cgnat, &[p(PortStatus::Open), p(PortStatus::Closed)]), "cgnat");
    assert_eq!(verdict(Nat::Cgnat, &[p(PortStatus::Closed)]), "cgnat");
    assert_eq!(verdict(Nat::Router, &[p(PortStatus::Open), p(PortStatus::Closed)]), "closed");
    assert_eq!(verdict(Nat::Unknown, &[p(PortStatus::Open), p(PortStatus::Open)]), "open");
    assert_eq!(verdict(Nat::Router, &[p(PortStatus::Open), p(PortStatus::Unknown)]), "unknown");
    assert_eq!(verdict(Nat::Router, &[p(PortStatus::InUse)]), "unknown");
    assert_eq!(verdict(Nat::Router, &[p(PortStatus::Closed), p(PortStatus::Firewall)]), "firewall");
    // 방화벽부터 풀어야 공유기 설정을 확인할 수 있다
    assert_eq!(verdict(Nat::Router, &[p(PortStatus::Router), p(PortStatus::Firewall)]), "firewall");
    assert_eq!(verdict(Nat::Router, &[p(PortStatus::Open), p(PortStatus::Router)]), "router");
    assert_eq!(verdict(Nat::Cgnat, &[p(PortStatus::Router)]), "cgnat");
}

// ── 릴레이 흉내: 실제 릴레이처럼 요청이 온 주소(루프백)의 그 포트로 확인값을 보낸다 ──

#[derive(Clone, Copy)]
enum Mode {
    /// 확인값을 보낸다 (포트가 열린 공유기)
    Deliver,
    /// 보내지 않는다 (공유기가 막는다): TCP는 timeout
    Drop,
    RateLimited,
}

async fn relay_handler(State(mode): State<Mode>, Json(body): Json<Value>) -> (StatusCode, Json<Value>) {
    if let Mode::RateLimited = mode {
        return (StatusCode::TOO_MANY_REQUESTS, Json(json!({ "error": "RATE_LIMITED" })));
    }
    let nonce = body["nonce"].as_str().unwrap().to_string();
    let line = format!("saba-chan-port-check {}\n", nonce);
    let mut probes = Vec::new();
    for p in body["ports"].as_array().unwrap() {
        let port = p["port"].as_u64().unwrap() as u16;
        let protocol = p["protocol"].as_str().unwrap().to_string();
        let result = match (mode, protocol.as_str()) {
            (Mode::Drop, "tcp") => "timeout",
            (Mode::Drop, _) => "sent",
            (_, "tcp") => match tokio::net::TcpStream::connect(("127.0.0.1", port)).await {
                Ok(mut s) => {
                    let _ = s.write_all(line.as_bytes()).await;
                    let _ = s.shutdown().await;
                    "connected"
                }
                Err(_) => "refused",
            },
            _ => {
                let s = tokio::net::UdpSocket::bind("127.0.0.1:0").await.unwrap();
                for _ in 0..3 {
                    let _ = s.send_to(line.as_bytes(), ("127.0.0.1", port)).await;
                }
                "sent"
            }
        };
        probes.push(json!({ "port": port, "protocol": protocol, "result": result }));
    }
    (StatusCode::OK, Json(json!({ "address": "8.8.4.4", "publicAddress": true, "probes": probes })))
}

async fn mock_relay(mode: Mode) -> Relay {
    let app = Router::new().route("/api/port-check", post(relay_handler)).with_state(mode);
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });
    Relay { base: format!("http://{}", addr), token: "sbn_host1.secret".into() }
}

fn free_port() -> u16 {
    std::net::TcpListener::bind("127.0.0.1:0").unwrap().local_addr().unwrap().port()
}

/// UDP는 UDP로 빈 포트를 받는다: TCP로 비어 있던 번호를 Windows 러너의 다른 프로그램이 UDP로 쥐고 있어 `InUse`가 나왔다
fn free_udp_port() -> u16 {
    std::net::UdpSocket::bind("127.0.0.1:0").unwrap().local_addr().unwrap().port()
}

fn target(id: &str, running: bool, ports: &[(u16, Protocol)]) -> Target {
    Target {
        instance_id: id.into(),
        running,
        ports: ports.iter().map(|&(port, protocol)| PublicPort { field: "port".into(), port, protocol }).collect(),
    }
}

const LOOPBACK: IpAddr = IpAddr::V4(Ipv4Addr::LOCALHOST);

/// 방화벽을 읽지 못한 PC (시험 결과가 시험하는 PC의 방화벽에 따라 달라지지 않게)
fn unread(queries: &[firewall::Query]) -> Vec<Option<firewall::Verdict>> {
    vec![None; queries.len()]
}

async fn run(relay: &Relay, targets: &[Target], bind_ip: IpAddr) -> Result<Vec<Report>, CheckError> {
    run_with(relay, targets, bind_ip, unread).await
}

fn statuses(r: &Report) -> Vec<PortStatus> {
    r.ports.iter().map(|p| p.status).collect()
}

#[tokio::test]
async fn a_stopped_server_is_open_when_the_probe_reaches_the_test_listener() {
    let relay = mock_relay(Mode::Deliver).await;
    let (tcp, udp) = (free_port(), free_udp_port());
    let reports = run(&relay, &[target("pc-open", false, &[(tcp, Protocol::Tcp), (udp, Protocol::Udp)])], LOOPBACK).await.unwrap();
    assert_eq!(statuses(&reports[0]), vec![PortStatus::Open, PortStatus::Open]);
    assert_eq!(reports[0].verdict, "open");
    assert_eq!(reports[0].address.as_deref(), Some("8.8.4.4"));
    // 확인이 끝나면 리스너를 닫는다 (서버가 그 포트를 열 수 있다)
    std::net::TcpListener::bind(("127.0.0.1", tcp)).expect("tcp port released");
    std::net::UdpSocket::bind(("127.0.0.1", udp)).expect("udp port released");
}

#[tokio::test]
async fn a_stopped_server_is_closed_when_nothing_arrives() {
    let relay = mock_relay(Mode::Drop).await;
    let (tcp, udp) = (free_port(), free_udp_port());
    let reports = run(&relay, &[target("pc-closed", false, &[(tcp, Protocol::Tcp), (udp, Protocol::Udp)])], LOOPBACK).await.unwrap();
    assert_eq!(statuses(&reports[0]), vec![PortStatus::Closed, PortStatus::Closed]);
    assert_eq!(reports[0].verdict, "closed");
}

#[tokio::test]
async fn an_unreachable_port_is_blamed_on_the_firewall_or_the_router_by_the_firewall_rules() {
    use firewall::{Program, Verdict};
    let relay = mock_relay(Mode::Drop).await;
    let (tcp, udp) = (free_port(), free_udp_port());
    // 방화벽이 시험 리스너(데몬)의 TCP는 막고 UDP는 허용한다
    fn blocks_tcp(queries: &[firewall::Query]) -> Vec<Option<Verdict>> {
        queries
            .iter()
            .map(|&(_, protocol, program)| {
                assert_eq!(program, Program::Daemon, "a stopped server is reached through the daemon's test listener");
                Some(if protocol == Protocol::Tcp { Verdict::Blocked } else { Verdict::Allowed })
            })
            .collect()
    }
    let reports =
        run_with(&relay, &[target("pc-firewall", false, &[(tcp, Protocol::Tcp), (udp, Protocol::Udp)])], LOOPBACK, blocks_tcp).await.unwrap();
    assert_eq!(statuses(&reports[0]), vec![PortStatus::Firewall, PortStatus::Router]);
    assert_eq!(reports[0].verdict, "firewall");
    // 방화벽이 모두 허용하면 원인은 공유기다
    fn allows(queries: &[firewall::Query]) -> Vec<Option<Verdict>> {
        vec![Some(Verdict::Allowed); queries.len()]
    }
    let reports = run_with(&relay, &[target("pc-router", false, &[(tcp, Protocol::Tcp)])], LOOPBACK, allows).await.unwrap();
    assert_eq!(statuses(&reports[0]), vec![PortStatus::Router]);
    assert_eq!(reports[0].verdict, "router");
}

#[tokio::test]
async fn a_running_server_is_judged_by_the_program_that_opened_the_port() {
    use firewall::{Program, Verdict};
    let relay = mock_relay(Mode::Drop).await;
    let game = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let tcp = game.local_addr().unwrap().port();
    let udp = free_udp_port();
    fn blocks_the_game(queries: &[firewall::Query]) -> Vec<Option<Verdict>> {
        queries.iter().map(|&(_, _, program)| (program == Program::Listener).then_some(Verdict::Blocked)).collect()
    }
    let reports =
        run_with(&relay, &[target("pc-running-fw", true, &[(tcp, Protocol::Tcp), (udp, Protocol::Udp)])], LOOPBACK, blocks_the_game).await.unwrap();
    // 켜진 서버의 UDP는 닿았는지 몰라도 방화벽이 막는 것은 안다
    assert_eq!(statuses(&reports[0]), vec![PortStatus::Firewall, PortStatus::Firewall]);
    fn allows(queries: &[firewall::Query]) -> Vec<Option<Verdict>> {
        vec![Some(Verdict::Allowed); queries.len()]
    }
    let reports = run_with(&relay, &[target("pc-running-fw", true, &[(tcp, Protocol::Tcp), (udp, Protocol::Udp)])], LOOPBACK, allows).await.unwrap();
    assert_eq!(statuses(&reports[0]), vec![PortStatus::Router, PortStatus::Unknown]);
    drop(game);
}

#[tokio::test]
async fn ports_past_the_relay_limit_are_unknown_not_closed() {
    let relay = mock_relay(Mode::Drop).await;
    // 서로 다른 포트여야 한다 (같은 포트가 두 번 나오면 하나로 묶여 한도 안에 든다)
    let mut ports: Vec<(u16, Protocol)> = Vec::new();
    while ports.len() < MAX_PORTS + 1 {
        let p = (free_udp_port(), Protocol::Udp);
        if !ports.contains(&p) {
            ports.push(p);
        }
    }
    let reports = run(&relay, &[target("pc-many", false, &ports)], LOOPBACK).await.unwrap();
    let got = statuses(&reports[0]);
    assert_eq!(got[..MAX_PORTS], vec![PortStatus::Closed; MAX_PORTS][..]);
    assert_eq!(got[MAX_PORTS], PortStatus::Unknown);
}

#[tokio::test]
async fn a_connection_left_open_does_not_keep_the_port() {
    // 확인값 없이 연결만 붙잡고 있는 상대가 있어도, 확인이 끝나면 서버가 그 포트를 열 수 있다
    let port = free_port();
    let seen: Seen = Arc::default();
    let stop = CancellationToken::new();
    let (_, tasks) = open_listeners(&[(port, Protocol::Tcp)], LOOPBACK, "abc", &seen, &stop).await;
    let mut idle = tokio::net::TcpStream::connect(("127.0.0.1", port)).await.unwrap();
    tokio::time::sleep(Duration::from_millis(50)).await;
    stop.cancel();
    for task in tasks {
        task.await.unwrap();
    }
    // 받은 연결이 확인과 함께 닫혔다 (남아 있으면 3초 동안 읽기를 기다린다)
    let mut buf = [0u8; 1];
    let read = tokio::time::timeout(Duration::from_millis(500), idle.read(&mut buf)).await.expect("the accepted connection is closed");
    assert!(matches!(read, Ok(0) | Err(_)));
    std::net::TcpListener::bind(("127.0.0.1", port)).expect("port free after the check");
}

#[tokio::test]
async fn a_probe_without_the_nonce_does_not_count() {
    // 누군가 그 포트로 다른 값을 보내도 열림이 아니다
    let port = free_port();
    let seen: Seen = Arc::default();
    let stop = CancellationToken::new();
    let (busy, _tasks) = open_listeners(&[(port, Protocol::Tcp)], LOOPBACK, "abc", &seen, &stop).await;
    assert!(busy.is_empty());
    let mut s = tokio::net::TcpStream::connect(("127.0.0.1", port)).await.unwrap();
    s.write_all(b"saba-chan-port-check other\n").await.unwrap();
    s.shutdown().await.unwrap();
    tokio::time::sleep(Duration::from_millis(100)).await;
    assert!(seen.lock().unwrap().is_empty());
    stop.cancel();
}

#[tokio::test]
async fn a_port_another_program_holds_is_in_use() {
    let relay = mock_relay(Mode::Deliver).await;
    let other = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
    let port = other.local_addr().unwrap().port();
    let reports = run(&relay, &[target("pc-in-use", false, &[(port, Protocol::Tcp)])], LOOPBACK).await.unwrap();
    assert_eq!(statuses(&reports[0]), vec![PortStatus::InUse]);
    assert_eq!(reports[0].verdict, "unknown");
}

#[tokio::test]
async fn a_running_server_is_checked_by_connecting_and_udp_stays_unknown() {
    let relay = mock_relay(Mode::Deliver).await;
    // 게임 서버가 연 포트
    let game = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let tcp = game.local_addr().unwrap().port();
    tokio::spawn(async move {
        while let Ok((mut s, _)) = game.accept().await {
            let mut buf = Vec::new();
            let _ = s.read_to_end(&mut buf).await;
        }
    });
    let udp = free_udp_port();
    let reports = run(&relay, &[target("pc-running", true, &[(tcp, Protocol::Tcp), (udp, Protocol::Udp)])], LOOPBACK).await.unwrap();
    assert_eq!(statuses(&reports[0]), vec![PortStatus::Open, PortStatus::Unknown]);
    // 켜진 서버의 TCP가 닿지 않으면 닫힘
    let relay = mock_relay(Mode::Drop).await;
    let reports = run(&relay, &[target("pc-running", true, &[(tcp, Protocol::Tcp)])], LOOPBACK).await.unwrap();
    assert_eq!(statuses(&reports[0]), vec![PortStatus::Closed]);
}

#[tokio::test]
async fn relay_refusals_are_reported_and_the_ports_are_released() {
    let relay = mock_relay(Mode::RateLimited).await;
    let port = free_port();
    let err = run(&relay, &[target("pc-limited", false, &[(port, Protocol::Tcp)])], LOOPBACK).await.unwrap_err();
    assert_eq!(err, CheckError::RateLimited);
    std::net::TcpListener::bind(("127.0.0.1", port)).expect("port released after a refusal");
    assert!(!is_held("pc-limited"));
}

#[tokio::test]
async fn a_start_waits_while_the_ports_are_held_and_a_second_check_is_busy() {
    let (hold, _) = Hold::take(&["pc-hold".to_string()]).unwrap();
    let relay = mock_relay(Mode::Deliver).await;
    assert_eq!(run(&relay, &[target("pc-hold", false, &[(free_port(), Protocol::Tcp)])], LOOPBACK).await.unwrap_err(), CheckError::Busy);
    let waiting = tokio::spawn(async { begin_start("pc-hold").await });
    tokio::time::sleep(Duration::from_millis(50)).await;
    assert!(!waiting.is_finished(), "a start waits for the check");
    drop(hold);
    let started = tokio::time::timeout(Duration::from_secs(1), waiting).await.expect("released start continues").unwrap();
    drop(started);
    // 다른 인스턴스는 기다리지 않는다
    let _other = tokio::time::timeout(Duration::from_millis(50), begin_start("pc-other")).await.unwrap();
}

#[tokio::test]
async fn a_check_during_a_start_leaves_the_ports_to_the_server() {
    let relay = mock_relay(Mode::Deliver).await;
    let port = free_port();
    let starting = begin_start("pc-starting").await;
    let reports = run(&relay, &[target("pc-starting", false, &[(port, Protocol::Tcp)])], LOOPBACK).await.unwrap();
    // 서버가 곧 열 포트를 잡지 않고, 아직 열리지 않았을 수 있어 닫힘이라고 하지도 않는다
    assert_eq!(statuses(&reports[0]), vec![PortStatus::Unknown]);
    assert!(reports[0].running);
    drop(starting);
    assert!(with_holds(|h| !h.starting.contains_key("pc-starting")));
}

#[tokio::test]
async fn a_start_during_a_check_waits_and_then_the_server_can_open_the_ports() {
    // 확인 도중 들어온 시작 요청: 확인이 끝날 때까지 기다렸다가, 끝나면 곧바로 게임 서버가 같은 포트를 연다
    let relay = mock_relay(Mode::Drop).await;
    let (tcp, udp) = (free_port(), free_udp_port());
    let check = tokio::spawn(async move {
        run(&relay, &[target("pc-start-wait", false, &[(tcp, Protocol::Tcp), (udp, Protocol::Udp)])], LOOPBACK).await
    });
    while !is_held("pc-start-wait") {
        tokio::time::sleep(Duration::from_millis(5)).await;
    }
    let started = tokio::time::Instant::now();
    let starting = tokio::time::timeout(Duration::from_secs(5), begin_start("pc-start-wait")).await.expect("the start continues after the check");
    assert!(started.elapsed() >= Duration::from_millis(100), "the start waited for the check");
    let _game_tcp = std::net::TcpListener::bind(("127.0.0.1", tcp)).expect("the server opens the TCP port right after the check");
    let _game_udp = std::net::UdpSocket::bind(("127.0.0.1", udp)).expect("the server opens the UDP port right after the check");
    let reports = check.await.unwrap().unwrap();
    assert_eq!(statuses(&reports[0]), vec![PortStatus::Closed, PortStatus::Closed]);
    drop(starting);
}
