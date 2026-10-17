//! 앱 상태 · 화면 네비게이션 · 공유 타입 정의
//!
//! `Screen` 열거형으로 화면을 관리하고, `screen_stack`으로 Esc(뒤로) 동작을 구현합니다.
//! 기존 커맨드 모드(Out 열거형) 호환성도 유지합니다.

use std::collections::HashMap;
use std::sync::{Arc, Mutex};

use serde_json::Value;

use tokio::sync::mpsc::{UnboundedReceiver, UnboundedSender};

use crate::cli_config::CliSettings;
use crate::client::DaemonClient;
use crate::config;
use crate::i18n::I18n;
use crate::module_registry::ModuleRegistry;

use super::tabs::{TabEvent, TabSet};

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
    /// 인스턴스별 익스텐션 데이터 (e.g. docker_enabled, docker_cpu_limit)
    /// GUI의 server.extension_data에 대응
    pub extension_data: HashMap<String, Value>,
}

/// 백그라운드 상태 스냅샷 (모니터 태스크 → App)
pub struct Snapshot {
    pub daemon: bool,
    pub servers: Vec<ServerInfo>,
    /// bot-config.json 전체 (데몬 API 경유). `None`이면 데몬 오프라인/오류.
    pub bot_config: Option<serde_json::Value>,
    /// settings.json 전체 (데몬 API 경유). `None`이면 데몬 오프라인/오류.
    pub settings_config: Option<serde_json::Value>,
}

// ═══════════════════════════════════════════════════════
// 공유 타입
// ═══════════════════════════════════════════════════════

pub type OutputBuf = Arc<Mutex<Vec<Out>>>;
pub type SharedClientId = Arc<Mutex<Option<String>>>;

pub fn push_out(buf: &OutputBuf, lines: Vec<Out>) {
    // mutex poisoning 시에도 TUI 패닉이 발생하지 않도록 `into_inner()`로 복구.
    let mut b = buf.lock().unwrap_or_else(|poisoned| poisoned.into_inner());
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
    ModuleManifest,
    Settings,
    Updates,
    Daemon,
    Extensions,
    ExtensionList,
    ExtensionDetail { ext_id: String, ext_name: String },
    ExtensionManifest,
    CommandMode,
    // ★ 인스턴스 생성 위자드 (신규)
    CreateInstanceStep1,
    CreateInstanceStep2 { module_name: String },
}

impl Screen {
    /// 브레드크럼 경로 반환
    pub fn breadcrumb(&self) -> Vec<&str> {
        match self {
            Self::Dashboard      => vec!["saba-chan"],
            Self::Servers        => vec!["saba-chan", "Instances"],
            Self::ServerDetail  { .. } => vec!["saba-chan", "Instances", "Detail"],
            Self::ServerConsole { .. } => vec!["saba-chan", "Instances", "Console"],
            Self::ServerSettings { .. } => vec!["saba-chan", "Instances", "Settings"],
            Self::ServerProperties { .. } => vec!["saba-chan", "Instances", "Properties"],
            Self::Modules        => vec!["saba-chan", "Modules"],
            Self::ModuleDetail { .. } => vec!["saba-chan", "Modules", "Detail"],
            Self::ModuleManifest => vec!["saba-chan", "Modules", "Manifest"],
            Self::Settings       => vec!["saba-chan", "Settings"],
            Self::Updates        => vec!["saba-chan", "Updates"],
            Self::Daemon         => vec!["saba-chan", "Saba-Core"],
            Self::Extensions     => vec!["saba-chan", "Extensions"],
            Self::ExtensionList  => vec!["saba-chan", "Extensions", "Installed"],
            Self::ExtensionDetail { .. } => vec!["saba-chan", "Extensions", "Detail"],
            Self::ExtensionManifest => vec!["saba-chan", "Extensions", "Manifest"],
            Self::CommandMode    => vec!["saba-chan", "Console"],
            // ★ 인스턴스 생성 위자드 (신규)
            Self::CreateInstanceStep1 => vec!["saba-chan", "Instances", "New", "Select Game"],
            Self::CreateInstanceStep2 { .. } => vec!["saba-chan", "Instances", "New", "Configure"],

        }
    }
}

