//! 창 바탕 반투명: 앱 설정(`config.json` `gui.windowBackdrop`, GUI 설정 → 외관)을 따른다.
//! 설치·업데이트 중에는 설정을 바꾸지 않으므로 창을 만들 때 한 번 정한다. 고르는 값과 OS별 효과는
//! `saba_chan_updater_lib::window_backdrop`(GUI 셸과 같은 코드)이 정한다

use saba_chan_updater_lib::config_file::ConfigFile;
use saba_chan_updater_lib::window_backdrop::{self as wb, Support, WindowBackdrop};
use serde_json::{json, Value};
use std::sync::OnceLock;
use tauri::window::Color;

struct Backdrop {
    on: bool,
    config: WindowBackdrop,
    support: Support,
}

/// 처음 부르는 곳은 창을 만드는 메인 스레드다 (Linux는 GTK에 묻는다)
fn backdrop() -> &'static Backdrop {
    static B: OnceLock<Backdrop> = OnceLock::new();
    B.get_or_init(|| {
        let config = WindowBackdrop::from_gui(&ConfigFile::default_location().section("gui"));
        let support = wb::support_now();
        Backdrop {
            on: support.available && config.enabled,
            config,
            support,
        }
    })
}

/// 투명 창으로 만들지
pub fn on() -> bool {
    backdrop().on
}

/// 만든 창의 바닥을 비우고 OS 효과를 깐다
pub fn apply(win: &tauri::WebviewWindow) {
    let b = backdrop();
    if !b.on {
        return;
    }
    // Windows 창 바닥은 알파를 무시하므로 비울 때는 None, 웹뷰는 알파 0
    let _ = win.as_ref().window().set_background_color(None);
    let _ = win.as_ref().set_background_color(Some(Color(0, 0, 0, 0)));
    #[cfg(any(windows, target_os = "macos"))]
    {
        use tauri::window::{Effect, EffectsBuilder};
        let effects: Vec<Effect> = wb::effect_names(&b.support, &b.config)
            .into_iter()
            .filter_map(|n| serde_json::from_value(json!(n)).ok())
            .collect();
        let _ = win.set_effects(EffectsBuilder::new().effects(effects).build());
    }
    #[cfg(target_os = "linux")]
    {
        use gtk::prelude::WidgetExt;
        if b.support.blur == wb::Blur::System
            && let Some(gdk_window) = win.gtk_window().ok().and_then(|w| w.window())
        {
            wb::set_kwin_blur(&gdk_window, true);
        }
    }
}

/// 화면이 바탕을 비울지와 덮을 진하기·밝기 (common.js)
#[tauri::command]
pub fn get_window_backdrop() -> Value {
    let b = backdrop();
    json!({
        "on": b.on,
        "opacity": b.support.opacity(b.config.opacity),
        "brightness": b.config.brightness,
    })
}
