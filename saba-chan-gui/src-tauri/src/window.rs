//! 창 — 메인 창(크기 기억·닫기 가로채기), 콘솔 팝아웃(PiP), 트레이 팝업, 탐색 잠금
//!
//! 창은 필요할 때만 만들고 닫으면 없앤다(웹뷰를 내린다). 창이 없을 때는 트레이만 남는다 (`lifecycle.rs`)

use crate::{emit_main, AppState};
use serde_json::{json, Value};
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Mutex, OnceLock};
use std::time::Instant;
use tauri::webview::NewWindowResponse;
use tauri::window::Color;
use tauri::{AppHandle, Manager, Url, WebviewUrl, WebviewWindow, WebviewWindowBuilder, WindowEvent};
use tauri_plugin_opener::OpenerExt;

pub const MAIN: &str = "main";
const POPOUT_PREFIX: &str = "popout-";
const MIN_WIDTH: f64 = 780.0;
const BOUNDS_DEBOUNCE_MS: u64 = 500;

/// 렌더러가 테마의 바탕색을 알리기 전에 쓰는 창 바탕색 (tokens.css의 --bg-0과 같은 값)
const BG_DARK: Color = Color(0x0d, 0x0e, 0x10, 0xff);
const BG_LIGHT: Color = Color(0xe6, 0xe8, 0xed, 0xff);
/// 렌더러가 마지막으로 알린 바탕색. 새로 여는 창이 처음부터 이 색으로 뜬다
static WINDOW_BG: Mutex<Option<Color>> = Mutex::new(None);

/// 창 크기 저장 디바운스용 세대 번호
static BOUNDS_GEN: AtomicU64 = AtomicU64::new(0);

/// 닫기 요청을 렌더러에 보내고 답(app_close_response)을 아직 못 받은 시각(`monotonic_ms`). 0이면 기다리는 요청이 없다
static CLOSE_ASKED_AT: AtomicU64 = AtomicU64::new(0);
/// 이만큼 답이 없는데 OS 닫기(Alt+F4·작업 표시줄)가 다시 오면 렌더러가 멈춘 것으로 보고 인터페이스만 끝낸다 (L18c).
/// 창 안의 X는 렌더러 버튼이라 멈추면 이 경로로 오지 않는다(그때는 트레이 메뉴가 끝낸다)
const CLOSE_ANSWER_TIMEOUT_MS: u64 = 5000;
/// 이보다 오래된 물음은 렌더러가 못 받은 것(구독 전·재로드 중)으로 보고 새로 묻는다
const CLOSE_ASK_STALE_MS: u64 = 60_000;

/// 이 셸이 뜬 뒤 지난 ms + 1 (0은 "기다리는 요청 없음"). 단조 시계라 OS 시각을 바꿔도 대기 시간이 틀어지지 않는다
fn monotonic_ms() -> u64 {
    static START: OnceLock<Instant> = OnceLock::new();
    START.get_or_init(Instant::now).elapsed().as_millis() as u64 + 1
}

fn renderer_unresponsive(asked_at: u64, now: u64) -> bool {
    asked_at != 0 && (CLOSE_ANSWER_TIMEOUT_MS..CLOSE_ASK_STALE_MS).contains(&now.saturating_sub(asked_at))
}

/// 렌더러가 닫기 요청에 답했다 (받음 확인 포함)
pub fn close_answered() {
    CLOSE_ASKED_AT.store(0, Ordering::SeqCst);
}

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

/// 창 바탕색: 렌더러가 알린 색 > 테마 설정 > 시스템 테마 (모르면 기본 테마인 다크)
fn pick_background(reported: Option<Color>, theme: Option<&str>, system: Option<tauri::Theme>) -> Color {
    if let Some(c) = reported {
        return c;
    }
    match (theme, system) {
        (Some("light"), _) => BG_LIGHT,
        (Some("dark"), _) => BG_DARK,
        (_, Some(tauri::Theme::Light)) => BG_LIGHT,
        _ => BG_DARK,
    }
}

fn background_for(app: &AppHandle, system: Option<tauri::Theme>) -> Color {
    let reported = *WINDOW_BG.lock().unwrap();
    let settings = app.state::<AppState>().settings();
    pick_background(reported, settings["theme"].as_str(), system)
}

