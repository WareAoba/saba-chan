//! `config` — 앱 설정 (config.json의 `gui` 칸 + CLI 전용 `cli` 칸)
//!
//! 키는 데몬이 쓰는 그대로(camelCase)다. `cli.`로 시작하면 CLI 전용 칸으로 간다.
//! 값의 형식은 지금 값의 형식을 따른다 (참거짓 칸에 "on"을 쓰면 true).
//! `reset`은 데몬이 기본값으로 되돌린다 (CLI 칸은 기본값이 없어 키를 지운다).

use crate::cli::ConfigCmd;
use crate::cmd::{flatten, get_path, guess, parse_bool, split_pairs, typed};
use crate::ctx::Ctx;
use crate::error::{usage, CliError, Result};
use crate::output;
use serde_json::{json, Map, Value};

pub async fn run(ctx: &Ctx, cmd: ConfigCmd) -> Result<()> {
    match cmd {
        ConfigCmd::Show => {
            let (app, cli) = tokio::join!(ctx.api.app_config(), ctx.api.cli_config());
            let all = json!({ "app": app?, "cli": cli.unwrap_or_else(|_| json!({})) });
            ctx.out.data(&all, |all| {
                let mut rows = Vec::new();
                flatten(&all["app"], "", &mut rows);
                flatten(&all["cli"], "cli", &mut rows);
                rows.retain(|(k, _)| !k.is_empty() && k != "cli");
                output::print_kv(
                    &rows
                        .iter()
                        .map(|(k, v)| (k.clone(), output::mask(k, v)))
                        .collect::<Vec<_>>(),
                );
            });
            Ok(())
        }
        ConfigCmd::Get { key } => {
            let (root, path) = split_key(&key);
            let doc = load(ctx, root).await?;
            let v = get_path(&doc, path)
                .cloned()
                .ok_or_else(|| not_found(&key, &doc, root))?;
            ctx.out.data(&v, |v| println!("{}", output::show(v)));
            Ok(())
        }
        ConfigCmd::Set { pairs } => set(ctx, &pairs).await,
        ConfigCmd::Defaults => {
            let r = ctx.api.config_defaults("gui").await?;
            let d = r.get("defaults").cloned().unwrap_or(Value::Null);
            ctx.out.data(&d, |d| {
                let mut rows = Vec::new();
                flatten(d, "", &mut rows);
                output::print_kv(
                    &rows
                        .iter()
                        .map(|(k, v)| (k.clone(), output::show(v)))
                        .collect::<Vec<_>>(),
                );
            });
            Ok(())
        }
        ConfigCmd::Reset { keys } => reset(ctx, &keys).await,
    }
}

async fn reset(ctx: &Ctx, keys: &[String]) -> Result<()> {
    if keys.iter().any(|k| split_key(k).1.contains('.')) {
        return Err(usage(t!("config.reset_top_only")));
    }
    let q = if keys.is_empty() {
        t!("config.confirm_reset_all")
    } else {
        t!("config.confirm_reset", keys = keys.join(", "))
    };
    ctx.ask.ensure(&q)?;
    let app: Vec<String> = keys.iter().filter(|k| split_key(k).0 == Root::App).cloned().collect();
    let cli: Map<String, Value> = keys
        .iter()
        .filter(|k| split_key(k).0 == Root::Cli)
        .map(|k| (split_key(k).1.to_string(), Value::Null))
        .collect();
    let mut done = Map::new();
    // 키를 안 주면 앱 설정 전체 (CLI 칸은 그대로)
    if keys.is_empty() || !app.is_empty() {
        done.insert("app".into(), ctx.api.config_reset("gui", &app).await?);
    }
    if !cli.is_empty() {
        done.insert("cli".into(), save(ctx, Root::Cli, Value::Object(cli)).await?);
    }
    let what = if keys.is_empty() {
        t!("config.all")
    } else {
        keys.join(", ")
    };
    ctx.out.done(&t!("config.reset", key = what), &Value::Object(done));
    Ok(())
}

