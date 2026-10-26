//! 데몬 이벤트 스트림(GET /api/events, SSE)과 업데이트 알림 확인
//!
//! 연결 하나를 유지하고 받은 이벤트를 `events:message`({id, type, data})로 넘긴다.
//! 끊기면 마지막 번호(Last-Event-ID)로 3초 뒤 다시 붙는다. 놓친 건이 있으면 데몬이 `resync`를 보낸다.
//! 원격 데몬을 고른 동안에는 이 PC 데몬에도 따로 붙어 중대 알림을 놓치지 않는다 (`Feed`).

use crate::daemon::DaemonClient;
use futures_util::StreamExt;
use serde::Serialize;
use serde_json::{json, Value};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::time::Duration;

pub const EVENTS_RETRY: Duration = Duration::from_secs(3);
/// 확인 주기는 데몬이 정한다 (U6). GUI는 데몬이 캐시한 결과를 가끔 읽는다
pub const UPDATE_STATUS_POLL: Duration = Duration::from_secs(10 * 60);

#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct SseEvent {
    pub id: Option<u64>,
    #[serde(rename = "type")]
    pub kind: String,
    pub data: Value,
}

/// SSE 한 덩어리 → 이벤트. 주석(ping)뿐이거나 data가 JSON이 아니면 None
pub fn parse_sse_block(block: &str) -> Option<SseEvent> {
    let mut id = None;
    let mut kind = "message".to_string();
    let mut data: Vec<&str> = Vec::new();
    for line in block.split('\n') {
        let line = line.strip_suffix('\r').unwrap_or(line);
        if line.is_empty() || line.starts_with(':') {
            continue;
        }
        let (field, value) = match line.find(':') {
            Some(at) => (&line[..at], line[at + 1..].strip_prefix(' ').unwrap_or(&line[at + 1..])),
            None => (line, ""),
        };
        match field {
            "id" => id = value.trim().parse().ok(),
            "event" => kind = value.to_string(),
            "data" => data.push(value),
            _ => {}
        }
    }
    if data.is_empty() {
        return None;
    }
    let data = serde_json::from_str(&data.join("\n")).ok()?;
    Some(SseEvent { id, kind, data })
}

/// 받은 바이트를 모아 빈 줄(\n\n)마다 이벤트로 자른다 (청크 경계가 아무 데나 와도 된다)
#[derive(Default)]
pub struct SseBuffer {
    buf: Vec<u8>,
}

impl SseBuffer {
    pub fn push(&mut self, chunk: &[u8]) -> Vec<SseEvent> {
        self.buf.extend_from_slice(chunk);
        let mut out = Vec::new();
        while let Some(end) = self.buf.windows(2).position(|w| w == b"\n\n") {
            let block: Vec<u8> = self.buf.drain(..end + 2).collect();
            if let Some(ev) = parse_sse_block(&String::from_utf8_lossy(&block[..end])) {
                out.push(ev);
            }
        }
        out
    }
}

/// 이벤트 스트림이 붙는 데몬
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Feed {
    /// 고른 데몬 (화면·중대 알림·효과음)
    Chosen,
    /// 원격 데몬을 고른 동안의 이 PC 데몬 (중대 알림·크래시 소리만). 이 PC를 고르면 `Chosen`이 이 PC다
    LocalWhileRemote,
}

impl Feed {
    /// 지금 붙을 데몬(안의 None = 이 PC). 바깥 None이면 붙지 않고 쉰다
    pub fn source(self, chosen: Option<String>) -> Option<Option<String>> {
        match (self, chosen) {
            (Feed::Chosen, chosen) => Some(chosen),
            (Feed::LocalWhileRemote, Some(_)) => Some(None),
            (Feed::LocalWhileRemote, None) => None,
        }
    }

    /// 이 스트림의 이벤트로 낼 소리. 이 PC 감시는 크래시(예기치 않은 종료)만
    pub fn plays(self, cue: crate::sounds::Cue) -> bool {
        self == Feed::Chosen || cue == crate::sounds::Cue::Crashed
    }

    /// 앱 화면이 이 스트림의 데몬을 보이는가 (창이 앞에 있으면 앱 안 알림으로 충분한지)
    pub fn shown_in_app(self) -> bool {
        self == Feed::Chosen
    }
}

