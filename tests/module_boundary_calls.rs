use axum::{extract::{Path, State}, http::StatusCode, Json};
use saba_core::{
    daemon_log::DaemonLogBuffer,
    instance::ServerInstance,
    ipc::{handlers::{instance, managed, server}, internal::into_internal, IPCServer, ProvisionProgress},
    supervisor::Supervisor,
};
use serde_json::{json, Value};
use std::{fs, path::{Path as FsPath, PathBuf}, sync::{Arc, OnceLock}, time::Duration};
use tokio::sync::RwLock;

const MODULE: &str = "boundary-errors";
static TEST_LOCK: tokio::sync::Mutex<()> = tokio::sync::Mutex::const_new(());

fn test_root() -> &'static FsPath {
    static ROOT: OnceLock<PathBuf> = OnceLock::new();
    ROOT.get_or_init(|| {
        let root = tempfile::tempdir().unwrap().keep();
        for name in ["data", "modules", "extensions", "appdata", "localappdata", "temp"] {
            fs::create_dir_all(root.join(name)).unwrap();
        }
        for (key, name) in [
            ("SABA_DATA_DIR", "data"), ("SABA_MODULES_PATH", "modules"), ("SABA_EXTENSIONS_DIR", "extensions"),
            ("APPDATA", "appdata"), ("LOCALAPPDATA", "localappdata"), ("TEMP", "temp"), ("TMP", "temp"),
        ] {
            std::env::set_var(key, root.join(name));
        }
        let module = root.join("modules").join(MODULE);
        fs::create_dir_all(&module).unwrap();
        fs::write(module.join("module.toml"), r#"[module]
schema_version = 1
name = "boundary-errors"
version = "1.0.0"
entry = "lifecycle.py"

[install]
method = "download"
install_subdir = "server"

[[queries]]
name = "path"
function = "query_path"
"#).unwrap();
        fs::write(module.join("lifecycle.py"), r#"import json
import sys

sys.stdin.read()
if sys.argv[1] == "functions":
    result = {"success": True, "functions": ["query_path", "stop", "list_versions", "install_server"]}
elif sys.argv[1] == "install_server":
    raise RuntimeError("install_server must not run after list_versions fails")
else:
    result = {"success": False, "error_code": "module_path_outside", "message": "blocked world", "path": "../blocked-world"}
print(json.dumps(result))
"#).unwrap();
        saba_core::sandbox::set_consent(saba_core::sandbox::Consent::Unisolated, "module-boundary-errors", true).unwrap();
        root
    }).as_path()
}

async fn boot(case: &str) -> IPCServer {
    let root = test_root();
    let mut supervisor = Supervisor::new_with_instances_dir(
        root.join("modules").to_str().unwrap(), root.join(case).to_str().unwrap(),
    );
    supervisor.initialize().await.unwrap();
    IPCServer::new(Arc::new(RwLock::new(supervisor)), "127.0.0.1:0", DaemonLogBuffer::new())
}

async fn add_instance(state: &IPCServer, name: &str) -> String {
    let instance = ServerInstance::new(name, MODULE);
    let id = instance.id.clone();
    state.supervisor.write().await.instance_store.add(instance).unwrap();
    id
}

fn assert_module_failure(body: &Value, function: &str) {
    for (key, expected) in json!({
        "success": false, "error_code": "module_path_outside", "message": "blocked world", "path": "../blocked-world",
        "module": MODULE, "function": function,
    }).as_object().unwrap() {
        assert_eq!(&body[key], expected, "lost {key}: {body}");
    }
}

#[tokio::test]
async fn progress_http_keeps_module_failure_fields() {
    let _serial = TEST_LOCK.lock().await;
    let state = boot("progress").await;
    state.provision_tracker.update("progress", ProvisionProgress {
        step: 0, total: 1, label: "done".into(), message: "install failed".into(), done: true,
        error: None, percent: Some(100), steps: None, details: Default::default(),
    }.with_failure(json!({
        "success": false, "error_code": "module_path_outside", "message": "blocked world", "path": "../blocked-world",
        "module": MODULE, "function": "list_versions",
    })));
    let response = into_internal(instance::get_provision_progress(Path("progress".into()), State(state)).await).await;
    assert_eq!(response.status, StatusCode::OK);
    assert_module_failure(&response.body, "list_versions");
}

#[tokio::test]
async fn query_http_keeps_the_modules_path_failure() {
    let _serial = TEST_LOCK.lock().await;
    let state = boot("query").await;
    let id = add_instance(&state, "query").await;
    let response = into_internal(managed::query_instance_handler(
        Path((id, "path".into())), State(state), Some(Json(json!({ "args": {} }))),
    ).await).await;
    assert_eq!(response.status, StatusCode::OK);
    assert_module_failure(&response.body, "query_path");
}

#[tokio::test]
async fn nonmanaged_stop_http_keeps_the_modules_path_failure() {
    let _serial = TEST_LOCK.lock().await;
    let state = boot("stop").await;
    let id = add_instance(&state, "stop").await;
    let python = saba_core::python_env::get_python_path().await.unwrap();
    let mut process = tokio::process::Command::new(python)
        .args(["-c", "import time; time.sleep(60)"])
        .kill_on_drop(true)
        .spawn().unwrap();
    state.supervisor.read().await.tracker.track(&id, process.id().unwrap()).unwrap();
    let response = into_internal(server::stop_instance(state, &id, false).await).await;
    let _ = process.kill().await;
    assert_eq!(response.status, StatusCode::INTERNAL_SERVER_ERROR);
    assert_module_failure(&response.body, "stop");
}

#[tokio::test]
async fn download_version_failure_reaches_progress_http() {
    let _serial = TEST_LOCK.lock().await;
    let state = boot("download").await;
    let made = into_internal(instance::create_instance(State(state.clone()), Json(json!({
        "name": "download", "module_name": MODULE,
    }))).await).await;
    assert!(made.status.is_success(), "{:?}", made.body);
    let progress = tokio::time::timeout(Duration::from_secs(120), async {
        loop {
            let response = into_internal(instance::get_provision_progress(Path("download".into()), State(state.clone())).await).await;
            if response.body["done"] == true {
                break response;
            }
            tokio::time::sleep(Duration::from_millis(20)).await;
        }
    }).await.expect("download provisioning finished");
    assert_eq!(progress.status, StatusCode::OK);
    assert_module_failure(&progress.body, "list_versions");
}
