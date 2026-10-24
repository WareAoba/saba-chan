//! 명령 처리 — 갈래마다 파일 하나

pub mod audit;
pub mod backup;
pub mod bot;
pub mod chatbot;
pub mod config;
pub mod console;
pub mod daemon;
pub mod discord_words;
pub mod do_action;
pub mod events;
pub mod extension;
pub mod install;
pub mod module;
pub mod monitor;
pub mod perm;
pub mod sandbox;
pub mod schedule;
pub mod server;
pub mod status;
pub mod uninstall;
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
        Cmd::Do { run_id: Some(id), .. } => do_action::run_status(ctx, &id).await,
        Cmd::Do { steps: Some(steps), plan, .. } => do_action::steps(ctx, &steps, plan).await,
        Cmd::Do { action, server, plan, input, .. } => do_action::run(ctx, action.as_deref(), server.as_deref(), plan, &input).await,
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
        Cmd::Events { kind, server, since } => events::run(ctx, &kind, server.as_deref(), since).await,
        Cmd::Help { command } => help(ctx, &command),
        Cmd::Completions { shell } => {
            let mut cmd = crate::cli::localized();
            clap_complete::generate(shell, &mut cmd, "saba-chan-cli", &mut std::io::stdout());
            Ok(())
        }
        Cmd::Shell => Box::pin(crate::shell::run(ctx)).await,
        Cmd::Tui => Box::pin(crate::tui::run(ctx)).await,
        Cmd::Install(a) => install::run(ctx, a).await,
        Cmd::Uninstall(a) => uninstall::run(ctx, a).await,
    }
}

/// `help [명령…]`: 번역된 도움말. `--json`이면 명령 트리·인자·종료 코드
fn help(ctx: &Ctx, path: &[String]) -> Result<()> {
    let mut root = crate::cli::localized();
    root.build();
    let unknown = || usage(t!("err.unknown_command", path = path.join(" ")));
    if ctx.out.json {
        let v = crate::cli::schema(&root, path).ok_or_else(unknown)?;
        ctx.out.data(&v, |_| {});
        return Ok(());
    }
    let mut cmd = &mut root;
    for word in path {
        cmd = cmd.find_subcommand_mut(word).ok_or_else(unknown)?;
    }
    let text = cmd.render_help();
    let text = if ::console::colors_enabled() { text.ansi().to_string() } else { text.to_string() };
    say!("{}", text.trim_end());
    Ok(())
}

/// 데몬 없이도 되는 명령은 데몬을 깨우지 않는다
fn needs_daemon(cmd: &Cmd) -> bool {
    use crate::cli::DaemonCmd;
    !matches!(
        cmd,
        Cmd::Status
            | Cmd::Version
            | Cmd::Completions { .. }
            | Cmd::Help { .. }
            | Cmd::Shell
            | Cmd::Tui
            | Cmd::Install(_)
            | Cmd::Uninstall(_)
            | Cmd::Daemon(DaemonCmd::Start | DaemonCmd::Status | DaemonCmd::Stop { .. } | DaemonCmd::Restart { .. })
    )
}

/// POST /api/chat/command — 디스코드와 같은 처리기
pub async fn chat(ctx: &Ctx, text: &str) -> Result<()> {
    let r = ctx.api.chat(text).await?;
    // 답은 디스코드용 마크다운이다: 터미널에서는 강조 기호만 뗀다
    ctx.out
        .data(&r, |r| say!("{}", plain(crate::output::s(r, "reply"))));
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
        say!();
        if findings.is_empty() {
            say!("{}", crate::output::dim(&t!("audit.no_findings")));
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
            say!();
        }
        if undeclared.is_empty() {
            say!("{} {}", ::console::style("✓").green().bold(), t!("audit.clean"));
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
        // 스키마 2 목록: JSON 배열 그대로, 아니면 쉼표로 나눈 글자들 (빈 값은 빈 목록)
        "list" => match guess(raw) {
            // 글자만 든 JSON 배열만 그대로. `[1]` 같은 한 항목은 글자 "[1]"이다
            Value::Array(a) if a.iter().all(Value::is_string) => Ok(Value::Array(a)),
            _ => Ok(Value::Array(
                raw.split(',')
                    .map(str::trim)
                    .filter(|p| !p.is_empty())
                    .map(|p| Value::String(p.to_string()))
                    .collect(),
            )),
        },
        // 한 줄로 치므로 `\n`을 줄바꿈으로, `\\`를 `\`로
        "multiline" => {
            let mut out = String::with_capacity(raw.len());
            let mut chars = raw.chars();
            while let Some(c) = chars.next() {
                match (c, chars.clone().next()) {
                    ('\\', Some('n')) => {
                        out.push('\n');
                        chars.next();
                    }
                    ('\\', Some('\\')) => {
                        out.push('\\');
                        chars.next();
                    }
                    _ => out.push(c),
                }
            }
            Ok(Value::String(out))
        }
        _ => Ok(Value::String(raw.to_string())),
    }
}

