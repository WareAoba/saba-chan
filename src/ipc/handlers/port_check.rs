//! 포트포워딩 확인 (`crate::port_check`)
//!
//!   POST /api/instance/:id/port-check   → 확인 결과 (`port_check::Report`). 릴레이에 연결하지 않았으면 409 `node_token_missing`
//!   GET  /api/instance/:id/port-check   → 마지막 결과, 확인한 적 없으면 `{instanceId, verdict:"not_checked"}`
//!   GET  /api/instance/:id/address      → `{instanceId, address, custom, shareAddress, running, ports:[{field, label, port, protocols}]}` 서버 주소.
//!        `address`는 이 PC의 공개 주소(`remote_daemon::public_address`): 정한 도메인·IP(`custom:true`), 없으면 릴레이가 본 공인 IP
//!        (연결돼 있지 않거나 모르면 null). `shareAddress`는 디스코드에 알려 줄지(`/api/public-address`). 포트는 확인할 포트와 같다 (같은 포트의 TCP·UDP는 하나로)
//!
//! `:id`는 인스턴스 id, 없으면 이름으로 찾는다 (CLI·채팅처럼 이름만 아는 쪽)

use axum::{extract::Path, extract::State, http::StatusCode, response::IntoResponse, Json};
use serde_json::json;

use super::super::IPCServer;
use crate::port_check::CheckError;

async fn resolve(state: &IPCServer, id: &str) -> Option<String> {
    let sup = state.supervisor.read().await;
    sup.instance_store.find(id).map(|i| i.id.clone())
}

fn not_found(id: &str) -> axum::response::Response {
    super::error(StatusCode::NOT_FOUND, "not_found", format!("Instance not found: {}", id))
}

/// POST /api/instance/:id/port-check
pub async fn check(Path(id): Path<String>, State(state): State<IPCServer>) -> axum::response::Response {
    let Some(id) = resolve(&state, &id).await else { return not_found(&id) };
    match crate::port_check::check(&state, std::slice::from_ref(&id)).await {
        Ok(mut reports) => match reports.pop() {
            Some(report) => Json(report).into_response(),
            None => not_found(&id),
        },
        Err(e) => {
            let status = match e {
                CheckError::NotPaired | CheckError::Busy => StatusCode::CONFLICT,
                CheckError::RateLimited => StatusCode::TOO_MANY_REQUESTS,
                CheckError::Rejected | CheckError::Relay(_) => StatusCode::BAD_GATEWAY,
            };
            match &e {
                CheckError::Relay(detail) => tracing::warn!("Port check for {} failed: {}", id, detail),
                CheckError::Rejected => tracing::warn!("Port check for {} failed: relay answered 400", id),
                _ => {}
            }
            let message = match &e {
                CheckError::NotPaired => "This PC is not linked to the relay",
                CheckError::Busy => "A port check is already running",
                CheckError::RateLimited => "Too many port checks; try again later",
                CheckError::Rejected | CheckError::Relay(_) => "Could not reach the relay",
            };
            super::error(status, e.code(), message)
        }
    }
}

/// GET /api/instance/:id/port-check
pub async fn last(Path(id): Path<String>, State(state): State<IPCServer>) -> axum::response::Response {
    let Some(id) = resolve(&state, &id).await else { return not_found(&id) };
    match crate::port_check::last(&id) {
        Some(report) => Json(report).into_response(),
        None => Json(json!({ "instanceId": id, "verdict": "not_checked" })).into_response(),
    }
}

/// GET /api/instance/:id/address
pub async fn address(Path(id): Path<String>, State(state): State<IPCServer>) -> axum::response::Response {
    let Some(id) = resolve(&state, &id).await else { return not_found(&id) };
    let labels: std::collections::HashMap<String, String> = {
        let sup = state.supervisor.read().await;
        let module = sup.instance_store.get(&id).and_then(|i| sup.module_loader.get_module(&i.module_name).ok());
        module.and_then(|m| m.metadata.settings).map(|s| s.fields.into_iter().map(|f| (f.name, f.label)).collect()).unwrap_or_default()
    };
    let Some(target) = crate::port_check::targets(&state, std::slice::from_ref(&id)).await.pop() else { return not_found(&id) };
    let mut ports: Vec<serde_json::Value> = Vec::new();
    for p in &target.ports {
        let protocol = json!(p.protocol);
        match ports.iter_mut().find(|e| e["port"] == p.port) {
            Some(e) => e["protocols"].as_array_mut().into_iter().for_each(|a| a.push(protocol.clone())),
            None => ports.push(json!({ "field": p.field, "label": labels.get(&p.field).cloned().unwrap_or_else(|| p.field.clone()), "port": p.port, "protocols": [protocol] })),
        }
    }
    let settings = crate::remote_daemon::config::settings_in(&saba_chan_updater_lib::config_file::ConfigFile::default_location());
    let (address, custom) = crate::remote_daemon::public_address(&settings, state.relay_agent.seen_address());
    Json(json!({ "instanceId": id, "address": address, "custom": custom, "shareAddress": settings.share_address, "running": target.running, "ports": ports }))
        .into_response()
}
