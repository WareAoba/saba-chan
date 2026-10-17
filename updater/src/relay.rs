//! 릴레이 기반 업데이트 API 클라이언트
//!
//! 서명된 manifest를 취득·검증한다.
//!
//! 엔드포인트:
//! - `GET  /v1/updates/check`
//! - `GET  /v1/updates/manifest/:version`
//!
//! 본 모듈은 **네트워크 호출과 서명 검증만** 책임진다.
//! 다운로드·적용 등 상위 정책은 `UpdateManager` 가 담당.

use serde::{Deserialize, Serialize};
use thiserror::Error;

use crate::signature::{
    self, PublicKeyRegistry, SignatureError, SignedManifest,
};

/// 업데이트 채널
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum Channel {
    Stable,
    Beta,
    Nightly,
}

impl Channel {
    pub fn as_str(&self) -> &'static str {
        match self {
            Channel::Stable => "stable",
            Channel::Beta => "beta",
            Channel::Nightly => "nightly",
        }
    }
}

#[derive(Debug, Error)]
pub enum RelayError {
    #[error("HTTP 오류: {0}")]
    Http(String),
    #[error("릴레이 응답 파싱 실패: {0}")]
    ParseResponse(String),
    #[error("서명 검증 실패: {0}")]
    Signature(#[from] SignatureError),
    #[error("상태 코드 {status}: {body}")]
    Status { status: u16, body: String },
}

// ─────────────────────────────────────────────────────────────
//  응답 타입
// ─────────────────────────────────────────────────────────────

#[derive(Debug, Clone, Deserialize)]
pub struct CheckResponse {
    #[serde(rename = "upToDate", default)]
    pub up_to_date: bool,
    #[serde(default)]
    pub latest: Option<LatestInfo>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct LatestInfo {
    pub version: String,
    pub channel: String,
    #[serde(rename = "releasedAt", default)]
    pub released_at: Option<String>,
    #[serde(default)]
    pub notes: Option<serde_json::Value>,
}

// ─────────────────────────────────────────────────────────────
//  RelayClient
// ─────────────────────────────────────────────────────────────

/// 릴레이 업데이트 API 클라이언트.
///
/// `base_url`은 끝에 슬래시가 있어도 없어도 되도록 정규화한다.
pub struct RelayClient {
    base_url: String,
    http: reqwest::Client,
}

impl RelayClient {
    pub fn new(base_url: &str) -> Self {
        Self {
            base_url: base_url.trim_end_matches('/').to_string(),
            http: reqwest::Client::builder()
                .user_agent(concat!("saba-chan-updater/", env!("CARGO_PKG_VERSION")))
                .build()
                .unwrap_or_else(|_| reqwest::Client::new()),
        }
    }

    fn url(&self, path: &str) -> String {
        format!("{}{}", self.base_url, path)
    }

    /// `GET /v1/updates/check`
    pub async fn check(
        &self,
        channel: Channel,
        platform: &str,
        current: &str,
        client_id: &str,
    ) -> Result<CheckResponse, RelayError> {
        let url = self.url("/v1/updates/check");
        let resp = self
            .http
            .get(&url)
            .header("x-saba-client", "1")
            .query(&[
                ("channel", channel.as_str()),
                ("platform", platform),
                ("current", current),
                ("client_id", client_id),
            ])
            .send()
            .await
            .map_err(|e| RelayError::Http(e.to_string()))?;

        let status = resp.status();
        let body = resp
            .text()
            .await
            .map_err(|e| RelayError::Http(e.to_string()))?;
        if !status.is_success() {
            return Err(RelayError::Status {
                status: status.as_u16(),
                body,
            });
        }

        serde_json::from_str::<CheckResponse>(&body)
            .map_err(|e| RelayError::ParseResponse(format!("{}: {}", e, body)))
    }

