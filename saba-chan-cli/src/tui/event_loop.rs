//! 이벤트 루프 · 키/마우스 라우팅 · 비동기 플러시
//!
//! `mod.rs`의 setup/teardown 코드 이후에 호출되는 메인 루프 본체와
//! 키 이벤트/마우스/인라인 액션/비동기 메시지 처리 로직을 담당한다.
//!
//! 역사적으로 모든 로직이 `mod.rs`에 한 덩어리로 있었으나, Stage 0 리팩터링에서
//! 포커스·오버레이·탭 등 계층을 확장할 수 있도록 별도 모듈로 분리했다. 기능 변화는 없다.
//!
//! ## 키 이벤트 디스패치 계층 (현재)
//!
//! ```text
//! Layer 1 — Ctrl+C / Ctrl+D  (run()에서 처리, 이 모듈 진입 전에 필터)
//! Layer 2 — InputMode 기반 라우팅 (Normal/Command/Editing/Console/Confirm/InlineInput/InlineSelect)
//! ```
//!
//! 이후 Stage에서 Overlay/Slash/Tab 레이어가 이 앞단에 삽입된다.

use std::io::{self, Stdout};
use std::time::{Duration, Instant};

use crossterm::event::{
    self, DisableMouseCapture, Event, KeyCode, KeyEvent, KeyEventKind, KeyModifiers,
    MouseEvent, MouseEventKind,
};
use crossterm::execute;
use crossterm::terminal::{disable_raw_mode, LeaveAlternateScreen};
use ratatui::prelude::*;
use tokio::sync::mpsc;

use super::app::*;
use super::commands;
use super::render;
use super::screens;
use super::tabs::{LogLine, TabEvent};
use crate::client::DaemonClient;
use crate::config;

// ═══════════════════════════════════════════════════════
// 메인 이벤트 루프
// ═══════════════════════════════════════════════════════

/// 터미널 초기화 이후의 메인 루프.
///
/// `run()`이 수행해야 할 책임(터미널 모드 토글, 백그라운드 태스크 스폰 등)은
/// 호출 측에 남겨두고, 이 함수는 순수하게 "이벤트 수신 → 상태 갱신 → 렌더"만 담당한다.
pub async fn run_event_loop(
    app: &mut App,
    terminal: &mut Terminal<CrosstermBackend<Stdout>>,
    client: &DaemonClient,
    status_rx: &mut mpsc::UnboundedReceiver<Snapshot>,
) -> anyhow::Result<()> {
    let tick = Duration::from_millis(50);
    let mut last_render = Instant::now();

    // 탭 폴러가 보내는 이벤트를 이 루프가 일괄 수신한다.
    // `App::new`에서 채널을 만들어 tx는 App이 보유, rx는 여기서 take.
    let mut tab_rx = app.tab_rx.take()
        .expect("tab_rx must be initialized in App::new and only taken once");

    loop {
        // --- 비동기 결과 수신 ---
        while let Ok(snap) = status_rx.try_recv() {
            app.apply_status(snap);
        }
        while let Ok(ev) = tab_rx.try_recv() {
            app.tabs.apply_event(ev);
        }
        flush_async_with_fields(app);

        // --- 메뉴 항목 재생성 ---
        app.menu_items = screens::build_menu(app);
        if app.menu_selected >= app.menu_items.len() && !app.menu_items.is_empty() {
            app.menu_selected = app.menu_items.len() - 1;
        }

        // 콘솔 폴링은 `tabs::spawn_server_poller`가 전담한다 (탭이 열릴 때 시작, 닫힐 때 abort).

        // --- 데몬 로그 스트리밍 (콘솔 모드에서 모든 컴포넌트 로그 표시) ---
        if matches!(app.screen, Screen::CommandMode) && app.daemon_on {
            let should_fetch = app.last_daemon_log_fetch
                .map(|t| t.elapsed() >= Duration::from_secs(1))
                .unwrap_or(true);
            if should_fetch {
                let since_id = app.last_daemon_log_id;
                let client = client.clone();
                let buf = app.async_out.clone();
                tokio::spawn(async move {
                    if let Ok(data) = client.get_daemon_console(Some(since_id), Some(50)).await {
                        if let Some(lines) = data.get("lines").and_then(|v| v.as_array()) {
                            let out: Vec<Out> = lines.iter().filter_map(|entry| {
                                let id = entry.get("id").and_then(|v| v.as_u64()).unwrap_or(0);
                                let level = entry.get("level").and_then(|v| v.as_str()).unwrap_or("info");
                                let target = entry.get("target").and_then(|v| v.as_str()).unwrap_or("");
                                let msg = entry.get("message").and_then(|v| v.as_str()).unwrap_or("");
                                if msg.is_empty() { return None; }
                                Some(Out::Text(format!("DAEMON_LOG:{}:{}:{}: {}", id, level, target, msg)))
                            }).collect();
                            if !out.is_empty() {
                                push_out(&buf, out);
                            }
                        }
                    }
                });
                app.last_daemon_log_fetch = Some(Instant::now());
            }
        } else {
            app.last_daemon_log_fetch = None;
        }

        // --- 렌더링 ---
        if last_render.elapsed() >= Duration::from_millis(16) {
            // output_height를 터미널 크기에 맞게 갱신
            // 레이아웃: 상태바 4 + 힌트바 2 + 커맨드입력 3 + 테두리 2 = 약 11줄 오버헤드
            let term_height = terminal.size().map(|s| s.height as usize).unwrap_or(24);
            app.output_height = term_height.saturating_sub(11);

            terminal.draw(|f| {
                render::render(app, f);
            })?;
            last_render = Instant::now();
        }

        if app.quit { break; }

        // --- 이벤트 폴링 ---
        if event::poll(tick)? {
            match event::read()? {
                Event::Key(key) => {
                    // Release/Repeat 이벤트 무시 — Press만 처리
                    if key.kind != KeyEventKind::Press {
                        continue;
                    }

                    // Ctrl+C/Ctrl+D → 강제 종료
                    if key.modifiers.contains(KeyModifiers::CONTROL)
                        && matches!(key.code, KeyCode::Char('c') | KeyCode::Char('d'))
                    {
                        app.quit = true;
                        continue;
                    }

                    dispatch_key(app, key);
                }
                Event::Mouse(mouse) => {
                    dispatch_mouse(app, mouse);
                }
                _ => {}
            }
        }
    }

    Ok(())
}

