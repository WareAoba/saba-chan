//! Saba-chan Installer — 설치/제거 GUI (Tauri)
//!
//! ## 실행 모드
//! - `saba-chan-installer`                     → GUI 설치 모드 (5-page wizard)
//! - `saba-chan-installer --uninstall`          → GUI 제거 모드
//! - `saba-chan-installer --uninstall --silent` → 사일런트 제거 (GUI 없음)
//! - `saba-chan-installer --elevate-mkdir <dir> --grant-sid <sid>` → UAC 상승 디렉토리 생성 (내부용)

pub mod elevate;
#[cfg(target_os = "windows")]
pub mod registry;
pub mod runtime_bootstrap;
mod runtime_pins;
#[cfg(target_os = "windows")]
pub mod shortcuts;
pub mod uninstall;

pub use elevate::validate_elevate_mkdir_path;

use saba_chan_updater_lib::integrity::ProgressCallback;
use saba_chan_updater_lib::manifest::ReleaseManifest;
use saba_chan_updater_lib::{constants, Component, UpdateConfig, UpdateManager};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use tauri::{AppHandle, Emitter, Manager, State};
use tokio::sync::RwLock;

/// 현재 인스톨러 바이너리의 버전 — tauri.conf.json과 동기화
const INSTALLER_VERSION: &str = env!("CARGO_PKG_VERSION");

/// 실패하면 설치 전체를 중단하는 핵심 컴포넌트
const CRITICAL_COMPONENTS: &[&str] = &["saba-core", "gui"];

// ═══════════════════════════════════════════════════════
// 타입
// ═══════════════════════════════════════════════════════

/// 인스톨러 전역 상태
#[derive(Default)]
pub struct InstallerState {
    pub install_path: String,
    pub language: String,
    pub create_desktop_shortcut: bool,
    pub create_start_menu_shortcut: bool,
    pub selected_modules: Vec<String>,
    pub progress: InstallProgress,
    /// 로드된 모듈 메타데이터 (required_extensions 해석용)
    pub module_metadata: Vec<ModuleInfo>,
    /// 한 번 받은 서명 매니페스트와 그 캐시를 가진 UpdateManager.
    /// 모듈 목록·버전 표시·설치가 같은 매니페스트를 재사용한다.
    release: Option<CachedRelease>,
}

impl InstallerState {
    fn new() -> Self {
        Self {
            install_path: default_install_path().to_string_lossy().to_string(),
            language: "en".to_string(),
            create_desktop_shortcut: true,
            create_start_menu_shortcut: true,
            ..Default::default()
        }
    }
}

/// 릴레이에서 받은 매니페스트와 그것을 캐시한 UpdateManager
struct CachedRelease {
    manager: UpdateManager,
    manifest: ReleaseManifest,
}

/// 설치 진행 상태
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct InstallProgress {
    pub step: String,
    pub message: String,
    pub percent: i32,
    pub complete: bool,
    pub error: Option<String>,
    pub installed_components: Vec<String>,
    /// 현재 다운로드 중인 파일 이름
    #[serde(skip_serializing_if = "Option::is_none")]
    pub download_file: Option<String>,
    /// 현재 다운로드 수신 바이트
    #[serde(skip_serializing_if = "Option::is_none")]
    pub download_received: Option<u64>,
    /// 현재 다운로드 전체 바이트 (Content-Length)
    #[serde(skip_serializing_if = "Option::is_none")]
    pub download_total: Option<u64>,
}

/// 사용 가능한 모듈 정보
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ModuleInfo {
    pub id: String,
    pub name: String,
    pub description: String,
    pub icon: String,
    /// 이 모듈이 요구하는 익스텐션 ID 목록 (예: ["steamcmd", "ue4-ini"])
    #[serde(default)]
    pub required_extensions: Vec<String>,
}

/// 앱 모드
#[derive(Debug, Clone, Default)]
pub struct AppMode {
    pub uninstall: bool,
    pub silent: bool,
}

type SharedState = Arc<RwLock<InstallerState>>;

// ═══════════════════════════════════════════════════════
// Tauri 커맨드 — 상태
// ═══════════════════════════════════════════════════════

#[tauri::command]
async fn get_installer_state(state: State<'_, SharedState>) -> Result<serde_json::Value, String> {
    let s = state.read().await;
    Ok(serde_json::json!({
        "install_path": s.install_path,
        "language": s.language,
        "create_desktop_shortcut": s.create_desktop_shortcut,
        "create_start_menu_shortcut": s.create_start_menu_shortcut,
        "selected_modules": s.selected_modules,
        "latest_release_tag": s.release.as_ref().map(|r| release_tag(&r.manifest)),
        "progress": s.progress,
    }))
}

#[tauri::command]
async fn set_install_path(state: State<'_, SharedState>, path: String) -> Result<(), String> {
    state.write().await.install_path = path;
    Ok(())
}

#[tauri::command]
async fn set_language(state: State<'_, SharedState>, language: String) -> Result<(), String> {
    state.write().await.language = language;
    Ok(())
}

