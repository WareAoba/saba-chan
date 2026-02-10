//! saba-chan TUI — ratatui 기반 반응형 터미널 인터페이스
//!
//! 모든 명령은 백그라운드 태스크로 비동기 실행되어 UI가 절대 블로킹되지 않습니다.
//! 시작 시 GUI처럼 데몬과 봇을 자동으로 기동합니다.

use std::io;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use crossterm::{
    event::{self, Event, KeyCode, KeyEventKind, KeyModifiers, EnableMouseCapture, DisableMouseCapture},
    execute,
    terminal::{disable_raw_mode, enable_raw_mode, EnterAlternateScreen, LeaveAlternateScreen},
};
use ratatui::{
    prelude::*,
    widgets::{Block, BorderType, Borders, Padding, Paragraph, Scrollbar, ScrollbarOrientation, ScrollbarState},
};
use tokio::sync::mpsc;

use crate::cli_config::CliSettings;
use crate::client::DaemonClient;
use crate::gui_config;
use crate::i18n::I18n;
use crate::module_registry::ModuleRegistry;
use crate::process;

// ═══════════════════════════════════════════════════════
// Types
// ═══════════════════════════════════════════════════════

/// 출력 영역의 한 줄
#[derive(Clone)]
enum Out {
    Info(String),
    Cmd(String),
    Ok(String),
    Err(String),
    Text(String),
    Blank,
}

/// 서버 상태 (백그라운드 갱신용)
#[derive(Clone)]
#[allow(dead_code)]
struct ServerInfo {
    name: String,
    module: String,
    status: String,
}

/// 백그라운드 스냅샷
struct Snapshot {
    daemon: bool,
    bot: bool,
    token: bool,
    prefix: String,
    servers: Vec<ServerInfo>,
}

/// 비동기 결과를 모으는 공유 버퍼
type OutputBuf = Arc<Mutex<Vec<Out>>>;

fn push_out(buf: &OutputBuf, lines: Vec<Out>) {
    let mut b = buf.lock().unwrap();
    b.extend(lines);
    b.push(Out::Blank);
}

/// char 인덱스 → 바이트 오프셋 변환 (다국어 안전)
fn char_to_byte(s: &str, char_idx: usize) -> usize {
    s.char_indices()
        .nth(char_idx)
        .map(|(byte_pos, _)| byte_pos)
        .unwrap_or(s.len())
}

// ═══════════════════════════════════════════════════════
// App State
// ═══════════════════════════════════════════════════════

struct App {
    client: DaemonClient,
    // 상태
    daemon_on: bool,
    bot_on: bool,
    token_ok: bool,
    bot_prefix: String,
    servers: Vec<ServerInfo>,
    // 모듈 레지스트리
    registry: Arc<ModuleRegistry>,
    // 설정 & i18n
    settings: CliSettings,
    i18n: Arc<I18n>,
    // UI
    input: String,
    cursor: usize,
    output: Vec<Out>,
    history: Vec<String>,
    hist_idx: Option<usize>,
    scroll_up: usize,
    output_height: usize,
    quit: bool,
    // 비동기 결과 수신용
    async_out: OutputBuf,
}

impl App {
    fn new(client: DaemonClient) -> Self {
        // CLI 설정 로드
        let settings = CliSettings::load();

        // i18n 로드 (CLI 설정 → GUI 설정 → "en" 폴백)
        let lang = settings.effective_language();
        let i18n = Arc::new(I18n::load(&lang));

        // 시작 시 토큰/프리픽스를 즉시 읽어서 "NO TOKEN" 깜빡임 방지
        let token_ok = gui_config::get_discord_token()
            .ok()
            .flatten()
            .is_some();
        let bot_prefix = if settings.bot_prefix.is_empty() {
            gui_config::get_bot_prefix().unwrap_or_else(|_| "!saba".into())
        } else {
            settings.bot_prefix.clone()
        };

        // 모듈 레지스트리 로드 (module.toml 읽기)
        let modules_path = gui_config::get_modules_path().unwrap_or_default();
        let registry = Arc::new(ModuleRegistry::load(&modules_path));

        let welcome = i18n.t("welcome");

        Self {
            client,
            daemon_on: false,
            bot_on: false,
            token_ok,
            bot_prefix,
            servers: vec![],
            registry,
            settings,
            i18n,
            input: String::new(),
            cursor: 0,
            output: vec![
                Out::Info(welcome),
                Out::Blank,
            ],
            history: vec![],
            hist_idx: None,
            scroll_up: 0,
            output_height: 20,
            quit: false,
            async_out: Arc::new(Mutex::new(Vec::new())),
        }
    }

    fn apply_status(&mut self, snap: Snapshot) {
        self.daemon_on = snap.daemon;
        self.bot_on = snap.bot;
        self.token_ok = snap.token;
        self.bot_prefix = snap.prefix;
        self.servers = snap.servers;
    }

    /// 비동기 태스크 결과를 output에 플러시
    fn flush_async(&mut self) {
        let drained = {
            let mut buf = self.async_out.lock().unwrap();
            if buf.is_empty() {
                return;
            }
            buf.drain(..).collect::<Vec<_>>()
        };
        let cmd_start = self.output.len().saturating_sub(1);
        self.output.extend(drained);
        self.smart_scroll(cmd_start);
    }

    /// 명령 출력 후 스마트 스크롤: 출력이 뷰포트보다 크면 명령행을 맨 위로
    fn smart_scroll(&mut self, cmd_start: usize) {
        let total = self.output.len();
        let visible = self.output_height;
        if visible == 0 || total <= visible {
            self.scroll_up = 0;
            return;
        }
        let added = total.saturating_sub(cmd_start);
        if added <= visible {
            self.scroll_up = 0;
        } else {
            self.scroll_up = added.saturating_sub(visible);
        }
    }

    // ─── Key handling ───────────────────────────────

