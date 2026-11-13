//! 바탕화면 오버레이 창 — 익스텐션 slot `Desktop.overlay`를 그리는 작은 창 (데스크톱 펫 등)
//!
//! 셸은 창만 준다: 투명·틀 없음·항상 위·작업 표시줄에 없음. 무엇을 그릴지는 그 slot을 선언한 익스텐션이 정한다.
//! - 켜진 익스텐션 중 하나라도 `Desktop.overlay`를 선언했고(번들을 내줄 수 있고) 설정 `desktopOverlay.enabled`가 켜져 있을 때만 띄운다
//! - 창은 내용 크기에 맞추고(`overlay_fit`) 아래 가운데 점을 붙박이로 둔다. 끌어 옮긴 자리는 `desktopOverlay.{x,y}`(물리 px)에 남는다
//! - 끌기(`overlay_drag`)로 옮긴 자리만 기억한다. 모니터를 빼서 OS가 옮긴 자리는 기억하지 않고, 모니터 구성을 지켜보다
//!   고른 자리가 있는 모니터가 돌아오면 그 자리로 되돌린다
//! - 우클릭하면(`overlay_menu`) 펫 옆에 작은 판 창 `overlay-menu`를 띄운다(트레이 팝업과 같은 `window::Popup`):
//!   서버 켜고 끄기, 펫 크기(`desktopOverlay.scale`, `overlay_set_scale`), 앱 열기, 숨기기. 포커스를 잃거나 Esc면 닫힌다
//! - 사용자가 보는 창으로 세지 않는다: 트레이만 남은 상태 판단(`window::tray_only`)과 "창 모두 닫기"에서 빠진다
//!
//! 리눅스 Wayland는 창 자리와 항상 위를 앱이 정할 수 없어 띄우지 않는다.

use crate::window::PxRect;
use crate::AppState;
use serde_json::{json, Value};
use std::sync::atomic::{AtomicBool, AtomicU32, AtomicU64, Ordering};
use std::sync::Mutex;
use tauri::{AppHandle, Emitter, Manager, WebviewUrl, WebviewWindow, WebviewWindowBuilder, WindowEvent};

pub const OVERLAY: &str = "desktop-overlay";
/// 익스텐션 manifest `gui.slots`의 이 창 slot 이름 (`extensions/slotTable.ts`와 같다)
pub const OVERLAY_SLOT: &str = "Desktop.overlay";
/// 펫 우클릭 판 창
pub const OVERLAY_MENU: &str = "overlay-menu";
static MENU: crate::window::Popup = crate::window::Popup::new(OVERLAY_MENU, "index.html?overlay-menu=1", 280.0);
/// 펫 크기 (%): 기본, 한 칸 (한도는 데몬 설정 `DESKTOP_OVERLAY_SCALE` 50~200과 같다)
const DEFAULT_SCALE: u32 = 100;
const SCALE_MIN: u32 = 50;
const SCALE_MAX: u32 = 200;
const SCALE_STEP: f64 = 10.0;
/// 내용 크기를 받기 전 처음 크기 (보이지 않는다)
const INITIAL_SIZE: (f64, f64) = (200.0, 200.0);
/// 창 크기 한도 (논리 px)
const MIN_SIDE: f64 = 48.0;
const MAX_WIDTH: f64 = 360.0;
const MAX_HEIGHT: f64 = 420.0;
/// 처음 자리: 작업 영역 오른쪽 아래에서 띄우는 간격 (논리 px)
const MARGIN: f64 = 16.0;
const SAVE_DEBOUNCE_MS: u64 = 600;
/// 끌기를 시작한 뒤 첫 이동을 기다리는 시간, 그 뒤 이동마다 늘리는 시간 (ms). 이 안의 이동만 사용자가 옮긴 것이다
const DRAG_START_MS: u64 = 10_000;
const DRAG_MOVE_MS: u64 = 2_000;
/// 모니터를 꽂고 빼거나 OS가 창을 옮겼는지 보는 간격
const WATCH_MS: u64 = 3_000;
/// 화면 밖에 걸친 펫을 그 모니터의 것으로 볼 거리 (물리 px). 뺀 모니터의 자리는 보통 이보다 멀다
const NEAR_SCREEN: f64 = 600.0;

/// 사용자가 고른 아래 가운데 점 (물리 px). 끌어 옮길 때만 바뀐다: 크기가 바뀌어 가장자리에서 당겨도,
/// 그 모니터를 빼서 다른 모니터에 놓아도 이 점은 그대로다
static ANCHOR: Mutex<Option<(f64, f64)>> = Mutex::new(None);
/// 셸이 마지막으로 놓은 창의 아래 가운데 점: 이와 다른 자리로 옮겨졌으면 사용자가 끈 것이다
static PLACED: Mutex<Option<(f64, f64)>> = Mutex::new(None);
/// 셸이 창을 옮기고 크기를 바꾸는 중: 그 사이의 이동 알림은 사용자가 끈 것이 아니다
static PLACING: AtomicBool = AtomicBool::new(false);
static SAVE_GEN: AtomicU64 = AtomicU64::new(0);
/// 렌더러가 마지막으로 알린 내용 크기 (논리 px). 모니터가 바뀌면 이 크기로 다시 놓는다
static LAST_FIT: Mutex<Option<(f64, f64)>> = Mutex::new(None);
/// 이 시각(유닉스 ms)까지의 이동은 사용자가 끈 것이다
static DRAG_UNTIL: AtomicU64 = AtomicU64::new(0);
/// 창을 열 때마다 늘린다. 옛 창의 지켜보기는 멈춘다
static OPEN_GEN: AtomicU64 = AtomicU64::new(0);
/// 지금 펫 크기 (%). 창을 열 때 설정에서 읽고 크기 막대가 바꾼다
static SCALE: AtomicU32 = AtomicU32::new(DEFAULT_SCALE);
static SCALE_GEN: AtomicU64 = AtomicU64::new(0);

