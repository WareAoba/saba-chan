//! Shared utility functions for the saba-chan core daemon.

use tokio::process::Command;

/// Apply platform-specific flags to hide the console window on Windows.
/// On non-Windows platforms, this is a no-op.
#[cfg(target_os = "windows")]
pub fn apply_creation_flags(cmd: &mut Command) -> &mut Command {
    const CREATE_NO_WINDOW: u32 = 0x08000000;
    cmd.creation_flags(CREATE_NO_WINDOW)
}

#[cfg(not(target_os = "windows"))]
pub fn apply_creation_flags(cmd: &mut Command) -> &mut Command {
    cmd
}

/// 경로 한 칸으로만 쓸 수 있는 이름인지 (인스턴스 id, 모듈 이름, 로케일 등).
/// 구분자·`.`/`..`·드라이브 표기(`C:`)·NUL이 있으면 다른 폴더를 가리킬 수 있어 거부한다.
/// 끝의 `.`·공백도 거부한다: Windows가 버려서 `x.`가 `x` 폴더를 가리킨다
pub fn is_single_path_component(name: &str) -> bool {
    !name.is_empty()
        && name != "."
        && name != ".."
        && !name.ends_with(['.', ' '])
        && !name.contains(['/', '\\', ':', '\0'])
}

/// 구성요소 폴더 안을 가리키는 상대 경로인가 (`module.entry`, 익스텐션 `python_modules` 값).
/// 비었거나 절대 경로이거나 `..`·`.`·드라이브 같은 성분이 있으면 아니다
pub fn is_inner_relative_path(path: &str) -> bool {
    let p = std::path::Path::new(path);
    !path.is_empty() && !p.is_absolute() && p.components().all(|c| matches!(c, std::path::Component::Normal(_)))
}

/// 설치 도중 데몬이 끝나 남은 준비·옛 폴더와 받다 만 파일 (`.staging-*`·`.old-*`·`.tmp-*`)을 지운다.
/// 옛 폴더를 옆으로 옮긴 뒤 새 폴더를 두기 전에 끝났으면(제자리가 비었으면) 옛 폴더를 되돌린다.
/// 설치가 돌 수 없는 기동 때만 부른다
pub fn remove_install_leftovers(dir: &std::path::Path) {
    let Ok(entries) = std::fs::read_dir(dir) else { return };
    let mut leftovers: Vec<_> = entries.flatten().map(|e| e.path()).collect();
    // 되돌리기를 먼저: 되돌린 폴더는 지우지 않는다
    leftovers.retain(|path| {
        let Some(original) = path.file_name().and_then(|n| n.to_str()).and_then(|n| n.strip_prefix(".old-")) else { return true };
        let target = dir.join(original);
        if original.is_empty() || !path.is_dir() || target.exists() {
            return true;
        }
        match std::fs::rename(path, &target) {
            Ok(()) => {
                tracing::warn!("Restored {} after an interrupted install", target.display());
                false
            }
            Err(e) => {
                // 지우면 마지막 정상 설치까지 잃는다: 그대로 둔다
                tracing::warn!("Could not restore {} after an interrupted install: {}", target.display(), e);
                false
            }
        }
    });
    for path in leftovers {
        let name = path.file_name().unwrap_or_default().to_string_lossy().into_owned();
        if ![".staging-", ".old-", ".tmp-"].iter().any(|p| name.starts_with(p)) {
            continue;
        }
        let removed = if path.is_dir() { std::fs::remove_dir_all(&path) } else { std::fs::remove_file(&path) };
        match removed {
            Ok(()) => tracing::info!("Removed leftover of an interrupted install: {}", path.display()),
            Err(e) => tracing::warn!("Could not remove leftover {}: {}", path.display(), e),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn only_plain_names_are_single_components() {
        for ok in ["minecraft", "ko", "zh-TW", "a1b2-c3", "my.server"] {
            assert!(is_single_path_component(ok), "{ok}");
        }
        // Windows는 끝의 `.`·공백을 버려 `x.`와 `x `가 `x` 폴더를 가리킨다 (백로그 L17c)
        for bad in ["", ".", "..", "../x", "a/b", "a\\b", "C:", "C:x", "a\0b", "x.", "x ", "x. "] {
            assert!(!is_single_path_component(bad), "{bad:?}");
        }
    }

    #[test]
    fn an_install_cut_off_mid_swap_gets_its_old_folder_back() {
        let tmp = tempfile::tempdir().unwrap();
        // 옛 폴더를 옆으로 옮긴 뒤, 새 폴더를 제자리에 두기 전에 데몬이 끝났다
        for dir in [".old-box", ".staging-box"] {
            std::fs::create_dir_all(tmp.path().join(dir)).unwrap();
        }
        std::fs::write(tmp.path().join(".old-box").join("module.toml"), "kept").unwrap();
        remove_install_leftovers(tmp.path());
        let left: Vec<String> = std::fs::read_dir(tmp.path()).unwrap().map(|e| e.unwrap().file_name().to_string_lossy().into_owned()).collect();
        assert_eq!(left, ["box"]);
        assert_eq!(std::fs::read_to_string(tmp.path().join("box").join("module.toml")).unwrap(), "kept");
    }

    #[test]
    fn leftovers_of_an_interrupted_install_are_removed_and_nothing_else() {
        let tmp = tempfile::tempdir().unwrap();
        for dir in [".staging-box", ".staging-box-1a2b", ".old-box", "box", ".hidden"] {
            std::fs::create_dir_all(tmp.path().join(dir)).unwrap();
        }
        std::fs::write(tmp.path().join(".tmp-module-box.zip"), "partial").unwrap();
        std::fs::write(tmp.path().join("box.zip"), "kept").unwrap();
        remove_install_leftovers(tmp.path());
        let mut left: Vec<String> = std::fs::read_dir(tmp.path()).unwrap().map(|e| e.unwrap().file_name().to_string_lossy().into_owned()).collect();
        left.sort();
        assert_eq!(left, [".hidden", "box", "box.zip"]);
    }
}