/// 지금 값의 형식에 맞춰 변환한다. 목록은 쉼표로 나눈 글자들 (JSON 배열도 받는다)
pub fn coerce(raw: &str, current: Option<&Value>) -> Result<Value> {
    match current {
        Some(Value::Bool(_)) => parse_bool(raw)
            .map(Value::Bool)
            .ok_or_else(|| usage(t!("err.not_a_bool", value = raw))),
        Some(Value::Number(_)) => typed(raw, "number"),
        Some(Value::String(_)) => Ok(Value::String(raw.to_string())),
        Some(Value::Array(_)) => match guess(raw) {
            Value::Array(a) => Ok(Value::Array(a)),
            _ => Ok(Value::Array(
                raw.split(',')
                    .map(str::trim)
                    .filter(|p| !p.is_empty())
                    .map(|p| Value::String(p.to_string()))
                    .collect(),
            )),
        },
        _ => Ok(guess(raw)),
    }
}

/// 없는 설정 키: 비슷한 이름을 몇 개 권한다
pub fn unknown_setting(key: &str, names: &[String]) -> anyhow::Error {
    let suggestions = match crate::resolve::find(names, key, |n| vec![n.clone()], |n| n.clone()) {
        crate::resolve::Found::None(s) => s,
        crate::resolve::Found::One(n) => vec![n],
        crate::resolve::Found::Many(v) => v.into_iter().take(3).collect(),
    };
    crate::error::CliError::NotFound {
        what: t!("noun.setting"),
        query: key.into(),
        suggestions,
    }
    .into()
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

/// 고친 경로만 담은 merge patch (null은 그 키를 지운다). 읽은 때의 revision을 실어, 그사이 다른 곳이 바꿨으면
/// 데몬이 409 `settings_changed`로 거절한다
pub fn config_patch(cfg: &Value, changes: Vec<(String, Value)>) -> Value {
    let mut patch = Map::new();
    if let Some(revision) = cfg.get("revision") {
        patch.insert("revision".into(), revision.clone());
    }
    for (key, value) in changes {
        let mut cur = &mut patch;
        let mut parts = key.split('.').peekable();
        while let Some(part) = parts.next() {
            if parts.peek().is_none() {
                cur.insert(part.to_string(), value);
                break;
            }
            let next = cur.entry(part.to_string()).or_insert_with(|| Value::Object(Map::new()));
            if !next.is_object() {
                *next = Value::Object(Map::new());
            }
            cur = next.as_object_mut().expect("object");
        }
    }
    Value::Object(patch)
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
        say!("{} {}", mark, msg);
        let fix = crate::output::s(i, "solution");
        if !fix.is_empty() {
            say!("  {}", crate::output::dim(fix));
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

    /// 스키마 2 `list`는 문자열 배열로 (쉼표로 나누거나 JSON 배열), `multiline`은 `\\n`을 줄바꿈으로 보낸다 (06 §8.1)
    #[test]
    fn list_and_multiline_values_take_their_shape() {
        assert_eq!(typed(" a, b ,,c", "list").unwrap(), json!(["a", "b", "c"]));
        assert_eq!(typed("[\"a,b\", \"c\"]", "list").unwrap(), json!(["a,b", "c"]));
        assert_eq!(typed("", "list").unwrap(), json!([]));
        assert_eq!(typed("[1]", "list").unwrap(), json!(["[1]"]));
        assert_eq!(
            typed("Welcome\\nHave fun", "multiline").unwrap(),
            json!("Welcome\nHave fun")
        );
        assert_eq!(typed("C:\\\\new", "multiline").unwrap(), json!("C:\\new"));
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
