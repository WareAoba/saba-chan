//! 접속·퇴장 알림 (MK8 §8.3): 데몬이 콘솔을 읽는 시작(managed)에서 모듈의 `[extensions.saba-core]`
//! `player_join_patterns`·`player_leave_patterns`가 콘솔 줄에 맞으면 `instance.player {instanceId, instance, player, change}`를 낸다.
//!
//! - 시스템 줄(`LogSource::System`)은 보지 않는다. 이름 그룹 `name`만 싣고(IP·고유 ID 없음), 64자에서 자르고 제어 문자를 뺀다.
//! - relay로 올리지 않는다 (0.1.9).
//! - 같은 줄로 접속자를 센다(`players::roster_*`). 모듈이 접속자 수를 주지 않을 때 서버 목록·하트비트가 이 수를 쓴다.

use serde_json::{json, Value};
use std::sync::Arc;
use std::time::Duration;

use super::managed_process::{LogSource, ManagedProcess};

/// 콘솔을 다시 보는 간격
const POLL: Duration = Duration::from_millis(500);
/// 이벤트에 싣는 이름의 최대 길이 (문자)
const NAME_MAX: usize = 64;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Change {
    Join,
    Leave,
}

impl Change {
    fn as_str(self) -> &'static str {
        match self {
            Change::Join => "join",
            Change::Leave => "leave",
        }
    }
}

/// 한 모듈의 접속·퇴장 정규식
pub struct Patterns {
    join: Vec<regex::Regex>,
    leave: Vec<regex::Regex>,
}

impl Patterns {
    /// 로더가 걸러 둔 문자열을 컴파일한다. 둘 다 비면 `None`
    pub fn new(join: &[String], leave: &[String]) -> Option<Self> {
        let compile = |list: &[String]| list.iter().filter_map(|p| regex::Regex::new(p).ok()).collect::<Vec<_>>();
        let patterns = Self { join: compile(join), leave: compile(leave) };
        (!patterns.join.is_empty() || !patterns.leave.is_empty()).then_some(patterns)
    }

    /// 콘솔 줄 하나에서 접속·퇴장과 이름을 찾는다. 이름이 비면 없는 것으로 본다
    pub fn parse(&self, line: &str) -> Option<(String, Change)> {
        let find = |list: &[regex::Regex]| {
            list.iter().find_map(|re| {
                let name: String = re.captures(line)?.name("name")?.as_str().chars().filter(|c| !c.is_control()).take(NAME_MAX).collect();
                (!name.trim().is_empty()).then_some(name)
            })
        };
        find(&self.join).map(|n| (n, Change::Join)).or_else(|| find(&self.leave).map(|n| (n, Change::Leave)))
    }
}

/// 시작 결과를 받는다. 새로 띄운 managed 프로세스이고 문구가 있으면 프로세스가 끝날 때까지 콘솔을 본다
pub fn after_start(instance_id: &str, instance: &str, result: &anyhow::Result<Value>, console: Option<Arc<ManagedProcess>>, join: &[String], leave: &[String]) {
    let Ok(body) = result else { return };
    if body.get("success").and_then(Value::as_bool) != Some(true) || body.get("already_running").and_then(Value::as_bool) == Some(true) {
        return;
    }
    let (Some(proc), Some(patterns)) = (console, Patterns::new(join, leave)) else { return };
    let (id, name) = (instance_id.to_string(), instance.to_string());
    tokio::spawn(async move { watch(id, name, proc, patterns).await });
}

async fn watch(id: String, name: String, proc: Arc<ManagedProcess>, patterns: Patterns) {
    let roster = super::players::roster_start(&id);
    let mut next_line = 0;
    loop {
        // 끝난 것을 먼저 보고 마지막 줄까지 읽은 뒤 나간다
        let running = proc.is_running();
        for line in proc.get_console_since(next_line).await {
            next_line = line.id + 1;
            if line.source == LogSource::System {
                continue;
            }
            if let Some((player, change)) = patterns.parse(&line.content) {
                super::players::roster_change(&id, roster, &player, change == Change::Join);
                crate::events::emit("instance.player", json!({ "instanceId": id, "instance": name, "player": player, "change": change.as_str() }));
            }
        }
        if !running {
            super::players::roster_end(&id, roster);
            return;
        }
        tokio::time::sleep(POLL).await;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn only_the_name_group_is_carried_and_it_is_cleaned() {
        let p = Patterns::new(&[r"^(?P<ip>[\d.]+) (?P<name>.+) joined$".to_string()], &[r"(?P<name>\S*) left".to_string()]).unwrap();
        assert_eq!(p.parse("10.0.0.7 Steve joined"), Some(("Steve".to_string(), Change::Join)));
        assert_eq!(p.parse("1.2.3.4 Ste\u{7}ve joined").unwrap().0, "Steve", "control characters are dropped");
        assert_eq!(p.parse(&format!("1.2.3.4 {} joined", "a".repeat(70))).unwrap().0, "a".repeat(64));
        assert_eq!(p.parse(" left"), None, "an empty name is no player");
        assert_eq!(p.parse("Alex left"), Some(("Alex".to_string(), Change::Leave)));
        assert!(Patterns::new(&[], &[]).is_none());
    }
}