#[tauri::command]
async fn set_shortcut_options(
    state: State<'_, SharedState>,
    desktop: bool,
    start_menu: bool,
) -> Result<(), String> {
    let mut s = state.write().await;
    s.create_desktop_shortcut = desktop;
    s.create_start_menu_shortcut = start_menu;
    Ok(())
}

#[tauri::command]
async fn set_selected_modules(
    state: State<'_, SharedState>,
    modules: Vec<String>,
) -> Result<(), String> {
    state.write().await.selected_modules = modules;
    Ok(())
}

// ═══════════════════════════════════════════════════════
// 릴레이 매니페스트 (1회 fetch 후 캐시)
// ═══════════════════════════════════════════════════════

/// 릴레이에서 서명 매니페스트를 받는다. Ed25519 서명 검증은 `UpdateManager` 내부에서 수행된다.
async fn fetch_release() -> Result<CachedRelease, String> {
    let modules_dir = constants::resolve_modules_dir();
    let mut manager = UpdateManager::new(UpdateConfig::default(), &modules_dir.to_string_lossy());
    let manifest = manager
        .fetch_remote_manifest()
        .await
        .map_err(|e| format!("Failed to fetch signed manifest from Relay: {}", e))?;
    Ok(CachedRelease { manager, manifest })
}

/// 캐시된 매니페스트를 반환하고, 없으면 받아서 캐시한다.
async fn load_manifest(state: &SharedState) -> Result<ReleaseManifest, String> {
    if let Some(release) = &state.read().await.release {
        return Ok(release.manifest.clone());
    }
    let release = fetch_release().await?;
    let manifest = release.manifest.clone();
    state.write().await.release = Some(release);
    Ok(manifest)
}

fn release_tag(manifest: &ReleaseManifest) -> String {
    format!("v{}", manifest.version)
}

// ═══════════════════════════════════════════════════════
// Tauri 커맨드 — 모듈
// ═══════════════════════════════════════════════════════

/// 하드코딩 폴백 모듈 목록 (네트워크 실패 시 사용).
/// Relay 매니페스트가 도달 불가일 때만 사용된다. 아이콘은 R2 공개 URL로 폴백.
fn fallback_modules() -> Vec<ModuleInfo> {
    vec![
        ModuleInfo {
            id: "minecraft".into(),
            name: "Minecraft".into(),
            description: "Minecraft server management with RCON support".into(),
            icon: constants::default_module_icon_url("minecraft"),
            required_extensions: Vec::new(),
        },
        ModuleInfo {
            id: "palworld".into(),
            name: "Palworld".into(),
            description: "Palworld dedicated server management via REST API".into(),
            icon: constants::default_module_icon_url("palworld"),
            required_extensions: vec!["steamcmd".into(), "ue4-ini".into()],
        },
        ModuleInfo {
            id: "zomboid".into(),
            name: "Project Zomboid".into(),
            description: "Project Zomboid dedicated server management".into(),
            icon: constants::default_module_icon_url("zomboid"),
            required_extensions: vec!["steamcmd".into()],
        },
    ]
}

#[tauri::command]
async fn get_available_modules(state: State<'_, SharedState>) -> Result<Vec<ModuleInfo>, String> {
    let modules = match load_manifest(&state).await.map(|m| modules_from_manifest(&m)) {
        Ok(modules) if !modules.is_empty() => {
            tracing::info!("Relay 매니페스트에서 {} 모듈 메타데이터 로드", modules.len());
            modules
        }
        Ok(_) => {
            tracing::warn!("Relay 매니페스트에 module-* 항목이 없음, 폴백 사용");
            fallback_modules()
        }
        Err(e) => {
            tracing::warn!("Relay 매니페스트 로드 실패: {}, 폴백 사용", e);
            fallback_modules()
        }
    };

    state.write().await.module_metadata = modules.clone();
    Ok(modules)
}

/// 매니페스트의 `module-*` 컴포넌트를 모듈 카탈로그로 변환한다 (id 순 정렬).
fn modules_from_manifest(manifest: &ReleaseManifest) -> Vec<ModuleInfo> {
    let mut modules: Vec<ModuleInfo> = manifest
        .components
        .iter()
        .filter_map(|(key, info)| {
            let id = key.strip_prefix("module-")?;
            let mut required_extensions: Vec<String> = info
                .dependencies
                .as_ref()
                .map(|deps| deps.keys().cloned().collect())
                .unwrap_or_default();
            required_extensions.sort();
            // 매니페스트에 icon_url 이 비어 있으면 R2 공개 고정 경로로 폴백
            let icon = match info.icon_url.as_deref() {
                Some(u) if !u.trim().is_empty() => u.to_string(),
                _ => constants::default_module_icon_url(id),
            };
            Some(ModuleInfo {
                id: id.to_string(),
                name: info.display_name.clone().unwrap_or_else(|| id.to_string()),
                description: info.description.clone().unwrap_or_default(),
                icon,
                required_extensions,
            })
        })
        .collect();
    modules.sort_by(|a, b| a.id.cmp(&b.id));
    modules
}

