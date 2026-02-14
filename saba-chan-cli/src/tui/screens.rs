//! 화면별 렌더링 · 키 처리 — 대시보드, 서버, 모듈, 봇, 설정, 업데이트, 데몬
//!
//! 각 화면은 세 가지를 제공합니다:
//! 1. `build_menu_*()` — 메뉴 아이템 생성
//! 2. `render_*()` — ratatui 렌더링
//! 3. `handle_*_select()` — Enter 키 처리 (화면 전환/액션)

use std::time::Duration;

use ratatui::prelude::*;
use ratatui::widgets::{Block, BorderType, Borders, Paragraph};

use super::app::*;
use super::theme::Theme;
use super::render;
use crate::client::DaemonClient;
use crate::gui_config;
use crate::process;

// ═══════════════════════════════════════════════════════
// 메뉴 빌더 (화면별)
// ═══════════════════════════════════════════════════════

/// 현재 화면에 맞는 메뉴 아이템 생성
pub fn build_menu(app: &App) -> Vec<MenuItem> {
    match &app.screen {
        Screen::Dashboard      => build_dashboard_menu(app),
        Screen::Servers        => build_servers_menu(app),
        Screen::ServerDetail { name, .. } => build_server_detail_menu(app, name),
        Screen::ServerSettings { .. } | Screen::ServerProperties { .. } => vec![], // 에디터 사용
        Screen::ServerConsole { .. } => vec![], // 콘솔 사용
        Screen::Modules        => build_modules_menu(app),
        Screen::ModuleDetail { name } => build_module_detail_menu(name),
        Screen::Bot            => build_bot_menu(app),
        Screen::BotAliases     => vec![], // 별도 처리
        Screen::Settings       => build_settings_menu(app),
        Screen::Updates        => build_updates_menu(),
        Screen::Daemon         => build_daemon_menu(app),
        Screen::CommandMode    => vec![], // 커맨드 모드는 메뉴 없음
    }
}

fn build_dashboard_menu(_app: &App) -> Vec<MenuItem> {
    vec![
        MenuItem::new("Servers", Some('1'), "서버 관리"),
        MenuItem::new("Modules", Some('2'), "게임 모듈 관리"),
        MenuItem::new("Discord Bot", Some('3'), "디스코드 봇 설정"),
        MenuItem::new("Settings", Some('4'), "CLI · GUI 설정"),
        MenuItem::new("Updates", Some('5'), "업데이트 관리"),
        MenuItem::new("Daemon", Some('6'), "데몬 프로세스 관리"),
        MenuItem::new("Command Mode", Some(':'), "레거시 명령어 입력"),
    ]
}

fn build_servers_menu(app: &App) -> Vec<MenuItem> {
    let mut items: Vec<MenuItem> = app.servers.iter().map(|s| {
        let sym = if s.status == "running" { "▶" } else { "■" };
        MenuItem::new(
            &format!("{} {}", sym, s.name),
            None,
            &format!("[{}] {}", s.module, s.status),
        )
    }).collect();

    if items.is_empty() {
        items.push(MenuItem::new("(No servers configured)", None, "").with_enabled(false));
    }

    items.push(MenuItem::new("+ New Server (instance create)", Some('n'), "새 서버 인스턴스 생성"));
    items
}

fn build_server_detail_menu(app: &App, name: &str) -> Vec<MenuItem> {
    let is_running = app.servers.iter().any(|s| s.name == name && s.status == "running");

    // 모듈 이름 찾기
    let module_name = app.servers.iter()
        .find(|s| s.name == name)
        .map(|s| s.module.as_str())
        .unwrap_or("");

    // 모듈 interaction_mode 확인 (file 모드 = server.properties 지원)
    let module_info = app.registry.get_module(module_name);
    let interaction_mode = module_info
        .and_then(|m| m.interaction_mode.as_deref())
        .unwrap_or("auto");
    let has_properties = interaction_mode == "file" || module_name.contains("minecraft");
    let has_eula = module_name.contains("minecraft");

    let mut items = vec![
        if is_running {
            MenuItem::new("■ Stop Server", Some('s'), "서버 정지")
        } else {
            MenuItem::new("▶ Start Server", Some('s'), "서버 시작")
        },
        MenuItem::new("↻ Restart", Some('r'), "서버 재시작"),
        MenuItem::new("⚡ Managed Start", Some('m'), "자동 감지 시작"),
        MenuItem::new("📟 Console", Some('c'), "서버 콘솔 (실시간)"),
        MenuItem::new("⚙ Settings", Some('e'), "인스턴스 설정 편집"),
    ];

    if has_properties {
        items.push(MenuItem::new("📋 Properties", Some('p'), "server.properties 편집"));
    }

    items.push(MenuItem::new("💻 Execute Command", Some('x'), "서버 명령어 실행"));
    items.push(MenuItem::new("🔍 Diagnose", Some('d'), "서버 진단"));
    items.push(MenuItem::new("✓ Validate", Some('v'), "설정 검증"));

    if has_eula {
        items.push(MenuItem::new("📜 Accept EULA", Some('u'), "EULA 수락"));
    }

    items.push(MenuItem::new("🗑 Delete Instance", Some('D'), "인스턴스 삭제"));
    items
}

fn build_modules_menu(app: &App) -> Vec<MenuItem> {
    let mut items: Vec<MenuItem> = app.registry.modules.iter().map(|m| {
        let mode = m.interaction_mode.as_deref().unwrap_or("-");
        MenuItem::new(
            &m.display_name,
            None,
            &format!("[{}] mode: {}", m.name, mode),
        )
    }).collect();

    if items.is_empty() {
        items.push(MenuItem::new("(No modules loaded)", None, "").with_enabled(false));
    }

    items.push(MenuItem::new("↻ Refresh Modules", Some('r'), "모듈 새로고침"));
    items
}

fn build_module_detail_menu(name: &str) -> Vec<MenuItem> {
    vec![
        MenuItem::new("Info", Some('i'), "모듈 상세 정보"),
        MenuItem::new("Versions", Some('v'), "사용 가능한 버전 목록"),
        MenuItem::new("Install", Some('I'), &format!("{} 서버 설치", name)),
    ]
}

