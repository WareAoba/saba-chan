//! 그리기. 위에서 아래로 머리줄, 쪽 탭(오른쪽 끝에 찾기 안내·GUI로 전환), 쪽, 작업 줄, 출력 창(열렸을 때), 맨 아래 줄
//!
//! 서버 쪽은 왼쪽 서버 목록, 오른쪽 고른 서버(상세 탭과 버튼 → 정보 한 줄 → 탭 내용)다. 탭 내용 맨 아래 줄에 같은 뜻의 CLI 명령을 보인다.
//! 100칸보다 좁으면 목록과 상세를 한 번에 하나씩 보이고, 60칸보다 좁으면 넓혀 달라는 안내만.
//! 묻는 창·찾기 창이 뜨면 뒤를 어둡게 한다.

use super::app::{App, AskKind, Button, Focus, Page, PaneState};
use super::data::s;
use super::form::{Form, Kind, Row};
use super::highlight::{console_line, daemon_line, wrap_spans};
use super::wizard::ConfirmRow;
use ratatui::layout::{Alignment, Constraint, Direction, Layout, Rect};
use ratatui::style::{Color, Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, BorderType, Borders, Clear, List, ListItem, Paragraph, Wrap};
use ratatui::Frame;
use serde_json::Value;
use unicode_width::{UnicodeWidthChar, UnicodeWidthStr};

const MIN_WIDTH: u16 = 60;
/// 이보다 좁으면 목록과 상세를 한 번에 하나씩
const NARROW: u16 = 100;

const ACCENT: Color = Color::Cyan;

fn dim() -> Style {
    Style::default().fg(Color::DarkGray)
}

fn highlight(focused: bool) -> Style {
    if focused {
        Style::default().fg(Color::Black).bg(ACCENT).add_modifier(Modifier::BOLD)
    } else {
        Style::default().add_modifier(Modifier::REVERSED)
    }
}

/// 고른 탭: 화살표가 여기 있으면 채운 칸, 아니면 강조색 밑줄 (쪽 탭과 같다)
fn selected_tab(focused: bool) -> Style {
    if focused {
        highlight(true)
    } else {
        Style::default().fg(ACCENT).add_modifier(Modifier::BOLD | Modifier::UNDERLINED)
    }
}

fn panel(title: impl Into<String>, focused: bool) -> Block<'static> {
    let style = if focused { Style::default().fg(ACCENT) } else { dim() };
    let title = title.into();
    let block = Block::default().borders(Borders::ALL).border_type(BorderType::Rounded).border_style(style);
    if title.is_empty() {
        return block;
    }
    block.title(Span::styled(format!(" {title} "), if focused { style.add_modifier(Modifier::BOLD) } else { Style::default() }))
}

/// 표시 폭으로 줄을 접는다 (한글은 두 칸)
pub fn wrap(line: &str, width: usize) -> Vec<String> {
    if width == 0 {
        return vec![line.to_string()];
    }
    let mut out = Vec::new();
    let mut cur = String::new();
    let mut w = 0;
    for ch in line.chars() {
        let cw = ch.width().unwrap_or(0);
        if w + cw > width && !cur.is_empty() {
            out.push(std::mem::take(&mut cur));
            w = 0;
        }
        cur.push(ch);
        w += cw;
    }
    out.push(cur);
    out
}

/// 폭에 맞춰 앞을 자른다 (끝이 보이게)
fn tail_fit(text: &str, room: usize) -> String {
    let mut w = 0;
    let mut tail: Vec<char> = Vec::new();
    for ch in text.chars().rev() {
        w += ch.width().unwrap_or(0);
        if w > room {
            break;
        }
        tail.push(ch);
    }
    tail.into_iter().rev().collect()
}

/// 폭에 맞춰 뒤를 자른다
fn head_fit(text: &str, room: usize) -> String {
    let mut w = 0;
    text.chars()
        .take_while(|ch| {
            w += ch.width().unwrap_or(0);
            w <= room
        })
        .collect()
}

fn status_dot(status: &str) -> (&'static str, Color) {
    match status {
        "running" => ("●", Color::Green),
        "starting" | "provisioning" | "stopping" => ("◐", Color::Yellow),
        "crashed" | "error" => ("●", Color::Red),
        _ => ("○", Color::DarkGray),
    }
}

fn status_label(status: &str) -> String {
    crate::i18n::try_t(&format!("status.{status}")).unwrap_or_else(|| status.to_string())
}

fn uptime(v: &Value) -> String {
    v.get("start_time")
        .and_then(Value::as_u64)
        .filter(|_| s(v, "status") == "running")
        .map(|st| {
            let now = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_secs()).unwrap_or(st);
            crate::output::duration(now.saturating_sub(st))
        })
        .unwrap_or_default()
}

fn pct(v: Option<f64>) -> String {
    v.map(|p| format!("{:.0}%", p)).unwrap_or_else(|| "-".into())
}

/// 맨 위 한 줄: 이름·데몬·서버 수·CPU·메모리·디스코드·업데이트.
/// 폭이 모자라면 덜 중요한 칸(메모리 → CPU → 업데이트 → 디스코드)부터 통째로 뺀다
fn header(app: &App, width: u16) -> Line<'static> {
    let snap = &app.snap;
    let brand = Span::styled(
        format!(" {} ", t!("tui.brand", version = env!("CARGO_PKG_VERSION"))),
        Style::default().fg(Color::Black).bg(ACCENT).add_modifier(Modifier::BOLD),
    );
    let Some(h) = &snap.health else {
        return Line::from(vec![brand, Span::raw("  "), Span::styled(t!("tui.daemon_down"), Style::default().fg(Color::Red))]);
    };
    // (뺄 차례: 클수록 먼저, 칸)
    let mut items: Vec<(u8, Span<'static>)> = vec![
        (0, Span::styled(t!("tui.daemon_up", version = s(h, "version")), Style::default().fg(Color::Green))),
        (0, Span::raw(t!("tui.servers_running", running = snap.running(), total = snap.servers.len()))),
    ];
    if let Some(m) = &snap.metrics {
        items.push((5, Span::raw(format!("CPU {}", pct(m.pointer("/cpu/total_percent").and_then(Value::as_f64))))));
        if let (Some(u), Some(t)) = (m.pointer("/memory/used_bytes").and_then(Value::as_f64), m.pointer("/memory/total_bytes").and_then(Value::as_f64)) {
            items.push((6, Span::raw(t!("tui.memory_of", used = crate::cmd::monitor::bytes(u), total = crate::cmd::monitor::bytes(t)))));
        }
    }
    let relay = snap.agent.as_ref().and_then(|a| a.get("running")).and_then(Value::as_bool) == Some(true);
    items.push((
        3,
        Span::styled(
            if relay { t!("tui.discord_on") } else { t!("tui.discord_off") },
            Style::default().fg(if relay { Color::Green } else { Color::DarkGray }),
        ),
    ));
    let updates = snap.updates.as_ref().and_then(|u| u.get("updates_available")).and_then(Value::as_u64).unwrap_or(0);
    let failed = snap.updates.as_ref().is_some_and(|u| !s(u, "error").is_empty());
    if updates > 0 {
        items.push((4, Span::styled(t!("tui.updates_n", count = updates), Style::default().fg(Color::Yellow))));
    } else if snap.updates.as_ref().is_some_and(update_checked) && !failed {
        items.push((4, Span::styled(t!("tui.updates_none"), dim())));
    }
    const SEP: &str = "  │  ";
    let used = |items: &[(u8, Span)]| brand.width() + 2 + items.iter().map(|(_, s)| s.width()).sum::<usize>() + SEP.width() * items.len().saturating_sub(1);
    while used(&items) > width as usize {
        let Some(at) = items.iter().enumerate().filter(|(_, (p, _))| *p > 0).max_by_key(|(_, (p, _))| *p).map(|(i, _)| i) else { break };
        items.remove(at);
    }
    let mut spans = vec![brand, Span::raw("  ")];
    for (i, (_, span)) in items.into_iter().enumerate() {
        if i > 0 {
            spans.push(Span::styled(SEP, dim()));
        }
        spans.push(span);
    }
    Line::from(spans)
}

/// 버튼을 폭에 맞춰 여러 줄로 놓는다. (줄 수, 버튼 자리)
fn layout_buttons(buttons: &[Button], area: Rect) -> (u16, Vec<Rect>) {
    let mut rects = Vec::new();
    let (mut x, mut y) = (area.x, area.y);
    for b in buttons {
        let w = button_text(b).width() as u16 + 2;
        if x > area.x && x + w > area.x + area.width {
            x = area.x;
            y += 1;
        }
        rects.push(Rect { x, y, width: w.min(area.width), height: 1 });
        x += w + 1;
    }
    let rows = rects.last().map(|r| r.y - area.y + 1).unwrap_or(0);
    (rows, rects)
}

fn button_text(b: &Button) -> String {
    if let Some(k) = b.key {
        return format!("{} {k}", b.label);
    }
    if b.more {
        format!("{} ▾", b.label)
    } else if b.fill {
        format!("{}…", b.label)
    } else {
        b.label.clone()
    }
}

/// 넘쳐 그리지 못한 버튼은 누를 자리도 없다 (아래 칸 위에 겹쳐 눌리지 않게). 순서는 버튼 번호라 빈 자리로 남긴다
fn clickable(rects: Vec<Rect>, area: Rect) -> Vec<Rect> {
    rects.into_iter().map(|r| if r.y >= area.y + area.height { Rect::default() } else { r }).collect()
}

fn kv_lines(rows: &[(String, String)]) -> Vec<Line<'static>> {
    let w = rows.iter().map(|(k, _)| k.width()).max().unwrap_or(0);
    rows.iter()
        .map(|(k, v)| {
            let pad = " ".repeat(w.saturating_sub(k.width()));
            Line::from(vec![Span::styled(format!("{k}{pad}  "), dim()), Span::raw(v.clone())])
        })
        .collect()
}

/// 한 줄로 늘어놓는 정보: `이름 값 · 이름 값`
fn inline_kv(rows: &[(String, String)]) -> Line<'static> {
    let mut spans = Vec::new();
    for (i, (k, v)) in rows.iter().filter(|(_, v)| !v.is_empty()).enumerate() {
        if i > 0 {
            spans.push(Span::styled("  ·  ", dim()));
        }
        spans.push(Span::styled(format!("{k} "), dim()));
        spans.push(Span::raw(v.clone()));
    }
    Line::from(spans)
}

fn draw_list(f: &mut Frame, app: &mut App, area: Rect, title: String, items: Vec<ListItem<'static>>, empty: String) {
    let focused = app.focus == Focus::List;
    app.areas.list = area;
    if items.is_empty() {
        f.render_widget(Paragraph::new(Span::styled(empty, dim())).block(panel(title, focused)).wrap(Wrap { trim: false }), area);
        return;
    }
    let list = List::new(items).block(panel(title, focused)).highlight_style(highlight(focused));
    f.render_stateful_widget(list, area, &mut app.list_state);
}

fn list_width(area: Rect, pct: u16, min: u16, max: u16) -> u16 {
    (area.width * pct / 100).clamp(min, max).min(area.width.saturating_sub(20))
}

fn split_list(area: Rect, width: u16) -> (Rect, Rect) {
    let cols = Layout::default().direction(Direction::Horizontal).constraints([Constraint::Length(width), Constraint::Min(10)]).split(area);
    (cols[0], cols[1])
}

