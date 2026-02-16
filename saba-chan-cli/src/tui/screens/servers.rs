//! 서버 목록 화면

use crate::tui::app::*;

pub(super) fn build_servers_menu(app: &App) -> Vec<MenuItem> {
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
