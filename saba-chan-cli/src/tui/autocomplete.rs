//! 자동완성 엔진 — 명령어, 서브커맨드, 인스턴스 이름, 모듈 이름, 슬래시 명령 등.
//!
//! Stage 4에서 `commands.rs::update_autocomplete_preview`를 흡수하며,
//! ghost-text 계산(입력 뒤에 dimmed suffix)을 담당한다.

use super::app::App;

const TOP_COMMANDS: &[&str] = &[
    "instance", "module", "extension",
    "daemon", "config", "update", "help",
    "exit", "back", "menu", "migration",
];

const SLASH_COMMANDS: &[&str] = &[
    "help", "clear", "status", "quit",
    "theme", "tab", "find", "run",
];

/// 현재 입력(`app.input`)에서 가능한 자동완성 후보를 계산하고,
/// 가장 유력한 후보의 "남은 부분"을 `ghost_suffix`로 설정한다.
pub fn refresh(app: &mut App) {
    // 슬래시 모드인가?
    if let Some(ref buf) = app.slash_buffer {
        refresh_slash(app, buf.clone());
        return;
    }

    let input_full = app.input.clone();
    let trimmed = input_full.trim_start();
    let input = trimmed.to_string();

    app.autocomplete_candidates.clear();
    app.autocomplete_visible = false;
    app.ghost_suffix = None;

    if input.is_empty() {
        return;
    }

    let parts: Vec<&str> = input.split_whitespace().collect();
    let ends_with_space = input_full.ends_with(' ');

    let mut candidates: Vec<String> = Vec::new();

    if parts.len() == 1 && !ends_with_space {
        let prefix = parts[0].to_lowercase();
        for cmd in TOP_COMMANDS {
            if cmd.starts_with(&prefix) && *cmd != parts[0] {
                candidates.push((*cmd).to_string());
            }
        }
        for name in app.registry.module_names() {
            let lower = name.to_lowercase();
            if lower.starts_with(&prefix) && name != parts[0] {
                candidates.push(name.to_string());
            }
        }
        for inst in &app.servers {
            let lower = inst.name.to_lowercase();
            if lower.starts_with(&prefix) && inst.name != parts[0] {
                candidates.push(inst.name.clone());
            }
        }
    } else if parts.len() == 2 || (parts.len() == 1 && ends_with_space) {
        let first = parts[0];
        let partial = if parts.len() > 1 && !ends_with_space { parts[1] } else { "" };
        let subs = get_subcommands_for(first);
        for sub in subs {
            if sub.starts_with(&partial.to_lowercase()) && (partial.is_empty() || sub != partial) {
                candidates.push(format!("{} {}", first, sub));
            }
        }
    }

    // ghost-text: 첫 후보의 나머지 부분
    if let Some(top) = candidates.first() {
        if let Some(rest) = top.strip_prefix(input.as_str()) {
            if !rest.is_empty() {
                app.ghost_suffix = Some(rest.to_string());
            }
        }
    }

    app.autocomplete_candidates = candidates;
    app.autocomplete_selected = 0;
    app.autocomplete_visible = !app.autocomplete_candidates.is_empty();
}

fn refresh_slash(app: &mut App, buf: String) {
    app.autocomplete_candidates.clear();
    app.autocomplete_visible = false;
    app.ghost_suffix = None;

    let trimmed = buf.trim();
    if trimmed.is_empty() {
        return;
    }

    let parts: Vec<&str> = trimmed.split_whitespace().collect();
    let ends_with_space = buf.ends_with(' ');

    let mut candidates: Vec<String> = Vec::new();
    if parts.len() == 1 && !ends_with_space {
        let prefix = parts[0].to_lowercase();
        for cmd in SLASH_COMMANDS {
            if cmd.starts_with(&prefix) && *cmd != parts[0] {
                candidates.push((*cmd).to_string());
            }
        }
    } else if parts[0] == "tab" && parts.len() <= 2 {
        let partial = if parts.len() == 2 && !ends_with_space { parts[1] } else { "" };
        for sub in ["new", "close", "list"] {
            if sub.starts_with(&partial.to_lowercase()) && sub != partial {
                candidates.push(format!("tab {}", sub));
            }
        }
    } else if parts[0] == "tab" && parts.get(1).copied() == Some("new") && parts.len() <= 3 {
        let partial = if parts.len() == 3 && !ends_with_space { parts[2] } else { "" };
        for kind in ["daemon"] {
            if kind.starts_with(&partial.to_lowercase()) {
                candidates.push(format!("tab new {}", kind));
            }
        }
    }

    if let Some(top) = candidates.first() {
        if let Some(rest) = top.strip_prefix(trimmed) {
            if !rest.is_empty() {
                app.ghost_suffix = Some(rest.to_string());
            }
        }
    }

    app.autocomplete_candidates = candidates;
    app.autocomplete_selected = 0;
    app.autocomplete_visible = !app.autocomplete_candidates.is_empty();
}

