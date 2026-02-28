//! 업데이트 화면

use ratatui::prelude::*;
use ratatui::widgets::{Block, BorderType, Borders, Paragraph};

use crate::tui::app::*;
use crate::tui::theme::Theme;
use crate::tui::render;

pub(super) fn build_updates_menu(app: &App) -> Vec<MenuItem> {
    let mut items = vec![
        MenuItem::new("Check for Updates", Some('c'), "업데이트 확인"),
        MenuItem::new("Update Status", Some('s'), "현재 업데이트 상태 조회"),
    ];

    // cached_update_status가 있으면 동적 메뉴
    if let Some(ref status) = app.cached_update_status {
        if let Some(comps) = status.get("components").and_then(|v| v.as_array()) {
            let any_available = comps.iter().any(|c| c["update_available"].as_bool().unwrap_or(false));
            if any_available {
                items.push(MenuItem::new("⬆ Download & Apply All", Some('A'), "모든 업데이트 다운로드 후 적용"));
            }
        }
    }

    items.extend([
        MenuItem::new("Download Updates", Some('d'), "업데이트 다운로드"),
        MenuItem::new("Apply Updates", Some('a'), "다운로드된 업데이트 적용"),
        MenuItem::new("Updater Config", Some('C'), "업데이터 설정 조회"),
        MenuItem::new("Set Config", Some('S'), "업데이터 설정 변경"),
    ]);
    items
}

pub(super) fn render_updates_screen(app: &App, frame: &mut Frame, area: Rect) {
    let title = if app.daemon_on {
        " Updates "
    } else {
        " Updates — ⚠ Saba-Core offline "
    };
    let block = Block::default()
        .title(title)
        .title_alignment(Alignment::Left)
        .borders(Borders::ALL)
        .border_type(BorderType::Rounded)
        .border_style(if app.daemon_on { Theme::border() } else {
            Style::default().fg(Color::Yellow)
        });

    let inner = block.inner(area);
    frame.render_widget(block, area);

    if !app.daemon_on {
        let warn = vec![
            Line::from(""),
            Line::from(Span::styled(
                "  ⚠ Saba-Core가 실행중이지 않아 업데이트 기능을 사용할 수 없습니다.",
                Style::default().fg(Color::Yellow),
            )),
            Line::from(Span::styled(
                "    'daemon start' 명령어로 Saba-Core를 먼저 시작해주세요.",
                Theme::dimmed(),
            )),
            Line::from(""),
        ];
        frame.render_widget(Paragraph::new(warn), Rect::new(
            inner.x, inner.y, inner.width, 5,
        ));

        render::render_menu(&app.menu_items, app.menu_selected, frame, Rect::new(
            inner.x + 1, inner.y + 5,
            inner.width.saturating_sub(2), inner.height.saturating_sub(6),
        ));
    } else {
        render::render_menu(&app.menu_items, app.menu_selected, frame, Rect::new(
            inner.x + 1, inner.y + 1,
            inner.width.saturating_sub(2), inner.height.saturating_sub(2),
        ));
    }
}

