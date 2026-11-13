//! 인스턴스 지우기: 삭제 전 hook, 가져온 서버 폴더 떼어 두기

use super::*;

/// 가져온 서버를 지우기 전에 거둔 결과
struct Unlinked {
    /// 남는 서버 폴더 (그 자리에 연결한 서버). 옮겨 온 서버는 인스턴스 폴더와 함께 지워져 없다
    folder: Option<String>,
    restored: crate::instance::migration::Restored,
}

/// 가져온 서버를 지우기 전에: 보관한 원래 게임 설정 파일로 되돌리고(`keep_settings`가 아니면. 옮겨 온 서버도
/// 서버 폴더 밖 설정 파일(좀보이드 데이터 폴더 등)이 남으므로 되돌린다), 인스턴스 폴더 밖 서버 폴더에는
/// 격리 실행에 준 폴더 권한·낮은 무결성 표시를 거둔다. 서버 폴더 밖의 파일은 지우지 않는다. 가져온 서버가 아니면 None
/// `detached`: 지우려고 옮겨 둔 인스턴스 폴더(`InstanceStore::detach_dir`). 보관한 원래 설정은 거기서 읽는다
async fn unlink_server_folder(state: &IPCServer, instance: &crate::instance::ServerInstance, keep_settings: bool, detached: Option<&std::path::Path>) -> Option<Unlinked> {
    let (instance_dir, metadata, others) = {
        let supervisor = state.supervisor.read().await;
        let others = other_instance_folders(&supervisor, &instance.id);
        let metadata = supervisor.module_loader.get_module(&instance.module_name).ok().map(|m| m.metadata);
        (supervisor.instance_store.instance_dir(&instance.id), metadata, others)
    };
    let folder = crate::instance::migration::linked_folder(instance.working_dir.as_deref(), &instance_dir);
    let records = detached.map_or_else(|| instance_dir.clone(), std::path::Path::to_path_buf);
    if folder.is_none() && !crate::instance::migration::saved(&records) {
        return None;
    }
    let boundary = crate::module_paths::Boundary::for_instance(instance, &records, metadata.as_ref());
    let exe_dir = instance.executable_path.as_deref().and_then(|p| std::path::Path::new(p).parent()).map(std::path::Path::to_path_buf);
    // 실행 파일 폴더는 그 서버 폴더 안일 때만 (밖이면 다른 인스턴스와 함께 쓰는 런타임일 수 있다)
    let dirs: Vec<std::path::PathBuf> = folder.iter().map(std::path::PathBuf::from).collect();
    let exe_dir = exe_dir.filter(|d| crate::module_paths::is_inside(d, &dirs));
    let dirs: Vec<std::path::PathBuf> = dirs.iter().cloned().chain(exe_dir).collect();
    let name = instance.name.clone();
    // 파일 복사와 폴더 걷기(표시 덮기)는 막는 작업이라 런타임 밖에서
    let restored = tokio::task::spawn_blocking(move || {
        let restored = if keep_settings {
            crate::instance::migration::Restored::default()
        } else {
            crate::instance::migration::restore(&records, |path| boundary.resolve(path).is_some())
        };
        if dirs.is_empty() {
            return restored;
        }
        if let Err(failed) = crate::sandbox::release(&dirs, &others) {
            tracing::warn!("Could not take back sandbox permissions on the server folder of '{}': {}", name, failed.join("; "));
        }
        restored
    })
    .await
    .unwrap_or_default();
    Some(Unlinked { folder, restored })
}

/// `id` 말고 다른 인스턴스들의 인스턴스 폴더와 서버 폴더 (`sandbox::release`가 그들이 함께 쓰는 위 폴더를 둔다)
pub(super) fn other_instance_folders(supervisor: &crate::supervisor::Supervisor, id: &str) -> Vec<std::path::PathBuf> {
    supervisor.instance_store.list().iter()
        .filter(|i| i.id != id)
        .flat_map(|i| [Some(supervisor.instance_store.instance_dir(&i.id)), i.working_dir.as_deref().map(std::path::PathBuf::from)])
        .flatten()
        .collect()
}

