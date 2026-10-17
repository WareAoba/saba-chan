//! # saba-chan 업데이터 라이브러리
//!
//! 코어 데몬, CLI, GUI, 업데이터, 모듈, 익스텐션, 로케일 컴포넌트를 설치·업데이트한다.
//!
//! ## 흐름
//! 1. 릴레이 `/v1/updates/check` → 최신 버전 확인
//! 2. 릴레이 `/v1/updates/manifest/:version` → Ed25519 서명 검증된 매니페스트
//! 3. 매니페스트의 `url`(R2)에서 자산 다운로드 + SHA-256 검증 (해시 필수)
//! 4. 적용 직전 스테이징 파일 SHA-256 재검증 → 압축 해제 (실패 시 롤백)
//!
//! ## 모듈 구성
//! - [`relay`] / [`signature`] / [`manifest`] — 릴레이 API, 서명 검증, 매니페스트 스키마
//! - `staging` — 다운로드와 `pending.json` / `apply-targets.json` 관리
//! - `apply` — 다운로드된 업데이트 적용
//! - `install` — 신규 설치 / 재설치
//! - `extract` — 안전한 압축 해제(zip-slip·심볼릭 링크 차단, 바이너리 교체 롤백)
//! - `local_versions` — 로컬 버전 수집, `installed-manifest.json`
//! - [`process`] — 실행 중인 프로세스 확인
//! - [`completion`] — 업데이트 완료 마커
//! - [`integrity`] — 파일 해시, 검증 다운로드, SHA256SUMS 파싱
//! - [`launch_ticket`] — 업데이터 실행 티켓

pub mod completion;
pub mod constants;
pub mod integrity;
pub mod launch_ticket;
pub mod manifest;
pub mod process;
pub mod relay;
pub mod signature;
pub mod version;

mod apply;
mod extract;
mod install;
mod local_versions;
mod staging;

#[cfg(test)]
mod tests;

pub use apply::ApplyBatchOutcome;
pub use completion::UpdateCompletionMarker;
pub use manifest::{ComponentInfo, ReleaseManifest, ResolvedComponent};
pub use process::ProcessChecker;

use anyhow::Result;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::{Arc, Mutex as StdMutex};

use relay::{Channel, RelayClient};
use signature::PublicKeyRegistry;
use version::SemVer;

// ══════════════════════════════════════════════════════
// 다운로드 진행률
// ══════════════════════════════════════════════════════

/// 다운로드 진행 상태 (Manager 외부에서 안전하게 폴링 가능)
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct DownloadProgress {
    /// 현재 다운로드 중인 컴포넌트 키 (None = 대기 중)
    pub component: Option<String>,
    /// 수신한 바이트 수
    pub bytes_received: u64,
    /// 전체 바이트 수 (Content-Length, 0이면 알 수 없음)
    pub total_bytes: u64,
    /// 다운로드 활성 여부
    pub active: bool,
}

// ══════════════════════════════════════════════════════
// 컴포넌트 정의
// ══════════════════════════════════════════════════════

/// 업데이트 대상. 각 컴포넌트를 나타내는 열거형
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, Hash)]
#[serde(rename_all = "snake_case")]
pub enum Component {
    CoreDaemon,
    Cli,
    Gui,
    Updater,
    Module(String),
    Extension(String),
    Locales,
}

impl Component {
    /// manifest.json에서 사용하는 키 반환
    pub fn manifest_key(&self) -> String {
        match self {
            Component::CoreDaemon => "saba-core".to_string(),
            Component::Cli => "cli".to_string(),
            Component::Gui => "gui".to_string(),
            Component::Updater => "updater".to_string(),
            Component::Module(name) => format!("module-{}", name),
            Component::Extension(name) => format!("ext-{}", name),
            Component::Locales => "locales".to_string(),
        }
    }

    /// manifest 키로부터 Component 생성 (알 수 없는 키는 None)
    pub fn try_from_manifest_key(key: &str) -> Option<Self> {
        match key {
            "saba-core" => Some(Component::CoreDaemon),
            "cli" => Some(Component::Cli),
            "gui" => Some(Component::Gui),
            "updater" => Some(Component::Updater),
            "locales" => Some(Component::Locales),
            _ => {
                if let Some(name) = key.strip_prefix("module-") {
                    Some(Component::Module(name.to_string()))
                } else {
                    key.strip_prefix("ext-")
                        .map(|name| Component::Extension(name.to_string()))
                }
            }
        }
    }

