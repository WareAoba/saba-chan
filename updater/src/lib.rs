//! # saba-chan 업데이터 라이브러리
//!
//! 코어 데몬, CLI, GUI, 모듈, 익스텐션 등 모든 컴포넌트를 업데이트합니다.
//! 릴리즈 매니페스트를 활용하여 컴포넌트를 관리합니다.
//!
//! ## 파일
//! - `lib.rs`: 컴포넌트 정의, `UpdateManager` 확인·다운로드·적용 흐름
//! - `state.rs`: 적용 대기 목록, 설치된 버전 기록
//! - `apply.rs`: 받은 파일을 설치 폴더에 적용
//! - `relay.rs`·`signature.rs`·`download.rs`: relay manifest 확인, 서명 검증, R2 다운로드
//!
//! 경로는 모두 `install_root` 기준입니다.


pub mod config_file;
pub mod constants;
pub mod download;
pub mod python_pin;
pub mod launch_ticket;
pub mod error;
pub mod integrity;
pub mod marker;
pub mod process;
pub mod relay;
pub mod signature;
pub mod uninstall_entry;
pub mod version;
pub mod versions;
mod apply;
mod state;
mod time;

#[cfg(test)]
mod tests;

// Re-exports for convenience
pub use error::{UpdaterError, RecoveryStrategy, NetworkChecker, ErrorContext};
pub use process::ProcessChecker;
pub use relay::{ReleaseManifest, ManifestComponent};
pub use integrity::{IntegrityReport, IntegrityStatus, OverallIntegrity, ComponentIntegrity};
pub use marker::UpdateCompletionMarker;

use anyhow::Result;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex as StdMutex};

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
            k if k.starts_with("module-") => {
                Some(Component::Module(k.strip_prefix("module-").unwrap().to_string()))
            }
            k if k.starts_with("ext-") => {
                Some(Component::Extension(k.strip_prefix("ext-").unwrap().to_string()))
            }
            _ => None,
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
    /// 받은 파일의 sha256 (manifest 값). 적용 직전에 다시 대조한다 (C13)
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub staged_sha256: Option<String>,
    /// 해당 컴포넌트가 설치되어 있는지 여부 (false면 미설치 상태)
    pub installed: bool,
}

/// 전체 업데이트/설치 상태 정보
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct UpdateStatus {
    pub last_check: Option<String>,
    pub next_check: Option<String>,
    pub components: Vec<ComponentVersion>,
    pub checking: bool,
    pub error: Option<String>,
}

/// 버전 의존성 확인 결과
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DependencyCheck {
    /// 확인 대상 컴포넌트 키
    pub component: String,
    /// 모든 의존성이 충족되었는지 여부
    pub satisfied: bool,
    /// 충족되지 않은 의존성 목록
    pub issues: Vec<DependencyIssue>,
}

/// 충족되지 않은 개별 의존성 정보
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DependencyIssue {
    /// 필요한 컴포넌트 키 (예: "saba-core")
    pub required_component: String,
    /// 필요한 최소 버전 (예: ">=0.3.0")
    pub required_version: String,
    /// 현재 설치된 버전 (None이면 미설치)
    pub installed_version: Option<String>,
    /// 사람이 읽을 수 있는 메시지
    pub message: String,
}

// ══════════════════════════════════════════════════════
// 업데이트 적용 관련 구조체 정의 (2-flow 아키텍처)
// ══════════════════════════════════════════════════════

/// 개별 컴포넌트 적용 결과
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ApplyComponentResult {
    /// 컴포넌트 manifest 키 (예: "module-minecraft", "saba-core")
    pub component: String,
    /// 적용 성공 여부
    pub success: bool,
    /// 결과 메시지
    pub message: String,
    /// 업데이트 적용을 위해 중단한 프로세스 목록 (데몬 IPC 경유 적용 시)
    pub stopped_processes: Vec<String>,
    /// 재시작 필요 여부
    pub restart_needed: bool,
}

/// 전체 업데이트 적용 결과
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ApplyResult {
    /// 개별 컴포넌트 적용 결과
    pub results: Vec<ApplyComponentResult>,
    /// 적용 후 재시작이 필요한 컴포넌트 목록 (CoreDaemon 업데이트 시 필수)
    pub daemon_restart_script: Option<String>,
    /// GUI/CLI 자신의 업데이트가 포함 — 별도 self-update flow가 필요 (self-update flow)
    pub self_update_components: Vec<String>,
}

/// GUI/CLI 자신의 업데이트 정보 (업데이터 실행파일이 컴포넌트를 교체한 후 재시작하는 프로세스)
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SelfUpdateInfo {
    /// 업데이터 실행파일 경로
    pub updater_executable: String,
    /// 업데이터 실행파일에 전달할 커맨드라인 인자
    pub args: Vec<String>,
    /// 대상 컴포넌트
    pub component: String,
    /// 스테이징 파일 경로
    pub staged_path: Option<String>,
}

