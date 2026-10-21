//! 인스턴스 예약 작업 (A3): 백업·재시작·공지·업데이트
//!
//! 인스턴스마다 `instance.json`의 `schedule`에 작업 목록이 있다. 데몬은 30초마다 한 번 보고,
//! 때가 된 작업을 실행한다. 마지막 실행 시각은 메모리에만 둔다: 데몬을 다시 켜면 주기 작업은
//! 그때부터 다시 세고, 시각 작업은 지나간 시각을 따라잡지 않는다 (밀린 재시작이 한꺼번에 돌지 않게).
//! 데몬이 켜진 뒤 더하거나 고친 작업은 그때부터 세고, 밀린 차례(PC 절전 등)는 가장 최근 것 한 번만 실행한다.
//! 재시작·업데이트는 `warn_minutes`만큼 먼저 깨어나 게임 안에 예고한 뒤 정한 시각에 실행한다.

use chrono::{Duration, NaiveDateTime, NaiveTime};
use serde::{Deserialize, Serialize};
use std::collections::{HashMap, HashSet};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum TaskAction {
    /// 서버가 켜져 있을 때 백업 (꺼져 있으면 바뀐 것이 없으니 건너뛴다)
    Backup,
    /// 켜져 있으면 다시 시작
    Restart,
    /// 켜져 있으면 게임 안 공지 (`message`)
    Announce,
    /// 서버 업데이트를 확인하고 있으면 적용 (켜져 있었으면 다시 켠다)
    Update,
}

#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct ScheduledTask {
    pub action: TaskAction,
    /// N분마다. `at`과 둘 중 하나
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub every_minutes: Option<u32>,
    /// 매일 이 시각 (데몬 PC 시간대, "HH:MM")
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub at: Option<String>,
    /// 재시작·업데이트 몇 분 전에 게임 안에 예고할지 (예: [10, 5, 1])
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub warn_minutes: Vec<u32>,
    /// 공지 문구 (announce)
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub message: Option<String>,
    #[serde(default = "enabled_default")]
    pub enabled: bool,
}

fn enabled_default() -> bool {
    true
}

impl ScheduledTask {
    /// 저장 전에 고칠 점. 비면 통과
    pub fn problems(&self) -> Vec<String> {
        let mut out = Vec::new();
        match (self.every_minutes, &self.at) {
            (Some(0), _) => out.push("every_minutes must be at least 1".into()),
            (Some(_), Some(_)) | (None, None) => out.push("set exactly one of every_minutes or at".into()),
            (None, Some(at)) if parse_at(at).is_none() => out.push(format!("at '{}' must be HH:MM", at)),
            _ => {}
        }
        if self.action == TaskAction::Announce && self.message.as_deref().unwrap_or("").trim().is_empty() {
            out.push("announce needs a message".into());
        }
        if !self.warn_minutes.is_empty() && !matches!(self.action, TaskAction::Restart | TaskAction::Update) {
            out.push("warn_minutes only applies to restart and update".into());
        }
        out
    }

    fn lead(&self) -> Duration {
        Duration::minutes(self.warn_minutes.iter().copied().max().unwrap_or(0) as i64)
    }

    /// 작업을 실행할 다음 시각. `last`는 마지막 실행(없으면 데몬 시작) 시각
    fn next_fire(&self, last: NaiveDateTime) -> Option<NaiveDateTime> {
        if let Some(n) = self.every_minutes {
            return Some(last + Duration::minutes(n as i64));
        }
        let at = parse_at(self.at.as_deref()?)?;
        // 예고 시간만큼 먼저 깨어나므로, 깨어날 시각이 last보다 뒤인 가장 이른 슬롯
        let mut slot = last.date().and_time(at);
        while slot - self.lead() <= last {
            slot += Duration::days(1);
        }
        Some(slot)
    }
}

fn parse_at(at: &str) -> Option<NaiveTime> {
    NaiveTime::parse_from_str(at.trim(), "%H:%M").ok()
}

/// 지금 깨울 작업: (작업, 실행 시각). 예고가 있으면 실행 시각보다 먼저 나온다
#[derive(Default)]
pub struct Clock {
    last: HashMap<String, NaiveDateTime>,
    running: HashSet<String>,
    /// 첫 차례의 시각 (그때 있던 작업만 데몬 시작부터 센다)
    first_scan: Option<NaiveDateTime>,
}

