//! 데몬 API를 통한 GUI 설정(`settings.json`) 읽기/쓰기와 경로 해석.
//!
//! 원칙: 설정 읽기/쓰기는 데몬 API(`GET/PUT /api/config/gui`)를 경유한다.
//! 로컬 파일 직접 접근은 IPC 포트 부트스트랩([`crate::ipc::ipc_port`])에만 허용한다.

use std::path::PathBuf;

use reqwest::Method;
use saba_chan_updater_lib::constants;
use serde_json::Value;

use crate::ipc::blocking_request;

const OFFLINE_MSG: &str = "데몬에 연결할 수 없습니다. 데몬이 실행 중인지 확인하세요.";

// ============ GUI 설정 (settings.json) ============

/// GUI 설정 전체 로드 (blocking).
pub fn load_settings() -> anyhow::Result<Value> {
    blocking_request(Method::GET, "/api/config/gui", None)
        .map_err(|e| anyhow::anyhow!("{} ({})", OFFLINE_MSG, e))
}

/// GUI 설정 하나를 바꿔 저장한다 (load → set → save, blocking).
pub fn set_gui_setting(key: &str, value: Value) -> anyhow::Result<()> {
    let mut settings = load_settings()?;
    settings[key] = value;
    blocking_request(Method::PUT, "/api/config/gui", Some(&settings)).map(|_| ())
}

/// 설정 스냅샷에서 bool 값을 읽는다 (없거나 타입이 다르면 기본값).
pub fn setting_bool(settings: Option<&Value>, key: &str, default: bool) -> bool {
    settings.and_then(|s| s.get(key)).and_then(|v| v.as_bool()).unwrap_or(default)
}

/// 설정 스냅샷에서 u64 값을 읽는다 (없거나 타입이 다르면 기본값).
pub fn setting_u64(settings: Option<&Value>, key: &str, default: u64) -> u64 {
    settings.and_then(|s| s.get(key)).and_then(|v| v.as_u64()).unwrap_or(default)
}

/// 설정 스냅샷에서 GUI 언어를 읽는다. 비어 있거나 "auto"면 `None`.
pub fn setting_language(settings: Option<&Value>) -> Option<String> {
    settings
        .and_then(|s| s.get("language"))
        .and_then(|v| v.as_str())
        .filter(|s| !s.is_empty() && *s != "auto")
        .map(|s| s.to_string())
}

// ============ 경로 ============

/// 모듈 경로 — `SABA_MODULES_PATH`가 있으면 우선, 없으면 `<data>/modules`.
pub fn get_modules_path() -> anyhow::Result<String> {
    path_with_override("SABA_MODULES_PATH", "modules")
}

/// 익스텐션 경로 — `SABA_EXTENSIONS_DIR`가 있으면 우선, 없으면 `<data>/extensions`.
pub fn get_extensions_path() -> anyhow::Result<String> {
    path_with_override("SABA_EXTENSIONS_DIR", "extensions")
}

/// 인스턴스 디렉토리 경로 (`<data>/instances`).
pub fn get_instances_path() -> PathBuf {
    constants::resolve_data_dir().join("instances")
}

fn path_with_override(env_key: &str, sub_dir: &str) -> anyhow::Result<String> {
    let dir = match std::env::var(env_key) {
        Ok(p) if !p.is_empty() => PathBuf::from(p),
        _ => constants::resolve_data_dir().join(sub_dir),
    };
    std::fs::create_dir_all(&dir)?;
    Ok(dir.to_string_lossy().to_string())
}

// ============ 시스템 언어 ============

/// 시스템 언어 감지 (Windows: `Get-Culture`, 그 외: 로케일 환경 변수).
pub fn get_system_language() -> String {
    #[cfg(target_os = "windows")]
    {
        use std::process::{Command, Stdio};
        if let Ok(output) = Command::new("powershell")
            .args(["-NoProfile", "-Command", "(Get-Culture).Name"])
            .stdin(Stdio::null())
            .stderr(Stdio::null())
            .output()
        {
            let locale = String::from_utf8_lossy(&output.stdout).trim().to_string();
            if !locale.is_empty() {
                return constants::resolve_locale(&locale);
            }
        }
    }
    for var in ["LANG", "LC_ALL", "LC_MESSAGES", "LANGUAGE"] {
        if let Ok(val) = std::env::var(var) {
            if !val.is_empty() {
                return constants::resolve_locale(&val);
            }
        }
    }
    "en".to_string()
}

// ============ 디렉토리 스캔 (migration) ============

/// 디렉토리 스캔 — (파일 목록, 하위 디렉토리 목록). 데몬 API 우선, 실패 시 로컬.
pub fn scan_directory(dir_path: &str) -> anyhow::Result<(Vec<String>, Vec<String>)> {
    let body = serde_json::json!({ "path": dir_path });
    if let Ok(data) = blocking_request(Method::POST, "/api/fs/scan-dir", Some(&body)) {
        return Ok((string_list(&data, "files"), string_list(&data, "dirs")));
    }
    scan_directory_local(dir_path)
}

fn string_list(data: &Value, key: &str) -> Vec<String> {
    data.get(key)
        .and_then(|v| v.as_array())
        .map(|a| a.iter().filter_map(|v| v.as_str().map(|s| s.to_string())).collect())
        .unwrap_or_default()
}

fn scan_directory_local(dir_path: &str) -> anyhow::Result<(Vec<String>, Vec<String>)> {
    let path = PathBuf::from(dir_path);
    if !path.is_dir() {
        anyhow::bail!("Directory not found: {}", dir_path);
    }
    let mut files = Vec::new();
    let mut dirs = Vec::new();
    for entry in std::fs::read_dir(&path)? {
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

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn setting_readers_fall_back_on_missing_or_wrong_type() {
        let s = json!({ "autoRefresh": "yes", "consoleBufferSize": 500, "portConflictCheck": false });
        assert!(setting_bool(Some(&s), "autoRefresh", true));
        assert!(!setting_bool(Some(&s), "portConflictCheck", true));
        assert_eq!(setting_u64(Some(&s), "consoleBufferSize", 2000), 500);
        assert_eq!(setting_u64(Some(&s), "missing", 2000), 2000);
        assert_eq!(setting_u64(None, "consoleBufferSize", 7), 7);
    }

    #[test]
    fn setting_language_ignores_auto_and_empty() {
        assert_eq!(setting_language(Some(&json!({"language": "ko"}))), Some("ko".into()));
        assert_eq!(setting_language(Some(&json!({"language": "auto"}))), None);
        assert_eq!(setting_language(Some(&json!({"language": ""}))), None);
        assert_eq!(setting_language(None), None);
    }

    #[test]
    fn scan_directory_local_lists_sorted_files_and_dirs() {
        let tmp = tempfile::tempdir().unwrap();
        std::fs::create_dir_all(tmp.path().join("subdir2")).unwrap();
        std::fs::create_dir_all(tmp.path().join("subdir1")).unwrap();
        std::fs::write(tmp.path().join("file2.json"), "{}").unwrap();
        std::fs::write(tmp.path().join("file1.txt"), "x").unwrap();

        let (files, dirs) = scan_directory_local(&tmp.path().to_string_lossy()).unwrap();
        assert_eq!(dirs, vec!["subdir1", "subdir2"]);
        assert_eq!(files, vec!["file1.txt", "file2.json"]);
    }

    #[test]
    fn scan_directory_local_missing_dir_is_error() {
        assert!(scan_directory_local("/nonexistent/path/12345").is_err());
    }
}
