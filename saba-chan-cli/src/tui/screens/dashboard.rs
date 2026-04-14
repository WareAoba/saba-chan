//! 대시보드 화면 — 메인 메뉴

use ratatui::prelude::*;
use ratatui::widgets::{Block, BorderType, Borders};

use crate::tui::app::*;
use crate::tui::theme::Theme;
use crate::tui::render;

pub(super) fn build_dashboard_menu(app: &App) -> Vec<MenuItem> {
    let t = |k| app.i18n.t(k);
    vec![
        MenuItem::new(&t("menu.servers"), Some('1'), &t("screen.server_console")),
        MenuItem::new(&t("menu.modules"), Some('2'), &t("screen.module_info")),
        MenuItem::new("Extensions", Some('3'), &t("screen.ext_installed")),
        MenuItem::new(&t("menu.bot"), Some('4'), &t("screen.bot_token")),
        MenuItem::new(&t("menu.settings"), Some('5'), &t("screen.settings_language")),
        MenuItem::new(&t("menu.updates"), Some('6'), &t("screen.update_check")),
        MenuItem::new(&t("menu.daemon"), Some('7'), &t("screen.daemon_status")),
        MenuItem::new(&t("menu.command_mode"), Some(':'), &t("screen.server_execute")),
    ]
}

pub(super) fn render_dashboard(app: &App, frame: &mut Frame, area: Rect) {
    let block = Block::default()
        .title(" Main Menu ")
        .title_alignment(Alignment::Left)
        .borders(Borders::ALL)
        .border_type(BorderType::Rounded)
        .border_style(Theme::border());

    let inner = block.inner(area);
    frame.render_widget(block, area);
    render::render_menu(&app.menu_items, app.menu_selected, frame, Rect::new(
        inner.x + 1, inner.y + 1,
        inner.width.saturating_sub(2), inner.height.saturating_sub(2),
    ));
}

pub(super) fn handle_dashboard_select(app: &mut App, sel: usize) {
    match sel {
        0 => { // Servers
            let buf = app.async_out.clone();
            let client = app.client.clone();
            tokio::spawn(async move {
                let _ = client.list_instances().await;
                let _ = buf;
            });
            app.push_screen(Screen::Servers);
        }
        1 => app.push_screen(Screen::Modules),
        2 => app.push_screen(Screen::Extensions),
        3 => app.push_screen(Screen::Bot),
        4 => app.push_screen(Screen::Settings),
        5 => app.push_screen(Screen::Updates),
        6 => app.push_screen(Screen::Daemon),
        7 => {
            // Console mode
            app.push_screen(Screen::CommandMode);
            app.input_mode = InputMode::Command;
        }
        _ => {}
    }
}
