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
    }
    let mut asks = tokio::task::JoinSet::new();
    for (id, call) in calls {
        asks.spawn(async move {
            let result = crate::plugin::run_plugin_with_timeout(&call.entry, &call.function, call.config, CALL_TIMEOUT_SECS).await;
            if let Err(e) = &result {
                tracing::debug!("player count for {} failed: {}", id, e);
            }
            set(&id, result.as_ref().ok().and_then(parse));
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
}
