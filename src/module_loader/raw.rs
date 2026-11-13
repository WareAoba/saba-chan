//! `module.toml` 원본 구조체와 읽기(`parse_module_toml`), 값 검사

use super::*;
use anyhow::Result;
use serde::{Deserialize, Serialize};

/// module.toml 최상위 구조
#[derive(Debug, Deserialize, schemars::JsonSchema)]
#[schemars(deny_unknown_fields, title = "saba-chan module.toml (schema v1, v2)")]
pub(crate) struct ModuleToml {
    module: ModuleSection,
    #[serde(default)]
    protocols: Option<ProtocolsSection>,
    #[serde(default)]
    config: Option<ConfigSection>,
    #[serde(default)]
    detection: Option<DetectionSection>,
    #[serde(default)]
    settings: Option<SettingsSection>,
    #[serde(default)]
    commands: Option<CommandsSection>,
    #[serde(default)]
    syntax_highlight: Option<SyntaxHighlightSection>,
    #[serde(default)]
    install: Option<InstallSectionToml>,
    /// [launch] 실행 관련 선언 (C8b)
    #[serde(default)]
    launch: Option<LaunchSection>,
    /// [aliases] 모듈 및 명령어 별명 (Discord/CLI 다국어 지원)
    #[serde(default)]
    aliases: Option<AliasesSection>,
    /// [backup] 월드·세이브 백업 (A2)
    #[serde(default)]
    backup: Option<ModuleBackup>,
    /// [permissions] 모듈 코드가 쓰는 능력 (M12)
    #[serde(default)]
    permissions: ModulePermissions,
    /// [errors] 오류 코드 → 사용자 메시지 (채팅 명령 응답용)
    #[serde(default)]
    errors: std::collections::HashMap<String, String>,
    /// [extensions.<익스텐션 id>] 익스텐션이 읽는 모듈별 설정. 데몬은 내용을 보지 않고
    /// `ModuleMetadata.extensions`에 그대로 넘긴다 (익스텐션이 늘어나도 스키마는 그대로)
    #[serde(default)]
    #[schemars(with = "std::collections::HashMap<String, serde_json::Map<String, serde_json::Value>>")]
    extensions: std::collections::HashMap<String, toml::Value>,
    /// [[presets]] 추천 설정 묶음 (I1-b)
    #[serde(default)]
    presets: Vec<ModulePreset>,
    /// [[queries]] 읽기 전용 조회 (X-2)
    #[serde(default)]
    queries: Vec<QueryToml>,
    /// [[agreements]] 켜기 전에 동의할 약관
    #[serde(default)]
    agreements: Vec<ModuleAgreement>,
    /// [[lists]] 플레이어 목록 (화이트리스트·관리자·밴)
    #[serde(default)]
    lists: Vec<ModuleList>,
    /// [status] 켜진 서버의 상태를 더 알려 주는 함수
    #[serde(default)]
    status: Option<StatusSection>,
    /// [[diagnose]] 로그 진단 규칙 (`ModuleMetadata::diagnose`)
    #[serde(default)]
    diagnose: Vec<DiagnoseRule>,
    /// [remote_access] 사바쨩이 정하는 원격 접속 값
    #[serde(default)]
    #[allow(dead_code)]
    remote_access: Option<RemoteAccessSection>,
    /// [runtime] 실행 방식. 실행기가 호출 때 module.toml에서 읽는다 (`plugin::Runtime`)
    #[serde(default)]
    #[allow(dead_code)]
    runtime: crate::plugin::Runtime,
    /// [patterns] 이름 붙인 정규식 조각. 데몬이 읽는 정규식 키에서 `{이름}`으로 쓰면 로드할 때 `(?:조각)`으로 바뀐다
    /// (이름은 글자·숫자·`_`, 숫자로 시작하지 않음. 조각끼리도 참조할 수 있지만 자기 자신은 안 된다)
    #[serde(default)]
    patterns: std::collections::BTreeMap<String, String>,
}

#[derive(Debug, Deserialize, schemars::JsonSchema)]
#[schemars(deny_unknown_fields)]
struct StatusSection {
    /// 접속자 수를 돌려주는 lifecycle.py 함수 이름. 데몬이 켜진 서버마다 1분에 한 번 부른다.
    /// `validate`와 같은 config를 받고 `{"success": true, "online": <정수>, "max": <정수>?}`를 돌려준다
    #[serde(default)]
    players: Option<String>,
}

#[derive(Debug, Deserialize, schemars::JsonSchema)]
#[schemars(deny_unknown_fields)]
struct QueryToml {
    /// 소문자·숫자·`-`·`_` (모듈 안에서 겹치지 않음). 액션 id는 `server.query:<name>`
    name: String,
    #[serde(default)]
    label: Option<String>,
    #[serde(default)]
    description: Option<String>,
    /// lifecycle.py `FUNCTIONS`의 함수 이름. 데몬이 따로 쓰는 함수(stop·command·configure 등)는 안 된다
    function: String,
    /// 입력 (명령 입력과 같은 모양). type은 text·number·select만
    #[serde(default)]
    inputs: Vec<CommandInputToml>,
    /// 부른 사람만 보면 되는 결과 (진단·관리용). 길드 설정 "조회 결과 나만 보기"가 켜지면 디스코드 답을 부른 사람에게만 보인다
    #[serde(default)]
    private: bool,
}

#[derive(Debug, Deserialize, schemars::JsonSchema)]
#[schemars(deny_unknown_fields)]
struct ModuleSection {
    /// 이 파일이 따르는 스키마 버전. 지금은 1
    #[serde(default)]
    #[schemars(schema_with = "schema_version_schema")]
    schema_version: Option<u32>,
    /// 모듈 id (폴더 이름과 같게)
    name: String,
    version: String,
    #[serde(default)]
    description: Option<String>,
    /// 모듈 폴더 기준 Python 진입 파일 (예: "lifecycle.py")
    entry: String,
    #[serde(default)]
    game_name: Option<String>,
    #[serde(default)]
    display_name: Option<String>,
    #[serde(default)]
    author: Option<String>,
    #[serde(default)]
    icon: Option<String>,
    #[serde(default)]
    log_pattern: Option<String>,
    /// 컴포넌트 의존성 — 이 모듈이 요구하는 다른 컴포넌트의 최소 버전
    /// 예: { "saba-core" = ">=0.3.0", "ext-steamcmd" = ">=1.0.0" }
    #[serde(default)]
    dependencies: std::collections::HashMap<String, String>,
}

#[derive(Debug, Default, Deserialize, schemars::JsonSchema)]
#[schemars(deny_unknown_fields)]
struct LaunchSection {
    /// 서버가 콘솔 대신 파일에 쓰는 로그 (작업 디렉터리 기준, 마지막 칸만 `*` `?`)
    #[serde(default)]
    log_files: Vec<String>,
    /// "pipe"(기본) | "pty". 출력을 파이프로 주면 끝날 때까지 모아 두는 서버(stdout 블록 버퍼링)는
    /// "pty"로 의사 콘솔(Windows ConPTY)에 띄워 줄마다 받는다. stdout과 stderr가 한 갈래로 합쳐진다.
    #[serde(default)]
    console: ConsoleMode,
    /// 줄 머리 정규식. 서버가 줄바꿈 없이 이어 쓴 출력(예: 팰월드 REST 접근 로그)을 맞는 자리마다 새 줄로 나눈다.
    /// 없으면 나누지 않는다. 빈 글자에 맞는 정규식은 받지 않는다
    #[serde(default)]
    line_start: Option<String>,
    /// 로그 진단이 콘솔 줄이 없을 때 끝부분을 읽는 로그 파일 (서버 폴더 기준 상대 경로, 앞에서부터 줄이 있는 첫 파일).
    /// 데몬은 모양만 검사하고, 공용 도우미 `daemon_rcon.diagnose_log`가 읽는다 (saba-core >=0.1.14-beta.2)
    #[serde(default)]
    diagnose_files: Vec<String>,
}

/// `[remote_access]`: 사바쨩이 정하는 원격 접속(RCON·REST) 값. 키는 게임 설정 파일의 키다 (saba-core >=0.1.14-beta.2).
/// 데몬은 읽지 않는다: 공용 도우미 `daemon_rcon.remote_access_values`가 설정 저장·첫 설치·실행 전에 같은 값을 쓴다
#[derive(Debug, Deserialize, schemars::JsonSchema)]
#[schemars(deny_unknown_fields)]
#[allow(dead_code)]
struct RemoteAccessSection {
    /// 늘 이 값으로 둔다 (예: `{ RESTAPIEnabled = "True", RCONEnabled = "False" }`)
    #[serde(default)]
    forced: std::collections::BTreeMap<String, String>,
    #[serde(default)]
    rcon: Option<RemoteAccessRcon>,
}

/// `[remote_access.rcon]`: 관리형 시작을 끈 서버만 데몬이 만든 포트·비밀번호로 RCON을 켠다
#[derive(Debug, Deserialize, schemars::JsonSchema)]
#[schemars(deny_unknown_fields)]
#[allow(dead_code)]
struct RemoteAccessRcon {
    /// RCON을 켤 때 함께 쓰는 값
    #[serde(default)]
    on: std::collections::BTreeMap<String, String>,
    /// RCON을 끌 때 쓰는 값 (관리형 시작, 또는 데몬이 접속 정보를 주지 않았을 때)
    #[serde(default)]
    off: std::collections::BTreeMap<String, String>,
    /// 데몬이 준 RCON 포트를 쓰는 키
    port: String,
    /// 데몬이 준 RCON 비밀번호를 쓰는 키
    password: String,
}

