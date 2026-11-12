//! 설치·제거 창 (Tauri): 화면 상태와 Tauri 커맨드. 설치 순서는 `saba-chan-installer-core`에 있다

use std::path::Path;
use std::sync::Arc;
use tauri::{AppHandle, Emitter, State};
use tokio::sync::RwLock;
use saba_chan_installer_core::{release, uninstall, InstallHooks, InstallOptions, InstallProgress, ModuleInfo, SetupOptions, VerifiedRelease};
use crate::elevate::elevate_create_dir;

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
    pub release: Option<VerifiedRelease>,
    /// 설정 화면에서 고른 업데이트 채널 (`set_update_channel`). 고르는 칸이 없는 준비 모드는 None: 이 빌드의 채널
    pub channel: Option<String>,
    pub progress: InstallProgress,
    /// 로드된 모듈 메타데이터 (required_extensions 해석용)
    pub module_metadata: Vec<ModuleInfo>,
    /// 모듈·익스텐션을 둘 폴더. 지금 위치와 같으면 None (옮기지 않는다)
    pub components_dir: Option<std::path::PathBuf>,
}

impl Default for InstallerState {
    fn default() -> Self {
        Self {
            install_path: saba_chan_installer_core::default_install_path(),
            language: "en".to_string(),
            create_desktop_shortcut: true,
            create_start_menu_shortcut: true,
            selected_modules: Vec::new(),
            release: None,
            channel: None,
            progress: InstallProgress::default(),
            module_metadata: Vec::new(),
            components_dir: None,
        }
    }
}

/// 모듈·익스텐션 폴더의 지금 위치 (고르는 칸에 보이는 값): 모듈 폴더의 상위
fn current_components_dir() -> std::path::PathBuf {
    let modules = saba_chan_updater_lib::constants::resolve_modules_dir();
    modules.parent().map(Path::to_path_buf).unwrap_or(modules)
}

/// 앱 모드
#[derive(Debug, Clone, Default)]
pub struct AppMode {
    pub uninstall: bool,
    pub silent: bool,
    /// 제거 화면 체크박스의 처음 값 (인자에서)
    pub keep: Option<uninstall::Keep>,
    /// 맥 앱 묶음을 처음 켤 때 준비 (`--setup`): 설치 위치·구성요소는 고르지 않고 모듈만 고른다
    pub setup: bool,
    /// 디스크 이미지·임시 자리에서 열려 준비하지 않고 옮기라고만 알린다
    pub move_first: bool,
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
        "components_dir": s.components_dir.clone().unwrap_or_else(current_components_dir),
        "latest_release_tag": s.release.as_ref().map(|r| r.version.clone()),
        // 채널 칸에 미리 골라 둘 값: 고른 것, 없으면 지금 쓰는 채널(다시 설치할 때), 그것도 없으면 이 설치기의 채널
        "channel": s.channel.clone().unwrap_or_else(|| {
            release::default_channel(&saba_chan_updater_lib::config_file::ConfigFile::default_location())
        }),
        "channels": release::CHANNELS,
        "progress": s.progress,
    }))
}

/// 업데이트 채널을 고른다: 그 채널의 최신 판을 설치하고 이후 업데이트도 그 채널로 받는다.
/// 받아 둔 릴리즈와 모듈 목록은 다른 채널의 것일 수 있어 버린다 (화면이 모듈 목록을 다시 받는다)
#[tauri::command]
async fn set_update_channel(state: State<'_, SharedState>, channel: String) -> Result<(), String> {
    if !release::CHANNELS.contains(&channel.as_str()) {
        return Err(format!("Unknown update channel: {}", channel));
    }
    let mut s = state.write().await;
    if s.channel.as_deref() != Some(channel.as_str()) {
        s.release = None;
        s.module_metadata = Vec::new();
        s.channel = Some(channel);
    }
    Ok(())
}

#[tauri::command]
async fn set_install_path(state: State<'_, SharedState>, path: String) -> Result<String, String> {
    let path = saba_chan_installer_core::dedicated_install_dir(Path::new(&path)).to_string_lossy().into_owned();
    state.write().await.install_path = path.clone();
    Ok(path)
}

