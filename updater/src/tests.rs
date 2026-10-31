//! 업데이터 통합 테스트
//!
//! ## 테스트 시나리오
//! 1. 버전 페치: Mock 서버에서 릴리스 정보 가져오기

use crate::{Component, UpdateConfig, UpdateManager};

// ═══════════════════════════════════════════════════════
// 테스트 4: 백그라운드 워커
// ═══════════════════════════════════════════════════════

// ═══════════════════════════════════════════════════════
// 테스트 5: 유틸리티
// ═══════════════════════════════════════════════════════

#[test]
fn test_component_manifest_key() {
    assert_eq!(Component::CoreDaemon.manifest_key(), "saba-core");
    assert_eq!(Component::Cli.manifest_key(), "cli");
    assert_eq!(Component::Gui.manifest_key(), "gui");
    assert_eq!(
        Component::Module("minecraft".to_string()).manifest_key(),
        "module-minecraft"
    );
    assert_eq!(
        Component::Extension("docker".to_string()).manifest_key(),
        "ext-docker"
    );
    
    // 역방향 파싱
    assert_eq!(
        Component::from_manifest_key("saba-core"),
        Component::CoreDaemon
    );
    assert_eq!(
        Component::from_manifest_key("module-palworld"),
        Component::Module("palworld".to_string())
    );
    assert_eq!(
        Component::from_manifest_key("ext-music"),
        Component::Extension("music".to_string())
    );
    
    // display_name 확인
    assert_eq!(Component::CoreDaemon.display_name(), "Saba-Core");
    assert_eq!(Component::Extension("docker".to_string()).display_name(), "Extension: docker");
    
    println!("✓ 컴포넌트 매니페스트 키 테스트 통과");
}

// ═══════════════════════════════════════════════════════
// 테스트 8: 의존성 구조체 및 검증
// ═══════════════════════════════════════════════════════

#[test]
fn test_dependency_check_satisfied() {
    use crate::DependencyCheck;

    let check = DependencyCheck {
        component: "gui".to_string(),
        satisfied: true,
        issues: vec![],
    };
    assert!(check.satisfied);
    assert!(check.issues.is_empty());
}

#[test]
fn test_dependency_check_unsatisfied() {
    use crate::{DependencyCheck, DependencyIssue};

    let check = DependencyCheck {
        component: "gui".to_string(),
        satisfied: false,
        issues: vec![DependencyIssue {
            required_component: "saba-core".to_string(),
            required_version: ">=0.3.0".to_string(),
            installed_version: Some("0.2.0".to_string()),
            message: "gui requires saba-core >=0.3.0 but 0.2.0 is installed".to_string(),
        }],
    };
    assert!(!check.satisfied);
    assert_eq!(check.issues.len(), 1);
    assert_eq!(check.issues[0].required_component, "saba-core");
    assert_eq!(check.issues[0].installed_version.as_deref(), Some("0.2.0"));
}

#[test]
fn test_dependency_issue_not_installed() {
    use crate::DependencyIssue;

    let issue = DependencyIssue {
        required_component: "gui".to_string(),
        required_version: ">=0.1.0".to_string(),
        installed_version: None,
        message: "cli requires gui >=0.1.0 but not installed is installed".to_string(),
    };
    assert!(issue.installed_version.is_none());
}

#[test]
fn test_dependency_check_serde_roundtrip() {
    use crate::{DependencyCheck, DependencyIssue};

    let check = DependencyCheck {
        component: "module-minecraft".to_string(),
        satisfied: false,
        issues: vec![
            DependencyIssue {
                required_component: "saba-core".to_string(),
                required_version: ">=0.5.0".to_string(),
                installed_version: Some("0.3.0".to_string()),
                message: "version mismatch".to_string(),
            },
            DependencyIssue {
                required_component: "ext-steamcmd".to_string(),
                required_version: ">=1.0.0".to_string(),
                installed_version: None,
                message: "not installed".to_string(),
            },
        ],
    };

    let json = serde_json::to_string(&check).expect("serialize DependencyCheck");
    let deserialized: DependencyCheck =
        serde_json::from_str(&json).expect("deserialize DependencyCheck");
    assert_eq!(deserialized.component, "module-minecraft");
    assert!(!deserialized.satisfied);
    assert_eq!(deserialized.issues.len(), 2);
    assert_eq!(deserialized.issues[0].required_component, "saba-core");
    assert!(deserialized.issues[1].installed_version.is_none());
}

#[test]
fn test_component_info_requires_field() {
    use crate::ManifestComponent;

    let json = r#"{
        "version": "0.3.0",
        "asset": "gui-windows-x64.zip",
        "sha256": "00",
        "url": "https://r2.example/gui.zip",
        "requires": {
            "saba-core": ">=0.3.0",
            "cli": ">=0.1.0"
        }
    }"#;
    let info: ManifestComponent = serde_json::from_str(json).unwrap();
    assert_eq!(info.version, "0.3.0");
    let requires = info.requires.unwrap();
    assert_eq!(requires.len(), 2);
    assert_eq!(requires.get("saba-core").unwrap(), ">=0.3.0");
    assert_eq!(requires.get("cli").unwrap(), ">=0.1.0");
}

#[test]
fn test_component_info_requires_empty_by_default() {
    use crate::ManifestComponent;

    let json = r#"{"version": "0.1.0", "asset": "a.zip", "sha256": "00", "url": "https://r2.example/a.zip"}"#;
    let info: ManifestComponent = serde_json::from_str(json).unwrap();
    assert!(info.requires.is_none());
}

// ═══════════════════════════════════════════════════════
// 테스트: 모듈/익스텐션 핫로드 (재시작 불필요)
// ═══════════════════════════════════════════════════════

#[test]
fn test_module_extension_no_restart_in_apply_result() {
    use crate::ApplyComponentResult;

    // Module의 ApplyComponentResult는 restart_needed: false여야 함
    let module_result = ApplyComponentResult {
        component: "module-minecraft".to_string(),
        success: true,
        message: "Module 'minecraft' updated".to_string(),
        stopped_processes: Vec::new(),
        restart_needed: false,
    };
    assert!(!module_result.restart_needed, "모듈 업데이트는 재시작 불필요");

    // Extension의 ApplyComponentResult는 restart_needed: false여야 함
    let ext_result = ApplyComponentResult {
        component: "ext-docker".to_string(),
        success: true,
        message: "Extension 'docker' updated".to_string(),
        stopped_processes: Vec::new(),
        restart_needed: false,
    };
    assert!(!ext_result.restart_needed, "익스텐션 업데이트는 재시작 불필요");

    // CoreDaemon은 재시작 필요
    let daemon_result = ApplyComponentResult {
        component: "saba-core".to_string(),
        success: true,
        message: "Saba-Core updated (restart required)".to_string(),
        stopped_processes: Vec::new(),
        restart_needed: true,
    };
    assert!(daemon_result.restart_needed, "코어 데몬 업데이트는 재시작 필요");

    println!("✓ 모듈/익스텐션 핫로드 (restart_needed=false) 테스트 통과");
}

#[test]
fn test_component_apply_priority_categories() {
    // 모듈/익스텐션/Locales는 프로세스 중단 불필요 (우선순위 2)
    // GUI/CLI는 재시작 필요 (우선순위 4~5)
    // CoreDaemon은 데몬 재시작 필요 (우선순위 1)
    let module = Component::Module("minecraft".to_string());
    let extension = Component::Extension("docker".to_string());
    let locales = Component::Locales;
    let core_daemon = Component::CoreDaemon;
    let updater = Component::Updater;

    // 모듈, 익스텐션, Locales는 동일 우선순위
    assert_eq!(
        UpdateManager::component_apply_priority_for_test(&module),
        UpdateManager::component_apply_priority_for_test(&extension),
        "모듈과 익스텐션은 동일 우선순위"
    );
    assert_eq!(
        UpdateManager::component_apply_priority_for_test(&module),
        UpdateManager::component_apply_priority_for_test(&locales),
        "모듈과 Locales는 동일 우선순위"
    );

    // Updater는 최우선
    assert!(
        UpdateManager::component_apply_priority_for_test(&updater) <
        UpdateManager::component_apply_priority_for_test(&module),
        "Updater가 모듈보다 먼저 적용"
    );

    // 모듈은 새 코어를 요구할 수 있다(`requires`): CoreDaemon이 모듈/익스텐션보다 먼저
    assert!(
        UpdateManager::component_apply_priority_for_test(&core_daemon) <
        UpdateManager::component_apply_priority_for_test(&module),
        "CoreDaemon은 모듈보다 먼저 적용"
    );

    println!("✓ 컴포넌트 적용 우선순위 카테고리 테스트 통과");
}

#[cfg(test)]
mod run_all {
    use super::*;

    /// 유닛 테스트만 실행 (mock 서버 불필요)
    #[test]
    fn run_unit_tests() {
        test_component_manifest_key();
        test_dependency_check_satisfied();
        test_dependency_check_unsatisfied();
        test_dependency_issue_not_installed();
        test_dependency_check_serde_roundtrip();
        test_component_info_requires_field();
        test_component_info_requires_empty_by_default();
        test_module_extension_no_restart_in_apply_result();
        test_component_apply_priority_categories();
        println!("\n═══════════════════════════════════════");
        println!("✓ 모든 유닛 테스트 통과!");
        println!("═══════════════════════════════════════\n");
    }
}

#[test]
fn test_extract_zip_rejects_paths_outside_target() {
    use std::io::Write;
    let tmp = tempfile::tempdir().unwrap();
    let zip_path = tmp.path().join("evil.zip");
    {
        let mut zw = zip::ZipWriter::new(std::fs::File::create(&zip_path).unwrap());
        let opts = zip::write::SimpleFileOptions::default();
        zw.start_file("ok/inner.txt", opts).unwrap();
        zw.write_all(b"fine").unwrap();
        zw.start_file("../escaped.txt", opts).unwrap();
        zw.write_all(b"bad").unwrap();
        zw.finish().unwrap();
    }
    let target = tmp.path().join("out");
    std::fs::create_dir_all(&target).unwrap();

    let result = crate::extract_zip(&zip_path, &target, |_| Ok(()));
    assert!(result.is_err(), "archive with ../ entry must be rejected");
    assert!(!tmp.path().join("escaped.txt").exists());
}