#[tauri::command]
async fn fetch_latest_release(state: State<'_, SharedState>) -> Result<serde_json::Value, String> {
    let tag = release_tag(&load_manifest(&state).await?);
    Ok(serde_json::json!({
        "tag": tag,
        "name": tag,
        "published_at": serde_json::Value::Null,
    }))
}

// ═══════════════════════════════════════════════════════
// Tauri 커맨드 — 설치 실행
// ═══════════════════════════════════════════════════════

#[tauri::command]
async fn start_install(app: AppHandle, state: State<'_, SharedState>) -> Result<(), String> {
    let config = {
        let s = state.read().await;
        InstallConfig {
            install_path: s.install_path.clone(),
            language: s.language.clone(),
            create_desktop_shortcut: s.create_desktop_shortcut,
            create_start_menu_shortcut: s.create_start_menu_shortcut,
            selected_modules: s.selected_modules.clone(),
            module_metadata: s.module_metadata.clone(),
        }
    };

    let state_clone = state.inner().clone();
    tauri::async_runtime::spawn(async move {
        do_install(app, state_clone, config).await;
    });
    Ok(())
}

/// 설치 설정 (스냅샷)
#[derive(Debug, Clone)]
struct InstallConfig {
    install_path: String,
    language: String,
    #[cfg_attr(not(target_os = "windows"), allow(dead_code))]
    create_desktop_shortcut: bool,
    #[cfg_attr(not(target_os = "windows"), allow(dead_code))]
    create_start_menu_shortcut: bool,
    selected_modules: Vec<String>,
    module_metadata: Vec<ModuleInfo>,
}

/// 설치 실행 (비동기). 진행·완료·실패를 `install:progress` 이벤트로 알린다.
async fn do_install(app: AppHandle, state: SharedState, config: InstallConfig) {
    // 매니페스트 캐시는 설치 동안 꺼내 쓰고 끝나면 돌려놓는다 (설치 중 상태 잠금 방지).
    let mut release = state.write().await.release.take();
    let result = run_install(&app, &mut release, &config).await;
    state.write().await.release = release;

    let progress = match result {
        Ok(installed) => InstallProgress {
            step: "complete".into(),
            message: format!("{} components installed!", installed.len()),
            percent: 100,
            complete: true,
            installed_components: installed,
            ..Default::default()
        },
        Err(msg) => InstallProgress {
            step: "error".into(),
            message: msg.clone(),
            error: Some(msg),
            ..Default::default()
        },
    };
    state.write().await.progress = progress.clone();
    app.emit("install:progress", &progress).ok();

    // 임시 디렉토리 정리
    let _ = std::fs::remove_dir_all(std::env::temp_dir().join("saba-chan-installer"));
}

/// 설치 단계를 순서대로 실행하고, 설치된 컴포넌트 목록을 반환한다.
async fn run_install(
    app: &AppHandle,
    release_slot: &mut Option<CachedRelease>,
    config: &InstallConfig,
) -> Result<Vec<String>, String> {
    let install_dir = PathBuf::from(&config.install_path);

    emit_progress(app, "prepare", "Creating install directory...", 2);
    prepare_install_dir(app, &install_dir)?;

    emit_progress(app, "fetch", "Fetching signed release manifest...", 7);
    let release = match release_slot.take() {
        Some(release) => release,
        None => fetch_release().await?,
    };
    let release = release_slot.insert(release);
    check_installer_version(&release.manifest)?;

    // 다운로드 대상 루트를 사용자가 고른 설치 경로로 바꾼다 (캐시된 매니페스트는 유지).
    let mut updater_config = release.manager.get_config();
    updater_config.install_root = Some(config.install_path.clone());
    release.manager.update_config(updater_config);

    let mut installed = install_core_components(app, release).await?;

    emit_progress(app, "modules-shared", "Installing shared module utilities...", 46);
    install_shared_modules(&install_dir);

    if !config.selected_modules.is_empty() {
        installed.extend(install_modules(app, &mut release.manager, &config.selected_modules).await);
        emit_progress(app, "extensions", "Checking required extensions...", 54);
        let required = collect_required_extensions(&config.selected_modules, &config.module_metadata);
        installed.extend(install_extensions(app, &mut release.manager, &required).await);
    }

    installed.extend(install_runtimes(app, &install_dir).await);

    emit_progress(app, "config", "Saving language settings...", 88);
    let _ = std::fs::create_dir_all(install_dir.join("locales"));
    save_language_setting(&config.language);

    #[cfg(target_os = "windows")]
    register_app(app, &install_dir, &release_tag(&release.manifest), config);

    emit_progress(app, "manifest", "Recording installed versions...", 98);
    record_installed_manifest(&release.manifest, &installed);

    Ok(installed)
}