// ═══════════════════════════════════════════════════════
// 키 이벤트 라우팅
// ═══════════════════════════════════════════════════════

/// 활성 오버레이의 키 이벤트 처리. 소비 시 `true` 반환.
/// Esc → 오버레이 닫기, Enter → 확정, 그 외 편집 키 → 쿼리 갱신.
fn handle_overlay_key(app: &mut App, key: &KeyEvent) -> bool {
    use super::overlay::Overlay;
    // borrow 규칙 회피: take()로 소유권 가져와 수정 후 다시 넣거나 None으로 닫음.
    let Some(mut ov) = app.overlay.take() else { return false; };
    let (keep, new_filter): (bool, Option<Option<String>>) = match &mut ov {
        Overlay::Search(s) => match key.code {
            KeyCode::Esc => (false, Some(None)),
            KeyCode::Enter => {
                let q = s.query.trim().to_string();
                (false, Some(if q.is_empty() { None } else { Some(q) }))
            }
            KeyCode::Backspace => {
                s.backspace();
                let q = s.query.trim().to_string();
                (true, Some(if q.is_empty() { None } else { Some(q) }))
            }
            KeyCode::Delete => {
                s.delete();
                let q = s.query.trim().to_string();
                (true, Some(if q.is_empty() { None } else { Some(q) }))
            }
            KeyCode::Left => { s.move_left(); (true, None) }
            KeyCode::Right => { s.move_right(); (true, None) }
            KeyCode::Home => { s.cursor = 0; (true, None) }
            KeyCode::End => { s.cursor = s.query.chars().count(); (true, None) }
            KeyCode::Char(c) => {
                if key.modifiers.contains(KeyModifiers::CONTROL) {
                    (true, None)
                } else {
                    s.insert_char(c);
                    let q = s.query.trim().to_string();
                    (true, Some(if q.is_empty() { None } else { Some(q) }))
                }
            }
            _ => (true, None),
        },
    };

    if let Some(f) = new_filter {
        if let Some(tab) = app.tabs.active_tab_mut() {
            tab.filter = f;
        }
    }

    if keep {
        app.overlay = Some(ov);
    }
    true
}

/// 슬래시 버퍼 활성 상태에서의 키 처리. 소비 시 `true`.
fn handle_slash_key(app: &mut App, key: &KeyEvent) -> bool {
    let Some(ref mut buf) = app.slash_buffer else { return false; };
    match key.code {
        KeyCode::Esc => {
            app.slash_buffer = None;
            app.slash_cursor = 0;
            app.ghost_suffix = None;
            app.autocomplete_candidates.clear();
            app.autocomplete_visible = false;
        }
        KeyCode::Enter => {
            let body = std::mem::take(buf);
            app.slash_buffer = None;
            app.slash_cursor = 0;
            app.ghost_suffix = None;
            app.autocomplete_candidates.clear();
            app.autocomplete_visible = false;
            // Stage 5: 슬래시 히스토리에 기록 (비어있지 않을 때만)
            app.push_slash_history(body.clone());
            let cmd = super::slash::parse(&body);
            super::slash::execute(app, cmd);
        }
        KeyCode::Tab => {
            if !super::autocomplete::accept_ghost(app) {
                // ghost가 없으면 첫 후보 삽입
                if app.autocomplete_visible && !app.autocomplete_candidates.is_empty() {
                    let top = app.autocomplete_candidates[app.autocomplete_selected].clone();
                    if let Some(buf) = app.slash_buffer.as_mut() {
                        *buf = top;
                        app.slash_cursor = buf.chars().count();
                    }
                    super::autocomplete::refresh(app);
                }
            } else {
                super::autocomplete::refresh(app);
            }
        }
        KeyCode::Backspace => {
            if app.slash_cursor > 0 {
                let byte_pos = char_to_byte(buf, app.slash_cursor - 1);
                let end_pos = char_to_byte(buf, app.slash_cursor);
                buf.replace_range(byte_pos..end_pos, "");
                app.slash_cursor -= 1;
            }
            super::autocomplete::refresh(app);
        }
        KeyCode::Left => {
            if app.slash_cursor > 0 { app.slash_cursor -= 1; }
        }
        KeyCode::Right => {
            let max = buf.chars().count();
            if app.slash_cursor < max { app.slash_cursor += 1; }
        }
        KeyCode::Home => app.slash_cursor = 0,
        KeyCode::End => app.slash_cursor = buf.chars().count(),
        KeyCode::Up => {
            // Stage 5: 슬래시 히스토리 탐색
            let _ = buf;
            app.slash_history_prev();
            super::autocomplete::refresh(app);
        }
        KeyCode::Down => {
            let _ = buf;
            app.slash_history_next();
            super::autocomplete::refresh(app);
        }
        KeyCode::Char(c) => {
            if key.modifiers.contains(KeyModifiers::CONTROL) {
                return true;
            }
            let byte_pos = char_to_byte(buf, app.slash_cursor);
            buf.insert(byte_pos, c);
            app.slash_cursor += 1;
            super::autocomplete::refresh(app);
        }
        _ => {}
    }
    true
}

