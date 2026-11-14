//! 인스턴스 만들기 요청·응답: 새 서버, 갈래(branch), 가져오기(migration). 본체는 `service::instance::create`

use super::*;

/// POST /api/instances - 새 인스턴스 생성
pub async fn create_instance(
    State(state): State<IPCServer>,
    Json(payload): Json<Value>,
) -> axum::response::Response {
    // 요청이 끊겨도(클라이언트 시간 초과) 만들기를 끝까지 마친다: 가져오기는 큰 서버 폴더에 권한을 주는 데 수십 초가 들 수 있고,
    // 중간에 멈추면 인스턴스만 남고 설정 가져오기·원래 설정 보관이 빠진다
    // 따로 돌려도 요청의 출처·언어는 지닌다 (설치 진행 문구가 요청한 화면 언어로 오게)
    match crate::audit::spawn(create_instance_to_the_end(state, payload)).await {
        Ok(response) => response,
        Err(e) => {
            tracing::error!("Instance creation task failed: {}", e);
            super::super::error(StatusCode::INTERNAL_SERVER_ERROR, "internal_server_error", "instance creation failed")
        }
    }
}

async fn create_instance_to_the_end(state: IPCServer, payload: Value) -> axum::response::Response {
    match crate::service::instance::create::create(&state, payload).await {
        Ok(body) => (StatusCode::CREATED, Json(body)).into_response(),
        Err(failure) => failure.into_response(),
    }
}

