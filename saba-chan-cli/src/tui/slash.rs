//! 슬래시 명령 (`/...`) — 화면 독립 메타 명령.
//!
//! `:`로 시작하는 레거시 명령과 분리된, 어디서든 쓸 수 있는 전역 명령.
//! `/`를 누르면 `App.slash_buffer`가 활성화되고, Enter로 실행한다.
//!
//! 지원 명령:
//! - `/help [명령]`        — 도움말
//! - `/clear`              — 활성 탭 로그 비우기
//! - `/status`             — 상태 요약
//! - `/quit`               — 종료
//! - `/theme <이름>`        — 테마 전환 (Stage 5에서 실제 적용)
//! - `/tab new <종류>`      — 새 탭 (daemon 등)
//! - `/tab close`          — 활성 탭 닫기
//! - `/tab list`           — 탭 목록
//! - `/find <패턴>`         — 검색 오버레이
//! - `/run <:명령>`         — 레거시 명령 실행 브리지

use super::app::{App, InputMode, Out};

#[derive(Clone, Debug, PartialEq)]
#[allow(dead_code)]
pub enum SlashCmd {
    Help(Option<String>),
    Clear,
    Status,
    Quit,
    Theme(String),
    TabNew(String),
    TabClose,
    TabList,
    Find(String),
    Run(String),
    Unknown(String),
}

/// 파서 — 슬래시 제외 본문(`help topic`)을 받아 명령으로 해석.
pub fn parse(input: &str) -> SlashCmd {
    let trimmed = input.trim();
    if trimmed.is_empty() {
        return SlashCmd::Unknown(String::new());
    }
    let mut parts = trimmed.splitn(2, char::is_whitespace);
    let head = parts.next().unwrap_or("").to_lowercase();
    let rest = parts.next().unwrap_or("").trim();

    match head.as_str() {
        "help" | "h" | "?" => {
            if rest.is_empty() {
                SlashCmd::Help(None)
            } else {
                SlashCmd::Help(Some(rest.to_string()))
            }
        }
        "clear" | "cls" => SlashCmd::Clear,
        "status" | "stat" => SlashCmd::Status,
        "quit" | "q" | "exit" => SlashCmd::Quit,
        "theme" => SlashCmd::Theme(rest.to_string()),
        "tab" => {
            let mut sub = rest.splitn(2, char::is_whitespace);
            let op = sub.next().unwrap_or("").to_lowercase();
            let arg = sub.next().unwrap_or("").trim();
            match op.as_str() {
                "new" | "open" => SlashCmd::TabNew(arg.to_string()),
                "close" | "x" => SlashCmd::TabClose,
                "list" | "ls" => SlashCmd::TabList,
                _ => SlashCmd::Unknown(format!("tab {}", op)),
            }
        }
        "find" | "search" | "f" => SlashCmd::Find(rest.to_string()),
        "run" | "cmd" => SlashCmd::Run(rest.to_string()),
        other => SlashCmd::Unknown(other.to_string()),
    }
}