/// 인스턴스를 지우기 전 익스텐션 hook `server.pre_delete`. 익스텐션이 정리했으면 파일 잠금이 풀리기를 잠깐 기다린다
pub(super) async fn pre_delete_hook(state: &IPCServer, instance: &crate::instance::ServerInstance) {
    let ext_mgr = state.extension_manager.clone();
    let ctx = {
        let sup = state.supervisor.read().await;
        let module = sup.module_loader.get_module(&instance.module_name).ok();
        crate::extension::hook_context(instance, &sup.instance_store.instance_dir(&instance.id), module.as_ref().map(|m| &m.metadata))
    };
    let results = crate::plugin::with_instance(Some(state.host_context(&instance.id)), crate::extension::run_hook(&ext_mgr, "server.pre_delete", ctx)).await;
    let handled = results.iter().any(|(_id, r)| {
        r.as_ref()
            .map(|v| v.get("handled").and_then(|h| h.as_bool()) == Some(true))
            .unwrap_or(false)
    });
    if handled {
        tracing::info!("Extension handled pre_delete cleanup for '{}'", instance.name);
        // 익스텐션 정리 후 파일 잠금 해제 대기
        tokio::time::sleep(std::time::Duration::from_secs(2)).await;
    }
}

/// `DELETE /api/instance/:id`의 선택 값
#[derive(Debug, Default, serde::Deserialize)]
pub struct DeleteQuery {
    /// 가져온 서버의 게임 설정 파일을 가져오기 전 내용으로 되돌리지 않는다 (사바쨩에서 바꾼 설정을 남긴다)
    #[serde(default)]
    pub keep_settings: bool,
}

/// DELETE /api/instance/:id - 인스턴스 삭제
pub async fn delete_instance(
    Path(id): Path<String>,
    State(state): State<IPCServer>,
    Query(query): Query<DeleteQuery>,
) -> impl IntoResponse {
    // 백업·복원·업데이트 중에는 폴더를 지우지 않는다
    let Some(guard) = super::super::backup::InstanceLock::take_for_writes(&id).await else {
        return super::super::backup::instance_busy();
    };
    // 요청이 끊겨도 정지·삭제를 끝까지 한다: 추적을 뺀 채 멈추면 돌던 서버가 추적 밖에 남는다
    let work = crate::audit::spawn(async move {
        let _guard = guard;
        crate::port_check::forget(&id);
        delete_instance_held(state, id, query.keep_settings).await
    });
    work.await.unwrap_or_else(|e| {
        let error = json!({ "success": false, "error": format!("Failed to delete instance: {}", e) });
        (StatusCode::INTERNAL_SERVER_ERROR, Json(error)).into_response()
    })
}