/// 설치 디렉토리를 만들고 쓰기 권한을 확인한다. 권한이 없을 때만 UAC 상승을 요청한다.
fn prepare_install_dir(app: &AppHandle, install_dir: &Path) -> Result<(), String> {
    if elevate::install_dir_needs_elevation(install_dir)? {
        emit_progress(app, "elevate", "Requesting administrator privileges...", 1);
        elevate::elevate_create_dir(install_dir)
            .map_err(|e| format!("Failed to create directory: {}", e))?;
    }
    Ok(())
}

/// 매니페스트가 요구하는 인스톨러 버전보다 낮으면 설치를 막는다.
fn check_installer_version(manifest: &ReleaseManifest) -> Result<(), String> {
    let Some(installer_info) = manifest.components.get("installer") else {
        return Ok(());
    };
    let required = &installer_info.version;
    if version_compare(INSTALLER_VERSION, required) == std::cmp::Ordering::Less {
        return Err(format!(
            "This installer (v{}) is outdated. Version v{} or later is required. \
             Please download the latest installer.",
            INSTALLER_VERSION, required
        ));
    }
    tracing::info!("인스톨러 버전 확인 OK: current={}, required={}", INSTALLER_VERSION, required);
    Ok(())
}

/// 모듈·익스텐션·인스톨러 자신을 제외한 코어 컴포넌트 키 (정렬)
fn core_component_keys(manifest: &ReleaseManifest) -> Vec<String> {
    let mut keys: Vec<String> = manifest
        .components
        .keys()
        .filter(|k| k.as_str() != "installer" && !k.starts_with("module-") && !k.starts_with("ext-"))
        .cloned()
        .collect();
    keys.sort();
    keys
}

/// 코어 컴포넌트 설치 (10-45%). 핵심 컴포넌트 실패 시 설치를 중단한다.
async fn install_core_components(
    app: &AppHandle,
    release: &mut CachedRelease,
) -> Result<Vec<String>, String> {
    let keys = core_component_keys(&release.manifest);
    let total = keys.len().max(1);
    let mut installed = Vec::new();
    for (i, key) in keys.iter().enumerate() {
        let pct = 10 + (i * 35 / total) as i32;
        let component = Component::from_manifest_key(key);
        match install_component(app, &mut release.manager, &component, "download", key, pct).await {
            Ok(()) => installed.push(key.clone()),
            Err(e) if CRITICAL_COMPONENTS.contains(&key.as_str()) => {
                return Err(format!("Critical component '{}' install failed: {}", key, e));
            }
            Err(e) => tracing::error!("Component {} install failed: {}", key, e),
        }
    }
    Ok(installed)
}

/// 선택한 게임 모듈 설치 (47-52%). 실패한 모듈은 건너뛴다.
async fn install_modules(app: &AppHandle, manager: &mut UpdateManager, module_ids: &[String]) -> Vec<String> {
    let total = module_ids.len().max(1);
    let mut installed = Vec::new();
    for (i, id) in module_ids.iter().enumerate() {
        let pct = 47 + (i * 5 / total) as i32;
        let component = Component::Module(id.clone());
        let label = format!("module {}", id);
        match install_component(app, manager, &component, "modules-download", &label, pct).await {
            Ok(()) => installed.push(component.manifest_key()),
            Err(e) => tracing::warn!("Failed to install module {}: {}", id, e),
        }
    }
    installed
}

/// 선택한 모듈이 요구하는 익스텐션 설치 (54%). 실패한 익스텐션은 건너뛴다.
async fn install_extensions(app: &AppHandle, manager: &mut UpdateManager, ext_ids: &[String]) -> Vec<String> {
    if !ext_ids.is_empty() {
        tracing::info!("선택된 모듈에 필요한 익스텐션: {:?}", ext_ids);
    }
    let mut installed = Vec::new();
    for id in ext_ids {
        let component = Component::Extension(id.clone());
        let label = format!("extension {}", id);
        match install_component(app, manager, &component, "extensions", &label, 54).await {
            Ok(()) => installed.push(component.manifest_key()),
            Err(e) => tracing::warn!("익스텐션 '{}' 설치 실패: {}", id, e),
        }
    }
    installed
}

/// 컴포넌트 하나를 릴레이 + R2 경유로 설치한다 (SHA256 + Ed25519 검증, 덮어쓰기).
async fn install_component(
    app: &AppHandle,
    manager: &mut UpdateManager,
    component: &Component,
    step: &'static str,
    label: &str,
    percent: i32,
) -> Result<(), String> {
    emit_progress(app, step, &format!("Installing {}...", label), percent);
    let progress = download_progress_callback(app, step, label, component, percent);
    let path = manager
        .reinstall_component_with_progress(component, Some(progress))
        .await
        .map_err(|e| e.to_string())?;
    tracing::info!("[Installer] {} → {}", component.manifest_key(), path);
    Ok(())
}

