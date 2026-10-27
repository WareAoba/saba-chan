//! Windows 방화벽이 그 포트로 들어오는 연결을 받는 프로그램을 막는지 판정한다 (읽기만 한다)
//!
//! 방화벽 API(`INetFwPolicy2`)로 지금 연결된 네트워크의 프로필과 규칙을 읽고, 방화벽과 같은 순서로 따진다:
//! 꺼져 있으면 허용, "모두 차단"이면 차단, 맞는 차단 규칙이 있으면 차단, 맞는 허용 규칙이 있거나 기본 동작이 허용이면 허용, 아니면 차단.
//! Windows는 연결마다 그 네트워크의 프로필을 적용하므로, 연결된 프로필이 둘 이상인데 판정이 서로 다르면 어느 연결로 들어올지 몰라 판정하지 않는다.
//! 규칙이 "인터넷 어디서나"가 아니면(원격 주소·인터페이스·서비스·앱 패키지 제한) 밖에서 오는 연결에는 맞지 않는 것으로 본다.
//! 그룹 정책이 로컬 규칙을 덮어쓰거나, 읽지 못하거나, 받는 프로그램을 모르면 판정하지 않는다 (None).

use super::Protocol;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum Verdict {
    Allowed,
    Blocked,
}

/// 그 포트로 들어오는 연결을 받는 프로그램
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub(super) enum Program {
    /// 꺼진 서버 대신 시험 리스너를 연 이 데몬
    Daemon,
    /// 켜진 서버에서 그 포트를 연 프로세스
    Listener,
}

pub(super) type Query = (u16, Protocol, Program);

/// 연결된 네트워크 프로필 하나
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) struct Profile {
    /// NET_FW_PROFILE2_* 비트
    pub mask: i32,
    pub enabled: bool,
    pub block_all: bool,
    pub default_allow: bool,
}

/// 켜진 들어오는 규칙 하나
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub(super) struct Rule {
    pub allow: bool,
    /// NET_FW_PROFILE2_* 비트
    pub profiles: i32,
    /// 프로그램 경로 (없으면 모든 프로그램)
    pub app: Option<String>,
    /// 6 TCP, 17 UDP (없으면 모든 프로토콜)
    pub protocol: Option<i32>,
    /// 받는 포트 범위 (비었으면 모든 포트)
    pub ports: Vec<(u16, u16)>,
    /// 인터넷 어디서 오는 연결에나 맞는다 (원격 주소·원격 포트·로컬 주소·인터페이스·서비스·앱 패키지 제한이 없다)
    pub anywhere: bool,
}

/// `LocalPorts` 글: `*`·빈 글은 모든 포트, 쉼표로 나눈 포트와 범위. `RPC` 같은 이름 포트는 숫자 포트에 맞지 않는다
pub(super) fn parse_ports(text: &str) -> Vec<(u16, u16)> {
    let text = text.trim();
    if text.is_empty() || text == "*" {
        return Vec::new();
    }
    text.split(',')
        .map(|part| {
            let part = part.trim();
            let range = match part.split_once('-') {
                Some((a, b)) => a.trim().parse().ok().zip(b.trim().parse().ok()),
                None => part.parse().ok().map(|p| (p, p)),
            };
            range.unwrap_or((1, 0))
        })
        .collect()
}

/// `%ProgramFiles%` 같은 환경 변수를 풀고 대소문자를 무시해 견준다
fn same_app(rule_app: &str, exe: &str, env: &dyn Fn(&str) -> Option<String>) -> bool {
    let mut expanded = String::new();
    let mut rest = rule_app;
    while let Some(start) = rest.find('%') {
        let Some(len) = rest[start + 1..].find('%') else { break };
        let name = &rest[start + 1..start + 1 + len];
        expanded.push_str(&rest[..start]);
        match env(name) {
            Some(v) => expanded.push_str(&v),
            None => expanded.push_str(&rest[start..start + len + 2]),
        }
        rest = &rest[start + len + 2..];
    }
    expanded.push_str(rest);
    expanded.eq_ignore_ascii_case(exe)
}

