//! manifest.json 계약: 타입, 읽기·검사, 데몬이 부르는 hook 이름, GUI slot, 요구 버전

use super::*;

/// 이 데몬이 아는 manifest.json 스키마 버전 (`schema_version`)
pub const EXTENSION_SCHEMA_VERSION: u32 = 1;

/// 데몬이 실제로 부르는 hook. 여기에 없는 이름에 바인딩하면 절대 실행되지 않는다
/// (이 밖에 이벤트 버스를 따르는 `event:<종류>`, [`event_hooks`])
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

/// 배열 형식과 맵 형식 모두 지원 (0.1.13까지 배포한 공식 익스텐션은 `[]`를 쓰고, 설치된 사본은 그 익스텐션이 업데이트될 때까지 남는다):
/// - `["steamcmd", "ue4-ini"]` → `{"steamcmd": "*", "ue4-ini": "*"}`
/// - `{"steamcmd": ">=0.1.0", "saba-core": ">=0.3.0"}` → 그대로
pub(super) fn deserialize_dependencies<'de, D>(deserializer: D) -> Result<HashMap<String, String>, D::Error>
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
    /// 배열 형식(`["steamcmd"]`)도 받는다 → `{ "steamcmd": "*" }` (`deserialize_dependencies`)
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
    pub permissions: crate::module_loader::ModulePermissions,
    /// 실행 방식 (모듈 `[runtime]`과 같은 표). `persistent`면 hook·명령을 오래 사는 작업자로 부른다 (실행기가 manifest에서 읽는다)
    #[serde(default, skip_serializing_if = "crate::plugin::Runtime::is_default")]
    pub runtime: crate::plugin::Runtime,
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
    pub inputs: Vec<crate::module_loader::CommandInput>,
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
    /// 번들이 처음 쓸 때 받는 정적 파일 폴더 (익스텐션 폴더 기준, 예: "gui/assets").
    /// 데몬이 `GET /api/extensions/:id/gui/assets/*path`로 번들과 같은 조건에 서빙한다 (03 §4.1)
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub assets: Option<String>,
    #[serde(default)]
    pub builtin: Option<bool>,
    /// slot id → 표시 정보 `{label?, icon?, description?}` (옛 모양: 컴포넌트 이름 문자열).
    /// 데몬은 그대로 넘기고, GUI가 자기 slot 표와 대조하고 이 창에 읽을 번들을 고른다 (03 §4.2)
    #[serde(default)]
    pub slots: HashMap<String, Value>,
}

/// CLI 매니페스트 — GUI의 GuiManifest에 대응하는 TUI 슬롯 선언
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CliManifest {
    /// slot_id → 슬롯별 JSON 설정 (데이터 기반 렌더링)
    /// 예: "InstanceList.badge" → { "text": "🐳", "condition": "..." }
    #[serde(default)]
    pub slots: HashMap<String, Value>,
}

/// 데이터 slot 이름 (Q-7). GUI 어휘로 통일했고, 옛 CLI 이름은 [`SLOT_ALIASES`]로 받는다
pub const SLOT_BADGE: &str = "ServerCard.badge";

pub const SLOT_STATUS: &str = "ServerDetail.status";

pub const SLOT_OPTIONS: &str = "AddServer.options";

/// 옛 이름 → 통일 이름. 한 릴리스 동안 받고 지운다
pub const SLOT_ALIASES: &[(&str, &str)] = &[
    ("InstanceList.badge", SLOT_BADGE),
    ("InstanceDetail.status", SLOT_STATUS),
    ("CreateInstance.options", SLOT_OPTIONS),
];

/// manifest `cli.slots` → 통일 이름 키, 값은 배열 (하나만 적었으면 한 칸짜리). 옛 이름과 새 이름을 함께 쓰면 이어 붙인다
pub fn normalized_slots(cli: &CliManifest) -> serde_json::Map<String, Value> {
    let mut keys: Vec<&String> = cli.slots.keys().collect();
    keys.sort();
    let mut out = serde_json::Map::new();
    for key in keys {
        let name = SLOT_ALIASES.iter().find(|(old, _)| old == key).map_or(key.as_str(), |(_, new)| new);
        let items = match &cli.slots[key] {
            Value::Array(a) => a.clone(),
            other => vec![other.clone()],
        };
        if let Value::Array(list) = out.entry(name.to_string()).or_insert_with(|| Value::Array(Vec::new())) {
            list.extend(items);
        }
    }
    out
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
/// `saba-core`는 모듈의 `[extensions.saba-core]`가 데몬 몫이라 쓸 수 없다
pub fn is_valid_extension_id(id: &str) -> bool {
    !id.is_empty()
        && id.len() <= 64
        && id.chars().all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_')
        && id != crate::module_loader::DAEMON_TABLE
}

pub(super) fn ensure_valid_extension_id(id: &str) -> Result<()> {
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
    // 파이썬 모듈은 익스텐션 폴더 안의 파일이어야 한다: 예전 형식(경고만)이어도 받지 않는다
    let mut modules: Vec<_> = manifest.python_modules.iter().collect();
    modules.sort();
    if let Some((name, path)) = modules.into_iter().find(|(_, path)| !crate::utils::is_inner_relative_path(path)) {
        anyhow::bail!("python module '{}' path '{}' must be a file inside the extension folder", name, path);
    }

    let mut problems: Vec<String> = unknown.into_iter().map(|k| format!("unknown key '{}'", k)).collect();
    let mut hooks: Vec<_> = manifest.hooks.iter().collect();
    hooks.sort_by_key(|(name, _)| name.as_str());
    for (name, binding) in hooks {
        let on_event = event_hooks::is_event_hook(name);
        if !FIRED_HOOKS.contains(&name.as_str()) && !on_event {
            problems.push(format!("hook '{}' is never fired by the daemon", name));
        }
        // 이벤트에는 extension_data가 없어 조건이 늘 거짓이다
        if on_event && binding.condition.is_some() {
            problems.push(format!("event hook '{}' cannot have a condition", name));
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

/// `min_app_version`을 이 데몬이 만족하지 않으면 사용자에게 보일 이유
pub(super) fn app_version_problem(min_app_version: Option<&str>) -> Option<String> {
    min_app_version
        .filter(|v| !saba_chan_updater_lib::version::satisfies(APP_VERSION, v))
        .map(|v| format!("requires saba-chan {} or newer (this is {})", v, APP_VERSION))
}

/// `min_app_version`을 이 데몬이 만족하지 않으면 켜거나 설치하지 않는다
pub(super) fn require_app_version(ext_id: &str, min_app_version: Option<&str>) -> Result<()> {
    match min_app_version.filter(|v| !saba_chan_updater_lib::version::satisfies(APP_VERSION, v)) {
        Some(required) => Err(ExtensionError::app_too_old(ext_id, required).into()),
        None => Ok(()),
    }
}
