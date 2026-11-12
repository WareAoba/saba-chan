//! 접속자 없을 때 자동 정지: 인스턴스 설정 `idle_stop`이 켜져 있고 접속자 0명이 `idle_stop_minutes`분 이어지면 정지한다.
//! 켠 뒤 `idle_stop_grace_minutes`분 안에는 정지하지 않는다 (아무도 들어오기 전에 꺼지지 않게).
//! 접속자 수는 데몬이 이미 아는 값(`players::get`)만 본다: 이 판정을 위해 서버에 따로 묻지 않는다.
//! 수를 모르는 동안(None)은 비어 있다고 보지 않고 처음부터 다시 센다

use std::collections::HashMap;
use std::sync::{Mutex, OnceLock};

use serde::Serialize;

pub const DEFAULT_MINUTES: u64 = 30;
pub const DEFAULT_GRACE_MINUTES: u64 = 15;
const MAX_MINUTES: u64 = 24 * 60;

/// 켜 둔 인스턴스의 설정 (분)
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Rule {
    pub minutes: u64,
    pub grace_minutes: u64,
}

/// `module_settings`의 `idle_stop`·`idle_stop_minutes`·`idle_stop_grace_minutes`. 꺼져 있으면 None
pub fn rule_of(instance: &crate::instance::ServerInstance) -> Option<Rule> {
    if !super::setting_flag(instance, "idle_stop", false) {
        return None;
    }
    let minutes = |key: &str, default: u64, least: u64| {
        let value = instance.module_settings.get(key);
        let n = value.and_then(|v| v.as_u64().or_else(|| v.as_str()?.trim().parse().ok())).unwrap_or(default);
        n.clamp(least, MAX_MINUTES)
    };
    Some(Rule { minutes: minutes("idle_stop_minutes", DEFAULT_MINUTES, 1), grace_minutes: minutes("idle_stop_grace_minutes", DEFAULT_GRACE_MINUTES, 0) })
}

/// 인스턴스마다 (그 프로세스를 켠 시각, 0명이 되기 시작한 시각) (Unix 초)
fn empty_since() -> &'static Mutex<HashMap<String, (u64, u64)>> {
    static EMPTY: OnceLock<Mutex<HashMap<String, (u64, u64)>>> = OnceLock::new();
    EMPTY.get_or_init(Default::default)
}

/// 켜진 서버를 한 번 본다. 지금 정지해야 하면 비어 있던 것으로 알릴 분(`rule.minutes`).
/// `started`는 그 프로세스를 켠 시각: 다시 켠 서버는 처음부터 센다
pub fn observe(instance_id: &str, rule: Option<Rule>, started: u64, players: Option<u32>, now: u64) -> Option<u64> {
    let mut map = empty_since().lock().unwrap_or_else(|e| e.into_inner());
    let (Some(rule), Some(0)) = (rule, players) else {
        map.remove(instance_id);
        return None;
    };
    let entry = map.entry(instance_id.to_string()).or_insert((started, now));
    if entry.0 != started {
        *entry = (started, now);
    }
    let waited = now.saturating_sub(entry.1) >= rule.minutes * 60;
    let past_grace = now.saturating_sub(started) >= rule.grace_minutes * 60;
    (waited && past_grace).then_some(rule.minutes)
}

/// 꺼졌거나 지운 서버는 잊는다
pub fn forget(instance_id: &str) {
    empty_since().lock().unwrap_or_else(|e| e.into_inner()).remove(instance_id);
}

/// 마지막 정지가 자동 정지였을 때: 정지한 시각(Unix 초)과 비어 있던 분. 서버 목록(`ServerInfo.idle_stopped`)이 꺼진 서버에 싣는다
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, serde::Deserialize)]
pub struct Stopped {
    pub at: u64,
    pub minutes: u64,
}

fn stops() -> &'static Mutex<HashMap<String, Stopped>> {
    static STOPS: OnceLock<Mutex<HashMap<String, Stopped>>> = OnceLock::new();
    STOPS.get_or_init(Default::default)
}