/// `#rrggbb` → 불투명 색
fn parse_hex_color(css: &str) -> Option<Color> {
    let hex = css.trim().strip_prefix('#')?;
    if hex.len() != 6 || !hex.bytes().all(|b| b.is_ascii_hexdigit()) {
        return None;
    }
    let ch = |i: usize| u8::from_str_radix(&hex[i..i + 2], 16).ok();
    Some(Color(ch(0)?, ch(2)?, ch(4)?, 0xff))
}

/// 렌더러가 테마의 바탕색을 알린다. 창 크기를 바꾸는 사이 웹뷰가 아직 못 그린 자리가 이 색으로 보인다 (기본은 흰색)
pub fn set_background(win: &WebviewWindow, css: &str) {
    // 트레이 팝업은 투명 창이라 바탕을 칠하지 않는다
    if win.label() == TRAY_POPUP {
        return;
    }
    let Some(color) = parse_hex_color(css) else {
        return;
    };
    *WINDOW_BG.lock().unwrap() = Some(color);
    let _ = win.set_background_color(Some(color));
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

/// 창 틀. Windows·리눅스는 틀 없이 앱이 그린 버튼을 쓰고, 맥은 왼쪽 위 신호등을 앱 상단바 위에 겹쳐 그린다
/// (렌더러는 `data-platform="mac"`이면 자기 창 버튼을 숨기고 신호등 자리를 비운다). `bar_height`는 그 상단바 높이
fn frame<'a>(
    builder: WebviewWindowBuilder<'a, tauri::Wry, AppHandle>,
    bar_height: f64,
) -> WebviewWindowBuilder<'a, tauri::Wry, AppHandle> {
    #[cfg(target_os = "macos")]
    {
        // 상단바 세로 가운데에 놓는다. tao는 제목 막대 높이를 (버튼 높이 14 + y)로 잡아 y와 버튼 위치가 1:1이 아니다
        // (macOS 26 실측: 48pt 막대에서 y 28이면 버튼 가운데가 27, 가운데 24에는 y 25)
        builder
            .title_bar_style(tauri::TitleBarStyle::Overlay)
            .hidden_title(true)
            .traffic_light_position(tauri::LogicalPosition::new(16.0, (bar_height - 14.0) / 2.0 + 8.0))
    }
    #[cfg(not(target_os = "macos"))]
    {
        let _ = bar_height;
        builder.decorations(false)
    }
}

/// 렌더러 상단바 높이 (`tokens.css` `--appbar-h`, `.console-popout-titlebar`)
const APPBAR_HEIGHT: f64 = 48.0;
const POPOUT_BAR_HEIGHT: f64 = 36.0;

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
        .visible(false) // 페이지가 뜰 때까지 보이지 않음
        .background_color(background_for(app, None)) // CSS 로드 전·크기 바꾸는 중 흰 화면 방지
        .on_page_load(|w, payload| {
            // 처음 뜰 때만 앞으로 올린다 (보이는 창을 새로고침했을 때 다른 앱의 포커스를 뺏지 않게)
            if matches!(payload.event(), tauri::webview::PageLoadEvent::Finished) && !w.is_visible().unwrap_or(false) {
                bring_to_front(&w);
            }
        });
    if let (Some(x), Some(y)) = (bounds["x"].as_f64(), bounds["y"].as_f64()) {
        builder = builder.position(x, y);
    }
    let win = lock_navigation(app, frame(builder, APPBAR_HEIGHT)).build()?;
    // 테마가 "시스템 따라"면 창이 생긴 뒤에야 시스템 테마를 안다
    let _ = win.set_background_color(Some(background_for(app, win.theme().ok())));
    apply_icon(&win);
    // 상주 여부는 업데이트 뒤 다시 띄울 모양을 정한다: 30초 heartbeat를 기다리지 않고 바로 알린다
    crate::lifecycle::report_resident(app, false);

    let handle = app.clone();
    win.on_window_event(move |event| match event {
        WindowEvent::Destroyed => crate::lifecycle::report_resident(&handle, tray_only_without(&handle, Some(MAIN))),
        // 닫기는 React 확인 창으로 (app_close_response가 답한다)
        WindowEvent::CloseRequested { api, .. } => {
            api.prevent_close();
            let now = monotonic_ms();
            let asked = CLOSE_ASKED_AT.load(Ordering::SeqCst);
            if renderer_unresponsive(asked, now) {
                // 멈춘 렌더러는 확인 창을 띄울 수 없다: 데몬과 서버는 두고 창만 끝낸다
                tracing::warn!("Renderer did not answer the close request; quitting the interface only");
                let app = handle.clone();
                tauri::async_runtime::spawn(async move { crate::lifecycle::interface_only_quit(&app).await });
                return;
            }
            if asked == 0 || now.saturating_sub(asked) >= CLOSE_ASK_STALE_MS {
                CLOSE_ASKED_AT.store(now, Ordering::SeqCst);
            }
            emit_main(&handle, "app:closeRequest", Value::Null);
        }
        WindowEvent::Moved(_) | WindowEvent::Resized(_) => schedule_save_bounds(&handle),
        WindowEvent::ThemeChanged(_) => crate::tray::refresh_icon(&handle),
        _ => {}
    });
    Ok(win)
}

