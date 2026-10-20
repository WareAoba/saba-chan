//! 중앙 설정 저장소 (ConfigStore)
//!
//! 설정 파일은 `<data_dir>/config.json` 하나다(`saba_chan_updater_lib::config_file`).
//! 이 저장소는 그중 타입이 정해진 네임스페이스를 캐시하고 검증한다.
//! - `gui` (GUI 설정)
//! - `relay` (릴레이·디스코드 연동 설정)
//! - 그 밖의 클라이언트 네임스페이스(`cli` 등)는 JSON 그대로 (`/api/config/client/:kind`)
//!
//! 설계 원칙:
//! - `serde(default)` + 강타입으로 필드 누락/오타를 컴파일 타임에 방지
//! - 인메모리 캐시 + 변경 시 flush
//! - 기존 JSON 파일과 하위호환 유지 (`#[serde(rename_all = "camelCase")]`)

use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::collections::HashMap;
use saba_chan_updater_lib::config_file::ConfigFile;
use std::path::Path;
use tokio::sync::RwLock;

// ── GUI Settings (settings.json) ──

/// GUI 설정 — settings.json에 저장되는 모든 필드
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase", default)]
pub struct GuiSettings {
    pub auto_refresh: bool,
    pub refresh_interval: u64,
    pub ipc_port: u16,
    pub console_buffer_size: u32,
    pub auto_generate_passwords: bool,
    pub port_conflict_check: bool,

    // Theme customization
    /// "dark" | "light" | "auto". 없으면 GUI가 정한다(예전 브라우저 저장값 이전, 기본 auto)
    #[serde(skip_serializing_if = "Option::is_none")]
    pub theme: Option<String>,
    pub accent_color: String,
    pub accent_secondary: String,
    pub use_gradient: bool,
    pub font_scale: u32,
    pub enable_transitions: bool,
    pub console_syntax_highlight: bool,
    pub console_bg_color: String,
    pub console_text_color: String,
    pub sidebar_compact: bool,
    pub window_opacity: u32,
    /// 콘솔 글꼴 배율 (%)
    pub console_font_scale: u32,
    /// 서버 페이지의 시스템 모니터 펼침 여부
    pub system_monitor_open: bool,

    // Window state (GUI가 저장하지만 데몬이 중개)
    #[serde(skip_serializing_if = "Option::is_none")]
    pub window_bounds: Option<WindowBounds>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub language: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase", default)]
pub struct WindowBounds {
    pub width: u32,
    pub height: u32,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub x: Option<i32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub y: Option<i32>,
}

impl Default for WindowBounds {
    fn default() -> Self {
        Self {
            width: 1200,
            height: 840,
            x: None,
            y: None,
        }
    }
}

impl Default for GuiSettings {
    fn default() -> Self {
        Self {
            auto_refresh: true,
            refresh_interval: 2000,
            ipc_port: saba_chan_updater_lib::constants::DEFAULT_IPC_PORT,
            console_buffer_size: 2000,
            auto_generate_passwords: true,
            port_conflict_check: true,
            theme: None,
            accent_color: "#667eea".to_string(),
            accent_secondary: "#764ba2".to_string(),
            use_gradient: true,
            font_scale: 100,
            enable_transitions: true,
            console_syntax_highlight: true,
            console_bg_color: "#1e1e2e".to_string(),
            console_text_color: "#cdd6f4".to_string(),
            sidebar_compact: false,
            window_opacity: 100,
            console_font_scale: 100,
            system_monitor_open: true,
            window_bounds: None,
            language: None,
        }
    }
}

// ── Bot Config (bot-config.json) ──

/// Discord 봇 설정 — bot-config.json에 저장되는 모든 필드
///
/// 디스코드 봇은 클라우드 릴레이에서만 돈다. 로컬 봇용 필드(token, autoStart,
/// mode, music*)는 제거됐고, 예전 파일에 남아 있어도 읽을 때 무시된다.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase", default)]
pub struct BotConfig {
    /// 채팅 접두사(호출어). 비어 있으면 릴레이가 응답 언어의 기본값을 쓴다 (ko `사바쨩`)
    pub prefixes: Vec<String>,
    /// 예전 `prefix` 하나. 읽을 때만 받고 `normalize`가 `prefixes`로 옮긴다
    #[serde(rename = "prefix", skip_serializing)]
    pub(crate) legacy_prefix: Option<String>,
    pub cloud: CloudConfig,
    /// 예전 전역 별명 (모듈 → "a, b", 모듈 → 명령 → "a, b", 인스턴스 → "a, b").
    /// 길드가 있으면 시작할 때 `nodeSettings[guild].instances`로 옮기고 비운다 (`chat::words::migrate`)
    pub module_aliases: HashMap<String, String>,
    pub command_aliases: HashMap<String, Value>,
    pub instance_aliases: HashMap<String, String>,
    pub node_settings: Value,
    pub cloud_nodes: Vec<Value>,
    pub cloud_members: HashMap<String, Value>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase", default)]
#[derive(Default)]
pub struct CloudConfig {
    pub relay_url: String,
    pub host_id: String,
}


impl Default for BotConfig {
    fn default() -> Self {
        Self {
            prefixes: Vec::new(),
            legacy_prefix: None,
            cloud: CloudConfig::default(),
            module_aliases: HashMap::new(),
            command_aliases: HashMap::new(),
            instance_aliases: HashMap::new(),
            node_settings: Value::Object(serde_json::Map::new()),
            cloud_nodes: Vec::new(),
            cloud_members: HashMap::new(),
        }
    }
}

/// 접두사 하나의 최대 글자 수와 최대 개수 (릴레이 계약과 같다)
pub const PREFIX_MAX_CHARS: usize = 32;
pub const PREFIXES_MAX: usize = 10;

impl BotConfig {
    /// 예전 `prefix`를 `prefixes`로 옮긴다. 예전 기본값 `!saba`와 빈 값은 기본값(빈 목록)이다
    fn normalize(&mut self) {
        if let Some(old) = self.legacy_prefix.take() {
            let old = old.trim();
            if self.prefixes.is_empty() && !old.is_empty() && old != "!saba" {
                self.prefixes = vec![old.to_string()];
            }
        }
        self.prefixes = self.prefixes.iter().map(|p| p.trim().to_string()).collect();
    }