/// 작업 잠금을 잡은 뒤의 인스턴스 삭제
async fn delete_instance_held(state: IPCServer, id: String, keep_settings: bool) -> axum::response::Response {
    let supervisor = state.supervisor.write().await;

    // ── 인스턴스 존재 확인 ──
    let instance = match supervisor.instance_store.get(&id) {
        Some(inst) => inst.clone(),
        None => {
            let error = json!({ "success": false, "error": "Instance not found" });
            return (StatusCode::NOT_FOUND, Json(error)).into_response();
        }
    };

    // ── 실행 중이면 먼저 정지 ──
    // 저장소에서 떼어 내는 것만 락 안에서, 강제 종료 대기는 락 밖에서 (C12)
    let running_pid = supervisor.tracker.get_pid(&id).ok();
    let started = supervisor.tracker.get_start_time(&id).ok();
    let managed = supervisor.managed_store.remove(&id).await;
    let _ = supervisor.tracker.untrack(&id);
    drop(supervisor);
    if let Some(pid) = running_pid {
        use crate::supervisor::process::{force_kill_tree, is_process_alive, wait_gone};
        tracing::info!("Instance '{}' is running, force-stopping before delete...", instance.name);
        // Managed process가 있으면 그 PID, 아니면 추적하던 PID를 강제 종료
        let killed = match managed.as_ref() {
            Some(m) if m.is_running() => m.force_kill().await,
            Some(_) => Ok(()),
            None => tokio::task::spawn_blocking(move || force_kill_tree(pid)).await.unwrap_or_else(|e| Err(e.into())),
        };
        let gone = match killed {
            Ok(()) => {
                let alive = || managed.as_ref().map_or_else(|| is_process_alive(pid), |m| m.is_running());
                wait_gone(std::time::Duration::from_secs(5), alive).await
            }
            Err(e) => {
                tracing::warn!("Could not stop instance '{}' before delete: {}", instance.name, e);
                false
            }
        };
        // 서버가 아직 돌고 있으면 추적을 되돌리고 파일을 지우지 않는다
        if !gone {
            let supervisor = state.supervisor.write().await;
            if let Some(m) = managed {
                supervisor.managed_store.insert(&id, m).await;
            }
            let _ = supervisor.tracker.track_since(&id, pid, started.unwrap_or_default());
            let error = json!({ "success": false, "error": "The server is still running and could not be stopped", "error_code": "server_running" });
            return (StatusCode::CONFLICT, Json(error)).into_response();
        }
    }

    // ── Extension hook: server.pre_delete ──
    pre_delete_hook(&state, &instance).await;

    // ── 지울 수 있는지 먼저 본다: 인스턴스 폴더를 데몬 전용 자리로 옮긴다 (Windows 파일 잠금 대비 재시도) ──
    // 옮기지 못하면 원래 설정 되돌리기·권한 거두기 전에 멈춘다 (되돌린 뒤 인스턴스가 남으면 설정과 파일이 어긋난다)
    let mut detached = None;
    let mut last_err = None;
    for attempt in 0..3 {
        let moved = state.supervisor.read().await.instance_store.detach_dir(&id);
        match moved {
            Ok(dir) => {
                detached = Some(dir);
                break;
            }
            Err(e) => {
                tracing::warn!("Instance '{}' delete attempt {}/3 failed: {:#}", instance.name, attempt + 1, e);
                last_err = Some(e);
                if attempt < 2 {
                    // 재시도 전 대기 — 파일 잠금 해제 기다림
                    tokio::time::sleep(std::time::Duration::from_secs(2)).await;
                }
            }
        }
    }
    let Some(detached) = detached else {
        let err_msg = last_err.map(|e| format!("{:#}", e)).unwrap_or_else(|| "Unknown error".to_string());
        let error = json!({ "success": false, "error": format!("Failed to delete instance: {}", err_msg) });
        return (StatusCode::INTERNAL_SERVER_ERROR, Json(error)).into_response();
    };

    // ── 가져온 서버: 서버 폴더는 두고, 사바쨩이 남긴 것을 거둔다 ──
    let unlinked = unlink_server_folder(&state, &instance, keep_settings, detached.as_deref()).await;

    // 되돌리지 못한 원래 설정은 유일한 사본이다: 옮겨 둔 폴더와 함께 지우지 않고 기록 폴더에 남긴다.
    // 남기지 못하면 옮겨 둔 폴더를 지우지 않는다 (표시가 없으니 다음 시작 때 인스턴스로 되돌아온다)
    let mut kept_at = None;
    let mut keep_folder = false;
    if let (Some(dir), Some(unlinked)) = (&detached, &unlinked) {
        if !unlinked.restored.failed.is_empty() {
            let to = state.supervisor.read().await.instance_store.state_root().join(crate::instance::migration::UNRESTORED).join(dir.file_name().unwrap_or_default());
            match crate::instance::migration::keep_unrestored(dir, &to) {
                Ok(()) => kept_at = Some(to),
                Err(e) => {
                    tracing::error!("Could not keep the settings of '{}' that were not restored: {}", instance.name, e);
                    keep_folder = true;
                }
            }
        }
    }
    // 연결 해제를 마쳤다는 표시가 없으면 다음 시작 때 인스턴스를 되돌린다 (그 사이 데몬이 멈춘 경우)
    if let Some(dir) = detached.as_ref().filter(|_| !keep_folder) {
        if let Err(e) = crate::instance::InstanceStore::mark_unlinked(dir) {
            tracing::warn!("{:#}", e);
        }
    }
    let removed = state.supervisor.write().await.remove_instance(&id);
    if let Some(dir) = detached.filter(|_| !keep_folder) {
        let name = instance.name.clone();
        let _ = tokio::task::spawn_blocking(move || {
            match std::fs::remove_dir_all(&dir) {
                Ok(()) => crate::instance::InstanceStore::forget_unlinked(&dir),
                Err(e) => tracing::warn!("Could not remove the folder of deleted instance '{}' (removed at next start): {}", name, e),
            }
        })
        .await;
    }
    match removed {
        Ok(_) => {
            state.provision_tracker.forget(&id);
            // 지운 서버의 브랜치는 독립 서버로 남는다
            {
                let mut supervisor = state.supervisor.write().await;
                let branches: Vec<_> = supervisor.instance_store.list().iter().filter(|i| i.branch_of.as_deref() == Some(id.as_str())).cloned().collect();
                for mut branch in branches {
                    branch.branch_of = None;
                    let branch_id = branch.id.clone();
                    if let Err(e) = supervisor.instance_store.update(&branch_id, branch) {
                        tracing::warn!("Could not release branch server {} of deleted instance '{}': {}", branch_id, instance.name, e);
                    }
                }
            }
            // 지운 서버의 디스코드 낱말은 heartbeat에 더 싣지 않는다
            if let Err(e) = state.config_store.update_server_words(|t| ((), t.remove(&id).is_some())).await {
                tracing::warn!("Could not forget the Discord words of deleted instance '{}': {}", instance.name, e);
            }
            let mut response = json!({ "success": true });
            if let Some(unlinked) = &unlinked {
                if let Some(folder) = &unlinked.folder {
                    response["linked_folder"] = json!(folder);
                }
                response["settings_restored"] = json!(unlinked.restored.restored);
                response["settings_restore_failed"] = json!(unlinked.restored.failed.len());
                if let Some(kept_at) = &kept_at {
                    response["settings_kept_at"] = json!(kept_at.to_string_lossy());
                }
            }
            (StatusCode::OK, Json(response)).into_response()
        }
        Err(e) => {
            let error = json!({ "success": false, "error": format!("Failed to delete instance: {}", e) });
            (StatusCode::INTERNAL_SERVER_ERROR, Json(error)).into_response()
        }
    }
}