/// 실행기 — 파싱된 명령을 `App` 상태에 적용.
pub fn execute(app: &mut App, cmd: SlashCmd) {
    match cmd {
        SlashCmd::Help(topic) => {
            let lines: Vec<&str> = if let Some(t) = topic.as_deref() {
                match t {
                    "tab" => vec![
                        "/tab new <종류>  — 새 탭 (daemon)",
                        "/tab close       — 활성 탭 닫기",
                        "/tab list        — 탭 목록",
                    ],
                    "theme" => vec!["/theme <이름>  — dark | light"],
                    "find" => vec!["/find <패턴>   — 활성 탭 필터 (Ctrl+F과 동일)"],
                    _ => vec!["지원 명령: /help /clear /status /quit /theme /tab /find /run"],
                }
            } else {
                vec![
                    "── 슬래시 명령 ──",
                    "/help [명령]   도움말",
                    "/clear         활성 탭 로그 비우기",
                    "/status        상태 요약",
                    "/quit          종료",
                    "/theme <이름>   테마 전환",
                    "/tab new|close|list",
                    "/find <패턴>    검색",
                    "/run <:명령>    레거시 명령 브리지",
                ]
            };
            for l in lines {
                app.output.push(Out::Info(l.into()));
            }
        }
        SlashCmd::Clear => {
            if let Some(tab) = app.tabs.active_tab_mut() {
                tab.buffer.clear();
                tab.scroll = 0;
            } else {
                app.output.clear();
            }
        }
        SlashCmd::Status => {
            let d = if app.daemon_on { "● ON" } else { "○ OFF" };
            let srv = app.servers.len();
            let running = app.servers.iter().filter(|s| s.status == "running").count();
            app.output.push(Out::Info(format!(
                "Daemon: {}  Servers: {}/{} running", d, running, srv
            )));
        }
        SlashCmd::Quit => {
            app.quit = true;
        }
        SlashCmd::Theme(name) => {
            let name = name.trim().to_lowercase();
            if name.is_empty() {
                app.output.push(Out::Info(app.i18n.t_with("misc.slash_theme_info", &[("current", super::theme::Theme::mode_name())])));
            } else if super::theme::Theme::set_mode(&name) {
                if let Some(s) = app.session.as_mut() {
                    s.theme = name.clone();
                }
                let msg = app.i18n.t_with("flash.theme_switched", &[("name", &name)]);
                app.flash(&msg);
            } else {
                app.output.push(Out::Err(format!("✗ {}: {}", app.i18n.t("errors.unknown_theme"), name)));
            }
        }
        SlashCmd::TabNew(kind) => {
            let kind = kind.trim().to_lowercase();
            if !app.daemon_on && kind == "daemon" {
                let msg = app.i18n.t("flash.daemon_offline_warning");
                app.flash(&msg);
                return;
            }
            match kind.as_str() {
                "daemon" | "d" => {
                    let tid = app.tabs.open(
                        super::tabs::TabSource::Daemon,
                        app.client.clone(),
                        app.tab_tx.clone(),
                    );
                    app.focus = super::app::Focus::Tab(tid);
                    app.push_screen(super::app::Screen::ServerConsole {
                        name: app.i18n.t("misc.daemon_log"),
                        id: String::new(),
                    });
                    app.input_mode = InputMode::Console;
                }
                _ => {
                    app.output.push(Out::Err(format!("✗ {}: {}", app.i18n.t("errors.unknown_tab_kind"), kind)));
                }
            }
        }
        SlashCmd::TabClose => {
            app.tabs.close_active();
            if app.tabs.active.is_none() {
                app.input_mode = InputMode::Normal;
                app.pop_screen();
            }
        }
        SlashCmd::TabList => {
            if app.tabs.tabs.is_empty() {
                app.output.push(Out::Info("(열린 탭 없음)".into()));
            } else {
                for t in &app.tabs.tabs {
                    let marker = if Some(t.id) == app.tabs.active { "●" } else { " " };
                    let d = if t.dirty { "*" } else { " " };
                    app.output.push(Out::Info(format!(
                        " {} {} [{}] {}", marker, d, t.id, t.title
                    )));
                }
            }
        }
        SlashCmd::Find(q) => {
            if q.is_empty() {
                app.overlay = Some(super::overlay::Overlay::Search(
                    super::overlay::SearchOverlay::new(),
                ));
            } else if let Some(tab) = app.tabs.active_tab_mut() {
                tab.filter = Some(q);
            }
        }
        SlashCmd::Run(cmdline) => {
            // `:명령` 브리지 — 기존 command 실행 경로에 주입
            app.input = cmdline;
            app.cursor = app.input.chars().count();
            super::commands::submit(app);
        }
        SlashCmd::Unknown(s) => {
            app.output.push(Out::Err(format!("✗ 알 수 없는 명령: /{}", s)));
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_help_bare() {
        assert_eq!(parse("help"), SlashCmd::Help(None));
        assert_eq!(parse("h"), SlashCmd::Help(None));
        assert_eq!(parse("?"), SlashCmd::Help(None));
    }

    #[test]
    fn parse_help_with_topic() {
        assert_eq!(parse("help tab"), SlashCmd::Help(Some("tab".into())));
    }

    #[test]
    fn parse_tab_subcommands() {
        assert_eq!(parse("tab new daemon"), SlashCmd::TabNew("daemon".into()));
        assert_eq!(parse("tab close"), SlashCmd::TabClose);
        assert_eq!(parse("tab list"), SlashCmd::TabList);
    }

    #[test]
    fn parse_find() {
        assert_eq!(parse("find error"), SlashCmd::Find("error".into()));
        assert_eq!(parse("f warn"), SlashCmd::Find("warn".into()));
    }

    #[test]
    fn parse_run_passes_through() {
        assert_eq!(parse("run instance list"), SlashCmd::Run("instance list".into()));
    }

    #[test]
    fn parse_unknown_falls_through() {
        matches!(parse("foobar"), SlashCmd::Unknown(_));
    }

    #[test]
    fn parse_theme() {
        assert_eq!(parse("theme dark"), SlashCmd::Theme("dark".into()));
    }
}
