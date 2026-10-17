//! 테마 · 스타일 상수 — 전체 TUI에서 일관된 색상 사용
//!
//! Stage 5: 런타임 모드(`dark`/`light`) 전환 지원.
//! `Theme::set_mode(name)`로 변경, `Theme::mode()`로 조회.

use std::sync::atomic::{AtomicU8, Ordering};

use ratatui::style::{Color, Modifier, Style};

#[derive(Copy, Clone, Debug, PartialEq, Eq)]
pub enum ThemeMode {
    Dark = 0,
    Light = 1,
}

static MODE: AtomicU8 = AtomicU8::new(0);

fn current() -> ThemeMode {
    match MODE.load(Ordering::Relaxed) {
        1 => ThemeMode::Light,
        _ => ThemeMode::Dark,
    }
}

/// 모든 TUI 스타일을 중앙 관리하는 네임스페이스
pub struct Theme;

#[allow(dead_code)]
impl Theme {
    pub fn set_mode(name: &str) -> bool {
        match name.to_lowercase().as_str() {
            "dark" | "default" => { MODE.store(0, Ordering::Relaxed); true }
            "light" => { MODE.store(1, Ordering::Relaxed); true }
            _ => false,
        }
    }

    pub fn mode() -> ThemeMode { current() }

    pub fn mode_name() -> &'static str {
        match current() {
            ThemeMode::Dark => "dark",
            ThemeMode::Light => "light",
        }
    }

    // ─── 상태 표시 ───
    pub fn running()  -> Style { Style::default().fg(Color::Green).add_modifier(Modifier::BOLD) }
    pub fn stopped()  -> Style {
        match current() {
            ThemeMode::Dark  => Style::default().fg(Color::DarkGray),
            ThemeMode::Light => Style::default().fg(Color::Gray),
        }
    }
    pub fn warning()  -> Style { Style::default().fg(Color::Yellow) }
    pub fn error()    -> Style { Style::default().fg(Color::Red) }
    pub fn success()  -> Style { Style::default().fg(Color::Green) }

    // ─── 테두리 ───
    pub fn border() -> Style {
        match current() {
            ThemeMode::Dark  => Style::default().fg(Color::DarkGray),
            ThemeMode::Light => Style::default().fg(Color::Gray),
        }
    }
    pub fn border_active() -> Style {
        match current() {
            ThemeMode::Dark  => Style::default().fg(Color::Cyan),
            ThemeMode::Light => Style::default().fg(Color::Blue),
        }
    }

    // ─── 타이틀 · 라벨 ───
    pub fn title() -> Style {
        match current() {
            ThemeMode::Dark  => Style::default().fg(Color::Cyan).add_modifier(Modifier::BOLD),
            ThemeMode::Light => Style::default().fg(Color::Blue).add_modifier(Modifier::BOLD),
        }
    }
    pub fn label_daemon()  -> Style { Style::default().fg(Color::Cyan) }
    pub fn label_bot()     -> Style { Style::default().fg(Color::Magenta) }
    pub fn label_servers() -> Style { Style::default().fg(Color::Blue) }

    // ─── 메뉴 ───
    pub fn selected() -> Style {
        match current() {
            ThemeMode::Dark  => Style::default().bg(Color::DarkGray).fg(Color::White).add_modifier(Modifier::BOLD),
            ThemeMode::Light => Style::default().bg(Color::Gray).fg(Color::Black).add_modifier(Modifier::BOLD),
        }
    }
    pub fn selected_arrow() -> Style {
        match current() {
            ThemeMode::Dark  => Style::default().fg(Color::Cyan).add_modifier(Modifier::BOLD),
            ThemeMode::Light => Style::default().fg(Color::Blue).add_modifier(Modifier::BOLD),
        }
    }
    pub fn shortcut()       -> Style { Style::default().fg(Color::Yellow).add_modifier(Modifier::BOLD) }
    pub fn badge() -> Style {
        match current() {
            ThemeMode::Dark  => Style::default().fg(Color::DarkGray),
            ThemeMode::Light => Style::default().fg(Color::Gray),
        }
    }
    pub fn disabled() -> Style { Theme::badge() }

    // ─── 브레드크럼 ───
    pub fn breadcrumb() -> Style { Theme::badge() }
    pub fn breadcrumb_current() -> Style {
        match current() {
            ThemeMode::Dark  => Style::default().fg(Color::Cyan).add_modifier(Modifier::BOLD),
            ThemeMode::Light => Style::default().fg(Color::Blue).add_modifier(Modifier::BOLD),
        }
    }

    // ─── 에디터 ───
    pub fn editor_key()       -> Style { Style::default().fg(Color::Cyan) }
    pub fn editor_value() -> Style {
        match current() {
            ThemeMode::Dark  => Style::default().fg(Color::White),
            ThemeMode::Light => Style::default().fg(Color::Black),
        }
    }
    pub fn editor_editing()   -> Style { Style::default().fg(Color::Yellow).add_modifier(Modifier::UNDERLINED) }
    pub fn editor_changed()   -> Style { Style::default().fg(Color::Yellow) }
    pub fn group_header()     -> Style { Style::default().fg(Color::Magenta).add_modifier(Modifier::BOLD) }
    pub fn required_mark()    -> Style { Style::default().fg(Color::Red).add_modifier(Modifier::BOLD) }

    // ─── 커맨드 모드 (레거시) ───
    pub fn prompt() -> Style {
        match current() {
            ThemeMode::Dark  => Style::default().fg(Color::Cyan).add_modifier(Modifier::BOLD),
            ThemeMode::Light => Style::default().fg(Color::Blue).add_modifier(Modifier::BOLD),
        }
    }
    pub fn cmd_text() -> Style {
        match current() {
            ThemeMode::Dark  => Style::default().fg(Color::White).add_modifier(Modifier::BOLD),
            ThemeMode::Light => Style::default().fg(Color::Black).add_modifier(Modifier::BOLD),
        }
    }
    pub fn info()      -> Style { Style::default().fg(Color::DarkGray).add_modifier(Modifier::ITALIC) }
    pub fn dimmed()    -> Style { Theme::badge() }
    pub fn hint_bar()  -> Style { Theme::badge() }

    // ─── 콘솔 ───
    pub fn console_text() -> Style {
        match current() {
            ThemeMode::Dark  => Style::default().fg(Color::White),
            ThemeMode::Light => Style::default().fg(Color::Black),
        }
    }
    pub fn console_input() -> Style { Style::default().fg(Color::Yellow) }

    // ─── 자동완성 팝업 ───
    pub fn autocomplete_selected() -> Style {
        match current() {
            ThemeMode::Dark  => Style::default().bg(Color::DarkGray).fg(Color::Cyan).add_modifier(Modifier::BOLD),
            ThemeMode::Light => Style::default().bg(Color::Gray).fg(Color::Blue).add_modifier(Modifier::BOLD),
        }
    }
    pub fn autocomplete_normal() -> Style {
        match current() {
            ThemeMode::Dark  => Style::default().fg(Color::White),
            ThemeMode::Light => Style::default().fg(Color::Black),
        }
    }

    // ─── 인라인 입력/선택 ───
    pub fn inline_border()  -> Style { Theme::border_active() }
    pub fn inline_prompt()  -> Style { Theme::prompt() }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn set_mode_dark_light() {
        assert!(Theme::set_mode("dark"));
        assert_eq!(Theme::mode_name(), "dark");
        assert!(Theme::set_mode("light"));
        assert_eq!(Theme::mode_name(), "light");
        // reset
        Theme::set_mode("dark");
    }

    #[test]
    fn set_mode_unknown_returns_false() {
        let prev = Theme::mode_name();
        assert!(!Theme::set_mode("nonexistent"));
        // 변경 안 됐음
        assert_eq!(Theme::mode_name(), prev);
    }
}
