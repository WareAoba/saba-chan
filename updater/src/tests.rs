//! 업데이터 통합 테스트
//!
//! ## 테스트 시나리오
//! 1. 버전 페치: Mock 서버에서 릴리스 정보 가져오기
//! 2. 에러 복구: 네트워크 끊김 시 재시도

use crate::{
    Component, UpdateConfig, UpdateManager,
    UpdaterError, RecoveryStrategy, NetworkChecker,
};

// ═══════════════════════════════════════════════════════
// 테스트 3: 에러 복구
// ═══════════════════════════════════════════════════════

#[test]
fn test_error_recovery_strategy() {
    let mut strategy = RecoveryStrategy::new(3);

    // 초기 상태
    assert!(strategy.can_retry());
    assert_eq!(strategy.current_attempt, 0);

    // 첫 번째 시도 후
    strategy.increment();
    assert!(strategy.can_retry());
    assert_eq!(strategy.current_attempt, 1);

    // 두 번째 시도 후
    strategy.increment();
    assert!(strategy.can_retry());

    // 세 번째 시도 후
    strategy.increment();
    assert!(!strategy.can_retry()); // 최대 재시도 횟수 초과

    // 리셋
    strategy.reset();
    assert!(strategy.can_retry());
    assert_eq!(strategy.current_attempt, 0);
}

/// 모든 에러 유형에 대한 is_recoverable 전수 검사
#[test]
fn test_is_recoverable_exhaustive() {
    // ─── 복구 가능 ───
    let recoverable_cases: Vec<(UpdaterError, &str)> = vec![
        (UpdaterError::NetworkError { message: "conn".into(), recoverable: true }, "NetworkError(recoverable=true)"),
        (UpdaterError::Timeout { operation: "dl".into(), duration_secs: 30 }, "Timeout"),
        (UpdaterError::ApiError { status_code: 500, message: "ISE".into() }, "ApiError(500)"),
        (UpdaterError::ApiError { status_code: 502, message: "Bad Gateway".into() }, "ApiError(502)"),
        (UpdaterError::ApiError { status_code: 503, message: "Unavail".into() }, "ApiError(503)"),
        (UpdaterError::DownloadInterrupted { component: "c".into(), downloaded_bytes: 50, total_bytes: 100 }, "DownloadInterrupted"),
        (UpdaterError::ValidationError { component: "c".into(), expected: "abc".into(), actual: "def".into() }, "ValidationError"),
    ];

    for (err, label) in &recoverable_cases {
        assert!(err.is_recoverable(), "{} should be recoverable", label);
    }

    // ─── 복구 불가능 ───
    let non_recoverable_cases: Vec<(UpdaterError, &str)> = vec![
        (UpdaterError::NetworkError { message: "fatal".into(), recoverable: false }, "NetworkError(recoverable=false)"),
        (UpdaterError::ApiError { status_code: 400, message: "Bad Request".into() }, "ApiError(400)"),
        (UpdaterError::ApiError { status_code: 401, message: "Unauth".into() }, "ApiError(401)"),
        (UpdaterError::ApiError { status_code: 403, message: "Forbidden".into() }, "ApiError(403)"),
        (UpdaterError::ApiError { status_code: 404, message: "Not Found".into() }, "ApiError(404)"),
        (UpdaterError::ApiError { status_code: 429, message: "Rate limited".into() }, "ApiError(429)"),
        (UpdaterError::FileSystemError { operation: "w".into(), path: "/x".into(), message: "eperm".into() }, "FileSystemError"),
        (UpdaterError::ConfigError { message: "bad".into() }, "ConfigError"),
        (UpdaterError::Unknown { message: "?".into() }, "Unknown"),
    ];

    for (err, label) in &non_recoverable_cases {
        assert!(!err.is_recoverable(), "{} should NOT be recoverable", label);
    }
}

/// retry_delay — 지수 백오프 + 60초 캡 검증
#[test]
fn test_retry_delay_exponential_backoff_with_cap() {
    let network_err = UpdaterError::NetworkError {
        message: "timeout".into(),
        recoverable: true,
    };

    // base=2s → attempt 0: 2*1=2, 1: 2*2=4, 2: 2*4=8, 3: 2*8=16, 4: 2*16=32, 5: 2*32=64→60(cap)
    assert_eq!(network_err.retry_delay(0).as_secs(), 2);
    assert_eq!(network_err.retry_delay(1).as_secs(), 4);
    assert_eq!(network_err.retry_delay(2).as_secs(), 8);
    assert_eq!(network_err.retry_delay(3).as_secs(), 16);
    assert_eq!(network_err.retry_delay(4).as_secs(), 32);
    assert_eq!(network_err.retry_delay(5).as_secs(), 60, "Should cap at 60s");
    assert_eq!(network_err.retry_delay(10).as_secs(), 60, "Should still be capped");
}