/// 다운로드 바이트 진행률을 프론트엔드로 보내는 콜백
fn download_progress_callback(
    app: &AppHandle,
    step: &'static str,
    label: &str,
    component: &Component,
    percent: i32,
) -> ProgressCallback {
    let app = app.clone();
    let label = label.to_string();
    let file = format!("{}.zip", component.manifest_key());
    Box::new(move |received, total| {
        let p = InstallProgress {
            step: step.to_string(),
            message: download_message(&label, received, total),
            percent,
            download_file: Some(file.clone()),
            download_received: Some(received),
            download_total: (total > 0).then_some(total),
            ..Default::default()
        };
        app.emit("install:progress", &p).ok();
    })
}

fn download_message(label: &str, received: u64, total: u64) -> String {
    const MB: f64 = 1_048_576.0;
    let mb_received = received as f64 / MB;
    if total > 0 {
        let pct = (received as f64 / total as f64 * 100.0) as i32;
        format!(
            "Downloading {} — {:.1}/{:.1} MB ({}%)",
            label,
            mb_received,
            total as f64 / MB,
            pct
        )
    } else {
        format!("Downloading {} — {:.1} MB", label, mb_received)
    }
}

/// 포터블 Python(57%)·Node.js(72%) 준비. 실패는 치명적이지 않다 — 메인 앱이 첫 실행 시 재시도.
async fn install_runtimes(app: &AppHandle, install_dir: &Path) -> Vec<String> {
    let runtime_data_dir = runtime_bootstrap::resolve_runtime_data_dir(install_dir);
    let _ = std::fs::create_dir_all(&runtime_data_dir);
    let mut installed = Vec::new();

    emit_progress(app, "runtime-python", "Preparing Python runtime...", 57);
    match runtime_bootstrap::setup_python(&runtime_data_dir).await {
        Ok(path) => {
            tracing::info!("Python 환경 준비 완료: {}", path.display());
            installed.push("runtime:python".to_string());
        }
        Err(e) => tracing::warn!("Python 환경 설정 실패 (비치명적): {}", e),
    }

    emit_progress(app, "runtime-node", "Preparing Node.js runtime...", 72);
    match runtime_bootstrap::setup_node(&runtime_data_dir).await {
        Ok(path) => {
            tracing::info!("Node.js 환경 준비 완료: {}", path.display());
            installed.push("runtime:nodejs".to_string());
        }
        Err(e) => tracing::warn!("Node.js 환경 설정 실패 (비치명적): {}", e),
    }
    installed
}

/// 레지스트리 등록 (92%) + 바로가기 생성 (96%)
#[cfg(target_os = "windows")]
fn register_app(app: &AppHandle, install_dir: &Path, tag: &str, config: &InstallConfig) {
    emit_progress(app, "registry", "Registering application...", 92);
    if let Err(e) = registry::register_uninstall_entry(install_dir, tag) {
        tracing::warn!("Failed to create registry entry: {}", e);
    }

    emit_progress(app, "shortcuts", "Creating shortcuts...", 96);
    // 기존 바로가기 제거 (언어 변경 시 이전 이름의 잔재 방지)
    for old_name in shortcuts::ALL_SHORTCUT_NAMES {
        let _ = shortcuts::remove_desktop_shortcut(old_name);
        let _ = shortcuts::remove_start_menu_shortcut(old_name);
    }
    let gui_exe = install_dir.join("saba-chan-gui.exe");
    if !gui_exe.exists() {
        return;
    }
    let app_name = shortcuts::localized_app_name(&config.language);
    if config.create_desktop_shortcut {
        if let Err(e) = shortcuts::create_desktop_shortcut(&gui_exe, app_name) {
            tracing::warn!("Desktop shortcut failed: {}", e);
        }
    }
    if config.create_start_menu_shortcut {
        if let Err(e) = shortcuts::create_start_menu_shortcut(&gui_exe, app_name) {
            tracing::warn!("Start menu shortcut failed: {}", e);
        }
    }
}

/// 이번에 실제로 설치된 컴포넌트의 매니페스트 버전 맵
fn installed_versions(manifest: &ReleaseManifest, installed: &[String]) -> HashMap<String, String> {
    installed
        .iter()
        .filter_map(|key| {
            manifest
                .components
                .get(key)
                .map(|info| (key.clone(), info.version.clone()))
        })
        .collect()
}

/// installed-manifest.json 갱신 (업데이터가 현재 버전을 인식하도록).
/// 설치에 성공한 컴포넌트만 기존 기록 위에 덮어쓴다.
fn record_installed_manifest(manifest: &ReleaseManifest, installed: &[String]) {
    let mut versions = UpdateManager::load_installed_manifest();
    versions.extend(installed_versions(manifest, installed));
    let path = constants::resolve_installed_manifest_path();
    if let Some(parent) = path.parent() {
        let _ = std::fs::create_dir_all(parent);
    }
    let result = serde_json::to_string_pretty(&versions)
        .map_err(|e| e.to_string())
        .and_then(|json| std::fs::write(&path, json).map_err(|e| e.to_string()));
    match result {
        Ok(()) => tracing::info!("Written {:?} with {} components", path, versions.len()),
        Err(e) => tracing::warn!("Failed to write installed-manifest.json: {}", e),
    }
}

