//! 바탕화면 오버레이 창 — 익스텐션 slot `Desktop.overlay`를 그리는 작은 창 (데스크톱 펫 등)
//!
//! 셸은 창만 준다: 투명·틀 없음·항상 위·작업 표시줄에 없음. 무엇을 그릴지는 그 slot을 선언한 익스텐션이 정한다.
//! - 켜진 익스텐션 중 하나라도 `Desktop.overlay`를 선언했고(번들을 내줄 수 있고) 설정 `desktopOverlay.enabled`가 켜져 있을 때만 띄운다
//! - 창은 내용 크기에 맞추고(`overlay_fit`) 아래 가운데 점을 붙박이로 둔다. 끌어 옮긴 자리는 `desktopOverlay.{x,y}`(물리 px)에 남는다
//! - 우클릭 메뉴는 렌더러가 항목을 주면 셸이 OS 메뉴로 띄우고 고른 것을 `overlay:menu`로 돌려준다 (작은 창 안에 그리면 잘린다)
//! - 사용자가 보는 창으로 세지 않는다: 트레이만 남은 상태 판단(`window::tray_only`)과 "창 모두 닫기"에서 빠진다
//!
//! 리눅스 Wayland는 창 자리와 항상 위를 앱이 정할 수 없어 띄우지 않는다.

use crate::window::PxRect;
use crate::AppState;
use serde::Deserialize;
use serde_json::{json, Value};
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::Mutex;
use tauri::menu::{IsMenuItem, Menu, MenuItem, PredefinedMenuItem, Submenu};
use tauri::{AppHandle, Emitter, Manager, WebviewUrl, WebviewWindow, WebviewWindowBuilder, WindowEvent};

pub const OVERLAY: &str = "desktop-overlay";
/// 익스텐션 manifest `gui.slots`의 이 창 slot 이름 (`extensions/slotTable.ts`와 같다)
pub const OVERLAY_SLOT: &str = "Desktop.overlay";
/// OS 메뉴 항목 id 앞머리. 트레이 메뉴(리눅스)와 같은 전역 메뉴 이벤트로 오므로 섞이지 않게 붙인다
const MENU_PREFIX: &str = "overlay-menu:";
/// 내용 크기를 받기 전 처음 크기 (보이지 않는다)
const INITIAL_SIZE: (f64, f64) = (200.0, 200.0);
/// 창 크기 한도 (논리 px)
const MIN_SIDE: f64 = 48.0;
const MAX_WIDTH: f64 = 360.0;
const MAX_HEIGHT: f64 = 420.0;
/// 처음 자리: 작업 영역 오른쪽 아래에서 띄우는 간격 (논리 px)
const MARGIN: f64 = 16.0;
const SAVE_DEBOUNCE_MS: u64 = 600;
/// 메뉴 항목 한도 (렌더러가 보낸 것)
const MENU_MAX_ITEMS: usize = 80;
const MENU_MAX_LABEL: usize = 80;

/// 지킬 아래 가운데 점 (물리 px). 크기가 바뀌어도 이 점을 지킨다 (가장자리에서 당겨도 바꾸지 않는다)
static ANCHOR: Mutex<Option<(f64, f64)>> = Mutex::new(None);
/// 셸이 마지막으로 놓은 창의 아래 가운데 점: 이와 다른 자리로 옮겨졌으면 사용자가 끈 것이다
static PLACED: Mutex<Option<(f64, f64)>> = Mutex::new(None);
/// 셸이 창을 옮기고 크기를 바꾸는 중: 그 사이의 이동 알림은 사용자가 끈 것이 아니다
static PLACING: AtomicBool = AtomicBool::new(false);
static SAVE_GEN: AtomicU64 = AtomicU64::new(0);
/// 마지막으로 연 메뉴의 세대. 늦게 온 옛 메뉴의 선택은 버린다
static MENU_GEN: AtomicU64 = AtomicU64::new(0);

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

