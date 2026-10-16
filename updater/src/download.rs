//! 검증 다운로드 — 받으면서 SHA-256을 계산하고, 기대값과 다르면 파일을 지운다.
//!
//! 업데이트·설치 경로의 모든 다운로드가 이 함수를 거친다. 반쯤 쓰인 파일이나
//! 변조된 파일이 스테이징 폴더에 남지 않는다.

use futures_util::StreamExt;
use sha2::{Digest, Sha256};
use std::io::Write;
use std::path::Path;
use std::sync::{Arc, Mutex};

use crate::DownloadProgress;

#[derive(Debug, thiserror::Error)]
pub enum DownloadError {
    #[error("download failed: {0}")]
    Http(String),
    #[error("I/O error: {0}")]
    Io(#[from] std::io::Error),
    #[error("SHA-256 mismatch for {file}: expected {expected}, got {actual}")]
    Mismatch { file: String, expected: String, actual: String },
    #[error("invalid SHA-256 value: {0}")]
    BadHash(String),
}

/// 64자리 hex인지 확인하고 소문자로 정규화한다.
pub fn parse_sha256(s: &str) -> Result<String, DownloadError> {
    let s = s.trim();
    if s.len() == 64 && s.chars().all(|c| c.is_ascii_hexdigit()) {
        Ok(s.to_ascii_lowercase())
    } else {
        Err(DownloadError::BadHash(s.to_string()))
    }
}

/// `url`을 `dest`로 받는다. 반환값은 받은 파일의 SHA-256(hex 소문자).
///
/// `expected`가 있으면 일치하지 않을 때 `dest`를 지우고 `Mismatch`를 돌려준다.
/// `None`이면 검증을 건너뛴다(해시가 없는 개발 빌드).
pub async fn download_verified(
    client: &reqwest::Client,
    url: &str,
    dest: &Path,
    expected: Option<&str>,
    progress: Option<&Arc<Mutex<DownloadProgress>>>,
) -> Result<String, DownloadError> {
    let expected = expected.map(parse_sha256).transpose()?;
    let result = stream_to_file(client, url, dest, progress).await;
    let actual = match result {
        Ok(hash) => hash,
        Err(e) => {
            let _ = std::fs::remove_file(dest);
            return Err(e);
        }
    };
    if let Some(expected) = expected {
        if expected != actual {
            let _ = std::fs::remove_file(dest);
            return Err(DownloadError::Mismatch {
                file: dest.display().to_string(),
                expected,
                actual,
            });
        }
    } else {
        tracing::warn!("[Download] No SHA-256 for {} — skipping verification", url);
    }
    Ok(actual)
}

async fn stream_to_file(
    client: &reqwest::Client,
    url: &str,
    dest: &Path,
    progress: Option<&Arc<Mutex<DownloadProgress>>>,
) -> Result<String, DownloadError> {
    let response = client
        .get(url)
        .send()
        .await
        .map_err(|e| DownloadError::Http(e.to_string()))?;
    if !response.status().is_success() {
        return Err(DownloadError::Http(format!("{} returned {}", url, response.status())));
    }
    if let Some(p) = progress {
        if let Ok(mut p) = p.lock() {
            p.total_bytes = response.content_length().unwrap_or(0);
            p.bytes_received = 0;
        }
    }
    if let Some(parent) = dest.parent() {
        std::fs::create_dir_all(parent)?;
    }
    let mut file = std::fs::File::create(dest)?;
    let mut hasher = Sha256::new();
    let mut received: u64 = 0;
    let mut stream = response.bytes_stream();
    while let Some(chunk) = stream.next().await {
        let chunk = chunk.map_err(|e| DownloadError::Http(e.to_string()))?;
        file.write_all(&chunk)?;
        hasher.update(&chunk);
        received += chunk.len() as u64;
        if let Some(p) = progress {
            if let Ok(mut p) = p.lock() {
                p.bytes_received = received;
            }
        }
    }
    file.flush()?;
    Ok(hasher.finalize().iter().map(|b| format!("{:02x}", b)).collect())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_sha256_accepts_hex_and_normalizes_case() {
        let upper = "E3B0C44298FC1C149AFBF4C8996FB92427AE41E4649B934CA495991B7852B855";
        assert_eq!(parse_sha256(upper).unwrap(), upper.to_ascii_lowercase());
        assert!(parse_sha256("abc").is_err());
        assert!(parse_sha256(&"g".repeat(64)).is_err());
    }
}
