//! 화면 상태와 키·마우스 처리. 그리기·터미널과 떨어져 있어 시험에서 키를 눌러 볼 수 있다
//!
//! 화면은 위에서 아래로 쪽 탭, 쪽(목록 → 버튼 → 콘솔), 출력 창, 명령 줄이다. 화살표는 보이는 방향대로 옮긴다:
//! 목록 맨 위에서 ↑는 탭으로, 목록에서 →는 버튼으로, 버튼에서 ↓는 서버 콘솔로.
//! 버튼 하나는 같은 뜻의 명령 한 줄이라 버튼으로 되는 일은 `:` 명령 줄에 쳐서도 된다. 자주 쓰지 않는 것은 [더 보기]에 둔다.

use super::data::{s, Snapshot};
use ratatui::crossterm::event::{KeyCode, KeyEvent, KeyModifiers, MouseButton, MouseEvent, MouseEventKind};
use ratatui::layout::Rect;
use ratatui::widgets::ListState;
use serde_json::Value;
use std::collections::{BTreeMap, VecDeque};

/// 출력 창에 남기는 최대 줄 수
const MAX_LINES: usize = 2000;

/// 쪽 탭
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Page {
    Servers,
    Modules,
    Extensions,
    Discord,
    Updates,
    Daemon,
}

impl Page {
    pub const ALL: [Page; 6] = [Page::Servers, Page::Modules, Page::Extensions, Page::Discord, Page::Updates, Page::Daemon];

    pub fn key(self) -> &'static str {
        match self {
            Page::Servers => "servers",
            Page::Modules => "modules",
            Page::Extensions => "extensions",
            Page::Discord => "discord",
            Page::Updates => "updates",
            Page::Daemon => "daemon",
        }
    }

    pub fn title(self) -> String {
        t!(&format!("tui.page.{}", self.key()))
    }

    /// 고를 목록이 있는 쪽
    pub fn has_list(self) -> bool {
        matches!(self, Page::Servers | Page::Modules | Page::Extensions)
    }

    fn slot(self) -> usize {
        Page::ALL.iter().position(|p| *p == self).unwrap_or(0)
    }
}

/// 버튼이 보이는 때. `More`는 [더 보기] 창에만
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum When {
    Always,
    /// 고른 서버가 꺼져 있을 때
    Stopped,
    /// 고른 서버가 켜져 있거나 켜지는 중일 때
    Running,
    /// 고른 익스텐션이 켜져 있을 때
    On,
    Off,
    /// 디스코드 릴레이가 붙어 있을 때
    Linked,
    Unlinked,
    DaemonUp,
    DaemonDown,
    More,
}

/// 쪽마다 버튼: (문구 키, 명령, 보이는 때). `{}`는 목록에서 고른 것, 끝이 공백이면 명령 줄을 채워 열어 나머지를 받는다
pub fn page_buttons(page: Page) -> &'static [(&'static str, &'static str, When)] {
    use When::*;
    match page {
        Page::Servers => &[
            ("start", "start {}", Stopped),
            ("stop", "stop {}", Running),
            ("restart", "restart {}", Running),
            ("players", "server players {}", Running),
            ("backup_create", "backup create {}", Always),
            ("settings", "server settings {}", Always),
            ("info", "server info {}", Always),
            ("server_add", "server add", Always),
            ("backup_list", "backup list {}", More),
            ("schedule", "schedule show {}", More),
            ("commands", "server commands {}", More),
            ("log", "server log {} -n 100", More),
            ("validate", "server validate {}", More),
            ("remove", "server rm {}", More),
        ],
        Page::Modules => &[
            ("info", "module info {}", Always),
            ("versions", "module versions {}", Always),
            ("available", "module available", Always),
            ("refresh", "module refresh", Always),
            ("commands", "module commands {}", More),
            ("remove", "module remove {}", More),
        ],
        Page::Extensions => &[
            ("enable", "extension enable {}", Off),
            ("disable", "extension disable {}", On),
            ("config", "extension config {}", Always),
            ("ext_update", "extension update {}", Always),
            ("available", "extension available", Always),
            ("rescan", "extension rescan", Always),
            ("info", "extension info {}", More),
        ],
        Page::Discord => &[
            ("bot_start", "bot start", Unlinked),
            ("bot_stop", "bot stop", Linked),
            ("bot_pair", "bot pair", Always),
            ("bot_nodes", "bot nodes", Always),
            ("perm", "perm show", Always),
            ("chatbot", "chatbot show", Always),
            ("bot_status", "bot status", More),
            ("bot_channels", "bot channels", More),
            ("bot_moderation", "bot moderation", More),
            ("bot_invite", "bot invite", More),
            ("bot_config", "bot config", More),
        ],
        Page::Updates => &[
            ("update_check", "update check", Always),
            ("update_download", "update download", Always),
            ("update_last", "update last", Always),
            ("update_status", "update status", More),
            ("update_integrity", "update integrity", More),
            ("update_config", "update config", More),
        ],
        Page::Daemon => &[
            ("daemon_start", "daemon start", DaemonDown),
            ("daemon_restart", "daemon restart", DaemonUp),
            ("daemon_stop", "daemon stop", DaemonUp),
            ("diagnostics", "daemon diagnostics", DaemonUp),
            ("config_set", "config set ", Always),
            ("config_show", "config show", More),
            ("config_defaults", "config defaults", More),
            ("clients", "daemon clients", More),
            ("sandbox", "sandbox list", More),
            ("audit", "audit", More),
            ("monitor", "monitor", More),
            ("status", "status", More),
            ("version", "version", More),
        ],
    }
}

