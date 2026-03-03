//! Saba-chan Installer — 설치/제거 GUI (Tauri)
//!
//! ## 실행 모드
//! - `saba-chan-installer`                     → GUI 설치 모드 (5-page wizard)
//! - `saba-chan-installer --uninstall`          → GUI 제거 모드
//! - `saba-chan-installer --uninstall --silent` → 사일런트 제거 (GUI 없음)

pub mod github;
pub mod registry;
pub mod runtime_bootstrap;
pub mod shortcuts;
pub mod uninstall;

use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};
use std::sync::Arc;
use tauri::{AppHandle, Emitter, Manager, State};
use tokio::sync::RwLock;

const SUPPORTED_LANGUAGES: [&str; 10] = [
    "en", "ko", "ja", "zh-CN", "zh-TW", "es", "pt-BR", "ru", "de", "fr",
];

const DEFAULT_GITHUB_OWNER: &str = "WareAoba";
const DEFAULT_GITHUB_REPO: &str = "saba-chan";
const MODULES_GITHUB_REPO: &str = "saba-chan-modules";

// ═══════════════════════════════════════════════════════
// 타입
// ═══════════════════════════════════════════════════════

/// 인스톨러 전역 상태
#[derive(Debug)]
pub struct InstallerState {
    pub install_path: String,
    pub github_owner: String,
    pub github_repo: String,
    pub language: String,
    pub create_desktop_shortcut: bool,
    pub create_start_menu_shortcut: bool,
    pub selected_modules: Vec<String>,
    pub latest_release_tag: Option<String>,
    pub progress: InstallProgress,
}

impl Default for InstallerState {
    fn default() -> Self {
        Self {
            install_path: get_default_install_path(),
            github_owner: DEFAULT_GITHUB_OWNER.to_string(),
            github_repo: DEFAULT_GITHUB_REPO.to_string(),
            language: "en".to_string(),
            create_desktop_shortcut: true,
            create_start_menu_shortcut: true,
            selected_modules: Vec::new(),
            latest_release_tag: None,
            progress: InstallProgress::default(),
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
}

/// 사용 가능한 모듈 정보
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ModuleInfo {
    pub id: String,
    pub name: String,
    pub description: String,
    pub icon: String,
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
        "latest_release_tag": s.latest_release_tag,
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
// Tauri 커맨드 — 모듈
// ═══════════════════════════════════════════════════════

#[tauri::command]
async fn get_available_modules() -> Result<Vec<ModuleInfo>, String> {
    Ok(vec![
        ModuleInfo {
            id: "minecraft".into(),
            name: "Minecraft".into(),
            description: "Minecraft server management with RCON support".into(),
            icon: "icon-minecraft.png".into(),
        },
        ModuleInfo {
            id: "palworld".into(),
            name: "Palworld".into(),
            description: "Palworld dedicated server management via REST API".into(),
            icon: "icon-palworld.png".into(),
        },
        ModuleInfo {
            id: "zomboid".into(),
            name: "Project Zomboid".into(),
            description: "Project Zomboid dedicated server management".into(),
            icon: "icon-zomboid.png".into(),
        },
    ])
}

// ═══════════════════════════════════════════════════════
// Tauri 커맨드 — GitHub (항상 최신 릴리스)
// ═══════════════════════════════════════════════════════

#[tauri::command]
async fn fetch_latest_release(
    state: State<'_, SharedState>,
) -> Result<serde_json::Value, String> {
    let (owner, repo) = {
        let s = state.read().await;
        (s.github_owner.clone(), s.github_repo.clone())
    };

    let releases = github::fetch_releases(&owner, &repo)
        .await
        .map_err(|e| format!("Failed to fetch releases: {}", e))?;

    let latest = releases
        .iter()
        .find(|r| !r.prerelease)
        .or(releases.first())
        .ok_or("No releases found")?;

    let tag = latest.tag_name.clone();
    let name = latest.name.clone().unwrap_or_else(|| tag.clone());

    state.write().await.latest_release_tag = Some(tag.clone());

    Ok(serde_json::json!({
        "tag": tag,
        "name": name,
        "published_at": latest.published_at,
    }))
}

// ═══════════════════════════════════════════════════════
// Tauri 커맨드 — 설치 실행
// ═══════════════════════════════════════════════════════

#[tauri::command]
async fn start_install(
    app: AppHandle,
    state: State<'_, SharedState>,
) -> Result<(), String> {
    let config = {
        let s = state.read().await;
        InstallConfig {
            install_path: s.install_path.clone(),
            github_owner: s.github_owner.clone(),
            github_repo: s.github_repo.clone(),
            language: s.language.clone(),
            create_desktop_shortcut: s.create_desktop_shortcut,
            create_start_menu_shortcut: s.create_start_menu_shortcut,
            selected_modules: s.selected_modules.clone(),
            latest_release_tag: s.latest_release_tag.clone(),
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
    github_owner: String,
    github_repo: String,
    language: String,
    create_desktop_shortcut: bool,
    create_start_menu_shortcut: bool,
    selected_modules: Vec<String>,
    latest_release_tag: Option<String>,
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
        };
        app.emit("install:progress", &p).ok();
    };

    // Step 1: 설치 디렉토리 생성 + 쓰기 권한 확인 (0-5%)
    emit("prepare", "Creating install directory...", 2);
    let install_dir = PathBuf::from(&config.install_path);
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
                &format!("Failed to create directory: {}", elev_err),
            )
            .await;
            return;
        }
    }

