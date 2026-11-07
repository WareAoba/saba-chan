//! 창 바탕 반투명 (GUI 설정 → 외관). 설정값(`config.json` `gui.windowBackdrop`)과 OS마다 쓸 수 있는 창 효과.
//!
//! 셸(GUI·인스톨러)은 창을 투명하게 만들고 OS 효과(Windows Mica·아크릴, macOS 리퀴드 글래스·vibrancy,
//! Linux KWin 흐림)를 창 뒤에 깐다. 화면은 그 위에 테마 바탕을 `opacity`만큼 한 겹 덮는다.
//! 데몬은 이 모양으로 설정을 검사하고, GUI 셸·인스톨러는 같은 값을 읽어 창을 만든다.

use serde::{Deserialize, Serialize};
use serde_json::Value;

/// Windows에서 고르는 재질 (Tauri `Effect` 이름과 같다)
pub const WINDOWS_CHOICES: [&str; 3] = ["tabbed", "mica", "acrylic"];
/// macOS 리퀴드 글래스 종류: 기본(regular) · 더 투명(clear)
pub const MAC_CHOICES: [&str; 2] = ["regular", "clear"];
pub const OPACITY_MAX: u32 = 100;
pub const BRIGHTNESS_LIMIT: i32 = 50;
/// 흐림 없이 그대로 비치는 환경(GNOME 등)에서 글씨를 지키는 최소 진하기
pub const PLAIN_MIN_OPACITY: u32 = 85;

/// Windows 빌드: 시스템 바탕(DWMWA_SYSTEMBACKDROP_TYPE)·Windows 11·아크릴(SetWindowCompositionAttribute) 최소판
const BUILD_SYSTEM_BACKDROP: u32 = 22523;
const BUILD_WINDOWS_11: u32 = 22000;
const BUILD_ACRYLIC: u32 = 17763;

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct WindowBackdrop {
    pub enabled: bool,
    /// Windows 재질: `WINDOWS_CHOICES`
    pub windows: String,
    /// macOS 유리 종류: `MAC_CHOICES`
    pub mac: String,
    /// 화면이 덮는 테마 바탕의 진하기 (0 = OS 효과만, 100 = 불투명)
    pub opacity: u32,
    /// 덮는 바탕을 흰색(+)·검은색(-) 쪽으로 섞는 정도 (%)
    pub brightness: i32,
}

impl Default for WindowBackdrop {
    fn default() -> Self {
        Self {
            enabled: true,
            windows: "tabbed".into(),
            mac: "regular".into(),
            opacity: 60,
            brightness: 0,
        }
    }
}

impl WindowBackdrop {
    /// 고를 수 없는 값을 알린다 (데몬이 저장 요청을 거절할 때)
    pub fn validate(&self) -> Result<(), String> {
        if !WINDOWS_CHOICES.contains(&self.windows.as_str()) {
            return Err(format!(
                "windowBackdrop.windows must be one of {}",
                WINDOWS_CHOICES.join(", ")
            ));
        }
        if !MAC_CHOICES.contains(&self.mac.as_str()) {
            return Err(format!("windowBackdrop.mac must be one of {}", MAC_CHOICES.join(", ")));
        }
        if self.opacity > OPACITY_MAX {
            return Err(format!("windowBackdrop.opacity must be 0..={OPACITY_MAX}"));
        }
        if self.brightness.abs() > BRIGHTNESS_LIMIT {
            return Err(format!(
                "windowBackdrop.brightness must be -{BRIGHTNESS_LIMIT}..={BRIGHTNESS_LIMIT}"
            ));
        }
        Ok(())
    }

    /// 손으로 고친 파일의 고를 수 없는 값은 기본값·한도로 둔다
    pub fn sanitize(&mut self) {
        let default = Self::default();
        if !WINDOWS_CHOICES.contains(&self.windows.as_str()) {
            self.windows = default.windows;
        }
        if !MAC_CHOICES.contains(&self.mac.as_str()) {
            self.mac = default.mac;
        }
        self.opacity = self.opacity.min(OPACITY_MAX);
        self.brightness = self.brightness.clamp(-BRIGHTNESS_LIMIT, BRIGHTNESS_LIMIT);
    }

