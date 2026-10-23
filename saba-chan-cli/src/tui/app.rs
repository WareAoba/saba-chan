//! 화면 상태와 키·마우스 처리. 그리기·터미널과 떨어져 있어 시험에서 키를 눌러 볼 수 있다
//!
//! 화면은 왼쪽 메뉴(쪽), 가운데 목록과 버튼, 아래 콘솔로 나뉜다. 버튼 하나는 같은 뜻의 명령 한 줄이고,
//! 누르면 그 명령이 콘솔에 찍히며 실행된다. 그래서 버튼으로 되는 일은 콘솔에 쳐서도 된다.

use super::data::{s, Snapshot};
use ratatui::crossterm::event::{KeyCode, KeyEvent, KeyModifiers, MouseButton, MouseEvent, MouseEventKind};
use ratatui::layout::Rect;
use ratatui::widgets::ListState;
use serde_json::Value;
use std::collections::{BTreeMap, VecDeque};

/// 콘솔에 남기는 최대 줄 수
const MAX_LINES: usize = 2000;

/// 명령 표의 맨 위 명령 하나 ("모든 명령" 쪽)
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

/// "모든 명령"에 두지 않는 명령: 화면을 따로 차지하거나(셸, 자동완성 스크립트) 메뉴 자체와 같은 것
pub const NOT_IN_MENU: &[&str] = &["shell", "tui", "help", "completions", "install"];

/// "모든 명령" = 명령 표. 새 명령을 더하면 여기에도 생긴다
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

/// 왼쪽 메뉴의 쪽
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Page {
    Dashboard,
    Servers,
    Modules,
    Extensions,
    Discord,
    Updates,
    Settings,
    DaemonLog,
    Commands,
}

impl Page {
    pub const ALL: [Page; 9] = [
        Page::Dashboard,
        Page::Servers,
        Page::Modules,
        Page::Extensions,
        Page::Discord,
        Page::Updates,
        Page::Settings,
        Page::DaemonLog,
        Page::Commands,
    ];

    pub fn key(self) -> &'static str {
        match self {
            Page::Dashboard => "dashboard",
            Page::Servers => "servers",
            Page::Modules => "modules",
            Page::Extensions => "extensions",
            Page::Discord => "discord",
            Page::Updates => "updates",
            Page::Settings => "settings",
            Page::DaemonLog => "daemon_log",
            Page::Commands => "commands",
        }
    }

    pub fn title(self) -> String {
        t!(&format!("tui.page.{}", self.key()))
    }

    /// 고를 목록이 있는 쪽
    pub fn has_list(self) -> bool {
        matches!(self, Page::Servers | Page::Modules | Page::Extensions | Page::Commands)
    }
}

/// 쪽마다 버튼: (문구 키, 명령). `{}`는 목록에서 고른 것, 끝이 공백이면 콘솔 입력 줄을 채워 열어 나머지를 받는다
pub fn page_buttons(page: Page) -> &'static [(&'static str, &'static str)] {
    match page {
        Page::Dashboard => &[("status", "status"), ("monitor", "monitor"), ("server_add", "server add"), ("audit", "audit")],
        Page::Servers => &[
            ("start", "start {}"),
            ("stop", "stop {}"),
            ("restart", "restart {}"),
            ("players", "server players {}"),
            ("run", "run {} "),
            ("send", "server send {} "),
            ("info", "server info {}"),
            ("settings", "server settings {}"),
            ("backup_create", "backup create {}"),
            ("backup_list", "backup list {}"),
            ("schedule", "schedule show {}"),
            ("log", "server log {} -n 100"),
            ("validate", "server validate {}"),
            ("server_add", "server add"),
            ("remove", "server rm {}"),
        ],
        Page::Modules => &[
            ("info", "module info {}"),
            ("commands", "module commands {}"),
            ("versions", "module versions {}"),
            ("available", "module available"),
            ("refresh", "module refresh"),
            ("remove", "module remove {}"),
        ],
        Page::Extensions => &[
            ("enable", "extension enable {}"),
            ("disable", "extension disable {}"),
            ("info", "extension info {}"),
            ("config", "extension config {}"),
            ("ext_update", "extension update {}"),
            ("available", "extension available"),
            ("rescan", "extension rescan"),
        ],
        Page::Discord => &[
            ("bot_status", "bot status"),
            ("bot_pair", "bot pair"),
            ("bot_start", "bot start"),
            ("bot_stop", "bot stop"),
            ("bot_nodes", "bot nodes"),
            ("bot_channels", "bot channels"),
            ("bot_invite", "bot invite"),
            ("perm", "perm show"),
            ("chatbot", "chatbot show"),
            ("bot_config", "bot config"),
        ],
        Page::Updates => &[
            ("update_status", "update status"),
            ("update_check", "update check"),
            ("update_download", "update download"),
            ("update_last", "update last"),
            ("update_integrity", "update integrity"),
            ("update_config", "update config"),
        ],
        Page::Settings => &[
            ("config_show", "config show"),
            ("config_set", "config set "),
            ("config_defaults", "config defaults"),
            ("diagnostics", "daemon diagnostics"),
            ("version", "version"),
            ("sandbox", "sandbox list"),
            ("audit", "audit"),
        ],
        Page::DaemonLog => &[
            ("daemon_start", "daemon start"),
            ("daemon_restart", "daemon restart"),
            ("daemon_stop", "daemon stop"),
            ("clients", "daemon clients"),
            ("events", "events"),
        ],
        Page::Commands => &[],
    }
}

