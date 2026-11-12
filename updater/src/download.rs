//! 검증 다운로드 — 받으면서 SHA-256을 계산하고, 기대값과 다르면 파일을 지운다.
//!
//! 업데이트·설치 경로의 모든 다운로드가 이 함수를 거친다. 반쯤 쓰인 파일이나
//! 변조된 파일이 스테이징 폴더에 남지 않는다. `<이름>.part`에 받고 다 맞으면 이름을 바꾸므로,
//! 다시 받다 실패해도 이미 받아 둔 파일은 그대로다. 받다 끊기면 `.part`에 이어 받는다.

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

/// 끊기거나 서버가 잠깐 실패하면 다시 받는 횟수 (새로 받은 것이 있으면 다시 센다)
const RETRIES: u32 = 3;

#[derive(Clone, Copy)]
struct Retry {
    idle: Duration,
    retries: u32,
    /// 다시 받기 전 기다림 (연달아 실패한 횟수만큼 곱한다)
    backoff: Duration,
}

/// `url`을 `dest`로 받는다. 반환값은 받은 파일의 SHA-256(hex 소문자).
///
/// `expected`가 있으면 일치하지 않을 때 `Mismatch`를 돌려준다.
/// `None`이면 검증을 건너뛴다(해시가 없는 개발 빌드).
/// `max_bytes`(서명된 manifest의 `size_bytes`)보다 많이 오면 끊는다.
/// 연결이 끊기거나 멈추거나 서버가 5xx로 답하면 몇 번 다시 받는다. 받아 둔 데서 이어 받고(`Range`),
/// 서버가 이어 주지 않으면 처음부터 받는다. 없는 파일(4xx)·해시 불일치·크기 초과는 다시 받지 않는다.
/// 실패하면 `dest`는 받기 전 그대로다.
pub async fn download_verified(
    client: &reqwest::Client,
    url: &str,
    dest: &Path,
    expected: Option<&str>,
    max_bytes: Option<u64>,
    progress: Option<&Arc<Mutex<DownloadProgress>>>,
) -> Result<String, DownloadError> {
    let retry = Retry { idle: IDLE_TIMEOUT, retries: RETRIES, backoff: Duration::from_secs(1) };
    download_with(client, url, dest, expected, max_bytes, progress, retry).await
}

async fn download_with(
    client: &reqwest::Client,
    url: &str,
    dest: &Path,
    expected: Option<&str>,
    max_bytes: Option<u64>,
    progress: Option<&Arc<Mutex<DownloadProgress>>>,
    retry: Retry,
) -> Result<String, DownloadError> {
    let expected = expected.map(parse_sha256).transpose()?;
    let mut part = dest.as_os_str().to_owned();
    part.push(".part");
    let part = std::path::PathBuf::from(part);
    let mut partial = Partial { path: &part, hasher: Sha256::new(), received: 0 };
    let mut failures = 0;
    loop {
        let before = partial.received;
        let failed = match partial.fetch(client, url, max_bytes, progress, retry.idle).await {
            Ok(()) => break,
            Err(failed) => failed,
        };
        if partial.received > before {
            failures = 0;
        }
        if !failed.transient || failures >= retry.retries {
            let _ = std::fs::remove_file(&part);
            return Err(failed.error);
        }
        failures += 1;
        tracing::warn!("[Download] {} failed at {} bytes, trying again ({}/{}): {}", url, partial.received, failures, retry.retries, failed.error);
        tokio::time::sleep(retry.backoff * failures).await;
    }
    let actual: String = partial.hasher.finalize().iter().map(|b| format!("{:02x}", b)).collect();
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

struct Failed {
    error: DownloadError,
    /// 다시 받으면 될 수 있는 실패 (연결·멈춤·5xx)
    transient: bool,
}

impl Failed {
    fn transient(error: DownloadError) -> Self {
        Self { error, transient: true }
    }

    fn last(error: DownloadError) -> Self {
        Self { error, transient: false }
    }
}

/// 받는 중인 `.part` 파일: 지금까지 받은 길이와 그만큼의 해시
struct Partial<'a> {
    path: &'a Path,
    hasher: Sha256,
    received: u64,
}

