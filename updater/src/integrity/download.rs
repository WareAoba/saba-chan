//! 스트리밍 다운로드 + SHA-256 검증 일체화
//!
//! 실패 시 대상 파일을 자동 삭제하여 "반쯤 쓰인 파일"을 방지합니다.

use std::path::Path;

use futures_util::StreamExt;
use sha2::{Digest, Sha256};
use tokio::io::AsyncWriteExt;

use super::hash::{FileHash, IntegrityError};

/// 다운로드 진행률 콜백 — `(downloaded_bytes, total_bytes)`.
/// `total_bytes = 0`이면 Content-Length가 없음을 의미.
pub type ProgressCallback = Box<dyn FnMut(u64, u64) + Send>;

/// URL에서 파일을 다운로드하고 SHA-256을 검증합니다.
///
/// - `expected = Some(h)`: 다운로드 직후 해시 비교. 불일치 시 파일 삭제 + `Mismatch` 에러.
/// - `expected = None`: 검증 스킵. 실제 해시만 반환 (개발 빌드 허용).
/// - 실패 시 `dest` 파일은 자동 삭제됩니다.
pub async fn download_verified(
    client: &reqwest::Client,
    url: &str,
    dest: &Path,
    expected: Option<&FileHash>,
    mut progress: Option<ProgressCallback>,
) -> Result<FileHash, IntegrityError> {
    if let Some(parent) = dest.parent() {
        tokio::fs::create_dir_all(parent).await?;
    }

    let resp = client
        .get(url)
        .send()
        .await
        .map_err(|e| IntegrityError::Download(format!("요청 실패 {}: {}", url, e)))?;

    if !resp.status().is_success() {
        return Err(IntegrityError::Download(format!(
            "HTTP {} for {}",
            resp.status(),
            url
        )));
    }

    let total = resp.content_length().unwrap_or(0);
    if let Some(cb) = progress.as_mut() {
        cb(0, total);
    }

    let mut file = tokio::fs::File::create(dest).await?;
    let mut hasher = Sha256::new();
    let mut downloaded: u64 = 0;

    let mut stream = resp.bytes_stream();
    let result: Result<(), IntegrityError> = async {
        while let Some(chunk) = stream.next().await {
            let bytes = chunk.map_err(|e| {
                IntegrityError::Download(format!("스트림 읽기 실패: {}", e))
            })?;
            hasher.update(&bytes);
            file.write_all(&bytes).await?;
            downloaded += bytes.len() as u64;
            if let Some(cb) = progress.as_mut() {
                cb(downloaded, total);
            }
        }
        file.flush().await?;
        file.sync_all().await?;
        Ok(())
    }
    .await;

    if let Err(e) = result {
        let _ = tokio::fs::remove_file(dest).await;
        return Err(e);
    }

    let digest = hasher.finalize();
    let mut arr = [0u8; 32];
    arr.copy_from_slice(&digest);
    let actual = FileHash::from_bytes(arr);

    if let Some(exp) = expected {
        if &actual != exp {
            let _ = tokio::fs::remove_file(dest).await;
            return Err(IntegrityError::Mismatch {
                expected: exp.to_hex_lower(),
                actual: actual.to_hex_lower(),
                file: dest.to_path_buf(),
            });
        }
    }

    Ok(actual)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::integrity::hash::hash_bytes;
    use std::sync::{Arc, Mutex};

    async fn start_mock_server(
        body: Vec<u8>,
        status: u16,
    ) -> (String, tokio::task::JoinHandle<()>) {
        use tokio::net::TcpListener;
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();
        let url = format!("http://{}/file.bin", addr);
        let handle = tokio::spawn(async move {
            if let Ok((mut stream, _)) = listener.accept().await {
                use tokio::io::{AsyncReadExt, AsyncWriteExt};
                let mut buf = vec![0u8; 4096];
                let _ = stream.read(&mut buf).await;
                let status_line = match status {
                    200 => "HTTP/1.1 200 OK",
                    404 => "HTTP/1.1 404 Not Found",
                    _ => "HTTP/1.1 500 Internal Server Error",
                };
                let resp = format!(
                    "{}\r\nContent-Length: {}\r\nContent-Type: application/octet-stream\r\nConnection: close\r\n\r\n",
                    status_line,
                    body.len()
                );
                let _ = stream.write_all(resp.as_bytes()).await;
                let _ = stream.write_all(&body).await;
                let _ = stream.shutdown().await;
            }
        });
        (url, handle)
    }

    #[tokio::test]
    async fn download_verified_success() {
        let body = b"hello world".to_vec();
        let expected = hash_bytes(&body);
        let (url, _h) = start_mock_server(body.clone(), 200).await;

        let dir = tempfile::tempdir().unwrap();
        let dest = dir.path().join("out.bin");
        let client = reqwest::Client::new();
        let actual = download_verified(&client, &url, &dest, Some(&expected), None)
            .await
            .unwrap();
        assert_eq!(actual, expected);
        assert_eq!(std::fs::read(&dest).unwrap(), body);
    }

    #[tokio::test]
    async fn download_verified_mismatch_deletes_file() {
        let body = b"tampered".to_vec();
        let wrong = hash_bytes(b"original");
        let (url, _h) = start_mock_server(body, 200).await;

        let dir = tempfile::tempdir().unwrap();
        let dest = dir.path().join("out.bin");
        let client = reqwest::Client::new();
        let err = download_verified(&client, &url, &dest, Some(&wrong), None)
            .await
            .unwrap_err();
        assert!(matches!(err, IntegrityError::Mismatch { .. }));
        assert!(!dest.exists(), "failed download must be deleted");
    }

    #[tokio::test]
    async fn download_verified_no_expected_returns_hash() {
        let body = b"raw".to_vec();
        let expected = hash_bytes(&body);
        let (url, _h) = start_mock_server(body, 200).await;

        let dir = tempfile::tempdir().unwrap();
        let dest = dir.path().join("out.bin");
        let client = reqwest::Client::new();
        let actual = download_verified(&client, &url, &dest, None, None)
            .await
            .unwrap();
        assert_eq!(actual, expected);
    }

    #[tokio::test]
    async fn download_verified_http_error_deletes_file() {
        let (url, _h) = start_mock_server(b"not found".to_vec(), 404).await;
        let dir = tempfile::tempdir().unwrap();
        let dest = dir.path().join("out.bin");
        let client = reqwest::Client::new();
        let err = download_verified(&client, &url, &dest, None, None)
            .await
            .unwrap_err();
        assert!(matches!(err, IntegrityError::Download(_)));
        assert!(!dest.exists());
    }

    #[tokio::test]
    async fn download_verified_with_progress() {
        let body = vec![0u8; 1024];
        let expected = hash_bytes(&body);
        let (url, _h) = start_mock_server(body.clone(), 200).await;

        let dir = tempfile::tempdir().unwrap();
        let dest = dir.path().join("out.bin");
        let client = reqwest::Client::new();
        let last: Arc<Mutex<(u64, u64)>> = Arc::new(Mutex::new((0, 0)));
        let last_clone = Arc::clone(&last);
        let cb: ProgressCallback = Box::new(move |d, t| {
            *last_clone.lock().unwrap() = (d, t);
        });
        let _ = download_verified(&client, &url, &dest, Some(&expected), Some(cb))
            .await
            .unwrap();
        let (downloaded, total) = *last.lock().unwrap();
        assert_eq!(downloaded, 1024);
        assert_eq!(total, 1024);
    }
}