//! 서비스 계층: 시작·정지·명령·백업 같은 기능의 본체.
//! HTTP 핸들러(`ipc::handlers`)·채팅·예약·릴레이 에이전트·플러그인 호스트가 같은 함수를 부른다.
//! 핸들러는 요청을 풀어 여기를 부르고 결과를 응답으로 바꾸기만 한다

pub(crate) mod backup;
pub(crate) mod chatbot;
pub(crate) mod command;
pub(crate) mod discord;
pub mod lifecycle;
pub(crate) mod onboarding;
pub mod server_update;
pub(crate) mod servers;
pub(crate) mod settings;
pub(crate) mod shared_folder;

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

/// `audited`와 같되 서비스 함수의 결과를 남기고 그대로 돌려준다 (성공은 200)
pub(crate) async fn audited_result(state: &crate::ipc::IPCServer, action: &str, key: &str, result: Outcome) -> Outcome {
    if !crate::audit::is_quiet() {
        let (status, body) = match &result {
            Ok(body) => (axum::http::StatusCode::OK, body),
            Err(failure) => (failure.status, &failure.body),
        };
        record(state, action, key, None, None, status, Some(body)).await;
    }
    result
}

pub(crate) async fn record(
    state: &crate::ipc::IPCServer,
    action: &str,
    key: &str,
    command: Option<&str>,
    args: Option<&serde_json::Value>,
    status: axum::http::StatusCode,
    body: Option<&serde_json::Value>,
) {
    let (id, name, command, args) = {
        let sup = state.supervisor.read().await;
        let instance = sup.instance_store.find(key);
        let module = instance.and_then(|i| sup.module_loader.get_module(&i.module_name).ok()).map(|m| m.metadata);
        // 비밀 인자·설정값은 가려서 남긴다
        let (command, args) = crate::audit::redact(module.as_ref(), instance, command, args);
        match instance {
            Some(i) => (i.id.clone(), Some(i.name.clone()), command, args),
            None => (key.to_string(), None, command, args),
        }
    };
    let declined = body.and_then(|v| v.get("success").and_then(serde_json::Value::as_bool)) == Some(false);
    let ok = status.is_success() && !declined;
    let code = if ok { None } else { body.and_then(crate::audit::failure_code) };
    crate::audit::record(crate::audit::Event {
        action,
        instance_id: &id,
        instance: name.as_deref(),
        command: command.as_deref(),
        args: args.as_ref(),
        daemon: None,
        ok,
        status: status.as_u16(),
        code: code.as_deref(),
    });
}

/// 감사 로그 (A10): 응답은 그대로 돌려주고, 출처(`crate::audit::current`)와 함께 한 건 남긴다.
/// `key`는 인스턴스 id나 이름. 성공은 2xx이면서 본문의 `success`가 false가 아닐 때
pub(crate) async fn audited(
    state: &crate::ipc::IPCServer,
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
