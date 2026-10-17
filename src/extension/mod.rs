//! 범용 익스텐션 시스템
//!
//! `extensions/` 디렉토리의 서브디렉토리를 스캔하여 `manifest.json`을 파싱하고,
//! 데몬/서버 수명주기 Hook을 Python 모듈로 디스패치합니다.

use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::collections::{HashMap, HashSet};
use std::path::PathBuf;
use saba_chan_updater_lib::config_file::ConfigFile;
use saba_chan_updater_lib::version::SemVer;

/// npm package.json 스타일 dependencies 디시리얼라이저.
/// 예전 hook 이름 → 지금 이름. `server.post_stop`은 실제로는 정지 **전에** 불려
/// 익스텐션이 정지를 대신 처리하는 hook이었으므로 `server.stop`으로 바뀌었다.
const HOOK_ALIASES: &[(&str, &str)] = &[("server.post_stop", "server.stop")];

/// 이 데몬이 아는 manifest.json 스키마 버전 (`schema_version`)
pub const EXTENSION_SCHEMA_VERSION: u32 = 1;

/// 데몬이 실제로 부르는 hook. 여기에 없는 이름에 바인딩하면 절대 실행되지 않는다
pub const FIRED_HOOKS: &[&str] = &[
    "daemon.startup",
    "daemon.shutdown",
    "server.pre_start",
    "server.stop",
    "server.status",
    "server.list_enrich",
    "server.post_create",
    "server.pre_delete",
    "server.settings_changed",
    "server.check_update",
];

/// 지금 데몬 버전 (`min_app_version`, `dependencies."saba-core"` 비교용)
pub const APP_VERSION: &str = env!("CARGO_PKG_VERSION");

