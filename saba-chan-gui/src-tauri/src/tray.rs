//! 시스템 트레이 — 아이콘을 누르면 GUI와 같은 디자인의 팝업(`window::toggle_tray_popup`)을 연다.
//! 리눅스(앱인디케이터)는 아이콘 클릭을 알려 주지 않으므로 OS 메뉴를 단다.
//! 메뉴와 팝업의 동작은 `action` 하나로 처리한다 (`tray_action` 명령)

use crate::{lifecycle, window, AppState};
use serde_json::json;
use tauri::image::Image;
use tauri::tray::{MouseButton, MouseButtonState, TrayIconBuilder, TrayIconEvent};
use tauri::{AppHandle, Manager, Theme};

const TRAY_ID: &str = "main";
const ICON_LIGHT: &[u8] = include_bytes!("../../public/favicon-light.png");
const ICON_DARK: &[u8] = include_bytes!("../../public/favicon-dark.png");

/// 테마별 favicon (어두운 테마면 favicon-dark). Windows는 작업 표시줄 테마를, 그 밖에는 메인 창 테마를 따른다
fn theme_icon(app: &AppHandle) -> Option<Image<'static>> {
    Image::from_bytes(if is_dark(app) { ICON_DARK } else { ICON_LIGHT }).ok()
}

fn is_dark(app: &AppHandle) -> bool {
    system_dark().unwrap_or_else(|| {
        app.get_webview_window(window::MAIN)
            .and_then(|w| w.theme().ok())
            .is_some_and(|t| t == Theme::Dark)
    })
}

/// Windows 작업 표시줄이 어두운가 (`SystemUsesLightTheme`). 창이 없을 때도 읽을 수 있다
fn system_dark() -> Option<bool> {
    #[cfg(windows)]
    {
        let key = winreg::RegKey::predef(winreg::enums::HKEY_CURRENT_USER)
            .open_subkey(r"Software\Microsoft\Windows\CurrentVersion\Themes\Personalize")
            .ok()?;
        let light: u32 = key.get_value("SystemUsesLightTheme").ok()?;
        Some(light == 0)
    }
    #[cfg(not(windows))]
    {
        None
    }
}

/// 트레이 아이콘 자리 (Windows·macOS는 물리 px로 준다)
fn icon_rect(rect: &tauri::Rect) -> window::PxRect {
    let pos = rect.position.to_physical::<f64>(1.0);
    let size = rect.size.to_physical::<f64>(1.0);
    window::PxRect {
        x: pos.x,
        y: pos.y,
        w: size.width,
        h: size.height,
    }
}

pub fn create(app: &AppHandle) -> tauri::Result<()> {
    let state = app.state::<AppState>();
    let mut builder = TrayIconBuilder::with_id(TRAY_ID)
        .tooltip(state.t("tray.tooltip", &[]))
        .show_menu_on_left_click(false)
        .on_tray_icon_event(|tray, event| match event {
            TrayIconEvent::Click {
                button: MouseButton::Left | MouseButton::Right,
                button_state: MouseButtonState::Up,
                rect,
                ..
            } => window::toggle_tray_popup(tray.app_handle(), Some(icon_rect(&rect))),
            TrayIconEvent::DoubleClick {
                button: MouseButton::Left,
                ..
            } => window::show_main(tray.app_handle()),
            _ => {}
        });
    #[cfg(target_os = "linux")]
    {
        builder = builder.menu(&linux_menu::build(app)?).on_menu_event(|app, event| {
            action(app, event.id().as_ref());
        });
    }
    if let Some(icon) = theme_icon(app).or_else(|| app.default_window_icon().cloned()) {
        builder = builder.icon(icon);
    }
    builder.build(app)?;
    Ok(())
}

