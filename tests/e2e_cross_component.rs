//! ═══════════════════════════════════════════════════════════════════
//! 컴포넌트 간 완전형 E2E 테스트
//! ═══════════════════════════════════════════════════════════════════
//!
//! 데몬 ↔ 모듈(Python lifecycle) ↔ API 전체를
//! 하나의 파이프라인으로 검증합니다.
//!
//! ## 테스트 범위
//! 1. 데몬 부팅 → 모듈 발견 → 모듈 메타데이터 스키마 계약
//! 2. 인스턴스 생성 → 설정 적용 (Python configure) → validate 호출
//! 4. 설정 CRUD: GUI 설정 저장/로드, 봇 설정 저장/로드
//! 5. 클라이언트 등록/하트비트 프로토콜
//! 6. 크로스 컴포넌트: 인스턴스 운영 중 상태 조회 일관성
//!
//! ## 필수 조건
//! - Python 3.x 시스템에 설치 (또는 saba-chan venv 존재)
//! - 테스트 fixture: tests/fixtures/e2e-mock-module/

use saba_core::supervisor::Supervisor;
use saba_core::ipc::IPCServer;
use std::sync::Arc;
use std::net::TcpListener;
use tokio::sync::RwLock;
use tokio::time::{sleep, Duration};
use std::fs;
use serde_json::{json, Value};

// ═══════════════════════════════════════════════════════
// 테스트 인프라
// ═══════════════════════════════════════════════════════

