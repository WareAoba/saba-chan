//! `extension` — 익스텐션 목록·켜기·끄기·설치·업데이트·설정

use crate::cli::ExtCmd;
use crate::cmd::{guess, split_pairs};
use crate::ctx::Ctx;
use crate::error::{CliError, Result};
use crate::output::{self, s};
use crate::resolve;
use serde_json::{json, Map, Value};

pub async fn run(ctx: &Ctx, cmd: ExtCmd) -> Result<()> {
    match cmd {
        ExtCmd::List => list(ctx).await,
        ExtCmd::Info { extension } => info(ctx, extension.as_deref()).await,
        ExtCmd::Enable { extension } => {
            let e = resolve::extension(ctx, extension.as_deref()).await?;
            let id = s(&e, "id").to_string();
            let r = ctx.api.enable_extension(&id).await?;
            ctx.out.done(&t!("ext.enabled", id = id), &r);
            super::print_trust_warnings(ctx, &r);
            Ok(())
        }
        ExtCmd::Disable { extension } => {
            let e = resolve::extension(ctx, extension.as_deref()).await?;
            let id = s(&e, "id").to_string();
            let r = ctx.api.disable_extension(&id).await?;
            ctx.out.done(&t!("ext.disabled", id = id), &r);
            Ok(())
        }
        ExtCmd::Available => available(ctx).await,
        ExtCmd::Install { ids, url, sha256 } => install_many(ctx, &ids, url, sha256).await,
        ExtCmd::Update { extension } => update(ctx, extension.as_deref()).await,
        ExtCmd::Remove { extension } => {
            let e = resolve::extension(ctx, extension.as_deref()).await?;
            let id = s(&e, "id").to_string();
            ctx.ask.ensure(&t!("ext.confirm_remove", id = id))?;
            let r = ctx.api.remove_extension(&id).await?;
            ctx.out.done(&t!("ext.removed", id = id), &r);
            Ok(())
        }
        ExtCmd::Config { extension, pairs } => config(ctx, extension.as_deref(), &pairs).await,
        ExtCmd::Audit { extension } => {
            let e = resolve::extension(ctx, extension.as_deref()).await?;
            let spin = crate::progress::spinner(&t!("audit.scanning", name = s(&e, "id")));
            let r = ctx.api.extension_audit(s(&e, "id")).await;
            spin.finish_and_clear();
            crate::cmd::print_audit(ctx, &resolve::extension_label(&e), &r?);
            Ok(())
        }
        ExtCmd::Rescan => {
            let r = ctx.api.rescan_extensions().await?;
            ctx.out.done(&t!("ext.rescanned"), &r);
            Ok(())
        }
        ExtCmd::Status => {
            let r = ctx.api.extension_init_status().await?;
            ctx.out.data(&r, |r| {
                // {initializing, in_progress: {id: 단계}, completed: [...]}
                let busy: Vec<String> = r
                    .get("in_progress")
                    .and_then(Value::as_object)
                    .into_iter()
                    .flatten()
                    .map(|(id, v)| format!("{} ({})", id, output::show(v)))
                    .collect();
                if busy.is_empty() {
                    say!("{}", output::dim(&t!("ext.status_idle")));
                } else {
                    say!("{}", t!("ext.status_busy", list = busy.join(", ")));
                }
                if let Some(done) = r
                    .get("completed")
                    .filter(|d| d.as_array().is_some_and(|a| !a.is_empty()))
                {
                    say!("{}", output::dim(&t!("ext.status_done", list = output::show(done))));
                }
            });
            Ok(())
        }
        ExtCmd::Mount { id } => {
            let r = ctx.api.mount_extension(&id).await?;
            ctx.out.done(&t!("ext.mounted", id = id), &r);
            Ok(())
        }
        ExtCmd::Unmount { id } => {
            let r = ctx.api.unmount_extension(&id).await?;
            ctx.out.done(&t!("ext.unmounted", id = id), &r);
            Ok(())
        }
    }
}

