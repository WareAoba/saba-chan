//! 무결성 검증 — 업데이터가 설치할 때 기록한 파일별 SHA-256과 지금 디스크의 파일을 비교한다.
//!
//! 릴리즈 manifest의 `sha256`은 zip의 해시라 풀린 파일과 비교할 수 없다. 그래서 적용할 때
//! zip에서 푼 파일마다 해시를 `<데이터 폴더>/installed-files.json`에 적어 두고(`record_install`),
//! 검사(`verify_installed`)는 그 기록만 본다. 네트워크를 쓰지 않는다.
//! 업데이터가 설치하지 않은 컴포넌트(설치기로 처음 깐 것)는 기록이 없어 결과에 나오지 않는다.
//!
//! 신뢰 출처(M12): 서명된 릴리즈 카탈로그에서 받은 컴포넌트 키는 `<데이터 폴더>/official-components.json`에
//! 따로 적는다(`set_official`). `trust_of`는 "카탈로그에서 받았고 그 뒤로 코드 파일이 그대로인가"를 답한다.
//! 이것은 설치 출처 표시다. 기록 파일은 서명되지 않으므로, 이미 실행된 코드가 기록을 고쳤다면 막지 못한다.

use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::collections::{BTreeMap, BTreeSet};
use std::io::Read;
use std::path::{Path, PathBuf};

/// 컴포넌트 키 → (파일 절대 경로 → sha256)
pub type InstalledFiles = BTreeMap<String, BTreeMap<String, String>>;

// ══════════════════════════════════════════════════════
// 검증 결과 구조체
// ══════════════════════════════════════════════════════

/// 개별 컴포넌트의 무결성 검증 결과
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ComponentIntegrity {
    /// 컴포넌트 키 (예: "saba-core", "module-minecraft", "ext-steamcmd")
    pub component: String,
    /// 사용자 표시용 이름
    pub display_name: String,
    /// 검증 상태
    pub status: IntegrityStatus,
    /// 기록된 해시 (문제가 난 파일의 것)
    pub expected_hash: Option<String>,
    /// 실제 계산된 해시
    pub actual_hash: Option<String>,
    /// 문제가 난 파일 경로
    pub file_path: Option<String>,
    /// 상세 메시지
    pub message: String,
}

/// 무결성 상태
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub enum IntegrityStatus {
    /// 기록된 파일이 모두 그대로다
    Verified,
    /// 설치한 뒤 내용이 바뀐 파일이 있다
    Tampered,
    /// 설치한 파일이 없어졌다
    FileNotFound,
    /// 파일을 읽지 못했다
    Error,
}

/// 전체 무결성 검증 결과
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct IntegrityReport {
    /// 검증 시각
    pub checked_at: String,
    /// 전체 결과 요약
    pub overall: OverallIntegrity,
    /// 개별 컴포넌트 결과
    pub components: Vec<ComponentIntegrity>,
    /// 검증한 컴포넌트 수
    pub total: usize,
    /// 통과한 컴포넌트 수
    pub verified: usize,
    /// 실패한 컴포넌트 수 (Tampered + Error)
    pub failed: usize,
    /// 파일이 없어진 컴포넌트 수
    pub skipped: usize,
}

/// 전체 무결성 요약
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub enum OverallIntegrity {
    /// 모든 컴포넌트 검증 통과
    AllVerified,
    /// 바뀐 파일은 없지만 없어진 파일이 있다
    Partial,
    /// 하나 이상의 컴포넌트에서 바뀐 파일이 있다
    TamperDetected,
    /// 기록된 설치가 없다
    Empty,
}

// ══════════════════════════════════════════════════════
// 기록과 검증
// ══════════════════════════════════════════════════════

fn records_path() -> PathBuf {
    crate::constants::resolve_data_dir().join("installed-files.json")
}

fn load_records(path: &Path) -> InstalledFiles {
    crate::read_json(path).ok().flatten().unwrap_or_default()
}