#[cfg(test)]
mod tests {
    /// 가져온 서버를 지우면(연결 해제) 서버 폴더는 남고, 사바쨩이 고친 게임 설정 파일은 가져오기 전 내용으로 돌아간다.
    /// 모듈이 서버 경계 밖 파일을 밝혀도 보관·되돌리기 대상이 아니다. `keep_settings`면 지금 내용을 둔다
    #[tokio::test]
    async fn unlinking_an_imported_server_keeps_its_folder_and_restores_its_settings_files() {
        use axum::{extract::{Path, Query, State}, Json};
        let tmp = tempfile::tempdir().unwrap();
        let modules = tmp.path().join("modules");
        let dir = modules.join("game");
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(dir.join("module.toml"), "[module]\nschema_version = 1\nname = 'game'\nversion = '1.0.0'\nentry = 'lifecycle.py'\n").unwrap();
        let outside = tmp.path().join("outside.ini");
        std::fs::write(dir.join("lifecycle.py"), format!(
            "import json, os, sys\nc = json.load(sys.stdin)\nf = sys.argv[1]\n\
             if f == 'functions': print(json.dumps({{'success': True, 'functions': ['import_settings', 'config_files']}}))\n\
             elif f == 'config_files': print(json.dumps({{'success': True, 'files': [os.path.join(c['working_dir'], 'game.ini'), {:?}]}}))\n\
             else: print(json.dumps({{'success': True, 'settings': {{}}}}))\n",
            outside.to_string_lossy()
        )).unwrap();
        crate::sandbox::consent_test_component(&dir.join("lifecycle.py"));
        let mut sup = crate::supervisor::Supervisor::new_with_instances_dir(modules.to_str().unwrap(), tmp.path().join("instances").to_str().unwrap());
        for module in sup.module_loader.discover_modules().unwrap() { crate::sandbox::consent_test_component(std::path::Path::new(&module.entry_path())); }
        sup.initialize().await.unwrap();
        let state = crate::ipc::IPCServer::new(std::sync::Arc::new(tokio::sync::RwLock::new(sup)), "127.0.0.1:0", crate::daemon_log::DaemonLogBuffer::new());

        for keep_settings in [false, true] {
            let server = tmp.path().join(format!("기존 서버 {keep_settings}"));
            std::fs::create_dir_all(&server).unwrap();
            let ini = server.join("game.ini");
            std::fs::write(&ini, "rcon=on\n").unwrap();
            std::fs::write(&outside, "before").unwrap();
            let name = format!("g{keep_settings}");
            let created = crate::ipc::internal::into_internal(super::create_instance(State(state.clone()), Json(serde_json::json!({"name": name, "module_name":"game", "migration_source":server}))).await).await;
            assert_eq!(created.status.as_u16(), 201, "{}", created.body);
            let id = created.body["id"].as_str().unwrap().to_string();
            let instance_dir = state.supervisor.read().await.instance_store.instance_dir(&id);
            assert!(crate::instance::migration::saved(&instance_dir));

            // 사바쨩이 쓰는 동안 바뀐 파일 (시작 전 정책 등)
            std::fs::write(&ini, "rcon=off\n").unwrap();
            std::fs::write(&outside, "after").unwrap();
            let removed = crate::ipc::internal::into_internal(super::delete_instance(Path(id.clone()), State(state.clone()), Query(super::DeleteQuery { keep_settings })).await).await;
            assert_eq!(removed.status.as_u16(), 200, "{}", removed.body);
            assert_eq!(removed.body["linked_folder"], serde_json::json!(server.to_string_lossy()));
            assert!(server.is_dir() && !instance_dir.exists());
            let expected = if keep_settings { "rcon=off\n" } else { "rcon=on\n" };
            assert_eq!(std::fs::read_to_string(&ini).unwrap(), expected);
            assert_eq!(std::fs::read_to_string(&outside).unwrap(), "after", "a file outside the server is never kept or restored");
        }
    }

