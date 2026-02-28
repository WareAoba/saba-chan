//! 화면별 렌더링 · 키 처리 — 대시보드, 서버, 모듈, 봇, 설정, 업데이트, 데몬
//!
//! 각 화면은 세 가지를 제공합니다:
//! 1. `build_menu_*()` — 메뉴 아이템 생성
//! 2. `render_*()` — ratatui 렌더링
//! 3. `handle_*_select()` — Enter 키 처리 (화면 전환/액션)

mod bot;
mod create_instance;
mod daemon;
mod dashboard;
mod extensions;
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
        Screen::ModuleRegistry => extensions::build_module_registry_menu(app),
        Screen::Bot            => bot::build_bot_menu(app),
        Screen::BotAliases     => vec![], // 별도 처리
        Screen::Settings       => settings::build_settings_menu(app),
        Screen::Updates        => updates::build_updates_menu(app),
        Screen::Daemon         => daemon::build_daemon_menu(app),
        Screen::Extensions     => extensions::build_extensions_menu(app),
        Screen::ExtensionList  => extensions::build_extension_list_menu(app),
        Screen::ExtensionDetail { ext_id, .. } => extensions::build_extension_detail_menu(app, ext_id),
        Screen::ExtensionRegistry => extensions::build_extension_registry_menu(app),
        Screen::CreateInstanceStep1 => create_instance::build_create_step1_menu(app),
        Screen::CreateInstanceStep2 { .. } => vec![],
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
        Screen::Servers => render_list_screen("Instances", &app.menu_items, app.menu_selected, frame, area),
        Screen::ServerDetail { name, .. } => render_detail_screen(
            &format!("Instance: {}", name),
            &app.menu_items, app.menu_selected, frame, area,
        ),
        Screen::ServerConsole { .. } => render::render_console(app, frame, area),
        Screen::ServerSettings { name, .. } => render_editor_screen(
            &format!("Settings: {}", name), app, frame, area,
        ),
        Screen::Modules => render_list_screen("Modules", &app.menu_items, app.menu_selected, frame, area),
        Screen::ModuleDetail { name } => render_detail_screen(
            &format!("Module: {}", name),
            &app.menu_items, app.menu_selected, frame, area,
        ),
        Screen::ModuleRegistry => render_list_screen("Module Registry", &app.menu_items, app.menu_selected, frame, area),
        Screen::Bot => render_detail_screen("Discord Bot", &app.menu_items, app.menu_selected, frame, area),
        Screen::BotAliases => bot::render_bot_aliases(app, frame, area),
        Screen::Settings => render_detail_screen("Settings", &app.menu_items, app.menu_selected, frame, area),
        Screen::Updates => updates::render_updates_screen(app, frame, area),
        Screen::Daemon => render_detail_screen("Saba-Core", &app.menu_items, app.menu_selected, frame, area),
        Screen::Extensions => render_detail_screen("Extensions", &app.menu_items, app.menu_selected, frame, area),
        Screen::ExtensionList => render_list_screen("Installed Extensions", &app.menu_items, app.menu_selected, frame, area),
        Screen::ExtensionDetail { ext_name, .. } => render_detail_screen(
            &format!("Extension: {}", ext_name),
            &app.menu_items, app.menu_selected, frame, area,
        ),
        Screen::ExtensionRegistry => render_list_screen("Extension Registry", &app.menu_items, app.menu_selected, frame, area),
        Screen::CreateInstanceStep1 => render_list_screen(
            "New Instance — Step 1/2: Select Game", &app.menu_items, app.menu_selected, frame, area,
        ),
        Screen::CreateInstanceStep2 { ref module_name } => {
            let mn = module_name.clone();
            create_instance::render_create_step2(app, &mn, frame, area);
        }
        Screen::CommandMode => render_command_mode(app, frame, area),
        // ServerProperties는 더 이상 사용하지 않지만 enum 호환성을 위해 남겨둠
        Screen::ServerProperties { .. } => {}
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

    if app.editor_fields.is_empty() {
        // 에디터 필드가 아직 로드되지 않음 → 로딩 표시
        let loading_text = vec![
            Line::from(""),
            Line::from(Span::styled("  ⏳ Loading...", Style::default().fg(Color::Yellow))),
            Line::from(""),
            Line::from(Span::styled(
                "  데이터를 불러오는 중입니다. 데몬이 오프라인이면 Esc로 돌아가세요.",
                Theme::dimmed(),
            )),
        ];
        frame.render_widget(Paragraph::new(loading_text), editor_area);
    } else {
        render::render_editor(app, frame, editor_area);

        // 편집 중이면 커서 표시
        if app.input_mode == InputMode::Editing {
            render::render_edit_cursor(app, frame, editor_area);
        }
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

    // ★ 자동완성 팝업 오버레이 (입력바 위에 떠오르게)
    render::render_autocomplete_popup(app, frame, chunks[1]);
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
        // ★ editor_fields가 비어있으면(로딩 중/실패) 키를 소비하지 않음
        Screen::ServerSettings { .. } => {
            if app.input_mode == InputMode::Normal && !app.editor_fields.is_empty() {
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
                        app.enter_edit_mode();
                        return true;
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
        Screen::ModuleRegistry => extensions::handle_module_registry_select(app, sel),
        Screen::Bot => bot::handle_bot_select(app, sel),
        Screen::Settings => settings::handle_settings_select(app, sel),
        Screen::Updates => updates::handle_updates_select(app, sel),
        Screen::Daemon => daemon::handle_daemon_select(app, sel),
        Screen::Extensions => extensions::handle_extensions_select(app, sel),
        Screen::ExtensionList => extensions::handle_extension_list_select(app, sel),
        Screen::ExtensionDetail { ref ext_id, .. } => {
            let ext_id = ext_id.clone();
            extensions::handle_extension_detail_select(app, sel, &ext_id);
        }
        Screen::ExtensionRegistry => extensions::handle_extension_registry_select(app, sel),
        Screen::CreateInstanceStep1 => create_instance::handle_create_step1_select(app, sel),
        Screen::CreateInstanceStep2 { .. } => {} // 인라인 모드에서 처리
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
            app.pop_screen(); // 인스턴스 목록으로 복귀
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
        ConfirmAction::ResetServer(id) => {
            tokio::spawn(async move {
                match client.reset_server(&id).await {
                    Ok(_) => push_out(&buf, vec![Out::Ok("✓ Instance reset complete".into())]),
                    Err(e) => push_out(&buf, vec![Out::Err(format!("✗ {}", e))]),
                }
            });
            app.flash("인스턴스 리셋 중...");
        }
        ConfirmAction::RemoveExtension(ext_id) => {
            let ext_id2 = ext_id.clone();
            tokio::spawn(async move {
                match client.remove_extension(&ext_id2).await {
                    Ok(_) => push_out(&buf, vec![Out::Ok(format!("✓ Extension '{}' removed", ext_id2))]),
                    Err(e) => push_out(&buf, vec![Out::Err(format!("✗ {}", e))]),
                }
            });
            app.cached_extensions.retain(|e| e.id != ext_id);
            app.pop_screen();
            app.flash("익스텐션 삭제됨");
        }
        ConfirmAction::InstallExtension(ext_id) => {
            tokio::spawn(async move {
                match client.install_extension(&ext_id, None).await {
                    Ok(_) => push_out(&buf, vec![Out::Ok(format!("✓ Extension '{}' installed", ext_id))]),
                    Err(e) => push_out(&buf, vec![Out::Err(format!("✗ {}", e))]),
                }
            });
            app.flash("익스텐션 설치 중...");
        }
        ConfirmAction::RemoveModule(module_id) => {
            tokio::spawn(async move {
                match client.remove_module(&module_id).await {
                    Ok(_) => push_out(&buf, vec![Out::Ok(format!("✓ Module '{}' removed", module_id))]),
                    Err(e) => push_out(&buf, vec![Out::Err(format!("✗ {}", e))]),
                }
            });
            app.flash("모듈 삭제됨");
        }
        ConfirmAction::InstallModuleFromRegistry(module_id) => {
            tokio::spawn(async move {
                match client.install_module_from_registry(&module_id).await {
                    Ok(_) => push_out(&buf, vec![Out::Ok(format!("✓ Module '{}' installed from registry", module_id))]),
                    Err(e) => push_out(&buf, vec![Out::Err(format!("✗ {}", e))]),
                }
            });
            app.flash("모듈 설치 중...");
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
                    // 변경사항을 일반 설정과 extension_data로 분류
                    let mut settings = serde_json::Map::new();
                    let mut ext_data = serde_json::Map::new();

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

                        // _extension_data.* 키는 extension_data 맵에 분리
                        if let Some(ext_key) = key.strip_prefix("_extension_data.") {
                            ext_data.insert(ext_key.to_string(), json_val);
                        } else {
                            settings.insert(key.clone(), json_val);
                        }
                    }

                    // extension_data가 있으면 별도 필드로 포함
                    if !ext_data.is_empty() {
                        settings.insert(
                            "extension_data".to_string(),
                            serde_json::Value::Object(ext_data),
                        );
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
            push_out(buf, vec![Out::Text(format!("EDITOR_LOAD_FAIL:Instance '{}' not found", name))]);
            return;
        }
    };

    // 인스턴스 현재 값 로드
    let inst_data = match client.get_instance(&instance_id).await {
        Ok(d) => d,
        Err(e) => {
            push_out(buf, vec![Out::Text(format!("EDITOR_LOAD_FAIL:{}", e))]);
            return;
        }
    };

    // 모듈 메타데이터 (스키마) 로드
    let module_data = client.get_module(module_name).await.ok();

    // EditorField 목록을 Out::Text로 인코딩하여 전달 (비동기→동기 경계)
    // 형식: "EDITOR_FIELD:{key}|{value}|{group}|{type}|{label}|{required}|{options}"
    let mut lines = vec![];

    if let Some(mdata) = &module_data {
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
    }
    // 모듈 메타 없거나 settings.fields가 비어있으면 인스턴스 raw 필드 표시
    if lines.is_empty() {
        if let Some(obj) = inst_data.as_object() {
            for (key, val) in obj {
                if key == "id" || key == "name" || key == "module_name" { continue; }
                // extension_data는 별도 섹션으로 표시하므로 여기서 스킵
                if key == "extension_data" { continue; }
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

    // ── InstanceSettings.fields 슬롯: 익스텐션 instance_fields 주입 ──
    // GUI의 <ExtensionSlot slotId="ServerSettings.tab"> 에 대응
    // 익스텐션이 선언한 instance_fields를 에디터 필드로 변환하여 추가
    if let Ok(exts) = client.list_extensions().await {
        let ext_data = inst_data.get("extension_data")
            .and_then(|v| v.as_object());

        for ext in &exts {
            let ext_enabled = ext["enabled"].as_bool().unwrap_or(false);
            if !ext_enabled { continue; }

            let ext_name = ext["name"].as_str().unwrap_or("Extension");
            if let Some(fields) = ext.get("instance_fields").and_then(|v| v.as_object()) {
                for (field_name, field_def) in fields {
                    let ftype = field_def.get("type").and_then(|v| v.as_str()).unwrap_or("text");

                    // 현재 값은 extension_data에서 가져옴
                    let current_val = ext_data
                        .and_then(|ed| ed.get(field_name))
                        .map(|v| match v {
                            serde_json::Value::String(s) => s.clone(),
                            serde_json::Value::Bool(b) => b.to_string(),
                            serde_json::Value::Null => String::new(),
                            _ => v.to_string(),
                        })
                        .unwrap_or_else(|| {
                            // 기본값 사용
                            field_def.get("default").map(|v| match v {
                                serde_json::Value::Bool(b) => b.to_string(),
                                serde_json::Value::Number(n) => n.to_string(),
                                serde_json::Value::String(s) => s.clone(),
                                _ => String::new(),
                            }).unwrap_or_default()
                        });

                    // extension_data 필드는 특수 접두사로 키를 인코딩
                    // 저장 시 _extension_data.key 형태로 서버에 전달
                    let editor_key = format!("_extension_data.{}", field_name);
                    let group = format!("⚡ {}", ext_name);

                    lines.push(Out::Text(format!(
                        "EDITOR_FIELD:{}|{}|{}|{}|{}|false|",
                        editor_key, current_val, group, ftype, field_name,
                    )));
                }
            }
        }
    }

    if lines.is_empty() {
        push_out(buf, vec![Out::Text(format!(
            "EDITOR_LOAD_FAIL:No configurable settings found for '{}'", name
        ))]);
        return;
    }

    push_out(buf, lines);
}