/// 버튼 하나 (그릴 때와 누를 때 같은 것을 쓴다)
#[derive(Debug, Clone, PartialEq)]
pub struct Button {
    pub label: String,
    /// 실행할 명령 한 줄 (앞에 `saba-chan-cli`를 붙이지 않는다)
    pub line: String,
    /// 콘솔 입력 줄을 채워 열고 나머지를 받는다
    pub fill: bool,
    /// 목록에서 고른 것이 있어야 하는데 없으면 false
    pub enabled: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Focus {
    Nav,
    List,
    Buttons,
    Console,
}

/// 진행 표시 하나 (콘솔 위 작업 줄)
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
    /// 쪽이 바뀌었다: 그 쪽에 필요한 것을 곧바로 읽는다
    Refresh,
    Quit,
}

/// 아래 콘솔: 명령 결과와 입력 줄
#[derive(Default)]
pub struct Console {
    pub lines: VecDeque<String>,
    /// 아래에서 몇 줄 올려 보고 있는가 (접힌 줄 기준)
    pub scroll: usize,
    pub input: String,
    pub history: Vec<String>,
    history_pos: Option<usize>,
    /// 크게 펼침 (F2)
    pub big: bool,
}

pub struct App {
    pub page: Page,
    pub focus: Focus,
    /// 콘솔에서 Esc로 돌아갈 곳
    back: Focus,
    pub list_state: ListState,
    pub button: usize,
    pub snap: Snapshot,
    pub menu: Vec<MenuItem>,
    pub console: Console,
    /// 마지막으로 실행한 명령 (콘솔 제목)
    pub last_command: Option<String>,
    pub dialog: Option<Dialog>,
    pub jobs: BTreeMap<u64, Job>,
    pub running: bool,
    pub show_help: bool,
    /// 마지막으로 그린 자리 (마우스)
    pub nav_area: Rect,
    pub list_area: Rect,
    pub button_areas: Vec<Rect>,
    pub console_area: Rect,
}

impl App {
    pub fn new(menu: Vec<MenuItem>) -> Self {
        let mut list_state = ListState::default();
        list_state.select(Some(0));
        Self {
            page: Page::Dashboard,
            focus: Focus::Nav,
            back: Focus::Nav,
            list_state,
            button: 0,
            snap: Snapshot::default(),
            menu,
            console: Console::default(),
            last_command: None,
            dialog: None,
            jobs: BTreeMap::new(),
            running: false,
            show_help: false,
            nav_area: Rect::default(),
            list_area: Rect::default(),
            button_areas: Vec::new(),
            console_area: Rect::default(),
        }
    }

    /// 지금 쪽 목록의 이름들 (버튼 명령의 `{}` 자리에 들어간다)
    pub fn list_names(&self) -> Vec<String> {
        match self.page {
            Page::Servers => self.snap.servers.iter().map(|v| s(v, "name").to_string()).collect(),
            Page::Modules => self.snap.modules.iter().map(|v| s(v, "name").to_string()).collect(),
            Page::Extensions => self.snap.extensions.iter().map(|v| s(v, "id").to_string()).collect(),
            Page::Commands => self.menu.iter().map(|m| m.name.clone()).collect(),
            _ => Vec::new(),
        }
    }

    pub fn selected_index(&self) -> usize {
        self.list_state.selected().unwrap_or(0)
    }

    /// 목록에서 고른 것
    pub fn selected(&self) -> Option<String> {
        self.list_names().get(self.selected_index()).cloned()
    }

