//! 감사 로그 (A10): 누가(GUI·CLI·디스코드·예약) 어떤 서버에 무엇을 했는지
//!
//! `<data_dir>/audit.log`에 한 줄 JSON으로 덧붙인다. 1 MiB를 넘으면 `audit.log.1`로 넘기고 새로 쓴다
//! (둘만 남는다). 출처는 요청마다 task-local `Origin`에 담는다:
//! - HTTP 요청: `layer`가 `X-Saba-Client` 헤더(GUI는 `gui`, CLI는 `cli`, 없으면 `api`)로 정한다
//! - 디스코드(릴레이 에이전트): `discord` + 요청한 사용자·길드 id
//! - 예약 작업: `schedule`
//! - 사바쨩 웹(릴레이 웹소켓, `crate::remote`): `web` + 링크를 받은 사용자 id
//!
//! 로컬 클라이언트는 같은 토큰을 쓰므로 출처 헤더는 보안 경계가 아니라 기록용이다.

use serde::Serialize;
use serde_json::{json, Value};
use std::future::Future;
use std::io::Write;
use std::path::{Path, PathBuf};
use std::sync::Mutex;

/// 이 크기를 넘으면 `audit.log.1`로 넘긴다
const MAX_BYTES: u64 = 1024 * 1024;
/// 인자·콘솔 줄은 이만큼만 남긴다
const MAX_TEXT: usize = 300;

#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct Origin {
    pub source: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub user: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub guild: Option<String>,
    /// 디스코드 표시 이름·서버 이름 (릴레이가 id와 함께 보낸다, 기록 당시 이름)
    #[serde(skip_serializing_if = "Option::is_none")]
    pub user_name: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub guild_name: Option<String>,
    /// 요청을 고른 길: 챗봇이면 "chatbot" (라우트가 챗봇에만 거는 제한을 잠금 안에서 판정한다)
    #[serde(skip_serializing_if = "Option::is_none")]
    pub via: Option<String>,
    /// 요청이 고른 언어 (`Accept-Language`의 첫 태그). 있으면 그 요청의 데몬·모듈 문구가 따른다(`ConfigStore::ui_language`). 기록하지 않는다
    #[serde(skip)]
    pub lang: Option<String>,
}

impl Origin {
    pub fn new(source: &str) -> Self {
        Self { source: source.to_string(), user: None, guild: None, user_name: None, guild_name: None, via: None, lang: None }
    }

    /// 채팅 명령을 보낸 사람·서버를 붙인다
    pub fn requested_by(self, req: &crate::chat::ChatRequest) -> Self {
        Self {
            user: req.user_id.clone(),
            guild: req.guild_id.clone(),
            user_name: req.user_name.clone(),
            guild_name: req.guild_name.clone(),
            ..self
        }
    }

    /// `X-Saba-Client` 헤더 → 출처. 소문자·숫자·`-`만, 16자까지 (로그를 더럽히지 않게).
    /// 데몬 안에서만 붙이는 출처(`INTERNAL_SOURCES`)는 헤더로 받지 않는다
    pub fn from_headers(headers: &axum::http::HeaderMap) -> Self {
        let client = headers
            .get("X-Saba-Client")
            .and_then(|v| v.to_str().ok())
            .map(|v| v.trim().to_ascii_lowercase())
            .filter(|v| !v.is_empty() && v.len() <= 16 && v.chars().all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '-'))
            .filter(|v| !INTERNAL_SOURCES.contains(&v.as_str()));
        Self { lang: accept_language(headers), ..Self::new(client.as_deref().unwrap_or("api")) }
    }
}

/// `Accept-Language`의 첫 언어 태그 (`ko-KR,ko;q=0.9` → `ko-KR`). 영문자·숫자·`-`만, 16자까지
fn accept_language(headers: &axum::http::HeaderMap) -> Option<String> {
    let value = headers.get(axum::http::header::ACCEPT_LANGUAGE)?.to_str().ok()?;
    let tag = value.split(',').next()?.split(';').next()?.trim();
    (!tag.is_empty() && tag.len() <= 16 && tag.chars().all(|c| c.is_ascii_alphanumeric() || c == '-')).then(|| tag.to_string())
}

