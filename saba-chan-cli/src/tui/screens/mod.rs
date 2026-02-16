//! 화면별 렌더링 · 키 처리 — 대시보드, 서버, 모듈, 봇, 설정, 업데이트, 데몬
//!
//! 각 화면은 세 가지를 제공합니다:
//! 1. `build_menu_*()` — 메뉴 아이템 생성
//! 2. `render_*()` — ratatui 렌더링
//! 3. `handle_*_select()` — Enter 키 처리 (화면 전환/액션)

mod bot;
mod daemon;
mod dashboard;
mod modules;
mod server_detail;
mod servers;
mod settings;
mod updates;

use ratatui::prelude::*;
use ratatui::widgets::{Block, BorderType, Borders, Paragraph};

use super::app::*;
use super::theme::Theme;
use super::render;
use crate::client::DaemonClient;

// ═══════════════════════════════════════════════════════
// 메뉴 빌더 (화면별)
// ═══════════════════════════════════════════════════════

/// 현재 화면에 맞는 메뉴 아이템 생성
pub fn build_menu(app: &App) -> Vec<MenuItem> {
    match &app.screen {
        Screen::Dashboard      => dashboard::build_dashboard_menu(app),
        Screen::Servers        => servers::build_servers_menu(app),
        Screen::ServerDetail { name, .. } => server_detail::build_server_detail_menu(app, name),
        Screen::ServerSettings { .. } | Screen::ServerProperties { .. } => vec![], // 에디터 사용
        Screen::ServerConsole { .. } => vec![], // 콘솔 사용
        Screen::Modules        => modules::build_modules_menu(app),
        Screen::ModuleDetail { name } => modules::build_module_detail_menu(name),
        Screen::Bot            => bot::build_bot_menu(app),
        Screen::BotAliases     => vec![], // 별도 처리
        Screen::Settings       => settings::build_settings_menu(app),
        Screen::Updates        => updates::build_updates_menu(),
        Screen::Daemon         => daemon::build_daemon_menu(app),
        Screen::CommandMode    => vec![], // 커맨드 모드는 메뉴 없음
    }
}

// ═══════════════════════════════════════════════════════
// 화면 렌더링
// ═══════════════════════════════════════════════════════

/// 현재 화면 렌더링 (메인 컨텐츠 영역)
pub fn render_screen(app: &App, frame: &mut Frame, area: Rect) {
    match &app.screen {
        Screen::Dashboard => dashboard::render_dashboard(app, frame, area),
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
        Screen::BotAliases => bot::render_bot_aliases(app, frame, area),
        Screen::Settings => render_detail_screen("Settings", &app.menu_items, app.menu_selected, frame, area),
        Screen::Updates => updates::render_updates_screen(app, frame, area),
        Screen::Daemon => render_detail_screen("Daemon", &app.menu_items, app.menu_selected, frame, area),
        Screen::CommandMode => render_command_mode(app, frame, area),
    }
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

fn render_command_mode(app: &App, frame: &mut Frame, area: Rect) {
    let chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints([Constraint::Min(5), Constraint::Length(3)])
        .split(area);

    render::render_output(app, frame, chunks[0]);
    render::render_command_input(app, frame, chunks[1]);
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
        Screen::Dashboard => dashboard::handle_dashboard_select(app, sel),
        Screen::Servers => servers::handle_servers_select(app, sel),
        Screen::ServerDetail { ref name, ref id, ref module_name } => {
            let name = name.clone();
            let id = id.clone();
            let module_name = module_name.clone();
            server_detail::handle_server_detail_select(app, sel, &name, &id, &module_name);
        }
        Screen::Modules => modules::handle_modules_select(app, sel),
        Screen::ModuleDetail { ref name } => {
            let name = name.clone();
            modules::handle_module_detail_select(app, sel, &name);
        }
        Screen::Bot => bot::handle_bot_select(app, sel),
        Screen::Settings => settings::handle_settings_select(app, sel),
        Screen::Updates => updates::handle_updates_select(app, sel),
        Screen::Daemon => daemon::handle_daemon_select(app, sel),
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

pub(crate) async fn find_instance_id(client: &DaemonClient, name: &str) -> Option<String> {
    if let Ok(instances) = client.list_instances().await {
        for inst in &instances {
            if inst["name"].as_str() == Some(name) {
                return inst["id"].as_str().map(|s| s.to_string());
            }
        }
    }
    None
}

pub(crate) async fn load_instance_settings(
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

pub(crate) async fn load_server_properties(
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