/// 창을 보이고 다른 앱 창들 앞으로 올려 포커스를 준다.
/// Windows는 다른 앱이 앞에 있으면 포커스 요청을 막고 작업 표시줄만 깜빡일 수 있어, 잠깐 항상 위로 두었다 풀어 맨 앞에 놓는다
fn bring_to_front(w: &WebviewWindow) {
    let _ = w.unminimize();
    let _ = w.show();
    let _ = w.set_always_on_top(true);
    let _ = w.set_always_on_top(false);
    let _ = w.set_focus();
}

/// 메인 창을 앞으로. 트레이만 남아 창이 없으면 새로 만든다 (페이지가 뜨면 앞으로 올라온다)
pub fn show_main(app: &AppHandle) {
    if let Some(w) = app.get_webview_window(MAIN) {
        bring_to_front(&w);
        return;
    }
    if let Err(e) = create_main_window(app) {
        tracing::warn!("[Window] Failed to open the main window: {}", e);
    }
}

/// 사용자가 보는 창이 없고 트레이만 들었는가 (트레이 팝업은 창으로 세지 않는다)
pub fn tray_only_now(app: &AppHandle) -> bool {
    tray_only_without(app, None)
}

/// 막 사라지는 창(`gone`)을 빼고 본다 (Destroyed 이벤트 중에는 아직 목록에 있을 수 있다)
fn tray_only_without(app: &AppHandle, gone: Option<&str>) -> bool {
    let labels: Vec<String> = app.webview_windows().into_keys().collect();
    tray_only(labels.iter().map(String::as_str).filter(|l| Some(*l) != gone))
}

fn tray_only<'a>(mut labels: impl Iterator<Item = &'a str>) -> bool {
    labels.all(|l| l == TRAY_POPUP)
}

/// 창을 모두 없앤다 (닫기 확인을 거치지 않는다). 웹뷰가 내려가고 트레이만 남는다
pub fn close_all(app: &AppHandle) {
    for (_, w) in app.webview_windows() {
        if let Err(e) = w.destroy() {
            tracing::warn!("[Window] Failed to close {}: {}", w.label(), e);
        }
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
        let bounds = json!({ "windowBounds": {
            "width": size.width.round() as i64, "height": size.height.round() as i64,
            "x": pos.x.round() as i64, "y": pos.y.round() as i64,
        }});
        crate::native::save_settings(&state, &bounds).await;
    });
}

// ── 트레이 팝업 ──

pub const TRAY_POPUP: &str = "tray-popup";
const TRAY_POPUP_WIDTH: f64 = 320.0;
/// 내용 높이를 받기 전 처음 크기 (보이지 않는다)
const TRAY_POPUP_INITIAL_HEIGHT: f64 = 360.0;
const TRAY_POPUP_GAP: f64 = 8.0;
/// 팝업이 포커스를 잃어 닫힌 직후의 아이콘 클릭은 다시 열지 않는다 (아이콘을 눌러 닫는 경우)
const TRAY_REOPEN_GUARD_MS: u64 = 300;