/// 데몬이 스스로 정하는 출처: HTTP 헤더로 흉내 낼 수 없다
const INTERNAL_SOURCES: [&str; 4] = ["discord", "schedule", "daemon", "web"];

tokio::task_local! {
    static ORIGIN: Origin;
    static QUIET: ();
}

/// 안에서 일어나는 일은 기록하지 않는다 (재시작이 부르는 정지·시작처럼 바깥에서 한 건으로 남길 때)
pub async fn quietly<F: Future>(f: F) -> F::Output {
    QUIET.scope((), f).await
}

pub fn is_quiet() -> bool {
    QUIET.try_with(|_| ()).is_ok()
}

/// `origin`을 출처로 `f`를 실행한다
pub async fn with_origin<F: Future>(origin: Origin, f: F) -> F::Output {
    ORIGIN.scope(origin, f).await
}

/// 지금 출처와 조용히 표시를 가진 채 따로 돌린다 (요청이 끊겨도 끝까지 가야 하는 작업)
pub fn spawn<F>(f: F) -> tokio::task::JoinHandle<F::Output>
where
    F: Future + Send + 'static,
    F::Output: Send + 'static,
{
    let (origin, quiet) = (current(), is_quiet());
    tokio::spawn(async move {
        if quiet {
            with_origin(origin, quietly(f)).await
        } else {
            with_origin(origin, f).await
        }
    })
}

/// 지금 요청의 출처 (정해진 게 없으면 데몬 자신)
pub fn current() -> Origin {
    ORIGIN.try_with(Clone::clone).unwrap_or_else(|_| Origin::new("daemon"))
}

/// HTTP 요청마다 출처를 정하는 미들웨어
/// 데몬 안에서 부르는 요청(액션 카탈로그)은 확장값 `Origin`을 그대로 쓴다. HTTP로는 넣을 수 없다
pub async fn layer(req: axum::extract::Request, next: axum::middleware::Next) -> axum::response::Response {
    let origin = req.extensions().get::<Origin>().cloned().unwrap_or_else(|| Origin::from_headers(req.headers()));
    with_origin(origin, next.run(req)).await
}

/// 기록 한 건
#[derive(Debug, Clone, Default)]
pub struct Event<'a> {
    pub action: &'a str,
    pub instance_id: &'a str,
    pub instance: Option<&'a str>,
    pub command: Option<&'a str>,
    pub args: Option<&'a Value>,
    pub ok: bool,
    pub status: u16,
}

fn clip(s: &str) -> String {
    if s.chars().count() <= MAX_TEXT {
        s.to_string()
    } else {
        format!("{}…", s.chars().take(MAX_TEXT).collect::<String>())
    }
}

pub struct AuditLog {
    path: PathBuf,
    lock: Mutex<()>,
}

impl AuditLog {
    pub fn at(path: PathBuf) -> Self {
        Self { path, lock: Mutex::new(()) }
    }

    pub fn path(&self) -> &Path {
        &self.path
    }

    fn rotated(&self) -> PathBuf {
        let mut name = self.path.as_os_str().to_owned();
        name.push(".1");
        PathBuf::from(name)
    }

