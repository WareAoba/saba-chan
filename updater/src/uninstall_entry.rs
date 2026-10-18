//! Windows "설치된 앱" 항목 (`HKCU\...\Uninstall\Saba-chan*`): 설치본마다 따로 둔다.
//!
//! 설치 폴더가 키의 기준이다. 첫 설치본은 예전과 같은 `Saba-chan` 키, 다른 폴더에 또 설치하면
//! `Saba-chan-<폴더 해시>` 키를 쓴다. 설치기·제거기·업데이터는 모두 자기 폴더의 키만 읽고 쓴다.

use std::path::Path;

pub const BASE_KEY: &str = "Saba-chan";
#[cfg(target_os = "windows")]
const UNINSTALL_ROOT: &str = r"Software\Microsoft\Windows\CurrentVersion\Uninstall";

/// 설치 폴더별 키 이름 후보 (`Saba-chan-` + 소문자 경로 sha256 앞 8자)
pub fn hashed_key_name(install_dir: &Path) -> String {
    use sha2::{Digest, Sha256};
    let normalized = install_dir.to_string_lossy().trim_end_matches(['\\', '/']).to_lowercase();
    let digest = Sha256::digest(normalized.as_bytes());
    let hex: String = digest.iter().take(4).map(|b| format!("{:02x}", b)).collect();
    format!("{}-{}", BASE_KEY, hex)
}

/// 이 설치 폴더의 항목 이름. `entries`는 이미 있는 `(키 이름, InstallLocation)` 목록
pub fn pick_key_name(entries: &[(String, String)], install_dir: &Path) -> String {
    if let Some((name, _)) = entries.iter().find(|(_, loc)| crate::same_path(Path::new(loc), install_dir)) {
        return name.clone();
    }
    if entries.iter().any(|(name, _)| name == BASE_KEY) {
        hashed_key_name(install_dir)
    } else {
        BASE_KEY.to_string()
    }
}

/// 이 설치 폴더를 가리키는 기존 항목 이름
pub fn find_key_name(entries: &[(String, String)], install_dir: &Path) -> Option<String> {
    entries
        .iter()
        .find(|(_, loc)| crate::same_path(Path::new(loc), install_dir))
        .map(|(name, _)| name.clone())
}

/// 이 설치본 말고 남은 사바쨩 설치가 없는지 (`install_dir`가 None이면 어느 항목도 내 것이 아니다).
/// 사용자 데이터 폴더는 설치본들이 함께 쓰므로 마지막 설치본을 지울 때만 지운다
pub fn is_last_install(entries: &[(String, String)], install_dir: Option<&Path>) -> bool {
    // 내 설치 폴더를 못 찾았으면 지우는 것이 내 것인지 알 수 없다
    let Some(dir) = install_dir else { return false };
    entries.iter().all(|(_, loc)| crate::same_path(Path::new(loc), dir))
}

/// 사용자 데이터 폴더에서 앱이 만들지 않는 항목: 제거기는 어떤 경우에도 지우지 않는다 (업데이트 서명 키)
const NEVER_REMOVED: &[&str] = &[".keys"];

/// 설정을 남기는 제거에서 지우는 항목: 다시 받거나 다시 만들 수 있는 런타임·캐시만.
/// 여기 없는 것(설정, 인스턴스, 토큰, 앞으로 생길 파일)은 남긴다
const RUNTIME_ENTRIES: &[&str] = &[
    "python-standalone",
    "python-env",
    "node-portable",
    "modules",
    "extensions",
    "updates",
    "logs",
    "pycache",
    "pycache-sandbox",
    "python-guard",
    "sandbox-temp",
    "_python_download.tar.gz",
    "daemon.lock",
    "daemon.log",
    "update-complete.json",
    "updater-state.json",
    "installed-files.json",
    "installed-manifest.json",
    "official-components.json",
    "scan-verdicts.json",
    "sandbox-grants.json",
];

/// 제거기가 사용자 데이터 폴더의 `name` 항목을 지울지
pub fn removable_data_entry(name: &str, keep_settings: bool) -> bool {
    if NEVER_REMOVED.contains(&name) {
        return false;
    }
    !keep_settings || RUNTIME_ENTRIES.contains(&name)
}