impl Clock {
    /// 이번 차례에 시작할 작업. 시작한 작업은 끝날 때 `finish`로 알린다
    pub fn due(&mut self, key: &str, task: &ScheduledTask, started: NaiveDateTime, now: NaiveDateTime) -> Option<NaiveDateTime> {
        // 저장 검사를 거치지 않은 틀린 작업(손으로 고친 파일의 `every_minutes: 0` 등)은 건너뛴다
        if !task.enabled || self.running.contains(key) || !task.problems().is_empty() {
            return None;
        }
        // 처음 보는 작업: 데몬이 켜질 때 있던 것(첫 차례)은 시작부터, 뒤에 더하거나 고친 것은 지금부터 센다
        let first = *self.first_scan.get_or_insert(now);
        let base = if now == first { started } else { now };
        let last = *self.last.entry(key.to_string()).or_insert(base);
        let mut fire = task.next_fire(last)?;
        if now < fire - task.lead() {
            return None;
        }
        // 밀린 차례(PC 절전 등)는 가장 최근 것 한 번만
        if let Some(n) = task.every_minutes {
            let period = Duration::minutes(n as i64);
            let behind = (now - fire).num_minutes() / n as i64;
            if behind > 0 {
                fire += period * behind as i32;
            }
        } else {
            while let Some(next) = task.next_fire(fire).filter(|next| *next <= now) {
                fire = next;
            }
        }
        // 따라잡은 차례 바로 뒤 차례의 예고가 이미 시작됐으면, 밀린 것은 건너뛰고 그 차례를 한다 (연달아 두 번 하지 않게)
        if fire < now {
            if let Some(next) = task.next_fire(fire).filter(|next| now >= *next - task.lead()) {
                fire = next;
            }
        }
        self.running.insert(key.to_string());
        Some(fire)
    }

    pub fn finish(&mut self, key: &str, fired_at: NaiveDateTime) {
        self.running.remove(key);
        self.last.insert(key.to_string(), fired_at);
    }

    /// 차례마다 먼저 부른다: 목록에서 사라진 작업은 잊는다
    pub fn retain(&mut self, keys: &HashSet<String>, now: NaiveDateTime) {
        self.first_scan.get_or_insert(now);
        self.last.retain(|k, _| keys.contains(k));
    }
}

/// 작업마다 고유한 열쇠 (인스턴스 id + 작업 내용). 작업을 고치면 새 작업으로 센다
pub fn task_key(instance_id: &str, task: &ScheduledTask) -> String {
    format!("{}:{}", instance_id, serde_json::to_string(task).unwrap_or_default())
}

