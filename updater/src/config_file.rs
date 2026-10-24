//! 사바쨩 설정 파일 하나 — `<data_dir>/config.json`
//!
//! 모든 사용자 설정이 이 파일 하나에 네임스페이스(최상위 키)별로 들어간다.
//!
//! | 키 | 주인 |
//! |---|---|
//! | `schemaVersion` | 이 모듈 |
//! | `gui` | GUI 설정 (데몬 `/api/config/gui`) |
//! | `relay` | 릴레이·디스코드 연동 (데몬 `/api/config/bot`) |
//! | `updates` | 업데이트 설정 (데몬 `/api/updates/config`) |
//! | `boot` | 부트 선택 기억 |
//! | `extensions` | `{ enabled: [id], settings: { id: { key: value } } }` |
//! | `paths` | 사용자가 고른 모듈·익스텐션 폴더 (`component_dirs`, 데몬 `/api/system/paths`) |
//! | `node` | 이 PC의 릴레이(디스코드) 노드 토큰 `{ token }` (데몬 `/api/config/node-token`, 내보내기에서 뺀다) |
//! | `cli` 등 기타 | 클라이언트별 자유 영역 (데몬 `/api/config/client/:kind`) |
//!
//! 새 기능은 자기 키 하나만 추가하면 된다(슬롯). 이 모듈은 키 안을 해석하지 않는다.
//!
//! ## 저장·로드 보장
//! - 저장: 임시 파일 → fsync → 직전 파일을 `config.json.bak`으로 → rename.
//! - 로드: `config.json`이 깨졌으면 `.bak`에서 복구하고, 깨진 파일은
//!   `config.json.corrupt-<시각>`으로 옮겨 둔다(덮어쓰지 않는다).
//! - 이관: 예전 개별 파일(`settings.json` 등)이 있고 해당 키가 아직 없으면 읽어 넣고,
//!   예전 파일은 그대로 두고 `.migrated` 사본을 남긴다. 예전 판(다운그레이드·다른 설치본)이 계속 읽을 수 있고,
//!   사본이 있는 파일은 다시 이관하지 않는다. 예전 노드 토큰 파일(`.node_token`)만은 비밀값이라
//!   `node.token`에 넣은 뒤 사본 없이 지운다. 이 판은 그 파일을 쓰지 않으므로, 다시 생긴 파일(예전 판이
//!   새로 페어링함)은 `node.token`을 덮어쓴다.
//!
//! 같은 프로세스 안의 쓰기는 전역 잠금으로 직렬화한다. 데몬이 떠 있는 동안 다른
//! 프로세스(GUI·CLI·설치기)는 데몬 API로만 쓴다. 데몬이 없을 때의 부트스트랩
//! 쓰기만 파일에 직접 한다.

use serde_json::{Map, Value};
use std::path::{Path, PathBuf};
use std::sync::Mutex;

pub const FILE_NAME: &str = "config.json";
pub const SCHEMA_VERSION: u64 = 1;

/// 예전 개별 파일 → 들어갈 위치. `extensions`는 두 파일이 한 키를 나눠 쓴다.
const LEGACY_FILES: &[(&str, &[&str])] = &[
    ("settings.json", &["gui"]),
    ("bot-config.json", &["relay"]),
    ("cli-settings.json", &["cli"]),
    ("update-config.json", &["updates"]),
    ("boot-config.json", &["boot"]),
    ("extensions_state.json", &["extensions", "enabled"]),
    ("extensionConfig.json", &["extensions", "settings"]),
];

/// 예전 노드 토큰 파일 (평문 한 줄) → `node.token`
const LEGACY_NODE_TOKEN: &str = ".node_token";

static WRITE_LOCK: Mutex<()> = Mutex::new(());

#[derive(Debug, Clone)]
pub struct ConfigFile {
    dir: PathBuf,
}

impl ConfigFile {
    /// 사용자 데이터 폴더의 설정 파일 (`SABA_DATA_DIR` 반영)
    pub fn default_location() -> Self {
        Self::at(&crate::constants::resolve_data_dir())
    }