/// 이 데스크톱에서 띄울 수 있는가 (Wayland는 자리·항상 위를 앱이 정할 수 없다)
pub fn supported() -> bool {
    !wayland(std::env::var("XDG_SESSION_TYPE").ok().as_deref(), std::env::var_os("WAYLAND_DISPLAY").is_some())
}

fn wayland(session: Option<&str>, wayland_display: bool) -> bool {
    cfg!(target_os = "linux") && (session.is_some_and(|s| s.eq_ignore_ascii_case("wayland")) || wayland_display)
}

/// 익스텐션 목록(`GET /api/extensions`)에 이 창을 쓸 익스텐션이 있는가:
/// 켜져 있고, 번들이 있고, 이 PC가 번들을 받을 수 있고(`gui_allowed`), `Desktop.overlay`를 선언했다
pub fn wanted_by(list: &Value) -> bool {
    list["extensions"].as_array().is_some_and(|exts| {
        exts.iter().any(|e| {
            e["enabled"].as_bool() == Some(true)
                && e["gui_allowed"].as_bool() != Some(false)
                && e["gui"]["bundle"].as_str().is_some_and(|b| !b.is_empty())
                && e["gui"]["slots"].get(OVERLAY_SLOT).is_some()
        })
    })
}

/// 설정 `desktopOverlay` (없으면 켬·자리 없음·100%)
#[derive(Debug, PartialEq)]
struct Saved {
    enabled: bool,
    at: Option<(f64, f64)>,
    scale: u32,
}

fn saved(settings: &Value) -> Saved {
    let o = &settings["desktopOverlay"];
    Saved {
        enabled: o["enabled"].as_bool().unwrap_or(true),
        at: o["x"].as_f64().zip(o["y"].as_f64()),
        scale: o["scale"].as_f64().map_or(DEFAULT_SCALE, scale_of),
    }
}

/// `desktopOverlay`의 몇 칸만 바꾸는 설정 저장 본문. GUI 설정 저장은 맨 위 키를 통째로 바꾸므로 나머지 칸을 함께 싣는다
fn patch(settings: &Value, change: Value) -> Value {
    let mut overlay = match &settings["desktopOverlay"] {
        Value::Object(o) => o.clone(),
        _ => serde_json::Map::new(),
    };
    if let Value::Object(change) = change {
        overlay.extend(change);
    }
    json!({ "desktopOverlay": overlay })
}

/// 펫 크기 (%): 10% 칸, 50~200%
fn scale_of(value: f64) -> u32 {
    if !value.is_finite() {
        return DEFAULT_SCALE;
    }
    ((value / SCALE_STEP).round() * SCALE_STEP).clamp(SCALE_MIN as f64, SCALE_MAX as f64) as u32
}

/// 창 크기 (물리 px): 내용 크기(논리 px)를 한도 안에 두고 모니터 배율을 곱한다. 한도는 펫 크기만큼 늘어난다
fn window_size(width: f64, height: f64, scale: u32, dpi: f64) -> (f64, f64) {
    let grow = scale as f64 / 100.0;
    (
        (width.clamp(MIN_SIDE, MAX_WIDTH * grow) * dpi).round(),
        (height.clamp(MIN_SIDE, MAX_HEIGHT * grow) * dpi).round(),
    )
}

/// 데몬 요청을 읽기와 서버 켜고 끄기로 묶는 창 (`allows_request`): 오버레이와 그 우클릭 판
pub fn limited(label: &str) -> bool {
    label == OVERLAY || label == OVERLAY_MENU
}

/// 창 자리(왼쪽 위, 물리 px). `anchor`는 아래 가운데 점이고, 없으면 작업 영역 오른쪽 아래에서 `margin`만큼 띄운다.
/// 창이 작업 영역 밖으로 나가지 않게 당긴다
pub fn place(anchor: Option<(f64, f64)>, size: (f64, f64), work: PxRect, margin: f64) -> (f64, f64) {
    let (w, h) = size;
    let (cx, bottom) = anchor.unwrap_or((work.x + work.w - margin - w / 2.0, work.y + work.h - margin));
    let clamp = |v: f64, lo: f64, hi: f64| v.min(hi).max(lo);
    (
        clamp(cx - w / 2.0, work.x, work.x + work.w - w),
        clamp(bottom - h, work.y, work.y + work.h - h),
    )
}

/// 놓을 자리와, 앞으로 지킬 아래 가운데 점. 지킬 점은 당기기 전의 점이다: 말풍선으로 넓어져 가장자리에서 안쪽으로
/// 당겨져도 좁아지면 원래 자리로 돌아온다. 처음(점 없음)이면 이 크기로 잡은 오른쪽 아래 점
pub fn settle(anchor: Option<(f64, f64)>, size: (f64, f64), work: PxRect, margin: f64) -> ((f64, f64), (f64, f64)) {
    let intended = anchor.unwrap_or((work.x + work.w - margin - size.0 / 2.0, work.y + work.h - margin));
    let (x, y) = place(Some(intended), size, work, margin);
    ((x.round(), y.round()), intended)
}

/// 설정과 익스텐션 목록을 보고 창을 띄우거나 내린다. 데몬에 닿지 않으면 지금 상태를 그대로 둔다
pub async fn sync(app: &AppHandle) {
    let state = app.state::<AppState>();
    if !saved(&state.settings()).enabled || !supported() {
        return close(app);
    }
    let Some(list) = local_extensions(&state).await else {
        return;
    };
    if wanted_by(&list) {
        open(app);
    } else {
        close(app);
    }
}