    // Step 2: 릴리즈 매니페스트 페치 (5-10%)
    emit("fetch", "Fetching release information...", 7);
    let tag = match &config.latest_release_tag {
        Some(t) => t.clone(),
        None => {
            match github::fetch_releases(&config.github_owner, &config.github_repo).await {
                Ok(releases) => {
                    match releases.iter().find(|r| !r.prerelease).or(releases.first()) {
                        Some(r) => r.tag_name.clone(),
                        None => {
                            emit_error(&app, &state, "No releases found").await;
                            return;
                        }
                    }
                }
                Err(e) => {
                    emit_error(
                        &app,
                        &state,
                        &format!("Failed to fetch releases: {}", e),
                    )
                    .await;
                    return;
                }
            }
        }
    };

    let manifest = match github::fetch_manifest(
        &config.github_owner,
        &config.github_repo,
        &tag,
    )
    .await
    {
        Ok(m) => m,
        Err(e) => {
            emit_error(&app, &state, &format!("Failed to fetch manifest: {}", e)).await;
            return;
        }
    };

    // Step 3: 에셋 다운로드 + 압축 해제 (10-45%)
    let components: Vec<_> = manifest.components.iter().collect();
    let total = components.len().max(1);
    let mut installed = Vec::new();

    for (i, (key, info)) in components.iter().enumerate() {
        let asset_name = match &info.asset {
            Some(a) if !a.is_empty() => a.clone(),
            _ => continue,
        };

        let pct = 10 + (i * 35 / total) as i32;
        emit("download", &format!("Downloading {}...", key), pct);

        let download_url = match github::get_asset_download_url(
            &config.github_owner,
            &config.github_repo,
            &tag,
            &asset_name,
        )
        .await
        {
            Ok(url) => url,
            Err(e) => {
                tracing::error!("Failed to get URL for {}: {}", key, e);
                continue;
            }
        };

        let temp_dir = std::env::temp_dir().join("saba-chan-installer");
        let _ = std::fs::create_dir_all(&temp_dir);
        let temp_file = temp_dir.join(&asset_name);

        if let Err(e) = github::download_asset(&download_url, &temp_file).await {
            tracing::error!("Failed to download {}: {}", asset_name, e);
            continue;
        }

        emit("extract", &format!("Extracting {}...", key), pct + 3);

        let target_dir = match &info.install_dir {
            Some(d) if !d.is_empty() && d != "." => install_dir.join(d),
            _ => install_dir.clone(),
        };

        if let Err(e) = extract_zip(&temp_file, &target_dir) {
            tracing::error!("Failed to extract {}: {}", asset_name, e);
            continue;
        }

        let _ = std::fs::remove_file(&temp_file);
        installed.push(key.to_string());
    }

    // Step 4: 모듈 다운로드 및 설치 (45-55%)
    if !config.selected_modules.is_empty() {
        emit("modules", "Downloading game modules...", 47);

        let temp_dir = std::env::temp_dir().join("saba-chan-installer");
        let _ = std::fs::create_dir_all(&temp_dir);
        let modules_zip = temp_dir.join("saba-chan-modules.zip");

        match github::download_repo_zipball(
            &config.github_owner,
            MODULES_GITHUB_REPO,
            &modules_zip,
        )
        .await
        {
            Ok(()) => {
                emit("modules", "Extracting game modules...", 52);
                match extract_modules_from_zipball(
                    &modules_zip,
                    &install_dir,
                    &config.selected_modules,
                ) {
                    Ok(module_names) => {
                        for name in module_names {
                            installed.push(format!("module:{}", name));
                        }
                    }
                    Err(e) => {
                        tracing::warn!("Failed to extract modules: {}", e);
                    }
                }
                let _ = std::fs::remove_file(&modules_zip);
            }
            Err(e) => {
                tracing::warn!("Failed to download modules: {}", e);
            }
        }
    }