/// 물리 px 사각형 (x, y, 너비, 높이)
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct PxRect {
    pub x: f64,
    pub y: f64,
    pub w: f64,
    pub h: f64,
}

/// 마지막으로 누른 트레이 아이콘 자리. 팝업 높이가 정해지면 이 자리에 맞춰 놓는다
static TRAY_ANCHOR: Mutex<Option<PxRect>> = Mutex::new(None);
/// 팝업이 포커스를 잃어 닫힌 시각 (`monotonic_ms`)
static TRAY_POPUP_CLOSED_AT: AtomicU64 = AtomicU64::new(0);
/// 지금 팝업이 보였는가 (`fit_tray_popup`이 보인 뒤 true, 새로 만들 때 false)
static TRAY_POPUP_SHOWN: AtomicBool = AtomicBool::new(false);

/// 숨긴 채 만들어지는 동안에도 포커스 잃음이 온다: 보인 뒤에만 그것으로 닫는다
fn popup_closes_on_blur(shown: bool) -> bool {
    shown
}

/// 아이콘 옆에 팝업을 놓을 자리 (물리 px). 가로는 아이콘 가운데, 아이콘이 작업 영역 아래쪽이면 위로,
/// 위쪽이면 아래로 띄운다. 작업 영역(작업 표시줄을 뺀 화면) 밖으로 나가지 않게 당긴다
pub fn popup_position(icon: PxRect, size: (f64, f64), work: PxRect, gap: f64) -> (f64, f64) {
    let (w, h) = size;
    let x = icon.x + icon.w / 2.0 - w / 2.0;
    let below_middle = icon.y + icon.h / 2.0 > work.y + work.h / 2.0;
    let y = if below_middle {
        icon.y - h - gap
    } else {
        icon.y + icon.h + gap
    };
    let clamp = |v: f64, lo: f64, hi: f64| v.min(hi).max(lo);
    (
        clamp(x, work.x, work.x + work.w - w),
        clamp(y, work.y, work.y + work.h - h),
    )
}

/// 트레이 아이콘을 눌렀다: 팝업이 있으면 닫고, 없으면 아이콘 옆에 연다
pub fn toggle_tray_popup(app: &AppHandle, anchor: Option<PxRect>) {
    if let Some(w) = app.get_webview_window(TRAY_POPUP) {
        let _ = w.destroy();
        return;
    }
    let closed_at = TRAY_POPUP_CLOSED_AT.load(Ordering::SeqCst);
    if closed_at != 0 && monotonic_ms().saturating_sub(closed_at) < TRAY_REOPEN_GUARD_MS {
        return;
    }
    *TRAY_ANCHOR.lock().unwrap() = anchor;
    TRAY_POPUP_SHOWN.store(false, Ordering::SeqCst);
    let builder = WebviewWindowBuilder::new(app, TRAY_POPUP, WebviewUrl::App("index.html?tray-popup=1".into()))
        .title("Saba-chan")
        .inner_size(TRAY_POPUP_WIDTH, TRAY_POPUP_INITIAL_HEIGHT)
        .decorations(false)
        .transparent(true)
        .shadow(false)
        .resizable(false)
        .skip_taskbar(true)
        .always_on_top(true)
        // 렌더러가 높이를 알려 오면(tray_popup_fit) 자리를 잡고 보인다
        .visible(false);
    let win = match lock_navigation(app, builder).build() {
        Ok(w) => w,
        Err(e) => {
            // 팝업을 못 띄우면 메인 창이라도 연다
            tracing::warn!("[Tray] Failed to open the tray popup: {}", e);
            return show_main(app);
        }
    };
    let popup = win.clone();
    win.on_window_event(move |event| {
        if let WindowEvent::Focused(false) = event {
            if !popup_closes_on_blur(TRAY_POPUP_SHOWN.load(Ordering::SeqCst)) {
                return;
            }
            TRAY_POPUP_CLOSED_AT.store(monotonic_ms(), Ordering::SeqCst);
            let _ = popup.destroy();
        }
    });
}