    /// 서버 쪽에서 고른 서버
    pub fn selected_server(&self) -> Option<&Value> {
        (self.page == Page::Servers).then(|| self.snap.servers.get(self.selected_index())).flatten()
    }

    /// 지금 쪽의 버튼
    pub fn buttons(&self) -> Vec<Button> {
        if self.page == Page::Commands {
            let Some(item) = self.menu.get(self.selected_index()) else { return Vec::new() };
            if item.subs.is_empty() {
                return vec![Button { label: item.name.clone(), line: item.name.clone(), fill: item.needs_args, enabled: true }];
            }
            return item
                .subs
                .iter()
                .map(|s| {
                    let line = format!("{} {}", item.name, s.name);
                    Button { label: s.name.clone(), line: if s.needs_args { format!("{line} ") } else { line }, fill: s.needs_args, enabled: true }
                })
                .collect();
        }
        let picked = self.selected().map(|n| shlex::try_quote(&n).map(|q| q.into_owned()).unwrap_or(n));
        page_buttons(self.page)
            .iter()
            .map(|(key, template)| {
                let needs_item = template.contains("{}");
                let line = match &picked {
                    Some(n) => template.replace("{}", n),
                    None => template.replace("{}", ""),
                };
                Button {
                    label: t!(&format!("tui.btn.{key}")),
                    line,
                    fill: template.ends_with(' '),
                    enabled: !needs_item || picked.is_some(),
                }
            })
            .collect()
    }

    /// 결과 한 줄(여러 줄도). 터미널용 안내의 `saba-chan-cli ` 머리는 뗀다: 콘솔에는 그 뒤만 치면 된다
    pub fn push_line(&mut self, line: &str) {
        for l in line.split('\n') {
            let indent = l.len() - l.trim_start().len();
            let shown = match l.trim_start().strip_prefix("saba-chan-cli ") {
                Some(rest) => format!("{}{}", &l[..indent], rest),
                None => l.to_string(),
            };
            self.console.lines.push_back(shown);
        }
        while self.console.lines.len() > MAX_LINES {
            self.console.lines.pop_front();
        }
    }

    fn go(&mut self, page: Page) -> Option<Action> {
        if self.page == page {
            return None;
        }
        self.page = page;
        self.list_state.select(Some(0));
        self.button = 0;
        Some(Action::Refresh)
    }

    fn move_page(&mut self, down: bool) -> Option<Action> {
        let n = Page::ALL.len();
        let i = Page::ALL.iter().position(|p| *p == self.page).unwrap_or(0);
        self.go(Page::ALL[if down { (i + 1) % n } else { (i + n - 1) % n }])
    }

    fn move_list(&mut self, down: bool) -> Option<Action> {
        let n = self.list_names().len().max(1);
        let i = self.selected_index().min(n - 1);
        self.list_state.select(Some(if down { (i + 1) % n } else { (i + n - 1) % n }));
        self.button = self.button.min(self.buttons().len().saturating_sub(1));
        // 서버를 바꾸면 그 서버의 콘솔을 읽는다
        (self.page == Page::Servers).then_some(Action::Refresh)
    }

    fn focus_console(&mut self) {
        if self.focus != Focus::Console {
            self.back = self.focus;
        }
        self.focus = Focus::Console;
    }

    /// Tab 순서: 메뉴 → 목록(있으면) → 버튼(있으면) → 콘솔
    fn cycle(&mut self, forward: bool) {
        let mut order = vec![Focus::Nav];
        if self.page.has_list() {
            order.push(Focus::List);
        }
        if !self.buttons().is_empty() {
            order.push(Focus::Buttons);
        }
        order.push(Focus::Console);
        let i = order.iter().position(|f| *f == self.focus).unwrap_or(0);
        let next = order[if forward { (i + 1) % order.len() } else { (i + order.len() - 1) % order.len() }];
        if next == Focus::Console {
            self.focus_console();
        } else {
            self.focus = next;
        }
    }

    /// 버튼 누르기: 채울 것이 있으면 콘솔 입력 줄로, 아니면 바로 실행
    fn press(&mut self, i: usize) -> Option<Action> {
        let b = self.buttons().get(i)?.clone();
        if !b.enabled {
            self.push_line(&t!("tui.pick_first"));
            return None;
        }
        if b.fill {
            self.console.input = b.line;
            self.focus_console();
            return None;
        }
        self.run_line(b.line)
    }

