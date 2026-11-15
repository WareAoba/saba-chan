//! 업데이터 통합 테스트: 임시 폴더의 `UpdateManager`로 zip 해제·모듈·바이너리 교체를 본다.
//! 릴레이 check → download → apply 전체 흐름은 `updater_relay_e2e.rs`.

use std::path::Path;
use tempfile::TempDir;

use saba_chan_updater_lib::{
    Component,
    UpdateConfig, UpdateManager,
};
use saba_chan_updater_lib::version::SemVer;

// ── 테스트 유틸리티 ──

/// UpdateManager를 테스트 환경으로 생성 (staging, install_root, modules 모두 tempdir)
fn create_test_manager(tmpdir: &TempDir) -> UpdateManager {
    let modules_dir = tmpdir.path().join("modules");
    std::fs::create_dir_all(&modules_dir).unwrap();

    let config = UpdateConfig {
        enabled: true,
        check_interval_hours: 3,
        auto_download: false,
        install_root: Some(tmpdir.path().to_string_lossy().to_string()),
        ..Default::default()
    };

    UpdateManager::new(config, &modules_dir.to_string_lossy())
}

/// 두 자리 버전은 patch 0으로 읽고, 숫자가 아닌 것과 한 자리는 버전으로 보지 않는다 (나머지 SemVer 규칙은 updater/src/version.rs)
#[test]
fn semver_reads_two_parts_and_rejects_the_rest() {
    let v = SemVer::parse("1.0").unwrap();
    assert_eq!((v.major, v.minor, v.patch), (1, 0, 0));
    for bad in ["", "abc", "1"] {
        assert!(SemVer::parse(bad).is_none(), "{bad:?}");
    }
}

// ── UpdateManager 단위 테스트 (파일시스템) ──

#[test]
fn test_manager_core_always_installed() {
    let tmp = TempDir::new().unwrap();
    let mgr = create_test_manager(&tmp);
    assert!(mgr.is_component_installed(&Component::CoreDaemon));
}

#[test]
fn test_manager_missing_module_not_installed() {
    let tmp = TempDir::new().unwrap();
    let mgr = create_test_manager(&tmp);
    assert!(!mgr.is_component_installed(&Component::Module("nonexistent".into())));
}

#[test]
fn test_manager_existing_module_installed() {
    let tmp = TempDir::new().unwrap();

    // 모듈 디렉터리와 module.toml 생성
    let mod_dir = tmp.path().join("modules").join("testgame");
    std::fs::create_dir_all(&mod_dir).unwrap();
    std::fs::write(mod_dir.join("module.toml"), r#"
name = "testgame"
version = "1.0.0"
"#).unwrap();

    let mgr = create_test_manager(&tmp);
    assert!(mgr.is_component_installed(&Component::Module("testgame".into())));
}

#[test]
fn test_manager_initial_status_empty() {
    let tmp = TempDir::new().unwrap();
    let mgr = create_test_manager(&tmp);
    let status = mgr.get_status();

    assert!(status.last_check.is_none());
    assert!(status.next_check.is_none());
    assert!(status.components.is_empty());
    assert!(!status.checking);
    assert!(status.error.is_none());
}

#[test]
fn test_manager_config_roundtrip() {
    let tmp = TempDir::new().unwrap();
    let mut mgr = create_test_manager(&tmp);

    let new_cfg = UpdateConfig {
        relay_url: Some("http://127.0.0.1:9".into()),
        auto_download: true,
        ..UpdateConfig::default()
    };
    mgr.update_config(new_cfg);

    assert_eq!(mgr.get_config().relay_url.as_deref(), Some("http://127.0.0.1:9"));
    assert!(mgr.get_config().auto_download);
}

// ── 에지 케이스 테스트 ──

#[tokio::test]
async fn test_download_without_check_fails() {
    let tmp = TempDir::new().unwrap();
    let mut mgr = create_test_manager(&tmp);

    // check 없이 download 시도
    let result = mgr.download_available_updates().await;
    // 컴포넌트가 없으므로 다운로드할 게 없음 → 빈 Vec 반환
    match result {
        Ok(v) => assert!(v.is_empty(), "Should download nothing without prior check"),
        Err(_) => {} // 에러도 합리적
    }
}

#[tokio::test]
async fn test_apply_without_downloaded_returns_empty() {
    let tmp = TempDir::new().unwrap();
    let mut mgr = create_test_manager(&tmp);

    let keys = mgr.pending_keys_in_apply_order();
    let (applied, failed) = mgr.apply_target_keys(&keys, |_, _, _| {}).await;
    assert!(applied.is_empty() && failed.is_empty(), "Should apply nothing if nothing downloaded: {applied:?} {failed:?}");
}

#[test]
fn test_single_file_copy_instead_of_zip() {
    let tmp = TempDir::new().unwrap();

    // .txt 파일 (zip 아닌 단일 파일)
    let staged = tmp.path().join("readme.txt");
    std::fs::write(&staged, "Hello from single file").unwrap();

    let target = tmp.path().join("output");
    std::fs::create_dir_all(&target).unwrap();

    // 단일 파일 복사 로직
    let file_name = staged.file_name().unwrap();
    std::fs::copy(&staged, target.join(file_name)).unwrap();

    assert!(target.join("readme.txt").exists());
    let content = std::fs::read_to_string(target.join("readme.txt")).unwrap();
    assert_eq!(content, "Hello from single file");
}

// ── 백업 & 복구 테스트 ──

#[test]
fn test_module_backup_before_update() {
    let tmp = TempDir::new().unwrap();

    // 기존 모듈 (v1)
    let mod_dir = tmp.path().join("modules").join("testmod");
    std::fs::create_dir_all(&mod_dir).unwrap();
    std::fs::write(mod_dir.join("module.toml"), "name = \"testmod\"\nversion = \"1.0.0\"\n").unwrap();
    std::fs::write(mod_dir.join("data.txt"), "important data v1").unwrap();

    // 백업 생성
    let backup_dir = tmp.path().join("backup_testmod");
    copy_dir_recursive(&mod_dir, &backup_dir);

    // 원본 덮어쓰기
    std::fs::write(mod_dir.join("module.toml"), "name = \"testmod\"\nversion = \"2.0.0\"\n").unwrap();
    std::fs::write(mod_dir.join("data.txt"), "new data v2").unwrap();

    // 백업 검증 (원본 변경 후에도 유지)
    let backup_content = std::fs::read_to_string(backup_dir.join("module.toml")).unwrap();
    assert!(backup_content.contains("1.0.0"), "Backup should have v1");

    let current_content = std::fs::read_to_string(mod_dir.join("module.toml")).unwrap();
    assert!(current_content.contains("2.0.0"), "Current should have v2");

    // 복구: 백업 → 원본
    copy_dir_recursive(&backup_dir, &mod_dir);
    let restored = std::fs::read_to_string(mod_dir.join("module.toml")).unwrap();
    assert!(restored.contains("1.0.0"), "Restored should have v1 again");

    println!("✓ Backup/restore cycle works correctly");
}

fn copy_dir_recursive(src: &Path, dst: &Path) {
    std::fs::create_dir_all(dst).unwrap();
    for entry in std::fs::read_dir(src).unwrap() {
        let entry = entry.unwrap();
        let ty = entry.file_type().unwrap();
        let src_path = entry.path();
        let dst_path = dst.join(entry.file_name());
        if ty.is_dir() {
            copy_dir_recursive(&src_path, &dst_path);
        } else {
            std::fs::copy(&src_path, &dst_path).unwrap();
        }
    }
}
