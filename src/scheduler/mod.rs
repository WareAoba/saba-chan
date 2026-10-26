//! 인스턴스 예약 작업 (A3): 시작·정지·재시작·저장·백업·공지·업데이트·모듈 명령
//!
//! 언제: N분마다(`every_minutes`) 또는 `at`(HH:MM) 시각에 매일, N일마다(`every_days`, `start_date`부터),
//! 정한 요일마다(`weekdays`), 정한 날 한 번(`date`). 시각은 데몬 PC의 시간대로 본다.
//!
//! 인스턴스마다 `instance.json`의 `schedule`에 작업 목록이 있다. 데몬은 30초마다 한 번 보고,
//! 때가 된 작업을 실행한다. 마지막 실행 시각은 메모리에만 둔다: 데몬을 다시 켜면 주기 작업은
//! 그때부터 다시 세고, 시각 작업은 지나간 시각을 따라잡지 않는다 (밀린 재시작이 한꺼번에 돌지 않게).
//! 데몬이 켜진 뒤 더하거나 고친 작업은 그때부터 세고, 밀린 차례(PC 절전 등)는 가장 최근 것 한 번만 실행한다.
//! 재시작·업데이트는 `warn_minutes`만큼 먼저 깨어나 게임 안에 예고한 뒤 정한 시각에 실행한다.

use chrono::{Datelike, Duration, LocalResult, NaiveDate, NaiveDateTime, NaiveTime, TimeZone};
use serde::{Deserialize, Serialize};
use serde_json::{Map, Value};
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
    /// 꺼져 있으면 시작
    Start,
    /// 켜져 있으면 정지 (예고할 수 있다)
    Stop,
    /// 켜져 있으면 모듈의 저장 명령 (`[backup] save_command`)
    Save,
    /// 모듈·인스턴스가 주는 게임 명령 (`command` + `args`)
    Command,
}