impl Partial<'_> {
    /// 한 번 요청해 끝까지 받는다. 받아 둔 것이 있으면 그 뒤부터 달라고 한다
    async fn fetch(
        &mut self,
        client: &reqwest::Client,
        url: &str,
        max_bytes: Option<u64>,
        progress: Option<&Arc<Mutex<DownloadProgress>>>,
        idle: Duration,
    ) -> Result<(), Failed> {
        let http = |e: reqwest::Error| Failed::transient(DownloadError::Http(e.to_string()));
        let mut request = client.get(url);
        if self.received > 0 {
            request = request.header(reqwest::header::RANGE, format!("bytes={}-", self.received));
        }
        let response = tokio::time::timeout(idle, request.send())
            .await
            .map_err(|_| Failed::transient(DownloadError::Stalled(idle)))?
            .map_err(http)?;
        let status = response.status();
        let resumed = status == reqwest::StatusCode::PARTIAL_CONTENT && self.received > 0 && continues_at(&response, self.received);
        if !resumed {
            let whole = status.is_success() && status != reqwest::StatusCode::PARTIAL_CONTENT;
            // 이어 주지 않는 답(전체를 다시 보냄, 범위를 못 맞춤)이면 받아 둔 것을 버리고 처음부터다
            let start_over = whole || (self.received > 0 && matches!(status.as_u16(), 206 | 416));
            if start_over {
                self.hasher = Sha256::new();
                self.received = 0;
            }
            if !whole {
                let error = DownloadError::Http(format!("{} returned {}", url, status));
                let transient = start_over || status.is_server_error() || matches!(status.as_u16(), 408 | 429);
                return Err(Failed { error, transient });
            }
        }
        let total = response.content_length().map(|len| self.received + len);
        if let (Some(limit), Some(total)) = (max_bytes, total) {
            if total > limit {
                return Err(Failed::last(DownloadError::TooLarge { limit }));
            }
        }
        if let Some(p) = progress {
            if let Ok(mut p) = p.lock() {
                p.total_bytes = total.unwrap_or(0);
                p.bytes_received = self.received;
            }
        }
        let io = |e: std::io::Error| Failed::last(e.into());
        if let Some(parent) = self.path.parent() {
            std::fs::create_dir_all(parent).map_err(io)?;
        }
        let mut file = if resumed {
            std::fs::OpenOptions::new().append(true).open(self.path).map_err(io)?
        } else {
            std::fs::File::create(self.path).map_err(io)?
        };
        let mut stream = response.bytes_stream();
        while let Some(chunk) =
            tokio::time::timeout(idle, stream.next()).await.map_err(|_| Failed::transient(DownloadError::Stalled(idle)))?
        {
            let chunk = chunk.map_err(http)?;
            let received = self.received + chunk.len() as u64;
            if let Some(limit) = max_bytes.filter(|limit| received > *limit) {
                return Err(Failed::last(DownloadError::TooLarge { limit }));
            }
            file.write_all(&chunk).map_err(io)?;
            self.hasher.update(&chunk);
            self.received = received;
            if let Some(p) = progress {
                if let Ok(mut p) = p.lock() {
                    p.bytes_received = received;
                }
            }
        }
        file.flush().map_err(io)
    }
}