async fn list(ctx: &Ctx) -> Result<()> {
    let r = ctx.api.extensions().await?;
    ctx.out.data(&r, |r| {
        let list = r
            .get("extensions")
            .and_then(Value::as_array)
            .cloned()
            .unwrap_or_default();
        if list.is_empty() {
            say!("{}", output::dim(&t!("ext.none")));
        } else {
            let mut t = output::table(&[
                t!("col.name"),
                t!("col.version"),
                t!("col.state"),
                t!("col.trust"),
                t!("col.what"),
            ]);
            for e in &list {
                let on = e.get("enabled").and_then(Value::as_bool) == Some(true);
                let state = if !s(e, "incompatible").is_empty() {
                    console::style(t!("ext.incompatible")).red().to_string()
                } else if on {
                    console::style(t!("ext.on")).green().to_string()
                } else {
                    output::dim(&t!("ext.off"))
                };
                t.add_row(vec![
                    resolve::extension_label(e),
                    s(e, "version").to_string(),
                    state,
                    output::trust_badge(s(e, "trust")),
                    s(e, "description").to_string(),
                ]);
            }
            output::print_table(t);
        }
        for e in &list {
            for w in e
                .get("warnings")
                .and_then(Value::as_array)
                .into_iter()
                .flatten()
                .filter_map(Value::as_str)
            {
                ctx.out.warn(&format!("{}: {}", s(e, "id"), w));
            }
        }
        for f in r.get("failed").and_then(Value::as_array).into_iter().flatten() {
            let what = if f.is_string() {
                output::show(f)
            } else {
                format!("{}: {}", s(f, "id"), s(f, "error"))
            };
            ctx.out.warn(&t!("ext.load_failed", what = what));
        }
    });
    Ok(())
}

async fn info(ctx: &Ctx, q: Option<&str>) -> Result<()> {
    let e = resolve::extension(ctx, q).await?;
    let id = s(&e, "id").to_string();
    let cfg = ctx.api.extension_config(&id).await.ok();
    if ctx.out.json {
        let mut v = e.clone();
        v["config"] = cfg.and_then(|c| c.get("config").cloned()).unwrap_or(Value::Null);
        ctx.out.data(&v, |_| {});
        return Ok(());
    }
    output::heading(&resolve::extension_label(&e));
    let mut rows = vec![
        (t!("col.version"), s(&e, "version").to_string()),
        (
            t!("col.state"),
            if e.get("enabled").and_then(Value::as_bool) == Some(true) {
                t!("ext.on")
            } else {
                t!("ext.off")
            },
        ),
        (t!("ext.author"), s(&e, "author").to_string()),
        (t!("col.what"), s(&e, "description").to_string()),
    ];
    if !s(&e, "trust").is_empty() {
        rows.push((t!("col.trust"), output::trust_badge(s(&e, "trust"))));
    }
    let hooks: Vec<&str> = e
        .get("hooks")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .filter_map(Value::as_str)
        .collect();
    if !hooks.is_empty() {
        rows.push((t!("ext.hooks"), hooks.join(", ")));
    }
    if let Some(deps) = e.get("dependencies").filter(|d| match d {
        Value::Array(a) => !a.is_empty(),
        Value::Object(o) => !o.is_empty(),
        Value::Null => false,
        _ => true,
    }) {
        rows.push((t!("ext.dependencies"), output::show(deps)));
    }
    if !s(&e, "incompatible").is_empty() {
        rows.push((t!("ext.incompatible"), s(&e, "incompatible").to_string()));
    }
    output::print_kv(&rows);
    let values = cfg
        .as_ref()
        .and_then(|c| c.get("config"))
        .cloned()
        .unwrap_or(Value::Null);
    print_config(&e, &values);
    Ok(())
}