/// 버튼 하나 (그릴 때와 누를 때 같은 것을 쓴다)
#[derive(Debug, Clone, PartialEq)]
pub struct Button {
    pub label: String,
    /// 실행할 명령 한 줄 (앞에 `saba-chan-cli`를 붙이지 않는다). [더 보기]는 빈 줄
    pub line: String,
    /// 명령 줄을 채워 열고 나머지를 받는다
    pub fill: bool,
    /// 목록에서 고른 것이 있어야 하는데 없으면 false
    pub enabled: bool,
    /// [더 보기] 창을 연다
    pub more: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Focus {
    Tabs,
    List,
    Actions,
    /// 서버 콘솔 입력 (친 줄을 그 서버로 보낸다)
    Console,
    /// 아래 명령 줄 (`saba-chan-cli` 뒤에 쓰는 그대로)
    Command,
}

/// 진행 표시 하나 (출력 창 위 작업 줄)
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
    /// 서버 콘솔에 친 한 줄 (서버 id, 줄). `console`의 입력과 같게 보낸다
    Send { server: String, text: String },
    /// 실행 중인 명령을 멈춤
    Cancel,
    /// 쪽이나 고른 것이 바뀌었다: 그 화면에 필요한 것을 곧바로 읽는다
    Refresh,
    /// 데스크톱 앱으로 바꾼다 (고른 서버를 넘긴다)
    SwitchToGui,
    Quit,
}

/// 입력 줄 하나와 지난 입력
#[derive(Default)]
pub struct LineInput {
    pub text: String,
    pub history: Vec<String>,
    pos: Option<usize>,
}

impl LineInput {
    /// 공통 편집 키. 처리했으면 true
    fn edit(&mut self, key: KeyEvent) -> bool {
        let ctrl = key.modifiers.contains(KeyModifiers::CONTROL);
        match key.code {
            KeyCode::Backspace => {
                self.text.pop();
            }
            KeyCode::Up | KeyCode::Down if !self.history.is_empty() => {
                let n = self.history.len();
                let pos = match (self.pos, key.code) {
                    (None, KeyCode::Up) => n - 1,
                    (Some(p), KeyCode::Up) => p.saturating_sub(1),
                    (Some(p), _) if p + 1 < n => p + 1,
                    _ => {
                        self.pos = None;
                        self.text.clear();
                        return true;
                    }
                };
                self.pos = Some(pos);
                self.text = self.history[pos].clone();
            }
            KeyCode::Char(ch) if !ctrl => self.text.push(ch),
            _ => return false,
        }
        true
    }

    /// Enter: 친 줄을 꺼내고 지난 입력에 남긴다
    fn take(&mut self) -> Option<String> {
        let line = self.text.trim().to_string();
        if line.is_empty() {
            return None;
        }
        self.text.clear();
        self.pos = None;
        if self.history.last() != Some(&line) {
            self.history.push(line.clone());
        }
        Some(line)
    }

    fn clear(&mut self) {
        self.text.clear();
        self.pos = None;
    }
}

/// 명령 결과가 쌓이는 출력 창 (평소에는 닫혀 있고, 결과가 두 줄 이상이면 열린다)
#[derive(Default)]
pub struct Output {
    pub lines: VecDeque<String>,
    /// 아래에서 몇 줄 올려 보고 있는가 (접힌 줄 기준)
    pub scroll: usize,
    pub open: bool,
    /// 지금 명령이 낸 줄 수
    cmd_lines: usize,
}

/// 마지막으로 그린 자리 (마우스)
#[derive(Default)]
pub struct Areas {
    /// 쪽 탭 (쪽 순서대로, 화면 밖이면 빈 칸)
    pub tabs: Vec<Rect>,
    pub gui: Rect,
    pub list: Rect,
    pub buttons: Vec<Rect>,
    /// [더 보기] 창의 줄들
    pub more: Vec<Rect>,
    /// 서버 콘솔·데몬 로그
    pub view: Rect,
    pub output: Rect,
    pub command: Rect,
}

impl Areas {
    pub fn clear(&mut self) {
        *self = Areas::default();
    }
}

pub struct App {
    pub page: Page,
    pub focus: Focus,
    /// 명령 줄에서 Esc로 돌아갈 곳
    back: Focus,
    pub list_state: ListState,
    /// 쪽마다 마지막으로 고른 항목 (Page::ALL 순서)
    picked: [usize; Page::ALL.len()],
    pub button: usize,
    pub snap: Snapshot,
    pub output: Output,
    pub command: LineInput,
    /// 서버 콘솔에 칠 줄
    pub server_input: LineInput,
    /// 서버 콘솔·데몬 로그를 아래에서 몇 줄 올려 보고 있는가
    pub view_scroll: usize,
    /// 마지막으로 실행한 명령 (출력 창 제목)
    pub last_command: Option<String>,
    /// 맨 아래 줄에 잠깐 보이는 결과 한 줄
    pub toast: Option<String>,
    pub dialog: Option<Dialog>,
    /// [더 보기] 창에서 고른 줄
    pub more: Option<usize>,
    pub jobs: BTreeMap<u64, Job>,
    pub running: bool,
    pub show_help: bool,
    /// 처음에 열 서버 (`tui --server`, GUI에서 넘어올 때). 서버 목록을 받으면 고르고 지운다
    pub open_server: Option<String>,
    /// 지금 붙은 데몬의 포트 (데몬 쪽)
    pub port: u16,
    pub areas: Areas,
}

