//! Saba-chan Installer — 설치/제거 GUI (Tauri)
//!
//! ## 실행 모드
//! - `saba-chan-installer`                     → GUI 설치 모드 (5-page wizard)
//! - `saba-chan-installer --uninstall`          → GUI 제거 모드
//! - `saba-chan-installer --uninstall --silent` → 사일런트 제거 (GUI 없음)

#[cfg(target_os = "windows")]
pub mod registry;
pub mod release;
pub mod runtime_bootstrap;
#[cfg(target_os = "windows")]
pub mod shortcuts;
pub mod uninstall;

use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};
use std::sync::Arc;
use tauri::{AppHandle, Emitter, Manager, State};
use tokio::sync::RwLock;
use saba_chan_updater_lib::config_file::ConfigFile;
use saba_chan_updater_lib::constants;
use saba_chan_updater_lib::relay::{self, ReleaseManifest};
use saba_chan_updater_lib::UpdateManager;

/// 현재 인스톨러 바이너리의 버전 — tauri.conf.json과 동기화
const INSTALLER_VERSION: &str = env!("CARGO_PKG_VERSION");

// ═══════════════════════════════════════════════════════
// 타입
// ═══════════════════════════════════════════════════════

/// 인스톨러 전역 상태
#[derive(Debug)]
pub struct InstallerState {
    pub install_path: String,
    pub language: String,
    pub create_desktop_shortcut: bool,
    pub create_start_menu_shortcut: bool,
    pub selected_modules: Vec<String>,
    /// fetch_latest_release가 받은 검증된 manifest
    pub release: Option<ReleaseManifest>,
    pub progress: InstallProgress,
    /// 로드된 모듈 메타데이터 (required_extensions 해석용)
    pub module_metadata: Vec<ModuleInfo>,
}

impl Default for InstallerState {
    fn default() -> Self {
        Self {
            install_path: get_default_install_path(),
            language: "en".to_string(),
            create_desktop_shortcut: true,
            create_start_menu_shortcut: true,
            selected_modules: Vec::new(),
            release: None,
            progress: InstallProgress::default(),
            module_metadata: Vec::new(),
        }
    }
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
    /// 설치는 끝났지만 실패한 단계 (완료 때만)
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub warnings: Vec<String>,
    /// 설치 로그 경로 (경고가 있을 때만)
    #[serde(skip_serializing_if = "Option::is_none")]
    pub log_path: Option<String>,
    /// 실패 종류 (화면이 번역 문구를 고른다. `message`는 원문 그대로 함께 보인다)
    #[serde(skip_serializing_if = "Option::is_none")]
    pub error_code: Option<String>,
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
        "latest_release_tag": s.release.as_ref().map(|r| r.version.clone()),
        "progress": s.progress,
    }))
}

#[tauri::command]
async fn set_install_path(state: State<'_, SharedState>, path: String) -> Result<String, String> {
    let path = dedicated_install_dir(Path::new(&path)).to_string_lossy().into_owned();
    state.write().await.install_path = path.clone();
    Ok(path)
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
// Tauri 커맨드 — 모듈
// ═══════════════════════════════════════════════════════

#[tauri::command]
async fn get_available_modules(state: State<'_, SharedState>) -> Result<Vec<ModuleInfo>, String> {
    // 목록은 최신 릴리즈 manifest의 `module-*` 컴포넌트다 (U9e). 실패하면 화면이 알리고, 모듈은 설치 뒤 앱에서 받을 수 있다.
    let cached = state.read().await.release.clone();
    let manifest = match cached {
        Some(m) => m,
        None => {
            let m = release::fetch_latest().await?;
            state.write().await.release = Some(m.clone());
            m
        }
    };
    let modules = release::modules(&manifest);
    if modules.is_empty() {
        return Err("No modules are published in the latest release".into());
    }
    // required_extensions 자동 설치에 쓴다
    state.write().await.module_metadata = modules.clone();
    Ok(modules)
}

// ═══════════════════════════════════════════════════════
// Tauri 커맨드 — 릴리즈 (relay 서명 manifest, U10)
// ═══════════════════════════════════════════════════════

#[tauri::command]
async fn fetch_latest_release(
    state: State<'_, SharedState>,
) -> Result<serde_json::Value, String> {
    let manifest = release::fetch_latest().await?;
    let info = serde_json::json!({
        "tag": manifest.version,
        "name": manifest.version,
        "published_at": manifest.released_at,
    });
    state.write().await.release = Some(manifest);
    Ok(info)
}

// ═══════════════════════════════════════════════════════
// Tauri 커맨드 — 설치 실행
// ═══════════════════════════════════════════════════════

/// 설치와 제거는 한 번에 하나만 돈다 (두 번 누르면 같은 임시 폴더·설치 폴더를 함께 쓴다)
static WORK_BUSY: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);

struct WorkGuard;

impl WorkGuard {
    fn try_begin() -> Option<WorkGuard> {
        use std::sync::atomic::Ordering;
        WORK_BUSY
            .compare_exchange(false, true, Ordering::AcqRel, Ordering::Acquire)
            .ok()
            .map(|_| WorkGuard)
    }
}

impl Drop for WorkGuard {
    fn drop(&mut self) {
        WORK_BUSY.store(false, std::sync::atomic::Ordering::Release);
    }
}