// ═══════════════════════════════════════════════════════
// 입력 모드
// ═══════════════════════════════════════════════════════

// ═══════════════════════════════════════════════════════
// Focus — 키 이벤트 디스패치 상위 레이어
// ═══════════════════════════════════════════════════════

/// 이벤트 루프가 어느 컨테이너로 키를 보낼지 결정하는 상위 포커스 상태.
///
/// **Stage 0**: `Menu`만 실제로 사용되며 `InputMode` 분기를 그대로 태운다.
/// 이후 단계에서 다음과 같이 확장된다:
/// - `Tab(id)` — 탭 콘솔 (Stage 1~2)
/// - `Slash` — `/...` 슬래시 명령 오버레이 (Stage 4)
/// - `Overlay` — 확인/검색/파일 브라우저 등 모달 오버레이 (Stage 3~)
///
/// 의도적으로 `#[derive(Default)]`로 `Menu`를 기본값으로 지정해, Stage 0 도입 시
/// 기존 동작에 변화가 없도록 보장한다.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub enum Focus {
    /// 기존 InputMode 기반 메뉴/커맨드/편집 경로
    #[default]
    Menu,
    /// 탭 컨테이너 (Stage 1에서 활성화). usize는 `TabSet` 인덱스.
    #[allow(dead_code)]
    Tab(usize),
    /// 슬래시 명령 입력 오버레이 (Stage 4)
    #[allow(dead_code)]
    Slash,
    /// 확인/검색/파일 브라우저 등 모달 (Stage 3)
    #[allow(dead_code)]
    Overlay,
}

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
    /// 인라인 텍스트 입력 (TUI 내에서 직접 값 입력)
    InlineInput {
        prompt: String,
        value: String,
        cursor: usize,
        on_submit: InlineAction,
    },
    /// 인라인 선택 (TUI 내에서 옵션 목록 선택)
    InlineSelect {
        prompt: String,
        options: Vec<String>,
        selected: usize,
        on_submit: InlineAction,
    },
}

#[derive(Clone, Debug, PartialEq)]
#[allow(dead_code)]
pub enum InlineAction {
    SetCliSetting { key: String },
    SetGuiSetting { key: String },
    CreateInstance { module_name: String },
    ExecuteCommand { instance_id: String },
    RconCommand { instance_name: String },
    InstallModule { module_name: String },
    UpdateSet,
    Custom(String),
}