fn applies(rule: &Rule, profile: &Profile, exe: &str, port: u16, protocol: Protocol, env: &dyn Fn(&str) -> Option<String>) -> bool {
    let proto = match protocol {
        Protocol::Tcp => 6,
        Protocol::Udp => 17,
    };
    rule.anywhere
        && rule.profiles & profile.mask != 0
        && rule.protocol.is_none_or(|p| p == proto)
        && (rule.ports.is_empty() || rule.ports.iter().any(|&(a, b)| a <= port && port <= b))
        && rule.app.as_deref().is_none_or(|a| same_app(a, exe, env))
}

/// 연결된 프로필마다 따진다. 연결된 프로필이 없거나 프로필마다 판정이 다르면 판정하지 않는다
pub(super) fn decide(
    active: &[Profile],
    rules: &[Rule],
    exe: &str,
    port: u16,
    protocol: Protocol,
    env: &dyn Fn(&str) -> Option<String>,
) -> Option<Verdict> {
    if active.is_empty() {
        return None;
    }
    let blocks = |profile: &Profile| {
        if !profile.enabled {
            return false;
        }
        if profile.block_all {
            return true;
        }
        let matching = || rules.iter().filter(|r| applies(r, profile, exe, port, protocol, env));
        if matching().any(|r| !r.allow) {
            return true;
        }
        !(profile.default_allow || matching().any(|r| r.allow))
    };
    let mut verdicts = active.iter().map(|p| if blocks(p) { Verdict::Blocked } else { Verdict::Allowed });
    let first = verdicts.next()?;
    verdicts.all(|v| v == first).then_some(first)
}

/// 포트마다 방화벽 판정 (Windows만, 다른 OS는 모두 None). 시간이 걸릴 수 있어 blocking 스레드에서 부른다
pub(super) fn verdicts(queries: &[Query]) -> Vec<Option<Verdict>> {
    #[cfg(windows)]
    {
        let Some((active, rules)) = win::read() else { return vec![None; queries.len()] };
        let env = |name: &str| std::env::var(name).ok();
        queries
            .iter()
            .map(|&(port, protocol, program)| {
                let exe = match program {
                    Program::Daemon => std::env::current_exe().ok().map(|p| p.to_string_lossy().into_owned()),
                    Program::Listener => win::listener_exe(port, protocol),
                }?;
                decide(&active, &rules, &exe, port, protocol, &env)
            })
            .collect()
    }
    #[cfg(not(windows))]
    {
        vec![None; queries.len()]
    }
}

#[cfg(windows)]
mod win {
    use super::{parse_ports, Profile, Protocol, Rule};
    use windows::core::Interface;
    use windows::Win32::NetworkManagement::WindowsFirewall::{
        INetFwPolicy2, INetFwRule, INetFwRule3, NetFwPolicy2, NET_FW_ACTION_ALLOW, NET_FW_MODIFY_STATE_GP_OVERRIDE, NET_FW_PROFILE2_DOMAIN,
        NET_FW_PROFILE2_PRIVATE, NET_FW_PROFILE2_PUBLIC, NET_FW_PROFILE_TYPE2, NET_FW_RULE_DIR_IN,
    };
    use windows::Win32::System::Com::{CoCreateInstance, CoInitializeEx, CoUninitialize, CLSCTX_INPROC_SERVER, COINIT_MULTITHREADED};
    use windows::Win32::System::Ole::IEnumVARIANT;
    use windows::Win32::System::Variant::{VariantClear, VARIANT, VT_DISPATCH, VT_EMPTY};

    /// 이 스레드에서 연 COM을 닫는다 (다른 방식으로 이미 열린 스레드면 닫지 않는다)
    struct Com(bool);

    impl Drop for Com {
        fn drop(&mut self) {
            if self.0 {
                unsafe { CoUninitialize() };
            }
        }
    }

    fn any_address(text: &str) -> bool {
        let text = text.trim();
        text.is_empty() || text == "*"
    }

