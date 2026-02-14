//! 업데이트 스케줄러 — 원샷 체크 유틸리티
//!
//! ## 아키텍처
//! 데몬 자체는 백그라운드 스케줄러를 돌리지 않습니다.
//! GUI/CLI가 주기적으로:
//! 1. 데몬 IPC `/api/updates/check`를 호출하거나
//! 2. `saba-chan-updater --cli check --notify` 프로세스를 스폰
//!    하여 업데이트를 확인합니다.
//!
//! 이 모듈은 그때 사용되는 원샷 유틸리티를 제공합니다.

use std::sync::Arc;
use tokio::sync::RwLock;

use super::UpdateManager;

/// 스케줄러 설정 — GUI/CLI가 타이머 간격을 결정할 때 참조
#[derive(Debug, Clone)]
pub struct SchedulerConfig {
    /// 체크 간격 (시간 단위, 기본 3시간)
    pub interval_hours: u32,
    /// 업데이터 활성화 여부
    pub enabled: bool,
}

impl Default for SchedulerConfig {
    fn default() -> Self {
        Self {
            interval_hours: 3,
            enabled: true,
        }
    }
}

impl SchedulerConfig {
    /// 체크 간격을 Duration으로 변환
    pub fn interval_duration(&self) -> std::time::Duration {
        std::time::Duration::from_secs(self.interval_hours as u64 * 3600)
    }

    /// 체크 간격을 밀리초로 (JS setInterval 등에서 사용)
    pub fn interval_millis(&self) -> u64 {
        self.interval_hours as u64 * 3600 * 1000
    }

    /// 하루에 몇 번 체크하는지 계산
    pub fn checks_per_day(&self) -> u32 {
        if self.interval_hours == 0 {
            return 0;
        }
        24 / self.interval_hours
    }
}

/// 원샷 업데이트 체크 결과
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct CheckResult {
    /// 업데이트가 있는 컴포넌트 수
    pub updates_available: usize,
    /// 전체 컴포넌트 수
    pub total_components: usize,
    /// 업데이트가 있는 컴포넌트 이름 목록
    pub update_names: Vec<String>,
    /// 오류 메시지 (있으면)
    pub error: Option<String>,
    /// 컴포넌트별 상세 상태 (GUI 표시용)
    #[serde(skip_serializing_if = "Option::is_none")]
    pub status: Option<super::UpdateStatus>,
}

/// 한 번 체크하고 결과를 반환합니다.
///
/// GUI/CLI가 주기적으로 호출하는 핵심 함수입니다.
/// 업데이트가 있으면 `CheckResult.updates_available > 0`,
/// 없으면 `0`을 반환합니다.
///
/// auto_download가 설정되어 있으면 자동으로 다운로드도 수행합니다.
pub async fn check_once(manager: &Arc<RwLock<UpdateManager>>) -> CheckResult {
    let mut mgr = manager.write().await;

    if !mgr.config.enabled {
        return CheckResult {
            updates_available: 0,
            total_components: 0,
            update_names: vec![],
            error: Some("Updater is disabled".into()),
            status: None,
        };
    }

    tracing::info!("[Updater] Running one-shot update check");

    match mgr.check_for_updates().await {
        Ok(status) => {
            let update_names: Vec<String> = status.components.iter()
                .filter(|c| c.update_available)
                .map(|c| c.component.display_name())
                .collect();
            let update_count = update_names.len();
            let total = status.components.len();

            if update_count > 0 {
                tracing::info!(
                    "[Updater] {} update(s) available: {}",
                    update_count,
                    update_names.join(", ")
                );

                // auto_download 설정 시 자동 다운로드
                if mgr.config.auto_download {
                    tracing::info!("[Updater] Auto-downloading updates...");
                    if let Err(e) = mgr.download_available_updates().await {
                        tracing::error!("[Updater] Auto-download failed: {}", e);
                    }
                }
            } else {
                tracing::info!("[Updater] All {} component(s) are up to date", total);
            }

            CheckResult {
                updates_available: update_count,
                total_components: total,
                update_names,
                error: None,
                status: Some(status),
            }
        }
        Err(e) => {
            tracing::error!("[Updater] Check failed: {}", e);
            CheckResult {
                updates_available: 0,
                total_components: 0,
                update_names: vec![],
                error: Some(format!("{}", e)),
                status: None,
            }
        }
    }
}

/// 체크 결과를 JSON 문자열로 직렬화 (프로세스 간 통신용)
///
/// `saba-chan-updater --cli check --json` 출력이나
/// GUI↔업데이터 프로세스 간 stdout 통신에 사용됩니다.
pub fn result_to_json(result: &CheckResult) -> String {
    serde_json::to_string(result).unwrap_or_else(|_| "{}".to_string())
}

/// 프로세스 종료 코드 결정
///
/// - `0` — 업데이트 있음 (호출측에서 알림 표시)
/// - `1` — 체크 실패 (에러)
/// - `2` — 업데이트 없음 (조용히 종료)
pub fn exit_code(result: &CheckResult) -> i32 {
    if result.error.is_some() {
        1
    } else if result.updates_available > 0 {
        0
    } else {
        2
    }
}