/// 이 PC 데몬의 익스텐션 목록. 오버레이는 번들을 늘 이 PC 데몬에서 받으므로(`target: 'local'`),
/// 원격 데몬을 고른 동안에도 띄울지는 이 목록으로 정한다
async fn local_extensions(state: &AppState) -> Option<Value> {
    match state.client.call_to(reqwest::Method::GET, "/api/extensions", None, 5000, Some("local")).await {
        Ok(list) => Some(list),
        Err(e) => {
            tracing::debug!("[Overlay] Extension list unavailable: {}", e.message);
            None
        }
    }
}

/// 이벤트를 오버레이 창에만 보낸다 (없으면 아무것도 하지 않는다)
pub fn emit<S: serde::Serialize + Clone>(app: &AppHandle, event: &str, payload: S) {
    if app.get_webview_window(OVERLAY).is_some() {
        let _ = app.emit_to(OVERLAY, event, payload);
    }
}

pub fn close(app: &AppHandle) {
    close_menu(app);
    // 맥은 패널을 원래 창으로 되돌린 뒤 없앤다. 둘 다 메인 스레드에서 차례로 한다
    #[cfg(target_os = "macos")]
    {
        let handle = app.clone();
        let _ = app.run_on_main_thread(move || {
            mac_panel::release(&handle);
            if let Some(w) = handle.get_webview_window(OVERLAY) {
                let _ = w.destroy();
            }
        });
    }
    #[cfg(not(target_os = "macos"))]
    if let Some(w) = app.get_webview_window(OVERLAY) {
        let _ = w.destroy();
    }
}

/// 창을 만든다 (이미 있으면 그대로). 내용 크기를 알려 오면(`fit`) 자리를 잡고 보인다
fn open(app: &AppHandle) {
    if app.get_webview_window(OVERLAY).is_some() {
        return;
    }
    let scale = saved(&app.state::<AppState>().settings()).scale;
    SCALE.store(scale, Ordering::SeqCst);
    // 펫 크기는 주소로 처음 값을 주고, 바뀌면 `overlay:scale`로 알린다 (이 창은 설정을 읽지 않는다)
    let url = format!("index.html?desktop-overlay=1&scale={scale}");
    let builder = WebviewWindowBuilder::new(app, OVERLAY, WebviewUrl::App(url.into()))
        .title("Saba-chan")
        .inner_size(INITIAL_SIZE.0, INITIAL_SIZE.1)
        .decorations(false)
        .transparent(true)
        .shadow(false)
        .resizable(false)
        .maximizable(false)
        .minimizable(false)
        .skip_taskbar(true)
        .always_on_top(true)
        .focused(false)
        // 맥: 패널은 키 창이 되지 않으므로(아래 mac_panel) 앞에 없는 창의 첫 클릭도 웹뷰가 받게 한다
        .accept_first_mouse(true)
        .visible(false);
    let win = match crate::window::lock_navigation(app, builder).build() {
        Ok(w) => w,
        Err(e) => {
            tracing::warn!("[Overlay] Failed to open the desktop overlay: {}", e);
            return;
        }
    };
    // AppKit 창 속성은 메인 스레드에서만 바꾼다 (open은 비동기 작업 스레드에서 불린다. 밖에서 바꾸면 macOS 26이 단언으로 멈춘다)
    #[cfg(target_os = "macos")]
    {
        let panel_win = win.clone();
        if let Err(e) = app.run_on_main_thread(move || mac_panel::adopt(&panel_win)) {
            tracing::warn!("[Overlay] Could not make the overlay a panel: {}", e);
        }
    }
    *ANCHOR.lock().unwrap() = None;
    *PLACED.lock().unwrap() = None;
    *LAST_FIT.lock().unwrap() = None;
    watch(app);
    let handle = app.clone();
    win.on_window_event(move |event| {
        // 셸이 놓은 자리(`fit`)와 OS가 옮긴 자리도 Moved로 온다: 끄는 중이 아니거나 놓은 점과 같으면 remember_moved가 거른다
        if let WindowEvent::Moved(_) = event {
            remember_moved(&handle);
        }
    });
}

/// 맥에서는 창을 NSPanel로 바꾼다(tauri-nspanel): 눌러도 사바쨩 앱이 앞으로 오지 않고(nonactivating),
/// 키 입력을 가져가지 않으며(게임 중 키가 펫으로 가지 않게), 모든 데스크톱과 전체 화면 앱의 Space에도 보인다.
/// 일반 창의 "항상 위"(NSFloatingWindowLevel)만으로는 한 Space에만 있고 누르면 앱이 활성화된다
#[cfg(target_os = "macos")]
mod mac_panel {
    use tauri::{AppHandle, WebviewWindow};
    use tauri_nspanel::{tauri_panel, CollectionBehavior, ManagerExt, PanelLevel, StyleMask, WebviewWindowExt};

    tauri_panel! {
        panel!(OverlayPanel {
            config: {
                can_become_key_window: false,
                can_become_main_window: false,
                is_floating_panel: true
            }
        })
    }

    pub fn adopt(win: &WebviewWindow) {
        let panel = match win.to_panel::<OverlayPanel<tauri::Wry>>() {
            Ok(p) => p,
            Err(e) => return tracing::warn!("[Overlay] Could not make the overlay a panel: {}", e),
        };
        // 창은 Tauri가 닫으며 놓는다. AppKit이 따로 놓지 않게 한다
        panel.set_released_when_closed(false);
        panel.set_level(PanelLevel::Floating.value());
        panel.set_collection_behavior(
            CollectionBehavior::new()
                .can_join_all_spaces()
                .full_screen_auxiliary()
                .stationary()
                .ignores_cycle()
                .value(),
        );
        if let Err(e) = panel.add_style_mask(StyleMask::empty().nonactivating_panel().value()) {
            tracing::warn!("[Overlay] Could not make the overlay non-activating: {}", e);
        }
    }

