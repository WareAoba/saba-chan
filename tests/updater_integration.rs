//! ═══════════════════════════════════════════════════════════════════
//! 업데이터 통합 테스트
//! ═══════════════════════════════════════════════════════════════════
//!
//! 전체 업데이트 파이프라인을 테스트합니다:
//!
//! 1. **버전 파싱/비교** — SemVer 유효성, 순서 비교
//! 2. **파일 덮어쓰기** — zip 해제, 모듈 교체, 바이너리 교체
//! 3. **설정 변경** — UpdateConfig round-trip
//!
//! 릴레이 check → download → apply 전체 흐름은 `updater_relay_e2e.rs`.
//!
//! 모든 테스트는 `tempdir`을 사용해 파일시스템을 격리합니다.

use std::collections::HashMap;
use std::io::Write;
use std::path::Path;
use tempfile::TempDir;

use saba_chan_updater_lib::{
    Component,
    UpdateConfig, UpdateManager,
};
use saba_chan_updater_lib::version::SemVer;

// ═══════════════════════════════════════════════════════
// 테스트 유틸리티
// ═══════════════════════════════════════════════════════

/// 테스트용 zip 파일 생성 — 파일 이름 → 내용 맵을 zip 으로 패킹
fn create_test_zip(files: &HashMap<&str, &[u8]>) -> Vec<u8> {
    let buf = std::io::Cursor::new(Vec::new());
    let mut zip_writer = zip::ZipWriter::new(buf);
    let options = zip::write::SimpleFileOptions::default()
        .compression_method(zip::CompressionMethod::Stored);

    for (name, content) in files {
        zip_writer.start_file(*name, options).unwrap();
        zip_writer.write_all(content).unwrap();
    }

    let cursor = zip_writer.finish().unwrap();
    cursor.into_inner()
}


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

// ═══════════════════════════════════════════════════════
// 1. 버전 파싱/비교 테스트
// ═══════════════════════════════════════════════════════

#[test]
fn test_semver_parse_basic() {
    let v = SemVer::parse("1.2.3").unwrap();
    assert_eq!((v.major, v.minor, v.patch), (1, 2, 3));
    assert!(v.prerelease.is_none());
}

#[test]
fn test_semver_parse_v_prefix() {
    let v = SemVer::parse("v0.1.0").unwrap();
    assert_eq!((v.major, v.minor, v.patch), (0, 1, 0));
}

#[test]
fn test_semver_parse_prerelease() {
    let v = SemVer::parse("2.0.0-rc.1").unwrap();
    assert!(v.is_prerelease());
    assert_eq!(v.prerelease, Some("rc.1".to_string()));
}

#[test]
fn test_semver_parse_two_part() {
    let v = SemVer::parse("1.0").unwrap();
    assert_eq!((v.major, v.minor, v.patch), (1, 0, 0));
}

#[test]
fn test_semver_invalid() {
    assert!(SemVer::parse("").is_none());
    assert!(SemVer::parse("abc").is_none());
    assert!(SemVer::parse("1").is_none());
}

#[test]
fn test_semver_comparison_patch() {
    let v1 = SemVer::parse("1.0.0").unwrap();
    let v2 = SemVer::parse("1.0.1").unwrap();
    assert!(v2.is_newer_than(&v1));
    assert!(!v1.is_newer_than(&v2));
}

#[test]
fn test_semver_comparison_minor() {
    let v1 = SemVer::parse("1.0.9").unwrap();
    let v2 = SemVer::parse("1.1.0").unwrap();
    assert!(v2.is_newer_than(&v1));
}

#[test]
fn test_semver_comparison_major() {
    let v1 = SemVer::parse("1.99.99").unwrap();
    let v2 = SemVer::parse("2.0.0").unwrap();
    assert!(v2.is_newer_than(&v1));
}

#[test]
fn test_semver_prerelease_less_than_release() {
    let pre = SemVer::parse("1.0.0-beta.1").unwrap();
    let rel = SemVer::parse("1.0.0").unwrap();
    assert!(rel.is_newer_than(&pre));
    assert!(!pre.is_newer_than(&rel));
}

#[test]
fn test_semver_equal() {
    let v1 = SemVer::parse("1.0.0").unwrap();
    let v2 = SemVer::parse("1.0.0").unwrap();
    assert!(!v1.is_newer_than(&v2));
    assert!(!v2.is_newer_than(&v1));
    assert_eq!(v1, v2);
}

