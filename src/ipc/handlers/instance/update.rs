//! 게임 서버 업데이트 확인·적용 API (본체는 `service::server_update`)

use super::*;
use crate::service::server_update;

/// GET /api/instance/:id/check-update - 게임 서버 업데이트 확인
///
/// 설치 방식 익스텐션(`install_method_<방식>`)으로 설치된 인스턴스에 대해
/// server.check_update hook을 디스패치하여 buildid 비교 결과를 반환합니다.
pub async fn check_server_update(
    Path(id): Path<String>,
    State(state): State<IPCServer>,
) -> impl IntoResponse {
    match server_update::apply::server_update_status(&state, &id).await {
        Some(status) => (StatusCode::OK, Json(status)).into_response(),
        None => super::super::error(StatusCode::NOT_FOUND, "instance_not_found", format!("Instance not found: {}", id)),
    }
}

/// `apply-update`의 본문 (`warn_minutes`: 켜진 서버면 게임 안에 예고할 시간, 기본 5분)
#[derive(Debug, Default, serde::Deserialize)]
pub struct ApplyBody {
    #[serde(default)]
    pub warn_minutes: Option<u32>,
}

/// POST /api/instance/:id/apply-update - 게임 서버 업데이트 적용
///
/// SteamCMD install (== update)을 재실행하여 서버를 최신 빌드로 업데이트합니다.
/// 서버가 켜져 있으면 본문 `warn_minutes`(기본 5)분 동안 게임 안에 예고한 뒤 정지 → 업데이트 → 다시 켜기를 뒤에서 하고
/// `{success, scheduled: true, warn_minutes, players}`로 바로 답합니다 (`server_update.rs`).
pub async fn apply_server_update(
    Path(id): Path<String>,
    State(state): State<IPCServer>,
    LooseJson(body): LooseJson<ApplyBody>,
) -> impl IntoResponse {
    let warn_minutes = body.and_then(|b| b.0.warn_minutes);
    super::super::answer(server_update::apply(&state, &id, warn_minutes).await)
}

/// `DELETE /api/instance/:id/apply-update`: 예고 중인 업데이트를 취소한다
pub async fn cancel_server_update(Path(id): Path<String>) -> axum::response::Response {
    super::super::answer(server_update::cancel(&id))
}