fn build_bot_menu(app: &App) -> Vec<MenuItem> {
    let is_running = app.bot_on;
    vec![
        if is_running {
            MenuItem::new("■ Stop Bot", Some('s'), "디스코드 봇 정지")
        } else {
            MenuItem::new("▶ Start Bot", Some('s'), "디스코드 봇 시작")
        },
        MenuItem::new("🔑 Token", Some('t'), "디스코드 토큰 관리"),
        MenuItem::new("📝 Prefix", Some('p'), "봇 명령어 프리픽스 설정"),
        MenuItem::new("🏷 Aliases", Some('a'), "모듈/커맨드 별명 관리"),
        MenuItem::new("⚙ Auto-start", Some('A'), &format!(
            "자동 시작: {}",
            if gui_config::get_discord_auto_start().unwrap_or(false) { "ON" } else { "OFF" },
        )),
    ]
}

fn build_settings_menu(app: &App) -> Vec<MenuItem> {
    vec![
        MenuItem::new("Language", Some('l'), &format!(
            "표시 언어: {}",
            app.settings.effective_language(),
        )),
        MenuItem::new("Auto-start", Some('a'), &format!(
            "시작 시 데몬/봇 자동 기동: {}",
            if app.settings.auto_start { "ON" } else { "OFF" },
        )),
        MenuItem::new("Refresh Interval", Some('r'), &format!(
            "상태 갱신 주기: {}초",
            app.settings.refresh_interval,
        )),
        MenuItem::new("Bot Prefix", Some('p'), &format!(
            "프리픽스: {}",
            app.bot_prefix,
        )),
        MenuItem::new("Modules Path", Some('m'), &format!(
            "모듈 경로: {}",
            gui_config::get_modules_path().unwrap_or_default(),
        )),
        MenuItem::new("GUI Language", Some('g'), &format!(
            "GUI 언어: {}",
            gui_config::get_language().unwrap_or_else(|_| "en".into()),
        )),
    ]
}

fn build_updates_menu() -> Vec<MenuItem> {
    vec![
        MenuItem::new("Check for Updates", Some('c'), "업데이트 확인"),
        MenuItem::new("Update Status", Some('s'), "현재 업데이트 상태 조회"),
        MenuItem::new("Download Updates", Some('d'), "업데이트 다운로드"),
        MenuItem::new("Apply Updates", Some('a'), "다운로드된 업데이트 적용"),
        MenuItem::new("Updater Config", Some('C'), "업데이터 설정 조회"),
    ]
}

fn build_daemon_menu(app: &App) -> Vec<MenuItem> {
    let is_running = app.daemon_on;
    vec![
        if is_running {
            MenuItem::new("■ Stop Daemon", Some('s'), "데몬 정지")
        } else {
            MenuItem::new("▶ Start Daemon", Some('s'), "데몬 시작")
        },
        MenuItem::new("↻ Restart", Some('r'), "데몬 재시작"),
        MenuItem::new("ℹ Status", Some('i'), "데몬 상태 상세 조회"),
    ]
}

// ═══════════════════════════════════════════════════════
// 화면 렌더링
// ═══════════════════════════════════════════════════════

/// 현재 화면 렌더링 (메인 컨텐츠 영역)
pub fn render_screen(app: &App, frame: &mut Frame, area: Rect) {
    match &app.screen {
        Screen::Dashboard => render_dashboard(app, frame, area),
        Screen::Servers => render_list_screen("Servers", &app.menu_items, app.menu_selected, frame, area),
        Screen::ServerDetail { name, .. } => render_detail_screen(
            &format!("Server: {}", name),
            &app.menu_items, app.menu_selected, frame, area,
        ),
        Screen::ServerConsole { .. } => render::render_console(app, frame, area),
        Screen::ServerSettings { name, .. } => render_editor_screen(
            &format!("Settings: {}", name), app, frame, area,
        ),
        Screen::ServerProperties { name, .. } => render_editor_screen(
            &format!("Properties: {}", name), app, frame, area,
        ),
        Screen::Modules => render_list_screen("Modules", &app.menu_items, app.menu_selected, frame, area),
        Screen::ModuleDetail { name } => render_detail_screen(
            &format!("Module: {}", name),
            &app.menu_items, app.menu_selected, frame, area,
        ),
        Screen::Bot => render_detail_screen("Discord Bot", &app.menu_items, app.menu_selected, frame, area),
        Screen::BotAliases => render_bot_aliases(app, frame, area),
        Screen::Settings => render_detail_screen("Settings", &app.menu_items, app.menu_selected, frame, area),
        Screen::Updates => render_updates_screen(app, frame, area),
        Screen::Daemon => render_detail_screen("Daemon", &app.menu_items, app.menu_selected, frame, area),
        Screen::CommandMode => render_command_mode(app, frame, area),
    }
}

fn render_dashboard(app: &App, frame: &mut Frame, area: Rect) {
    let block = Block::default()
        .title(" Main Menu ")
        .title_alignment(Alignment::Left)
        .borders(Borders::ALL)
        .border_type(BorderType::Rounded)
        .border_style(Theme::border());

    let inner = block.inner(area);
    frame.render_widget(block, area);
    render::render_menu(&app.menu_items, app.menu_selected, frame, Rect::new(
        inner.x + 1, inner.y + 1,
        inner.width.saturating_sub(2), inner.height.saturating_sub(2),
    ));
}

fn render_list_screen(title: &str, items: &[MenuItem], selected: usize, frame: &mut Frame, area: Rect) {
    let block = Block::default()
        .title(format!(" {} ", title))
        .title_alignment(Alignment::Left)
        .borders(Borders::ALL)
        .border_type(BorderType::Rounded)
        .border_style(Theme::border());

    let inner = block.inner(area);
    frame.render_widget(block, area);

    render::render_menu(items, selected, frame, Rect::new(
        inner.x + 1, inner.y + 1,
        inner.width.saturating_sub(2), inner.height.saturating_sub(2),
    ));
}

fn render_detail_screen(title: &str, items: &[MenuItem], selected: usize, frame: &mut Frame, area: Rect) {
    let block = Block::default()
        .title(format!(" {} ", title))
        .title_alignment(Alignment::Left)
        .borders(Borders::ALL)
        .border_type(BorderType::Rounded)
        .border_style(Theme::border());

    let inner = block.inner(area);
    frame.render_widget(block, area);

    render::render_menu(items, selected, frame, Rect::new(
        inner.x + 1, inner.y + 1,
        inner.width.saturating_sub(2), inner.height.saturating_sub(2),
    ));
}

