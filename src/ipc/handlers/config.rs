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

/// GET /api/config/gui — GUI 설정(settings.json) 조회
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
        Ok(_) => {
            // 언어 변경을 플러그인 러너에 즉시 반영 (모듈 i18n SABA_LANG)
            let lang = state.config_store.get_gui_settings().await.language;
            crate::plugin::set_plugin_lang(lang);
            (StatusCode::OK, Json(json!({ "ok": true }))).into_response()
        }
        Err(e) => (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(json!({ "error": format!("{}", e) })),
        ).into_response(),
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
    State(state): State<IPCServer>,
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
        Ok(_) => {
            // 페어링 직후 데몬 재시작 없이 하트비트/폴링이 시작되도록 핫 리로드
            let started = state.cloud_agent.restart_from_file().await;
            tracing::info!("[Config] node token 저장 — cloud agent 핫 리로드 (running={})", started);
            (StatusCode::OK, Json(json!({ "ok": true }))).into_response()
        }
        Err(e) => (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(json!({ "error": format!("Failed to write node token: {}", e) })),
        )
            .into_response(),
    }
}

/// DELETE /api/config/node-token
pub async fn delete_node_token(State(state): State<IPCServer>) -> impl IntoResponse {
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
    // 토큰 삭제 = 클라우드 연결 해제 — 실행 중인 에이전트 정지
    state.cloud_agent.stop().await;
    tracing::info!("[Config] node token 삭제 — cloud agent 정지");
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

/// GET /api/system/components — 설치된 컴포넌트 버전 일람
///
/// 단일 SoT는 `%APPDATA%/Roaming/saba-chan/installed-manifest.json`이다.
/// 핵심 코어 컴포넌트(saba-core/cli/gui/updater/installer)가 매니페스트에
/// 누락된 경우(예: 인스톨러 없이 개발 빌드를 실행한 경우)에는 데몬 자신의
/// `CARGO_PKG_VERSION`을 fallback으로 사용한다. workspace.package.version이
/// 단일 SoT이므로 5개 코어 컴포넌트는 동일 버전을 갖는다.
pub async fn get_system_components(State(_state): State<IPCServer>) -> impl IntoResponse {
    let manifest_path = saba_chan_updater_lib::constants::resolve_installed_manifest_path();
    let installed = saba_chan_updater_lib::UpdateManager::load_installed_manifest();
    let core_version = env!("CARGO_PKG_VERSION");

    // 핵심 코어 컴포넌트는 데몬 자신의 버전으로 채워둔다(설치본이 없을 때 fallback)
    let mut components = serde_json::Map::new();
    for key in ["saba-core", "cli", "gui", "updater", "installer"] {
        let ver = installed.get(key).map(String::as_str).unwrap_or(core_version);
        components.insert(key.to_string(), json!(ver));
    }
    // 모듈/익스텐션 등 추가 컴포넌트는 매니페스트에 기록된 그대로 노출
    for (k, v) in &installed {
        if !components.contains_key(k) {
            components.insert(k.clone(), json!(v));
        }
    }

    // lastUpdated는 epoch milliseconds(JS Date에서 직접 파싱 가능). manifest 파일이
    // 없을 때(=인스톨러 없이 개발 빌드를 직접 실행한 경우)는 null로 두어 GUI가
    // "마지막 업데이트" 라벨을 숨기도록 한다.
    let last_updated_ms: Option<u64> = std::fs::metadata(&manifest_path)
        .ok()
        .and_then(|m| m.modified().ok())
        .and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok())
        .map(|d| d.as_millis() as u64);

    (
        StatusCode::OK,
        Json(json!({
            "version": core_version,
            "components": components,
            "lastUpdated": last_updated_ms,
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
