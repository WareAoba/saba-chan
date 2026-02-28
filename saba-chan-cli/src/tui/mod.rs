//! saba-chan CLI — 인터랙티브 TUI 모듈
//!
//! `main.rs`에서 `tui::run(client).await`로 호출됩니다.
//! 계층적 메뉴 내비게이션 + vim-like 편집기 + 레거시 커맨드 모드를 제공합니다.

pub mod app;
pub mod commands;
pub mod render;
pub mod screens;
pub mod theme;

use std::collections::HashMap;
use std::io::{self, stdout};
use std::time::{Duration, Instant};

use crossterm::{
    event::{self, Event, KeyCode, KeyEvent, KeyEventKind, KeyModifiers,
            EnableMouseCapture, DisableMouseCapture, MouseEvent, MouseEventKind},
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
        let _ = execute!(io::stdout(), LeaveAlternateScreen, DisableMouseCapture);
        default_hook(info);
    }));

    // 터미널 초기화
    enable_raw_mode()?;
    let mut stdout = stdout();
    execute!(stdout, EnterAlternateScreen, EnableMouseCapture)?;
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
                            let ext_data = s.get("extension_data")
                                .and_then(|v| v.as_object())
                                .map(|obj| obj.iter().map(|(k, v)| (k.clone(), v.clone())).collect())
                                .unwrap_or_default();
                            servers.push(ServerInfo {
                                name: s["name"].as_str().unwrap_or("?").into(),
                                module: s["module"].as_str().unwrap_or("?").into(),
                                status: s["status"].as_str().unwrap_or("stopped").into(),
                                extension_data: ext_data,
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
                    Ok(Err(e)) => push_out(&buf, vec![Out::Err(format!("Auto-start Saba-Core failed: {}", e))]),
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

    // 4) 익스텐션 슬롯 초기 로드 — GUI의 ExtensionProvider.useEffect()에 대응
    //    데몬이 올라올 때까지 대기 후, 활성 익스텐션 목록을 가져와
    //    CLI 슬롯 레지스트리를 빌드한다.
    {
        let client = client.clone();
        let buf = app.async_out.clone();
        tokio::spawn(async move {
            // 데몬이 올라올 때까지 최대 30초 대기
            for _ in 0..30 {
                tokio::time::sleep(Duration::from_secs(1)).await;
                if client.list_servers().await.is_ok() {
                    break;
                }
            }
            // 익스텐션 목록 로드 → EXT_SLOTS_INIT 프로토콜로 전달
            if let Ok(exts) = client.list_extensions().await {
                let data = serde_json::to_string(&exts).unwrap_or_default();
                push_out(&buf, vec![Out::Text(format!("EXT_SLOTS_INIT:{}", data))]);
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

                    handle_key(&mut app, key);
                }
                Event::Mouse(mouse) => {
                    handle_mouse(&mut app, mouse);
                }
                _ => {}
            }
        }
    }

    // ── 정리 ──────────────────────────────────────────
    disable_raw_mode()?;
    execute!(terminal.backend_mut(), LeaveAlternateScreen, DisableMouseCapture)?;

    // 클라이언트 등록 해제
    let maybe_id = app.client_id.lock().unwrap().take();
    if let Some(id) = maybe_id {
        let _ = client.unregister_client(&id).await;
    }

    // 백그라운드 태스크(상태 폴링, 하트비트 등)가 남아있으면
    // tokio shutdown 시 blocking task 대기로 빈 콘솔 창이 잠시 보일 수 있으므로
    // 즉시 프로세스를 종료한다.
    std::process::exit(0);
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
                    if app.autocomplete_visible && !app.autocomplete_candidates.is_empty() {
                        // 이미 후보가 보이고 있으면 → 다음 후보 선택
                        app.autocomplete_selected =
                            (app.autocomplete_selected + 1) % app.autocomplete_candidates.len();
                    } else {
                        commands::update_autocomplete_preview(app);
                        if !app.autocomplete_visible {
                            // 미리보기 결과 없으면 기존 단일 매칭 완성 시도
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

fn handle_mouse(app: &mut App, mouse: MouseEvent) {
    match mouse.kind {
        MouseEventKind::ScrollUp => {
            match &app.screen {
                Screen::CommandMode => {
                    let max = app.output.len().saturating_sub(app.output_height);
                    app.scroll_up = (app.scroll_up + 3).min(max);
                }
                Screen::ServerConsole { .. } => {
                    let max = app.console_lines.len().saturating_sub(1);
                    if app.console_scroll < max { app.console_scroll = (app.console_scroll + 3).min(max); }
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
                    app.console_scroll = app.console_scroll.saturating_sub(3);
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

fn execute_inline_action(app: &mut App, action: InlineAction, value: &str) {
    match action {
        InlineAction::CreateInstance { module_name } => {
            let name = value.trim().to_string();
            if name.is_empty() {
                app.flash("이름을 입력해주세요");
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
            app.flash("인스턴스 생성 중...");
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
                "language" => gui_config::set_language(value),
                "ipc_port" => value.parse::<u16>().ok()
                    .map(|p| gui_config::set_ipc_port(p))
                    .unwrap_or_else(|| Err(anyhow::anyhow!("Invalid port"))),
                "refresh_interval" => value.parse::<u64>().ok()
                    .map(|ms| gui_config::set_refresh_interval(ms))
                    .unwrap_or_else(|| Err(anyhow::anyhow!("Invalid interval"))),
                "console_buffer" => value.parse::<u64>().ok()
                    .map(|n| gui_config::set_console_buffer_size(n))
                    .unwrap_or_else(|| Err(anyhow::anyhow!("Invalid number"))),
                _ => Err(anyhow::anyhow!("Unknown key")),
            };
            match result {
                Ok(()) => app.flash(&format!("✓ {} = {}", key, value)),
                Err(e) => app.flash(&format!("✗ {}", e)),
            }
        }
        InlineAction::SetBotToken => {
            if value.trim().is_empty() {
                match gui_config::clear_discord_token() {
                    Ok(()) => app.flash("✓ Discord token cleared"),
                    Err(e) => app.flash(&format!("✗ {}", e)),
                }
            } else {
                match gui_config::set_discord_token(value) {
                    Ok(()) => app.flash("✓ Discord token saved"),
                    Err(e) => app.flash(&format!("✗ {}", e)),
                }
            }
        }
        InlineAction::SetBotPrefix => {
            match gui_config::set_bot_prefix(value) {
                Ok(()) => {
                    app.bot_prefix = value.to_string();
                    app.flash(&format!("✓ Prefix: {}", value));
                }
                Err(e) => app.flash(&format!("✗ {}", e)),
            }
        }
        InlineAction::SetBotMode => {
            let mut config = gui_config::load_bot_config().unwrap_or_default();
            config["mode"] = serde_json::Value::String(value.to_string());
            let path = gui_config::get_bot_config_path_pub();
            match app::save_json_file(&path, &config) {
                Ok(()) => app.flash(&format!("✓ Bot mode: {}", value)),
                Err(e) => app.flash(&format!("✗ {}", e)),
            }
        }
        InlineAction::SetBotMusic => {
            let enabled = value == "ON" || value == "true" || value == "enabled";
            let mut config = gui_config::load_bot_config().unwrap_or_default();
            config["musicEnabled"] = serde_json::Value::Bool(enabled);
            let path = gui_config::get_bot_config_path_pub();
            match app::save_json_file(&path, &config) {
                Ok(()) => app.flash(&format!("✓ Music bot: {}", if enabled { "ON" } else { "OFF" })),
                Err(e) => app.flash(&format!("✗ {}", e)),
            }
        }
        InlineAction::SetBotRelayUrl => {
            let mut config = gui_config::load_bot_config().unwrap_or_default();
            if config.get("cloud").is_none() { config["cloud"] = serde_json::json!({}); }
            config["cloud"]["relayUrl"] = serde_json::Value::String(value.to_string());
            let path = gui_config::get_bot_config_path_pub();
            match app::save_json_file(&path, &config) {
                Ok(()) => app.flash(&format!("✓ Relay URL: {}", value)),
                Err(e) => app.flash(&format!("✗ {}", e)),
            }
        }
        InlineAction::SetBotRelayHostId => {
            let mut config = gui_config::load_bot_config().unwrap_or_default();
            if config.get("cloud").is_none() { config["cloud"] = serde_json::json!({}); }
            config["cloud"]["hostId"] = serde_json::Value::String(value.to_string());
            let path = gui_config::get_bot_config_path_pub();
            match app::save_json_file(&path, &config) {
                Ok(()) => app.flash(&format!("✓ Host ID: {}", value)),
                Err(e) => app.flash(&format!("✗ {}", e)),
            }
        }
        InlineAction::SetBotNodeToken => {
            if value.trim().is_empty() {
                match gui_config::clear_node_token() {
                    Ok(()) => app.flash("✓ Node token cleared"),
                    Err(e) => app.flash(&format!("✗ {}", e)),
                }
            } else {
                match gui_config::save_node_token(value.trim()) {
                    Ok(()) => app.flash("✓ Node token saved"),
                    Err(e) => app.flash(&format!("✗ {}", e)),
                }
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
            app.flash("명령 실행 중...");
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
            app.flash("설치 중...");
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
                app.flash("설정 변경 중...");
            } else {
                app.flash("형식: key=value");
            }
        }
        InlineAction::Custom(_) => {}
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
    let mut ext_lines = Vec::new();
    let mut reg_lines = Vec::new();
    let mut modreg_lines = Vec::new();
    let mut ext_slots_json: Option<String> = None;

    for out in drained {
        match &out {
            Out::Text(text) | Out::Ok(text) | Out::Info(text) => {
                if text.starts_with("EDITOR_LOAD_FAIL:") {
                    // 에디터 데이터 로드 실패 → 이전 화면으로 복귀 + 에러 메시지 표시
                    let msg = text["EDITOR_LOAD_FAIL:".len()..].to_string();
                    app.pop_screen();
                    app.output.push(Out::Err(format!("✗ {}", msg)));
                    app.output.push(Out::Blank);
                    return;
                } else if text.starts_with("EDITOR_FIELD:") {
                    editor_lines.push(text["EDITOR_FIELD:".len()..].to_string());
                } else if text.starts_with("EXT_ITEM:") {
                    ext_lines.push(text["EXT_ITEM:".len()..].to_string());
                } else if text.starts_with("EXT_SLOTS_INIT:") {
                    ext_slots_json = Some(text["EXT_SLOTS_INIT:".len()..].to_string());
                } else if text.starts_with("REG_ITEM:") {
                    reg_lines.push(text["REG_ITEM:".len()..].to_string());
                } else if text.starts_with("MODREG_ITEM:") {
                    modreg_lines.push(text["MODREG_ITEM:".len()..].to_string());
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
    // GUI의 ExtensionProvider가 registerSlots()를 호출하여 slots를 모으는 것에 대응
    if let Some(json_str) = ext_slots_json {
        if let Ok(exts) = serde_json::from_str::<Vec<serde_json::Value>>(&json_str) {
            app.ext_slots.clear();
            for ext in &exts {
                let id = ext["id"].as_str().unwrap_or("").to_string();
                let name = ext["name"].as_str().unwrap_or("").to_string();
                let enabled = ext["enabled"].as_bool().unwrap_or(false);
                if !enabled { continue; }

                // cli.slots → 슬롯 레지스트리 등록
                let cli_data = ext.get("cli");

                // instance_fields → FieldDefCli + 자동 파생 슬롯
                let inst_fields: HashMap<String, serde_json::Value> = ext.get("instance_fields")
                    .and_then(|v| v.as_object())
                    .map(|obj| obj.iter().map(|(k, v)| (k.clone(), v.clone())).collect())
                    .unwrap_or_default();

                app.ext_slots.register_extension(&id, &name, cli_data, &inst_fields);
            }
        }
    }

    // REG_ITEM 파싱: id|name|version|description
    if !reg_lines.is_empty() {
        app.cached_registry_extensions.clear();
        for line in reg_lines {
            let parts: Vec<&str> = line.splitn(4, '|').collect();
            if parts.len() >= 4 {
                app.cached_registry_extensions.push(RegistryItem {
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
        app.cached_registry_modules.clear();
        for line in modreg_lines {
            let parts: Vec<&str> = line.splitn(4, '|').collect();
            if parts.len() >= 4 {
                app.cached_registry_modules.push(RegistryItem {
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

/// 갱신 주기 (밀리초)
fn app_refresh_interval() -> u64 {
    1000
}