    /// 닫기 전에 원래 창으로 되돌린다 (Tauri가 자기 창으로 닫는다)
    pub fn release(app: &AppHandle) {
        if let Ok(panel) = app.get_webview_panel(super::OVERLAY) {
            let _ = panel.to_window();
        }
    }
}

/// 그릴 것이 있는 크기인가. slot이 비면(번들을 못 읽었다 등) 내용 크기가 0이다
fn has_content(width: f64, height: f64) -> bool {
    width >= 1.0 && height >= 1.0
}

/// 렌더러가 잰 내용 크기(논리 px)에 맞춰 크기와 자리를 정하고 보인다. 그릴 것이 없으면 숨긴다:
/// 보이지 않는 항상-위 창이 바탕화면 클릭을 가로채지 않게
pub fn fit(app: &AppHandle, width: f64, height: f64) {
    let Some(win) = app.get_webview_window(OVERLAY) else {
        return;
    };
    if !has_content(width, height) {
        *LAST_FIT.lock().unwrap() = None;
        let _ = win.hide();
        return;
    }
    *LAST_FIT.lock().unwrap() = Some((width, height));
    let Some((pos, size)) = plan(app, width, height) else {
        let _ = win.show();
        return;
    };
    *PLACED.lock().unwrap() = Some((pos.0 + size.0 / 2.0, pos.1 + size.1));
    PLACING.store(true, Ordering::SeqCst);
    // 먼저 옮기고 크기를 준다: 배율이 다른 모니터로 옮기면 창 크기가 그 배율로 다시 잡힌다
    let _ = win.set_position(tauri::PhysicalPosition::new(pos.0, pos.1));
    let _ = win.set_size(tauri::PhysicalSize::new(size.0, size.1));
    PLACING.store(false, Ordering::SeqCst);
    let _ = win.show();
}

/// 이 내용 크기(논리 px)를 놓을 자리와 크기 (물리 px). 고른 점이 있는 모니터에, 그 모니터가 없으면 주 모니터의 처음 자리에
fn plan(app: &AppHandle, width: f64, height: f64) -> Option<((f64, f64), (f64, f64))> {
    let anchor = *ANCHOR.lock().unwrap();
    let anchor = anchor.or_else(|| saved(&app.state::<AppState>().settings()).at);
    let monitors = app.available_monitors().unwrap_or_default();
    let screens: Vec<PxRect> = monitors.iter().map(|m| rect(*m.position(), *m.size())).collect();
    let (monitor, anchor) = match screen_for(anchor, &screens) {
        Some(i) => (monitors[i].clone(), anchor),
        None => (app.primary_monitor().ok().flatten().or_else(|| monitors.first().cloned())?, None),
    };
    let dpi = monitor.scale_factor();
    // 물리 px는 정수로: 창이 옮겨진 뒤 잰 자리와 기억한 점이 어긋나 사용자가 옮긴 것으로 보지 않게
    let size = window_size(width, height, SCALE.load(Ordering::SeqCst), dpi);
    let area = monitor.work_area();
    let (pos, _) = settle(anchor, size, rect(area.position, area.size), MARGIN * dpi);
    Some((pos, size))
}

fn rect(position: tauri::PhysicalPosition<i32>, size: tauri::PhysicalSize<u32>) -> PxRect {
    PxRect {
        x: position.x as f64,
        y: position.y as f64,
        w: size.width as f64,
        h: size.height as f64,
    }
}

/// 아래 가운데 점이 있는 모니터의 번호. 점이 화면 가장자리 밖에 걸쳐 있으면 `NEAR_SCREEN` 안의 가장 가까운 모니터.
/// 없으면(점이 없거나 그 모니터를 뺐다) None.
/// 모니터 전체로 본다: 작업 표시줄 위에 내려놓아도 그 모니터다 (작업 영역 안으로는 `settle`이 당긴다)
fn screen_for(anchor: Option<(f64, f64)>, screens: &[PxRect]) -> Option<usize> {
    let (x, y) = anchor?;
    let gap = |w: &PxRect| {
        let dx = (w.x - x).max(x - (w.x + w.w)).max(0.0);
        let dy = (w.y - y).max(y - (w.y + w.h)).max(0.0);
        dx.hypot(dy)
    };
    screens
        .iter()
        .position(|w| x >= w.x && x <= w.x + w.w && y > w.y && y <= w.y + w.h)
        .or_else(|| {
            screens
                .iter()
                .enumerate()
                .map(|(i, w)| (i, gap(w)))
                .filter(|&(_, d)| d <= NEAR_SCREEN)
                .min_by(|a, b| a.1.total_cmp(&b.1))
                .map(|(i, _)| i)
        })
}

fn now_ms() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |d| d.as_millis() as u64)
}

/// 지금(`now`) 사용자가 끄는 중인가
fn dragging(now: u64, until: u64) -> bool {
    now < until
}

/// 렌더러가 펫을 누른 채 끌기 시작했다: 이제부터의 이동을 사용자가 옮긴 것으로 본다
pub fn drag(app: &AppHandle) {
    let Some(win) = app.get_webview_window(OVERLAY) else {
        return;
    };
    DRAG_UNTIL.store(now_ms() + DRAG_START_MS, Ordering::SeqCst);
    if let Err(e) = win.start_dragging() {
        DRAG_UNTIL.store(0, Ordering::SeqCst);
        tracing::debug!("[Overlay] Could not start dragging: {}", e);
    }
}

