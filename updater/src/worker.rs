//! 백그라운드 워커 — 버전 체크 및 다운로드를 백그라운드에서 처리
//!
//! ## 아키텍처
//! - `BackgroundWorker`: 독립적인 tokio 태스크로 실행
//! - GUI/CLI는 이벤트 구독을 통해 상태 변화를 수신
//! - 포그라운드 작업(적용)은 명시적 요청 시에만 실행

use std::sync::Arc;
use tokio::sync::{mpsc, RwLock, broadcast};
use std::time::Duration;

use crate::{UpdateManager, Component, ComponentVersion};

/// 백그라운드 작업 타입
#[derive(Debug, Clone)]
pub enum BackgroundTask {
    /// 버전 체크 (자동/수동)
    CheckVersion { manual: bool },
    /// 단일 컴포넌트 다운로드
    DownloadComponent { component: Component },
    /// 모든 업데이트 다운로드
    DownloadAll,
    /// 워커 종료
    Shutdown,
}

/// 워커에서 발생하는 이벤트 (GUI/CLI에 브로드캐스트)
#[derive(Debug, Clone)]
pub enum WorkerEvent {
    /// 버전 체크 시작
    CheckStarted,
    /// 버전 체크 완료
    CheckCompleted {
        updates_available: usize,
        components: Vec<ComponentVersion>,
    },
    /// 버전 체크 실패
    CheckFailed { error: String },
    /// 다운로드 시작
    DownloadStarted { component: String },
    /// 다운로드 진행률
    DownloadProgress {
        component: String,
        downloaded: u64,
        total: u64,
    },
    /// 다운로드 완료
    DownloadCompleted { component: String },
    /// 다운로드 실패
    DownloadFailed { component: String, error: String },
    /// 모든 다운로드 완료
    AllDownloadsCompleted { count: usize },
    /// 업데이트 알림 (GUI에 표시용)
    UpdateNotification {
        title: String,
        message: String,
        update_count: usize,
    },
    /// 워커 종료됨
    WorkerShutdown,
}

/// 백그라운드 워커 상태
#[derive(Debug, Clone, Default)]
pub struct WorkerStatus {
    /// 현재 작업 중인지
    pub busy: bool,
    /// 현재 태스크 설명
    pub current_task: Option<String>,
    /// 마지막 체크 시각
    pub last_check: Option<String>,
    /// 다음 자동 체크 시각
    pub next_check: Option<String>,
    /// 대기 중인 태스크 수
    pub pending_tasks: usize,
}

/// 백그라운드 워커
pub struct BackgroundWorker {
    /// 태스크 전송 채널
    task_tx: mpsc::Sender<BackgroundTask>,
    /// 이벤트 브로드캐스트 송신자
    event_tx: broadcast::Sender<WorkerEvent>,
    /// 워커 상태
    status: Arc<RwLock<WorkerStatus>>,
}

impl BackgroundWorker {
    /// 새 백그라운드 워커 생성 및 시작
    pub fn spawn(manager: Arc<RwLock<UpdateManager>>) -> Self {
        let (task_tx, task_rx) = mpsc::channel::<BackgroundTask>(32);
        let (event_tx, _) = broadcast::channel::<WorkerEvent>(64);
        let status = Arc::new(RwLock::new(WorkerStatus::default()));

        let worker = Self {
            task_tx,
            event_tx: event_tx.clone(),
            status: status.clone(),
        };

        // 워커 태스크 스폰
        let event_tx_clone = event_tx.clone();
        let status_clone = status.clone();
        tokio::spawn(async move {
            worker_loop(manager, task_rx, event_tx_clone, status_clone).await;
        });

        worker
    }

    /// 태스크 제출
    pub async fn submit(&self, task: BackgroundTask) -> Result<(), String> {
        self.task_tx
            .send(task)
            .await
            .map_err(|e| format!("Failed to submit task: {}", e))
    }

    /// 버전 체크 요청 (수동)
    pub async fn check_now(&self) -> Result<(), String> {
        self.submit(BackgroundTask::CheckVersion { manual: true }).await
    }

