//! 서버 버전 설치(POST /api/module/:name/install)의 본체: 요청 풀기 → 인스턴스 잡기 → 모듈 확인 → 버전 고르기 →
//! 설치 전 백업·설치·되돌리기 (뒤에서 돌리면 진행은 provision-progress)

use axum::http::StatusCode;
use serde_json::{json, Value};

use crate::instance::lock::InstanceLock;
use crate::ipc::IPCServer;
use crate::service::{Failure, Outcome};
use crate::supervisor::{InstanceInstall, Supervisor};

/// 설치 요청 본문
struct Request {
    /// 비우면 그 인스턴스 설정으로 최신을 고른다
    version: Option<String>,
    instance_id: Option<String>,
    install_dir: String,
    /// 서버 실행 파일 이름 (없으면 모듈 `server_executable`)
    executable: Option<String>,
    accepted_agreements: Vec<String>,
    initial_settings: Option<Value>,
    background: bool,
}

/// 설치할 인스턴스: 그 설정으로 버전을 고르고 설치한다. 그 데이터 폴더를 모듈에 넘기므로 설치가 끝날 때까지 잡아 둔다
struct Target {
    id: String,
    name: String,
    install: InstanceInstall,
    /// 지금 쓰는 실행 파일 (설치가 실패하면 되돌린다)
    executable: Option<String>,
    guard: InstanceLock,
}

/// 설치를 끝까지 하거나(배경이 아니면) 뒤에서 시작하고 `provisioning: true`로 답한다.
/// 모듈이 `success: false`로 답하면 422, 설치 전 백업 실패는 422, 그 밖의 오류는 500
pub(crate) async fn install(state: &IPCServer, module_name: &str, payload: &Value) -> Outcome {
    let request = request(payload)?;
    let target = match request.instance_id.as_deref() {
        Some(id) => Some(hold_target(state, module_name, id).await?),
        None => None,
    };
    check_module(state, module_name, &request.accepted_agreements).await?;
    let version = match &request.version {
        Some(v) => v.clone(),
        None => latest_version(state, module_name, target.as_ref()).await?,
    };
    // 이 폴더를 쓰는 서버가 켜져 있거나 백업·복원 중이면 파일을 덮어쓰지 않는다.
    // 설치를 마칠 때까지 잡고, 요청이 끊겨도 잠금이 설치와 함께 남게 따로 돌린다
    let others = crate::service::shared_folder::hold_other_instances_using(state, &request.install_dir, request.instance_id.as_deref()).await?;
    // 인스턴스의 버전 설치는 뒤에서 돌릴 수 있다: 설치 전 백업이 길어도 요청 한도에 걸리지 않게
    let background = request.background.then(|| target.as_ref().map(|t| (t.id.clone(), t.name.clone()))).flatten();
    let text = crate::chat::i18n::Catalog::for_language(&state.config_store.ui_language().await);
    if let Some((id, name)) = &background {
        state.provision_tracker.take_creating(id);
        state.provision_tracker.update(id, name, installing(text.t("provision.version_installing", &[("version", &version)])));
    }
    let job = crate::audit::spawn(run(state.clone(), module_name.to_string(), version.clone(), request, target, others));
    let version_label = version.clone();
    let finish = move |outcome: Result<Result<Value, Value>, tokio::task::JoinError>| match outcome.unwrap_or_else(|e| Err(json!({ "success": false, "error": e.to_string(), "error_code": "internal_server_error" }))) {
        Ok(mut result) => {
            // 버전을 비운 클라이언트도 무엇을 설치했는지 안다
            if let Some(obj) = result.as_object_mut() {
                obj.entry("version").or_insert_with(|| json!(version));
            }
            Ok(result)
        }
        Err(body) => Err(body),
    };
    if let Some((id, name)) = background {
        let tracker = state.provision_tracker.clone();
        let body = json!({ "success": true, "provisioning": true, "id": id, "version": version_label });
        tokio::spawn(async move { report(tracker, &text, &id, &name, finish(job.await)).await });
        return Ok(body);
    }
    match finish(job.await) {
        Ok(result) if result.get("success").and_then(Value::as_bool) == Some(true) => Ok(result),
        Ok(result) => Err(Failure::new(StatusCode::UNPROCESSABLE_ENTITY, result)),
        Err(body) => {
            let status = if body["error_code"] == "backup_failed" { StatusCode::UNPROCESSABLE_ENTITY } else { StatusCode::INTERNAL_SERVER_ERROR };
            Err(Failure::new(status, body))
        }
    }
}

