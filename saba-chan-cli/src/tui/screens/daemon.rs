//! 데몬 관리 화면

use std::time::Duration;

use crate::tui::app::*;
use crate::process;

pub(super) fn build_daemon_menu(app: &App) -> Vec<MenuItem> {
    let t = |k| app.i18n.t(k);
    let is_running = app.daemon_on;
    let items = vec![
        if is_running {
            MenuItem::new(&format!("■ {}", t("screen.daemon_stop")), Some('s'), &t("screen.daemon_stop"))
        } else {
            MenuItem::new(&format!("▶ {}", t("screen.daemon_start")), Some('s'), &t("screen.daemon_start"))
        },
        MenuItem::new(&format!("↻ {}", t("screen.daemon_restart")), Some('r'), &t("screen.daemon_restart")),
        MenuItem::new(&format!("ℹ {}", t("screen.daemon_status")), Some('i'), &t("screen.daemon_status")),
        MenuItem::new(&format!("📊 {}", t("screen.daemon_components")), Some('c'), &t("screen.daemon_components")),
    ];

    items
}

pub(super) fn handle_daemon_select(app: &mut App, sel: usize) {
    let buf = app.async_out.clone();
    let client = app.client.clone();

    let shortcut = app.menu_items.get(sel).and_then(|item| item.shortcut);

    match shortcut {
        Some('s') => { // Start/Stop
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
            app.flash(if app.daemon_on { "Saba-Core 정지 중..." } else { "Saba-Core 시작 중..." });
        }
        Some('r') => { // Restart
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
            app.flash("Saba-Core 재시작 중...");
        }
        Some('i') => { // Status
            tokio::spawn(async move {
                let running = tokio::task::spawn_blocking(process::check_daemon_running)
                    .await.unwrap_or(false);
                if running {
                    let base = crate::config::get_ipc_base_url();
                    let port = crate::config::get_ipc_port();
                    let mut lines = vec![Out::Ok("Saba-Core: ● RUNNING".into())];
                    lines.push(Out::Text("  Host:     127.0.0.1".into()));
                    lines.push(Out::Text(format!("  Port:     {}", port)));
                    lines.push(Out::Text("  Protocol: HTTP REST".into()));

                    if let Ok(mods) = client.list_modules().await {
                        lines.push(Out::Text(format!("  Modules:  {}", mods.len())));
                    }
                    if let Ok(srvs) = client.list_servers().await {
                        let running_count = srvs.iter()
                            .filter(|s| s["status"].as_str() == Some("running"))
                            .count();
                        lines.push(Out::Text(format!("  Servers:  {}/{} running", running_count, srvs.len())));
                    }
                    if let Ok(exts) = client.list_extensions().await {
                        let enabled = exts.iter()
                            .filter(|e| e["enabled"].as_bool().unwrap_or(false))
                            .count();
                        lines.push(Out::Text(format!("  Extensions: {}/{} enabled", enabled, exts.len())));
                    }

                    let _ = base; // already used via client
                    push_out(&buf, lines);
                } else {
                    push_out(&buf, vec![Out::Text("Saba-Core: ○ OFFLINE".into())]);
                }
            });
        }
        Some('c') => { // Components
            if !app.daemon_on {
                app.flash("⚠ Saba-Core가 오프라인입니다");
                return;
            }
            tokio::spawn(async move {
                match client.get_system_components().await {
                    Ok(data) => {
                        let mut lines = vec![Out::Ok("System Components:".into())];
                        if let Some(obj) = data.as_object() {
                            for (key, val) in obj {
                                let display = match val {
                                    serde_json::Value::String(s) => s.clone(),
                                    serde_json::Value::Object(map) => {
                                        let version = map.get("version").and_then(|v| v.as_str()).unwrap_or("?");
                                        let path = map.get("path").and_then(|v| v.as_str()).unwrap_or("");
                                        if path.is_empty() {
                                            version.to_string()
                                        } else {
                                            format!("v{} ({})", version, path)
                                        }
                                    }
                                    _ => val.to_string(),
                                };
                                lines.push(Out::Text(format!("  {:<20} {}", key, display)));
                            }
                        } else if let Some(arr) = data.as_array() {
                            for comp in arr {
                                let name = comp.get("name").and_then(|v| v.as_str()).unwrap_or("?");
                                let ver = comp.get("version").and_then(|v| v.as_str()).unwrap_or("?");
                                lines.push(Out::Text(format!("  {:<20} v{}", name, ver)));
                            }
                        }
                        push_out(&buf, lines);
                    }
                    Err(e) => push_out(&buf, vec![Out::Err(format!("✗ {}", e))]),
                }
            });
            app.flash("컴포넌트 정보 조회 중...");
        }
        _ => {}
    }
}
