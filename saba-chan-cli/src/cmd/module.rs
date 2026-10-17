//! `module …` — 게임 모듈

use crate::cli::ModuleCmd;
use crate::ctx::Ctx;
use crate::error::Result;
use crate::output::{self, s, show};
use crate::resolve;
use serde_json::{json, Value};

pub async fn run(ctx: &Ctx, cmd: ModuleCmd) -> Result<()> {
    match cmd {
        ModuleCmd::List => list(ctx).await,
        ModuleCmd::Info { module } => info(ctx, module.as_deref()).await,
        ModuleCmd::Commands { module } => {
            let m = resolve::module(ctx, module.as_deref()).await?;
            let cat = crate::cmd::server::catalog_for(ctx, s(&m, "name")).await?;
            crate::cmd::server::print_commands(ctx, s(&m, "name"), &cat)
        }
        ModuleCmd::Versions {
            module,
            version,
            snapshots,
            page,
            per_page,
        } => versions(ctx, module.as_deref(), version.as_deref(), snapshots, page, per_page).await,
        ModuleCmd::Available => available(ctx).await,
        ModuleCmd::Install { id } => {
            let spin = crate::progress::spinner(&t!("module.installing", id = id));
            let r = ctx.api.install_module(&id).await;
            spin.finish_and_clear();
            let r = r?;
            ctx.out.done(&t!("module.installed", id = id), &r);
            Ok(())
        }
        ModuleCmd::Remove { id } => {
            let m = resolve::module(ctx, Some(&id)).await?;
            let name = s(&m, "name").to_string();
            let used: Vec<String> = resolve::servers(ctx)
                .await?
                .into_iter()
                .filter(|sv| sv.module == name)
                .map(|sv| sv.name)
                .collect();
            let q = if used.is_empty() {
                t!("module.confirm_remove", id = name)
            } else {
                t!("module.confirm_remove_used", id = name, servers = used.join(", "))
            };
            ctx.ask.ensure(&q)?;
            let r = ctx.api.remove_module(&name).await?;
            ctx.out.done(&t!("module.removed", id = name), &r);
            Ok(())
        }
        ModuleCmd::Refresh => {
            let r = ctx.api.refresh_modules().await?;
            ctx.out.done(&t!("module.refreshed"), &r);
            Ok(())
        }
        ModuleCmd::Detect { dir } => {
            let r = ctx.api.detect(&dir).await?;
            ctx.out.data(&r, |r| {
                let m = r.get("matches").and_then(Value::as_array).cloned().unwrap_or_default();
                if m.is_empty() {
                    println!("{}", t!("module.detect_none", dir = dir));
                }
                for x in m {
                    println!("{} {}", console::style("✓").green(), resolve::module_label(&x));
                }
            });
            Ok(())
        }
        ModuleCmd::Schema => {
            let r = ctx.api.module_schema().await?;
            println!("{}", serde_json::to_string_pretty(&r).unwrap_or_default());
            Ok(())
        }
        ModuleCmd::InstallFiles {
            module,
            version,
            dir,
            jar,
            accept_eula,
        } => {
            let m = resolve::module(ctx, Some(&module)).await?;
            let mut body = json!({ "version": version, "install_dir": dir, "accept_eula": accept_eula });
            if let Some(j) = jar {
                body["jar_name"] = json!(j);
            }
            let spin = crate::progress::spinner(&t!("module.downloading_files"));
            let r = ctx.api.install_server_files(s(&m, "name"), body).await;
            spin.finish_and_clear();
            let r = r?;
            ctx.out.done(&t!("module.files_installed", dir = dir), &r);
            Ok(())
        }
    }
}

async fn list(ctx: &Ctx) -> Result<()> {
    let r = ctx.api.modules().await?;
    ctx.out.data(&r, |r| {
        let mods = r.get("modules").and_then(Value::as_array).cloned().unwrap_or_default();
        if mods.is_empty() {
            println!("{}", output::dim(&t!("module.none")));
        } else {
            let mut t = output::table(&[t!("col.name"), t!("col.game"), t!("col.version"), t!("col.aliases")]);
            for m in &mods {
                let mut ver = s(m, "version").to_string();
                if m.get("warnings")
                    .and_then(Value::as_array)
                    .is_some_and(|w| !w.is_empty())
                    || m.get("unmet_dependencies")
                        .and_then(Value::as_array)
                        .is_some_and(|w| !w.is_empty())
                {
                    ver.push_str(&console::style(" !").yellow().to_string());
                }
                let aliases = m
                    .get("module_aliases")
                    .and_then(Value::as_array)
                    .map(|a| a.iter().filter_map(Value::as_str).collect::<Vec<_>>().join(", "))
                    .unwrap_or_default();
                t.add_row(vec![
                    s(m, "name").to_string(),
                    s(m, "display_name").to_string(),
                    ver,
                    output::dim(&aliases),
                ]);
            }
            output::print_table(t);
        }
        for f in r.get("failed").and_then(Value::as_array).into_iter().flatten() {
            let what = if s(f, "module").is_empty() {
                s(f, "path")
            } else {
                s(f, "module")
            };
            ctx.out.warn(&t!("module.failed", path = what, error = s(f, "error")));
            if let Some(h) = crate::error::api_hint(s(f, "error_code"), f) {
                ctx.out.hint(&h);
            }
        }
    });
    Ok(())
}