#[derive(Clone, Debug, PartialEq)]
#[allow(dead_code)]
pub enum ConfirmAction {
    DeleteInstance(String), // id
    StopServer(String),     // name
    ResetServer(String),    // id
    RemoveExtension(String), // ext_id
    InstallExtension(String), // ext_id
    RemoveModule(String),   // module_id
    InstallModuleFromManifest(String), // module_id
    ApplyInstanceUpdate(String), // instance name
    Custom(String),         // 범용 확인 액션
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
// 익스텐션 / 레지스트리 캐시 타입
// ═══════════════════════════════════════════════════════

/// 설치된 익스텐션 정보 (캐시)
#[derive(Clone, Debug)]
pub struct ExtensionInfo {
    pub id: String,
    pub name: String,
    pub version: String,
    pub enabled: bool,
}

// ═══════════════════════════════════════════════════════
// CLI 익스텐션 슬롯 시스템 — GUI ExtensionContext 대응
// ═══════════════════════════════════════════════════════

/// 익스텐션이 CLI 슬롯에 주입한 데이터 단위
#[derive(Clone, Debug)]
pub struct ExtSlotData {
    pub extension_id: String,
    pub extension_name: String,
    pub data: Value,
}

/// 익스텐션 인스턴스 필드 정의 (manifest.instance_fields 대응)
#[derive(Clone, Debug)]
#[allow(dead_code)]
pub struct FieldDefCli {
    pub field_type: String,      // boolean, number, string
    pub default_value: String,   // 기본값 문자열
    pub optional: bool,
}

/// CLI 슬롯 레지스트리 — GUI의 `ExtensionContext.slots`에 대응
///
/// 각 화면의 `build_*_menu()` 함수가 이 레지스트리를 조회하여
/// 익스텐션이 주입한 메뉴 아이템, 뱃지, 에디터 필드를 동적으로 병합합니다.
///
/// ## 슬롯 ID 규약 (GUI ↔ CLI 매핑)
///
/// | CLI Slot ID              | GUI Slot Equivalent        | 용도                        |
/// |--------------------------|----------------------------|-----------------------------|
/// | `InstanceList.badge`     | `ServerCard.badge`         | 인스턴스 목록 뱃지            |
/// | `InstanceDetail.menu`    | —                          | 상세 화면 메뉴 아이템          |
/// | `InstanceDetail.status`  | `ServerCard.expandedStats` | 상세 화면 상태 표시            |
/// | `InstanceSettings.fields`| `ServerSettings.tab`       | 설정 에디터 필드               |
/// | `CreateInstance.options`  | `AddServer.options`        | 인스턴스 생성 옵션             |
#[derive(Clone, Debug, Default)]
pub struct ExtensionSlotRegistry {
    /// slot_id → Vec<(extension_id, extension_name, slot_data)>
    slots: HashMap<String, Vec<ExtSlotData>>,
    /// extension_id → instance_fields 정의
    instance_fields: HashMap<String, HashMap<String, FieldDefCli>>,
}

impl ExtensionSlotRegistry {
    /// 특정 슬롯 ID에 등록된 모든 익스텐션 데이터 조회
    /// GUI의 `<ExtensionSlot slotId="...">` 에 대응
    pub fn get_slot(&self, slot_id: &str) -> &[ExtSlotData] {
        self.slots.get(slot_id).map(|v| v.as_slice()).unwrap_or(&[])
    }

    /// 모든 익스텐션의 instance_fields를 순회
    /// (extension_id, field_name, field_def) 튜플 반환
    #[allow(dead_code)]
    pub fn all_instance_fields(&self) -> Vec<(&str, &str, &FieldDefCli)> {
        self.instance_fields.iter()
            .flat_map(|(ext_id, fields)| {
                fields.iter().map(move |(name, def)| (ext_id.as_str(), name.as_str(), def))
            })
            .collect()
    }

    /// 전체 레지스트리 초기화 (익스텐션 데이터 재로드 시)
    pub fn clear(&mut self) {
        self.slots.clear();
        self.instance_fields.clear();
    }