    /// manifest 키로부터 Component 생성 (하위 호환용, 알 수 없는 키는 Module로 폴백)
    pub fn from_manifest_key(key: &str) -> Self {
        Self::try_from_manifest_key(key).unwrap_or_else(|| Component::Module(key.to_string()))
    }

    /// 사용자 표시용 이름을 반환하는 메서드
    pub fn display_name(&self) -> String {
        match self {
            Component::CoreDaemon => "Saba-Core".to_string(),
            Component::Cli => "CLI".to_string(),
            Component::Gui => "GUI".to_string(),
            Component::Updater => "Updater".to_string(),
            Component::Module(name) => format!("Module: {}", name),
            Component::Extension(name) => format!("Extension: {}", name),
            Component::Locales => "Locales".to_string(),
        }
    }
}

/// 컴포넌트별 버전 추적 정보
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ComponentVersion {
    pub component: Component,
    pub current_version: String,
    pub latest_version: Option<String>,
    pub update_available: bool,
    pub download_url: Option<String>,
    pub asset_name: Option<String>,
    pub release_notes: Option<String>,
    pub published_at: Option<String>,
    /// 다운로드 완료 여부
    pub downloaded: bool,
    /// 다운로드 완료된 파일의 경로 (적용 대기 중인 경우)
    pub downloaded_path: Option<String>,
    /// 해당 컴포넌트가 설치되어 있는지 여부 (false면 미설치 상태)
    pub installed: bool,
    /// 서명된 매니페스트의 자산 SHA-256 (hex). 적용 직전 스테이징 파일 재검증에 사용.
    #[serde(default)]
    pub sha256: Option<String>,
}

/// 전체 업데이트 상태 정보
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct UpdateStatus {
    pub last_check: Option<String>,
    pub next_check: Option<String>,
    pub components: Vec<ComponentVersion>,
    pub checking: bool,
    pub error: Option<String>,
}

/// 개별 컴포넌트 적용 결과
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ApplyComponentResult {
    /// 컴포넌트 manifest 키 (예: "module-minecraft", "saba-core")
    pub component: String,
    /// 적용 성공 여부 (실패는 `Err`로 반환되므로 `Ok`이면 항상 true)
    pub success: bool,
    /// 결과 메시지
    pub message: String,
    /// 업데이트 적용을 위해 중단한 프로세스 목록 (현재 항상 비어 있음)
    pub stopped_processes: Vec<String>,
    /// 재시작 필요 여부
    pub restart_needed: bool,
}

/// 기본 릴레이 채널.
fn default_channel() -> String {
    "stable".to_string()
}

/// 현재 빌드 타겟 기본 플랫폼 문자열.
fn default_platform() -> String {
    // Windows x64 가 주 타겟. 다른 플랫폼은 설정에서 명시 필요.
    if cfg!(target_os = "windows") {
        "windows-x64".to_string()
    } else if cfg!(target_os = "linux") {
        "linux-x64".to_string()
    } else if cfg!(target_os = "macos") {
        "macos-x64".to_string()
    } else {
        "unknown".to_string()
    }
}

/// 업데이트 설정 (Relay 전용).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct UpdateConfig {
    pub enabled: bool,
    /// 확인 주기 (시간 단위, 기본값 3, 최소 1시간에서 최대 8시간)
    pub check_interval_hours: u32,
    /// 다운로드 후 자동 적용 여부
    pub auto_download: bool,
    /// 다운로드 완료 후 자동 적용 (모듈만 자동 적용, CoreDaemon/CLI/GUI는 재시작 필요로 별도 처리)
    pub auto_apply: bool,
    /// 설치 루트 (기본값: 실행 파일 디렉터리)
    pub install_root: Option<String>,
    /// 릴레이 서버 베이스 URL (끝 슬래시 선택).
    pub relay_base_url: String,
    /// 업데이트 채널 (`stable` | `beta` | `nightly`).
    #[serde(default = "default_channel")]
    pub channel: String,
    /// 플랫폼 식별자 (`windows-x64` | `linux-x64` 등).
    #[serde(default = "default_platform")]
    pub platform: String,
    /// 클라이언트 식별자 (롤아웃 버킷 계산용, 최소 8자).
    #[serde(default)]
    pub client_id: String,
}