    unsafe fn read_rule(rule: &INetFwRule) -> Option<Rule> {
        if !rule.Enabled().ok()?.as_bool() || rule.Direction().ok()? != NET_FW_RULE_DIR_IN {
            return None;
        }
        let protocol = rule.Protocol().ok()?;
        let ports = if protocol == 6 || protocol == 17 { parse_ports(&rule.LocalPorts().map(|b| b.to_string()).unwrap_or_default()) } else { Vec::new() };
        let text = |b: windows::core::Result<windows::core::BSTR>| b.map(|b| b.to_string()).unwrap_or_default();
        let package = rule.cast::<INetFwRule3>().ok().map(|r| text(r.LocalAppPackageId())).unwrap_or_default();
        // 개별 어댑터 목록이 있으면 그 어댑터로 오는 연결에만 맞는다
        let adapters = match rule.Interfaces() {
            Ok(mut v) => {
                let listed = v.Anonymous.Anonymous.vt != VT_EMPTY;
                let _ = VariantClear(&mut v);
                listed
            }
            Err(_) => true,
        };
        let anywhere = any_address(&text(rule.RemoteAddresses()))
            && any_address(&text(rule.RemotePorts()))
            && any_address(&text(rule.LocalAddresses()))
            && !adapters
            && matches!(text(rule.InterfaceTypes()).trim(), "" | "All")
            && any_address(&text(rule.ServiceName()))
            && package.trim().is_empty();
        let app = text(rule.ApplicationName());
        Some(Rule {
            allow: rule.Action().ok()? == NET_FW_ACTION_ALLOW,
            profiles: rule.Profiles().ok()?,
            app: (!app.trim().is_empty()).then_some(app),
            protocol: (protocol != 256).then_some(protocol),
            ports,
            anywhere,
        })
    }

    /// (연결된 프로필, 켜진 들어오는 규칙). 그룹 정책이 로컬 규칙을 덮어쓰면 None
    pub(super) fn read() -> Option<(Vec<Profile>, Vec<Rule>)> {
        unsafe {
            let _com = Com(CoInitializeEx(None, COINIT_MULTITHREADED).is_ok());
            let policy: INetFwPolicy2 = CoCreateInstance(&NetFwPolicy2, None, CLSCTX_INPROC_SERVER).ok()?;
            let current = policy.CurrentProfileTypes().ok()?;
            if policy.LocalPolicyModifyState().ok()? == NET_FW_MODIFY_STATE_GP_OVERRIDE {
                return None;
            }
            let mut active = Vec::new();
            for p in [NET_FW_PROFILE2_DOMAIN, NET_FW_PROFILE2_PRIVATE, NET_FW_PROFILE2_PUBLIC] {
                if current & p.0 == 0 {
                    continue;
                }
                let kind: NET_FW_PROFILE_TYPE2 = p;
                active.push(Profile {
                    mask: p.0,
                    enabled: policy.get_FirewallEnabled(kind).ok()?.as_bool(),
                    block_all: policy.get_BlockAllInboundTraffic(kind).ok()?.as_bool(),
                    default_allow: policy.get_DefaultInboundAction(kind).ok()? == NET_FW_ACTION_ALLOW,
                });
            }
            let items: IEnumVARIANT = policy.Rules().ok()?._NewEnum().ok()?.cast().ok()?;
            let mut rules = Vec::new();
            loop {
                let mut item = [VARIANT::default()];
                let mut fetched = 0u32;
                if items.Next(&mut item, &mut fetched).is_err() || fetched == 0 {
                    break;
                }
                let inner = &item[0].Anonymous.Anonymous;
                if inner.vt == VT_DISPATCH {
                    if let Some(rule) = inner.Anonymous.pdispVal.as_ref().and_then(|d| d.cast::<INetFwRule>().ok()) {
                        rules.extend(read_rule(&rule));
                    }
                }
                let _ = VariantClear(&mut item[0]);
            }
            Some((active, rules))
        }
    }