/// 콘솔 줄들을 접어 끝에서부터 `scroll`만큼 올려 그린다. 넘친 만큼 `scroll`을 줄인다
fn draw_lines(f: &mut Frame, area: Rect, lines: Vec<Line<'static>>, scroll: &mut usize) {
    if area.height == 0 {
        return;
    }
    let h = area.height as usize;
    *scroll = (*scroll).min(lines.len().saturating_sub(h));
    let end = lines.len() - *scroll;
    let start = end.saturating_sub(h);
    f.render_widget(Paragraph::new(lines[start..end].to_vec()), area);
}

/// 판: 지금 판, 새 판이 있으면 그것도
fn version_cell(v: &Value, newer: Option<String>) -> String {
    match newer {
        Some(latest) => format!("{}  →  {}", s(v, "version"), t!("tui.newer", version = latest)),
        None => s(v, "version").to_string(),
    }
}

/// 출처: 확인됨·확인 안 됨·바뀜과 그 뜻
fn trust_cell(v: &Value) -> Option<String> {
    let trust = s(v, "trust");
    if trust.is_empty() {
        return None;
    }
    let label = crate::i18n::try_t(&format!("trust.{trust}")).unwrap_or_else(|| trust.to_string());
    Some(match crate::i18n::try_t(&format!("tui.trust_about.{trust}")) {
        Some(about) => format!("{label} · {about}"),
        None => label,
    })
}

fn module_rows(app: &App, v: &Value) -> Vec<(String, String)> {
    let name = s(v, "name");
    let mut rows = Vec::new();
    if !s(v, "game_name").is_empty() {
        rows.push((t!("col.game"), s(v, "game_name").to_string()));
    }
    rows.push((t!("col.version"), version_cell(v, app.newer_version())));
    let used: Vec<&str> = app.snap.servers.iter().filter(|sv| s(sv, "module") == name).map(|sv| s(sv, "name")).collect();
    rows.push((t!("tui.col.used_by"), if used.is_empty() { t!("tui.none") } else { used.join(", ") }));
    if let Some(trust) = trust_cell(v) {
        rows.push((t!("col.trust"), trust));
    }
    for (k, label) in [("description", "tui.col.about"), ("author", "col.author")] {
        if !s(v, k).is_empty() {
            rows.push((t!(label), s(v, k).to_string()));
        }
    }
    rows
}

fn extension_rows(app: &App, v: &Value) -> Vec<(String, String)> {
    let mut rows = Vec::new();
    if !s(v, "name").is_empty() {
        rows.push((t!("tui.col.name"), s(v, "name").to_string()));
    }
    rows.push((t!("tui.col.enabled"), crate::output::yes_no(v.get("enabled").and_then(Value::as_bool) == Some(true))));
    rows.push((t!("col.version"), version_cell(v, app.newer_version())));
    if let Some(trust) = trust_cell(v) {
        rows.push((t!("col.trust"), trust));
    }
    for (k, label) in [("description", "tui.col.about"), ("author", "col.author")] {
        if !s(v, k).is_empty() {
            rows.push((t!(label), s(v, k).to_string()));
        }
    }
    rows
}

fn update_checked(u: &Value) -> bool {
    !s(u, "last_check").is_empty()
}

fn update_lines(app: &App) -> Vec<Line<'static>> {
    let u = app.snap.updates.clone().unwrap_or(Value::Null);
    let n = u.get("updates_available").and_then(Value::as_u64).unwrap_or(0);
    let failed = !s(&u, "error").is_empty();
    let mut lines = Vec::new();
    if u.is_null() || (n == 0 && !failed && !update_checked(&u)) {
        // 아직 답을 못 받았거나 한 번도 확인하지 않았다: 최신인지 모른다
        lines.push(Line::from(Span::styled(t!("tui.updates_checking"), dim())));
    } else if n > 0 {
        lines.push(Line::from(Span::styled(t!("tui.updates_n", count = n), Style::default().fg(Color::Yellow).add_modifier(Modifier::BOLD))));
    } else if !failed {
        // 확인에 실패했으면 "최신"이라고 말하지 않는다
        lines.push(Line::from(Span::styled(t!("tui.updates_none"), Style::default().fg(Color::Green))));
    }
    if u.get("checking").and_then(Value::as_bool) == Some(true) {
        lines.push(Line::from(Span::styled(t!("tui.updates_checking"), dim())));
    }
    if failed {
        lines.push(Line::from(Span::styled(t!("tui.updates_error"), Style::default().fg(Color::Red))));
    }
    lines.push(Line::from(""));
    for c in u.get("components").and_then(Value::as_array).into_iter().flatten() {
        let name = s(c, "display_name");
        let name = if name.is_empty() { s(c, "component") } else { name };
        let cur = s(c, "current_version");
        let latest = s(c, "latest_version");
        let mark = if c.get("update_available").and_then(Value::as_bool) == Some(true) { Span::styled(" ↑", Style::default().fg(Color::Yellow)) } else { Span::raw("") };
        lines.push(Line::from(vec![Span::raw(format!("{name}  ")), Span::styled(format!("{cur} → {latest}"), dim()), mark]));
    }
    lines
}

/// 출력 창: 명령 결과(접어서, 끝에서부터)
fn draw_output(f: &mut Frame, app: &mut App, area: Rect) {
    let title = match &app.last_command {
        Some(c) if app.running => format!("{} › {c}  {}", t!("tui.output"), t!("tui.running_hint")),
        Some(c) => format!("{} › {c}", t!("tui.output")),
        None => t!("tui.output"),
    };
    let block = panel(title, true);
    let inner = block.inner(area);
    f.render_widget(block, area);
    app.areas.output = area;
    let lines: Vec<Line<'static>> = app
        .output
        .lines
        .iter()
        .flat_map(|l| wrap(l, inner.width as usize))
        .map(|l| {
            let style = if l.starts_with("› ") {
                Style::default().fg(ACCENT)
            } else if l.starts_with('✗') {
                Style::default().fg(Color::Red)
            } else if l.starts_with('✓') {
                Style::default().fg(Color::Green)
            } else if l.starts_with('!') {
                Style::default().fg(Color::Yellow)
            } else {
                Style::default()
            };
            Line::from(Span::styled(l, style))
        })
        .collect();
    draw_lines(f, inner, lines, &mut app.output.scroll);
}

/// 맨 아래 한 줄: 명령 줄(열었을 때), 아니면 방금 결과와 키 안내
fn footer(app: &mut App, area: Rect) -> Line<'static> {
    app.areas.command = area;
    let width = area.width as usize;
    if app.focus == Focus::Command {
        let prompt = Span::styled("saba› ", Style::default().fg(ACCENT).add_modifier(Modifier::BOLD));
        let hint = key_hint(app, width / 2);
        let room = width.saturating_sub(prompt.width() + hint.width() + 3);
        let text = tail_fit(&app.command.text, room);
        let pad = " ".repeat(width.saturating_sub(prompt.width() + text.width() + 1 + hint.width()));
        return Line::from(vec![prompt, Span::raw(text), Span::styled("▏", Style::default().fg(ACCENT)), Span::raw(pad), Span::styled(hint, dim())]);
    }
    let mut spans = Vec::new();
    let mut used = 0;
    if let Some(toast) = &app.toast {
        let style = if toast.starts_with('✗') {
            Style::default().fg(Color::Red)
        } else if toast.starts_with('✓') {
            Style::default().fg(Color::Green)
        } else if toast.starts_with('!') {
            Style::default().fg(Color::Yellow)
        } else {
            Style::default()
        };
        let shown = head_fit(toast.trim(), width * 3 / 5);
        used = shown.width() + 3;
        spans.push(Span::styled(shown, style.add_modifier(Modifier::BOLD)));
        spans.push(Span::raw("   "));
    }
    spans.push(Span::styled(key_hint(app, width.saturating_sub(used)), dim()));
    Line::from(spans)
}

fn jobs(app: &App) -> Vec<Line<'static>> {
    app.jobs
        .values()
        .map(|j| {
            let pct = j.length.filter(|l| *l > 0).map(|l| format!(" {}%", j.position * 100 / l)).unwrap_or_default();
            Line::from(Span::styled(format!(" ⠿ {}{pct}", j.message), Style::default().fg(Color::Yellow)))
        })
        .collect()
}

fn centered(area: Rect, width: u16, height: u16) -> Rect {
    let w = width.min(area.width);
    let h = height.min(area.height);
    Rect { x: area.x + (area.width - w) / 2, y: area.y + (area.height - h) / 2, width: w, height: h }
}

/// 왼쪽 테두리에 걸친 두 칸 글자를 지운다 (남으면 터미널에서 테두리를 덮는다)
fn clear_wide_edge(f: &mut Frame, r: Rect) {
    if r.x > 0 {
        let buf = f.buffer_mut();
        for y in r.y..r.y + r.height {
            if buf[(r.x - 1, y)].symbol().width() > 1 {
                buf[(r.x - 1, y)].set_symbol(" ");
            }
        }
    }
}

/// 가운데 창. 높이는 접힌 줄까지 세어 마지막 줄(키 안내)이 잘리지 않게
fn draw_popup(f: &mut Frame, title: &str, lines: Vec<Line>, area: Rect) {
    let width = (area.width * 2 / 3).max(50).min(area.width);
    let inner_w = width.saturating_sub(2) as usize;
    let height: usize = lines.iter().map(|l| wrap(&l.to_string(), inner_w).len()).sum::<usize>() + 2;
    let r = centered(area, width, height as u16);
    f.render_widget(Clear, r);
    clear_wide_edge(f, r);
    f.render_widget(Paragraph::new(lines).wrap(Wrap { trim: false }).block(panel(title.to_string(), true)), r);
}

/// [더 보기] 창: 덜 쓰는 버튼 목록
fn draw_more(f: &mut Frame, app: &mut App, area: Rect) {
    let Some(cursor) = app.more else { return };
    let items = app.more_buttons();
    let width = (items.iter().map(|b| button_text(b).width()).max().unwrap_or(10) as u16 + 6).max(24).min(area.width);
    let r = centered(area, width, items.len() as u16 + 2);
    f.render_widget(Clear, r);
    clear_wide_edge(f, r);
    let block = panel(t!("tui.btn.more"), true);
    let inner = block.inner(r);
    f.render_widget(block, r);
    app.areas.more.clear();
    for (i, b) in items.iter().enumerate() {
        let row = Rect { y: inner.y + i as u16, height: 1, ..inner };
        if row.y >= inner.y + inner.height {
            break;
        }
        let style = if i == cursor { highlight(true) } else if b.enabled { Style::default() } else { dim() };
        let label = format!(" {}", button_text(b));
        let pad = " ".repeat((row.width as usize).saturating_sub(label.width()));
        f.render_widget(Paragraph::new(Span::styled(format!("{label}{pad}"), style)), row);
        app.areas.more.push(row);
    }
}