    fn on_key(&mut self, key: event::KeyEvent) {
        match key.code {
            KeyCode::Char('c') if key.modifiers.contains(KeyModifiers::CONTROL) => {
                // Ctrl+C: 데몬/봇 종료 후 퇴장
                self.output.push(Out::Info("Shutting down...".into()));
                let buf = self.async_out.clone();
                tokio::spawn(async move {
                    let mut lines = Vec::new();
                    if process::check_bot_running() {
                        match tokio::task::spawn_blocking(process::stop_bot).await {
                            Ok(Ok(msg)) => lines.push(Out::Ok(msg)),
                            _ => {}
                        }
                    }
                    if process::check_daemon_running() {
                        match tokio::task::spawn_blocking(process::stop_daemon).await {
                            Ok(Ok(msg)) => lines.push(Out::Ok(msg)),
                            _ => {}
                        }
                    }
                    push_out(&buf, lines);
                });
                self.quit = true;
            }
            KeyCode::Char('d') if key.modifiers.contains(KeyModifiers::CONTROL) => {
                self.output.push(Out::Info("Shutting down...".into()));
                let buf = self.async_out.clone();
                tokio::spawn(async move {
                    let mut lines = Vec::new();
                    if process::check_bot_running() {
                        match tokio::task::spawn_blocking(process::stop_bot).await {
                            Ok(Ok(msg)) => lines.push(Out::Ok(msg)),
                            _ => {}
                        }
                    }
                    if process::check_daemon_running() {
                        match tokio::task::spawn_blocking(process::stop_daemon).await {
                            Ok(Ok(msg)) => lines.push(Out::Ok(msg)),
                            _ => {}
                        }
                    }
                    push_out(&buf, lines);
                });
                self.quit = true;
            }
            KeyCode::Enter => self.submit(),
            KeyCode::Tab => self.autocomplete(),
            KeyCode::Char(c) => {
                let byte_pos = char_to_byte(&self.input, self.cursor);
                self.input.insert(byte_pos, c);
                self.cursor += 1;
                self.hist_idx = None;
            }
            KeyCode::Backspace => {
                if self.cursor > 0 {
                    self.cursor -= 1;
                    let byte_pos = char_to_byte(&self.input, self.cursor);
                    self.input.remove(byte_pos);
                }
            }
            KeyCode::Delete => {
                let char_count = self.input.chars().count();
                if self.cursor < char_count {
                    let byte_pos = char_to_byte(&self.input, self.cursor);
                    self.input.remove(byte_pos);
                }
            }
            KeyCode::Left => self.cursor = self.cursor.saturating_sub(1),
            KeyCode::Right => {
                let char_count = self.input.chars().count();
                self.cursor = (self.cursor + 1).min(char_count);
            }
            KeyCode::Home if key.modifiers.contains(KeyModifiers::CONTROL) => {
                // Ctrl+Home → 출력 맨 위로
                self.scroll_up = usize::MAX;
            }
            KeyCode::End if key.modifiers.contains(KeyModifiers::CONTROL) => {
                // Ctrl+End → 출력 맨 아래로
                self.scroll_up = 0;
            }
            KeyCode::Home => self.cursor = 0,
            KeyCode::End => self.cursor = self.input.chars().count(),
            KeyCode::Up => self.history_prev(),
            KeyCode::Down => self.history_next(),
            KeyCode::PageUp => self.scroll_up = self.scroll_up.saturating_add(10),
            KeyCode::PageDown => self.scroll_up = self.scroll_up.saturating_sub(10),
            _ => {}
        }
    }

    fn history_prev(&mut self) {
        if self.history.is_empty() {
            return;
        }
        let idx = match self.hist_idx {
            Some(i) => i.saturating_sub(1),
            None => self.history.len() - 1,
        };
        self.hist_idx = Some(idx);
        self.input = self.history[idx].clone();
        self.cursor = self.input.chars().count();
    }

    fn history_next(&mut self) {
        if let Some(idx) = self.hist_idx {
            if idx + 1 < self.history.len() {
                self.hist_idx = Some(idx + 1);
                self.input = self.history[idx + 1].clone();
                self.cursor = self.input.chars().count();
            } else {
                self.hist_idx = None;
                self.input.clear();
                self.cursor = 0;
            }
        }
    }

    fn autocomplete(&mut self) {
        let input = self.input.trim().to_string();
        if input.is_empty() {
            return;
        }

        let parts: Vec<&str> = input.split_whitespace().collect();

        if parts.len() == 1 && !self.input.ends_with(' ') {
            // 첫 단어 완성 — 내장 명령어 + 모듈 이름
            let mut candidates: Vec<String> = vec![
                "server".into(), "module".into(), "daemon".into(), "bot".into(),
                "exec".into(), "config".into(), "help".into(), "exit".into(),
            ];
            for name in self.registry.module_names() {
                candidates.push(name.to_string());
            }
            let matches: Vec<&String> = candidates
                .iter()
                .filter(|c| c.to_lowercase().starts_with(&parts[0].to_lowercase()))
                .collect();
            if matches.len() == 1 {
                self.input = format!("{} ", matches[0]);
                self.cursor = self.input.chars().count();
            }
        } else if parts.len() <= 2 {
            // 두 번째 단어 — 모듈이면 모듈 명령어, 아니면 내장 서브커맨드
            let first = parts[0];
            let partial = if parts.len() > 1 && !self.input.ends_with(' ') {
                parts[1]
            } else {
                ""
            };

            if let Some(module_name) = self.registry.resolve_module_name(first) {
                // 모듈 명령어 후보: lifecycle + module-specific
                let mut sub: Vec<String> = vec![
                    "start".into(), "stop".into(), "restart".into(), "status".into(),
                ];
                if let Some(mi) = self.registry.get_module(&module_name) {
                    for cmd in &mi.commands {
                        sub.push(cmd.name.clone());
                    }
                }
                let matches: Vec<&String> = sub
                    .iter()
                    .filter(|c| c.starts_with(partial))
                    .collect();
                if matches.len() == 1 {
                    self.input = format!("{} {} ", first, matches[0]);
                    self.cursor = self.input.chars().count();
                }
            } else {
                // 내장 서브커맨드 (기존 로직)
                let full_cmds = [
                    "server list", "server start", "server stop", "server restart", "server status",
                    "module list", "module reload",
                    "daemon start", "daemon stop",
                    "bot start", "bot stop", "bot token", "bot prefix",
                    "config show", "config set", "config get", "config reset",
                ];
                let prefix = self.input.trim();
                let matches: Vec<&&str> = full_cmds.iter().filter(|c| c.starts_with(prefix)).collect();
                if matches.len() == 1 {
                    self.input = format!("{} ", matches[0]);
                    self.cursor = self.input.chars().count();
                }
            }
        }
    }

