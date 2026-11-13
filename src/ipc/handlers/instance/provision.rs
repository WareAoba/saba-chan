//! 새 인스턴스의 서버 설치(프로비저닝)와 그 진행 표시

use super::*;

/// 새 서버 설치 (만들기 뒤 백그라운드): 모듈 `install_server`를 부르고 진행·결과를 프로비저닝 추적에 알린다
pub(super) async fn install_new_server(job: InstallJob) {
    let InstallJob {
        mut install_guard,
        supervisor_arc,
        module_name_owned,
        tracker_done,
        name_done,
        id_done,
        text,
        chosen_version,
        install_method,
        leftovers_dir,
        made_folder,
        install_dir_str,
        accepted_agreements,
        install_settings,
        instance_data_dirs,
        provision_label,
        host_context,
    } = job;
    let leftovers: Option<InstallStaging>;
    // ── Phase 1: supervisor lock을 최소 시간만 잡고 모듈 경로 복사 ──
    let module_path = {
        let sup = supervisor_arc.read().await;
        match sup.module_loader.get_module(&module_name_owned) {
            Ok(m) => m.entry_path(),
            Err(e) => {
                drop(install_guard.take()); // 끝났다고 알리기 전에 놓는다 (알림을 본 삭제가 막히지 않게)
                tracker_done.update(&id_done, &name_done, crate::ipc::ProvisionProgress {
                    step: 0, total: 1,
                    label: "done".to_string(),
                    message: text.t("provision.install_failed", &[]),
                    done: true,
                    error: Some(text.t("provision.module_missing", &[("error", &e.to_string())])),
                    percent: Some(100),
                    steps: None, details: Default::default(),
                });
                return;
            }
        }
    };
    // supervisor read lock 해제됨 — 이후 플러그인 호출이
    // 모니터 태스크의 write lock을 차단하지 않음

    // ── Phase 2: 플러그인 호출 (lock 없이 실행) ──
    // lifecycle.py install_server 호출 (SteamCMD / download 모두 처리)
    // 설치한 버전 (고른 것, 고르지 않았으면 최신). 설치가 끝나면 인스턴스에 적는다
    let installed_version;
    let result = {
        // download 방식 모듈은 먼저 list_versions로 최신 버전 조회.
        // 인스턴스 설정(서버 종류 등)을 넘겨, 그 설정으로 받을 수 있는 최신을 고르게 한다
        let version = if let Some(version) = chosen_version {
            Some(version)
        } else if install_method == "download" {
            match crate::supervisor::Supervisor::latest_version(&module_path, &serde_json::json!(&install_settings)).await {
                Ok(version) => Some(version),
                Err(e) => {
                    drop(install_guard.take());
                    tracker_done.update(&id_done, &name_done, crate::ipc::ProvisionProgress {
                        step: 0, total: 1,
                        label: "done".to_string(),
                        message: text.t("provision.install_failed", &[]),
                        done: true,
                        error: Some(text.t("provision.no_latest_version", &[])),
                        percent: Some(100),
                        steps: None, details: Default::default(),
                    }.with_failure(crate::plugin::error_response(&e)));
                    return;
                }
            }
        } else {
            None // 익스텐션 방식은 version 불필요
        };
        installed_version = version.clone();

        // 만들 수 없으면 고른 폴더에 바로 설치한다 (실패해도 아무것도 지우지 않는다)
        leftovers = leftovers_dir.as_deref().and_then(|dir| {
            InstallStaging::new(dir, made_folder)
                .map_err(|e| tracing::warn!("Could not prepare the install folder in {}: {}", dir.display(), e))
                .ok()
        });
        let target_dir = leftovers.as_ref().map_or_else(|| install_dir_str.clone(), |l| l.staging.to_string_lossy().to_string());
        let mut config = serde_json::json!({
            "install_dir": &target_dir,
            "accepted_agreements": &accepted_agreements,
            "settings": &install_settings,
            crate::sandbox::INSTANCE_DATA_DIRS: &instance_data_dirs,
        });
        if let Some(ref ver) = version {
            config["version"] = serde_json::json!(ver);
        }

        let tracker_progress = tracker_done.clone();
        let (id_progress, name_progress) = (id_done.clone(), name_done.clone());
        let on_progress = move |prog: crate::plugin::ExtensionProgress| {
            let pct = prog.percent.unwrap_or(0);
            let msg = prog.message.clone().unwrap_or_default();
            tracker_progress.update(&id_progress, &name_progress, crate::ipc::ProvisionProgress {
                step: prog.step.unwrap_or(0),
                total: prog.total.unwrap_or(1),
                label: prog.label.unwrap_or_else(|| provision_label.clone()),
                message: msg,
                done: false,
                error: None,
                percent: Some(pct),
                steps: None, details: Default::default(),
            });
        };

        // 설치가 끝날 때까지 모듈·익스텐션 자동 교체를 막는다
        let _addons = crate::supervisor::Supervisor::hold_addons().await;
        crate::plugin::PluginCall::new(&module_path, "install_server", config)
            .in_instance(Some(host_context))
            .timeout(crate::plugin::INSTALL_TIMEOUT_SECS)
            .run_with_progress(Box::new(on_progress)).await
    };

    let result = match result {
        Ok(mut value) if value.get("success").and_then(Value::as_bool) == Some(true) => 'checked: {
            // 받은 서버를 고른 폴더로 옮긴다 (경로 확인은 옮긴 자리로)
            if let Some(left) = &leftovers {
                if let Err(e) = left.promote(&mut value) {
                    break 'checked Err(anyhow::anyhow!("Could not move the installed server into its folder: {}", e));
                }
            }
            let sup = supervisor_arc.read().await;
            match sup.instance_store.get(&id_done) {
                Some(inst) => {
                    let metadata = sup.module_loader.get_module(&inst.module_name).ok();
                    let boundary = crate::module_paths::Boundary::for_instance(inst, &sup.instance_store.instance_dir(&id_done), metadata.as_ref().map(|m| &m.metadata));
                    boundary.returned_paths(&mut value, &module_name_owned, "install_server").map(|()| value)
                }
                None => Err(anyhow::anyhow!("Instance disappeared during install")),
            }
        }
        other => other,
    };

    match result {
        Ok(val) => {
            let success = val.get("success").and_then(|s| s.as_bool()).unwrap_or(false);
            if success {
                // 설치 성공: executable_path, working_dir 업데이트
                let exe_path = val.get("executable_path")
                    .and_then(|v| v.as_str())
                    .map(|s| s.to_string());
                let install_path = val.get("install_path")
                    .and_then(|v| v.as_str())
                    .map(|s| s.to_string());

                let saved = {
                    let mut sup = supervisor_arc.write().await;
                    match sup.instance_store.get(&id_done).cloned() {
                        Some(inst) => {
                            let mut updated = inst;
                            if let Some(version) = &installed_version {
                                updated.record_installed_version(&install_method, version);
                            }
                            if let Some(ref exe) = exe_path {
                                if !exe.is_empty() {
                                    updated.executable_path = Some(exe.clone());
                                }
                            }
                            if let Some(ip) = install_path.as_ref().filter(|ip| !ip.trim().is_empty()) {
                                // 확인되지 않은 모듈은 인스턴스 폴더 안만 (서버 폴더는 다음 격리 실행에 쓰기를 받는다)
                                let root = sup.instance_store.instance_dir(&id_done);
                                if crate::sandbox::accepts_module_dir(std::path::Path::new(&module_path), ip, &root) {
                                    updated.working_dir = Some(ip.clone());
                                } else {
                                    tracing::warn!("Ignoring install_path outside the instance folder from unverified module: {}", ip);
                                }
                            }
                            sup.instance_store.update(&id_done, updated)
                        }
                        // 그 사이 지워졌다
                        None => Ok(()),
                    }
                };

                // 받은 파일을 치우는 동안은 작업 잠금을 쥔다 (그 사이 시작·복원이 같은 폴더를 쓰지 않게)
                if let (Err(_), Some(left)) = (&saved, &leftovers) { left.clear() }
                drop(install_guard.take()); // 끝났다고 알리기 전에 놓는다 (알림을 본 삭제가 막히지 않게)
                // 실행 경로를 저장하지 못했으면 설치했다고 알리지 않는다 (시작할 실행 파일을 모른다)
                if let Err(e) = saved {
                    tracing::warn!("Native provisioning for '{}' finished but the instance could not be saved: {}", name_done, e);
                    tracker_done.update(&id_done, &name_done, crate::ipc::ProvisionProgress {
                        step: 0, total: 1,
                        label: "done".to_string(),
                        message: text.t("provision.install_failed", &[]),
                        done: true,
                        error: Some(text.t("provision.save_failed", &[])),
                        percent: Some(100),
                        steps: None, details: Default::default(),
                    });
                    return;
                }
                tracing::info!("Native provisioning completed for '{}'", name_done);
                tracker_done.update(&id_done, &name_done, crate::ipc::ProvisionProgress {
                    step: 0, total: 1,
                    label: "done".to_string(),
                    message: text.t("provision.installed", &[]),
                    done: true,
                    error: None,
                    percent: Some(100),
                    steps: None, details: Default::default(),
                });

                // 5초 후 tracker 자동 정리
                let tracker_cleanup = tracker_done.clone();
                let id_cleanup = id_done.clone();
                tokio::spawn(async move {
                    tokio::time::sleep(std::time::Duration::from_secs(5)).await;
                    tracker_cleanup.remove(&id_cleanup);
                });
            } else {
                let err_msg = val.get("message")
                    .and_then(|v| v.as_str())
                    .map(str::to_string)
                    .unwrap_or_else(|| text.t("provision.install_failed", &[]));
                tracing::warn!("Native provisioning failed for '{}': {}", name_done, err_msg);
                if let Some(left) = &leftovers { left.clear() }
                drop(install_guard.take()); // 끝났다고 알리기 전에 놓는다 (알림을 본 삭제가 막히지 않게)
                tracker_done.update(&id_done, &name_done, crate::ipc::ProvisionProgress {
                    step: 0, total: 1,
                    label: "done".to_string(),
                    message: text.t("provision.install_failed", &[]),
                    done: true,
                    error: Some(err_msg),
                    percent: Some(100),
                    steps: None, details: Default::default(),
                }.with_failure(val));
            }
        }
        Err(e) => {
            tracing::warn!("Native provisioning error for '{}': {}", name_done, e);
            if let Some(left) = &leftovers { left.clear() }
            drop(install_guard.take()); // 끝났다고 알리기 전에 놓는다 (알림을 본 삭제가 막히지 않게)
            tracker_done.update(&id_done, &name_done, crate::ipc::ProvisionProgress {
                step: 0, total: 1,
                label: "done".to_string(),
                message: text.t("provision.install_failed", &[]),
                done: true,
                error: Some(text.t("provision.script_failed", &[])),
                percent: Some(100),
                steps: None, details: Default::default(),
            }.with_failure(crate::plugin::error_response(&e)));
        }
    }
}