fn request(payload: &Value) -> Result<Request, Failure> {
    // 있으면 문자열이어야 한다 (형식이 틀린 값을 "비움"으로 받아 다른 버전을 설치하지 않게)
    let text_field = |field: &str| match payload.get(field) {
        None => Ok(None),
        Some(Value::String(v)) => Ok(Some(v.clone())),
        Some(_) => Err(super::invalid_field(field, format!("'{}' must be a string", field))),
    };
    let version = text_field("version")?.filter(|v| !v.is_empty());
    let instance_id = text_field("instance_id")?;
    let Some(install_dir) = payload.get("install_dir").and_then(Value::as_str) else {
        return Err(Failure::coded(StatusCode::BAD_REQUEST, "invalid_request", "Missing 'install_dir' field"));
    };
    let accepted_agreements = super::requested_agreements(payload, "accept_agreements")?;
    let background = instance_id.is_some() && payload.get("background").and_then(Value::as_bool) == Some(true);
    Ok(Request {
        version,
        instance_id,
        install_dir: install_dir.to_string(),
        executable: payload.get("server_executable").and_then(Value::as_str).map(str::to_string),
        accepted_agreements,
        initial_settings: payload.get("initial_settings").cloned(),
        background,
    })
}

/// 인스턴스를 잡는다: 켜져 있거나 백업·복원·삭제 중이면 409. 설정은 잡은 뒤에 읽는다
async fn hold_target(state: &IPCServer, module_name: &str, id: &str) -> Result<Target, Failure> {
    let not_found = || Failure::coded(StatusCode::NOT_FOUND, "not_found", format!("Instance not found: {}", id));
    match state.supervisor.read().await.instance_store.get(id) {
        Some(inst) if inst.module_name == module_name => {}
        Some(_) => return Err(super::invalid_field("instance_id", format!("Instance '{}' is not a '{}' server", id, module_name))),
        None => return Err(not_found()),
    }
    let Some(guard) = InstanceLock::take_for_writes(id).await else {
        return Err(Failure::busy());
    };
    if crate::service::backup::is_running(state, id).await {
        return Err(crate::service::shared_folder::server_running());
    }
    let sup = state.supervisor.read().await;
    // 잡기 전에 지워졌을 수 있다
    let Some(inst) = sup.instance_store.get(id) else {
        return Err(not_found());
    };
    let dir = sup.instance_store.instance_dir(id);
    let module = sup.module_loader.get_module(module_name).ok();
    let install = InstanceInstall::of(inst, &dir, module.as_ref().map(|m| &m.metadata), state.host_context(id));
    Ok(Target { id: id.to_string(), name: inst.name.clone(), install, executable: sup.resolve_executable(inst, module_name), guard })
}

fn module_not_found(module_name: &str, e: impl std::fmt::Display) -> Failure {
    Failure::coded(StatusCode::NOT_FOUND, "not_found", format!("Module '{}' not found: {}", module_name, e))
}

/// 모듈이 있고, 받은 약관이 그 모듈의 것이고, 필수 익스텐션이 켜져 있는가
async fn check_module(state: &IPCServer, module_name: &str, accepted_agreements: &[String]) -> Result<(), Failure> {
    let found = state.supervisor.read().await.module_loader.get_module(module_name);
    let module = found.map_err(|e| module_not_found(module_name, e))?;
    if let Some(unknown) = accepted_agreements.iter().find(|id| !module.metadata.agreements.iter().any(|a| &a.id == *id)) {
        return Err(super::invalid_field("accept_agreements", format!("Module '{}' has no agreement '{}'", module_name, unknown)));
    }
    if let Some(ref install_cfg) = module.metadata.install {
        if !install_cfg.requires_extensions.is_empty() {
            let found = state.extension_manager.read().await.missing_required(&install_cfg.requires_extensions);
            if !found.missing.is_empty() {
                return Err(super::extension_required("install server", &found));
            }
        }
    }
    Ok(())
}

