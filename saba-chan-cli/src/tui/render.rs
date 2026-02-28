//! 공통 렌더링 함수 — 상태바, 브레드크럼, 출력 영역, 입력바, 힌트바
//!
//! 모든 화면에서 공유되는 UI 컴포넌트들을 여기에 모아둡니다.

use ratatui::prelude::*;
use ratatui::widgets::{
    Block, BorderType, Borders, Clear, Padding, Paragraph,
    Scrollbar, ScrollbarOrientation, ScrollbarState,
};

use super::app::{App, Out, InputMode};
use super::theme::Theme;
use super::screens;

// ═══════════════════════════════════════════════════════
// 최상위 렌더 함수 (전체 레이아웃)
// ═══════════════════════════════════════════════════════

pub fn render(app: &App, frame: &mut Frame) {
    let area = frame.area();

    let chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Length(4),  // 상태바
            Constraint::Min(8),     // 메인 컨텐츠
            Constraint::Length(2),  // 힌트바
        ])
        .split(area);

    render_status_bar(app, frame, chunks[0]);
    screens::render_screen(app, frame, chunks[1]);
    render_hint_bar(app, frame, chunks[2]);

    // 로딩 오버레이
    if let Some(ref msg) = app.loading {
        render_loading(msg, frame, chunks[1]);
    }

    // 확인 대화상자 (모달)
    if let InputMode::Confirm { ref prompt, .. } = app.input_mode {
        render_confirm_dialog(prompt, frame, area);
    }

    // 인라인 입력 오버레이 (모달)
    if let InputMode::InlineInput { ref prompt, ref value, cursor, .. } = app.input_mode {
        let popup_height = 5u16;
        let popup_width = 60.min(area.width.saturating_sub(4));
        let popup_x = area.x + (area.width.saturating_sub(popup_width)) / 2;
        let popup_y = area.y + (area.height.saturating_sub(popup_height)) / 2;
        let popup = Rect::new(popup_x, popup_y, popup_width, popup_height);
        frame.render_widget(Clear, popup);
        render_inline_input(prompt, value, cursor, frame, popup);
    }

    // 인라인 선택 오버레이 (모달)
    if let InputMode::InlineSelect { ref prompt, ref options, selected, .. } = app.input_mode {
        let popup_height = (options.len() as u16 + 3).min(area.height.saturating_sub(4));
        let popup_width = 50.min(area.width.saturating_sub(4));
        let popup_x = area.x + (area.width.saturating_sub(popup_width)) / 2;
        let popup_y = area.y + (area.height.saturating_sub(popup_height)) / 2;
        let popup = Rect::new(popup_x, popup_y, popup_width, popup_height);
        frame.render_widget(Clear, popup);
        render_inline_select(prompt, options, selected, frame, popup);
    }
}

// ═══════════════════════════════════════════════════════
// 상태바 (최상단 — 데몬/봇/서버 상태)
// ═══════════════════════════════════════════════════════

pub fn render_status_bar(app: &App, frame: &mut Frame, area: Rect) {
    let d = if app.daemon_on {
        Span::styled("● RUNNING", Theme::running())
    } else {
        Span::styled("○ OFFLINE", Theme::stopped())
    };

    let b = if app.bot_on {
        Span::styled("● RUNNING", Theme::running())
    } else if !app.token_ok {
        Span::styled("○ NO TOKEN", Theme::warning())
    } else {
        Span::styled("○ OFFLINE", Theme::stopped())
    };

    let running = app.servers.iter().filter(|s| s.status == "running").count();

    // 브레드크럼
    let crumbs = app.screen.breadcrumb();
    let mut bc_spans = Vec::new();
    for (i, crumb) in crumbs.iter().enumerate() {
        if i > 0 {
            bc_spans.push(Span::styled(" > ", Theme::breadcrumb()));
        }
        if i == crumbs.len() - 1 {
            bc_spans.push(Span::styled(crumb.to_string(), Theme::breadcrumb_current()));
        } else {
            bc_spans.push(Span::styled(crumb.to_string(), Theme::breadcrumb()));
        }
    }

    let status_line = Line::from(vec![
        Span::styled("Saba-Core ", Theme::label_daemon()),
        d,
        Span::raw("  "),
        Span::styled("Bot ", Theme::label_bot()),
        b,
        Span::raw("  "),
        Span::styled(
            format!("Instances {}/{}", running, app.servers.len()),
            Theme::label_servers(),
        ),
    ]);

    let breadcrumb_line = Line::from(bc_spans);

    let block = Block::default()
        .borders(Borders::ALL)
        .border_type(BorderType::Rounded)
        .border_style(Theme::border());

    let inner = block.inner(area);
    frame.render_widget(block, area);

    // 한 줄에 상태 + 브레드크럼을 같이 표시
    if inner.height >= 2 {
        frame.render_widget(
            Paragraph::new(status_line).alignment(Alignment::Center),
            Rect::new(inner.x, inner.y, inner.width, 1),
        );
        frame.render_widget(
            Paragraph::new(breadcrumb_line),
            Rect::new(inner.x + 1, inner.y + 1, inner.width.saturating_sub(2), 1),
        );
    } else {
        frame.render_widget(
            Paragraph::new(status_line).alignment(Alignment::Center),
            inner,
        );
    }
}