async fn info(ctx: &Ctx, q: Option<&str>) -> Result<()> {
    let m = resolve::module(ctx, q).await?;
    ctx.out.data(&m, |m| {
        output::heading(&resolve::module_label(m));
        let desc = s(m, "description");
        if !desc.is_empty() {
            println!("{}", desc);
        }
        let mut rows = vec![(t!("col.version"), s(m, "version").to_string())];
        for (k, label) in [
            ("game_name", "col.game"),
            ("author", "col.author"),
            ("interaction_mode", "col.mode"),
            ("path", "col.folder"),
        ] {
            if !s(m, k).is_empty() {
                rows.push((t!(label), s(m, k).to_string()));
            }
        }
        if let Some(i) = m.get("install").filter(|i| !i.is_null()) {
            let mut line = s(i, "method").to_string();
            if let Some(req) = i
                .get("requires_extensions")
                .and_then(Value::as_array)
                .filter(|r| !r.is_empty())
            {
                line.push_str(&output::dim(&format!(
                    "  ({})",
                    t!(
                        "module.needs",
                        ids = req.iter().filter_map(Value::as_str).collect::<Vec<_>>().join(", ")
                    )
                )));
            }
            rows.push((t!("col.install"), line));
        }
        if let Some(p) = m.get("protocols").filter(|p| !p.is_null()) {
            rows.push((t!("col.protocols"), p.get("supported").map(show).unwrap_or_default()));
        }
        let ncmd = m
            .pointer("/commands/fields")
            .and_then(Value::as_array)
            .map(Vec::len)
            .unwrap_or(0);
        rows.push((t!("col.commands"), ncmd.to_string()));
        output::print_kv(&rows);
        for w in m.get("warnings").and_then(Value::as_array).into_iter().flatten() {
            ctx.out.warn(&show(w));
        }
        for d in m
            .get("unmet_dependencies")
            .and_then(Value::as_array)
            .into_iter()
            .flatten()
        {
            ctx.out.warn(&t!(
                "module.unmet",
                component = s(d, "component"),
                required = s(d, "required"),
                installed = d.get("installed").map(show).unwrap_or_default()
            ));
        }
    });
    Ok(())
}

async fn versions(
    ctx: &Ctx,
    q: Option<&str>,
    version: Option<&str>,
    snapshots: bool,
    page: u32,
    per_page: u32,
) -> Result<()> {
    let m = resolve::module(ctx, q).await?;
    let name = s(&m, "name");
    let spin = crate::progress::spinner(&t!("module.fetching_versions"));
    let r = match version {
        Some(v) => ctx.api.module_version(name, v).await,
        None => ctx.api.module_versions(name, snapshots, page, per_page).await,
    };
    spin.finish_and_clear();
    let r = r?;
    ctx.out.data(&r, |r| {
        if version.is_some() {
            return crate::cmd::server::print_value(r);
        }
        if let Some(latest) = r.get("latest").filter(|l| !l.is_null()) {
            println!(
                "{} {}",
                output::dim(&t!("module.latest")),
                show(latest.get("release").unwrap_or(latest))
            );
        }
        let list = r.get("versions").and_then(Value::as_array).cloned().unwrap_or_default();
        for v in list {
            match v.as_str() {
                Some(id) => println!("{}", id),
                None => println!("{}  {}", s(&v, "id"), output::dim(s(&v, "type"))),
            }
        }
    });
    Ok(())
}

/// 원격 목록: `{id: {...}}` 맵이든 배열이든 (id, 항목)으로
pub fn entries(v: Option<&Value>) -> Vec<(String, Value)> {
    match v {
        Some(Value::Object(map)) => map.iter().map(|(k, v)| (k.clone(), v.clone())).collect(),
        Some(Value::Array(a)) => a.iter().map(|x| (s(x, "id").to_string(), x.clone())).collect(),
        _ => vec![],
    }
}

async fn available(ctx: &Ctx) -> Result<()> {
    let spin = crate::progress::spinner(&t!("common.fetching"));
    let (r, installed) = tokio::join!(ctx.api.module_manifest(), resolve::modules(ctx));
    spin.finish_and_clear();
    let r = r?;
    let installed = installed.unwrap_or_default();
    let manifest = r.get("manifest").cloned().unwrap_or(Value::Null);
    ctx.out.data(&manifest, |m| {
        let list = entries(m.get("modules").or(Some(m)));
        let mut t = output::table(&[t!("col.name"), t!("col.version"), t!("col.what"), t!("col.state")]);
        for (id, e) in list.iter().filter(|(_, e)| e.is_object()) {
            let have = installed.iter().find(|x| s(x, "name") == id);
            let state = match have {
                Some(h) if s(h, "version") != s(e, "version") => {
                    console::style(t!("common.update_available")).yellow().to_string()
                }
                Some(_) => output::dim(&t!("common.installed")),
                None => String::new(),
            };
            t.add_row(vec![
                id.clone(),
                s(e, "version").to_string(),
                s(e, "description").to_string(),
                state,
            ]);
        }
        output::print_table(t);
        println!();
        println!("{}", output::dim("saba-chan-cli module install <NAME>"));
    });
    Ok(())
}
