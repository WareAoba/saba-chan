//! 알림함 (`crate::notices`): GET /api/notices, POST /api/notices/read, DELETE /api/notices/{id}, DELETE /api/notices

use super::super::IPCServer;
use axum::{
    extract::{Path, Query, State},
    http::StatusCode,
    response::{IntoResponse, Response},
    Json,
};
use serde::Deserialize;
use serde_json::{json, Value};

#[derive(Debug, Deserialize, Default)]
pub struct ListQuery {
    locale: Option<String>,
}

#[allow(clippy::result_large_err)] // 오류는 바로 돌려줄 HTTP 응답
fn store() -> Result<&'static crate::notices::Store, Response> {
    crate::notices::store().ok_or_else(|| super::error(StatusCode::SERVICE_UNAVAILABLE, "notices_unavailable", "notice inbox is not open"))
}

fn body(store: &crate::notices::Store, cat: &crate::chat::i18n::Catalog) -> Value {
    let notices: Vec<Value> = store.list().iter().map(|n| crate::notices::render(cat, n)).collect();
    json!({ "success": true, "notices": notices, "unread": store.unread() })
}

/// GET /api/notices?locale= — 최신이 먼저. 문장은 요청한 언어(없으면 앱 언어)로
pub async fn list(State(state): State<IPCServer>, Query(q): Query<ListQuery>) -> Response {
    let store = match store() {
        Ok(s) => s,
        Err(r) => return r,
    };
    let cat = crate::chat::i18n::Catalog::for_language(&super::actions::locale(&state, q.locale).await);
    Json(body(store, &cat)).into_response()
}

/// POST /api/notices/read — 모두 읽음
pub async fn read_all() -> Response {
    match store() {
        Ok(s) => {
            s.mark_all_read();
            Json(json!({ "success": true, "unread": 0 })).into_response()
        }
        Err(r) => r,
    }
}

/// DELETE /api/notices/{id} — 한 건 지우기. 업데이트 결과 알림을 지우면 결과를 확인한 것으로 남긴다
pub async fn remove(Path(id): Path<u64>) -> Response {
    let store = match store() {
        Ok(s) => s,
        Err(r) => return r,
    };
    match store.remove(id) {
        Some(removed) => {
            crate::notices::acknowledge_removed(std::slice::from_ref(&removed));
            Json(json!({ "success": true, "unread": store.unread() })).into_response()
        }
        None => super::error(StatusCode::NOT_FOUND, "notice_not_found", format!("no notice {id}")),
    }
}

/// DELETE /api/notices — 모두 지우기
pub async fn clear() -> Response {
    let store = match store() {
        Ok(s) => s,
        Err(r) => return r,
    };
    let removed = store.clear();
    crate::notices::acknowledge_removed(&removed);
    Json(json!({ "success": true, "removed": removed.len() })).into_response()
}