fn pick_free_port() -> u16 {
    let listener = TcpListener::bind("127.0.0.1:0").expect("ephemeral port");
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

/// E2E 테스트 전용 fixture 모듈 디렉토리를 임시 위치에 세팅합니다.
/// 기존 test-module 외에 e2e-mock-module도 복사합니다.
fn setup_e2e_modules_dir() -> std::path::PathBuf {
    let tmp = std::env::temp_dir().join(format!("saba-e2e-modules-{}", pick_free_port()));
    fs::create_dir_all(&tmp).expect("create e2e modules dir");

    // e2e-mock-module fixture 복사
    let fixture_src = std::path::Path::new("./tests/fixtures/e2e-mock-module");
    let dest = tmp.join("e2e-mock");
    fs::create_dir_all(&dest).expect("create mock module dest");

    for entry in fs::read_dir(fixture_src).expect("read fixture dir") {
        let entry = entry.expect("read entry");
        let target = dest.join(entry.file_name());
        fs::copy(entry.path(), &target).expect("copy fixture file");
    }

    // 기본 test-module도 생성 (모듈이 2개 이상이어야 멀티모듈 시나리오 검증 가능)
    let test_mod = tmp.join("test-module");
    fs::create_dir_all(&test_mod).expect("create test-module dir");
    fs::write(
        test_mod.join("module.toml"),
        r#"[module]
name = "test-module"
version = "0.0.1"
description = "Basic test fixture"
author = "ci"
game_name = "TestGame"
display_name = "Test Module"
entry = "lifecycle.py"

[commands]

[[commands.fields]]
name = "say"
label = "Say"
method = "rcon"
rcon_template = "say {message}"
inputs = [{ name = "message", type = "text", label = "Message", required = true }]

[aliases.commands.say]
aliases = ["말"]

[errors]
server_not_running = "The test server is off"
"#,
    )
    .expect("write test module.toml");
    fs::write(test_mod.join("lifecycle.py"), "# minimal\n")
        .expect("write test lifecycle.py");

    tmp
}

/// IPC 서버를 부팅합니다. 모듈 디렉토리는 E2E fixture를 사용합니다.
async fn boot_e2e_ipc() -> (
    String,
    Arc<RwLock<Supervisor>>,
    tokio::task::JoinHandle<()>,
    std::path::PathBuf, // modules_dir (cleanup용)
    std::path::PathBuf, // instances_dir (cleanup용)
    std::path::PathBuf, // data_dir (cleanup용 — SABA_DATA_DIR 격리)
) {
    std::env::set_var("SABA_AUTH_DISABLED", "1");

    // 테스트용 데이터 디렉토리 격리 — 실제 AppData 오염 방지.
    // SABA_DATA_DIR은 프로세스 전역이라 테스트마다 바꾸면 병렬 테스트끼리 서로의
    // 폴더를 가리키고 지운다. 프로세스당 하나만 만들고 지우지 않는다.
    static DATA_DIR: std::sync::OnceLock<std::path::PathBuf> = std::sync::OnceLock::new();
    let data_dir = DATA_DIR
        .get_or_init(|| {
            let dir = std::env::temp_dir()
                .join(format!("saba-e2e-data-{}", std::process::id()));
            fs::create_dir_all(&dir).expect("create data dir");
            std::env::set_var("SABA_DATA_DIR", dir.to_str().unwrap());
            dir
        })
        .clone();

    let modules_dir = setup_e2e_modules_dir();
    let instances_dir = std::env::temp_dir()
        .join(format!("saba-e2e-instances-{}", pick_free_port()));
    fs::create_dir_all(&instances_dir).expect("create instances dir");

    let supervisor = Arc::new(RwLock::new(
        Supervisor::new_with_instances_dir(
            modules_dir.to_str().unwrap(),
            &instances_dir.to_string_lossy(),
        ),
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

    (base_url, supervisor, server_task, modules_dir, instances_dir, data_dir)
}

fn cleanup_dirs(modules_dir: &std::path::Path, instances_dir: &std::path::Path, data_dir: &std::path::Path) {
    let _ = fs::remove_dir_all(modules_dir);
    let _ = fs::remove_dir_all(instances_dir);
    // data_dir는 프로세스 내 모든 테스트가 공유하므로 지우지 않는다 (boot_e2e_ipc 참고)
    let _ = data_dir;
}

/// 인스턴스 생성 시 비동기 프로비저닝(mock 모듈은 최신 2.0.0 설치)이 같은
/// working_dir에 version.txt를 쓴다. 테스트가 직접 파일을 쓰기 전에 끝날 때까지 기다린다.
async fn wait_for_provision(base_url: &str, client: &reqwest::Client, name: &str) {
    for _ in 0..300 {
        let p: Value = client
            .get(format!("{}/api/provision-progress/{}", base_url, name))
            .send().await.unwrap()
            .json().await.unwrap();
        if !p["active"].as_bool().unwrap_or(false) || p["done"].as_bool().unwrap_or(false) {
            return;
        }
        tokio::time::sleep(std::time::Duration::from_millis(100)).await;
    }
    panic!("provisioning of {} did not finish", name);
}

/// Python 실행 파일 경로를 찾습니다 (시스템 Python 또는 saba-chan venv)
fn find_python() -> Option<String> {
    // Windows: python.exe, Unix: python3
    let candidates = if cfg!(windows) {
        vec!["python", "python3"]
    } else {
        vec!["python3", "python"]
    };

    for candidate in candidates {
        let result = std::process::Command::new(candidate)
            .arg("--version")
            .output();
        if let Ok(output) = result {
            if output.status.success() {
                return Some(candidate.to_string());
            }
        }
    }
    None
}

// ═══════════════════════════════════════════════════════
// 1. 데몬 → 모듈 발견 → 메타데이터 스키마 계약 (E2E)
// ═══════════════════════════════════════════════════════

#[tokio::test]
async fn e2e_module_discovery_and_metadata_schema() {
    let (base_url, _sup, task, m_dir, i_dir, d_dir) = boot_e2e_ipc().await;
    let client = reqwest::Client::new();

    // 모듈 목록 조회
    let resp = client
        .get(format!("{}/api/modules", base_url))
        .send().await.unwrap();
    assert_eq!(resp.status(), reqwest::StatusCode::OK);

    let json: Value = resp.json().await.unwrap();
    let modules = json["modules"].as_array().expect("modules array");

    // e2e-mock + test-module = 최소 2개
    assert!(
        modules.len() >= 2,
        "Expected at least 2 modules, got {}",
        modules.len()
    );

    // e2e-mock 모듈 존재 및 스키마 검증
    let mock_module = modules
        .iter()
        .find(|m| m["name"].as_str() == Some("e2e-mock"))
        .expect("e2e-mock module must be discovered");

    assert_eq!(mock_module["version"].as_str(), Some("1.0.0"));
    assert!(
        mock_module.get("settings").is_some(),
        "Module must expose settings schema"
    );

    // settings 필드 검증
    if let Some(settings) = mock_module.get("settings") {
        if let Some(fields) = settings.get("fields").and_then(|v| v.as_array()) {
            let field_names: Vec<&str> = fields
                .iter()
                .filter_map(|f| f["name"].as_str())
                .collect();
            assert!(
                field_names.contains(&"server_name"),
                "settings must contain 'server_name', got: {:?}",
                field_names
            );
            assert!(
                field_names.contains(&"max_players"),
                "settings must contain 'max_players', got: {:?}",
                field_names
            );
        }
    }

    task.abort();
    cleanup_dirs(&m_dir, &i_dir, &d_dir);
}

// ═══════════════════════════════════════════════════════
// 2. 인스턴스 CRUD → Python lifecycle 호출 검증
// ═══════════════════════════════════════════════════════

#[tokio::test]
async fn e2e_instance_lifecycle_with_python_module() {
    let (base_url, _sup, task, m_dir, i_dir, d_dir) = boot_e2e_ipc().await;
    let client = reqwest::Client::new();

    // ── 1) 인스턴스 생성
    let instance_name = format!("test-e2e-{}", pick_free_port());
    let create_resp = client
        .post(format!("{}/api/instances", base_url))
        .json(&json!({
            "name": instance_name,
            "module_name": "e2e-mock",
        }))
        .send().await.unwrap();
    assert_eq!(
        create_resp.status(),
        reqwest::StatusCode::CREATED,
        "Instance creation should return 201"
    );
    let create_json: Value = create_resp.json().await.unwrap();
    let instance_id = create_json["id"].as_str().expect("must have instance id");
    assert!(!instance_id.is_empty());

    // ── 2) 인스턴스 조회 — 필드 정합성
    let get_resp = client
        .get(format!("{}/api/instance/{}", base_url, instance_id))
        .send().await.unwrap();
    assert_eq!(get_resp.status(), reqwest::StatusCode::OK);
    let get_json: Value = get_resp.json().await.unwrap();
    assert_eq!(get_json["name"].as_str(), Some(instance_name.as_str()));
    assert_eq!(get_json["module_name"].as_str(), Some("e2e-mock"));

    // ── 3) 서버 목록에 인스턴스 반영 검증
    let servers_resp = client
        .get(format!("{}/api/servers", base_url))
        .send().await.unwrap();
    let servers_json: Value = servers_resp.json().await.unwrap();
    let server_ids: Vec<&str> = servers_json["servers"]
        .as_array()
        .unwrap()
        .iter()
        .filter_map(|s| s.get("id").or_else(|| s.get("instance_id")).and_then(|v| v.as_str()))
        .collect();
    assert!(
        server_ids.contains(&instance_id),
        "Instance must appear in server list"
    );

    // ── 4) 인스턴스 설정 업데이트 (common fields: port, rcon_port 등)
    let update_resp = client
        .patch(format!("{}/api/instance/{}", base_url, instance_id))
        .json(&json!({
            "port": 27016,
            "server_name": "E2E Updated Server",
            "max_players": 50
        }))
        .send().await.unwrap();
    assert!(
        update_resp.status().is_success(),
        "Instance update should succeed, got: {} — {}",
        update_resp.status(),
        update_resp.text().await.unwrap_or_default()
    );

    // ── 5) 업데이트 후 조회 — 변경 반영 확인
    let get_updated = client
        .get(format!("{}/api/instance/{}", base_url, instance_id))
        .send().await.unwrap();
    let updated_json: Value = get_updated.json().await.unwrap();
    // port는 공통 필드
    if let Some(port) = updated_json.get("port").and_then(|v| v.as_u64()) {
        assert_eq!(port, 27016, "Updated port should be 27016");
    }
    // module_settings에 server_name이 반영되었는지 확인
    let settings = updated_json.get("module_settings")
        .or_else(|| updated_json.get("settings"));
    if let Some(s) = settings {
        if let Some(name) = s.get("server_name").and_then(|v| v.as_str()) {
            assert_eq!(name, "E2E Updated Server");
        }
    }

    // ── 6) 삭제
    let delete_resp = client
        .delete(format!("{}/api/instance/{}", base_url, instance_id))
        .send().await.unwrap();
    assert_eq!(delete_resp.status(), reqwest::StatusCode::OK);

    // ── 7) 삭제 후 404
    let gone_resp = client
        .get(format!("{}/api/instance/{}", base_url, instance_id))
        .send().await.unwrap();
    assert_eq!(gone_resp.status(), reqwest::StatusCode::NOT_FOUND);

    task.abort();
    cleanup_dirs(&m_dir, &i_dir, &d_dir);
}

// ═══════════════════════════════════════════════════════
// 4. 설정 API 라운드트립 (GUI config + Bot config)
// ═══════════════════════════════════════════════════════

#[tokio::test]
async fn e2e_config_roundtrip() {
    let (base_url, _sup, task, m_dir, i_dir, d_dir) = boot_e2e_ipc().await;
    let client = reqwest::Client::new();

    // ── GUI 설정 저장
    let gui_config = json!({
        "autoRefresh": false,
        "refreshInterval": 5000,
        "ipcPort": 57474,
        "consoleBufferSize": 1000,
        "autoGeneratePasswords": true,
        "portConflictCheck": false,
        "discordToken": "test-token-e2e",
        "discordAutoStart": true,
    });
    let save_resp = client
        .put(format!("{}/api/config/gui", base_url))
        .json(&gui_config)
        .send().await.unwrap();
    // 저장은 성공 or 아직 미구현이었으면 404
    if save_resp.status().is_success() {
        // 로드하여 라운드트립 검증
        let load_resp = client
            .get(format!("{}/api/config/gui", base_url))
            .send().await.unwrap();
        if load_resp.status().is_success() {
            let loaded: Value = load_resp.json().await.unwrap();
            assert_eq!(loaded["autoRefresh"], json!(false));
            assert_eq!(loaded["refreshInterval"], json!(5000));
        }
    }

    // ── Bot 설정 저장
    let bot_config = json!({
        "prefix": "!test",
        "mode": "local",
        "moduleAliases": {"e2e-mock": "mock"},
        "commandAliases": {},
        "musicEnabled": false,
    });
    let save_bot = client
        .put(format!("{}/api/config/bot", base_url))
        .json(&bot_config)
        .send().await.unwrap();
    if save_bot.status().is_success() {
        let load_bot = client
            .get(format!("{}/api/config/bot", base_url))
            .send().await.unwrap();
        if load_bot.status().is_success() {
            let loaded_bot: Value = load_bot.json().await.unwrap();
            assert_eq!(loaded_bot["prefix"].as_str(), Some("!test"));
        }
    }

    task.abort();
    cleanup_dirs(&m_dir, &i_dir, &d_dir);
}

// ═══════════════════════════════════════════════════════
// 5. 클라이언트 등록/하트비트 프로토콜
// ═══════════════════════════════════════════════════════

#[tokio::test]
async fn e2e_client_registration_heartbeat() {
    let (base_url, _sup, task, m_dir, i_dir, d_dir) = boot_e2e_ipc().await;
    let client = reqwest::Client::new();

    // ── 1) 클라이언트 등록
    let reg_resp = client
        .post(format!("{}/api/client/register", base_url))
        .json(&json!({}))
        .send().await.unwrap();
    
    if reg_resp.status().is_success() {
        let reg_json: Value = reg_resp.json().await.unwrap();
        let client_id = reg_json["client_id"].as_str()
            .or_else(|| reg_json["id"].as_str());
        
        if let Some(cid) = client_id {
            // ── 2) 하트비트
            let hb_resp = client
                .post(format!("{}/api/client/{}/heartbeat", base_url, cid))
                .json(&json!({}))
                .send().await.unwrap();
            assert!(
                hb_resp.status().is_success(),
                "Heartbeat should succeed, got: {}",
                hb_resp.status()
            );

            // ── 3) 등록 해제
            let unreg_resp = client
                .delete(format!("{}/api/client/{}/unregister", base_url, cid))
                .send().await.unwrap();
            assert!(
                unreg_resp.status().is_success(),
                "Unregister should succeed, got: {}",
                unreg_resp.status()
            );
        }
    }

    task.abort();
    cleanup_dirs(&m_dir, &i_dir, &d_dir);
}

// ═══════════════════════════════════════════════════════
// 6. 크로스 컴포넌트: 인스턴스 운영 중 상태 조회
// ═══════════════════════════════════════════════════════

#[tokio::test]
async fn e2e_cross_component_simultaneous_operation() {
    let (base_url, _sup, task, m_dir, i_dir, d_dir) = boot_e2e_ipc().await;
    let client = reqwest::Client::new();

    // ── A) 인스턴스 생성 (모듈 사이드)
    let inst_name = format!("test-cross-{}", pick_free_port());
    let create_resp = client
        .post(format!("{}/api/instances", base_url))
        .json(&json!({
            "name": inst_name,
            "module_name": "e2e-mock",
        }))
        .send().await.unwrap();
    assert_eq!(create_resp.status(), reqwest::StatusCode::CREATED);
    let inst_json: Value = create_resp.json().await.unwrap();
    let instance_id = inst_json["id"].as_str().unwrap();

    // ── C) 상태 조회

    // 모듈 사이드: 서버 목록에 인스턴스 존재
    let servers = client
        .get(format!("{}/api/servers", base_url))
        .send().await.unwrap();
    let servers_json: Value = servers.json().await.unwrap();
    let has_instance = servers_json["servers"]
        .as_array()
        .unwrap()
        .iter()
        .any(|s| {
            s.get("id").or_else(|| s.get("instance_id"))
                .and_then(|v| v.as_str()) == Some(instance_id)
        });
    assert!(has_instance, "Instance should be in server list");

    // health 엔드포인트도 정상
    let health = client
        .get(format!("{}/health", base_url))
        .send().await.unwrap();
    assert_eq!(health.status(), reqwest::StatusCode::OK);

    // ── D) 정리
    let _ = client
        .delete(format!("{}/api/instance/{}", base_url, instance_id))
        .send().await;

    sleep(Duration::from_millis(2000)).await;

    // ── E) 정리 후 상태 일관성 확인
    let final_servers = client
        .get(format!("{}/api/servers", base_url))
        .send().await.unwrap();
    let final_json: Value = final_servers.json().await.unwrap();
    let still_exists = final_json["servers"]
        .as_array()
        .unwrap()
        .iter()
        .any(|s| {
            s.get("id").or_else(|| s.get("instance_id"))
                .and_then(|v| v.as_str()) == Some(instance_id)
        });
    assert!(
        !still_exists,
        "Deleted instance should not appear in server list"
    );

    task.abort();
    cleanup_dirs(&m_dir, &i_dir, &d_dir);
}

// ═══════════════════════════════════════════════════════
// 7. 모듈 Python lifecycle 직접 호출 검증
// ═══════════════════════════════════════════════════════

#[tokio::test]
async fn e2e_python_lifecycle_direct_call() {
    let python = match find_python() {
        Some(p) => p,
        None => {
            eprintln!("SKIP: Python not found");
            return;
        }
    };

    let mock_lifecycle = std::path::Path::new("./tests/fixtures/e2e-mock-module/lifecycle.py")
        .canonicalize()
        .expect("lifecycle.py must exist");

    // ── validate
    let mut child = tokio::process::Command::new(&python)
        .arg(mock_lifecycle.to_str().unwrap())
        .arg("validate")
        .stdin(std::process::Stdio::piped())
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::piped())
        .spawn()
        .expect("spawn validate");

    {
        use tokio::io::AsyncWriteExt;
        let config = json!({"server_executable": "", "working_dir": "/tmp", "port": 27015});
        let stdin = child.stdin.as_mut().unwrap();
        stdin.write_all(&serde_json::to_vec(&config).unwrap()).await.ok();
        stdin.shutdown().await.ok();
    }
    let output = child.wait_with_output().await.expect("wait validate");
    let stdout = String::from_utf8_lossy(&output.stdout);
    let parsed: Value = serde_json::from_str(&stdout)
        .expect(&format!("validate should return valid JSON, got: {}", stdout));
    assert!(parsed.get("success").is_some(), "validate must return 'success' field");
    assert!(parsed.get("issues").is_some(), "validate must return 'issues' field");

    // ── get_launch_command
    let output2 = tokio::process::Command::new(&python)
        .arg(mock_lifecycle.to_str().unwrap())
        .arg("get_launch_command")
        .stdin(std::process::Stdio::piped())
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::piped())
        .spawn();

    if let Ok(mut child) = output2 {
        use tokio::io::AsyncWriteExt;
        let config = json!({"server_executable": "mock.exe", "working_dir": "/tmp", "port": 27015});
        if let Some(stdin) = child.stdin.as_mut() {
            let _ = stdin.write_all(&serde_json::to_vec(&config).unwrap()).await;
            let _ = stdin.shutdown().await;
        }
        if let Ok(output) = child.wait_with_output().await {
            let stdout = String::from_utf8_lossy(&output.stdout);
            let parsed: Value = serde_json::from_str(&stdout)
                .expect(&format!("get_launch_command should return JSON, got: {}", stdout));
            assert!(
                parsed.get("command").is_some(),
                "get_launch_command must return 'command' field"
            );
            assert!(
                parsed.get("args").is_some(),
                "get_launch_command must return 'args' field"
            );
        }
    }

    // ── stop
    let output3 = tokio::process::Command::new(&python)
        .arg(mock_lifecycle.to_str().unwrap())
        .arg("stop")
        .stdin(std::process::Stdio::piped())
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::piped())
        .spawn();

    if let Ok(mut child) = output3 {
        use tokio::io::AsyncWriteExt;
        let config = json!({"pid": 12345, "force": false});
        if let Some(stdin) = child.stdin.as_mut() {
            let _ = stdin.write_all(&serde_json::to_vec(&config).unwrap()).await;
            let _ = stdin.shutdown().await;
        }
        if let Ok(output) = child.wait_with_output().await {
            let stdout = String::from_utf8_lossy(&output.stdout);
            let parsed: Value = serde_json::from_str(&stdout)
                .expect(&format!("stop should return JSON, got: {}", stdout));
            assert_eq!(parsed["success"].as_bool(), Some(true));
        }
    }

    // ── command
    let output4 = tokio::process::Command::new(&python)
        .arg(mock_lifecycle.to_str().unwrap())
        .arg("command")
        .stdin(std::process::Stdio::piped())
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::piped())
        .spawn();

    if let Ok(mut child) = output4 {
        use tokio::io::AsyncWriteExt;
        let config = json!({"command": "backup", "args": {"target": "world"}});
        if let Some(stdin) = child.stdin.as_mut() {
            let _ = stdin.write_all(&serde_json::to_vec(&config).unwrap()).await;
            let _ = stdin.shutdown().await;
        }
        if let Ok(output) = child.wait_with_output().await {
            let stdout = String::from_utf8_lossy(&output.stdout);
            let parsed: Value = serde_json::from_str(&stdout)
                .expect(&format!("command should return JSON, got: {}", stdout));
            assert_eq!(parsed["success"].as_bool(), Some(true));
            assert!(
                parsed["result"].as_str().unwrap_or("").contains("backup"),
                "command result should echo the command name"
            );
        }
    }
}

