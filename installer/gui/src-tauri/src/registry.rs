//! Windows 레지스트리 관리 — 프로그램 추가/제거 등록
//!
//! `HKCU\Software\Microsoft\Windows\CurrentVersion\Uninstall\Saba-chan` 키에
//! 인스톨러 정보를 등록하여 [설정 > 앱 > 설치된 앱] 목록에 표시합니다.

#[cfg(target_os = "windows")]
use std::path::Path;

#[cfg(target_os = "windows")]
const UNINSTALL_KEY: &str =
    r"Software\Microsoft\Windows\CurrentVersion\Uninstall\Saba-chan";

/// 프로그램 추가/제거 레지스트리 등록
#[cfg(target_os = "windows")]
pub fn register_uninstall_entry(
    install_dir: &Path,
    version: &str,
) -> anyhow::Result<()> {
    use winreg::enums::*;
    use winreg::RegKey;

    let hkcu = RegKey::predef(HKEY_CURRENT_USER);
    let (key, _) = hkcu.create_subkey(UNINSTALL_KEY)?;

    let display_name = "Saba-chan";
    let install_location = install_dir.to_string_lossy().to_string();

    // 인스톨러 자체 경로 (언인스톨에 사용)
    let uninstall_string = if let Ok(exe) = std::env::current_exe() {
        format!("\"{}\" --uninstall", exe.to_string_lossy())
    } else {
        String::new()
    };

    // 인스톨러를 설치 디렉토리에 복사 (제거 시 사용)
    copy_installer_to_install_dir(install_dir);

    // 설치 디렉토리의 인스톨러로 언인스톨 커맨드 설정
    let installer_copy = install_dir.join("saba-chan-installer.exe");
    let uninstall_cmd = if installer_copy.exists() {
        format!("\"{}\" --uninstall", installer_copy.to_string_lossy())
    } else {
        uninstall_string
    };

    key.set_value("DisplayName", &display_name)?;
    key.set_value("DisplayVersion", &version.trim_start_matches('v'))?;
    key.set_value("Publisher", &"WareAoba")?;
    key.set_value("InstallLocation", &install_location)?;
    key.set_value("UninstallString", &uninstall_cmd)?;
    key.set_value("DisplayIcon", &format!("{}", install_dir.join("saba-chan-gui.exe").to_string_lossy()))?;
    key.set_value("NoModify", &1u32)?;
    key.set_value("NoRepair", &1u32)?;

    // 설치 크기 추정 (MB)
    if let Ok(size) = estimate_dir_size(install_dir) {
        let size_kb = (size / 1024) as u32;
        key.set_value("EstimatedSize", &size_kb)?;
    }

    tracing::info!(
        "[Registry] Registered uninstall entry: {} at {}",
        display_name,
        install_location
    );

    Ok(())
}

/// 레지스트리에서 설치 경로 읽기
#[cfg(target_os = "windows")]
pub fn get_install_location() -> Option<String> {
    use winreg::enums::*;
    use winreg::RegKey;

    let hkcu = RegKey::predef(HKEY_CURRENT_USER);
    let key = hkcu.open_subkey(UNINSTALL_KEY).ok()?;
    key.get_value::<String, _>("InstallLocation").ok()
}

/// 레지스트리에서 표시 버전 읽기
#[cfg(target_os = "windows")]
pub fn get_installed_version() -> Option<String> {
    use winreg::enums::*;
    use winreg::RegKey;

    let hkcu = RegKey::predef(HKEY_CURRENT_USER);
    let key = hkcu.open_subkey(UNINSTALL_KEY).ok()?;
    key.get_value::<String, _>("DisplayVersion").ok()
}

/// 프로그램 추가/제거 레지스트리 삭제
#[cfg(target_os = "windows")]
pub fn remove_uninstall_entry() -> anyhow::Result<()> {
    use winreg::enums::*;
    use winreg::RegKey;

    let hkcu = RegKey::predef(HKEY_CURRENT_USER);
    match hkcu.delete_subkey_all(UNINSTALL_KEY) {
        Ok(_) => {
            tracing::info!("[Registry] Removed uninstall entry");
            Ok(())
        }
        Err(e) => {
            // 키가 존재하지 않으면 무시
            if e.kind() == std::io::ErrorKind::NotFound {
                tracing::info!("[Registry] Uninstall entry not found (already clean)");
                Ok(())
            } else {
                Err(anyhow::anyhow!("Failed to remove registry key: {}", e))
            }
        }
    }
}

/// 인스톨러를 설치 디렉토리에 복사 (언인스톨 시 사용)
#[cfg(target_os = "windows")]
fn copy_installer_to_install_dir(install_dir: &Path) {
    if let Ok(exe) = std::env::current_exe() {
        let dest = install_dir.join("saba-chan-installer.exe");
        if exe != dest {
            let _ = std::fs::copy(&exe, &dest);
        }
    }
}

/// 디렉토리 크기 추정 (바이트)
#[cfg(target_os = "windows")]
fn estimate_dir_size(dir: &Path) -> anyhow::Result<u64> {
    let mut total: u64 = 0;
    if dir.is_dir() {
        for entry in std::fs::read_dir(dir)? {
            let entry = entry?;
            let path = entry.path();
            if path.is_dir() {
                total += estimate_dir_size(&path).unwrap_or(0);
            } else {
                total += entry.metadata().map(|m| m.len()).unwrap_or(0);
            }
        }
    }
    Ok(total)
}

// Non-Windows stubs
#[cfg(not(target_os = "windows"))]
pub fn register_uninstall_entry(
    _install_dir: &std::path::Path,
    _version: &str,
) -> anyhow::Result<()> {
    Ok(())
}

#[cfg(not(target_os = "windows"))]
pub fn get_install_location() -> Option<String> {
    None
}

#[cfg(not(target_os = "windows"))]
pub fn get_installed_version() -> Option<String> {
    None
}

#[cfg(not(target_os = "windows"))]
pub fn remove_uninstall_entry() -> anyhow::Result<()> {
    Ok(())
}