/// 쪽 탭 한 줄. 오른쪽 끝에 무엇이든 찾기 안내와 GUI로 전환 (GUI를 쓸 수 있을 때만). 지금 쪽이 보이게 앞을 자른다
fn draw_tabs(f: &mut Frame, app: &mut App, area: Rect) {
    let mut area = area;
    if app.can_switch_to_gui() {
        let label = format!(" ⇄ {} F9 ", t!("tui.gui_switch"));
        let w = (label.width() as u16).min(area.width / 2);
        let r = Rect { x: area.x + area.width - w, y: area.y, width: w, height: 1 };
        f.render_widget(Paragraph::new(Span::styled(label, Style::default().fg(Color::Black).bg(Color::Magenta).add_modifier(Modifier::BOLD))), r);
        app.areas.gui = r;
        area.width -= w + 1;
    }
    let find = format!("Ctrl+K {}", t!("tui.find.title"));
    let mut spans = Vec::new();
    let focused = app.focus == Focus::Tabs;
    for p in Page::ALL.iter() {
        let text = format!(" {} ", p.title());
        spans.push(if *p == app.page {
            Span::styled(text, if focused { highlight(true) } else { Style::default().fg(ACCENT).add_modifier(Modifier::BOLD | Modifier::UNDERLINED) })
        } else {
            Span::styled(text, dim())
        });
        spans.push(Span::raw(" "));
    }
    let tabs_w: usize = spans.iter().map(|s| s.content.width()).sum();
    if tabs_w + find.width() + 2 <= area.width as usize {
        let r = Rect { x: area.x + area.width - find.width() as u16, width: find.width() as u16, ..area };
        f.render_widget(Paragraph::new(Span::styled(find, dim())), r);
    }
    let at = app.page.slot() * 2;
    let before: usize = spans[..at].iter().map(|s| s.content.width()).sum();
    let mine = spans[at].content.width();
    let skip = (before + mine).saturating_sub(area.width as usize);
    // 앞을 자른 만큼 밀어서 보이는 부분만 클릭 자리로 남긴다
    let mut x = 0usize;
    let mut rects = Vec::new();
    for (i, s) in spans.iter().enumerate() {
        let (start, end) = (x, x + s.content.width());
        x = end;
        if i % 2 == 1 {
            continue;
        }
        let (l, r) = (start.max(skip) - skip, (end.saturating_sub(skip)).min(area.width as usize));
        rects.push(if r <= l { Rect::default() } else { Rect { x: area.x + l as u16, y: area.y, width: (r - l) as u16, height: 1 } });
    }
    app.areas.tabs = rects;
    let w = (tabs_w as u16).min(area.width);
    f.render_widget(Paragraph::new(Line::from(spans)).scroll((0, skip as u16)), Rect { width: w, ..area });
}

/// 상세 탭 줄과 (오른쪽에 들어가면) 버튼. 남은 자리를 돌려준다
fn tab_row(f: &mut Frame, app: &mut App, area: Rect) -> Rect {
    if area.height == 0 {
        return area;
    }
    let tabs = app.page.tabs();
    let focused = app.focus == Focus::Detail;
    let mut x = area.x;
    app.areas.detail.clear();
    for (i, key) in tabs.iter().enumerate() {
        let mut label = format!(" {} ", app.page.tab_label(key));
        // 접속자 수는 탭 이름에
        if *key == "players" {
            if let Some(n) = app.selected_server().and_then(|v| v.pointer("/players/online")).and_then(Value::as_u64).filter(|n| *n > 0) {
                label = format!(" {} {n} ", app.page.tab_label(key));
            }
        }
        let w = label.width() as u16;
        if x + w > area.x + area.width {
            app.areas.detail.push(Rect::default());
            continue;
        }
        let style = if i == app.tab_index() {
            // 채운 칸은 지금 화살표가 움직이는 곳 하나뿐: 고른 탭은 밑줄로만
            selected_tab(focused)
        } else {
            Style::default()
        };
        let r = Rect { x, y: area.y, width: w, height: 1 };
        f.render_widget(Paragraph::new(Span::styled(label, style)), r);
        app.areas.detail.push(r);
        x += w + 1;
    }
    // 버튼은 탭 오른쪽에 들어가면 같은 줄, 아니면 다음 줄
    let buttons = app.buttons();
    if buttons.is_empty() {
        app.areas.buttons.clear();
        return Rect { y: area.y + 1, height: area.height - 1, ..area };
    }
    let need: u16 = buttons.iter().map(|b| button_text(b).width() as u16 + 3).sum();
    let used = x - area.x;
    if tabs.is_empty() || used + need < area.width {
        let bx = if tabs.is_empty() { area.x } else { area.x + area.width.saturating_sub(need) };
        draw_buttons(f, app, &buttons, Rect { x: bx, y: area.y, width: area.x + area.width - bx, height: 1 });
        Rect { y: area.y + 1, height: area.height - 1, ..area }
    } else {
        let below = Rect { y: area.y + 1, height: area.height - 1, ..area };
        let rows = draw_buttons(f, app, &buttons, below);
        Rect { y: below.y + rows, height: below.height.saturating_sub(rows), ..below }
    }
}

/// 버튼을 그리고 쓴 줄 수를 돌려준다
fn draw_buttons(f: &mut Frame, app: &mut App, buttons: &[Button], area: Rect) -> u16 {
    if area.height == 0 {
        app.areas.buttons.clear();
        return 0;
    }
    let (rows, rects) = layout_buttons(buttons, area);
    let focused = app.focus == Focus::Actions;
    for (i, (b, r)) in buttons.iter().zip(&rects).enumerate() {
        if r.y >= area.y + area.height {
            break;
        }
        let style = if focused && i == app.button {
            highlight(true)
        } else if !b.enabled {
            dim()
        } else {
            match b.line.split(' ').next() {
                Some("stop") => Style::default().fg(Color::Red).bg(Color::Rgb(63, 29, 29)),
                Some("start") => Style::default().fg(Color::Green).bg(Color::Rgb(5, 46, 22)),
                _ => Style::default().fg(Color::White).bg(Color::DarkGray),
            }
        };
        // format!의 폭은 글자 수라서 한글이 넘친다: 화면 폭으로 직접 가운데 맞춤
        let label = button_text(b);
        let pad = (r.width as usize).saturating_sub(label.width());
        let label = format!("{}{label}{}", " ".repeat(pad / 2), " ".repeat(pad - pad / 2));
        f.render_widget(Paragraph::new(Span::styled(label, style)), *r);
    }
    app.areas.buttons = clickable(rects, area);
    rows.min(area.height)
}

fn rule(f: &mut Frame, area: Rect) {
    f.render_widget(Paragraph::new(Span::styled("─".repeat(area.width as usize), dim())), Rect { height: 1, ..area });
}

/// 좁은 화면에서 목록 대신 상세를 볼 때 맨 위 한 줄
fn back_line(app: &App) -> Line<'static> {
    let (what, name) = match app.page {
        Page::Servers => (t!("tui.back.servers"), app.selected_server().map(|v| s(v, "name").to_string())),
        Page::Discord => (t!("tui.back.guilds"), app.selected_guild().map(|g| s(g, "guildName").to_string())),
        _ => (t!("tui.back.list"), app.selected()),
    };
    Line::from(vec![Span::styled(format!("‹ {what}  Esc"), dim()), Span::raw("   "), Span::styled(name.unwrap_or_default(), Style::default().add_modifier(Modifier::BOLD))])
}

/// 목록과 상세를 나눈다. 좁으면 지금 칸에 따라 하나만 (목록 → 상세로 한 칸씩)
fn split_or_stack(app: &mut App, area: Rect, width: u16) -> (Option<Rect>, Option<Rect>) {
    if app.narrow {
        if matches!(app.focus, Focus::Tabs | Focus::List) || (app.focus == Focus::Command && matches!(app.command_back(), Focus::Tabs | Focus::List)) {
            return (Some(area), None);
        }
        return (None, Some(area));
    }
    let (l, r) = split_list(area, width);
    (Some(l), Some(r))
}

fn draw_servers(f: &mut Frame, app: &mut App, area: Rect) {
    let (left, right) = split_or_stack(app, area, list_width(area, 28, 24, 34));
    if let Some(left) = left {
        let inner_w = left.width.saturating_sub(2) as usize;
        let items: Vec<ListItem<'static>> = app
            .snap
            .servers
            .iter()
            .map(|v| {
                let (dot, color) = status_dot(s(v, "status"));
                let name = head_fit(s(v, "name"), inner_w.saturating_sub(4));
                let game = head_fit(s(v, "module"), inner_w.saturating_sub(name.width() + 6));
                let pad = " ".repeat(inner_w.saturating_sub(3 + name.width() + game.width()));
                let mut sub: Vec<String> = Vec::new();
                if matches!(s(v, "status"), "running" | "starting") {
                    if let Some(p) = crate::output::players(v) {
                        sub.push(t!("tui.list.players", count = p));
                    }
                    let up = uptime(v);
                    if !up.is_empty() {
                        sub.push(up);
                    }
                } else {
                    sub.push(status_label(s(v, "status")));
                }
                if let Some(p) = v.get("port").filter(|p| !p.is_null()) {
                    sub.push(format!("{} {p}", t!("tui.col.port")));
                }
                ListItem::new(vec![
                    Line::from(vec![Span::styled(format!("{dot} "), Style::default().fg(color)), Span::styled(name, Style::default().add_modifier(Modifier::BOLD)), Span::raw(pad), Span::styled(game, dim())]),
                    Line::from(Span::styled(format!("  {}", head_fit(&sub.join(" · "), inner_w.saturating_sub(2))), dim())),
                    Line::from(""),
                ])
            })
            .collect();
        let title = format!("{} {}", t!("tui.page.servers"), app.snap.servers.len());
        draw_list(f, app, left, title, items, t!("tui.no_servers"));
        // 목록 아래 테두리에 [+ 새 서버]
        let label = format!(" + {}  n ", t!("tui.btn.server_add"));
        let add = Rect { x: left.x + 1, y: left.y + left.height.saturating_sub(1), width: (label.width() as u16).min(left.width.saturating_sub(2)), height: 1 };
        f.render_widget(Paragraph::new(Span::styled(label, Style::default().fg(ACCENT))), add);
        app.areas.add = add;
    }
    let Some(right) = right else { return };
    let focused = matches!(app.focus, Focus::Detail | Focus::Actions | Focus::Content);
    let Some(v) = app.selected_server().cloned() else {
        let block = panel(t!("tui.detail"), focused);
        let inner = block.inner(right);
        f.render_widget(block, right);
        f.render_widget(Paragraph::new(Span::styled(t!("tui.no_servers"), dim())).wrap(Wrap { trim: false }), inner);
        return;
    };
    let (dot, color) = status_dot(s(&v, "status"));
    let block = panel(format!("{dot} {}  {}", s(&v, "name"), status_label(s(&v, "status"))), focused).title_style(Style::default().fg(color).add_modifier(Modifier::BOLD));
    let mut inner = block.inner(right);
    f.render_widget(block, right);
    if app.narrow && inner.height > 4 {
        f.render_widget(Paragraph::new(back_line(app)), Rect { height: 1, ..inner });
        inner = Rect { y: inner.y + 1, height: inner.height - 1, ..inner };
    }
    if inner.height < 4 {
        return;
    }
    let rest = tab_row(f, app, inner);
    if rest.height < 3 {
        return;
    }
    let info = [
        (String::new(), s(&v, "module").to_string()),
        (t!("tui.col.port"), v.get("port").filter(|p| !p.is_null()).map(|p| p.to_string()).unwrap_or_default()),
        (t!("tui.col.uptime"), uptime(&v)),
    ];
    f.render_widget(Paragraph::new(inline_kv(&info)), Rect { height: 1, ..rest });
    rule(f, Rect { y: rest.y + 1, ..rest });
    let body = Rect { y: rest.y + 2, height: rest.height - 2, ..rest };
    draw_content(f, app, body);
}

