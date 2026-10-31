//! Saba-chan Installer — 설치·제거·업데이트 적용을 모두 하는 한 프로그램 (Tauri)
//!
//! ## 실행 모드
//! - `saba-chan-installer`                     → 설치 창 (5-page wizard)
//! - `saba-chan-installer --uninstall`          → 제거 창
//! - `saba-chan-installer --uninstall --silent` → 명령창 제거 (창 없음)
//! - 둘 다 `--remove-settings`·`--remove-instances`로 지울 것을 고른다. 없으면 설정·인스턴스를 남긴다
//! - `saba-chan-installer --apply ...`          → 데몬이 받아 둔 업데이트 적용 (`apply.rs`, 데몬이 띄운다)
//! - `saba-chan-installer --elevate-mkdir ...`  → 관리자 권한으로 설치 폴더 만들기 (`elevate.rs`, 자기 자신이 띄운다)
//!
//! `window` 기능 없이 빌드하면(화면 없는 리눅스 서버용) 창을 띄우지 않는다: 적용은 창 없이, 제거는 명령창으로 한다.
//! 설치 순서는 `saba-chan-installer-core`에 있다 (`saba-chan-cli install`과 같은 코드)

mod apply;
mod elevate;
#[cfg(feature = "window")]
mod install_window;

use saba_chan_installer_core::uninstall;

/// 인자로 고른 실행 모드
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Mode {
    Install,
    Uninstall,
    SilentUninstall,
    Apply,
    ElevateMkdir,
}

impl Mode {
    /// `--relaunch` 뒤는 다시 띄울 프로그램의 인자라 보지 않는다
    fn from_args(args: &[String]) -> Mode {
        let own = &args[..args.iter().position(|a| a == "--relaunch").unwrap_or(args.len())];
        let has = |flag: &str| own.iter().any(|a| a == flag);
        if has("--elevate-mkdir") {
            Mode::ElevateMkdir
        } else if has("--apply") {
            Mode::Apply
        } else if has("--uninstall") && has("--silent") {
            Mode::SilentUninstall
        } else if has("--uninstall") {
            Mode::Uninstall
        } else {
            Mode::Install
        }
    }
}

// ═══════════════════════════════════════════════════════
// 창 (설치·제거 창과 업데이트 창이 같이 쓴다)
// ═══════════════════════════════════════════════════════

/// 창에 띄우는 화면 (`installer/gui/src`)
#[cfg(feature = "window")]
#[derive(Debug, Clone, Copy)]
enum Page {
    /// 설치·제거 (`index.html`)
    Install,
    /// 업데이트 적용 (`update.html`)
    Update,
}

/// 프레임 없는 고정 크기 메인 창을 만든다 (보이지 않은 채로). 작업 표시줄·제목 표시줄 아이콘은 창 DPI에 맞춘다
#[cfg(feature = "window")]
fn main_window(app: &tauri::AppHandle, page: Page, title: &str) -> tauri::Result<tauri::WebviewWindow> {
    use tauri::window::Color;
    let (url, height, background) = match page {
        Page::Install => ("index.html", 520.0, Color(0xf5, 0xf6, 0xfa, 0xff)),
        Page::Update => ("update.html", 500.0, Color(0x66, 0x7e, 0xea, 0xff)),
    };
    let win = tauri::WebviewWindowBuilder::new(app, "main", tauri::WebviewUrl::App(url.into()))
        .title(title)
        .inner_size(750.0, height)
        .resizable(false)
        .decorations(false)
        .center()
        .visible(false)
        .background_color(background)
        .build()?;
    // 배율이 바뀌면 다시 건다
    #[cfg(windows)]
    {
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
    Ok(win)
}

/// 화면 언어 — 앱 설정(`config.json`의 `gui.language`) → 시스템 로케일 → "en"
#[cfg(feature = "window")]
#[tauri::command]
async fn get_preferred_language() -> Result<String, String> {
    Ok(saba_chan_installer_core::preferred_language())
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

/// 명령창 제거. 창에서 띄웠으면(Windows) 부른 명령창에 붙거나 새 명령창을 연다
fn silent_uninstall(args: &[String]) -> i32 {
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
    rt.block_on(uninstall::do_silent_uninstall(uninstall::keep_from_args(args)))
}

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    let args: Vec<String> = std::env::args().collect();

    match Mode::from_args(&args) {
        Mode::ElevateMkdir => std::process::exit(elevate::run(&args)),
        Mode::Apply => apply::run(&args),
        Mode::SilentUninstall => exit_on_failure(silent_uninstall(&args)),
        // 창 없는 빌드: 화면이 없으니 제거는 명령창으로 한다
        #[cfg(not(feature = "window"))]
        Mode::Uninstall => exit_on_failure(silent_uninstall(&args)),
        #[cfg(not(feature = "window"))]
        Mode::Install => {
            eprintln!("This build of the Saba-chan installer has no window. Install with `saba-chan-cli install`.");
            std::process::exit(1);
        }
        #[cfg(feature = "window")]
        Mode::Uninstall => {
            init_logging();
            install_window::run(true, Some(uninstall::keep_from_args(&args)));
        }
        #[cfg(feature = "window")]
        Mode::Install => {
            init_logging();
            install_window::run(false, None);
        }
    }
}

fn exit_on_failure(code: i32) {
    if code != 0 {
        std::process::exit(code);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn args(list: &[&str]) -> Vec<String> {
        std::iter::once("saba-chan-installer").chain(list.iter().copied()).map(str::to_string).collect()
    }

    /// 한 프로그램이 인자로 설치·제거·업데이트 적용을 고른다. 다시 띄울 프로그램의 인자(`--relaunch` 뒤)는 보지 않는다
    #[test]
    fn the_arguments_pick_install_uninstall_or_apply() {
        assert_eq!(Mode::from_args(&args(&[])), Mode::Install);
        assert_eq!(Mode::from_args(&args(&["--uninstall"])), Mode::Uninstall);
        assert_eq!(Mode::from_args(&args(&["--uninstall", "--remove-settings"])), Mode::Uninstall);
        assert_eq!(Mode::from_args(&args(&["--uninstall", "--silent"])), Mode::SilentUninstall);
        assert_eq!(
            Mode::from_args(&args(&["--apply", "--wait-pid", "7", "--launch-token", "t", "--relaunch", "/opt/saba/saba-chan-gui"])),
            Mode::Apply
        );
        assert_eq!(Mode::from_args(&args(&["--elevate-mkdir", r"C:\Saba", "--grant-sid", "S-1-5-21-1"])), Mode::ElevateMkdir);
        assert_eq!(
            Mode::from_args(&args(&["--relaunch", "/opt/saba/saba-chan-cli", "--uninstall"])),
            Mode::Install,
            "arguments after --relaunch belong to the relaunched program"
        );
    }
}