    /// 접두사는 공백 없는 1~32자, 10개까지
    fn prefix_problem(&self) -> Option<String> {
        if self.prefixes.len() > PREFIXES_MAX {
            return Some(format!("prefixes: at most {} entries", PREFIXES_MAX));
        }
        self.prefixes
            .iter()
            .find(|p| p.is_empty() || p.chars().count() > PREFIX_MAX_CHARS || p.chars().any(char::is_whitespace))
            .map(|p| format!("prefixes: '{}' must be 1-{} characters without spaces", p, PREFIX_MAX_CHARS))
    }
}

/// 릴레이 주소 규칙: 빈 값(기본 릴레이), https, 루프백 http(모의 릴레이)만. 노드 토큰·페어링이 평문으로 오가지 않게.
/// 저장·페어링 때만 본다 (이미 저장된 값은 읽을 때 막지 않는다)
pub fn relay_url_problem(url: &str) -> Option<String> {
    let url = url.trim();
    if url.is_empty() {
        return None;
    }
    let ok = match reqwest::Url::parse(url) {
        Ok(u) => match (u.scheme(), u.host_str()) {
            ("https", Some(h)) => !h.is_empty(),
            ("http", Some(h)) => matches!(h, "localhost" | "127.0.0.1" | "[::1]"),
            _ => false,
        },
        Err(_) => false,
    };
    (!ok).then(|| format!("'{}' must be an https URL (http only for localhost)", url))
}

// ── Chatbot (config.json `chatbot`) ──

/// AI 챗봇 설정의 원본. 릴레이는 이것을 받아 쓴다 (`/api/nodes/:guildId/chatbot`).
/// 설정 값의 뜻과 검증은 릴레이 몫이라 데몬은 객체 그대로 둔다 (필드가 늘어도 데몬은 그대로)
#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase", default)]
pub struct ChatbotConfig {
    /// 모델 API 키. 클라이언트에는 `hasApiKey`만 보인다
    #[serde(skip_serializing_if = "Option::is_none")]
    pub api_key: Option<String>,
    /// 모든 길드에 쓰는 설정 (ChatbotSettings 일부)
    pub settings: serde_json::Map<String, Value>,
    /// 길드별로 덮어쓰는 설정 (예: memo). 채널 배정은 릴레이가 원본이라 두지 않는다
    pub guilds: HashMap<String, serde_json::Map<String, Value>>,
    /// 사용자가 지웠는데 릴레이 삭제를 아직 확인 못 함. 다음 연결 때 다시 지운다
    #[serde(skip_serializing_if = "std::ops::Not::not")]
    pub pending_relay_delete: bool,
}

impl ChatbotConfig {
    /// 예전 키를 지금 계약으로 (2026-10-17: `persona` → `memo`; 챗봇 채널 `channelIds`는 릴레이가 원본이 되어 지운다)
    fn migrate(&mut self) {
        for map in std::iter::once(&mut self.settings).chain(self.guilds.values_mut()) {
            if let Some(persona) = map.remove("persona") {
                map.entry("memo").or_insert(persona);
            }
            map.remove("channelIds");
        }
        self.guilds.retain(|_, over| !over.is_empty());
    }

    /// 한 길드에 보낼 설정 (공통 위에 길드 것)
    pub fn effective(&self, guild_id: &str) -> serde_json::Map<String, Value> {
        let mut out = self.settings.clone();
        if let Some(over) = self.guilds.get(guild_id) {
            out.extend(over.clone());
        }
        out
    }
}

// ── ConfigStore ──

/// 중앙 설정 저장소 — config.json의 타입 있는 네임스페이스 캐시
pub struct ConfigStore {
    file: ConfigFile,
    gui_settings: RwLock<GuiSettings>,
    bot_config: RwLock<BotConfig>,
}

/// 이 PC에만 맞는 네임스페이스. 내보내기에서 뺀다 (`sandbox`: 이 PC 사용자가 확인한 동의)
const LOCAL_SECTIONS: &[&str] = &["schemaVersion", "updates", "boot", "extensions", "sandbox"];

/// 클라이언트 네임스페이스로 쓸 수 없는 키 (각자 전용 API가 있다)
const RESERVED_SECTIONS: &[&str] = &["schemaVersion", "gui", "relay", "updates", "boot", "extensions", "chatbot", "sandbox"];

/// `nodeSettings[guild]`에서 전용 API만 바꾸는 키: 권한 API(P1), 디스코드 설정 API(I2, `discord`),
/// 길드·인스턴스 낱말 API(`instances`), 길드가 쓸 서버 API(`allowedInstances`)
pub const PERMISSION_KEYS: &[&str] = &[
    "instances",
    "allowedInstances",
    "ownerDiscordId",
    "relayOwnerDiscordId",
    "defaultLevel",
    "members",
    "preset",
    "commandLevels",
    "chatbotCap",
    "memberPermissions",
    "discord",
];

/// `merged.nodeSettings`의 권한 키를 `current` 값으로 되돌린다 (없던 키는 지운다).
/// 다른 값을 실어 왔다가 되돌린 키를 `nodeSettings.<길드>.<키>`로 돌려준다 (이름 순)
fn keep_permission_keys(merged: &mut Value, current: &Value) -> Vec<String> {
    let mut reverted = Vec::new();
    let Some(merged) = merged.as_object_mut() else { return reverted };
    let nodes = merged.entry("nodeSettings").or_insert_with(|| Value::Object(serde_json::Map::new()));
    if !nodes.is_object() {
        *nodes = Value::Object(serde_json::Map::new());
    }
    let nodes = nodes.as_object_mut().expect("object");
    // 새 값에서 빠진 길드도 권한은 남긴다
    for guild in current.get("nodeSettings").and_then(Value::as_object).into_iter().flatten().map(|(g, _)| g) {
        nodes.entry(guild.clone()).or_insert_with(|| Value::Object(serde_json::Map::new()));
    }
    for (guild, node) in nodes.iter_mut() {
        let Some(node) = node.as_object_mut() else { continue };
        let before = current.get("nodeSettings").and_then(|n| n.get(guild));
        for key in PERMISSION_KEYS {
            // 빠진 키는 알리지 않는다: 얕은 머지라 다른 키만 고친 요청도 nodeSettings 전체를 싣고, 권한 키를 빼고 보낼 수 있다
            if node.get(*key).is_some_and(|v| Some(v) != before.and_then(|b| b.get(*key))) {
                reverted.push(format!("nodeSettings.{}.{}", guild, key));
            }
            match before.and_then(|b| b.get(*key)) {
                Some(v) => {
                    node.insert(key.to_string(), v.clone());
                }
                None => {
                    node.remove(*key);
                }
            }
        }
    }
    reverted.sort();
    reverted
}

impl ConfigStore {
    /// 새 ConfigStore 생성 (디스크에서 기존 설정 로드, 예전 파일은 이관)
    pub fn new(data_dir: &Path) -> Self {
        let file = ConfigFile::at(data_dir);
        let doc = file.load();
        let gui = section_or_default::<GuiSettings>(&doc, "gui");
        let mut bot = section_or_default::<BotConfig>(&doc, "relay");
        bot.normalize();
        // 손으로 고친 파일의 틀린 접두사는 버리고 기본값으로 둔다
        if let Some(problem) = bot.prefix_problem() {
            tracing::warn!("[Config] {}, using the default prefix", problem);
            bot.prefixes.clear();
        }

        Self {
            file,
            gui_settings: RwLock::new(gui),
            bot_config: RwLock::new(bot),
        }
    }

    // ── Chatbot ──

    pub fn get_chatbot(&self) -> ChatbotConfig {
        let mut cfg: ChatbotConfig = serde_json::from_value(self.file.section("chatbot")).unwrap_or_default();
        cfg.migrate();
        cfg
    }

    pub fn set_chatbot(&self, config: &ChatbotConfig) -> Result<(), ConfigStoreError> {
        self.file.set_section("chatbot", serde_json::to_value(config)?)?;
        Ok(())
    }

    // ── GUI Settings ──

    /// GUI 설정 읽기 (인메모리 캐시에서 반환)
    pub async fn get_gui_settings(&self) -> GuiSettings {
        self.gui_settings.read().await.clone()
    }

    /// 데몬이 쓰는 화면 언어: 요청이 고른 언어(Accept-Language), 설정한 언어, 없으면 OS 언어 (GUI와 같은 규칙, 지원하지 않으면 en)
    pub async fn ui_language(&self) -> String {
        match crate::audit::current().lang {
            Some(lang) => lang,
            None => self.pc_language().await,
        }
    }

    /// 이 PC의 언어: 설정한 언어, 없으면 OS 언어. 요청 언어와 상관없이 릴레이에 알리는 값 (하트비트·디스코드 설정의 `language`)
    pub async fn pc_language(&self) -> String {
        match self.gui_settings.read().await.language.as_deref() {
            Some(l) if !l.trim().is_empty() => l.to_string(),
            _ => crate::chat::i18n::os_language(),
        }
    }

    /// GUI 설정을 JSON Value로 반환 (API 응답용)
    pub async fn get_gui_settings_json(&self) -> Value {
        let settings = self.gui_settings.read().await;
        serde_json::to_value(&*settings).unwrap_or_default()
    }

