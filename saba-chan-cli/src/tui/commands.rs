//! 레거시 커맨드 디스패치 — 기존 명령어 입력 방식 호환
//!
//! `:` 키로 진입하는 커맨드 모드에서 사용됩니다.
//! `submit()`이 호출되면 동기 → 비동기 순으로 명령을 시도합니다.

use std::time::Duration;

use super::app::*;
use crate::cli_config::CliSettings;
use crate::client::DaemonClient;
use crate::gui_config;
use crate::extension_registry::{ExtensionRegistry, LIFECYCLE_COMMANDS};
use crate::process;

// ═══════════════════════════════════════════════════════
// 커맨드 제출 (Enter 키)
// ═══════════════════════════════════════════════════════

pub fn submit(app: &mut App) {
    let cmd = app.input.trim().to_string();
    app.input.clear();
    app.cursor = 0;
    if cmd.is_empty() { return; }

    app.history.push(cmd.clone());
    app.hist_idx = None;

    // exit / quit
    if matches!(cmd.to_lowercase().as_str(), "exit" | "quit" | "q") {
        app.output.push(Out::Cmd(cmd.clone()));
        app.output.push(Out::Info("Shutting down...".into()));

        let buf = app.async_out.clone();
        let exit_client = app.client.clone();
        let exit_client_id = app.client_id.clone();
        tokio::spawn(async move {
            let mut lines = Vec::new();
            let maybe_id = exit_client_id.lock().unwrap().take();
            if let Some(id) = maybe_id {
                let _ = exit_client.unregister_client(&id).await;
            }
            if process::check_bot_running() {
                match tokio::task::spawn_blocking(process::stop_bot).await {
                    Ok(Ok(msg)) => lines.push(Out::Ok(msg)),
                    Ok(Err(e)) => lines.push(Out::Err(format!("Bot stop failed: {}", e))),
                    Err(e) => lines.push(Out::Err(format!("Bot stop failed: {}", e))),
                }
            }
            if process::check_daemon_running() {
                match tokio::task::spawn_blocking(process::stop_daemon).await {
                    Ok(Ok(msg)) => lines.push(Out::Ok(msg)),
                    Ok(Err(e)) => lines.push(Out::Err(format!("Daemon stop failed: {}", e))),
                    Err(e) => lines.push(Out::Err(format!("Daemon stop failed: {}", e))),
                }
            }
            if lines.is_empty() { lines.push(Out::Info("Nothing to stop.".into())); }
            push_out(&buf, lines);
        });
        app.quit = true;
        return;
    }

    // "back" → 이전 화면으로 복귀
    if cmd.to_lowercase() == "back" {
        if app.screen_stack.is_empty() {
            app.output.push(Out::Info("Already at root. Use 'menu' or F2 to enter interactive menu.".into()));
        } else {
            app.pop_screen();
        }
        return;
    }

    // "menu" / "dashboard" → 인터랙티브 메뉴 모드
    if matches!(cmd.to_lowercase().as_str(), "menu" | "dashboard") {
        app.push_screen(Screen::Dashboard);
        app.input_mode = InputMode::Normal;
        return;
    }

    let cmd_start = app.output.len();
    app.output.push(Out::Cmd(cmd.clone()));

    let orig_parts: Vec<&str> = cmd.split_whitespace().collect();
    let lower_cmd = cmd.to_lowercase();
    let lower_parts: Vec<&str> = lower_cmd.split_whitespace().collect();

    // 동기 명령 시도
    if let Some(lines) = dispatch_sync(app, &lower_parts, &orig_parts) {
        app.output.extend(lines);
        app.output.push(Out::Blank);
        app.smart_scroll(cmd_start);
        return;
    }

    // 비동기 명령
    let client = app.client.clone();
    let buf = app.async_out.clone();
    let registry = app.registry.clone();
    let lower_owned = lower_cmd.clone();
    let orig_owned = cmd.clone();

    tokio::spawn(async move {
        let lower_parts: Vec<&str> = lower_owned.split_whitespace().collect();
        let orig_parts: Vec<&str> = orig_owned.split_whitespace().collect();
        let lines = match lower_parts.first().copied() {
            Some("server") => exec_server(&client, &lower_parts[1..], &orig_parts[1..]).await,
            Some("instance") => exec_instance(&client, &lower_parts[1..], &orig_parts[1..], &registry).await,
            Some("module") => exec_module(&client, &lower_parts[1..]).await,
            Some("daemon") => exec_daemon(&lower_parts[1..]).await,
            Some("bot") => exec_bot(&lower_parts[1..]).await,
            Some("exec") => exec_exec(&client, &orig_parts[1..]).await,
            Some("update") => exec_update(&client, &lower_parts[1..]).await,
            Some(word) => {
                if let Some(module_name) = registry.resolve_module_name(word) {
                    exec_module_cmd(&client, &registry, &module_name, &lower_parts[1..]).await
                } else {
                    vec![Out::Err(format!("Unknown command '{}'. Type 'help'.", word))]
                }
            }
            None => vec![],
        };
        push_out(&buf, lines);
    });

    app.scroll_up = 0;
}

// ═══════════════════════════════════════════════════════
// 자동완성 (Tab)
// ═══════════════════════════════════════════════════════

pub fn autocomplete(app: &mut App) {
    let input = app.input.trim().to_string();
    if input.is_empty() { return; }
    let parts: Vec<&str> = input.split_whitespace().collect();

    if parts.len() == 1 && !app.input.ends_with(' ') {
        let mut candidates: Vec<String> = vec![
            "server".into(), "instance".into(), "module".into(), "daemon".into(), "bot".into(),
            "exec".into(), "config".into(), "help".into(), "exit".into(), "update".into(),
            "back".into(), "menu".into(),
        ];
        for name in app.registry.module_names() {
            candidates.push(name.to_string());
        }
        let matches: Vec<&String> = candidates.iter()
            .filter(|c| c.to_lowercase().starts_with(&parts[0].to_lowercase()))
            .collect();
        if matches.len() == 1 {
            app.input = format!("{} ", matches[0]);
            app.cursor = app.input.chars().count();
        }
    } else if parts.len() <= 2 {
        let first = parts[0];
        let partial = if parts.len() > 1 && !app.input.ends_with(' ') { parts[1] } else { "" };

        if let Some(module_name) = app.registry.resolve_module_name(first) {
            let mut sub: Vec<String> = vec!["start".into(), "stop".into(), "restart".into(), "status".into()];
            if let Some(mi) = app.registry.get_module(&module_name) {
                for cmd in &mi.commands {
                    sub.push(cmd.name.clone());
                }
            }
            let matches: Vec<&String> = sub.iter().filter(|c| c.starts_with(partial)).collect();
            if matches.len() == 1 {
                app.input = format!("{} {} ", first, matches[0]);
                app.cursor = app.input.chars().count();
            }
        } else {
            let full_cmds = [
                "server list", "server start", "server stop", "server restart", "server status",
                "server managed", "server console", "server stdin", "server diagnose",
                "server validate", "server eula", "server properties", "server set-property",
                "instance list", "instance create", "instance delete", "instance show",
                "instance set", "instance settings",
                "module list", "module refresh", "module versions", "module install", "module info",
                "daemon start", "daemon stop", "daemon status", "daemon restart",
                "bot start", "bot stop", "bot status", "bot token", "bot prefix", "bot alias",
                "config show", "config set", "config get", "config reset",
                "update check", "update status", "update download", "update apply",
            ];
            let prefix = app.input.trim();
            let matches: Vec<&&str> = full_cmds.iter().filter(|c| c.starts_with(prefix)).collect();
            if matches.len() == 1 {
                app.input = format!("{} ", matches[0]);
                app.cursor = app.input.chars().count();
            }
        }
    }
}

