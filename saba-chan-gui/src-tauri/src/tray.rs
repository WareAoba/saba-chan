//! 시스템 트레이 — main.js 메뉴와 같은 항목·동작

use crate::{lifecycle, window, AppState};
use serde_json::json;
use tauri::image::Image;
use tauri::menu::{Menu, MenuItem, PredefinedMenuItem};
use tauri::tray::{MouseButton, MouseButtonState, TrayIconBuilder, TrayIconEvent};
use tauri::{AppHandle, Manager, Theme};

const TRAY_ID: &str = "main";
const ICON_LIGHT: &[u8] = include_bytes!("../../public/favicon-light.png");
const ICON_DARK: &[u8] = include_bytes!("../../public/favicon-dark.png");

/// 테마별 favicon (main.js와 같이 어두운 테마면 favicon-dark)
fn theme_icon(app: &AppHandle) -> Option<Image<'static>> {
    let dark = app
        .get_webview_window(window::MAIN)
        .and_then(|w| w.theme().ok())
        .is_some_and(|t| t == Theme::Dark);
    Image::from_bytes(if dark { ICON_DARK } else { ICON_LIGHT }).ok()
}

pub fn create(app: &AppHandle) -> tauri::Result<()> {
    let state = app.state::<AppState>();
    let mut builder = TrayIconBuilder::with_id(TRAY_ID)
        .tooltip(state.t("tray.tooltip", &[]))
        .menu(&build_menu(app)?)
        .show_menu_on_left_click(false)
        .on_menu_event(|app, event| on_menu(app, event.id().as_ref()))
        .on_tray_icon_event(|tray, event| match event {
            TrayIconEvent::Click {
                button: MouseButton::Left,
                button_state: MouseButtonState::Up,
                ..
            }
            | TrayIconEvent::DoubleClick {
                button: MouseButton::Left,
                ..
            } => window::show_main(tray.app_handle()),
            _ => {}
        });
    if let Some(icon) = theme_icon(app).or_else(|| app.default_window_icon().cloned()) {
        builder = builder.icon(icon);
    }
    builder.build(app)?;
    Ok(())
}

fn build_menu(app: &AppHandle) -> tauri::Result<Menu<tauri::Wry>> {
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
    // CLI로 전환: 데몬이 새 콘솔 창에 CLI 메뉴를 띄우고 이 GUI는 인터페이스만 끝낸다 (지금은 Windows만)
    if cfg!(windows) {
        menu.append(&MenuItem::with_id(
            app,
            "switch_to_cli",
            t("tray.switch_to_cli"),
            true,
            None::<&str>,
        )?)?;
    }
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
        "quit_interface",
        t("tray.quit_interface"),
        true,
        None::<&str>,
    )?)?;
    menu.append(&MenuItem::with_id(
        app,
        "quit_all",
        t("tray.quit_all"),
        true,
        None::<&str>,
    )?)?;
    Ok(menu)
}

fn on_menu(app: &AppHandle, id: &str) {
    let app = app.clone();
    match id {
        "open_window" => window::show_main(&app),
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
            tauri::async_runtime::spawn(async move {
                lifecycle::stop_daemon(&app).await;
                update_menu(&app);
            });
        }
        "start_daemon" => {
            lifecycle::start_daemon(&app);
            tauri::async_runtime::spawn(async move {
                tokio::time::sleep(std::time::Duration::from_secs(1)).await;
                update_menu(&app);
            });
        }
        "quit_interface" => {
            tauri::async_runtime::spawn(async move { lifecycle::interface_only_quit(&app).await });
        }
        "quit_all" => {
            tauri::async_runtime::spawn(async move { lifecycle::clean_quit(&app).await });
        }
        _ => {}
    }
}

/// 데몬 상태·언어가 바뀌면 메뉴를 다시 만든다
pub fn update_menu(app: &AppHandle) {
    let Some(tray) = app.tray_by_id(TRAY_ID) else { return };
    match build_menu(app) {
        Ok(menu) => {
            let _ = tray.set_menu(Some(menu));
        }
        Err(e) => tracing::warn!("[Tray] Failed to build menu: {}", e),
    }
    let _ = tray.set_tooltip(Some(app.state::<AppState>().t("tray.tooltip", &[])));
}

/// 시스템 테마가 바뀌면 아이콘도 바꾼다
pub fn refresh_icon(app: &AppHandle) {
    if let (Some(tray), Some(icon)) = (app.tray_by_id(TRAY_ID), theme_icon(app)) {
        let _ = tray.set_icon(Some(icon));
    }
}

pub fn remove(app: &AppHandle) {
    app.remove_tray_by_id(TRAY_ID);
}
