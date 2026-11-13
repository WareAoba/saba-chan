//! 게임 서버 업데이트 확인과 적용 (켜진 서버 예고·자동 업데이트는 `server_update.rs`)

use super::*;

/// GET /api/instance/:id/check-update - 게임 서버 업데이트 확인
///
/// 설치 방식 익스텐션(`install_method_<방식>`)으로 설치된 인스턴스에 대해
/// server.check_update hook을 디스패치하여 buildid 비교 결과를 반환합니다.
pub async fn check_server_update(
    Path(id): Path<String>,
    State(state): State<IPCServer>,
) -> impl IntoResponse {
    match server_update_status(&state, &id).await {
        Some(status) => (StatusCode::OK, Json(status)).into_response(),
        None => (
            StatusCode::NOT_FOUND,
            Json(json!({
                "error": format!("Instance not found: {}", id),
                "error_code": "instance_not_found",
            })),
        )
            .into_response(),
    }
}

/// `server.check_update` hook이 공통 문맥(`extension::hook_context`) 위에 더 받는 값. 설치 값은 모듈 `[install]` 그대로(`install`)이고 인스턴스에 복사해 두지 않는다:
/// 모듈이 앱 ID를 바꾸면 기존 서버의 확인도 따라간다. `branch`는 사용자가 고른 브랜치(`<방식>_beta`), 없으면 모듈 `[install] beta`.
/// `app_id`는 `install.app_id`와 같은 값이다 (0.1.14-beta.1까지의 익스텐션이 읽는다)
fn check_update_context(
    instance: &crate::instance::ServerInstance,
    instance_dir: &std::path::Path,
    install: Option<&crate::supervisor::module_loader::ModuleInstallConfig>,
) -> Value {
    let branch = install.and_then(|cfg| instance.install_branch(&cfg.method).or(cfg.beta.as_deref()));
    json!({
        "install_dir": install_dir_of(instance, instance_dir, install).to_string_lossy(),
        "install": install,
        "app_id": install.and_then(|cfg| cfg.app_id),
        "branch": branch,
    })
}

/// 게임 서버 업데이트 확인 결과 (`check-update`의 답). 인스턴스가 없으면 None.
/// 서버를 켤 때도 뒤에서 불러 새 빌드가 있으면 알린다 (`server.rs` `announce_updates`)
pub async fn server_update_status(state: &IPCServer, id: &str) -> Option<Value> {
    let supervisor = state.supervisor.read().await;
    let instance = supervisor.instance_store.get(id)?.clone();

    // 모듈에서 install config 가져오기 (install_subdir, app_id 등)
    let module = supervisor.module_loader.get_module(&instance.module_name).ok();
    let install_config = module.as_ref().and_then(|m| m.metadata.install.clone());

    // 다운로드·수동 설치 서버는 사바쨩이 업데이트를 확인하지 않는다 (적용 `begin_server_update`과 같은 조건).
    // 모듈을 읽지 못했으면 설치 방식을 모르므로 아래 hook에 맡긴다
    if module.is_some() && !install_config.as_ref().is_some_and(|cfg| installed_by_extension(&instance, &cfg.method)) {
        return Some(json!({
            "update_available": false,
            "reason": "update_unsupported",
        }));
    }

    // extension_data 가 비어 있으면 hook 이 동작하지 않으므로 조기 반환
    if instance.extension_data.is_empty() {
        return Some(json!({
            "update_available": false,
            "reason": "no_extension_data",
        }));
    }

    let instance_dir = supervisor.instance_store.instance_dir(id);
    let mut ctx = crate::extension::hook_context(&instance, &instance_dir, module.as_ref().map(|m| &m.metadata));
    if let (Value::Object(ctx), Value::Object(extra)) = (&mut ctx, check_update_context(&instance, &instance_dir, install_config.as_ref())) {
        ctx.extend(extra);
    }
    // supervisor lock 해제 (hook 실행 중 필요 없음)
    drop(supervisor);

    let results = crate::plugin::with_instance(Some(state.host_context(id)), crate::extension::run_hook(&state.extension_manager, "server.check_update", ctx)).await;
    let status = update_status_from_hooks(results);
    // 누가 확인했든(데몬의 주기 확인·서버 시작·앱·디스코드) 결과를 기억해 목록에 싣는다
    super::super::server_update::remember_check(id, &status);
    record_build(state, id, &status).await;
    Some(status)
}

/// 업데이트 확인이 알려 준 설치 빌드(`local_buildid`, 받은 때 `local_build_time`)를 인스턴스에 적어 둔다.
/// 화면은 이 값을 보이고 게임 서버에 버전을 묻지 않는다. 확인이 설치 빌드를 읽지 못했으면 적어 둔 것을 그대로 둔다
pub(crate) async fn record_build(state: &IPCServer, id: &str, status: &Value) {
    let build_id = match status.get("local_buildid") {
        Some(Value::String(b)) if !b.is_empty() => b.clone(),
        Some(v @ Value::Number(_)) => v.to_string(),
        _ => return,
    };
    let build = crate::instance::ServerBuild { id: build_id, time: status.get("local_build_time").and_then(Value::as_i64) };
    let mut supervisor = state.supervisor.write().await;
    let Some(mut instance) = supervisor.instance_store.get(id).cloned() else { return };
    if instance.server_build.as_ref() == Some(&build) {
        return;
    }
    instance.server_build = Some(build);
    if let Err(e) = supervisor.instance_store.update(id, instance) {
        tracing::warn!("Failed to record the installed build of '{}': {}", id, e);
    }
}

