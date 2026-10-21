//! 그리기. 폭에 따라 배치를 바꾼다: 120칸 이상 목록·결과 나란히, 80칸 이상 메뉴·목록 2단, 60칸 이상 한 단, 그보다 좁으면 안내만

use super::app::{App, AskKind, Focus};
use ratatui::layout::{Alignment, Constraint, Direction, Layout, Rect};
use ratatui::style::{Color, Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, Borders, Clear, List, ListItem, Paragraph, Wrap};
use ratatui::Frame;
use serde_json::Value;

const MIN_WIDTH: u16 = 60;

fn dim() -> Style {
    Style::default().fg(Color::DarkGray)
}

fn highlight(focused: bool) -> Style {
    if focused {
        Style::default().fg(Color::Black).bg(Color::Cyan).add_modifier(Modifier::BOLD)
    } else {
        Style::default().add_modifier(Modifier::REVERSED)
    }
}

fn s<'a>(v: &'a Value, k: &str) -> &'a str {
    v.get(k).and_then(Value::as_str).unwrap_or("")
}

/// 맨 위 두 줄: 버전·데몬·서버 수
fn header(app: &App) -> Vec<Line<'static>> {
    let daemon = match &app.health {
        Some(h) => Span::styled(t!("tui.daemon_up", version = s(h, "version")), Style::default().fg(Color::Green)),
        None => Span::styled(t!("tui.daemon_down"), Style::default().fg(Color::Red)),
    };
    let running = app.servers.iter().filter(|v| s(v, "status") == "running").count();
    vec![
        Line::from(vec![
            Span::styled(
                t!("shell.welcome", version = env!("CARGO_PKG_VERSION")),
                Style::default().fg(Color::Cyan).add_modifier(Modifier::BOLD),
            ),
            Span::raw("   "),
            daemon,
        ]),
        Line::from(Span::styled(
            t!("tui.servers_running", running = running, total = app.servers.len()),
            dim(),
        )),
    ]
}

/// 서버 한 줄: 상태 점, 이름, 상태, 플레이어
fn server_line(v: &Value) -> Line<'static> {
    let status = s(v, "status");
    let (dot, color) = match status {
        "running" => ("●", Color::Green),
        "starting" | "provisioning" | "stopping" => ("◐", Color::Yellow),
        "crashed" | "error" => ("●", Color::Red),
        _ => ("○", Color::DarkGray),
    };
    let label = crate::i18n::try_t(&format!("status.{status}")).unwrap_or_else(|| status.to_string());
    let mut spans = vec![
        Span::styled(format!("{dot} "), Style::default().fg(color)),
        Span::raw(s(v, "name").to_string()),
        Span::styled(format!("  {label}"), dim()),
    ];
    if let Some(p) = crate::output::players(v) {
        spans.push(Span::styled(format!("  👥 {p}"), dim()));
    }
    Line::from(spans)
}

fn draw_menu(f: &mut Frame, app: &mut App, area: Rect) {
    let items: Vec<ListItem> = app.menu.iter().map(|m| ListItem::new(m.name.clone())).collect();
    let list = List::new(items)
        .block(Block::default().borders(Borders::ALL).title(t!("tui.menu")))
        .highlight_style(highlight(app.focus == Focus::Menu));
    app.menu_area = area;
    f.render_stateful_widget(list, area, &mut app.menu_state);
}

fn draw_list(f: &mut Frame, app: &mut App, area: Rect) {
    let item = app.current().clone();
    let title = if item.about.is_empty() { item.name.clone() } else { format!("{} · {}", item.name, item.about) };
    let items: Vec<ListItem> = if app.shows_servers() {
        if app.servers.is_empty() {
            vec![ListItem::new(Span::styled(t!("server.none_yet"), dim()))]
        } else {
            app.servers.iter().map(|v| ListItem::new(server_line(v))).collect()
        }
    } else if item.subs.is_empty() {
        vec![ListItem::new(Line::from(vec![
            Span::raw(item.name.clone()),
            Span::styled(format!("  {}", t!("tui.enter_to_run")), dim()),
        ]))]
    } else {
        item.subs
            .iter()
            .map(|sub| {
                ListItem::new(Line::from(vec![
                    Span::raw(sub.name.clone()),
                    Span::styled(format!("  {}", sub.about), dim()),
                ]))
            })
            .collect()
    };
    let list = List::new(items)
        .block(Block::default().borders(Borders::ALL).title(title))
        .highlight_style(highlight(app.focus == Focus::List));
    app.list_area = area;
    f.render_stateful_widget(list, area, &mut app.list_state);
}

