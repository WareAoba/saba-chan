//! `module.toml`이 데몬에 주는 공개 메타데이터 타입과 상수

use super::*;
use std::path::Path;
use anyhow::Result;
use serde::{Deserialize, Serialize};

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
    /// [config] stop_signal — 일반 정지에 콘솔 명령 대신 보낼 신호 (`interrupt` = Ctrl+C)
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub stop_signal: Option<StopSignal>,
    /// [config] announce_command — 게임 안 공지에 쓰는 `[commands.fields]` 이름 (`message` 입력 하나). 예약 재시작 예고·공지(A3)용
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub announce_command: Option<String>,
    /// [config] kick_command — 플레이어를 내보내는 `[commands.fields]` 이름 (첫 입력 닉네임, 둘째 입력이 있으면 이유).
    /// 조작 가능한 사람만 접속(`crate::controllers`)이 쓴다
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub kick_command: Option<String>,
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
    /// `[protocols.rest]`: REST 접속값(호스트·포트·비밀번호)이 든 설정 이름과 사용자 이름 (`ModuleMetadata::rest`)
    #[serde(default)]
    pub rest: RestSettings,
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
    /// [launch] line_start — 줄바꿈 없이 이어 쓴 출력을 나눌 줄 머리 (정규식). saba-runner `--line-start`
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub line_start: Option<String>,
    /// [detection] graceful_exit_patterns — 정상 종료 때 콘솔에 찍히는 문구 (정규식). A1 크래시 판정용
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub graceful_exit_patterns: Vec<String>,
    /// [detection] crash_artifacts — 크래시 때 생기는 파일 (작업 디렉터리 기준, 마지막 칸만 `*` `?`). A1용
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub crash_artifacts: Vec<String>,
    /// [extensions.saba-core] ready_patterns — 서버가 접속을 받기 시작할 때 콘솔에 찍히는 문구 (정규식). R39 `instance.ready`용
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub ready_patterns: Vec<String>,
    /// [extensions.saba-core] progress_patterns — 갱신마다 새 줄로 찍히는 진행 표시 (정규식). 러너가 단계마다 첫·마지막 줄로 접는다
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub progress_patterns: Vec<String>,
    /// [extensions.saba-core] platforms — 이 모듈이 돌아가는 OS (`std::env::consts::OS` 이름: windows·linux·macos). 없으면 막지 않는다 (D3·M1)
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub platforms: Option<Vec<String>>,
    /// [extensions.saba-core] architectures — 이 모듈이 돌아가는 CPU (`std::env::consts::ARCH` 이름: x86_64·aarch64). 없으면 막지 않는다
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub architectures: Option<Vec<String>>,
    /// [extensions.saba-core] ready_timeout — 준비 문구를 기다리는 시간(초). 없으면 600
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub ready_timeout: Option<u64>,
    /// [extensions.saba-core] player_join_patterns·player_leave_patterns — 접속·퇴장 때 콘솔에 찍히는 문구 (정규식, `(?P<name>…)` 필수). MK8 `instance.player`용
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub player_join_patterns: Vec<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub player_leave_patterns: Vec<String>,
    /// [extensions.saba-core] chat_patterns — 게임 채팅이 콘솔에 찍히는 문구 (정규식, `(?P<name>…)`·`(?P<message>…)` 필수). 게임 채팅 전달(`game_chat.rs`)용
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub chat_patterns: Vec<String>,
    /// [extensions.saba-core] chat_send — 디스코드 글을 게임 채팅으로 보내는 lifecycle 함수 (config의 `chat_name`·`chat_text`). 액션 `server.chat`용
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub chat_send_function: Option<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub data_roots: Vec<String>,
    /// [[presets]] 서버를 만들 때 고르는 추천 설정 묶음 (I1-b)
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub presets: Vec<ModulePreset>,
    /// [[queries]] 읽기 전용 조회 (X-2). 액션 `server.query:<name>`이 된다
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub queries: Vec<ModuleQuery>,
    /// [[agreements]] 서버를 켜기 전에 사용자가 동의해야 하는 약관 (예: 게임 EULA)
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub agreements: Vec<ModuleAgreement>,
    /// [[lists]] 플레이어 목록 (화이트리스트·관리자·밴 같은 이름 명단). `GET /api/instance/:id/lists`가 읽어 보인다
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub lists: Vec<ModuleList>,
    /// [status] players — 켜진 서버의 접속자 수를 돌려주는 lifecycle 함수 (Q-5, `supervisor::players`)
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub players_function: Option<String>,
    /// [[diagnose]] 로그 진단 규칙. 데몬은 검사만 하고, 모듈 `diagnose_log()`가 공용 도우미
    /// `daemon_rcon.load_diagnose_rules`·`diagnose_lines`로 쓴다 (saba-core 0.1.11)
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub diagnose: Vec<DiagnoseRule>,
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
    /// 부른 사람만 보면 되는 결과 (길드 설정 "조회 결과 나만 보기"가 켜지면 디스코드 답을 부른 사람에게만)
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub private: bool,
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