pub fn mark_stopped(instance_id: &str, minutes: u64, now: u64) {
    stops().lock().unwrap_or_else(|e| e.into_inner()).insert(instance_id.to_string(), Stopped { at: now, minutes });
}

pub fn stopped(instance_id: &str) -> Option<Stopped> {
    stops().lock().unwrap_or_else(|e| e.into_inner()).get(instance_id).copied()
}

/// 다시 켜면 지난 자동 정지 안내를 지운다
pub fn clear_stopped(instance_id: &str) {
    stops().lock().unwrap_or_else(|e| e.into_inner()).remove(instance_id);
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    const MIN: u64 = 60;

    #[test]
    fn an_empty_server_stops_after_the_wait_but_not_inside_the_grace_after_start() {
        let id = "idle-wait-test";
        let rule = Some(Rule { minutes: 30, grace_minutes: 15 });
        let started = 1_000_000;
        assert_eq!(observe(id, rule, started, Some(0), started + MIN), None);
        assert_eq!(observe(id, rule, started, Some(0), started + 30 * MIN), None, "29 minutes empty so far");
        assert_eq!(observe(id, rule, started, Some(0), started + 31 * MIN), Some(30));

        // 켠 직후 기다리기가 더 길면 그때까지는 끄지 않는다
        let id = "idle-grace-test";
        let rule = Some(Rule { minutes: 5, grace_minutes: 15 });
        assert_eq!(observe(id, rule, started, Some(0), started), None);
        assert_eq!(observe(id, rule, started, Some(0), started + 10 * MIN), None, "empty long enough, but started 10 minutes ago");
        assert_eq!(observe(id, rule, started, Some(0), started + 15 * MIN), Some(5));
    }

    #[test]
    fn a_player_an_unknown_count_a_restart_or_the_setting_turned_off_starts_the_wait_over() {
        let id = "idle-reset-test";
        let rule = Some(Rule { minutes: 10, grace_minutes: 0 });
        let started = 2_000_000;
        assert_eq!(observe(id, rule, started, Some(0), started), None);
        assert_eq!(observe(id, rule, started, Some(1), started + 9 * MIN), None);
        assert_eq!(observe(id, rule, started, Some(0), started + 10 * MIN), None);
        assert_eq!(observe(id, rule, started, Some(0), started + 19 * MIN), None, "nine minutes since the player left");
        assert_eq!(observe(id, rule, started, None, started + 20 * MIN), None, "an unknown count is not an empty server");
        assert_eq!(observe(id, rule, started, Some(0), started + 21 * MIN), None);
        assert_eq!(observe(id, None, started, Some(0), started + 40 * MIN), None, "the setting is off");
        assert_eq!(observe(id, rule, started, Some(0), started + 41 * MIN), None);
        // 다시 켠 프로세스는 처음부터 센다
        let restarted = started + 50 * MIN;
        assert_eq!(observe(id, rule, restarted, Some(0), started + 52 * MIN), None);
        assert_eq!(observe(id, rule, restarted, Some(0), started + 62 * MIN), Some(10));
    }

    #[test]
    fn the_rule_is_off_by_default_and_reads_minutes_saved_as_numbers_or_text() {
        let mut instance = crate::instance::ServerInstance::new("srv", "plain");
        assert_eq!(rule_of(&instance), None);
        instance.module_settings.insert("idle_stop".into(), json!(true));
        assert_eq!(rule_of(&instance), Some(Rule { minutes: DEFAULT_MINUTES, grace_minutes: DEFAULT_GRACE_MINUTES }));
        instance.module_settings.insert("idle_stop_minutes".into(), json!("45"));
        instance.module_settings.insert("idle_stop_grace_minutes".into(), json!(0));
        assert_eq!(rule_of(&instance), Some(Rule { minutes: 45, grace_minutes: 0 }));
        instance.module_settings.insert("idle_stop_minutes".into(), json!(0));
        assert_eq!(rule_of(&instance).unwrap().minutes, 1, "zero minutes would stop a server the moment it empties");
    }
}
