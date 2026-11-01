//! 접속자 수 (Q-5). 모듈이 `[status] players`로 lifecycle 함수를 선언하면 데몬이 켜진 서버마다
//! 주기적으로 불러 마지막 값을 기억한다. 서버 목록(`ServerInfo.players`)과 relay 하트비트가 이 값을 읽는다.
//! 함수는 `{"success": true, "online": <정수>, "max": <정수>?}`를 돌려준다 (02-modules §2.3)
//!
//! 모듈이 접속·퇴장 문구를 주면 그 문구로 센 수(`player_events`)가 먼저다 (사용자 지정). 모듈 함수는 최대 인원과,
//! 콘솔을 처음부터 읽지 못한 서버에만 쓴다. 센 수는 데몬이 콘솔을 처음부터 읽은 프로세스에서만 세고, 프로세스가 끝나면 지운다

use serde::{Deserialize, Serialize};
use futures_util::StreamExt;
use serde_json::{json, Value};
use std::collections::HashMap;
use std::sync::{Mutex, OnceLock};

use super::Supervisor;

/// 다시 묻는 간격
pub const REFRESH_SECS: u64 = 60;
/// 함수 하나를 기다리는 시간. 넘기면 그 서버의 값은 비운다
pub const CALL_TIMEOUT_SECS: u64 = 10;
/// REST 포트에 붙어 보는 시간
const PROBE_TIMEOUT_SECS: u64 = 3;
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

/// 인스턴스의 마지막 접속자 수. 접속·퇴장 문구로 센 수가 먼저고(최대 인원은 모듈 답에서 빌린다),
/// 콘솔을 처음부터 읽지 못한 서버(데몬 재시작 뒤 다시 붙음 등)는 모듈이 알려 준 수. 둘 다 없으면 None
pub fn get(instance_id: &str) -> Option<PlayerCount> {
    let counted = rosters().lock().unwrap_or_else(|e| e.into_inner()).get(instance_id).map(|r| u32::try_from(r.names.len()).unwrap_or(u32::MAX));
    let reported = counts().lock().unwrap_or_else(|e| e.into_inner()).get(instance_id).copied();
    let Some(online) = counted else { return reported };
    let max = reported.and_then(|c| c.max).or_else(|| fallbacks().lock().unwrap_or_else(|e| e.into_inner()).get(instance_id).copied().flatten());
    Some(PlayerCount { online, max })
}

