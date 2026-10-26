//! 데몬 액션 카탈로그 (P2): 데몬이 할 수 있는 일을 한 곳에 선언하고 모든 표면(GUI·CLI·디스코드·챗봇)이 읽는다
//!
//! 액션은 기존 HTTP 라우트를 데몬 안에서 부르는 얇은 층이다 (`IPCServer::router`를 직접 부른다. 새 로직은 없다).
//! - 목록: [`list`]. 인스턴스를 주면 모듈·익스텐션 명령이 `server.command:<이름>`으로, 모듈 `[[queries]]`가
//!   `server.query:<이름>`(읽기 전용, 모든 표면)으로 펼쳐진다.
//! - 계획: [`plan`]은 입력을 검증하고 요약만 만든다(부작용 없음). planId는 요청자·길드·via에 묶이고 10분 산다.
//! - 실행: [`run`]은 밑 라우트를 불러 `{success, result}`로 감싼다.
//! - 여러 단계 계획(I1): [`plan_steps`]가 단계마다 검증·판정해 planId 하나로 묶는다 (`steps.rs`).
//! - 원격 설정 변경은 바꾸기 전 값을 요약에 싣고, 실행 뒤 되돌리는 계획(`undoPlanId`)을 하나 더 만든다.
//! - 원격 관리 실행은 길드당 시간당 횟수를 넘으면 `rate_limited` (`nodeSettings[guild].adminRunsPerHour`, 기본 10).
//! - 표면: `surfaces`는 코드 상수다. 챗봇 경로는 여기 없는 액션을 레벨 판정 전에 거부한다(설정으로 풀 수 없다).
//!   릴레이에서 온 요청만 P1 레벨을 본다(`crate::chat::permissions`). GUI·CLI는 판정하지 않는다.

use crate::chat::permissions::{NodePermissions, Requester};
use crate::ipc::internal::{into_internal, InternalResponse};
use crate::ipc::IPCServer;
use crate::supervisor::module_loader::{LoadedModule, ModuleQuery};
use serde_json::{json, Map, Value};
use std::collections::HashMap;
use std::sync::Mutex;
use std::time::{Duration, Instant};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Surface {
    Gui,
    Cli,
    Discord,
    Chatbot,
}

impl Surface {
    pub fn name(self) -> &'static str {
        match self {
            Surface::Gui => "gui",
            Surface::Cli => "cli",
            Surface::Discord => "discord",
            Surface::Chatbot => "chatbot",
        }
    }

    pub fn parse(s: &str) -> Option<Self> {
        [Surface::Gui, Surface::Cli, Surface::Discord, Surface::Chatbot].into_iter().find(|v| v.name() == s)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Kind {
    Query,
    Command,
    Admin,
}

impl Kind {
    fn name(self) -> &'static str {
        match self {
            Kind::Query => "query",
            Kind::Command => "command",
            Kind::Admin => "admin",
        }
    }
}

/// 입력 필드 하나
#[derive(Debug)]
pub struct Field {
    pub name: &'static str,
    /// JSON Schema 타입
    pub ty: &'static str,
    pub required: bool,
    /// 경로의 `:<name>` 자리에 들어간다 (본문에는 넣지 않는다)
    pub in_path: bool,
    /// 다른 조회 액션의 결과에서 고르는 값 (스키마의 `optionsFrom`)
    pub options_from: Option<OptionsFrom>,
}

/// 입력 칸의 값을 고를 목록: 같은 인스턴스에서 `action`을 부른 결과의 `list` 배열, 항목의 `value` 키가 값이고 `label` 키가 보일 이름
#[derive(Debug)]
pub struct OptionsFrom {
    pub action: &'static str,
    pub list: &'static str,
    pub value: &'static str,
    pub label: &'static str,
}

const fn field(name: &'static str, ty: &'static str, required: bool) -> Field {
    Field { name, ty, required, in_path: false, options_from: None }
}

const fn path_field(name: &'static str) -> Field {
    Field { name, ty: "string", required: true, in_path: true, options_from: None }
}

/// 액션 선언
#[derive(Debug)]
pub struct Def {
    pub id: &'static str,
    pub kind: Kind,
    pub risk: &'static str,
    pub surfaces: &'static [Surface],
    /// 인스턴스를 받는가 (경로의 `:id`)
    pub instance: bool,
    pub method: &'static str,
    pub path: &'static str,
    pub fields: &'static [Field],
    /// true면 입력 객체를 그대로 본문으로 보낸다 (설정 바꾸기처럼 키가 정해져 있지 않은 것)
    pub open_input: bool,
    /// 디스코드에서 고정 요구 레벨 (프리셋·덮어쓰기와 상관없이)
    pub discord_level: Option<i64>,
    /// 디스코드 작업 메뉴에 싣는가 (heartbeat `surfaces`에 discord를 넣는가). 전용 명령이 같은 액션을 부르는
    /// 것(길드 설정은 "/사바쨩")과 메뉴에서 뺀 것은 false다. 챗봇 목록에는 그대로 남고, 액션 자체도 디스코드에서 계속 부를 수 있다
    pub remote_menu: bool,
    /// 디스코드 서버 주인도 `discord_level` 주인으로 본다 (길드 공통 설정, 원격 데몬 R2b). 챗봇 경로는 아니다
    pub guild_owner: bool,
}

use Surface::{Chatbot, Cli, Discord, Gui};
const ALL: &[Surface] = &[Gui, Cli, Discord, Chatbot];
const LOCAL: &[Surface] = &[Gui, Cli];
const LOCAL_AND_CHATBOT: &[Surface] = &[Gui, Cli, Chatbot];
const LOCAL_AND_DISCORD: &[Surface] = &[Gui, Cli, Discord];

const fn def(id: &'static str, kind: Kind, risk: &'static str, surfaces: &'static [Surface], instance: bool, method: &'static str, path: &'static str) -> Def {
    Def { id, kind, risk, surfaces, instance, method, path, fields: &[], open_input: false, discord_level: None, remote_menu: true, guild_owner: false }
}

/// 첫 판 액션. 새 기능은 여기 한 줄을 더하면 모든 표면에 퍼진다.
/// 챗봇 범위(사용자 결정): 관리 액션은 설치·생성·설정·예약·복원만. 삭제·종료·업데이트·권한·비밀값은 열지 않는다
pub static ACTIONS: &[Def] = &[
    def("instance.list", Kind::Query, "safe", ALL, false, "GET", "/api/servers"),
    def("server.status", Kind::Query, "safe", ALL, true, "GET", "/api/server/:iname/status"),
    def("server.start", Kind::Command, "caution", ALL, true, "POST", "/api/instance/:id/start"),
    def("server.stop", Kind::Command, "caution", ALL, true, "POST", "/api/instance/:id/stop"),
    def("server.restart", Kind::Command, "caution", ALL, true, "POST", "/api/instance/:id/restart"),
    def("server.console.tail", Kind::Query, "safe", LOCAL_AND_CHATBOT, true, "GET", "/api/instance/:id/console?count=20"),
    // 접속자 이름까지 (검수 13번). 모듈이 `[status] players`를 선언한 서버만 (heartbeat `modules`, 인스턴스 목록)
    def("server.players", Kind::Query, "safe", ALL, true, "GET", "/api/instance/:id/players"),
    // 포트포워딩 확인: 릴레이가 밖에서 게임 포트에 접속해 본다. 공인 주소가 결과에 있어 챗봇에는 열지 않는다 (디스코드는 요약만 보인다)
    def("server.port_check", Kind::Query, "safe", LOCAL_AND_DISCORD, true, "POST", "/api/instance/:id/port-check"),
    // 시작·정지 기록은 디스코드에서 쓰지 않는다 (사용자 디스코드 검수 3번)
    def("instance.events", Kind::Query, "safe", LOCAL_AND_CHATBOT, true, "GET", "/api/instance/:id/events"),
    // 로그 전체 읽기·내보내기 (검수 9번). 디스코드 메뉴에는 없고 내보내기는 파일로 첨부된다
    Def {
        fields: &[field("source", "string", false), field("lines", "integer", false), field("before", "integer", false)],
        ..def("server.logs.read", Kind::Query, "safe", LOCAL_AND_CHATBOT, true, "GET", "/api/instance/:id/logs")
    },
    Def {
        fields: &[field("source", "string", false), field("since", "integer", false), field("until", "integer", false)],
        ..def("server.logs.export", Kind::Query, "safe", LOCAL_AND_CHATBOT, true, "GET", "/api/instance/:id/logs/export")
    },
    def("monitor.summary", Kind::Query, "safe", ALL, false, "GET", "/api/system/metrics?sizes=false"),
    // 디스코드 메뉴에서만 뺀다 (검수 4번). 챗봇 복원이 목록을 보고, 디스코드에서 부르는 것은 계속 받는다
    Def { remote_menu: false, ..def("backup.list", Kind::Query, "safe", ALL, true, "GET", "/api/instance/:id/backups") },
    // from_snapshot: 지금 상태 대신 게임이 만든 가장 새 자동 백업을 보관한다 (`[backup] snapshots` 모듈)
    Def { fields: &[field("from_snapshot", "boolean", false)], ..def("backup.create", Kind::Command, "caution", ALL, true, "POST", "/api/instance/:id/backups") },
    // 디스코드에서도 복원한다 (검수 14번). 백업은 목록에서 고른다
    Def {
        fields: &[Field { options_from: Some(OptionsFrom { action: "backup.list", list: "backups", value: "name", label: "label" }), ..path_field("name") }],
        ..def("backup.restore", Kind::Admin, "danger", ALL, true, "POST", "/api/instance/:id/backups/:name/restore")
    },
    Def {
        fields: &[field("name", "string", true), field("module_name", "string", true), field("preset", "string", false), field("module_settings", "object", false)],
        ..def("instance.create", Kind::Admin, "caution", LOCAL_AND_CHATBOT, false, "POST", "/api/instances")
    },
    Def { open_input: true, ..def("instance.settings.set", Kind::Admin, "caution", LOCAL_AND_CHATBOT, true, "PATCH", "/api/instance/:id") },
    Def {
        fields: &[field("schedule", "array", true), field("backup_keep", "integer", false)],
        ..def("instance.schedule.set", Kind::Admin, "caution", LOCAL_AND_CHATBOT, true, "PUT", "/api/instance/:id/schedule")
    },
    def("instance.delete", Kind::Admin, "danger", LOCAL, true, "DELETE", "/api/instance/:id"),
    Def { fields: &[path_field("module")], ..def("module.install", Kind::Admin, "caution", LOCAL_AND_CHATBOT, false, "POST", "/api/modules/manifest/:module/install") },
    Def { fields: &[path_field("extension")], ..def("extension.enable", Kind::Admin, "caution", LOCAL, false, "POST", "/api/extensions/:extension/enable") },
    Def { fields: &[path_field("extension")], ..def("extension.disable", Kind::Admin, "caution", LOCAL, false, "POST", "/api/extensions/:extension/disable") },
    // 챗봇 설정 (I2-b). 디스코드에서는 소유자만, 그 길드 것만. 키는 받지 않고 챗봇 자신은 못 바꾼다(확인을 끄지 못하게)
    Def { fields: &[path_field("guild")], discord_level: Some(crate::chat::permissions::OWNER), remote_menu: false, ..def("chatbot.settings.get", Kind::Query, "safe", LOCAL_AND_DISCORD, false, "GET", "/api/chatbot/guilds/:guild") },
    Def {
        fields: &[
            path_field("guild"),
            field("enabled", "boolean", false),
            field("model", "string", false),
            field("memo", "string", false),
            field("confirmCommands", "boolean", false),
            field("memory", "boolean", false),
            field("webSearch", "boolean", false),
            field("disabledTools", "array", false),
        ],
        discord_level: Some(crate::chat::permissions::OWNER),
        remote_menu: false,
        ..def("chatbot.settings.set", Kind::Admin, "caution", LOCAL_AND_DISCORD, false, "PATCH", "/api/chatbot/guilds/:guild")
    },
    // 디스코드 길드 설정 (I2). 원격에서는 요청이 온 길드만 (`guild`를 그 길드로 고정한다)
    Def {
        fields: &[path_field("guild"), field("locale", "string", false), field("sleeping", "boolean", false), field("music", "object", false)],
        discord_level: Some(crate::chat::permissions::OWNER),
        remote_menu: false,
        guild_owner: true,
        ..def("discord.settings.set", Kind::Admin, "caution", LOCAL_AND_DISCORD, false, "PATCH", "/api/discord/:guild/settings")
    },
    // 켜짐 알림 (길드마다 서버마다, 기본 꺼짐). 디스코드에서는 관리자 이상, 요청이 온 길드만. 서버 루트 메뉴(`/<서버> 켜짐-알림`)에 싣는다
    Def {
        fields: &[path_field("guild"), field("enabled", "boolean", true)],
        discord_level: Some(crate::chat::permissions::ADMIN),
        guild_owner: true,
        ..def("instance.start_notice.set", Kind::Admin, "caution", LOCAL_AND_DISCORD, true, "PUT", "/api/instance/:id/start-notice/:guild")
    },
    def("update.check", Kind::Query, "safe", LOCAL_AND_DISCORD, false, "POST", "/api/updates/check"),
    // 원격(디스코드)에서도 끝까지 가도록 받기·적용·업데이터 띄우기를 한 번에 한다 (`/api/updates/install`)
    Def { discord_level: Some(crate::chat::permissions::OWNER), ..def("update.apply", Kind::Admin, "danger", LOCAL_AND_DISCORD, false, "POST", "/api/updates/install") },
];

