//! GUI/CLI ↔ 업데이터 통신
//!
//! ## 통신 방식
//! 1. **데몬 IPC**: Saba-Core 데몬의 `/api/updates/*` 엔드포인트
//! 2. **직접 프로세스 통신**: 업데이터 CLI stdout/stdin
//! 3. **파일 기반**: 상태 파일을 통한 통신

use serde::{Deserialize, Serialize};
use std::path::PathBuf;
use std::time::Duration;

/// GUI/CLI에서 업데이터로 보내는 메시지
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type")]
pub enum UpdaterCommand {
    /// 버전 체크 요청
    CheckUpdates,
    /// 모든 업데이트 다운로드
    DownloadAll,
    /// 특정 컴포넌트 다운로드
    DownloadComponent { component: String },
    /// 업데이트 적용 (모듈만)
    ApplyModules,
    /// 전체 적용 시작 (GUI/CLI 종료 후)
    StartFullApply { 
        relaunch_exe: Option<String>,
        relaunch_args: Vec<String>,
    },
    /// 상태 조회
    GetStatus,
    /// 설정 조회
    GetConfig,
    /// 설정 업데이트
    UpdateConfig { config: serde_json::Value },
}

/// 업데이터에서 GUI/CLI로 보내는 응답
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type")]
pub enum UpdaterResponse {
    /// 성공 (데이터 포함)
    Success { data: serde_json::Value },
    /// 오류
    Error { message: String, recoverable: bool },
    /// 진행 상태
    Progress { 
        operation: String,
        percent: Option<u8>,
        message: String,
    },
    /// 알림
    Notification {
        title: String,
        message: String,
        severity: NotificationSeverity,
    },
}

/// 알림 심각도
#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum NotificationSeverity {
    Info,
    Warning,
    Error,
    Success,
}

/// 업데이트 상태 요약 (GUI 표시용)
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct UpdateSummary {
    /// 업데이트 가능한 컴포넌트 수
    pub updates_available: usize,
    /// 다운로드된 컴포넌트 수
    pub downloaded: usize,
    /// 마지막 체크 시각
    pub last_check: Option<String>,
    /// 현재 작업
    pub current_operation: Option<String>,
    /// 에러 메시지
    pub error: Option<String>,
}

/// 데몬 IPC 클라이언트
pub struct DaemonIpcClient {
    base_url: String,
    client: reqwest::Client,
}

impl DaemonIpcClient {
    pub fn new(daemon_port: u16) -> Self {
        let client = reqwest::Client::builder()
            .timeout(Duration::from_secs(30))
            .build()
            .expect("Failed to create HTTP client");

        Self {
            base_url: format!("http://127.0.0.1:{}", daemon_port),
            client,
        }
    }

    /// 버전 체크 요청
    pub async fn check_updates(&self) -> Result<serde_json::Value, String> {
        self.post("/api/updates/check", None).await
    }

    /// 상태 조회
    pub async fn get_status(&self) -> Result<serde_json::Value, String> {
        self.get("/api/updates/status").await
    }

    /// 설정 조회
    pub async fn get_config(&self) -> Result<serde_json::Value, String> {
        self.get("/api/updates/config").await
    }

    /// 설정 업데이트
    pub async fn update_config(&self, config: serde_json::Value) -> Result<serde_json::Value, String> {
        self.put("/api/updates/config", Some(config)).await
    }

    /// 다운로드 요청
    pub async fn download_all(&self) -> Result<serde_json::Value, String> {
        self.post("/api/updates/download", None).await
    }

    /// 적용 요청
    pub async fn apply(&self) -> Result<serde_json::Value, String> {
        self.post("/api/updates/apply", None).await
    }

    async fn get(&self, path: &str) -> Result<serde_json::Value, String> {
        let url = format!("{}{}", self.base_url, path);
        let resp = self.client
            .get(&url)
            .send()
            .await
            .map_err(|e| format!("Request failed: {}", e))?;

        if !resp.status().is_success() {
            return Err(format!("HTTP {}", resp.status()));
        }

        resp.json()
            .await
            .map_err(|e| format!("Parse error: {}", e))
    }

    async fn post(&self, path: &str, body: Option<serde_json::Value>) -> Result<serde_json::Value, String> {
        let url = format!("{}{}", self.base_url, path);
        let mut req = self.client.post(&url);
        
        if let Some(b) = body {
            req = req.json(&b);
        }

        let resp = req
            .send()
            .await
            .map_err(|e| format!("Request failed: {}", e))?;

        if !resp.status().is_success() {
            return Err(format!("HTTP {}", resp.status()));
        }

        resp.json()
            .await
            .map_err(|e| format!("Parse error: {}", e))
    }