fn draw_discord(f: &mut Frame, app: &mut App, area: Rect) {
    let linked = app.snap.agent.as_ref().and_then(|a| a.get("running")).and_then(Value::as_bool) == Some(true);
    let (left, right) = split_or_stack(app, area, list_width(area, 28, 24, 34));
    if let Some(left) = left {
        let inner_w = left.width.saturating_sub(2) as usize;
        let items: Vec<ListItem<'static>> = app
            .snap
            .guilds
            .iter()
            .map(|g| {
                let name = if s(g, "guildName").is_empty() { s(g, "guildId") } else { s(g, "guildName") };
                ListItem::new(vec![
                    Line::from(Span::styled(head_fit(name, inner_w), Style::default().add_modifier(Modifier::BOLD))),
                    Line::from(Span::styled(head_fit(&format!(" {}", s(g, "guildId")), inner_w), dim())),
                    Line::from(""),
                ])
            })
            .collect();
        let title = format!("{} {}", t!("tui.guilds"), app.snap.guilds.len());
        draw_list(f, app, left, title, items, t!("tui.empty"));
        // 목록 아래 테두리에 연결 상태
        let state = if linked { t!("tui.discord_on") } else { t!("tui.discord_off") };
        let label = format!(" {state} ");
        let r = Rect { x: left.x + 1, y: left.y + left.height.saturating_sub(1), width: (label.width() as u16).min(left.width.saturating_sub(2)), height: 1 };
        f.render_widget(Paragraph::new(Span::styled(label, Style::default().fg(if linked { Color::Green } else { Color::DarkGray }))), r);
    }
    let Some(right) = right else { return };
    let focused = matches!(app.focus, Focus::Detail | Focus::Actions | Focus::Content);
    let Some(g) = app.selected_guild().cloned() else {
        // 디스코드 서버가 없으면 연결 상태와 버튼만
        let block = panel(t!("tui.page.discord"), focused);
        let inner = block.inner(right);
        f.render_widget(block, right);
        let rest = tab_row_buttons_only(f, app, inner);
        f.render_widget(Paragraph::new(discord_lines(app)).wrap(Wrap { trim: false }), rest);
        return;
    };
    let name = if s(&g, "guildName").is_empty() { s(&g, "guildId") } else { s(&g, "guildName") };
    let block = panel(name.to_string(), focused);
    let mut inner = block.inner(right);
    f.render_widget(block, right);
    if app.narrow && inner.height > 4 {
        f.render_widget(Paragraph::new(back_line(app)), Rect { height: 1, ..inner });
        inner = Rect { y: inner.y + 1, height: inner.height - 1, ..inner };
    }
    let rest = tab_row(f, app, inner);
    if rest.height < 2 {
        return;
    }
    rule(f, rest);
    draw_content(f, app, Rect { y: rest.y + 1, height: rest.height - 1, ..rest });
}

/// 탭 없는 칸의 버튼 줄
fn tab_row_buttons_only(f: &mut Frame, app: &mut App, area: Rect) -> Rect {
    let buttons = app.buttons();
    app.areas.detail.clear();
    let rows = draw_buttons(f, app, &buttons, area);
    let used = (rows + 1).min(area.height);
    Rect { y: area.y + used, height: area.height - used, ..area }
}

fn discord_lines(app: &App) -> Vec<Line<'static>> {
    let a = app.snap.agent.clone().unwrap_or(Value::Null);
    let running = a.get("running").and_then(Value::as_bool) == Some(true);
    let mut rows = vec![
        (t!("tui.discord.relay"), if running { t!("tui.discord_on") } else { t!("tui.discord_off") }),
        (t!("tui.discord.paired"), crate::output::yes_no(a.get("hasToken").and_then(Value::as_bool) == Some(true))),
        (t!("tui.discord.handled"), a.get("commandsHandled").map(|v| v.to_string()).unwrap_or_else(|| "-".into())),
    ];
    if let Some(at) = a.get("lastHeartbeatAt").and_then(Value::as_u64) {
        rows.push((t!("tui.discord.heartbeat"), super::data::clock(at)));
    }
    if !s(&a, "lastError").is_empty() {
        rows.push((t!("tui.discord.error"), s(&a, "lastError").to_string()));
    }
    let mut lines = kv_lines(&rows);
    if a.get("hasToken").and_then(Value::as_bool) != Some(true) {
        lines.push(Line::from(""));
        lines.push(Line::from(Span::styled(t!("tui.discord.pair_hint"), Style::default().fg(Color::Yellow))));
    }
    lines
}

fn draw_addons(f: &mut Frame, app: &mut App, area: Rect) {
    let top = Rect { height: 1, ..area };
    let modules = app.tab_index() == 0;
    // 탭은 목록 위 (모듈 ↔ 익스텐션)
    let saved_buttons = std::mem::take(&mut app.areas.buttons);
    {
        let tabs = app.page.tabs();
        let focused = app.focus == Focus::Detail;
        let mut x = top.x + 1;
        app.areas.detail.clear();
        for (i, key) in tabs.iter().enumerate() {
            let label = format!(" {} ", app.page.tab_label(key));
            let w = label.width() as u16;
            let style = if i == app.tab_index() {
                selected_tab(focused)
            } else {
                Style::default()
            };
            let r = Rect { x, y: top.y, width: w.min(top.width), height: 1 };
            f.render_widget(Paragraph::new(Span::styled(label, style)), r);
            app.areas.detail.push(r);
            x += w + 1;
        }
    }
    app.areas.buttons = saved_buttons;
    let body = Rect { y: area.y + 1, height: area.height.saturating_sub(1), ..area };
    let list = if modules { app.snap.modules.clone() } else { app.snap.extensions.clone() };
    let id_key = if modules { "name" } else { "id" };
    let (left, right) = split_or_stack(app, body, list_width(body, 32, 22, 40));
    if let Some(left) = left {
        let items = list
            .iter()
            .map(|v| {
                let on = v.get("enabled").and_then(Value::as_bool);
                let mark = match on {
                    Some(true) => Span::styled("● ", Style::default().fg(Color::Green)),
                    Some(false) => Span::styled("○ ", dim()),
                    None => Span::raw(""),
                };
                ListItem::new(Line::from(vec![mark, Span::raw(s(v, id_key).to_string()), Span::styled(format!("  {}", s(v, "version")), dim())]))
            })
            .collect();
        let title = if modules { t!("tui.page.modules") } else { t!("tui.page.extensions") };
        draw_list(f, app, left, title, items, t!("tui.empty"));
    }
    let Some(right) = right else { return };
    let picked = list.get(app.selected_index());
    let (name, rows) = match picked {
        Some(v) => (s(v, id_key).to_string(), if modules { module_rows(app, v) } else { extension_rows(app, v) }),
        None => (t!("tui.detail"), Vec::new()),
    };
    let block = panel(name, app.focus == Focus::Actions);
    let inner = block.inner(right);
    f.render_widget(block, right);
    // 탭 자리는 위에서 그렸다: 버튼 줄이 지우지 않게 맡겨 둔다
    let tabs = std::mem::take(&mut app.areas.detail);
    let rest = tab_row_buttons_only(f, app, inner);
    app.areas.detail = tabs;
    f.render_widget(Paragraph::new(kv_lines(&rows)).wrap(Wrap { trim: false }), rest);
}

fn draw_updates(f: &mut Frame, app: &mut App, area: Rect) {
    let block = panel(t!("tui.page.updates"), app.focus == Focus::Actions);
    let inner = block.inner(area);
    f.render_widget(block, area);
    let rest = tab_row_buttons_only(f, app, inner);
    let lines = update_lines(app);
    f.render_widget(Paragraph::new(lines).wrap(Wrap { trim: false }), rest);
}

fn draw_settings(f: &mut Frame, app: &mut App, area: Rect) {
    let focused = matches!(app.focus, Focus::Detail | Focus::Actions | Focus::Content);
    let block = panel(t!("tui.page.settings"), focused);
    let inner = block.inner(area);
    f.render_widget(block, area);
    if inner.height < 4 {
        return;
    }
    let rest = tab_row(f, app, inner);
    let h = app.snap.health.clone().unwrap_or(Value::Null);
    let info = [
        (t!("tui.settings.daemon"), if h.is_null() { t!("tui.daemon_down") } else { format!("v{} · pid {}", s(&h, "version"), h.get("pid").map(|p| p.to_string()).unwrap_or_default()) }),
        (t!("tui.settings.port"), app.port.to_string()),
        (t!("tui.settings.language"), crate::i18n::lang().to_string()),
    ];
    f.render_widget(Paragraph::new(inline_kv(&info)), Rect { height: 1, ..rest });
    if rest.height < 3 {
        return;
    }
    rule(f, Rect { y: rest.y + 1, ..rest });
    draw_content(f, app, Rect { y: rest.y + 2, height: rest.height - 2, ..rest });
}

/// 상세 탭의 내용. 맨 아래 줄은 같은 뜻의 CLI 명령(폼은 바뀐 칸이 있으면 저장 줄)
fn draw_content(f: &mut Frame, app: &mut App, area: Rect) {
    if area.height == 0 {
        return;
    }
    app.areas.view = area;
    let focused = app.focus == Focus::Content;
    let tab = app.tab_key();
    if app.page == Page::Servers && tab == "console" {
        draw_console(f, app, area);
        return;
    }
    if app.page == Page::Settings && tab == "daemon_log" {
        let lines: Vec<Line<'static>> = if app.snap.daemon_log.is_empty() {
            vec![Line::from(Span::styled(t!("tui.console_empty"), dim()))]
        } else {
            app.snap.daemon_log.iter().flat_map(|l| wrap_spans(daemon_line(l), area.width as usize)).collect()
        };
        draw_lines(f, area, lines, &mut app.view_scroll);
        return;
    }
    let body = Rect { height: area.height.saturating_sub(1), ..area };
    let bottom = Rect { y: area.y + area.height - 1, height: 1, ..area };
    let same = app.same_command().map(|c| format!("{} saba-chan {c}", t!("tui.same_command")));
    if !app.pane_is_current() {
        f.render_widget(Paragraph::new(Span::styled(t!("tui.pane.loading"), dim())), body);
        f.render_widget(Paragraph::new(Span::styled(head_fit(&same.unwrap_or_default(), bottom.width as usize), dim())), bottom);
        return;
    }
    let edit = app.edit_prefix().is_some();
    let pane_scroll = app.pane.scroll;
    let cursor = app.pane.cursor;
    match &mut app.pane.state {
        PaneState::Loading => f.render_widget(Paragraph::new(Span::styled(t!("tui.pane.loading"), dim())), body),
        PaneState::Failed(why) => f.render_widget(Paragraph::new(Span::styled(format!("! {why}"), Style::default().fg(Color::Yellow))).wrap(Wrap { trim: false }), body),
        PaneState::Form(form, _) => {
            let dirty = form.fields.iter().filter(|x| x.dirty()).count();
            draw_form(f, form, body, focused);
            let line = if dirty > 0 {
                Line::from(vec![
                    Span::styled(t!("tui.form.changed", count = dirty), Style::default().fg(Color::Yellow).add_modifier(Modifier::BOLD)),
                    Span::styled("  ·  ", dim()),
                    Span::styled(format!(" {} ", t!("tui.form.save")), Style::default().fg(Color::Black).bg(ACCENT).add_modifier(Modifier::BOLD)),
                    Span::raw("  "),
                    Span::styled(format!(" {} ", t!("tui.form.revert")), Style::default().fg(Color::White).bg(Color::DarkGray)),
                    Span::styled(format!("   {}", t!("tui.form.keys")), dim()),
                ])
            } else {
                Line::from(Span::styled(same.unwrap_or_default(), dim()))
            };
            f.render_widget(Paragraph::new(line), bottom);
            return;
        }
        PaneState::Rows(rows) => {
            let lines = table_lines(tab, rows, cursor, focused, body.width as usize);
            let empty = lines.len() <= 1;
            f.render_widget(Paragraph::new(lines), body);
            if empty {
                let r = Rect { y: body.y + 1, height: 1, ..body };
                f.render_widget(Paragraph::new(Span::styled(t!("tui.empty"), dim())), r);
            }
        }
        PaneState::Text(lines) => {
            let shown: Vec<Line<'static>> = if lines.is_empty() {
                vec![Line::from(Span::styled(if app.capture.is_some() { t!("tui.pane.loading") } else { t!("tui.empty") }, dim()))]
            } else {
                lines.iter().flat_map(|l| wrap(l, body.width as usize)).map(|l| Line::from(output_span(l))).collect()
            };
            if tab == "log" {
                let mut scroll = pane_scroll;
                draw_lines(f, body, shown, &mut scroll);
                app.pane.scroll = scroll;
            } else {
                let start = pane_scroll.min(shown.len().saturating_sub(body.height as usize));
                app.pane.scroll = start;
                f.render_widget(Paragraph::new(shown[start..].to_vec()), body);
            }
        }
    }
    let mut hint = same.unwrap_or_default();
    let keys = match (app.page, tab) {
        (Page::Servers, "backups") => t!("tui.keys_backups"),
        (Page::Servers, "players") => t!("tui.keys_players"),
        _ if edit => t!("tui.keys_edit"),
        _ => String::new(),
    };
    if !keys.is_empty() {
        hint = format!("{keys}   {hint}");
    }
    f.render_widget(Paragraph::new(Span::styled(head_fit(&hint, bottom.width as usize), dim())), bottom);
}