/// 접속·퇴장 줄로 센 접속자 수만 (모듈 함수가 알려 준 수는 보지 않는다). 세지 않는 서버면 None. 채팅 연동(`chat_link`)이 쓴다
pub fn counted(instance_id: &str) -> Option<u32> {
    rosters().lock().unwrap_or_else(|e| e.into_inner()).get(instance_id).map(|r| u32::try_from(r.names.len()).unwrap_or(u32::MAX))
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

/// 센 수에 붙일 최대 인원: 모듈에 접속자 함수가 없거나(None), 함수가 수 없이 답했다
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

/// 켜진 서버 중 모듈이 접속자 함수를 선언한 것만 묻고, 나머지는 기억을 지운다.
/// REST 서버(`[protocols] supported`에 rest)는 응답 없음을 REST 포트에 붙어 보기만 해서 정한다: 요청을 보내면
/// 서버 로그에 접근 줄이 남는다(팰월드 `REST accessed endpoint`). 주기 경로는 콘솔·RCON·REST 요청을 보내지 않는다(사용자 지정)
pub async fn refresh(sup: &tokio::sync::RwLock<Supervisor>) {
    let (mut calls, idle, probes): (Vec<_>, Vec<_>, Vec<_>) = {
        let s = sup.read().await;
        let mut calls = Vec::new();
        let mut idle = Vec::new();
        let mut probes = Vec::new();
        for instance in s.instance_store.list() {
            let module = s.module_loader.get_module(&instance.module_name).ok();
            let function = module.as_ref().and_then(|m| m.metadata.players_function.clone());
            let pid = s.tracker.get_pid(&instance.id).ok();
            let probe = module.as_ref().filter(|m| m.metadata.supports_protocol("rest")).zip(pid).map(|(m, pid)| {
                let host = instance.rest_host.clone().unwrap_or_else(|| m.metadata.default_rest_host());
                (instance.id.clone(), pid, host, instance.rest_port.unwrap_or_else(|| m.metadata.default_rest_port()))
            });
            let call = match (function, pid) {
                (Some(f), Some(pid)) => s.instance_call(&instance.id, &f).ok().map(|c| (pid, c)),
                _ => None,
            };
            match call {
                Some((pid, c)) => calls.push((instance.id.clone(), pid, c, probe.is_none())),
                None => idle.push((instance.id.clone(), probe.is_some())),
            }
            probes.extend(probe);
        }
        (calls, idle, probes)
    };
    for (id, probed) in idle {
        set(&id, None);
        if !probed {
            forget(&id);
        }
        fallbacks().lock().unwrap_or_else(|e| e.into_inner()).insert(id, None);
    }
    static START: Mutex<usize> = Mutex::new(0);
    calls.sort_by(|a, b| a.0.cmp(&b.0));
    rotate_calls(&mut calls, &mut START.lock().unwrap_or_else(|e| e.into_inner()));
    let mut asks = futures_util::stream::FuturesUnordered::new();
    for (id, pid, call, records) in calls {
        asks.push(Box::pin(async move {
            // 앞에서 돌린 순서로 자리를 시도한다. 기다리는 인스턴스는 이번 갱신을 건너뛴다
            let result = call.timeout(CALL_TIMEOUT_SECS).try_run().await;
            apply_refresh(&id, Some(pid), result, std::time::Instant::now(), records);
        }) as std::pin::Pin<Box<dyn std::future::Future<Output = ()> + Send>>);
    }
    for (id, pid, host, port) in probes {
        asks.push(Box::pin(async move {
            let result = port_open(&host, port).await;
            apply_probe(&id, Some(pid), result, std::time::Instant::now());
        }));
    }
    while asks.next().await.is_some() {}
}

/// 포트 확인 실패 이유의 머리. 화면에 낼 때 `reason_text`가 화면 언어 문구로 바꾼다
const PORT_CLOSED: &str = "rest_port_closed:";

/// 포트에 TCP로 붙었다가 아무것도 보내지 않고 바로 끊는다
pub async fn port_open(host: &str, port: u16) -> Result<(), String> {
    match tokio::time::timeout(std::time::Duration::from_secs(PROBE_TIMEOUT_SECS), tokio::net::TcpStream::connect((host, port))).await {
        Ok(Ok(_)) => Ok(()),
        Ok(Err(_)) | Err(_) => Err(format!("{PORT_CLOSED}{port}")),
    }
}

/// 응답 없음 이유를 화면에 낼 글로. 포트 확인 실패는 화면 언어 문구(bot.json `server.rest_port_closed`), 모듈이 준 이유는 그대로
pub fn reason_text(reason: &str, catalog: &crate::chat::i18n::Catalog) -> String {
    match reason.strip_prefix(PORT_CLOSED) {
        Some(port) => catalog.t("server.rest_port_closed", &[("port", port)]),
        None => reason.to_string(),
    }
}

fn apply_probe(id: &str, pid: Option<u32>, result: Result<(), String>, now: std::time::Instant) {
    let ok = json!({ "success": true });
    record(id, pid, result.map(|()| &ok), now);
}

fn rotate_calls<T>(calls: &mut [T], start: &mut usize) {
    if !calls.is_empty() {
        calls.rotate_left(*start % calls.len());
        *start = (*start + 1) % calls.len();
    }
}

/// `records`가 false면 응답 없음 기록은 포트 확인(`apply_probe`)이 맡는다
fn apply_refresh(id: &str, pid: Option<u32>, result: anyhow::Result<Option<Value>>, now: std::time::Instant, records: bool) {
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
    if records {
        record(id, pid, result.as_ref().map_err(|e| e.to_string()), now);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    // 콘솔 접속·퇴장 문구로 센 수가 먼저다 (사용자 지정). 모듈 함수는 최대 인원과, 콘솔을 처음부터 못 읽은 경우에만 쓴다
    #[test]
    fn join_and_leave_lines_count_players_before_the_module_count() {
        let id = "roster-count-test";
        assert_eq!(get(id), None);
        let generation = roster_start(id);
        assert_eq!(get(id), Some(PlayerCount { online: 0, max: None }), "counting starts at zero right away");
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
        apply_refresh(id, Some(1), Ok(Some(json!({ "success": true, "max": 8 }))), std::time::Instant::now(), true);
        assert_eq!(get(id), Some(PlayerCount { online: 1, max: Some(8) }));
        // 모듈이 수를 줘도 센 수가 먼저고, 최대 인원만 빌린다
        apply_refresh(id, Some(1), Ok(Some(json!({ "success": true, "online": 5, "max": 32 }))), std::time::Instant::now(), true);
        assert_eq!(get(id), Some(PlayerCount { online: 1, max: Some(32) }));
        // 모듈 함수가 실패해도 센 수는 그대로 보인다
        apply_refresh(id, Some(1), Err(anyhow::anyhow!("timeout")), std::time::Instant::now(), true);
        assert_eq!(get(id), Some(PlayerCount { online: 1, max: None }));

        roster_end(id, generation);
        assert_eq!(get(id), None, "a stopped server has no count");
        // 콘솔을 처음부터 못 읽은 서버(데몬 재시작 뒤 다시 붙음)는 모듈이 준 수를 쓴다
        apply_refresh(id, Some(1), Ok(Some(json!({ "success": true, "online": 5, "max": 32 }))), std::time::Instant::now(), true);
        assert_eq!(get(id), Some(PlayerCount { online: 5, max: Some(32) }));
        set(id, None);
        forget(id);
        fallbacks().lock().unwrap().remove(id);
    }

    #[test]
    fn a_restart_starts_from_zero_and_the_old_watch_cannot_touch_it() {
        let id = "roster-restart-test";
        let old = roster_start(id);
        roster_change(id, old, "Bob", true);
        let new = roster_start(id);
        assert_eq!(get(id).unwrap().online, 0);
        roster_change(id, old, "Alex", true);
        roster_end(id, old);
        assert_eq!(get(id), Some(PlayerCount { online: 0, max: None }));
        roster_end(id, new);
        assert_eq!(get(id), None);
    }

    // REST 서버는 포트에 붙어 보기만 한다 (요청을 보내면 서버 로그에 접근 줄이 1분마다 남는다, 사용자 지정)
    #[tokio::test]
    async fn a_rest_server_is_probed_by_connecting_only() {
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let port = listener.local_addr().unwrap().port();
        assert!(port_open("127.0.0.1", port).await.is_ok());
        drop(listener);
        assert!(port_open("127.0.0.1", port).await.is_err());

        // 함수가 답해도 포트가 닫혀 있으면 응답 없음이다. 함수 결과는 응답 없음을 정하지 않는다
        let id = "probe-test";
        let now = std::time::Instant::now();
        apply_refresh(id, Some(7), Ok(Some(json!({ "success": true, "max": 32 }))), now, false);
        let closed = port_open("127.0.0.1", port).await.unwrap_err();
        apply_probe(id, Some(7), Err(closed.clone()), now);
        apply_probe(id, Some(7), Err(closed), now + UNRESPONSIVE_AFTER);
        // 사용자에게 보이는 이유는 화면 언어로 (locales bot.json)
        let reason = unresponsive(id, Some(7), now + UNRESPONSIVE_AFTER).unwrap();
        let ko = reason_text(&reason, &crate::chat::i18n::Catalog::for_language("ko"));
        assert!(ko.contains(&port.to_string()) && ko.contains("REST"), "{ko}");
        assert!(!ko.contains("closed"), "{ko}");
        assert_eq!(reason_text("connection refused", &crate::chat::i18n::Catalog::for_language("ko")), "connection refused");
        apply_refresh(id, Some(7), Err(anyhow::anyhow!("x")), now + UNRESPONSIVE_AFTER, false);
        apply_probe(id, Some(7), Ok(()), now + UNRESPONSIVE_AFTER);
        assert!(unresponsive(id, Some(7), now + UNRESPONSIVE_AFTER).is_none(), "an open port clears it");
        set(id, None);
        forget(id);
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
        apply_refresh(id, Some(123), Ok(None), now + UNRESPONSIVE_AFTER, true);
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
