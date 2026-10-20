//! 창 — 메인 창(크기 기억·닫기 가로채기), 콘솔 팝아웃(PiP), 탐색 잠금

use crate::{emit_main, settings, AppState};
use serde_json::{json, Value};
use std::path::PathBuf;
use std::sync::atomic::{AtomicU64, Ordering};
use tauri::webview::NewWindowResponse;
use tauri::window::Color;
use tauri::{AppHandle, Manager, Url, WebviewUrl, WebviewWindow, WebviewWindowBuilder, WindowEvent};
use tauri_plugin_opener::OpenerExt;

pub const MAIN: &str = "main";
const POPOUT_PREFIX: &str = "popout-";
const MIN_WIDTH: f64 = 780.0;
const BOUNDS_DEBOUNCE_MS: u64 = 500;

/// 창 크기 저장 디바운스용 세대 번호
static BOUNDS_GEN: AtomicU64 = AtomicU64::new(0);

/// 팝아웃 창 라벨: instanceId를 hex로 (라벨에 쓸 수 있는 글자만 남게)
pub fn popout_label(instance_id: &str) -> String {
    let hex: String = instance_id.bytes().map(|b| format!("{b:02x}")).collect();
    format!("{POPOUT_PREFIX}{hex}")
}

pub fn popout_instance_id(label: &str) -> Option<String> {
    let hex = label.strip_prefix(POPOUT_PREFIX)?;
    if hex.len() % 2 != 0 {
        return None;
    }
    let bytes: Option<Vec<u8>> = (0..hex.len())
        .step_by(2)
        .map(|i| u8::from_str_radix(hex.get(i..i + 2)?, 16).ok())
        .collect();
    String::from_utf8(bytes?).ok()
}

/// encodeURIComponent
pub fn encode_component(s: &str) -> String {
    s.bytes()
        .map(|b| match b {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'_' | b'.' | b'!' | b'~' | b'*' | b'\'' | b'(' | b')' => {
                (b as char).to_string()
            }
            _ => format!("%{b:02X}"),
        })
        .collect()
}

/// 바깥 브라우저로 열어도 되는 주소: http(s)만 (file:, 사용자 정의 프로토콜은 실행이 될 수 있다)
pub fn is_web_url(url: &str) -> bool {
    Url::parse(url).is_ok_and(|u| matches!(u.scheme(), "http" | "https"))
}

/// 앱 화면 주소인가: 내장 프런트(tauri://localhost, Windows는 http(s)://tauri.localhost) 또는 개발 서버
pub fn is_app_url(url: &Url, dev_origin: Option<&Url>) -> bool {
    if url.scheme() == "tauri" || url.host_str() == Some("tauri.localhost") || url.as_str() == "about:blank" {
        return true;
    }
    dev_origin.is_some_and(|d| url.origin() == d.origin())
}

fn open_external(app: &AppHandle, url: &str) {
    if let Err(e) = app.opener().open_url(url, None::<&str>) {
        tracing::warn!("[Window] Failed to open external url: {}", e);
    }
}

/// 제목 표시줄·작업 표시줄 아이콘을 exe 아이콘에서 창 DPI에 맞는 크기로 건다.
/// 배율이 다른 모니터로 옮기거나 배율을 바꾸면 그 DPI에 맞게 다시 건다
fn apply_icon(_win: &WebviewWindow) {
    #[cfg(windows)]
    {
        fn set(win: &WebviewWindow) {
            if let Ok(hwnd) = win.hwnd() {
                saba_chan_updater_lib::window_icon::apply_exe_icon(hwnd.0 as isize);
            }
        }
        set(_win);
        let win = _win.clone();
        _win.on_window_event(move |event| {
            if matches!(event, WindowEvent::ScaleFactorChanged { .. }) {
                set(&win);
            }
        });
    }
}

/// 창이 앱 화면 밖으로 이동하거나 새 창을 열지 못하게 한다. 웹 링크는 바깥 브라우저로
fn lock_navigation<'a>(
    app: &AppHandle,
    builder: WebviewWindowBuilder<'a, tauri::Wry, AppHandle>,
) -> WebviewWindowBuilder<'a, tauri::Wry, AppHandle> {
    let dev_origin = if tauri::is_dev() {
        app.config().build.dev_url.clone()
    } else {
        None
    };
    let (nav_app, new_app) = (app.clone(), app.clone());
    builder
        .on_navigation(move |url| {
            if is_app_url(url, dev_origin.as_ref()) {
                return true;
            }
            if is_web_url(url.as_str()) {
                open_external(&nav_app, url.as_str());
            }
            false
        })
        .on_new_window(move |url, _features| {
            if is_web_url(url.as_str()) {
                open_external(&new_app, url.as_str());
            }
            NewWindowResponse::Deny
        })
}