    // Step 5: 포터블 Python 다운로드 + venv 생성 (55-70%)
    emit("runtime", "파이썬 런타임 준비중...", 57);
    let runtime_data_dir = runtime_bootstrap::resolve_runtime_data_dir(&install_dir);
    let _ = std::fs::create_dir_all(&runtime_data_dir);

    match runtime_bootstrap::setup_python(&runtime_data_dir).await {
        Ok(python_path) => {
            tracing::info!("Python 환경 준비 완료: {}", python_path.display());
            installed.push("runtime:python".to_string());
        }
        Err(e) => {
            tracing::warn!("Python 환경 설정 실패 (비치명적): {}", e);
            // Python 설정 실패는 치명적이지 않음 — 메인 앱이 첫 실행 시 재시도
        }
    }

    // Step 6: 포터블 Node.js 다운로드 + npm install (70-85%)
    emit("runtime", "Discord 봇 환경 준비중...", 72);

    match runtime_bootstrap::setup_node(&runtime_data_dir).await {
        Ok(node_path) => {
            tracing::info!("Node.js 환경 준비 완료: {}", node_path.display());
            installed.push("runtime:nodejs".to_string());

            // Discord Bot npm install
            let bot_dir = install_dir.join("discord_bot");
            if bot_dir.join("package.json").exists() {
                emit("runtime", "Discord 봇 환경 준비중...", 78);
                match runtime_bootstrap::npm_install(&node_path, &bot_dir).await {
                    Ok(()) => {
                        tracing::info!("Discord Bot npm install 완료");
                        installed.push("runtime:npm-deps".to_string());
                    }
                    Err(e) => {
                        tracing::warn!("npm install 실패 (비치명적): {}", e);
                    }
                }
            }
        }
        Err(e) => {
            tracing::warn!("Node.js 환경 설정 실패 (비치명적): {}", e);
        }
    }

    // Step 7: 설정 파일 생성 (85-88%)
    emit("config", "Setting up configuration...", 86);
    setup_config(&install_dir, &config);

    // Step 8: 언어 설정 저장 (88-90%)
    emit("config", "Saving language settings...", 89);
    save_language_setting(&config.language);

    // Step 9: 레지스트리 등록 (90-95%)
    emit("registry", "Registering application...", 92);
    #[cfg(target_os = "windows")]
    {
        if let Err(e) = registry::register_uninstall_entry(&install_dir, &tag) {
            tracing::warn!("Failed to create registry entry: {}", e);
        }
    }