/// 명령 출력 한 줄의 색 (✓ ✗ !)
fn output_span(l: String) -> Span<'static> {
    let style = if l.starts_with("› ") {
        Style::default().fg(ACCENT)
    } else if l.starts_with('✗') {
        Style::default().fg(Color::Red)
    } else if l.starts_with('✓') {
        Style::default().fg(Color::Green)
    } else if l.starts_with('!') {
        Style::default().fg(Color::Yellow)
    } else {
        Style::default()
    };
    Span::styled(l, style)
}

/// 표의 칸: (머리, 값 꺼내기)
type Column = (String, Box<dyn Fn(&Value) -> String>);

/// 접속자·백업 표
fn table_lines(tab: &str, rows: &[Value], cursor: usize, focused: bool, width: usize) -> Vec<Line<'static>> {
    let cols: Vec<Column> = if tab == "backups" {
        vec![
            (t!("col.name"), Box::new(|v: &Value| s(v, "name").to_string())),
            (t!("tui.col.kind"), Box::new(|v: &Value| s(v, "kind").to_string())),
            (t!("tui.col.created"), Box::new(|v: &Value| v.get("created").and_then(Value::as_u64).map(|t| super::data::clock_fmt(t, "%m-%d %H:%M")).unwrap_or_else(|| s(v, "created").to_string()))),
            (t!("tui.col.size"), Box::new(|v: &Value| v.get("size").and_then(Value::as_f64).map(crate::cmd::monitor::bytes).unwrap_or_default())),
        ]
    } else {
        vec![(t!("col.name"), Box::new(|v: &Value| s(v, "name").to_string()))]
    };
    let cells: Vec<Vec<String>> = rows.iter().map(|r| cols.iter().map(|(_, c)| c(r)).collect()).collect();
    let widths: Vec<usize> = cols.iter().enumerate().map(|(i, (h, _))| cells.iter().map(|r| r[i].width()).chain([h.width()]).max().unwrap_or(0) + 2).collect();
    let line = |vals: Vec<String>| -> String {
        let mut out = String::new();
        for (v, w) in vals.iter().zip(&widths) {
            out.push_str(v);
            out.push_str(&" ".repeat(w.saturating_sub(v.width())));
        }
        head_fit(&out, width)
    };
    let mut out = vec![Line::from(Span::styled(line(cols.iter().map(|(h, _)| h.clone()).collect()), dim().add_modifier(Modifier::BOLD)))];
    for (i, r) in cells.into_iter().enumerate() {
        let text = line(r);
        let pad = " ".repeat(width.saturating_sub(text.width()));
        out.push(Line::from(Span::styled(format!("{text}{pad}"), if i == cursor && focused { highlight(true) } else { Style::default() })));
    }
    out
}

/// 서버 콘솔: 큰 출력, 입력 줄, 치는 동안 게임 명령 제안
fn draw_console(f: &mut Frame, app: &mut App, body: Rect) {
    if body.height < 2 {
        return;
    }
    let out = Rect { height: body.height.saturating_sub(1), ..body };
    let Some(v) = app.selected_server().cloned() else { return };
    let id = s(&v, "id");
    let lines: Vec<Line<'static>> = match &app.snap.console {
        Some((cid, lines)) if cid == id && !lines.is_empty() => lines.iter().flat_map(|l| wrap_spans(console_line(l), out.width as usize)).collect(),
        _ => vec![Line::from(Span::styled(t!("tui.console_empty"), dim()))],
    };
    draw_lines(f, out, lines, &mut app.view_scroll);
    // 입력 줄
    let input_row = Rect { y: body.y + body.height - 1, height: 1, ..body };
    let typing = app.focus == Focus::Content;
    let prompt = Span::styled(format!("{} › ", s(&v, "name")), Style::default().fg(if typing { ACCENT } else { Color::DarkGray }).add_modifier(Modifier::BOLD));
    let line = if typing {
        let room = (input_row.width as usize).saturating_sub(prompt.width() + 1);
        let mut spans = vec![prompt, Span::raw(tail_fit(&app.server_input.text, room)), Span::styled("▏", Style::default().fg(ACCENT))];
        if !app.suggestions().is_empty() {
            spans.push(Span::styled(format!("   {}", t!("tui.suggest_keys")), dim()));
        }
        Line::from(spans)
    } else {
        Line::from(vec![prompt, Span::styled(t!("tui.console_input_hint"), dim())])
    };
    f.render_widget(Paragraph::new(line), input_row);
    // 제안: 입력 줄 바로 위 작은 창
    let found = app.suggestions();
    if typing && !found.is_empty() {
        let w = (body.width.saturating_sub(2)).min(64);
        let h = found.len() as u16 + 2;
        if out.height < h {
            return;
        }
        let r = Rect { x: body.x + 1, y: input_row.y - h, width: w, height: h };
        f.render_widget(Clear, r);
        clear_wide_edge(f, r);
        let block = panel(String::new(), false);
        let inner = block.inner(r);
        f.render_widget(block, r);
        let pick = app.suggest.min(found.len() - 1);
        for (i, (name, usage, about, risk)) in found.iter().enumerate() {
            let row = Rect { y: inner.y + i as u16, height: 1, ..inner };
            let left = head_fit(&format!(" {name}{usage}"), (inner.width as usize) * 3 / 5);
            let about = if *risk == crate::catalog::Risk::Danger { format!("{about} · {}", t!("tui.risk_danger")) } else { about.clone() };
            let pad = " ".repeat((inner.width as usize * 3 / 5 + 1).saturating_sub(left.width()));
            let right = head_fit(&about, (inner.width as usize).saturating_sub(left.width() + pad.width()));
            let base = if i == pick { Style::default().bg(Color::Rgb(30, 58, 95)).add_modifier(Modifier::BOLD) } else { Style::default() };
            let about_style = if *risk == crate::catalog::Risk::Danger { base.fg(Color::Red) } else { base.fg(Color::DarkGray) };
            let fill = " ".repeat((inner.width as usize).saturating_sub(left.width() + pad.width() + right.width()));
            f.render_widget(Paragraph::new(Line::from(vec![Span::styled(left, base), Span::styled(pad, base), Span::styled(right, about_style), Span::styled(fill, base)])), row);
        }
    }
}

/// 설정 폼 그리기: 찾기 줄, 묶음 머리줄, 칸(이름 · 값 · 바뀐 것), 고른 칸의 설명
fn draw_form(f: &mut Frame, form: &mut Form, area: Rect, focused: bool) {
    if area.height < 2 {
        return;
    }
    // 찾기 줄
    let groups = form.groups().iter().map(|g| group_label(g)).collect::<Vec<_>>().join(" · ");
    let find = if form.filtering || !form.filter.is_empty() {
        Line::from(vec![Span::styled("/ ", Style::default().fg(ACCENT)), Span::raw(form.filter.clone()), Span::styled(if form.filtering { "▏" } else { "" }, Style::default().fg(ACCENT))])
    } else {
        Line::from(vec![Span::styled("/ ", dim()), Span::styled(t!("tui.form.find"), dim()), Span::styled(format!("     {groups}"), dim())])
    };
    f.render_widget(Paragraph::new(find), Rect { height: 1, ..area });
    let rows = form.rows();
    let desc_h: u16 = 1;
    let list_h = area.height.saturating_sub(2 + desc_h) as usize;
    if list_h == 0 {
        return;
    }
    // 고른 줄이 보이게
    if form.cursor < form.scroll {
        form.scroll = form.cursor;
    } else if form.cursor >= form.scroll + list_h {
        form.scroll = form.cursor + 1 - list_h;
    }
    let label_w = form.fields.iter().map(|x| x.label.width()).max().unwrap_or(10).clamp(10, 22);
    let w = area.width as usize;
    for (n, row) in rows.iter().enumerate().skip(form.scroll).take(list_h) {
        let y = area.y + 2 + (n - form.scroll) as u16;
        let r = Rect { y, height: 1, ..area };
        let selected = focused && n == form.cursor;
        let line = match row {
            Row::Group(g) => {
                let open = form.open.contains(g) || !form.filter.is_empty();
                let title = if g == "advanced" { format!("{} ({})", group_label(g), form.group_len(g)) } else { group_label(g) };
                let tail = if open { String::new() } else { format!("  ▸ {}", t!("tui.form.folded")) };
                let bar = "─".repeat(w.saturating_sub(title.width() + tail.width() + 3));
                let style = if selected { highlight(true) } else { Style::default().fg(ACCENT).add_modifier(Modifier::BOLD) };
                Line::from(vec![Span::styled(title, style), Span::styled(tail, dim()), Span::styled(format!("  {bar}"), dim())])
            }
            Row::Field(i) => field_line(&form.fields[*i], form, selected, label_w, w),
        };
        f.render_widget(Paragraph::new(line), r);
    }
    // 고른 칸의 설명 (재시작이 필요하면 덧붙인다)
    if let Some(cur) = form.current() {
        let mut text = cur.description.clone();
        if cur.restart && cur.dirty() {
            text = format!("{} {}", text, t!("tui.form.restart")).trim().to_string();
        }
        if form.show_keys && text.is_empty() {
            text = cur.key.clone();
        }
        if !text.is_empty() {
            let r = Rect { y: area.y + area.height - 1, height: 1, ..area };
            f.render_widget(Paragraph::new(Span::styled(head_fit(&format!("└ {text}"), w), dim())), r);
        }
    }
}

fn group_label(g: &str) -> String {
    crate::i18n::try_t(&format!("tui.form.group.{g}")).unwrap_or_else(|| g.to_string())
}