/// `[[lists]]` — 게임이 지키는 플레이어 이름 명단 하나 (예: 화이트리스트, 관리자, 밴).
/// 지금 명단은 lifecycle `read_lists`가 읽어 주고, 넣기·빼기는 이미 선언한 명령(`[[commands.fields]]`)으로 한다.
/// 이름은 모듈 locales의 `lists.<id>`
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, schemars::JsonSchema)]
#[schemars(deny_unknown_fields)]
pub struct ModuleList {
    /// 소문자·숫자·`-`·`_` (모듈 안에서 겹치지 않음, 예: "whitelist")
    pub id: String,
    /// 이름을 넣는 `[[commands.fields]]` 이름. 첫 입력이 플레이어 이름이고, 나머지 입력(예: 이유)은 화면이 옆 칸으로 보인다
    pub add: String,
    /// 이름을 빼는 `[[commands.fields]]` 이름 (첫 입력이 플레이어 이름)
    pub remove: String,
    /// 이 명단을 쓸지 정하는 boolean `[[settings.fields]]` 이름 (예: 화이트리스트 사용). 없으면 늘 쓰는 명단
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub setting: Option<String>,
    /// 넣으면 그 사람을 막는 명단 (밴). 화면이 넣기 단추를 위험 색으로 보인다
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub danger: bool,
}

/// `[[diagnose]]` — 콘솔 줄에서 알려진 문제를 찾는 규칙. 문구는 모듈 locales의
/// `errors.detect.<소문자 code>`·`errors.solutions.<소문자 code>`(모듈이 다른 키를 고를 수 있다)
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, schemars::JsonSchema)]
#[schemars(deny_unknown_fields)]
pub struct DiagnoseRule {
    /// 문제 코드 (모듈 안에서 겹치지 않음, 예: "PORT_IN_USE")
    pub code: String,
    /// "critical" | "error"(기본) | "warning" | "info"
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub severity: Option<String>,
    /// Python 정규식 (대소문자 무시). 하나라도 맞으면 그 줄이 문제다
    pub patterns: Vec<String>,
}

/// `[[diagnose]] severity`로 받는 값
pub const DIAGNOSE_SEVERITIES: &[&str] = &["critical", "error", "warning", "info"];

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
    /// 서버가 켜져 있을 때 `save_command` 전에 실행해 백업하는 동안 서버의 자동 저장을 멈추는 `[commands.fields]` 이름
    /// (예: "save-off"). 있으면 `resume_command`도 있어야 한다
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub pause_command: Option<String>,
    /// 백업이 끝나면(실패해도) 실행해 자동 저장을 되돌리는 `[commands.fields]` 이름 (예: "save-on")
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub resume_command: Option<String>,
    /// 켜진 서버에서 `save_command`가 디스크에 다 썼다고 알리는 콘솔 줄의 정규식 (예: "Saved the game$").
    /// 있으면 데몬이 고정 대기 대신 이 줄을 기다리고(최대 120초), 나오지 않으면 백업을 만들지 않는다
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub save_done_pattern: Option<String>,
    /// 백업할 폴더 안에서 담지 않을 파일 이름 패턴 (`*` 가능). 서버가 켜져 있는 동안 잠가 두는 파일용 (예: "session.lock")
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub exclude: Vec<String>,
    /// true면 lifecycle `backup_snapshots(config)`가 게임이 스스로 만든 자동 백업(스냅샷)을
    /// `{"success": true, "snapshots": [{"id", "created", "paths": [{"from", "to"}]}]}`(새것부터)로 준다.
    /// 백업 목록에 함께 보이고, 사바쨩 백업으로 보관하거나 바로 복원할 수 있다
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub snapshots: bool,
    /// true면 lifecycle `backup_paths(config)`가 `{"success": true, "paths": [...]}`로 경로를 준다
    /// (세이브 위치를 모듈 코드가 계산하는 게임용). `paths`에 더해진다
    #[serde(default)]
    pub hook: bool,
}

