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
//! - `tls.rs`: 모든 HTTPS·WSS 클라이언트의 rustls 설정 (ring + OS 인증서 저장소)
//! - `window_icon.rs`: Tauri 창(GUI·업데이터·인스톨러)의 Windows 아이콘
//!
//! 경로는 모두 `install_root` 기준입니다.


pub mod app_bundle;
pub mod component_dirs;
pub mod config_file;
pub mod constants;
pub mod download;
pub mod python_pin;
pub mod launch_ticket;
pub mod live_apply;
pub mod integrity;
pub mod marker;
pub mod module_support;
pub mod notify_app_id;
pub mod process;
pub mod profile;
pub mod relay;
pub mod signature;
pub mod tls;
pub mod uninstall_entry;
pub mod version;
pub mod versions;
pub mod window_backdrop;
pub mod window_icon;
mod apply;
pub mod apply_session;
pub mod autostart;
mod state;
mod time;

#[cfg(test)]
mod tests;

// Re-exports for convenience
pub use process::ProcessChecker;
pub use relay::{ReleaseManifest, ManifestComponent};
pub use integrity::{IntegrityReport, IntegrityStatus, OverallIntegrity, ComponentIntegrity};
pub use marker::UpdateCompletionMarker;
pub(crate) use apply::swap_dir_untouched;
#[cfg(test)]
pub(crate) use apply::replace_dir;
pub use apply::{carry_swap_dot_entries, Replacing, SWAP_FINGERPRINT};

use anyhow::Result;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex as StdMutex};

use version::SemVer;

/// 같은 폴더의 임시 파일에 쓰고 이름을 바꿔 `path`를 통째로 바꾼다. 쓰다 끊겨도 반쯤 쓰인 파일이 남지 않는다
pub(crate) fn write_atomic(path: &Path, contents: &[u8]) -> std::io::Result<()> {
    saba_atomic_file::write(path, contents, saba_atomic_file::Mode::Plain)
}

/// 고쳐 쓰려고 기록 파일을 읽는다. 없으면 `None`. 내용이 깨졌으면(UTF-8이 아닌 것 포함) `<이름>.corrupt-<초>`로
/// 옮겨 두고 `None`: 그대로 두면 기록이 영영 안 되고, 새 값으로 덮어쓰면 다른 항목이 흔적 없이 사라진다.
/// 파일을 읽지 못하면(권한 등) 오류
pub(crate) fn read_json_for_update<T: serde::de::DeserializeOwned>(path: &Path) -> std::result::Result<Option<T>, String> {
    match read_json(path) {
        Ok(value) => Ok(value),
        Err(e) if path.is_file() && std::fs::read(path).is_ok() => {
            let secs = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_secs()).unwrap_or(0);
            let mut corrupt = path.as_os_str().to_os_string();
            corrupt.push(format!(".corrupt-{secs}"));
            let corrupt = PathBuf::from(corrupt);
            std::fs::rename(path, &corrupt).map_err(|e| e.to_string())?;
            tracing::warn!("{} was unreadable ({}); moved to {}", path.display(), e, corrupt.display());
            Ok(None)
        }
        Err(e) => Err(e),
    }
}

/// JSON 파일을 읽는다. 없으면 `None`. 읽거나 해석할 수 없으면 오류: 고쳐 쓰는 쪽이 덮어쓰지 않게
pub(crate) fn read_json<T: serde::de::DeserializeOwned>(path: &Path) -> std::result::Result<Option<T>, String> {
    match std::fs::read_to_string(path) {
        Ok(text) => serde_json::from_str(&text).map(Some).map_err(|e| format!("{}: {}", path.display(), e)),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(None),
        Err(e) => Err(format!("{}: {}", path.display(), e)),
    }
}

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
    /// 0.1.12 전의 업데이터(`saba-chan-updater`). 지금 릴리즈에는 없고, 옛 상태 파일·manifest를 읽을 때만 나온다
    Updater,
    /// 설치 폴더의 `saba-chan-installer.exe` (제거기로 쓰인다)
    Installer,
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
            Component::Installer => "installer".to_string(),
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
            "installer" => Some(Component::Installer),
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
            Component::Installer => "Installer".to_string(),
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
    /// manifest의 `requires` (예: `{"saba-core": ">=0.1.9"}`). pending.json에 실어 업데이터 `--apply`도 적용 전에 본다
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub requires: Option<HashMap<String, String>>,
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
    /// 같은 판 베타 규칙으로 기다리는 정식 판 `X.Y.Z` (정식 채널인데 지금 판이 그 판의 프리릴리스). 정식을 받으면 사라진다
    #[serde(default)]
    pub stable_wait: Option<String>,
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

/// 설치 기록의 모듈·익스텐션 항목을 디스크에서 찾은 것으로 바꾼다 (사라진 것은 빠진다)
fn merge_addons_from_disk(mut versions: HashMap<String, String>, disk: HashMap<String, String>) -> HashMap<String, String> {
    versions.retain(|key, _| !key.starts_with("module-") && !key.starts_with("ext-"));
    versions.extend(disk);
    versions
}

