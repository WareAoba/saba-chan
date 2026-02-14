//! 다운로드 큐 — 다수의 다운로드 요청을 순차적으로 처리
//!
//! ## 특징
//! - FIFO 큐로 다운로드 요청 관리
//! - 재시도 로직 (네트워크 오류 시)
//! - 우선순위 지원 (긴급 다운로드)
//! - 일시정지/재개 기능

use std::collections::VecDeque;
use std::sync::Arc;
use std::time::Duration;
use tokio::sync::{mpsc, RwLock, Mutex};

use crate::{Component, UpdateManager};

/// 다운로드 요청
#[derive(Debug, Clone)]
pub struct DownloadRequest {
    /// 다운로드할 컴포넌트
    pub component: Component,
    /// 우선순위 (높을수록 먼저)
    pub priority: u8,
    /// 재시도 횟수
    pub retries: u8,
    /// 최대 재시도 횟수
    pub max_retries: u8,
    /// 콜백 ID (옵션)
    pub callback_id: Option<String>,
}

impl DownloadRequest {
    pub fn new(component: Component) -> Self {
        Self {
            component,
            priority: 0,
            retries: 0,
            max_retries: 3,
            callback_id: None,
        }
    }

    pub fn with_priority(mut self, priority: u8) -> Self {
        self.priority = priority;
        self
    }

    pub fn with_callback(mut self, id: String) -> Self {
        self.callback_id = Some(id);
        self
    }
}

/// 다운로드 결과
#[derive(Debug, Clone)]
pub struct DownloadResult {
    pub component: Component,
    pub success: bool,
    pub error: Option<String>,
    pub callback_id: Option<String>,
}

/// 큐 상태
#[derive(Debug, Clone)]
pub struct QueueStatus {
    /// 대기 중인 요청 수
    pub pending: usize,
    /// 처리 완료된 요청 수
    pub completed: usize,
    /// 실패한 요청 수
    pub failed: usize,
    /// 현재 처리 중인 컴포넌트
    pub current: Option<String>,
    /// 일시정지 여부
    pub paused: bool,
}

/// 다운로드 큐 매니저
pub struct DownloadQueue {
    /// 요청 큐 (우선순위 정렬)
    queue: Arc<Mutex<VecDeque<DownloadRequest>>>,
    /// 결과 수신 채널
    result_rx: Arc<Mutex<mpsc::Receiver<DownloadResult>>>,
    /// 결과 송신 채널
    result_tx: mpsc::Sender<DownloadResult>,
    /// 상태
    status: Arc<RwLock<QueueStatus>>,
    /// 일시정지 플래그
    paused: Arc<RwLock<bool>>,
    /// 처리 중 플래그
    processing: Arc<RwLock<bool>>,
}

impl DownloadQueue {
    pub fn new() -> Self {
        let (result_tx, result_rx) = mpsc::channel(64);
        
        Self {
            queue: Arc::new(Mutex::new(VecDeque::new())),
            result_rx: Arc::new(Mutex::new(result_rx)),
            result_tx,
            status: Arc::new(RwLock::new(QueueStatus {
                pending: 0,
                completed: 0,
                failed: 0,
                current: None,
                paused: false,
            })),
            paused: Arc::new(RwLock::new(false)),
            processing: Arc::new(RwLock::new(false)),
        }
    }

    /// 다운로드 요청 추가
    pub async fn enqueue(&self, request: DownloadRequest) {
        let mut queue = self.queue.lock().await;
        
        // 우선순위에 따라 삽입 위치 결정
        let pos = queue
            .iter()
            .position(|r| r.priority < request.priority)
            .unwrap_or(queue.len());
        
        queue.insert(pos, request);
        
        let mut status = self.status.write().await;
        status.pending = queue.len();
        
        tracing::debug!("[Queue] Request enqueued, pending: {}", queue.len());
    }

    /// 여러 요청 일괄 추가
    pub async fn enqueue_batch(&self, requests: Vec<DownloadRequest>) {
        for req in requests {
            self.enqueue(req).await;
        }
    }

