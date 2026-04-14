//! 데몬 API를 통한 설정 읽기/쓰기 모듈
//!
//! 원칙: 모든 설정 읽기/쓰기는 데몬 API를 경유한다.
//! 로컬 파일 직접 접근은 IPC 포트 부트스트랩에만 허용한다.
//!
//! - settings.json → GET/PUT /api/config/gui
//! - bot-config.json → GET/PUT /api/config/bot

use saba_chan_updater_lib::constants;
use serde_json::Value;
use std::path::PathBuf;

// ============ Daemon API 헬퍼 (blocking) ============

/// IPC 인증 토큰 읽기 (데몬이 생성한 .ipc_token)
fn read_ipc_token() -> Option<String> {
    let path = constants::token_file_path();
    std::fs::read_to_string(path)
        .ok()
        .map(|s| s.trim().to_string())
        .filter(|s| !s.is_empty())
}

/// blocking reqwest 클라이언트로 데몬 API GET 호출
fn daemon_get(path: &str) -> Option<Value> {
    let base = format!("http://127.0.0.1:{}", get_ipc_port_local());
    let token = read_ipc_token()?;
    reqwest::blocking::Client::builder()
        .timeout(std::time::Duration::from_secs(3))
        .build()
        .ok()?
        .get(format!("{}{}", base, path))
        .header("X-Saba-Token", &token)
        .send()
        .ok()
        .filter(|r| r.status().is_success())
        .and_then(|r| r.json().ok())
}

/// blocking reqwest 클라이언트로 데몬 API PUT 호출
fn daemon_put(path: &str, body: &Value) -> anyhow::Result<()> {
    let base = format!("http://127.0.0.1:{}", get_ipc_port_local());
    let token = read_ipc_token()
        .ok_or_else(|| anyhow::anyhow!("IPC token not available"))?;
    let resp = reqwest::blocking::Client::builder()
        .timeout(std::time::Duration::from_secs(3))
        .build()?
        .put(format!("{}{}", base, path))
        .header("X-Saba-Token", &token)
        .json(body)
        .send()?;
    if !resp.status().is_success() {
        anyhow::bail!("Daemon PUT {} failed: {}", path, resp.status());
    }
    Ok(())
}

/// blocking reqwest 클라이언트로 데몬 API DELETE 호출
fn daemon_delete(path: &str) -> anyhow::Result<()> {
    let base = format!("http://127.0.0.1:{}", get_ipc_port_local());
    let token = read_ipc_token()
        .ok_or_else(|| anyhow::anyhow!("IPC token not available"))?;
    let resp = reqwest::blocking::Client::builder()
        .timeout(std::time::Duration::from_secs(3))
        .build()?
        .delete(format!("{}{}", base, path))
        .header("X-Saba-Token", &token)
        .send()?;
    if !resp.status().is_success() {
        anyhow::bail!("Daemon DELETE {} failed: {}", path, resp.status());
    }
    Ok(())
}

/// blocking reqwest 클라이언트로 데몬 API POST 호출
fn daemon_post(path: &str, body: &Value) -> Option<Value> {
    let base = format!("http://127.0.0.1:{}", get_ipc_port_local());
    let token = read_ipc_token()?;
    reqwest::blocking::Client::builder()
        .timeout(std::time::Duration::from_secs(3))
        .build()
        .ok()?
        .post(format!("{}{}", base, path))
        .header("X-Saba-Token", &token)
        .json(body)
        .send()
        .ok()
        .filter(|r| r.status().is_success())
        .and_then(|r| r.json().ok())
}

/// 공용 설정 디렉토리 경로 — constants 모듈에 위임
fn get_config_dir() -> anyhow::Result<PathBuf> {
    Ok(constants::resolve_data_dir())
}

/// IPC 포트 — 로컬 파일에서만 읽기 (daemon_get의 기반이므로 순환 방지)
/// NOTE: 이것이 유일한 로컬 파일 직접 읽기. daemon API는 이 포트가 있어야 호출 가능.
fn get_ipc_port_local() -> u16 {
    let path = constants::resolve_settings_path();
    std::fs::read_to_string(path)
        .ok()
        .and_then(|c| serde_json::from_str::<Value>(&c).ok())
        .and_then(|s| s.get("ipcPort").and_then(|v| v.as_u64()))
        .map(|p| p as u16)
        .unwrap_or(constants::DEFAULT_IPC_PORT)
}

// ============ Settings (settings.json) — 데몬 API 전용 ============

/// GUI 설정 로드 (데몬 API 전용)
pub fn load_settings() -> anyhow::Result<Value> {
    daemon_get("/api/config/gui")
        .ok_or_else(|| anyhow::anyhow!("데몬에 연결할 수 없습니다. 데몬이 실행 중인지 확인하세요."))
}

/// Discord 봇 토큰 가져오기 (bot-config.json → token)
pub fn get_discord_token() -> anyhow::Result<Option<String>> {
    let config = load_bot_config()?;
    Ok(config
        .get("token")
        .and_then(|v| v.as_str())
        .filter(|s| !s.is_empty())
        .map(|s| s.to_string()))
}

