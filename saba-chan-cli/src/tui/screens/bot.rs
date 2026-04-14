//! 디스코드 봇 화면 — 시작/정지, 토큰, 프리픽스, 별명

use ratatui::prelude::*;
use ratatui::widgets::{Block, BorderType, Borders, Paragraph};

use crate::tui::app::*;
use crate::tui::theme::Theme;
use crate::config;
use crate::process;

pub(super) fn build_bot_menu(app: &App) -> Vec<MenuItem> {
    let t = |k| app.i18n.t(k);
    let is_running = app.bot_on;
    let bot_config = config::load_bot_config().unwrap_or_default();
    let music_on = bot_config.get("musicEnabled")
        .and_then(|v| v.as_bool())
        .unwrap_or(false);
    let mode = bot_config.get("mode")
        .and_then(|v| v.as_str())
        .unwrap_or("local");
    let is_cloud = mode == "cloud";

    let mut items = vec![
        if is_running {
            MenuItem::new(&format!("■ {}", t("screen.bot_stop")), Some('s'), &t("screen.bot_stop"))
        } else {
            MenuItem::new(&format!("▶ {}", t("screen.bot_start")), Some('s'), &t("screen.bot_start"))
        },
        MenuItem::new(&format!("🔑 {}", t("screen.bot_token")), Some('t'), &t("screen.bot_token")),
        MenuItem::new(&format!("📝 {}", t("screen.bot_prefix")), Some('p'), &t("screen.bot_prefix")),
        MenuItem::new(&format!("🎵 {}", t("screen.bot_music")), Some('m'), &format!(
            "{}: {}",
            t("screen.bot_music"),
            if music_on { "ON" } else { "OFF" },
        )),
        MenuItem::new(&format!("🔄 {}", t("screen.bot_mode")), Some('M'), &format!(
            "{}: {}",
            t("screen.bot_mode"),
            mode,
        )),
    ];

    // 클라우드 모드 전용 옵션 — local 모드에서는 비활성화
    if is_cloud {
        items.push(MenuItem::new(&format!("🌐 {}", t("screen.bot_relay_url")), Some('R'), &t("screen.bot_relay_url")));
        items.push(MenuItem::new(&format!("🏠 {}", t("screen.bot_relay_host_id")), Some('H'), &t("screen.bot_relay_host_id")));
        items.push(MenuItem::new(&format!("🔐 {}", t("screen.bot_node_token")), Some('N'), &t("screen.bot_node_token")));
    } else {
        items.push(MenuItem::new(&format!("🌐 {}", t("screen.bot_relay_url")), None, "cloud mode only").with_enabled(false));
        items.push(MenuItem::new(&format!("🏠 {}", t("screen.bot_relay_host_id")), None, "cloud mode only").with_enabled(false));
        items.push(MenuItem::new(&format!("🔐 {}", t("screen.bot_node_token")), None, "cloud mode only").with_enabled(false));
    }

    items.push(MenuItem::new(&format!("🏷 {}", t("screen.bot_aliases")), Some('a'), &t("screen.bot_aliases")));
    items.push(MenuItem::new(&format!("⚙ {}", t("screen.bot_autostart")), Some('A'), &format!(
        "{}: {}",
        t("screen.bot_autostart"),
        if config::get_discord_auto_start().unwrap_or(false) { "ON" } else { "OFF" },
    )));

    // 봇 콘솔 (데몬 API 경유)
    if is_running {
        items.push(MenuItem::new(&format!("📟 {}", t("screen.bot_console")), Some('c'), &t("screen.bot_console")));
    }

    items
}

pub(super) fn build_bot_aliases_menu(app: &App) -> Vec<MenuItem> {
    let t = |k| app.i18n.t(k);
    vec![
        MenuItem::new(&format!("📝 {}", t("screen.bot_alias_set_module")), Some('m'), &t("screen.bot_alias_set_module")),
        MenuItem::new(&format!("📝 {}", t("screen.bot_alias_set_command")), Some('c'), &t("screen.bot_alias_set_command")),
        MenuItem::new(&format!("🔄 {}", t("screen.bot_alias_reset")), Some('R'), &t("screen.bot_alias_reset")),
    ]
}

