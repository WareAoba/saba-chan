//! 화면 상태와 키·마우스 처리. 그리기·터미널과 떨어져 있어 시험에서 키를 눌러 볼 수 있다
//!
//! 화면은 위에서 아래로 쪽 탭(서버·디스코드·모듈·익스텐션·업데이트·앱 설정), 쪽, 출력 창, 맨 아래 줄이다.
//! 쪽은 왼쪽 목록과 오른쪽 상세(상세 탭 → 버튼 → 내용)로 나뉜다. Tab은 칸을 차례로 옮기고 Esc는 언제나 한 단계 뒤로 간다.
//! 글자 키는 바로 행동한다: s 시작·정지, r 재시작, n 새 서버, Ctrl+K 무엇이든 찾기.
//! 버튼과 폼 저장은 같은 뜻의 명령 한 줄로 실행하므로, 버튼으로 되는 일은 `:` 명령 줄에 쳐서도 된다.

use super::data::{s, Snapshot};
use super::form::{Form, FormOut, Kind};
use super::palette::{Entry, EntryKind, Go, Palette, PaletteOut};
use super::wizard::{WizOut, Wizard};
use crate::catalog::Catalog;
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
    Discord,
    /// 모듈·익스텐션 (상세 탭으로 나눈다)
    Addons,
    Updates,
    /// 앱 설정 (설정 폼·데몬 로그)
    Settings,
}

impl Page {
    pub const ALL: [Page; 5] = [Page::Servers, Page::Discord, Page::Addons, Page::Updates, Page::Settings];

    pub fn key(self) -> &'static str {
        match self {
            Page::Servers => "servers",
            Page::Discord => "discord",
            Page::Addons => "addons",
            Page::Updates => "updates",
            Page::Settings => "settings",
        }
    }

    pub fn title(self) -> String {
        t!(&format!("tui.page.{}", self.key()))
    }

    /// 고를 목록이 있는 쪽
    pub fn has_list(self) -> bool {
        matches!(self, Page::Servers | Page::Discord | Page::Addons)
    }

    pub fn slot(self) -> usize {
        Page::ALL.iter().position(|p| *p == self).unwrap_or(0)
    }

    /// 상세 탭 (문구 키 끝부분)
    pub fn tabs(self) -> &'static [&'static str] {
        match self {
            Page::Servers => &["console", "settings", "players", "backups", "schedule", "log"],
            Page::Discord => &["perm", "ladder", "words", "music", "tts", "moderation", "notice", "chatbot"],
            Page::Addons => &["modules", "extensions"],
            Page::Updates => &[],
            Page::Settings => &["app", "daemon_log"],
        }
    }

    pub fn tab_label(self, key: &str) -> String {
        match self {
            Page::Servers => t!(&format!("tui.tab.{key}")),
            Page::Discord => t!(&format!("tui.dtab.{key}")),
            Page::Addons => t!(&format!("tui.page.{key}")),
            _ => t!(&format!("tui.stab.{key}")),
        }
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
    /// 고른 모듈·익스텐션에 새 판이 있을 때
    Newer,
    More,
}

/// 새 서버 마법사를 여는 버튼의 명령 자리
pub const WIZARD: &str = "@wizard";