    /// 되돌리지 못한 원래 설정(그 사이 위 폴더가 사라진 자리 등)의 사본은 인스턴스 폴더와 함께 지우지 않고
    /// 데몬 기록 폴더에 옮겨 남기고, 그 자리를 응답에 싣는다
    #[tokio::test]
    async fn unlinking_keeps_the_copies_it_could_not_restore() {
        use axum::{extract::{Path, Query, State}, Json};
        let tmp = tempfile::tempdir().unwrap();
        let modules = tmp.path().join("modules");
        let dir = modules.join("game");
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(dir.join("module.toml"), "[module]\nschema_version = 1\nname = 'game'\nversion = '1.0.0'\nentry = 'lifecycle.py'\n").unwrap();
        std::fs::write(dir.join("lifecycle.py"),
            "import json, os, sys\nc = json.load(sys.stdin)\nf = sys.argv[1]\n\
             if f == 'functions': print(json.dumps({'success': True, 'functions': ['import_settings', 'config_files']}))\n\
             elif f == 'config_files': print(json.dumps({'success': True, 'files': [os.path.join(c['working_dir'], 'cfg', 'game.ini')]}))\n\
             else: print(json.dumps({'success': True, 'settings': {}}))\n",
        ).unwrap();
        crate::sandbox::consent_test_component(&dir.join("lifecycle.py"));
        let mut sup = crate::supervisor::Supervisor::new_with_instances_dir(modules.to_str().unwrap(), tmp.path().join("instances").to_str().unwrap());
        for module in sup.module_loader.discover_modules().unwrap() { crate::sandbox::consent_test_component(std::path::Path::new(&module.entry_path())); }
        sup.initialize().await.unwrap();
        let state = crate::ipc::IPCServer::new(std::sync::Arc::new(tokio::sync::RwLock::new(sup)), "127.0.0.1:0", crate::daemon_log::DaemonLogBuffer::new());
        let server = tmp.path().join("server");
        std::fs::create_dir_all(server.join("cfg")).unwrap();
        std::fs::write(server.join("cfg").join("game.ini"), "rcon=on\n").unwrap();
        let created = crate::ipc::internal::into_internal(super::create_instance(State(state.clone()), Json(serde_json::json!({"name": "g", "module_name":"game", "migration_source":server}))).await).await;
        assert_eq!(created.status.as_u16(), 201, "{}", created.body);
        let id = created.body["id"].as_str().unwrap().to_string();
        std::fs::remove_dir_all(server.join("cfg")).unwrap();

        let removed = crate::ipc::internal::into_internal(super::delete_instance(Path(id.clone()), State(state.clone()), Query(super::DeleteQuery { keep_settings: false })).await).await;
        assert_eq!(removed.status.as_u16(), 200, "{}", removed.body);
        assert_eq!(removed.body["settings_restore_failed"], 1);
        let kept = std::path::PathBuf::from(removed.body["settings_kept_at"].as_str().expect("where the copies were kept"));
        assert!(kept.starts_with(state.supervisor.read().await.instance_store.state_root()));
        assert!(kept.join("originals.json").is_file());
        assert_eq!(std::fs::read_to_string(kept.join("0")).unwrap(), "rcon=on\n");
    }

