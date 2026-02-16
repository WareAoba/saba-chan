//! 디스코드 봇 화면 — 시작/정지, 토큰, 프리픽스, 별명

use ratatui::prelude::*;
use ratatui::widgets::{Block, BorderType, Borders, Paragraph};

use crate::tui::app::*;
use crate::tui::theme::Theme;
use crate::gui_config;
use crate::process;

pub(super) fn build_bot_menu(app: &App) -> Vec<MenuItem> {
    let is_running = app.bot_on;
    vec![
        if is_running {
            MenuItem::new("■ Stop Bot", Some('s'), "디스코드 봇 정지")
        } else {
            MenuItem::new("▶ Start Bot", Some('s'), "디스코드 봇 시작")
        },
        MenuItem::new("🔑 Token", Some('t'), "디스코드 토큰 관리"),
        MenuItem::new("📝 Prefix", Some('p'), "봇 명령어 프리픽스 설정"),
        MenuItem::new("🏷 Aliases", Some('a'), "모듈/커맨드 별명 관리"),
        MenuItem::new("⚙ Auto-start", Some('A'), &format!(
            "자동 시작: {}",
            if gui_config::get_discord_auto_start().unwrap_or(false) { "ON" } else { "OFF" },
        )),
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
    let config = gui_config::load_bot_config().unwrap_or_default();
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
        "  Use ':' command mode to edit: bot alias set module <name> <aliases>",
        Theme::dimmed(),
    )));

    frame.render_widget(
        Paragraph::new(lines),
        Rect::new(inner.x, inner.y + 1, inner.width, inner.height.saturating_sub(1)),
    );
}

pub(super) fn handle_bot_select(app: &mut App, sel: usize) {
    let _client = app.client.clone();
    let buf = app.async_out.clone();

    match sel {
        0 => { // Start/Stop
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
        1 => { // Token → 커맨드 모드
            app.push_screen(Screen::CommandMode);
            app.input_mode = InputMode::Command;
            app.input = "bot token ".to_string();
            app.cursor = app.input.chars().count();
        }
        2 => { // Prefix → 커맨드 모드
            app.push_screen(Screen::CommandMode);
            app.input_mode = InputMode::Command;
            app.input = "bot prefix ".to_string();
            app.cursor = app.input.chars().count();
        }
        3 => { // Aliases
            app.push_screen(Screen::BotAliases);
        }
        4 => { // Auto-start toggle
            let current = gui_config::get_discord_auto_start().unwrap_or(false);
            let _ = gui_config::set_discord_auto_start(!current);
            app.flash(&format!("Auto-start: {}", if !current { "ON" } else { "OFF" }));
        }
        _ => {}
    }
}
