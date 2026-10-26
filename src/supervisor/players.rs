//! 접속자 수 (Q-5). 모듈이 `[status] players`로 lifecycle 함수를 선언하면 데몬이 켜진 서버마다
//! 주기적으로 불러 마지막 값을 기억한다. 서버 목록(`ServerInfo.players`)과 relay 하트비트가 이 값을 읽는다.
//! 함수는 `{"success": true, "online": <정수>, "max": <정수>?}`를 돌려준다 (02-modules §2.3)
//!
//! 모듈이 수를 주지 않으면(함수가 없거나 `online` 없이 답함) 접속·퇴장 문구로 센 수(`player_events`)를 쓴다.
//! 그 수는 데몬이 콘솔을 처음부터 읽은 프로세스에서만 세고, 프로세스가 끝나면 지운다

use serde::{Deserialize, Serialize};
use futures_util::StreamExt;
use serde_json::Value;
use std::collections::HashMap;
use std::sync::{Mutex, OnceLock};

use super::Supervisor;

/// 다시 묻는 간격
pub const REFRESH_SECS: u64 = 60;
/// 함수 하나를 기다리는 시간. 넘기면 그 서버의 값은 비운다
pub const CALL_TIMEOUT_SECS: u64 = 10;
/// 켜진 서버가 이만큼 계속 답하지 않으면 응답 없음으로 알린다. 첫 월드 생성(Minecraft)처럼 늦게 여는 서버를 기다린다
pub const UNRESPONSIVE_AFTER: std::time::Duration = std::time::Duration::from_secs(300);

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct PlayerCount {
    pub online: u32,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub max: Option<u32>,
}

fn counts() -> &'static Mutex<HashMap<String, PlayerCount>> {
    static COUNTS: OnceLock<Mutex<HashMap<String, PlayerCount>>> = OnceLock::new();
    COUNTS.get_or_init(Default::default)
}

/// 인스턴스의 마지막 접속자 수. 모듈이 알려 준 수가 먼저고, 없으면 접속·퇴장 문구로 센 수. 둘 다 없으면 None
pub fn get(instance_id: &str) -> Option<PlayerCount> {
    if let Some(count) = counts().lock().unwrap_or_else(|e| e.into_inner()).get(instance_id).copied() {
        return Some(count);
    }
    // 모듈 함수가 실패한 동안에는 세지 않은 것처럼 비운다(두 출처 사이를 오가지 않게)
    let max = *fallbacks().lock().unwrap_or_else(|e| e.into_inner()).get(instance_id)?;
    let online = rosters().lock().unwrap_or_else(|e| e.into_inner()).get(instance_id).map(|r| u32::try_from(r.names.len()).unwrap_or(u32::MAX))?;
    Some(PlayerCount { online, max })
}

/// 접속·퇴장 문구로 센 접속자 이름. 같은 이름이 다시 들어오면 하나로 본다(퇴장 줄을 놓쳐도 다시 들어오면 바로잡힌다).
/// `generation`은 그 프로세스를 지켜보는 쪽 표시다
struct Roster {
    generation: u64,
    names: std::collections::HashSet<String>,
}

fn rosters() -> &'static Mutex<HashMap<String, Roster>> {
    static ROSTERS: OnceLock<Mutex<HashMap<String, Roster>>> = OnceLock::new();
    ROSTERS.get_or_init(Default::default)
}

/// 센 수를 써도 되는 인스턴스와 붙일 최대 인원: 모듈에 접속자 함수가 없거나(None), 함수가 수 없이 답했다
/// (`{"success": true, "max": 8}` → Some(8)). 함수가 실패하면 빠진다. 락은 한 번에 하나만 잡는다
fn fallbacks() -> &'static Mutex<HashMap<String, Option<u32>>> {
    static FALLBACKS: OnceLock<Mutex<HashMap<String, Option<u32>>>> = OnceLock::new();
    FALLBACKS.get_or_init(Default::default)
}