/// 명령어 별명 정보
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct CommandAliasInfo {
    #[serde(default)]
    pub aliases: Vec<String>,
    #[serde(default)]
    pub description: Option<String>,
}

impl ModuleMetadata {
    /// `[protocols] supported`에 이 프로토콜(`rest`·`rcon`·`stdin`)이 있는가
    pub fn supports_protocol(&self, protocol: &str) -> bool {
        self.protocols_supported.as_deref().unwrap_or(&[]).iter().any(|p| p == protocol)
    }

    /// 인스턴스 `protocol_mode`로 쓸 값이 지금 값과 다르면 그 값. 모듈이 `[protocols] supported`를 선언했으면
    /// `auto`·예전 값 `rcon`(RCON만 받는 모듈은 기본값이 `rcon`이라 그대로)·받지 않는 값을 모듈 기본값(없으면 `supported` 첫 값)으로 바꾼다.
    /// 선언하지 않은 모듈은 그대로 둔다
    pub fn protocol_mode_for(&self, current: &str) -> Option<String> {
        let supported = self.protocols_supported.as_deref().filter(|s| !s.is_empty())?;
        if current != "auto" && current != "rcon" && supported.iter().any(|p| p == current) {
            return None;
        }
        let target = self.protocols_default.as_deref().filter(|d| !d.is_empty()).unwrap_or(&supported[0]);
        (target != current).then(|| target.to_string())
    }

    /// 명령을 RCON으로 보내는 모듈이면 그 기본 RCON 포트 (`RconRoute::Only`)
    pub fn rcon_only_port(&self) -> Option<u16> {
        let only = rcon_route(self.protocols_supported.as_deref()?) == RconRoute::Only;
        self.rcon_port.filter(|p| only && *p > 0)
    }