/// 폼의 칸 한 줄: `▌ 이름  값            ● 원래 → 새`
fn field_line(x: &super::form::Field, form: &Form, selected: bool, label_w: usize, width: usize) -> Line<'static> {
    let base = if selected { Style::default().bg(Color::Rgb(30, 58, 95)) } else { Style::default() };
    let mark = if x.dirty() { Span::styled("▌", base.fg(Color::Yellow)) } else { Span::styled(" ", base) };
    let mut label = x.label.clone();
    if form.show_keys {
        label = format!("{label} [{}]", x.key);
    }
    let label_shown = head_fit(&label, label_w.max(if form.show_keys { 30 } else { 0 }));
    let pad = " ".repeat((label_w + 1).saturating_sub(label_shown.width()));
    let editing = selected && form.editing.is_some();
    let unset = x.value.is_empty();
    let val_style = if unset { base.fg(Color::DarkGray) } else { base.add_modifier(Modifier::BOLD) };
    let mut spans = vec![mark, Span::styled(" ", base), Span::styled(label_shown, if selected { base.add_modifier(Modifier::BOLD) } else { base }), Span::styled(pad, base)];
    match &x.kind {
        _ if editing => {
            let text = form.editing.clone().unwrap_or_default();
            spans.push(Span::styled(format!(" {text}"), Style::default().bg(Color::Rgb(31, 41, 55))));
            spans.push(Span::styled("▏", Style::default().fg(ACCENT)));
        }
        Kind::Bool => {
            let on = x.shown(&x.value) == crate::output::yes_no(true);
            spans.push(Span::styled(if on { "[■] " } else { "[ ] " }, base.fg(if on { Color::Green } else { Color::DarkGray })));
            spans.push(Span::styled(x.shown(&x.value), val_style));
        }
        Kind::Choice(_) => {
            spans.push(Span::styled("‹ ", base.fg(Color::DarkGray)));
            spans.push(Span::styled(x.shown(&x.value), val_style));
            spans.push(Span::styled(" ›", base.fg(Color::DarkGray)));
        }
        Kind::Number { min, max } => {
            spans.push(Span::styled(format!(" {} ", x.shown(&x.value)), val_style.bg(Color::Rgb(31, 41, 55))));
            if let (Some(a), Some(b)) = (min, max) {
                spans.push(Span::styled(format!("  {a}–{b}"), base.fg(Color::DarkGray)));
            }
        }
        Kind::Text { .. } => spans.push(Span::styled(format!(" {} ", head_fit(&x.shown(&x.value), 40)), val_style.bg(Color::Rgb(31, 41, 55)))),
        Kind::ReadOnly => spans.push(Span::styled(head_fit(&x.shown(&x.value), 40), base.fg(Color::DarkGray))),
    }
    if unset && !editing && !x.default.is_empty() {
        spans.push(Span::styled(format!("  {}", t!("tui.form.default")), base.fg(Color::DarkGray)));
    }
    if x.dirty() {
        let from = if x.original.is_empty() { t!("tui.form.default") } else { x.shown(&x.original) };
        spans.push(Span::styled(format!("   ● {} → {}", head_fit(&from, 16), head_fit(&x.shown(&x.value), 16)), base.fg(Color::Yellow)));
    }
    let used: usize = spans.iter().map(|s| s.width()).sum();
    if selected && used < width {
        spans.push(Span::styled(" ".repeat(width - used), base));
    }
    Line::from(spans)
}

/// 새 서버 마법사: 왼쪽 단계, 오른쪽 지금 단계
fn draw_wizard(f: &mut Frame, app: &mut App, area: Rect) {
    let Some(w) = app.wizard.clone() else { return };
    let modules = app.snap.modules.clone();
    let block = panel(t!("tui.wiz.title"), true);
    let inner = block.inner(area);
    f.render_widget(block, area);
    if inner.height < 6 {
        return;
    }
    let side = if app.narrow { 0 } else { 26u16.min(inner.width / 3) };
    if side > 0 {
        for (i, key) in super::wizard::STEPS.iter().enumerate() {
            let y = inner.y + 1 + i as u16 * 2;
            if y + 1 >= inner.y + inner.height {
                break;
            }
            let (mark, color) = if i < w.step {
                ("✓", Color::Green)
            } else if i == w.step {
                ("▶", ACCENT)
            } else {
                ("○", Color::DarkGray)
            };
            let title = format!("{mark} {}. {}", i + 1, t!(&format!("tui.wiz.step.{key}")));
            let style = if i == w.step { Style::default().fg(color).add_modifier(Modifier::BOLD) } else { Style::default().fg(color) };
            f.render_widget(Paragraph::new(Span::styled(title, style)), Rect { x: inner.x + 1, y, width: side - 1, height: 1 });
            let value = match i {
                0 if w.step > 0 => if w.method == 0 { t!("tui.wiz.method_new") } else { t!("tui.wiz.method_import") },
                1 if w.step > 1 => w.module(&modules).map(super::wizard::Wizard::game_name).unwrap_or_default(),
                2 if w.step > 2 => w.name.clone(),
                3 if w.step > 3 => w.preset_label(&modules),
                _ => String::new(),
            };
            if !value.is_empty() {
                f.render_widget(Paragraph::new(Span::styled(head_fit(&format!("   {value}"), side as usize - 1), dim())), Rect { x: inner.x + 1, y: y + 1, width: side - 1, height: 1 });
            }
        }
        for y in inner.y..inner.y + inner.height {
            f.render_widget(Paragraph::new(Span::styled("│", dim())), Rect { x: inner.x + side, y, width: 1, height: 1 });
        }
    }
    let body = Rect { x: inner.x + side + 2, y: inner.y + 1, width: inner.width.saturating_sub(side + 3), height: inner.height.saturating_sub(1) };
    let bw = body.width as usize;
    let mut lines: Vec<Line<'static>> = Vec::new();
    let pick = |i: usize, text: String, about: String| -> Line<'static> {
        let sel = i == w.cursor;
        let style = if sel { highlight(true) } else { Style::default() };
        let mut spans = vec![Span::styled(format!(" {} ", if sel { "›" } else { " " }), style), Span::styled(text, style)];
        if !about.is_empty() {
            spans.push(Span::styled(format!("   {}", head_fit(&about, bw / 2)), dim()));
        }
        Line::from(spans)
    };
    let title = |key: &str| Line::from(Span::styled(t!(key), Style::default().add_modifier(Modifier::BOLD)));
    match w.step {
        0 => {
            lines.push(title("tui.wiz.ask_method"));
            lines.push(Line::from(""));
            lines.push(pick(0, t!("tui.wiz.method_new"), t!("tui.wiz.method_new_about")));
            lines.push(pick(1, t!("tui.wiz.method_import"), t!("tui.wiz.method_import_about")));
        }
        1 => {
            lines.push(title("tui.wiz.ask_game"));
            lines.push(Line::from(""));
            if modules.is_empty() {
                lines.push(Line::from(Span::styled(t!("tui.wiz.no_modules"), dim())));
            }
            for (i, m) in modules.iter().enumerate() {
                lines.push(pick(i, super::wizard::Wizard::game_name(m), s(m, "description").to_string()));
            }
        }
        2 => {
            lines.push(title("tui.wiz.ask_name"));
            lines.push(Line::from(""));
            let input = |label: String, text: &str, on: bool| -> Line<'static> {
                Line::from(vec![
                    Span::styled(format!("{label}  "), if on { Style::default().fg(ACCENT).add_modifier(Modifier::BOLD) } else { dim() }),
                    Span::styled(format!(" {text}"), Style::default().bg(Color::Rgb(31, 41, 55))),
                    Span::styled(if on { "▏" } else { " " }, Style::default().fg(ACCENT)),
                ])
            };
            lines.push(input(t!("tui.wiz.name"), &w.name, !w.on_dir));
            if w.method == 1 {
                lines.push(Line::from(""));
                lines.push(input(t!("tui.wiz.folder"), &w.dir, w.on_dir));
                lines.push(Line::from(Span::styled(t!("tui.wiz.folder_hint"), dim())));
            }
        }
        3 => {
            lines.push(title("tui.wiz.ask_preset"));
            lines.push(Line::from(""));
            lines.push(pick(0, t!("tui.wiz.no_preset"), String::new()));
            for (i, p) in w.presets(&modules).iter().enumerate() {
                let label = if s(p, "label").is_empty() { s(p, "id") } else { s(p, "label") };
                lines.push(pick(i + 1, label.to_string(), s(p, "description").to_string()));
            }
        }
        _ => {
            lines.push(Line::from(vec![Span::styled(t!("tui.wiz.ask_confirm"), Style::default().add_modifier(Modifier::BOLD)), Span::styled(format!("   {}", t!("tui.wiz.confirm_hint")), dim())]));
            lines.push(Line::from(""));
            for (i, row) in w.confirm_rows(&modules).iter().enumerate() {
                let sel = i == w.cursor;
                let base = if sel { Style::default().bg(Color::Rgb(30, 58, 95)) } else { Style::default() };
                let check = |on: bool| Span::styled(if on { "[■] " } else { "[ ] " }, base.fg(if on { Color::Green } else { Color::DarkGray }));
                let line = match row {
                    ConfirmRow::Summary(_, key, value) => {
                        let label = t!(key);
                        let pad = " ".repeat(12usize.saturating_sub(label.width()));
                        let mut spans = vec![Span::styled(format!(" {label}{pad}"), base.fg(Color::DarkGray)), Span::styled(value.clone(), base.add_modifier(Modifier::BOLD))];
                        if sel {
                            spans.push(Span::styled(format!("   {} ↵", t!("tui.wiz.fix")), base.fg(ACCENT)));
                        }
                        Line::from(spans)
                    }
                    ConfirmRow::Agreement(id, url) => Line::from(vec![
                        Span::styled(" ! ", base.fg(Color::Yellow)),
                        check(w.accepted.contains(id)),
                        Span::styled(t!("tui.wiz.agree", id = id.to_uppercase()), base),
                        Span::styled(format!("  {url}"), base.fg(Color::DarkGray)),
                    ]),
                    ConfirmRow::Start => Line::from(vec![Span::styled("   ", base), check(w.start), Span::styled(t!("tui.wiz.start_now"), base)]),
                    ConfirmRow::Create => {
                        lines.push(Line::from(""));
                        Line::from(vec![
                            Span::styled(format!("  {}  Enter  ", t!("tui.wiz.create")), if sel { Style::default().fg(Color::Black).bg(ACCENT).add_modifier(Modifier::BOLD) } else { Style::default().fg(Color::White).bg(Color::DarkGray) }),
                            Span::raw("   "),
                            Span::styled(format!("  {}  Esc  ", t!("tui.wiz.back")), Style::default().fg(Color::White).bg(Color::DarkGray)),
                        ])
                    }
                };
                lines.push(line);
            }
        }
    }
    lines.push(Line::from(""));
    lines.push(Line::from(Span::styled(format!("{} saba-chan {}", t!("tui.same_command"), w.command(&modules)), dim())));
    f.render_widget(Paragraph::new(lines).wrap(Wrap { trim: false }), body);
}