/// 새 프로세스의 콘솔을 처음부터 보기 시작한다: 0명에서 센다. 돌려준 값을 `roster_change`·`roster_end`에 넘긴다
pub fn roster_start(instance_id: &str) -> u64 {
    static NEXT: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(1);
    let generation = NEXT.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
    rosters().lock().unwrap_or_else(|e| e.into_inner()).insert(instance_id.to_string(), Roster { generation, names: Default::default() });
    generation
}

/// 접속(`joined`)·퇴장 한 줄. 모르는 이름의 퇴장은 무시한다. 다른 프로세스의 것이면 무시한다
pub fn roster_change(instance_id: &str, generation: u64, name: &str, joined: bool) {
    let mut map = rosters().lock().unwrap_or_else(|e| e.into_inner());
    let Some(roster) = map.get_mut(instance_id).filter(|r| r.generation == generation) else { return };
    if joined {
        roster.names.insert(name.to_string());
    } else {
        roster.names.remove(name);
    }
}

/// 프로세스가 끝났다. 그 사이 새 프로세스가 세기 시작했으면 그대로 둔다
pub fn roster_end(instance_id: &str, generation: u64) {
    let mut map = rosters().lock().unwrap_or_else(|e| e.into_inner());
    if map.get(instance_id).is_some_and(|r| r.generation == generation) {
        map.remove(instance_id);
    }
}

fn set(instance_id: &str, count: Option<PlayerCount>) {
    let mut map = counts().lock().unwrap_or_else(|e| e.into_inner());
    match count {
        Some(c) => map.insert(instance_id.to_string(), c),
        None => map.remove(instance_id),
    };
}

/// 계속 실패한 서버: 그 프로세스의 pid, 처음 실패한 때, 마지막 실패 이유, 로그에 남겼는지.
/// pid가 다르면 재시작한 새 프로세스이므로 이전 기록을 잇지 않는다
type Failure = (Option<u32>, std::time::Instant, String, bool);

fn failures() -> &'static Mutex<HashMap<String, Failure>> {
    static FAILURES: OnceLock<Mutex<HashMap<String, Failure>>> = OnceLock::new();
    FAILURES.get_or_init(Default::default)
}

/// 한 번의 결과를 적는다. 답하면 실패 기록을 지우고, 실패하면 처음 실패한 때는 두고 이유만 바꾼다.
/// 수 없이 `{"success": true}`만 돌려준 것도 답이다 (모듈이 수를 알 수 없는 설정, 예: 마인크래프트 enable-status=false)
fn record(instance_id: &str, pid: Option<u32>, result: Result<&Value, String>, now: std::time::Instant) {
    let reason = match result {
        Ok(value) if parse(value).is_some() || answered_without_count(value) => {
            failures().lock().unwrap_or_else(|e| e.into_inner()).remove(instance_id);
            return;
        }
        Ok(value) => value.get("message").and_then(Value::as_str).unwrap_or("no answer").to_string(),
        Err(e) => e,
    };
    let mut map = failures().lock().unwrap_or_else(|e| e.into_inner());
    let entry = map.entry(instance_id.to_string()).or_insert((pid, now, String::new(), false));
    if entry.0 != pid {
        *entry = (pid, now, String::new(), false);
    }
    entry.2 = reason;
    if !entry.3 && now.duration_since(entry.1) >= UNRESPONSIVE_AFTER {
        entry.3 = true;
        tracing::warn!("Server {} has not answered its status query for {:?}: {}", instance_id, now.duration_since(entry.1), entry.2);
    }
}

fn answered_without_count(value: &Value) -> bool {
    value.get("success").and_then(Value::as_bool) == Some(true) && value.get("online").is_none_or(Value::is_null)
}

fn forget(instance_id: &str) {
    failures().lock().unwrap_or_else(|e| e.into_inner()).remove(instance_id);
}

/// 켜진 서버가 `UNRESPONSIVE_AFTER` 넘게 상태 질문(접속자 수)에 답하지 않으면 마지막 실패 이유.
/// 그 pid의 프로세스에서 센 것만 본다. 팰월드처럼 REST 포트를 열지 못하면 명령·정상 종료도 닿지 않는다
pub fn unresponsive(instance_id: &str, pid: Option<u32>, now: std::time::Instant) -> Option<String> {
    let map = failures().lock().unwrap_or_else(|e| e.into_inner());
    let (failed_pid, since, reason, logged) = map.get(instance_id)?;
    // 데몬 로그에 경고를 남긴 뒤에만 알린다: 화면과 로그가 어긋나지 않게
    (*failed_pid == pid && *logged && now.duration_since(*since) >= UNRESPONSIVE_AFTER).then(|| reason.clone())
}