pub(super) fn render_bot_aliases(_app: &App, frame: &mut Frame, area: Rect) {
    let block = Block::default()
        .title(" Bot Aliases — [Esc] Back ")
        .borders(Borders::ALL)
        .border_type(BorderType::Rounded)
        .border_style(Theme::border());

    let inner = block.inner(area);
    frame.render_widget(block, area);

    // 봇 별명 데이터를 출력
    let config = config::load_bot_config().unwrap_or_default();
    let mut lines = Vec::new();

    lines.push(Line::from(Span::styled("  Module Aliases:", Theme::group_header())));
    if let Some(aliases) = config.get("moduleAliases").and_then(|v| v.as_object()) {
        if aliases.is_empty() {
            lines.push(Line::from("    (none)"));
        } else {
            for (module, alias) in aliases {
                lines.push(Line::from(format!("    {} → {}", module, alias.as_str().unwrap_or("?"))));
            }
        }
    } else {
        lines.push(Line::from("    (none)"));
    }

    lines.push(Line::from(""));
    lines.push(Line::from(Span::styled("  Command Aliases:", Theme::group_header())));
    if let Some(cmd_aliases) = config.get("commandAliases").and_then(|v| v.as_object()) {
        if cmd_aliases.is_empty() {
            lines.push(Line::from("    (none)"));
        } else {
            for (module, cmds) in cmd_aliases {
                if let Some(cmd_map) = cmds.as_object() {
                    for (cmd, alias) in cmd_map {
                        lines.push(Line::from(format!("    {}.{} → {}", module, cmd, alias.as_str().unwrap_or("?"))));
                    }
                }
            }
        }
    } else {
        lines.push(Line::from("    (none)"));
    }

    lines.push(Line::from(""));
    lines.push(Line::from(Span::styled(
        "  'm' 모듈 별명 설정 | 'c' 명령어 별명 설정 | 'R' 전체 초기화",
        Theme::dimmed(),
    )));

    frame.render_widget(
        Paragraph::new(lines),
        Rect::new(inner.x, inner.y + 1, inner.width, inner.height.saturating_sub(1)),
    );
}

