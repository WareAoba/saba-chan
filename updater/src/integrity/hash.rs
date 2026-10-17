//! SHA-256 해시 타입 및 파일/바이트 해시 계산·검증 유틸
//!
//! `sha2` 크레이트 기반. 이전의 순수 Rust SHA-256 구현을 대체합니다.

use std::fs::File;
use std::io::{BufReader, Read};
use std::path::{Path, PathBuf};

use sha2::{Digest, Sha256};

/// SHA-256 해시값 (32 bytes 고정)
#[derive(Clone, Copy, PartialEq, Eq, Hash)]
pub struct FileHash([u8; 32]);

impl FileHash {
    pub fn from_bytes(b: [u8; 32]) -> Self {
        Self(b)
    }

    pub fn as_bytes(&self) -> &[u8; 32] {
        &self.0
    }

    pub fn parse_hex(s: &str) -> Result<Self, IntegrityError> {
        let trimmed = s.trim();
        if trimmed.len() != 64 {
            return Err(IntegrityError::InvalidHashFormat(format!(
                "expected 64 hex chars, got {}",
                trimmed.len()
            )));
        }
        let bytes = hex::decode(trimmed).map_err(|e| {
            IntegrityError::InvalidHashFormat(format!("invalid hex: {}", e))
        })?;
        if bytes.len() != 32 {
            return Err(IntegrityError::InvalidHashFormat(format!(
                "decoded length {} != 32",
                bytes.len()
            )));
        }
        let mut out = [0u8; 32];
        out.copy_from_slice(&bytes);
        Ok(Self(out))
    }

    pub fn to_hex_lower(&self) -> String {
        hex::encode(self.0)
    }
}

impl std::fmt::Debug for FileHash {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "FileHash({})", self.to_hex_lower())
    }
}

impl std::fmt::Display for FileHash {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.to_hex_lower())
    }
}

#[derive(Debug, thiserror::Error)]
pub enum IntegrityError {
    #[error("IO 오류: {0}")]
    Io(#[from] std::io::Error),
    #[error("해시 형식 오류 ({0})")]
    InvalidHashFormat(String),
    #[error("해시 불일치: 기대 {expected}, 실제 {actual}, 파일 {file}")]
    Mismatch {
        expected: String,
        actual: String,
        file: PathBuf,
    },
    #[error("다운로드 오류: {0}")]
    Download(String),
}

/// 파일의 SHA-256을 계산합니다. (동기, 8 KiB 버퍼 스트리밍)
pub fn hash_file(path: &Path) -> std::io::Result<FileHash> {
    let file = File::open(path)?;
    let mut reader = BufReader::new(file);
    let mut hasher = Sha256::new();
    let mut buf = [0u8; 8192];
    loop {
        let n = reader.read(&mut buf)?;
        if n == 0 {
            break;
        }
        hasher.update(&buf[..n]);
    }
    let out = hasher.finalize();
    let mut arr = [0u8; 32];
    arr.copy_from_slice(&out);
    Ok(FileHash(arr))
}

/// 바이트 버퍼의 SHA-256을 계산합니다.
pub fn hash_bytes(bytes: &[u8]) -> FileHash {
    let mut hasher = Sha256::new();
    hasher.update(bytes);
    let out = hasher.finalize();
    let mut arr = [0u8; 32];
    arr.copy_from_slice(&out);
    FileHash(arr)
}

/// 파일의 해시가 기대값과 일치하는지 검증합니다. 불일치 시 `Mismatch` 에러.
pub fn verify_file(path: &Path, expected: &FileHash) -> Result<(), IntegrityError> {
    let actual = hash_file(path)?;
    if &actual == expected {
        Ok(())
    } else {
        Err(IntegrityError::Mismatch {
            expected: expected.to_hex_lower(),
            actual: actual.to_hex_lower(),
            file: path.to_path_buf(),
        })
    }
}

/// 바이트의 해시가 기대값과 일치하는지 검증합니다.
pub fn verify_bytes(bytes: &[u8], expected: &FileHash) -> Result<(), IntegrityError> {
    let actual = hash_bytes(bytes);
    if &actual == expected {
        Ok(())
    } else {
        Err(IntegrityError::Mismatch {
            expected: expected.to_hex_lower(),
            actual: actual.to_hex_lower(),
            file: PathBuf::new(),
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write;

    const EMPTY_HASH: &str = "e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855";
    const ABC_HASH: &str = "ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad";

    #[test]
    fn hash_bytes_empty() {
        assert_eq!(hash_bytes(b"").to_hex_lower(), EMPTY_HASH);
    }

    #[test]
    fn hash_bytes_abc() {
        assert_eq!(hash_bytes(b"abc").to_hex_lower(), ABC_HASH);
    }

    #[test]
    fn hash_bytes_multi_block() {
        // >64 bytes requires multiple SHA-256 blocks
        let data = vec![b'a'; 10_000];
        let h = hash_bytes(&data);
        // Known: sha256 of 10000 'a' chars
        assert_eq!(
            h.to_hex_lower(),
            "27dd1f61b867b6a0f6e9d8a41c43231de52107e53ae424de8f847b821db4b711"
        );
    }

    #[test]
    fn parse_hex_ok_lower_and_upper() {
        let lower = FileHash::parse_hex(EMPTY_HASH).unwrap();
        let upper = FileHash::parse_hex(&EMPTY_HASH.to_uppercase()).unwrap();
        assert_eq!(lower, upper);
    }

    #[test]
    fn parse_hex_bad_length() {
        assert!(FileHash::parse_hex("abc").is_err());
        assert!(FileHash::parse_hex(&"0".repeat(63)).is_err());
        assert!(FileHash::parse_hex(&"0".repeat(65)).is_err());
    }

    #[test]
    fn parse_hex_bad_chars() {
        let bad = "zz".repeat(32);
        assert!(FileHash::parse_hex(&bad).is_err());
    }

    #[test]
    fn verify_file_match_and_mismatch() {
        let dir = tempfile::tempdir().unwrap();
        let p = dir.path().join("data.bin");
        let mut f = File::create(&p).unwrap();
        f.write_all(b"abc").unwrap();
        drop(f);

        let expected = FileHash::parse_hex(ABC_HASH).unwrap();
        verify_file(&p, &expected).unwrap();

        let wrong = FileHash::parse_hex(EMPTY_HASH).unwrap();
        let err = verify_file(&p, &wrong).unwrap_err();
        assert!(matches!(err, IntegrityError::Mismatch { .. }));
    }

    #[test]
    fn verify_file_missing() {
        let expected = FileHash::parse_hex(EMPTY_HASH).unwrap();
        let err = verify_file(Path::new("/nonexistent/path/xyz.bin"), &expected).unwrap_err();
        assert!(matches!(err, IntegrityError::Io(_)));
    }

    #[test]
    fn hash_file_empty_and_large() {
        let dir = tempfile::tempdir().unwrap();
        let p = dir.path().join("empty.bin");
        File::create(&p).unwrap();
        assert_eq!(hash_file(&p).unwrap().to_hex_lower(), EMPTY_HASH);

        // 10 MB file
        let big = dir.path().join("big.bin");
        let mut f = File::create(&big).unwrap();
        let chunk = vec![0u8; 64 * 1024];
        for _ in 0..160 {
            f.write_all(&chunk).unwrap();
        }
        drop(f);
        let h = hash_file(&big).unwrap();
        // Just ensure it runs + consistent
        let h2 = hash_file(&big).unwrap();
        assert_eq!(h, h2);
    }
}