/// 길드를 받는 액션의 경로 필드 (원격이면 요청이 온 길드로 고정)
const GUILD_FIELD: &str = "guild";

/// 모듈·익스텐션 명령 액션의 접두어 (`server.command:<이름>`)
pub const COMMAND_PREFIX: &str = "server.command:";

/// 모듈 조회 액션의 접두어 (`server.query:<이름>`, module.toml `[[queries]]`)
pub const QUERY_PREFIX: &str = "server.query:";

/// 챗봇에 넣지 않는 설정 키 (비밀값). 모듈 설정의 password 형식 필드도 뺀다
const SECRET_KEYS: &[&str] = &["rest_password", "rcon_password", "password", "token", "secret"];

pub fn find(id: &str) -> Option<&'static Def> {
    ACTIONS.iter().find(|d| d.id == id)
}

/// 관리 액션의 요구 레벨 (프리셋 열 `admin`): 느슨 2, 기본 2, 엄격 3
pub fn admin_level(preset: &str) -> i64 {
    use crate::chat::permissions::{ADMIN, OWNER};
    if preset == "strict" {
        OWNER
    } else {
        ADMIN
    }
}

pub(crate) fn is_secret_key(key: &str, password_fields: &[String]) -> bool {
    let lower = key.to_ascii_lowercase();
    password_fields.iter().any(|f| f == key) || SECRET_KEYS.iter().any(|s| lower == *s || lower.ends_with(&format!("_{}", s)))
}

/// 원격(디스코드·챗봇)에서 바꿀 수 없는 인스턴스 키: 실행 파일·폴더·인자·메모리 상한과 그 밖으로 새는 연결 대상.
/// `java_path`는 모듈이 어떤 형식으로 선언하든 실행 대상 검사가 인정하는 런타임이라 늘 막는다
const LAUNCH_KEYS: &[&str] = &["executable_path", "working_dir", "instance_dir", "extra_args", "extension_data", "rest_host", "memory_limit_mb", "java_path"];

/// 모듈 설정 필드 중 원격에서 다루지 않는 것들
#[derive(Debug, Default)]
struct FieldRules {
    /// password 형식
    secret: Vec<String>,
    /// 실행 대상·인자가 되는 값: file 형식, path 형식, 형식·정규식 없이 실행 인자로 들어가는 글자 값
    launch: Vec<String>,
}

fn field_rules(module: &crate::supervisor::module_loader::LoadedModule) -> FieldRules {
    let mut rules = FieldRules::default();
    for f in module.metadata.settings.iter().flat_map(|s| s.fields.iter()) {
        if f.field_type == "password" {
            rules.secret.push(f.name.clone());
        }
        let free_text_arg = !f.arg.is_empty() && f.field_type == "text" && f.format.is_none() && f.pattern.is_none();
        if f.field_type == "file" || f.format.as_deref() == Some("path") || free_text_arg {
            rules.launch.push(f.name.clone());
        }
    }
    rules
}

/// 원격 입력에서 비밀값·실행 관련 키를 어느 깊이에서든 찾는다
fn forbidden_remote_key(v: &Value, rules: &FieldRules) -> Option<String> {
    match v {
        Value::Object(m) => m.iter().find_map(|(k, v)| {
            if is_secret_key(k, &rules.secret) || LAUNCH_KEYS.contains(&k.as_str()) || rules.launch.iter().any(|f| f == k) {
                Some(k.clone())
            } else {
                forbidden_remote_key(v, rules)
            }
        }),
        Value::Array(a) => a.iter().find_map(|x| forbidden_remote_key(x, rules)),
        _ => None,
    }
}

/// 원격으로 나가는 결과에서 비밀값을 어느 깊이에서든 지운다
fn strip_secrets(v: &mut Value, password_fields: &[String]) {
    match v {
        Value::Object(m) => {
            m.retain(|k, _| !is_secret_key(k, password_fields));
            m.values_mut().for_each(|x| strip_secrets(x, password_fields));
        }
        Value::Array(a) => a.iter_mut().for_each(|x| strip_secrets(x, password_fields)),
        _ => {}
    }
}

/// 원격으로 나가는 결과에서 PC의 로컬 정보를 어느 깊이에서든 뺀다 (I3, 알림 push 경로와 같은 기준):
/// 실행 파일·폴더·인자 키, 콘솔 줄이 든 종료 근거, 자동 재시작 실패 원문, 그리고 문자열 안의 절대 경로
pub(crate) fn strip_local_details(v: &mut Value) {
    const LOCAL_KEYS: [&str; 6] = ["working_dir", "executable_path", "extra_args", "extension_data", "install_path", "stderr_tail"];
    if v.get("success").and_then(Value::as_bool) == Some(false) && v.get("module").is_some() && v.get("function").is_some() {
        for (key, value) in v.as_object_mut().into_iter().flatten() {
            if !matches!(key.as_str(), "module" | "function" | "error_code" | "phase") {
                mask_console_with_paths(value, true);
            }
        }
    }
    match v {
        Value::Object(m) => {
            m.retain(|k, _| !LOCAL_KEYS.contains(&k.as_str()));
            // 파일로 첨부되는 결과(로그 내보내기)는 줄마다 가린다: 통째로 보면 traceback 규칙이 내용을 한 줄로 줄인다
            let file = m.contains_key("fileName").then(|| m.remove("content")).flatten();
            if m.get("evidence").and_then(Value::as_str).is_some_and(|e| !crate::supervisor::crash::shareable_evidence(e)) {
                m.remove("evidence");
            }
            if let Some(args) = m.get_mut("note_args").and_then(Value::as_object_mut) {
                args.remove("reason");
            }
            m.values_mut().for_each(strip_local_details);
            if let Some(content) = file {
                let masked = match content {
                    Value::String(text) => Value::String(text.split('\n').map(|line| mask_local_text(line, false).unwrap_or_else(|| line.to_string())).collect::<Vec<_>>().join("\n")),
                    other => other,
                };
                if let (Some(text), true) = (masked.as_str(), m.contains_key("bytes")) {
                    m.insert("bytes".into(), text.len().into());
                }
                m.insert("content".into(), masked);
            }
        }
        Value::Array(a) => a.iter_mut().for_each(strip_local_details),
        Value::String(s) => {
            if let Some(masked) = mask_local_text(s, false) {
                *s = masked;
            }
        }
        _ => {}
    }
}

/// 전역 권한 덮어쓰기 키로 쓸 수 있는 액션인가: 원격(디스코드·챗봇)에서 부를 수 있고 레벨이 코드에 고정되지 않은 것.
/// 인스턴스 액션은 `<instance>/<action id>`로만 판정하므로(`authorize`) 전역 키를 받지 않는다
pub fn overridable(id: &str) -> bool {
    find(id).is_some_and(|d| !d.instance && d.discord_level.is_none() && d.surfaces.iter().any(|s| matches!(s, Surface::Discord | Surface::Chatbot)))
}