/// 모듈 응답을 읽는다. 실패 응답이나 정수가 아닌 값은 None (모르는 값을 0으로 보이지 않는다)
pub fn parse(result: &Value) -> Option<PlayerCount> {
    if result.get("success").and_then(Value::as_bool) == Some(false) {
        return None;
    }
    let int = |key: &str| result.get(key).and_then(Value::as_u64).and_then(|n| u32::try_from(n).ok());
    let online = int("online")?;
    Some(PlayerCount { online, max: max_of(result) })
}

/// 답의 `max`. 0이거나 정수가 아니면 모른다
fn max_of(result: &Value) -> Option<u32> {
    result.get("max").and_then(Value::as_u64).and_then(|n| u32::try_from(n).ok()).filter(|m| *m > 0)
}

/// 켜진 서버 중 모듈이 접속자 함수를 선언한 것만 묻고, 나머지는 기억을 지운다
pub async fn refresh(sup: &tokio::sync::RwLock<Supervisor>) {
    let (mut calls, idle): (Vec<_>, Vec<_>) = {
        let s = sup.read().await;
        let mut calls = Vec::new();
        let mut idle = Vec::new();
        for instance in s.instance_store.list() {
            let function = s.module_loader.get_module(&instance.module_name).ok().and_then(|m| m.metadata.players_function.clone());
            let call = match (function, s.tracker.get_pid(&instance.id)) {
                (Some(f), Ok(pid)) => s.instance_call(&instance.id, &f).ok().map(|c| (pid, c)),
                _ => None,
            };
            match call {
                Some((pid, c)) => calls.push((instance.id.clone(), pid, c)),
                None => idle.push(instance.id.clone()),
            }
        }
        (calls, idle)
    };
    for id in idle {
        set(&id, None);
        forget(&id);
        fallbacks().lock().unwrap_or_else(|e| e.into_inner()).insert(id, None);
    }
    static START: Mutex<usize> = Mutex::new(0);
    calls.sort_by(|a, b| a.0.cmp(&b.0));
    rotate_calls(&mut calls, &mut START.lock().unwrap_or_else(|e| e.into_inner()));
    let mut asks = futures_util::stream::FuturesUnordered::new();
    for (id, pid, call) in calls {
        asks.push(async move {
            // 앞에서 돌린 순서로 자리를 시도한다. 기다리는 인스턴스는 이번 갱신을 건너뛴다
            let result = call.timeout(CALL_TIMEOUT_SECS).try_run().await;
            apply_refresh(&id, Some(pid), result, std::time::Instant::now());
        });
    }
    while asks.next().await.is_some() {}
}

fn rotate_calls<T>(calls: &mut [T], start: &mut usize) {
    if !calls.is_empty() {
        calls.rotate_left(*start % calls.len());
        *start = (*start + 1) % calls.len();
    }
}

