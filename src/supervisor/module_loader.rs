use std::fs;
use std::path::{Path, PathBuf};
use std::sync::RwLock;
use std::io::Read;
use anyhow::Result;
use serde::{Deserialize, Serialize};
use zip::ZipArchive;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ModuleMetadata {
    /// module.toml `schema_version`. 없으면 0(스키마 이전 모듈: 모르는 키는 경고만)
    #[serde(default)]
    pub schema_version: u32,
    pub name: String,
    pub version: String,
    pub description: Option<String>,
    /// 사람에게 보여줄 이름 (없으면 name)
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub display_name: Option<String>,
    /// 게임 이름 (예: "Palworld")
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub game_name: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub author: Option<String>,
    pub entry: String,  // 모듈 폴더 기준 Python 진입 파일 (보통 lifecycle.py)
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
    /// [config] announce_command — 게임 안 공지에 쓰는 `[commands.fields]` 이름 (`message` 입력 하나). 예약 재시작 예고·공지(A3)용
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub announce_command: Option<String>,
    /// [backup] 월드·세이브 백업 선언 (A2). 없으면 이 모듈은 백업을 지원하지 않는다
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub backup: Option<ModuleBackup>,
    /// [permissions] 모듈 코드가 쓰는 능력 (M12). 코드 검사 결과와 대조한다
    #[serde(default)]
    pub permissions: ModulePermissions,
    #[serde(default)]
    pub log_pattern: Option<String>,  // regex pattern for log level extraction (e.g. "/(?P<level>INFO|WARN|ERROR|DEBUG)/")
    #[serde(default)]
    pub interaction_mode: Option<String>,  // "console" or "commands" (from [protocols])
    #[serde(default)]
    pub protocols_supported: Option<Vec<String>>,  // ["rcon", "stdin", "rest"] etc.
    #[serde(default)]
    pub protocols_default: Option<String>,  // default protocol mode
    /// `[protocols] rcon_port`: 게임의 기본 RCON 포트. `rcon`만 받는 모듈에서만 쓴다 (`rcon_only_port`)
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub rcon_port: Option<u16>,
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
    /// [errors] 오류 코드 → 사용자 메시지 (예: server_not_running, auth_failed)
    /// 채팅 명령 처리기가 데몬 오류를 모듈 고유 문구로 바꿀 때 사용
    #[serde(default, skip_serializing_if = "std::collections::HashMap::is_empty")]
    pub errors: std::collections::HashMap<String, String>,
    /// [launch] log_files — 서버가 콘솔 대신 파일에 쓰는 로그 (작업 디렉터리 기준, 마지막 칸만 `*` `?` 허용).
    /// saba-runner가 따라 읽어 콘솔에 합친다. 예: ["Pal/Saved/Logs/*.log"]
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub log_files: Vec<String>,
    /// [launch] console — 서버 콘솔을 잇는 방식. 기본 pipe
    #[serde(default)]
    pub console: ConsoleMode,
    /// [detection] graceful_exit_patterns — 정상 종료 때 콘솔에 찍히는 문구 (정규식). A1 크래시 판정용
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub graceful_exit_patterns: Vec<String>,
    /// [detection] crash_artifacts — 크래시 때 생기는 파일 (작업 디렉터리 기준, 마지막 칸만 `*` `?`). A1용
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub crash_artifacts: Vec<String>,
    /// [[presets]] 서버를 만들 때 고르는 추천 설정 묶음 (I1-b)
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub presets: Vec<ModulePreset>,
    /// [[queries]] 읽기 전용 조회 (X-2). 액션 `server.query:<name>`이 된다
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub queries: Vec<ModuleQuery>,
    /// [[agreements]] 서버를 켜기 전에 사용자가 동의해야 하는 약관 (예: 게임 EULA)
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub agreements: Vec<ModuleAgreement>,
    /// [status] players — 켜진 서버의 접속자 수를 돌려주는 lifecycle 함수 (Q-5, `supervisor::players`)
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub players_function: Option<String>,
    /// 로드는 됐지만 고쳐야 할 점 (스키마 이전 모듈의 모르는 키 등)
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub warnings: Vec<String>,
}

/// `[[presets]]` — 서버를 만들 때 고르는 추천 설정 묶음 (예: "친구용 소규모", "하드코어").
/// 표시 문구는 모듈 locales의 `presets.<id>.label`·`description`이 덮는다
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize, schemars::JsonSchema)]
#[schemars(deny_unknown_fields)]
pub struct ModulePreset {
    /// 소문자·숫자·`-`·`_` (예: "friends")
    pub id: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub label: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,
    /// `[[settings.fields]]` 이름 → 값. 비밀번호·파일·경로 설정은 넣지 못한다
    #[serde(default)]
    #[schemars(with = "serde_json::Map<String, serde_json::Value>")]
    pub settings: serde_json::Map<String, serde_json::Value>,
}

/// `[[queries]]` — 읽기 전용 조회 (예: 접속자 목록). 액션 `server.query:<name>`이 되어 GUI·CLI·디스코드·챗봇이
/// 부른다. 데몬은 `validate`와 같은 config에 `args`(입력 이름 → 값)를 더해 lifecycle `function`을 부른다.
/// 표시 문구는 모듈 locales의 `queries.<name>.label`·`description`이 덮는다
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ModuleQuery {
    /// 소문자·숫자·`-`·`_` (예: "players")
    pub name: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub label: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,
    /// lifecycle.py `FUNCTIONS`의 함수 이름
    pub function: String,
    /// 명령 입력과 같은 모양·검사 (`crate::commands::bind`)
    #[serde(default)]
    pub inputs: Vec<CommandInput>,
}

/// `[[agreements]]` — 사용자가 원문을 읽고 동의해야 서버를 켤 수 있는 약관.
/// 이름은 모듈 locales의 `agreements.<id>`, 동의는 인스턴스 레코드 `accepted_agreements`에 남는다
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, schemars::JsonSchema)]
#[schemars(deny_unknown_fields)]
pub struct ModuleAgreement {
    /// 소문자·숫자·`-`·`_` (모듈 안에서 겹치지 않음, 예: "eula")
    pub id: String,
    /// 사용자가 읽을 원문 (https)
    pub url: String,
}

/// `[permissions]` — 모듈 Python 코드가 쓰는 능력 (M12). 선언하지 않은 능력을 코드에서 찾으면
/// `saba-core module audit`과 `GET /api/module/:name/audit`이 알린다. 새 권한은 여기와
/// `PERMISSIONS`, `modules/saba_audit.py` 표에 한 줄씩 더한다
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize, schemars::JsonSchema)]
#[schemars(deny_unknown_fields)]
pub struct ModulePermissions {
    /// 네트워크 접속 (socket, urllib, requests 등). 데몬 API 호출(daemon_rcon)은 포함하지 않는다
    #[serde(default)]
    pub network: bool,
    /// 다른 프로그램 실행·프로세스 신호 (subprocess, os.system, os.kill 등)
    #[serde(default)]
    pub spawn_process: bool,
    /// 네이티브 코드 (ctypes, cffi, winreg)
    #[serde(default)]
    pub native_code: bool,
    /// 실행 중에 만든 코드 실행 (eval, exec, compile, 동적 __import__, pickle, importlib)
    #[serde(default)]
    pub dynamic_code: bool,
    /// 환경 변수 전체 읽기 (dict(os.environ) 등)
    #[serde(default)]
    pub env_secrets: bool,
    /// 서버 작업 폴더 밖의 파일 쓰기 (정적 검사로는 찾지 못한다: 선언만)
    #[serde(default)]
    pub fs_outside_instance: bool,
}

impl ModulePermissions {
    /// 선언했는지. 모르는 이름이면 None (검사기가 새 권한을 먼저 알게 된 경우)
    pub fn declares(&self, name: &str) -> Option<bool> {
        Some(match name {
            "network" => self.network,
            "spawn_process" => self.spawn_process,
            "native_code" => self.native_code,
            "dynamic_code" => self.dynamic_code,
            "env_secrets" => self.env_secrets,
            "fs_outside_instance" => self.fs_outside_instance,
            _ => return None,
        })
    }
}

