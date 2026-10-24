//! Saba-chan Installer — 설치/제거 GUI (Tauri)
//!
//! ## 실행 모드
//! - `saba-chan-installer`                     → GUI 설치 모드 (5-page wizard)
//! - `saba-chan-installer --uninstall`          → GUI 제거 모드
//! - `saba-chan-installer --uninstall --silent` → 명령창 제거 (GUI 없음)
//! - 둘 다 `--remove-settings`·`--remove-instances`로 지울 것을 고른다. 없으면 설정·인스턴스를 남긴다
//!
//! 설치 순서는 `saba-chan-installer-core`에 있다 (`saba-chan-cli install`과 같은 코드)

use std::path::Path;
use std::sync::Arc;
use tauri::{AppHandle, Emitter, Manager, State};
use tokio::sync::RwLock;
use saba_chan_installer_core::{release, uninstall, InstallHooks, InstallOptions, InstallProgress, ModuleInfo, VerifiedRelease};

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
        "progress": s.progress,
    }))
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
        InstallOptions {
            install_path: s.install_path.clone().into(),
            // 창 인스톨러는 데스크톱 설치다
            profile: saba_chan_installer_core::Profile::Desktop,
            language: s.language.clone(),
            create_desktop_shortcut: s.create_desktop_shortcut,
            create_start_menu_shortcut: s.create_start_menu_shortcut,
            selected_modules: s.selected_modules.clone(),
            release: s.release.clone(),
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

/// Windows 명령줄 인자 하나로 따옴표를 씌운다 (CommandLineToArgvW 규칙: `"` 앞과 끝의 `\`는 두 배로)
#[cfg_attr(not(target_os = "windows"), allow(dead_code))]
fn quote_arg(arg: &str) -> String {
    let mut out = String::from("\"");
    let mut backslashes = 0;
    for c in arg.chars() {
        match c {
            '\\' => backslashes += 1,
            '"' => {
                out.extend(std::iter::repeat_n('\\', backslashes * 2 + 1));
                out.push('"');
                backslashes = 0;
            }
            _ => {
                out.extend(std::iter::repeat_n('\\', backslashes));
                out.push(c);
                backslashes = 0;
            }
        }
    }
    out.extend(std::iter::repeat_n('\\', backslashes * 2));
    out.push('"');
    out
}

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
    let params = to_wide(&format!("--elevate-mkdir {} --grant {}", quote_arg(&dir.to_string_lossy()), quote_arg(&user)));

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
        || saba_chan_installer_core::is_empty_dir(dir)
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
    Ok(saba_chan_installer_core::preferred_language())
}

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

// ═══════════════════════════════════════════════════════
// 엔트리
// ═══════════════════════════════════════════════════════

pub use saba_chan_installer_core::log_file_path;

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
            let icacls = saba_chan_updater_lib::process::hide_console(&mut std::process::Command::new(saba_chan_updater_lib::process::system_tool("icacls")))
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
        let code = rt.block_on(uninstall::do_silent_uninstall(uninstall::keep_from_args(&args)));
        if code != 0 {
            std::process::exit(code);
        }
        return;
    }

    // GUI 모드
    init_logging();

    let app_mode = AppMode {
        uninstall: is_uninstall,
        silent: is_silent,
        keep: is_uninstall.then(|| uninstall::keep_from_args(&args)),
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
            // 작업 표시줄·제목 표시줄 아이콘을 창 DPI에 맞는 크기로. 배율이 바뀌면 다시 건다
            #[cfg(windows)]
            if let Some(win) = app.get_webview_window("main") {
                if let Ok(hwnd) = win.hwnd() {
                    saba_chan_updater_lib::window_icon::apply_exe_icon(hwnd.0 as isize);
                }
                let w = win.clone();
                win.on_window_event(move |event| {
                    if let (tauri::WindowEvent::ScaleFactorChanged { .. }, Ok(hwnd)) = (event, w.hwnd()) {
                        saba_chan_updater_lib::window_icon::apply_exe_icon(hwnd.0 as isize);
                    }
                });
            }
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
            set_components_dir,
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
            browse_components_folder,
            launch_app,
        ])
        .run(tauri::generate_context!())
        .expect("error while running tauri application");
}

#[cfg(test)]
mod tests {
    // L15(a): `\`로 끝나는 경로는 그대로 따옴표로 감싸면 `\"`가 되어 다음 인자까지 먹는다
    #[test]
    fn elevation_arguments_survive_trailing_backslashes_and_quotes() {
        assert_eq!(quote_arg(r"C:\Program Files\Saba-chan"), r#""C:\Program Files\Saba-chan""#);
        assert_eq!(quote_arg(r"D:\"), r#""D:\\""#);
        assert_eq!(quote_arg(r#"a\"b"#), r#""a\\\"b""#);
    }

    use super::*;

    #[test]
    fn only_one_install_or_uninstall_runs_at_a_time() {
        let first = WorkGuard::try_begin().expect("nothing is running yet");
        assert!(WorkGuard::try_begin().is_none(), "a second click must not start another run");
        drop(first);
        assert!(WorkGuard::try_begin().is_some(), "finishing frees the slot");
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