/// 무엇이든 찾기 창
fn draw_palette(f: &mut Frame, app: &mut App, area: Rect) {
    let Some(p) = &app.palette else { return };
    let results = p.results();
    let width = (area.width * 4 / 5).clamp(40.min(area.width), 96);
    let height = ((super::palette::SHOWN as u16) * 2 + 6).min(area.height);
    let r = Rect { x: area.x + (area.width - width) / 2, y: area.y + 2.min(area.height.saturating_sub(height)), width, height };
    f.render_widget(Clear, r);
    clear_wide_edge(f, r);
    let block = panel(t!("tui.find.title"), true);
    let inner = block.inner(r);
    f.render_widget(block, r);
    let w = inner.width as usize;
    let mut lines = vec![
        Line::from(vec![
            Span::styled("› ", Style::default().fg(ACCENT).add_modifier(Modifier::BOLD)),
            Span::styled(p.query.clone(), Style::default().add_modifier(Modifier::BOLD)),
            Span::styled("▏", Style::default().fg(ACCENT)),
            Span::styled(format!("   {}", t!("tui.find.about")), dim()),
        ]),
        Line::from(Span::styled("─".repeat(w), dim())),
    ];
    let room = (inner.height as usize).saturating_sub(5);
    let start = p.cursor.saturating_sub(room.saturating_sub(1));
    for (i, e) in results.iter().enumerate().skip(start).take(room) {
        let sel = i == p.cursor;
        let base = if sel { Style::default().bg(Color::Rgb(30, 58, 95)) } else { Style::default() };
        let kind_color = match e.kind {
            super::palette::EntryKind::Screen => ACCENT,
            super::palette::EntryKind::Setting => Color::Yellow,
            super::palette::EntryKind::Command => Color::Magenta,
        };
        let kind = e.kind.label();
        let kind_pad = " ".repeat(5usize.saturating_sub(kind.width()));
        let title = head_fit(&e.title, w * 2 / 5);
        let title_pad = " ".repeat((w * 2 / 5 + 1).saturating_sub(title.width()));
        let rest_w = w.saturating_sub(7 + w * 2 / 5 + 1);
        let place = head_fit(&e.path, rest_w / 2);
        let place_pad = " ".repeat((rest_w / 2 + 1).saturating_sub(place.width()));
        let cmd = head_fit(&e.cmd, rest_w.saturating_sub(rest_w / 2 + 1));
        let mut spans = vec![
            Span::styled(" ", base),
            Span::styled(format!("{kind}{kind_pad}"), base.fg(kind_color)),
            Span::styled(" ", base),
            Span::styled(title, base.add_modifier(Modifier::BOLD)),
            Span::styled(title_pad, base),
            Span::styled(place, base.fg(Color::DarkGray)),
            Span::styled(place_pad, base),
            Span::styled(cmd, base.fg(Color::DarkGray)),
        ];
        let used: usize = spans.iter().map(|s| s.width()).sum();
        if sel && used < w {
            spans.push(Span::styled(" ".repeat(w - used), base));
        }
        lines.push(Line::from(spans));
    }
    if results.is_empty() {
        lines.push(Line::from(Span::styled(t!("tui.find.none"), dim())));
    }
    let body_h = inner.height.saturating_sub(3);
    f.render_widget(Paragraph::new(lines), Rect { height: body_h, ..inner });
    let foot = Rect { y: inner.y + body_h, height: inner.height - body_h, ..inner };
    let same = p.selected().filter(|e| !e.cmd.is_empty()).map(|e| format!("{} saba-chan {}", t!("tui.same_command"), e.cmd)).unwrap_or_default();
    f.render_widget(
        Paragraph::new(vec![Line::from(Span::styled("─".repeat(w), dim())), Line::from(Span::styled(t!("tui.find.keys"), dim())), Line::from(Span::styled(head_fit(&same, w), dim()))]),
        foot,
    );
}

/// 창 뒤를 어둡게 한다 (창에 눈이 가게)
fn dim_behind(f: &mut Frame) {
    let area = f.area();
    let buf = f.buffer_mut();
    for y in area.y..area.y + area.height {
        for x in area.x..area.x + area.width {
            let cell = &mut buf[(x, y)];
            cell.set_style(Style::default().fg(Color::DarkGray).bg(Color::Reset).remove_modifier(Modifier::BOLD | Modifier::REVERSED | Modifier::UNDERLINED));
        }
    }
}

fn draw_overlays(f: &mut Frame, app: &mut App, area: Rect) {
    if app.dialog.is_some() || app.palette.is_some() || app.show_help || app.more.is_some() {
        dim_behind(f);
    }
    draw_more(f, app, area);
    draw_palette(f, app, area);
    if let Some(d) = &app.dialog {
        let mut lines = vec![Line::from(d.question.clone()), Line::from("")];
        let title = match &d.kind {
            AskKind::Confirm { .. } => {
                let pick = |i: usize, text: String| Span::styled(format!(" {text} "), if d.cursor == i { highlight(true) } else { Style::default() });
                lines.push(Line::from(vec![pick(0, t!("tui.yes")), Span::raw("  "), pick(1, t!("tui.no"))]));
                t!("tui.question")
            }
            AskKind::Select { items } => {
                for (i, it) in items.iter().enumerate() {
                    let style = if d.cursor == i { highlight(true) } else { Style::default() };
                    lines.push(Line::from(Span::styled(it.clone(), style)));
                }
                t!("tui.choose")
            }
            AskKind::Text { secret, .. } => {
                let shown = if *secret { "•".repeat(d.input.chars().count()) } else { d.input.clone() };
                lines.push(Line::from(format!("> {shown}▏")));
                t!("tui.enter_text")
            }
        };
        lines.push(Line::from(""));
        lines.push(Line::from(Span::styled(t!("tui.dialog_keys"), dim())));
        draw_popup(f, &title, lines, area);
    }
    if app.show_help {
        let lines = t!("tui.help").lines().map(|l| Line::from(l.to_string())).collect();
        draw_popup(f, &t!("tui.help_title"), lines, area);
    }
}

/// 키 안내: 지금 칸에서 쓸 수 있는 키. 폭이 모자라면 끝(나가기)은 두고 그 앞 안내부터 통째로 뺀다
fn key_hint(app: &App, room: usize) -> String {
    let text = if app.running && app.capture.is_none() {
        t!("tui.keys_running")
    } else if app.palette.is_some() {
        t!("tui.find.keys")
    } else if app.wizard.is_some() && app.focus != Focus::Command {
        t!("tui.keys_wizard")
    } else {
        match app.focus {
            Focus::Tabs => t!("tui.keys_tabs"),
            Focus::List => t!("tui.keys_list"),
            Focus::Detail => t!("tui.keys_detail"),
            Focus::Actions => t!("tui.keys_actions"),
            Focus::Content => match (&app.pane.state, app.tab_key()) {
                (_, "console") if app.page == Page::Servers => t!("tui.keys_console"),
                (PaneState::Form(..), _) if app.pane_is_current() => t!("tui.keys_form"),
                _ => t!("tui.keys_content"),
            },
            Focus::Command => t!("tui.keys_command"),
        }
    };
    let mut parts: Vec<&str> = text.split(" · ").collect();
    while parts.len() > 2 && parts.join(" · ").width() > room {
        parts.remove(parts.len() - 2);
    }
    let joined = parts.join(" · ");
    if joined.width() > room {
        head_fit(&joined, room)
    } else {
        joined
    }
}

fn draw_page(f: &mut Frame, app: &mut App, area: Rect) {
    if app.wizard.is_some() {
        draw_wizard(f, app, area);
        return;
    }
    match app.page {
        Page::Servers => draw_servers(f, app, area),
        Page::Discord => draw_discord(f, app, area),
        Page::Addons => draw_addons(f, app, area),
        Page::Updates => draw_updates(f, app, area),
        Page::Settings => draw_settings(f, app, area),
    }
}

pub fn draw(f: &mut Frame, app: &mut App) {
    let area = f.area();
    app.areas.clear();
    if area.width < MIN_WIDTH {
        // 안내만 보이므로 예전 클릭 자리는 지운 채로 둔다 (보이지 않는 버튼이 눌리지 않게)
        let rows = Layout::default().direction(Direction::Vertical).constraints([Constraint::Length(1), Constraint::Min(1)]).split(area);
        f.render_widget(Paragraph::new(header(app, rows[0].width)), rows[0]);
        f.render_widget(Paragraph::new(t!("tui.too_narrow")).alignment(Alignment::Center).wrap(Wrap { trim: true }), rows[1]);
        // 명령이 묻는 중이면 좁아도 질문은 그린다
        let body = rows[1];
        draw_overlays(f, app, body);
        app.areas.clear();
        return;
    }
    app.narrow = area.width < NARROW;
    let job_lines = jobs(app);
    let output_h = if app.output.open { (area.height * 2 / 5).max(6).min(area.height.saturating_sub(12)) } else { 0 };
    let rows = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Length(1),
            Constraint::Length(1),
            Constraint::Min(6),
            Constraint::Length(job_lines.len() as u16),
            Constraint::Length(output_h),
            Constraint::Length(1),
        ])
        .split(area);
    f.render_widget(Paragraph::new(header(app, rows[0].width)), rows[0]);
    draw_tabs(f, app, rows[1]);
    draw_page(f, app, rows[2]);
    f.render_widget(Paragraph::new(job_lines), rows[3]);
    if output_h > 0 {
        draw_output(f, app, rows[4]);
    }
    let line = footer(app, rows[5]);
    f.render_widget(Paragraph::new(line), rows[5]);
    draw_overlays(f, app, area);
}

#[cfg(test)]
mod tests {
    use super::super::app::{Action, App, Page};
    use super::*;
    use ratatui::backend::TestBackend;
    use ratatui::crossterm::event::{KeyCode, KeyEvent, KeyModifiers, MouseButton, MouseEvent, MouseEventKind};
    use ratatui::Terminal;
    use serde_json::json;

    fn app_with_data() -> App {
        crate::i18n::init("ko");
        let mut app = App::new();
        app.snap.health = Some(json!({ "version": "0.1.10" }));
        app.snap.servers = vec![
            json!({ "id": "a", "name": "pal-1", "module": "palworld", "status": "running", "players": { "online": 3, "max": 32 } }),
            json!({ "id": "b", "name": "mc-2", "module": "minecraft", "status": "stopped" }),
        ];
        app
    }

    fn text(term: &Terminal<TestBackend>) -> String {
        let buf = term.backend().buffer().clone();
        let (w, h) = (buf.area.width, buf.area.height);
        // 두 칸 글자 뒤 칸은 비워 두므로 공백을 빼고 비교한다
        (0..h).map(|y| (0..w).map(|x| buf[(x, y)].symbol().to_string()).collect::<String>().replace(' ', "")).collect::<Vec<_>>().join("\n")
    }

    fn render(app: &mut App, width: u16, height: u16) -> Terminal<TestBackend> {
        let mut term = Terminal::new(TestBackend::new(width, height)).unwrap();
        term.draw(|f| draw(f, app)).unwrap();
        term
    }

    fn screen(app: &mut App, width: u16, height: u16) -> String {
        text(&render(app, width, height))
    }

    fn nospace(s: String) -> String {
        s.replace(' ', "")
    }

    /// 폭마다 상태·서버·고른 서버의 콘솔이 보이고, 너무 좁으면 넓혀 달라는 안내만
    #[test]
    fn the_screen_fits_each_width() {
        for width in [100u16, 140] {
            let mut app = app_with_data();
            app.snap.console = Some(("a".into(), vec![json!({ "content": "Done (3.2s)!" })]));
            let t = screen(&mut app, width, 30);
            assert!(t.contains("0.1.10"), "{width}: {t}");
            assert!(t.contains("pal-1") && t.contains("3/32"), "{width}: {t}");
            assert!(t.contains("Done(3.2s)!"), "{width}: {t}");
            assert!(t.contains(&nospace(t!("tui.tab.console"))), "{width}: {t}");
        }
        let mut app = app_with_data();
        let narrow = screen(&mut app, 50, 20);
        assert!(!narrow.contains("pal-1"));
    }

