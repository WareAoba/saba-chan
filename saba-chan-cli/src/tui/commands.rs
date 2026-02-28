//! 레거시 커맨드 디스패치 — 기존 명령어 입력 방식 호환
//!
//! `:` 키로 진입하는 커맨드 모드에서 사용됩니다.
//! `submit()`이 호출되면 동기 → 비동기 순으로 명령을 시도합니다.

use std::time::Duration;

use serde_json::Value;

use super::app::*;
use crate::cli_config::CliSettings;
use crate::client::DaemonClient;
use crate::gui_config;
use crate::module_registry::{ModuleRegistry, LIFECYCLE_COMMANDS};
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
                    Ok(Err(e)) => lines.push(Out::Err(format!("Saba-Core stop failed: {}", e))),
                    Err(e) => lines.push(Out::Err(format!("Saba-Core stop failed: {}", e))),
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
        // CommandMode를 스택에서 제거 후 Dashboard로 교체
        app.pop_screen();
        app.push_screen(Screen::Dashboard);
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
            Some("instance") => exec_instance(&client, &lower_parts[1..], &orig_parts[1..], &registry).await,
            Some("module") => exec_module(&client, &lower_parts[1..]).await,
            Some("extension") | Some("ext") => exec_extension(&client, &lower_parts[1..]).await,
            Some("daemon") => exec_daemon(&lower_parts[1..]).await,
            Some("bot") => exec_bot(&lower_parts[1..]).await,
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

/// 서브커맨드 목록 반환 (자동완성용)
fn get_subcommands_for(first: &str) -> Vec<&'static str> {
    match first {
        "instance" => vec!["list", "create", "delete", "set", "reset", "reorder"],
        "module" => vec!["list", "info", "refresh", "versions", "install", "registry", "remove", "install-registry"],
        "extension" | "ext" => vec!["list", "enable", "disable", "install", "remove", "registry", "rescan"],
        "daemon" => vec!["start", "stop", "status", "restart"],
        "bot" => vec!["start", "stop", "status", "token", "prefix", "mode", "relay", "node-token"],
        "config" => vec!["show", "set", "get", "reset", "gui", "system-language"],
        "update" => vec!["check", "status", "download", "apply", "config", "set", "install", "launch-apply"],
        "migration" => vec!["scan"],
        _ => vec![],
    }
}

/// 실시간 자동완성 미리보기 갱신 (입력할 때마다 호출)
pub fn update_autocomplete_preview(app: &mut App) {
    let input = app.input.trim().to_string();
    if input.is_empty() {
        app.autocomplete_candidates.clear();
        app.autocomplete_visible = false;
        return;
    }

    let parts: Vec<&str> = input.split_whitespace().collect();
    let mut candidates = Vec::new();

    if parts.len() == 1 && !app.input.ends_with(' ') {
        // 1단계 명령어 후보
        let top_cmds = [
            "instance", "module", "extension",
            "daemon", "bot", "config", "update", "help",
            "exit", "back", "menu", "migration",
        ];
        for cmd in &top_cmds {
            if cmd.starts_with(&parts[0].to_lowercase()) && *cmd != parts[0] {
                candidates.push(cmd.to_string());
            }
        }
        // 모듈 이름도 추가
        for name in app.registry.module_names() {
            if name.to_lowercase().starts_with(&parts[0].to_lowercase()) && name != parts[0] {
                candidates.push(name.to_string());
            }
        }
    } else if parts.len() == 2 || (parts.len() == 1 && app.input.ends_with(' ')) {
        // 2단계 서브커맨드 후보
        let first = parts[0];
        let partial = if parts.len() > 1 && !app.input.ends_with(' ') { parts[1] } else { "" };
        let sub_cmds = get_subcommands_for(first);
        for sub in &sub_cmds {
            if sub.starts_with(&partial.to_lowercase()) && (partial.is_empty() || *sub != partial) {
                candidates.push(format!("{} {}", first, sub));
            }
        }
    }

    app.autocomplete_candidates = candidates;
    app.autocomplete_selected = 0;
    app.autocomplete_visible = !app.autocomplete_candidates.is_empty();
}