    /// 서명된 manifest를 fetch하고 검증까지 수행한다.
    ///
    /// 서명 검증 후 manifest의 `version`(또는 별칭 `release_version`)이 요청한
    /// 버전과 같은지 확인한다. 버전 필드가 없으면 거부한다 (치환 공격 방어).
    pub async fn fetch_manifest(
        &self,
        version: &str,
        keys: &PublicKeyRegistry,
    ) -> Result<serde_json::Value, RelayError> {
        let url = self.url(&format!("/v1/updates/manifest/{}", urlencode(version)));
        let resp = self
            .http
            .get(&url)
            .header("x-saba-client", "1")
            .send()
            .await
            .map_err(|e| RelayError::Http(e.to_string()))?;
        let status = resp.status();
        let body = resp
            .text()
            .await
            .map_err(|e| RelayError::Http(e.to_string()))?;
        if !status.is_success() {
            return Err(RelayError::Status {
                status: status.as_u16(),
                body,
            });
        }

        let signed: SignedManifest = serde_json::from_str(&body)
            .map_err(|e| RelayError::ParseResponse(format!("{}: {}", e, body)))?;
        let manifest = signature::verify_signed_manifest(&signed, keys)?;

        check_manifest_version(&manifest, version)?;
        Ok(manifest)
    }

}

/// 서명된 manifest 의 `version` / `release_version` 이 요청 버전과 같은지 확인한다.
/// 두 필드가 모두 있으면 둘 다 일치해야 한다.
fn check_manifest_version(manifest: &serde_json::Value, requested: &str) -> Result<(), RelayError> {
    let fields: Vec<(&str, &serde_json::Value)> = ["version", "release_version"]
        .iter()
        .filter_map(|name| manifest.get(*name).map(|v| (*name, v)))
        .collect();
    if fields.is_empty() {
        return Err(RelayError::ParseResponse("manifest 에 version 필드가 없음".to_string()));
    }
    for (name, value) in fields {
        if value.as_str() != Some(requested) {
            return Err(RelayError::ParseResponse(format!(
                "manifest.{} {} != 요청 version '{}'",
                name, value, requested
            )));
        }
    }
    Ok(())
}

/// URL 경로 세그먼트 인코딩 (RFC 3986 unreserved 외 문자는 UTF-8 바이트 단위 %XX).
fn urlencode(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    for byte in s.bytes() {
        match byte {
            b'a'..=b'z' | b'A'..=b'Z' | b'0'..=b'9' | b'.' | b'-' | b'_' | b'~' => out.push(byte as char),
            _ => out.push_str(&format!("%{:02X}", byte)),
        }
    }
    out
}

// ═══════════════════════════════════════════════════════════════
#[cfg(test)]
mod tests {
    use super::*;
    use crate::signature::SignedManifest;
    use ed25519_dalek::{Signer, SigningKey};
    use serde_json::json;
    use std::io::{Read, Write};
    use std::net::TcpListener;

    // ── Mock HTTP 서버 ───────────────────────────────────────
    //
    // integrity/download 테스트와 같은 Mock TCP 기반. 단일 요청을 받고
    // 사전 정의된 응답(status + body + content-type)을 돌려준다.

    struct MockServer {
        port: u16,
        handle: std::thread::JoinHandle<()>,
    }

    impl MockServer {
        fn start(
            responses: Vec<(u16, String, String)>, // (status, body, content-type)
        ) -> Self {
            let listener = TcpListener::bind("127.0.0.1:0").unwrap();
            let port = listener.local_addr().unwrap().port();

            let handle = std::thread::spawn(move || {
                for (i, (status, body, ctype)) in responses.into_iter().enumerate() {
                    let (mut stream, _) = match listener.accept() {
                        Ok(x) => x,
                        Err(_) => return,
                    };
                    // 요청 헤더 끝까지 읽기 (매우 단순한 파서)
                    let mut buf = [0u8; 4096];
                    let mut total = 0usize;
                    loop {
                        let n = match stream.read(&mut buf[total..]) {
                            Ok(0) => break,
                            Ok(n) => n,
                            Err(_) => break,
                        };
                        total += n;
                        if buf[..total].windows(4).any(|w| w == b"\r\n\r\n") {
                            break;
                        }
                        if total >= buf.len() {
                            break;
                        }
                    }
                    let reason = match status {
                        200 => "OK",
                        204 => "No Content",
                        400 => "Bad Request",
                        404 => "Not Found",
                        500 => "Internal Server Error",
                        _ => "Response",
                    };
                    let resp = format!(
                        "HTTP/1.1 {} {}\r\nContent-Type: {}\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}",
                        status,
                        reason,
                        ctype,
                    body.len(),
                        body
                    );
                    let _ = stream.write_all(resp.as_bytes());
                    let _ = stream.flush();
                    let _ = i; // unused silencer
                }
            });

            Self { port, handle }
        }

