//! 인스턴스 설정 저장(PATCH) 요청·응답. 본체는 `service::instance::settings`

use super::*;

/// PATCH /api/instance/:id - 인스턴스 설정 업데이트 (보낸 키만). `revision`(GET에서 받은 값)을 주면
/// 그사이 다른 곳에서 바뀐 인스턴스에는 쓰지 않고 409 `settings_changed`로 거절한다
pub async fn update_instance_settings(
    State(state): State<IPCServer>,
    Path(id): Path<String>,
    Json(settings): Json<serde_json::Value>,
) -> axum::response::Response {
    super::super::answer(crate::service::instance::settings::update(&state, &id, settings).await)
}

#[cfg(test)]
mod tests {
    use crate::service::instance::settings::{memory_limit_from, stop_timeout_from};

    /// 설정 저장의 거절과 답 본문을 고정한다 (본체를 서비스로 옮기며 한 글자도 바뀌지 않게)
    #[tokio::test]
    async fn a_settings_save_answers_with_the_same_bodies() {
        use axum::{extract::{Path, State}, Json};
        use serde_json::json;
        let tmp = tempfile::tempdir().unwrap();
        let modules = tmp.path().join("modules");
        let dir = modules.join("game");
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(dir.join("module.toml"), "[module]\nschema_version = 1\nname = 'game'\nversion = '1.0.0'\nentry = 'lifecycle.py'\n").unwrap();
        std::fs::write(dir.join("lifecycle.py"), "import json\nprint(json.dumps({'success': True}))\n").unwrap();
        crate::sandbox::consent_test_component(&dir.join("lifecycle.py"));
        let mut sup = crate::supervisor::Supervisor::new_with_instances_dir(modules.to_str().unwrap(), tmp.path().join("instances").to_str().unwrap());
        sup.initialize().await.unwrap();
        let mut inst = crate::instance::ServerInstance::new("srv", "game");
        inst.id = "pin".into();
        sup.instance_store.add(inst).unwrap();
        let mut other = crate::instance::ServerInstance::new("other", "game");
        other.id = "pin-other".into();
        other.working_dir = Some(tmp.path().join("other").to_string_lossy().into_owned());
        sup.instance_store.add(other.clone()).unwrap();
        let state = crate::ipc::IPCServer::new(std::sync::Arc::new(tokio::sync::RwLock::new(sup)), "127.0.0.1:0", crate::daemon_log::DaemonLogBuffer::new());
        let save = |id: &str, body: serde_json::Value| {
            let (state, id) = (state.clone(), id.to_string());
            async move {
                let r = crate::ipc::internal::into_internal(super::update_instance_settings(State(state), Path(id), Json(body)).await).await;
                (r.status.as_u16(), r.body)
            }
        };
        let invalid = |field: &str, message: &str| json!({ "success": false, "error": message, "error_code": "invalid_format", "message": message, "fields": [{ "field": field }] });
        assert_eq!(save("nope", json!({})).await, (404, json!({ "success": false, "error": "Instance not found", "error_code": "not_found" })));
        assert_eq!(save("pin", json!({ "port": 70000 })).await, (400, json!({ "success": false, "error": "port must be a number from 1 to 65535", "error_code": "invalid_format", "fields": [{ "field": "port" }] })));
        assert_eq!(save("pin", json!({ "extra_args": 5 })).await, (400, invalid("extra_args", "extra_args must be a string or an array of strings")));
        assert_eq!(save("pin", json!({ "stop_timeout": 99999 })).await, (400, invalid("stop_timeout", "stop_timeout must be empty or a whole number of seconds from 1 to 3600")));
        assert_eq!(save("pin", json!({ "auto_update": "yes" })).await, (400, invalid("auto_update", "auto_update must be true or false")));
        assert_eq!(save("pin", json!({ "working_dir": other.working_dir })).await, (409, json!({ "success": false, "error": "Server 'other' already uses this folder", "error_code": "working_dir_in_use", "instance": "other" })));
        let (status, saved) = save("pin", json!({ "rcon_port": 1 })).await;
        assert_eq!(status, 200, "{saved}");
        let revision = saved["revision"].clone();
        assert_eq!(saved, json!({ "success": true, "synced": true, "restart_required": [], "revision": revision, "ignored": ["rcon_port"] }));
        assert_eq!(save("pin", json!({ "revision": "stale" })).await, (409, json!({ "success": false, "error": "The server settings were changed elsewhere; read them again", "error_code": "settings_changed", "revision": revision })));
    }
    use serde_json::json;

    /// 외부 감사 10-21: 같은 서버의 저장 둘이 겹치면 늦게 끝난 게임 파일 쓰기가 나중 저장을 덮었다(데몬과 파일이 달라짐).
    /// 저장은 인스턴스마다 하나씩, 게임 파일 쓰기까지 마친 뒤 다음 저장이 시작된다. 범위 밖 포트는 다른 숫자로 저장하지 않고 거절한다
    /// 저장이 게임 파일을 쓰는 동안 설정 창이 실제 값을 읽으면(`settings/live`) 저장이 끝난 뒤 읽는다:
    /// 아직 쓰지 않은 옛 파일 값으로 캐시를 덮지 않게
    #[tokio::test]
    async fn a_live_read_during_a_save_waits_for_the_game_file_write() {
        use axum::extract::{Path, State};
        use axum::Json;
        let tmp = tempfile::tempdir().unwrap();
        // 데몬의 모듈 폴더 아래에 둔다: 그 밖의 모듈은 Windows 격리 실행에서 인스턴스 폴더 쓰기를 받지 못한다
        let modules = crate::plugin::resolve_modules_dir();
        let module = tempfile::Builder::new().prefix("live-read-").tempdir_in(&modules).unwrap();
        let dir = module.path();
        let name = dir.file_name().unwrap().to_string_lossy().into_owned();
        std::fs::write(
            dir.join("module.toml"),
            format!("[module]\nschema_version = 1\nname = \"{name}\"\nversion = \"1.0.0\"\nentry = \"lifecycle.py\"\n\n[[settings.fields]]\nname = \"motd\"\ntype = \"text\"\n"),
        )
        .unwrap();
        let file = tmp.path().join("motd.txt");
        std::fs::write(&file, "old").unwrap();
        std::fs::write(
            dir.join("lifecycle.py"),
            format!(
                "import json, sys, time\nconfig = json.load(sys.stdin)\nf = {:?}\n\
                 if sys.argv[1] == 'functions':\n    print(json.dumps({{'success': True, 'functions': ['configure', 'import_settings']}}))\n\
                 elif sys.argv[1] == 'import_settings':\n    print(json.dumps({{'success': True, 'settings': {{'motd': open(f).read()}}}}))\n\
                 else:\n    time.sleep(1.0)\n    open(f, 'w').write((config.get('properties') or config.get('settings') or {{}}).get('motd', ''))\n    print(json.dumps({{'success': True}}))\n",
                file.to_string_lossy()
            ),
        )
        .unwrap();
        let mut sup = crate::supervisor::Supervisor::new_with_instances_dir(modules.to_str().unwrap(), tmp.path().join("instances").to_str().unwrap());
        for module in sup.module_loader.discover_modules().unwrap() { crate::sandbox::consent_test_component(std::path::Path::new(&module.entry_path())); }
        sup.initialize().await.unwrap();
        let mut inst = crate::instance::ServerInstance::new("a", &name);
        inst.working_dir = Some(tmp.path().to_string_lossy().to_string());
        let id = inst.id.clone();
        sup.instance_store.add(inst).unwrap();
        let state = crate::ipc::IPCServer::new(std::sync::Arc::new(tokio::sync::RwLock::new(sup)), "127.0.0.1:0", crate::daemon_log::DaemonLogBuffer::new());
        let save = {
            let (state, id) = (state.clone(), id.clone());
            tokio::spawn(async move { crate::ipc::internal::into_internal(super::update_instance_settings(State(state), Path(id), Json(json!({ "motd": "new" }))).await).await })
        };
        tokio::time::sleep(std::time::Duration::from_millis(400)).await;
        let live = crate::ipc::internal::into_internal(super::super::super::managed::read_live_settings_handler(Path(id.clone()), State(state.clone())).await).await;
        let saved = save.await.unwrap();
        assert!(saved.status.is_success(), "{}", saved.body);
        assert_eq!(saved.body["synced"], true, "{}", saved.body);
        assert_eq!(std::fs::read_to_string(&file).unwrap(), "new");
        assert_eq!(live.body["settings"]["motd"], "new", "{}", live.body);
        let inst = state.supervisor.read().await.instance_store.get(&id).cloned().unwrap();
        assert_eq!(inst.module_settings["motd"], "new");
        // 읽기가 캐시를 맞춘 뒤의 revision을 준다: 설정 창은 이 값으로 저장한다 (연 때의 값이면 409가 된다)
        assert_eq!(live.body["revision"], json!(inst.settings_revision()), "{}", live.body);
            }

