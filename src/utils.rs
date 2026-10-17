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
}
