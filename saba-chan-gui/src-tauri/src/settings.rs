//! GUI 설정 — <data>/config.json의 `gui` 네임스페이스 (데몬 config_file과 같은 파일)
//!
//! 데몬이 떠 있으면 데몬 API로만 쓴다. 여기서 파일을 직접 다루는 건 읽기와
//! 데몬이 응답하지 않을 때(부트스트랩) 쓰기뿐이다.

use crate::i18n::strip_bom;
use saba_chan_updater_lib::constants::{DEFAULT_IPC_PORT, SUPPORTED_LANGUAGES};
use serde_json::{json, Map, Value};
use std::path::{Path, PathBuf};

pub fn config_path(data_dir: &Path) -> PathBuf {
    data_dir.join("config.json")
}

/// JSON 파일 (UTF-8 BOM 허용). 없으면 Ok(None)
pub fn read_json_file(path: &Path) -> Result<Option<Value>, String> {
    if !path.exists() {
        return Ok(None);
    }
    let text = std::fs::read_to_string(path).map_err(|e| e.to_string())?;
    serde_json::from_str(strip_bom(&text))
        .map(Some)
        .map_err(|e| e.to_string())
}

/// config.json 전체 (깨졌으면 .bak, 없으면 빈 객체)
pub fn read_config_document(data_dir: &Path) -> Map<String, Value> {
    let path = config_path(data_dir);
    let bak = with_suffix(&path, ".bak");
    for candidate in [&path, &bak] {
        match read_json_file(candidate) {
            Ok(Some(Value::Object(doc))) => return doc,
            Ok(_) => {}
            Err(e) => tracing::error!("Failed to read {}: {}", candidate.display(), e),
        }
    }
    Map::new()
}

fn with_suffix(path: &Path, suffix: &str) -> PathBuf {
    let mut s = path.as_os_str().to_os_string();
    s.push(suffix);
    PathBuf::from(s)
}

/// 데몬 기동 전 GUI 설정 저장 — 다른 네임스페이스는 보존, tmp → rename, 이전 파일은 .bak
pub fn write_gui_section(data_dir: &Path, gui: &Value) -> std::io::Result<()> {
    let path = config_path(data_dir);
    std::fs::create_dir_all(data_dir)?;
    let mut doc = read_config_document(data_dir);
    doc.insert("gui".into(), gui.clone());
    if doc.get("schemaVersion").is_none_or(|v| v.is_null()) {
        doc.insert("schemaVersion".into(), json!(1));
    }
    let tmp = with_suffix(&path, ".tmp");
    std::fs::write(&tmp, serde_json::to_string_pretty(&Value::Object(doc))?)?;
    if path.exists() {
        std::fs::copy(&path, with_suffix(&path, ".bak"))?;
    }
    std::fs::rename(&tmp, &path)
}

pub fn default_settings() -> Map<String, Value> {
    let v = json!({
        "autoRefresh": true,
        "refreshInterval": 2000,
        "windowBounds": { "width": 1200, "height": 840 },
        "language": system_language(),
        "ipcPort": DEFAULT_IPC_PORT,
        "consoleBufferSize": 2000,
        "autoGeneratePasswords": true,
        "portConflictCheck": true,
    });
    match v {
        Value::Object(m) => m,
        _ => unreachable!(),
    }
}

/// `gui` 섹션 + 기본값 (파일에 없는 칸만 채운다). 아직 이관 전이면 예전 settings.json
pub fn load_settings(data_dir: &Path) -> Value {
    let mut gui = read_config_document(data_dir).remove("gui").filter(|v| !v.is_null());
    if gui.is_none() {
        match read_json_file(&data_dir.join("settings.json")) {
            Ok(v) => gui = v,
            Err(e) => tracing::error!("Failed to load legacy settings.json: {}", e),
        }
    }
    let mut out = default_settings();
    if let Some(Value::Object(m)) = gui {
        out.extend(m);
    }
    Value::Object(out)
}

/// 얕은 병합 ({...base, ...overlay})
pub fn merge(base: Value, overlay: &Value) -> Value {
    let mut out = match base {
        Value::Object(m) => m,
        _ => Map::new(),
    };
    if let Value::Object(m) = overlay {
        out.extend(m.clone());
    }
    Value::Object(out)
}

pub fn ipc_port(settings: &Value) -> u16 {
    let port = &settings["ipcPort"];
    port.as_u64()
        .or_else(|| port.as_str().and_then(|s| s.trim().parse().ok()))
        .and_then(|p| u16::try_from(p).ok())
        .filter(|p| *p != 0)
        .unwrap_or(DEFAULT_IPC_PORT)
}