    /// GUI 설정(`gui` 섹션)에서 읽는다. 없거나 깨진 값은 기본값
    pub fn from_gui(gui: &Value) -> Self {
        let mut v: Self = gui
            .get("windowBackdrop")
            .and_then(|v| serde_json::from_value(v.clone()).ok())
            .unwrap_or_default();
        v.sanitize();
        v
    }
}

/// OS 효과가 창 뒤를 어떻게 보이는가
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum Blur {
    /// OS가 흐리게 그린다 (Mica·아크릴·리퀴드 글래스·KWin)
    System,
    /// 창 관리자(Hyprland·picom 등) 설정에 달렸다
    Compositor,
    /// 흐림 없이 그대로 비친다 (GNOME 등)
    None,
}

/// 이 컴퓨터에서 쓸 수 있는 것 (설정 화면이 고를 거리와 안내를 정한다)
#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Support {
    /// "windows" | "macos" | "linux"
    pub platform: &'static str,
    pub available: bool,
    /// 못 쓰는 까닭: "windows-old" | "no-compositor"
    #[serde(skip_serializing_if = "Option::is_none")]
    pub reason: Option<&'static str>,
    /// 고를 수 있는 재질 (Windows: `WINDOWS_CHOICES` 일부, macOS: `MAC_CHOICES`, Linux: 없음)
    pub choices: Vec<&'static str>,
    /// 창을 끌거나 크기를 바꿀 때 느린 재질
    pub slow: Vec<&'static str>,
    pub blur: Blur,
    /// 켜고 끄기가 바로 되는가 (false면 창을 다시 열 때 적용: 투명 창은 만들 때만 정할 수 있다)
    pub live_toggle: bool,
    pub min_opacity: u32,
}

impl Support {
    fn unavailable(platform: &'static str, reason: &'static str) -> Self {
        Self {
            platform,
            available: false,
            reason: Some(reason),
            choices: vec![],
            slow: vec![],
            blur: Blur::None,
            live_toggle: true,
            min_opacity: 0,
        }
    }

    /// 고른 재질이 여기서 안 되면 고를 수 있는 첫 재질 (Windows 11 초기판의 탭 → Mica)
    pub fn pick<'a>(&self, choice: &'a str) -> Option<&'a str>
    where
        'static: 'a,
    {
        if self.choices.contains(&choice) {
            Some(choice)
        } else {
            self.choices.first().copied()
        }
    }

    /// 화면이 덮을 진하기 (흐림이 없으면 글씨를 지키는 최소값)
    pub fn opacity(&self, wanted: u32) -> u32 {
        wanted.min(OPACITY_MAX).max(self.min_opacity)
    }
}

/// Windows: 빌드 번호로 고를 수 있는 재질. 22523 미만의 아크릴은 옛 API라 창을 끌 때 느리다 (window-vibrancy)
pub fn windows_support(build: u32) -> Support {
    let (choices, slow) = if build >= BUILD_SYSTEM_BACKDROP {
        (vec!["tabbed", "mica", "acrylic"], vec![])
    } else if build >= BUILD_WINDOWS_11 {
        (vec!["mica", "acrylic"], vec!["acrylic"])
    } else if build >= BUILD_ACRYLIC {
        (vec!["acrylic"], vec!["acrylic"])
    } else {
        return Support::unavailable("windows", "windows-old");
    };
    Support {
        platform: "windows",
        available: true,
        reason: None,
        choices,
        slow,
        blur: Blur::System,
        live_toggle: true,
        min_opacity: 0,
    }
}

pub fn mac_support() -> Support {
    Support {
        platform: "macos",
        available: true,
        reason: None,
        choices: MAC_CHOICES.to_vec(),
        slow: vec![],
        blur: Blur::System,
        live_toggle: true,
        min_opacity: 0,
    }
}