#[tauri::command]
async fn start_install(
    app: AppHandle,
    state: State<'_, SharedState>,
) -> Result<(), String> {
    let guard = WorkGuard::try_begin().ok_or_else(|| "busy".to_string())?;
    let config = {
        let s = state.read().await;
        InstallConfig {
            install_path: s.install_path.clone(),
            language: s.language.clone(),
            create_desktop_shortcut: s.create_desktop_shortcut,
            create_start_menu_shortcut: s.create_start_menu_shortcut,
            selected_modules: s.selected_modules.clone(),
            release: s.release.clone(),
            module_metadata: s.module_metadata.clone(),
        }
    };

    let state_clone = state.inner().clone();

    tauri::async_runtime::spawn(async move {
        let _guard = guard;
        do_install(app, state_clone, config).await;
    });

    Ok(())
}

/// 설치 설정 (스냅샷)
#[derive(Debug, Clone)]
struct InstallConfig {
    install_path: String,
    language: String,
    // 바로가기는 Windows에서만 만든다
    #[cfg_attr(not(windows), allow(dead_code))]
    create_desktop_shortcut: bool,
    #[cfg_attr(not(windows), allow(dead_code))]
    create_start_menu_shortcut: bool,
    selected_modules: Vec<String>,
    release: Option<ReleaseManifest>,
    module_metadata: Vec<ModuleInfo>,
}