/// 버전을 비운 요청: 그 인스턴스 설정으로 모듈에 최신을 묻는다 (모르면 422)
async fn latest_version(state: &IPCServer, module_name: &str, target: Option<&Target>) -> Result<String, Failure> {
    let module_path = match state.supervisor.read().await.module_loader.get_module(module_name) {
        Ok(m) => m.entry_path(),
        Err(e) => return Err(module_not_found(module_name, e)),
    };
    let settings = target.map(|t| t.install.settings().clone()).unwrap_or_else(|| json!({}));
    Supervisor::latest_version(&module_path, &settings).await.map_err(|e| {
        let error = if e.downcast_ref::<crate::plugin::ModuleCallError>().is_some() {
            crate::plugin::error_response(&e)
        } else {
            json!({ "success": false, "error": format!("Could not find the latest version of this server: {}", e), "error_code": "version_unknown" })
        };
        Failure::new(StatusCode::UNPROCESSABLE_ENTITY, error)
    })
}

/// 설치 전 백업 → 설치 → (실패면) 백업으로 되돌리기. `Ok`는 모듈의 답(실패일 수 있다), `Err`는 데몬이 만든 실패 본문
async fn run(state: IPCServer, module_name: String, version: String, request: Request, target: Option<Target>, others: Vec<InstanceLock>) -> Result<Value, Value> {
    let _others = others;
    let Request { install_dir, executable, accepted_agreements, initial_settings, .. } = request;
    let (install, keep_executable, own) = match target {
        Some(Target { id, install, executable, guard, .. }) => (Some(install), executable, Some((id, guard))),
        None => (None, None, None),
    };
    let own = own.as_ref().map(|(id, guard)| (id.as_str(), guard));
    // 설치 전 백업 (결정 3): 백업할 것이 없으면 그냥 진행하고, 실패하면 설치하지 않는다
    let before = match own {
        Some((id, guard)) => match crate::service::backup::take_backup_held(&state, id, crate::backup::BackupKind::PreUpdate, guard).await {
            Ok(made) => made.map(|b| b.name),
            Err(r) => {
                let reason = r.reason().unwrap_or("backup failed").to_string();
                return Err(json!({ "success": false, "error": format!("Pre-install backup failed: {}", reason), "error_code": "backup_failed", "reason": reason }));
            }
        },
        None => None,
    };
    // 받은 파일로 켜도록 버전·실행 파일·작업 폴더를 함께 맞춘다 (클라이언트가 따로 저장하지 않는다).
    // 실행 파일 백업을 지우기 전에 저장해, 저장하지 못하면 실행 파일도 되돌린다
    let commit = |sup: &mut Supervisor, installed: &Value| match own {
        Some((id, _)) => save_installed(sup, id, &version, installed, &install_dir),
        None => Ok(()),
    };
    let result = Supervisor::install_server(
        &state.supervisor,
        &module_name,
        &version,
        &install_dir,
        executable.as_deref(),
        &accepted_agreements,
        initial_settings,
        install,
        keep_executable.as_deref(),
        Some(&commit),
    )
    .await
    .map_err(|e| install_error(&e));
    let Some((id, guard)) = own else {
        return result;
    };
    if result.as_ref().is_ok_and(|r| r.get("success").and_then(Value::as_bool) == Some(true)) {
        // 받은 빌드를 적어 둔다 (확인은 오래 걸릴 수 있어 뒤에서)
        let (state, id) = (state.clone(), id.to_string());
        tokio::spawn(async move { crate::service::server_update::apply::server_update_status(&state, &id).await });
        return result;
    }
    let mut result = result;
    // 실패: 설치 전 백업으로 되돌린다
    if let Some(name) = before {
        let restored = crate::service::backup::restore_held(&state, id, &name, guard, false).await;
        let body = match &mut result {
            Ok(body) | Err(body) => body,
        };
        if body.is_object() {
            match restored {
                Ok(_) => body["restored_backup"] = json!(name),
                Err(r) => {
                    let reason = r.reason().unwrap_or("restore failed").to_string();
                    tracing::error!("Could not put back the pre-install backup '{}' for '{}': {}", name, id, reason);
                    body["kept_backup"] = json!(name);
                }
            }
        }
    }
    result
}

/// 설치 오류 본문. 되돌리지 못한 파일은 422와 같은 칸으로도 싣는다
fn install_error(e: &anyhow::Error) -> Value {
    let mut body = crate::plugin::error_response(e);
    let commit_failed = e.downcast_ref::<crate::supervisor::CommitFailed>();
    if let Some(put_back) = e.downcast_ref::<crate::supervisor::PutBackFailed>().or(commit_failed.map(|c| &c.0)).filter(|p| !p.files.is_empty()) {
        body["restore_failed"] = json!(put_back.files);
        body["kept_backups"] = json!(put_back.kept);
    }
    if commit_failed.is_some() {
        tracing::error!("Installed the server files but could not save the instance: {:#}", e);
        body["success"] = json!(false);
        body["error"] = json!(format!("The server files were installed, but the server details could not be saved: {:#}", e));
        body["error_code"] = json!("save_failed");
    }
    body
}

