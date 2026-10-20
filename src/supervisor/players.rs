//! 접속자 수 (Q-5). 모듈이 `[status] players`로 lifecycle 함수를 선언하면 데몬이 켜진 서버마다
//! 주기적으로 불러 마지막 값을 기억한다. 서버 목록(`ServerInfo.players`)과 relay 하트비트가 이 값을 읽는다.
//! 함수는 `{"success": true, "online": <정수>, "max": <정수>?}`를 돌려준다 (02-modules §2.3)

use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::collections::HashMap;
use std::sync::{Mutex, OnceLock};

use super::Supervisor;

/// 다시 묻는 간격
pub const REFRESH_SECS: u64 = 60;
/// 함수 하나를 기다리는 시간. 넘기면 그 서버의 값은 비운다
const CALL_TIMEOUT_SECS: u64 = 10;
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

/// 인스턴스의 마지막 접속자 수. 꺼져 있거나 모듈이 알려 주지 않으면 None
pub fn get(instance_id: &str) -> Option<PlayerCount> {
    counts().lock().unwrap_or_else(|e| e.into_inner()).get(instance_id).copied()
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
    let max = int("max").filter(|m| *m > 0);
    Some(PlayerCount { online, max })
}

/// 켜진 서버 중 모듈이 접속자 함수를 선언한 것만 묻고, 나머지는 기억을 지운다
pub async fn refresh(sup: &tokio::sync::RwLock<Supervisor>) {
    let (calls, idle): (Vec<_>, Vec<_>) = {
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
    }
    let mut asks = tokio::task::JoinSet::new();
    for (id, pid, call) in calls {
        asks.spawn(async move {
            // `run`으로 불러야 서버 PID가 함께 간다 (격리 실행의 루프백 중계가 그 서버의 포트만 잇는다)
            let result = call.timeout(CALL_TIMEOUT_SECS).run().await;
            if let Err(e) = &result {
                tracing::debug!("player count for {} failed: {}", id, e);
            }
            set(&id, result.as_ref().ok().and_then(parse));
            record(&id, Some(pid), result.as_ref().map_err(|e| e.to_string()), std::time::Instant::now());
        });
    }
    while asks.join_next().await.is_some() {}
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

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