        fn base_url(&self) -> String {
            format!("http://127.0.0.1:{}", self.port)
        }

        fn join(self) {
            let _ = self.handle.join();
        }
    }

    fn make_keypair() -> (SigningKey, ed25519_dalek::VerifyingKey) {
        let seed = [3u8; 32];
        let sk = SigningKey::from_bytes(&seed);
        let vk = sk.verifying_key();
        (sk, vk)
    }

    fn sign_payload(sk: &SigningKey, manifest: &serde_json::Value) -> String {
        let bytes = crate::signature::canonical_json_bytes(manifest).unwrap();
        let sig = sk.sign(&bytes);
        use base64::Engine as _;
        base64::engine::general_purpose::STANDARD.encode(sig.to_bytes())
    }

    // ── check ────────────────────────────────────────────────

    #[tokio::test]
    async fn check_returns_up_to_date() {
        let server = MockServer::start(vec![(
            200,
            r#"{"upToDate":true}"#.to_string(),
            "application/json".to_string(),
        )]);
        let client = RelayClient::new(&server.base_url());
        let resp = client
            .check(Channel::Stable, "windows-x64", "0.7.2", "client-12345678")
            .await
            .unwrap();
        assert!(resp.up_to_date);
        assert!(resp.latest.is_none());
        server.join();
    }

    #[tokio::test]
    async fn check_returns_latest_info() {
        let body = json!({
            "upToDate": false,
            "latest": {
                "version": "0.8.0",
                "channel": "stable",
                "releasedAt": "2026-01-02T00:00:00Z",
                "manifestUrl": "https://relay.example/v1/updates/manifest/0.8.0",
                "notes": { "ko": "보안 패치" }
            }
        });
        let server = MockServer::start(vec![(
            200,
            body.to_string(),
            "application/json".to_string(),
        )]);
        let client = RelayClient::new(&server.base_url());
        let resp = client
            .check(Channel::Stable, "windows-x64", "0.7.0", "client-12345678")
            .await
            .unwrap();
        assert!(!resp.up_to_date);
        let latest = resp.latest.unwrap();
        assert_eq!(latest.version, "0.8.0");
        assert_eq!(latest.released_at.as_deref(), Some("2026-01-02T00:00:00Z"));
        server.join();
    }

    #[tokio::test]
    async fn check_reports_error_status() {
        let server = MockServer::start(vec![(
            400,
            r#"{"error":"INVALID_CHANNEL"}"#.to_string(),
            "application/json".to_string(),
        )]);
        let client = RelayClient::new(&server.base_url());
        let err = client
            .check(Channel::Stable, "windows-x64", "0.7.0", "client-12345678")
            .await
            .unwrap_err();
        match err {
            RelayError::Status { status, body } => {
                assert_eq!(status, 400);
                assert!(body.contains("INVALID_CHANNEL"));
            }
            other => panic!("unexpected: {:?}", other),
        }
        server.join();
    }

    // ── fetch_manifest (서명 검증 포함) ─────────────────────