    /// 지정한 폴더의 설정 파일 (테스트·격리용)
    pub fn at(dir: &Path) -> Self {
        Self { dir: dir.to_path_buf() }
    }

    pub fn path(&self) -> PathBuf {
        self.dir.join(FILE_NAME)
    }

    fn backup_path(&self) -> PathBuf {
        self.dir.join(format!("{}.bak", FILE_NAME))
    }

    /// 문서 전체. 실패하지 않는다(최악이면 빈 문서). 이관할 예전 파일이 있으면 저장까지 한다.
    pub fn load(&self) -> Map<String, Value> {
        let _guard = WRITE_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        self.load_locked()
    }

    /// 한 네임스페이스. 없으면 `Value::Null`.
    pub fn section(&self, key: &str) -> Value {
        self.load().remove(key).unwrap_or(Value::Null)
    }

    /// 한 네임스페이스를 읽기만 한다: 잠그지 않고 이관·복구·저장도 하지 않는다 (경로 해석처럼 자주, 어디서나 부르는 곳).
    /// `config.json`을 못 읽으면 `.bak`을 본다. 둘 다 없거나 깨졌으면 `Value::Null`
    pub fn peek_section(&self, key: &str) -> Value {
        [self.path(), self.backup_path()]
            .iter()
            .find_map(|p| read_object(p).ok().flatten())
            .and_then(|mut doc| doc.remove(key))
            .unwrap_or(Value::Null)
    }

    /// 읽고 → 고치고 → 저장을 한 번에. 고친 뒤의 문서를 돌려준다.
    pub fn update<F>(&self, edit: F) -> std::io::Result<Map<String, Value>>
    where
        F: FnOnce(&mut Map<String, Value>),
    {
        let _guard = WRITE_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        let mut doc = self.load_locked();
        edit(&mut doc);
        self.save_locked(&mut doc)?;
        Ok(doc)
    }

    /// 한 네임스페이스를 통째로 바꾼다.
    pub fn set_section(&self, key: &str, value: Value) -> std::io::Result<()> {
        self.update(|doc| {
            doc.insert(key.to_string(), value);
        })
        .map(|_| ())
    }

    fn load_locked(&self) -> Map<String, Value> {
        let path = self.path();
        let mut doc = match read_object(&path) {
            Ok(Some(doc)) => doc,
            Ok(None) => Map::new(),
            Err(reason) => {
                let corrupt = self.dir.join(format!(
                    "{}.corrupt-{}",
                    FILE_NAME,
                    std::time::SystemTime::now()
                        .duration_since(std::time::UNIX_EPOCH)
                        .map(|d| d.as_secs())
                        .unwrap_or(0)
                ));
                tracing::error!(
                    "[Config] {} is unreadable ({}); moved to {} and restoring from backup",
                    path.display(),
                    reason,
                    corrupt.display()
                );
                let _ = std::fs::rename(&path, &corrupt);
                match read_object(&self.backup_path()) {
                    Ok(Some(doc)) => {
                        let mut restored = doc;
                        let _ = self.save_locked(&mut restored);
                        restored
                    }
                    _ => Map::new(),
                }
            }
        };

        let token = self.migrate_node_token(&mut doc);
        let migrated = self.migrate_legacy(&mut doc);
        if migrated || token {
            if let Err(e) = self.save_locked(&mut doc) {
                tracing::error!("[Config] Failed to save migrated settings: {}", e);
                return doc;
            }
            if migrated {
                for (name, _) in LEGACY_FILES {
                    let legacy = self.dir.join(name);
                    let marker = self.dir.join(format!("{}.migrated", name));
                    if legacy.is_file() && !marker.exists() {
                        let _ = std::fs::copy(&legacy, &marker);
                    }
                }
            }
            if token {
                if let Err(e) = std::fs::remove_file(self.dir.join(LEGACY_NODE_TOKEN)) {
                    tracing::warn!("[Config] Failed to remove the old node token file: {}", e);
                }
            }
        }
        doc
    }

