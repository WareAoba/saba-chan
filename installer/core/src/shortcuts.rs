//! Windows 바로가기(.lnk) 생성 — 바탕화면 및 시작 메뉴
//!
//! COM IShellLink를 사용하지 않고, PowerShell 기반으로 .lnk 파일을 생성합니다.
//! (안정적이고 추가 COM 초기화 불필요)
//!
//! 경로 해석에는 `SHGetKnownFolderPath` Windows API를 사용하여
//! OneDrive 폴더 리디렉션이나 사용자 지정 경로를 올바르게 처리합니다.

#[cfg(target_os = "windows")]
use std::path::Path;

/// 언어별로 쓴 바로가기 이름 전부 (언어를 바꿔 다시 설치해도 옛 이름을 지울 수 있게)
pub const ALL_SHORTCUT_NAMES: &[&str] = &["Saba-chan", "사바쨩", "サバちゃん"];

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

/// 이 설치 폴더를 가리키는 바로가기만 지운다 (바탕화면 `<이름>.lnk`, 시작 메뉴 `<이름>\*.lnk`).
/// 같은 이름이라도 다른 설치(예: 다른 폴더에 한 테스트 설치와 진짜 설치)의 바로가기는 두고,
/// 시작 메뉴 폴더는 비었을 때만 지운다.
#[cfg(target_os = "windows")]
pub fn remove_shortcuts_into(install_dir: &Path, names: &[&str]) {
    const SCRIPT: &str = r#"
$sh = New-Object -ComObject WScript.Shell
$root = $env:SABA_INSTALL_DIR.TrimEnd('\') + '\'
function Check($lnk) {
  try { $t = $sh.CreateShortcut($lnk).TargetPath } catch { $t = '' }
  if ($t -and $t.StartsWith($root, [StringComparison]::OrdinalIgnoreCase)) {
    Remove-Item -LiteralPath $lnk -Force -ErrorAction SilentlyContinue; "removed|$lnk"
  } else { "kept|$lnk|$t" }
}
foreach ($p in ($env:SABA_LNKS -split '\|')) { if ($p -and (Test-Path -LiteralPath $p)) { Check $p } }
foreach ($d in ($env:SABA_DIRS -split '\|')) {
  if ($d -and (Test-Path -LiteralPath $d)) {
    Get-ChildItem -LiteralPath $d -Filter *.lnk -ErrorAction SilentlyContinue | ForEach-Object { Check $_.FullName }
    if (-not (Get-ChildItem -LiteralPath $d -Force -ErrorAction SilentlyContinue)) {
      Remove-Item -LiteralPath $d -Force -ErrorAction SilentlyContinue; "removed|$d"
    } else { "kept|$d" }
  }
}"#;
    let desktop = get_desktop_path().ok();
    let start_menu = get_start_menu_path().ok();
    let lnks: Vec<String> = desktop
        .iter()
        .flat_map(|d| names.iter().map(move |n| d.join(format!("{}.lnk", n)).to_string_lossy().into_owned()))
        .collect();
    let dirs: Vec<String> = start_menu
        .iter()
        .flat_map(|d| names.iter().map(move |n| d.join(n).to_string_lossy().into_owned()))
        .collect();
    let out = saba_chan_updater_lib::process::hide_console(&mut std::process::Command::new(saba_chan_updater_lib::process::system_tool("powershell")))
        .args(["-NoProfile", "-NonInteractive", "-Command", SCRIPT])
        .env("SABA_INSTALL_DIR", install_dir)
        .env("SABA_LNKS", lnks.join("|"))
        .env("SABA_DIRS", dirs.join("|"))
        .output();
    match out {
        Ok(o) => {
            for line in String::from_utf8_lossy(&o.stdout).lines().filter(|l| !l.trim().is_empty()) {
                tracing::info!("[Shortcut] {}", line.trim());
            }
        }
        Err(e) => tracing::warn!("[Shortcut] Could not check old shortcuts: {}", e),
    }
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

    // 경로는 스크립트에 끼우지 않고 환경변수로 넘긴다 (따옴표가 든 경로도 그대로)
    let ps_script = r#"$ws = New-Object -ComObject WScript.Shell; $sc = $ws.CreateShortcut($env:SABA_LNK); $sc.TargetPath = $env:SABA_TARGET; $sc.WorkingDirectory = $env:SABA_WORKDIR; $sc.Description = $env:SABA_DESC; $sc.Save()"#;

    let output = saba_chan_updater_lib::process::hide_console(
        std::process::Command::new(saba_chan_updater_lib::process::system_tool("powershell"))
            .args(["-NoProfile", "-NonInteractive", "-Command", ps_script])
            .env("SABA_LNK", lnk_str.as_ref())
            .env("SABA_TARGET", target_str.as_ref())
            .env("SABA_WORKDIR", &working_dir)
            .env("SABA_DESC", description),
    )
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

// ── Linux: 앱 메뉴 항목 (freedesktop Desktop Entry) ──

/// 앱 메뉴 항목 파일 이름 (제거기의 흔적 정리 `traces`도 이 이름을 본다)
pub const APP_ENTRY_FILE: &str = "saba-chan.desktop";

/// GUI zip에 함께 든 아이콘 (GUI 실행 파일 옆)
#[cfg(any(target_os = "linux", test))]
pub const APP_ICON_FILE: &str = "saba-chan-gui.png";

/// 앱 메뉴 항목 내용. 아이콘이 GUI 옆에 없으면 테마 아이콘 이름으로 둔다
#[cfg(any(target_os = "linux", test))]
fn app_entry(target_exe: &std::path::Path, name: &str) -> String {
    use saba_chan_updater_lib::autostart::{desktop_exec, Entry};
    let exec = desktop_exec(&Entry { program: target_exe.to_path_buf(), args: Vec::new() });
    let icon = target_exe.with_file_name(APP_ICON_FILE);
    let icon = if icon.is_file() { icon.to_string_lossy().into_owned() } else { "saba-chan-gui".to_string() };
    format!(
        "[Desktop Entry]\nType=Application\nName={name}\nExec={exec}\nIcon={}\nTerminal=false\nCategories=Game;Utility;\nStartupWMClass=saba-chan-gui\n",
        icon.replace('\\', "\\\\")
    )
}

/// 앱 메뉴 항목이 이 설치 폴더의 프로그램을 가리키는지
#[cfg(any(target_os = "linux", test))]
fn entry_points_into(text: &str, install_dir: &std::path::Path) -> bool {
    saba_chan_updater_lib::autostart::parse_desktop_file(text)
        .and_then(|e| e.program.parent().map(|p| saba_chan_updater_lib::same_path(p, install_dir)))
        .unwrap_or(false)
}

#[cfg(target_os = "linux")]
fn applications_dir() -> anyhow::Result<std::path::PathBuf> {
    dirs::data_dir()
        .map(|d| d.join("applications"))
        .ok_or_else(|| anyhow::anyhow!("Could not determine the applications folder"))
}

/// 리눅스는 바탕화면 바로가기를 만들지 않는다 (배포판 대부분이 바탕화면 아이콘을 보이지 않거나 실행을 막는다)
#[cfg(target_os = "linux")]
pub fn create_desktop_shortcut(_target_exe: &std::path::Path, _name: &str) -> anyhow::Result<()> {
    Ok(())
}

/// 앱 메뉴(`~/.local/share/applications/saba-chan.desktop`)에 항목을 만든다. 시작 메뉴 바로가기에 해당한다
#[cfg(target_os = "linux")]
pub fn create_start_menu_shortcut(target_exe: &std::path::Path, name: &str) -> anyhow::Result<()> {
    let dir = applications_dir()?;
    std::fs::create_dir_all(&dir)?;
    let path = dir.join(APP_ENTRY_FILE);
    std::fs::write(&path, app_entry(target_exe, name))?;
    tracing::info!("[Shortcut] App menu: {:?}", path);
    Ok(())
}

/// 이 설치 폴더를 가리키는 앱 메뉴 항목만 지운다 (다른 폴더의 설치 항목은 둔다)
#[cfg(target_os = "linux")]
pub fn remove_shortcuts_into(install_dir: &std::path::Path, _names: &[&str]) {
    let Ok(dir) = applications_dir() else { return };
    let path = dir.join(APP_ENTRY_FILE);
    if std::fs::read_to_string(&path).is_ok_and(|t| entry_points_into(&t, install_dir)) {
        let _ = std::fs::remove_file(&path);
    }
}

// ── 그 밖의 OS ──

#[cfg(not(any(target_os = "windows", target_os = "linux")))]
pub fn create_desktop_shortcut(
    _target_exe: &std::path::Path,
    _name: &str,
) -> anyhow::Result<()> {
    Ok(())
}

#[cfg(not(any(target_os = "windows", target_os = "linux")))]
pub fn create_start_menu_shortcut(
    _target_exe: &std::path::Path,
    _name: &str,
) -> anyhow::Result<()> {
    Ok(())
}

#[cfg(not(any(target_os = "windows", target_os = "linux")))]
pub fn remove_shortcuts_into(_install_dir: &std::path::Path, _names: &[&str]) {}

#[cfg(test)]
mod tests {
    use super::*;

    /// 리눅스 앱 메뉴 항목: 공백·특수 글자가 든 설치 경로도 그대로 실행되고, 아이콘은 GUI 옆 파일이다
    #[test]
    fn the_app_menu_entry_runs_the_installed_gui() {
        let tmp = tempfile::tempdir().unwrap();
        let dir = tmp.path().join("My $aba");
        std::fs::create_dir_all(&dir).unwrap();
        let gui = dir.join("saba-chan-gui");
        let text = app_entry(&gui, "사바쨩");
        assert!(text.contains("Name=사바쨩\n"), "{text}");
        assert!(text.contains("Icon=saba-chan-gui\n"), "no icon file yet: {text}");
        let parsed = saba_chan_updater_lib::autostart::parse_desktop_file(&text).unwrap();
        assert_eq!(parsed.program, gui);
        assert!(parsed.args.is_empty());

        std::fs::write(dir.join(APP_ICON_FILE), b"png").unwrap();
        let text = app_entry(&gui, "Saba-chan");
        let icon = dir.join(APP_ICON_FILE).display().to_string().replace('\\', "\\\\");
        assert!(text.contains(&format!("Icon={icon}\n")), "{text}");
        assert!(entry_points_into(&text, &dir));
        assert!(!entry_points_into(&text, tmp.path()), "another install folder");
    }
}