/// saba-runner가 서버 콘솔을 잇는 방식 (`[launch] console`)
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize, schemars::JsonSchema)]
#[serde(rename_all = "lowercase")]
pub enum ConsoleMode {
    #[default]
    Pipe,
    Pty,
}

/// 일반 정지 때 서버에 보내는 신호 (`[config] stop_signal`). 없으면 콘솔에 `stop_command`를 보낸다
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, schemars::JsonSchema)]
#[serde(rename_all = "lowercase")]
pub enum StopSignal {
    /// Ctrl+C (unix는 SIGINT). 저장 처리기가 Ctrl+C에만 붙은 서버용
    Interrupt,
}

impl ConsoleMode {
    /// saba-runner `--console` 값
    pub fn as_arg(self) -> &'static str {
        match self {
            ConsoleMode::Pipe => "pipe",
            ConsoleMode::Pty => "pty",
        }
    }
}

#[derive(Debug, Deserialize, schemars::JsonSchema)]
#[schemars(deny_unknown_fields)]
struct ProtocolsSection {
    /// "rcon" | "rest" | "stdin"
    #[serde(default)]
    supported: Option<Vec<String>>,
    #[serde(default)]
    default: Option<String>,
    /// "console" | "commands"
    #[serde(default)]
    interaction_mode: Option<String>,
    /// 게임의 기본 RCON 포트. `supported`에 rcon만 있을 때 데몬이 이 포트로 접속한다
    #[serde(default)]
    rcon_port: Option<u16>,
    /// `[protocols.rest]` REST 접속값이 든 설정 (saba-core >=0.1.14-beta.2)
    #[serde(default)]
    rest: Option<RestSettings>,
}

/// 0.1.14-beta.1까지의 모듈은 선언 없이 이 이름의 설정을 런타임 실행 파일 경로로 썼다 (설정 이름 → 런타임 종류).
/// 그 모듈이 새 데몬에서도 돌도록 `runtime`이 없으면 이 표로 읽는다. 모듈이 모두 `runtime`을 선언하면 지운다
const LEGACY_RUNTIME_SETTINGS: &[(&str, &str)] = &[("java_path", "java")];

/// 설정 필드의 런타임 종류: 선언한 `runtime`, 없으면 선언 전 모듈이 쓰던 이름(`LEGACY_RUNTIME_SETTINGS`)
pub fn setting_runtime<'a>(name: &str, declared: Option<&'a str>) -> Option<&'a str> {
    declared.or_else(|| LEGACY_RUNTIME_SETTINGS.iter().find(|(legacy, _)| *legacy == name).map(|(_, kind)| *kind))
}

/// `[protocols.rest]`: 데몬이 REST로 붙을 때 쓰는 값이 어느 설정에 있는가 (saba-core >=0.1.14-beta.2).
/// 값은 인스턴스 `module_settings`의 그 설정에만 있고, 모듈은 다른 설정처럼 그 설정 이름으로 받는다
/// (`command` 호출 config에는 데몬이 붙을 주소를 `rest_host`·`rest_port`로도 싣는다)
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, schemars::JsonSchema)]
#[schemars(deny_unknown_fields)]
pub struct RestSettings {
    /// 호스트가 든 설정 이름. 기본 `rest_host` (값도 기본값도 없으면 127.0.0.1)
    #[serde(default = "default_rest_host_setting")]
    pub host_setting: String,
    /// 포트가 든 설정 이름. 기본 `rest_port`
    #[serde(default = "default_rest_port_setting")]
    pub port_setting: String,
    /// 비밀번호가 든 설정 이름. 있으면 "비밀번호 자동 생성"이 빈 값을 채우고 모듈이 준 `rest_credentials`를 여기에 적는다
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub password_setting: Option<String>,
    /// 게임이 정한 사용자 이름 (기본 인증). 없으면 모듈 `get_credentials`가 준 이름만 쓴다
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub username: Option<String>,
}

fn default_rest_host_setting() -> String { "rest_host".to_string() }
fn default_rest_port_setting() -> String { "rest_port".to_string() }

impl Default for RestSettings {
    fn default() -> Self {
        Self { host_setting: default_rest_host_setting(), port_setting: default_rest_port_setting(), password_setting: None, username: None }
    }
}

#[derive(Debug, Deserialize, schemars::JsonSchema)]
#[schemars(deny_unknown_fields)]
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
    /// 일반 정지에 콘솔 명령 대신 보낼 신호. `interrupt`면 Ctrl+C(unix는 SIGINT)를 보내고 서버가 끝나기를 기다린다
    #[serde(default)]
    stop_signal: Option<StopSignal>,
    /// 게임 안 공지에 쓰는 `[commands.fields]` 이름 (`message` 입력 하나)
    #[serde(default)]
    announce_command: Option<String>,
    /// 플레이어를 내보내는 `[commands.fields]` 이름 (첫 입력 닉네임, 둘째 입력이 있으면 이유)
    #[serde(default)]
    kick_command: Option<String>,
}

#[derive(Debug, Deserialize, schemars::JsonSchema)]
#[schemars(deny_unknown_fields)]
struct DetectionSection {
    #[serde(default)]
    process_patterns: Option<Vec<String>>,
    #[serde(default)]
    cmd_patterns: Option<Vec<String>>,
    /// 디렉토리 내 파일/폴더 시그니처 — 전부 있어야 이 게임의 서버 폴더로 본다
    #[serde(default)]
    dir_signatures: Option<Vec<String>>,
    /// 정상 종료 때 콘솔에 찍히는 문구 (정규식)
    #[serde(default)]
    graceful_exit_patterns: Vec<String>,
    /// 크래시 때 생기는 파일 (작업 디렉터리 기준, 마지막 칸만 `*` `?`)
    #[serde(default)]
    crash_artifacts: Vec<String>,
}

#[derive(Debug, Deserialize, schemars::JsonSchema)]
#[schemars(deny_unknown_fields)]
struct SettingsSection {
    #[serde(default)]
    fields: Vec<SettingFieldToml>,
}

#[derive(Debug, Deserialize, schemars::JsonSchema)]
#[schemars(deny_unknown_fields)]
struct SettingFieldToml {
    name: String,
    /// "text" | "number" | "password" | "boolean" | "select" | "file" 등
    #[serde(rename = "type", default = "default_field_type")]
    field_type: String,
    #[serde(default)]
    label: Option<String>,
    #[serde(default)]
    description: Option<String>,
    #[serde(default)]
    required: Option<bool>,
    #[serde(default)]
    #[schemars(with = "Option<serde_json::Value>")]
    default: Option<toml::Value>,
    #[serde(default)]
    min: Option<f64>,
    #[serde(default)]
    max: Option<f64>,
    #[serde(default)]
    step: Option<f64>,
    #[serde(default)]
    options: Option<Vec<String>>,
    /// "basic"(기본) | "advanced" | "saba-chan" 등
    #[serde(default)]
    group: Option<String>,
    /// 네트워크 포트 값이면 true. 인스턴스를 만들 때 다른 인스턴스와 겹치지 않게 옮겨진다
    #[serde(default)]
    port: bool,
    /// 밖에서 붙는 포트의 프로토콜 `tcp`·`udp`·`both` (`SettingField::public`). `port = true`인 필드만
    #[serde(default)]
    public: Option<String>,
    /// 서버를 만들 때 고르는 설정이면 true (설치할 파일이 달라지는 값)
    #[serde(default)]
    install: bool,
    /// 만들 때 고를 수 있는 `options` (사바쨩이 설치할 수 있는 값). 없으면 `options` 전부
    #[serde(default)]
    install_options: Option<Vec<String>>,
    /// 실행 인자 모양. 문자열 하나나 배열 (`SettingField::arg`)
    #[serde(default)]
    arg: Option<OneOrMany>,
    #[serde(default)]
    format: Option<String>,
    #[serde(default)]
    pattern: Option<String>,
    /// 스키마 2: `list` 항목 수 범위
    #[serde(default)]
    min_items: Option<u32>,
    #[serde(default)]
    max_items: Option<u32>,
    /// 스키마 2: `multiline` 글자 수 상한
    #[serde(default)]
    max_length: Option<u32>,
    /// 스키마 2: 켜져 있을 때 바꾸면 다시 켜야 적용된다
    #[serde(default)]
    restart_required: Option<bool>,
    /// 스키마 2: 다른 설정 값에 따라 보일지
    #[serde(default)]
    #[schemars(with = "Option<VisibleWhen>")]
    visible_when: Option<VisibleWhenToml>,
    /// 게임 설정 파일의 키 (`SettingField::file_key`)
    #[serde(default)]
    file_key: Option<String>,
    /// 이 설정이 있는 게임 버전 범위: 비교(`<` `<=` `>` `>=` `=`)와 점으로 나눈 숫자 버전을 공백이나 쉼표로 잇는다
    /// (예: `<1.21.9`, `>=1.13 <1.20`). 설치된 버전이 밖이면 화면이 숨긴다 (`SettingField::versions`)
    #[serde(default)]
    versions: Option<String>,
    /// 이 설정 값이 서버를 띄우는 런타임 실행 파일의 경로면 그 종류 (`SettingField::runtime`, saba-core >=0.1.14-beta.2)
    #[serde(default)]
    runtime: Option<String>,
}