// ═══════════════════════════════════════════════════════
// 동기 디스패치
// ═══════════════════════════════════════════════════════

fn dispatch_sync(app: &mut App, lower: &[&str], orig: &[&str]) -> Option<Vec<Out>> {
    match lower.first().copied() {
        Some("config") => Some(cmd_config(app, &orig[1..])),
        Some("help") => Some(cmd_help(app)),
        // 서브커맨드 없이 카테고리만 입력 → 간단 도움말
        Some("server") if lower.len() == 1 => Some(vec![
            Out::Text("  server list|start|stop|restart|status <name>".into()),
            Out::Text("  server managed|console|stdin|diagnose|validate|eula|properties <name>".into()),
            Out::Text("  server set-property <name> <key> <value>".into()),
            Out::Text("  Tip: 대시보드에서 Servers 메뉴를 이용하면 더 편리합니다.".into()),
        ]),
        Some("instance") if lower.len() == 1 => Some(vec![
            Out::Text("  instance list|show|create|delete|settings|set <name>".into()),
        ]),
        Some("module") if lower.len() == 1 => Some(vec![
            Out::Text("  module list|info|refresh|versions|install".into()),
        ]),
        Some("update") if lower.len() == 1 => Some(vec![
            Out::Text("  update check|status|download|apply|config|install".into()),
        ]),
        Some("daemon") if lower.len() == 1 => Some(vec![
            Out::Text("  daemon start|stop|status|restart".into()),
        ]),
        Some("bot") if lower.len() == 1 => Some(vec![
            Out::Text("  bot start|stop|status|token|prefix|alias".into()),
        ]),
        Some("bot") if lower.len() >= 2 && lower[1] == "token" => Some(cmd_bot_token(&orig[2..])),
        Some("bot") if lower.len() >= 2 && lower[1] == "prefix" => Some(cmd_bot_prefix(&orig[2..])),
        Some("bot") if lower.len() >= 2 && lower[1] == "status" => Some(cmd_bot_status()),
        Some("bot") if lower.len() >= 2 && lower[1] == "alias" => Some(cmd_bot_alias(&lower[2..], &orig[2..])),
        Some("exec") if lower.len() < 4 => Some(vec![
            Out::Text("  exec <id> cmd|rcon|rest <command>".into()),
        ]),
        _ => {
            if lower.len() == 1 && lower[0] == "sabachan" {
                return Some(cmd_sabachan());
            }
            if lower.len() == 1 {
                if let Some(module_name) = app.registry.resolve_module_name(lower[0]) {
                    return Some(show_extension_commands(&app.registry, &module_name));
                }
            }
            None
        }
    }
}

// ═══════════════════════════════════════════════════════
// 동기 커맨드 구현
// ═══════════════════════════════════════════════════════

fn cmd_config(app: &mut App, args: &[&str]) -> Vec<Out> {
    match args.first().copied() {
        None | Some("show") => {
            let token = gui_config::get_discord_token().ok().flatten();
            let modules = gui_config::get_extensions_path().unwrap_or_default();
            let prefix = gui_config::get_bot_prefix().unwrap_or_else(|_| "!saba".into());
            let gui_lang = gui_config::get_language().unwrap_or_else(|_| "en".into());
            let auto_start_gui = gui_config::get_discord_auto_start().unwrap_or(false);
            let mut lines = vec![
                Out::Info("CLI Settings:".into()),
                Out::Text(format!("  language         {}", app.settings.get_value("language").unwrap_or_else(|| "(auto)".into()))),
                Out::Text(format!("  auto_start       {}", app.settings.auto_start)),
                Out::Text(format!("  refresh_interval {}", app.settings.refresh_interval)),
                Out::Text(format!("  bot_prefix       {}", app.settings.bot_prefix)),
                Out::Blank,
                Out::Info("GUI Settings:".into()),
                Out::Text(format!("  token            {}", if token.is_some() { "✓ set" } else { "✗ not set" })),
                Out::Text(format!("  prefix           {}", prefix)),
                Out::Text(format!("  extensions_path     {}", modules)),
                Out::Text(format!("  language         {}", gui_lang)),
                Out::Text(format!("  discord_auto     {}", auto_start_gui)),
            ];
            lines.push(Out::Blank);
            lines.push(Out::Info("CLI: config set|get|reset <key> <value>".into()));
            lines.push(Out::Info("GUI: config gui language|modules_path|token <value>".into()));
            lines.push(Out::Text(format!("  keys: {}", CliSettings::available_keys().iter().map(|(k,_)| *k).collect::<Vec<_>>().join(", "))));
            lines
        }
        Some("gui") => cmd_config_gui(&args[1..]),
        Some("get") => {
            if args.len() < 2 { return vec![Out::Err("Usage: config get <key>".into())]; }
            match app.settings.get_value(args[1]) {
                Some(v) => vec![Out::Ok(format!("{} = {}", args[1], v))],
                None => vec![Out::Err(format!("Unknown key: {}", args[1]))],
            }
        }
        Some("set") => {
            if args.len() < 3 { return vec![Out::Err("Usage: config set <key> <value>".into())]; }
            let val = args[2..].join(" ");
            match app.settings.set_value(args[1], &val) {
                Ok(()) => {
                    if let Err(e) = app.settings.save() {
                        vec![Out::Err(format!("Set ok but save failed: {}", e))]
                    } else {
                        vec![Out::Ok(format!("{} = {}", args[1], val))]
                    }
                }
                Err(e) => vec![Out::Err(format!("{}", e))],
            }
        }
        Some("reset") => {
            if args.len() < 2 { return vec![Out::Err("Usage: config reset <key>".into())]; }
            match app.settings.reset_value(args[1]) {
                Ok(()) => {
                    if let Err(e) = app.settings.save() {
                        vec![Out::Err(format!("Reset ok but save failed: {}", e))]
                    } else {
                        let new_val = app.settings.get_value(args[1]).unwrap_or_default();
                        vec![Out::Ok(format!("{} reset → {}", args[1], new_val))]
                    }
                }
                Err(e) => vec![Out::Err(format!("{}", e))],
            }
        }
        Some(sub) => vec![Out::Err(format!("Unknown config subcommand: {}. Try: show, get, set, reset, gui", sub))],
    }
}

