//! 데몬 통합 테스트: 저장소의 `./modules`(시험 모듈을 만들어 둔다)로 Supervisor와 IPC 서버를 띄워
//! HTTP API로 인스턴스를 만들고 지우며 응답을 본다

mod common;

use saba_core::supervisor::Supervisor;
use std::sync::Arc;
use tokio::sync::RwLock;
use std::fs;
use serde_json::Value;

// ═══════════════════════════════════════════════════════
// 테스트 유틸리티
// ═══════════════════════════════════════════════════════

/// 테스트 종료 시 인스턴스 디렉토리에서 테스트 데이터 자동 제거
fn cleanup_test_instances() {
    let instances_dir = std::env::var("SABA_INSTANCES_PATH")
        .unwrap_or_else(|_| {
            saba_chan_updater_lib::constants::resolve_instances_dir()
                .to_string_lossy()
                .to_string()
        });
    let instances_dir = std::path::Path::new(&instances_dir);

    if let Ok(entries) = fs::read_dir(instances_dir) {
        for entry in entries.flatten() {
            let path = entry.path();
            if !path.is_dir() { continue; }
            let instance_json = path.join("instance.json");
            if let Ok(content) = fs::read_to_string(&instance_json) {
                if let Ok(instance) = serde_json::from_str::<Value>(&content) {
                    if instance.get("name")
                        .and_then(|v| v.as_str())
                        .map(|n| n.starts_with("test-"))
                        .unwrap_or(false)
                    {
                        let _ = fs::remove_dir_all(&path);
                    }
                }
            }
        }
    }
}

/// 테스트 안에서 겹치지 않는 이름
fn unique_name(prefix: &str) -> String {
    format!("{}-{}", prefix, uuid::Uuid::new_v4())
}

/// 테스트용 모듈이 없으면 자동 생성 (로컬/CI 모두 대응).
/// 병렬 시험이 덜 쓴 모듈을 보지 않게 한 번만 만들고, lifecycle.py를 먼저 쓴 뒤 module.toml을 rename으로 놓는다 (L47)
fn ensure_test_module() {
    static ONCE: std::sync::Once = std::sync::Once::new();
    ONCE.call_once(write_test_module);
}

fn write_test_module() {
    let module_dir = std::path::Path::new("./modules/test-module");
    let toml_path = module_dir.join("module.toml");
    if !toml_path.exists() {
        fs::create_dir_all(module_dir).expect("failed to create test-module dir");
        fs::write(module_dir.join("lifecycle.py"), "# minimal test lifecycle\n")
            .expect("failed to write lifecycle.py");
        let tmp_path = module_dir.join(format!("module.toml.{}.tmp", std::process::id()));
        fs::write(
            &tmp_path,
            r#"[module]
name = "test-module"
version = "0.0.1"
description = "Integration test fixture"
author = "ci"
game_name = "TestGame"
display_name = "Test Module"
entry = "lifecycle.py"
"#,
        )
        .expect("failed to write module.toml");
        fs::rename(&tmp_path, &toml_path).expect("failed to place module.toml");
    }
}

/// 테스트마다 따로 둔 임시 인스턴스 폴더로 Supervisor와 IPC 서버를 띄워 (base_url, supervisor, server_task)를 돌려준다.
/// 임시 폴더는 서버 작업이 끝날 때(abort) 함께 지운다
async fn boot_ipc() -> (String, Arc<RwLock<Supervisor>>, tokio::task::JoinHandle<()>) {
    ensure_test_module();
    let tmp_instances = Arc::new(
        tempfile::Builder::new()
            .prefix("saba-test-instances-")
            .tempdir()
            .expect("failed to create temp instances dir"),
    );
    let supervisor = Arc::new(RwLock::new(
        Supervisor::new_with_instances_dir("./modules", &tmp_instances.path().to_string_lossy()),
    ));
    supervisor.write().await.initialize().await.expect("supervisor init failed");
    let (base_url, server_task) = common::serve_ipc(supervisor.clone(), tmp_instances).await;
    (base_url, supervisor, server_task)
}

// ═══════════════════════════════════════════════════════
// 1. Supervisor 초기화 & 모듈 발견
// ═══════════════════════════════════════════════════════

#[tokio::test]
async fn test_module_discovery_returns_known_modules() {
    ensure_test_module();
    let supervisor = Arc::new(RwLock::new(Supervisor::new("./modules")));
    {
        let mut sup = supervisor.write().await;
        sup.initialize().await.unwrap();
    }

    let modules = {
        let sup = supervisor.read().await;
        sup.list_modules().unwrap_or_default()
    };

    // ./modules 에 최소 1개 이상의 module.toml이 존재해야 함
    assert!(
        !modules.is_empty(),
        "At least one module should be discovered in ./modules"
    );

    // 모든 모듈에 필수 메타데이터가 있어야 함
    for m in &modules {
        assert!(!m.metadata.name.is_empty(), "Module name must not be empty");
        assert!(!m.metadata.version.is_empty(), "Module version must not be empty");
        assert!(!m.metadata.entry.is_empty(), "Module entry must not be empty");
        assert!(!m.path.is_empty(), "Module path must not be empty");
    }
}