// ═══════════════════════════════════════════════════════
// 힌트바 (최하단 — 컨텍스트 민감 단축키 안내)
// ═══════════════════════════════════════════════════════

pub fn render_hint_bar(app: &App, frame: &mut Frame, area: Rect) {
    let hints = match &app.input_mode {
        InputMode::Normal => {
            match &app.screen {
                super::app::Screen::Dashboard => {
                    vec![
                        ("↑↓/jk", "이동"),
                        ("Enter", "선택"),
                        (":", "명령"),
                        ("?", "도움말"),
                        ("q", "종료"),
                    ]
                }
                super::app::Screen::CommandMode => {
                    vec![
                        ("Enter", "실행"),
                        ("Tab", "자동완성"),
                        ("↑↓", "히스토리"),
                        ("PgUp/Dn/Wheel", "스크롤"),
                        ("F2", "메뉴"),
                    ]
                }
                super::app::Screen::ServerConsole { .. } => {
                    vec![
                        ("Esc", "뒤로"),
                        ("Enter", "stdin 전송"),
                        ("PgUp/Dn/Wheel", "스크롤"),
                    ]
                }
                _ => {
                    vec![
                        ("↑↓/jk", "이동"),
                        ("Enter", "선택"),
                        ("Esc", "뒤로"),
                        (":", "명령"),
                    ]
                }
            }
        }
        InputMode::Editing => {
            vec![
                ("Enter", "확정"),
                ("Esc", "취소"),
                ("←→", "커서"),
            ]
        }
        InputMode::Command => {
            let has_stack = !app.screen_stack.is_empty();
            let mut h = vec![
                ("Enter", "실행"),
                ("Tab", "자동완성"),
                ("↑↓", "히스토리"),
                ("PgUp/Dn", "스크롤"),
                ("F2", "메뉴"),
            ];
            if has_stack {
                h.insert(0, ("Esc", "뒤로"));
            }
            h
        }
        InputMode::Console => {
            vec![
                ("Esc", "뒤로"),
                ("Enter", "전송"),
            ]
        }
        InputMode::Confirm { .. } => {
            vec![
                ("y", "확인"),
                ("n/Esc", "취소"),
            ]
        }
        InputMode::InlineInput { .. } => {
            vec![
                ("Enter", "확인"),
                ("Esc", "취소"),
                ("←→", "커서"),
            ]
        }
        InputMode::InlineSelect { .. } => {
            vec![
                ("↑↓/jk", "이동"),
                ("Enter", "선택"),
                ("Esc", "취소"),
            ]
        }
    };

    let mut spans = vec![Span::raw(" ")];
    for (i, (key, desc)) in hints.iter().enumerate() {
        if i > 0 {
            spans.push(Span::styled("  │  ", Theme::dimmed()));
        }
        spans.push(Span::styled(key.to_string(), Theme::shortcut()));
        spans.push(Span::styled(format!(" {}", desc), Theme::hint_bar()));
    }

    // 일시적 상태 메시지가 있으면 우측에 표시
    if let Some((msg, at)) = &app.status_message {
        if at.elapsed().as_secs() < 5 {
            let padding = area.width.saturating_sub(
                spans.iter().map(|s| s.width() as u16).sum::<u16>() + msg.len() as u16 + 4
            );
            if padding > 0 {
                spans.push(Span::raw(" ".repeat(padding as usize)));
            }
            spans.push(Span::styled(format!(" {} ", msg), Theme::success()));
        }
    }

    let block = Block::default()
        .borders(Borders::TOP)
        .border_style(Theme::border());

    frame.render_widget(
        Paragraph::new(Line::from(spans)).block(block),
        area,
    );
}

