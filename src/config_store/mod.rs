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
    pub prefix: String,
    pub cloud: CloudConfig,
    pub module_aliases: HashMap<String, String>,
    pub command_aliases: HashMap<String, Value>,
    /// 인스턴스 ID → 그 인스턴스를 부르는 별명 ("a, b"). 같은 게임 서버가 여럿일 때 쓴다
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
            prefix: "!saba".to_string(),
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
    /// 길드별로 덮어쓰는 설정 (예: channelIds)
    pub guilds: HashMap<String, serde_json::Map<String, Value>>,
}

impl ChatbotConfig {
    /// 예전 키를 지금 계약으로 (2026-10-17: `persona` → `memo`)
    fn migrate(&mut self) {
        for map in std::iter::once(&mut self.settings).chain(self.guilds.values_mut()) {
            if let Some(persona) = map.remove("persona") {
                map.entry("memo").or_insert(persona);
            }
        }
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

/// 클라이언트 네임스페이스로 쓸 수 없는 키 (각자 전용 API가 있다)
const RESERVED_SECTIONS: &[&str] = &["schemaVersion", "gui", "relay", "updates", "boot", "extensions", "chatbot"];

impl ConfigStore {
    /// 새 ConfigStore 생성 (디스크에서 기존 설정 로드, 예전 파일은 이관)
    pub fn new(data_dir: &Path) -> Self {
        let file = ConfigFile::at(data_dir);
        let doc = file.load();
        let gui = section_or_default::<GuiSettings>(&doc, "gui");
        let bot = section_or_default::<BotConfig>(&doc, "relay");

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

    /// GUI 설정을 JSON Value로 반환 (API 응답용)
    pub async fn get_gui_settings_json(&self) -> Value {
        let settings = self.gui_settings.read().await;
        serde_json::to_value(&*settings).unwrap_or_default()
    }

    /// JSON Value로 GUI 설정 교체 (API 요청 수용, 스키마 검증 포함)
    pub async fn set_gui_settings_from_json(&self, json: Value) -> Result<GuiSettings, ConfigStoreError> {
        // 기존 설정을 base로 하여 부분 업데이트 지원
        let current = self.gui_settings.read().await.clone();
        let current_json = serde_json::to_value(&current).unwrap_or_default();

        // 기존 JSON에 새 JSON을 머지
        let merged = merge_json(current_json, json);

        let settings: GuiSettings = serde_json::from_value(merged)
            .map_err(|e| ConfigStoreError::InvalidData(format!("Invalid GUI settings: {}", e)))?;
        self.file.set_section("gui", serde_json::to_value(&settings)?)?;
        *self.gui_settings.write().await = settings.clone();
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
        // 기존 설정을 base로 하여 부분 업데이트 지원
        let current = self.bot_config.read().await.clone();
        let current_json = serde_json::to_value(&current).unwrap_or_default();

        let merged = merge_json(current_json, json);

        let config: BotConfig = serde_json::from_value(merged)
            .map_err(|e| ConfigStoreError::InvalidData(format!("Invalid bot config: {}", e)))?;
        self.file.set_section("relay", serde_json::to_value(&config)?)?;
        *self.bot_config.write().await = config.clone();
        Ok(config)
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
        let full = match keys {
            None => defaults,
            Some([]) => {
                return Err(ConfigStoreError::InvalidData("keys is empty; omit keys to reset everything".into()))
            }
            Some(keys) => {
                let current = if kind == "gui" { self.get_gui_settings_json().await } else { self.get_bot_config_json().await };
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
        if kind == "gui" {
            let settings: GuiSettings = serde_json::from_value(Value::Object(full))
                .map_err(|e| ConfigStoreError::InvalidData(format!("Invalid GUI settings: {}", e)))?;
            self.file.set_section("gui", serde_json::to_value(&settings)?)?;
            *self.gui_settings.write().await = settings;
            Ok(self.get_gui_settings_json().await)
        } else {
            let config: BotConfig = serde_json::from_value(Value::Object(full))
                .map_err(|e| ConfigStoreError::InvalidData(format!("Invalid bot config: {}", e)))?;
            self.file.set_section("relay", serde_json::to_value(&config)?)?;
            *self.bot_config.write().await = config;
            Ok(self.get_bot_config_json().await)
        }
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
        assert!(value.get("prefix").is_some());
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
        let json = r#"{"prefix": "!test", "cloud": {"hostId": "h1"}}"#;
        let loaded: BotConfig = serde_json::from_str(json).unwrap();
        assert_eq!(loaded.prefix, "!test");
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
        assert_eq!(loaded.prefix, "!saba");
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
        assert_eq!(loaded.prefix, "!saba");
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
        config.prefix = "!test".to_string();
        config.cloud.relay_url = "https://relay.test".to_string();
        config.module_aliases.insert("mc".to_string(), "minecraft".to_string());

        store.set_bot_config_from_json(serde_json::to_value(&config).unwrap()).await.unwrap();

        // 인메모리 확인
        let loaded = store.get_bot_config().await;
        assert_eq!(loaded.prefix, "!test");
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
            "prefix": "!new"
        });
        let result = store.set_bot_config_from_json(partial).await.unwrap();

        // 업데이트된 필드
        assert_eq!(result.prefix, "!new");
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
