//! 설정 화면

use crate::tui::app::*;
use crate::gui_config;

pub(super) fn build_settings_menu(app: &App) -> Vec<MenuItem> {
    vec![
        MenuItem::new("Language", Some('l'), &format!(
            "표시 언어: {}",
            app.settings.effective_language(),
        )),
        MenuItem::new("Auto-start", Some('a'), &format!(
            "시작 시 데몬/봇 자동 기동: {}",
            if app.settings.auto_start { "ON" } else { "OFF" },
        )),
        MenuItem::new("Refresh Interval", Some('r'), &format!(
            "상태 갱신 주기: {}초",
            app.settings.refresh_interval,
        )),
        MenuItem::new("Bot Prefix", Some('p'), &format!(
            "프리픽스: {}",
            app.bot_prefix,
        )),
        MenuItem::new("Modules Path", Some('m'), &format!(
            "모듈 경로: {}",
            gui_config::get_modules_path().unwrap_or_default(),
        )),
        MenuItem::new("GUI Language", Some('g'), &format!(
            "GUI 언어: {}",
            gui_config::get_language().unwrap_or_else(|_| "en".into()),
        )),
    ]
}

pub(super) fn handle_settings_select(app: &mut App, sel: usize) {
    // 설정은 대부분 커맨드 모드에서 편집하도록 유도
    match sel {
        0 => { // Language
            app.push_screen(Screen::CommandMode);
            app.input_mode = InputMode::Command;
            app.input = "config set language ".to_string();
            app.cursor = app.input.chars().count();
        }
        1 => { // Auto-start toggle
            app.settings.auto_start = !app.settings.auto_start;
            let _ = app.settings.save();
            app.flash(&format!("Auto-start: {}", if app.settings.auto_start { "ON" } else { "OFF" }));
        }
        2 => { // Refresh interval
            app.push_screen(Screen::CommandMode);
            app.input_mode = InputMode::Command;
            app.input = "config set refresh_interval ".to_string();
            app.cursor = app.input.chars().count();
        }
        3 => { // Bot prefix
            app.push_screen(Screen::CommandMode);
            app.input_mode = InputMode::Command;
            app.input = "bot prefix set ".to_string();
            app.cursor = app.input.chars().count();
        }
        4 => { // Modules path
            app.push_screen(Screen::CommandMode);
            app.input_mode = InputMode::Command;
            app.input = "config gui modules_path ".to_string();
            app.cursor = app.input.chars().count();
        }
        5 => { // GUI language
            app.push_screen(Screen::CommandMode);
            app.input_mode = InputMode::Command;
            app.input = "config gui language ".to_string();
            app.cursor = app.input.chars().count();
        }
        _ => {}
    }
}