/// macOS 효과: 26 이상은 리퀴드 글래스, 15 이하는 Tauri가 뒤의 vibrancy 재질로 내려간다
pub fn mac_effects(choice: &str) -> [&'static str; 2] {
    let glass = if choice == "clear" {
        "liquidGlassClear"
    } else {
        "liquidGlassRegular"
    };
    [glass, "underWindowBackground"]
}

/// Linux: 흐림은 창 관리자가 정한다. KWin(X11)은 앱이 흐림을 요청할 수 있고(`_KDE_NET_WM_BLUR_BEHIND_REGION`),
/// 흐림을 스스로 거는 합성기(Hyprland·Sway 계열)는 그 설정을 따르며, GNOME 등은 흐림 없이 비친다.
/// 합성기가 없는 X11은 투명한 곳이 검게 나오므로 쓰지 않는다
pub fn linux_support(desktop: &str, wayland: bool, composited: bool) -> Support {
    if !composited {
        return Support::unavailable("linux", "no-compositor");
    }
    let desktop = desktop.to_ascii_lowercase();
    let has = |name: &str| desktop.split(':').any(|d| d == name);
    let blur = if has("kde") && !wayland {
        Blur::System
    } else if ["hyprland", "sway", "wayfire", "niri"].iter().any(|d| has(d)) {
        Blur::Compositor
    } else {
        Blur::None
    };
    Support {
        platform: "linux",
        available: true,
        reason: None,
        choices: vec![],
        slow: vec![],
        blur,
        live_toggle: false,
        min_opacity: if blur == Blur::None { PLAIN_MIN_OPACITY } else { 0 },
    }
}

/// 이 컴퓨터에서 쓸 수 있는 것. Linux는 GTK 메인 스레드에서 부른다 (`gtk` 기능이 없으면 쓰지 않는다)
pub fn support_now() -> Support {
    #[cfg(windows)]
    return windows_support(windows_version::OsVersion::current().build);
    #[cfg(target_os = "macos")]
    return mac_support();
    #[cfg(all(target_os = "linux", feature = "gtk"))]
    {
        use gdk::glib::prelude::*;
        let composited = gdk::Screen::default().is_some_and(|s| s.is_composited());
        let wayland = gdk::Display::default().is_some_and(|d| d.type_().name().contains("Wayland"));
        return linux_support(
            &std::env::var("XDG_CURRENT_DESKTOP").unwrap_or_default(),
            wayland,
            composited,
        );
    }
    #[allow(unreachable_code)]
    linux_support("", false, false)
}

/// 창에 걸 Tauri `Effect` 이름: Windows는 고른 재질(여기서 안 되면 대신할 재질), macOS는 리퀴드 글래스 + vibrancy.
/// Linux는 Tauri 효과가 없다 (`set_kwin_blur`)
pub fn effect_names(support: &Support, config: &WindowBackdrop) -> Vec<&'static str> {
    match support.platform {
        "macos" => mac_effects(&config.mac).to_vec(),
        "windows" => WINDOWS_CHOICES
            .iter()
            .copied()
            .filter(|c| Some(*c) == support.pick(&config.windows))
            .collect(),
        _ => vec![],
    }
}

