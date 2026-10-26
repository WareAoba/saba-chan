//! 서버 준비 알림 (R39): 시작 한 번(자동 재시작 포함)마다 `instance.ready {instanceId, instance, ok, code?}`를 한 번 낸다.
//!
//! - 모듈이 `module.toml`의 `[extensions.saba-core] ready_patterns`를 선언했고 데몬이 콘솔을 읽는 시작(managed)이면, 그 문구가 콘솔에 찍힐 때 `ok:true`.
//! - 선언이 없거나 콘솔을 읽지 않는 시작(non-managed, 익스텐션)이면 시작이 성공한 때 바로 `ok:true`.
//! - 준비되기 전에 끝나면 `ok:false`: `start_failed`(시작 실패, 준비 전에 프로세스가 끝남), `stopped_while_starting`(사용자 정지),
//!   `ready_timeout`(`ready_timeout`초가 지남). 원문·경로·콘솔 줄은 싣지 않는다 (I3).

use serde_json::{json, Value};
use std::collections::HashMap;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex, OnceLock};
use std::time::Duration;

use super::managed_process::{LogSource, ManagedProcess};

/// 준비 문구를 기다리는 시간의 기본값 (초)
pub const DEFAULT_READY_TIMEOUT_SECS: u64 = 600;
/// 콘솔을 다시 보는 간격
const POLL: Duration = Duration::from_millis(500);
/// 프로세스가 끝난 것을 본 뒤 종료 판정(`exited`)이 먼저 닫기를 기다리는 시간
const EXIT_GRACE: Duration = Duration::from_secs(2);

/// 준비를 기다리는 시작: 인스턴스 id → 그 시작의 번호
fn pending() -> &'static Mutex<HashMap<String, u64>> {
    static PENDING: OnceLock<Mutex<HashMap<String, u64>>> = OnceLock::new();
    PENDING.get_or_init(Default::default)
}

/// 이번 실행에서 준비 문구가 찍힌 인스턴스. 종료 판정(`take_ready`)이 가져가며 지운다
fn seen() -> &'static Mutex<std::collections::HashSet<String>> {
    static SEEN: OnceLock<Mutex<std::collections::HashSet<String>>> = OnceLock::new();
    SEEN.get_or_init(Default::default)
}

/// 이번 실행에서 준비 문구가 찍혔는가. 한 번 읽으면 지운다 (다음 실행은 새로 센다)
pub fn take_ready(instance_id: &str) -> bool {
    seen().lock().unwrap_or_else(|e| e.into_inner()).remove(instance_id)
}

fn next_token() -> u64 {
    static NEXT: AtomicU64 = AtomicU64::new(1);
    NEXT.fetch_add(1, Ordering::Relaxed)
}

fn emit(instance_id: &str, instance: &str, code: Option<&str>) {
    let mut data = json!({ "instanceId": instance_id, "instance": instance, "ok": code.is_none() });
    if let Some(code) = code {
        data["code"] = json!(code);
    }
    crate::events::emit("instance.ready", data);
}

/// 그 시작이 아직 기다리는 중이면 닫고 알린다 (`token`이 없으면 지금 기다리는 시작)
fn finish(instance_id: &str, instance: &str, token: Option<u64>, code: Option<&str>) -> bool {
    let mut map = pending().lock().unwrap_or_else(|e| e.into_inner());
    match (map.get(instance_id), token) {
        (Some(t), Some(want)) if *t != want => return false,
        (None, _) => return false,
        _ => {}
    }
    map.remove(instance_id);
    drop(map);
    emit(instance_id, instance, code);
    true
}