/// Press 이벤트 진입점. `Focus` 레이어링은 이후 Stage에서 여기에 삽입된다.
/// 현재는 `InputMode` 단일 match와 동일한 동작을 한다.
pub(super) fn dispatch_key(app: &mut App, key: KeyEvent) {
    // Stage 0: Focus는 아직 모든 variant가 Menu처럼 동작 — 분기 없음.
    // 이후 Stage에서 Overlay/Slash/Tab 계층이 이 위치에 삽입된다.
    let _ = app.focus; // suppress unused-field warning until Stage 1

    // Stage 3: Overlay 레이어 — 활성 오버레이가 키 이벤트를 우선 소비
    if app.overlay.is_some() && handle_overlay_key(app, &key) {
        return;
    }

    // Stage 4: 슬래시 모드 — Normal 이외에서도 활성 시 우선 소비
    if app.slash_buffer.is_some() && handle_slash_key(app, &key) { return; }

    // Stage 3: 글로벌 단축키 — Ctrl+F 검색 오버레이 진입
    if key.modifiers.contains(KeyModifiers::CONTROL)
        && matches!(key.code, KeyCode::Char('f') | KeyCode::Char('F'))
        && !matches!(app.input_mode, InputMode::Editing | InputMode::InlineInput { .. })
    {
        app.overlay = Some(super::overlay::Overlay::Search(
            super::overlay::SearchOverlay::new()
        ));
        return;
    }

    match app.input_mode {
        // ────────────────
        // 노멀 모드 (메뉴)
        // ────────────────
        InputMode::Normal => {
            // 화면별 키 핸들러 (화면별 특수 키 먼저)
            if screens::handle_screen_key(app, &key) { return; }

            match key.code {
                // 메뉴 내비게이션
                KeyCode::Up | KeyCode::Char('k') => app.menu_up(),
                KeyCode::Down | KeyCode::Char('j') => app.menu_down(),
                KeyCode::Home => app.menu_selected = 0,
                KeyCode::End => if !app.menu_items.is_empty() { app.menu_selected = app.menu_items.len() - 1; },
                KeyCode::Enter => {
                    screens::handle_menu_select(app);
                }
                // Esc → 뒤로
                KeyCode::Esc => {
                    if !app.pop_screen() {
                        // 이미 대시보드면 → 종료 확인
                        app.flash("Press 'q' to quit");
                    }
                }
                // 커맨드 모드 진입
                KeyCode::Char(':') => {
                    app.push_screen(Screen::CommandMode);
                    app.input_mode = InputMode::Command;
                    app.input.clear();
                    app.cursor = 0;
                }
                // Stage 4: 슬래시 명령 진입 — 전역 오버레이 느낌으로 바닥에서 입력
                KeyCode::Char('/') => {
                    app.slash_buffer = Some(String::new());
                    app.slash_cursor = 0;
                    super::autocomplete::refresh(app);
                }
                // 에디터 진입 (설정/속성 화면에서)
                KeyCode::Char('i') => {
                    if !app.editor_fields.is_empty() {
                        app.enter_edit_mode();
                    }
                }
                // 에디터 변경사항 저장
                KeyCode::Char('w') => {
                    if !app.editor_changes.is_empty() {
                        screens::save_editor_changes(app);
                    }
                }
                // 숫자 단축키
                KeyCode::Char(c @ '1'..='9') => {
                    if app.try_shortcut(c) {
                        screens::handle_menu_select(app);
                    }
                }
                KeyCode::Char('0') => {
                    if app.try_shortcut('0') {
                        screens::handle_menu_select(app);
                    }
                }
                // 종료
                KeyCode::Char('q') => {
                    app.quit = true;
                }
                // 도움말 (커맨드 모드에서 help 실행)
                KeyCode::Char('?') | KeyCode::F(1) => {
                    app.push_screen(Screen::CommandMode);
                    app.input_mode = InputMode::Command;
                    app.input = "help".into();
                    app.cursor = 4;
                    commands::submit(app);
                }
                _ => {}
            }
        }

        // ──────────────────
        // 커맨드 모드 (타이핑)
        // ──────────────────
        InputMode::Command => {
            match key.code {
                KeyCode::Esc => {
                    if app.autocomplete_visible {
                        // 자동완성 팝업만 닫기
                        app.autocomplete_visible = false;
                        app.autocomplete_candidates.clear();
                    } else {
                        app.input.clear();
                        app.cursor = 0;
                        // 스택이 있으면 이전 화면으로, 없으면 무시 (기본 모드)
                        if !app.screen_stack.is_empty() {
                            app.pop_screen();
                        }
                    }
                }
                // F2 → 인터랙티브 메뉴 모드 진입
                KeyCode::F(2) => {
                    app.input.clear();
                    app.cursor = 0;
                    // CommandMode를 스택에서 제거 후 Dashboard로 교체
                    app.pop_screen();
                    app.push_screen(Screen::Dashboard);
                }
                KeyCode::Enter => {
                    if app.autocomplete_visible && !app.autocomplete_candidates.is_empty() {
                        // 선택된 자동완성 후보를 입력으로 적용
                        if let Some(candidate) = app.autocomplete_candidates.get(app.autocomplete_selected) {
                            app.input = format!("{} ", candidate);
                            app.cursor = app.input.chars().count();
                        }
                        app.autocomplete_candidates.clear();
                        app.autocomplete_visible = false;
                    } else if app.input.trim().is_empty() {
                        // 스택이 있으면 이전 화면으로, 없으면 무시
                        if !app.screen_stack.is_empty() {
                            app.pop_screen();
                        }
                    } else {
                        app.autocomplete_visible = false;
                        app.autocomplete_candidates.clear();
                        commands::submit(app);
                    }
                }
                KeyCode::Tab => {
                    // Stage 4: ghost-text 먼저 수락
                    if super::autocomplete::accept_ghost(app) {
                        commands::update_autocomplete_preview(app);
                    } else if app.autocomplete_visible && !app.autocomplete_candidates.is_empty() {
                        // 후보 순회
                        app.autocomplete_selected =
                            (app.autocomplete_selected + 1) % app.autocomplete_candidates.len();
                    } else {
                        commands::update_autocomplete_preview(app);
                        if !app.autocomplete_visible {
                            commands::autocomplete(app);
                        }
                    }
                }
                KeyCode::BackTab => { // Shift+Tab
                    if app.autocomplete_visible && !app.autocomplete_candidates.is_empty() {
                        if app.autocomplete_selected == 0 {
                            app.autocomplete_selected = app.autocomplete_candidates.len() - 1;
                        } else {
                            app.autocomplete_selected -= 1;
                        }
                    }
                }
                // 출력 스크롤 (PgUp/PgDn)
                KeyCode::PageUp => {
                    let max = app.output.len().saturating_sub(app.output_height);
                    app.scroll_up = (app.scroll_up + app.output_height.max(1)).min(max);
                }
                KeyCode::PageDown => {
                    app.scroll_up = app.scroll_up.saturating_sub(app.output_height.max(1));
                }
                KeyCode::Backspace => {
                    if app.cursor > 0 {
                        let byte_pos = char_to_byte(&app.input, app.cursor - 1);
                        let end_pos = char_to_byte(&app.input, app.cursor);
                        app.input = format!("{}{}", &app.input[..byte_pos], &app.input[end_pos..]);
                        app.cursor -= 1;
                    }
                    commands::update_autocomplete_preview(app);
                }
                KeyCode::Delete => {
                    let char_len = app.input.chars().count();
                    if app.cursor < char_len {
                        let byte_pos = char_to_byte(&app.input, app.cursor);
                        let end_pos = char_to_byte(&app.input, app.cursor + 1);
                        app.input = format!("{}{}", &app.input[..byte_pos], &app.input[end_pos..]);
                    }
                }
                KeyCode::Left => { if app.cursor > 0 { app.cursor -= 1; } }
                KeyCode::Right => { let max = app.input.chars().count(); if app.cursor < max { app.cursor += 1; } }
                KeyCode::Home => app.cursor = 0,
                KeyCode::End => app.cursor = app.input.chars().count(),
                KeyCode::Up => app.history_prev(),
                KeyCode::Down => app.history_next(),
                KeyCode::Char(c) => {
                    let byte_pos = char_to_byte(&app.input, app.cursor);
                    app.input.insert(byte_pos, c);
                    app.cursor += 1;
                    // ★ 입력 후 자동완성 미리보기 갱신
                    commands::update_autocomplete_preview(app);
                }
                _ => {}
            }
        }

        // ──────────────────
        // 필드 편집 모드
        // ──────────────────
        InputMode::Editing => {
            match key.code {
                KeyCode::Esc => app.cancel_edit(),
                KeyCode::Enter => app.commit_edit(),
                KeyCode::Backspace => {
                    if app.edit_cursor > 0 {
                        let byte_pos = char_to_byte(&app.edit_buffer, app.edit_cursor - 1);
                        let end_pos = char_to_byte(&app.edit_buffer, app.edit_cursor);
                        app.edit_buffer = format!("{}{}", &app.edit_buffer[..byte_pos], &app.edit_buffer[end_pos..]);
                        app.edit_cursor -= 1;
                    }
                }
                KeyCode::Delete => {
                    let char_len = app.edit_buffer.chars().count();
                    if app.edit_cursor < char_len {
                        let byte_pos = char_to_byte(&app.edit_buffer, app.edit_cursor);
                        let end_pos = char_to_byte(&app.edit_buffer, app.edit_cursor + 1);
                        app.edit_buffer = format!("{}{}", &app.edit_buffer[..byte_pos], &app.edit_buffer[end_pos..]);
                    }
                }
                KeyCode::Left => { if app.edit_cursor > 0 { app.edit_cursor -= 1; } }
                KeyCode::Right => { let max = app.edit_buffer.chars().count(); if app.edit_cursor < max { app.edit_cursor += 1; } }
                KeyCode::Home => app.edit_cursor = 0,
                KeyCode::End => app.edit_cursor = app.edit_buffer.chars().count(),
                KeyCode::Char(c) => {
                    let byte_pos = char_to_byte(&app.edit_buffer, app.edit_cursor);
                    app.edit_buffer.insert(byte_pos, c);
                    app.edit_cursor += 1;
                }
                _ => {}
            }
        }

        // ──────────────────
        // 콘솔 모드 (stdin 입력) — 활성 탭 기반
        // ──────────────────
        InputMode::Console => {
            // Ctrl+Tab / Ctrl+Shift+Tab / Ctrl+W 전역 처리 — Console 모드에서만 활성
            if key.modifiers.contains(KeyModifiers::CONTROL) {
                match key.code {
                    KeyCode::Tab => { app.tabs.cycle_next(); return; }
                    KeyCode::BackTab => { app.tabs.cycle_prev(); return; }
                    KeyCode::Char('w') => {
                        app.tabs.close_active();
                        if app.tabs.active.is_none() {
                            app.input_mode = InputMode::Normal;
                            app.pop_screen();
                        }
                        return;
                    }
                    _ => {}
                }
            }

            match key.code {
                KeyCode::Esc => {
                    app.input_mode = InputMode::Normal;
                    app.pop_screen();
                }
                KeyCode::Enter => {
                    let text = app.tabs.active_tab_mut().map(|t| {
                        let s = std::mem::take(&mut t.stdin);
                        t.stdin_cursor = 0;
                        s
                    }).unwrap_or_default();
                    let text = text.trim().to_string();
                    if text.is_empty() { return; }
                    // stdin 전송 대상 (tab source별 라우팅)
                    enum StdinTarget { Server(String), ExtProcess(String), None }
                    let send_target = app.tabs.active_tab().map(|t| match &t.source {
                        super::tabs::TabSource::Server { id, .. } => StdinTarget::Server(id.clone()),
                        super::tabs::TabSource::ExtProcess { name } => StdinTarget::ExtProcess(name.clone()),
                        super::tabs::TabSource::Daemon => StdinTarget::None,
                    }).unwrap_or(StdinTarget::None);
                    // 로컬 에코
                    if let Some(tab) = app.tabs.active_tab_mut() {
                        tab.push_line(LogLine::stdin_echo(format!("> {}", text)));
                    }
                    let client = app.client.clone();
                    let tx = app.tab_tx.clone();
                    let tab_id = app.tabs.active.unwrap_or(0);
                    match send_target {
                        StdinTarget::Server(iid) => {
                            tokio::spawn(async move {
                                if let Err(e) = client.send_stdin(&iid, &text).await {
                                    let _ = tx.send(TabEvent::NewLines {
                                        tab_id,
                                        lines: vec![LogLine::error(format!("✗ stdin: {}", e))],
                                    });
                                }
                            });
                        }
                        StdinTarget::ExtProcess(name) => {
                            tokio::spawn(async move {
                                if let Err(e) = client.send_ext_process_stdin(&name, &text).await {
                                    let _ = tx.send(TabEvent::NewLines {
                                        tab_id,
                                        lines: vec![LogLine::error(format!("✗ stdin: {}", e))],
                                    });
                                }
                            });
                        }
                        StdinTarget::None => {
                            if let Some(tab) = app.tabs.active_tab_mut() {
                                tab.push_line(LogLine::warn(app.i18n.t("errors.tab_no_input")));
                            }
                        }
                    }
                }
                KeyCode::Backspace => {
                    if let Some(tab) = app.tabs.active_tab_mut() {
                        if tab.stdin_cursor > 0 {
                            let byte_pos = char_to_byte(&tab.stdin, tab.stdin_cursor - 1);
                            let end_pos = char_to_byte(&tab.stdin, tab.stdin_cursor);
                            tab.stdin = format!("{}{}", &tab.stdin[..byte_pos], &tab.stdin[end_pos..]);
                            tab.stdin_cursor -= 1;
                        }
                    }
                }
                KeyCode::Delete => {
                    if let Some(tab) = app.tabs.active_tab_mut() {
                        let char_len = tab.stdin.chars().count();
                        if tab.stdin_cursor < char_len {
                            let byte_pos = char_to_byte(&tab.stdin, tab.stdin_cursor);
                            let end_pos = char_to_byte(&tab.stdin, tab.stdin_cursor + 1);
                            tab.stdin = format!("{}{}", &tab.stdin[..byte_pos], &tab.stdin[end_pos..]);
                        }
                    }
                }
                KeyCode::Left => {
                    if let Some(tab) = app.tabs.active_tab_mut() {
                        if tab.stdin_cursor > 0 { tab.stdin_cursor -= 1; }
                    }
                }
                KeyCode::Right => {
                    if let Some(tab) = app.tabs.active_tab_mut() {
                        let max = tab.stdin.chars().count();
                        if tab.stdin_cursor < max { tab.stdin_cursor += 1; }
                    }
                }
                KeyCode::Home => {
                    if let Some(tab) = app.tabs.active_tab_mut() { tab.stdin_cursor = 0; }
                }
                KeyCode::End => {
                    if let Some(tab) = app.tabs.active_tab_mut() {
                        tab.stdin_cursor = tab.stdin.chars().count();
                    }
                }
                KeyCode::Char(c) => {
                    if let Some(tab) = app.tabs.active_tab_mut() {
                        let byte_pos = char_to_byte(&tab.stdin, tab.stdin_cursor);
                        tab.stdin.insert(byte_pos, c);
                        tab.stdin_cursor += 1;
                    }
                }
                KeyCode::Up => {
                    if let Some(tab) = app.tabs.active_tab_mut() {
                        let max = tab.buffer.len().saturating_sub(1);
                        if tab.scroll < max { tab.scroll += 1; }
                    }
                }
                KeyCode::Down => {
                    if let Some(tab) = app.tabs.active_tab_mut() {
                        if tab.scroll > 0 { tab.scroll -= 1; }
                    }
                }
                KeyCode::PageUp => {
                    if let Some(tab) = app.tabs.active_tab_mut() {
                        tab.scroll = (tab.scroll + 10).min(tab.buffer.len().saturating_sub(1));
                    }
                }
                KeyCode::PageDown => {
                    if let Some(tab) = app.tabs.active_tab_mut() {
                        tab.scroll = tab.scroll.saturating_sub(10);
                    }
                }
                _ => {}
            }
        }

        // ── 확인 모드 (y/n) ──
        InputMode::Confirm { .. } => {
            match key.code {
                KeyCode::Char('y') | KeyCode::Char('Y') | KeyCode::Enter => {
                    // InputMode에서 action을 추출
                    if let InputMode::Confirm { action, .. } =
                        std::mem::replace(&mut app.input_mode, InputMode::Normal)
                    {
                        screens::execute_confirm(app, action);
                    }
                }
                _ => {
                    app.input_mode = InputMode::Normal;
                }
            }
        }

        // ── 인라인 텍스트 입력 모드 ──
        InputMode::InlineInput { ref mut value, ref mut cursor, .. } => {
            match key.code {
                KeyCode::Esc => {
                    app.input_mode = InputMode::Normal;
                }
                KeyCode::Enter => {
                    if let InputMode::InlineInput { value, on_submit, .. } =
                        std::mem::replace(&mut app.input_mode, InputMode::Normal)
                    {
                        execute_inline_action(app, on_submit, &value);
                    }
                }
                KeyCode::Char(c) => {
                    let byte_pos = char_to_byte(value, *cursor);
                    value.insert(byte_pos, c);
                    *cursor += 1;
                }
                KeyCode::Backspace => {
                    if *cursor > 0 {
                        let byte_pos = char_to_byte(value, *cursor - 1);
                        let end_pos = char_to_byte(value, *cursor);
                        *value = format!("{}{}", &value[..byte_pos], &value[end_pos..]);
                        *cursor -= 1;
                    }
                }
                KeyCode::Delete => {
                    let char_len = value.chars().count();
                    if *cursor < char_len {
                        let byte_pos = char_to_byte(value, *cursor);
                        let end_pos = char_to_byte(value, *cursor + 1);
                        *value = format!("{}{}", &value[..byte_pos], &value[end_pos..]);
                    }
                }
                KeyCode::Left => { if *cursor > 0 { *cursor -= 1; } }
                KeyCode::Right => { let max = value.chars().count(); if *cursor < max { *cursor += 1; } }
                KeyCode::Home => *cursor = 0,
                KeyCode::End => *cursor = value.chars().count(),
                _ => {}
            }
        }

        // ── 인라인 선택 모드 ──
        InputMode::InlineSelect { ref mut selected, ref options, .. } => {
            match key.code {
                KeyCode::Esc => {
                    app.input_mode = InputMode::Normal;
                }
                KeyCode::Up | KeyCode::Char('k') => {
                    if *selected > 0 { *selected -= 1; } else { *selected = options.len().saturating_sub(1); }
                }
                KeyCode::Down | KeyCode::Char('j') => {
                    if !options.is_empty() { *selected = (*selected + 1) % options.len(); }
                }
                KeyCode::Enter => {
                    if let InputMode::InlineSelect { options, selected, on_submit, .. } =
                        std::mem::replace(&mut app.input_mode, InputMode::Normal)
                    {
                        let value = options.get(selected).cloned().unwrap_or_default();
                        execute_inline_action(app, on_submit, &value);
                    }
                }
                _ => {}
            }
        }
    }
}

