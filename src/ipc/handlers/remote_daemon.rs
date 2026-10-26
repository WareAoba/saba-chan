//! 원격 데몬 설정과 짝 맺은 PC (D1, `crate::remote_daemon`). 로컬 전용: 원격 포트에서는 막는다
//!
//!   GET    /api/remote/settings            {enabled, port, address, transport, status}
//!   PUT    /api/remote/settings            {enabled?, port?, address?} → 저장하고 원격 포트를 다시 맞춘다
//!   GET    /api/remote/clients             {clients:[{hostId, name, pairedAt}]}
//!   DELETE /api/remote/clients/:hostId     짝을 푼다 (그 PC의 토큰이 더는 통하지 않는다)
//!
//! 붙는 쪽 (D1b, `remote_daemon::peer`):
//!   GET    /api/remote/peers                         {peers:[{hostId, name, status, hasEndpoint, saved}]} 같은 주인의 다른 PC (릴레이)
//!   GET    /api/remote/daemons                       {daemons:[{hostId, name, address, port, pairedAt}]} 이 PC가 붙는 원격 데몬
//!   POST   /api/remote/daemons {hostId}              짝을 맺고 저장한다 → {hostId, name}
//!   DELETE /api/remote/daemons/:hostId               잊는다
//!   ANY    /api/remote/daemons/:hostId/proxy/*path   원격 데몬의 `/path`(`api/…`·`health`)에 그대로 보낸다

use axum::{
    body::Body,
    extract::{Path, State},
    http::{Request, StatusCode},
    response::{IntoResponse, Response},
    Json,
};
use saba_chan_updater_lib::config_file::ConfigFile;
use serde::Deserialize;
use serde_json::{json, Value};

use crate::ipc::IPCServer;
use crate::remote_daemon::{self, config, peer};

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

async fn relay_target(state: &IPCServer) -> Option<remote_daemon::relay::Target> {
    remote_daemon::server::RelaySource::Configured(state.clone()).target().await
}

pub async fn list_peers(State(state): State<IPCServer>) -> Response {
    let Some(target) = relay_target(&state).await else {
        return peer::error(StatusCode::SERVICE_UNAVAILABLE, "relay_not_paired");
    };
    let Ok(peers) = remote_daemon::relay::peers(&target).await else {
        return peer::error(StatusCode::BAD_GATEWAY, "relay_unreachable");
    };
    let saved = config::daemons_in(&ConfigFile::default_location());
    let peers: Vec<Value> = peers
        .into_iter()
        .map(|p| {
            let is_saved = saved.iter().any(|d| d.host_id == p.host_id);
            json!({ "hostId": p.host_id, "name": p.name, "status": p.status, "hasEndpoint": p.endpoint.is_some(), "saved": is_saved })
        })
        .collect();
    Json(json!({ "peers": peers })).into_response()
}

pub async fn list_daemons() -> Json<Value> {
    let daemons: Vec<Value> = config::daemons_in(&ConfigFile::default_location())
        .into_iter()
        .map(|d| json!({ "hostId": d.host_id, "name": d.name, "address": d.address, "port": d.port, "pairedAt": d.paired_at }))
        .collect();
    Json(json!({ "daemons": daemons }))
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ConnectBody {
    host_id: String,
}

pub async fn connect_daemon(State(state): State<IPCServer>, Json(body): Json<ConnectBody>) -> Response {
    let host_id = body.host_id.trim();
    if host_id.is_empty() {
        return bad("invalid_host", "hostId is required");
    }
    match peer::connect(&ConfigFile::default_location(), relay_target(&state).await, host_id).await {
        Ok(d) => Json(json!({ "hostId": d.host_id, "name": d.name })).into_response(),
        Err(e) => peer::error(e.status(), e.code()),
    }
}

pub async fn forget_daemon(Path(host_id): Path<String>) -> Response {
    match config::forget_daemon_in(&ConfigFile::default_location(), &host_id) {
        Ok(true) => StatusCode::NO_CONTENT.into_response(),
        Ok(false) => peer::error(StatusCode::NOT_FOUND, "remote_unknown"),
        Err(_) => peer::error(StatusCode::INTERNAL_SERVER_ERROR, "save_failed"),
    }
}

/// 원격 데몬에 넘길 경로: 데몬 API와 상태 확인만 (짝 맺기 같은 원격 포트 자체의 길은 넘기지 않는다)
fn forward_path(rest: &str, query: Option<&str>) -> Option<String> {
    let rest = rest.trim_start_matches('/');
    if !(rest == "health" || rest.starts_with("api/")) || rest.split('/').any(|s| s == ".." || s == ".") {
        return None;
    }
    Some(match query {
        Some(q) => format!("/{rest}?{q}"),
        None => format!("/{rest}"),
    })
}

pub async fn proxy(State(state): State<IPCServer>, Path((host_id, _)): Path<(String, String)>, req: Request<Body>) -> Response {
    // 풀지 않은 경로 그대로 넘긴다 (원격 데몬이 다시 푼다)
    // `/api/remote/daemons/<hostId>/proxy/<rest>`
    let mut parts = req.uri().path().splitn(7, '/').skip(5);
    let rest = match (parts.next(), parts.next()) {
        (Some("proxy"), Some(rest)) => rest,
        _ => "",
    };
    let Some(path) = forward_path(rest, req.uri().query()) else {
        return peer::error(StatusCode::NOT_FOUND, "not_found");
    };
    peer::forward(&ConfigFile::default_location(), relay_target(&state).await, &host_id, &path, req).await
}

#[cfg(test)]
mod tests {
    #[test]
    fn only_daemon_api_paths_are_forwarded() {
        assert_eq!(super::forward_path("api/instances", Some("a=1")).as_deref(), Some("/api/instances?a=1"));
        assert_eq!(super::forward_path("health", None).as_deref(), Some("/health"));
        for bad in ["remote/pair", "api/../remote/pair", "", "apix"] {
            assert_eq!(super::forward_path(bad, None), None, "{bad}");
        }
    }
}