/// `server.check_update` hook 결과를 확인 답으로: 첫 성공 결과, 모두 실패했으면 확인 실패(`check_failed`),
/// hook이 없거나 모두 condition 불일치면 `no_hook_handled`
fn update_status_from_hooks(results: Vec<(String, anyhow::Result<Value>)>) -> Value {
    // 익스텐션의 정식 실패(`{success:false}`, 예외·격리 실행 실패도 이 모양으로 온다)는 답이 아니라 실패다
    let refused = |v: &Value| v.get("success").and_then(Value::as_bool) == Some(false);
    if let Some(val) = results.iter().find_map(|(_, result)| result.as_ref().ok().filter(|v| !refused(v))) {
        return val.clone();
    }
    if let Some(val) = results.iter().find_map(|(_, result)| result.as_ref().ok()) {
        let mut failure = val.clone();
        failure["update_available"] = json!(false);
        failure["reason"] = json!("check_failed");
        if failure.get("error").and_then(Value::as_str).is_none() {
            failure["error"] = json!("update check failed");
        }
        return failure;
    }
    // 모두 실패했으면 그 오류를 구조화해 돌려준다. 화면이 "업데이트 없음"과 구분하도록 이유를 붙인다
    if let Some(error) = results.iter().find_map(|(_, result)| result.as_ref().err()) {
        let mut failure = crate::plugin::error_response(error);
        failure["update_available"] = json!(false);
        failure["reason"] = json!("check_failed");
        return failure;
    }
    json!({
        "update_available": false,
        "reason": "no_hook_handled",
    })
}

/// 설치 방식 익스텐션(`install_method_<방식>`)으로 설치한 인스턴스인가. 이런 서버만 사바쨩이 업데이트를 확인·적용한다
fn installed_by_extension(instance: &crate::instance::ServerInstance, method: &str) -> bool {
    !crate::instance::BUILTIN_INSTALL_METHODS.contains(&method) && instance.ext_enabled(&crate::instance::install_method_key(method))
}

/// POST /api/instance/:id/apply-update - 게임 서버 업데이트 적용
///
/// SteamCMD install (== update)을 재실행하여 서버를 최신 빌드로 업데이트합니다.
/// 서버가 켜져 있으면 본문 `warn_minutes`(기본 5)분 동안 게임 안에 예고한 뒤 정지 → 업데이트 → 다시 켜기를 뒤에서 하고
/// `{success, scheduled: true, warn_minutes, players}`로 바로 답합니다 (`server_update.rs`).
pub async fn apply_server_update(
    Path(id): Path<String>,
    State(state): State<IPCServer>,
    super::super::LooseJson(body): super::super::LooseJson<super::super::server_update::ApplyBody>,
) -> impl IntoResponse {
    // 켜져 있으면 게임 안에 예고(기본 5분) → 저장하고 정지 → 업데이트 → 다시 켜기를 뒤에서 건다
    if state.supervisor.read().await.is_running(&id).await {
        return super::super::server_update::apply_to_running(&state, &id, &body.map(|b| b.0).unwrap_or_default()).await;
    }
    if super::super::server_update::is_pending(&id) {
        return (StatusCode::CONFLICT, Json(json!({ "success": false, "error": "An update of this server is already under way", "error_code": "update_pending" }))).into_response();
    }
    match begin_server_update(&state, &id).await {
        Ok(_) => (
            StatusCode::OK,
            Json(json!({
                "success": true,
                "id": id,
                "provisioning": true,
            })),
        )
            .into_response(),
        Err(failure) => failure.into_response(),
    }
}

/// 게임 서버 업데이트가 실패했다 `instance.update_failed {instanceId, instance, error}` (어디서 시켰든: 앱·예약·디스코드·자동)
pub(crate) async fn emit_update_failed(state: &IPCServer, id: &str, error: &str) {
    let name = state.supervisor.read().await.instance_store.get(id).map(|i| i.name.clone()).unwrap_or_default();
    crate::events::emit("instance.update_failed", json!({ "instanceId": id, "instance": name, "error": error }));
}

