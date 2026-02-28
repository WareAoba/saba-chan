//! 범용 익스텐션 시스템
//!
//! `extensions/` 디렉토리의 서브디렉토리를 스캔하여 `manifest.json`을 파싱하고,
//! 데몬/서버 수명주기 Hook을 Python 모듈로 디스패치합니다.

use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::collections::{HashMap, HashSet};
use std::path::PathBuf;
use saba_chan_updater_lib::version::SemVer;

/// npm package.json 스타일 dependencies 디시리얼라이저.
/// 배열 형식과 맵 형식 모두 지원:
/// - `["steamcmd", "ue4-ini"]` → `{"steamcmd": "*", "ue4-ini": "*"}`
/// - `{"steamcmd": ">=0.1.0", "saba-core": ">=0.3.0"}` → 그대로
fn deserialize_dependencies<'de, D>(deserializer: D) -> Result<HashMap<String, String>, D::Error>
where
    D: serde::Deserializer<'de>,
{
    use serde::de;

    struct DepsVisitor;

    impl<'de> de::Visitor<'de> for DepsVisitor {
        type Value = HashMap<String, String>;

        fn expecting(&self, formatter: &mut std::fmt::Formatter) -> std::fmt::Result {
            formatter.write_str(
                "a map of dependency names to version requirements, or an array of dependency names",
            )
        }

        fn visit_seq<A>(self, mut seq: A) -> Result<Self::Value, A::Error>
        where
            A: de::SeqAccess<'de>,
        {
            let mut map = HashMap::new();
            while let Some(name) = seq.next_element::<String>()? {
                map.insert(name, "*".to_string());
            }
            Ok(map)
        }

        fn visit_map<M>(self, mut access: M) -> Result<Self::Value, M::Error>
        where
            M: de::MapAccess<'de>,
        {
            let mut map = HashMap::new();
            while let Some((key, value)) = access.next_entry::<String, String>()? {
                map.insert(key, value);
            }
            Ok(map)
        }
    }

    deserializer.deserialize_any(DepsVisitor)
}

// ═══════════════════════════════════════════════════════════════
//  구조화된 에러 타입
// ═══════════════════════════════════════════════════════════════

/// 익스텐션 조작 시 발생할 수 있는 에러.
/// `error_code` 필드를 통해 GUI에서 종류별로 분기할 수 있음.
#[derive(Debug, Clone, Serialize)]
pub struct ExtensionError {
    /// 머신 판별용 코드 (예: "not_found", "dependency_missing", "dependency_not_enabled",
    /// "has_dependents", "in_use", "not_mounted", "id_mismatch", "manifest_not_found")
    pub error_code: String,
    /// 사람이 읽을 수 있는 메시지
    pub message: String,
    /// 관련 식별자 목록 (의존 익스텐션 ID, 인스턴스 이름 등)
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub related: Vec<String>,
}

impl std::fmt::Display for ExtensionError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", self.message)
    }
}

impl std::error::Error for ExtensionError {}

impl ExtensionError {
    fn not_found(ext_id: &str) -> Self {
        Self {
            error_code: "not_found".to_string(),
            message: format!("Extension '{}' not found in discovered extensions", ext_id),
            related: vec![ext_id.to_string()],
        }
    }
    fn dependency_missing(ext_id: &str, dep: &str) -> Self {
        Self {
            error_code: "dependency_missing".to_string(),
            message: format!("Cannot enable '{}': dependency '{}' is not mounted", ext_id, dep),
            related: vec![dep.to_string()],
        }
    }
    fn dependency_not_enabled(ext_id: &str, dep: &str) -> Self {
        Self {
            error_code: "dependency_not_enabled".to_string(),
            message: format!("Cannot enable '{}': dependency '{}' is not enabled", ext_id, dep),
            related: vec![dep.to_string()],
        }
    }
    fn component_version_unsatisfied(ext_id: &str, component: &str, required: &str, installed: Option<&str>) -> Self {
        Self {
            error_code: "component_version_unsatisfied".to_string(),
            message: format!(
                "Cannot enable '{}': requires {} {} but {} is installed",
                ext_id, component, required,
                installed.unwrap_or("not installed")
            ),
            related: vec![component.to_string(), required.to_string()],
        }
    }
    fn has_dependents(ext_id: &str, dependents: &[String]) -> Self {
        Self {
            error_code: "has_dependents".to_string(),
            message: format!(
                "Cannot disable/unmount '{}': depended on by active extension(s): {}",
                ext_id,
                dependents.join(", ")
            ),
            related: dependents.to_vec(),
        }
    }
    fn in_use(ext_id: &str, instances: &[String]) -> Self {
        Self {
            error_code: "in_use".to_string(),
            message: format!(
                "Cannot disable/unmount '{}': in use by instance(s): {}",
                ext_id,
                instances.join(", ")
            ),
            related: instances.to_vec(),
        }
    }
    fn not_mounted(ext_id: &str) -> Self {
        Self {
            error_code: "not_mounted".to_string(),
            message: format!("Extension '{}' is not mounted", ext_id),
            related: vec![ext_id.to_string()],
        }
    }
    fn manifest_not_found(path: &str) -> Self {
        Self {
            error_code: "manifest_not_found".to_string(),
            message: format!("Extension directory or manifest not found: {}", path),
            related: vec![],
        }
    }
    fn id_mismatch(manifest_id: &str, dir_name: &str) -> Self {
        Self {
            error_code: "id_mismatch".to_string(),
            message: format!(
                "Manifest id '{}' does not match directory name '{}'",
                manifest_id, dir_name
            ),
            related: vec![manifest_id.to_string(), dir_name.to_string()],
        }
    }
}

// ═══════════════════════════════════════════════════════════════
//  Manifest 타입 정의
// ═══════════════════════════════════════════════════════════════

