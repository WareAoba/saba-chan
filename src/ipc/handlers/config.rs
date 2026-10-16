use axum::{
    extract::{Path, State},
    http::StatusCode,
    response::IntoResponse,
    Json,
};
use serde::Deserialize;
use serde_json::{json, Value};
use std::collections::HashMap;

use super::super::IPCServer;

// ── GUI Settings ──

/// GET /api/config/gui — GUI 설정(config.json의 gui) 조회
pub async fn get_gui_config(State(state): State<IPCServer>) -> impl IntoResponse {
    let json = state.config_store.get_gui_settings_json().await;
    (StatusCode::OK, Json(json)).into_response()
}

/// PUT /api/config/gui — GUI 설정 전체 저장
pub async fn save_gui_config(
    State(state): State<IPCServer>,
    Json(settings): Json<Value>,
) -> impl IntoResponse {
    // portConflictCheck 동기화 (기존 sync_gui_config 로직 통합)
    if let Some(port_conflict_check) = settings.get("portConflictCheck").and_then(|v| v.as_bool()) {
        let skip = !port_conflict_check;
        let supervisor = state.supervisor.read().await;
        supervisor
            .skip_port_check
            .store(skip, std::sync::atomic::Ordering::Relaxed);
    }

    match state.config_store.set_gui_settings_from_json(settings).await {
        Ok(_) => (StatusCode::OK, Json(json!({ "ok": true }))).into_response(),
        Err(e) => (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(json!({ "error": format!("{}", e) })),
        ).into_response(),
    }
}

// ── Client namespaces ──

/// GET /api/config/client/:kind — 클라이언트별 설정 네임스페이스 (예: `cli`)
pub async fn get_client_config(
    Path(kind): Path<String>,
    State(state): State<IPCServer>,
) -> impl IntoResponse {
    match state.config_store.get_client_section(&kind) {
        Ok(v) => (StatusCode::OK, Json(v)).into_response(),
        Err(e) => (StatusCode::BAD_REQUEST, Json(json!({ "error": e.to_string() }))).into_response(),
    }
}

/// PUT /api/config/client/:kind — 최상위 키 단위로 머지해 저장, 결과를 돌려준다
pub async fn save_client_config(
    Path(kind): Path<String>,
    State(state): State<IPCServer>,
    Json(patch): Json<Value>,
) -> impl IntoResponse {
    match state.config_store.merge_client_section(&kind, patch) {
        Ok(v) => (StatusCode::OK, Json(v)).into_response(),
        Err(crate::config_store::ConfigStoreError::InvalidData(msg)) => {
            (StatusCode::BAD_REQUEST, Json(json!({ "error": msg }))).into_response()
        }
        Err(e) => (StatusCode::INTERNAL_SERVER_ERROR, Json(json!({ "error": e.to_string() }))).into_response(),
    }
}

// ── Node Token ──

/// GET /api/config/node-token
pub async fn get_node_token(State(_state): State<IPCServer>) -> impl IntoResponse {
    let data_dir = saba_chan_updater_lib::constants::resolve_data_dir();
    let token_path = data_dir.join(".node_token");
    match std::fs::read_to_string(&token_path) {
        Ok(token) => (StatusCode::OK, Json(json!({ "token": token.trim() }))).into_response(),
        Err(_) => (StatusCode::OK, Json(json!({ "token": "" }))).into_response(),
    }
}

/// PUT /api/config/node-token
pub async fn save_node_token(
    State(_state): State<IPCServer>,
    Json(payload): Json<Value>,
) -> impl IntoResponse {
    let token = payload
        .get("token")
        .and_then(|v| v.as_str())
        .unwrap_or("");
    let data_dir = saba_chan_updater_lib::constants::resolve_data_dir();
    let token_path = data_dir.join(".node_token");

    if let Some(parent) = token_path.parent() {
        let _ = std::fs::create_dir_all(parent);
    }

    match std::fs::write(&token_path, token) {
        Ok(_) => (StatusCode::OK, Json(json!({ "ok": true }))).into_response(),
        Err(e) => (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(json!({ "error": format!("Failed to write node token: {}", e) })),
        )
            .into_response(),
    }
}