fn render_editor_screen(title: &str, app: &App, frame: &mut Frame, area: Rect) {
    let block = Block::default()
        .title(format!(" {} — [i] Edit  [w] Save  [Esc] Back ", title))
        .title_alignment(Alignment::Left)
        .borders(Borders::ALL)
        .border_type(BorderType::Rounded)
        .border_style(if app.input_mode == InputMode::Editing {
            Theme::border_active()
        } else {
            Theme::border()
        });

    let inner = block.inner(area);
    frame.render_widget(block, area);

    let editor_area = Rect::new(
        inner.x + 1, inner.y + 1,
        inner.width.saturating_sub(2), inner.height.saturating_sub(2),
    );
    render::render_editor(app, frame, editor_area);

    // 편집 중이면 커서 표시
    if app.input_mode == InputMode::Editing {
        render::render_edit_cursor(app, frame, editor_area);
    }

    // 변경 사항 개수 표시
    if !app.editor_changes.is_empty() {
        let changes_text = format!(" {} change(s) ", app.editor_changes.len());
        let x = area.right().saturating_sub(changes_text.len() as u16 + 2);
        frame.render_widget(
            Paragraph::new(Span::styled(changes_text, Theme::editor_changed())),
            Rect::new(x, area.y, 20, 1),
        );
    }
}

fn render_bot_aliases(_app: &App, frame: &mut Frame, area: Rect) {
    let block = Block::default()
        .title(" Bot Aliases — [Esc] Back ")
        .borders(Borders::ALL)
        .border_type(BorderType::Rounded)
        .border_style(Theme::border());

    let inner = block.inner(area);
    frame.render_widget(block, area);

    // 봇 별명 데이터를 출력
    let config = gui_config::load_bot_config().unwrap_or_default();
    let mut lines = Vec::new();

    lines.push(Line::from(Span::styled("  Module Aliases:", Theme::group_header())));
    if let Some(aliases) = config.get("moduleAliases").and_then(|v| v.as_object()) {
        if aliases.is_empty() {
            lines.push(Line::from("    (none)"));
        } else {
            for (module, alias) in aliases {
                lines.push(Line::from(format!("    {} → {}", module, alias.as_str().unwrap_or("?"))));
            }
        }
    } else {
        lines.push(Line::from("    (none)"));
    }

    lines.push(Line::from(""));
    lines.push(Line::from(Span::styled("  Command Aliases:", Theme::group_header())));
    if let Some(cmd_aliases) = config.get("commandAliases").and_then(|v| v.as_object()) {
        if cmd_aliases.is_empty() {
            lines.push(Line::from("    (none)"));
        } else {
            for (module, cmds) in cmd_aliases {
                if let Some(cmd_map) = cmds.as_object() {
                    for (cmd, alias) in cmd_map {
                        lines.push(Line::from(format!("    {}.{} → {}", module, cmd, alias.as_str().unwrap_or("?"))));
                    }
                }
            }
        }
    } else {
        lines.push(Line::from("    (none)"));
    }

    lines.push(Line::from(""));
    lines.push(Line::from(Span::styled(
        "  Use ':' command mode to edit: bot alias set module <name> <aliases>",
        Theme::dimmed(),
    )));

    frame.render_widget(
        Paragraph::new(lines),
        Rect::new(inner.x, inner.y + 1, inner.width, inner.height.saturating_sub(1)),
    );
}

fn render_command_mode(app: &App, frame: &mut Frame, area: Rect) {
    let chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints([Constraint::Min(5), Constraint::Length(3)])
        .split(area);

    render::render_output(app, frame, chunks[0]);
    render::render_command_input(app, frame, chunks[1]);
}

