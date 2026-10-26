//! 개인정보 모두 지우기 (사용자 결정 2026-10-26)
//!
//!   POST /api/relay/privacy/erase   `{locale?}` → `{url, expiresAt}`  릴레이에서 삭제표를 받아 웹사이트 주소를 준다
//!   GET  /api/relay/privacy/erase   → `{status}`  none | pending | done | expired | failed
//!
//! 사용자가 웹사이트에서 동의하면 릴레이가 그 디스코드 계정의 정보를 지운다. 데몬은 삭제표를 받은 뒤 끝났는지 묻다가,
//! 끝났으면 연결 해제(DELETE /api/config/node-token)와 같이 에이전트를 멈추고 노드 토큰을 지운다.
//! 지운 뒤에는 토큰이 없으니 상태는 삭제표와 함께 받은 비밀(pollSecret)로만 묻는다.

use axum::{extract::State, http::StatusCode, response::IntoResponse, Json};
use serde_json::{json, Value};
use std::sync::Mutex;
use std::time::Duration;
use tokio_util::sync::CancellationToken;

use super::super::IPCServer;
use super::relay::{apply_signed_headers, client, load_node_token, send, signed_relay_url};

/// 삭제표는 30분 산다. 릴레이가 그 뒤에는 expired 로 답한다. 릴레이에 닿지 않을 때만 이 시간에 그만둔다
const GIVE_UP: Duration = Duration::from_secs(35 * 60);
const POLL: Duration = if cfg!(test) { Duration::from_millis(20) } else { Duration::from_secs(3) };

/// 지금 삭제표의 상태. 새로 받으면 앞의 것을 기다리던 일은 멈춘다
struct Slot {
    status: &'static str,
    cancel: Option<CancellationToken>,
}

static SLOT: Mutex<Slot> = Mutex::new(Slot { status: "none", cancel: None });

fn slot() -> std::sync::MutexGuard<'static, Slot> {
    SLOT.lock().unwrap_or_else(|e| e.into_inner())
}

/// 이 세대(cancel)의 일이 아직 지금 일일 때만 상태를 바꾼다
fn set_status(cancel: &CancellationToken, status: &'static str) {
    let mut s = slot();
    if !cancel.is_cancelled() {
        s.status = status;
    }
}

/// 릴레이에서 받은 삭제표
#[derive(Debug, PartialEq)]
struct Ticket {
    id: String,
    url: String,
    poll_secret: String,
    expires_at: String,
}

fn bad_gateway(message: &str) -> axum::response::Response {
    (StatusCode::BAD_GATEWAY, Json(json!({ "error": message }))).into_response()
}

/// POST `<relay>/api/privacy/erasures` (노드 토큰 서명)
async fn issue(base: &str, locale: Option<&str>) -> Result<Ticket, axum::response::Response> {
    let client = client()?;
    let path = "/api/privacy/erasures";
    let text = match locale {
        Some(l) => json!({ "locale": l }),
        None => json!({}),
    }
    .to_string();
    let req = apply_signed_headers(client.post(format!("{}{}", base, path)), "POST", path, Some(&text))
        .header("Content-Type", "application/json")
        .body(text);
    match send(req).await? {
        (code, Some(data)) if code.is_success() => {
            let field = |k: &str| data.get(k).and_then(Value::as_str).unwrap_or("").to_string();
            let ticket = Ticket { id: field("id"), url: field("url"), poll_secret: field("pollSecret"), expires_at: field("expiresAt") };
            // 웹사이트 주소만 연다 (릴레이가 다른 곳을 주면 받지 않는다)
            if ticket.id.is_empty() || ticket.poll_secret.is_empty() || !is_site_url(&ticket.url) {
                return Err(bad_gateway("Invalid relay response"));
            }
            Ok(ticket)
        }
        (code, Some(data)) => Err((code, Json(data)).into_response()),
        (_, None) => Err(bad_gateway("Invalid relay response")),
    }
}

/// 동의 화면은 공식 웹사이트(또는 모의 릴레이가 준 루프백 주소)에만 있다
fn is_site_url(url: &str) -> bool {
    url.starts_with("https://saba-chan.online/") || (cfg!(test) && url.starts_with("http://127.0.0.1"))
}

/// 릴레이가 답한 상태. 닿지 않으면 None (다시 묻는다)
async fn status_once(base: &str, ticket: &Ticket) -> Option<&'static str> {
    let client = client().ok()?;
    let req = client
        .post(format!("{}/api/privacy/erasures/{}/status", base, super::relay::encode_path_segment(&ticket.id)))
        .json(&json!({ "pollSecret": ticket.poll_secret }));
    match send(req).await.ok()? {
        (code, Some(data)) if code.is_success() => match data.get("status").and_then(Value::as_str) {
            Some("done") => Some("done"),
            Some("expired") => Some("expired"),
            Some("pending") => Some("pending"),
            _ => None,
        },
        // 삭제표가 없어졌다 (기한이 한참 지나 릴레이가 치웠다)
        (StatusCode::NOT_FOUND, _) => Some("expired"),
        _ => None,
    }
}

/// 끝나거나(done) 기한이 지날(expired) 때까지 묻는다. 멈추면 None
async fn wait_done(base: &str, ticket: &Ticket, cancel: &CancellationToken, give_up: Duration) -> Option<&'static str> {
    let started = std::time::Instant::now();
    loop {
        tokio::select! {
            _ = cancel.cancelled() => return None,
            _ = tokio::time::sleep(POLL) => {}
        }
        match status_once(base, ticket).await {
            Some(s @ ("done" | "expired")) => return Some(s),
            _ if started.elapsed() >= give_up => return Some("expired"),
            _ => {}
        }
    }
}