/// 문자열의 절대 경로를 `<path>`로 바꾸고, Python traceback은 마지막 줄(예외 문구)만 남긴다. 바뀐 것이 없으면 None
fn mask_local_text(text: &str, single_component: bool) -> Option<String> {
    static QUOTED: std::sync::OnceLock<regex::Regex> = std::sync::OnceLock::new();
    static PATHS: std::sync::OnceLock<regex::Regex> = std::sync::OnceLock::new();
    // 따옴표로 시작하는 경로는 닫는 따옴표까지 (공백 포함)
    let quoted = QUOTED.get_or_init(|| {
        regex::Regex::new(r#""(?:\\\\|[A-Za-z]:[\\/]|/[^\s/"]+/)[^"\r\n]*"|'(?:\\\\|[A-Za-z]:[\\/]|/[^\s/']+/)[^'\r\n]*'"#).unwrap()
    });
    // Windows 드라이브·UNC 경로, 그리고 둘 이상의 조각으로 된 Unix 절대·홈 경로 (URL의 `://`는 `/` 다음이 `/`라 걸리지 않는다).
    // 경로에 공백이 있을 수 있어 그 줄의 끝(또는 따옴표)까지 가린다
    let paths = PATHS.get_or_init(|| {
        regex::Regex::new(r#"\\\\[^\\\s"'<>|]+\\[^\r\n"'<>|]*|\b[A-Za-z]:[\\/][^\r\n"'<>|]*|(?P<pre>^|[\s"'(=:\[])~?/(?:[^\s/"'<>|]+/)+[^\r\n"'<>|]*"#).unwrap()
    });
    let base = if text.contains("Traceback (most recent call last)") {
        text.lines().rev().map(str::trim).find(|l| !l.is_empty()).unwrap_or("")
    } else {
        text
    };
    let base = quoted.replace_all(base, |c: &regex::Captures| {
        let q = &c[0][..1];
        format!("{q}<path>{q}")
    });
    let mut masked = paths.replace_all(&base, "${pre}<path>").into_owned();
    if single_component {
        static SINGLE: std::sync::OnceLock<regex::Regex> = std::sync::OnceLock::new();
        let paths = SINGLE.get_or_init(|| regex::Regex::new(r#"(?P<pre>^|[\s\"'(=:\[])~?/[^\s/\"'<>|][^\r\n\"'<>|]*"#).unwrap());
        masked = paths.replace_all(&masked, "${pre}<path>").into_owned();
    }
    (masked != text).then_some(masked)
}

/// 이 길드에 열지 않은 인스턴스를 결과의 `servers`·`instances` 목록(어느 깊이든, 원소의 `id`)에서 뺀다
fn retain_allowed(v: &mut Value, allowed: &[String]) {
    match v {
        Value::Object(m) => {
            for (k, x) in m.iter_mut() {
                if let (true, Some(items)) = (k == "servers" || k == "instances", x.as_array_mut()) {
                    items.retain(|s| s.get("id").and_then(Value::as_str).is_none_or(|id| allowed.iter().any(|a| a == id)));
                }
                retain_allowed(x, allowed);
            }
        }
        Value::Array(a) => a.iter_mut().for_each(|x| retain_allowed(x, allowed)),
        _ => {}
    }
}

// ── 목록 ────────────────────────────────────────────────

/// 언어 카탈로그 (`locales/<lang>/bot.json`의 `actions`)
fn catalog(locale: &str) -> crate::chat::i18n::Catalog {
    crate::chat::i18n::Catalog::for_language(locale)
}

fn translated(cat: &crate::chat::i18n::Catalog, key: &str) -> Option<String> {
    let text = cat.t(key, &[]);
    (text != key).then_some(text)
}

fn schema(def: &Def) -> Value {
    if def.open_input {
        return json!({ "type": "object", "additionalProperties": true });
    }
    let props: Map<String, Value> = def
        .fields
        .iter()
        .map(|f| {
            let mut prop = json!({ "type": f.ty });
            if let Some(o) = &f.options_from {
                prop["optionsFrom"] = json!({ "action": o.action, "list": o.list, "value": o.value, "label": o.label });
            }
            (f.name.to_string(), prop)
        })
        .collect();
    let required: Vec<&str> = def.fields.iter().filter(|f| f.required).map(|f| f.name).collect();
    json!({ "type": "object", "properties": props, "required": required })
}

/// 원격 요청의 입력: 길드 칸은 요청이 온 길드로 데몬이 채우므로(`plan`) 묻지 않는다
fn remote_schema(def: &Def) -> Value {
    let mut input = schema(def);
    if let Some(props) = input["properties"].as_object_mut() {
        props.remove(GUILD_FIELD);
    }
    if let Some(required) = input["required"].as_array_mut() {
        required.retain(|f| f != GUILD_FIELD);
    }
    input
}

/// 같은 일을 하는 CLI 한 줄 (I4-c "CLI로 보기"). `<server>`·`<필드>` 자리는 화면이 채운다
/// 배포되는 CLI 실행 파일 이름
const CLI_BIN: &str = "saba-chan-cli";

/// 필수 입력마다 ` 이름=<이름>`
fn cli_args(required: &[&str]) -> String {
    required.iter().map(|n| format!(" {}=<{}>", n, n)).collect()
}

fn cli_line(def: &Def) -> String {
    let mut line = format!("{} do {}", CLI_BIN, def.id);
    if def.instance {
        line.push_str(" --server <server>");
    }
    let required: Vec<&str> = def.fields.iter().filter(|f| f.required).map(|f| f.name).collect();
    line + &cli_args(&required)
}

fn view(def: &Def, cat: &crate::chat::i18n::Catalog) -> Value {
    let title_key = format!("actions.{}.title", def.id);
    let description = translated(cat, &format!("actions.{}.description", def.id));
    json!({
        "id": def.id,
        "kind": def.kind.name(),
        "risk": def.risk,
        "surfaces": def.surfaces.iter().map(|s| s.name()).collect::<Vec<_>>(),
        "scope": if def.instance { "instance" } else { "none" },
        "input": schema(def),
        "titleKey": title_key,
        "title": translated(cat, &title_key).unwrap_or_else(|| def.id.to_string()),
        "description": description,
        "route": format!("{} {}", def.method, def.path.split('?').next().unwrap_or(def.path)),
        "cli": cli_line(def),
    })
}

/// 모듈 명령 한 개 (`GET /api/instance/:id/commands`의 항목) → 액션
fn command_view(cmd: &Value) -> Option<Value> {
    let name = cmd.get("name")?.as_str()?;
    let risk = crate::commands::risk_of(cmd.get("risk").and_then(Value::as_str));
    // danger 명령도 챗봇이 고를 수 있다: 늘 확인 계획을 거치고 요청자가 확인한다 (`needs_confirm`).
    // `remote = false` 명령은 이 PC 화면에서만
    let surfaces = if cmd.get("remote").and_then(Value::as_bool) == Some(false) { vec!["gui", "cli"] } else { vec!["gui", "cli", "discord", "chatbot"] };
    let inputs = cmd.get("inputs").cloned().unwrap_or(json!([]));
    let props: Map<String, Value> = inputs
        .as_array()
        .into_iter()
        .flatten()
        .filter_map(|i| Some((i.get("name")?.as_str()?.to_string(), json!({ "type": input_type(i) }))))
        .collect();
    let required: Vec<&str> = inputs
        .as_array()
        .into_iter()
        .flatten()
        .filter(|i| i.get("required").and_then(Value::as_bool) == Some(true))
        .filter_map(|i| i.get("name")?.as_str())
        .collect();
    Some(json!({
        "id": format!("{}{}", COMMAND_PREFIX, name),
        "kind": "command",
        "risk": risk,
        "surfaces": surfaces,
        "scope": "instance",
        "input": { "type": "object", "properties": props, "required": required },
        "titleKey": Value::Null,
        "title": cmd.get("label").and_then(Value::as_str).unwrap_or(name),
        "description": cmd.get("description"),
        "source": cmd.get("source"),
        "route": "POST /api/instance/:id/command",
        "cli": format!("{} do {}{} --server <server>{}", CLI_BIN, COMMAND_PREFIX, name, cli_args(&required)),
    }))
}

fn input_type(input: &Value) -> &'static str {
    match input.get("type").and_then(Value::as_str) {
        Some("number") | Some("integer") => "number",
        Some("boolean") => "boolean",
        _ => "string",
    }
}

/// 모듈 locales 한 언어 (`<모듈>/locales/<lang>.json`). 없거나 못 읽으면 Null
pub(crate) fn module_locale(module: &LoadedModule, lang: &str) -> Value {
    let path = std::path::Path::new(&module.path).join("locales").join(format!("{}.json", lang));
    std::fs::read_to_string(path).ok().and_then(|t| serde_json::from_str(&t).ok()).unwrap_or(Value::Null)
}

/// 조회 문구: 모듈 locales `queries.<name>.<key>` → module.toml 값
fn query_text(locale: &Value, query: &ModuleQuery, key: &str) -> Option<String> {
    let declared = if key == "label" { &query.label } else { &query.description };
    locale.pointer(&format!("/queries/{}/{}", query.name, key)).and_then(Value::as_str).map(str::to_string).or_else(|| declared.clone())
}

fn query_input_schema(query: &ModuleQuery) -> Value {
    let props: Map<String, Value> = query.inputs.iter().map(|i| (i.name.clone(), json!({ "type": if i.input_type.as_deref() == Some("number") { "number" } else { "string" } }))).collect();
    let required: Vec<&str> = query.inputs.iter().filter(|i| i.required == Some(true)).map(|i| i.name.as_str()).collect();
    json!({ "type": "object", "properties": props, "required": required })
}

/// 모듈 조회 한 개 → 액션 (`locale`은 그 모듈의 locales 한 언어)
fn query_view(query: &ModuleQuery, locale: &Value) -> Value {
    json!({
        "id": format!("{}{}", QUERY_PREFIX, query.name),
        "kind": "query",
        "risk": "safe",
        "surfaces": ALL.iter().map(|s| s.name()).collect::<Vec<_>>(),
        "scope": "instance",
        "input": query_input_schema(query),
        "titleKey": Value::Null,
        "title": query_text(locale, query, "label").unwrap_or_else(|| query.name.clone()),
        "description": query_text(locale, query, "description"),
        "route": "POST /api/instance/:id/query/:name",
        "cli": format!("{} do {}{} --server <server>{}", CLI_BIN, QUERY_PREFIX, query.name, cli_args(&query.inputs.iter().filter(|i| i.required == Some(true)).map(|i| i.name.as_str()).collect::<Vec<_>>())),
    })
}

/// 인스턴스의 모듈
async fn instance_module(server: &IPCServer, instance: &str) -> Option<LoadedModule> {
    let sup = server.supervisor.read().await;
    let module = sup.instance_store.get(instance)?.module_name.clone();
    sup.module_loader.get_module(&module).ok()
}

/// `GET /api/actions` 본문
pub async fn list(server: &IPCServer, surface: Option<Surface>, instance: Option<&str>, locale: &str) -> Value {
    let cat = catalog(locale);
    let mut actions: Vec<Value> = ACTIONS
        .iter()
        .filter(|d| surface.is_none_or(|s| d.surfaces.contains(&s)))
        .filter(|d| instance.is_none() || d.instance)
        .map(|d| view(d, &cat))
        .collect();
    if let Some(id) = instance {
        // 모듈이 지원해야 하는 액션은 그 인스턴스의 모듈이 지원할 때만
        let module = instance_module(server, id).await;
        actions.retain(|a| match a["id"].as_str().and_then(module_support) {
            Some(supports) => module.as_ref().is_some_and(supports),
            None => true,
        });
        // 조회는 모든 표면에 열려 있다
        if let Some(module) = module {
            let locale = module_locale(&module, &crate::chat::i18n::normalize_language(locale));
            actions.extend(module.metadata.queries.iter().map(|q| query_view(q, &locale)));
        }
        let resp = call(server, "GET", &format!("/api/instance/{}/commands", seg(id)), None).await;
        for cmd in resp.body.get("commands").and_then(Value::as_array).into_iter().flatten() {
            if let Some(v) = command_view(cmd) {
                let on = surface.is_none_or(|s| v["surfaces"].as_array().is_some_and(|a| a.iter().any(|x| x == s.name())));
                if on {
                    actions.push(v);
                }
            }
        }
    }
    json!({ "actions": actions, "locale": crate::chat::i18n::normalize_language(locale) })
}

/// 입력 칸마다 언어별 제목 (`title: {<locale>: …}`). 제목이 하나도 없는 칸은 그대로 둔다
fn with_field_titles(mut input: Value, titles: impl Fn(&str) -> Map<String, Value>) -> Value {
    if let Some(props) = input["properties"].as_object_mut() {
        for (name, field) in props.iter_mut() {
            let title = titles(name);
            if !title.is_empty() {
                field["title"] = Value::Object(title);
            }
        }
    }
    input
}

/// heartbeat `metadata.actions` (디스코드·챗봇 표면만, 인스턴스 펼침 없이). 상한: 64개, 32768바이트.
/// 고정 액션 뒤에 모듈 조회(`server.query:<이름>`, `module` 필드로 어느 모듈의 것인지)를 모듈·이름 순으로 붙인다
pub const HEARTBEAT_MAX_ACTIONS: usize = 64;
pub const HEARTBEAT_MAX_BYTES: usize = 32768;

/// 모듈이 지원해야 쓸 수 있는 고정 액션과 그 판정
fn module_support(id: &str) -> Option<fn(&LoadedModule) -> bool> {
    match id {
        "server.players" => Some(|m| m.metadata.players_function.is_some()),
        // 밖에 여는 포트(`public`)를 선언한 모듈만 확인할 것이 있다
        "server.port_check" => Some(|m| m.metadata.settings.as_ref().is_some_and(|s| s.fields.iter().any(|f| f.public.is_some()))),
        _ => None,
    }
}

pub fn heartbeat_actions(locales: &[&str], modules: &[LoadedModule]) -> (Vec<Value>, bool) {
    let cats: Vec<(&str, crate::chat::i18n::Catalog)> = locales.iter().map(|l| (*l, catalog(l))).collect();
    let remote = |d: &Def| -> Vec<Surface> { d.surfaces.iter().copied().filter(|s| *s == Chatbot || (*s == Discord && d.remote_menu)).collect() };
    let mut defs: Vec<&Def> = ACTIONS.iter().filter(|d| !remote(d).is_empty()).collect();
    defs.sort_by_key(|d| d.id);
    let mut out = Vec::new();
    let mut size = 2usize;
    let mut truncated = false;
    let mut items = Vec::new();
    for d in defs {
        let title: Map<String, Value> = cats
            .iter()
            .map(|(l, c)| (l.to_string(), json!(translated(c, &format!("actions.{}.title", d.id)).unwrap_or_else(|| d.id.to_string()))))
            .collect();
        let mut item = json!({
            "id": d.id,
            "kind": d.kind.name(),
            "risk": d.risk,
            "surfaces": remote(d).iter().map(|s| s.name()).collect::<Vec<_>>(),
            "scope": if d.instance { "instance" } else { "none" },
            "title": title,
            "input": with_field_titles(remote_schema(d), |name| {
                cats.iter().filter_map(|(l, c)| Some((l.to_string(), json!(translated(c, &format!("actions.field.{}", name))?)))).collect()
            }),
        });
        // 서버 슬래시 루트 아래 짧은 이름 (`/<서버> 상태`): 제목의 "서버"를 뺀 것. 문구가 있는 언어만
        let name: Map<String, Value> =
            cats.iter().filter_map(|(l, c)| Some((l.to_string(), json!(translated(c, &format!("actions.{}.name", d.id))?)))).collect();
        if !name.is_empty() {
            item["name"] = Value::Object(name);
        }
        // 고정 요구 레벨: 명령별 덮어쓰기·프리셋보다 먼저다 (`authorize`)
        if let Some(level) = d.discord_level {
            item["level"] = json!(level);
        }
        // 모듈이 지원해야 하는 액션: 그 모듈 이름들 (릴레이는 이 모듈의 서버에만 하위 명령을 만든다)
        if let Some(supports) = module_support(d.id) {
            let mut names: Vec<&str> = modules.iter().filter(|m| supports(m)).map(|m| m.metadata.name.as_str()).collect();
            names.sort_unstable();
            item["modules"] = json!(names);
        }
        items.push(item);
    }
    let mut modules: Vec<&LoadedModule> = modules.iter().filter(|m| !m.metadata.queries.is_empty()).collect();
    modules.sort_by(|a, b| a.metadata.name.cmp(&b.metadata.name));
    for m in modules {
        let module_locales: Vec<(&str, Value)> = locales.iter().map(|l| (*l, module_locale(m, l))).collect();
        let mut queries: Vec<&ModuleQuery> = m.metadata.queries.iter().collect();
        queries.sort_by(|a, b| a.name.cmp(&b.name));
        for q in queries {
            let text = |key: &str| -> Map<String, Value> {
                module_locales.iter().filter_map(|(l, loc)| Some((l.to_string(), json!(query_text(loc, q, key)?)))).collect()
            };
            let mut title = text("label");
            for l in locales {
                title.entry(l.to_string()).or_insert_with(|| json!(q.name));
            }
            let mut item = json!({
                "id": format!("{}{}", QUERY_PREFIX, q.name),
                "kind": "query",
                "risk": "safe",
                "surfaces": ["discord", "chatbot"],
                "scope": "instance",
                "module": m.metadata.name,
                "title": title,
                "input": with_field_titles(query_input_schema(q), |name| {
                    let declared = q.inputs.iter().find(|i| i.name == name).and_then(|i| i.label.clone());
                    module_locales
                        .iter()
                        .filter_map(|(l, loc)| {
                            let label = loc.pointer(&format!("/queries/{}/inputs/{}/label", q.name, name)).and_then(Value::as_str).map(str::to_string).or_else(|| declared.clone())?;
                            Some((l.to_string(), json!(label)))
                        })
                        .collect()
                }),
            });
            let description = text("description");
            if !description.is_empty() {
                item["description"] = Value::Object(description);
            }
            items.push(item);
        }
    }
    // 상한을 넘으면 칸 제목부터 뺀다 (릴레이는 제목이 없으면 칸 이름을 보인다)
    let total: usize = 2 + items.iter().map(|i| i.to_string().len() + 1).sum::<usize>();
    if total > HEARTBEAT_MAX_BYTES {
        for item in &mut items {
            for field in item["input"]["properties"].as_object_mut().into_iter().flat_map(|p| p.values_mut()) {
                if let Some(f) = field.as_object_mut() {
                    f.remove("title");
                }
            }
        }
    }
    for item in items {
        let len = item.to_string().len() + 1;
        if out.len() >= HEARTBEAT_MAX_ACTIONS || size + len > HEARTBEAT_MAX_BYTES {
            truncated = true;
            break;
        }
        size += len;
        out.push(item);
    }
    (out, truncated)
}

// ── 계획·실행 ────────────────────────────────────────────

/// 요청한 쪽
#[derive(Debug, Clone)]
pub struct Caller {
    pub surface: Surface,
    /// 릴레이에서 온 요청 (레벨 판정 대상)
    pub remote: Option<Requester>,
    pub origin: crate::audit::Origin,
    pub locale: String,
}

impl Caller {
    /// GUI·CLI에서 온 HTTP 요청 (`X-Saba-Client`)
    pub fn local(origin: crate::audit::Origin, locale: String) -> Self {
        let surface = if origin.source == "gui" { Gui } else { Cli };
        Self { surface, remote: None, origin, locale }
    }

    /// planId를 묶는 키: 출처·길드·사용자 (via는 계획에 따로 남겨 실행 때 상한으로 쓴다)
    fn owner_key(&self) -> String {
        match &self.remote {
            Some(r) => format!("relay|{}|{}", r.guild_id.as_deref().unwrap_or(""), r.user_id.as_deref().unwrap_or("")),
            None => format!("local|{}", self.origin.source),
        }
    }
}

/// 실패 (`{success:false, error_code, error, ...}`)
#[derive(Debug, Clone, PartialEq)]
pub struct ActionError {
    pub status: u16,
    pub code: &'static str,
    pub message: String,
    pub data: Value,
}

impl ActionError {
    fn new(status: u16, code: &'static str, message: impl Into<String>) -> Self {
        Self { status, code, message: message.into(), data: Value::Null }
    }

    pub fn to_json(&self) -> Value {
        let mut v = json!({ "success": false, "error_code": self.code, "error": self.message });
        if let Value::Object(extra) = &self.data {
            for (k, val) in extra {
                v[k] = val.clone();
            }
        }
        v
    }
}

/// 실행할 것 하나로 푼 액션
#[derive(Debug, Clone)]
struct Resolved {
    id: String,
    kind: Kind,
    risk: String,
    surfaces: Vec<Surface>,
    discord_level: Option<i64>,
    guild_owner: bool,
    method: &'static str,
    uri: String,
    body: Option<Value>,
    instance: Option<String>,
    /// `server.command:<이름>`의 명령 이름 (레벨 덮어쓰기 키)
    command: Option<String>,
    input: Map<String, Value>,
}

fn seg(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    for b in s.bytes() {
        if b.is_ascii_alphanumeric() || matches!(b, b'-' | b'_' | b'.' | b'~') {
            out.push(b as char);
        } else {
            out.push_str(&format!("%{:02X}", b));
        }
    }
    out
}

fn bad_input(field: &str, reason: &str) -> ActionError {
    ActionError { data: json!({ "field": field, "reason": reason }), ..ActionError::new(400, "invalid_input", format!("Invalid input '{}': {}", field, reason)) }
}

fn type_ok(ty: &str, v: &Value) -> bool {
    match ty {
        "string" => v.is_string(),
        "object" => v.is_object(),
        "array" => v.is_array(),
        "integer" => v.is_i64() || v.is_u64(),
        "number" => v.is_number(),
        "boolean" => v.is_boolean(),
        _ => true,
    }
}

/// 선언과 입력으로 부를 라우트를 만든다 (검증만, 부작용 없음)
async fn resolve(server: &IPCServer, id: &str, instance: Option<&str>, input: &Value, caller: &Caller) -> Result<Resolved, ActionError> {
    resolve_in(server, id, instance, input, caller, None).await
}

/// `module_hint`: 아직 없는 서버(같은 계획에서 만들 서버)의 모듈. 원격 설정 검사에 쓴다
async fn resolve_in(server: &IPCServer, id: &str, instance: Option<&str>, input: &Value, caller: &Caller, module_hint: Option<&str>) -> Result<Resolved, ActionError> {
    let mut input = match input {
        Value::Null => Map::new(),
        Value::Object(m) => m.clone(),
        _ => return Err(bad_input("input", "must be an object")),
    };
    if let Some(name) = id.strip_prefix(QUERY_PREFIX) {
        let Some(instance) = instance else { return Err(bad_input("instance", "required")) };
        if server.supervisor.read().await.instance_store.get(instance).is_none() {
            return Err(ActionError::new(404, "not_found", format!("Instance not found: {}", instance)));
        }
        let query = instance_module(server, instance).await.and_then(|m| m.metadata.queries.into_iter().find(|q| q.name == name));
        let Some(query) = query else { return Err(ActionError::new(404, "unknown_action", format!("Unknown action: {}", id))) };
        if let Some(key) = input.keys().find(|k| !query.inputs.iter().any(|i| &i.name == *k)) {
            return Err(bad_input(key, "unknown field"));
        }
        let args = crate::commands::bind(&query.inputs, &crate::commands::Args::Named(input.clone()))
            .map_err(|e| bad_input(e.field.as_deref().unwrap_or("input"), &e.message))?;
        return Ok(Resolved {
            id: id.to_string(),
            kind: Kind::Query,
            risk: "safe".into(),
            surfaces: ALL.to_vec(),
            discord_level: None,
            guild_owner: false,
            method: "POST",
            uri: format!("/api/instance/{}/query/{}", seg(instance), seg(name)),
            body: Some(json!({ "args": Value::Object(args) })),
            instance: Some(instance.to_string()),
            command: None,
            input,
        });
    }
    if let Some(name) = id.strip_prefix(COMMAND_PREFIX) {
        let Some(instance) = instance else { return Err(bad_input("instance", "required")) };
        let resp = call(server, "GET", &format!("/api/instance/{}/commands", seg(instance)), None).await;
        if !resp.is_success() {
            return Err(ActionError::new(404, "not_found", resp.str_field("error").unwrap_or("Instance not found").to_string()));
        }
        let Some(view) = resp.body.get("commands").and_then(Value::as_array).into_iter().flatten().find(|c| c.get("name").and_then(Value::as_str) == Some(name)).and_then(command_view) else {
            return Err(ActionError::new(404, "unknown_action", format!("Unknown action: {}", id)));
        };
        let surfaces = view["surfaces"].as_array().into_iter().flatten().filter_map(|s| s.as_str().and_then(Surface::parse)).collect();
        return Ok(Resolved {
            id: id.to_string(),
            kind: Kind::Command,
            risk: view["risk"].as_str().unwrap_or("caution").to_string(),
            surfaces,
            discord_level: None,
            guild_owner: false,
            method: "POST",
            uri: format!("/api/instance/{}/command", seg(instance)),
            body: Some(json!({ "command": name, "args": Value::Object(input.clone()) })),
            instance: Some(instance.to_string()),
            command: Some(name.to_string()),
            input,
        });
    }
    let Some(def) = find(id) else { return Err(ActionError::new(404, "unknown_action", format!("Unknown action: {}", id))) };
    let instance = match (def.instance, instance) {
        (true, None) => return Err(bad_input("instance", "required")),
        (true, Some(i)) => Some(i.to_string()),
        (false, _) => None,
    };
    // 원격 요청이 다루는 길드는 그 요청이 온 길드뿐이다
    if let Some(who) = &caller.remote {
        if def.fields.iter().any(|f| f.in_path && f.name == GUILD_FIELD) {
            let Some(own) = who.guild_id.clone() else { return Err(bad_input(GUILD_FIELD, "required")) };
            match input.get(GUILD_FIELD) {
                Some(g) if g.as_str() != Some(own.as_str()) => return Err(bad_input(GUILD_FIELD, "only this server's settings")),
                _ => {
                    input.insert(GUILD_FIELD.into(), Value::String(own));
                }
            }
        }
    }
    let mut body = input.clone();
    if !def.open_input {
        for key in input.keys() {
            if !def.fields.iter().any(|f| f.name == key) {
                return Err(bad_input(key, "unknown field"));
            }
        }
    }
    let mut uri = def.path.to_string();
    if let Some(i) = &instance {
        uri = uri.replace(":id", &seg(i));
        // 이름으로 찾는 라우트 (`/api/server/:name/status`)
        if uri.contains(":iname") {
            let name = server.supervisor.read().await.instance_store.get(i).map(|x| x.name.clone());
            let Some(name) = name else { return Err(ActionError::new(404, "not_found", format!("Instance not found: {}", i))) };
            uri = uri.replace(":iname", &seg(&name));
        }
    }
    for f in def.fields {
        match input.get(f.name) {
            None if f.required => return Err(bad_input(f.name, "required")),
            None => {}
            // PATCH는 null이 그 키를 기본값으로 되돌린다
            Some(Value::Null) if !f.required && def.method == "PATCH" => {}
            Some(v) if !type_ok(f.ty, v) => return Err(bad_input(f.name, &format!("must be {}", f.ty))),
            Some(v) if f.in_path => {
                let text = v.as_str().unwrap_or_default();
                if text.is_empty() {
                    return Err(bad_input(f.name, "required"));
                }
                uri = uri.replace(&format!(":{}", f.name), &seg(text));
                body.remove(f.name);
            }
            Some(_) => {}
        }
    }
    // 원격(디스코드·챗봇)은 비밀값과 실행 파일·폴더·인자를 바꾸지 못한다 (어느 깊이든, 모듈 필드 포함)
    if caller.remote.is_some() && matches!(def.id, "instance.settings.set" | "instance.create") {
        let module = match &instance {
            Some(i) => server.supervisor.read().await.instance_store.get(i).map(|x| x.module_name.clone()).or_else(|| module_hint.map(str::to_string)),
            None => input.get("module_name").and_then(Value::as_str).map(str::to_string),
        };
        let rules = match module {
            Some(m) => server.supervisor.read().await.module_loader.get_module(&m).map(|m| field_rules(&m)).unwrap_or_default(),
            None => FieldRules::default(),
        };
        if let Some(key) = forbidden_remote_key(&Value::Object(input.clone()), &rules) {
            return Err(bad_input(&key, "this value cannot be changed remotely"));
        }
        // 서버 설정은 한 칸씩만 (묶음 값은 안에 무엇이 있는지 원격에서 다 볼 수 없다)
        if def.id == "instance.settings.set" {
            if let Some((key, _)) = input.iter().find(|(_, v)| v.is_object() || v.is_array()) {
                return Err(bad_input(key, "this value cannot be changed remotely"));
            }
        }
    }
    // GET은 본문이 없으므로 남은 입력을 쿼리로 싣는다
    if def.method == "GET" && !body.is_empty() {
        let query: Vec<String> = body
            .iter()
            .map(|(k, v)| format!("{}={}", seg(k), seg(&v.as_str().map(str::to_string).unwrap_or_else(|| v.to_string()))))
            .collect();
        uri.push(if uri.contains('?') { '&' } else { '?' });
        uri.push_str(&query.join("&"));
    }
    let body = if def.method == "GET" || def.method == "DELETE" || (body.is_empty() && !def.open_input && def.fields.is_empty()) {
        None
    } else {
        Some(Value::Object(body))
    };
    Ok(Resolved {
        id: def.id.to_string(),
        kind: def.kind,
        risk: def.risk.to_string(),
        surfaces: def.surfaces.to_vec(),
        discord_level: def.discord_level,
        guild_owner: def.guild_owner,
        method: def.method,
        uri,
        body,
        instance,
        command: None,
        input,
    })
}

/// 원격으로 나가는 결과에서 지울 모듈 password 필드 이름 (모든 모듈)
async fn all_password_fields(server: &IPCServer) -> Vec<String> {
    let sup = server.supervisor.read().await;
    sup.module_loader.discover_modules().unwrap_or_default().iter().flat_map(|m| field_rules(m).secret).collect()
}

/// 길드가 `allowedInstances`를 정했으면 그 목록 (원격만)
async fn allowed_instances(server: &IPCServer, caller: &Caller) -> Option<Vec<String>> {
    let guild = caller.remote.as_ref()?.guild_id.clone()?;
    let bot = server.config_store.get_bot_config().await;
    let list = bot.node_settings.get(&guild)?.get("allowedInstances")?.as_array()?.clone();
    Some(list.iter().filter_map(|v| v.as_str().map(str::to_string)).collect())
}

/// 표면과 레벨 판정. 챗봇 경로는 표면부터 본다 (레벨·설정으로 풀 수 없다)
async fn authorize(server: &IPCServer, r: &Resolved, caller: &Caller) -> Result<(), ActionError> {
    if !r.surfaces.contains(&caller.surface) {
        return Err(ActionError::new(403, "action_not_allowed", format!("'{}' is not available from {}", r.id, caller.surface.name())));
    }
    let Some(who) = &caller.remote else { return Ok(()) };
    let bot = server.config_store.get_bot_config().await;
    let node = who.guild_id.as_deref().and_then(|g| bot.node_settings.get(g));
    let perms = NodePermissions::from_node(node);
    let denied = |need: i64| {
        let level = perms.effective_level(who);
        ActionError {
            data: json!({ "args": { "need": need, "level": level, "via": who.via_chatbot.then_some("chatbot") } }),
            ..ActionError::new(403, "PERMISSION_DENIED", "Permission denied")
        }
    };
    // 디스코드 서버 주인은 길드 공통 설정에서 PC 주인과 같다 (사다리·금지와 상관없이, 직접 명령만)
    if r.guild_owner && !who.via_chatbot && who.user_id.is_some() && who.guild_owner_id == who.user_id {
        return Ok(());
    }
    perms.check_basic(who).map_err(|d| denied(d.need))?;
    let instance = r.instance.as_deref().unwrap_or("");
    // 이 길드에 열리지 않은 인스턴스
    if let (Some(i), Some(allowed)) = (&r.instance, allowed_instances(server, caller).await) {
        if !allowed.contains(i) {
            // 레벨 문제가 아니므로 PERMISSION_DENIED(레벨 안내)와 따로 둔다. relay 번역표의 NODE_NOT_ALLOWED
            return Err(ActionError {
                data: json!({ "args": { "instance": i } }),
                ..ActionError::new(403, "NODE_NOT_ALLOWED", "This server is not available in this Discord server")
            });
        }
    }
    let need = match (r.discord_level, &r.command) {
        (Some(level), _) => level,
        (None, Some(cmd)) => perms.required(instance, cmd, &r.risk),
        (None, None) => {
            let key = if instance.is_empty() { r.id.clone() } else { format!("{}/{}", instance, r.id) };
            // 내장 서버 액션은 명령 경로·GUI 권한 편집기와 같은 키(`<instance>/stop` 등)를 먼저 본다
            let command_key = builtin_command(&r.id).filter(|_| !instance.is_empty()).map(|cmd| format!("{}/{}", instance, cmd));
            match command_key.and_then(|k| perms.override_level(&k)).or_else(|| perms.override_level(&key)) {
                Some(level) => level,
                None if r.kind == Kind::Admin => perms.admin_need(),
                None => perms.preset_need(&r.risk),
            }
        }
    };
    // GUI에서 이 사람에게 막아 둔 명령은 채팅 경로와 같이 주인만 (액션 메뉴·확인 버튼으로 돌아가지 않게)
    let command = r.command.as_deref().or_else(|| builtin_command(&r.id));
    let need = match command {
        Some(cmd) if !instance.is_empty() && perms.member_denied(who, instance, cmd) => crate::chat::permissions::OWNER,
        _ => need,
    };
    if !perms.meets(perms.effective_level(who), need) {
        return Err(denied(need));
    }
    // 예약 작업은 나중에 요청자 없이 돈다: 새로 넣거나 켜는 서버 명령은 요청자가 지금 그 명령을 쓸 수 있어야 한다
    if r.id == "instance.schedule.set" && !instance.is_empty() {
        for (cmd, risk) in scheduled_commands(server, instance, r.body.as_ref()).await {
            perms.check(who, instance, &cmd, &risk).map_err(|d| denied(d.need))?;
        }
    }
    Ok(())
}

/// 예약 저장에서 새로 넣거나 바꾼 켜진 작업이 돌릴 서버 명령과 그 risk (정식 이름. 모르는 명령은 저장이 거절한다)
async fn scheduled_commands(server: &IPCServer, instance: &str, body: Option<&Value>) -> Vec<(String, String)> {
    let Some(tasks) = body
        .and_then(|b| b.get("schedule"))
        .and_then(|s| serde_json::from_value::<Vec<crate::scheduler::ScheduledTask>>(s.clone()).ok())
    else {
        return Vec::new();
    };
    let sup = server.supervisor.read().await;
    let Some(current) = sup.instance_store.get(instance) else { return Vec::new() };
    let meta = sup.module_loader.get_module(&current.module_name).ok().map(|m| m.metadata);
    let mut out = Vec::new();
    for task in tasks.iter().filter(|t| t.enabled && !current.schedule.contains(t)) {
        use crate::scheduler::TaskAction;
        let found = match task.action {
            TaskAction::Start => Some(("start".to_string(), "caution")),
            TaskAction::Stop => Some(("stop".to_string(), "caution")),
            TaskAction::Restart => Some(("restart".to_string(), "caution")),
            TaskAction::Command => meta
                .as_ref()
                .and_then(|m| crate::commands::find(m, task.command.as_deref().unwrap_or("")))
                .map(|f| (f.name.clone(), crate::commands::risk_of(f.risk.as_deref()))),
            _ => None,
        };
        out.extend(found.map(|(cmd, risk)| (cmd, risk.to_string())));
    }
    out
}

/// 내장 서버 액션에 대응하는 내장 명령 이름 (레벨 키 `<instance>/<명령>`)
fn builtin_command(action: &str) -> Option<&'static str> {
    match action {
        "server.start" => Some("start"),
        "server.stop" => Some("stop"),
        "server.restart" => Some("restart"),
        "server.status" => Some("status"),
        _ => None,
    }
}

/// 확인이 필요한가: danger이거나, 챗봇이 고른 관리 액션. 사람이 직접 친 caution 관리 명령은 바로 실행한다
fn needs_confirm(r: &Resolved, caller: &Caller) -> bool {
    let via_chatbot = caller.surface == Chatbot || caller.remote.as_ref().is_some_and(|w| w.via_chatbot);
    r.risk == "danger" || (r.kind == Kind::Admin && via_chatbot)
}

/// 계획 하나: 액션 하나 또는 여러 단계
enum PlanBody {
    One(Box<Resolved>),
    Steps(Vec<steps::Step>),
}

struct StoredPlan {
    body: PlanBody,
    owner: String,
    via_chatbot: bool,
    /// 확인을 거쳐야 하는 계획 (`needsConfirm`)
    needs_confirm: bool,
    created: Instant,
}

impl StoredPlan {
    fn new(body: PlanBody, caller: &Caller, needs_confirm: bool) -> Self {
        Self { body, owner: caller.owner_key(), via_chatbot: caller.remote.as_ref().is_some_and(|w| w.via_chatbot), needs_confirm, created: Instant::now() }
    }
}

fn store_plan(plan_id: String, plan: StoredPlan) {
    let mut store = plans().lock().unwrap_or_else(|e| e.into_inner());
    store.retain(|_, p| p.created.elapsed() < PLAN_TTL);
    // 확인을 기다리는 계획은 많아야 이만큼. 넘으면 가장 오래된 것부터 버린다
    while store.len() >= MAX_PLANS {
        let Some(oldest) = store.iter().min_by_key(|(_, p)| p.created).map(|(k, _)| k.clone()) else { break };
        store.remove(&oldest);
    }
    store.insert(plan_id, plan);
}

fn expires_at() -> String {
    chrono::DateTime::<chrono::Utc>::from(std::time::SystemTime::now() + PLAN_TTL).to_rfc3339()
}

const PLAN_TTL: Duration = Duration::from_secs(600);
const MAX_PLANS: usize = 500;

fn plans() -> &'static Mutex<HashMap<String, StoredPlan>> {
    static PLANS: std::sync::OnceLock<Mutex<HashMap<String, StoredPlan>>> = std::sync::OnceLock::new();
    PLANS.get_or_init(Default::default)
}

fn new_plan_id() -> String {
    uuid::Uuid::new_v4().simple().to_string()
}

/// `POST /api/actions/:id/plan`: 검증과 요약만
pub async fn plan(server: &IPCServer, id: &str, instance: Option<&str>, input: &Value, caller: &Caller) -> Result<Value, ActionError> {
    let r = resolve(server, id, instance, input, caller).await?;
    authorize(server, &r, caller).await?;
    refuse_restore_while_running(server, &r).await?;
    let summary = summarize(server, &r, &caller.locale).await;
    let plan_id = new_plan_id();
    let confirm = needs_confirm(&r, caller);
    let view = json!({
        "success": true,
        "planId": plan_id,
        "actionId": r.id,
        "instance": r.instance,
        "summary": summary,
        "risk": r.risk,
        "needsConfirm": confirm,
        "expiresAt": expires_at(),
    });
    store_plan(plan_id, StoredPlan::new(PlanBody::One(Box::new(r)), caller, confirm));
    Ok(view)
}

/// 켜진 서버의 복원은 계획부터 거절한다 (먼저 끄는 단계가 앞에 있는 여러 단계 계획은 실행 때 라우트가 본다)
async fn refuse_restore_while_running(server: &IPCServer, r: &Resolved) -> Result<(), ActionError> {
    match (r.id.as_str(), &r.instance) {
        ("backup.restore", Some(i)) if crate::ipc::handlers::backup::is_running(server, i).await => {
            Err(ActionError::new(409, "server_running", "Stop the server before restoring a backup"))
        }
        _ => Ok(()),
    }
}

/// 원격 설정 변경의 바꾸기 전 값 (`input`의 키마다). 인스턴스 칸에 없으면 모듈 설정에서, 그것도 없으면 Null
async fn current_values(server: &IPCServer, instance: &str, keys: impl Iterator<Item = &String>) -> Map<String, Value> {
    let sup = server.supervisor.read().await;
    let Some(inst) = sup.instance_store.get(instance) else { return Map::new() };
    let whole = serde_json::to_value(inst).unwrap_or_default();
    keys.map(|k| {
        let v = whole.get(k).filter(|v| !v.is_null()).or_else(|| inst.module_settings.get(k)).cloned().unwrap_or(Value::Null);
        (k.clone(), v)
    })
    .collect()
}

/// 계획 요약에 싣는 값: 비밀값 키는 어느 깊이에서든 `***`로 (있었는지만 보인다)
fn masked(key: &str, v: &Value, password_fields: &[String]) -> Value {
    if is_secret_key(key, password_fields) {
        return if v.is_null() { Value::Null } else { json!("***") };
    }
    match v {
        Value::Object(m) => Value::Object(m.iter().map(|(k, x)| (k.clone(), masked(k, x, password_fields))).collect()),
        Value::Array(a) => Value::Array(a.iter().map(|x| masked("", x, password_fields)).collect()),
        other => other.clone(),
    }
}

/// 묶음 값의 이전 값은 바꾸는 칸만 남긴다
fn project(before: &Value, input: &Value) -> Value {
    match (before, input) {
        (Value::Object(b), Value::Object(i)) => Value::Object(i.iter().map(|(k, x)| (k.clone(), b.get(k).map_or(Value::Null, |y| project(y, x)))).collect()),
        _ => before.clone(),
    }
}

/// 무엇을 바꾸는지 사람이 읽을 요약 `[{labelKey, label, value}]`
async fn summarize(server: &IPCServer, r: &Resolved, locale: &str) -> Vec<Value> {
    let cat = catalog(locale);
    let item = |key: &str, value: Value| {
        let label_key = format!("actions.field.{}", key);
        json!({ "labelKey": label_key, "label": translated(&cat, &label_key).unwrap_or_else(|| key.to_string()), "value": value })
    };
    let mut out = vec![item("action", json!(translated(&cat, &format!("actions.{}.title", r.id)).unwrap_or_else(|| r.id.clone())))];
    if let Some(i) = &r.instance {
        let name = server.supervisor.read().await.instance_store.get(i).map(|x| x.name.clone()).unwrap_or_else(|| i.clone());
        out.push(item("instance", json!(name)));
    }
    // 서버 설정은 바꾸기 전 값을 함께 (`before`)
    let before = match (r.id.as_str(), &r.instance) {
        ("instance.settings.set", Some(i)) => Some(current_values(server, i, r.input.keys()).await),
        _ => None,
    };
    let secret = all_password_fields(server).await;
    for (k, v) in &r.input {
        let mut row = item(k, masked(k, v, &secret));
        if let Some(b) = &before {
            row["before"] = masked(k, &project(b.get(k).unwrap_or(&Value::Null), v), &secret);
        }
        out.push(row);
    }

    match r.id.as_str() {
        // 덮어쓸 대상: 복원할 백업의 시각
        "backup.restore" => {
            if let Some(i) = &r.instance {
                let resp = call(server, "GET", &format!("/api/instance/{}/backups", seg(i)), None).await;
                let name = r.input.get("name").and_then(Value::as_str).unwrap_or_default();
                let found = resp.body.get("backups").and_then(Value::as_array).into_iter().flatten().find(|b| b.get("name").and_then(Value::as_str) == Some(name)).cloned();
                out.push(item("backup", found.unwrap_or(Value::Null)));
                out.push(item("overwrites", json!(translated(&cat, "actions.field.overwrites_instance").unwrap_or_else(|| "server files".into()))));
            }
        }
        // 받을 모듈의 출처·신뢰와 지금 판
        "module.install" => {
            let module = r.input.get("module").and_then(Value::as_str).unwrap_or_default();
            let manifest = call(server, "GET", "/api/modules/manifest", None).await;
            let entry = manifest.body.pointer("/manifest/modules").and_then(|m| m.get(module)).cloned();
            out.push(item("source", json!(translated(&cat, "actions.field.official_catalog").unwrap_or_else(|| "official catalog".into()))));
            out.push(item("trust", json!("verified")));
            out.push(item("version", entry.as_ref().and_then(|e| e.get("version")).cloned().unwrap_or(Value::Null)));
            let installed = server.supervisor.read().await.module_loader.get_module(module).ok().map(|m| m.metadata.version.clone());
            out.push(item("installed", json!(installed)));
        }
        _ => {}
    }
    out
}

/// `POST /api/actions/:id/run`: 입력으로 바로, 또는 planId로 (여러 단계 계획은 id `plan` 또는 빈 값)
pub async fn run(server: &IPCServer, id: &str, instance: Option<&str>, input: &Value, plan_id: Option<&str>, caller: &Caller) -> Result<Value, ActionError> {
    let (r, caller) = match plan_id {
        Some(pid) => {
            // 여러 단계 실행이 돌고 있으면 계획을 쓰기 전에 409. 자리는 실행이 끝날 때까지 쥔다 (한 액션·되돌리기 계획은 막지 않는다)
            let lane = if is_steps_plan(pid) { Some(steps::Lane::take(caller)?) } else { None };
            let stored = take_plan(pid, id, caller)?;
            // 계획을 만든 경로의 상한을 쓴다 (챗봇이 만든 계획은 사람이 확인해도 챗봇 상한)
            let mut caller = caller.clone();
            if let Some(who) = caller.remote.as_mut() {
                who.via_chatbot |= stored.via_chatbot;
            }
            if stored.via_chatbot {
                caller.surface = Chatbot;
                caller.origin.via = Some("chatbot".to_string());
            }
            match stored.body {
                PlanBody::Steps(steps) => {
                    // 누른 사람으로 지금 있는 단계를 다시 판정한다 (새 서버를 쓰는 단계는 그 차례에)
                    for step in &steps {
                        if let steps::Step::Ready(r) = step {
                            authorize(server, r, &caller).await?;
                        }
                    }
                    if steps.iter().any(steps::Step::is_admin) {
                        admit(server, &caller).await?;
                    }
                    return steps::run_steps(server, steps, caller, lane).await;
                }
                PlanBody::One(r) => {
                    authorize(server, &r, &caller).await?;
                    (*r, caller)
                }
            }
        }
        None => {
            let r = resolve(server, id, instance, input, caller).await?;
            authorize(server, &r, caller).await?;
            // 릴레이에서는 확인이 필요한 액션을 계획 없이 실행하지 않는다
            if caller.remote.is_some() && needs_confirm(&r, caller) {
                return Err(ActionError::new(409, "confirm_required", "This action needs a plan and a confirmation"));
            }
            (r, caller.clone())
        }
    };
    if r.kind == Kind::Admin {
        admit(server, &caller).await?;
    }
    Ok(execute(server, &r, &caller).await)
}

fn is_steps_plan(pid: &str) -> bool {
    plans().lock().unwrap_or_else(|e| e.into_inner()).get(pid).is_some_and(|p| matches!(p.body, PlanBody::Steps(_)))
}

/// planId를 꺼낸다 (1회용). 다른 요청자·다른 액션이면 `plan_not_yours`
fn take_plan(pid: &str, id: &str, caller: &Caller) -> Result<StoredPlan, ActionError> {
    let mut store = plans().lock().unwrap_or_else(|e| e.into_inner());
    store.retain(|_, p| p.created.elapsed() < PLAN_TTL);
    let Some(p) = store.get(pid) else { return Err(ActionError::new(404, "plan_expired", "The plan expired or does not exist")) };
    let same_action = match &p.body {
        PlanBody::One(r) => id.is_empty() || r.id == id,
        PlanBody::Steps(_) => id.is_empty() || id == steps::PLAN_ID,
    };
    if p.owner != caller.owner_key() || !same_action {
        return Err(ActionError::new(403, "plan_not_yours", "This plan belongs to another request"));
    }
    // 챗봇이 만든 확인 계획은 요청자 본인이 확인 버튼을 눌렀을 때만 (릴레이 `confirmedBy`). 계획은 버튼을 위해 남긴다
    if p.via_chatbot && p.needs_confirm && !caller.remote.as_ref().is_some_and(|w| w.confirmed_by_requester()) {
        return Err(ActionError::new(409, "confirm_required", "This plan needs the requester's confirmation"));
    }
    Ok(store.remove(pid).expect("checked"))
}

/// 원격 관리 실행 횟수 (길드당 최근 한 시간). 여러 단계 계획은 한 번으로 센다
const DEFAULT_ADMIN_RUNS_PER_HOUR: usize = 10;

fn admin_runs() -> &'static Mutex<HashMap<String, std::collections::VecDeque<Instant>>> {
    static RUNS: std::sync::OnceLock<Mutex<HashMap<String, std::collections::VecDeque<Instant>>>> = std::sync::OnceLock::new();
    RUNS.get_or_init(Default::default)
}