    /// 단일 익스텐션의 슬롯 + instance_fields 등록
    pub fn register_extension(
        &mut self,
        ext_id: &str,
        ext_name: &str,
        cli_slots: Option<&Value>,
        instance_fields: &HashMap<String, Value>,
    ) {
        // CLI 슬롯 등록 (manifest.cli.slots)
        if let Some(slots_obj) = cli_slots.and_then(|v| v.get("slots")).and_then(|v| v.as_object()) {
            for (slot_id, slot_data) in slots_obj {
                self.slots.entry(slot_id.clone()).or_default().push(ExtSlotData {
                    extension_id: ext_id.to_string(),
                    extension_name: ext_name.to_string(),
                    data: slot_data.clone(),
                });
            }
        }

        // instance_fields 등록
        if !instance_fields.is_empty() {
            let mut fields = HashMap::new();
            for (field_name, field_def) in instance_fields {
                let ftype = field_def.get("type").and_then(|v| v.as_str()).unwrap_or("string").to_string();
                let default = match field_def.get("default") {
                    Some(Value::Bool(b)) => b.to_string(),
                    Some(Value::Number(n)) => n.to_string(),
                    Some(Value::String(s)) => s.clone(),
                    _ => String::new(),
                };
                let optional = field_def.get("optional").and_then(|v| v.as_bool()).unwrap_or(false);
                fields.insert(field_name.clone(), FieldDefCli {
                    field_type: ftype,
                    default_value: default,
                    optional,
                });
            }
            self.instance_fields.insert(ext_id.to_string(), fields);

            // instance_fields가 있지만 CLI 슬롯이 없는 경우 → 자동 파생
            // InstanceSettings.fields 슬롯을 자동 생성
            if !self.slots.contains_key("InstanceSettings.fields")
                || !self.slots["InstanceSettings.fields"].iter().any(|s| s.extension_id == ext_id)
            {
                let auto_fields: Vec<Value> = instance_fields.iter().map(|(name, def)| {
                    let ftype = def.get("type").and_then(|v| v.as_str()).unwrap_or("string");
                    let default = def.get("default").cloned().unwrap_or(Value::Null);
                    serde_json::json!({
                        "field": name,
                        "type": ftype,
                        "label": name,
                        "default": default,
                    })
                }).collect();

                self.slots.entry("InstanceSettings.fields".to_string()).or_default().push(ExtSlotData {
                    extension_id: ext_id.to_string(),
                    extension_name: ext_name.to_string(),
                    data: Value::Array(auto_fields),
                });
            }
        }
    }
}

/// 매니페스트 아이템 (모듈 또는 익스텐션 공용)
#[derive(Clone, Debug)]
pub struct ManifestItem {
    pub id: String,
    pub name: String,
    pub version: String,
    pub description: String,
}

// ═══════════════════════════════════════════════════════
// App (메인 상태)
// ═══════════════════════════════════════════════════════

#[allow(dead_code)]
pub struct App {
    pub client: DaemonClient,

    // ── 실시간 상태 ──
    pub daemon_on: bool,
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
    /// 키 라우팅 상위 레이어. Stage 0에서는 항상 `Menu`.
    pub focus: Focus,
    /// 전역 오버레이 (검색, 파일 브라우저 등). 활성 상태에서 키 이벤트 우선 소비.
    pub overlay: Option<super::overlay::Overlay>,

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

    // ── 멀티 콘솔 탭 (Stage 1) ──
    pub tabs: TabSet,
    /// 탭 폴러가 새 라인을 보낼 때 사용하는 송신자. `DaemonClient`와 함께 poll 태스크로 이동.
    pub tab_tx: UnboundedSender<TabEvent>,
    /// 이벤트 루프 시작 시 `take()`로 회수하는 일회성 수신자.
    /// `App`이 `new()`에서 생성해 event loop에 넘긴다.
    pub tab_rx: Option<UnboundedReceiver<TabEvent>>,

    // ── 자동완성 상태 ──
    pub autocomplete_candidates: Vec<String>,
    pub autocomplete_selected: usize,
    pub autocomplete_visible: bool,
    /// 입력 끝에 회색으로 렌더되는 ghost-text (Stage 4). Tab으로 수락.
    pub ghost_suffix: Option<String>,
    /// 슬래시 명령 입력 버퍼 (Stage 4). `None` = 슬래시 모드 아님.
    pub slash_buffer: Option<String>,
    pub slash_cursor: usize,
    /// 슬래시 명령 입력 히스토리 (Stage 5). `/`로 입력된 명령이 최신 뒤쪽으로 쌓임.
    pub slash_history: Vec<String>,
    /// 슬래시 히스토리 탐색 인덱스. `None` = 현재 입력 중.
    pub slash_hist_idx: Option<usize>,

