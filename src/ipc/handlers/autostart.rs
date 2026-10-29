//! 로그인할 때 자동 실행 API (`saba_chan_updater_lib::autostart`)
//!
//! - `GET /api/system/autostart` — `{ok, supported, mode: "off"|"daemon"|"full", service, lingering}` (OS에 등록된 항목을 읽는다).
//!   `service`: 데몬을 바로 등록했다(GUI 없는 리눅스의 systemd 사용자 서비스, 맥의 LaunchAgent). `lingering`: 로그인하지 않아도 부팅할 때 돈다(리눅스가 아니면 null)
//! - `PUT /api/system/autostart` `{mode}` — 이 설치 폴더의 실행 파일로 등록하거나 지운다.
//!   실패는 HTTP 200 `{ok: false, error_code: "autostart_<코드>", error}`

use axum::{extract::State, response::IntoResponse, Json};
use saba_chan_updater_lib::autostart::{self, AutostartError, Mode};
use serde::Deserialize;
use serde_json::json;
use std::path::PathBuf;

use super::super::IPCServer;

fn status() -> serde_json::Value {
    let service = autostart::read().is_some_and(|e| e.is_service() && e.program.is_file());
    json!({
        "ok": true,
        "supported": autostart::supported(),
        "mode": autostart::current_mode().as_str(),
        "service": service,
        "lingering": autostart::lingering(),
    })
}

fn refused(e: AutostartError) -> axum::response::Response {
    Json(json!({ "ok": false, "error_code": format!("autostart_{}", e.code), "error": e.detail })).into_response()
}

/// GET /api/system/autostart
pub async fn get_autostart(State(_state): State<IPCServer>) -> impl IntoResponse {
    match tokio::task::spawn_blocking(status).await {
        Ok(v) => Json(v).into_response(),
        Err(e) => refused(AutostartError { code: "read_failed", detail: e.to_string() }),
    }
}

#[derive(Deserialize)]
pub struct AutostartRequest {
    mode: String,
}

/// PUT /api/system/autostart
pub async fn put_autostart(State(_state): State<IPCServer>, Json(req): Json<AutostartRequest>) -> impl IntoResponse {
    match set_mode(&req.mode).await {
        Ok(v) => Json(v).into_response(),
        Err(e) => refused(e),
    }
}

/// 등록하고 바뀐 상태를 돌려준다 (레지스트리·파일을 읽고 쓰므로 블로킹 스레드에서)
async fn set_mode(mode: &str) -> Result<serde_json::Value, AutostartError> {
    let Some(mode) = Mode::parse(mode) else {
        return Err(AutostartError { code: "unknown_mode", detail: mode.to_string() });
    };
    let install_dir = std::env::current_exe()
        .ok()
        .and_then(|exe| exe.parent().map(PathBuf::from))
        .ok_or_else(|| AutostartError { code: "write_failed", detail: "install folder unknown".into() })?;
    let now = tokio::task::spawn_blocking(move || autostart::apply(mode, &install_dir).map(|_| status()))
        .await
        .map_err(|e| AutostartError { code: "write_failed", detail: e.to_string() })??;
    tracing::info!("[Autostart] Set to {}", mode.as_str());
    Ok(now)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn an_unknown_mode_is_refused_before_touching_the_os() {
        let e = set_mode("gui").await.unwrap_err();
        assert_eq!(e.code, "unknown_mode");
        assert_eq!(e.detail, "gui");
    }
}