    /// JSON Value로 GUI 설정 교체 (API 요청 수용, 스키마 검증 포함)
    pub async fn set_gui_settings_from_json(&self, json: Value) -> Result<GuiSettings, ConfigStoreError> {
        // 기존 설정을 base로 하여 부분 업데이트 지원. 읽기·병합·저장 동안 쓰기 잠금을 잡아 동시 저장이 서로 덮지 않게
        let mut current = self.gui_settings.write().await;
        let current_json = serde_json::to_value(&*current).unwrap_or_default();

        // 기존 JSON에 새 JSON을 머지
        let merged = merge_json(current_json, json);

        let settings: GuiSettings = serde_json::from_value(merged)
            .map_err(|e| ConfigStoreError::InvalidData(format!("Invalid GUI settings: {}", e)))?;
        self.file.set_section("gui", serde_json::to_value(&settings)?)?;
        *current = settings.clone();
        Ok(settings)
    }

    // ── Bot Config ──

    /// 봇 설정 읽기 (인메모리 캐시에서 반환)
    pub async fn get_bot_config(&self) -> BotConfig {
        self.bot_config.read().await.clone()
    }

    /// 봇 설정을 JSON Value로 반환 (API 응답용)
    pub async fn get_bot_config_json(&self) -> Value {
        let config = self.bot_config.read().await;
        serde_json::to_value(&*config).unwrap_or_default()
    }

    /// JSON Value로 봇 설정 교체 (API 요청 수용, 스키마 검증 포함)
    pub async fn set_bot_config_from_json(&self, json: Value) -> Result<BotConfig, ConfigStoreError> {
        Ok(self.merge_bot_config(json, true).await?.0)
    }

    /// `set_bot_config_from_json`과 같고, 권한 키라 바꾸지 않고 둔 경로(`nodeSettings.<길드>.<키>`)도 돌려준다 (L61)
    pub async fn save_bot_config_json(&self, json: Value) -> Result<(BotConfig, Vec<String>), ConfigStoreError> {
        self.merge_bot_config(json, true).await
    }

    /// 부분 업데이트. 읽기·병합·저장 동안 쓰기 잠금을 잡아 그사이의 권한 변경(주인 고정 등)을 덮지 않는다.
    /// `keep_permissions`면 권한 키는 권한 API(`update_node_settings`)로만 바뀐다 (가져오기는 로컬 동작이라 false)
    async fn merge_bot_config(&self, json: Value, keep_permissions: bool) -> Result<(BotConfig, Vec<String>), ConfigStoreError> {
        let mut current = self.bot_config.write().await;
        let current_json = serde_json::to_value(&*current).unwrap_or_default();

        let mut merged = merge_json(current_json.clone(), json);
        let reverted = if keep_permissions { keep_permission_keys(&mut merged, &current_json) } else { Vec::new() };

        let mut config: BotConfig = serde_json::from_value(merged)
            .map_err(|e| ConfigStoreError::InvalidData(format!("Invalid bot config: {}", e)))?;
        config.normalize();
        if let Some(problem) = config.prefix_problem() {
            return Err(ConfigStoreError::InvalidData(problem));
        }
        if config.cloud.relay_url.trim() != current.cloud.relay_url.trim() {
            if let Some(problem) = relay_url_problem(&config.cloud.relay_url) {
                return Err(ConfigStoreError::InvalidRelayUrl(problem));
            }
        }
        self.file.set_section("relay", serde_json::to_value(&config)?)?;
        *current = config.clone();
        Ok((config, reverted))
    }


    /// `nodeSettings[guild]` 객체를 고친다 (권한 API·릴레이 주인 고정). `f`가 true를 돌려주면 저장한다
    pub async fn update_node_settings<R>(
        &self,
        guild_id: &str,
        f: impl FnOnce(&mut serde_json::Map<String, Value>) -> (R, bool),
    ) -> Result<R, ConfigStoreError> {
        let mut config = self.bot_config.write().await;
        let mut next = config.clone();
        if !next.node_settings.is_object() {
            next.node_settings = Value::Object(serde_json::Map::new());
        }
        let nodes = next.node_settings.as_object_mut().expect("object");
        let node = nodes.entry(guild_id.to_string()).or_insert_with(|| Value::Object(serde_json::Map::new()));
        if !node.is_object() {
            *node = Value::Object(serde_json::Map::new());
        }
        let (out, changed) = f(node.as_object_mut().expect("object"));
        if changed {
            self.file.set_section("relay", serde_json::to_value(&next)?)?;
            *config = next;
        }
        Ok(out)
    }

    /// 예전 전역 별명을 길드·인스턴스 낱말로 옮긴 결과를 저장한다: `nodeSettings`를 바꾸고 옛 세 키를 비운다
    /// `migrate`는 쓰기 잠금 안에서 지금 설정으로 옮긴 `nodeSettings`를 만든다: 그사이 바뀐 길드 권한을 덮지 않게 (D-F5).
    /// 옮길 것이 없으면(None) 아무것도 바꾸지 않는다
    pub async fn finish_words_migration<T>(&self, migrate: impl FnOnce(&BotConfig) -> Option<(Value, T)>) -> Result<Option<T>, ConfigStoreError> {
        let mut config = self.bot_config.write().await;
        let Some((node_settings, out)) = migrate(&config) else { return Ok(None) };
        let mut next = config.clone();
        next.node_settings = node_settings;
        next.module_aliases.clear();
        next.command_aliases.clear();
        next.instance_aliases.clear();
        self.file.set_section("relay", serde_json::to_value(&next)?)?;
        *config = next;
        Ok(Some(out))
    }

    // ── 기본값 ──

    /// `gui` | `bot` 설정의 기본값 (JSON)
    pub fn defaults_json(kind: &str) -> Option<Value> {
        match kind {
            "gui" => serde_json::to_value(GuiSettings::default()).ok(),
            "bot" => serde_json::to_value(BotConfig::default()).ok(),
            _ => None,
        }
    }

    /// 기본값이 "없음"이라 기본값 JSON에 나오지 않는 키 (초기화하면 지운다)
    fn optional_keys(kind: &str) -> &'static [&'static str] {
        match kind {
            "gui" => &["windowBounds", "language", "theme"],
            _ => &[],
        }
    }