/// `[backup]` — 무엇을 백업할지
#[derive(Debug, Clone, Default, Serialize, Deserialize, schemars::JsonSchema)]
#[schemars(deny_unknown_fields)]
pub struct ModuleBackup {
    /// 백업할 폴더·파일 (서버 작업 폴더 기준, 절대 경로 가능). `{설정 이름}`은 인스턴스 설정 값으로 바뀌고
    /// 값이 비면 그 경로는 빠진다. 없는 경로도 빠진다. 예: ["{level_name}", "{level_name}_nether"]
    #[serde(default)]
    pub paths: Vec<String>,
    /// 서버가 켜져 있을 때 묶기 전에 실행할 `[commands.fields]` 이름 (예: "save-all")
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub save_command: Option<String>,
    /// true면 lifecycle `backup_paths(config)`가 `{"success": true, "paths": [...]}`로 경로를 준다
    /// (세이브 위치를 모듈 코드가 계산하는 게임용). `paths`에 더해진다
    #[serde(default)]
    pub hook: bool,
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
    /// 명령을 RCON으로 보내는 모듈이면 그 기본 RCON 포트. `supported`에 rcon이 있고 stdin·rest가 없을 때만이다
    /// (명령 경로 우선순위 STDIN → REST → RCON: 다른 경로가 있으면 RCON을 쓰지 않는다)
    pub fn rcon_only_port(&self) -> Option<u16> {
        let supported = self.protocols_supported.as_deref()?;
        let only_rcon = supported.iter().any(|p| p == "rcon") && !supported.iter().any(|p| p == "stdin" || p == "rest");
        self.rcon_port.filter(|p| only_rcon && *p > 0)
    }

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
    
    /// REST API 기본 포트를 가져옵니다 (모듈에 정의되지 않으면 8212)
    pub fn default_rest_port(&self) -> u16 {
        self.get_setting_default("rest_port").unwrap_or(8212)
    }
    
    /// REST API 기본 호스트를 가져옵니다 (모듈에 정의되지 않으면 127.0.0.1)
    pub fn default_rest_host(&self) -> String {
        self.get_setting_default("rest_host").unwrap_or_else(|| "127.0.0.1".to_string())
    }


    /// 설정 이름 → 필드 타입 (`number`, `text`, `password`…). 파일에서 읽은 값을 타입대로 저장할 때 쓴다
    pub fn setting_types(&self) -> std::collections::HashMap<String, String> {
        self.settings.iter().flat_map(|s| &s.fields).map(|f| (f.name.clone(), f.field_type.clone())).collect()
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

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct ModuleCommands {
    pub fields: Vec<CommandField>,
    /// true면 lifecycle `instance_commands()`가 인스턴스마다 명령을 더 준다 (설치된 플러그인 명령 등).
    /// 해석은 `crate::commands::InstanceCommands`
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub hook: bool,
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
    /// 배경색 (CSS 색상값 또는 시맨틱 토큰)
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
    /// 겹칠 때 큰 값이 이긴다 (기본 0)
    #[serde(default)]
    pub priority: i32,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CommandField {
    pub name: String,
    pub label: String,
    pub description: Option<String>,
    /// "console"(서버 콘솔 stdin) | "rest" | "module"(lifecycle.py `command()`). 예전 이름 "rcon"은
    /// 읽을 때 "console"로, "dual"/"both"는 "module"로 본다. 해석은 `crate::commands`
    pub method: Option<String>,
    pub http_method: Option<String>,  // "GET", "POST" (REST only)
    pub endpoint_template: Option<String>,  // REST endpoint template
    /// 콘솔에 보낼 줄 (예: "kick {userid}"). 예전 이름 `rcon_template`
    #[serde(alias = "rcon_template")]
    pub console_template: Option<String>,
    /// 위험도: "safe" | "caution" | "danger". 없으면 "caution"으로 본다.
    /// 모델이 추론한 명령의 확인 정책이 이 값을 쓴다 (roadmap §10)
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub risk: Option<String>,
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
    /// number 입력의 허용 범위
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub min: Option<f64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub max: Option<f64>,
    /// 고를 수 있는 값 목록 (있으면 이 중 하나여야 한다)
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub options: Option<Vec<String>>,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
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
    /// 네트워크 포트 값인가. 같은 게임을 여러 개 만들 때 데몬이 겹치지 않게 옮긴다
    #[serde(default)]
    pub port: bool,
    /// 서버를 만들 때 고르는 설정 (받을 서버 파일이 달라지는 값. 예: 마인크래프트 서버 종류).
    /// GUI·CLI가 만들기 화면에서 묻고 `POST /api/instances`의 `module_settings`로 보낸다
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub install: bool,
    /// 이 값이 실행 인자로 들어가는 모양 (예: `-port={value}`, `-servername {value}`). `{value}`가 없으면
    /// boolean 플래그. 실행 인자 미리보기가 사용자가 친 인자를 이 필드로 되돌릴 때 쓴다 (`crate::launch_args`)
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub arg: Vec<String>,
    /// 값 형식: "memory_size"(4G·1024M) | "port" | "integer" | "path" (`crate::validator`)
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub format: Option<String>,
    /// 값 전체가 맞아야 하는 정규식
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub pattern: Option<String>,
    /// `list`의 항목 수 범위 (스키마 2)
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub min_items: Option<u32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub max_items: Option<u32>,
    /// `multiline`의 글자 수 상한 (스키마 2)
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub max_length: Option<u32>,
    /// 서버가 켜져 있을 때 바꾸면 다시 켜야 적용된다 (스키마 2). 데몬은 스스로 재시작하지 않는다
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub restart_required: bool,
    /// 다른 설정 값에 따라 보일지 (스키마 2)
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub visible_when: Option<VisibleWhen>,
}

/// `visible_when = { field, equals }` 또는 `{ field, in = [...] }` (06 §8.2)
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, schemars::JsonSchema)]
#[schemars(deny_unknown_fields)]
pub struct VisibleWhen {
    pub field: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub equals: Option<serde_json::Value>,
    #[serde(default, rename = "in", skip_serializing_if = "Option::is_none")]
    pub any_of: Option<Vec<serde_json::Value>>,
}

impl VisibleWhen {
    /// 비교할 값들 (`equals` 하나 또는 `in` 목록)
    pub fn values(&self) -> Vec<&serde_json::Value> {
        self.equals.iter().chain(self.any_of.iter().flatten()).collect()
    }
}

/// 문자열 하나 또는 여럿 (`arg = "-x"` / `arg = ["-a", "-b"]`)
#[derive(Debug, Deserialize, schemars::JsonSchema)]
#[serde(untagged)]
enum OneOrMany {
    One(String),
    Many(Vec<String>),
}

impl OneOrMany {
    fn into_vec(self) -> Vec<String> {
        match self {
            Self::One(s) => vec![s],
            Self::Many(v) => v,
        }
    }
}

/// 설정 값 형식 이름 (`format`)
pub const SETTING_FORMATS: &[&str] = &["memory_size", "port", "integer", "path"];
/// 설정 필드 `type`으로 받는 값 (GUI·CLI가 그리는 것)
pub const SETTING_TYPES: &[&str] = &["text", "password", "number", "file", "select", "boolean"];
/// 스키마 2에서 더한 설정 `type` (06 §8.1)
pub const SETTING_TYPES_V2: &[&str] = &["list", "multiline"];
/// `visible_when`이 가리킬 수 없는 설정 타입 (값을 하나로 비교할 수 없다)
const NOT_A_CONDITION: &[&str] = &["list", "multiline", "password", "file"];
/// 명령 입력 `inputs[].type`으로 받는 값
pub const COMMAND_INPUT_TYPES: &[&str] = &["text", "password", "number", "select", "boolean"];

/// 인스턴스 필드로도 저장되는 포트 설정 이름 (`ServerInstance::port` 등)
pub const CORE_PORT_FIELDS: &[&str] = &["port", "rest_port"];

/// 사바쨩 설정 묶음: 실행 방식(Java 경로·메모리·실행 인자 등)처럼 사바쨩이 쓰는 값. 게임 설정 파일로 넘기지 않는다
pub const SABA_SETTINGS_GROUP: &str = "saba-chan";

impl ModuleSettings {
    /// 포트로 표시된 설정 이름 (`port = true`)
    pub fn port_fields(&self) -> Vec<&str> {
        self.fields.iter().filter(|f| f.port).map(|f| f.name.as_str()).collect()
    }