impl Default for UpdateConfig {
    fn default() -> Self {
        Self {
            enabled: true,
            check_interval_hours: 3,
            auto_download: false,
            auto_apply: false,
            install_root: None,
            relay_base_url: crate::constants::DEFAULT_RELAY_BASE_URL.to_string(),
            channel: default_channel(),
            platform: default_platform(),
            client_id: String::new(),
        }
    }
}

// ══════════════════════════════════════════════════════
// UpdateManager
// ══════════════════════════════════════════════════════

/// 업데이트 확인·다운로드·적용과 단일 컴포넌트 설치를 관리한다.
///
/// 기능별 구현은 하위 모듈(`staging`, `apply`, `install`, `local_versions`)의
/// `impl UpdateManager` 블록에 나뉘어 있다.
pub struct UpdateManager {
    pub config: UpdateConfig,
    status: UpdateStatus,
    /// 모듈 디렉터리 경로 (%APPDATA%/saba-chan/modules)
    modules_dir: PathBuf,
    /// 익스텐션 디렉터리 경로 (%APPDATA%/saba-chan/extensions)
    extensions_dir: PathBuf,
    /// 업데이트 다운로드 파일 저장 디렉터리
    staging_dir: PathBuf,
    /// 설치 루트 디렉터리 (다운로드/적용 기준)
    install_root: PathBuf,
    /// 캐시된 최신 매니페스트 (릴레이에서 서명 검증 후 저장)
    cached_manifest: Option<ReleaseManifest>,
    public_keys: PublicKeyRegistry,
    /// 매니페스트 키 → 다운로드 소스
    resolved_components: HashMap<String, ResolvedComponent>,
    /// 최신 릴리스 노트 (CheckResponse.latest.notes).
    cached_release_notes: Option<String>,
    /// 최신 릴리스 발행일 (CheckResponse.latest.releasedAt).
    cached_published_at: Option<String>,
    /// 다운로드 진행 상태 (Arc로 공유 — Manager 잠금 없이 폴링 가능)
    pub download_progress: Arc<StdMutex<DownloadProgress>>,
}

impl UpdateManager {
    pub fn new(config: UpdateConfig, modules_dir: &str) -> Self {
        let install_root = config.install_root.as_ref()
            .map(PathBuf::from)
            .unwrap_or_else(|| {
                std::env::current_exe()
                    .ok()
                    .and_then(|p| p.parent().map(|d| d.to_path_buf()))
                    .unwrap_or_else(|| PathBuf::from("."))
            });

        let modules_dir = PathBuf::from(modules_dir);
        let extensions_dir = crate::constants::resolve_extensions_dir();
        let _ = std::fs::create_dir_all(&modules_dir);
        let _ = std::fs::create_dir_all(&extensions_dir);

        Self {
            config,
            status: UpdateStatus {
                last_check: None,
                next_check: None,
                components: Vec::new(),
                checking: false,
                error: None,
            },
            modules_dir,
            extensions_dir,
            staging_dir: crate::constants::resolve_staging_dir(),
            install_root,
            cached_manifest: None,
            public_keys: PublicKeyRegistry::bundled(),
            resolved_components: HashMap::new(),
            cached_release_notes: None,
            cached_published_at: None,
            download_progress: Arc::new(StdMutex::new(DownloadProgress::default())),
        }
    }

    pub fn with_public_keys(mut self, public_keys: PublicKeyRegistry) -> Self {
        self.public_keys = public_keys;
        self
    }

    /// 익스텐션 ID에서 디스크 상의 디렉토리 경로를 해석합니다.
    /// Python import 호환을 위해 하이픈→언더스코어 변환도 시도합니다.
    fn resolve_ext_dir(&self, ext_name: &str) -> PathBuf {
        let exact = self.extensions_dir.join(ext_name);
        if exact.exists() {
            return exact;
        }
        let safe_name = ext_name.replace('-', "_");
        if safe_name != ext_name {
            let safe = self.extensions_dir.join(&safe_name);
            if safe.exists() {
                return safe;
            }
        }
        // 없으면 Python-safe 이름으로 반환 (신규 설치용)
        self.extensions_dir.join(&safe_name)
    }

    /// 현재 업데이트 상태를 반환
    pub fn get_status(&self) -> UpdateStatus {
        self.status.clone()
    }

    /// 설정된 채널 문자열을 `Channel` enum 으로 파싱.
    fn parse_channel(&self) -> Channel {
        match self.config.channel.to_ascii_lowercase().as_str() {
            "beta" => Channel::Beta,
            "nightly" => Channel::Nightly,
            _ => Channel::Stable,
        }
    }