async fn admit(server: &IPCServer, caller: &Caller) -> Result<(), ActionError> {
    let Some(guild) = caller.remote.as_ref().and_then(|w| w.guild_id.clone()) else { return Ok(()) };
    let limit = {
        let bot = server.config_store.get_bot_config().await;
        bot.node_settings.get(&guild).and_then(|n| n.get("adminRunsPerHour")).and_then(Value::as_u64).filter(|n| *n > 0).map(|n| n as usize).unwrap_or(DEFAULT_ADMIN_RUNS_PER_HOUR)
    };
    let hour = Duration::from_secs(3600);
    let mut runs = admin_runs().lock().unwrap_or_else(|e| e.into_inner());
    let recent = runs.entry(guild).or_default();
    while recent.front().is_some_and(|t| t.elapsed() >= hour) {
        recent.pop_front();
    }
    if recent.len() >= limit {
        let wait = recent.front().map(|t| hour.saturating_sub(t.elapsed()).as_secs()).unwrap_or(0);
        return Err(ActionError {
            data: json!({ "args": { "perHour": limit, "retryInSecs": wait } }),
            ..ActionError::new(429, "rate_limited", "Too many changes from this Discord server in the last hour")
        });
    }
    recent.push_back(Instant::now());
    Ok(())
}

