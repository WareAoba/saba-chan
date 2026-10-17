//! 명령 처리 — 갈래마다 파일 하나

pub mod audit;
pub mod backup;
pub mod bot;
pub mod chatbot;
pub mod config;
pub mod console;
pub mod daemon;
pub mod do_action;
pub mod extension;
pub mod module;
pub mod monitor;
pub mod perm;
pub mod sandbox;
pub mod schedule;
pub mod server;
pub mod status;
pub mod update;

use crate::cli::Cmd;
use crate::ctx::Ctx;
use crate::error::{usage, Result};
use serde_json::{Map, Value};

pub async fn dispatch(ctx: &Ctx, cmd: Cmd) -> Result<()> {
    if needs_daemon(&cmd) {
        ctx.ensure_daemon().await?;
    }
    match cmd {
        Cmd::Status => status::run(ctx).await,
        Cmd::Version => status::version(ctx).await,
        Cmd::Monitor(a) => monitor::run(ctx, a).await,
        Cmd::Audit { server, limit, follow } => audit::run(ctx, server.as_deref(), limit, follow).await,
        Cmd::Perm(c) => perm::run(ctx, c).await,
        Cmd::Sandbox(c) => sandbox::run(ctx, c).await,
        Cmd::Do { action, server, plan, input } => do_action::run(ctx, action.as_deref(), server.as_deref(), plan, &input).await,
        Cmd::Server(c) => server::run(ctx, c).await,
        Cmd::Start(a) => server::start(ctx, a).await,
        Cmd::Stop(a) => server::stop(ctx, a).await,
        Cmd::Restart(a) => server::restart(ctx, a).await,
        Cmd::Console(a) => console::run(ctx, a).await,
        Cmd::Run(a) => server::run_command(ctx, a).await,
        Cmd::Daemon(c) => daemon::run(ctx, c).await,
        Cmd::Module(c) => module::run(ctx, c).await,
        Cmd::Extension(c) => extension::run(ctx, c).await,
        Cmd::Bot(c) => bot::run(ctx, c).await,
        Cmd::Backup(c) => backup::run(ctx, c).await,
        Cmd::Schedule(c) => schedule::run(ctx, c).await,
        Cmd::Chatbot(c) => chatbot::run(ctx, c).await,
        Cmd::Update(c) => update::run(ctx, c).await,
        Cmd::Config(c) => config::run(ctx, c).await,
        Cmd::Chat { text } => chat(ctx, &text.join(" ")).await,
        Cmd::Completions { shell } => {
            let mut cmd = crate::cli::localized();
            clap_complete::generate(shell, &mut cmd, "saba-chan-cli", &mut std::io::stdout());
            Ok(())
        }
        Cmd::Shell => Box::pin(crate::shell::run(ctx)).await,
    }
}

/// 데몬 없이도 되는 명령은 데몬을 깨우지 않는다
fn needs_daemon(cmd: &Cmd) -> bool {
    use crate::cli::DaemonCmd;
    !matches!(
        cmd,
        Cmd::Status
            | Cmd::Version
            | Cmd::Completions { .. }
            | Cmd::Shell
            | Cmd::Daemon(DaemonCmd::Start | DaemonCmd::Status | DaemonCmd::Stop { .. } | DaemonCmd::Restart { .. })
    )
}

/// POST /api/chat/command — 디스코드와 같은 처리기
pub async fn chat(ctx: &Ctx, text: &str) -> Result<()> {
    let r = ctx.api.chat(text).await?;
    // 답은 디스코드용 마크다운이다: 터미널에서는 강조 기호만 뗀다
    ctx.out
        .data(&r, |r| println!("{}", plain(crate::output::s(r, "reply"))));
    Ok(())
}

fn plain(md: &str) -> String {
    md.replace("**", "").replace('`', "")
}

/// 응답의 `warnings[{code, module|extension}]` (미검증·변경된 구성요소 알림)을 경고로 보인다
pub fn print_trust_warnings(ctx: &Ctx, r: &Value) {
    for w in r.get("warnings").and_then(Value::as_array).into_iter().flatten() {
        let code = w.get("code").and_then(Value::as_str).unwrap_or_default();
        let id = w.get("module").or_else(|| w.get("extension")).and_then(Value::as_str).unwrap_or_default();
        if let Some(text) = crate::i18n::try_t(&format!("trust.notice.{}", code)) {
            ctx.out.warn(&text.replace("{{id}}", id));
        }
        if w.get("isolated").and_then(Value::as_bool) == Some(true) {
            ctx.out.warn(&t!("trust.notice.isolated"));
        }
    }
}