    /// 사바쨩 설정 이름 (`group = "saba-chan"`)
    pub fn saba_fields(&self) -> Vec<&str> {
        self.fields.iter().filter(|f| f.group.as_deref() == Some(SABA_SETTINGS_GROUP)).map(|f| f.name.as_str()).collect()
    }
}

#[derive(Debug, Clone)]
pub struct LoadedModule {
    pub metadata: ModuleMetadata,
    pub path: String,  // 압축 해제된 디렉토리 또는 원본 디렉토리 절대 경로
}

impl LoadedModule {
    /// 데몬이 실행할 Python 진입 파일 (`[module] entry`)
    pub fn entry_path(&self) -> String {
        Path::new(&self.path).join(&self.metadata.entry).to_string_lossy().to_string()
    }

    /// 데몬이 이 모듈을 쓸 수 있는가: 진입 파일이 있고, `[module.dependencies] saba-core`를 만족한다 (M9)
    fn check_loadable(&self) -> Result<()> {
        if !Path::new(&self.entry_path()).is_file() {
            anyhow::bail!("module.entry '{}' not found in {}", self.metadata.entry, self.path);
        }
        let core = std::collections::HashMap::from([(CORE_COMPONENT.to_string(), DAEMON_VERSION.to_string())]);
        let needs_newer_core = self
            .metadata
            .check_dependencies(&core)
            .into_iter()
            .find(|(component, _, _)| component == CORE_COMPONENT);
        if let Some((_, required, _)) = needs_newer_core {
            return Err(UnmetCoreDependency { module: self.metadata.name.clone(), required }.into());
        }
        Ok(())
    }

    /// 이 폴더가 이 모듈의 게임 서버 폴더인가 (`[detection] dir_signatures`가 전부 있으면)
    pub fn matches_dir(&self, dir: &Path) -> bool {
        let sigs = &self.metadata.dir_signatures;
        !sigs.is_empty() && sigs.iter().all(|s| s.split('|').any(|alt| signature_present(dir, alt.trim())))
    }
}

/// `dir_signatures` 한 칸: 파일 이름이거나, `*`가 든 파일 이름 패턴 (그 폴더 바로 아래만 본다)
fn signature_present(dir: &Path, sig: &str) -> bool {
    if sig.is_empty() || sig.contains("..") {
        return false;
    }
    if !sig.contains('*') {
        return dir.join(sig).exists();
    }
    let Ok(pattern) = glob::Pattern::new(sig) else { return false };
    std::fs::read_dir(dir)
        .into_iter()
        .flatten()
        .flatten()
        .any(|e| pattern.matches(&e.file_name().to_string_lossy()))
}

/// 로드하지 못한 모듈과 그 이유 (`GET /api/modules`의 `failed`)
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ModuleLoadError {
    pub path: String,
    pub error: String,
    /// module.toml을 읽었으면 모듈 이름
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub module: Option<String>,
    /// 기계가 읽는 이유. 데몬 버전이 모자라면 `module_dependency_unsatisfied`
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub error_code: Option<String>,
    /// 못 맞춘 의존성 (`"saba-core >=0.3.0 (installed 0.1.1)"` 꼴)
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub missing_dependencies: Vec<String>,
    /// `module_schema_invalid`일 때 무엇이 틀렸는지 (필드 이름과 이유, 경로 없음)
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub problems: Vec<String>,
    /// `module_schema_unsupported`일 때 모듈이 요구한 스키마 버전
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub required: Option<u32>,
}

impl ModuleLoadError {
    fn new(path: &Path, e: &anyhow::Error) -> Self {
        let unmet = e.downcast_ref::<UnmetCoreDependency>();
        let invalid = e.downcast_ref::<SchemaInvalid>();
        let unsupported = e.downcast_ref::<SchemaUnsupported>();
        let error_code = if unmet.is_some() {
            Some("module_dependency_unsatisfied")
        } else if invalid.is_some() {
            Some("module_schema_invalid")
        } else if unsupported.is_some() {
            Some("module_schema_unsupported")
        } else {
            None
        };
        Self {
            path: path.to_string_lossy().to_string(),
            error: e.to_string(),
            module: unmet.map(|u| u.module.clone()),
            error_code: error_code.map(str::to_string),
            missing_dependencies: unmet
                .map(|u| vec![format!("{} {} (installed {})", CORE_COMPONENT, u.required, DAEMON_VERSION)])
                .unwrap_or_default(),
            problems: invalid.map(|i| i.problems.clone()).unwrap_or_default(),
            required: unsupported.map(|u| u.version),
        }
    }
}

/// module.toml이 선언한 스키마에 맞지 않는다 (`failed[]`의 `module_schema_invalid`)
#[derive(Debug, thiserror::Error)]
#[error("module.toml does not match schema v{version}: {joined}", joined = .problems.join("; "))]
pub struct SchemaInvalid {
    pub version: u32,
    pub problems: Vec<String>,
}

/// 이 데몬이 모르는 더 새 스키마 (`failed[]`의 `module_schema_unsupported`)
#[derive(Debug, thiserror::Error)]
#[error("module.toml schema_version {version} is not supported by this saba-chan (supports {MODULE_SCHEMA_VERSION}); update saba-chan")]
pub struct SchemaUnsupported {
    pub version: u32,
}

/// 모듈이 이 데몬보다 새 saba-core를 요구한다 (로드하지 않는다)
#[derive(Debug, thiserror::Error)]
#[error("module '{module}' requires saba-core {required} (this is {DAEMON_VERSION}); update saba-chan")]
pub struct UnmetCoreDependency {
    pub module: String,
    pub required: String,
}

// ── module.toml 스키마 v1 (serde 직접 역직렬화) ──
//
// 이 구조체들이 module.toml 계약 그 자체다. 키를 더하거나 빼면 `module_json_schema()`가
// 내보내는 JSON Schema(saba-chan-modules/schema/module.schema.json)도 같이 바뀐다.
// 모르는 키: schema_version = 1 이면 로드 거부, 없으면(스키마 이전 모듈) 경고만 남긴다.

/// 이 데몬이 아는 가장 새 module.toml 스키마 버전. 1과 2를 받는다 (2는 1에 06 §8의 설정 타입·키를 더한 것)
pub const MODULE_SCHEMA_VERSION: u32 = 2;
/// 스키마 표시가 없는 모듈에게 권하는 버전. 0.1.8 데몬도 읽는 1이다 (06 §7)
const BASE_SCHEMA_VERSION: u32 = 1;

/// 명령 위험도로 쓸 수 있는 값
/// 모듈 `[module.dependencies]`에서 데몬을 가리키는 키
pub const CORE_COMPONENT: &str = "saba-core";
/// 이 데몬의 버전 (모듈 호환 검사 기준)
pub const DAEMON_VERSION: &str = env!("CARGO_PKG_VERSION");
pub const COMMAND_RISKS: [&str; 3] = ["safe", "caution", "danger"];
/// `[protocols] supported`·`default`가 받는 값
pub const PROTOCOLS: [&str; 3] = ["rcon", "rest", "stdin"];

/// JSON 스키마에서도 로더와 같은 값만 받게 한다 (적합성 검사가 데몬과 어긋나지 않게)
fn schema_version_schema(_: &mut schemars::gen::SchemaGenerator) -> schemars::schema::Schema {
    serde_json::from_value(serde_json::json!({
        "description": "이 파일이 따르는 스키마 버전. 1, 또는 list·multiline 설정과 restart_required·visible_when 등을 쓰면 2(saba-core >=0.1.9)",
        "enum": [BASE_SCHEMA_VERSION, MODULE_SCHEMA_VERSION, null]
    }))
    .expect("static schema")
}

fn risk_schema(_: &mut schemars::gen::SchemaGenerator) -> schemars::schema::Schema {
    serde_json::from_value(serde_json::json!({
        "description": "\"safe\" | \"caution\" | \"danger\" (없으면 caution)",
        "enum": [COMMAND_RISKS[0], COMMAND_RISKS[1], COMMAND_RISKS[2], null]
    }))
    .expect("static schema")
}

/// module.toml 최상위 구조
#[derive(Debug, Deserialize, schemars::JsonSchema)]
#[schemars(deny_unknown_fields, title = "saba-chan module.toml (schema v1, v2)")]
struct ModuleToml {
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
    /// [credential_map] 데몬 표준 키 ↔ 게임별 설정 키 매핑
    /// 예: rest_password = "AdminPassword" → 데몬의 rest_password가 게임 INI의 AdminPassword에 대응
    #[serde(default)]
    credential_map: Option<std::collections::HashMap<String, String>>,
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
    /// [status] 켜진 서버의 상태를 더 알려 주는 함수
    #[serde(default)]
    status: Option<StatusSection>,
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
}

/// saba-runner가 서버 콘솔을 잇는 방식 (`[launch] console`)
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize, schemars::JsonSchema)]
#[serde(rename_all = "lowercase")]
pub enum ConsoleMode {
    #[default]
    Pipe,
    Pty,
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
    /// 게임 안 공지에 쓰는 `[commands.fields]` 이름 (`message` 입력 하나)
    #[serde(default)]
    announce_command: Option<String>,
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
    /// 서버를 만들 때 고르는 설정이면 true (설치할 파일이 달라지는 값)
    #[serde(default)]
    install: bool,
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

impl ModuleInstallConfig {
    /// 설치 방식 익스텐션에 넘기는 값 (`Instance::apply_install_extension_data`)
    pub fn extension_fields(&self) -> Vec<(&'static str, serde_json::Value)> {
        let mut fields = vec![("anonymous", serde_json::json!(self.anonymous))];
        if let Some(id) = self.app_id {
            fields.push(("app_id", serde_json::json!(id)));
        }
        if let Some(beta) = &self.beta {
            fields.push(("beta", serde_json::json!(beta)));
        }
        if let Some(platform) = &self.platform {
            fields.push(("platform", serde_json::json!(platform)));
        }
        if let Some(url) = &self.download_url {
            fields.push(("download_url", serde_json::json!(url)));
        }
        fields
    }
}

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
                install: f.install,
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
            }).collect();
            if fields.is_empty() && !c.hook { None } else { Some(ModuleCommands { fields, hook: c.hook }) }
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
            log_files: self.launch.map(|l| l.log_files).unwrap_or_default(),
            dependencies: self.module.dependencies,
            credential_map: self.credential_map.unwrap_or_default(),
            process_name: self.config.as_ref().and_then(|c| c.process_name.clone()),
            default_port: self.config.as_ref().and_then(|c| c.default_port),
            executable_path: self.config.as_ref().and_then(|c| c.executable_path.clone()),
            server_executable: self.config.as_ref().and_then(|c| c.server_executable.clone()),
            stop_command: self.config.as_ref().and_then(|c| c.stop_command.clone()),
            stop_timeout: self.config.as_ref().and_then(|c| c.stop_timeout),
            announce_command: self.config.as_ref().and_then(|c| c.announce_command.clone()),
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
            }).collect(),
            agreements: self.agreements,
            players_function: self.status.and_then(|s| s.players),
            warnings: Vec::new(),
        }
    }
}