pub(super) fn handle_bot_select(app: &mut App, sel: usize) {
    let shortcut = app.menu_items.get(sel).and_then(|item| item.shortcut);
    let _client = app.client.clone();
    let buf = app.async_out.clone();

    match shortcut {
        Some('s') => { // Start/Stop
            if app.bot_on {
                tokio::spawn(async move {
                    match tokio::task::spawn_blocking(process::stop_bot).await {
                        Ok(Ok(msg)) => push_out(&buf, vec![Out::Ok(msg)]),
                        Ok(Err(e)) => push_out(&buf, vec![Out::Err(format!("✗ {}", e))]),
                        Err(e) => push_out(&buf, vec![Out::Err(format!("✗ {}", e))]),
                    }
                });
            } else {
                tokio::spawn(async move {
                    match tokio::task::spawn_blocking(process::start_bot).await {
                        Ok(Ok(msg)) => push_out(&buf, vec![Out::Ok(msg)]),
                        Ok(Err(e)) => push_out(&buf, vec![Out::Err(format!("✗ {}", e))]),
                        Err(e) => push_out(&buf, vec![Out::Err(format!("✗ {}", e))]),
                    }
                });
            }
            app.flash(if app.bot_on { "봇 정지 중..." } else { "봇 시작 중..." });
        }
        Some('t') => { // Token → 인라인 Input
            let current = config::load_bot_config()
                .ok()
                .and_then(|c| c.get("token").and_then(|v| v.as_str()).map(|s| s.to_string()))
                .unwrap_or_default();
            app.input_mode = InputMode::InlineInput {
                prompt: "디스코드 봇 토큰".into(),
                value: current.clone(),
                cursor: current.chars().count(),
                on_submit: InlineAction::SetBotToken,
            };
        }
        Some('p') => { // Prefix → 인라인 Input
            app.input_mode = InputMode::InlineInput {
                prompt: "봇 명령어 프리픽스".into(),
                value: app.bot_prefix.clone(),
                cursor: app.bot_prefix.chars().count(),
                on_submit: InlineAction::SetBotPrefix,
            };
        }
        Some('m') => { // Music toggle
            let mut config = config::load_bot_config().unwrap_or_default();
            let current = config.get("musicEnabled")
                .and_then(|v| v.as_bool())
                .unwrap_or(false);
            config["musicEnabled"] = serde_json::Value::Bool(!current);
            let _ = config::save_bot_config(&config);
            app.flash(&format!("Music: {}", if !current { "ON" } else { "OFF" }));
        }
        Some('M') => { // Mode → 인라인 Select
            app.input_mode = InputMode::InlineSelect {
                prompt: "봇 모드 선택".into(),
                options: vec!["local".into(), "cloud".into()],
                selected: 0,
                on_submit: InlineAction::SetBotMode,
            };
        }
        Some('R') => { // Relay URL → 인라인 Input
            let current = config::load_bot_config()
                .ok()
                .and_then(|c| c.get("relayUrl").and_then(|v| v.as_str()).map(|s| s.to_string()))
                .unwrap_or_default();
            app.input_mode = InputMode::InlineInput {
                prompt: "릴레이 서버 URL".into(),
                value: current.clone(),
                cursor: current.chars().count(),
                on_submit: InlineAction::SetBotRelayUrl,
            };
        }
        Some('H') => { // Relay Host ID → 인라인 Input
            let current = config::load_bot_config()
                .ok()
                .and_then(|c| c.get("relayHostId").and_then(|v| v.as_str()).map(|s| s.to_string()))
                .unwrap_or_default();
            app.input_mode = InputMode::InlineInput {
                prompt: "릴레이 호스트 ID".into(),
                value: current.clone(),
                cursor: current.chars().count(),
                on_submit: InlineAction::SetBotRelayHostId,
            };
        }
        Some('N') => { // Node Token → 인라인 Input
            let current = config::load_bot_config()
                .ok()
                .and_then(|c| c.get("nodeToken").and_then(|v| v.as_str()).map(|s| s.to_string()))
                .unwrap_or_default();
            app.input_mode = InputMode::InlineInput {
                prompt: "노드 토큰".into(),
                value: current.clone(),
                cursor: current.chars().count(),
                on_submit: InlineAction::SetBotNodeToken,
            };
        }
        Some('a') => { // Aliases
            app.push_screen(Screen::BotAliases);
        }
        Some('A') => { // Auto-start toggle
            let current = config::get_discord_auto_start().unwrap_or(false);
            let _ = config::set_discord_auto_start(!current);
            app.flash(&format!("Auto-start: {}", if !current { "ON" } else { "OFF" }));
        }
        Some('c') => { // Bot Console → 데몬 API 경유
            let client = app.client.clone();
            let buf2 = app.async_out.clone();
            tokio::spawn(async move {
                match client.get_ext_process_console("discord-bot").await {
                    Ok(data) => {
                        let mut lines = vec![Out::Ok("=== Bot Console ===".into())];
                        if let Some(log_lines) = data.get("lines").and_then(|v| v.as_array()) {
                            for line in log_lines.iter().rev().take(50).collect::<Vec<_>>().into_iter().rev() {
                                lines.push(Out::Text(line.as_str().unwrap_or("").into()));
                            }
                        } else if let Some(output) = data.get("output").and_then(|v| v.as_str()) {
                            for line in output.lines().rev().take(50).collect::<Vec<_>>().into_iter().rev() {
                                lines.push(Out::Text(line.into()));
                            }
                        }
                        push_out(&buf2, lines);
                    }
                    Err(e) => push_out(&buf2, vec![Out::Err(format!("✗ Bot console: {}", e))]),
                }
            });
            app.flash("봇 콘솔 조회 중...");
        }
        _ => {}
    }
}

pub(super) fn handle_bot_aliases_select(app: &mut App, sel: usize) {
    match sel {
        0 => { // Set Module Alias → InlineInput "module alias1,alias2"
            app.input_mode = InputMode::InlineInput {
                prompt: "모듈 별명 설정 (형식: module alias1,alias2)".into(),
                value: String::new(),
                cursor: 0,
                on_submit: InlineAction::Custom("BOT_MODULE_ALIAS".into()),
            };
        }
        1 => { // Set Command Alias → InlineInput "module.command alias1,alias2"
            app.input_mode = InputMode::InlineInput {
                prompt: "명령어 별명 설정 (형식: module.command alias1,alias2)".into(),
                value: String::new(),
                cursor: 0,
                on_submit: InlineAction::Custom("BOT_CMD_ALIAS".into()),
            };
        }
        2 => { // Reset All
            app.input_mode = InputMode::Confirm {
                prompt: "모든 별명을 초기화하시겠습니까? (y/n)".into(),
                action: ConfirmAction::Custom("BOT_ALIAS_RESET".into()),
            };
        }
        _ => {}
    }
}