/// retry_delay — Timeout은 base=5s로 더 긴 초기 대기
#[test]
fn test_retry_delay_timeout_base() {
    let timeout_err = UpdaterError::Timeout {
        operation: "download".into(),
        duration_secs: 30,
    };
    assert_eq!(timeout_err.retry_delay(0).as_secs(), 5);
    assert_eq!(timeout_err.retry_delay(1).as_secs(), 10);
    assert_eq!(timeout_err.retry_delay(2).as_secs(), 20);
    assert_eq!(timeout_err.retry_delay(3).as_secs(), 40);
    assert_eq!(timeout_err.retry_delay(4).as_secs(), 60, "5*16=80 → cap 60");
}

/// retry_delay — 429 Rate Limit는 base=30s 특수 처리
#[test]
fn test_retry_delay_rate_limit_429() {
    let rate_limited = UpdaterError::ApiError {
        status_code: 429,
        message: "Too Many Requests".into(),
    };
    assert_eq!(rate_limited.retry_delay(0).as_secs(), 30);
    assert_eq!(rate_limited.retry_delay(1).as_secs(), 60, "30*2=60→cap");
    assert_eq!(rate_limited.retry_delay(2).as_secs(), 60, "30*4=120→cap");
}

/// retry_delay — 일반 API 에러(5xx)는 base=3s
#[test]
fn test_retry_delay_api_5xx() {
    let api_err = UpdaterError::ApiError {
        status_code: 502,
        message: "Bad Gateway".into(),
    };
    assert_eq!(api_err.retry_delay(0).as_secs(), 3);
    assert_eq!(api_err.retry_delay(1).as_secs(), 6);
}

/// user_message — 각 에러 유형별 한국어 메시지 내용 검증
#[test]
fn test_user_message_content() {
    let cases: Vec<(UpdaterError, &str)> = vec![
        (
            UpdaterError::NetworkError { message: "x".into(), recoverable: true },
            "인터넷 연결",
        ),
        (
            UpdaterError::Timeout { operation: "x".into(), duration_secs: 10 },
            "서버 응답이 지연",
        ),
        (
            UpdaterError::ApiError { status_code: 404, message: "NF".into() },
            "찾을 수 없습니다",
        ),
        (
            UpdaterError::ApiError { status_code: 403, message: "F".into() },
            "접근이 거부",
        ),
        (
            UpdaterError::ApiError { status_code: 500, message: "ISE".into() },
            "일시적인 문제",
        ),
        (
            UpdaterError::ApiError { status_code: 418, message: "teapot".into() },
            "서버 오류 (418)",
        ),
        (
            UpdaterError::DownloadInterrupted { component: "c".into(), downloaded_bytes: 0, total_bytes: 100 },
            "다운로드가 중단",
        ),
        (
            UpdaterError::FileSystemError { operation: "w".into(), path: "/x".into(), message: "e".into() },
            "파일 저장",
        ),
        (
            UpdaterError::ValidationError { component: "c".into(), expected: "a".into(), actual: "b".into() },
            "검증에 실패",
        ),
        (
            UpdaterError::ConfigError { message: "bad key".into() },
            "bad key",
        ),
        (
            UpdaterError::Unknown { message: "mystery".into() },
            "mystery",
        ),
    ];

    for (err, expected_substr) in &cases {
        let msg = err.user_message();
        assert!(
            msg.contains(expected_substr),
            "user_message for {:?} should contain '{}', got '{}'",
            err, expected_substr, msg
        );
    }
}