#[test]
fn test_extract_zip_writes_nested_files() {
    use std::io::Write;
    let tmp = tempfile::tempdir().unwrap();
    let zip_path = tmp.path().join("ok.zip");
    {
        let mut zw = zip::ZipWriter::new(std::fs::File::create(&zip_path).unwrap());
        let opts = zip::write::SimpleFileOptions::default();
        zw.add_directory("a/", opts).unwrap();
        zw.start_file("a/b/c.txt", opts).unwrap();
        zw.write_all(b"hello").unwrap();
        zw.finish().unwrap();
    }
    let target = tmp.path().join("out");
    let mut seen = Vec::new();
    crate::extract_zip(&zip_path, &target, |p| {
        seen.push(p.to_path_buf());
        Ok(())
    })
    .unwrap();
    assert_eq!(std::fs::read_to_string(target.join("a/b/c.txt")).unwrap(), "hello");
    assert_eq!(seen, vec![target.join("a/b/c.txt")]);
}

// D3c: 리눅스 zip의 실행 파일(saba-core, saba-runner 등)은 실행 권한을 지닌 채 풀린다
#[cfg(unix)]
#[test]
fn extract_zip_keeps_the_executable_bit() {
    use std::io::Write;
    use std::os::unix::fs::PermissionsExt;
    let tmp = tempfile::tempdir().unwrap();
    let zip_path = tmp.path().join("core.zip");
    {
        let mut zw = zip::ZipWriter::new(std::fs::File::create(&zip_path).unwrap());
        zw.start_file("saba-runner", zip::write::SimpleFileOptions::default().unix_permissions(0o755)).unwrap();
        zw.write_all(b"\x7fELF").unwrap();
        zw.start_file("i18n.py", zip::write::SimpleFileOptions::default().unix_permissions(0o644)).unwrap();
        zw.write_all(b"x = 1").unwrap();
        zw.finish().unwrap();
    }
    let target = tmp.path().join("out");
    crate::extract_zip(&zip_path, &target, |_| Ok(())).unwrap();
    let mode = |name: &str| std::fs::metadata(target.join(name)).unwrap().permissions().mode() & 0o777;
    assert_eq!(mode("saba-runner") & 0o111, 0o111);
    assert_eq!(mode("i18n.py") & 0o111, 0);
}

// C13: 모듈 폴더 교체가 실패하면 원래 내용으로 되돌린다
#[test]
fn replace_dir_restores_backup_when_extract_fails() {
    let tmp = tempfile::tempdir().unwrap();
    let mut manager = UpdateManager::new(UpdateConfig::default(), tmp.path().join("modules").to_str().unwrap());
    manager.staging_dir = tmp.path().join("staging");
    std::fs::create_dir_all(&manager.staging_dir).unwrap();
    let target = tmp.path().join("modules").join("game");
    std::fs::create_dir_all(&target).unwrap();
    std::fs::write(target.join("module.toml"), "old").unwrap();
    let bad = manager.staging_dir.join("module-game.zip");
    std::fs::write(&bad, b"not a zip").unwrap();

    assert!(manager.replace_dir(&target, &bad, "game_backup").is_err());
    assert_eq!(std::fs::read_to_string(target.join("module.toml")).unwrap(), "old");
}

// C13: 받은 뒤 바뀐 파일은 적용하지 않는다
#[test]
fn staged_file_is_rechecked_before_apply() {
    let tmp = tempfile::tempdir().unwrap();
    let file = tmp.path().join("a.zip");
    std::fs::write(&file, b"abc").unwrap();
    let mut comp = crate::ComponentVersion {
        component: Component::Locales,
        current_version: "0.1.0".into(),
        latest_version: Some("0.2.0".into()),
        update_available: true,
        download_url: None,
        asset_name: None,
        release_notes: None,
        published_at: None,
        downloaded: true,
        downloaded_path: Some(file.to_string_lossy().to_string()),
        staged_sha256: Some("ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad".into()),
        requires: None,
        installed: true,
    };
    assert!(UpdateManager::verified_staged_path(&comp).is_ok());
    comp.staged_sha256 = Some("0".repeat(64));
    assert!(UpdateManager::verified_staged_path(&comp).is_err());
}

#[test]
fn same_path_ignores_dot_components() {
    let dir = std::path::Path::new("/opt/saba");
    assert!(super::same_path(&dir.join("saba-core.exe"), &dir.join("./saba-core.exe")));
    assert!(!super::same_path(&dir.join("a.exe"), &dir.join("b.exe")));
}