    /// 좁으면 목록과 상세를 한 번에 하나씩: 목록에 있으면 목록만, 상세로 가면 `‹ 서버 목록 Esc` 줄과 상세만
    #[test]
    fn a_narrow_screen_stacks_the_list_and_the_detail() {
        let mut app = app_with_data();
        app.snap.console = Some(("a".into(), vec![json!({ "content": "Done (3.2s)!" })]));
        let t = screen(&mut app, 80, 24);
        assert!(t.contains("mc-2") && !t.contains("Done(3.2s)!"), "{t}");
        app.on_key(KeyEvent::from(KeyCode::Right));
        let t = screen(&mut app, 80, 24);
        assert!(t.contains(&format!("‹{}Esc", nospace(t!("tui.back.servers")))), "{t}");
        assert!(t.contains("Done(3.2s)!") && !t.contains("mc-2"), "{t}");
        // 넓으면 둘 다
        let t = screen(&mut app, 120, 24);
        assert!(t.contains("mc-2") && t.contains("Done(3.2s)!") && !t.contains(&nospace(t!("tui.back.servers"))), "{t}");
    }

    /// 쪽마다 그려지고 (상세 탭 이름까지) 버튼 자리가 마우스용으로 남는다
    #[test]
    fn every_page_draws_with_its_tabs_and_buttons() {
        for page in Page::ALL {
            let mut app = app_with_data();
            app.snap.guilds = vec![json!({ "guildId": "1", "guildName": "길드" })];
            app.go(page);
            let t = screen(&mut app, 140, 40);
            assert!(t.contains(&nospace(page.title())), "{page:?}: {t}");
            if let Some(tab) = page.tabs().first() {
                assert!(t.contains(&nospace(page.tab_label(tab))), "{page:?}: {t}");
            }
            assert_eq!(app.areas.buttons.len(), app.buttons().len(), "{page:?}");
        }
    }

    /// 상세 탭 아래에 같은 일을 하는 CLI 명령이 보인다 (CLI를 배우는 길)
    #[test]
    fn the_detail_shows_the_same_cli_command() {
        let mut app = app_with_data();
        app.tab[Page::Servers.slot()] = 3;
        let t = screen(&mut app, 120, 30);
        assert!(t.contains("saba-chanbackuplistpal-1"), "{t}");
    }

    /// 채운 칸은 화살표가 움직일 곳 하나뿐: 버튼에 있으면 고른 상세 탭은 채우지 않는다
    #[test]
    fn only_the_focused_place_is_filled() {
        let mut app = app_with_data();
        app.go(Page::Addons);
        app.tab[Page::Addons.slot()] = 1;
        app.snap.extensions = vec![json!({ "id": "steamcmd", "enabled": true })];
        app.focus = super::super::app::Focus::Actions;
        let term = render(&mut app, 120, 30);
        let tab = app.areas.detail[1];
        assert_ne!(term.backend().buffer()[(tab.x + 1, tab.y)].bg, ACCENT, "the selected tab is not filled");
        app.focus = super::super::app::Focus::Detail;
        let term = render(&mut app, 120, 30);
        assert_eq!(term.backend().buffer()[(tab.x + 1, tab.y)].bg, ACCENT);
    }

    /// 창(찾기·묻기·더 보기·도움말)이 뜨면 뒤를 어둡게 한다
    #[test]
    fn overlays_dim_what_is_behind() {
        let mut app = app_with_data();
        let before = render(&mut app, 120, 30);
        let title = |t: &Terminal<TestBackend>| t.backend().buffer()[(1, 0)].fg;
        assert_ne!(title(&before), Color::DarkGray);
        app.on_key(KeyEvent::new(KeyCode::Char('k'), KeyModifiers::CONTROL));
        let term = render(&mut app, 120, 30);
        assert_eq!(title(&term), Color::DarkGray);
        assert!(text(&term).contains(&nospace(t!("tui.find.title"))), "{}", text(&term));
    }

    /// 데몬 로그는 시각·레벨·태그와 함께 그려지고, 레벨마다 색이 다르다
    #[test]
    fn the_daemon_log_tab_highlights_its_log() {
        let mut app = app_with_data();
        app.go(Page::Settings);
        app.tab[Page::Settings.slot()] = 1;
        app.snap.daemon_log = vec![
            json!({ "timestamp": 1_700_000_000_000u64, "level": "error", "target": "saba_core::ipc", "message": "[Launch] Could not start", "spans": [[0, 8, "keyword"]] }),
        ];
        let term = render(&mut app, 120, 30);
        let t = text(&term);
        assert!(t.contains("ERROR[Launch]Couldnotstart"), "{t}");
        let buf = term.backend().buffer().clone();
        let (x, y) = (0..buf.area.height)
            .flat_map(|y| (0..buf.area.width.saturating_sub(5)).map(move |x| (x, y)))
            .find(|&(x, y)| (0..5).map(|i| buf[(x + i, y)].symbol()).collect::<String>() == "ERROR")
            .unwrap();
        assert_eq!(buf[(x, y)].fg, Color::Red);
    }

    /// 앱 설정 쪽에 지금 붙은 데몬의 포트가 보인다
    #[test]
    fn the_settings_page_shows_the_port_in_use() {
        let mut app = app_with_data();
        app.port = 41234;
        app.go(Page::Settings);
        assert!(screen(&mut app, 140, 34).contains("41234"));
    }

    /// 머리줄·키 안내가 폭을 넘으면 덜 중요한 칸을 통째로 뺀다
    #[test]
    fn header_and_footer_drop_whole_items_when_narrow() {
        for width in [60u16, 80] {
            let mut app = app_with_data();
            app.snap.metrics = Some(json!({ "cpu": { "total_percent": 3.0 }, "memory": { "used_bytes": 6.5e8, "total_bytes": 1.7e10 } }));
            let t = screen(&mut app, width, 34);
            let lines: Vec<&str> = t.lines().collect();
            assert!(!lines[0].contains("메모리") || lines[0].contains("16GB"), "{width}: {}", lines[0]);
            assert!(lines[0].contains("서버1/2실행"), "{width}: {}", lines[0]);
        }
    }

    /// GUI로 전환 버튼은 GUI를 쓸 수 있을 때만 보이고, 누르면 전환한다
    #[test]
    fn the_gui_switch_shows_only_when_the_gui_can_open() {
        let mut app = app_with_data();
        let label = nospace(t!("tui.gui_switch"));
        assert!(!screen(&mut app, 120, 30).contains(&label));
        app.snap.launchable = Some(json!({ "gui": true, "cli": true }));
        assert!(screen(&mut app, 120, 30).contains(&label));
        let r = app.areas.gui;
        let click = MouseEvent { kind: MouseEventKind::Down(MouseButton::Left), column: r.x + 1, row: r.y, modifiers: KeyModifiers::NONE };
        assert_eq!(app.on_mouse(click), Some(Action::SwitchToGui));
    }

    /// 보이는 쪽 탭을 누르면 그 쪽으로 간다 (80칸에서도 모두 보인다)
    #[test]
    fn a_visible_tab_can_be_clicked() {
        let mut app = app_with_data();
        screen(&mut app, 80, 24);
        let r = app.areas.tabs[Page::Settings.slot()];
        assert!(r.width > 0, "the settings tab is visible at 80 columns");
        app.on_mouse(MouseEvent { kind: MouseEventKind::Down(MouseButton::Left), column: r.x, row: r.y, modifiers: KeyModifiers::NONE });
        assert_eq!(app.page, Page::Settings);
    }

    /// 좁아져 안내만 보일 때 예전 버튼 자리가 클릭되지 않게 지운다
    #[test]
    fn a_too_narrow_screen_forgets_old_click_areas() {
        let mut app = app_with_data();
        screen(&mut app, 140, 34);
        assert!(!app.areas.buttons.is_empty() && !app.areas.tabs.is_empty());
        screen(&mut app, 50, 20);
        assert!(app.areas.buttons.is_empty() && app.areas.list == Rect::default() && app.areas.view == Rect::default() && app.areas.tabs.is_empty());
    }

    #[test]
    fn buttons_that_do_not_fit_cannot_be_clicked() {
        let area = Rect::new(0, 2, 60, 1);
        let rects = vec![Rect::new(0, 2, 6, 1), Rect::new(0, 3, 6, 1)];
        assert_eq!(clickable(rects, area), vec![Rect::new(0, 2, 6, 1), Rect::default()]);
    }

    /// 창 왼쪽 테두리에 걸친 두 칸 글자는 지운다
    #[test]
    fn popup_edge_does_not_sit_under_a_wide_character() {
        let mut term = Terminal::new(TestBackend::new(80, 12)).unwrap();
        term.draw(|f| {
            let area = f.area();
            for y in 0..area.height {
                f.render_widget(Paragraph::new("가".repeat(40)), Rect { x: 0, y, width: area.width, height: 1 });
            }
            draw_popup(f, "t", vec![Line::from("x")], area);
        })
        .unwrap();
        let buf = term.backend().buffer();
        let r = centered(Rect::new(0, 0, 80, 12), 53, 3);
        for y in r.y..r.y + r.height {
            assert!(buf[(r.x - 1, y)].symbol().width() < 2, "row {y}");
        }
    }

    /// 업데이트 상태를 아직 못 받았으면 "최신"이라고 하지 않는다
    #[test]
    fn updates_are_not_up_to_date_before_the_daemon_answers() {
        let mut app = app_with_data();
        app.go(Page::Updates);
        assert!(!screen(&mut app, 140, 34).contains("최신"));
        app.snap.updates = Some(json!({ "updates_available": 0, "components": [], "error": null, "last_check": null }));
        assert!(!screen(&mut app, 140, 34).contains("최신"));
        app.snap.updates = Some(json!({ "updates_available": 0, "components": [], "last_check": "2026-10-24T00:00:00Z" }));
        assert!(screen(&mut app, 140, 34).contains("최신"));
    }

    /// 출력 창은 긴 줄을 접는다. 명령 줄을 열면 맨 아래가 입력 줄이 된다
    #[test]
    fn the_output_wraps_long_lines_and_the_command_line_takes_the_bottom_row() {
        let mut app = app_with_data();
        app.begin_command("module info x");
        app.command_output(&format!("https://aka.ms/MinecraftEULA{}", "x".repeat(150)));
        app.command_output("두 번째 줄");
        let t = screen(&mut app, 100, 34);
        assert!(t.matches('x').count() >= 150, "{t}");
        app.on_key(KeyEvent::from(KeyCode::Char(':')));
        let t = screen(&mut app, 100, 34);
        assert!(t.lines().last().unwrap().starts_with("saba›"), "{t}");
    }

    #[test]
    fn wrap_counts_wide_characters_as_two() {
        assert_eq!(wrap("가나다라", 4), vec!["가나", "다라"]);
        assert_eq!(wrap("abc", 0), vec!["abc"]);
        assert_eq!(wrap("", 5), vec![""]);
    }

    /// 질문이 길어 접혀도 창 아래 키 안내가 보인다. 좁아도 질문은 그린다
    #[test]
    fn a_long_question_keeps_its_key_hint_and_shows_on_a_narrow_screen() {
        for width in [50u16, 120] {
            let mut app = app_with_data();
            let (reply, _rx) = std::sync::mpsc::channel();
            app.dialog = Some(super::super::app::Dialog {
                question: format!("마크-야생: {} 동의할까요?", "약관 원문을 읽고 ".repeat(12)),
                kind: AskKind::Confirm { default: true },
                cursor: 0,
                input: String::new(),
                reply,
            });
            let t = screen(&mut app, width, 30);
            assert!(t.contains("동의할까요"), "{t}");
            assert!(t.contains(&nospace(t!("tui.dialog_keys"))), "{width}: {t}");
        }
    }
}
