use axum::{
    extract::{Path, State},
    http::StatusCode,
    response::IntoResponse,
    Json,
};
use serde_json::json;
use std::collections::HashMap;

use super::super::IPCServer;

/// POST /api/instance/:id/managed/start — Start with managed process (stdin/stdout capture)
pub async fn start_managed_handler(
    Path(id): Path<String>,
    State(state): State<IPCServer>,
    payload: Option<Json<serde_json::Value>>,
) -> impl IntoResponse {
    let supervisor = state.supervisor.read().await;

    let instance = match supervisor.instance_store.get(&id) {
        Some(i) => i,
        None => {
            return (
                StatusCode::NOT_FOUND,
                Json(json!({"error": format!("Instance not found: {}", id)})),
            )
                .into_response()
        }
    };

    // API 경유 시작 기록 (name과 id 모두)
    state.api_actions.record(&instance.name);
    state.api_actions.record(&id);

    let module_name = instance.module_name.clone();
    let payload_val = payload.map(|j| j.0).unwrap_or(json!({}));
    let config = payload_val.get("config").cloned().unwrap_or(json!({}));

    match supervisor
        .start_managed_server(&id, &module_name, config)
        .await
    {
        Ok(result) => (StatusCode::OK, Json(result)).into_response(),
        Err(e) => (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(json!({"error": e.to_string()})),
        )
            .into_response(),
    }
}

/// GET /api/instance/:id/console?since=0&count=100 — Get console output
pub async fn get_console_handler(
    Path(id): Path<String>,
    State(state): State<IPCServer>,
    axum::extract::Query(params): axum::extract::Query<HashMap<String, String>>,
) -> impl IntoResponse {
    let supervisor = state.supervisor.read().await;

    let since_id = params.get("since").and_then(|s| s.parse::<u64>().ok());
    let count = params.get("count").and_then(|c| c.parse::<usize>().ok());

    match supervisor.get_console_output(&id, since_id, count).await {
        Ok(result) => (StatusCode::OK, Json(result)).into_response(),
        Err(e) => (
            StatusCode::NOT_FOUND,
            Json(json!({"error": e.to_string()})),
        )
            .into_response(),
    }
}

/// POST /api/instance/:id/stdin — Send command to stdin
pub async fn send_stdin_handler(
    Path(id): Path<String>,
    State(state): State<IPCServer>,
    Json(payload): Json<serde_json::Value>,
) -> impl IntoResponse {
    let supervisor = state.supervisor.read().await;

    let command = match payload.get("command").and_then(|v| v.as_str()) {
        Some(cmd) => cmd,
        None => {
            return (
                StatusCode::BAD_REQUEST,
                Json(json!({"error": "Missing 'command' field"})),
            )
                .into_response()
        }
    };

    match supervisor.send_stdin_command(&id, command).await {
        Ok(msg) => (
            StatusCode::OK,
            Json(json!({"success": true, "message": msg})),
        )
            .into_response(),
        Err(e) => (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(json!({"error": e.to_string()})),
        )
            .into_response(),
    }
}

/// POST /api/instance/:id/validate — Validate prerequisites
pub async fn validate_instance_handler(
    Path(id): Path<String>,
    State(state): State<IPCServer>,
) -> impl IntoResponse {
    let supervisor = state.supervisor.read().await;

    match supervisor.validate_instance(&id).await {
        Ok(result) => (StatusCode::OK, Json(result)).into_response(),
        Err(e) => (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(json!({"error": e.to_string()})),
        )
            .into_response(),
    }
}

/// GET /api/instance/:id/properties — Read server.properties
pub async fn read_properties_handler(
    Path(id): Path<String>,
    State(state): State<IPCServer>,
) -> impl IntoResponse {
    let supervisor = state.supervisor.read().await;

    match supervisor.manage_properties(&id, "read", None).await {
        Ok(result) => (StatusCode::OK, Json(result)).into_response(),
        Err(e) => (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(json!({"error": e.to_string()})),
        )
            .into_response(),
    }
}

/// PUT /api/instance/:id/properties — Update server.properties
pub async fn write_properties_handler(
    Path(id): Path<String>,
    State(state): State<IPCServer>,
    Json(payload): Json<serde_json::Value>,
) -> impl IntoResponse {
    let supervisor = state.supervisor.read().await;

    let settings = payload.get("settings").cloned();
    match supervisor
        .manage_properties(&id, "write", settings)
        .await
    {
        Ok(result) => (StatusCode::OK, Json(result)).into_response(),
        Err(e) => (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(json!({"error": e.to_string()})),
        )
            .into_response(),
    }
}

/// POST /api/instance/:id/properties/reset — Reset server.properties to defaults
pub async fn reset_properties_handler(
    Path(id): Path<String>,
    State(state): State<IPCServer>,
) -> impl IntoResponse {
    let supervisor = state.supervisor.read().await;

    match supervisor.manage_properties(&id, "reset", None).await {
        Ok(result) => (StatusCode::OK, Json(result)).into_response(),
        Err(e) => (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(json!({"error": e.to_string()})),
        )
            .into_response(),
    }
}