    /// 설정 저장은 게임 파일을 쓴다: 업데이트·복원처럼 서버 파일을 바꾸는 작업 중에는 거절하고(409),
    /// 파일을 읽기만 하는 백업 중에는 저장한다. 약관 동의도 같은 줄에 선다
    #[tokio::test]
    async fn settings_writes_are_refused_while_server_files_change() {
        use axum::extract::{Path, State};
        use axum::Json;
        let tmp = tempfile::tempdir().unwrap();
        let modules = tmp.path().join("modules");
        let dir = modules.join("game");
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(
            dir.join("module.toml"),
            "[module]\nschema_version = 1\nname = \"game\"\nversion = \"1.0.0\"\nentry = \"lifecycle.py\"\n\n\
             [[settings.fields]]\nname = \"motd\"\ntype = \"text\"\n",
        )
        .unwrap();
        std::fs::write(dir.join("lifecycle.py"), "import json, sys\njson.load(sys.stdin)\nprint(json.dumps({'success': True}))\n").unwrap();
        let mut sup = crate::supervisor::Supervisor::new_with_instances_dir(modules.to_str().unwrap(), tmp.path().join("instances").to_str().unwrap());
        sup.initialize().await.unwrap();
        let mut inst = crate::instance::ServerInstance::new("a", "game");
        inst.working_dir = Some(tmp.path().to_string_lossy().to_string());
        let id = inst.id.clone();
        sup.instance_store.add(inst).unwrap();
        let state = crate::ipc::IPCServer::new(std::sync::Arc::new(tokio::sync::RwLock::new(sup)), "127.0.0.1:0", crate::daemon_log::DaemonLogBuffer::new());
        let save = |motd: &'static str| {
            let (state, id) = (state.clone(), id.clone());
            async move { crate::ipc::internal::into_internal(super::update_instance_settings(State(state), Path(id), Json(json!({ "motd": motd }))).await).await }
        };

        let update = crate::instance::lock::InstanceLock::take_for_writes(&id).await.unwrap();
        let refused = save("during update").await;
        assert_eq!(refused.status, axum::http::StatusCode::CONFLICT);
        assert_eq!(refused.body["error_code"], "instance_busy");
        let agreement = crate::ipc::internal::into_internal(
            super::super::super::managed::accept_agreements_handler(Path(id.clone()), State(state.clone()), Json(json!({ "accept": ["eula"] }))).await,
        )
        .await;
        assert_eq!(agreement.status, axum::http::StatusCode::CONFLICT);
        drop(update);