    /// `config.client_id` 가 비어 있으면 install_root 해시로 결정적 식별자를 만든다.
    /// (릴레이는 client_id 최소 8자를 요구함 — 롤아웃 버킷 계산용)
    fn effective_client_id(&self) -> String {
        let cid = self.config.client_id.trim();
        if cid.len() >= 8 {
            cid.to_string()
        } else {
            use sha2::{Digest, Sha256};
            let digest = Sha256::digest(self.install_root.to_string_lossy().as_bytes());
            hex::encode(&digest[..8])
        }
    }

    /// 현재 설정 반환
    pub fn get_config(&self) -> UpdateConfig {
        self.config.clone()
    }

    /// 설정 업데이트 (install_root 가 지정되면 실제 경로도 갱신)
    pub fn update_config(&mut self, new_config: UpdateConfig) {
        if let Some(ref root) = new_config.install_root {
            let new_root = PathBuf::from(root);
            if new_root != self.install_root {
                tracing::info!("[UpdateManager] install_root updated: {:?} -> {:?}", self.install_root, new_root);
                self.install_root = new_root;
            }
        }
        self.config = new_config;
    }

    // ─── 릴레이 매니페스트 ────────────────────────────────────────────────

    /// 릴레이에서 서명된 최신 매니페스트를 가져온다(공개 API).
    ///
    /// Ed25519 서명 검증 후 파싱된 `ReleaseManifest` 를 반환하며 인스턴스에 캐시한다.
    /// 같은 `UpdateManager` 에서 이어지는 `install_component()` 는 재-fetch 없이 사용한다.
    pub async fn fetch_remote_manifest(&mut self) -> Result<ReleaseManifest> {
        self.fetch_latest_manifest_via_relay().await
    }

    /// 릴레이에 check → 최신 버전의 서명된 매니페스트 fetch → 경로 검증 → 캐시.
    async fn fetch_latest_manifest_via_relay(&mut self) -> Result<ReleaseManifest> {
        if self.config.relay_base_url.trim().is_empty() {
            anyhow::bail!("relay_base_url not configured");
        }
        if self.public_keys.is_empty() {
            anyhow::bail!("공개키 레지스트리가 비어 있음 — 서명된 매니페스트 검증 불가");
        }
        let client = RelayClient::new(&self.config.relay_base_url);

        let current_core = self
            .collect_local_versions()
            .remove("saba-core")
            .unwrap_or_else(|| env!("CARGO_PKG_VERSION").to_string());
        let client_id = self.effective_client_id();

        let resp = client
            .check(self.parse_channel(), &self.config.platform, &current_core, &client_id)
            .await
            .map_err(|e| anyhow::anyhow!("릴레이 check 실패: {}", e))?;

        let latest = resp
            .latest
            .ok_or_else(|| anyhow::anyhow!("릴레이가 latest 정보를 반환하지 않았습니다"))?;

        self.cached_release_notes = latest.notes.as_ref().map(|v| match v {
            serde_json::Value::String(s) => s.clone(),
            other => other.to_string(),
        });
        self.cached_published_at = latest.released_at.clone();

        let manifest_json = client
            .fetch_manifest(&latest.version, &self.public_keys)
            .await
            .map_err(|e| anyhow::anyhow!("매니페스트 fetch/검증 실패: {}", e))?;

        let manifest: ReleaseManifest = serde_json::from_value(manifest_json)
            .map_err(|e| anyhow::anyhow!("매니페스트 파싱 실패: {}", e))?;

        self.validate_manifest_paths(&manifest)?;
        self.resolved_components = Self::build_resolved_components(&manifest);
        self.cached_manifest = Some(manifest.clone());
        Ok(manifest)
    }

    /// 매니페스트의 `components` 를 `ResolvedComponent` 맵으로 변환.
    /// `url` 이 없는 컴포넌트는 다운로드할 수 없으므로 제외한다.
    fn build_resolved_components(manifest: &ReleaseManifest) -> HashMap<String, ResolvedComponent> {
        manifest
            .components
            .iter()
            .filter_map(|(key, info)| {
                let url = info.url.as_deref().filter(|u| !u.is_empty())?;
                let resolved = ResolvedComponent {
                    latest_version: info.version.clone(),
                    source_release_tag: manifest.version.clone(),
                    download_url: url.to_string(),
                    asset_name: info.asset.clone().unwrap_or_else(|| format!("{}.zip", key)),
                    install_dir: info.install_dir.clone(),
                    sha256: info.sha256.clone(),
                    requires: info.requires.clone(),
                };
                Some((key.clone(), resolved))
            })
            .collect()
    }

