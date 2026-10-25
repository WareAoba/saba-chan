//! 업데이터 릴레이(R2) 경로 E2E (U9g) — mock 릴레이 + mock R2로 확인 → 서명 검증 → 다운로드
//! → 해시 검증 → 적용 → 설치 버전 기록까지 한 번에 돌린다. 변조된 에셋과 신뢰하지 않는 서명도 확인한다.

use axum::{extract::State, http::{HeaderMap, StatusCode}, response::IntoResponse, routing::get, Json, Router};
use saba_chan_updater_lib::signature;
use saba_chan_updater_lib::{relay, Component, UpdateConfig, UpdateManager};
use serde_json::{json, Value};
use sha2::{Digest, Sha256};
use std::sync::Arc;

/// R2에 올라간 module-good 9.0.0 zip (module.toml 하나)
fn good_asset() -> &'static [u8] {
    static ZIP: std::sync::OnceLock<Vec<u8>> = std::sync::OnceLock::new();
    ZIP.get_or_init(|| {
        use std::io::Write;
        let mut out = zip::ZipWriter::new(std::io::Cursor::new(Vec::new()));
        out.start_file("module.toml", zip::write::SimpleFileOptions::default()).unwrap();
        out.write_all(b"[module]\nname = \"good\"\nversion = \"9.0.0\"\n").unwrap();
        out.finish().unwrap().into_inner()
    })
}

struct Mock {
    base: String,
    signed: Value,
}

/// 운영 릴레이처럼 `X-Saba-Client: 1`이 없으면 거절한다
fn has_client_header(headers: &HeaderMap) -> bool {
    headers.get(relay::CLIENT_HEADER.0).and_then(|v| v.to_str().ok()) == Some(relay::CLIENT_HEADER.1)
}

fn sha256_hex(bytes: &[u8]) -> String {
    hex::encode(Sha256::digest(bytes))
}

async fn start_mock(sign_with: &str, key_id: &str) -> String {
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let base = format!("http://{}", listener.local_addr().unwrap());

    let manifest = json!({
        "version": "9.0.0",
        "channel": "stable",
        "platform": relay::current_platform(),
        "components": {
            "module-good": {
                "version": "9.0.0",
                "asset": "module-good-9.0.0.zip",
                "sha256": sha256_hex(good_asset()),
                "url": format!("{}/r2/module-good-9.0.0.zip", base),
            },
            "module-bad": {
                "version": "9.0.0",
                "asset": "module-bad-9.0.0.zip",
                // R2가 다른 바이트를 준다 (변조·손상 흉내)
                "sha256": sha256_hex(b"what the manifest promised"),
                "url": format!("{}/r2/module-bad-9.0.0.zip", base),
            }
        }
    });
    let signed = serde_json::to_value(signature::sign(manifest, key_id, sign_with).unwrap()).unwrap();
    let state = Arc::new(Mock { base: base.clone(), signed });

    let app = Router::new()
        .route(
            "/v1/updates/check",
            get(|State(m): State<Arc<Mock>>, headers: HeaderMap| async move {
                if !has_client_header(&headers) {
                    return StatusCode::BAD_REQUEST.into_response();
                }
                Json(json!({
                    "upToDate": false,
                    "latest": {
                        "version": "9.0.0",
                        "channel": "stable",
                        "manifestUrl": format!("{}/v1/updates/manifest/9.0.0", m.base),
                        "notes": { "ko": "테스트 릴리즈" }
                    }
                }))
                .into_response()
            }),
        )
        .route(
            "/v1/updates/manifest/9.0.0",
            get(|State(m): State<Arc<Mock>>, headers: HeaderMap| async move {
                if !has_client_header(&headers) {
                    return StatusCode::BAD_REQUEST.into_response();
                }
                Json(m.signed.clone()).into_response()
            }),
        )
        .route("/r2/module-good-9.0.0.zip", get(|| async { good_asset().to_vec() }))
        .route("/r2/module-bad-9.0.0.zip", get(|| async { b"tampered".to_vec() }))
        .with_state(state);
    tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });
    base
}

