//! Dispatcher for one-shot CLI subcommands.
//!
//! Each public `run_*` function returns either:
//!   * `Ok(())` — printed something already, exit 0
//!   * `Err(anyhow::Error)` — caller maps via `classify_error`
//!
//! Keep handlers thin: parse args -> call `DaemonClient` / `process` /
//! `cli_config` -> hand a `Value` to `Output`.

use anyhow::Context;
use serde_json::{json, Value};

use crate::cli_config::CliSettings;
use crate::client::DaemonClient;
use crate::payload::{self, parse_scalar};
use crate::process;
use crate::update_flow::{self, Tone};

use super::format;
use super::{
    AliasCmd, Cli, Command, ConfigCmd, DaemonCmd,
    ExtensionCmd, ModuleCmd, Output, ServerCmd, SystemCmd, UpdateCmd,
};

/// Top-level dispatch. Returns the desired process exit code.
pub async fn dispatch(cli: Cli, client: DaemonClient) -> i32 {
    let out = Output::new(cli.json, cli.quiet);
    let yes = cli.yes;
    let cmd = cli.command.expect("dispatch should not be called without a subcommand");

    let result: anyhow::Result<()> = match cmd {
        Command::Tui => unreachable!("Tui handled by main"),
        Command::Shell { .. } => unreachable!("Shell handled by main"),
        Command::Status => run_status(&client, &out).await,
        Command::Server(c) => run_server(&client, &out, yes, c).await,
        Command::Module(c) => run_module(&client, &out, yes, c).await,
        Command::Extension(c) => run_extension(&client, &out, yes, c).await,
        Command::Daemon(c) => run_daemon(&client, &out, yes, c).await,
        Command::Update(c) => run_update(&client, &out, yes, c).await,
        Command::Config(c) => run_config(&client, &out, c),
        Command::Alias(c) => run_alias(&client, &out, c).await,
        Command::System(c) => run_system(&client, &out, c).await,
    };

    match result {
        Ok(()) => super::EXIT_OK,
        Err(err) => {
            let code = super::classify_error(&err);
            if code == super::EXIT_DAEMON_OFFLINE {
                out.print_error(&format!("{}\nhint: {}", err, super::daemon_offline_hint()));
            } else {
                out.print_error(&format!("{:#}", err));
            }
            code
        }
    }
}

// ─── status ────────────────────────────────────────────────────────────

async fn run_status(client: &DaemonClient, out: &Output) -> anyhow::Result<()> {
    let instances = client.list_instances().await.map_err(|e| {
        anyhow::anyhow!(
            "Saba-Core daemon is offline ({}): {}",
            client.base_url(),
            e
        )
    })?;
    let modules = client.list_modules().await?;
    let exts = client.list_extensions().await?;

    let summary = json!({
        "online": true,
        "daemon_url": client.base_url(),
        "instances": instances.len(),
        "modules": modules.len(),
        "extensions": exts.len(),
    });
    if out.json {
        out.print_value(&summary);
    } else {
        println!(
            "Saba-Core   : online ({})\n\
             Instances   : {}\n\
             Modules     : {}\n\
             Extensions  : {}",
            client.base_url(),
            instances.len(),
            modules.len(),
            exts.len(),
        );
    }
    Ok(())
}

// ─── server ────────────────────────────────────────────────────────────

