//! 데몬 내장 업데이트 API — `/api/updates/*`
//!
//! 업데이트 확인(check)과 다운로드(download)를 데몬 프로세스 내에서 수행합니다.
//! 파일 적용(apply)은 별도 업데이터 프로세스에서 처리합니다.
//!
//! ## 엔드포인트
//! - `GET  /api/updates/status`              — 캐시된 업데이트 상태 조회
//! - `POST /api/updates/check`               — 업데이트 확인 (GitHub API 호출)
//! - `POST /api/updates/download`            — 선택 컴포넌트 다운로드
//! - `POST /api/updates/apply`               — 업데이터 exe 스폰하여 적용
//! - `GET  /api/updates/config`              — 업데이트 설정 조회
//! - `PUT  /api/updates/config`              — 업데이트 설정 변경

use axum::{
    extract::State,
    response::IntoResponse,
    routing::{get, post},
    Json, Router,
};
use serde::Deserialize;
use serde_json::{json, Value};
use std::sync::Arc;
use tokio::sync::RwLock;

use saba_chan_updater_lib::{
    Component, UpdateConfig, UpdateManager,
};

// ═══════════════════════════════════════════════════════
// 공유 상태
// ═══════════════════════════════════════════════════════

/// 데몬 업데이트 매니저 — Arc<RwLock>으로 공유
#[derive(Clone)]
pub struct UpdateState {
    pub manager: Arc<RwLock<UpdateManager>>,
}

impl UpdateState {
    /// updater.toml 또는 global.toml [updater] 섹션에서 설정 로드 후 생성
    pub fn new() -> Self {
        let cfg = load_updater_config();
        let modules_dir = resolve_modules_dir();
        let manager = Arc::new(RwLock::new(UpdateManager::new(cfg, &modules_dir)));
        Self { manager }
    }
}

impl Default for UpdateState {
    fn default() -> Self {
        Self::new()
    }
}

// ═══════════════════════════════════════════════════════
// 라우터
// ═══════════════════════════════════════════════════════

/// `/api/updates/*` 라우트를 포함하는 axum Router 생성
pub fn updates_router(state: UpdateState) -> Router {
    Router::new()
        .route("/api/updates/status", get(get_status))
        .route("/api/updates/check", post(check_updates))
        .route("/api/updates/download", post(download_components))
        .route("/api/updates/apply", post(apply_updates))
        .route("/api/updates/config", get(get_config))
        .route("/api/updates/config", post(set_config))
        .with_state(state)
}

// ═══════════════════════════════════════════════════════
// 핸들러
// ═══════════════════════════════════════════════════════

/// GET /api/updates/status — 캐시된 상태 반환 (GitHub API 호출 없음)
async fn get_status(
    State(state): State<UpdateState>,
) -> impl IntoResponse {
    let mgr = state.manager.read().await;
    let status = mgr.get_status();

    let components: Vec<Value> = status.components.iter().map(|c| {
        json!({
            "component": c.component.manifest_key(),
            "display_name": c.component.display_name(),
            "current_version": c.current_version,
            "latest_version": c.latest_version,
            "update_available": c.update_available,
            "downloaded": c.downloaded,
            "installed": c.installed,
        })
    }).collect();

    Json(json!({
        "ok": true,
        "last_check": status.last_check,
        "next_check": status.next_check,
        "checking": status.checking,
        "error": status.error,
        "updates_available": status.components.iter().filter(|c| c.update_available).count(),
        "components": components,
    }))
}

/// POST /api/updates/check — GitHub API를 호출하여 최신 릴리스 확인
async fn check_updates(
    State(state): State<UpdateState>,
) -> impl IntoResponse {
    let result = {
        let mut mgr = state.manager.write().await;
        mgr.check_for_updates().await
    };

    match result {
        Ok(status) => {
            let components: Vec<Value> = status.components.iter().map(|c| {
                json!({
                    "component": c.component.manifest_key(),
                    "display_name": c.component.display_name(),
                    "current_version": c.current_version,
                    "latest_version": c.latest_version,
                    "update_available": c.update_available,
                    "downloaded": c.downloaded,
                    "installed": c.installed,
                    "release_notes": c.release_notes,
                })
            }).collect();

            let update_names: Vec<String> = status.components.iter()
                .filter(|c| c.update_available)
                .map(|c| c.component.display_name())
                .collect();

            Json(json!({
                "ok": true,
                "updates_available": update_names.len(),
                "update_names": update_names,
                "components": components,
                "last_check": status.last_check,
            }))
        }
        Err(e) => {
            Json(json!({
                "ok": false,
                "error": e.to_string(),
            }))
        }
    }
}