/// 창이 있는 동안 모니터 구성과 창 자리를 지켜본다: 놓아야 할 자리와 지금 자리가 다르면(모니터를 꽂거나 뺐다,
/// OS가 옮겼다) 다시 놓는다. 사용자가 끄는 중에는 건드리지 않는다
fn watch(app: &AppHandle) {
    let generation = OPEN_GEN.fetch_add(1, Ordering::SeqCst) + 1;
    let app = app.clone();
    tauri::async_runtime::spawn(async move {
        loop {
            tokio::time::sleep(std::time::Duration::from_millis(WATCH_MS)).await;
            if OPEN_GEN.load(Ordering::SeqCst) != generation {
                return;
            }
            let Some(win) = app.get_webview_window(OVERLAY) else {
                return;
            };
            let Some((width, height)) = *LAST_FIT.lock().unwrap() else {
                continue;
            };
            if dragging(now_ms(), DRAG_UNTIL.load(Ordering::SeqCst)) {
                continue;
            }
            let (Some((pos, size)), Ok(at)) = (plan(&app, width, height), win.outer_position()) else {
                continue;
            };
            if (pos.0 - at.x as f64).abs() > 1.0 || (pos.1 - at.y as f64).abs() > 1.0 {
                fit(&app, width, height);
            } else if let Ok(now) = win.outer_size() {
                if (size.0 - now.width as f64).abs() > 1.0 || (size.1 - now.height as f64).abs() > 1.0 {
                    fit(&app, width, height);
                }
            }
        }
    });
}

/// 창의 아래 가운데 점 (물리 px)
fn anchor_of(win: &WebviewWindow) -> Option<(f64, f64)> {
    let (pos, size) = (win.outer_position().ok()?, win.outer_size().ok()?);
    Some((pos.x as f64 + size.width as f64 / 2.0, pos.y as f64 + size.height as f64))
}

/// 사용자가 창을 끌어 옮겼다: 아래 가운데 점을 기억하고, 멈추면 그때의 자리를 저장한다.
/// 끄는 중이 아닌 이동(모니터를 빼서 OS가 옮겼다 등)은 기억하지 않는다. 지켜보기(`watch`)가 제자리로 되돌린다
fn remember_moved(app: &AppHandle) {
    if PLACING.load(Ordering::SeqCst) || !dragging(now_ms(), DRAG_UNTIL.load(Ordering::SeqCst)) {
        return;
    }
    let Some(anchor) = app.get_webview_window(OVERLAY).as_ref().and_then(anchor_of) else {
        return;
    };
    {
        let mut placed = PLACED.lock().unwrap();
        // 셸이 놓은 자리 그대로면(보이기·크기 맞춤) 저장하지 않는다. 아직 놓기 전이면 기억할 것도 없다
        match *placed {
            None => return,
            Some((x, y)) if (x - anchor.0).abs() <= 1.0 && (y - anchor.1).abs() <= 1.0 => return,
            _ => {}
        }
        *placed = Some(anchor);
        *ANCHOR.lock().unwrap() = Some(anchor);
    }
    DRAG_UNTIL.fetch_max(now_ms() + DRAG_MOVE_MS, Ordering::SeqCst);
    let generation = SAVE_GEN.fetch_add(1, Ordering::SeqCst) + 1;
    let app = app.clone();
    tauri::async_runtime::spawn(async move {
        tokio::time::sleep(std::time::Duration::from_millis(SAVE_DEBOUNCE_MS)).await;
        if SAVE_GEN.load(Ordering::SeqCst) != generation {
            return;
        }
        // 끌기가 멈춘 뒤의 자리 (그 사이 크기 맞춤이 있었어도 아래 가운데 점은 같다)
        let Some(anchor) = app.get_webview_window(OVERLAY).as_ref().and_then(anchor_of) else {
            return;
        };
        *ANCHOR.lock().unwrap() = Some(anchor);
        *PLACED.lock().unwrap() = Some(anchor);
        let state = app.state::<AppState>();
        let body = patch(&state.settings(), json!({ "x": anchor.0.round() as i64, "y": anchor.1.round() as i64 }));
        crate::native::save_settings(&state, &body).await;
    });
}

/// 설정의 켜기 스위치를 바꾸고 창을 맞춘다
pub async fn set_enabled(app: &AppHandle, enabled: bool) -> bool {
    let state = app.state::<AppState>();
    let ok = crate::native::save_settings(&state, &patch(&state.settings(), json!({ "enabled": enabled }))).await;
    sync(app).await;
    ok
}

/// 설정 화면이 보일 것: 켜기 스위치, 이 창을 쓸 익스텐션이 있는지, 이 데스크톱에서 띄울 수 있는지
pub async fn state(app: &AppHandle) -> Value {
    let state = app.state::<AppState>();
    let enabled = saved(&state.settings()).enabled;
    let wanted = local_extensions(&state).await.is_some_and(|list| wanted_by(&list));
    json!({ "enabled": enabled, "available": wanted, "supported": supported() })
}

/// 오버레이 창이 보낼 수 있는 데몬 요청: 읽기(GET)와 서버 시작·정지·재시작뿐이다.
/// 익스텐션 번들이 이 창에서 돌아도 설정·서버 구성을 바꾸지 못하게 한다
pub fn allows_request(method: &reqwest::Method, path: &str) -> bool {
    if *method == reqwest::Method::GET {
        return true;
    }
    if *method != reqwest::Method::POST {
        return false;
    }
    let route = path.split('?').next().unwrap_or("");
    let Some(rest) = route.strip_prefix("/api/instance/") else {
        return false;
    };
    match rest.split_once('/') {
        Some((id, action)) => is_instance_id(id) && matches!(action, "start" | "stop" | "restart"),
        None => false,
    }
}

