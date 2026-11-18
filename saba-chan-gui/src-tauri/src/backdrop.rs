//! 창 바탕 반투명 (설정 → 외관): 투명 창으로 만들지 정하고, OS 효과를 깔거나 걷는다.
//!
//! 설정값·OS별로 쓸 수 있는 효과는 `saba_chan_updater_lib::window_backdrop`(인스톨러와 같은 코드)이 정한다.
//! 투명 창은 만들 때만 정할 수 있다. Windows·macOS는 효과를 쓸 수 있으면 늘 투명하게 만들고 끄면 바탕을 칠하며,
//! Linux는 켜져 있을 때만 투명하게 만든다(합성기가 바뀌면 투명한 곳이 검게 나올 수 있다)

use crate::AppState;
use saba_chan_updater_lib::window_backdrop::{self as wb, Support, WindowBackdrop};
use serde_json::{json, Value};
use std::collections::{HashMap, HashSet};
use std::sync::{Mutex, OnceLock};
use tauri::window::Color;
use tauri::{AppHandle, Manager, WebviewWindow, WebviewWindowBuilder};

/// 렌더러가 마지막으로 알린 설정 (설정 파일보다 앞선다: 저장이 늦어도 새 창이 같은 값으로 뜬다)
static CURRENT: Mutex<Option<WindowBackdrop>> = Mutex::new(None);
/// 투명하게 만든 창 라벨
static TRANSPARENT: Mutex<Option<HashSet<String>>> = Mutex::new(None);
/// OS에 건 효과 이름 (None = 걷음)
type Effects = Option<Vec<&'static str>>;
/// 창마다 마지막으로 OS에 건 효과. 같은 효과를 다시 걸면 Windows가 재질을 새로 그려 깜빡인다
static APPLIED: Mutex<Option<HashMap<String, Effects>>> = Mutex::new(None);

/// 이 컴퓨터에서 쓸 수 있는 것. 처음 부르는 곳은 메인 스레드다 (창 만들기·동기 명령)
pub fn support() -> &'static Support {
    static SUPPORT: OnceLock<Support> = OnceLock::new();
    SUPPORT.get_or_init(wb::support_now)
}

pub fn current(app: &AppHandle) -> WindowBackdrop {
    if let Some(v) = CURRENT.lock().unwrap().clone() {
        return v;
    }
    WindowBackdrop::from_gui(&app.state::<AppState>().settings())
}

/// 이 창에 건 효과를 기록한다. 처음이거나 바뀌었을 때만 true (그때만 OS에 건다)
fn remember_effects(label: &str, effects: Effects) -> bool {
    let mut applied = APPLIED.lock().unwrap();
    let applied = applied.get_or_insert_with(HashMap::new);
    if applied.get(label) == Some(&effects) {
        return false;
    }
    applied.insert(label.to_string(), effects);
    true
}

/// 새로 만드는 창은 효과가 없다
fn forget_effects(label: &str) {
    if let Some(applied) = APPLIED.lock().unwrap().as_mut() {
        applied.remove(label);
    }
}

fn is_transparent(label: &str) -> bool {
    TRANSPARENT.lock().unwrap().as_ref().is_some_and(|s| s.contains(label))
}

/// 새 창을 투명하게 만들지: 효과를 쓸 수 있고, 켜고 끄기가 바로 되거나(Windows·macOS) 지금 켜져 있을 때
fn wants_transparent(app: &AppHandle) -> bool {
    let s = support();
    s.available && (s.live_toggle || current(app).enabled)
}

/// 창을 만들기 전에: 투명하게 만들 창이면 표시해 두고 투명 창으로
pub fn prepare<'a>(
    app: &AppHandle,
    label: &str,
    builder: WebviewWindowBuilder<'a, tauri::Wry, AppHandle>,
) -> WebviewWindowBuilder<'a, tauri::Wry, AppHandle> {
    let transparent = wants_transparent(app);
    forget_effects(label);
    let mut set = TRANSPARENT.lock().unwrap();
    let set = set.get_or_insert_with(HashSet::new);
    if transparent {
        set.insert(label.to_string());
    } else {
        set.remove(label);
    }
    builder.transparent(transparent)
}