#[derive(Clone, Copy, PartialEq)]
enum Root {
    App,
    Cli,
}

fn split_key(key: &str) -> (Root, &str) {
    match key.strip_prefix("cli.") {
        Some(rest) => (Root::Cli, rest),
        None => (Root::App, key),
    }
}

async fn load(ctx: &Ctx, root: Root) -> Result<Value> {
    match root {
        Root::App => ctx.api.app_config().await,
        Root::Cli => ctx.api.cli_config().await,
    }
}

async fn save(ctx: &Ctx, root: Root, patch: Value) -> Result<Value> {
    match root {
        Root::App => ctx.api.save_app_config(patch).await,
        Root::Cli => ctx.api.save_cli_config(patch).await,
    }
}

fn not_found(key: &str, doc: &Value, root: Root) -> anyhow::Error {
    let mut rows = Vec::new();
    flatten(doc, "", &mut rows);
    let prefix = if root == Root::Cli { "cli." } else { "" };
    let names: Vec<String> = rows.into_iter().map(|(k, _)| format!("{}{}", prefix, k)).collect();
    let found = crate::resolve::find(&names, key, |n| vec![n.clone()], |n| n.clone());
    let suggestions = match found {
        crate::resolve::Found::None(s) => s,
        crate::resolve::Found::One(n) => vec![n],
        crate::resolve::Found::Many(v) => v.into_iter().take(3).collect(),
    };
    CliError::NotFound {
        what: t!("noun.setting"),
        query: key.into(),
        suggestions,
    }
    .into()
}

/// 지금 값의 형식에 맞춰 변환한다
fn coerce(raw: &str, current: Option<&Value>) -> Result<Value> {
    match current {
        Some(Value::Bool(_)) => parse_bool(raw)
            .map(Value::Bool)
            .ok_or_else(|| usage(t!("err.not_a_bool", value = raw))),
        Some(Value::Number(_)) => typed(raw, "number"),
        Some(Value::String(_)) => Ok(Value::String(raw.to_string())),
        _ => Ok(guess(raw)),
    }
}

async fn set(ctx: &Ctx, pairs: &[String]) -> Result<()> {
    let pairs = split_pairs(pairs)?;
    let mut done = Map::new();
    for root in [Root::App, Root::Cli] {
        let mine: Vec<(String, String)> = pairs.iter().filter(|(k, _)| split_key(k).0 == root).cloned().collect();
        if mine.is_empty() {
            continue;
        }
        let doc = load(ctx, root).await?;
        // 최상위 키 단위로 머지되므로, 바뀐 최상위 키는 현재 값 전체에 덮어써서 보낸다
        let mut full = doc.clone();
        let mut touched: Vec<String> = Vec::new();
        for (key, raw) in &mine {
            let path = split_key(key).1;
            let current = get_path(&doc, path);
            if root == Root::App && current.is_none() {
                return Err(not_found(key, &doc, root));
            }
            crate::cmd::set_path(&mut full, path, coerce(raw, current)?);
            let top = path.split('.').next().unwrap_or("").to_string();
            if !touched.contains(&top) {
                touched.push(top);
            }
        }
        let patch: Map<String, Value> = touched
            .iter()
            .map(|k| (k.clone(), full.get(k).cloned().unwrap_or(Value::Null)))
            .collect();
        let r = save(ctx, root, Value::Object(patch)).await?;
        done.insert(if root == Root::App { "app" } else { "cli" }.into(), r);
    }
    let keys: Vec<&str> = pairs.iter().map(|(k, _)| k.as_str()).collect();
    ctx.out
        .done(&t!("config.saved", keys = keys.join(", ")), &Value::Object(done));
    if pairs.iter().any(|(k, _)| k == "ipcPort") {
        ctx.out.hint(&t!("config.restart_needed"));
    }
    Ok(())
}