    // Step 10: 바로가기 (95-100%)
    emit("shortcuts", "Creating shortcuts...", 96);
    #[cfg(target_os = "windows")]
    {
        let gui_exe = install_dir.join("saba-chan-gui").join("saba-chan-gui.exe");
        let app_name = localized_app_name(&config.language);

        // 기존 바로가기 제거 (언어 변경 시 이전 이름의 잔재 방지)
        const ALL_SHORTCUT_NAMES: &[&str] = &["Saba-chan", "사바쨩", "サバちゃん"];
        for old_name in ALL_SHORTCUT_NAMES {
            let _ = shortcuts::remove_desktop_shortcut(old_name);
            let _ = shortcuts::remove_start_menu_shortcut(old_name);
        }

        if gui_exe.exists() {
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
    }

    // 완료
    let final_progress = InstallProgress {
        step: "complete".into(),
        message: format!("{} components installed!", installed.len()),
        percent: 100,
        complete: true,
        error: None,
        installed_components: installed.clone(),
    };
    state.write().await.progress = final_progress.clone();
    app.emit("install:progress", &final_progress).ok();

    // 임시 디렉토리 정리
    let _ = std::fs::remove_dir_all(std::env::temp_dir().join("saba-chan-installer"));
}

async fn emit_error(app: &AppHandle, state: &SharedState, msg: &str) {
    let p = InstallProgress {
        step: "error".into(),
        message: msg.to_string(),
        percent: 0,
        complete: false,
        error: Some(msg.to_string()),
        installed_components: Vec::new(),
    };
    state.write().await.progress = p.clone();
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
        .join("saba-chan-gui")
        .join(if cfg!(windows) { "saba-chan-gui.exe" } else { "saba-chan-gui" });
    if exe.exists() {
        std::process::Command::new(&exe)
            .current_dir(&install_path)
            .spawn()
            .map_err(|e| format!("Failed to launch Saba-chan: {}", e))?;
        Ok(())
    } else {
        Err(format!("Executable not found: {}", exe.display()))
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
    let params = to_wide(&format!("--elevate-mkdir \"{}\"", dir.to_string_lossy()));

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

#[cfg(not(target_os = "windows"))]
fn elevate_create_dir(dir: &Path) -> Result<(), String> {
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
    Ok(path.map(|p| p.to_string()))
}

// ═══════════════════════════════════════════════════════
// 헬퍼 함수
// ═══════════════════════════════════════════════════════

/// 설치 언어에 맞는 앱 표시 이름을 반환한다.
fn localized_app_name(language: &str) -> &'static str {
    match language {
        "ko" => "사바쨩",
        "ja" => "サバちゃん",
        _ => "Saba-chan",
    }
}

fn get_default_install_path() -> String {
    #[cfg(target_os = "windows")]
    {
        return r"C:\Program Files\Saba-chan".to_string();
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
    let path = get_settings_path()?;
    let content = std::fs::read_to_string(path).ok()?;
    let val: serde_json::Value = serde_json::from_str(&content).ok()?;
    val.get("language")
        .and_then(|v| v.as_str())
        .map(|s| s.to_string())
}

fn get_settings_path() -> Option<PathBuf> {
    #[cfg(target_os = "windows")]
    {
        let appdata = std::env::var("APPDATA").ok()?;
        Some(PathBuf::from(appdata).join("saba-chan").join("settings.json"))
    }
    #[cfg(not(target_os = "windows"))]
    {
        let home = std::env::var("HOME").ok()?;
        Some(
            PathBuf::from(home)
                .join(".config")
                .join("saba-chan")
                .join("settings.json"),
        )
    }
}

fn normalize_language_tag(input: &str) -> Option<String> {
    let trimmed = input.trim();
    if trimmed.is_empty() {
        return None;
    }
    let canonical = trimmed.replace('_', "-");
    for supported in SUPPORTED_LANGUAGES {
        if supported.eq_ignore_ascii_case(&canonical) {
            return Some(supported.to_string());
        }
    }
    let lower = canonical.to_lowercase();
    if lower.starts_with("pt") {
        return Some("pt-BR".to_string());
    }
    if lower.starts_with("zh-cn") || lower.starts_with("zh-hans") {
        return Some("zh-CN".to_string());
    }
    if lower.starts_with("zh-tw") || lower.starts_with("zh-hant") {
        return Some("zh-TW".to_string());
    }
    let base = lower.split('-').next().unwrap_or("en");
    match base {
        "en" => Some("en".to_string()),
        "ko" => Some("ko".to_string()),
        "ja" => Some("ja".to_string()),
        "zh" => Some("zh-CN".to_string()),
        "es" => Some("es".to_string()),
        "ru" => Some("ru".to_string()),
        "de" => Some("de".to_string()),
        "fr" => Some("fr".to_string()),
        _ => None,
    }
}

fn extract_zip(zip_path: &PathBuf, target_dir: &PathBuf) -> anyhow::Result<()> {
    let file = std::fs::File::open(zip_path)?;
    let mut archive = zip::ZipArchive::new(file)?;

    std::fs::create_dir_all(target_dir)?;

    for i in 0..archive.len() {
        let mut entry = archive.by_index(i)?;
        let name = entry.name().to_string();
        let out_path = target_dir.join(&name);

        if entry.is_dir() {
            std::fs::create_dir_all(&out_path)?;
        } else {
            if let Some(parent) = out_path.parent() {
                std::fs::create_dir_all(parent)?;
            }
            let mut out_file = std::fs::File::create(&out_path)?;
            std::io::copy(&mut entry, &mut out_file)?;
        }
    }

    Ok(())
}

fn extract_modules_from_zipball(
    zip_path: &Path,
    install_dir: &Path,
    selected_modules: &[String],
) -> anyhow::Result<Vec<String>> {
    let file = std::fs::File::open(zip_path)?;
    let mut archive = zip::ZipArchive::new(file)?;
    let mut installed: Vec<String> = Vec::new();

    let modules_dir = install_dir.join("modules");
    std::fs::create_dir_all(&modules_dir)?;

    for i in 0..archive.len() {
        let mut entry = archive.by_index(i)?;
        let name = entry.name().to_string();

        // Strip root dir prefix (e.g., "WareAoba-saba-chan-modules-abc1234/")
        let parts: Vec<&str> = name.splitn(2, '/').collect();
        if parts.len() < 2 {
            continue;
        }
        let relative = parts[1];
        if relative.is_empty() {
            continue;
        }

        // Check if this file belongs to a selected module
        let module_name = relative.split('/').next().unwrap_or("");
        if !selected_modules.iter().any(|m| m == module_name) {
            continue;
        }

        // Skip __pycache__, .git, etc.
        if relative.contains("__pycache__") || relative.starts_with('.') {
            continue;
        }

        let out_path = modules_dir.join(relative);

        if entry.is_dir() {
            std::fs::create_dir_all(&out_path)?;
        } else {
            if let Some(parent) = out_path.parent() {
                std::fs::create_dir_all(parent)?;
            }
            let mut out_file = std::fs::File::create(&out_path)?;
            std::io::copy(&mut entry, &mut out_file)?;
        }

        if !installed.contains(&module_name.to_string()) {
            installed.push(module_name.to_string());
        }
    }

    Ok(installed)
}

fn setup_config(install_dir: &PathBuf, config: &InstallConfig) {
    // 설정은 코드에 내장되므로 config 파일 생성 불필요
    // 필수 디렉터리만 생성
    let _ = std::fs::create_dir_all(install_dir.join("locales"));
    let _ = std::fs::create_dir_all(install_dir.join("modules"));

    // 언어 설정만 별도 저장
    save_language_setting(&config.language);
}

fn save_language_setting(language: &str) {
    #[cfg(target_os = "windows")]
    {
        if let Ok(appdata) = std::env::var("APPDATA") {
            let settings_dir = PathBuf::from(appdata).join("saba-chan");
            let _ = std::fs::create_dir_all(&settings_dir);
            let settings = serde_json::json!({ "language": language });
            let _ = std::fs::write(
                settings_dir.join("settings.json"),
                serde_json::to_string_pretty(&settings).unwrap_or_default(),
            );
        }
    }
    #[cfg(not(target_os = "windows"))]
    {
        if let Ok(home) = std::env::var("HOME") {
            let settings_dir = PathBuf::from(home).join(".config").join("saba-chan");
            let _ = std::fs::create_dir_all(&settings_dir);
            let settings = serde_json::json!({ "language": language });
            let _ = std::fs::write(
                settings_dir.join("settings.json"),
                serde_json::to_string_pretty(&settings).unwrap_or_default(),
            );
        }
    }
}

// ═══════════════════════════════════════════════════════
// 엔트리
// ═══════════════════════════════════════════════════════

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    let args: Vec<String> = std::env::args().collect();

    // --elevate-mkdir <path> → UAC 상승된 상태에서 디렉토리 생성 + 권한 부여 후 즉시 종료
    // 이 분기는 elevate_create_dir()가 ShellExecuteW("runas")로 자기 자신을 재실행할 때 진입한다.
    if let Some(pos) = args.iter().position(|a| a == "--elevate-mkdir") {
        if let Some(dir_str) = args.get(pos + 1) {
            let dir = Path::new(dir_str);
            // 디렉토리 생성
            if let Err(e) = std::fs::create_dir_all(dir) {
                eprintln!("elevate-mkdir: failed to create dir: {}", e);
                std::process::exit(1);
            }
            // 현재 사용자에게 전체 권한 부여
            let username = std::env::var("USERNAME").unwrap_or_else(|_| "Users".into());
            let icacls = std::process::Command::new("icacls")
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

        tracing_subscriber::fmt()
            .with_writer(std::io::stderr)
            .with_env_filter(
                tracing_subscriber::EnvFilter::try_from_default_env()
                    .unwrap_or_else(|_| tracing_subscriber::EnvFilter::new("info")),
            )
            .with_target(false)
            .init();

        let rt = tokio::runtime::Runtime::new().expect("Failed to create tokio runtime");
        rt.block_on(async {
            uninstall::do_silent_uninstall().await;
        });
        return;
    }

    // GUI 모드
    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| tracing_subscriber::EnvFilter::new("info")),
        )
        .with_target(false)
        .init();

    let app_mode = AppMode {
        uninstall: is_uninstall,
        silent: is_silent,
    };

    let installer_state: SharedState = Arc::new(RwLock::new(InstallerState::default()));

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