fn cmd_config_gui(args: &[&str]) -> Vec<Out> {
    match args.first().copied() {
        Some("language") | Some("lang") => {
            if args.len() < 2 {
                let cur = gui_config::get_language().unwrap_or_else(|_| "en".into());
                return vec![
                    Out::Ok(format!("GUI language: {}", cur)),
                    Out::Text("  Available: en, ko, ja, zh-CN, zh-TW, es, pt-BR, ru, de, fr".into()),
                ];
            }
            match gui_config::set_language(args[1]) {
                Ok(()) => vec![Out::Ok(format!("✓ GUI language set to: {}", args[1]))],
                Err(e) => vec![Out::Err(format!("✗ {}", e))],
            }
        }
        Some("extensions_path") | Some("modules") => {
            if args.len() < 2 {
                let cur = gui_config::get_extensions_path().unwrap_or_default();
                return vec![Out::Ok(format!("Extensions path: {}", cur))];
            }
            let path = args[1..].join(" ");
            match gui_config::set_extensions_path(&path) {
                Ok(()) => vec![Out::Ok(format!("✓ Extensions path set to: {}", path))],
                Err(e) => vec![Out::Err(format!("✗ {}", e))],
            }
        }
        Some("token") => {
            if args.len() < 2 {
                match gui_config::get_discord_token() {
                    Ok(Some(t)) => {
                        let masked = if t.len() > 8 { format!("{}...{}", &t[..4], &t[t.len()-4..]) } else { "****".into() };
                        vec![Out::Ok(format!("Token: {}", masked))]
                    }
                    _ => vec![Out::Text("Token: not set".into())],
                }
            } else if args[1] == "clear" {
                match gui_config::clear_discord_token() {
                    Ok(()) => vec![Out::Ok("✓ Discord token cleared.".into())],
                    Err(e) => vec![Out::Err(format!("✗ {}", e))],
                }
            } else {
                match gui_config::set_discord_token(args[1]) {
                    Ok(()) => vec![Out::Ok("✓ Discord token saved.".into())],
                    Err(e) => vec![Out::Err(format!("✗ {}", e))],
                }
            }
        }
        _ => vec![Out::Err("Usage: config gui [language|modules_path|token] <value>".into())],
    }
}

fn cmd_bot_token(args: &[&str]) -> Vec<Out> {
    match args.first().copied() {
        None | Some("show") => {
            match gui_config::get_discord_token() {
                Ok(Some(t)) => {
                    let masked = if t.len() > 8 { format!("{}...{}", &t[..4], &t[t.len()-4..]) } else { "****".into() };
                    vec![Out::Ok(format!("Token: {}", masked))]
                }
                Ok(None) => vec![Out::Text("Token: not set".into())],
                Err(e) => vec![Out::Err(format!("Error: {}", e))],
            }
        }
        Some("set") => {
            if args.len() < 2 { return vec![Out::Err("Usage: bot token set <TOKEN>".into())]; }
            match gui_config::set_discord_token(args[1]) {
                Ok(()) => vec![Out::Ok("Discord token saved.".into())],
                Err(e) => vec![Out::Err(format!("Failed: {}", e))],
            }
        }
        Some("clear") => {
            match gui_config::clear_discord_token() {
                Ok(()) => vec![Out::Ok("Discord token cleared.".into())],
                Err(e) => vec![Out::Err(format!("Failed: {}", e))],
            }
        }
        Some(sub) => vec![Out::Err(format!("Unknown: bot token {}. Try: show, set, clear", sub))],
    }
}

fn cmd_bot_prefix(args: &[&str]) -> Vec<Out> {
    match args.first().copied() {
        None | Some("show") => {
            let prefix = gui_config::get_bot_prefix().unwrap_or_else(|_| "!saba".into());
            vec![Out::Ok(format!("Bot prefix: {}", prefix))]
        }
        Some("set") => {
            if args.len() < 2 { return vec![Out::Err("Usage: bot prefix set <PREFIX>".into())]; }
            match gui_config::set_bot_prefix(args[1]) {
                Ok(()) => vec![Out::Ok(format!("Bot prefix set to: {}", args[1]))],
                Err(e) => vec![Out::Err(format!("Failed: {}", e))],
            }
        }
        Some(sub) => vec![Out::Err(format!("Unknown: bot prefix {}. Try: show, set", sub))],
    }
}

fn cmd_bot_status() -> Vec<Out> {
    let running = process::check_bot_running();
    let token = gui_config::get_discord_token().ok().flatten();
    let prefix = gui_config::get_bot_prefix().unwrap_or_else(|_| "!saba".into());
    let auto = gui_config::get_discord_auto_start().unwrap_or(false);
    let status_str = if running { "● RUNNING" } else if token.is_none() { "○ NO TOKEN" } else { "○ OFFLINE" };
    vec![
        Out::Ok(format!("Discord Bot: {}", status_str)),
        Out::Text(format!("  Token:      {}", if token.is_some() { "✓ set" } else { "✗ not set" })),
        Out::Text(format!("  Prefix:     {}", prefix)),
        Out::Text(format!("  Auto-start: {}", auto)),
    ]
}

