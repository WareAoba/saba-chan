use std::fs;
use std::path::{Path, PathBuf};
use std::sync::RwLock;
use std::io::Read;
use anyhow::Result;
use serde::{Deserialize, Serialize};
use zip::ZipArchive;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ModuleMetadata {
    pub name: String,
    pub version: String,
    pub description: Option<String>,
    pub entry: String,  // lifecycle.py 경로
    pub process_name: Option<String>,  // config.process_name
    pub default_port: Option<u16>,  // config.default_port
    pub executable_path: Option<String>,  // config.executable_path
    #[serde(default)]
    pub server_executable: Option<String>,  // config.server_executable (바이너리 파일명, e.g. "PalServer.exe")
    #[serde(default)]
    pub icon: Option<String>,  // 아이콘 파일명 (icon.png 등)
    #[serde(default)]
    pub stop_command: Option<String>,  // config.stop_command (e.g. "stop" for Minecraft)
    /// Graceful stop 타임아웃 (초). 설정하지 않으면 기본값 30초 사용.
    /// 게임 서버마다 종료 시간이 다르므로 모듈별로 설정.
    #[serde(default)]
    pub stop_timeout: Option<u64>,  // config.stop_timeout
    #[serde(default)]
    pub log_pattern: Option<String>,  // regex pattern for log level extraction (e.g. "/(?P<level>INFO|WARN|ERROR|DEBUG)/")
    #[serde(default)]
    pub interaction_mode: Option<String>,  // "console" or "commands" (from [protocols])
    #[serde(default)]
    pub protocols_supported: Option<Vec<String>>,  // ["rcon", "stdin", "rest"] etc.
    #[serde(default)]
    pub protocols_default: Option<String>,  // default protocol mode
    #[serde(default)]
    pub settings: Option<ModuleSettings>,  // 설정 스키마
    #[serde(default)]
    pub commands: Option<ModuleCommands>,  // 명령어 스키마
    #[serde(default)]
    pub syntax_highlight: Option<SyntaxHighlight>,  // 콘솔 구문 하이라이팅 규칙
    #[serde(default)]
    pub install: Option<ModuleInstallConfig>,  // [install] 설치 방식 (steamcmd 등)
    /// npm package.json 스타일 의존성: 이 모듈이 요구하는 다른 컴포넌트의 최소 버전
    /// 예: { "saba-core": ">=0.3.0", "ext-steamcmd": ">=1.0.0" }
    #[serde(default)]
    pub dependencies: std::collections::HashMap<String, String>,
    /// 데몬 표준 키 ↔ 게임별 설정 키 매핑
    /// 예: { "rest_password": "AdminPassword" } → 데몬의 rest_password가 게임 INI의 AdminPassword에 대응
    /// 모듈의 get_launch_command 반환 시 이 맵을 통해 자동 동기화
    #[serde(default)]
    pub credential_map: std::collections::HashMap<String, String>,
    /// [extension.*] 섹션들을 범용으로 저장 (예: extensions["<ext_id>"] = {...})
    #[serde(default)]
    pub extensions: std::collections::HashMap<String, serde_json::Value>,
    #[serde(default)]
    pub process_patterns: Vec<String>,  // [detection].process_patterns — 컨테이너 내 서버 프로세스 탐지용
    /// 커맨드라인 인수 패턴 — java.exe 같은 범용 프로세스를 구분하기 위한 추가 매칭 패턴
    /// 예: ["server.jar"] (Minecraft), ["zombie.network.GameServer"] (Zomboid)
    #[serde(default)]
    pub cmd_patterns: Vec<String>,  // [detection].cmd_patterns
    /// 디렉토리 시그니처 — 기존 서버 폴더 내 파일/폴더명으로 모듈을 자동 감지
    /// 예: ["PalServer.exe"] (Palworld), ["server.jar", "server.properties"] (Minecraft)
    #[serde(default)]
    pub dir_signatures: Vec<String>,  // [detection].dir_signatures
    /// 모듈 별명 — Discord/CLI에서 다국어 이름으로 모듈을 찾을 수 있게 함
    /// 예: ["팰월드", "pw", "palworld"]
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub module_aliases: Vec<String>,
    /// 명령어 별명 — 각 명령어에 대한 다국어/단축 별명
    /// 예: { "start": { "aliases": ["시작", "실행"], "description": "Start the server" } }
    #[serde(default, skip_serializing_if = "std::collections::HashMap::is_empty")]
    pub command_aliases: std::collections::HashMap<String, CommandAliasInfo>,
}

/// 명령어 별명 정보
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CommandAliasInfo {
    #[serde(default)]
    pub aliases: Vec<String>,
    #[serde(default)]
    pub description: Option<String>,
}

impl ModuleMetadata {
    /// 설정 필드의 기본값을 가져옵니다
    pub fn get_setting_default<T: std::str::FromStr>(&self, field_name: &str) -> Option<T> {
        self.settings.as_ref().and_then(|s| {
            s.fields.iter().find(|f| f.name == field_name).and_then(|f| {
                f.default.as_ref().and_then(|d| {
                    match d {
                        toml::Value::Integer(i) => i.to_string().parse().ok(),
                        toml::Value::String(s) => s.parse().ok(),
                        toml::Value::Float(f) => f.to_string().parse().ok(),
                        toml::Value::Boolean(b) => b.to_string().parse().ok(),
                        _ => None,
                    }
                })
            })
        })
    }
    
    /// RCON 기본 포트를 가져옵니다 (모듈에 정의되지 않으면 25575)
    pub fn default_rcon_port(&self) -> u16 {
        self.get_setting_default("rcon_port").unwrap_or(25575)
    }
    
    /// REST API 기본 포트를 가져옵니다 (모듈에 정의되지 않으면 8212)
    pub fn default_rest_port(&self) -> u16 {
        self.get_setting_default("rest_port").unwrap_or(8212)
    }
    
    /// REST API 기본 호스트를 가져옵니다 (모듈에 정의되지 않으면 127.0.0.1)
    pub fn default_rest_host(&self) -> String {
        self.get_setting_default("rest_host").unwrap_or_else(|| "127.0.0.1".to_string())
    }

    /// SteamCMD app ID를 가져옵니다 (install.method == "steamcmd"일 때)
    #[allow(dead_code)]
    pub fn steam_app_id(&self) -> Option<u32> {
        self.install.as_ref()
            .filter(|i| i.method == "steamcmd")
            .and_then(|i| i.app_id)
    }

    /// 특정 익스텐션의 모듈 설정이 있는지 확인합니다
    #[allow(dead_code)]
    pub fn has_extension_config(&self, ext_name: &str) -> bool {
        self.extensions.contains_key(ext_name)
    }