    fn submit(&mut self) {
        let cmd = self.input.trim().to_string();
        self.input.clear();
        self.cursor = 0;
        if cmd.is_empty() {
            return;
        }

        self.history.push(cmd.clone());
        self.hist_idx = None;

        if matches!(cmd.to_lowercase().as_str(), "exit" | "quit" | "q") {
            self.output.push(Out::Cmd(cmd.clone()));
            self.output.push(Out::Info("Shutting down...".into()));

            // 데몬·봇 종료를 백그라운드로 발사
            let buf = self.async_out.clone();
            tokio::spawn(async move {
                let mut lines = Vec::new();
                if process::check_bot_running() {
                    match tokio::task::spawn_blocking(process::stop_bot).await {
                        Ok(Ok(msg)) => lines.push(Out::Ok(msg)),
                        Ok(Err(e)) => lines.push(Out::Err(format!("Bot stop failed: {}", e))),
                        Err(e) => lines.push(Out::Err(format!("Bot stop failed: {}", e))),
                    }
                }
                if process::check_daemon_running() {
                    match tokio::task::spawn_blocking(process::stop_daemon).await {
                        Ok(Ok(msg)) => lines.push(Out::Ok(msg)),
                        Ok(Err(e)) => lines.push(Out::Err(format!("Daemon stop failed: {}", e))),
                        Err(e) => lines.push(Out::Err(format!("Daemon stop failed: {}", e))),
                    }
                }
                if lines.is_empty() {
                    lines.push(Out::Info("Nothing to stop.".into()));
                }
                push_out(&buf, lines);
            });

            self.quit = true;
            return;
        }

        let cmd_start = self.output.len();
        self.output.push(Out::Cmd(cmd.clone()));

        // 원본 파트 (값 보존용) + 소문자 파트 (명령어 매칭용)
        let orig_parts: Vec<&str> = cmd.split_whitespace().collect();
        let lower_cmd = cmd.to_lowercase();
        let lower_parts: Vec<&str> = lower_cmd.split_whitespace().collect();

        // 동기 명령 시도 — 명령 키워드는 lower, 값은 orig 사용
        if let Some(lines) = self.dispatch_sync(&lower_parts, &orig_parts) {
            self.output.extend(lines);
            self.output.push(Out::Blank);
            self.smart_scroll(cmd_start);
            return;
        }

        // 비동기 명령 — 백그라운드 태스크로 발사 (UI 블로킹 없음)
        let client = self.client.clone();
        let buf = self.async_out.clone();
        let registry = self.registry.clone();
        let lower_owned = lower_cmd.clone();
        let orig_owned = cmd.clone();

        tokio::spawn(async move {
            let lower_parts: Vec<&str> = lower_owned.split_whitespace().collect();
            let orig_parts: Vec<&str> = orig_owned.split_whitespace().collect();
            let lines = match lower_parts.first().copied() {
                Some("server") => exec_server(&client, &lower_parts[1..]).await,
                Some("module") => exec_module(&client, &lower_parts[1..]).await,
                Some("daemon") => exec_daemon(&lower_parts[1..]).await,
                Some("bot") => exec_bot(&lower_parts[1..]).await,
                Some("exec") => exec_exec(&client, &orig_parts[1..]).await,
                Some(word) => {
                    // 모듈 별명 매칭 시도 (레지스트리가 이미 대소문자 무시)
                    if let Some(module_name) = registry.resolve_module_name(word) {
                        exec_module_cmd(&client, &registry, &module_name, &lower_parts[1..]).await
                    } else {
                        vec![Out::Err(format!("Unknown command '{}'. Type 'help'.", word))]
                    }
                }
                None => vec![],
            };
            push_out(&buf, lines);
        });

        self.scroll_up = 0;
    }

    /// 동기 명령 디스패치 — 즉시 결과를 반환; None이면 비동기로 넘어감
    /// `lower`: 소문자 매칭용, `orig`: 원본 값 보존용
    fn dispatch_sync(&mut self, lower: &[&str], orig: &[&str]) -> Option<Vec<Out>> {
        match lower.first().copied() {
            Some("config") => Some(self.cmd_config(&orig[1..])),
            Some("help") => Some(self.cmd_help()),
            Some("server") if lower.len() == 1 => Some(vec![
                Out::Text("  server list              list all servers".into()),
                Out::Text("  server status <name>     show server status".into()),
                Out::Text("  server start <name>      start a server".into()),
                Out::Text("  server stop <name>       stop a server".into()),
                Out::Text("  server restart <name>    restart a server".into()),
            ]),
            Some("module") if lower.len() == 1 => Some(vec![
                Out::Text("  module list              list loaded modules".into()),
                Out::Text("  module reload            reload all modules".into()),
            ]),
            Some("daemon") if lower.len() == 1 => Some(vec![
                Out::Text("  daemon start             start core daemon".into()),
                Out::Text("  daemon stop              stop core daemon".into()),
            ]),
            Some("bot") if lower.len() == 1 => Some(vec![
                Out::Text("  bot start                start Discord bot".into()),
                Out::Text("  bot stop                 stop Discord bot".into()),
                Out::Text("  bot token [show|set|clear]  manage Discord token".into()),
                Out::Text("  bot prefix [show|set]    manage bot prefix".into()),
            ]),
            Some("bot") if lower.len() >= 2 && lower[1] == "token" => {
                Some(self.cmd_bot_token(&orig[2..]))
            }
            Some("bot") if lower.len() >= 2 && lower[1] == "prefix" => {
                Some(self.cmd_bot_prefix(&orig[2..]))
            }
            Some("exec") if lower.len() < 4 => Some(vec![
                Out::Text("  exec <id> cmd <command>  execute command".into()),
                Out::Text("  exec <id> rcon <cmd>     execute RCON".into()),
                Out::Text("  exec <id> rest <cmd>     execute REST".into()),
            ]),
            _ => {
                // 모듈 이름만 입력 → 사용 가능한 명령어 표시
                if lower.len() == 1 {
                    if let Some(module_name) = self.registry.resolve_module_name(lower[0]) {
                        return Some(show_module_commands(&self.registry, &module_name));
                    }
                }
                None
            }
        }
    }

    fn cmd_config(&mut self, args: &[&str]) -> Vec<Out> {
        match args.first().copied() {
            None | Some("show") => {
                // CLI 설정 + GUI 설정 통합 표시
                let token = gui_config::get_discord_token().ok().flatten();
                let modules = gui_config::get_modules_path().unwrap_or_default();
                let prefix = gui_config::get_bot_prefix().unwrap_or_else(|_| "!saba".into());
                let gui_lang = gui_config::get_language().unwrap_or_else(|_| "en".into());
                let mut lines = vec![
                    Out::Info("CLI Settings:".into()),
                    Out::Text(format!("  language         {}", self.settings.get_value("language").unwrap_or_else(|| "(auto)".into()))),
                    Out::Text(format!("  auto_start       {}", self.settings.auto_start)),
                    Out::Text(format!("  refresh_interval {}", self.settings.refresh_interval)),
                    Out::Text(format!("  bot_prefix       {}", self.settings.bot_prefix)),
                    Out::Blank,
                    Out::Info("GUI Settings:".into()),
                    Out::Text(format!(
                        "  token            {}",
                        if token.is_some() { "✓ set" } else { "✗ not set" }
                    )),
                    Out::Text(format!("  prefix           {}", prefix)),
                    Out::Text(format!("  modules_path     {}", modules)),
                    Out::Text(format!("  language         {}", gui_lang)),
                ];
                lines.push(Out::Blank);
                lines.push(Out::Text("  config set <key> <value>   change a CLI setting".into()));
                lines.push(Out::Text("  config get <key>           show one setting".into()));
                lines.push(Out::Text("  config reset <key>         reset to default".into()));
                lines.push(Out::Text(format!("  keys: {}", CliSettings::available_keys().iter().map(|(k,_)| *k).collect::<Vec<_>>().join(", "))));
                lines
            }
            Some("get") => {
                if args.len() < 2 {
                    return vec![Out::Err("Usage: config get <key>".into())];
                }
                let key = args[1];
                match self.settings.get_value(key) {
                    Some(v) => vec![Out::Ok(format!("{} = {}", key, v))],
                    None => vec![Out::Err(format!("Unknown key: {}", key))],
                }
            }
            Some("set") => {
                if args.len() < 3 {
                    return vec![Out::Err("Usage: config set <key> <value>".into())];
                }
                let key = args[1];
                let val = args[2..].join(" ");
                match self.settings.set_value(key, &val) {
                    Ok(()) => {
                        if let Err(e) = self.settings.save() {
                            vec![Out::Err(format!("Set ok but save failed: {}", e))]
                        } else {
                            vec![Out::Ok(format!("{} = {}", key, val))]
                        }
                    }
                    Err(e) => vec![Out::Err(format!("{}", e))],
                }
            }
            Some("reset") => {
                if args.len() < 2 {
                    return vec![Out::Err("Usage: config reset <key>".into())];
                }
                let key = args[1];
                match self.settings.reset_value(key) {
                    Ok(()) => {
                        if let Err(e) = self.settings.save() {
                            vec![Out::Err(format!("Reset ok but save failed: {}", e))]
                        } else {
                            let new_val = self.settings.get_value(key).unwrap_or_default();
                            vec![Out::Ok(format!("{} reset → {}", key, new_val))]
                        }
                    }
                    Err(e) => vec![Out::Err(format!("{}", e))],
                }
            }
            Some(sub) => {
                vec![Out::Err(format!("Unknown config subcommand: {}. Try: show, get, set, reset", sub))]
            }
        }
    }

