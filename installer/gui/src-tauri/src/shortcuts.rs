//! Windows 바로가기(.lnk) 생성 — 바탕화면 및 시작 메뉴
//!
//! COM IShellLink를 사용하지 않고, PowerShell 기반으로 .lnk 파일을 생성합니다.
//! (안정적이고 추가 COM 초기화 불필요)

#[cfg(target_os = "windows")]
use std::path::Path;

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

#[cfg(target_os = "windows")]
fn get_desktop_path() -> anyhow::Result<std::path::PathBuf> {
    if let Ok(profile) = std::env::var("USERPROFILE") {
        return Ok(std::path::PathBuf::from(profile).join("Desktop"));
    }
    anyhow::bail!("Could not determine Desktop path")
}

#[cfg(target_os = "windows")]
fn get_start_menu_path() -> anyhow::Result<std::path::PathBuf> {
    if let Ok(appdata) = std::env::var("APPDATA") {
        return Ok(std::path::PathBuf::from(appdata)
            .join("Microsoft")
            .join("Windows")
            .join("Start Menu")
            .join("Programs"));
    }
    anyhow::bail!("Could not determine Start Menu path")
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