// ═══════════════════════════════════════════════════════
// 3. Python 플러그인 graceful 에러
// ═══════════════════════════════════════════════════════

#[tokio::test]
async fn test_run_plugin_with_nonexistent_file_returns_error() {
    use saba_core::plugin::run_plugin;

    let result = run_plugin("nonexistent_module_abc123.py", "test_func", serde_json::json!({})).await;
    assert!(
        result.is_err(),
        "run_plugin with nonexistent file must return Err, not Ok({:?})",
        result.ok()
    );
    let err_msg = result.unwrap_err().to_string();
    assert!(
        !err_msg.is_empty(),
        "Error message should be descriptive, not empty"
    );
}

// ═══════════════════════════════════════════════════════
// 5. IPC 인스턴스 CRUD 전체 라이프사이클
// ═══════════════════════════════════════════════════════

#[tokio::test]
async fn test_ipc_instance_full_lifecycle() {
    let (base_url, _sup, server_task) = boot_ipc().await;
    let client = saba_chan_updater_lib::tls::http_client_builder().build().unwrap();

    // ── 1) 모듈 목록 조회 — 응답 스키마 검증
    let modules_resp = client
        .get(format!("{}/api/modules", base_url))
        .send().await.unwrap();
    assert_eq!(modules_resp.status(), reqwest::StatusCode::OK);
    let modules_json: Value = modules_resp.json().await.unwrap();
    let modules_arr = modules_json["modules"].as_array()
        .expect("Response must have 'modules' array");
    assert!(!modules_arr.is_empty(), "At least one module required for CRUD test");

    let first_module = modules_arr[0]["name"].as_str().unwrap();

    // 모든 모듈에 필수 필드가 존재하는지 스키마 계약 검증
    for m in modules_arr {
        assert!(m.get("name").is_some(), "Module must have 'name' field");
        assert!(m.get("version").is_some(), "Module must have 'version' field");
    }

    // ── 2) 인스턴스 생성
    let test_name = unique_name("test-lifecycle");
    let create_resp = client
        .post(format!("{}/api/instances", base_url))
        .json(&serde_json::json!({
            "name": test_name,
            "module_name": first_module,
            "executable_path": "C:/tmp/test-server.exe"
        }))
        .send().await.unwrap();
    assert_eq!(create_resp.status(), reqwest::StatusCode::CREATED, "Create should return 201");
    let create_json: Value = create_resp.json().await.unwrap();
    let instance_id = create_json["id"].as_str()
        .expect("Create response must contain 'id'");
    assert!(!instance_id.is_empty(), "Instance ID must not be empty");

    // ── 3) 생성된 인스턴스 조회 → 필드 정합성
    let get_resp = client
        .get(format!("{}/api/instance/{}", base_url, instance_id))
        .send().await.unwrap();
    assert_eq!(get_resp.status(), reqwest::StatusCode::OK);
    let get_json: Value = get_resp.json().await.unwrap();
    assert_eq!(get_json["name"].as_str(), Some(test_name.as_str()));
    assert_eq!(get_json["module_name"].as_str(), Some(first_module));

    // ── 4) 서버 목록에 새 인스턴스가 포함되어야 함
    let servers_resp = client
        .get(format!("{}/api/servers", base_url))
        .send().await.unwrap();
    assert_eq!(servers_resp.status(), reqwest::StatusCode::OK);
    let servers_json: Value = servers_resp.json().await.unwrap();
    let server_ids: Vec<&str> = servers_json["servers"]
        .as_array().unwrap()
        .iter()
        .filter_map(|s| s.get("id").or_else(|| s.get("instance_id")).and_then(|v| v.as_str()))
        .collect();
    assert!(
        server_ids.contains(&instance_id),
        "Newly created instance '{}' must appear in /api/servers, got: {:?}",
        instance_id, server_ids
    );

    // ── 5) 삭제
    let delete_resp = client
        .delete(format!("{}/api/instance/{}", base_url, instance_id))
        .send().await.unwrap();
    assert_eq!(delete_resp.status(), reqwest::StatusCode::OK);

    // ── 6) 삭제 후 조회 → 404
    let gone_resp = client
        .get(format!("{}/api/instance/{}", base_url, instance_id))
        .send().await.unwrap();
    assert_eq!(gone_resp.status(), reqwest::StatusCode::NOT_FOUND);

    // ── 7) 이중 삭제 → 멱등성 또는 404
    let double_delete = client
        .delete(format!("{}/api/instance/{}", base_url, instance_id))
        .send().await.unwrap();
    assert!(
        double_delete.status() == reqwest::StatusCode::NOT_FOUND
            || double_delete.status() == reqwest::StatusCode::OK,
        "Double delete should be idempotent or 404, got: {}",
        double_delete.status()
    );

    server_task.abort();
    cleanup_test_instances();
}