    fn cmd_bot_token(&self, args: &[&str]) -> Vec<Out> {
        match args.first().copied() {
            None | Some("show") => {
                match gui_config::get_discord_token() {
                    Ok(Some(t)) => {
                        // 토큰 일부만 표시 (보안)
                        let masked = if t.len() > 8 {
                            format!("{}...{}", &t[..4], &t[t.len()-4..])
                        } else {
                            "****".into()
                        };
                        vec![Out::Ok(format!("Token: {}", masked))]
                    }
                    Ok(None) => vec![Out::Text("Token: not set".into())],
                    Err(e) => vec![Out::Err(format!("Error reading token: {}", e))],
                }
            }
            Some("set") => {
                if args.len() < 2 {
                    return vec![Out::Err("Usage: bot token set <TOKEN>".into())];
                }
                let token = args[1];
                match gui_config::set_discord_token(token) {
                    Ok(()) => vec![Out::Ok("Discord token saved.".into())],
                    Err(e) => vec![Out::Err(format!("Failed to save token: {}", e))],
                }
            }
            Some("clear") => {
                match gui_config::clear_discord_token() {
                    Ok(()) => vec![Out::Ok("Discord token cleared.".into())],
                    Err(e) => vec![Out::Err(format!("Failed to clear token: {}", e))],
                }
            }
            Some(sub) => {
                vec![Out::Err(format!("Unknown: bot token {}. Try: show, set, clear", sub))]
            }
        }
    }

    fn cmd_bot_prefix(&self, args: &[&str]) -> Vec<Out> {
        match args.first().copied() {
            None | Some("show") => {
                let prefix = gui_config::get_bot_prefix().unwrap_or_else(|_| "!saba".into());
                vec![Out::Ok(format!("Bot prefix: {}", prefix))]
            }
            Some("set") => {
                if args.len() < 2 {
                    return vec![Out::Err("Usage: bot prefix set <PREFIX>".into())];
                }
                let prefix = args[1];
                match gui_config::set_bot_prefix(prefix) {
                    Ok(()) => vec![Out::Ok(format!("Bot prefix set to: {}", prefix))],
                    Err(e) => vec![Out::Err(format!("Failed: {}", e))],
                }
            }
            Some(sub) => {
                vec![Out::Err(format!("Unknown: bot prefix {}. Try: show, set", sub))]
            }
        }
    }

    fn cmd_help(&self) -> Vec<Out> {
        let mut lines = vec![
            Out::Text("  server  [list|start|stop|restart|status] <name>".into()),
            Out::Text("  module  [list|reload]".into()),
            Out::Text("  daemon  [start|stop]".into()),
            Out::Text("  bot     [start|stop]".into()),
            Out::Text("  bot     token [show|set|clear]".into()),
            Out::Text("  bot     prefix [show|set]".into()),
            Out::Text("  exec    <id> [cmd|rcon|rest] <command>".into()),
            Out::Text("  config  [show|set|get|reset] — CLI/GUI settings".into()),
            Out::Text("  help    — This help".into()),
            Out::Text("  exit    — Quit (Ctrl+C)".into()),
        ];

        // 등록된 모듈 단축키 표시
        if !self.registry.modules.is_empty() {
            lines.push(Out::Blank);
            lines.push(Out::Info("Module shortcuts:".into()));
            for mi in &self.registry.modules {
                lines.push(Out::Text(format!(
                    "  {:<10} {} — type '{}' for commands",
                    mi.name, mi.display_name, mi.name,
                )));
            }
        }

        lines
    }
}

// ═══════════════════════════════════════════════════════
// 비동기 명령 실행 (tokio::spawn에서 호출되는 자유 함수)
// ═══════════════════════════════════════════════════════

async fn exec_server(client: &DaemonClient, args: &[&str]) -> Vec<Out> {
    match args.first().copied() {
        Some("list") => match client.list_servers().await {
            Ok(list) if list.is_empty() => vec![Out::Text("No servers configured.".into())],
            Ok(list) => {
                let mut o = vec![Out::Ok(format!("{} server(s):", list.len()))];
                for s in &list {
                    let st = s["status"].as_str().unwrap_or("?");
                    let sym = if st == "running" { "▶" } else { "■" };
                    o.push(Out::Text(format!(
                        "  {} {} [{}] — {}",
                        sym,
                        s["name"].as_str().unwrap_or("?"),
                        s["module"].as_str().unwrap_or("?"),
                        st,
                    )));
                }
                o
            }
            Err(e) => vec![Out::Err(format!("✗ {}", e))],
        },
        Some("start") if args.len() > 1 => {
            let name = args[1];
            // 인스턴스 목록에서 module_name 조회 (start API에 필요)
            let module = match find_module_for_server(client, name).await {
                Some(m) => m,
                None => return vec![Out::Err(format!("✗ Server '{}' not found", name))],
            };
            match client.start_server(name, &module).await {
                Ok(r) => vec![Out::Ok(format!(
                    "✓ {}",
                    r.get("message").and_then(|v| v.as_str()).unwrap_or("Started")
                ))],
                Err(e) => vec![Out::Err(format!("✗ {}", e))],
            }
        }
        Some("stop") if args.len() > 1 => {
            match client.stop_server(args[1], false).await {
                Ok(r) => vec![Out::Ok(format!(
                    "✓ {}",
                    r.get("message").and_then(|v| v.as_str()).unwrap_or("Stopped")
                ))],
                Err(e) => vec![Out::Err(format!("✗ {}", e))],
            }
        }
        Some("restart") if args.len() > 1 => {
            let name = args[1];
            if let Err(e) = client.stop_server(name, false).await {
                return vec![Out::Err(format!("✗ Stop: {}", e))];
            }
            tokio::time::sleep(Duration::from_secs(1)).await;
            let module = match find_module_for_server(client, name).await {
                Some(m) => m,
                None => return vec![Out::Err(format!("✗ Server '{}' not found", name))],
            };
            match client.start_server(name, &module).await {
                Ok(_) => vec![Out::Ok("✓ Server restarted".into())],
                Err(e) => vec![Out::Err(format!("✗ Start: {}", e))],
            }
        }
        Some("status") if args.len() > 1 => match client.get_server_status(args[1]).await {
            Ok(s) => {
                let status = s["status"].as_str().unwrap_or("unknown");
                let pid_str = match s["pid"].as_u64() {
                    Some(p) => p.to_string(),
                    None => "-".into(),
                };
                vec![Out::Text(format!("{} — {} | PID {}", args[1], status, pid_str))]
            }
            Err(e) => vec![Out::Err(format!("✗ {}", e))],
        },
        _ => vec![Out::Err("Usage: server [list|start|stop|restart|status] <name>".into())],
    }
}

