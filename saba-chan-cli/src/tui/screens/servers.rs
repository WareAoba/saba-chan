//! 인스턴스 목록 화면

use crate::tui::app::*;

/// 익스텐션 슬롯 조건을 평가한다 (GUI의 DockerBadge 조건 평가에 대응)
/// condition 형식: "instance.ext_data.<key>"
fn evaluate_badge_condition(condition: &str, server: &ServerInfo) -> bool {
    if let Some(key) = condition.strip_prefix("instance.ext_data.") {
        server.extension_data.get(key)
            .and_then(|v| v.as_bool())
            .unwrap_or(false)
    } else {
        true // 조건이 없거나 인식 불가 → 표시
    }
}

pub(super) fn build_servers_menu(app: &App) -> Vec<MenuItem> {
    // ── InstanceList.badge 슬롯 조회 ──
    let badge_slots = app.ext_slots.get_slot("InstanceList.badge");

    let mut items: Vec<MenuItem> = app.servers.iter().map(|s| {
        let sym = if s.status == "running" { "▶" } else { "■" };
        let mut item = MenuItem::new(
            &format!("{} {}", sym, s.name),
            None,
            &format!("[{}] {}", s.module, s.status),
        );

        // ── 익스텐션 뱃지 주입 (GUI의 <ExtensionSlot slotId="ServerCard.badge"> 대응) ──
        let mut badges = Vec::new();
        for slot in badge_slots {
            if let Some(condition) = slot.data.get("condition").and_then(|v| v.as_str()) {
                if !evaluate_badge_condition(condition, s) {
                    continue;
                }
            }
            if let Some(text) = slot.data.get("text").and_then(|v| v.as_str()) {
                badges.push(text.to_string());
            }
        }
        if !badges.is_empty() {
            item.badge = Some(badges.join(" "));
        }

        item
    }).collect();

    if items.is_empty() {
        items.push(MenuItem::new("(No instances configured)", None, "").with_enabled(false));
    }

    items.push(MenuItem::new("+ New Instance", Some('n'), "새 인스턴스 생성"));
    items
}

pub(super) fn handle_servers_select(app: &mut App, sel: usize) {
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
                        let id = inst["id"].as_str().unwrap_or("").to_string();
                        push_out(&buf, vec![Out::Info(format!("Instance ID: {}", id))]);
                        return;
                    }
                }
            }
        });
    } else if sel == server_count {
        // New Server → 인스턴스 생성 위자드 Step 1
        app.push_screen(Screen::CreateInstanceStep1);
    }
}