    /// 모든 업데이트 다운로드 요청
    pub async fn download_all(&self) -> Result<(), String> {
        self.submit(BackgroundTask::DownloadAll).await
    }

    /// 특정 컴포넌트 다운로드 요청
    pub async fn download_component(&self, component: Component) -> Result<(), String> {
        self.submit(BackgroundTask::DownloadComponent { component }).await
    }

    /// 이벤트 구독
    pub fn subscribe(&self) -> broadcast::Receiver<WorkerEvent> {
        self.event_tx.subscribe()
    }

    /// 현재 상태 조회
    pub async fn get_status(&self) -> WorkerStatus {
        self.status.read().await.clone()
    }

    /// 워커 종료
    pub async fn shutdown(&self) -> Result<(), String> {
        self.submit(BackgroundTask::Shutdown).await
    }
}

/// 워커 메인 루프
async fn worker_loop(
    manager: Arc<RwLock<UpdateManager>>,
    mut task_rx: mpsc::Receiver<BackgroundTask>,
    event_tx: broadcast::Sender<WorkerEvent>,
    status: Arc<RwLock<WorkerStatus>>,
) {
    tracing::info!("[Worker] Background worker started");

    loop {
        tokio::select! {
            Some(task) = task_rx.recv() => {
                match task {
                    BackgroundTask::Shutdown => {
                        tracing::info!("[Worker] Shutdown requested");
                        let _ = event_tx.send(WorkerEvent::WorkerShutdown);
                        break;
                    }
                    BackgroundTask::CheckVersion { manual } => {
                        handle_check_version(&manager, &event_tx, &status, manual).await;
                    }
                    BackgroundTask::DownloadComponent { component } => {
                        handle_download_component(&manager, &event_tx, &status, &component).await;
                    }
                    BackgroundTask::DownloadAll => {
                        handle_download_all(&manager, &event_tx, &status).await;
                    }
                }
            }
        }
    }

    tracing::info!("[Worker] Background worker stopped");
}

/// 버전 체크 처리
async fn handle_check_version(
    manager: &Arc<RwLock<UpdateManager>>,
    event_tx: &broadcast::Sender<WorkerEvent>,
    status: &Arc<RwLock<WorkerStatus>>,
    manual: bool,
) {
    {
        let mut s = status.write().await;
        s.busy = true;
        s.current_task = Some("Checking for updates...".to_string());
    }

    let _ = event_tx.send(WorkerEvent::CheckStarted);
    tracing::info!("[Worker] Starting version check (manual={})", manual);

    let result = {
        let mut mgr = manager.write().await;
        mgr.check_for_updates().await
    };

    match result {
        Ok(update_status) => {
            let updates: Vec<ComponentVersion> = update_status
                .components
                .iter()
                .filter(|c| c.update_available)
                .cloned()
                .collect();
            let update_count = updates.len();

            {
                let mut s = status.write().await;
                s.last_check = update_status.last_check.clone();
                s.next_check = update_status.next_check.clone();
            }

            let _ = event_tx.send(WorkerEvent::CheckCompleted {
                updates_available: update_count,
                components: update_status.components.clone(),
            });

            // 업데이트가 있으면 알림
            if update_count > 0 {
                let names: Vec<String> = updates.iter()
                    .map(|c| c.component.display_name())
                    .collect();
                let _ = event_tx.send(WorkerEvent::UpdateNotification {
                    title: format!("{} Update(s) Available", update_count),
                    message: names.join(", "),
                    update_count,
                });
            }

            tracing::info!("[Worker] Check completed: {} update(s) available", update_count);
        }
        Err(e) => {
            let error = format!("{}", e);
            let _ = event_tx.send(WorkerEvent::CheckFailed { error: error.clone() });
            tracing::error!("[Worker] Check failed: {}", error);
        }
    }

    {
        let mut s = status.write().await;
        s.busy = false;
        s.current_task = None;
    }
}