/// 인스턴스 목록에서 해당 이름의 module_name을 찾는 유틸리티
async fn find_module_for_server(client: &DaemonClient, name: &str) -> Option<String> {
    if let Ok(instances) = client.list_instances().await {
        for inst in &instances {
            if inst["name"].as_str() == Some(name) {
                return inst["module_name"].as_str().map(|s| s.to_string());
            }
        }
    }
    None
}

async fn exec_module(client: &DaemonClient, args: &[&str]) -> Vec<Out> {
    match args.first().copied() {
        Some("list") => match client.list_modules().await {
            Ok(list) if list.is_empty() => vec![Out::Text("No modules loaded.".into())],
            Ok(list) => {
                let mut o = vec![Out::Ok(format!("{} module(s):", list.len()))];
                for m in &list {
                    o.push(Out::Text(format!(
                        "  • {} v{}",
                        m["name"].as_str().unwrap_or("?"),
                        m["version"].as_str().unwrap_or("?"),
                    )));
                }
                o
            }
            Err(e) => vec![Out::Err(format!("✗ {}", e))],
        },
        Some("reload") => match client.reload_modules().await {
            Ok(_) => vec![Out::Ok("✓ Modules reloaded".into())],
            Err(e) => vec![Out::Err(format!("✗ {}", e))],
        },
        _ => vec![Out::Err("Usage: module [list|reload]".into())],
    }
}

async fn exec_daemon(args: &[&str]) -> Vec<Out> {
    match args.first().copied() {
        Some("start") => match tokio::task::spawn_blocking(process::start_daemon).await {
            Ok(Ok(msg)) => msg.lines().map(|l| Out::Ok(l.into())).collect(),
            Ok(Err(e)) => vec![Out::Err(format!("✗ {}", e))],
            Err(e) => vec![Out::Err(format!("✗ {}", e))],
        },
        Some("stop") => match tokio::task::spawn_blocking(process::stop_daemon).await {
            Ok(Ok(msg)) => vec![Out::Ok(msg)],
            Ok(Err(e)) => vec![Out::Err(format!("✗ {}", e))],
            Err(e) => vec![Out::Err(format!("✗ {}", e))],
        },
        _ => vec![Out::Err("Usage: daemon [start|stop]".into())],
    }
}

async fn exec_bot(args: &[&str]) -> Vec<Out> {
    match args.first().copied() {
        Some("start") => match tokio::task::spawn_blocking(process::start_bot).await {
            Ok(Ok(msg)) => vec![Out::Ok(msg)],
            Ok(Err(e)) => vec![Out::Err(format!("✗ {}", e))],
            Err(e) => vec![Out::Err(format!("✗ {}", e))],
        },
        Some("stop") => match tokio::task::spawn_blocking(process::stop_bot).await {
            Ok(Ok(msg)) => vec![Out::Ok(msg)],
            Ok(Err(e)) => vec![Out::Err(format!("✗ {}", e))],
            Err(e) => vec![Out::Err(format!("✗ {}", e))],
        },
        _ => vec![Out::Err("Usage: bot [start|stop]".into())],
    }
}

async fn exec_exec(client: &DaemonClient, args: &[&str]) -> Vec<Out> {
    if args.len() < 3 {
        return vec![Out::Err("Usage: exec <id> [cmd|rcon|rest] <command>".into())];
    }
    let (id, mode) = (args[0], args[1]);
    let cmd = args[2..].join(" ");
    let result = match mode {
        "rcon" => client.execute_rcon_command(id, &cmd).await,
        "rest" => client.execute_rest_command(id, &cmd).await,
        _ => client.execute_command(id, &cmd, None).await,
    };
    match result {
        Ok(r) => vec![Out::Ok(
            r.get("message")
                .and_then(|v| v.as_str())
                .unwrap_or("OK")
                .into(),
        )],
        Err(e) => vec![Out::Err(format!("✗ {}", e))],
    }
}

// ═══════════════════════════════════════════════════════
// 모듈 단축 명령어 실행 (`palworld start`, `팰월드 시작` 등)
// ═══════════════════════════════════════════════════════

use crate::module_registry::LIFECYCLE_COMMANDS;

/// 모듈 명령어 실행 메인 디스패처
async fn exec_module_cmd(
    client: &DaemonClient,
    registry: &ModuleRegistry,
    module_name: &str,
    args: &[&str],
) -> Vec<Out> {
    // 명령어가 없으면 도움말 (여기 오면 안 되지만 안전장치)
    if args.is_empty() {
        return show_module_commands(registry, module_name);
    }

    // 명령어 별명 리졸브
    let cmd_input = args[0];
    let cmd_name = match registry.resolve_command(module_name, cmd_input) {
        Some(name) => name,
        None => {
            return vec![Out::Err(format!(
                "✗ Unknown command '{}' for module '{}'. Type '{}' for commands.",
                cmd_input, module_name, module_name,
            ))];
        }
    };

    // 이 모듈에 속하는 인스턴스 찾기
    let instance = match find_instance_for_module(client, module_name).await {
        FindResult::One(inst) => inst,
        FindResult::Multiple(list) => {
            let mut lines = vec![Out::Info(format!(
                "{} has {} instances — using first one:",
                module_name,
                list.len(),
            ))];
            for (i, inst) in list.iter().enumerate() {
                let n = inst["name"].as_str().unwrap_or("?");
                if i == 0 {
                    lines.push(Out::Text(format!("  ▶ {}", n)));
                } else {
                    lines.push(Out::Text(format!("    {}", n)));
                }
            }
            list.into_iter().next().unwrap()
        }
        FindResult::None => {
            return vec![Out::Err(format!(
                "✗ No server instance found for module '{}'. Create one in the GUI first.",
                module_name,
            ))];
        }
    };

    let instance_name = instance["name"].as_str().unwrap_or("?").to_string();
    let instance_id = instance["id"].as_str().unwrap_or("").to_string();

    // 라이프사이클 명령어 → 서버 API
    if LIFECYCLE_COMMANDS.contains(&cmd_name.as_str()) {
        return exec_lifecycle(client, module_name, &instance_name, &cmd_name).await;
    }

    // 모듈 고유 명령어 → 인스턴스 커맨드 API
    let remain = &args[1..];
    let arg_value = if let Some(cmd_def) = registry.get_command_def(module_name, &cmd_name) {
        build_args_map(&cmd_def.inputs, remain)
    } else {
        // 정의 없는 명령어: 인자를 하나의 문자열로 전달
        let mut map = serde_json::Map::new();
        if !remain.is_empty() {
            map.insert(
                "args".to_string(),
                serde_json::Value::String(remain.join(" ")),
            );
        }
        serde_json::Value::Object(map)
    };

    match client
        .execute_command(&instance_id, &cmd_name, Some(arg_value))
        .await
    {
        Ok(r) => {
            let msg = r
                .get("message")
                .or_else(|| r.get("result"))
                .and_then(|v| match v {
                    serde_json::Value::String(s) => Some(s.as_str()),
                    _ => None,
                })
                .unwrap_or("OK");
            vec![Out::Ok(format!("✓ {} — {}", instance_name, msg))]
        }
        Err(e) => vec![Out::Err(format!("✗ {}", e))],
    }
}

