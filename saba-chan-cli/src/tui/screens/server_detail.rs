//! 인스턴스 상세 화면 — 시작/정지, 콘솔, 설정, 진단 등

use std::time::Duration;

use crate::tui::app::*;

use super::find_instance_id;

pub(super) fn build_server_detail_menu(app: &App, name: &str) -> Vec<MenuItem> {
    let is_running = app.servers.iter().any(|s| s.name == name && s.status == "running");

    let mut items = vec![
        if is_running {
            MenuItem::new("■ Stop", Some('s'), "인스턴스 정지")
        } else {
            MenuItem::new("▶ Start", Some('s'), "인스턴스 시작")
        },
    ];
    if is_running {
        items.push(MenuItem::new("↻ Restart", Some('r'), "인스턴스 재시작"));
    }
    items.extend([
        MenuItem::new("📟 Console", Some('c'), "서버 콘솔 (실시간)"),
        MenuItem::new("⚙ Settings", Some('e'), "인스턴스 설정 편집"),
    ]);

    items.push(MenuItem::new("💻 Execute Command", Some('x'), "서버 명령어 실행"));

    // ── InstanceDetail.menu 슬롯 주입 ──
    // GUI의 <ExtensionSlot slotId="ServerCard.expandedStats"> 등에 대응
    let server_ext_data = app.servers.iter()
        .find(|s| s.name == name)
        .map(|s| &s.extension_data);

    let detail_menu_slots = app.ext_slots.get_slot("InstanceDetail.menu");
    for slot in detail_menu_slots {
        if let Some(menu_items) = slot.data.as_array() {
            for menu_item in menu_items {
                // 조건 평가: condition이 있으면 인스턴스의 ext_data를 확인
                if let Some(condition) = menu_item.get("condition").and_then(|v| v.as_str()) {
                    if let Some(key) = condition.strip_prefix("instance.ext_data.") {
                        let enabled = server_ext_data
                            .and_then(|ed| ed.get(key))
                            .and_then(|v| v.as_bool())
                            .unwrap_or(false);
                        if !enabled { continue; }
                    }
                }

                let label = menu_item.get("label").and_then(|v| v.as_str()).unwrap_or("?");
                let desc = menu_item.get("description").and_then(|v| v.as_str()).unwrap_or("");
                let action = menu_item.get("action").and_then(|v| v.as_str()).unwrap_or("");

                let mut item = MenuItem::new(label, None, desc);
                // action을 badge로 표시 (내부 식별자)
                item.badge = Some(format!("ext:{}/{}", slot.extension_id, action));
                items.push(item);
            }
        }
    }

    // ── InstanceDetail.status 슬롯 주입 (상태 정보 라인) ──
    let status_slots = app.ext_slots.get_slot("InstanceDetail.status");
    for slot in status_slots {
        if let Some(status_items) = slot.data.as_array() {
            for status_item in status_items {
                let label = status_item.get("label").and_then(|v| v.as_str()).unwrap_or("?");
                let value_key = status_item.get("value_from").and_then(|v| v.as_str()).unwrap_or("");

                // ext_data에서 값 조회
                let value = server_ext_data
                    .and_then(|ed| ed.get(value_key))
                    .map(|v| match v {
                        serde_json::Value::String(s) => s.clone(),
                        _ => v.to_string(),
                    })
                    .unwrap_or_else(|| "-".to_string());

                let mut item = MenuItem::new(
                    &format!("📊 {}: {}", label, value),
                    None, "",
                ).with_enabled(false);
                item.badge = Some(format!("[{}]", slot.extension_name));
                items.push(item);
            }
        }
    }

    items.push(MenuItem::new("🔍 Diagnose", Some('d'), "서버 진단"));

    items.push(MenuItem::new("⚠ Reset Instance", Some('W'), "인스턴스 리셋 (월드/설정 삭제)"));
    items.push(MenuItem::new("🗑 Delete Instance", Some('D'), "인스턴스 삭제"));
    items
}

pub(super) fn handle_server_detail_select(
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
        Some('r') => { // Restart (only shown when running)
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
                super::load_instance_settings(&client2, &inst_name, &mod_name, &buf2).await;
            });
        }
        Some('x') => { // Execute Command → 인라인 Input
            let iid = if id.is_empty() { name.to_string() } else { id.to_string() };
            app.input_mode = InputMode::InlineInput {
                prompt: format!("서버 명령어 ({})", name),
                value: String::new(),
                cursor: 0,
                on_submit: InlineAction::ExecuteCommand { instance_id: iid },
            };
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
        Some('W') => { // Reset Instance
            app.input_mode = InputMode::Confirm {
                prompt: format!("Reset instance '{}'? This will DELETE world data, configs, and logs!", name),
                action: ConfirmAction::ResetServer(id.to_string()),
            };
        }
        Some('D') => { // Delete
            app.input_mode = InputMode::Confirm {
                prompt: format!("Delete instance '{}'?", name),
                action: ConfirmAction::DeleteInstance(id.to_string()),
            };
        }
        _ => {
            // ── 익스텐션 주입 메뉴 아이템 처리 ──
            // badge에 "ext:{ext_id}/{action}" 형태로 인코딩된 액션을 디스패치
            if let Some(badge) = app.menu_items.get(sel).and_then(|item| item.badge.as_ref()) {
                if let Some(ext_action) = badge.strip_prefix("ext:") {
                    if let Some((_ext_id, action)) = ext_action.split_once('/') {
                        match action {
                            "open_ext_settings" => {
                                // 인스턴스 설정 화면으로 이동 (익스텐션 필드 포함)
                                app.editor_fields.clear();
                                app.editor_selected = 0;
                                app.editor_changes.clear();
                                app.push_screen(Screen::ServerSettings {
                                    name: name.clone(),
                                    id: id.clone(),
                                    module_name: module_name.clone(),
                                });

                                let buf2 = app.async_out.clone();
                                let client2 = app.client.clone();
                                let inst_name = name.clone();
                                let mod_name = module_name.clone();
                                tokio::spawn(async move {
                                    super::load_instance_settings(&client2, &inst_name, &mod_name, &buf2).await;
                                });
                            }
                            _ => {
                                app.flash(&format!("Unknown ext action: {}", action));
                            }
                        }
                    }
                }
            }
        }
    }
}