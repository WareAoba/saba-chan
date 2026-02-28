//! 설정 화면

use crate::tui::app::*;
use crate::gui_config;

pub(super) fn build_settings_menu(app: &App) -> Vec<MenuItem> {
    let auto_refresh = gui_config::get_auto_refresh().unwrap_or(true);
    let refresh_ms = gui_config::get_refresh_interval().unwrap_or(2000);
    let ipc_port = gui_config::get_ipc_port();
    let console_buf = gui_config::get_console_buffer_size().unwrap_or(2000);
    let auto_pwd = gui_config::get_auto_generate_passwords().unwrap_or(true);
    let port_check = gui_config::get_port_conflict_check().unwrap_or(true);
    let discord_auto = gui_config::get_discord_auto_start().unwrap_or(false);

    vec![
        // CLI 설정
        MenuItem::new("Language", Some('l'), &format!(
            "CLI 표시 언어: {}",
            app.settings.effective_language(),
        )),
        MenuItem::new("Auto-start", Some('a'), &format!(
            "시작 시 데몬/봇 자동 기동: {}",
            if app.settings.auto_start { "ON" } else { "OFF" },
        )),
        MenuItem::new("Refresh Interval (CLI)", Some('r'), &format!(
            "CLI 상태 갱신 주기: {}초",
            app.settings.refresh_interval,
        )),
        MenuItem::new("Bot Prefix", Some('p'), &format!(
            "프리픽스: {}",
            app.bot_prefix,
        )),
        // GUI 설정 (settings.json)
        MenuItem::new("GUI Language", Some('g'), &format!(
            "GUI 언어: {}",
            gui_config::get_language().unwrap_or_else(|_| "en".into()),
        )),
        MenuItem::new("Discord Auto-start", Some('d'), &format!(
            "Discord 봇 자동 시작: {}",
            if discord_auto { "ON" } else { "OFF" },
        )),
        MenuItem::new("Auto Refresh (GUI)", Some('A'), &format!(
            "GUI 자동 새로고침: {}",
            if auto_refresh { "ON" } else { "OFF" },
        )),
        MenuItem::new("Refresh Interval (GUI)", Some('R'), &format!(
            "GUI 새로고침 간격: {}ms",
            refresh_ms,
        )),
        MenuItem::new("IPC Port", Some('P'), &format!(
            "데몬 IPC 포트: {}",
            ipc_port,
        )),
        MenuItem::new("Console Buffer", Some('c'), &format!(
            "콘솔 버퍼 크기: {}",
            console_buf,
        )),
        MenuItem::new("Auto Passwords", Some('w'), &format!(
            "비밀번호 자동 생성: {}",
            if auto_pwd { "ON" } else { "OFF" },
        )),
        MenuItem::new("Port Conflict Check", Some('k'), &format!(
            "포트 충돌 확인: {}",
            if port_check { "ON" } else { "OFF" },
        )),
    ]
}

pub(super) fn handle_settings_select(app: &mut App, sel: usize) {
    match sel {
        0 => { // Language (CLI) → 인라인 Select
            let available = vec![
                "auto".into(), "en".into(), "ko".into(), "ja".into(),
                "zh-CN".into(), "zh-TW".into(), "es".into(), "pt-BR".into(),
                "ru".into(), "de".into(), "fr".into(),
            ];
            app.input_mode = InputMode::InlineSelect {
                prompt: "CLI 표시 언어 선택".into(),
                options: available,
                selected: 0,
                on_submit: InlineAction::SetCliSetting { key: "language".into() },
            };
        }
        1 => { // Auto-start toggle
            app.settings.auto_start = !app.settings.auto_start;
            let _ = app.settings.save();
            app.flash(&format!("Auto-start: {}", if app.settings.auto_start { "ON" } else { "OFF" }));
        }
        2 => { // Refresh interval (CLI) → 인라인 Input
            app.input_mode = InputMode::InlineInput {
                prompt: "CLI 상태 갱신 주기 (초)".into(),
                value: app.settings.refresh_interval.to_string(),
                cursor: app.settings.refresh_interval.to_string().chars().count(),
                on_submit: InlineAction::SetCliSetting { key: "refresh_interval".into() },
            };
        }
        3 => { // Bot prefix → 인라인 Input
            app.input_mode = InputMode::InlineInput {
                prompt: "봇 명령어 프리픽스".into(),
                value: app.bot_prefix.clone(),
                cursor: app.bot_prefix.chars().count(),
                on_submit: InlineAction::SetBotPrefix,
            };
        }
        4 => { // GUI language → 인라인 Select
            let available = vec![
                "en".into(), "ko".into(), "ja".into(),
                "zh-CN".into(), "zh-TW".into(), "es".into(), "pt-BR".into(),
                "ru".into(), "de".into(), "fr".into(),
            ];
            app.input_mode = InputMode::InlineSelect {
                prompt: "GUI 표시 언어 선택".into(),
                options: available,
                selected: 0,
                on_submit: InlineAction::SetGuiSetting { key: "language".into() },
            };
        }
        5 => { // Discord auto-start toggle
            let current = gui_config::get_discord_auto_start().unwrap_or(false);
            let _ = gui_config::set_discord_auto_start(!current);
            app.flash(&format!("Discord Auto-start: {}", if !current { "ON" } else { "OFF" }));
        }
        6 => { // Auto Refresh (GUI) toggle
            let current = gui_config::get_auto_refresh().unwrap_or(true);
            let _ = gui_config::set_auto_refresh(!current);
            app.flash(&format!("GUI Auto-refresh: {}", if !current { "ON" } else { "OFF" }));
        }
        7 => { // Refresh interval (GUI) → 인라인 Input
            let current = gui_config::get_refresh_interval().unwrap_or(2000);
            app.input_mode = InputMode::InlineInput {
                prompt: "GUI 새로고침 간격 (ms, 500-60000)".into(),
                value: current.to_string(),
                cursor: current.to_string().chars().count(),
                on_submit: InlineAction::SetGuiSetting { key: "refresh_interval".into() },
            };
        }
        8 => { // IPC Port → 인라인 Input
            let current = gui_config::get_ipc_port();
            app.input_mode = InputMode::InlineInput {
                prompt: "데몬 IPC 포트 (1024+)".into(),
                value: current.to_string(),
                cursor: current.to_string().chars().count(),
                on_submit: InlineAction::SetGuiSetting { key: "ipc_port".into() },
            };
        }
        9 => { // Console buffer → 인라인 Input
            let current = gui_config::get_console_buffer_size().unwrap_or(2000);
            app.input_mode = InputMode::InlineInput {
                prompt: "콘솔 버퍼 크기 (100-50000)".into(),
                value: current.to_string(),
                cursor: current.to_string().chars().count(),
                on_submit: InlineAction::SetGuiSetting { key: "console_buffer".into() },
            };
        }
        10 => { // Auto Passwords toggle
            let current = gui_config::get_auto_generate_passwords().unwrap_or(true);
            let _ = gui_config::set_auto_generate_passwords(!current);
            app.flash(&format!("Auto Passwords: {}", if !current { "ON" } else { "OFF" }));
        }
        11 => { // Port Conflict Check toggle
            let current = gui_config::get_port_conflict_check().unwrap_or(true);
            let _ = gui_config::set_port_conflict_check(!current);
            app.flash(&format!("Port Conflict Check: {}", if !current { "ON" } else { "OFF" }));
        }
        _ => {}
    }
}