/// 이벤트 스트림 요청. 붙을 때 정한 데몬(None = 이 PC)에 묻는다
pub fn events_request(client: &DaemonClient, source: Option<&str>) -> reqwest::RequestBuilder {
    client
        .builder_for(reqwest::Method::GET, "/api/events", Some(source.unwrap_or("local")))
        .header("Accept", "text/event-stream")
}

/// 멈출 때까지 스트림을 유지한다. `emit`은 이벤트와 그 이벤트를 낸 데몬(None = 이 PC)을 받는다
pub async fn run_event_stream<F: Fn(SseEvent, Option<&str>)>(
    client: Arc<DaemonClient>,
    stop: Arc<AtomicBool>,
    feed: Feed,
    emit: F,
) {
    let mut last_id: Option<u64> = None;
    // 다룰 데몬(G1)이 바뀌면 끊고 바로 새 대상에 붙는다. 번호는 데몬마다 따로라 처음부터 받는다
    let mut target = client.watch_target();
    while !stop.load(Ordering::SeqCst) {
        let mut switched = false;
        let Some(source) = feed.source(client.target()) else {
            // 이 PC를 골랐다: 고른 스트림이 이 PC의 것을 받는다
            tokio::select! {
                _ = tokio::time::sleep(EVENTS_RETRY) => {}
                _ = target.changed() => {}
            }
            last_id = None;
            continue;
        };
        let mut req = events_request(&client, source.as_deref());
        if let Some(id) = last_id {
            req = req.header("Last-Event-ID", id.to_string());
        }
        // 머리가 늦는 원격 데몬에 붙는 중에도 대상을 바꾸면 바로 놓는다
        let sent = tokio::select! {
            r = req.send() => Some(r),
            _ = target.changed() => None,
        };
        let Some(sent) = sent else {
            last_id = None;
            continue;
        };
        match sent {
            Ok(res) if res.status().as_u16() == 200 => {
                let mut stream = res.bytes_stream();
                let mut buf = SseBuffer::default();
                loop {
                    let chunk = tokio::select! {
                        c = stream.next() => c,
                        _ = target.changed() => {
                            switched = true;
                            break;
                        }
                    };
                    let Some(chunk) = chunk else { break };
                    if stop.load(Ordering::SeqCst) {
                        return;
                    }
                    let Ok(chunk) = chunk else { break };
                    for ev in buf.push(&chunk) {
                        if ev.id.is_some() {
                            last_id = ev.id;
                        }
                        emit(ev, source.as_deref());
                    }
                }
            }
            Ok(res) => {
                // 토큰이 바뀌었을 수 있다 (데몬 재시작)
                if res.status().as_u16() == 401 {
                    client.load_token();
                }
            }
            Err(_) => {}
        }
        if !switched {
            tokio::select! {
                _ = tokio::time::sleep(EVENTS_RETRY) => {}
                _ = target.changed() => switched = true,
            }
        }
        if switched {
            last_id = None;
        }
    }
}

/// 꼭 알아야 할 데몬 이벤트 → OS 알림 본문 (`notify.*` 문구). 크래시·원인 모를 종료·재시작 실패·
/// 사바쨩 밖의 시작·자동 백업 실패만. 이름만 싣고 경로·오류 원문은 싣지 않는다
pub fn critical_notice(ev: &SseEvent, i18n: &crate::i18n::I18n) -> Option<String> {
    let d = &ev.data;
    let name = d["instance"].as_str().filter(|n| !n.is_empty()).or_else(|| d["instanceId"].as_str())?;
    let text = |key: &str| i18n.t(key, &[("name", name)]);
    match ev.kind.as_str() {
        "instance.exit" => {
            let head = match d["kind"].as_str()? {
                "crash" => text("notify.crash"),
                "unknown_exit" => text("notify.unknown_exit"),
                _ => return None,
            };
            let tail = match (d["restartInSecs"].as_u64(), d["noteCode"].as_str()) {
                (Some(secs), _) => Some(i18n.t("notify.restart_in", &[("secs", &secs.to_string())])),
                (None, Some("gave_up")) => Some(i18n.t("notify.gave_up", &[])),
                _ => None,
            };
            Some(tail.map_or(head.clone(), |t| format!("{head} {t}")))
        }
        "instance.restart_failed" => Some(text("notify.restart_failed")),
        "instance.started" if d["unexpected"].as_bool() == Some(true) => Some(text("notify.external_start")),
        "backup.done" if d["ok"].as_bool() == Some(false) && d["kind"] != "manual" => Some(text("notify.backup_failed")),
        _ => None,
    }
}