fn emit_progress(app: &AppHandle, step: &str, message: &str, percent: i32) {
    let p = InstallProgress {
        step: step.to_string(),
        message: message.to_string(),
        percent,
        ..Default::default()
    };
    app.emit("install:progress", &p).ok();
}

// ═══════════════════════════════════════════════════════
// Tauri 커맨드 — 제거
// ═══════════════════════════════════════════════════════

#[tauri::command]
async fn start_uninstall(app: AppHandle, keep_settings: Option<bool>) -> Result<(), String> {
    let preserve = keep_settings.unwrap_or(false);
    tauri::async_runtime::spawn(async move {
        uninstall::do_uninstall(&app, preserve).await;
    });
    Ok(())
}

#[tauri::command]
async fn get_app_mode(mode: State<'_, AppMode>) -> Result<serde_json::Value, String> {
    Ok(serde_json::json!({
        "uninstall": mode.uninstall,
        "silent": mode.silent,
    }))
}

#[tauri::command]
async fn launch_app(state: State<'_, SharedState>) -> Result<(), String> {
    let install_path = state.read().await.install_path.clone();
    let exe = PathBuf::from(&install_path)
        .join(if cfg!(windows) { "saba-chan-gui.exe" } else { "saba-chan-gui" });
    if !exe.exists() {
        return Err(format!("Executable not found: {}", exe.display()));
    }
    std::process::Command::new(&exe)
        .current_dir(&install_path)
        .spawn()
        .map_err(|e| format!("Failed to launch Saba-chan: {}", e))?;
    Ok(())
}

// ═══════════════════════════════════════════════════════
// Tauri 커맨드 — 언어
// ═══════════════════════════════════════════════════════

#[tauri::command]
async fn get_preferred_language() -> Result<String, String> {
    if let Some(lang) = load_main_app_language() {
        if let Some(normalized) = normalize_language_tag(&lang) {
            return Ok(normalized);
        }
    }
    if let Some(locale) = sys_locale::get_locale() {
        if let Some(normalized) = normalize_language_tag(&locale) {
            return Ok(normalized);
        }
    }
    Ok("en".to_string())
}

#[tauri::command]
async fn browse_folder(app: AppHandle) -> Result<Option<String>, String> {
    use tauri_plugin_dialog::DialogExt;
    let path = app
        .dialog()
        .file()
        .set_title("Select install directory")
        .blocking_pick_folder();
    Ok(path.map(|p| p.to_string()))
}

// ═══════════════════════════════════════════════════════
// 헬퍼 함수
// ═══════════════════════════════════════════════════════

/// 기본 설치 경로. 설치와 제거 폴백이 같은 값을 쓴다.
pub(crate) fn default_install_path() -> PathBuf {
    #[cfg(target_os = "windows")]
    {
        PathBuf::from(r"C:\Program Files\Saba-chan")
    }
    #[cfg(not(target_os = "windows"))]
    {
        match std::env::var("HOME") {
            Ok(home) => PathBuf::from(home).join(".local").join("share").join("saba-chan"),
            Err(_) => PathBuf::from("saba-chan"),
        }
    }
}

fn load_main_app_language() -> Option<String> {
    let content = std::fs::read_to_string(constants::resolve_settings_path()).ok()?;
    let val: serde_json::Value = serde_json::from_str(&content).ok()?;
    val.get("language")
        .and_then(|v| v.as_str())
        .map(|s| s.to_string())
}

fn normalize_language_tag(input: &str) -> Option<String> {
    let result = constants::resolve_locale(input);
    if result == "en" && !input.trim().is_empty() {
        // resolve_locale always returns a value; check if input was explicitly "en"
        let trimmed = input.trim();
        let canonical = trimmed.replace('_', "-");
        if canonical.eq_ignore_ascii_case("en") || canonical.to_lowercase().starts_with("en") {
            return Some("en".to_string());
        }
        // If resolve_locale fell back to "en" for an unknown input, return None
        // to let the caller decide
        for supported in constants::SUPPORTED_LANGUAGES {
            if supported.eq_ignore_ascii_case(&canonical) {
                return Some(supported.to_string());
            }
        }
        let base = canonical.to_lowercase();
        let base = base.split('-').next().unwrap_or("");
        if ["ko", "ja", "zh", "es", "pt", "ru", "de", "fr"].contains(&base) {
            return Some(result);
        }
        return None;
    }
    Some(result)
}

