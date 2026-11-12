//! 디스코드 탭이 읽는 릴레이 자료(노드·멤버·채널·서버 관리·대화 모델·도구)를 데몬이 잠깐 기억한다.
//! 탭을 다시 열 때마다 릴레이(멤버는 디스코드까지)를 다시 부르지 않는다.
//! 디스코드·릴레이·설정 쪽에 쓰는 요청(GET이 아닌 것)이 끝나면 모두 잊는다: 앱이 바꾼 것은 바로 다시 읽힌다

use axum::{
    body::Body,
    extract::Request,
    http::{Method, StatusCode},
    middleware::Next,
    response::{IntoResponse, Response},
};
use std::collections::HashMap;
use std::sync::{LazyLock, Mutex};
use std::time::{Duration, Instant};

/// 기억하는 시간. 디스코드에서 바뀐 것(새 멤버, 디스코드에서 한 음소거)은 이만큼 늦게 보일 수 있다
pub const FRESH_FOR: Duration = Duration::from_secs(60);
/// 답 하나의 크기 상한 (넘으면 기억하지 않고 그대로 준다)
const MAX_BODY: usize = 4 * 1024 * 1024;

/// 기억한 답: 받은 때, 헤더, 본문
type Remembered = (Instant, axum::http::HeaderMap, axum::body::Bytes);

static READS: LazyLock<Mutex<HashMap<String, Remembered>>> = LazyLock::new(|| Mutex::new(HashMap::new()));

/// 기억해 둘 릴레이 읽기인가 (연결 확인 `/status`·크레딧은 늘 새로 읽는다)
fn cacheable(path: &str) -> bool {
    if let Some(rest) = path.strip_prefix("/api/relay/host/") {
        return rest.ends_with("/nodes");
    }
    let Some(rest) = path.strip_prefix("/api/relay/node/") else { return false };
    ["/members", "/channels", "/moderation", "/chatbot/models", "/chatbot/tools"]
        .iter()
        .any(|end| rest.ends_with(end) && rest.len() > end.len())
}

/// 쓰면 기억한 릴레이 읽기를 잊는 경로. 클라이언트 heartbeat·서버 조작 같은 쓰기는 디스코드 자료를 바꾸지 않는다
fn invalidates(path: &str) -> bool {
    ["/api/relay/", "/api/chatbot", "/api/chat/", "/api/config", "/api/guilds", "/api/actions", "/api/discord", "/api/bot"]
        .iter()
        .any(|p| path.starts_with(p))
}

/// 다 잊는다
pub fn forget_all() {
    READS.lock().unwrap_or_else(|e| e.into_inner()).clear();
}

/// 라우터 층: 릴레이 읽기는 기억한 답을 주고, 디스코드 자료를 바꿀 수 있는 쓰기가 끝나면 다 잊는다
pub async fn layer(req: Request, next: Next) -> Response {
    let writes = req.method() != Method::GET && req.method() != Method::HEAD && invalidates(req.uri().path());
    let key = (req.method() == Method::GET && cacheable(req.uri().path())).then(|| req.uri().to_string());
    if let Some(key) = &key {
        let hit = READS.lock().unwrap_or_else(|e| e.into_inner()).get(key).and_then(|(at, headers, body)| {
            (at.elapsed() < FRESH_FOR).then(|| (headers.clone(), body.clone()))
        });
        if let Some((headers, body)) = hit {
            let mut resp = Response::new(Body::from(body));
            *resp.headers_mut() = headers;
            return resp;
        }
    }
    let resp = next.run(req).await;
    if writes {
        forget_all();
        return resp;
    }
    let Some(key) = key else { return resp };
    if resp.status() != StatusCode::OK {
        return resp;
    }
    let (parts, body) = resp.into_parts();
    match axum::body::to_bytes(body, MAX_BODY).await {
        Ok(bytes) => {
            let mut reads = READS.lock().unwrap_or_else(|e| e.into_inner());
            reads.retain(|_, (at, _, _)| at.elapsed() < FRESH_FOR);
            reads.insert(key, (Instant::now(), parts.headers.clone(), bytes.clone()));
            Response::from_parts(parts, Body::from(bytes))
        }
        Err(_) => (StatusCode::BAD_GATEWAY, "relay response too large").into_response(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use axum::{routing::get, Router};
    use std::sync::atomic::{AtomicUsize, Ordering};
    use std::sync::Arc;
    use tower::ServiceExt;

    #[test]
    fn only_discord_tab_reads_are_remembered() {
        assert!(cacheable("/api/relay/host/h1/nodes"));
        assert!(cacheable("/api/relay/node/123/members"));
        assert!(cacheable("/api/relay/node/123/moderation"));
        assert!(cacheable("/api/relay/node/123/chatbot/models"));
        assert!(!cacheable("/api/relay/host/h1/status"), "연결 확인은 늘 새로");
        assert!(!cacheable("/api/relay/node/123/chatbot/credit"), "사용량은 늘 새로");
        assert!(!cacheable("/api/relay/node//members"));
        assert!(!cacheable("/api/servers"));
    }

    #[test]
    fn heartbeats_and_server_actions_do_not_forget() {
        assert!(invalidates("/api/relay/node/1/channels"));
        assert!(invalidates("/api/chatbot/settings"));
        assert!(invalidates("/api/config/bot"));
        assert!(!invalidates("/api/client/heartbeat"));
        assert!(!invalidates("/api/instance/a/stdin"));
    }

    #[tokio::test]
    async fn a_second_read_is_answered_from_memory_until_something_is_written() {
        forget_all();
        let hits = Arc::new(AtomicUsize::new(0));
        let h = hits.clone();
        let app = Router::new()
            .route(
                "/api/relay/node/{g}/channels",
                get(move || {
                    let h = h.clone();
                    async move { axum::Json(serde_json::json!({ "n": h.fetch_add(1, Ordering::SeqCst) })) }
                })
                .put(|| async { StatusCode::OK }),
            )
            .layer(axum::middleware::from_fn(layer));
        let call = |method: Method| {
            let app = app.clone();
            async move {
                let req = Request::builder().method(method).uri("/api/relay/node/g9/channels?relay_url=x").body(Body::empty()).unwrap();
                let resp = app.oneshot(req).await.unwrap();
                let body = axum::body::to_bytes(resp.into_body(), usize::MAX).await.unwrap();
                String::from_utf8(body.to_vec()).unwrap()
            }
        };
        assert_eq!(call(Method::GET).await, r#"{"n":0}"#);
        assert_eq!(call(Method::GET).await, r#"{"n":0}"#, "두 번째는 기억한 답");
        assert_eq!(hits.load(Ordering::SeqCst), 1);
        call(Method::PUT).await;
        assert_eq!(call(Method::GET).await, r#"{"n":1}"#, "쓴 뒤에는 다시 읽는다");
    }
}