fn manager(relay_url: &str, modules_dir: &std::path::Path) -> UpdateManager {
    let cfg = UpdateConfig {
        relay_url: Some(relay_url.to_string()),
        ..Default::default()
    };
    UpdateManager::new(cfg, &modules_dir.to_string_lossy())
}

#[tokio::test]
async fn relay_update_cycle_verifies_signature_and_hashes() {
    let (private, public) = signature::generate_key_pair().unwrap();
    std::env::set_var(signature::TEST_PUBKEY_ENV, &public);
    let base = start_mock(&private, signature::TEST_KEY_ID).await;
    let modules = tempfile::tempdir().unwrap();
    // 업데이트 대상은 설치된 모듈뿐이다
    for name in ["good", "bad"] {
        let dir = modules.path().join(name);
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(dir.join("module.toml"), format!("[module]\nname = \"{}\"\nversion = \"1.0.0\"\n", name)).unwrap();
    }
    let mut mgr = manager(&base, modules.path());

    // 1. 확인: 서명된 manifest에서 두 모듈이 나온다
    let status = mgr.check_for_updates().await.expect("relay check");
    let good = status
        .components
        .iter()
        .find(|c| c.component == Component::Module("good".into()))
        .expect("module-good listed");
    assert!(good.update_available);
    assert_eq!(good.latest_version.as_deref(), Some("9.0.0"));
    assert_eq!(good.release_notes.as_deref(), Some("테스트 릴리즈"));

    // 2. 정상 에셋은 받아지고 해시가 맞는다
    let name = mgr.download_component(&Component::Module("good".into())).await.expect("good download");
    let staged = saba_chan_updater_lib::constants::resolve_staging_dir().join(&name);
    assert_eq!(std::fs::read(&staged).unwrap(), good_asset());

    // 3. 해시가 다른 에셋은 거부되고 스테이징에 남지 않는다
    let err = mgr
        .download_component(&Component::Module("bad".into()))
        .await
        .expect_err("tampered asset must be rejected");
    assert!(err.to_string().contains("SHA-256 mismatch"), "{}", err);
    assert!(!saba_chan_updater_lib::constants::resolve_staging_dir().join("module-bad-9.0.0.zip").exists());

    // 4. 적용: 모듈 폴더가 새 버전으로 바뀌고, 설치 버전이 기록되고, 받은 파일은 남지 않는다
    let applied = mgr.apply_components(&["module-good".to_string()]).await.expect("apply");
    assert_eq!(applied.len(), 1, "{:?}", applied);
    let toml = std::fs::read_to_string(modules.path().join("good").join("module.toml")).unwrap();
    assert!(toml.contains("9.0.0"), "{}", toml);
    assert_eq!(UpdateManager::load_installed_manifest().get("module-good").map(String::as_str), Some("9.0.0"));
    let bad = std::fs::read_to_string(modules.path().join("bad").join("module.toml")).unwrap();
    assert!(bad.contains("1.0.0"), "the rejected module stays as it was");
    let good = mgr.get_status().components.into_iter().find(|c| c.component == Component::Module("good".into())).unwrap();
    assert!(!good.update_available && good.current_version == "9.0.0");

    let _ = std::fs::remove_file(staged);
}

#[tokio::test]
async fn manifest_signed_by_unknown_key_is_rejected() {
    let (private, _) = signature::generate_key_pair().unwrap();
    // 번들 키 ID를 사칭하지만 다른 개인키로 서명
    let base = start_mock(&private, signature::TRUSTED_KEYS[0].0).await;
    let modules = tempfile::tempdir().unwrap();
    // 업데이트 대상은 설치된 모듈뿐이다
    for name in ["good", "bad"] {
        let dir = modules.path().join(name);
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(dir.join("module.toml"), format!("[module]\nname = \"{}\"\nversion = \"1.0.0\"\n", name)).unwrap();
    }
    let mut mgr = manager(&base, modules.path());

    let err = mgr.check_for_updates().await.expect_err("forged manifest must fail");
    assert!(err.to_string().contains("signature"), "{}", err);
    assert!(mgr.get_status().components.is_empty());
}