/// 설치 실행 (비동기)
async fn do_install(app: AppHandle, state: SharedState, config: InstallConfig) {
    let emit = |step: &str, msg: &str, pct: i32| {
        let p = InstallProgress {
            step: step.to_string(),
            message: msg.to_string(),
            percent: pct,
            complete: false,
            error: None,
            installed_components: Vec::new(),
            download_file: None,
            download_received: None,
            download_total: None,
            warnings: Vec::new(),
            log_path: None,
            error_code: None,
        };
        app.emit("install:progress", &p).ok();
    };

    // 다운로드 진행률을 포함한 emit (Fix 4)
    let _emit_download = |step: &str, msg: &str, pct: i32, file: &str, received: u64, total: Option<u64>| {
        let p = InstallProgress {
            step: step.to_string(),
            message: msg.to_string(),
            percent: pct,
            complete: false,
            error: None,
            installed_components: Vec::new(),
            download_file: Some(file.to_string()),
            download_received: Some(received),
            download_total: total,
            warnings: Vec::new(),
            log_path: None,
            error_code: None,
        };
        app.emit("install:progress", &p).ok();
    };

    // 설치는 계속하되 사용자가 알아야 하는 실패 (로그 파일과 완료 화면에 남는다)
    let warnings = std::sync::Mutex::new(Vec::<String>::new());
    let warn = |msg: String| {
        tracing::warn!("{}", msg);
        if let Ok(mut w) = warnings.lock() {
            w.push(msg);
        }
    };

    // Step 1: 설치 디렉토리 생성 + 쓰기 권한 확인 (0-5%)
    emit("prepare", "Creating install directory...", 2);
    let install_dir = PathBuf::from(&config.install_path);
    // 설치기가 새로 만든 폴더만 전용 폴더로 표시한다 (제거기가 폴더째 지워도 되는 곳).
    // 원래 있던 빈 폴더(드라이브 루트, 문서 폴더 등)는 나중에 다른 파일이 생길 수 있어 표시하지 않는다
    let dedicated = !install_dir.is_dir() || install_dir.join(uninstall::INSTALL_MARKER).is_file();
    let _ = std::fs::create_dir_all(&install_dir); // 이미 있어도 OK

    // 실제 쓰기 가능 여부 테스트
    let needs_elevation = {
        let probe = install_dir.join(".saba-write-test");
        match std::fs::write(&probe, b"test") {
            Ok(_) => { let _ = std::fs::remove_file(&probe); false }
            Err(e) if e.kind() == std::io::ErrorKind::PermissionDenied => true,
            Err(_) => {
                // 디렉토리 자체가 없는 경우 (create_dir_all도 실패)
                !install_dir.exists()
            }
        }
    };

    if needs_elevation {
        emit("elevate", "Requesting administrator privileges...", 1);
        if let Err(elev_err) = elevate_create_dir(&install_dir) {
            emit_error(
                &app,
                &state,
                "elevation_failed",
                &format!("Failed to create directory: {}", elev_err),
            )
            .await;
            return;
        }
    }

    // 폴더를 만든 바로 뒤에 표시한다: 내려받다 실패한 뒤 다시 시도해도 전용 폴더로 남는다
    if dedicated {
        if let Err(e) = std::fs::write(install_dir.join(uninstall::INSTALL_MARKER), b"") {
            warn(format!("Failed to mark the install folder: {}", e));
        }
    }

    // Step 2: 서명된 릴리즈 manifest (5-10%)
    emit("fetch", "Fetching release information...", 7);
    let manifest = match config.release.clone() {
        Some(m) => m,
        None => match release::fetch_latest().await {
            Ok(m) => m,
            Err(e) => {
                emit_error(&app, &state, "fetch_failed", &e).await;
                return;
            }
        },
    };

    // 설치기 자체가 이 릴리즈를 설치할 수 있는 버전인지
    if let Some(required_version) = release::required_installer_version(&manifest) {
        if version_compare(INSTALLER_VERSION, required_version) == std::cmp::Ordering::Less {
            emit_error(
                &app,
                &state,
                "installer_outdated",
                &format!(
                    "This installer (v{}) is outdated. Version v{} or later is required. \
                     Please download the latest installer.",
                    INSTALLER_VERSION, required_version
                ),
            )
            .await;
            return;
        }
    }

    // Step 3: R2에서 받고 SHA-256 확인 후 압축 해제 (10-45%)
    // 하나라도 실패하면 설치를 멈춘다. 반쯤 설치된 앱은 업데이터도 고치기 어렵다.
    // 필수 컴포넌트가 빠진 릴리즈는 기존 설치를 건드리기 전에 멈춘다
    let components = match release::installable(&manifest) {
        Ok(c) => c,
        Err(missing) => {
            let msg = format!("The release is missing required components: {}", missing.join(", "));
            emit_error(&app, &state, "release_incomplete", &msg).await;
            return;
        }
    };

    // 덮어쓰기 설치: 이 폴더의 데몬이 떠 있으면 먼저 끈다 (파일 잠금 방지)
    uninstall::stop_daemon_before_install(&install_dir).await;
    let total = components.len().max(1);
    let mut installed = Vec::new();
    let mut installed_versions = UpdateManager::load_installed_manifest();
    let temp_dir = std::env::temp_dir().join("saba-chan-installer");
    let _ = std::fs::create_dir_all(&temp_dir);

    for (i, (key, info)) in components.iter().enumerate() {
        let pct = 10 + (i * 35 / total) as i32;
        emit("download", &format!("Downloading {}...", key), pct);

        let temp_file = temp_dir.join(&info.filename);
        let app_dl = app.clone();
        let file_name = info.filename.clone();
        let key_name = key.to_string();
        let progress_cb = move |received: u64, total_bytes: Option<u64>| {
            let mb_received = received as f64 / 1_048_576.0;
            let msg = match total_bytes {
                Some(total) => format!(
                    "Downloading {} — {:.1}/{:.1} MB ({}%)",
                    key_name,
                    mb_received,
                    total as f64 / 1_048_576.0,
                    received * 100 / total.max(1)
                ),
                None => format!("Downloading {} — {:.1} MB", key_name, mb_received),
            };
            let p = InstallProgress {
                step: "download".to_string(),
                message: msg,
                percent: pct,
                download_file: Some(file_name.clone()),
                download_received: Some(received),
                download_total: total_bytes,
                ..Default::default()
            };
            app_dl.emit("install:progress", &p).ok();
        };

        if let Err(e) = release::download(info, &temp_file, progress_cb).await {
            emit_error(&app, &state, "download_failed", &format!("Failed to download {}: {}", key, e)).await;
            return;
        }

        emit("extract", &format!("Extracting {}...", key), pct + 3);
        if let Err(e) = extract_zip(&temp_file, &release::target_dir(&install_dir, info)) {
            emit_error(&app, &state, "extract_failed", &format!("Failed to extract {}: {}", key, e)).await;
            return;
        }
        let _ = std::fs::remove_file(&temp_file);
        installed_versions.insert(key.to_string(), info.version.clone());
        installed.push(key.to_string());
    }

    // Step 3.5: 공유 모듈 유틸리티(i18n.py, daemon_rcon.py)를 APPDATA/modules/로 설치
    emit("modules-shared", "Installing shared module utilities...", 46);
    for failure in install_shared_modules(&install_dir) {
        warn(failure);
    }

    // Step 4: 모듈 다운로드 및 설치 (45-55%) — Fix 4: 스트리밍 진행률
    if !config.selected_modules.is_empty() {
        emit("modules-download", "Downloading game modules...", 47);

        let data_dir = resolve_data_dir();
        for module_id in &config.selected_modules {
            let key = format!("{}{}", relay::MODULE_PREFIX, module_id);
            let Some(info) = manifest.components.get(&key) else {
                warn(format!("Module '{}' is not in the release", module_id));
                continue;
            };
            emit("modules-download", &format!("Installing module: {}...", module_id), 49);
            match release::install_addon(&key, info, &data_dir.join("modules").join(module_id)).await {
                Ok(()) => installed.push(format!("module:{}", module_id)),
                Err(e) => warn(format!("Failed to install module '{}': {}", module_id, e)),
            }
        }

        // ── Fix 3: 모듈 의존 익스텐션 자동 설치 ──
        emit("extensions", "Checking required extensions...", 54);
        let required_extensions = collect_required_extensions(
            &config.selected_modules,
            &config.module_metadata,
        );

        if !required_extensions.is_empty() {
            tracing::info!(
                "선택된 모듈에 필요한 익스텐션: {:?}",
                required_extensions
            );

            let (ext_names, failed) = install_required_extensions(&app, &manifest, &required_extensions).await;
            for name in ext_names {
                installed.push(format!("extension:{}", name));
            }
            for e in failed {
                warn(e);
            }
        }
    }

    // Step 5: 포터블 Python 다운로드 + venv 생성 (55-70%)
    // Fix 5: 기존 런타임이 손상되었으면 삭제 후 재설치
    emit("runtime-python", "Preparing Python runtime...", 57);
    let runtime_data_dir = runtime_bootstrap::resolve_runtime_data_dir(&install_dir);
    let _ = std::fs::create_dir_all(&runtime_data_dir);

    match runtime_bootstrap::setup_python_with_repair(&runtime_data_dir).await {
        Ok(python_path) => {
            tracing::info!("Python 환경 준비 완료: {}", python_path.display());
            installed.push("runtime:python".to_string());
        }
        Err(e) => {
            // Python 설정 실패는 치명적이지 않음 — 메인 앱이 첫 실행 시 재시도
            warn(format!("Python runtime was not prepared ({}); Saba-chan will retry on first start", e));
        }
    }

    // Step 7: 설정 파일 생성 (85-88%)
    emit("config", "Setting up configuration...", 86);
    setup_config(&install_dir, &config);

    // Step 8: 언어 설정 저장 (88-90%)
    emit("config-lang", "Saving language settings...", 89);
    save_language_setting(&config.language);

    // Step 9: 레지스트리 등록 (90-95%)
    emit("registry", "Registering application...", 92);
    #[cfg(target_os = "windows")]
    {
        if let Err(e) = registry::register_uninstall_entry(&install_dir, &manifest.version) {
            warn(format!("Failed to register in Installed apps: {}", e));
        }
    }

    // Step 10: 바로가기 (95-100%)
    emit("shortcuts", "Creating shortcuts...", 96);
    #[cfg(target_os = "windows")]
    {
        let gui_exe = saba_chan_updater_lib::find_gui_exe(&install_dir);
        let app_name = localized_app_name(&config.language);

        // 이 설치가 전에 만든 바로가기 제거 (언어 변경 시 이전 이름의 잔재 방지).
        // 다른 폴더에 설치된 같은 이름의 바로가기는 둔다
        shortcuts::remove_shortcuts_into(&install_dir, shortcuts::ALL_SHORTCUT_NAMES);

        match &gui_exe {
            Some(gui_exe) => {
                if config.create_desktop_shortcut {
                    if let Err(e) = shortcuts::create_desktop_shortcut(gui_exe, app_name) {
                        warn(format!("Desktop shortcut failed: {}", e));
                    }
                }
                if config.create_start_menu_shortcut {
                    if let Err(e) = shortcuts::create_start_menu_shortcut(gui_exe, app_name) {
                        warn(format!("Start menu shortcut failed: {}", e));
                    }
                }
            }
            None => warn(format!("No GUI executable in {}; shortcuts not created", install_dir.display())),
        }
    }

    // Step 10.5: 설치한 버전 기록 (업데이터가 같은 파일을 읽는다)
    emit("manifest", "Recording installed versions...", 98);
    if let Err(e) = UpdateManager::save_installed_manifest(&installed_versions) {
        warn(format!("Failed to write installed manifest: {}", e));
    }

    // 완료. 레지스트리·바로가기 등 실패한 단계는 숨기지 않고 완료 화면에 함께 보여 준다
    let warnings = warnings.into_inner().unwrap_or_default();
    let final_progress = InstallProgress {
        step: "complete".into(),
        message: if warnings.is_empty() {
            format!("{} components installed!", installed.len())
        } else {
            format!(
                "{} components installed, with problems:\n{}\nLog: {}",
                installed.len(),
                warnings.join("\n"),
                log_file_path().display()
            )
        },
        percent: 100,
        complete: true,
        error: None,
        installed_components: installed.clone(),
        download_file: None,
        download_received: None,
        download_total: None,
        log_path: (!warnings.is_empty()).then(|| log_file_path().display().to_string()),
        warnings,
        error_code: None,
    };
    state.write().await.progress = final_progress.clone();
    app.emit("install:progress", &final_progress).ok();

    // 임시 디렉토리 정리
    let _ = std::fs::remove_dir_all(std::env::temp_dir().join("saba-chan-installer"));
}

