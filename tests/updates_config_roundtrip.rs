//! ═══════════════════════════════════════════════════════════════════
//! `/api/updates/config` — PUT round-trip 통합 테스트
//! ═══════════════════════════════════════════════════════════════════
//!
//! Stage A Relay 리팩터로 `UpdateConfig` 에서 GitHub 전용 필드는 제거되고
//! `relay_base_url / channel / platform / client_id` 가 추가되었다.
//!
//! 본 테스트는 데몬 IPC 라우터를 직접 실행해(HTTP 바인딩 없이 `tower::oneshot`)
//! 다음을 검증한다:
//!
//! 1. GET `/api/updates/config` 초기 응답이 Relay 필드를 모두 노출한다.
//! 2. PUT `/api/updates/config` 로 Relay 필드를 부분 갱신하면 GET 결과에 반영된다.
//! 3. PUT 요청 본문에 **레거시** 키(`api_base_url`, `github_owner`, `github_repo`)가
//!    포함되어도 Relay 필드는 영향을 받지 않는다.
//!
//! 이 테스트는 `saba-chan-cli` → `saba-chan` 데몬의 PUT 호출 회귀를 방지한다.

use axum::body::{to_bytes, Body};
use axum::http::{Request, StatusCode};
use saba_core::ipc::updates::{updates_router, UpdateState};
use serde_json::{json, Value};
use tower::ServiceExt;

async fn send_json(
    router: axum::Router,
    method: &str,
    uri: &str,
    body: Option<Value>,
) -> (StatusCode, Value) {
    let req_builder = Request::builder().method(method).uri(uri);
    let req = match body {
        Some(v) => req_builder
            .header("content-type", "application/json")
            .body(Body::from(serde_json::to_vec(&v).unwrap()))
            .unwrap(),
        None => req_builder.body(Body::empty()).unwrap(),
    };
    let resp = router.oneshot(req).await.expect("router call failed");
    let status = resp.status();
    let bytes = to_bytes(resp.into_body(), 1024 * 1024)
        .await
        .expect("body read failed");
    let value: Value = if bytes.is_empty() {
        Value::Null
    } else {
        serde_json::from_slice(&bytes).unwrap_or(Value::Null)
    };
    (status, value)
}

#[tokio::test]
async fn get_config_exposes_relay_fields() {
    let router = updates_router(UpdateState::new());
    let (status, body) = send_json(router, "GET", "/api/updates/config", None).await;
    assert_eq!(status, StatusCode::OK);
    let cfg = body.get("config").expect("config 필드 누락");

    // Relay 기반 필드 노출 확인
    assert!(cfg.get("relay_base_url").is_some(), "relay_base_url 누락");
    assert!(cfg.get("channel").is_some(), "channel 누락");
    assert!(cfg.get("platform").is_some(), "platform 누락");
    assert!(cfg.get("client_id").is_some(), "client_id 누락");

    // 레거시 필드는 더 이상 직렬화되지 않아야 한다
    assert!(
        cfg.get("api_base_url").is_none(),
        "api_base_url 이 UpdateConfig 에 남아 있으면 안 됨"
    );
    assert!(
        cfg.get("github_owner").is_none(),
        "github_owner 이 UpdateConfig 에 남아 있으면 안 됨"
    );
    assert!(
        cfg.get("github_repo").is_none(),
        "github_repo 이 UpdateConfig 에 남아 있으면 안 됨"
    );
    assert!(
        cfg.get("include_prerelease").is_none(),
        "include_prerelease 이 UpdateConfig 에 남아 있으면 안 됨"
    );
}

#[tokio::test]
async fn put_config_updates_relay_fields() {
    let router = updates_router(UpdateState::new());

    let payload = json!({
        "relay_base_url": "http://127.0.0.1:9876",
        "channel": "beta",
        "platform": "linux-x64",
        "client_id": "roundtrip-test-01",
        "enabled": false,
        "auto_apply": true
    });

    let (put_status, put_body) =
        send_json(router.clone(), "PUT", "/api/updates/config", Some(payload)).await;
    assert_eq!(put_status, StatusCode::OK, "PUT 실패: {}", put_body);
    assert_eq!(put_body["ok"].as_bool(), Some(true));

    let updated = put_body.get("config").expect("PUT 응답에 config 없음");
    assert_eq!(
        updated["relay_base_url"].as_str(),
        Some("http://127.0.0.1:9876")
    );
    assert_eq!(updated["channel"].as_str(), Some("beta"));
    assert_eq!(updated["platform"].as_str(), Some("linux-x64"));
    assert_eq!(updated["client_id"].as_str(), Some("roundtrip-test-01"));
    assert_eq!(updated["enabled"].as_bool(), Some(false));
    assert_eq!(updated["auto_apply"].as_bool(), Some(true));

    // 후속 GET 이 동일 결과를 반환해야 한다 (상태가 UpdateManager 내부에 반영)
    let (get_status, get_body) = send_json(router, "GET", "/api/updates/config", None).await;
    assert_eq!(get_status, StatusCode::OK);
    let cfg = get_body.get("config").expect("GET 응답에 config 없음");
    assert_eq!(
        cfg["relay_base_url"].as_str(),
        Some("http://127.0.0.1:9876")
    );
    assert_eq!(cfg["channel"].as_str(), Some("beta"));
    assert_eq!(cfg["platform"].as_str(), Some("linux-x64"));
    assert_eq!(cfg["client_id"].as_str(), Some("roundtrip-test-01"));
}

#[tokio::test]
async fn put_config_ignores_legacy_keys() {
    let router = updates_router(UpdateState::new());

    // 이전 버전 클라이언트가 실수로 보낼 수 있는 레거시 필드 — 무시되어야 한다
    let payload = json!({
        "api_base_url": "http://127.0.0.1:9876",
        "github_owner": "someone",
        "github_repo": "saba-chan",
        "include_prerelease": true,
        // 단 하나의 정상 필드만 갱신 대상
        "channel": "nightly"
    });

    let (status, body) = send_json(router, "PUT", "/api/updates/config", Some(payload)).await;
    assert_eq!(status, StatusCode::OK, "PUT 실패: {}", body);
    let cfg = body.get("config").expect("config 없음");

    // 정상 필드만 반영
    assert_eq!(cfg["channel"].as_str(), Some("nightly"));

    // 레거시 필드는 응답 스키마에 나타나지 않아야 한다
    assert!(cfg.get("api_base_url").is_none());
    assert!(cfg.get("github_owner").is_none());
    assert!(cfg.get("github_repo").is_none());
    assert!(cfg.get("include_prerelease").is_none());

    // relay_base_url 은 기본값을 유지해야 한다 (api_base_url 에 현혹되지 않음)
    let relay = cfg["relay_base_url"].as_str().unwrap_or("");
    assert_ne!(
        relay, "http://127.0.0.1:9876",
        "PUT 본문의 api_base_url 이 relay_base_url 에 유입되면 안 됨"
    );
}