/// 트레이 동작 (팝업 버튼·리눅스 메뉴). 모르는 것은 false
pub fn action(app: &AppHandle, id: &str) -> bool {
    let app = app.clone();
    match id {
        "open_window" => window::show_main(&app),
        // CLI로 전환: 데몬이 새 콘솔 창에 CLI 메뉴를 띄우고 이 GUI는 창을 닫는다 (지금은 Windows만)
        "switch_to_cli" => {
            tauri::async_runtime::spawn(async move {
                let state = app.state::<AppState>();
                let body = json!({ "kind": "cli" });
                let res = state.client.post("/api/clients/launch", &body, 30000).await;
                let ok = res.as_ref().is_ok_and(|v| v["success"].as_bool().unwrap_or(false));
                if ok {
                    lifecycle::interface_only_quit(&app).await;
                } else {
                    tracing::warn!(
                        "[Tray] Could not switch to the CLI: {:?}",
                        res.map_err(|e| e.error_body(json!({})))
                    );
                }
            });
        }
        "stop_daemon" => {
            lifecycle::note_tray_stop(&app, true);
            tauri::async_runtime::spawn(async move {
                // 끄지 못했으면 트레이에서 끈 것이 아니다
                let accepted = lifecycle::stop_daemon(&app).await;
                if !lifecycle::tray_stop_stands(accepted) {
                    lifecycle::note_tray_stop(&app, false);
                }
                update_menu(&app);
            });
        }
        "start_daemon" => {
            lifecycle::note_tray_stop(&app, false);
            lifecycle::start_daemon(&app);
            tauri::async_runtime::spawn(async move {
                tokio::time::sleep(std::time::Duration::from_secs(1)).await;
                update_menu(&app);
            });
        }
        "quit_all" => {
            tauri::async_runtime::spawn(async move { lifecycle::clean_quit(&app).await });
        }
        _ => return false,
    }
    true
}

/// 데몬 상태·언어가 바뀌면 메뉴(리눅스)와 툴팁을 다시 만든다
pub fn update_menu(app: &AppHandle) {
    let Some(tray) = app.tray_by_id(TRAY_ID) else { return };
    #[cfg(target_os = "linux")]
    match linux_menu::build(app) {
        Ok(menu) => {
            let _ = tray.set_menu(Some(menu));
        }
        Err(e) => tracing::warn!("[Tray] Failed to build menu: {}", e),
    }
    let _ = tray.set_tooltip(Some(app.state::<AppState>().t("tray.tooltip", &[])));
}

#[cfg(target_os = "linux")]
mod linux_menu {
    use crate::{lifecycle, AppState};
    use tauri::menu::{Menu, MenuItem, PredefinedMenuItem};
    use tauri::{AppHandle, Manager};

    pub fn build(app: &AppHandle) -> tauri::Result<Menu<tauri::Wry>> {
        let state = app.state::<AppState>();
        let t = |k: &str| state.t(k, &[]);
        let running = lifecycle::daemon_running(app);
        let menu = Menu::new(app)?;
        menu.append(&MenuItem::with_id(
            app,
            "open_window",
            t("tray.open_window"),
            true,
            None::<&str>,
        )?)?;
        menu.append(&PredefinedMenuItem::separator(app)?)?;
        let status = if running {
            t("tray.daemon_running")
        } else {
            t("tray.daemon_stopped")
        };
        menu.append(&MenuItem::with_id(app, "daemon_status", status, false, None::<&str>)?)?;
        menu.append(&MenuItem::with_id(
            app,
            "stop_daemon",
            t("tray.stop_daemon"),
            running,
            None::<&str>,
        )?)?;
        menu.append(&MenuItem::with_id(
            app,
            "start_daemon",
            t("tray.start_daemon"),
            !running,
            None::<&str>,
        )?)?;
        menu.append(&PredefinedMenuItem::separator(app)?)?;
        menu.append(&MenuItem::with_id(
            app,
            "quit_all",
            t("tray.quit_all"),
            true,
            None::<&str>,
        )?)?;
        Ok(menu)
    }
}

/// 시스템 테마가 바뀌면 아이콘도 바꾼다
/// 창이 없으면 테마 이벤트가 오지 않아 heartbeat마다도 부른다: 바뀌었을 때만 아이콘을 다시 넣는다
pub fn refresh_icon(app: &AppHandle) {
    static SHOWN: std::sync::atomic::AtomicU8 = std::sync::atomic::AtomicU8::new(0);
    let shown = if is_dark(app) { 2 } else { 1 };
    if SHOWN.swap(shown, std::sync::atomic::Ordering::SeqCst) == shown {
        return;
    }
    if let (Some(tray), Some(icon)) = (app.tray_by_id(TRAY_ID), theme_icon(app)) {
        let _ = tray.set_icon(Some(icon));
    }
}

pub fn remove(app: &AppHandle) {
    app.remove_tray_by_id(TRAY_ID);
}