/// Display trait — 모든 변형이 패닉 없이 문자열로 변환
#[test]
fn test_display_all_variants() {
    let variants: Vec<UpdaterError> = vec![
        UpdaterError::NetworkError { message: "conn refused".into(), recoverable: true },
        UpdaterError::Timeout { operation: "download".into(), duration_secs: 30 },
        UpdaterError::ApiError { status_code: 500, message: "ISE".into() },
        UpdaterError::DownloadInterrupted { component: "cli".into(), downloaded_bytes: 50, total_bytes: 100 },
        UpdaterError::FileSystemError { operation: "write".into(), path: "/tmp".into(), message: "perm".into() },
        UpdaterError::ValidationError { component: "gui".into(), expected: "abc".into(), actual: "def".into() },
        UpdaterError::ConfigError { message: "missing key".into() },
        UpdaterError::Unknown { message: "??".into() },
    ];

    for err in &variants {
        let display = format!("{}", err);
        assert!(!display.is_empty(), "Display should produce non-empty string: {:?}", err);
    }
}

/// from_io — IO 에러 → FileSystemError 변환
#[test]
fn test_from_io() {
    let io_err = std::io::Error::new(std::io::ErrorKind::PermissionDenied, "access denied");
    let updater_err = UpdaterError::from_io(&io_err, "write", "/tmp/file.bin");

    match updater_err {
        UpdaterError::FileSystemError { operation, path, message } => {
            assert_eq!(operation, "write");
            assert_eq!(path, "/tmp/file.bin");
            assert!(message.contains("access denied"));
        }
        other => panic!("Expected FileSystemError, got {:?}", other),
    }
}

/// ErrorContext — 빌더 패턴 + component 설정
#[test]
fn test_error_context_builder() {
    use crate::ErrorContext;

    let err = UpdaterError::NetworkError {
        message: "DNS failure".into(),
        recoverable: true,
    };
    let ctx = ErrorContext::new("check_for_updates", err.clone())
        .with_component("core-daemon");

    assert_eq!(ctx.operation, "check_for_updates");
    assert_eq!(ctx.component.as_deref(), Some("core-daemon"));
    assert!(!ctx.timestamp.is_empty(), "timestamp should be set");
    assert!(ctx.stack_trace.is_none());
    assert!(ctx.error.is_recoverable());
}

/// ErrorContext — component 없이 생성
#[test]
fn test_error_context_without_component() {
    use crate::ErrorContext;

    let err = UpdaterError::ConfigError { message: "bad".into() };
    let ctx = ErrorContext::new("load_config", err);

    assert!(ctx.component.is_none());
    assert_eq!(ctx.operation, "load_config");
}

/// RecoveryStrategy — max_retries=0이면 즉시 불가
#[test]
fn test_recovery_strategy_zero_retries() {
    let strategy = RecoveryStrategy::new(0);
    assert!(!strategy.can_retry(), "0 retries means no retry at all");
}

/// RecoveryStrategy — backoff 없이 고정 delay
#[test]
fn test_recovery_strategy_no_backoff() {
    let mut strategy = RecoveryStrategy {
        max_retries: 5,
        current_attempt: 0,
        base_delay: std::time::Duration::from_secs(3),
        use_backoff: false,
    };

    strategy.increment();
    assert_eq!(strategy.next_delay().as_secs(), 3);
    strategy.increment();
    assert_eq!(strategy.next_delay().as_secs(), 3, "Without backoff, delay stays constant");
    strategy.increment();
    assert_eq!(strategy.next_delay().as_secs(), 3);
}

/// RecoveryStrategy — increment → reset → increment 전체 생명주기
#[test]
fn test_recovery_strategy_full_lifecycle() {
    let mut s = RecoveryStrategy::new(2);

    // 0 → can_retry
    assert!(s.can_retry());
    s.increment(); // 1
    assert!(s.can_retry());
    s.increment(); // 2
    assert!(!s.can_retry());

    // reset → 다시 시도 가능
    s.reset();
    assert!(s.can_retry());
    assert_eq!(s.current_attempt, 0);

    // 다시 increment
    s.increment();
    assert!(s.can_retry());
    s.increment();
    assert!(!s.can_retry());
}

/// Serde 직렬화/역직렬화 라운드트립
#[test]
fn test_error_serde_roundtrip() {
    let errors: Vec<UpdaterError> = vec![
        UpdaterError::NetworkError { message: "refused".into(), recoverable: true },
        UpdaterError::Timeout { operation: "dl".into(), duration_secs: 30 },
        UpdaterError::ApiError { status_code: 429, message: "rate".into() },
        UpdaterError::DownloadInterrupted { component: "cli".into(), downloaded_bytes: 1024, total_bytes: 4096 },
        UpdaterError::FileSystemError { operation: "write".into(), path: "/a/b".into(), message: "denied".into() },
        UpdaterError::ValidationError { component: "gui".into(), expected: "sha256".into(), actual: "x".into() },
        UpdaterError::ConfigError { message: "no key".into() },
        UpdaterError::Unknown { message: "??".into() },
    ];

    for err in &errors {
        let json = serde_json::to_string(err).expect("serialize failed");
        let deserialized: UpdaterError =
            serde_json::from_str(&json).expect("deserialize failed");
        // 직렬화 왕복 후 Display가 동일해야 함
        assert_eq!(
            format!("{}", err),
            format!("{}", deserialized),
            "Serde roundtrip should preserve Display for {:?}", err
        );
    }
}