    pub fn record(&self, origin: &Origin, event: &Event<'_>) {
        let ts = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_secs())
            .unwrap_or(0);
        let mut line = json!({
            "ts": ts,
            "source": origin.source,
            "action": event.action,
            "instanceId": event.instance_id,
            "ok": event.ok,
            "status": event.status,
        });
        let obj = line.as_object_mut().expect("object");
        let fields = [
            ("user", origin.user.as_deref()),
            ("userName", origin.user_name.as_deref()),
            ("guild", origin.guild.as_deref()),
            ("guildName", origin.guild_name.as_deref()),
            ("instance", event.instance),
            ("command", event.command),
        ];
        for (key, value) in fields {
            if let Some(v) = value {
                obj.insert(key.into(), Value::String(clip(v)));
            }
        }
        if let Some(args) = event.args.filter(|a| !a.is_null()) {
            obj.insert("args".into(), Value::String(clip(&args.to_string())));
        }
        {
            let _guard = self.lock.lock().unwrap_or_else(|e| e.into_inner());
            if let Err(e) = self.append(&line) {
                tracing::warn!("[Audit] Could not write {}: {}", self.path.display(), e);
            }
        }
        // 활동 줄·토스트용 (P4). 기록과 같은 모양, `ts`는 이벤트의 `at`이 대신한다
        if let Some(obj) = line.as_object_mut() {
            obj.remove("ts");
        }
        crate::events::emit("action.run", line);
    }

    fn append(&self, line: &Value) -> std::io::Result<()> {
        if let Some(parent) = self.path.parent() {
            std::fs::create_dir_all(parent)?;
        }
        if std::fs::metadata(&self.path).is_ok_and(|m| m.len() >= MAX_BYTES) {
            // 넘기지 못해도 (편집기가 열어 둔 Windows 등) 기록은 이어서 남긴다
            if let Err(e) = std::fs::rename(&self.path, self.rotated()) {
                tracing::warn!("[Audit] Could not rotate {}: {}", self.path.display(), e);
            }
        }
        let mut f = std::fs::OpenOptions::new().create(true).append(true).open(&self.path)?;
        writeln!(f, "{}", line)
    }

    /// 최근 기록 (최신이 먼저). `instance_id`가 있으면 그 서버 것만
    pub fn recent(&self, limit: usize, instance_id: Option<&str>) -> Vec<Value> {
        let _guard = self.lock.lock().unwrap_or_else(|e| e.into_inner());
        let mut out = Vec::new();
        for path in [self.path.clone(), self.rotated()] {
            let Ok(text) = std::fs::read_to_string(&path) else { continue };
            for line in text.lines().rev() {
                let Ok(v) = serde_json::from_str::<Value>(line) else { continue };
                if instance_id.is_some_and(|id| v.get("instanceId").and_then(Value::as_str) != Some(id)) {
                    continue;
                }
                out.push(v);
                if out.len() >= limit {
                    return out;
                }
            }
        }
        out
    }
}

static GLOBAL: std::sync::OnceLock<AuditLog> = std::sync::OnceLock::new();

/// 데몬의 감사 로그 (`<data_dir>/audit.log`)
pub fn log() -> &'static AuditLog {
    GLOBAL.get_or_init(|| AuditLog::at(saba_chan_updater_lib::constants::resolve_data_dir().join("audit.log")))
}

/// 지금 출처로 한 건 남긴다
pub fn record(event: Event<'_>) {
    if !is_quiet() {
        log().record(&current(), &event);
    }
}

/// 이름만 보고 비밀로 치는 낱말 (인자·명령 이름)
const SECRET_WORDS: [&str; 6] = ["pass", "pwd", "token", "secret", "apikey", "api_key"];

fn sensitive(name: &str) -> bool {
    let name = name.to_ascii_lowercase();
    SECRET_WORDS.iter().any(|w| name.contains(w))
}

const MASK: &str = "***";