fn cmd_bot_alias(lower: &[&str], orig: &[&str]) -> Vec<Out> {
    match lower.first().copied() {
        None | Some("show") => {
            let config = gui_config::load_bot_config().unwrap_or_default();
            let mut lines = vec![Out::Ok("Discord Bot Aliases:".into())];

            lines.push(Out::Blank);
            lines.push(Out::Info("Module Aliases:".into()));
            if let Some(aliases) = config.get("moduleAliases").and_then(|v| v.as_object()) {
                if aliases.is_empty() { lines.push(Out::Text("  (none)".into())); }
                else { for (m, a) in aliases { lines.push(Out::Text(format!("  {} → {}", m, a.as_str().unwrap_or("?")))); } }
            } else { lines.push(Out::Text("  (none)".into())); }

            lines.push(Out::Blank);
            lines.push(Out::Info("Command Aliases:".into()));
            if let Some(cmd_aliases) = config.get("commandAliases").and_then(|v| v.as_object()) {
                if cmd_aliases.is_empty() { lines.push(Out::Text("  (none)".into())); }
                else {
                    for (m, cmds) in cmd_aliases {
                        if let Some(cmd_map) = cmds.as_object() {
                            for (c, a) in cmd_map {
                                lines.push(Out::Text(format!("  {}.{} → {}", m, c, a.as_str().unwrap_or("?"))));
                            }
                        }
                    }
                }
            } else { lines.push(Out::Text("  (none)".into())); }

            lines.push(Out::Blank);
            lines.push(Out::Text("  bot alias set module <module> <aliases>".into()));
            lines.push(Out::Text("  bot alias set command <module> <cmd> <aliases>".into()));
            lines.push(Out::Text("  bot alias reset".into()));
            lines
        }
        Some("set") => {
            if lower.len() < 2 { return vec![Out::Err("Usage: bot alias set [module|command] ...".into())]; }
            match lower[1] {
                "module" => {
                    if orig.len() < 4 { return vec![Out::Err("Usage: bot alias set module <name> <alias1,alias2>".into())]; }
                    let module_name = orig[2];
                    let aliases = orig[3];
                    let mut config = gui_config::load_bot_config().unwrap_or_default();
                    if config.get("moduleAliases").is_none() { config["moduleAliases"] = serde_json::json!({}); }
                    config["moduleAliases"][module_name] = serde_json::Value::String(aliases.to_string());
                    let path = gui_config::get_bot_config_path_pub();
                    match save_json_file(&path, &config) {
                        Ok(()) => vec![Out::Ok(format!("✓ Module alias set: {} → {}", module_name, aliases))],
                        Err(e) => vec![Out::Err(format!("✗ {}", e))],
                    }
                }
                "command" | "cmd" => {
                    if orig.len() < 5 { return vec![Out::Err("Usage: bot alias set command <module> <cmd> <aliases>".into())]; }
                    let module_name = orig[2];
                    let cmd_name = orig[3];
                    let aliases = orig[4];
                    let mut config = gui_config::load_bot_config().unwrap_or_default();
                    if config.get("commandAliases").is_none() { config["commandAliases"] = serde_json::json!({}); }
                    if config["commandAliases"].get(module_name).is_none() { config["commandAliases"][module_name] = serde_json::json!({}); }
                    config["commandAliases"][module_name][cmd_name] = serde_json::Value::String(aliases.to_string());
                    let path = gui_config::get_bot_config_path_pub();
                    match save_json_file(&path, &config) {
                        Ok(()) => vec![Out::Ok(format!("✓ Command alias: {}.{} → {}", module_name, cmd_name, aliases))],
                        Err(e) => vec![Out::Err(format!("✗ {}", e))],
                    }
                }
                _ => vec![Out::Err("Usage: bot alias set [module|command] ...".into())],
            }
        }
        Some("reset") => {
            let mut config = gui_config::load_bot_config().unwrap_or_default();
            config["moduleAliases"] = serde_json::json!({});
            config["commandAliases"] = serde_json::json!({});
            let path = gui_config::get_bot_config_path_pub();
            match save_json_file(&path, &config) {
                Ok(()) => vec![Out::Ok("✓ All aliases reset.".into())],
                Err(e) => vec![Out::Err(format!("✗ {}", e))],
            }
        }
        Some(sub) => vec![Out::Err(format!("Unknown: bot alias {}. Try: show, set, reset", sub))],
    }
}

fn cmd_help(app: &App) -> Vec<Out> {
    let mut lines = vec![
        Out::Info("─── Commands ───".into()),
        Out::Text("  server   [list|start|stop|restart|status] <name>".into()),
        Out::Text("  server   [managed|console|stdin|diagnose|validate|eula|properties] <name>".into()),
        Out::Text("  server   set-property <name> <key> <value>".into()),
        Out::Text("  instance [list|show|create|delete|settings|set] <name>".into()),
        Out::Text("  module   [list|info|refresh|versions|install]".into()),
        Out::Text("  daemon   [start|stop|status|restart]".into()),
        Out::Text("  bot      [start|stop|status|token|prefix|alias]".into()),
        Out::Text("  exec     <id> [cmd|rcon|rest] <command>".into()),
        Out::Text("  update   [check|status|download|apply|config|install]".into()),
        Out::Text("  config   [show|set|get|reset|gui]".into()),
        Out::Text("  menu     — Interactive menu mode (F2)".into()),
        Out::Text("  help     — This help".into()),
        Out::Text("  exit     — Quit (Ctrl+C)".into()),
    ];

    if !app.registry.extensions.is_empty() {
        lines.push(Out::Blank);
        lines.push(Out::Info("Module shortcuts:".into()));
        for mi in &app.registry.extensions {
            let mode = mi.interaction_mode.as_deref().unwrap_or("-");
            lines.push(Out::Text(format!(
                "  {:<10} {} [{}] — type '{}' for commands",
                mi.name, mi.display_name, mode, mi.name,
            )));
        }
    }

    lines.push(Out::Blank);
    lines.push(Out::Info("─── Keys ───".into()));
    lines.push(Out::Text("  PgUp/PgDn  Scroll output".into()));
    lines.push(Out::Text("  ↑ / ↓      Command history".into()));
    lines.push(Out::Text("  Tab        Autocomplete".into()),);
    lines.push(Out::Text("  F2         Interactive menu mode".into()));
    lines.push(Out::Text("  Ctrl+C     Force quit".into()));
    lines
}

fn show_extension_commands(registry: &ExtensionRegistry, module_name: &str) -> Vec<Out> {
    let module = match registry.get_module(module_name) {
        Some(m) => m,
        None => return vec![Out::Err(format!("Module '{}' not found", module_name))],
    };
    let mode_tag = module.interaction_mode.as_deref().unwrap_or("auto");
    let mut lines = vec![Out::Ok(format!("{} ({}) [mode: {}]:", module.display_name, module.name, mode_tag))];
    lines.push(Out::Text(format!("  {:<14} 서버 시작", "start")));
    lines.push(Out::Text(format!("  {:<14} 서버 종료", "stop")));
    lines.push(Out::Text(format!("  {:<14} 서버 재시작", "restart")));
    lines.push(Out::Text(format!("  {:<14} 서버 상태", "status")));
    for cmd in &module.commands {
        let desc = truncate_str(&cmd.description, 35);
        lines.push(Out::Text(format!("  {:<14} {}", cmd.name, desc)));
    }
    lines
}