/// 원격 데몬의 중대 알림 본문에 그 데몬 이름을 붙인다 (이 PC 서버로 읽히지 않게)
pub fn on_daemon(body: String, daemon: Option<&str>, i18n: &crate::i18n::I18n) -> String {
    match daemon {
        Some(daemon) => i18n.t("notify.on_daemon", &[("daemon", daemon), ("body", &body)]),
        None => body,
    }
}

/// `GET /api/remote/daemons` 결과에서 원격 데몬 이름. 없으면 hostId
pub fn daemon_name(list: &Value, host_id: &str) -> String {
    list["daemons"]
        .as_array()
        .and_then(|a| a.iter().find(|d| d["hostId"] == host_id))
        .and_then(|d| d["name"].as_str())
        .filter(|n| !n.is_empty())
        .unwrap_or(host_id)
        .to_string()
}

/// 알릴 업데이트 (Locales는 백그라운드 자동 적용이라 뺀다)
#[derive(Debug, PartialEq)]
pub struct UpdateNotice {
    pub names: Vec<String>,
    /// 같은 목록을 두 번 알리지 않기 위한 지문
    pub key: String,
    pub payload: Value,
}

/// GET /api/updates/status 결과 → 알릴 것 (main.js checkForUpdates)
pub fn update_notice(data: &Value) -> Option<UpdateNotice> {
    let ok = data["ok"].as_bool().unwrap_or(false);
    let has_error = data.get("error").is_some_and(|e| !e.is_null() && e != "" && e != false);
    if !ok || has_error {
        if has_error {
            tracing::warn!("[UpdateChecker] Daemon check failed: {}", data["error"]);
        }
        return None;
    }
    if data["updates_available"].as_u64().unwrap_or(0) == 0 {
        return None;
    }
    let comps: Vec<&Value> = data["components"]
        .as_array()
        .map(|a| a.iter().collect())
        .unwrap_or_default();
    let names: Vec<String> = comps
        .iter()
        .filter(|c| c["update_available"].as_bool().unwrap_or(false) && c["display_name"] != "Locales")
        .filter_map(|c| c["display_name"].as_str().map(String::from))
        .collect();
    if names.is_empty() {
        tracing::info!("[UpdateChecker] Only Locales updates — skipping notification (auto-applied)");
        return None;
    }
    let mut sorted = names.clone();
    sorted.sort();
    let components: Vec<Value> = comps
        .into_iter()
        .filter(|c| c["display_name"] != "Locales")
        .cloned()
        .collect();
    let payload = json!({ "count": names.len(), "names": names, "components": components });
    Some(UpdateNotice {
        key: sorted.join("\0"),
        names,
        payload,
    })
}

/// 데몬이 확인에 성공했고 알릴 업데이트가 없다 (적용을 마쳤거나 Locales만 남았다).
/// 데몬이 막 떠 아직 확인하지 않았거나(`last_check` 없음) 확인하는 중이면 목록이 비어 있어도 "없음"이 아니다
pub fn updates_cleared(data: &Value) -> bool {
    let ok = data["ok"].as_bool().unwrap_or(false)
        && data["last_check"].is_string()
        && !data["checking"].as_bool().unwrap_or(false);
    let has_error = data.get("error").is_some_and(|e| !e.is_null() && e != "" && e != false);
    ok && !has_error && update_notice(data).is_none()
}

#[cfg(test)]
mod tests {
    use super::*;