impl App {
    pub fn new() -> Self {
        let mut list_state = ListState::default();
        list_state.select(Some(0));
        Self {
            page: Page::Servers,
            focus: Focus::List,
            back: Focus::List,
            list_state,
            picked: [0; Page::ALL.len()],
            button: 0,
            snap: Snapshot::default(),
            output: Output::default(),
            command: LineInput::default(),
            server_input: LineInput::default(),
            view_scroll: 0,
            last_command: None,
            toast: None,
            dialog: None,
            more: None,
            jobs: BTreeMap::new(),
            running: false,
            show_help: false,
            open_server: None,
            port: 0,
            areas: Areas::default(),
        }
    }

    /// 지금 쪽 목록의 이름들 (버튼 명령의 `{}` 자리에 들어간다)
    pub fn list_names(&self) -> Vec<String> {
        match self.page {
            Page::Servers => self.snap.servers.iter().map(|v| s(v, "name").to_string()).collect(),
            Page::Modules => self.snap.modules.iter().map(|v| s(v, "name").to_string()).collect(),
            Page::Extensions => self.snap.extensions.iter().map(|v| s(v, "id").to_string()).collect(),
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

    /// 읽어 온 콘솔을 화면에 둔다. 그사이 다른 서버를 골랐으면 늦게 온 앞 서버 것은 버린다
    pub fn apply_view(&mut self, v: super::data::View) {
        if let super::data::View::Console(id, _) = &v {
            if self.selected_server().map(|s| super::data::s(s, "id")) != Some(id.as_str()) {
                return;
            }
        }
        self.snap.apply_view(v);
    }

    /// 쪽이 어디든 서버 쪽에서 마지막으로 고른 서버의 id (GUI로 넘길 때)
    pub fn current_server_id(&self) -> Option<String> {
        let i = if self.page == Page::Servers { self.selected_index() } else { self.picked[Page::Servers.slot()] };
        self.snap.servers.get(i).map(|v| s(v, "id").to_string())
    }

    /// GUI로 전환할 수 있는가 (데몬이 이 PC에서 GUI를 띄울 수 있다고 할 때만)
    pub fn can_switch_to_gui(&self) -> bool {
        self.snap.health.is_some() && self.snap.launchable.as_ref().and_then(|l| l.get("gui")).and_then(Value::as_bool) == Some(true)
    }

    /// 처음에 열 서버를 고른다 (id나 이름)
    pub fn apply_open_server(&mut self) {
        let Some(want) = self.open_server.clone() else { return };
        if self.snap.servers.is_empty() {
            return;
        }
        self.open_server = None;
        if let Some(i) = self.snap.servers.iter().position(|v| s(v, "id") == want || s(v, "name") == want) {
            self.go(Page::Servers);
            self.list_state.select(Some(i));
            self.focus = Focus::Actions;
        }
    }

    fn when(&self, w: When) -> bool {
        let server = || self.selected_server().map(|v| s(v, "status")).unwrap_or("");
        let enabled = || self.snap.extensions.get(self.selected_index()).and_then(|v| v.get("enabled")).and_then(Value::as_bool) == Some(true);
        let linked = || self.snap.agent.as_ref().and_then(|a| a.get("running")).and_then(Value::as_bool) == Some(true);
        match w {
            When::Always => true,
            When::Running => matches!(server(), "running" | "starting" | "provisioning"),
            When::Stopped => !matches!(server(), "running" | "starting" | "provisioning" | "stopping"),
            When::On => enabled(),
            When::Off => !enabled(),
            When::Linked => linked(),
            When::Unlinked => !linked(),
            When::DaemonUp => self.snap.health.is_some(),
            When::DaemonDown => self.snap.health.is_none(),
            When::More => false,
        }
    }

    fn button_of(&self, key: &str, template: &str) -> Button {
        let picked = self.selected().map(|n| shlex::try_quote(&n).map(|q| q.into_owned()).unwrap_or(n));
        let needs_item = template.contains("{}");
        Button {
            label: t!(&format!("tui.btn.{key}")),
            line: template.replace("{}", picked.as_deref().unwrap_or("")),
            fill: template.ends_with(' '),
            enabled: !needs_item || picked.is_some(),
            more: false,
        }
    }

    /// 지금 쪽의 버튼 (지금 상태에 맞는 것만, 끝에 [더 보기])
    pub fn buttons(&self) -> Vec<Button> {
        let mut out: Vec<Button> = page_buttons(self.page)
            .iter()
            .filter(|(_, _, w)| self.when(*w))
            .map(|(key, template, _)| self.button_of(key, template))
            .collect();
        if page_buttons(self.page).iter().any(|(_, _, w)| *w == When::More) {
            out.push(Button { label: t!("tui.btn.more"), line: String::new(), fill: false, enabled: true, more: true });
        }
        out
    }

    /// [더 보기] 창의 버튼
    pub fn more_buttons(&self) -> Vec<Button> {
        page_buttons(self.page)
            .iter()
            .filter(|(_, _, w)| *w == When::More)
            .map(|(key, template, _)| self.button_of(key, template))
            .collect()
    }

    /// 출력 창에 한 줄(여러 줄도). 터미널용 안내의 `saba-chan-cli ` 머리는 뗀다: 명령 줄에는 그 뒤만 치면 된다
    pub fn push_line(&mut self, line: &str) {
        for l in line.split('\n') {
            let indent = l.len() - l.trim_start().len();
            let shown = match l.trim_start().strip_prefix("saba-chan-cli ") {
                Some(rest) => format!("{}{}", &l[..indent], rest),
                None => l.to_string(),
            };
            self.output.lines.push_back(shown);
        }
        while self.output.lines.len() > MAX_LINES {
            self.output.lines.pop_front();
        }
    }

    /// 명령이 낸 결과. 한 줄이면 맨 아래 줄에 잠깐, 두 줄째부터는 출력 창을 연다
    pub fn command_output(&mut self, line: &str) {
        self.push_line(line);
        self.output.scroll = 0;
        self.output.cmd_lines += line.split('\n').count();
        if self.output.cmd_lines >= 2 {
            self.output.open = true;
        }
        if let Some(last) = self.output.lines.back() {
            self.toast = Some(last.clone());
        }
    }

    /// 화면이 알리는 한 줄 (출력 창은 열지 않는다)
    pub fn notify(&mut self, line: &str) {
        self.push_line(line);
        self.toast = Some(line.to_string());
    }

    /// 명령을 시작한다: 출력 창 제목과 줄 수를 새로
    pub fn begin_command(&mut self, shown: &str) {
        self.push_line("");
        self.push_line(&format!("› {shown}"));
        self.last_command = Some(shown.to_string());
        self.output.cmd_lines = 0;
        self.output.scroll = 0;
        self.toast = None;
        self.running = true;
    }

    fn go(&mut self, page: Page) -> Option<Action> {
        if self.page == page {
            return None;
        }
        self.picked[self.page.slot()] = self.list_state.selected().unwrap_or(0);
        self.page = page;
        self.list_state.select(Some(self.picked[page.slot()]));
        self.button = 0;
        self.view_scroll = 0;
        if matches!(self.focus, Focus::List | Focus::Actions | Focus::Console) {
            self.focus = Focus::Tabs;
        }
        Some(Action::Refresh)
    }

    fn move_page(&mut self, right: bool) -> Option<Action> {
        let n = Page::ALL.len();
        let i = self.page.slot();
        self.go(Page::ALL[if right { (i + 1) % n } else { (i + n - 1) % n }])
    }

    /// 쪽 안으로 들어가는 첫 칸
    fn enter_page(&mut self) {
        self.focus = if self.page.has_list() && !self.list_names().is_empty() { Focus::List } else { Focus::Actions };
    }

    fn select(&mut self, i: usize) -> Option<Action> {
        let n = self.list_names().len();
        if n == 0 {
            return None;
        }
        let i = i.min(n - 1);
        if i == self.selected_index() {
            return None;
        }
        self.list_state.select(Some(i));
        self.button = 0;
        self.view_scroll = 0;
        // 서버를 바꾸면 그 서버의 콘솔을 읽는다
        (self.page == Page::Servers).then_some(Action::Refresh)
    }

    fn open_command(&mut self) {
        if self.focus != Focus::Command {
            self.back = self.focus;
        }
        self.focus = Focus::Command;
    }

    fn can_type_to_server(&self) -> bool {
        self.page == Page::Servers && self.selected_server().is_some()
    }

    /// Tab 순서: 탭 → 목록(있으면) → 버튼 → 서버 콘솔(서버 쪽)
    fn cycle(&mut self, forward: bool) {
        let mut order = vec![Focus::Tabs];
        if self.page.has_list() && !self.list_names().is_empty() {
            order.push(Focus::List);
        }
        order.push(Focus::Actions);
        if self.can_type_to_server() {
            order.push(Focus::Console);
        }
        let i = order.iter().position(|f| *f == self.focus).unwrap_or(0);
        self.focus = order[if forward { (i + 1) % order.len() } else { (i + order.len() - 1) % order.len() }];
    }

    /// 버튼 누르기: [더 보기]는 창을, 채울 것이 있으면 명령 줄을, 아니면 바로 실행
    fn press(&mut self, b: Button) -> Option<Action> {
        if b.more {
            self.more = Some(0);
            return None;
        }
        if !b.enabled {
            self.notify(&t!("tui.pick_first"));
            return None;
        }
        if b.fill {
            self.command.text = b.line;
            self.open_command();
            return None;
        }
        self.run_line(b.line)
    }

    /// 한 줄 실행 요청. 다른 명령이 도는 중이면 버리지 않고 알린다
    fn run_line(&mut self, line: String) -> Option<Action> {
        if self.running {
            self.notify(&t!("tui.busy"));
            return None;
        }
        Some(Action::Run(line))
    }

    /// 지금 보이는 서버 콘솔·데몬 로그·출력 창을 넘긴다
    fn scroll(&mut self, up: bool, by: usize) {
        let target = if self.output.open { &mut self.output.scroll } else { &mut self.view_scroll };
        *target = if up { *target + by } else { target.saturating_sub(by) };
    }

    pub fn on_key(&mut self, key: KeyEvent) -> Option<Action> {
        let ctrl = key.modifiers.contains(KeyModifiers::CONTROL);
        if self.dialog.is_some() {
            self.dialog_key(key);
            return None;
        }
        if ctrl && key.code == KeyCode::Char('c') {
            // 쓰던 줄이 있으면 그 줄만 지운다
            let input = match self.focus {
                Focus::Command => Some(&mut self.command),
                Focus::Console => Some(&mut self.server_input),
                _ => None,
            };
            if let Some(input) = input.filter(|i| !i.text.is_empty()) {
                input.clear();
                return None;
            }
            return Some(if self.running { Action::Cancel } else { Action::Quit });
        }
        if self.more.is_some() {
            return self.more_key(key);
        }
        if self.show_help {
            self.show_help = false;
            return None;
        }
        match key.code {
            KeyCode::F(2) => {
                self.output.open = !self.output.open;
                return None;
            }
            KeyCode::F(9) if self.can_switch_to_gui() => return Some(Action::SwitchToGui),
            KeyCode::PageUp => {
                self.scroll(true, 10);
                return None;
            }
            KeyCode::PageDown => {
                self.scroll(false, 10);
                return None;
            }
            KeyCode::Tab if self.focus != Focus::Command => {
                self.cycle(true);
                return None;
            }
            KeyCode::BackTab => {
                self.cycle(false);
                return None;
            }
            _ => {}
        }
        if self.focus == Focus::Command {
            return self.command_key(key);
        }
        if self.focus == Focus::Console {
            return self.console_key(key);
        }
        match key.code {
            KeyCode::Char('q') if !self.running => return Some(Action::Quit),
            KeyCode::Char(':') | KeyCode::Char('/') => {
                self.open_command();
                return None;
            }
            KeyCode::Char('?') => {
                self.show_help = true;
                return None;
            }
            KeyCode::Char(c @ '1'..='9') => {
                let page = Page::ALL.get(c as usize - '1' as usize).copied()?;
                return self.go(page);
            }
            KeyCode::Esc => {
                if self.output.open {
                    self.output.open = false;
                } else {
                    self.toast = None;
                    self.focus = match self.focus {
                        Focus::Actions if self.page.has_list() && !self.list_names().is_empty() => Focus::List,
                        _ => Focus::Tabs,
                    };
                }
                return None;
            }
            _ => {}
        }
        match self.focus {
            Focus::Tabs => match key.code {
                KeyCode::Left => return self.move_page(false),
                KeyCode::Right => return self.move_page(true),
                KeyCode::Down | KeyCode::Enter => self.enter_page(),
                _ => {}
            },
            Focus::List => match key.code {
                KeyCode::Up if self.selected_index() == 0 => self.focus = Focus::Tabs,
                KeyCode::Up => return self.select(self.selected_index() - 1),
                KeyCode::Down => return self.select(self.selected_index() + 1),
                KeyCode::Right | KeyCode::Enter => self.focus = Focus::Actions,
                _ => {}
            },
            Focus::Actions => {
                let n = self.buttons().len();
                self.button = self.button.min(n.saturating_sub(1));
                match key.code {
                    KeyCode::Left if self.button == 0 && self.page.has_list() && !self.list_names().is_empty() => self.focus = Focus::List,
                    KeyCode::Left => self.button = self.button.saturating_sub(1),
                    KeyCode::Right if n > 0 => self.button = (self.button + 1).min(n - 1),
                    KeyCode::Up => self.focus = Focus::Tabs,
                    KeyCode::Down if self.can_type_to_server() => self.focus = Focus::Console,
                    KeyCode::Enter => {
                        let b = self.buttons().get(self.button)?.clone();
                        return self.press(b);
                    }
                    _ => {}
                }
            }
            Focus::Console | Focus::Command => {}
        }
        None
    }

    /// 아래 명령 줄
    fn command_key(&mut self, key: KeyEvent) -> Option<Action> {
        match key.code {
            KeyCode::Esc => self.focus = self.back,
            KeyCode::Tab => {
                if let Some(done) = complete(&self.command.text) {
                    self.command.text = done;
                }
            }
            KeyCode::Enter => {
                if self.command.text.trim().is_empty() {
                    return None;
                }
                if self.running {
                    // 줄은 남겨 둔다: 지금 명령이 끝나면 다시 Enter
                    self.notify(&t!("tui.busy"));
                    return None;
                }
                return self.command.take().map(Action::Run);
            }
            _ => {
                self.command.edit(key);
            }
        }
        None
    }

    /// 서버 콘솔 입력: 친 줄을 그 서버로
    fn console_key(&mut self, key: KeyEvent) -> Option<Action> {
        match key.code {
            KeyCode::Esc => self.focus = Focus::Actions,
            KeyCode::Enter => {
                let server = self.selected_server().map(|v| s(v, "id").to_string())?;
                if self.server_input.text.trim().is_empty() {
                    return None;
                }
                if self.running {
                    self.notify(&t!("tui.busy"));
                    return None;
                }
                self.view_scroll = 0;
                return self.server_input.take().map(|text| Action::Send { server, text });
            }
            _ => {
                self.server_input.edit(key);
            }
        }
        None
    }

    fn more_key(&mut self, key: KeyEvent) -> Option<Action> {
        let items = self.more_buttons();
        let i = self.more.unwrap_or(0).min(items.len().saturating_sub(1));
        match key.code {
            KeyCode::Esc | KeyCode::Char('q') => self.more = None,
            KeyCode::Up => self.more = Some(i.checked_sub(1).unwrap_or(items.len().saturating_sub(1))),
            KeyCode::Down => self.more = Some(if i + 1 < items.len() { i + 1 } else { 0 }),
            KeyCode::Enter => {
                self.more = None;
                return self.press(items.get(i)?.clone());
            }
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
                if self.more.is_some() {
                    let picked = self.areas.more.iter().position(|r| inside(*r));
                    self.more = None;
                    return self.press(self.more_buttons().get(picked?)?.clone());
                }
                if inside(self.areas.gui) && self.can_switch_to_gui() {
                    return Some(Action::SwitchToGui);
                }
                if let Some(i) = self.areas.buttons.iter().position(|r| inside(*r)) {
                    self.focus = Focus::Actions;
                    self.button = i;
                    return self.press(self.buttons().get(i)?.clone());
                }
                if let Some(i) = self.areas.tabs.iter().position(|r| inside(*r)) {
                    self.focus = Focus::Tabs;
                    return self.go(Page::ALL[i]);
                }
                if inside(self.areas.list) {
                    let i = row_in(self.areas.list)? + self.list_state.offset();
                    if i < self.list_names().len() {
                        self.focus = Focus::List;
                        return self.select(i);
                    }
                }
                if inside(self.areas.view) && self.can_type_to_server() {
                    self.focus = Focus::Console;
                }
                if inside(self.areas.command) {
                    self.open_command();
                }
            }
            MouseEventKind::ScrollUp if inside(self.areas.output) => self.output.scroll += 3,
            MouseEventKind::ScrollDown if inside(self.areas.output) => self.output.scroll = self.output.scroll.saturating_sub(3),
            MouseEventKind::ScrollUp if inside(self.areas.view) => self.view_scroll += 3,
            MouseEventKind::ScrollDown if inside(self.areas.view) => self.view_scroll = self.view_scroll.saturating_sub(3),
            MouseEventKind::ScrollUp if inside(self.areas.list) => return self.select(self.selected_index().saturating_sub(1)),
            MouseEventKind::ScrollDown if inside(self.areas.list) => return self.select(self.selected_index() + 1),
            _ => {}
        }
        None
    }
}

/// 명령 줄 자동 완성: 친 만큼과 같은 깊이의 명령 중 앞이 같은 것들의 공통 부분까지. 하나뿐이면 뒤에 공백
pub fn complete(input: &str) -> Option<String> {
    let depth = input.matches(' ').count();
    let words: Vec<String> = crate::cli::command_words()
        .into_iter()
        .chain(["help".to_string()])
        .filter(|w| w.matches(' ').count() == depth && w.starts_with(input))
        .collect();
    let first = words.first()?;
    if words.len() == 1 {
        return Some(format!("{first} "));
    }
    let common: String = words.iter().fold(first.clone(), |acc, w| acc.chars().zip(w.chars()).take_while(|(a, b)| a == b).map(|(a, _)| a).collect());
    (common.len() > input.len()).then_some(common)
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
        Cmd::Shell | Cmd::Tui { .. } | Cmd::Completions { .. } | Cmd::Install(_) => Some(t!("tui.not_here")),
        Cmd::Gui { .. } => Some(t!("tui.use_switch")),
        Cmd::Monitor(a) if a.watch => Some(t!("tui.not_here")),
        Cmd::Update(UpdateCmd::Apply { .. }) => Some(t!("tui.not_here")),
        _ => None,
    }
}

