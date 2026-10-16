//! 디스코드 봇 화면 — 릴레이 에이전트 시작/정지, 프리픽스, 릴레이 연결, 별명

use ratatui::prelude::*;
use ratatui::widgets::{Block, BorderType, Borders, Paragraph};

use crate::tui::app::*;
use crate::tui::theme::Theme;
use crate::config;
use crate::process;

pub(super) fn build_bot_menu(app: &App) -> Vec<MenuItem> {
    let t = |k| app.i18n.t(k);
    vec![
        if app.bot_on {
            MenuItem::new(&format!("■ {}", t("screen.bot_stop")), Some('s'), &t("screen.bot_stop"))
        } else {
            MenuItem::new(&format!("▶ {}", t("screen.bot_start")), Some('s'), &t("screen.bot_start"))
        },
        MenuItem::new(&format!("📝 {}", t("screen.bot_prefix")), Some('p'), &t("screen.bot_prefix")),
        MenuItem::new(&format!("🌐 {}", t("screen.bot_relay_url")), Some('R'), &t("screen.bot_relay_url")),
        MenuItem::new(&format!("🏠 {}", t("screen.bot_relay_host_id")), Some('H'), &t("screen.bot_relay_host_id")),
        MenuItem::new(&format!("🔐 {}", t("screen.bot_node_token")), Some('N'), &t("screen.bot_node_token")),
        MenuItem::new(&format!("🏷 {}", t("screen.bot_aliases")), Some('a'), &t("screen.bot_aliases")),
    ]
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
        Some('p') => { // Prefix → 인라인 Input
            app.input_mode = InputMode::InlineInput {
                prompt: "봇 명령어 프리픽스".into(),
                value: app.bot_prefix.clone(),
                cursor: app.bot_prefix.chars().count(),
                on_submit: InlineAction::SetBotPrefix,
            };
        }
        Some('R') => { // Relay URL → 인라인 Input
            let current = config::load_bot_config()
                .ok()
                .and_then(|c| c.pointer("/cloud/relayUrl").and_then(|v| v.as_str()).map(|s| s.to_string()))
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
                .and_then(|c| c.pointer("/cloud/hostId").and_then(|v| v.as_str()).map(|s| s.to_string()))
                .unwrap_or_default();
            app.input_mode = InputMode::InlineInput {
                prompt: "릴레이 호스트 ID".into(),
                value: current.clone(),
                cursor: current.chars().count(),
                on_submit: InlineAction::SetBotRelayHostId,
            };
        }
        Some('N') => { // Node Token → 인라인 Input (비밀값이라 미리 채우지 않음, 빈 값 제출 = 삭제)
            app.input_mode = InputMode::InlineInput {
                prompt: "노드 토큰 (sbn_…)".into(),
                value: String::new(),
                cursor: 0,
                on_submit: InlineAction::SetBotNodeToken,
            };
        }
        Some('a') => { // Aliases
            app.push_screen(Screen::BotAliases);
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