    /// 예전 노드 토큰 파일을 읽어 `node.token`에 넣는다. 읽었으면 true (저장 뒤 파일을 지운다).
    /// 이 판은 그 파일을 쓰지 않으므로, 옮긴 뒤 다시 생긴 파일은 예전 판이 새로 페어링한 토큰이다: 파일 값이 이긴다
    /// (빈 파일은 지우기만 한다)
    fn migrate_node_token(&self, doc: &mut Map<String, Value>) -> bool {
        let Ok(text) = std::fs::read_to_string(self.dir.join(LEGACY_NODE_TOKEN)) else {
            return false;
        };
        let token = text.trim();
        let node = doc.entry("node").or_insert_with(|| Value::Object(Map::new()));
        if !node.is_object() {
            *node = Value::Object(Map::new());
        }
        let node = node.as_object_mut().expect("just made an object");
        if !token.is_empty() && node.get("token").and_then(Value::as_str) != Some(token) {
            tracing::info!("[Config] Migrating the node token into config.json");
            node.insert("token".into(), Value::from(token));
        }
        true
    }

    /// 예전 파일을 읽어 비어 있는 키에 넣는다. 하나라도 있으면 true.
    fn migrate_legacy(&self, doc: &mut Map<String, Value>) -> bool {
        let mut found = false;
        for (name, target) in LEGACY_FILES {
            let legacy = self.dir.join(name);
            // `.migrated` 사본이 있으면 이미 이관했다: 그 뒤 config.json에서 지운 값을 되살리지 않는다
            if !legacy.is_file() || self.dir.join(format!("{}.migrated", name)).exists() {
                continue;
            }
            found = true;
            let Ok(Some(value)) = read_value(&legacy) else {
                tracing::warn!("[Config] Skipping unreadable legacy {}", legacy.display());
                continue;
            };
            let slot = match target {
                [key] => doc.entry(key.to_string()),
                [outer, inner] => {
                    let outer = doc
                        .entry(outer.to_string())
                        .or_insert_with(|| Value::Object(Map::new()));
                    if !outer.is_object() {
                        *outer = Value::Object(Map::new());
                    }
                    outer.as_object_mut().expect("just made an object").entry(inner.to_string())
                }
                _ => unreachable!(),
            };
            if let serde_json::map::Entry::Vacant(v) = slot {
                tracing::info!("[Config] Migrating {} into config.json", name);
                v.insert(value);
            }
        }
        found
    }

    fn save_locked(&self, doc: &mut Map<String, Value>) -> std::io::Result<()> {
        doc.insert("schemaVersion".into(), Value::from(SCHEMA_VERSION));
        let path = self.path();
        let text = serde_json::to_string_pretty(doc).map_err(std::io::Error::other)?;
        if path.is_file() {
            // 직전 정상본을 .bak으로 (rename이 아니라 copy: 교체 순간에도 config.json이 있게)
            let _ = std::fs::copy(&path, self.backup_path());
        }
        // 챗봇 API 키 같은 비밀값이 들어 있다
        saba_atomic_file::write(&path, text.as_bytes(), saba_atomic_file::Mode::Secret)
    }
}

fn read_value(path: &Path) -> Result<Option<Value>, String> {
    let text = match std::fs::read_to_string(path) {
        Ok(t) => t,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(None),
        Err(e) => return Err(e.to_string()),
    };
    let text = text.trim_start_matches('\u{feff}');
    serde_json::from_str(text).map(Some).map_err(|e| e.to_string())
}