/// 인스턴스 id: 영문·숫자·`-`·`_` 128자까지
fn is_instance_id(id: &str) -> bool {
    !id.is_empty() && id.len() <= 128 && id.chars().all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_')
}

/// 펫이 고른 화면 `server:<인스턴스 id>`의 id (그 밖의 화면은 앱만 연다)
fn server_of(route: Option<&str>) -> Option<String> {
    let id = route?.strip_prefix("server:")?;
    is_instance_id(id).then(|| id.to_string())
}

/// 펫이 고른 앱 탭: `servers`·`discord`·`activity`·`settings`, 익스텐션 탭 `ext:<id>`
fn tab_of(route: Option<&str>) -> Option<String> {
    let route = route?;
    let ok = match route.strip_prefix("ext:") {
        Some(id) => is_instance_id(id),
        None => matches!(route, "servers" | "discord" | "activity" | "settings"),
    };
    ok.then(|| route.to_string())
}

/// 오버레이 창과 우클릭 판의 셸 동작: open_app(메인 창. `route`가 `server:<id>`면 그 서버를, 앱 탭이면 그 탭을 연다) |
/// hide(설정에서 끄고 창을 내린다) | close(판만 닫는다). 판은 늘 닫는다
pub async fn action(app: &AppHandle, action: &str, route: Option<&str>) -> bool {
    close_menu(app);
    match action {
        "close" => {}
        "open_app" => {
            crate::window::show_main(app);
            if let Some(id) = server_of(route) {
                crate::lifecycle::open_server(app, id);
            } else if let Some(tab) = tab_of(route) {
                crate::emit_main(app, "app:openRoute", json!({ "route": tab }));
            }
        }
        "hide" => {
            set_enabled(app, false).await;
        }
        _ => return false,
    }
    true
}

// ── 우클릭 판 ──

/// 펫을 우클릭했다: 판이 있으면 닫고, 없으면 펫 옆에 연다 (지금 펫 크기를 주소로 준다)
pub fn toggle_menu(app: &AppHandle) -> bool {
    let Some(win) = app.get_webview_window(OVERLAY) else {
        return false;
    };
    let pet = win.outer_position().ok().zip(win.outer_size().ok()).map(|(p, s)| PxRect {
        x: p.x as f64,
        y: p.y as f64,
        w: s.width as f64,
        h: s.height as f64,
    });
    let query = format!("&scale={}", SCALE.load(Ordering::SeqCst));
    crate::window::toggle_popup(app, &MENU, pet, &query)
}

pub fn fit_menu(app: &AppHandle, height: f64) {
    crate::window::fit_popup(app, &MENU, height);
}

fn close_menu(app: &AppHandle) {
    if let Some(w) = app.get_webview_window(OVERLAY_MENU) {
        let _ = w.destroy();
    }
}

/// 크기 막대를 움직였다: 펫에 바로 알리고(`overlay:scale`), 멈추면 저장한다. 고른 크기(%)를 돌려준다
pub fn set_scale(app: &AppHandle, value: f64) -> u32 {
    let scale = scale_of(value);
    SCALE.store(scale, Ordering::SeqCst);
    emit(app, "overlay:scale", scale);
    let generation = SCALE_GEN.fetch_add(1, Ordering::SeqCst) + 1;
    let app = app.clone();
    tauri::async_runtime::spawn(async move {
        tokio::time::sleep(std::time::Duration::from_millis(SAVE_DEBOUNCE_MS)).await;
        if SCALE_GEN.load(Ordering::SeqCst) != generation {
            return;
        }
        let state = app.state::<AppState>();
        crate::native::save_settings(&state, &patch(&state.settings(), json!({ "scale": scale }))).await;
    });
    scale
}

#[cfg(test)]
mod tests {
    use super::*;

    /// 펫이 고른 앱 화면(탭): 정해진 탭과 `ext:<id>`만
    #[test]
    fn only_known_tabs_are_opened() {
        for tab in ["servers", "discord", "activity", "settings", "ext:desktop_pet"] {
            assert_eq!(tab_of(Some(tab)).as_deref(), Some(tab));
        }
        for bad in ["server:abc", "ext:", "ext:../x", "admin", ""] {
            assert_eq!(tab_of(Some(bad)), None, "{bad}");
        }
        assert_eq!(tab_of(None), None);
    }

    /// 오버레이 창은 읽기와 서버 켜고 끄기만 한다
    #[test]
    fn the_overlay_only_reads_and_starts_or_stops_servers() {
        use reqwest::Method;
        assert!(allows_request(&Method::GET, "/api/servers?notices_after=0"));
        assert!(allows_request(&Method::GET, "/api/extensions/desktop_pet/gui"));
        for action in ["start", "stop", "restart"] {
            assert!(allows_request(&Method::POST, &format!("/api/instance/abc-1/{action}")));
        }
        assert!(!allows_request(&Method::POST, "/api/instance/abc-1/rename"));
        assert!(!allows_request(&Method::POST, "/api/instance/abc-1/start/../rename"));
        assert!(!allows_request(&Method::POST, "/api/instance//start"));
        assert!(!allows_request(&Method::POST, "/api/instance/../start"));
        assert!(!allows_request(&Method::PUT, "/api/config/gui"));
        assert!(!allows_request(&Method::DELETE, "/api/instance/abc-1"));
        assert!(!allows_request(&Method::POST, "/api/extensions/desktop_pet/disable"));
    }

    /// slot이 비면 창을 숨긴다 (48×48 최소 크기로 띄우지 않는다)
    #[test]
    fn an_empty_slot_is_not_shown() {
        assert!(!has_content(0.0, 0.0));
        assert!(!has_content(120.0, 0.0));
        assert!(!has_content(f64::NAN, 40.0));
        assert!(has_content(120.0, 140.0));
    }