/// 렌더러가 잰 팝업 내용 높이(논리 px)에 맞춰 크기와 자리를 정하고 보인다
pub fn fit_tray_popup(app: &AppHandle, height: f64) {
    let Some(win) = app.get_webview_window(TRAY_POPUP) else {
        return;
    };
    let anchor = *TRAY_ANCHOR.lock().unwrap();
    let monitor = anchor
        .and_then(|a| app.monitor_from_point(a.x, a.y).ok().flatten())
        .or_else(|| app.primary_monitor().ok().flatten());
    // 배율은 팝업이 처음 뜬 모니터가 아니라 아이콘이 있는 모니터의 것 (모니터마다 배율이 다를 수 있다)
    let scale = monitor
        .as_ref()
        .map(|m| m.scale_factor())
        .unwrap_or_else(|| win.scale_factor().unwrap_or(1.0));
    let size = (TRAY_POPUP_WIDTH * scale, height.clamp(120.0, 640.0) * scale);
    if let Some(m) = monitor {
        let area = m.work_area();
        let work = PxRect {
            x: area.position.x as f64,
            y: area.position.y as f64,
            w: area.size.width as f64,
            h: area.size.height as f64,
        };
        // 아이콘 자리를 모르면(키보드로 연 경우 등) 작업 영역 오른쪽 아래
        let icon = anchor.unwrap_or(PxRect {
            x: work.x + work.w,
            y: work.y + work.h,
            w: 0.0,
            h: 0.0,
        });
        let (x, y) = popup_position(icon, size, work, TRAY_POPUP_GAP * scale);
        // 먼저 옮기고 크기를 준다: 배율이 다른 모니터로 옮기면 창 크기가 그 배율로 다시 잡힌다
        let _ = win.set_position(tauri::PhysicalPosition::new(x, y));
    }
    let _ = win.set_size(tauri::PhysicalSize::new(size.0, size.1));
    let _ = win.show();
    let _ = win.set_focus();
    TRAY_POPUP_SHOWN.store(true, Ordering::SeqCst);
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
    let background = background_for(app, app.get_webview_window(MAIN).and_then(|w| w.theme().ok()));
    let builder = WebviewWindowBuilder::new(app, popout_label(instance_id), WebviewUrl::App(PathBuf::from(query)))
        .title(format!("Console — {server_name}"))
        .inner_size(700.0, 450.0)
        .min_inner_size(400.0, 250.0)
        .background_color(background)
        .always_on_top(true);
    let win = lock_navigation(app, frame(builder, POPOUT_BAR_HEIGHT)).build()?;
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

/// 팝아웃 콘솔을 모두 닫는다. 다룰 데몬을 바꾸면 그 창들의 instanceId는 다른 PC의 것이 된다
pub fn close_popouts(app: &AppHandle) {
    for (label, win) in app.webview_windows() {
        if popout_instance_id(&label).is_some() {
            let _ = win.close();
        }
    }
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
    fn the_window_background_follows_the_reported_color_then_the_theme() {
        let reported = Color(1, 2, 3, 0xff);
        assert_eq!(pick_background(Some(reported), Some("light"), None), reported);
        assert_eq!(pick_background(None, Some("light"), Some(tauri::Theme::Dark)), BG_LIGHT);
        assert_eq!(pick_background(None, Some("dark"), Some(tauri::Theme::Light)), BG_DARK);
        assert_eq!(pick_background(None, Some("auto"), Some(tauri::Theme::Light)), BG_LIGHT);
        assert_eq!(pick_background(None, None, Some(tauri::Theme::Dark)), BG_DARK);
        assert_eq!(pick_background(None, Some("auto"), None), BG_DARK);
    }

    #[test]
    fn only_six_digit_hex_colors_are_accepted() {
        assert_eq!(parse_hex_color(" #0d0E10"), Some(Color(0x0d, 0x0e, 0x10, 0xff)));
        assert_eq!(parse_hex_color("#fff"), None);
        assert_eq!(parse_hex_color("rgb(1, 2, 3)"), None);
        assert_eq!(parse_hex_color("#gg0000"), None);
        assert_eq!(parse_hex_color("#ééé"), None);
    }

    #[test]
    fn the_tray_popup_may_call_what_it_needs_and_nothing_that_notifies() {
        let dir = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("capabilities");
        let mut granted: Vec<String> = Vec::new();
        for entry in std::fs::read_dir(dir).unwrap() {
            let cap: serde_json::Value =
                serde_json::from_str(&std::fs::read_to_string(entry.unwrap().path()).unwrap()).unwrap();
            let windows = cap["windows"].as_array().cloned().unwrap_or_default();
            if windows.iter().any(|w| w == TRAY_POPUP) {
                granted.extend(
                    cap["permissions"]
                        .as_array()
                        .unwrap()
                        .iter()
                        .filter_map(|p| p.as_str().map(String::from)),
                );
            }
        }
        for need in [
            "allow-daemon-request",
            "allow-daemon-status",
            "allow-daemon-target-get",
            "allow-language-get",
            "allow-tray-action",
            "allow-tray-popup-fit",
        ] {
            assert!(granted.iter().any(|g| g == need), "the tray popup cannot call {need}");
        }
        assert!(
            !granted.iter().any(|g| g == "allow-notify"),
            "every new popup would repeat old notices"
        );
        let build = include_str!("../build.rs");
        for cmd in ["\"tray_action\"", "\"tray_popup_fit\""] {
            assert!(build.contains(cmd), "{cmd} is missing from the app manifest");
        }
    }

    #[test]
    fn a_tray_popup_closes_on_blur_only_after_it_is_shown() {
        // 숨긴 채 만들어지는 동안 받은 포커스 잃음으로 닫히면 팝업이 보이기도 전에 사라진다 (PC 실측 80ms)
        assert!(!popup_closes_on_blur(false));
        assert!(popup_closes_on_blur(true));
    }

    #[test]
    fn only_the_tray_popup_left_means_tray_only() {
        assert!(tray_only([].into_iter()));
        assert!(tray_only([TRAY_POPUP].into_iter()));
        assert!(!tray_only([MAIN].into_iter()));
        assert!(
            !tray_only(["popout-abc"].into_iter()),
            "a console pop-out is a window the user sees"
        );
    }

    #[test]
    fn close_falls_through_only_when_the_renderer_never_answered() {
        // 묻지 않았거나 답을 받았으면(0) 렌더러가 묻는다
        assert!(!renderer_unresponsive(0, 1_000_000));
        let first = monotonic_ms();
        assert!(
            first > 0 && monotonic_ms() >= first,
            "0 means no request, and the clock never goes back"
        );
        // 물은 지 얼마 안 됐으면 기다린다
        let asked = 1_000_000;
        assert!(!renderer_unresponsive(asked, asked + CLOSE_ANSWER_TIMEOUT_MS - 1));
        // 답 없이 지났으면 멈춘 것으로 본다
        assert!(renderer_unresponsive(asked, asked + CLOSE_ANSWER_TIMEOUT_MS));
        // 오래된 물음은 렌더러가 못 받은 것: 확인 없이 끝내지 않고 다시 묻는다
        assert!(!renderer_unresponsive(asked, asked + CLOSE_ASK_STALE_MS));
    }

    #[test]
    fn the_tray_popup_sits_next_to_the_icon_inside_the_work_area() {
        let work = PxRect {
            x: 0.0,
            y: 0.0,
            w: 1920.0,
            h: 1040.0,
        };
        // 아래 작업 표시줄: 아이콘 위에, 가운데를 맞춘다 (작업 표시줄을 덮지 않게 작업 영역 끝까지)
        let icon = PxRect {
            x: 1700.0,
            y: 1050.0,
            w: 24.0,
            h: 24.0,
        };
        assert_eq!(popup_position(icon, (320.0, 400.0), work, 8.0), (1552.0, 640.0));
        // 오른쪽 끝 아이콘: 화면 밖으로 나가지 않게 당긴다
        let corner = PxRect {
            x: 1900.0,
            y: 1050.0,
            w: 24.0,
            h: 24.0,
        };
        assert_eq!(popup_position(corner, (320.0, 400.0), work, 8.0).0, 1600.0);
        // 위 작업 표시줄: 아이콘 아래에
        let top = PxRect {
            x: 0.0,
            y: 40.0,
            w: 1920.0,
            h: 1040.0,
        };
        let icon = PxRect {
            x: 1700.0,
            y: 8.0,
            w: 24.0,
            h: 24.0,
        };
        assert_eq!(popup_position(icon, (320.0, 400.0), top, 8.0), (1552.0, 40.0));
    }

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