// ═══════════════════════════════════════════════════════
// 마우스 이벤트 처리
// ═══════════════════════════════════════════════════════

pub(super) fn dispatch_mouse(app: &mut App, mouse: MouseEvent) {
    match mouse.kind {
        MouseEventKind::ScrollUp => {
            match &app.screen {
                Screen::CommandMode => {
                    let max = app.output.len().saturating_sub(app.output_height);
                    app.scroll_up = (app.scroll_up + 3).min(max);
                }
                Screen::ServerConsole { .. } => {
                    if let Some(tab) = app.tabs.active_tab_mut() {
                        let max = tab.buffer.len().saturating_sub(1);
                        tab.scroll = (tab.scroll + 3).min(max);
                    }
                }
                Screen::ServerSettings { .. } | Screen::ServerProperties { .. } => {
                    for _ in 0..3 { app.editor_up(); }
                }
                _ => {
                    for _ in 0..3 { app.menu_up(); }
                }
            }
        }
        MouseEventKind::ScrollDown => {
            match &app.screen {
                Screen::CommandMode => {
                    app.scroll_up = app.scroll_up.saturating_sub(3);
                }
                Screen::ServerConsole { .. } => {
                    if let Some(tab) = app.tabs.active_tab_mut() {
                        tab.scroll = tab.scroll.saturating_sub(3);
                    }
                }
                Screen::ServerSettings { .. } | Screen::ServerProperties { .. } => {
                    for _ in 0..3 { app.editor_down(); }
                }
                _ => {
                    for _ in 0..3 { app.menu_down(); }
                }
            }
        }
        _ => {} // 클릭 등은 추후 확장
    }
}

