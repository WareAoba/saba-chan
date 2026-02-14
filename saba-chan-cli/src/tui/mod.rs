//! saba-chan CLI — 인터랙티브 TUI 모듈
//!
//! `main.rs`에서 `tui::run(client).await`로 호출됩니다.
//! 계층적 메뉴 내비게이션 + vim-like 편집기 + 레거시 커맨드 모드를 제공합니다.

pub mod app;
pub mod commands;
pub mod render;
pub mod screens;
pub mod theme;

use std::io::{self, stdout};
use std::time::{Duration, Instant};

use crossterm::{
    event::{self, Event, KeyCode, KeyEvent, KeyEventKind, KeyModifiers},
    execute,
    terminal::{disable_raw_mode, enable_raw_mode, EnterAlternateScreen, LeaveAlternateScreen},
};
use ratatui::prelude::*;
use tokio::sync::mpsc;

use app::*;
use crate::client::DaemonClient;
use crate::gui_config;
use crate::process;

// ═══════════════════════════════════════════════════════
// 엔트리포인트
// ═══════════════════════════════════════════════════════

pub async fn run(client: DaemonClient) -> anyhow::Result<()> {
    // 패닉 시 터미널 복원
    let default_hook = std::panic::take_hook();
    std::panic::set_hook(Box::new(move |info| {
        let _ = disable_raw_mode();
        let _ = execute!(io::stdout(), LeaveAlternateScreen);
        default_hook(info);
    }));

    // 터미널 초기화
    enable_raw_mode()?;
    let mut stdout = stdout();
    execute!(stdout, EnterAlternateScreen)?;
    let backend = CrosstermBackend::new(stdout);
    let mut terminal = Terminal::new(backend)?;
    terminal.clear()?;

    // 앱 상태 생성
    let mut app = App::new(client.clone());
    // 초기 메뉴 빌드
    app.menu_items = screens::build_menu(&app);

    // ── 백그라운드 태스크 ──────────────────────────────
    let (status_tx, mut status_rx) = mpsc::unbounded_channel::<Snapshot>();

    // 1) 상태 모니터 (1초마다 폴링)
    {
        let client = client.clone();
        let tx = status_tx.clone();
        tokio::spawn(async move {
            loop {
                let daemon = tokio::task::spawn_blocking(process::check_daemon_running).await.unwrap_or(false);
                let bot = tokio::task::spawn_blocking(process::check_bot_running).await.unwrap_or(false);
                let token = gui_config::get_discord_token().ok().flatten().is_some();
                let prefix = gui_config::get_bot_prefix().unwrap_or_else(|_| "!saba".into());
                let mut servers = Vec::new();
                if daemon {
                    if let Ok(list) = client.list_servers().await {
                        for s in list {
                            servers.push(ServerInfo {
                                name: s["name"].as_str().unwrap_or("?").into(),
                                module: s["module"].as_str().unwrap_or("?").into(),
                                status: s["status"].as_str().unwrap_or("stopped").into(),
                            });
                        }
                    }
                }
                let _ = tx.send(Snapshot { daemon, bot, token, prefix, servers });
                tokio::time::sleep(Duration::from_millis(app_refresh_interval())).await;
            }
        });
    }

    // 2) 하트비트 등록
    {
        let client = client.clone();
        let client_id = app.client_id.clone();
        tokio::spawn(async move {
            tokio::time::sleep(Duration::from_secs(2)).await;
            match client.register_client("cli").await {
                Ok(id) => {
                    let mut lock = client_id.lock().unwrap();
                    *lock = Some(id.clone());
                    drop(lock);
                    let id_owned = id;
                    tokio::spawn(async move {
                        loop {
                            tokio::time::sleep(Duration::from_secs(30)).await;
                            let _ = client.send_heartbeat(&id_owned, None).await;
                        }
                    });
                }
                Err(_) => {}
            }
        });
    }

    // 3) 자동 시작
    {
        let buf = app.async_out.clone();
        let auto_start = app.settings.auto_start;
        tokio::spawn(async move {
            if !auto_start { return; }
            tokio::time::sleep(Duration::from_secs(1)).await;
            let daemon_running = tokio::task::spawn_blocking(process::check_daemon_running).await.unwrap_or(false);
            if !daemon_running {
                match tokio::task::spawn_blocking(process::start_daemon).await {
                    Ok(Ok(msg)) => push_out(&buf, msg.lines().map(|l| Out::Info(l.into())).collect()),
                    Ok(Err(e)) => push_out(&buf, vec![Out::Err(format!("Auto-start daemon failed: {}", e))]),
                    Err(e) => push_out(&buf, vec![Out::Err(format!("Auto-start: {}", e))]),
                }
            }
            let bot_running = tokio::task::spawn_blocking(process::check_bot_running).await.unwrap_or(false);
            let auto_bot = gui_config::get_discord_auto_start().unwrap_or(false);
            if !bot_running && auto_bot {
                tokio::time::sleep(Duration::from_secs(2)).await;
                match tokio::task::spawn_blocking(process::start_bot).await {
                    Ok(Ok(msg)) => push_out(&buf, vec![Out::Info(msg)]),
                    Ok(Err(e)) => push_out(&buf, vec![Out::Err(format!("Auto-start bot: {}", e))]),
                    Err(e) => push_out(&buf, vec![Out::Err(format!("Auto-start bot: {}", e))]),
                }
            }
        });
    }

    // ── 메인 이벤트 루프 ──────────────────────────────
    let tick = Duration::from_millis(50);
    let mut last_render = Instant::now();

    loop {
        // --- 비동기 결과 수신 ---
        while let Ok(snap) = status_rx.try_recv() {
            app.apply_status(snap);
        }
        flush_async_with_fields(&mut app);

        // --- 메뉴 항목 재생성 ---
        app.menu_items = screens::build_menu(&app);
        if app.menu_selected >= app.menu_items.len() && !app.menu_items.is_empty() {
            app.menu_selected = app.menu_items.len() - 1;
        }

        // --- 렌더링 ---
        if last_render.elapsed() >= Duration::from_millis(16) {
            // output_height를 터미널 크기에 맞게 갱신
            // 레이아웃: 상태바 4 + 힌트바 2 + 커맨드입력 3 + 테두리 2 = 약 11줄 오버헤드
            let term_height = terminal.size().map(|s| s.height as usize).unwrap_or(24);
            app.output_height = term_height.saturating_sub(11);

            terminal.draw(|f| {
                render::render(&app, f);
            })?;
            last_render = Instant::now();
        }

        if app.quit { break; }

        // --- 이벤트 폴링 ---
        if event::poll(tick)? {
            if let Event::Key(key) = event::read()? {
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

                handle_key(&mut app, key);
            }
        }
    }

    // ── 정리 ──────────────────────────────────────────
    disable_raw_mode()?;
    execute!(terminal.backend_mut(), LeaveAlternateScreen)?;

    // 클라이언트 등록 해제
    let maybe_id = app.client_id.lock().unwrap().take();
    if let Some(id) = maybe_id {
        let _ = client.unregister_client(&id).await;
    }

    Ok(())
}