/// 레지스트리에 있는 사바쨩 항목들 `(키 이름, InstallLocation)`
#[cfg(target_os = "windows")]
pub fn entries() -> Vec<(String, String)> {
    use winreg::enums::*;
    use winreg::RegKey;
    let Ok(root) = RegKey::predef(HKEY_CURRENT_USER).open_subkey(UNINSTALL_ROOT) else { return Vec::new() };
    root.enum_keys()
        .flatten()
        .filter(|name| name == BASE_KEY || name.starts_with(&format!("{}-", BASE_KEY)))
        .filter_map(|name| {
            let loc: String = root.open_subkey(&name).ok()?.get_value("InstallLocation").ok()?;
            Some((name, loc))
        })
        .collect()
}

#[cfg(not(target_os = "windows"))]
pub fn entries() -> Vec<(String, String)> {
    Vec::new()
}

/// 키 전체 경로 (HKCU 아래)
#[cfg(target_os = "windows")]
pub fn key_path(name: &str) -> String {
    format!(r"{}\{}", UNINSTALL_ROOT, name)
}

/// 이 설치본의 표시 버전만 고친다. 항목이 없으면 (설치기 없이 푼 경우) 아무것도 안 한다
#[cfg(target_os = "windows")]
pub fn set_display_version(install_dir: &Path, version: &str) -> std::io::Result<bool> {
    use winreg::enums::*;
    use winreg::RegKey;
    let Some(name) = find_key_name(&entries(), install_dir) else { return Ok(false) };
    let key = RegKey::predef(HKEY_CURRENT_USER).open_subkey_with_flags(key_path(&name), KEY_WRITE)?;
    key.set_value("DisplayVersion", &version.trim_start_matches('v'))?;
    Ok(true)
}

#[cfg(not(target_os = "windows"))]
pub fn set_display_version(_install_dir: &Path, _version: &str) -> std::io::Result<bool> {
    Ok(false)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn entry(name: &str, loc: &str) -> (String, String) {
        (name.to_string(), loc.to_string())
    }

    #[test]
    fn first_install_takes_the_base_key_and_later_ones_get_their_own() {
        let a = Path::new("/opt/saba-a");
        let b = Path::new("/opt/saba-b");
        assert_eq!(pick_key_name(&[], a), BASE_KEY);
        let existing = vec![entry(BASE_KEY, "/opt/saba-a")];
        assert_eq!(pick_key_name(&existing, a), BASE_KEY, "reinstalling keeps its key");
        let other = pick_key_name(&existing, b);
        assert_ne!(other, BASE_KEY, "a second install never overwrites the first one's entry");
        assert_eq!(other, hashed_key_name(b));
        let both = vec![entry(BASE_KEY, "/opt/saba-a"), entry(&other, "/opt/saba-b")];
        assert_eq!(pick_key_name(&both, b), other);
        assert_eq!(find_key_name(&both, b).as_deref(), Some(other.as_str()));
        assert_eq!(find_key_name(&both, Path::new("/opt/saba-c")), None);
    }

    #[test]
    fn shared_data_goes_only_with_the_last_install() {
        let a = Path::new("/opt/saba-a");
        assert!(is_last_install(&[], Some(a)));
        assert!(!is_last_install(&[], None), "no install of our own found: never wipe shared data");
        assert!(is_last_install(&[entry(BASE_KEY, "/opt/saba-a")], Some(a)));
        let two = vec![entry(BASE_KEY, "/opt/saba-a"), entry("Saba-chan-1234abcd", "/opt/saba-b")];
        assert!(!is_last_install(&two, Some(a)));
        assert!(!is_last_install(&[entry(BASE_KEY, "/opt/saba-a")], None));
    }

    #[test]
    fn signing_keys_and_unknown_files_are_never_user_data_to_wipe() {
        assert!(!removable_data_entry(".keys", false), "the app never writes .keys");
        assert!(!removable_data_entry(".keys", true));
        assert!(removable_data_entry("instances", false));
        assert!(removable_data_entry("config.json", false));
    }

    #[test]
    fn keeping_settings_removes_only_runtime_entries() {
        for name in ["python-standalone", "python-env", "modules", "extensions", "updates", "daemon.log"] {
            assert!(removable_data_entry(name, true), "{name} is runtime");
        }
        for name in ["config.json", "instances", ".node_token", ".ipc_token", ".anon_id", "audit.log", "something-new"] {
            assert!(!removable_data_entry(name, true), "{name} is kept");
        }
    }

    #[test]
    fn hashed_name_ignores_trailing_separators_and_case() {
        assert_eq!(hashed_key_name(Path::new("/Opt/Saba/")), hashed_key_name(Path::new("/opt/saba")));
    }
}