async fn run_server(
    client: &DaemonClient,
    out: &Output,
    yes: bool,
    cmd: ServerCmd,
) -> anyhow::Result<()> {
    match cmd {
        ServerCmd::List => {
            let instances = client.list_instances().await?;
            if out.json {
                out.print_value(&Value::Array(instances));
            } else {
                println!("{}", format::instances_table(&instances));
            }
        }
        ServerCmd::Info { name } => {
            let inst = super::resolve_instance(client, &name).await?;
            out.print_value(&inst);
        }
        ServerCmd::Create { name, module, extra } => {
            // Build payload: {name, module_name, ...extra}
            let mut payload = json!({ "name": name, "module_name": module });
            if let Some(raw) = extra {
                let merge: Value = serde_json::from_str(&raw)
                    .with_context(|| format!("--extra is not valid JSON: {}", raw))?;
                merge_json(&mut payload, merge)?;
            }
            let resp = client.create_instance(payload).await?;
            out.print_value(&resp);
        }
        ServerCmd::Install { module, game_version, name, extra } => {
            let mut payload = json!({ "version": game_version });
            if let Some(n) = name { payload["name"] = json!(n); }
            if let Some(raw) = extra {
                let merge: Value = serde_json::from_str(&raw)
                    .with_context(|| format!("--extra is not valid JSON: {}", raw))?;
                merge_json(&mut payload, merge)?;
            }
            let resp = client.install_server(&module, payload).await?;
            out.print_value(&resp);
        }
        ServerCmd::Set { name, pairs, body: body_json } => {
            let id = super::resolve_instance_id(client, &name).await?;
            let body = if let Some(raw) = body_json {
                serde_json::from_str::<Value>(&raw)
                    .with_context(|| format!("--body must be a JSON object: {}", raw))?
            } else if !pairs.is_empty() {
                let mut obj = serde_json::Map::new();
                for kv in &pairs {
                    let (k, v) = kv.split_once('=').ok_or_else(|| {
                        anyhow::anyhow!("expected KEY=VALUE, got '{}'", kv)
                    })?;
                    obj.insert(k.to_string(), parse_scalar(v));
                }
                Value::Object(obj)
            } else {
                anyhow::bail!("provide at least one KEY=VALUE pair or --body <object>");
            };
            let resp = client.update_instance(&id, body).await?;
            out.print_value(&resp);
        }
        ServerCmd::Reorder { order } => {
            // Resolve every supplied name/id to its canonical instance id.
            let mut ids = Vec::with_capacity(order.len());
            for q in &order {
                ids.push(super::resolve_instance_id(client, q).await?);
            }
            let resp = client.reorder_instances(&ids).await?;
            out.print_value(&resp);
        }
        ServerCmd::PropertiesWrite { name, pairs, body: body_json } => {
            let id = super::resolve_instance_id(client, &name).await?;
            // KEY=VALUE 모드는 기존 값과 병합하므로 먼저 읽는다.
            let current = if body_json.is_none() && !pairs.is_empty() {
                Some(client.read_properties(&id).await?)
            } else {
                None
            };
            let payload = properties_write_payload(current.as_ref(), &pairs, body_json.as_deref())?;
            let resp = client.write_properties(&id, payload).await?;
            out.print_value(&resp);
        }
        ServerCmd::Status { name } => {
            let inst = super::resolve_instance(client, &name).await?;
            let server_name = inst.get("name").and_then(|v| v.as_str()).unwrap_or(&name);
            let status = client.get_server_status(server_name).await?;
            out.print_value(&status);
        }
        ServerCmd::Start { name } => {
            let inst = super::resolve_instance(client, &name).await?;
            let server_name = inst.get("name").and_then(|v| v.as_str()).unwrap_or(&name);
            let module = inst
                .get("module_name")
                .or_else(|| inst.get("module"))
                .and_then(|v| v.as_str())
                .unwrap_or("");
            client.start_server(server_name, module).await?;
            out.print_success(&format!("Started server '{}'", server_name));
        }
        ServerCmd::Stop { name, force } => {
            let inst = super::resolve_instance(client, &name).await?;
            let server_name = inst.get("name").and_then(|v| v.as_str()).unwrap_or(&name);
            client.stop_server(server_name, force).await?;
            out.print_success(&format!("Stopped server '{}'", server_name));
        }
        ServerCmd::Restart { name } => {
            let inst = super::resolve_instance(client, &name).await?;
            let server_name = inst.get("name").and_then(|v| v.as_str()).unwrap_or(&name).to_string();
            let module = inst
                .get("module_name")
                .or_else(|| inst.get("module"))
                .and_then(|v| v.as_str())
                .unwrap_or("")
                .to_string();
            // best-effort stop then start
            let _ = client.stop_server(&server_name, false).await;
            tokio::time::sleep(std::time::Duration::from_millis(800)).await;
            client.start_server(&server_name, &module).await?;
            out.print_success(&format!("Restarted server '{}'", server_name));
        }
        ServerCmd::Exec { name, command } => {
            let id = super::resolve_instance_id(client, &name).await?;
            let line = command.join(" ");
            let resp = client.send_stdin(&id, &line).await?;
            out.print_value(&resp);
        }
        ServerCmd::Rcon { name, command } => {
            let id = super::resolve_instance_id(client, &name).await?;
            let line = command.join(" ");
            let resp = client.execute_rcon_command(&id, &line).await?;
            out.print_value(&resp);
        }
        ServerCmd::Rest {
            name,
            endpoint,
            method,
            body,
        } => {
            let id = super::resolve_instance_id(client, &name).await?;
            let parsed_body = match body {
                Some(s) => Some(
                    serde_json::from_str::<Value>(&s)
                        .with_context(|| format!("--body is not valid JSON: {}", s))?,
                ),
                None => None,
            };
            let resp = client
                .execute_rest_command(&id, &endpoint, &method, parsed_body)
                .await?;
            out.print_value(&resp);
        }
        ServerCmd::Console { name, count, follow, interval } => {
            let id = super::resolve_instance_id(client, &name).await?;
            if follow {
                follow_server_console(client, &id, count as usize, interval, out).await?;
            } else {
                let resp = client.get_console(&id, None, Some(count as usize)).await?;
                if out.json {
                    out.print_value(&resp);
                } else {
                    println!("{}", format::console_lines(&resp, count as usize));
                }
            }
        }
        ServerCmd::Diagnose { name } => {
            let id = super::resolve_instance_id(client, &name).await?;
            let resp = client.diagnose(&id).await?;
            out.print_value(&resp);
        }
        ServerCmd::Properties { name } => {
            let id = super::resolve_instance_id(client, &name).await?;
            let resp = client.read_properties(&id).await?;
            out.print_value(&resp);
        }
        ServerCmd::PropertiesReset { name } => {
            require_yes(yes, "reset properties")?;
            let id = super::resolve_instance_id(client, &name).await?;
            client.reset_properties(&id).await?;
            out.print_success("Properties reset to defaults");
        }
        ServerCmd::Reset { name } => {
            require_yes(yes, "reset server")?;
            let id = super::resolve_instance_id(client, &name).await?;
            client.reset_server(&id).await?;
            out.print_success("Server reset to initial state");
        }
        ServerCmd::AcceptEula { name } => {
            let id = super::resolve_instance_id(client, &name).await?;
            client.accept_eula(&id).await?;
            out.print_success("EULA accepted");
        }
        ServerCmd::CheckUpdate { name } => {
            let id = super::resolve_instance_id(client, &name).await?;
            let resp = client.check_instance_update(&id).await?;
            out.print_value(&resp);
        }
        ServerCmd::ApplyUpdate { name } => {
            let id = super::resolve_instance_id(client, &name).await?;
            let resp = client.apply_instance_update(&id).await?;
            out.print_value(&resp);
        }
        ServerCmd::InstalledVersion { name } => {
            let id = super::resolve_instance_id(client, &name).await?;
            let resp = client.get_installed_version(&id).await?;
            out.print_value(&resp);
        }
        ServerCmd::Validate { name } => {
            let id = super::resolve_instance_id(client, &name).await?;
            let resp = client.validate_instance(&id).await?;
            out.print_value(&resp);
        }
        ServerCmd::Delete { name } => {
            require_yes(yes, "delete instance")?;
            let id = super::resolve_instance_id(client, &name).await?;
            client.delete_instance(&id).await?;
            out.print_success(&format!("Instance '{}' deleted", name));
        }
    }
    Ok(())
}