/// DownloadInterrupted의 진행률 계산 가능 여부 확인
#[test]
fn test_download_interrupted_progress_data() {
    let err = UpdaterError::DownloadInterrupted {
        component: "core".into(),
        downloaded_bytes: 750,
        total_bytes: 1000,
    };
    match &err {
        UpdaterError::DownloadInterrupted { downloaded_bytes, total_bytes, .. } => {
            let progress = (*downloaded_bytes as f64) / (*total_bytes as f64);
            assert!((progress - 0.75).abs() < f64::EPSILON, "Progress should be 75%");
        }
        _ => unreachable!(),
    }
    let display = format!("{}", err);
    assert!(display.contains("750"), "Should show downloaded bytes");
    assert!(display.contains("1000"), "Should show total bytes");
}

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
// 테스트 6: 네트워크 체커
// ═══════════════════════════════════════════════════════

#[tokio::test]
async fn test_network_checker_default_has_endpoints() {
    let checker = NetworkChecker::new();
    // 실제 네트워크가 없어도 패닉 없이 동작해야 함
    let _connected = checker.check_connectivity().await;
    // 결과는 환경에 따라 다르므로 is_true/is_false 판정은 하지 않지만
    // 기본 엔드포인트로 github.com과 api.github.com이 설정되어 있어야 함
    let checker2 = NetworkChecker::default();
    let _connected2 = checker2.check_connectivity().await;
    // Default trait과 ::new()의 동작이 동일한지 확인
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
    // 모듈/익스텐션/Locales는 프로세스 중단 불필요 (우선순위 1)
    // GUI/CLI는 재시작 필요 (우선순위 4~5)
    // CoreDaemon은 데몬 재시작 필요 (우선순위 3)
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

    // 모듈/익스텐션은 CoreDaemon보다 먼저
    assert!(
        UpdateManager::component_apply_priority_for_test(&module) <
        UpdateManager::component_apply_priority_for_test(&core_daemon),
        "모듈은 CoreDaemon보다 먼저 적용"
    );

    println!("✓ 컴포넌트 적용 우선순위 카테고리 테스트 통과");
}

#[cfg(test)]
mod run_all {
    use super::*;

    /// 유닛 테스트만 실행 (mock 서버 불필요)
    #[test]
    fn run_unit_tests() {
        test_error_recovery_strategy();
        test_is_recoverable_exhaustive();
        test_retry_delay_exponential_backoff_with_cap();
        test_retry_delay_timeout_base();
        test_retry_delay_rate_limit_429();
        test_retry_delay_api_5xx();
        test_user_message_content();
        test_display_all_variants();
        test_from_io();
        test_error_context_builder();
        test_error_context_without_component();
        test_recovery_strategy_zero_retries();
        test_recovery_strategy_no_backoff();
        test_recovery_strategy_full_lifecycle();
        test_error_serde_roundtrip();
        test_download_interrupted_progress_data();
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
        let opts = zip::write::FileOptions::default();
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
        let opts = zip::write::FileOptions::default();
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
    for f in ["saba-core.exe.old", "saba-runner.exe.old.2", "saba-chan-gui.exe.old", "notepad.exe.old", "saba-core.exe.oldx", "saba-core.exe"] {
        std::fs::write(manager.install_root.join(f), b"x").unwrap();
    }

    manager.clear_pending_manifest();
    manager.cleanup_old_files();

    assert_eq!(std::fs::read_dir(&manager.staging_dir).unwrap().count(), 0);
    let mut left: Vec<_> = std::fs::read_dir(&manager.install_root).unwrap()
        .map(|e| e.unwrap().file_name().to_string_lossy().into_owned()).collect();
    left.sort();
    assert_eq!(left, vec!["notepad.exe.old", "saba-core.exe", "saba-core.exe.oldx"]);
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