        let backup = crate::instance::lock::InstanceLock::take(&id).unwrap();
        assert!(save("during backup").await.status.is_success());
        drop(backup);
        assert_eq!(state.supervisor.read().await.instance_store.get(&id).unwrap().module_settings["motd"], "during backup");
    }

    #[tokio::test]
    async fn saves_of_one_server_run_one_after_another_and_ports_stay_in_range() {
        use axum::extract::{Path, State};
        use axum::Json;
        let tmp = tempfile::tempdir().unwrap();
        let modules = tmp.path().join("modules");
        let dir = modules.join("game");
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(
            dir.join("module.toml"),
            "[module]\nschema_version = 1\nname = \"game\"\nversion = \"1.0.0\"\nentry = \"lifecycle.py\"\n\n\
             [[settings.fields]]\nname = \"motd\"\ntype = \"text\"\n",
        )
        .unwrap();
        // 첫 저장의 게임 파일 쓰기는 늦게 끝난다
        std::fs::write(
            dir.join("lifecycle.py"),
            "import json, sys, time\nconfig = json.load(sys.stdin)\n\
             if (config.get('properties') or config.get('settings') or {}).get('motd') == 'first':\n    time.sleep(1.0)\n\
             print(json.dumps({'success': True}))\n",
        )
        .unwrap();
        let mut sup = crate::supervisor::Supervisor::new_with_instances_dir(modules.to_str().unwrap(), tmp.path().join("instances").to_str().unwrap());
        for module in sup.module_loader.discover_modules().unwrap() { crate::sandbox::consent_test_component(std::path::Path::new(&module.entry_path())); }
        sup.initialize().await.unwrap();
        let mut inst = crate::instance::ServerInstance::new("a", "game");
        inst.working_dir = Some(tmp.path().to_string_lossy().to_string());
        let id = inst.id.clone();
        sup.instance_store.add(inst).unwrap();
        let state = crate::ipc::IPCServer::new(std::sync::Arc::new(tokio::sync::RwLock::new(sup)), "127.0.0.1:0", crate::daemon_log::DaemonLogBuffer::new());
        let patch = |body: serde_json::Value| {
            let (state, id) = (state.clone(), id.clone());
            async move { crate::ipc::internal::into_internal(super::update_instance_settings(State(state), Path(id), Json(body)).await).await }
        };

        let done = std::sync::Arc::new(std::sync::Mutex::new(Vec::new()));
        let save = |motd: &'static str| {
            let (done, call) = (done.clone(), patch(json!({ "motd": motd })));
            tokio::spawn(async move {
                let r = call.await;
                assert!(r.status.is_success(), "{:?}", r.body);
                done.lock().unwrap().push(motd);
            })
        };
        let first = save("first");
        tokio::time::sleep(std::time::Duration::from_millis(300)).await;
        let second = save("second");
        first.await.unwrap();
        second.await.unwrap();
        assert_eq!(*done.lock().unwrap(), ["first", "second"], "the second save waited for the first one's game file write");
        assert_eq!(state.supervisor.read().await.instance_store.get(&id).unwrap().module_settings["motd"], "second");

        for (key, value) in [("port", json!(65537)), ("port", json!(0)), ("port", json!("70000"))] {
            let bad = patch(json!({ key: value.clone() })).await;
            assert_eq!((bad.status.as_u16(), bad.body["error_code"].as_str(), bad.body["fields"][0]["field"].as_str()), (400, Some("invalid_format"), Some(key)), "{key}={value}");
        }
        let inst = state.supervisor.read().await.instance_store.get(&id).cloned().unwrap();
        assert_ne!(inst.port, Some(1));
        assert!(patch(json!({ "port": "25570" })).await.status.is_success());
        let inst = state.supervisor.read().await.instance_store.get(&id).cloned().unwrap();
        assert_eq!(inst.port, Some(25570));
    }

    #[tokio::test]
    async fn settings_save_rejects_unknown_keys_and_reports_a_failed_game_file_write() {
        use axum::extract::{Path, State};
        use axum::Json;
        let tmp = tempfile::tempdir().unwrap();
        let modules = tmp.path().join("modules");
        let dir = modules.join("game");
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(
            dir.join("module.toml"),
            "[module]\nschema_version = 1\nname = \"game\"\nversion = \"1.0.0\"\nentry = \"lifecycle.py\"\n\n\
             [[settings.fields]]\nname = \"motd\"\ntype = \"text\"\n",
        )
        .unwrap();
        std::fs::write(
            dir.join("lifecycle.py"),
            "import json, sys\nprint(json.dumps({'success': False, 'message': 'server.properties is read-only'}))\n",
        )
        .unwrap();
        let mut sup = crate::supervisor::Supervisor::new_with_instances_dir(
            modules.to_str().unwrap(),
            tmp.path().join("instances").to_str().unwrap(),
        );
        for module in sup.module_loader.discover_modules().unwrap() { crate::sandbox::consent_test_component(std::path::Path::new(&module.entry_path())); }
        sup.initialize().await.unwrap();
        let mut inst = crate::instance::ServerInstance::new("a", "game");
        inst.working_dir = Some(tmp.path().to_string_lossy().to_string());
        let id = inst.id.clone();
        sup.instance_store.add(inst).unwrap();
        let state = crate::ipc::IPCServer::new(std::sync::Arc::new(tokio::sync::RwLock::new(sup)), "127.0.0.1:0", crate::daemon_log::DaemonLogBuffer::new());
        let patch = |body: serde_json::Value| {
            let (state, id) = (state.clone(), id.clone());
            async move { crate::ipc::internal::into_internal(super::update_instance_settings(State(state), Path(id), Json(body)).await).await }
        };

        let typo = patch(json!({ "motdd": "hi" })).await;
        assert_eq!((typo.status.as_u16(), typo.body["error_code"].as_str()), (400, Some("unknown_setting")));
        assert_eq!(typo.body["fields"][0]["field"], "motdd");

        let saved = patch(json!({ "motd": "hi", "auto_restart": false, "enable_rcon": true })).await;
        assert!(saved.status.is_success(), "{:?}", saved.body);
        assert_eq!(saved.body["synced"], false);
        // Codex 리뷰 #550: 모듈 실패 칸을 합치면서 저장 성공 표시를 false로 덮어 GUI가 저장 실패로 보였다
        assert_eq!(saved.body["success"], true, "the settings were saved");
        assert_eq!(saved.body["sync_error"], "server.properties is read-only");
        assert_eq!(saved.body["ignored"], json!(["enable_rcon"]));
        let instance = state.supervisor.read().await.instance_store.get(&id).cloned().unwrap();
        assert_eq!(instance.module_settings["motd"], json!("hi"), "the setting is kept even when the game file write fails");
    }

    /// 설정 저장에 GET의 revision을 실으면, 그사이 다른 곳에서 바뀐 인스턴스에는 쓰지 않는다 (0.1.10 결정 1)
    #[tokio::test]
    async fn a_settings_save_with_a_stale_revision_is_refused() {
        use axum::extract::{Path, State};
        use axum::Json;
        let tmp = tempfile::tempdir().unwrap();
        let modules = tmp.path().join("modules");
        let dir = modules.join("game");
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(
            dir.join("module.toml"),
            "[module]\nschema_version = 1\nname = \"game\"\nversion = \"1.0.0\"\nentry = \"lifecycle.py\"\n\n\
             [[settings.fields]]\nname = \"motd\"\ntype = \"text\"\n",
        )
        .unwrap();
        std::fs::write(dir.join("lifecycle.py"), "import json\nprint(json.dumps({'success': True}))\n").unwrap();
        let mut sup = crate::supervisor::Supervisor::new_with_instances_dir(
            modules.to_str().unwrap(),
            tmp.path().join("instances").to_str().unwrap(),
        );
        sup.initialize().await.unwrap();
        let inst = crate::instance::ServerInstance::new("a", "game");
        let id = inst.id.clone();
        sup.instance_store.add(inst).unwrap();
        let state = crate::ipc::IPCServer::new(std::sync::Arc::new(tokio::sync::RwLock::new(sup)), "127.0.0.1:0", crate::daemon_log::DaemonLogBuffer::new());
        let patch = |body: serde_json::Value| {
            let (state, id) = (state.clone(), id.clone());
            async move { crate::ipc::internal::into_internal(super::update_instance_settings(State(state), Path(id), Json(body)).await).await }
        };
        let read = || {
            let (state, id) = (state.clone(), id.clone());
            async move { crate::ipc::internal::into_internal(super::get_instance(Path(id), State(state)).await).await.body }
        };

        let seen = read().await["revision"].as_str().unwrap().to_string();
        let first = patch(json!({ "motd": "one", "revision": seen })).await;
        assert!(first.status.is_success(), "{:?}", first.body);
        assert_eq!(first.body["revision"], read().await["revision"], "the reply carries the new revision");
        let stale = patch(json!({ "motd": "two", "revision": seen })).await;
        assert_eq!((stale.status.as_u16(), stale.body["error_code"].as_str()), (409, Some("settings_changed")));
        assert_eq!(stale.body["revision"], first.body["revision"]);
        let instance = state.supervisor.read().await.instance_store.get(&id).cloned().unwrap();
        assert_eq!(instance.module_settings["motd"], json!("one"));
        // revision 없이 보내면 예전처럼 받는다
        assert!(patch(json!({ "motd": "three" })).await.status.is_success());
    }

    /// 메모리 상한은 러너가 Windows Job으로만 건다: 다른 OS의 데몬은 상한을 저장하지 않고 적용할 수 없다고 답한다
    /// (저장되면 사용자는 걸린 줄 안다). 비우기는 어디서나 받는다
    #[tokio::test]
    async fn a_memory_limit_is_only_saved_where_it_is_enforced() {
        use axum::extract::{Path, State};
        use axum::Json;
        let tmp = tempfile::tempdir().unwrap();
        let modules = tmp.path().join("modules");
        let dir = modules.join("game");
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(dir.join("module.toml"), "[module]\nschema_version = 1\nname = \"game\"\nversion = \"1.0.0\"\nentry = \"lifecycle.py\"\n").unwrap();
        std::fs::write(dir.join("lifecycle.py"), "import json\nprint(json.dumps({'success': True}))\n").unwrap();
        let mut sup = crate::supervisor::Supervisor::new_with_instances_dir(modules.to_str().unwrap(), tmp.path().join("instances").to_str().unwrap());
        sup.initialize().await.unwrap();
        let inst = crate::instance::ServerInstance::new("a", "game");
        let id = inst.id.clone();
        sup.instance_store.add(inst).unwrap();
        let state = crate::ipc::IPCServer::new(std::sync::Arc::new(tokio::sync::RwLock::new(sup)), "127.0.0.1:0", crate::daemon_log::DaemonLogBuffer::new());
        let patch = |body: serde_json::Value| {
            let (state, id) = (state.clone(), id.clone());
            async move { crate::ipc::internal::into_internal(super::update_instance_settings(State(state), Path(id), Json(body)).await).await }
        };
        let stored = || {
            let (state, id) = (state.clone(), id.clone());
            async move { state.supervisor.read().await.instance_store.get(&id).map(|i| i.memory_limit_mb) }
        };

        let saved = patch(json!({ "memory_limit_mb": "4G" })).await;
        if cfg!(windows) {
            assert!(saved.status.is_success(), "{:?}", saved.body);
            assert_eq!(stored().await, Some(Some(4096)));
        } else {
            assert_eq!((saved.status.as_u16(), saved.body["error_code"].as_str()), (422, Some("memory_limit_unsupported")), "{:?}", saved.body);
            assert_eq!(saved.body["fields"], json!([{ "field": "memory_limit_mb" }]));
            assert_eq!(stored().await, Some(None));
        }
        assert!(patch(json!({ "memory_limit_mb": null })).await.status.is_success());
        assert_eq!(stored().await, Some(None));

        // 0.1.14까지 저장된 상한: 인스턴스 응답이 이 OS에서 걸리지 않는다고 알린다
        let read = || {
            let (state, id) = (state.clone(), id.clone());
            async move { crate::ipc::internal::into_internal(super::get_instance(Path(id), State(state)).await).await.body }
        };
        assert_eq!(read().await.get("memory_limit_unsupported"), None);
        {
            let mut sup = state.supervisor.write().await;
            let mut inst = sup.instance_store.get(&id).unwrap().clone();
            inst.memory_limit_mb = Some(2048);
            sup.instance_store.update(&id, inst).unwrap();
        }
        let body = read().await;
        assert_eq!(body["memory_limit_mb"], json!(2048));
        assert_eq!(body.get("memory_limit_unsupported"), (!cfg!(windows)).then_some(&json!(true)));

        // 다른 PC의 데몬을 보는 클라이언트는 자기 OS로 짐작하지 않는다: 단건 조회와 서버 목록이 이 데몬의 판정을 싣는다
        assert_eq!(body["memory_limit_supported"], json!(cfg!(windows)));
        let list = crate::ipc::internal::InternalResponse::from(crate::service::servers::list(&state, 0).await).body;
        assert_eq!(list["servers"][0]["memory_limit_supported"], json!(cfg!(windows)));
    }

    /// 06 §8.1·8.2 (스키마 2): 옛 문자열 값은 목록으로 읽고, 숨긴 설정은 required를 보지 않으며 `hidden`에 실린다.
    /// 켜져 있을 때 바꾼 restart_required 설정은 응답 `restart_required`에 실린다
    #[tokio::test]
    async fn schema_v2_settings_migrate_hide_and_ask_for_a_restart() {
        use axum::extract::{Path, State};
        use axum::Json;
        let tmp = tempfile::tempdir().unwrap();
        let modules = tmp.path().join("modules");
        let dir = modules.join("game");
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(
            dir.join("module.toml"),
            "[module]\nschema_version = 2\nname = \"game\"\nversion = \"1.0.0\"\nentry = \"lifecycle.py\"\n\n\
             [[settings.fields]]\nname = \"pvp\"\ntype = \"boolean\"\ndefault = false\n\n\
             [[settings.fields]]\nname = \"damage\"\ntype = \"text\"\nrequired = true\nvisible_when = { field = \"pvp\", equals = true }\n\n\
             [[settings.fields]]\nname = \"admins\"\ntype = \"list\"\nrestart_required = true\n\n\
             [[settings.fields]]\nname = \"port\"\ntype = \"number\"\nrestart_required = true\n\n\
             [[settings.fields]]\nname = \"motd\"\ntype = \"multiline\"\n",
        )
        .unwrap();
        std::fs::write(dir.join("lifecycle.py"), "import json\nprint(json.dumps({'success': True}))\n").unwrap();
        let mut sup = crate::supervisor::Supervisor::new_with_instances_dir(modules.to_str().unwrap(), tmp.path().join("instances").to_str().unwrap());
        for module in sup.module_loader.discover_modules().unwrap() { crate::sandbox::consent_test_component(std::path::Path::new(&module.entry_path())); }
        sup.initialize().await.unwrap();
        let mut inst = crate::instance::ServerInstance::new("a", "game");
        inst.module_settings.insert("admins".into(), json!("Steve, Alex"));
        let id = inst.id.clone();
        sup.instance_store.add(inst).unwrap();
        sup.migrate_settings();
        let state = crate::ipc::IPCServer::new(std::sync::Arc::new(tokio::sync::RwLock::new(sup)), "127.0.0.1:0", crate::daemon_log::DaemonLogBuffer::new());
        let get = || {
            let (state, id) = (state.clone(), id.clone());
            async move { crate::ipc::internal::into_internal(super::get_instance(Path(id), State(state)).await).await }
        };
        let patch = |body: serde_json::Value| {
            let (state, id) = (state.clone(), id.clone());
            async move { crate::ipc::internal::into_internal(super::update_instance_settings(State(state), Path(id), Json(body)).await).await }
        };

        let read = get().await;
        assert_eq!(read.body["module_settings"]["admins"], json!(["Steve", "Alex"]), "an old string value is read as a list");
        assert_eq!(read.body["hidden"], json!(["damage"]));

        let saved = patch(json!({ "damage": "", "motd": "line one\nline two", "admins": ["Steve"] })).await;
        assert!(saved.status.is_success(), "a hidden setting is not required: {:?}", saved.body);
        assert_eq!(saved.body["restart_required"], json!([]), "the server is off");
        let shown = patch(json!({ "pvp": true, "damage": "" })).await;
        assert_eq!(shown.status.as_u16(), 400, "shown again, it is required: {:?}", shown.body);

        state.supervisor.read().await.tracker.track(&id, std::process::id()).unwrap();
        let running = patch(json!({ "admins": ["Steve"], "motd": "x" })).await;
        assert_eq!(running.body["restart_required"], json!([]), "unchanged values ask for nothing");
        let running = patch(json!({ "admins": ["Alex"], "motd": "y" })).await;
        assert_eq!(running.body["restart_required"], json!(["admins"]));
        // 인스턴스 칸에 저장되는 설정(port)은 그 칸의 지금 값과 비교한다
        let running = patch(json!({ "port": 25570 })).await;
        assert_eq!(running.body["restart_required"], json!(["port"]));
        let running = patch(json!({ "port": 25570 })).await;
        assert_eq!(running.body["restart_required"], json!([]), "saving the same value again asks for nothing");
        state.supervisor.read().await.tracker.untrack(&id).unwrap();
        let stored = std::fs::read_to_string(state.supervisor.read().await.instance_store.instance_dir(&id).join("settings.json")).unwrap();
        assert!(stored.contains("\"Alex\""), "{stored}");
    }

    /// 설치된 버전에 없는 설정(`versions` 밖)은 인스턴스 조회와 서버 목록이 `version_hidden`으로 알리고,
    /// 숨긴 설정이라 `required`를 보지 않는다. 버전을 모르는 서버는 모든 설정을 보인다
    #[tokio::test]
    async fn settings_the_installed_version_does_not_have_are_reported_as_hidden() {
        use axum::extract::{Path, State};
        use axum::Json;
        let tmp = tempfile::tempdir().unwrap();
        let modules = tmp.path().join("modules");
        let dir = modules.join("game");
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(
            dir.join("module.toml"),
            "[module]\nschema_version = 2\nname = \"game\"\nversion = \"1.0.0\"\nentry = \"lifecycle.py\"\n\n\
             [[settings.fields]]\nname = \"pvp\"\ntype = \"text\"\nrequired = true\nversions = \"<1.21.9\"\n\n\
             [[settings.fields]]\nname = \"difficulty\"\ntype = \"text\"\n",
        )
        .unwrap();
        std::fs::write(dir.join("lifecycle.py"), "import json\nprint(json.dumps({'success': True}))\n").unwrap();
        let mut sup = crate::supervisor::Supervisor::new_with_instances_dir(modules.to_str().unwrap(), tmp.path().join("instances").to_str().unwrap());
        for module in sup.module_loader.discover_modules().unwrap() { crate::sandbox::consent_test_component(std::path::Path::new(&module.entry_path())); }
        sup.initialize().await.unwrap();
        let state = crate::ipc::IPCServer::new(std::sync::Arc::new(tokio::sync::RwLock::new(sup)), "127.0.0.1:0", crate::daemon_log::DaemonLogBuffer::new());
        let made = crate::ipc::internal::into_internal(super::create_instance(State(state.clone()), Json(json!({ "name": "a", "module_name": "game" }))).await).await;
        assert!(made.status.is_success(), "{:?}", made.body);
        let id = made.body["id"].as_str().unwrap().to_string();
        let get = || {
            let (state, id) = (state.clone(), id.clone());
            async move { crate::ipc::internal::into_internal(super::get_instance(Path(id), State(state)).await).await.body }
        };
        let listed = || {
            let state = state.clone();
            async move { crate::ipc::internal::InternalResponse::from(crate::service::servers::list(&state, 0).await).body["servers"][0].clone() }
        };
        let patch = |body: serde_json::Value| {
            let (state, id) = (state.clone(), id.clone());
            async move { crate::ipc::internal::into_internal(super::update_instance_settings(State(state), Path(id), Json(body)).await).await }
        };
        assert_eq!(get().await["version_hidden"], json!([]), "the version is not known yet: every setting stays");
        assert_eq!(listed().await.get("version_hidden"), None);
        let empty = patch(json!({ "pvp": "" })).await;
        assert_eq!(empty.status.as_u16(), 400, "still a required setting: {:?}", empty.body);

        assert!(patch(json!({ "server_version": "1.21.10" })).await.status.is_success());
        assert_eq!(get().await["version_hidden"], json!(["pvp"]));
        assert_eq!(get().await["hidden"], json!(["pvp"]), "the CLI leaves it out with the other hidden settings");
        assert_eq!(listed().await["version_hidden"], json!(["pvp"]));
        let empty = patch(json!({ "pvp": "" })).await;
        assert!(empty.status.is_success(), "a setting this version does not have is not required: {:?}", empty.body);

        assert!(patch(json!({ "server_version": "1.21.8" })).await.status.is_success());
        assert_eq!(get().await["version_hidden"], json!([]));
    }

    /// 리뷰 554: 숨김 조건은 인스턴스 칸에 저장되는 설정(port 등)의 지금 값을 보고, list 기본값은 만들 때 채우며,
    /// list에 빈 문자열은 받지 않는다. 모듈을 다시 읽어 문자열 설정이 list가 되면 그때 옮긴다
    #[tokio::test]
    async fn list_defaults_instance_field_conditions_and_refresh_follow_the_contract() {
        use axum::extract::{Path, State};
        use axum::Json;
        let tmp = tempfile::tempdir().unwrap();
        let modules = tmp.path().join("modules");
        let dir = modules.join("game");
        std::fs::create_dir_all(&dir).unwrap();
        let toml = |admins_type: &str| {
            format!(
                "[module]\nschema_version = 2\nname = \"game\"\nversion = \"1.0.0\"\nentry = \"lifecycle.py\"\n\n\
                 [[settings.fields]]\nname = \"port\"\ntype = \"number\"\ndefault = 25565\n\n\
                 [[settings.fields]]\nname = \"note\"\ntype = \"text\"\nvisible_when = {{ field = \"port\", equals = 25565 }}\n\n\
                 [[settings.fields]]\nname = \"ops\"\ntype = \"list\"\ndefault = [\"a\", \"b\"]\n\n\
                 [[settings.fields]]\nname = \"admins\"\ntype = \"{admins_type}\"\n"
            )
        };
        std::fs::write(dir.join("module.toml"), toml("text")).unwrap();
        std::fs::write(dir.join("lifecycle.py"), "import json\nprint(json.dumps({'success': True}))\n").unwrap();
        let mut sup = crate::supervisor::Supervisor::new_with_instances_dir(modules.to_str().unwrap(), tmp.path().join("instances").to_str().unwrap());
        for module in sup.module_loader.discover_modules().unwrap() { crate::sandbox::consent_test_component(std::path::Path::new(&module.entry_path())); }
        sup.initialize().await.unwrap();
        let state = crate::ipc::IPCServer::new(std::sync::Arc::new(tokio::sync::RwLock::new(sup)), "127.0.0.1:0", crate::daemon_log::DaemonLogBuffer::new());
        let made = crate::ipc::internal::into_internal(super::create_instance(State(state.clone()), Json(json!({ "name": "a", "module_name": "game", "port": 25565 }))).await).await;
        assert!(made.status.is_success(), "{:?}", made.body);
        let id = made.body["id"].as_str().unwrap().to_string();
        let get = || {
            let (state, id) = (state.clone(), id.clone());
            async move { crate::ipc::internal::into_internal(super::get_instance(Path(id), State(state)).await).await }
        };
        let patch = |body: serde_json::Value| {
            let (state, id) = (state.clone(), id.clone());
            async move { crate::ipc::internal::into_internal(super::update_instance_settings(State(state), Path(id), Json(body)).await).await }
        };
        assert_eq!(get().await.body["module_settings"]["ops"], json!(["a", "b"]), "a list default is filled in");
        assert_eq!(get().await.body["hidden"], json!([]));
        assert!(patch(json!({ "port": 26000 })).await.status.is_success());
        assert_eq!(get().await.body["hidden"], json!(["note"]), "the condition reads the port the instance has now");
        let empty = patch(json!({ "ops": "" })).await;
        assert_eq!(empty.status.as_u16(), 400, "a list is never a string: {:?}", empty.body);

        patch(json!({ "admins": "Steve, Alex" })).await;
        std::fs::write(dir.join("module.toml"), toml("list")).unwrap();
        let refreshed = crate::ipc::internal::into_internal(crate::ipc::handlers::server::refresh_modules(State(state.clone())).await).await;
        assert!(refreshed.status.is_success(), "{:?}", refreshed.body);
        assert_eq!(get().await.body["module_settings"]["admins"], json!(["Steve", "Alex"]), "moved when the modules are read again");
    }

    /// 게임 설정 파일로 넘기는 값(configure)에서 사바쨩 설정(`group = "saba-chan"`)과 데몬 필드는 뺀다.
    /// 어느 키가 사바쨩 설정인지는 모듈 선언이 정한다: 데몬은 게임별 키 이름을 모른다 (MK6)
    #[tokio::test]
    async fn only_game_settings_are_passed_to_configure() {
        use axum::extract::{Path, State};
        use axum::Json;
        let tmp = tempfile::tempdir().unwrap();
        let modules = tmp.path().join("modules");
        let dir = modules.join("game");
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(
            dir.join("module.toml"),
            "[module]\nschema_version = 1\nname = \"game\"\nversion = \"1.0.0\"\nentry = \"lifecycle.py\"\n\n\
             [[settings.fields]]\nname = \"motd\"\ntype = \"text\"\n\n\
             [[settings.fields]]\nname = \"heap\"\ntype = \"text\"\ngroup = \"saba-chan\"\n\n\
             [[settings.fields]]\nname = \"ram\"\ntype = \"text\"\ngroup = \"advanced\"\n",
        )
        .unwrap();
        std::fs::write(
            dir.join("lifecycle.py"),
            "import json, sys\nconfig = json.load(sys.stdin)\n\
             print(json.dumps({'success': False, 'message': json.dumps(sorted(config.get('settings', {})))}))\n",
        )
        .unwrap();
        let mut sup = crate::supervisor::Supervisor::new_with_instances_dir(
            modules.to_str().unwrap(),
            tmp.path().join("instances").to_str().unwrap(),
        );
        for module in sup.module_loader.discover_modules().unwrap() { crate::sandbox::consent_test_component(std::path::Path::new(&module.entry_path())); }
        sup.initialize().await.unwrap();
        let mut inst = crate::instance::ServerInstance::new("a", "game");
        inst.working_dir = Some(tmp.path().to_string_lossy().to_string());
        let id = inst.id.clone();
        sup.instance_store.add(inst).unwrap();
        let state = crate::ipc::IPCServer::new(std::sync::Arc::new(tokio::sync::RwLock::new(sup)), "127.0.0.1:0", crate::daemon_log::DaemonLogBuffer::new());

        // 데몬이 읽는 설정(DAEMON_SETTINGS)과 실행 칸도 게임 설정 파일로 가지 않는다: GUI는 저장마다 auto_restart를 함께 보낸다
        let body = json!({
            "motd": "hi", "heap": "4G", "ram": "8", "protocol_mode": "rest",
            "auto_restart": true, "restart_on_unknown_exit": false, "managed_start": true, "log_sessions_keep": 5,
            "stop_timeout": 90, "memory_limit_mb": "", "extra_args": "-Dx=1", "game_chat": "all",
        });
        let saved = crate::ipc::internal::into_internal(super::update_instance_settings(State(state.clone()), Path(id.clone()), Json(body)).await).await;
        assert!(saved.status.is_success(), "{:?}", saved.body);
        // 받은 키는 실패 메시지로 돌려받는다: Windows에서 확인되지 않은 모듈은 격리되어 모듈 폴더에 쓸 수 없다
        let configured: Vec<String> = serde_json::from_str(saved.body["sync_error"].as_str().unwrap()).unwrap();
        assert_eq!(configured, vec!["motd", "ram"], "a saba-chan setting and daemon fields stay out; a game setting named like another game's launcher setting goes in");
        assert_eq!(crate::supervisor::game_chat::settings(&id).mode, crate::supervisor::game_chat::Mode::All, "a running server follows at once");

        // 게임 채팅 감지 단어 모드에는 단어가 있어야 한다
        let patch = |body: serde_json::Value| {
            let (state, id) = (state.clone(), id.clone());
            async move { crate::ipc::internal::into_internal(super::update_instance_settings(State(state), Path(id), Json(body)).await).await }
        };
        assert_eq!(patch(json!({ "game_chat": "keyword" })).await.status, axum::http::StatusCode::BAD_REQUEST);
        assert_eq!(patch(json!({ "game_chat": "loud" })).await.status, axum::http::StatusCode::BAD_REQUEST);
        patch(json!({ "game_chat": "keyword", "game_chat_keyword": " !d " })).await;
        let stored = state.supervisor.read().await.instance_store.get(&id).cloned().unwrap();
        assert_eq!((stored.game_chat, stored.game_chat_keyword.as_str()), (crate::supervisor::game_chat::Mode::Keyword, "!d"));

        // 전용 멤버 명단: 빈 명단으로는 명단 밖 자동 강퇴를 켤 수 없고, 겹친 닉네임은 거절한다
        assert_eq!(patch(json!({ "access": "members", "controllers_only": true })).await.status, axum::http::StatusCode::BAD_REQUEST);
        assert_eq!(patch(json!({ "controllers": [{ "name": "a" }, { "name": "A" }] })).await.status, axum::http::StatusCode::BAD_REQUEST);
        let saved = patch(json!({ "access": "members", "controllers": [{ "name": " Steve ", "discord_id": "332007306403250178" }], "controllers_only": true })).await;
        assert!(saved.status.is_success(), "{:?}", saved.body);
        let stored = state.supervisor.read().await.instance_store.get(&id).cloned().unwrap();
        assert_eq!((stored.controllers[0].name.as_str(), stored.controllers_only), ("Steve", true));
        assert_eq!(patch(json!({ "controllers": [] })).await.status, axum::http::StatusCode::BAD_REQUEST, "emptying the list keeps the gate on");
        // 서버 공용으로 바꾸면 명단 밖 자동 강퇴(전용 멤버의 스위치)는 꺼진다 (리뷰 P1: access 없는 예전 파일과 가려진다)
        assert!(patch(json!({ "access": "shared" })).await.status.is_success());
        let stored = state.supervisor.read().await.instance_store.get(&id).cloned().unwrap();
        assert_eq!((stored.access, stored.controllers_only), (crate::controllers::Access::Shared, false));
    }

    /// 설치·업데이트·백업·복원이 그 폴더를 쓰는 동안 서버 폴더(`working_dir`)를 옮기지 않는다:
    /// 업데이트는 시작할 때 계산한 폴더를 기준으로 다른 인스턴스를 잠그고 받는다
    #[tokio::test]
    async fn the_server_folder_does_not_move_while_a_task_holds_the_instance() {
        use axum::extract::{Path, State};
        use axum::Json;
        let tmp = tempfile::tempdir().unwrap();
        let modules = tmp.path().join("modules");
        std::fs::create_dir_all(modules.join("game")).unwrap();
        std::fs::write(
            modules.join("game").join("module.toml"),
            "[module]\nschema_version = 1\nname = \"game\"\nversion = \"1.0.0\"\nentry = \"lifecycle.py\"\n",
        )
        .unwrap();
        std::fs::write(modules.join("game").join("lifecycle.py"), "").unwrap();
        let mut sup = crate::supervisor::Supervisor::new_with_instances_dir(modules.to_str().unwrap(), tmp.path().join("instances").to_str().unwrap());
        for module in sup.module_loader.discover_modules().unwrap() { crate::sandbox::consent_test_component(std::path::Path::new(&module.entry_path())); }
        sup.initialize().await.unwrap();
        let mut inst = crate::instance::ServerInstance::new("srv", "game");
        let here = tmp.path().join("here").to_string_lossy().into_owned();
        inst.working_dir = Some(here.clone());
        let id = inst.id.clone();
        sup.instance_store.add(inst).unwrap();
        let state = crate::ipc::IPCServer::new(std::sync::Arc::new(tokio::sync::RwLock::new(sup)), "127.0.0.1:0", crate::daemon_log::DaemonLogBuffer::new());

        let busy = crate::instance::lock::InstanceLock::take(&id).unwrap();
        let elsewhere = tmp.path().join("elsewhere").to_string_lossy().into_owned();
        let moved = crate::ipc::internal::into_internal(super::update_instance_settings(State(state.clone()), Path(id.clone()), Json(json!({ "working_dir": elsewhere }))).await).await;
        assert_eq!(moved.status, 409, "{}", moved.body);
        assert_eq!(moved.body["error_code"], "instance_busy");
        // 같은 값을 다시 보내는 저장(GUI는 설정 전체를 보낸다)은 막지 않는다
        let same = crate::ipc::internal::into_internal(super::update_instance_settings(State(state.clone()), Path(id.clone()), Json(json!({ "working_dir": here }))).await).await;
        assert_eq!(same.status, 200, "{}", same.body);
        drop(busy);
        let moved = crate::ipc::internal::into_internal(super::update_instance_settings(State(state.clone()), Path(id.clone()), Json(json!({ "working_dir": elsewhere }))).await).await;
        assert_eq!(moved.status, 200, "{}", moved.body);

        // 다른 인스턴스가 쓰는 폴더로는 옮기지 않는다: 만들 때처럼 한 폴더에 서버 둘을 두지 않는다
        // (켜져 있거나 작업 중이면 그 서버가 옮긴 서버의 파일을 덮어쓴다)
        let mut other = crate::instance::ServerInstance::new("other", "game");
        let theirs = tmp.path().join("theirs").to_string_lossy().into_owned();
        other.working_dir = Some(theirs.clone());
        let other_id = other.id.clone();
        let their_instance_dir = {
            let mut sup = state.supervisor.write().await;
            sup.instance_store.add(other).unwrap();
            sup.instance_store.instance_dir(&other_id).to_string_lossy().into_owned()
        };
        // 그 폴더의 안이나 바깥도 같다: 한쪽의 복원·삭제가 다른 쪽 서버 파일까지 치운다
        let inside = tmp.path().join("theirs").join("sub").to_string_lossy().into_owned();
        let outside = tmp.path().to_string_lossy().into_owned();
        for dir in [&theirs, &their_instance_dir, &inside, &outside] {
            let taken = crate::ipc::internal::into_internal(super::update_instance_settings(State(state.clone()), Path(id.clone()), Json(json!({ "working_dir": dir }))).await).await;
            assert_eq!(taken.status, 409, "{dir}: {}", taken.body);
            assert_eq!(taken.body["error_code"], "working_dir_in_use");
            assert_eq!(taken.body["instance"], "other");
            let made = crate::ipc::internal::into_internal(super::create_instance(State(state.clone()), Json(json!({ "name": "new", "module_name": "game", "working_dir": dir }))).await).await;
            assert_eq!(made.status, 409, "{dir}: {}", made.body);
            assert_eq!(made.body["error_code"], "working_dir_in_use");
        }
        // 이름만 비슷한 옆 폴더는 다른 폴더다
        let beside = tmp.path().join("theirs2").to_string_lossy().into_owned();
        let moved = crate::ipc::internal::into_internal(super::update_instance_settings(State(state.clone()), Path(id.clone()), Json(json!({ "working_dir": beside }))).await).await;
        assert_eq!(moved.status, 200, "{}", moved.body);
    }

    #[tokio::test]
    async fn only_values_the_user_changed_count_as_user_settings() {
        use axum::extract::{Path, State};
        use axum::Json;
        let tmp = tempfile::tempdir().unwrap();
        let modules = tmp.path().join("modules");
        let dir = modules.join("game");
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(
            dir.join("module.toml"),
            "[module]\nschema_version = 1\nname = \"game\"\nversion = \"1.0.0\"\nentry = \"lifecycle.py\"\n\n\
             [[settings.fields]]\nname = \"java_path\"\ntype = \"text\"\ndefault = \"{instance_dir}/jdk/bin/java\"\n",
        )
        .unwrap();
        std::fs::write(dir.join("lifecycle.py"), "import json, sys\nprint(json.dumps({'success': True}))\n").unwrap();
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
        let user_settings = |id: String| {
            let state = state.clone();
            async move { state.supervisor.read().await.instance_store.get(&id).unwrap().user_settings.clone() }
        };

        // 기본값을 채워 보내도, 아무것도 안 보내도 모듈이 정한 값이다
        let filled = create(json!({ "name": "a", "module_name": "game", "module_settings": { "java_path": "{instance_dir}/jdk/bin/java" } })).await;
        let id = filled.body["id"].as_str().unwrap().to_string();
        assert!(user_settings(id.clone()).await.is_empty());
        let typed = create(json!({ "name": "b", "module_name": "game", "module_settings": { "java_path": "/opt/jdk/bin/java" } })).await;
        assert!(user_settings(typed.body["id"].as_str().unwrap().to_string()).await.contains("java_path"));

        // 저장할 때 고치지 않은 값을 함께 보내도 사용자가 정한 것이 아니다. 바꾼 값만 기록한다
        let patch = |body: serde_json::Value| {
            let (state, id) = (state.clone(), id.clone());
            async move { crate::ipc::internal::into_internal(super::update_instance_settings(State(state), Path(id), Json(body)).await).await }
        };
        let current = state.supervisor.read().await.instance_store.get(&id).unwrap().module_settings["java_path"].clone();
        assert!(patch(json!({ "java_path": current })).await.status.is_success());
        assert!(user_settings(id.clone()).await.is_empty());
        assert!(patch(json!({ "java_path": "/opt/jdk/bin/java" })).await.status.is_success());
        assert!(user_settings(id.clone()).await.contains("java_path"));
    }

    /// 정지 대기 시간은 잘못된 값을 저장하지 않고 400으로 알린다. 숫자 글자는 숫자로, 빈 값은 모듈 값으로 되돌린다
    #[tokio::test]
    async fn a_stop_timeout_patch_is_checked_before_it_is_saved() {
        use axum::extract::{Path, State};
        use axum::Json;
        let tmp = tempfile::tempdir().unwrap();
        let dir = tmp.path().join("modules").join("game");
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(dir.join("module.toml"), "[module]\nschema_version = 1\nname = \"game\"\nversion = \"1.0.0\"\nentry = \"lifecycle.py\"\n").unwrap();
        std::fs::write(dir.join("lifecycle.py"), "import json, sys\nprint(json.dumps({'success': True}))\n").unwrap();
        let mut sup = crate::supervisor::Supervisor::new_with_instances_dir(
            tmp.path().join("modules").to_str().unwrap(),
            tmp.path().join("instances").to_str().unwrap(),
        );
        for module in sup.module_loader.discover_modules().unwrap() { crate::sandbox::consent_test_component(std::path::Path::new(&module.entry_path())); }
        sup.initialize().await.unwrap();
        let state = crate::ipc::IPCServer::new(std::sync::Arc::new(tokio::sync::RwLock::new(sup)), "127.0.0.1:0", crate::daemon_log::DaemonLogBuffer::new());
        let made = crate::ipc::internal::into_internal(super::create_instance(State(state.clone()), Json(json!({ "name": "a", "module_name": "game" }))).await).await;
        let id = made.body["id"].as_str().unwrap().to_string();
        let patch = |body: serde_json::Value| {
            let (state, id) = (state.clone(), id.clone());
            async move { crate::ipc::internal::into_internal(super::update_instance_settings(State(state), Path(id), Json(body)).await).await }
        };
        let saved = || async { state.supervisor.read().await.instance_store.get(&id).unwrap().module_settings.get("stop_timeout").cloned() };

        assert!(patch(json!({ "stop_timeout": "60" })).await.status.is_success());
        assert_eq!(saved().await, Some(json!(60)));
        let bad = patch(json!({ "stop_timeout": -5 })).await;
        assert_eq!(bad.status, axum::http::StatusCode::BAD_REQUEST);
        assert_eq!(bad.body["error_code"], "invalid_format");
        assert_eq!(bad.body["fields"][0]["field"], "stop_timeout");
        assert_eq!(saved().await, Some(json!(60)), "a rejected value changes nothing");
        assert!(patch(json!({ "stop_timeout": null })).await.status.is_success());
        assert_eq!(saved().await, None);
    }

    #[tokio::test]
    async fn null_clears_the_executable_override() {
        use axum::extract::{Path, State};
        use axum::Json;
        let tmp = tempfile::tempdir().unwrap();
        let modules = tmp.path().join("modules");
        let dir = modules.join("game");
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(dir.join("module.toml"), "[module]\nschema_version = 1\nname = \"game\"\nversion = \"1.0.0\"\nentry = \"lifecycle.py\"\n").unwrap();
        std::fs::write(dir.join("lifecycle.py"), "import json, sys\nprint(json.dumps({'success': True}))\n").unwrap();
        let mut sup = crate::supervisor::Supervisor::new_with_instances_dir(
            modules.to_str().unwrap(),
            tmp.path().join("instances").to_str().unwrap(),
        );
        for module in sup.module_loader.discover_modules().unwrap() { crate::sandbox::consent_test_component(std::path::Path::new(&module.entry_path())); }
        sup.initialize().await.unwrap();
        let state = crate::ipc::IPCServer::new(std::sync::Arc::new(tokio::sync::RwLock::new(sup)), "127.0.0.1:0", crate::daemon_log::DaemonLogBuffer::new());
        let made = crate::ipc::internal::into_internal(super::create_instance(State(state.clone()), Json(json!({ "name": "a", "module_name": "game" }))).await).await;
        let id = made.body["id"].as_str().unwrap().to_string();
        let patch = |body: serde_json::Value| {
            let (state, id) = (state.clone(), id.clone());
            async move { crate::ipc::internal::into_internal(super::update_instance_settings(State(state), Path(id), Json(body)).await).await }
        };
        let exe = tmp.path().join("server").join("run.sh");
        assert!(patch(json!({ "executable_path": exe.to_str().unwrap() })).await.status.is_success());
        assert!(patch(json!({ "executable_path": null })).await.status.is_success());
        let inst = state.supervisor.read().await.instance_store.get(&id).cloned().unwrap();
        assert_eq!(inst.executable_path, None);
        assert!(!inst.module_settings.contains_key("executable_path"));

        // 작업 폴더 없이 실행 파일만 정했던 인스턴스: 지울 때 옛 실행 파일 폴더로 작업 폴더를 채우지 않는다
        {
            let mut sup = state.supervisor.write().await;
            let mut legacy = sup.instance_store.get(&id).cloned().unwrap();
            legacy.executable_path = Some(exe.to_string_lossy().into_owned());
            legacy.working_dir = None;
            sup.instance_store.update(&id, legacy).unwrap();
        }
        assert!(patch(json!({ "executable_path": null })).await.status.is_success());
        let inst = state.supervisor.read().await.instance_store.get(&id).cloned().unwrap();
        assert_eq!((inst.executable_path, inst.working_dir), (None, None));

        // 비울 수 없는 칸은 null이어도 지금 값을 둔다 (CLI가 이 표로 빈 값을 거절한다)
        let set = json!({ "port": 25000, "working_dir": "/srv/game", "protocol_mode": "rest", "server_version": "1.2" });
        assert!(patch(set.clone()).await.status.is_success());
        for key in saba_chan_updater_lib::constants::UNCLEARABLE_INSTANCE_FIELDS {
            assert!(patch(json!({ *key: null })).await.status.is_success());
        }
        let after = serde_json::to_value(state.supervisor.read().await.instance_store.get(&id).cloned().unwrap()).unwrap();
        for key in saba_chan_updater_lib::constants::UNCLEARABLE_INSTANCE_FIELDS {
            assert_eq!(after[*key], set[*key], "{key}");
        }
    }

    #[test]
    fn stop_timeout_is_empty_or_whole_seconds_up_to_an_hour() {
        for empty in [json!(null), json!(""), json!(0), json!(" ")] {
            assert_eq!(stop_timeout_from(&empty), Ok(None), "{empty}");
        }
        assert_eq!(stop_timeout_from(&json!(60)), Ok(Some(60)));
        assert_eq!(stop_timeout_from(&json!(" 90 ")), Ok(Some(90)));
        assert_eq!(stop_timeout_from(&json!(3600)), Ok(Some(3600)));
        for bad in [json!(-5), json!("abc"), json!(1.5), json!(true), json!(99999), json!("3601")] {
            assert!(stop_timeout_from(&bad).is_err(), "{bad}");
        }
    }

    #[test]
    fn memory_limit_is_empty_or_a_whole_number_of_mib() {
        assert_eq!(memory_limit_from(&json!(null)), Ok(None));
        assert_eq!(memory_limit_from(&json!("")), Ok(None));
        assert_eq!(memory_limit_from(&json!(0)), Ok(None));
        assert_eq!(memory_limit_from(&json!(6144)), Ok(Some(6144)));
        assert_eq!(memory_limit_from(&json!(" 4096 ")), Ok(Some(4096)));
        assert!(memory_limit_from(&json!(100)).is_err());
        assert!(memory_limit_from(&json!(4.5)).is_err());
        assert!(memory_limit_from(&json!(-1)).is_err());
        assert_eq!(memory_limit_from(&json!("4G")), Ok(Some(4096)));
        assert_eq!(memory_limit_from(&json!("6gb")), Ok(Some(6144)));
        assert_eq!(memory_limit_from(&json!("1024M")), Ok(Some(1024)));
        assert!(memory_limit_from(&json!("4T")).is_err());
        assert!(memory_limit_from(&json!("B")).is_err());
        assert!(memory_limit_from(&json!(true)).is_err());
    }
}