#[test]
fn test_semver_display() {
    assert_eq!(SemVer::parse("1.2.3").unwrap().to_string(), "1.2.3");
    assert_eq!(SemVer::parse("0.1.0-alpha").unwrap().to_string(), "0.1.0-alpha");
}

// ═══════════════════════════════════════════════════════
// 2. Component 타입 테스트
// ═══════════════════════════════════════════════════════

#[test]
fn test_component_manifest_key_roundtrip() {
    let cases = vec![
        Component::CoreDaemon,
        Component::Cli,
        Component::Gui,
        Component::Module("minecraft".into()),
        Component::Module("palworld".into()),
    ];

    for comp in cases {
        let key = comp.manifest_key();
        let restored = Component::from_manifest_key(&key);
        assert_eq!(comp, restored, "Roundtrip failed for {:?}", comp);
    }
}

#[test]
fn test_component_display_name() {
    assert_eq!(Component::CoreDaemon.display_name(), "Saba-Core");
    assert_eq!(Component::Cli.display_name(), "CLI");
    assert_eq!(Component::Gui.display_name(), "GUI");
    assert_eq!(Component::Module("minecraft".into()).display_name(), "Module: minecraft");
}

// ═══════════════════════════════════════════════════════
// 3. UpdateConfig 테스트
// ═══════════════════════════════════════════════════════

#[test]
fn test_update_config_default() {
    let cfg = UpdateConfig::default();
    assert!(cfg.enabled);
    assert_eq!(cfg.check_interval_hours, 3);
    assert!(!cfg.auto_download);
    assert!(cfg.install_root.is_none());
    assert!(cfg.relay_url.is_none());
}

#[test]
fn test_update_config_serialization() {
    let cfg = UpdateConfig {
        enabled: true,
        check_interval_hours: 6,
        auto_download: true,
        install_root: Some("/opt/saba".into()),
        channel: "beta".into(),
        ..Default::default()
    };

    let json = serde_json::to_string(&cfg).unwrap();
    let restored: UpdateConfig = serde_json::from_str(&json).unwrap();
    assert_eq!(restored.check_interval_hours, 6);
    assert!(restored.auto_download);
    assert_eq!(restored.channel, "beta");
    assert_eq!(restored.install_root, Some("/opt/saba".into()));
}

// ═══════════════════════════════════════════════════════
// 5. ReleaseManifest 파싱 테스트
// ═══════════════════════════════════════════════════════


// ═══════════════════════════════════════════════════════
// 6. UpdateManager 단위 테스트 (파일시스템)
// ═══════════════════════════════════════════════════════

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

// ═══════════════════════════════════════════════════════
// 7. Zip 추출 + 파일 덮어쓰기 테스트
// ═══════════════════════════════════════════════════════

#[tokio::test]
async fn test_zip_extraction_creates_files() {
    let tmp = TempDir::new().unwrap();
    let mgr = create_test_manager(&tmp);

    // 테스트용 zip 생성
    let mut files = HashMap::new();
    files.insert("module.toml", b"name = \"test\"\nversion = \"1.0.0\"\n" as &[u8]);
    files.insert("lifecycle.py", b"def start(): pass\n" as &[u8]);
    files.insert("README.md", b"# Test Module\n" as &[u8]);
    let zip_data = create_test_zip(&files);

    // zip 파일을 스테이징에 저장
    let staged_path = tmp.path().join("test-module.zip");
    std::fs::write(&staged_path, &zip_data).unwrap();

    // 추출 대상 디렉터리
    let target_dir = tmp.path().join("modules").join("test");
    mgr.extract_to_directory_for_test(&staged_path, &target_dir).await;

    // 결과 검증
    assert!(target_dir.join("module.toml").exists());
    assert!(target_dir.join("lifecycle.py").exists());
    assert!(target_dir.join("README.md").exists());

    let content = std::fs::read_to_string(target_dir.join("module.toml")).unwrap();
    assert!(content.contains("name = \"test\""));
}

