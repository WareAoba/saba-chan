//! Windows 알림의 앱 등록 (`HKCU\Software\Classes\AppUserModelId\<앱 ID>`).
//!
//! 알림 플러그인은 설치본에서 GUI의 Tauri 식별자를 앱 ID로 쓴다. 시작 메뉴 바로가기 없이 설치한 앱은 이 키가
//! 있어야 Windows가 그 ID의 알림을 띄운다. 데몬이 시작할 때마다 맞춰 두고, 마지막 설치본을 지울 때 제거기가 지운다.

#[cfg(target_os = "windows")]
use std::path::Path;

/// GUI의 Tauri 식별자 (`saba-chan-gui/src-tauri/tauri.conf.json` `identifier`)
pub const GUI_APP_ID: &str = "com.saba-chan.app";

/// 알림에 보일 앱 이름
pub const DISPLAY_NAME: &str = "Saba-chan";

/// 알림 아이콘. GUI 앱 데이터 폴더(`%LOCALAPPDATA%\<GUI_APP_ID>`)에 둔다: 제거기가 이 폴더째 지운다
pub const ICON_FILE: &str = "notify-icon.png";

#[cfg(target_os = "windows")]
fn key_path(app_id: &str) -> String {
    format!(r"Software\Classes\AppUserModelId\{}", app_id)
}

/// 알림 아이콘 파일 위치
#[cfg(target_os = "windows")]
pub fn icon_path() -> Option<std::path::PathBuf> {
    std::env::var_os("LOCALAPPDATA").map(|d| Path::new(&d).join(GUI_APP_ID).join(ICON_FILE))
}

/// GUI 알림을 등록한다. 아이콘은 토스트 이미지 크기 제한 안에 드는 작은 PNG를 준다. 아이콘 파일이 없거나 다르면 `icon_png`로 쓴다 (못 만들면 이름만 등록)
#[cfg(target_os = "windows")]
pub fn ensure(icon_png: &[u8]) -> std::io::Result<()> {
    // 바뀐 아이콘으로 다시 쓴다 (크기로 비교)
    let icon = icon_path().filter(|p| {
        std::fs::metadata(p).is_ok_and(|m| m.len() == icon_png.len() as u64)
            || std::fs::create_dir_all(p.parent().unwrap_or(p)).and_then(|_| std::fs::write(p, icon_png)).is_ok()
    });
    register(GUI_APP_ID, DISPLAY_NAME, icon.as_deref())
}

#[cfg(not(target_os = "windows"))]
pub fn ensure(_icon_png: &[u8]) -> std::io::Result<()> {
    Ok(())
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

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::Path;

    /// 알림 플러그인은 Tauri 식별자를 앱 ID로 쓰므로 둘이 같아야 등록이 맞는다
    #[test]
    fn app_id_is_the_gui_tauri_identifier() {
        let conf = Path::new(env!("CARGO_MANIFEST_DIR")).join("../saba-chan-gui/src-tauri/tauri.conf.json");
        let conf: serde_json::Value = serde_json::from_str(&std::fs::read_to_string(conf).unwrap()).unwrap();
        assert_eq!(conf["identifier"], GUI_APP_ID);
        assert_eq!(conf["productName"], DISPLAY_NAME);
    }

    #[cfg(target_os = "windows")]
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