    /// 큐 처리 시작
    pub async fn process(&self, manager: Arc<RwLock<UpdateManager>>) {
        let mut processing = self.processing.write().await;
        if *processing {
            tracing::warn!("[Queue] Already processing");
            return;
        }
        *processing = true;
        drop(processing);

        tracing::info!("[Queue] Starting queue processing");

        loop {
            // 일시정지 체크
            if *self.paused.read().await {
                tokio::time::sleep(Duration::from_millis(100)).await;
                continue;
            }

            // 다음 요청 가져오기
            let request = {
                let mut queue = self.queue.lock().await;
                queue.pop_front()
            };

            let request = match request {
                Some(r) => r,
                None => {
                    // 큐가 비었으면 종료
                    tracing::info!("[Queue] Queue empty, stopping");
                    break;
                }
            };

            // 현재 처리 중 상태 업데이트
            {
                let mut status = self.status.write().await;
                status.current = Some(request.component.display_name());
                status.pending = self.queue.lock().await.len();
            }

            // 다운로드 실행
            let result = self.execute_download(&manager, &request).await;

            // 결과 처리
            if result.success {
                let mut status = self.status.write().await;
                status.completed += 1;
                status.current = None;
            } else {
                // 재시도 로직
                if request.retries < request.max_retries {
                    let mut retry_request = request.clone();
                    retry_request.retries += 1;
                    tracing::warn!(
                        "[Queue] Download failed, scheduling retry ({}/{}): {}",
                        retry_request.retries,
                        retry_request.max_retries,
                        result.error.as_deref().unwrap_or("unknown")
                    );
                    
                    // 재시도 전 대기
                    tokio::time::sleep(Duration::from_secs(2u64.pow(retry_request.retries as u32))).await;
                    
                    // 큐 앞에 다시 추가 (우선 처리)
                    let mut queue = self.queue.lock().await;
                    queue.push_front(retry_request);
                } else {
                    let mut status = self.status.write().await;
                    status.failed += 1;
                    status.current = None;
                    tracing::error!(
                        "[Queue] Download failed after {} retries: {}",
                        request.max_retries,
                        request.component.display_name()
                    );
                }
            }

            // 결과 전송
            let _ = self.result_tx.send(result).await;
        }

        let mut processing = self.processing.write().await;
        *processing = false;
        
        tracing::info!("[Queue] Queue processing completed");
    }

    /// 단일 다운로드 실행
    async fn execute_download(
        &self,
        manager: &Arc<RwLock<UpdateManager>>,
        request: &DownloadRequest,
    ) -> DownloadResult {
        tracing::info!("[Queue] Downloading: {}", request.component.display_name());

        let result = {
            let mut mgr = manager.write().await;
            mgr.download_component(&request.component).await
        };

        match result {
            Ok(_) => DownloadResult {
                component: request.component.clone(),
                success: true,
                error: None,
                callback_id: request.callback_id.clone(),
            },
            Err(e) => DownloadResult {
                component: request.component.clone(),
                success: false,
                error: Some(format!("{}", e)),
                callback_id: request.callback_id.clone(),
            },
        }
    }

    /// 큐 일시정지
    pub async fn pause(&self) {
        let mut paused = self.paused.write().await;
        *paused = true;
        let mut status = self.status.write().await;
        status.paused = true;
        tracing::info!("[Queue] Paused");
    }

    /// 큐 재개
    pub async fn resume(&self) {
        let mut paused = self.paused.write().await;
        *paused = false;
        let mut status = self.status.write().await;
        status.paused = false;
        tracing::info!("[Queue] Resumed");
    }

    /// 큐 비우기
    pub async fn clear(&self) {
        let mut queue = self.queue.lock().await;
        queue.clear();
        let mut status = self.status.write().await;
        status.pending = 0;
        tracing::info!("[Queue] Cleared");
    }

    /// 상태 조회
    pub async fn get_status(&self) -> QueueStatus {
        self.status.read().await.clone()
    }

    /// 결과 수신 (비동기)
    pub async fn recv_result(&self) -> Option<DownloadResult> {
        let mut rx = self.result_rx.lock().await;
        rx.recv().await
    }

    /// 처리 중 여부
    pub async fn is_processing(&self) -> bool {
        *self.processing.read().await
    }
}

impl Default for DownloadQueue {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn test_queue_priority() {
        let queue = DownloadQueue::new();

        // 낮은 우선순위 먼저 추가
        queue.enqueue(DownloadRequest::new(Component::Module("low".into()))).await;
        
        // 높은 우선순위 추가 (앞으로 가야 함)
        queue.enqueue(DownloadRequest::new(Component::Module("high".into())).with_priority(10)).await;

        let q = queue.queue.lock().await;
        assert_eq!(q.len(), 2);
        // 첫 번째가 high여야 함
        if let Component::Module(name) = &q[0].component {
            assert_eq!(name, "high");
        }
    }

    #[tokio::test]
    async fn test_queue_batch() {
        let queue = DownloadQueue::new();

        let requests = vec![
            DownloadRequest::new(Component::Cli),
            DownloadRequest::new(Component::Gui),
            DownloadRequest::new(Component::Module("minecraft".into())),
        ];

        queue.enqueue_batch(requests).await;
        
        let status = queue.get_status().await;
        assert_eq!(status.pending, 3);
    }
}
