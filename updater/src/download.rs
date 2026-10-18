//! 검증 다운로드 — 받으면서 SHA-256을 계산하고, 기대값과 다르면 파일을 지운다.
//!
//! 업데이트·설치 경로의 모든 다운로드가 이 함수를 거친다. 반쯤 쓰인 파일이나
//! 변조된 파일이 스테이징 폴더에 남지 않는다. `<이름>.part`에 받고 다 맞으면 이름을 바꾸므로,
//! 다시 받다 실패해도 이미 받아 둔 파일은 그대로다.

use futures_util::StreamExt;
use sha2::{Digest, Sha256};
use std::io::Write;
use std::path::Path;
use std::sync::{Arc, Mutex};
use std::time::Duration;

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
    #[error("download stalled: no data for {0:?}")]
    Stalled(Duration),
    #[error("download is larger than the expected {limit} bytes")]
    TooLarge { limit: u64 },
}

/// 응답 머리나 다음 조각을 이만큼 못 받으면 멈춘 것으로 보고 끊는다
pub const IDLE_TIMEOUT: Duration = Duration::from_secs(30);

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
/// `expected`가 있으면 일치하지 않을 때 `Mismatch`를 돌려준다.
/// `None`이면 검증을 건너뛴다(해시가 없는 개발 빌드).
/// `max_bytes`(서명된 manifest의 `size_bytes`)보다 많이 오면 끊는다.
/// 실패하면 `dest`는 받기 전 그대로다.
pub async fn download_verified(
    client: &reqwest::Client,
    url: &str,
    dest: &Path,
    expected: Option<&str>,
    max_bytes: Option<u64>,
    progress: Option<&Arc<Mutex<DownloadProgress>>>,
) -> Result<String, DownloadError> {
    download_with(client, url, dest, expected, max_bytes, progress, IDLE_TIMEOUT).await
}

async fn download_with(
    client: &reqwest::Client,
    url: &str,
    dest: &Path,
    expected: Option<&str>,
    max_bytes: Option<u64>,
    progress: Option<&Arc<Mutex<DownloadProgress>>>,
    idle: Duration,
) -> Result<String, DownloadError> {
    let expected = expected.map(parse_sha256).transpose()?;
    let mut part = dest.as_os_str().to_owned();
    part.push(".part");
    let part = std::path::PathBuf::from(part);
    let result = stream_to_file(client, url, &part, max_bytes, progress, idle).await;
    let actual = match result {
        Ok(hash) => hash,
        Err(e) => {
            let _ = std::fs::remove_file(&part);
            return Err(e);
        }
    };
    if let Some(expected) = expected {
        if expected != actual {
            let _ = std::fs::remove_file(&part);
            return Err(DownloadError::Mismatch {
                file: dest.display().to_string(),
                expected,
                actual,
            });
        }
    } else {
        tracing::warn!("[Download] No SHA-256 for {} — skipping verification", url);
    }
    if let Err(e) = std::fs::rename(&part, dest) {
        let _ = std::fs::remove_file(&part);
        return Err(e.into());
    }
    Ok(actual)
}