/// 이 인스턴스를 `apply-update`로 업데이트할 수 있는가: 있고, 모듈에 설치 설정이 있고, 설치 방식 익스텐션이 설치했다.
/// 켜진 서버의 업데이트는 예고·정지 전에 이것부터 본다 (업데이트할 수 없는 서버를 끄지 않게)
pub(crate) fn updatable(
    supervisor: &crate::supervisor::Supervisor,
    id: &str,
) -> Result<(crate::instance::ServerInstance, crate::supervisor::module_loader::ModuleInstallConfig), super::super::Failure> {
    let instance = match supervisor.instance_store.get(id) {
        Some(inst) => inst.clone(),
        None => {
            return Err(super::super::Failure::new(
                StatusCode::NOT_FOUND,
                json!({
                    "error": format!("Instance not found: {}", id),
                    "error_code": "instance_not_found",
                }),
            ));
        }
    };

    // 모듈에서 install config 가져오기
    let module_info = supervisor
        .module_loader
        .get_module(&instance.module_name)
        .ok()
        .map(|m| (m.metadata.install.clone(), m.path.clone()));

    let (install_config, _module_path) = match module_info {
        Some((Some(cfg), path)) => (cfg, path),
        _ => {
            return Err(super::super::Failure::new(
                StatusCode::BAD_REQUEST,
                json!({
                    "error": "Module install configuration not found.",
                    "error_code": "no_install_config",
                }),
            ));
        }
    };

    // 익스텐션이 설치한 인스턴스만 이 경로로 업데이트한다 (설치 방식 슬롯)
    let method = install_config.method.clone();
    if !installed_by_extension(&instance, &method) {
        return Err(super::super::Failure::new(
            StatusCode::BAD_REQUEST,
            json!({
                "error": "Only instances installed by an install-method extension can be updated this way.",
                "error_code": "update_unsupported",
            }),
        ));
    }

    Ok((instance, install_config))
}

/// 같은 서버 폴더를 쓰는 다른 인스턴스가 켜져 있거나 작업 중이라 지금은 업데이트를 시작하지 못한다
/// (`begin_server_update`가 409 `instance_busy`·`server_running`으로 거절한다). 자동 업데이트가 끄기 전에 본다
pub(crate) async fn update_blocked_by_shared_folder(state: &IPCServer, id: &str) -> bool {
    let install_dir = {
        let supervisor = state.supervisor.read().await;
        let Ok((instance, install_config)) = updatable(&supervisor, id) else {
            return false;
        };
        install_dir_of(&instance, &supervisor.instance_store.instance_dir(id), Some(&install_config))
    };
    super::super::managed::other_instances_busy(state, &install_dir.to_string_lossy(), Some(id)).await
}

/// 업데이트를 백그라운드로 시작한다. 작업은 성공 여부를 돌려준다 (예약 업데이트가 기다린다)
pub(crate) async fn begin_server_update(state: &IPCServer, id: &str) -> Result<tokio::task::JoinHandle<bool>, super::super::Failure> {
    begin_server_update_holding(state, id, None).await
}

