//! 로컬 릴레이의 서명 검증과 자산 다운로드 파이프라인을 검증한다.

use std::io::{Read, Write};
use std::net::TcpListener;

use ed25519_dalek::{Signer, SigningKey};
use saba_chan_updater_lib::relay::{Channel, RelayClient};
use saba_chan_updater_lib::signature::{self, PublicKeyRegistry};
use sha2::{Digest, Sha256};

// ─────────────────────────────────────────────────────────────
//  Mock HTTP 서버 (단일 요청/응답, 순차 처리)
// ─────────────────────────────────────────────────────────────
struct MockServer {
    port: u16,
    _handle: std::thread::JoinHandle<()>,
}

impl MockServer {
    fn start(responses: Vec<(u16, String, String)>) -> Self {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let port = listener.local_addr().unwrap().port();
        let handle = std::thread::spawn(move || {
            for (status, body, ctype) in responses {
                let (mut stream, _) = match listener.accept() {
                    Ok(x) => x,
                    Err(_) => return,
                };
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
            }
        });
        Self { port, _handle: handle }
    }

    fn base(&self) -> String {
        format!("http://127.0.0.1:{}", self.port)
    }
}

fn test_signing_key() -> SigningKey {
    SigningKey::from_bytes(&[0x5a; 32])
}

fn test_public_keys(key: &SigningKey) -> PublicKeyRegistry {
    PublicKeyRegistry::from_entries(vec![("test-updater-v1".into(), key.verifying_key())])
}

/// manifest JSON 에 서명하고 `SignedManifest` JSON 반환.
fn sign_manifest(sk: &SigningKey, manifest: &serde_json::Value) -> String {
    use base64::Engine as _;
    let canonical = signature::canonical_json_bytes(manifest).expect("canonical json");
    let sig = sk.sign(&canonical);
    let sig_b64 = base64::engine::general_purpose::STANDARD.encode(sig.to_bytes());
    let signed = serde_json::json!({
        "manifest": manifest,
        "signature": sig_b64,
        "keyId": "test-updater-v1",
    });
    serde_json::to_string(&signed).unwrap()
}

fn sha256_hex(bytes: &[u8]) -> String {
    let mut h = Sha256::new();
    h.update(bytes);
    hex::encode(h.finalize())
}

// ─────────────────────────────────────────────────────────────
//  테스트
// ─────────────────────────────────────────────────────────────

/// `RelayClient::check()` + `fetch_manifest()` round-trip.
///
/// 로컬 테스트 키로 서명한 매니페스트를 검증한다.
#[tokio::test]
async fn relay_check_fetch_and_download_with_test_key() {
    let sk = test_signing_key();

    let check_body = serde_json::json!({
        "upToDate": false,
        "latest": {
            "version": "0.9.0-test1",
            "channel": "beta",
            "isCritical": false,
            "releasedAt": "2026-04-24T07:00:00Z",
            "minSupported": null,
            "manifestUrl": "http://placeholder/ignored",
            "notes": null,
        }
    })
    .to_string();

    let payload = "local signed update payload";
    let asset_server = MockServer::start(vec![(200, payload.into(), "application/octet-stream".into())]);

    let manifest = serde_json::json!({
        "version": "0.9.0-test1",
        "channel": "beta",
        "platform": "windows-x64",
        "components": {
            "saba-core": {
                "version": "0.9.0-test1",
                "asset": "saba-core-windows-x64.zip",
                "url": format!("{}/asset.zip", asset_server.base()),
                "sha256": sha256_hex(payload.as_bytes()),
                "size_bytes": payload.len()
            }
        }
    });
    let signed_json = sign_manifest(&sk, &manifest);

    let server = MockServer::start(vec![
        (200, check_body, "application/json".to_string()),
        (200, signed_json, "application/json".to_string()),
    ]);

    let client = RelayClient::new(&server.base());

    // ── 1. check ──
    let check_resp = client
        .check(Channel::Beta, "windows-x64", "0.8.0", "test_client_0000")
        .await
        .expect("check 실패");
    assert!(!check_resp.up_to_date);
    let latest = check_resp.latest.expect("latest 없음");
    assert_eq!(latest.version, "0.9.0-test1");
    assert_eq!(latest.channel, "beta");

    // ── 2. fetch_manifest + 서명 검증 ──
    let keys = test_public_keys(&sk);
    assert!(!keys.is_empty(), "테스트 공개키 레지스트리가 비어 있음");
    let verified_manifest = client
        .fetch_manifest("0.9.0-test1", &keys)
        .await
        .expect("fetch_manifest 실패");

    assert_eq!(
        verified_manifest.get("version").and_then(|v| v.as_str()),
        Some("0.9.0-test1")
    );
    let url = verified_manifest["components"]["saba-core"]["url"]
        .as_str()
        .unwrap()
        .to_string();
    let expected_sha = verified_manifest["components"]["saba-core"]["sha256"]
        .as_str()
        .unwrap()
        .to_string();
    let expected_size = verified_manifest["components"]["saba-core"]["size_bytes"]
        .as_u64()
        .unwrap();

    // ── 3. 로컬 자산 다운로드 + SHA256 검증 ──
    eprintln!("→ downloading {}", url);
    let http = reqwest::Client::builder()
        .timeout(std::time::Duration::from_secs(30))
        .build()
        .unwrap();
    let resp = http.get(&url).send().await.expect("local asset download" );
    assert!(resp.status().is_success(), "로컬 다운로드 실패: {}", resp.status());
    let bytes = resp.bytes().await.expect("본문 읽기 실패");

    assert_eq!(
        bytes.len() as u64,
        expected_size,
        "바이너리 크기가 매니페스트와 다름"
    );
    let actual_sha = sha256_hex(&bytes);
    assert_eq!(
        actual_sha, expected_sha,
        "다운로드된 바이너리의 SHA256이 매니페스트와 다름"
    );
    eprintln!("✓ 다운로드 완료: {} bytes, sha256 일치", bytes.len());
}

/// 변조된 매니페스트는 검증 실패해야 한다 (downgrade/치환 방어).
#[tokio::test]
async fn relay_fetch_manifest_tampered_is_rejected() {
    let sk = test_signing_key();

    let manifest = serde_json::json!({
        "version": "0.9.0-test1",
        "channel": "beta",
        "components": {}
    });
    let signed = sign_manifest(&sk, &manifest);

    // 서명 유지하면서 manifest 를 변조
    let mut v: serde_json::Value = serde_json::from_str(&signed).unwrap();
    v["manifest"]["version"] = serde_json::Value::String("9.9.9-evil".into());
    let tampered = serde_json::to_string(&v).unwrap();

    let server = MockServer::start(vec![(200, tampered, "application/json".to_string())]);
    let client = RelayClient::new(&server.base());
    let keys = test_public_keys(&sk);
    let result = client.fetch_manifest("9.9.9-evil", &keys).await;
    assert!(
        result.is_err(),
        "변조된 매니페스트가 검증 통과 (있으면 안 됨)"
    );
}
