//! 데몬 액션 카탈로그 (P2): 데몬이 할 수 있는 일을 한 곳에 선언하고 모든 표면(GUI·CLI·디스코드·챗봇)이 읽는다
//!
//! 액션은 기존 HTTP 라우트를 데몬 안에서 부르는 얇은 층이다 (`IPCServer::router`를 직접 부른다. 새 로직은 없다).
//! - 목록: [`list`]. 인스턴스를 주면 모듈·익스텐션 명령이 `server.command:<이름>`으로 펼쳐진다.
//! - 계획: [`plan`]은 입력을 검증하고 요약만 만든다(부작용 없음). planId는 요청자·길드·via에 묶이고 10분 산다.
//! - 실행: [`run`]은 밑 라우트를 불러 `{success, result}`로 감싼다.
//! - 표면: `surfaces`는 코드 상수다. 챗봇 경로는 여기 없는 액션을 레벨 판정 전에 거부한다(설정으로 풀 수 없다).
//!   릴레이에서 온 요청만 P1 레벨을 본다(`crate::chat::permissions`). GUI·CLI는 판정하지 않는다.

use crate::chat::permissions::{NodePermissions, Requester};
use crate::ipc::internal::{into_internal, InternalResponse};
use crate::ipc::IPCServer;
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
}

const fn field(name: &'static str, ty: &'static str, required: bool) -> Field {
    Field { name, ty, required, in_path: false }
}

const fn path_field(name: &'static str) -> Field {
    Field { name, ty: "string", required: true, in_path: true }
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
}

use Surface::{Chatbot, Cli, Discord, Gui};
const ALL: &[Surface] = &[Gui, Cli, Discord, Chatbot];
const LOCAL: &[Surface] = &[Gui, Cli];
const LOCAL_AND_CHATBOT: &[Surface] = &[Gui, Cli, Chatbot];
const LOCAL_AND_DISCORD: &[Surface] = &[Gui, Cli, Discord];

const fn def(id: &'static str, kind: Kind, risk: &'static str, surfaces: &'static [Surface], instance: bool, method: &'static str, path: &'static str) -> Def {
    Def { id, kind, risk, surfaces, instance, method, path, fields: &[], open_input: false, discord_level: None }
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
    def("instance.events", Kind::Query, "safe", ALL, true, "GET", "/api/instance/:id/events"),
    def("monitor.summary", Kind::Query, "safe", ALL, false, "GET", "/api/system/metrics?sizes=false"),
    def("backup.list", Kind::Query, "safe", ALL, true, "GET", "/api/instance/:id/backups"),
    def("backup.create", Kind::Command, "caution", ALL, true, "POST", "/api/instance/:id/backups"),
    Def { fields: &[path_field("name")], ..def("backup.restore", Kind::Admin, "danger", LOCAL_AND_CHATBOT, true, "POST", "/api/instance/:id/backups/:name/restore") },
    Def {
        fields: &[field("name", "string", true), field("module_name", "string", true), field("module_settings", "object", false)],
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
    // 디스코드 길드 설정 (I2). 원격에서는 요청이 온 길드만 (`guild`를 그 길드로 고정한다)
    Def { fields: &[path_field("guild")], ..def("discord.settings.get", Kind::Query, "safe", LOCAL_AND_DISCORD, false, "GET", "/api/discord/:guild/settings") },
    Def {
        fields: &[path_field("guild"), field("locale", "string", false), field("sleeping", "boolean", false), field("music", "object", false)],
        discord_level: Some(crate::chat::permissions::OWNER),
        ..def("discord.settings.set", Kind::Admin, "caution", LOCAL_AND_DISCORD, false, "PATCH", "/api/discord/:guild/settings")
    },
    def("update.check", Kind::Query, "safe", LOCAL_AND_DISCORD, false, "GET", "/api/updates/check"),
    Def { discord_level: Some(crate::chat::permissions::OWNER), ..def("update.apply", Kind::Admin, "danger", LOCAL_AND_DISCORD, false, "POST", "/api/updates/apply") },
];

/// 길드를 받는 액션의 경로 필드 (원격이면 요청이 온 길드로 고정)
const GUILD_FIELD: &str = "guild";

/// 모듈·익스텐션 명령 액션의 접두어 (`server.command:<이름>`)
pub const COMMAND_PREFIX: &str = "server.command:";

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

fn is_secret_key(key: &str, password_fields: &[String]) -> bool {
    let lower = key.to_ascii_lowercase();
    password_fields.iter().any(|f| f == key) || SECRET_KEYS.iter().any(|s| lower == *s || lower.ends_with(&format!("_{}", s)))
}

/// 원격(디스코드·챗봇)에서 바꿀 수 없는 인스턴스 키: 실행 파일·폴더·인자·메모리 상한과 그 밖으로 새는 연결 대상
const LAUNCH_KEYS: &[&str] = &["executable_path", "working_dir", "instance_dir", "extra_args", "extension_data", "rest_host", "memory_limit_mb"];

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
    let props: Map<String, Value> = def.fields.iter().map(|f| (f.name.to_string(), json!({ "type": f.ty }))).collect();
    let required: Vec<&str> = def.fields.iter().filter(|f| f.required).map(|f| f.name).collect();
    json!({ "type": "object", "properties": props, "required": required })
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
    })
}