#[derive(Debug, Deserialize)]
struct VisibleWhenToml {
    field: String,
    #[serde(default)]
    equals: Option<toml::Value>,
    #[serde(default, rename = "in")]
    any_of: Option<Vec<toml::Value>>,
}

impl SettingFieldToml {
    /// 스키마 2에서 더한 키 중 이 필드에 있는 것
    fn v2_keys(&self) -> Vec<&'static str> {
        [
            ("min_items", self.min_items.is_some()),
            ("max_items", self.max_items.is_some()),
            ("max_length", self.max_length.is_some()),
            ("restart_required", self.restart_required.is_some()),
            ("visible_when", self.visible_when.is_some()),
        ]
        .into_iter()
        .filter_map(|(k, set)| set.then_some(k))
        .collect()
    }
}

fn default_field_type() -> String { "text".to_string() }

/// module.toml [install] 섹션
#[derive(Debug, Deserialize, schemars::JsonSchema)]
#[schemars(deny_unknown_fields)]
struct InstallSectionToml {
    /// "manual"(기본) | "download" | "steamcmd"
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
    /// 설치할 베타 브랜치 (지정하면 항상 이 브랜치로 설치)
    #[serde(default)]
    beta: Option<String>,
    /// 이 설치 방식이 요구하는 익스텐션 ID 목록
    /// 예: requires_extensions = ["steamcmd"]
    #[serde(default)]
    requires_extensions: Vec<String>,
    /// 받을 파일의 대략적인 크기(바이트). 설치 방식 익스텐션이 진행률을 어림하는 데 쓴다 (saba-core >=0.1.14-beta.2)
    #[serde(default)]
    download_size: Option<u64>,
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
    /// 받을 파일의 대략적인 크기(바이트). 설치 방식 익스텐션이 진행률을 어림하는 데 쓴다
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub download_size: Option<u64>,
}

fn default_true_mod() -> bool { true }

/// module.toml [aliases] 섹션
#[derive(Debug, Deserialize, schemars::JsonSchema)]
#[schemars(deny_unknown_fields)]
struct AliasesSection {
    #[serde(default)]
    module_aliases: Vec<String>,
    #[serde(default)]
    commands: std::collections::HashMap<String, CommandAliasSectionToml>,
}

/// module.toml [aliases.commands.*] 항목
#[derive(Debug, Deserialize, schemars::JsonSchema)]
#[schemars(deny_unknown_fields)]
struct CommandAliasSectionToml {
    #[serde(default)]
    aliases: Vec<String>,
    #[serde(default)]
    description: Option<String>,
}

#[derive(Debug, Deserialize, schemars::JsonSchema)]
#[schemars(deny_unknown_fields)]
struct CommandsSection {
    #[serde(default)]
    fields: Vec<CommandFieldToml>,
    /// lifecycle `instance_commands()`로 인스턴스별 명령을 더 받는다
    #[serde(default)]
    hook: bool,
    /// 콘솔 답이 맞으면 실패로 보는 정규식 (`reason` 그룹이 이유)
    #[serde(default)]
    failure_patterns: Vec<String>,
}

#[derive(Debug, Deserialize, schemars::JsonSchema)]
#[schemars(deny_unknown_fields)]
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
    /// 콘솔에 보낼 줄. 예전 이름 `rcon_template`도 받는다
    #[serde(default, alias = "rcon_template")]
    console_template: Option<String>,
    /// "safe" | "caution" | "danger" (없으면 caution)
    #[serde(default)]
    #[schemars(schema_with = "risk_schema")]
    risk: Option<String>,
    #[serde(default)]
    inputs: Vec<CommandInputToml>,
    /// false면 디스코드·챗봇에 내보내지 않는다 (기본 true)
    #[serde(default)]
    remote: Option<bool>,
    /// 콘솔 답에서 값을 뽑는 정규식 (이름 있는 그룹 → locales `commands.<이름>.done`의 자리)
    #[serde(default)]
    reply_pattern: Option<String>,
}

#[derive(Debug, Deserialize, schemars::JsonSchema)]
#[schemars(deny_unknown_fields)]
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
    #[schemars(with = "Option<serde_json::Value>")]
    default: Option<toml::Value>,
    #[serde(default)]
    min: Option<f64>,
    #[serde(default)]
    max: Option<f64>,
    #[serde(default)]
    options: Option<Vec<String>>,
}

impl From<CommandInputToml> for CommandInput {
    fn from(i: CommandInputToml) -> Self {
        CommandInput {
            name: i.name,
            label: i.label,
            input_type: i.input_type,
            required: i.required,
            placeholder: i.placeholder,
            default: i.default,
            min: i.min,
            max: i.max,
            options: i.options,
        }
    }
}

#[derive(Debug, Deserialize, schemars::JsonSchema)]
#[schemars(deny_unknown_fields)]
struct SyntaxHighlightSection {
    #[serde(default)]
    rules: Vec<HighlightRuleToml>,
}

#[derive(Debug, Deserialize, schemars::JsonSchema)]
#[schemars(deny_unknown_fields)]
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

/// 패턴은 GUI(브라우저 JS RegExp)에서 돈다. Python `re`에만 있는 문법은 로드할 때 경고한다.
/// 규칙은 지우지 않는다: 최종 판단은 GUI 컴파일러가 한다
fn lint_highlight_pattern(module: &str, rule: &str, pattern: &str) {
    const PY_ONLY: [(&str, &str); 3] = [("(?P=", "backreference (?P=name); use \\k<name>"), ("(?(", "conditional group"), ("(?#", "inline comment")];
    for (needle, what) in PY_ONLY {
        if pattern.contains(needle) {
            tracing::warn!("Module '{}' highlight rule '{}': Python-only {} does not work in the GUI", module, rule, what);
        }
    }
    // JS는 (?i) 같은 패턴 안 단독 플래그를 모른다 ((?i:...)는 된다)
    for (idx, _) in pattern.match_indices("(?") {
        let mut rest = pattern[idx + 2..].chars();
        if let (Some(flag), next) = (rest.next(), rest.next()) {
            if "imsxauL".contains(flag) && next == Some(')') {
                tracing::warn!("Module '{}' highlight rule '{}': inline flag (?{}) does not work in the GUI", module, rule, flag);
            }
        }
    }
}