    /// `keys`(None이면 전부)를 기본값으로 되돌리고 바뀐 설정 전체를 돌려준다.
    /// 키는 최상위 이름이다 (예: `language`, `portConflictCheck`). 빈 목록·모르는 키는 오류.
    pub async fn reset_to_defaults(&self, kind: &str, keys: Option<&[String]>) -> Result<Value, ConfigStoreError> {
        let defaults = Self::defaults_json(kind)
            .and_then(|d| d.as_object().cloned())
            .ok_or_else(|| ConfigStoreError::InvalidData(format!("Unknown config: {}", kind)))?;
        // 읽기부터 저장까지 쓰기 잠금을 잡는다: 그사이 저장된 다른 키를 되돌리지 않게 (D-F4)
        let mut gui = if kind == "gui" { Some(self.gui_settings.write().await) } else { None };
        let mut bot = if kind == "gui" { None } else { Some(self.bot_config.write().await) };
        let full = match keys {
            None => defaults,
            Some([]) => {
                return Err(ConfigStoreError::InvalidData("keys is empty; omit keys to reset everything".into()))
            }
            Some(keys) => {
                let current = match (&gui, &bot) {
                    (Some(g), _) => serde_json::to_value(&**g).unwrap_or_default(),
                    (None, Some(b)) => serde_json::to_value(&**b).unwrap_or_default(),
                    (None, None) => unreachable!("one of the two is held"),
                };
                let mut current = current.as_object().cloned().unwrap_or_default();
                for key in keys {
                    if let Some(value) = defaults.get(key) {
                        current.insert(key.clone(), value.clone());
                    } else if Self::optional_keys(kind).contains(&key.as_str()) {
                        current.remove(key);
                    } else {
                        return Err(ConfigStoreError::InvalidData(format!("Unknown {} setting: {}", kind, key)));
                    }
                }
                current
            }
        };
        // 머지하지 않고 통째로 바꾼다 (지운 키가 되살아나지 않게)
        if let Some(gui) = gui.as_mut() {
            let settings: GuiSettings = serde_json::from_value(Value::Object(full))
                .map_err(|e| ConfigStoreError::InvalidData(format!("Invalid GUI settings: {}", e)))?;
            self.file.set_section("gui", serde_json::to_value(&settings)?)?;
            **gui = settings;
            Ok(serde_json::to_value(&**gui).unwrap_or_default())
        } else {
            let mut current = bot.take().expect("held above");
            // 초기화도 고정된 주인·부여값은 지우지 않는다 (권한 API로만 바뀐다)
            let mut full = Value::Object(full);
            keep_permission_keys(&mut full, &serde_json::to_value(&*current).unwrap_or_default());
            let mut config: BotConfig = serde_json::from_value(full)
                .map_err(|e| ConfigStoreError::InvalidData(format!("Invalid bot config: {}", e)))?;
            // hostId는 설정이 아니라 노드 토큰을 따르는 값이다: 토큰이 남아 있는 동안 비우면 릴레이 정리가 빈 목록을 본다
            config.cloud.host_id = current.cloud.host_id.clone();
            config.normalize();
            self.file.set_section("relay", serde_json::to_value(&config)?)?;
            *current = config;
            drop(current);
            Ok(self.get_bot_config_json().await)
        }
    }

    // ── 내보내기·가져오기 (A9) ──

    /// 다른 PC로 옮길 설정 네임스페이스. 이 PC에만 맞는 것(업데이트 채널·부트·익스텐션 켜짐)은 뺀다.
    /// `secrets`가 false면 챗봇 API 키를 뺀다.
    pub fn portable_sections(&self, secrets: bool) -> serde_json::Map<String, Value> {
        let mut doc = self.file.load();
        doc.retain(|k, _| !LOCAL_SECTIONS.contains(&k.as_str()));
        if !secrets {
            if let Some(Value::Object(chatbot)) = doc.get_mut("chatbot") {
                chatbot.remove("apiKey");
                chatbot.remove("pendingRelayDelete");
            }
        }
        doc
    }

    /// `import_sections`가 받을 수 있는지만 본다 (아무것도 쓰지 않는다). 가져오기가 다른 것을 바꾸기 전에 부른다
    pub async fn check_sections(&self, sections: &serde_json::Map<String, Value>) -> Result<(), ConfigStoreError> {
        for (key, value) in sections.iter().filter(|(_, v)| v.is_object()) {
            let invalid = |what: &str, e: serde_json::Error| ConfigStoreError::InvalidData(format!("Invalid {}: {}", what, e));
            match key.as_str() {
                "gui" => {
                    let merged = merge_json(self.get_gui_settings_json().await, value.clone());
                    serde_json::from_value::<GuiSettings>(merged).map_err(|e| invalid("GUI settings", e))?;
                }
                "relay" => {
                    // 저장(`merge_bot_config`)과 같은 검사: 접두사·릴레이 주소 규칙까지
                    let merged = merge_json(self.get_bot_config_json().await, value.clone());
                    let mut config = serde_json::from_value::<BotConfig>(merged).map_err(|e| invalid("bot config", e))?;
                    config.normalize();
                    if let Some(problem) = config.prefix_problem() {
                        return Err(ConfigStoreError::InvalidData(problem));
                    }
                    if config.cloud.relay_url.trim() != self.get_bot_config().await.cloud.relay_url.trim() {
                        if let Some(problem) = relay_url_problem(&config.cloud.relay_url) {
                            return Err(ConfigStoreError::InvalidRelayUrl(problem));
                        }
                    }
                }
                "chatbot" => {
                    let merged = merge_json(serde_json::to_value(self.get_chatbot())?, value.clone());
                    serde_json::from_value::<ChatbotConfig>(merged).map_err(|e| invalid("chatbot config", e))?;
                }
                _ => {}
            }
        }
        Ok(())
    }

    /// 내보낸 설정을 지금 설정 위에 머지한다(없는 키는 그대로 둔다). 반영한 네임스페이스 이름을 돌려준다.
    pub async fn import_sections(&self, sections: serde_json::Map<String, Value>) -> Result<Vec<String>, ConfigStoreError> {
        let mut applied = Vec::new();
        for (key, value) in sections {
            if !value.is_object() {
                continue;
            }
            match key.as_str() {
                "gui" => {
                    self.set_gui_settings_from_json(value).await?;
                }
                "relay" => {
                    // 내보낸 파일을 이 PC에서 가져오는 로컬 동작이라 권한 키도 함께 가져온다
                    self.merge_bot_config(value, false).await?;
                }
                "chatbot" => {
                    let merged = merge_json(serde_json::to_value(self.get_chatbot())?, value);
                    let cfg: ChatbotConfig = serde_json::from_value(merged)
                        .map_err(|e| ConfigStoreError::InvalidData(format!("Invalid chatbot config: {}", e)))?;
                    self.set_chatbot(&cfg)?;
                }
                k if Self::is_client_section(k) => {
                    self.merge_client_section(k, value)?;
                }
                _ => continue,
            }
            applied.push(key);
        }
        Ok(applied)
    }

    // ── Client namespaces ──

    /// 클라이언트 네임스페이스 이름 검사: 소문자로 시작, 소문자·숫자·`-`, 32자 이하, 예약어 제외
    pub fn is_client_section(kind: &str) -> bool {
        let mut chars = kind.chars();
        matches!(chars.next(), Some(c) if c.is_ascii_lowercase())
            && kind.len() <= 32
            && chars.all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '-')
            && !RESERVED_SECTIONS.contains(&kind)
    }

    /// 클라이언트 네임스페이스 읽기 (없으면 빈 객체)
    pub fn get_client_section(&self, kind: &str) -> Result<Value, ConfigStoreError> {
        if !Self::is_client_section(kind) {
            return Err(ConfigStoreError::InvalidData(format!("Invalid config section: {}", kind)));
        }
        Ok(match self.file.section(kind) {
            Value::Null => Value::Object(serde_json::Map::new()),
            v => v,
        })
    }

    /// 클라이언트 네임스페이스에 부분 업데이트(최상위 키 머지). 결과를 돌려준다.
    pub fn merge_client_section(&self, kind: &str, patch: Value) -> Result<Value, ConfigStoreError> {
        if !Self::is_client_section(kind) {
            return Err(ConfigStoreError::InvalidData(format!("Invalid config section: {}", kind)));
        }
        if !patch.is_object() {
            return Err(ConfigStoreError::InvalidData("Config section must be a JSON object".into()));
        }
        let mut merged = Value::Null;
        self.file.update(|doc| {
            let current = doc.remove(kind).filter(Value::is_object).unwrap_or_else(|| Value::Object(Default::default()));
            merged = merge_json(current, patch);
            doc.insert(kind.to_string(), merged.clone());
        })?;
        Ok(merged)
    }
}

// ── Error ──