/// TUI에서 쓰도록 고친 명령: 콘솔은 입력 줄을 열지 않고 따라 읽는다 (서버에 칠 줄은 서버 콘솔에)
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
        let mut a = App::new();
        a.snap.health = Some(json!({ "version": "0.1.10" }));
        a.snap.servers = vec![
            json!({ "id": "s1", "name": "my server", "status": "stopped" }),
            json!({ "id": "s2", "name": "pal", "status": "running" }),
        ];
        a
    }

    fn typed(a: &mut App, text: &str) {
        for c in text.chars() {
            a.on_key(key(KeyCode::Char(c)));
        }
    }

    fn lines(a: &App) -> Vec<String> {
        a.buttons().into_iter().map(|b| b.line).collect()
    }

    /// 쪽 버튼의 명령은 모두 실제 명령이고 문구가 있다 (명령 표가 바뀌어 버튼이 깨지지 않게)
    #[test]
    fn every_page_button_is_a_real_command() {
        crate::i18n::init("ko");
        for page in Page::ALL {
            for (key, template, _) in page_buttons(page) {
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
        assert!(crate::i18n::try_t("tui.btn.more").is_some());
    }

    /// 서버를 고르고 → 버튼 → ↓ 서버 콘솔. 보이는 방향대로 옮긴다
    /// 서버를 빨리 바꾸면 앞 서버의 늦은 콘솔 응답이 온다: 지금 고른 서버 것만 화면에 둔다
    #[test]
    fn a_late_console_for_another_server_is_dropped() {
        use super::super::data::View;
        let mut a = app();
        a.list_state.select(Some(1));
        a.apply_view(View::Console("s2".into(), vec![json!({ "content": "pal" })]));
        a.apply_view(View::Console("s1".into(), vec![json!({ "content": "old" })]));
        assert_eq!(a.snap.console.as_ref().map(|(id, l)| (id.as_str(), l.len())), Some(("s2", 1)));
        a.apply_view(View::DaemonLog(vec![json!({ "message": "x" })]));
        assert_eq!(a.snap.daemon_log.len(), 1, "the daemon log is one view: always kept");
    }

    #[test]
    fn arrows_move_the_way_the_screen_is_laid_out() {
        let mut a = app();
        assert_eq!((a.page, a.focus), (Page::Servers, Focus::List));
        // 목록 맨 위에서 ↑는 탭으로, 탭에서 →는 다음 쪽, ↓는 쪽 안으로
        a.on_key(key(KeyCode::Up));
        assert_eq!(a.focus, Focus::Tabs);
        assert_eq!(a.on_key(key(KeyCode::Right)), Some(Action::Refresh));
        assert_eq!(a.page, Page::Modules);
        a.on_key(key(KeyCode::Left));
        a.on_key(key(KeyCode::Down));
        assert_eq!(a.focus, Focus::List);
        // 두 번째 서버 → 버튼 (켜진 서버라 정지가 맨 앞)
        assert_eq!(a.on_key(key(KeyCode::Down)), Some(Action::Refresh));
        a.on_key(key(KeyCode::Right));
        assert_eq!(a.focus, Focus::Actions);
        assert_eq!(a.on_key(key(KeyCode::Enter)), Some(Action::Run("stop pal".into())));
        // 버튼에서 ↓는 서버 콘솔, Esc는 버튼으로, 버튼 첫 칸에서 ←는 목록으로
        a.on_key(key(KeyCode::Down));
        assert_eq!(a.focus, Focus::Console);
        a.on_key(key(KeyCode::Esc));
        assert_eq!(a.focus, Focus::Actions);
        a.on_key(key(KeyCode::Left));
        assert_eq!(a.focus, Focus::List);
        // 숫자 키로 쪽 바로 가기
        assert_eq!(a.on_key(key(KeyCode::Char('6'))), Some(Action::Refresh));
        assert_eq!(a.page, Page::Daemon);
    }

    /// 버튼은 서버 상태에 맞는 것만: 꺼진 서버는 시작, 켜진 서버는 정지·재시작. 덜 쓰는 것은 [더 보기]에
    #[test]
    fn buttons_follow_the_server_state_and_rare_ones_go_under_more() {
        let mut a = app();
        let stopped = lines(&a);
        assert_eq!(stopped[0], "start 'my server'");
        assert!(!stopped.iter().any(|l| l.starts_with("stop") || l.starts_with("restart")));
        assert!(!stopped.iter().any(|l| l.starts_with("server rm")), "rare buttons live under more");
        a.list_state.select(Some(1));
        let running = lines(&a);
        assert_eq!(&running[..2], ["stop pal", "restart pal"]);
        // [더 보기] → 창 → 지우기
        a.focus = Focus::Actions;
        a.button = a.buttons().iter().position(|b| b.more).unwrap();
        assert_eq!(a.on_key(key(KeyCode::Enter)), None);
        assert_eq!(a.more, Some(0));
        let rm = a.more_buttons().iter().position(|b| b.line == "server rm pal").unwrap();
        for _ in 0..rm {
            a.on_key(key(KeyCode::Down));
        }
        assert_eq!(a.on_key(key(KeyCode::Enter)), Some(Action::Run("server rm pal".into())));
        assert_eq!(a.more, None);
    }

    /// 서버 콘솔에 친 줄은 그 서버로 간다. ↑는 지난 입력
    #[test]
    fn the_server_console_sends_lines_to_the_selected_server() {
        let mut a = app();
        a.list_state.select(Some(1));
        a.focus = Focus::Console;
        typed(&mut a, "say \"hi there\"");
        assert_eq!(a.on_key(key(KeyCode::Enter)), Some(Action::Send { server: "s2".into(), text: "say \"hi there\"".into() }));
        // 콘솔에서는 q도 :도 글자다
        typed(&mut a, "q:");
        assert_eq!(a.server_input.text, "q:");
        a.server_input.clear();
        a.on_key(key(KeyCode::Up));
        assert_eq!(a.server_input.text, "say \"hi there\"");
    }

    /// 채워 여는 버튼은 명령 줄로 간다. Esc는 원래 자리로
    #[test]
    fn fill_buttons_open_the_command_line() {
        let mut a = app();
        a.go(Page::Daemon);
        a.focus = Focus::Actions;
        a.button = a.buttons().iter().position(|b| b.fill).unwrap();
        assert_eq!(a.on_key(key(KeyCode::Enter)), None);
        assert_eq!(a.focus, Focus::Command);
        assert_eq!(a.command.text, "config set ");
        typed(&mut a, "language=ko");
        assert_eq!(a.on_key(key(KeyCode::Enter)), Some(Action::Run("config set language=ko".into())));
        a.on_key(key(KeyCode::Esc));
        assert_eq!(a.focus, Focus::Actions);
    }

    /// 고를 서버가 없으면 서버가 필요한 버튼은 실행하지 않고 알린다. [새 서버]는 된다
    #[test]
    fn a_button_that_needs_a_server_waits_for_one() {
        let mut a = app();
        a.snap.servers.clear();
        a.focus = Focus::Actions;
        a.button = 0;
        assert_eq!(a.on_key(key(KeyCode::Enter)), None);
        assert_eq!(a.toast.as_deref(), Some(t!("tui.pick_first").as_str()));
        a.button = a.buttons().iter().position(|b| b.line == "server add").unwrap();
        assert_eq!(a.on_key(key(KeyCode::Enter)), Some(Action::Run("server add".into())));
        // 서버가 없으면 ↓로 콘솔에 가지 않는다
        a.on_key(key(KeyCode::Down));
        assert_eq!(a.focus, Focus::Actions);
    }

    /// 명령 줄: `:`로 열고 Enter. 다른 명령이 도는 중이면 줄을 남기고 알린다. Tab은 자동 완성
    #[test]
    fn the_command_line_runs_completes_and_says_when_it_is_busy() {
        let mut a = app();
        a.on_key(key(KeyCode::Char(':')));
        assert_eq!(a.focus, Focus::Command);
        typed(&mut a, "stat");
        a.on_key(key(KeyCode::Tab));
        assert_eq!(a.command.text, "status ");
        a.running = true;
        assert_eq!(a.on_key(key(KeyCode::Enter)), None);
        assert_eq!(a.command.text, "status ");
        assert_eq!(a.toast.as_deref(), Some(t!("tui.busy").as_str()));
        a.running = false;
        assert_eq!(a.on_key(key(KeyCode::Enter)), Some(Action::Run("status".into())));
        a.on_key(key(KeyCode::Up));
        assert_eq!(a.command.text, "status");
    }

    #[test]
    fn completion_goes_as_far_as_the_candidates_agree() {
        assert_eq!(complete("serv").as_deref(), Some("server"));
        assert_eq!(complete("server inf").as_deref(), Some("server info "));
        assert_eq!(complete("zzz"), None);
    }

    /// 한 줄 결과는 맨 아래 줄에만, 두 줄째부터 출력 창이 열린다. Esc로 닫는다
    #[test]
    fn short_results_toast_and_long_ones_open_the_output() {
        let mut a = app();
        a.begin_command("start pal");
        a.command_output("✓ 시작했어요");
        assert!(!a.output.open);
        assert_eq!(a.toast.as_deref(), Some("✓ 시작했어요"));
        a.running = false;
        a.begin_command("server info pal");
        a.command_output("이름  pal");
        a.command_output("상태  실행 중");
        assert!(a.output.open);
        a.focus = Focus::Actions;
        a.on_key(key(KeyCode::Esc));
        assert!(!a.output.open);
        assert_eq!(a.focus, Focus::Actions, "the first Esc only closes the output");
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
        // 명령 줄에 쓰던 줄이 있으면 Ctrl+C는 줄만 지운다
        a.on_key(key(KeyCode::Char(':')));
        typed(&mut a, "server");
        assert_eq!(a.on_key(ctrl_c()), None);
        assert!(a.command.text.is_empty());
        assert_eq!(a.on_key(ctrl_c()), Some(Action::Quit));
    }

    /// GUI로 전환은 데몬이 GUI를 띄울 수 있다고 할 때만, 고른 서버를 넘긴다
    #[test]
    fn switching_to_the_gui_needs_a_gui_and_carries_the_server() {
        let mut a = app();
        assert_eq!(a.on_key(key(KeyCode::F(9))), None);
        a.snap.launchable = Some(json!({ "cli": true, "gui": true }));
        a.list_state.select(Some(1));
        a.go(Page::Daemon);
        assert_eq!(a.on_key(key(KeyCode::F(9))), Some(Action::SwitchToGui));
        assert_eq!(a.current_server_id().as_deref(), Some("s2"));
    }

    /// `tui --server`·GUI에서 넘겨받은 서버를 목록이 오면 고른다
    #[test]
    fn the_server_handed_over_is_selected_when_the_list_arrives() {
        let mut a = app();
        let servers = std::mem::take(&mut a.snap.servers);
        a.open_server = Some("s2".into());
        a.go(Page::Daemon);
        a.apply_open_server();
        assert!(a.open_server.is_some(), "waits for the list");
        a.snap.servers = servers;
        a.apply_open_server();
        assert_eq!((a.page, a.selected_index(), a.focus), (Page::Servers, 1, Focus::Actions));
        assert!(a.open_server.is_none());
    }

    /// 출력 창의 안내에는 `saba-chan-cli` 머리가 붙지 않는다 (명령 줄에는 그 뒤만 친다)
    #[test]
    fn hints_in_the_output_drop_the_program_name() {
        let mut a = app();
        a.push_line("다음 할 일\n  saba-chan-cli bot pair");
        assert_eq!(a.output.lines.back().map(String::as_str), Some("  bot pair"));
    }

    #[test]
    fn commands_that_take_the_whole_screen_are_refused_or_adapted() {
        crate::i18n::init("ko");
        let w = |s: &str| shlex::split(s).unwrap();
        assert!(refuse(&w("shell")).is_some());
        assert!(refuse(&w("tui --server pal")).is_some());
        assert!(refuse(&w("gui")).is_some());
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

    /// 다른 쪽에 다녀와도 쪽마다 고른 항목이 남는다
    #[test]
    fn each_page_keeps_its_selection() {
        let mut a = app();
        a.snap.servers.push(json!({ "id": "c" }));
        a.list_state.select(Some(2));
        a.go(Page::Discord);
        a.go(Page::Servers);
        assert_eq!(a.list_state.selected(), Some(2));
        a.go(Page::Modules);
        assert_eq!(a.list_state.selected(), Some(0));
    }
}