// ═══════════════════════════════════════════════════════
// 인라인 액션 실행
// ═══════════════════════════════════════════════════════

pub(super) fn execute_inline_action(app: &mut App, action: InlineAction, value: &str) {
    match action {
        InlineAction::CreateInstance { module_name } => {
            let name = value.trim().to_string();
            if name.is_empty() {
                let msg = app.i18n.t("flash.name_required");
                app.flash(&msg);
                return;
            }
            let client = app.client.clone();
            let buf = app.async_out.clone();
            let data = serde_json::json!({
                "name": name,
                "module_name": module_name,
            });
            let mn = module_name.clone();
            let nm = name.clone();
            tokio::spawn(async move {
                match client.create_instance(data).await {
                    Ok(r) => {
                        let id = r.get("id").and_then(|v| v.as_str()).unwrap_or("?");
                        push_out(&buf, vec![Out::Ok(format!(
                            "✓ Instance '{}' created (module: {}, id: {})",
                            nm, mn, id,
                        ))]);
                    }
                    Err(e) => push_out(&buf, vec![Out::Err(format!("✗ {}", e))]),
                }
            });
            // 생성 후 서버 목록으로 복귀
            app.pop_screen(); // Step2
            app.pop_screen(); // Step1
            let msg = app.i18n.t("flash.creating_instance");
            app.flash(&msg);
        }
        InlineAction::SetCliSetting { key } => {
            match app.settings.set_value(&key, value) {
                Ok(()) => {
                    let _ = app.settings.save();
                    app.flash(&format!("{} = {}", key, value));
                }
                Err(e) => app.flash(&format!("✗ {}", e)),
            }
        }
        InlineAction::SetGuiSetting { key } => {
            let result = match key.as_str() {
                "language" => config::set_language(value),
                "ipc_port" => value.parse::<u16>().ok()
                    .map(config::set_ipc_port)
                    .unwrap_or_else(|| Err(anyhow::anyhow!("Invalid port"))),
                "refresh_interval" => value.parse::<u64>().ok()
                    .map(config::set_refresh_interval)
                    .unwrap_or_else(|| Err(anyhow::anyhow!("Invalid interval"))),
                "console_buffer" => value.parse::<u64>().ok()
                    .map(config::set_console_buffer_size)
                    .unwrap_or_else(|| Err(anyhow::anyhow!("Invalid number"))),
                _ => Err(anyhow::anyhow!("Unknown key")),
            };
            match result {
                Ok(()) => app.flash(&format!("✓ {} = {}", key, value)),
                Err(e) => app.flash(&format!("✗ {}", e)),
            }
        }
        InlineAction::ExecuteCommand { instance_id } => {
            let cmd = value.trim().to_string();
            if cmd.is_empty() { return; }
            let client = app.client.clone();
            let buf = app.async_out.clone();
            tokio::spawn(async move {
                match client.execute_command(&instance_id, &cmd, None).await {
                    Ok(r) => push_out(&buf, vec![Out::Ok(
                        r.get("message").and_then(|v| v.as_str()).unwrap_or("OK").into()
                    )]),
                    Err(e) => push_out(&buf, vec![Out::Err(format!("✗ {}", e))]),
                }
            });
            let msg = app.i18n.t("flash.running_command");
            app.flash(&msg);
        }
        InlineAction::RconCommand { instance_name } => {
            let cmd = value.trim().to_string();
            if cmd.is_empty() { return; }
            let client = app.client.clone();
            let buf = app.async_out.clone();
            let name = instance_name.clone();
            tokio::spawn(async move {
                let iid = screens::find_instance_id(&client, &name).await;
                if let Some(iid) = iid {
                    match client.execute_rcon_command(&iid, &cmd).await {
                        Ok(r) => {
                            let response = r.get("response").and_then(|v| v.as_str())
                                .or_else(|| r.get("message").and_then(|v| v.as_str()))
                                .unwrap_or("OK");
                            push_out(&buf, vec![Out::Ok(format!("RCON> {}", response))]);
                        }
                        Err(e) => push_out(&buf, vec![Out::Err(format!("✗ RCON: {}", e))]),
                    }
                } else {
                    push_out(&buf, vec![Out::Err(format!("✗ Instance '{}' not found", name))]);
                }
            });
            let msg = app.i18n.t("flash.rcon_running");
            app.flash(&msg);
        }
        InlineAction::InstallModule { module_name } => {
            let version = value.to_string();
            let client = app.client.clone();
            let buf = app.async_out.clone();
            tokio::spawn(async move {
                match client.install_server(&module_name, serde_json::json!({ "version": version })).await {
                    Ok(r) => push_out(&buf, vec![Out::Ok(format!(
                        "✓ {}", r.get("message").and_then(|v| v.as_str()).unwrap_or("Install started")
                    ))]),
                    Err(e) => push_out(&buf, vec![Out::Err(format!("✗ {}", e))]),
                }
            });
            let msg = app.i18n.t("flash.installing");
            app.flash(&msg);
        }
        InlineAction::UpdateSet => {
            // "key=value" 형식 파싱
            if let Some((key, val)) = value.split_once('=') {
                let key = key.trim();
                let val = val.trim();
                let json_value = if val == "true" { serde_json::Value::Bool(true) }
                    else if val == "false" { serde_json::Value::Bool(false) }
                    else if let Ok(n) = val.parse::<i64>() { serde_json::json!(n) }
                    else { serde_json::Value::String(val.to_string()) };
                let client = app.client.clone();
                let buf = app.async_out.clone();
                let k = key.to_string();
                let v = val.to_string();
                tokio::spawn(async move {
                    match client.set_update_config(serde_json::json!({ k.clone(): json_value })).await {
                        Ok(_) => push_out(&buf, vec![Out::Ok(format!("✓ updater.{} = {}", k, v))]),
                        Err(e) => push_out(&buf, vec![Out::Err(format!("✗ {}", e))]),
                    }
                });
                let msg = app.i18n.t("flash.config_changing");
                app.flash(&msg);
            } else {
                let msg = app.i18n.t("flash.format_keyvalue");
                app.flash(&msg);
            }
        }
        InlineAction::Custom(ref tag) => {
            if let Some(ext_id) = tag.strip_prefix("EXT_CONFIG:") {
                // key=value 형식이면 저장, 비어있으면 조회만 (이미 비동기로 출력됨)
                if !value.is_empty() {
                    if let Some((k, v)) = value.split_once('=') {
                        let ext_id = ext_id.to_string();
                        let key = k.trim().to_string();
                        let val = v.trim().to_string();
                        let client = app.client.clone();
                        let buf = app.async_out.clone();
                        tokio::spawn(async move {
                            // 기존 config를 가져와서 키 업데이트
                            let mut config = match client.get_extension_config(&ext_id).await {
                                Ok(c) => c,
                                Err(e) => {
                                    push_out(&buf, vec![Out::Err(format!("✗ {}", e))]);
                                    return;
                                }
                            };
                            // JSON 값 파싱 시도 (숫자, bool 등)
                            let json_val = if val == "true" {
                                serde_json::Value::Bool(true)
                            } else if val == "false" {
                                serde_json::Value::Bool(false)
                            } else if let Ok(n) = val.parse::<i64>() {
                                serde_json::json!(n)
                            } else {
                                serde_json::Value::String(val.clone())
                            };
                            config[&key] = json_val;
                            match client.save_extension_config(&ext_id, config).await {
                                Ok(_) => push_out(&buf, vec![Out::Ok(format!("✓ {} = {}", key, val))]),
                                Err(e) => push_out(&buf, vec![Out::Err(format!("✗ {}", e))]),
                            }
                        });
                        let msg = app.i18n.t("flash.saving_config");
                        app.flash(&msg);
                    } else {
                        let msg = app.i18n.t("flash.format_keyvalue");
                        app.flash(&msg);
                    }
                }
            }
        }
    }
}

