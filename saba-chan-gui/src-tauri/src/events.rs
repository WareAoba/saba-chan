//! 데몬 이벤트 스트림(GET /api/events, SSE)과 업데이트 알림 확인
//!
//! 연결 하나를 유지하고 받은 이벤트를 `events:message`({id, type, data})로 넘긴다.
//! 끊기면 마지막 번호(Last-Event-ID)로 3초 뒤 다시 붙는다. 놓친 건이 있으면 데몬이 `resync`를 보낸다.

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

/// 멈출 때까지 스트림을 유지한다
pub async fn run_event_stream<F: Fn(SseEvent)>(client: Arc<DaemonClient>, stop: Arc<AtomicBool>, emit: F) {
    let mut last_id: Option<u64> = None;
    // 다룰 데몬(G1)이 바뀌면 끊고 바로 새 대상에 붙는다. 번호는 데몬마다 따로라 처음부터 받는다
    let mut target = client.watch_target();
    while !stop.load(Ordering::SeqCst) {
        let mut switched = false;
        let mut req = client
            .builder(reqwest::Method::GET, "/api/events")
            .header("Accept", "text/event-stream");
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
                        emit(ev);
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