/// 결과를 챗봇에 IP·비밀처럼 보이는 값을 가려 보내는 액션: 콘솔, 모듈 조회(접속자 목록 등), 모듈 명령의 답
fn chatbot_masked(id: &str) -> bool {
    matches!(id, "server.console.tail" | "server.players" | "server.logs.read" | "server.logs.export") || id.starts_with(QUERY_PREFIX) || id.starts_with(COMMAND_PREFIX)
}

/// 푼 액션 하나를 실행한다: 밑 라우트 응답을 `{success, actionId, status, result}`로.
/// 원격 서버 설정 변경이 성공하면 바꾸기 전 값으로 되돌리는 계획을 함께 만든다 (`undoPlanId`)
async fn execute(server: &IPCServer, r: &Resolved, caller: &Caller) -> Value {
    let previous = match (r.id.as_str(), &r.instance, &caller.remote) {
        ("instance.settings.set", Some(i), Some(_)) => Some((i.clone(), current_values(server, i, r.input.keys()).await)),
        _ => None,
    };
    let resp = crate::audit::with_origin(caller.origin.clone(), call_as(server, r.method, &r.uri, r.body.clone(), Some(caller.origin.clone()))).await;
    let mut result = resp.body;
    if chatbot_masked(&r.id) && caller.surface == Chatbot {
        mask_console(&mut result);
    }
    if caller.remote.is_some() {
        strip_secrets(&mut result, &all_password_fields(server).await);
        strip_local_details(&mut result);
        if let Some(allowed) = allowed_instances(server, caller).await {
            retain_allowed(&mut result, &allowed);
        }
    }
    let ok = resp.status.is_success() && result.get("success").and_then(Value::as_bool) != Some(false) && result.get("ok").and_then(Value::as_bool) != Some(false);
    // 관리 액션은 밑 라우트가 감사에 남기지 않으므로 액션 id로 한 줄 남긴다 (시작·명령은 라우트가 남긴다)
    if r.kind == Kind::Admin {
        record_admin(server, r, caller, ok, resp.status.as_u16()).await;
    }
    // 원격에서 만든 서버는 그 길드의 허용 목록에 더한다 (목록을 정해 둔 길드만)
    if let (true, "instance.create", Some(new_id)) = (ok, r.id.as_str(), result.get("id").and_then(Value::as_str)) {
        steps::allow_in_guild(server, caller, new_id).await;
    }
    // 백업 목록 항목에 사람이 읽을 이름을 붙인다 (복원 칸에서 고를 때 보인다)
    if ok && r.id == "backup.list" {
        summary::label_backups(&mut result, &catalog(&caller.locale));
    }
    let mut out = json!({ "success": ok, "actionId": r.id, "status": resp.status.as_u16(), "result": result });
    // 원격 화면은 JSON을 그리지 않으므로 요청 언어의 요약을 함께 준다. 실패면 짧은 이유 (`reason`)
    if let (true, Some(_)) = (ok, &caller.remote) {
        if let Some(text) = summary::text(&r.id, &out["result"], &catalog(&caller.locale)) {
            out["text"] = json!(text);
        }
    }
    if let (false, Some(_)) = (ok, &caller.remote) {
        let code = crate::commands::error_code(resp.status.as_u16(), &out["result"]);
        out["reason"] = json!(crate::chat::reason(&catalog(&caller.locale), &code, &out["result"]));
    }
    if let (true, Some((instance, before))) = (ok, previous) {
        if let Some(undo) = undo_plan(server, &instance, before, caller).await {
            out["undoPlanId"] = json!(undo);
            out["undoExpiresAt"] = json!(expires_at());
        }
    }
    out
}

