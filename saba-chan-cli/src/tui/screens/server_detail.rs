//! 서버 상세 화면 — 시작/정지, 콘솔, 설정, 진단 등

use std::time::Duration;

use crate::tui::app::*;

use super::{find_instance_id, load_instance_settings, load_server_properties};

pub(super) fn build_server_detail_menu(app: &App, name: &str) -> Vec<MenuItem> {
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
