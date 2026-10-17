//! ═══════════════════════════════════════════════════════════════════
//! 데몬 통합 테스트
//! ═══════════════════════════════════════════════════════════════════
//!
//! Supervisor ↔ IPC ↔ InstanceStore ↔ ModuleLoader 가 실제로 연결되어
//! 올바르게 동작하는지 검증합니다.
//!
//! ## 설계 원칙
//! - 모든 테스트는 **의미 있는 assert**를 포함 (단순 "패닉 안남" 금지)
//! - 파일시스템 격리: 프로덕션 데이터를 건드리지 않음
//! - 동시성 안전: 테스트 간 포트/상태 격리
//!
//! ## 테스트 카테고리
//! 1. 모듈 발견 — module.toml 파싱, 캐시 일관성, 동시 접근
//! 2. IPC CRUD — HTTP API를 통한 인스턴스 생명주기
//! 3. IPC 직렬화 — 응답 스키마 계약 검증
//! 4. Python 플러그인 — graceful 에러 처리
//! 5. 프로세스 모니터링 — idle 상태에서의 모니터링 안정성

use saba_core::supervisor::Supervisor;
use saba_core::ipc::IPCServer;
use std::sync::Arc;
use std::net::TcpListener;
use tokio::sync::RwLock;
use tokio::time::{sleep, Duration};
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

fn pick_free_port() -> u16 {
    let listener = TcpListener::bind("127.0.0.1:0").expect("failed to bind ephemeral port");
    let port = listener.local_addr().unwrap().port();
    drop(listener);
    port
}

async fn wait_for_ipc_ready(base_url: &str, client: &reqwest::Client) {
    for _ in 0..50 {
        if let Ok(resp) = client.get(format!("{}/api/modules", base_url)).send().await {
            if resp.status().is_success() {
                return;
            }
        }
        sleep(Duration::from_millis(300)).await;
    }
    panic!("IPC server did not become ready: {}", base_url);
}