/// 바꾸기 전 값으로 되돌리는 계획 (같은 요청자, 같은 경로 상한). 전에 값이 없던 키는 되돌릴 수 없어 뺀다
async fn undo_plan(server: &IPCServer, instance: &str, before: Map<String, Value>, caller: &Caller) -> Option<String> {
    let input: Map<String, Value> = before.into_iter().filter(|(_, v)| !v.is_null()).collect();
    if input.is_empty() {
        return None;
    }
    let r = resolve(server, "instance.settings.set", Some(instance), &Value::Object(input), caller).await.ok()?;
    let plan_id = new_plan_id();
    let confirm = needs_confirm(&r, caller);
    store_plan(plan_id.clone(), StoredPlan::new(PlanBody::One(Box::new(r)), caller, confirm));
    Some(plan_id)
}

/// 관리 액션 감사 한 줄: `action`은 액션 id, 입력은 비밀을 가려 남긴다
async fn record_admin(server: &IPCServer, r: &Resolved, caller: &Caller, ok: bool, status: u16) {
    let (name, args) = {
        let sup = server.supervisor.read().await;
        let instance = r.instance.as_deref().and_then(|i| sup.instance_store.get(i));
        let module_name = instance.map(|i| i.module_name.clone()).or_else(|| r.input.get("module_name").and_then(Value::as_str).map(str::to_string));
        let module = module_name.and_then(|m| sup.module_loader.get_module(&m).ok()).map(|m| m.metadata);
        let input = Value::Object(r.input.clone());
        let (_, args) = crate::audit::redact(module.as_ref(), instance, None, Some(&input));
        (instance.map(|i| i.name.clone()), args)
    };
    let event = crate::audit::Event {
        action: &r.id,
        instance_id: r.instance.as_deref().unwrap_or(""),
        instance: name.as_deref(),
        command: None,
        args: args.as_ref(),
        daemon: None,
        ok,
        status,
    };
    crate::audit::log().record(&caller.origin, &event);
}