impl ModuleToml {
    /// ModuleToml → ModuleMetadata 변환
    fn into_metadata(self) -> ModuleMetadata {
        let settings = if let Some(s) = self.settings {
            let fields: Vec<SettingField> = s.fields.into_iter().map(|f| SettingField {
                runtime: setting_runtime(&f.name, f.runtime.as_deref()).map(str::to_string),
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
                port: f.port,
                public: f.public,
                install: f.install,
                install_options: f.install_options,
                arg: f.arg.map(OneOrMany::into_vec).unwrap_or_default(),
                format: f.format,
                pattern: f.pattern,
                min_items: f.min_items,
                max_items: f.max_items,
                max_length: f.max_length,
                restart_required: f.restart_required.unwrap_or(false),
                visible_when: f.visible_when.map(|v| VisibleWhen {
                    field: v.field,
                    equals: v.equals.and_then(|e| serde_json::to_value(e).ok()),
                    any_of: v.any_of.map(|list| list.into_iter().filter_map(|e| serde_json::to_value(e).ok()).collect()),
                }),
                file_key: f.file_key,
                versions: f.versions,
            }).collect();
            if fields.is_empty() { None } else { Some(ModuleSettings { fields }) }
        } else {
            None
        };

        let commands = if let Some(c) = self.commands {
            let fields: Vec<CommandField> = c.fields.into_iter().map(|f| CommandField {
                name: f.name.clone(),
                label: f.label.unwrap_or(f.name),
                description: f.description,
                method: f.method.map(|m| if m == "rcon" { "console".to_string() } else { m }),
                http_method: f.http_method,
                endpoint_template: f.endpoint_template,
                console_template: f.console_template,
                risk: f.risk,
                inputs: f.inputs.into_iter().map(CommandInput::from).collect(),
                remote: f.remote.unwrap_or(true),
                reply_pattern: f.reply_pattern,
            }).collect();
            if fields.is_empty() && !c.hook { None } else { Some(ModuleCommands { fields, hook: c.hook, failure_patterns: c.failure_patterns }) }
        } else {
            None
        };

        let syntax_highlight = if let Some(sh) = self.syntax_highlight {
            let module_name = self.module.name.clone();
            let rules: Vec<HighlightRule> = sh.rules.into_iter().map(|r| {
                lint_highlight_pattern(&module_name, &r.name, &r.pattern);
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
        } else {
            None
        };

        ModuleMetadata {
            schema_version: self.module.schema_version.unwrap_or(0),
            name: self.module.name,
            version: self.module.version,
            description: self.module.description,
            display_name: self.module.display_name,
            game_name: self.module.game_name,
            author: self.module.author,
            entry: self.module.entry,
            icon: self.module.icon,
            log_pattern: self.module.log_pattern,
            console: self.launch.as_ref().map(|l| l.console).unwrap_or_default(),
            line_start: self.launch.as_ref().and_then(|l| l.line_start.clone()),
            log_files: self.launch.map(|l| l.log_files).unwrap_or_default(),
            dependencies: self.module.dependencies,
            rest: self.protocols.as_ref().and_then(|p| p.rest.clone()).unwrap_or_default(),
            process_name: self.config.as_ref().and_then(|c| c.process_name.clone()),
            default_port: self.config.as_ref().and_then(|c| c.default_port),
            executable_path: self.config.as_ref().and_then(|c| c.executable_path.clone()),
            server_executable: self.config.as_ref().and_then(|c| c.server_executable.clone()),
            stop_command: self.config.as_ref().and_then(|c| c.stop_command.clone()),
            stop_timeout: self.config.as_ref().and_then(|c| c.stop_timeout),
            stop_signal: self.config.as_ref().and_then(|c| c.stop_signal),
            announce_command: self.config.as_ref().and_then(|c| c.announce_command.clone()),
            kick_command: self.config.as_ref().and_then(|c| c.kick_command.clone()),
            backup: self.backup,
            permissions: self.permissions,
            interaction_mode: self.protocols.as_ref().and_then(|p| p.interaction_mode.clone()),
            protocols_supported: self.protocols.as_ref().and_then(|p| p.supported.clone()),
            protocols_default: self.protocols.as_ref().and_then(|p| p.default.clone()),
            rcon_port: self.protocols.as_ref().and_then(|p| p.rcon_port),
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
                download_size: i.download_size,
            }),
            extensions: {
                let ext_map: std::collections::HashMap<String, serde_json::Value> = self.extensions
                    .into_iter()
                    .filter_map(|(id, v)| Some((id, serde_json::to_value(v).ok()?)))
                    .collect();
                ext_map
            },
            process_patterns: self.detection.as_ref()
                .and_then(|d| d.process_patterns.clone())
                .unwrap_or_default(),
            cmd_patterns: self.detection.as_ref()
                .and_then(|d| d.cmd_patterns.clone())
                .unwrap_or_default(),
            dir_signatures: self.detection.as_ref()
                .and_then(|d| d.dir_signatures.clone())
                .unwrap_or_default(),
            graceful_exit_patterns: self.detection.as_ref()
                .map(|d| d.graceful_exit_patterns.clone())
                .unwrap_or_default(),
            crash_artifacts: self.detection
                .map(|d| d.crash_artifacts)
                .unwrap_or_default(),
            ready_patterns: Vec::new(),
            progress_patterns: Vec::new(),
            platforms: None,
            architectures: None,
            ready_timeout: None,
            player_join_patterns: Vec::new(),
            chat_patterns: Vec::new(),
            chat_send_function: None,
            player_leave_patterns: Vec::new(),
            data_roots: Vec::new(),
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
            errors: self.errors,
            presets: self.presets,
            queries: self.queries.into_iter().map(|q| ModuleQuery {
                name: q.name,
                label: q.label,
                description: q.description,
                function: q.function,
                inputs: q.inputs.into_iter().map(CommandInput::from).collect(),
                private: q.private,
            }).collect(),
            agreements: self.agreements,
            lists: self.lists,
            players_function: self.status.and_then(|s| s.players),
            diagnose: self.diagnose,
            warnings: Vec::new(),
        }
    }
}

/// TOML 문자열에서 ModuleMetadata를 파싱합니다.
///
/// - `schema_version`이 이 데몬보다 새로우면 거부한다 (모듈을 먼저 업데이트한 경우).
/// - `schema_version = 1`이면 잘못된 값을 오류로 낸다. 모르는 키는 `warnings`에 남기고 무시한다
///   (더 새 데몬의 키를 쓴 모듈도 읽게. `module validate`가 경고로 보여 준다).
/// - `schema_version`이 없으면 스키마 이전 모듈로 보고 잘못된 값도 `warnings`에만 남긴다.
pub(crate) fn parse_module_toml(content: &str) -> Result<ModuleMetadata> {
    let content = content.trim_start_matches('\u{feff}');
    let mut unknown = Vec::new();
    let de = toml::Deserializer::parse(content).map_err(|e| anyhow::anyhow!("Failed to parse module.toml: {}", e))?;
    let mut toml_data: ModuleToml = serde_ignored::deserialize(de, |path| {
        // serde_ignored는 Option 한 겹마다 `?`를 끼운다: "config.?.ram" → "config.ram"
        let key: Vec<String> = path.to_string().split('.').filter(|p| *p != "?").map(str::to_string).collect();
        unknown.push(key.join("."))
    })
    .map_err(|e| {
        // 스키마를 밝힌 모듈의 키 타입 오류도 `module_schema_invalid`로 알린다 (`failed[].problems`)
        let declared = content.parse::<toml::Table>().ok().and_then(|v| v.get("module")?.get("schema_version")?.as_integer());
        match declared.and_then(|v| u32::try_from(v).ok()).filter(|v| (1..=MODULE_SCHEMA_VERSION).contains(v)) {
            Some(version) => SchemaInvalid { version, problems: vec![e.to_string()] }.into(),
            None => anyhow::anyhow!("Failed to parse module.toml: {}", e),
        }
    })?;

    // 모르는 키(더 새 데몬의 키나 오타)는 거부하지 않고 경고만 남긴다. 꼭 필요한 기능은 `saba-core` 버전 조건으로 막는다
    let unknown: Vec<String> = unknown.into_iter().map(|k| format!("unknown key '{}' (ignored)", k)).collect();
    // 정규식 키의 `{이름}`은 값을 검사하기 전에 `[patterns]` 조각으로 바꾼다
    let (mut fragments, mut problems) = resolve_pattern_fragments(&toml_data.patterns);
    problems.extend(toml_data.expand_pattern_refs(&mut fragments));
    problems.extend(toml_data.value_problems());

    let mut meta = toml_data.into_metadata();
    let daemon_warnings = read_daemon_table(&mut meta, &mut fragments);
    if meta.schema_version >= 2 {
        problems.extend(list_default_problems(&meta));
    }
    problems.extend(preset_problems(&meta));
    let bad_queries = query_problems(&meta);
    // 스키마 이전 모듈도 잘못된 조회는 켜지 않는다: 조회는 확인 없이 모든 표면에 열린다
    if meta.schema_version == 0 && !bad_queries.is_empty() {
        meta.queries.clear();
    }
    problems.extend(bad_queries);
    problems.extend(agreement_problems(&meta));
    let bad_lists = player_list_problems(&meta);
    // 스키마 이전 모듈도 잘못된 목록은 켜지 않는다 (없는 명령을 보내는 화면이 생기지 않게)
    if meta.schema_version == 0 && !bad_lists.is_empty() {
        meta.lists.clear();
    }
    problems.extend(bad_lists);
    if let Some(problem) = meta.players_function.as_deref().and_then(|f| function_problem(f, "status.players")) {
        // 잘못 선언한 함수는 부르지 않는다
        meta.players_function = None;
        problems.push(problem);
    }
    // 스키마 이전 모듈도 쓸 수 없는 줄 머리는 러너에 넘기지 않는다 (위 problems에 실렸다)
    if meta.line_start.as_deref().is_some_and(|p| line_start_problem(p).is_some()) {
        meta.line_start = None;
    }
    if meta.schema_version < 2 {
        // 스키마 2 키는 스키마 1·이전 모듈에서 켜지 않는다 (위 problems에 이미 실렸다)
        for f in meta.settings.iter_mut().flat_map(|s| s.fields.iter_mut()) {
            (f.min_items, f.max_items, f.max_length, f.restart_required, f.visible_when) = (None, None, None, false, None);
            // 스키마 이전 모듈의 새 타입은 0.1.8처럼 글자로 다룬다 (스키마 1은 위에서 실패한다)
            if SETTING_TYPES_V2.contains(&f.field_type.as_str()) {
                f.field_type = "text".into();
            }
        }
    }
    match meta.schema_version {
        0 => {
            for p in &problems {
                tracing::warn!("module '{}': {} (ignored)", meta.name, p);
            }
            meta.warnings = problems;
            meta.warnings.push(format!(
                "no schema_version; add `schema_version = {}` under [module]",
                BASE_SCHEMA_VERSION
            ));
        }
        version @ 1..=MODULE_SCHEMA_VERSION => {
            if !problems.is_empty() {
                return Err(SchemaInvalid { version, problems }.into());
            }
        }
        version => return Err(SchemaUnsupported { version }.into()),
    }
    // 데몬이 호출마다 정해 넣는 이름: 모듈 설정 값은 넘기지 않는다 (06 §2·§6, `RESERVED_CALL_KEYS`)
    for f in meta.settings.iter().flat_map(|s| &s.fields).filter(|f| RESERVED_CALL_KEYS.contains(&f.name.as_str())) {
        meta.warnings.push(format!("settings field '{}' is a name saba-chan sets on each call; its value is not passed to the module", f.name));
    }
    for w in unknown.iter().chain(&daemon_warnings) {
        tracing::warn!("module '{}': {}", meta.name, w);
    }
    meta.warnings.extend(unknown);
    meta.warnings.extend(daemon_warnings);
    Ok(meta)
}