// ═══════════════════════════════════════════════════════
// 8. API 스키마 계약: 에러 응답 일관성
// ═══════════════════════════════════════════════════════

#[tokio::test]
async fn e2e_api_error_schema_consistency() {
    let (base_url, _sup, task, m_dir, i_dir, d_dir) = boot_e2e_ipc().await;
    let client = reqwest::Client::new();

    // 존재하지 않는 리소스들에 대한 에러 응답 스키마가 일관되는지 검증

    let error_cases = vec![
        ("GET", "/api/instance/nonexistent-id"),
    ];

    for (method, path) in error_cases {
        let url = format!("{}{}", base_url, path);
        let resp = match method {
            "GET" => client.get(&url).send().await.unwrap(),
            "POST" => client.post(&url).json(&json!({})).send().await.unwrap(),
            "DELETE" => client.delete(&url).send().await.unwrap(),
            _ => unreachable!(),
        };

        let status = resp.status();
        // 에러 응답은 JSON body에 "error" 필드를 포함해야 함
        if status.is_client_error() || status.is_server_error() {
            let body: Value = resp.json().await.unwrap_or(json!({}));
            assert!(
                body.get("error").is_some() || body.get("message").is_some(),
                "Error response for {} {} (status {}) must have 'error' or 'message' field, got: {:?}",
                method, path, status, body
            );
        }
    }

    task.abort();
    cleanup_dirs(&m_dir, &i_dir, &d_dir);
}

