//! Windows 방화벽이 시험 리스너(이 데몬 실행 파일)의 들어오는 연결을 막는지 (읽기만 한다)
//!
//! 규칙은 레지스트리에 `v2.x|Action=Allow|Active=TRUE|Dir=In|Protocol=6|LPort=7777|App=C:\…\saba-core.exe|…` 한 줄씩 있다.
//! 지금 연결된 네트워크의 프로필은 보지 않으므로, 어느 프로필에서든 허용하는 규칙이 있으면 막지 않는다고 본다
//! (허용이 있는데 막는다고 잘못 알리지 않게). 읽지 못하면 막지 않는다고 본다.

use super::Protocol;

#[derive(Debug, Default, PartialEq, Eq)]
pub(super) struct Rule {
    allow: bool,
    /// 프로그램 경로 (없으면 모든 프로그램)
    app: Option<String>,
    /// 6 TCP, 17 UDP (없으면 모든 프로토콜)
    protocol: Option<u8>,
    /// 받는 포트 범위 (비었으면 모든 포트)
    ports: Vec<(u16, u16)>,
}

/// 들어오는 연결의 켜진 규칙만 돌려준다
pub(super) fn parse_rule(line: &str) -> Option<Rule> {
    let mut rule = Rule::default();
    let (mut active, mut inbound) = (false, false);
    for part in line.split('|') {
        let Some((key, value)) = part.split_once('=') else { continue };
        match key {
            "Action" => rule.allow = value.eq_ignore_ascii_case("Allow"),
            "Active" => active = value.eq_ignore_ascii_case("TRUE"),
            "Dir" => inbound = value.eq_ignore_ascii_case("In"),
            "Protocol" => rule.protocol = value.parse().ok().or(Some(u8::MAX)),
            "App" => rule.app = Some(value.to_string()),
            "LPort" => {
                let range = match value.split_once('-') {
                    Some((a, b)) => a.parse().ok().zip(b.parse().ok()),
                    None => value.parse().ok().map(|p| (p, p)),
                };
                // RPC 같은 이름 포트는 숫자 포트에 맞지 않는다
                rule.ports.push(range.unwrap_or((1, 0)));
            }
            _ => {}
        }
    }
    (active && inbound).then_some(rule)
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

fn applies(rule: &Rule, exe: &str, port: u16, protocol: Protocol, env: &dyn Fn(&str) -> Option<String>) -> bool {
    let proto = match protocol {
        Protocol::Tcp => 6,
        Protocol::Udp => 17,
    };
    rule.protocol.is_none_or(|p| p == proto)
        && (rule.ports.is_empty() || rule.ports.iter().any(|&(a, b)| a <= port && port <= b))
        && rule.app.as_deref().is_none_or(|a| same_app(a, exe, env))
}

/// 방화벽이 켜져 있고 이 실행 파일의 그 포트를 허용하는 규칙이 하나도 없으면 true
pub(super) fn blocks(
    enabled: bool,
    rules: &[Rule],
    exe: &str,
    port: u16,
    protocol: Protocol,
    env: &dyn Fn(&str) -> Option<String>,
) -> bool {
    enabled && !rules.iter().any(|r| r.allow && applies(r, exe, port, protocol, env))
}

/// 이 PC의 방화벽이 지금 데몬의 그 포트를 막는가 (Windows만, 다른 OS는 false)
pub(super) fn blocked_ports(ports: &[(u16, Protocol)]) -> Vec<(u16, Protocol)> {
    #[cfg(windows)]
    {
        let Some((enabled, rules)) = windows::read() else { return Vec::new() };
        let Some(exe) = std::env::current_exe().ok().map(|p| p.to_string_lossy().into_owned()) else { return Vec::new() };
        let env = |name: &str| std::env::var(name).ok();
        ports.iter().copied().filter(|&(port, protocol)| blocks(enabled, &rules, &exe, port, protocol, &env)).collect()
    }
    #[cfg(not(windows))]
    {
        let _ = ports;
        Vec::new()
    }
}

#[cfg(windows)]
mod windows {
    use super::{parse_rule, Rule};
    use winreg::enums::HKEY_LOCAL_MACHINE;
    use winreg::RegKey;

    const POLICY: &str = r"SYSTEM\CurrentControlSet\Services\SharedAccess\Parameters\FirewallPolicy";
    const GROUP_POLICY: &str = r"SOFTWARE\Policies\Microsoft\WindowsFirewall";

    /// (어느 프로필이든 켜져 있는가, 들어오는 규칙)
    pub(super) fn read() -> Option<(bool, Vec<Rule>)> {
        let hklm = RegKey::predef(HKEY_LOCAL_MACHINE);
        let policy = hklm.open_subkey(POLICY).ok()?;
        let enabled = ["DomainProfile", "StandardProfile", "PublicProfile"].iter().any(|p| {
            policy.open_subkey(p).and_then(|k| k.get_value::<u32, _>("EnableFirewall")).map(|v| v != 0).unwrap_or(true)
        });
        let mut rules = Vec::new();
        for key in [policy.open_subkey("FirewallRules").ok(), hklm.open_subkey(format!(r"{GROUP_POLICY}\FirewallRules")).ok()]
            .into_iter()
            .flatten()
        {
            for (_, value) in key.enum_values().flatten() {
                if let Some(rule) = parse_rule(&value.to_string()) {
                    rules.push(rule);
                }
            }
        }
        Some((enabled, rules))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const EXE: &str = r"C:\Program Files\saba-chan\saba-core.exe";

    fn env(name: &str) -> Option<String> {
        (name == "ProgramFiles").then(|| r"C:\Program Files".to_string())
    }

    fn rules(lines: &[&str]) -> Vec<Rule> {
        lines.iter().filter_map(|l| parse_rule(l)).collect()
    }

    #[test]
    fn no_rule_for_the_daemon_means_the_firewall_blocks_it() {
        let r = rules(&[r"v2.33|Action=Allow|Active=TRUE|Dir=In|Protocol=6|LPort=7777|App=C:\Games\other.exe|"]);
        assert!(blocks(true, &r, EXE, 7777, Protocol::Tcp, &env));
        // 방화벽이 꺼져 있으면 막지 않는다
        assert!(!blocks(false, &r, EXE, 7777, Protocol::Tcp, &env));
    }

    #[test]
    fn an_allow_rule_for_the_daemon_lets_the_port_in() {
        // 허용 창에서 허용하면 생기는 규칙: 프로토콜마다, 포트 제한 없음
        let r = rules(&[
            r"v2.33|Action=Allow|Active=TRUE|Dir=In|Protocol=17|Profile=Private|App=%ProgramFiles%\saba-chan\SABA-CORE.exe|Name=saba-core|",
        ]);
        assert!(!blocks(true, &r, EXE, 8211, Protocol::Udp, &env));
        assert!(blocks(true, &r, EXE, 8211, Protocol::Tcp, &env), "the UDP rule does not cover TCP");
    }

    #[test]
    fn port_ranges_disabled_and_outbound_rules_are_read() {
        let r = rules(&[
            r"v2.33|Action=Allow|Active=TRUE|Dir=In|Protocol=6|LPort=25000-26000|",
            r"v2.33|Action=Allow|Active=FALSE|Dir=In|App=C:\Program Files\saba-chan\saba-core.exe|",
            r"v2.33|Action=Allow|Active=TRUE|Dir=Out|App=C:\Program Files\saba-chan\saba-core.exe|",
            r"v2.33|Action=Allow|Active=TRUE|Dir=In|Protocol=6|LPort=RPC|",
        ]);
        assert!(!blocks(true, &r, EXE, 25565, Protocol::Tcp, &env));
        assert!(blocks(true, &r, EXE, 27015, Protocol::Tcp, &env));
    }

    #[test]
    fn a_deny_answer_alone_blocks() {
        // 허용 창에서 거부하면 막는 규칙만 생긴다
        let r = rules(&[r"v2.33|Action=Block|Active=TRUE|Dir=In|Protocol=6|App=C:\Program Files\saba-chan\saba-core.exe|"]);
        assert!(blocks(true, &r, EXE, 7777, Protocol::Tcp, &env));
    }
}