#[derive(Debug, thiserror::Error)]
pub enum ConfigStoreError {
    #[error("I/O error: {0}")]
    Io(#[from] std::io::Error),
    #[error("Serialization error: {0}")]
    Serialization(#[from] serde_json::Error),
    #[error("Invalid data: {0}")]
    InvalidData(String),
    #[error("Invalid relay URL: {0}")]
    InvalidRelayUrl(String),
}

// ── File helpers ──

/// 문서의 네임스페이스를 타입 T로. 없거나 형식이 틀리면 Default (틀린 경우 경고).
fn section_or_default<T: for<'de> Deserialize<'de> + Default>(
    doc: &serde_json::Map<String, Value>,
    key: &str,
) -> T {
    match doc.get(key) {
        None | Some(Value::Null) => T::default(),
        Some(v) => serde_json::from_value(v.clone()).unwrap_or_else(|e| {
            tracing::warn!("[Config] Section '{}' is invalid, using defaults: {}", key, e);
            T::default()
        }),
    }
}

/// 두 JSON Value를 머지 (base에 overlay를 덮어씌움)
fn merge_json(base: Value, overlay: Value) -> Value {
    match (base, overlay) {
        (Value::Object(mut base_map), Value::Object(overlay_map)) => {
            for (key, val) in overlay_map {
                base_map.insert(key, val);
            }
            Value::Object(base_map)
        }
        (_, overlay) => overlay,
    }
}

// ── Tests ──

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::TempDir;

    // ── GuiSettings 직렬화/역직렬화 ──

    #[tokio::test]
    async fn portable_sections_leave_out_local_ones_and_import_keeps_the_local_api_key() {
        let dir = tempfile::tempdir().unwrap();
        let store = ConfigStore::new(dir.path());
        store.file.set_section("updates", serde_json::json!({"channel": "beta"})).unwrap();
        store.file.set_section("chatbot", serde_json::json!({"apiKey": "k", "settings": {"model": "m"}})).unwrap();
        store.merge_client_section("cli", serde_json::json!({"color": false})).unwrap();

        let out = store.portable_sections(false);
        assert!(!out.contains_key("updates"));
        assert!(out["chatbot"].get("apiKey").is_none());
        assert_eq!(out["cli"], serde_json::json!({"color": false}));
        assert!(store.portable_sections(true)["chatbot"].get("apiKey").is_some());

        let mut incoming = serde_json::Map::new();
        incoming.insert("chatbot".into(), serde_json::json!({"settings": {"model": "n"}}));
        incoming.insert("gui".into(), serde_json::json!({"language": "ja"}));
        incoming.insert("updates".into(), serde_json::json!({"channel": "stable"}));
        let applied = store.import_sections(incoming).await.unwrap();
        assert_eq!(applied, ["chatbot", "gui"]);
        let chatbot = store.get_chatbot();
        assert_eq!(chatbot.api_key.as_deref(), Some("k"));
        assert_eq!(chatbot.settings["model"], "n");
        assert_eq!(store.get_gui_settings().await.language.as_deref(), Some("ja"));
        assert_eq!(store.file.section("updates")["channel"], "beta");

        // 잘못된 설정은 쓰기 전에 걸러진다
        let mut bad = serde_json::Map::new();
        bad.insert("gui".into(), serde_json::json!({"fontScale": "big"}));
        assert!(store.check_sections(&bad).await.is_err());
        assert_eq!(store.get_gui_settings().await.font_scale, 100);
    }

    #[tokio::test]
    async fn ui_language_follows_the_os_until_one_is_chosen() {
        let dir = tempfile::tempdir().unwrap();
        let store = ConfigStore::new(dir.path());
        let os = crate::chat::i18n::os_language();
        assert_eq!(store.ui_language().await, os);
        store.set_gui_settings_from_json(serde_json::json!({"language": "ja"})).await.unwrap();
        assert_eq!(store.ui_language().await, "ja");
    }

    /// 요청이 Accept-Language로 언어를 고르면 그 요청의 데몬 문구는 그 언어를 따른다 (CLI `--lang`)
    #[tokio::test]
    async fn a_request_language_wins_over_the_gui_language() {
        let dir = tempfile::tempdir().unwrap();
        let store = ConfigStore::new(dir.path());
        store.set_gui_settings_from_json(serde_json::json!({"language": "ja"})).await.unwrap();
        let mut headers = axum::http::HeaderMap::new();
        headers.insert("Accept-Language", "ko-KR,ko;q=0.9,en;q=0.8".parse().unwrap());
        let origin = crate::audit::Origin::from_headers(&headers);
        assert_eq!(crate::audit::with_origin(origin.clone(), store.ui_language()).await, "ko-KR");
        assert_eq!(crate::audit::with_origin(origin, store.pc_language()).await, "ja", "what this PC sends the relay ignores the request");
        assert_eq!(store.ui_language().await, "ja", "without a request language the GUI language stays");
    }

    #[test]
    fn chatbot_persona_moves_to_memo() {
        let dir = tempfile::tempdir().unwrap();
        let store = ConfigStore::new(dir.path());
        store.file.set_section("chatbot", serde_json::json!({"settings": {"persona": "hi"}, "guilds": {"g": {"persona": "x", "memo": "keep"}}})).unwrap();
        let cfg = store.get_chatbot();
        assert_eq!(cfg.settings.get("memo"), Some(&serde_json::json!("hi")));
        assert!(!cfg.settings.contains_key("persona"));
        assert_eq!(cfg.guilds["g"].get("memo"), Some(&serde_json::json!("keep")));
    }

    /// 챗봇 채널은 릴레이가 원본이다: 예전 `channelIds`는 읽을 때 지우고, 그것만 있던 길드도 지운다
    #[test]
    fn chatbot_channel_ids_are_dropped() {
        let dir = tempfile::tempdir().unwrap();
        let store = ConfigStore::new(dir.path());
        store.file.set_section("chatbot", serde_json::json!({
            "settings": {"channelIds": ["1"], "model": "m"},
            "guilds": {"a": {"channelIds": ["2"]}, "b": {"channelIds": ["3"], "memo": "keep"}}
        })).unwrap();
        let cfg = store.get_chatbot();
        assert!(!cfg.settings.contains_key("channelIds") && cfg.settings.contains_key("model"));
        assert!(!cfg.guilds.contains_key("a"));
        assert_eq!(cfg.guilds["b"].len(), 1);
    }

    #[test]
    fn gui_settings_default_roundtrip() {
        let settings = GuiSettings::default();
        let json_str = serde_json::to_string_pretty(&settings).unwrap();
        let loaded: GuiSettings = serde_json::from_str(&json_str).unwrap();
        assert_eq!(settings, loaded);
    }

    #[test]
    fn gui_settings_camel_case_serialization() {
        let settings = GuiSettings::default();
        let value = serde_json::to_value(&settings).unwrap();
        // camelCase 키 확인
        assert!(value.get("autoRefresh").is_some());
        assert!(value.get("refreshInterval").is_some());
        assert!(value.get("ipcPort").is_some());
        assert!(value.get("consoleBufferSize").is_some());
        assert!(value.get("accentColor").is_some());
        assert!(value.get("portConflictCheck").is_some());
        assert!(value.get("windowOpacity").is_some());
        assert!(value.get("sidebarCompact").is_some());
        // snake_case가 아니어야 함
        assert!(value.get("auto_refresh").is_none());
        assert!(value.get("ipc_port").is_none());
    }

    #[test]
    fn gui_settings_missing_fields_use_defaults() {
        // 빈 JSON -> 모든 필드가 기본값
        let loaded: GuiSettings = serde_json::from_str("{}").unwrap();
        assert_eq!(loaded, GuiSettings::default());
    }

    #[test]
    fn gui_settings_partial_json() {
        // 일부 필드만 있는 JSON → 나머지는 기본값
        let json = r#"{"autoRefresh": false, "ipcPort": 12345}"#;
        let loaded: GuiSettings = serde_json::from_str(json).unwrap();
        assert!(!loaded.auto_refresh);
        assert_eq!(loaded.ipc_port, 12345);
        assert_eq!(loaded.refresh_interval, 2000); // default
        assert_eq!(loaded.accent_color, "#667eea"); // default
    }

    #[test]
    fn gui_settings_unknown_fields_ignored() {
        // 알 수 없는 필드가 있어도 에러 없이 파싱
        let json = r#"{"autoRefresh": true, "unknownField": 42, "legacy_field": "abc"}"#;
        let loaded: GuiSettings = serde_json::from_str(json).unwrap();
        assert!(loaded.auto_refresh);
    }

    #[test]
    fn gui_settings_legacy_compat_with_discord_fields() {
        // 레거시 settings.json에 discordToken/discordAutoStart가 있어도 파싱 가능
        let json = r#"{
            "autoRefresh": true,
            "discordToken": "old-token",
            "discordAutoStart": true,
            "ipcPort": 57474
        }"#;
        let loaded: GuiSettings = serde_json::from_str(json).unwrap();
        assert!(loaded.auto_refresh);
        assert_eq!(loaded.ipc_port, 57474);
    }

