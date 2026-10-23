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
            let m = resolve::module_exact(ctx, Some(&id)).await?;
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
        ModuleCmd::Audit { module } => {
            let m = resolve::module(ctx, module.as_deref()).await?;
            let name = s(&m, "name").to_string();
            let spin = crate::progress::spinner(&t!("audit.scanning", name = name));
            let r = ctx.api.module_audit(&name).await;
            spin.finish_and_clear();
            crate::cmd::print_audit(ctx, &resolve::module_label(&m), &r?);
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
                    say!("{}", t!("module.detect_none", dir = dir));
                }
                for x in m {
                    say!("{} {}", console::style("✓").green(), resolve::module_label(&x));
                }
            });
            Ok(())
        }
        ModuleCmd::Schema => {
            let r = ctx.api.module_schema().await?;
            say!("{}", serde_json::to_string_pretty(&r).unwrap_or_default());
            Ok(())
        }
        ModuleCmd::InstallFiles {
            module,
            version,
            dir,
            executable,
            accept,
        } => {
            let m = resolve::module(ctx, Some(&module)).await?;
            let body = install_files_body(&version, &dir, executable.as_deref(), &accept);
            let spin = crate::progress::spinner(&t!("module.downloading_files"));
            let r = ctx.api.install_server_files(s(&m, "name"), body).await;
            spin.finish_and_clear();
            let r = r?;
            ctx.out.done(&t!("module.files_installed", dir = dir), &r);
            Ok(())
        }
    }
}

/// `POST /api/module/:name/install` 본문. 실행 파일 이름을 주지 않으면 모듈 `server_executable`로 받는다
fn install_files_body(version: &str, dir: &str, executable: Option<&str>, accept: &[String]) -> Value {
    let mut body = json!({ "version": version, "install_dir": dir, "accept_agreements": accept });
    if let Some(e) = executable {
        body["server_executable"] = json!(e);
    }
    body
}

async fn list(ctx: &Ctx) -> Result<()> {
    let r = ctx.api.modules().await?;
    ctx.out.data(&r, |r| {
        let mods = r.get("modules").and_then(Value::as_array).cloned().unwrap_or_default();
        if mods.is_empty() {
            say!("{}", output::dim(&t!("module.none")));
        } else {
            let mut t = output::table(&[
                t!("col.name"),
                t!("col.game"),
                t!("col.version"),
                t!("col.trust"),
                t!("col.aliases"),
            ]);
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
                    output::trust_badge(s(m, "trust")),
                    output::dim(&aliases),
                ]);
            }
            output::print_table(t);
        }
        for f in r.get("failed").and_then(Value::as_array).into_iter().flatten() {
            let lines = failed_lines(f);
            ctx.out.warn(&lines[0]);
            for problem in &lines[1..] {
                say!("  · {}", problem);
            }
            if let Some(h) = crate::error::api_hint(s(f, "error_code"), f) {
                ctx.out.hint(&h);
            }
        }
    });
    Ok(())
}

/// 읽지 못한 모듈 한 건: 첫 줄은 무엇을 왜, 나머지는 스키마에서 틀린 곳 (06 §7)
fn failed_lines(f: &Value) -> Vec<String> {
    let folder = || {
        let path = s(f, "path").trim_end_matches(['/', '\\']);
        path.rsplit(['/', '\\']).next().unwrap_or(path).to_string()
    };
    let module = if s(f, "module").is_empty() {
        folder()
    } else {
        s(f, "module").to_string()
    };
    match s(f, "error_code") {
        "module_schema_invalid" => std::iter::once(t!("module.load_failed.invalid", module = module))
            .chain(
                f.get("problems")
                    .and_then(Value::as_array)
                    .into_iter()
                    .flatten()
                    .filter_map(Value::as_str)
                    .map(str::to_string),
            )
            .collect(),
        _ if crate::error::update_required(f, &module).is_some() => crate::error::update_required(f, &module).into_iter().collect(),
        _ => {
            let what = if s(f, "module").is_empty() {
                s(f, "path")
            } else {
                s(f, "module")
            };
            vec![t!("module.failed", path = what, error = s(f, "error"))]
        }
    }
}

async fn info(ctx: &Ctx, q: Option<&str>) -> Result<()> {
    let m = resolve::module(ctx, q).await?;
    ctx.out.data(&m, |m| {
        output::heading(&resolve::module_label(m));
        let desc = s(m, "description");
        if !desc.is_empty() {
            say!("{}", desc);
        }
        let mut rows = vec![(t!("col.version"), s(m, "version").to_string())];
        if !s(m, "trust").is_empty() {
            rows.push((t!("col.trust"), output::trust_badge(s(m, "trust"))));
        }
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
        let presets: Vec<&str> = m.get("presets").and_then(Value::as_array).into_iter().flatten().map(|p| s(p, "id")).collect();
        if !presets.is_empty() {
            rows.push((t!("col.presets"), presets.join(", ")));
        }
        let missing = unsupported(m);
        if !missing.is_empty() {
            rows.push((t!("col.unsupported"), missing.join(", ")));
        }
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
            say!(
                "{} {}",
                output::dim(&t!("module.latest")),
                show(latest.get("release").unwrap_or(latest))
            );
        }
        let list = r.get("versions").and_then(Value::as_array).cloned().unwrap_or_default();
        for v in list {
            match v.as_str() {
                Some(id) => say!("{}", id),
                None => say!("{}  {}", s(&v, "id"), output::dim(s(&v, "type"))),
            }
        }
    });
    Ok(())
}

