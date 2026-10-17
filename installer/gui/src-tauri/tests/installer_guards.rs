//! 인스톨러 보안 가드 테스트
//!
//! 1. `--elevate-mkdir` 경로 검증 — 시스템 디렉토리, 루트, 경로 traversal 차단.
//! 2. 설치 디렉토리 마커 검증 — saba-chan 설치물인지 확인 후에만 삭제.

use saba_chan_installer_app::{
    validate_elevate_mkdir_path, verify_install_dir_marker,
};
use tempfile::TempDir;

// ═══════════════════════════════════════════════════════
// validate_elevate_mkdir_path
// ═══════════════════════════════════════════════════════

#[test]
fn elevate_mkdir_rejects_relative_path() {
    let err = validate_elevate_mkdir_path("foo/bar").unwrap_err();
    assert!(err.contains("absolute"), "got: {}", err);
}

#[test]
fn elevate_mkdir_rejects_parent_traversal() {
    #[cfg(windows)]
    let raw = r"C:\Users\victim\..\Windows";
    #[cfg(not(windows))]
    let raw = "/home/victim/../etc";
    let err = validate_elevate_mkdir_path(raw).unwrap_err();
    assert!(err.contains(".."), "got: {}", err);
}

#[cfg(windows)]
#[test]
fn elevate_mkdir_rejects_windows_system_dir() {
    for raw in [
        r"C:\Windows",
        r"C:\Windows\System32",
        r"c:\windows\system32\drivers",
        r"C:\Users\Default",
        r"C:\Users\All Users",
    ] {
        let err = validate_elevate_mkdir_path(raw)
            .unwrap_err_or_else_panic(&format!("should reject: {}", raw));
        assert!(
            err.contains("protected") || err.contains("system"),
            "raw={} err={}",
            raw,
            err
        );
    }
}

#[cfg(windows)]
#[test]
fn elevate_mkdir_rejects_root_level_path() {
    let err = validate_elevate_mkdir_path(r"C:\").unwrap_err();
    assert!(err.contains("shallow") || err.contains("root"), "got: {}", err);
}

#[cfg(windows)]
#[test]
fn elevate_mkdir_accepts_typical_install_path() {
    let path = validate_elevate_mkdir_path(r"C:\Games\saba-chan").unwrap();
    assert_eq!(path.to_string_lossy(), r"C:\Games\saba-chan");
}

#[cfg(windows)]
#[test]
fn elevate_mkdir_accepts_user_localappdata() {
    let path =
        validate_elevate_mkdir_path(r"C:\Users\alice\AppData\Local\Programs\saba-chan").unwrap();
    assert!(path.is_absolute());
}

// ═══════════════════════════════════════════════════════
// verify_install_dir_marker
// ═══════════════════════════════════════════════════════

#[test]
fn install_dir_marker_rejects_empty_dir() {
    let tmp = TempDir::new().unwrap();
    assert!(!verify_install_dir_marker(tmp.path()));
}

#[test]
fn install_dir_marker_rejects_directory_named_like_binary() {
    let tmp = TempDir::new().unwrap();
    std::fs::create_dir(tmp.path().join("saba-core.exe")).unwrap();
    assert!(!verify_install_dir_marker(tmp.path()));
}

#[test]
fn install_dir_marker_rejects_nonexistent_path() {
    let tmp = TempDir::new().unwrap();
    let missing = tmp.path().join("does-not-exist");
    assert!(!verify_install_dir_marker(&missing));
}

#[test]
fn install_dir_marker_rejects_unrelated_dir_with_other_files() {
    let tmp = TempDir::new().unwrap();
    std::fs::write(tmp.path().join("readme.txt"), "not saba").unwrap();
    std::fs::write(tmp.path().join("data.bin"), "junk").unwrap();
    assert!(!verify_install_dir_marker(tmp.path()));
}

#[test]
fn install_dir_marker_accepts_dir_with_gui_exe() {
    let tmp = TempDir::new().unwrap();
    std::fs::write(tmp.path().join("saba-chan-gui.exe"), b"stub").unwrap();
    assert!(verify_install_dir_marker(tmp.path()));
}

#[test]
fn install_dir_marker_accepts_dir_with_saba_core() {
    let tmp = TempDir::new().unwrap();
    std::fs::write(tmp.path().join("saba-core.exe"), b"stub").unwrap();
    assert!(verify_install_dir_marker(tmp.path()));
}

#[test]
fn install_dir_marker_accepts_dir_with_updater() {
    let tmp = TempDir::new().unwrap();
    std::fs::write(tmp.path().join("saba-chan-updater.exe"), b"stub").unwrap();
    assert!(verify_install_dir_marker(tmp.path()));
}

// ═══════════════════════════════════════════════════════
// 도우미 trait (테스트용 깔끔한 expect 메시지)
// ═══════════════════════════════════════════════════════

trait UnwrapErrPanic<E> {
    fn unwrap_err_or_else_panic(self, msg: &str) -> E;
}

impl<T, E> UnwrapErrPanic<E> for Result<T, E> {
    fn unwrap_err_or_else_panic(self, msg: &str) -> E {
        match self {
            Err(e) => e,
            Ok(_) => panic!("{}", msg),
        }
    }
}
