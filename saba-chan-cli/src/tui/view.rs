//! 그리기. 위에서 아래로 머리줄, 쪽 탭(오른쪽 끝에 GUI로 전환), 쪽, 작업 줄, 출력 창(열렸을 때), 맨 아래 줄
//!
//! 서버 쪽은 왼쪽 서버 목록, 오른쪽 고른 서버(정보 한 줄 → 버튼 → 큰 콘솔 → 콘솔 입력 줄)다.
//! 맨 아래 줄은 평소에는 키 안내와 방금 결과 한 줄, `:`를 누르면 명령 줄이 된다. 60칸보다 좁으면 넓혀 달라는 안내만.

use super::app::{App, AskKind, Button, Focus, Page};
use super::data::s;
use super::highlight::{console_line, daemon_line, wrap_spans};
use ratatui::layout::{Alignment, Constraint, Direction, Layout, Rect};
use ratatui::style::{Color, Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, BorderType, Borders, Clear, List, ListItem, Paragraph, Wrap};
use ratatui::Frame;
use serde_json::Value;
use unicode_width::{UnicodeWidthChar, UnicodeWidthStr};

const MIN_WIDTH: u16 = 60;

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

fn panel(title: impl Into<String>, focused: bool) -> Block<'static> {
    let style = if focused { Style::default().fg(ACCENT) } else { dim() };
    Block::default()
        .borders(Borders::ALL)
        .border_type(BorderType::Rounded)
        .border_style(style)
        .title(Span::styled(format!(" {} ", title.into()), if focused { style.add_modifier(Modifier::BOLD) } else { Style::default() }))
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

/// 쪽 탭 한 줄. 오른쪽 끝에 GUI로 전환 (GUI를 쓸 수 있을 때만). 지금 쪽이 보이게 앞을 자른다
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
    let focused = app.focus == Focus::Tabs;
    let mut spans = Vec::new();
    for (i, p) in Page::ALL.iter().enumerate() {
        let text = format!(" {} {} ", i + 1, p.title());
        spans.push(if *p == app.page {
            Span::styled(text, if focused { highlight(true) } else { Style::default().fg(ACCENT).add_modifier(Modifier::BOLD | Modifier::UNDERLINED) })
        } else {
            Span::styled(text, dim())
        });
        spans.push(Span::raw(" "));
    }
    let at = Page::ALL.iter().position(|p| *p == app.page).unwrap_or(0) * 2;
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
    f.render_widget(Paragraph::new(Line::from(spans)).scroll((0, skip as u16)), area);
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
    if b.more {
        format!("{} ▾", b.label)
    } else if b.fill {
        format!("{}…", b.label)
    } else {
        b.label.clone()
    }
}

/// 버튼 줄을 그리고 그 아래 나머지(한 줄 띄우고)를 돌려준다
fn with_buttons(f: &mut Frame, app: &mut App, area: Rect) -> Rect {
    let buttons = app.buttons();
    if buttons.is_empty() || area.height == 0 {
        app.areas.buttons.clear();
        return area;
    }
    let (rows, rects) = layout_buttons(&buttons, area);
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
            Style::default().fg(Color::White).bg(Color::DarkGray)
        };
        // format!의 폭은 글자 수라서 한글이 넘친다: 화면 폭으로 직접 가운데 맞춤
        let label = button_text(b);
        let pad = (r.width as usize).saturating_sub(label.width());
        let label = format!("{}{label}{}", " ".repeat(pad / 2), " ".repeat(pad - pad / 2));
        f.render_widget(Paragraph::new(Span::styled(label, style)), *r);
    }
    app.areas.buttons = clickable(rects, area);
    let used = (rows + 1).min(area.height);
    Rect { y: area.y + used, height: area.height - used, ..area }
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