/// Tab 키 — ghost_suffix가 있으면 수락, 없으면 기존 팝업 동작.
/// 반환값: ghost를 수락했으면 true.
pub fn accept_ghost(app: &mut App) -> bool {
    if let Some(ref rest) = app.ghost_suffix.clone() {
        if app.slash_buffer.is_some() {
            if let Some(buf) = app.slash_buffer.as_mut() {
                buf.push_str(rest);
                app.slash_cursor = buf.chars().count();
            }
        } else {
            app.input.push_str(rest);
            app.cursor = app.input.chars().count();
        }
        app.ghost_suffix = None;
        app.autocomplete_candidates.clear();
        app.autocomplete_visible = false;
        return true;
    }
    false
}

fn get_subcommands_for(cmd: &str) -> Vec<&'static str> {
    match cmd {
        "instance" => vec![
            "list", "create", "delete", "start", "stop", "restart",
            "status", "set", "reset", "reorder", "exec",
        ],
        "module" => vec![
            "list", "refresh", "versions", "install", "info", "set", "remove", "update",
        ],
        "extension" => vec![
            "list", "install", "remove", "update", "enable", "disable", "status", "refresh",
        ],
        "daemon" => vec!["start", "stop", "status", "restart"],
        "config" => vec!["show", "set", "get", "reset", "gui", "system-language"],
        "update" => vec!["check", "status", "download", "apply", "config", "set", "install", "launch-apply"],
        "migration" => vec!["scan"],
        _ => vec![],
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::client::DaemonClient;
    use crate::tui::app::App;

    fn fresh_app() -> App {
        App::new(DaemonClient::new(None).unwrap())
    }

    #[test]
    fn empty_input_clears_ghost() {
        let mut app = fresh_app();
        app.input = String::new();
        app.cursor = 0;
        refresh(&mut app);
        assert!(app.ghost_suffix.is_none());
        assert!(!app.autocomplete_visible);
    }

    #[test]
    fn single_letter_produces_ghost_suffix() {
        let mut app = fresh_app();
        app.input = "ins".into();
        app.cursor = 3;
        refresh(&mut app);
        // "instance"가 후보에 있어야 함
        assert!(app.ghost_suffix.is_some(), "ghost_suffix should be set for prefix 'ins'");
        let g = app.ghost_suffix.as_deref().unwrap();
        assert!(g == "tance" || g.starts_with("tance"), "got {:?}", g);
    }

    #[test]
    fn accept_ghost_appends_suffix() {
        let mut app = fresh_app();
        app.input = "ins".into();
        app.cursor = 3;
        refresh(&mut app);
        let accepted = accept_ghost(&mut app);
        assert!(accepted);
        assert_eq!(app.input, "instance");
    }

    #[test]
    fn subcommand_completion() {
        let mut app = fresh_app();
        app.input = "instance l".into();
        app.cursor = app.input.chars().count();
        refresh(&mut app);
        assert!(app.autocomplete_visible);
        assert!(app.autocomplete_candidates.iter().any(|c| c == "instance list"));
    }

    #[test]
    fn slash_tab_completion() {
        let mut app = fresh_app();
        app.slash_buffer = Some("t".into());
        refresh(&mut app);
        assert!(app.autocomplete_visible);
        assert!(app.autocomplete_candidates.iter().any(|c| c == "tab"));
    }
}