/// 데몬 주소: 환경변수 IPC_BASE가 있으면 그것, 없으면 설정 포트
pub fn ipc_base(settings: &Value) -> String {
    match std::env::var("IPC_BASE") {
        Ok(b) if !b.is_empty() => b,
        _ => format!("http://127.0.0.1:{}", ipc_port(settings)),
    }
}

pub fn language(settings: &Value) -> String {
    match settings["language"].as_str() {
        Some(l) if !l.is_empty() => l.to_string(),
        _ => system_language(),
    }
}

pub fn system_language() -> String {
    match_language(&sys_locale::get_locale().unwrap_or_else(|| "en".into()))
}

/// OS 로케일 → 지원 언어 (정확히 같으면 그것, 아니면 언어 코드로, 없으면 en)
pub fn match_language(locale: &str) -> String {
    let locale = locale.replace('_', "-");
    if let Some(l) = SUPPORTED_LANGUAGES.iter().find(|l| **l == locale) {
        return l.to_string();
    }
    let base = locale.split('-').next().unwrap_or("");
    // 데몬과 같은 규칙: 스크립트 태그가 지역보다 우선 (zh-Hans-HK → 간체, zh-HK·zh-MO·zh-Hant → 번체)
    if base.eq_ignore_ascii_case("zh") {
        let subtags: Vec<String> = locale.split('-').skip(1).map(|t| t.to_ascii_lowercase()).collect();
        let has = |t: &str| subtags.iter().any(|s| s == t);
        return if has("hans") {
            "zh-CN".into()
        } else if ["hant", "tw", "hk", "mo"].into_iter().any(has) {
            "zh-TW".into()
        } else {
            "zh-CN".into()
        };
    }
    SUPPORTED_LANGUAGES
        .iter()
        .find(|l| **l == base || l.starts_with(&format!("{base}-")))
        .map(|l| l.to_string())
        .unwrap_or_else(|| "en".into())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn local_save_preserves_other_namespaces_and_keeps_backup() {
        let dir = tempfile::tempdir().unwrap();
        let path = config_path(dir.path());
        // 데몬이 쓴 BOM 붙은 파일
        std::fs::write(
            &path,
            "\u{feff}{\"schemaVersion\":3,\"daemon\":{\"x\":1},\"gui\":{\"ipcPort\":1}}",
        )
        .unwrap();

        write_gui_section(dir.path(), &json!({"ipcPort": 60000, "language": "ko"})).unwrap();

        let doc = read_json_file(&path).unwrap().unwrap();
        assert_eq!(doc["daemon"]["x"], 1);
        assert_eq!(doc["schemaVersion"], 3);
        assert_eq!(doc["gui"]["ipcPort"], 60000);
        let bak = read_json_file(&with_suffix(&path, ".bak")).unwrap().unwrap();
        assert_eq!(bak["gui"]["ipcPort"], 1);
        assert!(!with_suffix(&path, ".tmp").exists());

        let s = load_settings(dir.path());
        assert_eq!(ipc_port(&s), 60000);
        assert_eq!(s["language"], "ko");
        // 기본값은 없는 칸만 채운다
        assert_eq!(s["consoleBufferSize"], 2000);
    }

    #[test]
    fn broken_config_falls_back_to_bak_then_legacy() {
        let dir = tempfile::tempdir().unwrap();
        let path = config_path(dir.path());
        std::fs::write(&path, "{broken").unwrap();
        std::fs::write(with_suffix(&path, ".bak"), "{\"gui\":{\"ipcPort\":50001}}").unwrap();
        assert_eq!(ipc_port(&load_settings(dir.path())), 50001);

        let dir = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join("settings.json"), "{\"ipcPort\":50002}").unwrap();
        assert_eq!(ipc_port(&load_settings(dir.path())), 50002);
        // 새 파일은 schemaVersion 1로 만든다
        write_gui_section(dir.path(), &json!({})).unwrap();
        assert_eq!(
            read_json_file(&config_path(dir.path())).unwrap().unwrap()["schemaVersion"],
            1
        );
    }

    #[test]
    fn matches_system_locale() {
        assert_eq!(match_language("ko-KR"), "ko");
        assert_eq!(match_language("zh-TW"), "zh-TW");
        assert_eq!(match_language("zh-HK"), "zh-TW");
        assert_eq!(match_language("zh_MO"), "zh-TW");
        assert_eq!(match_language("zh-Hant-SG"), "zh-TW");
        assert_eq!(match_language("zh-Hans-HK"), "zh-CN");
        assert_eq!(match_language("zh-SG"), "zh-CN");
        assert_eq!(match_language("zh"), "zh-CN");
        assert_eq!(match_language("pt_PT"), "pt-BR");
        assert_eq!(match_language("nl-NL"), "en");
    }
}