fn apply_refresh(id: &str, pid: Option<u32>, result: anyhow::Result<Option<Value>>, now: std::time::Instant) {
    if matches!(result, Ok(None)) {
        return;
    }
    let result = result.map(|value| value.expect("skipped above"));
    if let Err(e) = &result {
        tracing::debug!("player count for {} failed: {}", id, e);
    }
    set(id, result.as_ref().ok().and_then(parse));
    {
        let mut fallbacks = fallbacks().lock().unwrap_or_else(|e| e.into_inner());
        match result.as_ref() {
            Ok(value) if answered_without_count(value) => fallbacks.insert(id.to_string(), max_of(value)),
            _ => fallbacks.remove(id),
        };
    }
    record(id, pid, result.as_ref().map_err(|e| e.to_string()), now);
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    // 테라리아처럼 조회 API가 없는 게임: 콘솔 접속·퇴장 문구로 센 수를 쓴다
    #[test]
    fn join_and_leave_lines_count_players_when_the_module_gives_no_count() {
        let id = "roster-count-test";
        assert_eq!(get(id), None);
        let generation = roster_start(id);
        assert_eq!(get(id), None, "not used until the module is known to give no count");
        fallbacks().lock().unwrap().insert(id.to_string(), None);
        assert_eq!(get(id), Some(PlayerCount { online: 0, max: None }));
        roster_change(id, generation, "Bob", true);
        roster_change(id, generation, "사바쨩", true);
        roster_change(id, generation, "Ghost", false);
        assert_eq!(get(id), Some(PlayerCount { online: 2, max: None }));
        roster_change(id, generation, "Bob", true);
        assert_eq!(get(id).unwrap().online, 2, "a rejoin after a missed leave is one player");
        roster_change(id, generation, "Bob", false);
        roster_change(id, generation, "Bob", false);
        assert_eq!(get(id).unwrap().online, 1, "a second leave does not go below zero");

        // 모듈이 수 없이 최대 인원만 알려 주면 그 값을 붙인다
        apply_refresh(id, Some(1), Ok(Some(json!({ "success": true, "max": 8 }))), std::time::Instant::now());
        assert_eq!(get(id), Some(PlayerCount { online: 1, max: Some(8) }));
        // 모듈이 준 수가 먼저다
        apply_refresh(id, Some(1), Ok(Some(json!({ "success": true, "online": 5, "max": 8 }))), std::time::Instant::now());
        assert_eq!(get(id), Some(PlayerCount { online: 5, max: Some(8) }));
        // 모듈 함수가 실패하면 센 수로 바꿔 보이지 않는다
        apply_refresh(id, Some(1), Err(anyhow::anyhow!("timeout")), std::time::Instant::now());
        assert_eq!(get(id), None);
        apply_refresh(id, Some(1), Ok(Some(json!({ "success": true }))), std::time::Instant::now());
        assert_eq!(get(id), Some(PlayerCount { online: 1, max: None }));

        roster_end(id, generation);
        assert_eq!(get(id), None, "a stopped server has no count");
        forget(id);
        fallbacks().lock().unwrap().remove(id);
    }

    #[test]
    fn a_restart_starts_from_zero_and_the_old_watch_cannot_touch_it() {
        let id = "roster-restart-test";
        fallbacks().lock().unwrap().insert(id.to_string(), None);
        let old = roster_start(id);
        roster_change(id, old, "Bob", true);
        let new = roster_start(id);
        assert_eq!(get(id).unwrap().online, 0);
        roster_change(id, old, "Alex", true);
        roster_end(id, old);
        assert_eq!(get(id), Some(PlayerCount { online: 0, max: None }));
        roster_end(id, new);
        assert_eq!(get(id), None);
        fallbacks().lock().unwrap().remove(id);
    }

    #[test]
    fn each_refresh_rotates_the_first_instance() {
        let mut calls = vec!["a", "b", "c"];
        let mut start = 0;
        let mut first = Vec::new();
        for _ in 0..4 {
            calls.sort();
            rotate_calls(&mut calls, &mut start);
            first.push(calls[0]);
        }
        assert_eq!(first, ["a", "b", "c", "a"]);
    }

    #[test]
    fn skipping_a_busy_lane_preserves_count_and_failure_history() {
        let id = "busy-lane-test";
        let now = std::time::Instant::now();
        set(id, Some(PlayerCount { online: 3, max: None }));
        record(id, Some(123), Err("previous failure".into()), now);
        apply_refresh(id, Some(123), Ok(None), now + UNRESPONSIVE_AFTER);
        assert_eq!(get(id), Some(PlayerCount { online: 3, max: None }));
        assert_eq!(unresponsive(id, Some(123), now + UNRESPONSIVE_AFTER), None);
        assert_eq!(failures().lock().unwrap()[id].1, now);
        set(id, None);
        forget(id);
    }

    #[test]
    fn a_module_reply_becomes_a_count_only_when_it_is_a_real_number() {
        assert_eq!(parse(&json!({ "success": true, "online": 3, "max": 20 })), Some(PlayerCount { online: 3, max: Some(20) }));
        assert_eq!(parse(&json!({ "online": 0 })), Some(PlayerCount { online: 0, max: None }));
        assert_eq!(parse(&json!({ "online": 2, "max": 0 })), Some(PlayerCount { online: 2, max: None }), "max 0 means unknown");
        for bad in [json!({ "success": false, "online": 3 }), json!({ "online": -1 }), json!({ "online": "3" }), json!({ "online": 1.5 }), json!({})] {
            assert_eq!(parse(&bad), None, "{bad}");
        }
        assert_eq!(serde_json::to_value(PlayerCount { online: 1, max: None }).unwrap(), json!({ "online": 1 }));
    }

    // PC 실사용: 팰월드 REST 포트 bind가 실패했는데 데몬은 "실행 중"만 보여 줘서 명령·접속자·종료가 모두 조용히 안 됐다
    #[test]
    fn a_server_that_keeps_failing_its_status_query_is_reported_unresponsive() {
        let id = "unresponsive-test";
        let pid = Some(100);
        let start = std::time::Instant::now();
        let refused = json!({ "success": false, "message": "connection refused" });
        // 막 켜진 서버는 아직 답하지 않는다: 바로 알리지 않는다
        record(id, pid, Err("connection refused".into()), start);
        assert_eq!(unresponsive(id, pid, start + std::time::Duration::from_secs(60)), None);
        record(id, pid, Ok(&refused), start + std::time::Duration::from_secs(60));
        let later = start + UNRESPONSIVE_AFTER;
        // PC 확인: 화면에는 떴는데 데몬 로그에 경고가 없었다. 경고를 남긴 질문 뒤에야 알린다
        assert_eq!(unresponsive(id, pid, later), None, "not yet logged");
        record(id, pid, Ok(&refused), later);
        assert_eq!(unresponsive(id, pid, later).as_deref(), Some("connection refused"));

        // 한 번이라도 답하면 지운다
        record(id, pid, Ok(&json!({ "success": true, "online": 0 })), later);
        assert_eq!(unresponsive(id, pid, later + UNRESPONSIVE_AFTER), None);

        // 꺼진 서버는 기억하지 않는다
        record(id, pid, Ok(&refused), later);
        forget(id);
        assert_eq!(unresponsive(id, pid, later + UNRESPONSIVE_AFTER * 2), None);
    }

    // 리뷰 282: 마인크래프트 enable-status=false면 핑이 늘 실패해 서버가 꺼질 때까지 거짓 경보가 떴다.
    // 모듈이 수를 모른다고 답하는 것(`{"success": true}`)은 답한 것이다
    #[test]
    fn a_server_that_answers_without_a_count_is_not_unresponsive() {
        let id = "answers-without-count-test";
        let start = std::time::Instant::now();
        record(id, Some(1), Err("timeout".into()), start);
        record(id, Some(1), Ok(&json!({ "success": true })), start + UNRESPONSIVE_AFTER);
        assert_eq!(unresponsive(id, Some(1), start + UNRESPONSIVE_AFTER * 2), None);
    }

    // 리뷰 282: 60초 사이에 끝난 재시작이 이전 프로세스의 실패 시각을 이어받아 곧바로 응답 없음으로 보였다
    #[test]
    fn a_restarted_server_does_not_inherit_the_old_process_failures() {
        let id = "restart-inherits-test";
        let start = std::time::Instant::now();
        record(id, Some(1), Err("timeout".into()), start);
        record(id, Some(1), Err("timeout".into()), start + UNRESPONSIVE_AFTER);
        assert!(unresponsive(id, Some(1), start + UNRESPONSIVE_AFTER).is_some());
        // 새 프로세스: 이전 기록은 보이지 않고, 새로 실패하면 그때부터 다시 센다
        assert_eq!(unresponsive(id, Some(2), start + UNRESPONSIVE_AFTER), None);
        record(id, Some(2), Err("timeout".into()), start + UNRESPONSIVE_AFTER);
        assert_eq!(unresponsive(id, Some(2), start + UNRESPONSIVE_AFTER + std::time::Duration::from_secs(60)), None);
    }
}