/// hook 맵을 읽으며 예전 이름을 지금 이름으로 바꾼다 (둘 다 있으면 지금 이름이 이긴다).
fn deserialize_hooks<'de, D>(deserializer: D) -> Result<HashMap<String, HookBinding>, D::Error>
where
    D: serde::Deserializer<'de>,
{
    let mut hooks = HashMap::<String, HookBinding>::deserialize(deserializer)?;
    for (old, new) in HOOK_ALIASES {
        if let Some(binding) = hooks.remove(*old) {
            hooks.entry(new.to_string()).or_insert(binding);
        }
    }
    Ok(hooks)
}

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
    fn app_too_old(ext_id: &str, required: &str) -> Self {
        Self {
            error_code: "app_version_unsatisfied".to_string(),
            message: format!(
                "Cannot enable '{}': requires saba-chan {} or newer (this is {})",
                ext_id, required, APP_VERSION
            ),
            related: vec![required.to_string()],
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
    /// manifest 스키마 버전. 없으면 0(스키마 이전: 모르는 키는 경고만)
    #[serde(default)]
    pub schema_version: u32,
    pub id: String,
    pub name: String,
    pub version: String,
    /// 영어 설명 (번역이 없는 언어에 쓴다)
    #[serde(default)]
    pub description: String,
    /// 언어별 설명 (예: { "ko": "..." }). 목록 API가 GUI 언어에 맞는 것을 고른다
    #[serde(default, skip_serializing_if = "HashMap::is_empty")]
    pub descriptions: HashMap<String, String>,
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
    #[serde(default, deserialize_with = "deserialize_hooks")]
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
    /// 익스텐션 글로벌 설정 스키마 — `config.json`의 `extensions.settings.<id>`에 저장되는 필드 정의.
    /// 인스턴스와 무관한 익스텐션 자체 설정 (예: Discord 봇 토큰, 기본 볼륨 등).
    #[serde(default)]
    pub config_fields: HashMap<String, FieldDef>,
    #[serde(default)]
    pub i18n_dir: Option<String>,
    /// 같은 hook을 여러 익스텐션이 받을 때의 순서. 작을수록 먼저 (기본 100, 같으면 id 순)
    #[serde(default = "default_hook_priority")]
    pub priority: i32,
    /// 인스턴스 명령 카탈로그에 더하는 명령. 모듈 명령과 같은 화면·CLI·디스코드·AI 해석에 실린다
    #[serde(default)]
    pub commands: Vec<ExtensionCommand>,
    /// 코드가 쓰는 위험한 능력 선언 (모듈 `[permissions]`와 같은 표). `extension audit`가 코드와 대조한다
    #[serde(default)]
    pub permissions: crate::supervisor::module_loader::ModulePermissions,
}

/// 익스텐션이 선언하는 게임 명령. 실행하면 `module`의 `function`을 부른다
/// (context: instance_id, instance_name, module, extension_data, args, extension_config)
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ExtensionCommand {
    pub name: String,
    #[serde(default)]
    pub label: Option<String>,
    #[serde(default)]
    pub description: Option<String>,
    /// "safe" | "caution" | "danger" (없으면 caution)
    #[serde(default)]
    pub risk: Option<String>,
    #[serde(default)]
    pub inputs: Vec<crate::supervisor::module_loader::CommandInput>,
    /// python_modules의 키
    pub module: String,
    pub function: String,
    /// 이 조건이 참인 인스턴스에서만 보인다 (예: "instance.ext_data.<flag>")
    #[serde(default)]
    pub condition: Option<String>,
}

fn default_hook_priority() -> i32 {
    100
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct HookBinding {
    pub module: String,   // python_modules의 키
    pub function: String, // Python 함수명
    #[serde(default)]
    pub condition: Option<String>, // 예: "instance.ext_data.<flag>"
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
    /// 로드는 됐지만 고쳐야 할 점 (스키마 이전 manifest의 모르는 키 등)
    pub warnings: Vec<String>,
}

/// 로드하지 못한 익스텐션과 이유 (`GET /api/extensions`의 `failed`)
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ExtensionLoadError {
    pub path: String,
    pub error: String,
}

/// 익스텐션 id는 폴더 이름이 되므로 한 단계 이름이어야 한다: 영문·숫자·`-`·`_`만.
pub fn is_valid_extension_id(id: &str) -> bool {
    !id.is_empty() && id.len() <= 64 && id.chars().all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_')
}

fn ensure_valid_extension_id(id: &str) -> Result<()> {
    if !is_valid_extension_id(id) {
        anyhow::bail!("invalid extension id '{}': use only letters, digits, '-' and '_'", id);
    }
    Ok(())
}

/// manifest.json을 읽는다.
/// - `schema_version = 1`: 모르는 키, 데몬이 부르지 않는 hook, 없는 python_modules 참조는 오류
/// - 없음(0): 같은 문제를 경고로만 돌려준다
/// - 그보다 새 버전: 거부
pub fn parse_manifest(content: &str) -> Result<(ExtensionManifest, Vec<String>)> {
    let mut unknown = Vec::new();
    let mut de = serde_json::Deserializer::from_str(content.trim_start_matches('\u{feff}'));
    let manifest: ExtensionManifest = serde_ignored::deserialize(&mut de, |path| {
        let key: Vec<String> = path.to_string().split('.').filter(|p| *p != "?").map(str::to_string).collect();
        unknown.push(key.join("."))
    })?;
    de.end()?;
    ensure_valid_extension_id(&manifest.id)?;

    let mut problems: Vec<String> = unknown.into_iter().map(|k| format!("unknown key '{}'", k)).collect();
    let mut hooks: Vec<_> = manifest.hooks.iter().collect();
    hooks.sort_by_key(|(name, _)| name.as_str());
    for (name, binding) in hooks {
        if !FIRED_HOOKS.contains(&name.as_str()) {
            problems.push(format!("hook '{}' is never fired by the daemon", name));
        }
        if !manifest.python_modules.contains_key(&binding.module) {
            problems.push(format!("hook '{}' uses unknown python module '{}'", name, binding.module));
        }
    }
    for cmd in &manifest.commands {
        if !manifest.python_modules.contains_key(&cmd.module) {
            problems.push(format!("command '{}' uses unknown python module '{}'", cmd.name, cmd.module));
        }
    }

    match manifest.schema_version {
        0 => {
            problems.push(format!(
                "no schema_version; add \"schema_version\": {} to manifest.json",
                EXTENSION_SCHEMA_VERSION
            ));
            Ok((manifest, problems))
        }
        EXTENSION_SCHEMA_VERSION if problems.is_empty() => Ok((manifest, problems)),
        EXTENSION_SCHEMA_VERSION => anyhow::bail!(
            "manifest.json does not match schema v{}: {}",
            EXTENSION_SCHEMA_VERSION,
            problems.join("; ")
        ),
        v => anyhow::bail!(
            "manifest.json schema_version {} is not supported by this saba-chan (supports {}); update saba-chan",
            v,
            EXTENSION_SCHEMA_VERSION
        ),
    }
}

/// `">=1.2.0"` 또는 `"1.2.0"` 요구를 `have`가 만족하는가
/// `min_app_version`을 이 데몬이 만족하지 않으면 사용자에게 보일 이유
fn app_version_problem(min_app_version: Option<&str>) -> Option<String> {
    min_app_version
        .filter(|v| !version_satisfies(APP_VERSION, v))
        .map(|v| format!("requires saba-chan {} or newer (this is {})", v, APP_VERSION))
}

fn version_satisfies(have: &str, required: &str) -> bool {
    let min = required.trim_start_matches(">=").trim();
    matches!((SemVer::parse(have), SemVer::parse(min)), (Some(h), Some(m)) if h >= m)
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
    #[serde(skip)]
    pub descriptions: HashMap<String, String>,
    pub author: String,
    pub enabled: bool,
    pub hooks: Vec<String>,
    /// npm package.json 스타일 의존성 (이름 → 버전 요구사항)
    pub dependencies: HashMap<String, String>,
    pub gui: Option<GuiManifest>,
    pub cli: Option<CliManifest>,
    pub instance_fields: HashMap<String, FieldDef>,
    pub config_fields: HashMap<String, FieldDef>,
    /// 익스텐션 디렉토리에 icon.png가 존재하는지 여부
    #[serde(default)]
    pub has_icon: bool,
    #[serde(default)]
    pub schema_version: u32,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub min_app_version: Option<String>,
    /// 이 데몬 버전에서 켤 수 없는 이유 (min_app_version 미충족 등)
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub incompatible: Option<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub warnings: Vec<String>,
    /// 신뢰 표시: verified(서명된 카탈로그에서 받았고 그대로) · modified · unverified (M12)
    pub trust: saba_chan_updater_lib::integrity::Trust,
}

// ═══════════════════════════════════════════════════════════════
//  원격 매니페스트 타입 정의
// ═══════════════════════════════════════════════════════════════

/// 릴리즈 manifest의 `ext-<id>` 컴포넌트에서 만든 익스텐션 항목 (`from_release`)
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RemoteExtensionInfo {
    /// 익스텐션 ID (manifest 키에서 `ext-`를 뗀 것, 역직렬화 후 주입)
    #[serde(default)]
    pub id: String,
    pub name: String,
    pub version: String,
    #[serde(default)]
    pub description: String,
    #[serde(default)]
    pub author: String,
    /// 배포 패키지 다운로드 URL (.zip)
    #[serde(default)]
    pub download_url: String,
    /// 패키지 SHA-256 체크섬 (무결성 검증용)
    #[serde(default)]
    pub sha256: Option<String>,
    /// 최소 앱 버전 요구사항
    #[serde(default)]
    pub min_app_version: Option<String>,
    /// 이 데몬에서 설치할 수 없으면 그 이유 (`min_app_version` 미달). 데몬이 채운다 (M9)
    #[serde(default, skip_deserializing)]
    pub incompatible: Option<String>,
    /// 에셋 파일명 (예: "extension-docker.zip")
    #[serde(default)]
    pub asset: Option<String>,
    /// 설치 경로 (예: "extensions/docker")
    #[serde(default)]
    pub install_dir: Option<String>,
    /// 의존성 — 배열 또는 맵 형식 모두 허용
    #[serde(default, deserialize_with = "deserialize_dependencies")]
    pub dependencies: HashMap<String, String>,
    /// GUI 컴포넌트 포함 여부
    #[serde(default)]
    pub has_gui: bool,
    /// i18n 번역 파일 포함 여부
    #[serde(default)]
    pub has_i18n: bool,
    #[serde(default)]
    pub tags: Vec<String>,
    #[serde(default)]
    pub homepage: Option<String>,
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
    /// 새 버전이 이 데몬에서 돌지 않으면 그 이유 (M9)
    pub incompatible: Option<String>,
}

// ═══════════════════════════════════════════════════════════════
//  ExtensionManager
// ═══════════════════════════════════════════════════════════════

pub struct ExtensionManager {
    extensions_dir: PathBuf,
    discovered: HashMap<String, DiscoveredExtension>,
    enabled: HashSet<String>,
    /// 익스텐션 글로벌 설정 (`config.json`의 `extensions.settings`)
    /// 구조: ext_id → { key → value }
    extension_config: HashMap<String, HashMap<String, Value>>,
    /// 활성 목록·글로벌 설정을 저장하는 설정 파일 (`extensions` 네임스페이스)
    config_file: ConfigFile,
    /// 마지막 스캔에서 로드하지 못한 익스텐션
    load_errors: Vec<ExtensionLoadError>,
}

impl ExtensionManager {
    /// 새 ExtensionManager 생성. `extensions_dir`은 `extensions/` 디렉토리 경로.
    pub fn new(extensions_dir: &str) -> Self {
        Self::with_config_file(extensions_dir, ConfigFile::default_location())
    }

    /// 커스텀 state 경로를 지정한 생성자 (테스트 격리용)
    #[cfg(test)]
    pub fn new_isolated(extensions_dir: &str) -> Self {
        Self::with_config_file(extensions_dir, ConfigFile::at(std::path::Path::new(extensions_dir)))
    }

    fn with_config_file(extensions_dir: &str, config_file: ConfigFile) -> Self {
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
            extension_config: HashMap::new(),
            config_file,
            load_errors: Vec::new(),
        };
        mgr.load_persisted();
        mgr
    }

    /// Python import에서 사용할 수 있도록 ext_id의 하이픈을 언더스코어로 변환합니다.
    /// 예: "ue4-ini" → "ue4_ini"
    fn python_safe_dir_name(ext_id: &str) -> String {
        ext_id.replace('-', "_")
    }

    /// ext_id에 대응하는 디스크 상의 디렉토리를 반환합니다.
    ///
    /// 탐색 순서:
    /// 1. discovered에 등록되어 있으면 해당 `dir` 경로 반환
    /// 2. `extensions_dir/ext_id` (원본 ID)
    /// 3. `extensions_dir/python_safe_dir_name(ext_id)` (하이픈→언더스코어)
    fn ext_dir_on_disk(&self, ext_id: &str) -> PathBuf {
        // 1. 이미 discover된 익스텐션이면 실제 경로 반환
        if let Some(ext) = self.discovered.get(ext_id) {
            return ext.dir.clone();
        }
        // 2. ext_id 그대로
        let path_exact = self.extensions_dir.join(ext_id);
        if path_exact.exists() {
            return path_exact;
        }
        // 3. 하이픈→언더스코어 변환 (Python import 호환)
        let safe_name = Self::python_safe_dir_name(ext_id);
        if safe_name != ext_id {
            let path_safe = self.extensions_dir.join(&safe_name);
            if path_safe.exists() {
                return path_safe;
            }
        }
        // 없으면 Python-safe 디렉토리명 반환 (신규 설치 시 사용)
        self.extensions_dir.join(&safe_name)
    }

    /// extensions/ 디렉토리를 스캔하여 익스텐션 발견.
    ///
    /// 지원 형식:
    /// - **폴더형**: `<id>/manifest.json` (현재 방식)
    /// - **단일 파일형**: `<id>.zip` → 자동 압축 해제 후 폴더형으로 등록
    pub fn discover(&mut self) -> Result<Vec<String>> {
        if !self.extensions_dir.is_dir() {
            tracing::warn!(
                "Extensions directory does not exist: {}",
                self.extensions_dir.display()
            );
            return Ok(Vec::new());
        }

        let zips: Vec<PathBuf> = std::fs::read_dir(&self.extensions_dir)
            .with_context(|| format!("Failed to read extensions directory: {}", self.extensions_dir.display()))?
            .filter_map(|e| e.ok().map(|e| e.path()))
            .filter(|p| p.is_file() && p.extension().and_then(|e| e.to_str()) == Some("zip"))
            .collect();
        for zip in zips {
            match self.extract_zip_extension(&zip) {
                Ok(Some(ext_id)) => tracing::info!("Auto-extracted zip extension: {}", ext_id),
                Ok(None) => {}
                Err(e) => tracing::warn!("Failed to extract zip extension {}: {}", zip.display(), e),
            }
        }

        let found = self.scan()?;
        tracing::info!("Extension discovery complete: {} found", found.len());
        Ok(found)
    }

    /// extensions/ 폴더에 놓인 `.zip`을 풀어 설치한다. 같은 익스텐션 폴더가 있으면 zip 내용으로 바꾼다
    /// (예전에는 zip을 지우고 새 내용을 버렸다). 다 풀고 나면 zip을 지운다.
    fn extract_zip_extension(&self, zip_path: &std::path::Path) -> Result<Option<String>> {
        let stem = zip_path
            .file_stem()
            .and_then(|s| s.to_str())
            .ok_or_else(|| anyhow::anyhow!("Invalid zip filename: {}", zip_path.display()))?;
        let bytes = std::fs::read(zip_path)
            .with_context(|| format!("Failed to read zip: {}", zip_path.display()))?;
        let staging = self.extensions_dir.join(format!(".staging-{}", Self::python_safe_dir_name(stem)));
        if staging.exists() {
            std::fs::remove_dir_all(&staging)?;
        }
        let ext_id = (|| -> Result<String> {
            zip::ZipArchive::new(std::io::Cursor::new(&bytes))?.extract(&staging)?;
            let content = std::fs::read_to_string(staging.join("manifest.json"))
                .context("zip has no manifest.json at its root")?;
            Ok(parse_manifest(&content)?.0.id)
        })();
        let ext_id = match ext_id {
            Ok(id) => id,
            Err(e) => {
                let _ = std::fs::remove_dir_all(&staging);
                return Err(e);
            }
        };
        let dest = self.extensions_dir.join(Self::python_safe_dir_name(&ext_id));
        Self::swap_into_place(&staging, &dest)?;
        Self::record_files(&ext_id, &dest, false);
        if let Err(e) = std::fs::remove_file(zip_path) {
            tracing::warn!("Failed to remove zip after extraction: {}", e);
        }
        tracing::info!("Extracted zip extension '{}' to {}", ext_id, dest.display());
        Ok(Some(ext_id))
    }

    /// 설치한 파일의 해시를 적어 둔다. 무결성 검사가 이 기록과 비교한다.
    /// `official`: 서명된 카탈로그에서 받은 것인지 (신뢰 표시)
    fn record_files(ext_id: &str, dest: &std::path::Path, official: bool) {
        let key = format!("ext-{}", ext_id);
        let recorded = saba_chan_updater_lib::integrity::record_dir(&key, dest)
            .and_then(|_| saba_chan_updater_lib::integrity::set_official(&key, official));
        if let Err(e) = recorded {
            tracing::warn!("Could not record installed files for extension '{}': {}", ext_id, e);
        }
    }

    /// `staged` 폴더를 `dest` 자리에 둔다. 옛 `dest`는 새 폴더가 자리를 잡은 뒤에 지우고,
    /// 실패하면 옛 폴더를 되돌린다.
    fn swap_into_place(staged: &std::path::Path, dest: &std::path::Path) -> Result<()> {
        let backup = dest.with_file_name(format!(
            ".old-{}",
            dest.file_name().map(|n| n.to_string_lossy().to_string()).unwrap_or_default()
        ));
        if backup.exists() {
            std::fs::remove_dir_all(&backup)?;
        }
        let had_old = dest.exists();
        if had_old {
            std::fs::rename(dest, &backup)
                .with_context(|| format!("Failed to move old {} aside", dest.display()))?;
        }
        if let Err(e) = std::fs::rename(staged, dest) {
            if had_old {
                let _ = std::fs::rename(&backup, dest);
            }
            let _ = std::fs::remove_dir_all(staged);
            anyhow::bail!("Failed to put {} in place: {}", dest.display(), e);
        }
        if had_old {
            if let Err(e) = std::fs::remove_dir_all(&backup) {
                tracing::warn!("Failed to remove old extension files {}: {}", backup.display(), e);
            }
        }
        Ok(())
    }

    fn load_manifest(&self, path: &std::path::Path) -> Result<(ExtensionManifest, Vec<String>)> {
        let content = std::fs::read_to_string(path)
            .with_context(|| format!("Failed to read {}", path.display()))?;
        parse_manifest(&content).with_context(|| format!("Failed to parse {}", path.display()))
    }

    /// 마지막 스캔에서 로드하지 못한 익스텐션과 이유
    pub fn load_errors(&self) -> &[ExtensionLoadError] {
        &self.load_errors
    }

    /// 폴더마다 manifest.json을 (다시) 읽어 등록한다. 새로 찾은 id를 돌려준다.
    /// 폴더 이름은 manifest id(또는 하이픈을 밑줄로 바꾼 이름)와 같아야 한다.
    /// 디스크에서 사라진 (unmount 안 한) 익스텐션은 목록에 남는다.
    fn scan(&mut self) -> Result<Vec<String>> {
        let mut newly_found = Vec::new();
        let mut errors = Vec::new();
        if !self.extensions_dir.is_dir() {
            return Ok(newly_found);
        }
        let mut dirs: Vec<PathBuf> = std::fs::read_dir(&self.extensions_dir)
            .with_context(|| format!("Failed to read extensions directory: {}", self.extensions_dir.display()))?
            .filter_map(|e| e.ok().map(|e| e.path()))
            .filter(|p| p.is_dir() && p.join("manifest.json").is_file())
            // `.staging-*`, `.old-*`는 설치 도중의 임시 폴더
            .filter(|p| !p.file_name().is_some_and(|n| n.to_string_lossy().starts_with('.')))
            .collect();
        dirs.sort();

        for path in dirs {
            let manifest_path = path.join("manifest.json");
            let fail = |error: String| ExtensionLoadError { path: path.to_string_lossy().to_string(), error };
            let (manifest, warnings) = match self.load_manifest(&manifest_path) {
                Ok(m) => m,
                Err(e) => {
                    tracing::warn!("Failed to load manifest {}: {:#}", manifest_path.display(), e);
                    errors.push(fail(format!("{:#}", e)));
                    continue;
                }
            };
            let dir_name = path.file_name().map(|n| n.to_string_lossy().to_string()).unwrap_or_default();
            if dir_name != manifest.id && dir_name != Self::python_safe_dir_name(&manifest.id) {
                let e = ExtensionError::id_mismatch(&manifest.id, &dir_name);
                tracing::warn!("Skipping extension at {}: {}", path.display(), e);
                errors.push(fail(e.to_string()));
                continue;
            }
            for w in &warnings {
                tracing::warn!("extension '{}': {}", manifest.id, w);
            }
            let id = manifest.id.clone();
            if !self.discovered.contains_key(&id) {
                tracing::info!("Discovered extension: {} v{} ({})", manifest.name, manifest.version, id);
                newly_found.push(id.clone());
            }
            self.discovered.insert(id, DiscoveredExtension { manifest, dir: path, warnings });
        }

        self.load_errors = errors;
        Ok(newly_found)
    }

    // ═══════════════════════════════════════════════════════════════
    //  동적 마운트/언마운트 (재시작 불필요)
    // ═══════════════════════════════════════════════════════════════

    /// 런타임 중 extensions/ 디렉토리를 재스캔하여 새로 추가된 익스텐션 발견.
    /// 기존에 이미 발견된 익스텐션은 매니페스트를 리로드(갱신),
    /// 디스크에서 제거된 (unmount 아닌) 익스텐션은 유지.
    pub fn rescan(&mut self) -> Result<Vec<String>> {
        let newly_found = self.scan()?;
        tracing::info!("Rescan complete: {} new extension(s)", newly_found.len());
        Ok(newly_found)
    }

    /// 단일 익스텐션을 핫 마운트 (디스크에서 로드 → discovered에 추가).
    /// 이미 존재하면 매니페스트를 리로드.
    pub fn mount(&mut self, ext_id: &str) -> Result<()> {
        let ext_path = self.ext_dir_on_disk(ext_id);
        let manifest_path = ext_path.join("manifest.json");

        if !manifest_path.exists() {
            return Err(ExtensionError::manifest_not_found(
                &manifest_path.display().to_string(),
            ).into());
        }

        let (manifest, warnings) = self.load_manifest(&manifest_path)?;
        if manifest.id != ext_id {
            return Err(ExtensionError::id_mismatch(&manifest.id, ext_id).into());
        }

        tracing::info!("Mounted extension: {} v{}", manifest.name, manifest.version);
        self.discovered.insert(
            ext_id.to_string(),
            DiscoveredExtension { manifest, dir: ext_path, warnings },
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
                field_keys.iter().any(|key| match data.get(*key) {
                    Some(Value::Bool(b)) => *b,
                    Some(Value::String(v)) => !v.is_empty(),
                    Some(Value::Number(n)) => n.as_f64() != Some(0.0),
                    Some(Value::Array(v)) => !v.is_empty(),
                    Some(Value::Object(v)) => !v.is_empty(),
                    _ => false,
                })
            })
            .map(|(name, _)| name.to_string())
            .collect()
    }

    /// 익스텐션 활성화 — 통합 dependencies 맵에서 의존성 전부 검증.
    pub fn enable(&mut self, ext_id: &str) -> Result<()> {
        let installed = HashMap::from([("saba-core".to_string(), APP_VERSION.to_string())]);
        self.enable_with_versions(ext_id, &installed)
    }

    /// 이 데몬에서 켤 수 없는 이유 (`min_app_version`이 더 높음)
    fn incompatibility(m: &ExtensionManifest) -> Option<String> {
        app_version_problem(m.min_app_version.as_deref())
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

        if let Some(required) = &self.discovered[ext_id].manifest.min_app_version {
            if !version_satisfies(APP_VERSION, required) {
                return Err(ExtensionError::app_too_old(ext_id, required).into());
            }
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
        // 꺼져 있어도 이 익스텐션을 쓰는 인스턴스가 있으면 지우지 않는다
        let using = self.instances_using_ext(ext_id, active_ext_data);
        if !using.is_empty() {
            return Err(ExtensionError::in_use(ext_id, &using).into());
        }
        // 활성화 상태면 먼저 비활성화 (의존성 검사 포함)
        if self.enabled.contains(ext_id) {
            self.disable(ext_id, active_ext_data)?;
        }
        // 발견 목록에서 제거
        self.discovered.remove(ext_id);
        // 디렉토리 삭제
        let ext_path = self.ext_dir_on_disk(ext_id);
        if ext_path.exists() {
            std::fs::remove_dir_all(&ext_path)
                .with_context(|| format!("Failed to remove extension directory: {}", ext_path.display()))?;
        }
        // extensionConfig에서 해당 익스텐션의 설정 제거
        if self.extension_config.remove(ext_id).is_some() {
            self.save_extension_config();
        }
        self.save_state();
        tracing::info!("Extension removed: {}", ext_id);
        Ok(())
    }

    /// 활성 여부 확인
    #[cfg(test)]
    pub fn is_enabled(&self, ext_id: &str) -> bool {
        self.enabled.contains(ext_id)
    }

    /// 활성화되어 있고 **디스크에 실제로 설치된** 익스텐션 ID 집합을 반환합니다.
    /// `enabled_set()`과 달리, `discover()`로 발견된 익스텐션만 포함합니다.
    /// 모듈 필수 익스텐션 검증 시 이 메서드를 사용하세요.
    pub fn installed_and_enabled_set(&self) -> HashSet<String> {
        self.enabled
            .iter()
            .filter(|ext_id| self.discovered.contains_key(ext_id.as_str()))
            .cloned()
            .collect()
    }

    /// 활성화되어 있지만 디스크에 설치되지 않은 익스텐션 ID 목록을 반환합니다.
    pub fn enabled_but_not_installed(&self) -> Vec<String> {
        self.enabled
            .iter()
            .filter(|ext_id| !self.discovered.contains_key(ext_id.as_str()))
            .cloned()
            .collect()
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
                    descriptions: m.descriptions.clone(),
                    author: m.author.clone(),
                    enabled: self.enabled.contains(&m.id),
                    hooks: m.hooks.keys().cloned().collect(),
                    dependencies: m.dependencies.clone(),
                    gui: m.gui.clone(),
                    cli: m.cli.clone(),
                    instance_fields: m.instance_fields.clone(),
                    config_fields: m.config_fields.clone(),
                    has_icon,
                    schema_version: m.schema_version,
                    min_app_version: m.min_app_version.clone(),
                    incompatible: Self::incompatibility(m),
                    warnings: ext.warnings.clone(),
                    trust: saba_chan_updater_lib::integrity::trust_of(&format!("ext-{}", m.id)),
                }
            })
            .collect()
    }

    /// 지정된 hook에 바인딩된 활성 익스텐션 목록. 실행 순서대로 (`priority`, 그다음 id)
    pub fn hooks_for(&self, hook_name: &str) -> Vec<(&DiscoveredExtension, &HookBinding)> {
        let mut result: Vec<_> = self
            .discovered
            .values()
            .filter(|ext| self.enabled.contains(&ext.manifest.id))
            .filter_map(|ext| ext.manifest.hooks.get(hook_name).map(|b| (ext, b)))
            .collect();
        result.sort_by(|(a, _), (b, _)| {
            (a.manifest.priority, &a.manifest.id).cmp(&(b.manifest.priority, &b.manifest.id))
        });
        result
    }

    /// 이 인스턴스(`ext_data`)에서 쓸 수 있는 익스텐션 명령. `priority`, id 순
    pub fn commands_for(&self, ext_data: &HashMap<String, Value>) -> Vec<(&str, &ExtensionCommand)> {
        let mut exts: Vec<_> = self.discovered.values().filter(|e| self.enabled.contains(&e.manifest.id)).collect();
        exts.sort_by(|a, b| (a.manifest.priority, &a.manifest.id).cmp(&(b.manifest.priority, &b.manifest.id)));
        exts.into_iter()
            .flat_map(|ext| ext.manifest.commands.iter().map(move |c| (ext.manifest.id.as_str(), c)))
            .filter(|(_, c)| c.condition.as_deref().is_none_or(|cond| Self::evaluate_condition(cond, ext_data)))
            .collect()
    }

    /// 익스텐션 명령 `name`을 부를 계획. 이 인스턴스에서 그 명령을 가진 익스텐션이 없으면 None.
    /// `context`에는 extension_data가 들어 있어야 한다
    /// 익스텐션 명령 실행 계획. 인자는 명령의 `inputs`로 검사해 `context.args`(이름 → 값)에 넣는다.
    /// 그런 명령이 없으면 None, 인자가 틀리면 Some(Err).
    pub fn plan_command(
        &self,
        name: &str,
        args: &crate::commands::Args,
        mut context: Value,
    ) -> Option<Result<HookPlan, crate::commands::PlanError>> {
        let ext_data: HashMap<String, Value> = context
            .get("extension_data")
            .and_then(|v| serde_json::from_value(v.clone()).ok())
            .unwrap_or_default();
        let (ext_id, cmd) = self
            .commands_for(&ext_data)
            .into_iter()
            .find(|(_, c)| c.name.eq_ignore_ascii_case(name))?;
        let ext = self.discovered.get(ext_id)?;
        let bound = match crate::commands::bind(&cmd.inputs, args) {
            Ok(bound) => bound,
            Err(e) => return Some(Err(e)),
        };
        if let Some(obj) = context.as_object_mut() {
            obj.insert("args".into(), Value::Object(bound));
        }
        let target = ext
            .manifest
            .python_modules
            .get(&cmd.module)
            .map(|rel| (ext.dir.join(rel).to_string_lossy().to_string(), cmd.function.clone()))
            .ok_or_else(|| format!("Unknown python module: {}", cmd.module));
        Some(Ok(HookPlan {
            hook: format!("command.{}", cmd.name),
            context: self.inject_extension_config(context),
            calls: vec![HookCall { ext_id: ext_id.to_string(), target }],
        }))
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

    /// hook context에 extension_config를 주입합니다.
    /// 이미 존재하지 않는 경우에만 추가합니다.
    fn inject_extension_config(&self, mut context: Value) -> Value {
        if let Some(obj) = context.as_object_mut() {
            if !obj.contains_key("extension_config") {
                obj.insert(
                    "extension_config".to_string(),
                    self.extension_config_as_value(),
                );
            }
        }
        context
    }

    /// 이 hook에서 부를 익스텐션 함수 목록을 정한다 (조건 평가, extension_config 주입).
    /// Python 실행은 돌려준 `HookPlan`으로, 매니저 락을 놓은 뒤에 한다 (C12).
    pub fn plan_hook(&self, hook_name: &str, context: Value, only: Option<&str>) -> HookPlan {
        let mut hooks = self.hooks_for(hook_name);
        if let Some(id) = only {
            hooks.retain(|(ext, _)| ext.manifest.id == id);
        }
        if hooks.is_empty() {
            return HookPlan { hook: hook_name.to_string(), context, calls: Vec::new() };
        }

        // extension_config를 context에 자동 주입
        let context = self.inject_extension_config(context);

        let ext_data: HashMap<String, Value> = context
            .get("extension_data")
            .and_then(|v| serde_json::from_value(v.clone()).ok())
            .unwrap_or_default();

        let calls = hooks
            .into_iter()
            .filter(|(ext, binding)| match binding.condition {
                Some(ref cond) if !Self::evaluate_condition(cond, &ext_data) => {
                    tracing::debug!("Hook '{}' from '{}' skipped: condition '{}' is false", hook_name, ext.manifest.id, cond);
                    false
                }
                _ => true,
            })
            .map(|(ext, binding)| {
                let target = match ext.manifest.python_modules.get(&binding.module) {
                    Some(rel_path) => Ok((ext.dir.join(rel_path).to_string_lossy().to_string(), binding.function.clone())),
                    None => {
                        tracing::error!(
                            "Extension '{}' hook '{}' references unknown module '{}'",
                            ext.manifest.id,
                            hook_name,
                            binding.module
                        );
                        Err(format!("Unknown python module: {}", binding.module))
                    }
                };
                HookCall { ext_id: ext.manifest.id.clone(), target }
            })
            .collect();
        HookPlan { hook: hook_name.to_string(), context, calls }
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
    /// 발견된 익스텐션 (manifest + 폴더)
    pub fn discovered(&self, ext_id: &str) -> Option<&DiscoveredExtension> {
        self.discovered.get(ext_id)
    }

    pub fn extension_file_path(&self, ext_id: &str, relative: &str) -> Option<PathBuf> {
        self.discovered.get(ext_id).map(|ext| ext.dir.join(relative))
    }

    // ═══════════════════════════════════════════════════════════════
    //  원격 매니페스트 & 버전 관리
    // ═══════════════════════════════════════════════════════════════

    /// 받을 수 있는 익스텐션 목록: 최신 릴리즈의 서명된 manifest에 든 `ext-*` 컴포넌트 (U9e).
    /// 네트워크 요청이므로 매니저 락을 놓고 부른다 (C12)
    pub async fn fetch_remote() -> Result<Vec<RemoteExtensionInfo>> {
        let catalog = saba_chan_updater_lib::relay::latest_catalog(None)
            .await
            .context("Failed to fetch the release manifest")?;
        let mut list = catalog.map(|m| Self::from_release(&m)).unwrap_or_default();
        for ext in &mut list {
            ext.incompatible = app_version_problem(ext.min_app_version.as_deref());
        }
        Ok(list)
    }

    /// 릴리즈 manifest의 `ext-<id>` 컴포넌트를 익스텐션 목록으로 (meta에 이름·설명·의존성)
    pub fn from_release(manifest: &saba_chan_updater_lib::relay::ReleaseManifest) -> Vec<RemoteExtensionInfo> {
        manifest
            .addons(saba_chan_updater_lib::relay::EXTENSION_PREFIX)
            .into_iter()
            .filter_map(|(id, comp)| {
                let mut info = match &comp.meta {
                    Some(Value::Object(meta)) => meta.clone(),
                    _ => serde_json::Map::new(),
                };
                info.entry("name").or_insert_with(|| Value::String(id.to_string()));
                info.insert("version".into(), Value::String(comp.version.clone()));
                info.insert("download_url".into(), Value::String(comp.url.clone()));
                info.insert("sha256".into(), Value::String(comp.sha256.clone()));
                info.insert("asset".into(), Value::String(comp.filename.clone()));
                if let Some(dir) = &comp.install_dir {
                    info.insert("install_dir".into(), Value::String(dir.clone()));
                }
                match serde_json::from_value::<RemoteExtensionInfo>(Value::Object(info)) {
                    Ok(mut ext) => {
                        ext.id = id.to_string();
                        Some(ext)
                    }
                    Err(e) => {
                        tracing::warn!("Skipping extension '{}' in release manifest: {}", id, e);
                        None
                    }
                }
            })
            .collect()
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
                        incompatible: app_version_problem(remote_ext.min_app_version.as_deref()),
                    });
                }
            }
        }
        updates
    }

    /// 익스텐션 패키지(zip)를 받는다. 락 없이 부르면 된다
    pub async fn download_package(download_url: &str) -> Result<Vec<u8>> {
        let response = reqwest::get(download_url)
            .await
            .with_context(|| format!("Failed to download extension from {}", download_url))?;
        if !response.status().is_success() {
            anyhow::bail!("Download failed: HTTP {}", response.status());
        }
        Ok(response.bytes().await.context("Failed to read download response body")?.to_vec())
    }

    pub fn extensions_dir(&self) -> &std::path::Path {
        &self.extensions_dir
    }

    /// 받은 zip을 검사하고 `extensions/.staging-<id>`에 푼다. 락 없이 부르면 된다.
    /// sha256(있으면), manifest 스키마, manifest id, min_app_version을 여기서 확인하므로
    /// 실패해도 설치된 익스텐션은 그대로다.
    pub fn stage_package(
        extensions_dir: &std::path::Path,
        ext_id: &str,
        bytes: &[u8],
        expected_sha256: Option<&str>,
    ) -> Result<PathBuf> {
        ensure_valid_extension_id(ext_id)?;
        if let Some(expected) = expected_sha256.filter(|s| !s.is_empty()) {
            use sha2::{Digest, Sha256};
            let actual = hex::encode(Sha256::digest(bytes));
            if !actual.eq_ignore_ascii_case(expected) {
                anyhow::bail!("sha256 mismatch for '{}': expected {}, got {}", ext_id, expected, actual);
            }
        }
        let staging = extensions_dir.join(format!(".staging-{}", Self::python_safe_dir_name(ext_id)));
        if staging.exists() {
            std::fs::remove_dir_all(&staging)
                .with_context(|| format!("Failed to clear {}", staging.display()))?;
        }
        let result = (|| -> Result<()> {
            let mut archive = zip::ZipArchive::new(std::io::Cursor::new(bytes))
                .context("Failed to read downloaded zip archive")?;
            archive.extract(&staging).context("Failed to extract extension zip")?;
            let content = std::fs::read_to_string(staging.join("manifest.json"))
                .context("Package has no manifest.json at its root")?;
            let (manifest, _) = parse_manifest(&content)?;
            if manifest.id != ext_id {
                return Err(ExtensionError::id_mismatch(&manifest.id, ext_id).into());
            }
            if let Some(reason) = Self::incompatibility(&manifest) {
                anyhow::bail!("Cannot install '{}': {}", ext_id, reason);
            }
            Ok(())
        })();
        if let Err(e) = result {
            let _ = std::fs::remove_dir_all(&staging);
            return Err(e);
        }
        Ok(staging)
    }

    /// 검사를 마친 패키지 폴더로 익스텐션을 바꿔 끼운다 (새로 설치 또는 업데이트).
    /// 실행 중인 인스턴스가 쓰고 있으면 거부한다. 옛 폴더는 새 폴더가 자리를 잡은 뒤에 지운다.
    pub fn install_staged(
        &mut self,
        ext_id: &str,
        staged: &std::path::Path,
        running_ext_data: &[(&str, &HashMap<String, Value>)],
        official: bool,
    ) -> Result<()> {
        if let Err(e) = ensure_valid_extension_id(ext_id) {
            let _ = std::fs::remove_dir_all(staged);
            return Err(e);
        }
        let using = self.instances_using_ext(ext_id, running_ext_data);
        if !using.is_empty() {
            let _ = std::fs::remove_dir_all(staged);
            return Err(ExtensionError::in_use(ext_id, &using).into());
        }
        let dest = self.ext_dir_on_disk(ext_id);
        Self::swap_into_place(staged, &dest)?;
        Self::record_files(ext_id, &dest, official);
        self.mount(ext_id)?;
        tracing::info!("Extension '{}' installed", ext_id);
        Ok(())
    }

    /// 이 익스텐션이 의존하지만 아직 없는 익스텐션 (원격 목록에 있는 것만; `saba-core` 같은 컴포넌트는 제외)
    pub fn missing_dependencies(&self, ext_id: &str, remote: &[RemoteExtensionInfo]) -> Vec<String> {
        let Some(ext) = self.discovered.get(ext_id) else { return Vec::new() };
        let mut missing: Vec<String> = ext
            .manifest
            .dependencies
            .keys()
            .filter(|dep| !self.discovered.contains_key(*dep) && remote.iter().any(|r| &r.id == *dep))
            .cloned()
            .collect();
        missing.sort();
        missing
    }

    /// i18n JSON 로드
    pub fn load_i18n(&self, ext_id: &str, locale: &str) -> Option<Value> {
        if !crate::utils::is_single_path_component(locale) {
            return None;
        }
        let ext = self.discovered.get(ext_id)?;
        let i18n_dir = ext.manifest.i18n_dir.as_deref()?;
        let path = ext.dir.join(i18n_dir).join(format!("{}.json", locale));
        if !path.exists() {
            return None;
        }
        let content = std::fs::read_to_string(&path).ok()?;
        serde_json::from_str(&content).ok()
    }

    // ═══════════════════════════════════════════════════════════════
    //  익스텐션 글로벌 설정 (config.json의 extensions.settings)
    // ═══════════════════════════════════════════════════════════════

    /// 특정 익스텐션의 글로벌 설정을 반환합니다.
    pub fn get_extension_config(&self, ext_id: &str) -> HashMap<String, Value> {
        self.extension_config
            .get(ext_id)
            .cloned()
            .unwrap_or_default()
    }

    /// 전체 익스텐션 설정 맵을 serde_json::Value로 반환합니다 (hook context 주입용).
    pub fn extension_config_as_value(&self) -> Value {
        serde_json::to_value(&self.extension_config).unwrap_or_default()
    }

    /// 특정 익스텐션의 글로벌 설정을 업데이트합니다.
    /// 기존 키를 머지(덮어쓰기)하며, 디스크에 즉시 영속화합니다.
    pub fn set_extension_config(&mut self, ext_id: &str, values: HashMap<String, Value>) {
        let entry = self.extension_config
            .entry(ext_id.to_string())
            .or_default();
        for (k, v) in values {
            entry.insert(k, v);
        }
        self.save_extension_config();
    }

    /// `extensions` 네임스페이스의 한 키를 저장
    fn persist(&self, key: &str, value: Value) {
        let result = self.config_file.update(|doc| {
            let section = doc
                .entry("extensions".to_string())
                .or_insert_with(|| Value::Object(Default::default()));
            if !section.is_object() {
                *section = Value::Object(Default::default());
            }
            section.as_object_mut().expect("object").insert(key.to_string(), value);
        });
        if let Err(e) = result {
            tracing::error!("Failed to save extension {}: {}", key, e);
        }
    }

    /// enabled 목록 영속화
    fn save_state(&self) {
        let mut list: Vec<&str> = self.enabled.iter().map(String::as_str).collect();
        list.sort_unstable();
        self.persist("enabled", serde_json::json!(list));
    }

    /// 익스텐션 글로벌 설정 영속화
    fn save_extension_config(&self) {
        match serde_json::to_value(&self.extension_config) {
            Ok(v) => self.persist("settings", v),
            Err(e) => tracing::error!("Failed to serialize extension config: {}", e),
        }
    }

    /// 저장된 enabled 목록과 글로벌 설정 로드
    fn load_persisted(&mut self) {
        let section = self.config_file.section("extensions");
        if let Some(list) = section.get("enabled") {
            match serde_json::from_value::<Vec<String>>(list.clone()) {
                Ok(list) => {
                    self.enabled = list.into_iter().collect();
                    tracing::info!("Loaded extension state: {} enabled", self.enabled.len());
                }
                Err(e) => tracing::warn!("Invalid extensions.enabled in config: {}", e),
            }
        }
        if let Some(settings) = section.get("settings") {
            match serde_json::from_value(settings.clone()) {
                Ok(config) => self.extension_config = config,
                Err(e) => tracing::warn!("Invalid extensions.settings in config: {}", e),
            }
        }
    }
}