/// POST /api/updates/download — 선택한 컴포넌트 다운로드
///
/// Body: `{ "components": ["gui", "cli", "module-minecraft"] }`
/// 비어있으면 업데이트 가능한 전체를 다운로드
#[derive(Deserialize)]
struct DownloadRequest {
    /// 다운로드할 컴포넌트 키 목록. 비어있으면 전체.
    #[serde(default)]
    components: Vec<String>,
}

async fn download_components(
    State(state): State<UpdateState>,
    Json(body): Json<DownloadRequest>,
) -> impl IntoResponse {
    let mut mgr = state.manager.write().await;

    // 아직 체크하지 않았으면 먼저 체크
    if mgr.get_status().components.is_empty() {
        if let Err(e) = mgr.check_for_updates().await {
            return Json(json!({
                "ok": false,
                "error": format!("Auto-check failed: {}", e),
            }));
        }
    }

    if body.components.is_empty() {
        // 전체 다운로드
        match mgr.download_available_updates().await {
            Ok(downloaded) => {
                // 업데이터 --apply 모드를 위해 매니페스트 저장
                if let Err(e) = mgr.save_pending_manifest() {
                    tracing::warn!("[Updates] Failed to save pending manifest: {}", e);
                }
                Json(json!({
                    "ok": true,
                    "downloaded": downloaded,
                    "count": downloaded.len(),
                }))
            }
            Err(e) => {
                Json(json!({
                    "ok": false,
                    "error": e.to_string(),
                }))
            }
        }
    } else {
        // 선택 컴포넌트만 다운로드
        let mut downloaded = Vec::new();
        let mut errors = Vec::new();

        for key in &body.components {
            let component = Component::from_manifest_key(key);
            match mgr.download_component(&component).await {
                Ok(asset) => {
                    downloaded.push(json!({
                        "component": key,
                        "asset": asset,
                    }));
                }
                Err(e) => {
                    errors.push(json!({
                        "component": key,
                        "error": e.to_string(),
                    }));
                }
            }
        }

        // 업데이터 --apply 모드를 위해 매니페스트 저장
        if !downloaded.is_empty() {
            if let Err(e) = mgr.save_pending_manifest() {
                tracing::warn!("[Updates] Failed to save pending manifest: {}", e);
            }
        }

        Json(json!({
            "ok": errors.is_empty(),
            "downloaded": downloaded,
            "count": downloaded.len(),
            "errors": errors,
        }))
    }
}

/// POST /api/updates/apply — 다운로드된 업데이트 적용
///
/// - 모듈: 데몬이 직접 적용 (파일 교체)
/// - 데몬/GUI/CLI: 업데이터 exe를 스폰하여 적용 (응답에 requires_updater: true)
///
/// Body: `{ "components": ["module-minecraft", "saba-core"] }` (선택 적용, 비어있으면 전체)
#[derive(Deserialize)]
struct ApplyRequest {
    #[serde(default)]
    components: Vec<String>,
}

