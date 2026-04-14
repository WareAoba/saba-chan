//! 익스텐션 관리 화면 — 목록, 활성화/비활성화, 매니페스트, 설치/삭제

use crate::client::DaemonClient;
use crate::tui::app::*;

/// 익스텐션 슬롯 레지스트리 재로드 트리거
/// GUI의 ExtensionContext 리프레시에 대응
async fn reload_ext_slots(client: &DaemonClient, buf: &OutputBuf) {
    if let Ok(exts) = client.list_extensions().await {
        let data = serde_json::to_string(&exts).unwrap_or_default();
        push_out(buf, vec![Out::Text(format!("EXT_SLOTS_INIT:{}", data))]);
    }
}

// ═══════════════════════════════════════════════════════
// 메인 익스텐션 메뉴
// ═══════════════════════════════════════════════════════

pub(super) fn build_extensions_menu(app: &App) -> Vec<MenuItem> {
    let t = |k| app.i18n.t(k);
    vec![
        MenuItem::new(&format!("📦 {}", t("screen.ext_installed")), Some('i'), &t("screen.ext_installed")),
        MenuItem::new(&format!("🌐 {}", t("screen.ext_manifest")), Some('r'), &t("screen.ext_manifest")),
        MenuItem::new(&format!("🔄 {}", t("screen.ext_check_updates")), Some('u'), &t("screen.ext_check_updates")),
        MenuItem::new(&format!("🔍 {}", t("screen.ext_rescan")), Some('s'), &t("screen.ext_rescan")),
    ]
}

pub(super) fn handle_extensions_select(app: &mut App, sel: usize) {
    let client = app.client.clone();
    let buf = app.async_out.clone();

    match sel {
        0 => { // Installed Extensions
            app.push_screen(Screen::ExtensionList);
            tokio::spawn(async move {
                match client.list_extensions().await {
                    Ok(list) => {
                        let mut lines = vec![];
                        if list.is_empty() {
                            lines.push(Out::Info("No extensions installed.".into()));
                        } else {
                            for ext in &list {
                                let id = ext["id"].as_str().or_else(|| ext["name"].as_str()).unwrap_or("?");
                                let name = ext["display_name"].as_str().or_else(|| ext["name"].as_str()).unwrap_or("?");
                                let ver = ext["version"].as_str().unwrap_or("?");
                                let enabled = ext["enabled"].as_bool().unwrap_or(true);
                                let status = if enabled { "✓" } else { "○" };
                                lines.push(Out::Text(format!(
                                    "EXT_ITEM:{}|{}|{}|{}",
                                    id, name, ver, status,
                                )));
                            }
                        }
                        push_out(&buf, lines);
                    }
                    Err(e) => push_out(&buf, vec![Out::Err(format!("✗ {}", e))]),
                }
            });
            app.flash("익스텐션 목록 로드 중...");
        }
        1 => { // Extension Manifest
            app.push_screen(Screen::ExtensionManifest);
            tokio::spawn(async move {
                match client.fetch_extension_manifest().await {
                    Ok(data) => {
                        let mut lines = vec![];
                        let exts = data.get("extensions").and_then(|v| v.as_array())
                            .or_else(|| data.as_array());
                        if let Some(arr) = exts {
                            if arr.is_empty() {
                                lines.push(Out::Info("Manifest is empty.".into()));
                            } else {
                                for ext in arr {
                                    let id = ext["id"].as_str().or_else(|| ext["name"].as_str()).unwrap_or("?");
                                    let name = ext["display_name"].as_str().or_else(|| ext["name"].as_str()).unwrap_or("?");
                                    let ver = ext["version"].as_str().or_else(|| ext["latest_version"].as_str()).unwrap_or("?");
                                    let desc = ext["description"].as_str().unwrap_or("");
                                    lines.push(Out::Text(format!(
                                        "REG_ITEM:{}|{}|{}|{}",
                                        id, name, ver, desc,
                                    )));
                                }
                            }
                        } else {
                            lines.push(Out::Info(format!("Manifest: {}", data)));
                        }
                        push_out(&buf, lines);
                    }
                    Err(e) => push_out(&buf, vec![Out::Err(format!("✗ {}", e))]),
                }
            });
            app.flash("매니페스트 조회 중...");
        }
        2 => { // Check Updates
            tokio::spawn(async move {
                match client.check_extension_updates().await {
                    Ok(data) => {
                        let updates = data.get("updates").and_then(|v| v.as_array())
                            .or_else(|| data.as_array());
                        if let Some(arr) = updates {
                            if arr.is_empty() {
                                push_out(&buf, vec![Out::Ok("All extensions are up to date.".into())]);
                            } else {
                                let mut lines = vec![Out::Ok(format!("{} update(s) available:", arr.len()))];
                                for u in arr {
                                    let id = u["id"].as_str().or_else(|| u["name"].as_str()).unwrap_or("?");
                                    let cur = u["current_version"].as_str().unwrap_or("?");
                                    let lat = u["latest_version"].as_str().unwrap_or("?");
                                    lines.push(Out::Text(format!("  ⬆ {}: {} → {}", id, cur, lat)));
                                }
                                push_out(&buf, lines);
                            }
                        } else {
                            push_out(&buf, vec![Out::Ok(format!("{}", data))]);
                        }
                    }
                    Err(e) => push_out(&buf, vec![Out::Err(format!("✗ {}", e))]),
                }
            });
            app.flash("업데이트 확인 중...");
        }
        3 => { // Rescan
            tokio::spawn(async move {
                match client.rescan_extensions().await {
                    Ok(_) => push_out(&buf, vec![Out::Ok("✓ Extensions rescanned".into())]),
                    Err(e) => push_out(&buf, vec![Out::Err(format!("✗ {}", e))]),
                }
            });
            app.flash("재스캔 중...");
        }
        _ => {}
    }
}

