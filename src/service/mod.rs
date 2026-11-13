//! 서비스 계층: 시작·정지·명령·백업 같은 기능의 본체.
//! HTTP 핸들러(`ipc::handlers`)·채팅·예약·릴레이 에이전트·플러그인 호스트가 같은 함수를 부른다.
//! 핸들러는 요청을 풀어 여기를 부르고 결과를 응답으로 바꾸기만 한다

pub(crate) mod onboarding;

/// 서비스 함수(시작·정지·재시작·백업·복원)의 결과. 성공이면 응답 본문
pub(crate) type Outcome = Result<serde_json::Value, Failure>;

/// 서비스 함수가 거절하거나 실패한 결과. HTTP 핸들러는 담은 상태와 본문을 그대로 답한다
/// (모듈이 `success: false`로 답한 것은 200 그대로의 실패다)
#[derive(Debug)]
pub(crate) struct Failure {
    pub(crate) status: axum::http::StatusCode,
    pub(crate) body: serde_json::Value,
    /// 다른 작업이나 켜진 서버 때문에 거절했다: 그것이 끝나면 다시 할 수 있다
    in_use: Option<InUse>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum InUse {
    /// 백업·복원·업데이트가 작업 잠금을 쥐고 있다
    Busy,
    /// 서버가 켜져 있다
    Running,
}

impl Failure {
    pub(crate) fn new(status: axum::http::StatusCode, body: serde_json::Value) -> Self {
        Self { status, body, in_use: None }
    }

    /// `{success: false, error, error_code}`
    pub(crate) fn coded(status: axum::http::StatusCode, code: &str, message: impl Into<String>) -> Self {
        Self::new(status, serde_json::json!({ "success": false, "error": message.into(), "error_code": code }))
    }

    /// 다른 긴 작업이 작업 잠금을 쥐고 있다 (409 `instance_busy`)
    pub(crate) fn busy() -> Self {
        let failure = Self::coded(axum::http::StatusCode::CONFLICT, "instance_busy", "A backup, restore or update is running for this instance");
        Self { in_use: Some(InUse::Busy), ..failure }
    }

    /// 서버가 켜져 있어 파일을 바꾸지 않는다 (409 `server_running`, 본문은 부르는 쪽이 정한다)
    pub(crate) fn running(body: serde_json::Value) -> Self {
        Self { in_use: Some(InUse::Running), ..Self::new(axum::http::StatusCode::CONFLICT, body) }
    }

    pub(crate) fn is_busy(&self) -> bool {
        self.in_use == Some(InUse::Busy)
    }

    /// 작업 잠금이나 켜진 서버 때문에 거절했다 (`busy`·`running`)
    pub(crate) fn is_in_use(&self) -> bool {
        self.in_use.is_some()
    }

    /// 기록에 남길 이유 코드 (`error_code`)
    pub(crate) fn code(&self) -> Option<String> {
        crate::audit::failure_code(&self.body)
    }

    /// 사람이 읽는 까닭 (`error`)
    pub(crate) fn reason(&self) -> Option<&str> {
        self.body.get("error").and_then(serde_json::Value::as_str)
    }
}

impl axum::response::IntoResponse for Failure {
    fn into_response(self) -> axum::response::Response {
        (self.status, axum::Json(self.body)).into_response()
    }
}

/// 모듈·감독자가 답한 본문: `success: false`면 200 그대로의 실패
pub(crate) fn settled(body: serde_json::Value) -> Outcome {
    if body.get("success").and_then(serde_json::Value::as_bool) == Some(false) {
        Err(Failure::new(axum::http::StatusCode::OK, body))
    } else {
        Ok(body)
    }
}