pub(super) fn handle_updates_select(app: &mut App, sel: usize) {
    if !app.daemon_on {
        app.flash("⚠ Saba-Core가 오프라인입니다. 'daemon start'를 먼저 실행하세요.");
        return;
    }

    // 동적 메뉴이므로 shortcut으로 판별
    let shortcut = app.menu_items.get(sel).and_then(|item| item.shortcut);

    let client = app.client.clone();
    let buf = app.async_out.clone();

    match shortcut {
        Some('c') => { // Check
            tokio::spawn(async move {
                match client.check_updates().await {
                    Ok(v) => {
                        let components = v["components"].as_array();
                        let mut lines = vec![];
                        if let Some(comps) = components {
                            let any = comps.iter().any(|c| c["update_available"].as_bool().unwrap_or(false));
                            if any {
                                lines.push(Out::Ok("Updates available:".into()));
                                for c in comps {
                                    let name = c["component"].as_str().unwrap_or("?");
                                    let cur = c["current_version"].as_str().unwrap_or("?");
                                    let lat = c["latest_version"].as_str().unwrap_or("?");
                                    let avail = c["update_available"].as_bool().unwrap_or(false);
                                    let marker = if avail { "⬆" } else { "✓" };
                                    lines.push(Out::Text(format!("  {} {:<20} {} → {}", marker, name, cur, lat)));
                                }
                            } else {
                                lines.push(Out::Ok("All components are up to date.".into()));
                            }
                        } else {
                            lines.push(Out::Ok(format!("{}", v)));
                        }
                        push_out(&buf, lines);
                    }
                    Err(e) => push_out(&buf, vec![Out::Err(format!("✗ {}", e))]),
                }
            });
            app.flash("업데이트 확인 중...");
        }
        Some('s') => { // Status
            tokio::spawn(async move {
                match client.get_update_status().await {
                    Ok(v) => {
                        let mut lines = vec![Out::Ok("Update Status:".into())];
                        let checked = v["last_checked"].as_str().unwrap_or("never");
                        lines.push(Out::Text(format!("  Last checked: {}", checked)));
                        if let Some(comps) = v["components"].as_array() {
                            for c in comps {
                                let name = c["component"].as_str().unwrap_or("?");
                                let cur = c["current_version"].as_str().unwrap_or("?");
                                let dl = if c["downloaded"].as_bool().unwrap_or(false) { " [downloaded]" } else { "" };
                                lines.push(Out::Text(format!("  {:<20} v{}{}", name, cur, dl)));
                            }
                        }
                        push_out(&buf, lines);
                    }
                    Err(e) => push_out(&buf, vec![Out::Err(format!("✗ {}", e))]),
                }
            });
        }
        Some('A') => { // Download & Apply All
            tokio::spawn(async move {
                match client.download_updates().await {
                    Ok(_) => {
                        push_out(&buf, vec![Out::Ok("✓ Download complete. Applying...".into())]);
                        match client.apply_updates().await {
                            Ok(v) => {
                                let msg = v.get("message").and_then(|m| m.as_str()).unwrap_or("Updates applied");
                                push_out(&buf, vec![Out::Ok(format!("✓ {}", msg))]);
                            }
                            Err(e) => push_out(&buf, vec![Out::Err(format!("✗ Apply: {}", e))]),
                        }
                    }
                    Err(e) => push_out(&buf, vec![Out::Err(format!("✗ Download: {}", e))]),
                }
            });
            app.flash("다운로드 & 적용 중...");
        }
        Some('d') => { // Download
            tokio::spawn(async move {
                match client.download_updates().await {
                    Ok(v) => {
                        let msg = v.get("message").and_then(|m| m.as_str()).unwrap_or("Download initiated");
                        push_out(&buf, vec![Out::Ok(format!("✓ {}", msg))]);
                    }
                    Err(e) => push_out(&buf, vec![Out::Err(format!("✗ {}", e))]),
                }
            });
            app.flash("다운로드 중...");
        }
        Some('a') => { // Apply
            tokio::spawn(async move {
                match client.apply_updates().await {
                    Ok(v) => {
                        let msg = v.get("message").and_then(|m| m.as_str()).unwrap_or("Updates applied");
                        push_out(&buf, vec![Out::Ok(format!("✓ {}", msg))]);
                    }
                    Err(e) => push_out(&buf, vec![Out::Err(format!("✗ {}", e))]),
                }
            });
            app.flash("업데이트 적용 중...");
        }
        Some('C') => { // Config
            tokio::spawn(async move {
                match client.get_update_config().await {
                    Ok(v) => {
                        let mut lines = vec![Out::Ok("Updater Config:".into())];
                        if let Some(map) = v.as_object() {
                            for (k, val) in map {
                                lines.push(Out::Text(format!("  {}: {}", k, val)));
                            }
                        }
                        lines.push(Out::Blank);
                        lines.push(Out::Info("메뉴에서 'Set Config'으로 변경 가능".into()));
                        push_out(&buf, lines);
                    }
                    Err(e) => push_out(&buf, vec![Out::Err(format!("✗ {}", e))]),
                }
            });
            app.flash("설정 조회 중...");
        }
        Some('S') => { // Set Config → 인라인 Input
            app.input_mode = InputMode::InlineInput {
                prompt: "업데이터 설정 (key=value)".into(),
                value: String::new(),
                cursor: 0,
                on_submit: InlineAction::UpdateSet,
            };
        }
        _ => {}
    }
}