/// 읽기 전용 조회 (릴레이 `query`): `kind=query`인 액션만 계획 없이
pub async fn query(server: &IPCServer, id: &str, instance: Option<&str>, input: &Value, caller: &Caller) -> Result<Value, ActionError> {
    let is_query = if id.starts_with(QUERY_PREFIX) {
        true
    } else if id.starts_with(COMMAND_PREFIX) {
        false
    } else {
        find(id).is_some_and(|d| d.kind == Kind::Query)
    };
    if !is_query {
        return Err(ActionError::new(400, "not_a_query", format!("'{}' is not a read-only action", id)));
    }
    run(server, id, instance, input, None, caller).await
}

/// 챗봇에 보내는 콘솔에서 IP 주소(IPv4·IPv6)와 비밀처럼 보이는 값을 가린다
/// (`password=…`·`token: …`의 값, 글자와 숫자가 섞인 32자 이상 문자열)
pub(crate) fn mask_console(result: &mut Value) {
    mask_console_with_paths(result, false);
}

fn mask_console_with_paths(result: &mut Value, module_paths: bool) {
    struct Masks {
        ipv4: regex::Regex,
        ipv6: regex::Regex,
        keyed: regex::Regex,
        token: regex::Regex,
    }
    static MASKS: std::sync::OnceLock<Masks> = std::sync::OnceLock::new();
    let m = MASKS.get_or_init(|| Masks {
        ipv4: regex::Regex::new(r"\b(?:\d{1,3}\.){3}\d{1,3}\b").expect("regex"),
        // 후보만 넓게 잡고 실제 IPv6인지는 파싱으로 본다 (`12:34:56` 같은 시각은 남는다)
        ipv6: regex::Regex::new(r"[0-9A-Fa-f]{0,4}(?::[0-9A-Fa-f]{0,4}){2,7}").expect("regex"),
        keyed: regex::Regex::new(r"(?i)\b((?:pass(?:word|wd)?|token|secret|api[_-]?key|key)\s*[=:]\s*)\S+").expect("regex"),
        token: regex::Regex::new(r"\b[A-Za-z0-9_\-]{32,}").expect("regex"),
    });
    fn mask(s: &str, m: &Masks) -> String {
        let s = m.ipv4.replace_all(s, "x.x.x.x");
        let s = m.ipv6.replace_all(&s, |c: &regex::Captures| {
            let found = &c[0];
            if found.parse::<std::net::Ipv6Addr>().is_ok() { "x:x::x".to_string() } else { found.to_string() }
        });
        let s = m.keyed.replace_all(&s, "${1}***");
        m.token
            .replace_all(&s, |c: &regex::Captures| {
                let found = &c[0];
                let mixed = found.chars().any(|ch| ch.is_ascii_alphabetic()) && found.chars().any(|ch| ch.is_ascii_digit());
                if mixed { "***".to_string() } else { found.to_string() }
            })
            .into_owned()
    }
    fn walk(v: &mut Value, m: &Masks, module_paths: bool) {
        match v {
            Value::String(s) => {
                let masked = mask(s, m);
                let masked = if module_paths { mask_local_text(&masked, true).unwrap_or(masked) } else { masked };
                if masked != *s {
                    *s = masked;
                }
            }
            Value::Array(a) => a.iter_mut().for_each(|x| walk(x, m, module_paths)),
            Value::Object(o) => o.values_mut().for_each(|x| walk(x, m, module_paths)),
            _ => {}
        }
    }
    walk(result, m, module_paths);
}