/// 예고를 시작한 작업이 아직 그대로 예약돼 있는지. 지웠거나 껐거나 고쳤으면 키가 달라진다
pub fn still_scheduled(instance_id: &str, schedule: &[ScheduledTask], key: &str) -> bool {
    schedule.iter().any(|t| t.enabled && task_key(instance_id, t) == key)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn t(s: &str) -> NaiveDateTime {
        NaiveDateTime::parse_from_str(s, "%Y-%m-%d %H:%M").unwrap()
    }

    fn task(json: serde_json::Value) -> ScheduledTask {
        serde_json::from_value(json).unwrap()
    }

    #[test]
    fn interval_counts_from_daemon_start_then_last_run() {
        let mut c = Clock::default();
        let job = task(serde_json::json!({"action": "backup", "every_minutes": 30}));
        let start = t("2026-10-17 10:00");
        assert_eq!(c.due("k", &job, start, t("2026-10-17 10:29")), None);
        let fire = c.due("k", &job, start, t("2026-10-17 10:30")).unwrap();
        assert_eq!(c.due("k", &job, start, t("2026-10-17 10:31")), None, "still running");
        c.finish("k", fire);
        assert_eq!(c.due("k", &job, start, t("2026-10-17 10:59")), None);
        assert!(c.due("k", &job, start, t("2026-10-17 11:00")).is_some());
    }

    #[test]
    fn daily_restart_wakes_early_for_warnings_and_skips_missed_slots() {
        let mut c = Clock::default();
        let job = task(serde_json::json!({"action": "restart", "at": "04:00", "warn_minutes": [10, 1]}));
        // 04:00이 지난 뒤에 켜진 데몬은 그날 것을 따라잡지 않는다
        let start = t("2026-10-17 05:00");
        assert_eq!(c.due("k", &job, start, t("2026-10-18 03:49")), None);
        let fire = c.due("k", &job, start, t("2026-10-18 03:50")).unwrap();
        assert_eq!(fire, t("2026-10-18 04:00"));
        c.finish("k", fire);
        assert_eq!(c.due("k", &job, start, t("2026-10-18 04:05")), None);
        assert_eq!(c.due("k", &job, start, t("2026-10-19 03:50")), Some(t("2026-10-19 04:00")));
    }

    #[test]
    fn daemon_started_inside_the_warning_window_waits_for_tomorrow() {
        let mut c = Clock::default();
        let job = task(serde_json::json!({"action": "restart", "at": "04:00", "warn_minutes": [10]}));
        assert_eq!(c.due("k", &job, t("2026-10-17 03:55"), t("2026-10-17 03:56")), None);
        assert_eq!(c.due("k", &job, t("2026-10-17 03:55"), t("2026-10-18 03:50")), Some(t("2026-10-18 04:00")));
    }

    /// 외부 감사: 데몬이 켜진 뒤에 더한 작업이 데몬 시작부터 세어 지나간 횟수를 연달아 실행했다.
    /// 새로 더하거나 고친 작업은 그때부터 세고, 밀린 작업(PC 절전 등)은 마지막 한 번만 실행한다
    #[test]
    fn new_tasks_count_from_when_they_appear_and_missed_runs_fire_once() {
        let mut c = Clock::default();
        let job = task(serde_json::json!({"action": "restart", "every_minutes": 60}));
        let start = t("2026-10-17 10:00");
        c.retain(&HashSet::new(), t("2026-10-17 10:00"));
        assert_eq!(c.due("k", &job, start, t("2026-10-17 14:00")), None, "added at 14:00");
        assert_eq!(c.due("k", &job, start, t("2026-10-17 14:59")), None);
        let fire = c.due("k", &job, start, t("2026-10-17 15:00")).unwrap();
        assert_eq!(fire, t("2026-10-17 15:00"));
        c.finish("k", fire);

        // 15:00 실행 뒤 PC가 잠들었다가 18:30에 깬다: 18:00 한 번만
        let fire = c.due("k", &job, start, t("2026-10-17 18:30")).unwrap();
        assert_eq!(fire, t("2026-10-17 18:00"));
        c.finish("k", fire);
        assert_eq!(c.due("k", &job, start, t("2026-10-17 18:31")), None);
        assert_eq!(c.due("k", &job, start, t("2026-10-17 19:00")), Some(t("2026-10-17 19:00")));

        // 시각 작업도 며칠 밀렸으면 가장 최근 시각 한 번만
        let daily = task(serde_json::json!({"action": "backup", "at": "04:00"}));
        let fire = c.due("d", &daily, start, t("2026-10-17 10:00")).inspect(|f| c.finish("d", *f));
        assert_eq!(fire, None);
        let fire = c.due("d", &daily, start, t("2026-10-18 04:00")).unwrap();
        c.finish("d", fire);
        let fire = c.due("d", &daily, start, t("2026-10-21 12:00")).unwrap();
        assert_eq!(fire, t("2026-10-21 04:00"));
        c.finish("d", fire);
        assert_eq!(c.due("d", &daily, start, t("2026-10-21 12:01")), None);
    }

    /// 리뷰 594: 손으로 고친 파일의 `every_minutes: 0` 같은 틀린 작업은 건너뛴다(실행기가 멈추지 않게).
    /// 밀린 차례를 따라잡을 때 다음 차례의 예고가 이미 시작됐으면 밀린 것은 건너뛰고 다음 차례만 한다
    #[test]
    fn bad_tasks_are_skipped_and_a_catch_up_inside_the_next_warning_waits_for_it() {
        let mut c = Clock::default();
        let start = t("2026-10-17 10:00");
        let zero = task(serde_json::json!({"action": "backup", "every_minutes": 0}));
        assert_eq!(c.due("z", &zero, start, t("2026-10-17 10:30")), None);
        assert_eq!(c.due("z", &zero, start, t("2026-10-17 11:30")), None);

        let job = task(serde_json::json!({"action": "restart", "every_minutes": 60, "warn_minutes": [10]}));
        c.finish("k", t("2026-10-17 15:00"));
        assert_eq!(c.due("k", &job, start, t("2026-10-17 18:55")), Some(t("2026-10-17 19:00")));
        c.finish("k", t("2026-10-17 19:00"));
        assert_eq!(c.due("k", &job, start, t("2026-10-17 19:01")), None);

        let daily = task(serde_json::json!({"action": "restart", "at": "04:00", "warn_minutes": [10]}));
        c.finish("d", t("2026-10-17 04:00"));
        assert_eq!(c.due("d", &daily, start, t("2026-10-19 03:55")), Some(t("2026-10-19 04:00")));
    }

    #[test]
    fn problems_catch_bad_tasks() {
        let bad = [
            serde_json::json!({"action": "backup"}),
            serde_json::json!({"action": "backup", "every_minutes": 0}),
            serde_json::json!({"action": "backup", "every_minutes": 5, "at": "04:00"}),
            serde_json::json!({"action": "restart", "at": "4pm"}),
            serde_json::json!({"action": "announce", "every_minutes": 60}),
            serde_json::json!({"action": "backup", "every_minutes": 60, "warn_minutes": [5]}),
        ];
        for b in bad {
            assert!(!task(b.clone()).problems().is_empty(), "{}", b);
        }
        let ok = task(serde_json::json!({"action": "announce", "every_minutes": 60, "message": "hi"}));
        assert!(ok.problems().is_empty());
        assert!(ok.enabled);
    }

    #[test]
    fn a_deleted_disabled_or_edited_task_is_no_longer_scheduled() {
        let job = task(serde_json::json!({"action": "restart", "at": "04:00", "warn_minutes": [10]}));
        let key = task_key("srv", &job);
        assert!(still_scheduled("srv", std::slice::from_ref(&job), &key));
        assert!(!still_scheduled("srv", &[], &key), "deleted");
        let off = ScheduledTask { enabled: false, ..job.clone() };
        assert!(!still_scheduled("srv", &[off], &key), "disabled");
        let moved = ScheduledTask { at: Some("05:00".into()), ..job.clone() };
        assert!(!still_scheduled("srv", &[moved], &key), "edited");
        assert!(!still_scheduled("other", std::slice::from_ref(&job), &key));
    }
}
