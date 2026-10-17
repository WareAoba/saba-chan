use axum::{
    extract::{Path, State},
    http::StatusCode,
    response::IntoResponse,
    Json,
};
use serde_json::json;
use std::collections::HashMap;

use super::super::IPCServer;
use crate::supervisor::Supervisor;

/// POST /api/instance/:id/managed/start — Start with managed process (stdin/stdout capture)
pub async fn start_managed_handler(
    Path(id): Path<String>,
    State(state): State<IPCServer>,
    payload: Option<Json<serde_json::Value>>,
) -> impl IntoResponse {
    // ── 실행 전 비밀번호 자동 생성 (비어있으면 채움) ──
    {
        let mut supervisor = state.supervisor.write().await;
        if let Some(mut inst) = supervisor.instance_store.get(&id).cloned() {
            let changed = inst.ensure_passwords();

            if changed {
                if let Err(e) = supervisor.instance_store.update(&id, inst) {
                    tracing::warn!("Failed to save auto-generated passwords for {}: {}", id, e);
                }
            }
        }
    }

    let (module_name, config) = {
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
        (module_name, config)
    };

    // 락은 start_managed_server가 단계마다 짧게 잡는다 (C12)
    match Supervisor::start_managed_server(&state.supervisor, &id, &module_name, config).await {
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
            Json(json!({"error": e.to_string(), "error_code": "not_found"})),
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
    // 명령 경로와 같은 응답: 보낸 뒤 찍힌 콘솔 줄을 data.response로
    let resp = super::command::via_stdin(&state, &id, &payload).await.into_response();
    let line = payload.get("command").or_else(|| payload.get("line")).and_then(|v| v.as_str()).map(str::to_string);
    super::audited(&state, "stdin", &id, line.as_deref(), None, resp).await
}

/// POST /api/instance/:id/validate — Validate prerequisites
pub async fn validate_instance_handler(
    Path(id): Path<String>,
    State(state): State<IPCServer>,
) -> impl IntoResponse {
    match Supervisor::validate_instance(&state.supervisor, &id).await {
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
    match Supervisor::manage_properties(&state.supervisor, &id, "read", None).await {
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
    let settings = payload.get("settings").cloned();
    match Supervisor::manage_properties(&state.supervisor, &id, "write", settings).await {
        Ok(result) => (StatusCode::OK, Json(result)).into_response(),
        Err(e) => (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(json!({"error": e.to_string()})),
        )
            .into_response(),
    }
}

/// GET /api/instance/:id/settings/live — 디스크의 실제 설정을 모듈 `import_settings`로 읽어 돌려준다.
///
/// `module_settings`는 마지막으로 saba-chan이 저장한 값의 캐시라 게임 안이나 파일에서 바꾼 값과
/// 어긋날 수 있다. 읽기에 성공하면 캐시도 실제 값으로 맞춘다. 실패하면 `success: false`를 그대로
/// 돌려주므로, 인터페이스는 그때 기본값으로 덮어쓰지 말아야 한다. Python은 락 밖에서 돈다.
pub async fn read_live_settings_handler(
    Path(id): Path<String>,
    State(state): State<IPCServer>,
) -> impl IntoResponse {
    match Supervisor::manage_properties(&state.supervisor, &id, "import", None).await {
        Ok(result) => {
            let ok = result.get("success").and_then(|v| v.as_bool()) == Some(true);
            let settings = result.get("settings").and_then(|v| v.as_object()).filter(|s| !s.is_empty());
            if let (true, Some(settings)) = (ok, settings) {
                let mut supervisor = state.supervisor.write().await;
                if let Some(mut inst) = supervisor.instance_store.get(&id).cloned() {
                    let (credential_map, types) = supervisor.module_loader.get_module(&inst.module_name)
                        .map(|m| (m.metadata.credential_map.clone(), m.metadata.setting_types()))
                        .unwrap_or_default();
                    // 포트·비밀번호 같은 인스턴스 필드도 파일 값으로 맞춘다 (GUI는 그쪽을 먼저 읽는다)
                    inst.apply_imported_settings(settings, &credential_map, &types);
                    if let Err(e) = supervisor.instance_store.update(&id, inst) {
                        tracing::warn!("Live settings read: cache refresh failed for {}: {}", id, e);
                    }
                }
            }
            (StatusCode::OK, Json(result)).into_response()
        }
        Err(e) => (StatusCode::INTERNAL_SERVER_ERROR, Json(json!({ "success": false, "error": e.to_string() }))).into_response(),
    }
}

/// POST /api/instance/:id/properties/reset — Reset server.properties to defaults
pub async fn reset_properties_handler(
    Path(id): Path<String>,
    State(state): State<IPCServer>,
) -> impl IntoResponse {
    match Supervisor::manage_properties(&state.supervisor, &id, "reset", None).await {
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
    match Supervisor::manage_properties(&state.supervisor, &id, "reset_server", None).await {
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
    match Supervisor::accept_eula(&state.supervisor, &id).await {
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
    match Supervisor::diagnose_instance(&state.supervisor, &id).await {
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

    match Supervisor::list_versions(&state.supervisor, &module_name, include_snapshots, page, per_page).await {
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
    match Supervisor::get_version_details(&state.supervisor, &module_name, &version).await {
        Ok(result) => (StatusCode::OK, Json(result)).into_response(),
        Err(e) => (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(json!({"error": e.to_string()})),
        )
            .into_response(),
    }
}

/// GET /api/instance/:id/installed-version — Detect installed server version from binary
/// 감지 성공 시 instance.server_version에 자동 저장하여 이후 리스트에서 바로 표시
pub async fn get_installed_version_handler(
    Path(id): Path<String>,
    State(state): State<IPCServer>,
) -> impl IntoResponse {
    // Phase 1: 감지 (모듈 호출은 락 밖에서)
    let (_module_name, result) = {
        let found = state.supervisor.read().await.instance_store.list().iter().find(|i| i.id == id).cloned();
        let instance = match found {
            Some(i) => i,
            None => {
                return (
                    StatusCode::NOT_FOUND,
                    Json(json!({"error": format!("Instance '{}' not found", id)})),
                )
                    .into_response()
            }
        };

        let module_name = instance.module_name.clone();
        let res = Supervisor::get_installed_version(&state.supervisor, &module_name, &id).await;
        (module_name, res)
    };

    match result {
        Ok(result) => {
            // Phase 2: 감지 성공 시 write lock으로 인스턴스에 버전 저장
            if result.get("success").and_then(|s| s.as_bool()) == Some(true) {
                if let Some(version) = result.get("version").and_then(|v| v.as_str()) {
                    let mut supervisor = state.supervisor.write().await;
                    if let Some(mut instance) = supervisor.instance_store.get(&id).cloned() {
                        let already_set = instance.server_version.as_deref() == Some(version);
                        if !already_set {
                            instance.server_version = Some(version.to_string());
                            if let Err(e) = supervisor.instance_store.update(&id, instance) {
                                tracing::warn!("Failed to persist detected version for '{}': {}", id, e);
                            } else {
                                tracing::info!("Auto-saved detected server version '{}' for instance '{}'", version, id);
                            }
                        }
                    }
                }
            }
            (StatusCode::OK, Json(result)).into_response()
        }
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

    // ── 필수 익스텐션 의존성 검증 ──
    {
        let found = state.supervisor.read().await.module_loader.get_module(&module_name);
        let module = match found {
            Ok(m) => m,
            Err(e) => {
                return (
                    StatusCode::NOT_FOUND,
                    Json(json!({"error": format!("Module '{}' not found: {}", module_name, e)})),
                )
                    .into_response()
            }
        };
        if let Some(ref install_cfg) = module.metadata.install {
            if !install_cfg.requires_extensions.is_empty() {
                let ext_mgr = state.extension_manager.read().await;
                let ready = ext_mgr.installed_and_enabled_set();
                let enabled_not_installed = ext_mgr.enabled_but_not_installed();
                drop(ext_mgr);

                let missing: Vec<String> = install_cfg
                    .requires_extensions
                    .iter()
                    .filter(|ext_id| !ready.contains(ext_id.as_str()))
                    .cloned()
                    .collect();

                if !missing.is_empty() {
                    let not_installed: Vec<&String> = missing
                        .iter()
                        .filter(|id| enabled_not_installed.contains(id))
                        .collect();
                    let msg = if !not_installed.is_empty() {
                        format!(
                            "Cannot install server: required extension(s) not installed: {}. Install them in Settings → Extensions first.",
                            not_installed.iter().map(|s| s.as_str()).collect::<Vec<_>>().join(", ")
                        )
                    } else {
                        format!(
                            "Cannot install server: required extension(s) not enabled: {}. Enable them in Settings → Extensions first.",
                            missing.join(", ")
                        )
                    };
                    return (
                        StatusCode::UNPROCESSABLE_ENTITY,
                        Json(json!({
                            "error": msg,
                            "error_code": "extension_required",
                            "missing_extensions": missing,
                            "not_installed": not_installed,
                        })),
                    )
                        .into_response();
                }
            }
        }
    }

    match Supervisor::install_server(
            &state.supervisor,
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