/// 뒤에서 돈 설치의 끝을 진행 표시에 적는다. 주의가 붙었거나 실패했으면 사용자가 닫을 때까지 남긴다
async fn report(tracker: crate::ipc::ProvisionTracker, text: &crate::chat::i18n::Catalog, id: &str, name: &str, outcome: Result<Value, Value>) {
    let mut keep = true;
    let progress = match outcome {
        Ok(r) if r.get("success").and_then(Value::as_bool) == Some(true) => {
            // 모듈이 붙인 주의 한 줄 (`warning`)
            let warning = r.get("warning").and_then(Value::as_str).filter(|w| !w.is_empty());
            keep = warning.is_some();
            let installed = text.t("provision.version_installed", &[("version", r.get("version").and_then(Value::as_str).unwrap_or_default())]);
            let message = match warning {
                Some(w) => format!("{installed}\n⚠️ {w}"),
                None => installed,
            };
            crate::ipc::ProvisionProgress::finished(message, None)
        }
        // 모듈이 정한 실패 문구는 그대로, 스크립트 오류(stderr·경로)는 로그에만
        Ok(r) => crate::ipc::ProvisionProgress::finished(
            text.t("provision.install_failed", &[]),
            Some(r.get("message").and_then(Value::as_str).map(str::to_string).unwrap_or_else(|| text.t("provision.install_failed", &[]))),
        ),
        Err(body) => {
            let error = match body["error_code"].as_str() {
                Some("backup_failed") => text.t("provision.install_backup_failed", &[("reason", body["reason"].as_str().unwrap_or_default())]),
                Some("save_failed") => text.t("provision.save_failed", &[]),
                _ => text.t("provision.script_failed", &[]),
            };
            crate::ipc::ProvisionProgress::finished(text.t("provision.install_failed", &[]), Some(error))
        }
    };
    tracker.update(id, name, progress);
    if !keep {
        tokio::time::sleep(std::time::Duration::from_secs(5)).await;
        tracker.remove(id);
    }
}

/// 버전 설치를 시작한 진행 표시 (인스턴스 업데이트와 같은 모양)
fn installing(message: String) -> crate::ipc::ProvisionProgress {
    crate::ipc::ProvisionProgress { label: "install".to_string(), done: false, percent: None, ..crate::ipc::ProvisionProgress::finished(message, None) }
}

/// 설치가 끝난 인스턴스에 버전과 받은 실행 파일·작업 폴더를 저장한다. 모듈이 경로를 알려 주지 않으면 버전만
pub(crate) fn save_installed(sup: &mut Supervisor, id: &str, version: &str, result: &Value, install_dir: &str) -> anyhow::Result<()> {
    let Some(mut inst) = sup.instance_store.get(id).cloned() else {
        anyhow::bail!("instance {} is gone", id);
    };
    let method = sup.module_loader.get_module(&inst.module_name).ok().and_then(|m| m.metadata.install.map(|i| i.method)).unwrap_or_default();
    inst.record_installed_version(&method, version);
    let exe = ["executable_path", "jar_path"].iter().find_map(|k| result.get(*k).and_then(Value::as_str)).filter(|p| !p.is_empty());
    if let Some(exe) = exe {
        let folder = result
            .get("install_path")
            .and_then(Value::as_str)
            .filter(|p| !p.is_empty())
            .map(str::to_string)
            .or_else(|| std::path::Path::new(exe).parent().map(|p| p.to_string_lossy().into_owned()))
            .unwrap_or_else(|| install_dir.to_string());
        // 확인되지 않은 모듈이 알려 준 폴더는 인스턴스 폴더 안만 (작업 폴더는 다음 격리 실행에 쓰기를 받는다)
        let entry = sup.module_loader.get_module(&inst.module_name).ok().map(|m| m.entry_path());
        let root = sup.instance_store.instance_dir(id);
        let folder = if folder == install_dir || entry.is_some_and(|e| crate::sandbox::accepts_module_dir(std::path::Path::new(&e), &folder, &root)) {
            folder
        } else {
            tracing::warn!("Ignoring a server folder outside the instance folder from an unverified module: {}", folder);
            install_dir.to_string()
        };
        inst.executable_path = Some(exe.to_string());
        inst.working_dir = Some(folder);
    }
    sup.instance_store.update(id, inst)
}