fn cmd_sabachan() -> Vec<Out> {
    // Easter egg retained — abbreviated
    vec![Out::Ok("(◕‿◕) saba-chan desu~".into())]
}

// ═══════════════════════════════════════════════════════
// 비동기 커맨드 실행
// ═══════════════════════════════════════════════════════

async fn find_module_for_server(client: &DaemonClient, name: &str) -> Option<String> {
    if let Ok(instances) = client.list_instances().await {
        for inst in &instances {
            if inst["name"].as_str() == Some(name) {
                return inst["module_name"].as_str().map(|s| s.to_string());
            }
        }
    }
    None
}

async fn find_instance_id_by_name(client: &DaemonClient, name: &str) -> Option<String> {
    if let Ok(instances) = client.list_instances().await {
        for inst in &instances {
            if inst["name"].as_str() == Some(name) {
                return inst["id"].as_str().map(|s| s.to_string());
            }
        }
    }
    None
}

async fn exec_server(client: &DaemonClient, args: &[&str], orig_args: &[&str]) -> Vec<Out> {
    match args.first().copied() {
        Some("list") => match client.list_servers().await {
            Ok(list) if list.is_empty() => vec![Out::Text("No servers configured.".into())],
            Ok(list) => {
                let mut o = vec![Out::Ok(format!("{} server(s):", list.len()))];
                for s in &list {
                    let st = s["status"].as_str().unwrap_or("?");
                    let sym = if st == "running" { "▶" } else { "■" };
                    let pid_str = match s["pid"].as_u64() { Some(p) => format!(" PID:{}", p), None => String::new() };
                    let uptime = match s["start_time"].as_u64() { Some(_) => format!(" ⏱{}", format_uptime(s["start_time"].as_u64())), None => String::new() };
                    o.push(Out::Text(format!("  {} {} [{}] — {}{}{}", sym, s["name"].as_str().unwrap_or("?"), s["module"].as_str().unwrap_or("?"), st, pid_str, uptime)));
                }
                o
            }
            Err(e) => vec![Out::Err(format!("✗ {}", e))],
        },
        Some("start") if args.len() > 1 => {
            let module = match find_module_for_server(client, args[1]).await { Some(m) => m, None => return vec![Out::Err(format!("✗ Server '{}' not found", args[1]))] };
            match client.start_server(args[1], &module).await {
                Ok(r) => vec![Out::Ok(format!("✓ {}", r.get("message").and_then(|v| v.as_str()).unwrap_or("Started")))],
                Err(e) => vec![Out::Err(format!("✗ {}", e))],
            }
        }
        Some("stop") if args.len() > 1 => {
            let force = args.get(2).map(|&s| s == "force" || s == "true").unwrap_or(false);
            match client.stop_server(args[1], force).await {
                Ok(r) => vec![Out::Ok(format!("✓ {}", r.get("message").and_then(|v| v.as_str()).unwrap_or("Stopped")))],
                Err(e) => vec![Out::Err(format!("✗ {}", e))],
            }
        }
        Some("restart") if args.len() > 1 => {
            let name = args[1];
            if let Err(e) = client.stop_server(name, false).await { return vec![Out::Err(format!("✗ Stop: {}", e))]; }
            tokio::time::sleep(Duration::from_secs(1)).await;
            let module = match find_module_for_server(client, name).await { Some(m) => m, None => return vec![Out::Err(format!("✗ Server '{}' not found", name))] };
            match client.start_server(name, &module).await {
                Ok(_) => vec![Out::Ok("✓ Server restarted".into())],
                Err(e) => vec![Out::Err(format!("✗ Start: {}", e))],
            }
        }
        Some("status") if args.len() > 1 => match client.get_server_status(args[1]).await {
            Ok(s) => vec![Out::Text(format!("{} — {} | PID {} | Uptime {}", args[1], s["status"].as_str().unwrap_or("?"), s["pid"].as_u64().map(|p| p.to_string()).unwrap_or("-".into()), format_uptime(s["start_time"].as_u64())))],
            Err(e) => vec![Out::Err(format!("✗ {}", e))],
        },
        Some("managed") if args.len() > 1 => {
            let iid = match find_instance_id_by_name(client, args[1]).await { Some(id) => id, None => return vec![Out::Err(format!("✗ '{}' not found", args[1]))] };
            match client.start_managed(&iid).await {
                Ok(r) => vec![Out::Ok(format!("✓ {}", r.get("message").and_then(|v| v.as_str()).unwrap_or("Managed started")))],
                Err(e) => vec![Out::Err(format!("✗ {}", e))],
            }
        }
        Some("console") if args.len() > 1 => {
            let iid = match find_instance_id_by_name(client, args[1]).await { Some(id) => id, None => return vec![Out::Err(format!("✗ '{}' not found", args[1]))] };
            match client.get_console(&iid).await {
                Ok(data) => {
                    let mut o = vec![Out::Ok("Console output:".into())];
                    if let Some(lines) = data.get("lines").and_then(|v| v.as_array()) {
                        for l in lines.iter().rev().take(50).collect::<Vec<_>>().into_iter().rev() { o.push(Out::Text(l.as_str().unwrap_or("").into())); }
                    } else if let Some(output) = data.get("output").and_then(|v| v.as_str()) {
                        for l in output.lines().rev().take(50).collect::<Vec<_>>().into_iter().rev() { o.push(Out::Text(l.into())); }
                    }
                    o
                }
                Err(e) => vec![Out::Err(format!("✗ {}", e))],
            }
        }
        Some("stdin") if args.len() > 2 => {
            let iid = match find_instance_id_by_name(client, args[1]).await { Some(id) => id, None => return vec![Out::Err(format!("✗ '{}' not found", args[1]))] };
            let text = args[2..].join(" ");
            match client.send_stdin(&iid, &text).await {
                Ok(_) => vec![Out::Ok(format!("✓ Sent: {}", text))],
                Err(e) => vec![Out::Err(format!("✗ {}", e))],
            }
        }
        Some("diagnose") if args.len() > 1 => {
            let iid = match find_instance_id_by_name(client, args[1]).await { Some(id) => id, None => return vec![Out::Err(format!("✗ '{}' not found", args[1]))] };
            match client.diagnose(&iid).await {
                Ok(data) => {
                    let mut o = vec![Out::Ok(format!("Diagnosis for '{}':", args[1]))];
                    if let Some(obj) = data.as_object() { for (k, v) in obj { o.push(Out::Text(format!("  {}: {}", k, v))); } }
                    o
                }
                Err(e) => vec![Out::Err(format!("✗ {}", e))],
            }
        }
        Some("validate") if args.len() > 1 => {
            let iid = match find_instance_id_by_name(client, args[1]).await { Some(id) => id, None => return vec![Out::Err(format!("✗ '{}' not found", args[1]))] };
            match client.validate_instance(&iid).await {
                Ok(r) => vec![Out::Ok(format!("✓ {}", r.get("message").and_then(|v| v.as_str()).unwrap_or("Validation passed")))],
                Err(e) => vec![Out::Err(format!("✗ {}", e))],
            }
        }
        Some("eula") if args.len() > 1 => {
            let iid = match find_instance_id_by_name(client, args[1]).await { Some(id) => id, None => return vec![Out::Err(format!("✗ '{}' not found", args[1]))] };
            match client.accept_eula(&iid).await {
                Ok(_) => vec![Out::Ok(format!("✓ EULA accepted for '{}'", args[1]))],
                Err(e) => vec![Out::Err(format!("✗ {}", e))],
            }
        }
        Some("properties") if args.len() > 1 => {
            let iid = match find_instance_id_by_name(client, args[1]).await { Some(id) => id, None => return vec![Out::Err(format!("✗ '{}' not found", args[1]))] };
            match client.read_properties(&iid).await {
                Ok(data) => {
                    let mut o = vec![Out::Ok(format!("Properties for '{}':", args[1]))];
                    let obj = data.get("properties").and_then(|v| v.as_object()).or_else(|| data.as_object());
                    if let Some(obj) = obj { for (k, v) in obj { o.push(Out::Text(format!("  {} = {}", k, v))); } }
                    o
                }
                Err(e) => vec![Out::Err(format!("✗ {}", e))],
            }
        }
        Some("set-property") if orig_args.len() > 3 => {
            let name = orig_args[0]; let key = orig_args[1]; let value = orig_args[2..].join(" ");
            let iid = match find_instance_id_by_name(client, name).await { Some(id) => id, None => return vec![Out::Err(format!("✗ '{}' not found", name))] };
            match client.read_properties(&iid).await {
                Ok(data) => {
                    let mut props = data.get("properties").cloned().unwrap_or(data.clone());
                    props[key] = serde_json::Value::String(value.clone());
                    match client.write_properties(&iid, serde_json::json!({ "properties": props })).await {
                        Ok(_) => vec![Out::Ok(format!("✓ {} = {}", key, value))],
                        Err(e) => vec![Out::Err(format!("✗ Write: {}", e))],
                    }
                }
                Err(e) => vec![Out::Err(format!("✗ Read: {}", e))],
            }
        }
        _ => vec![Out::Err("Usage: server [list|start|stop|restart|status|managed|console|stdin|diagnose|validate|eula|properties|set-property] <name>".into())],
    }
}