/// 모듈 경로 가져오기 — %APPDATA%/saba-chan/modules 고정
/// SABA_MODULES_PATH 환경 변수가 설정되면 해당 경로를 우선 사용 (테스트/개발용)
pub fn get_modules_path() -> anyhow::Result<String> {
    // 환경 변수 오버라이드
    if let Ok(p) = std::env::var("SABA_MODULES_PATH") {
        if !p.is_empty() {
            let path = PathBuf::from(&p);
            if !path.exists() {
                let _ = std::fs::create_dir_all(&path);
            }
            return Ok(p);
        }
    }

    let dir = get_config_dir()?.join("modules");
    if !dir.exists() {
        std::fs::create_dir_all(&dir)?;
    }
    Ok(dir.to_string_lossy().to_string())
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

/// IPC 포트 가져오기 (settings.json → ipcPort)
pub fn get_ipc_port() -> u16 {
    load_settings()
        .ok()
        .and_then(|s| s.get("ipcPort").and_then(|v| v.as_u64()))
        .map(|p| p as u16)
        .unwrap_or(constants::DEFAULT_IPC_PORT)
}

/// IPC base URL 가져오기 (http://127.0.0.1:{port})
/// NOTE: 로컬 파일에서 포트를 읽는다 — async 런타임 안에서도 안전.
pub fn get_ipc_base_url() -> String {
    format!("http://127.0.0.1:{}", get_ipc_port_local())
}

/// Discord 자동 시작 설정 가져오기 (bot-config.json → autoStart)
#[allow(dead_code)]
pub fn get_discord_auto_start() -> anyhow::Result<bool> {
    let config = load_bot_config()?;
    Ok(config
        .get("autoStart")
        .and_then(|v| v.as_bool())
        .unwrap_or(false))
}

// ============ Bot Config (bot-config.json) — 데몬 API 전용 ============

/// 봇 설정 로드 (데몬 API 전용)
pub fn load_bot_config() -> anyhow::Result<Value> {
    daemon_get("/api/config/bot")
        .ok_or_else(|| anyhow::anyhow!("데몬에 연결할 수 없습니다. 데몬이 실행 중인지 확인하세요."))
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

/// 인스턴스 디렉토리 경로 (%APPDATA%/saba-chan/instances/)
pub fn get_instances_path() -> anyhow::Result<PathBuf> {
    let config_dir = get_config_dir()?;
    Ok(config_dir.join("instances"))
}

// ============ Write functions — 데몬 API 전용 ============

/// GUI 설정 저장 (데몬 API 전용)
fn save_settings(settings: &Value) -> anyhow::Result<()> {
    daemon_put("/api/config/gui", settings)
}

/// 봇 설정 저장 (데몬 API 전용)
pub fn save_bot_config(config: &Value) -> anyhow::Result<()> {
    daemon_put("/api/config/bot", config)
}

/// Discord 토큰 설정 (bot-config.json → token)
pub fn set_discord_token(token: &str) -> anyhow::Result<()> {
    let mut config = load_bot_config()?;
    config["token"] = serde_json::Value::String(token.to_string());
    save_bot_config(&config)
}

/// Discord 토큰 삭제 (bot-config.json → token)
pub fn clear_discord_token() -> anyhow::Result<()> {
    let mut config = load_bot_config()?;
    config["token"] = serde_json::Value::String(String::new());
    save_bot_config(&config)
}

/// 봇 prefix 변경
pub fn set_bot_prefix(prefix: &str) -> anyhow::Result<()> {
    let mut config = load_bot_config()?;
    config["prefix"] = serde_json::Value::String(prefix.to_string());
    save_bot_config(&config)
}

/// Discord 봇 자동 시작 설정 변경 (bot-config.json → autoStart)
pub fn set_discord_auto_start(enabled: bool) -> anyhow::Result<()> {
    let mut config = load_bot_config()?;
    config["autoStart"] = serde_json::Value::Bool(enabled);
    save_bot_config(&config)
}

/// autoRefresh 가져오기
pub fn get_auto_refresh() -> anyhow::Result<bool> {
    let settings = load_settings()?;
    Ok(settings.get("autoRefresh").and_then(|v| v.as_bool()).unwrap_or(true))
}

/// autoRefresh 설정
pub fn set_auto_refresh(enabled: bool) -> anyhow::Result<()> {
    let mut settings = load_settings()?;
    settings["autoRefresh"] = serde_json::Value::Bool(enabled);
    save_settings(&settings)
}

/// refreshInterval 가져오기 (ms)
pub fn get_refresh_interval() -> anyhow::Result<u64> {
    let settings = load_settings()?;
    Ok(settings.get("refreshInterval").and_then(|v| v.as_u64()).unwrap_or(2000))
}

/// refreshInterval 설정 (ms)
pub fn set_refresh_interval(ms: u64) -> anyhow::Result<()> {
    let mut settings = load_settings()?;
    settings["refreshInterval"] = serde_json::json!(ms);
    save_settings(&settings)
}

/// ipcPort 설정
pub fn set_ipc_port(port: u16) -> anyhow::Result<()> {
    let mut settings = load_settings()?;
    settings["ipcPort"] = serde_json::json!(port);
    save_settings(&settings)
}

/// consoleBufferSize 가져오기
pub fn get_console_buffer_size() -> anyhow::Result<u64> {
    let settings = load_settings()?;
    Ok(settings.get("consoleBufferSize").and_then(|v| v.as_u64()).unwrap_or(2000))
}

/// consoleBufferSize 설정
pub fn set_console_buffer_size(size: u64) -> anyhow::Result<()> {
    let mut settings = load_settings()?;
    settings["consoleBufferSize"] = serde_json::json!(size);
    save_settings(&settings)
}

/// autoGeneratePasswords 가져오기
pub fn get_auto_generate_passwords() -> anyhow::Result<bool> {
    let settings = load_settings()?;
    Ok(settings.get("autoGeneratePasswords").and_then(|v| v.as_bool()).unwrap_or(true))
}

/// autoGeneratePasswords 설정
pub fn set_auto_generate_passwords(enabled: bool) -> anyhow::Result<()> {
    let mut settings = load_settings()?;
    settings["autoGeneratePasswords"] = serde_json::Value::Bool(enabled);
    save_settings(&settings)
}

/// portConflictCheck 가져오기
pub fn get_port_conflict_check() -> anyhow::Result<bool> {
    let settings = load_settings()?;
    Ok(settings.get("portConflictCheck").and_then(|v| v.as_bool()).unwrap_or(true))
}

/// portConflictCheck 설정
pub fn set_port_conflict_check(enabled: bool) -> anyhow::Result<()> {
    let mut settings = load_settings()?;
    settings["portConflictCheck"] = serde_json::Value::Bool(enabled);
    save_settings(&settings)
}

/// 언어 설정 변경
#[allow(dead_code)]
pub fn set_language(lang: &str) -> anyhow::Result<()> {
    let mut settings = load_settings()?;
    settings["language"] = serde_json::Value::String(lang.to_string());
    save_settings(&settings)
}

/// 익스텐션 경로 가져오기 — %APPDATA%/saba-chan/extensions 고정
/// SABA_EXTENSIONS_DIR 환경 변수가 설정되면 해당 경로를 우선 사용 (테스트/개발용)
#[allow(dead_code)]
pub fn get_extensions_path() -> anyhow::Result<String> {
    // 환경 변수 오버라이드
    if let Ok(p) = std::env::var("SABA_EXTENSIONS_DIR") {
        if !p.is_empty() {
            let path = PathBuf::from(&p);
            if !path.exists() {
                let _ = std::fs::create_dir_all(&path);
            }
            return Ok(p);
        }
    }

    let dir = get_config_dir()?.join("extensions");
    if !dir.exists() {
        std::fs::create_dir_all(&dir)?;
    }
    Ok(dir.to_string_lossy().to_string())
}

// ============ Node Token (Cloud Pairing) ============

/// 노드 토큰 로드 (데몬 API 전용)
pub fn load_node_token() -> anyhow::Result<String> {
    let data = daemon_get("/api/config/node-token")
        .ok_or_else(|| anyhow::anyhow!("데몬에 연결할 수 없습니다. 데몬이 실행 중인지 확인하세요."))?;
    Ok(data.get("token")
        .and_then(|v| v.as_str())
        .unwrap_or("")
        .to_string())
}

/// 노드 토큰 저장 (데몬 API 경유)
pub fn save_node_token(token: &str) -> anyhow::Result<()> {
    daemon_put("/api/config/node-token", &serde_json::json!({ "token": token }))
}

/// 노드 토큰 삭제 (데몬 API 경유)
pub fn clear_node_token() -> anyhow::Result<()> {
    daemon_delete("/api/config/node-token")
}

// ============ System Language ============

#[cfg(test)]
const SUPPORTED_LANGUAGES: &[&str] = saba_chan_updater_lib::constants::SUPPORTED_LANGUAGES;

fn resolve_locale(locale: &str) -> String {
    saba_chan_updater_lib::constants::resolve_locale(locale)
}

/// 시스템 언어 감지 (Electron의 app.getLocale() 대체)
pub fn get_system_language() -> String {
    #[cfg(target_os = "windows")]
    {
        use std::process::Command;
        // PowerShell (Get-Culture).Name → "ko-KR", "en-US" 등
        if let Ok(output) = Command::new("powershell")
            .args(["-NoProfile", "-Command", "(Get-Culture).Name"])
            .stdin(std::process::Stdio::null())
            .stderr(std::process::Stdio::null())
            .output()
        {
            let locale = String::from_utf8_lossy(&output.stdout).trim().to_string();
            if !locale.is_empty() {
                return resolve_locale(&locale);
            }
        }
    }
    // Fallback: 환경 변수
    for var in &["LANG", "LC_ALL", "LC_MESSAGES", "LANGUAGE"] {
        if let Ok(val) = std::env::var(var) {
            if !val.is_empty() {
                return resolve_locale(&val);
            }
        }
    }
    "en".to_string()
}

// ============ Migration (Directory Scan) ============

/// 디렉토리 스캔 — 파일 목록과 하위 디렉토리 목록 반환 (데몬 API 우선)
pub fn scan_directory(dir_path: &str) -> anyhow::Result<(Vec<String>, Vec<String>)> {
    if let Some(data) = daemon_post("/api/fs/scan-dir", &serde_json::json!({ "path": dir_path })) {
        let files = data.get("files")
            .and_then(|v| v.as_array())
            .map(|a| a.iter().filter_map(|v| v.as_str().map(|s| s.to_string())).collect())
            .unwrap_or_default();
        let dirs = data.get("dirs")
            .and_then(|v| v.as_array())
            .map(|a| a.iter().filter_map(|v| v.as_str().map(|s| s.to_string())).collect())
            .unwrap_or_default();
        return Ok((files, dirs));
    }
    // 데몬 미연결 시 로컬 폴백 (파일시스템 유틸리티)
    let path = PathBuf::from(dir_path);
    if !path.exists() {
        anyhow::bail!("Directory not found: {}", dir_path);
    }
    if !path.is_dir() {
        anyhow::bail!("Not a directory: {}", dir_path);
    }
    let entries = std::fs::read_dir(&path)?;
    let mut files = Vec::new();
    let mut dirs = Vec::new();
    for entry in entries {
        let entry = entry?;
        let name = entry.file_name().to_string_lossy().to_string();
        if entry.file_type()?.is_dir() {
            dirs.push(name);
        } else {
            files.push(name);
        }
    }
    files.sort();
    dirs.sort();
    Ok((files, dirs))
}

// ============ Module Locales ============

/// 모듈 로케일 데이터 읽기 (데몬 API 우선)
#[allow(dead_code)]
pub fn get_module_locales(module_name: &str) -> anyhow::Result<std::collections::HashMap<String, Value>> {
    if let Some(data) = daemon_get(&format!("/api/module/{}/locales", module_name)) {
        let mut result = std::collections::HashMap::new();
        if let Some(obj) = data.as_object() {
            for (lang, val) in obj {
                result.insert(lang.clone(), val.clone());
            }
        }
        return Ok(result);
    }
    // 데몬 미연결 시 로컬 폴백
    let modules_path = get_modules_path()?;
    let locales_dir = PathBuf::from(&modules_path).join(module_name).join("locales");
    let mut result = std::collections::HashMap::new();

    if !locales_dir.exists() {
        return Ok(result); // 빈 결과 (로케일 없음)
    }

    for entry in std::fs::read_dir(&locales_dir)? {
        let entry = entry?;
        let name = entry.file_name().to_string_lossy().to_string();
        if name.ends_with(".json") {
            let lang = name.trim_end_matches(".json").to_string();
            if let Ok(content) = std::fs::read_to_string(entry.path()) {
                if let Ok(parsed) = serde_json::from_str::<Value>(&content) {
                    result.insert(lang, parsed);
                }
            }
        }
    }
    Ok(result)
}

/// 주어진 JSON에서 settings 값을 추출하는 순수 함수들 (테스트 가능)
#[cfg(test)]
mod extract {
    use super::*;

    pub(crate) fn extract_auto_refresh(settings: &Value) -> bool {
        settings.get("autoRefresh").and_then(|v| v.as_bool()).unwrap_or(true)
    }

    pub(crate) fn extract_refresh_interval(settings: &Value) -> u64 {
        settings.get("refreshInterval").and_then(|v| v.as_u64()).unwrap_or(2000)
    }

    pub(crate) fn extract_ipc_port(settings: &Value) -> u16 {
        settings.get("ipcPort").and_then(|v| v.as_u64()).map(|p| p as u16).unwrap_or(constants::DEFAULT_IPC_PORT)
    }

    pub(crate) fn extract_console_buffer_size(settings: &Value) -> u64 {
        settings.get("consoleBufferSize").and_then(|v| v.as_u64()).unwrap_or(2000)
    }

    pub(crate) fn extract_auto_generate_passwords(settings: &Value) -> bool {
        settings.get("autoGeneratePasswords").and_then(|v| v.as_bool()).unwrap_or(true)
    }

    pub(crate) fn extract_port_conflict_check(settings: &Value) -> bool {
        settings.get("portConflictCheck").and_then(|v| v.as_bool()).unwrap_or(true)
    }

    pub(crate) fn extract_discord_token(settings: &Value) -> Option<String> {
        settings.get("discordToken").and_then(|v| v.as_str()).filter(|s| !s.is_empty()).map(|s| s.to_string())
    }

    pub(crate) fn extract_discord_auto_start(settings: &Value) -> bool {
        settings.get("discordAutoStart").and_then(|v| v.as_bool()).unwrap_or(false)
    }

    pub(crate) fn extract_language(settings: &Value) -> String {
        settings.get("language").and_then(|v| v.as_str()).unwrap_or("en").to_string()
    }

    pub(crate) fn extract_bot_prefix(config: &Value) -> String {
        config.get("prefix").and_then(|v| v.as_str()).unwrap_or("!saba").to_string()
    }

    pub(crate) fn extract_bot_mode(config: &Value) -> String {
        config.get("mode").and_then(|v| v.as_str()).unwrap_or("local").to_string()
    }

    pub(crate) fn extract_music_enabled(config: &Value) -> bool {
        config.get("musicEnabled").and_then(|v| v.as_bool()).unwrap_or(false)
    }
}

/// 설정 요약 출력용
#[allow(dead_code)]
pub fn config_summary() -> String {
    let token = get_discord_token().ok().flatten();
    let modules_path = get_modules_path().unwrap_or_default();
    let extensions_path = get_extensions_path().unwrap_or_default();
    let prefix = get_bot_prefix().unwrap_or_else(|_| "!saba".to_string());
    let lang = get_language().unwrap_or_else(|_| "en".to_string());
    let settings_path = constants::resolve_settings_path().display().to_string();

    let mut lines = Vec::new();
    lines.push("GUI Configuration:".to_string());
    lines.push(format!("  Settings:    {}", settings_path));
    lines.push(format!("  Token:       {}", if token.is_some() { "✓ configured" } else { "✗ not set" }));
    lines.push(format!("  Prefix:      {}", prefix));
    lines.push(format!("  Modules:     {}", modules_path));
    lines.push(format!("  Extensions:  {}", extensions_path));
    lines.push(format!("  Language:    {}", lang));
    lines.join("\n")
}

// ============ Tests ============

#[cfg(test)]
mod tests {
    use super::*;
    use super::extract::*;
    use serde_json::json;

    // ── Settings JSON extraction tests ──

    #[test]
    fn test_extract_auto_refresh_present() {
        let s = json!({"autoRefresh": false});
        assert_eq!(extract_auto_refresh(&s), false);
        let s = json!({"autoRefresh": true});
        assert_eq!(extract_auto_refresh(&s), true);
    }

    #[test]
    fn test_extract_auto_refresh_missing() {
        let s = json!({});
        assert_eq!(extract_auto_refresh(&s), true); // default true
    }

    #[test]
    fn test_extract_refresh_interval() {
        let s = json!({"refreshInterval": 5000});
        assert_eq!(extract_refresh_interval(&s), 5000);
    }

    #[test]
    fn test_extract_refresh_interval_default() {
        let s = json!({});
        assert_eq!(extract_refresh_interval(&s), 2000);
    }

    #[test]
    fn test_extract_ipc_port() {
        let s = json!({"ipcPort": 12345});
        assert_eq!(extract_ipc_port(&s), 12345);
    }

    #[test]
    fn test_extract_ipc_port_default() {
        let s = json!({});
        assert_eq!(extract_ipc_port(&s), 57474);
    }

    #[test]
    fn test_extract_console_buffer_size() {
        let s = json!({"consoleBufferSize": 500});
        assert_eq!(extract_console_buffer_size(&s), 500);
        let s = json!({});
        assert_eq!(extract_console_buffer_size(&s), 2000);
    }

    #[test]
    fn test_extract_auto_generate_passwords() {
        let s = json!({"autoGeneratePasswords": false});
        assert_eq!(extract_auto_generate_passwords(&s), false);
        let s = json!({});
        assert_eq!(extract_auto_generate_passwords(&s), true); // default true
    }

    #[test]
    fn test_extract_port_conflict_check() {
        let s = json!({"portConflictCheck": false});
        assert_eq!(extract_port_conflict_check(&s), false);
        let s = json!({"portConflictCheck": true});
        assert_eq!(extract_port_conflict_check(&s), true);
        let s = json!({});
        assert_eq!(extract_port_conflict_check(&s), true); // default true
    }

    #[test]
    fn test_extract_discord_token_present() {
        let s = json!({"discordToken": "abc123"});
        assert_eq!(extract_discord_token(&s), Some("abc123".to_string()));
    }

    #[test]
    fn test_extract_discord_token_empty() {
        let s = json!({"discordToken": ""});
        assert_eq!(extract_discord_token(&s), None);
    }

    #[test]
    fn test_extract_discord_token_missing() {
        let s = json!({});
        assert_eq!(extract_discord_token(&s), None);
    }

    #[test]
    fn test_extract_discord_auto_start() {
        let s = json!({"discordAutoStart": true});
        assert_eq!(extract_discord_auto_start(&s), true);
        let s = json!({});
        assert_eq!(extract_discord_auto_start(&s), false); // default false
    }

    #[test]
    fn test_extract_language() {
        let s = json!({"language": "ko"});
        assert_eq!(extract_language(&s), "ko");
        let s = json!({});
        assert_eq!(extract_language(&s), "en"); // default "en"
    }

    // ── Bot config extraction tests ──

    #[test]
    fn test_extract_bot_prefix() {
        let c = json!({"prefix": "사바쨩"});
        assert_eq!(extract_bot_prefix(&c), "사바쨩");
        let c = json!({});
        assert_eq!(extract_bot_prefix(&c), "!saba");
    }

    #[test]
    fn test_extract_bot_mode() {
        let c = json!({"mode": "cloud"});
        assert_eq!(extract_bot_mode(&c), "cloud");
        let c = json!({});
        assert_eq!(extract_bot_mode(&c), "local"); // default "local"
    }

    #[test]
    fn test_extract_music_enabled() {
        let c = json!({"musicEnabled": true});
        assert_eq!(extract_music_enabled(&c), true);
        let c = json!({"musicEnabled": false});
        assert_eq!(extract_music_enabled(&c), false);
        let c = json!({});
        assert_eq!(extract_music_enabled(&c), false); // default false
    }

    // ── Combined real-world data simulation ──

    #[test]
    fn test_full_settings_parsing() {
        let settings = json!({
            "autoRefresh": true,
            "refreshInterval": 1000,
            "ipcPort": 57474,
            "consoleBufferSize": 2000,
            "autoGeneratePasswords": true,
            "portConflictCheck": false,
            "discordToken": "test-token-abc",
            "discordAutoStart": true,
            "language": "ko"
        });

        assert_eq!(extract_auto_refresh(&settings), true);
        assert_eq!(extract_refresh_interval(&settings), 1000);
        assert_eq!(extract_ipc_port(&settings), 57474);
        assert_eq!(extract_console_buffer_size(&settings), 2000);
        assert_eq!(extract_auto_generate_passwords(&settings), true);
        assert_eq!(extract_port_conflict_check(&settings), false);
        assert_eq!(extract_discord_token(&settings), Some("test-token-abc".to_string()));
        assert_eq!(extract_discord_auto_start(&settings), true);
        assert_eq!(extract_language(&settings), "ko");
    }

    #[test]
    fn test_full_bot_config_parsing() {
        let config = json!({
            "prefix": "사바쨩",
            "moduleAliases": {"palworld": "팰월드"},
            "commandAliases": {},
            "musicEnabled": true,
            "mode": "local"
        });

        assert_eq!(extract_bot_prefix(&config), "사바쨩");
        assert_eq!(extract_bot_mode(&config), "local");
        assert_eq!(extract_music_enabled(&config), true);
    }

    // ── Settings mutation tests (JSON level) ──

    #[test]
    fn test_settings_mutation_auto_refresh() {
        let mut s = json!({"autoRefresh": true});
        s["autoRefresh"] = Value::Bool(false);
        assert_eq!(extract_auto_refresh(&s), false);
    }

    #[test]
    fn test_settings_mutation_refresh_interval() {
        let mut s = json!({"refreshInterval": 2000});
        s["refreshInterval"] = json!(500);
        assert_eq!(extract_refresh_interval(&s), 500);
    }

    #[test]
    fn test_settings_mutation_ipc_port() {
        let mut s = json!({"ipcPort": 57474});
        s["ipcPort"] = json!(9999);
        assert_eq!(extract_ipc_port(&s), 9999);
    }

    #[test]
    fn test_settings_mutation_console_buffer() {
        let mut s = json!({});
        s["consoleBufferSize"] = json!(4096);
        assert_eq!(extract_console_buffer_size(&s), 4096);
    }

    #[test]
    fn test_settings_mutation_auto_passwords() {
        let mut s = json!({"autoGeneratePasswords": true});
        s["autoGeneratePasswords"] = Value::Bool(false);
        assert_eq!(extract_auto_generate_passwords(&s), false);
    }

    #[test]
    fn test_settings_mutation_port_conflict() {
        let mut s = json!({"portConflictCheck": true});
        s["portConflictCheck"] = Value::Bool(false);
        assert_eq!(extract_port_conflict_check(&s), false);
    }

    #[test]
    fn test_settings_mutation_discord_auto_start() {
        let mut s = json!({});
        s["discordAutoStart"] = Value::Bool(true);
        assert_eq!(extract_discord_auto_start(&s), true);
    }

    #[test]
    fn test_settings_mutation_language() {
        let mut s = json!({"language": "en"});
        s["language"] = Value::String("ja".to_string());
        assert_eq!(extract_language(&s), "ja");
    }

    // ── File-based round-trip test ──

    #[test]
    fn test_settings_file_roundtrip() {
        // Use a temp dir as fake APPDATA
        let tmp = tempfile::tempdir().unwrap();
        let saba_dir = tmp.path().join("saba-chan");
        std::fs::create_dir_all(&saba_dir).unwrap();

        let settings_path = saba_dir.join("settings.json");
        let original = json!({
            "autoRefresh": false,
            "refreshInterval": 3000,
            "ipcPort": 8080,
            "consoleBufferSize": 1024,
            "autoGeneratePasswords": false,
            "portConflictCheck": false,
            "discordToken": "roundtrip-token",
            "discordAutoStart": true,
            "language": "ja"
        });

        // Write
        let content = serde_json::to_string_pretty(&original).unwrap();
        std::fs::write(&settings_path, &content).unwrap();

        // Read back
        let loaded: Value = serde_json::from_str(&std::fs::read_to_string(&settings_path).unwrap()).unwrap();

        assert_eq!(extract_auto_refresh(&loaded), false);
        assert_eq!(extract_refresh_interval(&loaded), 3000);
        assert_eq!(extract_ipc_port(&loaded), 8080);
        assert_eq!(extract_console_buffer_size(&loaded), 1024);
        assert_eq!(extract_auto_generate_passwords(&loaded), false);
        assert_eq!(extract_port_conflict_check(&loaded), false);
        assert_eq!(extract_discord_token(&loaded), Some("roundtrip-token".to_string()));
        assert_eq!(extract_discord_auto_start(&loaded), true);
        assert_eq!(extract_language(&loaded), "ja");

        // Mutate and re-write
        let mut modified = loaded;
        modified["autoRefresh"] = Value::Bool(true);
        modified["refreshInterval"] = json!(1500);
        modified["language"] = Value::String("ko".to_string());

        let content2 = serde_json::to_string_pretty(&modified).unwrap();
        std::fs::write(&settings_path, &content2).unwrap();

        let reloaded: Value = serde_json::from_str(&std::fs::read_to_string(&settings_path).unwrap()).unwrap();
        assert_eq!(extract_auto_refresh(&reloaded), true);
        assert_eq!(extract_refresh_interval(&reloaded), 1500);
        assert_eq!(extract_language(&reloaded), "ko");
        // Other values should remain unchanged
        assert_eq!(extract_ipc_port(&reloaded), 8080);
        assert_eq!(extract_discord_token(&reloaded), Some("roundtrip-token".to_string()));
    }

    #[test]
    fn test_bot_config_file_roundtrip() {
        let tmp = tempfile::tempdir().unwrap();
        let saba_dir = tmp.path().join("saba-chan");
        std::fs::create_dir_all(&saba_dir).unwrap();

        let bot_config_path = saba_dir.join("bot-config.json");
        let original = json!({
            "prefix": "!test",
            "moduleAliases": {"palworld": "팰월드"},
            "commandAliases": {},
            "musicEnabled": true,
            "mode": "cloud"
        });

        std::fs::write(&bot_config_path, serde_json::to_string_pretty(&original).unwrap()).unwrap();

        let loaded: Value = serde_json::from_str(&std::fs::read_to_string(&bot_config_path).unwrap()).unwrap();
        assert_eq!(extract_bot_prefix(&loaded), "!test");
        assert_eq!(extract_bot_mode(&loaded), "cloud");
        assert_eq!(extract_music_enabled(&loaded), true);

        // Mutate
        let mut modified = loaded;
        modified["prefix"] = Value::String("사바쨩".to_string());
        modified["mode"] = Value::String("local".to_string());
        modified["musicEnabled"] = Value::Bool(false);

        std::fs::write(&bot_config_path, serde_json::to_string_pretty(&modified).unwrap()).unwrap();

        let reloaded: Value = serde_json::from_str(&std::fs::read_to_string(&bot_config_path).unwrap()).unwrap();
        assert_eq!(extract_bot_prefix(&reloaded), "사바쨩");
        assert_eq!(extract_bot_mode(&reloaded), "local");
        assert_eq!(extract_music_enabled(&reloaded), false);
    }

    // ── Edge cases ──

    #[test]
    fn test_wrong_type_falls_to_default() {
        let s = json!({
            "autoRefresh": "yes",       // string, not bool
            "refreshInterval": "fast",  // string, not u64
            "ipcPort": "abc",           // string, not u64
            "consoleBufferSize": null,
            "autoGeneratePasswords": 42,
            "portConflictCheck": []
        });
        assert_eq!(extract_auto_refresh(&s), true);         // default
        assert_eq!(extract_refresh_interval(&s), 2000);     // default
        assert_eq!(extract_ipc_port(&s), 57474);            // default
        assert_eq!(extract_console_buffer_size(&s), 2000);  // default
        assert_eq!(extract_auto_generate_passwords(&s), true); // default
        assert_eq!(extract_port_conflict_check(&s), true);  // default
    }

    #[test]
    fn test_empty_object() {
        let s = json!({});
        assert_eq!(extract_auto_refresh(&s), true);
        assert_eq!(extract_refresh_interval(&s), 2000);
        assert_eq!(extract_ipc_port(&s), 57474);
        assert_eq!(extract_console_buffer_size(&s), 2000);
        assert_eq!(extract_auto_generate_passwords(&s), true);
        assert_eq!(extract_port_conflict_check(&s), true);
        assert_eq!(extract_discord_token(&s), None);
        assert_eq!(extract_discord_auto_start(&s), false);
        assert_eq!(extract_language(&s), "en");
    }

    #[test]
    fn test_load_settings_default_when_no_file() {
        // load_settings returns default JSON when file doesn't exist.
        // We can't easily test this without changing APPDATA, but we can verify
        // the default JSON structure matches our expectations.
        let default = json!({
            "autoRefresh": true,
            "refreshInterval": 2000,
            "language": "en"
        });
        assert_eq!(extract_auto_refresh(&default), true);
        assert_eq!(extract_refresh_interval(&default), 2000);
        assert_eq!(extract_language(&default), "en");
    }

    #[test]
    fn test_load_bot_config_default_when_no_file() {
        let default = json!({
            "prefix": "!saba",
            "moduleAliases": {},
            "commandAliases": {}
        });
        assert_eq!(extract_bot_prefix(&default), "!saba");
        assert_eq!(extract_bot_mode(&default), "local"); // not in default → fallback
        assert_eq!(extract_music_enabled(&default), false); // not in default → fallback
    }

    // ── Node Token tests ──

    #[test]
    fn test_node_token_roundtrip() {
        let tmp = tempfile::tempdir().unwrap();
        let saba_dir = tmp.path().join("saba-chan");
        std::fs::create_dir_all(&saba_dir).unwrap();

        let token_path = saba_dir.join(".node_token");

        // Write
        std::fs::write(&token_path, "test-node-token-123").unwrap();

        // Read
        let loaded = std::fs::read_to_string(&token_path).unwrap().trim().to_string();
        assert_eq!(loaded, "test-node-token-123");

        // Delete
        std::fs::remove_file(&token_path).unwrap();
        assert!(!token_path.exists());
    }

    // ── System Language tests ──

    #[test]
    fn test_resolve_locale_exact() {
        assert_eq!(resolve_locale("ko"), "ko");
        assert_eq!(resolve_locale("en"), "en");
        assert_eq!(resolve_locale("ja"), "ja");
        assert_eq!(resolve_locale("zh-CN"), "zh-CN");
        assert_eq!(resolve_locale("pt-BR"), "pt-BR");
    }

    #[test]
    fn test_resolve_locale_with_region() {
        assert_eq!(resolve_locale("ko-KR"), "ko");
        assert_eq!(resolve_locale("en-US"), "en");
        assert_eq!(resolve_locale("ja-JP"), "ja");
        assert_eq!(resolve_locale("de-DE"), "de");
    }

    #[test]
    fn test_resolve_locale_with_underscore() {
        assert_eq!(resolve_locale("ko_KR"), "ko");
        assert_eq!(resolve_locale("en_US"), "en");
    }

    #[test]
    fn test_resolve_locale_fallback() {
        assert_eq!(resolve_locale("xx"), "en");
        assert_eq!(resolve_locale(""), "en");
    }

    #[test]
    fn test_get_system_language_returns_something() {
        // Just verify it doesn't panic and returns a valid value
        let lang = get_system_language();
        assert!(SUPPORTED_LANGUAGES.contains(&lang.as_str()));
    }

    // ── Scan Directory tests ──

    #[test]
    fn test_scan_directory() {
        let tmp = tempfile::tempdir().unwrap();
        // Create some test files and dirs
        std::fs::create_dir_all(tmp.path().join("subdir1")).unwrap();
        std::fs::create_dir_all(tmp.path().join("subdir2")).unwrap();
        std::fs::write(tmp.path().join("file1.txt"), "content").unwrap();
        std::fs::write(tmp.path().join("file2.json"), "{}").unwrap();

        let (files, dirs) = scan_directory(&tmp.path().to_string_lossy()).unwrap();
        assert_eq!(dirs, vec!["subdir1", "subdir2"]);
        assert_eq!(files, vec!["file1.txt", "file2.json"]);
    }

    #[test]
    fn test_scan_directory_empty() {
        let tmp = tempfile::tempdir().unwrap();
        let (files, dirs) = scan_directory(&tmp.path().to_string_lossy()).unwrap();
        assert!(files.is_empty());
        assert!(dirs.is_empty());
    }

    #[test]
    fn test_scan_directory_not_found() {
        let result = scan_directory("/nonexistent/path/12345");
        assert!(result.is_err());
    }

    // ── Module Locales tests ──

    #[test]
    fn test_get_module_locales_with_locales() {
        let tmp = tempfile::tempdir().unwrap();
        let locales_dir = tmp.path().join("testmod").join("locales");
        std::fs::create_dir_all(&locales_dir).unwrap();
        std::fs::write(locales_dir.join("en.json"), r#"{"hello": "Hello"}"#).unwrap();
        std::fs::write(locales_dir.join("ko.json"), r#"{"hello": "안녕"}"#).unwrap();

        // Can't test get_module_locales directly because it uses get_modules_path()
        // which reads settings.json, but we can test the file reading logic
        let mut result = std::collections::HashMap::new();
        for entry in std::fs::read_dir(&locales_dir).unwrap() {
            let entry = entry.unwrap();
            let name = entry.file_name().to_string_lossy().to_string();
            if name.ends_with(".json") {
                let lang = name.trim_end_matches(".json").to_string();
                let content = std::fs::read_to_string(entry.path()).unwrap();
                let parsed: Value = serde_json::from_str(&content).unwrap();
                result.insert(lang, parsed);
            }
        }

        assert_eq!(result.len(), 2);
        assert!(result.contains_key("en"));
        assert!(result.contains_key("ko"));
        assert_eq!(result["en"]["hello"].as_str().unwrap(), "Hello");
        assert_eq!(result["ko"]["hello"].as_str().unwrap(), "안녕");
    }
}