    /// 꼭 알아야 할 일만 OS 알림으로: 크래시·원인 모를 종료·재시작 실패·사바쨩 밖의 시작·자동 백업 실패.
    /// 사바쨩을 거친 시작·정지, 게임 안 정지, 시작 실패(시킨 쪽이 이미 본다)는 알리지 않는다
    #[test]
    fn only_critical_events_become_os_notifications() {
        let i18n = crate::i18n::I18n::load("en", None);
        let ev = |kind: &str, data: Value| SseEvent { id: Some(1), kind: kind.into(), data };
        let body = |kind: &str, data: Value| critical_notice(&ev(kind, data), &i18n);
        assert_eq!(
            body("instance.exit", json!({"instance": "Survival", "kind": "crash", "restartInSecs": 5})).as_deref(),
            Some("Survival crashed and stopped. Restarting in 5s.")
        );
        assert_eq!(
            body("instance.exit", json!({"instance": "Survival", "kind": "unknown_exit", "noteCode": "gave_up"})).as_deref(),
            Some("Survival stopped unexpectedly. Auto restart gave up. Please check the server.")
        );
        assert_eq!(
            body("instance.restart_failed", json!({"instance": "Survival", "noteArgs": {"reason": "C:\\x"}})).as_deref(),
            Some("Could not restart Survival.")
        );
        assert_eq!(
            body("instance.started", json!({"instance": "Survival", "source": "external", "unexpected": true})).as_deref(),
            Some("Survival was started outside Saba-chan. Saba-chan did not start this process.")
        );
        assert_eq!(
            body("backup.done", json!({"instance": "Survival", "kind": "scheduled", "ok": false, "error": "D:\\x"})).as_deref(),
            Some("Automatic backup of Survival failed.")
        );
        for (kind, data) in [
            ("instance.exit", json!({"instance": "Survival", "kind": "user_stop"})),
            ("instance.exit", json!({"instance": "Survival", "kind": "in_game_stop"})),
            ("instance.exit", json!({"instance": "Survival", "kind": "start_failed"})),
            ("instance.started", json!({"instance": "Survival", "source": "discord", "unexpected": false})),
            ("instance.stopped", json!({"instance": "Survival", "source": "gui", "kind": "user_stop"})),
            ("backup.done", json!({"instance": "Survival", "kind": "manual", "ok": false})),
            ("backup.done", json!({"instance": "Survival", "kind": "scheduled", "ok": true})),
        ] {
            assert_eq!(body(kind, data.clone()), None, "{kind} {data}");
        }
        // 이름이 없으면 id
        assert_eq!(body("instance.exit", json!({"instanceId": "i-1", "kind": "crash"})).as_deref(), Some("i-1 crashed and stopped."));
    }

    /// 원격 데몬의 중대 알림은 그 데몬 이름을 붙인다 (이 PC 서버로 읽히지 않게). 이 PC의 것은 그대로
    #[test]
    fn a_remote_daemons_notice_names_the_daemon() {
        let i18n = crate::i18n::I18n::load("en", None);
        let body = "Survival crashed and stopped.".to_string();
        assert_eq!(on_daemon(body.clone(), None, &i18n), body);
        assert_eq!(
            on_daemon(body, Some("Living room PC"), &i18n),
            "On Living room PC: Survival crashed and stopped."
        );
        let list = json!({"daemons": [{"hostId": "host-b", "name": "Living room PC"}, {"hostId": "host-c"}]});
        assert_eq!(daemon_name(&list, "host-b"), "Living room PC");
        assert_eq!(daemon_name(&list, "host-c"), "host-c");
        assert_eq!(daemon_name(&json!({}), "host-d"), "host-d");
    }

    /// 원격 데몬을 고른 동안에도 이 PC 데몬의 중대 알림을 받는다: 이 PC 감시 스트림은 원격을 고른 동안만 붙고
    /// 늘 이 PC에 묻는다. 이 PC를 고르면 고른 스트림이 이 PC라 쉰다 (같은 알림을 두 번 띄우지 않게)
    #[test]
    fn the_local_watch_runs_only_while_a_remote_daemon_is_chosen() {
        assert_eq!(Feed::Chosen.source(None), Some(None));
        assert_eq!(Feed::Chosen.source(Some("host-b".into())), Some(Some("host-b".into())));
        assert_eq!(Feed::LocalWhileRemote.source(None), None);
        assert_eq!(Feed::LocalWhileRemote.source(Some("host-b".into())), Some(None));

        let dir = tempfile::tempdir().unwrap();
        let client = DaemonClient::new("http://127.0.0.1:57474".into(), dir.path().join(".ipc_token"));
        client.set_target(Some("host-b".into()));
        let target = |source: Option<&str>| {
            let req = events_request(&client, source).build().unwrap();
            req.headers().get("X-Saba-Target").map(|v| v.to_str().unwrap().to_string())
        };
        assert_eq!(target(None).as_deref(), Some("local"));
        // 붙을 때 정한 데몬에 묻는다 (그사이 고른 데몬이 바뀌어도)
        client.set_target(Some("host-c".into()));
        assert_eq!(target(Some("host-b")).as_deref(), Some("host-b"));
    }

