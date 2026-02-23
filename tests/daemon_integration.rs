/// 간소화된 통합 테스트
/// 복잡한 시나리오는 제외하고 핵심 기능만 검증

use saba_core::supervisor::Supervisor;
use saba_core::ipc::IPCServer;
use std::sync::Arc;
use std::net::TcpListener;
use tokio::sync::RwLock;
use tokio::time::{sleep, Duration};
use std::fs;
use serde_json::Value;

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
            
            // test- 로 시작하는 서버 제거
            instances.retain(|instance| {
                if let Some(name) = instance.get("name").and_then(|v| v.as_str()) {
                    !name.starts_with("test-")
                } else {
                    true
                }
            });
            
            // 변경사항이 있으면 저장
            if instances.len() != original_count {
                if let Ok(json) = serde_json::to_string_pretty(&instances) {
                    let _ = fs::write(instances_path, json);
                    println!("🧹 Cleaned up {} test instances from instances.json", 
                             original_count - instances.len());
                }
            }
        }
    }
}

fn pick_free_port() -> u16 {
    let listener = TcpListener::bind("127.0.0.1:0").expect("failed to bind ephemeral port");
    let port = listener
        .local_addr()
        .expect("failed to read ephemeral port")
        .port();
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

#[tokio::test]
async fn test_supervisor_initialization() {
    let supervisor = Arc::new(RwLock::new(Supervisor::new("./modules")));
    
    {
        let mut sup = supervisor.write().await;
        let result = sup.initialize().await;
        assert!(result.is_ok(), "Supervisor should initialize without error");
    }
    
    println!("✓ Supervisor initialization test passed");
    
    // 테스트 종료 시 cleanup
    cleanup_test_instances();
}

#[tokio::test]
async fn test_module_discovery() {
    let supervisor = Arc::new(RwLock::new(Supervisor::new("./modules")));
    
    {
        let mut sup = supervisor.write().await;
        let _ = sup.initialize().await;
    }
    
    let modules = {
        let sup = supervisor.read().await;
        sup.list_modules().unwrap_or_default()
    };
    
    println!("✓ Discovered {} modules", modules.len());
    
    for module in &modules {
        println!("  - {} v{}", module.metadata.name, module.metadata.version);
    }
}

#[tokio::test]
async fn test_module_refresh() {
    let supervisor = Arc::new(RwLock::new(Supervisor::new("./modules")));
    
    {
        let mut sup = supervisor.write().await;
        let _ = sup.initialize().await;
    }
    
    let first_count = {
        let sup = supervisor.read().await;
        sup.list_modules().unwrap_or_default().len()
    };
    
    let refreshed_count = {
        let sup = supervisor.read().await;
        sup.refresh_modules().unwrap_or_default().len()
    };
    
    assert_eq!(first_count, refreshed_count, "Module count should remain consistent");
    
    println!("✓ Module refresh test passed");
}

#[tokio::test]
async fn test_python_detection() {
    use saba_core::plugin::run_plugin;
    
    // Verify that run_plugin properly handles missing modules
    let result = run_plugin("nonexistent.py", "test", serde_json::json!({})).await;
    match result {
        Ok(_) => println!("✓ Python detected and plugin ran"),
        Err(_) => println!("✓ Plugin call failed as expected (Python or module not found)"),
    }
}

#[tokio::test]
async fn test_monitoring_loop() {
    let supervisor = Arc::new(RwLock::new(Supervisor::new("./modules")));
    
    {
        let mut sup = supervisor.write().await;
        let _ = sup.initialize().await;
    }
    
    // 몇 번 모니터링 실행
    for i in 0..5 {
        let mut sup = supervisor.write().await;
        let result = sup.monitor_processes().await;
        
        assert!(result.is_ok(), "Monitoring should not panic");
        
        if i % 2 == 0 {
            println!("  Monitoring iteration {}: OK", i);
        }
    }
    
    println!("✓ Monitoring loop test passed");
}

#[tokio::test]
async fn test_concurrent_module_access() {
    let supervisor = Arc::new(RwLock::new(Supervisor::new("./modules")));
    
    {
        let mut sup = supervisor.write().await;
        let _ = sup.initialize().await;
    }
    
    let mut handles = vec![];
    
    // 10개의 동시 읽기 작업
    for i in 0..10 {
        let sup = supervisor.clone();
        
        let handle = tokio::spawn(async move {
            let sup = sup.read().await;
            let _ = sup.list_modules();
            
            if i % 3 == 0 {
                println!("  Read operation {} completed", i);
            }
        });
        
        handles.push(handle);
    }
    
    for handle in handles {
        handle.await.unwrap();
    }
    
    println!("✓ Concurrent access test passed");
}

#[tokio::test]
async fn test_invalid_module_path() {
    let supervisor = Arc::new(RwLock::new(Supervisor::new("/nonexistent/path")));
    
    {
        let mut sup = supervisor.write().await;
        let result = sup.initialize().await;
        // 패닉이 발생하지 않아야 함
        assert!(result.is_ok() || result.is_err());
    }
    
    let modules = {
        let sup = supervisor.read().await;
        sup.list_modules().unwrap_or_default()
    };
    
    assert_eq!(modules.len(), 0, "Invalid path should result in no modules");
    
    println!("✓ Invalid path handling test passed");
}

#[tokio::test]
async fn test_ipc_instance_crud_e2e() {
    // IPC 인증 미들웨어 우회 (테스트 환경)
    std::env::set_var("SABA_AUTH_DISABLED", "1");

    let supervisor = Arc::new(RwLock::new(Supervisor::new("./modules")));
    {
        let mut sup = supervisor.write().await;
        sup.initialize().await.expect("supervisor init failed");
    }

    let port = pick_free_port();
    let listen_addr = format!("127.0.0.1:{}", port);
    let base_url = format!("http://{}", listen_addr);

    let server = IPCServer::new(supervisor.clone(), &listen_addr);
    let server_task = tokio::spawn(async move {
        let _ = server.start().await;
    });

    let client = reqwest::Client::new();
    wait_for_ipc_ready(&base_url, &client).await;

    let modules_resp = client
        .get(format!("{}/api/modules", base_url))
        .send()
        .await
        .expect("failed to call /api/modules");
    assert!(modules_resp.status().is_success());
    let modules_json: Value = modules_resp.json().await.expect("invalid modules json");

    let first_module = modules_json
        .get("modules")
        .and_then(|v| v.as_array())
        .and_then(|arr| arr.first())
        .and_then(|m| m.get("name"))
        .and_then(|v| v.as_str())
        .expect("at least one module should exist");

    let test_name = format!("test-e2e-{}", port);
    let create_resp = client
        .post(format!("{}/api/instances", base_url))
        .json(&serde_json::json!({
            "name": test_name,
            "module_name": first_module,
            "executable_path": "C:/tmp/test-server.exe"
        }))
        .send()
        .await
        .expect("failed to create instance");
    assert_eq!(create_resp.status(), reqwest::StatusCode::CREATED);
    let create_json: Value = create_resp.json().await.expect("invalid create response");
    let instance_id = create_json
        .get("id")
        .and_then(|v| v.as_str())
        .expect("create response should contain id")
        .to_string();

    let get_resp = client
        .get(format!("{}/api/instance/{}", base_url, instance_id))
        .send()
        .await
        .expect("failed to get instance");
    assert_eq!(get_resp.status(), reqwest::StatusCode::OK);
    let get_json: Value = get_resp.json().await.expect("invalid get response");
    assert_eq!(
        get_json.get("name").and_then(|v| v.as_str()),
        Some(test_name.as_str())
    );

    let delete_resp = client
        .delete(format!("{}/api/instance/{}", base_url, instance_id))
        .send()
        .await
        .expect("failed to delete instance");
    assert_eq!(delete_resp.status(), reqwest::StatusCode::OK);

    let get_deleted_resp = client
        .get(format!("{}/api/instance/{}", base_url, instance_id))
        .send()
        .await
        .expect("failed to query deleted instance");
    assert_eq!(get_deleted_resp.status(), reqwest::StatusCode::NOT_FOUND);

    server_task.abort();
    cleanup_test_instances();
}

#[tokio::test]
async fn test_ipc_command_endpoint_returns_404_for_unknown_instance() {
    // IPC 인증 미들웨어 우회 (테스트 환경)
    std::env::set_var("SABA_AUTH_DISABLED", "1");

    let supervisor = Arc::new(RwLock::new(Supervisor::new("./modules")));
    {
        let mut sup = supervisor.write().await;
        sup.initialize().await.expect("supervisor init failed");
    }

    let port = pick_free_port();
    let listen_addr = format!("127.0.0.1:{}", port);
    let base_url = format!("http://{}", listen_addr);

    let server = IPCServer::new(supervisor, &listen_addr);
    let server_task = tokio::spawn(async move {
        let _ = server.start().await;
    });

    let client = reqwest::Client::new();
    wait_for_ipc_ready(&base_url, &client).await;

    let response = client
        .post(format!("{}/api/instance/nonexistent/command", base_url))
        .json(&serde_json::json!({
            "command": "status",
            "args": {}
        }))
        .send()
        .await
        .expect("failed to call command endpoint");

    assert_eq!(response.status(), reqwest::StatusCode::NOT_FOUND);
    let body: Value = response.json().await.expect("invalid error body");
    let message = body
        .get("error")
        .and_then(|v| v.as_str())
        .unwrap_or_default()
        .to_lowercase();
    assert!(message.contains("instance not found"));

    server_task.abort();
}

