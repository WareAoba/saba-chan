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

/// 계속 실패한 서버: 처음 실패한 때, 마지막 실패 이유, 로그에 남겼는지
type Failure = (std::time::Instant, String, bool);

fn failures() -> &'static Mutex<HashMap<String, Failure>> {
    static FAILURES: OnceLock<Mutex<HashMap<String, Failure>>> = OnceLock::new();
    FAILURES.get_or_init(Default::default)
}

/// 한 번의 결과를 적는다. 답하면 실패 기록을 지우고, 실패하면 처음 실패한 때는 두고 이유만 바꾼다
fn record(instance_id: &str, result: Result<&Value, String>, now: std::time::Instant) {
    let reason = match result {
        Ok(value) if parse(value).is_some() => {
            failures().lock().unwrap_or_else(|e| e.into_inner()).remove(instance_id);
            return;
        }
        Ok(value) => value.get("message").and_then(Value::as_str).unwrap_or("no answer").to_string(),
        Err(e) => e,
    };
    let mut map = failures().lock().unwrap_or_else(|e| e.into_inner());
    let entry = map.entry(instance_id.to_string()).or_insert((now, String::new(), false));
    entry.1 = reason;
    if !entry.2 && now.duration_since(entry.0) >= UNRESPONSIVE_AFTER {
        entry.2 = true;
        tracing::warn!("Server {} has not answered its status query for {:?}: {}", instance_id, now.duration_since(entry.0), entry.1);
    }
}

fn forget(instance_id: &str) {
    failures().lock().unwrap_or_else(|e| e.into_inner()).remove(instance_id);
}

/// 켜진 서버가 `UNRESPONSIVE_AFTER` 넘게 상태 질문(접속자 수)에 답하지 않으면 마지막 실패 이유.
/// 프로세스는 살아 있어도 명령·정상 종료가 닿지 않는 상태다 (팰월드 REST 포트를 열지 못함 등)
pub fn unresponsive(instance_id: &str, now: std::time::Instant) -> Option<String> {
    let map = failures().lock().unwrap_or_else(|e| e.into_inner());
    let (since, reason, _) = map.get(instance_id)?;
    (now.duration_since(*since) >= UNRESPONSIVE_AFTER).then(|| reason.clone())
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
            let call = match function {
                Some(f) if s.tracker.get_pid(&instance.id).is_ok() => s.instance_call(&instance.id, &f).ok(),
                _ => None,
            };
            match call {
                Some(c) => calls.push((instance.id.clone(), c)),
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
    for (id, call) in calls {
        asks.spawn(async move {
            let result = crate::plugin::run_plugin_with_timeout(&call.entry, &call.function, call.config, CALL_TIMEOUT_SECS).await;
            if let Err(e) = &result {
                tracing::debug!("player count for {} failed: {}", id, e);
            }
            set(&id, result.as_ref().ok().and_then(parse));
            record(&id, result.as_ref().map_err(|e| e.to_string()), std::time::Instant::now());
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
        let start = std::time::Instant::now();
        let refused = json!({ "success": false, "message": "connection refused" });
        // 막 켜진 서버는 아직 답하지 않는다: 바로 알리지 않는다
        record(id, Err("connection refused".into()), start);
        assert_eq!(unresponsive(id, start + std::time::Duration::from_secs(60)), None);
        record(id, Ok(&refused), start + std::time::Duration::from_secs(60));
        let later = start + UNRESPONSIVE_AFTER;
        assert_eq!(unresponsive(id, later).as_deref(), Some("connection refused"));

        // 한 번이라도 답하면 지운다
        record(id, Ok(&json!({ "success": true, "online": 0 })), later);
        assert_eq!(unresponsive(id, later + UNRESPONSIVE_AFTER), None);

        // 꺼진 서버는 기억하지 않는다
        record(id, Ok(&refused), later);
        forget(id);
        assert_eq!(unresponsive(id, later + UNRESPONSIVE_AFTER * 2), None);
    }
}
