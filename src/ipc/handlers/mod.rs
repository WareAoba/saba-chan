pub mod server;
pub mod instance;
pub mod command;
pub mod managed;
pub mod long_poll;
pub mod client;
pub mod bot;
pub mod config;
pub mod extension;
pub mod relay;
pub mod relay_cache;
pub mod privacy;
pub mod port_check;
pub mod remote_daemon;
pub mod chat;
pub mod backup;
pub mod schedule;
pub mod server_update;
pub mod logs;
pub mod chatbot;
pub mod discord;
pub mod words;
pub mod start_notice;
pub mod events;
pub mod migration;
pub mod diagnostics;
pub mod guides;
pub mod system;
pub mod uninstall;
pub mod paths;
pub mod autostart;
pub mod maker;
pub mod permissions;
pub mod sandbox;
pub mod actions;
pub mod locales;

/// 본문이 없거나 JSON으로 읽히지 않으면 None인 본문. axum 0.7의 `Option<Json<T>>`와 같다
/// (0.8의 `Option<Json<T>>`는 Content-Type이 있는데 본문이 비었거나 틀리면 400으로 거절한다)
pub struct LooseJson<T>(pub Option<axum::Json<T>>);

impl<T, S> axum::extract::FromRequest<S> for LooseJson<T>
where
    T: serde::de::DeserializeOwned,
    S: Send + Sync,
{
    type Rejection = std::convert::Infallible;

    async fn from_request(req: axum::extract::Request, state: &S) -> Result<Self, Self::Rejection> {
        Ok(Self(axum::Json::<T>::from_request(req, state).await.ok()))
    }
}

/// 감사 로그 (A10): 응답은 그대로 돌려주고, 출처(`crate::audit::current`)와 함께 한 건 남긴다.
/// `key`는 인스턴스 id나 이름. 성공은 2xx이면서 본문의 `success`가 false가 아닐 때
pub(crate) async fn audited(
    state: &super::IPCServer,
    action: &str,
    key: &str,
    command: Option<&str>,
    args: Option<&serde_json::Value>,
    resp: axum::response::Response,
) -> axum::response::Response {
    if crate::audit::is_quiet() {
        return resp;
    }
    let (parts, body) = resp.into_parts();
    let bytes = axum::body::to_bytes(body, usize::MAX).await.unwrap_or_default();
    let parsed = serde_json::from_slice::<serde_json::Value>(&bytes).ok();
    record(state, action, key, command, args, parts.status, parsed.as_ref()).await;
    axum::response::Response::from_parts(parts, axum::body::Body::from(bytes))
}

pub(crate) use crate::service::{record, Failure, Outcome};

/// 서비스 함수의 결과를 HTTP 응답으로 (성공은 200)
pub(crate) fn answer(result: Outcome) -> axum::response::Response {
    use axum::response::IntoResponse;
    match result {
        Ok(body) => axum::Json(body).into_response(),
        Err(failure) => failure.into_response(),
    }
}

impl From<Outcome> for crate::ipc::internal::InternalResponse {
    fn from(result: Outcome) -> Self {
        match result {
            Ok(body) => Self { status: axum::http::StatusCode::OK, body },
            Err(failure) => Self { status: failure.status, body: failure.body },
        }
    }
}