/// DELETE /api/config/node-token
pub async fn delete_node_token(State(_state): State<IPCServer>) -> impl IntoResponse {
    let data_dir = saba_chan_updater_lib::constants::resolve_data_dir();
    let token_path = data_dir.join(".node_token");

    if token_path.exists() {
        if let Err(e) = std::fs::remove_file(&token_path) {
            return (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(json!({ "error": format!("Failed to delete node token: {}", e) })),
            )
                .into_response();
        }
    }
    (StatusCode::OK, Json(json!({ "ok": true }))).into_response()
}

// ── Module Locales ──

/// GET /api/module/:name/locales — 모듈 로케일 파일 전체 반환
pub async fn get_module_locales(
    State(state): State<IPCServer>,
    Path(module_name): Path<String>,
) -> impl IntoResponse {
    let supervisor = state.supervisor.read().await;
    let modules_dir = supervisor.module_loader.modules_dir().to_string();
    drop(supervisor);

    let locales_dir = std::path::PathBuf::from(&modules_dir)
        .join(&module_name)
        .join("locales");

    let mut result: HashMap<String, Value> = HashMap::new();

    if locales_dir.exists() {
        if let Ok(entries) = std::fs::read_dir(&locales_dir) {
            for entry in entries.flatten() {
                let path = entry.path();
                if path.extension().and_then(|e| e.to_str()) == Some("json") {
                    if let Some(lang) = path.file_stem().and_then(|s| s.to_str()) {
                        if let Ok(content) = std::fs::read_to_string(&path) {
                            if let Ok(parsed) = serde_json::from_str::<Value>(&content) {
                                result.insert(lang.to_string(), parsed);
                            }
                        }
                    }
                }
            }
        }
    }

    (StatusCode::OK, Json(json!(result)))
}

// ── System Components ──

/// GET /api/system/components — 설치된 컴포넌트 버전
///
/// `installed-manifest.json`(업데이터가 적용할 때마다 기록)이 기준이고, 거기에 없는
/// 컴포넌트는 이 빌드에 묶인 `versions.toml` 버전으로 채운다.
pub async fn get_system_components(State(_state): State<IPCServer>) -> impl IntoResponse {
    use saba_chan_updater_lib::{versions, UpdateManager};

    let bundled = versions::bundled();
    let mut components: std::collections::BTreeMap<String, String> = bundled.components.clone();
    components.extend(UpdateManager::load_installed_manifest());

    let manifest_path = saba_chan_updater_lib::constants::resolve_installed_manifest_path();
    let last_updated = std::fs::metadata(&manifest_path)
        .and_then(|m| m.modified())
        .ok()
        .and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok())
        .map(|d| d.as_secs().to_string());

    (
        StatusCode::OK,
        Json(json!({
            "version": bundled.release,
            "components": components,
            "lastUpdated": last_updated
        })),
    )
        .into_response()
}

// ── Directory Scan (migration) ──

#[derive(Deserialize)]
pub struct ScanDirRequest {
    pub path: String,
}

/// POST /api/fs/scan-dir — 디렉토리 내용 스캔 (마이그레이션 등)
pub async fn scan_directory(
    State(_state): State<IPCServer>,
    Json(req): Json<ScanDirRequest>,
) -> impl IntoResponse {
    let dir_path = std::path::Path::new(&req.path);

    if !dir_path.exists() {
        return (
            StatusCode::NOT_FOUND,
            Json(json!({ "error": "Directory not found" })),
        )
            .into_response();
    }

    match std::fs::read_dir(dir_path) {
        Ok(entries) => {
            let mut files = Vec::new();
            let mut dirs = Vec::new();
            for entry in entries.flatten() {
                let name = entry.file_name().to_string_lossy().to_string();
                if let Ok(ft) = entry.file_type() {
                    if ft.is_file() {
                        files.push(name);
                    } else if ft.is_dir() {
                        dirs.push(name);
                    }
                }
            }
            (StatusCode::OK, Json(json!({ "ok": true, "files": files, "dirs": dirs }))).into_response()
        }
        Err(e) => {
            let msg = if e.kind() == std::io::ErrorKind::PermissionDenied {
                "Permission denied"
            } else {
                "Failed to read directory"
            };
            (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(json!({ "error": msg })),
            )
                .into_response()
        }
    }
}