    /// 그 포트를 연 프로세스의 실행 파일. AppContainer 안의 프로세스는 따로 제한되므로, 읽지 못한 프로세스가 있어도 판정하지 않는다
    pub(super) fn listener_exe(port: u16, protocol: Protocol) -> Option<String> {
        let pids = match protocol {
            Protocol::Tcp => crate::validator::tcp_listener_pids(port)?,
            Protocol::Udp => crate::validator::udp_owner_pids(port)?,
        };
        // 다른 주소에 같은 포트를 연 프로그램이 여럿이면 어느 쪽이 밖의 연결을 받는지 몰라 판정하지 않는다
        let mut images: Vec<String> = Vec::new();
        for pid in pids.into_iter().filter(|&pid| pid > 4) {
            let image = process_image(pid)?;
            if !images.iter().any(|i| i.eq_ignore_ascii_case(&image)) {
                images.push(image);
            }
        }
        (images.len() == 1).then(|| images.remove(0))
    }

    fn process_image(pid: u32) -> Option<String> {
        use windows_sys::Win32::Foundation::{CloseHandle, HANDLE};
        use windows_sys::Win32::Security::{GetTokenInformation, TokenIsAppContainer, TOKEN_QUERY};
        use windows_sys::Win32::System::Threading::{
            OpenProcess, OpenProcessToken, QueryFullProcessImageNameW, PROCESS_NAME_WIN32, PROCESS_QUERY_LIMITED_INFORMATION,
        };
        unsafe {
            let process = OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION, 0, pid);
            if process.is_null() {
                return None;
            }
            let mut token: HANDLE = std::ptr::null_mut();
            let mut contained = 1u32;
            if OpenProcessToken(process, TOKEN_QUERY, &mut token) != 0 {
                let mut len = 0u32;
                if GetTokenInformation(token, TokenIsAppContainer, (&mut contained as *mut u32).cast(), 4, &mut len) == 0 {
                    contained = 1;
                }
                CloseHandle(token);
            }
            let mut buf = [0u16; 1024];
            let mut size = buf.len() as u32;
            let ok = QueryFullProcessImageNameW(process, PROCESS_NAME_WIN32, buf.as_mut_ptr(), &mut size) != 0;
            CloseHandle(process);
            (ok && contained == 0).then(|| String::from_utf16_lossy(&buf[..size as usize]))
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const EXE: &str = r"C:\Program Files\saba-chan\saba-core.exe";
    const PRIVATE: i32 = 2;
    const PUBLIC: i32 = 4;
    const ALL: i32 = 0x7fff_ffff;

    fn env(name: &str) -> Option<String> {
        (name == "ProgramFiles").then(|| r"C:\Program Files".to_string())
    }

    fn profile(mask: i32) -> Profile {
        Profile { mask, enabled: true, block_all: false, default_allow: false }
    }

    fn allow(app: Option<&str>, protocol: Option<i32>, profiles: i32) -> Rule {
        Rule { allow: true, profiles, app: app.map(str::to_string), protocol, ports: Vec::new(), anywhere: true }
    }

    #[test]
    fn without_an_allow_rule_the_default_inbound_block_applies() {
        let rules = [allow(Some(r"C:\Games\other.exe"), Some(6), ALL)];
        assert_eq!(decide(&[profile(PRIVATE)], &rules, EXE, 7777, Protocol::Tcp, &env), Some(Verdict::Blocked));
        // 방화벽이 꺼져 있거나 기본 동작이 허용이면 막지 않는다
        let off = Profile { enabled: false, ..profile(PRIVATE) };
        assert_eq!(decide(&[off], &rules, EXE, 7777, Protocol::Tcp, &env), Some(Verdict::Allowed));
        let open = Profile { default_allow: true, ..profile(PRIVATE) };
        assert_eq!(decide(&[open], &rules, EXE, 7777, Protocol::Tcp, &env), Some(Verdict::Allowed));
        // "모두 차단"은 허용 규칙보다 앞선다
        let shut = Profile { block_all: true, ..profile(PRIVATE) };
        assert_eq!(decide(&[shut], &[allow(None, None, ALL)], EXE, 7777, Protocol::Tcp, &env), Some(Verdict::Blocked));
        assert_eq!(decide(&[], &rules, EXE, 7777, Protocol::Tcp, &env), None);
    }

    #[test]
    fn the_rule_must_cover_the_connected_profile() {
        // 허용 창에서 개인 네트워크만 허용했는데 지금 공용 네트워크에 연결돼 있다
        let rules = [allow(Some(r"%ProgramFiles%\saba-chan\SABA-CORE.exe"), Some(17), PRIVATE)];
        assert_eq!(decide(&[profile(PRIVATE)], &rules, EXE, 8211, Protocol::Udp, &env), Some(Verdict::Allowed));
        assert_eq!(decide(&[profile(PUBLIC)], &rules, EXE, 8211, Protocol::Udp, &env), Some(Verdict::Blocked));
        assert_eq!(decide(&[profile(PRIVATE)], &rules, EXE, 8211, Protocol::Tcp, &env), Some(Verdict::Blocked), "the UDP rule does not cover TCP");
    }

    #[test]
    fn a_block_rule_wins_over_an_allow_rule() {
        // 허용 창에서 거부하면 막는 규칙이 생긴다: 모든 프로그램을 여는 규칙이 있어도 막는다
        let rules = [allow(None, Some(6), ALL), Rule { allow: false, ..allow(Some(EXE), Some(6), ALL) }];
        assert_eq!(decide(&[profile(PRIVATE)], &rules, EXE, 7777, Protocol::Tcp, &env), Some(Verdict::Blocked));
    }

    #[test]
    fn port_rules_and_remote_limits_are_read() {
        assert_eq!(parse_ports("25000-26000, 7777"), vec![(25000, 26000), (7777, 7777)]);
        assert_eq!(parse_ports("*"), Vec::<(u16, u16)>::new());
        assert_eq!(parse_ports("RPC"), vec![(1, 0)]);
        let ranged = Rule { ports: parse_ports("25000-26000"), ..allow(None, Some(6), ALL) };
        assert_eq!(decide(&[profile(PRIVATE)], &[ranged.clone()], EXE, 25565, Protocol::Tcp, &env), Some(Verdict::Allowed));
        assert_eq!(decide(&[profile(PRIVATE)], &[ranged], EXE, 27015, Protocol::Tcp, &env), Some(Verdict::Blocked));
        let named = Rule { ports: parse_ports("RPC"), ..allow(None, Some(6), ALL) };
        assert_eq!(decide(&[profile(PRIVATE)], &[named], EXE, 135, Protocol::Tcp, &env), Some(Verdict::Blocked));
        // 같은 서브넷에서만 허용하는 규칙은 인터넷에서 오는 친구를 들이지 않는다
        let lan = Rule { anywhere: false, ..allow(Some(EXE), None, ALL) };
        assert_eq!(decide(&[profile(PRIVATE)], &[lan.clone()], EXE, 7777, Protocol::Tcp, &env), Some(Verdict::Blocked));
        // 막는 규칙도 일부 주소에만 걸리면 인터넷 전체를 막지는 않는다
        let lan_block = Rule { allow: false, ..lan };
        let open = Profile { default_allow: true, ..profile(PRIVATE) };
        assert_eq!(decide(&[open], &[lan_block], EXE, 7777, Protocol::Tcp, &env), Some(Verdict::Allowed));
    }

    #[test]
    fn connected_profiles_that_disagree_are_not_judged() {
        // 개인·공용 네트워크에 함께 연결돼 있으면 친구의 연결이 어느 쪽으로 들어올지 모른다
        let rules = [allow(Some(EXE), None, PRIVATE)];
        assert_eq!(decide(&[profile(PRIVATE), profile(PUBLIC)], &rules, EXE, 7777, Protocol::Tcp, &env), None);
        let rules = [allow(Some(EXE), None, PRIVATE | PUBLIC)];
        assert_eq!(decide(&[profile(PRIVATE), profile(PUBLIC)], &rules, EXE, 7777, Protocol::Tcp, &env), Some(Verdict::Allowed));
    }
}
