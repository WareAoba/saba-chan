//! GUI가 만든 설정 파일을 공유해서 읽는 모듈
//!
//! - settings.json (%APPDATA%/saba-chan/settings.json)
//!   → discordToken, modulesPath, language, discordAutoStart 등
//! - bot-config.json (%APPDATA%/saba-chan/bot-config.json)
//!   → prefix, moduleAliases, commandAliases

use serde_json::Value;
use std::fs;
use std::path::PathBuf;

/// 공용 설정 디렉토리 경로 (%APPDATA%/saba-chan)
fn get_config_dir() -> anyhow::Result<PathBuf> {
    #[cfg(target_os = "windows")]
    {
        let appdata = std::env::var("APPDATA")?;
        Ok(PathBuf::from(appdata).join("saba-chan"))
    }

    #[cfg(not(target_os = "windows"))]
    {
        let home = std::env::var("HOME")?;
        Ok(PathBuf::from(home).join(".config").join("saba-chan"))
    }
}

/// settings.json 경로
fn get_settings_path() -> anyhow::Result<PathBuf> {
    Ok(get_config_dir()?.join("settings.json"))
}

/// bot-config.json 경로
fn get_bot_config_path() -> anyhow::Result<PathBuf> {
    Ok(get_config_dir()?.join("bot-config.json"))
}

/// bot-config.json 경로 (public — TUI에서 직접 접근 시)
pub fn get_bot_config_path_pub() -> PathBuf {
    get_config_dir()
        .map(|d| d.join("bot-config.json"))
        .unwrap_or_else(|_| PathBuf::from("bot-config.json"))
}

// ============ Settings (settings.json) ============

/// GUI의 settings.json 전체 로드
pub fn load_settings() -> anyhow::Result<Value> {
    let path = get_settings_path()?;
    if path.exists() {
        let content = fs::read_to_string(&path)?;
        Ok(serde_json::from_str(&content)?)
    } else {
        Ok(serde_json::json!({
            "modulesPath": "",
            "autoRefresh": true,
            "refreshInterval": 2000,
            "language": "en"
        }))
    }
}

/// Discord 봇 토큰 가져오기 (settings.json → discordToken)
pub fn get_discord_token() -> anyhow::Result<Option<String>> {
    let settings = load_settings()?;
    Ok(settings
        .get("discordToken")
        .and_then(|v| v.as_str())
        .filter(|s| !s.is_empty())
        .map(|s| s.to_string()))
}

/// 모듈 경로 가져오기 (settings.json → modulesPath)
/// GUI가 설정한 경로 또는 프로젝트 루트/modules 폴백
pub fn get_modules_path() -> anyhow::Result<String> {
    let settings = load_settings()?;
    let configured = settings
        .get("modulesPath")
        .and_then(|v| v.as_str())
        .filter(|s| !s.is_empty())
        .map(|s| s.to_string());

    if let Some(path) = configured {
        // 상대 경로면 프로젝트 루트 기준으로 해석
        let p = PathBuf::from(&path);
        if p.is_absolute() && p.exists() {
            return Ok(path);
        }
        // 상대 경로라면 프로젝트 루트 기준
        let root = crate::process::find_project_root()?;
        let resolved = root.join(&path);
        if resolved.exists() {
            return Ok(resolved.to_string_lossy().to_string());
        }
        // 존재하지 않아도 설정된 값 반환
        return Ok(path);
    }

    // 기본값: 프로젝트 루트/modules
    let root = crate::process::find_project_root()?;
    Ok(root.join("modules").to_string_lossy().to_string())
}

/// 언어 설정 가져오기
pub fn get_language() -> anyhow::Result<String> {
    let settings = load_settings()?;
    Ok(settings
        .get("language")
        .and_then(|v| v.as_str())
        .unwrap_or("en")
        .to_string())
}

/// Discord 자동 시작 설정 가져오기
#[allow(dead_code)]
pub fn get_discord_auto_start() -> anyhow::Result<bool> {
    let settings = load_settings()?;
    Ok(settings
        .get("discordAutoStart")
        .and_then(|v| v.as_bool())
        .unwrap_or(false))
}

// ============ Bot Config (bot-config.json) ============

/// GUI의 bot-config.json 전체 로드
pub fn load_bot_config() -> anyhow::Result<Value> {
    let path = get_bot_config_path()?;
    if path.exists() {
        let content = fs::read_to_string(&path)?;
        Ok(serde_json::from_str(&content)?)
    } else {
        Ok(serde_json::json!({
            "prefix": "!saba",
            "moduleAliases": {},
            "commandAliases": {}
        }))
    }
}

/// 봇 prefix 가져오기 (bot-config.json)
pub fn get_bot_prefix() -> anyhow::Result<String> {
    let config = load_bot_config()?;
    Ok(config
        .get("prefix")
        .and_then(|v| v.as_str())
        .unwrap_or("!saba")
        .to_string())
}

