//! 원격 데몬 설정과 짝 맺은 PC (D1, `crate::remote_daemon`). 로컬 전용: 원격 포트에서는 막는다
//!
//!   GET    /api/remote/settings            {enabled, port, address, transport, status}
//!   PUT    /api/remote/settings            {enabled?, port?, address?} → 저장하고 원격 포트를 다시 맞춘다
//!   GET    /api/remote/clients             {clients:[{hostId, name, pairedAt}]}
//!   DELETE /api/remote/clients/:hostId     짝을 푼다 (그 PC의 토큰이 더는 통하지 않는다)

use axum::{
    extract::{Path, State},
    http::StatusCode,
    response::{IntoResponse, Response},
    Json,
};
use saba_chan_updater_lib::config_file::ConfigFile;
use serde::Deserialize;
use serde_json::{json, Value};

use crate::ipc::IPCServer;
use crate::remote_daemon::{self, config};

fn bad(code: &str, message: &str) -> Response {
    (StatusCode::BAD_REQUEST, Json(json!({ "error": message, "error_code": code }))).into_response()
}

fn view(settings: &config::Settings, status: &remote_daemon::Status) -> Value {
    let mut v = serde_json::to_value(settings).unwrap_or_default();
    v["status"] = serde_json::to_value(status).unwrap_or_default();
    v
}

pub async fn get_settings() -> Json<Value> {
    let settings = config::settings_in(&ConfigFile::default_location());
    Json(view(&settings, &remote_daemon::status().await))
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SettingsPatch {
    enabled: Option<bool>,
    port: Option<u16>,
    address: Option<String>,
}

pub async fn put_settings(State(state): State<IPCServer>, Json(patch): Json<SettingsPatch>) -> Response {
    let file = ConfigFile::default_location();
    let mut settings = config::settings_in(&file);
    if let Some(port) = patch.port {
        if port == 0 {
            return bad("invalid_port", "Port must be between 1 and 65535");
        }
        settings.port = port;
    }
    if let Some(address) = patch.address {
        let address = address.trim().to_string();
        if !address.is_empty() && !remote_daemon::valid_address(&address) {
            return bad("invalid_address", "Address must be a host name or an IP address");
        }
        settings.address = address;
    }
    if let Some(enabled) = patch.enabled {
        settings.enabled = enabled;
    }
    if let Err(e) = config::save_settings_in(&file, &settings) {
        return (StatusCode::INTERNAL_SERVER_ERROR, Json(json!({ "error": e.to_string(), "error_code": "save_failed" }))).into_response();
    }
    let status = remote_daemon::apply(&state).await;
    Json(view(&settings, &status)).into_response()
}

pub async fn list_clients() -> Json<Value> {
    let clients: Vec<Value> = config::clients_in(&ConfigFile::default_location())
        .into_iter()
        .map(|c| json!({ "hostId": c.host_id, "name": c.name, "pairedAt": c.paired_at }))
        .collect();
    Json(json!({ "clients": clients }))
}

pub async fn revoke_client(Path(host_id): Path<String>) -> Response {
    let file = ConfigFile::default_location();
    match config::revoke_in(&file, &host_id) {
        Ok(true) => {
            remote_daemon::reload_clients(&file);
            StatusCode::NO_CONTENT.into_response()
        }
        Ok(false) => (StatusCode::NOT_FOUND, Json(json!({ "error": "Not paired", "error_code": "not_paired" }))).into_response(),
        Err(e) => (StatusCode::INTERNAL_SERVER_ERROR, Json(json!({ "error": e.to_string(), "error_code": "save_failed" }))).into_response(),
    }
}
