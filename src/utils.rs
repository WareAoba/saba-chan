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
/// 구분자·`.`/`..`·드라이브 표기(`C:`)·NUL이 있으면 다른 폴더를 가리킬 수 있어 거부한다
pub fn is_single_path_component(name: &str) -> bool {
    !name.is_empty()
        && name != "."
        && name != ".."
        && !name.contains(['/', '\\', ':', '\0'])
}

/// 설치 도중 데몬이 끝나 남은 준비·옛 폴더와 받다 만 파일 (`.staging-*`·`.old-*`·`.tmp-*`)을 지운다.
/// 설치가 돌 수 없는 기동 때만 부른다
pub fn remove_install_leftovers(dir: &std::path::Path) {
    let Ok(entries) = std::fs::read_dir(dir) else { return };
    for entry in entries.flatten() {
        let name = entry.file_name();
        if ![".staging-", ".old-", ".tmp-"].iter().any(|p| name.to_string_lossy().starts_with(p)) {
            continue;
        }
        let path = entry.path();
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
        for bad in ["", ".", "..", "../x", "a/b", "a\\b", "C:", "C:x", "a\0b"] {
            assert!(!is_single_path_component(bad), "{bad:?}");
        }
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