    #[tokio::test]
    async fn fetch_manifest_verifies_signature_and_version() {
        let (sk, vk) = make_keypair();
        let manifest = json!({
            "version": "0.8.0",
            "channel": "stable",
            "platform": "windows-x64",
            "components": {}
        });
        let sig = sign_payload(&sk, &manifest);
        let signed = SignedManifest {
            manifest: manifest.clone(),
            signature: sig,
            key_id: "v1".to_string(),
        };
        let body = serde_json::to_string(&signed).unwrap();

        let server = MockServer::start(vec![(200, body, "application/json".to_string())]);
        let client = RelayClient::new(&server.base_url());
        let registry = PublicKeyRegistry::from_entries(vec![("v1".to_string(), vk)]);

        let verified = client.fetch_manifest("0.8.0", &registry).await.unwrap();
        assert_eq!(verified, manifest);
        server.join();
    }

    #[tokio::test]
    async fn fetch_manifest_rejects_bad_signature() {
        let (_, vk) = make_keypair();
        // 다른 키로 서명
        let wrong_sk = SigningKey::from_bytes(&[11u8; 32]);
        let manifest = json!({ "version": "0.8.0" });
        let sig = sign_payload(&wrong_sk, &manifest);
        let signed = SignedManifest {
            manifest: manifest.clone(),
            signature: sig,
            key_id: "v1".to_string(),
        };
        let body = serde_json::to_string(&signed).unwrap();
        let server = MockServer::start(vec![(200, body, "application/json".to_string())]);
        let client = RelayClient::new(&server.base_url());
        let registry = PublicKeyRegistry::from_entries(vec![("v1".to_string(), vk)]);

        let err = client.fetch_manifest("0.8.0", &registry).await.unwrap_err();
        assert!(matches!(err, RelayError::Signature(_)));
        server.join();
    }

    #[tokio::test]
    async fn fetch_manifest_rejects_version_mismatch() {
        let (sk, vk) = make_keypair();
        let manifest = json!({ "version": "0.8.1" }); // URL은 0.8.0 요청
        let sig = sign_payload(&sk, &manifest);
        let signed = SignedManifest {
            manifest,
            signature: sig,
            key_id: "v1".to_string(),
        };
        let body = serde_json::to_string(&signed).unwrap();
        let server = MockServer::start(vec![(200, body, "application/json".to_string())]);
        let client = RelayClient::new(&server.base_url());
        let registry = PublicKeyRegistry::from_entries(vec![("v1".to_string(), vk)]);

        let err = client.fetch_manifest("0.8.0", &registry).await.unwrap_err();
        assert!(matches!(err, RelayError::ParseResponse(_)));
        server.join();
    }

    #[tokio::test]
    async fn fetch_manifest_404() {
        let server = MockServer::start(vec![(
            404,
            r#"{"error":"RELEASE_NOT_FOUND"}"#.to_string(),
            "application/json".to_string(),
        )]);
        let client = RelayClient::new(&server.base_url());
        let registry = PublicKeyRegistry::from_entries(vec![]);

        let err = client.fetch_manifest("9.9.9", &registry).await.unwrap_err();
        assert!(matches!(err, RelayError::Status { status: 404, .. }));
        server.join();
    }

    #[test]
    fn urlencode_basic() {
        assert_eq!(urlencode("0.7.2"), "0.7.2");
        assert_eq!(urlencode("foo bar"), "foo%20bar");
        assert_eq!(urlencode("a/b"), "a%2Fb");
        assert_eq!(urlencode("é"), "%C3%A9");
    }

    #[test]
    fn manifest_version_check_covers_release_version_alias() {
        assert!(check_manifest_version(&json!({ "version": "0.8.0" }), "0.8.0").is_ok());
        assert!(check_manifest_version(&json!({ "release_version": "0.8.0" }), "0.8.0").is_ok());
        assert!(check_manifest_version(&json!({ "release_version": "0.8.1" }), "0.8.0").is_err());
        assert!(check_manifest_version(&json!({ "version": "0.8.0", "release_version": "0.7.0" }), "0.8.0").is_err());
        assert!(check_manifest_version(&json!({ "components": {} }), "0.8.0").is_err());
    }
}