/// 창에 효과를 깔거나 걷고 바탕을 칠한다. 켜졌으면 true (렌더러는 그때만 바탕을 비운다)
pub fn apply(win: &WebviewWindow, config: &WindowBackdrop, theme_bg: Color) -> bool {
    let s = support();
    let transparent = is_transparent(win.label());
    let on = transparent && s.available && config.enabled;
    if !transparent {
        let _ = win.set_background_color(Some(theme_bg));
        return false;
    }
    // 웹뷰는 늘 비우고(화면이 CSS로 칠한다), 창 바닥은 켜면 비우고 끄면 테마색으로 (크기를 바꾸는 중 바탕화면이 비치지 않게).
    // Windows 창 바닥은 알파를 무시하므로 비울 때는 None이다
    let _ = win
        .as_ref()
        .window()
        .set_background_color(if on { None } else { Some(theme_bg) });
    let _ = win.as_ref().set_background_color(Some(Color(0, 0, 0, 0)));
    // 진하기·밝기는 화면(CSS)만 바꾼다: 켜고 끄기나 재질이 바뀔 때만 OS에 건다
    if remember_effects(win.label(), on.then(|| wb::effect_names(s, config))) {
        set_effects(win, s, config, on);
    }
    on
}

#[cfg(any(windows, target_os = "macos"))]
fn set_effects(win: &WebviewWindow, s: &Support, config: &WindowBackdrop, on: bool) {
    use tauri::window::{Effect, EffectsBuilder};
    if !on {
        let _ = win.set_effects(None);
        return;
    }
    let effects: Vec<Effect> = wb::effect_names(s, config)
        .into_iter()
        .filter_map(|n| serde_json::from_value(json!(n)).ok())
        .collect();
    let _ = win.set_effects(EffectsBuilder::new().effects(effects).build());
}

#[cfg(target_os = "linux")]
fn set_effects(win: &WebviewWindow, s: &Support, _config: &WindowBackdrop, on: bool) {
    use gtk::prelude::WidgetExt;
    if s.blur != wb::Blur::System {
        return;
    }
    if let Some(gdk_window) = win.gtk_window().ok().and_then(|w| w.window()) {
        wb::set_kwin_blur(&gdk_window, on);
    }
}

#[cfg(not(any(windows, target_os = "macos", target_os = "linux")))]
fn set_effects(_: &WebviewWindow, _: &Support, _: &WindowBackdrop, _: bool) {}

/// 렌더러가 이 창의 설정을 알린다 (창이 뜰 때·설정을 바꿀 때). 쓸 수 있는 것과 켜졌는지를 돌려준다
pub fn set_from_renderer(win: &WebviewWindow, config: Value, theme_bg: Color) -> Value {
    let mut config: WindowBackdrop = serde_json::from_value(config).unwrap_or_default();
    config.sanitize();
    *CURRENT.lock().unwrap() = Some(config.clone());
    let active = apply(win, &config, theme_bg);
    info(win, active)
}

fn info(win: &WebviewWindow, active: bool) -> Value {
    let mut v = serde_json::to_value(support()).unwrap_or(Value::Null);
    if let Value::Object(m) = &mut v {
        m.insert("active".into(), json!(active));
        m.insert("transparent".into(), json!(is_transparent(win.label())));
    }
    v
}

#[cfg(test)]
mod tests {
    use super::*;

    /// 진하기·밝기 막대를 끌거나 테마 바탕색을 알릴 때마다 같은 재질을 다시 걸면 Windows가 재질을 새로 그려 깜빡인다.
    /// 켜고 끄기나 재질이 바뀔 때만 OS에 건다
    #[test]
    fn effects_are_set_only_when_they_change() {
        let label = "test-effects-change";
        let tabbed = Some(vec!["tabbed"]);
        assert!(remember_effects(label, tabbed.clone()), "first apply sets the effect");
        assert!(!remember_effects(label, tabbed.clone()), "same effect again is skipped");
        assert!(remember_effects(label, Some(vec!["mica"])), "material change applies");
        assert!(remember_effects(label, None), "turning off clears");
        assert!(!remember_effects(label, None));
        assert!(remember_effects(label, tabbed.clone()), "turning on again applies");

        // 창을 새로 만들면 효과가 없는 창이다: 같은 값이라도 다시 건다
        forget_effects(label);
        assert!(remember_effects(label, tabbed));
    }
}