// ─── module ────────────────────────────────────────────────────────────

async fn run_module(
    client: &DaemonClient,
    out: &Output,
    yes: bool,
    cmd: ModuleCmd,
) -> anyhow::Result<()> {
    match cmd {
        ModuleCmd::List => {
            let mods = client.list_modules().await?;
            if out.json {
                out.print_value(&Value::Array(mods));
            } else {
                println!("{}", format::modules_table(&mods));
            }
        }
        ModuleCmd::Info { name } => {
            let m = client.get_module(&name).await?;
            out.print_value(&m);
        }
        ModuleCmd::Versions { name } => {
            let v = client.list_versions(&name).await?;
            out.print_value(&v);
        }
        ModuleCmd::Refresh => {
            client.refresh_modules().await?;
            out.print_success("Modules refreshed");
        }
        ModuleCmd::Manifest => {
            let m = client.fetch_module_manifest().await?;
            out.print_value(&m);
        }
        ModuleCmd::Install { id } => {
            let v = client.install_module_from_manifest(&id).await?;
            out.print_value(&v);
        }
        ModuleCmd::Remove { id } => {
            require_yes(yes, "remove module")?;
            client.remove_module(&id).await?;
            out.print_success(&format!("Module '{}' removed", id));
        }
    }
    Ok(())
}