fn draw_servers(f: &mut Frame, app: &mut App, area: Rect) {
    let (left, right) = split_list(area, list_width(area, 28, 22, 34));
    let inner_w = left.width.saturating_sub(2) as usize;
    let items = app
        .snap
        .servers
        .iter()
        .map(|v| {
            let (dot, color) = status_dot(s(v, "status"));
            let players = crate::output::players(v).unwrap_or_default();
            let name = head_fit(s(v, "name"), inner_w.saturating_sub(players.width() + 3));
            let pad = " ".repeat(inner_w.saturating_sub(2 + name.width() + players.width()));
            ListItem::new(Line::from(vec![Span::styled(format!("{dot} "), Style::default().fg(color)), Span::raw(name), Span::raw(pad), Span::styled(players, dim())]))
        })
        .collect();
    let title = format!("{} {}", t!("tui.page.servers"), app.snap.servers.len());
    draw_list(f, app, left, title, items, t!("tui.no_servers"));

    let focused = matches!(app.focus, Focus::Actions | Focus::Console);
    let Some(v) = app.selected_server().cloned() else {
        let block = panel(t!("tui.detail"), focused);
        let inner = block.inner(right);
        f.render_widget(block, right);
        let rest = with_buttons(f, app, inner);
        f.render_widget(Paragraph::new(Span::styled(t!("tui.no_servers"), dim())).wrap(Wrap { trim: false }), rest);
        return;
    };
    let (dot, color) = status_dot(s(&v, "status"));
    let block = panel(format!("{dot} {}  {}", s(&v, "name"), status_label(s(&v, "status"))), focused).title_style(Style::default().fg(color));
    let inner = block.inner(right);
    f.render_widget(block, right);
    if inner.height < 3 {
        return;
    }
    let info = [
        (t!("tui.col.game"), s(&v, "module").to_string()),
        (t!("tui.col.port"), v.get("port").map(|p| p.to_string()).unwrap_or_default()),
        (t!("tui.col.players"), crate::output::players(&v).unwrap_or_default()),
        (t!("tui.col.uptime"), uptime(&v)),
    ];
    f.render_widget(Paragraph::new(inline_kv(&info)), Rect { height: 1, ..inner });
    let rest = with_buttons(f, app, Rect { y: inner.y + 1, height: inner.height - 1, ..inner });
    if rest.height < 2 {
        return;
    }
    // 콘솔 제목 줄 (가는 선)
    let rule = format!("─ {} {}", t!("tui.server_console"), "─".repeat(rest.width as usize));
    f.render_widget(Paragraph::new(Span::styled(head_fit(&rule, rest.width as usize), dim())), Rect { height: 1, ..rest });
    let body = Rect { y: rest.y + 1, height: rest.height - 1, ..rest };
    app.areas.view = body;
    let out = Rect { height: body.height.saturating_sub(1), ..body };
    let id = s(&v, "id");
    let lines: Vec<Line<'static>> = match &app.snap.console {
        Some((cid, lines)) if cid == id && !lines.is_empty() => lines.iter().flat_map(|l| wrap_spans(console_line(l), out.width as usize)).collect(),
        _ => vec![Line::from(Span::styled(t!("tui.console_empty"), dim()))],
    };
    draw_lines(f, out, lines, &mut app.view_scroll);
    // 입력 줄
    let input_row = Rect { y: body.y + body.height - 1, height: 1, ..body };
    let typing = app.focus == Focus::Console;
    let prompt = Span::styled(format!("{}› ", s(&v, "name")), Style::default().fg(if typing { ACCENT } else { Color::DarkGray }).add_modifier(Modifier::BOLD));
    let line = if typing {
        let room = (input_row.width as usize).saturating_sub(prompt.width() + 1);
        Line::from(vec![prompt, Span::raw(tail_fit(&app.server_input.text, room)), Span::styled("▏", Style::default().fg(ACCENT))])
    } else {
        Line::from(vec![prompt, Span::styled(t!("tui.console_input_hint"), dim())])
    };
    f.render_widget(Paragraph::new(line), input_row);
}