// ═══════════════════════════════════════════════════════
// 키 이벤트 라우팅
// ═══════════════════════════════════════════════════════

fn handle_key(app: &mut App, key: KeyEvent) {
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
                    app.input.clear();
                    app.cursor = 0;
                    // 스택이 있으면 이전 화면으로, 없으면 무시 (기본 모드)
                    if !app.screen_stack.is_empty() {
                        app.pop_screen();
                    }
                }
                // F2 → 인터랙티브 메뉴 모드 진입
                KeyCode::F(2) => {
                    app.input.clear();
                    app.cursor = 0;
                    app.push_screen(Screen::Dashboard);
                    app.input_mode = InputMode::Normal;
                }
                KeyCode::Enter => {
                    if app.input.trim().is_empty() {
                        // 스택이 있으면 이전 화면으로, 없으면 무시
                        if !app.screen_stack.is_empty() {
                            app.pop_screen();
                        }
                    } else {
                        commands::submit(app);
                    }
                }
                KeyCode::Tab => {
                    commands::autocomplete(app);
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
        // 콘솔 모드 (stdin 입력)
        // ──────────────────
        InputMode::Console => {
            match key.code {
                KeyCode::Esc => {
                    app.console_input.clear();
                    app.input_mode = InputMode::Normal;
                    app.pop_screen();
                }
                KeyCode::Enter => {
                    let text = app.console_input.trim().to_string();
                    app.console_input.clear();
                    if !text.is_empty() {
                        // 서버 stdin으로 전송
                        if let Screen::ServerConsole { ref id, .. } = app.screen {
                            let client = app.client.clone();
                            let iid = id.clone();
                            let buf = app.async_out.clone();
                            tokio::spawn(async move {
                                match client.send_stdin(&iid, &text).await {
                                    Ok(_) => {}
                                    Err(e) => push_out(&buf, vec![Out::Err(format!("✗ stdin: {}", e))]),
                                }
                            });
                        }
                    }
                }
                KeyCode::Backspace => { app.console_input.pop(); }
                KeyCode::Char(c) => app.console_input.push(c),
                KeyCode::Up => {
                    let max = app.console_lines.len().saturating_sub(1);
                    if app.console_scroll < max { app.console_scroll += 1; }
                }
                KeyCode::Down => {
                    if app.console_scroll > 0 { app.console_scroll -= 1; }
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
    }
}

// ═══════════════════════════════════════════════════════
// 비동기 플러시 + EDITOR_FIELD 파서
// ═══════════════════════════════════════════════════════

/// async_out에서 결과를 가져오면서 EDITOR_FIELD: 접두사가 있는 라인을
/// 파싱하여 editor_fields로 변환합니다.
fn flush_async_with_fields(app: &mut App) {
    let drained = {
        let mut buf = app.async_out.lock().unwrap();
        if buf.is_empty() { return; }
        buf.drain(..).collect::<Vec<_>>()
    };

    let mut regular_lines = Vec::new();
    let mut editor_lines = Vec::new();

    for out in drained {
        match &out {
            Out::Text(text) | Out::Ok(text) | Out::Info(text) => {
                if text.starts_with("EDITOR_FIELD:") {
                    editor_lines.push(text["EDITOR_FIELD:".len()..].to_string());
                } else if text == "LOADING_DONE" {
                    app.loading = None;
                } else if text.starts_with("CONSOLE_LINE:") {
                    app.console_lines.push(text["CONSOLE_LINE:".len()..].to_string());
                    // 콘솔 라인은 최대 500줄 유지
                    if app.console_lines.len() > 500 {
                        app.console_lines.drain(..app.console_lines.len() - 500);
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

    // 일반 라인을 출력에 추가 (커맨드 모드에서 보임)
    if !regular_lines.is_empty() {
        let cmd_start = app.output.len();
        app.output.extend(regular_lines);
        app.output.push(Out::Blank);
        app.smart_scroll(cmd_start);
    }
}

/// 갱신 주기 (밀리초)
fn app_refresh_interval() -> u64 {
    1000
}