// ═══════════════════════════════════════════════════════
// 설치된 익스텐션 목록 화면
// ═══════════════════════════════════════════════════════

pub(super) fn build_extension_list_menu(app: &App) -> Vec<MenuItem> {
    let mut items: Vec<MenuItem> = app.cached_extensions.iter().map(|ext| {
        let enabled = ext.enabled;
        let status_icon = if enabled { "✓" } else { "○" };
        MenuItem::new(
            &format!("{} {}", status_icon, ext.name),
            None,
            &format!("v{} — {}", ext.version, ext.id),
        )
    }).collect();

    if items.is_empty() {
        items.push(MenuItem::new("(No extensions installed)", None, "").with_enabled(false));
    }

    items.push(MenuItem::new("↻ Refresh", Some('r'), "익스텐션 목록 새로고침"));
    items
}

pub(super) fn handle_extension_list_select(app: &mut App, sel: usize) {
    let ext_count = app.cached_extensions.len();

    if sel < ext_count {
        let ext = app.cached_extensions[sel].clone();
        app.push_screen(Screen::ExtensionDetail {
            ext_id: ext.id.clone(),
            ext_name: ext.name.clone(),
        });
    } else if sel == ext_count {
        // Refresh
        let client = app.client.clone();
        let buf = app.async_out.clone();
        tokio::spawn(async move {
            match client.list_extensions().await {
                Ok(list) => {
                    let mut lines = vec![];
                    for ext in &list {
                        let id = ext["id"].as_str().or_else(|| ext["name"].as_str()).unwrap_or("?");
                        let name = ext["display_name"].as_str().or_else(|| ext["name"].as_str()).unwrap_or("?");
                        let ver = ext["version"].as_str().unwrap_or("?");
                        let enabled = ext["enabled"].as_bool().unwrap_or(true);
                        let status = if enabled { "✓" } else { "○" };
                        lines.push(Out::Text(format!("EXT_ITEM:{}|{}|{}|{}", id, name, ver, status)));
                    }
                    push_out(&buf, lines);
                }
                Err(e) => push_out(&buf, vec![Out::Err(format!("✗ {}", e))]),
            }
        });
        app.flash("새로고침 중...");
    }
}

// ═══════════════════════════════════════════════════════
// 익스텐션 상세 화면
// ═══════════════════════════════════════════════════════

pub(super) fn build_extension_detail_menu(app: &App, ext_id: &str) -> Vec<MenuItem> {
    let t = |k| app.i18n.t(k);
    let is_enabled = app.cached_extensions.iter()
        .find(|e| e.id == ext_id)
        .map(|e| e.enabled)
        .unwrap_or(true);

    vec![
        if is_enabled {
            MenuItem::new(&format!("○ {}", t("screen.ext_disable")), Some('d'), &t("screen.ext_disable"))
        } else {
            MenuItem::new(&format!("✓ {}", t("screen.ext_enable")), Some('e'), &t("screen.ext_enable"))
        },
        MenuItem::new(&format!("🔧 {}", t("screen.ext_config")), Some('c'), &t("screen.ext_config")),
        MenuItem::new(&format!("🗑 {}", t("screen.ext_remove")), Some('D'), &t("screen.ext_remove")),
    ]
}

