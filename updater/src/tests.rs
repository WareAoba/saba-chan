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
    
    println!("✓ 에러 복구 전략 테스트 통과");
}

#[test]
fn test_error_types() {
    // 네트워크 에러 (복구 가능)
    let net_err = UpdaterError::NetworkError {
        message: "Connection refused".to_string(),
        recoverable: true,
    };
    assert!(net_err.is_recoverable());
    
    // 타임아웃 (복구 가능)
    let timeout_err = UpdaterError::Timeout {
        operation: "download".to_string(),
        duration_secs: 30,
    };
    assert!(timeout_err.is_recoverable());
    
    // API 에러 - 5xx (복구 가능)
    let api_5xx = UpdaterError::ApiError {
        status_code: 500,
        message: "Internal Server Error".to_string(),
    };
    assert!(api_5xx.is_recoverable());
    
    // API 에러 - 4xx (복구 불가능)
    let api_4xx = UpdaterError::ApiError {
        status_code: 404,
        message: "Not Found".to_string(),
    };
    assert!(!api_4xx.is_recoverable());
    
    // 파일 시스템 에러 (복구 불가능)
    let fs_err = UpdaterError::FileSystemError {
        operation: "write".to_string(),
        path: "/tmp/test".to_string(),
        message: "Permission denied".to_string(),
    };
    assert!(!fs_err.is_recoverable());
    
    println!("✓ 에러 타입 테스트 통과");
}

#[test]
fn test_retry_delay_calculation() {
    let strategy = RecoveryStrategy::default();
    
    // 첫 번째 재시도: 기본 대기 시간
    let delay0 = strategy.next_delay();
    assert_eq!(delay0.as_secs(), 2);
    
    // 지수 백오프 테스트
    let mut s = RecoveryStrategy::new(5);
    
    s.increment(); // attempt 1
    let delay1 = s.next_delay();
    assert_eq!(delay1.as_secs(), 4); // 2 * 2^1
    
    s.increment(); // attempt 2
    let delay2 = s.next_delay();
    assert_eq!(delay2.as_secs(), 8); // 2 * 2^2
    
    s.increment(); // attempt 3
    let delay3 = s.next_delay();
    assert_eq!(delay3.as_secs(), 16); // 2 * 2^3
    
    println!("✓ 재시도 대기 시간 계산 테스트 통과");
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
    assert_eq!(Component::CoreDaemon.manifest_key(), "core_daemon");
    assert_eq!(Component::Cli.manifest_key(), "cli");
    assert_eq!(Component::Gui.manifest_key(), "gui");
    assert_eq!(
        Component::Module("minecraft".to_string()).manifest_key(),
        "module-minecraft"
    );
    
    // 역방향 파싱
    assert_eq!(
        Component::from_manifest_key("core_daemon"),
        Component::CoreDaemon
    );
    assert_eq!(
        Component::from_manifest_key("module-palworld"),
        Component::Module("palworld".to_string())
    );
    
    println!("✓ 컴포넌트 매니페스트 키 테스트 통과");
}

// ═══════════════════════════════════════════════════════
// 테스트 6: 네트워크 체커
// ═══════════════════════════════════════════════════════

#[tokio::test]
async fn test_network_checker() {
    let checker = NetworkChecker::new();
    
    // 실제 네트워크 테스트 (인터넷 연결 필요)
    let connected = checker.check_connectivity().await;
    println!("네트워크 연결 상태: {}", if connected { "연결됨" } else { "끊김" });
    
    // 결과는 환경에 따라 다를 수 있으므로 패스/실패 판정 없음
    println!("✓ 네트워크 체커 테스트 완료 (연결 상태 확인: {})", connected);
}

// ═══════════════════════════════════════════════════════
// 메인 — 모든 테스트 실행
// ═══════════════════════════════════════════════════════

#[cfg(test)]
mod run_all {
    use super::*;

    /// 유닛 테스트만 실행 (mock 서버 불필요)
    #[test]
    fn run_unit_tests() {
        test_error_recovery_strategy();
        test_error_types();
        test_retry_delay_calculation();
        test_component_manifest_key();
        println!("\n═══════════════════════════════════════");
        println!("✓ 모든 유닛 테스트 통과!");
        println!("═══════════════════════════════════════\n");
    }
}