/// 익스텐션 매니페스트 — manifest.json을 역직렬화한 것
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ExtensionManifest {
    pub id: String,
    pub name: String,
    pub version: String,
    #[serde(default)]
    pub description: String,
    #[serde(default)]
    pub author: String,
    #[serde(default)]
    pub min_app_version: Option<String>,
    /// npm package.json 스타일 의존성 선언.
    /// 익스텐션·컴포넌트 ID를 키로, 버전 요구사항을 값으로 사용.
    /// 예: `{ "steamcmd": ">=0.1.0", "saba-core": ">=0.3.0" }`
    /// 배열 형식(`["steamcmd"]`)도 하위 호환으로 지원 → `{ "steamcmd": "*" }`로 변환.
    #[serde(default, deserialize_with = "deserialize_dependencies")]
    pub dependencies: HashMap<String, String>,
    #[serde(default)]
    pub python_modules: HashMap<String, String>, // name → relative path
    #[serde(default)]
    pub hooks: HashMap<String, HookBinding>, // hook_name → binding
    #[serde(default)]
    pub gui: Option<GuiManifest>,
    /// CLI TUI 슬롯 선언 (GUI의 gui.slots에 대응)
    #[serde(default)]
    pub cli: Option<CliManifest>,
    /// 이 익스텐션이 관할하는 module.toml 섹션명 (예: 컨테이너 격리 익스텐션)
    #[serde(default)]
    pub module_config_section: Option<String>,
    #[serde(default)]
    pub instance_fields: HashMap<String, FieldDef>,
    #[serde(default)]
    pub i18n_dir: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct HookBinding {
    pub module: String,   // python_modules의 키
    pub function: String, // Python 함수명
    #[serde(default)]
    pub condition: Option<String>, // "instance.ext_data.docker_enabled"
    #[serde(default, rename = "async")]
    pub is_async: Option<bool>, // true면 tokio::spawn으로 백그라운드 실행
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct GuiManifest {
    #[serde(default)]
    pub bundle: Option<String>,
    #[serde(default)]
    pub styles: Option<String>,
    #[serde(default)]
    pub builtin: Option<bool>,
    #[serde(default)]
    pub slots: HashMap<String, String>, // slot_id → component_name
}

/// CLI 매니페스트 — GUI의 GuiManifest에 대응하는 TUI 슬롯 선언
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CliManifest {
    /// slot_id → 슬롯별 JSON 설정 (데이터 기반 렌더링)
    /// 예: "InstanceList.badge" → { "text": "🐳", "condition": "..." }
    #[serde(default)]
    pub slots: HashMap<String, Value>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FieldDef {
    #[serde(rename = "type")]
    pub field_type: String,
    #[serde(default)]
    pub default: Option<Value>,
    #[serde(default)]
    pub optional: Option<bool>,
}

// ═══════════════════════════════════════════════════════════════
//  발견된 익스텐션
// ═══════════════════════════════════════════════════════════════

/// 발견된 익스텐션 정보 (manifest + 디렉토리 경로)
#[derive(Debug, Clone)]
pub struct DiscoveredExtension {
    pub manifest: ExtensionManifest,
    pub dir: PathBuf,
}

// ═══════════════════════════════════════════════════════════════
//  API 응답용 타입
// ═══════════════════════════════════════════════════════════════

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ExtensionListItem {
    pub id: String,
    pub name: String,
    pub version: String,
    pub description: String,
    pub author: String,
    pub enabled: bool,
    pub hooks: Vec<String>,
    /// npm package.json 스타일 의존성 (이름 → 버전 요구사항)
    pub dependencies: HashMap<String, String>,
    pub gui: Option<GuiManifest>,
    pub cli: Option<CliManifest>,
    pub instance_fields: HashMap<String, FieldDef>,
    /// 익스텐션 디렉토리에 icon.png가 존재하는지 여부
    #[serde(default)]
    pub has_icon: bool,
}

// ═══════════════════════════════════════════════════════════════
//  원격 레지스트리 타입 정의
// ═══════════════════════════════════════════════════════════════

/// GitHub 원격 레지스트리에서 가져온 익스텐션 항목
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RemoteExtensionInfo {
    pub id: String,
    pub name: String,
    pub version: String,
    #[serde(default)]
    pub description: String,
    #[serde(default)]
    pub author: String,
    /// 배포 패키지 다운로드 URL (.zip)
    pub download_url: String,
    /// 패키지 SHA-256 체크섬 (검증용, null 허용)
    #[serde(default)]
    pub sha256: Option<String>,
    /// 최소 앱 버전 요구사항
    #[serde(default)]
    pub min_app_version: Option<String>,
    #[serde(default)]
    pub tags: Vec<String>,
    #[serde(default)]
    pub homepage: Option<String>,
}

/// 원격 레지스트리 응답 전체 형식
///
/// registry.json 예시:
/// ```json
/// {
///   "registry_version": "1",
///   "extensions": [...]
/// }
/// ```
#[allow(dead_code)]
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ExtensionRegistryResponse {
    #[serde(default)]
    pub registry_version: String,
    pub extensions: Vec<RemoteExtensionInfo>,
}

/// 업데이트 가용 정보 (로컬 버전 vs 원격 버전 비교 결과)
///
/// 업데이터의 `ComponentVersion`과 구조를 맞추어 향후 통합을 용이하게 함.
#[derive(Debug, Clone, Serialize)]
pub struct ExtensionUpdateInfo {
    pub id: String,
    pub installed_version: String,
    pub latest_version: String,
    pub download_url: String,
    /// 다운로드 완료 여부 (현재는 항상 false — 다운로드 큐 구현 시 활용)
    pub downloaded: bool,
    /// 적용(설치) 완료 여부
    pub installed: bool,
}

// ═══════════════════════════════════════════════════════════════
//  ExtensionManager
// ═══════════════════════════════════════════════════════════════

/// 원격 레지스트리 기본 URL (레포지토리 미완성 — 토대만)
const DEFAULT_REGISTRY_URL: &str =
    "https://raw.githubusercontent.com/WareAoba/saba-chan-extensions/main/registry.json";

pub struct ExtensionManager {
    extensions_dir: PathBuf,
    discovered: HashMap<String, DiscoveredExtension>,
    enabled: HashSet<String>,
    state_path: PathBuf,
    /// 원격 레지스트리 URL (커스텀 오버라이드 가능)
    pub registry_url: String,
}

#[allow(dead_code)]
impl ExtensionManager {
    /// 새 ExtensionManager 생성. `extensions_dir`은 `extensions/` 디렉토리 경로.
    pub fn new(extensions_dir: &str) -> Self {
        Self::with_state_path(extensions_dir, Self::resolve_state_path())
    }

    /// 커스텀 state 경로를 지정한 생성자 (테스트 격리용)
    #[cfg(test)]
    pub fn new_isolated(extensions_dir: &str) -> Self {
        let state_path = PathBuf::from(extensions_dir).join(".extensions_state.json");
        Self::with_state_path(extensions_dir, state_path)
    }

    fn with_state_path(extensions_dir: &str, state_path: PathBuf) -> Self {
        let extensions_dir = PathBuf::from(extensions_dir);

        // extensions/ 디렉토리가 없으면 생성 (최초 실행 대응)
        if !extensions_dir.exists() {
            if let Err(e) = std::fs::create_dir_all(&extensions_dir) {
                tracing::warn!("Failed to create extensions directory: {}", e);
            }
        }

        let mut mgr = Self {
            extensions_dir,
            discovered: HashMap::new(),
            enabled: HashSet::new(),
            state_path,
            registry_url: DEFAULT_REGISTRY_URL.to_string(),
        };
        mgr.load_state();
        mgr
    }

    /// %APPDATA%/saba-chan/extensions_state.json 경로 해석
    fn resolve_state_path() -> PathBuf {
        if let Ok(appdata) = std::env::var("APPDATA") {
            PathBuf::from(appdata)
                .join("saba-chan")
                .join("extensions_state.json")
        } else {
            PathBuf::from("./extensions_state.json")
        }
    }

    /// extensions/ 디렉토리를 스캔하여 익스텐션 발견.
    ///
    /// 지원 형식:
    /// - **폴더형**: `<id>/manifest.json` (현재 방식)
    /// - **단일 파일형**: `<id>.zip` → 자동 압축 해제 후 폴더형으로 등록
    pub fn discover(&mut self) -> Result<Vec<String>> {
        let mut found = Vec::new();

        if !self.extensions_dir.is_dir() {
            tracing::warn!(
                "Extensions directory does not exist: {}",
                self.extensions_dir.display()
            );
            return Ok(found);
        }

        let entries = std::fs::read_dir(&self.extensions_dir)
            .with_context(|| {
                format!(
                    "Failed to read extensions directory: {}",
                    self.extensions_dir.display()
                )
            })?;

        for entry in entries {
            let entry = match entry {
                Ok(e) => e,
                Err(e) => {
                    tracing::warn!("Failed to read directory entry: {}", e);
                    continue;
                }
            };

            let path = entry.path();

            // ── 단일 파일형: .zip 자동 압축 해제 ──────────────────────
            if path.is_file() {
                if path.extension().and_then(|e| e.to_str()) == Some("zip") {
                    match self.extract_zip_extension(&path) {
                        Ok(Some(ext_id)) => {
                            tracing::info!("Auto-extracted zip extension: {}", ext_id);
                        }
                        Ok(None) => {} // 이미 폴더가 존재하는 경우 스킵
                        Err(e) => {
                            tracing::warn!(
                                "Failed to extract zip extension {}: {}",
                                path.display(), e
                            );
                        }
                    }
                }
                // .zip 이외의 단일 파일은 무시
                continue;
            }

            if !path.is_dir() {
                continue;
            }

            // ── 폴더형: manifest.json 탐색 ──────────────────────────
            let manifest_path = path.join("manifest.json");
            if !manifest_path.exists() {
                continue;
            }

            match self.load_manifest(&manifest_path) {
                Ok(manifest) => {
                    let id = manifest.id.clone();
                    tracing::info!(
                        "Discovered extension: {} v{} ({})",
                        manifest.name,
                        manifest.version,
                        id
                    );
                    self.discovered.insert(
                        id.clone(),
                        DiscoveredExtension {
                            manifest,
                            dir: path,
                        },
                    );
                    found.push(id);
                }
                Err(e) => {
                    tracing::warn!(
                        "Failed to load manifest from {}: {}",
                        manifest_path.display(),
                        e
                    );
                }
            }
        }

        // zip에서 새로 추출된 익스텐션을 재스캔하여 등록
        let newly_extracted = self.rescan_extracted()?;
        found.extend(newly_extracted);

        tracing::info!("Extension discovery complete: {} found", found.len());
        Ok(found)
    }

    /// `.zip` 파일을 같은 이름의 폴더로 압축 해제.
    /// 이미 폴더가 있으면 None 반환 (스킵).
    fn extract_zip_extension(&self, zip_path: &std::path::Path) -> Result<Option<String>> {
        let stem = zip_path
            .file_stem()
            .and_then(|s| s.to_str())
            .ok_or_else(|| anyhow::anyhow!("Invalid zip filename: {}", zip_path.display()))?;

        let dest = self.extensions_dir.join(stem);
        if dest.is_dir() {
            // 이미 추출된 폴더 존재 → zip 파일 삭제 후 스킵
            if let Err(e) = std::fs::remove_file(zip_path) {
                tracing::warn!("Failed to remove zip after extraction: {}", e);
            }
            return Ok(None);
        }

        let file = std::fs::File::open(zip_path)
            .with_context(|| format!("Failed to open zip: {}", zip_path.display()))?;
        let mut archive = zip::ZipArchive::new(file)
            .with_context(|| format!("Failed to read zip archive: {}", zip_path.display()))?;

        for i in 0..archive.len() {
            let mut zip_file = archive.by_index(i)?;
            let outpath = match zip_file.enclosed_name() {
                Some(p) => dest.join(p),
                None => continue,
            };
            if zip_file.is_dir() {
                std::fs::create_dir_all(&outpath)?;
            } else {
                if let Some(parent) = outpath.parent() {
                    std::fs::create_dir_all(parent)?;
                }
                let mut outfile = std::fs::File::create(&outpath)
                    .with_context(|| format!("Failed to create {}", outpath.display()))?;
                std::io::copy(&mut zip_file, &mut outfile)
                    .with_context(|| format!("Failed to write {}", outpath.display()))?;
            }
        }

        // 성공 후 zip 파일 제거
        if let Err(e) = std::fs::remove_file(zip_path) {
            tracing::warn!("Failed to remove zip after extraction: {}", e);
        }

        tracing::info!("Extracted zip extension '{}' to {}", stem, dest.display());
        Ok(Some(stem.to_string()))
    }

    /// 방금 추출된 폴더들의 manifest를 로드하여 discovered에 추가 (내부용)
    fn rescan_extracted(&mut self) -> Result<Vec<String>> {
        let mut newly_found = Vec::new();
        if !self.extensions_dir.is_dir() {
            return Ok(newly_found);
        }
        let entries = std::fs::read_dir(&self.extensions_dir)
            .with_context(|| format!("Failed to read extensions directory: {}", self.extensions_dir.display()))?;
        for entry in entries {
            let entry = match entry {
                Ok(e) => e,
                Err(_) => continue,
            };
            let path = entry.path();
            if !path.is_dir() { continue; }
            let manifest_path = path.join("manifest.json");
            if !manifest_path.exists() { continue; }
            match self.load_manifest(&manifest_path) {
                Ok(manifest) => {
                    let id = manifest.id.clone();
                    if !self.discovered.contains_key(&id) {
                        self.discovered.insert(id.clone(), DiscoveredExtension { manifest, dir: path });
                        newly_found.push(id);
                    }
                }
                Err(e) => {
                    tracing::warn!("Failed to load manifest {}: {}", manifest_path.display(), e);
                }
            }
        }
        Ok(newly_found)
    }

    fn load_manifest(&self, path: &std::path::Path) -> Result<ExtensionManifest> {
        let content = std::fs::read_to_string(path)
            .with_context(|| format!("Failed to read {}", path.display()))?;
        let manifest: ExtensionManifest = serde_json::from_str(&content)
            .with_context(|| format!("Failed to parse {}", path.display()))?;
        Ok(manifest)
    }

    // ═══════════════════════════════════════════════════════════════
    //  동적 마운트/언마운트 (재시작 불필요)
    // ═══════════════════════════════════════════════════════════════

    /// 런타임 중 extensions/ 디렉토리를 재스캔하여 새로 추가된 익스텐션 발견.
    /// 기존에 이미 발견된 익스텐션은 매니페스트를 리로드(갱신),
    /// 디스크에서 제거된 (unmount 아닌) 익스텐션은 유지.
    pub fn rescan(&mut self) -> Result<Vec<String>> {
        let mut newly_found = Vec::new();

        if !self.extensions_dir.is_dir() {
            return Ok(newly_found);
        }

        let entries = std::fs::read_dir(&self.extensions_dir)
            .with_context(|| format!("Failed to read extensions directory: {}", self.extensions_dir.display()))?;

        for entry in entries {
            let entry = match entry {
                Ok(e) => e,
                Err(_) => continue,
            };

            let path = entry.path();
            if !path.is_dir() {
                continue;
            }

            let manifest_path = path.join("manifest.json");
            if !manifest_path.exists() {
                continue;
            }

            match self.load_manifest(&manifest_path) {
                Ok(manifest) => {
                    let id = manifest.id.clone();
                    let is_new = !self.discovered.contains_key(&id);
                    self.discovered.insert(
                        id.clone(),
                        DiscoveredExtension { manifest, dir: path },
                    );
                    if is_new {
                        tracing::info!("Rescan: newly discovered extension '{}'", id);
                        newly_found.push(id);
                    } else {
                        tracing::debug!("Rescan: reloaded manifest for '{}'", id);
                    }
                }
                Err(e) => {
                    tracing::warn!("Failed to load manifest {}: {}", manifest_path.display(), e);
                }
            }
        }

        tracing::info!("Rescan complete: {} new extension(s)", newly_found.len());
        Ok(newly_found)
    }

    /// 단일 익스텐션을 핫 마운트 (디스크에서 로드 → discovered에 추가).
    /// 이미 존재하면 매니페스트를 리로드.
    pub fn mount(&mut self, ext_id: &str) -> Result<()> {
        let ext_path = self.extensions_dir.join(ext_id);
        let manifest_path = ext_path.join("manifest.json");

        if !manifest_path.exists() {
            return Err(ExtensionError::manifest_not_found(
                &manifest_path.display().to_string(),
            ).into());
        }

        let manifest = self.load_manifest(&manifest_path)?;
        if manifest.id != ext_id {
            return Err(ExtensionError::id_mismatch(&manifest.id, ext_id).into());
        }

        tracing::info!("Mounted extension: {} v{}", manifest.name, manifest.version);
        self.discovered.insert(
            ext_id.to_string(),
            DiscoveredExtension { manifest, dir: ext_path },
        );
        Ok(())
    }

    /// 익스텐션 언마운트 (discovered + enabled에서 제거).
    /// 다른 활성 익스텐션이 이 익스텐션에 의존하면 실패.
    /// `active_ext_data`는 현재 존재하는 인스턴스들의 extension_data 목록 —
    /// 인스턴스가 이 익스텐션을 사용 중이면 실패.
    pub fn unmount(
        &mut self,
        ext_id: &str,
        active_ext_data: &[(&str, &HashMap<String, Value>)],
    ) -> Result<()> {
        if !self.discovered.contains_key(ext_id) {
            return Err(ExtensionError::not_mounted(ext_id).into());
        }

        // 역의존성 검사: 다른 활성 익스텐션이 이 익스텐션을 dependency로 선언했는지
        let dependents = self.dependents_of(ext_id);
        if !dependents.is_empty() {
            return Err(ExtensionError::has_dependents(ext_id, &dependents).into());
        }

        // 인스턴스 사용 여부 검사
        let using = self.instances_using_ext(ext_id, active_ext_data);
        if !using.is_empty() {
            return Err(ExtensionError::in_use(ext_id, &using).into());
        }

        self.enabled.remove(ext_id);
        self.discovered.remove(ext_id);
        self.save_state();
        tracing::info!("Unmounted extension: {}", ext_id);
        Ok(())
    }

    // ═══════════════════════════════════════════════════════════════
    //  의존성 검증
    // ═══════════════════════════════════════════════════════════════

    /// ext_id를 dependency로 선언한 *활성* 익스텐션 목록
    fn dependents_of(&self, ext_id: &str) -> Vec<String> {
        self.discovered
            .values()
            .filter(|ext| {
                self.enabled.contains(&ext.manifest.id)
                    && ext.manifest.dependencies.contains_key(ext_id)
            })
            .map(|ext| ext.manifest.id.clone())
            .collect()
    }

    /// 인스턴스의 extension_data에서 이 익스텐션이 선언한 instance_fields를
    /// 하나라도 truthy 값으로 사용하는 인스턴스 이름 목록.
    fn instances_using_ext(
        &self,
        ext_id: &str,
        active_ext_data: &[(&str, &HashMap<String, Value>)],
    ) -> Vec<String> {
        let ext = match self.discovered.get(ext_id) {
            Some(e) => e,
            None => return Vec::new(),
        };

        let field_keys: Vec<&String> = ext.manifest.instance_fields.keys().collect();
        if field_keys.is_empty() {
            return Vec::new();
        }

        active_ext_data
            .iter()
            .filter(|(_, data)| {
                field_keys.iter().any(|key| {
                    matches!(
                        data.get(*key),
                        Some(Value::Bool(true))
                            | Some(Value::String(_))
                            | Some(Value::Number(_))
                            | Some(Value::Object(_))
                            | Some(Value::Array(_))
                    )
                })
            })
            .map(|(name, _)| name.to_string())
            .collect()
    }

    /// 익스텐션 활성화 — 통합 dependencies 맵에서 의존성 전부 검증.
    pub fn enable(&mut self, ext_id: &str) -> Result<()> {
        self.enable_with_versions(ext_id, &HashMap::new())
    }

    /// 컴포넌트 버전 정보를 함께 받아 dependencies를 검증하면서 활성화.
    /// `installed_versions`: 컴포넌트 키 → 설치된 버전 (예: "saba-core" → "0.3.0")
    ///
    /// dependencies 맵의 각 키를 먼저 discovered 익스텐션에서 찾고,
    /// 있으면 익스텐션 의존성(마운트+활성화+버전)으로, 없으면 컴포넌트 의존성(설치 버전)으로 처리.
    pub fn enable_with_versions(
        &mut self,
        ext_id: &str,
        installed_versions: &HashMap<String, String>,
    ) -> Result<()> {
        if !self.discovered.contains_key(ext_id) {
            return Err(ExtensionError::not_found(ext_id).into());
        }

        let deps = self.discovered[ext_id].manifest.dependencies.clone();
        for (dep_key, version_req) in &deps {
            if let Some(dep_ext) = self.discovered.get(dep_key) {
                // ── 익스텐션 의존성: discovered에 있으면 ext dep ──
                if !self.enabled.contains(dep_key) {
                    return Err(
                        ExtensionError::dependency_not_enabled(ext_id, dep_key).into()
                    );
                }
                // 버전 검증 ("*"면 스킵)
                if version_req != "*" {
                    let min_clean = version_req.trim_start_matches(">=").trim();
                    let satisfied = match (SemVer::parse(&dep_ext.manifest.version), SemVer::parse(min_clean)) {
                        (Some(iv), Some(rv)) => iv >= rv,
                        _ => false,
                    };
                    if !satisfied {
                        return Err(
                            ExtensionError::component_version_unsatisfied(
                                ext_id, dep_key, version_req,
                                Some(&dep_ext.manifest.version),
                            ).into()
                        );
                    }
                }
            } else {
                // ── 비-익스텐션 컴포넌트 의존성 ──
                if version_req == "*" {
                    // 이름만 선언 → discovered에 없으면 마운트 안 된 익스텐션으로 간주
                    return Err(
                        ExtensionError::dependency_missing(ext_id, dep_key).into()
                    );
                }
                let min_clean = version_req.trim_start_matches(">=").trim();
                let installed = installed_versions.get(dep_key);
                let satisfied = installed.is_some_and(|v| {
                    match (SemVer::parse(v), SemVer::parse(min_clean)) {
                        (Some(iv), Some(rv)) => iv >= rv,
                        _ => false,
                    }
                });
                if !satisfied {
                    return Err(
                        ExtensionError::component_version_unsatisfied(
                            ext_id, dep_key, version_req,
                            installed.map(|s| s.as_str()),
                        ).into()
                    );
                }
            }
        }

        self.enabled.insert(ext_id.to_string());
        self.save_state();
        tracing::info!("Extension enabled: {}", ext_id);
        Ok(())
    }

    /// 익스텐션 비활성화 — 역의존성 검사 + 인스턴스 사용 여부 검사
    pub fn disable(
        &mut self,
        ext_id: &str,
        active_ext_data: &[(&str, &HashMap<String, Value>)],
    ) -> Result<()> {
        if !self.enabled.contains(ext_id) {
            // 이미 비활성 → no-op
            return Ok(());
        }

        // 역의존성 검사
        let dependents = self.dependents_of(ext_id);
        if !dependents.is_empty() {
            return Err(ExtensionError::has_dependents(ext_id, &dependents).into());
        }

        // 인스턴스 사용 여부 검사
        let using = self.instances_using_ext(ext_id, active_ext_data);
        if !using.is_empty() {
            return Err(ExtensionError::in_use(ext_id, &using).into());
        }

        self.enabled.remove(ext_id);
        self.save_state();
        tracing::info!("Extension disabled: {}", ext_id);
        Ok(())
    }

    /// 강제 비활성화 (인스턴스/의존성 무시) — 내부 마이그레이션/관리용
    pub fn force_disable(&mut self, ext_id: &str) {
        self.enabled.remove(ext_id);
        self.save_state();
        tracing::warn!("Extension force-disabled: {}", ext_id);
    }

    /// 익스텐션 제거 — 비활성화 후 디렉토리 삭제
    pub fn remove(
        &mut self,
        ext_id: &str,
        active_ext_data: &[(&str, &HashMap<String, Value>)],
    ) -> Result<()> {
        // 발견된 익스텐션인지 확인
        if !self.discovered.contains_key(ext_id) {
            return Err(ExtensionError::not_found(ext_id).into());
        }
        // 활성화 상태면 먼저 비활성화 (의존성·인스턴스 사용 검사 포함)
        if self.enabled.contains(ext_id) {
            self.disable(ext_id, active_ext_data)?;
        }
        // 발견 목록에서 제거
        self.discovered.remove(ext_id);
        // 디렉토리 삭제
        let ext_path = self.extensions_dir.join(ext_id);
        if ext_path.exists() {
            std::fs::remove_dir_all(&ext_path)
                .with_context(|| format!("Failed to remove extension directory: {}", ext_path.display()))?;
        }
        self.save_state();
        tracing::info!("Extension removed: {}", ext_id);
        Ok(())
    }

    /// 활성 여부 확인
    pub fn is_enabled(&self, ext_id: &str) -> bool {
        self.enabled.contains(ext_id)
    }

    /// 현재 활성화된 익스텐션 ID 집합의 복제본을 반환합니다.
    pub fn enabled_set(&self) -> HashSet<String> {
        self.enabled.clone()
    }

    /// 발견된 전체 익스텐션 목록 (활성 상태 포함)
    pub fn list(&self) -> Vec<ExtensionListItem> {
        self.discovered
            .values()
            .map(|ext| {
                let m = &ext.manifest;
                let has_icon = ext.dir.join("icon.png").is_file();
                ExtensionListItem {
                    id: m.id.clone(),
                    name: m.name.clone(),
                    version: m.version.clone(),
                    description: m.description.clone(),
                    author: m.author.clone(),
                    enabled: self.enabled.contains(&m.id),
                    hooks: m.hooks.keys().cloned().collect(),
                    dependencies: m.dependencies.clone(),
                    gui: m.gui.clone(),
                    cli: m.cli.clone(),
                    instance_fields: m.instance_fields.clone(),
                    has_icon,
                }
            })
            .collect()
    }

    /// 지정된 hook에 바인딩된 활성 익스텐션 목록
    pub fn hooks_for(&self, hook_name: &str) -> Vec<(&DiscoveredExtension, &HookBinding)> {
        let mut result = Vec::new();
        for ext in self.discovered.values() {
            if !self.enabled.contains(&ext.manifest.id) {
                continue;
            }
            if let Some(binding) = ext.manifest.hooks.get(hook_name) {
                result.push((ext, binding));
            }
        }
        result
    }

    /// 조건 문자열 평가: "instance.ext_data.<key>" → ext_data[key] == true
    pub fn evaluate_condition(
        condition: &str,
        ext_data: &HashMap<String, Value>,
    ) -> bool {
        // "instance.ext_data.<key>" 패턴
        if let Some(key) = condition.strip_prefix("instance.ext_data.") {
            match ext_data.get(key) {
                Some(Value::Bool(b)) => *b,
                Some(Value::Number(n)) => n.as_f64().map(|v| v != 0.0).unwrap_or(false),
                Some(Value::String(s)) => !s.is_empty(),
                _ => false,
            }
        } else {
            tracing::warn!("Unknown condition pattern: {}", condition);
            false
        }
    }

    /// Hook 디스패치: 조건 평가 → run_plugin 호출 → handled 체크
    ///
    /// 반환: Vec<(ext_id, Result<Value>)>
    /// handled=true가 나오면 이후 익스텐션은 스킵 (chain-of-responsibility)
    pub async fn dispatch_hook(
        &self,
        hook_name: &str,
        context: Value,
    ) -> Vec<(String, Result<Value>)> {
        self.dispatch_hook_timed(hook_name, context, crate::plugin::DEFAULT_PLUGIN_TIMEOUT_SECS).await
    }

    /// 타임아웃 지정 가능한 hook 디스패치 (server.list_enrich 등 빠른 반환이 필요한 hook용)
    pub async fn dispatch_hook_timed(
        &self,
        hook_name: &str,
        context: Value,
        timeout_secs: u64,
    ) -> Vec<(String, Result<Value>)> {
        let hooks = self.hooks_for(hook_name);
        if hooks.is_empty() {
            return Vec::new();
        }

        let ext_data: HashMap<String, Value> = context
            .get("extension_data")
            .and_then(|v| serde_json::from_value(v.clone()).ok())
            .unwrap_or_default();

        let mut results = Vec::new();

        for (ext, binding) in hooks {
            // 조건 평가
            if let Some(ref cond) = binding.condition {
                if !Self::evaluate_condition(cond, &ext_data) {
                    continue;
                }
            }

            // Python 모듈 절대 경로 해석
            let module_file = match ext.manifest.python_modules.get(&binding.module) {
                Some(rel_path) => ext.dir.join(rel_path),
                None => {
                    tracing::error!(
                        "Extension '{}' hook '{}' references unknown module '{}'",
                        ext.manifest.id,
                        hook_name,
                        binding.module
                    );
                    results.push((
                        ext.manifest.id.clone(),
                        Err(anyhow::anyhow!(
                            "Unknown python module: {}",
                            binding.module
                        )),
                    ));
                    continue;
                }
            };

            let module_path = module_file.to_string_lossy().to_string();
            tracing::debug!(
                "Dispatching hook '{}' → ext '{}' → {}::{}",
                hook_name,
                ext.manifest.id,
                binding.module,
                binding.function
            );

            let result = crate::plugin::run_plugin_with_timeout(
                &module_path,
                &binding.function,
                context.clone(),
                timeout_secs,
            )
            .await;

            match &result {
                Ok(val) => {
                    tracing::debug!(
                        "Hook '{}' ext '{}' returned: {}",
                        hook_name,
                        ext.manifest.id,
                        serde_json::to_string(val).unwrap_or_default()
                    );
                    results.push((ext.manifest.id.clone(), Ok(val.clone())));

                    // handled=true → chain 종료
                    if val
                        .get("handled")
                        .and_then(|h| h.as_bool())
                        == Some(true)
                    {
                        tracing::debug!(
                            "Hook '{}' handled by extension '{}', skipping remaining",
                            hook_name,
                            ext.manifest.id
                        );
                        break;
                    }
                }
                Err(e) => {
                    tracing::error!(
                        "Hook '{}' ext '{}' failed: {}",
                        hook_name,
                        ext.manifest.id,
                        e
                    );
                    results.push((
                        ext.manifest.id.clone(),
                        Err(anyhow::anyhow!("Hook dispatch failed: {}", e)),
                    ));
                    // 에러 시 graceful degradation — 기본 동작 진행을 위해 계속
                }
            }
        }

        results
    }

    /// Hook 디스패치 + 진행률 콜백 (server.post_create 등 장시간 hook용)
    pub async fn dispatch_hook_with_progress<F>(
        &self,
        hook_name: &str,
        context: Value,
        on_progress: F,
    ) -> Vec<(String, Result<Value>)>
    where
        F: Fn(crate::plugin::ExtensionProgress) + Send + 'static,
    {
        let hooks = self.hooks_for(hook_name);
        if hooks.is_empty() {
            tracing::warn!("dispatch_hook_with_progress('{}') — no hooks registered (enabled: {:?})", hook_name, self.enabled);
            return Vec::new();
        }

        let ext_data: HashMap<String, Value> = context
            .get("extension_data")
            .and_then(|v| serde_json::from_value(v.clone()).ok())
            .unwrap_or_default();

        tracing::info!("dispatch_hook_with_progress('{}') — {} hook(s), ext_data keys: {:?}", hook_name, hooks.len(), ext_data.keys().collect::<Vec<_>>());

        let mut results = Vec::new();

        for (ext, binding) in hooks {
            if let Some(ref cond) = binding.condition {
                if !Self::evaluate_condition(cond, &ext_data) {
                    tracing::warn!("Hook '{}' from '{}' skipped: condition '{}' evaluated to false (ext_data: {:?})", hook_name, ext.manifest.id, cond, ext_data);
                    continue;
                }
            }

            let module_file = match ext.manifest.python_modules.get(&binding.module) {
                Some(rel_path) => ext.dir.join(rel_path),
                None => {
                    results.push((
                        ext.manifest.id.clone(),
                        Err(anyhow::anyhow!(
                            "Unknown python module: {}",
                            binding.module
                        )),
                    ));
                    continue;
                }
            };

            let module_path = module_file.to_string_lossy().to_string();

            let result = crate::plugin::run_plugin_with_progress(
                &module_path,
                &binding.function,
                context.clone(),
                on_progress,
            )
            .await;

            match &result {
                Ok(val) => {
                    results.push((ext.manifest.id.clone(), Ok(val.clone())));
                    if val.get("handled").and_then(|h| h.as_bool()) == Some(true) {
                        break;
                    }
                }
                Err(e) => {
                    results.push((
                        ext.manifest.id.clone(),
                        Err(anyhow::anyhow!("Hook dispatch failed: {}", e)),
                    ));
                }
            }

            // progress 콜백은 한 번만 소비 가능하므로 첫 번째 익스텐션만 progress 지원
            break;
        }

        results
    }

    /// 해당 config 섹션명을 관할하는 활성 익스텐션이 있는지
    pub fn should_parse_config_section(&self, section: &str) -> bool {
        self.discovered.values().any(|ext| {
            self.enabled.contains(&ext.manifest.id)
                && ext.manifest.module_config_section.as_deref() == Some(section)
        })
    }

    /// 활성 익스텐션의 instance_fields를 합산
    pub fn all_instance_fields(&self) -> HashMap<String, FieldDef> {
        let mut fields = HashMap::new();
        for ext in self.discovered.values() {
            if self.enabled.contains(&ext.manifest.id) {
                for (k, v) in &ext.manifest.instance_fields {
                    fields.insert(k.clone(), v.clone());
                }
            }
        }
        fields
    }

    /// 활성 익스텐션의 GUI 매니페스트 목록
    pub fn gui_manifests(&self) -> Vec<(&str, &GuiManifest)> {
        self.discovered
            .values()
            .filter(|ext| self.enabled.contains(&ext.manifest.id))
            .filter_map(|ext| {
                ext.manifest
                    .gui
                    .as_ref()
                    .map(|gui| (ext.manifest.id.as_str(), gui))
            })
            .collect()
    }

    /// 익스텐션 파일 절대 경로
    pub fn extension_file_path(&self, ext_id: &str, relative: &str) -> Option<PathBuf> {
        self.discovered.get(ext_id).map(|ext| ext.dir.join(relative))
    }

    // ═══════════════════════════════════════════════════════════════
    //  원격 레지스트리 & 버전 관리
    // ═══════════════════════════════════════════════════════════════

    /// 원격 레지스트리 URL을 커스텀 주소로 오버라이드
    pub fn set_registry_url(&mut self, url: &str) {
        self.registry_url = url.to_string();
    }

    /// 원격 레지스트리에서 가용 익스텐션 목록을 페치합니다.
    ///
    /// ⚠️  레포지토리 미완성 — 현재는 빈 목록을 반환하는 스텁.
    ///     레포지토리 완성 후 실제 HTTP 요청으로 교체할 것.
    pub async fn fetch_registry(&self) -> Result<Vec<RemoteExtensionInfo>> {
        tracing::debug!("Fetching extension registry from: {}", self.registry_url);

        // TODO: 레포지토리 완성 후 실제 HTTP 요청으로 교체
        // 현재는 서버 연결 없이 빈 목록 반환
        // let response = reqwest::get(&self.registry_url).await
        //     .with_context(|| format!("Failed to fetch registry from {}", self.registry_url))?;
        // let registry: ExtensionRegistryResponse = response.json().await
        //     .context("Failed to parse registry response")?;
        // return Ok(registry.extensions);

        Ok(Vec::new())
    }

    /// 설치된 익스텐션 중 원격 버전보다 낮은 것의 업데이트 정보를 반환합니다.
    pub fn check_updates_against(
        &self,
        remote: &[RemoteExtensionInfo],
    ) -> Vec<ExtensionUpdateInfo> {
        let mut updates = Vec::new();
        for local in self.discovered.values() {
            if let Some(remote_ext) = remote.iter().find(|r| r.id == local.manifest.id) {
                // updater 크레이트의 SemVer를 사용하여 버전 비교
                let is_newer = match (
                    SemVer::parse(&remote_ext.version),
                    SemVer::parse(&local.manifest.version),
                ) {
                    (Some(remote_v), Some(local_v)) => remote_v.is_newer_than(&local_v),
                    // 파싱 실패 시 문자열 사전순 비교로 폴백
                    _ => remote_ext.version > local.manifest.version,
                };

                if is_newer {
                    updates.push(ExtensionUpdateInfo {
                        id: local.manifest.id.clone(),
                        installed_version: local.manifest.version.clone(),
                        latest_version: remote_ext.version.clone(),
                        download_url: remote_ext.download_url.clone(),
                        downloaded: false,
                        installed: false,
                    });
                }
            }
        }
        updates
    }

    /// 버전 문자열 비교 (updater 크레이트의 SemVer를 사용, 폴백 포함)
    ///
    /// 기존 업데이터와 동일한 `SemVer` 타입을 사용하여 동작을 보장합니다.
    pub fn is_newer_version(candidate: &str, current: &str) -> bool {
        match (SemVer::parse(candidate), SemVer::parse(current)) {
            (Some(c), Some(cur)) => c.is_newer_than(&cur),
            _ => candidate > current,
        }
    }

    /// 원격에서 zip을 다운로드하여 extensions/ 폴더에 설치합니다.
    ///
    /// ⚠️  레포지토리 미완성 — 현재는 스텁 구현.
    ///     `download_url`에 실제 파일이 있을 때 동작합니다.
    pub async fn install_from_url(
        &self,
        ext_id: &str,
        download_url: &str,
        _expected_sha256: Option<&str>,
    ) -> Result<()> {
        tracing::info!("Installing extension '{}' from {}", ext_id, download_url);

        // TODO: sha256 검증 로직 구현
        // 다운로드
        let response = reqwest::get(download_url)
            .await
            .with_context(|| format!("Failed to download extension from {}", download_url))?;

        if !response.status().is_success() {
            return Err(anyhow::anyhow!(
                "Download failed: HTTP {}",
                response.status()
            ));
        }

        let bytes = response
            .bytes()
            .await
            .context("Failed to read download response body")?;

        // 임시 zip 파일로 저장
        let zip_path = self.extensions_dir.join(format!("{}.zip", ext_id));
        std::fs::write(&zip_path, &bytes)
            .with_context(|| format!("Failed to write download to {}", zip_path.display()))?;

        // 압축 해제 (기존 폴더가 있으면 먼저 제거)
        let dest = self.extensions_dir.join(ext_id);
        if dest.is_dir() {
            std::fs::remove_dir_all(&dest)
                .with_context(|| format!("Failed to remove existing extension dir: {}", dest.display()))?;
        }

        let file = std::fs::File::open(&zip_path)?;
        let mut archive = zip::ZipArchive::new(file)
            .context("Failed to read downloaded zip archive")?;

        for i in 0..archive.len() {
            let mut zip_file = archive.by_index(i)?;
            let outpath = match zip_file.enclosed_name() {
                Some(p) => dest.join(p),
                None => continue,
            };
            if zip_file.is_dir() {
                std::fs::create_dir_all(&outpath)?;
            } else {
                if let Some(parent) = outpath.parent() {
                    std::fs::create_dir_all(parent)?;
                }
                let mut outfile = std::fs::File::create(&outpath)?;
                std::io::copy(&mut zip_file, &mut outfile)?;
            }
        }

        // 임시 zip 삭제
        let _ = std::fs::remove_file(&zip_path);

        tracing::info!("Extension '{}' installed successfully", ext_id);
        Ok(())
    }

    /// i18n JSON 로드
    pub fn load_i18n(&self, ext_id: &str, locale: &str) -> Option<Value> {
        let ext = self.discovered.get(ext_id)?;
        let i18n_dir = ext.manifest.i18n_dir.as_deref()?;
        let path = ext.dir.join(i18n_dir).join(format!("{}.json", locale));
        if !path.exists() {
            return None;
        }
        let content = std::fs::read_to_string(&path).ok()?;
        serde_json::from_str(&content).ok()
    }

    /// enabled 목록 영속화
    fn save_state(&self) {
        let enabled_list: Vec<&str> = self.enabled.iter().map(|s| s.as_str()).collect();
        let json = match serde_json::to_string_pretty(&enabled_list) {
            Ok(j) => j,
            Err(e) => {
                tracing::error!("Failed to serialize extension state: {}", e);
                return;
            }
        };

        if let Some(parent) = self.state_path.parent() {
            if !parent.exists() {
                let _ = std::fs::create_dir_all(parent);
            }
        }

        if let Err(e) = std::fs::write(&self.state_path, json) {
            tracing::error!(
                "Failed to save extension state to {}: {}",
                self.state_path.display(),
                e
            );
        }
    }

    /// 저장된 enabled 목록 로드
    fn load_state(&mut self) {
        if !self.state_path.exists() {
            return;
        }
        match std::fs::read_to_string(&self.state_path) {
            Ok(content) => {
                match serde_json::from_str::<Vec<String>>(&content) {
                    Ok(list) => {
                        self.enabled = list.into_iter().collect();
                        tracing::info!(
                            "Loaded extension state: {} enabled",
                            self.enabled.len()
                        );
                    }
                    Err(e) => {
                        tracing::warn!(
                            "Failed to parse extension state {}: {}",
                            self.state_path.display(),
                            e
                        );
                    }
                }
            }
            Err(e) => {
                tracing::warn!(
                    "Failed to read extension state {}: {}",
                    self.state_path.display(),
                    e
                );
            }
        }
    }
}

// ═══════════════════════════════════════════════════════════════
//  Tests
// ═══════════════════════════════════════════════════════════════

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn test_evaluate_condition_bool_true() {
        let mut ext_data = HashMap::new();
        ext_data.insert(
            "docker_enabled".to_string(),
            Value::Bool(true),
        );
        assert!(ExtensionManager::evaluate_condition(
            "instance.ext_data.docker_enabled",
            &ext_data
        ));
    }

    #[test]
    fn test_evaluate_condition_bool_false() {
        let mut ext_data = HashMap::new();
        ext_data.insert(
            "docker_enabled".to_string(),
            Value::Bool(false),
        );
        assert!(!ExtensionManager::evaluate_condition(
            "instance.ext_data.docker_enabled",
            &ext_data
        ));
    }

    #[test]
    fn test_evaluate_condition_missing_key() {
        let ext_data = HashMap::new();
        assert!(!ExtensionManager::evaluate_condition(
            "instance.ext_data.docker_enabled",
            &ext_data
        ));
    }

    #[test]
    fn test_evaluate_condition_unknown_pattern() {
        let ext_data = HashMap::new();
        assert!(!ExtensionManager::evaluate_condition(
            "some.other.pattern",
            &ext_data
        ));
    }

    #[test]
    fn test_manifest_deserialization() {
        let json = json!({
            "id": "docker",
            "name": "Docker Isolation",
            "version": "1.0.0",
            "description": "Docker container isolation",
            "python_modules": {
                "compose_manager": "compose_manager.py"
            },
            "hooks": {
                "server.pre_start": {
                    "module": "compose_manager",
                    "function": "start",
                    "condition": "instance.ext_data.docker_enabled"
                }
            },
            "instance_fields": {
                "docker_enabled": { "type": "boolean", "default": false }
            }
        });

        let manifest: ExtensionManifest = serde_json::from_value(json).unwrap();
        assert_eq!(manifest.id, "docker");
        assert_eq!(manifest.hooks.len(), 1);
        assert!(manifest.hooks.contains_key("server.pre_start"));
        assert_eq!(manifest.instance_fields.len(), 1);
    }

    #[test]
    fn test_extension_manager_empty_dir() {
        let tmp = tempfile::tempdir().unwrap();
        let mut mgr = ExtensionManager::new_isolated(tmp.path().to_str().unwrap());
        let found = mgr.discover().unwrap();
        assert!(found.is_empty());
    }

    #[test]
    fn test_extension_manager_discover() {
        let tmp = tempfile::tempdir().unwrap();
        let ext_dir = tmp.path().join("test_ext");
        std::fs::create_dir_all(&ext_dir).unwrap();
        std::fs::write(
            ext_dir.join("manifest.json"),
            r#"{"id":"test_ext","name":"Test","version":"0.1.0"}"#,
        )
        .unwrap();

        let mut mgr = ExtensionManager::new_isolated(tmp.path().to_str().unwrap());
        let found = mgr.discover().unwrap();
        assert_eq!(found, vec!["test_ext"]);
        assert!(mgr.discovered.contains_key("test_ext"));
    }

    #[test]
    fn test_enable_disable() {
        let tmp = tempfile::tempdir().unwrap();
        let ext_dir = tmp.path().join("test_ext");
        std::fs::create_dir_all(&ext_dir).unwrap();
        std::fs::write(
            ext_dir.join("manifest.json"),
            r#"{"id":"test_ext","name":"Test","version":"0.1.0"}"#,
        )
        .unwrap();

        let mut mgr = ExtensionManager::new_isolated(tmp.path().to_str().unwrap());
        mgr.discover().unwrap();

        let no_instances: Vec<(&str, &HashMap<String, Value>)> = vec![];

        assert!(!mgr.is_enabled("test_ext"));
        mgr.enable("test_ext").unwrap();
        assert!(mgr.is_enabled("test_ext"));
        mgr.disable("test_ext", &no_instances).unwrap();
        assert!(!mgr.is_enabled("test_ext"));
    }

    #[test]
    fn test_enable_unknown_extension() {
        let tmp = tempfile::tempdir().unwrap();
        let mut mgr = ExtensionManager::new_isolated(tmp.path().to_str().unwrap());
        assert!(mgr.enable("nonexistent").is_err());
    }

    // ── 동적 마운트/언마운트 테스트 ──

    #[test]
    fn test_mount_unmount() {
        let tmp = tempfile::tempdir().unwrap();
        let ext_dir = tmp.path().join("my_ext");
        std::fs::create_dir_all(&ext_dir).unwrap();
        std::fs::write(
            ext_dir.join("manifest.json"),
            r#"{"id":"my_ext","name":"My Extension","version":"0.1.0"}"#,
        )
        .unwrap();

        let mut mgr = ExtensionManager::new_isolated(tmp.path().to_str().unwrap());
        // 아직 discover 안 했으므로 비어 있음
        assert!(mgr.list().is_empty());

        // 핫 마운트
        mgr.mount("my_ext").unwrap();
        assert_eq!(mgr.list().len(), 1);

        // 핫 언마운트
        let no_instances: Vec<(&str, &HashMap<String, Value>)> = vec![];
        mgr.unmount("my_ext", &no_instances).unwrap();
        assert!(mgr.list().is_empty());
    }

    #[test]
    fn test_mount_bad_id_mismatch() {
        let tmp = tempfile::tempdir().unwrap();
        // 디렉토리 이름은 "wrong_dir"이지만 manifest id는 "correct_id"
        let ext_dir = tmp.path().join("wrong_dir");
        std::fs::create_dir_all(&ext_dir).unwrap();
        std::fs::write(
            ext_dir.join("manifest.json"),
            r#"{"id":"correct_id","name":"Test","version":"0.1.0"}"#,
        )
        .unwrap();

        let mut mgr = ExtensionManager::new_isolated(tmp.path().to_str().unwrap());
        let result = mgr.mount("wrong_dir");
        assert!(result.is_err());
        assert!(result.unwrap_err().to_string().contains("does not match"));
    }

    #[test]
    fn test_rescan_finds_new_extensions() {
        let tmp = tempfile::tempdir().unwrap();
        let mut mgr = ExtensionManager::new_isolated(tmp.path().to_str().unwrap());
        mgr.discover().unwrap();
        assert!(mgr.list().is_empty());

        // 디스크에 새 익스텐션 추가
        let ext_dir = tmp.path().join("late_ext");
        std::fs::create_dir_all(&ext_dir).unwrap();
        std::fs::write(
            ext_dir.join("manifest.json"),
            r#"{"id":"late_ext","name":"Late","version":"0.2.0"}"#,
        )
        .unwrap();

        let newly_found = mgr.rescan().unwrap();
        assert_eq!(newly_found, vec!["late_ext"]);
        assert_eq!(mgr.list().len(), 1);
    }

    // ── 의존성 검증 테스트 ──

    #[test]
    fn test_enable_with_missing_dependency() {
        let tmp = tempfile::tempdir().unwrap();
        let ext_dir = tmp.path().join("child_ext");
        std::fs::create_dir_all(&ext_dir).unwrap();
        std::fs::write(
            ext_dir.join("manifest.json"),
            r#"{"id":"child_ext","name":"Child","version":"0.1.0","dependencies":["parent_ext"]}"#,
        )
        .unwrap();

        let mut mgr = ExtensionManager::new_isolated(tmp.path().to_str().unwrap());
        mgr.discover().unwrap();

        let result = mgr.enable("child_ext");
        assert!(result.is_err());
        assert!(result.unwrap_err().to_string().contains("parent_ext"));
    }

    #[test]
    fn test_enable_with_satisfied_dependency() {
        let tmp = tempfile::tempdir().unwrap();

        // parent_ext
        let parent_dir = tmp.path().join("parent_ext");
        std::fs::create_dir_all(&parent_dir).unwrap();
        std::fs::write(
            parent_dir.join("manifest.json"),
            r#"{"id":"parent_ext","name":"Parent","version":"0.1.0"}"#,
        )
        .unwrap();

        // child_ext depends on parent_ext
        let child_dir = tmp.path().join("child_ext");
        std::fs::create_dir_all(&child_dir).unwrap();
        std::fs::write(
            child_dir.join("manifest.json"),
            r#"{"id":"child_ext","name":"Child","version":"0.1.0","dependencies":["parent_ext"]}"#,
        )
        .unwrap();

        let mut mgr = ExtensionManager::new_isolated(tmp.path().to_str().unwrap());
        mgr.discover().unwrap();

        // parent 먼저 활성화
        mgr.enable("parent_ext").unwrap();
        // 이제 child 활성화 가능
        mgr.enable("child_ext").unwrap();
        assert!(mgr.is_enabled("child_ext"));
    }

    #[test]
    fn test_disable_blocked_by_dependent() {
        let tmp = tempfile::tempdir().unwrap();

        let parent_dir = tmp.path().join("parent_ext");
        std::fs::create_dir_all(&parent_dir).unwrap();
        std::fs::write(
            parent_dir.join("manifest.json"),
            r#"{"id":"parent_ext","name":"Parent","version":"0.1.0"}"#,
        )
        .unwrap();

        let child_dir = tmp.path().join("child_ext");
        std::fs::create_dir_all(&child_dir).unwrap();
        std::fs::write(
            child_dir.join("manifest.json"),
            r#"{"id":"child_ext","name":"Child","version":"0.1.0","dependencies":["parent_ext"]}"#,
        )
        .unwrap();

        let mut mgr = ExtensionManager::new_isolated(tmp.path().to_str().unwrap());
        mgr.discover().unwrap();
        mgr.enable("parent_ext").unwrap();
        mgr.enable("child_ext").unwrap();

        let no_instances: Vec<(&str, &HashMap<String, Value>)> = vec![];

        // parent를 비활성화하려 하면 child가 의존하므로 실패
        let result = mgr.disable("parent_ext", &no_instances);
        assert!(result.is_err());
        assert!(result.unwrap_err().to_string().contains("child_ext"));
    }

    #[test]
    fn test_disable_blocked_by_instance_usage() {
        let tmp = tempfile::tempdir().unwrap();

        let ext_dir = tmp.path().join("docker");
        std::fs::create_dir_all(&ext_dir).unwrap();
        std::fs::write(
            ext_dir.join("manifest.json"),
            r#"{"id":"docker","name":"Docker","version":"1.0.0","instance_fields":{"docker_enabled":{"type":"boolean","default":false}}}"#,
        )
        .unwrap();

        let mut mgr = ExtensionManager::new_isolated(tmp.path().to_str().unwrap());
        mgr.discover().unwrap();
        mgr.enable("docker").unwrap();

        // 인스턴스가 docker_enabled=true로 사용 중
        let mut ext_data = HashMap::new();
        ext_data.insert("docker_enabled".to_string(), Value::Bool(true));
        let instances: Vec<(&str, &HashMap<String, Value>)> =
            vec![("my_server", &ext_data)];

        let result = mgr.disable("docker", &instances);
        assert!(result.is_err());
        assert!(result.unwrap_err().to_string().contains("my_server"));
    }

    #[test]
    fn test_unmount_blocked_by_dependent() {
        let tmp = tempfile::tempdir().unwrap();

        let parent_dir = tmp.path().join("parent_ext");
        std::fs::create_dir_all(&parent_dir).unwrap();
        std::fs::write(
            parent_dir.join("manifest.json"),
            r#"{"id":"parent_ext","name":"Parent","version":"0.1.0"}"#,
        )
        .unwrap();

        let child_dir = tmp.path().join("child_ext");
        std::fs::create_dir_all(&child_dir).unwrap();
        std::fs::write(
            child_dir.join("manifest.json"),
            r#"{"id":"child_ext","name":"Child","version":"0.1.0","dependencies":["parent_ext"]}"#,
        )
        .unwrap();

        let mut mgr = ExtensionManager::new_isolated(tmp.path().to_str().unwrap());
        mgr.discover().unwrap();
        mgr.enable("parent_ext").unwrap();
        mgr.enable("child_ext").unwrap();

        let no_instances: Vec<(&str, &HashMap<String, Value>)> = vec![];
        let result = mgr.unmount("parent_ext", &no_instances);
        assert!(result.is_err());
        assert!(result.unwrap_err().to_string().contains("child_ext"));
    }

    #[test]
    fn test_force_disable() {
        let tmp = tempfile::tempdir().unwrap();
        let ext_dir = tmp.path().join("test_ext");
        std::fs::create_dir_all(&ext_dir).unwrap();
        std::fs::write(
            ext_dir.join("manifest.json"),
            r#"{"id":"test_ext","name":"Test","version":"0.1.0"}"#,
        )
        .unwrap();

        let mut mgr = ExtensionManager::new_isolated(tmp.path().to_str().unwrap());
        mgr.discover().unwrap();
        mgr.enable("test_ext").unwrap();
        assert!(mgr.is_enabled("test_ext"));

        mgr.force_disable("test_ext");
        assert!(!mgr.is_enabled("test_ext"));
    }

    // ═══════════════════════════════════════════════════════════════
    //  추가 심층 테스트
    // ═══════════════════════════════════════════════════════════════

    /// 조건 평가 — 숫자 0 → false, 비제로 → true
    #[test]
    fn test_evaluate_condition_number_values() {
        let mut ext_data = HashMap::new();
        ext_data.insert("cpu_limit".to_string(), Value::Number(serde_json::Number::from(0)));
        assert!(!ExtensionManager::evaluate_condition(
            "instance.ext_data.cpu_limit", &ext_data
        ));

        ext_data.insert("cpu_limit".to_string(), Value::Number(serde_json::Number::from(4)));
        assert!(ExtensionManager::evaluate_condition(
            "instance.ext_data.cpu_limit", &ext_data
        ));
    }

    /// 조건 평가 — 빈 문자열 → false, 비빈 문자열 → true
    #[test]
    fn test_evaluate_condition_string_values() {
        let mut ext_data = HashMap::new();
        ext_data.insert("image".to_string(), Value::String("".to_string()));
        assert!(!ExtensionManager::evaluate_condition(
            "instance.ext_data.image", &ext_data
        ));

        ext_data.insert("image".to_string(), Value::String("cm2network/steamcmd".to_string()));
        assert!(ExtensionManager::evaluate_condition(
            "instance.ext_data.image", &ext_data
        ));
    }

    /// 매니페스트 — 풀 필드 역직렬화 (GUI, CLI, hooks, dependencies, i18n)
    #[test]
    fn test_manifest_full_fields_deserialization() {
        let json = json!({
            "id": "docker",
            "name": "Docker Isolation",
            "version": "2.0.0",
            "description": "Container isolation for game servers",
            "author": "saba-chan",
            "dependencies": ["steamcmd"],
            "python_modules": {
                "compose_manager": "compose_manager.py",
                "health_check": "health.py"
            },
            "hooks": {
                "server.pre_start": {
                    "module": "compose_manager",
                    "function": "pre_start",
                    "condition": "instance.ext_data.docker_enabled"
                },
                "server.post_stop": {
                    "module": "compose_manager",
                    "function": "post_stop"
                }
            },
            "instance_fields": {
                "docker_enabled": { "type": "boolean", "default": false },
                "docker_image": { "type": "string" }
            },
            "gui": {
                "bundle": "docker-panel.js",
                "styles": "docker-panel.css",
                "slots": { "InstanceList.badge": "DockerBadge" }
            },
            "cli": {
                "slots": { "InstanceList.badge": {"text": "🐳"} }
            },
            "i18n_dir": "locales",
            "module_config_section": "docker"
        });

        let manifest: ExtensionManifest = serde_json::from_value(json).unwrap();
        assert_eq!(manifest.id, "docker");
        assert_eq!(manifest.version, "2.0.0");
        assert_eq!(manifest.author, "saba-chan");
        assert_eq!(manifest.dependencies.len(), 1);
        assert_eq!(manifest.dependencies.get("steamcmd").unwrap(), "*");
        assert_eq!(manifest.hooks.len(), 2);
        assert!(manifest.hooks.contains_key("server.pre_start"));
        assert!(manifest.hooks.contains_key("server.post_stop"));
        assert_eq!(manifest.python_modules.len(), 2);
        assert_eq!(manifest.instance_fields.len(), 2);
        assert!(manifest.gui.is_some());
        assert!(manifest.cli.is_some());
        assert_eq!(manifest.i18n_dir.as_deref(), Some("locales"));
        assert_eq!(manifest.module_config_section.as_deref(), Some("docker"));
    }

    /// 매니페스트 — 최소 필드만으로도 역직렬화 가능
    #[test]
    fn test_manifest_minimal_deserialization() {
        let json = json!({"id": "x", "name": "X", "version": "0.0.1"});
        let manifest: ExtensionManifest = serde_json::from_value(json).unwrap();
        assert_eq!(manifest.id, "x");
        assert!(manifest.hooks.is_empty());
        assert!(manifest.dependencies.is_empty());
        assert!(manifest.gui.is_none());
    }

    /// 잘못된 JSON으로 매니페스트 역직렬화 실패
    #[test]
    fn test_manifest_invalid_json() {
        let json_no_id = json!({"name": "NoID", "version": "0.1.0"});
        assert!(serde_json::from_value::<ExtensionManifest>(json_no_id).is_err());
    }

    /// 다이아몬드 의존성 — A→B, A→C, B→D, C→D
    #[test]
    fn test_diamond_dependency_enable_order() {
        let tmp = tempfile::tempdir().unwrap();

        let create_ext = |id: &str, deps: &[&str]| {
            let dir = tmp.path().join(id);
            std::fs::create_dir_all(&dir).unwrap();
            let manifest = json!({
                "id": id,
                "name": id,
                "version": "0.1.0",
                "dependencies": deps
            });
            std::fs::write(dir.join("manifest.json"), manifest.to_string()).unwrap();
        };

        create_ext("ext_d", &[]);
        create_ext("ext_b", &["ext_d"]);
        create_ext("ext_c", &["ext_d"]);
        create_ext("ext_a", &["ext_b", "ext_c"]);

        let mut mgr = ExtensionManager::new_isolated(tmp.path().to_str().unwrap());
        mgr.discover().unwrap();

        // 순서대로 활성화해야 함
        assert!(mgr.enable("ext_a").is_err(), "A는 B, C 미활성 시 실패");
        mgr.enable("ext_d").unwrap();
        assert!(mgr.enable("ext_b").is_ok());
        assert!(mgr.enable("ext_a").is_err(), "A는 C 미활성 시 여전히 실패");
        assert!(mgr.enable("ext_c").is_ok());
        assert!(mgr.enable("ext_a").is_ok(), "A의 모든 의존성 충족");
    }

    /// 삭제 — 비활성화 후 디렉토리 삭제
    #[test]
    fn test_remove_extension_cleans_directory() {
        let tmp = tempfile::tempdir().unwrap();
        let ext_dir = tmp.path().join("removable");
        std::fs::create_dir_all(&ext_dir).unwrap();
        std::fs::write(
            ext_dir.join("manifest.json"),
            r#"{"id":"removable","name":"Remove Me","version":"0.1.0"}"#,
        ).unwrap();

        let mut mgr = ExtensionManager::new_isolated(tmp.path().to_str().unwrap());
        mgr.discover().unwrap();
        assert_eq!(mgr.list().len(), 1);

        let no_instances: Vec<(&str, &HashMap<String, Value>)> = vec![];
        mgr.remove("removable", &no_instances).unwrap();
        assert!(mgr.list().is_empty());
        assert!(!ext_dir.exists(), "Extension directory should be deleted");
    }

    /// 삭제 — 의존하는 익스텐션이 있으면 실패
    #[test]
    fn test_remove_blocked_by_active_dependent() {
        let tmp = tempfile::tempdir().unwrap();
        let parent_dir = tmp.path().join("parent");
        std::fs::create_dir_all(&parent_dir).unwrap();
        std::fs::write(parent_dir.join("manifest.json"),
            r#"{"id":"parent","name":"Parent","version":"0.1.0"}"#).unwrap();

        let child_dir = tmp.path().join("child");
        std::fs::create_dir_all(&child_dir).unwrap();
        std::fs::write(child_dir.join("manifest.json"),
            r#"{"id":"child","name":"Child","version":"0.1.0","dependencies":["parent"]}"#).unwrap();

        let mut mgr = ExtensionManager::new_isolated(tmp.path().to_str().unwrap());
        mgr.discover().unwrap();
        mgr.enable("parent").unwrap();
        mgr.enable("child").unwrap();

        let no_instances: Vec<(&str, &HashMap<String, Value>)> = vec![];
        let result = mgr.remove("parent", &no_instances);
        assert!(result.is_err(), "Cannot remove parent while child depends on it");
    }

    /// list() 결과 검증 — enabled 상태, hooks, instance_fields 정확히 반영
    #[test]
    fn test_list_reflects_extension_state() {
        let tmp = tempfile::tempdir().unwrap();
        let ext_dir = tmp.path().join("ext_a");
        std::fs::create_dir_all(&ext_dir).unwrap();
        std::fs::write(ext_dir.join("manifest.json"), json!({
            "id": "ext_a",
            "name": "Extension A",
            "version": "1.2.3",
            "description": "Test extension",
            "author": "Tester",
            "hooks": { "server.pre_start": { "module": "m", "function": "f" } },
            "instance_fields": { "my_flag": { "type": "boolean", "default": false } }
        }).to_string()).unwrap();

        let mut mgr = ExtensionManager::new_isolated(tmp.path().to_str().unwrap());
        mgr.discover().unwrap();

        let list = mgr.list();
        assert_eq!(list.len(), 1);
        let item = &list[0];
        assert_eq!(item.id, "ext_a");
        assert_eq!(item.version, "1.2.3");
        assert!(!item.enabled, "Initially disabled");
        assert_eq!(item.hooks, vec!["server.pre_start"]);
        assert!(item.instance_fields.contains_key("my_flag"));

        mgr.enable("ext_a").unwrap();
        let list = mgr.list();
        assert!(list[0].enabled, "Should be enabled after enable()");
    }

    /// hooks_for — 비활성 익스텐션의 hook은 반환되지 않아야 함
    #[test]
    fn test_hooks_for_only_returns_enabled() {
        let tmp = tempfile::tempdir().unwrap();
        let ext_dir = tmp.path().join("hook_ext");
        std::fs::create_dir_all(&ext_dir).unwrap();
        std::fs::write(ext_dir.join("manifest.json"), json!({
            "id": "hook_ext", "name": "Hook Ext", "version": "0.1.0",
            "hooks": { "server.pre_start": { "module": "m", "function": "f" } },
            "python_modules": { "m": "m.py" }
        }).to_string()).unwrap();

        let mut mgr = ExtensionManager::new_isolated(tmp.path().to_str().unwrap());
        mgr.discover().unwrap();

        // 비활성 → hooks_for 비어있음
        assert!(mgr.hooks_for("server.pre_start").is_empty());

        // 활성화 → hooks_for에 포함
        mgr.enable("hook_ext").unwrap();
        let hooks = mgr.hooks_for("server.pre_start");
        assert_eq!(hooks.len(), 1);
        assert_eq!(hooks[0].0.manifest.id, "hook_ext");
        assert_eq!(hooks[0].1.function, "f");

        // 존재하지 않는 hook 이름
        assert!(mgr.hooks_for("nonexistent.hook").is_empty());
    }

    /// should_parse_config_section — module_config_section 매칭
    #[test]
    fn test_should_parse_config_section() {
        let tmp = tempfile::tempdir().unwrap();
        let ext_dir = tmp.path().join("docker");
        std::fs::create_dir_all(&ext_dir).unwrap();
        std::fs::write(ext_dir.join("manifest.json"), json!({
            "id": "docker", "name": "Docker", "version": "1.0.0",
            "module_config_section": "docker"
        }).to_string()).unwrap();

        let mut mgr = ExtensionManager::new_isolated(tmp.path().to_str().unwrap());
        mgr.discover().unwrap();

        // 비활성 → false
        assert!(!mgr.should_parse_config_section("docker"));

        mgr.enable("docker").unwrap();
        assert!(mgr.should_parse_config_section("docker"));
        assert!(!mgr.should_parse_config_section("other_section"));
    }

    /// all_instance_fields — 여러 익스텐션의 필드 합산
    #[test]
    fn test_all_instance_fields_merges_across_extensions() {
        let tmp = tempfile::tempdir().unwrap();

        let make_ext = |id: &str, field: &str| {
            let dir = tmp.path().join(id);
            std::fs::create_dir_all(&dir).unwrap();
            std::fs::write(dir.join("manifest.json"), json!({
                "id": id, "name": id, "version": "0.1.0",
                "instance_fields": { field: { "type": "boolean", "default": false } }
            }).to_string()).unwrap();
        };

        make_ext("ext_a", "field_a");
        make_ext("ext_b", "field_b");

        let mut mgr = ExtensionManager::new_isolated(tmp.path().to_str().unwrap());
        mgr.discover().unwrap();
        mgr.enable("ext_a").unwrap();
        mgr.enable("ext_b").unwrap();

        let fields = mgr.all_instance_fields();
        assert!(fields.contains_key("field_a"));
        assert!(fields.contains_key("field_b"));
        assert_eq!(fields.len(), 2);
    }

    /// is_newer_version 유틸리티
    #[test]
    fn test_is_newer_version() {
        assert!(ExtensionManager::is_newer_version("1.1.0", "1.0.0"));
        assert!(ExtensionManager::is_newer_version("2.0.0", "1.9.9"));
        assert!(!ExtensionManager::is_newer_version("1.0.0", "1.0.0"));
        assert!(!ExtensionManager::is_newer_version("0.9.0", "1.0.0"));
    }

    /// check_updates_against — 로컬 < 원격이면 업데이트 정보 반환
    #[test]
    fn test_check_updates_against() {
        let tmp = tempfile::tempdir().unwrap();
        let ext_dir = tmp.path().join("test_ext");
        std::fs::create_dir_all(&ext_dir).unwrap();
        std::fs::write(ext_dir.join("manifest.json"),
            r#"{"id":"test_ext","name":"Test","version":"1.0.0"}"#).unwrap();

        let mut mgr = ExtensionManager::new_isolated(tmp.path().to_str().unwrap());
        mgr.discover().unwrap();

        let remote = vec![RemoteExtensionInfo {
            id: "test_ext".to_string(),
            name: "Test".to_string(),
            version: "2.0.0".to_string(),
            description: String::new(),
            author: String::new(),
            download_url: "https://example.com/test_ext.zip".to_string(),
            sha256: None,
            min_app_version: None,
            tags: vec![],
            homepage: None,
        }];

        let updates = mgr.check_updates_against(&remote);
        assert_eq!(updates.len(), 1);
        assert_eq!(updates[0].id, "test_ext");
        assert_eq!(updates[0].installed_version, "1.0.0");
        assert_eq!(updates[0].latest_version, "2.0.0");
        assert!(!updates[0].downloaded);
        assert!(!updates[0].installed);
    }

    /// check_updates_against — 이미 최신이면 빈 목록
    #[test]
    fn test_check_updates_already_latest() {
        let tmp = tempfile::tempdir().unwrap();
        let ext_dir = tmp.path().join("test_ext");
        std::fs::create_dir_all(&ext_dir).unwrap();
        std::fs::write(ext_dir.join("manifest.json"),
            r#"{"id":"test_ext","name":"Test","version":"2.0.0"}"#).unwrap();

        let mut mgr = ExtensionManager::new_isolated(tmp.path().to_str().unwrap());
        mgr.discover().unwrap();

        let remote = vec![RemoteExtensionInfo {
            id: "test_ext".to_string(),
            name: "Test".to_string(),
            version: "2.0.0".to_string(),
            description: String::new(),
            author: String::new(),
            download_url: "https://example.com/test_ext.zip".to_string(),
            sha256: None,
            min_app_version: None,
            tags: vec![],
            homepage: None,
        }];

        let updates = mgr.check_updates_against(&remote);
        assert!(updates.is_empty(), "Same version should not be an update");
    }

    /// 영속화 — enable → new_isolated 재생성 → enabled 상태 유지
    #[test]
    fn test_state_persistence_across_reload() {
        let tmp = tempfile::tempdir().unwrap();
        let ext_dir = tmp.path().join("persistent_ext");
        std::fs::create_dir_all(&ext_dir).unwrap();
        std::fs::write(ext_dir.join("manifest.json"),
            r#"{"id":"persistent_ext","name":"Persistent","version":"0.1.0"}"#).unwrap();

        // 1차: enable
        {
            let mut mgr = ExtensionManager::new_isolated(tmp.path().to_str().unwrap());
            mgr.discover().unwrap();
            mgr.enable("persistent_ext").unwrap();
            assert!(mgr.is_enabled("persistent_ext"));
        }

        // 2차: 재생성 → 상태 복원
        {
            let mut mgr = ExtensionManager::new_isolated(tmp.path().to_str().unwrap());
            mgr.discover().unwrap();
            assert!(mgr.is_enabled("persistent_ext"), "Enabled state must persist across reload");
        }
    }

    /// zip 자동 추출 테스트
    #[test]
    fn test_discover_extracts_zip_extension() {
        let tmp = tempfile::tempdir().unwrap();

        // manifest.json이 들어있는 zip 파일 생성
        let zip_path = tmp.path().join("zip_ext.zip");
        let manifest_content = r#"{"id":"zip_ext","name":"Zip Extension","version":"0.1.0"}"#;

        let file = std::fs::File::create(&zip_path).unwrap();
        let mut zip_writer = zip::ZipWriter::new(file);
        let options = zip::write::FileOptions::default()
            .compression_method(zip::CompressionMethod::Stored);
        zip_writer.start_file("manifest.json", options).unwrap();
        std::io::Write::write_all(&mut zip_writer, manifest_content.as_bytes()).unwrap();
        zip_writer.finish().unwrap();

        let mut mgr = ExtensionManager::new_isolated(tmp.path().to_str().unwrap());
        let found = mgr.discover().unwrap();
        assert!(
            found.contains(&"zip_ext".to_string()),
            "Zip extension should be auto-extracted and discovered: {:?}", found
        );

        // zip 파일이 삭제되었어야 함
        assert!(!zip_path.exists(), "Zip file should be removed after extraction");
    }

    // ── 컴포넌트 버전 의존성(dependencies) 테스트 ──

    #[test]
    fn test_enable_with_component_version_satisfied() {
        let tmp = tempfile::tempdir().unwrap();
        let ext_dir = tmp.path().join("my_ext");
        std::fs::create_dir_all(&ext_dir).unwrap();
        std::fs::write(ext_dir.join("manifest.json"), json!({
            "id": "my_ext",
            "name": "My Extension",
            "version": "1.0.0",
            "dependencies": { "saba-core": ">=0.3.0" }
        }).to_string()).unwrap();

        let mut mgr = ExtensionManager::new_isolated(tmp.path().to_str().unwrap());
        mgr.discover().unwrap();

        let mut versions = HashMap::new();
        versions.insert("saba-core".to_string(), "0.5.0".to_string());

        let result = mgr.enable_with_versions("my_ext", &versions);
        assert!(result.is_ok(), "Should enable when component version is satisfied");
    }

    #[test]
    fn test_enable_with_component_version_too_low() {
        let tmp = tempfile::tempdir().unwrap();
        let ext_dir = tmp.path().join("my_ext");
        std::fs::create_dir_all(&ext_dir).unwrap();
        std::fs::write(ext_dir.join("manifest.json"), json!({
            "id": "my_ext",
            "name": "My Extension",
            "version": "1.0.0",
            "dependencies": { "saba-core": ">=0.3.0" }
        }).to_string()).unwrap();

        let mut mgr = ExtensionManager::new_isolated(tmp.path().to_str().unwrap());
        mgr.discover().unwrap();

        let mut versions = HashMap::new();
        versions.insert("saba-core".to_string(), "0.2.0".to_string());

        let result = mgr.enable_with_versions("my_ext", &versions);
        assert!(result.is_err(), "Should fail when component version is too low");
        let err_msg = result.unwrap_err().to_string();
        assert!(err_msg.contains("saba-core"), "Error should mention the component");
        assert!(err_msg.contains("0.2.0"), "Error should mention installed version");
    }

    #[test]
    fn test_enable_with_component_not_installed() {
        let tmp = tempfile::tempdir().unwrap();
        let ext_dir = tmp.path().join("my_ext");
        std::fs::create_dir_all(&ext_dir).unwrap();
        std::fs::write(ext_dir.join("manifest.json"), json!({
            "id": "my_ext",
            "name": "My Extension",
            "version": "1.0.0",
            "dependencies": { "gui": ">=0.2.0" }
        }).to_string()).unwrap();

        let mut mgr = ExtensionManager::new_isolated(tmp.path().to_str().unwrap());
        mgr.discover().unwrap();

        // No versions provided → gui not installed
        let result = mgr.enable_with_versions("my_ext", &HashMap::new());
        assert!(result.is_err(), "Should fail when required component is not installed");
        let err_msg = result.unwrap_err().to_string();
        assert!(err_msg.contains("gui"));
        assert!(err_msg.contains("not installed"));
    }

    #[test]
    fn test_enable_without_versions_skips_requires_check() {
        let tmp = tempfile::tempdir().unwrap();
        let ext_dir = tmp.path().join("my_ext");
        std::fs::create_dir_all(&ext_dir).unwrap();
        std::fs::write(ext_dir.join("manifest.json"), json!({
            "id": "my_ext",
            "name": "My Extension",
            "version": "1.0.0",
            "dependencies": { "saba-core": ">=99.0.0" }
        }).to_string()).unwrap();

        let mut mgr = ExtensionManager::new_isolated(tmp.path().to_str().unwrap());
        mgr.discover().unwrap();

        // enable() without versions → no installed_versions → requires check fails
        let result = mgr.enable("my_ext");
        assert!(result.is_err(), "enable() without version info should fail if requires is set");
    }

    #[test]
    fn test_enable_cross_type_requires() {
        let tmp = tempfile::tempdir().unwrap();
        let ext_dir = tmp.path().join("advanced_ext");
        std::fs::create_dir_all(&ext_dir).unwrap();
        std::fs::write(ext_dir.join("manifest.json"), json!({
            "id": "advanced_ext",
            "name": "Advanced",
            "version": "1.0.0",
            "dependencies": {
                "saba-core": ">=0.3.0",
                "gui": ">=0.2.0",
                "discord_bot": ">=0.1.0"
            }
        }).to_string()).unwrap();

        let mut mgr = ExtensionManager::new_isolated(tmp.path().to_str().unwrap());
        mgr.discover().unwrap();

        let mut versions = HashMap::new();
        versions.insert("saba-core".to_string(), "0.5.0".to_string());
        versions.insert("gui".to_string(), "0.3.0".to_string());
        versions.insert("discord_bot".to_string(), "0.1.0".to_string());

        let result = mgr.enable_with_versions("advanced_ext", &versions);
        assert!(result.is_ok(), "All cross-type component deps satisfied");
    }

    #[test]
    fn test_enable_requires_plus_extension_dependency() {
        let tmp = tempfile::tempdir().unwrap();

        // parent extension (no requires)
        let parent_dir = tmp.path().join("parent_ext");
        std::fs::create_dir_all(&parent_dir).unwrap();
        std::fs::write(parent_dir.join("manifest.json"), json!({
            "id": "parent_ext", "name": "Parent", "version": "0.1.0"
        }).to_string()).unwrap();

        // child extension — depends on parent_ext + requires saba-core >=0.3.0
        let child_dir = tmp.path().join("child_ext");
        std::fs::create_dir_all(&child_dir).unwrap();
        std::fs::write(child_dir.join("manifest.json"), json!({
            "id": "child_ext",
            "name": "Child",
            "version": "1.0.0",
            "dependencies": {
                "parent_ext": "*",
                "saba-core": ">=0.3.0"
            }
        }).to_string()).unwrap();

        let mut mgr = ExtensionManager::new_isolated(tmp.path().to_str().unwrap());
        mgr.discover().unwrap();

        let mut versions = HashMap::new();
        versions.insert("saba-core".to_string(), "0.5.0".to_string());

        // parent not enabled → child fails on ext dependency
        let result = mgr.enable_with_versions("child_ext", &versions);
        assert!(result.is_err(), "Should fail: parent not enabled");

        // enable parent, then child should succeed
        mgr.enable_with_versions("parent_ext", &versions).unwrap();
        let result = mgr.enable_with_versions("child_ext", &versions);
        assert!(result.is_ok(), "Both ext dep and component dep satisfied");
    }

    #[test]
    fn test_manifest_dependencies_field_deserialization() {
        // 맵 형식
        let manifest: ExtensionManifest = serde_json::from_value(json!({
            "id": "test_ext",
            "name": "Test",
            "version": "1.0.0",
            "dependencies": {
                "saba-core": ">=0.3.0",
                "gui": ">=0.2.0",
                "docker": ">=1.0.0"
            }
        })).unwrap();

        assert_eq!(manifest.dependencies.len(), 3);
        assert_eq!(manifest.dependencies.get("saba-core").unwrap(), ">=0.3.0");
        assert_eq!(manifest.dependencies.get("gui").unwrap(), ">=0.2.0");
        assert_eq!(manifest.dependencies.get("docker").unwrap(), ">=1.0.0");

        // 배열 형식 (하위 호환)
        let manifest2: ExtensionManifest = serde_json::from_value(json!({
            "id": "test_ext",
            "name": "Test",
            "version": "1.0.0",
            "dependencies": ["steamcmd", "ue4-ini"]
        })).unwrap();

        assert_eq!(manifest2.dependencies.len(), 2);
        assert_eq!(manifest2.dependencies.get("steamcmd").unwrap(), "*");
        assert_eq!(manifest2.dependencies.get("ue4-ini").unwrap(), "*");
    }

    #[test]
    fn test_manifest_dependencies_empty_by_default() {
        let manifest: ExtensionManifest = serde_json::from_value(json!({
            "id": "test_ext",
            "name": "Test",
            "version": "1.0.0"
        })).unwrap();

        assert!(manifest.dependencies.is_empty(), "dependencies should default to empty");
    }

    #[test]
    fn test_list_includes_dependencies() {
        let tmp = tempfile::tempdir().unwrap();
        let ext_dir = tmp.path().join("ext_req");
        std::fs::create_dir_all(&ext_dir).unwrap();
        std::fs::write(ext_dir.join("manifest.json"), json!({
            "id": "ext_req",
            "name": "Ext with Dependencies",
            "version": "1.0.0",
            "dependencies": { "saba-core": ">=0.5.0" }
        }).to_string()).unwrap();

        let mut mgr = ExtensionManager::new_isolated(tmp.path().to_str().unwrap());
        mgr.discover().unwrap();

        let list = mgr.list();
        assert_eq!(list.len(), 1);
        assert_eq!(list[0].dependencies.get("saba-core").unwrap(), ">=0.5.0");
    }

    #[test]
    fn test_component_version_unsatisfied_error() {
        let err = ExtensionError::component_version_unsatisfied(
            "my_ext", "saba-core", ">=0.5.0", Some("0.3.0")
        );
        assert_eq!(err.error_code, "component_version_unsatisfied");
        assert!(err.message.contains("saba-core"));
        assert!(err.message.contains(">=0.5.0"));
        assert!(err.message.contains("0.3.0"));
        assert_eq!(err.related, vec!["saba-core", ">=0.5.0"]);
    }

    #[test]
    fn test_component_version_unsatisfied_not_installed() {
        let err = ExtensionError::component_version_unsatisfied(
            "my_ext", "gui", ">=0.2.0", None
        );
        assert!(err.message.contains("not installed"));
    }
}