fn read_object(path: &Path) -> Result<Option<Map<String, Value>>, String> {
    match read_value(path)? {
        None => Ok(None),
        Some(Value::Object(map)) => Ok(Some(map)),
        Some(_) => Err("not a JSON object".into()),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn file() -> (tempfile::TempDir, ConfigFile) {
        let dir = tempfile::tempdir().unwrap();
        let cfg = ConfigFile::at(dir.path());
        (dir, cfg)
    }

    #[test]
    fn sections_survive_reload_and_other_sections_are_kept() {
        let (_d, cfg) = file();
        cfg.set_section("gui", json!({ "language": "ko" })).unwrap();
        cfg.set_section("cli", json!({ "autoStart": false })).unwrap();
        let again = ConfigFile::at(cfg.dir.as_path());
        assert_eq!(again.section("gui")["language"], "ko");
        assert_eq!(again.section("cli")["autoStart"], false);
        assert_eq!(again.load()["schemaVersion"], SCHEMA_VERSION);
    }

    #[test]
    fn corrupt_file_is_restored_from_backup_and_kept_aside() {
        let (d, cfg) = file();
        cfg.set_section("gui", json!({ "language": "ko" })).unwrap();
        cfg.set_section("gui", json!({ "language": "ja" })).unwrap();
        std::fs::write(cfg.path(), "{ broken").unwrap();

        // .bak은 직전 저장본(ko)
        assert_eq!(cfg.section("gui")["language"], "ko");
        let corrupt = std::fs::read_dir(d.path())
            .unwrap()
            .flatten()
            .any(|e| e.file_name().to_string_lossy().starts_with("config.json.corrupt-"));
        assert!(corrupt, "broken file must be kept, not overwritten");
    }

    #[test]
    fn legacy_files_are_migrated_once() {
        let (d, cfg) = file();
        std::fs::write(d.path().join("settings.json"), "\u{feff}{\"language\":\"de\",\"ipcPort\":1234}").unwrap();
        std::fs::write(d.path().join("bot-config.json"), r#"{"prefix":"!s"}"#).unwrap();
        std::fs::write(d.path().join("extensions_state.json"), r#"["docker"]"#).unwrap();
        std::fs::write(d.path().join("extensionConfig.json"), r#"{"docker":{"mem":"2g"}}"#).unwrap();

        let doc = cfg.load();
        assert_eq!(doc["gui"]["ipcPort"], 1234);
        assert_eq!(doc["relay"]["prefix"], "!s");
        assert_eq!(doc["extensions"]["enabled"], json!(["docker"]));
        assert_eq!(doc["extensions"]["settings"]["docker"]["mem"], "2g");
        assert!(d.path().join("settings.json").exists(), "older versions still read the original");
        assert!(d.path().join("settings.json.migrated").exists());
        assert!(cfg.path().exists());

        // 이관한 뒤 지운 값은 남은 예전 파일에서 되살아나지 않는다
        cfg.set_section("relay", json!({})).unwrap();
        assert!(cfg.load()["relay"].get("prefix").is_none());
    }

    #[test]
    fn the_old_node_token_file_moves_into_the_node_section_and_is_removed() {
        let (d, cfg) = file();
        std::fs::write(d.path().join(".node_token"), "sbn_a.secret\n").unwrap();

        assert_eq!(cfg.section("node")["token"], "sbn_a.secret");
        assert!(!d.path().join(".node_token").exists(), "the token must live in one place");
        assert!(!d.path().join(".node_token.migrated").exists(), "no second copy of a secret");

        // 옮긴 뒤 이 판은 그 파일을 쓰지 않는다: 다시 생겼으면 예전 판(0.1.8)이 새로 페어링한 토큰이라 그것을 쓴다
        std::fs::write(d.path().join(".node_token"), "sbn_b.newer").unwrap();
        assert_eq!(cfg.section("node")["token"], "sbn_b.newer");
        assert!(!d.path().join(".node_token").exists());
        // 빈 파일은 지우기만 한다
        std::fs::write(d.path().join(".node_token"), "").unwrap();
        assert_eq!(cfg.section("node")["token"], "sbn_b.newer");
        assert!(!d.path().join(".node_token").exists());
    }

    #[test]
    fn existing_section_wins_over_stale_legacy_file() {
        let (d, cfg) = file();
        cfg.set_section("gui", json!({ "language": "ko" })).unwrap();
        std::fs::write(d.path().join("settings.json"), r#"{"language":"en"}"#).unwrap();
        assert_eq!(cfg.section("gui")["language"], "ko");
        assert!(d.path().join("settings.json.migrated").exists());
    }
}