/// 적용한 컴포넌트의 파일 해시를 기록한다. 같은 컴포넌트의 이전 기록은 바꾼다
pub fn record_install(component_key: &str, files: &[PathBuf]) -> Result<(), String> {
    record_install_at(&records_path(), component_key, files)
}

fn record_install_at(path: &Path, component_key: &str, files: &[PathBuf]) -> Result<(), String> {
    let mut hashes = BTreeMap::new();
    for file in files {
        hashes.insert(file.to_string_lossy().to_string(), compute_sha256(file)?);
    }
    // 읽을 수 없는 기록을 이 키 하나로 덮어쓰면 다른 컴포넌트의 기록이 사라진다
    let mut records: InstalledFiles = crate::read_json(path)?.unwrap_or_default();
    records.insert(component_key.to_string(), hashes);
    let json = serde_json::to_string_pretty(&records).map_err(|e| e.to_string())?;
    crate::write_atomic(path, json.as_bytes()).map_err(|e| e.to_string())
}

/// 폴더째 설치한 컴포넌트(익스텐션 스토어 설치 등)의 모든 파일을 기록한다
pub fn record_dir(component_key: &str, dir: &Path) -> Result<(), String> {
    fn walk(dir: &Path, out: &mut Vec<PathBuf>) -> std::io::Result<()> {
        for entry in std::fs::read_dir(dir)? {
            let entry = entry?;
            let kind = entry.file_type()?;
            if kind.is_dir() {
                walk(&entry.path(), out)?;
            } else if kind.is_file() {
                out.push(entry.path());
            }
        }
        Ok(())
    }
    let mut files = Vec::new();
    walk(dir, &mut files).map_err(|e| e.to_string())?;
    record_install(component_key, &files)
}

/// 모듈·익스텐션이 어디서 왔고 그대로인지 (GUI·CLI에 보이는 신뢰 표시)
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Trust {
    /// 서명된 릴리즈 카탈로그에서 받았고 설치한 파일이 그대로다
    Verified,
    /// 카탈로그에서 받았지만 설치한 뒤 파일이 바뀌었거나 없어졌다
    Modified,
    /// 카탈로그에서 받은 것이 아니다 (직접 넣은 폴더·zip, 다른 주소에서 받은 것)
    Unverified,
}

fn official_path() -> PathBuf {
    crate::constants::resolve_data_dir().join("official-components.json")
}

fn load_official(path: &Path) -> BTreeSet<String> {
    crate::read_json(path).ok().flatten().unwrap_or_default()
}

/// 이 컴포넌트를 서명된 카탈로그에서 받았는지 적는다. 다른 곳에서 다시 깔면 false로 지운다
pub fn set_official(component_key: &str, official: bool) -> Result<(), String> {
    set_official_at(&official_path(), component_key, official)
}

fn set_official_at(path: &Path, component_key: &str, official: bool) -> Result<(), String> {
    let mut keys: BTreeSet<String> = crate::read_json(path)?.unwrap_or_default();
    let changed = if official { keys.insert(component_key.to_string()) } else { keys.remove(component_key) };
    if !changed {
        return Ok(());
    }
    let json = serde_json::to_string_pretty(&keys).map_err(|e| e.to_string())?;
    crate::write_atomic(path, json.as_bytes()).map_err(|e| e.to_string())
}

/// Python이 불러올 수 있는 파일. 설치 기록에 없는 것이 폴더에 생기면 "modified"
const CODE_SUFFIXES: &[&str] = &["py", "pyc", "pyo", "pyw", "so", "pyd", "dll", "dylib", "pth"];

/// 컴포넌트 키(`module-<id>`, `ext-<id>`)의 신뢰 표시. `dir`은 그 컴포넌트가 설치된 폴더
pub fn trust_of(component_key: &str, dir: &Path) -> Trust {
    trust_at(&records_path(), &official_path(), component_key, dir)
}