// ─── extension ─────────────────────────────────────────────────────────

async fn run_extension(
    client: &DaemonClient,
    out: &Output,
    yes: bool,
    cmd: ExtensionCmd,
) -> anyhow::Result<()> {
    match cmd {
        ExtensionCmd::List => {
            let exts = client.list_extensions().await?;
            if out.json {
                out.print_value(&Value::Array(exts));
            } else {
                println!("{}", format::extensions_table(&exts));
            }
        }
        ExtensionCmd::Enable { id } => {
            client.enable_extension(&id).await?;
            out.print_success(&format!("Extension '{}' enabled", id));
        }
        ExtensionCmd::Disable { id } => {
            client.disable_extension(&id).await?;
            out.print_success(&format!("Extension '{}' disabled", id));
        }
        ExtensionCmd::Manifest => {
            let v = client.fetch_extension_manifest().await?;
            out.print_value(&v);
        }
        ExtensionCmd::Install { id } => {
            let v = client.install_extension(&id, None).await?;
            out.print_value(&v);
        }
        ExtensionCmd::Remove { id } => {
            require_yes(yes, "remove extension")?;
            client.remove_extension(&id).await?;
            out.print_success(&format!("Extension '{}' removed", id));
        }
        ExtensionCmd::Rescan => {
            client.rescan_extensions().await?;
            out.print_success("Extensions rescanned");
        }
        ExtensionCmd::CheckUpdates => {
            let v = client.check_extension_updates().await?;
            out.print_value(&v);
        }
    }
    Ok(())
}

// ─── bot ─── removed: cloud-only relay bot lives entirely on the relay server.

// ─── daemon ────────────────────────────────────────────────────────────

async fn run_daemon(
    client: &DaemonClient,
    out: &Output,
    yes: bool,
    cmd: DaemonCmd,
) -> anyhow::Result<()> {
    match cmd {
        DaemonCmd::Status => {
            let online = client.is_reachable().await;
            if out.json {
                out.print_value(&json!({
                    "online": online,
                    "daemon_url": client.base_url(),
                }));
            } else {
                println!(
                    "Saba-Core : {} ({})",
                    if online { "online" } else { "offline" },
                    client.base_url()
                );
            }
        }
        DaemonCmd::Start => {
            let msg = tokio::task::spawn_blocking(process::start_daemon).await??;
            out.print_success(&msg);
        }
        DaemonCmd::Stop => {
            let msg = tokio::task::spawn_blocking(process::stop_daemon).await??;
            out.print_success(&msg);
        }
        DaemonCmd::Restart => {
            let _ = tokio::task::spawn_blocking(process::stop_daemon).await?;
            tokio::time::sleep(std::time::Duration::from_secs(1)).await;
            let msg = tokio::task::spawn_blocking(process::start_daemon).await??;
            out.print_success(&msg);
        }
        DaemonCmd::Console { since, count, follow, interval } => {
            if follow {
                follow_daemon_console(client, since, count as usize, interval, out).await?;
            } else {
                let resp = client
                    .get_daemon_console(Some(since), Some(count as usize))
                    .await?;
                if out.json {
                    out.print_value(&resp);
                } else {
                    println!("{}", format::console_lines(&resp, count as usize));
                }
            }
        }
        DaemonCmd::Shutdown => {
            require_yes(yes, "shutdown daemon")?;
            client.shutdown_daemon().await?;
            out.print_success("Shutdown requested");
        }
    }
    Ok(())
}

// ─── update ────────────────────────────────────────────────────────────