/// 라이프사이클 명령어 실행 (start/stop/restart/status)
async fn exec_lifecycle(
    client: &DaemonClient,
    module_name: &str,
    instance_name: &str,
    cmd: &str,
) -> Vec<Out> {
    match cmd {
        "start" => match client.start_server(instance_name, module_name).await {
            Ok(r) => vec![Out::Ok(format!(
                "✓ {} — {}",
                instance_name,
                r.get("message")
                    .and_then(|v| v.as_str())
                    .unwrap_or("Started"),
            ))],
            Err(e) => vec![Out::Err(format!("✗ {}", e))],
        },
        "stop" => match client.stop_server(instance_name, false).await {
            Ok(r) => vec![Out::Ok(format!(
                "✓ {} — {}",
                instance_name,
                r.get("message")
                    .and_then(|v| v.as_str())
                    .unwrap_or("Stopped"),
            ))],
            Err(e) => vec![Out::Err(format!("✗ {}", e))],
        },
        "restart" => {
            if let Err(e) = client.stop_server(instance_name, false).await {
                return vec![Out::Err(format!("✗ Stop: {}", e))];
            }
            tokio::time::sleep(Duration::from_secs(1)).await;
            match client.start_server(instance_name, module_name).await {
                Ok(_) => vec![Out::Ok(format!("✓ {} — Restarted", instance_name))],
                Err(e) => vec![Out::Err(format!("✗ Start: {}", e))],
            }
        }
        "status" => match client.get_server_status(instance_name).await {
            Ok(s) => {
                let status = s["status"].as_str().unwrap_or("unknown");
                let pid_str = match s["pid"].as_u64() {
                    Some(p) => p.to_string(),
                    None => "-".into(),
                };
                vec![Out::Text(format!(
                    "{} — {} | PID {}",
                    instance_name, status, pid_str,
                ))]
            }
            Err(e) => vec![Out::Err(format!("✗ {}", e))],
        },
        _ => vec![Out::Err(format!("Unknown lifecycle command '{}'", cmd))],
    }
}

/// 인스턴스 검색 결과
enum FindResult {
    One(serde_json::Value),
    Multiple(Vec<serde_json::Value>),
    None,
}

/// 모듈 이름으로 인스턴스를 찾기
async fn find_instance_for_module(client: &DaemonClient, module_name: &str) -> FindResult {
    let instances = match client.list_instances().await {
        Ok(list) => list,
        Err(_) => return FindResult::None,
    };

    let matching: Vec<serde_json::Value> = instances
        .into_iter()
        .filter(|inst| inst["module_name"].as_str() == Some(module_name))
        .collect();

    match matching.len() {
        0 => FindResult::None,
        1 => FindResult::One(matching.into_iter().next().unwrap()),
        _ => FindResult::Multiple(matching),
    }
}

/// 명령어 입력 정의에 따라 위치 인자를 JSON 맵으로 변환
fn build_args_map(
    inputs: &[crate::module_registry::CommandInput],
    args: &[&str],
) -> serde_json::Value {
    let mut map = serde_json::Map::new();
    let mut arg_idx = 0;

    for (i, input) in inputs.iter().enumerate() {
        if arg_idx >= args.len() {
            break;
        }

        // 마지막 text 타입 입력은 나머지 인자를 모두 합침
        if i == inputs.len() - 1 && input.input_type == "text" {
            map.insert(
                input.name.clone(),
                serde_json::Value::String(args[arg_idx..].join(" ")),
            );
            break;
        }

        if input.input_type == "number" {
            if let Ok(n) = args[arg_idx].parse::<i64>() {
                map.insert(input.name.clone(), serde_json::json!(n));
            } else {
                map.insert(
                    input.name.clone(),
                    serde_json::Value::String(args[arg_idx].to_string()),
                );
            }
        } else {
            map.insert(
                input.name.clone(),
                serde_json::Value::String(args[arg_idx].to_string()),
            );
        }
        arg_idx += 1;
    }

    serde_json::Value::Object(map)
}

/// 모듈의 사용 가능한 명령어 목록 표시
fn show_module_commands(registry: &ModuleRegistry, module_name: &str) -> Vec<Out> {
    let module = match registry.get_module(module_name) {
        Some(m) => m,
        None => return vec![Out::Err(format!("Module '{}' not found", module_name))],
    };

    let mut lines = vec![Out::Ok(format!(
        "{} ({}) commands:",
        module.display_name, module.name,
    ))];

    // 라이프사이클 명령어 (공통)
    lines.push(Out::Text(format!(
        "  {:<14} 서버 시작",
        "start",
    )));
    lines.push(Out::Text(format!(
        "  {:<14} 서버 종료",
        "stop",
    )));
    lines.push(Out::Text(format!(
        "  {:<14} 서버 재시작",
        "restart",
    )));
    lines.push(Out::Text(format!(
        "  {:<14} 서버 상태",
        "status",
    )));

    // 모듈 고유 명령어
    for cmd in &module.commands {
        let desc = truncate_str(&cmd.description, 35);
        lines.push(Out::Text(format!("  {:<14} {}", cmd.name, desc)));
    }

    lines
}

/// UTF-8 안전한 문자열 자르기 (표시 폭 기준)
fn truncate_str(s: &str, max_chars: usize) -> String {
    let chars: Vec<char> = s.chars().collect();
    if chars.len() <= max_chars {
        s.to_string()
    } else {
        let truncated: String = chars[..max_chars].iter().collect();
        format!("{}…", truncated)
    }
}

// ═══════════════════════════════════════════════════════
// 자동 시작 시퀀스 (GUI와 동일)
// ═══════════════════════════════════════════════════════

async fn auto_start(buf: OutputBuf) {
    // 1단계: 데몬 시작
    buf.lock().unwrap().push(Out::Info("Auto-starting daemon...".into()));
    let daemon_result = tokio::task::spawn_blocking(process::start_daemon).await;
    match daemon_result {
        Ok(Ok(msg)) => {
            let lines: Vec<Out> = msg.lines().map(|l| Out::Ok(l.into())).collect();
            push_out(&buf, lines);
        }
        Ok(Err(e)) => push_out(&buf, vec![Out::Err(format!("✗ Daemon: {}", e))]),
        Err(e) => push_out(&buf, vec![Out::Err(format!("✗ Daemon: {}", e))]),
    }

    // 데몬이 실제로 HTTP 응답할 때까지 대기 (최대 10초)
    let start = std::time::Instant::now();
    loop {
        if start.elapsed() > Duration::from_secs(10) {
            push_out(&buf, vec![Out::Err("✗ Daemon did not respond in 10s".into())]);
            break;
        }
        let running = tokio::task::spawn_blocking(process::check_daemon_running)
            .await
            .unwrap_or(false);
        if running {
            break;
        }
        tokio::time::sleep(Duration::from_millis(500)).await;
    }

    // 2단계: 봇 시작 (토큰이 설정된 경우만)
    let has_token = gui_config::get_discord_token().ok().flatten().is_some();
    if has_token {
        buf.lock().unwrap().push(Out::Info("Auto-starting Discord bot...".into()));
        let bot_result = tokio::task::spawn_blocking(process::start_bot).await;
        match bot_result {
            Ok(Ok(msg)) => push_out(&buf, vec![Out::Ok(msg)]),
            Ok(Err(e)) => push_out(&buf, vec![Out::Err(format!("✗ Bot: {}", e))]),
            Err(e) => push_out(&buf, vec![Out::Err(format!("✗ Bot: {}", e))]),
        }
    } else {
        push_out(&buf, vec![Out::Info("Bot skipped — no Discord token configured.".into())]);
    }
}