#[derive(serde::Deserialize, Default)]
pub struct EraseRequest {
    #[serde(default)]
    locale: Option<String>,
}

/// POST /api/relay/privacy/erase
pub async fn start_erase(State(state): State<IPCServer>, body: super::LooseJson<EraseRequest>) -> axum::response::Response {
    if load_node_token().is_none() {
        return (StatusCode::CONFLICT, Json(json!({ "error": "not paired", "error_code": "node_token_missing" }))).into_response();
    }
    let locale = body.0.and_then(|Json(b)| b.locale).filter(|l| !l.is_empty() && l.len() <= 16);
    let configured = state.config_store.get_bot_config().await.cloud.relay_url;
    let base = signed_relay_url(&configured, None);
    let ticket = match issue(&base, locale.as_deref()).await {
        Ok(t) => t,
        Err(r) => return r,
    };
    let cancel = CancellationToken::new();
    {
        let mut s = slot();
        if let Some(old) = s.cancel.replace(cancel.clone()) {
            old.cancel();
        }
        s.status = "pending";
    }
    let reply = json!({ "url": ticket.url, "expiresAt": ticket.expires_at });
    tokio::spawn(async move {
        let Some(outcome) = wait_done(&base, &ticket, &cancel, GIVE_UP).await else { return };
        if outcome == "done" {
            // 릴레이에서는 이미 지워졌다. 이 PC 의 연결도 끊는다 (연결 해제와 같다)
            if let Err(e) = super::relay::disconnect(&state).await {
                tracing::warn!("[Privacy] erased on the relay but could not remove the node token: {}", e);
                set_status(&cancel, "failed");
                return;
            }
            tracing::info!("[Privacy] account data erased on the relay; relay connection removed");
        }
        set_status(&cancel, outcome);
    });
    (StatusCode::OK, Json(reply)).into_response()
}

/// GET /api/relay/privacy/erase
pub async fn erase_status() -> impl IntoResponse {
    Json(json!({ "status": slot().status }))
}

#[cfg(test)]
mod tests {
    use super::*;
    use axum::{extract::Path, routing::post, Router};
    use std::sync::atomic::{AtomicUsize, Ordering};
    use std::sync::Arc;

    /// 릴레이 흉내: 삭제표를 주고, 상태는 `pending` 을 두 번 답한 뒤 `done`
    async fn mock_relay(url: &'static str) -> String {
        let asks = Arc::new(AtomicUsize::new(0));
        let app = Router::new()
            .route(
                "/api/privacy/erasures",
                post(move |Json(b): Json<Value>| async move {
                    (StatusCode::CREATED, Json(json!({ "id": "e1", "url": format!("{url}#e1.s"), "pollSecret": "p", "expiresAt": "x", "locale": b["locale"] })))
                }),
            )
            .route(
                "/api/privacy/erasures/{id}/status",
                post(move |Path(id): Path<String>, Json(b): Json<Value>| {
                    let asks = asks.clone();
                    async move {
                        if id != "e1" || b["pollSecret"] != "p" {
                            return (StatusCode::NOT_FOUND, Json(json!({ "error": "NOT_FOUND" })));
                        }
                        let n = asks.fetch_add(1, Ordering::SeqCst);
                        (StatusCode::OK, Json(json!({ "status": if n < 2 { "pending" } else { "done" } })))
                    }
                }),
            );
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();
        tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });
        format!("http://{}", addr)
    }

    #[tokio::test]
    async fn a_ticket_opens_the_site_and_waits_until_the_relay_says_done() {
        let base = mock_relay("https://saba-chan.online/ko/legal/erase").await;
        let ticket = issue(&base, Some("ko")).await.unwrap();
        assert_eq!(ticket.url, "https://saba-chan.online/ko/legal/erase#e1.s");
        assert_eq!(ticket.poll_secret, "p");
        let cancel = CancellationToken::new();
        assert_eq!(wait_done(&base, &ticket, &cancel, Duration::from_secs(5)).await, Some("done"));
    }

    /// 릴레이가 웹사이트가 아닌 주소를 주면 열지 않는다
    #[tokio::test]
    async fn a_ticket_pointing_anywhere_but_the_site_is_refused() {
        let base = mock_relay("https://evil.example/legal/erase").await;
        assert_eq!(issue(&base, None).await.unwrap_err().status(), StatusCode::BAD_GATEWAY);
    }

    /// 모르는 삭제표(404)는 기한이 지난 것, 닿지 않는 릴레이는 정한 시간에 그만둔다, 멈추면 아무것도 하지 않는다
    #[tokio::test]
    async fn waiting_ends_on_a_lost_ticket_an_unreachable_relay_or_a_newer_ticket() {
        let base = mock_relay("https://saba-chan.online/legal/erase").await;
        let lost = Ticket { id: "other".into(), url: String::new(), poll_secret: "p".into(), expires_at: String::new() };
        let cancel = CancellationToken::new();
        assert_eq!(wait_done(&base, &lost, &cancel, Duration::from_secs(5)).await, Some("expired"));
        assert_eq!(wait_done("http://127.0.0.1:9", &lost, &cancel, Duration::from_millis(50)).await, Some("expired"));
        cancel.cancel();
        assert_eq!(wait_done(&base, &lost, &cancel, Duration::from_secs(5)).await, None);
    }
}
