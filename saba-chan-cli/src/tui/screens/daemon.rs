//! 데몬 관리 화면

use std::time::Duration;

use crate::tui::app::*;
use crate::process;

pub(super) fn build_daemon_menu(app: &App) -> Vec<MenuItem> {
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

pub(super) fn handle_daemon_select(app: &mut App, sel: usize) {
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
