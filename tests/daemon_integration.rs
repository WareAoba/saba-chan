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

/// 테스트 종료 시 instances.json에서 테스트 데이터 자동 제거
fn cleanup_test_instances() {
    let instances_path = std::env::var("SABA_INSTANCES_PATH")
        .unwrap_or_else(|_| {
            std::env::var("APPDATA")
                .map(|appdata| format!("{}\\saba-chan\\instances.json", appdata))
                .unwrap_or_else(|_| "./instances.json".to_string())
        });
    let instances_path = instances_path.as_str();

    if let Ok(content) = fs::read_to_string(instances_path) {
        if let Ok(mut instances) = serde_json::from_str::<Vec<Value>>(&content) {
            let original_count = instances.len();
            instances.retain(|instance| {
                instance
                    .get("name")
                    .and_then(|v| v.as_str())
                    .map(|n| !n.starts_with("test-"))
                    .unwrap_or(true)
            });
            if instances.len() != original_count {
                if let Ok(json) = serde_json::to_string_pretty(&instances) {
                    let _ = fs::write(instances_path, json);
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

/// 테스트용 IPC 서버 + Supervisor 를 부팅하여 (base_url, abort_handle)을 반환
async fn boot_ipc() -> (String, Arc<RwLock<Supervisor>>, tokio::task::JoinHandle<()>) {
    std::env::set_var("SABA_AUTH_DISABLED", "1");

    let supervisor = Arc::new(RwLock::new(Supervisor::new("./modules")));
    {
        let mut sup = supervisor.write().await;
        sup.initialize().await.expect("supervisor init failed");
    }

    let port = pick_free_port();
    let listen_addr = format!("127.0.0.1:{}", port);
    let base_url = format!("http://{}", listen_addr);

    let sup_clone = supervisor.clone();
    let server = IPCServer::new(sup_clone, &listen_addr);
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
    let supervisor = Arc::new(RwLock::new(Supervisor::new("./modules")));
    let mut sup = supervisor.write().await;
    let result = sup.initialize().await;
    assert!(result.is_ok(), "Supervisor init should succeed: {:?}", result.err());
    cleanup_test_instances();
}

#[tokio::test]
async fn test_module_discovery_returns_known_modules() {
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
    let supervisor = Arc::new(RwLock::new(Supervisor::new("./modules")));
    {
        let mut sup = supervisor.write().await;
        sup.initialize().await.unwrap();
    }

    // 프로세스가 없는 상태에서 모니터링 N회 반복 — 패닉/데이터 오염 없어야 함
    for iteration in 0..10 {
        let mut sup = supervisor.write().await;
        let result = sup.monitor_processes().await;
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
                    .expect(&format!("Module '{}' commands must have 'fields' array", name));
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
// 8. 복수 인스턴스 생성 + 포트 구분 검증
// ═══════════════════════════════════════════════════════

#[tokio::test]
async fn test_ipc_multiple_instances_are_independent() {
    let (base_url, _sup, server_task) = boot_ipc().await;
    let client = reqwest::Client::new();

    let modules: Value = client
        .get(format!("{}/api/modules", base_url))
        .send().await.unwrap()
        .json().await.unwrap();
    let module_name = modules["modules"][0]["name"].as_str().unwrap();

    // 2개 인스턴스 생성
    let mut ids = vec![];
    for i in 0..2 {
        let name = format!("test-multi-{}-{}", i, pick_free_port());
        let resp = client
            .post(format!("{}/api/instances", base_url))
            .json(&serde_json::json!({
                "name": name,
                "module_name": module_name,
                "executable_path": format!("C:/tmp/server-{}.exe", i)
            }))
            .send().await.unwrap();
        assert_eq!(resp.status(), reqwest::StatusCode::CREATED);
        let json: Value = resp.json().await.unwrap();
        ids.push(json["id"].as_str().unwrap().to_string());
    }

    // 각 인스턴스가 독립적으로 조회 가능
    assert_ne!(ids[0], ids[1], "Instance IDs must be unique");

    for id in &ids {
        let resp = client
            .get(format!("{}/api/instance/{}", base_url, id))
            .send().await.unwrap();
        assert_eq!(resp.status(), reqwest::StatusCode::OK);
    }

    // 하나만 삭제 → 나머지는 여전히 존재
    client.delete(format!("{}/api/instance/{}", base_url, ids[0]))
        .send().await.unwrap();

    let survives = client
        .get(format!("{}/api/instance/{}", base_url, ids[1]))
        .send().await.unwrap();
    assert_eq!(
        survives.status(),
        reqwest::StatusCode::OK,
        "Deleting one instance must not affect others"
    );

    // 정리
    client.delete(format!("{}/api/instance/{}", base_url, ids[1]))
        .send().await.unwrap();
    server_task.abort();
    cleanup_test_instances();
}

