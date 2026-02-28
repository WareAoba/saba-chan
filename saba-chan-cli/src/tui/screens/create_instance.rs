//! 인스턴스 생성 위자드 — 2단계 흐름
//!
//! GUI의 AddInstanceNewServer.js와 동일한 UX를 제공한다.
//! Step 1: 게임(모듈) 선택
//! Step 2: 인스턴스 이름 입력

use ratatui::prelude::*;
use ratatui::widgets::{Block, BorderType, Borders, Paragraph};

use crate::tui::app::*;
use crate::tui::theme::Theme;

// Step 1: 게임 모듈 선택 (메뉴 아이템으로 표시)
pub(super) fn build_create_step1_menu(app: &App) -> Vec<MenuItem> {
    let mut items: Vec<MenuItem> = app.registry.modules.iter().map(|m| {
        let mode = m.interaction_mode.as_deref().unwrap_or("-");
        // 이미 네이티브 인스턴스가 있는지 확인
        let has_native = app.servers.iter().any(|s| s.module == m.name);
        let badge = if has_native { Some("● 인스턴스 있음".into()) } else { None };

        let mut item = MenuItem::new(
            &format!("🎮 {}", m.display_name),
            None,
            &format!("[{}] mode: {}", m.name, mode),
        );
        item.badge = badge;
        item
    }).collect();

    if items.is_empty() {
        items.push(MenuItem::new(
            "(설치된 모듈 없음)", None,
            "Modules 메뉴에서 모듈을 먼저 설치하세요",
        ).with_enabled(false));
    }
    items
}

// Step 1: Enter → Step 2 진입
pub(super) fn handle_create_step1_select(app: &mut App, sel: usize) {
    if sel < app.registry.modules.len() {
        let module = &app.registry.modules[sel];
        let module_name = module.name.clone();

        // 자동 이름 생성 (GUI와 동일: my-{module}-{n})
        let existing_count = app.servers.iter()
            .filter(|s| s.module == module_name)
            .count();
        let default_name = format!("my-{}-{}", module_name, existing_count + 1);

        app.push_screen(Screen::CreateInstanceStep2 {
            module_name: module_name.clone(),
        });

        // ── CreateInstance.options 슬롯에서 익스텐션 옵션 수집 ──
        // GUI의 <ExtensionSlot slotId="AddServer.options"> 에 대응
        let create_slots = app.ext_slots.get_slot("CreateInstance.options");
        let mut ext_option_summary: Vec<String> = Vec::new();
        for slot in create_slots {
            if let Some(options) = slot.data.as_array() {
                for opt in options {
                    let field = opt.get("field").and_then(|v| v.as_str()).unwrap_or("?");
                    let label = opt.get("label").and_then(|v| v.as_str()).unwrap_or(field);
                    ext_option_summary.push(format!("{} ({})", label, slot.extension_name));
                    // 옵션 정보를 App에 저장 (인스턴스 생성 시 사용)
                    let _field_type = opt.get("type").and_then(|v| v.as_str()).unwrap_or("boolean");
                    let _default = opt.get("default");
                    // Note: 현재 인라인 입력 방식에서는 이름 입력이 우선
                    // 향후 multi-step wizard로 확장 시 각 옵션을 별도 단계로 분리 가능
                    let _ = (field, label, _field_type, _default);
                }
            }
        }

        // 인라인 입력 모드 진입 (이름 입력)
        app.input_mode = InputMode::InlineInput {
            prompt: format!("{} 서버 인스턴스 이름", module_name),
            value: default_name.clone(),
            cursor: default_name.chars().count(),
            on_submit: InlineAction::CreateInstance { module_name },
        };
    }
}

// Step 2: 렌더링 (선택된 게임 요약 + 이름 입력 필드 + 익스텐션 옵션)
pub(super) fn render_create_step2(
    app: &App, module_name: &str,
    frame: &mut ratatui::Frame, area: ratatui::prelude::Rect,
) {
    let block = Block::default()
        .title(" New Server — Step 2/2: Configure ")
        .borders(Borders::ALL)
        .border_type(BorderType::Rounded)
        .border_style(Theme::border_active());

    let inner = block.inner(area);
    frame.render_widget(block, area);

    let module_info = app.registry.get_module(module_name);
    let display_name = module_info
        .map(|m| m.display_name.as_str())
        .unwrap_or(module_name);

    let mut lines = vec![
        Line::from(""),
        Line::from(vec![
            Span::styled("  Selected Game: ", Theme::dimmed()),
            Span::styled(display_name, Theme::title()),
        ]),
        Line::from(""),
        Line::from(Span::styled(
            "  인스턴스 이름을 입력하세요 (모달에서 입력 중...)",
            Theme::dimmed(),
        )),
        Line::from(""),
    ];

    // ── CreateInstance.options 슬롯: 익스텐션 옵션 표시 ──
    // GUI의 <ExtensionSlot slotId="AddServer.options"> 렌더링에 대응
    let create_slots = app.ext_slots.get_slot("CreateInstance.options");
    if !create_slots.is_empty() {
        lines.push(Line::from(Span::styled(
            "  ⚡ Extension Options:",
            Theme::title(),
        )));
        lines.push(Line::from(""));

        for slot in create_slots {
            if let Some(options) = slot.data.as_array() {
                for opt in options {
                    let label = opt.get("label").and_then(|v| v.as_str()).unwrap_or("?");
                    let ftype = opt.get("type").and_then(|v| v.as_str()).unwrap_or("text");
                    let default = opt.get("default");
                    let default_str = match default {
                        Some(serde_json::Value::Bool(b)) => if *b { "enabled" } else { "disabled" },
                        Some(serde_json::Value::Number(n)) => &n.to_string(),
                        Some(serde_json::Value::String(s)) => s.as_str(),
                        _ => "-",
                    };

                    lines.push(Line::from(vec![
                        Span::styled("    ", Style::default()),
                        Span::styled(
                            format!("{} ", label),
                            Style::default().fg(Color::Cyan),
                        ),
                        Span::styled(
                            format!("[{}: {}]", ftype, default_str),
                            Theme::dimmed(),
                        ),
                    ]));
                }
            }
            lines.push(Line::from(Span::styled(
                format!("    └─ from: {}", slot.extension_name),
                Theme::dimmed(),
            )));
        }

        lines.push(Line::from(""));
        lines.push(Line::from(Span::styled(
            "  (인스턴스 생성 후 Settings에서 옵션을 변경할 수 있습니다)",
            Theme::dimmed(),
        )));
    }

    frame.render_widget(Paragraph::new(lines), inner);
}