/// 모듈 명령 한 개 (`GET /api/instance/:id/commands`의 항목) → 액션
fn command_view(cmd: &Value) -> Option<Value> {
    let name = cmd.get("name")?.as_str()?;
    let risk = cmd.get("risk").and_then(Value::as_str).unwrap_or("caution");
    let mut surfaces = vec!["gui", "cli", "discord"];
    // 모듈이 표면을 선언할 길이 없으므로 danger 명령은 챗봇에서 뺀다
    if risk != "danger" {
        surfaces.push("chatbot");
    }
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
    }))
}

fn input_type(input: &Value) -> &'static str {
    match input.get("type").and_then(Value::as_str) {
        Some("number") | Some("integer") => "number",
        Some("boolean") => "boolean",
        _ => "string",
    }
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

/// heartbeat `metadata.actions` (디스코드·챗봇 표면만, 인스턴스 펼침 없이). 상한: 64개, 32768바이트
pub const HEARTBEAT_MAX_ACTIONS: usize = 64;
pub const HEARTBEAT_MAX_BYTES: usize = 32768;

pub fn heartbeat_actions(locales: &[&str]) -> (Vec<Value>, bool) {
    let cats: Vec<(&str, crate::chat::i18n::Catalog)> = locales.iter().map(|l| (*l, catalog(l))).collect();
    let mut defs: Vec<&Def> = ACTIONS.iter().filter(|d| d.surfaces.contains(&Discord) || d.surfaces.contains(&Chatbot)).collect();
    defs.sort_by_key(|d| d.id);
    let mut out = Vec::new();
    let mut size = 2usize;
    let mut truncated = false;
    for d in defs {
        let title: Map<String, Value> = cats
            .iter()
            .map(|(l, c)| (l.to_string(), json!(translated(c, &format!("actions.{}.title", d.id)).unwrap_or_else(|| d.id.to_string()))))
            .collect();
        let item = json!({
            "id": d.id,
            "kind": d.kind.name(),
            "risk": d.risk,
            "surfaces": d.surfaces.iter().filter(|s| matches!(s, Discord | Chatbot)).map(|s| s.name()).collect::<Vec<_>>(),
            "scope": if d.instance { "instance" } else { "none" },
            "title": title,
            "input": schema(d),
        });
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
    let mut input = match input {
        Value::Null => Map::new(),
        Value::Object(m) => m.clone(),
        _ => return Err(bad_input("input", "must be an object")),
    };
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
            Some(i) => server.supervisor.read().await.instance_store.get(i).map(|x| x.module_name.clone()),
            None => input.get("module_name").and_then(Value::as_str).map(str::to_string),
        };
        let rules = match module {
            Some(m) => server.supervisor.read().await.module_loader.get_module(&m).map(|m| field_rules(&m)).unwrap_or_default(),
            None => FieldRules::default(),
        };
        if let Some(key) = forbidden_remote_key(&Value::Object(input.clone()), &rules) {
            return Err(bad_input(&key, "this value cannot be changed remotely"));
        }
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
    perms.check_basic(who).map_err(|d| denied(d.need))?;
    let instance = r.instance.as_deref().unwrap_or("");
    // 이 길드에 열리지 않은 인스턴스
    if let (Some(i), Some(allowed)) = (&r.instance, allowed_instances(server, caller).await) {
        if !allowed.contains(i) {
            return Err(ActionError {
                data: json!({ "args": { "need": Value::Null, "level": perms.effective_level(who), "via": who.via_chatbot.then_some("chatbot"), "instance": i } }),
                ..ActionError::new(403, "PERMISSION_DENIED", "Permission denied")
            });
        }
    }
    let need = match (r.discord_level, &r.command) {
        (Some(level), _) => level,
        (None, Some(cmd)) => perms.required(instance, cmd, &r.risk),
        (None, None) => {
            let key = if instance.is_empty() { r.id.clone() } else { format!("{}/{}", instance, r.id) };
            match perms.override_level(&key) {
                Some(level) => level,
                None if r.kind == Kind::Admin => admin_level(&perms.preset),
                None => crate::chat::permissions::preset_level(&perms.preset, &r.risk),
            }
        }
    };
    if perms.effective_level(who) < need {
        return Err(denied(need));
    }
    Ok(())
}