// ── 안에서 라우트 부르기 ─────────────────────────────────

async fn call(server: &IPCServer, method: &str, uri: &str, body: Option<Value>) -> InternalResponse {
    call_as(server, method, uri, body, None).await
}

/// 데몬 라우터를 안에서 부른다 (인증·감사 미들웨어를 그대로 거친다). `origin`이 있으면 감사 출처로 쓴다
pub(crate) async fn call_as(server: &IPCServer, method: &str, uri: &str, body: Option<Value>, origin: Option<crate::audit::Origin>) -> InternalResponse {
    use tower::ServiceExt;
    let mut req = axum::http::Request::builder()
        .method(method)
        .uri(uri)
        .header("host", "127.0.0.1")
        .header("content-type", "application/json");
    if let Some(token) = crate::ipc::auth::expected_token() {
        req = req.header("X-Saba-Token", token);
    }
    let body = body.map(|b| b.to_string()).unwrap_or_default();
    let Ok(mut req) = req.body(axum::body::Body::from(body)) else {
        return InternalResponse { status: axum::http::StatusCode::INTERNAL_SERVER_ERROR, body: json!({ "error": "bad request" }) };
    };
    if let Some(origin) = origin.or_else(|| Some(crate::audit::current())) {
        req.extensions_mut().insert(origin);
    }
    match server.router().oneshot(req).await {
        Ok(resp) => into_internal(resp).await,
        Err(e) => InternalResponse { status: axum::http::StatusCode::INTERNAL_SERVER_ERROR, body: json!({ "error": e.to_string() }) },
    }
}

mod steps;
pub(crate) mod summary;
pub use steps::{plan_steps, run_status};

#[cfg(test)]
mod tests;
