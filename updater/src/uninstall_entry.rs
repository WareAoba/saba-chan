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

/// 데몬 실행 파일 이름: 목록의 폴더에 이것이 남아 있어야 설치본으로 친다
pub const CORE_EXE: &str = if cfg!(windows) { "saba-core.exe" } else { "saba-core" };

/// 설치한 폴더 목록 (사용자 데이터 폴더의 `installs.json`). 제거 항목이 없는 설치(CLI `install --dir`)도 여기에 남는다
pub fn installs_file() -> std::path::PathBuf {
    crate::constants::resolve_data_dir().join("installs.json")
}

pub fn listed_installs(file: &Path) -> Vec<String> {
    std::fs::read(file)
        .ok()
        .and_then(|b| serde_json::from_slice::<Vec<String>>(&b).ok())
        .unwrap_or_default()
}

fn write_installs(file: &Path, list: &[String]) -> std::io::Result<()> {
    if let Some(parent) = file.parent() {
        std::fs::create_dir_all(parent)?;
    }
    crate::write_atomic(file, &serde_json::to_vec_pretty(list).unwrap_or_default())
}

/// 설치를 마친 폴더를 목록에 더한다 (같은 폴더는 한 번)
pub fn remember_install(file: &Path, install_dir: &Path) -> std::io::Result<()> {
    let mut list = listed_installs(file);
    if list.iter().any(|d| crate::same_path(Path::new(d), install_dir)) {
        return Ok(());
    }
    list.push(install_dir.to_string_lossy().to_string());
    write_installs(file, &list)
}

/// 지우는 폴더를 목록에서 뺀다
pub fn forget_install(file: &Path, install_dir: &Path) -> std::io::Result<()> {
    let mut list = listed_installs(file);
    let before = list.len();
    list.retain(|d| !crate::same_path(Path::new(d), install_dir));
    if list.len() == before {
        return Ok(());
    }
    write_installs(file, &list)
}

/// 목록에 이 폴더 말고 데몬 실행 파일이 남은 설치본이 있는지
pub fn other_listed_install(file: &Path, install_dir: &Path) -> bool {
    listed_installs(file)
        .iter()
        .any(|d| !crate::same_path(Path::new(d), install_dir) && Path::new(d).join(CORE_EXE).is_file())
}

/// 사용자 데이터 폴더에서 앱이 만들지 않는 항목: 제거기는 어떤 경우에도 지우지 않는다 (업데이트 서명 키)
const NEVER_REMOVED: &[&str] = &[".keys"];

/// 제거할 때 사용자 데이터 폴더에 남길 것. 아무것도 정하지 않으면 둘 다 남긴다 (사용자 지정 10-22)
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Keep {
    /// 설정·연결 토큰 등 아래 두 목록에 없는 모든 항목
    pub settings: bool,
    /// 인스턴스 폴더 (서버 파일, 월드, 백업, 인스턴스별 설정)
    pub instances: bool,
}

impl Keep {
    pub const ALL: Keep = Keep { settings: true, instances: true };
    pub const NOTHING: Keep = Keep { settings: false, instances: false };
}

/// 무엇을 남기든 지우는 항목: 다시 받거나 다시 만들 수 있는 런타임·캐시·상태 파일.
/// 여기와 [`INSTANCE_ENTRIES`]에 없는 것(설정, 토큰, 앞으로 생길 파일)은 설정으로 친다
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
    "temp",
    "pip-cache",
    "_python_download.tar.gz",
    "daemon.lock",
    "daemon.lock.owner",
    "daemon.log",
    "update-complete.json",
    "updater-state.json",
    "installed-files.json",
    "installed-manifest.json",
    "installs.json",
    "official-components.json",
    "scan-verdicts.json",
    "sandbox-grants.json",
];

/// 인스턴스를 남기면 두는 항목
const INSTANCE_ENTRIES: &[&str] = &["instances"];

/// 제거기가 사용자 데이터 폴더의 `name` 항목을 지울지
pub fn removable_data_entry(name: &str, keep: Keep) -> bool {
    if NEVER_REMOVED.contains(&name) {
        false
    } else if RUNTIME_ENTRIES.contains(&name) {
        true
    } else if INSTANCE_ENTRIES.contains(&name) {
        !keep.instances
    } else {
        !keep.settings
    }
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

    /// 항목 없이 설치된 사본(CLI `install --dir`)도 다른 설치본으로 친다: 공용 사용자 데이터를 남긴다
    #[test]
    fn an_install_without_an_entry_still_keeps_shared_data() {
        let tmp = tempfile::tempdir().unwrap();
        let file = tmp.path().join("installs.json");
        let gui = tmp.path().join("gui");
        let cli = tmp.path().join("cli");
        for dir in [&gui, &cli] {
            std::fs::create_dir_all(dir).unwrap();
            std::fs::write(dir.join(CORE_EXE), b"").unwrap();
            remember_install(&file, dir).unwrap();
        }
        remember_install(&file, &cli).unwrap();
        assert_eq!(listed_installs(&file).len(), 2, "the same folder is listed once");
        assert!(other_listed_install(&file, &gui), "the CLI copy is another install");

        // 지운 폴더(데몬 실행 파일이 없는 폴더)는 남은 설치본이 아니다
        std::fs::remove_file(cli.join(CORE_EXE)).unwrap();
        assert!(!other_listed_install(&file, &gui));

        std::fs::write(cli.join(CORE_EXE), b"").unwrap();
        forget_install(&file, &cli).unwrap();
        assert!(!other_listed_install(&file, &gui));
        assert!(!other_listed_install(&tmp.path().join("missing.json"), &gui), "no list: nothing else known");
    }

    #[test]
    fn signing_keys_are_never_removed() {
        for keep in [Keep::NOTHING, Keep::ALL, Keep { settings: true, instances: false }, Keep { settings: false, instances: true }] {
            assert!(!removable_data_entry(".keys", keep), "the app never writes .keys");
        }
    }

    #[test]
    fn runtime_entries_go_whatever_is_kept() {
        for name in ["python-standalone", "python-env", "modules", "extensions", "updates", "daemon.log", "installs.json", "temp", "pip-cache"] {
            assert!(removable_data_entry(name, Keep::ALL), "{name} is runtime");
        }
    }

    #[test]
    fn settings_and_instances_are_kept_separately() {
        let settings = ["config.json", ".node_token", ".ipc_token", ".anon_id", "audit.log", "something-new"];
        let only_settings = Keep { settings: true, instances: false };
        let only_instances = Keep { settings: false, instances: true };
        for name in settings {
            assert!(!removable_data_entry(name, only_settings), "{name} is a setting");
            assert!(removable_data_entry(name, only_instances), "{name} goes without settings");
        }
        assert!(removable_data_entry("instances", only_settings));
        assert!(!removable_data_entry("instances", only_instances));
        assert!(removable_data_entry("instances", Keep::NOTHING) && removable_data_entry("config.json", Keep::NOTHING));
    }

    #[test]
    fn hashed_name_ignores_trailing_separators_and_case() {
        assert_eq!(hashed_key_name(Path::new("/Opt/Saba/")), hashed_key_name(Path::new("/opt/saba")));
    }
}