/// 기록하기 전에 비밀을 가린다 (A10 P3-1). `text`는 명령 원문이나 stdin 줄, `args`는 따로 받은 인자.
/// - 모듈이 `password`로 선언한 명령 인자, 이름에 pass·token·secret 같은 낱말이 든 인자는 값을 가린다
/// - 그런 인자를 받는 명령(이름·별명이나 `console_template`의 첫 낱말이 같은 줄)과 이름부터 비밀스러운
///   명령은 첫 낱말 뒤를 모두 가린다
/// - 인스턴스의 `password` 설정값과 REST 비밀번호는 어디에 나오든 가린다
pub fn redact(
    module: Option<&crate::supervisor::module_loader::ModuleMetadata>,
    instance: Option<&crate::instance::ServerInstance>,
    text: Option<&str>,
    args: Option<&Value>,
) -> (Option<String>, Option<Value>) {
    use crate::supervisor::module_loader::CommandInput;
    let secret_input = |i: &CommandInput| i.input_type.as_deref() == Some("password") || sensitive(&i.name);
    let fields = module.and_then(|m| m.commands.as_ref()).map(|c| c.fields.as_slice()).unwrap_or(&[]);
    let word = text.and_then(|t| t.split_whitespace().next()).unwrap_or("");
    let field = module.filter(|_| !word.is_empty()).and_then(|m| crate::commands::find(m, word));
    // 이 줄이 비밀 인자를 받는 명령의 콘솔 줄이면 인자 자리를 모른다: 첫 낱말 뒤를 모두 가린다
    let whole = !word.is_empty()
        && (sensitive(word)
            || fields.iter().any(|f| {
                f.inputs.iter().any(secret_input)
                    && f.console_template.as_deref().and_then(|t| t.split_whitespace().next()).is_some_and(|t| t.eq_ignore_ascii_case(word))
            }));
    let inputs = field.map(|f| f.inputs.as_slice()).unwrap_or(&[]);
    // 이름이 비밀스럽거나 모듈이 password로 선언한 설정 이름 (액션 입력의 module_settings처럼 중첩돼도)
    let password_settings: Vec<&str> = module
        .and_then(|m| m.settings.as_ref())
        .map(|s| s.fields.iter().filter(|f| f.field_type == "password").map(|f| f.name.as_str()).collect())
        .unwrap_or_default();
    let secret_key = |k: &str| sensitive(k) || password_settings.contains(&k);
    let secret_at = |i: usize| whole || inputs.get(i).is_some_and(secret_input);

    let text = text.map(|t| {
        let mut words = t.split_whitespace();
        let first = words.next().unwrap_or("").to_string();
        let rest: Vec<String> =
            words.enumerate().map(|(i, w)| if secret_at(i) { MASK.to_string() } else { w.to_string() }).collect();
        if whole && !rest.is_empty() {
            format!("{} {}", first, MASK)
        } else {
            std::iter::once(first).chain(rest).collect::<Vec<_>>().join(" ")
        }
    });
    let args = args.map(|a| match a {
        Value::Array(items) => Value::Array(
            items.iter().enumerate().map(|(i, v)| if secret_at(i) { json!(MASK) } else { v.clone() }).collect(),
        ),
        Value::Object(map) => Value::Object(
            map.iter()
                .map(|(k, v)| {
                    let secret = whole || inputs.iter().any(|i| i.name == *k && secret_input(i)) || secret_key(k);
                    (k.clone(), if secret { json!(MASK) } else { mask_keys(v, &secret_key) })
                })
                .collect(),
        ),
        other => other.clone(),
    });

    // 설정해 둔 비밀값이 그대로 나오면 가린다 (짧은 값은 흔한 낱말과 겹치므로 뺀다)
    let mut values: Vec<String> = instance.into_iter().flat_map(|inst| {
        let settings = module.and_then(|m| m.settings.as_ref()).map(|s| s.fields.as_slice()).unwrap_or(&[]);
        settings
            .iter()
            .filter(|f| f.field_type == "password" || sensitive(&f.name))
            .filter_map(|f| inst.module_settings.get(&f.name).and_then(Value::as_str).map(str::to_string))
            .chain(inst.rest_password.clone())
            .collect::<Vec<_>>()
    })
    .filter(|v| v.chars().count() >= 4)
    .collect();
    values.sort_by_key(|v| std::cmp::Reverse(v.len()));
    let scrub = |s: &str| values.iter().fold(s.to_string(), |acc, v| acc.replace(v.as_str(), MASK));
    let text = text.map(|t| scrub(&t));
    // 직렬화한 글자가 아니라 값 하나하나에서 가린다 (따옴표가 든 비밀번호는 직렬화하면 모양이 바뀐다)
    fn scrub_value(v: Value, scrub: &dyn Fn(&str) -> String) -> Value {
        match v {
            Value::String(s) => Value::String(scrub(&s)),
            Value::Array(items) => Value::Array(items.into_iter().map(|i| scrub_value(i, scrub)).collect()),
            Value::Object(map) => Value::Object(map.into_iter().map(|(k, v)| (k, scrub_value(v, scrub))).collect()),
            other => other,
        }
    }
    let args = args.map(|a| scrub_value(a, &scrub));
    (text, args)
}