    /// 이 PC 감시 스트림은 소리 중 크래시(예기치 않은 종료)만 낸다. 고른 스트림은 지금 규칙 그대로
    #[test]
    fn the_local_watch_plays_only_the_crash_sound() {
        use crate::sounds::Cue;
        for cue in [Cue::Started, Cue::Stopped, Cue::InstallSuccess, Cue::InstallFailed, Cue::Crashed] {
            assert!(Feed::Chosen.plays(cue), "{cue:?}");
            assert_eq!(Feed::LocalWhileRemote.plays(cue), cue == Cue::Crashed, "{cue:?}");
        }
        // 앱 화면은 고른 데몬을 보이므로, 이 PC 감시 스트림의 알림은 창이 앞에 있어도 띄운다
        assert!(Feed::Chosen.shown_in_app());
        assert!(!Feed::LocalWhileRemote.shown_in_app());
    }

    #[test]
    fn comment_or_ping_is_ignored() {
        assert_eq!(parse_sse_block(": ping"), None);
        assert_eq!(parse_sse_block(":\n: keep-alive"), None);
        assert_eq!(parse_sse_block("id: 4\nevent: x"), None);
        // JSON이 아니면 버린다
        assert_eq!(parse_sse_block("data: not json"), None);
    }

    #[test]
    fn parses_id_event_and_multiline_data() {
        let ev = parse_sse_block("id: 12\nevent: instance.status\ndata: {\"a\":\ndata: 1}").unwrap();
        assert_eq!(ev.id, Some(12));
        assert_eq!(ev.kind, "instance.status");
        assert_eq!(ev.data, json!({"a": 1}));

        let ev = parse_sse_block(": c\ndata:{\"b\":2}").unwrap();
        assert_eq!(ev.id, None);
        assert_eq!(ev.kind, "message");
        assert_eq!(ev.data, json!({"b": 2}));
        assert_eq!(
            serde_json::to_value(&ev).unwrap(),
            json!({"id": null, "type": "message", "data": {"b": 2}})
        );
    }

    #[test]
    fn buffer_splits_events_across_chunks() {
        let mut b = SseBuffer::default();
        assert!(b.push(b"id: 1\ndata: {\"x\"").is_empty());
        let evs = b.push(b":1}\n\n: ping\n\nid: 2\nevent: resync\ndata: {}\n");
        assert_eq!(evs.len(), 1);
        assert_eq!(evs[0].id, Some(1));
        let evs = b.push(b"\n");
        assert_eq!(evs[0].kind, "resync");
    }

    #[test]
    fn update_notice_skips_locales_only() {
        let only_locales = json!({"ok": true, "updates_available": 1,
            "components": [{"display_name": "Locales", "update_available": true}]});
        assert_eq!(update_notice(&only_locales), None);
        let mixed = json!({"ok": true, "updates_available": 2, "components": [
            {"display_name": "Locales", "update_available": true},
            {"display_name": "GUI", "update_available": true},
            {"display_name": "CLI", "update_available": false}]});
        let n = update_notice(&mixed).unwrap();
        assert_eq!(n.names, vec!["GUI"]);
        assert_eq!(n.payload["count"], 1);
        assert_eq!(n.payload["components"].as_array().unwrap().len(), 2);
    }

    /// 받을 업데이트가 없어지면 알림함의 업데이트 알림을 지우도록 알린다. 확인 실패는 "없음"이 아니다
    #[test]
    fn updates_cleared_only_when_the_check_succeeded_with_nothing_to_show() {
        let at = "2026-10-20T07:00:00Z";
        assert!(updates_cleared(
            &json!({"ok": true, "last_check": at, "updates_available": 0, "components": []})
        ));
        assert!(updates_cleared(
            &json!({"ok": true, "last_check": at, "updates_available": 1,
            "components": [{"display_name": "Locales", "update_available": true}]})
        ));
        assert!(!updates_cleared(
            &json!({"ok": true, "last_check": at, "updates_available": 1,
            "components": [{"display_name": "GUI", "update_available": true}]})
        ));
        assert!(!updates_cleared(
            &json!({"ok": true, "last_check": at, "updates_available": 0, "error": "relay down"})
        ));
        assert!(!updates_cleared(&json!({"ok": false})));
        // 데몬이 막 떠서 아직 확인한 적이 없으면(last_check 없음) "없음"이 아니다
        assert!(!updates_cleared(
            &json!({"ok": true, "last_check": null, "updates_available": 0, "components": []})
        ));
        assert!(!updates_cleared(
            &json!({"ok": true, "last_check": "2026-10-20T07:00:00Z",
            "checking": true, "updates_available": 0, "components": []})
        ));
    }
}
