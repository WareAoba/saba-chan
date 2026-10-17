//! 핸들러 내부 호출 어댑터
//!
//! 데몬 내부 기능(채팅 명령 처리기, 릴레이 에이전트 등)이 HTTP 왕복 없이
//! 기존 axum 핸들러를 그대로 재사용하기 위한 변환 계층입니다.
//! 핸들러가 내는 응답을 (상태 코드, JSON 본문)으로 풀어 돌려줍니다.

use axum::{http::StatusCode, response::IntoResponse};
use serde_json::Value;

/// 핸들러 응답 — 상태 코드와 JSON 본문
#[derive(Debug, Clone)]
pub struct InternalResponse {
    pub status: StatusCode,
    pub body: Value,
}

impl InternalResponse {
    pub fn is_success(&self) -> bool {
        self.status.is_success()
    }

    /// 본문의 문자열 필드 조회 (예: "error", "message")
    pub fn str_field(&self, key: &str) -> Option<&str> {
        self.body.get(key).and_then(|v| v.as_str())
    }
}

/// 핸들러 응답을 (상태, JSON)으로 변환합니다. JSON이 아니면 Null 본문을 돌려줍니다.
pub async fn into_internal(response: impl IntoResponse) -> InternalResponse {
    let response = response.into_response();
    let status = response.status();
    let body = match axum::body::to_bytes(response.into_body(), usize::MAX).await {
        Ok(bytes) if !bytes.is_empty() => serde_json::from_slice(&bytes).unwrap_or(Value::Null),
        _ => Value::Null,
    };
    InternalResponse { status, body }
}

#[cfg(test)]
mod tests {
    use super::*;
    use axum::Json;
    use serde_json::json;

    #[tokio::test]
    async fn converts_status_and_json_body() {
        let resp = into_internal((StatusCode::NOT_FOUND, Json(json!({"error": "nope"})))).await;
        assert_eq!(resp.status, StatusCode::NOT_FOUND);
        assert!(!resp.is_success());
        assert_eq!(resp.str_field("error"), Some("nope"));
    }

    #[tokio::test]
    async fn non_json_body_becomes_null() {
        let resp = into_internal((StatusCode::OK, "plain")).await;
        assert!(resp.is_success());
        assert!(resp.body.is_null());
    }
}