// ═══════════════════════════════════════════════════════════════
//  Tests
// ═══════════════════════════════════════════════════════════════

/// `ExtensionManager::plan_hook`이 정한 hook 실행 목록
pub struct HookPlan {
    hook: String,
    context: Value,
    calls: Vec<HookCall>,
}

struct HookCall {
    ext_id: String,
    /// (Python 파일 경로, 함수 이름), 또는 왜 부를 수 없는지
    target: std::result::Result<(String, String), String>,
}

impl HookPlan {
    /// 우선순위 순서로 부른다. `handled=true`가 나오면 나머지는 건너뛴다 (chain-of-responsibility).
    /// 실패한 익스텐션은 기록만 하고 다음으로 넘어간다.
    pub async fn run(self, timeout_secs: u64) -> Vec<(String, Result<Value>)> {
        let mut results = Vec::new();
        for call in self.calls {
            let (module_path, function) = match call.target {
                Ok(t) => t,
                Err(msg) => {
                    results.push((call.ext_id, Err(anyhow::anyhow!(msg))));
                    continue;
                }
            };
            tracing::debug!("Dispatching hook '{}' → ext '{}' → {}::{}", self.hook, call.ext_id, module_path, function);
            let result = crate::plugin::run_plugin_with_timeout(&module_path, &function, self.context.clone(), timeout_secs).await;
            match result {
                Ok(val) => {
                    let handled = val.get("handled").and_then(|h| h.as_bool()) == Some(true);
                    results.push((call.ext_id.clone(), Ok(val)));
                    if handled {
                        tracing::debug!("Hook '{}' handled by extension '{}', skipping remaining", self.hook, call.ext_id);
                        break;
                    }
                }
                Err(e) => {
                    tracing::error!("Hook '{}' ext '{}' failed: {}", self.hook, call.ext_id, e);
                    results.push((call.ext_id, Err(anyhow::anyhow!("Hook dispatch failed: {}", e))));
                }
            }
        }
        results
    }
}