async fn run_update(
    client: &DaemonClient,
    out: &Output,
    yes: bool,
    cmd: UpdateCmd,
) -> anyhow::Result<()> {
    match cmd {
        UpdateCmd::Check => {
            let v = client.check_updates().await?;
            print_or_lines(out, &v, update_flow::status_lines(&v));
        }
        UpdateCmd::Status => {
            let v = client.get_update_status().await?;
            print_or_lines(out, &v, update_flow::status_lines(&v));
        }
        UpdateCmd::Download => {
            let v = client.download_updates().await?;
            out.print_value(&v);
        }
        UpdateCmd::Apply => {
            require_yes(yes, "apply updates")?;
            let (v, lines) = update_flow::apply(client).await?;
            print_or_lines(out, &v, lines);
        }
        UpdateCmd::Progress => {
            let v = client.get_download_progress().await?;
            out.print_value(&v);
        }
        UpdateCmd::ConfigShow => {
            let v = client.get_update_config().await?;
            print_or_lines(out, &v, update_flow::config_lines(&v));
        }
        UpdateCmd::ConfigSet { key, value } => {
            // 데몬 PUT은 본문에 있는 키만 바꾸는 부분 갱신이다.
            client.set_update_config(json!({ key.clone(): parse_scalar(&value) })).await?;
            out.print_success(&format!("Update config '{}' set", key));
        }
    }
    Ok(())
}

/// JSON 모드면 원본 응답, 아니면 사람이 읽는 줄(에러 줄은 stderr).
fn print_or_lines(out: &Output, value: &Value, lines: update_flow::Lines) {
    if out.json {
        out.print_value(value);
        return;
    }
    for (tone, line) in lines {
        if tone == Tone::Err {
            eprintln!("{}", line);
        } else {
            println!("{}", line);
        }
    }
}

// ─── config ────────────────────────────────────────────────────────────

fn run_config(client: &DaemonClient, out: &Output, cmd: ConfigCmd) -> anyhow::Result<()> {
    match cmd {
        ConfigCmd::Show => {
            let s = CliSettings::load();
            let mut obj = serde_json::Map::new();
            for (k, _desc) in CliSettings::available_keys() {
                if let Some(v) = s.get_value(k) {
                    obj.insert((*k).to_string(), Value::String(v));
                }
            }
            obj.insert("daemon_url".into(), Value::String(client.base_url().to_string()));
            out.print_value(&Value::Object(obj));
        }
        ConfigCmd::Get { key } => {
            let s = CliSettings::load();
            match s.get_value(&key) {
                Some(v) => println!("{}", v),
                None => anyhow::bail!("unknown config key: {}", key),
            }
        }
        ConfigCmd::Set { key, value } => {
            let mut s = CliSettings::load();
            s.set_value(&key, &value).map_err(anyhow::Error::msg)?;
            s.save()?;
            out.print_success(&format!("{} = {}", key, value));
        }
        ConfigCmd::Reset { key } => {
            let mut s = CliSettings::load();
            s.reset_value(&key).map_err(anyhow::Error::msg)?;
            s.save()?;
            out.print_success(&format!("{} reset", key));
        }
        ConfigCmd::Path => {
            let p = saba_chan_updater_lib::constants::resolve_data_dir().join("cli-settings.json");
            println!("{}", p.display());
        }
        ConfigCmd::DaemonUrl => {
            println!("{}", client.base_url());
        }
    }
    Ok(())
}

// ─── alias ─────────────────────────────────────────────────────────────

async fn run_alias(client: &DaemonClient, out: &Output, cmd: AliasCmd) -> anyhow::Result<()> {
    match cmd {
        AliasCmd::List => {
            // 데몬 호출 실패는 무조건 에러로 전파한다. (이전에는 `unwrap_or`로
            //  `(none)`을 보여줘서 사용자가 alias 부재로 오인할 수 있었음.)
            let cfg_v = client.get_bot_config().await?;
            let aliases = cfg_v
                .get("moduleAliases")
                .cloned()
                .unwrap_or_else(|| json!({}));
            let instances = client.list_instances().await?;
            let names: Vec<String> = instances
                .iter()
                .filter_map(|i| i.get("name").and_then(|v| v.as_str()).map(|s| s.to_string()))
                .collect();
            if out.json {
                out.print_value(&json!({
                    "instances": names,
                    "module_aliases": aliases,
                }));
            } else {
                println!("Server instances:");
                if names.is_empty() {
                    println!("  (none)");
                } else {
                    for n in &names {
                        println!("  - {}", n);
                    }
                }
                println!("\nModule aliases:");
                if let Some(map) = aliases.as_object() {
                    if map.is_empty() {
                        println!("  (none)");
                    } else {
                        for (k, v) in map {
                            println!("  {} -> {}", k, v.as_str().unwrap_or(""));
                        }
                    }
                }
            }
        }
        AliasCmd::Add { alias, module } => {
            let mut cfg_v = client.get_bot_config().await?;
            let map = cfg_v
                .get_mut("moduleAliases")
                .and_then(|v| v.as_object_mut());
            if let Some(m) = map {
                m.insert(alias.clone(), json!(module));
            } else {
                cfg_v["moduleAliases"] = json!({ alias.clone(): module });
            }
            client.save_bot_config(cfg_v).await?;
            out.print_success(&format!("alias '{}' added", alias));
        }
        AliasCmd::Remove { alias } => {
            let mut cfg_v = client.get_bot_config().await?;
            if let Some(m) = cfg_v
                .get_mut("moduleAliases")
                .and_then(|v| v.as_object_mut())
            {
                m.remove(&alias);
            }
            client.save_bot_config(cfg_v).await?;
            out.print_success(&format!("alias '{}' removed", alias));
        }
    }
    Ok(())
}

