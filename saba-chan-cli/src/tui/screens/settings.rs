//! 설정 화면

use crate::tui::app::*;
use crate::config;

pub(super) fn build_settings_menu(app: &App) -> Vec<MenuItem> {
    let t = |k| app.i18n.t(k);
    let ipc_port = config::get_ipc_port();
    let console_buf = config::get_console_buffer_size().unwrap_or(2000);
    let auto_pwd = config::get_auto_generate_passwords().unwrap_or(true);
    let port_check = config::get_port_conflict_check().unwrap_or(true);
    let discord_auto = config::get_discord_auto_start().unwrap_or(false);

    vec![
        // CLI 설정
        MenuItem::new(&t("screen.settings_language"), Some('l'), &format!(
            "{}: {}",
            t("screen.settings_language"),
            app.settings.effective_language(),
        )),
        MenuItem::new(&t("screen.settings_autostart"), Some('a'), &format!(
            "{}: {}",
            t("screen.settings_autostart"),
            if app.settings.auto_start { "ON" } else { "OFF" },
        )),
        MenuItem::new(&format!("{} (CLI)", t("screen.settings_refresh")), Some('r'), &format!(
            "{}: {}s",
            t("screen.settings_refresh"),
            app.settings.refresh_interval,
        )),
        MenuItem::new(&t("screen.settings_prefix"), Some('p'), &format!(
            "{}: {}",
            t("screen.settings_prefix"),
            app.bot_prefix,
        )),
        // 공용 설정 (데몬/서버에 영향)
        MenuItem::new(&t("screen.settings_discord_auto"), Some('d'), &format!(
            "{}: {}",
            t("screen.settings_discord_auto"),
            if discord_auto { "ON" } else { "OFF" },
        )),
        MenuItem::new(&t("screen.settings_ipc_port"), Some('P'), &format!(
            "{}: {}",
            t("screen.settings_ipc_port"),
            ipc_port,
        )),
        MenuItem::new(&t("screen.settings_console_buffer"), Some('c'), &format!(
            "{}: {}",
            t("screen.settings_console_buffer"),
            console_buf,
        )),
        MenuItem::new(&t("screen.settings_auto_passwords"), Some('w'), &format!(
            "{}: {}",
            t("screen.settings_auto_passwords"),
            if auto_pwd { "ON" } else { "OFF" },
        )),
        MenuItem::new(&t("screen.settings_port_check"), Some('k'), &format!(
            "{}: {}",
            t("screen.settings_port_check"),
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
        4 => { // Discord auto-start toggle
            let current = config::get_discord_auto_start().unwrap_or(false);
            let _ = config::set_discord_auto_start(!current);
            app.flash(&format!("Discord Auto-start: {}", if !current { "ON" } else { "OFF" }));
        }
        5 => { // IPC Port → 인라인 Input
            let current = config::get_ipc_port();
            app.input_mode = InputMode::InlineInput {
                prompt: "데몬 IPC 포트 (1024+)".into(),
                value: current.to_string(),
                cursor: current.to_string().chars().count(),
                on_submit: InlineAction::SetGuiSetting { key: "ipc_port".into() },
            };
        }
        6 => { // Console buffer → 인라인 Input
            let current = config::get_console_buffer_size().unwrap_or(2000);
            app.input_mode = InputMode::InlineInput {
                prompt: "콘솔 버퍼 크기 (100-50000)".into(),
                value: current.to_string(),
                cursor: current.to_string().chars().count(),
                on_submit: InlineAction::SetGuiSetting { key: "console_buffer".into() },
            };
        }
        7 => { // Auto Passwords toggle
            let current = config::get_auto_generate_passwords().unwrap_or(true);
            let _ = config::set_auto_generate_passwords(!current);
            app.flash(&format!("Auto Passwords: {}", if !current { "ON" } else { "OFF" }));
        }
        8 => { // Port Conflict Check toggle
            let current = config::get_port_conflict_check().unwrap_or(true);
            let _ = config::set_port_conflict_check(!current);
            app.flash(&format!("Port Conflict Check: {}", if !current { "ON" } else { "OFF" }));
        }
        _ => {}
    }
}