/// 데몬이 읽는 `[extensions.<이름>]` 표. 0.1.8 로더가 내용을 검사하지 않는 자리라 새 키를 여기 둔다.
/// 그래서 이 이름의 익스텐션은 만들 수 없다 (`extension::is_valid_extension_id`)
pub const DAEMON_TABLE: &str = CORE_COMPONENT;
/// 이 데몬이 읽는 `[extensions.saba-core]` 키
const DAEMON_TABLE_KEYS: &[&str] =
    &["ready_patterns", "ready_timeout", "player_join_patterns", "player_leave_patterns", "chat_patterns", "chat_send", "data_roots", "progress_patterns", "platforms", "architectures"];
/// 이 데몬의 OS·CPU에서 돌아가지 않는 모듈의 서버를 만들거나 켜지 않는 응답. 돌아가면 None
pub fn host_unsupported_body(module: &str, meta: &ModuleMetadata) -> Option<serde_json::Value> {
    match meta.unsupported_on(std::env::consts::OS, std::env::consts::ARCH)? {
        "os" => Some(platform_unsupported_body(module, meta.platforms.as_deref().unwrap_or_default())),
        _ => Some(arch_unsupported_body(module, meta.architectures.as_deref().unwrap_or_default())),
    }
}

/// 이 CPU를 지원하지 않는 모듈의 응답 (`module_arch_unsupported`)
fn arch_unsupported_body(module: &str, architectures: &[String]) -> serde_json::Value {
    serde_json::json!({
        "success": false,
        "error": format!("Module '{}' runs on {} only, not {}", module, architectures.join(", "), std::env::consts::ARCH),
        "error_code": "module_arch_unsupported",
        "module": module,
        "architectures": architectures,
        "arch": std::env::consts::ARCH,
    })
}

/// 이 OS를 지원하지 않는 모듈의 서버를 만들거나 켜지 않는 응답 (`module_platform_unsupported`)
fn platform_unsupported_body(module: &str, platforms: &[String]) -> serde_json::Value {
    serde_json::json!({
        "success": false,
        "error": format!("Module '{}' runs on {} only, not {}", module, platforms.join(", "), std::env::consts::OS),
        "error_code": "module_platform_unsupported",
        "module": module,
        "platforms": platforms,
        "os": std::env::consts::OS,
    })
}



pub use saba_chan_updater_lib::module_support::{self, catalog_unsupported_on};
/// `ready_timeout` 상한(초)
pub const READY_TIMEOUT_MAX_SECS: u64 = 900;

/// `[patterns]` 조각 이름인지: 글자나 `_`로 시작하고 글자·숫자·`_`만. 반복 횟수(`{2}`·`{1,16}`)는 이름이 아니다
fn is_pattern_name(name: &str) -> bool {
    let mut chars = name.chars();
    chars.next().is_some_and(|c| c.is_ascii_alphabetic() || c == '_') && chars.all(|c| c.is_ascii_alphanumeric() || c == '_')
}

/// 정규식 안의 `{이름}` 참조 자리와 이름. `is_declared`가 아닌 이름과 `\{`로 막은 자리는 참조가 아니다
fn pattern_refs(pattern: &str, is_declared: impl Fn(&str) -> bool) -> Vec<(std::ops::Range<usize>, &str)> {
    let mut refs = Vec::new();
    let mut from = 0;
    while let Some(open) = pattern[from..].find('{').map(|i| from + i) {
        from = open + 1;
        let Some(len) = pattern[from..].find('}') else { break };
        let name = &pattern[from..from + len];
        if !pattern[..open].ends_with('\\') && is_declared(name) {
            refs.push((open..from + len + 1, name));
            from += len + 1;
        }
    }
    refs
}

/// 한 모듈에서 `[patterns]` 조각을 풀어 넣을 수 있는 글자 수 (조각끼리 풀 때와 정규식 키에서 풀 때를 모두 센다)
const PATTERN_EXPANSION_LIMIT: usize = 1 << 20;

/// 풀어 둔 `[patterns]` 조각과, 이 모듈에서 더 풀어 넣을 수 있는 글자 수
struct PatternFragments {
    resolved: std::collections::HashMap<String, String>,
    budget: usize,
}

impl PatternFragments {
    /// 정규식 안의 `{이름}`을 풀어 둔 조각 `(?:조각)`으로 바꾼다. 조각에 없는 이름은 그대로 둔다.
    /// 풀어 넣을 글자가 남은 한도를 넘으면 바꾸지 않고 그 까닭을 돌려준다
    fn expand(&mut self, pattern: &str) -> Result<String, String> {
        let refs = pattern_refs(pattern, |n| self.resolved.contains_key(n));
        let inserted: usize = refs.iter().map(|(_, name)| self.resolved[*name].len() + 4).sum();
        if inserted > self.budget {
            return Err(format!("[patterns] expand to more than {} bytes in this module", PATTERN_EXPANSION_LIMIT));
        }
        self.budget -= inserted;
        let mut out = String::with_capacity(pattern.len() + inserted);
        let mut last = 0;
        for (range, name) in refs {
            out.push_str(&pattern[last..range.start]);
            out.push_str("(?:");
            out.push_str(&self.resolved[name]);
            out.push(')');
            last = range.end;
        }
        out.push_str(&pattern[last..]);
        Ok(out)
    }
}

/// `[patterns]` 조각을 서로의 참조까지 풀어 둔다. 이름이 틀렸거나, 자기 자신을 (다른 조각을 거쳐서라도) 참조하거나,
/// 풀어도 정규식이 아니거나 한도를 넘는 조각과 그런 조각을 쓰는 조각은 빼고 그 까닭을 돌려준다: 빠진 조각의 `{이름}`은 글자 그대로 남는다
fn resolve_pattern_fragments(raw: &std::collections::BTreeMap<String, String>) -> (PatternFragments, Vec<String>) {
    use std::collections::HashMap;
    /// `name`을 풀어 `fragments`에 넣는다. 못 쓰는 조각은 까닭과 함께 `failed`에 넣는다
    fn resolve(
        name: &str,
        raw: &std::collections::BTreeMap<String, String>,
        fragments: &mut PatternFragments,
        failed: &mut HashMap<String, String>,
        stack: &mut Vec<String>,
    ) -> Result<(), String> {
        if fragments.resolved.contains_key(name) {
            return Ok(());
        }
        if let Some(problem) = failed.get(name) {
            return Err(problem.clone());
        }
        if let Some(at) = stack.iter().position(|n| n == name) {
            let chain: Vec<&str> = stack[at..].iter().map(String::as_str).chain([name]).collect();
            return Err(format!("patterns.{} refers to itself ({})", name, chain.join(" -> ")));
        }
        stack.push(name.to_string());
        let text = &raw[name];
        let deps = pattern_refs(text, |n| raw.contains_key(n) && is_pattern_name(n));
        let result = deps.iter().try_for_each(|(_, dep)| resolve(dep, raw, fragments, failed, stack)).and_then(|()| {
            let expanded = fragments.expand(text).map_err(|e| format!("patterns.{}: {}", name, e))?;
            match regex::Regex::new(&format!("(?:{})", expanded)) {
                Ok(_) => Ok(expanded),
                Err(e) => Err(format!("patterns.{} is not a valid regex ({})", name, e)),
            }
        });
        stack.pop();
        match result {
            Ok(expanded) => {
                fragments.resolved.insert(name.to_string(), expanded);
                Ok(())
            }
            Err(problem) => {
                failed.insert(name.to_string(), problem.clone());
                Err(problem)
            }
        }
    }

    let mut problems: Vec<String> = raw.keys().filter(|n| !is_pattern_name(n))
        .map(|n| format!("patterns.{}: a name is letters, digits and _, not starting with a digit", n))
        .collect();
    let mut fragments = PatternFragments { resolved: HashMap::new(), budget: PATTERN_EXPANSION_LIMIT };
    let mut failed = HashMap::new();
    for name in raw.keys().filter(|n| is_pattern_name(n)) {
        if let Err(problem) = resolve(name, raw, &mut fragments, &mut failed, &mut Vec::new()) {
            if !problems.contains(&problem) {
                problems.push(problem);
            }
        }
    }
    (fragments, problems)
}

impl ModuleToml {
    /// 데몬이 읽는 정규식 키의 `{이름}`을 `[patterns]` 조각으로 바꾼다.
    /// `[extensions.saba-core]`의 정규식 목록은 `read_daemon_table`이 읽으면서 바꾼다. 한도를 넘는 키는 쓴 그대로 두고 그 까닭을 돌려준다
    fn expand_pattern_refs(&mut self, fragments: &mut PatternFragments) -> Vec<String> {
        if fragments.resolved.is_empty() {
            return Vec::new();
        }
        let mut regexes: Vec<&mut String> = Vec::new();
        regexes.extend(self.module.log_pattern.as_mut());
        if let Some(launch) = self.launch.as_mut() {
            regexes.extend(launch.line_start.as_mut());
        }
        if let Some(detection) = self.detection.as_mut() {
            regexes.extend(detection.graceful_exit_patterns.iter_mut());
        }
        if let Some(backup) = self.backup.as_mut() {
            regexes.extend(backup.save_done_pattern.as_mut());
        }
        if let Some(settings) = self.settings.as_mut() {
            regexes.extend(settings.fields.iter_mut().filter_map(|f| f.pattern.as_mut()));
        }
        if let Some(commands) = self.commands.as_mut() {
            regexes.extend(commands.failure_patterns.iter_mut());
            regexes.extend(commands.fields.iter_mut().filter_map(|f| f.reply_pattern.as_mut()));
        }
        if let Some(highlight) = self.syntax_highlight.as_mut() {
            regexes.extend(highlight.rules.iter_mut().map(|r| &mut r.pattern));
        }
        let mut problems = Vec::new();
        for pattern in regexes {
            match fragments.expand(pattern) {
                Ok(expanded) => *pattern = expanded,
                Err(e) => problems.push(format!("'{}': {}", pattern, e)),
            }
        }
        problems
    }
}