    /// 콘솔 stdin이 없는 인스턴스(관리형 시작을 끔)가 명령을 RCON으로 받는 모듈이면 그 기본 RCON 포트.
    /// RCON만 받는 모듈도 포함한다 (`RconRoute::Only`·`WithoutConsole`)
    pub fn rcon_port_without_console(&self) -> Option<u16> {
        let route = rcon_route(self.protocols_supported.as_deref()?) != RconRoute::Never;
        self.rcon_port.filter(|p| route && *p > 0)
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
    
    /// REST 접속값이 든 설정 (`[protocols.rest]`). REST를 받지 않는 모듈이면 None.
    /// 접속값은 인스턴스 `module_settings`에만 있다: 인스턴스에 따로 두지 않는다
    pub fn rest(&self) -> Option<&RestSettings> {
        self.supports_protocol("rest").then_some(&self.rest)
    }

    /// 데몬이 REST로 붙을 포트: 인스턴스 설정 값, 없으면 그 설정의 모듈 기본값
    pub fn rest_port(&self, instance: &crate::instance::ServerInstance) -> Option<u16> {
        let key = &self.rest()?.port_setting;
        match instance.module_settings.get(key) {
            Some(serde_json::Value::Number(n)) => n.as_u64().and_then(|p| u16::try_from(p).ok()),
            Some(serde_json::Value::String(s)) if !s.trim().is_empty() => s.trim().parse().ok(),
            _ => self.get_setting_default(key),
        }
        .filter(|p| *p > 0)
    }

    /// 데몬이 REST로 붙을 호스트: 인스턴스 설정 값, 없으면 그 설정의 모듈 기본값, 그것도 없으면 이 PC
    pub fn rest_host(&self, instance: &crate::instance::ServerInstance) -> Option<String> {
        let key = &self.rest()?.host_setting;
        let saved = instance.module_settings.get(key).and_then(|v| v.as_str()).map(str::trim).filter(|h| !h.is_empty()).map(str::to_string);
        Some(saved.or_else(|| self.get_setting_default(key)).unwrap_or_else(|| "127.0.0.1".to_string()))
    }

    /// 데몬이 REST로 붙을 곳을 정하는 설정 이름 (호스트·포트). 원격에서는 바꾸지 못한다: 다른 곳으로 돌리면 관리자 비밀번호가 실려 나간다
    pub fn rest_target_settings(&self) -> Vec<&str> {
        self.rest().map(|r| vec![r.host_setting.as_str(), r.port_setting.as_str()]).unwrap_or_default()
    }

    /// 인스턴스 설정에 든 REST 비밀번호 (`password_setting`). 비어 있으면 None
    pub fn rest_password<'a>(&self, instance: &'a crate::instance::ServerInstance) -> Option<&'a str> {
        instance.module_settings.get(self.rest()?.password_setting.as_deref()?).and_then(|v| v.as_str()).filter(|p| !p.is_empty())
    }

    /// 런타임 실행 파일 경로인 설정 `(설정 이름, 런타임 종류)` (설정 필드 `runtime`)
    pub fn runtime_settings(&self) -> Vec<(&str, &str)> {
        self.settings.iter().flat_map(|s| &s.fields).filter_map(|f| Some((f.name.as_str(), f.runtime.as_deref()?))).collect()
    }

    /// 사용자가 직접 정한 런타임 실행 파일 경로. 실행 대상 검사와 격리 실행은 이것만 런타임으로 인정한다:
    /// 모듈 기본값·프리셋·가져온 값처럼 사용자가 정하지 않은 값은 인정하지 않는다
    pub fn user_runtime_paths<'a>(&self, instance: &'a crate::instance::ServerInstance) -> Vec<&'a str> {
        self.runtime_settings()
            .into_iter()
            .filter(|(name, _)| instance.user_settings.contains(*name))
            .filter_map(|(name, _)| instance.module_settings.get(name)?.as_str())
            .collect()
    }

    /// 설정 이름 → 필드 타입 (`number`, `text`, `password`…). 파일에서 읽은 값을 타입대로 저장할 때 쓴다
    pub fn setting_types(&self) -> std::collections::HashMap<String, String> {
        self.settings.iter().flat_map(|s| &s.fields).map(|f| (f.name.clone(), f.field_type.clone())).collect()
    }

    /// 이 OS·CPU에서 돌아가지 않는 이유: `"os"`(`platforms`) 또는 `"arch"`(`architectures`). 돌아가면 None
    pub fn unsupported_on(&self, os: &str, arch: &str) -> Option<&'static str> {
        module_support::unsupported_reason(self.platforms.as_deref(), self.architectures.as_deref(), os, arch)
    }

    /// 컴포넌트 버전 의존성을 확인합니다.
    /// `installed_versions`: 컴포넌트 키 → 설치된 버전 (예: "saba-core" → "0.3.0")
    /// 미충족 의존성 목록을 (컴포넌트키, 요구버전, 설치버전Option) 튜플로 반환합니다.
    pub fn check_dependencies(
        &self,
        installed_versions: &std::collections::HashMap<String, String>,
    ) -> Vec<(String, String, Option<String>)> {
        saba_chan_updater_lib::unmet_requires(&self.name, &self.dependencies, installed_versions)
            .into_iter()
            .map(|issue| (issue.required_component, issue.required_version, issue.installed_version))
            .collect()
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
    /// 콘솔 답이 이 정규식과 맞으면 명령이 실패한 것이다 (`reason` 그룹이 있으면 그것이 이유). 채팅 답이 쓴다
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub failure_patterns: Vec<String>,
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
    /// false면 디스코드·챗봇에 내보내지 않는다 (데몬 기능이 같은 일을 하는 명령. 예: 접속자 목록은 `players`)
    #[serde(default = "yes", skip_serializing_if = "Clone::clone")]
    pub remote: bool,
    /// 콘솔 답에서 값을 뽑는 정규식 (이름 있는 그룹). 맞으면 채팅 답이 모듈 locales `commands.<이름>.done`에 그 값을 넣는다
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub reply_pattern: Option<String>,
}