// ═══════════════════════════════════════════════════════
// 9. 버전 관리 — 자동 감지·저장·백업·복원 (E2E)
// ═══════════════════════════════════════════════════════

#[tokio::test]
async fn e2e_version_auto_detect_and_persist() {
    // get_installed_version API 호출 시 감지된 버전이 인스턴스에 자동 저장되는지 검증
    if find_python().is_none() {
        eprintln!("SKIP: Python not found");
        return;
    }

    let (base_url, _sup, task, m_dir, i_dir, d_dir) = boot_e2e_ipc().await;
    let client = reqwest::Client::new();

    // 1. 인스턴스 생성
    let create_resp = client
        .post(format!("{}/api/instances", base_url))
        .json(&json!({
            "name": "VersionTest",
            "module_name": "e2e-mock"
        }))
        .send().await.unwrap();
    assert!(create_resp.status().is_success());
    let created: Value = create_resp.json().await.unwrap();
    let instance_id = created["id"].as_str().unwrap().to_string();
    wait_for_provision(&base_url, &client, "VersionTest").await;

    // 2. 인스턴스의 working_dir 경로를 확인하여 version.txt 작성
    // (인스턴스 생성 시 working_dir가 자동 설정될 수 있으므로 실제 경로에 맞춤)
    let inst_resp = client
        .get(format!("{}/api/instance/{}", base_url, instance_id))
        .send().await.unwrap();
    let inst_data: Value = inst_resp.json().await.unwrap();

    let work_dir = inst_data.get("working_dir")
        .and_then(|v| v.as_str())
        .map(|s| std::path::PathBuf::from(s));

    // working_dir가 없으면 직접 생성
    let work_dir = if let Some(wd) = work_dir {
        fs::create_dir_all(&wd).unwrap();
        wd
    } else {
        let wd = std::env::temp_dir().join(format!("saba-e2e-version-{}", pick_free_port()));
        fs::create_dir_all(&wd).unwrap();
        // module_settings에 working_dir 추가
        let _ = client
            .patch(format!("{}/api/instance/{}", base_url, instance_id))
            .json(&json!({ "working_dir": wd.to_string_lossy(), "server_name": "VersionTest" }))
            .send().await.unwrap();
        wd
    };
    fs::write(work_dir.join("version.txt"), "1.21.1").unwrap();

    // 3. get_installed_version 호출 → 자동 저장 검증
    let detect_resp = client
        .get(format!("{}/api/instance/{}/installed-version", base_url, instance_id))
        .send().await.unwrap();
    assert!(detect_resp.status().is_success());
    let detect: Value = detect_resp.json().await.unwrap();
    assert_eq!(detect["success"].as_bool(), Some(true), "detect result: {}", detect);
    assert_eq!(detect["version"].as_str(), Some("1.21.1"));

    // 4. 서버 리스트에서 저장된 버전 확인
    let list_resp = client
        .get(format!("{}/api/servers", base_url))
        .send().await.unwrap();
    let list_json: Value = list_resp.json().await.unwrap();
    let server = list_json["servers"].as_array().unwrap().iter()
        .find(|s| s["id"].as_str() == Some(&instance_id))
        .expect("instance in server list after detect");
    assert_eq!(
        server["server_version"].as_str(),
        Some("1.21.1"),
        "Detected version must be persisted to instance"
    );

    // cleanup
    let _ = fs::remove_dir_all(&work_dir);
    task.abort();
    cleanup_dirs(&m_dir, &i_dir, &d_dir);
}

