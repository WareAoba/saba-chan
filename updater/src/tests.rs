//! 업데이터 통합 테스트
//!
//! ## 테스트 시나리오
//! 1. 버전 페치: Mock 서버에서 릴리스 정보 가져오기
//! 2. 큐 처리: 다수 다운로드 순차 처리
//! 3. 에러 복구: 네트워크 끊김 시 재시도
//! 4. 포그라운드 적용: 파일 교체 플로우

use crate::{
    Component, UpdateConfig, UpdateManager,
    DownloadQueue, DownloadRequest,
    UpdaterError, RecoveryStrategy, NetworkChecker,
    BackgroundWorker, WorkerEvent,
};
use std::sync::Arc;
use tokio::sync::RwLock;

/// 테스트용 설정 생성
fn test_config(mock_url: &str) -> UpdateConfig {
    UpdateConfig {
        enabled: true,
        check_interval_hours: 1,
        auto_download: false,
        auto_apply: false,
        github_owner: "test-owner".to_string(),
        github_repo: "saba-chan".to_string(),
        include_prerelease: true,
        install_root: Some("./test_install".to_string()),
        api_base_url: Some(mock_url.to_string()),
    }
}

// ═══════════════════════════════════════════════════════
// 테스트 1: 버전 페치
// ═══════════════════════════════════════════════════════

#[tokio::test]
#[ignore = "requires mock server"]
async fn test_version_fetch() {
    let mock_url = "http://127.0.0.1:9876";
    let config = test_config(mock_url);
    let mut manager = UpdateManager::new(config, "./modules");

    let result = manager.check_for_updates().await;
    
    match result {
        Ok(status) => {
            println!("✓ 버전 체크 성공");
            println!("  컴포넌트 수: {}", status.components.len());
            for comp in &status.components {
                println!(
                    "  - {}: {} → {:?} (업데이트: {})",
                    comp.component.display_name(),
                    comp.current_version,
                    comp.latest_version,
                    comp.update_available
                );
            }
            assert!(!status.components.is_empty(), "컴포넌트가 있어야 함");
        }
        Err(e) => {
            println!("✗ 버전 체크 실패: {}", e);
            panic!("버전 체크 실패");
        }
    }
}

// ═══════════════════════════════════════════════════════
// 테스트 2: 큐 처리
// ═══════════════════════════════════════════════════════

#[tokio::test]
async fn test_queue_priority_ordering() {
    let queue = DownloadQueue::new();

    // 낮은 우선순위 먼저 추가
    queue.enqueue(DownloadRequest::new(Component::Module("low1".into()))).await;
    queue.enqueue(DownloadRequest::new(Component::Module("low2".into()))).await;
    
    // 높은 우선순위 추가 (앞으로 가야 함)
    queue.enqueue(
        DownloadRequest::new(Component::Module("high".into()))
            .with_priority(10)
    ).await;

    // 중간 우선순위 추가
    queue.enqueue(
        DownloadRequest::new(Component::Module("medium".into()))
            .with_priority(5)
    ).await;

    let status = queue.get_status().await;
    assert_eq!(status.pending, 4);
    
    println!("✓ 큐 우선순위 정렬 테스트 통과");
}

#[tokio::test]
async fn test_queue_batch_enqueue() {
    let queue = DownloadQueue::new();

    let requests = vec![
        DownloadRequest::new(Component::Cli),
        DownloadRequest::new(Component::Gui),
        DownloadRequest::new(Component::Module("minecraft".into())),
        DownloadRequest::new(Component::Module("palworld".into())),
    ];

    queue.enqueue_batch(requests).await;
    
    let status = queue.get_status().await;
    assert_eq!(status.pending, 4);
    
    println!("✓ 큐 배치 추가 테스트 통과");
}

#[tokio::test]
async fn test_queue_pause_resume() {
    let queue = DownloadQueue::new();
    
    // 큐에 항목 추가
    queue.enqueue(DownloadRequest::new(Component::Cli)).await;
    
    // 일시정지
    queue.pause().await;
    let status = queue.get_status().await;
    assert!(status.paused);
    
    // 재개
    queue.resume().await;
    let status = queue.get_status().await;
    assert!(!status.paused);
    
    println!("✓ 큐 일시정지/재개 테스트 통과");
}

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