/// `[extensions.saba-core]`의 정규식 목록 하나를 읽는다 (`{이름}`은 `[patterns]` 조각으로 바꾼다). `needs_name`이면 `(?P<name>…)`이 없는 항목도 버린다
fn read_pattern_list(
    table: &serde_json::Value,
    key: &str,
    groups: &[&str],
    fragments: &mut PatternFragments,
    warnings: &mut Vec<String>,
) -> Vec<String> {
    let mut out = Vec::new();
    match table.get(key) {
        None => {}
        Some(serde_json::Value::Array(items)) => {
            for item in items {
                let Some(p) = item.as_str() else {
                    warnings.push(format!("extensions.{}.{} must hold strings; ignored {}", DAEMON_TABLE, key, item));
                    continue;
                };
                let pattern = match fragments.expand(p) {
                    Ok(pattern) => pattern,
                    Err(e) => {
                        warnings.push(format!("extensions.{}.{} '{}': {}; ignored", DAEMON_TABLE, key, p, e));
                        continue;
                    }
                };
                match regex::Regex::new(&pattern) {
                    Ok(re) => match groups.iter().find(|g| !re.capture_names().any(|n| n == Some(**g))) {
                        None => out.push(pattern),
                        Some(g) => warnings.push(format!("extensions.{}.{} '{}' has no (?P<{}>...) group; ignored", DAEMON_TABLE, key, p, g)),
                    },
                    Err(_) => warnings.push(format!("extensions.{}.{} '{}' is not a valid regex; ignored", DAEMON_TABLE, key, p)),
                }
            }
        }
        Some(_) => warnings.push(format!("extensions.{}.{} must be a list of strings; ignored", DAEMON_TABLE, key)),
    }
    out
}



/// `[extensions.saba-core]`의 `ready_patterns`·`ready_timeout`(R39), `player_join_patterns`·`player_leave_patterns`(MK8), `chat_patterns`(게임 채팅), `progress_patterns`를 읽는다.
/// 틀린 값은 그 값만 빼고 경고로 남긴다
fn read_daemon_table(meta: &mut ModuleMetadata, fragments: &mut PatternFragments) -> Vec<String> {
    let Some(table) = meta.extensions.get(DAEMON_TABLE).cloned() else { return Vec::new() };
    let mut warnings = Vec::new();
    // 이 표는 버전 사이를 잇는 자리라, 더 새 데몬의 키가 있어도 모듈을 거부하지 않는다
    if let Some(obj) = table.as_object() {
        for key in obj.keys().filter(|k| !DAEMON_TABLE_KEYS.contains(&k.as_str())) {
            warnings.push(format!("extensions.{}.{} is not known to this saba-chan; ignored", DAEMON_TABLE, key));
        }
    }
    meta.ready_patterns.extend(read_pattern_list(&table, "ready_patterns", &[], fragments, &mut warnings));
    meta.player_join_patterns.extend(read_pattern_list(&table, "player_join_patterns", &["name"], fragments, &mut warnings));
    meta.player_leave_patterns.extend(read_pattern_list(&table, "player_leave_patterns", &["name"], fragments, &mut warnings));
    meta.chat_patterns.extend(read_pattern_list(&table, "chat_patterns", &["name", "message"], fragments, &mut warnings));
    meta.progress_patterns.extend(read_pattern_list(&table, "progress_patterns", &[], fragments, &mut warnings));
    match table.get("chat_send") {
        None => {}
        Some(v) => match v.as_str().map(|f| (f, function_problem(f, ""))) {
            Some((f, None)) => meta.chat_send_function = Some(f.to_string()),
            Some((_, Some(problem))) => warnings.push(format!("extensions.{}.chat_send: {}; ignored", DAEMON_TABLE, problem)),
            None => warnings.push(format!("extensions.{}.chat_send must be a function name; ignored", DAEMON_TABLE)),
        },
    }
    meta.platforms = module_support::read_platforms(&table, &mut warnings);
    meta.architectures = module_support::read_architectures(&table, &mut warnings);
    match table.get("data_roots") {
        None => {}
        Some(serde_json::Value::Array(items)) => {
            for item in items {
                let valid = item.as_str().and_then(crate::module_paths::data_root_parts).is_some_and(|(key, _)| {
                    key == "home" || meta.settings.iter().flat_map(|s| &s.fields).any(|f| f.name == key && (f.field_type == "file" || f.format.as_deref() == Some("path")))
                });
                if valid {
                    meta.data_roots.push(item.as_str().unwrap().to_string());
                } else {
                    warnings.push(format!("extensions.{}.data_roots contains an invalid root {}; ignored", DAEMON_TABLE, item));
                }
            }
        }
        Some(_) => warnings.push(format!("extensions.{}.data_roots must be a list of strings; ignored", DAEMON_TABLE)),
    }
    match table.get("ready_timeout") {
        None => {}
        Some(v) => match v.as_u64().filter(|n| *n > 0) {
            // 디스코드는 시작을 시킨 채널을 READY_TIMEOUT_MAX_SECS 동안만 기억한다 (05 §4.4): 더 길면 알림이 사라진다
            Some(n) if n > READY_TIMEOUT_MAX_SECS => {
                meta.ready_timeout = Some(READY_TIMEOUT_MAX_SECS);
                warnings.push(format!("extensions.{}.ready_timeout is at most {} seconds; using {}", DAEMON_TABLE, READY_TIMEOUT_MAX_SECS, READY_TIMEOUT_MAX_SECS));
            }
            Some(n) => meta.ready_timeout = Some(n),
            None => warnings.push(format!("extensions.{}.ready_timeout must be a positive number of seconds; using the default", DAEMON_TABLE)),
        },
    }
    warnings
}

/// list 기본값은 PATCH와 같은 검사(항목 수·중복·options·형식)를 받는다: 새 인스턴스가 그대로 받는 값이다
fn list_default_problems(meta: &ModuleMetadata) -> Vec<String> {
    meta.settings
        .iter()
        .flat_map(|s| &s.fields)
        .filter(|f| f.field_type == "list")
        .filter_map(|f| {
            let default = serde_json::to_value(f.default.as_ref()?).ok()?;
            let e = crate::validator::validate_setting_value(f, Some(&default)).err()?;
            Some(format!("settings field '{}': default {}", f.name, e.message))
        })
        .collect()
}

/// 스키마 2 설정 키의 값 검사 (06 §8.1·8.2)
fn v2_field_problems(field: &SettingFieldToml, fields: &[&SettingFieldToml]) -> Vec<String> {
    let mut problems = Vec::new();
    let name = &field.name;
    let ty = field.field_type.as_str();
    if ty != "list" && (field.min_items.is_some() || field.max_items.is_some()) {
        problems.push(format!("settings field '{}': min_items/max_items are only for type 'list'", name));
    }
    if let (Some(min), Some(max)) = (field.min_items, field.max_items) {
        if min > max {
            problems.push(format!("settings field '{}': min_items is larger than max_items", name));
        }
    }
    if ty != "multiline" && field.max_length.is_some() {
        problems.push(format!("settings field '{}': max_length is only for type 'multiline'", name));
    }
    let Some(cond) = &field.visible_when else { return problems };
    let Some(target) = fields.iter().find(|f| f.name == cond.field) else {
        problems.push(format!("settings field '{}': visible_when.field '{}' is not a setting of this module", name, cond.field));
        return problems;
    };
    if target.name == *name {
        problems.push(format!("settings field '{}': visible_when cannot point at itself", name));
        return problems;
    }
    if NOT_A_CONDITION.contains(&target.field_type.as_str()) {
        problems.push(format!("settings field '{}': visible_when.field '{}' is a {} setting", name, target.name, target.field_type));
    }
    let values: Vec<&toml::Value> = cond.equals.iter().chain(cond.any_of.iter().flatten()).collect();
    if cond.equals.is_some() == cond.any_of.is_some() || values.is_empty() {
        problems.push(format!("settings field '{}': visible_when needs exactly one of equals or in", name));
    }
    let fits = |v: &toml::Value| match target.field_type.as_str() {
        "boolean" => v.is_bool(),
        "number" => v.is_integer() || v.is_float(),
        "select" => v.as_str().is_some_and(|s| target.options.as_ref().is_none_or(|o| o.iter().any(|x| x == s))),
        _ => v.is_str(),
    };
    if values.iter().any(|v| !fits(v)) {
        problems.push(format!("settings field '{}': visible_when value does not fit the {} setting '{}'", name, target.field_type, target.name));
    }
    // 순환: 조건을 따라가다 자기에게 돌아오면 안 된다
    let mut seen = vec![name.as_str()];
    let mut next = Some(cond.field.as_str());
    while let Some(current) = next {
        if seen.contains(&current) {
            problems.push(format!("settings field '{}': visible_when forms a cycle", name));
            break;
        }
        seen.push(current);
        next = fields.iter().find(|f| f.name == current).and_then(|f| f.visible_when.as_ref()).map(|c| c.field.as_str());
    }
    problems
}