fn yes() -> bool {
    true
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
    /// 친구가 밖에서 붙는 포트면 그 프로토콜: "tcp" | "udp" | "both". 포트포워딩 확인이 이 포트만 본다 (`crate::port_check`)
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub public: Option<String>,
    /// 서버를 만들 때 고르는 설정 (받을 서버 파일이 달라지는 값. 예: 마인크래프트 서버 종류).
    /// GUI·CLI가 만들기 화면에서 묻고 `POST /api/instances`의 `module_settings`로 보낸다
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub install: bool,
    /// 새 서버를 만들 때 고를 수 있는 `options` (사바쨩이 설치할 수 있는 값). 없으면 `options` 전부.
    /// 나머지 값은 가져온 서버를 판정할 때만 쓴다
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub install_options: Option<Vec<String>>,
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
    /// 이 설정이 게임 설정 파일에서 쓰는 키 (예: `max-players`). 데몬은 읽지 않는다: 공용 도우미
    /// `daemon_rcon.settings_to_file`·`settings_from_file`이 이름과 파일 키를 이 한 곳에서 잇는다 (saba-core 0.1.11)
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub file_key: Option<String>,
    /// 이 설정이 있는 게임 버전 범위 (예: `<1.21.9`, `>=1.13 <1.20`). 설치된 버전(`server_version`)이 밖이면
    /// 그 서버에는 없는 설정이라 화면이 숨긴다 (`crate::validator::version_hidden`). 없으면 모든 버전
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub versions: Option<String>,
    /// 이 설정 값이 서버를 띄우는 런타임 실행 파일의 경로면 그 종류 (`crate::sandbox::RUNTIME_KINDS`의 id, 예: `java`).
    /// 사용자가 정한 값만 실행 대상 검사가 런타임으로 인정하고 격리 실행에 읽기로 연다. 원격에서는 바꿀 수 없다.
    /// 데몬은 이 종류를 선언한 모듈의 호출에만 찾은 런타임 목록(환경 변수)을 넘긴다
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub runtime: Option<String>,
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
pub(crate) enum OneOrMany {
    One(String),
    Many(Vec<String>),
}

impl OneOrMany {
    pub(crate) fn into_vec(self) -> Vec<String> {
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
pub(crate) const NOT_A_CONDITION: &[&str] = &["list", "multiline", "password", "file"];
/// 명령 입력 `inputs[].type`으로 받는 값
pub const COMMAND_INPUT_TYPES: &[&str] = &["text", "password", "number", "select", "boolean"];

/// 인스턴스 필드로도 저장되는 포트 설정 이름 (`ServerInstance::port`)
pub const CORE_PORT_FIELDS: &[&str] = &["port"];

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
    /// 찾은 원본: 모듈 폴더, 또는 modules 폴더에 놓인 ZIP 파일 (제거 API가 지우는 것)
    pub source: String,
}

impl LoadedModule {
    /// 모듈 `[install] requires_extensions`
    pub(crate) fn required_extensions(&self) -> Vec<String> {
        self.metadata.install.as_ref().map(|i| i.requires_extensions.clone()).unwrap_or_default()
    }

    /// 데몬이 실행할 Python 진입 파일 (`[module] entry`)
    pub fn entry_path(&self) -> String {
        Path::new(&self.path).join(&self.metadata.entry).to_string_lossy().to_string()
    }

    /// 데몬이 이 모듈을 쓸 수 있는가: 진입 파일이 있고, `[module.dependencies] saba-core`를 만족한다 (M9)
    pub(crate) fn check_loadable(&self) -> Result<()> {
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
    /// 데몬이 모자라면 요구하는 saba-core 버전 (`>=0.3.0` → `0.3.0`, 0.1.14-beta.2~). 클라이언트는 위 문구를 쪼개지 않고 이것을 읽는다
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub required_core: Option<String>,
    /// `module_schema_invalid`일 때 무엇이 틀렸는지 (필드 이름과 이유, 경로 없음)
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub problems: Vec<String>,
    /// `module_schema_unsupported`일 때 모듈이 요구한 스키마 버전
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub required: Option<u32>,
}

impl ModuleLoadError {
    pub(crate) fn new(path: &Path, e: &anyhow::Error) -> Self {
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
            required_core: unmet.map(|u| u.required.trim().trim_start_matches(">=").trim().to_string()),
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
// 모르는 키: 로드는 하고 경고만 남긴다 (JSON Schema는 모듈을 쓰는 쪽을 위해 모르는 키를 막는다).

/// 이 데몬이 아는 가장 새 module.toml 스키마 버전. 1과 2를 받는다 (2는 1에 06 §8의 설정 타입·키를 더한 것)
pub const MODULE_SCHEMA_VERSION: u32 = 2;
/// 스키마 표시가 없는 모듈에게 권하는 버전. 0.1.8 데몬도 읽는 1이다 (06 §7)
pub(crate) const BASE_SCHEMA_VERSION: u32 = 1;

/// 이 데몬이 받는 module.toml 스키마 버전들 (모듈 메이커 안내 `GET /api/maker`)
pub fn supported_schema_versions() -> [u32; 2] {
    [BASE_SCHEMA_VERSION, MODULE_SCHEMA_VERSION]
}

/// 모듈이 명령을 RCON으로 받는 때. 명령 경로 우선순위는 STDIN → REST → RCON이라, REST가 있으면 RCON을 쓰지 않는다
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RconRoute {
    /// RCON이 명령 길이 아니다 (rcon이 없거나 rest가 있다)
    Never,
    /// RCON만 받는다 (stdin·rest 없음)
    Only,
    /// 콘솔 stdin이 없는 인스턴스(관리형 시작을 끔)만 RCON으로 받는다 (stdin과 함께, rest 없음)
    WithoutConsole,
}

/// `[protocols] supported`로 정한 RCON 명령 길
pub fn rcon_route(supported: &[String]) -> RconRoute {
    let has = |p: &str| supported.iter().any(|s| s == p);
    match (has("rcon") && !has("rest"), has("stdin")) {
        (false, _) => RconRoute::Never,
        (true, false) => RconRoute::Only,
        (true, true) => RconRoute::WithoutConsole,
    }
}

/// 명령 위험도로 쓸 수 있는 값
/// 모듈 `[module.dependencies]`에서 데몬을 가리키는 키
pub const CORE_COMPONENT: &str = "saba-core";
/// 이 데몬의 버전 (모듈 호환 검사 기준)
pub const DAEMON_VERSION: &str = env!("CARGO_PKG_VERSION");
pub const COMMAND_RISKS: [&str; 3] = ["safe", "caution", "danger"];
/// `[protocols] supported`·`default`가 받는 값
pub const PROTOCOLS: [&str; 3] = ["rcon", "rest", "stdin"];

/// JSON 스키마에서도 로더와 같은 값만 받게 한다 (적합성 검사가 데몬과 어긋나지 않게)
pub(crate) fn schema_version_schema(_: &mut schemars::SchemaGenerator) -> schemars::Schema {
    serde_json::from_value(serde_json::json!({
        "description": "이 파일이 따르는 스키마 버전. 1, 또는 list·multiline 설정과 restart_required·visible_when 등을 쓰면 2(saba-core >=0.1.9)",
        "enum": [BASE_SCHEMA_VERSION, MODULE_SCHEMA_VERSION, null]
    }))
    .expect("static schema")
}

pub(crate) fn risk_schema(_: &mut schemars::SchemaGenerator) -> schemars::Schema {
    serde_json::from_value(serde_json::json!({
        "description": "\"safe\" | \"caution\" | \"danger\" (없으면 caution)",
        "enum": [COMMAND_RISKS[0], COMMAND_RISKS[1], COMMAND_RISKS[2], null]
    }))
    .expect("static schema")
}
