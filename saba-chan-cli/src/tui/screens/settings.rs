//! 설정 화면

use crate::tui::app::*;
use crate::config;

pub(super) fn build_settings_menu(app: &App) -> Vec<MenuItem> {
    let t = |k| app.i18n.t(k);
    // 캐시만 참조 — blocking reqwest를 렌더/메뉴 스레드에서 호출하면 데몬 다운 시
    // 항목 5개 × 3s = 매 프레임 최대 15s hang. 캐시는 상태 모니터가 갱신.
    let settings = app.cached_settings.as_ref();
    let ipc_port = settings
        .and_then(|s| s.get("ipcPort").and_then(|v| v.as_u64()))
        .map(|p| p as u16)
        .unwrap_or(saba_chan_updater_lib::constants::DEFAULT_IPC_PORT);
    let console_buf = settings
        .and_then(|s| s.get("consoleBufferSize").and_then(|v| v.as_u64()))
        .unwrap_or(2000);
    let auto_pwd = settings
        .and_then(|s| s.get("autoGeneratePasswords").and_then(|v| v.as_bool()))
        .unwrap_or(true);
    let port_check = settings
        .and_then(|s| s.get("portConflictCheck").and_then(|v| v.as_bool()))
        .unwrap_or(true);

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
        // 공용 설정 (데몬/서버에 영향)
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
                prompt: app.i18n.t("prompt.cli_refresh"),
                value: app.settings.refresh_interval.to_string(),
                cursor: app.settings.refresh_interval.to_string().chars().count(),
                on_submit: InlineAction::SetCliSetting { key: "refresh_interval".into() },
            };
        }
        3 => { // IPC Port → 인라인 Input
            let current = config::get_ipc_port();
            app.input_mode = InputMode::InlineInput {
                prompt: app.i18n.t("prompt.ipc_port"),
                value: current.to_string(),
                cursor: current.to_string().chars().count(),
                on_submit: InlineAction::SetGuiSetting { key: "ipc_port".into() },
            };
        }
        4 => { // Console buffer → 인라인 Input
            let current = config::get_console_buffer_size().unwrap_or(2000);
            app.input_mode = InputMode::InlineInput {
                prompt: app.i18n.t("prompt.console_buffer"),
                value: current.to_string(),
                cursor: current.to_string().chars().count(),
                on_submit: InlineAction::SetGuiSetting { key: "console_buffer".into() },
            };
        }
        5 => { // Auto Passwords toggle
            let current = config::get_auto_generate_passwords().unwrap_or(true);
            let _ = config::set_auto_generate_passwords(!current);
            app.flash(&format!("Auto Passwords: {}", if !current { "ON" } else { "OFF" }));
        }
        6 => { // Port Conflict Check toggle
            let current = config::get_port_conflict_check().unwrap_or(true);
            let _ = config::set_port_conflict_check(!current);
            app.flash(&format!("Port Conflict Check: {}", if !current { "ON" } else { "OFF" }));
        }
        _ => {}
    }
}