/// `install_new_server`가 받는 값 (만들기 요청이 정한 것)
pub(super) struct InstallJob {
    pub(super) install_guard: Option<super::super::backup::InstanceLock>,
    pub(super) supervisor_arc: std::sync::Arc<tokio::sync::RwLock<crate::supervisor::Supervisor>>,
    pub(super) module_name_owned: String,
    pub(super) tracker_done: crate::ipc::ProvisionTracker,
    pub(super) name_done: String,
    pub(super) id_done: String,
    pub(super) text: crate::chat::i18n::Catalog,
    pub(super) chosen_version: Option<String>,
    pub(super) install_method: String,
    /// 고른 폴더에 설치할 때 그 폴더 (실패하면 설치가 만든 것만 치운다)
    pub(super) leftovers_dir: Option<std::path::PathBuf>,
    pub(super) made_folder: bool,
    pub(super) install_dir_str: String,
    pub(super) accepted_agreements: Vec<String>,
    pub(super) install_settings: std::collections::HashMap<String, Value>,
    pub(super) instance_data_dirs: Vec<String>,
    pub(super) provision_label: String,
    pub(super) host_context: crate::plugin::host::Context,
}

/// 고른 폴더에 처음 설치할 때 모듈이 받는 설치 폴더 (그 폴더 안). 점으로 시작해 탐색이 건너뛴다
pub(super) const INSTALL_STAGING: &str = ".saba-installing";

/// 고른 폴더(`working_dir`)에 처음 설치하기. 모듈은 그 안의 `INSTALL_STAGING`에 설치하고, 성공하면 데몬이 내용을 고른 폴더로 옮긴다.
/// 설치가 실패하면 그 설치가 만든 것(설치 폴더와 옮긴 항목)만 지운다: 설치하는 동안 사용자가 고른 폴더에 넣은 파일은 남는다.
/// 데몬이 만든 폴더였으면 비었을 때 그 폴더도 지운다
struct InstallStaging {
    dir: std::path::PathBuf,
    staging: std::path::PathBuf,
    made_folder: bool,
    /// 고른 폴더로 옮긴 항목 (옮긴 뒤 저장하지 못했을 때 지운다)
    moved: std::sync::Mutex<Vec<std::path::PathBuf>>,
}

impl InstallStaging {
    fn new(dir: &std::path::Path, made_folder: bool) -> std::io::Result<Self> {
        let staging = dir.join(INSTALL_STAGING);
        // 중간에 끝난 예전 설치가 남긴 것 (이 이름은 데몬 것이다)
        if std::fs::symlink_metadata(&staging).is_ok() {
            remove_entry(&staging)?;
        }
        std::fs::create_dir(&staging)?;
        Ok(Self { dir: dir.to_path_buf(), staging, made_folder, moved: Default::default() })
    }