async fn exec_instance(client: &DaemonClient, lower: &[&str], orig: &[&str], registry: &ExtensionRegistry) -> Vec<Out> {
    match lower.first().copied() {
        Some("list") => match client.list_instances().await {
            Ok(list) if list.is_empty() => vec![Out::Text("No instances configured.".into())],
            Ok(list) => {
                let mut o = vec![Out::Ok(format!("{} instance(s):", list.len()))];
                for inst in &list {
                    o.push(Out::Text(format!("  {} [{}] id:{}",
                        inst["name"].as_str().unwrap_or("?"), inst["module_name"].as_str().unwrap_or("?"), inst["id"].as_str().unwrap_or("?"))));
                }
                o
            }
            Err(e) => vec![Out::Err(format!("✗ {}", e))],
        },
        Some("show") if orig.len() > 1 => {
            let iid = match find_instance_id_by_name(client, orig[1]).await { Some(id) => id, None => return vec![Out::Err(format!("✗ '{}' not found", orig[1]))] };
            match client.get_instance(&iid).await {
                Ok(data) => {
                    let mut o = vec![Out::Ok(format!("Instance: {}", orig[1]))];
                    if let Some(obj) = data.as_object() {
                        for (k, v) in obj {
                            if k == "id" || k == "name" || k == "module_name" { continue; }
                            let val_str = match v { serde_json::Value::String(s) => s.clone(), serde_json::Value::Null => "(not set)".into(), _ => v.to_string() };
                            o.push(Out::Text(format!("  {:<24} {}", k, val_str)));
                        }
                    }
                    o
                }
                Err(e) => vec![Out::Err(format!("✗ {}", e))],
            }
        }
        Some("create") if orig.len() > 2 => {
            let name = orig[1]; let module = orig[2];
            let module_name = registry.resolve_module_name(module).unwrap_or_else(|| module.to_string());
            let data = serde_json::json!({ "name": name, "module_name": module_name });
            match client.create_instance(data).await {
                Ok(r) => {
                    let id = r.get("id").and_then(|v| v.as_str()).unwrap_or("?");
                    vec![Out::Ok(format!("✓ Instance '{}' created (module: {}, id: {})", name, module_name, id))]
                }
                Err(e) => vec![Out::Err(format!("✗ {}", e))],
            }
        }
        Some("delete") if orig.len() > 1 => {
            let iid = match find_instance_id_by_name(client, orig[1]).await { Some(id) => id, None => return vec![Out::Err(format!("✗ '{}' not found", orig[1]))] };
            match client.delete_instance(&iid).await {
                Ok(_) => vec![Out::Ok(format!("✓ Instance '{}' deleted", orig[1]))],
                Err(e) => vec![Out::Err(format!("✗ {}", e))],
            }
        }
        Some("set") if orig.len() > 3 => {
            let name = orig[1]; let key = orig[2]; let value = orig[3..].join(" ");
            let iid = match find_instance_id_by_name(client, name).await { Some(id) => id, None => return vec![Out::Err(format!("✗ '{}' not found", name))] };
            let json_value = if value == "true" { serde_json::Value::Bool(true) } else if value == "false" { serde_json::Value::Bool(false) }
                else if let Ok(n) = value.parse::<i64>() { serde_json::json!(n) } else { serde_json::Value::String(value.clone()) };
            match client.update_instance(&iid, serde_json::json!({ key: json_value })).await {
                Ok(_) => vec![Out::Ok(format!("✓ {}.{} = {}", name, key, value))],
                Err(e) => vec![Out::Err(format!("✗ {}", e))],
            }
        }
        Some("settings") if orig.len() > 1 => {
            vec![Out::Info("Tip: 대시보드 > Servers > 서버 선택 > Settings 메뉴를 사용하면 편리합니다.".into()),
                 Out::Text(format!("  instance set {} <key> <value>", orig[1]))]
        }
        _ => vec![Out::Err("Usage: instance [list|show|create|delete|set|settings] <name>".into())],
    }
}