    fn validate_component_name(name: &str) -> Result<()> {
        if name.is_empty() || matches!(name, "." | "..")
            || name.contains(['/', '\\', ':']) || name.ends_with(['.', ' ']) {
            anyhow::bail!("Invalid component or asset name: {}", name);
        }
        Ok(())
    }

    fn validate_component(component: &Component) -> Result<()> {
        if let Component::Module(name) | Component::Extension(name) = component {
            Self::validate_component_name(name)?;
        }
        Ok(())
    }

    fn validate_manifest_paths(&self, manifest: &ReleaseManifest) -> Result<()> {
        for (key, info) in &manifest.components {
            Self::validate_component_name(key)?;
            let component = Component::from_manifest_key(key);
            Self::validate_component(&component)?;
            if let Some(asset) = &info.asset {
                Self::validate_component_name(asset)?;
            }
            if let Some(asset) = &info.asset_linux {
                Self::validate_component_name(asset)?;
            }
            self.resolve_install_dir(&component, info.install_dir.as_deref())?;
        }
        Ok(())
    }

    // ─── 업데이트 확인 ────────────────────────────────────────────────────────

    /// 릴레이에서 최신 매니페스트를 받아 컴포넌트별 업데이트 여부를 반환한다.
    ///
    /// 모든 컴포넌트(코어·모듈·익스텐션)가 동일 매니페스트에 포함된다.
    pub async fn check_for_updates(&mut self) -> Result<UpdateStatus> {
        if self.config.relay_base_url.trim().is_empty() {
            anyhow::bail!("relay_base_url not configured");
        }

        self.status.checking = true;
        self.status.error = None;

        let local_versions = self.collect_local_versions();

        let manifest = match self.fetch_latest_manifest_via_relay().await {
            Ok(m) => m,
            Err(e) => {
                tracing::error!("[Updater] Relay check failed: {}", e);
                self.status.checking = false;
                self.status.error = Some(format!("Relay check failed: {}", e));
                return Err(e);
            }
        };

        let mut components = Vec::new();
        for (key, info) in &manifest.components {
            // 알 수 없는 키(installer 등)는 건너뛰기
            let Some(component) = Component::try_from_manifest_key(key) else {
                tracing::debug!("[Updater] Skipping unknown manifest key: {}", key);
                continue;
            };
            let current = local_versions.get(key).cloned().unwrap_or_default();
            let installed = self.is_component_installed(&component);

            let update_available = if installed && current.is_empty() {
                tracing::warn!(
                    "[Updater] Cannot detect local version for installed component '{}' — \
                     assuming up-to-date to avoid false notification.",
                    key
                );
                let _ = Self::update_installed_version(key, &info.version);
                false
            } else {
                Self::is_newer_version(&info.version, &current)
            };

            let resolved = self.resolved_components.get(key);
            components.push(ComponentVersion {
                component,
                current_version: current,
                latest_version: Some(info.version.clone()),
                update_available,
                download_url: resolved.map(|rc| rc.download_url.clone()),
                asset_name: resolved.map(|rc| rc.asset_name.clone()),
                release_notes: self.cached_release_notes.clone(),
                published_at: self.cached_published_at.clone(),
                downloaded: false,
                downloaded_path: None,
                installed,
                sha256: info.sha256.clone(),
            });
        }

        let now = chrono::Utc::now();
        let next = now + chrono::Duration::hours(i64::from(self.config.check_interval_hours));

        self.status = UpdateStatus {
            last_check: Some(now.to_rfc3339_opts(chrono::SecondsFormat::Secs, true)),
            next_check: Some(next.to_rfc3339_opts(chrono::SecondsFormat::Secs, true)),
            components,
            checking: false,
            error: None,
        };

        Ok(self.status.clone())
    }

    /// `latest` 가 `current` 보다 새 버전인지 (로컬 버전을 모르면 업데이트 필요로 판단)
    fn is_newer_version(latest: &str, current: &str) -> bool {
        match (SemVer::parse(latest), SemVer::parse(current)) {
            (Some(l), Some(c)) => l.is_newer_than(&c),
            (Some(_), None) => true,
            _ => false,
        }
    }
}