async fn stream_to_file(
    client: &reqwest::Client,
    url: &str,
    dest: &Path,
    max_bytes: Option<u64>,
    progress: Option<&Arc<Mutex<DownloadProgress>>>,
    idle: Duration,
) -> Result<String, DownloadError> {
    let response = tokio::time::timeout(idle, client.get(url).send())
        .await
        .map_err(|_| DownloadError::Stalled(idle))?
        .map_err(|e| DownloadError::Http(e.to_string()))?;
    if !response.status().is_success() {
        return Err(DownloadError::Http(format!("{} returned {}", url, response.status())));
    }
    if let (Some(limit), Some(len)) = (max_bytes, response.content_length()) {
        if len > limit {
            return Err(DownloadError::TooLarge { limit });
        }
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
    while let Some(chunk) = tokio::time::timeout(idle, stream.next()).await.map_err(|_| DownloadError::Stalled(idle))? {
        let chunk = chunk.map_err(|e| DownloadError::Http(e.to_string()))?;
        received += chunk.len() as u64;
        if let Some(limit) = max_bytes.filter(|limit| received > *limit) {
            return Err(DownloadError::TooLarge { limit });
        }
        file.write_all(&chunk)?;
        hasher.update(&chunk);
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
    use std::time::Duration;

    #[test]
    fn parse_sha256_accepts_hex_and_normalizes_case() {
        let upper = "E3B0C44298FC1C149AFBF4C8996FB92427AE41E4649B934CA495991B7852B855";
        assert_eq!(parse_sha256(upper).unwrap(), upper.to_ascii_lowercase());
        assert!(parse_sha256("abc").is_err());
        assert!(parse_sha256(&"g".repeat(64)).is_err());
    }

    /// 한 번만 응답하는 가짜 서버. `response`를 보내고 연결을 닫는다
    async fn serve_once(response: &'static [u8]) -> String {
        use tokio::io::{AsyncReadExt, AsyncWriteExt};
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();
        tokio::spawn(async move {
            let (mut sock, _) = listener.accept().await.unwrap();
            let mut buf = [0u8; 1024];
            let _ = sock.read(&mut buf).await;
            let _ = sock.write_all(response).await;
        });
        format!("http://{}/asset.zip", addr)
    }

    #[tokio::test]
    async fn a_cut_off_download_fails_and_leaves_nothing() {
        let url = serve_once(b"HTTP/1.1 200 OK\r\nContent-Length: 100\r\n\r\nonly ten b").await;
        let tmp = tempfile::tempdir().unwrap();
        let dest = tmp.path().join("asset.zip");
        let err = download_verified(&reqwest::Client::new(), &url, &dest, None, None, None).await.unwrap_err();
        assert!(matches!(err, DownloadError::Http(_)), "{:?}", err);
        assert!(!dest.exists());
    }

    /// 응답을 조금 보내고 멈추는 서버
    async fn serve_and_stall(head: &'static [u8]) -> String {
        use tokio::io::{AsyncReadExt, AsyncWriteExt};
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();
        tokio::spawn(async move {
            let (mut sock, _) = listener.accept().await.unwrap();
            let mut buf = [0u8; 1024];
            let _ = sock.read(&mut buf).await;
            let _ = sock.write_all(head).await;
            tokio::time::sleep(Duration::from_secs(60)).await;
        });
        format!("http://{}/asset.zip", addr)
    }

    async fn fetch_with_idle(url: &str, dest: &Path, idle: Duration) -> Result<String, DownloadError> {
        let client = reqwest::Client::new();
        let fut = download_with(&client, url, dest, None, None, None, idle);
        tokio::time::timeout(Duration::from_secs(10), fut).await.expect("a stalled download must give up")
    }

    #[tokio::test]
    async fn a_stalled_body_times_out() {
        let url = serve_and_stall(b"HTTP/1.1 200 OK\r\nContent-Length: 1000\r\n\r\nabc").await;
        let tmp = tempfile::tempdir().unwrap();
        let dest = tmp.path().join("asset.zip");
        let err = fetch_with_idle(&url, &dest, Duration::from_millis(300)).await.unwrap_err();
        assert!(matches!(err, DownloadError::Stalled(_)), "{:?}", err);
        assert!(!dest.exists());
    }

    #[tokio::test]
    async fn a_server_that_never_answers_times_out() {
        let url = serve_and_stall(b"").await;
        let tmp = tempfile::tempdir().unwrap();
        let dest = tmp.path().join("asset.zip");
        let err = fetch_with_idle(&url, &dest, Duration::from_millis(300)).await.unwrap_err();
        assert!(matches!(err, DownloadError::Stalled(_)), "{:?}", err);
    }

    #[tokio::test]
    async fn more_bytes_than_the_manifest_size_are_refused() {
        // 길이를 밝히지 않고 연결을 닫을 때까지 보내는 응답
        let url = serve_once(b"HTTP/1.1 200 OK\r\nConnection: close\r\n\r\n0123456789abcdefghij").await;
        let tmp = tempfile::tempdir().unwrap();
        let dest = tmp.path().join("asset.zip");
        let err = download_verified(&reqwest::Client::new(), &url, &dest, None, Some(10), None).await.unwrap_err();
        assert!(matches!(err, DownloadError::TooLarge { limit: 10 }), "{:?}", err);
        assert!(!dest.exists());
    }

    #[tokio::test]
    async fn a_failed_download_keeps_the_file_already_there() {
        let url = serve_once(b"HTTP/1.1 200 OK\r\nContent-Length: 100\r\n\r\nonly ten b").await;
        let tmp = tempfile::tempdir().unwrap();
        let dest = tmp.path().join("asset.zip");
        std::fs::write(&dest, b"good staged file").unwrap();
        assert!(download_verified(&reqwest::Client::new(), &url, &dest, None, None, None).await.is_err());
        assert_eq!(std::fs::read(&dest).unwrap(), b"good staged file");
        assert!(!tmp.path().join("asset.zip.part").exists());
    }

    #[tokio::test]
    async fn an_error_status_is_not_saved() {
        let url = serve_once(b"HTTP/1.1 404 Not Found\r\nContent-Length: 0\r\n\r\n").await;
        let tmp = tempfile::tempdir().unwrap();
        let dest = tmp.path().join("asset.zip");
        assert!(download_verified(&reqwest::Client::new(), &url, &dest, None, None, None).await.is_err());
        assert!(!dest.exists());
    }
}
