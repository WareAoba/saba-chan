//! 중앙 설정 저장소 (ConfigStore)
//!
//! 모든 JSON 설정 파일의 저장/로드를 타입 안전하게 처리한다.
//! - settings.json (GUI 설정)
//! - bot-config.json (Discord 봇 설정)
//!
//! 설계 원칙:
//! - `serde(default)` + 강타입으로 필드 누락/오타를 컴파일 타임에 방지
//! - 인메모리 캐시 + 변경 시 flush
//! - 기존 JSON 파일과 하위호환 유지 (`#[serde(rename_all = "camelCase")]`)

use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::collections::HashMap;
use std::path::{Path, PathBuf};
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
            window_bounds: None,
            language: None,
        }
    }
}

// ── Bot Config (bot-config.json) ──

/// Discord 봇 설정 — bot-config.json에 저장되는 모든 필드
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase", default)]
pub struct BotConfig {
    pub prefix: String,
    pub token: String,
    pub auto_start: bool,
    pub mode: String,
    pub cloud: CloudConfig,
    pub module_aliases: HashMap<String, String>,
    pub command_aliases: HashMap<String, Value>,
    pub music_enabled: bool,
    pub music_channel_id: String,
    pub music_ui_settings: MusicUiSettings,
    pub node_settings: Value,
    pub cloud_nodes: Vec<Value>,
    pub cloud_members: HashMap<String, Value>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase", default)]
pub struct CloudConfig {
    pub relay_url: String,
    pub host_id: String,
}

impl Default for CloudConfig {
    fn default() -> Self {
        Self {
            relay_url: String::new(),
            host_id: String::new(),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase", default)]
pub struct MusicUiSettings {
    pub queue_lines: u32,
    pub refresh_interval: u32,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub normalize: Option<bool>,
}

impl Default for MusicUiSettings {
    fn default() -> Self {
        Self {
            queue_lines: 5,
            refresh_interval: 4000,
            normalize: None,
        }
    }
}

impl Default for BotConfig {
    fn default() -> Self {
        Self {
            prefix: "!saba".to_string(),
            token: String::new(),
            auto_start: false,
            mode: "local".to_string(),
            cloud: CloudConfig::default(),
            module_aliases: HashMap::new(),
            command_aliases: HashMap::new(),
            music_enabled: true,
            music_channel_id: String::new(),
            music_ui_settings: MusicUiSettings::default(),
            node_settings: Value::Object(serde_json::Map::new()),
            cloud_nodes: Vec::new(),
            cloud_members: HashMap::new(),
        }
    }
}

// ── ConfigStore ──

/// 중앙 설정 저장소 — 모든 JSON 설정의 인메모리 캐시 + 파일 I/O
pub struct ConfigStore {
    data_dir: PathBuf,
    gui_settings: RwLock<GuiSettings>,
    bot_config: RwLock<BotConfig>,
}

#[allow(dead_code)]
impl ConfigStore {
    /// 새 ConfigStore 생성 (디스크에서 기존 설정 로드)
    pub fn new(data_dir: &Path) -> Self {
        let gui = load_from_file::<GuiSettings>(&data_dir.join("settings.json"));
        let bot = load_from_file::<BotConfig>(&data_dir.join("bot-config.json"));

        Self {
            data_dir: data_dir.to_path_buf(),
            gui_settings: RwLock::new(gui),
            bot_config: RwLock::new(bot),
        }
    }

    // ── GUI Settings ──

    /// GUI 설정 읽기 (인메모리 캐시에서 반환)
    pub async fn get_gui_settings(&self) -> GuiSettings {
        self.gui_settings.read().await.clone()
    }

    /// GUI 설정 전체 교체 (인메모리 + 디스크)
    pub async fn set_gui_settings(&self, settings: GuiSettings) -> Result<(), ConfigStoreError> {
        let path = self.data_dir.join("settings.json");
        save_to_file(&path, &settings)?;
        *self.gui_settings.write().await = settings;
        Ok(())
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
        let path = self.data_dir.join("settings.json");
        save_to_file(&path, &settings)?;
        *self.gui_settings.write().await = settings.clone();
        Ok(settings)
    }

    // ── Bot Config ──

    /// 봇 설정 읽기 (인메모리 캐시에서 반환)
    pub async fn get_bot_config(&self) -> BotConfig {
        self.bot_config.read().await.clone()
    }

    /// 봇 설정 전체 교체 (인메모리 + 디스크)
    pub async fn set_bot_config(&self, config: BotConfig) -> Result<(), ConfigStoreError> {
        let path = self.data_dir.join("bot-config.json");
        save_to_file(&path, &config)?;
        *self.bot_config.write().await = config;
        Ok(())
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
        let path = self.data_dir.join("bot-config.json");
        save_to_file(&path, &config)?;
        *self.bot_config.write().await = config.clone();
        Ok(config)
    }

    /// data_dir 경로 반환 (테스트/디버그용)
    pub fn data_dir(&self) -> &Path {
        &self.data_dir
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

/// 파일에서 타입 T를 로드. 실패 시 Default 반환.
fn load_from_file<T: for<'de> Deserialize<'de> + Default>(path: &Path) -> T {
    match std::fs::read_to_string(path) {
        Ok(content) => {
            // BOM 제거
            let content = content.strip_prefix('\u{feff}').unwrap_or(&content);
            serde_json::from_str(content).unwrap_or_default()
        }
        Err(_) => T::default(),
    }
}

/// 타입 T를 파일에 저장 (pretty print).
fn save_to_file<T: Serialize>(path: &Path, value: &T) -> Result<(), ConfigStoreError> {
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)?;
    }
    let json_str = serde_json::to_string_pretty(value)?;
    std::fs::write(path, json_str)?;
    Ok(())
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
        assert!(value.get("autoStart").is_some());
        assert!(value.get("moduleAliases").is_some());
        assert!(value.get("commandAliases").is_some());
        assert!(value.get("musicEnabled").is_some());
        assert!(value.get("musicChannelId").is_some());
        assert!(value.get("musicUiSettings").is_some());
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
        let json = r#"{"prefix": "!test", "token": "tok-123"}"#;
        let loaded: BotConfig = serde_json::from_str(json).unwrap();
        assert_eq!(loaded.prefix, "!test");
        assert_eq!(loaded.token, "tok-123");
        assert!(!loaded.auto_start); // default
        assert_eq!(loaded.mode, "local"); // default
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
        assert_eq!(loaded.prefix, "!saba");
        assert_eq!(loaded.token, "my-token");
        assert!(loaded.auto_start);
        assert_eq!(loaded.mode, "cloud");
        assert_eq!(loaded.cloud.relay_url, "https://relay.test");
        assert_eq!(loaded.cloud.host_id, "host-1");
        assert_eq!(loaded.module_aliases.get("mc").map(|s| s.as_str()), Some("minecraft"));
        assert!(!loaded.music_enabled);
        assert_eq!(loaded.music_channel_id, "ch-123");
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
    async fn config_store_gui_save_load_roundtrip() {
        let tmp = TempDir::new().unwrap();
        let store = ConfigStore::new(tmp.path());

        let mut settings = GuiSettings::default();
        settings.auto_refresh = false;
        settings.ipc_port = 12345;
        settings.accent_color = "#ff0000".to_string();
        settings.font_scale = 150;

        store.set_gui_settings(settings.clone()).await.unwrap();

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
        config.token = "secret-token".to_string();
        config.auto_start = true;
        config.mode = "cloud".to_string();
        config.cloud.relay_url = "https://relay.test".to_string();
        config.module_aliases.insert("mc".to_string(), "minecraft".to_string());

        store.set_bot_config(config.clone()).await.unwrap();

        // 인메모리 확인
        let loaded = store.get_bot_config().await;
        assert_eq!(loaded.prefix, "!test");
        assert_eq!(loaded.token, "secret-token");
        assert!(loaded.auto_start);
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
            "token": "tok-1",
            "autoStart": true,
            "musicEnabled": true,
            "musicChannelId": "ch-1"
        });
        store.set_bot_config_from_json(full).await.unwrap();

        // 일부만 업데이트
        let partial = serde_json::json!({
            "prefix": "!new",
            "token": "tok-2"
        });
        let result = store.set_bot_config_from_json(partial).await.unwrap();

        // 업데이트된 필드
        assert_eq!(result.prefix, "!new");
        assert_eq!(result.token, "tok-2");
        // 기존 값 유지
        assert!(result.auto_start);
        assert!(result.music_enabled);
        assert_eq!(result.music_channel_id, "ch-1");
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