/// 업데이트 설정
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct UpdateConfig {
    pub enabled: bool,
    /// 확인 주기 (시간 단위, 기본값 3, 최소 1시간에서 최대 8시간)
    pub check_interval_hours: u32,
    /// 확인 뒤 새 버전을 미리 내려받아 둘지 (적용은 언제나 사용자가 고른다)
    pub auto_download: bool,
    /// 스테이징 디렉터리 (다운로드와 임시 파일 저장, 기본값: 실행파일 경로 기준)
    pub install_root: Option<String>,
    /// 릴레이 주소 (None이면 relay::DEFAULT_RELAY_URL). 모의 릴레이 시험은 여기에
    /// `scripts/mock-release-server.js` 주소를 넣는다
    pub relay_url: Option<String>,
    /// 릴레이 업데이트 채널 (stable | beta | nightly)
    pub channel: String,
}

impl Default for UpdateConfig {
    fn default() -> Self {
        Self {
            enabled: true,
            check_interval_hours: 3,
            auto_download: false,
            install_root: None,
            relay_url: None,
            channel: relay::DEFAULT_CHANNEL.to_string(),
        }
    }
}

/// 익스텐션 manifest.json의 (id, version)
fn read_extension_version(manifest: &Path) -> Option<(String, String)> {
    let v: serde_json::Value = serde_json::from_str(&std::fs::read_to_string(manifest).ok()?).ok()?;
    Some((v.get("id")?.as_str()?.to_string(), v.get("version")?.as_str()?.to_string()))
}

/// GUI 실행 파일 이름들: 표준 이름과, 0.1.1 GUI zip처럼 Electron 폴더를 그대로 담은 경우의 productName
pub const GUI_EXE_NAMES: &[&str] = if cfg!(windows) { &["saba-chan-gui.exe", "Saba-chan.exe"] } else { &["saba-chan-gui"] };

/// 설치 폴더 바로 안의 GUI 실행 파일
pub fn find_gui_exe(install_root: &Path) -> Option<PathBuf> {
    GUI_EXE_NAMES.iter().map(|n| install_root.join(n)).find(|p| p.is_file())
}

/// 같은 파일을 가리키는 경로인지. 구성 요소로 비교해 `./`는 무시하고,
/// Windows 파일 이름은 대소문자를 가리지 않는다
pub fn same_path(a: &Path, b: &Path) -> bool {
    let parts = |p: &Path| -> Vec<String> {
        p.components()
            .filter(|c| !matches!(c, std::path::Component::CurDir))
            .map(|c| {
                let s = c.as_os_str().to_string_lossy();
                if cfg!(windows) { s.to_lowercase() } else { s.into_owned() }
            })
            .collect()
    };
    parts(a) == parts(b)
}

/// zip을 `target_dir`에 푼다. 모든 압축 해제는 이 함수를 거친다.
///
/// 엔트리 경로는 `enclosed_name()`으로 검사해 `..`나 절대 경로로
/// `target_dir` 밖에 쓰는 엔트리(zip-slip)를 거부한다. `before_write`는
/// 파일을 쓰기 직전에 호출된다(실행 중 바이너리를 비켜 두는 데 쓴다).
/// 쓴 파일 경로를 돌려준다(무결성 기록용).
pub fn extract_zip(
    staged: &Path,
    target_dir: &Path,
    mut before_write: impl FnMut(&Path) -> Result<()>,
) -> Result<Vec<PathBuf>> {
    let mut written = Vec::new();
    let file = std::fs::File::open(staged)?;
    let mut archive = zip::ZipArchive::new(file)?;
    for i in 0..archive.len() {
        let mut entry = archive.by_index(i)?;
        let Some(rel) = entry.enclosed_name() else {
            anyhow::bail!("Unsafe path in archive {}: {}", staged.display(), entry.name());
        };
        let out_path = target_dir.join(rel);
        if entry.is_dir() {
            std::fs::create_dir_all(&out_path)?;
            continue;
        }
        if let Some(parent) = out_path.parent() {
            std::fs::create_dir_all(parent)?;
        }
        before_write(&out_path)?;
        let mut outfile = std::fs::File::create(&out_path)?;
        std::io::copy(&mut entry, &mut outfile)?;
        written.push(out_path);
    }
    Ok(written)
}

// ══════════════════════════════════════════════════════
// UpdateManager
// ══════════════════════════════════════════════════════

/// 업데이트 확인/다운로드 및 적용을 관리하는 업데이트/초기 설치 매니저
pub struct UpdateManager {
    pub config: UpdateConfig,
    /// 설정 저장용 레퍼런스
    status: UpdateStatus,
    /// 모듈 디렉터리 경로 (%APPDATA%/saba-chan/modules)
    modules_dir: PathBuf,
    /// 익스텐션 디렉터리 경로 (%APPDATA%/saba-chan/extensions)
    extensions_dir: PathBuf,
    /// 업데이트 다운로드 파일 저장 디렉터리
    staging_dir: PathBuf,
    /// 설치 루트 디렉터리 (다운로드/적용 기준)
    install_root: PathBuf,
    /// 마지막으로 받은 서명 검증된 manifest. 무결성·의존성 검사와 다운로드가 같이 쓴다
    cached_manifest: Option<ReleaseManifest>,
    /// 다운로드 진행 상태 (Arc로 공유 — Manager 잠금 없이 폴링 가능)
    pub download_progress: Arc<StdMutex<DownloadProgress>>,
}