/// 설정 칸(config_fields)과 저장된 값을 같이 보인다
/// 설정 칸이 하나도 없으면 false
fn print_config(ext: &Value, values: &Value) -> bool {
    let fields = ext
        .get("config_fields")
        .and_then(Value::as_object)
        .cloned()
        .unwrap_or_default();
    let mut keys: Vec<String> = fields.keys().cloned().collect();
    if let Some(map) = values.as_object() {
        keys.extend(map.keys().filter(|k| !fields.contains_key(*k)).cloned());
    }
    if keys.is_empty() {
        return false;
    }
    say!();
    output::heading(&t!("ext.config"));
    let rows: Vec<(String, String)> = keys
        .iter()
        .map(|k| {
            let v = values
                .get(k)
                .or_else(|| fields.get(k).and_then(|f| f.get("default")))
                .cloned()
                .unwrap_or(Value::Null);
            let kind = fields.get(k).map(|f| s(f, "type").to_string()).unwrap_or_default();
            let label = if kind.is_empty() {
                k.clone()
            } else {
                format!("{} {}", k, output::dim(&format!("({})", kind)))
            };
            (label, output::mask(k, &v))
        })
        .collect();
    output::print_kv(&rows);
    true
}

async fn available(ctx: &Ctx) -> Result<()> {
    let spin = crate::progress::spinner(&t!("common.fetching"));
    let (r, installed) = tokio::join!(ctx.api.extension_manifest(), resolve::extensions(ctx));
    spin.finish_and_clear();
    let r = r?;
    let installed = installed.unwrap_or_default();
    ctx.out.data(&r, |r| {
        let list = crate::cmd::module::entries(r.get("extensions"));
        let updates: Vec<String> = r
            .get("updates")
            .and_then(Value::as_array)
            .into_iter()
            .flatten()
            .map(|u| s(u, "id").to_string())
            .collect();
        let mut t = output::table(&[t!("col.name"), t!("col.version"), t!("col.what"), t!("col.state")]);
        for (id, e) in &list {
            let state = if updates.contains(id) {
                console::style(t!("common.update_available")).yellow().to_string()
            } else if installed.iter().any(|x| s(x, "id") == id) {
                output::dim(&t!("common.installed"))
            } else {
                String::new()
            };
            t.add_row(vec![
                id.clone(),
                s(e, "version").to_string(),
                s(e, "description").to_string(),
                state,
            ]);
        }
        output::print_table(t);
        say!();
        say!("{}", output::dim("saba-chan-cli extension install <ID>"));
    });
    Ok(())
}

async fn install_many(ctx: &Ctx, ids: &[String], url: Option<String>, sha256: Option<String>) -> Result<()> {
    if ids.len() == 1 {
        return install(ctx, &ids[0], url, sha256).await.map(|_| ());
    }
    if url.is_some() || sha256.is_some() {
        return Err(crate::error::usage(t!("ext.url_one_id")));
    }
    // 하나씩 설치하고, 실패하면 거기서 멈춘다 (앞의 것은 이미 설치됨)
    let mut results = Vec::new();
    for id in ids {
        results.push(install(ctx, id, None, None).await?);
    }
    ctx.out.data(&json!({ "results": results }), |_| {});
    Ok(())
}

async fn install(ctx: &Ctx, id: &str, url: Option<String>, sha256: Option<String>) -> Result<Value> {
    let mut body = Map::new();
    if let Some(u) = url {
        body.insert("download_url".into(), Value::String(u));
    }
    if let Some(h) = sha256 {
        body.insert("sha256".into(), Value::String(h));
    }
    let spin = crate::progress::spinner(&t!("ext.installing", id = id));
    let r = ctx.api.install_extension(id, Value::Object(body)).await;
    spin.finish_and_clear();
    let r = r?;
    let installed: Vec<&str> = r
        .get("installed")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .filter_map(Value::as_str)
        .collect();
    let msg = if installed.len() > 1 {
        t!("ext.installed_with", id = id, all = installed.join(", "))
    } else {
        t!("ext.installed", id = id)
    };
    ctx.out.done(&msg, &r);
    if !ctx.out.json {
        ctx.out.hint(&format!("saba-chan-cli extension enable {}", id));
    }
    Ok(r)
}