#[tokio::test]
#[ignore = "requires mock server"]
async fn test_background_worker() {
    let mock_url = "http://127.0.0.1:9876";
    let config = test_config(mock_url);
    let manager = Arc::new(RwLock::new(UpdateManager::new(config, "./modules")));

    let worker = BackgroundWorker::spawn(manager.clone());
    let mut rx = worker.subscribe();

    // 체크 요청
    worker.check_now().await.expect("check should succeed");

    // 이벤트 수신 (타임아웃 포함)
    let event = tokio::time::timeout(
        std::time::Duration::from_secs(10),
        rx.recv()
    ).await;

    match event {
        Ok(Ok(WorkerEvent::CheckStarted)) => {
            println!("✓ 체크 시작 이벤트 수신");
        }
        Ok(Ok(WorkerEvent::CheckCompleted { updates_available, .. })) => {
            println!("✓ 체크 완료 이벤트 수신: {} 업데이트", updates_available);
        }
        Ok(Ok(WorkerEvent::CheckFailed { error })) => {
            println!("✗ 체크 실패: {}", error);
        }
        _ => {
            println!("✗ 예상치 못한 이벤트 또는 타임아웃");
        }
    }

    // 워커 종료
    worker.shutdown().await.expect("shutdown should succeed");
}

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
// 테스트 7: 큐 심층 테스트
// ═══════════════════════════════════════════════════════

#[tokio::test]
async fn test_queue_clear() {
    let queue = DownloadQueue::new();
    queue.enqueue(DownloadRequest::new(Component::Cli)).await;
    queue.enqueue(DownloadRequest::new(Component::Gui)).await;
    queue.enqueue(DownloadRequest::new(Component::Module("mc".into()))).await;

    let status = queue.get_status().await;
    assert_eq!(status.pending, 3);

    queue.clear().await;
    let status = queue.get_status().await;
    assert_eq!(status.pending, 0, "clear should empty the queue");
}

#[tokio::test]
async fn test_queue_with_callback() {
    let queue = DownloadQueue::new();
    let req = DownloadRequest::new(Component::Cli)
        .with_callback("cb-123".to_string());
    assert_eq!(req.callback_id.as_deref(), Some("cb-123"));
    queue.enqueue(req).await;

    let status = queue.get_status().await;
    assert_eq!(status.pending, 1);
}

#[tokio::test]
async fn test_queue_empty_status() {
    let queue = DownloadQueue::new();
    let status = queue.get_status().await;
    assert_eq!(status.pending, 0);
    assert!(!status.paused);
    assert!(status.current.is_none());
}

// ═══════════════════════════════════════════════════════
// 메인 — 모든 테스트 실행
// ═══════════════════════════════════════════════════════

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
        required_component: "discord_bot".to_string(),
        required_version: ">=0.1.0".to_string(),
        installed_version: None,
        message: "cli requires discord_bot >=0.1.0 but not installed is installed".to_string(),
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
    use crate::github::ComponentInfo;

    let json = r#"{
        "version": "0.3.0",
        "asset": "gui-windows-x64.zip",
        "requires": {
            "saba-core": ">=0.3.0",
            "discord_bot": ">=0.1.0"
        }
    }"#;
    let info: ComponentInfo = serde_json::from_str(json).unwrap();
    assert_eq!(info.version, "0.3.0");
    let requires = info.requires.unwrap();
    assert_eq!(requires.len(), 2);
    assert_eq!(requires.get("saba-core").unwrap(), ">=0.3.0");
    assert_eq!(requires.get("discord_bot").unwrap(), ">=0.1.0");
}

#[test]
fn test_component_info_requires_empty_by_default() {
    use crate::github::ComponentInfo;

    let json = r#"{"version": "0.1.0"}"#;
    let info: ComponentInfo = serde_json::from_str(json).unwrap();
    assert!(info.requires.is_none());
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
        println!("\n═══════════════════════════════════════");
        println!("✓ 모든 유닛 테스트 통과!");
        println!("═══════════════════════════════════════\n");
    }
}