// ═══════════════════════════════════════════════════════
// 메뉴 렌더링 (재사용 가능)
// ═══════════════════════════════════════════════════════

pub fn render_menu(items: &[super::app::MenuItem], selected: usize, frame: &mut Frame, area: Rect) {
    let lines: Vec<Line> = items
        .iter()
        .enumerate()
        .map(|(i, item)| {
            let is_sel = i == selected;
            let arrow = if is_sel { "▸ " } else { "  " };
            let shortcut_str = match item.shortcut {
                Some(c) => format!("[{}] ", c),
                None => "    ".to_string(),
            };

            let mut spans = vec![
                Span::styled(
                    arrow,
                    if is_sel { Theme::selected_arrow() } else { Style::default() },
                ),
                Span::styled(shortcut_str, Theme::shortcut()),
            ];

            let label_style = if !item.enabled {
                Theme::disabled()
            } else if is_sel {
                Theme::selected()
            } else {
                Style::default()
            };

            spans.push(Span::styled(item.label.clone(), label_style));

            if !item.description.is_empty() {
                spans.push(Span::raw("  "));
                spans.push(Span::styled(item.description.clone(), Theme::dimmed()));
            }

            if let Some(badge) = &item.badge {
                spans.push(Span::raw("  "));
                spans.push(Span::styled(badge.clone(), Theme::badge()));
            }

            Line::from(spans)
        })
        .collect();

    frame.render_widget(Paragraph::new(lines), area);
}

// ═══════════════════════════════════════════════════════
// 에디터 렌더링 (vim-like 프로퍼티 에디터)
// ═══════════════════════════════════════════════════════

pub fn render_editor(app: &App, frame: &mut Frame, area: Rect) {
    let mut lines: Vec<Line> = Vec::new();
    let mut current_group = String::new();

    for (i, field) in app.editor_fields.iter().enumerate() {
        // 그룹 헤더 삽입
        if field.group != current_group {
            if !current_group.is_empty() {
                lines.push(Line::from(""));
            }
            current_group = field.group.clone();
            lines.push(Line::from(Span::styled(
                format!("  {}", current_group.to_uppercase()),
                Theme::group_header(),
            )));
        }

        let is_sel = i == app.editor_selected;
        let is_editing = is_sel && app.input_mode == InputMode::Editing;
        let is_changed = field.value != field.original_value;

        let arrow = if is_sel { "  ▸ " } else { "    " };
        let req = if field.required { "*" } else { " " };

        let value_display = if is_editing {
            // 편집 중 — 커서 표시
            format!("│{}│", app.edit_buffer)
        } else if field.field_type == "password" && !field.value.is_empty() {
            "****".to_string()
        } else if field.field_type == "boolean" {
            let symbol = if field.value == "true" { "☑" } else { "☐" };
            format!("{} {}", symbol, field.value)
        } else if field.field_type == "select" {
            format!("◆ {}", field.value)
        } else if field.value.is_empty() {
            "(not set)".to_string()
        } else {
            field.value.clone()
        };

        let val_style = if is_editing {
            Theme::editor_editing()
        } else if is_changed {
            Theme::editor_changed()
        } else {
            Theme::editor_value()
        };

        let line = Line::from(vec![
            Span::styled(
                arrow,
                if is_sel { Theme::selected_arrow() } else { Style::default() },
            ),
            Span::styled(req, Theme::required_mark()),
            Span::styled(
                format!("{:<24}", field.key),
                Theme::editor_key(),
            ),
            Span::styled(value_display, val_style),
            if !field.label.is_empty() && !is_editing {
                Span::styled(format!("  — {}", field.label), Theme::dimmed())
            } else {
                Span::raw("")
            },
        ]);

        lines.push(line);
    }

    // 선택된 필드의 줄 번호 계산 (그룹 헤더 + 빈 줄 포함)
    let mut sel_row = 0usize;
    {
        let mut grp = String::new();
        for (i, f) in app.editor_fields.iter().enumerate() {
            if f.group != grp {
                if !grp.is_empty() { sel_row += 1; } // blank
                grp = f.group.clone();
                sel_row += 1; // group header
            }
            if i == app.editor_selected { break; }
            sel_row += 1;
        }
    }

    let visible = area.height as usize;
    let total = lines.len();
    if total > visible {
        // 선택 필드가 보이도록 스크롤
        let scroll_y = if sel_row >= visible {
            (sel_row - visible / 2).min(total.saturating_sub(visible))
        } else {
            0
        };
        frame.render_widget(Paragraph::new(lines).scroll((scroll_y as u16, 0)), area);
    } else {
        frame.render_widget(Paragraph::new(lines), area);
    }
}

