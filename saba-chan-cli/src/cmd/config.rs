//! `config` — 앱 설정 (config.json의 `gui` 칸 + CLI 전용 `cli` 칸)
//!
//! 키는 데몬이 쓰는 그대로(camelCase)다. `cli.`로 시작하면 CLI 전용 칸으로 간다.
//! 값의 형식은 지금 값의 형식을 따른다 (참거짓 칸에 "on"을 쓰면 true).
//! `reset`은 데몬이 기본값으로 되돌린다 (CLI 칸은 기본값이 없어 키를 지운다).

use crate::cli::ConfigCmd;
use crate::cmd::{coerce, config_patch, flatten, get_path, split_pairs};
use crate::ctx::Ctx;
use crate::error::{usage, Result};
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
                // revision은 저장 확인용 표식이라 설정 칸이 아니다
                rows.retain(|(k, _)| !k.is_empty() && k != "cli" && k != "revision");
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
            ctx.out.data(&v, |v| say!("{}", output::show(v)));
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
        ConfigCmd::Export { file, secrets } => export(ctx, &file, secrets).await,
        ConfigCmd::Import { file, dry_run, overwrite } => import(ctx, &file, dry_run, overwrite).await,
        ConfigCmd::Paths => paths(ctx).await,
        ConfigCmd::Move { kind, folder } => move_dir(ctx, kind, &folder).await,
        ConfigCmd::Browse { path, files } => browse(ctx, path.as_deref().unwrap_or(""), files.as_deref()).await,
        ConfigCmd::Autostart { mode } => autostart(ctx, mode).await,
    }
}

/// 폴더 위치와 기본값 (GUI 설정의 "폴더" 칸과 같은 값)
async fn paths(ctx: &Ctx) -> Result<()> {
    let r = ctx.api.paths().await?;
    ctx.out.data(&r, |r| {
        let s = |v: &Value| v.as_str().unwrap_or("-").to_string();
        let dir = |v: &Value| {
            let source = v["source"].as_str().unwrap_or("default");
            let mut line = format!("{}  ({})", s(&v["path"]), t!(&format!("config.paths.source_{}", source)));
            if source != "default" {
                line.push_str(&format!("  {}", t!("config.paths.default_is", path = s(&v["default"]))));
            }
            line
        };
        output::print_kv(&[
            (t!("config.paths.data"), s(&r["data_dir"])),
            (t!("config.paths.install"), s(&r["install_dir"])),
            (t!("config.paths.default_install"), s(&r["default_install_dir"])),
            (t!("config.paths.instances"), s(&r["instances_dir"])),
            (t!("config.paths.modules"), dir(&r["modules"])),
            (t!("config.paths.extensions"), dir(&r["extensions"])),
        ]);
    });
    Ok(())
}

/// 데몬 PC의 폴더 보기 (GUI 원격 경로 고르기와 같은 API). `--daemon`이면 그 PC의 폴더다
async fn browse(ctx: &Ctx, path: &str, files: Option<&str>) -> Result<()> {
    let r = ctx.api.browse(path, files).await.map_err(|e| {
        match e.downcast_ref::<crate::error::CliError>() {
            Some(crate::error::CliError::Api { code: Some(code), .. }) if code.starts_with("browse_") => {
                crate::error::failed(t!(&format!("config.browse.err_{code}")))
            }
            _ => e,
        }
    })?;
    ctx.out.data(&r, |r| {
        let s = |v: &Value| v.as_str().unwrap_or("").to_string();
        match r["path"].as_str() {
            Some(p) => say!("{}", p),
            None => say!("{}", t!("config.browse.roots")),
        }
        let entries = r["entries"].as_array().cloned().unwrap_or_default();
        if entries.is_empty() {
            say!("  {}", t!("config.browse.empty"));
        }
        for e in &entries {
            let suffix = if e["dir"] == true && r["path"].is_string() { s(&r["sep"]) } else { String::new() };
            say!("  {}{}", s(&e["name"]), suffix);
        }
        if r["truncated"] == true {
            say!("{}", t!("config.browse.truncated"));
        }
        if r["path"].is_null() && r["home"].is_string() {
            say!("{}", t!("config.browse.home", path = s(&r["home"])));
        }
    });
    Ok(())
}