pub(super) fn handle_extension_detail_select(app: &mut App, sel: usize, ext_id: &str) {
    let client = app.client.clone();
    let client2 = app.client.clone();
    let buf = app.async_out.clone();
    let ext_id = ext_id.to_string();

    let is_enabled = app.cached_extensions.iter()
        .find(|e| e.id == ext_id)
        .map(|e| e.enabled)
        .unwrap_or(true);

    match sel {
        0 => { // Enable/Disable toggle
            let ext_id2 = ext_id.clone();
            if is_enabled {
                let buf2 = buf.clone();
                tokio::spawn(async move {
                    match client.disable_extension(&ext_id2).await {
                        Ok(_) => {
                            push_out(&buf, vec![Out::Ok(format!("✓ Extension '{}' disabled", ext_id2))]);
                            // 슬롯 레지스트리 재로드 트리거
                            reload_ext_slots(&client2, &buf2).await;
                        }
                        Err(e) => push_out(&buf, vec![Out::Err(format!("✗ {}", e))]),
                    }
                });
                // Update cached state
                if let Some(ext) = app.cached_extensions.iter_mut().find(|e| e.id == ext_id) {
                    ext.enabled = false;
                }
                app.flash("비활성화 중...");
            } else {
                let buf2 = buf.clone();
                tokio::spawn(async move {
                    match client.enable_extension(&ext_id2).await {
                        Ok(_) => {
                            push_out(&buf, vec![Out::Ok(format!("✓ Extension '{}' enabled", ext_id2))]);
                            // 슬롯 레지스트리 재로드 트리거
                            reload_ext_slots(&client2, &buf2).await;
                        }
                        Err(e) => push_out(&buf, vec![Out::Err(format!("✗ {}", e))]),
                    }
                });
                if let Some(ext) = app.cached_extensions.iter_mut().find(|e| e.id == ext_id) {
                    ext.enabled = true;
                }
                app.flash("활성화 중...");
            }
        }
        1 => { // Config → 조회 후 편집 InlineInput
            let ext_id_c = ext_id.clone();
            let ext_id_edit = ext_id.clone();
            tokio::spawn(async move {
                match client.get_extension_config(&ext_id_c).await {
                    Ok(config) => {
                        let mut lines = vec![Out::Ok(format!("=== Extension Config: {} ===", ext_id_c))];
                        if let Some(obj) = config.as_object() {
                            if obj.is_empty() {
                                lines.push(Out::Text("  (no config)".into()));
                            } else {
                                for (k, v) in obj {
                                    lines.push(Out::Text(format!("  {} = {}", k, v)));
                                }
                            }
                        } else {
                            lines.push(Out::Text(format!("  {}", config)));
                        }
                        lines.push(Out::Blank);
                        lines.push(Out::Info("Edit: select 'Config' again and enter key=value".into()));
                        push_out(&buf, lines);
                    }
                    Err(e) => push_out(&buf, vec![Out::Err(format!("✗ {}", e))]),
                }
            });
            // 키=값 편집용 InlineInput
            app.input_mode = InputMode::InlineInput {
                prompt: format!("{} config (key=value, 빈 값은 조회만)", ext_id_edit),
                value: String::new(),
                cursor: 0,
                on_submit: InlineAction::Custom(format!("EXT_CONFIG:{}", ext_id)),
            };
        }
        2 => { // Remove
            app.input_mode = InputMode::Confirm {
                prompt: format!("Remove extension '{}'?", ext_id),
                action: ConfirmAction::RemoveExtension(ext_id),
            };
        }
        _ => {}
    }
}

// ═══════════════════════════════════════════════════════
// 익스텐션 매니페스트 화면
// ═══════════════════════════════════════════════════════