    /// 설치 폴더의 내용을 고른 폴더로 옮기고, 모듈이 알려 준 경로(`install_path`·`executable_path`)를 옮긴 자리로 바꾼다.
    /// 고른 폴더에 같은 이름이 생겼으면(사용자가 넣었다) 덮지 않고 실패한다
    fn promote(&self, result: &mut Value) -> std::io::Result<()> {
        let canonical = std::fs::canonicalize(&self.staging).ok();
        for entry in std::fs::read_dir(&self.staging)? {
            let entry = entry?;
            let target = self.dir.join(entry.file_name());
            if std::fs::symlink_metadata(&target).is_ok() {
                return Err(std::io::Error::new(std::io::ErrorKind::AlreadyExists, format!("{} already exists", target.display())));
            }
            // Windows는 방금 쓴 파일을 백신 등이 잠깐 쥐고 있어 옮기기가 실패하기도 한다 (폴더 옮기기와 같이 다시 한다)
            let mut tries = 0;
            while let Err(e) = std::fs::rename(entry.path(), &target) {
                tries += 1;
                if tries == 3 {
                    return Err(e);
                }
                std::thread::sleep(std::time::Duration::from_millis(500));
            }
            self.moved.lock().unwrap_or_else(|e| e.into_inner()).push(target);
        }
        std::fs::remove_dir(&self.staging)?;
        for field in ["install_path", "executable_path"] {
            let Some(path) = result.get(field).and_then(Value::as_str).map(std::path::PathBuf::from) else { continue };
            let rel = std::iter::once(&self.staging).chain(canonical.as_ref()).find_map(|base| path.strip_prefix(base).ok());
            if let Some(rel) = rel {
                let moved = if rel.as_os_str().is_empty() { self.dir.clone() } else { self.dir.join(rel) };
                result[field] = json!(moved.to_string_lossy());
            }
        }
        Ok(())
    }

    fn clear(&self) {
        let moved = std::mem::take(&mut *self.moved.lock().unwrap_or_else(|e| e.into_inner()));
        for path in moved.iter().chain(std::iter::once(&self.staging)) {
            match remove_entry(path) {
                Err(e) if e.kind() != std::io::ErrorKind::NotFound => tracing::warn!("Could not remove {} left by a failed install: {}", path.display(), e),
                _ => {}
            }
        }
        if self.made_folder {
            let _ = std::fs::remove_dir(&self.dir); // 비었을 때만 지워진다
        }
    }
}

/// 항목 하나를 지운다. 링크(정션 포함)는 링크만 지운다 (가리키는 곳은 이 설치가 만든 것이 아니다)
fn remove_entry(path: &std::path::Path) -> std::io::Result<()> {
    match std::fs::symlink_metadata(path) {
        Ok(meta) if meta.is_dir() => std::fs::remove_dir_all(path),
        Ok(_) => std::fs::remove_file(path).or_else(|_| std::fs::remove_dir(path)),
        Err(e) => Err(e),
    }
}

/// DELETE /api/provision-progress/:name - 프로비저닝 상태 클리어 (에러 dismiss용)
/// 신규 생성 프로비저닝이 실패한 상태였으면 인스턴스도 롤백(삭제)한다.
/// 기존 인스턴스의 업데이트 실패는 진행 표시만 지운다.
pub async fn dismiss_provision_progress(
    Path(name): Path<String>,
    State(state): State<IPCServer>,
) -> impl IntoResponse {
    // 끝난 설치만 닫는다: 진행 중(또는 막 시작해 아직 표시가 없는) 설치의 신규 생성 표시를 지우면
    // 그 설치가 실패해도 롤백되지 않는다
    let Some((id, progress)) = provisioning_of(&state, &name).await.filter(|(_, p)| p.done) else {
        return (StatusCode::OK, Json(json!({ "success": true, "rolled_back": false }))).into_response();
    };
    let had_error = progress.error.is_some();
    let creating = state.provision_tracker.take_creating(&id);

    state.provision_tracker.remove(&id);

    // 신규 생성이 실패했을 때만 롤백 — 그 설치의 인스턴스를 ID로 지운다
    // 실제로 지웠을 때만 rolled_back (건너뛰거나 실패하면 인스턴스가 남아 있다)
    let mut rolled_back = false;
    if creating && had_error {
        // 다른 작업(업데이트·복원·삭제)이 그 폴더를 쓰는 중이면 지우지 않는다
        match super::super::backup::InstanceLock::take_for_writes(&id).await {
            None => tracing::warn!("Skipping rollback of '{}' (id={}): another task holds the instance", name, id),
            Some(_guard) => {
                let mut supervisor = state.supervisor.write().await;
                if supervisor.instance_store.get(&id).is_some() {
                    match supervisor.remove_instance(&id) {
                        Ok(_) => {
                            rolled_back = true;
                            tracing::info!("Provisioning rollback: removed instance '{}' (id={})", name, id)
                        }
                        Err(e) => tracing::warn!("Failed to rollback instance '{}': {}", name, e),
                    }
                }
            }
        }
    }

    (StatusCode::OK, Json(json!({ "success": true, "rolled_back": rolled_back }))).into_response()
}

/// 경로의 서버 이름을 인스턴스 id로 바꿔 그 진행 상태를 찾는다 (진행 상태는 id로 기억한다)
async fn provisioning_of(state: &IPCServer, name: &str) -> Option<(String, crate::ipc::ProvisionProgress)> {
    let id = state.supervisor.read().await.instance_store.find(name).map(|i| i.id.clone())?;
    let progress = state.provision_tracker.get(&id)?;
    Some((id, progress))
}