// ═══════════════════════════════════════════════════════
// Rendering
// ═══════════════════════════════════════════════════════

fn render(app: &App, frame: &mut Frame) {
    let chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Length(3), // Status bar
            Constraint::Min(5),   // Output
            Constraint::Length(3), // Input
        ])
        .split(frame.area());

    render_status(app, frame, chunks[0]);
    render_output(app, frame, chunks[1]);
    render_input(app, frame, chunks[2]);
}

fn render_status(app: &App, frame: &mut Frame, area: Rect) {
    let d = if app.daemon_on {
        Span::styled(
            "● RUNNING",
            Style::default()
                .fg(Color::Green)
                .add_modifier(Modifier::BOLD),
        )
    } else {
        Span::styled("○ OFFLINE", Style::default().fg(Color::DarkGray))
    };

    let b = if app.bot_on {
        Span::styled(
            "● RUNNING",
            Style::default()
                .fg(Color::Green)
                .add_modifier(Modifier::BOLD),
        )
    } else if !app.token_ok {
        Span::styled("○ NO TOKEN", Style::default().fg(Color::Yellow))
    } else {
        Span::styled("○ OFFLINE", Style::default().fg(Color::DarkGray))
    };

    let running = app
        .servers
        .iter()
        .filter(|s| s.status == "running")
        .count();

    let status_line = Line::from(vec![
        Span::styled("Daemon ", Style::default().fg(Color::Cyan)),
        d,
        Span::raw("   "),
        Span::styled("Bot ", Style::default().fg(Color::Magenta)),
        b,
        Span::raw("   "),
        Span::styled(
            format!("Servers {}/{}", running, app.servers.len()),
            Style::default().fg(Color::Blue),
        ),
    ]);

    let block = Block::default()
        .title(" saba-chan ")
        .title_alignment(Alignment::Center)
        .borders(Borders::ALL)
        .border_type(BorderType::Rounded)
        .border_style(Style::default().fg(Color::DarkGray));

    frame.render_widget(
        Paragraph::new(status_line)
            .block(block)
            .alignment(Alignment::Center),
        area,
    );
}

fn render_output(app: &App, frame: &mut Frame, area: Rect) {
    let block = Block::default()
        .borders(Borders::LEFT | Borders::RIGHT)
        .border_style(Style::default().fg(Color::DarkGray))
        .padding(Padding::horizontal(1));

    let inner = block.inner(area);
    frame.render_widget(block, area);

    let lines: Vec<Line> = app
        .output
        .iter()
        .map(|e| match e {
            Out::Info(s) => Line::from(Span::styled(
                s.clone(),
                Style::default()
                    .fg(Color::DarkGray)
                    .add_modifier(Modifier::ITALIC),
            )),
            Out::Cmd(s) => Line::from(vec![
                Span::styled(
                    "❯ ",
                    Style::default()
                        .fg(Color::Cyan)
                        .add_modifier(Modifier::BOLD),
                ),
                Span::styled(
                    s.clone(),
                    Style::default()
                        .fg(Color::White)
                        .add_modifier(Modifier::BOLD),
                ),
            ]),
            Out::Ok(s) => Line::from(Span::styled(s.clone(), Style::default().fg(Color::Green))),
            Out::Err(s) => Line::from(Span::styled(s.clone(), Style::default().fg(Color::Red))),
            Out::Text(s) => Line::from(s.clone()),
            Out::Blank => Line::from(""),
        })
        .collect();

    let total = lines.len();
    let visible = inner.height as usize;
    let max_up = total.saturating_sub(visible);
    let eff_up = app.scroll_up.min(max_up);
    let y = max_up.saturating_sub(eff_up);

    frame.render_widget(Paragraph::new(lines).scroll((y as u16, 0)), inner);

    // 스크롤바 — 내용이 뷰포트보다 클 때만 표시
    if total > visible {
        let mut scrollbar_state = ScrollbarState::new(max_up)
            .position(max_up.saturating_sub(eff_up));
        let scrollbar = Scrollbar::new(ScrollbarOrientation::VerticalRight)
            .begin_symbol(Some("↑"))
            .end_symbol(Some("↓"))
            .track_symbol(Some("│"))
            .thumb_symbol("█");
        frame.render_stateful_widget(scrollbar, area, &mut scrollbar_state);

        // 스크롤 위치 표시 (우하단)
        if eff_up > 0 {
            let indicator = format!(" ↑{} ", eff_up);
            let ind_x = area.right().saturating_sub(indicator.len() as u16 + 1);
            let ind_y = area.y;
            frame.render_widget(
                Paragraph::new(Span::styled(
                    indicator,
                    Style::default().fg(Color::Yellow).add_modifier(Modifier::BOLD),
                )),
                Rect::new(ind_x, ind_y, 8, 1),
            );
        }
    }
}

fn render_input(app: &App, frame: &mut Frame, area: Rect) {
    let block = Block::default()
        .borders(Borders::ALL)
        .border_type(BorderType::Rounded)
        .border_style(Style::default().fg(Color::Cyan));

    let inner = block.inner(area);
    frame.render_widget(block, area);

    let prompt = "saba> ";
    let line = Line::from(vec![
        Span::styled(
            prompt,
            Style::default()
                .fg(Color::Cyan)
                .add_modifier(Modifier::BOLD),
        ),
        Span::raw(app.input.clone()),
    ]);

    frame.render_widget(Paragraph::new(line), inner);

    // Cursor — 화면 표시 폭은 unicode_width 기준
    let display_width: usize = app.input.chars().take(app.cursor)
        .map(|c| if c.is_ascii() { 1 } else { 2 })
        .sum();
    let cx = inner.x + prompt.len() as u16 + display_width as u16;
    let cy = inner.y;
    frame.set_cursor_position(Position::new(cx, cy));
}

// ═══════════════════════════════════════════════════════
// Entry point
// ═══════════════════════════════════════════════════════

