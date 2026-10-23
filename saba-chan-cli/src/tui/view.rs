//! 그리기. 맨 위 머리줄, 왼쪽 메뉴(좁으면 위 탭 줄), 가운데 쪽, 아래 콘솔, 맨 아래 단축키 줄
//!
//! 100칸 이상이면 메뉴가 왼쪽 세로 줄, 그보다 좁으면 쪽 이름 탭 한 줄. 60칸보다 좁으면 넓혀 달라는 안내만.

use super::app::{App, AskKind, Button, Focus, Page};
use super::data::s;
use ratatui::layout::{Alignment, Constraint, Direction, Layout, Rect};
use ratatui::style::{Color, Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, BorderType, Borders, Cell, Clear, List, ListItem, Paragraph, Row, Sparkline, Table, Wrap};
use ratatui::Frame;
use serde_json::Value;
use unicode_width::{UnicodeWidthChar, UnicodeWidthStr};

const MIN_WIDTH: u16 = 60;
/// 이보다 넓으면 메뉴를 왼쪽 세로 줄로
const NAV_WIDTH_FROM: u16 = 100;
const NAV_WIDTH: u16 = 20;
/// 콘솔 높이 (테두리 포함)
const CONSOLE_HEIGHT: u16 = 10;

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

/// 맨 위 한 줄: 이름·데몬·서버 수·CPU·메모리·디스코드·업데이트
/// 맨 위 한 줄. 폭이 모자라면 덜 중요한 칸(메모리 → CPU → 업데이트 → 디스코드)부터 통째로 뺀다
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
    } else if snap.updates.is_some() && !failed {
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

fn draw_nav(f: &mut Frame, app: &mut App, area: Rect) {
    let items: Vec<ListItem> = Page::ALL
        .iter()
        .enumerate()
        .map(|(i, p)| ListItem::new(Line::from(vec![Span::styled(format!("{} ", i + 1), dim()), Span::raw(p.title())])))
        .collect();
    let mut state = ratatui::widgets::ListState::default();
    state.select(Page::ALL.iter().position(|p| *p == app.page));
    let list = List::new(items).block(panel(t!("tui.menu"), app.focus == Focus::Nav)).highlight_style(highlight(app.focus == Focus::Nav));
    app.nav_area = area;
    app.tab_areas.clear();
    f.render_stateful_widget(list, area, &mut state);
}

/// 좁은 화면: 쪽 이름을 탭 한 줄로 (지금 쪽이 보이게 앞을 자른다)
fn draw_tabs(f: &mut Frame, app: &mut App, area: Rect) {
    let mut spans = Vec::new();
    for (i, p) in Page::ALL.iter().enumerate() {
        let text = format!(" {} {} ", i + 1, p.title());
        spans.push(if *p == app.page { Span::styled(text, highlight(app.focus == Focus::Nav)) } else { Span::styled(text, dim()) });
    }
    let at = Page::ALL.iter().position(|p| *p == app.page).unwrap_or(0);
    let before: usize = spans[..at].iter().map(|s| s.content.width()).sum();
    let mine = spans[at].content.width();
    let skip = (before + mine).saturating_sub(area.width as usize);
    app.nav_area = Rect::default();
    // 앞을 자른 만큼 밀어서 보이는 부분만 클릭 자리로 남긴다
    let mut x = 0usize;
    app.tab_areas = spans
        .iter()
        .map(|s| {
            let (start, end) = (x, x + s.content.width());
            x = end;
            let (l, r) = (start.max(skip) - skip, (end.saturating_sub(skip)).min(area.width as usize));
            if r <= l {
                return Rect::default();
            }
            Rect { x: area.x + l as u16, y: area.y, width: (r - l) as u16, height: 1 }
        })
        .collect();
    f.render_widget(Paragraph::new(Line::from(spans)).scroll((0, skip as u16)), area);
}

/// 버튼을 폭에 맞춰 여러 줄로 놓는다. (줄 수, 버튼 자리)
fn layout_buttons(buttons: &[Button], area: Rect) -> (u16, Vec<Rect>) {
    let mut rects = Vec::new();
    let (mut x, mut y) = (area.x, area.y);
    for b in buttons {
        let w = b.label.width() as u16 + 4;
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

fn button_rows(app: &App, width: u16) -> u16 {
    layout_buttons(&app.buttons(), Rect { x: 0, y: 0, width, height: 1 }).0
}

fn draw_buttons(f: &mut Frame, app: &mut App, area: Rect) {
    let buttons = app.buttons();
    let (_, rects) = layout_buttons(&buttons, area);
    let focused = app.focus == Focus::Buttons;
    for (i, (b, r)) in buttons.iter().zip(&rects).enumerate() {
        let style = if focused && i == app.button {
            highlight(true)
        } else if !b.enabled {
            dim()
        } else {
            Style::default().fg(Color::White).bg(Color::DarkGray)
        };
        let label = if b.fill { format!(" {}… ", b.label) } else { format!(" {} ", b.label) };
        // format!의 폭은 글자 수라서 한글이 넘친다: 화면 폭으로 직접 가운데 맞춤
        let pad = (r.width as usize).saturating_sub(label.width());
        let label = format!("{}{label}{}", " ".repeat(pad / 2), " ".repeat(pad - pad / 2));
        f.render_widget(Paragraph::new(Span::styled(label, style)), *r);
    }
    app.button_areas = rects;
}

/// 쪽 머리: 버튼 줄 (있으면) 아래에 나머지를 돌려준다
fn with_buttons(f: &mut Frame, app: &mut App, area: Rect) -> Rect {
    if app.buttons().is_empty() {
        app.button_areas.clear();
        return area;
    }
    let rows = button_rows(app, area.width);
    let parts = Layout::default().direction(Direction::Vertical).constraints([Constraint::Length(rows), Constraint::Length(1), Constraint::Min(1)]).split(area);
    draw_buttons(f, app, parts[0]);
    parts[2]
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

fn card(f: &mut Frame, area: Rect, title: String, big: String, sub: Line<'static>, color: Color) {
    let block = panel(title, false);
    let inner = block.inner(area);
    f.render_widget(block, area);
    let lines = vec![Line::from(Span::styled(big, Style::default().fg(color).add_modifier(Modifier::BOLD))), sub];
    f.render_widget(Paragraph::new(lines), inner);
}

fn spark_card(f: &mut Frame, area: Rect, title: String, data: &[u64], color: Color) {
    let block = panel(title.clone(), false);
    let inner = block.inner(area);
    f.render_widget(block, area);
    let now = data.last().map(|v| format!("{v}%")).unwrap_or_else(|| "-".into());
    let parts = Layout::default().direction(Direction::Vertical).constraints([Constraint::Length(1), Constraint::Min(1)]).split(inner);
    f.render_widget(Paragraph::new(Span::styled(now, Style::default().fg(color).add_modifier(Modifier::BOLD))), parts[0]);
    let take = parts[1].width as usize;
    let tail: Vec<u64> = data.iter().rev().take(take).rev().copied().collect();
    // 낮은 사용량도 보이게 위 끝을 기록의 최댓값에 맞춘다 (10% 아래로는 줄이지 않는다)
    let top = tail.iter().copied().max().unwrap_or(0).max(10);
    f.render_widget(Sparkline::default().data(&tail).max(top).style(Style::default().fg(color)), parts[1]);
}

fn server_table(app: &App, area: Rect) -> Table<'static> {
    // 좁으면 게임·포트·가동 시간 칸을 뺀다
    let wide = area.width >= 62;
    let rows: Vec<Row> = app
        .snap
        .servers
        .iter()
        .map(|v| {
            let (dot, color) = status_dot(s(v, "status"));
            let mut cells = vec![
                Cell::from(Span::styled(dot, Style::default().fg(color))),
                Cell::from(s(v, "name").to_string()),
                Cell::from(status_label(s(v, "status"))),
                Cell::from(crate::output::players(v).unwrap_or_default()),
            ];
            if wide {
                cells.insert(2, Cell::from(s(v, "module").to_string()));
                cells.push(Cell::from(v.get("port").map(|p| p.to_string()).unwrap_or_default()));
                cells.push(Cell::from(uptime(v)));
            }
            Row::new(cells)
        })
        .collect();
    let mut header = vec![String::new(), t!("tui.col.name"), t!("tui.col.status"), t!("tui.col.players")];
    let mut widths = vec![Constraint::Length(1), Constraint::Min(8), Constraint::Length(8), Constraint::Length(7)];
    if wide {
        header.insert(2, t!("tui.col.game"));
        header.extend([t!("tui.col.port"), t!("tui.col.uptime")]);
        widths.insert(2, Constraint::Length(10));
        widths.extend([Constraint::Length(6), Constraint::Length(9)]);
    }
    Table::new(rows, widths).header(Row::new(header).style(dim())).column_spacing(1)
}

fn draw_dashboard(f: &mut Frame, app: &mut App, area: Rect) {
    let rows = Layout::default().direction(Direction::Vertical).constraints([Constraint::Length(5), Constraint::Min(4)]).split(area);
    let cards = Layout::default().direction(Direction::Horizontal).constraints([Constraint::Ratio(1, 4); 4]).split(rows[0]);
    let snap = &app.snap;
    card(
        f,
        cards[0],
        t!("tui.card.servers"),
        format!("{} / {}", snap.running(), snap.servers.len()),
        Line::from(Span::styled(t!("tui.card.servers_sub"), dim())),
        Color::Green,
    );
    card(
        f,
        cards[1],
        t!("tui.card.players"),
        snap.players_online().map(|n| n.to_string()).unwrap_or_else(|| "-".into()),
        Line::from(Span::styled(t!("tui.card.players_sub"), dim())),
        ACCENT,
    );
    let cpu: Vec<u64> = snap.cpu.iter().copied().collect();
    let mem: Vec<u64> = snap.mem.iter().copied().collect();
    spark_card(f, cards[2], "CPU".into(), &cpu, Color::Yellow);
    spark_card(f, cards[3], t!("tui.card.memory"), &mem, Color::Magenta);

    let body = with_buttons(f, app, rows[1]);
    let cols = Layout::default().direction(Direction::Horizontal).constraints([Constraint::Percentage(60), Constraint::Percentage(40)]).split(body);
    let block = panel(t!("tui.page.servers"), false);
    if app.snap.servers.is_empty() {
        f.render_widget(Paragraph::new(Span::styled(t!("tui.no_servers"), dim())).block(block).wrap(Wrap { trim: false }), cols[0]);
    } else {
        f.render_widget(server_table(app, cols[0]).block(block), cols[0]);
    }
    let feed: Vec<Line> = if app.snap.feed.is_empty() {
        vec![Line::from(Span::styled(t!("tui.feed_empty"), dim()))]
    } else {
        app.snap.feed.iter().map(|(at, text)| Line::from(vec![Span::styled(format!("{at} "), dim()), Span::raw(text.clone())])).collect()
    };
    f.render_widget(Paragraph::new(feed).block(panel(t!("tui.feed"), false)).wrap(Wrap { trim: false }), cols[1]);
}

fn draw_list(f: &mut Frame, app: &mut App, area: Rect, title: String, items: Vec<ListItem<'static>>) {
    let focused = app.focus == Focus::List;
    let items = if items.is_empty() { vec![ListItem::new(Span::styled(t!("tui.empty"), dim()))] } else { items };
    let list = List::new(items).block(panel(title, focused)).highlight_style(highlight(focused));
    app.list_area = area;
    f.render_stateful_widget(list, area, &mut app.list_state);
}

fn split_list(area: Rect, list_pct: u16) -> (Rect, Rect) {
    let cols = Layout::default().direction(Direction::Horizontal).constraints([Constraint::Percentage(list_pct), Constraint::Percentage(100 - list_pct)]).split(area);
    (cols[0], cols[1])
}

fn draw_servers(f: &mut Frame, app: &mut App, area: Rect) {
    let body = with_buttons(f, app, area);
    let (left, right) = split_list(body, 34);
    let items = app
        .snap
        .servers
        .iter()
        .map(|v| {
            let (dot, color) = status_dot(s(v, "status"));
            ListItem::new(Line::from(vec![Span::styled(format!("{dot} "), Style::default().fg(color)), Span::raw(s(v, "name").to_string())]))
        })
        .collect();
    draw_list(f, app, left, t!("tui.page.servers"), items);
    let Some(v) = app.selected_server().cloned() else {
        f.render_widget(Paragraph::new(Span::styled(t!("tui.no_servers"), dim())).block(panel(t!("tui.detail"), false)).wrap(Wrap { trim: false }), right);
        return;
    };
    let mut rows = vec![
        (t!("tui.col.status"), status_label(s(&v, "status"))),
        (t!("tui.col.game"), s(&v, "module").to_string()),
        (t!("tui.col.port"), v.get("port").map(|p| p.to_string()).unwrap_or_default()),
    ];
    if let Some(p) = crate::output::players(&v) {
        rows.push((t!("tui.col.players"), p));
    }
    let up = uptime(&v);
    if !up.is_empty() {
        rows.push((t!("tui.col.uptime"), up));
    }
    let info_h = rows.len() as u16 + 2;
    let parts = Layout::default().direction(Direction::Vertical).constraints([Constraint::Length(info_h), Constraint::Min(3)]).split(right);
    f.render_widget(Paragraph::new(kv_lines(&rows)).block(panel(s(&v, "name").to_string(), false)), parts[0]);
    let id = s(&v, "id");
    let lines: Vec<String> = match &app.snap.console {
        Some((cid, lines)) if cid == id => lines.clone(),
        _ => Vec::new(),
    };
    draw_tail(f, parts[1], t!("tui.server_console"), &lines, &t!("tui.console_empty"));
}

/// 끝에서부터 보이는 만큼 (접어서)
fn draw_tail(f: &mut Frame, area: Rect, title: String, lines: &[String], empty: &str) {
    let block = panel(title, false);
    let inner = block.inner(area);
    f.render_widget(block, area);
    if lines.is_empty() {
        f.render_widget(Paragraph::new(Span::styled(empty.to_string(), dim())).wrap(Wrap { trim: false }), inner);
        return;
    }
    let wrapped: Vec<String> = lines.iter().flat_map(|l| wrap(l, inner.width as usize)).collect();
    let start = wrapped.len().saturating_sub(inner.height as usize);
    f.render_widget(Paragraph::new(wrapped[start..].iter().map(|l| Line::from(l.clone())).collect::<Vec<_>>()), inner);
}

fn draw_named(f: &mut Frame, app: &mut App, area: Rect, list: Vec<Value>, title: String, id_key: &str, detail: impl Fn(&Value) -> Vec<(String, String)>) {
    let body = with_buttons(f, app, area);
    let (left, right) = split_list(body, 40);
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
    draw_list(f, app, left, title, items);
    let picked = list.get(app.selected_index());
    let (name, rows) = match picked {
        Some(v) => (s(v, id_key).to_string(), detail(v)),
        None => (t!("tui.detail"), Vec::new()),
    };
    f.render_widget(Paragraph::new(kv_lines(&rows)).block(panel(name, false)).wrap(Wrap { trim: false }), right);
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

fn draw_discord(f: &mut Frame, app: &mut App, area: Rect) {
    let body = with_buttons(f, app, area);
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
    f.render_widget(Paragraph::new(lines).block(panel(t!("tui.page.discord"), false)).wrap(Wrap { trim: false }), body);
}

fn draw_updates(f: &mut Frame, app: &mut App, area: Rect) {
    let body = with_buttons(f, app, area);
    let u = app.snap.updates.clone().unwrap_or(Value::Null);
    let n = u.get("updates_available").and_then(Value::as_u64).unwrap_or(0);
    let failed = !s(&u, "error").is_empty();
    let mut lines = Vec::new();
    if u.is_null() {
        // 아직 답을 못 받았다: 최신인지 모른다
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
    f.render_widget(Paragraph::new(lines).block(panel(t!("tui.page.updates"), false)).wrap(Wrap { trim: false }), body);
}

fn draw_settings(f: &mut Frame, app: &mut App, area: Rect) {
    let body = with_buttons(f, app, area);
    let h = app.snap.health.clone().unwrap_or(Value::Null);
    let rows = vec![
        (t!("tui.settings.daemon"), format!("v{} · pid {}", s(&h, "version"), h.get("pid").map(|p| p.to_string()).unwrap_or_default())),
        (t!("tui.settings.port"), app.port.to_string()),
        (t!("tui.settings.language"), crate::i18n::lang().to_string()),
    ];
    let mut lines = kv_lines(&rows);
    lines.push(Line::from(""));
    lines.push(Line::from(Span::styled(t!("tui.settings.hint"), dim())));
    f.render_widget(Paragraph::new(lines).block(panel(t!("tui.page.settings"), false)).wrap(Wrap { trim: false }), body);
}

fn draw_daemon_log(f: &mut Frame, app: &mut App, area: Rect) {
    let body = with_buttons(f, app, area);
    let lines = app.snap.daemon_log.clone();
    draw_tail(f, body, t!("tui.page.daemon_log"), &lines, &t!("tui.console_empty"));
}

fn draw_commands(f: &mut Frame, app: &mut App, area: Rect) {
    let (left, right) = split_list(area, 30);
    let items = app.menu.iter().map(|m| ListItem::new(m.name.clone())).collect();
    draw_list(f, app, left, t!("tui.page.commands"), items);
    let Some(item) = app.menu.get(app.selected_index()).cloned() else { return };
    let block = panel(item.name.clone(), app.focus == Focus::Buttons);
    let inner = block.inner(right);
    f.render_widget(block, right);
    let parts = Layout::default().direction(Direction::Vertical).constraints([Constraint::Length(2), Constraint::Min(1)]).split(inner);
    f.render_widget(Paragraph::new(Span::styled(item.about.clone(), dim())).wrap(Wrap { trim: true }), parts[0]);
    // 하위 명령은 한 줄에 하나: 이름과 설명
    let buttons = app.buttons();
    let mut rects = Vec::new();
    let focused = app.focus == Focus::Buttons;
    let about: Vec<String> = if item.subs.is_empty() { vec![t!("tui.enter_to_run")] } else { item.subs.iter().map(|s| s.about.clone()).collect() };
    let top = app.button.saturating_sub(parts[1].height.saturating_sub(1) as usize);
    for (i, b) in buttons.iter().enumerate().skip(top) {
        let y = parts[1].y + (i - top) as u16;
        if y >= parts[1].y + parts[1].height {
            break;
        }
        let r = Rect { x: parts[1].x, y, width: parts[1].width, height: 1 };
        let style = if focused && i == app.button { highlight(true) } else { Style::default() };
        let label = if b.fill { format!("{}…", b.label) } else { b.label.clone() };
        f.render_widget(Paragraph::new(Line::from(vec![Span::styled(format!("{label:<14}"), style), Span::styled(format!(" {}", about.get(i).cloned().unwrap_or_default()), dim())])), r);
        rects.push(r);
    }
    // 화면에 보이는 것만 누를 수 있다: 앞에서 건너뛴 칸은 빈 자리로 둔다
    let mut areas = vec![Rect::default(); top];
    areas.extend(rects);
    app.button_areas = areas;
}

fn draw_page(f: &mut Frame, app: &mut App, area: Rect) {
    app.list_area = Rect::default();
    match app.page {
        Page::Dashboard => draw_dashboard(f, app, area),
        Page::Servers => draw_servers(f, app, area),
        Page::Modules => {
            let list = app.snap.modules.clone();
            draw_named(f, app, area, list, t!("tui.page.modules"), "name", module_rows)
        }
        Page::Extensions => {
            let list = app.snap.extensions.clone();
            draw_named(f, app, area, list, t!("tui.page.extensions"), "id", extension_rows)
        }
        Page::Discord => draw_discord(f, app, area),
        Page::Updates => draw_updates(f, app, area),
        Page::Settings => draw_settings(f, app, area),
        Page::DaemonLog => draw_daemon_log(f, app, area),
        Page::Commands => draw_commands(f, app, area),
    }
}

/// 아래 콘솔: 결과(접어서, 끝에서부터)와 입력 줄
fn draw_console(f: &mut Frame, app: &mut App, area: Rect) {
    let focused = app.focus == Focus::Console;
    let title = match &app.last_command {
        Some(c) if app.running => format!("{} › {c}  {}", t!("tui.console"), t!("tui.running_hint")),
        Some(c) => format!("{} › {c}", t!("tui.console")),
        None => t!("tui.console"),
    };
    let block = panel(title, focused);
    let inner = block.inner(area);
    f.render_widget(block, area);
    app.console_area = area;
    if inner.height == 0 {
        return;
    }
    let out_h = inner.height.saturating_sub(1) as usize;
    let wrapped: Vec<String> = app.console.lines.iter().flat_map(|l| wrap(l, inner.width as usize)).collect();
    app.console.scroll = app.console.scroll.min(wrapped.len().saturating_sub(out_h));
    let end = wrapped.len() - app.console.scroll;
    let start = end.saturating_sub(out_h);
    let lines: Vec<Line> = wrapped[start..end]
        .iter()
        .map(|l| {
            if l.starts_with("› ") {
                Line::from(Span::styled(l.clone(), Style::default().fg(ACCENT)))
            } else if l.starts_with('✗') {
                Line::from(Span::styled(l.clone(), Style::default().fg(Color::Red)))
            } else if l.starts_with('✓') {
                Line::from(Span::styled(l.clone(), Style::default().fg(Color::Green)))
            } else if l.starts_with('!') {
                Line::from(Span::styled(l.clone(), Style::default().fg(Color::Yellow)))
            } else {
                Line::from(l.clone())
            }
        })
        .collect();
    f.render_widget(Paragraph::new(lines), Rect { height: out_h as u16, ..inner });
    let input_row = Rect { y: inner.y + inner.height - 1, height: 1, ..inner };
    let prompt = Span::styled("saba› ", Style::default().fg(if focused { ACCENT } else { Color::DarkGray }).add_modifier(Modifier::BOLD));
    let line = if focused {
        // 입력이 길면 끝이 보이게
        let room = (inner.width as usize).saturating_sub(8);
        let shown: String = {
            let mut w = 0;
            let mut tail: Vec<char> = Vec::new();
            for ch in app.console.input.chars().rev() {
                w += ch.width().unwrap_or(0);
                if w > room {
                    break;
                }
                tail.push(ch);
            }
            tail.into_iter().rev().collect()
        };
        Line::from(vec![prompt, Span::raw(shown), Span::styled("▏", Style::default().fg(ACCENT))])
    } else if app.console.input.is_empty() {
        Line::from(vec![prompt, Span::styled(t!("tui.console_hint"), dim())])
    } else {
        Line::from(vec![prompt, Span::styled(app.console.input.clone(), dim())])
    };
    f.render_widget(Paragraph::new(line), input_row);
}

/// 맨 아래 한 줄: 지금 할 수 있는 키 (콘솔에 있을 때와 아닐 때가 다르다).
/// 폭이 모자라면 끝(나가기)은 두고 그 앞 안내부터 통째로 뺀다
fn footer(app: &App, width: u16) -> Line<'static> {
    let text = if app.running {
        t!("tui.keys_running")
    } else if app.focus == Focus::Console {
        t!("tui.keys_console")
    } else {
        t!("tui.keys")
    };
    let mut parts: Vec<&str> = text.split(" · ").collect();
    while parts.len() > 2 && parts.join(" · ").width() > width as usize {
        parts.remove(parts.len() - 2);
    }
    Line::from(Span::styled(parts.join(" · "), dim()))
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

/// 가운데 창. 높이는 접힌 줄까지 세어 마지막 줄(키 안내)이 잘리지 않게
fn draw_popup(f: &mut Frame, title: &str, lines: Vec<Line>, area: Rect) {
    let width = (area.width * 2 / 3).max(50).min(area.width);
    let inner_w = width.saturating_sub(2) as usize;
    let height: usize = lines.iter().map(|l| wrap(&l.to_string(), inner_w).len()).sum::<usize>() + 2;
    let r = centered(area, width, height as u16);
    f.render_widget(Clear, r);
    // 왼쪽 테두리에 걸친 두 칸 글자를 지운다 (남으면 터미널에서 테두리를 덮는다)
    if r.x > 0 {
        let buf = f.buffer_mut();
        for y in r.y..r.y + r.height {
            if buf[(r.x - 1, y)].symbol().width() > 1 {
                buf[(r.x - 1, y)].set_symbol(" ");
            }
        }
    }
    f.render_widget(Paragraph::new(lines).wrap(Wrap { trim: false }).block(panel(title.to_string(), true)), r);
}

fn draw_overlays(f: &mut Frame, app: &App, area: Rect) {
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
    let job_lines = jobs(app);
    if area.width < MIN_WIDTH {
        // 안내만 보이므로 예전 클릭 자리를 지운다 (보이지 않는 버튼이 눌리지 않게)
        app.button_areas.clear();
        app.nav_area = Rect::default();
        app.tab_areas.clear();
        app.list_area = Rect::default();
        app.console_area = Rect::default();
        let rows = Layout::default().direction(Direction::Vertical).constraints([Constraint::Length(1), Constraint::Min(1)]).split(area);
        f.render_widget(Paragraph::new(header(app, rows[0].width)), rows[0]);
        f.render_widget(Paragraph::new(t!("tui.too_narrow")).alignment(Alignment::Center).wrap(Wrap { trim: true }), rows[1]);
        // 명령이 묻는 중이면 좁아도 질문은 그린다
        draw_overlays(f, app, rows[1]);
        return;
    }
    let console_h = if app.console.big { area.height / 2 } else { CONSOLE_HEIGHT.min(area.height / 3).max(4) };
    let rows = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Length(1),
            Constraint::Min(6),
            Constraint::Length(job_lines.len() as u16),
            Constraint::Length(console_h),
            Constraint::Length(1),
        ])
        .split(area);
    f.render_widget(Paragraph::new(header(app, rows[0].width)), rows[0]);
    let body = rows[1];
    let page_area = if area.width >= NAV_WIDTH_FROM {
        let cols = Layout::default().direction(Direction::Horizontal).constraints([Constraint::Length(NAV_WIDTH), Constraint::Min(30)]).split(body);
        draw_nav(f, app, cols[0]);
        cols[1]
    } else {
        let parts = Layout::default().direction(Direction::Vertical).constraints([Constraint::Length(1), Constraint::Min(4)]).split(body);
        draw_tabs(f, app, parts[0]);
        parts[1]
    };
    draw_page(f, app, page_area);
    f.render_widget(Paragraph::new(job_lines), rows[2]);
    draw_console(f, app, rows[3]);
    f.render_widget(Paragraph::new(footer(app, rows[4].width)), rows[4]);
    draw_overlays(f, app, area);
}

#[cfg(test)]
mod tests {
    use super::super::app::{menu_from, App, Page};
    use super::*;
    use ratatui::backend::TestBackend;
    use ratatui::Terminal;
    use serde_json::json;

    fn app_with_data() -> App {
        crate::i18n::init("ko");
        let mut app = App::new(menu_from(&crate::cli::localized()));
        app.snap.health = Some(json!({ "version": "0.1.9" }));
        app.snap.servers = vec![
            json!({ "id": "a", "name": "pal-1", "module": "palworld", "status": "running", "players": { "online": 3, "max": 32 } }),
            json!({ "id": "b", "name": "mc-2", "module": "minecraft", "status": "stopped" }),
        ];
        app.push_line("결과 한 줄");
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

    /// 폭마다 배치가 바뀌어도 상태·서버·콘솔이 보이고, 좁으면 넓혀 달라는 안내만
    #[test]
    fn the_screen_fits_each_width() {
        for width in [60u16, 80, 100, 140] {
            let mut app = app_with_data();
            let t = screen(&mut app, width, 34);
            assert!(t.contains("0.1.9"), "{width}: {t}");
            assert!(t.contains("pal-1") && t.contains("3/32"), "{width}: {t}");
            assert!(t.contains("결과한줄"), "{width}: {t}");
            assert!(t.contains("saba›"), "{width}: {t}");
        }
        let mut app = app_with_data();
        let narrow = screen(&mut app, 50, 20);
        assert!(!narrow.contains("pal-1"));
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
        app.snap.updates = Some(json!({ "updates_available": 1, "components": [{ "component": "saba-core", "display_name": "SabaCore", "current_version": "0.1.9", "latest_version": "0.1.10", "update_available": true }] }));
        assert!(screen(&mut app, 140, 34).contains("SabaCore"));
    }

    /// 설정 화면의 데몬 포트는 지금 붙은 데몬의 포트다
    #[test]
    fn settings_show_the_port_in_use() {
        let mut app = app_with_data();
        app.port = 41234;
        app.page = Page::Settings;
        assert!(screen(&mut app, 140, 34).contains("41234"));
    }

    /// 좁아져 안내만 보일 때 예전 버튼 자리가 클릭되지 않게 지운다
    #[test]
    fn a_too_narrow_screen_forgets_old_click_areas() {
        let mut app = app_with_data();
        app.page = Page::Servers;
        screen(&mut app, 140, 34);
        assert!(!app.button_areas.is_empty());
        screen(&mut app, 50, 20);
        assert!(app.button_areas.is_empty() && app.list_area == Rect::default() && app.console_area == Rect::default() && app.nav_area == Rect::default() && app.tab_areas.is_empty());
    }

    /// 80칸처럼 탭 줄로 그릴 때도 보이는 탭을 누르면 그 쪽으로 간다
    #[test]
    fn a_visible_tab_can_be_clicked() {
        use ratatui::crossterm::event::{KeyModifiers, MouseButton, MouseEvent, MouseEventKind};
        let mut app = app_with_data();
        let mut term = Terminal::new(TestBackend::new(80, 24)).unwrap();
        term.draw(|f| draw(f, &mut app)).unwrap();
        let buf = term.backend().buffer().clone();
        let first = Page::Servers.title().chars().next().unwrap().to_string();
        // 탭 줄에서 "2 <쪽 이름>"이 놓인 칸을 찾는다
        let (col, row) = (0..buf.area.height)
            .flat_map(|y| (0..buf.area.width.saturating_sub(2)).map(move |x| (x, y)))
            .find(|&(x, y)| buf[(x, y)].symbol() == "2" && buf[(x + 2, y)].symbol() == first)
            .expect("servers tab is drawn");
        app.on_mouse(MouseEvent { kind: MouseEventKind::Down(MouseButton::Left), column: col, row, modifiers: KeyModifiers::NONE });
        assert_eq!(app.page, Page::Servers);
    }

    /// 쪽마다 그려지고 버튼 자리가 마우스용으로 남는다
    #[test]
    fn every_page_draws_with_its_buttons() {
        for page in Page::ALL {
            let mut app = app_with_data();
            app.page = page;
            let t = screen(&mut app, 140, 40);
            assert!(t.contains(&page.title().replace(' ', "")), "{page:?}: {t}");
            if page != Page::Commands {
                assert_eq!(app.button_areas.len(), app.buttons().len(), "{page:?}");
            }
        }
    }

    /// 콘솔은 긴 줄을 접는다: 잘리지 않고 끝까지 보인다
    #[test]
    fn long_console_lines_wrap_instead_of_being_cut() {
        let mut app = app_with_data();
        app.push_line(&format!("https://aka.ms/MinecraftEULA{}", "x".repeat(150)));
        let t = screen(&mut app, 100, 34);
        assert!(t.matches('x').count() >= 150, "{t}");
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