    /// 펫이 고른 서버: `server:<인스턴스 id>`만, id는 영문·숫자·`-`·`_` 128자까지
    #[test]
    fn only_a_plain_server_route_opens_a_server() {
        assert_eq!(server_of(Some("server:inst-1_a")), Some("inst-1_a".to_string()));
        for bad in [None, Some(""), Some("server:"), Some("settings"), Some("server:a/b"), Some("server:a b")] {
            assert_eq!(server_of(bad), None, "{bad:?}");
        }
        assert_eq!(server_of(Some(&format!("server:{}", "a".repeat(129)))), None);
    }

    const WORK: PxRect = PxRect {
        x: 0.0,
        y: 0.0,
        w: 1920.0,
        h: 1040.0,
    };

    #[test]
    fn the_overlay_opens_only_for_an_enabled_extension_that_declares_its_slot_and_may_load() {
        let ext = |enabled: bool, allowed: Option<bool>, slots: Value| {
            let mut e = json!({ "id": "pet", "enabled": enabled, "gui": { "bundle": "gui/dist/pet.umd.js", "slots": slots } });
            if let Some(a) = allowed {
                e["gui_allowed"] = json!(a);
            }
            json!({ "extensions": [e] })
        };
        assert!(wanted_by(&ext(true, None, json!({ "Desktop.overlay": {} }))));
        assert!(wanted_by(&ext(true, Some(true), json!({ "Desktop.overlay": {} }))));
        assert!(!wanted_by(&ext(false, Some(true), json!({ "Desktop.overlay": {} }))), "disabled");
        assert!(!wanted_by(&ext(true, Some(false), json!({ "Desktop.overlay": {} }))), "bundle not allowed on this PC");
        assert!(!wanted_by(&ext(true, None, json!({ "App.page": {} }))), "another slot");
        assert!(!wanted_by(&json!({ "extensions": [{ "enabled": true, "gui": { "bundle": null, "slots": { "Desktop.overlay": {} } } }] })));
        assert!(!wanted_by(&json!({ "error": "x" })));
    }

    #[test]
    fn the_overlay_starts_bottom_right_and_keeps_its_bottom_centre_inside_the_work_area() {
        // 처음: 오른쪽 아래에서 margin만큼
        assert_eq!(place(None, (200.0, 160.0), WORK, 16.0), (1704.0, 864.0));
        // 기억한 아래 가운데 점: 크기가 바뀌어도 그 점을 지킨다
        assert_eq!(place(Some((500.0, 900.0)), (200.0, 160.0), WORK, 16.0), (400.0, 740.0));
        assert_eq!(place(Some((500.0, 900.0)), (200.0, 260.0), WORK, 16.0), (400.0, 640.0));
        // 작업 영역 밖으로 나가지 않는다
        assert_eq!(place(Some((10.0, 5000.0)), (200.0, 160.0), WORK, 16.0), (0.0, 880.0));
    }

    /// 말풍선으로 넓어져 가장자리에서 당겨져도, 좁아지면 원래 자리로 돌아온다 (당긴 자리를 기억하지 않는다)
    #[test]
    fn widening_at_the_edge_does_not_move_the_remembered_place() {
        let mut anchor = None;
        let mut xs = vec![];
        for w in [144.0, 300.0, 144.0] {
            let (pos, intended) = settle(anchor, (w, 200.0), WORK, 16.0);
            anchor = Some(intended);
            xs.push(pos.0);
        }
        assert_eq!(xs[0], xs[2], "{xs:?}");
        assert_eq!(xs[1], 1920.0 - 300.0, "pulled inside while wide");
    }

    /// 듀얼 모니터: 고른 자리가 있는 모니터에 놓는다. 그 모니터를 빼면 주 모니터의 처음 자리로 가되,
    /// 고른 자리는 잊지 않아서 다시 꽂으면 돌아간다
    #[test]
    fn a_place_on_an_unplugged_monitor_comes_back_when_it_is_plugged_in() {
        let right = PxRect {
            x: 1920.0,
            y: 0.0,
            w: 2560.0,
            h: 1400.0,
        };
        let chosen = Some((3000.0, 1300.0));
        assert_eq!(screen_for(chosen, &[WORK, right]), Some(1), "on the second monitor");
        assert_eq!(screen_for(chosen, &[WORK]), None, "unplugged: the primary monitor's first place");
        assert_eq!(screen_for(chosen, &[WORK, right]), Some(1), "plugged back in");
        assert_eq!(screen_for(Some((500.0, 900.0)), &[right, WORK]), Some(1));
        assert_eq!(screen_for(None, &[WORK, right]), None, "never moved");
    }

    /// 펫을 화면 아래 가장자리 밖으로 걸쳐 놓아도 그 모니터에 둔다 (`settle`이 안으로 당긴다)
    #[test]
    fn a_pet_dragged_past_the_screen_edge_stays_on_that_monitor() {
        let below = Some((900.0, WORK.y + WORK.h + 120.0));
        assert_eq!(screen_for(below, &[WORK]), Some(0));
        let left = Some((WORK.x - 80.0, 500.0));
        assert_eq!(screen_for(left, &[WORK]), Some(0));
    }

    /// 사용자가 끈 동안의 이동만 기억한다. 모니터를 빼서 OS가 옮긴 것은 기억하지 않는다
    #[test]
    fn only_moves_while_the_user_drags_are_remembered() {
        assert!(!dragging(1_000, 0), "no drag yet");
        assert!(dragging(1_000, 1_500));
        assert!(!dragging(2_000, 1_500), "the drag is over");
    }