pub(super) fn build_extension_manifest_menu(app: &App) -> Vec<MenuItem> {
    let mut items: Vec<MenuItem> = app.cached_manifest_extensions.iter().map(|ext| {
        MenuItem::new(
            &ext.name,
            None,
            &format!("v{} — {}", ext.version, ext.description),
        )
    }).collect();

    if items.is_empty() {
        items.push(MenuItem::new("(Loading or empty manifest...)", None, "").with_enabled(false));
    }

    items.push(MenuItem::new("↻ Refresh Manifest", Some('r'), "매니페스트 새로고침"));
    items
}

pub(super) fn handle_extension_manifest_select(app: &mut App, sel: usize) {
    let reg_count = app.cached_manifest_extensions.len();

    if sel < reg_count {
        let ext = &app.cached_manifest_extensions[sel];
        let ext_id = ext.id.clone();
        let ext_name = ext.name.clone();

        app.input_mode = InputMode::Confirm {
            prompt: format!("Install extension '{}'?", ext_name),
            action: ConfirmAction::InstallExtension(ext_id),
        };
    } else if sel == reg_count {
        // Refresh manifest
        let client = app.client.clone();
        let buf = app.async_out.clone();
        tokio::spawn(async move {
            match client.fetch_extension_manifest().await {
                Ok(data) => {
                    let mut lines = vec![];
                    let exts = data.get("extensions").and_then(|v| v.as_array())
                        .or_else(|| data.as_array());
                    if let Some(arr) = exts {
                        for ext in arr {
                            let id = ext["id"].as_str().or_else(|| ext["name"].as_str()).unwrap_or("?");
                            let name = ext["display_name"].as_str().or_else(|| ext["name"].as_str()).unwrap_or("?");
                            let ver = ext["version"].as_str().or_else(|| ext["latest_version"].as_str()).unwrap_or("?");
                            let desc = ext["description"].as_str().unwrap_or("");
                            lines.push(Out::Text(format!("REG_ITEM:{}|{}|{}|{}", id, name, ver, desc)));
                        }
                    }
                    push_out(&buf, lines);
                }
                Err(e) => push_out(&buf, vec![Out::Err(format!("✗ {}", e))]),
            }
        });
        app.flash("매니페스트 새로고침 중...");
    }
}

// ═══════════════════════════════════════════════════════
// 모듈 매니페스트 화면
// ═══════════════════════════════════════════════════════

pub(super) fn build_module_manifest_menu(app: &App) -> Vec<MenuItem> {
    let mut items: Vec<MenuItem> = app.cached_manifest_modules.iter().map(|m| {
        MenuItem::new(
            &m.name,
            None,
            &format!("v{} — {}", m.version, m.description),
        )
    }).collect();

    if items.is_empty() {
        items.push(MenuItem::new("(Loading or empty manifest...)", None, "").with_enabled(false));
    }

    items.push(MenuItem::new("↻ Refresh Manifest", Some('r'), "매니페스트 새로고침"));
    items
}

pub(super) fn handle_module_manifest_select(app: &mut App, sel: usize) {
    let reg_count = app.cached_manifest_modules.len();

    if sel < reg_count {
        let m = &app.cached_manifest_modules[sel];
        let mod_id = m.id.clone();
        let mod_name = m.name.clone();

        app.input_mode = InputMode::Confirm {
            prompt: format!("Install module '{}'?", mod_name),
            action: ConfirmAction::InstallModuleFromManifest(mod_id),
        };
    } else if sel == reg_count {
        // Refresh manifest
        let client = app.client.clone();
        let buf = app.async_out.clone();
        tokio::spawn(async move {
            match client.fetch_module_manifest().await {
                Ok(data) => {
                    let mut lines = vec![];
                    let mods = data.get("modules").and_then(|v| v.as_array())
                        .or_else(|| data.as_array());
                    if let Some(arr) = mods {
                        for m in arr {
                            let id = m["id"].as_str().or_else(|| m["name"].as_str()).unwrap_or("?");
                            let name = m["display_name"].as_str().or_else(|| m["name"].as_str()).unwrap_or("?");
                            let ver = m["version"].as_str().or_else(|| m["latest_version"].as_str()).unwrap_or("?");
                            let desc = m["description"].as_str().unwrap_or("");
                            lines.push(Out::Text(format!("MODREG_ITEM:{}|{}|{}|{}", id, name, ver, desc)));
                        }
                    }
                    push_out(&buf, lines);
                }
                Err(e) => push_out(&buf, vec![Out::Err(format!("✗ {}", e))]),
            }
        });
        app.flash("모듈 매니페스트 새로고침 중...");
    }
}