fn render_updates_screen(app: &App, frame: &mut Frame, area: Rect) {
    let title = if app.daemon_on {
        " Updates "
    } else {
        " Updates — ⚠ daemon offline "
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
                "  ⚠ 데몬이 실행중이지 않아 업데이트 기능을 사용할 수 없습니다.",
                Style::default().fg(Color::Yellow),
            )),
            Line::from(Span::styled(
                "    'daemon start' 명령어로 데몬을 먼저 시작해주세요.",
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

// ═══════════════════════════════════════════════════════
// 화면별 키 처리 (Normal 모드에서 화면 특정 키)
// ═══════════════════════════════════════════════════════

/// 현재 화면에 특화된 키를 처리합니다.
/// 처리했으면 true, 처리하지 않았으면 false를 반환합니다.
pub fn handle_screen_key(app: &mut App, key: &crossterm::event::KeyEvent) -> bool {
    use crossterm::event::KeyCode;

    match &app.screen {
        // 에디터 화면: ↑↓ 로 필드 내비게이션
        Screen::ServerSettings { .. } | Screen::ServerProperties { .. } => {
            if app.input_mode == InputMode::Normal {
                match key.code {
                    KeyCode::Up | KeyCode::Char('k') => {
                        app.editor_up();
                        return true;
                    }
                    KeyCode::Down | KeyCode::Char('j') => {
                        app.editor_down();
                        return true;
                    }
                    KeyCode::Char('i') | KeyCode::Enter => {
                        if !app.editor_fields.is_empty() {
                            app.enter_edit_mode();
                            return true;
                        }
                    }
                    _ => {}
                }
            }
            false
        }
        _ => false,
    }
}

// ═══════════════════════════════════════════════════════
// 메뉴 선택 처리 (Enter)
// ═══════════════════════════════════════════════════════

/// Enter 키 처리 — 화면 전환 + 비동기 액션 발동
pub fn handle_menu_select(app: &mut App) {
    let screen = app.screen.clone();
    let sel = app.menu_selected;

    match screen {
        Screen::Dashboard => handle_dashboard_select(app, sel),
        Screen::Servers => handle_servers_select(app, sel),
        Screen::ServerDetail { ref name, ref id, ref module_name } => {
            let name = name.clone();
            let id = id.clone();
            let module_name = module_name.clone();
            handle_server_detail_select(app, sel, &name, &id, &module_name);
        }
        Screen::Modules => handle_modules_select(app, sel),
        Screen::ModuleDetail { ref name } => {
            let name = name.clone();
            handle_module_detail_select(app, sel, &name);
        }
        Screen::Bot => handle_bot_select(app, sel),
        Screen::Settings => handle_settings_select(app, sel),
        Screen::Updates => handle_updates_select(app, sel),
        Screen::Daemon => handle_daemon_select(app, sel),
        _ => {}
    }
}

fn handle_dashboard_select(app: &mut App, sel: usize) {
    match sel {
        0 => { // Servers
            let buf = app.async_out.clone();
            let client = app.client.clone();
            // 서버 목록 + 인스턴스 목록을 미리 캐시
            tokio::spawn(async move {
                // 서버 목록과 인스턴스 목록은 화면 전환 후 자동 갱신
                let _ = client.list_instances().await;
                let _ = buf; // keep buf alive
            });
            app.push_screen(Screen::Servers);
        }
        1 => app.push_screen(Screen::Modules),
        2 => app.push_screen(Screen::Bot),
        3 => app.push_screen(Screen::Settings),
        4 => app.push_screen(Screen::Updates),
        5 => app.push_screen(Screen::Daemon),
        6 => {
            // Command mode
            app.push_screen(Screen::CommandMode);
            app.input_mode = InputMode::Command;
        }
        _ => {}
    }
}

fn handle_servers_select(app: &mut App, sel: usize) {
    let server_count = app.servers.len();

    if sel < server_count {
        let server = &app.servers[sel];
        let server_name = server.name.clone();
        let module_name = server.module.clone();

        // 인스턴스 ID 조회를 비동기로 실행
        let client = app.client.clone();
        let buf = app.async_out.clone();
        let name_for_lookup = server_name.clone();

        // 인스턴스 대비 ID를 캐시 조회 → 화면 전환
        // 일단 빈 ID로 전환하고 비동기로 ID를 채움
        app.push_screen(Screen::ServerDetail {
            name: server_name.clone(),
            id: String::new(),
            module_name: module_name.clone(),
        });

        // 비동기로 인스턴스 ID 조회
        tokio::spawn(async move {
            if let Ok(instances) = client.list_instances().await {
                for inst in &instances {
                    if inst["name"].as_str() == Some(&name_for_lookup) {
                        // ID를 찾았으면 push_out으로 상태 메시지를 보냄 (화면 갱신 시 반영)
                        let id = inst["id"].as_str().unwrap_or("").to_string();
                        push_out(&buf, vec![Out::Info(format!("Instance ID: {}", id))]);
                        return;
                    }
                }
            }
        });
    } else if sel == server_count {
        // New Server → 커맨드 모드로 전환 (instance create)
        app.push_screen(Screen::CommandMode);
        app.input_mode = InputMode::Command;
        app.input = "instance create ".to_string();
        app.cursor = app.input.chars().count();
    }
}

fn handle_server_detail_select(
    app: &mut App, sel: usize, name: &str, id: &str, module_name: &str,
) {
    // 동적 메뉴이므로 인덱스 대신 단축키로 판별
    let shortcut = app.menu_items.get(sel).and_then(|item| item.shortcut);

    let client = app.client.clone();
    let buf = app.async_out.clone();
    let name = name.to_string();
    let id = id.to_string();
    let module_name = module_name.to_string();

    match shortcut {
        Some('s') => { // Start/Stop
            let is_running = app.servers.iter().any(|s| s.name == name && s.status == "running");
            if is_running {
                tokio::spawn(async move {
                    match client.stop_server(&name, false).await {
                        Ok(r) => push_out(&buf, vec![Out::Ok(format!(
                            "✓ {}", r.get("message").and_then(|v| v.as_str()).unwrap_or("Stopped")
                        ))]),
                        Err(e) => push_out(&buf, vec![Out::Err(format!("✗ {}", e))]),
                    }
                });
            } else {
                tokio::spawn(async move {
                    match client.start_server(&name, &module_name).await {
                        Ok(r) => push_out(&buf, vec![Out::Ok(format!(
                            "✓ {}", r.get("message").and_then(|v| v.as_str()).unwrap_or("Started")
                        ))]),
                        Err(e) => push_out(&buf, vec![Out::Err(format!("✗ {}", e))]),
                    }
                });
            }
            app.flash("명령 실행 중...");
        }
        Some('r') => { // Restart
            tokio::spawn(async move {
                if let Err(e) = client.stop_server(&name, false).await {
                    push_out(&buf, vec![Out::Err(format!("✗ Stop: {}", e))]);
                    return;
                }
                tokio::time::sleep(Duration::from_secs(1)).await;
                match client.start_server(&name, &module_name).await {
                    Ok(_) => push_out(&buf, vec![Out::Ok("✓ Server restarted".into())]),
                    Err(e) => push_out(&buf, vec![Out::Err(format!("✗ Start: {}", e))]),
                }
            });
            app.flash("재시작 중...");
        }
        Some('m') => { // Managed Start
            tokio::spawn(async move {
                let instance_id = find_instance_id(&client, &name).await;
                if let Some(iid) = instance_id {
                    match client.start_managed(&iid).await {
                        Ok(r) => push_out(&buf, vec![Out::Ok(format!(
                            "✓ {}", r.get("message").and_then(|v| v.as_str()).unwrap_or("Managed started")
                        ))]),
                        Err(e) => push_out(&buf, vec![Out::Err(format!("✗ {}", e))]),
                    }
                } else {
                    push_out(&buf, vec![Out::Err(format!("✗ Instance '{}' not found", name))]);
                }
            });
            app.flash("Managed start...");
        }
        Some('c') => { // Console
            let console_name = name.clone();
            let console_id = id.clone();
            app.console_lines.clear();
            app.console_input.clear();
            app.console_scroll = 0;
            app.push_screen(Screen::ServerConsole {
                name: console_name.clone(),
                id: console_id.clone(),
            });
            app.input_mode = InputMode::Console;

            // 콘솔 데이터 비동기 로드
            let buf2 = app.async_out.clone();
            let client2 = app.client.clone();
            tokio::spawn(async move {
                let iid = find_instance_id(&client2, &console_name).await;
                if let Some(iid) = iid {
                    match client2.get_console(&iid).await {
                        Ok(data) => {
                            let mut lines_out = vec![];
                            if let Some(lines) = data.get("lines").and_then(|v| v.as_array()) {
                                for line in lines.iter().rev().take(200).collect::<Vec<_>>().into_iter().rev() {
                                    lines_out.push(Out::Text(line.as_str().unwrap_or("").into()));
                                }
                            } else if let Some(output) = data.get("output").and_then(|v| v.as_str()) {
                                for line in output.lines().rev().take(200).collect::<Vec<_>>().into_iter().rev() {
                                    lines_out.push(Out::Text(line.into()));
                                }
                            }
                            push_out(&buf2, lines_out);
                        }
                        Err(e) => push_out(&buf2, vec![Out::Err(format!("✗ Console: {}", e))]),
                    }
                }
            });
        }
        Some('e') => { // Settings
            app.editor_fields.clear();
            app.editor_selected = 0;
            app.editor_changes.clear();
            app.push_screen(Screen::ServerSettings {
                name: name.clone(),
                id: id.clone(),
                module_name: module_name.clone(),
            });

            // 비동기로 설정 스키마 + 현재 값 로드
            let buf2 = app.async_out.clone();
            let client2 = app.client.clone();
            let inst_name = name.clone();
            let mod_name = module_name.clone();
            tokio::spawn(async move {
                load_instance_settings(&client2, &inst_name, &mod_name, &buf2).await;
            });
        }
        Some('p') => { // Properties
            app.editor_fields.clear();
            app.editor_selected = 0;
            app.editor_changes.clear();
            app.push_screen(Screen::ServerProperties {
                name: name.clone(),
                id: id.clone(),
            });

            let buf2 = app.async_out.clone();
            let client2 = app.client.clone();
            let inst_name = name.clone();
            tokio::spawn(async move {
                load_server_properties(&client2, &inst_name, &buf2).await;
            });
        }
        Some('x') => { // Execute Command
            app.push_screen(Screen::CommandMode);
            app.input_mode = InputMode::Command;
            let iid = if id.is_empty() { name.to_string() } else { id.to_string() };
            app.input = format!("exec {} cmd ", iid);
            app.cursor = app.input.chars().count();
        }
        Some('d') => { // Diagnose
            tokio::spawn(async move {
                let iid = find_instance_id(&client, &name).await;
                if let Some(iid) = iid {
                    match client.diagnose(&iid).await {
                        Ok(data) => {
                            let mut lines = vec![Out::Ok(format!("Diagnosis for '{}':", name))];
                            if let Some(obj) = data.as_object() {
                                for (k, v) in obj {
                                    let val = match v {
                                        serde_json::Value::String(s) => s.clone(),
                                        _ => v.to_string(),
                                    };
                                    lines.push(Out::Text(format!("  {}: {}", k, val)));
                                }
                            }
                            push_out(&buf, lines);
                        }
                        Err(e) => push_out(&buf, vec![Out::Err(format!("✗ {}", e))]),
                    }
                }
            });
            app.flash("진단 중...");
        }
        Some('v') => { // Validate
            tokio::spawn(async move {
                let iid = find_instance_id(&client, &name).await;
                if let Some(iid) = iid {
                    match client.validate_instance(&iid).await {
                        Ok(r) => push_out(&buf, vec![Out::Ok(format!(
                            "✓ {}", r.get("message").and_then(|v| v.as_str()).unwrap_or("Validation passed")
                        ))]),
                        Err(e) => push_out(&buf, vec![Out::Err(format!("✗ {}", e))]),
                    }
                }
            });
            app.flash("검증 중...");
        }
        Some('u') => { // EULA
            app.input_mode = InputMode::Confirm {
                prompt: format!("Accept EULA for '{}'?", name),
                action: ConfirmAction::AcceptEula(id.to_string()),
            };
        }
        Some('D') => { // Delete
            app.input_mode = InputMode::Confirm {
                prompt: format!("Delete instance '{}'?", name),
                action: ConfirmAction::DeleteInstance(id.to_string()),
            };
        }
        _ => {}
    }
}

fn handle_modules_select(app: &mut App, sel: usize) {
    let module_count = app.registry.modules.len();

    if sel < module_count {
        let module = &app.registry.modules[sel];
        let name = module.name.clone();

        // 모듈 상세 데이터 로드
        let client = app.client.clone();
        let buf = app.async_out.clone();
        let mod_name = name.clone();
        tokio::spawn(async move {
            match client.get_module(&mod_name).await {
                Ok(data) => {
                    let mut lines = vec![Out::Ok(format!("Module: {}", mod_name))];
                    for key in &["name", "version", "description", "game_name", "display_name", "interaction_mode"] {
                        if let Some(val) = data.get(*key).and_then(|v| v.as_str()) {
                            lines.push(Out::Text(format!("  {:<20} {}", key, val)));
                        }
                    }
                    push_out(&buf, lines);
                }
                Err(e) => push_out(&buf, vec![Out::Err(format!("✗ {}", e))]),
            }
        });

        app.push_screen(Screen::ModuleDetail { name });
    } else if sel == module_count {
        // Refresh
        let client = app.client.clone();
        let buf = app.async_out.clone();
        tokio::spawn(async move {
            match client.refresh_modules().await {
                Ok(_) => push_out(&buf, vec![Out::Ok("✓ Modules refreshed".into())]),
                Err(e) => push_out(&buf, vec![Out::Err(format!("✗ {}", e))]),
            }
        });
        app.flash("새로고침 중...");
    }
}

fn handle_module_detail_select(app: &mut App, sel: usize, name: &str) {
    let client = app.client.clone();
    let buf = app.async_out.clone();
    let name = name.to_string();

    match sel {
        0 => { // Info
            tokio::spawn(async move {
                match client.get_module(&name).await {
                    Ok(data) => {
                        let mut lines = vec![Out::Ok(format!("Module: {}", name))];
                        for key in &["name", "version", "description", "game_name", "display_name", "interaction_mode"] {
                            if let Some(val) = data.get(*key).and_then(|v| v.as_str()) {
                                lines.push(Out::Text(format!("  {:<20} {}", key, val)));
                            }
                        }
                        if let Some(settings) = data.get("settings").and_then(|v| v.get("fields")).and_then(|v| v.as_array()) {
                            lines.push(Out::Blank);
                            lines.push(Out::Info(format!("Settings ({} fields):", settings.len())));
                            for field in settings {
                                let fname = field["name"].as_str().unwrap_or("?");
                                let ftype = field["type"].as_str().unwrap_or("?");
                                let flabel = field["label"].as_str().unwrap_or("");
                                let req = if field["required"].as_bool().unwrap_or(false) { "*" } else { " " };
                                lines.push(Out::Text(format!("  {}{:<24} {:>8} {}", req, fname, ftype, flabel)));
                            }
                        }
                        if let Some(cmds) = data.get("commands").and_then(|v| v.get("fields")).and_then(|v| v.as_array()) {
                            lines.push(Out::Blank);
                            lines.push(Out::Info(format!("Commands ({}):", cmds.len())));
                            for cmd in cmds {
                                let cname = cmd["name"].as_str().unwrap_or("?");
                                let cdesc = cmd["description"].as_str().unwrap_or("");
                                let method = cmd["method"].as_str().unwrap_or("-");
                                lines.push(Out::Text(format!("  {:<16} [{}] {}", cname, method, cdesc)));
                            }
                        }
                        push_out(&buf, lines);
                    }
                    Err(e) => push_out(&buf, vec![Out::Err(format!("✗ {}", e))]),
                }
            });
        }
        1 => { // Versions
            tokio::spawn(async move {
                match client.list_versions(&name).await {
                    Ok(data) => {
                        if let Some(versions) = data.get("versions").and_then(|v| v.as_array()) {
                            let mut lines = vec![Out::Ok(format!("{} version(s) for '{}':", versions.len(), name))];
                            for v in versions {
                                let id = v.as_str().or_else(|| v["id"].as_str()).unwrap_or("?");
                                lines.push(Out::Text(format!("  • {}", id)));
                            }
                            push_out(&buf, lines);
                        }
                    }
                    Err(e) => push_out(&buf, vec![Out::Err(format!("✗ {}", e))]),
                }
            });
        }
        2 => { // Install → 커맨드 모드
            app.push_screen(Screen::CommandMode);
            app.input_mode = InputMode::Command;
            app.input = format!("module install {} ", name);
            app.cursor = app.input.chars().count();
        }
        _ => {}
    }
}

fn handle_bot_select(app: &mut App, sel: usize) {
    let _client = app.client.clone();
    let buf = app.async_out.clone();

    match sel {
        0 => { // Start/Stop
            if app.bot_on {
                tokio::spawn(async move {
                    match tokio::task::spawn_blocking(process::stop_bot).await {
                        Ok(Ok(msg)) => push_out(&buf, vec![Out::Ok(msg)]),
                        Ok(Err(e)) => push_out(&buf, vec![Out::Err(format!("✗ {}", e))]),
                        Err(e) => push_out(&buf, vec![Out::Err(format!("✗ {}", e))]),
                    }
                });
            } else {
                tokio::spawn(async move {
                    match tokio::task::spawn_blocking(process::start_bot).await {
                        Ok(Ok(msg)) => push_out(&buf, vec![Out::Ok(msg)]),
                        Ok(Err(e)) => push_out(&buf, vec![Out::Err(format!("✗ {}", e))]),
                        Err(e) => push_out(&buf, vec![Out::Err(format!("✗ {}", e))]),
                    }
                });
            }
            app.flash(if app.bot_on { "봇 정지 중..." } else { "봇 시작 중..." });
        }
        1 => { // Token → 커맨드 모드
            app.push_screen(Screen::CommandMode);
            app.input_mode = InputMode::Command;
            app.input = "bot token ".to_string();
            app.cursor = app.input.chars().count();
        }
        2 => { // Prefix → 커맨드 모드
            app.push_screen(Screen::CommandMode);
            app.input_mode = InputMode::Command;
            app.input = "bot prefix ".to_string();
            app.cursor = app.input.chars().count();
        }
        3 => { // Aliases
            app.push_screen(Screen::BotAliases);
        }
        4 => { // Auto-start toggle
            let current = gui_config::get_discord_auto_start().unwrap_or(false);
            let _ = gui_config::set_discord_auto_start(!current);
            app.flash(&format!("Auto-start: {}", if !current { "ON" } else { "OFF" }));
        }
        _ => {}
    }
}

fn handle_settings_select(app: &mut App, sel: usize) {
    // 설정은 대부분 커맨드 모드에서 편집하도록 유도
    match sel {
        0 => { // Language
            app.push_screen(Screen::CommandMode);
            app.input_mode = InputMode::Command;
            app.input = "config set language ".to_string();
            app.cursor = app.input.chars().count();
        }
        1 => { // Auto-start toggle
            app.settings.auto_start = !app.settings.auto_start;
            let _ = app.settings.save();
            app.flash(&format!("Auto-start: {}", if app.settings.auto_start { "ON" } else { "OFF" }));
        }
        2 => { // Refresh interval
            app.push_screen(Screen::CommandMode);
            app.input_mode = InputMode::Command;
            app.input = "config set refresh_interval ".to_string();
            app.cursor = app.input.chars().count();
        }
        3 => { // Bot prefix
            app.push_screen(Screen::CommandMode);
            app.input_mode = InputMode::Command;
            app.input = "bot prefix set ".to_string();
            app.cursor = app.input.chars().count();
        }
        4 => { // Modules path
            app.push_screen(Screen::CommandMode);
            app.input_mode = InputMode::Command;
            app.input = "config gui modules_path ".to_string();
            app.cursor = app.input.chars().count();
        }
        5 => { // GUI language
            app.push_screen(Screen::CommandMode);
            app.input_mode = InputMode::Command;
            app.input = "config gui language ".to_string();
            app.cursor = app.input.chars().count();
        }
        _ => {}
    }
}

fn handle_updates_select(app: &mut App, sel: usize) {
    if !app.daemon_on {
        app.flash("⚠ 데몬이 오프라인입니다. 'daemon start'를 먼저 실행하세요.");
        return;
    }

    let client = app.client.clone();
    let buf = app.async_out.clone();

    match sel {
        0 => { // Check
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
        1 => { // Status
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
        2 => { // Download
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
        3 => { // Apply
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
        4 => { // Config
            tokio::spawn(async move {
                match client.get_update_config().await {
                    Ok(v) => {
                        let mut lines = vec![Out::Ok("Updater Config:".into())];
                        if let Some(map) = v.as_object() {
                            for (k, val) in map {
                                lines.push(Out::Text(format!("  {}: {}", k, val)));
                            }
                        }
                        push_out(&buf, lines);
                    }
                    Err(e) => push_out(&buf, vec![Out::Err(format!("✗ {}", e))]),
                }
            });
        }
        _ => {}
    }
}

fn handle_daemon_select(app: &mut App, sel: usize) {
    let buf = app.async_out.clone();
    let _client = app.client.clone();

    match sel {
        0 => { // Start/Stop
            if app.daemon_on {
                tokio::spawn(async move {
                    match tokio::task::spawn_blocking(process::stop_daemon).await {
                        Ok(Ok(msg)) => push_out(&buf, vec![Out::Ok(msg)]),
                        Ok(Err(e)) => push_out(&buf, vec![Out::Err(format!("✗ {}", e))]),
                        Err(e) => push_out(&buf, vec![Out::Err(format!("✗ {}", e))]),
                    }
                });
            } else {
                tokio::spawn(async move {
                    match tokio::task::spawn_blocking(process::start_daemon).await {
                        Ok(Ok(msg)) => {
                            let lines: Vec<Out> = msg.lines().map(|l| Out::Ok(l.into())).collect();
                            push_out(&buf, lines);
                        }
                        Ok(Err(e)) => push_out(&buf, vec![Out::Err(format!("✗ {}", e))]),
                        Err(e) => push_out(&buf, vec![Out::Err(format!("✗ {}", e))]),
                    }
                });
            }
            app.flash(if app.daemon_on { "데몬 정지 중..." } else { "데몬 시작 중..." });
        }
        1 => { // Restart
            tokio::spawn(async move {
                let stop_result = tokio::task::spawn_blocking(process::stop_daemon).await;
                match stop_result {
                    Ok(Ok(msg)) => {
                        let mut lines = vec![Out::Ok(msg)];
                        tokio::time::sleep(Duration::from_secs(1)).await;
                        match tokio::task::spawn_blocking(process::start_daemon).await {
                            Ok(Ok(msg2)) => {
                                for l in msg2.lines() { lines.push(Out::Ok(l.into())); }
                            }
                            Ok(Err(e)) => lines.push(Out::Err(format!("✗ Start: {}", e))),
                            Err(e) => lines.push(Out::Err(format!("✗ Start: {}", e))),
                        }
                        push_out(&buf, lines);
                    }
                    Ok(Err(e)) => push_out(&buf, vec![Out::Err(format!("✗ Stop: {}", e))]),
                    Err(e) => push_out(&buf, vec![Out::Err(format!("✗ Stop: {}", e))]),
                }
            });
            app.flash("데몬 재시작 중...");
        }
        2 => { // Status
            tokio::spawn(async move {
                let running = tokio::task::spawn_blocking(process::check_daemon_running)
                    .await.unwrap_or(false);
                if running {
                    let http = reqwest::Client::builder().timeout(Duration::from_secs(2)).build().unwrap();
                    let mut lines = vec![Out::Ok("Daemon: ● RUNNING".into())];
                    lines.push(Out::Text("  Host:     127.0.0.1".into()));
                    lines.push(Out::Text("  Port:     57474".into()));
                    lines.push(Out::Text("  Protocol: HTTP REST".into()));
                    if let Ok(resp) = http.get("http://127.0.0.1:57474/api/modules").send().await {
                        if let Ok(data) = resp.json::<serde_json::Value>().await {
                            let mods = data.get("modules").and_then(|v| v.as_array()).map(|a| a.len()).unwrap_or(0);
                            lines.push(Out::Text(format!("  Modules:  {}", mods)));
                        }
                    }
                    if let Ok(resp) = http.get("http://127.0.0.1:57474/api/servers").send().await {
                        if let Ok(data) = resp.json::<serde_json::Value>().await {
                            let srvs = data.get("servers").and_then(|v| v.as_array()).map(|a| a.len()).unwrap_or(0);
                            let running_count = data.get("servers").and_then(|v| v.as_array())
                                .map(|a| a.iter().filter(|s| s["status"].as_str() == Some("running")).count())
                                .unwrap_or(0);
                            lines.push(Out::Text(format!("  Servers:  {}/{} running", running_count, srvs)));
                        }
                    }
                    push_out(&buf, lines);
                } else {
                    push_out(&buf, vec![Out::Text("Daemon: ○ OFFLINE".into())]);
                }
            });
        }
        _ => {}
    }
}

// ═══════════════════════════════════════════════════════
// 확인 액션 실행
// ═══════════════════════════════════════════════════════

pub fn execute_confirm(app: &mut App, action: ConfirmAction) {
    let client = app.client.clone();
    let buf = app.async_out.clone();

    match action {
        ConfirmAction::DeleteInstance(id) => {
            tokio::spawn(async move {
                match client.delete_instance(&id).await {
                    Ok(_) => push_out(&buf, vec![Out::Ok(format!("✓ Instance deleted"))]),
                    Err(e) => push_out(&buf, vec![Out::Err(format!("✗ {}", e))]),
                }
            });
            app.pop_screen(); // 서버 목록으로 복귀
            app.flash("삭제 완료");
        }
        ConfirmAction::StopServer(name) => {
            tokio::spawn(async move {
                match client.stop_server(&name, true).await {
                    Ok(r) => push_out(&buf, vec![Out::Ok(format!(
                        "✓ {}", r.get("message").and_then(|v| v.as_str()).unwrap_or("Stopped")
                    ))]),
                    Err(e) => push_out(&buf, vec![Out::Err(format!("✗ {}", e))]),
                }
            });
        }
        ConfirmAction::AcceptEula(id) => {
            tokio::spawn(async move {
                match client.accept_eula(&id).await {
                    Ok(_) => push_out(&buf, vec![Out::Ok("✓ EULA accepted".into())]),
                    Err(e) => push_out(&buf, vec![Out::Err(format!("✗ {}", e))]),
                }
            });
            app.flash("EULA 수락됨");
        }
    }
    app.input_mode = InputMode::Normal;
}

// ═══════════════════════════════════════════════════════
// 에디터 저장 (w 키)
// ═══════════════════════════════════════════════════════

pub fn save_editor_changes(app: &mut App) {
    if app.editor_changes.is_empty() {
        app.flash("No changes to save");
        return;
    }

    let client = app.client.clone();
    let buf = app.async_out.clone();
    let changes = app.editor_changes.clone();
    let screen = app.screen.clone();

    match screen {
        Screen::ServerSettings { name, .. } => {
            let inst_name = name.clone();
            tokio::spawn(async move {
                let iid = find_instance_id(&client, &inst_name).await;
                if let Some(iid) = iid {
                    // 변경사항을 적절한 JSON 값으로 변환
                    let mut settings = serde_json::Map::new();
                    for (key, val) in &changes {
                        let json_val = if val == "true" {
                            serde_json::Value::Bool(true)
                        } else if val == "false" {
                            serde_json::Value::Bool(false)
                        } else if let Ok(n) = val.parse::<i64>() {
                            serde_json::json!(n)
                        } else if let Ok(f) = val.parse::<f64>() {
                            serde_json::json!(f)
                        } else {
                            serde_json::Value::String(val.clone())
                        };
                        settings.insert(key.clone(), json_val);
                    }
                    match client.update_instance(&iid, serde_json::Value::Object(settings)).await {
                        Ok(_) => push_out(&buf, vec![Out::Ok(format!(
                            "✓ {} setting(s) saved for '{}'", changes.len(), inst_name
                        ))]),
                        Err(e) => push_out(&buf, vec![Out::Err(format!("✗ {}", e))]),
                    }
                }
            });
        }
        Screen::ServerProperties { name, .. } => {
            let inst_name = name.clone();
            tokio::spawn(async move {
                let iid = find_instance_id(&client, &inst_name).await;
                if let Some(iid) = iid {
                    // 현재 properties 전체 로드 → 변경분 머지 → 쓰기
                    match client.read_properties(&iid).await {
                        Ok(data) => {
                            let mut props = if let Some(obj) = data.get("properties") {
                                obj.clone()
                            } else {
                                data.clone()
                            };
                            for (key, val) in &changes {
                                props[key.as_str()] = serde_json::Value::String(val.clone());
                            }
                            let write_data = serde_json::json!({ "properties": props });
                            match client.write_properties(&iid, write_data).await {
                                Ok(_) => push_out(&buf, vec![Out::Ok(format!(
                                    "✓ {} property(ies) saved for '{}'", changes.len(), inst_name
                                ))]),
                                Err(e) => push_out(&buf, vec![Out::Err(format!("✗ Write: {}", e))]),
                            }
                        }
                        Err(e) => push_out(&buf, vec![Out::Err(format!("✗ Read: {}", e))]),
                    }
                }
            });
        }
        _ => {}
    }

    app.editor_changes.clear();
    // 원본 값도 현재 값으로 갱신
    for field in &mut app.editor_fields {
        field.original_value = field.value.clone();
    }
    app.flash("저장 완료!");
}

// ═══════════════════════════════════════════════════════
// 비동기 데이터 로더
// ═══════════════════════════════════════════════════════

async fn find_instance_id(client: &DaemonClient, name: &str) -> Option<String> {
    if let Ok(instances) = client.list_instances().await {
        for inst in &instances {
            if inst["name"].as_str() == Some(name) {
                return inst["id"].as_str().map(|s| s.to_string());
            }
        }
    }
    None
}

async fn load_instance_settings(
    client: &DaemonClient, name: &str, module_name: &str, buf: &OutputBuf,
) {
    let instance_id = match find_instance_id(client, name).await {
        Some(id) => id,
        None => {
            push_out(buf, vec![Out::Err(format!("✗ Instance '{}' not found", name))]);
            return;
        }
    };

    // 인스턴스 현재 값 로드
    let inst_data = match client.get_instance(&instance_id).await {
        Ok(d) => d,
        Err(e) => {
            push_out(buf, vec![Out::Err(format!("✗ {}", e))]);
            return;
        }
    };

    // 모듈 메타데이터 (스키마) 로드
    let module_data = client.get_module(module_name).await.ok();

    // EditorField 목록을 Out::Text로 인코딩하여 전달 (비동기→동기 경계)
    // 형식: "EDITOR_FIELD:{key}|{value}|{group}|{type}|{label}|{required}|{options}"
    let mut lines = vec![];

    if let Some(mdata) = module_data {
        if let Some(fields) = mdata.get("settings").and_then(|v| v.get("fields")).and_then(|v| v.as_array()) {
            for field in fields {
                let fname = field["name"].as_str().unwrap_or("?");
                let ftype = field["type"].as_str().unwrap_or("text");
                let flabel = field["label"].as_str().unwrap_or("");
                let fgroup = field["group"].as_str().unwrap_or("basic");
                let freq = field["required"].as_bool().unwrap_or(false);

                let current_val = inst_data.get(fname).map(|v| match v {
                    serde_json::Value::String(s) => s.clone(),
                    serde_json::Value::Null => String::new(),
                    _ => v.to_string(),
                }).unwrap_or_default();

                let options = field.get("options")
                    .and_then(|v| v.as_array())
                    .map(|arr| arr.iter().filter_map(|v| v.as_str()).collect::<Vec<_>>().join(","))
                    .unwrap_or_default();

                lines.push(Out::Text(format!(
                    "EDITOR_FIELD:{}|{}|{}|{}|{}|{}|{}",
                    fname, current_val, fgroup, ftype, flabel, freq, options,
                )));
            }
        }
    } else {
        // 모듈 메타데이터 없음 — 인스턴스의 모든 필드를 표시
        if let Some(obj) = inst_data.as_object() {
            for (key, val) in obj {
                if key == "id" || key == "name" || key == "module_name" { continue; }
                let val_str = match val {
                    serde_json::Value::String(s) => s.clone(),
                    serde_json::Value::Null => String::new(),
                    _ => val.to_string(),
                };
                lines.push(Out::Text(format!(
                    "EDITOR_FIELD:{}|{}|basic|text|||",
                    key, val_str,
                )));
            }
        }
    }

    push_out(buf, lines);
}

async fn load_server_properties(
    client: &DaemonClient, name: &str, buf: &OutputBuf,
) {
    let instance_id = match find_instance_id(client, name).await {
        Some(id) => id,
        None => {
            push_out(buf, vec![Out::Err(format!("✗ Instance '{}' not found", name))]);
            return;
        }
    };

    match client.read_properties(&instance_id).await {
        Ok(data) => {
            let props = if let Some(obj) = data.get("properties").and_then(|v| v.as_object()) {
                obj.clone()
            } else if let Some(obj) = data.as_object() {
                obj.clone()
            } else {
                push_out(buf, vec![Out::Err("✗ Unexpected response format".into())]);
                return;
            };

            let mut lines = vec![];
            for (key, val) in &props {
                let val_str = match val {
                    serde_json::Value::String(s) => s.clone(),
                    _ => val.to_string(),
                };
                lines.push(Out::Text(format!(
                    "EDITOR_FIELD:{}|{}|properties|text|||",
                    key, val_str,
                )));
            }
            push_out(buf, lines);
        }
        Err(e) => push_out(buf, vec![Out::Err(format!("✗ {}", e))]),
    }
}
