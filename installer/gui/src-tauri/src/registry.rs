//! Windows 레지스트리 관리 — 프로그램 추가/제거 등록
//!
//! `HKCU\Software\Microsoft\Windows\CurrentVersion\Uninstall\Saba-chan*` 키에
//! 인스톨러 정보를 등록하여 [설정 > 앱 > 설치된 앱] 목록에 표시합니다.
//! 키는 설치 폴더마다 따로다 (`saba_chan_updater_lib::uninstall_entry`): 다른 설치본의 항목을 덮거나 지우지 않는다.

#[cfg(target_os = "windows")]
use std::path::Path;

#[cfg(target_os = "windows")]
use saba_chan_updater_lib::uninstall_entry;

/// 프로그램 추가/제거 레지스트리 등록
#[cfg(target_os = "windows")]
pub fn register_uninstall_entry(
    install_dir: &Path,
    version: &str,
) -> anyhow::Result<()> {
    use winreg::enums::*;
    use winreg::RegKey;

    let hkcu = RegKey::predef(HKEY_CURRENT_USER);
    let key_name = uninstall_entry::pick_key_name(&uninstall_entry::entries(), install_dir);
    let (key, _) = hkcu.create_subkey(uninstall_entry::key_path(&key_name))?;

    // 두 번째 설치본부터는 목록에서 구분되게 폴더를 붙인다
    let display_name = if key_name == uninstall_entry::BASE_KEY {
        "Saba-chan".to_string()
    } else {
        format!("Saba-chan ({})", install_dir.display())
    };
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
    key.set_value("Publisher", &saba_chan_updater_lib::constants::PUBLISHER)?;
    key.set_value("InstallLocation", &install_location)?;
    key.set_value("UninstallString", &uninstall_cmd)?;
    let icon = saba_chan_updater_lib::find_gui_exe(install_dir).unwrap_or_else(|| install_dir.join("saba-chan-gui.exe"));
    key.set_value("DisplayIcon", &icon.to_string_lossy().to_string())?;
    key.set_value("NoModify", &1u32)?;
    key.set_value("NoRepair", &1u32)?;

    // 설치 크기 추정 (MB)
    if let Ok(size) = estimate_dir_size(install_dir) {
        let size_kb = (size / 1024) as u32;
        key.set_value("EstimatedSize", &size_kb)?;
    }

    tracing::info!(
        "[Registry] Registered uninstall entry {} ({}) at {}",
        key_name,
        display_name,
        install_location
    );

    Ok(())
}

/// 이 설치 폴더의 프로그램 추가/제거 항목 삭제 (다른 설치본 항목은 그대로)
#[cfg(target_os = "windows")]
pub fn remove_uninstall_entry(install_dir: &Path) -> anyhow::Result<()> {
    use winreg::enums::*;
    use winreg::RegKey;

    let Some(name) = uninstall_entry::find_key_name(&uninstall_entry::entries(), install_dir) else {
        tracing::info!("[Registry] No uninstall entry for {} (already clean)", install_dir.display());
        return Ok(());
    };
    RegKey::predef(HKEY_CURRENT_USER)
        .delete_subkey_all(uninstall_entry::key_path(&name))
        .map_err(|e| anyhow::anyhow!("Failed to remove registry key {}: {}", name, e))?;
    tracing::info!("[Registry] Removed uninstall entry {}", name);
    Ok(())
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
pub fn remove_uninstall_entry(_install_dir: &std::path::Path) -> anyhow::Result<()> {
    Ok(())
}
