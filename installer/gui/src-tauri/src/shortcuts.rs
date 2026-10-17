//! Windows 바로가기(.lnk) 생성 — 바탕화면 및 시작 메뉴
//!
//! COM IShellLink를 사용하지 않고, PowerShell 기반으로 .lnk 파일을 생성합니다.
//! (안정적이고 추가 COM 초기화 불필요)
//!
//! 경로 해석에는 `SHGetKnownFolderPath` Windows API를 사용하여
//! OneDrive 폴더 리디렉션이나 사용자 지정 경로를 올바르게 처리합니다.

use std::path::Path;

/// 설치 언어별 앱 표시 이름. 언어 변경·제거 시 모든 이름의 바로가기를 정리한다.
pub const ALL_SHORTCUT_NAMES: &[&str] = &["Saba-chan", "사바쨩", "サバちゃん"];

/// 설치 언어에 맞는 앱 표시 이름을 반환한다.
pub fn localized_app_name(language: &str) -> &'static str {
    match language {
        "ko" => "사바쨩",
        "ja" => "サバちゃん",
        _ => "Saba-chan",
    }
}

// ── Windows Known Folder GUIDs ──────────────────────────

/// `{B4BFCC3A-DB2C-424C-B029-7FE99A87C641}` — 바탕화면
const FOLDERID_DESKTOP: windows_sys::core::GUID = windows_sys::core::GUID {
    data1: 0xB4BFCC3A,
    data2: 0xDB2C,
    data3: 0x424C,
    data4: [0xB0, 0x29, 0x7F, 0xE9, 0x9A, 0x87, 0xC6, 0x41],
};

/// `{A77F5D68-2E2E-44C3-A6A2-ABA601054A51}` — 시작 메뉴 > 프로그램
const FOLDERID_PROGRAMS: windows_sys::core::GUID = windows_sys::core::GUID {
    data1: 0xA77F5D68,
    data2: 0x2E2E,
    data3: 0x44C3,
    data4: [0xA6, 0xA2, 0xAB, 0xA6, 0x01, 0x05, 0x4A, 0x51],
};

/// 바탕화면에 바로가기 생성
pub fn create_desktop_shortcut(target_exe: &Path, name: &str) -> anyhow::Result<()> {
    let desktop = get_desktop_path()?;
    let lnk_path = desktop.join(format!("{}.lnk", name));
    create_shortcut_via_powershell(target_exe, &lnk_path, name)?;
    tracing::info!("[Shortcut] Desktop: {:?}", lnk_path);
    Ok(())
}

/// 시작 메뉴에 바로가기 생성
pub fn create_start_menu_shortcut(target_exe: &Path, name: &str) -> anyhow::Result<()> {
    let start_menu = get_start_menu_path()?;
    let program_dir = start_menu.join(name);
    std::fs::create_dir_all(&program_dir)?;
    let lnk_path = program_dir.join(format!("{}.lnk", name));
    create_shortcut_via_powershell(target_exe, &lnk_path, name)?;
    tracing::info!("[Shortcut] Start Menu: {:?}", lnk_path);
    Ok(())
}

/// 바탕화면 바로가기 제거
pub fn remove_desktop_shortcut(name: &str) -> anyhow::Result<()> {
    let desktop = get_desktop_path()?;
    let lnk_path = desktop.join(format!("{}.lnk", name));
    if lnk_path.exists() {
        std::fs::remove_file(&lnk_path)?;
        tracing::info!("[Shortcut] Removed desktop shortcut: {:?}", lnk_path);
    }
    Ok(())
}

/// 시작 메뉴 바로가기 제거
pub fn remove_start_menu_shortcut(name: &str) -> anyhow::Result<()> {
    let start_menu = get_start_menu_path()?;
    let program_dir = start_menu.join(name);
    if program_dir.exists() {
        std::fs::remove_dir_all(&program_dir)?;
        tracing::info!(
            "[Shortcut] Removed start menu folder: {:?}",
            program_dir
        );
    }
    Ok(())
}