async fn apply_updates(
    State(state): State<UpdateState>,
    Json(body): Json<ApplyRequest>,
) -> impl IntoResponse {
    let mut mgr = state.manager.write().await;

    // 적용 대상 분류
    let pending = mgr.get_pending_components();
    let targets: Vec<Component> = if body.components.is_empty() {
        pending.iter().map(|c| c.component.clone()).collect()
    } else {
        body.components.iter()
            .map(|k| Component::from_manifest_key(k))
            .filter(|c| pending.iter().any(|p| p.component == *c))
            .collect()
    };

    // 모듈/CLI/DiscordBot는 데몬이 직접 적용, GUI/CoreDaemon은 업데이터 exe 필요
    let mut applied = Vec::new();
    let mut errors = Vec::new();
    let mut needs_updater = Vec::new();

    for comp in &targets {
        match comp {
            // 모듈/CLI/DiscordBot: 데몬이 직접 적용 (파일 교체)
            // CoreDaemon: Windows에서 실행 중 exe를 .exe.old로 rename 후 새 바이너리 추출
            Component::Module(_) | Component::Cli | Component::DiscordBot | Component::CoreDaemon => {
                match mgr.apply_single_component(comp).await {
                    Ok(result) if result.success => {
                        applied.push(comp.display_name());
                    }
                    Ok(result) => {
                        errors.push(format!("{}: {}", comp.display_name(), result.message));
                    }
                    Err(e) => {
                        errors.push(format!("{}: {}", comp.display_name(), e));
                    }
                }
            }
            // GUI: 업데이터 exe에서 적용 — GUI 종료+파일교체+재시작 필요
            Component::Gui => {
                needs_updater.push(comp.manifest_key());
            }
            // Extension: 모듈과 동일 — 데몬이 직접 적용
            Component::Extension(_) => {
                match mgr.apply_single_component(comp).await {
                    Ok(result) if result.success => {
                        applied.push(comp.display_name());
                    }
                    Ok(result) => {
                        errors.push(format!("{}: {}", comp.display_name(), result.message));
                    }
                    Err(e) => {
                        errors.push(format!("{}: {}", comp.display_name(), e));
                    }
                }
            }
        }
    }

    // 업데이터 exe가 필요한 컴포넌트가 있으면 pending manifest 재저장
    // (updater exe가 load_pending_manifest로 읽을 수 있도록 보장)
    if !needs_updater.is_empty() {
        if let Err(e) = mgr.save_pending_manifest() {
            tracing::warn!("[Updates] Failed to save pending manifest for updater: {}", e);
        }
    }

    Json(json!({
        "ok": errors.is_empty(),
        "applied": applied,
        "needs_updater": needs_updater,
        "requires_updater": !needs_updater.is_empty(),
        "errors": errors,
    }))
}

/// GET /api/updates/config
async fn get_config(
    State(state): State<UpdateState>,
) -> impl IntoResponse {
    let mgr = state.manager.read().await;
    let config = mgr.get_config();
    Json(json!({
        "ok": true,
        "config": config,
    }))
}

/// PUT /api/updates/config
async fn set_config(
    State(state): State<UpdateState>,
    Json(body): Json<Value>,
) -> impl IntoResponse {
    let mut mgr = state.manager.write().await;
    let mut cfg = mgr.get_config();

    // 부분 업데이트
    if let Some(v) = body.get("enabled").and_then(|v| v.as_bool()) {
        cfg.enabled = v;
    }
    if let Some(v) = body.get("github_owner").and_then(|v| v.as_str()) {
        cfg.github_owner = v.to_string();
    }
    if let Some(v) = body.get("github_repo").and_then(|v| v.as_str()) {
        cfg.github_repo = v.to_string();
    }
    if let Some(v) = body.get("check_interval_hours").and_then(|v| v.as_u64()) {
        cfg.check_interval_hours = v as u32;
    }
    if let Some(v) = body.get("auto_download").and_then(|v| v.as_bool()) {
        cfg.auto_download = v;
    }
    if let Some(v) = body.get("auto_apply").and_then(|v| v.as_bool()) {
        cfg.auto_apply = v;
    }
    if let Some(v) = body.get("include_prerelease").and_then(|v| v.as_bool()) {
        cfg.include_prerelease = v;
    }
    if let Some(v) = body.get("install_root").and_then(|v| v.as_str()) {
        cfg.install_root = Some(v.to_string());
    }
    if let Some(v) = body.get("api_base_url").and_then(|v| v.as_str()) {
        cfg.api_base_url = Some(v.to_string());
    }

    mgr.update_config(cfg.clone());

    // 파일에도 저장 시도
    if let Err(e) = save_updater_config(&cfg) {
        tracing::warn!("[Updates] Config save failed: {}", e);
    }

    Json(json!({
        "ok": true,
        "config": cfg,
    }))
}

// ═══════════════════════════════════════════════════════
// 설정 유틸리티
// ═══════════════════════════════════════════════════════