/// core zip에 포함된 공유 Python 모듈(i18n.py, daemon_rcon.py)을
/// %APPDATA%/saba-chan/modules/로 복사한다.
/// 게임 모듈(minecraft, palworld 등)의 lifecycle.py가 이 유틸리티를 import 한다.
fn install_shared_modules(install_dir: &Path) {
    const SHARED_MODULES: &[&str] = &["i18n.py", "daemon_rcon.py"];

    let modules_dir = constants::resolve_data_dir().join("modules");
    let _ = std::fs::create_dir_all(&modules_dir);

    for name in SHARED_MODULES {
        let src = install_dir.join(name);
        let dst = modules_dir.join(name);
        if !src.exists() {
            tracing::debug!("Shared module not found in zip: {}", name);
            continue;
        }
        match std::fs::copy(&src, &dst) {
            Ok(_) => {
                tracing::info!("Shared module installed: {} → {}", src.display(), dst.display());
                // install_dir에 남아 있는 복사본 제거 (Program Files에 불필요)
                let _ = std::fs::remove_file(&src);
            }
            Err(e) => tracing::warn!("Failed to install shared module {}: {}", name, e),
        }
    }
}

/// settings.json 에 language 만 머지해서 저장한다 (다른 키 보존).
fn save_language_setting(language: &str) {
    let settings_path = constants::resolve_settings_path();
    if let Some(parent) = settings_path.parent() {
        let _ = std::fs::create_dir_all(parent);
    }

    let mut settings = std::fs::read_to_string(&settings_path)
        .ok()
        .and_then(|content| serde_json::from_str::<serde_json::Value>(&content).ok())
        .filter(|v| v.is_object())
        .unwrap_or_else(|| serde_json::json!({}));
    settings["language"] = serde_json::Value::String(language.to_string());

    let _ = std::fs::write(
        &settings_path,
        serde_json::to_string_pretty(&settings).unwrap_or_default(),
    );
}

/// semver 문자열 비교 (major.minor.patch)
fn version_compare(a: &str, b: &str) -> std::cmp::Ordering {
    let parse = |s: &str| -> Vec<u64> {
        s.trim_start_matches('v')
            .split('.')
            .filter_map(|p| p.parse::<u64>().ok())
            .collect()
    };
    let va = parse(a);
    let vb = parse(b);
    let max_len = va.len().max(vb.len());
    for i in 0..max_len {
        let a_part = va.get(i).copied().unwrap_or(0);
        let b_part = vb.get(i).copied().unwrap_or(0);
        match a_part.cmp(&b_part) {
            std::cmp::Ordering::Equal => continue,
            other => return other,
        }
    }
    std::cmp::Ordering::Equal
}

/// 선택된 모듈의 required_extensions를 수집 (중복 제거)
fn collect_required_extensions(selected_modules: &[String], module_metadata: &[ModuleInfo]) -> Vec<String> {
    let mut extensions: Vec<String> = Vec::new();
    for module_id in selected_modules {
        if let Some(meta) = module_metadata.iter().find(|m| m.id == *module_id) {
            for ext in &meta.required_extensions {
                if !extensions.contains(ext) {
                    extensions.push(ext.clone());
                }
            }
        }
    }
    extensions
}

/// 설치 디렉토리가 saba-chan 설치물인지 시그니처 파일로 검증한다.
///
/// 레지스트리 `InstallLocation`가 공격자에 의해 변조되어 임의 디렉토리를 삭제하지 못하도록,
/// 최소 한 개의 saba-chan 바이너리 파일이 존재할 때만 `true`를 반환한다.
pub fn verify_install_dir_marker(dir: &Path) -> bool {
    if !dir.is_dir() {
        return false;
    }
    const MARKERS: &[&str] = &[
        "saba-chan-gui.exe",
        "saba-core.exe",
        "saba-chan-updater.exe",
        "saba-chan-cli.exe",
        "saba-chan-gui",
        "saba-core",
        "saba-chan-updater",
        "saba-chan-cli",
    ];
    MARKERS.iter().any(|marker| {
        std::fs::symlink_metadata(dir.join(marker))
            .map(|metadata| metadata.file_type().is_file())
            .unwrap_or(false)
    })
}

// ═══════════════════════════════════════════════════════
// 엔트리
// ═══════════════════════════════════════════════════════