/// 모듈 별명 조회 (bot-config.json → moduleAliases)
#[allow(dead_code)]
pub fn get_module_aliases() -> anyhow::Result<Vec<(String, String)>> {
    let config = load_bot_config()?;
    let mut aliases = Vec::new();
    if let Some(map) = config.get("moduleAliases").and_then(|m| m.as_object()) {
        for (alias, module) in map {
            if let Some(module_str) = module.as_str() {
                aliases.push((alias.clone(), module_str.to_string()));
            }
        }
    }
    Ok(aliases)
}

/// instances.json 경로 (%APPDATA%/saba-chan/instances.json)
pub fn get_instances_path() -> anyhow::Result<PathBuf> {
    let config_dir = get_config_dir()?;
    let appdata_path = config_dir.join("instances.json");

    // AppData에 있으면 그대로 사용
    if appdata_path.exists() {
        return Ok(appdata_path);
    }

    // 레거시 경로 마이그레이션: 프로젝트 루트에 있던 instances.json을 AppData로 이동
    if let Ok(root) = crate::process::find_project_root() {
        for legacy in &[
            root.join("config").join("instances.json"),
            root.join("instances.json"),
        ] {
            if legacy.exists() {
                std::fs::create_dir_all(&config_dir)?;
                std::fs::rename(legacy, &appdata_path)?;
                return Ok(appdata_path);
            }
        }
    }

    // 기본: AppData 경로 (없으면 새로 생성됨)
    Ok(appdata_path)
}

// ============ Write functions ============

/// settings.json에 값 쓰기 (없으면 파일 생성)
fn save_settings(settings: &Value) -> anyhow::Result<()> {
    let path = get_settings_path()?;
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)?;
    }
    let content = serde_json::to_string_pretty(settings)?;
    fs::write(&path, content)?;
    Ok(())
}

/// bot-config.json에 값 쓰기
fn save_bot_config(config: &Value) -> anyhow::Result<()> {
    let path = get_bot_config_path()?;
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)?;
    }
    let content = serde_json::to_string_pretty(config)?;
    fs::write(&path, content)?;
    Ok(())
}

/// Discord 토큰 설정
pub fn set_discord_token(token: &str) -> anyhow::Result<()> {
    let mut settings = load_settings()?;
    settings["discordToken"] = serde_json::Value::String(token.to_string());
    save_settings(&settings)
}

/// Discord 토큰 삭제
pub fn clear_discord_token() -> anyhow::Result<()> {
    let mut settings = load_settings()?;
    settings["discordToken"] = serde_json::Value::String(String::new());
    save_settings(&settings)
}

/// 봇 prefix 변경
pub fn set_bot_prefix(prefix: &str) -> anyhow::Result<()> {
    let mut config = load_bot_config()?;
    config["prefix"] = serde_json::Value::String(prefix.to_string());
    save_bot_config(&config)
}

/// 모듈 경로 변경
#[allow(dead_code)]
pub fn set_modules_path(path: &str) -> anyhow::Result<()> {
    let mut settings = load_settings()?;
    settings["modulesPath"] = serde_json::Value::String(path.to_string());
    save_settings(&settings)
}

/// Discord 봇 자동 시작 설정 변경
#[allow(dead_code)]
pub fn set_discord_auto_start(enabled: bool) -> anyhow::Result<()> {
    let mut settings = load_settings()?;
    settings["discordAutoStart"] = serde_json::Value::Bool(enabled);
    save_settings(&settings)
}

/// 언어 설정 변경
#[allow(dead_code)]
pub fn set_language(lang: &str) -> anyhow::Result<()> {
    let mut settings = load_settings()?;
    settings["language"] = serde_json::Value::String(lang.to_string());
    save_settings(&settings)
}

/// 설정 요약 출력용
#[allow(dead_code)]
pub fn config_summary() -> String {
    let token = get_discord_token().ok().flatten();
    let modules_path = get_modules_path().unwrap_or_default();
    let prefix = get_bot_prefix().unwrap_or_else(|_| "!saba".to_string());
    let lang = get_language().unwrap_or_else(|_| "en".to_string());
    let settings_path = get_settings_path()
        .map(|p| p.display().to_string())
        .unwrap_or_else(|_| "?".to_string());

    let mut lines = Vec::new();
    lines.push("GUI Configuration:".to_string());
    lines.push(format!("  Settings:    {}", settings_path));
    lines.push(format!("  Token:       {}", if token.is_some() { "✓ configured" } else { "✗ not set" }));
    lines.push(format!("  Prefix:      {}", prefix));
    lines.push(format!("  Modules:     {}", modules_path));
    lines.push(format!("  Language:    {}", lang));
    lines.join("\n")
}