/// POST /api/instance/:id/server/reset — Full server reset (delete worlds, configs, etc.)
pub async fn reset_server_handler(
    Path(id): Path<String>,
    State(state): State<IPCServer>,
) -> impl IntoResponse {
    let supervisor = state.supervisor.read().await;

    match supervisor
        .manage_properties(&id, "reset_server", None)
        .await
    {
        Ok(result) => (StatusCode::OK, Json(result)).into_response(),
        Err(e) => (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(json!({"error": e.to_string()})),
        )
            .into_response(),
    }
}

/// POST /api/instance/:id/accept-eula — Accept Minecraft EULA
pub async fn accept_eula_handler(
    Path(id): Path<String>,
    State(state): State<IPCServer>,
) -> impl IntoResponse {
    let supervisor = state.supervisor.read().await;

    match supervisor.accept_eula(&id).await {
        Ok(result) => (StatusCode::OK, Json(result)).into_response(),
        Err(e) => (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(json!({"error": e.to_string()})),
        )
            .into_response(),
    }
}

/// POST /api/instance/:id/diagnose — Diagnose errors
pub async fn diagnose_handler(
    Path(id): Path<String>,
    State(state): State<IPCServer>,
) -> impl IntoResponse {
    let supervisor = state.supervisor.read().await;

    match supervisor.diagnose_instance(&id).await {
        Ok(result) => (StatusCode::OK, Json(result)).into_response(),
        Err(e) => (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(json!({"error": e.to_string()})),
        )
            .into_response(),
    }
}

// ─── Server Installation Handlers ────────────────────────────

/// GET /api/module/:name/versions?include_snapshots=false&page=1&per_page=25
/// List available Minecraft server versions from Mojang
pub async fn list_versions_handler(
    Path(module_name): Path<String>,
    State(state): State<IPCServer>,
    axum::extract::Query(params): axum::extract::Query<HashMap<String, String>>,
) -> impl IntoResponse {
    let supervisor = state.supervisor.read().await;

    let include_snapshots = params
        .get("include_snapshots")
        .map(|v| v == "true" || v == "1")
        .unwrap_or(false);
    let page = params
        .get("page")
        .and_then(|v| v.parse::<u32>().ok())
        .unwrap_or(1);
    let per_page = params
        .get("per_page")
        .and_then(|v| v.parse::<u32>().ok())
        .unwrap_or(25);

    match supervisor
        .list_versions(&module_name, include_snapshots, page, per_page)
        .await
    {
        Ok(result) => (StatusCode::OK, Json(result)).into_response(),
        Err(e) => (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(json!({"error": e.to_string()})),
        )
            .into_response(),
    }
}

/// GET /api/module/:name/version/:version — Get detailed info for a specific version
pub async fn get_version_details_handler(
    Path((module_name, version)): Path<(String, String)>,
    State(state): State<IPCServer>,
) -> impl IntoResponse {
    let supervisor = state.supervisor.read().await;

    match supervisor
        .get_version_details(&module_name, &version)
        .await
    {
        Ok(result) => (StatusCode::OK, Json(result)).into_response(),
        Err(e) => (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(json!({"error": e.to_string()})),
        )
            .into_response(),
    }
}

/// POST /api/module/:name/install — Install a server
/// Body: { "version": "1.21.11", "install_dir": "/path/to/server",
///         "jar_name": "server.jar", "accept_eula": true, "initial_settings": {...} }
pub async fn install_server_handler(
    Path(module_name): Path<String>,
    State(state): State<IPCServer>,
    Json(payload): Json<serde_json::Value>,
) -> impl IntoResponse {
    let supervisor = state.supervisor.read().await;

    let version = match payload.get("version").and_then(|v| v.as_str()) {
        Some(v) => v.to_string(),
        None => {
            return (
                StatusCode::BAD_REQUEST,
                Json(json!({"error": "Missing 'version' field"})),
            )
                .into_response()
        }
    };

    let install_dir = match payload.get("install_dir").and_then(|v| v.as_str()) {
        Some(v) => v.to_string(),
        None => {
            return (
                StatusCode::BAD_REQUEST,
                Json(json!({"error": "Missing 'install_dir' field"})),
            )
                .into_response()
        }
    };

    let jar_name = payload.get("jar_name").and_then(|v| v.as_str());
    let accept_eula = payload
        .get("accept_eula")
        .and_then(|v| v.as_bool())
        .unwrap_or(false);
    let initial_settings = payload.get("initial_settings").cloned();

    match supervisor
        .install_server(
            &module_name,
            &version,
            &install_dir,
            jar_name,
            accept_eula,
            initial_settings,
        )
        .await
    {
        Ok(result) => {
            let status = if result.get("success").and_then(|s| s.as_bool()) == Some(true) {
                StatusCode::OK
            } else {
                StatusCode::UNPROCESSABLE_ENTITY
            };
            (status, Json(result)).into_response()
        }
        Err(e) => (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(json!({"error": e.to_string()})),
        )
            .into_response(),
    }
}
