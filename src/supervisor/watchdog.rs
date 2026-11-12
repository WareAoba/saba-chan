//! 응답 없는 서버 감시: 프로세스는 살아 있는데 상태 질문에 답하지 않는 서버를 인스턴스 설정대로 알리거나 다시 시작한다.
//! `unresponsive_action`(`off`·`notify`·`restart`, 기본 `notify`)과 `unresponsive_minutes`(기본 10, 5~1440).
//! 판정에는 접속자 수 갱신이 이미 모은 응답 실패 기록(`players::silent_for`)만 쓴다: 이 감시를 위해 서버에 따로 묻지 않는다.
//! 한 번 응답이 끊긴 동안에는 한 번만 알리고, 다시 시작한 뒤에도 답이 없으면 또 다시 시작하지 않고 알리기만 한다

use std::collections::HashMap;
use std::sync::{Mutex, OnceLock};
use std::time::{Duration, Instant};

pub const DEFAULT_MINUTES: u64 = 10;
/// 그보다 일찍은 판정하지 않는다 (`players::UNRESPONSIVE_AFTER`: 늦게 여는 서버를 기다린다)
pub const MIN_MINUTES: u64 = super::players::UNRESPONSIVE_AFTER.as_secs() / 60;
const MAX_MINUTES: u64 = 24 * 60;
/// 다시 시작한 서버가 이만큼 실패 없이 지내면 답한 것으로 본다 (갱신 세 번)
const ANSWERED_AFTER: Duration = Duration::from_secs(3 * super::players::REFRESH_SECS);

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Action {
    Off,
    Notify,
    Restart,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Rule {
    pub action: Action,
    /// 응답 없음이 이만큼 이어지면 판정한다
    pub after: Duration,
}

/// `module_settings`의 `unresponsive_action`·`unresponsive_minutes`. 모르는 값은 기본값으로 읽는다
pub fn rule_of(instance: &crate::instance::ServerInstance) -> Rule {
    let action = match instance.module_settings.get("unresponsive_action").and_then(|v| v.as_str()) {
        Some("off") => Action::Off,
        Some("restart") => Action::Restart,
        _ => Action::Notify,
    };
    let minutes = instance
        .module_settings
        .get("unresponsive_minutes")
        .and_then(|v| v.as_u64().or_else(|| v.as_str()?.trim().parse().ok()))
        .unwrap_or(DEFAULT_MINUTES)
        .clamp(MIN_MINUTES, MAX_MINUTES);
    Rule { action, after: Duration::from_secs(minutes * 60) }
}

/// 이번 차례에 할 일
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Verdict {
    /// 알린다
    Notify,
    /// 알리고 다시 시작한다
    Restart,
}

#[derive(Default)]
struct Watch {
    /// 이미 알린 끊김: 그 프로세스의 pid
    alerted: Option<Option<u32>>,
    /// 감시가 다시 시작한 때. 그 뒤로 답한 적이 없으면 또 다시 시작하지 않는다
    restarted: Option<Instant>,
}

fn watches() -> &'static Mutex<HashMap<String, Watch>> {
    static WATCHES: OnceLock<Mutex<HashMap<String, Watch>>> = OnceLock::new();
    WATCHES.get_or_init(Default::default)
}

/// 켜진 서버를 한 번 본다. `silent`는 지금 프로세스가 답하지 않은 시간(답하고 있으면 None)
pub fn observe(instance_id: &str, pid: Option<u32>, rule: Rule, silent: Option<Duration>, now: Instant) -> Option<Verdict> {
    let mut map = watches().lock().unwrap_or_else(|e| e.into_inner());
    let watch = map.entry(instance_id.to_string()).or_default();
    let Some(silent) = silent else {
        watch.alerted = None;
        if watch.restarted.is_some_and(|at| now.duration_since(at) >= ANSWERED_AFTER) {
            watch.restarted = None;
        }
        return None;
    };
    if rule.action == Action::Off || silent < rule.after || watch.alerted == Some(pid) {
        return None;
    }
    watch.alerted = Some(pid);
    if rule.action == Action::Restart && watch.restarted.is_none() {
        watch.restarted = Some(now);
        return Some(Verdict::Restart);
    }
    Some(Verdict::Notify)
}

/// 꺼졌거나 지운 서버는 잊는다 (사용자가 끄고 다시 켠 서버는 처음부터 본다)
pub fn forget(instance_id: &str) {
    watches().lock().unwrap_or_else(|e| e.into_inner()).remove(instance_id);
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    const MIN: Duration = Duration::from_secs(60);

    #[test]
    fn a_silent_server_is_reported_once_after_the_wait_and_again_only_after_it_answered() {
        let id = "watchdog-notify-test";
        let rule = Rule { action: Action::Notify, after: 10 * MIN };
        let now = Instant::now();
        assert_eq!(observe(id, Some(1), rule, Some(9 * MIN), now), None);
        assert_eq!(observe(id, Some(1), rule, Some(10 * MIN), now), Some(Verdict::Notify));
        assert_eq!(observe(id, Some(1), rule, Some(11 * MIN), now), None, "one notice per silence");
        assert_eq!(observe(id, Some(1), rule, None, now), None);
        assert_eq!(observe(id, Some(1), rule, Some(10 * MIN), now), Some(Verdict::Notify), "silent again after answering");
        let off = Rule { action: Action::Off, after: 10 * MIN };
        assert_eq!(observe("watchdog-off-test", Some(1), off, Some(60 * MIN), now), None);
    }

    #[test]
    fn restart_is_tried_once_and_a_server_still_silent_after_it_is_only_reported() {
        let id = "watchdog-restart-test";
        let rule = Rule { action: Action::Restart, after: 10 * MIN };
        let now = Instant::now();
        assert_eq!(observe(id, Some(1), rule, Some(10 * MIN), now), Some(Verdict::Restart));
        // 다시 시작한 프로세스(pid 2)도 답이 없다: 끝없이 다시 시작하지 않는다
        assert_eq!(observe(id, Some(2), rule, Some(10 * MIN), now + 11 * MIN), Some(Verdict::Notify));
        assert_eq!(observe(id, Some(2), rule, Some(20 * MIN), now + 21 * MIN), None);
        // 한동안 답한 뒤에 다시 끊기면 다시 시작한다
        assert_eq!(observe(id, Some(2), rule, None, now + 30 * MIN), None);
        assert_eq!(observe(id, Some(2), rule, Some(10 * MIN), now + 50 * MIN), Some(Verdict::Restart));
    }

    #[test]
    fn the_rule_defaults_to_notify_after_ten_minutes_and_never_judges_before_five() {
        let mut instance = crate::instance::ServerInstance::new("srv", "plain");
        assert_eq!(rule_of(&instance), Rule { action: Action::Notify, after: 10 * MIN });
        instance.module_settings.insert("unresponsive_action".into(), json!("restart"));
        instance.module_settings.insert("unresponsive_minutes".into(), json!("1"));
        assert_eq!(rule_of(&instance), Rule { action: Action::Restart, after: 5 * MIN });
        instance.module_settings.insert("unresponsive_action".into(), json!("off"));
        assert_eq!(rule_of(&instance).action, Action::Off);
    }
}