    /// 컴포넌트 버전 의존성을 확인합니다.
    /// `installed_versions`: 컴포넌트 키 → 설치된 버전 (예: "saba-core" → "0.3.0")
    /// 미충족 의존성 목록을 (컴포넌트키, 요구버전, 설치버전Option) 튜플로 반환합니다.
    pub fn check_dependencies(
        &self,
        installed_versions: &std::collections::HashMap<String, String>,
    ) -> Vec<(String, String, Option<String>)> {
        use saba_chan_updater_lib::version::SemVer;

        self.dependencies.iter().filter_map(|(component_key, min_version_str)| {
            let min_clean = min_version_str.trim_start_matches(">=").trim();
            let installed = installed_versions.get(component_key);
            let satisfied = installed.is_some_and(|v| {
                match (SemVer::parse(v), SemVer::parse(min_clean)) {
                    (Some(installed_v), Some(required_v)) => installed_v >= required_v,
                    _ => false,
                }
            });
            if satisfied {
                None
            } else {
                Some((
                    component_key.clone(),
                    min_version_str.clone(),
                    installed.cloned(),
                ))
            }
        }).collect()
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ModuleSettings {
    pub fields: Vec<SettingField>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ModuleCommands {
    pub fields: Vec<CommandField>,
}

/// 콘솔 구문 하이라이팅 — 모듈별로 정의되는 로그 하이라이팅 규칙
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SyntaxHighlight {
    pub rules: Vec<HighlightRule>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct HighlightRule {
    /// 이름/식별자 (디버그·GUI 표시용)
    pub name: String,
    /// 매칭할 정규식 패턴 — 명명 캡처 `(?P<hl>...)` 이 있으면 해당 부분만, 없으면 전체 매치
    pub pattern: String,
    /// CSS 색상값 (예: "#f38ba8", "#a6e3a1") 또는 시맨틱 토큰 ("error", "warn", "info", …)
    pub color: String,
    /// 배경색 (optional, CSS 색상값 또는 시맨틱 토큰)
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub background: Option<String>,
    /// 굵게 표시 여부
    #[serde(default)]
    pub bold: bool,
    /// 기울임 표시 여부
    #[serde(default)]
    pub italic: bool,
    /// 밑줄 표시 여부
    #[serde(default)]
    pub underline: bool,
    /// 우선순위 (기본 0, 큰 값이 먼저 적용됨)
    #[serde(default)]
    pub priority: i32,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CommandField {
    pub name: String,
    pub label: String,
    pub description: Option<String>,
    pub method: Option<String>,  // "rest", "rcon", "both"
    pub http_method: Option<String>,  // "GET", "POST" (REST only)
    pub endpoint_template: Option<String>,  // REST endpoint template
    pub rcon_template: Option<String>,  // RCON command template (e.g., "kick {userid}")
    #[serde(default)]
    pub inputs: Vec<CommandInput>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CommandInput {
    pub name: String,
    pub label: Option<String>,
    #[serde(rename = "type")]
    pub input_type: Option<String>,
    pub required: Option<bool>,
    pub placeholder: Option<String>,
    #[serde(default)]
    pub default: Option<toml::Value>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SettingField {
    pub name: String,
    pub field_type: String,  // "text", "number", "password", "file", "select", etc.
    pub label: String,
    pub description: Option<String>,
    pub required: Option<bool>,
    pub default: Option<toml::Value>,
    pub min: Option<f64>,
    pub max: Option<f64>,
    pub step: Option<f64>,
    pub options: Option<Vec<String>>,
    pub group: Option<String>,  // "basic" (default), "advanced", "saba-chan" etc.
}

#[derive(Debug, Clone)]
pub struct LoadedModule {
    pub metadata: ModuleMetadata,
    pub path: String,  // 압축 해제된 디렉토리 또는 원본 디렉토리 절대 경로
    #[allow(dead_code)]
    pub is_zip: bool,  // ZIP에서 로드되었는지 여부
}

// ── module.toml 강타입 스키마 (serde 직접 역직렬화) ──

/// module.toml 최상위 구조
#[derive(Debug, Deserialize)]
struct ModuleToml {
    module: ModuleSection,
    #[serde(default, rename = "update")]
    _update: Option<UpdateSection>,
    #[serde(default)]
    protocols: Option<ProtocolsSection>,
    #[serde(default)]
    config: Option<ConfigSection>,
    #[serde(default, rename = "detection")]
    _detection: Option<DetectionSection>,
    #[serde(default)]
    settings: Option<SettingsSection>,
    #[serde(default)]
    commands: Option<CommandsSection>,
    #[serde(default)]
    syntax_highlight: Option<SyntaxHighlightSection>,
    #[serde(default)]
    install: Option<InstallSectionToml>,
    /// [docker] 익스텐션 설정 섹션 (컨테이너 격리용)
    #[serde(default, rename = "docker")]
    container: Option<ContainerSectionToml>,
    /// [credential_map] 데몬 표준 키 ↔ 게임별 설정 키 매핑
    /// 예: rest_password = "AdminPassword" → 데몬의 rest_password가 게임 INI의 AdminPassword에 대응
    #[serde(default)]
    credential_map: Option<std::collections::HashMap<String, String>>,
    /// [aliases] 모듈 및 명령어 별명 (Discord/CLI 다국어 지원)
    #[serde(default)]
    aliases: Option<AliasesSection>,
}

#[derive(Debug, Deserialize)]
struct ModuleSection {
    name: String,
    version: String,
    #[serde(default)]
    description: Option<String>,
    entry: String,
    #[serde(default, rename = "game_name")]
    _game_name: Option<String>,
    #[serde(default, rename = "display_name")]
    _display_name: Option<String>,
    #[serde(default)]
    icon: Option<String>,
    #[serde(default)]
    log_pattern: Option<String>,
    /// 컴포넌트 의존성 — 이 모듈이 요구하는 다른 컴포넌트의 최소 버전
    /// 예: { "saba-core" = ">=0.3.0", "ext-steamcmd" = ">=1.0.0" }
    #[serde(default)]
    dependencies: std::collections::HashMap<String, String>,
}

#[derive(Debug, Deserialize)]
struct UpdateSection {
    #[serde(default)]
    #[allow(dead_code)]
    github_repo: Option<String>,
}

#[derive(Debug, Deserialize)]
struct ProtocolsSection {
    #[serde(default)]
    supported: Option<Vec<String>>,
    #[serde(default)]
    default: Option<String>,
    #[serde(default)]
    interaction_mode: Option<String>,
}

#[derive(Debug, Deserialize)]
struct ConfigSection {
    #[serde(default)]
    executable_path: Option<String>,
    #[serde(default)]
    server_executable: Option<String>,
    #[serde(default)]
    process_name: Option<String>,
    #[serde(default)]
    default_port: Option<u16>,
    #[serde(default)]
    stop_command: Option<String>,
    /// Graceful stop 타임아웃 (초). 예: Minecraft=30, ARK=120
    #[serde(default)]
    stop_timeout: Option<u64>,
    // Allow additional unknown fields to pass through
    #[serde(flatten)]
    _extra: std::collections::HashMap<String, toml::Value>,
}

#[derive(Debug, Deserialize)]
struct DetectionSection {
    #[serde(default)]
    process_patterns: Option<Vec<String>>,
    #[serde(default)]
    cmd_patterns: Option<Vec<String>>,
    #[serde(default)]
    #[allow(dead_code)]
    common_paths: Option<Vec<String>>,
    /// 디렉토리 내 파일/폴더 시그니처 — 기존 서버 마이그레이션 시 자동 감지에 사용
    #[serde(default)]
    dir_signatures: Option<Vec<String>>,
}

#[derive(Debug, Deserialize)]
struct SettingsSection {
    #[serde(default)]
    fields: Vec<SettingFieldToml>,
}

#[derive(Debug, Deserialize)]
struct SettingFieldToml {
    name: String,
    #[serde(rename = "type", default = "default_field_type")]
    field_type: String,
    #[serde(default)]
    label: Option<String>,
    #[serde(default)]
    description: Option<String>,
    #[serde(default)]
    required: Option<bool>,
    #[serde(default)]
    default: Option<toml::Value>,
    #[serde(default)]
    min: Option<f64>,
    #[serde(default)]
    max: Option<f64>,
    #[serde(default)]
    step: Option<f64>,
    #[serde(default)]
    options: Option<Vec<String>>,
    #[serde(default)]
    group: Option<String>,
}

fn default_field_type() -> String { "text".to_string() }

/// module.toml [install] 섹션
#[derive(Debug, Deserialize)]
struct InstallSectionToml {
    #[serde(default)]
    method: Option<String>,
    #[serde(default)]
    app_id: Option<u32>,
    #[serde(default)]
    anonymous: Option<bool>,
    #[serde(default)]
    install_subdir: Option<String>,
    #[serde(default)]
    platform: Option<String>,
    #[serde(default)]
    download_url: Option<String>,
    #[serde(default)]
    beta: Option<String>,
    /// 이 설치 방식이 요구하는 익스텐션 ID 목록
    /// 예: requires_extensions = ["steamcmd"]
    #[serde(default)]
    requires_extensions: Vec<String>,
}

/// module.toml [docker] 섹션 — 컨테이너 격리 익스텐션 설정
#[derive(Debug, Deserialize)]
struct ContainerSectionToml {
    #[serde(default)]
    image: Option<String>,
    #[serde(default)]
    working_dir: Option<String>,
    #[serde(default)]
    restart: Option<String>,
    #[serde(default)]
    command: Option<String>,
    #[serde(default)]
    entrypoint: Option<String>,
    #[serde(default)]
    user: Option<String>,
    #[serde(default)]
    ports: Vec<String>,
    #[serde(default)]
    volumes: Vec<String>,
    #[serde(default)]
    environment: std::collections::HashMap<String, String>,
    #[serde(default)]
    dockerfile: Option<String>,
    /// CPU 제한 (코어 수, 예: 2.0)
    #[serde(default)]
    cpu_limit: Option<f64>,
    /// 메모리 제한 (예: "4g", "512m")
    #[serde(default)]
    memory_limit: Option<String>,
}

/// ModuleMetadata에서 사용하는 공개 설치 설정
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ModuleInstallConfig {
    pub method: String,
    #[serde(default)]
    pub app_id: Option<u32>,
    #[serde(default = "default_true_mod")]
    pub anonymous: bool,
    #[serde(default)]
    pub install_subdir: Option<String>,
    #[serde(default)]
    pub platform: Option<String>,
    #[serde(default)]
    pub download_url: Option<String>,
    #[serde(default)]
    pub beta: Option<String>,
    /// 이 설치 방식이 요구하는 익스텐션 ID 목록
    /// 예: ["steamcmd"]
    #[serde(default)]
    pub requires_extensions: Vec<String>,
}

fn default_true_mod() -> bool { true }

/// module.toml [aliases] 섹션
#[derive(Debug, Deserialize)]
struct AliasesSection {
    #[serde(default)]
    module_aliases: Vec<String>,
    #[serde(default)]
    commands: std::collections::HashMap<String, CommandAliasSectionToml>,
}

/// module.toml [aliases.commands.*] 항목
#[derive(Debug, Deserialize)]
struct CommandAliasSectionToml {
    #[serde(default)]
    aliases: Vec<String>,
    #[serde(default)]
    description: Option<String>,
}

/// Container isolation extension configuration for modules.
/// Parsed from module.toml [docker] section and stored in ModuleMetadata.extensions.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ContainerExtensionConfig {
    /// Container image (e.g. "cm2network/steamcmd:latest")
    pub image: String,
    /// Working directory inside the container
    #[serde(default)]
    pub working_dir: Option<String>,
    /// Restart policy (default: "unless-stopped")
    #[serde(default = "default_restart_policy")]
    pub restart: String,
    /// Container command (supports template variables like {port})
    #[serde(default)]
    pub command: Option<String>,
    /// Container entrypoint override
    #[serde(default)]
    pub entrypoint: Option<String>,
    /// User to run the container as (e.g. "1000:1000")
    #[serde(default)]
    pub user: Option<String>,
    /// Port mappings: "{host_port}:{container_port}/protocol" with template variables
    #[serde(default)]
    pub ports: Vec<String>,
    /// Volume mounts: "host_path:container_path"
    #[serde(default)]
    pub volumes: Vec<String>,
    /// Environment variables (supports template variables)
    #[serde(default)]
    pub environment: std::collections::HashMap<String, String>,
    /// Optional Dockerfile path (relative to module directory) for custom builds
    #[serde(default)]
    pub dockerfile: Option<String>,
    /// Optional: additional compose service options as raw YAML
    #[serde(default)]
    pub extra_options: std::collections::HashMap<String, String>,
    /// CPU limit (number of cores, e.g. 2.0)
    #[serde(default)]
    pub cpu_limit: Option<f64>,
    /// Memory limit (e.g. "4g", "512m")
    #[serde(default)]
    pub memory_limit: Option<String>,
}

fn default_restart_policy() -> String { "unless-stopped".to_string() }

#[derive(Debug, Deserialize)]
struct CommandsSection {
    #[serde(default)]
    fields: Vec<CommandFieldToml>,
}

#[derive(Debug, Deserialize)]
struct CommandFieldToml {
    name: String,
    #[serde(default)]
    label: Option<String>,
    #[serde(default)]
    description: Option<String>,
    #[serde(default)]
    method: Option<String>,
    #[serde(default)]
    http_method: Option<String>,
    #[serde(default)]
    endpoint_template: Option<String>,
    #[serde(default)]
    rcon_template: Option<String>,
    #[serde(default)]
    command_template: Option<String>,
    #[serde(default)]
    inputs: Vec<CommandInputToml>,
}

#[derive(Debug, Deserialize)]
struct CommandInputToml {
    name: String,
    #[serde(default)]
    label: Option<String>,
    #[serde(rename = "type", default)]
    input_type: Option<String>,
    #[serde(default)]
    required: Option<bool>,
    #[serde(default)]
    placeholder: Option<String>,
    #[serde(default)]
    default: Option<toml::Value>,
}

#[derive(Debug, Deserialize)]
struct SyntaxHighlightSection {
    #[serde(default)]
    rules: Vec<HighlightRuleToml>,
}

#[derive(Debug, Deserialize)]
struct HighlightRuleToml {
    name: String,
    pattern: String,
    color: String,
    #[serde(default)]
    background: Option<String>,
    #[serde(default)]
    bold: bool,
    #[serde(default)]
    italic: bool,
    #[serde(default)]
    underline: bool,
    #[serde(default)]
    priority: i32,
}

/// [settings] 섹션 → 공개 `ModuleSettings`. 필드가 없으면 None.
fn convert_settings(section: Option<SettingsSection>) -> Option<ModuleSettings> {
    let s = section?;
    let fields: Vec<SettingField> = s.fields.into_iter().map(|f| SettingField {
        name: f.name.clone(),
        field_type: f.field_type,
        label: f.label.unwrap_or(f.name),
        description: f.description,
        required: f.required,
        default: f.default,
        min: f.min,
        max: f.max,
        step: f.step,
        options: f.options,
        group: f.group,
    }).collect();
    if fields.is_empty() { None } else { Some(ModuleSettings { fields }) }
}

/// [commands] 섹션 → 공개 `ModuleCommands`. 필드가 없으면 None.
fn convert_commands(section: Option<CommandsSection>) -> Option<ModuleCommands> {
    let c = section?;
    let fields: Vec<CommandField> = c.fields.into_iter().map(|f| CommandField {
        name: f.name.clone(),
        label: f.label.unwrap_or(f.name),
        description: f.description,
        method: f.method,
        http_method: f.http_method,
        endpoint_template: f.endpoint_template,
        rcon_template: f.rcon_template.or(f.command_template),
        inputs: f.inputs.into_iter().map(|i| CommandInput {
            name: i.name,
            label: i.label,
            input_type: i.input_type,
            required: i.required,
            placeholder: i.placeholder,
            default: i.default,
        }).collect(),
    }).collect();
    if fields.is_empty() { None } else { Some(ModuleCommands { fields }) }
}

/// [syntax_highlight] 섹션 → 공개 `SyntaxHighlight`. 규칙이 없으면 None.
fn convert_syntax_highlight(section: Option<SyntaxHighlightSection>) -> Option<SyntaxHighlight> {
    let sh = section?;
    let rules: Vec<HighlightRule> = sh.rules.into_iter().map(|r| {
        // JS 호환성 린트: 런타임에 실패할 패턴을 사전 경고.
        lint_highlight_pattern(&r.name, &r.pattern);
        HighlightRule {
            name: r.name,
            pattern: r.pattern,
            color: r.color,
            background: r.background,
            bold: r.bold,
            italic: r.italic,
            underline: r.underline,
            priority: r.priority,
        }
    }).collect();
    if rules.is_empty() { None } else { Some(SyntaxHighlight { rules }) }
}

/// [docker] 섹션 → `extensions` 맵 (image가 있을 때만 "docker" 키 삽입).
fn convert_container_extensions(
    container: Option<ContainerSectionToml>,
) -> std::collections::HashMap<String, serde_json::Value> {
    let mut ext_map = std::collections::HashMap::new();
    if let Some(d) = container {
        if let Some(img) = d.image {
            let cfg = ContainerExtensionConfig {
                image: img,
                working_dir: d.working_dir,
                restart: d.restart.unwrap_or_else(|| "unless-stopped".to_string()),
                command: d.command,
                entrypoint: d.entrypoint,
                user: d.user,
                ports: d.ports,
                volumes: d.volumes,
                environment: d.environment,
                dockerfile: d.dockerfile,
                extra_options: std::collections::HashMap::new(),
                cpu_limit: d.cpu_limit,
                memory_limit: d.memory_limit,
            };
            if let Ok(val) = serde_json::to_value(cfg) {
                ext_map.insert("docker".to_string(), val);
            }
        }
    }
    ext_map
}

impl ModuleToml {
    /// ModuleToml → ModuleMetadata 변환
    fn into_metadata(self) -> ModuleMetadata {
        let settings = convert_settings(self.settings);
        let commands = convert_commands(self.commands);
        let syntax_highlight = convert_syntax_highlight(self.syntax_highlight);

        ModuleMetadata {
            name: self.module.name,
            version: self.module.version,
            description: self.module.description,
            entry: self.module.entry,
            icon: self.module.icon,
            log_pattern: self.module.log_pattern,
            dependencies: self.module.dependencies,
            credential_map: self.credential_map.unwrap_or_default(),
            process_name: self.config.as_ref().and_then(|c| c.process_name.clone()),
            default_port: self.config.as_ref().and_then(|c| c.default_port),
            executable_path: self.config.as_ref().and_then(|c| c.executable_path.clone()),
            server_executable: self.config.as_ref().and_then(|c| c.server_executable.clone()),
            stop_command: self.config.as_ref().and_then(|c| c.stop_command.clone()),
            stop_timeout: self.config.as_ref().and_then(|c| c.stop_timeout),
            interaction_mode: self.protocols.as_ref().and_then(|p| p.interaction_mode.clone()),
            protocols_supported: self.protocols.as_ref().and_then(|p| p.supported.clone()),
            protocols_default: self.protocols.as_ref().and_then(|p| p.default.clone()),
            settings,
            commands,
            syntax_highlight,
            install: self.install.map(|i| ModuleInstallConfig {
                method: i.method.unwrap_or_else(|| "manual".to_string()),
                app_id: i.app_id,
                anonymous: i.anonymous.unwrap_or(true),
                install_subdir: i.install_subdir,
                platform: i.platform,
                download_url: i.download_url,
                beta: i.beta,
                requires_extensions: i.requires_extensions,
            }),
            extensions: convert_container_extensions(self.container),
            process_patterns: self._detection.as_ref()
                .and_then(|d| d.process_patterns.clone())
                .unwrap_or_default(),
            cmd_patterns: self._detection.as_ref()
                .and_then(|d| d.cmd_patterns.clone())
                .unwrap_or_default(),
            dir_signatures: self._detection.as_ref()
                .and_then(|d| d.dir_signatures.clone())
                .unwrap_or_default(),
            module_aliases: self.aliases.as_ref()
                .map(|a| a.module_aliases.clone())
                .unwrap_or_default(),
            command_aliases: self.aliases.map(|a| {
                a.commands.into_iter().map(|(name, info)| {
                    (name, CommandAliasInfo {
                        aliases: info.aliases,
                        description: info.description,
                    })
                }).collect()
            }).unwrap_or_default(),
        }
    }
}

/// TOML 문자열에서 ModuleMetadata를 파싱합니다.
/// 필수 필드 누락 시 명확한 에러 메시지를 반환합니다.
fn parse_module_toml(content: &str) -> Result<ModuleMetadata> {
    let toml_data: ModuleToml = toml::from_str(content)
        .map_err(|e| anyhow::anyhow!("Failed to parse module.toml: {}", e))?;
    Ok(toml_data.into_metadata())
}

pub struct ModuleLoader {
    modules_dir: String,
    cached_modules: RwLock<Option<Vec<LoadedModule>>>,
}

impl ModuleLoader {
    pub fn new(modules_dir: &str) -> Self {
        Self {
            modules_dir: modules_dir.to_string(),
            cached_modules: RwLock::new(None),
        }
    }

    /// 모듈 디렉토리 경로 반환
    pub fn modules_dir(&self) -> &str {
        &self.modules_dir
    }

    /// 캐시를 무효화합니다 (새로운 모듈이 추가되었을 때 호출)
    pub fn invalidate_cache(&self) {
        *self.cached_modules.write().unwrap() = None;
        tracing::info!("Module cache invalidated");
    }

    /// 모듈 디렉터리에서 모든 사용 가능한 모듈 발견 (ZIP 및 폴더 모두 지원)
    pub fn discover_modules(&self) -> Result<Vec<LoadedModule>> {
        // 캐시 확인
        if let Some(modules) = self.cached_modules.read().unwrap().as_ref() {
            return Ok(modules.clone());
        }

        let mut modules = Vec::new();

        if !Path::new(&self.modules_dir).exists() {
            tracing::warn!("Modules directory does not exist: {}", self.modules_dir);
            *self.cached_modules.write().unwrap() = Some(modules.clone());
            return Ok(modules);
        }

        for entry in fs::read_dir(&self.modules_dir)? {
            let entry = entry?;
            let path = entry.path();

            // ZIP 파일 체크
            if path.is_file() && path.extension().and_then(|s| s.to_str()) == Some("zip") {
                match self.load_module_from_zip(&path) {
                    Ok(module) => {
                        tracing::info!(
                            "Discovered ZIP module: {} v{} from {}",
                            module.metadata.name, module.metadata.version, path.display()
                        );
                        modules.push(module);
                    }
                    Err(e) => {
                        tracing::warn!(
                            "Failed to load ZIP module from {}: {}",
                            path.display(),
                            e
                        );
                    }
                }
            }
            // 폴더 체크 (기존 방식)
            else if path.is_dir() {
                let toml_path = path.join("module.toml");
                if toml_path.exists() {
                    match self.load_module_from_dir(&path) {
                        Ok(module) => {
                            tracing::info!(
                                "Discovered folder module: {} v{} at {}",
                                module.metadata.name, module.metadata.version, module.path
                            );
                            modules.push(module);
                        }
                        Err(e) => {
                            tracing::warn!(
                                "Failed to load folder module from {}: {}",
                                path.display(),
                                e
                            );
                        }
                    }
                }
            }
        }

        *self.cached_modules.write().unwrap() = Some(modules.clone());
        Ok(modules)
    }

    /// 폴더에서 개별 모듈 로드 (module.toml 파싱)
    fn load_module_from_dir(&self, module_path: &Path) -> Result<LoadedModule> {
        let toml_path = module_path.join("module.toml");
        let content = fs::read_to_string(&toml_path)
            .map_err(|e| anyhow::anyhow!("Failed to read {}: {}", toml_path.display(), e))?;
        let metadata = parse_module_toml(&content)?;

        Ok(LoadedModule {
            metadata,
            path: module_path.to_string_lossy().to_string(),
            is_zip: false,
        })
    }

    /// ZIP 파일에서 모듈 로드 (압축 해제 후 임시 디렉토리 사용)
    fn load_module_from_zip(&self, zip_path: &Path) -> Result<LoadedModule> {
        let file = fs::File::open(zip_path)?;
        let mut archive = ZipArchive::new(file)?;

        // ZIP 내부에서 module.toml 찾기
        let mut toml_content = String::new();
        for i in 0..archive.len() {
            let mut file = archive.by_index(i)?;
            let name = file.name().to_string();
            if name.ends_with("module.toml") {
                file.read_to_string(&mut toml_content)?;
                break;
            }
        }

        if toml_content.is_empty() {
            return Err(anyhow::anyhow!("No module.toml found in ZIP"));
        }

        // 강타입 TOML 파싱
        let metadata = parse_module_toml(&toml_content)?;
        anyhow::ensure!(
            crate::utils::is_single_path_component(&metadata.name),
            "Invalid ZIP module name"
        );

        // 임시 디렉토리에 압축 해제
        let temp_dir = tempfile::tempdir()?;
        let extract_path = temp_dir.path().to_path_buf();
        
        let file = fs::File::open(zip_path)?;
        let mut archive = ZipArchive::new(file)?;
        archive.extract(&extract_path)?;

        // 임시 디렉토리 경로를 영구적으로 유지 (tempdir는 Drop시 삭제되므로)
        // 대신 modules/.extracted/ 디렉토리 사용
        let extraction_root = PathBuf::from(&self.modules_dir).join(".extracted");
        if let Ok(metadata) = fs::symlink_metadata(&extraction_root) {
            anyhow::ensure!(!metadata.file_type().is_symlink(), "Module extraction root cannot be a symbolic link");
        }
        let extracted_dir = extraction_root.join(&metadata.name);
        if let Ok(metadata) = fs::symlink_metadata(&extracted_dir) {
            anyhow::ensure!(!metadata.file_type().is_symlink(), "Module extraction directory cannot be a symbolic link");
        }
        
        if extracted_dir.exists() {
            fs::remove_dir_all(&extracted_dir)?;
        }
        fs::create_dir_all(&extracted_dir)?;

        // temp_dir에서 extracted_dir로 복사
        copy_dir_all(&extract_path, &extracted_dir)?;

        Ok(LoadedModule {
            metadata,
            path: extracted_dir.to_string_lossy().to_string(),
            is_zip: true,
        })
    }

    /// 모듈 이름으로 모듈 조회
    #[allow(dead_code)]
    pub fn get_module(&self, name: &str) -> Result<LoadedModule> {
        let modules = self.discover_modules()?;
        modules
            .iter()
            .find(|m| m.metadata.name == name)
            .cloned()
            .ok_or_else(|| anyhow::anyhow!("Module '{}' not found", name))
    }
}

/// 디렉토리 전체 복사 헬퍼 함수
fn copy_dir_all(src: &Path, dst: &Path) -> std::io::Result<()> {
    fs::create_dir_all(dst)?;
    for entry in fs::read_dir(src)? {
        let entry = entry?;
        let ty = entry.file_type()?;
        let src_path = entry.path();
        let dst_path = dst.join(entry.file_name());

        if ty.is_dir() {
            copy_dir_all(&src_path, &dst_path)?;
        } else {
            fs::copy(&src_path, &dst_path)?;
        }
    }
    Ok(())
}

// ───────────────────────────────────────────────────────────
// syntax_highlight 정규식 호환성 린트
// ───────────────────────────────────────────────────────────
//
// 모듈 저자는 Python `re` 기준으로 패턴을 작성하는 경우가 많지만, 실제 적용은
// 브라우저 JS RegExp 에서 이루어진다. 런타임 실패를 사전에 개발자에게 알리기
// 위해 명백히 JS 에서 실패하는 문법을 로드 시점에 stderr 로 경고한다.
//
// ⚠️ 경고만 출력하고 규칙을 제거하지는 않는다 — GUI 의 compileRules 가 최종
// 권한을 가지며, 진단도 그쪽에서 수집된다. 여기는 모듈 저자용 조기 경보다.
fn lint_highlight_pattern(rule_name: &str, pattern: &str) {
    // 1. Python 전용 문법 — JS 미지원
    let js_incompatible: &[(&str, &str)] = &[
        ("(?P=", "Python backreference (?P=name); use \\k<name> for JS compat"),
        ("(?(",  "Python conditional (?(id)yes|no); unsupported in JS"),
        ("(?#",  "Python comment (?#...); unsupported in JS"),
    ];
    for (needle, msg) in js_incompatible {
        if pattern.contains(needle) {
            eprintln!(
                "[module_loader] syntax_highlight rule '{}': JS-incompatible pattern — {}",
                rule_name, msg
            );
        }
    }

    // 2. 인라인 플래그 (?i), (?s), (?m), (?x) — JS 는 (?i:...) / (?ims:...) 같은
    //    범위 플래그만 ES2018+ 에서 제한 지원. 단독 (?flags) 는 미지원.
    //    `(?<` (named group), `(?:` (non-capturing), `(?=`/`(?!` (lookahead),
    //    `(?<=`/`(?<!` (lookbehind) 는 정상이므로 제외.
    if let Some(idx) = pattern.find("(?") {
        let rest = &pattern[idx + 2..];
        let first = rest.chars().next();
        if let Some(c) = first {
            if matches!(c, 'i' | 's' | 'm' | 'x' | 'a' | 'L' | 'u') {
                // (?i:...) 같은 범위형은 허용, (?i) 단독 또는 (?i)... 은 JS에서 에러
                let second = rest.chars().nth(1);
                if second != Some(':') && !matches!(second, Some('a'..='z') | Some('A'..='Z')) {
                    eprintln!(
                        "[module_loader] syntax_highlight rule '{}': bare inline flag (?{}...) \
                         is unsupported in JS; use RegExp flags or (?{}:...)",
                        rule_name, c, c
                    );
                }
            }
        }
    }

    // 3. 길이 상한 — GUI 의 MAX_PATTERN_LENGTH 와 맞춤
    const MAX_PATTERN_LENGTH: usize = 4000;
    if pattern.len() > MAX_PATTERN_LENGTH {
        eprintln!(
            "[module_loader] syntax_highlight rule '{}': pattern length {} exceeds {} and will be rejected by GUI",
            rule_name, pattern.len(), MAX_PATTERN_LENGTH
        );
    }

    // 4. 중첩 수량자 — ReDoS 위험. GUI 가 거부하므로 경고.
    let bytes = pattern.as_bytes();
    let mut i = 0;
    while i + 2 < bytes.len() {
        if bytes[i] == b')' && (bytes[i + 1] == b'+' || bytes[i + 1] == b'*') {
            // 그룹 내부에 수량자가 있는지 역방향 검사
            let mut depth = 1;
            let mut j = i;
            while j > 0 && depth > 0 {
                j -= 1;
                match bytes[j] {
                    b')' => depth += 1,
                    b'(' => depth -= 1,
                    b'+' | b'*' if depth == 1 => {
                        eprintln!(
                            "[module_loader] syntax_highlight rule '{}': nested quantifier detected \
                             (catastrophic backtracking risk); GUI will reject this pattern",
                            rule_name
                        );
                        break;
                    }
                    _ => {}
                }
            }
        }
        i += 1;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn recovery_zip_module_name_cannot_escape_extraction_root() {
        use std::io::Write;
        let tmp = tempfile::tempdir().unwrap();
        let modules_dir = tmp.path().join("modules");
        let outside = tmp.path().join("outside");
        fs::create_dir_all(modules_dir.join(".extracted")).unwrap();
        fs::create_dir_all(&outside).unwrap();
        fs::write(outside.join("keep.txt"), "keep").unwrap();
        let zip_path = modules_dir.join("unsafe.zip");
        let mut archive = zip::ZipWriter::new(fs::File::create(&zip_path).unwrap());
        archive.start_file("module.toml", zip::write::FileOptions::default()).unwrap();
        archive.write_all(br#"[module]
name = "../../outside"
version = "1.0.0"
description = "Test"
entry = "lifecycle.py"
"#).unwrap();
        archive.finish().unwrap();
        let loader = ModuleLoader::new(modules_dir.to_str().unwrap());

        assert!(loader.load_module_from_zip(&zip_path).is_err());
        assert!(outside.join("keep.txt").exists());
    }

    #[test]
    fn test_module_loader_creation() {
        let loader = ModuleLoader::new("./modules");
        assert_eq!(loader.modules_dir, "./modules");
    }

    #[test]
    fn test_discover_modules_empty_dir() {
        let loader = ModuleLoader::new("./nonexistent_modules");
        let modules = loader.discover_modules().unwrap();
        assert!(modules.is_empty());
    }

    #[test]
    fn test_parse_module_toml_minimal() {
        let toml = r#"
[module]
name = "test-game"
version = "1.0.0"
entry = "lifecycle.py"
"#;
        let meta = parse_module_toml(toml).unwrap();
        assert_eq!(meta.name, "test-game");
        assert_eq!(meta.version, "1.0.0");
        assert_eq!(meta.entry, "lifecycle.py");
        assert!(meta.description.is_none());
        assert!(meta.settings.is_none());
        assert!(meta.commands.is_none());
    }

    #[test]
    fn test_parse_module_toml_missing_name() {
        let toml = r#"
[module]
version = "1.0.0"
entry = "lifecycle.py"
"#;
        let result = parse_module_toml(toml);
        assert!(result.is_err());
        let err_msg = result.unwrap_err().to_string();
        assert!(err_msg.contains("name"), "Error should mention missing field 'name': {}", err_msg);
    }

    #[test]
    fn test_parse_module_toml_with_settings_and_commands() {
        let toml = r#"
[module]
name = "minecraft"
version = "2.0.0"
entry = "lifecycle.py"
log_pattern = '/(?P<level>INFO|WARN|ERROR)\]'

[protocols]
supported = ["rcon", "stdin"]
default = "rcon"
interaction_mode = "console"

[config]
process_name = "java.exe"
default_port = 25565
stop_command = "stop"

[[settings.fields]]
name = "ram"
type = "text"
label = "Memory"
default = "2G"
group = "saba-chan"

[[commands.fields]]
name = "players"
label = "Player List"
method = "rcon"
rcon_template = "list"
"#;
        let meta = parse_module_toml(toml).unwrap();
        assert_eq!(meta.name, "minecraft");
        assert_eq!(meta.log_pattern.as_deref(), Some("/(?P<level>INFO|WARN|ERROR)\\]"));
        assert_eq!(meta.interaction_mode.as_deref(), Some("console"));
        assert_eq!(meta.default_port, Some(25565));
        assert_eq!(meta.stop_command.as_deref(), Some("stop"));
        
        let settings = meta.settings.unwrap();
        assert_eq!(settings.fields.len(), 1);
        assert_eq!(settings.fields[0].name, "ram");
        
        let commands = meta.commands.unwrap();
        assert_eq!(commands.fields.len(), 1);
        assert_eq!(commands.fields[0].name, "players");
        assert_eq!(commands.fields[0].rcon_template.as_deref(), Some("list"));
    }

    #[test]
    fn test_parse_module_toml_extra_config_fields_ignored() {
        let toml = r#"
[module]
name = "palworld"
version = "1.0.0"
entry = "lifecycle.py"

[config]
executable_path = "PalServer.exe"
server_executable = "PalServer.exe"
process_name = "PalServer"
default_port = 8211
ram = "4G"
custom_unknown_field = "should not cause error"
"#;
        let meta = parse_module_toml(toml).unwrap();
        assert_eq!(meta.name, "palworld");
        assert_eq!(meta.executable_path.as_deref(), Some("PalServer.exe"));
        assert_eq!(meta.server_executable.as_deref(), Some("PalServer.exe"));
        assert_eq!(meta.default_port, Some(8211));
    }

    #[test]
    fn test_parse_module_toml_missing_version() {
        let toml = r#"
[module]
name = "test-game"
entry = "lifecycle.py"
"#;
        let result = parse_module_toml(toml);
        assert!(result.is_err());
        let err_msg = result.unwrap_err().to_string();
        assert!(err_msg.contains("version"), "Error should mention missing 'version': {}", err_msg);
    }

    #[test]
    fn test_parse_module_toml_missing_entry() {
        let toml = r#"
[module]
name = "test-game"
version = "1.0.0"
"#;
        let result = parse_module_toml(toml);
        assert!(result.is_err());
        let err_msg = result.unwrap_err().to_string();
        assert!(err_msg.contains("entry"), "Error should mention missing 'entry': {}", err_msg);
    }

    #[test]
    fn test_parse_module_toml_empty_string() {
        let result = parse_module_toml("");
        assert!(result.is_err(), "Empty TOML should fail");
    }

    #[test]
    fn test_parse_module_toml_aliases() {
        let toml = r#"
[module]
name = "palworld"
version = "1.0.0"
entry = "lifecycle.py"

[aliases]
module_aliases = ["팰월드", "팔월드", "pw"]

[aliases.commands]
start = { aliases = ["시작", "실행", "켜기"], description = "Start the server" }
stop = { aliases = ["정지", "끄기"], description = "Stop the server" }
"#;
        let meta = parse_module_toml(toml).unwrap();
        assert_eq!(meta.module_aliases.len(), 3);
        assert!(meta.module_aliases.contains(&"팰월드".to_string()));
        assert!(meta.module_aliases.contains(&"pw".to_string()));

        assert_eq!(meta.command_aliases.len(), 2);
        let start = meta.command_aliases.get("start").unwrap();
        assert_eq!(start.aliases.len(), 3);
        assert!(start.aliases.contains(&"시작".to_string()));
        assert_eq!(start.description.as_deref(), Some("Start the server"));

        let stop = meta.command_aliases.get("stop").unwrap();
        assert_eq!(stop.aliases.len(), 2);
    }

    #[test]
    fn test_parse_module_toml_aliases_empty_by_default() {
        let toml = r#"
[module]
name = "test-game"
version = "1.0.0"
entry = "lifecycle.py"
"#;
        let meta = parse_module_toml(toml).unwrap();
        assert!(meta.module_aliases.is_empty());
        assert!(meta.command_aliases.is_empty());
    }

    #[test]
    fn test_parse_module_toml_invalid_toml_syntax() {
        let result = parse_module_toml("this is not [valid toml =!");
        assert!(result.is_err(), "Invalid TOML syntax should fail");
    }

    #[test]
    fn test_parse_module_toml_no_module_section() {
        let toml = r#"
[config]
process_name = "java"
"#;
        let result = parse_module_toml(toml);
        assert!(result.is_err(), "TOML without [module] section should fail");
    }

    #[test]
    fn test_parse_module_toml_protocols_section() {
        let toml = r#"
[module]
name = "test-game"
version = "1.0.0"
entry = "lifecycle.py"

[protocols]
supported = ["rcon", "rest"]
default = "rest"
interaction_mode = "commands"
"#;
        let meta = parse_module_toml(toml).unwrap();
        assert_eq!(meta.interaction_mode.as_deref(), Some("commands"));
    }

    #[test]
    fn test_parse_module_toml_multiple_settings_groups() {
        let toml = r#"
[module]
name = "test-game"
version = "1.0.0"
entry = "lifecycle.py"

[[settings.fields]]
name = "ram"
type = "text"
label = "Memory"
default = "2G"
group = "performance"

[[settings.fields]]
name = "port"
type = "number"
label = "Port"
default = "25565"
group = "network"
"#;
        let meta = parse_module_toml(toml).unwrap();
        let settings = meta.settings.unwrap();
        assert_eq!(settings.fields.len(), 2);
        assert_eq!(settings.fields[0].group.as_deref(), Some("performance"));
        assert_eq!(settings.fields[1].group.as_deref(), Some("network"));
    }

    #[test]
    fn test_parse_module_toml_cmd_patterns() {
        let toml = r#"
[module]
name = "zomboid"
version = "1.0.0"
entry = "lifecycle.py"

[config]
process_name = "java.exe"
default_port = 16261

[detection]
process_patterns = ["ProjectZomboid64", "zombie.network.GameServer"]
cmd_patterns = ["zombie.network.GameServer", "ProjectZomboid", "pzserver"]
common_paths = ["C:\\PZServer"]
"#;
        let meta = parse_module_toml(toml).unwrap();
        assert_eq!(meta.process_name.as_deref(), Some("java.exe"));
        assert_eq!(meta.process_patterns.len(), 2);
        assert_eq!(meta.cmd_patterns.len(), 3);
        assert!(meta.cmd_patterns.contains(&"zombie.network.GameServer".to_string()));
        assert!(meta.cmd_patterns.contains(&"ProjectZomboid".to_string()));
    }

    #[test]
    fn test_parse_module_toml_cmd_patterns_empty_by_default() {
        let toml = r#"
[module]
name = "test-game"
version = "1.0.0"
entry = "lifecycle.py"
"#;
        let meta = parse_module_toml(toml).unwrap();
        assert!(meta.cmd_patterns.is_empty());
    }

    // ── 컴포넌트 의존성(dependencies) 테스트 ──

    #[test]
    fn test_parse_module_toml_dependencies_field() {
        let toml = r#"
[module]
name = "test-game"
version = "1.0.0"
entry = "lifecycle.py"

[module.dependencies]
"saba-core" = ">=0.3.0"
"ext-steamcmd" = ">=1.0.0"
"#;
        let meta = parse_module_toml(toml).unwrap();
        assert_eq!(meta.dependencies.len(), 2);
        assert_eq!(meta.dependencies.get("saba-core").unwrap(), ">=0.3.0");
        assert_eq!(meta.dependencies.get("ext-steamcmd").unwrap(), ">=1.0.0");
    }

    #[test]
    fn test_parse_module_toml_dependencies_empty_by_default() {
        let toml = r#"
[module]
name = "test-game"
version = "1.0.0"
entry = "lifecycle.py"
"#;
        let meta = parse_module_toml(toml).unwrap();
        assert!(meta.dependencies.is_empty());
    }

    #[test]
    fn test_module_check_dependencies_all_satisfied() {
        let toml = r#"
[module]
name = "test-game"
version = "1.0.0"
entry = "lifecycle.py"

[module.dependencies]
"saba-core" = ">=0.3.0"
"ext-steamcmd" = ">=1.0.0"
"#;
        let meta = parse_module_toml(toml).unwrap();
        let mut installed = std::collections::HashMap::new();
        installed.insert("saba-core".to_string(), "0.5.0".to_string());
        installed.insert("ext-steamcmd".to_string(), "1.2.0".to_string());

        let issues = meta.check_dependencies(&installed);
        assert!(issues.is_empty(), "All dependencies should be satisfied");
    }

    #[test]
    fn test_module_check_dependencies_version_too_low() {
        let toml = r#"
[module]
name = "test-game"
version = "1.0.0"
entry = "lifecycle.py"

[module.dependencies]
"saba-core" = ">=0.3.0"
"#;
        let meta = parse_module_toml(toml).unwrap();
        let mut installed = std::collections::HashMap::new();
        installed.insert("saba-core".to_string(), "0.2.0".to_string());

        let issues = meta.check_dependencies(&installed);
        assert_eq!(issues.len(), 1);
        assert_eq!(issues[0].0, "saba-core");
        assert_eq!(issues[0].1, ">=0.3.0");
        assert_eq!(issues[0].2.as_deref(), Some("0.2.0"));
    }

    #[test]
    fn test_module_check_dependencies_component_missing() {
        let toml = r#"
[module]
name = "test-game"
version = "1.0.0"
entry = "lifecycle.py"

[module.dependencies]
"ext-docker" = ">=1.0.0"
"#;
        let meta = parse_module_toml(toml).unwrap();
        let installed = std::collections::HashMap::new(); // empty

        let issues = meta.check_dependencies(&installed);
        assert_eq!(issues.len(), 1);
        assert_eq!(issues[0].0, "ext-docker");
        assert!(issues[0].2.is_none(), "Component not installed → None");
    }

    #[test]
    fn test_module_check_dependencies_exact_version_boundary() {
        let toml = r#"
[module]
name = "test-game"
version = "1.0.0"
entry = "lifecycle.py"

[module.dependencies]
"saba-core" = ">=0.3.0"
"#;
        let meta = parse_module_toml(toml).unwrap();
        let mut installed = std::collections::HashMap::new();
        installed.insert("saba-core".to_string(), "0.3.0".to_string()); // exactly matches

        let issues = meta.check_dependencies(&installed);
        assert!(issues.is_empty(), "Exact version match should satisfy >=");
    }

    #[test]
    fn test_module_check_dependencies_cross_type() {
        let toml = r#"
[module]
name = "palworld"
version = "1.0.0"
entry = "lifecycle.py"

[module.dependencies]
"saba-core" = ">=0.3.0"
"gui" = ">=0.2.0"
"ext-steamcmd" = ">=1.0.0"
"ext-music" = ">=0.1.0"
"#;
        let meta = parse_module_toml(toml).unwrap();
        let mut installed = std::collections::HashMap::new();
        installed.insert("saba-core".to_string(), "0.5.0".to_string());
        installed.insert("gui".to_string(), "0.3.0".to_string());
        installed.insert("ext-steamcmd".to_string(), "1.0.0".to_string());
        installed.insert("ext-music".to_string(), "0.1.0".to_string());

        let issues = meta.check_dependencies(&installed);
        assert!(issues.is_empty(), "All cross-type dependencies should be satisfied");

        // Now test with one missing
        let mut partial = std::collections::HashMap::new();
        partial.insert("saba-core".to_string(), "0.5.0".to_string());
        partial.insert("gui".to_string(), "0.1.0".to_string()); // too low
        partial.insert("ext-steamcmd".to_string(), "1.0.0".to_string());
        // ext-music missing entirely

        let issues = meta.check_dependencies(&partial);
        assert_eq!(issues.len(), 2, "gui too low + ext-music missing");
    }

    // ── syntax_highlight 스키마 테스트 ──

    #[test]
    fn test_parse_syntax_highlight_basic_rule() {
        let toml = r#"
[module]
name = "test"
version = "1.0.0"
entry = "lifecycle.py"

[[syntax_highlight.rules]]
name = "error"
pattern = "ERROR"
color = "error"
bold = true
"#;
        let meta = parse_module_toml(toml).unwrap();
        let sh = meta.syntax_highlight.expect("syntax_highlight missing");
        assert_eq!(sh.rules.len(), 1);
        let r = &sh.rules[0];
        assert_eq!(r.name, "error");
        assert_eq!(r.pattern, "ERROR");
        assert_eq!(r.color, "error");
        assert!(r.bold);
        assert!(!r.italic);
        assert!(!r.underline);
        assert_eq!(r.priority, 0);
        assert!(r.background.is_none());
    }

    #[test]
    fn test_parse_syntax_highlight_extended_fields() {
        let toml = r##"
[module]
name = "test"
version = "1.0.0"
entry = "lifecycle.py"

[[syntax_highlight.rules]]
name = "critical"
pattern = "CRITICAL"
color = "#f38ba8"
background = "#1a1a2e"
bold = true
italic = true
underline = true
priority = 100
"##;
        let meta = parse_module_toml(toml).unwrap();
        let sh = meta.syntax_highlight.unwrap();
        let r = &sh.rules[0];
        assert_eq!(r.background.as_deref(), Some("#1a1a2e"));
        assert!(r.bold && r.italic && r.underline);
        assert_eq!(r.priority, 100);
    }

    #[test]
    fn test_syntax_highlight_serializes_to_json_with_new_fields() {
        let toml = r##"
[module]
name = "test"
version = "1.0.0"
entry = "lifecycle.py"

[[syntax_highlight.rules]]
name = "x"
pattern = "x"
color = "#fff"
underline = true
priority = 5
background = "dim"
"##;
        let meta = parse_module_toml(toml).unwrap();
        let json = serde_json::to_value(meta.syntax_highlight.unwrap()).unwrap();
        let rules = json.get("rules").unwrap().as_array().unwrap();
        let rule = &rules[0];
        assert_eq!(rule.get("underline").and_then(|v| v.as_bool()), Some(true));
        assert_eq!(rule.get("priority").and_then(|v| v.as_i64()), Some(5));
        assert_eq!(rule.get("background").and_then(|v| v.as_str()), Some("dim"));
    }

    #[test]
    fn test_lint_highlight_pattern_does_not_panic() {
        // 린트 함수는 stderr 경고만 출력하고 패닉하지 않아야 한다.
        lint_highlight_pattern("ok", "^\\[\\d{2}:\\d{2}:\\d{2}\\]");
        lint_highlight_pattern("redos", "(a+)+b");
        lint_highlight_pattern("py_backref", "(?P<hl>\\w+)(?P=hl)");
        lint_highlight_pattern("inline_flag", "(?i)hello");
        lint_highlight_pattern("long", &"x".repeat(5000));
    }
}