fn trust_at(records: &Path, official: &Path, component_key: &str, dir: &Path) -> Trust {
    if !load_official(official).contains(component_key) {
        return Trust::Unverified;
    }
    let records = load_records(records);
    let Some(files) = records.get(component_key) else {
        return Trust::Unverified;
    };
    if verify_component(component_key, files, cached_sha256).status != IntegrityStatus::Verified || has_unrecorded_code(dir, files) {
        return Trust::Modified;
    }
    Trust::Verified
}

/// 설치 기록에 없는 코드 파일이 폴더에 있나 (`__pycache__`는 데몬이 읽지 않으므로 뺀다)
fn has_unrecorded_code(dir: &Path, recorded: &BTreeMap<String, String>) -> bool {
    // 기록의 경로와 폴더 경로가 다른 모양(상대 경로, `\\?\` 접두사)일 수 있어 둘 다 정규화해 비교한다
    let recorded: BTreeSet<PathBuf> = recorded.keys().filter_map(|p| std::fs::canonicalize(p).ok()).collect();
    let Ok(dir) = std::fs::canonicalize(dir) else { return false };
    fn walk(dir: &Path, recorded: &BTreeSet<PathBuf>) -> bool {
        let Ok(entries) = std::fs::read_dir(dir) else { return false };
        entries.flatten().any(|entry| {
            let path = entry.path();
            let Ok(kind) = entry.file_type() else { return false };
            // 설치 기록은 링크를 적지 않는다. 링크는 폴더 밖 코드를 끌어올 수 있으니 바뀐 것으로 본다
            if kind.is_symlink() {
                return true;
            }
            if kind.is_dir() {
                return entry.file_name() != "__pycache__" && walk(&path, recorded);
            }
            let name = entry.file_name().to_string_lossy().to_ascii_lowercase();
            let code = path
                .extension()
                .map(|e| e.to_string_lossy().to_ascii_lowercase())
                .is_some_and(|e| CODE_SUFFIXES.contains(&e.as_str()))
                || name.contains(".so.");
            code && !recorded.contains(&path)
        })
    }
    walk(&dir, &recorded)
}

/// 기록된 모든 설치를 검사한다
pub fn verify_installed() -> IntegrityReport {
    verify_records(&load_records(&records_path()))
}

pub fn verify_records(records: &InstalledFiles) -> IntegrityReport {
    let mut components: Vec<ComponentIntegrity> = records
        .iter()
        .map(|(key, files)| verify_component(key, files, compute_sha256))
        .collect();
    components.sort_by_key(|c| component_sort_key(&c.component));

    let total = components.len();
    let count = |f: fn(&IntegrityStatus) -> bool| components.iter().filter(|c| f(&c.status)).count();
    let verified = count(|s| *s == IntegrityStatus::Verified);
    let failed = count(|s| matches!(s, IntegrityStatus::Tampered | IntegrityStatus::Error));
    let skipped = count(|s| *s == IntegrityStatus::FileNotFound);
    let overall = if total == 0 {
        OverallIntegrity::Empty
    } else if failed > 0 {
        OverallIntegrity::TamperDetected
    } else if verified == total {
        OverallIntegrity::AllVerified
    } else {
        OverallIntegrity::Partial
    };

    IntegrityReport {
        checked_at: chrono::Utc::now().to_rfc3339(),
        overall,
        components,
        total,
        verified,
        failed,
        skipped,
    }
}