    #[test]
    fn gui_settings_bom_handling() {
        // BOM이 포함된 JSON 파싱
        let json_with_bom = "\u{feff}{\"autoRefresh\": false}";
        let content = json_with_bom.strip_prefix('\u{feff}').unwrap_or(json_with_bom);
        let loaded: GuiSettings = serde_json::from_str(content).unwrap();
        assert!(!loaded.auto_refresh);
    }

    // ── BotConfig 직렬화/역직렬화 ──

    #[test]
    fn bot_config_default_roundtrip() {
        let config = BotConfig::default();
        let json_str = serde_json::to_string_pretty(&config).unwrap();
        let loaded: BotConfig = serde_json::from_str(&json_str).unwrap();
        assert_eq!(config, loaded);
    }

    #[test]
    fn bot_config_camel_case_serialization() {
        let config = BotConfig::default();
        let value = serde_json::to_value(&config).unwrap();
        assert_eq!(value.get("prefixes"), Some(&serde_json::json!([])));
        assert!(value.get("prefix").is_none());
        assert!(value.get("autoStart").is_none());
        assert!(value.get("moduleAliases").is_some());
        assert!(value.get("commandAliases").is_some());
        assert!(value.get("token").is_none());
        assert!(value.get("musicEnabled").is_none());
        assert!(value.get("nodeSettings").is_some());
        assert!(value.get("cloudNodes").is_some());
        assert!(value.get("cloudMembers").is_some());
    }

    #[test]
    fn bot_config_missing_fields_use_defaults() {
        let loaded: BotConfig = serde_json::from_str("{}").unwrap();
        assert_eq!(loaded, BotConfig::default());
    }

    #[test]
    fn bot_config_partial_json() {
        let json = r#"{"prefixes": ["!test"], "cloud": {"hostId": "h1"}}"#;
        let loaded: BotConfig = serde_json::from_str(json).unwrap();
        assert_eq!(loaded.prefixes, vec!["!test"]);
        assert_eq!(loaded.cloud.host_id, "h1");
        assert!(loaded.module_aliases.is_empty()); // default
    }

