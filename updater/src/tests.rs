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
        UpdateManager::component_apply_priority(&module),
        UpdateManager::component_apply_priority(&extension),
        "모듈과 익스텐션은 동일 우선순위"
    );
    assert_eq!(
        UpdateManager::component_apply_priority(&module),
        UpdateManager::component_apply_priority(&locales),
        "모듈과 Locales는 동일 우선순위"
    );

    // Updater는 최우선
    assert!(
        UpdateManager::component_apply_priority(&updater) <
        UpdateManager::component_apply_priority(&module),
        "Updater가 모듈보다 먼저 적용"
    );

    // 모듈은 새 코어를 요구할 수 있다(`requires`): CoreDaemon이 모듈/익스텐션보다 먼저
    assert!(
        UpdateManager::component_apply_priority(&core_daemon) <
        UpdateManager::component_apply_priority(&module),
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

/// 설치기는 받은 zip을 적용 전에 확인한다. 확인은 풀어 쓰지 않고 읽기만 해서, 깨진 내용(CRC)과 밖으로 나가는 경로를 잡는다
#[test]
fn verify_zip_catches_a_damaged_or_unsafe_archive_without_writing() {
    use std::io::Write;
    let tmp = tempfile::tempdir().unwrap();
    let stored = zip::write::SimpleFileOptions::default().compression_method(zip::CompressionMethod::Stored);
    let good = tmp.path().join("good.zip");
    {
        let mut zw = zip::ZipWriter::new(std::fs::File::create(&good).unwrap());
        zw.start_file("dir/inner.txt", stored).unwrap();
        zw.write_all(b"the-content-of-the-file").unwrap();
        zw.finish().unwrap();
    }
    crate::verify_zip(&good).unwrap();

    let mut bytes = std::fs::read(&good).unwrap();
    let at = bytes.windows(11).position(|w| w == b"the-content").unwrap();
    bytes[at] ^= 0xff;
    let damaged = tmp.path().join("damaged.zip");
    std::fs::write(&damaged, bytes).unwrap();
    assert!(crate::verify_zip(&damaged).is_err(), "a changed byte must fail the CRC check");

    let evil = tmp.path().join("evil.zip");
    {
        let mut zw = zip::ZipWriter::new(std::fs::File::create(&evil).unwrap());
        zw.start_file("../escaped.txt", stored).unwrap();
        zw.write_all(b"bad").unwrap();
        zw.finish().unwrap();
    }
    assert!(crate::verify_zip(&evil).is_err());

    let mut left: Vec<_> = std::fs::read_dir(tmp.path()).unwrap().flatten().map(|e| e.file_name().to_string_lossy().into_owned()).collect();
    left.sort();
    assert_eq!(left, ["damaged.zip", "evil.zip", "good.zip"]);
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

    assert!(crate::replace_dir(&target, &bad, &manager.staging_dir, "game_backup").is_err());
    assert_eq!(std::fs::read_to_string(target.join("module.toml")).unwrap(), "old");
}

/// 모듈 폴더 교체: 내용은 새 판으로 바뀌고, 점으로 시작하는 항목과 `__pycache__`는 남고, 백업·임시 폴더는 남지 않는다
#[test]
fn replace_dir_swaps_the_content_and_keeps_dot_entries() {
    let tmp = tempfile::tempdir().unwrap();
    let staging = tmp.path().join("staging");
    let target = tmp.path().join("modules").join("game");
    std::fs::create_dir_all(target.join(".git")).unwrap();
    std::fs::create_dir_all(target.join("__pycache__")).unwrap();
    std::fs::create_dir_all(target.join("assets")).unwrap();
    std::fs::write(target.join("module.toml"), "old").unwrap();
    std::fs::write(target.join("assets/old.bin"), "old").unwrap();
    std::fs::write(target.join(".git/HEAD"), "ref").unwrap();
    std::fs::write(target.join("__pycache__/lifecycle.pyc"), "pyc").unwrap();
    let staged = new_module_zip(&staging);

    let files = crate::replace_dir(&target, &staged, &staging, "game_backup").unwrap();

    assert_eq!(files, [target.join("module.toml")]);
    assert_eq!(names_in(&target), [".git", "__pycache__", "module.toml"]);
    assert_eq!(std::fs::read_to_string(target.join("module.toml")).unwrap(), "new");
    assert_eq!(std::fs::read_to_string(target.join(".git/HEAD")).unwrap(), "ref");
    assert_eq!(std::fs::read_to_string(target.join("__pycache__/lifecycle.pyc")).unwrap(), "pyc");
    assert!(names_in(&staging).is_empty(), "nothing is left in staging: {:?}", names_in(&staging));
}

/// 맞바꾸는 두 이름 바꾸기 사이에 꺼졌으면(제자리가 비고 백업만 있다) 다음 교체가 원래 내용을 되살린 뒤 바꾼다
#[test]
fn replace_dir_recovers_a_folder_that_was_moved_aside_and_never_replaced() {
    let tmp = tempfile::tempdir().unwrap();
    let staging = tmp.path().join("staging");
    let target = tmp.path().join("modules").join("game");
    std::fs::create_dir_all(target.parent().unwrap()).unwrap();
    let backup = staging.join("game_backup");
    std::fs::create_dir_all(backup.join(".git")).unwrap();
    std::fs::write(backup.join("module.toml"), "old").unwrap();
    std::fs::write(backup.join(".git/HEAD"), "ref").unwrap();
    let staged = new_module_zip(&staging);

    crate::replace_dir(&target, &staged, &staging, "game_backup").unwrap();

    assert_eq!(std::fs::read_to_string(target.join("module.toml")).unwrap(), "new");
    assert_eq!(std::fs::read_to_string(target.join(".git/HEAD")).unwrap(), "ref");
    assert!(names_in(&staging).is_empty());
}

/// 모듈 폴더가 다른 곳을 가리키는 링크면(개발 폴더를 걸어 둠) 링크를 지우거나 옮기지 않고 그 안의 내용을 바꾼다
#[cfg(unix)]
#[test]
fn replace_dir_writes_into_a_linked_folder_and_keeps_the_link() {
    let tmp = tempfile::tempdir().unwrap();
    let staging = tmp.path().join("staging");
    let real = tmp.path().join("dev").join("game");
    std::fs::create_dir_all(&real).unwrap();
    std::fs::write(real.join("module.toml"), "old").unwrap();
    let target = tmp.path().join("modules").join("game");
    std::fs::create_dir_all(target.parent().unwrap()).unwrap();
    std::os::unix::fs::symlink(&real, &target).unwrap();
    let staged = new_module_zip(&staging);

    crate::replace_dir(&target, &staged, &staging, "game_backup").unwrap();

    assert!(target.is_symlink());
    assert_eq!(std::fs::read_to_string(real.join("module.toml")).unwrap(), "new");
}

/// 묻지 않는 교체에 쓸 새 판 zip (`module.toml` 하나)
fn new_module_zip(dir: &std::path::Path) -> std::path::PathBuf {
    use std::io::Write;
    std::fs::create_dir_all(dir).unwrap();
    let path = dir.join("module-game.zip");
    let mut zip = zip::ZipWriter::new(std::fs::File::create(&path).unwrap());
    zip.start_file("module.toml", zip::write::SimpleFileOptions::default()).unwrap();
    zip.write_all(b"new").unwrap();
    zip.finish().unwrap();
    path
}

fn old_module(target: &std::path::Path) {
    std::fs::create_dir_all(target.join(".git")).unwrap();
    std::fs::write(target.join("module.toml"), "old").unwrap();
    std::fs::write(target.join(".git/HEAD"), "ref").unwrap();
}

fn names_in(dir: &std::path::Path) -> Vec<String> {
    let mut names: Vec<String> = std::fs::read_dir(dir).unwrap().map(|e| e.unwrap().file_name().to_string_lossy().to_string()).collect();
    names.sort();
    names
}

// 묻지 않는 교체(Codex #926 재리뷰): 원래 폴더는 옆으로 옮긴 뒤 그 순간의 지문으로 검사하고, 바꿔 넣은 뒤에도 바로 지우지 않는다.
// 옮긴 폴더는 다음 교체 때 지문이 그대로일 때만 지운다
#[test]
fn an_untouched_swap_puts_the_new_version_in_and_keeps_the_old_folder_until_the_next_swap() {
    let tmp = tempfile::tempdir().unwrap();
    let modules = tmp.path().join("modules");
    let target = modules.join("game");
    old_module(&target);
    let staged = new_module_zip(&tmp.path().join("staging"));
    let mut checked = None;
    let files = crate::swap_dir_untouched(&target, &staged, |fp| {
        checked = Some(fp.keys().cloned().collect::<Vec<_>>());
        assert!(!target.exists(), "the original is moved away before it is checked");
        true
    })
    .unwrap();
    assert_eq!(checked, Some(vec!["module.toml".to_string()]), "dot entries are not part of the check");
    assert_eq!(std::fs::read_to_string(target.join("module.toml")).unwrap(), "new");
    assert_eq!(std::fs::read_to_string(target.join(".git/HEAD")).unwrap(), "ref", "dot entries come along");
    assert_eq!(files, vec![target.join("module.toml")]);
    assert_eq!(names_in(&modules), [".game.saba-old", "game"], "the old folder stays until the next swap");

    // 다음 교체: 옮긴 폴더가 그대로면 그때 지운다
    let staged = new_module_zip(&tmp.path().join("staging"));
    crate::swap_dir_untouched(&target, &staged, |_| true).unwrap();
    assert_eq!(names_in(&modules), [".game.saba-old", "game"]);
    assert_eq!(std::fs::read_to_string(modules.join(".game.saba-old/module.toml")).unwrap(), "new", "only the last old folder is kept");
}

#[test]
fn an_untouched_swap_puts_a_changed_folder_back_as_it_was() {
    let tmp = tempfile::tempdir().unwrap();
    let modules = tmp.path().join("modules");
    let target = modules.join("game");
    old_module(&target);
    std::fs::write(target.join("server.properties"), "mine").unwrap();
    let staged = new_module_zip(&tmp.path().join("staging"));
    assert!(crate::swap_dir_untouched(&target, &staged, |_| false).is_err());
    assert_eq!(std::fs::read_to_string(target.join("module.toml")).unwrap(), "old");
    assert_eq!(std::fs::read_to_string(target.join("server.properties")).unwrap(), "mine");
    assert_eq!(names_in(&target), [".git", "module.toml", "server.properties"], "no bookkeeping file is left in the folder");
    assert_eq!(names_in(&modules), ["game"]);
}

/// 원래 폴더를 옮긴 뒤 같은 경로에 사용자가 파일을 만들면 아무것도 지우지 않고 멈춘다: 그 파일도, 옮긴 원래 폴더도 남는다
#[test]
fn a_file_written_after_the_move_is_never_deleted() {
    let tmp = tempfile::tempdir().unwrap();
    let target = tmp.path().join("modules").join("game");
    old_module(&target);
    let staged = new_module_zip(&tmp.path().join("staging"));
    let err = crate::swap_dir_untouched(&target, &staged, |_| {
        std::fs::create_dir_all(&target).unwrap();
        std::fs::write(target.join("late.cfg"), "late").unwrap();
        true
    })
    .unwrap_err();
    assert_eq!(std::fs::read_to_string(target.join("late.cfg")).unwrap(), "late");
    let aside = tmp.path().join("modules").join(".game.saba-old");
    assert_eq!(std::fs::read_to_string(aside.join("module.toml")).unwrap(), "old", "the original is kept");
    assert!(err.to_string().contains(".game.saba-old"), "the error names where the original is: {err}");
}

/// 교체 전에 열어 둔 파일에 교체 뒤 저장해도(유닉스에서는 옮긴 폴더의 파일에 써진다) 지우지 않는다:
/// 다음 교체가 옮긴 폴더의 지문이 달라진 것을 보고 남긴다 (Codex #926 5차 재리뷰)
#[cfg(unix)]
#[test]
fn a_save_through_a_handle_opened_before_the_swap_is_kept() {
    use std::io::Write;
    let tmp = tempfile::tempdir().unwrap();
    let modules = tmp.path().join("modules");
    let target = modules.join("game");
    old_module(&target);
    let mut editor = std::fs::OpenOptions::new().write(true).open(target.join("module.toml")).unwrap();
    let staged = new_module_zip(&tmp.path().join("staging"));
    crate::swap_dir_untouched(&target, &staged, |_| true).unwrap();
    editor.write_all(b"edited").unwrap();
    drop(editor);

    let staged = new_module_zip(&tmp.path().join("staging"));
    let err = crate::swap_dir_untouched(&target, &staged, |_| true).unwrap_err();
    assert!(err.to_string().contains(".game.saba-old"), "{err}");
    assert_eq!(std::fs::read_to_string(modules.join(".game.saba-old/module.toml")).unwrap(), "edited");
    assert_eq!(std::fs::read_to_string(target.join("module.toml")).unwrap(), "new", "the new version stays in place");
}

/// 옮긴 뒤 꺼졌으면(원래 경로가 비었다) 다음 교체가 먼저 원래 폴더를 돌려놓는다
#[test]
fn an_interrupted_swap_is_put_back_first() {
    let tmp = tempfile::tempdir().unwrap();
    let modules = tmp.path().join("modules");
    let aside = modules.join(".game.saba-old");
    old_module(&aside);
    std::fs::create_dir_all(modules.join(".game.saba-new")).unwrap();
    std::fs::write(modules.join(".game.saba-new/half"), "x").unwrap();
    let target = modules.join("game");
    let staged = new_module_zip(&tmp.path().join("staging"));
    assert!(crate::swap_dir_untouched(&target, &staged, |_| false).is_err());
    assert_eq!(std::fs::read_to_string(target.join("module.toml")).unwrap(), "old");
    assert_eq!(names_in(&target), [".git", "module.toml"]);
    assert_eq!(names_in(&modules), ["game"]);
}

// 새 판을 넣지 못하고 되돌리지도 못했으면 옮긴 폴더는 아직 끝나지 않은 교체다: 지문(끝난 표시)을 남기지 않아 기동이 되돌린다
#[test]
fn a_swap_that_could_not_put_anything_back_is_not_marked_finished() {
    let tmp = tempfile::tempdir().unwrap();
    let modules = tmp.path().join("modules");
    let target = modules.join("game");
    old_module(&target);
    let staged = new_module_zip(&tmp.path().join("staging"));
    let blocker = target.clone();
    // 검사하는 사이 제자리에 비지 않은 폴더가 생겨 새 판도 원래 폴더도 들어가지 못한다 (파일이면 Windows rename이 덮어쓴다)
    let result = crate::swap_dir_untouched(&target, &staged, |_| {
        std::fs::create_dir_all(&blocker).unwrap();
        std::fs::write(blocker.join("user.txt"), "x").unwrap();
        true
    });
    assert!(result.is_err());
    let aside = modules.join(".game.saba-old");
    assert_eq!(names_in(&aside), [".git", "module.toml"], "no finished mark on a swap that did not finish");
}

// 새 판을 넣은 직후 꺼져 점 항목을 옮기지 못했으면 다음 교체가 먼저 옮겨 준다
#[test]
fn dot_entries_left_by_a_cut_off_swap_are_carried_first() {
    let tmp = tempfile::tempdir().unwrap();
    let modules = tmp.path().join("modules");
    let aside = modules.join(".game.saba-old");
    std::fs::create_dir_all(&aside).unwrap();
    std::fs::write(aside.join("module.toml"), "old").unwrap();
    let fingerprint = crate::integrity::dir_fingerprint(&aside).unwrap();
    std::fs::write(aside.join(crate::SWAP_FINGERPRINT), serde_json::to_string(&fingerprint).unwrap()).unwrap();
    std::fs::write(aside.join(".env"), "secret").unwrap();
    let target = modules.join("game");
    std::fs::create_dir_all(&target).unwrap();
    std::fs::write(target.join("module.toml"), "mid").unwrap();
    let staged = new_module_zip(&tmp.path().join("staging"));
    crate::swap_dir_untouched(&target, &staged, |_| true).unwrap();
    assert_eq!(std::fs::read_to_string(target.join(".env")).unwrap(), "secret");
    assert_eq!(std::fs::read_to_string(target.join("module.toml")).unwrap(), "new");
}

// 새 판을 넣은 직후, 지문을 적기 전에 꺼졌다: 다음 교체가 점 항목을 옮기고 옛 폴더를 정리한다
#[test]
fn a_swap_cut_off_before_its_mark_is_finished_by_the_next_one() {
    let tmp = tempfile::tempdir().unwrap();
    let modules = tmp.path().join("modules");
    let aside = modules.join(".game.saba-old");
    std::fs::create_dir_all(&aside).unwrap();
    std::fs::write(aside.join("module.toml"), "old").unwrap();
    std::fs::write(aside.join(".env"), "secret").unwrap();
    let target = modules.join("game");
    std::fs::create_dir_all(&target).unwrap();
    std::fs::write(target.join("module.toml"), "mid").unwrap();
    let staged = new_module_zip(&tmp.path().join("staging"));
    crate::swap_dir_untouched(&target, &staged, |_| true).unwrap();
    assert_eq!(std::fs::read_to_string(target.join(".env")).unwrap(), "secret");
    assert_eq!(std::fs::read_to_string(target.join("module.toml")).unwrap(), "new");
}

// 지난 교체를 마친 뒤 사용자가 지운 모듈은 되살리지도, 새 판으로 깔지도 않는다
#[test]
fn a_folder_removed_after_a_finished_swap_is_not_brought_back() {
    let tmp = tempfile::tempdir().unwrap();
    let modules = tmp.path().join("modules");
    let aside = modules.join(".game.saba-old");
    std::fs::create_dir_all(&aside).unwrap();
    std::fs::write(aside.join("module.toml"), "old").unwrap();
    let fingerprint = crate::integrity::dir_fingerprint(&aside).unwrap();
    std::fs::write(aside.join(crate::SWAP_FINGERPRINT), serde_json::to_string(&fingerprint).unwrap()).unwrap();
    let target = modules.join("game");
    let staged = new_module_zip(&tmp.path().join("staging"));
    assert!(crate::swap_dir_untouched(&target, &staged, |_| true).is_err());
    assert!(names_in(&modules).is_empty(), "nothing comes back: {:?}", names_in(&modules));
}

// 교체 도중 꺼졌으면 남은 백업이 원래 내용이다: 다음 시도가 반쯤 바뀐 폴더로 백업을 덮지 않는다
#[test]
fn replace_dir_keeps_the_backup_left_by_an_interrupted_replace() {
    let tmp = tempfile::tempdir().unwrap();
    let mut manager = UpdateManager::new(UpdateConfig::default(), tmp.path().join("modules").to_str().unwrap());
    manager.staging_dir = tmp.path().join("staging");
    std::fs::create_dir_all(&manager.staging_dir).unwrap();
    let target = tmp.path().join("modules").join("game");
    std::fs::create_dir_all(&target).unwrap();
    // 지난 시도: 백업은 끝냈고, 원래 파일을 지운 뒤 새 파일 하나만 풀고 꺼졌다
    let backup = manager.staging_dir.join("game_backup");
    std::fs::create_dir_all(&backup).unwrap();
    std::fs::write(backup.join("module.toml"), "old").unwrap();
    std::fs::write(target.join("half.py"), "new").unwrap();
    let bad = manager.staging_dir.join("module-game.zip");
    std::fs::write(&bad, b"not a zip").unwrap();

    assert!(crate::replace_dir(&target, &bad, &manager.staging_dir, "game_backup").is_err());
    assert_eq!(std::fs::read_to_string(target.join("module.toml")).unwrap(), "old");
    assert!(!target.join("half.py").exists());
}

// 실패한 교체를 다 되돌렸으면 백업을 지운다: 그 사이 다른 판이 깔려도 다음 교체가 옛 백업으로 되돌리지 않는다
#[test]
fn replace_dir_drops_the_backup_after_a_clean_rollback() {
    let tmp = tempfile::tempdir().unwrap();
    let mut manager = UpdateManager::new(UpdateConfig::default(), tmp.path().join("modules").to_str().unwrap());
    manager.staging_dir = tmp.path().join("staging");
    std::fs::create_dir_all(&manager.staging_dir).unwrap();
    let target = tmp.path().join("modules").join("game");
    std::fs::create_dir_all(&target).unwrap();
    std::fs::write(target.join("module.toml"), "v1").unwrap();
    let bad = manager.staging_dir.join("module-game.zip");
    std::fs::write(&bad, b"not a zip").unwrap();
    assert!(crate::replace_dir(&target, &bad, &manager.staging_dir, "game_backup").is_err());
    assert!(!manager.staging_dir.join("game_backup").exists());

    // 설치기가 v2를 깔았다. 다음 교체가 또 실패해도 v2로 남아야 한다
    std::fs::write(target.join("module.toml"), "v2").unwrap();
    std::fs::write(&bad, b"not a zip").unwrap();
    assert!(crate::replace_dir(&target, &bad, &manager.staging_dir, "game_backup").is_err());
    assert_eq!(std::fs::read_to_string(target.join("module.toml")).unwrap(), "v2");
}

// 백업을 만들다 꺼졌으면 그 반쪽 백업은 쓰지 않는다 (폴더는 아직 그대로다)
#[test]
fn replace_dir_ignores_a_backup_that_was_cut_off_while_copying() {
    let tmp = tempfile::tempdir().unwrap();
    let mut manager = UpdateManager::new(UpdateConfig::default(), tmp.path().join("modules").to_str().unwrap());
    manager.staging_dir = tmp.path().join("staging");
    std::fs::create_dir_all(&manager.staging_dir).unwrap();
    let target = tmp.path().join("modules").join("game");
    std::fs::create_dir_all(&target).unwrap();
    std::fs::write(target.join("module.toml"), "old").unwrap();
    std::fs::write(target.join("lifecycle.py"), "old").unwrap();
    let partial = manager.staging_dir.join("game_backup.partial");
    std::fs::create_dir_all(&partial).unwrap();
    std::fs::write(partial.join("module.toml"), "old").unwrap();
    let bad = manager.staging_dir.join("module-game.zip");
    std::fs::write(&bad, b"not a zip").unwrap();

    assert!(crate::replace_dir(&target, &bad, &manager.staging_dir, "game_backup").is_err());
    assert_eq!(std::fs::read_to_string(target.join("lifecycle.py")).unwrap(), "old");
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
        repair: false,
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
        repair: false,
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
        repair: false,
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

/// 설치 폴더에 GUI가 없으면 GUI 업데이트는 그렇다고 알리며 끝난다. 작업 폴더 옆의 `saba-chan-gui`(소스 트리)를 GUI로 보고 풀지 않는다
#[tokio::test]
async fn a_gui_update_without_a_gui_in_the_install_folder_fails_and_writes_nowhere_else() {
    use std::io::Write;
    let tmp = tempfile::tempdir().unwrap();
    let mut manager = UpdateManager::new(UpdateConfig::default(), tmp.path().join("modules").to_str().unwrap());
    manager.install_root = tmp.path().join("install");
    std::fs::create_dir_all(&manager.install_root).unwrap();
    let zip_path = tmp.path().join("gui.zip");
    {
        let mut zw = zip::ZipWriter::new(std::fs::File::create(&zip_path).unwrap());
        zw.start_file(crate::GUI_EXE_NAMES[0], zip::write::SimpleFileOptions::default()).unwrap();
        zw.write_all(b"new gui").unwrap();
        zw.finish().unwrap();
    }

    let err = manager.apply_gui_update(&zip_path.to_string_lossy()).await.unwrap_err().to_string();

    assert!(err.contains(&manager.install_root.display().to_string()), "{err}");
    assert!(!manager.is_component_installed(&Component::Gui));
    assert!(zip_path.exists(), "the staged file is kept");
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
        repair: false,
    }
}

// L3(a)·U-9: 여러 컴포넌트를 적용하다 실패해도 먼저 적용한 것은 버전을 기록하고 적용한 것으로 돌려준다
// (그대로 두면 다시 받으라고 뜬다. 업데이터는 완료 마커에 싣고 pending을 지운다)
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

    let keys = manager.pending_keys_in_apply_order();
    let (applied, failed) = manager.apply_target_keys(&keys, |_, _, _| {}).await;

    assert_eq!(applied, vec![ext.manifest_key()]);
    assert!(!failed.is_empty());
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

    assert!(crate::replace_dir(&target, &bad, &manager.staging_dir, "fresh_backup").is_err());
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

// Unix는 확장자 없는 바이너리를 `<이름>.old`로 비켜 둔다: 끊긴 적용 뒤 되돌리고, 새 판이 있으면 정리 때 지운다
#[cfg(unix)]
#[test]
fn a_unix_binary_moved_aside_by_an_interrupted_apply_is_put_back() {
    let tmp = tempfile::tempdir().unwrap();
    let mut manager = UpdateManager::new(UpdateConfig::default(), tmp.path().join("modules").to_str().unwrap());
    manager.install_root = tmp.path().to_path_buf();
    std::fs::write(tmp.path().join("saba-core.old"), b"previous").unwrap();
    std::fs::write(tmp.path().join("saba-chan-cli"), b"new").unwrap();
    std::fs::write(tmp.path().join("saba-chan-cli.old"), b"old cli").unwrap();
    std::fs::write(tmp.path().join("notes.old"), b"not ours").unwrap();

    manager.settle_interrupted_updates();

    assert_eq!(std::fs::read(tmp.path().join("saba-core")).unwrap(), b"previous");
    assert_eq!(std::fs::read(tmp.path().join("saba-chan-cli")).unwrap(), b"new", "a binary that is there stays");
    assert!(!tmp.path().join("notes").exists());

    manager.cleanup_old_files();

    assert!(!tmp.path().join("saba-chan-cli.old").exists());
    assert!(tmp.path().join("notes.old").exists());
}

// saba-core zip에 같이 든 saba-runner도 Unix에서 코어와 함께 되돌릴 수 있게 비켜 둔다 (Windows는 `saba-runner.exe.old`)
#[cfg(unix)]
#[tokio::test]
async fn a_unix_core_apply_keeps_the_previous_runner_for_rollback() {
    use std::io::Write;
    let tmp = tempfile::tempdir().unwrap();
    let mut manager = UpdateManager::new(UpdateConfig::default(), tmp.path().join("modules").to_str().unwrap());
    manager.install_root = tmp.path().join("install");
    std::fs::create_dir_all(&manager.install_root).unwrap();
    for exe in ["saba-core", "saba-runner"] {
        std::fs::write(manager.install_root.join(exe), b"previous").unwrap();
    }
    let zip_path = tmp.path().join("saba-core-linux-x64.zip");
    {
        let mut zw = zip::ZipWriter::new(std::fs::File::create(&zip_path).unwrap());
        for exe in ["saba-core", "saba-runner"] {
            zw.start_file(exe, zip::write::SimpleFileOptions::default().unix_permissions(0o755)).unwrap();
            zw.write_all(b"new").unwrap();
        }
        zw.finish().unwrap();
    }
    let before = manager.exe_backups();

    manager.apply_binary_update("saba-core", &zip_path.to_string_lossy()).await.unwrap();
    assert_eq!(std::fs::read(manager.install_root.join("saba-runner")).unwrap(), b"new");

    manager.roll_back_exes_since(&before).unwrap();

    for exe in ["saba-core", "saba-runner"] {
        assert_eq!(std::fs::read(manager.install_root.join(exe)).unwrap(), b"previous", "{exe}");
    }
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
    let _ = crate::Replacing::default().extract(&archive, &target);
    assert!(!outside.join("new.txt").exists());
    assert!(!target.join("sub").is_symlink());

    std::fs::remove_dir_all(target.join("sub")).unwrap();
    std::os::unix::fs::symlink(&outside, target.join("sub")).unwrap();
    assert!(crate::Replacing::default().extract(&archive, &target).is_err());
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
        install_dir: None, requires: None, meta: None, reads_runner_formats: None,
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

// 같은 묶음에서 먼저 적용한 것은 뒤의 requires를 채운다
#[tokio::test]
async fn an_unlisted_apply_counts_what_it_applied_earlier() {
    let tmp = tempfile::tempdir().unwrap();
    let mut manager = UpdateManager::new(UpdateConfig::default(), tmp.path().join("modules").to_str().unwrap());
    manager.install_root = tmp.path().join("install");
    manager.installed_manifest_file = tmp.path().join("installed-manifest.json");
    std::fs::create_dir_all(&manager.install_root).unwrap();
    let (first, second) = (format!("first-{}", std::process::id()), format!("second-{}", std::process::id()));
    let (a, b) = (Component::Extension(first.clone()), Component::Extension(second.clone()));
    let zip_dir = |name: &str| {
        let dir = tmp.path().join(name);
        std::fs::create_dir_all(&dir).unwrap();
        dir
    };
    // 먼저 적용한 익스텐션은 디스크의 manifest 버전으로 센다
    let a_zip = zip_dir("a").join("ext.zip");
    {
        use std::io::Write;
        let mut zw = zip::ZipWriter::new(std::fs::File::create(&a_zip).unwrap());
        zw.start_file("manifest.json", zip::write::SimpleFileOptions::default()).unwrap();
        zw.write_all(format!(r#"{{"id":"{first}","version":"9.9.9"}}"#).as_bytes()).unwrap();
        zw.finish().unwrap();
    }
    manager.status.components = vec![
        crate::ComponentVersion {
            requires: Some([(a.manifest_key(), ">=9.9.9".to_string())].into_iter().collect()),
            ..staged(b.clone(), &extension_zip(&zip_dir("b")))
        },
        crate::ComponentVersion { latest_version: Some("9.9.9".into()), ..staged(a.clone(), &a_zip) },
    ];

    let keys = manager.pending_keys_in_apply_order();
    let (applied, failed) = manager.apply_target_keys(&keys, |_, _, _| {}).await;

    assert!(failed.is_empty(), "{:?}", failed);
    assert_eq!(applied, vec![a.manifest_key(), b.manifest_key()]);
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

/// `apply_staged`는 완료 마커를 한 자리(데이터 폴더)에 남긴다: 마커를 남기는 시험끼리 차례로
static MARKER: tokio::sync::Mutex<()> = tokio::sync::Mutex::const_new(());

// 업데이터 `--apply` 한 번(창·창 없음 공통): 받아 둔 것을 적용하고 목록을 지운다
#[tokio::test]
async fn apply_staged_applies_the_pending_list_and_clears_it() {
    let _marker = MARKER.lock().await;
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
    let _marker = MARKER.lock().await;
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

// 복구(같은 판 다시 받기)는 업데이터가 적용하고, 결과는 업데이트로 알리지 않는다: 앱은 알림을 띄우지 않고 `update last`가 본다
#[tokio::test]
async fn the_updater_applies_a_repair_and_does_not_announce_it_as_an_update() {
    let _marker = MARKER.lock().await;
    let tmp = tempfile::tempdir().unwrap();
    let mut manager = UpdateManager::new(UpdateConfig::default(), tmp.path().join("modules").to_str().unwrap());
    manager.install_root = tmp.path().join("install");
    manager.staging_dir = tmp.path().join("staging");
    manager.installed_manifest_file = tmp.path().join("installed-manifest.json");
    std::fs::create_dir_all(&manager.install_root).unwrap();
    let name = format!("repair-{}", std::process::id());
    let ext = Component::Extension(name.clone());
    let same = crate::ComponentVersion { current_version: "0.2.0".into(), repair: true, ..staged(ext.clone(), &extension_zip(tmp.path())) };
    manager.status.components = vec![same];
    manager.save_pending_manifest().unwrap();
    manager.status.components.clear();
    let installed: std::collections::HashMap<String, String> = [(ext.manifest_key(), "0.2.0".to_string())].into_iter().collect();
    std::fs::write(&manager.installed_manifest_file, serde_json::to_string(&installed).unwrap()).unwrap();

    let (applied, failed) = manager.apply_staged(|_| {}).await.unwrap();

    assert_eq!(applied, vec![ext.manifest_key()]);
    assert!(failed.is_empty(), "{:?}", failed);
    let marker = crate::UpdateCompletionMarker::load().unwrap();
    assert_eq!((marker.result.as_deref(), marker.acknowledged, marker.success), (Some("repaired"), true, true));
    let _ = std::fs::remove_dir_all(manager.resolve_ext_dir(&name));
}

// 적용 대상 목록을 남긴 뒤 설치기로 그중 하나를 같거나 더 새 판으로 깔았으면, 그 키는 건너뛰고 나머지는 적용한다
#[tokio::test]
async fn an_apply_target_already_installed_is_skipped_and_the_rest_are_applied() {
    let _marker = MARKER.lock().await;
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

// 일부만 적용하면 고르지 않은 것은 받아 둔 채 다음 적용을 기다린다 (다시 확인·다운로드하지 않게)
#[tokio::test]
async fn applying_some_targets_keeps_the_rest_pending() {
    let _marker = MARKER.lock().await;
    let tmp = tempfile::tempdir().unwrap();
    let mut manager = UpdateManager::new(UpdateConfig::default(), tmp.path().join("modules").to_str().unwrap());
    manager.install_root = tmp.path().join("install");
    manager.staging_dir = tmp.path().join("staging");
    manager.installed_manifest_file = tmp.path().join("installed-manifest.json");
    std::fs::create_dir_all(&manager.install_root).unwrap();
    std::fs::create_dir_all(&manager.staging_dir).unwrap();
    let name = format!("partial-{}", std::process::id());
    let ext = Component::Extension(name.clone());
    let left = manager.staging_dir.join("locales.zip");
    std::fs::write(&left, b"zip").unwrap();
    manager.status.components = vec![staged(ext.clone(), &extension_zip(tmp.path())), staged(Component::Locales, &left)];
    manager.save_pending_manifest().unwrap();
    manager.save_updater_apply_targets(&[ext.manifest_key()]).unwrap();
    manager.status.components.clear();

    let (applied, failed) = manager.apply_staged(|_| {}).await.unwrap();

    assert_eq!(applied, vec![ext.manifest_key()]);
    assert!(failed.is_empty(), "{:?}", failed);
    assert!(left.exists(), "the download that was not chosen stays");
    assert!(!manager.staging_dir.join("apply-targets.json").exists());
    let mut next = UpdateManager::new(UpdateConfig::default(), tmp.path().join("modules").to_str().unwrap());
    next.staging_dir = manager.staging_dir.clone();
    next.installed_manifest_file = manager.installed_manifest_file.clone();
    assert_eq!(next.load_pending_manifest().unwrap(), 1);
    assert_eq!(next.status.components[0].component, Component::Locales);
    let _ = std::fs::remove_dir_all(manager.resolve_ext_dir(&name));
}

// 코어가 바뀌는 묶음에서 지금 코어로 돌지 않는 모듈·익스텐션은 적용하지 않고 받아 둔 채 둔다:
// 새 데몬이 건강하지 않아 코어만 되돌아가면 옛 코어가 거절하는 판이 남는다. 완료 마커의 `deferred`를 보고 건강한 새 데몬이 마저 바꾼다
#[tokio::test]
async fn an_addon_needing_the_new_core_waits_for_the_new_daemon() {
    use std::io::Write;
    let _marker = MARKER.lock().await;
    let tmp = tempfile::tempdir().unwrap();
    let mut manager = UpdateManager::new(UpdateConfig::default(), tmp.path().join("modules").to_str().unwrap());
    manager.install_root = tmp.path().join("install");
    manager.staging_dir = tmp.path().join("staging");
    manager.installed_manifest_file = tmp.path().join("installed-manifest.json");
    std::fs::create_dir_all(&manager.install_root).unwrap();
    std::fs::create_dir_all(&manager.staging_dir).unwrap();
    let core_zip = manager.staging_dir.join("saba-core.zip");
    {
        let mut zw = zip::ZipWriter::new(std::fs::File::create(&core_zip).unwrap());
        zw.start_file("saba-core", zip::write::SimpleFileOptions::default().unix_permissions(0o755)).unwrap();
        zw.write_all(b"new").unwrap();
        zw.finish().unwrap();
    }
    let (needs_new, runs_now) = (format!("needs-new-{}", std::process::id()), format!("runs-now-{}", std::process::id()));
    let (held, applied_now) = (Component::Extension(needs_new.clone()), Component::Extension(runs_now.clone()));
    let zip_dir = |name: &str| {
        let dir = tmp.path().join(name);
        std::fs::create_dir_all(&dir).unwrap();
        extension_zip(&dir)
    };
    let requires = |min: &str| Some([("saba-core".to_string(), min.to_string())].into_iter().collect());
    manager.status.components = vec![
        crate::ComponentVersion { latest_version: Some("99.0.0".into()), ..staged(Component::CoreDaemon, &core_zip) },
        crate::ComponentVersion { requires: requires(">=99.0.0"), ..staged(held.clone(), &zip_dir("held")) },
        crate::ComponentVersion { requires: requires(">=0.0.1"), ..staged(applied_now.clone(), &zip_dir("now")) },
    ];
    manager.save_pending_manifest().unwrap();
    manager.save_updater_apply_targets(&["saba-core".to_string(), held.manifest_key(), applied_now.manifest_key()]).unwrap();
    manager.status.components.clear();

    let (applied, failed) = manager.apply_staged(|_| {}).await.unwrap();

    assert!(failed.is_empty(), "{:?}", failed);
    assert_eq!(applied, vec!["saba-core".to_string(), applied_now.manifest_key()]);
    assert!(!manager.resolve_ext_dir(&needs_new).exists(), "the addon needing the new core is not applied yet");
    let marker = crate::UpdateCompletionMarker::load().unwrap();
    assert!(marker.success);
    assert_eq!(marker.deferred, vec![held.manifest_key()], "the new daemon finds what is left of the chosen bundle");
    let mut next = UpdateManager::new(UpdateConfig::default(), tmp.path().join("modules").to_str().unwrap());
    next.staging_dir = manager.staging_dir.clone();
    next.installed_manifest_file = tmp.path().join("other-installed-manifest.json");
    next.load_pending_manifest().unwrap();
    assert_eq!(next.status.components.iter().map(|c| c.component.clone()).collect::<Vec<_>>(), vec![held], "it stays staged");
    let _ = std::fs::remove_dir_all(manager.resolve_ext_dir(&runs_now));
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

/// 베타 빌드를 깐 사람이 채널을 고르지 않았으면 그 빌드의 채널에서 받는다.
/// 정식 채널이 기본이면 베타 빌드가 정식의 옛 판을 내림 업데이트로 거절해 업데이트·카탈로그를 받지 못한다.
#[test]
fn test_default_update_channel_is_the_build_channel() {
    assert_eq!(UpdateConfig::default().channel, crate::versions::bundled().channel);
    let saved: UpdateConfig = serde_json::from_str(r#"{"enabled": true}"#).unwrap();
    assert_eq!(saved.channel, crate::versions::bundled().channel);
}

// C(W1): 번역 묶음은 옆 폴더에 다 푼 뒤 맞바꾸고 판 번호를 `locales/version`에 적는다. 옆 폴더는 남지 않는다
#[tokio::test]
async fn locales_are_swapped_in_whole_with_their_version() {
    use std::io::Write;
    let tmp = tempfile::tempdir().unwrap();
    let mut manager = UpdateManager::new(UpdateConfig::default(), tmp.path().join("modules").to_str().unwrap());
    manager.install_root = tmp.path().join("install");
    manager.staging_dir = tmp.path().join("staging");
    let locales = manager.install_root.join("locales");
    std::fs::create_dir_all(locales.join("ko")).unwrap();
    std::fs::write(locales.join("ko/old.json"), b"{}").unwrap();
    let zip_path = tmp.path().join("locales.zip");
    {
        let mut zw = zip::ZipWriter::new(std::fs::File::create(&zip_path).unwrap());
        zw.start_file("ko/gui.json", zip::write::SimpleFileOptions::default()).unwrap();
        zw.write_all(b"{\"a\":1}").unwrap();
        zw.finish().unwrap();
    }
    let mut comp = staged(Component::Locales, &zip_path);
    comp.latest_version = Some("0.1.14".into());
    manager.status.components = vec![comp];

    let files = manager.apply_one(&Component::Locales, zip_path.to_str().unwrap()).await.unwrap();

    assert_eq!(std::fs::read_to_string(locales.join("version")).unwrap(), "0.1.14");
    assert!(locales.join("ko/gui.json").exists());
    assert!(!locales.join("ko/old.json").exists());
    assert!(files.contains(&locales.join("ko/gui.json")), "recorded paths point into locales/: {files:?}");
    let names: Vec<String> = std::fs::read_dir(&manager.install_root).unwrap().flatten().map(|e| e.file_name().to_string_lossy().into_owned()).collect();
    assert_eq!(names, vec!["locales".to_string()], "no side folder is left");
}

// C(W1): 맞바꾸다 끊겨 `locales`가 없고 옛 묶음만 옆에 남았으면 다음 정리가 옛 묶음을 되돌린다
#[test]
fn an_interrupted_locales_swap_is_put_back() {
    let tmp = tempfile::tempdir().unwrap();
    let mut manager = UpdateManager::new(UpdateConfig::default(), tmp.path().join("modules").to_str().unwrap());
    manager.install_root = tmp.path().to_path_buf();
    std::fs::create_dir_all(tmp.path().join(".locales-old/ko")).unwrap();
    std::fs::write(tmp.path().join(".locales-old/ko/bot.json"), b"{}").unwrap();
    std::fs::create_dir_all(tmp.path().join(".locales-new/ko")).unwrap();

    manager.settle_interrupted_updates();

    assert!(tmp.path().join("locales/ko/bot.json").exists());
    assert!(!tmp.path().join(".locales-old").exists());
    assert!(!tmp.path().join(".locales-new").exists());
}

/// 이 OS에서 비켜 둔 데몬·CLI 실행 파일 이름 (실행 파일, 백업)
fn moved_aside(name: &str) -> (String, String) {
    if cfg!(windows) { (format!("{name}.exe"), format!("{name}.exe.old")) } else { (name.to_string(), format!("{name}.old")) }
}

// Codex 4차 점검: 실행 파일을 비켜 두지 못하면 덮어쓰지 않고 실패한다 (백업 없이 덮으면 되돌릴 수 없다)
#[cfg(unix)]
#[test]
fn a_binary_that_cannot_be_moved_aside_is_not_overwritten() {
    let tmp = tempfile::tempdir().unwrap();
    let exe = tmp.path().join("saba-core");
    std::fs::write(&exe, b"old").unwrap();
    std::fs::create_dir_all(tmp.path().join("saba-core.old").join("x")).unwrap();
    assert!(UpdateManager::move_aside(&exe).is_err());
    assert_eq!(std::fs::read(&exe).unwrap(), b"old");
}

// C(W1): 되돌릴 때는 이번 적용이 비켜 둔 실행 파일만 모두 되돌린다. 적용 전부터 있던 백업(지난 업데이트 뒤로 도는 러너 등)은 건드리지 않는다
#[test]
fn a_rollback_restores_only_the_exes_this_apply_moved_aside() {
    let tmp = tempfile::tempdir().unwrap();
    let mut manager = UpdateManager::new(UpdateConfig::default(), tmp.path().join("modules").to_str().unwrap());
    manager.install_root = tmp.path().to_path_buf();
    let (core, core_old) = moved_aside("saba-core");
    let (cli, cli_old) = moved_aside("saba-chan-cli");
    // 적용 전: CLI 백업이 이미 있다 (쥐고 있어 못 지운 것)
    std::fs::write(tmp.path().join(&cli), b"cli now").unwrap();
    std::fs::write(tmp.path().join(&cli_old), b"cli older").unwrap();
    std::fs::write(tmp.path().join(&core), b"core old").unwrap();
    let before = manager.exe_backups();

    // 적용: 코어를 비켜 두고 새 판을 둔다
    std::fs::rename(tmp.path().join(&core), tmp.path().join(&core_old)).unwrap();
    std::fs::write(tmp.path().join(&core), b"core new").unwrap();

    let rollback = manager.roll_back_exes_since(&before).unwrap();

    assert_eq!(rollback.restored, vec![tmp.path().join(&core)]);
    assert!(rollback.pending.is_empty());
    assert_eq!(std::fs::read(tmp.path().join(&core)).unwrap(), b"core old");
    assert!(!tmp.path().join(&core_old).exists());
    assert_eq!(std::fs::read(tmp.path().join(&cli)).unwrap(), b"cli now", "a backup from before stays where it is");
    assert!(tmp.path().join(&cli_old).exists());
}

// Codex 4차 점검: 실행 파일을 되돌리지 못하면 성공으로 넘기지 않고 남은 것(백업 → 제자리)을 알린다.
// 새 판을 먼저 지우지 않는다 (예전처럼 비키기에 실패하면 지우고 나서 백업 옮기기도 실패하면 실행 파일이 아예 없어진다)
#[test]
fn a_rollback_that_cannot_move_the_new_exe_aside_reports_it() {
    let tmp = tempfile::tempdir().unwrap();
    let mut manager = UpdateManager::new(UpdateConfig::default(), tmp.path().join("modules").to_str().unwrap());
    manager.install_root = tmp.path().to_path_buf();
    let (core, core_old) = moved_aside("saba-core");
    std::fs::write(tmp.path().join(&core), b"core old").unwrap();
    let before = manager.exe_backups();
    std::fs::rename(tmp.path().join(&core), tmp.path().join(&core_old)).unwrap();
    std::fs::write(tmp.path().join(&core), b"core new").unwrap();
    // 비켜 둘 자리에 지울 수 없는 것(비지 않은 폴더)이 있다
    let mut aside = core.clone();
    aside.push_str(".rolled-back");
    std::fs::create_dir_all(tmp.path().join(&aside).join("x")).unwrap();

    let rollback = manager.roll_back_exes_since(&before).unwrap();

    assert!(rollback.restored.is_empty());
    assert_eq!(rollback.pending, vec![(tmp.path().join(&core_old), tmp.path().join(&core))]);
    assert_eq!(std::fs::read(tmp.path().join(&core)).unwrap(), b"core new", "the new exe is not deleted");
    assert_eq!(std::fs::read(tmp.path().join(&core_old)).unwrap(), b"core old");
}

// C(W1): 업데이터가 새 데몬을 지켜보는 동안에는 새 데몬이 `.old`를 지우지 않는다 (되돌림에 쓴다)
#[test]
fn old_backups_are_kept_while_the_updater_watches_the_new_daemon() {
    let tmp = tempfile::tempdir().unwrap();
    let mut manager = UpdateManager::new(UpdateConfig::default(), tmp.path().join("modules").to_str().unwrap());
    manager.install_root = tmp.path().join("install");
    manager.staging_dir = tmp.path().join("staging");
    std::fs::create_dir_all(&manager.install_root).unwrap();
    std::fs::create_dir_all(&manager.staging_dir).unwrap();
    let (core, core_old) = moved_aside("saba-core");
    std::fs::write(manager.install_root.join(&core), b"new").unwrap();
    std::fs::write(manager.install_root.join(&core_old), b"old").unwrap();

    crate::live_apply::begin_watch(&manager.staging_dir);
    manager.cleanup_old_files();
    assert!(manager.install_root.join(&core_old).exists(), "kept while the watch is on");

    crate::live_apply::end_watch(&manager.staging_dir);
    manager.cleanup_old_files();
    assert!(!manager.install_root.join(&core_old).exists());
}

// CLI zip은 명령 실행기를 `bin/saba-chan.exe`에 둔다: 그 백업도 설치 폴더 바로 안의 exe와 같게 되돌리고 지운다
#[test]
fn a_launcher_backup_inside_bin_is_rolled_back_and_cleaned_up_too() {
    let tmp = tempfile::tempdir().unwrap();
    let mut manager = UpdateManager::new(UpdateConfig::default(), tmp.path().join("modules").to_str().unwrap());
    manager.install_root = tmp.path().join("install");
    manager.staging_dir = tmp.path().join("staging");
    let bin = manager.install_root.join("bin");
    std::fs::create_dir_all(&bin).unwrap();
    std::fs::create_dir_all(&manager.staging_dir).unwrap();
    let (launcher, backup) = (bin.join("saba-chan.exe"), bin.join("saba-chan.exe.old"));

    let before = manager.exe_backups();
    std::fs::write(&backup, b"old").unwrap();
    std::fs::write(&launcher, b"new").unwrap();
    let rollback = manager.roll_back_exes_since(&before).unwrap();
    assert_eq!(rollback.restored, vec![launcher.clone()]);
    assert_eq!(std::fs::read(&launcher).unwrap(), b"old");

    // 적용이 끊겨 실행기가 없으면 되살린다
    std::fs::rename(&launcher, &backup).unwrap();
    manager.restore_moved_aside_exes();
    assert_eq!(std::fs::read(&launcher).unwrap(), b"old");

    std::fs::write(&backup, b"older").unwrap();
    manager.cleanup_old_files();
    assert!(!backup.exists());
    assert!(launcher.exists());
}

// 사바쨩 실행 파일 목록은 versions.toml `exes` 한 곳에서 온다: 거기 적힌 것(명령 실행기 포함)과 0.1.12 전 업데이터만
// 실행 파일로 다루고, 이름이 `saba-`로 시작할 뿐인 다른 파일의 `.exe.old`는 백업으로 보지 않는다(정리가 지우지 않게)
#[test]
fn the_exe_list_comes_from_versions_toml() {
    for stem in crate::versions::exe_stems() {
        assert!(UpdateManager::is_known_binary(std::path::Path::new(stem)), "{stem}");
    }
    assert!(UpdateManager::is_known_binary(std::path::Path::new("saba-chan-updater")));
    assert!(!UpdateManager::is_known_binary(std::path::Path::new("i18n.py")));

    let tmp = tempfile::tempdir().unwrap();
    let mut manager = UpdateManager::new(UpdateConfig::default(), tmp.path().join("modules").to_str().unwrap());
    manager.install_root = tmp.path().join("install");
    std::fs::create_dir_all(&manager.install_root).unwrap();
    for name in ["saba-runner.exe.old", "saba-notes.exe.old"] {
        std::fs::write(manager.install_root.join(name), b"").unwrap();
    }
    let found: Vec<String> = manager.old_exe_backups().into_iter().map(|(_, exe, ..)| exe).collect();
    assert_eq!(found, vec!["saba-runner.exe".to_string()]);
}

// 설치 점검·복구: 바뀌었거나 없어진 구성요소만, 확인한 릴리즈가 지금 설치된 판을 내줄 때만 그 판으로 다시 받는다.
// 다른 판을 내주면(업데이트가 있음) 고치지 않고 알린다. 읽지 못한 것·멀쩡한 것은 고르지 않는다
#[test]
fn repair_picks_broken_components_at_the_installed_version() {
    use crate::integrity::{ComponentIntegrity, IntegrityStatus};
    let tmp = tempfile::tempdir().unwrap();
    let mut manager = UpdateManager::new(UpdateConfig::default(), tmp.path().to_str().unwrap());
    let at = |component: Component, current: &str, latest: &str| crate::ComponentVersion {
        current_version: current.into(),
        latest_version: Some(latest.into()),
        update_available: current != latest,
        downloaded: false,
        downloaded_path: None,
        ..staged(component, tmp.path())
    };
    manager.status.components = vec![
        at(Component::Cli, "0.1.14", "0.1.14"),
        at(Component::Module("minecraft".into()), "1.8.2", "1.8.2"),
        at(Component::Gui, "0.1.14", "0.1.15"),
        at(Component::CoreDaemon, "0.1.14", "0.1.14"),
        at(Component::Module("palworld".into()), "2.0.0", "2.0.0"),
    ];
    let check = |key: &str, status: IntegrityStatus| ComponentIntegrity {
        component: key.into(),
        display_name: key.into(),
        status,
        expected_hash: None,
        actual_hash: None,
        file_path: None,
        message: String::new(),
    };
    let report = crate::integrity::IntegrityReport {
        checked_at: String::new(),
        overall: crate::integrity::OverallIntegrity::TamperDetected,
        components: vec![
            check("cli", IntegrityStatus::Tampered),
            check("module-minecraft", IntegrityStatus::FileNotFound),
            check("gui", IntegrityStatus::Tampered),
            check("saba-core", IntegrityStatus::Verified),
            check("module-palworld", IntegrityStatus::Error),
            check("ext-gone", IntegrityStatus::FileNotFound),
        ],
        total: 6,
        verified: 1,
        failed: 3,
        skipped: 2,
    };

    let plan = manager.repair_plan(&report);
    let keys = |items: &[crate::RepairItem]| items.iter().map(|i| (i.component.clone(), i.installed.clone(), i.offered.clone())).collect::<Vec<_>>();
    assert_eq!(
        keys(&plan.repair),
        vec![("cli".into(), "0.1.14".into(), Some("0.1.14".into())), ("module-minecraft".into(), "1.8.2".into(), Some("1.8.2".into()))]
    );
    assert_eq!(keys(&plan.unavailable), vec![("gui".into(), "0.1.14".into(), Some("0.1.15".into())), ("ext-gone".into(), String::new(), None)]);

    // 고칠 것만 다시 받을 수 있게 표시한다 (받기·적용은 업데이트와 같은 길이다)
    manager.mark_for_repair(&plan);
    let ready: Vec<String> = manager.status.components.iter().filter(|c| c.update_available).map(|c| c.component.manifest_key()).collect();
    assert_eq!(ready, vec!["cli", "module-minecraft", "gui"]);
}

// 코어·GUI·업데이터 복구는 업데이터가 적용한다: 받아 둔 같은 판을 "이미 설치됨"으로 건너뛰면 아무것도 고치지 않고 성공으로 끝난다
#[test]
fn the_updater_applies_a_repair_at_the_installed_version() {
    let tmp = tempfile::tempdir().unwrap();
    let file = tmp.path().join("core.zip");
    std::fs::write(&file, b"zip").unwrap();
    let mut manager = UpdateManager::new(UpdateConfig::default(), tmp.path().join("modules").to_str().unwrap());
    manager.staging_dir = tmp.path().join("staging");
    let same = |component: Component| crate::ComponentVersion { current_version: "0.1.14".into(), latest_version: Some("0.1.14".into()), ..staged(component, &file) };
    manager.status.components = vec![same(Component::CoreDaemon), same(Component::Gui)];
    let item = |key: &str| crate::RepairItem { component: key.into(), display_name: key.into(), installed: "0.1.14".into(), offered: Some("0.1.14".into()) };
    manager.mark_for_repair(&crate::RepairPlan { repair: vec![item("saba-core")], unavailable: Vec::new() });
    for c in &mut manager.status.components {
        c.downloaded = true;
        c.downloaded_path = Some(file.to_string_lossy().to_string());
    }
    manager.save_pending_manifest().unwrap();

    let mut loaded = UpdateManager::new(UpdateConfig::default(), tmp.path().join("modules").to_str().unwrap());
    loaded.staging_dir = manager.staging_dir.clone();
    loaded.installed_manifest_file = tmp.path().join("installed-manifest.json");
    let installed: std::collections::HashMap<String, String> =
        [("saba-core".to_string(), "0.1.14".to_string()), ("gui".to_string(), "0.1.14".to_string())].into_iter().collect();
    std::fs::write(&loaded.installed_manifest_file, serde_json::to_string(&installed).unwrap()).unwrap();
    loaded.load_pending_manifest().unwrap();
    assert_eq!(loaded.status.components.iter().map(|c| c.component.clone()).collect::<Vec<_>>(), vec![Component::CoreDaemon]);
    assert_eq!(loaded.already_installed, vec!["gui".to_string()], "only what was not marked for repair is skipped");
}