/// 데몬이 모듈 호출 config에 직접 넣는 키. 같은 이름의 모듈 설정 값은 config에 싣지 않는다
pub const RESERVED_CALL_KEYS: &[&str] = &["players_detail", "stop_call_limit"];

impl ModuleToml {
    /// 키는 맞지만 값이 계약을 벗어난 곳
    fn value_problems(&self) -> Vec<String> {
        let mut problems = Vec::new();
        if !crate::utils::is_inner_relative_path(&self.module.entry) {
            problems.push(format!("module.entry '{}' must be a path inside the module folder", self.module.entry));
        }
        let v2 = self.module.schema_version == Some(2);
        let fields: Vec<&SettingFieldToml> = self.settings.iter().flat_map(|s| &s.fields).collect();
        for field in &fields {
            let ty = field.field_type.as_str();
            if SETTING_TYPES_V2.contains(&ty) && !v2 {
                problems.push(format!("settings field '{}': type '{}' needs schema_version = 2", field.name, ty));
            } else if !SETTING_TYPES.contains(&ty) && !SETTING_TYPES_V2.contains(&ty) {
                let known = if v2 { [SETTING_TYPES, SETTING_TYPES_V2].concat() } else { SETTING_TYPES.to_vec() };
                problems.push(format!("settings field '{}': type '{}' must be one of {}", field.name, ty, known.join(", ")));
            }
            if !v2 {
                for key in field.v2_keys() {
                    problems.push(format!("settings field '{}': {} needs schema_version = 2", field.name, key));
                }
            } else {
                problems.extend(v2_field_problems(field, &fields));
            }
            if let Some(runtime) = field.runtime.as_deref().filter(|r| crate::sandbox::runtime_kind(r).is_none()) {
                let known: Vec<&str> = crate::sandbox::RUNTIME_KINDS.iter().map(|k| k.id).collect();
                problems.push(format!("settings field '{}': unknown runtime '{}' (one of {})", field.name, runtime, known.join(", ")));
            }
            if let Some(format) = field.format.as_deref().filter(|f| !SETTING_FORMATS.contains(f)) {
                problems.push(format!("settings field '{}': unknown format '{}' (one of {})", field.name, format, SETTING_FORMATS.join(", ")));
            }
            if let Some(public) = field.public.as_deref() {
                if crate::port_check::protocols_of(public).is_none() {
                    problems.push(format!("settings field '{}': public '{}' must be one of tcp, udp, both", field.name, public));
                } else if !field.port {
                    problems.push(format!("settings field '{}': public needs port = true", field.name));
                }
            }
            if let Some(allowed) = &field.install_options {
                let options = field.options.as_deref().unwrap_or(&[]);
                if !field.install || ty != "select" {
                    problems.push(format!("settings field '{}': install_options needs type = \"select\" and install = true", field.name));
                } else if allowed.is_empty() || allowed.iter().any(|v| !options.contains(v)) {
                    problems.push(format!("settings field '{}': install_options must be a non-empty subset of options", field.name));
                } else if let Some(d) = field.default.as_ref().and_then(|d| d.as_str()).filter(|d| !allowed.iter().any(|v| v == d)) {
                    problems.push(format!("settings field '{}': default '{}' is not in install_options", field.name, d));
                }
            }
            if let Some(Err(e)) = field.pattern.as_deref().map(regex::Regex::new) {
                problems.push(format!("settings field '{}': pattern is not a valid regex: {}", field.name, e));
            }
            if let Some(problem) = field.versions.as_deref().and_then(crate::validator::version_range_problem) {
                problems.push(format!("settings field '{}': versions {}", field.name, problem));
            }
            let args = match &field.arg {
                Some(OneOrMany::One(s)) => std::slice::from_ref(s),
                Some(OneOrMany::Many(v)) => v.as_slice(),
                None => &[],
            };
            for arg in args {
                if let Err(e) = crate::launch_args::ArgPattern::parse(arg) {
                    problems.push(format!("settings field '{}': arg '{}' {}", field.name, arg, e));
                }
            }
        }
        let mut file_keys = std::collections::HashSet::new();
        for field in &fields {
            if let Some(key) = &field.file_key {
                if key.trim().is_empty() || key.contains(['=', '\n', '\r']) {
                    problems.push(format!("settings field '{}': file_key must be a non-empty key without '=' or line breaks", field.name));
                } else if !file_keys.insert(key.as_str()) {
                    problems.push(format!("settings field '{}': file_key '{}' is used by another field", field.name, key));
                }
            }
        }
        let mut codes = std::collections::HashSet::new();
        for rule in &self.diagnose {
            if !codes.insert(rule.code.as_str()) {
                problems.push(format!("diagnose '{}': duplicate code", rule.code));
            }
            if rule.patterns.is_empty() {
                problems.push(format!("diagnose '{}': patterns must not be empty", rule.code));
            }
            if let Some(severity) = rule.severity.as_deref().filter(|s| !DIAGNOSE_SEVERITIES.contains(s)) {
                problems.push(format!("diagnose '{}': severity '{}' must be one of {}", rule.code, severity, DIAGNOSE_SEVERITIES.join(", ")));
            }
        }
        for cmd in self.commands.iter().flat_map(|c| &c.fields) {
            for input in &cmd.inputs {
                if let Some(ty) = input.input_type.as_deref().filter(|t| !COMMAND_INPUT_TYPES.contains(t)) {
                    problems.push(format!(
                        "commands '{}': input '{}' type '{}' must be one of {}",
                        cmd.name,
                        input.name,
                        ty,
                        COMMAND_INPUT_TYPES.join(", ")
                    ));
                }
            }
            if let Some(risk) = &cmd.risk {
                if !COMMAND_RISKS.contains(&risk.as_str()) {
                    problems.push(format!(
                        "commands '{}': risk '{}' must be one of {}",
                        cmd.name,
                        risk,
                        COMMAND_RISKS.join(", ")
                    ));
                }
            }
        }
        problems.extend(self.launch.as_ref().and_then(|l| l.line_start.as_deref()).and_then(line_start_problem));
        for file in self.launch.iter().flat_map(|l| &l.diagnose_files) {
            if !crate::utils::is_inner_relative_path(file) {
                problems.push(format!("launch.diagnose_files '{}' must be a path inside the server folder", file));
            }
        }
        if let Some(p) = &self.protocols {
            for value in p.supported.iter().flatten().chain(&p.default) {
                if !PROTOCOLS.contains(&value.as_str()) {
                    problems.push(format!("protocols: '{}' must be one of {}", value, PROTOCOLS.join(", ")));
                }
            }
            let route = p.supported.as_deref().map_or(RconRoute::Never, rcon_route);
            if route == RconRoute::Only && p.rcon_port.unwrap_or(0) == 0 {
                problems.push("protocols: rcon_port is required when rcon is the only protocol".to_string());
            }
            // stdin과 함께 쓰는 rcon은 관리형 시작을 끈 서버의 명령 길이다: 포트가 있어야 데몬이 접속 정보를 만든다
            if route == RconRoute::WithoutConsole && p.rcon_port.unwrap_or(0) == 0 {
                problems.push("protocols: rcon_port is required when rcon is the command path without a console".to_string());
            }
            if let Some(rest) = &p.rest {
                // 호스트 설정은 없어도 된다 (이 PC로 붙는다). 포트는 `port = true` 설정을, 비밀번호는 password 설정을 가리켜야 한다:
                // 데몬이 포트 충돌·비밀값 가리기를 그 표시로 한다
                let field = |name: &str| fields.iter().find(|f| f.name == name);
                match field(&rest.port_setting) {
                    None => problems.push(format!("protocols.rest: port_setting '{}' is not a [[settings.fields]] name", rest.port_setting)),
                    Some(f) if !f.port => problems.push(format!("protocols.rest: port_setting '{}' must be a `port = true` setting", rest.port_setting)),
                    Some(_) => {}
                }
                if let Some(name) = &rest.password_setting {
                    match field(name) {
                        None => problems.push(format!("protocols.rest: password_setting '{}' is not a [[settings.fields]] name", name)),
                        Some(f) if f.field_type != "password" => problems.push(format!("protocols.rest: password_setting '{}' must be a `type = \"password\"` setting", name)),
                        Some(_) => {}
                    }
                }
            }
        }
        // 공지·저장 명령은 [commands.fields]의 이름을 가리킨다
        let named = |name: &str| self.commands.iter().flat_map(|c| &c.fields).any(|f| f.name == name);
        let refs = [
            ("config.announce_command", self.config.as_ref().and_then(|c| c.announce_command.as_deref())),
            ("config.kick_command", self.config.as_ref().and_then(|c| c.kick_command.as_deref())),
            ("backup.save_command", self.backup.as_ref().and_then(|b| b.save_command.as_deref())),
            ("backup.pause_command", self.backup.as_ref().and_then(|b| b.pause_command.as_deref())),
            ("backup.resume_command", self.backup.as_ref().and_then(|b| b.resume_command.as_deref())),
        ];
        for (key, name) in refs {
            if let Some(name) = name.filter(|n| !named(n)) {
                problems.push(format!("{} '{}' is not a [[commands.fields]] name", key, name));
            }
        }
        if let Some(c) = &self.commands {
            let patterns = c.fields.iter().filter_map(|f| f.reply_pattern.as_ref().map(|p| (format!("commands.fields '{}' reply_pattern", f.name), p)));
            let failures = c.failure_patterns.iter().map(|p| ("commands.failure_patterns".to_string(), p));
            for (key, pattern) in patterns.chain(failures) {
                if let Err(e) = regex::Regex::new(pattern) {
                    problems.push(format!("{}: invalid regex ({})", key, e));
                }
            }
        }
        if let Some(Err(e)) = self.backup.as_ref().and_then(|b| b.save_done_pattern.as_deref()).map(regex::Regex::new) {
            problems.push(format!("backup.save_done_pattern: invalid regex ({})", e));
        }
        if self.backup.as_ref().is_some_and(|b| b.pause_command.is_some() && b.resume_command.is_none()) {
            problems.push("backup.pause_command needs backup.resume_command".to_string());
        }
        problems
    }
}