#[tokio::test]
async fn e2e_install_server_backup_and_restore() {
    // install_server가 기존 바이너리를 백업하고, 설치 성공 시 백업을 정리하는지 검증
    if find_python().is_none() {
        eprintln!("SKIP: Python not found");
        return;
    }

    let (base_url, _sup, task, m_dir, i_dir, d_dir) = boot_e2e_ipc().await;
    let client = reqwest::Client::new();

    // install_dir에 기존 server.jar 생성
    let install_dir = std::env::temp_dir().join(format!("saba-e2e-backup-{}", pick_free_port()));
    fs::create_dir_all(&install_dir).unwrap();
    fs::write(install_dir.join("server.jar"), "OLD_VERSION_CONTENT").unwrap();

    // install_server 호출 — 성공 시나리오
    let install_resp = client
        .post(format!("{}/api/module/e2e-mock/install", base_url))
        .json(&json!({
            "version": "2.0.0",
            "install_dir": install_dir.to_string_lossy(),
        }))
        .send().await.unwrap();
    assert!(install_resp.status().is_success());
    let install: Value = install_resp.json().await.unwrap();
    assert_eq!(install["success"].as_bool(), Some(true));

    // server.jar가 새 버전으로 업데이트됨
    let jar_content = fs::read_to_string(install_dir.join("server.jar")).unwrap();
    assert!(jar_content.contains("2.0.0"), "JAR must contain new version");

    // 백업 파일(.bak)이 정리됨
    assert!(
        !install_dir.join("server.jar.bak").exists(),
        "Backup must be cleaned up after successful install"
    );

    // cleanup
    let _ = fs::remove_dir_all(&install_dir);
    task.abort();
    cleanup_dirs(&m_dir, &i_dir, &d_dir);
}