    /// 한 줄 실행 요청. 다른 명령이 도는 중이면 버리지 않고 알린다
    fn run_line(&mut self, line: String) -> Option<Action> {
        if self.running {
            self.push_line(&t!("tui.busy"));
            return None;
        }
        Some(Action::Run(line))
    }

    pub fn on_key(&mut self, key: KeyEvent) -> Option<Action> {
        let ctrl = key.modifiers.contains(KeyModifiers::CONTROL);
        if self.dialog.is_some() {
            self.dialog_key(key);
            return None;
        }
        if ctrl && key.code == KeyCode::Char('c') {
            // 쓰던 줄이 있으면 그 줄만 지운다
            if self.focus == Focus::Console && !self.console.input.is_empty() {
                self.console.input.clear();
                self.console.history_pos = None;
                return None;
            }
            return Some(if self.running { Action::Cancel } else { Action::Quit });
        }
        if self.show_help {
            self.show_help = false;
            return None;
        }
        match key.code {
            KeyCode::F(2) => {
                self.console.big = !self.console.big;
                return None;
            }
            KeyCode::PageUp => {
                self.console.scroll += 10;
                return None;
            }
            KeyCode::PageDown => {
                self.console.scroll = self.console.scroll.saturating_sub(10);
                return None;
            }
            KeyCode::Tab => {
                self.cycle(true);
                return None;
            }
            KeyCode::BackTab => {
                self.cycle(false);
                return None;
            }
            _ => {}
        }
        if self.focus == Focus::Console {
            return self.console_key(key);
        }
        match key.code {
            KeyCode::Char('q') if !self.running => return Some(Action::Quit),
            KeyCode::Char(':') | KeyCode::Char('/') => self.focus_console(),
            KeyCode::Char('?') => self.show_help = true,
            KeyCode::Char(c @ '1'..='9') => {
                let page = Page::ALL.get(c as usize - '1' as usize).copied()?;
                self.focus = Focus::Nav;
                return self.go(page);
            }
            _ => {}
        }
        match self.focus {
            Focus::Nav => match key.code {
                KeyCode::Up => return self.move_page(false),
                KeyCode::Down => return self.move_page(true),
                KeyCode::Right | KeyCode::Enter => {
                    self.focus = if self.page.has_list() { Focus::List } else { Focus::Buttons };
                }
                _ => {}
            },
            Focus::List => match key.code {
                KeyCode::Up => return self.move_list(false),
                KeyCode::Down => return self.move_list(true),
                KeyCode::Left => self.focus = Focus::Nav,
                KeyCode::Right | KeyCode::Enter => self.focus = Focus::Buttons,
                _ => {}
            },
            Focus::Buttons => {
                let n = self.buttons().len();
                match key.code {
                    KeyCode::Left if self.button == 0 => self.focus = if self.page.has_list() { Focus::List } else { Focus::Nav },
                    KeyCode::Left => self.button -= 1,
                    KeyCode::Right if n > 0 => self.button = (self.button + 1).min(n - 1),
                    KeyCode::Up => self.focus = if self.page.has_list() { Focus::List } else { Focus::Nav },
                    KeyCode::Down => self.focus_console(),
                    KeyCode::Enter => return self.press(self.button),
                    _ => {}
                }
            }
            Focus::Console => {}
        }
        None
    }