/// 설정 `desktopOverlay` (없으면 켬·자리 없음)
fn saved(settings: &Value) -> (bool, Option<(f64, f64)>) {
    let o = &settings["desktopOverlay"];
    let enabled = o["enabled"].as_bool().unwrap_or(true);
    let at = o["x"].as_f64().zip(o["y"].as_f64());
    (enabled, at)
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
    let (enabled, _) = saved(&state.settings());
    if !enabled || !supported() {
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
    #[cfg(target_os = "macos")]
    mac_panel::release(app);
    if let Some(w) = app.get_webview_window(OVERLAY) {
        let _ = w.destroy();
    }
}

/// 창을 만든다 (이미 있으면 그대로). 내용 크기를 알려 오면(`fit`) 자리를 잡고 보인다
fn open(app: &AppHandle) {
    if app.get_webview_window(OVERLAY).is_some() {
        return;
    }
    let builder = WebviewWindowBuilder::new(app, OVERLAY, WebviewUrl::App("index.html?desktop-overlay=1".into()))
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
    #[cfg(target_os = "macos")]
    mac_panel::adopt(&win);
    *ANCHOR.lock().unwrap() = None;
    *PLACED.lock().unwrap() = None;
    let handle = app.clone();
    win.on_window_event(move |event| {
        // 셸이 놓은 자리(`fit`)도 Moved로 온다: 놓는 중이거나 기억한 점과 같으면 remember_moved가 거른다
        if let WindowEvent::Moved(_) = event {
            remember_moved(&handle);
        }
    });
    let handle = app.clone();
    win.on_menu_event(move |_, event| {
        if let Some(choice) = menu_choice(event.id().as_ref(), MENU_GEN.load(Ordering::SeqCst)) {
            let _ = handle.emit_to(OVERLAY, "overlay:menu", choice);
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
        let _ = win.hide();
        return;
    }
    let anchor = *ANCHOR.lock().unwrap();
    let anchor = anchor.or_else(|| saved(&app.state::<AppState>().settings()).1);
    let monitor = anchor
        .and_then(|(x, y)| app.monitor_from_point(x, y - 1.0).ok().flatten())
        .or_else(|| app.primary_monitor().ok().flatten());
    let Some(monitor) = monitor else {
        let _ = win.show();
        return;
    };
    let scale = monitor.scale_factor();
    // 물리 px는 정수로: 창이 옮겨진 뒤 잰 자리와 기억한 점이 어긋나 사용자가 옮긴 것으로 보지 않게
    let size = (
        (width.clamp(MIN_SIDE, MAX_WIDTH) * scale).round(),
        (height.clamp(MIN_SIDE, MAX_HEIGHT) * scale).round(),
    );
    let area = monitor.work_area();
    let work = PxRect {
        x: area.position.x as f64,
        y: area.position.y as f64,
        w: area.size.width as f64,
        h: area.size.height as f64,
    };
    // 다른 모니터에 남은 자리(모니터를 뺐다 등)는 버리고 처음 자리로
    let anchor = anchor.filter(|&(x, y)| x >= work.x && x <= work.x + work.w && y > work.y && y <= work.y + work.h);
    let ((x, y), intended) = settle(anchor, size, work, MARGIN * scale);
    *ANCHOR.lock().unwrap() = Some(intended);
    *PLACED.lock().unwrap() = Some((x + size.0 / 2.0, y + size.1));
    PLACING.store(true, Ordering::SeqCst);
    // 먼저 옮기고 크기를 준다: 배율이 다른 모니터로 옮기면 창 크기가 그 배율로 다시 잡힌다
    let _ = win.set_position(tauri::PhysicalPosition::new(x, y));
    let _ = win.set_size(tauri::PhysicalSize::new(size.0, size.1));
    PLACING.store(false, Ordering::SeqCst);
    let _ = win.show();
}

/// 창의 아래 가운데 점 (물리 px)
fn anchor_of(win: &WebviewWindow) -> Option<(f64, f64)> {
    let (pos, size) = (win.outer_position().ok()?, win.outer_size().ok()?);
    Some((pos.x as f64 + size.width as f64 / 2.0, pos.y as f64 + size.height as f64))
}

/// 사용자가 창을 끌어 옮겼다: 아래 가운데 점을 기억하고, 멈추면 그때의 자리를 저장한다
fn remember_moved(app: &AppHandle) {
    if PLACING.load(Ordering::SeqCst) {
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
        let (enabled, _) = saved(&state.settings());
        let patch = json!({ "desktopOverlay": { "enabled": enabled, "x": anchor.0.round() as i64, "y": anchor.1.round() as i64 } });
        crate::native::save_settings(&state, &patch).await;
    });
}

/// 설정의 켜기 스위치를 바꾸고 창을 맞춘다
pub async fn set_enabled(app: &AppHandle, enabled: bool) -> bool {
    let state = app.state::<AppState>();
    let (_, at) = saved(&state.settings());
    let mut overlay = json!({ "enabled": enabled });
    if let Some((x, y)) = at {
        overlay["x"] = json!(x as i64);
        overlay["y"] = json!(y as i64);
    }
    let ok = crate::native::save_settings(&state, &json!({ "desktopOverlay": overlay })).await;
    sync(app).await;
    ok
}

/// 설정 화면이 보일 것: 켜기 스위치, 이 창을 쓸 익스텐션이 있는지, 이 데스크톱에서 띄울 수 있는지
pub async fn state(app: &AppHandle) -> Value {
    let state = app.state::<AppState>();
    let (enabled, _) = saved(&state.settings());
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

/// 오버레이 창의 셸 동작: open_app(메인 창. `route`가 `server:<id>`면 그 서버를, 앱 탭이면 그 탭을 연다) |
/// hide(설정에서 끄고 창을 내린다)
pub async fn action(app: &AppHandle, action: &str, route: Option<&str>) -> bool {
    match action {
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

// ── 우클릭 메뉴 ──

/// 렌더러가 보낸 메뉴 한 칸: 구분선이거나, id가 있는 항목이거나, 하위 메뉴(`items`)
#[derive(Deserialize, Debug, Clone)]
#[serde(rename_all = "camelCase")]
pub struct MenuSpec {
    #[serde(default)]
    pub id: Option<String>,
    #[serde(default)]
    pub label: String,
    #[serde(default = "yes")]
    pub enabled: bool,
    #[serde(default)]
    pub separator: bool,
    #[serde(default)]
    pub items: Option<Vec<MenuSpec>>,
}

fn yes() -> bool {
    true
}

/// 메뉴가 받을 만한가: 항목 수·글자 수·깊이(하위 메뉴 두 단계: 서버 ▸ 정지 ▸ 확인) 한도
pub fn menu_ok(items: &[MenuSpec]) -> bool {
    fn walk(items: &[MenuSpec], depth: usize, count: &mut usize) -> bool {
        items.iter().all(|i| {
            *count += 1;
            *count <= MENU_MAX_ITEMS
                && i.label.chars().count() <= MENU_MAX_LABEL
                && i.id.as_ref().is_none_or(|id| id.len() <= 200)
                && match &i.items {
                    Some(sub) => depth < 3 && walk(sub, depth + 1, count),
                    None => true,
                }
        })
    }
    !items.is_empty() && walk(items, 1, &mut 0)
}

/// OS 메뉴 id: 앞머리 + 세대 + 렌더러 id
fn menu_id(generation: u64, id: &str) -> String {
    format!("{MENU_PREFIX}{generation}:{id}")
}

/// 받은 메뉴 이벤트가 지금 메뉴의 항목이면 렌더러 id
fn menu_choice(raw: &str, generation: u64) -> Option<String> {
    let rest = raw.strip_prefix(MENU_PREFIX)?;
    let (g, id) = rest.split_once(':')?;
    (g.parse::<u64>().ok()? == generation).then(|| id.to_string())
}

fn build_items(app: &AppHandle, items: &[MenuSpec], generation: u64) -> tauri::Result<Vec<Box<dyn IsMenuItem<tauri::Wry>>>> {
    let mut out: Vec<Box<dyn IsMenuItem<tauri::Wry>>> = Vec::new();
    for spec in items {
        if spec.separator {
            out.push(Box::new(PredefinedMenuItem::separator(app)?));
        } else if let Some(sub) = &spec.items {
            let children = build_items(app, sub, generation)?;
            let refs: Vec<&dyn IsMenuItem<tauri::Wry>> = children.iter().map(|b| b.as_ref()).collect();
            out.push(Box::new(Submenu::with_items(app, &spec.label, spec.enabled, &refs)?));
        } else {
            let id = spec.id.as_deref().unwrap_or_default();
            out.push(Box::new(MenuItem::with_id(
                app,
                menu_id(generation, id),
                &spec.label,
                spec.enabled && !id.is_empty(),
                None::<&str>,
            )?));
        }
    }
    Ok(out)
}

/// 렌더러가 준 항목으로 OS 메뉴를 마우스 자리에 띄운다. 고르면 `overlay:menu`(항목 id)가 이 창에 간다
pub fn popup_menu(app: &AppHandle, items: Vec<MenuSpec>) -> bool {
    let Some(win) = app.get_webview_window(OVERLAY) else {
        return false;
    };
    if !menu_ok(&items) {
        return false;
    }
    let generation = MENU_GEN.fetch_add(1, Ordering::SeqCst) + 1;
    let built = build_items(app, &items, generation).and_then(|children| {
        let refs: Vec<&dyn IsMenuItem<tauri::Wry>> = children.iter().map(|b| b.as_ref()).collect();
        Menu::with_items(app, &refs)
    });
    match built.and_then(|menu| win.popup_menu(&menu)) {
        Ok(()) => true,
        Err(e) => {
            tracing::warn!("[Overlay] Failed to show the menu: {}", e);
            false
        }
    }
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

    #[test]
    fn the_overlay_switch_defaults_on_and_reads_its_place() {
        assert_eq!(saved(&json!({})), (true, None));
        assert_eq!(
            saved(&json!({ "desktopOverlay": { "enabled": false, "x": 100, "y": 200 } })),
            (false, Some((100.0, 200.0)))
        );
    }

    #[test]
    fn only_wayland_sessions_skip_the_overlay() {
        assert_eq!(wayland(Some("wayland"), false), cfg!(target_os = "linux"));
        assert_eq!(wayland(None, true), cfg!(target_os = "linux"));
        assert!(!wayland(Some("x11"), false));
        assert!(!wayland(None, false));
    }

    #[test]
    fn menu_choices_from_an_older_menu_or_the_tray_are_ignored() {
        let id = menu_id(3, "srv:a1:stop");
        assert_eq!(menu_choice(&id, 3).as_deref(), Some("srv:a1:stop"));
        assert_eq!(menu_choice(&id, 4), None, "an older menu");
        assert_eq!(menu_choice("open_window", 3), None, "the Linux tray menu");
    }

    #[test]
    fn the_menu_is_bounded() {
        let item = |label: &str| MenuSpec {
            id: Some("x".into()),
            label: label.into(),
            enabled: true,
            separator: false,
            items: None,
        };
        let sub = |items: Vec<MenuSpec>| MenuSpec {
            id: None,
            label: "sub".into(),
            enabled: true,
            separator: false,
            items: Some(items),
        };
        assert!(menu_ok(&[item("열기"), sub(vec![item("정지")])]));
        assert!(!menu_ok(&[]));
        assert!(menu_ok(&[sub(vec![sub(vec![item("확인")])])]), "server ▸ stop ▸ confirm");
        assert!(!menu_ok(&[sub(vec![sub(vec![sub(vec![item("too deep")])])])]), "two levels of submenus");
        assert!(!menu_ok(&vec![item("x"); MENU_MAX_ITEMS + 1]));
        assert!(!menu_ok(&[item(&"가".repeat(MENU_MAX_LABEL + 1))]));
        let parsed: Vec<MenuSpec> =
            serde_json::from_value(json!([{ "id": "a", "label": "A" }, { "separator": true }, { "label": "B", "items": [{ "id": "b", "label": "C", "enabled": false }] }]))
                .unwrap();
        assert!(menu_ok(&parsed));
        assert!(parsed[0].enabled && !parsed[2].items.as_ref().unwrap()[0].enabled);
    }

    #[test]
    fn the_overlay_may_call_what_it_needs_and_nothing_that_notifies() {
        let dir = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("capabilities");
        let mut granted: Vec<String> = Vec::new();
        for entry in std::fs::read_dir(dir).unwrap() {
            let cap: Value = serde_json::from_str(&std::fs::read_to_string(entry.unwrap().path()).unwrap()).unwrap();
            if cap["windows"].as_array().is_some_and(|w| w.iter().any(|w| w == OVERLAY)) {
                granted.extend(cap["permissions"].as_array().unwrap().iter().filter_map(|p| p.as_str().map(String::from)));
            }
        }
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
            "core:window:allow-start-dragging",
        ] {
            assert!(granted.iter().any(|g| g == need), "the overlay cannot call {need}");
        }
        for never in ["core:event:default", "core:event:allow-emit", "core:event:allow-emit-to", "allow-notify", "allow-overlay-set-enabled", "allow-daemon-target-set", "allow-settings-local-save"] {
            assert!(!granted.iter().any(|g| g == never), "the overlay must not call {never}");
        }
        let build = include_str!("../build.rs");
        for cmd in ["overlay_fit", "overlay_menu", "overlay_action", "overlay_state", "overlay_set_enabled", "overlay_sync"] {
            assert!(build.contains(&format!("\"{cmd}\"")), "{cmd} is missing from the app manifest");
        }
    }
}
