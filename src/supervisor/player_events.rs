//! 접속·퇴장 알림 (MK8 §8.3): 데몬이 콘솔을 읽는 시작(managed)에서 모듈의 `[extensions.saba-core]`
//! `player_join_patterns`·`player_leave_patterns`가 콘솔 줄에 맞으면 `instance.player {instanceId, instance, player, change}`를 낸다.
//!
//! - 서버가 찍은 줄(`LogSource::is_server_output`)만 본다: 시스템 줄과 보낸 명령 줄은 보지 않는다. 이름 그룹 `name`만 싣고(IP·고유 ID 없음), 64자에서 자르고 제어 문자를 뺀다.
//! - relay로 올리지 않는다 (0.1.9).
//! - 같은 줄로 접속자를 센다(`players::roster_*`). 모듈이 접속자 수를 주지 않을 때 서버 목록·하트비트가 이 수를 쓴다.
//! - 같은 콘솔에서 `chat_patterns`에 맞는 채팅 줄을 찾아 게임 채팅 전달(`game_chat.rs`)과 채팅 연동(`chat_link.rs`)에 넘긴다.
//! - 센 접속자가 0명이 되거나 프로세스가 끝나면 채팅 연동을 끈다.

use serde_json::{json, Value};
use std::sync::Arc;
use std::time::Duration;

use super::managed_process::ManagedProcess;

/// 콘솔을 다시 보는 간격 (메모리 버퍼만 본다. 채팅이 늦게 올라가지 않게 짧게)
const POLL: Duration = Duration::from_millis(100);
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

/// 한 모듈의 접속·퇴장·채팅 정규식
pub struct Patterns {
    join: Vec<regex::Regex>,
    leave: Vec<regex::Regex>,
    chat: Vec<regex::Regex>,
}

fn compile(list: &[String]) -> Vec<regex::Regex> {
    list.iter().filter_map(|p| regex::Regex::new(p).ok()).collect()
}

impl Patterns {
    /// 로더가 걸러 둔 문자열을 컴파일한다. 둘 다 비면 `None`
    #[cfg(test)]
    pub fn new(join: &[String], leave: &[String]) -> Option<Self> {
        Self::with_chat(join, leave, &[])
    }

    /// 채팅 문구(`name`·`message` 그룹)까지. 셋 다 비면 `None`
    pub fn with_chat(join: &[String], leave: &[String], chat: &[String]) -> Option<Self> {
        let patterns = Self { join: compile(join), leave: compile(leave), chat: compile(chat) };
        (!patterns.join.is_empty() || !patterns.leave.is_empty() || !patterns.chat.is_empty()).then_some(patterns)
    }