fn draw_named(f: &mut Frame, app: &mut App, area: Rect, list: Vec<Value>, title: String, id_key: &str, detail: impl Fn(&Value) -> Vec<(String, String)>) {
    let (left, right) = split_list(area, list_width(area, 32, 22, 40));
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
    draw_list(f, app, left, title, items, t!("tui.empty"));
    let picked = list.get(app.selected_index());
    let (name, rows) = match picked {
        Some(v) => (s(v, id_key).to_string(), detail(v)),
        None => (t!("tui.detail"), Vec::new()),
    };
    let block = panel(name, app.focus == Focus::Actions);
    let inner = block.inner(right);
    f.render_widget(block, right);
    let rest = with_buttons(f, app, inner);
    f.render_widget(Paragraph::new(kv_lines(&rows)).wrap(Wrap { trim: false }), rest);
}

fn module_rows(v: &Value) -> Vec<(String, String)> {
    let mut rows = vec![(t!("col.version"), s(v, "version").to_string())];
    for (k, label) in [("game_name", "col.game"), ("description", "tui.col.about"), ("trust", "col.trust"), ("author", "col.author")] {
        if !s(v, k).is_empty() {
            rows.push((t!(label), s(v, k).to_string()));
        }
    }
    rows
}

fn extension_rows(v: &Value) -> Vec<(String, String)> {
    let mut rows = vec![
        (t!("col.version"), s(v, "version").to_string()),
        (t!("tui.col.enabled"), crate::output::yes_no(v.get("enabled").and_then(Value::as_bool) == Some(true))),
    ];
    for (k, label) in [("name", "tui.col.name"), ("description", "tui.col.about"), ("trust", "col.trust"), ("author", "col.author")] {
        if !s(v, k).is_empty() {
            rows.push((t!(label), s(v, k).to_string()));
        }
    }
    let hooks: Vec<&str> = v.get("hooks").and_then(Value::as_array).into_iter().flatten().filter_map(Value::as_str).collect();
    if !hooks.is_empty() {
        rows.push(("hooks".into(), hooks.join(", ")));
    }
    rows
}

/// 목록 없는 쪽: 한 칸 안에 버튼, 그 아래 내용
fn draw_plain(f: &mut Frame, app: &mut App, area: Rect, title: String, lines: Vec<Line<'static>>) {
    let block = panel(title, app.focus == Focus::Actions);
    let inner = block.inner(area);
    f.render_widget(block, area);
    let rest = with_buttons(f, app, inner);
    f.render_widget(Paragraph::new(lines).wrap(Wrap { trim: false }), rest);
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

/// 데몬이 업데이트를 한 번이라도 확인했는가 (`last_check`가 없으면 결과가 비어 있어도 최신이라는 뜻이 아니다)
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

/// 데몬 쪽: 정보 한 줄, 버튼, 그 아래 데몬 로그(구문 강조)
fn draw_daemon(f: &mut Frame, app: &mut App, area: Rect) {
    let block = panel(t!("tui.page.daemon"), app.focus == Focus::Actions);
    let inner = block.inner(area);
    f.render_widget(block, area);
    if inner.height < 2 {
        return;
    }
    let h = app.snap.health.clone().unwrap_or(Value::Null);
    let info = [
        (t!("tui.settings.daemon"), if h.is_null() { t!("tui.daemon_down") } else { format!("v{} · pid {}", s(&h, "version"), h.get("pid").map(|p| p.to_string()).unwrap_or_default()) }),
        (t!("tui.settings.port"), app.port.to_string()),
        (t!("tui.settings.language"), crate::i18n::lang().to_string()),
    ];
    f.render_widget(Paragraph::new(inline_kv(&info)), Rect { height: 1, ..inner });
    let rest = with_buttons(f, app, Rect { y: inner.y + 1, height: inner.height - 1, ..inner });
    if rest.height < 2 {
        return;
    }
    let rule = format!("─ {} {}", t!("tui.daemon_log"), "─".repeat(rest.width as usize));
    f.render_widget(Paragraph::new(Span::styled(head_fit(&rule, rest.width as usize), dim())), Rect { height: 1, ..rest });
    let body = Rect { y: rest.y + 1, height: rest.height - 1, ..rest };
    app.areas.view = body;
    let lines: Vec<Line<'static>> = if app.snap.daemon_log.is_empty() {
        vec![Line::from(Span::styled(t!("tui.console_empty"), dim()))]
    } else {
        app.snap.daemon_log.iter().flat_map(|l| wrap_spans(daemon_line(l), body.width as usize)).collect()
    };
    draw_lines(f, body, lines, &mut app.view_scroll);
}