impl UpdateManager {
    pub fn new(config: UpdateConfig, modules_dir: &str) -> Self {
        // staging 디렉터리: %APPDATA%/saba-chan/updates/ 또는 ./updates/
        let staging_dir = Self::resolve_staging_dir();

        // install_root: config 경로 또는 실행 파일 기준
        let install_root = config.install_root.as_ref()
            .map(PathBuf::from)
            .unwrap_or_else(|| {
                std::env::current_exe()
                    .ok()
                    .and_then(|p| p.parent().map(|d| d.to_path_buf()))
                    .unwrap_or_else(|| PathBuf::from("."))
            });

        // extensions_dir: %APPDATA%/saba-chan/extensions 고정 경로
        let extensions_dir = Self::resolve_extensions_dir();

        let modules_dir_path = PathBuf::from(modules_dir);
        if !modules_dir_path.exists() {
            let _ = std::fs::create_dir_all(&modules_dir_path);
        }
        if !extensions_dir.exists() {
            let _ = std::fs::create_dir_all(&extensions_dir);
        }

        let mgr = Self {
            config,
            status: UpdateStatus {
                last_check: None,
                next_check: None,
                components: Vec::new(),
                checking: false,
                error: None,
            },
            modules_dir: PathBuf::from(modules_dir),
            extensions_dir,
            staging_dir,
            install_root,
            cached_manifest: None,
            download_progress: Arc::new(StdMutex::new(DownloadProgress::default())),
        };
        // 지난 업데이트가 남긴 .exe.old: 적용 직후엔 옛 프로세스가 쥐고 있어 못 지운 것을 다음 실행 때 지운다
        mgr.cleanup_old_files();
        mgr
    }

    fn resolve_staging_dir() -> PathBuf {
        crate::constants::resolve_staging_dir()
    }

    /// 익스텐션 디렉터리 — constants 모듈에 위임
    fn resolve_extensions_dir() -> PathBuf {
        crate::constants::resolve_extensions_dir()
    }

