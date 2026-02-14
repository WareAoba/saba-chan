//! 앱 상태 · 화면 네비게이션 · 공유 타입 정의
//!
//! `Screen` 열거형으로 화면을 관리하고, `screen_stack`으로 Esc(뒤로) 동작을 구현합니다.
//! 기존 커맨드 모드(Out 열거형) 호환성도 유지합니다.

use std::collections::HashMap;
use std::sync::{Arc, Mutex};

use serde_json::Value;

use crate::cli_config::CliSettings;
use crate::client::DaemonClient;
use crate::gui_config;
use crate::i18n::I18n;
use crate::module_registry::ModuleRegistry;

// ═══════════════════════════════════════════════════════
// 출력 타입 (커맨드 모드 호환)
// ═══════════════════════════════════════════════════════

/// 출력 영역의 한 줄 — 기존 커맨드 모드에서 사용
#[derive(Clone)]
pub enum Out {
    Info(String),
    Cmd(String),
    Ok(String),
    Err(String),
    Text(String),
    Blank,
}

// ═══════════════════════════════════════════════════════
// 서버 상태
// ═══════════════════════════════════════════════════════

#[derive(Clone)]
pub struct ServerInfo {
    pub name: String,
    pub module: String,
    pub status: String,
}

/// 백그라운드 상태 스냅샷 (모니터 태스크 → App)
pub struct Snapshot {
    pub daemon: bool,
    pub bot: bool,
    pub token: bool,
    pub prefix: String,
    pub servers: Vec<ServerInfo>,
}

// ═══════════════════════════════════════════════════════
// 공유 타입
// ═══════════════════════════════════════════════════════

pub type OutputBuf = Arc<Mutex<Vec<Out>>>;
pub type SharedClientId = Arc<Mutex<Option<String>>>;

pub fn push_out(buf: &OutputBuf, lines: Vec<Out>) {
    let mut b = buf.lock().unwrap();
    b.extend(lines);
    b.push(Out::Blank);
}

// ═══════════════════════════════════════════════════════
// 화면 (Screen) 열거형
// ═══════════════════════════════════════════════════════

#[derive(Clone, Debug)]
#[allow(dead_code)]
pub enum Screen {
    Dashboard,
    Servers,
    ServerDetail  { name: String, id: String, module_name: String },
    ServerConsole { name: String, id: String },
    ServerSettings { name: String, id: String, module_name: String },
    ServerProperties { name: String, id: String },
    Modules,
    ModuleDetail  { name: String },
    Bot,
    BotAliases,
    Settings,
    Updates,
    Daemon,
    CommandMode,
}

impl Screen {
    /// 브레드크럼 경로 반환
    pub fn breadcrumb(&self) -> Vec<&str> {
        match self {
            Self::Dashboard      => vec!["saba-chan"],
            Self::Servers        => vec!["saba-chan", "Servers"],
            Self::ServerDetail  { .. } => vec!["saba-chan", "Servers", "Detail"],
            Self::ServerConsole { .. } => vec!["saba-chan", "Servers", "Console"],
            Self::ServerSettings { .. } => vec!["saba-chan", "Servers", "Settings"],
            Self::ServerProperties { .. } => vec!["saba-chan", "Servers", "Properties"],
            Self::Modules        => vec!["saba-chan", "Modules"],
            Self::ModuleDetail { .. } => vec!["saba-chan", "Modules", "Detail"],
            Self::Bot            => vec!["saba-chan", "Discord Bot"],
            Self::BotAliases     => vec!["saba-chan", "Discord Bot", "Aliases"],
            Self::Settings       => vec!["saba-chan", "Settings"],
            Self::Updates        => vec!["saba-chan", "Updates"],
            Self::Daemon         => vec!["saba-chan", "Daemon"],
            Self::CommandMode    => vec!["saba-chan", "Command"],
        }
    }
}

// ═══════════════════════════════════════════════════════
// 입력 모드
// ═══════════════════════════════════════════════════════

