//! 화면 상태와 키 처리. 그리기·터미널과 떨어져 있어 시험에서 키를 눌러 볼 수 있다

use ratatui::crossterm::event::{KeyCode, KeyEvent, KeyModifiers, MouseButton, MouseEvent, MouseEventKind};
use ratatui::layout::Rect;
use ratatui::widgets::ListState;
use serde_json::Value;
use std::collections::{BTreeMap, VecDeque};

/// 결과 창에 남기는 최대 줄 수
const MAX_LINES: usize = 2000;

/// 메뉴 한 칸 = 맨 위 명령 하나 (clap 명령 트리에서 만든다)
#[derive(Debug, Clone)]
pub struct MenuItem {
    pub name: String,
    pub about: String,
    /// 하위 명령 (이름, 설명, 꼭 줘야 하는 인자가 있는가)
    pub subs: Vec<Sub>,
    /// 하위 명령 없이 바로 실행할 때 꼭 줘야 하는 인자가 있는가
    pub needs_args: bool,
}

#[derive(Debug, Clone)]
pub struct Sub {
    pub name: String,
    pub about: String,
    pub needs_args: bool,
}

/// 메뉴에 두지 않는 명령: 화면을 따로 차지하거나(셸, 자동완성 스크립트) 메뉴 자체와 같은 것
pub const NOT_IN_MENU: &[&str] = &["shell", "tui", "help", "completions", "install"];