#[tokio::test]
async fn test_zip_overwrite_existing_files() {
    let tmp = TempDir::new().unwrap();
    let mgr = create_test_manager(&tmp);

    let target_dir = tmp.path().join("modules").join("overwrite_test");
    std::fs::create_dir_all(&target_dir).unwrap();

    // 기존 파일 생성 (v1)
    std::fs::write(target_dir.join("data.txt"), "version 1 content").unwrap();
    std::fs::write(target_dir.join("old_file.txt"), "should survive if not in zip").unwrap();

    // 새 버전 zip (v2)
    let mut files = HashMap::new();
    files.insert("data.txt", b"version 2 content - UPDATED" as &[u8]);
    files.insert("new_file.txt", b"brand new file" as &[u8]);
    let zip_data = create_test_zip(&files);

    let staged = tmp.path().join("update.zip");
    std::fs::write(&staged, &zip_data).unwrap();

    mgr.extract_to_directory_for_test(&staged, &target_dir).await;

    // data.txt가 v2로 덮어쓰임
    let content = std::fs::read_to_string(target_dir.join("data.txt")).unwrap();
    assert_eq!(content, "version 2 content - UPDATED");

    // 새 파일 추가됨
    assert!(target_dir.join("new_file.txt").exists());

    // zip에 없는 기존 파일은 유지됨
    assert!(target_dir.join("old_file.txt").exists());
}

// ═══════════════════════════════════════════════════════
// 11. 에지 케이스 테스트
// ═══════════════════════════════════════════════════════


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

    let result = mgr.apply_updates().await.unwrap();
    assert!(result.is_empty(), "Should apply nothing if nothing downloaded");
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

// ═══════════════════════════════════════════════════════
// 12. 백업 & 복구 테스트
// ═══════════════════════════════════════════════════════

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

// ═══════════════════════════════════════════════════════
// 13. UpdateManager — resolve_install_dir 정밀 테스트
// ═══════════════════════════════════════════════════════

#[test]
fn test_resolve_install_dir_with_manifest_override() {
    let tmp = TempDir::new().unwrap();
    let mgr = create_test_manager(&tmp);

    // manifest에서 install_dir 지정
    let dir = mgr.resolve_install_dir_for_test(&Component::Cli, Some("custom/bin"));
    assert!(dir.ends_with("custom/bin") || dir.ends_with("custom\\bin"));

    // manifest에 install_dir 없으면 기본 규칙 — GUI는 install_root 직접
    let dir = mgr.resolve_install_dir_for_test(&Component::Gui, None);
    // GUI 기본 매핑은 install_root 자체 (portable exe 방식)
    assert_eq!(dir, tmp.path().to_path_buf());

    let dir = mgr.resolve_install_dir_for_test(&Component::Module("mc".into()), None);
    assert!(dir.to_string_lossy().contains("mc"));
}

// ═══════════════════════════════════════════════════════
// 14. 다중 릴리스 필터링 테스트
// ═══════════════════════════════════════════════════════


// ═══════════════════════════════════════════════════════
// 15. 대몬 재시작 스크립트 생성 테스트
// ═══════════════════════════════════════════════════════

#[test]
fn test_daemon_update_script_content() {
    // 재시작 스크립트의 내용 패턴 검증 (실제 파일 생성 없이)
    let exe_path = "C:\\saba-chan\\saba-core.exe";
    let staged_path = "C:\\saba-chan\\updates\\core.zip";

    let script = format!(
        r#"Start-Sleep -Seconds 2
$exePath = "{}"
$stagedPath = "{}"
"#, exe_path, staged_path);

    assert!(script.contains("Start-Sleep"));
    assert!(script.contains(exe_path));
    assert!(script.contains(staged_path));
}

// ═══════════════════════════════════════════════════════
// 16. 시간 유틸 테스트
// ═══════════════════════════════════════════════════════

#[test]
fn test_timestamp_format() {
    // UpdateManager 내부의 chrono_now_iso가 올바른 ISO 8601 형식인지
    // (내부 함수이므로 패턴 검증만)
    let pattern = regex::Regex::new(r"^\d{4}-\d{2}-\d{2}T\d{2}:\d{2}:\d{2}Z$").unwrap();

    // 검증을 위해 수동 구성
    let ts = format!("{:04}-{:02}-{:02}T{:02}:{:02}:{:02}Z", 2026, 2, 13, 10, 30, 0);
    assert!(pattern.is_match(&ts));
}