/// 테스트용 모듈이 없으면 자동 생성 (로컬/CI 모두 대응)
fn ensure_test_module() {
    let module_dir = std::path::Path::new("./modules/test-module");
    let toml_path = module_dir.join("module.toml");
    if !toml_path.exists() {
        fs::create_dir_all(module_dir).expect("failed to create test-module dir");
        fs::write(
            &toml_path,
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
        fs::write(module_dir.join("lifecycle.py"), "# minimal test lifecycle\n")
            .expect("failed to write lifecycle.py");
    }
    // import 픽스처 모듈도 여기서 함께 보장 — 병렬 실행 중 다른 테스트가
    // ./modules를 스캔하는 도중에 모듈이 새로 나타나 목록 일관성 테스트가
    // 깨지는 경합을 막는다 (모든 테스트가 셋업 시점에 동일한 모듈 집합을 봄).
    ensure_import_test_module();
}

/// 테스트용 IPC 서버 + Supervisor 를 부팅하여 (base_url, abort_handle)을 반환
/// 각 테스트마다 고유한 임시 인스턴스 디렉토리를 사용하여 병렬 실행 시 격리를 보장한다.
async fn boot_ipc() -> (String, Arc<RwLock<Supervisor>>, tokio::task::JoinHandle<()>) {
    std::env::set_var("SABA_AUTH_DISABLED", "1");
    ensure_test_module();

    // 테스트별 고유 임시 인스턴스 디렉토리 생성 (병렬 격리)
    let tmp_instances = std::env::temp_dir()
        .join(format!("saba-test-instances-{}", pick_free_port()));
    fs::create_dir_all(&tmp_instances).expect("failed to create temp instances dir");

    let supervisor = Arc::new(RwLock::new(
        Supervisor::new_with_instances_dir("./modules", &tmp_instances.to_string_lossy()),
    ));
    {
        let mut sup = supervisor.write().await;
        sup.initialize().await.expect("supervisor init failed");
    }

    let port = pick_free_port();
    let listen_addr = format!("127.0.0.1:{}", port);
    let base_url = format!("http://{}", listen_addr);

    let sup_clone = supervisor.clone();
    let server = IPCServer::new(sup_clone, &listen_addr, saba_core::daemon_log::DaemonLogBuffer::new());
    let server_task = tokio::spawn(async move {
        let _ = server.start().await;
    });

    let client = reqwest::Client::new();
    wait_for_ipc_ready(&base_url, &client).await;

    (base_url, supervisor, server_task)
}

// ═══════════════════════════════════════════════════════
// 1. Supervisor 초기화 & 모듈 발견
// ═══════════════════════════════════════════════════════

#[tokio::test]
async fn test_supervisor_initialization_succeeds() {
    ensure_test_module();
    let supervisor = Arc::new(RwLock::new(Supervisor::new("./modules")));
    let mut sup = supervisor.write().await;
    let result = sup.initialize().await;
    assert!(result.is_ok(), "Supervisor init should succeed: {:?}", result.err());
    cleanup_test_instances();
}

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

#[tokio::test]
async fn test_module_refresh_returns_consistent_result() {
    ensure_test_module();
    let supervisor = Arc::new(RwLock::new(Supervisor::new("./modules")));
    {
        let mut sup = supervisor.write().await;
        sup.initialize().await.unwrap();
    }

    let first_names: Vec<String> = {
        let sup = supervisor.read().await;
        let modules = sup.list_modules().unwrap_or_default();
        modules.iter().map(|m| m.metadata.name.clone()).collect()
    };

    let refreshed_names: Vec<String> = {
        let sup = supervisor.read().await;
        let modules = sup.refresh_modules().unwrap_or_default();
        modules.iter().map(|m| m.metadata.name.clone()).collect()
    };

    assert_eq!(
        first_names.len(),
        refreshed_names.len(),
        "Refresh must return same module count"
    );
    // 이름 집합이 동일해야 함 (순서 무관)
    let mut sorted_first = first_names.clone();
    let mut sorted_refresh = refreshed_names.clone();
    sorted_first.sort();
    sorted_refresh.sort();
    assert_eq!(sorted_first, sorted_refresh, "Refreshed module names must match original");
}

#[tokio::test]
async fn test_invalid_module_path_yields_zero_modules() {
    let supervisor = Arc::new(RwLock::new(Supervisor::new("/nonexistent/path/that/does/not/exist")));
    let mut sup = supervisor.write().await;
    // 패닉 없이 정상 반환되어야 함 (graceful degradation)
    let result = sup.initialize().await;
    assert!(result.is_ok(), "Initialize with bad path must not panic, got: {:?}", result.err());

    let modules = sup.list_modules().unwrap_or_default();
    assert_eq!(modules.len(), 0, "Nonexistent path must yield zero modules");
}

// ═══════════════════════════════════════════════════════
// 2. 동시 접근 안전성
// ═══════════════════════════════════════════════════════

#[tokio::test]
async fn test_concurrent_module_reads_return_identical_results() {
    ensure_test_module();
    let supervisor = Arc::new(RwLock::new(Supervisor::new("./modules")));
    {
        let mut sup = supervisor.write().await;
        sup.initialize().await.unwrap();
    }

    let expected_count = {
        let sup = supervisor.read().await;
        sup.list_modules().unwrap_or_default().len()
    };

    let mut handles = vec![];
    for _ in 0..20 {
        let sup = supervisor.clone();
        handles.push(tokio::spawn(async move {
            let sup = sup.read().await;
            sup.list_modules().unwrap_or_default().len()
        }));
    }

    for handle in handles {
        let count = handle.await.expect("Task should not panic");
        assert_eq!(
            count, expected_count,
            "All concurrent readers must see the same module count"
        );
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
// 4. 프로세스 모니터링 (idle 상태)
// ═══════════════════════════════════════════════════════

#[tokio::test]
async fn test_monitoring_loop_is_stable_under_idle() {
    ensure_test_module();
    let supervisor = Arc::new(RwLock::new(Supervisor::new("./modules")));
    {
        let mut sup = supervisor.write().await;
        sup.initialize().await.unwrap();
    }

    // 프로세스가 없는 상태에서 모니터링 N회 반복 — 패닉/데이터 오염 없어야 함
    let empty_snapshot: Vec<saba_core::process_monitor::RunningProcess> = vec![];
    for iteration in 0..10 {
        let mut sup = supervisor.write().await;
        let result = sup.monitor_processes(&empty_snapshot).await;
        assert!(
            result.is_ok(),
            "Monitoring iteration {} should succeed: {:?}",
            iteration,
            result.err()
        );
    }

    // 모니터링 후에도 모듈 목록이 정상이어야 함
    let modules = {
        let sup = supervisor.read().await;
        sup.list_modules().unwrap_or_default()
    };
    assert!(
        !modules.is_empty(),
        "Modules should still be available after idle monitoring"
    );
}

// ═══════════════════════════════════════════════════════
// 5. IPC 인스턴스 CRUD 전체 라이프사이클
// ═══════════════════════════════════════════════════════

#[tokio::test]
async fn test_ipc_instance_full_lifecycle() {
    let (base_url, _sup, server_task) = boot_ipc().await;
    let client = reqwest::Client::new();

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
    let test_name = format!("test-lifecycle-{}", pick_free_port());
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
// 6. IPC 에러 응답 스키마 계약
// ═══════════════════════════════════════════════════════

#[tokio::test]
async fn test_ipc_error_responses_have_consistent_schema() {
    let (base_url, _sup, server_task) = boot_ipc().await;
    let client = reqwest::Client::new();

    // 존재하지 않는 인스턴스 → 404 + error 필드
    let r1 = client
        .get(format!("{}/api/instance/nonexistent-id-abc123", base_url))
        .send().await.unwrap();
    assert_eq!(r1.status(), reqwest::StatusCode::NOT_FOUND);
    let body1: Value = r1.json().await.unwrap();
    assert!(
        body1.get("error").is_some(),
        "404 response must contain 'error' field, got: {:?}",
        body1
    );

    // 존재하지 않는 인스턴스에 명령 → 404 + error 필드
    let r2 = client
        .post(format!("{}/api/instance/nonexistent-id/command", base_url))
        .json(&serde_json::json!({ "command": "status", "args": {} }))
        .send().await.unwrap();
    assert_eq!(r2.status(), reqwest::StatusCode::NOT_FOUND);
    let body2: Value = r2.json().await.unwrap();
    let err_msg = body2["error"].as_str().unwrap_or_default().to_lowercase();
    assert!(
        err_msg.contains("not found") || err_msg.contains("instance"),
        "Error message should mention instance, got: '{}'",
        err_msg
    );

    server_task.abort();
}

// ═══════════════════════════════════════════════════════
// 7. IPC 모듈 메타데이터 응답 스키마
// ═══════════════════════════════════════════════════════

#[tokio::test]
async fn test_ipc_module_metadata_contains_required_fields() {
    let (base_url, _sup, server_task) = boot_ipc().await;
    let client = reqwest::Client::new();

    let resp = client
        .get(format!("{}/api/modules", base_url))
        .send().await.unwrap();
    let json: Value = resp.json().await.unwrap();

    for module in json["modules"].as_array().unwrap() {
        let name = module["name"].as_str().unwrap_or("<unknown>");

        // 필수 스키마 필드 확인
        assert!(
            module.get("name").and_then(|v| v.as_str()).is_some(),
            "Module must have string 'name'"
        );
        assert!(
            module.get("version").and_then(|v| v.as_str()).is_some(),
            "Module '{}' must have string 'version'", name
        );

        // commands 필드가 있다면 내부 구조 검증
        if let Some(cmds) = module.get("commands") {
            if !cmds.is_null() {
                let fields = cmds.get("fields").and_then(|v| v.as_array())
                    .unwrap_or_else(|| panic!("Module '{}' commands must have 'fields' array", name));
                for cmd in fields {
                    assert!(
                        cmd.get("name").and_then(|v| v.as_str()).is_some(),
                        "Command in module '{}' must have 'name'", name
                    );
                    assert!(
                        cmd.get("label").and_then(|v| v.as_str()).is_some(),
                        "Command in module '{}' must have 'label'", name
                    );
                }
            }
        }
    }

    server_task.abort();
}

// ═══════════════════════════════════════════════════════
// 8. 인스턴스 생성·삭제·재생성 독립성 검증
//    네이티브 인스턴스는 모듈당 1개 제한이므로,
//    생성→삭제→재생성 사이클로 독립성을 검증한다.
// ═══════════════════════════════════════════════════════

#[tokio::test]
async fn test_ipc_multiple_instances_are_independent() {
    let (base_url, _sup, server_task) = boot_ipc().await;
    let client = reqwest::Client::new();

    let modules: Value = client
        .get(format!("{}/api/modules", base_url))
        .send().await.unwrap()
        .json().await.unwrap();
    let module_name = modules["modules"][0]["name"].as_str()
        .expect("No modules found — ensure ./modules/test-module/module.toml exists");

    // ── 1) 첫 번째 인스턴스 생성 ──
    let name_a = format!("test-multi-0-{}", pick_free_port());
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
    let name_b = format!("test-multi-1-{}", pick_free_port());
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

// ═══════════════════════════════════════════════════════
// 7. 서버 업데이트 확인 API
// ═══════════════════════════════════════════════════════

/// GET /api/instance/:id/check-update — 존재하지 않는 인스턴스 → 404
#[tokio::test]
async fn test_check_update_not_found() {
    let (base_url, _sup, server_task) = boot_ipc().await;
    let client = reqwest::Client::new();
    wait_for_ipc_ready(&base_url, &client).await;

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
    let client = reqwest::Client::new();
    wait_for_ipc_ready(&base_url, &client).await;

    // 기존 인스턴스 목록에서 ext_data 없는 인스턴스를 찾거나,
    // 찾지 못하면 migration 모드로 생성 (중복 네이티브 제한 우회)
    let list_resp = client
        .get(format!("{}/api/instances", base_url))
        .send()
        .await
        .unwrap();
    let instances: Vec<Value> = list_resp.json().await.unwrap();

    let test_id = if let Some(inst) = instances.iter().find(|i| {
        i.get("extension_data")
            .and_then(|ed| ed.as_object())
            .map(|m| m.is_empty())
            .unwrap_or(true)
    }) {
        inst["id"].as_str().unwrap().to_string()
    } else {
        // migration 모드로 생성 (임시 디렉터리)
        let tmp = std::env::temp_dir().join("saba-test-check-update");
        let _ = fs::create_dir_all(&tmp);
        let create_resp = client
            .post(format!("{}/api/instances", base_url))
            .json(&serde_json::json!({
                "name": format!("test-check-update-{}", pick_free_port()),
                "module_name": "minecraft",
                "migration_source": tmp.to_string_lossy(),
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
    // reason이 "no_extension_data" 또는 "no_hook_handled" (기존 인스턴스는 ext_data가 있을 수 있음)
    assert!(
        body["reason"] == "no_extension_data" || body["reason"] == "no_hook_handled",
        "Expected reason to be no_extension_data or no_hook_handled, got: {:?}",
        body["reason"]
    );

    server_task.abort();
    cleanup_test_instances();
}

// ═══════════════════════════════════════════════════════
// 9. 에이전트 통합 명령 진입점 (/api/agent/command)
// ═══════════════════════════════════════════════════════

/// 이름(대소문자 무시)으로 인스턴스를 해석하여 status 동작이 동작하는지 검증.
/// 데몬 측 단일 진입점이 MCP/AI 에이전트가 보내는 자연어 의도를 올바른
/// 인스턴스로 라우팅하는지 확인한다.
#[tokio::test]
async fn test_agent_command_resolves_target_by_name() {
    let (base_url, _sup, server_task) = boot_ipc().await;
    let client = reqwest::Client::new();

    let first_module = {
        let modules: Value = client
            .get(format!("{}/api/modules", base_url))
            .send().await.unwrap()
            .json().await.unwrap();
        modules["modules"][0]["name"].as_str().unwrap().to_string()
    };

    // 인스턴스 생성 (대문자 포함 이름으로 대소문자 무시 매칭 검증)
    let test_name = format!("Test-Agent-{}", pick_free_port());
    let created: Value = client
        .post(format!("{}/api/instances", base_url))
        .json(&serde_json::json!({
            "name": test_name,
            "module_name": first_module,
            "executable_path": "C:/tmp/test-server.exe"
        }))
        .send().await.unwrap()
        .json().await.unwrap();
    let instance_id = created["id"].as_str().unwrap().to_string();

    // 소문자로 이름을 보내도 해석되어야 함 → status는 not-found가 아니어야 함
    let resp = client
        .post(format!("{}/api/agent/command", base_url))
        .json(&serde_json::json!({
            "target": test_name.to_lowercase(),
            "action": "status"
        }))
        .send().await.unwrap();
    assert_ne!(
        resp.status(),
        reqwest::StatusCode::NOT_FOUND,
        "Agent endpoint must resolve instance name case-insensitively"
    );

    // ── 정리
    let _ = client
        .delete(format!("{}/api/instance/{}", base_url, instance_id))
        .send().await;
    server_task.abort();
    cleanup_test_instances();
}

/// 존재하지 않는 대상 → 404 + instance_not_found error_code.
#[tokio::test]
async fn test_agent_command_unknown_target_returns_not_found() {
    let (base_url, _sup, server_task) = boot_ipc().await;
    let client = reqwest::Client::new();

    let resp = client
        .post(format!("{}/api/agent/command", base_url))
        .json(&serde_json::json!({
            "target": "no-such-server-xyz",
            "action": "start"
        }))
        .send().await.unwrap();
    assert_eq!(resp.status(), reqwest::StatusCode::NOT_FOUND);
    let body: Value = resp.json().await.unwrap();
    assert_eq!(body["error_code"].as_str(), Some("instance_not_found"));

    server_task.abort();
    cleanup_test_instances();
}

/// command 동작인데 command 필드가 비면 400.
#[tokio::test]
async fn test_agent_command_missing_command_field_returns_bad_request() {
    let (base_url, _sup, server_task) = boot_ipc().await;
    let client = reqwest::Client::new();

    let first_module = {
        let modules: Value = client
            .get(format!("{}/api/modules", base_url))
            .send().await.unwrap()
            .json().await.unwrap();
        modules["modules"][0]["name"].as_str().unwrap().to_string()
    };
    let test_name = format!("test-agent-cmd-{}", pick_free_port());
    let created: Value = client
        .post(format!("{}/api/instances", base_url))
        .json(&serde_json::json!({
            "name": test_name,
            "module_name": first_module,
            "executable_path": "C:/tmp/test-server.exe"
        }))
        .send().await.unwrap()
        .json().await.unwrap();
    let instance_id = created["id"].as_str().unwrap().to_string();

    let resp = client
        .post(format!("{}/api/agent/command", base_url))
        .json(&serde_json::json!({
            "target": test_name,
            "action": "command"
        }))
        .send().await.unwrap();
    assert_eq!(resp.status(), reqwest::StatusCode::BAD_REQUEST);

    let _ = client
        .delete(format!("{}/api/instance/{}", base_url, instance_id))
        .send().await;
    server_task.abort();
    cleanup_test_instances();
}

// ═══════════════════════════════════════════════════════
// 10. 마이그레이션 설정 임포트 플래그 (settings_import_pending)
// ═══════════════════════════════════════════════════════
//
// 사고 배경: 마이그레이션 등록 시 기존 게임 설정 파일 임포트가 조용히 실패하면
// module_settings 캐시가 스키마 기본값으로 남고, GUI가 그 기본값을 실제 값처럼
// 표시/저장해 게임 설정 파일을 기본값으로 덮어쓰는 사고가 발생했다.
// 임포트 실패는 인스턴스에 settings_import_pending 플래그로 기록되어야 하고,
// 이후 라이브 읽기(import)가 성공하면 캐시 갱신과 함께 해제되어야 한다.

/// import_settings가 정상 동작하는 테스트 모듈 픽스처를 생성한다.
/// 멱등 + module.toml은 rename으로 원자적으로 배치 — 병렬 테스트가
/// ./modules 스캔 도중 반쯤 쓰인 module.toml을 읽는 경합을 방지한다.
fn ensure_import_test_module() {
    let module_dir = std::path::Path::new("./modules/test-module-import");
    let toml_path = module_dir.join("module.toml");
    if toml_path.exists() {
        return;
    }
    fs::create_dir_all(module_dir).expect("failed to create test-module-import dir");
    fs::write(
        module_dir.join("lifecycle.py"),
        r#"import sys, json
fn = sys.argv[1] if len(sys.argv) > 1 else ""
try:
    cfg = json.loads(sys.stdin.read() or "{}")
except Exception:
    cfg = {}
if fn == "import_settings":
    print(json.dumps({"success": True, "settings": {"exp_rate": 2, "server_name": "ImportedName"}}))
else:
    print(json.dumps({"success": True, "message": "noop:" + fn}))
"#,
    )
    .expect("failed to write lifecycle.py");
    let tmp_path = module_dir.join("module.toml.tmp");
    fs::write(
        &tmp_path,
        r#"[module]
name = "test-module-import"
version = "0.0.1"
description = "Integration test fixture (import_settings ok)"
author = "ci"
game_name = "TestGame"
display_name = "Test Module Import"
entry = "lifecycle.py"
"#,
    )
    .expect("failed to write module.toml.tmp");
    // 동시 생성 경합 시 어느 한쪽만 성공하면 됨 (내용 동일)
    let _ = fs::rename(&tmp_path, &toml_path);
}

/// 마이그레이션 임포트 실패(모듈 lifecycle에 import_settings 없음) 시
/// settings_import_pending 플래그가 true로 저장되는지 검증.
#[tokio::test]
async fn test_migration_import_failure_sets_pending_flag() {
    let (base_url, _sup, server_task) = boot_ipc().await;
    let client = reqwest::Client::new();

    let tmp = std::env::temp_dir().join(format!("saba-test-import-fail-{}", pick_free_port()));
    let _ = fs::create_dir_all(&tmp);

    let create_resp = client
        .post(format!("{}/api/instances", base_url))
        .json(&serde_json::json!({
            "name": format!("test-import-fail-{}", pick_free_port()),
            "module_name": "test-module",
            "migration_source": tmp.to_string_lossy(),
        }))
        .send()
        .await
        .unwrap();
    assert!(create_resp.status().is_success(), "create failed: {}", create_resp.status());
    let created: Value = create_resp.json().await.unwrap();
    let id = created["id"].as_str().unwrap().to_string();

    let list: Vec<Value> = client
        .get(format!("{}/api/instances", base_url))
        .send().await.unwrap()
        .json().await.unwrap();
    let inst = list.iter().find(|i| i["id"] == id.as_str()).expect("instance not in list");
    assert_eq!(
        inst["settings_import_pending"], true,
        "import 실패 시 settings_import_pending=true여야 함, got: {:?}",
        inst.get("settings_import_pending")
    );

    let _ = client.delete(format!("{}/api/instance/{}", base_url, id)).send().await;
    server_task.abort();
    cleanup_test_instances();
}

/// 마이그레이션 임포트 성공 시 플래그가 false이고 캐시에 임포트 값이 반영되는지 검증.
#[tokio::test]
async fn test_migration_import_success_keeps_flag_clear() {
    ensure_import_test_module();
    let (base_url, _sup, server_task) = boot_ipc().await;
    let client = reqwest::Client::new();

    let tmp = std::env::temp_dir().join(format!("saba-test-import-ok-{}", pick_free_port()));
    let _ = fs::create_dir_all(&tmp);

    let create_resp = client
        .post(format!("{}/api/instances", base_url))
        .json(&serde_json::json!({
            "name": format!("test-import-ok-{}", pick_free_port()),
            "module_name": "test-module-import",
            "migration_source": tmp.to_string_lossy(),
        }))
        .send()
        .await
        .unwrap();
    assert!(create_resp.status().is_success(), "create failed: {}", create_resp.status());
    let created: Value = create_resp.json().await.unwrap();
    let id = created["id"].as_str().unwrap().to_string();

    let list: Vec<Value> = client
        .get(format!("{}/api/instances", base_url))
        .send().await.unwrap()
        .json().await.unwrap();
    let inst = list.iter().find(|i| i["id"] == id.as_str()).expect("instance not in list");
    assert_eq!(inst["settings_import_pending"], false);
    assert_eq!(
        inst["module_settings"]["exp_rate"], 2,
        "임포트된 값이 module_settings 캐시에 반영되어야 함"
    );

    let _ = client.delete(format!("{}/api/instance/{}", base_url, id)).send().await;
    server_task.abort();
    cleanup_test_instances();
}

/// GET /settings/live 성공 시 module_settings 캐시가 실제 값으로 갱신되고
/// settings_import_pending 플래그가 해제되는지 검증.
#[tokio::test]
async fn test_read_live_settings_refreshes_cache_and_clears_flag() {
    ensure_import_test_module();
    let (base_url, sup, server_task) = boot_ipc().await;
    let client = reqwest::Client::new();

    let tmp = std::env::temp_dir().join(format!("saba-test-live-refresh-{}", pick_free_port()));
    let _ = fs::create_dir_all(&tmp);

    let create_resp = client
        .post(format!("{}/api/instances", base_url))
        .json(&serde_json::json!({
            "name": format!("test-live-refresh-{}", pick_free_port()),
            "module_name": "test-module-import",
            "migration_source": tmp.to_string_lossy(),
        }))
        .send()
        .await
        .unwrap();
    assert!(create_resp.status().is_success());
    let created: Value = create_resp.json().await.unwrap();
    let id = created["id"].as_str().unwrap().to_string();

    // 사고 상황 재현: 플래그 true + 캐시에 오염된(기본값) 설정 주입
    {
        let mut supervisor = sup.write().await;
        let mut inst = supervisor.instance_store.get(&id).unwrap().clone();
        inst.settings_import_pending = true;
        inst.module_settings.insert("exp_rate".to_string(), serde_json::json!(999));
        supervisor.instance_store.update(&id, inst).unwrap();
    }

    let live: Value = client
        .get(format!("{}/api/instance/{}/settings/live", base_url, id))
        .send().await.unwrap()
        .json().await.unwrap();
    assert_eq!(live["success"], true, "live read should succeed: {:?}", live);
    assert_eq!(live["settings"]["exp_rate"], 2);

    // 캐시가 실제 값으로 갱신되고 플래그가 해제되어야 한다
    {
        let supervisor = sup.read().await;
        let inst = supervisor.instance_store.get(&id).unwrap();
        assert!(!inst.settings_import_pending, "live read 성공 후 플래그 해제되어야 함");
        assert_eq!(
            inst.module_settings.get("exp_rate"),
            Some(&serde_json::json!(2)),
            "live read 성공 시 module_settings 캐시가 실제 값으로 갱신되어야 함"
        );
    }

    let _ = client.delete(format!("{}/api/instance/{}", base_url, id)).send().await;
    server_task.abort();
    cleanup_test_instances();
}