#[derive(Clone, Debug, PartialEq)]
pub enum InputMode {
    /// 메뉴 내비게이션 (↑↓ jk Enter Esc)
    Normal,
    /// 레거시 명령어 입력 (타이핑)
    Command,
    /// 필드 인라인 편집 (vim-like)
    Editing,
    /// 서버 콘솔 stdin 입력
    Console,
    /// 확인 대화상자 (y/n)
    Confirm { prompt: String, action: ConfirmAction },
}

#[derive(Clone, Debug, PartialEq)]
#[allow(dead_code)]
pub enum ConfirmAction {
    DeleteInstance(String), // id
    StopServer(String),     // name
    AcceptEula(String),     // id
}

// ═══════════════════════════════════════════════════════
// 메뉴 아이템
// ═══════════════════════════════════════════════════════

#[derive(Clone)]
pub struct MenuItem {
    pub label: String,
    pub shortcut: Option<char>,
    pub description: String,
    pub enabled: bool,
    pub badge: Option<String>,  // 우측에 표시할 뱃지 (예: 서버 수, 상태)
}

impl MenuItem {
    pub fn new(label: &str, shortcut: Option<char>, desc: &str) -> Self {
        Self {
            label: label.into(),
            shortcut,
            description: desc.into(),
            enabled: true,
            badge: None,
        }
    }

    pub fn with_enabled(mut self, enabled: bool) -> Self {
        self.enabled = enabled;
        self
    }
}

// ═══════════════════════════════════════════════════════
// 에디터 필드 (vim-like 프로퍼티 편집기용)
// ═══════════════════════════════════════════════════════

#[derive(Clone)]
pub struct EditorField {
    pub key: String,
    pub value: String,
    pub original_value: String,
    pub group: String,
    pub field_type: String,       // text, number, select, boolean, password, file
    pub label: String,
    pub required: bool,
    pub options: Vec<String>,     // select 타입 옵션 목록
}

// ═══════════════════════════════════════════════════════
// App (메인 상태)
// ═══════════════════════════════════════════════════════

#[allow(dead_code)]
pub struct App {
    pub client: DaemonClient,

    // ── 실시간 상태 ──
    pub daemon_on: bool,
    pub bot_on: bool,
    pub token_ok: bool,
    pub bot_prefix: String,
    pub servers: Vec<ServerInfo>,

    // ── 설정 · i18n ──
    pub registry: Arc<ModuleRegistry>,
    pub settings: CliSettings,
    pub i18n: Arc<I18n>,

    // ── 하트비트 ──
    pub client_id: SharedClientId,

    // ── 화면 내비게이션 ──
    pub screen: Screen,
    pub screen_stack: Vec<Screen>,
    pub input_mode: InputMode,

    // ── 메뉴 상태 ──
    pub menu_items: Vec<MenuItem>,
    pub menu_selected: usize,

    // ── 에디터 상태 ──
    pub editor_fields: Vec<EditorField>,
    pub editor_selected: usize,
    pub edit_buffer: String,
    pub edit_cursor: usize,
    pub editor_changes: HashMap<String, String>,

    // ── 커맨드 모드 (레거시 호환) ──
    pub input: String,
    pub cursor: usize,
    pub output: Vec<Out>,
    pub history: Vec<String>,
    pub hist_idx: Option<usize>,
    pub scroll_up: usize,
    pub output_height: usize,

    // ── 콘솔 모드 ──
    pub console_lines: Vec<String>,
    pub console_input: String,
    pub console_scroll: usize,

    // ── 캐시 데이터 ──
    pub cached_instances: Vec<Value>,
    pub cached_modules: Vec<Value>,
    pub cached_server_detail: Option<Value>,
    pub cached_module_detail: Option<Value>,
    pub cached_update_status: Option<Value>,

    // ── 로딩 · 상태 메시지 ──
    pub loading: Option<String>,
    pub status_message: Option<(String, std::time::Instant)>,