/// `[launch] line_start`가 쓸 수 없는 정규식이면 그 까닭. 빈 글자에 맞으면 모든 글자 사이에서 줄이 나뉜다
fn line_start_problem(pattern: &str) -> Option<String> {
    match regex::Regex::new(pattern) {
        Err(e) => Some(format!("launch.line_start: invalid regex ({})", e)),
        Ok(re) if re.is_match("") => Some("launch.line_start must not match the empty string".to_string()),
        Ok(_) => None,
    }
}

/// 프리셋이 계약을 벗어난 곳: id 형식·중복, 없는 설정 이름, 비밀번호·파일·경로 설정, 설정 규칙에 안 맞는 값
fn preset_problems(meta: &ModuleMetadata) -> Vec<String> {
    let mut problems = Vec::new();
    let fields = meta.settings.as_ref().map(|s| s.fields.as_slice()).unwrap_or(&[]);
    let mut seen = std::collections::HashSet::new();
    for preset in &meta.presets {
        if !is_slug(&preset.id) {
            problems.push(format!("presets '{}': id must be lowercase letters, digits, '-' or '_'", preset.id));
        }
        if !seen.insert(preset.id.as_str()) {
            problems.push(format!("presets '{}': duplicate id", preset.id));
        }
        for (key, value) in &preset.settings {
            let Some(field) = fields.iter().find(|f| &f.name == key) else {
                problems.push(format!("presets '{}': '{}' is not a [[settings.fields]] name", preset.id, key));
                continue;
            };
            if field.field_type == "password" || field.field_type == "file" || field.format.as_deref() == Some("path") {
                problems.push(format!("presets '{}': '{}' cannot be set by a preset", preset.id, key));
                continue;
            }
            // list는 항목마다 options를 본다 (validate_setting_value)
            let option_ok = (field.field_type == "list" && value.is_array()) || match (&field.options, value.as_str()) {
                (Some(options), Some(v)) => options.iter().any(|o| o == v),
                (Some(_), None) => false,
                (None, _) => !value.is_object() && !value.is_array(),
            };
            if !option_ok || crate::validator::validate_setting_value(field, Some(value)).is_err() {
                problems.push(format!("presets '{}': '{}' value is not allowed for this setting", preset.id, key));
            } else if field.install_options.as_ref().is_some_and(|ok| !value.as_str().is_some_and(|v| ok.iter().any(|o| o == v))) {
                // 프리셋으로 새 서버를 만들면 그 값으로 설치한다: 설치할 수 없는 값이면 그 프리셋은 늘 거절된다
                problems.push(format!("presets '{}': '{}' is not in install_options", preset.id, key));
            }
        }
    }
    problems
}

/// 데몬이 정해진 뜻으로 부르는 lifecycle 함수 중 상태를 바꾸는 것: 조회(`[[queries]]`)·`[status] players`의 함수가 될 수 없다
const NON_QUERY_FUNCTIONS: &[&str] = &[
    "start", "stop", "command", "configure", "get_launch_command", "import_settings", "install_server",
    "reset_properties", "reset_server", "accept_agreements", "functions",
];
/// 조회 입력으로 쓸 수 있는 형식 (없으면 text)
const QUERY_INPUT_TYPES: &[&str] = &["text", "number", "select"];

/// 소문자·숫자로 시작하고 소문자·숫자·`-`·`_`만 (프리셋 id·조회 이름)
fn is_slug(s: &str) -> bool {
    s.chars().next().is_some_and(|c| c.is_ascii_lowercase() || c.is_ascii_digit())
        && s.chars().all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '-' || c == '_')
}

/// 약관 선언이 계약을 벗어난 곳: id 형식·중복, https가 아닌 원문 주소
fn agreement_problems(meta: &ModuleMetadata) -> Vec<String> {
    let mut problems = Vec::new();
    let mut seen = std::collections::HashSet::new();
    for a in &meta.agreements {
        if !is_slug(&a.id) {
            problems.push(format!("agreements '{}': id must be lowercase letters, digits, '-' or '_'", a.id));
        }
        if !seen.insert(a.id.as_str()) {
            problems.push(format!("agreements '{}': duplicate id", a.id));
        }
        if !a.url.starts_with("https://") {
            problems.push(format!("agreements '{}': url must start with https://", a.id));
        }
    }
    problems
}

/// 플레이어 목록 선언이 계약을 벗어난 곳: id 형식·중복, 넣기·빼기 명령이 없거나 이름 입력이 없음, boolean이 아닌 설정
fn player_list_problems(meta: &ModuleMetadata) -> Vec<String> {
    let mut problems = Vec::new();
    let mut seen = std::collections::HashSet::new();
    let commands = meta.commands.as_ref().map(|c| c.fields.as_slice()).unwrap_or_default();
    let settings = meta.settings.as_ref().map(|s| s.fields.as_slice()).unwrap_or_default();
    for list in &meta.lists {
        let mut problem = |what: String| problems.push(format!("lists '{}': {}", list.id, what));
        if !is_slug(&list.id) {
            problem("id must be lowercase letters, digits, '-' or '_'".into());
        }
        if !seen.insert(list.id.as_str()) {
            problem("duplicate id".into());
        }
        for (key, name) in [("add", &list.add), ("remove", &list.remove)] {
            match commands.iter().find(|c| &c.name == name) {
                None => problem(format!("{key} '{name}' is not a [[commands.fields]] name")),
                Some(c) if c.inputs.is_empty() => problem(format!("{key} '{name}' needs an input for the player name")),
                Some(_) => {}
            }
        }
        if let Some(name) = &list.setting {
            if !settings.iter().any(|f| &f.name == name && f.field_type == "boolean") {
                problem(format!("setting '{name}' is not a boolean [[settings.fields]] name"));
            }
        }
    }
    problems
}

/// 데몬이 읽기 용도로 부를 함수가 Python 식별자가 아니거나 상태를 바꾸는 함수면 그 문제 (`place`가 있으면 앞에 붙인다)
fn function_problem(function: &str, place: &str) -> Option<String> {
    let identifier = function.chars().next().is_some_and(|c| c.is_ascii_alphabetic() || c == '_')
        && function.chars().all(|c| c.is_ascii_alphanumeric() || c == '_');
    let what = if !identifier {
        format!("function '{}' is not a Python function name", function)
    } else if NON_QUERY_FUNCTIONS.contains(&function) {
        format!("function '{}' changes the server and cannot be a query", function)
    } else {
        return None;
    };
    Some(if place.is_empty() { what } else { format!("{}: {}", place, what) })
}

/// 조회가 계약을 벗어난 곳: 이름 형식·중복, Python 식별자가 아니거나 상태를 바꾸는 함수, 입력 이름·형식·범위
fn query_problems(meta: &ModuleMetadata) -> Vec<String> {
    let mut problems = Vec::new();
    let mut seen = std::collections::HashSet::new();
    for query in &meta.queries {
        let mut problem = |what: String| problems.push(format!("queries '{}': {}", query.name, what));
        if !is_slug(&query.name) {
            problem("name must be lowercase letters, digits, '-' or '_'".into());
        }
        if !seen.insert(query.name.as_str()) {
            problem("duplicate name".into());
        }
        if let Some(what) = function_problem(&query.function, "") {
            problem(what);
        }
        let mut inputs = std::collections::HashSet::new();
        for input in &query.inputs {
            let valid_name = !input.name.is_empty() && input.name.chars().all(|c| c.is_ascii_alphanumeric() || c == '_' || c == '-');
            if !valid_name {
                problem(format!("input '{}': name must be letters, digits, '-' or '_'", input.name));
            }
            if !inputs.insert(input.name.as_str()) {
                problem(format!("input '{}': duplicate name", input.name));
            }
            let ty = input.input_type.as_deref().unwrap_or("text");
            if !QUERY_INPUT_TYPES.contains(&ty) {
                problem(format!("input '{}': type '{}' must be one of {}", input.name, ty, QUERY_INPUT_TYPES.join(", ")));
            }
            if ty == "select" && input.options.as_ref().is_none_or(|o| o.is_empty()) {
                problem(format!("input '{}': select needs options", input.name));
            }
            if let (Some(min), Some(max)) = (input.min, input.max) {
                if min > max {
                    problem(format!("input '{}': min is greater than max", input.name));
                }
            }
        }
    }
    problems
}