async fn move_dir(ctx: &Ctx, kind: crate::cli::DirKind, folder: &std::path::Path) -> Result<()> {
    let target = daemon_path(ctx, folder)?;
    ctx.ask.ensure(&t!(&format!("config.confirm_move_{}", kind.as_str()), dir = target))?;
    let r = ctx.api.move_dir(kind.as_str(), &target).await?;
    let path = r["path"].as_str().unwrap_or(&target).to_string();
    ctx.out.done(&t!("config.moved", path = path), &r);
    let left: Vec<&str> = r["left_behind"].as_array().into_iter().flatten().filter_map(Value::as_str).collect();
    if !left.is_empty() {
        ctx.out.warn(&t!("config.move_left_behind", files = left.join(", ")));
    }
    Ok(())
}

/// 로그인할 때 자동 실행 (GUI 설정 > 일반과 같은 값)
async fn autostart(ctx: &Ctx, mode: Option<crate::cli::AutostartMode>) -> Result<()> {
    let r = match mode {
        Some(mode) => ctx.api.set_autostart(mode.as_str()).await?,
        None => ctx.api.autostart().await?,
    };
    let name = |r: &Value| t!(&format!("config.autostart.{}", r["mode"].as_str().unwrap_or("off")));
    if r["supported"] == false {
        ctx.out.data(&r, |_| say!("{}", t!("config.autostart.unsupported")));
    } else if mode.is_some() {
        ctx.out.done(&t!("config.autostart.set", mode = name(&r)), &r);
    } else {
        ctx.out.data(&r, |r| say!("{}", t!("config.autostart.current", mode = name(r))));
    }
    // 화면 없는 리눅스: 데몬을 systemd 사용자 서비스로 돌린다. 로그인 없이 부팅할 때 켜려면 linger가 필요하다
    if r["service"] == true {
        ctx.out.note(&t!("config.autostart.service"));
        if r["lingering"] == false {
            ctx.out.hint(&t!("config.autostart.linger_hint"));
        }
    }
    Ok(())
}

/// 데몬은 절대 경로만 받는다 (CLI와 같은 PC)
/// 데몬에 보낼 경로. 이 PC 데몬이면 지금 폴더 기준 전체 경로로, 원격 데몬이면 그 PC의 경로라 적은 그대로 보내고
/// 전체 경로만 받는다 (이 PC 기준으로 풀면 엉뚱한 곳을 가리킨다. 그 PC가 Windows인지 아닌지 여기서는 모른다)
pub(crate) fn daemon_path(ctx: &Ctx, file: &std::path::Path) -> Result<String> {
    if ctx.api.target.is_none() {
        return Ok(std::path::absolute(file).unwrap_or_else(|_| file.to_path_buf()).to_string_lossy().into_owned());
    }
    let raw = file.to_string_lossy().into_owned();
    if full_path(&raw) {
        Ok(raw)
    } else {
        Err(crate::error::failed(t!("config.browse.remote_full_path")))
    }
}

/// 어느 OS에서든 전체 경로인가: `/…`, `\\…`(공유 폴더), `X:\…`·`X:/…`
fn full_path(p: &str) -> bool {
    let b = p.as_bytes();
    p.starts_with('/') || p.starts_with("\\\\") || (b.len() >= 3 && b[0].is_ascii_alphabetic() && b[1] == b':' && (b[2] == b'\\' || b[2] == b'/'))
}

async fn export(ctx: &Ctx, file: &std::path::Path, secrets: bool) -> Result<()> {
    if secrets {
        ctx.ask.ensure(&t!("config.confirm_export_secrets"))?;
    }
    let path = daemon_path(ctx, file)?;
    let r = ctx.api.export_config(&path, secrets).await?;
    let n = r.get("instances").and_then(Value::as_array).map_or(0, Vec::len);
    ctx.out.done(&t!("config.exported", path = path, count = n), &r);
    if !secrets {
        ctx.out.hint(&t!("config.export_no_secrets"));
    }
    Ok(())
}