/// 매니저 락은 실행 목록을 정할 때만 잡고, Python은 락 밖에서 부른다 (C12).
/// 오래 걸리는 hook 동안에도 익스텐션 켜기·끄기와 다른 hook이 막히지 않는다.
pub async fn run_hook(mgr: &tokio::sync::RwLock<ExtensionManager>, hook_name: &str, context: Value) -> Vec<(String, Result<Value>)> {
    run_hook_timed(mgr, hook_name, context, crate::plugin::DEFAULT_PLUGIN_TIMEOUT_SECS).await
}

pub async fn run_hook_timed(
    mgr: &tokio::sync::RwLock<ExtensionManager>,
    hook_name: &str,
    context: Value,
    timeout_secs: u64,
) -> Vec<(String, Result<Value>)> {
    let plan = mgr.read().await.plan_hook(hook_name, context, None);
    plan.run(timeout_secs).await
}

/// 익스텐션 하나에만 (예: 방금 켠 익스텐션의 `daemon.startup`)
pub async fn run_hook_to(
    mgr: &tokio::sync::RwLock<ExtensionManager>,
    ext_id: &str,
    hook_name: &str,
    context: Value,
) -> Option<Result<Value>> {
    let plan = mgr.read().await.plan_hook(hook_name, context, Some(ext_id));
    plan.run(crate::plugin::DEFAULT_PLUGIN_TIMEOUT_SECS).await.into_iter().next().map(|(_, r)| r)
}

