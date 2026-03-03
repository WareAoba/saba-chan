//! Windows 바로가기(.lnk) 생성 — 바탕화면 및 시작 메뉴
//!
//! COM IShellLink를 사용하지 않고, PowerShell 기반으로 .lnk 파일을 생성합니다.
//! (안정적이고 추가 COM 초기화 불필요)
//!
//! 경로 해석에는 `SHGetKnownFolderPath` Windows API를 사용하여
//! OneDrive 폴더 리디렉션이나 사용자 지정 경로를 올바르게 처리합니다.

#[cfg(target_os = "windows")]
use std::path::Path;

// ── Windows Known Folder GUIDs ──────────────────────────

/// `{B4BFCC3A-DB2C-424C-B029-7FE99A87C641}` — 바탕화면
#[cfg(target_os = "windows")]
const FOLDERID_DESKTOP: windows_sys::core::GUID = windows_sys::core::GUID {
    data1: 0xB4BFCC3A,
    data2: 0xDB2C,
    data3: 0x424C,
    data4: [0xB0, 0x29, 0x7F, 0xE9, 0x9A, 0x87, 0xC6, 0x41],
};

/// `{A77F5D68-2E2E-44C3-A6A2-ABA601054A51}` — 시작 메뉴 > 프로그램
#[cfg(target_os = "windows")]
const FOLDERID_PROGRAMS: windows_sys::core::GUID = windows_sys::core::GUID {
    data1: 0xA77F5D68,
    data2: 0x2E2E,
    data3: 0x44C3,
    data4: [0xA6, 0xA2, 0xAB, 0xA6, 0x01, 0x05, 0x4A, 0x51],
};

/// 바탕화면에 바로가기 생성
#[cfg(target_os = "windows")]
pub fn create_desktop_shortcut(target_exe: &Path, name: &str) -> anyhow::Result<()> {
    let desktop = get_desktop_path()?;
    let lnk_path = desktop.join(format!("{}.lnk", name));
    create_shortcut_via_powershell(target_exe, &lnk_path, name)?;
    tracing::info!("[Shortcut] Desktop: {:?}", lnk_path);
    Ok(())
}

/// 시작 메뉴에 바로가기 생성
#[cfg(target_os = "windows")]
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
#[cfg(target_os = "windows")]
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
#[cfg(target_os = "windows")]
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
#[cfg(target_os = "windows")]
fn create_shortcut_via_powershell(
    target: &Path,
    lnk_path: &Path,
    description: &str,
) -> anyhow::Result<()> {
    let target_str = target.to_string_lossy();
    let lnk_str = lnk_path.to_string_lossy();
    let working_dir = target
        .parent()
        .map(|p| p.to_string_lossy().to_string())
        .unwrap_or_default();

    let ps_script = format!(
        r#"$ws = New-Object -ComObject WScript.Shell; $sc = $ws.CreateShortcut('{}'); $sc.TargetPath = '{}'; $sc.WorkingDirectory = '{}'; $sc.Description = '{}'; $sc.Save()"#,
        lnk_str, target_str, working_dir, description,
    );

    let output = std::process::Command::new("powershell")
        .args(["-NoProfile", "-NonInteractive", "-Command", &ps_script])
        .output()?;

    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        anyhow::bail!("PowerShell shortcut creation failed: {}", stderr);
    }

    Ok(())
}

/// `SHGetKnownFolderPath` 로 Known Folder의 실제 경로를 가져온다.
/// OneDrive 백업으로 인한 리디렉션, 사용자 지정 폴더 위치 등을 올바르게 처리.
#[cfg(target_os = "windows")]
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

#[cfg(target_os = "windows")]
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

#[cfg(target_os = "windows")]
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

// ── Non-Windows stubs ───────────────────────────────────

#[cfg(not(target_os = "windows"))]
pub fn create_desktop_shortcut(
    _target_exe: &std::path::Path,
    _name: &str,
) -> anyhow::Result<()> {
    Ok(())
}

#[cfg(not(target_os = "windows"))]
pub fn create_start_menu_shortcut(
    _target_exe: &std::path::Path,
    _name: &str,
) -> anyhow::Result<()> {
    Ok(())
}

#[cfg(not(target_os = "windows"))]
pub fn remove_desktop_shortcut(_name: &str) -> anyhow::Result<()> {
    Ok(())
}

#[cfg(not(target_os = "windows"))]
pub fn remove_start_menu_shortcut(_name: &str) -> anyhow::Result<()> {
    Ok(())
}