fn draw_page(f: &mut Frame, app: &mut App, area: Rect) {
    match app.page {
        Page::Servers => draw_servers(f, app, area),
        Page::Modules => {
            let list = app.snap.modules.clone();
            draw_named(f, app, area, list, t!("tui.page.modules"), "name", module_rows)
        }
        Page::Extensions => {
            let list = app.snap.extensions.clone();
            draw_named(f, app, area, list, t!("tui.page.extensions"), "id", extension_rows)
        }
        Page::Discord => {
            let lines = discord_lines(app);
            draw_plain(f, app, area, t!("tui.page.discord"), lines)
        }
        Page::Updates => {
            let lines = update_lines(app);
            draw_plain(f, app, area, t!("tui.page.updates"), lines)
        }
        Page::Daemon => draw_daemon(f, app, area),
    }
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

/// 키 안내: 지금 칸에서 쓸 수 있는 키. 폭이 모자라면 끝(나가기)은 두고 그 앞 안내부터 통째로 뺀다
fn key_hint(app: &App, room: usize) -> String {
    let text = if app.running {
        t!("tui.keys_running")
    } else {
        match app.focus {
            Focus::Tabs => t!("tui.keys_tabs"),
            Focus::List => t!("tui.keys_list"),
            Focus::Actions => t!("tui.keys_actions"),
            Focus::Console => t!("tui.keys_console"),
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

fn draw_overlays(f: &mut Frame, app: &mut App, area: Rect) {
    draw_more(f, app, area);
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
    use super::super::app::{App, Page};
    use super::*;
    use ratatui::backend::TestBackend;
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

    fn screen(app: &mut App, width: u16, height: u16) -> String {
        let mut term = Terminal::new(TestBackend::new(width, height)).unwrap();
        term.draw(|f| draw(f, app)).unwrap();
        text(&term)
    }

    /// 폭마다 상태·서버·고른 서버의 콘솔과 입력 줄이 보이고, 좁으면 넓혀 달라는 안내만
    #[test]
    fn the_screen_fits_each_width() {
        for width in [60u16, 80, 100, 140] {
            let mut app = app_with_data();
            app.snap.console = Some(("a".into(), vec![json!({ "content": "Done (3.2s)!" })]));
            let t = screen(&mut app, width, 30);
            assert!(t.contains("0.1.10"), "{width}: {t}");
            assert!(t.contains("pal-1") && t.contains("3/32"), "{width}: {t}");
            assert!(t.contains("Done(3.2s)!"), "{width}: {t}");
            assert!(t.contains("pal-1›"), "{width}: {t}");
        }
        let mut app = app_with_data();
        let narrow = screen(&mut app, 50, 20);
        assert!(!narrow.contains("pal-1"));
    }

    /// 서버 쪽은 목록 → 버튼 → 콘솔 순서로 위에서 아래로: 버튼이 콘솔 위, 콘솔이 화면 대부분
    #[test]
    fn the_server_console_sits_under_the_buttons_and_takes_most_of_the_height() {
        let mut app = app_with_data();
        app.snap.console = Some(("a".into(), (0..40).map(|i| json!({ "content": format!("line-{i}") })).collect()));
        let t = screen(&mut app, 120, 40);
        let rows: Vec<&str> = t.lines().collect();
        let stop = rows.iter().position(|r| r.contains(&t!("tui.btn.stop").replace(' ', ""))).expect("stop button");
        let first = rows.iter().position(|r| r.contains("line-")).expect("console lines");
        assert!(stop < first, "buttons above the console");
        let shown = rows.iter().filter(|r| r.contains("line-")).count();
        assert!(shown >= 25, "console shows {shown} of 40 rows on a 40-row screen");
        assert!(t.contains("line-39"), "the newest line is visible");
    }

    /// 데몬 로그는 시각·레벨·태그와 함께 그려지고, 레벨마다 색이 다르다
    #[test]
    fn the_daemon_page_highlights_its_log() {
        let mut app = app_with_data();
        app.page = Page::Daemon;
        app.snap.daemon_log = vec![
            json!({ "timestamp": 1_700_000_000_000u64, "level": "error", "target": "saba_core::ipc", "message": "[Launch] Could not start", "spans": [[0, 8, "keyword"]] }),
        ];
        let mut term = Terminal::new(TestBackend::new(120, 30)).unwrap();
        term.draw(|f| draw(f, &mut app)).unwrap();
        let t = text(&term);
        assert!(t.contains("ERROR[Launch]Couldnotstart"), "{t}");
        let buf = term.backend().buffer().clone();
        let (x, y) = (0..buf.area.height)
            .flat_map(|y| (0..buf.area.width.saturating_sub(5)).map(move |x| (x, y)))
            .find(|&(x, y)| (0..5).map(|i| buf[(x + i, y)].symbol()).collect::<String>() == "ERROR")
            .unwrap();
        assert_eq!(buf[(x, y)].fg, Color::Red);
        let tag = (x..buf.area.width).find(|&cx| buf[(cx, y)].symbol() == "[").unwrap();
        assert_eq!(buf[(tag, y)].fg, Color::Magenta);
    }

    /// 머리줄·키 안내가 폭을 넘으면 덜 중요한 칸을 통째로 뺀다: 숫자 중간에서 잘리지 않는다
    #[test]
    fn header_and_footer_drop_whole_items_when_narrow() {
        for width in [60u16, 80] {
            let mut app = app_with_data();
            app.snap.metrics = Some(json!({ "cpu": { "total_percent": 3.0 }, "memory": { "used_bytes": 6.5e8, "total_bytes": 1.7e10 } }));
            let t = screen(&mut app, width, 34);
            let lines: Vec<&str> = t.lines().collect();
            assert!(!lines[0].contains("메모리") || lines[0].contains("16GB"), "{width}: {}", lines[0]);
            assert!(lines[0].contains("서버1/2실행"), "{width}: {}", lines[0]);
            assert!(lines.last().unwrap().ends_with("q나가기"), "{width}: {}", lines.last().unwrap());
        }
    }

    /// GUI로 전환 버튼은 GUI를 쓸 수 있을 때만 보이고, 누르면 전환한다
    #[test]
    fn the_gui_switch_shows_only_when_the_gui_can_open() {
        use ratatui::crossterm::event::{KeyModifiers, MouseButton, MouseEvent, MouseEventKind};
        let mut app = app_with_data();
        let label = t!("tui.gui_switch").replace(' ', "");
        assert!(!screen(&mut app, 120, 30).contains(&label));
        app.snap.launchable = Some(json!({ "gui": true, "cli": true }));
        assert!(screen(&mut app, 120, 30).contains(&label));
        let r = app.areas.gui;
        let click = MouseEvent { kind: MouseEventKind::Down(MouseButton::Left), column: r.x + 1, row: r.y, modifiers: KeyModifiers::NONE };
        assert_eq!(app.on_mouse(click), Some(super::super::app::Action::SwitchToGui));
    }

    /// 창 왼쪽 테두리에 걸친 두 칸 글자는 지운다: 남아 있으면 터미널이 테두리 칸을 덮어 창이 깨져 보인다
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

    /// 업데이트 상태를 아직 못 받았으면 "최신"이라고 하지 않고, 구성요소는 데몬이 주는 이름으로 보인다
    #[test]
    fn updates_are_not_up_to_date_before_the_daemon_answers() {
        let mut app = app_with_data();
        assert!(!screen(&mut app, 140, 34).lines().next().unwrap().contains("최신"));
        app.page = Page::Updates;
        assert!(!screen(&mut app, 140, 34).contains("최신"));
        // Codex 리뷰 #548: 데몬의 첫 상태(아직 확인 전, last_check 없음)를 받은 것만으로 "최신"이라고 했다
        app.snap.updates = Some(json!({ "updates_available": 0, "components": [], "error": null, "last_check": null }));
        assert!(!screen(&mut app, 140, 34).contains("최신"));
        app.page = Page::Servers;
        assert!(!screen(&mut app, 140, 34).lines().next().unwrap().contains("최신"));
        app.page = Page::Updates;
        app.snap.updates = Some(json!({ "updates_available": 0, "components": [], "last_check": "2026-10-24T00:00:00Z" }));
        assert!(screen(&mut app, 140, 34).contains("최신"));
        app.snap.updates = Some(json!({ "updates_available": 1, "components": [{ "component": "saba-core", "display_name": "SabaCore", "current_version": "0.1.9", "latest_version": "0.1.10", "update_available": true }] }));
        assert!(screen(&mut app, 140, 34).contains("SabaCore"));
    }

    /// 데몬 쪽의 포트는 지금 붙은 데몬의 포트다
    #[test]
    fn the_daemon_page_shows_the_port_in_use() {
        let mut app = app_with_data();
        app.port = 41234;
        app.page = Page::Daemon;
        assert!(screen(&mut app, 140, 34).contains("41234"));
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

    /// 넘쳐 그리지 못한 버튼은 누를 자리도 없다 (아래 칸 위에 겹치지 않는다). 버튼 번호는 그대로다
    #[test]
    fn buttons_that_do_not_fit_cannot_be_clicked() {
        let area = Rect::new(0, 2, 60, 1);
        let rects = vec![Rect::new(0, 2, 6, 1), Rect::new(0, 3, 6, 1)];
        assert_eq!(clickable(rects, area), vec![Rect::new(0, 2, 6, 1), Rect::default()]);
    }

    /// 80칸에서도 보이는 탭을 누르면 그 쪽으로 간다
    #[test]
    fn a_visible_tab_can_be_clicked() {
        use ratatui::crossterm::event::{KeyModifiers, MouseButton, MouseEvent, MouseEventKind};
        let mut app = app_with_data();
        screen(&mut app, 80, 24);
        let r = app.areas.tabs[5];
        assert!(r.width > 0, "the daemon tab is visible at 80 columns");
        app.on_mouse(MouseEvent { kind: MouseEventKind::Down(MouseButton::Left), column: r.x, row: r.y, modifiers: KeyModifiers::NONE });
        assert_eq!(app.page, Page::Daemon);
    }

    /// 쪽마다 그려지고 버튼 자리가 마우스용으로 남는다
    #[test]
    fn every_page_draws_with_its_buttons() {
        for page in Page::ALL {
            let mut app = app_with_data();
            app.page = page;
            let t = screen(&mut app, 140, 40);
            assert!(t.contains(&page.title().replace(' ', "")), "{page:?}: {t}");
            assert_eq!(app.areas.buttons.len(), app.buttons().len(), "{page:?}");
        }
    }

    /// 출력 창은 긴 줄을 접는다: 잘리지 않고 끝까지 보인다. 명령 줄을 열면 맨 아래가 입력 줄이 된다
    #[test]
    fn the_output_wraps_long_lines_and_the_command_line_takes_the_bottom_row() {
        let mut app = app_with_data();
        app.begin_command("module info x");
        app.command_output(&format!("https://aka.ms/MinecraftEULA{}", "x".repeat(150)));
        app.command_output("두 번째 줄");
        let t = screen(&mut app, 100, 34);
        assert!(t.matches('x').count() >= 150, "{t}");
        app.on_key(ratatui::crossterm::event::KeyEvent::from(ratatui::crossterm::event::KeyCode::Char(':')));
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
            assert!(t.contains(&t!("tui.dialog_keys").replace(' ', "")), "{width}: {t}");
        }
    }
}