/// 시작 결과를 받는다. `console`은 데몬이 콘솔을 읽는 프로세스(managed)이고, `patterns`가 비었거나 없으면 바로 준비다
pub fn after_start(instance_id: &str, instance: &str, result: &anyhow::Result<Value>, console: Option<Arc<ManagedProcess>>, patterns: &[String], timeout: Duration) {
    let body = match result {
        Ok(body) => body,
        Err(_) => return emit(instance_id, instance, Some("start_failed")),
    };
    // 이미 켜져 있었거나 다른 시작이 진행 중이면 이번 요청은 새 시작이 아니다
    if body.get("already_running").and_then(Value::as_bool) == Some(true) {
        return;
    }
    if body.get("success").and_then(Value::as_bool) != Some(true) {
        match body.get("error_code").and_then(Value::as_str) {
            Some("start_in_progress") => {}
            Some("start_cancelled") => emit(instance_id, instance, Some("stopped_while_starting")),
            _ => emit(instance_id, instance, Some("start_failed")),
        }
        return;
    }
    let regexes: Vec<regex::Regex> = patterns.iter().filter_map(|p| regex::Regex::new(p).ok()).collect();
    if regexes.is_empty() {
        return emit(instance_id, instance, None);
    }
    // 데몬이 띄운 프로세스(`managed:true`)인데 콘솔이 없으면 그 사이 끝나 치워진 것이다
    let Some(proc) = console else {
        let managed = body.get("managed").and_then(Value::as_bool) == Some(true);
        return emit(instance_id, instance, managed.then_some("start_failed"));
    };
    let token = next_token();
    // 판정을 거치지 않고 끝난 지난 실행의 표시는 이번 실행의 것이 아니다
    seen().lock().unwrap_or_else(|e| e.into_inner()).remove(instance_id);
    pending().lock().unwrap_or_else(|e| e.into_inner()).insert(instance_id.to_string(), token);
    let (id, name) = (instance_id.to_string(), instance.to_string());
    tokio::spawn(async move { watch(id, name, token, proc, regexes, timeout).await });
}

/// 기다림이 끝나는 때. 너무 커서 시계로 나타낼 수 없으면 끝이 없다
fn deadline_after(timeout: Duration) -> Option<tokio::time::Instant> {
    tokio::time::Instant::now().checked_add(timeout)
}

/// 준비 문구·종료·시간 초과 중 먼저 오는 것으로 닫는다
async fn watch(id: String, name: String, token: u64, proc: Arc<ManagedProcess>, patterns: Vec<regex::Regex>, timeout: Duration) {
    let deadline = deadline_after(timeout);
    let mut next_line = 0;
    loop {
        if pending().lock().unwrap_or_else(|e| e.into_inner()).get(&id).is_none_or(|t| *t != token) {
            return;
        }
        for line in proc.get_console_since(next_line).await {
            next_line = line.id + 1;
            if line.source != LogSource::System && patterns.iter().any(|p| p.is_match(&line.content)) {
                seen().lock().unwrap_or_else(|e| e.into_inner()).insert(id.clone());
                finish(&id, &name, Some(token), None);
                return;
            }
        }
        if !proc.is_running() {
            // 종료 판정이 사용자 정지인지 가려 먼저 닫는다. 판정이 없는 경로(이미 치워진 프로세스)면 시작 실패다
            tokio::time::sleep(EXIT_GRACE).await;
            finish(&id, &name, Some(token), Some("start_failed"));
            return;
        }
        if deadline.is_some_and(|d| tokio::time::Instant::now() >= d) {
            finish(&id, &name, Some(token), Some("ready_timeout"));
            return;
        }
        tokio::time::sleep(POLL).await;
    }
}

/// 그 인스턴스의 시작이 아직 준비 문구를 기다리는가
pub fn is_waiting(instance_id: &str) -> bool {
    pending().lock().unwrap_or_else(|e| e.into_inner()).contains_key(instance_id)
}

/// managed 프로세스의 종료를 판정했을 때: 준비 전이었으면 닫는다
pub fn exited(instance_id: &str, instance: &str, stop_requested: bool) {
    finish(instance_id, instance, None, Some(if stop_requested { "stopped_while_starting" } else { "start_failed" }));
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_huge_ready_timeout_means_no_deadline_instead_of_a_panic() {
        assert!(deadline_after(Duration::from_secs(u64::MAX)).is_none());
        assert!(deadline_after(Duration::from_secs(600)).is_some());
    }

    /// managed 시작이 성공했는데 그 사이 프로세스가 끝나 치워졌으면 준비가 아니다
    #[tokio::test]
    async fn a_managed_start_whose_process_is_already_gone_is_not_ready() {
        let mut sub = crate::events::bus().subscribe(None);
        let ok = Ok(json!({ "success": true, "managed": true }));
        after_start("gone-1", "gone", &ok, None, &["^Done".to_string()], Duration::from_secs(5));
        loop {
            let e = sub.rx.recv().await.unwrap();
            if e.kind == "instance.ready" && e.data["instanceId"] == "gone-1" {
                assert_eq!((e.data["ok"].clone(), e.data["code"].clone()), (json!(false), json!("start_failed")));
                break;
            }
        }
    }
}