// ─── system ────────────────────────────────────────────────────────────

async fn run_system(client: &DaemonClient, out: &Output, cmd: SystemCmd) -> anyhow::Result<()> {
    match cmd {
        SystemCmd::Components => {
            let v = client.get_system_components().await?;
            out.print_value(&v);
        }
    }
    Ok(())
}

// ─── helpers ───────────────────────────────────────────────────────────

/// Merge a JSON value into the destination, doing a shallow object merge when
/// both sides are objects. Any other shape replaces the destination.
fn merge_json(dst: &mut Value, src: Value) -> anyhow::Result<()> {
    match (dst, src) {
        (Value::Object(d), Value::Object(s)) => {
            for (k, v) in s { d.insert(k, v); }
            Ok(())
        }
        (slot, src) => {
            *slot = src;
            Ok(())
        }
    }
}

fn require_yes(yes: bool, what: &str) -> anyhow::Result<()> {
    if !yes {
        anyhow::bail!(
            "destructive operation '{}' requires --yes (-y) to proceed",
            what
        );
    }
    Ok(())
}

/// `PUT /api/instance/{id}/properties` 본문 생성. 데몬은 `payload.settings`만 읽는다.
///
/// - `--body <JSON>`: 그 객체를 그대로 `settings`로 보낸다.
/// - `KEY=VALUE`: 읽기 응답(`{success, properties: {...}}`)의 `properties`에 병합한다.
fn properties_write_payload(
    current: Option<&Value>,
    pairs: &[String],
    body: Option<&str>,
) -> anyhow::Result<Value> {
    let settings = if let Some(raw) = body {
        let v = serde_json::from_str::<Value>(raw)
            .with_context(|| format!("--body must be a JSON object: {}", raw))?;
        if !v.is_object() {
            anyhow::bail!("--body must be a JSON object: {}", raw);
        }
        v
    } else if !pairs.is_empty() {
        let mut merged = current
            .and_then(|c| c.get("properties"))
            .and_then(|p| p.as_object())
            .cloned()
            .unwrap_or_default();
        for kv in pairs {
            let (k, v) = kv
                .split_once('=')
                .ok_or_else(|| anyhow::anyhow!("expected KEY=VALUE, got '{}'", kv))?;
            merged.insert(k.to_string(), parse_scalar(v));
        }
        Value::Object(merged)
    } else {
        anyhow::bail!("provide at least one KEY=VALUE pair or --body <object>");
    };
    Ok(json!({ "settings": settings }))
}

// ─── streaming console (--follow) ──────────────────────────────────────

/// 콘솔 응답에서 표시할 줄을 뽑고 커서(`다음 since`)를 전진시킨다.
fn take_new_lines(resp: &Value, cursor: &mut Option<u64>) -> Vec<String> {
    let entries = payload::console_entries(resp);
    if let Some(max) = payload::max_entry_id(entries) {
        *cursor = Some(max + 1);
    }
    entries.iter().filter_map(payload::entry_text).collect()
}