    /// 지운 서버의 디스코드 낱말은 남기지 않는다: 남으면 heartbeat가 없는 서버의 낱말을 계속 싣는다
    #[tokio::test]
    async fn deleting_an_instance_forgets_its_discord_words() {
        use axum::extract::{Path, Query, State};
        let tmp = tempfile::tempdir().unwrap();
        let modules = tmp.path().join("modules");
        std::fs::create_dir_all(&modules).unwrap();
        let sup = crate::supervisor::Supervisor::new_with_instances_dir(modules.to_str().unwrap(), tmp.path().join("instances").to_str().unwrap());
        let state = crate::ipc::IPCServer::new(std::sync::Arc::new(tokio::sync::RwLock::new(sup)), "127.0.0.1:0", crate::daemon_log::DaemonLogBuffer::new());
        let inst = crate::instance::ServerInstance::new("w", "game");
        let id = inst.id.clone();
        state.supervisor.write().await.instance_store.add(inst).unwrap();
        let words = crate::chat::words::InstanceWords { words: vec!["마크".into()], ..Default::default() };
        state.config_store.update_server_words(|t| ((), t.insert(id.clone(), words).is_none())).await.unwrap();

        let removed = crate::ipc::internal::into_internal(super::delete_instance(Path(id.clone()), State(state.clone()), Query(super::DeleteQuery { keep_settings: false })).await).await;
        assert_eq!(removed.status.as_u16(), 200, "{}", removed.body);
        assert!(!state.config_store.get_bot_config().await.server_words.contains_key(&id));
    }