/// `requires`(`{"saba-core": ">=0.1.9"}`) 중 설치된 버전이 못 미치는 것. 업데이터 적용과 데몬의 카탈로그 설치가 같이 쓴다
pub fn unmet_requires(component_key: &str, requires: &HashMap<String, String>, installed: &HashMap<String, String>) -> Vec<DependencyIssue> {
    requires
        .iter()
        .filter_map(|(dep_key, min_version_str)| {
            let dep_version = installed.get(dep_key);
            let satisfied = dep_version.is_some_and(|v| {
                // ">=" 접두사 제거 후 SemVer 비교
                let min_clean = min_version_str.trim_start_matches(">=").trim();
                match (SemVer::parse(v), SemVer::parse(min_clean)) {
                    (Some(installed_v), Some(required_v)) => installed_v >= required_v,
                    _ => false,
                }
            });
            (!satisfied).then(|| DependencyIssue {
                required_component: dep_key.clone(),
                required_version: min_version_str.clone(),
                installed_version: dep_version.cloned(),
                message: format!(
                    "{} requires {} {} but {} is installed",
                    component_key, dep_key, min_version_str,
                    dep_version.map_or("not installed".to_string(), |v| v.clone())
                ),
            })
        })
        .collect()
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

/// 주기 확인 간격 (시간). 연결 여부와 상관없는 안전망이다
pub const CHECK_EVERY_HOURS: u32 = 24;

/// 업데이트 설정
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct UpdateConfig {
    pub enabled: bool,
    /// 확인 주기 (시간 단위, 기본값 3, 최소 1시간에서 최대 8시간)
    pub check_interval_hours: u32,
    /// 확인 뒤 새 버전을 미리 내려받아 둘지
    pub auto_download: bool,
    /// 앱 업데이트를 묻지 않고 적용할지 (켜면 내려받기도 한다). 끄면 받아 두거나(`auto_download`) 알리기만 하고,
    /// 적용은 사용자가 다시 시작을 누를 때 한다
    pub auto_apply: bool,
    /// 묻지 않는 자동 적용으로 바꾸는 한 번의 이전을 했다 (데몬 `load_updater_config`). 이 키가 없는 옛 설정은
    /// 한 번 `auto_apply`·`auto_download`를 켠다. 그 뒤 사용자가 고른 값은 그대로 둔다
    #[serde(default)]
    pub live_update_migrated: bool,
    /// 스테이징 디렉터리 (다운로드와 임시 파일 저장, 기본값: 실행파일 경로 기준)
    pub install_root: Option<String>,
    /// 릴레이 주소 (None이면 relay::DEFAULT_RELAY_URL). 모의 릴레이 시험은 여기에
    /// `scripts/mock-release-server.js` 주소를 넣는다. 0.1.1은 `relay_base_url`로 저장했다
    #[serde(alias = "relay_base_url", deserialize_with = "blank_as_none")]
    pub relay_url: Option<String>,
    /// 릴레이 업데이트 채널 (stable | rc | beta | nightly). 기본은 이 빌드의 채널
    pub channel: String,
    /// 적용했다가 되돌린 판 (버전 문자열). 이 판은 자동 적용하지 않는다. 더 새 판이 나오거나 사용자가 다시 시작을 누르면 지운다
    #[serde(default)]
    pub failed_release: Option<String>,
}

/// 빈 주소는 "설정 안 함" (0.1.1은 비워 두면 `""`로 저장했다)
fn blank_as_none<'de, D: serde::Deserializer<'de>>(d: D) -> Result<Option<String>, D::Error> {
    let v: Option<String> = Option::deserialize(d)?;
    Ok(v.filter(|s| !s.trim().is_empty()))
}

impl Default for UpdateConfig {
    fn default() -> Self {
        Self {
            enabled: true,
            check_interval_hours: 3,
            // 앱 업데이트는 묻지 않고 적용한다 (사용자 결정 2026-11-09)
            auto_download: true,
            auto_apply: true,
            live_update_migrated: true,
            install_root: None,
            relay_url: None,
            // 고르지 않았으면 이 빌드의 채널(`versions.toml` `[release] channel`): 베타 빌드가 정식의 옛 판을 내림으로 거절하지 않게
            channel: crate::versions::bundled().channel.clone(),
            failed_release: None,
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

/// 설치기가 업데이트 적용(`--apply`)을 맡은 첫 판. 이보다 옛 설치기(Windows 0.1.12-beta.1까지)는 `--apply`를 모르고 설치 창을 연다
pub const INSTALLER_APPLIES_SINCE: &str = "0.1.12-beta.2";

/// 설치기가 설치 폴더에 남기는 자기 사본 (제거에 쓴다)
pub const INSTALLER_EXE: &str = if cfg!(windows) { "saba-chan-installer.exe" } else { "saba-chan-installer" };

/// 0.1.12 전에 업데이트를 적용하던 옛 업데이터. 지금은 설치기(`INSTALLER_EXE --apply`)가 적용한다
pub const LEGACY_UPDATER_EXE: &str = if cfg!(windows) { "saba-chan-updater.exe" } else { "saba-chan-updater" };

/// 설치기가 있으면 설치 폴더에 남은 옛 업데이터와 그 버전·파일 기록을 지운다. 실패해도 다음에 다시 한다
pub fn retire_legacy_updater() {
    let Some(root) = std::env::current_exe().ok().and_then(|p| p.parent().map(Path::to_path_buf)) else { return };
    retire_legacy_updater_at(&root, &constants::resolve_installed_manifest_path(), &integrity::records_path());
}

pub fn retire_legacy_updater_at(install_root: &Path, installed_manifest: &Path, records: &Path) {
    let legacy = install_root.join(LEGACY_UPDATER_EXE);
    // 설치 폴더의 설치기가 적용을 아는 판으로 기록돼 있을 때만: 새 사본 복사가 실패해 옛 설치기가 남았으면 옛 업데이터가 유일한 적용 수단이다
    let applies = UpdateManager::load_installed_manifest_at(installed_manifest)
        .get("installer")
        .and_then(|v| version::SemVer::parse(v))
        .zip(version::SemVer::parse(INSTALLER_APPLIES_SINCE))
        .is_some_and(|(installed, since)| installed >= since);
    if !install_root.join(INSTALLER_EXE).is_file() || !applies {
        return;
    }
    // 파일이 이미 없어도(지워졌거나 격리됨) 남은 기록은 걷는다
    if legacy.exists() {
        if let Err(e) = std::fs::remove_file(&legacy) {
            tracing::warn!("[Updater] Cannot remove the retired updater {}: {}", legacy.display(), e);
            return;
        }
        tracing::info!("[Updater] Removed the retired updater {}", legacy.display());
    }
    let key = Component::Updater.manifest_key();
    let mut versions = UpdateManager::load_installed_manifest_at(installed_manifest);
    if versions.remove(&key).is_some() {
        if let Err(e) = UpdateManager::save_installed_manifest_at(installed_manifest, &versions) {
            tracing::warn!("[Updater] Cannot update {}: {}", installed_manifest.display(), e);
        }
    }
    if let Err(e) = integrity::retain_at(records, |k| k != key) {
        tracing::warn!("[Updater] Cannot update {}: {}", records.display(), e);
    }
}

/// 설치 폴더 바로 안의 GUI 실행 파일
pub fn find_gui_exe(install_root: &Path) -> Option<PathBuf> {
    GUI_EXE_NAMES.iter().map(|n| install_root.join(n)).find(|p| p.is_file())
}

/// 같은 파일을 가리키는 경로인지. 구성 요소로 비교해 `./`는 무시하고,
/// Windows 파일 이름은 대소문자를 가리지 않는다
pub fn same_path(a: &Path, b: &Path) -> bool {
    path_parts(a) == path_parts(b)
}

/// 같은 폴더이거나 한쪽이 다른 쪽 안에 있는지 (`same_path`와 같은 비교). 빈 경로는 어디와도 겹치지 않는다
pub fn paths_overlap(a: &Path, b: &Path) -> bool {
    let (a, b) = (path_parts(a), path_parts(b));
    !a.is_empty() && !b.is_empty() && (a.starts_with(&b) || b.starts_with(&a))
}

fn path_parts(p: &Path) -> Vec<String> {
    p.components()
        .filter(|c| !matches!(c, std::path::Component::CurDir))
        .map(|c| {
            let s = c.as_os_str().to_string_lossy();
            if cfg!(windows) { s.to_lowercase() } else { s.into_owned() }
        })
        .collect()
}

const ZIP_ENTRY_LIMIT: usize = 20_000;
const ZIP_EXPANDED_LIMIT: u64 = 1024 * 1024 * 1024;

/// zip을 열고 파일 수와 풀린 크기(적힌 값)가 한도 안인지 본다
fn open_zip(staged: &Path) -> Result<zip::ZipArchive<std::fs::File>> {
    let mut archive = zip::ZipArchive::new(std::fs::File::open(staged)?)?;
    if archive.len() > ZIP_ENTRY_LIMIT {
        anyhow::bail!("Archive has too many files");
    }
    let mut declared = 0u64;
    for i in 0..archive.len() {
        declared = declared.saturating_add(archive.by_index(i)?.size());
        if declared > ZIP_EXPANDED_LIMIT {
            anyhow::bail!("Archive expands beyond the size limit");
        }
    }
    Ok(archive)
}

/// 엔트리가 풀릴 상대 경로. 밖으로 나가는 경로는 거부한다
fn zip_entry_path<R: std::io::Read>(entry: &zip::read::ZipFile<'_, R>, staged: &Path) -> Result<PathBuf> {
    // enclosed_name은 `a/../b`를 받아 주므로 tar와 같이 평범한 칸만 받는다 (`..`를 지나면 링크 검사가 빗나간다)
    match entry.enclosed_name().filter(|p| p.components().all(|c| matches!(c, std::path::Component::Normal(_)))) {
        Some(rel) => Ok(rel),
        None => anyhow::bail!("Unsafe path in archive {}: {}", staged.display(), String::from_utf8_lossy(entry.name_raw())),
    }
}

/// 엔트리 내용을 `out`에 옮긴다 (끝까지 읽으면 zip이 CRC를 확인한다). `expanded`는 이 zip에서 지금까지 푼 크기
fn copy_zip_entry(entry: &mut impl std::io::Read, out: &mut impl std::io::Write, expanded: &mut u64) -> std::io::Result<()> {
    let mut buffer = [0u8; 64 * 1024];
    loop {
        let count = entry.read(&mut buffer)?;
        if count == 0 {
            return Ok(());
        }
        *expanded = expanded.saturating_add(count as u64);
        if *expanded > ZIP_EXPANDED_LIMIT {
            return Err(std::io::Error::new(std::io::ErrorKind::InvalidData, "Archive expands beyond the size limit"));
        }
        out.write_all(&buffer[..count])?;
    }
}

/// zip을 풀지 않고 확인한다: `extract_zip`과 같은 한도·경로 규칙을 보고, 모든 엔트리를 끝까지 읽어 내용(CRC)이 온전한지 본다.
/// 파일을 쓰지 않는다 (적용 전에 받은 zip을 미리 확인할 때 쓴다)
pub fn verify_zip(staged: &Path) -> Result<()> {
    let mut archive = open_zip(staged)?;
    let mut expanded = 0u64;
    for i in 0..archive.len() {
        let mut entry = archive.by_index(i)?;
        zip_entry_path(&entry, staged)?;
        copy_zip_entry(&mut entry, &mut std::io::sink(), &mut expanded)?;
    }
    Ok(())
}

/// zip을 `target_dir`에 푼다. 모든 압축 해제는 이 함수를 거친다.
///
/// 엔트리 경로는 `enclosed_name()`으로 검사해 `..`나 절대 경로로
/// `target_dir` 밖에 쓰는 엔트리(zip-slip)를 거부한다. `before_write`는
/// 파일을 쓰기 직전에 호출된다(실행 중 바이너리를 비켜 두는 데 쓴다).
/// 대상 폴더 안의 기존 링크(Windows 정션 포함)를 따라 밖에 쓰지 않는다.
/// 쓴 파일 경로를 돌려준다(무결성 기록용).
pub fn extract_zip(
    staged: &Path,
    target_dir: &Path,
    mut before_write: impl FnMut(&Path) -> Result<()>,
) -> Result<Vec<PathBuf>> {
    let mut written = Vec::new();
    let mut archive = open_zip(staged)?;
    let mut expanded = 0u64;
    for i in 0..archive.len() {
        let mut entry = archive.by_index(i)?;
        let rel = zip_entry_path(&entry, staged)?;
        reject_links_below(target_dir, &rel)?;
        let out_path = target_dir.join(&rel);
        if entry.is_dir() {
            std::fs::create_dir_all(&out_path)?;
            continue;
        }
        if let Some(parent) = out_path.parent() {
            std::fs::create_dir_all(parent)?;
        }
        before_write(&out_path)?;
        // 옆 이름에 다 쓴 뒤 바꾼다. 쓰다 꺼져도 원래 이름에는 반쯤 쓴 파일이 생기지 않아,
        // 다음 정리(`cleanup_old_files`)가 비켜 둔 `.update-bak`을 믿고 되돌릴 수 있다
        let mut partial = out_path.clone().into_os_string();
        partial.push(".update-new");
        let partial = PathBuf::from(partial);
        let wrote = std::fs::File::create(&partial).and_then(|mut f| {
            copy_zip_entry(&mut entry, &mut f, &mut expanded)?;
            f.sync_all()
        });
        // 리눅스 zip의 실행 파일은 실행 권한을 지닌다 (zip에 적힌 권한 중 실행 비트만 옮긴다)
        #[cfg(unix)]
        let wrote = wrote.and_then(|_| {
            use std::os::unix::fs::PermissionsExt;
            match entry.unix_mode() {
                Some(mode) if mode & 0o111 != 0 => std::fs::set_permissions(&partial, std::fs::Permissions::from_mode(0o755)),
                _ => Ok(()),
            }
        });
        if let Err(e) = wrote.and_then(|_| std::fs::rename(&partial, &out_path)) {
            let _ = std::fs::remove_file(&partial);
            return Err(e.into());
        }
        written.push(out_path);
    }
    Ok(written)
}

/// `target_dir` 아래 `rel`까지 이미 있는 경로 중 링크가 있으면 거부한다 (`target_dir` 자체는 링크여도 된다).
/// Windows의 `is_symlink`는 정션도 링크로 본다
pub(crate) fn reject_links_below(target_dir: &Path, rel: &Path) -> Result<()> {
    let mut path = target_dir.to_path_buf();
    for part in rel.components() {
        path.push(part);
        match std::fs::symlink_metadata(&path) {
            Ok(md) if md.file_type().is_symlink() => {
                anyhow::bail!("Refusing to write through a link: {}", path.display());
            }
            Ok(_) => {}
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => break,
            Err(e) => return Err(e.into()),
        }
    }
    Ok(())
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
    /// 설치 기록(installed-manifest.json). 적용 결과를 여기에 적는다
    installed_manifest_file: PathBuf,
    /// 마지막으로 받은 서명 검증된 manifest. 무결성·의존성 검사와 다운로드가 같이 쓴다
    cached_manifest: Option<ReleaseManifest>,
    /// 마지막으로 확인한 릴리즈 (버전·패치노트). 받아 둘 때 적용 결과에 실으려고 남긴다 (A5)
    latest_release: Option<relay::LatestRelease>,
    /// 받아 둔 목록을 디스크에서 되살렸을 때 그 릴리즈 요약 (확인 전이라 `latest_release`가 없다)
    restored_release: Option<crate::marker::ReleaseSummary>,
    /// 받아 둔 목록을 되살릴 때 이미 같거나 더 새 판이 깔려 있어 뺀 키. 적용 대상 목록에서도 뺀다
    already_installed: Vec<String>,
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
                stable_wait: None,
            },
            modules_dir: PathBuf::from(modules_dir),
            extensions_dir,
            staging_dir,
            install_root,
            installed_manifest_file: crate::constants::resolve_installed_manifest_path(),
            cached_manifest: None,
            latest_release: None,
            restored_release: None,
            already_installed: Vec::new(),
            download_progress: Arc::new(StdMutex::new(DownloadProgress::default())),
        };
        // 지난 업데이트가 남긴 .exe.old: 적용 직후엔 옛 프로세스가 쥐고 있어 못 지운 것을 다음 실행 때 지운다
        mgr.cleanup_old_files();
        mgr
    }

    fn resolve_staging_dir() -> PathBuf {
        crate::constants::resolve_staging_dir()
    }

    /// 모듈·익스텐션 폴더를 옮긴 뒤 새 폴더에 적용하게 한다 (데몬 `/api/system/paths/move`)
    pub fn set_component_dir(&mut self, kind: component_dirs::Kind, dir: &Path) {
        match kind {
            component_dirs::Kind::Modules => self.modules_dir = dir.to_path_buf(),
            component_dirs::Kind::Extensions => self.extensions_dir = dir.to_path_buf(),
        }
    }

    /// 익스텐션 디렉터리 — constants 모듈에 위임
    fn resolve_extensions_dir() -> PathBuf {
        crate::constants::resolve_extensions_dir()
    }

    /// 익스텐션 ID에서 디스크 상의 디렉토리 경로를 해석합니다.
    /// Python import 호환을 위해 하이픈→언더스코어 변환도 시도합니다.
    fn resolve_ext_dir(&self, ext_name: &str) -> PathBuf {
        // 데몬(`extension::extension_dir`)과 같은 순서: Python-safe 이름(하이픈→언더스코어) 먼저, 다음 원본 이름.
        // 둘 다 없으면 Python-safe 이름 (신규 설치용)
        let safe = self.extensions_dir.join(ext_name.replace('-', "_"));
        let exact = self.extensions_dir.join(ext_name);
        if !safe.exists() && exact.exists() {
            return exact;
        }
        safe
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
            .with_manifest_cache(self.staging_dir.join("manifests"))
    }

    /// 받을 코어(manifest `saba-core`)가 다시 붙을 수 있는 러너 형식 범위. 확인 전이거나 코어가 없는 manifest면 None
    pub fn core_reads_runner_formats(&self) -> Option<(u32, u32)> {
        self.cached_manifest.as_ref()?.components.get("saba-core").map(relay::ManifestComponent::runner_formats)
    }

    /// 지금 적용하는(받아 둔) 릴리즈 버전
    pub fn applying_version(&self) -> Option<String> {
        self.applying_release().map(|r| r.version)
    }

    // ─── 업데이트 확인 ────────────────────────────────────────────────────────

    /// 릴레이의 서명된 manifest 하나로 모든 컴포넌트의 최신 버전과 R2 URL을 얻는다.
    pub async fn check_for_updates(&mut self) -> Result<UpdateStatus> {
        self.check_for_updates_hinted(None).await.map(|(status, _)| status)
    }

    /// `check_for_updates`에 heartbeat가 알린 릴리즈 id(`hint`)를 캐시 키로 붙인다. 확인 결과와,
    /// 릴레이 `/latest` 응답에 담긴 가장 큰 릴리즈 id(옛 릴레이면 None)를 준다
    pub async fn check_for_updates_hinted(&mut self, hint: Option<u64>) -> Result<(UpdateStatus, Option<u64>)> {
        self.status.checking = true;
        self.status.error = None;

        let local_versions = self.collect_local_versions();
        let client = self.relay_client().with_release_hint(hint);
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

        // 모듈·익스텐션은 앱 릴리즈와 따로 내는 판 목록에서, 지금 코어로 돌아가는 가장 새 판으로 고른다 (릴레이 `/v1/addons/catalog`).
        // 목록이 없으면(옛 릴레이, 아직 올린 판 없음) 릴리즈 manifest의 판을 그대로 쓰고, 읽지 못하면 이번에는 모듈·익스텐션을 내지 않는다
        let core = if current.is_empty() { relay::core_version() } else { current.clone() };
        let catalog = client.fetch_addon_catalog(&core, &self.config.channel).await;
        // 깐 판이 내린 판인지는 목록의 서명된 내림 기록으로 안다
        let addons = catalog.as_ref().ok().and_then(Option::as_ref).map(|c| relay::AddonCatalog { components: HashMap::new(), yanked: c.yanked.clone() });
        let release = relay::merge_addon_catalog(release, catalog, &core, &self.config.channel, &platform);

        let offered = release.as_ref().map(|r| r.latest.version.clone());
        let previous = std::mem::take(&mut self.status.components);
        let mut components = Vec::new();
        // 헤드리스 설치는 GUI처럼 그 구성에 들지 않는 컴포넌트를 받지 않는다
        let profile = profile::Profile::load();
        if let Some(mut release) = release {
            release.manifest = release.manifest.for_profile(profile);
            let notes = Self::release_notes_in(&release.latest, &Self::user_language());
            for (key, info) in &release.manifest.components {
                let Some(component) = Component::try_from_manifest_key(key) else {
                    tracing::debug!("[Updater] Skipping unknown manifest key: {}", key);
                    continue;
                };
                if !profile.includes(key) {
                    continue;
                }
                let current = local_versions.get(key).cloned().unwrap_or_default();
                let installed = self.is_component_installed(&component);
                // 모듈·익스텐션은 설치한 것만 업데이트한다. 새로 받는 건 목록 화면의 몫
                if !installed && matches!(component, Component::Module(_) | Component::Extension(_)) {
                    continue;
                }
                // 설치돼 있는데 로컬 버전을 모르면 알림 대신 최신으로 기록
                let update_available = if installed
                    && (Self::reinstall_from_catalog(&component, &current, &info.version)
                        || (current.is_empty() && Self::unknown_version_needs_update(&component)))
                {
                    true
                } else if installed && current.is_empty() {
                    let _ = Self::update_installed_version(key, &info.version);
                    false
                } else if addons.as_ref().is_some_and(|c| c.is_yanked(key, &current)) && info.version != current {
                    // 깐 판을 내렸다: 목록의 판으로 되돌린다 (더 낮은 판이어도)
                    true
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
                    requires: info.requires.clone(),
                    installed,
                });
                Self::keep_staged(&previous, components.last_mut().expect("just pushed"));
            }
            // 무결성 검사·의존성 검사·다운로드가 같은 manifest를 보게 한다
            self.cached_manifest = Some(release.manifest);
            self.latest_release = Some(release.latest);
        }

        let now = time::now_iso();
        // 주기 확인은 24시간마다다 (연결된 데몬은 heartbeat의 판 번호로 안다). `check_interval_hours`는 읽기만 한다
        let next = time::iso_after_hours(CHECK_EVERY_HOURS);
        // 같은 판 베타를 받는 동안 기다리는 정식 판. 정식(그 판 이상)을 받으면 사라진다
        let stable_wait = relay::stable_wait_base(&self.config.channel, &current)
            .filter(|_| offered.as_deref().and_then(SemVer::parse).is_none_or(|v| v.is_prerelease()));
        self.status = UpdateStatus {
            last_check: Some(now),
            next_check: Some(next),
            components,
            checking: false,
            error: None,
            stable_wait,
        };
        Ok((self.status.clone(), client.latest_seen_id()))
    }

    /// 상태에 싣는 패치노트: 적용 뒤 결과 마커와 같은 규칙(`ReleaseSummary::notes_for`)으로 고른다
    pub(crate) fn release_notes_in(latest: &relay::LatestRelease, lang: &str) -> Option<String> {
        marker::ReleaseSummary::from(latest).notes_for(lang).map(str::to_string)
    }

    /// GUI에서 고른 언어. 없으면 OS 언어, 그것도 없으면 영어 (GUI·데몬과 같은 순서)
    fn user_language() -> String {
        let chosen = config_file::ConfigFile::default_location().section("gui")["language"].as_str().map(str::to_string);
        chosen.filter(|l| !l.trim().is_empty()).or_else(sys_locale::get_locale).unwrap_or_else(|| "en".to_string())
    }

    /// 다시 확인해도 같은 버전을 받아 둔 파일이 남아 있으면 적용 대기를 유지한다.
    /// 다운로드 사이에 예약 확인이 끼어들면 적용할 것이 없어지던 문제.
    fn keep_staged(previous: &[ComponentVersion], comp: &mut ComponentVersion) {
        let Some(prev) = previous.iter().find(|p| p.component == comp.component) else { return };
        let still_there = prev.downloaded_path.as_deref().is_some_and(|p| Path::new(p).is_file());
        if prev.downloaded && still_there && prev.latest_version == comp.latest_version {
            comp.downloaded = true;
            comp.downloaded_path = prev.downloaded_path.clone();
            comp.staged_sha256 = prev.staged_sha256.clone();
        }
    }

    /// 설치 기록 없이 놓인 모듈·익스텐션(0.1.6 설치기가 받은 것)은 같은 버전이라도 카탈로그에서 다시 받는다.
    /// 다시 받아야 공식 표시가 생겨 데몬이 격리하지 않는다. 기록이 있으면(사용자가 고쳤을 수 있다) 버전만 본다
    /// 로컬 사본이 카탈로그보다 새 버전이면 내려받지 않는다
    pub(crate) fn reinstall_from_catalog(component: &Component, current: &str, latest: &str) -> bool {
        matches!(component, Component::Module(_) | Component::Extension(_))
            && !version::is_downgrade(current, latest)
            && !integrity::is_recorded(&component.manifest_key())
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
    /// 설치 기록에 없는 앱 컴포넌트를 빌드에 묶인 버전으로 채운다. 설치기는 0.1.7까지 기록되지 않았고
    /// 이 빌드와 함께 깔린다는 보장도 없어(업데이터만 바뀐 설치본) 채우지 않는다
    pub(crate) fn fill_bundled_versions(versions: &mut HashMap<String, String>) {
        for (key, version) in &crate::versions::bundled().components {
            if key != "installer" {
                versions.entry(key.clone()).or_insert_with(|| version.clone());
            }
        }
    }

    /// 깔려 있는데 버전을 모르면 최신으로 기록하는 대신 바꿔야 하는 컴포넌트 (기록이 없으면 옛 판이다)
    pub(crate) fn unknown_version_needs_update(component: &Component) -> bool {
        matches!(component, Component::Installer)
    }

    fn collect_local_versions(&self) -> HashMap<String, String> {
        // 1. 설치 매니페스트 우선 로드 (가장 신뢰할 수 있는 소스)
        let mut versions = Self::load_installed_manifest();

        // 2. 매니페스트에 없는 앱 컴포넌트는 빌드에 묶인 versions.toml 버전 (폴백)
        Self::fill_bundled_versions(&mut versions);

        // 3. 모듈·익스텐션: 디스크에서 감지 (기록에 없는 것만)
        for (key, version) in self.disk_addon_versions() {
            versions.entry(key).or_insert(version);
        }

        tracing::debug!("[UpdateManager] Local versions: {:?}", versions);
        versions
    }

    /// 디스크에 있는 모듈(`modules/*/module.toml`)과 익스텐션(`extensions/*/manifest.json`)의 키·버전
    fn disk_addon_versions(&self) -> HashMap<String, String> {
        let mut versions = HashMap::new();
        if let Ok(entries) = std::fs::read_dir(&self.modules_dir) {
            for entry in entries.flatten() {
                let path = entry.path();
                if path.is_dir() {
                    if let Some((name, version)) = self.read_module_version(&path.join("module.toml")) {
                        versions.insert(format!("module-{}", name), version);
                    }
                }
            }
        }
        if let Ok(entries) = std::fs::read_dir(&self.extensions_dir) {
            for entry in entries.flatten() {
                let path = entry.path();
                if path.is_dir() {
                    if let Some((name, version)) = read_extension_version(&path.join("manifest.json")) {
                        versions.insert(format!("ext-{}", name), version);
                    }
                }
            }
        }
        versions
    }

    /// requires를 비교할 버전: 앱 컴포넌트는 설치 기록(없으면 번들 버전), 모듈·익스텐션은 디스크에 실제로 있는 것만.
    /// 카탈로그 설치·삭제는 설치 기록을 고치지 않으므로 그 기록의 모듈·익스텐션은 믿지 않는다
    fn dependency_versions(&self) -> HashMap<String, String> {
        self.dependency_versions_for(profile::Profile::load())
    }

    /// 설치 구성에 들지 않는 컴포넌트(헤드리스의 GUI)는 번들 버전으로 채우지 않는다
    pub(crate) fn dependency_versions_for(&self, profile: profile::Profile) -> HashMap<String, String> {
        let mut versions = Self::load_installed_manifest();
        Self::fill_bundled_versions(&mut versions);
        versions.retain(|key, _| profile.includes(key));
        merge_addons_from_disk(versions, self.disk_addon_versions())
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
        let total = self.status.components.iter().filter(|c| c.update_available && !c.downloaded).count();
        let (downloaded, failed) = self.download_available_updates_report().await?;
        if !failed.is_empty() {
            let detail: Vec<String> = failed.iter().map(|(k, e)| format!("{}: {}", k, e)).collect();
            anyhow::bail!("Failed to download {} of {} updates: {}", failed.len(), total, detail.join("; "));
        }
        Ok(downloaded)
    }

    /// 받을 수 있는 업데이트를 모두 받고 `(받은 파일 이름, 실패한 (키, 이유))`를 돌려준다.
    /// 하나가 실패해도 나머지는 받고, 받은 것은 상태에 남긴다. 받기 전 준비가 안 되면 Err
    pub async fn download_available_updates_report(&mut self) -> Result<(Vec<String>, Vec<(String, String)>)> {
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

        // 하나가 실패해도 나머지는 받고, 받은 것은 아래에서 상태에 남긴다
        let client = crate::tls::http_client_builder().build().unwrap();
        let mut failed = Vec::new();
        for (key, rc) in &to_download {
            let dest = self.staging_dir.join(&rc.filename);
            tracing::info!("[Updater] Downloading {}", key);
            match download::download_verified(&client, &rc.url, &dest, Some(&rc.sha256), rc.size_bytes, None).await {
                Ok(_) => {
                    tracing::info!("[Updater] Downloaded {}", rc.filename);
                    downloaded.push(rc.filename.clone());
                }
                Err(e) => {
                    tracing::warn!("[Updater] Download of {} failed: {}", key, e);
                    failed.push((key.clone(), e.to_string()));
                }
            }
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

        Ok((downloaded, failed))
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
        let client = crate::tls::http_client_builder().build().unwrap();
        let result = download::download_verified(
            &client,
            &rc.url,
            &dest,
            Some(&rc.sha256),
            rc.size_bytes,
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

    /// 데몬이 정한 키 목록(apply-targets)을 차례로 적용한다. 첫 실패에서 멈춘다:
    /// 데몬·업데이터가 실패했는데 인터페이스만 새것으로 바뀌면 서로 맞지 않는다.
    /// `(적용한 키, 실패·건너뛴 키와 이유)`. `on_progress(순번, 키, 지금까지 적용한 키)`
    pub async fn apply_target_keys(
        &mut self,
        keys: &[String],
        mut on_progress: impl FnMut(usize, &str, &[String]),
    ) -> (Vec<String>, Vec<String>) {
        // 옛 데몬은 모듈을 코어보다 앞에 적는다: 적용 순서로 다시 세운다 (같은 순위는 받은 순서대로)
        let keys = self.apply_order(keys);
        let mut applied = Vec::new();
        let mut failed = Vec::new();
        for (i, key) in keys.iter().enumerate() {
            if !failed.is_empty() {
                failed.push(format!("{}: skipped after an earlier failure", key));
                continue;
            }
            on_progress(i, key, &applied);
            match self.apply_single_component(&Component::from_manifest_key(key)).await {
                Ok(result) if result.success => {
                    tracing::info!("[Apply] {} ✓", key);
                    applied.push(key.clone());
                }
                Ok(result) => {
                    tracing::warn!("[Apply] {} failed: {}", key, result.message);
                    failed.push(format!("{}: {}", key, result.message));
                }
                Err(e) => {
                    tracing::error!("[Apply] {} error: {}", key, e);
                    failed.push(format!("{}: {}", key, e));
                }
            }
        }
        (applied, failed)
    }

    /// 모듈·익스텐션 폴더를 바꾸되, 원래 폴더를 옮긴 뒤 설치 기록 그대로가 아니면 되돌리고 멈춘다 (`swap_dir_untouched`)
    fn replace_untouched_addon(&self, component: &Component, staged_path: &str) -> Result<Vec<PathBuf>> {
        let target = match component {
            Component::Module(name) => self.modules_dir.join(name),
            Component::Extension(name) => self.resolve_ext_dir(name),
            other => anyhow::bail!("{} is not a module or an extension", other.manifest_key()),
        };
        let key = component.manifest_key();
        let files = swap_dir_untouched(&target, Path::new(staged_path), |fingerprint| integrity::matches_install_record(&key, &target, fingerprint))?;
        std::fs::remove_file(staged_path).ok();
        Ok(files)
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
            Component::Installer => self.apply_installer_update(staged_path),
            // 옆 폴더에 다 푼 뒤 이름 바꾸기로 통째로 바꾸고 판 번호(`locales/version`)를 적는다: 읽는 쪽이 반쯤 바뀐 묶음을 보지 않는다
            Component::Locales => {
                let version = self.status.components.iter().find(|c| c.component == Component::Locales).and_then(|c| c.latest_version.clone());
                apply::replace_locales(&self.install_root.join("locales"), Path::new(staged_path), version.as_deref())
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

    /// 받아 둔 업데이트의 키를 적용 순서대로. 목록 없는 적용도 `apply_target_keys`로 키마다 결과를 남긴다
    pub fn pending_keys_in_apply_order(&self) -> Vec<String> {
        let pending: Vec<String> = self.status.components.iter()
            .filter(|c| c.downloaded && c.update_available)
            .map(|c| c.component.manifest_key())
            .collect();
        self.apply_order(&pending)
    }

    /// 컴포넌트의 적용 우선순위를 반환합니다.
    ///
    /// 낮은 값일수록 먼저 적용됩니다:
    /// - 0: Updater (업데이트 방식 변경 대응 — 디스크의 바이너리를 먼저 교체)
    /// - 1: CoreDaemon (데몬 프로세스 종료 후 교체). 모듈·익스텐션이 새 코어를 `requires`할 수 있어 먼저
    /// - 2: 모듈, 익스텐션, Locales, Installer (프로세스 중단 불필요)
    /// - 4: 사용하지 않는 인터페이스 (GUI 사용 중이면 CLI, 반대도 마찬가지)
    /// - 5: 현재 사용 중인 인터페이스 (재시작 필요)
    pub fn component_apply_priority(component: &Component) -> u8 {
        match component {
            Component::Updater => 0,
            Component::CoreDaemon => 1,
            Component::Module(_) | Component::Extension(_) | Component::Locales | Component::Installer => 2,
            // GUI/CLI는 현재 실행 중인 인터페이스를 마지막에 적용
            Component::Gui => {
                if ProcessChecker::is_gui_running() { 5 } else { 4 }
            }
            Component::Cli => {
                if ProcessChecker::is_cli_running() { 5 } else { 4 }
            }
        }
    }

    // ─────── 2-flow 아키텍처: 개별 컴포넌트 적용 ────────────────────────────────────────────────────────────────────────

    /// 적용 중 꺼져 남은 파일을 정리한다 (설치 폴더 바로 안만). 적용하는 프로세스만 적용 직전에 부른다:
    /// 다른 프로세스(데몬 시작 등)가 적용 도중에 부르면 비켜 둔 백업을 되돌리고 쓰는 중인 파일을 지운다
    pub fn settle_interrupted_updates(&self) {
        self.restore_moved_aside_exes();
        apply::settle_locales(&self.install_root.join("locales"));
        let Ok(entries) = std::fs::read_dir(&self.install_root) else { return };
        for path in entries.flatten().map(|e| e.path()) {
            let Some(name) = path.file_name().and_then(|n| n.to_str()) else { continue };
            // 쓰다 꺼져 남은 `extract_zip`의 반쪽 파일
            if name.ends_with(".update-new") {
                if let Err(e) = std::fs::remove_file(&path) {
                    tracing::debug!("[Updater] Cannot remove partial file {}: {}", path.display(), e);
                }
                continue;
            }
            // `extract_zip_or_restore`가 남긴 `.update-bak`: 원래 파일이 있으면 새 판이 다 들어간 것이라
            // (`extract_zip`은 다 쓴 뒤에만 원래 이름을 만든다) 지우고, 없으면 비켜 둔 채 끝난 것이라 되돌린다
            if let Some(original) = name.strip_suffix(".update-bak").filter(|o| !o.is_empty()) {
                let original = self.install_root.join(original);
                let done = if original.exists() { std::fs::remove_file(&path) } else { std::fs::rename(&path, &original) };
                if let Err(e) = done {
                    tracing::debug!("[Updater] Cannot settle leftover backup {}: {}", path.display(), e);
                }
            }
        }
    }

    /// 설치 폴더와 그 안 `bin`(CLI zip의 명령 실행기 `bin/saba-chan.exe`)의 사바쨩 실행 파일 백업 `<exe>.old` 또는 그 이름이 잡혀 있을 때 쓴 `<exe>.old.<n>` (`free_backup_path`),
    /// Unix는 확장자 없는 바이너리를 비켜 둔 `<이름>.old` (`move_aside`).
    /// (경로, 실행 파일 이름, 번호(`.old`는 0), GUI인가). 제자리는 백업과 같은 폴더다 (`backup_home`)
    fn old_exe_backups(&self) -> Vec<(PathBuf, String, u32, bool)> {
        self.try_old_exe_backups().unwrap_or_else(|e| {
            tracing::warn!("[Updater] Cannot list {}: {}", self.install_root.display(), e);
            Vec::new()
        })
    }

    /// `old_exe_backups`와 같되 설치 폴더(와 그 항목)를 읽지 못하면 오류
    pub(crate) fn try_old_exe_backups(&self) -> std::io::Result<Vec<(PathBuf, String, u32, bool)>> {
        let mut entries: Vec<std::io::Result<std::fs::DirEntry>> = std::fs::read_dir(&self.install_root)?.collect();
        match std::fs::read_dir(self.install_root.join("bin")) {
            Ok(bin) => entries.extend(bin),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => {}
            Err(e) => return Err(e),
        }
        let mut found = Vec::new();
        for entry in entries {
            let path = entry?.path();
            let Some(name) = path.file_name().and_then(|n| n.to_str()) else { continue };
            if let Some(exe) = name.strip_suffix(".old").filter(|_| cfg!(unix)).filter(|exe| Self::is_known_binary(Path::new(exe))) {
                found.push((path.clone(), exe.to_string(), 0, GUI_EXE_NAMES.contains(&exe)));
                continue;
            }
            let Some((stem, tail)) = name.split_once(".exe.old") else { continue };
            let number = match tail.strip_prefix('.') {
                None if tail.is_empty() => 0,
                Some(n) if !n.is_empty() && n.bytes().all(|b| b.is_ascii_digit()) => n.parse().unwrap_or(u32::MAX),
                _ => continue,
            };
            let gui = GUI_EXE_NAMES.iter().any(|g| g.trim_end_matches(".exe") == stem);
            if gui || Self::is_known_binary(Path::new(stem)) {
                found.push((path.clone(), format!("{stem}.exe"), number, gui));
            }
        }
        Ok(found)
    }

    /// 백업을 되돌릴 자리: 백업과 같은 폴더의 실행 파일
    fn backup_home(backup: &Path, exe: &str) -> PathBuf {
        backup.with_file_name(exe)
    }

    /// 실행 파일이 아예 없는 것: 비켜 둔 뒤 새 판을 두기 전에 적용이 끊겼다. 가장 최근 사본(번호가 가장 큰 것)을 되돌린다.
    /// (낮은 번호가 남아 있는 것은 그때 아직 쥐고 있어 지우지 못한 더 옛 사본이다)
    fn restore_moved_aside_exes(&self) {
        let mut latest: HashMap<PathBuf, (PathBuf, u32)> = HashMap::new();
        for (path, exe, number, gui) in self.old_exe_backups() {
            let exe = Self::backup_home(&path, &exe);
            let present = if gui { find_gui_exe(&self.install_root).is_some() } else { exe.exists() };
            if present {
                continue;
            }
            if latest.get(&exe).is_none_or(|(_, n)| number > *n) {
                latest.insert(exe, (path, number));
            }
        }
        for (exe, (path, _)) in latest {
            match std::fs::rename(&path, &exe) {
                Ok(()) => tracing::warn!("[Updater] Restored {} from {} (an earlier apply was interrupted)", exe.display(), path.display()),
                Err(e) => tracing::warn!("[Updater] Cannot restore {} from {}: {}", exe.display(), path.display(), e),
            }
        }
    }

    /// 새 데몬이 건강하지 않을 때: `before`(적용 전 목록)에 없던 백업, 곧 이번 적용이 비켜 둔 실행 파일을 모두 되돌린다
    /// (`restore_moved_aside_exes`를 넓힌 것. 그쪽은 실행 파일이 아예 없을 때만 되돌린다).
    /// 새 파일은 `<exe>.rolled-back`으로 비켜 두고 지운다(아직 쥐고 있으면 남는다). 되돌린 실행 파일 경로를 돌려준다
    pub fn roll_back_exes_since(&self, before: &live_apply::ExeBackups) -> std::io::Result<live_apply::ExeRollback> {
        let mut rollback = live_apply::ExeRollback::default();
        for (backup, target) in self.exe_rollbacks_since(before)? {
            // 새 판은 비켜 두기만 하고, 백업을 제자리에 놓은 뒤에 지운다: 어느 단계가 실패해도 실행 파일이 없어지지 않는다
            let mut name = target.file_name().unwrap_or_default().to_os_string();
            name.push(".rolled-back");
            let aside = target.with_file_name(name);
            let moved = if target.exists() {
                let _ = std::fs::remove_file(&aside);
                match std::fs::rename(&target, &aside) {
                    Ok(()) => true,
                    Err(e) => {
                        tracing::error!("[Updater] Cannot move {} aside to roll back: {}", target.display(), e);
                        rollback.pending.push((backup, target));
                        continue;
                    }
                }
            } else {
                false
            };
            match std::fs::rename(&backup, &target) {
                Ok(()) => {
                    tracing::warn!("[Updater] Rolled back {} from {}", target.display(), backup.display());
                    if moved {
                        let _ = std::fs::remove_file(&aside);
                    }
                    rollback.restored.push(target);
                }
                Err(e) => {
                    tracing::error!("[Updater] Cannot roll back {} from {}: {}", target.display(), backup.display(), e);
                    if moved {
                        // 새 판을 제자리에 돌려 둔다 (실행 파일이 없는 것보다 낫다)
                        if let Err(e) = std::fs::rename(&aside, &target) {
                            tracing::error!("[Updater] Cannot put {} back from {}: {}", target.display(), aside.display(), e);
                        }
                    }
                    rollback.pending.push((backup, target));
                }
            }
        }
        Ok(rollback)
    }

    /// 이번 적용이 비켜 둔 실행 파일마다 (가장 최근 백업, 제자리). 적용 전 목록(`before`)에 있던 백업은 고르지 않는다.
    /// 설치 폴더를 읽지 못하면 오류 (빈 목록으로 보고 되돌림을 건너뛰지 않는다)
    pub fn exe_rollbacks_since(&self, before: &live_apply::ExeBackups) -> std::io::Result<Vec<(PathBuf, PathBuf)>> {
        let mut latest: HashMap<PathBuf, (PathBuf, u32)> = HashMap::new();
        for (path, exe, number, _) in self.try_old_exe_backups()? {
            if before.contains(&path, live_apply::backup_id(&path)) {
                continue;
            }
            let exe = Self::backup_home(&path, &exe);
            if latest.get(&exe).is_none_or(|(_, n)| number > *n) {
                latest.insert(exe, (path, number));
            }
        }
        let mut pairs: Vec<(PathBuf, PathBuf)> = latest.into_iter().map(|(exe, (backup, _))| (backup, exe)).collect();
        pairs.sort();
        Ok(pairs)
    }

    /// 업데이트가 남긴 사바쨩 실행 파일 백업(`old_exe_backups`)을 지운다 (설치 폴더 바로 안과 `bin`만).
    /// Windows는 실행 중인 exe를 이름만 바꿔 두므로, 아직 쥐고 있는 파일은 다음 번에 지운다.
    /// 업데이터가 새 데몬을 지켜보는 동안(`live_apply::watch_in_progress`)은 지우지 않는다: 건강하지 않으면 되돌림에 쓴다
    pub fn cleanup_old_files(&self) {
        if live_apply::watch_in_progress(&self.staging_dir) {
            tracing::info!("[Updater] An update is still being checked; keeping the .old backups");
            return;
        }
        for (path, exe, _, gui) in self.old_exe_backups() {
            // 실행 파일이 없으면 되돌리지 못한 유일한 사본이다. 예전 이름의 GUI는 새 이름의 GUI가 있으면 지운다 (`retire_other_gui_exes`)
            let replaced = if gui { find_gui_exe(&self.install_root).is_some() } else { Self::backup_home(&path, &exe).exists() };
            if !replaced {
                tracing::warn!("[Updater] Keeping {}: {} is missing", path.display(), exe);
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
        // 모듈·익스텐션은 디스크에 실제로 있는 버전으로 센다 (카탈로그·수동 설치, 삭제 포함)
        let installed = self.dependency_versions();
        let issues = self.requires_of(component_key).map(|r| unmet_requires(component_key, r, &installed)).unwrap_or_default();

        DependencyCheck {
            component: component_key.to_string(),
            satisfied: issues.is_empty(),
            issues,
        }
    }

    /// `keys`에 코어가 있으면, 그중 지금 설치된 코어로는 `saba-core` requires를 채우지 못하는 모듈·익스텐션
    pub(crate) fn addons_waiting_for_the_new_core(&self, keys: &[String]) -> Vec<String> {
        let core = Component::CoreDaemon.manifest_key();
        if !keys.contains(&core) {
            return Vec::new();
        }
        let installed = self.dependency_versions();
        keys.iter()
            .filter(|k| matches!(Component::from_manifest_key(k), Component::Module(_) | Component::Extension(_)))
            .filter(|k| {
                let needs: HashMap<String, String> =
                    self.requires_of(k).and_then(|r| r.get(&core)).map(|min| HashMap::from([(core.clone(), min.clone())])).unwrap_or_default();
                !unmet_requires(k, &needs, &installed).is_empty()
            })
            .cloned()
            .collect()
    }

    /// 받아 둔 상태의 requires (pending.json에서 읽은 업데이터 `--apply`도), 없으면 캐시된 서버 매니페스트
    fn requires_of(&self, component_key: &str) -> Option<&HashMap<String, String>> {
        self.status.components.iter()
            .find(|c| c.component.manifest_key() == component_key)
            .and_then(|c| c.requires.as_ref())
            .or_else(|| self.cached_manifest.as_ref()?.components.get(component_key)?.requires.as_ref())
    }

    /// 적용 순서: 종류별 우선순위로 세운 뒤, 같은 묶음 안에서 requires하는 것을 앞으로 당긴다 (그 밖은 순서 유지).
    /// 순환하면 남은 것 중 맨 앞부터 적용한다
    pub fn apply_order(&self, keys: &[String]) -> Vec<String> {
        let mut rest = keys.to_vec();
        // 순위는 GUI·CLI 실행 여부를 프로세스 목록에서 읽으므로 키마다 한 번만 구한다
        rest.sort_by_cached_key(|k| Self::component_apply_priority(&Component::from_manifest_key(k)));
        let mut ordered = Vec::with_capacity(rest.len());
        while !rest.is_empty() {
            let waits = |key: &String| {
                self.requires_of(key).is_some_and(|r| r.keys().any(|dep| dep != key && rest.contains(dep)))
            };
            let next = rest.iter().position(|k| !waits(k)).unwrap_or(0);
            ordered.push(rest.remove(next));
        }
        ordered
    }

    /// 단일 컴포넌트만 개별적으로 적용하는 메서드 (데몬 IPC 경유 시 개별 컴포넌트를 순차적으로 처리)
    ///
    /// Flow 1 (백그라운드 워커): IPC 커맨드를 통해 데몬이 직접 적용한 후 재시작
    /// Flow 2 (GUI/CLI): 직접 적용, self-update flow로 전환
    pub async fn apply_single_component(&mut self, component: &Component) -> Result<ApplyComponentResult> {
        self.apply_single_component_with(component, false).await
    }

    /// 묻지 않는 모듈·익스텐션 교체: 교체 순간의 폴더가 카탈로그에서 깐 그대로일 때만 바꾼다 (`integrity::matches_install_record`).
    /// 사용자가 넣거나 고친 파일이 있으면 폴더를 건드리지 않고 실패로 돌려준다
    pub async fn apply_untouched_addon(&mut self, component: &Component) -> Result<ApplyComponentResult> {
        if !matches!(component, Component::Module(_) | Component::Extension(_)) {
            anyhow::bail!("{} is not a module or an extension", component.manifest_key());
        }
        self.apply_single_component_with(component, true).await
    }

    async fn apply_single_component_with(&mut self, component: &Component, untouched_only: bool) -> Result<ApplyComponentResult> {
        // 업데이터·데몬의 개별 적용도 지난 적용이 남긴 것부터 정리한다 (같은 프로세스가 적용하므로 안전하다)
        self.settle_interrupted_updates();
        let comp = self.status.components.iter()
            .find(|c| &c.component == component && c.downloaded && c.update_available)
            .cloned()
            .ok_or_else(|| anyhow::anyhow!("Component {:?} is not ready for apply", component))?;

        // 같은 묶음의 코어는 먼저 적용되어(`component_apply_priority`) 설치 기록에 새 버전이 적혀 있다
        let deps = self.check_dependencies(&component.manifest_key());
        if !deps.satisfied {
            anyhow::bail!("{}", deps.issues.iter().map(|i| i.message.as_str()).collect::<Vec<_>>().join("; "));
        }
        let staged_path = Self::verified_staged_path(&comp)?;
        let files = if untouched_only { self.replace_untouched_addon(component, staged_path)? } else { self.apply_one(component, staged_path).await? };
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
                if let Err(e) = Self::update_installed_version_at(
                    &self.installed_manifest_file,
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
}
