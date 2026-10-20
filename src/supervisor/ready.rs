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
    let Some(proc) = console.filter(|_| !regexes.is_empty()) else {
        return emit(instance_id, instance, None);
    };
    let token = next_token();
    pending().lock().unwrap_or_else(|e| e.into_inner()).insert(instance_id.to_string(), token);
    let (id, name) = (instance_id.to_string(), instance.to_string());
    tokio::spawn(async move { watch(id, name, token, proc, regexes, timeout).await });
}

/// 준비 문구·종료·시간 초과 중 먼저 오는 것으로 닫는다
async fn watch(id: String, name: String, token: u64, proc: Arc<ManagedProcess>, patterns: Vec<regex::Regex>, timeout: Duration) {
    let deadline = tokio::time::Instant::now() + timeout;
    let mut next_line = 0;
    loop {
        if !pending().lock().unwrap_or_else(|e| e.into_inner()).get(&id).is_some_and(|t| *t == token) {
            return;
        }
        for line in proc.get_console_since(next_line).await {
            next_line = line.id + 1;
            if line.source != LogSource::System && patterns.iter().any(|p| p.is_match(&line.content)) {
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
        if tokio::time::Instant::now() >= deadline {
            finish(&id, &name, Some(token), Some("ready_timeout"));
            return;
        }
        tokio::time::sleep(POLL).await;
    }
}

/// managed 프로세스의 종료를 판정했을 때: 준비 전이었으면 닫는다
pub fn exited(instance_id: &str, instance: &str, stop_requested: bool) {
    finish(instance_id, instance, None, Some(if stop_requested { "stopped_while_starting" } else { "start_failed" }));
}
