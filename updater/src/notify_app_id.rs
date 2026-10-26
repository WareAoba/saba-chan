//! Windows 알림의 앱 등록 (`HKCU\Software\Classes\AppUserModelId\<앱 ID>`).
//!
//! 알림 플러그인은 설치본에서 Tauri 식별자를 앱 ID로 쓴다. 시작 메뉴 바로가기 없이 설치한 앱은 이 키가
//! 있어야 Windows가 그 ID의 알림을 띄운다. GUI가 시작할 때마다 맞춰 두고, 마지막 설치본을 지울 때 제거기가 지운다.

use std::path::Path;

#[cfg(target_os = "windows")]
fn key_path(app_id: &str) -> String {
    format!(r"Software\Classes\AppUserModelId\{}", app_id)
}

/// 알림에 보일 이름과 아이콘(있으면)을 등록한다
#[cfg(target_os = "windows")]
pub fn register(app_id: &str, display_name: &str, icon: Option<&Path>) -> std::io::Result<()> {
    use winreg::enums::*;
    let (key, _) = winreg::RegKey::predef(HKEY_CURRENT_USER).create_subkey(key_path(app_id))?;
    key.set_value("DisplayName", &display_name)?;
    match icon {
        Some(icon) => key.set_value("IconUri", &icon.to_string_lossy().to_string()),
        None => Ok(()),
    }
}

#[cfg(not(target_os = "windows"))]
pub fn register(_app_id: &str, _display_name: &str, _icon: Option<&Path>) -> std::io::Result<()> {
    Ok(())
}

/// 등록을 지운다. 없으면 그대로 성공이다
#[cfg(target_os = "windows")]
pub fn remove(app_id: &str) -> std::io::Result<()> {
    use winreg::enums::*;
    match winreg::RegKey::predef(HKEY_CURRENT_USER).delete_subkey_all(key_path(app_id)) {
        Err(e) if e.kind() != std::io::ErrorKind::NotFound => Err(e),
        _ => Ok(()),
    }
}

#[cfg(not(target_os = "windows"))]
pub fn remove(_app_id: &str) -> std::io::Result<()> {
    Ok(())
}

#[cfg(all(test, target_os = "windows"))]
mod tests {
    use super::*;

    #[test]
    fn registers_the_name_windows_shows_and_removes_it_again() {
        use winreg::enums::*;
        let id = "com.saba-chan.test-notify-app-id";
        let icon = Path::new(r"C:\saba-chan\notify-icon.png");
        register(id, "Saba-chan", Some(icon)).unwrap();
        register(id, "Saba-chan", Some(icon)).unwrap();
        let key = winreg::RegKey::predef(HKEY_CURRENT_USER).open_subkey(key_path(id)).unwrap();
        assert_eq!(key.get_value::<String, _>("DisplayName").unwrap(), "Saba-chan");
        assert_eq!(key.get_value::<String, _>("IconUri").unwrap(), icon.to_string_lossy());
        remove(id).unwrap();
        assert!(winreg::RegKey::predef(HKEY_CURRENT_USER).open_subkey(key_path(id)).is_err());
        remove(id).unwrap();
    }
}