/// `weekdays`에 쓰는 이름 (월요일부터)
pub const WEEKDAYS: [&str; 7] = ["mon", "tue", "wed", "thu", "fri", "sat", "sun"];
/// `every_days`의 상한 (1년)
pub const MAX_EVERY_DAYS: u32 = 365;

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ScheduledTask {
    pub action: TaskAction,
    /// N분마다. `at`과 둘 중 하나
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub every_minutes: Option<u32>,
    /// 이 시각 (데몬 PC 시간대, "HH:MM"). 아래 날 규칙이 없으면 매일
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub at: Option<String>,
    /// `at`과 함께: N일마다 (`start_date`부터 센다)
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub every_days: Option<u32>,
    /// `every_days`를 세기 시작하는 날 ("YYYY-MM-DD", 그 전에는 돌지 않는다). 없으면 저장할 때 데몬이 오늘로 채운다
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub start_date: Option<String>,
    /// `at`과 함께: 이 요일들에만 (`WEEKDAYS`)
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub weekdays: Vec<String>,
    /// `at`과 함께: 이 날 한 번만 ("YYYY-MM-DD")
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub date: Option<String>,
    /// 재시작·업데이트 몇 분 전에 게임 안에 예고할지 (예: [10, 5, 1])
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub warn_minutes: Vec<u32>,
    /// 공지 문구 (announce)
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub message: Option<String>,
    /// 실행할 게임 명령 이름 (command)
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub command: Option<String>,
    /// 게임 명령 인자 (입력 이름 → 값, command)
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub args: Option<Map<String, Value>>,
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
        let rules = [self.every_days.is_some(), !self.weekdays.is_empty(), self.date.is_some()];
        let rule_count = rules.iter().filter(|r| **r).count();
        if self.every_minutes.is_some() && rule_count > 0 {
            out.push("every_days, weekdays and date need at, not every_minutes".into());
        }
        if rule_count > 1 {
            out.push("set at most one of every_days, weekdays or date".into());
        }
        if let Some(n) = self.every_days {
            if !(1..=MAX_EVERY_DAYS).contains(&n) {
                out.push(format!("every_days must be between 1 and {}", MAX_EVERY_DAYS));
            }
        }
        match (&self.start_date, self.every_days) {
            (Some(_), None) => out.push("start_date only applies to every_days".into()),
            (Some(d), Some(_)) if parse_date(d).is_none() => out.push(format!("start_date '{}' must be YYYY-MM-DD", d)),
            _ => {}
        }
        for (i, w) in self.weekdays.iter().enumerate() {
            if !WEEKDAYS.contains(&w.as_str()) {
                out.push(format!("weekday '{}' must be one of {}", w, WEEKDAYS.join(", ")));
            } else if self.weekdays[..i].contains(w) {
                out.push(format!("weekday '{}' is listed twice", w));
            }
        }
        if let Some(d) = &self.date {
            if parse_date(d).is_none() {
                out.push(format!("date '{}' must be YYYY-MM-DD", d));
            }
        }
        if self.action == TaskAction::Announce && self.message.as_deref().unwrap_or("").trim().is_empty() {
            out.push("announce needs a message".into());
        }
        if !self.warn_minutes.is_empty() && !matches!(self.action, TaskAction::Restart | TaskAction::Update | TaskAction::Stop) {
            out.push("warn_minutes only applies to restart, update and stop".into());
        }
        if self.action == TaskAction::Command {
            let name = self.command.as_deref().unwrap_or("");
            if name.trim().is_empty() || name.contains(char::is_whitespace) || crate::commands::has_line_break(name) {
                out.push("command needs one command name without spaces".into());
            }
        } else if self.command.is_some() || self.args.is_some() {
            out.push("command and args only apply to the command action".into());
        }
        out
    }

    /// 이 날이 작업을 하는 날인지 (`at` 작업)
    fn runs_on(&self, day: NaiveDate) -> bool {
        if let Some(n) = self.every_days {
            let from = self.start_date.as_deref().and_then(parse_date).unwrap_or(NaiveDate::MIN);
            return day >= from && (day - from).num_days() % n.max(1) as i64 == 0;
        }
        if !self.weekdays.is_empty() {
            let name = WEEKDAYS[day.weekday().num_days_from_monday() as usize];
            return self.weekdays.iter().any(|w| w == name);
        }
        match &self.date {
            Some(d) => parse_date(d) == Some(day),
            None => true,
        }
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
        // 예고 시간만큼 먼저 깨어나므로, 깨어날 시각이 last보다 뒤이고 작업하는 날인 가장 이른 슬롯
        // 한 번뿐인 날은 예고 시간 안에 저장했거나 데몬이 다시 켜졌어도 놓치지 않는다 (남은 예고만 한다)
        if let Some(day) = self.date.as_deref() {
            let slot = parse_date(day)?.and_time(at);
            return (slot > last).then_some(slot);
        }
        let mut day = (last + self.lead()).date() - Duration::days(1);
        if let Some(from) = self.every_days.and(self.start_date.as_deref()).and_then(parse_date) {
            day = day.max(from);
        }
        // N일마다는 N일 안에, 요일은 7일 안에 한 번은 맞는다 (하루 더 여유)
        for _ in 0..=MAX_EVERY_DAYS + 2 {
            let slot = day.and_time(at);
            if slot - self.lead() > last && self.runs_on(day) {
                return Some(slot);
            }
            day = day.succ_opt()?;
        }
        None
    }
}

fn parse_at(at: &str) -> Option<NaiveTime> {
    NaiveTime::parse_from_str(at.trim(), "%H:%M").ok()
}

fn parse_date(day: &str) -> Option<NaiveDate> {
    NaiveDate::parse_from_str(day.trim(), "%Y-%m-%d").ok()
}