async fn exec_module(client: &DaemonClient, args: &[&str]) -> Vec<Out> {
    match args.first().copied() {
        Some("list") => match client.list_extensions().await {
            Ok(list) if list.is_empty() => vec![Out::Text("No extensions loaded.".into())],
            Ok(list) => {
                let mut o = vec![Out::Ok(format!("{} module(s):", list.len()))];
                for m in &list { o.push(Out::Text(format!("  • {} v{} [{}]", m["name"].as_str().unwrap_or("?"), m["version"].as_str().unwrap_or("?"), m["interaction_mode"].as_str().unwrap_or("-")))); }
                o
            }
            Err(e) => vec![Out::Err(format!("✗ {}", e))],
        },
        Some("info") if args.len() > 1 => {
            match client.get_extension(args[1]).await {
                Ok(data) => {
                    let mut o = vec![Out::Ok(format!("Module: {}", args[1]))];
                    for key in &["name", "version", "description", "game_name", "display_name", "interaction_mode"] {
                        if let Some(val) = data.get(*key).and_then(|v| v.as_str()) { o.push(Out::Text(format!("  {:<20} {}", key, val))); }
                    }
                    o
                }
                Err(e) => vec![Out::Err(format!("✗ {}", e))],
            }
        }
        Some("refresh") | Some("reload") => match client.refresh_extensions().await {
            Ok(_) => vec![Out::Ok("✓ Modules refreshed".into())],
            Err(e) => vec![Out::Err(format!("✗ {}", e))],
        },
        Some("versions") if args.len() > 1 => {
            match client.list_versions(args[1]).await {
                Ok(data) => {
                    if let Some(versions) = data.get("versions").and_then(|v| v.as_array()) {
                        let mut o = vec![Out::Ok(format!("{} version(s):", versions.len()))];
                        for v in versions { o.push(Out::Text(format!("  • {}", v.as_str().or_else(|| v["id"].as_str()).unwrap_or("?")))); }
                        o
                    } else { vec![Out::Ok(format!("{}", data))] }
                }
                Err(e) => vec![Out::Err(format!("✗ {}", e))],
            }
        }
        Some("install") if args.len() > 1 => {
            let module = args[1]; let version = args.get(2).copied().unwrap_or("latest");
            match client.install_server(module, serde_json::json!({ "version": version })).await {
                Ok(r) => vec![Out::Ok(format!("✓ {}", r.get("message").and_then(|v| v.as_str()).unwrap_or("Install started")))],
                Err(e) => vec![Out::Err(format!("✗ {}", e))],
            }
        }
        _ => vec![Out::Err("Usage: module [list|info|refresh|versions|install] <name>".into())],
    }
}

async fn exec_daemon(args: &[&str]) -> Vec<Out> {
    match args.first().copied() {
        Some("start") => match tokio::task::spawn_blocking(process::start_daemon).await {
            Ok(Ok(msg)) => msg.lines().map(|l| Out::Ok(l.into())).collect(),
            Ok(Err(e)) => vec![Out::Err(format!("✗ {}", e))],
            Err(e) => vec![Out::Err(format!("✗ {}", e))],
        },
        Some("stop") => match tokio::task::spawn_blocking(process::stop_daemon).await {
            Ok(Ok(msg)) => vec![Out::Ok(msg)],
            Ok(Err(e)) => vec![Out::Err(format!("✗ {}", e))],
            Err(e) => vec![Out::Err(format!("✗ {}", e))],
        },
        Some("status") => {
            let running = tokio::task::spawn_blocking(process::check_daemon_running).await.unwrap_or(false);
            if running { vec![Out::Ok("Daemon: ● RUNNING".into())] } else { vec![Out::Text("Daemon: ○ OFFLINE".into())] }
        }
        Some("restart") => {
            let _ = tokio::task::spawn_blocking(process::stop_daemon).await;
            tokio::time::sleep(Duration::from_secs(1)).await;
            match tokio::task::spawn_blocking(process::start_daemon).await {
                Ok(Ok(msg)) => msg.lines().map(|l| Out::Ok(l.into())).collect(),
                Ok(Err(e)) => vec![Out::Err(format!("✗ {}", e))],
                Err(e) => vec![Out::Err(format!("✗ {}", e))],
            }
        }
        _ => vec![Out::Err("Usage: daemon [start|stop|status|restart]".into())],
    }
}

async fn exec_bot(args: &[&str]) -> Vec<Out> {
    match args.first().copied() {
        Some("start") => match tokio::task::spawn_blocking(process::start_bot).await {
            Ok(Ok(msg)) => vec![Out::Ok(msg)],
            Ok(Err(e)) => vec![Out::Err(format!("✗ {}", e))],
            Err(e) => vec![Out::Err(format!("✗ {}", e))],
        },
        Some("stop") => match tokio::task::spawn_blocking(process::stop_bot).await {
            Ok(Ok(msg)) => vec![Out::Ok(msg)],
            Ok(Err(e)) => vec![Out::Err(format!("✗ {}", e))],
            Err(e) => vec![Out::Err(format!("✗ {}", e))],
        },
        _ => vec![Out::Err("Usage: bot [start|stop|status|token|prefix|alias]".into())],
    }
}

async fn exec_exec(client: &DaemonClient, args: &[&str]) -> Vec<Out> {
    if args.len() < 3 { return vec![Out::Err("Usage: exec <id> [cmd|rcon|rest] <command>".into())]; }
    let (id, mode) = (args[0], args[1]);
    let cmd = args[2..].join(" ");
    let result = match mode {
        "rcon" => client.execute_rcon_command(id, &cmd).await,
        "rest" => client.execute_rest_command(id, &cmd).await,
        _ => client.execute_command(id, &cmd, None).await,
    };
    match result {
        Ok(r) => vec![Out::Ok(r.get("message").and_then(|v| v.as_str()).unwrap_or("OK").into())],
        Err(e) => vec![Out::Err(format!("✗ {}", e))],
    }
}

