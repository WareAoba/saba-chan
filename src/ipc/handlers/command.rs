use axum::{
    extract::{Path, State},
    response::IntoResponse,
    Json,
};
use serde_json::Value;

use super::super::{CommandRequest, IPCServer};

/// POST /api/instance/:id/command - 게임 명령 실행 (모든 인터페이스의 단일 경로)
///
/// 요청 해석·검증·라우팅은 `crate::commands`가 한다. 응답은 실제로 실행한 경로
/// (RCON·REST·stdin·모듈)의 응답 본문에 `command`를 더한 것이고, 실패하면
/// `error_code`와 모듈 `[errors]`의 사용자용 문구 `error_message`를 더한다.
pub async fn execute_command(
    Path(id): Path<String>,
    State(state): State<IPCServer>,
    Json(req): Json<CommandRequest>,
) -> axum::response::Response {
    crate::service::command::execute(&state, &id, req).await
}

/// GET /api/instance/:id/commands — 이 인스턴스에서 쓸 수 있는 명령 전체 (모듈 + 인스턴스별 + 활성 익스텐션)
///
/// `{commands: [{name, label, description, risk, inputs, source: "module"|"extension", ready, group?, extension?, aliases?}],
///   components?}` — 인스턴스별 명령(`[commands] hook`)은 source "module"에 `group`(예: 플러그인 이름)이 붙고,
/// `components`는 모듈이 판정한 구성요소(설치된 플러그인·모드 등)를 그대로 넘긴 것이다.
pub async fn list_instance_commands(Path(id): Path<String>, State(state): State<IPCServer>) -> axum::response::Response {
    crate::service::command::list(&state, &id).await
}

/// POST /api/instance/:id/rcon 라우트: 콘솔 명령을 감사에 남긴다 (안에서 부르는 `execute_rcon_command`는 부른 쪽이 남긴다)
pub async fn rcon_route(Path(id): Path<String>, State(state): State<IPCServer>, Json(payload): Json<Value>) -> axum::response::Response {
    let command = payload.get("command").and_then(Value::as_str).map(str::to_string);
    let resp = execute_rcon_command(Path(id.clone()), State(state.clone()), Json(payload)).await.into_response();
    super::audited(&state, "command", &id, command.as_deref(), None, resp).await
}

/// POST /api/instance/:id/rest 라우트: REST 명령을 감사에 남긴다
pub async fn rest_route(Path(id): Path<String>, State(state): State<IPCServer>, Json(payload): Json<Value>) -> axum::response::Response {
    let endpoint = payload.get("endpoint").and_then(Value::as_str).map(str::to_string);
    let body = payload.get("body").cloned();
    let resp = execute_rest_command(Path(id.clone()), State(state.clone()), Json(payload)).await.into_response();
    super::audited(&state, "command", &id, endpoint.as_deref(), body.as_ref(), resp).await
}

/// POST /api/instance/:id/rcon - RCON 명령어 실행
pub async fn execute_rcon_command(
    Path(id): Path<String>,
    State(state): State<IPCServer>,
    Json(payload): Json<serde_json::Value>,
) -> impl IntoResponse {
    crate::service::command::rcon_for(&state.supervisor, &id, &payload).await
}

/// POST /api/instance/:id/rest - REST API 명령어 실행
pub async fn execute_rest_command(
    Path(id): Path<String>,
    State(state): State<IPCServer>,
    Json(payload): Json<serde_json::Value>,
) -> impl IntoResponse {
    crate::service::command::rest_for(&state.supervisor, &state.credentials, &id, payload, None).await
}