fn draw_output(f: &mut Frame, app: &App, area: Rect) {
    let title = match &app.last_command {
        Some(c) if app.running => format!("› {c}  {}", t!("tui.running_hint")),
        Some(c) => format!("› {c}"),
        None => t!("tui.output"),
    };
    let height = area.height.saturating_sub(2) as usize;
    let end = app.lines.len().saturating_sub(app.scroll);
    let start = end.saturating_sub(height);
    let lines: Vec<Line> = app.lines.range(start..end).map(|l| Line::from(l.clone())).collect();
    let p = Paragraph::new(lines).block(Block::default().borders(Borders::ALL).title(Span::styled(title, dim())));
    f.render_widget(p, area);
}

/// 아래: 작업 줄들과 단축키 (또는 명령 입력 줄)
fn footer(app: &App) -> Vec<Line<'static>> {
    let mut out: Vec<Line> = app
        .jobs
        .values()
        .map(|j| {
            let pct = j.length.filter(|l| *l > 0).map(|l| format!(" {}%", j.position * 100 / l)).unwrap_or_default();
            Line::from(Span::styled(format!("⠿ {}{pct}", j.message), Style::default().fg(Color::Yellow)))
        })
        .collect();
    out.push(match &app.input {
        Some(text) => Line::from(vec![Span::styled(": ", Style::default().fg(Color::Cyan)), Span::raw(format!("{text}▏"))]),
        None => Line::from(Span::styled(t!("tui.keys"), dim())),
    });
    out
}

fn centered(area: Rect, width: u16, height: u16) -> Rect {
    let w = width.min(area.width);
    let h = height.min(area.height);
    Rect { x: area.x + (area.width - w) / 2, y: area.y + (area.height - h) / 2, width: w, height: h }
}

fn draw_popup(f: &mut Frame, title: &str, lines: Vec<Line>, area: Rect) {
    let width = (area.width * 2 / 3).max(40);
    let height = lines.len() as u16 + 2;
    let r = centered(area, width, height);
    f.render_widget(Clear, r);
    f.render_widget(
        Paragraph::new(lines).wrap(Wrap { trim: false }).block(Block::default().borders(Borders::ALL).title(title.to_string())),
        r,
    );
}

fn draw_overlays(f: &mut Frame, app: &App, area: Rect) {
    if let Some((name, cursor)) = &app.actions {
        let lines = super::app::server_actions(name)
            .into_iter()
            .enumerate()
            .map(|(i, (label, cmd))| {
                let style = if i == *cursor { highlight(true) } else { Style::default() };
                Line::from(vec![Span::styled(label, style), Span::styled(format!("  {cmd}"), dim())])
            })
            .collect();
        draw_popup(f, name, lines, area);
    }
    if let Some(d) = &app.dialog {
        let mut lines = vec![Line::from(d.question.clone()), Line::from("")];
        match &d.kind {
            AskKind::Confirm { .. } => {
                let pick = |i: usize, text: String| {
                    Span::styled(format!(" {text} "), if d.cursor == i { highlight(true) } else { Style::default() })
                };
                lines.push(Line::from(vec![pick(0, t!("tui.yes")), Span::raw("  "), pick(1, t!("tui.no"))]));
            }
            AskKind::Select { items } => {
                for (i, it) in items.iter().enumerate() {
                    let style = if d.cursor == i { highlight(true) } else { Style::default() };
                    lines.push(Line::from(Span::styled(it.clone(), style)));
                }
            }
            AskKind::Text { secret, .. } => {
                let shown = if *secret { "•".repeat(d.input.chars().count()) } else { d.input.clone() };
                lines.push(Line::from(format!("> {shown}▏")));
            }
        }
        lines.push(Line::from(Span::styled(t!("tui.dialog_keys"), dim())));
        draw_popup(f, &t!("tui.question"), lines, area);
    }
    if app.show_help {
        let lines = t!("tui.help").lines().map(|l| Line::from(l.to_string())).collect();
        draw_popup(f, &t!("tui.help_title"), lines, area);
    }
}