// ═══════════════════════════════════════════════════════
// 커맨드 모드 출력 영역 (레거시 호환)
// ═══════════════════════════════════════════════════════

pub fn render_output(app: &App, frame: &mut Frame, area: Rect) {
    let block = Block::default()
        .borders(Borders::LEFT | Borders::RIGHT)
        .border_style(Theme::border())
        .padding(Padding::horizontal(1));

    let inner = block.inner(area);
    frame.render_widget(block, area);

    let lines: Vec<Line> = app.output.iter().map(|e| match e {
        Out::Info(s) => Line::from(Span::styled(s.clone(), Theme::info())),
        Out::Cmd(s) => Line::from(vec![
            Span::styled("❯ ", Theme::prompt()),
            Span::styled(s.clone(), Theme::cmd_text()),
        ]),
        Out::Ok(s) => Line::from(Span::styled(s.clone(), Theme::success())),
        Out::Err(s) => Line::from(Span::styled(s.clone(), Theme::error())),
        Out::Text(s) => Line::from(s.clone()),
        Out::Blank => Line::from(""),
    }).collect();

    let total = lines.len();
    let visible = inner.height as usize;
    let max_up = total.saturating_sub(visible);
    let eff_up = app.scroll_up.min(max_up);
    let y = max_up.saturating_sub(eff_up);

    frame.render_widget(Paragraph::new(lines).scroll((y as u16, 0)), inner);

    // 스크롤바
    if total > visible {
        let mut scrollbar_state = ScrollbarState::new(max_up)
            .position(max_up.saturating_sub(eff_up));
        let scrollbar = Scrollbar::new(ScrollbarOrientation::VerticalRight)
            .begin_symbol(Some("↑"))
            .end_symbol(Some("↓"))
            .track_symbol(Some("│"))
            .thumb_symbol("█");
        frame.render_stateful_widget(scrollbar, area, &mut scrollbar_state);

        if eff_up > 0 {
            let indicator = format!(" ↑{} ", eff_up);
            let ind_x = area.right().saturating_sub(indicator.len() as u16 + 1);
            frame.render_widget(
                Paragraph::new(Span::styled(indicator, Style::default().fg(Color::Yellow).add_modifier(Modifier::BOLD))),
                Rect::new(ind_x, area.y, 8, 1),
            );
        }
    }
}

// ═══════════════════════════════════════════════════════
// 커맨드 모드 입력바
// ═══════════════════════════════════════════════════════

pub fn render_command_input(app: &App, frame: &mut Frame, area: Rect) {
    let block = Block::default()
        .borders(Borders::ALL)
        .border_type(BorderType::Rounded)
        .border_style(Theme::border_active());

    let inner = block.inner(area);
    frame.render_widget(block, area);

    let prompt = "saba> ";
    let line = Line::from(vec![
        Span::styled(prompt, Theme::prompt()),
        Span::raw(app.input.clone()),
    ]);

    frame.render_widget(Paragraph::new(line), inner);

    // 커서
    let display_width: usize = app.input.chars().take(app.cursor)
        .map(|c| if c.is_ascii() { 1 } else { 2 })
        .sum();
    let cx = inner.x + prompt.len() as u16 + display_width as u16;
    let cy = inner.y;
    frame.set_cursor_position(Position::new(cx, cy));
}

// ═══════════════════════════════════════════════════════
// 에디터 인라인 편집 커서
// ═══════════════════════════════════════════════════════

pub fn render_edit_cursor(app: &App, frame: &mut Frame, area: Rect) {
    if app.input_mode != InputMode::Editing { return; }
    // 현재 편집 중인 필드 위치를 역산하여 커서 배치
    // 간단히: 선택된 줄의 값 영역 안에 커서
    let field_row = app.editor_selected;
    // 그룹 헤더 + 빈 줄 수를 감안한 실제 줄 번호 계산
    let mut actual_row = 0;
    let mut current_group = String::new();
    for (i, f) in app.editor_fields.iter().enumerate() {
        if f.group != current_group {
            if !current_group.is_empty() { actual_row += 1; } // blank line
            current_group = f.group.clone();
            actual_row += 1; // group header
        }
        if i == field_row { break; }
        actual_row += 1;
    }

    let prefix_len = 4 + 1 + 24 + 1; // arrow + req + key + separator (│)
    let cursor_offset: usize = app.edit_buffer.chars().take(app.edit_cursor)
        .map(|c| if c.is_ascii() { 1 } else { 2 })
        .sum();

    let cx = area.x + prefix_len as u16 + cursor_offset as u16;
    let cy = area.y + actual_row as u16;

    if cx < area.right() && cy < area.bottom() {
        frame.set_cursor_position(Position::new(cx, cy));
    }
}

