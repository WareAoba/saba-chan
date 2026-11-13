//! 데몬 콘솔 스트림(SSE, `…/console/stream`)을 그 창에 넘긴다.
//! `daemon_stream_open {id, path, target?}` → 그 창에 `daemon:stream` `{id, event, data}`를 건마다 보내고, 끝나면 `{id, event:"end"}`.
//! `daemon_stream_close {id}`로 멈춘다. 창 하나가 여러 콘솔을 따로 연다 (id는 렌더러가 정한다)

use crate::daemon::DaemonClient;
use crate::events::SseBuffer;
use crate::AppState;
use futures_util::StreamExt;
use serde_json::{json, Value};
use std::collections::HashMap;
use std::sync::Mutex;
use tauri::{AppHandle, Emitter, Manager, WebviewWindow};

/// 열려 있는 스트림: id → 읽는 작업
#[derive(Default)]
pub struct Streams(Mutex<HashMap<String, tauri::async_runtime::JoinHandle<()>>>);

impl Streams {
    fn stop(&self, id: &str) {
        if let Some(task) = self.0.lock().unwrap_or_else(|e| e.into_inner()).remove(id) {
            task.abort();
        }
    }
}

/// 렌더러가 열 수 있는 스트림 경로 (콘솔 둘뿐)
pub fn is_stream_path(path: &str) -> bool {
    let route = path.split('?').next().unwrap_or("");
    if route == "/api/daemon/console/stream" {
        return true;
    }
    let Some(id) = route.strip_prefix("/api/instance/").and_then(|r| r.strip_suffix("/console/stream")) else {
        return false;
    };
    !id.is_empty() && !id.contains('/')
}

/// 연 스트림의 답: 열었으면 `{status:200}`, 데몬이 거절했으면 `{status, data}`(옛 데몬은 404), 연결 실패 `{status:0, code}`
#[tauri::command(rename_all = "camelCase")]
pub async fn daemon_stream_open(app: AppHandle, window: WebviewWindow, id: String, path: String, target: Option<String>) -> Value {
    if !is_stream_path(&path) || id.is_empty() || id.len() > 64 {
        return json!({ "status": 0, "error": format!("Invalid daemon stream: {path}"), "code": "EINVAL" });
    }
    let client = app.state::<AppState>().client.clone();
    let target = DaemonClient::normalize_target(target);
    let req = client.builder_for(reqwest::Method::GET, &path, target.as_deref()).header("Accept", "text/event-stream");
    let res = match req.send().await {
        Ok(res) => res,
        Err(e) => return json!({ "status": 0, "error": e.to_string(), "code": "ECONNREFUSED" }),
    };
    let status = res.status().as_u16();
    if !res.status().is_success() {
        let data: Value = res.json().await.unwrap_or(Value::Null);
        return json!({ "status": status, "data": data });
    }
    let streams = app.state::<Streams>();
    streams.stop(&id);
    let (label, sid, handle) = (window.label().to_string(), id.clone(), app.clone());
    let task = tauri::async_runtime::spawn(async move {
        let mut body = res.bytes_stream();
        let mut buf = SseBuffer::default();
        while let Some(Ok(chunk)) = body.next().await {
            // 창을 닫으면 렌더러가 닫지 못한다: 받는 창이 없으면 멈춘다 (keep-alive가 15초마다 깨운다)
            if handle.get_webview_window(&label).is_none() {
                break;
            }
            for ev in buf.push(&chunk) {
                let _ = handle.emit_to(label.as_str(), "daemon:stream", json!({ "id": sid, "event": ev.kind, "data": ev.data }));
            }
        }
        let _ = handle.emit_to(label.as_str(), "daemon:stream", json!({ "id": sid, "event": "end" }));
        handle.state::<Streams>().0.lock().unwrap_or_else(|e| e.into_inner()).remove(&sid);
    });
    streams.0.lock().unwrap_or_else(|e| e.into_inner()).insert(id, task);
    json!({ "status": 200 })
}

#[tauri::command(rename_all = "camelCase")]
pub fn daemon_stream_close(app: AppHandle, id: String) {
    app.state::<Streams>().stop(&id);
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn only_console_streams_can_be_opened() {
        assert!(is_stream_path("/api/daemon/console/stream?spans=1"));
        assert!(is_stream_path("/api/instance/mc-1/console/stream"));
        assert!(!is_stream_path("/api/instance//console/stream"));
        assert!(!is_stream_path("/api/instance/a/b/console/stream"));
        assert!(!is_stream_path("/api/events"));
        assert!(!is_stream_path("/api/config/node-token"));
    }
}