/// TOML 문자열에서 ModuleMetadata를 파싱합니다.
///
/// - `schema_version`이 이 데몬보다 새로우면 거부한다 (모듈을 먼저 업데이트한 경우).
/// - `schema_version = 1`이면 모르는 키와 잘못된 값을 오류로 낸다 (오타가 조용히 사라지지 않게).
/// - `schema_version`이 없으면 스키마 이전 모듈로 보고 같은 문제를 `warnings`에만 남긴다.
pub(crate) fn parse_module_toml(content: &str) -> Result<ModuleMetadata> {
    let content = content.trim_start_matches('\u{feff}');
    let mut unknown = Vec::new();
    let mut de = toml::Deserializer::new(content);
    let toml_data: ModuleToml = serde_ignored::deserialize(&mut de, |path| {
        // serde_ignored는 Option 한 겹마다 `?`를 끼운다: "config.?.ram" → "config.ram"
        let key: Vec<String> = path.to_string().split('.').filter(|p| *p != "?").map(str::to_string).collect();
        unknown.push(key.join("."))
    })
    .map_err(|e| {
        // 스키마를 밝힌 모듈의 키 타입 오류도 `module_schema_invalid`로 알린다 (`failed[].problems`)
        let declared = content.parse::<toml::Value>().ok().and_then(|v| v.get("module")?.get("schema_version")?.as_integer());
        match declared.and_then(|v| u32::try_from(v).ok()).filter(|v| (1..=MODULE_SCHEMA_VERSION).contains(v)) {
            Some(version) => SchemaInvalid { version, problems: vec![e.to_string()] }.into(),
            None => anyhow::anyhow!("Failed to parse module.toml: {}", e),
        }
    })?;

    let mut problems: Vec<String> = unknown.into_iter().map(|k| format!("unknown key '{}'", k)).collect();
    problems.extend(toml_data.value_problems());

    let mut meta = toml_data.into_metadata();
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
    if let Some(problem) = meta.players_function.as_deref().and_then(|f| function_problem(f, "status.players")) {
        // 잘못 선언한 함수는 부르지 않는다
        meta.players_function = None;
        problems.push(problem);
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
    Ok(meta)
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
            if let Some(format) = field.format.as_deref().filter(|f| !SETTING_FORMATS.contains(f)) {
                problems.push(format!("settings field '{}': unknown format '{}' (one of {})", field.name, format, SETTING_FORMATS.join(", ")));
            }
            if let Some(Err(e)) = field.pattern.as_deref().map(regex::Regex::new) {
                problems.push(format!("settings field '{}': pattern is not a valid regex: {}", field.name, e));
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
        if let Some(p) = &self.protocols {
            for value in p.supported.iter().flatten().chain(&p.default) {
                if !PROTOCOLS.contains(&value.as_str()) {
                    problems.push(format!("protocols: '{}' must be one of {}", value, PROTOCOLS.join(", ")));
                }
            }
            let only_rcon = p.supported.as_ref().is_some_and(|s| s.iter().any(|v| v == "rcon") && !s.iter().any(|v| v == "stdin" || v == "rest"));
            if only_rcon && p.rcon_port.unwrap_or(0) == 0 {
                problems.push("protocols: rcon_port is required when rcon is the only protocol".to_string());
            }
        }
        // 공지·저장 명령은 [commands.fields]의 이름을 가리킨다
        let named = |name: &str| self.commands.iter().flat_map(|c| &c.fields).any(|f| f.name == name);
        let refs = [
            ("config.announce_command", self.config.as_ref().and_then(|c| c.announce_command.as_deref())),
            ("backup.save_command", self.backup.as_ref().and_then(|b| b.save_command.as_deref())),
        ];
        for (key, name) in refs {
            if let Some(name) = name.filter(|n| !named(n)) {
                problems.push(format!("{} '{}' is not a [[commands.fields]] name", key, name));
            }
        }
        problems
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

/// 모듈 폴더 하나를 데몬과 같은 규칙으로 읽는다 (`saba-core module validate`)
pub fn load_module_dir(module_path: &Path) -> Result<LoadedModule> {
    let toml_path = module_path.join("module.toml");
    let content = fs::read_to_string(&toml_path)
        .map_err(|e| anyhow::anyhow!("Failed to read {}: {}", toml_path.display(), e))?;
    let module = LoadedModule {
        metadata: parse_module_toml(&content)?,
        path: module_path.to_string_lossy().to_string(),
    };
    module.check_loadable()?;
    Ok(module)
}

/// module.toml 스키마 v1의 JSON Schema (편집기 자동완성·모듈 저장소 CI 검증용)
pub fn module_json_schema() -> serde_json::Value {
    serde_json::to_value(schemars::schema_for!(ModuleToml)).unwrap_or_default()
}

pub struct ModuleLoader {
    modules_dir: String,
    cached_modules: RwLock<Option<Vec<LoadedModule>>>,
    /// 마지막 탐색에서 로드하지 못한 모듈
    load_errors: RwLock<Vec<ModuleLoadError>>,
}

impl ModuleLoader {
    pub fn new(modules_dir: &str) -> Self {
        Self {
            modules_dir: modules_dir.to_string(),
            cached_modules: RwLock::new(None),
            load_errors: RwLock::new(Vec::new()),
        }
    }

    /// 마지막 탐색에서 로드하지 못한 모듈과 이유
    pub fn load_errors(&self) -> Vec<ModuleLoadError> {
        self.load_errors.read().unwrap().clone()
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
        let mut errors = Vec::new();

        if !Path::new(&self.modules_dir).exists() {
            tracing::warn!("Modules directory does not exist: {}", self.modules_dir);
            *self.cached_modules.write().unwrap() = Some(modules.clone());
            return Ok(modules);
        }

        for entry in fs::read_dir(&self.modules_dir)? {
            let entry = entry?;
            let path = entry.path();
            // 점으로 시작하는 것은 설치 중인 준비 폴더·옛 폴더·받는 중인 파일이다
            if entry.file_name().to_string_lossy().starts_with('.') {
                continue;
            }

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
                        errors.push(ModuleLoadError::new(&path, &e));
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
                            errors.push(ModuleLoadError::new(&path, &e));
                        }
                    }
                }
            }
        }

        *self.cached_modules.write().unwrap() = Some(modules.clone());
        *self.load_errors.write().unwrap() = errors;
        Ok(modules)
    }

    /// 폴더에서 개별 모듈 로드 (module.toml 파싱)
    fn load_module_from_dir(&self, module_path: &Path) -> Result<LoadedModule> {
        load_module_dir(module_path)
    }

    /// ZIP 파일에서 모듈 로드 (압축 해제 후 임시 디렉토리 사용)
    fn load_module_from_zip(&self, zip_path: &Path) -> Result<LoadedModule> {
        let file = fs::File::open(zip_path)?;
        let mut archive = ZipArchive::new(file)?;

        // ZIP 내부에서 가장 얕은 위치의 module.toml을 고른다
        // (`ends_with`로 고르면 `foo_module.toml`이나 하위 폴더 것이 먼저 잡힌다)
        let toml_index = (0..archive.len())
            .filter_map(|i| {
                let name = archive.by_index(i).ok()?.name().to_string();
                let depth = name.trim_end_matches('/').matches('/').count();
                (name.rsplit('/').next() == Some("module.toml")).then_some((depth, i))
            })
            .min()
            .map(|(_, i)| i);
        let mut toml_content = String::new();
        if let Some(i) = toml_index {
            archive.by_index(i)?.read_to_string(&mut toml_content)?;
        }

        if toml_content.is_empty() {
            return Err(anyhow::anyhow!("No module.toml found in ZIP"));
        }

        // 강타입 TOML 파싱
        let metadata = parse_module_toml(&toml_content)?;

        // 임시 디렉토리에 압축 해제
        let temp_dir = tempfile::tempdir()?;
        let extract_path = temp_dir.path().to_path_buf();
        
        let file = fs::File::open(zip_path)?;
        let mut archive = ZipArchive::new(file)?;
        archive.extract(&extract_path)?;

        // 임시 디렉토리 경로를 영구적으로 유지 (tempdir는 Drop시 삭제되므로)
        // 대신 modules/.extracted/ 디렉토리 사용
        if !crate::utils::is_single_path_component(&metadata.name) {
            anyhow::bail!("module name {:?} in {} is not a plain folder name", metadata.name, zip_path.display());
        }
        let extracted_dir = PathBuf::from(&self.modules_dir)
            .join(".extracted")
            .join(&metadata.name);
        
        if extracted_dir.exists() {
            fs::remove_dir_all(&extracted_dir)?;
        }
        fs::create_dir_all(&extracted_dir)?;
        
        // temp_dir에서 extracted_dir로 복사
        copy_dir_all(&extract_path, &extracted_dir)?;

        let module = LoadedModule {
            metadata,
            path: extracted_dir.to_string_lossy().to_string(),
        };
        module.check_loadable()?;
        Ok(module)
    }

    /// 모듈 이름으로 모듈 조회
    pub fn get_module(&self, name: &str) -> Result<LoadedModule> {
        let modules = self.discover_modules()?;
        modules
            .iter()
            .find(|m| m.metadata.name == name)
            .cloned()
            .ok_or_else(|| match self.load_failure(name) {
                // 있지만 이 데몬으로는 못 쓰는 모듈: "없음" 대신 이유를 준다
                Some(failed) => anyhow::anyhow!(failed.error),
                None => anyhow::anyhow!("Module '{}' not found", name),
            })
    }

    /// 이름이 `name`인 모듈을 읽었지만 쓸 수 없었다면 그 기록
    pub fn load_failure(&self, name: &str) -> Option<ModuleLoadError> {
        self.load_errors.read().unwrap().iter().find(|e| e.module.as_deref() == Some(name)).cloned()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn highlight_rules_carry_background_underline_priority() {
        let meta = parse_module_toml(
            "[module]\nname = \"m\"\nversion = \"1.0.0\"\nentry = \"lifecycle.py\"\nschema_version = 1\n\n\
             [[syntax_highlight.rules]]\nname = \"a\"\npattern = \"x\"\ncolor = \"warn\"\n\n\
             [[syntax_highlight.rules]]\nname = \"b\"\npattern = \"(?i)y\"\ncolor = \"#fff\"\n\
             background = \"dim\"\nunderline = true\npriority = 5\n",
        )
        .unwrap();
        let rules = &meta.syntax_highlight.unwrap().rules;
        assert_eq!((rules[0].underline, rules[0].priority, rules[0].background.as_deref()), (false, 0, None));
        assert_eq!((rules[1].underline, rules[1].priority, rules[1].background.as_deref()), (true, 5, Some("dim")));
    }

    #[test]
    fn folders_and_files_starting_with_a_dot_are_not_modules() {
        let tmp = tempfile::tempdir().unwrap();
        let toml = "[module]\nschema_version = 1\nname = \"plain\"\nversion = \"1.0.0\"\nentry = \"lifecycle.py\"\n";
        for dir in ["plain", ".old-plain"] {
            std::fs::create_dir_all(tmp.path().join(dir)).unwrap();
            std::fs::write(tmp.path().join(dir).join("module.toml"), toml).unwrap();
            std::fs::write(tmp.path().join(dir).join("lifecycle.py"), "").unwrap();
        }
        std::fs::write(tmp.path().join(".tmp-module-plain.zip"), "partial").unwrap();
        let loader = ModuleLoader::new(tmp.path().to_str().unwrap());
        assert_eq!(loader.discover_modules().unwrap().len(), 1);
        assert!(loader.load_errors().is_empty(), "{:?}", loader.load_errors());
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
    fn commands_hook_alone_keeps_the_commands_section() {
        let meta = parse_module_toml(
            "[module]\nname = \"game\"\nversion = \"1.0.0\"\nentry = \"lifecycle.py\"\n\n[commands]\nhook = true\n",
        )
        .unwrap();
        let commands = meta.commands.expect("hook without fields still declares the section");
        assert!(commands.hook && commands.fields.is_empty());
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
        assert_eq!(commands.fields[0].console_template.as_deref(), Some("list"), "old name rcon_template still loads");
        assert_eq!(commands.fields[0].method.as_deref(), Some("console"), "old method name rcon becomes console");
        let v1 = format!("{V1}\n[[commands.fields]]\nname = \"say\"\nmethod = \"console\"\nconsole_template = \"say {{message}}\"\n");
        let meta = parse_module_toml(&v1).unwrap();
        assert!(meta.warnings.is_empty(), "{:?}", meta.warnings);
        assert_eq!(meta.commands.unwrap().fields[0].console_template.as_deref(), Some("say {message}"));
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
        assert_eq!(meta.rcon_only_port(), None, "rest is there: rcon is never used");
    }

    #[test]
    fn rcon_is_used_only_when_it_is_the_only_protocol_and_needs_a_port() {
        let with = |protocols: &str| parse_module_toml(&format!("[module]\nschema_version = 1\nname = \"g\"\nversion = \"1\"\nentry = \"lifecycle.py\"\n[protocols]\n{protocols}\n"));
        assert_eq!(with("supported = [\"rcon\"]\nrcon_port = 27015").unwrap().rcon_only_port(), Some(27015));
        assert_eq!(with("supported = [\"stdin\", \"rcon\"]\nrcon_port = 27015").unwrap().rcon_only_port(), None);
        let err = with("supported = [\"rcon\"]").unwrap_err().to_string();
        assert!(err.contains("rcon_port"), "{err}");
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
    fn launch_log_files_are_read() {
        let toml = "[module]\nname = \"g\"\nversion = \"1.0.0\"\nentry = \"lifecycle.py\"\n\n[launch]\nlog_files = [\"Pal/Saved/Logs/*.log\"]\n";
        assert_eq!(parse_module_toml(toml).unwrap().log_files, vec!["Pal/Saved/Logs/*.log"]);
        let plain = "[module]\nname = \"g\"\nversion = \"1.0.0\"\nentry = \"lifecycle.py\"\n";
        assert!(parse_module_toml(plain).unwrap().log_files.is_empty());
        assert_eq!(parse_module_toml(plain).unwrap().console, ConsoleMode::Pipe);
        let pty = toml.replace("log_files", "console = \"pty\"\nlog_files");
        assert_eq!(parse_module_toml(&pty).unwrap().console, ConsoleMode::Pty);
    }

    const V1: &str = "[module]\nschema_version = 1\nname = \"g\"\nversion = \"1.0.0\"\nentry = \"lifecycle.py\"\n";

    #[test]
    fn schema_v1_rejects_unknown_keys() {
        let err = parse_module_toml(&format!("{V1}\n[config]\nram = \"4G\"\n")).unwrap_err().to_string();
        assert!(err.contains("config.ram"), "{err}");
        let err = parse_module_toml(&format!("{V1}\n[[commands.fields]]\nname = \"kick\"\nrcon_templat = \"kick\"\n"))
            .unwrap_err()
            .to_string();
        assert!(err.contains("rcon_templat"), "{err}");
        assert!(parse_module_toml(V1).unwrap().warnings.is_empty());
    }

    #[test]
    fn modules_without_schema_version_still_load_with_warnings() {
        let toml = "\u{feff}[module]\nname = \"g\"\nversion = \"1.0.0\"\nentry = \"lifecycle.py\"\n\n[config]\nram = \"4G\"\n";
        let meta = parse_module_toml(toml).unwrap();
        assert_eq!(meta.schema_version, 0);
        assert!(meta.warnings.iter().any(|w| w.contains("config.ram")), "{:?}", meta.warnings);
        assert!(meta.warnings.iter().any(|w| w.contains("schema_version")));
    }

    /// 06 §2·§6: players_detail·stop_call_limit은 데몬 몫이다. 같은 이름의 설정은 로드하되 경고한다
    #[test]
    fn settings_named_like_daemon_call_keys_are_warned_about() {
        let meta = parse_module_toml(&format!("{V1}\n[[settings.fields]]\nname = \"players_detail\"\ntype = \"boolean\"\n")).unwrap();
        assert!(meta.warnings.iter().any(|w| w.contains("players_detail")), "{:?}", meta.warnings);
    }

    #[test]
    fn newer_schema_version_is_refused() {
        let toml = V1.replace("schema_version = 1", "schema_version = 3");
        let err = parse_module_toml(&toml).unwrap_err();
        assert!(err.to_string().contains("schema_version 3"), "{err}");
        let failed = ModuleLoadError::new(Path::new("m"), &err);
        assert_eq!((failed.error_code.as_deref(), failed.required), (Some("module_schema_unsupported"), Some(3)));
    }

    /// MK2 (06 §7): 스키마 2는 스키마 1에 list·multiline 타입과 설정 키를 더한 것이다. 스키마 1에 있으면 로드 실패이고,
    /// 실패한 모듈은 `failed[]`에 `module_schema_invalid`와 `problems`로 보인다
    #[test]
    fn schema_v2_adds_setting_types_and_keys_that_schema_v1_refuses() {
        let fields = "[[settings.fields]]\nname = \"admins\"\ntype = \"list\"\nmax_items = 5\nrestart_required = true\n\n\
                      [[settings.fields]]\nname = \"motd\"\ntype = \"multiline\"\nmax_length = 200\n";
        let v2 = parse_module_toml(&format!("{}\n{fields}", V1.replace("schema_version = 1", "schema_version = 2"))).unwrap();
        assert_eq!(v2.schema_version, 2);
        let admins = &v2.settings.as_ref().unwrap().fields[0];
        assert_eq!((admins.field_type.as_str(), admins.max_items, admins.restart_required), ("list", Some(5), true));

        let err = parse_module_toml(&format!("{V1}\n{fields}")).unwrap_err();
        let failed = ModuleLoadError::new(Path::new("m"), &err);
        assert_eq!(failed.error_code.as_deref(), Some("module_schema_invalid"));
        for needle in ["'list'", "'multiline'", "max_items", "restart_required", "max_length"] {
            assert!(failed.problems.iter().any(|p| p.contains(needle)), "{needle}: {:?}", failed.problems);
        }
        assert!(failed.error.contains("schema v1"), "{}", failed.error);
    }

    /// 스키마를 밝힌 모듈은 알려진 키의 타입 오류도 `module_schema_invalid`와 `problems`로 보이고,
    /// list 기본값은 항목 수·중복·형식까지 PATCH와 같은 검사를 받는다
    #[test]
    fn schema_type_errors_and_bad_list_defaults_are_schema_problems() {
        let v2 = V1.replace("schema_version = 1", "schema_version = 2");
        let err = parse_module_toml(&format!("{v2}\n[[settings.fields]]\nname = \"admins\"\ntype = \"list\"\nmin_items = \"two\"\n")).unwrap_err();
        let failed = ModuleLoadError::new(Path::new("m"), &err);
        assert_eq!(failed.error_code.as_deref(), Some("module_schema_invalid"), "{}", failed.error);
        assert!(failed.problems.iter().any(|p| p.contains("min_items")), "{:?}", failed.problems);

        let list = |keys: &str, default: &str| parse_module_toml(&format!("{v2}\n[[settings.fields]]\nname = \"admins\"\ntype = \"list\"\n{keys}\ndefault = {default}\n"));
        assert!(list("max_items = 2\npattern = \"[a-z]+\"", "[\"a\", \"b\"]").is_ok());
        for (keys, default) in [("max_items = 1", "[\"a\", \"b\"]"), ("min_items = 2", "[\"a\"]"), ("", "[\"a\", \"a\"]"), ("pattern = \"[a-z]+\"", "[\"A\"]")] {
            let err = list(keys, default).unwrap_err();
            let failed = ModuleLoadError::new(Path::new("m"), &err);
            assert!(failed.problems.iter().any(|p| p.contains("'admins'") && p.contains("default")), "{keys} {default}: {:?}", failed.problems);
        }
    }

    /// 리뷰 554: 스키마 이전 모듈의 list·multiline은 0.1.8처럼 글자로 다루고, list 프리셋 값도 로드 때 검사한다
    #[test]
    fn pre_schema_list_types_stay_text_and_list_presets_are_checked() {
        let old = format!("{}\n[[settings.fields]]\nname = \"admins\"\ntype = \"list\"\n\n[[settings.fields]]\nname = \"motd\"\ntype = \"multiline\"\n", V1.replace("schema_version = 1\n", ""));
        let meta = parse_module_toml(&old).unwrap();
        let types: Vec<&str> = meta.settings.as_ref().unwrap().fields.iter().map(|f| f.field_type.as_str()).collect();
        assert_eq!(types, ["text", "text"]);

        let v2 = V1.replace("schema_version = 1", "schema_version = 2");
        let preset = |value: &str| {
            parse_module_toml(&format!(
                "{v2}\n[[settings.fields]]\nname = \"mode\"\ntype = \"list\"\noptions = [\"a\", \"b\"]\n\n[[presets]]\nid = \"p\"\nlabel = \"P\"\nsettings = {{ mode = {value} }}\n"
            ))
        };
        assert!(preset("[\"a\"]").is_ok());
        for bad in ["\"a\"", "[\"c\"]", "[\"a\", \"a\"]"] {
            let err = preset(bad).unwrap_err().to_string();
            assert!(err.contains("presets 'p'"), "{bad}: {err}");
        }
    }

    #[test]
    fn schema_v1_checks_values() {
        let bad_risk = format!("{V1}\n[[commands.fields]]\nname = \"stop\"\nrisk = \"scary\"\n");
        assert!(parse_module_toml(&bad_risk).unwrap_err().to_string().contains("risk"));
        let ok_risk = bad_risk.replace("scary", "danger");
        let meta = parse_module_toml(&ok_risk).unwrap();
        assert_eq!(meta.commands.unwrap().fields[0].risk.as_deref(), Some("danger"));
        let escape = V1.replace("lifecycle.py", "../evil.py");
        assert!(parse_module_toml(&escape).unwrap_err().to_string().contains("module.entry"));
        for bad in ["supported = [\"stdin\", \"x</b>\"]", "supported = [\"stdin\"]\ndefault = \"telnet\""] {
            let err = parse_module_toml(&format!("{V1}\n[protocols]\n{bad}\n")).unwrap_err().to_string();
            assert!(err.contains("protocols"), "{err}");
        }
        assert!(parse_module_toml(&format!("{V1}\n[protocols]\nsupported = [\"rest\", \"stdin\"]\ndefault = \"rest\"\n")).is_ok());
    }

    /// 클라이언트가 그리지 못하는 설정·명령 입력 type은 화면에서 조용히 사라진다: 로드할 때 알린다 (MK2)
    #[test]
    fn schema_v1_checks_setting_and_command_input_types() {
        let setting = |ty: &str| format!("{V1}\n[[settings.fields]]\nname = \"motd\"\ntype = \"{ty}\"\n");
        let err = parse_module_toml(&setting("textarea")).unwrap_err().to_string();
        assert!(err.contains("settings field 'motd'") && err.contains("textarea"), "{err}");
        for ty in SETTING_TYPES {
            assert!(parse_module_toml(&setting(ty)).is_ok(), "{ty}");
        }
        let input = |ty: &str| format!("{V1}\n[[commands.fields]]\nname = \"kick\"\ninputs = [{{ name = \"player\", type = \"{ty}\" }}]\n");
        let err = parse_module_toml(&input("slider")).unwrap_err().to_string();
        assert!(err.contains("commands 'kick'") && err.contains("slider"), "{err}");
        assert!(parse_module_toml(&input("password")).is_ok());
        // 스키마 이전 모듈은 경고만 하고 올린다
        assert!(parse_module_toml(&setting("textarea").replace("schema_version = 1\n", "")).is_ok());
    }

    /// 06 §8.2: visible_when은 같은 모듈의 값 하나짜리 설정을 가리키고, 값은 그 설정 타입에 맞아야 하며, 순환하면 안 된다
    #[test]
    fn visible_when_is_checked_when_the_module_loads() {
        let v2 = V1.replace("schema_version = 1", "schema_version = 2");
        let module = |cond: &str| {
            parse_module_toml(&format!(
                "{v2}\n[[settings.fields]]\nname = \"pvp\"\ntype = \"boolean\"\n\n[[settings.fields]]\nname = \"port\"\ntype = \"number\"\n\n\
                 [[settings.fields]]\nname = \"admins\"\ntype = \"list\"\n\n[[settings.fields]]\nname = \"pvp_damage\"\ntype = \"text\"\nvisible_when = {cond}\n"
            ))
        };
        let ok = module("{ field = \"pvp\", equals = true }").unwrap();
        let cond = ok.settings.unwrap().fields[3].visible_when.clone().unwrap();
        assert_eq!((cond.field.as_str(), cond.equals), ("pvp", Some(serde_json::json!(true))));
        assert!(module("{ field = \"port\", in = [25565, 25566] }").is_ok());
        for (cond, needle) in [
            ("{ field = \"nope\", equals = 1 }", "not a setting"),
            ("{ field = \"pvp_damage\", equals = \"x\" }", "itself"),
            ("{ field = \"admins\", equals = \"a\" }", "list setting"),
            ("{ field = \"pvp\", equals = \"yes\" }", "does not fit"),
            ("{ field = \"pvp\" }", "exactly one"),
        ] {
            let err = module(cond).unwrap_err().to_string();
            assert!(err.contains(needle), "{cond}: {err}");
        }
        let cycle = format!(
            "{v2}\n[[settings.fields]]\nname = \"a\"\nvisible_when = {{ field = \"b\", equals = \"x\" }}\n\n[[settings.fields]]\nname = \"b\"\nvisible_when = {{ field = \"a\", equals = \"x\" }}\n"
        );
        assert!(parse_module_toml(&cycle).unwrap_err().to_string().contains("cycle"));
    }

    #[test]
    fn schema_v1_reads_names_detection_and_extension_sections() {
        let toml = format!(
            "{V1}display_name = \"G\"\ngame_name = \"Game\"\nauthor = \"me\"\n\n[detection]\ngraceful_exit_patterns = [\"Stopping\"]\ncrash_artifacts = [\"crash-reports/*.txt\"]\n\n[extensions.backup]\npaths = [\"world\"]\n"
        );
        let meta = parse_module_toml(&toml).unwrap();
        assert_eq!(meta.display_name.as_deref(), Some("G"));
        assert_eq!(meta.game_name.as_deref(), Some("Game"));
        assert_eq!(meta.graceful_exit_patterns, vec!["Stopping"]);
        assert_eq!(meta.crash_artifacts, vec!["crash-reports/*.txt"]);
        assert_eq!(meta.extensions["backup"]["paths"][0], "world");
    }

    #[test]
    fn schema_v1_reads_backup_and_checks_command_refs() {
        let toml = format!(
            "{V1}\n[config]\nannounce_command = \"say\"\n\n[backup]\npaths = [\"{{level_name}}\"]\nsave_command = \"save-all\"\n\n[[commands.fields]]\nname = \"say\"\n\n[[commands.fields]]\nname = \"save-all\"\n"
        );
        let meta = parse_module_toml(&toml).unwrap();
        let backup = meta.backup.unwrap();
        assert_eq!(backup.paths, vec!["{level_name}"]);
        assert_eq!(backup.save_command.as_deref(), Some("save-all"));
        assert!(!backup.hook);
        assert_eq!(meta.announce_command.as_deref(), Some("say"));
        let typo = toml.replace("save_command = \"save-all\"", "save_command = \"save_all\"");
        assert!(parse_module_toml(&typo).unwrap_err().to_string().contains("backup.save_command"));
    }

    #[test]
    fn schema_v1_reads_presets_and_rejects_bad_ones() {
        let toml = format!(
            "{V1}\n[[settings.fields]]\nname = \"max_players\"\ntype = \"number\"\nmin = 1\nmax = 100\n\n[[settings.fields]]\nname = \"difficulty\"\ntype = \"select\"\noptions = [\"easy\", \"hard\"]\n\n[[settings.fields]]\nname = \"secret\"\ntype = \"password\"\n\n[[presets]]\nid = \"friends\"\nlabel = \"Friends\"\n[presets.settings]\nmax_players = 10\ndifficulty = \"easy\"\n"
        );
        let meta = parse_module_toml(&toml).unwrap();
        assert_eq!(meta.presets.len(), 1);
        assert_eq!(meta.presets[0].id, "friends");
        assert_eq!(meta.presets[0].settings["max_players"], 10);
        for (bad, why) in [
            ("max_players = 10", "max_players = 500"),
            ("difficulty = \"easy\"", "difficulty = \"peaceful\""),
            ("difficulty = \"easy\"", "secret = \"x\""),
            ("difficulty = \"easy\"", "nope = 1"),
            ("id = \"friends\"", "id = \"Friends!\""),
        ] {
            let broken = toml.replace(bad, why);
            assert!(parse_module_toml(&broken).unwrap_err().to_string().contains("presets"), "{why}");
        }
    }

    #[test]
    fn schema_v1_reads_queries_and_rejects_bad_ones() {
        let toml = format!(
            "{V1}\n[[queries]]\nname = \"players\"\nlabel = \"Players\"\nfunction = \"list_players\"\n\n[[queries]]\nname = \"player-info\"\nfunction = \"player_info\"\ninputs = [{{ name = \"player\", type = \"text\", required = true }}, {{ name = \"limit\", type = \"number\", min = 1, max = 50 }}]\n"
        );
        let meta = parse_module_toml(&toml).unwrap();
        assert_eq!(meta.queries.len(), 2);
        assert_eq!((meta.queries[0].name.as_str(), meta.queries[0].function.as_str()), ("players", "list_players"));
        assert_eq!(meta.queries[1].inputs[1].max, Some(50.0));
        for (bad, why) in [
            ("name = \"player-info\"", "name = \"players\""),
            ("name = \"players\"", "name = \"Players!\""),
            ("function = \"list_players\"", "function = \"list-players\""),
            ("function = \"list_players\"", "function = \"stop\""),
            ("type = \"text\"", "type = \"password\""),
            ("name = \"limit\"", "name = \"player\""),
            ("min = 1, max = 50", "min = 9, max = 2"),
        ] {
            let broken = toml.replacen(bad, why, 1);
            assert!(parse_module_toml(&broken).unwrap_err().to_string().contains("queries"), "{why}");
        }
        let err = parse_module_toml(&format!("{V1}\n[[queries]]\nname = \"p\"\nfunction = \"f\"\nrisk = \"safe\"\n")).unwrap_err().to_string();
        assert!(err.contains("risk"), "{err}");
        let legacy = toml.replacen("schema_version = 1\n", "", 1).replacen("function = \"list_players\"", "function = \"reset_server\"", 1);
        assert_ne!(legacy, toml);
        let meta = parse_module_toml(&legacy).unwrap();
        assert!(meta.queries.is_empty(), "a module without a schema keeps no queries when one is wrong");
    }

    #[test]
    fn a_module_names_the_function_that_counts_players() {
        let toml = format!("{V1}\n[status]\nplayers = \"player_count\"\n");
        assert_eq!(parse_module_toml(&toml).unwrap().players_function.as_deref(), Some("player_count"));
        assert_eq!(parse_module_toml(V1).unwrap().players_function, None);
        for bad in ["stop", "player-count"] {
            let err = parse_module_toml(&toml.replace("player_count", bad)).unwrap_err().to_string();
            assert!(err.contains("status.players"), "{err}");
        }
        let legacy = toml.replacen("schema_version = 1\n", "", 1).replace("player_count", "reset_server");
        assert_eq!(parse_module_toml(&legacy).unwrap().players_function, None, "never call a function that changes the server");
    }

    #[test]
    fn schema_v1_reads_agreements_and_rejects_bad_ones() {
        let toml = format!("{V1}\n[[agreements]]\nid = \"eula\"\nurl = \"https://example.com/eula\"\n");
        let meta = parse_module_toml(&toml).unwrap();
        assert_eq!(meta.agreements, vec![ModuleAgreement { id: "eula".into(), url: "https://example.com/eula".into() }]);
        for (bad, why) in [("id = \"eula\"", "id = \"EULA\""), ("https://", "http://"), ("url = ", "title = \"x\"\nurl = ")] {
            assert!(parse_module_toml(&toml.replacen(bad, why, 1)).is_err(), "{why}");
        }
        let twice = format!("{toml}\n[[agreements]]\nid = \"eula\"\nurl = \"https://example.com/2\"\n");
        assert!(parse_module_toml(&twice).unwrap_err().to_string().contains("duplicate"));
    }

    #[test]
    fn json_schema_forbids_unknown_keys() {
        let schema = module_json_schema();
        assert_eq!(schema["additionalProperties"], false);
        assert_eq!(schema["required"], serde_json::json!(["module"]));
        let defs = &schema["definitions"];
        assert_eq!(defs["ConfigSection"]["additionalProperties"], false);
        assert!(defs["ModuleSection"]["properties"]["schema_version"].is_object());
    }

    #[test]
    fn loader_reports_broken_modules_and_missing_entry() {
        let dir = tempfile::tempdir().unwrap();
        let good = dir.path().join("good");
        std::fs::create_dir_all(&good).unwrap();
        std::fs::write(good.join("module.toml"), V1.replace("\"g\"", "\"good\"")).unwrap();
        std::fs::write(good.join("lifecycle.py"), "").unwrap();
        std::fs::write(good.join("server.jar"), "").unwrap();
        let no_entry = dir.path().join("noentry");
        std::fs::create_dir_all(&no_entry).unwrap();
        std::fs::write(no_entry.join("module.toml"), V1).unwrap();
        let loader = ModuleLoader::new(dir.path().to_str().unwrap());
        let modules = loader.discover_modules().unwrap();
        assert_eq!(modules.len(), 1);
        assert!(modules[0].entry_path().ends_with("lifecycle.py"));
        let errors = loader.load_errors();
        assert_eq!(errors.len(), 1);
        assert!(errors[0].error.contains("not found"), "{:?}", errors);

        let mut m = modules[0].clone();
        assert!(!m.matches_dir(&good), "no signatures, no match");
        m.metadata.dir_signatures = vec!["server.jar".into(), "lifecycle.py".into()];
        assert!(m.matches_dir(&good));
        m.metadata.dir_signatures.push("missing.txt".into());
        assert!(!m.matches_dir(&good));

        // 한 항목 안의 `|`는 그중 하나, `*`는 파일 이름 패턴 (Paper·Fabric처럼 jar 이름이 버전마다 다른 서버)
        std::fs::write(good.join("paper-1.21.4-15.jar"), "").unwrap();
        m.metadata.dir_signatures = vec!["missing.jar|paper-*.jar".into(), "lifecycle.py".into()];
        assert!(m.matches_dir(&good));
        m.metadata.dir_signatures = vec!["missing.jar|purpur-*.jar".into()];
        assert!(!m.matches_dir(&good));
        m.metadata.dir_signatures = vec!["../*".into()];
        assert!(!m.matches_dir(&good), "patterns stay inside the folder");
    }

    /// 모듈이 더 새 saba-core를 요구하면 로드하지 않고 이유를 남긴다 (M9)
    #[test]
    fn module_needing_newer_core_is_refused() {
        let dir = tempfile::tempdir().unwrap();
        for (name, need) in [("fits", DAEMON_VERSION.to_string()), ("future", "99.0.0".to_string())] {
            let m = dir.path().join(name);
            std::fs::create_dir_all(&m).unwrap();
            std::fs::write(
                m.join("module.toml"),
                format!("{}\n[module.dependencies]\n\"saba-core\" = \">={}\"\n", V1.replace("\"g\"", &format!("\"{name}\"")), need),
            )
            .unwrap();
            std::fs::write(m.join("lifecycle.py"), "").unwrap();
        }
        let loader = ModuleLoader::new(dir.path().to_str().unwrap());
        let modules = loader.discover_modules().unwrap();
        assert_eq!(modules.iter().map(|m| m.metadata.name.as_str()).collect::<Vec<_>>(), ["fits"]);
        let errors = loader.load_errors();
        assert!(errors[0].error.contains("requires saba-core >=99.0.0"), "{:?}", errors);
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
"cli" = ">=0.1.0"
"#;
        let meta = parse_module_toml(toml).unwrap();
        let mut installed = std::collections::HashMap::new();
        installed.insert("saba-core".to_string(), "0.5.0".to_string());
        installed.insert("gui".to_string(), "0.3.0".to_string());
        installed.insert("ext-steamcmd".to_string(), "1.0.0".to_string());
        installed.insert("cli".to_string(), "0.1.0".to_string());

        let issues = meta.check_dependencies(&installed);
        assert!(issues.is_empty(), "All cross-type dependencies should be satisfied");

        // Now test with one missing
        let mut partial = std::collections::HashMap::new();
        partial.insert("saba-core".to_string(), "0.5.0".to_string());
        partial.insert("gui".to_string(), "0.1.0".to_string()); // too low
        partial.insert("ext-steamcmd".to_string(), "1.0.0".to_string());
        // cli missing entirely

        let issues = meta.check_dependencies(&partial);
        assert_eq!(issues.len(), 2, "gui too low + cli missing");
    }
}

/// \ub514\ub809\ud1a0\ub9ac \uc804\uccb4 \ubcf5\uc0ac \ud5ec\ud37c \ud568\uc218
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