    fn console_key(&mut self, key: KeyEvent) -> Option<Action> {
        let ctrl = key.modifiers.contains(KeyModifiers::CONTROL);
        let c = &mut self.console;
        match key.code {
            KeyCode::Esc => {
                self.focus = self.back;
            }
            KeyCode::Enter => {
                let line = c.input.trim().to_string();
                if line.is_empty() {
                    return None;
                }
                if self.running {
                    // 줄은 남겨 둔다: 지금 명령이 끝나면 다시 Enter
                    self.push_line(&t!("tui.busy"));
                    return None;
                }
                c.input.clear();
                c.history_pos = None;
                if c.history.last() != Some(&line) {
                    c.history.push(line.clone());
                }
                return Some(Action::Run(line));
            }
            KeyCode::Backspace => {
                c.input.pop();
            }
            KeyCode::Up | KeyCode::Down if !c.history.is_empty() => {
                let n = c.history.len();
                let pos = match (c.history_pos, key.code) {
                    (None, KeyCode::Up) => n - 1,
                    (Some(p), KeyCode::Up) => p.saturating_sub(1),
                    (Some(p), _) if p + 1 < n => p + 1,
                    _ => {
                        c.history_pos = None;
                        c.input.clear();
                        return None;
                    }
                };
                c.history_pos = Some(pos);
                c.input = c.history[pos].clone();
            }
            KeyCode::Char(ch) if !ctrl => c.input.push(ch),
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
        if self.dialog.is_some() {
            return None;
        }
        let inside = |r: Rect| m.column >= r.x && m.column < r.x + r.width && m.row >= r.y && m.row < r.y + r.height;
        // 테두리 안쪽 몇 번째 줄인가
        let row_in = |r: Rect| (m.row > r.y && m.row + 1 < r.y + r.height).then(|| (m.row - r.y - 1) as usize);
        match m.kind {
            MouseEventKind::Down(MouseButton::Left) => {
                if let Some(i) = self.button_areas.iter().position(|r| inside(*r)) {
                    self.focus = Focus::Buttons;
                    self.button = i;
                    return self.press(i);
                }
                if inside(self.nav_area) {
                    let page = row_in(self.nav_area).and_then(|i| Page::ALL.get(i).copied())?;
                    self.focus = Focus::Nav;
                    return self.go(page);
                }
                if inside(self.list_area) {
                    let i = row_in(self.list_area)? + self.list_state.offset();
                    if i < self.list_names().len() {
                        let again = self.focus == Focus::List && self.selected_index() == i;
                        self.list_state.select(Some(i));
                        self.focus = if again { Focus::Buttons } else { Focus::List };
                        return (self.page == Page::Servers).then_some(Action::Refresh);
                    }
                }
                if inside(self.console_area) {
                    self.focus_console();
                }
            }
            MouseEventKind::ScrollUp if inside(self.console_area) => self.console.scroll += 3,
            MouseEventKind::ScrollDown if inside(self.console_area) => self.console.scroll = self.console.scroll.saturating_sub(3),
            MouseEventKind::ScrollUp if inside(self.list_area) => return self.move_list(false),
            MouseEventKind::ScrollDown if inside(self.list_area) => return self.move_list(true),
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

    fn ctrl_c() -> KeyEvent {
        KeyEvent { modifiers: KeyModifiers::CONTROL, ..key(KeyCode::Char('c')) }
    }

    fn app() -> App {
        crate::i18n::init("ko");
        let mut a = App::new(menu_from(&crate::cli::localized()));
        a.snap.servers = vec![json!({ "name": "my server", "status": "stopped" }), json!({ "name": "pal", "status": "running" })];
        a
    }

    fn typed(a: &mut App, text: &str) {
        for c in text.chars() {
            a.on_key(key(KeyCode::Char(c)));
        }
    }

    /// "모든 명령" = 명령 표: 빼 둔 몇 개 말고는 모든 맨 위 명령이 있다
    #[test]
    fn every_command_is_on_the_all_commands_page() {
        let a = app();
        let names: Vec<&str> = a.menu.iter().map(|m| m.name.as_str()).collect();
        for c in crate::cli::Cli::command().get_subcommands() {
            if !NOT_IN_MENU.contains(&c.get_name()) {
                assert!(names.contains(&c.get_name()), "{} is missing from the menu", c.get_name());
            }
        }
    }

    /// 쪽 버튼의 명령은 모두 실제 명령이다 (명령 표가 바뀌어 버튼이 깨지지 않게)
    #[test]
    fn every_page_button_is_a_real_command() {
        crate::i18n::init("ko");
        for page in Page::ALL {
            for (key, template) in page_buttons(page) {
                let line = template.replace("{}", "x");
                let mut words = shlex::split(line.trim()).unwrap();
                // 채워 여는 버튼은 사용자가 마저 쓴다: 인자 하나를 더해 읽어 본다
                if template.ends_with(' ') {
                    words.push("y".into());
                }
                assert!(parsed(&words).is_some(), "{page:?}/{key}: `{line}` does not parse");
                assert!(crate::i18n::try_t(&format!("tui.btn.{key}")).is_some(), "tui.btn.{key} has no label");
            }
            assert!(crate::i18n::try_t(&format!("tui.page.{}", page.key())).is_some(), "{page:?} has no title");
        }
    }

    /// 마우스 없이 숫자 키·화살표·Tab만으로 모든 쪽과 버튼에 닿는다
    #[test]
    fn every_page_and_button_is_reachable_by_keyboard() {
        let mut a = app();
        let mut seen = Vec::new();
        for _ in 0..Page::ALL.len() {
            seen.push(a.page);
            a.on_key(key(KeyCode::Down));
        }
        assert_eq!(seen, Page::ALL.to_vec());
        assert_eq!(a.on_key(key(KeyCode::Char('2'))), Some(Action::Refresh));
        assert_eq!(a.page, Page::Servers);
        // 서버 목록 → 두 번째 서버 → 버튼 → 정지
        a.on_key(key(KeyCode::Right));
        assert_eq!(a.focus, Focus::List);
        a.on_key(key(KeyCode::Down));
        a.on_key(key(KeyCode::Enter));
        assert_eq!(a.focus, Focus::Buttons);
        a.on_key(key(KeyCode::Right));
        assert_eq!(a.on_key(key(KeyCode::Enter)), Some(Action::Run("stop pal".into())));
    }

    /// 이름에 공백이 있어도 한 단어로. 채워 여는 버튼은 콘솔 입력 줄로 간다
    #[test]
    fn buttons_run_the_same_command_or_fill_the_console() {
        let mut a = app();
        a.on_key(key(KeyCode::Char('2')));
        let b = a.buttons();
        assert_eq!(b[0].line, "start 'my server'");
        let run = b.iter().position(|b| b.fill).unwrap();
        a.focus = Focus::Buttons;
        a.button = run;
        assert_eq!(a.on_key(key(KeyCode::Enter)), None);
        assert_eq!(a.focus, Focus::Console);
        assert_eq!(a.console.input, "run 'my server' ");
        typed(&mut a, "say hi");
        assert_eq!(a.on_key(key(KeyCode::Enter)), Some(Action::Run("run 'my server' say hi".into())));
        // Esc는 버튼으로 돌아간다
        a.on_key(key(KeyCode::Esc));
        assert_eq!(a.focus, Focus::Buttons);
    }

    /// 고를 서버가 없으면 서버가 필요한 버튼은 실행하지 않고 알린다
    #[test]
    fn a_button_that_needs_a_server_waits_for_one() {
        let mut a = app();
        a.snap.servers.clear();
        a.on_key(key(KeyCode::Char('2')));
        a.focus = Focus::Buttons;
        assert_eq!(a.on_key(key(KeyCode::Enter)), None);
        assert_eq!(a.console.lines.back().map(String::as_str), Some(t!("tui.pick_first").as_str()));
        let add = a.buttons().iter().position(|b| b.line == "server add").unwrap();
        a.button = add;
        assert_eq!(a.on_key(key(KeyCode::Enter)), Some(Action::Run("server add".into())));
    }

    /// 콘솔: `:`로 들어가 치고 Enter. 다른 명령이 도는 중이면 줄을 남기고 알린다
    #[test]
    fn the_console_runs_lines_and_says_when_it_is_busy() {
        let mut a = app();
        a.on_key(key(KeyCode::Char(':')));
        assert_eq!(a.focus, Focus::Console);
        typed(&mut a, "status");
        a.running = true;
        assert_eq!(a.on_key(key(KeyCode::Enter)), None);
        assert_eq!(a.console.input, "status");
        assert_eq!(a.console.lines.back().map(String::as_str), Some(t!("tui.busy").as_str()));
        a.running = false;
        assert_eq!(a.on_key(key(KeyCode::Enter)), Some(Action::Run("status".into())));
        // 콘솔에서는 q도 글자다
        typed(&mut a, "q");
        assert_eq!(a.console.input, "q");
        assert_eq!(a.on_key(key(KeyCode::Up)), None);
        assert_eq!(a.console.input, "status");
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
        a.on_key(ctrl_c());
        assert_eq!(rx.recv().unwrap(), None);

        a.running = true;
        assert_eq!(a.on_key(ctrl_c()), Some(Action::Cancel));
        assert_eq!(a.on_key(key(KeyCode::Char('q'))), None);
        a.running = false;
        // 콘솔에 쓰던 줄이 있으면 Ctrl+C는 줄만 지운다
        a.on_key(key(KeyCode::Char(':')));
        typed(&mut a, "server");
        assert_eq!(a.on_key(ctrl_c()), None);
        assert!(a.console.input.is_empty());
        assert_eq!(a.on_key(ctrl_c()), Some(Action::Quit));
    }

    /// 콘솔의 안내에는 `saba-chan-cli` 머리가 붙지 않는다 (콘솔에는 그 뒤만 친다)
    #[test]
    fn hints_in_the_console_drop_the_program_name() {
        let mut a = app();
        a.push_line("다음 할 일\n  saba-chan-cli bot pair");
        assert_eq!(a.console.lines.back().map(String::as_str), Some("  bot pair"));
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
