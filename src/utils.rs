//! Shared utility functions for the saba-chan core daemon.

use tokio::process::Command;

pub fn is_single_path_component(name: &str) -> bool {
    !name.is_empty()
        && name != "."
        && name != ".."
        && !name.ends_with(['.', ' '])
        && !name.chars().any(|c| c.is_control() || matches!(c, '/' | '\\' | ':'))
}

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

/// 파일을 원자적으로 기록한다: 같은 디렉토리의 임시 파일에 쓰고 rename으로 교체.
///
/// 쓰기 도중 크래시가 나도 기존 파일이 반쯤 잘린 상태로 남지 않는다
/// (설정·인스턴스 파일이 깨지면 로드 시 기본값으로 대체되어 사용자 데이터가 유실된다).
pub fn write_atomic(path: &std::path::Path, contents: &[u8]) -> std::io::Result<()> {
    use std::io::Write;
    let dir = path.parent().filter(|p| !p.as_os_str().is_empty()).unwrap_or(std::path::Path::new("."));
    std::fs::create_dir_all(dir)?;
    let file_name = path
        .file_name()
        .map(|n| n.to_string_lossy().to_string())
        .unwrap_or_else(|| "file".to_string());
    let tmp = dir.join(format!(".{}.{}.tmp", file_name, uuid::Uuid::new_v4().simple()));
    let result = (|| {
        let mut f = std::fs::File::create(&tmp)?;
        f.write_all(contents)?;
        f.sync_all()?;
        drop(f);
        std::fs::rename(&tmp, path)
    })();
    if result.is_err() {
        let _ = std::fs::remove_file(&tmp);
    }
    result
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn single_path_component_rejects_traversal_and_drive_forms() {
        for bad in ["", ".", "..", "a/b", "a\b", "C:foo", "x.", "x ", "a\0b"] {
            assert!(!is_single_path_component(bad), "{:?} must be rejected", bad);
        }
        for good in ["palworld", "ext-steamcmd", "ko", "pt-BR", "my_module.v2"] {
            assert!(is_single_path_component(good), "{:?} must be accepted", good);
        }
    }

    #[test]
    fn write_atomic_replaces_existing_file_and_leaves_no_temp() {
        let dir = tempfile::tempdir().unwrap();
        let p = dir.path().join("cfg.json");
        std::fs::write(&p, b"old").unwrap();
        write_atomic(&p, b"new").unwrap();
        assert_eq!(std::fs::read(&p).unwrap(), b"new");
        let names: Vec<_> = std::fs::read_dir(dir.path()).unwrap().map(|e| e.unwrap().file_name()).collect();
        assert_eq!(names.len(), 1, "temp file left behind: {:?}", names);
    }
}