async fn exec_update(client: &DaemonClient, args: &[&str]) -> Vec<Out> {
    match args.first().copied() {
        Some("check") => match client.check_updates().await {
            Ok(v) => {
                let comps = v["components"].as_array();
                if let Some(comps) = comps {
                    let any = comps.iter().any(|c| c["update_available"].as_bool().unwrap_or(false));
                    if any {
                        let mut lines = vec![Out::Ok("Updates available:".into())];
                        for c in comps {
                            let marker = if c["update_available"].as_bool().unwrap_or(false) { "⬆" } else { "✓" };
                            lines.push(Out::Text(format!("  {} {:<20} {} → {}",
                                marker, c["component"].as_str().unwrap_or("?"), c["current_version"].as_str().unwrap_or("?"), c["latest_version"].as_str().unwrap_or("?"))));
                        }
                        lines
                    } else { vec![Out::Ok("All up to date.".into())] }
                } else { vec![Out::Ok(format!("{}", v))] }
            }
            Err(e) => vec![Out::Err(format!("✗ {}", e))],
        },
        Some("status") => match client.get_update_status().await {
            Ok(v) => {
                let mut lines = vec![Out::Ok("Update Status:".into())];
                if let Some(comps) = v["components"].as_array() {
                    for c in comps { lines.push(Out::Text(format!("  {:<20} v{}", c["component"].as_str().unwrap_or("?"), c["current_version"].as_str().unwrap_or("?")))); }
                }
                lines
            }
            Err(e) => vec![Out::Err(format!("✗ {}", e))],
        },
        Some("download") => match client.download_updates().await {
            Ok(v) => vec![Out::Ok(format!("✓ {}", v.get("message").and_then(|m| m.as_str()).unwrap_or("Download initiated")))],
            Err(e) => vec![Out::Err(format!("✗ {}", e))],
        },
        Some("apply") => match client.apply_updates().await {
            Ok(v) => vec![Out::Ok(format!("✓ {}", v.get("message").and_then(|m| m.as_str()).unwrap_or("Applied")))],
            Err(e) => vec![Out::Err(format!("✗ {}", e))],
        },
        Some("config") => match client.get_update_config().await {
            Ok(v) => {
                let mut lines = vec![Out::Ok("Updater Config:".into())];
                if let Some(map) = v.as_object() { for (k, val) in map { lines.push(Out::Text(format!("  {}: {}", k, val))); } }
                lines
            }
            Err(e) => vec![Out::Err(format!("✗ {}", e))],
        },
        Some("install") if args.len() >= 2 => {
            match client.install_component(args[1]).await {
                Ok(v) => vec![Out::Ok(format!("✓ {}", v.get("message").and_then(|m| m.as_str()).unwrap_or("Installed")))],
                Err(e) => vec![Out::Err(format!("✗ {}", e))],
            }
        }
        Some("install") => match client.run_install(None).await {
            Ok(v) => vec![Out::Ok(format!("✓ {}", v.get("message").and_then(|m| m.as_str()).unwrap_or("Install initiated")))],
            Err(e) => vec![Out::Err(format!("✗ {}", e))],
        },
        _ => vec![Out::Text("  update check|status|download|apply|config|install [key]".into())],
    }
}

// ═══ 모듈 단축 명령어 (palworld start, 팰월드 시작 등) ═══

async fn exec_module_cmd(client: &DaemonClient, registry: &ExtensionRegistry, module_name: &str, args: &[&str]) -> Vec<Out> {
    if args.is_empty() { return show_extension_commands(registry, module_name); }
    let cmd_name = match registry.resolve_command(module_name, args[0]) {
        Some(n) => n,
        None => return vec![Out::Err(format!("✗ Unknown command '{}' for '{}'", args[0], module_name))],
    };
    let instances = client.list_instances().await.unwrap_or_default();
    let instance = instances.iter().find(|i| i["module_name"].as_str() == Some(module_name));
    let instance = match instance {
        Some(i) => i,
        None => return vec![Out::Err(format!("✗ No instance for module '{}'", module_name))],
    };
    let instance_name = instance["name"].as_str().unwrap_or("?").to_string();
    let instance_id = instance["id"].as_str().unwrap_or("").to_string();

    if LIFECYCLE_COMMANDS.contains(&cmd_name.as_str()) {
        match cmd_name.as_str() {
            "start" => match client.start_server(&instance_name, module_name).await {
                Ok(r) => vec![Out::Ok(format!("✓ {} — {}", instance_name, r.get("message").and_then(|v| v.as_str()).unwrap_or("Started")))],
                Err(e) => vec![Out::Err(format!("✗ {}", e))],
            },
            "stop" => match client.stop_server(&instance_name, false).await {
                Ok(r) => vec![Out::Ok(format!("✓ {} — {}", instance_name, r.get("message").and_then(|v| v.as_str()).unwrap_or("Stopped")))],
                Err(e) => vec![Out::Err(format!("✗ {}", e))],
            },
            "restart" => {
                if let Err(e) = client.stop_server(&instance_name, false).await { return vec![Out::Err(format!("✗ {}", e))]; }
                tokio::time::sleep(Duration::from_secs(1)).await;
                match client.start_server(&instance_name, module_name).await {
                    Ok(_) => vec![Out::Ok(format!("✓ {} — Restarted", instance_name))],
                    Err(e) => vec![Out::Err(format!("✗ {}", e))],
                }
            }
            "status" => match client.get_server_status(&instance_name).await {
                Ok(s) => vec![Out::Text(format!("{} — {} | PID {} | Up {}", instance_name, s["status"].as_str().unwrap_or("?"), s["pid"].as_u64().map(|p| p.to_string()).unwrap_or("-".into()), format_uptime(s["start_time"].as_u64())))],
                Err(e) => vec![Out::Err(format!("✗ {}", e))],
            },
            _ => vec![],
        }
    } else {
        let arg_value = if let Some(cmd_def) = registry.get_command_def(module_name, &cmd_name) {
            build_args_map(&cmd_def.inputs, &args[1..])
        } else {
            let mut map = serde_json::Map::new();
            if args.len() > 1 { map.insert("args".into(), serde_json::Value::String(args[1..].join(" "))); }
            serde_json::Value::Object(map)
        };
        match client.execute_command(&instance_id, &cmd_name, Some(arg_value)).await {
            Ok(r) => vec![Out::Ok(format!("✓ {} — {}", instance_name, r.get("message").or(r.get("result")).and_then(|v| v.as_str()).unwrap_or("OK")))],
            Err(e) => vec![Out::Err(format!("✗ {}", e))],
        }
    }
}

fn build_args_map(inputs: &[crate::extension_registry::CommandInput], args: &[&str]) -> serde_json::Value {
    let mut map = serde_json::Map::new();
    let mut arg_idx = 0;
    for (i, input) in inputs.iter().enumerate() {
        if arg_idx >= args.len() { break; }
        if i == inputs.len() - 1 && input.input_type == "text" {
            map.insert(input.name.clone(), serde_json::Value::String(args[arg_idx..].join(" ")));
            break;
        }
        if input.input_type == "number" {
            if let Ok(n) = args[arg_idx].parse::<i64>() { map.insert(input.name.clone(), serde_json::json!(n)); }
            else { map.insert(input.name.clone(), serde_json::Value::String(args[arg_idx].to_string())); }
        } else {
            map.insert(input.name.clone(), serde_json::Value::String(args[arg_idx].to_string()));
        }
        arg_idx += 1;
    }
    serde_json::Value::Object(map)
}
