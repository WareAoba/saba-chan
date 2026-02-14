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

/// 하트비트 관리용 공유 클라이언트 ID
type SharedClientId = Arc<Mutex<Option<String>>>;

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
    #[allow(dead_code)]
    i18n: Arc<I18n>,
    // 하트비트
    client_id: SharedClientId,
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
            client_id: Arc::new(Mutex::new(None)),
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
                "server".into(), "instance".into(), "module".into(), "daemon".into(), "bot".into(),
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
                    "server managed", "server console", "server stdin", "server diagnose",
                    "server validate", "server eula", "server properties", "server set-property",
                    "instance list", "instance create", "instance delete", "instance show",
                    "instance set", "instance settings",
                    "module list", "module refresh", "module versions", "module install", "module info",
                    "daemon start", "daemon stop", "daemon status", "daemon restart",
                    "bot start", "bot stop", "bot status", "bot token", "bot prefix", "bot alias",
                    "config show", "config set", "config get", "config reset",
                    "update check", "update status", "update download", "update apply",
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
            let exit_client = self.client.clone();
            let exit_client_id = self.client_id.clone();
            tokio::spawn(async move {
                let mut lines = Vec::new();
                // 하트비트 해제
                let maybe_id = exit_client_id.lock().unwrap().take();
                if let Some(id) = maybe_id {
                    let _ = exit_client.unregister_client(&id).await;
                }
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
                Some("server") => exec_server(&client, &lower_parts[1..], &orig_parts[1..]).await,
                Some("instance") => exec_instance(&client, &lower_parts[1..], &orig_parts[1..], &registry).await,
                Some("module") => exec_module(&client, &lower_parts[1..]).await,
                Some("daemon") => exec_daemon(&lower_parts[1..]).await,
                Some("bot") => exec_bot(&lower_parts[1..]).await,
                Some("exec") => exec_exec(&client, &orig_parts[1..]).await,
                Some("update") => exec_update(&client, &lower_parts[1..]).await,
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
                Out::Text("  server list                   list all servers".into()),
                Out::Text("  server status <name>          show server status".into()),
                Out::Text("  server start <name>           start a server".into()),
                Out::Text("  server stop <name> [force]    stop a server".into()),
                Out::Text("  server restart <name>         restart a server".into()),
                Out::Text("  server managed <name>         managed start (auto-launch)".into()),
                Out::Text("  server console <name>         view console output".into()),
                Out::Text("  server stdin <name> <text>    send input to server".into()),
                Out::Text("  server diagnose <name>        diagnose server issues".into()),
                Out::Text("  server validate <name>        validate server config".into()),
                Out::Text("  server eula <name>            accept EULA".into()),
                Out::Text("  server properties <name>      view server properties".into()),
                Out::Text("  server set-property <name> <key> <value>  set property".into()),
            ]),
            Some("instance") if lower.len() == 1 => Some(vec![
                Out::Text("  instance list                list all instances".into()),
                Out::Text("  instance show <name>         show instance details".into()),
                Out::Text("  instance create <name> <module>  create new instance".into()),
                Out::Text("  instance delete <name>       delete an instance".into()),
                Out::Text("  instance settings <name>     show instance settings schema".into()),
                Out::Text("  instance set <name> <key> <value>  update instance setting".into()),
            ]),
            Some("module") if lower.len() == 1 => Some(vec![
                Out::Text("  module list              list loaded modules".into()),
                Out::Text("  module info <name>       show module details".into()),
                Out::Text("  module refresh           refresh all modules".into()),
                Out::Text("  module versions <name>   list available versions".into()),
                Out::Text("  module install <name> [ver]  install server".into()),
            ]),
            Some("update") if lower.len() == 1 => Some(vec![
                Out::Text("  update check             check for updates".into()),
                Out::Text("  update status            show update status".into()),
                Out::Text("  update download          download available updates".into()),
                Out::Text("  update apply             apply downloaded updates".into()),
                Out::Text("  update config            show updater configuration".into()),
                Out::Text("  update install [key]     install component (or all)".into()),
                Out::Text("  update install-status    show installation status".into()),
                Out::Text("  update install-progress  show install progress".into()),
            ]),
            Some("daemon") if lower.len() == 1 => Some(vec![
                Out::Text("  daemon start             start core daemon".into()),
                Out::Text("  daemon stop              stop core daemon".into()),
                Out::Text("  daemon status            show daemon status".into()),
                Out::Text("  daemon restart           restart core daemon".into()),
            ]),
            Some("bot") if lower.len() == 1 => Some(vec![
                Out::Text("  bot start                start Discord bot".into()),
                Out::Text("  bot stop                 stop Discord bot".into()),
                Out::Text("  bot status               show bot status".into()),
                Out::Text("  bot token [show|set|clear]  manage Discord token".into()),
                Out::Text("  bot prefix [show|set]    manage bot prefix".into()),
                Out::Text("  bot alias [show|set|reset]  manage Discord aliases".into()),
            ]),
            Some("bot") if lower.len() >= 2 && lower[1] == "token" => {
                Some(self.cmd_bot_token(&orig[2..]))
            }
            Some("bot") if lower.len() >= 2 && lower[1] == "prefix" => {
                Some(self.cmd_bot_prefix(&orig[2..]))
            }
            Some("bot") if lower.len() >= 2 && lower[1] == "status" => {
                Some(self.cmd_bot_status())
            }
            Some("bot") if lower.len() >= 2 && lower[1] == "alias" => {
                Some(self.cmd_bot_alias(&lower[2..], &orig[2..]))
            }
            Some("exec") if lower.len() < 4 => Some(vec![
                Out::Text("  exec <id> cmd <command>  execute command".into()),
                Out::Text("  exec <id> rcon <cmd>     execute RCON".into()),
                Out::Text("  exec <id> rest <cmd>     execute REST".into()),
            ]),
            _ => {
                // Easter egg
                if lower.len() == 1 && lower[0] == "sabachan" {
                    return Some(cmd_sabachan());
                }
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
                let auto_start_gui = gui_config::get_discord_auto_start().unwrap_or(false);
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
                    Out::Text(format!("  discord_auto     {}", auto_start_gui)),
                ];
                lines.push(Out::Blank);
                lines.push(Out::Info("CLI config:".into()));
                lines.push(Out::Text("  config set <key> <value>   change a CLI setting".into()));
                lines.push(Out::Text("  config get <key>           show one setting".into()));
                lines.push(Out::Text("  config reset <key>         reset to default".into()));
                lines.push(Out::Text(format!("  keys: {}", CliSettings::available_keys().iter().map(|(k,_)| *k).collect::<Vec<_>>().join(", "))));
                lines.push(Out::Blank);
                lines.push(Out::Info("GUI config (shared with GUI):".into()));
                lines.push(Out::Text("  config gui language <lang>         set GUI language".into()));
                lines.push(Out::Text("  config gui modules_path <path>     set modules directory".into()));
                lines.push(Out::Text("  config gui token <token>           set Discord token".into()));
                lines.push(Out::Text("  config gui token clear             clear Discord token".into()));
                lines
            }
            Some("gui") => self.cmd_config_gui(&args[1..]),
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
                vec![Out::Err(format!("Unknown config subcommand: {}. Try: show, get, set, reset, gui", sub))]
            }
        }
    }

    /// GUI 공유 설정 변경 (settings.json / bot-config.json)
    fn cmd_config_gui(&self, args: &[&str]) -> Vec<Out> {
        match args.first().copied() {
            Some("language") | Some("lang") => {
                if args.len() < 2 {
                    let cur = gui_config::get_language().unwrap_or_else(|_| "en".into());
                    return vec![
                        Out::Ok(format!("GUI language: {}", cur)),
                        Out::Text("  Available: en, ko, ja, zh-CN, zh-TW, es, pt-BR, ru, de, fr".into()),
                        Out::Text("  Usage: config gui language <lang>".into()),
                    ];
                }
                let lang = args[1];
                match gui_config::set_language(lang) {
                    Ok(()) => vec![Out::Ok(format!("✓ GUI language set to: {}", lang))],
                    Err(e) => vec![Out::Err(format!("✗ {}", e))],
                }
            }
            Some("modules_path") | Some("modules") => {
                if args.len() < 2 {
                    let cur = gui_config::get_modules_path().unwrap_or_default();
                    return vec![
                        Out::Ok(format!("Modules path: {}", cur)),
                        Out::Text("  Usage: config gui modules_path <path>".into()),
                    ];
                }
                let path = args[1..].join(" ");
                match gui_config::set_modules_path(&path) {
                    Ok(()) => vec![Out::Ok(format!("✓ Modules path set to: {}", path))],
                    Err(e) => vec![Out::Err(format!("✗ {}", e))],
                }
            }
            Some("token") => {
                if args.len() < 2 {
                    match gui_config::get_discord_token() {
                        Ok(Some(t)) => {
                            let masked = if t.len() > 8 {
                                format!("{}...{}", &t[..4], &t[t.len()-4..])
                            } else {
                                "****".into()
                            };
                            vec![Out::Ok(format!("Token: {}", masked))]
                        }
                        _ => vec![Out::Text("Token: not set".into())],
                    }
                } else if args[1] == "clear" {
                    match gui_config::clear_discord_token() {
                        Ok(()) => vec![Out::Ok("✓ Discord token cleared.".into())],
                        Err(e) => vec![Out::Err(format!("✗ {}", e))],
                    }
                } else {
                    let token = args[1];
                    match gui_config::set_discord_token(token) {
                        Ok(()) => vec![Out::Ok("✓ Discord token saved.".into())],
                        Err(e) => vec![Out::Err(format!("✗ {}", e))],
                    }
                }
            }
            _ => vec![
                Out::Err("Usage: config gui [language|modules_path|token] <value>".into()),
            ],
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

    fn cmd_bot_status(&self) -> Vec<Out> {
        let running = process::check_bot_running();
        let token = gui_config::get_discord_token().ok().flatten();
        let prefix = gui_config::get_bot_prefix().unwrap_or_else(|_| "!saba".into());
        let auto = gui_config::get_discord_auto_start().unwrap_or(false);

        let status_str = if running {
            "● RUNNING"
        } else if token.is_none() {
            "○ NO TOKEN"
        } else {
            "○ OFFLINE"
        };

        vec![
            Out::Ok(format!("Discord Bot: {}", status_str)),
            Out::Text(format!("  Token:      {}", if token.is_some() { "✓ set" } else { "✗ not set" })),
            Out::Text(format!("  Prefix:     {}", prefix)),
            Out::Text(format!("  Auto-start: {}", auto)),
        ]
    }

    fn cmd_bot_alias(&self, lower: &[&str], orig: &[&str]) -> Vec<Out> {
        match lower.first().copied() {
            None | Some("show") => {
                let config = gui_config::load_bot_config().unwrap_or_default();
                let mut lines = vec![Out::Ok("Discord Bot Aliases:".into())];

                // Module aliases
                lines.push(Out::Blank);
                lines.push(Out::Info("Module Aliases:".into()));
                if let Some(aliases) = config.get("moduleAliases").and_then(|v| v.as_object()) {
                    if aliases.is_empty() {
                        lines.push(Out::Text("  (none)".into()));
                    } else {
                        for (module, alias) in aliases {
                            lines.push(Out::Text(format!("  {} → {}", module, alias.as_str().unwrap_or("?"))));
                        }
                    }
                } else {
                    lines.push(Out::Text("  (none)".into()));
                }

                // Command aliases
                lines.push(Out::Blank);
                lines.push(Out::Info("Command Aliases:".into()));
                if let Some(cmd_aliases) = config.get("commandAliases").and_then(|v| v.as_object()) {
                    if cmd_aliases.is_empty() {
                        lines.push(Out::Text("  (none)".into()));
                    } else {
                        for (module, cmds) in cmd_aliases {
                            if let Some(cmd_map) = cmds.as_object() {
                                for (cmd, alias) in cmd_map {
                                    lines.push(Out::Text(format!("  {}.{} → {}", module, cmd, alias.as_str().unwrap_or("?"))));
                                }
                            }
                        }
                    }
                } else {
                    lines.push(Out::Text("  (none)".into()));
                }

                lines.push(Out::Blank);
                lines.push(Out::Text("  bot alias set module <module> <aliases>    set module alias (comma-separated)".into()));
                lines.push(Out::Text("  bot alias set command <module> <cmd> <aliases>  set command alias".into()));
                lines.push(Out::Text("  bot alias reset                            reset all aliases".into()));
                lines
            }
            Some("set") => {
                if lower.len() < 2 {
                    return vec![Out::Err("Usage: bot alias set [module|command] ...".into())];
                }
                match lower[1] {
                    "module" => {
                        if orig.len() < 4 {
                            return vec![Out::Err("Usage: bot alias set module <module_name> <alias1,alias2>".into())];
                        }
                        let module_name = orig[2];
                        let aliases = orig[3];
                        let mut config = gui_config::load_bot_config().unwrap_or_default();
                        if config.get("moduleAliases").is_none() {
                            config["moduleAliases"] = serde_json::json!({});
                        }
                        config["moduleAliases"][module_name] = serde_json::Value::String(aliases.to_string());
                        match gui_config::set_bot_prefix("") {
                            _ => {} // side effect of saving
                        }
                        // Save the full config
                        let path = gui_config::get_bot_config_path_pub();
                        match save_json_file(&path, &config) {
                            Ok(()) => vec![Out::Ok(format!("✓ Module alias set: {} → {}", module_name, aliases))],
                            Err(e) => vec![Out::Err(format!("✗ {}", e))],
                        }
                    }
                    "command" | "cmd" => {
                        if orig.len() < 5 {
                            return vec![Out::Err("Usage: bot alias set command <module> <command> <alias1,alias2>".into())];
                        }
                        let module_name = orig[2];
                        let cmd_name = orig[3];
                        let aliases = orig[4];
                        let mut config = gui_config::load_bot_config().unwrap_or_default();
                        if config.get("commandAliases").is_none() {
                            config["commandAliases"] = serde_json::json!({});
                        }
                        if config["commandAliases"].get(module_name).is_none() {
                            config["commandAliases"][module_name] = serde_json::json!({});
                        }
                        config["commandAliases"][module_name][cmd_name] = serde_json::Value::String(aliases.to_string());
                        let path = gui_config::get_bot_config_path_pub();
                        match save_json_file(&path, &config) {
                            Ok(()) => vec![Out::Ok(format!("✓ Command alias set: {}.{} → {}", module_name, cmd_name, aliases))],
                            Err(e) => vec![Out::Err(format!("✗ {}", e))],
                        }
                    }
                    _ => vec![Out::Err("Usage: bot alias set [module|command] ...".into())],
                }
            }
            Some("reset") => {
                let mut config = gui_config::load_bot_config().unwrap_or_default();
                config["moduleAliases"] = serde_json::json!({});
                config["commandAliases"] = serde_json::json!({});
                let path = gui_config::get_bot_config_path_pub();
                match save_json_file(&path, &config) {
                    Ok(()) => vec![Out::Ok("✓ All aliases reset to defaults.".into())],
                    Err(e) => vec![Out::Err(format!("✗ {}", e))],
                }
            }
            Some(sub) => {
                vec![Out::Err(format!("Unknown: bot alias {}. Try: show, set, reset", sub))]
            }
        }
    }

    fn cmd_help(&self) -> Vec<Out> {
        let mut lines = vec![
            Out::Text("  server   [list|start|stop|restart|status] <name>".into()),
            Out::Text("  server   [managed|console|stdin|diagnose|validate|eula|properties] <name>".into()),
            Out::Text("  server   set-property <name> <key> <value>".into()),
            Out::Text("  instance [list|show|create|delete|settings|set] <name>".into()),
            Out::Text("  module   [list|info|refresh|versions|install]".into()),
            Out::Text("  daemon   [start|stop|status|restart]".into()),
            Out::Text("  bot      [start|stop|status]".into()),
            Out::Text("  bot      token [show|set|clear]".into()),
            Out::Text("  bot      prefix [show|set]".into()),
            Out::Text("  bot      alias [show|set|reset]".into()),
            Out::Text("  exec     <id> [cmd|rcon|rest] <command>".into()),
            Out::Text("  update   [check|status|download|apply|config|install]".into()),
            Out::Text("  config   [show|set|get|reset] — CLI/GUI settings".into()),
            Out::Text("  config   gui [language|modules_path|token] — GUI shared settings".into()),
            Out::Text("  help     — This help".into()),
            Out::Text("  exit     — Quit (Ctrl+C)".into()),
        ];

        // 등록된 모듈 단축키 표시
        if !self.registry.modules.is_empty() {
            lines.push(Out::Blank);
            lines.push(Out::Info("Module shortcuts:".into()));
            for mi in &self.registry.modules {
                let mode = mi.interaction_mode.as_deref().unwrap_or("-");
                lines.push(Out::Text(format!(
                    "  {:<10} {} [{}] — type '{}' for commands",
                    mi.name, mi.display_name, mode, mi.name,
                )));
            }
        }

        lines
    }
}

// ═══════════════════════════════════════════════════════
// 비동기 명령 실행 (tokio::spawn에서 호출되는 자유 함수)
// ═══════════════════════════════════════════════════════

/// JSON 파일 저장 헬퍼
fn save_json_file(path: &std::path::PathBuf, data: &serde_json::Value) -> anyhow::Result<()> {
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)?;
    }
    let content = serde_json::to_string_pretty(data)?;
    std::fs::write(path, content)?;
    Ok(())
}