pub fn draw(f: &mut Frame, app: &mut App) {
    let area = f.area();
    let foot = footer(app);
    let rows = Layout::default()
        .direction(Direction::Vertical)
        .constraints([Constraint::Length(2), Constraint::Min(3), Constraint::Length(foot.len() as u16)])
        .split(area);
    f.render_widget(Paragraph::new(header(app)), rows[0]);
    f.render_widget(Paragraph::new(foot), rows[2]);
    let body = rows[1];
    if area.width < MIN_WIDTH {
        f.render_widget(
            Paragraph::new(t!("tui.too_narrow")).alignment(Alignment::Center).wrap(Wrap { trim: true }),
            body,
        );
        // 명령이 묻는 중이면 좁아도 질문은 그린다
        draw_overlays(f, app, body);
        return;
    }
    if area.width < 80 {
        // 한 단: 메뉴는 목록 제목에 이름으로, ←→ 대신 Tab·↑↓로 메뉴를 고른다
        let parts = Layout::default()
            .direction(Direction::Vertical)
            .constraints([Constraint::Length(menu_height(app)), Constraint::Percentage(40), Constraint::Min(3)])
            .split(body);
        draw_menu(f, app, parts[0]);
        draw_list(f, app, parts[1]);
        draw_output(f, app, parts[2]);
    } else {
        let cols = Layout::default()
            .direction(Direction::Horizontal)
            .constraints([Constraint::Length(18), Constraint::Min(40)])
            .split(body);
        draw_menu(f, app, cols[0]);
        let right = Layout::default()
            .direction(if area.width >= 120 { Direction::Horizontal } else { Direction::Vertical })
            .constraints([Constraint::Percentage(45), Constraint::Percentage(55)])
            .split(cols[1]);
        draw_list(f, app, right[0]);
        draw_output(f, app, right[1]);
    }
    draw_overlays(f, app, body);
}

/// 한 단일 때 메뉴 칸 높이: 고른 줄 앞뒤만 보인다
fn menu_height(app: &App) -> u16 {
    (app.menu.len() as u16 + 2).min(7)
}

#[cfg(test)]
mod tests {
    use super::super::app::{menu_from, App};
    use super::*;
    use ratatui::backend::TestBackend;
    use ratatui::Terminal;
    use serde_json::json;

    fn screen(width: u16, height: u16) -> String {
        crate::i18n::init("ko");
        let mut app = App::new(menu_from(&crate::cli::localized()));
        app.health = Some(json!({ "version": "0.1.9" }));
        app.servers = vec![
            json!({ "name": "pal-1", "status": "running", "players": { "online": 3, "max": 32 } }),
            json!({ "name": "mc-2", "status": "stopped" }),
        ];
        let at = app.menu.iter().position(|m| m.name == "server").unwrap();
        app.menu_state.select(Some(at));
        app.push_line("결과 한 줄");
        let mut term = Terminal::new(TestBackend::new(width, height)).unwrap();
        term.draw(|f| draw(f, &mut app)).unwrap();
        let buf = term.backend().buffer().clone();
        // 두 칸 글자 뒤 칸은 비워 두므로 공백을 빼고 비교한다
        (0..height)
            .map(|y| (0..width).map(|x| buf[(x, y)].symbol().to_string()).collect::<String>().replace(' ', ""))
            .collect::<Vec<_>>()
            .join("\n")
    }

    /// 폭마다 배치가 바뀌어도 상태·서버·결과가 보이고, 좁으면 넓혀 달라는 안내만
    #[test]
    fn the_screen_fits_each_width() {
        for width in [60u16, 80, 120] {
            let text = screen(width, 30);
            assert!(text.contains("0.1.9"), "{width}: {text}");
            assert!(text.contains("pal-1") && text.contains("3/32"), "{width}: {text}");
            assert!(text.contains("결과"), "{width}: {text}");
        }
        let narrow = screen(50, 20);
        assert!(narrow.contains(&t!("tui.too_narrow").replace(' ', "").chars().take(4).collect::<String>()), "{narrow}");
        assert!(!narrow.contains("pal-1"));
    }

    /// 좁아도 묻는 말은 보인다: 보지 못한 질문에 Enter로 답하지 않게
    #[test]
    fn a_question_is_shown_even_on_a_narrow_screen() {
        crate::i18n::init("ko");
        let mut app = App::new(menu_from(&crate::cli::localized()));
        let (reply, _rx) = std::sync::mpsc::channel();
        app.dialog = Some(super::super::app::Dialog {
            question: "데몬을켤까요".into(),
            kind: super::super::app::AskKind::Confirm { default: true },
            cursor: 0,
            input: String::new(),
            reply,
        });
        let mut term = Terminal::new(TestBackend::new(50, 20)).unwrap();
        term.draw(|f| draw(f, &mut app)).unwrap();
        let buf = term.backend().buffer().clone();
        let text: String = (0..20)
            .map(|y| {
                (0..50)
                    .map(|x| buf[(x, y)].symbol().to_string())
                    .collect::<String>()
                    .replace(' ', "")
            })
            .collect();
        assert!(text.contains("데몬을켤까요"), "{text}");
    }
}