    // ── 제어 ──
    pub quit: bool,
    pub async_out: OutputBuf,
}

impl App {
    pub fn new(client: DaemonClient) -> Self {
        let settings = CliSettings::load();
        let lang = settings.effective_language();
        let i18n = Arc::new(I18n::load(&lang));

        let token_ok = gui_config::get_discord_token().ok().flatten().is_some();
        let bot_prefix = if settings.bot_prefix.is_empty() {
            gui_config::get_bot_prefix().unwrap_or_else(|_| "!saba".into())
        } else {
            settings.bot_prefix.clone()
        };

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

            screen: Screen::CommandMode,
            screen_stack: vec![],
            input_mode: InputMode::Command,

            menu_items: vec![],
            menu_selected: 0,

            editor_fields: vec![],
            editor_selected: 0,
            edit_buffer: String::new(),
            edit_cursor: 0,
            editor_changes: HashMap::new(),

            input: String::new(),
            cursor: 0,
            output: vec![Out::Info(welcome), Out::Blank],
            history: vec![],
            hist_idx: None,
            scroll_up: 0,
            output_height: 20,

            console_lines: vec![],
            console_input: String::new(),
            console_scroll: 0,

            cached_instances: vec![],
            cached_modules: vec![],
            cached_server_detail: None,
            cached_module_detail: None,
            cached_update_status: None,

            loading: None,
            status_message: None,

            quit: false,
            async_out: Arc::new(Mutex::new(Vec::new())),
        }
    }

    // ── 화면 전환 ──

    /// 새 화면으로 이동 (현재 화면을 스택에 push)
    pub fn push_screen(&mut self, screen: Screen) {
        let old = std::mem::replace(&mut self.screen, screen);
        self.screen_stack.push(old);
        self.menu_selected = 0;
        self.input_mode = InputMode::Normal;
    }

    /// 이전 화면으로 복귀 (스택에서 pop)
    pub fn pop_screen(&mut self) -> bool {
        if let Some(prev) = self.screen_stack.pop() {
            self.screen = prev;
            self.menu_selected = 0;
            self.input_mode = InputMode::Normal;
            self.editor_fields.clear();
            self.editor_changes.clear();
            true
        } else {
            false
        }
    }

    #[allow(dead_code)]
    pub fn go_home(&mut self) {
        self.screen = Screen::Dashboard;
        self.screen_stack.clear();
        self.menu_selected = 0;
        self.input_mode = InputMode::Normal;
    }

    // ── 상태 갱신 ──

    pub fn apply_status(&mut self, snap: Snapshot) {
        self.daemon_on = snap.daemon;
        self.bot_on = snap.bot;
        self.token_ok = snap.token;
        self.bot_prefix = snap.prefix;
        self.servers = snap.servers;
    }

    #[allow(dead_code)]
    pub fn flush_async(&mut self) {
        let drained = {
            let mut buf = self.async_out.lock().unwrap();
            if buf.is_empty() { return; }
            buf.drain(..).collect::<Vec<_>>()
        };
        let cmd_start = self.output.len().saturating_sub(1);
        self.output.extend(drained);
        self.smart_scroll(cmd_start);
    }

    pub fn smart_scroll(&mut self, cmd_start: usize) {
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

    // ── 메뉴 조작 ──

    pub fn menu_up(&mut self) {
        if self.menu_items.is_empty() { return; }
        if self.menu_selected > 0 {
            self.menu_selected -= 1;
        } else {
            self.menu_selected = self.menu_items.len() - 1;
        }
    }

    pub fn menu_down(&mut self) {
        if self.menu_items.is_empty() { return; }
        self.menu_selected = (self.menu_selected + 1) % self.menu_items.len();
    }

    /// 단축키로 메뉴 아이템 선택 — 일치하면 true
    pub fn try_shortcut(&mut self, ch: char) -> bool {
        for (i, item) in self.menu_items.iter().enumerate() {
            if item.shortcut == Some(ch) && item.enabled {
                self.menu_selected = i;
                return true;
            }
        }
        false
    }

    // ── 에디터 조작 ──

    pub fn editor_up(&mut self) {
        if self.editor_selected > 0 { self.editor_selected -= 1; }
    }

    pub fn editor_down(&mut self) {
        if self.editor_selected + 1 < self.editor_fields.len() {
            self.editor_selected += 1;
        }
    }

    /// i 키: 선택된 필드의 편집 시작
    pub fn enter_edit_mode(&mut self) {
        let field = match self.editor_fields.get(self.editor_selected) {
            Some(f) => f.clone(),
            None => return,
        };

        match field.field_type.as_str() {
            "boolean" => {
                // 불리언은 직접 토글
                let new_val = if field.value == "true" { "false" } else { "true" };
                self.editor_fields[self.editor_selected].value = new_val.to_string();
                self.editor_changes.insert(field.key.clone(), new_val.to_string());
            }
            "select" if !field.options.is_empty() => {
                // 셀렉트는 다음 옵션으로 순환
                let idx = field.options.iter().position(|o| o == &field.value).unwrap_or(0);
                let next = (idx + 1) % field.options.len();
                let new_val = field.options[next].clone();
                self.editor_fields[self.editor_selected].value = new_val.clone();
                self.editor_changes.insert(field.key.clone(), new_val);
            }
            _ => {
                // 텍스트/숫자/패스워드: 인라인 편집 모드 진입
                self.edit_buffer = field.value.clone();
                self.edit_cursor = self.edit_buffer.chars().count();
                self.input_mode = InputMode::Editing;
            }
        }
    }

    /// 편집 확정 (Enter)
    pub fn commit_edit(&mut self) {
        if let Some(field) = self.editor_fields.get_mut(self.editor_selected) {
            field.value = self.edit_buffer.clone();
            self.editor_changes.insert(field.key.clone(), self.edit_buffer.clone());
        }
        self.input_mode = InputMode::Normal;
        self.edit_buffer.clear();
    }

    /// 편집 취소 (Esc)
    pub fn cancel_edit(&mut self) {
        self.input_mode = InputMode::Normal;
        self.edit_buffer.clear();
    }

    // ── 히스토리 (커맨드 모드) ──

    pub fn history_prev(&mut self) {
        if self.history.is_empty() { return; }
        let idx = match self.hist_idx {
            Some(i) => i.saturating_sub(1),
            None => self.history.len() - 1,
        };
        self.hist_idx = Some(idx);
        self.input = self.history[idx].clone();
        self.cursor = self.input.chars().count();
    }

    pub fn history_next(&mut self) {
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

    /// 일시적 상태 메시지 표시
    pub fn flash(&mut self, msg: &str) {
        self.status_message = Some((msg.to_string(), std::time::Instant::now()));
    }
}

// ═══════════════════════════════════════════════════════
// 유틸리티
// ═══════════════════════════════════════════════════════

/// char 인덱스 → 바이트 오프셋 (다국어 안전)
pub fn char_to_byte(s: &str, char_idx: usize) -> usize {
    s.char_indices()
        .nth(char_idx)
        .map(|(byte_pos, _)| byte_pos)
        .unwrap_or(s.len())
}

/// UTF-8 안전한 문자열 자르기
pub fn truncate_str(s: &str, max_chars: usize) -> String {
    let chars: Vec<char> = s.chars().collect();
    if chars.len() <= max_chars {
        s.to_string()
    } else {
        let truncated: String = chars[..max_chars].iter().collect();
        format!("{}…", truncated)
    }
}

/// start_time (UNIX 타임스탬프) → hh:mm:ss 경과시간
pub fn format_uptime(start_time: Option<u64>) -> String {
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

/// JSON 파일 저장
pub fn save_json_file(path: &std::path::PathBuf, data: &serde_json::Value) -> anyhow::Result<()> {
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)?;
    }
    let content = serde_json::to_string_pretty(data)?;
    std::fs::write(path, content)?;
    Ok(())
}