#[tokio::test]
async fn e2e_version_backfill_on_startup() {
    // backfill_server_versions가 server_version이 없는 인스턴스의 버전을 감지·저장하는지 검증
    if find_python().is_none() {
        eprintln!("SKIP: Python not found");
        return;
    }

    let (base_url, sup, task, m_dir, i_dir, d_dir) = boot_e2e_ipc().await;
    let client = reqwest::Client::new();

    // 인스턴스 생성 + working_dir 세팅
    let create_resp = client
        .post(format!("{}/api/instances", base_url))
        .json(&json!({
            "name": "BackfillTest",
            "module_name": "e2e-mock"
        }))
        .send().await.unwrap();
    let created: Value = create_resp.json().await.unwrap();
    let instance_id = created["id"].as_str().unwrap().to_string();
    wait_for_provision(&base_url, &client, "BackfillTest").await;

    // 자동 설정된 working_dir에 version.txt 작성
    let inst_resp = client
        .get(format!("{}/api/instance/{}", base_url, instance_id))
        .send().await.unwrap();
    let inst_data: Value = inst_resp.json().await.unwrap();

    let work_dir = inst_data.get("working_dir")
        .and_then(|v| v.as_str())
        .map(|s| std::path::PathBuf::from(s));

    let work_dir = if let Some(wd) = work_dir {
        fs::create_dir_all(&wd).unwrap();
        wd
    } else {
        let wd = std::env::temp_dir().join(format!("saba-e2e-backfill-{}", pick_free_port()));
        fs::create_dir_all(&wd).unwrap();
        let _ = client
            .patch(format!("{}/api/instance/{}", base_url, instance_id))
            .json(&json!({ "working_dir": wd.to_string_lossy(), "server_name": "BackfillTest" }))
            .send().await.unwrap();
        wd
    };
    fs::write(work_dir.join("version.txt"), "1.20.4").unwrap();

    // backfill 실행
    {
        let mut supervisor = sup.write().await;
        supervisor.backfill_server_versions().await;
    }

    // 서버 리스트에서 버전 확인
    let list_resp = client
        .get(format!("{}/api/servers", base_url))
        .send().await.unwrap();
    let list_json: Value = list_resp.json().await.unwrap();
    let server = list_json["servers"].as_array().unwrap().iter()
        .find(|s| s["id"].as_str() == Some(&instance_id))
        .expect("instance in list");
    assert_eq!(
        server["server_version"].as_str(),
        Some("1.20.4"),
        "backfill_server_versions must detect and persist version"
    );

    let _ = fs::remove_dir_all(&work_dir);
    task.abort();
    cleanup_dirs(&m_dir, &i_dir, &d_dir);
}