/// 데몬 PC 시각(벽시계) → 실제 시각. 서머타임으로 없는 시각(시계가 건너뛴 사이)은 시계가 다시 이어지는 첫 시각,
/// 두 번 오는 시각은 처음 것. 실행기는 벽시계가 그 시각을 지나는 첫 확인에 돌리므로 그때와 같다
pub fn instant<Tz: TimeZone>(tz: &Tz, wall: NaiveDateTime) -> Option<chrono::DateTime<Tz>> {
    match tz.from_local_datetime(&wall) {
        LocalResult::Single(t) | LocalResult::Ambiguous(t, _) => Some(t),
        LocalResult::None => (1..=24 * 60).find_map(|m| tz.from_local_datetime(&(wall + Duration::minutes(m))).earliest()),
    }
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

    /// 다음에 실행할 시각 (화면에 보일 것, 상태를 바꾸지 않는다). 지난 시각이면 다음 확인 때 돈다
    pub fn peek(&self, key: &str, task: &ScheduledTask, started: NaiveDateTime, now: NaiveDateTime) -> Option<NaiveDateTime> {
        if !task.enabled || !task.problems().is_empty() {
            return None;
        }
        let base = if self.first_scan.is_none() { started } else { now };
        task.next_fire(self.last.get(key).copied().unwrap_or(base))
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

    #[test]
    fn day_rules_pick_the_next_matching_day() {
        // 2026-10-26은 월요일
        let weekly = task(serde_json::json!({"action": "stop", "at": "23:00", "weekdays": ["wed", "sat"]}));
        assert_eq!(weekly.next_fire(t("2026-10-26 12:00")), Some(t("2026-10-28 23:00")));
        assert_eq!(weekly.next_fire(t("2026-10-28 23:00")), Some(t("2026-10-31 23:00")));

        let every3 = task(serde_json::json!({"action": "save", "at": "04:00", "every_days": 3, "start_date": "2026-10-30"}));
        assert_eq!(every3.next_fire(t("2026-10-26 12:00")), Some(t("2026-10-30 04:00")), "waits for start_date");
        assert_eq!(every3.next_fire(t("2026-10-30 04:00")), Some(t("2026-11-02 04:00")), "across the month end");

        let once = task(serde_json::json!({"action": "start", "at": "18:30", "date": "2026-12-24"}));
        assert_eq!(once.next_fire(t("2026-10-26 12:00")), Some(t("2026-12-24 18:30")));
        assert_eq!(once.next_fire(t("2026-12-24 18:30")), None, "only once");
    }

    #[test]
    fn a_once_task_that_ran_does_not_run_again_and_a_past_one_never_runs() {
        let mut c = Clock::default();
        let once = task(serde_json::json!({"action": "start", "at": "18:30", "date": "2026-10-27"}));
        let start = t("2026-10-26 10:00");
        let fire = c.due("o", &once, start, t("2026-10-27 18:30")).unwrap();
        c.finish("o", fire);
        assert_eq!(c.due("o", &once, start, t("2026-10-28 18:30")), None);
        // 예고 시간 안에 저장한 한 번뿐인 정지도 그 시각에 한다
        let soon = task(serde_json::json!({"action": "stop", "at": "04:00", "date": "2026-10-27", "warn_minutes": [10]}));
        assert_eq!(c.due("s", &soon, start, t("2026-10-27 03:55")), Some(t("2026-10-27 04:00")));
        let past = task(serde_json::json!({"action": "start", "at": "18:30", "date": "2026-10-20"}));
        assert_eq!(c.due("p", &past, start, t("2026-10-26 10:00")), None);
        assert_eq!(c.peek("p", &past, start, t("2026-10-26 10:00")), None);
    }

    #[test]
    fn new_actions_and_day_rules_are_checked() {
        let bad = [
            serde_json::json!({"action": "stop", "every_minutes": 60, "weekdays": ["mon"]}),
            serde_json::json!({"action": "stop", "at": "04:00", "weekdays": ["mon"], "every_days": 2}),
            serde_json::json!({"action": "stop", "at": "04:00", "weekdays": ["monday"]}),
            serde_json::json!({"action": "stop", "at": "04:00", "weekdays": ["mon", "mon"]}),
            serde_json::json!({"action": "stop", "at": "04:00", "every_days": 0}),
            serde_json::json!({"action": "stop", "at": "04:00", "every_days": 366}),
            serde_json::json!({"action": "stop", "at": "04:00", "start_date": "2026-10-26"}),
            serde_json::json!({"action": "stop", "at": "04:00", "date": "2027-02-29"}),
            serde_json::json!({"action": "command", "at": "04:00"}),
            serde_json::json!({"action": "command", "at": "04:00", "command": "say hi"}),
            serde_json::json!({"action": "save", "at": "04:00", "command": "say"}),
            serde_json::json!({"action": "save", "at": "04:00", "warn_minutes": [5]}),
        ];
        for b in bad {
            assert!(!task(b.clone()).problems().is_empty(), "{}", b);
        }
        let ok = [
            serde_json::json!({"action": "stop", "at": "04:00", "warn_minutes": [5], "weekdays": ["mon", "sun"]}),
            serde_json::json!({"action": "command", "every_minutes": 60, "command": "say", "args": {"message": "hi"}}),
            serde_json::json!({"action": "start", "at": "04:00", "date": "2028-02-29"}),
        ];
        for o in ok {
            assert!(task(o.clone()).problems().is_empty(), "{}", o);
        }
    }

    /// 예전에 저장한 작업은 모양도 열쇠도 그대로다 (저장 형식 이전 없음)
    #[test]
    fn old_tasks_keep_their_shape() {
        let old = serde_json::json!({"action": "restart", "at": "04:00", "warn_minutes": [10], "enabled": true});
        assert_eq!(serde_json::to_value(task(old.clone())).unwrap(), old);
    }

    /// 서머타임: 건너뛴 시각은 시계가 이어지는 첫 시각, 두 번 오는 시각은 처음 것
    #[test]
    fn wall_times_in_a_daylight_saving_change_resolve_like_the_runner() {
        let tz = FakeDst;
        // 03-08 02:00 → 03:00 (건너뜀)
        let gap = instant(&tz, t("2026-03-08 02:30")).unwrap();
        assert_eq!(gap.naive_local(), t("2026-03-08 03:00"));
        // 11-01 02:00 → 01:00 (01:xx 두 번)
        let twice = instant(&tz, t("2026-11-01 01:30")).unwrap();
        assert_eq!(twice.naive_utc(), t("2026-11-01 00:30"), "the first, summer one");
        assert_eq!(instant(&tz, t("2026-07-01 12:00")).unwrap().naive_utc(), t("2026-07-01 11:00"));
    }

    /// 표준시 UTC+0, 서머타임 UTC+1 (03-08 02:00 ~ 11-01 02:00, 2026년만)
    #[derive(Clone, Copy, Debug)]
    struct FakeDst;
    impl FakeDst {
        fn summer_utc(utc: &NaiveDateTime) -> bool {
            *utc >= t("2026-03-08 02:00") && *utc < t("2026-11-01 01:00")
        }
    }
    impl TimeZone for FakeDst {
        type Offset = chrono::FixedOffset;
        fn from_offset(_: &chrono::FixedOffset) -> Self {
            FakeDst
        }
        fn offset_from_local_date(&self, local: &NaiveDate) -> LocalResult<chrono::FixedOffset> {
            self.offset_from_local_datetime(&local.and_hms_opt(12, 0, 0).unwrap())
        }
        fn offset_from_local_datetime(&self, local: &NaiveDateTime) -> LocalResult<chrono::FixedOffset> {
            let std = chrono::FixedOffset::east_opt(0).unwrap();
            let dst = chrono::FixedOffset::east_opt(3600).unwrap();
            let as_std = Self::summer_utc(local);
            let as_dst = Self::summer_utc(&(*local - Duration::hours(1)));
            match (as_dst, !as_std) {
                (true, true) => LocalResult::Ambiguous(dst, std),
                (true, false) => LocalResult::Single(dst),
                (false, true) => LocalResult::Single(std),
                (false, false) => LocalResult::None,
            }
        }
        fn offset_from_utc_date(&self, utc: &NaiveDate) -> chrono::FixedOffset {
            self.offset_from_utc_datetime(&utc.and_hms_opt(12, 0, 0).unwrap())
        }
        fn offset_from_utc_datetime(&self, utc: &NaiveDateTime) -> chrono::FixedOffset {
            chrono::FixedOffset::east_opt(if Self::summer_utc(utc) { 3600 } else { 0 }).unwrap()
        }
    }

    mod props {
        use super::*;
        use proptest::prelude::*;

        fn day(n: i64) -> NaiveDate {
            NaiveDate::from_ymd_opt(2024, 1, 1).unwrap() + Duration::days(n)
        }

        /// 월말·윤년이 자주 걸리게 날을 고른다 (2024~2031년, 28~31일과 1일에 몰아서)
        fn any_day() -> impl Strategy<Value = NaiveDate> {
            prop_oneof![
                (0i64..2920).prop_map(day),
                (2024i32..2032, 1u32..=12, prop_oneof![Just(1u32), 28u32..=31]).prop_map(|(y, m, d)| {
                    (0..4).find_map(|k| NaiveDate::from_ymd_opt(y, m, d - k.min(d - 1))).unwrap()
                }),
            ]
        }

        fn any_rule() -> impl Strategy<Value = Value> {
            prop_oneof![
                Just(serde_json::json!({})),
                (1u32..=MAX_EVERY_DAYS, any_day())
                    .prop_map(|(n, from)| serde_json::json!({"every_days": n, "start_date": from.to_string()})),
                proptest::sample::subsequence(WEEKDAYS.to_vec(), 1..=7).prop_map(|w| serde_json::json!({"weekdays": w})),
                any_day().prop_map(|d| serde_json::json!({"date": d.to_string()})),
            ]
        }

        fn any_task() -> impl Strategy<Value = ScheduledTask> {
            (any_rule(), 0u32..24, 0u32..60, proptest::collection::vec(1u32..3000, 0..3)).prop_map(|(rule, h, m, warn)| {
                let mut v = serde_json::json!({"action": "stop", "at": format!("{:02}:{:02}", h, m), "warn_minutes": warn});
                v.as_object_mut().unwrap().extend(rule.as_object().unwrap().clone());
                task(v)
            })
        }

        fn any_moment() -> impl Strategy<Value = NaiveDateTime> {
            (any_day(), 0u32..24 * 60).prop_map(|(d, m)| d.and_hms_opt(m / 60, m % 60, 0).unwrap())
        }

        proptest! {
            /// 다음 차례는 정한 시각이고, 작업하는 날이며, 예고가 last 뒤에 시작하는 가장 이른 날이다
            #[test]
            fn next_fire_is_the_earliest_matching_slot(job in any_task(), last in any_moment()) {
                prop_assert!(job.problems().is_empty(), "{:?}", job.problems());
                let at = parse_at(job.at.as_deref().unwrap()).unwrap();
                let lead = job.lead();
                // 대조: 하루씩 넘겨 보는 단순한 방법
                let mut oracle = None;
                let mut d = last.date() - Duration::days(1);
                for _ in 0..3200 {
                    let slot = d.and_time(at);
                    let matches = match (&job.date, job.every_days, job.weekdays.is_empty()) {
                        (Some(x), _, _) => parse_date(x) == Some(d) && slot > last,
                        (_, Some(n), _) => {
                            let from = parse_date(job.start_date.as_deref().unwrap()).unwrap();
                            d >= from && (d - from).num_days() % n as i64 == 0
                        }
                        (_, _, false) => job.weekdays.contains(&d.format("%a").to_string().to_lowercase()),
                        _ => true,
                    };
                    if matches && (job.date.is_some() || slot - lead > last) {
                        oracle = Some(slot);
                        break;
                    }
                    d = d.succ_opt().unwrap();
                }
                let next = job.next_fire(last);
                prop_assert_eq!(next, oracle);
                if let Some(next) = next {
                    prop_assert_eq!(next.time(), at);
                    prop_assert!(next - lead > last || (job.date.is_some() && next > last));
                    // 다음 차례에서 다시 구하면 늘 더 뒤다 (같은 차례를 두 번 하지 않는다)
                    if let Some(after) = job.next_fire(next) {
                        prop_assert!(after > next);
                    }
                }
            }

            /// N분마다는 마지막 실행에서 정확히 N분 뒤
            #[test]
            fn every_minutes_is_last_plus_n(n in 1u32..100_000, last in any_moment()) {
                let job = task(serde_json::json!({"action": "backup", "every_minutes": n}));
                prop_assert_eq!(job.next_fire(last), Some(last + Duration::minutes(n as i64)));
            }

            /// 실행기는 같은 차례를 두 번 하지 않고, 차례는 늘 앞으로 간다
            #[test]
            fn the_clock_never_fires_a_slot_twice(job in any_task(), start in any_moment(), steps in proptest::collection::vec(1i64..20_000, 1..30)) {
                let mut c = Clock::default();
                let mut now = start;
                let mut fired: Vec<NaiveDateTime> = Vec::new();
                for step in steps {
                    now += Duration::minutes(step);
                    if let Some(fire) = c.due("k", &job, start, now) {
                        prop_assert!(fired.last().is_none_or(|prev| fire > *prev), "{:?} after {:?}", fire, fired);
                        c.finish("k", fire);
                        fired.push(fire);
                    }
                }
            }
        }
    }
}