// ═══════════════════════════════════════════════════════
// 8. 인스턴스 생성·삭제·재생성 독립성 검증
//    네이티브 인스턴스는 모듈당 1개 제한이므로,
//    생성→삭제→재생성 사이클로 독립성을 검증한다.
// ═══════════════════════════════════════════════════════

#[tokio::test]
async fn test_ipc_multiple_instances_are_independent() {
    let (base_url, _sup, server_task) = boot_ipc().await;
    let client = saba_chan_updater_lib::tls::http_client_builder().build().unwrap();

    let modules: Value = client
        .get(format!("{}/api/modules", base_url))
        .send().await.unwrap()
        .json().await.unwrap();
    let module_name = modules["modules"][0]["name"].as_str()
        .expect("No modules found — ensure ./modules/test-module/module.toml exists");

    // ── 1) 첫 번째 인스턴스 생성 ──
    let name_a = unique_name("test-multi-0");
    let resp_a = client
        .post(format!("{}/api/instances", base_url))
        .json(&serde_json::json!({
            "name": name_a,
            "module_name": module_name,
            "executable_path": "C:/tmp/server-0.exe"
        }))
        .send().await.unwrap();
    assert_eq!(resp_a.status(), reqwest::StatusCode::CREATED, "First instance creation failed");
    let json_a: Value = resp_a.json().await.unwrap();
    let id_a = json_a["id"].as_str().unwrap().to_string();

    // 조회 가능 확인
    let get_a = client
        .get(format!("{}/api/instance/{}", base_url, id_a))
        .send().await.unwrap();
    assert_eq!(get_a.status(), reqwest::StatusCode::OK);

    // ── 2) 삭제 ──
    let del = client
        .delete(format!("{}/api/instance/{}", base_url, id_a))
        .send().await.unwrap();
    assert!(del.status().is_success(), "Delete should succeed, got {}", del.status());

    // 삭제 후 조회 → 404
    let gone = client
        .get(format!("{}/api/instance/{}", base_url, id_a))
        .send().await.unwrap();
    assert_eq!(
        gone.status(),
        reqwest::StatusCode::NOT_FOUND,
        "Deleted instance must return 404"
    );

    // ── 3) 같은 모듈로 새 인스턴스 재생성 (이전 인스턴스 삭제 후이므로 가능) ──
    let name_b = unique_name("test-multi-1");
    let resp_b = client
        .post(format!("{}/api/instances", base_url))
        .json(&serde_json::json!({
            "name": name_b,
            "module_name": module_name,
            "executable_path": "C:/tmp/server-1.exe"
        }))
        .send().await.unwrap();
    let resp_b_status = resp_b.status();
    let json_b: Value = resp_b.json().await.unwrap();
    assert_eq!(
        resp_b_status,
        reqwest::StatusCode::CREATED,
        "Second instance creation failed with {}: {:?}", resp_b_status, json_b
    );
    let id_b = json_b["id"].as_str().unwrap().to_string();

    // ID가 다름 확인 (독립적 인스턴스)
    assert_ne!(id_a, id_b, "New instance must have a different ID");

    // 조회 가능 확인
    let get_b = client
        .get(format!("{}/api/instance/{}", base_url, id_b))
        .send().await.unwrap();
    assert_eq!(get_b.status(), reqwest::StatusCode::OK);

    // 정리
    client.delete(format!("{}/api/instance/{}", base_url, id_b))
        .send().await.unwrap();
    server_task.abort();
    cleanup_test_instances();
}