// ── 메인 창 ──

pub fn create_main_window(app: &AppHandle) -> tauri::Result<WebviewWindow> {
    let s = app.state::<AppState>().settings();
    let bounds = &s["windowBounds"];
    let width = bounds["width"].as_f64().unwrap_or(1200.0);
    let height = bounds["height"].as_f64().unwrap_or(840.0);

    // 작은 화면에서도 창이 다 들어가게: 작업 영역보다 크게 열지 않는다
    let (work_w, work_h) = app
        .primary_monitor()
        .ok()
        .flatten()
        .map(|m| {
            let area = m.work_area();
            let scale = m.scale_factor();
            (area.size.width as f64 / scale, area.size.height as f64 / scale)
        })
        .unwrap_or((width, height.max(840.0)));
    let min_height = 840f64.min(560f64.max(work_h));

    let mut builder = WebviewWindowBuilder::new(app, MAIN, WebviewUrl::App("index.html".into()))
        .title("Saba-chan")
        .inner_size(width.min(work_w), height.max(min_height).min(work_h))
        .min_inner_size(MIN_WIDTH, min_height)
        .decorations(false)
        .visible(false) // 페이지가 뜰 때까지 보이지 않음
        .background_color(Color(0xf0, 0xf0, 0xf2, 0xff)) // CSS 로드 전 흰 화면 방지
        .on_page_load(|w, payload| {
            if matches!(payload.event(), tauri::webview::PageLoadEvent::Finished) {
                let _ = w.show();
            }
        });
    if let (Some(x), Some(y)) = (bounds["x"].as_f64(), bounds["y"].as_f64()) {
        builder = builder.position(x, y);
    }
    let win = lock_navigation(app, builder).build()?;
    apply_icon(&win);

    let handle = app.clone();
    win.on_window_event(move |event| match event {
        // 닫기는 React 확인 창으로 (app_close_response가 답한다)
        WindowEvent::CloseRequested { api, .. } => {
            api.prevent_close();
            emit_main(&handle, "app:closeRequest", Value::Null);
        }
        WindowEvent::Moved(_) | WindowEvent::Resized(_) => schedule_save_bounds(&handle),
        WindowEvent::ThemeChanged(_) => crate::tray::refresh_icon(&handle),
        _ => {}
    });
    Ok(win)
}

pub fn show_main(app: &AppHandle) {
    if let Some(w) = app.get_webview_window(MAIN) {
        let _ = w.unminimize();
        let _ = w.show();
        let _ = w.set_focus();
    }
}

/// 이동·크기 변경이 멈추면 저장 (최대화·최소화 중에는 저장하지 않는다)
fn schedule_save_bounds(app: &AppHandle) {
    let gen = BOUNDS_GEN.fetch_add(1, Ordering::SeqCst) + 1;
    let app = app.clone();
    tauri::async_runtime::spawn(async move {
        tokio::time::sleep(std::time::Duration::from_millis(BOUNDS_DEBOUNCE_MS)).await;
        if BOUNDS_GEN.load(Ordering::SeqCst) != gen {
            return;
        }
        let Some(win) = app.get_webview_window(MAIN) else {
            return;
        };
        if win.is_maximized().unwrap_or(false)
            || win.is_minimized().unwrap_or(false)
            || !win.is_visible().unwrap_or(false)
        {
            return;
        }
        let (Ok(pos), Ok(size), Ok(scale)) = (win.outer_position(), win.inner_size(), win.scale_factor()) else {
            return;
        };
        let (pos, size) = (pos.to_logical::<f64>(scale), size.to_logical::<f64>(scale));
        let state = app.state::<AppState>();
        let mut s = settings::load_settings(&state.data_dir);
        s["windowBounds"] = json!({
            "width": size.width.round() as i64, "height": size.height.round() as i64,
            "x": pos.x.round() as i64, "y": pos.y.round() as i64,
        });
        crate::native::save_settings(&state, &s).await;
    });
}

// ── 콘솔 팝아웃 ──

fn popout(app: &AppHandle, instance_id: &str) -> Option<WebviewWindow> {
    app.get_webview_window(&popout_label(instance_id))
}