/// 메뉴 = 명령 표. 새 명령을 더하면 메뉴에도 생긴다
pub fn menu_from(cmd: &clap::Command) -> Vec<MenuItem> {
    let needs = |c: &clap::Command| c.get_arguments().any(|a| a.is_required_set() && !a.is_global_set());
    cmd.get_subcommands()
        .filter(|c| !c.is_hide_set() && !NOT_IN_MENU.contains(&c.get_name()))
        .map(|c| MenuItem {
            name: c.get_name().to_string(),
            about: c.get_about().map(|a| a.to_string()).unwrap_or_default(),
            needs_args: needs(c),
            subs: c
                .get_subcommands()
                .filter(|s| !s.is_hide_set() && s.get_name() != "help")
                .map(|s| Sub {
                    name: s.get_name().to_string(),
                    about: s.get_about().map(|a| a.to_string()).unwrap_or_default(),
                    needs_args: needs(s),
                })
                .collect(),
        })
        .collect()
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Focus {
    Menu,
    List,
}

/// 진행 표시 하나 (아래 작업 줄)
#[derive(Debug, Clone, Default)]
pub struct Job {
    pub message: String,
    pub position: u64,
    pub length: Option<u64>,
}

/// 묻는 창 종류
#[derive(Debug, Clone, PartialEq)]
pub enum AskKind {
    Confirm { default: bool },
    Select { items: Vec<String> },
    Text { allow_empty: bool, secret: bool },
}

/// 묻는 창의 답. None은 취소
#[derive(Debug, Clone, PartialEq)]
pub enum Answer {
    Yes(bool),
    Index(usize),
    Text(String),
}

pub struct Dialog {
    pub question: String,
    pub kind: AskKind,
    /// 고른 칸 (예/아니오는 0=예, 1=아니오)
    pub cursor: usize,
    pub input: String,
    pub reply: std::sync::mpsc::Sender<Option<Answer>>,
}

/// 키를 누른 결과 중 화면 밖에서 할 일
#[derive(Debug, Clone, PartialEq)]
pub enum Action {
    /// 명령 한 줄을 실행 (앞에 `saba-chan-cli`를 붙이지 않은 단어들)
    Run(String),
    /// 실행 중인 명령을 멈춤
    Cancel,
    Quit,
}

/// 서버 목록 한 줄에서 고를 수 있는 동작 (같은 뜻의 명령)
pub fn server_actions(name: &str) -> Vec<(String, String)> {
    let q = shlex::try_quote(name).map(|c| c.into_owned()).unwrap_or_else(|_| name.to_string());
    [
        ("start", format!("start {q}")),
        ("stop", format!("stop {q}")),
        ("restart", format!("restart {q}")),
        ("console", format!("console {q} --print")),
        ("info", format!("server info {q}")),
        ("backup_create", format!("backup create {q}")),
        ("backup_list", format!("backup list {q}")),
    ]
    .into_iter()
    .map(|(k, cmd)| (t!(&format!("tui.action.{k}")), cmd))
    .collect()
}

pub struct App {
    pub menu: Vec<MenuItem>,
    pub menu_state: ListState,
    pub list_state: ListState,
    pub focus: Focus,
    /// 데몬 상태 (`GET /api/health`). None이면 꺼짐
    pub health: Option<Value>,
    /// `GET /api/servers`의 서버들
    pub servers: Vec<Value>,
    /// 결과 창 줄
    pub lines: VecDeque<String>,
    /// 결과 창을 아래에서 몇 줄 올려 보고 있는가
    pub scroll: usize,
    /// 마지막으로 실행한 명령 (결과 창 제목)
    pub last_command: Option<String>,
    /// `:` 명령 입력 줄
    pub input: Option<String>,
    pub history: Vec<String>,
    history_pos: Option<usize>,
    /// 서버 동작 고르기 창 (서버 이름, 고른 칸)
    pub actions: Option<(String, usize)>,
    pub dialog: Option<Dialog>,
    pub jobs: BTreeMap<u64, Job>,
    pub running: bool,
    pub show_help: bool,
    /// 마지막으로 그린 메뉴·목록 자리 (마우스)
    pub menu_area: Rect,
    pub list_area: Rect,
}

impl App {
    pub fn new(menu: Vec<MenuItem>) -> Self {
        let mut menu_state = ListState::default();
        menu_state.select(Some(0));
        let mut list_state = ListState::default();
        list_state.select(Some(0));
        Self {
            menu,
            menu_state,
            list_state,
            focus: Focus::Menu,
            health: None,
            servers: Vec::new(),
            lines: VecDeque::new(),
            scroll: 0,
            last_command: None,
            input: None,
            history: Vec::new(),
            history_pos: None,
            actions: None,
            dialog: None,
            jobs: BTreeMap::new(),
            running: false,
            show_help: false,
            menu_area: Rect::default(),
            list_area: Rect::default(),
        }
    }

    pub fn current(&self) -> &MenuItem {
        &self.menu[self.menu_state.selected().unwrap_or(0).min(self.menu.len() - 1)]
    }

    /// 지금 메뉴가 서버 목록을 보이는가
    pub fn shows_servers(&self) -> bool {
        self.current().name == "server"
    }

    /// 오른쪽 목록의 줄 수
    pub fn list_len(&self) -> usize {
        if self.shows_servers() {
            self.servers.len()
        } else {
            self.current().subs.len().max(1)
        }
    }

    pub fn push_line(&mut self, line: &str) {
        for l in line.split('\n') {
            self.lines.push_back(l.to_string());
        }
        while self.lines.len() > MAX_LINES {
            self.lines.pop_front();
        }
    }

    fn move_menu(&mut self, down: bool) {
        let n = self.menu.len();
        let i = self.menu_state.selected().unwrap_or(0);
        self.menu_state.select(Some(if down { (i + 1) % n } else { (i + n - 1) % n }));
        self.list_state.select(Some(0));
    }

    fn move_list(&mut self, down: bool) {
        let n = self.list_len().max(1);
        let i = self.list_state.selected().unwrap_or(0).min(n - 1);
        self.list_state.select(Some(if down { (i + 1) % n } else { (i + n - 1) % n }));
    }

    /// 오른쪽 목록에서 Enter: 서버면 동작 고르기, 명령이면 실행하거나 인자를 채울 줄을 연다
    fn activate(&mut self) -> Option<Action> {
        let item = self.current().clone();
        if self.shows_servers() {
            let i = self.list_state.selected().unwrap_or(0);
            if let Some(name) = self.servers.get(i).and_then(|s| s.get("name")).and_then(Value::as_str) {
                self.actions = Some((name.to_string(), 0));
            }
            return None;
        }
        let (path, needs_args) = match item.subs.get(self.list_state.selected().unwrap_or(0)) {
            Some(sub) => (format!("{} {}", item.name, sub.name), sub.needs_args),
            None => (item.name.clone(), item.needs_args),
        };
        if needs_args {
            self.input = Some(format!("{path} "));
            None
        } else {
            Some(Action::Run(path))
        }
    }

    pub fn on_key(&mut self, key: KeyEvent) -> Option<Action> {
        let ctrl = key.modifiers.contains(KeyModifiers::CONTROL);
        if self.dialog.is_some() {
            self.dialog_key(key);
            return None;
        }
        if ctrl && key.code == KeyCode::Char('c') {
            // 쓰던 줄이 있으면 그 줄만 닫는다
            if self.input.take().is_some() {
                self.history_pos = None;
                return None;
            }
            return Some(if self.running { Action::Cancel } else { Action::Quit });
        }
        if self.show_help {
            self.show_help = false;
            return None;
        }
        if let Some(input) = self.input.as_mut() {
            match key.code {
                KeyCode::Esc => self.input = None,
                KeyCode::Enter => {
                    let line = input.trim().to_string();
                    self.input = None;
                    self.history_pos = None;
                    if !line.is_empty() {
                        self.history.push(line.clone());
                        return Some(Action::Run(line));
                    }
                }
                KeyCode::Backspace => {
                    input.pop();
                }
                KeyCode::Up | KeyCode::Down if !self.history.is_empty() => {
                    let n = self.history.len();
                    let pos = match (self.history_pos, key.code) {
                        (None, KeyCode::Up) => n - 1,
                        (Some(p), KeyCode::Up) => p.saturating_sub(1),
                        (Some(p), _) if p + 1 < n => p + 1,
                        _ => {
                            self.history_pos = None;
                            *input = String::new();
                            return None;
                        }
                    };
                    self.history_pos = Some(pos);
                    *input = self.history[pos].clone();
                }
                KeyCode::Char(c) if !ctrl => input.push(c),
                _ => {}
            }
            return None;
        }
        if let Some((name, cursor)) = self.actions.as_mut() {
            let list = server_actions(name);
            match key.code {
                KeyCode::Esc | KeyCode::Left => self.actions = None,
                KeyCode::Up => *cursor = (*cursor + list.len() - 1) % list.len(),
                KeyCode::Down => *cursor = (*cursor + 1) % list.len(),
                KeyCode::Enter => {
                    let cmd = list[*cursor].1.clone();
                    self.actions = None;
                    return Some(Action::Run(cmd));
                }
                _ => {}
            }
            return None;
        }
        match key.code {
            KeyCode::Char('q') if !self.running => return Some(Action::Quit),
            KeyCode::Char(':') => self.input = Some(String::new()),
            KeyCode::Char('?') => self.show_help = true,
            KeyCode::Tab => self.focus = if self.focus == Focus::Menu { Focus::List } else { Focus::Menu },
            KeyCode::Left => self.focus = Focus::Menu,
            KeyCode::Right => self.focus = Focus::List,
            KeyCode::Up if self.focus == Focus::Menu => self.move_menu(false),
            KeyCode::Down if self.focus == Focus::Menu => self.move_menu(true),
            KeyCode::Up => self.move_list(false),
            KeyCode::Down => self.move_list(true),
            KeyCode::PageUp => self.scroll = (self.scroll + 10).min(self.lines.len()),
            KeyCode::PageDown => self.scroll = self.scroll.saturating_sub(10),
            KeyCode::Enter if self.focus == Focus::Menu => self.focus = Focus::List,
            KeyCode::Enter if !self.running => return self.activate(),
            _ => {}
        }
        None
    }

    fn dialog_key(&mut self, key: KeyEvent) {
        let Some(d) = self.dialog.as_mut() else { return };
        let ctrl = key.modifiers.contains(KeyModifiers::CONTROL);
        let answer: Option<Option<Answer>> = match (&d.kind, key.code) {
            (_, KeyCode::Esc) => Some(None),
            (_, KeyCode::Char('c')) if ctrl => Some(None),
            (_, KeyCode::Char(_)) if ctrl => None,
            (AskKind::Confirm { .. }, KeyCode::Char('y' | 'Y')) => Some(Some(Answer::Yes(true))),
            (AskKind::Confirm { .. }, KeyCode::Char('n' | 'N')) => Some(Some(Answer::Yes(false))),
            (AskKind::Confirm { .. }, KeyCode::Left | KeyCode::Right | KeyCode::Tab) => {
                d.cursor = 1 - d.cursor.min(1);
                None
            }
            (AskKind::Confirm { .. }, KeyCode::Enter) => Some(Some(Answer::Yes(d.cursor == 0))),
            (AskKind::Select { items }, KeyCode::Up) => {
                d.cursor = (d.cursor + items.len() - 1) % items.len();
                None
            }
            (AskKind::Select { items }, KeyCode::Down) => {
                d.cursor = (d.cursor + 1) % items.len();
                None
            }
            (AskKind::Select { .. }, KeyCode::Enter) => Some(Some(Answer::Index(d.cursor))),
            (AskKind::Text { .. }, KeyCode::Backspace) => {
                d.input.pop();
                None
            }
            (AskKind::Text { allow_empty, .. }, KeyCode::Enter) if *allow_empty || !d.input.is_empty() => {
                Some(Some(Answer::Text(d.input.clone())))
            }
            (AskKind::Text { .. }, KeyCode::Char(c)) => {
                d.input.push(c);
                None
            }
            _ => None,
        };
        if let Some(a) = answer {
            if let Some(d) = self.dialog.take() {
                let _ = d.reply.send(a);
            }
        }
    }

    pub fn on_mouse(&mut self, m: MouseEvent) -> Option<Action> {
        if self.dialog.is_some() || self.input.is_some() || self.actions.is_some() {
            return None;
        }
        let inside = |r: Rect| m.column >= r.x && m.column < r.x + r.width && m.row > r.y && m.row + 1 < r.y + r.height;
        match m.kind {
            MouseEventKind::Down(MouseButton::Left) if inside(self.menu_area) => {
                let i = (m.row - self.menu_area.y - 1) as usize + self.menu_state.offset();
                if i < self.menu.len() {
                    self.menu_state.select(Some(i));
                    self.list_state.select(Some(0));
                    self.focus = Focus::Menu;
                }
            }
            MouseEventKind::Down(MouseButton::Left) if inside(self.list_area) => {
                let i = (m.row - self.list_area.y - 1) as usize + self.list_state.offset();
                if i < self.list_len() {
                    let again = self.focus == Focus::List && self.list_state.selected() == Some(i);
                    self.list_state.select(Some(i));
                    self.focus = Focus::List;
                    // 고른 줄을 한 번 더 누르면 Enter
                    if again && !self.running {
                        return self.activate();
                    }
                }
            }
            MouseEventKind::ScrollUp => self.scroll = (self.scroll + 3).min(self.lines.len()),
            MouseEventKind::ScrollDown => self.scroll = self.scroll.saturating_sub(3),
            _ => {}
        }
        None
    }
}

/// 한 줄을 명령 표로 읽는다. 전역 옵션이 앞에 오거나 별칭으로 불러도 같은 명령으로 본다
fn parsed(words: &[String]) -> Option<crate::cli::Cli> {
    use clap::FromArgMatches;
    let args = std::iter::once("saba-chan-cli".into())
        .chain(words.iter().map(Into::into))
        .collect();
    let m = crate::cli::parse(args).ok()?;
    crate::cli::Cli::from_arg_matches(&m).ok()
}

/// TUI 안에서 바로 돌릴 수 없는 명령이면 그 까닭. 화면을 통째로 쓰거나(셸, 화면 지우기) 프로그램을 끝낸다
pub fn refuse(words: &[String]) -> Option<String> {
    use crate::cli::{Cmd, UpdateCmd};
    match parsed(words)?.cmd? {
        Cmd::Shell | Cmd::Tui | Cmd::Completions { .. } | Cmd::Install(_) => Some(t!("tui.not_here")),
        Cmd::Monitor(a) if a.watch => Some(t!("tui.not_here")),
        Cmd::Update(UpdateCmd::Apply { .. }) => Some(t!("tui.not_here")),
        _ => None,
    }
}

/// TUI에서 쓰도록 고친 명령: 콘솔은 입력 줄을 열지 않고 따라 읽는다 (명령은 `run`으로 보낸다)
pub fn adapt(mut words: Vec<String>) -> Vec<String> {
    use crate::cli::{Cmd, ServerCmd};
    let Some(cli) = parsed(&words) else { return words };
    let console = match cli.cmd {
        Some(Cmd::Console(a)) | Some(Cmd::Server(ServerCmd::Console(a))) => Some(a),
        _ => None,
    };
    if console.is_some_and(|a| !a.follow && !a.print) && !cli.global.json && !cli.global.json_stream {
        words.push("--follow".into());
    }
    words
}

#[cfg(test)]
mod tests {
    use super::*;
    use clap::CommandFactory;
    use ratatui::crossterm::event::KeyEventState;
    use serde_json::json;

    fn key(code: KeyCode) -> KeyEvent {
        KeyEvent { code, modifiers: KeyModifiers::NONE, kind: ratatui::crossterm::event::KeyEventKind::Press, state: KeyEventState::NONE }
    }

    fn app() -> App {
        crate::i18n::init("ko");
        App::new(menu_from(&crate::cli::localized()))
    }

    /// 메뉴 = 명령 표: 메뉴 밖으로 뺀 몇 개 말고는 모든 맨 위 명령이 메뉴에 있다
    #[test]
    fn every_command_has_a_menu_entry() {
        let a = app();
        let names: Vec<&str> = a.menu.iter().map(|m| m.name.as_str()).collect();
        for c in crate::cli::Cli::command().get_subcommands() {
            if !NOT_IN_MENU.contains(&c.get_name()) {
                assert!(names.contains(&c.get_name()), "{} is missing from the menu", c.get_name());
            }
        }
        let backup = a.menu.iter().find(|m| m.name == "backup").unwrap();
        assert!(backup.subs.iter().any(|s| s.name == "restore" && s.needs_args));
    }

    /// 마우스 없이 키보드만으로 모든 메뉴와 하위 명령에 닿는다
    #[test]
    fn every_menu_entry_is_reachable_by_keyboard() {
        let mut a = app();
        let mut seen = std::collections::BTreeSet::new();
        for _ in 0..a.menu.len() {
            seen.insert(a.current().name.clone());
            a.on_key(key(KeyCode::Down));
        }
        assert_eq!(seen.len(), a.menu.len());

        // backup → 목록으로 → restore 줄에서 Enter: 인자가 필요하니 명령 줄이 채워진다
        while a.current().name != "backup" {
            a.on_key(key(KeyCode::Down));
        }
        a.on_key(key(KeyCode::Enter));
        assert_eq!(a.focus, Focus::List);
        let at = a.current().subs.iter().position(|s| s.name == "restore").unwrap();
        for _ in 0..at {
            a.on_key(key(KeyCode::Down));
        }
        assert_eq!(a.on_key(key(KeyCode::Enter)), None);
        assert_eq!(a.input.as_deref(), Some("backup restore "));
        for c in "pal".chars() {
            a.on_key(key(KeyCode::Char(c)));
        }
        assert_eq!(a.on_key(key(KeyCode::Enter)), Some(Action::Run("backup restore pal".into())));

        // 인자가 필요 없는 명령은 바로 실행
        while a.current().name != "status" {
            a.focus = Focus::Menu;
            a.on_key(key(KeyCode::Down));
        }
        a.on_key(key(KeyCode::Right));
        assert_eq!(a.on_key(key(KeyCode::Enter)), Some(Action::Run("status".into())));
    }

    /// 서버 목록에서 Enter → 동작 고르기 → 같은 뜻의 명령을 실행 (이름에 공백이 있어도 한 단어로)
    #[test]
    fn a_server_action_runs_the_same_command() {
        let mut a = app();
        a.servers = vec![json!({ "name": "my server", "status": "stopped" })];
        while !a.shows_servers() {
            a.on_key(key(KeyCode::Down));
        }
        a.on_key(key(KeyCode::Right));
        a.on_key(key(KeyCode::Enter));
        assert_eq!(a.actions.as_ref().map(|(n, _)| n.as_str()), Some("my server"));
        assert_eq!(a.on_key(key(KeyCode::Enter)), Some(Action::Run("start 'my server'".into())));
    }

    /// 묻는 창은 키로 답하고 Esc는 취소. 명령이 돌 때 Ctrl+C는 끝내지 않고 멈춘다
    #[test]
    fn a_dialog_is_answered_by_keys_and_ctrl_c_cancels_a_running_command() {
        let mut a = app();
        let (tx, rx) = std::sync::mpsc::channel();
        a.dialog = Some(Dialog { question: "?".into(), kind: AskKind::Confirm { default: false }, cursor: 1, input: String::new(), reply: tx.clone() });
        a.on_key(key(KeyCode::Left));
        a.on_key(key(KeyCode::Enter));
        assert_eq!(rx.recv().unwrap(), Some(Answer::Yes(true)));
        a.dialog = Some(Dialog { question: "?".into(), kind: AskKind::Text { allow_empty: false, secret: true }, cursor: 0, input: String::new(), reply: tx });
        a.on_key(key(KeyCode::Enter));
        assert!(a.dialog.is_some(), "empty text is not an answer");
        a.on_key(key(KeyCode::Esc));
        assert_eq!(rx.recv().unwrap(), None);

        let ctrl_c = KeyEvent { modifiers: KeyModifiers::CONTROL, ..key(KeyCode::Char('c')) };
        a.running = true;
        assert_eq!(a.on_key(ctrl_c), Some(Action::Cancel));
        assert_eq!(a.on_key(key(KeyCode::Char('q'))), None);
        a.running = false;
        assert_eq!(a.on_key(ctrl_c), Some(Action::Quit));
    }

    #[test]
    fn commands_that_take_the_whole_screen_are_refused_or_adapted() {
        crate::i18n::init("ko");
        let w = |s: &str| shlex::split(s).unwrap();
        assert!(refuse(&w("shell")).is_some());
        assert!(refuse(&w("monitor --watch")).is_some());
        assert!(refuse(&w("update apply")).is_some());
        assert!(refuse(&w("install --dir x")).is_some());
        assert!(refuse(&w("monitor")).is_none());
        assert_eq!(adapt(w("console pal")), w("console pal --follow"));
        assert_eq!(adapt(w("console pal --print")), w("console pal --print"));
    }

    /// Ctrl+C는 대화상자에서는 취소, 입력 줄에서는 줄 닫기다 (TUI를 끝내지 않는다)
    #[test]
    fn ctrl_c_cancels_the_dialog_or_the_input_line() {
        let ctrl_c = KeyEvent {
            modifiers: KeyModifiers::CONTROL,
            ..key(KeyCode::Char('c'))
        };
        let mut a = app();
        let (reply, rx) = std::sync::mpsc::channel();
        a.dialog = Some(Dialog {
            question: "?".into(),
            kind: AskKind::Text {
                allow_empty: true,
                secret: false,
            },
            cursor: 0,
            input: String::new(),
            reply,
        });
        assert_eq!(a.on_key(ctrl_c), None);
        assert!(a.dialog.is_none());
        assert!(rx.recv().unwrap().is_none(), "취소는 답 없음");
        a.input = Some("server".into());
        assert_eq!(a.on_key(ctrl_c), None);
        assert!(a.input.is_none());
    }

    /// 전역 옵션을 앞에 두거나 별칭으로 불러도 같은 명령이다
    #[test]
    fn global_options_and_aliases_do_not_get_past_the_check() {
        crate::i18n::init("ko");
        let w = |s: &str| shlex::split(s).unwrap();
        assert!(refuse(&w("--yes update apply")).is_some());
        assert!(refuse(&w("updates apply")).is_some());
        assert!(refuse(&w("--lang en monitor -w")).is_some());
        assert!(refuse(&w("--yes update status")).is_none());
        assert_eq!(adapt(w("-y console pal")), w("-y console pal --follow"));
        assert_eq!(adapt(w("server logs pal")), w("server logs pal --follow"));
        assert_eq!(adapt(w("--json console pal")), w("--json console pal"));
    }
}