// ═══════════════════════════════════════════════════════
// 비동기 플러시 + EDITOR_FIELD 파서
// ═══════════════════════════════════════════════════════

/// async_out에서 결과를 가져오면서 EDITOR_FIELD: 접두사가 있는 라인을
/// 파싱하여 editor_fields로 변환한다.
pub(super) fn flush_async_with_fields(app: &mut App) {
    let drained = {
        let mut buf = app.async_out.lock().unwrap();
        if buf.is_empty() { return; }
        buf.drain(..).collect::<Vec<_>>()
    };

    let mut regular_lines = Vec::new();
    let mut editor_lines = Vec::new();
    let mut ext_lines = Vec::new();
    let mut reg_lines = Vec::new();
    let mut modreg_lines = Vec::new();
    let mut ext_slots_json: Option<String> = None;

    for out in drained {
        match &out {
            Out::Text(text) | Out::Ok(text) | Out::Info(text) => {
                if let Some(stripped) = text.strip_prefix("EDITOR_LOAD_FAIL:") {
                    // 에디터 데이터 로드 실패 → 이전 화면으로 복귀 + 에러 메시지 표시
                    let msg = stripped.to_string();
                    app.pop_screen();
                    app.output.push(Out::Err(format!("✗ {}", msg)));
                    app.output.push(Out::Blank);
                    return;
                } else if let Some(stripped) = text.strip_prefix("EDITOR_FIELD:") {
                    editor_lines.push(stripped.to_string());
                } else if let Some(stripped) = text.strip_prefix("EXT_ITEM:") {
                    ext_lines.push(stripped.to_string());
                } else if let Some(stripped) = text.strip_prefix("EXT_SLOTS_INIT:") {
                    ext_slots_json = Some(stripped.to_string());
                } else if let Some(stripped) = text.strip_prefix("REG_ITEM:") {
                    reg_lines.push(stripped.to_string());
                } else if let Some(stripped) = text.strip_prefix("MODREG_ITEM:") {
                    modreg_lines.push(stripped.to_string());
                } else if text == "LOADING_DONE" {
                    app.loading = None;
                } else if let Some(stripped) = text.strip_prefix("DAEMON_LOG:") {
                    // DAEMON_LOG:<id>:<level>:<target>: <msg>
                    let parts: Vec<&str> = stripped.splitn(4, ':').collect();
                    if parts.len() >= 4 {
                        if let Ok(id) = parts[0].parse::<u64>() {
                            if id >= app.last_daemon_log_id {
                                app.last_daemon_log_id = id + 1;
                            }
                        }
                        let level = parts[1];
                        let target = parts[2];
                        let msg = parts[3].trim_start();
                        let formatted = format!("[{}] {} {}", level.to_uppercase(), target, msg);
                        match level {
                            "error" => regular_lines.push(Out::Err(formatted)),
                            "warn" => regular_lines.push(Out::Info(formatted)),
                            _ => regular_lines.push(Out::Text(formatted)),
                        }
                    }
                } else if let Some(stripped) = text.strip_prefix("VERSION_SELECT:") {
                    // VERSION_SELECT:<module_name>:<v1>|<v2>|...
                    if let Some((module_name, versions_str)) = stripped.split_once(':') {
                        let mut options: Vec<String> = vec!["latest".to_string()];
                        options.extend(versions_str.split('|').map(|s| s.to_string()));
                        app.input_mode = InputMode::InlineSelect {
                            prompt: format!("{} 설치 버전 선택", module_name),
                            options,
                            selected: 0,
                            on_submit: InlineAction::InstallModule { module_name: module_name.to_string() },
                        };
                    }
                } else {
                    regular_lines.push(out);
                }
            }
            _ => regular_lines.push(out),
        }
    }

    // EDITOR_FIELD 파싱: key|value|group|type|label|required|options
    if !editor_lines.is_empty() {
        app.editor_fields.clear();
        app.editor_changes.clear();
        app.editor_selected = 0;
        for line in editor_lines {
            let parts: Vec<&str> = line.splitn(7, '|').collect();
            if parts.len() >= 5 {
                app.editor_fields.push(EditorField {
                    key: parts[0].to_string(),
                    value: parts[1].to_string(),
                    original_value: parts[1].to_string(),
                    group: parts[2].to_string(),
                    field_type: parts[3].to_string(),
                    label: parts[4].to_string(),
                    required: parts.get(5).map(|&s| s == "true").unwrap_or(false),
                    options: parts.get(6).map(|s| s.split(',').map(|o| o.to_string()).collect()).unwrap_or_default(),
                });
            }
        }
    }

    // EXT_ITEM 파싱: id|name|version|enabled
    if !ext_lines.is_empty() {
        app.cached_extensions.clear();
        for line in ext_lines {
            let parts: Vec<&str> = line.splitn(4, '|').collect();
            if parts.len() >= 4 {
                app.cached_extensions.push(ExtensionInfo {
                    id: parts[0].to_string(),
                    name: parts[1].to_string(),
                    version: parts[2].to_string(),
                    enabled: parts[3] == "true" || parts[3] == "✓",
                });
            }
        }
    }

    // EXT_SLOTS_INIT 파싱 — 익스텐션 슬롯 레지스트리 빌드
    if let Some(json_str) = ext_slots_json {
        if let Ok(exts) = serde_json::from_str::<Vec<serde_json::Value>>(&json_str) {
            app.ext_slots.clear();
            for ext in &exts {
                let id = ext["id"].as_str().unwrap_or("").to_string();
                let name = ext["name"].as_str().unwrap_or("").to_string();
                let enabled = ext["enabled"].as_bool().unwrap_or(false);
                if !enabled { continue; }

                let cli_data = ext.get("cli");

                let inst_fields: std::collections::HashMap<String, serde_json::Value> = ext
                    .get("instance_fields")
                    .and_then(|v| v.as_object())
                    .map(|obj| obj.iter().map(|(k, v)| (k.clone(), v.clone())).collect())
                    .unwrap_or_default();

                app.ext_slots.register_extension(&id, &name, cli_data, &inst_fields);
            }
        }
    }

    // REG_ITEM 파싱: id|name|version|description
    if !reg_lines.is_empty() {
        app.cached_manifest_extensions.clear();
        for line in reg_lines {
            let parts: Vec<&str> = line.splitn(4, '|').collect();
            if parts.len() >= 4 {
                app.cached_manifest_extensions.push(ManifestItem {
                    id: parts[0].to_string(),
                    name: parts[1].to_string(),
                    version: parts[2].to_string(),
                    description: parts[3].to_string(),
                });
            }
        }
    }

    // MODREG_ITEM 파싱: id|name|version|description
    if !modreg_lines.is_empty() {
        app.cached_manifest_modules.clear();
        for line in modreg_lines {
            let parts: Vec<&str> = line.splitn(4, '|').collect();
            if parts.len() >= 4 {
                app.cached_manifest_modules.push(ManifestItem {
                    id: parts[0].to_string(),
                    name: parts[1].to_string(),
                    version: parts[2].to_string(),
                    description: parts[3].to_string(),
                });
            }
        }
    }

    // 일반 라인을 출력에 추가 (커맨드 모드에서 보임)
    if !regular_lines.is_empty() {
        // ★ 커맨드 모드가 아닌 화면에서는 에러/성공 메시지를 flash로 표시
        if !matches!(app.screen, Screen::CommandMode) {
            // 마지막 의미있는 메시지를 flash
            let mut flash_msg: Option<String> = None;
            for line in &regular_lines {
                match line {
                    Out::Err(msg) => { flash_msg = Some(msg.clone()); }
                    Out::Ok(msg) => { flash_msg = Some(msg.clone()); }
                    _ => {}
                }
            }
            if let Some(msg) = flash_msg {
                app.flash(&msg);
            }
        }
        let cmd_start = app.output.len();
        app.output.extend(regular_lines);
        app.output.push(Out::Blank);
        app.smart_scroll(cmd_start);
    }
}

/// 상태 모니터 태스크의 갱신 주기 (ms).
pub(super) fn app_refresh_interval() -> u64 {
    1000
}

/// 패닉 시 터미널 복원을 보장하는 헬퍼 — `run()` 초기화 시 한 번 호출.
pub(super) fn install_panic_hook() {
    let default_hook = std::panic::take_hook();
    std::panic::set_hook(Box::new(move |info| {
        let _ = disable_raw_mode();
        let _ = execute!(io::stdout(), LeaveAlternateScreen, DisableMouseCapture);
        default_hook(info);
    }));
}