/// 확인이 필요한가: danger이거나 관리 액션
fn needs_confirm(r: &Resolved) -> bool {
    r.risk == "danger" || r.kind == Kind::Admin
}

struct StoredPlan {
    resolved: Resolved,
    owner: String,
    via_chatbot: bool,
    created: Instant,
}

const PLAN_TTL: Duration = Duration::from_secs(600);

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
    let summary = summarize(server, &r, &caller.locale).await;
    let plan_id = new_plan_id();
    let confirm = needs_confirm(&r);
    let expires = std::time::SystemTime::now() + PLAN_TTL;
    let expires_at = chrono::DateTime::<chrono::Utc>::from(expires).to_rfc3339();
    let view = json!({
        "success": true,
        "planId": plan_id,
        "actionId": r.id,
        "instance": r.instance,
        "summary": summary,
        "risk": r.risk,
        "needsConfirm": confirm,
        "expiresAt": expires_at,
    });
    let mut store = plans().lock().unwrap_or_else(|e| e.into_inner());
    store.retain(|_, p| p.created.elapsed() < PLAN_TTL);
    store.insert(
        plan_id,
        StoredPlan { resolved: r, owner: caller.owner_key(), via_chatbot: caller.remote.as_ref().is_some_and(|w| w.via_chatbot), created: Instant::now() },
    );
    Ok(view)
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
    for (k, v) in &r.input {
        out.push(item(k, v.clone()));
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

/// `POST /api/actions/:id/run`: 입력으로 바로, 또는 planId로
pub async fn run(server: &IPCServer, id: &str, instance: Option<&str>, input: &Value, plan_id: Option<&str>, caller: &Caller) -> Result<Value, ActionError> {
    let r = match plan_id {
        Some(pid) => {
            let stored = {
                let mut store = plans().lock().unwrap_or_else(|e| e.into_inner());
                store.retain(|_, p| p.created.elapsed() < PLAN_TTL);
                let Some(p) = store.get(pid) else { return Err(ActionError::new(404, "plan_expired", "The plan expired or does not exist")) };
                if p.owner != caller.owner_key() || (!id.is_empty() && p.resolved.id != id) {
                    return Err(ActionError::new(403, "plan_not_yours", "This plan belongs to another request"));
                }
                store.remove(pid).expect("checked")
            };
            // 계획을 만든 경로의 상한을 쓴다 (챗봇이 만든 계획은 사람이 확인해도 챗봇 상한)
            let mut caller = caller.clone();
            if let Some(who) = caller.remote.as_mut() {
                who.via_chatbot |= stored.via_chatbot;
            }
            if stored.via_chatbot {
                caller.surface = Chatbot;
            }
            authorize(server, &stored.resolved, &caller).await?;
            stored.resolved
        }
        None => {
            let r = resolve(server, id, instance, input, caller).await?;
            authorize(server, &r, caller).await?;
            // 릴레이에서는 확인이 필요한 액션을 계획 없이 실행하지 않는다
            if caller.remote.is_some() && needs_confirm(&r) {
                return Err(ActionError::new(409, "confirm_required", "This action needs a plan and a confirmation"));
            }
            r
        }
    };
    let resp = crate::audit::with_origin(caller.origin.clone(), call_as(server, r.method, &r.uri, r.body.clone(), Some(caller.origin.clone()))).await;
    let mut result = resp.body;
    if r.id == "server.console.tail" && caller.surface == Chatbot {
        mask_console(&mut result);
    }
    if caller.remote.is_some() {
        strip_secrets(&mut result, &all_password_fields(server).await);
        if let (Some(allowed), Some(servers)) = (allowed_instances(server, caller).await, result.get_mut("servers").and_then(Value::as_array_mut)) {
            servers.retain(|s| s.get("id").and_then(Value::as_str).is_some_and(|id| allowed.iter().any(|a| a == id)));
        }
    }
    let ok = resp.status.is_success() && result.get("success").and_then(Value::as_bool) != Some(false) && result.get("ok").and_then(Value::as_bool) != Some(false);
    Ok(json!({ "success": ok, "actionId": r.id, "status": resp.status.as_u16(), "result": result }))
}

/// 읽기 전용 조회 (릴레이 `query`): `kind=query`인 액션만 계획 없이
pub async fn query(server: &IPCServer, id: &str, instance: Option<&str>, input: &Value, caller: &Caller) -> Result<Value, ActionError> {
    let is_query = match id.strip_prefix(COMMAND_PREFIX) {
        Some(_) => false,
        None => find(id).is_some_and(|d| d.kind == Kind::Query),
    };
    if !is_query {
        return Err(ActionError::new(400, "not_a_query", format!("'{}' is not a read-only action", id)));
    }
    run(server, id, instance, input, None, caller).await
}

/// 챗봇에 보내는 콘솔에서 IPv4 주소를 가린다
fn mask_console(result: &mut Value) {
    static IPV4: std::sync::OnceLock<regex::Regex> = std::sync::OnceLock::new();
    let re = IPV4.get_or_init(|| regex::Regex::new(r"\b(?:\d{1,3}\.){3}\d{1,3}\b").expect("regex"));
    fn walk(v: &mut Value, re: &regex::Regex) {
        match v {
            Value::String(s) => {
                if re.is_match(s) {
                    *s = re.replace_all(s, "x.x.x.x").into_owned();
                }
            }
            Value::Array(a) => a.iter_mut().for_each(|x| walk(x, re)),
            Value::Object(m) => m.values_mut().for_each(|x| walk(x, re)),
            _ => {}
        }
    }
    walk(result, re);
}

// ── 안에서 라우트 부르기 ─────────────────────────────────

async fn call(server: &IPCServer, method: &str, uri: &str, body: Option<Value>) -> InternalResponse {
    call_as(server, method, uri, body, None).await
}

/// 데몬 라우터를 안에서 부른다 (인증·감사 미들웨어를 그대로 거친다). `origin`이 있으면 감사 출처로 쓴다
async fn call_as(server: &IPCServer, method: &str, uri: &str, body: Option<Value>, origin: Option<crate::audit::Origin>) -> InternalResponse {
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

#[cfg(test)]
mod tests;
