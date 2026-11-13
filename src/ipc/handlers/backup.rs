//! 백업 API (A2). 예약 작업 API (A3)는 `schedule.rs`
//!
//! - `GET/POST /api/instance/:id/backups` — 목록 / 지금 백업
//! - `POST /api/instance/:id/backups/:name/restore` — 복원 (서버가 꺼져 있어야 한다. 먼저 pre-restore 백업)
//! - `DELETE /api/instance/:id/backups/:name`
//! - `POST /api/instance/:id/backups/:name/export` — `{path}`: 백업 하나를 zip 하나로 쓴다
//! - `POST /api/instance/:id/backups/import` — `{path, preview}`: 내보낸 zip을 검사하고 목록에 올린다

use super::{Failure, LooseJson};
use axum::{
    extract::{Path, State},
    http::StatusCode,
    response::{IntoResponse, Response},
    Json,
};
use serde::Deserialize;
use serde_json::json;

use super::super::IPCServer;
use crate::service::backup as service;

pub(super) fn error(status: StatusCode, code: &str, message: impl Into<String>) -> Response {
    Failure::coded(status, code, message).into_response()
}

/// 다른 작업이 잡고 있을 때의 응답 (시작·업데이트·삭제)
pub(crate) fn instance_busy() -> Response {
    Failure::busy().into_response()
}

pub async fn list_backups(Path(id): Path<String>, State(state): State<IPCServer>) -> Response {
    super::answer(service::list(&state, &id).await)
}

/// POST 본문 (없으면 지금 상태). `source`: `current` | `snapshot`, `snapshot`: 게임 스냅샷 id (없으면 가장 새것)
#[derive(Deserialize, Default)]
pub struct CreateBody {
    #[serde(default)]
    source: Option<String>,
    #[serde(default)]
    snapshot: Option<String>,
    /// 원격 액션 `backup.create`의 칸: true면 `source: snapshot`과 같다
    #[serde(default)]
    from_snapshot: bool,
}

pub async fn create_backup(Path(id): Path<String>, State(state): State<IPCServer>, LooseJson(body): LooseJson<CreateBody>) -> Response {
    let body = body.map(|Json(b)| b).unwrap_or_default();
    let source = match body.source.as_deref() {
        None if body.from_snapshot => service::Source::Snapshot(body.snapshot),
        None | Some("current") => service::Source::Current,
        Some("snapshot") => service::Source::Snapshot(body.snapshot),
        Some(other) => return error(StatusCode::BAD_REQUEST, "invalid_request", format!("Unknown backup source: {}", other)),
    };
    let resp = match service::create_from(&state, &id, source).await {
        Ok(made) => Json(json!({ "success": true, "backup": made })).into_response(),
        Err(r) => r.into_response(),
    };
    super::audited(&state, "backup", &id, None, None, resp).await
}

pub async fn delete_backup(Path((id, name)): Path<(String, String)>, State(state): State<IPCServer>) -> Response {
    super::answer(service::delete(&state, &id, name).await)
}

#[derive(Deserialize)]
pub struct TransferBody {
    /// 데몬 PC의 zip 경로 (절대 경로)
    pub(crate) path: String,
    /// 가져오기: true면 검사만 하고 무엇인지 돌려준다
    #[serde(default)]
    pub(crate) preview: bool,
}

/// 백업 하나를 내용까지 담은 zip 하나로 쓴다 (다른 PC로 옮기거나 따로 보관). 게임 자동 백업은 먼저 보관한 뒤 내보낸다
pub async fn export_backup(Path((id, name)): Path<(String, String)>, State(state): State<IPCServer>, Json(body): Json<TransferBody>) -> Response {
    super::answer(service::export(&state, &id, name, &body.path).await)
}

/// 내보낸 zip을 검사하고(`preview`) 이 서버의 백업 목록에 올린다. 사바쨩이 내보낸 백업이 아니거나, 내용이 목록과 다르거나,
/// 다른 모듈의 서버에서 만든 백업이면 올리지 않는다. 서버에 넣는 것은 복원이 한다
pub async fn import_backup(Path(id): Path<String>, State(state): State<IPCServer>, Json(body): Json<TransferBody>) -> Response {
    super::answer(service::import(&state, &id, &body.path, body.preview).await)
}

/// 복원은 작업 기록에 `restore` 한 줄로 남는다 (어디서 시켰든: 앱·웹·CLI·디스코드)
pub async fn restore_backup(Path((id, name)): Path<(String, String)>, State(state): State<IPCServer>) -> Response {
    let resp = super::answer(service::restore(&state, &id, name).await);
    super::audited(&state, "restore", &id, None, None, resp).await
}