/// `begin_server_update`. `held`: 부르는 쪽이 이미 잡은 작업 잠금 (자동 업데이트가 끄기부터 잡고 있다: 놓았다 다시 잡는 틈이 없게)
pub(crate) async fn begin_server_update_holding(
    state: &IPCServer,
    id: &str,
    held: Option<super::super::backup::InstanceLock>,
) -> Result<tokio::task::JoinHandle<bool>, super::super::Failure> {
    // 업데이트가 끝날 때까지 시작·복원·삭제를 막는다 (작업 안으로 넘긴다)
    let held = match held {
        Some(lock) => Some(lock),
        None => super::super::backup::InstanceLock::take_for_writes(id).await,
    };
    let Some(guard) = held.map(|g| g.purpose(super::super::backup::Purpose::ServerUpdate)) else {
        return Err(super::super::Failure::busy());
    };
    let supervisor = state.supervisor.read().await;

    let (instance, install_config) = updatable(&supervisor, id)?;
    let method = install_config.method.clone();

    // 실행 중이면 업데이트 불가
    if supervisor.is_running(id).await {
        return Err(super::super::Failure::running(json!({
            "error": "Cannot update while server is running. Stop the server first.",
            "error_code": "server_running",
        })));
    }

    let instance_dir = supervisor.instance_store.instance_dir(id);
    let install_dir = install_dir_of(&instance, &instance_dir, Some(&install_config));
    let install_dir_str = install_dir.to_string_lossy().to_string();
    // 같은 서버 폴더를 쓰는 다른 인스턴스가 켜져 있거나 작업 중이면 덮어쓰지 않는다 (업데이트가 끝날 때까지 잡는다)
    drop(supervisor);
    let others = super::super::managed::hold_other_instances_using(state, &install_dir_str, Some(id)).await?;
    let supervisor = state.supervisor.read().await;

    let tracker = state.provision_tracker.clone();
    let supervisor_arc = state.supervisor.clone();
    let text = crate::chat::i18n::Catalog::for_language(&state.config_store.ui_language().await);
    let instance_name = instance.name.clone();
    let module_name_owned = instance.module_name.clone();
    let branch = instance.install_branch(&method).map(str::to_string);
    let accepted_agreements: Vec<String> = instance.accepted_agreements.keys().cloned().collect();
    let module = supervisor.module_loader.get_module(&instance.module_name).ok();
    // 처음 설치와 같은 인스턴스 설정·데이터 폴더·HOST 문맥 (D5)
    let target = crate::supervisor::InstanceInstall::of(&instance, &instance_dir, module.as_ref().map(|m| &m.metadata), state.host_context(id));

    // 프로비저닝 상태 등록 (기존 인스턴스 업데이트 — 실패해도 롤백 대상이 아니다)
    tracker.take_creating(id);
    tracker.update(
        id,
        &instance_name,
        crate::ipc::ProvisionProgress {
            step: 0,
            total: 1,
            label: method.clone(),
            message: text.t("provision.updating", &[]),
            done: false,
            error: None,
            percent: Some(0),
            steps: None, details: Default::default(),
        },
    );

    // supervisor lock 해제 후 백그라운드 태스크 실행
    drop(supervisor);

    let tracker_done = tracker.clone();
    let (id_done, name_done) = (id.to_string(), instance_name.clone());

    let backup_state = state.clone();
    let backup_id = id.to_string();
    // 요청의 출처·언어를 지닌 채 돈다 (모듈 진행 문구가 요청한 화면 언어로)
    Ok(crate::audit::spawn(async move {
        let _guard = guard;
        let _others = others;
        // ── Phase 0: 업데이트 전 백업 (A2). 백업할 것이 없으면 그냥 진행, 실패하면 업데이트하지 않는다 ──
        if let Err(r) = super::super::backup::take_backup_held(&backup_state, &backup_id, crate::backup::BackupKind::PreUpdate, &_guard).await {
            let reason = r.reason().unwrap_or("backup failed").to_string();
            let failure = r.body;
            tracing::error!("Pre-update backup failed for '{}': {}", name_done, reason);
            emit_update_failed(&backup_state, &backup_id, &reason).await;
            tracker_done.update(
                &id_done,
                &name_done,
                crate::ipc::ProvisionProgress {
                    step: 0,
                    total: 1,
                    label: "done".to_string(),
                    message: text.t("provision.update_failed", &[]),
                    done: true,
                    error: Some(text.t("provision.backup_failed", &[("reason", &reason)])),
                    percent: Some(100),
                    steps: None, details: Default::default(),
                }.with_failure(failure),
            );
            return false;
        }

        // ── Phase 1: supervisor lock을 최소 시간만 잡고 모듈 경로 복사 ──
        let lifecycle_path = supervisor_arc.read().await.module_loader.get_module(&module_name_owned).map(|m| m.entry_path());
        let lifecycle_path = {
            match lifecycle_path {
                Ok(path) => path,
                Err(e) => {
                    emit_update_failed(&backup_state, &backup_id, &e.to_string()).await;
                    tracker_done.update(
                        &id_done,
                        &name_done,
                        crate::ipc::ProvisionProgress {
                            step: 0,
                            total: 1,
                            label: "done".to_string(),
                            message: text.t("provision.update_failed", &[]),
                            done: true,
                            error: Some(text.t("provision.module_missing", &[("error", &e.to_string())])),
                            percent: Some(100),
                            steps: None, details: Default::default(),
                        },
                    );
                    return false;
                }
            }
        };
        // supervisor read lock 해제됨

        // ── Phase 2: 플러그인 호출 (lock 없이 실행) ──
        let result = {
            // 고른 브랜치가 있으면 그 브랜치로 받는다
            let call = crate::supervisor::InstallCall::new(&module_name_owned, lifecycle_path, &install_dir_str, &accepted_agreements)
                .version(branch.as_deref())
                .instance(target);
            let progress = crate::supervisor::progress_to_tracker(tracker_done.clone(), id_done.clone(), name_done.clone(), method.clone());

            // 업데이트가 끝날 때까지 모듈·익스텐션 자동 교체를 막는다
            let gate = supervisor_arc.read().await.addon_gate();
            let _addons = gate.hold_addons().await;
            call.run(Some(progress)).await
        };

        let updated = match result {
            Ok(val) => {
                let success = val
                    .get("success")
                    .and_then(|s| s.as_bool())
                    .unwrap_or(false);
                if success {
                    tracing::info!("Server update completed for '{}'", name_done);
                    // 받은 새 빌드를 적어 둔다 (확인은 오래 걸릴 수 있어 뒤에서)
                    let (state, id) = (backup_state.clone(), backup_id.clone());
                    tokio::spawn(async move { server_update_status(&state, &id).await });
                    // 켜짐·꺼짐 알림을 켠 길드의 알림 채널에 "업데이트 끝남" (릴레이)
                    crate::events::emit("instance.updated", json!({ "instanceId": backup_id, "instance": name_done }));
                    // 받은 빌드가 최신이다: 지난 확인의 "새 빌드 있음"을 지운다 (다음 주기 확인이 다시 채운다)
                    super::super::server_update::forget_check(&backup_id);
                    tracker_done.update(
                        &id_done,
                        &name_done,
                        crate::ipc::ProvisionProgress {
                            step: 0,
                            total: 1,
                            label: "done".to_string(),
                            message: text.t("provision.updated", &[]),
                            done: true,
                            error: None,
                            percent: Some(100),
                            steps: None, details: Default::default(),
                        },
                    );
                } else {
                    let err_msg = val
                        .get("message")
                        .or_else(|| val.get("error"))
                        .and_then(|v| v.as_str())
                        .map(str::to_string)
                        .unwrap_or_else(|| text.t("provision.update_failed", &[]));
                    tracing::error!("Server update failed for '{}': {}", name_done, err_msg);
                    emit_update_failed(&backup_state, &backup_id, &err_msg).await;
                    tracker_done.update(
                        &id_done,
                        &name_done,
                        crate::ipc::ProvisionProgress {
                            step: 0,
                            total: 1,
                            label: "done".to_string(),
                            message: text.t("provision.update_failed", &[]),
                            done: true,
                            error: Some(err_msg),
                            percent: Some(100),
                            steps: None, details: Default::default(),
                        }.with_failure(val.clone()),
                    );
                }
                success
            }
            Err(e) => {
                tracing::error!("Server update error for '{}': {}", name_done, e);
                emit_update_failed(&backup_state, &backup_id, &text.t("provision.script_failed", &[])).await;
                tracker_done.update(
                    &id_done,
                    &name_done,
                    crate::ipc::ProvisionProgress {
                        step: 0,
                        total: 1,
                        label: "done".to_string(),
                        message: text.t("provision.update_failed", &[]),
                        done: true,
                        error: Some(text.t("provision.script_failed", &[])),
                        percent: Some(100),
                        steps: None, details: Default::default(),
                    }.with_failure(crate::plugin::error_response(&e)),
                );
                false
            }
        };

        // 5초 후 프로비저닝 tracker 자동 정리
        let tracker_cleanup = tracker_done.clone();
        let id_cleanup = id_done.clone();
        tokio::spawn(async move {
            tokio::time::sleep(std::time::Duration::from_secs(5)).await;
            tracker_cleanup.remove(&id_cleanup);
        });
        updated
    }))

}