/// 커서 기반 폴링. 첫 요청은 `since` 없이 최근 `count`줄, 이후에는 `since=마지막 id+1`.
/// 데몬은 `since` 요청에 개수 제한 없이(서버 콘솔) 또는 `count`까지(데몬 로그) 돌려준다.
async fn follow_loop<F, Fut>(
    interval_ms: u64,
    out: &Output,
    initial_since: Option<u64>,
    mut fetch: F,
) -> anyhow::Result<()>
where
    F: FnMut(Option<u64>) -> Fut,
    Fut: std::future::Future<Output = anyhow::Result<Value>>,
{
    use tokio::time::{sleep, Duration};

    let print = |line: &str| {
        if out.json {
            println!("{}", serde_json::to_string(&json!({ "text": line })).unwrap_or_default());
        } else {
            println!("{}", line);
        }
    };

    let mut cursor = initial_since;
    let initial = fetch(cursor).await?;
    for line in take_new_lines(&initial, &mut cursor) {
        print(&line);
    }

    let interval = Duration::from_millis(interval_ms.max(100));
    loop {
        tokio::select! {
            _ = tokio::signal::ctrl_c() => {
                if !out.quiet {
                    eprintln!("\n(stopped)");
                }
                return Ok(());
            }
            _ = sleep(interval) => {}
        }
        match fetch(cursor).await {
            Ok(v) => {
                for line in take_new_lines(&v, &mut cursor) {
                    print(&line);
                }
            }
            Err(e) => {
                if super::classify_error(&e) != super::EXIT_DAEMON_OFFLINE {
                    return Err(e);
                }
                if !out.quiet {
                    eprintln!("(daemon offline; retrying...)");
                }
            }
        }
    }
}

async fn follow_server_console(
    client: &DaemonClient,
    id: &str,
    initial_count: usize,
    interval_ms: u64,
    out: &Output,
) -> anyhow::Result<()> {
    follow_loop(interval_ms, out, None, |since| async move {
        // 첫 요청만 개수 제한, 이후는 커서 이후 전체
        let count = if since.is_none() { Some(initial_count) } else { None };
        client.get_console(id, since, count).await
    })
    .await
}

async fn follow_daemon_console(
    client: &DaemonClient,
    since: u64,
    count: usize,
    interval_ms: u64,
    out: &Output,
) -> anyhow::Result<()> {
    follow_loop(interval_ms, out, Some(since), |cursor| async move {
        client.get_daemon_console(cursor, Some(count.max(200))).await
    })
    .await
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn require_yes_blocks_without_flag() {
        assert!(require_yes(false, "x").is_err());
        assert!(require_yes(true, "x").is_ok());
    }

    /// 회귀: 읽기 응답 래퍼(`{success, properties}`)를 그대로 PUT하면 데몬이
    /// `payload.settings`를 찾지 못한다. 본문은 `{"settings": {...}}`이어야 한다.
    #[test]
    fn properties_write_pairs_merge_into_settings() {
        let current = json!({ "success": true, "properties": { "pvp": true, "motd": "hi" } });
        let pairs = vec!["max-players=20".to_string(), "pvp=false".to_string()];
        let body = properties_write_payload(Some(&current), &pairs, None).unwrap();
        assert_eq!(body, json!({ "settings": { "pvp": false, "motd": "hi", "max-players": 20 } }));
    }

    #[test]
    fn properties_write_body_is_wrapped_and_must_be_object() {
        let body = properties_write_payload(None, &[], Some(r#"{"a":1}"#)).unwrap();
        assert_eq!(body, json!({ "settings": { "a": 1 } }));
        assert!(properties_write_payload(None, &[], Some("[1]")).is_err());
        let err = properties_write_payload(None, &[], None).unwrap_err().to_string();
        assert!(err.contains("--body"), "error must name the real flag: {err}");
    }

    /// 회귀: 콘솔 폴링이 길이 비교로 새 줄을 찾으면 버퍼가 꽉 찬(100줄) 뒤 멈춘다.
    /// 커서는 마지막 id + 1로 전진해야 한다.
    #[test]
    fn take_new_lines_advances_cursor_past_last_id() {
        let mut cursor = None;
        let first = json!({ "lines": [{ "id": 7, "content": "a" }, { "id": 8, "content": "b" }] });
        assert_eq!(take_new_lines(&first, &mut cursor), vec!["a", "b"]);
        assert_eq!(cursor, Some(9));
        let empty = json!({ "lines": [] });
        assert!(take_new_lines(&empty, &mut cursor).is_empty());
        assert_eq!(cursor, Some(9));
    }
}