pub fn autocomplete(app: &mut App) {
    // 이미 후보가 보이고 있으면 → 선택된 항목으로 입력 적용
    if app.autocomplete_visible && !app.autocomplete_candidates.is_empty() {
        if let Some(candidate) = app.autocomplete_candidates.get(app.autocomplete_selected) {
            app.input = format!("{} ", candidate);
            app.cursor = app.input.chars().count();
        }
        app.autocomplete_candidates.clear();
        app.autocomplete_visible = false;
        return;
    }

    let input = app.input.trim().to_string();
    if input.is_empty() { return; }
    let parts: Vec<&str> = input.split_whitespace().collect();

    if parts.len() == 1 && !app.input.ends_with(' ') {
        let mut candidates: Vec<String> = vec![
            "instance".into(), "module".into(), "extension".into(),
            "daemon".into(), "bot".into(),
            "exec".into(), "config".into(), "help".into(), "exit".into(), "update".into(),
            "back".into(), "menu".into(), "migration".into(),
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
                "instance list", "instance create", "instance delete",
                "instance set", "instance reset", "instance reorder",
                "module list", "module refresh", "module versions", "module install", "module info",
                "module registry", "module remove", "module install-registry",
                "extension list", "extension enable", "extension disable",
                "extension install", "extension remove", "extension registry", "extension rescan",
                "daemon start", "daemon stop", "daemon status", "daemon restart",
                "bot start", "bot stop", "bot status", "bot token", "bot prefix",
                "bot mode", "bot relay", "bot node-token",
                "config show", "config set", "config get", "config reset", "config system-language",
                "update check", "update status", "update download", "update apply",
                "update launch-apply",
                "migration scan",
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
        Some("config") if lower.len() >= 2 && (lower[1] == "system-language" || lower[1] == "system-lang") => {
            Some(vec![Out::Ok(format!("System language: {}", gui_config::get_system_language()))])
        }
        Some("config") => Some(cmd_config(app, &orig[1..])),
        Some("help") => Some(cmd_help(app)),
        // 서브커맨드 없이 카테고리만 입력 → 간단 도움말
        Some("instance") if lower.len() == 1 => Some(vec![
            Out::Text("  instance list|create|delete|set|reset|reorder <name>".into()),
        ]),
        Some("module") if lower.len() == 1 => Some(vec![
            Out::Text("  module list|info|refresh|versions|install|registry|remove|install-registry".into()),
        ]),
        Some("extension") | Some("ext") if lower.len() == 1 => Some(vec![
            Out::Text("  extension list|enable|disable|install|remove|registry|rescan".into()),
        ]),
        Some("update") if lower.len() == 1 => Some(vec![
            Out::Text("  update check|status|download|apply|config|set|install".into()),
        ]),
        Some("daemon") if lower.len() == 1 => Some(vec![
            Out::Text("  daemon start|stop|status|restart".into()),
        ]),
        Some("bot") if lower.len() == 1 => Some(vec![
            Out::Text("  bot start|stop|status|token|prefix|mode|relay|node-token".into()),
        ]),
        Some("migration") if lower.len() == 1 => Some(vec![
            Out::Text("  migration scan <directory>".into()),
        ]),
        Some("bot") if lower.len() >= 2 && lower[1] == "token" => Some(cmd_bot_token(&orig[2..])),
        Some("bot") if lower.len() >= 2 && lower[1] == "prefix" => Some(cmd_bot_prefix(&orig[2..])),
        Some("bot") if lower.len() >= 2 && lower[1] == "status" => Some(cmd_bot_status()),
        Some("bot") if lower.len() >= 2 && lower[1] == "mode" => Some(cmd_bot_mode(&lower[2..], &orig[2..])),
        Some("bot") if lower.len() >= 2 && lower[1] == "relay" => Some(cmd_bot_relay(&lower[2..], &orig[2..])),
        Some("bot") if lower.len() >= 2 && lower[1] == "node-token" => Some(cmd_bot_node_token(&lower[2..], &orig[2..])),
        Some("migration") if lower.len() >= 2 && lower[1] == "scan" => Some(cmd_migration_scan(&orig[2..])),
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
            let modules_path = gui_config::get_modules_path().unwrap_or_default();
            let extensions_path = gui_config::get_extensions_path().unwrap_or_default();
            let prefix = gui_config::get_bot_prefix().unwrap_or_else(|_| "!saba".into());
            let gui_lang = gui_config::get_language().unwrap_or_else(|_| "en".into());
            let auto_start_gui = gui_config::get_discord_auto_start().unwrap_or(false);
            let auto_refresh = gui_config::get_auto_refresh().unwrap_or(true);
            let refresh_ms = gui_config::get_refresh_interval().unwrap_or(2000);
            let ipc_port = gui_config::get_ipc_port();
            let console_buf = gui_config::get_console_buffer_size().unwrap_or(2000);
            let auto_pwd = gui_config::get_auto_generate_passwords().unwrap_or(true);
            let port_check = gui_config::get_port_conflict_check().unwrap_or(true);
            let mut lines = vec![
                Out::Info("CLI Settings:".into()),
                Out::Text(format!("  language         {}", app.settings.get_value("language").unwrap_or_else(|| "(auto)".into()))),
                Out::Text(format!("  auto_start       {}", app.settings.auto_start)),
                Out::Text(format!("  refresh_interval {}", app.settings.refresh_interval)),
                Out::Text(format!("  bot_prefix       {}", app.settings.bot_prefix)),
                Out::Blank,
                Out::Info("GUI Settings (settings.json):".into()),
                Out::Text(format!("  token              {}", if token.is_some() { "✓ set" } else { "✗ not set" })),
                Out::Text(format!("  prefix             {}", prefix)),
                Out::Text(format!("  modules_path       {} (fixed)", modules_path)),
                Out::Text(format!("  extensions_path    {} (fixed)", extensions_path)),
                Out::Text(format!("  language           {}", gui_lang)),
                Out::Text(format!("  discord_auto       {}", auto_start_gui)),
                Out::Text(format!("  auto_refresh       {}", auto_refresh)),
                Out::Text(format!("  refresh_interval   {}ms", refresh_ms)),
                Out::Text(format!("  ipc_port           {}", ipc_port)),
                Out::Text(format!("  console_buffer     {}", console_buf)),
                Out::Text(format!("  auto_passwords     {}", auto_pwd)),
                Out::Text(format!("  port_check         {}", port_check)),
            ];
            lines.push(Out::Blank);
            lines.push(Out::Info("CLI: config set|get|reset <key> <value>".into()));
            lines.push(Out::Info("GUI: config gui <key> <value>".into()));
            lines.push(Out::Text("  GUI keys: language, token, discord_auto, auto_refresh,".into()));
            lines.push(Out::Text("            refresh_interval, ipc_port, console_buffer, auto_passwords, port_check".into()));
            lines.push(Out::Text(format!("  CLI keys: {}", CliSettings::available_keys().iter().map(|(k,_)| *k).collect::<Vec<_>>().join(", "))));
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
        Some("discord_auto") | Some("discord_auto_start") => {
            if args.len() < 2 {
                let cur = gui_config::get_discord_auto_start().unwrap_or(false);
                return vec![Out::Ok(format!("Discord auto-start: {}", cur))];
            }
            match args[1].parse::<bool>() {
                Ok(v) => match gui_config::set_discord_auto_start(v) {
                    Ok(()) => vec![Out::Ok(format!("✓ Discord auto-start set to: {}", v))],
                    Err(e) => vec![Out::Err(format!("✗ {}", e))],
                },
                Err(_) => vec![Out::Err("Expected true/false".into())],
            }
        }
        Some("auto_refresh") => {
            if args.len() < 2 {
                let cur = gui_config::get_auto_refresh().unwrap_or(true);
                return vec![Out::Ok(format!("Auto-refresh: {}", cur))];
            }
            match args[1].parse::<bool>() {
                Ok(v) => match gui_config::set_auto_refresh(v) {
                    Ok(()) => vec![Out::Ok(format!("✓ Auto-refresh set to: {}", v))],
                    Err(e) => vec![Out::Err(format!("✗ {}", e))],
                },
                Err(_) => vec![Out::Err("Expected true/false".into())],
            }
        }
        Some("refresh_interval") => {
            if args.len() < 2 {
                let cur = gui_config::get_refresh_interval().unwrap_or(2000);
                return vec![Out::Ok(format!("Refresh interval: {}ms", cur))];
            }
            match args[1].parse::<u64>() {
                Ok(ms) if ms >= 500 && ms <= 60000 => match gui_config::set_refresh_interval(ms) {
                    Ok(()) => vec![Out::Ok(format!("✓ Refresh interval set to: {}ms", ms))],
                    Err(e) => vec![Out::Err(format!("✗ {}", e))],
                },
                Ok(_) => vec![Out::Err("Must be 500-60000 (ms)".into())],
                Err(_) => vec![Out::Err("Expected a number (ms)".into())],
            }
        }
        Some("ipc_port") | Some("port") => {
            if args.len() < 2 {
                let cur = gui_config::get_ipc_port();
                return vec![Out::Ok(format!("IPC port: {}", cur))];
            }
            match args[1].parse::<u16>() {
                Ok(p) if p >= 1024 => match gui_config::set_ipc_port(p) {
                    Ok(()) => vec![Out::Ok(format!("✓ IPC port set to: {} (restart required)", p))],
                    Err(e) => vec![Out::Err(format!("✗ {}", e))],
                },
                Ok(_) => vec![Out::Err("Port must be >= 1024".into())],
                Err(_) => vec![Out::Err("Expected a port number".into())],
            }
        }
        Some("console_buffer") | Some("console_buffer_size") => {
            if args.len() < 2 {
                let cur = gui_config::get_console_buffer_size().unwrap_or(2000);
                return vec![Out::Ok(format!("Console buffer size: {}", cur))];
            }
            match args[1].parse::<u64>() {
                Ok(n) if n >= 100 && n <= 50000 => match gui_config::set_console_buffer_size(n) {
                    Ok(()) => vec![Out::Ok(format!("✓ Console buffer size set to: {}", n))],
                    Err(e) => vec![Out::Err(format!("✗ {}", e))],
                },
                Ok(_) => vec![Out::Err("Must be 100-50000".into())],
                Err(_) => vec![Out::Err("Expected a number".into())],
            }
        }
        Some("auto_generate_passwords") | Some("auto_passwords") => {
            if args.len() < 2 {
                let cur = gui_config::get_auto_generate_passwords().unwrap_or(true);
                return vec![Out::Ok(format!("Auto-generate passwords: {}", cur))];
            }
            match args[1].parse::<bool>() {
                Ok(v) => match gui_config::set_auto_generate_passwords(v) {
                    Ok(()) => vec![Out::Ok(format!("✓ Auto-generate passwords set to: {}", v))],
                    Err(e) => vec![Out::Err(format!("✗ {}", e))],
                },
                Err(_) => vec![Out::Err("Expected true/false".into())],
            }
        }
        Some("port_conflict_check") | Some("port_check") => {
            if args.len() < 2 {
                let cur = gui_config::get_port_conflict_check().unwrap_or(true);
                return vec![Out::Ok(format!("Port conflict check: {}", cur))];
            }
            match args[1].parse::<bool>() {
                Ok(v) => match gui_config::set_port_conflict_check(v) {
                    Ok(()) => vec![Out::Ok(format!("✓ Port conflict check set to: {}", v))],
                    Err(e) => vec![Out::Err(format!("✗ {}", e))],
                },
                Err(_) => vec![Out::Err("Expected true/false".into())],
            }
        }
        _ => vec![Out::Err("Usage: config gui [language|token|discord_auto|auto_refresh|refresh_interval|ipc_port|console_buffer|auto_passwords|port_check] <value>".into())],
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

fn cmd_bot_mode(lower: &[&str], _orig: &[&str]) -> Vec<Out> {
    let config = gui_config::load_bot_config().unwrap_or_default();
    let current = config.get("mode").and_then(|v| v.as_str()).unwrap_or("local");

    match lower.first().copied() {
        None | Some("show") => {
            let mut lines = vec![Out::Ok(format!("Bot mode: {}", current))];
            lines.push(Out::Text("  Available: local, cloud".into()));
            lines.push(Out::Text("  bot mode set <local|cloud>".into()));
            lines
        }
        Some("set") if lower.len() > 1 => {
            let mode = lower[1];
            if mode != "local" && mode != "cloud" {
                return vec![Out::Err("Mode must be 'local' or 'cloud'".into())];
            }
            let mut config = gui_config::load_bot_config().unwrap_or_default();
            config["mode"] = serde_json::Value::String(mode.to_string());
            let path = gui_config::get_bot_config_path_pub();
            match save_json_file(&path, &config) {
                Ok(()) => vec![Out::Ok(format!("✓ Bot mode set to: {}", mode))],
                Err(e) => vec![Out::Err(format!("✗ {}", e))],
            }
        }
        _ => vec![Out::Err("Usage: bot mode [show|set <local|cloud>]".into())],
    }
}

fn cmd_bot_node_token(lower: &[&str], orig: &[&str]) -> Vec<Out> {
    match lower.first().copied() {
        None | Some("show") => {
            match gui_config::load_node_token() {
                Ok(token) if token.is_empty() => vec![Out::Text("Node token: (not set)".into())],
                Ok(token) => {
                    let masked = if token.len() > 12 {
                        format!("{}...{}", &token[..6], &token[token.len()-4..])
                    } else {
                        "****".into()
                    };
                    vec![Out::Ok(format!("Node token: {}", masked))]
                }
                Err(e) => vec![Out::Err(format!("✗ {}", e))],
            }
        }
        Some("set") if orig.len() > 1 => {
            let token = orig[1..].join(" ");
            match gui_config::save_node_token(token.trim()) {
                Ok(()) => vec![Out::Ok("✓ Node token saved.".into())],
                Err(e) => vec![Out::Err(format!("✗ {}", e))],
            }
        }
        Some("clear") => {
            match gui_config::clear_node_token() {
                Ok(()) => vec![Out::Ok("✓ Node token cleared.".into())],
                Err(e) => vec![Out::Err(format!("✗ {}", e))],
            }
        }
        _ => vec![Out::Err("Usage: bot node-token [show|set <token>|clear]".into())],
    }
}

fn cmd_migration_scan(args: &[&str]) -> Vec<Out> {
    if args.is_empty() {
        return vec![Out::Err("Usage: migration scan <directory>".into())];
    }
    let dir = args.join(" ");
    match gui_config::scan_directory(&dir) {
        Ok((files, dirs)) => {
            let mut o = vec![Out::Ok(format!("Scan: {}", dir))];
            if !dirs.is_empty() {
                o.push(Out::Info(format!("  {} directories:", dirs.len())));
                for d in &dirs {
                    o.push(Out::Text(format!("    📁 {}/", d)));
                }
            }
            if !files.is_empty() {
                o.push(Out::Info(format!("  {} files:", files.len())));
                for f in &files {
                    o.push(Out::Text(format!("    📄 {}", f)));
                }
            }
            if dirs.is_empty() && files.is_empty() {
                o.push(Out::Text("  (empty directory)".into()));
            }
            o
        }
        Err(e) => vec![Out::Err(format!("✗ {}", e))],
    }
}

fn cmd_bot_relay(lower: &[&str], orig: &[&str]) -> Vec<Out> {
    let config = gui_config::load_bot_config().unwrap_or_default();
    let cloud = config.get("cloud").cloned().unwrap_or(serde_json::json!({}));
    let relay_url = cloud.get("relayUrl").and_then(|v| v.as_str()).unwrap_or("");
    let host_id = cloud.get("hostId").and_then(|v| v.as_str()).unwrap_or("");

    match lower.first().copied() {
        None | Some("show") => {
            vec![
                Out::Ok("Cloud Relay Config:".into()),
                Out::Text(format!("  relayUrl: {}", if relay_url.is_empty() { "(not set)" } else { relay_url })),
                Out::Text(format!("  hostId:   {}", if host_id.is_empty() { "(not set)" } else { host_id })),
                Out::Blank,
                Out::Text("  bot relay url <URL>".into()),
                Out::Text("  bot relay hostid <ID>".into()),
            ]
        }
        Some("url") if orig.len() > 1 => {
            let url = orig[1..].join(" ");
            let mut config = gui_config::load_bot_config().unwrap_or_default();
            if config.get("cloud").is_none() { config["cloud"] = serde_json::json!({}); }
            config["cloud"]["relayUrl"] = serde_json::Value::String(url.clone());
            let path = gui_config::get_bot_config_path_pub();
            match save_json_file(&path, &config) {
                Ok(()) => vec![Out::Ok(format!("✓ Relay URL set to: {}", url))],
                Err(e) => vec![Out::Err(format!("✗ {}", e))],
            }
        }
        Some("hostid") if orig.len() > 1 => {
            let id = orig[1];
            let mut config = gui_config::load_bot_config().unwrap_or_default();
            if config.get("cloud").is_none() { config["cloud"] = serde_json::json!({}); }
            config["cloud"]["hostId"] = serde_json::Value::String(id.to_string());
            let path = gui_config::get_bot_config_path_pub();
            match save_json_file(&path, &config) {
                Ok(()) => vec![Out::Ok(format!("✓ Host ID set to: {}", id))],
                Err(e) => vec![Out::Err(format!("✗ {}", e))],
            }
        }
        _ => vec![Out::Err("Usage: bot relay [show|url <URL>|hostid <ID>]".into())],
    }
}

fn cmd_help(app: &App) -> Vec<Out> {
    let mut lines = vec![
        Out::Info("─── Commands ───".into()),
        Out::Text("  instance  [list|create|delete|set|reset|reorder] <name>".into()),
        Out::Text("  module    [list|info|refresh|versions|install|registry|remove|install-registry]".into()),
        Out::Text("  extension [list|enable|disable|install|remove|registry|rescan]".into()),
        Out::Text("  daemon    [start|stop|status|restart]".into()),
        Out::Text("  bot       [start|stop|status|token|prefix|mode|relay|node-token]".into()),
        Out::Text("  update    [check|status|download|apply|config|install|launch-apply]".into()),
        Out::Text("  config    [show|set|get|reset|gui|system-language]".into()),
        Out::Text("  migration [scan] <directory>".into()),
        Out::Text("  menu     — Interactive menu mode (F2)".into()),
        Out::Text("  help     — This help".into()),
        Out::Text("  exit     — Quit (Ctrl+C)".into()),
    ];

    if !app.registry.modules.is_empty() {
        lines.push(Out::Blank);
        lines.push(Out::Info("Module shortcuts:".into()));
        for mi in &app.registry.modules {
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

fn show_extension_commands(registry: &ModuleRegistry, module_name: &str) -> Vec<Out> {
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

/// 이름 또는 #N / 숫자 인덱스를 실제 인스턴스 이름으로 변환
fn resolve_name_by_index(instances: &[Value], input: &str) -> String {
    let idx_str = input.strip_prefix('#').unwrap_or(input);
    if let Ok(idx) = idx_str.parse::<usize>() {
        if idx >= 1 && idx <= instances.len() {
            if let Some(name) = instances[idx - 1]["name"].as_str() {
                return name.to_string();
            }
        }
    }
    input.to_string()
}

async fn find_instance_id_by_name(client: &DaemonClient, name: &str) -> Option<String> {
    if let Ok(instances) = client.list_instances().await {
        let resolved = resolve_name_by_index(&instances, name);
        for inst in &instances {
            if inst["name"].as_str() == Some(&resolved) {
                return inst["id"].as_str().map(|s| s.to_string());
            }
        }
    }
    None
}

async fn exec_instance(client: &DaemonClient, lower: &[&str], orig: &[&str], registry: &ModuleRegistry) -> Vec<Out> {
    match lower.first().copied() {
        Some("list") => match client.list_instances().await {
            Ok(list) if list.is_empty() => vec![Out::Text("No instances configured.".into())],
            Ok(list) => {
                let mut o = vec![Out::Ok(format!("{} instance(s):", list.len()))];
                for (i, inst) in list.iter().enumerate() {
                    o.push(Out::Text(format!("  #{} {} [{}] id:{}",
                        i + 1, inst["name"].as_str().unwrap_or("?"), inst["module_name"].as_str().unwrap_or("?"), inst["id"].as_str().unwrap_or("?"))));
                }
                o.push(Out::Blank);
                o.push(Out::Text("  Tip: 이름 대신 #번호 사용 가능 (예: instance show #1)".into()));
                o
            }
            Err(e) => vec![Out::Err(format!("✗ {}", e))],
        },
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
        Some("reset") if orig.len() > 1 => {
            let name = orig[1];
            match client.reset_server(name).await {
                Ok(r) => vec![Out::Ok(format!("✓ {} — {}", name, r.get("message").and_then(|v| v.as_str()).unwrap_or("Instance reset initiated")))],
                Err(e) => vec![Out::Err(format!("✗ {}", e))],
            }
        }
        Some("reorder") if orig.len() > 1 => {
            let ids: Vec<serde_json::Value> = orig[1..].iter().map(|s| serde_json::Value::String(s.to_string())).collect();
            match client.reorder_instances(serde_json::json!(ids)).await {
                Ok(_) => vec![Out::Ok("✓ Instance order updated".into())],
                Err(e) => vec![Out::Err(format!("✗ {}", e))],
            }
        }
        _ => vec![Out::Err("Usage: instance [list|create|delete|set|reset|reorder] <name>".into())],
    }
}

async fn exec_module(client: &DaemonClient, args: &[&str]) -> Vec<Out> {
    match args.first().copied() {
        Some("list") => match client.list_modules().await {
            Ok(list) if list.is_empty() => vec![Out::Text("No modules loaded.".into())],
            Ok(list) => {
                let mut o = vec![Out::Ok(format!("{} module(s):", list.len()))];
                for m in &list { o.push(Out::Text(format!("  • {} v{} [{}]", m["name"].as_str().unwrap_or("?"), m["version"].as_str().unwrap_or("?"), m["interaction_mode"].as_str().unwrap_or("-")))); }
                o
            }
            Err(e) => vec![Out::Err(format!("✗ {}", e))],
        },
        Some("info") if args.len() > 1 => {
            match client.get_module(args[1]).await {
                Ok(data) => {
                    let mut o = vec![Out::Ok(format!("Module: {}", args[1]))];
                    for key in &["name", "version", "description", "game_name", "display_name", "interaction_mode"] {
                        if let Some(val) = data.get(*key).and_then(|v: &Value| v.as_str()) { o.push(Out::Text(format!("  {:<20} {}", key, val))); }
                    }
                    o
                }
                Err(e) => vec![Out::Err(format!("✗ {}", e))],
            }
        }
        Some("refresh") | Some("reload") => match client.refresh_modules().await {
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
        Some("registry") => match client.fetch_module_registry().await {
            Ok(v) => {
                let list = v.as_array().cloned().unwrap_or_default();
                if list.is_empty() { return vec![Out::Text("Module registry is empty.".into())]; }
                let mut o = vec![Out::Ok(format!("{} module(s) in registry:", list.len()))];
                for m in &list {
                    let name = m["name"].as_str().or(m["id"].as_str()).unwrap_or("?");
                    let ver = m["version"].as_str().unwrap_or("-");
                    let desc = m["description"].as_str().unwrap_or("");
                    o.push(Out::Text(format!("  • {} v{} — {}", name, ver, desc)));
                }
                o
            }
            Err(e) => vec![Out::Err(format!("✗ {}", e))],
        },
        Some("remove") if args.len() > 1 => match client.remove_module(args[1]).await {
            Ok(r) => vec![Out::Ok(format!("✓ {}", r.get("message").and_then(|v| v.as_str()).unwrap_or("Module removed")))],
            Err(e) => vec![Out::Err(format!("✗ {}", e))],
        },
        Some("install-registry") if args.len() > 1 => match client.install_module_from_registry(args[1]).await {
            Ok(r) => vec![Out::Ok(format!("✓ {}", r.get("message").and_then(|v| v.as_str()).unwrap_or("Module installed from registry")))],
            Err(e) => vec![Out::Err(format!("✗ {}", e))],
        },
        _ => vec![Out::Err("Usage: module [list|info|refresh|versions|install|registry|remove|install-registry] <name>".into())],
    }
}

async fn exec_extension(client: &DaemonClient, args: &[&str]) -> Vec<Out> {
    match args.first().copied() {
        Some("list") => match client.list_extensions().await {
            Ok(list) if list.is_empty() => vec![Out::Text("No extensions installed.".into())],
            Ok(list) => {
                let mut o = vec![Out::Ok(format!("{} extension(s):", list.len()))];
                for ext in &list {
                    let name = ext["name"].as_str().or(ext["id"].as_str()).unwrap_or("?");
                    let ver = ext["version"].as_str().unwrap_or("-");
                    let enabled = ext["enabled"].as_bool().unwrap_or(false);
                    let marker = if enabled { "●" } else { "○" };
                    o.push(Out::Text(format!("  {} {} v{}", marker, name, ver)));
                }
                o
            }
            Err(e) => vec![Out::Err(format!("✗ {}", e))],
        },
        Some("enable") if args.len() > 1 => match client.enable_extension(args[1]).await {
            Ok(r) => vec![Out::Ok(format!("✓ {}", r.get("message").and_then(|v| v.as_str()).unwrap_or("Extension enabled")))],
            Err(e) => vec![Out::Err(format!("✗ {}", e))],
        },
        Some("disable") if args.len() > 1 => match client.disable_extension(args[1]).await {
            Ok(r) => vec![Out::Ok(format!("✓ {}", r.get("message").and_then(|v| v.as_str()).unwrap_or("Extension disabled")))],
            Err(e) => vec![Out::Err(format!("✗ {}", e))],
        },
        Some("install") if args.len() > 1 => match client.install_extension(args[1], None).await {
            Ok(r) => vec![Out::Ok(format!("✓ {}", r.get("message").and_then(|v| v.as_str()).unwrap_or("Extension installed")))],
            Err(e) => vec![Out::Err(format!("✗ {}", e))],
        },
        Some("remove") if args.len() > 1 => match client.remove_extension(args[1]).await {
            Ok(r) => vec![Out::Ok(format!("✓ {}", r.get("message").and_then(|v| v.as_str()).unwrap_or("Extension removed")))],
            Err(e) => vec![Out::Err(format!("✗ {}", e))],
        },
        Some("registry") => match client.fetch_extension_registry().await {
            Ok(v) => {
                let list = v.as_array().cloned().unwrap_or_default();
                if list.is_empty() { return vec![Out::Text("Extension registry is empty.".into())]; }
                let mut o = vec![Out::Ok(format!("{} extension(s) in registry:", list.len()))];
                for ext in &list {
                    let name = ext["name"].as_str().or(ext["id"].as_str()).unwrap_or("?");
                    let ver = ext["version"].as_str().unwrap_or("-");
                    let desc = ext["description"].as_str().unwrap_or("");
                    o.push(Out::Text(format!("  • {} v{} — {}", name, ver, desc)));
                }
                o
            }
            Err(e) => vec![Out::Err(format!("✗ {}", e))],
        },
        Some("rescan") => match client.rescan_extensions().await {
            Ok(r) => vec![Out::Ok(format!("✓ {}", r.get("message").and_then(|v| v.as_str()).unwrap_or("Extensions rescanned")))],
            Err(e) => vec![Out::Err(format!("✗ {}", e))],
        },
        _ => vec![Out::Err("Usage: extension [list|enable|disable|install|remove|registry|rescan] <id>".into())],
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
            if running { vec![Out::Ok("Saba-Core: ● RUNNING".into())] } else { vec![Out::Text("Saba-Core: ○ OFFLINE".into())] }
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
        _ => vec![Out::Err("Usage: bot [start|stop] (token/prefix/mode/relay는 TUI Bot 메뉴 이용)".into())],
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
                lines.push(Out::Blank);
                lines.push(Out::Info("Set config: update set <key> <value>".into()));
                lines
            }
            Err(e) => vec![Out::Err(format!("✗ {}", e))],
        },
        Some("set") if args.len() >= 3 => {
            let key = args[1];
            let value = args[2..].join(" ");
            let json_value = if value == "true" { serde_json::Value::Bool(true) }
                else if value == "false" { serde_json::Value::Bool(false) }
                else if let Ok(n) = value.parse::<i64>() { serde_json::json!(n) }
                else { serde_json::Value::String(value.clone()) };
            match client.set_update_config(serde_json::json!({ key: json_value })).await {
                Ok(_) => vec![Out::Ok(format!("✓ updater.{} = {}", key, value))],
                Err(e) => vec![Out::Err(format!("✗ {}", e))],
            }
        }
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
        Some("launch-apply") => {
            // updater exe 찾기
            let root = process::find_project_root().unwrap_or_else(|_| std::env::current_dir().unwrap_or_default());
            let candidates = [
                root.join("target").join("release").join("saba-chan-updater.exe"),
                root.join("target").join("debug").join("saba-chan-updater.exe"),
                root.join("saba-chan-updater.exe"),
                root.join("updater").join("gui").join("src-tauri").join("target").join("release").join("saba-chan-updater.exe"),
            ];
            let updater_exe = candidates.iter().find(|p| p.exists());
            match updater_exe {
                Some(exe) => {
                    let install_root = root.to_string_lossy().to_string();
                    let mut cmd_args = vec!["--apply".to_string(), "--install-root".to_string(), install_root];
                    // targets 추가
                    if args.len() > 1 {
                        cmd_args.extend(args[1..].iter().map(|s| s.to_string()));
                    }
                    match std::process::Command::new(exe)
                        .args(&cmd_args)
                        .stdin(std::process::Stdio::null())
                        .stdout(std::process::Stdio::null())
                        .stderr(std::process::Stdio::null())
                        .spawn()
                    {
                        Ok(_) => vec![Out::Ok(format!("✓ Updater launched: {}", exe.display()))],
                        Err(e) => vec![Out::Err(format!("✗ Failed to launch updater: {}", e))],
                    }
                }
                None => vec![Out::Err("✗ Updater executable not found. Searched in target/release and target/debug.".into())],
            }
        }
        _ => vec![Out::Text("  update check|status|download|apply|config|set|install|launch-apply [targets...]".into())],
    }
}

// ═══ 모듈 단축 명령어 (palworld start, 팰월드 시작 등) ═══

async fn exec_module_cmd(client: &DaemonClient, registry: &ModuleRegistry, module_name: &str, args: &[&str]) -> Vec<Out> {
    if args.is_empty() { return show_extension_commands(registry, module_name); }
    let cmd_name: String = match registry.resolve_command(module_name, args[0]) {
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

fn build_args_map(inputs: &[crate::module_registry::CommandInput], args: &[&str]) -> serde_json::Value {
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