pub async fn run(client: DaemonClient) -> anyhow::Result<()> {
    // Panic hook으로 터미널 상태 복원 보장
    let orig_hook = std::panic::take_hook();
    std::panic::set_hook(Box::new(move |info| {
        let _ = disable_raw_mode();
        let _ = execute!(io::stdout(), DisableMouseCapture, LeaveAlternateScreen);
        orig_hook(info);
    }));

    enable_raw_mode()?;
    let mut stdout = io::stdout();
    execute!(stdout, EnterAlternateScreen, EnableMouseCapture)?;
    let backend = CrosstermBackend::new(stdout);
    let mut terminal = Terminal::new(backend)?;

    let mut app = App::new(client);

    // ── 자동 시작 (백그라운드) ──
    let auto_buf = app.async_out.clone();
    tokio::spawn(auto_start(auto_buf));

    // 백그라운드 상태 모니터 (2초 주기)
    let (tx, mut rx) = mpsc::channel::<Snapshot>(1);
    let base = "http://127.0.0.1:57474".to_string();

    tokio::spawn(async move {
        let http = reqwest::Client::builder()
            .timeout(Duration::from_millis(1500))
            .build()
            .unwrap();

        loop {
            let mut daemon = false;
            let mut servers = vec![];

            // Daemon + 서버 목록을 한 번의 요청으로 확인
            if let Ok(resp) = http
                .get(format!("{}/api/servers", base))
                .send()
                .await
            {
                daemon = true;
                if let Ok(data) = resp.json::<serde_json::Value>().await {
                    if let Some(arr) = data.get("servers").and_then(|v| v.as_array()) {
                        for s in arr {
                            servers.push(ServerInfo {
                                name: s["name"].as_str().unwrap_or("").into(),
                                module: s["module"].as_str().unwrap_or("").into(),
                                status: s["status"].as_str().unwrap_or("").into(),
                            });
                        }
                    }
                }
            }

            let bot = tokio::task::spawn_blocking(process::check_bot_running)
                .await
                .unwrap_or(false);
            let token = gui_config::get_discord_token()
                .ok()
                .flatten()
                .is_some();
            let prefix = gui_config::get_bot_prefix().unwrap_or_else(|_| "!saba".into());

            let _ = tx
                .send(Snapshot {
                    daemon,
                    bot,
                    token,
                    prefix,
                    servers,
                })
                .await;

            tokio::time::sleep(Duration::from_secs(2)).await;
        }
    });

    // 메인 루프
    let result = async {
        loop {
            // 비동기 태스크 결과 플러시
            app.flush_async();

            // 백그라운드 상태 갱신 반영
            while let Ok(snap) = rx.try_recv() {
                app.apply_status(snap);
            }

            terminal.draw(|f| {
                // 화면 높이에서 status(3) + input(3) 제외 = 출력 영역 높이
                app.output_height = f.area().height.saturating_sub(6) as usize;
                render(&app, f);
            })?;

            // 이벤트 폴링 (50ms — ~20fps)
            if event::poll(Duration::from_millis(50))? {
                match event::read()? {
                    Event::Key(key) if key.kind == KeyEventKind::Press => {
                        app.on_key(key);
                    }
                    Event::Mouse(mouse) => match mouse.kind {
                        event::MouseEventKind::ScrollUp => {
                            app.scroll_up = app.scroll_up.saturating_add(3);
                        }
                        event::MouseEventKind::ScrollDown => {
                            app.scroll_up = app.scroll_up.saturating_sub(3);
                        }
                        _ => {}
                    },
                    _ => {}
                }
            }

            if app.quit {
                // 종료 전 잠시 대기 — 백그라운드 종료 태스크 완료 대기
                for _ in 0..40 {
                    // 종료 메시지를 화면에 표시
                    app.flush_async();
                    terminal.draw(|f| {
                        app.output_height = f.area().height.saturating_sub(6) as usize;
                        render(&app, f);
                    })?;
                    tokio::time::sleep(Duration::from_millis(50)).await;
                }
                break;
            }
        }
        Ok::<(), anyhow::Error>(())
    }
    .await;

    // 터미널 복원
    disable_raw_mode()?;
    execute!(terminal.backend_mut(), DisableMouseCapture, LeaveAlternateScreen)?;
    terminal.show_cursor()?;

    result
}

#[cfg(test)]
mod tests {
    use super::*;

    // ─── truncate_str 테스트 ─────────────────────

    #[test]
    fn test_truncate_str_short() {
        assert_eq!(truncate_str("hello", 10), "hello");
    }

    #[test]
    fn test_truncate_str_exact() {
        assert_eq!(truncate_str("hello", 5), "hello");
    }

    #[test]
    fn test_truncate_str_truncated() {
        assert_eq!(truncate_str("hello world", 5), "hello…");
    }

    #[test]
    fn test_truncate_str_empty() {
        assert_eq!(truncate_str("", 5), "");
    }

    #[test]
    fn test_truncate_str_korean() {
        // 한글 3자 + 말줄임
        assert_eq!(truncate_str("한글테스트", 3), "한글테…");
    }

    #[test]
    fn test_truncate_str_korean_exact() {
        assert_eq!(truncate_str("한글", 2), "한글");
    }

    #[test]
    fn test_truncate_str_mixed() {
        // ASCII + 한글 혼합
        assert_eq!(truncate_str("ab한글cd", 4), "ab한글…");
    }

    #[test]
    fn test_truncate_str_zero_max() {
        assert_eq!(truncate_str("hello", 0), "…");
    }

    // ─── smart_scroll 테스트 ─────────────────────

    fn make_app_for_scroll(output_len: usize, output_height: usize) -> App {
        let buf = std::sync::Arc::new(std::sync::Mutex::new(Vec::new()));
        let client = crate::client::DaemonClient::new(None);
        let mut app = App {
            client,
            daemon_on: false,
            bot_on: false,
            token_ok: false,
            bot_prefix: "!".into(),
            servers: Vec::new(),
            registry: std::sync::Arc::new(crate::module_registry::ModuleRegistry::load("")),
            settings: crate::cli_config::CliSettings::default(),
            i18n: std::sync::Arc::new(crate::i18n::I18n::load("en")),
            input: String::new(),
            cursor: 0,
            output: Vec::new(),
            history: Vec::new(),
            hist_idx: None,
            scroll_up: 999,
            output_height,
            quit: false,
            async_out: buf,
        };
        for i in 0..output_len {
            app.output.push(Out::Text(format!("line {}", i)));
        }
        app
    }

    #[test]
    fn test_smart_scroll_fits_viewport() {
        // 출력(5줄)이 viewport(10줄)보다 작으면 scroll_up = 0
        let mut app = make_app_for_scroll(5, 10);
        app.smart_scroll(0);
        assert_eq!(app.scroll_up, 0);
    }

    #[test]
    fn test_smart_scroll_output_exceeds_viewport() {
        // 총 20줄, viewport 10줄, cmd_start=15 → added=5 → fits → scroll_up=0
        let mut app = make_app_for_scroll(20, 10);
        app.smart_scroll(15);
        assert_eq!(app.scroll_up, 0);
    }

    #[test]
    fn test_smart_scroll_big_output() {
        // 총 30줄, viewport 10줄, cmd_start=5 → added=25 → 25>10 → scroll_up=15
        let mut app = make_app_for_scroll(30, 10);
        app.smart_scroll(5);
        assert_eq!(app.scroll_up, 15);
    }

    #[test]
    fn test_smart_scroll_zero_height() {
        // viewport 0이면 scroll_up = 0
        let mut app = make_app_for_scroll(10, 0);
        app.smart_scroll(0);
        assert_eq!(app.scroll_up, 0);
    }

    #[test]
    fn test_smart_scroll_empty_output() {
        let mut app = make_app_for_scroll(0, 10);
        app.smart_scroll(0);
        assert_eq!(app.scroll_up, 0);
    }
}