    /// 콘솔 줄 하나에서 채팅 이름과 본문을 찾는다. 이름이나 본문이 비면 없는 것으로 본다
    pub fn parse_chat<'a>(&self, line: &'a str) -> Option<(String, &'a str)> {
        self.chat.iter().find_map(|re| {
            let caps = re.captures(line)?;
            let name: String = caps.name("name")?.as_str().chars().filter(|c| !c.is_control()).take(NAME_MAX).collect();
            let message = caps.name("message")?.as_str();
            (!name.trim().is_empty() && !message.trim().is_empty()).then_some((name, message))
        })
    }

    /// 접속과 퇴장을 둘 다 알아볼 때만 접속자를 셀 수 있다 (한쪽만 있으면 늘기만 하거나 늘 0이다)
    pub fn can_count(&self) -> bool {
        !self.join.is_empty() && !self.leave.is_empty()
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
pub fn after_start(rt: &super::runtime::Runtimes, instance_id: &str, instance: &str, result: &anyhow::Result<Value>, console: Option<Arc<ManagedProcess>>, join: &[String], leave: &[String], chat: &[String]) {
    let Ok(body) = result else { return };
    if body.get("success").and_then(Value::as_bool) != Some(true) || body.get("already_running").and_then(Value::as_bool) == Some(true) {
        return;
    }
    let (Some(proc), Some(patterns)) = (console, Patterns::with_chat(join, leave, chat)) else { return };
    let (rt, id, name) = (rt.clone(), instance_id.to_string(), instance.to_string());
    tokio::spawn(async move { watch(rt, id, name, proc, patterns).await });
}

/// 데몬이 다시 켜져 아직 도는 서버(saba-runner)에 다시 붙었다. 붙기 전 줄은 알리지 않고, 그 줄에서 아직 들어와 있는
/// 것으로 보이는 사람을 `instance.players_present {instanceId, instance, players}`로 한 번 알린 뒤 새 줄부터 알린다
/// (조작 가능한 사람만 접속이 데몬이 꺼진 동안 들어온 사람도 본다). 접속자는 세지 않는다(모듈이 알려 준 수를 쓴다)
pub fn after_reattach(rt: &super::runtime::Runtimes, instance_id: &str, instance: &str, console: Arc<ManagedProcess>, join: &[String], leave: &[String], chat: &[String]) {
    let Some(patterns) = Patterns::with_chat(join, leave, chat) else { return };
    let (rt, id, name) = (rt.clone(), instance_id.to_string(), instance.to_string());
    tokio::spawn(async move { watch_from(rt, id, name, console, patterns, true).await });
}

async fn watch(rt: super::runtime::Runtimes, id: String, name: String, proc: Arc<ManagedProcess>, patterns: Patterns) {
    watch_from(rt, id, name, proc, patterns, false).await
}

async fn watch_from(rt: super::runtime::Runtimes, id: String, name: String, proc: Arc<ManagedProcess>, patterns: Patterns, reattached: bool) {
    // 다시 붙은 서버는 접속자를 세지 않는다: 되살린 콘솔은 끝부분만이라(러너 로그 꼬리) 센 수가 모자랄 수 있고,
    // 센 수는 모듈이 알려 준 수보다 앞선다 (`players::get`)
    let roster = (patterns.can_count() && !reattached).then(|| super::players::roster_start(&rt, &id));
    let mut next_line = 0;
    if reattached {
        // 되살린 줄에서 아직 들어와 있는 것으로 보이는 사람 (알 수 있는 만큼만)
        let mut present = std::collections::BTreeSet::new();
        for line in proc.get_console_since(0).await {
            next_line = line.id + 1;
            if !line.source.is_server_output() || !patterns.can_count() {
                continue;
            }
            match patterns.parse(&line.content) {
                Some((player, Change::Join)) => present.insert(player),
                Some((player, Change::Leave)) => present.remove(&player),
                None => false,
            };
        }
        if !present.is_empty() {
            crate::events::emit("instance.players_present", json!({ "instanceId": id, "instance": name, "players": present }));
        }
    }
    loop {
        // 끝난 것을 먼저 보고 마지막 줄까지 읽은 뒤 나간다
        let running = proc.is_running();
        for line in proc.get_console_since(next_line).await {
            next_line = line.id + 1;
            if !line.source.is_server_output() {
                continue;
            }
            if let Some((player, change)) = patterns.parse(&line.content) {
                if let Some(roster) = roster {
                    super::players::roster_change(&rt, &id, roster, &player, change == Change::Join);
                    if change == Change::Leave && super::players::counted(&rt, &id) == Some(0) {
                        super::chat_link::end(&rt, &id, &name, "empty", roster);
                    }
                }
                crate::events::emit("instance.player", json!({ "instanceId": id, "instance": name, "player": player, "change": change.as_str() }));
            } else if let Some((player, message)) = patterns.parse_chat(&line.content) {
                // 설정은 줄마다 읽는다: 켜 둔 채로 설정을 바꾸면 바로 따른다. 디스코드에서 보낸 글의 메아리는 되돌리지 않는다
                if super::game_chat::is_echo(&id, &player, message) {
                    continue;
                }
                if let Some(ev) = super::chat_link::chat_event(&id, &name, &player, message, &super::game_chat::settings(&id), super::chat_link::to_discord(&id)) {
                    crate::events::emit("instance.chat", ev);
                }
            }
        }
        if !running {
            // 연동은 접속자를 셀 때만 켜진다. 이 프로세스의 목록일 때만 끈다 (새 프로세스의 연동은 그대로 둔다)
            if let Some(roster) = roster {
                super::chat_link::end(&rt, &id, &name, "stopped", roster);
                super::players::roster_end(&rt, &id, roster);
            }
            return;
        }
        tokio::time::sleep(POLL).await;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// 데몬을 다시 켜 도는 서버에 다시 붙으면, 붙기 전 줄은 다시 알리지 않고 지금 들어와 있는 사람만 한 번 알린 뒤
    /// 새 줄부터 알린다 (리뷰 P1 2026-11-06: 재연결 뒤 조작 가능한 사람만 접속이 멈췄다)
    #[cfg(unix)]
    #[tokio::test]
    async fn reattaching_counts_who_is_in_and_reports_only_new_lines() {
        let script = "echo 'Steve joined'; echo 'Bob joined'; echo 'Bob left'; read go; echo 'Alex joined'; sleep 5";
        let proc = Arc::new(ManagedProcess::spawn("/bin/sh", &["-c".into(), script.into()], ".", Vec::new(), None, None).await.unwrap());
        for _ in 0..50 {
            if proc.get_recent_console(10).await.iter().any(|l| l.content == "Bob left") {
                break;
            }
            tokio::time::sleep(Duration::from_millis(50)).await;
        }
        let id = "reattach-test";
        let mut sub = crate::events::bus().subscribe(None);
        let (join, leave) = ([r"^(?P<name>\w+) joined$".to_string()], [r"^(?P<name>\w+) left$".to_string()]);
        let rt = crate::supervisor::runtime::Runtimes::default();
        after_reattach(&rt, id, "Test", proc.clone(), &join, &leave, &[]);
        tokio::time::sleep(Duration::from_millis(300)).await;
        proc.send_command("go").await.unwrap();
        let mut seen = Vec::new();
        let deadline = tokio::time::Instant::now() + Duration::from_secs(3);
        while seen.len() < 2 && tokio::time::Instant::now() < deadline {
            if let Ok(Ok(e)) = tokio::time::timeout(Duration::from_millis(200), sub.rx.recv()).await {
                if e.data["instanceId"] == id {
                    seen.push((e.kind.clone(), e.data.get("players").cloned().unwrap_or_else(|| e.data["player"].clone())));
                }
            }
        }
        assert_eq!(seen, [("instance.players_present".to_string(), json!(["Steve"])), ("instance.player".to_string(), json!("Alex"))]);
        // 되살린 콘솔은 끝부분만일 수 있어 세지 않는다: 접속자 수는 모듈이 알려 준 수를 쓴다
        assert_eq!(super::super::players::counted(&rt, id), None);
        let _ = proc.force_kill().await;
    }

    #[test]
    fn only_the_name_group_is_carried_and_it_is_cleaned() {
        let p = Patterns::new(&[r"^(?P<ip>[\d.]+) (?P<name>.+) joined$".to_string()], &[r"(?P<name>\S*) left".to_string()]).unwrap();
        assert_eq!(p.parse("10.0.0.7 Steve joined"), Some(("Steve".to_string(), Change::Join)));
        assert_eq!(p.parse("1.2.3.4 Ste\u{7}ve joined").unwrap().0, "Steve", "control characters are dropped");
        assert_eq!(p.parse(&format!("1.2.3.4 {} joined", "a".repeat(70))).unwrap().0, "a".repeat(64));
        assert_eq!(p.parse(" left"), None, "an empty name is no player");
        assert_eq!(p.parse("Alex left"), Some(("Alex".to_string(), Change::Leave)));
        assert!(Patterns::new(&[], &[]).is_none());
        assert!(p.can_count());
    }

    #[test]
    fn chat_lines_carry_the_name_and_the_message() {
        let p = Patterns::with_chat(&[], &[], &[r"^<(?P<name>\w+)> (?P<message>.*)$".to_string()]).unwrap();
        assert_eq!(p.parse_chat("<Steve> hello there"), Some(("Steve".to_string(), "hello there")));
        assert_eq!(p.parse_chat("<Steve>  "), None, "an empty message is no chat");
        assert_eq!(p.parse(""), None);
        assert!(!p.can_count(), "chat alone does not count players");
    }

    // 리뷰(Codex #625): 한쪽 문구만 있으면 이벤트는 내되 접속자 수는 세지 않는다
    #[test]
    fn players_are_counted_only_when_both_join_and_leave_are_known() {
        let join_only = Patterns::new(&["(?P<name>\\S+) joined".to_string()], &[]).unwrap();
        let leave_only = Patterns::new(&[], &["(?P<name>\\S+) left".to_string()]).unwrap();
        assert!(!join_only.can_count() && !leave_only.can_count());
        assert_eq!(join_only.parse("Bob joined"), Some(("Bob".to_string(), Change::Join)));
    }
}