pub fn open_popout(app: &AppHandle, instance_id: &str, server_name: &str) -> tauri::Result<Value> {
    if let Some(existing) = popout(app, instance_id) {
        let _ = existing.set_focus();
        return Ok(json!({ "ok": true, "message": "Focused existing window" }));
    }
    let query = format!(
        "index.html?console-popout={}&name={}",
        encode_component(instance_id),
        encode_component(server_name)
    );
    let builder = WebviewWindowBuilder::new(app, popout_label(instance_id), WebviewUrl::App(PathBuf::from(query)))
        .title(format!("Console — {server_name}"))
        .inner_size(700.0, 450.0)
        .min_inner_size(400.0, 250.0)
        .decorations(false)
        .always_on_top(true);
    let win = lock_navigation(app, builder).build()?;
    apply_icon(&win);

    // 메인 창에 팝아웃 열림/닫힘 알림 → 임베디드 콘솔 숨김 제어
    emit_main(app, "console:popoutOpened", instance_id);
    let (handle, id) = (app.clone(), instance_id.to_string());
    win.on_window_event(move |event| {
        if matches!(event, WindowEvent::Destroyed) {
            emit_main(&handle, "console:popoutClosed", &id);
        }
    });
    Ok(json!({ "ok": true }))
}

/// 열린 팝아웃의 instanceId (메인 창을 다시 불러오면 renderer가 이것으로 상태를 맞춘다)
pub fn popout_list(app: &AppHandle) -> Vec<String> {
    app.webview_windows()
        .keys()
        .filter_map(|l| popout_instance_id(l))
        .collect()
}

pub fn focus_popout(app: &AppHandle, instance_id: &str) -> Value {
    let Some(win) = popout(app, instance_id) else {
        return json!({ "ok": false });
    };
    let _ = win.unminimize();
    let _ = win.set_focus();
    // 깜빡임으로 주의 환기
    let _ = win.request_user_attention(Some(tauri::UserAttentionType::Informational));
    tauri::async_runtime::spawn(async move {
        tokio::time::sleep(std::time::Duration::from_secs(2)).await;
        let _ = win.request_user_attention(None);
    });
    json!({ "ok": true })
}

/// PiP 콘솔을 메인 창으로 되돌린다
pub fn popin(app: &AppHandle, instance_id: &str) -> Value {
    let Some(win) = popout(app, instance_id) else {
        return json!({ "ok": false });
    };
    let server_name = win
        .url()
        .ok()
        .and_then(|u| u.query_pairs().find(|(k, _)| k == "name").map(|(_, v)| v.into_owned()))
        .or_else(|| win.title().ok().map(|t| t.trim_start_matches("Console — ").to_string()))
        .unwrap_or_default();
    // 창을 닫기 전에 메인 창에 인앱 콘솔 열기를 요청한다 (닫히면 popoutClosed가 간다)
    emit_main(
        app,
        "console:popinRequest",
        json!({ "instanceId": instance_id, "serverName": server_name }),
    );
    let _ = win.close();
    json!({ "ok": true })
}

pub fn toggle_always_on_top(app: &AppHandle, instance_id: &str, pinned: bool) -> Value {
    let Some(win) = popout(app, instance_id) else {
        return json!({ "ok": false });
    };
    let _ = win.set_always_on_top(pinned);
    if pinned {
        // 다시 고정하면 최전면으로
        let _ = win.unminimize();
        let _ = win.set_focus();
    }
    json!({ "ok": true, "pinned": pinned })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn popout_label_round_trips() {
        for id in ["a1b2-c3", "서버 #1", "x/y:z", ""] {
            let label = popout_label(id);
            assert!(label.chars().all(|c| c.is_ascii_alphanumeric() || c == '-'), "{label}");
            assert_eq!(popout_instance_id(&label).as_deref(), Some(id));
        }
        assert_eq!(popout_instance_id("main"), None);
        assert_eq!(popout_instance_id("popout-zz"), None);
        assert_eq!(popout_instance_id("popout-abc"), None);
    }

    #[test]
    fn encodes_like_encode_uri_component() {
        assert_eq!(encode_component("My Server&x=1/é"), "My%20Server%26x%3D1%2F%C3%A9");
        assert_eq!(encode_component("a-b_c.d!~*'()"), "a-b_c.d!~*'()");
    }

    #[test]
    fn navigation_guard_allows_only_app_urls() {
        let dev = Url::parse("http://localhost:5173").unwrap();
        let ok = |s: &str, d: Option<&Url>| is_app_url(&Url::parse(s).unwrap(), d);
        assert!(ok("tauri://localhost/index.html", None));
        assert!(ok("http://tauri.localhost/index.html?console-popout=1", None));
        assert!(ok("http://localhost:5173/x", Some(&dev)));
        assert!(!ok("http://localhost:5173/x", None));
        assert!(!ok("http://localhost:5174/", Some(&dev)));
        assert!(!ok("https://example.com/", Some(&dev)));
        assert!(!ok("file:///etc/passwd", Some(&dev)));
        assert!(is_web_url("https://example.com"));
        assert!(!is_web_url("file:///C:/x.exe"));
        assert!(!is_web_url("javascript:alert(1)"));
    }
}