/// `hash`: 전체 검사는 매번 새로 계산하고(`compute_sha256`), 신뢰 표시는 바뀌지 않은 파일의 값을 다시 쓴다(`cached_sha256`)
fn verify_component(key: &str, files: &BTreeMap<String, String>, hash: fn(&Path) -> Result<String, String>) -> ComponentIntegrity {
    let result = |status, expected: Option<&String>, actual: Option<String>, file: Option<&String>, message: String| {
        ComponentIntegrity {
            component: key.to_string(),
            display_name: crate::Component::from_manifest_key(key).display_name(),
            status,
            expected_hash: expected.cloned(),
            actual_hash: actual,
            file_path: file.cloned(),
            message,
        }
    };
    // 모든 파일을 본다. 바뀐 파일이 없어진 파일보다 먼저 보고된다
    let mut missing: Option<ComponentIntegrity> = None;
    let mut unreadable: Option<ComponentIntegrity> = None;
    for (file, expected) in files {
        if !Path::new(file).is_file() {
            missing.get_or_insert_with(|| {
                result(IntegrityStatus::FileNotFound, Some(expected), None, Some(file), "installed file is missing".into())
            });
            continue;
        }
        match hash(Path::new(file)) {
            Ok(actual) if actual.eq_ignore_ascii_case(expected) => {}
            Ok(actual) => {
                return result(IntegrityStatus::Tampered, Some(expected), Some(actual), Some(file), "file changed since install".into());
            }
            Err(e) => {
                unreadable.get_or_insert_with(|| result(IntegrityStatus::Error, Some(expected), None, Some(file), e));
            }
        }
    }
    if let Some(r) = unreadable.or(missing) {
        return r;
    }
    result(IntegrityStatus::Verified, None, None, None, format!("{} files match", files.len()))
}

// ══════════════════════════════════════════════════════
// SHA256 계산
// ══════════════════════════════════════════════════════

/// 파일의 SHA256 해시를 계산합니다.
pub fn compute_sha256(path: &Path) -> Result<String, String> {
    use std::io::BufReader;

    let file = std::fs::File::open(path)
        .map_err(|e| format!("파일 열기 실패: {}", e))?;

    let mut reader = BufReader::new(file);
    let mut hasher = Sha256::new();
    let mut buffer = [0u8; 8192];

    loop {
        let n = reader.read(&mut buffer)
            .map_err(|e| format!("파일 읽기 실패: {}", e))?;
        if n == 0 {
            break;
        }
        hasher.update(&buffer[..n]);
    }

    Ok(hasher.finalize().iter().map(|b| format!("{:02x}", b)).collect())
}

/// 파일 크기·수정 시각 (같으면 내용이 그대로라고 본다)
fn file_stamp(path: &Path) -> Option<(u64, std::time::SystemTime)> {
    let meta = std::fs::metadata(path).ok()?;
    Some((meta.len(), meta.modified().ok()?))
}

/// 방금 고친 파일은 같은 수정 시각 안에 다시 바뀔 수 있어 저장하지 않는다
const RACY_STAMP: std::time::Duration = std::time::Duration::from_secs(2);

/// `compute_sha256`과 같지만, 경로·크기·수정 시각이 지난번과 같으면 다시 읽지 않는다 (데몬 실행 동안만 기억한다).
/// 목록 조회·Python 실행마다 설치한 파일을 모두 해시하지 않게 한다
pub fn cached_sha256(path: &Path) -> Result<String, String> {
    use std::collections::HashMap;
    use std::sync::{Mutex, OnceLock};
    type Cache = Mutex<HashMap<PathBuf, ((u64, std::time::SystemTime), String)>>;
    static CACHE: OnceLock<Cache> = OnceLock::new();
    let cache = CACHE.get_or_init(Default::default);
    let before = file_stamp(path);
    if let Some(stamp) = before {
        let hit = cache.lock().unwrap_or_else(|e| e.into_inner()).get(path).filter(|(s, _)| *s == stamp).map(|(_, h)| h.clone());
        if let Some(hash) = hit {
            return Ok(hash);
        }
    }
    let hash = compute_sha256(path)?;
    // 읽는 동안 바뀌지 않았고 오래된 파일만 기억한다
    let settled = before.filter(|(_, mtime)| mtime.elapsed().is_ok_and(|age| age >= RACY_STAMP));
    if let Some(stamp) = settled.filter(|s| file_stamp(path) == Some(*s)) {
        cache.lock().unwrap_or_else(|e| e.into_inner()).insert(path.to_path_buf(), (stamp, hash.clone()));
    }
    Ok(hash)
}