// ═══════════════════════════════════════════════════════
// 콘솔 렌더링
// ═══════════════════════════════════════════════════════

pub fn render_console(app: &App, frame: &mut Frame, area: Rect) {
    let chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints([Constraint::Min(3), Constraint::Length(3)])
        .split(area);

    // 콘솔 출력
    let block = Block::default()
        .title(" Console ")
        .borders(Borders::ALL)
        .border_type(BorderType::Rounded)
        .border_style(Theme::border());

    let inner = block.inner(chunks[0]);
    frame.render_widget(block, chunks[0]);

    let lines: Vec<Line> = app.console_lines.iter()
        .map(|l| Line::from(Span::styled(l.clone(), Theme::console_text())))
        .collect();

    let total = lines.len();
    let visible = inner.height as usize;
    let max_up = total.saturating_sub(visible);
    let eff_up = app.console_scroll.min(max_up);
    let y = max_up.saturating_sub(eff_up);

    frame.render_widget(Paragraph::new(lines).scroll((y as u16, 0)), inner);

    // stdin 입력
    let input_block = Block::default()
        .borders(Borders::ALL)
        .border_type(BorderType::Rounded)
        .border_style(Theme::border_active());
    let input_inner = input_block.inner(chunks[1]);
    frame.render_widget(input_block, chunks[1]);

    let line = Line::from(vec![
        Span::styled("> ", Theme::console_input()),
        Span::raw(app.console_input.clone()),
    ]);
    frame.render_widget(Paragraph::new(line), input_inner);

    if app.input_mode == InputMode::Console {
        let cursor_x = input_inner.x + 2 + app.console_input.chars().count() as u16;
        frame.set_cursor_position(Position::new(cursor_x, input_inner.y));
    }
}

// ═══════════════════════════════════════════════════════
// 확인 대화상자 렌더링
// ═══════════════════════════════════════════════════════

pub fn render_confirm_dialog(prompt: &str, frame: &mut Frame, area: Rect) {
    let width = (prompt.len() as u16 + 8).min(area.width.saturating_sub(4));
    let height = 5;
    let x = area.x + (area.width.saturating_sub(width)) / 2;
    let y = area.y + (area.height.saturating_sub(height)) / 2;
    let popup = Rect::new(x, y, width, height);

    // 배경 클리어
    frame.render_widget(Clear, popup);

    let block = Block::default()
        .title(" Confirm ")
        .borders(Borders::ALL)
        .border_type(BorderType::Rounded)
        .border_style(Style::default().fg(Color::Yellow));

    let inner = block.inner(popup);
    frame.render_widget(block, popup);

    let text = vec![
        Line::from(prompt.to_string()),
        Line::from(""),
        Line::from(vec![
            Span::styled("[y]", Theme::shortcut()),
            Span::raw(" Yes   "),
            Span::styled("[n/Esc]", Theme::shortcut()),
            Span::raw(" No"),
        ]),
    ];

    frame.render_widget(Paragraph::new(text).alignment(Alignment::Center), inner);
}

// ═══════════════════════════════════════════════════════
// 로딩 표시
// ═══════════════════════════════════════════════════════

#[allow(dead_code)]
pub fn render_loading(msg: &str, frame: &mut Frame, area: Rect) {
    let spinner_chars = ['⠋', '⠙', '⠹', '⠸', '⠼', '⠴', '⠦', '⠧', '⠇', '⠏'];
    let idx = (std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis() / 100) as usize % spinner_chars.len();

    let line = Line::from(vec![
        Span::styled(
            format!(" {} ", spinner_chars[idx]),
            Style::default().fg(Color::Cyan).add_modifier(Modifier::BOLD),
        ),
        Span::styled(msg, Theme::info()),
    ]);

    frame.render_widget(Paragraph::new(line), area);
}