/// 쪽(과 모듈·익스텐션 탭)마다 버튼: (문구 키, 명령, 보이는 때). `{}`는 목록에서 고른 것, 끝이 공백이면 명령 줄을 채워 열어 나머지를 받는다
pub fn page_buttons(page: Page, tab: usize) -> &'static [(&'static str, &'static str, When)] {
    use When::*;
    match (page, tab) {
        (Page::Servers, _) => &[
            ("start", "start {}", Stopped),
            ("stop", "stop {}", Running),
            ("restart", "restart {}", Running),
            ("server_add", WIZARD, More),
            ("info", "server info {}", More),
            ("address", "server address {}", More),
            ("port_check", "server port-check {}", More),
            ("commands", "server commands {}", More),
            ("validate", "server validate {}", More),
            ("remove", "server rm {}", More),
        ],
        (Page::Discord, _) => &[
            ("bot_start", "bot start", Unlinked),
            ("bot_stop", "bot stop", Linked),
            ("bot_pair", "bot pair", Always),
            ("bot_invite", "bot invite", Always),
            ("bot_status", "bot status", More),
            ("bot_nodes", "bot nodes", More),
            ("bot_channels", "bot channels", More),
            ("bot_config", "bot config", More),
        ],
        (Page::Addons, 0) => &[
            ("module_update", "update download module-{}\nupdate apply module-{}", Newer),
            ("module_new_server", WIZARD, Always),
            ("available", "module available", More),
            ("refresh", "module refresh", More),
            ("remove", "module remove {}", More),
        ],
        (Page::Addons, _) => &[
            ("enable", "extension enable {}", Off),
            ("disable", "extension disable {}", On),
            ("ext_update", "extension update {}", Newer),
            ("config", "extension config {}", Always),
            ("available", "extension available", More),
            ("rescan", "extension rescan", More),
            ("remove", "extension remove {}", More),
        ],
        (Page::Updates, _) => &[
            ("update_check", "update check", Always),
            ("update_download", "update download", Always),
            ("update_last", "update last", Always),
            ("update_status", "update status", More),
            ("update_integrity", "update integrity", More),
            ("update_config", "update config", More),
        ],
        (Page::Settings, _) => &[
            ("daemon_start", "daemon start", DaemonDown),
            ("daemon_restart", "daemon restart", DaemonUp),
            ("daemon_stop", "daemon stop", DaemonUp),
            ("diagnostics", "daemon diagnostics", DaemonUp),
            ("config_set", "config set ", More),
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
    /// 바로 누르는 글자 키
    pub key: Option<char>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Focus {
    /// 쪽 탭
    Tabs,
    List,
    /// 상세 탭 줄
    Detail,
    Actions,
    /// 상세 탭의 내용 (콘솔 입력, 설정 폼, 표)
    Content,
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

/// 상세 탭 내용을 어떻게 읽는가
#[derive(Debug, Clone, PartialEq)]
pub enum Load {
    /// 데몬 API로 읽는다
    Fetch(super::data::PaneWant),
    /// 이 명령들을 실행해 나온 글을 그대로 보인다
    Capture(Vec<String>),
    /// 따로 읽을 것 없음 (콘솔·데몬 로그는 늘 따라 읽는다)
    Nothing,
}

/// 키를 누른 결과 중 화면 밖에서 할 일
#[derive(Debug, Clone, PartialEq)]
pub enum Action {
    /// 명령 한 줄을 실행 (앞에 `saba-chan-cli`를 붙이지 않은 단어들)
    Run(String),
    /// 여러 줄을 차례로 (폼 저장이 명령 여러 개일 때)
    RunAll(Vec<String>),
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

/// 저장한 폼을 어떤 명령으로 저장하는가
#[derive(Debug, Clone, PartialEq)]
pub enum Saver {
    /// `server set 이름 키=값 …`
    ServerSet(String),
    /// `config set 키=값 …`
    Config,
    /// `bot music|tts 디스코드서버 --키 값 …`
    Bot(&'static str, String),
    /// `perm command|grant 디스코드서버 …` (줄마다 하나)
    Perm(String),
}

impl Saver {
    pub fn lines(&self, changes: &[(String, String)]) -> Vec<String> {
        let q = quote;
        match self {
            Saver::ServerSet(name) => vec![super::form::save_line(name, changes)],
            Saver::Config => {
                let mut line = "config set".to_string();
                for (k, v) in changes {
                    line.push(' ');
                    line.push_str(&q(&format!("{k}={v}")));
                }
                vec![line]
            }
            Saver::Bot(sub, guild) => {
                let mut line = format!("bot {sub} {}", q(guild));
                for (k, v) in changes {
                    let arg = match (k.as_str(), v.as_str()) {
                        // 지운 칸은 보내지 않는다 (`bot` 옵션에는 "기본값으로"가 없다)
                        (_, "") => continue,
                        ("enabled", "true") => "--on".to_string(),
                        ("enabled", "false") => "--off".to_string(),
                        ("enabled", _) => "--on|--off".to_string(),
                        (k, "true") => format!("--{k} on"),
                        (k, "false") => format!("--{k} off"),
                        (k, v) => format!("--{k} {}", q(v)),
                    };
                    line.push(' ');
                    line.push_str(&arg);
                }
                vec![line]
            }
            Saver::Perm(guild) => changes
                .iter()
                .map(|(k, v)| match k.strip_prefix("member:") {
                    // 지운 칸은 기본으로: 멤버는 레벨을 거두고, 명령은 레벨을 빼면 기본값
                    Some(member) if v.is_empty() => format!("perm revoke {} {}", q(guild), q(member)),
                    Some(member) => format!("perm grant {} {} {}", q(guild), q(member), q(v)),
                    None if v.is_empty() => format!("perm command {} {}", q(guild), q(k)),
                    None => format!("perm command {} {} {}", q(guild), q(k), q(v)),
                })
                .collect(),
        }
    }
}

/// 상세 탭 내용
pub enum PaneState {
    Loading,
    Form(Form, Saver),
    /// 표 (접속자·백업)
    Rows(Vec<Value>),
    /// 글 (명령 출력, 실행 로그)
    Text(Vec<String>),
    Failed(String),
}

/// 지금 보이는 상세 탭 내용. `key`가 쪽·탭·고른 것과 맞을 때만 쓴다
pub struct Pane {
    pub key: String,
    pub state: PaneState,
    /// 표의 고른 줄
    pub cursor: usize,
    /// 글을 아래에서 몇 줄 올려 보는가
    pub scroll: usize,
}

impl Default for Pane {
    fn default() -> Self {
        Pane { key: String::new(), state: PaneState::Loading, cursor: 0, scroll: 0 }
    }
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
    /// 상세 탭
    pub detail: Vec<Rect>,
    pub gui: Rect,
    pub list: Rect,
    /// 목록 아래 [+ 새 서버]
    pub add: Rect,
    pub buttons: Vec<Rect>,
    /// [더 보기] 창의 줄들
    pub more: Vec<Rect>,
    /// 상세 탭 내용 (콘솔·폼·표)
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
    /// 쪽마다 고른 상세 탭
    pub tab: [usize; Page::ALL.len()],
    pub button: usize,
    pub snap: Snapshot,
    pub output: Output,
    pub command: LineInput,
    /// 서버 콘솔에 칠 줄
    pub server_input: LineInput,
    /// 콘솔 명령 제안에서 고른 줄
    pub suggest: usize,
    /// 서버 콘솔·데몬 로그를 아래에서 몇 줄 올려 보고 있는가
    pub view_scroll: usize,
    pub pane: Pane,
    /// 콘솔 명령 제안에 쓰는 고른 서버의 명령 목록 (서버 id)
    pub catalog: Option<(String, Catalog)>,
    /// 명령 출력을 상세 탭으로 받는 중 (그 탭의 키)
    pub capture: Option<String>,
    /// 마지막으로 실행한 명령 (출력 창 제목)
    pub last_command: Option<String>,
    /// 맨 아래 줄에 잠깐 보이는 결과 한 줄
    pub toast: Option<String>,
    pub dialog: Option<Dialog>,
    /// [더 보기] 창에서 고른 줄
    pub more: Option<usize>,
    pub wizard: Option<Wizard>,
    pub palette: Option<Palette>,
    /// 찾기로 간 설정 칸 (폼을 읽으면 그 칸에 커서)
    pub want_field: Option<String>,
    pub jobs: BTreeMap<u64, Job>,
    pub running: bool,
    pub show_help: bool,
    /// 처음에 열 서버 (`tui --server`, GUI에서 넘어올 때). 서버 목록을 받으면 고르고 지운다
    pub open_server: Option<String>,
    /// 지금 붙은 데몬의 포트 (앱 설정 쪽)
    pub port: u16,
    /// 마지막으로 그린 폭이 좁아 목록과 상세를 한 번에 하나씩 보이는가
    pub narrow: bool,
    /// 멈춘 명령: 끝 소식이 와도 탭을 다시 읽지 않는다 (명령 출력 탭이 곧바로 같은 명령을 다시 돌리지 않게)
    pub cancelled: bool,
    /// 다음 명령 줄에서 가릴 값 (폼에서 저장하는 비밀번호)
    pub secrets: Vec<String>,
    /// 디스코드 서버 목록을 마지막으로 받은 때 (릴레이를 3초마다 부르지 않게)
    pub guilds_at: Option<std::time::Instant>,
    pub areas: Areas,
}

use super::form::quote;

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
            tab: [0; Page::ALL.len()],
            button: 0,
            snap: Snapshot::default(),
            output: Output::default(),
            command: LineInput::default(),
            server_input: LineInput::default(),
            suggest: 0,
            view_scroll: 0,
            pane: Pane::default(),
            catalog: None,
            capture: None,
            last_command: None,
            toast: None,
            dialog: None,
            more: None,
            wizard: None,
            palette: None,
            want_field: None,
            jobs: BTreeMap::new(),
            running: false,
            show_help: false,
            open_server: None,
            port: 0,
            narrow: false,
            cancelled: false,
            secrets: Vec::new(),
            guilds_at: None,
            areas: Areas::default(),
        }
    }

    /// 지금 쪽에서 고른 상세 탭 번호
    pub fn tab_index(&self) -> usize {
        self.tab[self.page.slot()]
    }

    /// 지금 쪽에서 고른 상세 탭 (없으면 빈 문자열)
    pub fn tab_key(&self) -> &'static str {
        self.page.tabs().get(self.tab_index()).copied().unwrap_or("")
    }

    /// 지금 쪽 목록의 이름들 (버튼 명령의 `{}` 자리에 들어간다)
    pub fn list_names(&self) -> Vec<String> {
        match (self.page, self.tab_index()) {
            (Page::Servers, _) => self.snap.servers.iter().map(|v| s(v, "name").to_string()).collect(),
            (Page::Discord, _) => self.snap.guilds.iter().map(|v| s(v, "guildId").to_string()).collect(),
            (Page::Addons, 0) => self.snap.modules.iter().map(|v| s(v, "name").to_string()).collect(),
            (Page::Addons, _) => self.snap.extensions.iter().map(|v| s(v, "id").to_string()).collect(),
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

    /// 디스코드 쪽에서 고른 디스코드 서버
    pub fn selected_guild(&self) -> Option<&Value> {
        (self.page == Page::Discord).then(|| self.snap.guilds.get(self.selected_index())).flatten()
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
            self.focus = Focus::Detail;
        }
    }

    // ── 상세 탭 내용 ──

    /// 지금 보이는 상세 탭의 키와 읽는 법. 고를 것이 없으면 None
    pub fn pane_want(&self) -> Option<(String, Load)> {
        use super::data::PaneWant;
        let tab = self.tab_key();
        let key = |target: &str| format!("{}:{tab}:{target}", self.page.key());
        match self.page {
            Page::Servers => {
                let v = self.selected_server()?;
                let (id, name) = (s(v, "id").to_string(), s(v, "name").to_string());
                let load = match tab {
                    "console" => Load::Fetch(PaneWant::Catalog(id.clone())),
                    "settings" => Load::Fetch(PaneWant::ServerSettings(id.clone())),
                    "players" => Load::Fetch(PaneWant::Players(id.clone())),
                    "backups" => Load::Fetch(PaneWant::Backups(id.clone())),
                    "schedule" => Load::Capture(vec![format!("schedule show {}", quote(&name))]),
                    _ => Load::Fetch(PaneWant::Log(id.clone())),
                };
                Some((key(&id), load))
            }
            Page::Discord => {
                let g = s(self.selected_guild()?, "guildId").to_string();
                let gq = quote(&g);
                let load = match tab {
                    "perm" => Load::Fetch(PaneWant::Perm(g.clone())),
                    "music" | "tts" => Load::Fetch(PaneWant::Discord(g.clone())),
                    "ladder" => Load::Capture(vec![format!("perm show {gq}")]),
                    "words" => Load::Capture(self.snap.servers.iter().map(|v| format!("server discord {}", quote(s(v, "name")))).collect()),
                    "moderation" => Load::Capture(vec![format!("bot moderation {gq}")]),
                    "notice" => Load::Capture(vec![format!("bot notice {gq}")]),
                    _ => Load::Capture(vec![format!("chatbot show --guild {gq}")]),
                };
                Some((key(&g), load))
            }
            Page::Settings => Some((key(""), if tab == "app" { Load::Fetch(PaneWant::AppConfig) } else { Load::Nothing })),
            _ => None,
        }
    }

    /// 지금 탭에 맞는 내용인가
    pub fn pane_is_current(&self) -> bool {
        self.pane_want().is_some_and(|(k, _)| k == self.pane.key)
    }

    /// 지금 탭의 명령이 바꾸기에 쓰는 명령 앞부분 (글로 보이는 탭에서 `e`)
    pub fn edit_prefix(&self) -> Option<String> {
        let gq = || self.selected_guild().map(|g| quote(s(g, "guildId")));
        let sq = || self.selected_server().map(|v| quote(s(v, "name")));
        Some(match (self.page, self.tab_key()) {
            (Page::Servers, "schedule") => format!("schedule add {} ", sq()?),
            (Page::Discord, "ladder") => format!("perm level add {} ", gq()?),
            // 별명은 서버마다: 어느 서버인지는 사용자가 쓴다
            (Page::Discord, "words") => "server discord ".to_string(),
            (Page::Discord, "moderation") => format!("bot channel {} ", gq()?),
            (Page::Discord, "notice") => format!("bot notice {} ", gq()?),
            (Page::Discord, "chatbot") => format!("chatbot set --guild {} ", gq()?),
            _ => return None,
        })
    }

    /// 같은 내용을 보는 CLI 명령 (상세 탭 아래 `같은 명령`)
    pub fn same_command(&self) -> Option<String> {
        let sq = || self.selected_server().map(|v| quote(s(v, "name")));
        let gq = || self.selected_guild().map(|g| quote(s(g, "guildId")));
        Some(match (self.page, self.tab_key()) {
            (Page::Servers, "console") => format!("console {}", sq()?),
            (Page::Servers, "settings") => format!("server settings {}", sq()?),
            (Page::Servers, "players") => format!("server players {}", sq()?),
            (Page::Servers, "backups") => format!("backup list {}", sq()?),
            (Page::Servers, "schedule") => format!("schedule show {}", sq()?),
            (Page::Servers, "log") => format!("server log {} -n 300", sq()?),
            (Page::Discord, "perm") => format!("perm commands {}", gq()?),
            (Page::Discord, "music") => format!("bot music {}", gq()?),
            (Page::Discord, "tts") => format!("bot tts {}", gq()?),
            (Page::Settings, "app") => "config show".to_string(),
            (Page::Settings, "daemon_log") => "daemon logs".to_string(),
            _ => match self.pane_want() {
                Some((_, Load::Capture(lines))) => lines.first()?.clone(),
                _ => return None,
            },
        })
    }

    /// 읽어 온 상세 탭 내용을 둔다. 그사이 다른 탭·서버로 갔으면 버린다. 같은 폼을 다시 읽었으면 커서·펼침을 남긴다
    pub fn apply_pane(&mut self, key: String, data: super::data::PaneData) {
        use super::data::PaneData;
        if self.pane_want().map(|(k, _)| k) != Some(key.clone()) {
            return;
        }
        if let PaneData::Catalog(id, cat) = data {
            self.catalog = Some((id, cat));
            self.pane = Pane { key, state: PaneState::Text(Vec::new()), ..Pane::default() };
            return;
        }
        let state = match data {
            PaneData::Settings(doc) => {
                let name = s(&doc, "server").to_string();
                PaneState::Form(Form::from_doc(&doc), Saver::ServerSet(name))
            }
            PaneData::Config(cfg) => PaneState::Form(super::data::config_form(&cfg), Saver::Config),
            PaneData::Perm(form, guild) => PaneState::Form(form, Saver::Perm(guild)),
            PaneData::Discord(settings, guild) => {
                let sub = if self.tab_key() == "music" { "music" } else { "tts" };
                PaneState::Form(super::data::discord_form(sub, &settings), Saver::Bot(sub, guild))
            }
            PaneData::Rows(rows) => PaneState::Rows(rows),
            PaneData::Text(lines) => PaneState::Text(lines),
            PaneData::Failed(why) => PaneState::Failed(why),
            PaneData::Catalog(..) => unreachable!(),
        };
        let same = self.pane.key == key;
        match (&mut self.pane.state, state) {
            (PaneState::Form(old, _), PaneState::Form(fresh, saver)) if same => {
                old.reload(fresh);
                let form = std::mem::take(old);
                self.pane.state = PaneState::Form(form, saver);
            }
            (_, state) => {
                let keep = same.then_some((self.pane.cursor, self.pane.scroll));
                self.pane = Pane { key, state, ..Pane::default() };
                if let Some((cursor, scroll)) = keep {
                    self.pane.cursor = cursor;
                    self.pane.scroll = scroll;
                }
            }
        }
        if let (Some(want), PaneState::Form(form, _)) = (self.want_field.clone(), &mut self.pane.state) {
            if let Some(i) = form.fields.iter().position(|f| f.key == want) {
                let group = form.fields[i].group.clone();
                form.open.insert(group);
                form.cursor = form.rows().iter().position(|r| *r == super::form::Row::Field(i)).unwrap_or(0);
                self.focus = Focus::Content;
            }
            self.want_field = None;
        }
    }

    /// 명령 출력을 상세 탭으로 받기 시작
    pub fn begin_capture(&mut self, key: String) {
        self.pane = Pane { key: key.clone(), state: PaneState::Text(Vec::new()), ..Pane::default() };
        self.capture = Some(key);
        self.running = true;
    }

    /// 상세 탭으로 받는 명령 출력 한 줄
    pub fn capture_line(&mut self, line: &str) {
        if self.capture.as_deref() != Some(self.pane.key.as_str()) {
            return;
        }
        if let PaneState::Text(lines) = &mut self.pane.state {
            for l in line.split('\n') {
                let indent = l.len() - l.trim_start().len();
                lines.push(match l.trim_start().strip_prefix("saba-chan-cli ") {
                    Some(rest) => format!("{}{rest}", &l[..indent]),
                    None => l.to_string(),
                });
            }
        }
    }

    // ── 버튼 ──

    /// 고른 모듈·익스텐션의 새 판 (업데이트 확인 결과에서, 지금 판과 다를 때만)
    pub fn newer_version(&self) -> Option<String> {
        if self.page != Page::Addons {
            return None;
        }
        let id = self.selected()?;
        let component = if self.tab_index() == 0 { format!("module-{id}") } else { format!("ext-{id}") };
        let c = self.snap.updates.as_ref()?.get("components")?.as_array()?.iter().find(|c| s(c, "component") == component)?;
        let (now, latest) = (s(c, "current_version"), s(c, "latest_version"));
        (c.get("update_available").and_then(Value::as_bool) == Some(true) && !latest.is_empty() && latest != now).then(|| latest.to_string())
    }

    fn when(&self, w: When) -> bool {
        let server = || self.selected_server().map(crate::output::status_of).unwrap_or("");
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
            When::Newer => self.newer_version().is_some(),
            When::More => false,
        }
    }

    fn button_of(&self, key: &str, template: &str) -> Button {
        let picked = self.selected().map(|n| quote(&n));
        let needs_item = template.contains("{}");
        Button {
            label: t!(&format!("tui.btn.{key}")),
            line: template.replace("{}", picked.as_deref().unwrap_or("")),
            fill: template.ends_with(' '),
            enabled: !needs_item || picked.is_some(),
            more: false,
            key: match (self.page, key) {
                (Page::Servers, "start" | "stop") => Some('s'),
                (Page::Servers, "restart") => Some('r'),
                (Page::Servers, "server_add") => Some('n'),
                _ => None,
            },
        }
    }

    /// 지금 쪽의 버튼 (지금 상태에 맞는 것만, 끝에 [더 보기])
    pub fn buttons(&self) -> Vec<Button> {
        let all = page_buttons(self.page, self.tab_index());
        let mut out: Vec<Button> = all.iter().filter(|(_, _, w)| self.when(*w)).map(|(key, template, _)| self.button_of(key, template)).collect();
        if all.iter().any(|(_, _, w)| *w == When::More) {
            out.push(Button { label: t!("tui.btn.more"), line: String::new(), fill: false, enabled: true, more: true, key: None });
        }
        out
    }

    /// [더 보기] 창의 버튼
    pub fn more_buttons(&self) -> Vec<Button> {
        page_buttons(self.page, self.tab_index())
            .iter()
            .filter(|(_, _, w)| *w == When::More)
            .map(|(key, template, _)| self.button_of(key, template))
            .collect()
    }

    // ── 출력 ──

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
        if self.capture.is_some() {
            self.capture_line(line);
            return;
        }
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

    /// 화면에 보일 명령 줄: 폼에서 저장한 비밀번호는 가린다
    pub fn masked(&mut self, line: &str) -> String {
        let mut out = line.to_string();
        for secret in std::mem::take(&mut self.secrets) {
            out = out.replace(&quote(&secret), "••••").replace(&secret, "••••");
        }
        out
    }

    /// 명령을 시작한다: 출력 창 제목과 줄 수를 새로
    pub fn begin_command(&mut self, shown: &str) {
        self.push_line("");
        self.push_line(&format!("› {shown}"));
        self.last_command = Some(shown.to_string());
        // 멈춘 탭 읽기가 남긴 받기 표시에 이 명령의 출력이 빨려 들지 않게
        self.capture = None;
        self.output.cmd_lines = 0;
        self.output.scroll = 0;
        self.toast = None;
        self.running = true;
    }

    // ── 옮기기 ──

    pub fn go(&mut self, page: Page) -> Option<Action> {
        if self.page == page {
            return None;
        }
        self.picked[self.page.slot()] = self.list_state.selected().unwrap_or(0);
        if page != Page::Servers {
            // 새 서버 창은 서버 쪽에만 뜬다: 다른 쪽으로 가면 닫는다
            self.wizard = None;
        }
        self.page = page;
        self.list_state.select(Some(self.picked[page.slot()]));
        self.button = 0;
        self.view_scroll = 0;
        if !matches!(self.focus, Focus::Command) {
            self.focus = Focus::Tabs;
        }
        Some(Action::Refresh)
    }

    fn move_page(&mut self, right: bool) -> Option<Action> {
        let n = Page::ALL.len();
        let i = self.page.slot();
        self.go(Page::ALL[if right { (i + 1) % n } else { (i + n - 1) % n }])
    }

    fn set_tab(&mut self, i: usize) -> Option<Action> {
        let n = self.page.tabs().len();
        if n == 0 || i >= n || i == self.tab_index() {
            return None;
        }
        self.tab[self.page.slot()] = i;
        self.button = 0;
        self.view_scroll = 0;
        if self.page == Page::Addons {
            self.list_state.select(Some(0));
        }
        Some(Action::Refresh)
    }

    /// 내용 칸이 있는가 (고른 서버·디스코드 서버가 있거나, 앱 설정 쪽)
    fn has_content(&self) -> bool {
        match self.page {
            Page::Servers => self.selected_server().is_some(),
            Page::Discord => self.selected_guild().is_some(),
            Page::Settings => true,
            _ => false,
        }
    }

    /// Tab 순서. 모듈·익스텐션은 탭이 목록 위에 있다
    pub fn order(&self) -> Vec<Focus> {
        let list = self.page.has_list() && !self.list_names().is_empty();
        let tabs = !self.page.tabs().is_empty();
        let mut order = vec![Focus::Tabs];
        if self.page == Page::Addons {
            order.push(Focus::Detail);
            if list {
                order.push(Focus::List);
            }
        } else {
            if list {
                order.push(Focus::List);
            }
            if tabs && (!self.page.has_list() || list) {
                order.push(Focus::Detail);
            }
        }
        if !self.buttons().is_empty() {
            order.push(Focus::Actions);
        }
        if self.has_content() {
            order.push(Focus::Content);
        }
        order
    }

    fn step_focus(&mut self, forward: bool) {
        let order = self.order();
        let i = order.iter().position(|f| *f == self.focus).unwrap_or(0);
        self.focus = order[if forward { (i + 1) % order.len() } else { (i + order.len() - 1) % order.len() }];
    }

    /// 다음 칸으로 (끝에서는 그대로)
    fn forward(&mut self) {
        let order = self.order();
        if let Some(i) = order.iter().position(|f| *f == self.focus) {
            if i + 1 < order.len() {
                self.focus = order[i + 1];
            }
        }
    }

    /// 한 단계 뒤로: 내용·버튼은 상세 탭으로, 그 밖은 앞 칸으로
    fn backward(&mut self) {
        let order = self.order();
        if matches!(self.focus, Focus::Content | Focus::Actions) && order.contains(&Focus::Detail) && self.page != Page::Addons {
            self.focus = Focus::Detail;
            return;
        }
        if let Some(i) = order.iter().position(|f| *f == self.focus) {
            if i > 0 {
                self.focus = order[i - 1];
            }
        }
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
        self.server_input.clear();
        // 고른 것이 바뀌면 그 서버의 콘솔·상세 탭을 읽는다
        Some(Action::Refresh)
    }

    /// 명령 줄을 닫으면 돌아갈 칸
    pub fn command_back(&self) -> Focus {
        self.back
    }

    fn open_command(&mut self) {
        if self.focus != Focus::Command {
            self.back = self.focus;
        }
        self.focus = Focus::Command;
    }

    /// 명령 줄을 채워 연다
    pub fn prefill(&mut self, text: String) {
        self.command.text = text;
        self.open_command();
    }

    /// 버튼 누르기: [더 보기]는 창을, 채울 것이 있으면 명령 줄을, 마법사는 마법사를, 아니면 바로 실행
    fn press(&mut self, b: Button) -> Option<Action> {
        if b.more {
            self.more = Some(0);
            return None;
        }
        if b.line == WIZARD {
            return self.open_wizard();
        }
        if !b.enabled {
            self.notify(&t!("tui.pick_first"));
            return None;
        }
        if b.fill {
            self.prefill(b.line);
            return None;
        }
        // 여러 줄 버튼 (받고 적용하기)은 차례로
        if b.line.contains('\n') {
            if self.running {
                self.notify(&t!("tui.busy"));
                return None;
            }
            return Some(Action::RunAll(b.line.lines().map(str::to_string).collect()));
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

    fn open_wizard(&mut self) -> Option<Action> {
        // 게임 모듈 탭에서 열면 그 게임을 골라 둔다 (마법사의 게임 목록은 같은 모듈 목록이다)
        let module = (self.page == Page::Addons && self.tab_index() == 0).then(|| self.selected_index());
        self.go(Page::Servers);
        let mut w = Wizard::new();
        if let Some(i) = module {
            w.module = i;
        }
        self.wizard = Some(w);
        Some(Action::Refresh)
    }

    /// 지금 보이는 서버 콘솔·데몬 로그·글·출력 창을 넘긴다
    fn scroll(&mut self, up: bool, by: usize) {
        let text = matches!(self.pane.state, PaneState::Text(_)) && self.pane_is_current() && self.tab_key() != "console";
        // 명령 출력 탭은 위에서 센다
        let up = if text && !self.output.open && self.tab_key() != "log" { !up } else { up };
        let target = if self.output.open {
            &mut self.output.scroll
        } else if text {
            &mut self.pane.scroll
        } else {
            &mut self.view_scroll
        };
        *target = if up { *target + by } else { target.saturating_sub(by) };
    }

    /// 글자 키를 바로 행동으로 받는가 (글을 치는 중이 아니면)
    fn typing(&self) -> bool {
        match self.focus {
            Focus::Command => true,
            Focus::Content => match (&self.pane.state, self.tab_key()) {
                (_, "console") if self.page == Page::Servers => true,
                // 폼에서는 글자 키가 칸을 다룬다: s·r·q·숫자가 서버를 끄거나 메뉴를 닫지 않게 (나가기는 Esc)
                (PaneState::Form(..), _) => self.pane_is_current(),
                _ => false,
            },
            _ => false,
        }
    }

    /// 고른 서버 시작·정지 (s), 재시작 (r)
    fn server_key(&mut self, c: char) -> Option<Action> {
        let v = self.selected_server()?;
        let running = matches!(crate::output::status_of(v), "running" | "starting" | "provisioning");
        let name = quote(s(v, "name"));
        let line = match (c, running) {
            ('s', true) => format!("stop {name}"),
            ('s', false) => format!("start {name}"),
            ('r', true) => format!("restart {name}"),
            _ => return None,
        };
        self.run_line(line)
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
                Focus::Content if self.page == Page::Servers && self.tab_key() == "console" => Some(&mut self.server_input),
                _ => None,
            };
            if let Some(input) = input.filter(|i| !i.text.is_empty()) {
                input.clear();
                return None;
            }
            if self.palette.take().is_some() || self.wizard.take().is_some() {
                return None;
            }
            return Some(if self.running { Action::Cancel } else { Action::Quit });
        }
        if ctrl && key.code == KeyCode::Char('k') && self.palette.is_none() {
            self.more = None;
            self.show_help = false;
            self.palette = Some(Palette::new(self.palette_entries()));
            return None;
        }
        if self.palette.is_some() {
            return self.palette_key(key);
        }
        if self.more.is_some() {
            return self.more_key(key);
        }
        if self.show_help {
            self.show_help = false;
            return None;
        }
        if self.wizard.is_some() && self.focus != Focus::Command {
            return self.wizard_key(key);
        }
        match key.code {
            KeyCode::F(2) => {
                self.output.open = !self.output.open;
                return None;
            }
            KeyCode::F(9) if self.can_switch_to_gui() => return Some(Action::SwitchToGui),
            KeyCode::PageUp if !self.is_form_focus() => {
                self.scroll(true, 10);
                return None;
            }
            KeyCode::PageDown if !self.is_form_focus() => {
                self.scroll(false, 10);
                return None;
            }
            KeyCode::Tab if self.focus != Focus::Command && !self.suggesting() => {
                self.step_focus(true);
                return None;
            }
            KeyCode::BackTab => {
                self.step_focus(false);
                return None;
            }
            _ => {}
        }
        if self.focus == Focus::Command {
            return self.command_key(key);
        }
        // 출력 창이 열려 있으면 Esc는 그것만 닫는다
        if key.code == KeyCode::Esc && self.output.open {
            self.output.open = false;
            return None;
        }
        if !self.typing() {
            match key.code {
                KeyCode::Char('q') if !self.running => return Some(Action::Quit),
                KeyCode::Char(':') => {
                    self.open_command();
                    return None;
                }
                KeyCode::Char('/') if !self.is_form_focus() => {
                    self.open_command();
                    return None;
                }
                KeyCode::Char('?') => {
                    self.show_help = true;
                    return None;
                }
                KeyCode::Char(c @ '1'..='9') if !ctrl => {
                    let page = Page::ALL.get(c as usize - '1' as usize).copied()?;
                    return self.go(page);
                }
                KeyCode::Char(c @ ('s' | 'r')) if self.page == Page::Servers && !ctrl => return self.server_key(c),
                KeyCode::Char('n') if self.page == Page::Servers && !ctrl => return self.open_wizard(),
                _ => {}
            }
        }
        if self.focus == Focus::Content {
            return self.content_key(key);
        }
        if key.code == KeyCode::Esc {
            self.toast = None;
            self.backward();
            return None;
        }
        match self.focus {
            Focus::Tabs => match key.code {
                KeyCode::Left => return self.move_page(false),
                KeyCode::Right => return self.move_page(true),
                KeyCode::Down | KeyCode::Enter => self.forward(),
                _ => {}
            },
            Focus::List => match key.code {
                KeyCode::Up if self.selected_index() == 0 => self.backward(),
                KeyCode::Up => return self.select(self.selected_index() - 1),
                KeyCode::Down => return self.select(self.selected_index() + 1),
                KeyCode::Right | KeyCode::Enter => self.forward(),
                KeyCode::Left if self.page == Page::Addons => self.backward(),
                _ => {}
            },
            Focus::Detail => match key.code {
                KeyCode::Left if self.tab_index() == 0 => self.backward(),
                KeyCode::Left => return self.set_tab(self.tab_index() - 1),
                KeyCode::Right if self.tab_index() + 1 >= self.page.tabs().len() => {
                    if self.order().contains(&Focus::Actions) {
                        self.focus = Focus::Actions;
                        self.button = 0;
                    }
                }
                KeyCode::Right => return self.set_tab(self.tab_index() + 1),
                KeyCode::Up => self.focus = Focus::Tabs,
                // 버튼은 탭 줄 오른쪽에 있다: ↓는 곧바로 내용으로
                KeyCode::Down | KeyCode::Enter if self.order().contains(&Focus::Content) => self.focus = Focus::Content,
                KeyCode::Down | KeyCode::Enter => self.forward(),
                _ => {}
            },
            Focus::Actions => {
                let n = self.buttons().len();
                self.button = self.button.min(n.saturating_sub(1));
                match key.code {
                    KeyCode::Left if self.button == 0 => self.backward(),
                    KeyCode::Left => self.button -= 1,
                    KeyCode::Right if n > 0 => self.button = (self.button + 1).min(n - 1),
                    KeyCode::Up => self.backward(),
                    KeyCode::Down => self.forward(),
                    KeyCode::Enter => {
                        let b = self.buttons().get(self.button)?.clone();
                        return self.press(b);
                    }
                    _ => {}
                }
            }
            Focus::Content | Focus::Command => {}
        }
        None
    }

    fn is_form_focus(&self) -> bool {
        self.focus == Focus::Content && matches!(self.pane.state, PaneState::Form(..)) && self.pane_is_current()
    }

    // ── 내용 칸 ──

    fn content_key(&mut self, key: KeyEvent) -> Option<Action> {
        if self.page == Page::Servers && self.tab_key() == "console" {
            return self.console_key(key);
        }
        if !self.pane_is_current() {
            if key.code == KeyCode::Esc {
                self.backward();
            }
            return None;
        }
        let tab = self.tab_key();
        let server = self.selected_server().map(|v| quote(s(v, "name")));
        if let PaneState::Form(form, saver) = &mut self.pane.state {
            match form.on_key(key) {
                Some(FormOut::Save(changes)) => {
                    let lines = saver.lines(&changes);
                    let secret = |k: &str| form.fields.iter().any(|f| f.key == k && f.kind == Kind::Text { secret: true });
                    self.secrets = changes.iter().filter(|(k, v)| secret(k) && !v.is_empty()).map(|(_, v)| v.clone()).collect();
                    if self.running {
                        self.notify(&t!("tui.busy"));
                        return None;
                    }
                    return Some(if lines.len() == 1 { Action::Run(lines[0].clone()) } else { Action::RunAll(lines) });
                }
                Some(FormOut::Leave) => self.backward(),
                None => {}
            }
            return None;
        }
        if let PaneState::Rows(rows) = &self.pane.state {
            let n = rows.len();
            let picked = rows.get(self.pane.cursor).map(|r| s(r, "name").to_string());
            match key.code {
                KeyCode::Esc => self.backward(),
                KeyCode::Up if self.pane.cursor == 0 => self.backward(),
                KeyCode::Up => self.pane.cursor -= 1,
                KeyCode::Down if self.pane.cursor + 1 < n => self.pane.cursor += 1,
                KeyCode::Char('c') if tab == "backups" => return self.run_line(format!("backup create {}", server?)),
                KeyCode::Enter if tab == "backups" => return self.run_line(format!("backup restore {} {}", server?, quote(&picked?))),
                KeyCode::Char('d') | KeyCode::Delete if tab == "backups" => return self.run_line(format!("backup delete {} {}", server?, quote(&picked?))),
                KeyCode::Enter if tab == "players" => self.prefill(format!("server run {} kick {}", server?, quote(&picked?))),
                _ => {}
            }
            return None;
        }
        // 기록 탭은 아래에서 센다(새 줄이 아래에 붙는다), 명령 출력 탭은 위에서 센다
        let from_bottom = tab == "log";
        match key.code {
            KeyCode::Esc => self.backward(),
            KeyCode::Up if from_bottom => self.pane.scroll += 1,
            KeyCode::Down if from_bottom => self.pane.scroll = self.pane.scroll.saturating_sub(1),
            KeyCode::Up => self.pane.scroll = self.pane.scroll.saturating_sub(1),
            KeyCode::Down => self.pane.scroll += 1,
            KeyCode::Char('c') if tab == "backups" => return self.run_line(format!("backup create {}", server?)),
            KeyCode::Char('e') | KeyCode::Enter => {
                if let Some(p) = self.edit_prefix() {
                    self.prefill(p);
                }
            }
            KeyCode::Char('d') if tab == "schedule" => self.prefill(format!("schedule remove {} ", server?)),
            _ => {}
        }
        None
    }

    // ── 콘솔 ──

    /// 콘솔 입력에 맞는 게임 명령 (이름, 쓰는 법, 설명, 위험). 첫 낱말을 치는 동안만
    pub fn suggestions(&self) -> Vec<(String, String, String, crate::catalog::Risk)> {
        let text = self.server_input.text.as_str();
        if text.is_empty() || text.contains(' ') {
            return Vec::new();
        }
        let Some((id, cat)) = &self.catalog else { return Vec::new() };
        if self.selected_server().map(|v| s(v, "id")) != Some(id.as_str()) {
            return Vec::new();
        }
        let q = text.trim_start_matches('/').to_lowercase();
        let mut out: Vec<(usize, (String, String, String, crate::catalog::Risk))> = cat
            .commands
            .iter()
            .filter_map(|c| {
                let rank = if c.name.to_lowercase().starts_with(&q) {
                    0
                } else if c.aliases.iter().any(|a| a.to_lowercase().starts_with(&q)) {
                    1
                } else if super::search::matches(&c.label, &q) {
                    2
                } else {
                    return None;
                };
                let usage: String = c.inputs.iter().map(|i| if i.required { format!(" <{}>", i.label) } else { format!(" [{}]", i.label) }).collect();
                let about = if c.label.is_empty() || c.label == c.name { c.description.clone() } else { c.label.clone() };
                Some((rank, (c.name.clone(), usage, about, c.risk)))
            })
            .collect();
        out.sort_by(|a, b| a.0.cmp(&b.0).then(a.1 .0.cmp(&b.1 .0)));
        out.into_iter().take(6).map(|(_, x)| x).collect()
    }

    fn suggesting(&self) -> bool {
        self.focus == Focus::Content && self.page == Page::Servers && self.tab_key() == "console" && !self.suggestions().is_empty()
    }

    /// 서버 콘솔 입력: 친 줄을 그 서버로. 명령 제안이 보이면 ↑↓로 고르고 Tab으로 채운다
    fn console_key(&mut self, key: KeyEvent) -> Option<Action> {
        let found = self.suggestions();
        if !found.is_empty() {
            let n = found.len();
            match key.code {
                KeyCode::Up => {
                    self.suggest = (self.suggest + n - 1) % n;
                    return None;
                }
                KeyCode::Down => {
                    self.suggest = (self.suggest + 1) % n;
                    return None;
                }
                KeyCode::Tab => {
                    self.server_input.text = format!("{} ", found[self.suggest.min(n - 1)].0);
                    self.suggest = 0;
                    return None;
                }
                _ => {}
            }
        }
        match key.code {
            KeyCode::Esc => {
                if self.server_input.text.is_empty() {
                    self.backward();
                } else {
                    self.server_input.clear();
                }
            }
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
                self.suggest = 0;
                return self.server_input.take().map(|text| Action::Send { server, text });
            }
            _ => {
                self.suggest = 0;
                self.server_input.edit(key);
            }
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
                self.focus = self.back;
                return self.command.take().map(Action::Run);
            }
            _ => {
                self.command.edit(key);
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

    fn wizard_key(&mut self, key: KeyEvent) -> Option<Action> {
        if key.code == KeyCode::Char(':') && self.wizard.as_ref().is_some_and(|w| w.step != 2) {
            self.open_command();
            return None;
        }
        let modules = self.snap.modules.clone();
        let out = self.wizard.as_mut()?.on_key(key, &modules)?;
        match out {
            WizOut::Close => {
                self.wizard = None;
                self.focus = Focus::List;
            }
            WizOut::Notice(text) => self.notify(&text),
            WizOut::Run(line) => {
                if self.running {
                    self.notify(&t!("tui.busy"));
                    return None;
                }
                self.wizard = None;
                self.focus = Focus::List;
                return Some(Action::Run(line));
            }
        }
        None
    }

    // ── 무엇이든 찾기 ──

    /// 찾을 것: 쪽·상세 탭·서버·디스코드 서버, 읽어 둔 설정 칸, CLI 명령
    pub fn palette_entries(&self) -> Vec<Entry> {
        let mut out = Vec::new();
        let screen = |title: String, path: String, cmd: String, page: Page, tab: Option<usize>, item: Option<usize>| Entry {
            kind: EntryKind::Screen,
            title,
            path,
            cmd,
            go: Go::Screen { page: page.slot(), tab, item },
        };
        for page in Page::ALL {
            out.push(screen(page.title(), String::new(), String::new(), page, None, None));
            if !page.has_list() {
                for (ti, tab) in page.tabs().iter().enumerate() {
                    out.push(screen(page.tab_label(tab), page.title(), String::new(), page, Some(ti), None));
                }
            }
        }
        for (ti, tab) in Page::Addons.tabs().iter().enumerate() {
            out.push(screen(Page::Addons.tab_label(tab), Page::Addons.title(), format!("{} list", if ti == 0 { "module" } else { "extension" }), Page::Addons, Some(ti), None));
        }
        for (i, v) in self.snap.servers.iter().enumerate() {
            let name = s(v, "name");
            let path = format!("{} › {name}", Page::Servers.title());
            out.push(screen(name.to_string(), Page::Servers.title(), format!("server info {}", quote(name)), Page::Servers, None, Some(i)));
            for (ti, tab) in Page::Servers.tabs().iter().enumerate() {
                let cmd = match *tab {
                    "console" => format!("console {}", quote(name)),
                    "settings" => format!("server settings {}", quote(name)),
                    "players" => format!("server players {}", quote(name)),
                    "backups" => format!("backup list {}", quote(name)),
                    "schedule" => format!("schedule show {}", quote(name)),
                    _ => format!("server log {}", quote(name)),
                };
                out.push(screen(Page::Servers.tab_label(tab), path.clone(), cmd, Page::Servers, Some(ti), Some(i)));
            }
        }
        for (i, g) in self.snap.guilds.iter().enumerate() {
            let name = if s(g, "guildName").is_empty() { s(g, "guildId") } else { s(g, "guildName") };
            let path = format!("{} › {name}", Page::Discord.title());
            out.push(screen(name.to_string(), Page::Discord.title(), String::new(), Page::Discord, None, Some(i)));
            for (ti, tab) in Page::Discord.tabs().iter().enumerate() {
                out.push(screen(Page::Discord.tab_label(tab), path.clone(), String::new(), Page::Discord, Some(ti), Some(i)));
            }
        }
        // 지금 읽어 둔 설정 폼의 칸
        if let (PaneState::Form(form, saver), true) = (&self.pane.state, self.pane_is_current()) {
            let item = self.page.has_list().then(|| self.selected_index());
            let place = match (self.selected_server(), self.selected_guild()) {
                (Some(v), _) => format!("{} › {}", s(v, "name"), self.page.tab_label(self.tab_key())),
                (_, Some(g)) => format!("{} › {}", s(g, "guildName"), self.page.tab_label(self.tab_key())),
                _ => self.page.tab_label(self.tab_key()),
            };
            for f in &form.fields {
                if f.kind == Kind::ReadOnly {
                    continue;
                }
                let cmd = saver.lines(&[(f.key.clone(), "VALUE".into())]).into_iter().next().unwrap_or_default();
                out.push(Entry {
                    kind: EntryKind::Setting,
                    title: f.label.clone(),
                    path: place.clone(),
                    cmd: cmd.strip_suffix("VALUE").unwrap_or(&cmd).trim_end().to_string(),
                    go: Go::Field { page: self.page.slot(), tab: self.tab_index(), item, key: f.key.clone() },
                });
            }
        }
        for path in crate::cli::command_words() {
            let Some(about) = crate::i18n::try_t(&format!("help.cmd.{path}")) else { continue };
            out.push(Entry { kind: EntryKind::Command, title: about, path: String::new(), cmd: path.clone(), go: Go::Prefill(format!("{path} ")) });
        }
        out
    }

    fn goto(&mut self, go: Go) -> Option<Action> {
        match go {
            Go::Prefill(text) => {
                self.prefill(text);
                None
            }
            Go::Screen { page, tab, item } => {
                self.go(Page::ALL[page]);
                if let Some(t) = tab {
                    self.tab[self.page.slot()] = t;
                }
                if let Some(i) = item {
                    self.list_state.select(Some(i));
                }
                self.focus = if tab.is_some() { Focus::Detail } else if item.is_some() { Focus::List } else { Focus::Tabs };
                Some(Action::Refresh)
            }
            Go::Field { page, tab, item, key } => {
                self.go(Page::ALL[page]);
                self.tab[self.page.slot()] = tab;
                if let Some(i) = item {
                    self.list_state.select(Some(i));
                }
                self.want_field = Some(key.clone());
                // 이미 읽어 둔 폼이면 곧바로 그 칸으로
                if let PaneState::Form(form, _) = &mut self.pane.state {
                    if let Some(i) = form.fields.iter().position(|f| f.key == key) {
                        let group = form.fields[i].group.clone();
                        form.open.insert(group);
                        form.cursor = form.rows().iter().position(|r| *r == super::form::Row::Field(i)).unwrap_or(0);
                        self.want_field = None;
                    }
                }
                self.focus = Focus::Content;
                Some(Action::Refresh)
            }
        }
    }

    fn palette_key(&mut self, key: KeyEvent) -> Option<Action> {
        let out = self.palette.as_mut()?.on_key(key)?;
        self.palette = None;
        match out {
            PaletteOut::Close => None,
            PaletteOut::Prefill(text) => {
                self.prefill(text);
                None
            }
            PaletteOut::Go(go) => self.goto(go),
        }
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
        if self.dialog.is_some() || self.palette.is_some() {
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
                if let Some(i) = self.areas.detail.iter().position(|r| inside(*r)) {
                    self.focus = Focus::Detail;
                    return self.set_tab(i);
                }
                if inside(self.areas.add) {
                    return self.open_wizard();
                }
                if inside(self.areas.list) {
                    let rows = if self.page == Page::Servers || self.page == Page::Discord { 3 } else { 1 };
                    let i = row_in(self.areas.list)? / rows + self.list_state.offset();
                    if i < self.list_names().len() {
                        self.focus = Focus::List;
                        return self.select(i);
                    }
                }
                if inside(self.areas.view) && self.has_content() {
                    self.focus = Focus::Content;
                }
                if inside(self.areas.command) {
                    self.open_command();
                }
            }
            MouseEventKind::ScrollUp if inside(self.areas.output) => self.output.scroll += 3,
            MouseEventKind::ScrollDown if inside(self.areas.output) => self.output.scroll = self.output.scroll.saturating_sub(3),
            MouseEventKind::ScrollUp if inside(self.areas.view) => self.scroll(true, 3),
            MouseEventKind::ScrollDown if inside(self.areas.view) => self.scroll(false, 3),
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
        // 되돌리기와 (데몬·GUI를 고치는) 복구는 데몬과 이 프로그램을 끝낸다
        Cmd::Shell | Cmd::Tui { .. } | Cmd::Completions { .. } | Cmd::Install(_) | Cmd::Update(UpdateCmd::Rollback | UpdateCmd::Repair) => Some(t!("tui.not_here")),
        Cmd::Gui { .. } => Some(t!("tui.use_switch")),
        Cmd::Monitor(a) if a.watch => Some(t!("tui.not_here")),
        // 모듈·익스텐션만 적용하는 것은 업데이터를 띄우지 않는다: 메뉴 안에서 된다
        Cmd::Update(UpdateCmd::Apply { components }) if components.is_empty() || !components.iter().all(|c| c.starts_with("module-") || c.starts_with("ext-")) => {
            Some(t!("tui.not_here"))
        }
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

    fn ctrl(c: char) -> KeyEvent {
        KeyEvent { modifiers: KeyModifiers::CONTROL, ..key(KeyCode::Char(c)) }
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

    fn parses(line: &str) -> bool {
        parsed(&shlex::split(line).unwrap()).is_some()
    }

    fn settings_doc() -> Value {
        json!({ "server": "my server", "fields": [
            { "key": "difficulty", "label": "난이도", "type": "select", "group": null, "value": "normal", "options": ["easy", "normal", "hard"] },
            { "key": "pvp", "label": "PVP", "type": "boolean", "group": null, "value": true },
        ]})
    }

    /// 쪽·탭마다 버튼의 명령은 모두 실제 명령이고 문구가 있다 (명령 표가 바뀌어 버튼이 깨지지 않게)
    #[test]
    fn every_page_button_is_a_real_command() {
        crate::i18n::init("ko");
        for page in Page::ALL {
            for tab in 0..page.tabs().len().max(1) {
                for (key, template, _) in page_buttons(page, tab) {
                    assert!(crate::i18n::try_t(&format!("tui.btn.{key}")).is_some(), "tui.btn.{key} has no label");
                    if *template == WIZARD {
                        continue;
                    }
                    for line in template.lines() {
                        let mut words = shlex::split(&line.replace("{}", "x")).unwrap();
                        // 채워 여는 버튼은 사용자가 마저 쓴다: 인자 하나를 더해 읽어 본다
                        if template.ends_with(' ') {
                            words.push("y=z".into());
                        }
                        assert!(parsed(&words).is_some(), "{page:?}/{key}: `{line}` does not parse");
                        assert!(refuse(&words).is_none(), "{page:?}/{key}: `{line}` is refused in the menu");
                    }
                }
            }
            assert!(crate::i18n::try_t(&format!("tui.page.{}", page.key())).is_some(), "{page:?} has no title");
            for tab in page.tabs() {
                assert_ne!(page.tab_label(tab), "", "{page:?}/{tab}");
            }
        }
        assert!(crate::i18n::try_t("tui.btn.more").is_some());
    }

    /// 상세 탭 아래 `같은 명령`과 글 탭의 `e`(바꾸기) 앞부분은 실제 명령이다
    #[test]
    fn the_same_command_and_edit_prefixes_are_real_commands() {
        let mut a = app();
        a.snap.guilds = vec![json!({ "guildId": "123", "guildName": "길드" })];
        for page in [Page::Servers, Page::Discord, Page::Settings] {
            a.go(page);
            for tab in 0..page.tabs().len() {
                a.tab[page.slot()] = tab;
                if let Some(line) = a.same_command() {
                    assert!(parses(&line), "{page:?}/{}: `{line}`", a.tab_key());
                }
                if let Some((_, Load::Capture(lines))) = a.pane_want() {
                    for line in lines {
                        assert!(parses(&line), "{page:?}/{}: capture `{line}`", a.tab_key());
                    }
                }
            }
        }
    }

    /// 폼 저장 명령: 바뀐 칸이 저장 명령 한 줄(권한은 줄마다 하나)이 되고, 모두 실제 명령이다
    #[test]
    fn form_savers_build_real_commands() {
        crate::i18n::init("ko");
        let c = |pairs: &[(&str, &str)]| pairs.iter().map(|(k, v)| (k.to_string(), v.to_string())).collect::<Vec<_>>();
        let cases = [
            (Saver::ServerSet("마크 야생".into()), c(&[("difficulty", "hard"), ("motd", "hello world")])),
            (Saver::Config, c(&[("language", "ko"), ("discord.prefix", "!")])),
            (Saver::Bot("music", "123".into()), c(&[("enabled", "true"), ("ui", "false"), ("queue-lines", "7")])),
            (Saver::Bot("tts", "123".into()), c(&[("speed", "1.1"), ("names", "never"), ("keep-read", "true")])),
            (Saver::Perm("123".into()), c(&[("start", "admin"), ("member:42", "guest")])),
        ];
        for (saver, changes) in cases {
            for line in saver.lines(&changes) {
                assert!(parses(&line), "{saver:?}: `{line}`");
            }
        }
        assert_eq!(Saver::Bot("music", "123".into()).lines(&c(&[("enabled", "true"), ("ui", "false")])), ["bot music 123 --on --ui off"]);
        assert_eq!(
            Saver::Perm("123".into()).lines(&c(&[("start", "admin"), ("member:42", "guest")])),
            ["perm command 123 start admin", "perm grant 123 42 guest"]
        );
        assert_eq!(Saver::ServerSet("마크 야생".into()).lines(&c(&[("motd", "a b")])), ["server set '마크 야생' 'motd=a b'"]);
    }

    /// 보이는 방향대로 옮기고, Esc는 언제나 한 단계 뒤로: 내용 → 상세 탭 → 목록 → 쪽 탭
    #[test]
    fn arrows_follow_the_screen_and_esc_goes_back_one_level() {
        let mut a = app();
        a.focus = Focus::Tabs;
        assert_eq!(a.on_key(key(KeyCode::Right)), Some(Action::Refresh));
        assert_eq!(a.page, Page::Discord);
        a.on_key(key(KeyCode::Left));
        a.on_key(key(KeyCode::Down));
        assert_eq!(a.focus, Focus::List);
        assert_eq!(a.on_key(key(KeyCode::Down)), Some(Action::Refresh));
        assert_eq!(a.selected().as_deref(), Some("pal"));
        a.on_key(key(KeyCode::Right));
        assert_eq!(a.focus, Focus::Detail);
        // 상세 탭에서 →는 다음 탭, ↓는 곧바로 내용
        assert_eq!(a.on_key(key(KeyCode::Right)), Some(Action::Refresh));
        assert_eq!(a.tab_key(), "settings");
        a.on_key(key(KeyCode::Down));
        assert_eq!(a.focus, Focus::Content);
        for want in [Focus::Detail, Focus::List, Focus::Tabs] {
            a.on_key(key(KeyCode::Esc));
            assert_eq!(a.focus, want);
        }
        // 숫자 키로 쪽 바로 가기
        assert_eq!(a.on_key(key(KeyCode::Char('5'))), Some(Action::Refresh));
        assert_eq!(a.page, Page::Settings);
    }

    /// 서버 쪽 글자 키: s 시작/정지, r 재시작, n 새 서버. 콘솔에 치는 중에는 글자다
    #[test]
    fn server_keys_start_stop_restart_and_open_the_wizard() {
        let mut a = app();
        assert_eq!(a.on_key(key(KeyCode::Char('s'))), Some(Action::Run("start 'my server'".into())));
        assert_eq!(a.on_key(key(KeyCode::Char('r'))), None, "a stopped server has no restart");
        a.list_state.select(Some(1));
        assert_eq!(a.on_key(key(KeyCode::Char('s'))), Some(Action::Run("stop pal".into())));
        assert_eq!(a.on_key(key(KeyCode::Char('r'))), Some(Action::Run("restart pal".into())));
        a.focus = Focus::Content;
        assert_eq!(a.tab_key(), "console");
        assert_eq!(a.on_key(key(KeyCode::Char('s'))), None);
        assert_eq!(a.server_input.text, "s");
        a.server_input.clear();
        a.focus = Focus::List;
        a.on_key(key(KeyCode::Char('n')));
        assert!(a.wizard.is_some());
        a.on_key(key(KeyCode::Esc));
        assert!(a.wizard.is_none());
    }

    /// 버튼은 서버 상태에 맞는 것만, 덜 쓰는 것은 [더 보기]에. 단축 키가 붙는다
    #[test]
    fn buttons_follow_the_server_state_and_rare_ones_go_under_more() {
        let mut a = app();
        let stopped = a.buttons();
        assert_eq!((stopped[0].line.as_str(), stopped[0].key), ("start 'my server'", Some('s')));
        assert!(!lines(&a).iter().any(|l| l.starts_with("stop") || l.starts_with("server rm")));
        a.list_state.select(Some(1));
        assert_eq!(&lines(&a)[..2], ["stop pal", "restart pal"]);
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

    /// 설정 탭: 읽어 온 폼에서 값을 바꾸고 Ctrl+S → `server set`. 다른 서버의 늦은 폼은 버린다
    #[test]
    fn the_settings_tab_saves_through_server_set() {
        let mut a = app();
        a.tab[Page::Servers.slot()] = 1;
        let (k, load) = a.pane_want().unwrap();
        assert_eq!(load, Load::Fetch(super::super::data::PaneWant::ServerSettings("s1".into())));
        a.apply_pane("servers:settings:s2".into(), super::super::data::PaneData::Settings(settings_doc()));
        assert!(!a.pane_is_current(), "a late form for another server is dropped");
        a.apply_pane(k, super::super::data::PaneData::Settings(settings_doc()));
        assert!(a.pane_is_current());
        a.focus = Focus::Content;
        a.on_key(key(KeyCode::Right));
        assert_eq!(a.on_key(ctrl('s')), Some(Action::Run("server set 'my server' difficulty=hard".into())));
        // Esc: 바뀐 칸 되돌리기 → 상세 탭
        a.on_key(key(KeyCode::Esc));
        assert_eq!(a.focus, Focus::Content);
        a.on_key(key(KeyCode::Esc));
        assert_eq!(a.focus, Focus::Detail);
    }

    /// 폼에서는 글자 키가 서버를 끄거나 메뉴를 닫지 않는다. 지운 칸은 기본값으로 저장한다. 비밀번호는 명령 줄에서 가린다
    #[test]
    fn a_form_keeps_letter_keys_and_hides_secrets() {
        let mut a = app();
        a.list_state.select(Some(1));
        a.tab[Page::Servers.slot()] = 1;
        let (k, _) = a.pane_want().unwrap();
        let mut doc = settings_doc();
        doc["fields"].as_array_mut().unwrap().push(json!({ "key": "rcon_password", "label": "RCON", "type": "password", "group": null, "value": "old" }));
        a.apply_pane(k, super::super::data::PaneData::Settings(doc));
        a.focus = Focus::Content;
        for c in ['s', 'r', 'q', '2'] {
            assert_eq!(a.on_key(key(KeyCode::Char(c))), None, "{c}");
        }
        assert_eq!((a.page, a.focus), (Page::Servers, Focus::Content));
        a.on_key(key(KeyCode::Down));
        a.on_key(key(KeyCode::Down));
        a.on_key(key(KeyCode::Enter));
        for _ in 0..3 {
            a.on_key(key(KeyCode::Backspace));
        }
        typed(&mut a, "hunter2");
        a.on_key(key(KeyCode::Enter));
        let Some(Action::Run(line)) = a.on_key(ctrl('s')) else { panic!("save") };
        assert!(line.contains("hunter2"));
        assert!(!a.masked(&line).contains("hunter2"));
        let c = |pairs: &[(&str, &str)]| pairs.iter().map(|(k, v)| (k.to_string(), v.to_string())).collect::<Vec<_>>();
        assert_eq!(Saver::Bot("music", "1".into()).lines(&c(&[("ui", ""), ("refresh", "3000")])), ["bot music 1 --refresh 3000"]);
        let perm = Saver::Perm("1".into()).lines(&c(&[("start", ""), ("member:42", "")]));
        assert_eq!(perm, ["perm command 1 start", "perm revoke 1 42"]);
        assert!(perm.iter().all(|l| parses(l)));
    }

    /// 새 서버 창은 다른 쪽으로 가면 닫힌다
    #[test]
    fn the_wizard_closes_on_another_page() {
        let mut a = app();
        a.on_key(key(KeyCode::Char('n')));
        assert!(a.wizard.is_some());
        a.go(Page::Updates);
        assert!(a.wizard.is_none());
    }

    /// 명령 출력으로 보이는 탭은 출력 창이 아니라 그 탭에 받는다 (`saba-chan-cli` 머리는 뗀다)
    #[test]
    fn captured_output_goes_into_the_tab() {
        let mut a = app();
        a.tab[Page::Servers.slot()] = 4;
        let (k, load) = a.pane_want().unwrap();
        assert_eq!(load, Load::Capture(vec!["schedule show 'my server'".into()]));
        a.begin_capture(k);
        a.command_output("예약 없음\n  saba-chan-cli schedule add x");
        assert!(a.output.lines.is_empty() && !a.output.open);
        match &a.pane.state {
            PaneState::Text(l) => assert_eq!(l, &["예약 없음", "  schedule add x"]),
            _ => panic!("not text"),
        }
        a.focus = Focus::Content;
        a.on_key(key(KeyCode::Char('e')));
        assert_eq!((a.focus, a.command.text.as_str()), (Focus::Command, "schedule add 'my server' "));
        // 탭 읽기를 멈추고 곧바로 다른 명령을 실행해도 그 출력은 출력 창으로 간다
        a.begin_command("status");
        a.command_output("데몬 실행 중");
        assert_eq!(a.toast.as_deref(), Some("데몬 실행 중"));
    }

    /// 백업 표: c 만들기, Enter 되돌리기, d 지우기
    #[test]
    fn the_backup_table_creates_restores_and_deletes() {
        let mut a = app();
        a.tab[Page::Servers.slot()] = 3;
        let (k, _) = a.pane_want().unwrap();
        a.apply_pane(k, super::super::data::PaneData::Rows(vec![json!({ "name": "b1" }), json!({ "name": "b 2" })]));
        a.focus = Focus::Content;
        assert_eq!(a.on_key(key(KeyCode::Char('c'))), Some(Action::Run("backup create 'my server'".into())));
        a.on_key(key(KeyCode::Down));
        assert_eq!(a.on_key(key(KeyCode::Enter)), Some(Action::Run("backup restore 'my server' 'b 2'".into())));
        assert_eq!(a.on_key(key(KeyCode::Char('d'))), Some(Action::Run("backup delete 'my server' 'b 2'".into())));
        assert!(parses("backup restore 'my server' 'b 2'") && parses("backup delete x y") && parses("backup create x"));
    }

    /// 무엇이든 찾기: Ctrl+K, 초성으로 찾고 Enter로 그 화면, Tab은 같은 명령을 명령 줄에
    #[test]
    fn the_palette_finds_screens_by_initials_and_goes_there() {
        let mut a = app();
        a.on_key(ctrl('k'));
        assert!(a.palette.is_some());
        typed(&mut a, "ㅇㅅㅈ");
        a.on_key(key(KeyCode::Enter));
        assert!(a.palette.is_none());
        assert_eq!((a.page, a.focus), (Page::Settings, Focus::Tabs));
        // 서버 탭: 그 서버·탭으로
        a.on_key(ctrl('k'));
        typed(&mut a, "백업");
        let pick = a.palette.as_ref().unwrap().results().iter().position(|e| e.path.contains("pal")).unwrap();
        for _ in 0..pick {
            a.on_key(key(KeyCode::Down));
        }
        assert_eq!(a.on_key(key(KeyCode::Enter)), Some(Action::Refresh));
        assert_eq!((a.page, a.selected().as_deref(), a.tab_key(), a.focus), (Page::Servers, Some("pal"), "backups", Focus::Detail));
        a.on_key(ctrl('k'));
        a.on_key(key(KeyCode::Esc));
        assert!(a.palette.is_none());
    }

    /// 게임 모듈 탭: 새 판이 있을 때만 [업데이트] (받고 적용), [새 서버]는 그 게임을 골라 마법사를 연다
    #[test]
    fn the_module_tab_updates_and_starts_a_server_with_that_game() {
        let mut a = app();
        a.snap.modules = vec![json!({ "name": "minecraft" }), json!({ "name": "palworld" })];
        a.go(Page::Addons);
        a.list_state.select(Some(1));
        let comp = |now: &str, latest: &str| json!({ "components": [{ "component": "module-palworld", "current_version": now, "latest_version": latest, "update_available": true }] });
        a.snap.updates = Some(comp("0.1.12", "0.1.12"));
        assert!(!lines(&a).iter().any(|l| l.starts_with("update")), "the same version is not an update");
        a.snap.updates = Some(comp("0.1.12", "0.1.13"));
        a.focus = Focus::Actions;
        a.button = 0;
        assert_eq!(a.on_key(key(KeyCode::Enter)), Some(Action::RunAll(vec!["update download module-palworld".into(), "update apply module-palworld".into()])));
        a.button = a.buttons().iter().position(|b| b.line == WIZARD).unwrap();
        a.on_key(key(KeyCode::Enter));
        assert_eq!((a.page, a.wizard.as_ref().map(|w| w.module)), (Page::Servers, Some(1)));
        // 코어 업데이트 적용은 여전히 메뉴 밖에서
        assert!(refuse(&shlex::split("update apply saba-core").unwrap()).is_some());
        assert!(refuse(&shlex::split("update apply").unwrap()).is_some());
        assert!(refuse(&shlex::split("update rollback").unwrap()).is_some());
        assert!(refuse(&shlex::split("update repair").unwrap()).is_some());
    }

    /// 찾기 결과에 보이는 화면의 같은 명령은 실제 명령이다
    #[test]
    fn palette_screen_commands_are_real_commands() {
        let mut a = app();
        a.snap.guilds = vec![json!({ "guildId": "1", "guildName": "길드" })];
        let entries = a.palette_entries();
        assert!(entries.iter().any(|e| e.kind == EntryKind::Command));
        for e in entries.iter().filter(|e| e.kind == EntryKind::Screen && !e.cmd.is_empty()) {
            assert!(parses(&e.cmd), "{}: `{}`", e.title, e.cmd);
        }
    }

    /// 읽어 둔 설정 칸도 찾는다: 고르면 폼의 그 칸에 커서
    #[test]
    fn the_palette_jumps_to_a_loaded_setting() {
        let mut a = app();
        a.tab[Page::Servers.slot()] = 1;
        let (k, _) = a.pane_want().unwrap();
        a.apply_pane(k, super::super::data::PaneData::Settings(settings_doc()));
        a.go(Page::Discord);
        a.go(Page::Servers);
        a.on_key(ctrl('k'));
        typed(&mut a, "ㄴㅇㄷ");
        let e = a.palette.as_ref().unwrap().selected().unwrap().clone();
        assert_eq!((e.kind, e.cmd.as_str()), (EntryKind::Setting, "server set 'my server' difficulty="));
        a.on_key(key(KeyCode::Enter));
        assert_eq!(a.focus, Focus::Content);
        match &a.pane.state {
            PaneState::Form(f, _) => assert_eq!(f.current().map(|x| x.key.as_str()), Some("difficulty")),
            _ => panic!("not a form"),
        }
    }

    /// 콘솔: 친 줄은 그 서버로. 첫 낱말을 치는 동안 게임 명령을 제안하고 Tab으로 채운다
    #[test]
    fn the_console_sends_lines_and_suggests_game_commands() {
        let mut a = app();
        a.list_state.select(Some(1));
        a.focus = Focus::Content;
        let cmd = |name: &str, label: &str, inputs: Vec<crate::catalog::Input>| crate::catalog::Command {
            name: name.into(),
            label: label.into(),
            description: String::new(),
            risk: crate::catalog::Risk::Safe,
            method: String::new(),
            aliases: Vec::new(),
            inputs,
            extension: None,
        };
        let player = crate::catalog::Input {
            name: "player".into(),
            label: "플레이어".into(),
            kind: "text".into(),
            required: true,
            placeholder: None,
            default: None,
            min: None,
            max: None,
            options: Vec::new(),
        };
        let cat = crate::catalog::Catalog { commands: vec![cmd("kick", "추방", vec![player]), cmd("say", "말하기", Vec::new())], interaction_mode: None };
        a.catalog = Some(("s2".into(), cat));
        typed(&mut a, "k");
        let found = a.suggestions();
        assert_eq!(found.len(), 1);
        assert_eq!((found[0].0.as_str(), found[0].1.as_str()), ("kick", " <플레이어>"));
        a.on_key(key(KeyCode::Tab));
        assert_eq!(a.server_input.text, "kick ");
        assert_eq!(a.focus, Focus::Content, "Tab fills instead of moving focus");
        typed(&mut a, "bob");
        assert_eq!(a.on_key(key(KeyCode::Enter)), Some(Action::Send { server: "s2".into(), text: "kick bob".into() }));
        // 초성으로도
        typed(&mut a, "ㅁㅎ");
        assert_eq!(a.suggestions().first().map(|s| s.0.as_str()), Some("say"));
        a.on_key(key(KeyCode::Esc));
        assert!(a.server_input.text.is_empty());
        a.on_key(key(KeyCode::Esc));
        assert_eq!(a.focus, Focus::Detail);
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
        assert_eq!(a.focus, Focus::List);
        a.on_key(key(KeyCode::Char(':')));
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
        a.focus = Focus::Detail;
        a.on_key(key(KeyCode::Esc));
        assert!(!a.output.open);
        assert_eq!(a.focus, Focus::Detail, "the first Esc only closes the output");
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
        a.on_key(ctrl('c'));
        assert_eq!(rx.recv().unwrap(), None);

        a.running = true;
        assert_eq!(a.on_key(ctrl('c')), Some(Action::Cancel));
        assert_eq!(a.on_key(key(KeyCode::Char('q'))), None);
        a.running = false;
        a.on_key(key(KeyCode::Char(':')));
        typed(&mut a, "server");
        assert_eq!(a.on_key(ctrl('c')), None);
        assert!(a.command.text.is_empty());
        assert_eq!(a.on_key(ctrl('c')), Some(Action::Quit));
    }

    /// GUI로 전환은 데몬이 GUI를 띄울 수 있다고 할 때만, 고른 서버를 넘긴다
    #[test]
    fn switching_to_the_gui_needs_a_gui_and_carries_the_server() {
        let mut a = app();
        assert_eq!(a.on_key(key(KeyCode::F(9))), None);
        a.snap.launchable = Some(json!({ "cli": true, "gui": true }));
        a.list_state.select(Some(1));
        a.go(Page::Settings);
        assert_eq!(a.on_key(key(KeyCode::F(9))), Some(Action::SwitchToGui));
        assert_eq!(a.current_server_id().as_deref(), Some("s2"));
    }

    /// `tui --server`·GUI에서 넘겨받은 서버를 목록이 오면 고른다
    #[test]
    fn the_server_handed_over_is_selected_when_the_list_arrives() {
        let mut a = app();
        let servers = std::mem::take(&mut a.snap.servers);
        a.open_server = Some("s2".into());
        a.go(Page::Settings);
        a.apply_open_server();
        assert!(a.open_server.is_some(), "waits for the list");
        a.snap.servers = servers;
        a.apply_open_server();
        assert_eq!((a.page, a.selected_index(), a.focus), (Page::Servers, 1, Focus::Detail));
        assert!(a.open_server.is_none());
    }

    /// 서버를 빨리 바꾸면 앞 서버의 늦은 콘솔 응답이 온다: 지금 고른 서버 것만 화면에 둔다
    #[test]
    fn a_late_console_for_another_server_is_dropped() {
        use super::super::data::View;
        let mut a = app();
        a.list_state.select(Some(1));
        a.apply_view(View::Console("s2".into(), vec![json!({ "content": "pal" })]));
        a.apply_view(View::Console("s1".into(), vec![json!({ "content": "old" })]));
        assert_eq!(a.snap.console.as_ref().map(|(id, l)| (id.as_str(), l.len())), Some(("s2", 1)));
    }

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
        assert!(refuse(&w("--yes update apply")).is_some());
        assert!(refuse(&w("updates apply")).is_some());
        assert_eq!(adapt(w("console pal")), w("console pal --follow"));
        assert_eq!(adapt(w("console pal --print")), w("console pal --print"));
        assert_eq!(adapt(w("server logs pal")), w("server logs pal --follow"));
        assert_eq!(adapt(w("--json console pal")), w("--json console pal"));
    }

    /// 다른 쪽에 다녀와도 쪽마다 고른 항목과 상세 탭이 남는다
    #[test]
    fn each_page_keeps_its_selection_and_tab() {
        let mut a = app();
        a.snap.servers.push(json!({ "id": "c", "name": "c" }));
        a.list_state.select(Some(2));
        a.tab[Page::Servers.slot()] = 3;
        a.go(Page::Discord);
        a.go(Page::Servers);
        assert_eq!((a.list_state.selected(), a.tab_key()), (Some(2), "backups"));
        a.go(Page::Addons);
        assert_eq!(a.list_state.selected(), Some(0));
    }
}
