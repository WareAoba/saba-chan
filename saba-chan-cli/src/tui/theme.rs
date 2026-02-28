//! 테마 · 스타일 상수 — 전체 TUI에서 일관된 색상 사용

use ratatui::style::{Color, Modifier, Style};

/// 모든 TUI 스타일을 중앙 관리하는 네임스페이스
pub struct Theme;

#[allow(dead_code)]
impl Theme {
    // ─── 상태 표시 ───
    pub fn running()  -> Style { Style::default().fg(Color::Green).add_modifier(Modifier::BOLD) }
    pub fn stopped()  -> Style { Style::default().fg(Color::DarkGray) }
    pub fn warning()  -> Style { Style::default().fg(Color::Yellow) }
    pub fn error()    -> Style { Style::default().fg(Color::Red) }
    pub fn success()  -> Style { Style::default().fg(Color::Green) }

    // ─── 테두리 ───
    pub fn border()        -> Style { Style::default().fg(Color::DarkGray) }
    pub fn border_active() -> Style { Style::default().fg(Color::Cyan) }

    // ─── 타이틀 · 라벨 ───
    pub fn title()         -> Style { Style::default().fg(Color::Cyan).add_modifier(Modifier::BOLD) }
    pub fn label_daemon()  -> Style { Style::default().fg(Color::Cyan) }
    pub fn label_bot()     -> Style { Style::default().fg(Color::Magenta) }
    pub fn label_servers() -> Style { Style::default().fg(Color::Blue) }

    // ─── 메뉴 ───
    pub fn selected()       -> Style { Style::default().bg(Color::DarkGray).fg(Color::White).add_modifier(Modifier::BOLD) }
    pub fn selected_arrow() -> Style { Style::default().fg(Color::Cyan).add_modifier(Modifier::BOLD) }
    pub fn shortcut()       -> Style { Style::default().fg(Color::Yellow).add_modifier(Modifier::BOLD) }
    pub fn badge()          -> Style { Style::default().fg(Color::DarkGray) }
    pub fn disabled()       -> Style { Style::default().fg(Color::DarkGray) }

    // ─── 브레드크럼 ───
    pub fn breadcrumb()         -> Style { Style::default().fg(Color::DarkGray) }
    pub fn breadcrumb_current() -> Style { Style::default().fg(Color::Cyan).add_modifier(Modifier::BOLD) }

    // ─── 에디터 ───
    pub fn editor_key()       -> Style { Style::default().fg(Color::Cyan) }
    pub fn editor_value()     -> Style { Style::default().fg(Color::White) }
    pub fn editor_editing()   -> Style { Style::default().fg(Color::Yellow).add_modifier(Modifier::UNDERLINED) }
    pub fn editor_changed()   -> Style { Style::default().fg(Color::Yellow) }
    pub fn group_header()     -> Style { Style::default().fg(Color::Magenta).add_modifier(Modifier::BOLD) }
    pub fn required_mark()    -> Style { Style::default().fg(Color::Red).add_modifier(Modifier::BOLD) }

    // ─── 커맨드 모드 (레거시) ───
    pub fn prompt()    -> Style { Style::default().fg(Color::Cyan).add_modifier(Modifier::BOLD) }
    pub fn cmd_text()  -> Style { Style::default().fg(Color::White).add_modifier(Modifier::BOLD) }
    pub fn info()      -> Style { Style::default().fg(Color::DarkGray).add_modifier(Modifier::ITALIC) }
    pub fn dimmed()    -> Style { Style::default().fg(Color::DarkGray) }
    pub fn hint_bar()  -> Style { Style::default().fg(Color::DarkGray) }

    // ─── 콘솔 ───
    pub fn console_text()  -> Style { Style::default().fg(Color::White) }
    pub fn console_input() -> Style { Style::default().fg(Color::Yellow) }

    // ─── 자동완성 팝업 ───
    pub fn autocomplete_selected() -> Style { Style::default().bg(Color::DarkGray).fg(Color::Cyan).add_modifier(Modifier::BOLD) }
    pub fn autocomplete_normal()   -> Style { Style::default().fg(Color::White) }

    // ─── 인라인 입력/선택 ───
    pub fn inline_border()  -> Style { Style::default().fg(Color::Cyan) }
    pub fn inline_prompt()  -> Style { Style::default().fg(Color::Cyan).add_modifier(Modifier::BOLD) }
}