/// 단일 컴포넌트 다운로드 처리
async fn handle_download_component(
    manager: &Arc<RwLock<UpdateManager>>,
    event_tx: &broadcast::Sender<WorkerEvent>,
    status: &Arc<RwLock<WorkerStatus>>,
    component: &Component,
) {
    let comp_name = component.display_name();
    
    {
        let mut s = status.write().await;
        s.busy = true;
        s.current_task = Some(format!("Downloading {}...", comp_name));
    }

    let _ = event_tx.send(WorkerEvent::DownloadStarted {
        component: comp_name.clone(),
    });
    tracing::info!("[Worker] Starting download: {}", comp_name);

    let result = {
        let mut mgr = manager.write().await;
        mgr.download_component(component).await
    };

    match result {
        Ok(_) => {
            let _ = event_tx.send(WorkerEvent::DownloadCompleted {
                component: comp_name.clone(),
            });
            tracing::info!("[Worker] Download completed: {}", comp_name);
        }
        Err(e) => {
            let error = format!("{}", e);
            let _ = event_tx.send(WorkerEvent::DownloadFailed {
                component: comp_name.clone(),
                error: error.clone(),
            });
            tracing::error!("[Worker] Download failed for {}: {}", comp_name, error);
        }
    }

    {
        let mut s = status.write().await;
        s.busy = false;
        s.current_task = None;
    }
}

/// 모든 업데이트 다운로드 처리
async fn handle_download_all(
    manager: &Arc<RwLock<UpdateManager>>,
    event_tx: &broadcast::Sender<WorkerEvent>,
    status: &Arc<RwLock<WorkerStatus>>,
) {
    {
        let mut s = status.write().await;
        s.busy = true;
        s.current_task = Some("Downloading all updates...".to_string());
    }

    tracing::info!("[Worker] Starting download all");

    let result = {
        let mut mgr = manager.write().await;
        mgr.download_available_updates().await
    };

    match result {
        Ok(downloaded) => {
            let count = downloaded.len();
            for name in &downloaded {
                let _ = event_tx.send(WorkerEvent::DownloadCompleted {
                    component: name.clone(),
                });
            }
            let _ = event_tx.send(WorkerEvent::AllDownloadsCompleted { count });
            tracing::info!("[Worker] All downloads completed: {} component(s)", count);
        }
        Err(e) => {
            let error = format!("{}", e);
            let _ = event_tx.send(WorkerEvent::DownloadFailed {
                component: "all".to_string(),
                error: error.clone(),
            });
            tracing::error!("[Worker] Download all failed: {}", error);
        }
    }

    {
        let mut s = status.write().await;
        s.busy = false;
        s.current_task = None;
    }
}

/// 자동 체크 스케줄러 — 설정된 간격으로 백그라운드 체크 실행
pub struct AutoCheckScheduler {
    worker: Arc<BackgroundWorker>,
    interval_hours: u32,
    enabled: bool,
    handle: Option<tokio::task::JoinHandle<()>>,
}

impl AutoCheckScheduler {
    pub fn new(worker: Arc<BackgroundWorker>, interval_hours: u32, enabled: bool) -> Self {
        Self {
            worker,
            interval_hours,
            enabled,
            handle: None,
        }
    }

    /// 스케줄러 시작
    pub fn start(&mut self) {
        if !self.enabled || self.interval_hours == 0 {
            tracing::info!("[Scheduler] Auto-check disabled");
            return;
        }

        let worker = self.worker.clone();
        let interval = Duration::from_secs(self.interval_hours as u64 * 3600);

        let handle = tokio::spawn(async move {
            loop {
                tokio::time::sleep(interval).await;
                tracing::info!("[Scheduler] Auto-check triggered");
                if let Err(e) = worker.submit(BackgroundTask::CheckVersion { manual: false }).await {
                    tracing::error!("[Scheduler] Failed to submit auto-check: {}", e);
                }
            }
        });

        self.handle = Some(handle);
        tracing::info!("[Scheduler] Auto-check started (every {} hour(s))", self.interval_hours);
    }

    /// 스케줄러 중지
    pub fn stop(&mut self) {
        if let Some(handle) = self.handle.take() {
            handle.abort();
            tracing::info!("[Scheduler] Auto-check stopped");
        }
    }

    /// 설정 업데이트
    pub fn update_config(&mut self, interval_hours: u32, enabled: bool) {
        self.stop();
        self.interval_hours = interval_hours;
        self.enabled = enabled;
        self.start();
    }
}