/// POST /api/instances/check - 만들기 전에 이름·서버 폴더·포트가 다른 서버와 겹치는지 본다 (만들기 화면의 확인 단계).
/// 몸체는 `POST /api/instances`와 같고 아무것도 만들거나 바꾸지 않는다. 만들기가 거절할 값(형식, 없는 프리셋 등)은
/// 같은 오류로 답한다
pub async fn check_new_instance(State(state): State<IPCServer>, Json(payload): Json<Value>) -> axum::response::Response {
    super::super::answer(crate::service::instance::create::check(&state, payload).await)
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    /// 만들기·확인의 거절과 답 본문을 고정한다 (본체를 서비스로 옮기며 한 글자도 바뀌지 않게)
    #[tokio::test]
    async fn create_and_check_answer_with_the_same_bodies() {
        use axum::{extract::State, Json};
        let tmp = tempfile::tempdir().unwrap();
        let modules = tmp.path().join("modules");
        let dir = modules.join("game");
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(dir.join("module.toml"), "[module]\nschema_version = 1\nname = 'game'\nversion = '1.0.0'\nentry = 'lifecycle.py'\n").unwrap();
        std::fs::write(dir.join("lifecycle.py"), "import json\nprint(json.dumps({'success': True}))\n").unwrap();
        crate::sandbox::consent_test_component(&dir.join("lifecycle.py"));
        let mut sup = crate::supervisor::Supervisor::new_with_instances_dir(modules.to_str().unwrap(), tmp.path().join("instances").to_str().unwrap());
        sup.initialize().await.unwrap();
        let state = crate::ipc::IPCServer::new(std::sync::Arc::new(tokio::sync::RwLock::new(sup)), "127.0.0.1:0", crate::daemon_log::DaemonLogBuffer::new());
        let create = |body: serde_json::Value| {
            let state = state.clone();
            async move { crate::ipc::internal::into_internal(super::create_instance(State(state), Json(body)).await).await }
        };
        let check = |body: serde_json::Value| {
            let state = state.clone();
            async move { crate::ipc::internal::into_internal(super::check_new_instance(State(state), Json(body)).await).await }
        };
        let answer = |r: crate::ipc::internal::InternalResponse| (r.status.as_u16(), r.body);

        for (sent, want) in [
            (json!({ "module_name": "game" }), (400, json!({ "success": false, "error": "Invalid request", "error_code": "invalid_request" }))),
            (json!({ "name": "g", "module_name": "game", "managed_start": "yes" }), (400, json!({ "success": false, "error": "managed_start must be true or false", "error_code": "invalid_format", "fields": [{ "field": "managed_start" }] }))),
            (json!({ "name": "g", "module_name": "game", "version": "../x" }), (400, json!({ "success": false, "error": "version must be a version name", "error_code": "invalid_format", "fields": [{ "field": "version" }] }))),
            (json!({ "name": "g", "module_name": "game", "extension_data": [] }), (400, json!({ "success": false, "error": "extension_data must be an object", "error_code": "invalid_format", "fields": [{ "field": "extension_data" }] }))),
            (json!({ "name": "g", "module_name": "game", "version": "1.0" }), (400, json!({ "success": false, "error": "this module does not install a chosen version", "error_code": "invalid_format", "fields": [{ "field": "version" }] }))),
            (json!({ "name": "g", "module_name": "game", "branch_of": "nope" }), (404, json!({ "success": false, "error": "The server to branch from does not exist", "error_code": "branch_parent_not_found" }))),
        ] {
            assert_eq!(answer(create(sent.clone()).await), want, "{sent}");
        }

        let checked = answer(check(json!({ "name": "g", "module_name": "game" })).await);
        assert_eq!(checked, (200, json!({ "success": true, "name": { "value": "g", "taken": false, "suggested": "game" }, "folder": { "custom": false, "path": null, "in_use_by": null, "not_empty": false }, "ports": [] })));
        let (status, made) = answer(create(json!({ "name": "g", "module_name": "game" })).await);
        assert_eq!(status, 201, "{made}");
        let id = made["id"].as_str().unwrap().to_string();
        // 미검증 모듈은 알림을 싣는다
        assert_eq!(made, json!({ "success": true, "id": id, "module_trust": "unverified", "warnings": [{ "code": "module_unverified", "isolated": cfg!(windows), "module": "game" }] }));
        let taken = answer(create(json!({ "name": "G", "module_name": "game" })).await);
        assert_eq!(taken, (409, json!({ "success": false, "error": "An instance named 'G' already exists", "error_code": "name_taken" })));
        let checked = answer(check(json!({ "name": "g", "module_name": "game" })).await).1;
        assert_eq!(checked["name"], json!({ "value": "g", "taken": true, "suggested": "game" }));
    }

    #[tokio::test]
    async fn contract_paths_migration_reports_outside_executables_without_saving_them() {
        use axum::{extract::State, Json};
        let tmp = tempfile::tempdir().unwrap();
        let modules = tmp.path().join("modules");
        let dir = modules.join("game");
        let server = tmp.path().join("server");
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::create_dir_all(&server).unwrap();
        let outside = tmp.path().join("outside.exe");
        std::fs::write(&outside, "outside").unwrap();
        std::fs::write(dir.join("module.toml"), "[module]\nschema_version = 1\nname = 'game'\nversion = '1.0.0'\nentry = 'lifecycle.py'\n").unwrap();
        let result = serde_json::json!({"success":true,"executable_path":outside,"settings":{"motd":"untrusted"}});
        std::fs::write(dir.join("lifecycle.py"), format!("import sys\nprint('{{\"success\":true,\"functions\":[\"import_settings\"]}}' if sys.argv[1] == 'functions' else {:?})\n", result.to_string())).unwrap();
        crate::sandbox::consent_test_component(&dir.join("lifecycle.py"));
        let mut sup = crate::supervisor::Supervisor::new_with_instances_dir(modules.to_str().unwrap(), tmp.path().join("instances").to_str().unwrap());
        for module in sup.module_loader.discover_modules().unwrap() { crate::sandbox::consent_test_component(std::path::Path::new(&module.entry_path())); }
        sup.initialize().await.unwrap();
        let state = crate::ipc::IPCServer::new(std::sync::Arc::new(tokio::sync::RwLock::new(sup)), "127.0.0.1:0", crate::daemon_log::DaemonLogBuffer::new());
        let response = crate::ipc::internal::into_internal(super::create_instance(State(state.clone()), Json(serde_json::json!({"name":"g", "module_name":"game", "migration_source":server}))).await).await;
        assert_eq!(response.status.as_u16(), 422, "{}", response.body);
        assert_eq!(response.body["error_code"], "module_path_outside");
        // 가져오기가 실패했으니 아무것도 만들지 않는다
        assert!(state.supervisor.read().await.instance_store.list().is_empty());
    }

    /// 옮겨 온 서버의 가져오기가 실패하면 서버 폴더를 원래 자리로 돌려놓고 인스턴스를 만들지 않는다
    /// (남은 인스턴스를 지우면 옮겨 온 서버도 함께 지워진다)
    #[tokio::test]
    async fn a_moved_server_goes_back_when_its_import_fails() {
        use axum::{extract::State, Json};
        let tmp = tempfile::tempdir().unwrap();
        let modules = tmp.path().join("modules");
        let dir = modules.join("game");
        let server = tmp.path().join("server");
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::create_dir_all(&server).unwrap();
        std::fs::write(server.join("world.dat"), "world").unwrap();
        let outside = tmp.path().join("outside.exe");
        std::fs::write(&outside, "outside").unwrap();
        std::fs::write(dir.join("module.toml"), "[module]\nschema_version = 1\nname = 'game'\nversion = '1.0.0'\nentry = 'lifecycle.py'\n").unwrap();
        let result = serde_json::json!({"success":true,"executable_path":outside});
        std::fs::write(dir.join("lifecycle.py"), format!("import sys\nprint('{{\"success\":true,\"functions\":[\"import_settings\"]}}' if sys.argv[1] == 'functions' else {:?})\n", result.to_string())).unwrap();
        crate::sandbox::consent_test_component(&dir.join("lifecycle.py"));
        let mut sup = crate::supervisor::Supervisor::new_with_instances_dir(modules.to_str().unwrap(), tmp.path().join("instances").to_str().unwrap());
        for module in sup.module_loader.discover_modules().unwrap() { crate::sandbox::consent_test_component(std::path::Path::new(&module.entry_path())); }
        sup.initialize().await.unwrap();
        let state = crate::ipc::IPCServer::new(std::sync::Arc::new(tokio::sync::RwLock::new(sup)), "127.0.0.1:0", crate::daemon_log::DaemonLogBuffer::new());
        let response = crate::ipc::internal::into_internal(super::create_instance(State(state.clone()), Json(serde_json::json!({"name":"g", "module_name":"game", "migration_source":server, "migration_mode":"move"}))).await).await;
        assert_eq!(response.status.as_u16(), 422, "{}", response.body);
        assert_eq!(std::fs::read_to_string(server.join("world.dat")).unwrap(), "world");
        assert!(state.supervisor.read().await.instance_store.list().is_empty());
        let left: Vec<_> = std::fs::read_dir(tmp.path().join("instances")).unwrap().flatten().map(|e| e.file_name()).filter(|n| n != "order.json" && n != crate::instance::STATE_DIR).collect();
        assert!(left.is_empty(), "instance folders left: {:?}", left);
    }

    /// 가져오면 그 시점을 수동 백업으로 남긴다 (모듈 `[backup] paths`의 월드)
    #[tokio::test]
    async fn an_imported_server_gets_a_first_backup() {
        use axum::{extract::State, Json};
        let tmp = tempfile::tempdir().unwrap();
        let modules = tmp.path().join("modules");
        let dir = modules.join("game");
        let server = tmp.path().join("server");
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::create_dir_all(server.join("world")).unwrap();
        std::fs::write(server.join("world").join("level.dat"), "world").unwrap();
        std::fs::write(dir.join("module.toml"), "[module]\nschema_version = 1\nname = 'game'\nversion = '1.0.0'\nentry = 'lifecycle.py'\n\n[backup]\npaths = ['world']\n").unwrap();
        std::fs::write(dir.join("lifecycle.py"), "import sys\nprint('{\"success\":true,\"functions\":[]}' if sys.argv[1] == 'functions' else '{\"success\":true}')\n").unwrap();
        crate::sandbox::consent_test_component(&dir.join("lifecycle.py"));
        let mut sup = crate::supervisor::Supervisor::new_with_instances_dir(modules.to_str().unwrap(), tmp.path().join("instances").to_str().unwrap());
        for module in sup.module_loader.discover_modules().unwrap() { crate::sandbox::consent_test_component(std::path::Path::new(&module.entry_path())); }
        sup.initialize().await.unwrap();
        let state = crate::ipc::IPCServer::new(std::sync::Arc::new(tokio::sync::RwLock::new(sup)), "127.0.0.1:0", crate::daemon_log::DaemonLogBuffer::new());
        let response = crate::ipc::internal::into_internal(super::create_instance(State(state.clone()), Json(serde_json::json!({"name":"g", "module_name":"game", "migration_source":server}))).await).await;
        assert_eq!(response.status.as_u16(), 201, "{}", response.body);
        let id = response.body["id"].as_str().unwrap().to_string();
        let backups = tmp.path().join("instances").join(&id).join("backups");
        let mut made = Vec::new();
        for _ in 0..100 {
            made = crate::backup::list(&backups);
            if !made.is_empty() {
                break;
            }
            tokio::time::sleep(std::time::Duration::from_millis(100)).await;
        }
        assert_eq!(made.len(), 1, "one backup right after the import");
        assert_eq!(made[0].kind, crate::backup::BackupKind::Manual, "kept out of pruning");
    }

    /// 가져온 설정을 저장하지 못하면 성공으로 알리지 않는다: 저장소에는 가져오기 전 인스턴스(실행 파일·설정 없음)만 남는다
    #[tokio::test]
    async fn an_import_whose_result_cannot_be_saved_is_not_reported_as_created() {
        use axum::{extract::State, Json};
        let tmp = tempfile::tempdir().unwrap();
        // 데몬의 모듈 폴더 아래에 둔다: 그 밖의 모듈은 Windows 격리 실행에서 인스턴스 폴더 쓰기를 받지 못한다
        let modules = crate::plugin::resolve_modules_dir();
        let fixture = tempfile::Builder::new().prefix("import-save-").tempdir_in(&modules).unwrap();
        let dir = fixture.path();
        let name = dir.file_name().unwrap().to_string_lossy().into_owned();
        std::fs::write(dir.join("module.toml"), format!("[module]\nschema_version = 1\nname = '{name}'\nversion = '1.0.0'\nentry = 'lifecycle.py'\n")).unwrap();
        // 가져오는 동안 instance.json 자리가 폴더로 바뀐다 (저장이 실패하는 디스크 상태 대신).
        // 기록은 테스트가 망가뜨린다: Windows 격리 실행은 인스턴스 폴더의 데몬 항목을 바꾸지 못한다
        std::fs::write(dir.join("lifecycle.py"),
            "import json, os, sys, time\nc = json.load(sys.stdin)\nf = sys.argv[1]\n\
             if f == 'functions': print(json.dumps({'success': True, 'functions': ['import_settings']}))\n\
             else:\n    d = c['instance_dir']\n    open(os.path.join(d, 'started'), 'w').close()\n    \
             for _ in range(300):\n        if os.path.exists(os.path.join(d, 'go')): break\n        time.sleep(0.1)\n    \
             print(json.dumps({'success': True, 'settings': {'motd': 'mine'}}))\n",
        ).unwrap();
        crate::sandbox::consent_test_component(&dir.join("lifecycle.py"));
        let instances = tmp.path().join("instances");
        let mut sup = crate::supervisor::Supervisor::new_with_instances_dir(modules.to_str().unwrap(), instances.to_str().unwrap());
        for module in sup.module_loader.discover_modules().unwrap() { crate::sandbox::consent_test_component(std::path::Path::new(&module.entry_path())); }
        sup.initialize().await.unwrap();
        let state = crate::ipc::IPCServer::new(std::sync::Arc::new(tokio::sync::RwLock::new(sup)), "127.0.0.1:0", crate::daemon_log::DaemonLogBuffer::new());
        let server = tmp.path().join("server");
        std::fs::create_dir_all(&server).unwrap();

        let request = serde_json::json!({"name":"g", "module_name":name, "migration_source":server});
        let created = tokio::spawn({
            let state = state.clone();
            async move { crate::ipc::internal::into_internal(super::create_instance(State(state), Json(request)).await).await }
        });
        let mut instance_dir = None;
        for _ in 0..300 {
            instance_dir = std::fs::read_dir(&instances).into_iter().flatten().flatten().map(|e| e.path()).find(|p| p.join("started").exists());
            if instance_dir.is_some() {
                break;
            }
            tokio::time::sleep(std::time::Duration::from_millis(100)).await;
        }
        let instance_dir = instance_dir.expect("the import started");
        let record = instance_dir.join("instance.json");
        std::fs::remove_file(&record).unwrap();
        std::fs::create_dir(&record).unwrap();
        std::fs::write(instance_dir.join("go"), b"").unwrap();

        let response = created.await.unwrap();
        assert_eq!(response.status.as_u16(), 500, "{}", response.body);
        assert_eq!(response.body["error_code"], "save_failed");
        assert!(server.is_dir());
    }

    /// 가져오기가 오래 걸려 클라이언트가 요청을 끊어도(GUI 시간 초과) 데몬은 설정 가져오기까지 마친다:
    /// 중간에 멈추면 기본값인 인스턴스만 남고 되돌릴 원래 설정도 보관되지 않는다
    #[tokio::test]
    async fn an_import_cut_off_by_the_client_still_finishes() {
        use axum::{extract::State, Json};
        let tmp = tempfile::tempdir().unwrap();
        let modules = tmp.path().join("modules");
        let dir = modules.join("game");
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(dir.join("module.toml"), "[module]\nschema_version = 1\nname = 'game'\nversion = '1.0.0'\nentry = 'lifecycle.py'\n").unwrap();
        std::fs::write(dir.join("lifecycle.py"),
            "import json, sys, time\nf = sys.argv[1]\n\
             if f == 'functions': print(json.dumps({'success': True, 'functions': ['import_settings']}))\n\
             else:\n    time.sleep(1.5)\n    print(json.dumps({'success': True, 'settings': {'motd': 'mine'}}))\n").unwrap();
        crate::sandbox::consent_test_component(&dir.join("lifecycle.py"));
        let mut sup = crate::supervisor::Supervisor::new_with_instances_dir(modules.to_str().unwrap(), tmp.path().join("instances").to_str().unwrap());
        for module in sup.module_loader.discover_modules().unwrap() { crate::sandbox::consent_test_component(std::path::Path::new(&module.entry_path())); }
        sup.initialize().await.unwrap();
        let state = crate::ipc::IPCServer::new(std::sync::Arc::new(tokio::sync::RwLock::new(sup)), "127.0.0.1:0", crate::daemon_log::DaemonLogBuffer::new());
        let server = tmp.path().join("server");
        std::fs::create_dir_all(&server).unwrap();

        let request = super::create_instance(State(state.clone()), Json(serde_json::json!({"name":"g", "module_name":"game", "migration_source":server})));
        assert!(tokio::time::timeout(std::time::Duration::from_millis(300), request).await.is_err(), "the client gave up first");

        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(20);
        loop {
            let imported = state.supervisor.read().await.instance_store.list().first().and_then(|i| i.module_settings.get("motd").cloned());
            if imported == Some(serde_json::json!("mine")) {
                break;
            }
            assert!(std::time::Instant::now() < deadline, "the import never finished after the request was dropped");
            tokio::time::sleep(std::time::Duration::from_millis(100)).await;
        }
    }

    /// 가져오기는 끝날 때까지 그 인스턴스의 작업 잠금을 쥔다: 그동안 시작·설정 저장·백업이 데몬 항목을 만들지 않는다
    /// (격리된 가져오기가 끝나면 시작할 때 없던 데몬 항목 자리를 지우므로, 그 사이 데몬이 만든 것이 지워지지 않게)
    #[tokio::test]
    async fn an_import_holds_the_instance_lock_until_it_is_done() {
        use axum::{extract::{Path, State}, Json};
        let tmp = tempfile::tempdir().unwrap();
        let modules = tmp.path().join("modules");
        let dir = modules.join("game");
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(dir.join("module.toml"), "[module]\nschema_version = 1\nname = 'game'\nversion = '1.0.0'\nentry = 'lifecycle.py'\n").unwrap();
        std::fs::write(dir.join("lifecycle.py"),
            "import json, sys, time\nf = sys.argv[1]\n\
             if f == 'functions': print(json.dumps({'success': True, 'functions': ['import_settings']}))\n\
             else:\n    time.sleep(1.5)\n    print(json.dumps({'success': True, 'settings': {'motd': 'mine'}}))\n").unwrap();
        crate::sandbox::consent_test_component(&dir.join("lifecycle.py"));
        let mut sup = crate::supervisor::Supervisor::new_with_instances_dir(modules.to_str().unwrap(), tmp.path().join("instances").to_str().unwrap());
        for module in sup.module_loader.discover_modules().unwrap() { crate::sandbox::consent_test_component(std::path::Path::new(&module.entry_path())); }
        sup.initialize().await.unwrap();
        let state = crate::ipc::IPCServer::new(std::sync::Arc::new(tokio::sync::RwLock::new(sup)), "127.0.0.1:0", crate::daemon_log::DaemonLogBuffer::new());
        let server = tmp.path().join("server");
        std::fs::create_dir_all(&server).unwrap();

        let request = tokio::spawn(super::create_instance(State(state.clone()), Json(serde_json::json!({"name":"g", "module_name":"game", "migration_source":server}))));
        let id = loop {
            if let Some(id) = state.supervisor.read().await.instance_store.list().first().map(|i| i.id.clone()) {
                break id;
            }
            tokio::time::sleep(std::time::Duration::from_millis(20)).await;
        };
        assert!(crate::instance::lock::InstanceLock::take(&id).is_none(), "the import holds the lock");
        let saved = crate::ipc::internal::into_internal(super::update_instance_settings(State(state.clone()), Path(id.clone()), Json(json!({ "motd": "theirs" }))).await).await;
        assert_eq!(saved.status, 409, "{}", saved.body);
        let created = crate::ipc::internal::into_internal(request.await.unwrap()).await;
        assert_eq!(created.status, 201, "{}", created.body);
        assert!(crate::instance::lock::InstanceLock::take(&id).is_some(), "the lock is released once the import is done");
    }

    /// 가져올 서버 폴더는 절대 경로여야 한다: 상대 경로는 데몬의 작업 폴더 기준으로 풀려 엉뚱한 폴더를 연결하거나 옮긴다
    #[tokio::test]
    async fn a_relative_migration_source_is_refused() {
        use axum::{extract::State, Json};
        let tmp = tempfile::tempdir().unwrap();
        let modules = tmp.path().join("modules");
        let dir = modules.join("game");
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(dir.join("module.toml"), "[module]\nschema_version = 1\nname = 'game'\nversion = '1.0.0'\nentry = 'lifecycle.py'\n").unwrap();
        std::fs::write(dir.join("lifecycle.py"), "import json\nprint(json.dumps({'success': True}))\n").unwrap();
        crate::sandbox::consent_test_component(&dir.join("lifecycle.py"));
        let mut sup = crate::supervisor::Supervisor::new_with_instances_dir(modules.to_str().unwrap(), tmp.path().join("instances").to_str().unwrap());
        for module in sup.module_loader.discover_modules().unwrap() { crate::sandbox::consent_test_component(std::path::Path::new(&module.entry_path())); }
        sup.initialize().await.unwrap();
        let state = crate::ipc::IPCServer::new(std::sync::Arc::new(tokio::sync::RwLock::new(sup)), "127.0.0.1:0", crate::daemon_log::DaemonLogBuffer::new());
        for mode in ["link", "move"] {
            let bad = crate::ipc::internal::into_internal(super::create_instance(State(state.clone()), Json(serde_json::json!({"name":"g", "module_name":"game", "migration_source":"rel/dir", "migration_mode":mode}))).await).await;
            assert_eq!((bad.status.as_u16(), bad.body["error_code"].as_str(), bad.body["fields"][0]["field"].as_str()), (400, Some("invalid_format"), Some("migration_source")), "{mode}: {}", bad.body);
        }
        assert!(state.supervisor.read().await.instance_store.list().is_empty());
    }

    /// 가져오기 "옮기기"는 서버 폴더를 인스턴스 폴더 안으로 옮겨 전속시킨다: 모듈은 옮긴 자리를 읽고,
    /// 지우면 서버 파일도 함께 지워진다. 모르는 방식은 아무것도 만들지 않고 거절한다
    #[tokio::test]
    async fn moving_an_existing_server_makes_it_part_of_the_instance() {
        use axum::{extract::{Path, Query, State}, Json};
        let tmp = tempfile::tempdir().unwrap();
        let modules = tmp.path().join("modules");
        let dir = modules.join("game");
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(dir.join("module.toml"), "[module]\nschema_version = 1\nname = 'game'\nversion = '1.0.0'\nentry = 'lifecycle.py'\n").unwrap();
        std::fs::write(dir.join("lifecycle.py"),
            "import json, os, sys\nc = json.load(sys.stdin)\nf = sys.argv[1]\n\
             if f == 'functions': print(json.dumps({'success': True, 'functions': ['import_settings']}))\n\
             else: print(json.dumps({'success': True, 'settings': {'motd': open(os.path.join(c['working_dir'], 'motd.txt')).read(), 'seen_instance_dir': c.get('instance_dir')}}))\n").unwrap();
        crate::sandbox::consent_test_component(&dir.join("lifecycle.py"));
        let mut sup = crate::supervisor::Supervisor::new_with_instances_dir(modules.to_str().unwrap(), tmp.path().join("instances").to_str().unwrap());
        for module in sup.module_loader.discover_modules().unwrap() { crate::sandbox::consent_test_component(std::path::Path::new(&module.entry_path())); }
        sup.initialize().await.unwrap();
        let state = crate::ipc::IPCServer::new(std::sync::Arc::new(tokio::sync::RwLock::new(sup)), "127.0.0.1:0", crate::daemon_log::DaemonLogBuffer::new());
        let server = tmp.path().join("기존 서버");
        std::fs::create_dir_all(&server).unwrap();
        std::fs::write(server.join("motd.txt"), "mine").unwrap();

        let bad = crate::ipc::internal::into_internal(super::create_instance(State(state.clone()), Json(serde_json::json!({"name":"x", "module_name":"game", "migration_source":server, "migration_mode":"copy"}))).await).await;
        assert_eq!(bad.body["error_code"], "invalid_format", "{}", bad.body);
        assert!(server.is_dir() && state.supervisor.read().await.instance_store.list().is_empty());

        let created = crate::ipc::internal::into_internal(super::create_instance(State(state.clone()), Json(serde_json::json!({"name":"g", "module_name":"game", "migration_source":server, "migration_mode":"move"}))).await).await;
        assert_eq!(created.status.as_u16(), 201, "{}", created.body);
        let id = created.body["id"].as_str().unwrap().to_string();
        let (instance, instance_dir) = {
            let sup = state.supervisor.read().await;
            (sup.instance_store.get(&id).unwrap().clone(), sup.instance_store.instance_dir(&id))
        };
        assert!(!server.exists(), "the folder was moved, not copied");
        let moved = instance_dir.join("server");
        assert_eq!(instance.working_dir.as_deref(), Some(moved.to_string_lossy().as_ref()));
        assert_eq!(instance.module_settings["motd"], "mine", "the module read the moved folder");
        // 가져오기도 인스턴스 폴더를 안다 (모듈이 포터블 런타임 같은 것을 서버 폴더 대신 그곳에 둔다)
        assert_eq!(instance.module_settings["seen_instance_dir"], serde_json::json!(instance_dir.to_string_lossy()));
        assert_eq!(crate::instance::migration::linked_folder(instance.working_dir.as_deref(), &instance_dir), None);

        let removed = crate::ipc::internal::into_internal(super::delete_instance(Path(id), State(state.clone()), Query(Default::default())).await).await;
        assert_eq!(removed.status.as_u16(), 200, "{}", removed.body);
        assert!(removed.body.get("linked_folder").is_none());
        assert!(!moved.exists());
    }

    #[tokio::test]
    async fn contract_paths_migration_keeps_declared_external_data_roots_after_reload() {
        use axum::{extract::State, Json};
        let tmp = tempfile::tempdir().unwrap();
        let modules = tmp.path().join("modules");
        let dir = modules.join("game");
        let server = tmp.path().join("server");
        let data = tmp.path().join("external-data");
        for folder in [&dir, &server, &data] { std::fs::create_dir_all(folder).unwrap(); }
        let executable = data.join("server.exe");
        std::fs::write(&executable, "server").unwrap();
        std::fs::write(dir.join("module.toml"), "[module]\nschema_version = 1\nname = 'game'\nversion = '1.0.0'\nentry = 'lifecycle.py'\n\n[[settings.fields]]\nname = 'data_home'\ntype = 'file'\n\n[extensions.saba-core]\ndata_roots = ['{data_home}']\n").unwrap();
        let result = serde_json::json!({"success":true,"executable_path":executable,"settings":{"data_home":data}});
        std::fs::write(dir.join("lifecycle.py"), format!("import sys\nprint('{{\"success\":true,\"functions\":[\"import_settings\"]}}' if sys.argv[1] == 'functions' else {:?})\n", result.to_string())).unwrap();
        crate::sandbox::consent_test_component(&dir.join("lifecycle.py"));
        let mut sup = crate::supervisor::Supervisor::new_with_instances_dir(modules.to_str().unwrap(), tmp.path().join("instances").to_str().unwrap());
        for module in sup.module_loader.discover_modules().unwrap() { crate::sandbox::consent_test_component(std::path::Path::new(&module.entry_path())); }
        sup.initialize().await.unwrap();
        let state = crate::ipc::IPCServer::new(std::sync::Arc::new(tokio::sync::RwLock::new(sup)), "127.0.0.1:0", crate::daemon_log::DaemonLogBuffer::new());
        let response = crate::ipc::internal::into_internal(super::create_instance(State(state.clone()), Json(serde_json::json!({"name":"g", "module_name":"game", "migration_source":server}))).await).await;
        assert_eq!(response.status.as_u16(), 201, "{}", response.body);
        let mut sup = state.supervisor.write().await;
        sup.instance_store.load().unwrap();
        let instance = sup.instance_store.get(response.body["id"].as_str().unwrap()).unwrap();
        assert_eq!(instance.executable_path.as_deref(), executable.to_str());
        assert!(instance.imported_settings.contains("data_home"));
        assert!(!instance.user_settings.contains("data_home"));
        let metadata = sup.module_loader.get_module("game").unwrap().metadata;
        let boundary = crate::module_paths::Boundary::for_instance(instance, &sup.instance_store.instance_dir(&instance.id), Some(&metadata));
        assert!(boundary.resolve(&data.join("deleted/Saves")).is_some());
    }

    /// D3·M1: 이 OS를 지원하지 않는 모듈로는 서버를 만들지 않는다
    #[tokio::test]
    async fn create_refuses_a_module_that_does_not_support_this_os() {
        use axum::extract::State;
        use axum::Json;
        let tmp = tempfile::tempdir().unwrap();
        let modules = tmp.path().join("modules");
        let dir = modules.join("game");
        std::fs::create_dir_all(&dir).unwrap();
        let other = if cfg!(windows) { "linux" } else { "windows" };
        std::fs::write(
            dir.join("module.toml"),
            format!("[module]\nschema_version = 1\nname = \"game\"\nversion = \"1.0.0\"\nentry = \"lifecycle.py\"\n\n[extensions.saba-core]\nplatforms = [\"{other}\"]\n"),
        )
        .unwrap();
        std::fs::write(dir.join("lifecycle.py"), "").unwrap();
        let mut sup = crate::supervisor::Supervisor::new_with_instances_dir(
            modules.to_str().unwrap(),
            tmp.path().join("instances").to_str().unwrap(),
        );
        sup.initialize().await.unwrap();
        let state = crate::ipc::IPCServer::new(std::sync::Arc::new(tokio::sync::RwLock::new(sup)), "127.0.0.1:0", crate::daemon_log::DaemonLogBuffer::new());
        let made = crate::ipc::internal::into_internal(
            super::create_instance(State(state.clone()), Json(json!({ "name": "a", "module_name": "game" }))).await,
        )
        .await;
        assert_eq!(made.status, axum::http::StatusCode::CONFLICT, "{:?}", made.body);
        assert_eq!(made.body["error_code"], "module_platform_unsupported");
        assert!(state.supervisor.read().await.instance_store.list().is_empty());
    }

    /// 새 서버는 사바쨩이 설치할 수 있는 값(`install_options`)으로만 만든다. 가져오는 서버는 판정된 값 그대로 받는다
    #[tokio::test]
    async fn a_new_server_takes_only_installable_options() {
        use axum::extract::State;
        use axum::Json;
        let tmp = tempfile::tempdir().unwrap();
        let modules = tmp.path().join("modules");
        let dir = modules.join("game");
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(
            dir.join("module.toml"),
            "[module]\nschema_version = 1\nname = \"game\"\nversion = \"1.0.0\"\nentry = \"lifecycle.py\"\n\n\
             [[settings.fields]]\nname = \"server_type\"\ntype = \"select\"\noptions = [\"vanilla\", \"spigot\"]\ndefault = \"vanilla\"\ninstall = true\ninstall_options = [\"vanilla\"]\n",
        )
        .unwrap();
        std::fs::write(dir.join("lifecycle.py"), "").unwrap();
        let mut sup = crate::supervisor::Supervisor::new_with_instances_dir(
            modules.to_str().unwrap(),
            tmp.path().join("instances").to_str().unwrap(),
        );
        for module in sup.module_loader.discover_modules().unwrap() { crate::sandbox::consent_test_component(std::path::Path::new(&module.entry_path())); }
        sup.initialize().await.unwrap();
        let state = crate::ipc::IPCServer::new(std::sync::Arc::new(tokio::sync::RwLock::new(sup)), "127.0.0.1:0", crate::daemon_log::DaemonLogBuffer::new());
        let create = |body: serde_json::Value| {
            let state = state.clone();
            async move { crate::ipc::internal::into_internal(super::create_instance(State(state), Json(body)).await).await }
        };

        let refused = create(json!({ "name": "a", "module_name": "game", "module_settings": { "server_type": "spigot" } })).await;
        assert_eq!(refused.status.as_u16(), 400, "{:?}", refused.body);
        assert_eq!(refused.body["error_code"], "not_installable");
        assert_eq!(refused.body["fields"][0]["field"], "server_type");
        let null_source = create(json!({ "name": "a", "module_name": "game", "migration_source": null, "module_settings": { "server_type": "spigot" } })).await;
        assert_eq!(null_source.body["error_code"], "not_installable", "null is not an import");
        assert!(state.supervisor.read().await.instance_store.list().is_empty());

        let made = create(json!({ "name": "a", "module_name": "game", "module_settings": { "server_type": "vanilla" } })).await;
        assert!(made.status.is_success(), "{:?}", made.body);

        let existing = tmp.path().join("old-server");
        std::fs::create_dir_all(&existing).unwrap();
        let imported = create(json!({ "name": "b", "module_name": "game", "migration_source": existing.to_string_lossy(), "module_settings": { "server_type": "spigot" } })).await;
        assert!(imported.status.is_success(), "{:?}", imported.body);
    }

    #[tokio::test]
    async fn create_applies_the_chosen_preset_under_explicit_settings() {
        use axum::extract::State;
        use axum::Json;
        let tmp = tempfile::tempdir().unwrap();
        let modules = tmp.path().join("modules");
        let dir = modules.join("game");
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(
            dir.join("module.toml"),
            "[module]\nschema_version = 1\nname = \"game\"\nversion = \"1.0.0\"\nentry = \"lifecycle.py\"\n\n\
             [[settings.fields]]\nname = \"max_players\"\ntype = \"number\"\ndefault = 20\n\n\
             [[settings.fields]]\nname = \"pvp\"\ntype = \"boolean\"\ndefault = true\n\n\
             [[presets]]\nid = \"friends\"\n[presets.settings]\nmax_players = 10\npvp = false\n",
        )
        .unwrap();
        std::fs::write(dir.join("lifecycle.py"), "").unwrap();
        let mut sup = crate::supervisor::Supervisor::new_with_instances_dir(
            modules.to_str().unwrap(),
            tmp.path().join("instances").to_str().unwrap(),
        );
        for module in sup.module_loader.discover_modules().unwrap() { crate::sandbox::consent_test_component(std::path::Path::new(&module.entry_path())); }
        sup.initialize().await.unwrap();
        let state = crate::ipc::IPCServer::new(std::sync::Arc::new(tokio::sync::RwLock::new(sup)), "127.0.0.1:0", crate::daemon_log::DaemonLogBuffer::new());
        let create = |body: serde_json::Value| {
            let state = state.clone();
            async move { crate::ipc::internal::into_internal(super::create_instance(State(state), Json(body)).await).await }
        };

        let made = create(json!({ "name": "a", "module_name": "game", "preset": "friends", "module_settings": { "pvp": true } })).await;
        assert!(made.status.is_success(), "{:?}", made.body);
        let id = made.body["id"].as_str().unwrap().to_string();
        let instance = state.supervisor.read().await.instance_store.get(&id).cloned().unwrap();
        assert_eq!(instance.module_settings["max_players"], json!(10), "preset value");
        assert_eq!(instance.module_settings["pvp"], json!(true), "explicit setting wins");

        let bad = create(json!({ "name": "c", "module_name": "game", "preset": "friends", "module_settings": { "max_players": "lots" } })).await;
        assert_eq!(bad.status.as_u16(), 400);
        assert_eq!(bad.body["fields"][0]["field"], "max_players");

        let unknown = create(json!({ "name": "b", "module_name": "game", "preset": "nope" })).await;
        assert_eq!(unknown.status.as_u16(), 400);
        assert_eq!(unknown.body["error_code"], "preset_not_found");
        let twin = create(json!({ "name": "a", "module_name": "game" })).await;
        assert_eq!((twin.status.as_u16(), twin.body["error_code"].as_str()), (409, Some("name_taken")));
        assert_eq!(state.supervisor.read().await.instance_store.list().len(), 1, "the second server is not made");
    }

    /// 브랜치 서버 (`branch_of`): 상위 서버의 모듈·설정을 만들 때 한 번 옮기고 포트는 따로 정한다. 그 뒤로는 서로 따로다.
    /// 브랜치는 한 단계만 두고, 상위 서버를 지우면 브랜치는 독립 서버로 남는다
    #[tokio::test]
    async fn a_branch_copies_its_parents_settings_once_and_stays_one_level() {
        use axum::extract::{Path, Query, State};
        use axum::Json;
        let tmp = tempfile::tempdir().unwrap();
        let modules = tmp.path().join("modules");
        let dir = modules.join("game");
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(
            dir.join("module.toml"),
            "[module]\nschema_version = 1\nname = \"game\"\nversion = \"1.0.0\"\nentry = \"lifecycle.py\"\n\n[config]\ndefault_port = 25565\n\n\
             [[settings.fields]]\nname = \"port\"\ntype = \"number\"\nport = true\n\n\
             [[settings.fields]]\nname = \"max_players\"\ntype = \"number\"\ndefault = 20\n\n\
             [[settings.fields]]\nname = \"data_dir\"\ntype = \"text\"\ndefault = \"{instance_dir}/data\"\n",
        )
        .unwrap();
        std::fs::write(dir.join("lifecycle.py"), "").unwrap();
        let mut sup = crate::supervisor::Supervisor::new_with_instances_dir(modules.to_str().unwrap(), tmp.path().join("instances").to_str().unwrap());
        for module in sup.module_loader.discover_modules().unwrap() { crate::sandbox::consent_test_component(std::path::Path::new(&module.entry_path())); }
        sup.initialize().await.unwrap();
        let state = crate::ipc::IPCServer::new(std::sync::Arc::new(tokio::sync::RwLock::new(sup)), "127.0.0.1:0", crate::daemon_log::DaemonLogBuffer::new());
        let create = |body: serde_json::Value| {
            let state = state.clone();
            async move { crate::ipc::internal::into_internal(super::create_instance(State(state), Json(body)).await).await }
        };
        let get = |id: String| {
            let state = state.clone();
            async move { state.supervisor.read().await.instance_store.get(&id).cloned().unwrap() }
        };

        let made = create(json!({ "name": "main", "module_name": "game", "port": 25565, "module_settings": { "max_players": 5 } })).await;
        assert!(made.status.is_success(), "{:?}", made.body);
        let parent_id = made.body["id"].as_str().unwrap().to_string();
        {
            let mut sup = state.supervisor.write().await;
            let mut parent = sup.instance_store.get(&parent_id).cloned().unwrap();
            parent.memory_limit_mb = Some(2048);
            parent.extra_args = vec!["--nogui".into()];
            parent.module_settings.insert("auto_restart".into(), json!(false));
            sup.instance_store.update(&parent_id, parent).unwrap();
        }
        let parent = get(parent_id.clone()).await;

        // 확인 단계: 비어 있는 포트를 알려 준다
        let check = crate::ipc::internal::into_internal(super::check_new_instance(State(state.clone()), Json(json!({ "name": "main test", "branch_of": parent_id }))).await).await;
        assert!(check.status.is_success(), "{:?}", check.body);
        let free = check.body["ports"][0]["port"].as_u64().unwrap();
        assert_ne!(free, 25565, "{:?}", check.body);
        assert_eq!(state.supervisor.read().await.instance_store.list().len(), 1, "a check makes nothing");

        let branched = create(json!({ "name": "main test", "branch_of": parent_id, "module_name": "other", "module_settings": { "max_players": 99 } })).await;
        assert!(branched.status.is_success(), "{:?}", branched.body);
        let branch_id = branched.body["id"].as_str().unwrap().to_string();
        let branch = get(branch_id.clone()).await;
        assert_eq!(branch.branch_of.as_deref(), Some(parent_id.as_str()));
        assert_eq!(branch.module_name, "game", "the parent's module, whatever the request says");
        assert_eq!(branch.module_settings["max_players"], json!(5), "the parent's setting");
        assert_eq!(branch.port.map(u64::from), Some(free), "its own port");
        assert_eq!((branch.memory_limit_mb, branch.extra_args.clone(), branch.module_settings["auto_restart"].clone()), (Some(2048), vec!["--nogui".to_string()], json!(false)));
        // 상위 서버 폴더를 가리키는 값은 옮기지 않는다: 브랜치는 제 폴더를 쓴다
        assert_ne!(branch.module_settings["data_dir"], parent.module_settings["data_dir"]);
        assert!(branch.module_settings["data_dir"].as_str().unwrap().contains(&branch_id));

        // 같은 이름으로 한 번 더 만들려 하면 그 이름에 번호를 붙여 권한다 (모듈 이름으로 바꾸지 않는다)
        let again = crate::ipc::internal::into_internal(super::check_new_instance(State(state.clone()), Json(json!({ "name": "main test", "branch_of": parent_id }))).await).await;
        assert_eq!((again.body["name"]["taken"].as_bool(), again.body["name"]["suggested"].as_str()), (Some(true), Some("main test 2")), "{:?}", again.body);

        // 그 뒤로는 따로: 상위 서버를 고쳐도 브랜치는 그대로다
        {
            let mut sup = state.supervisor.write().await;
            let mut parent = sup.instance_store.get(&parent_id).cloned().unwrap();
            parent.module_settings.insert("max_players".into(), json!(50));
            sup.instance_store.update(&parent_id, parent).unwrap();
        }
        assert_eq!(get(branch_id.clone()).await.module_settings["max_players"], json!(5));

        let deeper = create(json!({ "name": "deeper", "branch_of": branch_id })).await;
        assert_eq!((deeper.status.as_u16(), deeper.body["error_code"].as_str()), (409, Some("branch_of_branch")), "{:?}", deeper.body);
        let orphan = create(json!({ "name": "orphan", "branch_of": "nope" })).await;
        assert_eq!((orphan.status.as_u16(), orphan.body["error_code"].as_str()), (404, Some("branch_parent_not_found")), "{:?}", orphan.body);

        let removed = crate::ipc::internal::into_internal(super::delete_instance(Path(parent_id.clone()), State(state.clone()), Query(super::DeleteQuery { keep_settings: false })).await).await;
        assert!(removed.status.is_success(), "{:?}", removed.body);
        assert_eq!(get(branch_id).await.branch_of, None, "the branch stays as a server of its own");
    }

    /// 가져온 서버의 브랜치도 제 월드·데이터 폴더를 쓴다: 가져온 서버는 `{instance_dir}` 기본값 칸이 비어 있고
    /// 경로 칸이 서버 폴더 밖(쓰던 자리)을 가리키는데, 그 값을 옮기면 두 서버가 같은 월드를 연다
    #[tokio::test]
    async fn a_branch_of_an_imported_server_gets_its_own_data_folders() {
        use axum::extract::State;
        use axum::Json;
        let tmp = tempfile::tempdir().unwrap();
        let modules = tmp.path().join("modules");
        let dir = modules.join("game");
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(
            dir.join("module.toml"),
            "[module]\nschema_version = 1\nname = \"game\"\nversion = \"1.0.0\"\nentry = \"lifecycle.py\"\n\n[config]\ndefault_port = 25565\n\n\
             [[settings.fields]]\nname = \"port\"\ntype = \"number\"\nport = true\n\n\
             [[settings.fields]]\nname = \"world_name\"\ntype = \"text\"\ndefault = \"world\"\n\n\
             [[settings.fields]]\nname = \"cache_dir\"\ntype = \"text\"\ndefault = \"{instance_dir}/data\"\n\n\
             [[settings.fields]]\nname = \"world_dir\"\ntype = \"text\"\nformat = \"path\"\ndefault = \"{instance_dir}/Worlds\"\n\n\
             [[settings.fields]]\nname = \"data_home\"\ntype = \"text\"\nformat = \"path\"\ndefault = \"\"\n",
        )
        .unwrap();
        std::fs::write(dir.join("lifecycle.py"), "").unwrap();
        let mut sup = crate::supervisor::Supervisor::new_with_instances_dir(modules.to_str().unwrap(), tmp.path().join("instances").to_str().unwrap());
        for module in sup.module_loader.discover_modules().unwrap() { crate::sandbox::consent_test_component(std::path::Path::new(&module.entry_path())); }
        sup.initialize().await.unwrap();
        let state = crate::ipc::IPCServer::new(std::sync::Arc::new(tokio::sync::RwLock::new(sup)), "127.0.0.1:0", crate::daemon_log::DaemonLogBuffer::new());
        let create = |body: serde_json::Value| {
            let state = state.clone();
            async move { crate::ipc::internal::into_internal(super::create_instance(State(state), Json(body)).await).await }
        };

        let made = create(json!({ "name": "old", "module_name": "game", "port": 25565 })).await;
        assert!(made.status.is_success(), "{:?}", made.body);
        let parent_id = made.body["id"].as_str().unwrap().to_string();
        // 가져오기가 남기는 값: `{instance_dir}` 기본값 칸은 비고, 경로 칸은 쓰던 자리를 가리킨다
        let old_home = tmp.path().join("oldhome");
        {
            let mut sup = state.supervisor.write().await;
            let mut parent = sup.instance_store.get(&parent_id).cloned().unwrap();
            parent.module_settings.insert("world_name".into(), json!("myworld"));
            parent.module_settings.insert("cache_dir".into(), json!(""));
            parent.module_settings.insert("world_dir".into(), json!(old_home.join("Worlds").to_string_lossy()));
            parent.module_settings.insert("data_home".into(), json!(old_home.to_string_lossy()));
            sup.instance_store.update(&parent_id, parent).unwrap();
        }

        let branched = create(json!({ "name": "old b", "branch_of": parent_id })).await;
        assert!(branched.status.is_success(), "{:?}", branched.body);
        let branch_id = branched.body["id"].as_str().unwrap().to_string();
        let branch = state.supervisor.read().await.instance_store.get(&branch_id).cloned().unwrap();
        assert_eq!(branch.module_settings["world_name"], json!("myworld"), "the parent's setting");
        for key in ["cache_dir", "world_dir"] {
            assert!(branch.module_settings[key].as_str().unwrap().contains(&branch_id), "{key}: {:?}", branch.module_settings[key]);
        }
        assert_eq!(branch.module_settings["data_home"], json!(""), "the parent's old data folder is not the branch's");
    }

    /// 서버 이름의 쉼표는 받지 않는다: 문장 명령이 부르는 말을 쉼표로 나눈다 (사용자 지정 2026-11-06)
    #[tokio::test]
    async fn a_server_name_with_a_comma_is_refused() {
        use axum::extract::{Path, State};
        use axum::Json;
        let tmp = tempfile::tempdir().unwrap();
        let modules = tmp.path().join("modules");
        let dir = modules.join("game");
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(dir.join("module.toml"), "[module]\nschema_version = 1\nname = \"game\"\nversion = \"1.0.0\"\nentry = \"lifecycle.py\"\n").unwrap();
        std::fs::write(dir.join("lifecycle.py"), "").unwrap();
        let mut sup = crate::supervisor::Supervisor::new_with_instances_dir(
            modules.to_str().unwrap(),
            tmp.path().join("instances").to_str().unwrap(),
        );
        for module in sup.module_loader.discover_modules().unwrap() { crate::sandbox::consent_test_component(std::path::Path::new(&module.entry_path())); }
        sup.initialize().await.unwrap();
        let state = crate::ipc::IPCServer::new(std::sync::Arc::new(tokio::sync::RwLock::new(sup)), "127.0.0.1:0", crate::daemon_log::DaemonLogBuffer::new());
        let create = |body: serde_json::Value| {
            let state = state.clone();
            async move { crate::ipc::internal::into_internal(super::create_instance(State(state), Json(body)).await).await }
        };

        let check = crate::ipc::internal::into_internal(super::check_new_instance(State(state.clone()), Json(json!({ "name": "우리, 마크", "module_name": "game" }))).await).await;
        assert_eq!(check.body["name"]["invalid"], "comma", "{}", check.body);
        for name in ["우리, 마크", "우리，마크", "우리、마크"] {
            let refused = create(json!({ "name": name, "module_name": "game" })).await;
            assert_eq!((refused.status.as_u16(), refused.body["error_code"].as_str()), (400, Some("invalid_name")), "{name}");
        }
        assert!(state.supervisor.read().await.instance_store.list().is_empty());

        let made = create(json!({ "name": "우리 마크", "module_name": "game" })).await;
        assert!(made.status.is_success(), "{:?}", made.body);
        let id = made.body["id"].as_str().unwrap().to_string();
        let renamed = crate::ipc::internal::into_internal(super::rename_instance(State(state.clone()), Path(id.clone()), Json(json!({ "name": "마크,서버" }))).await).await;
        assert_eq!((renamed.status.as_u16(), renamed.body["error_code"].as_str()), (400, Some("invalid_name")));
        assert_eq!(state.supervisor.read().await.instance_store.get(&id).unwrap().name, "우리 마크");
    }

    /// 대소문자·공백 칸 수만 다른 이름은 같은 이름이다: 채팅이 그렇게 서버를 찾으므로 둘이면 이름으로 부를 수 없다
    #[tokio::test]
    async fn a_name_that_differs_only_in_case_or_spacing_is_taken() {
        use axum::extract::{Path, State};
        use axum::Json;
        let tmp = tempfile::tempdir().unwrap();
        let modules = tmp.path().join("modules");
        let dir = modules.join("game");
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(dir.join("module.toml"), "[module]\nschema_version = 1\nname = \"game\"\nversion = \"1.0.0\"\nentry = \"lifecycle.py\"\n").unwrap();
        std::fs::write(dir.join("lifecycle.py"), "").unwrap();
        let mut sup = crate::supervisor::Supervisor::new_with_instances_dir(
            modules.to_str().unwrap(),
            tmp.path().join("instances").to_str().unwrap(),
        );
        for module in sup.module_loader.discover_modules().unwrap() { crate::sandbox::consent_test_component(std::path::Path::new(&module.entry_path())); }
        sup.initialize().await.unwrap();
        let state = crate::ipc::IPCServer::new(std::sync::Arc::new(tokio::sync::RwLock::new(sup)), "127.0.0.1:0", crate::daemon_log::DaemonLogBuffer::new());
        let create = |body: serde_json::Value| {
            let state = state.clone();
            async move { crate::ipc::internal::into_internal(super::create_instance(State(state), Json(body)).await).await }
        };
        let rename = |id: String, name: &'static str| {
            let state = state.clone();
            async move { crate::ipc::internal::into_internal(super::rename_instance(State(state), Path(id), Json(json!({ "name": name }))).await).await }
        };

        let first = create(json!({ "name": "My Server", "module_name": "game" })).await;
        assert!(first.status.is_success(), "{:?}", first.body);
        let first = first.body["id"].as_str().unwrap().to_string();
        for name in ["my server", "MY  SERVER"] {
            let twin = create(json!({ "name": name, "module_name": "game" })).await;
            assert_eq!((twin.status.as_u16(), twin.body["error_code"].as_str()), (409, Some("name_taken")), "{name}");
        }
        let check = crate::ipc::internal::into_internal(super::check_new_instance(State(state.clone()), Json(json!({ "name": "my server", "module_name": "game" }))).await).await;
        assert_eq!(check.body["name"]["taken"], true, "{}", check.body);

        let other = create(json!({ "name": "other", "module_name": "game" })).await;
        let other = other.body["id"].as_str().unwrap().to_string();
        let twin = rename(other, "MY SERVER").await;
        assert_eq!((twin.status.as_u16(), twin.body["error_code"].as_str()), (409, Some("name_taken")), "{}", twin.body);
        // 자기 이름의 대소문자는 바꿀 수 있다
        let own = rename(first.clone(), "my server").await;
        assert!(own.status.is_success(), "{}", own.body);
        assert_eq!(state.supervisor.read().await.instance_store.get(&first).unwrap().name, "my server");
    }

    /// 앱 설정의 새 서버 기본값(자동 백업 보관 개수, 크래시 자동 재시작)을 만들 때 넣는다. 기본값이면 지금처럼 비워 둔다
    #[tokio::test]
    async fn create_applies_the_new_server_defaults_from_app_settings() {
        use axum::extract::State;
        use axum::Json;
        let tmp = tempfile::tempdir().unwrap();
        let modules = tmp.path().join("modules");
        let dir = modules.join("game");
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(dir.join("module.toml"), "[module]\nschema_version = 1\nname = \"game\"\nversion = \"1.0.0\"\nentry = \"lifecycle.py\"\n").unwrap();
        std::fs::write(dir.join("lifecycle.py"), "").unwrap();
        let mut sup = crate::supervisor::Supervisor::new_with_instances_dir(
            modules.to_str().unwrap(),
            tmp.path().join("instances").to_str().unwrap(),
        );
        for module in sup.module_loader.discover_modules().unwrap() { crate::sandbox::consent_test_component(std::path::Path::new(&module.entry_path())); }
        sup.initialize().await.unwrap();
        let state = crate::ipc::IPCServer::new(std::sync::Arc::new(tokio::sync::RwLock::new(sup)), "127.0.0.1:0", crate::daemon_log::DaemonLogBuffer::new());
        let create = |name: &str| {
            let (state, body) = (state.clone(), json!({ "name": name, "module_name": "game" }));
            async move {
                let made = crate::ipc::internal::into_internal(super::create_instance(State(state.clone()), Json(body)).await).await;
                assert!(made.status.is_success(), "{:?}", made.body);
                state.supervisor.read().await.instance_store.get(made.body["id"].as_str().unwrap()).cloned().unwrap()
            }
        };

        let plain = create("a").await;
        assert_eq!((plain.backup_keep, plain.module_settings.get("auto_restart")), (None, None));

        state.config_store
            .write_gui(json!({ "newServerBackupKeep": 3, "newServerAutoRestart": false }), crate::config_store::Apply::MergePatch, None)
            .await
            .unwrap();
        let tuned = create("b").await;
        assert_eq!((tuned.backup_keep, tuned.module_settings.get("auto_restart")), (Some(3), Some(&json!(false))));
    }

    #[tokio::test]
    async fn create_with_a_port_also_sets_the_module_port_setting() {
        use axum::extract::State;
        use axum::Json;
        let tmp = tempfile::tempdir().unwrap();
        let modules = tmp.path().join("modules");
        let dir = modules.join("game");
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(
            dir.join("module.toml"),
            "[module]\nschema_version = 1\nname = \"game\"\nversion = \"1.0.0\"\nentry = \"lifecycle.py\"\n\n\
             [[settings.fields]]\nname = \"port\"\ntype = \"number\"\n",
        )
        .unwrap();
        std::fs::write(dir.join("lifecycle.py"), "").unwrap();
        let mut sup = crate::supervisor::Supervisor::new_with_instances_dir(
            modules.to_str().unwrap(),
            tmp.path().join("instances").to_str().unwrap(),
        );
        for module in sup.module_loader.discover_modules().unwrap() { crate::sandbox::consent_test_component(std::path::Path::new(&module.entry_path())); }
        sup.initialize().await.unwrap();
        let state = crate::ipc::IPCServer::new(std::sync::Arc::new(tokio::sync::RwLock::new(sup)), "127.0.0.1:0", crate::daemon_log::DaemonLogBuffer::new());
        let body = json!({ "name": "a", "module_name": "game", "port": 25570 });
        let made = crate::ipc::internal::into_internal(super::create_instance(State(state.clone()), Json(body)).await).await;
        assert!(made.status.is_success(), "{:?}", made.body);
        let id = made.body["id"].as_str().unwrap().to_string();
        let instance = state.supervisor.read().await.instance_store.get(&id).cloned().unwrap();
        // 설치는 module_settings로 게임 설정 파일을 쓴다: 두 값이 같아야 한다
        assert_eq!((instance.port, &instance.module_settings["port"]), (Some(25570), &json!(25570)));

        // 포트로 쓸 수 없는 값은 만들지 않고 알린다
        for port in [json!(70000), json!(0), json!("abc")] {
            let body = json!({ "name": "b", "module_name": "game", "port": port });
            let bad = crate::ipc::internal::into_internal(super::create_instance(State(state.clone()), Json(body)).await).await;
            assert_eq!((bad.status.as_u16(), &bad.body["error_code"]), (400, &json!("invalid_format")), "{port}");
        }
    }

    /// 가져온 서버의 시작 스크립트에만 있던 실행 인자(모듈 `import_settings`의 `extra_args`)는 인스턴스의 추가 실행 인자가 된다.
    /// 모양이 틀린 값(문자열이 아닌 항목, 줄바꿈)은 받지 않는다
    #[tokio::test]
    async fn a_migration_keeps_the_launch_args_of_the_old_start_script() {
        use axum::{extract::State, Json};
        use serde_json::json;
        for (returned, expected) in [
            (json!(["-javaagent:agent.jar", "-Dmod.flag=1"]), vec!["-javaagent:agent.jar".to_string(), "-Dmod.flag=1".to_string()]),
            (json!(["-Dok=1", 3]), Vec::new()),
            (json!(["-Dbad=1\nrm"]), Vec::new()),
        ] {
            let tmp = tempfile::tempdir().unwrap();
            let modules = tmp.path().join("modules");
            let dir = modules.join("game");
            let server = tmp.path().join("server");
            for folder in [&dir, &server] { std::fs::create_dir_all(folder).unwrap(); }
            std::fs::write(dir.join("module.toml"), "[module]\nschema_version = 1\nname = 'game'\nversion = '1.0.0'\nentry = 'lifecycle.py'\n").unwrap();
            let result = json!({"success":true,"settings":{},"extra_args":returned});
            std::fs::write(dir.join("lifecycle.py"), format!("import sys\nprint('{{\"success\":true,\"functions\":[\"import_settings\"]}}' if sys.argv[1] == 'functions' else {:?})\n", result.to_string())).unwrap();
            crate::sandbox::consent_test_component(&dir.join("lifecycle.py"));
            let mut sup = crate::supervisor::Supervisor::new_with_instances_dir(modules.to_str().unwrap(), tmp.path().join("instances").to_str().unwrap());
            for module in sup.module_loader.discover_modules().unwrap() { crate::sandbox::consent_test_component(std::path::Path::new(&module.entry_path())); }
            sup.initialize().await.unwrap();
            let state = crate::ipc::IPCServer::new(std::sync::Arc::new(tokio::sync::RwLock::new(sup)), "127.0.0.1:0", crate::daemon_log::DaemonLogBuffer::new());
            let response = crate::ipc::internal::into_internal(super::create_instance(State(state.clone()), Json(json!({"name":"g", "module_name":"game", "migration_source":server}))).await).await;
            assert_eq!(response.status.as_u16(), 201, "{}", response.body);
            let sup = state.supervisor.read().await;
            let instance = sup.instance_store.get(response.body["id"].as_str().unwrap()).unwrap();
            assert_eq!(instance.extra_args, expected, "{returned}");
        }
    }
}