    /// 익스텐션 ID에서 디스크 상의 디렉토리 경로를 해석합니다.
    /// Python import 호환을 위해 하이픈→언더스코어 변환도 시도합니다.
    fn resolve_ext_dir(&self, ext_name: &str) -> PathBuf {
        // 1. 원본 이름
        let exact = self.extensions_dir.join(ext_name);
        if exact.exists() {
            return exact;
        }
        // 2. Python-safe 이름 (하이픈→언더스코어)
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

    /// 현재 설정 반환
    pub fn get_config(&self) -> UpdateConfig {
        self.config.clone()
    }

    /// 설정 업데이트
    pub fn update_config(&mut self, new_config: UpdateConfig) {
        // install_root override: config에 install_root가 설정되면 실제 경로 갱신
        if let Some(ref root) = new_config.install_root {
            let new_root = std::path::PathBuf::from(root);
            if new_root != self.install_root {
                tracing::info!("[UpdateManager] install_root updated: {:?} -> {:?}", self.install_root, new_root);
                self.install_root = new_root;
            }
        }
        self.config = new_config;
    }

    // ─── 무결성 검증 ────────────────────────────────────────────────────────

    /// 업데이터가 설치하며 기록한 파일 해시와 지금 파일을 비교한다 (네트워크 없음).
    /// 릴리즈 manifest의 sha256은 zip의 해시라 설치된 파일과 직접 비교할 수 없다
    pub fn verify_integrity(&self) -> integrity::IntegrityReport {
        integrity::verify_installed()
    }

    fn relay_client(&self) -> relay::RelayClient {
        relay::RelayClient::new(self.config.relay_url.as_deref().unwrap_or(relay::DEFAULT_RELAY_URL))
    }

    // ─── 업데이트 확인 ────────────────────────────────────────────────────────

    /// 릴레이의 서명된 manifest 하나로 모든 컴포넌트의 최신 버전과 R2 URL을 얻는다.
    pub async fn check_for_updates(&mut self) -> Result<UpdateStatus> {
        self.status.checking = true;
        self.status.error = None;

        let local_versions = self.collect_local_versions();
        let client = self.relay_client();
        let platform = relay::current_platform();
        let current = local_versions.get("saba-core").cloned().unwrap_or_default();

        let release = match client
            .fetch_verified_release(&self.config.channel, &platform, &current, &relay::anon_client_id())
            .await
        {
            Ok(r) => r,
            Err(e) => {
                tracing::error!("[Updater] Relay check failed: {}", e);
                self.status.checking = false;
                self.status.error = Some(format!("Relay check failed: {}", e));
                return Err(e.into());
            }
        };

        let mut components = Vec::new();
        if let Some(release) = release {
            let notes = release.latest.notes.as_ref().and_then(|n| n.get("ko").or_else(|| n.get("en")).cloned());
            for (key, info) in &release.manifest.components {
                let Some(component) = Component::try_from_manifest_key(key) else {
                    tracing::debug!("[Updater] Skipping unknown manifest key: {}", key);
                    continue;
                };
                let current = local_versions.get(key).cloned().unwrap_or_default();
                let installed = self.is_component_installed(&component);
                // 모듈·익스텐션은 설치한 것만 업데이트한다. 새로 받는 건 목록 화면의 몫
                if !installed && matches!(component, Component::Module(_) | Component::Extension(_)) {
                    continue;
                }
                // 설치돼 있는데 로컬 버전을 모르면 알림 대신 최신으로 기록
                let update_available = if installed && current.is_empty() {
                    let _ = Self::update_installed_version(key, &info.version);
                    false
                } else {
                    self.compare_versions(&info.version, &current)
                };
                components.push(ComponentVersion {
                    component,
                    update_available,
                    current_version: current,
                    latest_version: Some(info.version.clone()),
                    download_url: Some(info.url.clone()),
                    asset_name: Some(info.filename.clone()),
                    release_notes: notes.clone(),
                    published_at: release.latest.released_at.clone(),
                    downloaded: false,
                    downloaded_path: None,
                    staged_sha256: None,
                    installed,
                });
            }
            // 무결성 검사·의존성 검사·다운로드가 같은 manifest를 보게 한다
            self.cached_manifest = Some(release.manifest);
        }

        let now = time::now_iso();
        let next = time::iso_after_hours(self.config.check_interval_hours);
        self.status = UpdateStatus {
            last_check: Some(now),
            next_check: Some(next),
            components,
            checking: false,
            error: None,
        };
        Ok(self.status.clone())
    }

    fn compare_versions(&self, latest: &str, current: &str) -> bool {
        let latest_ver = SemVer::parse(latest);
        let current_ver = SemVer::parse(current);
        match (&latest_ver, &current_ver) {
            (Some(l), Some(c)) => l.is_newer_than(c),
            (Some(_), None) => true, // 로컬 버전 정보가 없으면 업데이트 필요
            _ => false,
        }
    }


    // ─────── 로컬 버전 수집 ────────────────────────────────────────────────────────────────────────

    /// 모든 컴포넌트의 현재 설치된 버전을 수집
    fn collect_local_versions(&self) -> HashMap<String, String> {
        // 1. 설치 매니페스트 우선 로드 (가장 신뢰할 수 있는 소스)
        let mut versions = Self::load_installed_manifest();

        // 2. 매니페스트에 없는 앱 컴포넌트는 빌드에 묶인 versions.toml 버전 (폴백)
        for (key, version) in &crate::versions::bundled().components {
            versions.entry(key.clone()).or_insert_with(|| version.clone());
        }

        // 모듈: modules/*/module.toml에서 감지
        if let Ok(entries) = std::fs::read_dir(&self.modules_dir) {
            for entry in entries.flatten() {
                let path = entry.path();
                if path.is_dir() {
                    let module_toml = path.join("module.toml");
                    if let Some((name, version)) = self.read_module_version(&module_toml) {
                        let key = format!("module-{}", name);
                        versions.entry(key).or_insert(version);
                    }
                }
            }
        }

        // 익스텐션: extensions/*/manifest.json의 id·version
        let extensions_dir = &self.extensions_dir;
        if let Ok(entries) = std::fs::read_dir(extensions_dir) {
            for entry in entries.flatten() {
                let path = entry.path();
                if path.is_dir() {
                    if let Some((name, version)) = read_extension_version(&path.join("manifest.json")) {
                        let key = format!("ext-{}", name);
                        versions.entry(key).or_insert(version);
                    }
                }
            }
        }

        tracing::debug!("[UpdateManager] Local versions: {:?}", versions);
        versions
    }

    fn read_module_version(&self, module_toml: &Path) -> Option<(String, String)> {
        let content = std::fs::read_to_string(module_toml).ok()?;
        let mut name = None;
        let mut version = None;

        for line in content.lines() {
            let trimmed = line.trim();
            if trimmed.starts_with("name") && trimmed.contains('=') {
                let val = trimmed.split('=').nth(1)?.trim().trim_matches('"');
                name = Some(val.to_string());
            }
            if trimmed.starts_with("version") && trimmed.contains('=') {
                let val = trimmed.split('=').nth(1)?.trim().trim_matches('"');
                version = Some(val.to_string());
            }
            if name.is_some() && version.is_some() {
                break;
            }
        }

        match (name, version) {
            (Some(n), Some(v)) => Some((n, v)),
            _ => None,
        }
    }

    // ─────── 다운로드 ────────────────────────────────────────────────────────────────────────

    /// 업데이트 가능한 모든 컴포넌트를 staging 디렉터리로 다운로드
    pub async fn download_available_updates(&mut self) -> Result<Vec<String>> {
        std::fs::create_dir_all(&self.staging_dir)?;

        let mut downloaded = Vec::new();

        // 업데이트 가능하고 아직 다운로드하지 않은 컴포넌트 목록
        let to_download: Vec<(String, ManifestComponent)> = self.status.components.iter()
            .filter(|c| c.update_available && !c.downloaded)
            .filter_map(|c| {
                let key = c.component.manifest_key();
                self.manifest_entry(&key).map(|rc| (key, rc.clone()))
            })
            .collect();

        let client = reqwest::Client::new();
        for (key, rc) in &to_download {
            let dest = self.staging_dir.join(&rc.filename);
            tracing::info!("[Updater] Downloading {}", key);
            download::download_verified(&client, &rc.url, &dest, Some(&rc.sha256), None).await?;
            tracing::info!("[Updater] Downloaded {}", rc.filename);
            downloaded.push(rc.filename.clone());
        }

        // 상태 업데이트: downloaded 플래그 설정
        for comp in &mut self.status.components {
            if let Some(ref name) = comp.asset_name {
                if downloaded.contains(name) {
                    comp.downloaded = true;
                    comp.downloaded_path = Some(
                        self.staging_dir.join(name).to_string_lossy().to_string()
                    );
                    comp.staged_sha256 = to_download.iter()
                        .find(|(_, rc)| &rc.filename == name)
                        .map(|(_, rc)| rc.sha256.clone());
                }
            }
        }

        Ok(downloaded)
    }

    /// 특정 컴포넌트만 다운로드
    pub async fn download_component(&mut self, component: &Component) -> Result<String> {
        std::fs::create_dir_all(&self.staging_dir)?;

        let comp_status = self.status.components.iter()
            .find(|c| &c.component == component)
            .ok_or_else(|| anyhow::anyhow!("Component {:?} not found in status", component))?;

        if !comp_status.update_available {
            anyhow::bail!("No update available for {}", component.display_name());
        }

        let key = component.manifest_key();
        let rc = self.manifest_entry(&key).cloned()
            .ok_or_else(|| anyhow::anyhow!("{} is not in the release manifest", component.display_name()))?;

        let dest = self.staging_dir.join(&rc.filename);

        tracing::info!("[Updater] Downloading {} v{}", key, rc.version);

        // 진행률 초기화
        {
            let mut prog = self.download_progress.lock().unwrap();
            prog.component = Some(key.clone());
            prog.bytes_received = 0;
            prog.total_bytes = 0;
            prog.active = true;
        }

        // 스트리밍 다운로드 + SHA-256 검증 (불일치면 파일 삭제)
        let client = reqwest::Client::new();
        let result = download::download_verified(
            &client,
            &rc.url,
            &dest,
            Some(&rc.sha256),
            Some(&self.download_progress),
        )
        .await;
        if let Err(e) = result {
            if let Ok(mut prog) = self.download_progress.lock() {
                prog.active = false;
            }
            anyhow::bail!("Failed to download {}: {}", rc.filename, e);
        }

        // 진행률 완료
        {
            let mut prog = self.download_progress.lock().unwrap();
            prog.active = false;
        }

        let asset_name = rc.filename;
        let sha256 = Some(rc.sha256);

        // 상태 업데이트
        if let Some(comp) = self.status.components.iter_mut().find(|c| &c.component == component) {
            comp.downloaded = true;
            comp.downloaded_path = Some(dest.to_string_lossy().to_string());
            comp.staged_sha256 = sha256;
        }

        Ok(asset_name)
    }
    fn manifest_entry(&self, key: &str) -> Option<&ManifestComponent> {
        self.cached_manifest.as_ref()?.components.get(key)
    }

    // ─────── 적용 ────────────────────────────────────────────────────────────────────────

    /// 다운로드 완료된 업데이트를 적용
    ///
    /// ## 주요 동작
    /// - **모듈**: 기존 파일을 백업하고 다운로드된 zip 압축 해제
    /// - **GUI/CLI**: 직접 교체 (별도 프로세스 실행으로 처리)
    /// - **코어 데몬**: 실행 중이면 교체가 불가하므로 재시작 후 업데이트를 예약
    pub async fn apply_updates(&mut self) -> Result<Vec<String>> {
        let all_keys: Vec<String> = self.status.components.iter()
            .filter(|c| c.downloaded && c.update_available)
            .map(|c| c.component.manifest_key())
            .collect();
        self.apply_components(&all_keys).await
    }

    /// 지정한 컴포넌트만 적용 (빈 목록이면 전체 적용)
    ///
    /// ## 적용 순서
    /// 1. **Updater** — 업데이터 자체를 먼저 교체 (데몬이 수행)
    /// 2. **비-인터페이스**: 사용하지 않는 인터페이스(GUI 사용 중이면 CLI),
    ///    CoreDaemon, 모듈, 익스텐션, Locales를 비동기 적용
    /// 3. **현재 인터페이스**: 마지막에 GUI 또는 CLI를 적용 (재시작 필요)
    ///
    /// 이 순서를 지키면 업데이트 도중 프로세스 충돌이 방지됩니다.
    pub async fn apply_components(&mut self, keys: &[String]) -> Result<Vec<String>> {
        let mut applied = Vec::new();

        let mut components: Vec<ComponentVersion> = self.status.components.iter()
            .filter(|c| c.downloaded && c.update_available)
            .filter(|c| keys.is_empty() || keys.contains(&c.component.manifest_key()))
            .cloned()
            .collect();

        // 적용 우선순위에 따라 정렬
        components.sort_by_key(|c| Self::component_apply_priority(&c.component));

        for comp in &components {
            let staged_path = Self::verified_staged_path(comp)?;
            let files = self.apply_one(&comp.component, staged_path).await?;
            Self::record_files(&comp.component, &files);
            applied.push(comp.component.display_name());
        }

        // 적용 완료된 컴포넌트의 상태 업데이트
        for comp in &mut self.status.components {
            if applied.contains(&comp.component.display_name()) {
                comp.update_available = false;
                comp.downloaded = false;
                comp.downloaded_path = None;
                if let Some(ref latest) = comp.latest_version {
                    comp.current_version = latest.clone();
                }
            }
        }

        // 적용 성공한 컴포넌트들의 버전을 로컬 매니페스트에 기록
        if !applied.is_empty() {
            if let Err(e) = self.update_installed_versions_batch(&applied) {
                tracing::warn!("[UpdateManager] Failed to update installed manifest: {}", e);
            }
        }

        // 코어 컴포넌트가 업데이트된 경우, 레지스트리의 DisplayVersion을 코어 버전으로 갱신
        if let Some(core_comp) = self.status.components.iter()
            .find(|c| matches!(c.component, Component::CoreDaemon))
        {
            let core_version = &core_comp.current_version;
            if let Err(e) = self.update_registry_display_version(core_version) {
                tracing::warn!("[UpdateManager] Failed to update registry DisplayVersion: {}", e);
            }
        }

        // 적용 완료 후 .old 백업 파일 정리
        if !applied.is_empty() {
            self.cleanup_old_files();
        }

        Ok(applied)
    }

    /// 컴포넌트 하나를 적용하고 쓴 파일을 돌려준다
    async fn apply_one(&self, component: &Component, staged_path: &str) -> Result<Vec<PathBuf>> {
        match component {
            Component::Module(name) => self.apply_module_update(name, staged_path).await,
            Component::Extension(name) => self.apply_extension_update(name, staged_path).await,
            Component::Cli => self.apply_binary_update("saba-cli", staged_path).await,
            // 업데이터 자신(Windows는 실행 중인 exe의 이름을 바꿀 수 있다)과 데몬은 같은 방식
            Component::Updater => self.apply_binary_update("saba-chan-updater", staged_path).await,
            Component::CoreDaemon => self.apply_binary_update("saba-core", staged_path).await,
            Component::Gui => self.apply_gui_update(staged_path).await,
            Component::Locales => {
                let target = self.install_root.join("locales");
                self.extract_to_directory(Path::new(staged_path), &target).await
            }
        }
    }

    /// 무결성 검사용으로 설치한 파일의 해시를 적는다. 실패해도 적용은 성공이다
    fn record_files(component: &Component, files: &[PathBuf]) {
        if files.is_empty() {
            return;
        }
        // 업데이터는 서명된 카탈로그의 것만 적용한다
        if let Err(e) = integrity::record_install(&component.manifest_key(), files)
            .and_then(|_| integrity::set_official(&component.manifest_key(), true))
        {
            tracing::warn!("[Updater] Could not record installed files for {}: {}", component.display_name(), e);
        }
    }

    /// 적용할 받은 파일. 다운로드 뒤 바뀌었으면(sha256 불일치) 적용하지 않는다 (C13)
    fn verified_staged_path(comp: &ComponentVersion) -> Result<&String> {
        let path = comp.downloaded_path.as_ref()
            .ok_or_else(|| anyhow::anyhow!("No staged file for {:?}", comp.component))?;
        if let Some(expected) = &comp.staged_sha256 {
            let got = crate::integrity::compute_sha256(Path::new(path)).map_err(|e| anyhow::anyhow!(e))?;
            if !got.eq_ignore_ascii_case(expected) {
                anyhow::bail!("Staged file for {} changed since download (sha256 {} != {})", comp.component.display_name(), got, expected);
            }
        }
        Ok(path)
    }

    /// 컴포넌트의 적용 우선순위를 반환합니다.
    ///
    /// 낮은 값일수록 먼저 적용됩니다:
    /// - 0: Updater (업데이트 방식 변경 대응 — 디스크의 바이너리를 먼저 교체)
    /// - 1: 모듈, 익스텐션, Locales (프로세스 중단 불필요)
    /// - 3: CoreDaemon (데몬 프로세스 종료 후 교체)
    /// - 4: 사용하지 않는 인터페이스 (GUI 사용 중이면 CLI, 반대도 마찬가지)
    /// - 5: 현재 사용 중인 인터페이스 (재시작 필요)
    fn component_apply_priority(component: &Component) -> u8 {
        match component {
            Component::Updater => 0,
            Component::Module(_) | Component::Extension(_) | Component::Locales => 1,
            Component::CoreDaemon => 3,
            // GUI/CLI는 현재 실행 중인 인터페이스를 마지막에 적용
            Component::Gui => {
                if ProcessChecker::is_gui_running() { 5 } else { 4 }
            }
            Component::Cli => {
                if ProcessChecker::is_cli_running() { 5 } else { 4 }
            }
        }
    }

    /// 테스트 전용: 컴포넌트 적용 우선순위 조회
    #[cfg(test)]
    pub fn component_apply_priority_for_test(component: &Component) -> u8 {
        Self::component_apply_priority(component)
    }

    // ─────── 2-flow 아키텍처: 개별 컴포넌트 적용 ────────────────────────────────────────────────────────────────────────

    /// 업데이트가 남긴 `<사바쨩 실행 파일>.exe.old` 백업을 지운다 (설치 폴더 바로 안만).
    /// Windows는 실행 중인 exe를 이름만 바꿔 두므로, 아직 쥐고 있는 파일은 다음 번에 지운다.
    pub fn cleanup_old_files(&self) {
        let Ok(entries) = std::fs::read_dir(&self.install_root) else { return };
        for path in entries.flatten().map(|e| e.path()) {
            let Some(name) = path.file_name().and_then(|n| n.to_str()) else { continue };
            let Some(stem) = name.strip_suffix(".exe.old") else { continue };
            let ours = stem.starts_with("saba-") || GUI_EXE_NAMES.iter().any(|g| g.trim_end_matches(".exe") == stem);
            if !ours {
                continue;
            }
            match std::fs::remove_file(&path) {
                Ok(()) => tracing::info!("[Updater] Cleaned up old file: {}", path.display()),
                Err(e) => tracing::debug!("[Updater] Cannot remove old file {} (still in use?): {}", path.display(), e),
            }
        }
    }

    /// 컴포넌트의 버전 의존성을 확인합니다.
    /// 서버 매니페스트의 `requires` 필드를 기반으로 설치된 버전과 비교합니다.
    ///
    /// 예: GUI 0.3.0 → requires: { "saba-core": ">=0.3.0" }
    ///     → saba-core이 0.3.0 미만이면 DependencyIssue 반환
    pub fn check_dependencies(&self, component_key: &str) -> DependencyCheck {
        let installed = Self::load_installed_manifest();
        let mut issues = Vec::new();

        // 캐시된 서버 매니페스트에서 requires 정보 조회
        if let Some(ref manifest) = self.cached_manifest {
            if let Some(info) = manifest.components.get(component_key) {
                if let Some(ref requires) = info.requires {
                    for (dep_key, min_version_str) in requires {
                        let dep_version = installed.get(dep_key);
                        let satisfied = dep_version.is_some_and(|v| {
                            // ">=" 접두사 제거 후 SemVer 비교
                            let min_clean = min_version_str.trim_start_matches(">=").trim();
                            match (SemVer::parse(v), SemVer::parse(min_clean)) {
                                (Some(installed_v), Some(required_v)) => installed_v >= required_v,
                                _ => false,
                            }
                        });

                        if !satisfied {
                            issues.push(DependencyIssue {
                                required_component: dep_key.clone(),
                                required_version: min_version_str.clone(),
                                installed_version: dep_version.cloned(),
                                message: format!(
                                    "{} requires {} {} but {} is installed",
                                    component_key, dep_key, min_version_str,
                                    dep_version.map_or("not installed".to_string(), |v| v.clone())
                                ),
                            });
                        }
                    }
                }
            }
        }

        DependencyCheck {
            component: component_key.to_string(),
            satisfied: issues.is_empty(),
            issues,
        }
    }

    /// 모든 업데이트 가능한 컴포넌트의 의존성을 일괄 확인합니다.
    pub fn check_all_dependencies(&self) -> Vec<DependencyCheck> {
        self.status.components.iter()
            .filter(|c| c.update_available)
            .map(|c| self.check_dependencies(&c.component.manifest_key()))
            .collect()
    }


    /// 단일 컴포넌트만 개별적으로 적용하는 메서드 (데몬 IPC 경유 시 개별 컴포넌트를 순차적으로 처리)
    ///
    /// Flow 1 (백그라운드 워커): IPC 커맨드를 통해 데몬이 직접 적용한 후 재시작
    /// Flow 2 (GUI/CLI): 직접 적용, self-update flow로 전환
    pub async fn apply_single_component(&mut self, component: &Component) -> Result<ApplyComponentResult> {
        let comp = self.status.components.iter()
            .find(|c| &c.component == component && c.downloaded && c.update_available)
            .cloned()
            .ok_or_else(|| anyhow::anyhow!("Component {:?} is not ready for apply", component))?;

        let staged_path = Self::verified_staged_path(&comp)?;
        let files = self.apply_one(component, staged_path).await?;
        Self::record_files(component, &files);

        let message = match component {
            Component::Module(name) => format!("Module '{}' updated", name),
            Component::Extension(name) => format!("Extension '{}' updated", name),
            Component::CoreDaemon => "Saba-Core updated (restart required)".to_string(),
            other => format!("{} updated", other.display_name()),
        };
        let result = ApplyComponentResult {
            component: component.manifest_key(),
            success: true,
            message,
            stopped_processes: Vec::new(), // IPC 커맨드 경유 시 해당 없음
            // 데몬은 새 프로세스로, GUI는 업데이터가 다시 띄운다
            restart_needed: matches!(component, Component::CoreDaemon | Component::Gui),
        };

        // 적용 성공 시 상태 업데이트
        self.mark_component_applied(component);

        // 로컬 매니페스트에 버전 기록
        if result.success {
            if let Some(comp_state) = self.status.components.iter()
                .find(|c| &c.component == component)
            {
                if let Err(e) = Self::update_installed_version(
                    &component.manifest_key(),
                    &comp_state.current_version,
                ) {
                    tracing::warn!("[UpdateManager] Failed to update installed manifest for {}: {}", component.manifest_key(), e);
                }
            }

            // 코어 컴포넌트가 업데이트된 경우, 레지스트리의 DisplayVersion도 갱신
            if matches!(component, Component::CoreDaemon) {
                if let Some(core_comp) = self.status.components.iter()
                    .find(|c| matches!(c.component, Component::CoreDaemon))
                {
                    if let Err(e) = self.update_registry_display_version(&core_comp.current_version) {
                        tracing::warn!("[UpdateManager] Failed to update registry DisplayVersion: {}", e);
                    }
                }
            }
        }

        Ok(result)
    }

    /// 컴포넌트의 적용 완료 상태를 표시
    pub fn mark_component_applied(&mut self, component: &Component) {
        for comp in &mut self.status.components {
            if &comp.component == component {
                comp.update_available = false;
                comp.downloaded = false;
                comp.downloaded_path = None;
                if let Some(ref latest) = comp.latest_version {
                    comp.current_version = latest.clone();
                }
            }
        }
    }

    /// GUI/CLI 자신의 업데이트 정보를 반환 (업데이터 실행파일을 통해 컴포넌트를 교체한 후 재시작하는 self-update 지원)
    pub fn get_self_update_info(&self, component: &Component) -> Result<SelfUpdateInfo> {
        let comp = self.status.components.iter()
            .find(|c| &c.component == component && c.downloaded)
            .ok_or_else(|| anyhow::anyhow!("Component {:?} not downloaded", component))?;

        // 업데이터 CLI 실행파일을 install_root/updater/cli/ 또는 근처 디렉터리에서 탐색
        let updater_exe = self.find_updater_executable()?;

        let staged_path = comp.downloaded_path.clone();
        let component_key = component.manifest_key();

        Ok(SelfUpdateInfo {
            updater_executable: updater_exe,
            args: vec![
                "apply".to_string(),
                "--component".to_string(),
                component_key.clone(),
                "--install-root".to_string(),
                self.install_root.display().to_string(),
            ],
            component: component_key,
            staged_path,
        })
    }

    /// 업데이터 CLI 실행파일의 경로를 탐색
    fn find_updater_executable(&self) -> Result<String> {
        // 배포 환경: install_root/saba-chan-updater(.exe)
        let candidates = if cfg!(target_os = "windows") {
            vec![
                self.install_root.join("saba-chan-updater.exe"),
            ]
        } else {
            vec![
                self.install_root.join("saba-chan-updater"),
            ]
        };

        for candidate in &candidates {
            if candidate.exists() {
                return Ok(candidate.display().to_string());
            }
        }

        // 개발 환경: target/release 또는 target/debug
        let dev_candidates = if cfg!(target_os = "windows") {
            vec![
                PathBuf::from("updater/gui/src-tauri/target/release/saba-chan-updater.exe"),
                PathBuf::from("updater/gui/src-tauri/target/debug/saba-chan-updater.exe"),
                PathBuf::from("target/release/saba-chan-updater.exe"),
                PathBuf::from("target/debug/saba-chan-updater.exe"),
            ]
        } else {
            vec![
                PathBuf::from("updater/gui/src-tauri/target/release/saba-chan-updater"),
                PathBuf::from("updater/gui/src-tauri/target/debug/saba-chan-updater"),
                PathBuf::from("target/release/saba-chan-updater"),
                PathBuf::from("target/debug/saba-chan-updater"),
            ]
        };

        for candidate in &dev_candidates {
            if candidate.exists() {
                return Ok(candidate.canonicalize()?.display().to_string());
            }
        }

        // 찾지 못하면 기본값을 반환 (배포 환경에서는 첫 후보 사용)
        Ok(candidates[0].display().to_string())
    }

    // ─────── 테스트 헬퍼 (pub) ────────────────────────────────────────────────────────────────────────

    /// 테스트 전용: extract_to_directory를 외부에서 호출
    #[doc(hidden)]
    pub async fn extract_to_directory_for_test(&self, staged: &Path, target: &Path) {
        self.extract_to_directory(staged, target).await.unwrap();
    }

    /// 테스트 전용: resolve_install_dir를 외부에서 호출
    #[doc(hidden)]
    pub fn resolve_install_dir_for_test(&self, component: &Component, manifest_dir: Option<&str>) -> PathBuf {
        self.resolve_install_dir(component, manifest_dir)
    }
}