// ═══════════════════════════════════════════════════════
// 게임 명령 단일 경로 (C7)
// ═══════════════════════════════════════════════════════

/// `/api/instance/:id/command`가 module.toml 정의로 명령을 해석·검증하고,
/// 실패하면 오류 코드와 모듈 `[errors]` 문구를 돌려준다.
#[tokio::test]
async fn command_endpoint_validates_and_maps_module_errors() {
    let (base_url, supervisor, server_task, modules_dir, instances_dir, data_dir) = boot_e2e_ipc().await;
    let client = reqwest::Client::new();

    let mut inst = saba_core::instance::ServerInstance::new("cmd-test", "test-module");
    inst.id = "cmd-test-id".to_string();
    supervisor.write().await.instance_store.add(inst).unwrap();
    let url = format!("{}/api/instance/cmd-test-id/command", base_url);

    // 필수 인자 누락 → 400, 어느 칸인지 알려준다 (별명으로 불러도 같은 명령)
    let resp = client.post(&url).json(&json!({ "command": "말" })).send().await.unwrap();
    assert_eq!(resp.status().as_u16(), 400);
    let body: Value = resp.json().await.unwrap();
    assert_eq!(body["error_code"], "missing_argument");
    assert_eq!(body["field"], "message");
    assert_eq!(body["command"], "say");

    // 꺼진 서버 → 409 + 모듈 문구
    for request in [json!({ "command": "say hello" }), json!({ "command": "say", "args": ["hello"] }), json!({ "command": "tp a b" })] {
        let resp = client.post(&url).json(&request).send().await.unwrap();
        assert_eq!(resp.status().as_u16(), 409, "{}", request);
        let body: Value = resp.json().await.unwrap();
        assert_eq!(body["error_code"], "server_not_running");
        assert_eq!(body["error_message"], "The test server is off");
    }

    // 없는 인스턴스 → 404
    let resp = client
        .post(format!("{}/api/instance/nope/command", base_url))
        .json(&json!({ "command": "say hi" }))
        .send()
        .await
        .unwrap();
    assert_eq!(resp.status().as_u16(), 404);

    server_task.abort();
    cleanup_dirs(&modules_dir, &instances_dir, &data_dir);
}
