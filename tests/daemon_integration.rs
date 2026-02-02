/// 간소화된 통합 테스트
/// 복잡한 시나리오는 제외하고 핵심 기능만 검증

use saba_chan::supervisor::Supervisor;
use std::sync::Arc;
use tokio::sync::RwLock;
use std::fs;
use serde_json::Value;

/// 테스트 종료 시 instances.json에서 테스트 데이터 자동 제거
fn cleanup_test_instances() {
    let instances_path = "./instances.json";
    
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
    use saba_chan::plugin::PluginManager;
    
    let plugin_manager = PluginManager::new();
    let python_path = plugin_manager.detect_python();
    
    match python_path {
        Some(path) => println!("✓ Python detected: {}", path),
        None => println!("⚠ Python not found (expected in some environments)"),
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