#[test]
fn extension_versions_come_from_manifest_json() {
    let dir = tempfile::tempdir().unwrap();
    let ext = dir.path().join("ue4_ini");
    std::fs::create_dir_all(&ext).unwrap();
    std::fs::write(ext.join("manifest.json"), r#"{"id": "ue4-ini", "version": "0.2.0"}"#).unwrap();
    assert_eq!(super::read_extension_version(&ext.join("manifest.json")), Some(("ue4-ini".into(), "0.2.0".into())));
}

// 업데이트가 끝나면 (실패한 것까지) 받아 둔 압축 파일과 .exe.old 백업이 남지 않는다
#[test]
fn finished_update_leaves_no_staged_archives_or_old_binaries() {
    let tmp = tempfile::tempdir().unwrap();
    let mut manager = UpdateManager::new(UpdateConfig::default(), tmp.path().join("modules").to_str().unwrap());
    manager.staging_dir = tmp.path().join("staging");
    manager.install_root = tmp.path().join("install");
    std::fs::create_dir_all(&manager.staging_dir).unwrap();
    std::fs::create_dir_all(&manager.install_root).unwrap();
    for f in ["pending.json", "saba-chan-gui-windows-x64.zip", "locales.zip.part"] {
        std::fs::write(manager.staging_dir.join(f), b"x").unwrap();
    }
    // 백업은 새 실행 파일이 제자리에 있을 때만 지운다
    for f in ["saba-core.exe.old", "saba-runner.exe.old.2", "saba-chan-gui.exe.old", "notepad.exe.old", "saba-core.exe.oldx", "saba-core.exe", "saba-runner.exe", crate::GUI_EXE_NAMES[0]] {
        std::fs::write(manager.install_root.join(f), b"x").unwrap();
    }

    manager.clear_pending_manifest();
    manager.cleanup_old_files();

    assert_eq!(std::fs::read_dir(&manager.staging_dir).unwrap().count(), 0);
    let mut left: Vec<_> = std::fs::read_dir(&manager.install_root).unwrap()
        .map(|e| e.unwrap().file_name().to_string_lossy().into_owned()).collect();
    left.sort();
    let mut want = vec!["notepad.exe.old", "saba-core.exe", "saba-core.exe.oldx", "saba-runner.exe", crate::GUI_EXE_NAMES[0]];
    want.sort();
    assert_eq!(left, want);
}

// GUI exe 이름이 바뀐 업데이트(0.1.1 `Saba-chan.exe` → `saba-chan-gui.exe`)는 예전 exe를 남기지 않는다
#[test]
fn a_renamed_gui_exe_retires_the_old_one() {
    let tmp = tempfile::tempdir().unwrap();
    let root = tmp.path();
    for f in ["Saba-chan.exe", "saba-chan-gui.exe", "saba-core.exe"] {
        std::fs::write(root.join(f), b"x").unwrap();
    }
    let names = ["saba-chan-gui.exe", "Saba-chan.exe"];
    let written = vec![root.join("saba-chan-gui.exe"), root.join("resources.pak")];

    let retired = UpdateManager::retire_other_exes(root, &written, &names);

    assert_eq!(retired, vec![(root.join("Saba-chan.exe"), root.join("saba-chan-gui.exe"))]);
    assert!(root.join("Saba-chan.exe.old").is_file() && !root.join("Saba-chan.exe").exists());
    assert!(root.join("saba-chan-gui.exe").is_file() && root.join("saba-core.exe").is_file());
    // GUI exe를 쓰지 않은 적용(디렉터리 모드 등)은 아무것도 건드리지 않는다
    assert!(UpdateManager::retire_other_exes(root, &[root.join("x.dll")], &names).is_empty());
}

// A5: 받아 둔 릴리즈의 버전·패치노트가 staging에 남아 업데이터가 완료 마커에 싣고, 정리하면 지워진다
#[test]
fn downloaded_release_notes_reach_the_completion_marker() {
    let tmp = tempfile::tempdir().unwrap();
    let mut manager = UpdateManager::new(UpdateConfig::default(), tmp.path().join("modules").to_str().unwrap());
    manager.staging_dir = tmp.path().join("staging");
    let file = tmp.path().join("locales.zip");
    std::fs::write(&file, b"x").unwrap();
    manager.status.components.push(crate::ComponentVersion {
        component: Component::Locales,
        current_version: "0.1.2".into(),
        latest_version: Some("0.1.3".into()),
        update_available: true,
        download_url: None,
        asset_name: None,
        release_notes: None,
        published_at: None,
        downloaded: true,
        downloaded_path: Some(file.to_string_lossy().to_string()),
        staged_sha256: None,
        requires: None,
        installed: true,
    });
    manager.latest_release = Some(crate::relay::LatestRelease {
        version: "0.1.3".into(),
        notes: Some([("ko".to_string(), "고침".to_string())].into()),
        ..Default::default()
    });

    manager.save_pending_manifest().unwrap();
    let release = manager.pending_release().expect("release summary saved with the pending manifest");
    assert_eq!((release.version.as_str(), release.notes_for("ko")), ("0.1.3", Some("고침")));

    // 데몬을 다시 켜 받아 둔 목록을 되살린 뒤 적용(업데이터 위임)해도 릴리즈 요약은 남는다 ('다음 실행 때 적용')
    let mut restarted = UpdateManager::new(UpdateConfig::default(), tmp.path().join("modules").to_str().unwrap());
    restarted.staging_dir = manager.staging_dir.clone();
    restarted.load_pending_manifest().unwrap();
    restarted.save_pending_manifest().unwrap();
    assert_eq!(restarted.pending_release().map(|r| r.version), Some("0.1.3".to_string()));

    manager.clear_pending_manifest();
    assert!(manager.pending_release().is_none());
}

// 받아 둔 파일은 다시 확인해도 그대로 적용 대기로 남는다(같은 버전이고 파일이 있을 때만)
#[test]
fn recheck_keeps_downloads_of_the_same_version() {
    let tmp = tempfile::tempdir().unwrap();
    let file = tmp.path().join("locales.zip");
    std::fs::write(&file, b"x").unwrap();
    let staged = |latest: &str, path: &std::path::Path| crate::ComponentVersion {
        component: Component::Locales,
        current_version: "0.1.5".into(),
        latest_version: Some(latest.into()),
        update_available: true,
        download_url: None,
        asset_name: None,
        release_notes: None,
        published_at: None,
        downloaded: true,
        downloaded_path: Some(path.to_string_lossy().to_string()),
        staged_sha256: Some("ab".into()),
        requires: None,
        installed: true,
    };
    let fresh = |latest: &str| crate::ComponentVersion { downloaded: false, downloaded_path: None, staged_sha256: None, ..staged(latest, &file) };

    let mut same = fresh("0.1.6");
    UpdateManager::keep_staged(&[staged("0.1.6", &file)], &mut same);
    assert!(same.downloaded);
    assert_eq!(same.staged_sha256.as_deref(), Some("ab"));

    let mut newer = fresh("0.1.7");
    UpdateManager::keep_staged(&[staged("0.1.6", &file)], &mut newer);
    assert!(!newer.downloaded && newer.downloaded_path.is_none());

    let mut gone = fresh("0.1.6");
    UpdateManager::keep_staged(&[staged("0.1.6", &tmp.path().join("missing.zip"))], &mut gone);
    assert!(!gone.downloaded);
}

// 지난 업데이트의 `.exe.old`가 아직 잡혀 있어도(떠 있는 서버의 saba-runner) 다음 업데이트가 막히지 않는다
#[test]
fn a_held_old_backup_does_not_block_the_next_replace() {
    let tmp = tempfile::tempdir().unwrap();
    let exe = tmp.path().join("saba-runner.exe");
    std::fs::write(&exe, b"new").unwrap();
    // 지울 수 없는 .exe.old 흉내: 비어 있지 않은 폴더는 remove_file도 덮어쓰는 rename도 실패한다
    let held = tmp.path().join("saba-runner.exe.old");
    std::fs::create_dir(&held).unwrap();
    std::fs::write(held.join("x"), b"x").unwrap();

    let backup = UpdateManager::move_aside(&exe).unwrap();

    assert_eq!(backup, Some(tmp.path().join("saba-runner.exe.old.1")));
    assert!(!exe.exists() && held.is_dir());
}

// 모르는 압축 형식을 받아 두었으면 아무것도 풀지 않고 성공으로 끝내지 않는다
#[tokio::test]
async fn an_unknown_archive_format_is_not_applied_as_success() {
    let tmp = tempfile::tempdir().unwrap();
    let mut manager = UpdateManager::new(UpdateConfig::default(), tmp.path().join("modules").to_str().unwrap());
    manager.install_root = tmp.path().join("install");
    std::fs::create_dir_all(&manager.install_root).unwrap();
    std::fs::write(manager.install_root.join(crate::GUI_EXE_NAMES[0]), b"old gui").unwrap();
    for name in ["saba-chan-updater-linux-x64.tgz", "saba-chan-gui-linux-x64.7z"] {
        std::fs::write(tmp.path().join(name), b"whatever").unwrap();
    }

    let core = tmp.path().join("saba-chan-updater-linux-x64.tgz");
    assert!(manager.apply_binary_update("saba-chan-updater", &core.to_string_lossy()).await.is_err());
    let gui = tmp.path().join("saba-chan-gui-linux-x64.7z");
    assert!(manager.apply_gui_update(&gui.to_string_lossy()).await.is_err());
}

// 0.1.6 설치기는 받은 모듈·익스텐션을 기록하지 않았다. 설치 기록이 없는 것은 같은 버전이라도
// 카탈로그에서 다시 받게 해서 공식 표시를 되찾게 한다 (기록이 있으면 사용자가 고친 것일 수 있어 건드리지 않는다)
#[test]
fn an_installed_addon_without_an_install_record_is_offered_again() {
    let tmp = tempfile::tempdir().unwrap();
    let file = tmp.path().join("lifecycle.py");
    std::fs::write(&file, b"print('ok')").unwrap();
    crate::integrity::record_install("module-recordedmod", &[file]).unwrap();

    let again = |c: Component, current: &str| UpdateManager::reinstall_from_catalog(&c, current, "0.1.2");
    assert!(again(Component::Module("unrecordedmod".into()), "0.1.2"));
    assert!(again(Component::Extension("unrecordedext".into()), "0.1.1"));
    assert!(!again(Component::Module("recordedmod".into()), "0.1.2"));
    assert!(!again(Component::CoreDaemon, "0.1.2"));
    assert!(!again(Component::Module("unrecordedmod".into()), "0.2.0"), "a newer local copy is not replaced by an older catalog one");
}

// GUI 교체 중 압축 해제가 실패하면 비켜 둔 GUI를 되돌린다 (그대로 두면 GUI가 사라진다)
#[tokio::test]
async fn a_failed_gui_extract_restores_the_previous_gui() {
    use std::io::Write;
    let tmp = tempfile::tempdir().unwrap();
    let mut manager = UpdateManager::new(UpdateConfig::default(), tmp.path().join("modules").to_str().unwrap());
    manager.install_root = tmp.path().join("install");
    std::fs::create_dir_all(&manager.install_root).unwrap();
    let gui = manager.install_root.join(crate::GUI_EXE_NAMES[0]);
    std::fs::write(&gui, b"old gui").unwrap();
    std::fs::write(manager.install_root.join("resources.pak"), b"old pak").unwrap();
    let zip_path = tmp.path().join("gui.zip");
    {
        let mut zw = zip::ZipWriter::new(std::fs::File::create(&zip_path).unwrap());
        let opts = zip::write::SimpleFileOptions::default();
        zw.start_file(crate::GUI_EXE_NAMES[0], opts).unwrap();
        zw.write_all(b"new gui").unwrap();
        // 실행 파일이 아닌 기존 파일과 새로 생기는 파일도 실패하면 원래대로 돌아가야 한다
        zw.start_file("resources.pak", opts).unwrap();
        zw.write_all(b"new pak").unwrap();
        zw.start_file("new.dll", opts).unwrap();
        zw.write_all(b"new dll").unwrap();
        zw.start_file("../escaped.txt", opts).unwrap();
        zw.write_all(b"bad").unwrap();
        zw.finish().unwrap();
    }

    assert!(manager.apply_gui_update(&zip_path.to_string_lossy()).await.is_err());
    assert_eq!(std::fs::read(&gui).unwrap(), b"old gui");
    assert_eq!(std::fs::read(manager.install_root.join("resources.pak")).unwrap(), b"old pak");
    let left: Vec<_> = std::fs::read_dir(&manager.install_root).unwrap().flatten().map(|e| e.file_name()).collect();
    assert_eq!(left.len(), 2, "no new file or backup left behind: {left:?}");
}

// 되돌리지 못해 실행 파일 없이 남은 `.exe.old`는 정리하지 않는다 (지우면 원래 GUI가 영영 사라진다)
#[test]
fn an_old_backup_without_its_exe_is_kept() {
    let tmp = tempfile::tempdir().unwrap();
    let mut manager = UpdateManager::new(UpdateConfig::default(), tmp.path().join("modules").to_str().unwrap());
    manager.install_root = tmp.path().to_path_buf();
    std::fs::write(tmp.path().join("saba-chan-gui.exe.old"), b"only copy").unwrap();
    std::fs::write(tmp.path().join("saba-core.exe"), b"new").unwrap();
    std::fs::write(tmp.path().join("saba-core.exe.old"), b"old").unwrap();

    manager.cleanup_old_files();

    assert!(tmp.path().join("saba-chan-gui.exe.old").exists());
    assert!(!tmp.path().join("saba-core.exe.old").exists());
}

fn staged(component: Component, path: &std::path::Path) -> crate::ComponentVersion {
    crate::ComponentVersion {
        component,
        current_version: "0.1.0".into(),
        latest_version: Some("0.2.0".into()),
        update_available: true,
        download_url: None,
        asset_name: None,
        release_notes: None,
        published_at: None,
        downloaded: true,
        downloaded_path: Some(path.to_string_lossy().to_string()),
        staged_sha256: None,
        requires: None,
        installed: true,
    }
}

// L3(a): 여러 컴포넌트를 적용하다 실패해도 먼저 적용한 것은 버전을 기록한다 (그대로 두면 다시 받으라고 뜬다)
#[tokio::test]
async fn components_applied_before_a_failure_are_recorded() {
    use std::io::Write;
    let tmp = tempfile::tempdir().unwrap();
    let mut manager = UpdateManager::new(UpdateConfig::default(), tmp.path().join("modules").to_str().unwrap());
    manager.install_root = tmp.path().join("install");
    manager.installed_manifest_file = tmp.path().join("installed-manifest.json");
    std::fs::create_dir_all(&manager.install_root).unwrap();
    let ext = Component::Extension(format!("l3-{}", std::process::id()));
    let ext_zip = tmp.path().join("ext.zip");
    {
        let mut zw = zip::ZipWriter::new(std::fs::File::create(&ext_zip).unwrap());
        zw.start_file("manifest.json", zip::write::SimpleFileOptions::default()).unwrap();
        zw.write_all(b"{}").unwrap();
        zw.finish().unwrap();
    }
    manager.status.components = vec![staged(ext.clone(), &ext_zip), staged(Component::Cli, &tmp.path().join("missing.zip"))];

    assert!(manager.apply_components(&[]).await.is_err());

    let recorded = UpdateManager::load_installed_manifest_at(&manager.installed_manifest_file);
    assert_eq!(recorded.get(&ext.manifest_key()).map(String::as_str), Some("0.2.0"));
    let _ = std::fs::remove_dir_all(manager.resolve_ext_dir(&format!("l3-{}", std::process::id())));
}

// L3(d): 새로 설치하다 실패하면 반쯤 풀린 폴더를 남기지 않는다
#[test]
fn a_failed_new_install_leaves_no_folder() {
    let tmp = tempfile::tempdir().unwrap();
    let mut manager = UpdateManager::new(UpdateConfig::default(), tmp.path().join("modules").to_str().unwrap());
    manager.staging_dir = tmp.path().join("staging");
    std::fs::create_dir_all(&manager.staging_dir).unwrap();
    let target = tmp.path().join("modules").join("fresh");
    let bad = manager.staging_dir.join("module-fresh.zip");
    {
        use std::io::Write;
        let mut zw = zip::ZipWriter::new(std::fs::File::create(&bad).unwrap());
        let opts = zip::write::SimpleFileOptions::default();
        zw.start_file("module.toml", opts).unwrap();
        zw.write_all(b"name = \"fresh\"").unwrap();
        zw.start_file("../escaped.txt", opts).unwrap();
        zw.write_all(b"bad").unwrap();
        zw.finish().unwrap();
    }

    assert!(manager.replace_dir(&target, &bad, "fresh_backup").is_err());
    assert!(!target.exists());
}

// EE4: 코어를 `.exe.old`로 비켜 둔 뒤 새 exe를 두기 전에 끊기면, 다음 정리는 가장 최근 사본을 되돌린다 (그대로면 데몬이 뜨지 못한다)
#[test]
fn an_exe_moved_aside_by_an_interrupted_apply_is_put_back() {
    let tmp = tempfile::tempdir().unwrap();
    let mut manager = UpdateManager::new(UpdateConfig::default(), tmp.path().join("modules").to_str().unwrap());
    manager.install_root = tmp.path().to_path_buf();
    std::fs::write(tmp.path().join("saba-core.exe.old"), b"older, still held").unwrap();
    std::fs::write(tmp.path().join("saba-core.exe.old.1"), b"latest").unwrap();
    std::fs::write(tmp.path().join("saba-chan-cli.exe"), b"new").unwrap();
    std::fs::write(tmp.path().join("saba-chan-cli.exe.old"), b"old cli").unwrap();
    std::fs::write(tmp.path().join("notepad.exe.old"), b"not ours").unwrap();

    manager.settle_interrupted_updates();

    assert_eq!(std::fs::read(tmp.path().join("saba-core.exe")).unwrap(), b"latest");
    assert_eq!(std::fs::read(tmp.path().join("saba-chan-cli.exe")).unwrap(), b"new", "an exe that is there stays");
    assert!(!tmp.path().join("notepad.exe").exists());
}

// L3(c): 적용 중 꺼져 남은 `.update-bak`은 다음 정리 때 원래 파일이 있으면 지우고, 없으면 되돌린다
#[test]
fn leftover_update_backups_are_cleaned_or_restored() {
    let tmp = tempfile::tempdir().unwrap();
    let mut manager = UpdateManager::new(UpdateConfig::default(), tmp.path().join("modules").to_str().unwrap());
    manager.install_root = tmp.path().to_path_buf();
    std::fs::write(tmp.path().join("resources.pak"), b"new").unwrap();
    std::fs::write(tmp.path().join("resources.pak.update-bak"), b"old").unwrap();
    std::fs::write(tmp.path().join("WebView2Loader.dll.update-bak"), b"only copy").unwrap();

    manager.settle_interrupted_updates();

    assert_eq!(std::fs::read(tmp.path().join("resources.pak")).unwrap(), b"new");
    assert!(!tmp.path().join("resources.pak.update-bak").exists());
    assert_eq!(std::fs::read(tmp.path().join("WebView2Loader.dll")).unwrap(), b"only copy");
    assert!(!tmp.path().join("WebView2Loader.dll.update-bak").exists());
}

// M4: 대상 폴더 안의 기존 링크(Windows는 정션 포함)를 따라 밖에 쓰지 않는다
#[cfg(unix)]
#[test]
fn extract_zip_does_not_write_through_links_in_the_target() {
    use std::io::Write;
    let tmp = tempfile::tempdir().unwrap();
    let target = tmp.path().join("target");
    let outside = tmp.path().join("outside");
    std::fs::create_dir_all(&target).unwrap();
    std::fs::create_dir_all(&outside).unwrap();
    std::fs::write(outside.join("victim.txt"), b"keep").unwrap();
    std::os::unix::fs::symlink(&outside, target.join("sub")).unwrap();
    std::os::unix::fs::symlink(outside.join("victim.txt"), target.join("file.txt")).unwrap();
    for name in ["sub/new.txt", "file.txt", "nope/../sub/new.txt"] {
        let zip_path = tmp.path().join("a.zip");
        {
            let mut zw = zip::ZipWriter::new(std::fs::File::create(&zip_path).unwrap());
            zw.start_file(name, zip::write::SimpleFileOptions::default()).unwrap();
            zw.write_all(b"bad").unwrap();
            zw.finish().unwrap();
        }
        assert!(crate::extract_zip(&zip_path, &target, |_| Ok(())).is_err(), "{name}");
    }
    assert!(!outside.join("new.txt").exists());
    assert_eq!(std::fs::read(outside.join("victim.txt")).unwrap(), b"keep");
}

// M4: tar의 링크 엔트리를 만들지 않고, 기존 링크를 따라 쓰지 않는다
#[cfg(unix)]
#[test]
fn extract_tar_gz_does_not_create_or_follow_links() {
    let tmp = tempfile::tempdir().unwrap();
    let target = tmp.path().join("target");
    let outside = tmp.path().join("outside");
    std::fs::create_dir_all(&target).unwrap();
    std::fs::create_dir_all(&outside).unwrap();
    let archive = tmp.path().join("a.tar.gz");
    {
        let gz = flate2::write::GzEncoder::new(std::fs::File::create(&archive).unwrap(), flate2::Compression::default());
        let mut tw = tar::Builder::new(gz);
        let mut link = tar::Header::new_gnu();
        link.set_entry_type(tar::EntryType::Symlink);
        link.set_size(0);
        tw.append_link(&mut link, "sub", &outside).unwrap();
        let mut file = tar::Header::new_gnu();
        file.set_size(3);
        file.set_mode(0o644);
        tw.append_data(&mut file, "sub/new.txt", &b"bad"[..]).unwrap();
        tw.into_inner().unwrap().finish().unwrap();
    }
    let _ = UpdateManager::extract_tar_gz(&archive, &target);
    assert!(!outside.join("new.txt").exists());
    assert!(!target.join("sub").is_symlink());

    std::fs::remove_dir_all(target.join("sub")).unwrap();
    std::os::unix::fs::symlink(&outside, target.join("sub")).unwrap();
    assert!(UpdateManager::extract_tar_gz(&archive, &target).is_err());
    assert!(!outside.join("new.txt").exists());
}

// M4: Windows 정션도 링크로 보고 따라 쓰지 않는다 (PC에서 실행)
#[cfg(windows)]
#[test]
fn extract_zip_does_not_write_through_a_junction() {
    use std::io::Write;
    let tmp = tempfile::tempdir().unwrap();
    let target = tmp.path().join("target");
    let outside = tmp.path().join("outside");
    std::fs::create_dir_all(&target).unwrap();
    std::fs::create_dir_all(&outside).unwrap();
    let made = std::process::Command::new(crate::process::system_tool("cmd"))
        .args(["/D", "/C", "mklink", "/J"])
        .arg(target.join("sub"))
        .arg(&outside)
        .status()
        .unwrap();
    assert!(made.success());
    let zip_path = tmp.path().join("a.zip");
    {
        let mut zw = zip::ZipWriter::new(std::fs::File::create(&zip_path).unwrap());
        zw.start_file("sub/new.txt", zip::write::SimpleFileOptions::default()).unwrap();
        zw.write_all(b"bad").unwrap();
        zw.finish().unwrap();
    }
    assert!(crate::extract_zip(&zip_path, &target, |_| Ok(())).is_err());
    assert!(!outside.join("new.txt").exists());
}

// L3(c): 새 파일은 `.update-new`에 다 쓴 뒤 이름을 바꾼다. 쓰다 꺼지면 원래 이름에는 아무것도 없어서
// 다음 정리가 `.update-bak`을 되돌리고 반쯤 쓴 `.update-new`를 지운다 (원래 이름에 깨진 파일이 남아 백업을 지우던 문제)
#[test]
fn an_interrupted_write_leaves_the_backup_to_be_restored() {
    use std::io::Write;
    let tmp = tempfile::tempdir().unwrap();
    let zip_path = tmp.path().join("a.zip");
    {
        let mut zw = zip::ZipWriter::new(std::fs::File::create(&zip_path).unwrap());
        zw.start_file("resources.pak", zip::write::SimpleFileOptions::default()).unwrap();
        zw.write_all(b"new").unwrap();
        zw.finish().unwrap();
    }
    let target = tmp.path().join("install");
    std::fs::create_dir_all(&target).unwrap();
    crate::extract_zip(&zip_path, &target, |_| Ok(())).unwrap();
    assert_eq!(std::fs::read(target.join("resources.pak")).unwrap(), b"new");
    assert!(!target.join("resources.pak.update-new").exists());

    // 쓰다 꺼진 상태: 원래 이름은 비켜 났고 반쯤 쓴 `.update-new`만 있다
    let mut manager = UpdateManager::new(UpdateConfig::default(), tmp.path().join("modules").to_str().unwrap());
    manager.install_root = target.clone();
    std::fs::remove_file(target.join("resources.pak")).unwrap();
    std::fs::write(target.join("resources.pak.update-bak"), b"old").unwrap();
    std::fs::write(target.join("resources.pak.update-new"), b"ne").unwrap();

    manager.settle_interrupted_updates();

    assert_eq!(std::fs::read(target.join("resources.pak")).unwrap(), b"old");
    assert!(!target.join("resources.pak.update-new").exists());
}

// L3(c): 데몬·업데이터가 뜰 때의 정리는 다른 프로세스가 적용하는 중일 수 있어 백업을 건드리지 않는다
#[test]
fn startup_cleanup_leaves_backups_of_a_running_apply_alone() {
    let tmp = tempfile::tempdir().unwrap();
    std::fs::write(tmp.path().join("resources.pak.update-bak"), b"old").unwrap();
    std::fs::write(tmp.path().join("resources.pak.update-new"), b"ne").unwrap();
    let mut manager = UpdateManager::new(UpdateConfig::default(), tmp.path().join("modules").to_str().unwrap());
    manager.install_root = tmp.path().to_path_buf();

    manager.cleanup_old_files();

    assert!(tmp.path().join("resources.pak.update-bak").exists());
    assert!(tmp.path().join("resources.pak.update-new").exists());
    assert!(!tmp.path().join("resources.pak").exists());
}

// M1: manifest의 `installer`도 업데이트 대상이다. 건너뛰면 설치 폴더의 제거기가 처음 설치한 판에 남는다
#[test]
fn the_installer_manifest_key_is_a_component() {
    assert_eq!(Component::try_from_manifest_key("installer"), Some(Component::Installer));
    assert_eq!(Component::Installer.manifest_key(), "installer");
}

// M1: 릴리스의 설치기는 zip이 아닌 exe 그대로다. 설치 폴더의 사본을 바꾼다
#[tokio::test]
async fn an_installer_update_replaces_the_installed_copy() {
    let tmp = tempfile::tempdir().unwrap();
    let mut manager = UpdateManager::new(UpdateConfig::default(), tmp.path().join("modules").to_str().unwrap());
    manager.install_root = tmp.path().join("install");
    std::fs::create_dir_all(&manager.install_root).unwrap();
    let target = manager.install_root.join(crate::INSTALLER_EXE);
    std::fs::write(&target, b"old installer").unwrap();
    assert!(manager.is_component_installed(&Component::Installer));
    let staged = tmp.path().join("saba-chan-installer.exe");
    std::fs::write(&staged, b"new installer").unwrap();

    let files = manager.apply_one(&Component::Installer, &staged.to_string_lossy()).await.unwrap();

    assert_eq!(std::fs::read(&target).unwrap(), b"new installer");
    assert_eq!(files, vec![target]);
    assert!(!staged.exists());
}

// M1: 바꾸다 실패하면 원래 설치기를 되돌린다 (제거기가 사라지면 안 된다)
#[tokio::test]
async fn a_failed_installer_update_keeps_the_old_copy() {
    let tmp = tempfile::tempdir().unwrap();
    let mut manager = UpdateManager::new(UpdateConfig::default(), tmp.path().join("modules").to_str().unwrap());
    manager.install_root = tmp.path().join("install");
    std::fs::create_dir_all(&manager.install_root).unwrap();
    let target = manager.install_root.join(crate::INSTALLER_EXE);
    std::fs::write(&target, b"old installer").unwrap();
    let missing = tmp.path().join("gone.exe");

    assert!(manager.apply_one(&Component::Installer, &missing.to_string_lossy()).await.is_err());

    assert_eq!(std::fs::read(&target).unwrap(), b"old installer");
    let left: Vec<_> = std::fs::read_dir(&manager.install_root).unwrap().flatten().map(|e| e.file_name()).collect();
    assert_eq!(left.len(), 1, "no backup left behind: {left:?}");
}

// M1: 0.1.7 이하 설치본은 설치기 버전을 기록하지 않았다. 번들 버전으로 채우면 최신으로 보여 영영 안 바뀌므로
// 기록이 없는 설치기는 버전을 모르는 채로 두고, 깔려 있으면 한 번 바꾼다
#[test]
fn an_unrecorded_installer_is_not_assumed_current() {
    let mut versions = std::collections::HashMap::new();
    UpdateManager::fill_bundled_versions(&mut versions);
    assert!(versions.contains_key("saba-core"));
    assert!(!versions.contains_key("installer"));

    assert!(UpdateManager::unknown_version_needs_update(&Component::Installer));
    assert!(!UpdateManager::unknown_version_needs_update(&Component::CoreDaemon));
}

// L3: 개별 적용(업데이터 키별 적용, 데몬 직접 적용)도 지난 적용이 남긴 백업을 되돌린다
#[tokio::test]
async fn a_single_component_apply_restores_a_backup_left_by_an_interrupted_apply() {
    let tmp = tempfile::tempdir().unwrap();
    let mut manager = UpdateManager::new(UpdateConfig::default(), tmp.path().join("modules").to_str().unwrap());
    manager.install_root = tmp.path().to_path_buf();
    std::fs::write(tmp.path().join("resources.pak.update-bak"), b"old").unwrap();

    // 받아 둔 업데이트가 없어 적용은 실패하지만, 정리는 그 앞에서 한다
    assert!(manager.apply_single_component(&Component::Locales).await.is_err());

    assert_eq!(std::fs::read(tmp.path().join("resources.pak")).unwrap(), b"old");
    assert!(!tmp.path().join("resources.pak.update-bak").exists());
}

// U-10: 상태의 패치노트도 사용자 언어로 고른다 (예전에는 늘 한국어, 없으면 영어)
#[test]
fn status_release_notes_follow_the_user_language() {
    let latest = crate::relay::LatestRelease {
        version: "0.2.0".into(),
        notes: Some([("ko", "한"), ("en", "E"), ("ja", "J"), ("zh", "Z")].iter().map(|(k, v)| (k.to_string(), v.to_string())).collect()),
        ..Default::default()
    };
    assert_eq!(UpdateManager::release_notes_in(&latest, "ja").as_deref(), Some("J"));
    assert_eq!(UpdateManager::release_notes_in(&latest, "zh-CN").as_deref(), Some("Z"));
    assert_eq!(UpdateManager::release_notes_in(&latest, "de").as_deref(), Some("E"));
}

/// 한 번만 응답하는 가짜 다운로드 서버
async fn serve_once(response: Vec<u8>) -> String {
    use tokio::io::{AsyncReadExt, AsyncWriteExt};
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    tokio::spawn(async move {
        let (mut sock, _) = listener.accept().await.unwrap();
        let mut buf = [0u8; 1024];
        let _ = sock.read(&mut buf).await;
        let _ = sock.write_all(&response).await;
    });
    format!("http://{}/asset.zip", addr)
}

// U-4: 여러 개를 받다 하나가 실패해도 이미 받은 것은 받은 것으로 남긴다 (그래야 pending에 실린다)
#[tokio::test]
async fn a_failed_download_keeps_the_ones_already_downloaded() {
    let tmp = tempfile::tempdir().unwrap();
    let mut manager = UpdateManager::new(UpdateConfig::default(), tmp.path().join("modules").to_str().unwrap());
    manager.staging_dir = tmp.path().join("staging");
    let body = b"cli bytes";
    let good = serve_once([format!("HTTP/1.1 200 OK\r\nContent-Length: {}\r\n\r\n", body.len()).into_bytes(), body.to_vec()].concat()).await;
    let bad = serve_once(b"HTTP/1.1 404 Not Found\r\nContent-Length: 0\r\n\r\n".to_vec()).await;
    let entry = |name: &str, url: String, sha: &str| crate::relay::ManifestComponent {
        version: "0.2.0".into(), filename: name.into(), size_bytes: None, sha256: sha.into(), url,
        install_dir: None, requires: None, meta: None,
    };
    let sha = {
        use sha2::Digest;
        sha2::Sha256::digest(body).iter().map(|b| format!("{:02x}", b)).collect::<String>()
    };
    manager.cached_manifest = Some(crate::relay::ReleaseManifest {
        version: "0.2.0".into(), channel: "stable".into(), platform: "windows-x64".into(), released_at: None, is_critical: false,
        components: [("gui".to_string(), entry("gui.zip", bad, &sha)), ("cli".to_string(), entry("cli.zip", good, &sha))].into_iter().collect(),
        platforms: Default::default(),
    });
    let pending = |component: Component, asset: &str| crate::ComponentVersion {
        asset_name: Some(asset.into()), downloaded: false, downloaded_path: None,
        ..staged(component, tmp.path())
    };
    manager.status.components = vec![pending(Component::Gui, "gui.zip"), pending(Component::Cli, "cli.zip")];

    // 데몬이 받은 것과 실패한 것을 응답에 함께 싣는다
    let (downloaded, failed) = manager.download_available_updates_report().await.unwrap();
    assert_eq!(downloaded, vec!["cli.zip".to_string()]);
    assert_eq!(failed.iter().map(|(k, _)| k.as_str()).collect::<Vec<_>>(), vec!["gui"]);

    let cli = manager.status.components.iter().find(|c| c.component == Component::Cli).unwrap();
    assert!(cli.downloaded, "cli was downloaded before gui failed");
    assert_eq!(cli.staged_sha256.as_deref(), Some(sha.as_str()));
    assert!(!manager.status.components.iter().find(|c| c.component == Component::Gui).unwrap().downloaded);
}

fn extension_zip(dir: &std::path::Path) -> std::path::PathBuf {
    use std::io::Write;
    let path = dir.join("ext.zip");
    let mut zw = zip::ZipWriter::new(std::fs::File::create(&path).unwrap());
    zw.start_file("manifest.json", zip::write::SimpleFileOptions::default()).unwrap();
    zw.write_all(b"{}").unwrap();
    zw.finish().unwrap();
    path
}

// U-3: 데몬이 정한 목록으로 적용할 때도 첫 실패에서 멈춘다 (데몬이 실패했는데 뒤의 것만 바뀌지 않게)
#[tokio::test]
async fn target_keys_stop_at_the_first_failure() {
    let tmp = tempfile::tempdir().unwrap();
    let mut manager = UpdateManager::new(UpdateConfig::default(), tmp.path().join("modules").to_str().unwrap());
    manager.install_root = tmp.path().join("install");
    std::fs::create_dir_all(&manager.install_root).unwrap();
    let name = format!("u3-{}", std::process::id());
    let ext = Component::Extension(name.clone());
    manager.status.components = vec![
        staged(Component::CoreDaemon, &tmp.path().join("missing.zip")),
        staged(ext.clone(), &extension_zip(tmp.path())),
    ];
    let keys = vec!["saba-core".to_string(), ext.manifest_key()];

    let (applied, failed) = manager.apply_target_keys(&keys, |_, _, _| {}).await;

    assert!(applied.is_empty(), "{:?}", applied);
    assert_eq!(failed.len(), 2, "{:?}", failed);
    assert!(failed[1].contains("skipped"), "{:?}", failed);
    assert!(!manager.resolve_ext_dir(&name).exists());
}

// 옛 데몬은 모듈을 코어보다 앞에 적어 넘긴다: 업데이터가 적용 순서로 다시 줄 세워 코어부터 적용한다
#[tokio::test]
async fn target_keys_apply_the_core_before_modules() {
    let tmp = tempfile::tempdir().unwrap();
    let mut manager = UpdateManager::new(UpdateConfig::default(), tmp.path().join("modules").to_str().unwrap());
    manager.install_root = tmp.path().join("install");
    std::fs::create_dir_all(&manager.install_root).unwrap();
    let name = format!("order-{}", std::process::id());
    let ext = Component::Extension(name.clone());
    manager.status.components = vec![
        staged(ext.clone(), &extension_zip(tmp.path())),
        staged(Component::CoreDaemon, &tmp.path().join("missing.zip")),
    ];
    let keys = vec![ext.manifest_key(), "saba-core".to_string()];

    let (applied, failed) = manager.apply_target_keys(&keys, |_, _, _| {}).await;

    assert!(applied.is_empty(), "{:?}", applied);
    assert!(failed[0].starts_with("saba-core:"), "{:?}", failed);
    assert!(failed[1].contains("skipped"), "{:?}", failed);
    assert!(!manager.resolve_ext_dir(&name).exists());
}

// 새 코어를 요구하는 모듈·익스텐션은 코어가 그 버전이 아니면 적용하지 않는다 (0.1.8은 requires를 보지 않았다)
#[tokio::test]
async fn an_update_that_needs_a_newer_core_is_refused() {
    let tmp = tempfile::tempdir().unwrap();
    let mut manager = UpdateManager::new(UpdateConfig::default(), tmp.path().join("modules").to_str().unwrap());
    manager.install_root = tmp.path().join("install");
    std::fs::create_dir_all(&manager.install_root).unwrap();
    let name = format!("needs-core-{}", std::process::id());
    let ext = Component::Extension(name.clone());
    manager.status.components = vec![crate::ComponentVersion {
        requires: Some([("saba-core".to_string(), ">=99.0.0".to_string())].into_iter().collect()),
        ..staged(ext.clone(), &extension_zip(tmp.path()))
    }];

    let (applied, failed) = manager.apply_target_keys(&[ext.manifest_key()], |_, _, _| {}).await;

    assert!(applied.is_empty(), "{:?}", applied);
    assert!(failed[0].contains("saba-core >=99.0.0"), "{:?}", failed);
    assert!(!manager.resolve_ext_dir(&name).exists());
}

// 업데이터 `--apply`는 네트워크 없이 pending.json만 읽는다: requires도 거기에 실려야 한다
#[test]
fn pending_manifest_keeps_requires() {
    let tmp = tempfile::tempdir().unwrap();
    let mut manager = UpdateManager::new(UpdateConfig::default(), tmp.path().join("modules").to_str().unwrap());
    manager.staging_dir = tmp.path().join("staging");
    let file = tmp.path().join("m.zip");
    std::fs::write(&file, b"zip").unwrap();
    let requires: std::collections::HashMap<String, String> = [("saba-core".to_string(), ">=0.1.9".to_string())].into_iter().collect();
    manager.status.components = vec![crate::ComponentVersion { requires: Some(requires.clone()), ..staged(Component::Module("m".into()), &file) }];
    manager.save_pending_manifest().unwrap();

    let mut loaded = UpdateManager::new(UpdateConfig::default(), tmp.path().join("modules").to_str().unwrap());
    loaded.staging_dir = manager.staging_dir.clone();
    loaded.load_pending_manifest().unwrap();
    assert_eq!(loaded.status.components[0].requires.as_ref(), Some(&requires));
}

#[test]
fn unmet_requires_compares_installed_versions() {
    let requires: std::collections::HashMap<String, String> =
        [("saba-core".to_string(), ">=0.1.9".to_string()), ("gui".to_string(), ">=0.1.0".to_string())].into_iter().collect();
    let installed = |core: &str| -> std::collections::HashMap<String, String> {
        [("saba-core".to_string(), core.to_string())].into_iter().collect()
    };
    let unmet = crate::unmet_requires("module-x", &requires, &installed("0.1.8"));
    let mut names: Vec<_> = unmet.iter().map(|i| i.required_component.as_str()).collect();
    names.sort();
    assert_eq!(names, vec!["gui", "saba-core"]);
    let unmet = crate::unmet_requires("module-x", &requires, &installed("0.1.9"));
    assert_eq!(unmet.iter().map(|i| i.required_component.as_str()).collect::<Vec<_>>(), vec!["gui"]);
}

// 목록 없는 적용도 키별로 적용·건너뜀을 남기도록, 받아 둔 것을 적용 순서대로 키 목록으로 준다
#[test]
fn pending_keys_come_in_apply_order() {
    let tmp = tempfile::tempdir().unwrap();
    let mut manager = UpdateManager::new(UpdateConfig::default(), tmp.path().join("modules").to_str().unwrap());
    let ext = Component::Extension("order".into());
    manager.status.components = vec![
        staged(Component::CoreDaemon, &tmp.path().join("core.zip")),
        staged(ext.clone(), &tmp.path().join("ext.zip")),
    ];

    assert_eq!(manager.pending_keys_in_apply_order(), vec!["saba-core".to_string(), ext.manifest_key()]);
}

// U-9: 목록 없는 적용이 실패해도 그 전에 적용한 것을 돌려준다 (업데이터가 완료 마커에 싣고 pending을 지운다)
#[tokio::test]
async fn a_failed_apply_reports_what_it_applied_before() {
    let tmp = tempfile::tempdir().unwrap();
    let mut manager = UpdateManager::new(UpdateConfig::default(), tmp.path().join("modules").to_str().unwrap());
    manager.install_root = tmp.path().join("install");
    std::fs::create_dir_all(&manager.install_root).unwrap();
    let name = format!("u9-{}", std::process::id());
    let ext = Component::Extension(name.clone());
    manager.status.components = vec![staged(ext.clone(), &extension_zip(tmp.path())), staged(Component::Cli, &tmp.path().join("missing.zip"))];

    let (applied, failure) = manager.apply_components_report(&[]).await;

    assert!(failure.is_some());
    assert_eq!(applied, vec![ext.display_name()]);
    let _ = std::fs::remove_dir_all(manager.resolve_ext_dir(&name));
}

// U-5: locales도 모듈처럼 백업 뒤 통째로 바꾼다. 새 판에 없는 옛 파일은 남지 않고, 풀다 실패하면 되돌린다
#[tokio::test]
async fn locales_are_replaced_with_a_backup() {
    use std::io::Write;
    let tmp = tempfile::tempdir().unwrap();
    let mut manager = UpdateManager::new(UpdateConfig::default(), tmp.path().join("modules").to_str().unwrap());
    manager.install_root = tmp.path().join("install");
    manager.staging_dir = tmp.path().join("staging");
    let locales = manager.install_root.join("locales");
    std::fs::create_dir_all(locales.join("ko")).unwrap();
    std::fs::write(locales.join("ko/old.json"), b"{}").unwrap();

    let broken = tmp.path().join("broken.zip");
    std::fs::write(&broken, b"not a zip").unwrap();
    assert!(manager.apply_one(&Component::Locales, broken.to_str().unwrap()).await.is_err());
    assert!(locales.join("ko/old.json").exists(), "a failed apply restores the old locales");

    let zip_path = tmp.path().join("locales.zip");
    {
        let mut zw = zip::ZipWriter::new(std::fs::File::create(&zip_path).unwrap());
        zw.start_file("ko/gui.json", zip::write::SimpleFileOptions::default()).unwrap();
        zw.write_all(b"{\"a\":1}").unwrap();
        zw.finish().unwrap();
    }
    manager.apply_one(&Component::Locales, zip_path.to_str().unwrap()).await.unwrap();
    assert!(locales.join("ko/gui.json").exists());
    assert!(!locales.join("ko/old.json").exists(), "files dropped from the new release are removed");
}

// L3(b): tar.gz로 받은 GUI도 풀다 실패하면 zip처럼 원래대로 되돌린다
#[tokio::test]
async fn a_failed_tar_gz_gui_extract_restores_the_previous_gui() {
    let tmp = tempfile::tempdir().unwrap();
    let mut manager = UpdateManager::new(UpdateConfig::default(), tmp.path().join("modules").to_str().unwrap());
    manager.install_root = tmp.path().join("install");
    std::fs::create_dir_all(&manager.install_root).unwrap();
    let gui = manager.install_root.join(crate::GUI_EXE_NAMES[0]);
    std::fs::write(&gui, b"old gui").unwrap();
    std::fs::write(manager.install_root.join("resources.pak"), b"old pak").unwrap();
    let tar_path = tmp.path().join("gui.tar.gz");
    {
        let gz = flate2::write::GzEncoder::new(std::fs::File::create(&tar_path).unwrap(), flate2::Compression::default());
        let mut tb = tar::Builder::new(gz);
        let mut add = |name: &str, data: &[u8]| {
            let mut h = tar::Header::new_gnu();
            h.set_size(data.len() as u64);
            h.set_mode(0o644);
            h.set_cksum();
            tb.append_data(&mut h, name, data).unwrap();
        };
        add(crate::GUI_EXE_NAMES[0], b"new gui");
        add("resources.pak", b"new pak");
        add("new.dll", b"new dll");
        // new.dll은 파일이라 그 아래에 쓸 수 없다: 여기서 실패한다
        add("new.dll/inner.txt", b"x");
        tb.into_inner().unwrap().finish().unwrap();
    }

    assert!(manager.apply_gui_update(&tar_path.to_string_lossy()).await.is_err());
    assert_eq!(std::fs::read(&gui).unwrap(), b"old gui");
    assert_eq!(std::fs::read(manager.install_root.join("resources.pak")).unwrap(), b"old pak");
    let left: Vec<_> = std::fs::read_dir(&manager.install_root).unwrap().flatten().map(|e| e.file_name()).collect();
    assert_eq!(left.len(), 2, "no new file or backup left behind: {left:?}");
}

// 카탈로그·수동 설치처럼 설치 기록에 없어도 디스크에 있는 모듈은 requires를 채운다
#[test]
fn requires_count_a_module_found_on_disk() {
    let tmp = tempfile::tempdir().unwrap();
    let modules = tmp.path().join("modules");
    std::fs::create_dir_all(modules.join("ondisk")).unwrap();
    std::fs::write(modules.join("ondisk/module.toml"), "[module]\nname = \"ondisk\"\nversion = \"1.2.0\"\n").unwrap();
    let mut manager = UpdateManager::new(UpdateConfig::default(), modules.to_str().unwrap());
    let ext = Component::Extension("needs-module".into());
    manager.status.components = vec![crate::ComponentVersion {
        requires: Some([("module-ondisk".to_string(), ">=1.0.0".to_string())].into_iter().collect()),
        ..staged(ext.clone(), &tmp.path().join("ext.zip"))
    }];

    assert!(manager.check_dependencies(&ext.manifest_key()).satisfied);
}

// 같은 묶음 안에서 requires하는 것을 먼저 적용한다 (같은 순위는 받은 순서보다 의존 관계가 앞선다)
#[test]
fn a_dependency_in_the_same_batch_is_applied_first() {
    let tmp = tempfile::tempdir().unwrap();
    let mut manager = UpdateManager::new(UpdateConfig::default(), tmp.path().join("modules").to_str().unwrap());
    let (a, b) = (Component::Extension("a".into()), Component::Extension("b".into()));
    manager.status.components = vec![
        crate::ComponentVersion {
            requires: Some([(b.manifest_key(), ">=1.0.0".to_string())].into_iter().collect()),
            ..staged(a.clone(), &tmp.path().join("a.zip"))
        },
        staged(b.clone(), &tmp.path().join("b.zip")),
        staged(Component::CoreDaemon, &tmp.path().join("core.zip")),
    ];

    assert_eq!(manager.pending_keys_in_apply_order(), vec!["saba-core".to_string(), b.manifest_key(), a.manifest_key()]);
}

// 목록 없는 적용(`apply_components_report`)도 requires를 지킨다
#[tokio::test]
async fn an_unlisted_apply_also_refuses_unmet_requires() {
    let tmp = tempfile::tempdir().unwrap();
    let mut manager = UpdateManager::new(UpdateConfig::default(), tmp.path().join("modules").to_str().unwrap());
    manager.install_root = tmp.path().join("install");
    std::fs::create_dir_all(&manager.install_root).unwrap();
    let name = format!("unlisted-needs-core-{}", std::process::id());
    let ext = Component::Extension(name.clone());
    manager.status.components = vec![crate::ComponentVersion {
        requires: Some([("saba-core".to_string(), ">=99.0.0".to_string())].into_iter().collect()),
        ..staged(ext.clone(), &extension_zip(tmp.path()))
    }];

    let (applied, failure) = manager.apply_components_report(&[]).await;

    assert!(applied.is_empty(), "{:?}", applied);
    assert!(format!("{:#}", failure.unwrap()).contains("saba-core >=99.0.0"));
    assert!(!manager.resolve_ext_dir(&name).exists());
}

// 같은 묶음에서 먼저 적용한 것은 기록 전이어도 뒤의 requires를 채운다
#[tokio::test]
async fn an_unlisted_apply_counts_what_it_applied_earlier() {
    let tmp = tempfile::tempdir().unwrap();
    let mut manager = UpdateManager::new(UpdateConfig::default(), tmp.path().join("modules").to_str().unwrap());
    manager.install_root = tmp.path().join("install");
    std::fs::create_dir_all(&manager.install_root).unwrap();
    let (first, second) = (format!("first-{}", std::process::id()), format!("second-{}", std::process::id()));
    let (a, b) = (Component::Extension(first.clone()), Component::Extension(second.clone()));
    let zip_dir = |name: &str| {
        let dir = tmp.path().join(name);
        std::fs::create_dir_all(&dir).unwrap();
        dir
    };
    manager.status.components = vec![
        crate::ComponentVersion {
            requires: Some([(a.manifest_key(), ">=9.9.9".to_string())].into_iter().collect()),
            ..staged(b.clone(), &extension_zip(&zip_dir("b")))
        },
        crate::ComponentVersion { latest_version: Some("9.9.9".into()), ..staged(a.clone(), &extension_zip(&zip_dir("a"))) },
    ];

    let (applied, failure) = manager.apply_components_report(&[]).await;

    assert!(failure.is_none(), "{:?}", failure);
    assert_eq!(applied, vec![a.display_name(), b.display_name()]);
    let _ = std::fs::remove_dir_all(manager.resolve_ext_dir(&first));
    let _ = std::fs::remove_dir_all(manager.resolve_ext_dir(&second));
}

// 카탈로그로 지우거나 바꾼 모듈은 설치 기록에 옛 값이 남는다: requires는 디스크에 있는 것만 센다
#[test]
fn requires_ignore_recorded_addons_missing_from_disk() {
    let recorded: std::collections::HashMap<String, String> = [("saba-core", "0.1.9"), ("module-gone", "1.0.0"), ("module-kept", "1.0.0")]
        .into_iter()
        .map(|(k, v)| (k.to_string(), v.to_string()))
        .collect();
    let disk = [("module-kept".to_string(), "2.0.0".to_string())].into_iter().collect();

    let versions = crate::merge_addons_from_disk(recorded, disk);

    assert_eq!(versions.get("saba-core").map(String::as_str), Some("0.1.9"));
    assert_eq!(versions.get("module-kept").map(String::as_str), Some("2.0.0"), "the version on disk wins");
    assert!(!versions.contains_key("module-gone"));
}

// 헤드리스 설치에는 GUI가 없으니 requires의 GUI 버전도 채워지지 않는다
#[test]
fn a_headless_install_does_not_count_the_bundled_gui_for_requires() {
    let tmp = tempfile::tempdir().unwrap();
    let manager = UpdateManager::new(UpdateConfig::default(), tmp.path().join("modules").to_str().unwrap());

    assert!(manager.dependency_versions_for(crate::profile::Profile::Desktop).contains_key("gui"));
    let headless = manager.dependency_versions_for(crate::profile::Profile::Headless);
    assert!(!headless.contains_key("gui"));
    assert!(headless.contains_key("saba-core"));
}

// 업데이터 `--apply` 한 번(창·창 없음 공통): 받아 둔 것을 적용하고 목록을 지운다
#[tokio::test]
async fn apply_staged_applies_the_pending_list_and_clears_it() {
    let tmp = tempfile::tempdir().unwrap();
    let mut manager = UpdateManager::new(UpdateConfig::default(), tmp.path().join("modules").to_str().unwrap());
    manager.install_root = tmp.path().join("install");
    manager.staging_dir = tmp.path().join("staging");
    std::fs::create_dir_all(&manager.install_root).unwrap();
    let name = format!("staged-{}", std::process::id());
    let ext = Component::Extension(name.clone());
    manager.status.components = vec![staged(ext.clone(), &extension_zip(tmp.path()))];
    manager.save_pending_manifest().unwrap();
    manager.status.components.clear();

    let mut steps = Vec::new();
    let (applied, failed) = manager.apply_staged(|p| steps.push((p.step.clone(), p.failed))).await.unwrap();

    assert_eq!(applied, vec![ext.manifest_key()]);
    assert!(failed.is_empty(), "{:?}", failed);
    assert_eq!(steps.last(), Some(&("complete".to_string(), 0)));
    assert!(!manager.staging_dir.join("pending.json").exists(), "the pending list is cleared");
    let _ = std::fs::remove_dir_all(manager.resolve_ext_dir(&name));
}

#[tokio::test]
async fn apply_staged_without_a_pending_list_fails() {
    let tmp = tempfile::tempdir().unwrap();
    let mut manager = UpdateManager::new(UpdateConfig::default(), tmp.path().join("modules").to_str().unwrap());
    manager.staging_dir = tmp.path().join("staging");

    let mut last = None;
    let result = manager.apply_staged(|p| last = Some((p.step.clone(), p.failed))).await;

    assert!(result.unwrap_err().contains("Failed to load manifest"));
    assert_eq!(last, Some(("error".to_string(), 1)));
}

// 받아 둔 뒤 설치기로 같거나 더 새 판을 깔았으면, 되살린 목록의 그 항목은 적용하지 않는다 (옛 판으로 되돌리지 않게)
#[test]
fn a_restored_pending_list_skips_what_is_already_installed_at_that_version_or_newer() {
    let tmp = tempfile::tempdir().unwrap();
    let file = tmp.path().join("x.zip");
    std::fs::write(&file, b"zip").unwrap();
    let mut manager = UpdateManager::new(UpdateConfig::default(), tmp.path().join("modules").to_str().unwrap());
    manager.staging_dir = tmp.path().join("staging");
    manager.status.components = vec![staged(Component::Locales, &file), staged(Component::Module("m".into()), &file)];
    manager.save_pending_manifest().unwrap();

    let mut loaded = UpdateManager::new(UpdateConfig::default(), tmp.path().join("modules").to_str().unwrap());
    loaded.staging_dir = manager.staging_dir.clone();
    loaded.installed_manifest_file = tmp.path().join("installed-manifest.json");
    let installed: std::collections::HashMap<String, String> =
        [(Component::Locales.manifest_key(), "0.3.0".to_string()), (Component::Module("m".into()).manifest_key(), "0.1.0".to_string())].into_iter().collect();
    std::fs::write(&loaded.installed_manifest_file, serde_json::to_string(&installed).unwrap()).unwrap();
    assert_eq!(loaded.load_pending_manifest().unwrap(), 1);
    assert_eq!(loaded.status.components.iter().map(|c| c.component.clone()).collect::<Vec<_>>(), vec![Component::Module("m".into())]);
}

// 적용 대상 목록을 남긴 뒤 설치기로 그중 하나를 같거나 더 새 판으로 깔았으면, 그 키는 건너뛰고 나머지는 적용한다
#[tokio::test]
async fn an_apply_target_already_installed_is_skipped_and_the_rest_are_applied() {
    let tmp = tempfile::tempdir().unwrap();
    let mut manager = UpdateManager::new(UpdateConfig::default(), tmp.path().join("modules").to_str().unwrap());
    manager.install_root = tmp.path().join("install");
    manager.staging_dir = tmp.path().join("staging");
    manager.installed_manifest_file = tmp.path().join("installed-manifest.json");
    std::fs::create_dir_all(&manager.install_root).unwrap();
    let name = format!("targets-{}", std::process::id());
    let ext = Component::Extension(name.clone());
    let zip = extension_zip(tmp.path());
    manager.status.components = vec![staged(Component::Locales, &zip), staged(ext.clone(), &zip)];
    manager.save_pending_manifest().unwrap();
    manager.save_updater_apply_targets(&[Component::Locales.manifest_key(), ext.manifest_key()]).unwrap();
    manager.status.components.clear();
    let installed: std::collections::HashMap<String, String> = [(Component::Locales.manifest_key(), "0.2.0".to_string())].into_iter().collect();
    std::fs::write(&manager.installed_manifest_file, serde_json::to_string(&installed).unwrap()).unwrap();

    let (applied, failed) = manager.apply_staged(|_| {}).await.unwrap();

    assert_eq!(applied, vec![ext.manifest_key()]);
    assert!(failed.is_empty(), "{:?}", failed);
    let _ = std::fs::remove_dir_all(manager.resolve_ext_dir(&name));
}

// 기록 없이 놓인 모듈의 같은 버전 재설치(공식 표시 복구)는 되살린 목록에서도 남긴다. 더 새 판이 깔렸으면 뺀다
#[test]
fn a_restored_same_version_reinstall_without_a_record_is_kept() {
    let tmp = tempfile::tempdir().unwrap();
    let file = tmp.path().join("x.zip");
    std::fs::write(&file, b"zip").unwrap();
    let same = Component::Module(format!("unrecorded-{}", std::process::id()));
    let newer = Component::Module(format!("newer-{}", std::process::id()));
    let mut manager = UpdateManager::new(UpdateConfig::default(), tmp.path().join("modules").to_str().unwrap());
    manager.staging_dir = tmp.path().join("staging");
    manager.installed_manifest_file = tmp.path().join("installed-manifest.json");
    manager.status.components = vec![staged(same.clone(), &file), staged(newer.clone(), &file)];
    manager.save_pending_manifest().unwrap();
    let installed: std::collections::HashMap<String, String> =
        [(same.manifest_key(), "0.2.0".to_string()), (newer.manifest_key(), "0.3.0".to_string())].into_iter().collect();
    std::fs::write(&manager.installed_manifest_file, serde_json::to_string(&installed).unwrap()).unwrap();

    assert_eq!(manager.load_pending_manifest().unwrap(), 1);
    assert_eq!(manager.status.components[0].component, same);
}

// 맥 앱 묶음 안의 GUI 업데이트: 새 묶음 정보·아이콘을 Contents 자리로 옮기고, 무결성 기록은 옮긴 자리를 적는다
#[tokio::test]
async fn a_gui_update_inside_an_app_bundle_moves_the_bundle_files_into_place() {
    use std::io::Write;
    use crate::app_bundle::{ASSETS_FILE, BUNDLE_NAME, ICON_FILE, PLIST_FILE};
    let tmp = tempfile::tempdir().unwrap();
    let mut manager = UpdateManager::new(UpdateConfig::default(), tmp.path().join("modules").to_str().unwrap());
    let contents = tmp.path().join(BUNDLE_NAME).join("Contents");
    manager.install_root = contents.join("MacOS");
    std::fs::create_dir_all(&manager.install_root).unwrap();
    std::fs::create_dir_all(contents.join("Resources")).unwrap();
    std::fs::write(manager.install_root.join(crate::GUI_EXE_NAMES[0]), b"old gui").unwrap();
    std::fs::write(contents.join(PLIST_FILE), b"old plist").unwrap();
    std::fs::write(contents.join("Resources").join(ICON_FILE), b"old icns").unwrap();
    let zip_path = tmp.path().join("gui.zip");
    {
        let mut zw = zip::ZipWriter::new(std::fs::File::create(&zip_path).unwrap());
        let opts = zip::write::SimpleFileOptions::default();
        for (name, data) in [(crate::GUI_EXE_NAMES[0], &b"new gui"[..]), (PLIST_FILE, b"new plist"), (ICON_FILE, b"new icns"), (ASSETS_FILE, b"new car")] {
            zw.start_file(name, opts).unwrap();
            zw.write_all(data).unwrap();
        }
        zw.finish().unwrap();
    }

    let files = manager.apply_gui_update(&zip_path.to_string_lossy()).await.unwrap();

    assert_eq!(std::fs::read(contents.join(PLIST_FILE)).unwrap(), b"new plist");
    assert_eq!(std::fs::read(contents.join("Resources").join(ICON_FILE)).unwrap(), b"new icns");
    assert_eq!(std::fs::read(contents.join("Resources").join(ASSETS_FILE)).unwrap(), b"new car");
    assert!([PLIST_FILE, ICON_FILE, ASSETS_FILE].iter().all(|f| !manager.install_root.join(f).exists()));
    assert!(files.iter().all(|f| f.exists()), "recorded files are where they ended up: {files:?}");
    assert!(files.contains(&contents.join(PLIST_FILE)));
}

/// 설치기가 업데이트를 적용하게 된 뒤(0.1.12) 설치 폴더에 남은 옛 업데이터와 그 기록을 걷는다.
/// 설치기가 없으면 옛 업데이터가 유일한 적용 프로그램이라 남긴다
#[test]
fn the_retired_updater_is_removed_once_the_installer_is_there() {
    let tmp = tempfile::tempdir().unwrap();
    let root = tmp.path().join("install");
    std::fs::create_dir_all(&root).unwrap();
    let legacy = root.join(crate::LEGACY_UPDATER_EXE);
    std::fs::write(&legacy, b"old updater").unwrap();
    let versions = tmp.path().join("installed-manifest.json");
    std::fs::write(&versions, r#"{"saba-core":"0.1.11","updater":"0.1.11"}"#).unwrap();
    let records = tmp.path().join("installed-files.json");
    std::fs::write(&records, r#"{"updater":{"/x/saba-chan-updater":"ab"},"cli":{}}"#).unwrap();

    crate::retire_legacy_updater_at(&root, &versions, &records);
    assert!(legacy.exists(), "without the installer the old updater is the only one");

    std::fs::write(root.join(crate::INSTALLER_EXE), b"installer").unwrap();
    crate::retire_legacy_updater_at(&root, &versions, &records);
    assert!(legacy.exists(), "an installer not recorded as able to apply may be the old one");
    std::fs::write(&versions, r#"{"saba-core":"0.1.11","updater":"0.1.11","installer":"0.1.12-beta.1"}"#).unwrap();
    crate::retire_legacy_updater_at(&root, &versions, &records);
    assert!(legacy.exists(), "the old installer copy stayed (its replacement failed)");

    std::fs::write(&versions, format!(r#"{{"saba-core":"0.1.11","updater":"0.1.11","installer":"{}"}}"#, crate::INSTALLER_APPLIES_SINCE)).unwrap();
    crate::retire_legacy_updater_at(&root, &versions, &records);
    assert!(!legacy.exists());
    let left: serde_json::Value = serde_json::from_str(&std::fs::read_to_string(&versions).unwrap()).unwrap();
    assert_eq!(left, serde_json::json!({"saba-core": "0.1.11", "installer": crate::INSTALLER_APPLIES_SINCE}));
    let left: serde_json::Value = serde_json::from_str(&std::fs::read_to_string(&records).unwrap()).unwrap();
    assert!(left.get("updater").is_none() && left.get("cli").is_some(), "{left}");

    // 파일이 이미 없어진 설치도 남은 기록을 걷는다
    std::fs::write(&versions, format!(r#"{{"updater":"0.1.11","installer":"{}"}}"#, crate::INSTALLER_APPLIES_SINCE)).unwrap();
    crate::retire_legacy_updater_at(&root, &versions, &records);
    let left: serde_json::Value = serde_json::from_str(&std::fs::read_to_string(&versions).unwrap()).unwrap();
    assert_eq!(left, serde_json::json!({"installer": crate::INSTALLER_APPLIES_SINCE}));
}

// 업데이터를 띄우기 전에 데몬이 무엇이 바뀌는지 본다: 남긴 적용 대상 목록을 지우지 않고 읽고, 목록이 없으면 받아 둔 것 전부
#[test]
fn the_updater_targets_are_read_without_consuming_them() {
    let tmp = tempfile::tempdir().unwrap();
    let mut manager = UpdateManager::new(UpdateConfig::default(), tmp.path().join("modules").to_str().unwrap());
    manager.staging_dir = tmp.path().join("staging");
    let zip = tmp.path().join("core.zip");
    std::fs::write(&zip, b"").unwrap();
    manager.status.components = vec![staged(Component::CoreDaemon, &zip), staged(Component::Gui, &zip)];

    assert_eq!(manager.updater_targets(), vec!["saba-core".to_string(), "gui".to_string()]);
    manager.save_updater_apply_targets(&["saba-core".to_string()]).unwrap();
    assert_eq!(manager.updater_targets(), vec!["saba-core".to_string()]);
    assert_eq!(manager.load_updater_apply_targets().unwrap(), vec!["saba-core".to_string()], "the updater still finds the list");
    manager.save_updater_apply_targets(&["gui".to_string()]).unwrap();
    manager.status.components = vec![staged(Component::CoreDaemon, &zip)];
    assert_eq!(manager.updater_targets(), vec!["saba-core".to_string()], "a stale list does not name what is not downloaded");
    manager.status.components.clear();
    assert!(manager.updater_targets().is_empty());
}