async fn emit_error(app: &AppHandle, state: &SharedState, code: &str, msg: &str) {
    // 안내하는 로그 파일에 실패 원인이 남도록 먼저 기록한다
    tracing::error!("[{}] {}", code, msg);
    let p = InstallProgress {
        step: "error".into(),
        message: msg.to_string(),
        percent: 0,
        complete: false,
        error: Some(msg.to_string()),
        installed_components: Vec::new(),
        download_file: None,
        download_received: None,
        download_total: None,
        warnings: Vec::new(),
        log_path: Some(log_file_path().display().to_string()),
        error_code: Some(code.to_string()),
    };
    state.write().await.progress = p.clone();
    app.emit("install:progress", &p).ok();
}

// ═══════════════════════════════════════════════════════
// Tauri 커맨드 — 제거
// ═══════════════════════════════════════════════════════

#[tauri::command]
async fn start_uninstall(app: AppHandle, keep_settings: Option<bool>) -> Result<(), String> {
    let guard = WorkGuard::try_begin().ok_or_else(|| "busy".to_string())?;
    let preserve = keep_settings.unwrap_or(false);
    tauri::async_runtime::spawn(async move {
        let _guard = guard;
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
    if let Some(exe) = saba_chan_updater_lib::find_gui_exe(Path::new(&install_path)) {
        std::process::Command::new(&exe)
            .current_dir(&install_path)
            .spawn()
            .map_err(|e| format!("Failed to launch Saba-chan: {}", e))?;
        Ok(())
    } else {
        Err(format!("Executable not found in {}", install_path))
    }
}

// ═══════════════════════════════════════════════════════
// 관리자 권한 상승 (UAC) — 디렉토리 생성 전용
// ═══════════════════════════════════════════════════════

/// UAC 프롬프트로 디렉토리를 생성하고 현재 사용자에게 쓰기 권한을 부여한다.
/// 앱 전체를 재시작하지 않고 디렉토리만 상승 처리한다.
/// ShellExecuteExW("runas")로 자기 자신을 --elevate-mkdir 모드로 실행하므로
/// UAC 대화상자에 "Windows PowerShell"이 아닌 인스톨러 이름이 표시된다.
#[cfg(target_os = "windows")]
fn elevate_create_dir(dir: &Path) -> Result<(), String> {
    use std::ffi::OsStr;
    use std::mem::{size_of, zeroed};
    use std::os::windows::ffi::OsStrExt;
    use windows_sys::Win32::Foundation::CloseHandle;
    use windows_sys::Win32::System::Threading::WaitForSingleObject;
    use windows_sys::Win32::UI::Shell::{
        ShellExecuteExW, SHELLEXECUTEINFOW, SEE_MASK_NOCLOSEPROCESS,
    };

    fn to_wide(s: &str) -> Vec<u16> {
        OsStr::new(s).encode_wide().chain(std::iter::once(0)).collect()
    }

    let self_exe = std::env::current_exe()
        .map_err(|e| format!("Failed to get current exe path: {}", e))?;

    let verb = to_wide("runas");
    let file = to_wide(&self_exe.to_string_lossy());
    // 권한은 상승 전 사용자에게 준다 (상승된 쪽의 USERNAME은 관리자 계정일 수 있다)
    let user = match (std::env::var("USERDOMAIN"), std::env::var("USERNAME")) {
        (Ok(d), Ok(u)) => format!("{}\\{}", d, u),
        (_, Ok(u)) => u,
        _ => return Err("Cannot determine the current user".into()),
    };
    let params = to_wide(&format!("--elevate-mkdir \"{}\" --grant \"{}\"", dir.to_string_lossy(), user));

    let mut sei: SHELLEXECUTEINFOW = unsafe { zeroed() };
    sei.cbSize = size_of::<SHELLEXECUTEINFOW>() as u32;
    sei.fMask = SEE_MASK_NOCLOSEPROCESS;
    sei.lpVerb = verb.as_ptr();
    sei.lpFile = file.as_ptr();
    sei.lpParameters = params.as_ptr();
    sei.nShow = 0; // SW_HIDE

    let ok = unsafe { ShellExecuteExW(&mut sei) };
    if ok == 0 {
        return Err("UAC elevation was cancelled or failed".into());
    }

    // 프로세스 완료 대기 (최대 30초)
    if !sei.hProcess.is_null() {
        unsafe {
            WaitForSingleObject(sei.hProcess, 30_000);
            CloseHandle(sei.hProcess);
        }
    }

    if dir.exists() {
        // 쓰기 권한 확인
        let probe = dir.join(".saba-write-test");
        if std::fs::write(&probe, b"test").is_ok() {
            let _ = std::fs::remove_file(&probe);
            return Ok(());
        }
        return Err("Directory exists but write permission was not granted".into());
    }

    Err("Directory creation was cancelled or failed".into())
}

/// 관리자 권한으로 만들 폴더 검사: 드라이브 절대 경로, `.`·`..` 없음, 두 단계 이상 깊이,
/// Windows 폴더 밖, 보호된 시스템 폴더(Program Files, ProgramData, Users, 사용자 폴더) 자체가 아님.
/// 이 폴더 전체에 사용자 전체 권한을 주므로 사바쨩 전용 하위 폴더만 받는다
fn check_elevated_target(dir: &Path) -> Result<(), String> {
    check_elevated_target_in(&dir.to_string_lossy(), &|name| std::env::var(name).ok())
}

fn check_elevated_target_in(dir: &str, env: &dyn Fn(&str) -> Option<String>) -> Result<(), String> {
    let norm = |p: &str| p.replace('/', "\\").trim_end_matches('\\').to_lowercase();
    let lower = norm(dir);
    let bytes = lower.as_bytes();
    if bytes.len() < 3 || !bytes[0].is_ascii_alphabetic() || bytes[1] != b':' || bytes[2] != b'\\' {
        return Err("path must be an absolute drive path".into());
    }
    let parts: Vec<&str> = lower[3..].split('\\').collect();
    if parts.iter().any(|c| c.is_empty() || *c == "." || *c == "..") {
        return Err("path must not contain . or ..".into());
    }
    if parts.len() < 2 {
        return Err("refusing a drive root or a top-level folder".into());
    }
    if let Some(sys) = env("SystemRoot").or_else(|| env("WINDIR")).map(|v| norm(&v)) {
        if !sys.is_empty() && (lower == sys || lower.starts_with(&format!("{}\\", sys))) {
            return Err("refusing a path inside the Windows folder".into());
        }
    }
    let users = env("SystemDrive").map(|d| format!("{}\\Users", d));
    let protected = ["ProgramFiles", "ProgramFiles(x86)", "ProgramW6432", "ProgramData", "USERPROFILE", "PUBLIC"]
        .into_iter()
        .filter_map(env)
        .chain(users);
    if protected.map(|p| norm(&p)).any(|p| !p.is_empty() && p == lower) {
        return Err("refusing a protected system folder".into());
    }
    Ok(())
}

fn is_link_or_reparse_point(path: &Path) -> bool {
    let Ok(meta) = std::fs::symlink_metadata(path) else { return false };
    #[cfg(windows)]
    {
        use std::os::windows::fs::MetadataExt;
        const FILE_ATTRIBUTE_REPARSE_POINT: u32 = 0x400;
        if meta.file_attributes() & FILE_ATTRIBUTE_REPARSE_POINT != 0 {
            return true;
        }
    }
    meta.file_type().is_symlink()
}

/// 권한을 줄 폴더의 내용 검사: 링크·정션을 거치지 않고, 아직 없거나, 비었거나, 사바쨩 설치 폴더(설치 표시나 실행 파일)여야 한다.
/// `icacls /T`가 하위 전체에 권한을 주므로 다른 파일이 든 기존 폴더는 받지 않는다
fn check_elevated_contents(dir: &Path) -> Result<(), String> {
    // 정션·심볼릭 링크를 따라가면 다른 폴더(System32 등)에 권한이 걸린다: 자신과 위 폴더 모두 확인
    if dir.ancestors().any(is_link_or_reparse_point) {
        return Err("refusing a path through a link or junction".into());
    }
    if !dir.exists()
        || is_empty_dir(dir)
        || dir.join(uninstall::INSTALL_MARKER).is_file()
        || uninstall::looks_like_install_dir(dir)
    {
        Ok(())
    } else {
        Err("refusing an existing folder that is not a Saba-chan install".into())
    }
}

#[cfg(not(target_os = "windows"))]
fn elevate_create_dir(_dir: &Path) -> Result<(), String> {
    Err("Elevation is only supported on Windows".to_string())
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
    Ok(path.map(|p| dedicated_install_dir(Path::new(&p.to_string())).to_string_lossy().into_owned()))
}

/// 설치에 쓸 폴더: 고른 폴더가 비어 있지 않고 사바쨩 설치도 아니면 그 아래 `Saba-chan` 폴더.
/// 제거기가 설치 폴더를 지우므로 사용자 파일이 든 폴더에 바로 풀지 않는다
fn dedicated_install_dir(chosen: &Path) -> PathBuf {
    if chosen.as_os_str().is_empty() || !chosen.is_dir() || is_empty_dir(chosen) || uninstall::looks_like_install_dir(chosen) {
        chosen.to_path_buf()
    } else {
        chosen.join("Saba-chan")
    }
}

fn is_empty_dir(dir: &Path) -> bool {
    std::fs::read_dir(dir).map(|mut it| it.next().is_none()).unwrap_or(false)
}

// ═══════════════════════════════════════════════════════
// 헬퍼 함수
// ═══════════════════════════════════════════════════════

/// 설치 언어에 맞는 앱 표시 이름을 반환한다 (바로가기 이름, Windows 전용).
#[cfg_attr(not(windows), allow(dead_code))]
fn localized_app_name(language: &str) -> &'static str {
    match language {
        "ko" => "사바쨩",
        "ja" => "サバちゃん",
        _ => "Saba-chan",
    }
}

/// saba-core와 동일한 데이터 디렉토리 경로 해석
/// Windows: `%APPDATA%/saba-chan`, Linux/macOS: `~/.config/saba-chan`
fn resolve_data_dir() -> PathBuf {
    constants::resolve_data_dir()
}

fn get_default_install_path() -> String {
    #[cfg(target_os = "windows")]
    {
        r"C:\Program Files\Saba-chan".to_string()
    }
    #[cfg(not(target_os = "windows"))]
    {
        if let Ok(home) = std::env::var("HOME") {
            return PathBuf::from(home)
                .join(".local")
                .join("share")
                .join("saba-chan")
                .to_string_lossy()
                .to_string();
        }
        "saba-chan".to_string()
    }
}

fn load_main_app_language() -> Option<String> {
    ConfigFile::default_location()
        .section("gui")
        .get("language")
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

fn extract_zip(zip_path: &Path, target_dir: &Path) -> anyhow::Result<()> {
    std::fs::create_dir_all(target_dir)?;
    // 덮어쓰기 설치 때 켜진 서버의 saba-runner처럼 잠긴 exe는 업데이터처럼 비켜 둔다
    saba_chan_updater_lib::extract_zip(zip_path, target_dir, saba_chan_updater_lib::move_aside_binary).map(|_| ())
}

/// core zip에 포함된 공유 Python 모듈(i18n.py, daemon_rcon.py)을
/// %APPDATA%/saba-chan/modules/로 복사한다.
/// 게임 모듈(minecraft, palworld 등)의 lifecycle.py가 이 유틸리티를 import 한다.
/// 실패한 것마다 안내 문장을 돌려준다 (완료 화면 경고)
fn install_shared_modules(install_dir: &Path) -> Vec<String> {
    const SHARED_MODULES: &[&str] = &["i18n.py", "daemon_rcon.py"];

    let data_dir = resolve_data_dir();
    let modules_dir = data_dir.join("modules");
    let _ = std::fs::create_dir_all(&modules_dir);
    let mut failures = Vec::new();

    for name in SHARED_MODULES {
        let src = install_dir.join(name);
        let dst = modules_dir.join(name);
        if src.exists() {
            match std::fs::copy(&src, &dst) {
                Ok(_) => {
                    tracing::info!("Shared module installed: {} → {}", src.display(), dst.display());
                    // install_dir에 남아 있는 복사본 제거 (Program Files에 불필요)
                    let _ = std::fs::remove_file(&src);
                }
                Err(e) => failures.push(format!("Failed to install shared module {}: {}", name, e)),
            }
        } else {
            tracing::debug!("Shared module not found in zip: {}", name);
        }
    }
    failures
}

fn setup_config(install_dir: &Path, config: &InstallConfig) {
    // 설정은 코드에 내장되므로 config 파일 생성 불필요
    // 필수 디렉터리만 생성
    let _ = std::fs::create_dir_all(install_dir.join("locales"));
    let data_dir = resolve_data_dir();
    let _ = std::fs::create_dir_all(data_dir.join("modules"));

    // 언어 설정만 별도 저장
    save_language_setting(&config.language);
}

/// 설치 시 고른 언어를 GUI 설정(`config.json`의 `gui.language`)에 넣는다. 다른 키는 보존.
fn save_language_setting(language: &str) {
    let result = ConfigFile::default_location().update(|doc| {
        let gui = doc
            .entry("gui".to_string())
            .or_insert_with(|| serde_json::json!({}));
        if !gui.is_object() {
            *gui = serde_json::json!({});
        }
        gui["language"] = serde_json::Value::String(language.to_string());
    });
    if let Err(e) = result {
        tracing::warn!("Failed to save language setting: {}", e);
    }
}

// ═══════════════════════════════════════════════════════
// Fix 1: 버전 비교 유틸리티
// ═══════════════════════════════════════════════════════

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

// ═══════════════════════════════════════════════════════
// Fix 3: 모듈 의존 익스텐션 자동 설치
// ═══════════════════════════════════════════════════════

/// 선택된 모듈의 required_extensions를 수집 (중복 제거)
fn collect_required_extensions(
    selected_modules: &[String],
    module_metadata: &[ModuleInfo],
) -> Vec<String> {
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

/// 필요한 익스텐션을 릴리즈 manifest의 `ext-*` 컴포넌트에서 받아 설치 (SHA-256 검증)
async fn install_required_extensions(
    app: &AppHandle,
    manifest: &ReleaseManifest,
    required_extensions: &[String],
) -> (Vec<String>, Vec<String>) {
    let data_dir = resolve_data_dir();
    let mut installed = Vec::new();
    let mut failed = Vec::new();
    for ext_id in required_extensions {
        let key = format!("{}{}", relay::EXTENSION_PREFIX, ext_id);
        let Some(info) = manifest.components.get(&key) else {
            failed.push(format!("Extension {} is not in this release", ext_id));
            continue;
        };
        let p = InstallProgress {
            step: "extensions".to_string(),
            message: format!("Installing extension: {}...", ext_id),
            percent: 54,
            download_file: Some(info.filename.clone()),
            ..Default::default()
        };
        app.emit("install:progress", &p).ok();

        let default_dir = format!("extensions/{}", ext_id.replace('-', "_"));
        let target_dir = data_dir.join(info.install_dir.as_deref().unwrap_or(&default_dir));
        match release::install_addon(&key, info, &target_dir).await {
            Ok(()) => {
                tracing::info!("익스텐션 '{}' 설치 완료: {}", ext_id, target_dir.display());
                installed.push(ext_id.clone());
            }
            Err(e) => failed.push(format!("Failed to install extension {}: {}", ext_id, e)),
        }
    }
    (installed, failed)
}

// ═══════════════════════════════════════════════════════
// 엔트리
// ═══════════════════════════════════════════════════════

/// 로그 파일: GUI 앱이라 stderr는 보이지 않는다. 실패한 단계를 나중에 확인할 수 있게 파일에도 남긴다
pub fn log_file_path() -> PathBuf {
    std::env::temp_dir().join("saba-chan-installer.log")
}

fn init_logging() {
    use tracing_subscriber::fmt::writer::MakeWriterExt;
    let filter = tracing_subscriber::EnvFilter::try_from_default_env()
        .unwrap_or_else(|_| tracing_subscriber::EnvFilter::new("info"));
    let builder = tracing_subscriber::fmt().with_env_filter(filter).with_target(false).with_ansi(false);
    match std::fs::OpenOptions::new().create(true).append(true).open(log_file_path()) {
        Ok(file) => builder.with_writer(std::io::stderr.and(std::sync::Mutex::new(file))).init(),
        Err(_) => builder.with_writer(std::io::stderr).init(),
    }
    tracing::info!("Saba-chan installer {} started", env!("CARGO_PKG_VERSION"));
}

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    let args: Vec<String> = std::env::args().collect();

    // --elevate-mkdir <path> → UAC 상승된 상태에서 디렉토리 생성 + 권한 부여 후 즉시 종료
    // 이 분기는 elevate_create_dir()가 ShellExecuteW("runas")로 자기 자신을 재실행할 때 진입한다.
    if let Some(pos) = args.iter().position(|a| a == "--elevate-mkdir") {
        if let Some(dir_str) = args.get(pos + 1) {
            let dir = Path::new(dir_str);
            if let Err(e) = check_elevated_target(dir).and_then(|_| check_elevated_contents(dir)) {
                eprintln!("elevate-mkdir: {}", e);
                std::process::exit(1);
            }
            // 디렉토리 생성
            if let Err(e) = std::fs::create_dir_all(dir) {
                eprintln!("elevate-mkdir: failed to create dir: {}", e);
                std::process::exit(1);
            }
            // 상승 전 사용자에게 전체 권한 부여
            let Some(username) = args.iter().position(|a| a == "--grant").and_then(|i| args.get(i + 1)).cloned() else {
                eprintln!("elevate-mkdir: missing --grant user");
                std::process::exit(1);
            };
            let icacls = saba_chan_updater_lib::process::hide_console(&mut std::process::Command::new("icacls"))
                .args([
                    dir_str.as_str(),
                    "/grant",
                    &format!("{}:(OI)(CI)F", username),
                    "/T",
                    "/Q",
                ])
                .output();
            match icacls {
                Ok(out) if out.status.success() => std::process::exit(0),
                Ok(out) => {
                    eprintln!(
                        "elevate-mkdir: icacls failed: {}",
                        String::from_utf8_lossy(&out.stderr)
                    );
                    std::process::exit(1);
                }
                Err(e) => {
                    eprintln!("elevate-mkdir: icacls spawn error: {}", e);
                    std::process::exit(1);
                }
            }
        } else {
            eprintln!("elevate-mkdir: missing path argument");
            std::process::exit(1);
        }
    }

    let is_uninstall = args.iter().any(|a| a == "--uninstall");
    let is_silent = args.iter().any(|a| a == "--silent");

    // --uninstall --silent → 사일런트 제거 (GUI 없이)
    if is_uninstall && is_silent {
        #[cfg(target_os = "windows")]
        {
            use windows_sys::Win32::System::Console::{
                AllocConsole, AttachConsole, ATTACH_PARENT_PROCESS,
            };
            unsafe {
                if AttachConsole(ATTACH_PARENT_PROCESS) == 0 {
                    let _ = AllocConsole();
                }
            }
        }

        init_logging();

        let rt = tokio::runtime::Runtime::new().expect("Failed to create tokio runtime");
        rt.block_on(async {
            uninstall::do_silent_uninstall().await;
        });
        return;
    }

    // GUI 모드
    init_logging();

    let app_mode = AppMode {
        uninstall: is_uninstall,
        silent: is_silent,
    };

    let mut initial = InstallerState::default();
    if is_uninstall {
        // 제거 화면에 보이는 경로 = 실제로 지울 폴더 (제거기 자신이 든 설치 폴더)
        initial.install_path = uninstall::target_dir().map(|d| d.to_string_lossy().into_owned()).unwrap_or_default();
    }
    let installer_state: SharedState = Arc::new(RwLock::new(initial));

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

    #[test]
    fn only_one_install_or_uninstall_runs_at_a_time() {
        let first = WorkGuard::try_begin().expect("nothing is running yet");
        assert!(WorkGuard::try_begin().is_none(), "a second click must not start another run");
        drop(first);
        assert!(WorkGuard::try_begin().is_some(), "finishing frees the slot");
    }

    #[test]
    fn a_folder_with_other_files_gets_its_own_saba_chan_subfolder() {
        let root = std::env::temp_dir().join(format!("saba-install-dir-test-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        std::fs::create_dir_all(&root).unwrap();
        assert_eq!(dedicated_install_dir(&root), root, "an empty folder is used as is");
        assert_eq!(dedicated_install_dir(&root.join("new")), root.join("new"), "a new folder is used as is");

        std::fs::write(root.join("notes.txt"), b"").unwrap();
        assert_eq!(dedicated_install_dir(&root), root.join("Saba-chan"));

        let core = format!("saba-core{}", std::env::consts::EXE_SUFFIX);
        std::fs::write(root.join(core), b"").unwrap();
        assert_eq!(dedicated_install_dir(&root), root, "an existing install is updated in place");
        std::fs::remove_dir_all(&root).unwrap();
    }

    #[test]
    fn elevation_grants_only_on_a_new_empty_or_saba_chan_folder() {
        let root = std::env::temp_dir().join(format!("saba-elevate-test-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        std::fs::create_dir_all(&root).unwrap();
        assert!(check_elevated_contents(&root.join("new")).is_ok(), "a folder that does not exist yet");
        assert!(check_elevated_contents(&root).is_ok(), "an empty folder");

        std::fs::write(root.join("report.docx"), b"").unwrap();
        assert!(check_elevated_contents(&root).is_err(), "a folder with someone else's files");

        std::fs::write(root.join(uninstall::INSTALL_MARKER), b"").unwrap();
        assert!(check_elevated_contents(&root).is_ok(), "a folder the installer made");
        std::fs::remove_dir_all(&root).unwrap();
    }

    #[cfg(unix)]
    #[test]
    fn elevation_refuses_a_link_or_a_folder_below_a_link() {
        let root = std::env::temp_dir().join(format!("saba-elevate-link-test-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        std::fs::create_dir_all(root.join("real")).unwrap();
        std::os::unix::fs::symlink(root.join("real"), root.join("link")).unwrap();
        assert!(check_elevated_contents(&root.join("link")).is_err(), "the folder itself is a link");
        assert!(check_elevated_contents(&root.join("link").join("Saba-chan")).is_err(), "a parent is a link");
        assert!(check_elevated_contents(&root.join("real").join("Saba-chan")).is_ok());
        std::fs::remove_dir_all(&root).unwrap();
    }

    #[test]
    fn elevation_only_creates_a_dedicated_folder_below_a_parent() {
        let env = |name: &str| match name {
            "SystemRoot" => Some(r"C:\Windows".to_string()),
            "ProgramFiles" => Some(r"C:\Program Files".to_string()),
            "ProgramFiles(x86)" => Some(r"C:\Program Files (x86)".to_string()),
            "ProgramData" => Some(r"C:\ProgramData".to_string()),
            "SystemDrive" => Some("C:".to_string()),
            "USERPROFILE" => Some(r"C:\Users\me".to_string()),
            _ => None,
        };
        for refused in [
            r"C:\Program Files",
            r"c:\program files\",
            r"C:\Program Files (x86)",
            r"C:\ProgramData",
            r"C:\Users",
            r"C:\Users\me",
            r"C:\Windows\Saba-chan",
            r"C:\Games",
            r"C:\",
            r"C:\Program Files\..\Users\x",
            r"Saba-chan",
        ] {
            assert!(check_elevated_target_in(refused, &env).is_err(), "{refused} must be refused");
        }
        for allowed in [r"C:\Program Files\Saba-chan", r"D:\Games\Saba-chan", r"C:\Users\me\Saba-chan"] {
            assert!(check_elevated_target_in(allowed, &env).is_ok(), "{allowed} must be allowed");
        }
    }
}