async fn update(ctx: &Ctx, q: Option<&str>) -> Result<()> {
    let spin = crate::progress::spinner(&t!("common.fetching"));
    let r = ctx.api.extension_updates().await;
    spin.finish_and_clear();
    let r = r?;
    let mut updates = r.get("updates").and_then(Value::as_array).cloned().unwrap_or_default();
    if let Some(q) = q {
        let e = resolve::extension(ctx, Some(q)).await?;
        updates.retain(|u| s(u, "id") == s(&e, "id"));
    }
    if updates.is_empty() {
        ctx.out.done(&t!("ext.up_to_date"), &json!({ "updated": [] }));
        return Ok(());
    }
    if !ctx.out.json {
        for u in &updates {
            say!(
                "  {}  {} → {}",
                s(u, "id"),
                s(u, "installed_version"),
                console::style(s(u, "latest_version")).green()
            );
        }
    }
    ctx.ask.ensure(&t!("ext.confirm_update", n = updates.len()))?;
    let mut done = Vec::new();
    let mut failures = Vec::new();
    for u in &updates {
        let id = s(u, "id");
        if !s(u, "incompatible").is_empty() {
            ctx.out
                .warn(&t!("ext.skip_incompatible", id = id, why = s(u, "incompatible")));
            continue;
        }
        let spin = crate::progress::spinner(&t!("ext.installing", id = id));
        // 주소·해시는 데몬이 서명된 카탈로그에서 고른다 (주소만 보내면 해시 검사와 공식 표시가 빠진다)
        let r = ctx.api.install_extension(id, json!({})).await;
        spin.finish_and_clear();
        match r {
            Ok(_) => {
                if !ctx.out.json {
                    say!(
                        "{} {}",
                        console::style("✓").green().bold(),
                        t!("ext.updated", id = id, version = s(u, "latest_version"))
                    );
                }
                done.push(id.to_string());
            }
            Err(e) => {
                let err = crate::error::classify(&e);
                if !ctx.out.json {
                    ctx.out.error(&err);
                }
                failures.push(json!({ "id": id, "error": err.to_string(), "error_code": err.code() }));
            }
        }
    }
    if !failures.is_empty() {
        return Err(CliError::Partial {
            message: t!("ext.update_failed", n = failures.len()),
            data: json!({ "updated": done, "failed": failures }),
        }
        .into());
    }
    if ctx.out.json {
        ctx.out.data(&json!({ "updated": done }), |_| {});
    }
    Ok(())
}

async fn config(ctx: &Ctx, q: Option<&str>, pairs: &[String]) -> Result<()> {
    let e = resolve::extension(ctx, q).await?;
    let id = s(&e, "id").to_string();
    if pairs.is_empty() {
        let r = ctx.api.extension_config(&id).await?;
        let values = r.get("config").cloned().unwrap_or(Value::Null);
        ctx.out.data(&values, |v| {
            if !print_config(&e, v) {
                say!("{}", output::dim(&t!("ext.no_config", id = id)));
            }
        });
        return Ok(());
    }
    let fields = e
        .get("config_fields")
        .and_then(Value::as_object)
        .cloned()
        .unwrap_or_default();
    let mut patch = Map::new();
    for (k, raw) in split_pairs(pairs)? {
        let v = if raw.is_empty() {
            Value::Null
        } else {
            match fields.get(&k).map(|f| s(f, "type")) {
                Some(kind) if !kind.is_empty() => crate::cmd::typed(&raw, kind)?,
                _ => guess(&raw),
            }
        };
        patch.insert(k, v);
    }
    let r = ctx.api.save_extension_config(&id, Value::Object(patch)).await?;
    ctx.out.done(&t!("ext.config_saved", id = id), &r);
    Ok(())
}
