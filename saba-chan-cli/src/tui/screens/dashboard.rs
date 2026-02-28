//! 대시보드 화면 — 메인 메뉴

use ratatui::prelude::*;
use ratatui::widgets::{Block, BorderType, Borders};

use crate::tui::app::*;
use crate::tui::theme::Theme;
use crate::tui::render;

pub(super) fn build_dashboard_menu(_app: &App) -> Vec<MenuItem> {
    vec![
        MenuItem::new("Instances", Some('1'), "인스턴스 관리"),
        MenuItem::new("Modules", Some('2'), "게임 모듈 관리"),
        MenuItem::new("Extensions", Some('3'), "익스텐션 관리"),
        MenuItem::new("Discord Bot", Some('4'), "디스코드 봇 설정"),
        MenuItem::new("Settings", Some('5'), "CLI · GUI 설정"),
        MenuItem::new("Updates", Some('6'), "업데이트 관리"),
        MenuItem::new("Saba-Core", Some('7'), "코어 데몬 프로세스 관리"),
        MenuItem::new("Command Mode", Some(':'), "레거시 명령어 입력"),
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
            // 서버 목록 + 인스턴스 목록을 미리 캐시
            tokio::spawn(async move {
                // 서버 목록과 인스턴스 목록은 화면 전환 후 자동 갱신
                let _ = client.list_instances().await;
                let _ = buf; // keep buf alive
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
            // Command mode
            app.push_screen(Screen::CommandMode);
            app.input_mode = InputMode::Command;
        }
        _ => {}
    }
}