    // ── 캐시 데이터 ──
    pub cached_instances: Vec<Value>,
    pub cached_modules: Vec<Value>,
    pub cached_server_detail: Option<Value>,
    pub cached_module_detail: Option<Value>,
    pub cached_update_status: Option<Value>,
    pub cached_extensions: Vec<ExtensionInfo>,
    pub cached_manifest_extensions: Vec<ManifestItem>,
    pub cached_manifest_modules: Vec<ManifestItem>,
    /// 봇 설정 캐시 (상태 모니터가 주기적으로 갱신). `None` = 아직 로드되지 않음.
    /// 렌더/메뉴 스레드는 반드시 이 캐시만 참조한다. 직접 `config::load_bot_config()`
    /// 호출 금지 — 데몬이 다운되면 3초 타임아웃이 매 프레임 발생.
    pub cached_bot_config: Option<Value>,
    /// GUI 설정(`settings.json`) 캐시. 상동.
    pub cached_settings: Option<Value>,

    // ── 익스텐션 슬롯 레지스트리 (GUI의 ExtensionContext에 대응) ──
    pub ext_slots: ExtensionSlotRegistry,

    // ── 로딩 · 상태 메시지 ──
    pub loading: Option<String>,
    pub status_message: Option<(String, std::time::Instant)>,

    // ── 데몬 로그 스트리밍 ──
    pub last_daemon_log_id: u64,
    pub last_daemon_log_fetch: Option<std::time::Instant>,

    // ── 제어 ──
    pub quit: bool,
    pub async_out: OutputBuf,

    /// TUI 종료 시 자동으로 데몬도 정지할지 여부.
    ///
    /// **이 세션이 직접 시작한 경우에만 true** — 외부에서 이미 떠 있던 데몬은
    /// 보호한다. 사용자가 단순 상태확인 목적으로 TUI를 띄웠다 빠져나갈 때
    /// 운영 중인 게임 서버가 함께 종료되는 사고를 방지하기 위한 플래그.
    pub stop_daemon_on_exit: Arc<std::sync::atomic::AtomicBool>,