/// 업데이트가 덮어쓸 폴더: 서버가 실제로 도는 working_dir, 없으면 instance_dir / install_subdir (기본 "server")
fn install_dir_of(
    instance: &crate::instance::ServerInstance,
    instance_dir: &std::path::Path,
    install_config: Option<&crate::supervisor::module_loader::ModuleInstallConfig>,
) -> std::path::PathBuf {
    if let Some(dir) = instance.working_dir.as_deref().filter(|d| !d.trim().is_empty()) {
        return std::path::PathBuf::from(dir);
    }
    let subdir = install_config
        .and_then(|cfg| cfg.install_subdir.as_deref())
        .unwrap_or("server");
    instance_dir.join(subdir)
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    /// 처음 설치(30분까지 걸린다)하는 동안 업데이트·삭제·복원이 같은 폴더에 끼어들지 못한다
    /// 가져온(마이그레이션) 서버는 working_dir이 실제 서버 폴더다. 업데이트 확인·적용이 빈 `instance_dir/server`를
    /// 보면 늘 "업데이트 있음"이 나오고, 적용하면 그 빈 폴더에 서버를 새로 받은 뒤 옛 서버를 다시 켠다
    #[test]
    fn updates_go_to_the_folder_the_server_runs_from() {
        let mut instance = crate::instance::ServerInstance::new("a", "palworld");
        let dir = std::path::Path::new("/data/instances/a");
        assert_eq!(super::install_dir_of(&instance, dir, None), dir.join("server"));
        instance.working_dir = Some("D:/Steam/steamapps/common/PalServer".into());
        assert_eq!(super::install_dir_of(&instance, dir, None), std::path::PathBuf::from("D:/Steam/steamapps/common/PalServer"));
    }

    // 업데이트는 서버가 도는 working_dir에 쓴다: 같은 폴더(그 안이나 바깥)를 쓰는 다른 인스턴스가 작업 중이면 덮어쓰지 않는다
    #[tokio::test]
    async fn an_update_waits_for_other_instances_sharing_the_server_folder() {
        let tmp = tempfile::tempdir().unwrap();
        let modules = tmp.path().join("modules");
        let dir = modules.join("game");
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(
            dir.join("module.toml"),
            "[module]\nschema_version = 1\nname = \"game\"\nversion = \"1.0.0\"\nentry = \"lifecycle.py\"\n\n[install]\nmethod = \"steamcmd\"\n",
        )
        .unwrap();
        std::fs::write(dir.join("lifecycle.py"), "").unwrap();
        let mut sup = crate::supervisor::Supervisor::new_with_instances_dir(modules.to_str().unwrap(), tmp.path().join("instances").to_str().unwrap());
        for module in sup.module_loader.discover_modules().unwrap() { crate::sandbox::consent_test_component(std::path::Path::new(&module.entry_path())); }
        sup.initialize().await.unwrap();
        let shared = tmp.path().join("PalServer").to_string_lossy().to_string();
        // 예전에 만든 인스턴스는 다른 서버 폴더 안에 있을 수 있다: 같은 폴더처럼 잠근다
        let inside = tmp.path().join("PalServer").join("Pal").to_string_lossy().to_string();
        let mut ids = Vec::new();
        for (name, dir) in [("a", &shared), ("b", &inside)] {
            let mut inst = crate::instance::ServerInstance::new(name, "game");
            inst.working_dir = Some(dir.clone());
            inst.extension_data.insert(crate::instance::install_method_key("steamcmd"), json!(true));
            ids.push(inst.id.clone());
            sup.instance_store.add(inst).unwrap();
        }
        let state = crate::ipc::IPCServer::new(std::sync::Arc::new(tokio::sync::RwLock::new(sup)), "127.0.0.1:0", crate::daemon_log::DaemonLogBuffer::new());
        let other = super::super::super::backup::InstanceLock::take(&ids[1]).expect("free");
        let refused = super::begin_server_update(&state, &ids[0]).await.err().expect("the other instance is busy");
        // 자동 업데이트는 이 거절을 실패로 남기지 않고 미룬다: 본문의 코드를 읽지 않고 가려낸다
        assert!(refused.is_in_use(), "{}", refused.body);
        assert_eq!(refused.body["error_code"], "instance_busy");
        drop(other);
        assert!(super::super::super::backup::InstanceLock::take(&ids[0]).is_some(), "a refused update must not keep its own lock");
    }

    /// 업데이트의 `install_server`도 처음 설치와 같이 인스턴스의 데이터 폴더(`instance_data_dirs`)를 받는다 (D5).
    /// 빠지면 확인되지 않은 모듈이 서버 폴더 밖 데이터 폴더(좀보이드 `cache_dir` 등)에 쓰지 못한다
    #[tokio::test]
    async fn an_update_gets_the_instance_data_folders_like_the_first_install() {
        let tmp = tempfile::tempdir().unwrap();
        let modules = tmp.path().join("modules");
        let dir = modules.join("game");
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(
            dir.join("module.toml"),
            "[module]\nschema_version = 1\nname = \"game\"\nversion = \"1.0.0\"\nentry = \"lifecycle.py\"\n\n[install]\nmethod = \"steamcmd\"\n",
        )
        .unwrap();
        let seen = tmp.path().join("install_config.json");
        std::fs::write(
            dir.join("lifecycle.py"),
            format!(
                "import json, sys\ncfg = sys.stdin.read()\nif sys.argv[1] == 'install_server':\n    open({seen:?}, 'w').write(cfg)\nprint(json.dumps({{'success': True}}))\n",
                seen = seen.to_str().unwrap()
            ),
        )
        .unwrap();
        crate::sandbox::consent_test_component(&dir.join("lifecycle.py"));
        let instances = tmp.path().join("instances");
        let mut sup = crate::supervisor::Supervisor::new_with_instances_dir(modules.to_str().unwrap(), instances.to_str().unwrap());
        sup.initialize().await.unwrap();
        let mut inst = crate::instance::ServerInstance::new("srv", "game");
        inst.working_dir = Some(tmp.path().join("server").to_string_lossy().to_string());
        inst.extension_data.insert(crate::instance::install_method_key("steamcmd"), json!(true));
        let cache = sup.instance_store.instance_dir(&inst.id).join("cache");
        inst.module_settings.insert("cache_dir".into(), json!(cache.to_string_lossy()));
        let id = inst.id.clone();
        sup.instance_store.add(inst).unwrap();
        let state = crate::ipc::IPCServer::new(std::sync::Arc::new(tokio::sync::RwLock::new(sup)), "127.0.0.1:0", crate::daemon_log::DaemonLogBuffer::new());

        let done = super::begin_server_update(&state, &id).await.ok().expect("the update starts");
        assert!(done.await.unwrap(), "the update succeeds");
        let config: serde_json::Value = serde_json::from_str(&std::fs::read_to_string(&seen).unwrap()).unwrap();
        assert_eq!(config[crate::sandbox::INSTANCE_DATA_DIRS], json!([cache.to_string_lossy()]), "{config}");
    }

    /// A-09: 업데이트 확인은 모듈 `[install]`을 그대로 받는다. 인스턴스에 남은 옛 사본(`steamcmd_app_id`)을 쓰지 않는다
    #[test]
    fn an_update_check_gets_the_install_values_from_the_module_not_from_instance_copies() {
        let install: crate::supervisor::module_loader::ModuleInstallConfig =
            serde_json::from_value(json!({ "method": "steamcmd", "app_id": 2394010, "beta": "preview", "install_subdir": "server", "download_size": 5700000000u64 })).unwrap();
        let mut inst = crate::instance::ServerInstance::new("pal", "pal");
        inst.extension_data.insert("steamcmd_app_id".into(), json!(111));
        let dir = std::path::Path::new("/data/instances/pal");
        let ctx = super::check_update_context(&inst, dir, Some(&install));
        assert_eq!((ctx["app_id"].clone(), ctx["install"]["app_id"].clone(), ctx["install"]["download_size"].clone()), (json!(2394010), json!(2394010), json!(5700000000u64)));
        assert_eq!(ctx["branch"], "preview", "the module's branch when the user chose none");
        assert!(ctx["install_dir"].as_str().unwrap().ends_with("server"), "{ctx}");
        inst.record_installed_version("steamcmd", "unstable");
        assert_eq!(super::check_update_context(&inst, dir, Some(&install))["branch"], "unstable", "the branch the user installed");
        let unknown = super::check_update_context(&inst, dir, None);
        assert_eq!((unknown["app_id"].clone(), unknown["install"].clone(), unknown["branch"].clone()), (json!(null), json!(null), json!(null)));
    }

    /// 다운로드·수동 설치 서버는 사바쨩이 업데이트를 확인하지 않는다: "설치 방법 기록이 없다"가 아니라 그렇다고 답한다
    /// (테라리아 점검). 설치 방식 익스텐션으로 설치한 서버만 hook으로 확인한다
    #[tokio::test]
    async fn only_servers_installed_by_an_extension_check_for_updates() {
        use std::sync::Arc;
        let tmp = tempfile::tempdir().unwrap();
        let modules = tmp.path().join("modules");
        for (name, method) in [("dl", "download"), ("sc", "steamcmd")] {
            let dir = modules.join(name);
            std::fs::create_dir_all(&dir).unwrap();
            std::fs::write(dir.join("module.toml"), format!("[module]\nschema_version = 1\nname = '{name}'\nversion = '1.0.0'\nentry = 'lifecycle.py'\n\n[install]\nmethod = '{method}'\n")).unwrap();
            std::fs::write(dir.join("lifecycle.py"), "").unwrap();
        }
        let mut sup = crate::supervisor::Supervisor::new_with_instances_dir(modules.to_str().unwrap(), tmp.path().join("instances").to_str().unwrap());
        sup.initialize().await.unwrap();
        let state = crate::ipc::IPCServer::new(Arc::new(tokio::sync::RwLock::new(sup)), "127.0.0.1:0", crate::daemon_log::DaemonLogBuffer::new());
        let add = |name: &str, module: &str, by_extension: bool| {
            let mut inst = crate::instance::ServerInstance::new(name, module);
            if by_extension {
                inst.apply_install_extension_data("steamcmd", &[]);
            }
            let id = inst.id.clone();
            state.supervisor.try_write().unwrap().instance_store.add(inst).unwrap();
            id
        };
        let (terra, imported, steam) = (add("terra", "dl", false), add("imported", "sc", false), add("pal", "sc", true));
        let gone = add("gone", "missing", false);
        let reason = |id: String| {
            let state = state.clone();
            async move { super::server_update_status(&state, &id).await.unwrap()["reason"].clone() }
        };
        assert_eq!(reason(terra).await, json!("update_unsupported"));
        assert_eq!(reason(imported).await, json!("update_unsupported"), "not installed through the extension");
        assert_eq!(reason(steam).await, json!("no_hook_handled"), "no steamcmd extension in this test");
        assert_eq!(reason(gone).await, json!("no_extension_data"), "a module that did not load is not called un-updatable");
    }

    /// 설치 방식 익스텐션으로 받은 서버는 업데이트 확인이 알려 준 설치 빌드를 적어 두고 서버 목록에 싣는다 (다시 묻지 않고 보인다)
    #[tokio::test]
    async fn an_update_check_records_the_installed_build_for_the_server_list() {
        use std::sync::Arc;
        let tmp = tempfile::tempdir().unwrap();
        let mut sup = crate::supervisor::Supervisor::new_with_instances_dir(tmp.path().join("modules").to_str().unwrap(), tmp.path().join("instances").to_str().unwrap());
        sup.initialize().await.unwrap();
        let state = crate::ipc::IPCServer::new(Arc::new(tokio::sync::RwLock::new(sup)), "127.0.0.1:0", crate::daemon_log::DaemonLogBuffer::new());
        let inst = crate::instance::ServerInstance::new("pal", "sc");
        let (id, opened) = (inst.id.clone(), inst.settings_revision());
        state.supervisor.write().await.instance_store.add(inst).unwrap();
        let listed = || {
            let state = state.clone();
            async move { crate::ipc::internal::into_internal(crate::ipc::handlers::server::list_servers_internal(&state).await).await.body["servers"][0].clone() }
        };
        assert_eq!(listed().await.get("server_build"), None, "nothing is known before the first check");

        super::record_build(&state, &id, &json!({"update_available": true, "local_buildid": "19283746", "local_build_time": 1762128000, "remote_buildid": "19402210"})).await;
        assert_eq!(listed().await["server_build"], json!({"id": "19283746", "time": 1762128000}));
        assert_eq!(listed().await["revision"], json!(opened), "a settings form opened before the check can still save");

        // 설치 폴더를 읽지 못한 확인은 적어 둔 빌드를 지우지 않는다
        super::record_build(&state, &id, &json!({"update_available": false, "reason": "check_failed", "local_buildid": null})).await;
        assert_eq!(listed().await["server_build"]["id"], json!("19283746"));

        // 업데이트한 뒤의 확인은 새 빌드로 바꾼다 (받은 때를 모르면 번호만)
        super::record_build(&state, &id, &json!({"update_available": false, "local_buildid": 19402210})).await;
        assert_eq!(listed().await["server_build"], json!({"id": "19402210"}));
    }

    /// 얇은 클라이언트 점검 2번: 화면이 서버마다 확인을 돌리지 않게 데몬이 한 확인의 결과를 기억해 목록에 싣는다.
    /// 확인하지 않는 서버는 비워 둔다
    #[tokio::test]
    async fn the_server_list_carries_the_last_update_check() {
        use std::sync::Arc;
        let tmp = tempfile::tempdir().unwrap();
        let modules = tmp.path().join("modules");
        for (name, method) in [("dl", "download"), ("sc", "steamcmd")] {
            let dir = modules.join(name);
            std::fs::create_dir_all(&dir).unwrap();
            std::fs::write(dir.join("module.toml"), format!("[module]\nschema_version = 1\nname = '{name}'\nversion = '1.0.0'\nentry = 'lifecycle.py'\n\n[install]\nmethod = '{method}'\n")).unwrap();
            std::fs::write(dir.join("lifecycle.py"), "").unwrap();
        }
        let mut sup = crate::supervisor::Supervisor::new_with_instances_dir(modules.to_str().unwrap(), tmp.path().join("instances").to_str().unwrap());
        sup.initialize().await.unwrap();
        let state = crate::ipc::IPCServer::new(Arc::new(tokio::sync::RwLock::new(sup)), "127.0.0.1:0", crate::daemon_log::DaemonLogBuffer::new());
        let mut steam = crate::instance::ServerInstance::new("pal-list", "sc");
        steam.apply_install_extension_data("steamcmd", &[]);
        let terra = crate::instance::ServerInstance::new("terra-list", "dl");
        let (steam_id, terra_id) = (steam.id.clone(), terra.id.clone());
        state.supervisor.try_write().unwrap().instance_store.add(steam).unwrap();
        state.supervisor.try_write().unwrap().instance_store.add(terra).unwrap();

        let listed = |state: crate::ipc::IPCServer| async move {
            crate::ipc::internal::into_internal(crate::ipc::handlers::server::list_servers_internal(&state).await).await.body
        };
        let check_of = |body: &serde_json::Value, id: &str| body["servers"].as_array().unwrap().iter().find(|s| s["id"] == json!(id)).unwrap()["update_check"].clone();
        let before = listed(state.clone()).await;
        assert_eq!(check_of(&before, &steam_id), serde_json::Value::Null, "not checked yet");

        super::server_update_status(&state, &steam_id).await.unwrap();
        super::server_update_status(&state, &terra_id).await.unwrap();
        let after = listed(state.clone()).await;
        let steam_check = check_of(&after, &steam_id);
        assert_eq!(steam_check["reason"], json!("no_hook_handled"));
        assert!(steam_check["checked_at"].as_u64().unwrap() > 0);
        assert_eq!(check_of(&after, &terra_id), serde_json::Value::Null, "a server sabachan does not update has nothing to show");

        crate::ipc::handlers::server_update::forget_check(&steam_id);
        assert_eq!(check_of(&listed(state.clone()).await, &steam_id), serde_json::Value::Null, "an applied update clears the old result");
    }

    /// 확인 hook이 실패하면(시간 초과·예외) "업데이트 없음"이 아니라 확인 실패로 답한다. 성공한 답이 있으면 그것을 쓴다
    #[test]
    fn a_failed_update_check_is_not_reported_as_up_to_date() {
        let failed = super::update_status_from_hooks(vec![("steamcmd".into(), Err(anyhow::anyhow!("timed out")))]);
        assert_eq!(failed["update_available"], json!(false));
        assert_eq!(failed["reason"], json!("check_failed"));
        assert_eq!(failed["error"], json!("timed out"));
        let answered = super::update_status_from_hooks(vec![
            ("a".into(), Err(anyhow::anyhow!("boom"))),
            ("steamcmd".into(), Ok(json!({"update_available": true}))),
        ]);
        assert_eq!(answered, json!({"update_available": true}));
        assert_eq!(super::update_status_from_hooks(vec![])["reason"], json!("no_hook_handled"));
        // 익스텐션이 정식 실패 형식({success:false})으로 답해도 확인 실패다. 다른 hook의 성공한 답이 우선
        let refused = super::update_status_from_hooks(vec![("steamcmd".into(), Ok(json!({"success": false, "error": "sandbox blocked"})))]);
        assert_eq!(refused["reason"], json!("check_failed"));
        assert_eq!(refused["update_available"], json!(false));
        assert_eq!(refused["error"], json!("sandbox blocked"));
        let mixed = super::update_status_from_hooks(vec![
            ("a".into(), Ok(json!({"success": false, "error": "x"}))),
            ("steamcmd".into(), Ok(json!({"update_available": false, "local_buildid": "1"}))),
        ]);
        assert_eq!(mixed, json!({"update_available": false, "local_buildid": "1"}));
    }
}