fn load_updater_config() -> UpdateConfig {
    let config_path = find_config_file();
    if let Some(path) = config_path {
        if let Ok(content) = std::fs::read_to_string(&path) {
            if let Ok(parsed) = content.parse::<toml::Value>() {
                // updater.toml 전체가 설정이거나, [updater] 섹션
                if let Some(updater) = parsed.get("updater") {
                    return parse_update_config(updater);
                }
                return parse_update_config(&parsed);
            }
        }
    }
    UpdateConfig::default()
}

fn find_config_file() -> Option<std::path::PathBuf> {
    use std::path::PathBuf;

    // 1. 실행 파일 옆 config/updater.toml
    if let Ok(exe) = std::env::current_exe() {
        if let Some(dir) = exe.parent() {
            let p = dir.join("config").join("updater.toml");
            if p.exists() { return Some(p); }
        }
    }

    // 2. CWD의 config/updater.toml
    let p = PathBuf::from("config").join("updater.toml");
    if p.exists() { return Some(p); }

    // 3. global.toml [updater] 섹션
    if let Ok(exe) = std::env::current_exe() {
        if let Some(dir) = exe.parent() {
            let p = dir.join("config").join("global.toml");
            if p.exists() { return Some(p); }
        }
    }
    let p = PathBuf::from("config").join("global.toml");
    if p.exists() { return Some(p); }

    None
}

fn parse_update_config(val: &toml::Value) -> UpdateConfig {
    let mut cfg = UpdateConfig::default();
    if let Some(v) = val.get("enabled").and_then(|v| v.as_bool()) { cfg.enabled = v; }
    if let Some(v) = val.get("github_owner").and_then(|v| v.as_str()) { cfg.github_owner = v.to_string(); }
    if let Some(v) = val.get("github_repo").and_then(|v| v.as_str()) { cfg.github_repo = v.to_string(); }
    if let Some(v) = val.get("check_interval_hours").and_then(|v| v.as_integer()) { cfg.check_interval_hours = v as u32; }
    if let Some(v) = val.get("auto_download").and_then(|v| v.as_bool()) { cfg.auto_download = v; }
    if let Some(v) = val.get("auto_apply").and_then(|v| v.as_bool()) { cfg.auto_apply = v; }
    if let Some(v) = val.get("include_prerelease").and_then(|v| v.as_bool()) { cfg.include_prerelease = v; }
    if let Some(v) = val.get("install_root").and_then(|v| v.as_str()) { cfg.install_root = Some(v.to_string()); }
    if let Some(v) = val.get("api_base_url").and_then(|v| v.as_str()) { cfg.api_base_url = Some(v.to_string()); }
    cfg
}

fn resolve_modules_dir() -> String {
    use std::path::PathBuf;
    for p in [
        std::env::current_exe()
            .ok()
            .and_then(|p| p.parent().map(|d| d.join("modules"))),
        Some(PathBuf::from("modules")),
    ].iter().flatten() {
        if p.exists() {
            return p.to_string_lossy().to_string();
        }
    }
    "modules".to_string()
}

fn save_updater_config(cfg: &UpdateConfig) -> anyhow::Result<()> {
    use std::path::PathBuf;

    let path = find_config_file()
        .unwrap_or_else(|| PathBuf::from("config").join("updater.toml"));

    // global.toml이면 [updater] 섹션 업데이트
    let is_global = path.file_name().map(|f| f == "global.toml").unwrap_or(false);

    if is_global {
        let content = std::fs::read_to_string(&path).unwrap_or_default();
        let mut doc: toml::value::Table = content.parse::<toml::Value>()
            .unwrap_or(toml::Value::Table(toml::value::Table::new()))
            .try_into()
            .unwrap_or_default();

        let updater_val = toml::Value::try_from(cfg)
            .map_err(|e| anyhow::anyhow!("Serialize error: {}", e))?;
        doc.insert("updater".to_string(), updater_val);

        let out = toml::to_string_pretty(&doc)?;
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent)?;
        }
        std::fs::write(&path, out)?;
    } else {
        let out = toml::to_string_pretty(cfg)
            .map_err(|e| anyhow::anyhow!("Serialize error: {}", e))?;
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent)?;
        }
        std::fs::write(&path, out)?;
    }

    Ok(())
}