/// 모듈·익스텐션 폴더를 고른다. 쓸 수 없는 폴더면 이유 코드(`component_dirs::PathError::code`)로 거절한다
#[tauri::command]
async fn set_components_dir(state: State<'_, SharedState>, path: String) -> Result<String, String> {
    use saba_chan_updater_lib::component_dirs::Layout;
    let path = path.trim();
    let chosen = std::path::PathBuf::from(path);
    let unchanged = path.is_empty() || saba_chan_updater_lib::same_path(&chosen, &current_components_dir());
    if !unchanged {
        let install = std::path::PathBuf::from(state.read().await.install_path.clone());
        saba_chan_installer_core::check_components_dir(&chosen, &Layout::current(), &install).map_err(|e| e.code.to_string())?;
    }
    state.write().await.components_dir = (!unchanged).then(|| chosen.clone());
    Ok(if unchanged { current_components_dir() } else { chosen }.to_string_lossy().into_owned())
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
    let (cached, channel) = {
        let s = state.read().await;
        (s.release.clone(), s.channel.clone())
    };
    let manifest = match cached {
        Some(m) => m,
        None => {
            let m = release::fetch_latest(channel.as_deref()).await?;
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
    let channel = state.read().await.channel.clone();
    let manifest = release::fetch_latest(channel.as_deref()).await?;
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

/// 설치 화면으로 넘어가기 전에 설치 폴더를 준비한다: 쓸 수 없으면 여기서 UAC를 띄운다.
/// 권한 확인이 끝난 뒤에 화면이 그림·설치를 시작하므로, 실패하면 설정 시트에 남아 이유를 보인다
#[tauri::command]
async fn prepare_install(app: AppHandle, state: State<'_, SharedState>) -> Result<(), serde_json::Value> {
    let busy = || serde_json::json!({ "error_code": "busy", "message": "busy" });
    let guard = WorkGuard::try_begin().ok_or_else(busy)?;
    let install_path = std::path::PathBuf::from(state.read().await.install_path.clone());
    // UAC 창을 기다리는 동안 막히므로 블로킹 스레드에서
    let result = tauri::async_runtime::spawn_blocking(move || {
        let _guard = guard;
        saba_chan_installer_core::prepare_install_dir(&install_path, &WindowHooks(app))
    })
    .await
    .map_err(|e| serde_json::json!({ "error_code": "unknown", "message": e.to_string() }))?;
    match result {
        Ok(Some(w)) => {
            // 표시 실패는 설치가 다시 준비하며 완료 화면 경고로 알린다
            tracing::warn!("{}", w);
            Ok(())
        }
        Ok(None) => Ok(()),
        Err((code, message)) => {
            tracing::error!("[{}] {}", code, message);
            Err(serde_json::json!({ "error_code": code, "message": message }))
        }
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
        InstallOptions {
            install_path: s.install_path.clone().into(),
            // 창 인스톨러는 데스크톱 설치다
            profile: saba_chan_installer_core::Profile::Desktop,
            language: s.language.clone(),
            create_desktop_shortcut: s.create_desktop_shortcut,
            create_start_menu_shortcut: s.create_start_menu_shortcut,
            selected_modules: s.selected_modules.clone(),
            release: s.release.clone(),
            channel: s.channel.clone(),
            module_metadata: s.module_metadata.clone(),
            // 설치 폴더에 제거기로 남는다
            uninstaller_exe: std::env::current_exe().ok(),
            components_dir: s.components_dir.clone(),
        }
    };

    let state_clone = state.inner().clone();

    tauri::async_runtime::spawn(async move {
        let _guard = guard;
        let last = saba_chan_installer_core::install(config, &WindowHooks(app)).await;
        state_clone.write().await.progress = last;
    });

    Ok(())
}

/// 맥 앱 묶음 준비 (`--setup`): 고른 모듈·익스텐션과 Python, 명령어를 준비한다. 진행은 설치와 같은 이벤트로 알린다
#[tauri::command]
async fn start_setup(app: AppHandle, state: State<'_, SharedState>) -> Result<(), String> {
    let guard = WorkGuard::try_begin().ok_or_else(|| "busy".to_string())?;
    let config = {
        let s = state.read().await;
        SetupOptions {
            install_path: s.install_path.clone().into(),
            language: s.language.clone(),
            selected_modules: s.selected_modules.clone(),
            release: s.release.clone(),
            module_metadata: s.module_metadata.clone(),
        }
    };
    let state_clone = state.inner().clone();
    tauri::async_runtime::spawn(async move {
        let _guard = guard;
        let last = saba_chan_installer_core::setup(config, &WindowHooks(app)).await;
        state_clone.write().await.progress = last;
    });
    Ok(())
}

/// 창 인스톨러의 설치 진행: 화면에 이벤트로 알리고, 쓸 수 없는 폴더는 UAC로 만든다
struct WindowHooks(AppHandle);

impl InstallHooks for WindowHooks {
    fn progress(&self, progress: &InstallProgress) {
        self.0.emit("install:progress", progress).ok();
    }

    fn elevate_create_dir(&self, dir: &Path) -> Result<(), String> {
        elevate_create_dir(dir)
    }
}

// ═══════════════════════════════════════════════════════
// Tauri 커맨드 — 제거
// ═══════════════════════════════════════════════════════

#[tauri::command]
async fn start_uninstall(app: AppHandle, keep_settings: bool, keep_instances: bool) -> Result<(), String> {
    let guard = WorkGuard::try_begin().ok_or_else(|| "busy".to_string())?;
    let keep = uninstall::Keep { settings: keep_settings, instances: keep_instances };
    tauri::async_runtime::spawn(async move {
        let _guard = guard;
        uninstall::do_uninstall(keep, |p| {
            app.emit("uninstall:progress", p).ok();
        })
        .await;
    });
    Ok(())
}

#[tauri::command]
async fn get_app_mode(mode: State<'_, AppMode>) -> Result<serde_json::Value, String> {
    Ok(serde_json::json!({
        "uninstall": mode.uninstall,
        "silent": mode.silent,
        "keepSettings": mode.keep.is_none_or(|k| k.settings),
        "keepInstances": mode.keep.is_none_or(|k| k.instances),
        "setup": mode.setup,
        "moveFirst": mode.move_first,
    }))
}

#[tauri::command]
async fn launch_app(state: State<'_, SharedState>) -> Result<(), String> {
    let install_path = state.read().await.install_path.clone();
    // 맥 앱 묶음은 묶음째 연다 (Dock·메뉴 막대가 앱으로 잡는다)
    #[cfg(target_os = "macos")]
    if let Some(bundle) = saba_chan_updater_lib::app_bundle::bundle_of(Path::new(&install_path)) {
        std::process::Command::new("open")
            .arg(&bundle)
            .spawn()
            .map_err(|e| format!("Failed to launch Saba-chan: {}", e))?;
        return Ok(());
    }
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
// Tauri 커맨드 — 언어
// ═══════════════════════════════════════════════════════

#[tauri::command]
async fn browse_folder(app: AppHandle) -> Result<Option<String>, String> {
    use tauri_plugin_dialog::DialogExt;
    let path = app
        .dialog()
        .file()
        .set_title("Select install directory")
        .blocking_pick_folder();
    Ok(path.map(|p| saba_chan_installer_core::dedicated_install_dir(Path::new(&p.to_string())).to_string_lossy().into_owned()))
}

/// 모듈·익스텐션 폴더 고르기 (고른 폴더 아래 `modules`·`extensions`를 만든다)
#[tauri::command]
async fn browse_components_folder(app: AppHandle) -> Result<Option<String>, String> {
    use tauri_plugin_dialog::DialogExt;
    let path = app.dialog().file().set_title("Select a folder for modules and extensions").blocking_pick_folder();
    Ok(path.map(|p| p.to_string()))
}


/// 설치 창을 띄운다. 제거(`uninstall`)·준비(`setup`) 창도 같은 화면이다
pub(crate) fn run(app_mode: AppMode) {
    let uninstall = app_mode.uninstall;
    let mut initial = InstallerState::default();
    if app_mode.setup {
        // 준비는 이 앱 묶음 자리 그대로다 (실행 파일이 이미 있다)
        initial.install_path = std::env::current_exe()
            .ok()
            .and_then(|p| p.parent().map(|d| d.to_string_lossy().into_owned()))
            .unwrap_or_default();
    }
    if uninstall {
        // 제거 화면에 보이는 경로 = 실제로 지울 폴더 (제거기 자신이 든 설치 폴더)
        initial.install_path = uninstall::target_dir().map(|d| d.to_string_lossy().into_owned()).unwrap_or_default();
    }
    let installer_state: SharedState = Arc::new(RwLock::new(initial));
    let setup = app_mode.setup;

    tauri::Builder::default()
        .plugin(tauri_plugin_shell::init())
        .plugin(tauri_plugin_dialog::init())
        .manage(installer_state)
        .manage(app_mode)
        // 설치·제거가 도는 동안은 닫지 않는다: 같은 프로세스의 작업이라 창을 닫으면 반쯤 된 채 끊긴다
        .on_window_event(|_, event| {
            if let tauri::WindowEvent::CloseRequested { api, .. } = event {
                if WORK_BUSY.load(std::sync::atomic::Ordering::Acquire) {
                    api.prevent_close();
                }
            }
        })
        .setup(move |app| {
            // 페이지가 창을 보인다 (installer.js `show()`)
            let title = if uninstall { "Saba-chan — Uninstaller" } else if setup { "Saba-chan" } else { "Saba-chan Installer" };
            crate::main_window(app.handle(), crate::Page::Install, title)?;
            Ok(())
        })
        .invoke_handler(tauri::generate_handler![
            get_installer_state,
            set_install_path,
            set_components_dir,
            set_language,
            set_shortcut_options,
            set_update_channel,
            set_selected_modules,
            get_available_modules,
            fetch_latest_release,
            prepare_install,
            start_install,
            start_setup,
            start_uninstall,
            get_app_mode,
            crate::get_preferred_language,
            browse_folder,
            browse_components_folder,
            launch_app,
            crate::backdrop::get_window_backdrop,
        ])
        .run(crate::context())
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
}