/// start_time (UNIX timestamp) 기반 경과시간을 hh:mm:ss 포맷으로 변환
fn format_uptime(start_time: Option<u64>) -> String {
    use std::time::{SystemTime, UNIX_EPOCH};
    match start_time {
        Some(t) => {
            let now = SystemTime::now().duration_since(UNIX_EPOCH).unwrap_or_default().as_secs();
            let elapsed = now.saturating_sub(t);
            let h = elapsed / 3600;
            let m = (elapsed % 3600) / 60;
            let s = elapsed % 60;
            format!("{:02}:{:02}:{:02}", h, m, s)
        }
        None => "-".into(),
    }
}

async fn exec_server(client: &DaemonClient, args: &[&str], orig_args: &[&str]) -> Vec<Out> {
    match args.first().copied() {
        Some("list") => match client.list_servers().await {
            Ok(list) if list.is_empty() => vec![Out::Text("No servers configured.".into())],
            Ok(list) => {
                let mut o = vec![Out::Ok(format!("{} server(s):", list.len()))];
                for s in &list {
                    let st = s["status"].as_str().unwrap_or("?");
                    let sym = if st == "running" { "▶" } else { "■" };
                    let pid_str = match s["pid"].as_u64() {
                        Some(p) => format!(" PID:{}", p),
                        None => String::new(),
                    };
                    let uptime = match s["start_time"].as_u64() {
                        Some(_) => format!(" ⏱{}", format_uptime(s["start_time"].as_u64())),
                        None => String::new(),
                    };
                    o.push(Out::Text(format!(
                        "  {} {} [{}] — {}{}{}",
                        sym,
                        s["name"].as_str().unwrap_or("?"),
                        s["module"].as_str().unwrap_or("?"),
                        st,
                        pid_str,
                        uptime,
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
            let force = args.get(2).map(|&s| s == "force" || s == "true").unwrap_or(false);
            match client.stop_server(args[1], force).await {
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
                let uptime = format_uptime(s["start_time"].as_u64());
                vec![Out::Text(format!("{} — {} | PID {} | Uptime {}", args[1], status, pid_str, uptime))]
            }
            Err(e) => vec![Out::Err(format!("✗ {}", e))],
        },
        Some("managed") if args.len() > 1 => {
            // 인스턴스 ID를 이름으로 찾아서 managed start 호출
            let name = args[1];
            let instance_id = match find_instance_id_by_name(client, name).await {
                Some(id) => id,
                None => return vec![Out::Err(format!("✗ Instance '{}' not found", name))],
            };
            match client.start_managed(&instance_id).await {
                Ok(r) => vec![Out::Ok(format!(
                    "✓ {}",
                    r.get("message").and_then(|v| v.as_str()).unwrap_or("Managed server started"),
                ))],
                Err(e) => vec![Out::Err(format!("✗ {}", e))],
            }
        }
        Some("console") if args.len() > 1 => {
            let name = args[1];
            let instance_id = match find_instance_id_by_name(client, name).await {
                Some(id) => id,
                None => return vec![Out::Err(format!("✗ Instance '{}' not found", name))],
            };
            match client.get_console(&instance_id).await {
                Ok(data) => {
                    if let Some(lines) = data.get("lines").and_then(|v| v.as_array()) {
                        let mut o = vec![Out::Ok(format!("Console output ({} lines):", lines.len()))];
                        // 마지막 50줄만 표시
                        let start = lines.len().saturating_sub(50);
                        for line in &lines[start..] {
                            o.push(Out::Text(line.as_str().unwrap_or("").into()));
                        }
                        o
                    } else if let Some(output) = data.get("output").and_then(|v| v.as_str()) {
                        // 단일 output 문자열 형태
                        let mut o = vec![Out::Ok("Console output:".into())];
                        for line in output.lines().rev().take(50).collect::<Vec<_>>().into_iter().rev() {
                            o.push(Out::Text(line.into()));
                        }
                        o
                    } else {
                        vec![Out::Ok(format!("{}", serde_json::to_string_pretty(&data).unwrap_or_default()))]
                    }
                }
                Err(e) => vec![Out::Err(format!("✗ {}", e))],
            }
        }
        Some("stdin") if args.len() > 2 => {
            let name = args[1];
            let text = args[2..].join(" ");
            let instance_id = match find_instance_id_by_name(client, name).await {
                Some(id) => id,
                None => return vec![Out::Err(format!("✗ Instance '{}' not found", name))],
            };
            match client.send_stdin(&instance_id, &text).await {
                Ok(_) => vec![Out::Ok(format!("✓ Sent to {}: {}", name, text))],
                Err(e) => vec![Out::Err(format!("✗ {}", e))],
            }
        }
        Some("diagnose") if args.len() > 1 => {
            let name = args[1];
            let instance_id = match find_instance_id_by_name(client, name).await {
                Some(id) => id,
                None => return vec![Out::Err(format!("✗ Instance '{}' not found", name))],
            };
            match client.diagnose(&instance_id).await {
                Ok(data) => {
                    let mut o = vec![Out::Ok(format!("Diagnosis for '{}':", name))];
                    if let Some(obj) = data.as_object() {
                        for (k, v) in obj {
                            let val = match v {
                                serde_json::Value::String(s) => s.clone(),
                                _ => v.to_string(),
                            };
                            o.push(Out::Text(format!("  {}: {}", k, val)));
                        }
                    } else {
                        o.push(Out::Text(format!("  {}", serde_json::to_string_pretty(&data).unwrap_or_default())));
                    }
                    o
                }
                Err(e) => vec![Out::Err(format!("✗ {}", e))],
            }
        }
        Some("validate") if args.len() > 1 => {
            let name = args[1];
            let instance_id = match find_instance_id_by_name(client, name).await {
                Some(id) => id,
                None => return vec![Out::Err(format!("✗ Instance '{}' not found", name))],
            };
            match client.validate_instance(&instance_id).await {
                Ok(r) => vec![Out::Ok(format!(
                    "✓ {}",
                    r.get("message").and_then(|v| v.as_str()).unwrap_or("Validation passed"),
                ))],
                Err(e) => vec![Out::Err(format!("✗ {}", e))],
            }
        }
        Some("eula") if args.len() > 1 => {
            let name = args[1];
            let instance_id = match find_instance_id_by_name(client, name).await {
                Some(id) => id,
                None => return vec![Out::Err(format!("✗ Instance '{}' not found", name))],
            };
            match client.accept_eula(&instance_id).await {
                Ok(_) => vec![Out::Ok(format!("✓ EULA accepted for '{}'", name))],
                Err(e) => vec![Out::Err(format!("✗ {}", e))],
            }
        }
        Some("properties") if args.len() > 1 => {
            let name = args[1];
            let instance_id = match find_instance_id_by_name(client, name).await {
                Some(id) => id,
                None => return vec![Out::Err(format!("✗ Instance '{}' not found", name))],
            };
            match client.read_properties(&instance_id).await {
                Ok(data) => {
                    let mut o = vec![Out::Ok(format!("Properties for '{}':", name))];
                    if let Some(obj) = data.get("properties").and_then(|v| v.as_object()) {
                        for (k, v) in obj {
                            o.push(Out::Text(format!("  {} = {}", k, v)));
                        }
                    } else if let Some(obj) = data.as_object() {
                        for (k, v) in obj {
                            let val = match v {
                                serde_json::Value::String(s) => s.clone(),
                                _ => v.to_string(),
                            };
                            o.push(Out::Text(format!("  {} = {}", k, val)));
                        }
                    }
                    o
                }
                Err(e) => vec![Out::Err(format!("✗ {}", e))],
            }
        }
        Some("set-property") if orig_args.len() > 3 => {
            let name = orig_args[1];
            let key = orig_args[2];
            let value = orig_args[3..].join(" ");
            let instance_id = match find_instance_id_by_name(client, name).await {
                Some(id) => id,
                None => return vec![Out::Err(format!("✗ Instance '{}' not found", name))],
            };
            // Read current properties, modify, write back
            match client.read_properties(&instance_id).await {
                Ok(data) => {
                    let mut props = if let Some(obj) = data.get("properties") {
                        obj.clone()
                    } else {
                        data.clone()
                    };
                    props[key] = serde_json::Value::String(value.clone());
                    let write_data = serde_json::json!({ "properties": props });
                    match client.write_properties(&instance_id, write_data).await {
                        Ok(_) => vec![Out::Ok(format!("✓ {} = {}", key, value))],
                        Err(e) => vec![Out::Err(format!("✗ Write failed: {}", e))],
                    }
                }
                Err(e) => vec![Out::Err(format!("✗ Read properties failed: {}", e))],
            }
        }
        _ => vec![Out::Err("Usage: server [list|start|stop|restart|status|managed|console|stdin|diagnose|validate|eula|properties|set-property] <name>".into())],
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

/// 인스턴스 이름으로 ID를 찾는 유틸리티
async fn find_instance_id_by_name(client: &DaemonClient, name: &str) -> Option<String> {
    if let Ok(instances) = client.list_instances().await {
        for inst in &instances {
            if inst["name"].as_str() == Some(name) {
                return inst["id"].as_str().map(|s| s.to_string());
            }
        }
    }
    None
}

/// 인스턴스 CRUD 명령어 (GUI의 서버 추가/삭제/설정 대체)
async fn exec_instance(client: &DaemonClient, lower: &[&str], orig: &[&str], registry: &ModuleRegistry) -> Vec<Out> {
    match lower.first().copied() {
        Some("list") => match client.list_instances().await {
            Ok(list) if list.is_empty() => vec![Out::Text("No instances configured.".into())],
            Ok(list) => {
                let mut o = vec![Out::Ok(format!("{} instance(s):", list.len()))];
                for inst in &list {
                    let name = inst["name"].as_str().unwrap_or("?");
                    let module = inst["module_name"].as_str().unwrap_or("?");
                    let id = inst["id"].as_str().unwrap_or("?");
                    o.push(Out::Text(format!("  {} [{}] id:{}", name, module, id)));
                }
                o
            }
            Err(e) => vec![Out::Err(format!("✗ {}", e))],
        },
        Some("show") if orig.len() > 1 => {
            let name = orig[1];
            let instance_id = match find_instance_id_by_name(client, name).await {
                Some(id) => id,
                None => return vec![Out::Err(format!("✗ Instance '{}' not found", name))],
            };
            match client.get_instance(&instance_id).await {
                Ok(data) => {
                    let mut o = vec![Out::Ok(format!("Instance: {}", name))];
                    o.push(Out::Text(format!("  id:          {}", data["id"].as_str().unwrap_or("?"))));
                    o.push(Out::Text(format!("  module:      {}", data["module_name"].as_str().unwrap_or("?"))));

                    // Show all settings
                    if let Some(settings) = data.as_object() {
                        o.push(Out::Blank);
                        o.push(Out::Info("Settings:".into()));
                        for (key, val) in settings {
                            if key == "id" || key == "name" || key == "module_name" {
                                continue;
                            }
                            let val_str = match val {
                                serde_json::Value::String(s) => {
                                    if key.contains("token") || key.contains("password") {
                                        if s.is_empty() { "(empty)".into() } else { "****".into() }
                                    } else {
                                        s.clone()
                                    }
                                }
                                serde_json::Value::Null => "(not set)".into(),
                                _ => val.to_string(),
                            };
                            o.push(Out::Text(format!("  {:<24} {}", key, val_str)));
                        }
                    }
                    o
                }
                Err(e) => vec![Out::Err(format!("✗ {}", e))],
            }
        }
        Some("create") if orig.len() > 2 => {
            let name = orig[1];
            let module = orig[2];

            // Validate module name
            let module_name = registry.resolve_module_name(module)
                .unwrap_or_else(|| module.to_string());

            let data = serde_json::json!({
                "name": name,
                "module_name": module_name,
            });
            match client.create_instance(data).await {
                Ok(r) => {
                    let id = r.get("id").and_then(|v| v.as_str()).unwrap_or("?");
                    vec![Out::Ok(format!("✓ Instance '{}' created (module: {}, id: {})", name, module_name, id))]
                }
                Err(e) => vec![Out::Err(format!("✗ {}", e))],
            }
        }
        Some("delete") if orig.len() > 1 => {
            let name = orig[1];
            let instance_id = match find_instance_id_by_name(client, name).await {
                Some(id) => id,
                None => return vec![Out::Err(format!("✗ Instance '{}' not found", name))],
            };
            match client.delete_instance(&instance_id).await {
                Ok(_) => vec![Out::Ok(format!("✓ Instance '{}' deleted", name))],
                Err(e) => vec![Out::Err(format!("✗ {}", e))],
            }
        }
        Some("settings") if orig.len() > 1 => {
            // Show the module's settings schema for this instance
            let name = orig[1];
            let instance_id = match find_instance_id_by_name(client, name).await {
                Some(id) => id,
                None => return vec![Out::Err(format!("✗ Instance '{}' not found", name))],
            };
            // Get instance to find module name
            let inst = match client.get_instance(&instance_id).await {
                Ok(d) => d,
                Err(e) => return vec![Out::Err(format!("✗ {}", e))],
            };
            let module_name = inst["module_name"].as_str().unwrap_or("?");

            // Get module metadata for settings schema
            match client.get_module(module_name).await {
                Ok(data) => {
                    let mut o = vec![Out::Ok(format!("Settings schema for '{}' (module: {}):", name, module_name))];
                    if let Some(fields) = data.get("settings").and_then(|v| v.get("fields")).and_then(|v| v.as_array()) {
                        for field in fields {
                            let fname = field["name"].as_str().unwrap_or("?");
                            let ftype = field["type"].as_str().unwrap_or("?");
                            let flabel = field["label"].as_str().unwrap_or("");
                            let group = field["group"].as_str().unwrap_or("basic");
                            let required = field["required"].as_bool().unwrap_or(false);
                            let default = field.get("default").map(|v| match v {
                                serde_json::Value::String(s) => s.clone(),
                                _ => v.to_string(),
                            }).unwrap_or_default();
                            let req_mark = if required { "*" } else { " " };

                            // Current value from instance
                            let current = inst.get(fname).map(|v| match v {
                                serde_json::Value::String(s) => s.clone(),
                                serde_json::Value::Null => "(not set)".into(),
                                _ => v.to_string(),
                            }).unwrap_or_else(|| "(not set)".into());

                            o.push(Out::Text(format!(
                                "  {}{:<24} {:>8} [{}] current={} default={} — {}",
                                req_mark, fname, ftype, group, current, default, flabel
                            )));

                            // Show options for select type
                            if ftype == "select" {
                                if let Some(opts) = field.get("options").and_then(|v| v.as_array()) {
                                    let opt_strs: Vec<&str> = opts.iter().filter_map(|v| v.as_str()).collect();
                                    o.push(Out::Text(format!("    options: {}", opt_strs.join(", "))));
                                }
                            }
                        }
                        o.push(Out::Blank);
                        o.push(Out::Text(format!("  Use: instance set {} <key> <value>", name)));
                    } else {
                        o.push(Out::Text("  No settings schema available.".into()));
                    }
                    o
                }
                Err(e) => vec![Out::Err(format!("✗ {}", e))],
            }
        }
        Some("set") if orig.len() > 3 => {
            let name = orig[1];
            let key = orig[2];
            let value = orig[3..].join(" ");
            let instance_id = match find_instance_id_by_name(client, name).await {
                Some(id) => id,
                None => return vec![Out::Err(format!("✗ Instance '{}' not found", name))],
            };

            // Try to parse value as appropriate type
            let json_value = if value == "true" {
                serde_json::Value::Bool(true)
            } else if value == "false" {
                serde_json::Value::Bool(false)
            } else if let Ok(n) = value.parse::<i64>() {
                serde_json::json!(n)
            } else if let Ok(f) = value.parse::<f64>() {
                serde_json::json!(f)
            } else {
                serde_json::Value::String(value.clone())
            };

            let settings = serde_json::json!({ key: json_value });
            match client.update_instance(&instance_id, settings).await {
                Ok(_) => vec![Out::Ok(format!("✓ {}.{} = {}", name, key, value))],
                Err(e) => vec![Out::Err(format!("✗ {}", e))],
            }
        }
        _ => vec![Out::Err("Usage: instance [list|show <name>|create <name> <module>|delete <name>|settings <name>|set <name> <key> <value>]".into())],
    }
}

async fn exec_module(client: &DaemonClient, args: &[&str]) -> Vec<Out> {
    match args.first().copied() {
        Some("list") => match client.list_modules().await {
            Ok(list) if list.is_empty() => vec![Out::Text("No modules loaded.".into())],
            Ok(list) => {
                let mut o = vec![Out::Ok(format!("{} module(s):", list.len()))];
                for m in &list {
                    let mode = m["interaction_mode"].as_str().unwrap_or("-");
                    o.push(Out::Text(format!(
                        "  • {} v{} [{}]",
                        m["name"].as_str().unwrap_or("?"),
                        m["version"].as_str().unwrap_or("?"),
                        mode,
                    )));
                }
                o
            }
            Err(e) => vec![Out::Err(format!("✗ {}", e))],
        },
        Some("info") if args.len() > 1 => {
            let name = args[1];
            match client.get_module(name).await {
                Ok(data) => {
                    let mut o = vec![Out::Ok(format!("Module: {}", name))];
                    // Basic info
                    for key in &["name", "version", "description", "game_name", "display_name", "interaction_mode"] {
                        if let Some(val) = data.get(*key).and_then(|v| v.as_str()) {
                            o.push(Out::Text(format!("  {:<20} {}", key, val)));
                        }
                    }
                    // Protocols
                    if let Some(protos) = data.get("protocols") {
                        o.push(Out::Blank);
                        o.push(Out::Info("Protocols:".into()));
                        if let Some(supported) = protos.get("supported").and_then(|v| v.as_array()) {
                            let list: Vec<&str> = supported.iter().filter_map(|v| v.as_str()).collect();
                            o.push(Out::Text(format!("  supported: {}", list.join(", "))));
                        }
                        if let Some(default) = protos.get("default").and_then(|v| v.as_str()) {
                            o.push(Out::Text(format!("  default:   {}", default)));
                        }
                    }
                    // Settings fields summary
                    if let Some(settings) = data.get("settings").and_then(|v| v.get("fields")).and_then(|v| v.as_array()) {
                        o.push(Out::Blank);
                        o.push(Out::Info(format!("Settings ({} fields):", settings.len())));
                        for field in settings {
                            let fname = field["name"].as_str().unwrap_or("?");
                            let ftype = field["type"].as_str().unwrap_or("?");
                            let flabel = field["label"].as_str().unwrap_or("");
                            let group = field["group"].as_str().unwrap_or("basic");
                            let required = field["required"].as_bool().unwrap_or(false);
                            let req_mark = if required { "*" } else { " " };
                            o.push(Out::Text(format!("  {}{:<24} {:>8} [{}] {}", req_mark, fname, ftype, group, flabel)));
                        }
                    }
                    // Commands
                    if let Some(cmds) = data.get("commands").and_then(|v| v.get("fields")).and_then(|v| v.as_array()) {
                        o.push(Out::Blank);
                        o.push(Out::Info(format!("Commands ({}):", cmds.len())));
                        for cmd in cmds {
                            let cname = cmd["name"].as_str().unwrap_or("?");
                            let cdesc = cmd["description"].as_str().unwrap_or("");
                            let method = cmd["method"].as_str().unwrap_or("-");
                            o.push(Out::Text(format!("  {:<16} [{}] {}", cname, method, cdesc)));
                        }
                    }
                    // Aliases
                    if let Some(aliases) = data.get("aliases").and_then(|v| v.get("module_aliases")).and_then(|v| v.as_array()) {
                        if !aliases.is_empty() {
                            o.push(Out::Blank);
                            let list: Vec<&str> = aliases.iter().filter_map(|v| v.as_str()).collect();
                            o.push(Out::Info(format!("Aliases: {}", list.join(", "))));
                        }
                    }
                    o
                }
                Err(e) => vec![Out::Err(format!("✗ {}", e))],
            }
        }
        Some("refresh") | Some("reload") => match client.refresh_modules().await {
            Ok(_) => vec![Out::Ok("✓ Modules refreshed".into())],
            Err(e) => vec![Out::Err(format!("✗ {}", e))],
        },
        Some("versions") if args.len() > 1 => {
            let module = args[1];
            match client.list_versions(module).await {
                Ok(data) => {
                    if let Some(versions) = data.get("versions").and_then(|v| v.as_array()) {
                        if versions.is_empty() {
                            return vec![Out::Text(format!("No versions found for '{}'.", module))];
                        }
                        let mut o = vec![Out::Ok(format!("{} version(s) for '{}':", versions.len(), module))];
                        for v in versions {
                            let id = v.as_str().or_else(|| v["id"].as_str()).unwrap_or("?");
                            o.push(Out::Text(format!("  • {}", id)));
                        }
                        o
                    } else {
                        vec![Out::Ok(format!("{}", serde_json::to_string_pretty(&data).unwrap_or_default()))]
                    }
                }
                Err(e) => vec![Out::Err(format!("✗ {}", e))],
            }
        }
        Some("install") if args.len() > 1 => {
            let module = args[1];
            let version = args.get(2).copied().unwrap_or("latest");
            let body = serde_json::json!({ "version": version });
            match client.install_server(module, body).await {
                Ok(r) => vec![Out::Ok(format!(
                    "✓ {}",
                    r.get("message").and_then(|v| v.as_str()).unwrap_or("Install started"),
                ))],
                Err(e) => vec![Out::Err(format!("✗ {}", e))],
            }
        }
        _ => vec![Out::Err("Usage: module [list|info <name>|refresh|versions <name>|install <name> [version]]".into())],
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
        Some("status") => {
            let running = tokio::task::spawn_blocking(process::check_daemon_running)
                .await
                .unwrap_or(false);
            if running {
                // Try to get additional info from daemon API
                let http = reqwest::Client::builder()
                    .timeout(Duration::from_secs(2))
                    .build()
                    .unwrap();
                let mut lines = vec![Out::Ok("Daemon: ● RUNNING".into())];
                lines.push(Out::Text("  Host:     127.0.0.1".into()));
                lines.push(Out::Text("  Port:     57474".into()));
                lines.push(Out::Text("  Protocol: HTTP REST".into()));
                // Get module/instance counts
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
                lines
            } else {
                vec![Out::Text("Daemon: ○ OFFLINE".into())]
            }
        }
        Some("restart") => {
            // Stop then start
            let stop_result = tokio::task::spawn_blocking(process::stop_daemon).await;
            match stop_result {
                Ok(Ok(msg)) => {
                    let mut lines = vec![Out::Ok(msg)];
                    tokio::time::sleep(Duration::from_secs(1)).await;
                    match tokio::task::spawn_blocking(process::start_daemon).await {
                        Ok(Ok(msg2)) => {
                            for l in msg2.lines() {
                                lines.push(Out::Ok(l.into()));
                            }
                        }
                        Ok(Err(e)) => lines.push(Out::Err(format!("✗ Start: {}", e))),
                        Err(e) => lines.push(Out::Err(format!("✗ Start: {}", e))),
                    }
                    lines
                }
                Ok(Err(e)) => vec![Out::Err(format!("✗ Stop: {}", e))],
                Err(e) => vec![Out::Err(format!("✗ Stop: {}", e))],
            }
        }
        _ => vec![Out::Err("Usage: daemon [start|stop|status|restart]".into())],
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
        // token, prefix, status, alias are handled synchronously in dispatch_sync
        _ => vec![Out::Err("Usage: bot [start|stop|status|token|prefix|alias]".into())],
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
                let uptime = format_uptime(s["start_time"].as_u64());
                vec![Out::Text(format!(
                    "{} — {} | PID {} | Uptime {}",
                    instance_name, status, pid_str, uptime,
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

// ═══════════════════════════════════════════════════════
// 업데이트 / 인스톨 명령
// ═══════════════════════════════════════════════════════

async fn exec_update(client: &DaemonClient, args: &[&str]) -> Vec<Out> {
    match args.first().copied() {
        Some("check") => match client.check_updates().await {
            Ok(v) => {
                let components = v["components"].as_array();
                let mut lines = vec![];
                if let Some(comps) = components {
                    let any = comps.iter().any(|c| c["update_available"].as_bool().unwrap_or(false));
                    if any {
                        lines.push(Out::Ok("Updates available:".into()));
                        for c in comps {
                            let name = c["component"].as_str().unwrap_or("?");
                            let cur = c["current_version"].as_str().unwrap_or("?");
                            let lat = c["latest_version"].as_str().unwrap_or("?");
                            let avail = c["update_available"].as_bool().unwrap_or(false);
                            let marker = if avail { "⬆" } else { "✓" };
                            lines.push(Out::Text(format!("  {} {:<20} {} → {}", marker, name, cur, lat)));
                        }
                    } else {
                        lines.push(Out::Ok("All components are up to date.".into()));
                    }
                } else {
                    lines.push(Out::Ok(format!("{}", v)));
                }
                lines
            }
            Err(e) => vec![Out::Err(format!("✗ {}", e))],
        },
        Some("status") => match client.get_update_status().await {
            Ok(v) => {
                let mut lines = vec![Out::Ok("Update Status:".into())];
                let checked = v["last_checked"].as_str().unwrap_or("never");
                lines.push(Out::Text(format!("  Last checked: {}", checked)));
                if let Some(comps) = v["components"].as_array() {
                    for c in comps {
                        let name = c["component"].as_str().unwrap_or("?");
                        let cur = c["current_version"].as_str().unwrap_or("?");
                        let dl = if c["downloaded"].as_bool().unwrap_or(false) { " [downloaded]" } else { "" };
                        lines.push(Out::Text(format!("  {:<20} v{}{}", name, cur, dl)));
                    }
                }
                lines
            }
            Err(e) => vec![Out::Err(format!("✗ {}", e))],
        },
        Some("download") => match client.download_updates().await {
            Ok(v) => {
                let msg = v.get("message").and_then(|m| m.as_str()).unwrap_or("Download initiated");
                vec![Out::Ok(format!("✓ {}", msg))]
            }
            Err(e) => vec![Out::Err(format!("✗ {}", e))],
        },
        Some("apply") => match client.apply_updates().await {
            Ok(v) => {
                let msg = v.get("message").and_then(|m| m.as_str()).unwrap_or("Updates applied");
                vec![Out::Ok(format!("✓ {}", msg))]
            }
            Err(e) => vec![Out::Err(format!("✗ {}", e))],
        },
        Some("config") => match client.get_update_config().await {
            Ok(v) => {
                let mut lines = vec![Out::Ok("Updater Config:".into())];
                if let Some(map) = v.as_object() {
                    for (k, val) in map {
                        lines.push(Out::Text(format!("  {}: {}", k, val)));
                    }
                }
                lines
            }
            Err(e) => vec![Out::Err(format!("✗ {}", e))],
        },
        Some("install") => {
            // update install [component_key]
            if args.len() >= 2 {
                let key = args[1];
                match client.install_component(key).await {
                    Ok(v) => {
                        let msg = v.get("message").and_then(|m| m.as_str()).unwrap_or("Install initiated");
                        vec![Out::Ok(format!("✓ {}", msg))]
                    }
                    Err(e) => vec![Out::Err(format!("✗ {}", e))],
                }
            } else {
                // 전체 설치
                match client.run_install(None).await {
                    Ok(v) => {
                        let msg = v.get("message").and_then(|m| m.as_str()).unwrap_or("Full install initiated");
                        vec![Out::Ok(format!("✓ {}", msg))]
                    }
                    Err(e) => vec![Out::Err(format!("✗ {}", e))],
                }
            }
        }
        Some("install-status") => match client.get_install_status().await {
            Ok(v) => {
                let mut lines = vec![Out::Ok("Install Status:".into())];
                if let Some(comps) = v["components"].as_array() {
                    for c in comps {
                        let name = c["key"].as_str().unwrap_or("?");
                        let installed = c["installed"].as_bool().unwrap_or(false);
                        let sym = if installed { "✓" } else { "✗" };
                        lines.push(Out::Text(format!("  {} {}", sym, name)));
                    }
                } else {
                    lines.push(Out::Text(format!("{}", v)));
                }
                lines
            }
            Err(e) => vec![Out::Err(format!("✗ {}", e))],
        },
        Some("install-progress") => match client.get_install_progress().await {
            Ok(v) => {
                let complete = v["complete"].as_bool().unwrap_or(false);
                let done = v["done"].as_u64().unwrap_or(0);
                let total = v["total"].as_u64().unwrap_or(0);
                let current = v["current_component"].as_str().unwrap_or("-");
                let mut lines = vec![Out::Ok(format!(
                    "Install progress: {}/{} {}",
                    done, total, if complete { "(complete)" } else { "" }
                ))];
                if !complete {
                    lines.push(Out::Text(format!("  Currently: {}", current)));
                }
                if let Some(errs) = v["errors"].as_array() {
                    for e in errs {
                        lines.push(Out::Err(format!("  {}", e.as_str().unwrap_or("?"))));
                    }
                }
                lines
            }
            Err(e) => vec![Out::Err(format!("✗ {}", e))],
        },
        _ => vec![
            Out::Text("  update check             check for updates".into()),
            Out::Text("  update status            show update status".into()),
            Out::Text("  update download          download available updates".into()),
            Out::Text("  update apply             apply downloaded updates".into()),
            Out::Text("  update config            show updater configuration".into()),
            Out::Text("  update install [key]     install component (or all)".into()),
            Out::Text("  update install-status    show installation status".into()),
            Out::Text("  update install-progress  show install progress".into()),
        ],
    }
}

/// 모듈의 사용 가능한 명령어 목록 표시
fn show_module_commands(registry: &ModuleRegistry, module_name: &str) -> Vec<Out> {
    let module = match registry.get_module(module_name) {
        Some(m) => m,
        None => return vec![Out::Err(format!("Module '{}' not found", module_name))],
    };

    let mode_tag = module.interaction_mode.as_deref().unwrap_or("auto");
    let mut lines = vec![Out::Ok(format!(
        "{} ({}) commands: [mode: {}]",
        module.display_name, module.name, mode_tag,
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

    // ── 하트비트 등록 (백그라운드) ──
    let hb_client = app.client.clone();
    let hb_client_id = app.client_id.clone();
    tokio::spawn(async move {
        // 데몬이 준비될 때까지 최대 15초 대기
        let start = std::time::Instant::now();
        loop {
            if start.elapsed() > Duration::from_secs(15) { break; }
            let running = tokio::task::spawn_blocking(process::check_daemon_running)
                .await.unwrap_or(false);
            if running { break; }
            tokio::time::sleep(Duration::from_millis(500)).await;
        }
        // 클라이언트 등록
        if let Ok(id) = hb_client.register_client("cli").await {
            *hb_client_id.lock().unwrap() = Some(id.clone());
            // 30초마다 하트비트 전송
            loop {
                tokio::time::sleep(Duration::from_secs(30)).await;
                if hb_client.send_heartbeat(&id, None).await.is_err() {
                    break;
                }
            }
        }
    });

    // ── 업데이트 체크 (시작 10초 후 1회) ──
    let upd_buf = app.async_out.clone();
    tokio::spawn(async move {
        tokio::time::sleep(Duration::from_secs(10)).await;
        let http = reqwest::Client::builder()
            .timeout(Duration::from_secs(15))
            .build()
            .unwrap();
        let resp = http
            .post("http://127.0.0.1:57474/api/updates/check")
            .send()
            .await;
        match resp {
            Ok(r) => {
                if let Ok(data) = r.json::<serde_json::Value>().await {
                    let components = data.get("components")
                        .and_then(|v| v.as_array())
                        .cloned()
                        .unwrap_or_default();
                    let updatable: Vec<_> = components.iter()
                        .filter(|c| c.get("update_available").and_then(|v| v.as_bool()).unwrap_or(false))
                        .collect();
                    if !updatable.is_empty() {
                        let names: Vec<&str> = updatable.iter()
                            .filter_map(|c| c.get("name").and_then(|v| v.as_str()))
                            .collect();
                        push_out(&upd_buf, vec![
                            Out::Info(format!(
                                "📦 {} update(s) available: {}",
                                updatable.len(),
                                names.join(", ")
                            )),
                            Out::Info("   Run 'update check' for details.".into()),
                        ]);
                    }
                }
            }
            Err(_) => { /* 데몬 미응답 — 무시 */ }
        }
    });

    // 백그라운드 상태 모니터 (refresh_interval 적용)
    let (tx, mut rx) = mpsc::channel::<Snapshot>(1);
    let base = "http://127.0.0.1:57474".to_string();
    let refresh_secs = app.settings.refresh_interval;

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

            tokio::time::sleep(Duration::from_secs(refresh_secs)).await;
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

fn cmd_sabachan() -> Vec<Out> {
    let art = r#"
                                             -~,,~~
                                          ,-,      ,~:,  ~~-
                                       .:!-          .*-!  .~,
                                       =,            .--;     :
                                   ;!; $.            ,--       ,,
                                 -~   --,     .,,    ---         ~
                           :-,,,-:~.  --~    .,,.-   ~--          -
                        --,    .--~::,.,~,   -,. ~, .---  ....    .~
                      .-   ,--*$   .:;:--~   -, .-~.--,~  ,  ,    .@-
                    .-  .,!. .==      !=~-~ ,-~-,..  .~~ .. .,    -. ;
                   ~ ..~~   ~. ..   .,,,*,        -!;-   -..,,  ,,-   ~
                  ~  ~,     ,   -,,,,,, ,:-,..   ,.  ..:-  ,, ,,,-     ~
                .-.,,      ~    --,,:,,-.       .---.   ,~, .~,,,-      :
               .-.~-      -.    ,-,:,~,.,,.    .,,,-~-    -~  ---,      .,
               ~,:        *     .-:~:.,--. .        .-~     ;, ,~.       ;
              !,-        .-      -*-..~,              ,~     -- -. ,,.    .
             :,.         ~~,,,,,.=,..~.  .             .~     .~ ,,,,     =
            ,~,         ~ -;,,,-*, .~   ..              .,     .: ,,.     !
            ;-         -.  -;--!-  -.   ...        .     ,.     ,: ~,,,.  :.
           ~:          :   --:;-  .,   ...         -,     ,      ,: -,,-:--:
           !          --   ,-:-,  ,    .. .         !      ,      -,.,,-   .-
          ;-          !,----~:-  ..    .. .         *~ .   .       * ,-.    -
          !           :,----:-,  -    . . ,         :!      ,       ; -,    -
         ~,           ~-----~-        ~ . ,         ,~: .           -,..    :
         !            -----~-,        ~ . ,          :~. .   .       ; --,  ;
                      ,@*::~~.  .     : . .        . :,; .           --,---.;:
                      :-~:;--  ;.     ; . .        , ~.,- .   ,       ~ ;---::
        ;             ~---~-.  #;    ,;   .        , - .:. . .,       -.---,-,
        *             ------...#@..:@@*   .        , ,  .: . . .       ;,~,.~ :
       .:             ~--:--. ~@@#@=@=!  ..        ,.,.  ,- . .-       - !,,: :
       ,!             ~--:-., $!$=;-~--.  ..   .   .,-:~-,-.. ,-       -,!**~ -
       ,!             :-~~~.. ;-.!  ~, .   -   ,   .~,, .,-;-,,-.       !~~.  ,,
       ,~             !,~-~ . .  $..:  , . ~   ,   .~.,     :.,-, .     ;,~   .*
       .-.            ;,*-- . .  !:-, ,;,. ..  ,   .:.,      !.-, .     -.~  ..*
        *!             #=-,.. ..  ;~~: ~..- -  ,.  ,:,-       ;-~       .-: . -;
        .::           ~!!-,.. ..  ;;.   , ; :. .-..,:--       .::       .:;....:.
         .-~..        :-:-... .  ,*~    ~ :.:,..!..,-~-,,..    ~;       .!; ...:.
                      ! ;-... ..,.!~    ,,-:-~..*.,~,~:,-:!**=. !-      .!:. -,!.
                     .~ ;- .. .,,.~~     --=~;--~~,, ,,:@#$$#@@;~-      .;:- ,,#.
                     !, :- .. .--~::  .,--:!:,.       -$:--,,:#@=~      .-:--,!#
                     !  :- .. .:-!;-.-*=!-..          :,=**$;.,$#!      ..!--~$@#$====;
                     ; .:-  ,  ;-!,~$#@@@$;.          .  -!!$. ,$=.       ##$#$=*==*!;;;!;
                     . -:-  ,  :-~~$@!~:=#$-           .  **==  :$;.      :-,~!$#$=!:;:::;!~
                   ~$!*$:-  -  -:!$@;, ,,!$=           =:!**=#  ,*        *~,...-;***;:::::*,
                 ~=!$~$@~-  -  .;~@=, -  !!#.          #~!=::#  ,:        *,--,,,,,-:$*;::;@.
               -$!;!*#@@~-  ~  .*;@:  #~:*==~          *~,.,~$  .,.       !,.,-...,----$*;#.
              $*!!=$:@@@--. ~.  #*#,  #!;$=:*          .;. .=    ,.    .. !- ..,   .-#@@#*=
            .$!*$$$$~@@#--. -~  ;=#,  #:~!:-!           ,~;*-.   -,    .. !.,  .. ,*-$@@@$
             !*$$$$#:@#$--..-:. :!!-  ~;-  -~                    ~-   .., *.,   .;!:* :##,
             .##$$$#:#*$--  ,:: .*,-   ;:,-!                    ,-~   . , *.., ,*;::!:
               ##$$#:!*=-- . ::~ =-     ~,                      ~,~   ..,,*..,:@*:::~*$-
                #$#*#*$!-~ , ~~:. *                .,.          ::.  ....:*,:!@@$;::~~!=$
                =***@*;!-~ - -~~@,-,          .:----~;         :-*   .., ==. #@@#;~~:-:!!#!
               ****$!;;!-~ -.,~~#;:~.         !:-,,,,~,       -::* ..,.,.#: .@@@@;: :, !;;$*
             ~!$$=*;;;;!-~ .,.:~#:**!-        :-,....,,      -:~~= ...,..$- ~@!$@;;..-  !;;==.
          -!=***=@#$!;;!-~ .,.~~$~-;.,.       .-.....,.        ,=*..,.,.-!. ;=;;@!:- -. ,!;;$*
          ***=!~=!!!;!##-: .,.,~$~~=,          ,.....,        .*!;....,.!:. ;;;;#*:~  -  -*$,
          :*=~-$-=;!;#@@-: .,..~$#, ;           ~.....       .=.;-.,.,..==  *;;;!=::  ,..,.:
           ;~~=.!;;;!@#=-*...,.,!   .:           ..         := ,;,...-.,;$ ,!;;;;$::.  , ; ..
          -:~:  *~;;#@!=~$-..,..:    ,;                   ~:,.. ;,,.-,.~.; ;;;;;;$::-  ,.,# -.
          ;:~  :::;!@=!$~#*..,-.~      ~~,              ,*,    -:-..:..:.  ;*;;;;$::~   , $* -.
         !:-   !~;;=#**$:$$,..~,-.       ,:--,.      ..,,~  .~;:-,.-!.- ;   :=!;;*:::   , ,=: -
        ::-   .~~;;@***=!*~~..---.            !#*;~---,,,-     -~.,;~,, !     *=;;;~:,  .. @.:
       ,~~    =~~;*=****$: =,.,~,.            ,;~-----,,,;     ~-.~;,~ -     -::#;!~~~   , ;, ,
      .;!    .:~:;=*****#. $:,.~,*-  ,~      ;!*-----:;!:~~:   :,-!,!  :     -:::=*:-~   ,  *
      .!     --~;;$$****#  :*~,--*-~;--~    :~--;:;;~~-----:,  ~-; =.-:~-~,  -::::;:~~.  .. !
             =~~;;;;*==*$   =::,;~~--~--:  ~---~#;;$~-------! !;~ ~!:,. ;:*  ,:::::::,,   , .
            ,:~~;!::::!=$.  :!=;:-:---:-!,~---~$:-~#=-------~=*  :!~,-.-::;. .:~;:::: -   ,
            ~-~::;:::::;$!  :@--!=!---:;*$!---$@;-:#@!,   .~*#...!~~-. ::::- .;~;::~:.-.
            ;~~;;;::~::;! ;:$@~--,**--!--*~-!@@#=!@@@@,   .~=@@*=~-.   @;::~  ;~!::~~~ ,
           .:~~;!:::~::;;  ##~:----:;;--:@#@@@$:!;::*@#.  -#@@:-~--   :@@!:!  !~!::~~~ .
           ~-~~;;::~~::;~  @*   .,--:~--$@@@$;--;=~~~~=#~;@@@$--~--   $@@@:!-:$~!:::-~. .
           .~-:;:::~:::!. =@=     -~;--!@@=;~~~;@@:----!@@@@@,,~----~!@@@$*---$~!:::~~,
             .;;::~~:::*  @@@    --:~--#@!~~~;=#@@;:,...:@@@!-.:-~-- ;@@$;$~-:!~!~~~: ~
             .;:::~~:::* .@@@.   .-:  *@@!--!*;@@@:-!.  $$** ..~~.   !@#!!*;-;:~;~~~;
               :;~~~:::: -@@@:   .,, .@@@@,!~.*@@@*.~#.=$!!:.  ,.,   =@=!!!$-;;~::~~:
                :~~~:::: $@*. ,-~~;  !=!!=$. .*!~~:..#@@!!* ,  ,~.   $#!!!!=::~~::~~
                .~~:::;- @@    .,,-  ~   ,~. :.   ,,.:$:~~~.,  -    .@*!!!!!*;.:~:~.
                 .-:::* -@#     ,-, --,, ~..:,....~~ .=    .  ~.    *=!!!!!!=! !~:.
                   ~::!.#@@.    -!  :   ;- -.      ;..;.   .  ~-.  .@*!!!!!!!= =-
                    .:;@#@@,   .-; ,    ,--~       ~:  :-,..  ~--, #@@@*!!!!!!$-
                      #**@@=  ,--, ,   .            , ..      ,--,,@@@@=!!!!!!
                       :*#@@-,,--,,                          ., . -@@#=!!!!!~
                        .;@@=,,--,,                          :-.  :@$*!!!!:.
                          ,=@-  ,.,                .         *-, -@@!!!!:.
                            ,;  -..               .          !,,;@@=!!:.
                                ~..                          ~-#@@@!;
                                                            .#@@@$
                                                            -@--
"#;
    art.lines().map(|line| Out::Text(line.to_string())).collect()
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
            client_id: std::sync::Arc::new(std::sync::Mutex::new(None)),
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