    #[test]
    fn the_overlay_switch_defaults_on_and_reads_its_place() {
        assert_eq!(saved(&json!({})), Saved { enabled: true, at: None, scale: 100 });
        assert_eq!(
            saved(&json!({ "desktopOverlay": { "enabled": false, "x": 100, "y": 200, "scale": 150 } })),
            Saved { enabled: false, at: Some((100.0, 200.0)), scale: 150 }
        );
    }

    #[test]
    fn only_wayland_sessions_skip_the_overlay() {
        assert_eq!(wayland(Some("wayland"), false), cfg!(target_os = "linux"));
        assert_eq!(wayland(None, true), cfg!(target_os = "linux"));
        assert!(!wayland(Some("x11"), false));
        assert!(!wayland(None, false));
    }

    /// 자리를 저장해도 켜기·펫 크기는 남는다 (GUI 설정 저장은 `desktopOverlay`를 통째로 바꾼다)
    #[test]
    fn saving_one_overlay_setting_keeps_the_others() {
        let settings = json!({ "desktopOverlay": { "enabled": true, "x": 1, "y": 2, "scale": 150 } });
        assert_eq!(
            patch(&settings, json!({ "x": 5, "y": 6 })),
            json!({ "desktopOverlay": { "enabled": true, "x": 5, "y": 6, "scale": 150 } })
        );
        assert_eq!(patch(&json!({}), json!({ "enabled": false })), json!({ "desktopOverlay": { "enabled": false } }));
    }

    /// 펫 크기는 10% 칸으로 50~200%
    #[test]
    fn the_pet_size_snaps_to_10_percent_steps_between_50_and_200() {
        assert_eq!(scale_of(100.0), 100);
        assert_eq!(scale_of(173.0), 170);
        assert_eq!(scale_of(10.0), 50);
        assert_eq!(scale_of(900.0), 200);
        assert_eq!(scale_of(f64::NAN), 100);
    }

    /// 펫을 키우면 창 한도도 그만큼 커진다 (말풍선이 잘리지 않게)
    #[test]
    fn the_window_limit_grows_with_the_pet() {
        assert_eq!(window_size(1000.0, 1000.0, 100, 1.0), (MAX_WIDTH, MAX_HEIGHT));
        assert_eq!(window_size(1000.0, 1000.0, 200, 1.5), (MAX_WIDTH * 3.0, MAX_HEIGHT * 3.0));
        assert_eq!(window_size(10.0, 10.0, 50, 1.0), (MIN_SIDE, MIN_SIDE));
    }

    /// 펫과 그 우클릭 판만 데몬 요청이 묶인다
    #[test]
    fn the_pet_and_its_menu_only_read_and_run_servers() {
        assert!(limited(OVERLAY));
        assert!(limited(OVERLAY_MENU));
        assert!(!limited("main"));
        assert!(!limited(crate::window::TRAY_POPUP));
    }

    fn granted_to(window: &str) -> Vec<String> {
        let dir = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("capabilities");
        let mut granted: Vec<String> = Vec::new();
        for entry in std::fs::read_dir(dir).unwrap() {
            let cap: Value = serde_json::from_str(&std::fs::read_to_string(entry.unwrap().path()).unwrap()).unwrap();
            if cap["windows"].as_array().is_some_and(|w| w.iter().any(|w| w == window)) {
                granted.extend(cap["permissions"].as_array().unwrap().iter().filter_map(|p| p.as_str().map(String::from)));
            }
        }
        granted
    }

    /// 우클릭 판은 서버 켜고 끄기, 펫 크기, 앱 열기·숨기기만 한다
    #[test]
    fn the_pet_menu_may_change_only_the_pet_size_and_run_servers() {
        let granted = granted_to(OVERLAY_MENU);
        for need in [
            "allow-daemon-request",
            "allow-daemon-status",
            "allow-daemon-target-get",
            "allow-language-get",
            "allow-overlay-action",
            "allow-overlay-menu-fit",
            "allow-overlay-set-scale",
        ] {
            assert!(granted.iter().any(|g| g == need), "the pet menu cannot call {need}");
        }
        for never in ["core:event:allow-emit", "allow-notify", "allow-overlay-set-enabled", "allow-daemon-target-set", "allow-settings-local-save"] {
            assert!(!granted.iter().any(|g| g == never), "the pet menu must not call {never}");
        }
    }

    #[test]
    fn the_overlay_may_call_what_it_needs_and_nothing_that_notifies() {
        let granted = granted_to(OVERLAY);
        for need in [
            "allow-daemon-request",
            "allow-daemon-status",
            "allow-daemon-target-get",
            "allow-language-get",
            "allow-overlay-fit",
            "allow-overlay-menu",
            "allow-overlay-action",
            "core:event:allow-listen",
            "core:event:allow-unlisten",
            "allow-overlay-drag",
        ] {
            assert!(granted.iter().any(|g| g == need), "the overlay cannot call {need}");
        }
        for never in ["core:event:default", "core:event:allow-emit", "core:event:allow-emit-to", "allow-notify", "allow-overlay-set-enabled", "allow-daemon-target-set", "allow-settings-local-save", "core:window:allow-start-dragging", "allow-overlay-set-scale"] {
            assert!(!granted.iter().any(|g| g == never), "the overlay must not call {never}");
        }
        let build = include_str!("../build.rs");
        for cmd in ["overlay_fit", "overlay_menu", "overlay_action", "overlay_state", "overlay_set_enabled", "overlay_sync", "overlay_drag", "overlay_menu_fit", "overlay_set_scale"] {
            assert!(build.contains(&format!("\"{cmd}\"")), "{cmd} is missing from the app manifest");
        }
    }
}