/// 모듈이 구현하지 않아도 되는 lifecycle 함수와 그 함수를 쓰는 CLI 명령 (데몬 `plugin::OPTIONAL_MODULE_FUNCTIONS`, MK1).
/// `import_settings`는 CLI 명령이 없다
const OPTIONAL: &[(&str, &str)] = &[
    ("reset_server", "server reset"),
    ("list_versions", "module versions"),
    ("get_installed_version", "server version"),
    ("diagnose_log", "server diagnose"),
    ("read_properties", "server properties"),
    ("configure", "server properties set"),
    ("reset_properties", "server properties reset"),
    ("install_server", "module install-files"),
    ("validate", "server validate"),
];

/// 모듈이 이 lifecycle 함수를 구현하는지. `functions`가 null이거나 없으면(옛 데몬·옛 모듈) 안다고 본다
pub fn supported(module: &Value, function: &str) -> bool {
    match module.get("functions").and_then(Value::as_array) {
        Some(list) => list.iter().any(|f| f.as_str() == Some(function)),
        None => true,
    }
}

/// 이 모듈로는 쓸 수 없는 CLI 명령
pub fn unsupported(module: &Value) -> Vec<&'static str> {
    OPTIONAL.iter().filter(|(f, _)| !supported(module, f)).map(|(_, cmd)| *cmd).collect()
}

/// 확인을 묻기 전에, 모듈이 구현하지 않은 기능이면 데몬과 같은 `not_supported`로 멈춘다
pub fn ensure_supported(module: &Value, function: &str) -> Result<()> {
    if supported(module, function) {
        return Ok(());
    }
    Err(crate::error::CliError::Api {
        status: 200,
        code: Some("not_supported".into()),
        message: t!("err.not_supported", module = resolve::module_label(module)),
        body: json!({ "success": false, "error_code": "not_supported", "function": function }),
    }
    .into())
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
        say!();
        say!("{}", output::dim("saba-chan-cli module install <NAME>"));
    });
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use clap::Parser;

    /// 스키마에 맞지 않거나 너무 새 스키마라 읽지 못한 모듈은 무엇이 틀렸는지·필요한 버전을 보인다 (06 §7)
    #[test]
    fn a_module_that_does_not_match_its_schema_lists_the_problems() {
        crate::i18n::init("ko");
        let invalid = json!({ "path": "/m/palworld", "error": "x", "error_code": "module_schema_invalid", "problems": ["settings.fields[2].type: unknown type 'list'", "unknown key 'restart_required'"] });
        let lines = failed_lines(&invalid);
        assert_eq!(lines[0], t!("module.load_failed.invalid", module = "palworld"));
        assert_eq!(
            &lines[1..],
            [
                "settings.fields[2].type: unknown type 'list'",
                "unknown key 'restart_required'"
            ]
        );
        let newer = json!({ "path": "/m/ark", "error": "x", "error_code": "module_schema_unsupported", "required": 2 });
        assert_eq!(failed_lines(&newer), vec![t!("update_required.module_newer", name = "ark")]);
        let core = json!({ "path": "/m/pal", "error": "x", "error_code": "module_dependency_unsatisfied", "module": "palworld",
            "missing_dependencies": ["saba-core >=0.1.10 (installed 0.1.9)"] });
        assert_eq!(failed_lines(&core), vec![t!("update_required.module", name = "palworld", version = "0.1.10")]);
        let other = json!({ "path": "/m/zz", "error": "boom" });
        assert_eq!(
            failed_lines(&other),
            vec![t!("module.failed", path = "/m/zz", error = "boom")]
        );
    }

    /// 모듈이 밝힌 함수 목록(MK1)에 없는 선택 함수의 명령만 지원하지 않음으로 보인다. 목록을 모르면(null·없음) 아무것도 숨기지 않는다
    #[test]
    fn only_commands_of_unlisted_optional_functions_are_unsupported() {
        assert!(unsupported(&json!({ "functions": null })).is_empty());
        assert!(unsupported(&json!({})).is_empty());
        let m = json!({ "functions": ["start", "stop", "validate", "list_versions", "get_version_details", "get_installed_version", "read_properties", "configure", "reset_properties", "install_server", "import_settings"] });
        assert_eq!(unsupported(&m), vec!["server reset", "server diagnose"]);
        assert!(supported(&m, "validate") && !supported(&m, "reset_server"));
        assert!(supported(&json!({ "functions": null }), "reset_server"));
    }

    /// 받을 실행 파일 이름은 게임 중립적인 `--executable`로 받아 `server_executable`로 보낸다(MK7). 예전 `--jar`도 받는다
    #[test]
    fn install_files_sends_the_executable_name_as_server_executable() {
        for flag in ["--executable", "--jar"] {
            let cli = crate::cli::Cli::try_parse_from(["saba-chan-cli", "module", "install-files", "m", "--version", "1", "--dir", "d", flag, "x.jar"]).unwrap();
            let Some(crate::cli::Cmd::Module(ModuleCmd::InstallFiles { version, dir, executable, accept, .. })) = cli.cmd else {
                panic!("{}: not install-files", flag)
            };
            let body = install_files_body(&version, &dir, executable.as_deref(), &accept);
            assert_eq!(body["server_executable"], "x.jar", "{}", flag);
            assert!(body.get("jar_name").is_none());
        }
        let body = install_files_body("1", "d", None, &[]);
        assert!(body.get("server_executable").is_none(), "without the flag the module default is used");
    }
}
