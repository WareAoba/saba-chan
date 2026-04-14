//! 모듈 목록 · 상세 화면

use crate::tui::app::*;

pub(super) fn build_modules_menu(app: &App) -> Vec<MenuItem> {
    let t = |k| app.i18n.t(k);
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

    items.push(MenuItem::new(&format!("↻ {}", t("screen.module_refresh")), Some('r'), &t("screen.module_refresh")));
    items.push(MenuItem::new(&format!("🌐 {}", t("screen.module_manifest")), Some('R'), &t("screen.module_manifest")));
    items
}

pub(super) fn build_module_detail_menu(app: &App, name: &str) -> Vec<MenuItem> {
    let t = |k| app.i18n.t(k);
    vec![
        MenuItem::new(&t("screen.module_info"), Some('i'), &t("screen.module_info")),
        MenuItem::new(&t("screen.module_versions"), Some('v'), &t("screen.module_versions")),
        MenuItem::new(&t("screen.module_install"), Some('I'), &format!("{} — {}", t("screen.module_install"), name)),
        MenuItem::new(&format!("🗑 {}", t("screen.module_remove")), Some('D'), &format!("{} — {}", t("screen.module_remove"), name)),
    ]
}

pub(super) fn handle_modules_select(app: &mut App, sel: usize) {
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
    } else if sel == module_count + 1 {
        // Module Manifest
        app.push_screen(Screen::ModuleManifest);
    }
}

pub(super) fn handle_module_detail_select(app: &mut App, sel: usize, name: &str) {
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
        2 => { // Install → 버전 목록 조회 후 InlineSelect
            let mod_name = name.to_string();
            let buf2 = app.async_out.clone();
            let client2 = app.client.clone();
            tokio::spawn(async move {
                match client2.list_versions(&mod_name).await {
                    Ok(data) => {
                        if let Some(versions) = data.get("versions").and_then(|v| v.as_array()) {
                            let version_ids: Vec<String> = versions.iter()
                                .filter_map(|v| v.as_str().or_else(|| v["id"].as_str()).map(|s| s.to_string()))
                                .collect();
                            if version_ids.is_empty() {
                                push_out(&buf2, vec![Out::Err("사용 가능한 버전이 없습니다".into())]);
                            } else {
                                // VERSION_SELECT 프로토콜로 전달
                                let encoded = version_ids.join("|");
                                push_out(&buf2, vec![Out::Text(format!("VERSION_SELECT:{}:{}", mod_name, encoded))]);
                            }
                        } else {
                            push_out(&buf2, vec![Out::Err("버전 목록을 파싱할 수 없습니다".into())]);
                        }
                    }
                    Err(e) => push_out(&buf2, vec![Out::Err(format!("✗ {}", e))]),
                }
            });
            app.flash("버전 목록 조회 중...");
        }
        3 => { // Remove Module
            app.input_mode = InputMode::Confirm {
                prompt: format!("모듈 '{}' 을(를) 삭제하시겠습니까? (y/n)", name),
                action: ConfirmAction::RemoveModule(name.to_string()),
            };
        }
        _ => {}
    }
}