    #[test]
    fn bot_config_full_json_compat() {
        // GUI가 보내는 실제 형태의 JSON
        let json = r#"{
            "prefix": "!saba",
            "token": "my-token",
            "autoStart": true,
            "mode": "cloud",
            "cloud": { "relayUrl": "https://relay.test", "hostId": "host-1" },
            "moduleAliases": { "mc": "minecraft" },
            "commandAliases": { "mc": { "start": "go" } },
            "musicEnabled": false,
            "musicChannelId": "ch-123",
            "musicUiSettings": { "queueLines": 10, "refreshInterval": 2000 },
            "nodeSettings": { "local": { "allowedInstances": ["a"] } },
            "cloudNodes": [{ "id": "n1" }],
            "cloudMembers": { "u1": "admin" }
        }"#;
        let loaded: BotConfig = serde_json::from_str(json).unwrap();
        // 예전 로컬 봇 필드(token, autoStart, mode, music*)는 무시된다
        assert_eq!(loaded.legacy_prefix.as_deref(), Some("!saba"));
        assert_eq!(loaded.cloud.relay_url, "https://relay.test");
        assert_eq!(loaded.cloud.host_id, "host-1");
        assert_eq!(loaded.module_aliases.get("mc").map(|s| s.as_str()), Some("minecraft"));
        assert!(serde_json::to_value(&loaded).unwrap().get("token").is_none());
    }

    #[test]
    fn bot_config_legacy_allowed_instances() {
        // 레거시: allowedInstances가 최상위에 있던 형태
        let json = r#"{
            "prefix": "!saba",
            "allowedInstances": ["id1", "id2"]
        }"#;
        // serde(default)로 파싱은 성공해야 함 (알 수 없는 필드 무시)
        let loaded: BotConfig = serde_json::from_str(json).unwrap();
        assert_eq!(loaded.legacy_prefix.as_deref(), Some("!saba"));
    }

    // ── ConfigStore 파일 I/O ──

    #[tokio::test]
    async fn config_store_creates_default_on_empty_dir() {
        let tmp = TempDir::new().unwrap();
        let store = ConfigStore::new(tmp.path());

        let gui = store.get_gui_settings().await;
        assert_eq!(gui, GuiSettings::default());

        let bot = store.get_bot_config().await;
        assert_eq!(bot, BotConfig::default());
    }

    #[tokio::test]
    async fn reset_clears_optional_keys_and_rejects_empty_or_unknown() {
        let tmp = TempDir::new().unwrap();
        let store = ConfigStore::new(tmp.path());
        store.set_gui_settings_from_json(serde_json::json!({"language": "ko", "fontScale": 150})).await.unwrap();

        let out = store.reset_to_defaults("gui", Some(&["language".to_string()])).await.unwrap();
        assert!(out.get("language").is_none());
        assert_eq!(out["fontScale"], 150);
        assert!(store.reset_to_defaults("gui", Some(&[])).await.is_err());
        assert!(store.reset_to_defaults("gui", Some(&["nope".to_string()])).await.is_err());
        let out = store.reset_to_defaults("gui", None).await.unwrap();
        assert_eq!(out["fontScale"], 100);
    }

    #[tokio::test]
    async fn resetting_the_bot_config_keeps_the_pinned_owner_and_import_brings_grants() {
        let tmp = TempDir::new().unwrap();
        let store = ConfigStore::new(tmp.path());
        store
            .update_node_settings("g", |node| {
                node.insert("ownerDiscordId".into(), serde_json::json!("own"));
                node.insert("members".into(), serde_json::json!({ "a": 2 }));
                ((), true)
            })
            .await
            .unwrap();
        // hostId는 노드 토큰을 따르는 값이라 초기화가 지우지 않는다 (relayUrl은 기본값으로)
        store
            .set_bot_config_from_json(serde_json::json!({ "cloud": { "relayUrl": "https://relay.example", "hostId": "h1" } }))
            .await
            .unwrap();
        let out = store.reset_to_defaults("bot", None).await.unwrap();
        assert_eq!(out["nodeSettings"]["g"], serde_json::json!({ "ownerDiscordId": "own", "members": { "a": 2 } }));
        assert_eq!((out["cloud"]["relayUrl"].as_str(), out["cloud"]["hostId"].as_str()), (Some(""), Some("h1")));
        let out = store.reset_to_defaults("bot", Some(&["cloud".to_string()])).await.unwrap();
        assert_eq!(out["cloud"]["hostId"], "h1");

        let mut sections = serde_json::Map::new();
        sections.insert("relay".into(), serde_json::json!({ "nodeSettings": { "h": { "ownerDiscordId": "x", "defaultLevel": -1 } } }));
        store.import_sections(sections).await.unwrap();
        assert_eq!(store.get_bot_config().await.node_settings["h"]["defaultLevel"], -1);
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 4)]
    async fn concurrent_gui_saves_keep_each_others_changes() {
        let tmp = TempDir::new().unwrap();
        let store = std::sync::Arc::new(ConfigStore::new(tmp.path()));
        for round in 0..40u32 {
            let a = store.clone();
            let b = store.clone();
            let ta = tokio::spawn(async move { a.set_gui_settings_from_json(serde_json::json!({ "fontScale": 100 + round })).await });
            let tb = tokio::spawn(async move { b.set_gui_settings_from_json(serde_json::json!({ "ipcPort": 20000 + round })).await });
            ta.await.unwrap().unwrap();
            tb.await.unwrap().unwrap();
            let now = store.get_gui_settings().await;
            assert_eq!((now.font_scale, now.ipc_port), (100 + round, 20000 + round as u16), "round {}", round);
        }
    }

    /// 봇 설정 키 일부 초기화가 그사이 저장된 다른 키를 되돌리지 않는다: 지금 값은 쓰기 잠금 안에서 읽는다 (D-F4)
    #[tokio::test(flavor = "multi_thread", worker_threads = 4)]
    async fn resetting_some_bot_keys_keeps_a_concurrent_save() {
        let tmp = TempDir::new().unwrap();
        let store = std::sync::Arc::new(ConfigStore::new(tmp.path()));
        for round in 0..200u32 {
            let a = store.clone();
            let b = store.clone();
            let ta = tokio::spawn(async move { a.reset_to_defaults("bot", Some(&["cloud".to_string()])).await });
            let tb = tokio::spawn(async move { b.set_bot_config_from_json(serde_json::json!({ "prefixes": [format!("p{round}")] })).await });
            ta.await.unwrap().unwrap();
            tb.await.unwrap().unwrap();
            assert_eq!(store.get_bot_config().await.prefixes, vec![format!("p{round}")], "round {}", round);
        }
    }

    #[tokio::test]
    async fn config_store_gui_save_load_roundtrip() {
        let tmp = TempDir::new().unwrap();
        let store = ConfigStore::new(tmp.path());

        let mut settings = GuiSettings::default();
        settings.auto_refresh = false;
        settings.ipc_port = 12345;
        settings.accent_color = "#ff0000".to_string();
        settings.font_scale = 150;

        store.set_gui_settings_from_json(serde_json::to_value(&settings).unwrap()).await.unwrap();

        // 인메모리 확인
        let loaded = store.get_gui_settings().await;
        assert_eq!(loaded.auto_refresh, false);
        assert_eq!(loaded.ipc_port, 12345);
        assert_eq!(loaded.accent_color, "#ff0000");
        assert_eq!(loaded.font_scale, 150);

        // 디스크에서 새로 로드하여 확인
        let store2 = ConfigStore::new(tmp.path());
        let loaded2 = store2.get_gui_settings().await;
        assert_eq!(loaded2, settings);
    }

    #[tokio::test]
    async fn config_store_bot_save_load_roundtrip() {
        let tmp = TempDir::new().unwrap();
        let store = ConfigStore::new(tmp.path());

        let mut config = BotConfig::default();
        config.prefixes = vec!["!test".to_string()];
        config.cloud.relay_url = "https://relay.test".to_string();
        config.module_aliases.insert("mc".to_string(), "minecraft".to_string());

        store.set_bot_config_from_json(serde_json::to_value(&config).unwrap()).await.unwrap();

        // 인메모리 확인
        let loaded = store.get_bot_config().await;
        assert_eq!(loaded.prefixes, vec!["!test"]);
        assert_eq!(loaded.cloud.relay_url, "https://relay.test");

        // 디스크에서 새로 로드
        let store2 = ConfigStore::new(tmp.path());
        let loaded2 = store2.get_bot_config().await;
        assert_eq!(loaded2, config);
    }

    #[tokio::test]
    async fn config_store_json_api_roundtrip() {
        let tmp = TempDir::new().unwrap();
        let store = ConfigStore::new(tmp.path());

        // JSON Value로 저장 (API 요청 시뮬레이션)
        let input = serde_json::json!({
            "autoRefresh": false,
            "ipcPort": 33333,
            "accentColor": "#00ff00"
        });
        let result = store.set_gui_settings_from_json(input).await.unwrap();
        assert!(!result.auto_refresh);
        assert_eq!(result.ipc_port, 33333);
        assert_eq!(result.accent_color, "#00ff00");
        // 머지되지 않은 필드는 기존값 유지
        assert_eq!(result.refresh_interval, 2000);

        // JSON Value로 읽기 (API 응답 시뮬레이션)
        let json = store.get_gui_settings_json().await;
        assert_eq!(json.get("autoRefresh").unwrap().as_bool().unwrap(), false);
        assert_eq!(json.get("ipcPort").unwrap().as_u64().unwrap(), 33333);
    }

    #[tokio::test]
    async fn an_old_single_prefix_becomes_a_list_and_the_old_default_becomes_the_default() {
        for (old, want) in [("!saba", vec![]), ("", vec![]), ("!x", vec!["!x"])] {
            let tmp = TempDir::new().unwrap();
            std::fs::write(tmp.path().join("config.json"), serde_json::json!({ "relay": { "prefix": old } }).to_string()).unwrap();
            let store = ConfigStore::new(tmp.path());
            assert_eq!(store.get_bot_config().await.prefixes, want, "{:?}", old);
            assert!(store.get_bot_config_json().await.get("prefix").is_none());
        }
    }

    #[tokio::test]
    async fn prefixes_must_be_short_words_and_few() {
        let tmp = TempDir::new().unwrap();
        let store = ConfigStore::new(tmp.path());
        let ok = store.set_bot_config_from_json(serde_json::json!({ "prefixes": ["사바쨩", "!s"] })).await.unwrap();
        assert_eq!(ok.prefixes, vec!["사바쨩", "!s"]);
        let long = "a".repeat(PREFIX_MAX_CHARS + 1);
        let many: Vec<String> = (0..=PREFIXES_MAX).map(|i| format!("p{}", i)).collect();
        for bad in [serde_json::json!(["두 단어"]), serde_json::json!([""]), serde_json::json!([long]), serde_json::json!(many)] {
            assert!(matches!(store.set_bot_config_from_json(serde_json::json!({ "prefixes": bad })).await, Err(ConfigStoreError::InvalidData(_))), "{}", bad);
        }
        assert_eq!(store.get_bot_config().await.prefixes, vec!["사바쨩", "!s"], "a rejected save keeps the old value");
    }

    /// 릴레이 주소는 https만, http는 루프백(모의 릴레이)만. 이미 저장된 값은 읽기와 무관한 저장을 막지 않는다
    #[tokio::test]
    async fn relay_url_must_be_https_or_loopback_http() {
        let tmp = TempDir::new().unwrap();
        std::fs::write(tmp.path().join("config.json"), serde_json::json!({ "relay": { "cloud": { "relayUrl": "http://old.example" } } }).to_string()).unwrap();
        let store = ConfigStore::new(tmp.path());
        assert_eq!(store.get_bot_config().await.cloud.relay_url, "http://old.example", "a stored value still loads");
        store.set_bot_config_from_json(serde_json::json!({ "prefixes": ["!s"] })).await.expect("an unrelated save keeps the stored relay");
        for ok in ["", "https://relay.test", "https://relay.test:8443/", "http://127.0.0.1:3000", "http://localhost:8080", "http://[::1]:9"] {
            let saved = store.set_bot_config_from_json(serde_json::json!({ "cloud": { "relayUrl": ok } })).await.unwrap_or_else(|e| panic!("{}: {}", ok, e));
            assert_eq!(saved.cloud.relay_url, ok);
        }
        for bad in ["http://relay.test", "http://127.0.0.1.evil.example", "ftp://relay.test", "relay.test", "https://", "javascript:alert(1)"] {
            let r = store.set_bot_config_from_json(serde_json::json!({ "cloud": { "relayUrl": bad } })).await;
            assert!(matches!(r, Err(ConfigStoreError::InvalidRelayUrl(_))), "{}: {:?}", bad, r);
            let mut sections = serde_json::Map::new();
            sections.insert("relay".into(), serde_json::json!({ "cloud": { "relayUrl": bad } }));
            assert!(matches!(store.check_sections(&sections).await, Err(ConfigStoreError::InvalidRelayUrl(_))), "import {}", bad);
        }
        assert_eq!(store.get_bot_config().await.cloud.relay_url, "http://[::1]:9", "a rejected save keeps the old value");
    }

    #[tokio::test]
    async fn config_store_bot_json_merge() {
        let tmp = TempDir::new().unwrap();
        let store = ConfigStore::new(tmp.path());

        // 완전한 설정 저장
        let full = serde_json::json!({
            "prefix": "!saba",
            "cloud": { "relayUrl": "https://relay.test", "hostId": "h1" },
            "moduleAliases": { "minecraft": "mc" }
        });
        store.set_bot_config_from_json(full).await.unwrap();

        // 일부만 업데이트
        let partial = serde_json::json!({
            "prefixes": ["!new"]
        });
        let result = store.set_bot_config_from_json(partial).await.unwrap();

        // 업데이트된 필드
        assert_eq!(result.prefixes, vec!["!new"]);
        // 기존 값 유지 (최상위 키 단위 병합)
        assert_eq!(result.cloud.host_id, "h1");
        assert_eq!(result.cloud.relay_url, "https://relay.test");
        assert_eq!(result.module_aliases.get("minecraft").map(|s| s.as_str()), Some("mc"));
    }

    #[tokio::test]
    async fn config_store_reads_existing_file() {
        let tmp = TempDir::new().unwrap();

        // 기존 settings.json 작성 (GUI가 이미 만든 파일 시뮬레이션)
        let existing = serde_json::json!({
            "autoRefresh": false,
            "refreshInterval": 5000,
            "ipcPort": 57474,
            "consoleBufferSize": 3000,
            "accentColor": "#ff00ff",
            "windowOpacity": 80
        });
        std::fs::write(
            tmp.path().join("settings.json"),
            serde_json::to_string_pretty(&existing).unwrap(),
        ).unwrap();

        let store = ConfigStore::new(tmp.path());
        let gui = store.get_gui_settings().await;
        assert!(!gui.auto_refresh);
        assert_eq!(gui.refresh_interval, 5000);
        assert_eq!(gui.console_buffer_size, 3000);
        assert_eq!(gui.accent_color, "#ff00ff");
        assert_eq!(gui.window_opacity, 80);
        // 파일에 없는 필드는 기본값
        assert!(gui.port_conflict_check);
        assert!(gui.enable_transitions);
    }

    #[tokio::test]
    async fn config_store_bom_file() {
        let tmp = TempDir::new().unwrap();

        // BOM이 있는 settings.json
        let json = r#"{"autoRefresh":false,"ipcPort":11111}"#;
        let bom_content = format!("\u{feff}{}", json);
        std::fs::write(tmp.path().join("settings.json"), bom_content).unwrap();

        let store = ConfigStore::new(tmp.path());
        let gui = store.get_gui_settings().await;
        assert!(!gui.auto_refresh);
        assert_eq!(gui.ipc_port, 11111);
    }

    // ── merge_json ──

    #[tokio::test]
    async fn client_sections_merge_and_reject_reserved_names() {
        let tmp = TempDir::new().unwrap();
        let store = ConfigStore::new(tmp.path());
        store.merge_client_section("cli", serde_json::json!({ "auto_start": false })).unwrap();
        store.merge_client_section("cli", serde_json::json!({ "language": "ko" })).unwrap();
        let cli = ConfigStore::new(tmp.path()).get_client_section("cli").unwrap();
        assert_eq!(cli["auto_start"], false);
        assert_eq!(cli["language"], "ko");

        for bad in ["gui", "updates", "extensions", "Cli", "../x", ""] {
            assert!(store.get_client_section(bad).is_err(), "{} must be rejected", bad);
        }
        // GUI 섹션은 클라이언트 섹션 쓰기에 영향받지 않는다
        store.set_gui_settings_from_json(serde_json::json!({ "language": "ja" })).await.unwrap();
        assert_eq!(ConfigStore::new(tmp.path()).get_client_section("cli").unwrap()["language"], "ko");
    }

    #[tokio::test]
    async fn consent_records_are_not_a_client_section_and_stay_on_this_pc() {
        let tmp = TempDir::new().unwrap();
        let store = ConfigStore::new(tmp.path());
        assert!(store.merge_client_section("sandbox", serde_json::json!({ "x": {} })).is_err(), "only the consent API writes it");
        store.file.set_section("sandbox", serde_json::json!({ "x": { "a": "1" } })).unwrap();
        assert!(!store.portable_sections(true).contains_key("sandbox"), "a consent given on this PC must not travel");
        let mut incoming = serde_json::Map::new();
        incoming.insert("sandbox".into(), serde_json::json!({ "x": { "b": "2" } }));
        let applied = store.import_sections(incoming).await.unwrap();
        assert!(applied.is_empty());
        assert_eq!(store.file.section("sandbox"), serde_json::json!({ "x": { "a": "1" } }));
    }

    #[test]
    fn saving_the_bot_config_keeps_permission_keys() {
        let current = serde_json::json!({ "nodeSettings": {
            "g": { "ownerDiscordId": "own", "members": { "a": 2 }, "allowedInstances": ["x"] },
            "h": { "defaultLevel": -1 },
            "k": { "allowedInstances": ["x"] }
        } });
        // 옛 스냅샷: k의 제한이 없던 때 것이라도 제한이 풀리면 안 된다
        let mut merged = merge_json(current.clone(), serde_json::json!({ "nodeSettings": {
            "g": { "ownerDiscordId": "evil", "members": { "evil": 2 }, "allowedInstances": ["y"], "preset": "loose" },
            "k": {}
        } }));
        // 되돌린 키는 알린다: 저장이 성공해도 그 값은 바뀌지 않았다 (L61)
        let ignored = keep_permission_keys(&mut merged, &current);
        assert_eq!(ignored, ["nodeSettings.g.allowedInstances", "nodeSettings.g.members", "nodeSettings.g.ownerDiscordId", "nodeSettings.g.preset"]);
        assert_eq!(merged["nodeSettings"], serde_json::json!({
            "g": { "ownerDiscordId": "own", "members": { "a": 2 }, "allowedInstances": ["x"] },
            "h": { "defaultLevel": -1 },
            "k": { "allowedInstances": ["x"] }
        }));
    }

    #[test]
    fn merge_json_overlay_wins() {
        let base = serde_json::json!({"a": 1, "b": 2});
        let overlay = serde_json::json!({"b": 99, "c": 3});
        let result = merge_json(base, overlay);
        assert_eq!(result, serde_json::json!({"a": 1, "b": 99, "c": 3}));
    }

    #[test]
    fn merge_json_non_object_overlay_replaces() {
        let base = serde_json::json!({"a": 1});
        let overlay = serde_json::json!("string");
        let result = merge_json(base, overlay);
        assert_eq!(result, serde_json::json!("string"));
    }
}
