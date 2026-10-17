//! 무결성 검증 — 업데이터가 설치할 때 기록한 파일별 SHA-256과 지금 디스크의 파일을 비교한다.
//!
//! 릴리즈 manifest의 `sha256`은 zip의 해시라 풀린 파일과 비교할 수 없다. 그래서 적용할 때
//! zip에서 푼 파일마다 해시를 `<데이터 폴더>/installed-files.json`에 적어 두고(`record_install`),
//! 검사(`verify_installed`)는 그 기록만 본다. 네트워크를 쓰지 않는다.
//! 업데이터가 설치하지 않은 컴포넌트(설치기로 처음 깐 것)는 기록이 없어 결과에 나오지 않는다.

use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::collections::BTreeMap;
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
    std::fs::read_to_string(path)
        .ok()
        .and_then(|s| serde_json::from_str(&s).ok())
        .unwrap_or_default()
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
    let mut records = load_records(path);
    records.insert(component_key.to_string(), hashes);
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent).map_err(|e| e.to_string())?;
    }
    let json = serde_json::to_string_pretty(&records).map_err(|e| e.to_string())?;
    std::fs::write(path, json).map_err(|e| e.to_string())
}

/// 기록된 모든 설치를 검사한다
pub fn verify_installed() -> IntegrityReport {
    verify_records(&load_records(&records_path()))
}

pub fn verify_records(records: &InstalledFiles) -> IntegrityReport {
    let mut components: Vec<ComponentIntegrity> = records
        .iter()
        .map(|(key, files)| verify_component(key, files))
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

fn verify_component(key: &str, files: &BTreeMap<String, String>) -> ComponentIntegrity {
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
    for (file, expected) in files {
        if !Path::new(file).is_file() {
            return result(IntegrityStatus::FileNotFound, Some(expected), None, Some(file), "installed file is missing".into());
        }
        match compute_sha256(Path::new(file)) {
            Ok(actual) if actual.eq_ignore_ascii_case(expected) => {}
            Ok(actual) => {
                return result(IntegrityStatus::Tampered, Some(expected), Some(actual), Some(file), "file changed since install".into());
            }
            Err(e) => return result(IntegrityStatus::Error, Some(expected), None, Some(file), e),
        }
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

        std::fs::write(&a, b"one").unwrap();
        std::fs::remove_file(&b).unwrap();
        let report = verify_records(&load_records(&records));
        assert_eq!(report.components[0].status, IntegrityStatus::FileNotFound);
        assert_eq!(report.overall, OverallIntegrity::Partial);
    }

    #[test]
    fn nothing_recorded_is_empty() {
        assert_eq!(verify_records(&InstalledFiles::new()).overall, OverallIntegrity::Empty);
    }
}