#[cfg(test)]
mod tests {
    /// FIRED_HOOKS의 이름마다 데몬 코드(이 파일 밖) 어딘가에서 그 이름으로 hook을 부른다
    #[test]
    fn every_listed_hook_is_fired_somewhere() {
        fn sources(dir: &std::path::Path, out: &mut String) {
            for e in std::fs::read_dir(dir).unwrap().flatten() {
                let p = e.path();
                if p.is_dir() {
                    sources(&p, out);
                } else if p.extension().is_some_and(|x| x == "rs") && !p.ends_with("extension/mod.rs") {
                    out.push_str(&std::fs::read_to_string(&p).unwrap());
                }
            }
        }
        let mut code = String::new();
        sources(&std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("src"), &mut code);
        let unfired: Vec<&&str> = FIRED_HOOKS.iter().filter(|h| !code.contains(&format!("\"{}\"", h))).collect();
        assert!(unfired.is_empty(), "listed in FIRED_HOOKS but never fired: {:?}", unfired);
    }

    use super::*;
    use serde_json::json;

    #[test]
    fn extension_id_must_be_a_single_folder_name() {
        for ok in ["steamcmd", "music-bot", "a_b2"] {
            assert!(is_valid_extension_id(ok), "{}", ok);
        }
        for bad in ["", "../instances", "..", "a/b", "a\\b", "C:x", "/abs", "a.b"] {
            assert!(!is_valid_extension_id(bad), "{}", bad);
        }
        let err = parse_manifest(r#"{"id":"../instances","name":"x","version":"1.0.0"}"#).unwrap_err();
        assert!(err.to_string().contains("invalid extension id"), "{}", err);
    }

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

    fn write_ext(root: &std::path::Path, dir: &str, manifest: Value) {
        let d = root.join(dir);
        std::fs::create_dir_all(&d).unwrap();
        std::fs::write(d.join("manifest.json"), manifest.to_string()).unwrap();
    }

    #[test]
    fn manifest_schema_v1_is_strict_and_legacy_warns() {
        let ok = json!({"schema_version": 1, "id": "a", "name": "A", "version": "0.1.0",
            "python_modules": {"m": "m.py"}, "hooks": {"daemon.startup": {"module": "m", "function": "f"}}});
        let (_, warnings) = parse_manifest(&ok.to_string()).unwrap();
        assert!(warnings.is_empty(), "{warnings:?}");

        let mut typo = ok.clone();
        typo["instance_feilds"] = json!({});
        assert!(parse_manifest(&typo.to_string()).unwrap_err().to_string().contains("instance_feilds"));

        let mut never = ok.clone();
        never["hooks"]["server.pre_create"] = json!({"module": "m", "function": "g", "async": true});
        let err = parse_manifest(&never.to_string()).unwrap_err().to_string();
        assert!(err.contains("server.pre_create") && err.contains("async"), "{err}");

        let mut legacy = never.clone();
        legacy.as_object_mut().unwrap().remove("schema_version");
        let (m, warnings) = parse_manifest(&legacy.to_string()).unwrap();
        assert_eq!(m.schema_version, 0);
        assert!(warnings.iter().any(|w| w.contains("never fired")), "{warnings:?}");

        let mut perms = ok.clone();
        perms["permissions"] = json!({"network": true, "spawn_process": true});
        let (m, _) = parse_manifest(&perms.to_string()).unwrap();
        assert!(m.permissions.network && m.permissions.spawn_process && !m.permissions.native_code);
        perms["permissions"]["netwrok"] = json!(true);
        assert!(parse_manifest(&perms.to_string()).unwrap_err().to_string().contains("netwrok"));

        let mut newer = ok.clone();
        newer["schema_version"] = json!(2);
        assert!(parse_manifest(&newer.to_string()).unwrap_err().to_string().contains("schema_version 2"));
    }

    #[test]
    fn scan_skips_folders_whose_name_is_not_the_id_and_reports_them() {
        let tmp = tempfile::tempdir().unwrap();
        write_ext(tmp.path(), "good", json!({"id": "good", "name": "G", "version": "0.1.0"}));
        write_ext(tmp.path(), "ue4_ini", json!({"id": "ue4-ini", "name": "U", "version": "0.1.0"}));
        write_ext(tmp.path(), "wrong_dir", json!({"id": "other", "name": "W", "version": "0.1.0"}));
        write_ext(tmp.path(), "broken", json!({"name": "no id"}));
        let mut mgr = ExtensionManager::new_isolated(tmp.path().to_str().unwrap());
        let mut found = mgr.discover().unwrap();
        found.sort();
        assert_eq!(found, ["good", "ue4-ini"]);
        let errors = mgr.load_errors();
        assert_eq!(errors.len(), 2, "{errors:?}");
        assert!(errors.iter().any(|e| e.error.contains("does not match")));
    }

    #[test]
    fn min_app_version_and_core_dependency_are_checked_on_enable() {
        let tmp = tempfile::tempdir().unwrap();
        write_ext(tmp.path(), "future", json!({"id": "future", "name": "F", "version": "0.1.0", "min_app_version": "999.0.0"}));
        write_ext(tmp.path(), "core", json!({"id": "core", "name": "C", "version": "0.1.0",
            "min_app_version": "0.0.1", "dependencies": {"saba-core": ">=0.0.1"}}));
        let mut mgr = ExtensionManager::new_isolated(tmp.path().to_str().unwrap());
        mgr.discover().unwrap();
        let err = mgr.enable("future").unwrap_err();
        assert_eq!(err.downcast_ref::<ExtensionError>().unwrap().error_code, "app_version_unsatisfied");
        let item = mgr.list().into_iter().find(|i| i.id == "future").unwrap();
        assert!(item.incompatible.unwrap().contains("999.0.0"));
        mgr.enable("core").expect("saba-core dependency is satisfied by this daemon");
    }

    #[test]
    fn in_use_ignores_empty_values_and_remove_checks_disabled_extensions() {
        let tmp = tempfile::tempdir().unwrap();
        write_ext(tmp.path(), "box", json!({"id": "box", "name": "B", "version": "0.1.0",
            "instance_fields": {"box_enabled": {"type": "boolean"}}}));
        let mut mgr = ExtensionManager::new_isolated(tmp.path().to_str().unwrap());
        mgr.discover().unwrap();
        let unused: HashMap<String, Value> =
            [("box_enabled".to_string(), json!(false))].into_iter().collect();
        let zero: HashMap<String, Value> = [("box_enabled".to_string(), json!(0))].into_iter().collect();
        let used: HashMap<String, Value> = [("box_enabled".to_string(), json!(true))].into_iter().collect();
        assert!(mgr.instances_using_ext("box", &[("a", &unused), ("b", &zero)]).is_empty());
        let err = mgr.remove("box", &[("stopped", &used)]).unwrap_err();
        assert_eq!(err.downcast_ref::<ExtensionError>().unwrap().error_code, "in_use");
        assert!(tmp.path().join("box").exists());
    }

    fn zip_package(manifest: Value, extra: &str) -> Vec<u8> {
        use std::io::Write;
        let mut buf = std::io::Cursor::new(Vec::new());
        {
            let mut w = zip::ZipWriter::new(&mut buf);
            let opts = zip::write::FileOptions::default();
            w.start_file("manifest.json", opts).unwrap();
            w.write_all(manifest.to_string().as_bytes()).unwrap();
            w.start_file("data.txt", opts).unwrap();
            w.write_all(extra.as_bytes()).unwrap();
            w.finish().unwrap();
        }
        buf.into_inner()
    }

    #[test]
    fn install_checks_package_then_swaps_folder() {
        let tmp = tempfile::tempdir().unwrap();
        let v = |ver: &str| json!({"id": "box", "name": "B", "version": ver,
            "instance_fields": {"box_enabled": {"type": "boolean"}}, "dependencies": {"helper": ">=0.1.0"}});
        write_ext(tmp.path(), "box", v("0.1.0"));
        std::fs::write(tmp.path().join("box/data.txt"), "old").unwrap();
        let mut mgr = ExtensionManager::new_isolated(tmp.path().to_str().unwrap());
        mgr.discover().unwrap();

        let pkg = zip_package(v("0.2.0"), "new");
        let err = ExtensionManager::stage_package(tmp.path(), "box", &pkg, Some("00")).unwrap_err();
        assert!(err.to_string().contains("sha256"), "{err}");
        let err = ExtensionManager::stage_package(tmp.path(), "other", &pkg, None).unwrap_err();
        assert!(format!("{err:#}").contains("does not match"), "{err:#}");
        assert_eq!(std::fs::read_to_string(tmp.path().join("box/data.txt")).unwrap(), "old");

        use sha2::{Digest, Sha256};
        let sha = hex::encode(Sha256::digest(&pkg));
        let staged = ExtensionManager::stage_package(tmp.path(), "box", &pkg, Some(&sha)).unwrap();
        let running: HashMap<String, Value> = [("box_enabled".to_string(), json!(true))].into_iter().collect();
        let err = mgr.install_staged("box", &staged, &[("srv", &running)], false).unwrap_err();
        assert_eq!(err.downcast_ref::<ExtensionError>().unwrap().error_code, "in_use");
        assert!(!staged.exists(), "refused package is cleaned up");

        let staged = ExtensionManager::stage_package(tmp.path(), "box", &pkg, None).unwrap();
        mgr.install_staged("box", &staged, &[], false).unwrap();
        assert_eq!(std::fs::read_to_string(tmp.path().join("box/data.txt")).unwrap(), "new");
        assert_eq!(mgr.discovered["box"].manifest.version, "0.2.0");
        assert!(!tmp.path().join(".old-box").exists());

        let remote: Vec<RemoteExtensionInfo> = serde_json::from_value(json!([
            {"id": "helper", "name": "H", "version": "0.1.0", "download_url": "x"}
        ])).unwrap();
        assert_eq!(mgr.missing_dependencies("box", &remote), ["helper"]);
    }

    #[test]
    fn dropped_zip_replaces_existing_folder() {
        let tmp = tempfile::tempdir().unwrap();
        let m = json!({"id": "box", "name": "B", "version": "0.1.0"});
        write_ext(tmp.path(), "box", m.clone());
        std::fs::write(tmp.path().join("box/data.txt"), "old").unwrap();
        std::fs::write(tmp.path().join("extension-box.zip"), zip_package(m, "new")).unwrap();
        let mut mgr = ExtensionManager::new_isolated(tmp.path().to_str().unwrap());
        mgr.discover().unwrap();
        assert_eq!(std::fs::read_to_string(tmp.path().join("box/data.txt")).unwrap(), "new");
        assert!(!tmp.path().join("extension-box.zip").exists());
        assert!(mgr.discovered.contains_key("box"));
    }

    #[test]
    fn hooks_run_in_priority_then_id_order() {
        let tmp = tempfile::tempdir().unwrap();
        for (id, priority) in [("beta", None), ("zeta", Some(10)), ("alpha", None), ("off", Some(0))] {
            let dir = tmp.path().join(id);
            std::fs::create_dir_all(&dir).unwrap();
            let mut manifest = json!({
                "id": id, "name": id, "version": "0.1.0",
                "python_modules": { "m": "m.py" },
                "hooks": { "daemon.startup": { "module": "m", "function": "f" } },
            });
            if let Some(p) = priority {
                manifest["priority"] = json!(p);
            }
            std::fs::write(dir.join("manifest.json"), manifest.to_string()).unwrap();
        }
        let mut mgr = ExtensionManager::new_isolated(tmp.path().to_str().unwrap());
        mgr.discover().unwrap();
        for id in ["alpha", "beta", "zeta"] {
            mgr.enabled.insert(id.to_string());
        }
        for _ in 0..5 {
            let order: Vec<&str> = mgr.hooks_for("daemon.startup").iter().map(|(e, _)| e.manifest.id.as_str()).collect();
            assert_eq!(order, ["zeta", "alpha", "beta"], "disabled 'off' is skipped, priority 10 before default 100");
        }
    }

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
        // 예전 이름은 읽을 때 지금 이름으로 바뀐다
        assert!(manifest.hooks.contains_key("server.stop"));
        assert!(!manifest.hooks.contains_key("server.post_stop"));
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
            incompatible: None,
            asset: None,
            install_dir: None,
            dependencies: HashMap::new(),
            has_gui: false,
            has_i18n: false,
            tags: vec![],
            homepage: None,
        }];

        let updates = mgr.check_updates_against(&remote);
        assert_eq!(updates.len(), 1);
        assert_eq!(updates[0].id, "test_ext");
        assert_eq!(updates[0].installed_version, "1.0.0");
        assert_eq!(updates[0].latest_version, "2.0.0");
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
            incompatible: None,
            asset: None,
            install_dir: None,
            dependencies: HashMap::new(),
            has_gui: false,
            has_i18n: false,
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
                "cli": ">=0.1.0"
            }
        }).to_string()).unwrap();

        let mut mgr = ExtensionManager::new_isolated(tmp.path().to_str().unwrap());
        mgr.discover().unwrap();

        let mut versions = HashMap::new();
        versions.insert("saba-core".to_string(), "0.5.0".to_string());
        versions.insert("gui".to_string(), "0.3.0".to_string());
        versions.insert("cli".to_string(), "0.1.0".to_string());

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

    // ═══════════════════════════════════════════════════════════════
    //  원격 manifest.json 파싱 호환성 테스트
    // ═══════════════════════════════════════════════════════════════



    /// 실제 manifest.json (로컬 설치용) 파싱 호환성 테스트
    /// 릴리즈 manifest의 `ext-*` 컴포넌트가 익스텐션 목록이 된다 (U9e)
    #[test]
    fn release_manifest_ext_components_become_the_catalog() {
        let manifest: saba_chan_updater_lib::relay::ReleaseManifest = serde_json::from_value(serde_json::json!({
            "version": "0.2.0", "channel": "stable", "platform": "windows-x64",
            "components": {
                "saba-core": {"version": "0.2.0", "asset": "c.zip", "sha256": "a", "url": "https://r2/c.zip"},
                "module-palworld": {"version": "0.1.0", "asset": "module-palworld.zip", "sha256": "b", "url": "https://r2/m.zip"},
                "ext-ue4-ini": {
                    "version": "0.1.2", "asset": "ext-ue4-ini.zip", "sha256": "c", "url": "https://r2/e.zip",
                    "install_dir": "extensions/ue4_ini",
                    "meta": {"name": "UE4 INI", "dependencies": ["steamcmd"], "min_app_version": "0.1.0"}
                }
            }
        })).unwrap();
        let list = ExtensionManager::from_release(&manifest);
        assert_eq!(list.len(), 1);
        let ext = &list[0];
        assert_eq!((ext.id.as_str(), ext.name.as_str(), ext.version.as_str()), ("ue4-ini", "UE4 INI", "0.1.2"));
        assert_eq!(ext.download_url, "https://r2/e.zip");
        assert_eq!(ext.sha256.as_deref(), Some("c"));
        assert_eq!(ext.install_dir.as_deref(), Some("extensions/ue4_ini"));
        assert!(ext.dependencies.contains_key("steamcmd"));
    }

    #[test]
    fn test_parse_docker_manifest() {
        let json = r#"{
          "id": "docker",
          "name": "Docker Isolation",
          "version": "0.1.0",
          "description": "Docker 컨테이너를 사용한 게임 서버 격리 실행",
          "author": "saba-chan",
          "min_app_version": "0.1.0",
          "dependencies": { "steamcmd": ">=0.1.0" },
          "python_modules": {
            "docker_engine": "docker_engine.py",
            "compose_manager": "compose_manager.py"
          },
          "hooks": {
            "daemon.startup": {
              "module": "docker_engine",
              "function": "ensure",
              "condition": null
            },
            "server.pre_start": {
              "module": "compose_manager",
              "function": "start",
              "condition": "instance.ext_data.docker_enabled"
            },
            "server.post_create": {
              "module": "compose_manager",
              "function": "provision",
              "condition": "instance.ext_data.docker_enabled",
              "async": true
            }
          },
          "gui": {
            "bundle": "gui/dist/docker.umd.js",
            "slots": {
              "ServerCard.badge": "DockerBadge",
              "ServerSettings.tab": "DockerTab"
            }
          },
          "cli": {
            "slots": {
              "InstanceList.badge": { "text": "🐳", "condition": "instance.ext_data.docker_enabled" }
            }
          },
          "module_config_section": "docker",
          "instance_fields": {
            "docker_enabled": { "type": "boolean", "default": false },
            "docker_cpu_limit": { "type": "number", "optional": true },
            "docker_memory_limit": { "type": "string", "optional": true }
          },
          "i18n_dir": "i18n/"
        }"#;

        let manifest: ExtensionManifest = serde_json::from_str(json).unwrap();
        assert_eq!(manifest.id, "docker");
        assert_eq!(manifest.dependencies.get("steamcmd").unwrap(), ">=0.1.0");
        assert_eq!(manifest.python_modules.len(), 2);
        assert_eq!(manifest.hooks.len(), 3);

        // condition: null → None
        let startup = &manifest.hooks["daemon.startup"];
        assert!(startup.condition.is_none());

        // condition: "..." → Some(...)
        let pre_start = &manifest.hooks["server.pre_start"];
        assert_eq!(pre_start.condition.as_deref(), Some("instance.ext_data.docker_enabled"));

        // async: true → is_async: Some(true)
        assert!(manifest.hooks.contains_key("server.post_create"));

        // gui
        let gui = manifest.gui.unwrap();
        assert_eq!(gui.bundle.as_deref(), Some("gui/dist/docker.umd.js"));
        assert_eq!(gui.slots.len(), 2);

        // cli
        let cli = manifest.cli.unwrap();
        assert_eq!(cli.slots.len(), 1);

        // module_config_section
        assert_eq!(manifest.module_config_section.as_deref(), Some("docker"));

        // instance_fields
        assert_eq!(manifest.instance_fields.len(), 3);
        assert_eq!(manifest.instance_fields["docker_enabled"].field_type, "boolean");

        // i18n_dir
        assert_eq!(manifest.i18n_dir.as_deref(), Some("i18n/"));
    }

    /// music manifest — gui.builtin, dependencies: [], i18n_dir: null
    #[test]
    fn test_parse_music_manifest() {
        let json = r#"{
          "id": "music",
          "name": "Music Bot",
          "version": "0.1.0",
          "dependencies": [],
          "python_modules": { "music_deps": "music_deps.py" },
          "hooks": {
            "daemon.startup": { "module": "music_deps", "function": "check_dependencies", "condition": null }
          },
          "gui": { "builtin": true, "slots": {} },
          "instance_fields": {},
          "i18n_dir": null
        }"#;

        let manifest: ExtensionManifest = serde_json::from_str(json).unwrap();
        assert_eq!(manifest.id, "music");
        assert!(manifest.dependencies.is_empty()); // [] → empty HashMap
        let gui = manifest.gui.unwrap();
        assert_eq!(gui.builtin, Some(true));
        assert!(gui.slots.is_empty());
        assert!(manifest.i18n_dir.is_none()); // null → None
    }
}