/// 206 응답이 `offset`부터 이어지는가 (`Content-Range: bytes <offset>-...`)
fn continues_at(response: &reqwest::Response, offset: u64) -> bool {
    response
        .headers()
        .get(reqwest::header::CONTENT_RANGE)
        .and_then(|v| v.to_str().ok())
        .and_then(|v| v.trim().strip_prefix("bytes "))
        .and_then(|v| v.split('-').next())
        .is_some_and(|start| start.trim().parse() == Ok(offset))
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
        let err = download_verified(&crate::tls::http_client_builder().build().unwrap(), &url, &dest, None, None, None).await.unwrap_err();
        assert!(matches!(err, DownloadError::Http(_)), "{:?}", err);
        assert!(!dest.exists());
    }

    /// 연결마다 `responses`를 차례로 하나씩 보내고 닫는 가짜 서버. 받은 요청 머리를 모아 둔다
    async fn serve_each(responses: Vec<&'static [u8]>) -> (String, Arc<Mutex<Vec<String>>>) {
        use tokio::io::{AsyncReadExt, AsyncWriteExt};
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();
        let seen = Arc::new(Mutex::new(Vec::new()));
        let requests = seen.clone();
        tokio::spawn(async move {
            for response in responses {
                let (mut sock, _) = listener.accept().await.unwrap();
                let mut buf = [0u8; 2048];
                let n = sock.read(&mut buf).await.unwrap_or(0);
                requests.lock().unwrap().push(String::from_utf8_lossy(&buf[..n]).to_lowercase());
                let _ = sock.write_all(response).await;
            }
        });
        (format!("http://{}/asset.zip", addr), seen)
    }

    fn sha256_hex(bytes: &[u8]) -> String {
        Sha256::digest(bytes).iter().map(|b| format!("{:02x}", b)).collect()
    }

    const QUICK: Retry = Retry { idle: Duration::from_secs(5), retries: 3, backoff: Duration::from_millis(10) };

    async fn fetch_quick(url: &str, dest: &Path, expected: &str) -> Result<String, DownloadError> {
        let client = crate::tls::http_client_builder().build().unwrap();
        download_with(&client, url, dest, Some(expected), None, None, QUICK).await
    }

    /// 느린 회선에서 큰 파일이 끊겨도 처음부터 다시 받지 않는다: 받은 데서 이어 받고, 해시는 전체로 확인한다
    #[tokio::test]
    async fn a_dropped_connection_resumes_from_where_it_stopped() {
        let (url, seen) = serve_each(vec![
            b"HTTP/1.1 200 OK\r\nContent-Length: 20\r\n\r\n0123456789",
            b"HTTP/1.1 503 Service Unavailable\r\nContent-Length: 0\r\n\r\n",
            b"HTTP/1.1 206 Partial Content\r\nContent-Range: bytes 10-19/20\r\nContent-Length: 10\r\n\r\nabcdefghij",
        ])
        .await;
        let tmp = tempfile::tempdir().unwrap();
        let dest = tmp.path().join("asset.zip");
        let whole = b"0123456789abcdefghij";
        assert_eq!(fetch_quick(&url, &dest, &sha256_hex(whole)).await.unwrap(), sha256_hex(whole));
        assert_eq!(std::fs::read(&dest).unwrap(), whole);
        let seen = seen.lock().unwrap();
        assert!(!seen[0].contains("range:"), "{}", seen[0]);
        // 사이에 서버가 잠깐 실패해도 받아 둔 것은 버리지 않는다
        assert!(seen[1].contains("range: bytes=10-"), "{}", seen[1]);
        assert!(seen[2].contains("range: bytes=10-"), "{}", seen[2]);
        assert!(!tmp.path().join("asset.zip.part").exists());
    }

    /// Range를 모르는 서버가 처음부터 다시 보내면 앞서 받은 것에 덧붙이지 않는다
    #[tokio::test]
    async fn a_server_that_ignores_range_is_read_from_the_start() {
        let (url, _) = serve_each(vec![
            b"HTTP/1.1 200 OK\r\nContent-Length: 20\r\n\r\n0123456789",
            b"HTTP/1.1 200 OK\r\nContent-Length: 20\r\n\r\n0123456789abcdefghij",
        ])
        .await;
        let tmp = tempfile::tempdir().unwrap();
        let dest = tmp.path().join("asset.zip");
        let whole = b"0123456789abcdefghij";
        fetch_quick(&url, &dest, &sha256_hex(whole)).await.unwrap();
        assert_eq!(std::fs::read(&dest).unwrap(), whole);
    }

    #[tokio::test]
    async fn a_server_error_is_tried_again_but_a_missing_file_is_not() {
        let (url, seen) = serve_each(vec![
            b"HTTP/1.1 503 Service Unavailable\r\nContent-Length: 0\r\n\r\n",
            b"HTTP/1.1 200 OK\r\nContent-Length: 4\r\n\r\nbody",
        ])
        .await;
        let tmp = tempfile::tempdir().unwrap();
        let dest = tmp.path().join("asset.zip");
        fetch_quick(&url, &dest, &sha256_hex(b"body")).await.unwrap();
        assert_eq!(seen.lock().unwrap().len(), 2);

        let (url, seen) = serve_each(vec![
            b"HTTP/1.1 404 Not Found\r\nContent-Length: 0\r\n\r\n",
            b"HTTP/1.1 200 OK\r\nContent-Length: 4\r\n\r\nbody",
        ])
        .await;
        let dest = tmp.path().join("missing.zip");
        assert!(fetch_quick(&url, &dest, &sha256_hex(b"body")).await.is_err());
        assert_eq!(seen.lock().unwrap().len(), 1, "a 404 is final");
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
        let client = crate::tls::http_client_builder().build().unwrap();
        let fut = download_with(&client, url, dest, None, None, None, Retry { idle, retries: 0, backoff: Duration::ZERO });
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
        let err = download_verified(&crate::tls::http_client_builder().build().unwrap(), &url, &dest, None, Some(10), None).await.unwrap_err();
        assert!(matches!(err, DownloadError::TooLarge { limit: 10 }), "{:?}", err);
        assert!(!dest.exists());
    }

    #[tokio::test]
    async fn a_failed_download_keeps_the_file_already_there() {
        let url = serve_once(b"HTTP/1.1 200 OK\r\nContent-Length: 100\r\n\r\nonly ten b").await;
        let tmp = tempfile::tempdir().unwrap();
        let dest = tmp.path().join("asset.zip");
        std::fs::write(&dest, b"good staged file").unwrap();
        assert!(download_verified(&crate::tls::http_client_builder().build().unwrap(), &url, &dest, None, None, None).await.is_err());
        assert_eq!(std::fs::read(&dest).unwrap(), b"good staged file");
        assert!(!tmp.path().join("asset.zip.part").exists());
    }

    #[tokio::test]
    async fn an_error_status_is_not_saved() {
        let url = serve_once(b"HTTP/1.1 404 Not Found\r\nContent-Length: 0\r\n\r\n").await;
        let tmp = tempfile::tempdir().unwrap();
        let dest = tmp.path().join("asset.zip");
        assert!(download_verified(&crate::tls::http_client_builder().build().unwrap(), &url, &dest, None, None, None).await.is_err());
        assert!(!dest.exists());
    }
}