/// 컴포넌트 키를 정렬 우선순위로 변환 (코어 → 모듈 → 익스텐션)
fn component_sort_key(key: &str) -> (u8, String) {
    match key {
        "saba-core" => (0, key.to_string()),
        "cli" => (1, key.to_string()),
        "gui" => (2, key.to_string()),
        "updater" => (3, key.to_string()),
        k if k.starts_with("module-") => (10, k.to_string()),
        k if k.starts_with("ext-") => (20, k.to_string()),
        k => (5, k.to_string()),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_sha256_hello() {
        let tmp = tempfile::tempdir().unwrap();
        let f = tmp.path().join("h.txt");
        std::fs::write(&f, b"hello").unwrap();
        assert_eq!(
            compute_sha256(&f).unwrap(),
            "2cf24dba5fb0a30e26e83b2ac5b9e29e1b161e5c1fa7425e73043362938b9824"
        );
    }

    /// 크기·수정 시각이 그대로면 다시 읽지 않고, 둘 중 하나라도 바뀌면 다시 계산한다
    #[test]
    fn unchanged_files_are_not_rehashed() {
        let tmp = tempfile::tempdir().unwrap();
        let f = tmp.path().join("server.jar");
        let old = std::time::SystemTime::now() - std::time::Duration::from_secs(60);
        let write = |content: &[u8], mtime| {
            std::fs::write(&f, content).unwrap();
            std::fs::File::options().write(true).open(&f).unwrap().set_modified(mtime).unwrap();
        };
        write(b"hello", old);
        let first = cached_sha256(&f).unwrap();
        assert_eq!(first, compute_sha256(&f).unwrap());
        // 같은 크기·같은 시각으로 바꾸면 기억한 값이 나온다 (다시 읽지 않았다는 뜻)
        write(b"HELLO", old);
        assert_eq!(cached_sha256(&f).unwrap(), first);
        // 시각이 바뀌면 다시 계산한다
        write(b"HELLO", old + std::time::Duration::from_secs(1));
        assert_eq!(cached_sha256(&f).unwrap(), compute_sha256(&f).unwrap());
        assert_ne!(cached_sha256(&f).unwrap(), first);
        // 방금 고친 파일은 기억하지 않는다
        write(b"fresh", std::time::SystemTime::now());
        let fresh = cached_sha256(&f).unwrap();
        std::fs::write(&f, b"FRESH").unwrap();
        assert_ne!(cached_sha256(&f).unwrap(), fresh);
        std::fs::remove_file(&f).unwrap();
        assert!(cached_sha256(&f).is_err());
    }

    #[test]
    fn only_untouched_catalog_installs_are_trusted() {
        let tmp = tempfile::tempdir().unwrap();
        let (records, official) = (tmp.path().join("installed-files.json"), tmp.path().join("official.json"));
        let dir = tmp.path().join("x");
        std::fs::create_dir(&dir).unwrap();
        let a = dir.join("lifecycle.py");
        std::fs::write(&a, b"one").unwrap();
        record_install_at(&records, "module-x", std::slice::from_ref(&a)).unwrap();
        let trust = |key: &str| trust_at(&records, &official, key, &dir);
        // 기록은 있어도 카탈로그에서 받은 것이 아니면 검증되지 않은 것
        assert_eq!(trust("module-x"), Trust::Unverified);

        set_official_at(&official, "module-x", true).unwrap();
        assert_eq!(trust("module-x"), Trust::Verified);
        // 코드가 아닌 파일이나 __pycache__는 상관없다
        std::fs::write(dir.join("server.properties"), b"x").unwrap();
        std::fs::create_dir(dir.join("__pycache__")).unwrap();
        std::fs::write(dir.join("__pycache__/lifecycle.cpython-312.pyc"), b"x").unwrap();
        assert_eq!(trust("module-x"), Trust::Verified);
        // 기록에 없는 코드 파일(표준 모듈을 가리는 json.py 등)이 생기면 바뀐 것 (cloud review 5)
        std::fs::write(dir.join("json.py"), b"import socket").unwrap();
        assert_eq!(trust("module-x"), Trust::Modified);
        std::fs::remove_file(dir.join("json.py")).unwrap();
        std::fs::create_dir(dir.join("lib")).unwrap();
        std::fs::write(dir.join("lib/fast.pyd"), b"MZ").unwrap();
        assert_eq!(trust("module-x"), Trust::Modified);
        std::fs::remove_dir_all(dir.join("lib")).unwrap();
        assert_eq!(trust("module-x"), Trust::Verified);
        #[cfg(unix)]
        {
            std::os::unix::fs::symlink(tmp.path(), dir.join("outside")).unwrap();
            assert_eq!(trust("module-x"), Trust::Modified);
            std::fs::remove_file(dir.join("outside")).unwrap();
            assert_eq!(trust("module-x"), Trust::Verified);
        }

        std::fs::write(&a, b"changed").unwrap();
        assert_eq!(trust("module-x"), Trust::Modified);

        set_official_at(&official, "module-x", false).unwrap();
        assert_eq!(trust("module-x"), Trust::Unverified);
        assert_eq!(trust("ext-none"), Trust::Unverified);
        assert_eq!(serde_json::to_value(Trust::Modified).unwrap(), "modified");
    }

    #[test]
    fn recorded_files_verify_until_changed_or_removed() {
        let tmp = tempfile::tempdir().unwrap();
        let records = tmp.path().join("installed-files.json");
        let a = tmp.path().join("a.py");
        let b = tmp.path().join("b.py");
        std::fs::write(&a, b"one").unwrap();
        std::fs::write(&b, b"two").unwrap();
        record_install_at(&records, "module-test", &[a.clone(), b.clone()]).unwrap();

        let report = verify_records(&load_records(&records));
        assert_eq!(report.overall, OverallIntegrity::AllVerified);
        assert_eq!(report.components[0].status, IntegrityStatus::Verified);

        std::fs::write(&a, b"changed").unwrap();
        let report = verify_records(&load_records(&records));
        assert_eq!(report.overall, OverallIntegrity::TamperDetected);
        assert_eq!(report.components[0].file_path.as_deref(), Some(a.to_string_lossy().as_ref()));

        // 없어진 파일이 앞에 있어도 뒤의 바뀐 파일을 놓치지 않는다
        std::fs::remove_file(&a).unwrap();
        std::fs::write(&b, b"changed").unwrap();
        let report = verify_records(&load_records(&records));
        assert_eq!(report.overall, OverallIntegrity::TamperDetected);

        std::fs::write(&a, b"one").unwrap();
        std::fs::remove_file(&b).unwrap();
        let report = verify_records(&load_records(&records));
        assert_eq!(report.components[0].status, IntegrityStatus::FileNotFound);
        assert_eq!(report.overall, OverallIntegrity::Partial);
    }

    /// 읽을 수 없는 기록 파일을 새 키 하나만으로 덮어쓰면 다른 컴포넌트의 기록이 모두 사라진다
    #[test]
    fn an_unreadable_record_file_is_not_overwritten() {
        let tmp = tempfile::tempdir().unwrap();
        let file = tmp.path().join("a.py");
        std::fs::write(&file, b"x").unwrap();
        let records = tmp.path().join("installed-files.json");
        let official = tmp.path().join("official-components.json");
        std::fs::write(&records, b"{\"module-a\": {").unwrap();
        std::fs::write(&official, b"[\"module-a\"").unwrap();

        assert!(record_install_at(&records, "module-b", std::slice::from_ref(&file)).is_err());
        assert!(set_official_at(&official, "module-b", true).is_err());
        assert_eq!(std::fs::read(&records).unwrap(), b"{\"module-a\": {");
        assert_eq!(std::fs::read(&official).unwrap(), b"[\"module-a\"");
    }

    #[test]
    fn nothing_recorded_is_empty() {
        assert_eq!(verify_records(&InstalledFiles::new()).overall, OverallIntegrity::Empty);
    }
}