/// 코드 검사 결과: 선언한 권한, 찾은 사용처(선언 안 한 것은 빨갛게)
/// `declared`는 `{권한: bool}`, `findings`·`undeclared`는 `[{permission, file, line, detail}]`
pub fn print_audit(ctx: &Ctx, label: &str, r: &Value) {
    ctx.out.data(r, |r| {
        let declared: Vec<&str> = r
            .get("declared")
            .and_then(Value::as_object)
            .into_iter()
            .flatten()
            .filter(|(_, v)| v.as_bool() == Some(true))
            .map(|(k, _)| k.as_str())
            .collect();
        let findings = r.get("findings").and_then(Value::as_array).cloned().unwrap_or_default();
        let undeclared = r
            .get("undeclared")
            .and_then(Value::as_array)
            .cloned()
            .unwrap_or_default();
        let key = |f: &Value| {
            format!(
                "{}|{}|{}",
                crate::output::s(f, "permission"),
                crate::output::s(f, "file"),
                f.get("line").map(|l| l.to_string()).unwrap_or_default()
            )
        };
        let bad: std::collections::HashSet<String> = undeclared.iter().map(key).collect();
        crate::output::heading(label);
        let declared_text = if declared.is_empty() {
            crate::output::dim(&t!("audit.none_declared"))
        } else {
            declared.join(", ")
        };
        crate::output::print_kv(&[(t!("audit.declared"), declared_text)]);
        println!();
        if findings.is_empty() {
            println!("{}", crate::output::dim(&t!("audit.no_findings")));
        } else {
            let mut t =
                crate::output::table(&[t!("audit.col.permission"), t!("audit.col.where"), t!("audit.col.what")]);
            for f in &findings {
                let perm = crate::output::s(f, "permission");
                let perm = if bad.contains(&key(f)) {
                    ::console::style(format!("✗ {}", perm)).red().to_string()
                } else {
                    format!("  {}", perm)
                };
                let line = f.get("line").and_then(Value::as_u64).unwrap_or(0);
                t.add_row(vec![
                    perm,
                    format!("{}:{}", crate::output::s(f, "file"), line),
                    crate::output::s(f, "detail").to_string(),
                ]);
            }
            crate::output::print_table(t);
            println!();
        }
        if undeclared.is_empty() {
            println!("{} {}", ::console::style("✓").green().bold(), t!("audit.clean"));
        } else {
            let mut perms: Vec<&str> = undeclared.iter().map(|f| crate::output::s(f, "permission")).collect();
            perms.sort_unstable();
            perms.dedup();
            ctx.out.warn(&t!(
                "audit.undeclared",
                count = undeclared.len(),
                permissions = perms.join(", ")
            ));
        }
    });
}

// ── KEY=VALUE 도우미 ──

pub fn split_pairs(pairs: &[String]) -> Result<Vec<(String, String)>> {
    pairs
        .iter()
        .map(|p| {
            p.split_once('=')
                .map(|(k, v)| (k.trim().to_string(), v.to_string()))
                .filter(|(k, _)| !k.is_empty())
                .ok_or_else(|| usage(t!("err.pair_expected", got = p)))
        })
        .collect()
}

/// 형식을 모를 때 값 추측: true/false, 숫자, null, JSON 배열·객체, 아니면 문자열
pub fn guess(raw: &str) -> Value {
    let s = raw.trim();
    match s {
        "true" => return Value::Bool(true),
        "false" => return Value::Bool(false),
        "null" => return Value::Null,
        _ => {}
    }
    if s.starts_with('[') || s.starts_with('{') || s.starts_with('"') {
        if let Ok(v) = serde_json::from_str(s) {
            return v;
        }
    }
    if let Ok(n) = s.parse::<i64>() {
        return Value::from(n);
    }
    if let Ok(f) = s.parse::<f64>() {
        if f.is_finite() && !s.ends_with('.') {
            return Value::from(f);
        }
    }
    Value::String(raw.to_string())
}

/// 형식을 아는 값: 숫자·참거짓 칸은 그 형식으로, 나머지는 문자열 그대로 (비밀번호 "1234"가 숫자가 되지 않게)
pub fn typed(raw: &str, kind: &str) -> Result<Value> {
    match kind {
        "number" | "integer" => {
            let s = raw.trim();
            if let Ok(n) = s.parse::<i64>() {
                Ok(Value::from(n))
            } else {
                s.parse::<f64>()
                    .map(Value::from)
                    .map_err(|_| usage(t!("err.not_a_number", value = raw)))
            }
        }
        "boolean" | "bool" | "checkbox" | "toggle" => parse_bool(raw)
            .map(Value::Bool)
            .ok_or_else(|| usage(t!("err.not_a_bool", value = raw))),
        _ => Ok(Value::String(raw.to_string())),
    }
}