/// GET /api/provision-progress/:name - 프로비저닝 진행 상태 (서버 이름 기준)
pub async fn get_provision_progress(
    Path(name): Path<String>,
    State(state): State<IPCServer>,
) -> impl IntoResponse {
    if let Some((_, progress)) = provisioning_of(&state, &name).await {
        let mut resp = serde_json::to_value(progress).expect("provision progress contains JSON fields");
        resp["active"] = json!(true);
        (StatusCode::OK, Json(resp)).into_response()
    } else {
        (StatusCode::OK, Json(json!({
            "active": false,
        }))).into_response()
    }
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    #[tokio::test]
    async fn contract_paths_first_install_reports_an_outside_install_folder() {
        use axum::{extract::State, Json};
        let tmp = tempfile::tempdir().unwrap();
        let modules = tmp.path().join("modules");
        let dir = modules.join("game");
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(dir.join("module.toml"), "[module]\nschema_version = 1\nname = 'game'\nversion = '1.0.0'\nentry = 'lifecycle.py'\n\n[install]\nmethod = 'manual'\n").unwrap();
        let result = serde_json::json!({"success":true,"install_path":tmp.path().join("unrelated")});
        std::fs::write(dir.join("lifecycle.py"), format!("import sys\nprint('{{\"success\":true,\"functions\":[\"install_server\"]}}' if sys.argv[1] == 'functions' else {:?})\n", result.to_string())).unwrap();
        crate::sandbox::consent_test_component(&dir.join("lifecycle.py"));
        let mut sup = crate::supervisor::Supervisor::new_with_instances_dir(modules.to_str().unwrap(), tmp.path().join("instances").to_str().unwrap());
        for module in sup.module_loader.discover_modules().unwrap() { crate::sandbox::consent_test_component(std::path::Path::new(&module.entry_path())); }
        sup.initialize().await.unwrap();
        let state = crate::ipc::IPCServer::new(std::sync::Arc::new(tokio::sync::RwLock::new(sup)), "127.0.0.1:0", crate::daemon_log::DaemonLogBuffer::new());
        let response = crate::ipc::internal::into_internal(super::create_instance(State(state.clone()), Json(serde_json::json!({"name":"g", "module_name":"game"}))).await).await;
        assert_eq!(response.status.as_u16(), 201);
        let progress = provisioned(&state, "g").await.unwrap();
        assert_eq!(progress.details["error_code"], "module_path_outside");
        assert!(progress.error.is_some());
    }

    /// 설치는 요청이 끝난 뒤에도 따로 돌지만, 모듈 문구는 만들기를 요청한 화면의 언어로 온다 (진행 문구가 영어로 나오던 문제)
    #[tokio::test]
    async fn the_install_speaks_the_language_of_the_request() {
        use axum::{extract::State, Json};
        let tmp = tempfile::tempdir().unwrap();
        let modules = tmp.path().join("modules");
        let dir = modules.join("game");
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(dir.join("module.toml"), "[module]\nschema_version = 1\nname = 'game'\nversion = '1.0.0'\nentry = 'lifecycle.py'\n\n[install]\nmethod = 'manual'\n").unwrap();
        std::fs::write(
            dir.join("lifecycle.py"),
            "import json, os, sys\nif sys.argv[1] == 'functions':\n    print(json.dumps({'success': True, 'functions': ['install_server']}))\nelse:\n    print(json.dumps({'success': False, 'message': 'lang=' + os.environ.get('SABA_LANG', '')}))\n",
        )
        .unwrap();
        let mut sup = crate::supervisor::Supervisor::new_with_instances_dir(modules.to_str().unwrap(), tmp.path().join("instances").to_str().unwrap());
        for module in sup.module_loader.discover_modules().unwrap() { crate::sandbox::consent_test_component(std::path::Path::new(&module.entry_path())); }
        sup.initialize().await.unwrap();
        let state = crate::ipc::IPCServer::new(std::sync::Arc::new(tokio::sync::RwLock::new(sup)), "127.0.0.1:0", crate::daemon_log::DaemonLogBuffer::new());
        let origin = crate::audit::Origin { lang: Some("zh-TW".into()), ..crate::audit::Origin::new("gui") };
        let request = super::create_instance(State(state.clone()), Json(serde_json::json!({"name":"g", "module_name":"game"})));
        let response = crate::ipc::internal::into_internal(crate::audit::with_origin(origin, request).await).await;
        assert_eq!(response.status.as_u16(), 201);
        let progress = provisioned(&state, "g").await.unwrap();
        assert!(progress.error.as_deref().unwrap_or_default().contains("lang=zh-TW"), "{progress:?}");
    }

    /// 설치가 끝났다는 표시를 기다린다. 상한은 멈춘 설치를 알아채려는 것이라, 병렬 테스트로 느려진 PC·CI에서도 넉넉히 둔다
    async fn provisioned(state: &crate::ipc::IPCServer, name: &str) -> Option<crate::ipc::ProvisionProgress> {
        for _ in 0..2400 {
            if let Some((_, progress)) = super::provisioning_of(state, name).await.filter(|(_, p)| p.done) {
                return Some(progress);
            }
            tokio::time::sleep(std::time::Duration::from_millis(50)).await;
        }
        None
    }

    #[tokio::test]
    async fn a_crashed_install_script_keeps_the_short_error_and_structured_details() {
        use axum::extract::State;
        use axum::Json;
        let tmp = tempfile::tempdir().unwrap();
        let modules = tmp.path().join("modules");
        let dir = modules.join("game");
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(
            dir.join("module.toml"),
            "[module]\nschema_version = 1\nname = \"game\"\nversion = \"1.0.0\"\nentry = \"lifecycle.py\"\n\n[install]\nmethod = \"manual\"\n",
        )
        .unwrap();
        std::fs::write(dir.join("lifecycle.py"), "import json, sys\nif sys.argv[1] == 'functions':\n    print(json.dumps({'success': True, 'functions': ['install_server']}))\nelse:\n    sys.stderr.write('Traceback: /home/me/secret.py line 3\\n')\n    sys.exit(1)\n").unwrap();
        let mut sup = crate::supervisor::Supervisor::new_with_instances_dir(
            modules.to_str().unwrap(),
            tmp.path().join("instances").to_str().unwrap(),
        );
        for module in sup.module_loader.discover_modules().unwrap() { crate::sandbox::consent_test_component(std::path::Path::new(&module.entry_path())); }
        sup.initialize().await.unwrap();
        let state = crate::ipc::IPCServer::new(std::sync::Arc::new(tokio::sync::RwLock::new(sup)), "127.0.0.1:0", crate::daemon_log::DaemonLogBuffer::new());
        let made = crate::ipc::internal::into_internal(super::create_instance(State(state.clone()), Json(json!({ "name": "a", "module_name": "game" }))).await).await;
        assert!(made.status.is_success(), "{:?}", made.body);

        let progress = provisioned(&state, "a").await;
        let progress = progress.expect("provisioning finished");
        let error = progress.error.unwrap();
        assert_eq!(progress.details["error_code"], "module_crashed");
        assert_eq!(progress.details["function"], "install_server");
        assert_eq!(progress.details["stderr_tail"][0], "Traceback: /home/me/secret.py line 3");
        assert!(!error.contains("Traceback") && !error.contains("stderr"));
    }

    #[tokio::test]
    async fn an_install_whose_result_cannot_be_saved_is_reported_as_failed() {
        use axum::extract::State;
        use axum::Json;
        let tmp = tempfile::tempdir().unwrap();
        // 데몬의 모듈 폴더 아래에 둔다: 그 밖의 모듈은 Windows 격리 실행에서 인스턴스 폴더 쓰기를 받지 못한다
        let modules = crate::plugin::resolve_modules_dir();
        let module = tempfile::Builder::new().prefix("save-failed-").tempdir_in(&modules).unwrap();
        let dir = module.path();
        let name = dir.file_name().unwrap().to_string_lossy().into_owned();
        std::fs::write(
            dir.join("module.toml"),
            format!("[module]\nschema_version = 1\nname = \"{name}\"\nversion = \"1.0.0\"\nentry = \"lifecycle.py\"\n\n[install]\nmethod = \"manual\"\n"),
        )
        .unwrap();
        // 설치는 끝났다고 답하지만, 그 사이 인스턴스 기록 자리가 폴더가 되어 실행 경로를 저장하지 못한다.
        // 기록은 테스트가 망가뜨린다: Windows 격리 실행은 설치 폴더 밖(인스턴스 기록)에 쓰지 못한다
        std::fs::write(
            dir.join("lifecycle.py"),
            "import json, os, sys, time\nctx = json.load(sys.stdin)\nd = ctx['install_dir']\nos.makedirs(d, exist_ok=True)\n\
             open(os.path.join(d, 'started'), 'w').close()\n\
             for _ in range(300):\n    if os.path.exists(os.path.join(d, 'go')): break\n    time.sleep(0.1)\n\
             print(json.dumps({'success': True, 'executable_path': os.path.join(d, 'server.bin')}))\n",
        )
        .unwrap();
        // 격리할 수 없는 OS(맥)에서는 동의한 것으로 돌린다. 격리할 수 있으면 동의는 쓰이지 않는다
        crate::sandbox::consent_test_component(&dir.join("lifecycle.py"));
        let mut sup = crate::supervisor::Supervisor::new_with_instances_dir(
            modules.to_str().unwrap(),
            tmp.path().join("instances").to_str().unwrap(),
        );
        sup.initialize().await.unwrap();
        let state = crate::ipc::IPCServer::new(std::sync::Arc::new(tokio::sync::RwLock::new(sup)), "127.0.0.1:0", crate::daemon_log::DaemonLogBuffer::new());
        let made = crate::ipc::internal::into_internal(super::create_instance(State(state.clone()), Json(json!({ "name": "a", "module_name": name }))).await).await;
        assert!(made.status.is_success(), "{:?}", made.body);
        let id = made.body["id"].as_str().unwrap().to_string();
        let instance_dir = tmp.path().join("instances").join(&id);
        let install_dir = instance_dir.join("server");
        for _ in 0..300 {
            if install_dir.join("started").exists() {
                break;
            }
            tokio::time::sleep(std::time::Duration::from_millis(100)).await;
        }
        let record = instance_dir.join("instance.json");
        std::fs::remove_file(&record).unwrap();
        std::fs::create_dir(&record).unwrap();
        std::fs::write(install_dir.join("go"), b"").unwrap();

        let progress = provisioned(&state, "a").await.expect("provisioning finished");
        let text = crate::chat::i18n::Catalog::for_language(&state.config_store.ui_language().await);
        assert_eq!(progress.error.as_deref(), Some(text.t("provision.save_failed", &[]).as_str()));
    }

    /// 만들기 마법사용 모듈: download 방식이고, install_server는 맞는 버전을 받았을 때만 성공한다
    /// (`chosen` 폴더는 고른 버전 1.0이고 데몬이 만들어 둔 폴더, 그 밖은 최신 2.0). `failing` 폴더에는 받다 만 파일을 남기고 실패한다.
    /// 시험 모듈은 모듈 폴더 밖이라 격리 실행(Windows)에서 설치 폴더에 쓰지 못한다: 파일을 쓰지 않고 받은 값만 본다
    async fn wizard_state(tmp: &std::path::Path) -> crate::ipc::IPCServer {
        let modules = tmp.join("modules");
        let dir = modules.join("game");
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(
            dir.join("module.toml"),
            "[module]\nschema_version = 1\nname = \"game\"\nversion = \"1.0.0\"\nentry = \"lifecycle.py\"\n\n\
             [protocols]\nsupported = [\"rest\"]\n\n[config]\ndefault_port = 25565\n\n\
             [[settings.fields]]\nname = \"port\"\ntype = \"number\"\nport = true\n\n[install]\nmethod = \"download\"\n",
        )
        .unwrap();
        std::fs::write(
            dir.join("lifecycle.py"),
            "import json, os, sys\nconfig = json.load(sys.stdin)\n\
             d = config.get('install_dir') or ''\nfolder = os.path.basename(os.path.dirname(d) if os.path.basename(d) == '.saba-installing' else d)\n\
             if sys.argv[1] == 'list_versions':\n    \
             print(json.dumps({'success': True, 'latest': {'release': '2.0'}}))\n\
             elif sys.argv[1] == 'install_server' and folder == 'failing':\n    \
             try:\n        os.makedirs(os.path.join(config['install_dir'], 'libraries'))\n        \
             open(os.path.join(config['install_dir'], 'partial.jar'), 'w').write('x')\n    \
             except OSError:\n        pass\n    \
             print(json.dumps({'success': False, 'message': 'download failed'}))\n\
             elif sys.argv[1] == 'install_server':\n    \
             want = '1.0' if folder == 'chosen' else '2.0'\n    \
             ok = config.get('version') == want and (want == '2.0' or os.path.isdir(config['install_dir']))\n    \
             print(json.dumps({'success': ok, 'install_path': config['install_dir'], 'message': 'wrong version or folder'}))\n\
             else:\n    print(json.dumps({'success': True}))\n",
        )
        .unwrap();
        // 설치 방식이 없는 모듈 (버전을 고르지 않는다)
        let plain = modules.join("plain");
        std::fs::create_dir_all(&plain).unwrap();
        std::fs::write(plain.join("module.toml"), "[module]\nschema_version = 1\nname = \"plain\"\nversion = \"1.0.0\"\nentry = \"lifecycle.py\"\n").unwrap();
        std::fs::write(plain.join("lifecycle.py"), "").unwrap();
        let mut sup = crate::supervisor::Supervisor::new_with_instances_dir(modules.to_str().unwrap(), tmp.join("instances").to_str().unwrap());
        for module in sup.module_loader.discover_modules().unwrap() { crate::sandbox::consent_test_component(std::path::Path::new(&module.entry_path())); }
        sup.initialize().await.unwrap();
        crate::ipc::IPCServer::new(std::sync::Arc::new(tokio::sync::RwLock::new(sup)), "127.0.0.1:0", crate::daemon_log::DaemonLogBuffer::new())
    }

    /// 만들기 전에 이름·폴더·포트가 다른 서버와 겹치는지 묻는다. 아무것도 만들지 않는다
    #[tokio::test]
    async fn check_reports_name_folder_and_port_conflicts_without_creating() {
        use axum::extract::State;
        use axum::Json;
        let tmp = tempfile::tempdir().unwrap();
        let state = wizard_state(tmp.path()).await;
        let used = tmp.path().join("used");
        {
            let mut sup = state.supervisor.write().await;
            let mut other = crate::instance::ServerInstance::new("game", "game");
            other.port = Some(25565);
            other.working_dir = Some(used.to_string_lossy().into_owned());
            sup.instance_store.add(other).unwrap();
        }
        let check = |body: serde_json::Value| {
            let state = state.clone();
            async move { crate::ipc::internal::into_internal(super::check_new_instance(State(state), Json(body)).await).await }
        };

        let clash = check(json!({ "name": "game", "module_name": "game", "port": 25565, "working_dir": used.join("srv").to_string_lossy() })).await;
        assert!(clash.status.is_success(), "{}", clash.body);
        assert_eq!(clash.body["name"], json!({ "value": "game", "taken": true, "suggested": "game-2" }), "{}", clash.body);
        assert_eq!(clash.body["folder"]["in_use_by"], "game", "{}", clash.body);
        let port = clash.body["ports"].as_array().unwrap().iter().find(|p| p["field"] == "port").cloned().unwrap();
        assert_eq!((&port["port"], &port["in_use_by"]), (&json!(25565), &json!(["game"])), "{}", clash.body);

        // 추천 이름은 대소문자를 가리지 않는다 (GUI·CLI 이름 칸과 같다)
        state.supervisor.write().await.instance_store.add(crate::instance::ServerInstance::new("GAME-2", "plain")).unwrap();
        let named = check(json!({ "name": "x", "module_name": "game" })).await;
        assert_eq!(named.body["name"]["suggested"], "game-3", "{}", named.body);

        // 포트를 정하지 않으면 만들기가 고를 포트(다른 서버와 겹치지 않게 옮긴 값)를 알려 준다
        let free = check(json!({ "name": "game-3", "module_name": "game" })).await;
        assert_eq!(free.body["name"]["taken"], false, "{}", free.body);
        assert_eq!(free.body["folder"], json!({ "custom": false, "path": null, "in_use_by": null, "not_empty": false }), "{}", free.body);
        let port = free.body["ports"].as_array().unwrap().iter().find(|p| p["field"] == "port").cloned().unwrap();
        assert_eq!((&port["port"], &port["in_use_by"]), (&json!(25575), &json!([])), "{}", free.body);

        // 상대 경로와 파일이 든 폴더도 미리 알린다
        let relative = check(json!({ "name": "x", "module_name": "game", "working_dir": "srv" })).await;
        assert_eq!(relative.body["error_code"], "invalid_format", "{}", relative.body);
        std::fs::create_dir_all(tmp.path().join("full")).unwrap();
        std::fs::write(tmp.path().join("full").join("a.txt"), "x").unwrap();
        let full = check(json!({ "name": "x", "module_name": "game", "working_dir": tmp.path().join("full").to_string_lossy() })).await;
        assert_eq!(full.body["folder"]["not_empty"], true, "{}", full.body);

        let sup = state.supervisor.read().await;
        assert_eq!(sup.instance_store.list().len(), 2, "확인은 아무것도 만들지 않는다");
    }

    /// 마법사에서 고른 폴더·버전·실행 방식으로 만든다
    #[tokio::test]
    async fn create_installs_the_chosen_version_into_the_chosen_folder() {
        use axum::extract::State;
        use axum::Json;
        let tmp = tempfile::tempdir().unwrap();
        let state = wizard_state(tmp.path()).await;
        let create = |body: serde_json::Value| {
            let state = state.clone();
            async move { crate::ipc::internal::into_internal(super::create_instance(State(state), Json(body)).await).await }
        };
        let folder = tmp.path().join("my servers").join("chosen");
        let made = create(json!({ "name": "a", "module_name": "game", "working_dir": folder.to_string_lossy(), "version": "1.0", "managed_start": false })).await;
        assert!(made.status.is_success(), "{}", made.body);
        let progress = provisioned(&state, "a").await.expect("provisioning finished");
        assert_eq!(progress.error, None, "{:?}", progress.details);
        let id = made.body["id"].as_str().unwrap();
        let inst = state.supervisor.read().await.instance_store.get(id).cloned().unwrap();
        assert_eq!(inst.working_dir.as_deref(), Some(folder.to_string_lossy().as_ref()));
        assert_eq!(inst.module_settings["managed_start"], json!(false));
        // 설치한 버전은 그때 안다 (사용자가 골랐다): 따로 읽어 올 때까지 비워 두지 않는다
        assert_eq!(inst.server_version.as_deref(), Some("1.0"));

        // 버전을 고르지 않으면 최신이다
        let latest = create(json!({ "name": "b", "module_name": "game" })).await;
        assert!(latest.status.is_success(), "{}", latest.body);
        let progress = provisioned(&state, "b").await.expect("provisioning finished");
        assert_eq!(progress.error, None, "{:?}", progress.details);
        let id = latest.body["id"].as_str().unwrap();
        assert!(state.supervisor.read().await.instance_store.get(id).unwrap().server_version.is_some(), "the latest version it installed");

        // 파일이 든 폴더에는 새로 설치하지 않는다 (있는 서버는 가져오기로)
        let refused = create(json!({ "name": "c", "module_name": "game", "working_dir": folder.to_string_lossy() })).await;
        assert_eq!((refused.status.as_u16(), &refused.body["error_code"]), (409, &json!("working_dir_in_use")), "{}", refused.body);
        let full = tmp.path().join("full");
        std::fs::create_dir_all(&full).unwrap();
        std::fs::write(full.join("a.txt"), "x").unwrap();
        let refused = create(json!({ "name": "c", "module_name": "game", "working_dir": full.to_string_lossy() })).await;
        assert_eq!((refused.status.as_u16(), &refused.body["error_code"]), (409, &json!("folder_not_empty")), "{}", refused.body);
        // 버전은 내려받아 설치하는 모듈만 받는다
        let refused = create(json!({ "name": "c", "module_name": "plain", "version": "1.0" })).await;
        assert_eq!((refused.status.as_u16(), &refused.body["error_code"]), (400, &json!("invalid_format")), "{}", refused.body);
        for (key, bad) in [("working_dir", json!("relative")), ("version", json!("../1.0")), ("version", json!(3)), ("managed_start", json!("no"))] {
            let refused = create(json!({ "name": "c", "module_name": "game", key: bad })).await;
            assert_eq!((refused.status.as_u16(), &refused.body["error_code"]), (400, &json!("invalid_format")), "{key} {}", refused.body);
        }
        assert_eq!(state.supervisor.read().await.instance_store.list().len(), 2);
    }

    /// 고른 폴더에 설치하다 실패하면 그 설치가 만든 것만 치운다: 데몬이 만든 폴더는 지우고, 원래 있던 빈 폴더는 남긴다
    #[tokio::test]
    async fn a_failed_install_into_a_chosen_folder_clears_only_what_it_made() {
        use axum::extract::State;
        use axum::Json;
        let tmp = tempfile::tempdir().unwrap();
        let state = wizard_state(tmp.path()).await;
        let create = |body: serde_json::Value| {
            let state = state.clone();
            async move { crate::ipc::internal::into_internal(super::create_instance(State(state), Json(body)).await).await }
        };
        let made = tmp.path().join("new").join("failing");
        let res = create(json!({ "name": "a", "module_name": "game", "working_dir": made.to_string_lossy() })).await;
        assert!(res.status.is_success(), "{}", res.body);
        assert!(provisioned(&state, "a").await.expect("provisioning finished").error.is_some());
        assert!(!made.exists(), "데몬이 만든 폴더는 받다 만 파일과 함께 지운다");
        assert!(tmp.path().join("new").is_dir(), "그 위 폴더는 그대로");

        let existing = tmp.path().join("kept").join("failing");
        std::fs::create_dir_all(&existing).unwrap();
        let res = create(json!({ "name": "b", "module_name": "game", "working_dir": existing.to_string_lossy() })).await;
        assert!(res.status.is_success(), "{}", res.body);
        assert!(provisioned(&state, "b").await.expect("provisioning finished").error.is_some());
        assert!(existing.is_dir(), "원래 있던 폴더는 남긴다");
        assert_eq!(std::fs::read_dir(&existing).unwrap().count(), 0, "그 설치가 만든 것만 지운다");
    }

    /// 설치하는 동안 사용자가 고른 폴더에 넣은 파일은 설치가 실패해도 남는다 (설치가 만든 것만 지운다)
    #[test]
    fn a_failed_install_keeps_files_the_user_added_meanwhile() {
        let tmp = tempfile::tempdir().unwrap();
        let dir = tmp.path().join("server");
        std::fs::create_dir_all(&dir).unwrap();
        let staging = super::InstallStaging::new(&dir, false).unwrap();
        std::fs::create_dir_all(staging.staging.join("libraries")).unwrap();
        std::fs::write(staging.staging.join("partial.jar"), "x").unwrap();
        std::fs::write(dir.join("my-world.zip"), "mine").unwrap();
        staging.clear();
        let left: Vec<_> = std::fs::read_dir(&dir).unwrap().map(|e| e.unwrap().file_name()).collect();
        assert_eq!(left, vec![std::ffi::OsString::from("my-world.zip")]);

        // 옮긴 뒤 저장하지 못해도 옮긴 것만 지운다
        let staging = super::InstallStaging::new(&dir, false).unwrap();
        std::fs::write(staging.staging.join("server.jar"), "x").unwrap();
        let mut result = json!({ "install_path": staging.staging.to_string_lossy(), "executable_path": staging.staging.join("server.jar").to_string_lossy() });
        staging.promote(&mut result).unwrap();
        assert_eq!(result["install_path"], json!(dir.to_string_lossy()));
        assert_eq!(result["executable_path"], json!(dir.join("server.jar").to_string_lossy()));
        assert!(dir.join("server.jar").is_file() && !staging.staging.exists());
        staging.clear();
        let left: Vec<_> = std::fs::read_dir(&dir).unwrap().map(|e| e.unwrap().file_name()).collect();
        assert_eq!(left, vec![std::ffi::OsString::from("my-world.zip")]);

        // 사용자가 같은 이름을 넣었으면 덮지 않는다
        let staging = super::InstallStaging::new(&dir, false).unwrap();
        std::fs::write(staging.staging.join("my-world.zip"), "installer").unwrap();
        assert!(staging.promote(&mut json!({})).is_err());
        staging.clear();
        assert_eq!(std::fs::read_to_string(dir.join("my-world.zip")).unwrap(), "mine");
    }

    /// 새 인스턴스의 버전을 고를 때 인스턴스 설정(서버 종류 등)을 넘긴다: 그 종류로 받을 수 있는 최신이 다르다
    #[tokio::test]
    async fn the_version_for_a_new_instance_is_chosen_with_its_settings() {
        use axum::extract::State;
        use axum::Json;
        let tmp = tempfile::tempdir().unwrap();
        let modules = tmp.path().join("modules");
        let dir = modules.join("game");
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(
            dir.join("module.toml"),
            "[module]\nschema_version = 1\nname = \"game\"\nversion = \"1.0.0\"\nentry = \"lifecycle.py\"\n\n\
             [[settings.fields]]\nname = \"server_type\"\ntype = \"text\"\ndefault = \"vanilla\"\n\n[install]\nmethod = \"download\"\n",
        )
        .unwrap();
        // 설정을 받았을 때만 버전을 알려 준다. 못 받으면 "최신 버전을 찾지 못함"으로 끝난다.
        // Windows에서는 격리 실행이라 모듈이 임시 폴더 밖에 파일을 쓸 수 없어 설치 결과로 확인한다
        std::fs::write(
            dir.join("lifecycle.py"),
            "import json, sys\nconfig = json.load(sys.stdin)\nif sys.argv[1] == 'list_versions':\n    \
             latest = {'release': '1.0'} if config.get('settings', {}).get('server_type') == 'paper' else {}\n    \
             print(json.dumps({'success': True, 'latest': latest}))\n\
             else:\n    print(json.dumps({'success': True}))\n",
        )
        .unwrap();
        let mut sup = crate::supervisor::Supervisor::new_with_instances_dir(
            modules.to_str().unwrap(),
            tmp.path().join("instances").to_str().unwrap(),
        );
        for module in sup.module_loader.discover_modules().unwrap() { crate::sandbox::consent_test_component(std::path::Path::new(&module.entry_path())); }
        sup.initialize().await.unwrap();
        let state = crate::ipc::IPCServer::new(std::sync::Arc::new(tokio::sync::RwLock::new(sup)), "127.0.0.1:0", crate::daemon_log::DaemonLogBuffer::new());
        crate::ipc::internal::into_internal(
            super::create_instance(State(state.clone()), Json(json!({ "name": "a", "module_name": "game", "module_settings": { "server_type": "paper" } }))).await,
        )
        .await;
        // Windows CI는 첫 격리 호출에서 venv를 만들어 오래 걸린다
        let progress = provisioned(&state, "a").await;
        let progress = progress.expect("provisioning finished");
        assert_eq!(progress.error, None, "list_versions did not get the instance settings");
    }

    #[tokio::test]
    async fn the_first_install_may_write_the_module_data_folder() {
        use axum::extract::State;
        use axum::Json;
        let tmp = tempfile::tempdir().unwrap();
        let modules = tmp.path().join("modules");
        let dir = modules.join("game");
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(
            dir.join("module.toml"),
            "[module]\nschema_version = 1\nname = \"game\"\nversion = \"1.0.0\"\nentry = \"lifecycle.py\"\n\n\
             [[settings.fields]]\nname = \"cache_dir\"\ntype = \"text\"\ndefault = \"{instance_dir}/data\"\n\n[install]\nmethod = \"manual\"\n",
        )
        .unwrap();
        // 설치 중에 데이터 폴더에 첫 설정 파일을 쓰는 모듈 (좀보이드 첫 INI). 격리 실행은 그 폴더를 넘겨받아야 쓸 수 있다
        std::fs::write(
            dir.join("lifecycle.py"),
            "import json, os, sys\nconfig = json.load(sys.stdin)\nif sys.argv[1] == 'functions':\n    print(json.dumps({'success': True, 'functions': ['install_server']})); sys.exit(0)\nnorm = lambda p: os.path.normcase(os.path.normpath(p))\n\
             ok = norm(config['settings']['cache_dir']) in [norm(d) for d in config.get('instance_data_dirs', [])]\n\
             print(json.dumps({'success': ok, 'message': 'no data folder'}))\n",
        )
        .unwrap();
        let mut sup = crate::supervisor::Supervisor::new_with_instances_dir(
            modules.to_str().unwrap(),
            tmp.path().join("instances").to_str().unwrap(),
        );
        for module in sup.module_loader.discover_modules().unwrap() { crate::sandbox::consent_test_component(std::path::Path::new(&module.entry_path())); }
        sup.initialize().await.unwrap();
        let state = crate::ipc::IPCServer::new(std::sync::Arc::new(tokio::sync::RwLock::new(sup)), "127.0.0.1:0", crate::daemon_log::DaemonLogBuffer::new());
        crate::ipc::internal::into_internal(super::create_instance(State(state.clone()), Json(json!({ "name": "a", "module_name": "game" }))).await).await;
        // Windows CI는 첫 격리 호출에서 venv를 만들어 오래 걸린다
        let progress = provisioned(&state, "a").await;
        let progress = progress.expect("provisioning finished");
        assert_eq!(progress.error, None, "install_server did not get the instance data folder");
    }

    #[tokio::test]
    async fn the_first_install_holds_the_instance_lock() {
        use axum::extract::State;
        use axum::Json;
        let tmp = tempfile::tempdir().unwrap();
        let modules = tmp.path().join("modules");
        let dir = modules.join("game");
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(
            dir.join("module.toml"),
            "[module]\nschema_version = 1\nname = \"game\"\nversion = \"1.0.0\"\nentry = \"lifecycle.py\"\n\n[install]\nmethod = \"download\"\n",
        )
        .unwrap();
        // 설치만 느리다: 버전 조회 같은 다른 호출까지 기다리면 부하가 큰 Windows에서 끝나는 시각이 크게 밀린다
        std::fs::write(
            dir.join("lifecycle.py"),
            "import json, sys, time\nif sys.argv[1] == 'install_server':\n    time.sleep(2)\nprint(json.dumps({'success': True, 'latest': {'release': '1.0'}}))\n",
        )
        .unwrap();
        let mut sup = crate::supervisor::Supervisor::new_with_instances_dir(
            modules.to_str().unwrap(),
            tmp.path().join("instances").to_str().unwrap(),
        );
        for module in sup.module_loader.discover_modules().unwrap() { crate::sandbox::consent_test_component(std::path::Path::new(&module.entry_path())); }
        sup.initialize().await.unwrap();
        let state = crate::ipc::IPCServer::new(std::sync::Arc::new(tokio::sync::RwLock::new(sup)), "127.0.0.1:0", crate::daemon_log::DaemonLogBuffer::new());
        let created = crate::ipc::internal::into_internal(super::create_instance(State(state.clone()), Json(json!({ "name": "a", "module_name": "game" }))).await).await;
        let id = created.body["id"].as_str().unwrap().to_string();
        assert!(super::super::super::backup::InstanceLock::take(&id).is_none(), "the install holds the lock");
        // 시간이 아니라 설치가 끝났다는 표시를 기다린다. 잠금은 그 표시보다 먼저 풀린다
        let done = provisioned(&state, "a").await;
        let done = done.expect("the install never finished");
        assert!(done.error.is_none(), "{:?}", done.error);
        assert!(super::super::super::backup::InstanceLock::take(&id).is_some(), "the lock is released once the install is done");
    }

    /// 설치 실패를 dismiss하면 그 설치의 인스턴스만 지운다: 그 사이 이름이 바뀌고 같은 이름의 다른 인스턴스가 생겨도
    #[tokio::test]
    async fn dismissing_a_failed_install_never_deletes_another_instance() {
        use axum::extract::{Path, State};
        use axum::Json;
        let tmp = tempfile::tempdir().unwrap();
        let modules = tmp.path().join("modules");
        for (name, install, script) in [
            ("game", "\n[install]\nmethod = \"download\"\n", "import json, sys, time\ntime.sleep(1)\nprint(json.dumps({'success': False, 'message': 'boom'}))\n"),
            ("plain", "", ""),
        ] {
            let dir = modules.join(name);
            std::fs::create_dir_all(&dir).unwrap();
            std::fs::write(
                dir.join("module.toml"),
                format!("[module]\nschema_version = 1\nname = \"{name}\"\nversion = \"1.0.0\"\nentry = \"lifecycle.py\"\n{install}"),
            )
            .unwrap();
            std::fs::write(dir.join("lifecycle.py"), script).unwrap();
        }
        let mut sup = crate::supervisor::Supervisor::new_with_instances_dir(
            modules.to_str().unwrap(),
            tmp.path().join("instances").to_str().unwrap(),
        );
        for module in sup.module_loader.discover_modules().unwrap() { crate::sandbox::consent_test_component(std::path::Path::new(&module.entry_path())); }
        sup.initialize().await.unwrap();
        let state = crate::ipc::IPCServer::new(std::sync::Arc::new(tokio::sync::RwLock::new(sup)), "127.0.0.1:0", crate::daemon_log::DaemonLogBuffer::new());
        let a = crate::ipc::internal::into_internal(super::create_instance(State(state.clone()), Json(json!({ "name": "a", "module_name": "game" }))).await).await;
        let a = a.body["id"].as_str().unwrap().to_string();

        // 설치하는 동안에는 이름을 바꿀 수 없다
        let renamed = crate::ipc::internal::into_internal(super::rename_instance(State(state.clone()), Path(a.clone()), Json(json!({ "name": "b" }))).await).await;
        assert_eq!(renamed.status, 409, "{}", renamed.body);
        assert_eq!(renamed.body["error_code"], "instance_busy");

        // 처음 부르는 선택 함수 앞에는 함수 목록을 묻는 실행이 하나 더 있다: 느린 Windows CI에서도 넉넉히 기다린다
        provisioned(&state, "a").await;
        assert!(state.provision_tracker.get(&a).is_some_and(|p| p.error.is_some()), "the install should have failed");

        // 실패 뒤 dismiss 전에 이름을 바꾸고, 그 이름으로 다른 인스턴스를 만든다
        let renamed = crate::ipc::internal::into_internal(super::rename_instance(State(state.clone()), Path(a.clone()), Json(json!({ "name": "b" }))).await).await;
        assert_eq!(renamed.status, 200, "{}", renamed.body);
        let c = crate::ipc::internal::into_internal(super::create_instance(State(state.clone()), Json(json!({ "name": "a", "module_name": "plain" }))).await).await;
        let c = c.body["id"].as_str().unwrap().to_string();

        // 실패 표시는 인스턴스를 따라간다: 같은 이름의 새 인스턴스가 설치 중으로 보이거나 그 배너로 A가 지워지지 않게
        assert!(state.provision_tracker.get(&c).is_none(), "the new instance with the old name is not provisioning");
        let other = crate::ipc::internal::into_internal(super::dismiss_provision_progress(Path("a".to_string()), State(state.clone())).await).await;
        assert_eq!(other.body["rolled_back"], false);
        assert!(state.supervisor.read().await.instance_store.get(&a).is_some(), "dismissing another name's banner keeps A");
        let dismissed = crate::ipc::internal::into_internal(super::dismiss_provision_progress(Path("b".to_string()), State(state.clone())).await).await;
        assert_eq!(dismissed.body["rolled_back"], true);
        let sup = state.supervisor.read().await;
        assert!(sup.instance_store.get(&c).is_some(), "another instance with the old name must survive");
        assert!(sup.instance_store.get(&a).is_none(), "the failed install is rolled back");
    }
}