// ═══════════════════════════════════════════════════════
// 자동완성 팝업
// ═══════════════════════════════════════════════════════

pub fn render_autocomplete_popup(app: &App, frame: &mut Frame, input_area: Rect) {
    if !app.autocomplete_visible || app.autocomplete_candidates.is_empty() {
        return;
    }

    let max_show = 8.min(app.autocomplete_candidates.len());
    let popup_height = max_show as u16 + 2; // +2 for border
    let popup_y = input_area.y.saturating_sub(popup_height);
    let popup_width = 40.min(input_area.width);

    // saba> 프롬프트 길이만큼 오프셋
    let popup_x = input_area.x + 6;

    let popup_area = Rect::new(popup_x, popup_y, popup_width, popup_height);

    // 배경 클리어
    frame.render_widget(Clear, popup_area);

    let block = Block::default()
        .borders(Borders::ALL)
        .border_type(BorderType::Rounded)
        .border_style(Style::default().fg(Color::DarkGray));

    let inner = block.inner(popup_area);
    frame.render_widget(block, popup_area);

    let lines: Vec<Line> = app.autocomplete_candidates.iter()
        .take(max_show)
        .enumerate()
        .map(|(i, candidate)| {
            let is_sel = i == app.autocomplete_selected;
            if is_sel {
                Line::from(Span::styled(
                    format!("▸ {}", candidate),
                    Style::default().bg(Color::DarkGray).fg(Color::Cyan).add_modifier(Modifier::BOLD),
                ))
            } else {
                Line::from(Span::styled(
                    format!("  {}", candidate),
                    Style::default().fg(Color::White),
                ))
            }
        })
        .collect();

    frame.render_widget(Paragraph::new(lines), inner);
}

// ═══════════════════════════════════════════════════════
// 인라인 입력 렌더링
// ═══════════════════════════════════════════════════════

pub fn render_inline_input(
    prompt: &str, value: &str, cursor: usize,
    frame: &mut Frame, area: Rect,
) {
    let block = Block::default()
        .title(format!(" {} ", prompt))
        .borders(Borders::ALL)
        .border_type(BorderType::Rounded)
        .border_style(Style::default().fg(Color::Cyan));

    let inner = block.inner(area);
    frame.render_widget(block, area);

    let line = Line::from(vec![
        Span::styled("❯ ", Theme::prompt()),
        Span::styled(value.to_string(), Theme::editor_editing()),
    ]);
    frame.render_widget(Paragraph::new(line), Rect::new(
        inner.x + 1, inner.y, inner.width.saturating_sub(2), 1,
    ));

    let hint = Line::from(Span::styled(
        "  Enter: 확인  |  Esc: 취소",
        Theme::dimmed(),
    ));
    if inner.height > 1 {
        frame.render_widget(Paragraph::new(hint), Rect::new(
            inner.x + 1, inner.y + 1, inner.width.saturating_sub(2), 1,
        ));
    }

    // 커서
    let display_width: usize = value.chars().take(cursor)
        .map(|c| if c.is_ascii() { 1 } else { 2 })
        .sum();
    let cx = inner.x + 3 + display_width as u16; // 1(padding) + 2(❯ )
    frame.set_cursor_position(Position::new(cx, inner.y));
}

// ═══════════════════════════════════════════════════════
// 인라인 선택 렌더링
// ═══════════════════════════════════════════════════════

pub fn render_inline_select(
    prompt: &str, options: &[String], selected: usize,
    frame: &mut Frame, area: Rect,
) {
    let block = Block::default()
        .title(format!(" {} ", prompt))
        .borders(Borders::ALL)
        .border_type(BorderType::Rounded)
        .border_style(Style::default().fg(Color::Cyan));

    let inner = block.inner(area);
    frame.render_widget(block, area);

    let lines: Vec<Line> = options.iter().enumerate().map(|(i, opt)| {
        let is_sel = i == selected;
        let arrow = if is_sel { "▸ " } else { "  " };
        let style = if is_sel {
            Style::default().bg(Color::DarkGray).fg(Color::Cyan).add_modifier(Modifier::BOLD)
        } else {
            Style::default().fg(Color::White)
        };
        Line::from(Span::styled(format!("{}{}", arrow, opt), style))
    }).collect();

    frame.render_widget(Paragraph::new(lines), Rect::new(
        inner.x + 1, inner.y, inner.width.saturating_sub(2), inner.height,
    ));
}