    /// 인스턴스 폴더를 지울 수 없으면 아무것도 되돌리지 않는다: 설정만 원래대로 돌아가고 인스턴스가 남으면,
    /// 남은 인스턴스는 사바쨩 설정과 다른 게임 설정 파일로 돈다
    #[cfg(windows)]
    #[tokio::test]
    async fn a_delete_that_cannot_remove_the_folder_restores_nothing() {
        use axum::{extract::{Path, Query, State}, Json};
        let tmp = tempfile::tempdir().unwrap();
        let modules = tmp.path().join("modules");
        let dir = modules.join("game");
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(dir.join("module.toml"), "[module]\nschema_version = 1\nname = 'game'\nversion = '1.0.0'\nentry = 'lifecycle.py'\n").unwrap();
        std::fs::write(dir.join("lifecycle.py"),
            "import json, os, sys\nc = json.load(sys.stdin)\nf = sys.argv[1]\n\
             if f == 'functions': print(json.dumps({'success': True, 'functions': ['import_settings', 'config_files']}))\n\
             elif f == 'config_files': print(json.dumps({'success': True, 'files': [os.path.join(c['working_dir'], 'game.ini')]}))\n\
             else: print(json.dumps({'success': True, 'settings': {}}))\n",
        ).unwrap();
        crate::sandbox::consent_test_component(&dir.join("lifecycle.py"));
        let mut sup = crate::supervisor::Supervisor::new_with_instances_dir(modules.to_str().unwrap(), tmp.path().join("instances").to_str().unwrap());
        for module in sup.module_loader.discover_modules().unwrap() { crate::sandbox::consent_test_component(std::path::Path::new(&module.entry_path())); }
        sup.initialize().await.unwrap();
        let state = crate::ipc::IPCServer::new(std::sync::Arc::new(tokio::sync::RwLock::new(sup)), "127.0.0.1:0", crate::daemon_log::DaemonLogBuffer::new());
        let server = tmp.path().join("server");
        std::fs::create_dir_all(&server).unwrap();
        let ini = server.join("game.ini");
        std::fs::write(&ini, "rcon=on\n").unwrap();
        let created = crate::ipc::internal::into_internal(super::create_instance(State(state.clone()), Json(serde_json::json!({"name": "g", "module_name":"game", "migration_source":server}))).await).await;
        assert_eq!(created.status.as_u16(), 201, "{}", created.body);
        let id = created.body["id"].as_str().unwrap().to_string();
        assert!(crate::instance::migration::saved(&state.supervisor.read().await.instance_store.instance_dir(&id)));
        std::fs::write(&ini, "rcon=off\n").unwrap();
        // 다른 프로세스가 지우기를 막고 연 파일이 든 인스턴스 폴더 (서버 로그를 연 편집기 등)
        let instance_dir = state.supervisor.read().await.instance_store.instance_dir(&id);
        std::fs::write(instance_dir.join("held.log"), "x").unwrap();
        let _held = {
            use std::os::windows::fs::OpenOptionsExt;
            std::fs::OpenOptions::new().read(true).share_mode(0).open(instance_dir.join("held.log")).unwrap()
        };

        let removed = crate::ipc::internal::into_internal(super::delete_instance(Path(id.clone()), State(state.clone()), Query(super::DeleteQuery { keep_settings: false })).await).await;
        assert_eq!(removed.status.as_u16(), 500, "{}", removed.body);
        assert!(state.supervisor.read().await.instance_store.get(&id).is_some());
        assert_eq!(std::fs::read_to_string(&ini).unwrap(), "rcon=off\n", "nothing was restored for an instance that stays");
    }

    /// 지운 인스턴스의 런타임 상태(정지 표시·실행 세대·정지 문·접속자 수 등)는 데몬이 끝날 때까지 남지 않는다
    #[tokio::test]
    async fn deleting_an_instance_leaves_no_runtime_state_behind() {
        use axum::extract::{Path, Query, State};
        let tmp = tempfile::tempdir().unwrap();
        let mut sup = crate::supervisor::Supervisor::new_with_instances_dir(
            tmp.path().join("modules").to_str().unwrap(),
            tmp.path().join("instances").to_str().unwrap(),
        );
        let inst = crate::instance::ServerInstance::new("gone", "game");
        let id = inst.id.clone();
        sup.instance_store.add(inst).unwrap();
        let state = crate::ipc::IPCServer::new(std::sync::Arc::new(tokio::sync::RwLock::new(sup)), "127.0.0.1:0", crate::daemon_log::DaemonLogBuffer::new());
        let runtime = state.supervisor.read().await.runtime.clone();
        // 정지를 한 번 거치면 정지 문과 실행 세대가, 콘솔로 세기 시작하면 접속자 목록이 생긴다
        let _ = crate::service::lifecycle::stop(&state, &id, false).await;
        let roster = crate::supervisor::players::roster_start(&runtime, &id);
        crate::supervisor::players::roster_change(&runtime, &id, roster, "alice", true);
        assert_eq!(crate::supervisor::players::counted(&runtime, &id), Some(1));

        let removed = crate::ipc::internal::into_internal(super::delete_instance(Path(id.clone()), State(state.clone()), Query(Default::default())).await).await;
        assert_eq!(removed.status.as_u16(), 200, "{}", removed.body);
        assert_eq!(crate::supervisor::players::get(&runtime, &id), None, "deleting the instance drops its runtime state");
        assert!(crate::supervisor::players::present(&runtime, Some(&id)).is_empty());
    }
}
