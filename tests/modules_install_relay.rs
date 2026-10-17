//! 모듈 신규 설치 E2E — Relay 서명 매니페스트 + R2 mock 다운로드

use std::io::{Read, Write};
use std::net::TcpListener;
use std::path::PathBuf;

use ed25519_dalek::{Signer, SigningKey};
use saba_chan_updater_lib::signature::{self};
use saba_chan_updater_lib::{Component, UpdateConfig, UpdateManager};
use sha2::{Digest, Sha256};

enum MockResponse {
    Json(String),
    Bytes(Vec<u8>, &'static str),
}

struct MockServer {
    port: u16,
    _handle: std::thread::JoinHandle<()>,
}

impl MockServer {
    fn start(responses: Vec<MockResponse>) -> Self {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let port = listener.local_addr().unwrap().port();
        let handle = std::thread::spawn(move || {
            for resp in responses {
                let (mut stream, _) = match listener.accept() {
                    Ok(x) => x,
                    Err(_) => return,
                };
                let mut buf = [0u8; 8192];
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
                let (body, ctype): (Vec<u8>, &str) = match resp {
                    MockResponse::Json(s) => (s.into_bytes(), "application/json"),
                    MockResponse::Bytes(b, c) => (b, c),
                };
                let header = format!(
                    "HTTP/1.1 200 OK\r\nContent-Type: {}\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
                    ctype,
                    body.len()
                );
                let _ = stream.write_all(header.as_bytes());
                let _ = stream.write_all(&body);
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
    SigningKey::from_bytes(&[42; 32])
}

fn sign_manifest(sk: &SigningKey, manifest: &serde_json::Value) -> String {
    use base64::Engine as _;
    let canonical = signature::canonical_json_bytes(manifest).expect("canonical json");
    let sig = sk.sign(&canonical);
    let sig_b64 = base64::engine::general_purpose::STANDARD.encode(sig.to_bytes());
    serde_json::to_string(&serde_json::json!({
        "manifest": manifest,
        "signature": sig_b64,
        "keyId": "test-key",
    }))
    .unwrap()
}

fn sha256_hex(bytes: &[u8]) -> String {
    let mut h = Sha256::new();
    h.update(bytes);
    hex::encode(h.finalize())
}

fn build_test_zip() -> Vec<u8> {
    let buf = std::io::Cursor::new(Vec::new());
    let mut zw = zip::ZipWriter::new(buf);
    let opts =
        zip::write::FileOptions::default().compression_method(zip::CompressionMethod::Stored);
    zw.start_file("module.toml", opts).unwrap();
    zw.write_all(b"[module]\nname = \"testmod\"\nversion = \"1.0.0\"\nentry = \"lifecycle.py\"\n")
        .unwrap();
    zw.start_file("lifecycle.py", opts).unwrap();
    zw.write_all(b"# test module lifecycle\n").unwrap();
    zw.finish().unwrap().into_inner()
}

fn make_check_json(version: &str) -> String {
    serde_json::json!({
        "upToDate": false,
        "latest": {
            "version": version,
            "channel": "stable",
            "isCritical": false,
            "releasedAt": "2026-04-24T00:00:00Z",
            "minSupported": null,
            "manifestUrl": "http://placeholder/ignored",
            "notes": null,
        }
    })
    .to_string()
}

#[tokio::test]
async fn install_module_via_relay_succeeds_with_valid_sha256() {
    let sk = test_signing_key();

    let zip_bytes = build_test_zip();
    let zip_sha = sha256_hex(&zip_bytes);
    let zip_size = zip_bytes.len() as u64;

    let asset_server = MockServer::start(vec![MockResponse::Bytes(
        zip_bytes.clone(),
        "application/zip",
    )]);
    let asset_url = format!(
        "http://127.0.0.1:{}/module-goodmod.zip",
        asset_server.port
    );

    let manifest = serde_json::json!({
        "version": "1.0.0",
        "channel": "stable",
        "platform": "windows-x64",
        "components": {
            "module-goodmod": {
                "version": "1.0.0",
                "asset": "module-goodmod.zip",
                "url": asset_url,
                "sha256": zip_sha,
                "size_bytes": zip_size,
                "install_dir": "goodmod",
            }
        }
    });
    let signed_json = sign_manifest(&sk, &manifest);

    let relay_server = MockServer::start(vec![
        MockResponse::Json(make_check_json("1.0.0")),
        MockResponse::Json(signed_json),
    ]);

    let tmp = tempfile::tempdir().unwrap();
    let modules_dir = tmp.path().join("modules");
    std::fs::create_dir_all(&modules_dir).unwrap();

    let cfg = UpdateConfig {
        relay_base_url: relay_server.base(),
        channel: "stable".into(),
        platform: "windows-x64".into(),
        client_id: "test_client_0000".into(),
        install_root: Some(tmp.path().to_string_lossy().to_string()),
        ..Default::default()
    };
    let mut mgr = UpdateManager::new(cfg, &modules_dir.to_string_lossy()).with_public_keys(
        signature::PublicKeyRegistry::from_entries(vec![("test-key".into(), sk.verifying_key())]),
    );

    let result = mgr
        .install_component(&Component::Module("goodmod".into()))
        .await;
    assert!(result.is_ok(), "설치 실패: {:?}", result.err());

    let install_path: PathBuf = result.unwrap().into();
    assert!(install_path.exists(), "install_dir 미생성: {:?}", install_path);
    assert!(install_path.join("module.toml").exists(), "module.toml 없음");
    assert!(install_path.join("lifecycle.py").exists(), "lifecycle.py 없음");
}

#[tokio::test]
async fn install_module_rejects_mismatched_sha256() {
    let sk = test_signing_key();

    let real_zip = build_test_zip();
    let real_sha = sha256_hex(&real_zip);
    let real_size = real_zip.len() as u64;

    let tampered_zip = {
        let mut v = real_zip.clone();
        v[0] ^= 0xFF;
        v
    };

    let asset_server = MockServer::start(vec![MockResponse::Bytes(
        tampered_zip,
        "application/zip",
    )]);
    let asset_url = format!(
        "http://127.0.0.1:{}/module-badmod.zip",
        asset_server.port
    );

    let manifest = serde_json::json!({
        "version": "1.0.0",
        "channel": "stable",
        "platform": "windows-x64",
        "components": {
            "module-badmod": {
                "version": "1.0.0",
                "asset": "module-badmod.zip",
                "url": asset_url,
                "sha256": real_sha,
                "size_bytes": real_size,
                "install_dir": "badmod",
            }
        }
    });
    let signed_json = sign_manifest(&sk, &manifest);

    let relay_server = MockServer::start(vec![
        MockResponse::Json(make_check_json("1.0.0")),
        MockResponse::Json(signed_json),
    ]);

    let tmp = tempfile::tempdir().unwrap();
    let modules_dir = tmp.path().join("modules");
    std::fs::create_dir_all(&modules_dir).unwrap();

    let cfg = UpdateConfig {
        relay_base_url: relay_server.base(),
        channel: "stable".into(),
        platform: "windows-x64".into(),
        client_id: "test_client_0000".into(),
        install_root: Some(tmp.path().to_string_lossy().to_string()),
        ..Default::default()
    };
    let mut mgr = UpdateManager::new(cfg, &modules_dir.to_string_lossy()).with_public_keys(
        signature::PublicKeyRegistry::from_entries(vec![("test-key".into(), sk.verifying_key())]),
    );

    let result = mgr
        .install_component(&Component::Module("badmod".into()))
        .await;
    assert!(
        result.is_err(),
        "변조된 ZIP 이 거부되지 않음 — SHA256 검증 실패"
    );
    let msg = result.unwrap_err().to_string();
    assert!(
        msg.contains("해시") || msg.to_lowercase().contains("sha") || msg.contains("검증"),
        "에러 메시지가 해시/검증 실패를 표시해야 함: {}",
        msg
    );

    let install_path = tmp.path().join("badmod");
    assert!(
        !install_path.exists()
            || std::fs::read_dir(&install_path)
                .map(|d| d.count() == 0)
                .unwrap_or(true),
        "검증 실패 후에도 파일이 남아있음: {:?}",
        install_path
    );
}