    async fn put(&self, path: &str, body: Option<serde_json::Value>) -> Result<serde_json::Value, String> {
        let url = format!("{}{}", self.base_url, path);
        let mut req = self.client.put(&url);
        
        if let Some(b) = body {
            req = req.json(&b);
        }

        let resp = req
            .send()
            .await
            .map_err(|e| format!("Request failed: {}", e))?;

        if !resp.status().is_success() {
            return Err(format!("HTTP {}", resp.status()));
        }

        resp.json()
            .await
            .map_err(|e| format!("Parse error: {}", e))
    }
}

/// 상태 파일 관리
pub struct StateFile {
    path: PathBuf,
}

impl StateFile {
    pub fn new() -> Self {
        let path = Self::default_path();
        Self { path }
    }

    pub fn with_path(path: PathBuf) -> Self {
        Self { path }
    }

    fn default_path() -> PathBuf {
        #[cfg(target_os = "windows")]
        {
            std::env::var("APPDATA")
                .map(|appdata| PathBuf::from(appdata).join("saba-chan").join("updater-state.json"))
                .unwrap_or_else(|_| PathBuf::from("updater-state.json"))
        }
        #[cfg(not(target_os = "windows"))]
        {
            std::env::var("HOME")
                .map(|home| PathBuf::from(home).join(".saba-chan").join("updater-state.json"))
                .unwrap_or_else(|_| PathBuf::from("updater-state.json"))
        }
    }

    /// 상태 저장
    pub fn save(&self, summary: &UpdateSummary) -> Result<(), String> {
        if let Some(parent) = self.path.parent() {
            std::fs::create_dir_all(parent)
                .map_err(|e| format!("Failed to create directory: {}", e))?;
        }

        let json = serde_json::to_string_pretty(summary)
            .map_err(|e| format!("Serialize error: {}", e))?;

        std::fs::write(&self.path, json)
            .map_err(|e| format!("Write error: {}", e))
    }

    /// 상태 로드
    pub fn load(&self) -> Result<UpdateSummary, String> {
        let content = std::fs::read_to_string(&self.path)
            .map_err(|e| format!("Read error: {}", e))?;

        serde_json::from_str(&content)
            .map_err(|e| format!("Parse error: {}", e))
    }

    /// 상태 파일 삭제
    pub fn clear(&self) -> Result<(), String> {
        if self.path.exists() {
            std::fs::remove_file(&self.path)
                .map_err(|e| format!("Delete error: {}", e))?;
        }
        Ok(())
    }
}

impl Default for StateFile {
    fn default() -> Self {
        Self::new()
    }
}

/// 업데이트 완료 마커 — GUI/CLI 재시작 시 확인
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct UpdateCompletionMarker {
    pub timestamp: String,
    pub updated_components: Vec<String>,
    pub success: bool,
    pub message: Option<String>,
}

impl UpdateCompletionMarker {
    pub fn success(components: Vec<String>) -> Self {
        Self {
            timestamp: chrono::Utc::now().to_rfc3339(),
            updated_components: components,
            success: true,
            message: None,
        }
    }

    pub fn failure(message: String) -> Self {
        Self {
            timestamp: chrono::Utc::now().to_rfc3339(),
            updated_components: Vec::new(),
            success: false,
            message: Some(message),
        }
    }

    fn marker_path() -> PathBuf {
        #[cfg(target_os = "windows")]
        {
            std::env::var("APPDATA")
                .map(|appdata| PathBuf::from(appdata).join("saba-chan").join("update-complete.json"))
                .unwrap_or_else(|_| PathBuf::from("update-complete.json"))
        }
        #[cfg(not(target_os = "windows"))]
        {
            std::env::var("HOME")
                .map(|home| PathBuf::from(home).join(".saba-chan").join("update-complete.json"))
                .unwrap_or_else(|_| PathBuf::from("update-complete.json"))
        }
    }

    /// 마커 저장
    pub fn save(&self) -> Result<(), String> {
        let path = Self::marker_path();
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent)
                .map_err(|e| format!("Failed to create directory: {}", e))?;
        }

        let json = serde_json::to_string_pretty(self)
            .map_err(|e| format!("Serialize error: {}", e))?;

        std::fs::write(&path, json)
            .map_err(|e| format!("Write error: {}", e))
    }

    /// 마커 로드
    pub fn load() -> Option<Self> {
        let path = Self::marker_path();
        let content = std::fs::read_to_string(&path).ok()?;
        serde_json::from_str(&content).ok()
    }

    /// 마커 삭제
    pub fn clear() -> Result<(), String> {
        let path = Self::marker_path();
        if path.exists() {
            std::fs::remove_file(&path)
                .map_err(|e| format!("Delete error: {}", e))?;
        }
        Ok(())
    }

    /// 마커 존재 여부
    pub fn exists() -> bool {
        Self::marker_path().exists()
    }
}