pub fn parse_bool(raw: &str) -> Option<bool> {
    match raw.trim().to_ascii_lowercase().as_str() {
        "true" | "yes" | "y" | "on" | "1" | "켜기" | "예" => Some(true),
        "false" | "no" | "n" | "off" | "0" | "끄기" | "아니오" => Some(false),
        _ => None,
    }
}

/// 점 경로(`a.b.c`)에 값을 넣는다. 중간 객체가 없으면 만든다. `Value::Null`이면 키를 지운다.
pub fn set_path(root: &mut Value, path: &str, value: Value) {
    if !root.is_object() {
        *root = Value::Object(Map::new());
    }
    let parts: Vec<&str> = path.split('.').collect();
    let mut cur = root;
    for (i, part) in parts.iter().enumerate() {
        let map = cur.as_object_mut().expect("object");
        if i == parts.len() - 1 {
            if value.is_null() {
                map.remove(*part);
            } else {
                map.insert(part.to_string(), value);
            }
            return;
        }
        let next = map.entry(part.to_string()).or_insert_with(|| Value::Object(Map::new()));
        if !next.is_object() {
            *next = Value::Object(Map::new());
        }
        cur = next;
    }
}

pub fn get_path<'v>(root: &'v Value, path: &str) -> Option<&'v Value> {
    path.split('.').try_fold(root, |v, part| v.get(part))
}

/// 중첩 객체를 `a.b = 값` 줄로 펼친다
pub fn flatten(v: &Value, prefix: &str, out: &mut Vec<(String, Value)>) {
    match v {
        Value::Object(map) if !map.is_empty() => {
            for (k, child) in map {
                let p = if prefix.is_empty() {
                    k.clone()
                } else {
                    format!("{}.{}", prefix, k)
                };
                flatten(child, &p, out);
            }
        }
        other => out.push((prefix.to_string(), other.clone())),
    }
}

/// 모듈 lifecycle 점검 결과(`issues: [{severity, message, solution}]`)를 그린다
pub fn print_issues(v: &Value) -> bool {
    let issues = v.get("issues").and_then(Value::as_array).cloned().unwrap_or_default();
    for i in &issues {
        let sev = crate::output::s(i, "severity");
        let mark = match sev {
            "critical" | "error" => ::console::style("✗").red().bold().to_string(),
            "warning" | "warn" => ::console::style("!").yellow().bold().to_string(),
            _ => ::console::style("·").dim().to_string(),
        };
        let msg = crate::output::s(i, "message");
        let msg = if msg.is_empty() {
            crate::output::s(i, "code")
        } else {
            msg
        };
        println!("{} {}", mark, msg);
        let fix = crate::output::s(i, "solution");
        if !fix.is_empty() {
            println!("  {}", crate::output::dim(fix));
        }
    }
    !issues.is_empty()
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn guesses_values() {
        assert_eq!(guess("true"), json!(true));
        assert_eq!(guess("25565"), json!(25565));
        assert_eq!(guess("1.5"), json!(1.5));
        assert_eq!(guess("[\"a\"]"), json!(["a"]));
        assert_eq!(guess("hello world"), json!("hello world"));
        assert_eq!(guess("null"), Value::Null);
    }

    #[test]
    fn typed_keeps_passwords_as_strings() {
        assert_eq!(typed("1234", "password").unwrap(), json!("1234"));
        assert_eq!(typed("1234", "number").unwrap(), json!(1234));
        assert_eq!(typed("on", "boolean").unwrap(), json!(true));
        assert!(typed("maybe", "boolean").is_err());
    }

    #[test]
    fn dotted_paths() {
        let mut v = json!({ "a": { "b": 1 } });
        set_path(&mut v, "a.c.d", json!("x"));
        set_path(&mut v, "a.b", Value::Null);
        assert_eq!(v, json!({ "a": { "c": { "d": "x" } } }));
        assert_eq!(get_path(&v, "a.c.d"), Some(&json!("x")));
        let mut flat = vec![];
        flatten(&v, "", &mut flat);
        assert_eq!(flat, vec![("a.c.d".to_string(), json!("x"))]);
    }

    #[test]
    fn pairs_need_equals() {
        assert!(split_pairs(&["port=1".into()]).is_ok());
        assert!(split_pairs(&["port".into()]).is_err());
        assert_eq!(split_pairs(&["motd=a=b".into()]).unwrap()[0].1, "a=b");
    }
}