/// 객체 안으로 내려가며 비밀 이름의 값을 가린다
fn mask_keys(v: &Value, secret_key: &dyn Fn(&str) -> bool) -> Value {
    match v {
        Value::Object(map) => Value::Object(
            map.iter().map(|(k, v)| (k.clone(), if secret_key(k) { json!(MASK) } else { mask_keys(v, secret_key) })).collect(),
        ),
        Value::Array(items) => Value::Array(items.iter().map(|i| mask_keys(i, secret_key)).collect()),
        other => other.clone(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn records_come_back_newest_first_and_survive_rotation() {
        let dir = tempfile::tempdir().unwrap();
        let log = AuditLog::at(dir.path().join("audit.log"));
        let cli = Origin::new("cli");
        let discord = Origin { user: Some("42".into()), guild: Some("7".into()), user_name: Some("사바".into()), ..Origin::new("discord") };
        log.record(&cli, &Event { action: "start", instance_id: "a", instance: Some("마크"), ok: true, status: 200, ..Default::default() });
        let args = json!(["x".repeat(400)]);
        log.record(&discord, &Event { action: "command", instance_id: "b", command: Some("say"), args: Some(&args), ok: false, status: 400, ..Default::default() });

        let all = log.recent(10, None);
        assert_eq!(all.len(), 2);
        assert_eq!(all[0]["source"], "discord");
        assert_eq!(all[0]["user"], "42");
        assert_eq!(all[0]["guild"], "7");
        assert_eq!(all[0]["userName"], "사바");
        assert!(all[0].get("guildName").is_none());
        assert!(all[0]["args"].as_str().unwrap().chars().count() <= MAX_TEXT + 1);
        assert_eq!(all[1]["instance"], "마크");
        assert!(all[1].get("user").is_none());
        assert_eq!(log.recent(10, Some("a")).len(), 1);

        // 넘긴 파일도 읽는다
        std::fs::rename(log.path(), log.rotated()).unwrap();
        log.record(&cli, &Event { action: "stop", instance_id: "a", ok: true, status: 200, ..Default::default() });
        let all = log.recent(10, None);
        assert_eq!(all.iter().map(|v| v["action"].as_str().unwrap()).collect::<Vec<_>>(), ["stop", "command", "start"]);
    }

    #[test]
    fn secrets_are_masked_before_they_reach_the_log() {
        let module = crate::supervisor::module_loader::parse_module_toml(
            r#"
[module]
name = "game"
version = "1.0.0"
entry = "lifecycle.py"

[[settings.fields]]
name = "admin_password"
type = "password"

[[commands.fields]]
name = "setadmin"
label = "setadmin"
method = "console"
console_template = "adminpass {user} {pw}"
inputs = [{ name = "user", type = "text" }, { name = "pw", type = "password" }]

[[commands.fields]]
name = "say"
label = "say"
method = "console"
console_template = "say {message}"
inputs = [{ name = "message", type = "text" }]
"#,
        )
        .expect("module.toml");
        let mut inst = crate::instance::ServerInstance::new("srv", "game");
        inst.module_settings.insert("admin_password".into(), json!("hunter22"));

        let r = |text: &str, args: Value| redact(Some(&module), Some(&inst), Some(text), Some(&args));
        // 명령 정의의 password 인자: 자리로도 이름으로도
        assert_eq!(r("setadmin", json!(["bob", "pw1"])), (Some("setadmin".into()), Some(json!(["bob", "***"]))));
        assert_eq!(r("setadmin", json!({"user": "bob", "pw": "pw1"})).1, Some(json!({"user": "bob", "pw": "***"})));
        assert_eq!(r("setadmin bob pw1", Value::Null).0.as_deref(), Some("setadmin bob ***"));
        // 그 명령의 콘솔 줄(stdin)과 이름부터 비밀스러운 명령은 뒤를 통째로
        assert_eq!(r("adminpass bob pw1", Value::Null).0.as_deref(), Some("adminpass ***"));
        assert_eq!(r("rcon_password abc", Value::Null).0.as_deref(), Some("rcon_password ***"));
        assert_eq!(r("say", json!({"token": "x", "message": "hi"})).1, Some(json!({"token": "***", "message": "hi"})));
        // 설정해 둔 비밀번호는 어디에 나오든
        assert_eq!(r("say my pw is hunter22", json!(["hunter22!"]),), (Some("say my pw is ***".into()), Some(json!(["***!"]))));
        // 평범한 명령은 그대로
        assert_eq!(r("say hello", json!(["hello"])), (Some("say hello".into()), Some(json!(["hello"]))));
        assert_eq!(redact(None, None, Some("stop"), None), (Some("stop".into()), None));
        // 따옴표가 든 비밀번호도 인자 값 안에서 가린다 (직렬화한 글자에서는 모양이 바뀐다)
        inst.module_settings.insert("admin_password".into(), json!("p\"ss1"));
        let quoted = json!(["x p\"ss1"]);
        assert_eq!(redact(Some(&module), Some(&inst), Some("say"), Some(&quoted)).1, Some(json!(["x ***"])));
    }

    #[tokio::test]
    async fn the_origin_follows_the_request_and_the_header_is_sanitized() {
        assert_eq!(current().source, "daemon");
        let seen = with_origin(Origin::new("schedule"), async { current().source }).await;
        assert_eq!(seen, "schedule");
        assert!(!is_quiet() && quietly(async { is_quiet() }).await);
        let mut h = axum::http::HeaderMap::new();
        assert_eq!(Origin::from_headers(&h).source, "api");
        h.insert("X-Saba-Client", "GUI".parse().unwrap());
        assert_eq!(Origin::from_headers(&h).source, "gui");
        h.insert("X-Saba-Client", "evil\"json".parse().unwrap());
        assert_eq!(Origin::from_headers(&h).source, "api");
        h.insert("X-Saba-Client", "discord".parse().unwrap());
        assert_eq!(Origin::from_headers(&h).source, "api");
    }

    #[test]
    fn nested_setting_secrets_are_masked_too() {
        let input = json!({ "name": "p", "module_name": "palworld", "module_settings": { "admin_password": "hunter2222", "port": 8211 } });
        let (_, args) = redact(None, None, None, Some(&input));
        let args = args.unwrap();
        assert_eq!(args["module_settings"]["admin_password"], MASK);
        assert_eq!(args["module_settings"]["port"], 8211);
        assert_eq!(args["name"], "p");
    }

    /// Accept-Language는 첫 언어 태그만 받는다. 모양이 이상하거나 `*`면 없는 것으로 본다. 감사 기록에는 남기지 않는다
    #[test]
    fn the_request_language_comes_from_accept_language() {
        let lang = |v: &str| {
            let mut h = axum::http::HeaderMap::new();
            h.insert("Accept-Language", v.parse().unwrap());
            Origin::from_headers(&h).lang
        };
        assert_eq!(lang("ko-KR,ko;q=0.9").as_deref(), Some("ko-KR"));
        assert_eq!(lang(" en ").as_deref(), Some("en"));
        assert_eq!(lang("zh-CN;q=0.8").as_deref(), Some("zh-CN"));
        assert_eq!(lang("*"), None);
        assert_eq!(lang("ko/../x"), None);
        assert_eq!(lang(""), None);
        assert_eq!(Origin::from_headers(&axum::http::HeaderMap::new()).lang, None);
        let origin = Origin { lang: Some("ko".into()), ..Origin::new("cli") };
        assert!(serde_json::to_value(&origin).unwrap().get("lang").is_none());
    }
}