fn init_tracing(to_stderr: bool) {
    let builder = tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| tracing_subscriber::EnvFilter::new("info")),
        )
        .with_target(false);
    if to_stderr {
        builder.with_writer(std::io::stderr).init();
    } else {
        builder.init();
    }
}

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    let args: Vec<String> = std::env::args().collect();

    // elevate_create_dir()가 ShellExecuteExW("runas")로 자기 자신을 재실행할 때 진입한다.
    if args.iter().any(|a| a == elevate::ELEVATE_MKDIR_ARG) {
        std::process::exit(elevate::run_elevated_mkdir(&args));
    }

    let is_uninstall = args.iter().any(|a| a == "--uninstall");
    let is_silent = args.iter().any(|a| a == "--silent");

    // --uninstall --silent → 사일런트 제거 (GUI 없이)
    if is_uninstall && is_silent {
        #[cfg(target_os = "windows")]
        {
            use windows_sys::Win32::System::Console::{AllocConsole, AttachConsole, ATTACH_PARENT_PROCESS};
            unsafe {
                if AttachConsole(ATTACH_PARENT_PROCESS) == 0 {
                    let _ = AllocConsole();
                }
            }
        }
        init_tracing(true);
        let rt = tokio::runtime::Runtime::new().expect("Failed to create tokio runtime");
        rt.block_on(uninstall::do_silent_uninstall());
        return;
    }

    // GUI 모드
    init_tracing(false);

    let app_mode = AppMode {
        uninstall: is_uninstall,
        silent: is_silent,
    };
    let installer_state: SharedState = Arc::new(RwLock::new(InstallerState::new()));

    tauri::Builder::default()
        .plugin(tauri_plugin_shell::init())
        .plugin(tauri_plugin_dialog::init())
        .manage(installer_state)
        .manage(app_mode)
        .setup(move |app| {
            if is_uninstall {
                if let Some(win) = app.get_webview_window("main") {
                    win.set_title("Saba-chan — Uninstaller").ok();
                }
            }
            Ok(())
        })
        .invoke_handler(tauri::generate_handler![
            get_installer_state,
            set_install_path,
            set_language,
            set_shortcut_options,
            set_selected_modules,
            get_available_modules,
            fetch_latest_release,
            start_install,
            start_uninstall,
            get_app_mode,
            get_preferred_language,
            browse_folder,
            launch_app,
        ])
        .run(tauri::generate_context!())
        .expect("error while running tauri application");
}

#[cfg(test)]
mod tests {
    use super::*;

    fn manifest(json: serde_json::Value) -> ReleaseManifest {
        serde_json::from_value(json).unwrap()
    }

    fn sample_manifest() -> ReleaseManifest {
        manifest(serde_json::json!({
            "version": "1.2.0",
            "components": {
                "saba-core": { "version": "1.2.0" },
                "gui": { "version": "1.2.1" },
                "installer": { "version": "0.1.0" },
                "module-palworld": {
                    "version": "2.0.0",
                    "display_name": "Palworld",
                    "dependencies": { "ue4-ini": "*", "steamcmd": "*" }
                },
                "module-minecraft": { "version": "3.0.0" },
                "ext-steamcmd": { "version": "0.5.0" }
            }
        }))
    }

    #[test]
    fn installed_manifest_records_only_installed_components() {
        let m = sample_manifest();
        let installed = vec![
            "saba-core".to_string(),
            "module-palworld".to_string(),
            "runtime:python".to_string(),
        ];
        let versions = installed_versions(&m, &installed);
        assert_eq!(versions.len(), 2);
        assert_eq!(versions["saba-core"], "1.2.0");
        assert_eq!(versions["module-palworld"], "2.0.0");
        // 실패한 gui, 선택하지 않은 module-minecraft, 설치하지 않은 ext 는 기록하지 않는다.
        assert!(!versions.contains_key("gui"));
        assert!(!versions.contains_key("module-minecraft"));
        assert!(!versions.contains_key("ext-steamcmd"));
        assert!(!versions.contains_key("installer"));
    }

    #[test]
    fn core_keys_exclude_installer_modules_and_extensions() {
        assert_eq!(core_component_keys(&sample_manifest()), vec!["gui", "saba-core"]);
    }

    #[test]
    fn modules_come_from_module_keys_sorted() {
        let modules = modules_from_manifest(&sample_manifest());
        let ids: Vec<&str> = modules.iter().map(|m| m.id.as_str()).collect();
        assert_eq!(ids, vec!["minecraft", "palworld"]);
        assert_eq!(modules[1].name, "Palworld");
        assert_eq!(modules[1].required_extensions, vec!["steamcmd", "ue4-ini"]);
        assert_eq!(modules[0].name, "minecraft");
    }

    #[test]
    fn installer_version_gate() {
        assert!(check_installer_version(&sample_manifest()).is_ok());
        let newer = manifest(serde_json::json!({
            "version": "9.0.0",
            "components": { "installer": { "version": "999.0.0" } }
        }));
        assert!(check_installer_version(&newer).unwrap_err().contains("outdated"));
    }

    #[test]
    fn required_extensions_are_deduplicated() {
        let meta = fallback_modules();
        let selected = vec!["palworld".to_string(), "zomboid".to_string(), "unknown".to_string()];
        assert_eq!(collect_required_extensions(&selected, &meta), vec!["steamcmd", "ue4-ini"]);
    }

    #[test]
    fn install_dir_marker_ignores_unwritten_saba_install_file() {
        // `.saba-install` 은 아무도 쓰지 않으므로 마커로 인정하지 않는다.
        let tmp = tempfile::tempdir().unwrap();
        std::fs::write(tmp.path().join(".saba-install"), b"").unwrap();
        assert!(!verify_install_dir_marker(tmp.path()));
    }

    #[test]
    fn download_message_formats() {
        assert_eq!(download_message("gui", 1_048_576, 0), "Downloading gui — 1.0 MB");
        assert_eq!(
            download_message("gui", 1_048_576, 2_097_152),
            "Downloading gui — 1.0/2.0 MB (50%)"
        );
    }
}