    // ── 세션 영속화 (Stage 5) ──
    /// 시작 시 로드, 종료 시 저장. `None`이면 세션 기능 비활성.
    pub session: Option<super::session::Session>,
}

impl App {
    pub fn new(client: DaemonClient) -> Self {
        let settings = CliSettings::load();
        let lang = settings.effective_language();
        let i18n = Arc::new(I18n::load(&lang));

        let modules_path = config::get_modules_path().unwrap_or_default();
        let registry = Arc::new(ModuleRegistry::load(&modules_path));
        let welcome = i18n.t("welcome");

        let (tab_tx, tab_rx) = tokio::sync::mpsc::unbounded_channel::<TabEvent>();

        Self {
            client,
            daemon_on: false,
            servers: vec![],
            registry,
            settings,
            i18n,
            client_id: Arc::new(Mutex::new(None)),

            screen: Screen::Dashboard,
            screen_stack: vec![],
            input_mode: InputMode::Normal,
            focus: Focus::default(),
            overlay: None,

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

            autocomplete_candidates: vec![],
            autocomplete_selected: 0,
            autocomplete_visible: false,
            ghost_suffix: None,
            slash_buffer: None,
            slash_cursor: 0,
            slash_history: vec![],
            slash_hist_idx: None,
            history: vec![],
            hist_idx: None,
            scroll_up: 0,
            output_height: 20,

            tabs: TabSet::default(),
            tab_tx,
            tab_rx: Some(tab_rx),

            cached_instances: vec![],
            cached_modules: vec![],
            cached_server_detail: None,
            cached_module_detail: None,
            cached_update_status: None,
            cached_extensions: vec![],
            cached_manifest_extensions: vec![],
            cached_manifest_modules: vec![],
            cached_bot_config: None,
            cached_settings: None,

            ext_slots: ExtensionSlotRegistry::default(),

            loading: None,
            status_message: None,

            last_daemon_log_id: 0,
            last_daemon_log_fetch: None,

            quit: false,
            async_out: Arc::new(Mutex::new(Vec::new())),
            stop_daemon_on_exit: Arc::new(std::sync::atomic::AtomicBool::new(false)),
            session: None,
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
    /// ★ 복원되는 화면에 맞는 InputMode를 자동으로 설정한다.
    ///   CommandMode → Command, ServerConsole → Console, 그 외 → Normal
    pub fn pop_screen(&mut self) -> bool {
        if let Some(prev) = self.screen_stack.pop() {
            self.screen = prev;
            self.menu_selected = 0;
            self.input_mode = match &self.screen {
                Screen::CommandMode => InputMode::Command,
                Screen::ServerConsole { .. } => InputMode::Console,
                _ => InputMode::Normal,
            };
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
        self.servers = snap.servers;
        // 캐시는 데몬 응답이 있을 때만 갱신(오프라인 시 직전 값 유지).
        if snap.bot_config.is_some() {
            self.cached_bot_config = snap.bot_config;
        }
        if snap.settings_config.is_some() {
            self.cached_settings = snap.settings_config;
        }
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

    // ── 슬래시 히스토리 (Stage 5) ──

    /// 슬래시 명령 제출 시 히스토리에 기록. 공백·연속 중복 무시, 최대 길이 캡.
    pub fn push_slash_history(&mut self, cmd: String) {        const CAP: usize = 200;
        if cmd.trim().is_empty() { return; }
        if self.slash_history.last() == Some(&cmd) {
            self.slash_hist_idx = None;
            return;
        }
        self.slash_history.push(cmd);
        if self.slash_history.len() > CAP {
            let drop = self.slash_history.len() - CAP;
            self.slash_history.drain(..drop);
        }
        self.slash_hist_idx = None;
    }

    /// 슬래시 버퍼에서 ↑ — 이전 히스토리 항목으로 이동.
    pub fn slash_history_prev(&mut self) {
        if self.slash_history.is_empty() { return; }
        let idx = match self.slash_hist_idx {
            Some(i) => i.saturating_sub(1),
            None => self.slash_history.len() - 1,
        };
        self.slash_hist_idx = Some(idx);
        let v = self.slash_history[idx].clone();
        self.slash_cursor = v.chars().count();
        self.slash_buffer = Some(v);
    }

    /// 슬래시 버퍼에서 ↓ — 다음 히스토리 또는 빈 버퍼로 복귀.
    pub fn slash_history_next(&mut self) {
        if let Some(idx) = self.slash_hist_idx {
            if idx + 1 < self.slash_history.len() {
                self.slash_hist_idx = Some(idx + 1);
                let v = self.slash_history[idx + 1].clone();
                self.slash_cursor = v.chars().count();
                self.slash_buffer = Some(v);
            } else {
                self.slash_hist_idx = None;
                self.slash_buffer = Some(String::new());
                self.slash_cursor = 0;
            }
        }
    }

    // ── 최근 탭 복원 (Stage 6) ──

    /// 세션에 저장된 `recent_tabs`로부터 탭을 재개설한다.
    ///
    /// - 현재 열린 탭이 없을 때만 동작(사용자 수동 조작을 덮지 않기 위함).
    /// - `valid_server_ids`가 `Some`이면 목록에 없는 서버 탭은 건너뛴다.
    ///   `None`이면(= 데몬 오프라인 등 목록 조회 불가) 서버 탭도 그대로 복원한다.
    /// - 복원 직후에는 `active = None` 으로 맞춰 포커스는 대시보드에 둔다.
    /// - 반환값: 실제로 (재)개설된 탭 수.
    pub fn restore_recent_tabs(&mut self, valid_server_ids: Option<&std::collections::HashSet<String>>) -> usize {
        if !self.tabs.tabs.is_empty() {
            return 0;
        }
        let recent = match self.session.as_ref() {
            Some(s) if !s.recent_tabs.is_empty() => s.recent_tabs.clone(),
            _ => return 0,
        };
        let count = super::tabs::restore_tabs_into(
            &mut self.tabs,
            &self.client,
            &self.tab_tx,
            &recent,
            valid_server_ids,
        );        // 자동 복원된 탭을 활성 상태로 두면 `/tab list` 등에서 혼란을 주므로 해제.
        self.tabs.active = None;
        count
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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::client::DaemonClient;

    fn fresh_app() -> App {
        App::new(DaemonClient::new(None).unwrap())
    }

    #[test]
    fn push_slash_history_dedups_consecutive() {
        let mut app = fresh_app();
        app.push_slash_history("theme dark".into());
        app.push_slash_history("theme dark".into());
        app.push_slash_history("help".into());
        assert_eq!(
            app.slash_history,
            vec!["theme dark".to_string(), "help".to_string()]
        );
    }

    #[test]
    fn push_slash_history_ignores_empty() {
        let mut app = fresh_app();
        app.push_slash_history("".into());
        app.push_slash_history("   ".into());
        assert!(app.slash_history.is_empty());
    }

    #[test]
    fn push_slash_history_caps_length() {
        let mut app = fresh_app();
        for i in 0..250 {
            app.push_slash_history(format!("cmd{}", i));
        }
        assert_eq!(app.slash_history.len(), 200);
        assert_eq!(app.slash_history.first().unwrap(), "cmd50");
    }

    #[test]
    fn slash_history_prev_next_cycles() {
        let mut app = fresh_app();
        app.push_slash_history("a".into());
        app.push_slash_history("b".into());
        app.push_slash_history("c".into());

        // 최신(c)부터 과거로
        app.slash_history_prev();
        assert_eq!(app.slash_buffer.as_deref(), Some("c"));
        app.slash_history_prev();
        assert_eq!(app.slash_buffer.as_deref(), Some("b"));
        app.slash_history_prev();
        assert_eq!(app.slash_buffer.as_deref(), Some("a"));
        // 경계: 더 이상 이전 없음 — 여전히 a
        app.slash_history_prev();
        assert_eq!(app.slash_buffer.as_deref(), Some("a"));

        // 다시 앞으로
        app.slash_history_next();
        assert_eq!(app.slash_buffer.as_deref(), Some("b"));
        app.slash_history_next();
        assert_eq!(app.slash_buffer.as_deref(), Some("c"));
        // 경계: 빈 버퍼로 복귀
        app.slash_history_next();
        assert_eq!(app.slash_buffer.as_deref(), Some(""));
        assert!(app.slash_hist_idx.is_none());
    }

    #[test]
    fn slash_history_prev_on_empty_is_noop() {
        let mut app = fresh_app();
        app.slash_history_prev();
        assert!(app.slash_buffer.is_none());
        assert!(app.slash_hist_idx.is_none());
    }

    // ── Stage 6: 최근 탭 복원 ──
    //
    // `App` 생성은 `reqwest::blocking::Client`를 내부적으로 drop할 수 있어
    // tokio 런타임 컨텍스트 내에서 drop하면 패닉한다. 따라서 복원 로직의
    // 상세 단위 테스트는 `tabs::restore_tabs_into`에서 커버하고, 여기서는
    // `App` 래퍼의 가드(빈 탭/빈 세션) 만 검증한다.

    #[test]
    fn restore_recent_tabs_empty_session_returns_zero() {
        let mut app = fresh_app();
        app.session = Some(super::super::session::Session::default());
        let n = app.restore_recent_tabs(None);
        assert_eq!(n, 0);
        assert!(app.tabs.tabs.is_empty());
    }

    #[test]
    fn restore_recent_tabs_no_session_returns_zero() {
        let mut app = fresh_app();
        app.session = None;
        let n = app.restore_recent_tabs(None);
        assert_eq!(n, 0);
    }
}