/// PowerShell WScript.Shell으로 .lnk 생성
fn create_shortcut_via_powershell(
    target: &Path,
    lnk_path: &Path,
    description: &str,
) -> anyhow::Result<()> {
    // 경로는 환경 변수로 넘긴다. 스크립트 문자열에 넣으면 경로의 `'` 가 명령을 깨뜨린다.
    const PS_SCRIPT: &str = "$ws = New-Object -ComObject WScript.Shell; \
        $sc = $ws.CreateShortcut($env:SABA_LNK_PATH); \
        $sc.TargetPath = $env:SABA_LNK_TARGET; \
        $sc.WorkingDirectory = $env:SABA_LNK_WORKDIR; \
        $sc.Description = $env:SABA_LNK_DESC; \
        $sc.Save()";

    let working_dir = target.parent().unwrap_or(target);
    let output = std::process::Command::new("powershell")
        .args(["-NoProfile", "-NonInteractive", "-Command", PS_SCRIPT])
        .env("SABA_LNK_PATH", lnk_path)
        .env("SABA_LNK_TARGET", target)
        .env("SABA_LNK_WORKDIR", working_dir)
        .env("SABA_LNK_DESC", description)
        .output()?;

    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        anyhow::bail!("PowerShell shortcut creation failed: {}", stderr);
    }

    Ok(())
}

/// `SHGetKnownFolderPath` 로 Known Folder의 실제 경로를 가져온다.
/// OneDrive 백업으로 인한 리디렉션, 사용자 지정 폴더 위치 등을 올바르게 처리.
fn get_known_folder_path(
    folder_id: &windows_sys::core::GUID,
) -> anyhow::Result<std::path::PathBuf> {
    unsafe {
        let mut path_ptr: *mut u16 = std::ptr::null_mut();
        let hr = windows_sys::Win32::UI::Shell::SHGetKnownFolderPath(
            folder_id as *const _,
            0,     // dwFlags — 기본값
            std::ptr::null_mut(),  // hToken — null (현재 사용자)
            &mut path_ptr,
        );
        if hr != 0 || path_ptr.is_null() {
            anyhow::bail!("SHGetKnownFolderPath failed: HRESULT {:#010X}", hr);
        }
        let len = (0..).take_while(|&i| *path_ptr.add(i) != 0).count();
        let path_str =
            String::from_utf16_lossy(std::slice::from_raw_parts(path_ptr, len));
        windows_sys::Win32::System::Com::CoTaskMemFree(path_ptr as _);
        Ok(std::path::PathBuf::from(path_str))
    }
}

fn get_desktop_path() -> anyhow::Result<std::path::PathBuf> {
    match get_known_folder_path(&FOLDERID_DESKTOP) {
        Ok(p) => {
            tracing::debug!("[Shortcut] Desktop path (API): {:?}", p);
            Ok(p)
        }
        Err(e) => {
            tracing::warn!(
                "[Shortcut] SHGetKnownFolderPath(Desktop) failed: {}, \
                 falling back to USERPROFILE",
                e
            );
            std::env::var("USERPROFILE")
                .map(|p| std::path::PathBuf::from(p).join("Desktop"))
                .map_err(|_| anyhow::anyhow!("Could not determine Desktop path"))
        }
    }
}

fn get_start_menu_path() -> anyhow::Result<std::path::PathBuf> {
    match get_known_folder_path(&FOLDERID_PROGRAMS) {
        Ok(p) => {
            tracing::debug!("[Shortcut] Start Menu Programs path (API): {:?}", p);
            Ok(p)
        }
        Err(e) => {
            tracing::warn!(
                "[Shortcut] SHGetKnownFolderPath(Programs) failed: {}, \
                 falling back to APPDATA",
                e
            );
            std::env::var("APPDATA")
                .map(|a| {
                    std::path::PathBuf::from(a)
                        .join("Microsoft")
                        .join("Windows")
                        .join("Start Menu")
                        .join("Programs")
                })
                .map_err(|_| anyhow::anyhow!("Could not determine Start Menu path"))
        }
    }
}