async fn import(ctx: &Ctx, file: &std::path::Path, dry_run: bool, overwrite: bool) -> Result<()> {
    let path = daemon_path(ctx, file)?;
    let r = ctx.api.import_config(&path, dry_run, overwrite).await?;
    let names = |key: &str| -> Vec<String> {
        r.pointer(&format!("/instances/{}", key))
            .and_then(Value::as_array)
            .map(|a| a.iter().map(|i| output::s(i, "name").to_string()).collect())
            .unwrap_or_default()
    };
    let (added, replaced) = (names("added"), names("replaced"));
    let msg = if dry_run { t!("config.import_preview") } else { t!("config.imported") };
    ctx.out.done(&msg, &r);
    if ctx.out.json {
        return Ok(());
    }
    let mut rows = Vec::new();
    if !added.is_empty() {
        rows.push((t!("config.import_added"), added.join(", ")));
    }
    if !replaced.is_empty() {
        rows.push((t!("config.import_replaced"), replaced.join(", ")));
    }
    let config: Vec<String> = r
        .get("config")
        .and_then(Value::as_array)
        .map(|a| a.iter().filter_map(Value::as_str).map(str::to_string).collect())
        .unwrap_or_default();
    if !config.is_empty() {
        rows.push((t!("config.import_settings"), config.join(", ")));
    }
    output::print_kv(&rows);
    for skip in r.pointer("/instances/skipped").and_then(Value::as_array).into_iter().flatten() {
        let reason = output::s(skip, "reason");
        let why = crate::i18n::try_t(&format!("config.skip_reason.{}", reason)).unwrap_or_else(|| reason.to_string());
        ctx.out.warn(&t!("config.import_skipped", name = output::s(skip, "name"), reason = why));
    }
    for m in r.get("missingModules").and_then(Value::as_array).into_iter().flatten() {
        ctx.out.warn(&t!("config.import_missing_module", module = output::show(m)));
    }
    if r.get("nodeToken").and_then(Value::as_str) == Some("imported") {
        ctx.out.warn(&t!("config.import_token_moved"));
    }
    if added.iter().chain(&replaced).next().is_some() {
        ctx.out.hint(&t!("config.import_move_files"));
    }
    Ok(())
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
    crate::cmd::unknown_setting(key, &names)
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
        let r = if root == Root::App {
            // 앱 설정은 고친 경로만 merge patch로, 읽은 때의 revision과 함께 보낸다
            let mut changes = Vec::new();
            for (key, raw) in &mine {
                let path = split_key(key).1;
                let current = get_path(&doc, path).filter(|_| path != "revision");
                let Some(current) = current else {
                    return Err(not_found(key, &doc, root));
                };
                changes.push((path.to_string(), coerce(raw, Some(current))?));
            }
            ctx.api.patch_app_config(config_patch(&doc, changes)).await?
        } else {
            // CLI 칸은 최상위 키 단위로 머지되므로, 바뀐 최상위 키는 현재 값 전체에 덮어써서 보낸다
            let mut full = doc.clone();
            let mut touched: Vec<String> = Vec::new();
            for (key, raw) in &mine {
                let path = split_key(key).1;
                crate::cmd::set_path(&mut full, path, coerce(raw, get_path(&doc, path))?);
                let top = path.split('.').next().unwrap_or("").to_string();
                if !touched.contains(&top) {
                    touched.push(top);
                }
            }
            let patch: Map<String, Value> = touched
                .iter()
                .map(|k| (k.clone(), full.get(k).cloned().unwrap_or(Value::Null)))
                .collect();
            save(ctx, root, Value::Object(patch)).await?
        };
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

#[cfg(test)]
mod tests {
    /// 원격 데몬에는 그 PC의 전체 경로만 보낸다: Windows·리눅스 어느 쪽 경로든 알아본다 (D2)
    #[test]
    fn a_remote_path_must_be_full_on_either_os() {
        for p in ["/srv/game", "C:\\Games\\a.zip", "d:/saba", "\\\\nas\\share"] {
            assert!(super::full_path(p), "{p}");
        }
        for p in ["a.zip", "games/x", "C:relative", "~/x", ""] {
            assert!(!super::full_path(p), "{p}");
        }
    }
}