/// 서버 추가 때 익스텐션이 물은 칸(`AddServer.options`)은 만들 때 `extension_data`로 한 번에 저장된다 (Q-7)
#[tokio::test]
async fn test_ipc_create_instance_saves_extension_data() {
    let (base_url, _sup, server_task) = boot_ipc().await;
    let client = saba_chan_updater_lib::tls::http_client_builder().build().unwrap();
    let modules: Value = client.get(format!("{}/api/modules", base_url)).send().await.unwrap().json().await.unwrap();
    let module_name = modules["modules"][0]["name"].as_str().unwrap();

    let bad = client
        .post(format!("{}/api/instances", base_url))
        .json(&serde_json::json!({ "name": unique_name("test-extdata-bad"), "module_name": module_name, "extension_data": "boxed" }))
        .send().await.unwrap();
    assert_eq!(bad.status(), reqwest::StatusCode::BAD_REQUEST);
    let bad: Value = bad.json().await.unwrap();
    assert_eq!(bad["error_code"], "invalid_format");

    let resp = client
        .post(format!("{}/api/instances", base_url))
        .json(&serde_json::json!({
            "name": unique_name("test-extdata"),
            "module_name": module_name,
            "extension_data": { "boxed": true, "skip": null }
        }))
        .send().await.unwrap();
    assert_eq!(resp.status(), reqwest::StatusCode::CREATED);
    let id = resp.json::<Value>().await.unwrap()["id"].as_str().unwrap().to_string();
    let inst: Value = client.get(format!("{}/api/instance/{}", base_url, id)).send().await.unwrap().json().await.unwrap();
    assert_eq!(inst["extension_data"]["boxed"], true, "{inst}");
    assert!(inst["extension_data"].get("skip").is_none());

    client.delete(format!("{}/api/instance/{}", base_url, id)).send().await.unwrap();
    server_task.abort();
    cleanup_test_instances();
}

// ═══════════════════════════════════════════════════════
// 7. 서버 업데이트 확인 API
// ═══════════════════════════════════════════════════════

/// GET /api/instance/:id/check-update — 존재하지 않는 인스턴스 → 404
#[tokio::test]
async fn test_check_update_not_found() {
    let (base_url, _sup, server_task) = boot_ipc().await;
    let client = saba_chan_updater_lib::tls::http_client_builder().build().unwrap();

    let resp = client
        .get(format!("{}/api/instance/nonexistent-id/check-update", base_url))
        .send()
        .await
        .unwrap();
    assert_eq!(resp.status(), reqwest::StatusCode::NOT_FOUND);
    let body: Value = resp.json().await.unwrap();
    assert_eq!(body["error_code"], "instance_not_found");

    server_task.abort();
}

/// GET /api/instance/:id/check-update — ext_data 없는 인스턴스 → update_available: false
#[tokio::test]
async fn test_check_update_no_extension_data() {
    let (base_url, _sup, server_task) = boot_ipc().await;
    let client = saba_chan_updater_lib::tls::http_client_builder().build().unwrap();

    // 기존 인스턴스 목록에서 ext_data 없는 인스턴스를 찾거나,
    // 찾지 못하면 migration 모드로 생성 (중복 네이티브 제한 우회)
    let list_resp = client
        .get(format!("{}/api/instances", base_url))
        .send()
        .await
        .unwrap();
    let instances: Vec<Value> = list_resp.json().await.unwrap();

    // migration 모드로 만들 때 쓰는 임시 디렉터리 (테스트가 끝날 때까지 둔다)
    let tmp = tempfile::Builder::new().prefix("saba-test-check-update-").tempdir().expect("failed to create temp dir");
    let test_id = if let Some(inst) = instances.iter().find(|i| {
        i.get("extension_data")
            .and_then(|ed| ed.as_object())
            .map(|m| m.is_empty())
            .unwrap_or(true)
    }) {
        inst["id"].as_str().unwrap().to_string()
    } else {
        // migration 모드로 생성 (임시 디렉터리)
        let create_resp = client
            .post(format!("{}/api/instances", base_url))
            .json(&serde_json::json!({
                "name": unique_name("test-check-update"),
                "module_name": "minecraft",
                "migration_source": tmp.path().to_string_lossy(),
            }))
            .send()
            .await
            .unwrap();
        assert!(
            create_resp.status().is_success(),
            "Instance creation should succeed, got {}",
            create_resp.status()
        );
        let created: Value = create_resp.json().await.unwrap();
        created["id"].as_str().unwrap().to_string()
    };

    let resp = client
        .get(format!("{}/api/instance/{}/check-update", base_url, test_id))
        .send()
        .await
        .unwrap();
    assert_eq!(resp.status(), reqwest::StatusCode::OK);
    let body: Value = resp.json().await.unwrap();
    assert_eq!(body["update_available"], false);
    // 다운로드 설치(minecraft)면 update_unsupported, 설치 방식 익스텐션 서버면 no_extension_data 또는 no_hook_handled
    assert!(
        ["update_unsupported", "no_extension_data", "no_hook_handled"].iter().any(|r| body["reason"] == *r),
        "Expected reason to be update_unsupported, no_extension_data or no_hook_handled, got: {:?}",
        body["reason"]
    );

    server_task.abort();
    cleanup_test_instances();
}