/// KWin(X11)에 창 뒤 흐림을 켜거나 끈다 (빈 영역 = 창 전체). 다른 창 관리자는 이 속성을 무시한다
#[cfg(all(target_os = "linux", feature = "gtk"))]
pub fn set_kwin_blur(window: &gdk::Window, on: bool) {
    let atom = gdk::Atom::intern("_KDE_NET_WM_BLUR_BEHIND_REGION");
    if on {
        let cardinal = gdk::Atom::intern("CARDINAL");
        gdk::property_change(
            window,
            &atom,
            &cardinal,
            32,
            gdk::PropMode::Replace,
            gdk::ChangeData::ULongs(&[]),
        );
    } else {
        gdk::property_delete(window, &atom);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn defaults_are_on_with_tabbed_and_regular_glass() {
        let d = WindowBackdrop::default();
        assert!(d.enabled);
        assert_eq!(
            (d.windows.as_str(), d.mac.as_str(), d.opacity, d.brightness),
            ("tabbed", "regular", 60, 0)
        );
        assert!(d.validate().is_ok());
    }

    #[test]
    fn unchoosable_values_are_rejected_then_sanitized() {
        let mut v: WindowBackdrop =
            serde_json::from_value(json!({"windows": "glass", "mac": "x", "opacity": 140, "brightness": -90})).unwrap();
        assert!(v.validate().is_err());
        v.sanitize();
        assert!(v.validate().is_ok());
        assert_eq!(
            (v.windows.as_str(), v.mac.as_str(), v.opacity, v.brightness),
            ("tabbed", "regular", 100, -50)
        );
    }

    #[test]
    fn reads_from_the_gui_section_with_defaults_for_missing_keys() {
        let v = WindowBackdrop::from_gui(&json!({"windowBackdrop": {"enabled": false, "opacity": 30}}));
        assert!(!v.enabled);
        assert_eq!(v.opacity, 30);
        assert_eq!(v.windows, "tabbed");
        assert_eq!(
            WindowBackdrop::from_gui(&json!({"windowBackdrop": "broken"})),
            WindowBackdrop::default()
        );
        assert_eq!(WindowBackdrop::from_gui(&json!({})), WindowBackdrop::default());
    }

    #[test]
    fn windows_choices_follow_the_build() {
        let new = windows_support(26100);
        assert_eq!(new.choices, ["tabbed", "mica", "acrylic"]);
        assert!(
            new.slow.is_empty(),
            "22523+ acrylic is the system backdrop, not the slow legacy API"
        );
        assert_eq!(new.pick("acrylic"), Some("acrylic"));

        let early11 = windows_support(22000);
        assert_eq!(early11.pick("tabbed"), Some("mica"), "tabbed needs 22523");
        assert_eq!(early11.slow, ["acrylic"]);

        let win10 = windows_support(19045);
        assert_eq!(win10.pick("tabbed"), Some("acrylic"));
        assert!(win10.available);

        let old = windows_support(17134);
        assert!(!old.available);
        assert_eq!(old.reason, Some("windows-old"));
        assert_eq!(old.pick("tabbed"), None);
    }

    #[test]
    fn mac_falls_back_to_vibrancy_below_26() {
        assert_eq!(mac_effects("regular"), ["liquidGlassRegular", "underWindowBackground"]);
        assert_eq!(mac_effects("clear"), ["liquidGlassClear", "underWindowBackground"]);
    }

    #[test]
    fn effect_names_follow_the_platform_and_choice() {
        let cfg = WindowBackdrop {
            windows: "tabbed".into(),
            mac: "clear".into(),
            ..Default::default()
        };
        assert_eq!(effect_names(&windows_support(26100), &cfg), ["tabbed"]);
        assert_eq!(effect_names(&windows_support(22000), &cfg), ["mica"]);
        assert_eq!(effect_names(&windows_support(19045), &cfg), ["acrylic"]);
        assert_eq!(
            effect_names(&mac_support(), &cfg),
            ["liquidGlassClear", "underWindowBackground"]
        );
        assert!(effect_names(&linux_support("KDE", false, true), &cfg).is_empty());
    }

    #[test]
    fn linux_blur_depends_on_the_desktop() {
        assert_eq!(linux_support("KDE", false, true).blur, Blur::System);
        assert_eq!(
            linux_support("KDE", true, true).blur,
            Blur::None,
            "KWin on Wayland needs a protocol we don't request"
        );
        assert_eq!(linux_support("Hyprland", true, true).blur, Blur::Compositor);
        let gnome = linux_support("ubuntu:GNOME", true, true);
        assert_eq!(gnome.blur, Blur::None);
        assert_eq!(
            gnome.opacity(40),
            PLAIN_MIN_OPACITY,
            "see-through without blur keeps text readable"
        );
        assert_eq!(gnome.opacity(95), 95);
        assert!(!gnome.live_toggle);
        let bare = linux_support("i3", false, false);
        assert!(!bare.available);
        assert_eq!(bare.reason, Some("no-compositor"));
    }
}
