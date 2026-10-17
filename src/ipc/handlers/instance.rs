use axum::{
    extract::{Path, State},
    http::StatusCode,
    response::IntoResponse,
    Json,
};
use serde_json::{json, Value};

use super::super::IPCServer;

/// GET /api/instances - 모든 인스턴스 목록
pub async fn list_instances(State(state): State<IPCServer>) -> impl IntoResponse {
    let supervisor = state.supervisor.read().await;
    let instances = supervisor.instance_store.list();
    (StatusCode::OK, Json(instances)).into_response()
}

/// PUT /api/instances/reorder - 인스턴스 순서 변경
pub async fn reorder_instances(
    State(state): State<IPCServer>,
    Json(payload): Json<Value>,
) -> impl IntoResponse {
    let mut supervisor = state.supervisor.write().await;

    let ordered_ids: Vec<String> = match payload.get("order").and_then(|v| v.as_array()) {
        Some(arr) => arr
            .iter()
            .filter_map(|v| v.as_str().map(|s| s.to_string()))
            .collect(),
        None => {
            let error = json!({ "error": "Missing 'order' array" });
            return (StatusCode::BAD_REQUEST, Json(error)).into_response();
        }
    };

    match supervisor.instance_store.reorder(&ordered_ids) {
        Ok(_) => {
            let response = json!({ "success": true });
            (StatusCode::OK, Json(response)).into_response()
        }
        Err(e) => {
            let error = json!({ "error": format!("Failed to reorder: {}", e) });
            (StatusCode::INTERNAL_SERVER_ERROR, Json(error)).into_response()
        }
    }
}

/// GET /api/instance/:id - 특정 인스턴스 조회
pub async fn get_instance(
    Path(id): Path<String>,
    State(state): State<IPCServer>,
) -> impl IntoResponse {
    let supervisor = state.supervisor.read().await;

    match supervisor.instance_store.get(&id) {
        Some(instance) => (StatusCode::OK, Json(instance)).into_response(),
        None => {
            let error = json!({
                "error": format!("Instance not found: {}", id),
                "error_code": "instance_not_found",
            });
            (StatusCode::NOT_FOUND, Json(error)).into_response()
        }
    }
}

/// 모듈 설정 기본값에서 인스턴스 폴더로 바뀌는 자리표시자
pub const INSTANCE_DIR_PLACEHOLDER: &str = "{instance_dir}";

/// POST /api/instances - 새 인스턴스 생성
pub async fn create_instance(
    State(state): State<IPCServer>,
    Json(payload): Json<Value>,
) -> impl IntoResponse {
    let mut supervisor = state.supervisor.write().await;

    let name = payload
        .get("name")
        .and_then(|v| v.as_str())
        .ok_or("Missing name")
        .map_err(|e| (StatusCode::BAD_REQUEST, Json(json!({ "error": e }))));

    let module_name = payload
        .get("module_name")
        .and_then(|v| v.as_str())
        .ok_or("Missing module_name")
        .map_err(|e| (StatusCode::BAD_REQUEST, Json(json!({ "error": e }))));

    if let (Ok(name), Ok(module)) = (name, module_name) {
        let mut instance = crate::instance::ServerInstance::new(name, module);

        // 모듈 정보에서 process_name, default_port, install 설정 가져오기
        let module_install = if let Ok(loaded_module) = supervisor.module_loader.get_module(module) {
            instance.process_name = loaded_module.metadata.process_name.clone();
            if instance.port.is_none() {
                instance.port = loaded_module.metadata.default_port;
            }
            // rcon_port / rest_port 기본값을 모듈이 해당 프로토콜을 지원할 때만 설정
            let supported = loaded_module.metadata.protocols_supported.as_deref().unwrap_or(&[]);
            if instance.rcon_port.is_none() && supported.iter().any(|s| s == "rcon") {
                instance.rcon_port = Some(loaded_module.metadata.default_rcon_port());
            }
            if instance.rest_port.is_none() && supported.iter().any(|s| s == "rest") {
                instance.rest_port = Some(loaded_module.metadata.default_rest_port());
            }

            // settings.fields의 default 값을 module_settings에 자동 채우기
            if let Some(ref settings) = loaded_module.metadata.settings {
                for field in &settings.fields {
                    if !instance.module_settings.contains_key(&field.name) {
                        if let Some(ref default_val) = field.default {
                            let json_val = match default_val {
                                // `{instance_dir}`: 이 인스턴스 폴더 (예: 인스턴스마다 따로 두는 게임 데이터 폴더).
                                // 기존 서버를 가져오는 경우엔 그 서버가 쓰던 위치를 모듈이 고르게 비워 둔다
                                toml::Value::String(s) if s.contains(INSTANCE_DIR_PLACEHOLDER) => {
                                    if payload.get("migration_source").is_some() {
                                        serde_json::json!("")
                                    } else {
                                        let dir = supervisor.instance_store.instance_dir(&instance.id);
                                        serde_json::json!(s.replace(INSTANCE_DIR_PLACEHOLDER, &dir.to_string_lossy()))
                                    }
                                }
                                toml::Value::String(s) => serde_json::json!(s),
                                toml::Value::Integer(n) => serde_json::json!(n),
                                toml::Value::Float(f) => serde_json::json!(f),
                                toml::Value::Boolean(b) => serde_json::json!(b),
                                _ => continue,
                            };
                            instance.module_settings.insert(field.name.clone(), json_val);
                        }
                    }
                }
            }

            Some((
                loaded_module.metadata.install.clone(),
                loaded_module.entry_path(),  // 모듈 진입 파일 (마이그레이션 설정 import 시 사용)
                loaded_module.metadata.server_executable.clone(),  // 바이너리 파일명 (마이그레이션 시 executable_path 자동 계산)
            ))
        } else {
            None
        };

        // ── 모듈의 install.requires_extensions → 인스턴스 required_extensions 자동 채우기 ──
        if let Some((Some(ref install), _, _)) = module_install {
            for ext_id in &install.requires_extensions {
                if !instance.required_extensions.contains(ext_id) {
                    instance.required_extensions.push(ext_id.clone());
                }
            }
        }

        // ── 필수 익스텐션 활성화 검증 ──
        if !instance.required_extensions.is_empty() {
            let ext_mgr = state.extension_manager.read().await;
            let ready = ext_mgr.installed_and_enabled_set();
            let enabled_not_installed = ext_mgr.enabled_but_not_installed();
            drop(ext_mgr);
            let missing = instance.missing_required_extensions(&ready);
            if !missing.is_empty() {
                // 활성화되었지만 설치되지 않은 항목 구분
                let not_installed: Vec<&String> = missing.iter()
                    .filter(|id| enabled_not_installed.contains(id))
                    .collect();
                let msg = if !not_installed.is_empty() {
                    format!(
                        "Cannot create instance: required extension(s) not installed: {}. Install them in Settings → Extensions first.",
                        not_installed.iter().map(|s| s.as_str()).collect::<Vec<_>>().join(", ")
                    )
                } else {
                    format!(
                        "Cannot create instance: required extension(s) not enabled: {}. Enable them in Settings → Extensions first.",
                        missing.join(", ")
                    )
                };
                let error = json!({
                    "error": msg,
                    "error_code": "extension_required",
                    "missing_extensions": missing,
                    "not_installed": not_installed,
                });
                return (StatusCode::UNPROCESSABLE_ENTITY, Json(error)).into_response();
            }
        }

        // 선택적 필드 설정
        if let Some(path) = payload.get("executable_path").and_then(|v| v.as_str()) {
            instance.executable_path = Some(path.to_string());
        }
        if let Some(dir) = payload.get("working_dir").and_then(|v| v.as_str()) {
            instance.working_dir = Some(dir.to_string());
        }
        // migration_source → working_dir (마이그레이션: 기존 서버 디렉토리 직접 연결)
        let migration_mode = if let Some(src) = payload.get("migration_source").and_then(|v| v.as_str()) {
            if instance.working_dir.is_none() {
                instance.working_dir = Some(src.to_string());
            }
            true
        } else {
            false
        };
        if let Some(port) = payload.get("port").and_then(|v| v.as_u64()) {
            instance.port = Some(port as u16);
        } else if !migration_mode {
            // 같은 게임을 여러 개 만들면 기본 포트가 겹친다: 다른 인스턴스와 겹치지 않게 옮긴다
            let port_fields_of = |module: &str| -> Vec<String> {
                supervisor.module_loader.get_module(module).ok()
                    .and_then(|m| m.metadata.settings.as_ref().map(|s| s.port_fields().into_iter().map(str::to_string).collect()))
                    .unwrap_or_default()
            };
            let mut taken = std::collections::HashSet::new();
            for other in supervisor.instance_store.list() {
                let fields = port_fields_of(&other.module_name);
                let fields: Vec<&str> = fields.iter().map(String::as_str).collect();
                taken.extend(crate::validator::instance_ports(other, &fields).into_iter().map(|(_, p)| p));
            }
            let fields = port_fields_of(module);
            let fields: Vec<&str> = fields.iter().map(String::as_str).collect();
            if let Some(delta) = crate::validator::shift_ports_clear_of(&mut instance, &fields, &taken) {
                tracing::info!("Instance '{}': ports moved by +{} to stay clear of other instances", instance.name, delta);
            }
        }

        let id = instance.id.clone();
        let instance_name = instance.name.clone();
        let module_name_owned = module.to_string();
        let instance_dir = supervisor.instance_store.instance_dir(&id);

        // 인스턴스 저장 (프로비저닝은 비동기로 수행)
        match supervisor.instance_store.add(instance.clone()) {
            Ok(_) => {
                // ── Extension hook: server.post_create ──
                if !migration_mode {
                    // ── Native provisioning: 서버 바이너리 설치 ──
                    // install 설정이 있는 모듈만 프로비저닝 (download / 익스텐션 방식)
                    let has_install = module_install.as_ref()
                        .and_then(|(install, _, _)| install.as_ref())
                        .is_some();

                    if has_install {
                        let install_config = module_install.as_ref()
                            .and_then(|(install, _, _)| install.clone())
                            .unwrap();

                        // ── 설치 방식 익스텐션이 읽을 extension_data (신규 생성) ──
                        instance.apply_install_extension_data(&install_config.method, &install_config.extension_fields());
                        // 인스턴스 재저장 (extension_data 업데이트)
                        let _ = supervisor.instance_store.update(&id, instance.clone());

                        let tracker = state.provision_tracker.clone();
                        let supervisor_arc = state.supervisor.clone();

                        // install_dir 결정: instance_dir / install_subdir (기본 "server")
                        let install_subdir = install_config.install_subdir.clone()
                            .unwrap_or_else(|| "server".to_string());
                        let install_dir = instance_dir.join(&install_subdir);
                        let install_dir_str = install_dir.to_string_lossy().to_string();

                        // working_dir가 아직 설정되지 않았으면 install_dir로 설정
                        if instance.working_dir.is_none() {
                            instance.working_dir = Some(install_dir_str.clone());
                            // 인스턴스 재저장 (working_dir 업데이트)
                            let _ = supervisor.instance_store.update(&id, instance.clone());
                        }

                        // 초기 프로비저닝 상태 등록
                        let provision_label = match install_config.method.as_str() {
                            "manual" | "download" => "download".to_string(),
                            other => other.to_string(),
                        };
                        tracker.mark_creating(&instance_name);
                        tracker.update(&instance_name, crate::ipc::ProvisionProgress {
                            step: 0,
                            total: 1,
                            label: provision_label.clone(),
                            message: "Installing server...".to_string(),
                            done: false,
                            error: None,
                            percent: Some(0),
                            steps: None,
                        });

                        let tracker_done = tracker.clone();
                        let name_done = instance_name.clone();
                        let id_done = id.clone();

                        // Drop supervisor lock before spawning background task
                        drop(supervisor);

                        tokio::spawn(async move {
                            // ── Phase 1: supervisor lock을 최소 시간만 잡고 모듈 경로 복사 ──
                            let module_path = {
                                let sup = supervisor_arc.read().await;
                                match sup.module_loader.get_module(&module_name_owned) {
                                    Ok(m) => m.entry_path(),
                                    Err(e) => {
                                        tracker_done.update(&name_done, crate::ipc::ProvisionProgress {
                                            step: 0, total: 1,
                                            label: "done".to_string(),
                                            message: "Provisioning failed".to_string(),
                                            done: true,
                                            error: Some(format!("Module not found: {}", e)),
                                            percent: Some(100),
                                            steps: None,
                                        });
                                        return;
                                    }
                                }
                            };
                            // supervisor read lock 해제됨 — 이후 플러그인 호출이
                            // 모니터 태스크의 write lock을 차단하지 않음

                            // ── Phase 2: 플러그인 호출 (lock 없이 실행) ──
                            // lifecycle.py install_server 호출 (SteamCMD / download 모두 처리)
                            let result = {
                                // download 방식 모듈은 먼저 list_versions로 최신 버전 조회
                                let version = if install_config.method == "download" {
                                    match crate::plugin::run_plugin(
                                        &module_path, "list_versions", serde_json::json!({}),
                                    ).await {
                                        Ok(ver_result) => {
                                            ver_result.get("latest")
                                                .and_then(|l| l.get("release"))
                                                .and_then(|r| r.as_str())
                                                .map(|s| s.to_string())
                                                .or_else(|| {
                                                    ver_result.get("latest_release")
                                                        .and_then(|r| r.as_str())
                                                        .map(|s| s.to_string())
                                                })
                                        }
                                        Err(e) => {
                                            tracing::warn!("Failed to fetch versions for '{}': {}", module_name_owned, e);
                                            None
                                        }
                                    }
                                } else {
                                    None // 익스텐션 방식은 version 불필요
                                };

                                // download 방식인데 버전을 파악하지 못한 경우 에러
                                if install_config.method == "download" && version.is_none() {
                                    tracker_done.update(&name_done, crate::ipc::ProvisionProgress {
                                        step: 0, total: 1,
                                        label: "done".to_string(),
                                        message: "Provisioning failed".to_string(),
                                        done: true,
                                        error: Some("Failed to determine latest server version".to_string()),
                                        percent: Some(100),
                                        steps: None,
                                    });
                                    return;
                                }

                                let mut config = serde_json::json!({
                                    "install_dir": &install_dir_str,
                                    "accept_eula": true,
                                });
                                if let Some(ref ver) = version {
                                    config["version"] = serde_json::json!(ver);
                                }

                                let tracker_progress = tracker_done.clone();
                                let name_progress = name_done.clone();
                                let on_progress = move |prog: crate::plugin::ExtensionProgress| {
                                    let pct = prog.percent.unwrap_or(0);
                                    let msg = prog.message.clone().unwrap_or_default();
                                    tracker_progress.update(&name_progress, crate::ipc::ProvisionProgress {
                                        step: prog.step.unwrap_or(0),
                                        total: prog.total.unwrap_or(1),
                                        label: prog.label.unwrap_or_else(|| provision_label.clone()),
                                        message: msg,
                                        done: false,
                                        error: None,
                                        percent: Some(pct),
                                        steps: None,
                                    });
                                };

                                crate::plugin::run_plugin_with_progress_and_timeout(
                                    &module_path, "install_server", config, on_progress,
                                    1800, // 30분 — 대용량 다운로드 허용
                                ).await
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

                                        {
                                            let mut sup = supervisor_arc.write().await;
                                            if let Some(inst) = sup.instance_store.get(&id_done).cloned() {
                                                let mut updated = inst;
                                                if let Some(ref exe) = exe_path {
                                                    if !exe.is_empty() {
                                                        updated.executable_path = Some(exe.clone());
                                                    }
                                                }
                                                if let Some(ref ip) = install_path {
                                                    updated.working_dir = Some(ip.clone());
                                                }
                                                let _ = sup.instance_store.update(&id_done, updated);
                                            }
                                        }

                                        tracing::info!("Native provisioning completed for '{}'", name_done);
                                        tracker_done.update(&name_done, crate::ipc::ProvisionProgress {
                                            step: 0, total: 1,
                                            label: "done".to_string(),
                                            message: "Server installed successfully".to_string(),
                                            done: true,
                                            error: None,
                                            percent: Some(100),
                                            steps: None,
                                        });

                                        // 5초 후 tracker 자동 정리
                                        let tracker_cleanup = tracker_done.clone();
                                        let name_cleanup = name_done.clone();
                                        tokio::spawn(async move {
                                            tokio::time::sleep(std::time::Duration::from_secs(5)).await;
                                            tracker_cleanup.remove(&name_cleanup);
                                        });
                                    } else {
                                        let err_msg = val.get("message")
                                            .and_then(|v| v.as_str())
                                            .unwrap_or("Installation failed")
                                            .to_string();
                                        tracing::warn!("Native provisioning failed for '{}': {}", name_done, err_msg);
                                        tracker_done.update(&name_done, crate::ipc::ProvisionProgress {
                                            step: 0, total: 1,
                                            label: "done".to_string(),
                                            message: "Provisioning failed".to_string(),
                                            done: true,
                                            error: Some(err_msg),
                                            percent: Some(100),
                                            steps: None,
                                        });
                                    }
                                }
                                Err(e) => {
                                    tracing::warn!("Native provisioning error for '{}': {}", name_done, e);
                                    tracker_done.update(&name_done, crate::ipc::ProvisionProgress {
                                        step: 0, total: 1,
                                        label: "done".to_string(),
                                        message: "Provisioning failed".to_string(),
                                        done: true,
                                        error: Some(e.to_string()),
                                        percent: Some(100),
                                        steps: None,
                                    });
                                }
                            }
                        });

                        let response = json!({
                            "success": true,
                            "id": id,
                            "provisioning": true,
                        });
                        (StatusCode::CREATED, Json(response)).into_response()
                    } else {
                        let response = json!({ "success": true, "id": id });
                        (StatusCode::CREATED, Json(response)).into_response()
                    }
                } else {
                    // 마이그레이션: 기존 디렉토리에 직접 연결, 추가 설치 없음

                    // ── 설치 방식 익스텐션이 읽을 extension_data (마이그레이션) ──
                    // hook 조건(`instance.ext_data.install_method_<방식>`)이 맞게 동작하도록
                    if let Some((Some(ref install), _, _)) = module_install {
                        instance.apply_install_extension_data(&install.method, &install.extension_fields());
                    }

                    // ── executable_path 자동 계산 ──
                    // migration_source(=working_dir)와 server_executable(바이너리 파일명)을 결합
                    // 프론트엔드가 모듈 기본값을 보낼 수 있으므로, migration에서는 항상 재계산
                    if let (Some(ref work_dir), Some((_, _, Some(ref exe_name)))) = (&instance.working_dir, &module_install) {
                        let resolved = std::path::Path::new(work_dir).join(exe_name);
                        if resolved.exists() {
                            instance.executable_path = Some(resolved.to_string_lossy().to_string());
                            tracing::info!("Migration: executable_path auto-resolved to '{}'", resolved.display());
                        } else {
                            tracing::warn!("Migration: expected executable '{}' not found in '{}'", exe_name, work_dir);
                        }
                    }

                    // ── 기존 INI 설정 import ──
                    // 모듈 호출은 데몬 락 밖에서 (C12). 필요한 값만 챙기고 락을 놓는다.
                    let (credential_map, setting_types) = supervisor.module_loader.get_module(&module_name_owned)
                        .map(|m| (m.metadata.credential_map.clone(), m.metadata.setting_types()))
                        .unwrap_or_default();
                    drop(supervisor);
                    if let Some((_, ref lifecycle_path, ref _server_exe)) = module_install {
                        let import_config = json!({
                            "working_dir": instance.working_dir,
                        });
                        match crate::plugin::run_plugin(lifecycle_path, "import_settings", import_config).await {
                            Ok(result) => {
                                if result.get("success").and_then(|v| v.as_bool()).unwrap_or(false) {
                                    if let Some(settings) = result.get("settings").and_then(|v| v.as_object()) {
                                        instance.apply_imported_settings(settings, &credential_map, &setting_types);
                                        tracing::info!("Migration import_settings: {} settings imported for '{}'", settings.len(), instance_name);
                                    }
                                }
                            }
                            Err(e) => {
                                tracing::warn!("Migration import_settings failed for '{}': {} (non-fatal)", instance_name, e);
                            }
                        }
                    }
                    // 마이그레이션 변경사항 저장 (executable_path, import된 설정 등)
                    let _ = state.supervisor.write().await.instance_store.update(&id, instance.clone());
                    let response = json!({ "success": true, "id": id });
                    (StatusCode::CREATED, Json(response)).into_response()
                }
            }
            Err(e) => {
                let error = json!({ "error": format!("Failed to create instance: {}", e) });
                (StatusCode::INTERNAL_SERVER_ERROR, Json(error)).into_response()
            }
        }
    } else {
        let error = json!({ "error": "Invalid request" });
        (StatusCode::BAD_REQUEST, Json(error)).into_response()
    }
}

/// DELETE /api/provision-progress/:name - 프로비저닝 상태 클리어 (에러 dismiss용)
/// 신규 생성 프로비저닝이 실패한 상태였으면 인스턴스도 롤백(삭제)한다.
/// 기존 인스턴스의 업데이트 실패는 진행 표시만 지운다.
pub async fn dismiss_provision_progress(
    Path(name): Path<String>,
    State(state): State<IPCServer>,
) -> impl IntoResponse {
    // 프로비저닝 에러가 있었는지 확인
    let had_error = state
        .provision_tracker
        .get(&name)
        .map(|p| p.error.is_some())
        .unwrap_or(false);
    let was_creating = state.provision_tracker.take_creating(&name);

    state.provision_tracker.remove(&name);

    // 신규 생성이 실패했을 때만 롤백 — 인스턴스 삭제
    let rolled_back = had_error && was_creating;
    if rolled_back {
        let mut supervisor = state.supervisor.write().await;
        if let Some(id) = supervisor
            .instance_store
            .list()
            .iter()
            .find(|i| i.name == name)
            .map(|i| i.id.clone())
        {
            if let Err(e) = supervisor.instance_store.remove(&id) {
                tracing::warn!("Failed to rollback instance '{}': {}", name, e);
            } else {
                tracing::info!(
                    "Provisioning rollback: removed instance '{}' (id={})",
                    name,
                    id
                );
            }
        }
    }

    (StatusCode::OK, Json(json!({ "success": true, "rolled_back": rolled_back }))).into_response()
}

/// GET /api/provision-progress/:name - 프로비저닝 진행 상태 (서버 이름 기준)
pub async fn get_provision_progress(
    Path(name): Path<String>,
    State(state): State<IPCServer>,
) -> impl IntoResponse {
    if let Some(progress) = state.provision_tracker.get(&name) {
        let mut resp = json!({
            "active": true,
            "step": progress.step,
            "total": progress.total,
            "label": progress.label,
            "message": progress.message,
            "done": progress.done,
            "error": progress.error,
        });
        if let Some(pct) = progress.percent {
            resp["percent"] = json!(pct);
        }
        if let Some(ref steps) = progress.steps {
            resp["steps"] = json!(steps);
        }
        (StatusCode::OK, Json(resp)).into_response()
    } else {
        (StatusCode::OK, Json(json!({
            "active": false,
        }))).into_response()
    }
}

/// DELETE /api/instance/:id - 인스턴스 삭제
pub async fn delete_instance(
    Path(id): Path<String>,
    State(state): State<IPCServer>,
) -> impl IntoResponse {
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
    let managed = supervisor.managed_store.remove(&id).await;
    let _ = supervisor.tracker.untrack(&id);
    drop(supervisor);
    if let Some(pid) = running_pid {
        tracing::info!("Instance '{}' is running, force-stopping before delete...", instance.name);
        // Managed process가 있으면 그 PID, 아니면 추적하던 PID를 강제 종료
        match managed.as_ref() {
            Some(m) if m.is_running() => m.force_kill(),
            Some(_) => {}
            None => {
                let _ = crate::supervisor::process::force_kill_pid(pid);
            }
        }
        if managed.as_ref().is_none_or(|m| m.is_running()) {
            // 프로세스 종료 대기
            tokio::time::sleep(std::time::Duration::from_secs(2)).await;
        }
    }

    // ── Extension hook: server.pre_delete ──
    {
        let ext_mgr = state.extension_manager.clone();
        let ctx = serde_json::json!({
            "instance_id": &id,
            "instance_name": &instance.name,
            "module": &instance.module_name,
            "extension_data": &instance.extension_data,
        });
        let results = crate::extension::run_hook(&ext_mgr, "server.pre_delete", ctx).await;
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

    // ── 디렉토리 삭제 (Windows 파일 잠금 대비 재시도) ──
    let mut last_err = None;
    for attempt in 0..3 {
        let removed = state.supervisor.write().await.instance_store.remove(&id);
        match removed {
            Ok(_) => {
                let response = json!({ "success": true });
                return (StatusCode::OK, Json(response)).into_response();
            }
            Err(e) => {
                tracing::warn!(
                    "Instance '{}' delete attempt {}/3 failed: {}",
                    instance.name, attempt + 1, e
                );
                last_err = Some(e);
                if attempt < 2 {
                    // 재시도 전 대기 — 파일 잠금 해제 기다림
                    tokio::time::sleep(std::time::Duration::from_secs(2)).await;
                }
            }
        }
    }

    let err_msg = last_err.map(|e| format!("{}", e)).unwrap_or_else(|| "Unknown error".to_string());
    let error = json!({ "success": false, "error": format!("Failed to delete instance: {}", err_msg) });
    (StatusCode::INTERNAL_SERVER_ERROR, Json(error)).into_response()
}

/// POST /api/instance/:id/rename — 이름 바꾸기 (`{name}`). 실행 중이면 409 `server_running`,
/// 같은 이름이 있으면 409 `name_taken`. 응답 `{success, id, name, previous}`
pub async fn rename_instance(
    State(state): State<IPCServer>,
    Path(id): Path<String>,
    Json(body): Json<serde_json::Value>,
) -> impl IntoResponse {
    let fail = |status: StatusCode, error: String, code: &str| {
        (status, Json(json!({ "success": false, "error": error, "error_code": code }))).into_response()
    };
    let Some(new_name) = body.get("name").and_then(|v| v.as_str()).map(str::trim).filter(|n| !n.is_empty()) else {
        return fail(StatusCode::BAD_REQUEST, "Missing 'name'".into(), "invalid_request");
    };
    let mut supervisor = state.supervisor.write().await;
    let Some(mut instance) = supervisor.instance_store.get(&id).cloned() else {
        return fail(StatusCode::NOT_FOUND, format!("Instance not found: {}", id), "not_found");
    };
    if instance.name == new_name {
        return Json(json!({ "success": true, "id": id, "name": new_name, "previous": new_name })).into_response();
    }
    if supervisor.instance_store.list().iter().any(|i| i.id != id && i.name == new_name) {
        return fail(StatusCode::CONFLICT, format!("An instance named '{}' already exists", new_name), "name_taken");
    }
    // 실행 중인 서버는 이름으로도 추적되므로 꺼진 뒤에만 바꾼다
    let running = supervisor.tracker.get_pid(&id).is_ok() || supervisor.managed_store.running_instance_ids().await.contains(&id);
    if running {
        return fail(StatusCode::CONFLICT, "Stop the server before renaming it".into(), "server_running");
    }
    let previous = std::mem::replace(&mut instance.name, new_name.to_string());
    if let Err(e) = supervisor.instance_store.update(&id, instance) {
        return fail(StatusCode::INTERNAL_SERVER_ERROR, e.to_string(), "internal");
    }
    Json(json!({ "success": true, "id": id, "name": new_name, "previous": previous })).into_response()
}

/// PATCH /api/instance/:id - 인스턴스 설정 업데이트
pub async fn update_instance_settings(
    State(state): State<IPCServer>,
    Path(id): Path<String>,
    Json(settings): Json<serde_json::Value>,
) -> impl IntoResponse {
    let mut supervisor = state.supervisor.write().await;

    // 인스턴스 찾기
    let instance = match supervisor.instance_store.get(&id) {
        Some(inst) => inst,
        None => {
            let error = json!({ "error": "Instance not found" });
            return (StatusCode::NOT_FOUND, Json(error)).into_response();
        }
    };

    // ── 설정값 타입/범위 스키마 검증 ──
    if let Some(settings_obj) = settings.as_object() {
        if let Ok(module) = supervisor.module_loader.get_module(&instance.module_name) {
            if let Some(ref settings_meta) = module.metadata.settings {
                // 부분 변경이므로 보낸 필드만 검증한다
                let errors = crate::validator::validate_settings_patch(
                    &settings_meta.fields,
                    settings_obj,
                );
                if !errors.is_empty() {
                    let error_details: Vec<String> = errors.iter().map(|e| e.to_string()).collect();
                    let fields: Vec<Value> = errors
                        .iter()
                        .map(|e| json!({ "field": e.field, "type": e.error_type, "message": e.message }))
                        .collect();
                    tracing::warn!(
                        "Settings validation failed for instance {}: {:?}",
                        id, error_details
                    );
                    let error = json!({
                        "error": "validation_failed",
                        "error_code": "validation_failed",
                        "message": "Settings validation failed",
                        "details": error_details,
                        "fields": fields,
                    });
                    return (StatusCode::BAD_REQUEST, Json(error)).into_response();
                }
            }
        }
    }

    // 설정값 업데이트
    let mut updated = instance.clone();

    // working_dir이 null인데 executable_path가 있으면 자동 보정
    if updated.working_dir.is_none() {
        if let Some(ref exe_path) = updated.executable_path {
            if let Some(parent) = std::path::Path::new(exe_path).parent() {
                updated.working_dir = Some(parent.to_string_lossy().to_string());
                tracing::info!(
                    "Auto-inferred working_dir to {} from existing executable_path",
                    parent.display()
                );
            }
        }
    }

    // 하드코딩된 공통 필드 목록
    let known_fields: std::collections::HashSet<&str> = [
        "port",
        "rcon_port",
        "rcon_password",
        "rest_host",
        "rest_port",
        "rest_username",
        "rest_password",
        "executable_path",
        "protocol_mode",
        "server_version",
        "extension_data",
    ]
    .iter()
    .cloned()
    .collect();

    // common settings
    // port: 숫자 또는 문자열 수용
    if let Some(port_value) = settings.get("port") {
        match port_value {
            serde_json::Value::Number(n) => {
                if let Some(port) = n.as_u64() {
                    updated.port = Some(port as u16);
                }
            }
            serde_json::Value::String(s) => {
                if let Ok(port) = s.parse::<u16>() {
                    updated.port = Some(port);
                }
            }
            _ => {}
        }
    }

    // rcon_port: 숫자 또는 문자열 수용
    if let Some(rcon_port_value) = settings.get("rcon_port") {
        match rcon_port_value {
            serde_json::Value::Number(n) => {
                if let Some(rcon_port) = n.as_u64() {
                    updated.rcon_port = Some(rcon_port as u16);
                }
            }
            serde_json::Value::String(s) => {
                if let Ok(rcon_port) = s.parse::<u16>() {
                    updated.rcon_port = Some(rcon_port);
                }
            }
            _ => {}
        }
    }

    if let Some(rcon_password) = settings.get("rcon_password").and_then(|v| v.as_str()) {
        updated.rcon_password = Some(rcon_password.to_string());
    }

    // managed_start ↔ RCON 연동:
    //   managed=true  → enable_rcon=false (stdin으로 제어, RCON 불필요)
    //   managed=false → enable_rcon=true  (RCON이 유일한 제어 수단)
    let managed_start = settings.get("managed_start").and_then(|v| match v {
        serde_json::Value::Bool(b) => Some(*b),
        serde_json::Value::String(s) => Some(s == "true"),
        _ => None,
    });
    let enable_rcon = match managed_start {
        Some(true) => Some(false),  // managed → RCON 비활성화
        Some(false) => Some(true),  // native → RCON 강제 활성화
        None => {
            // managed_start가 전송되지 않은 경우: 기존 enable_rcon 값 유지
            settings.get("enable_rcon").and_then(|v| match v {
                serde_json::Value::Bool(b) => Some(*b),
                serde_json::Value::String(s) => Some(s == "true"),
                _ => None,
            })
        }
    };

    // NOTE: RCON/REST 비밀번호 자동 생성은 인스턴스 실행 시점에 supervisor에서 처리합니다.

    if let Some(rest_host) = settings.get("rest_host").and_then(|v| v.as_str()) {
        updated.rest_host = Some(rest_host.to_string());
    }
    if let Some(rest_port_value) = settings.get("rest_port") {
        match rest_port_value {
            serde_json::Value::Number(n) => {
                if let Some(rest_port) = n.as_u64() {
                    updated.rest_port = Some(rest_port as u16);
                }
            }
            serde_json::Value::String(s) => {
                if let Ok(rest_port) = s.parse::<u16>() {
                    updated.rest_port = Some(rest_port);
                }
            }
            _ => {}
        }
    }
    if let Some(rest_username) = settings.get("rest_username").and_then(|v| v.as_str()) {
        updated.rest_username = Some(rest_username.to_string());
    }
    if let Some(rest_password) = settings.get("rest_password").and_then(|v| v.as_str()) {
        updated.rest_password = Some(rest_password.to_string());
    }
    // credential_map 범용 역동기화: module_settings 키 → daemon first-class 필드
    // 예: Palworld credential_map { rest_password = "AdminPassword" }
    //   → module_settings의 "admin_password" (snake_case) 값을 instance.rest_password에 동기화
    if let Ok(module) = supervisor.module_loader.get_module(&instance.module_name) {
        for (daemon_key, game_key) in &module.metadata.credential_map {
            // game_key를 snake_case로 변환하여 module_settings에서 탐색
            let snake_key = game_key
                .chars()
                .enumerate()
                .fold(String::new(), |mut acc, (i, c)| {
                    if c.is_uppercase() && i > 0 {
                        acc.push('_');
                    }
                    acc.push(c.to_lowercase().next().unwrap());
                    acc
                });
            let value = settings.get(&snake_key)
                .or_else(|| settings.get(game_key.as_str()))
                .and_then(|v| v.as_str());
            if let Some(pwd) = value {
                if !pwd.is_empty() {
                    match daemon_key.as_str() {
                        "rest_password" if updated.rest_password.as_deref().unwrap_or("").is_empty() => {
                            updated.rest_password = Some(pwd.to_string());
                        }
                        "rcon_password" if updated.rcon_password.as_deref().unwrap_or("").is_empty() => {
                            updated.rcon_password = Some(pwd.to_string());
                        }
                        _ => {}
                    }
                }
            }
        }
    }

    if let Some(executable_path) = settings.get("executable_path").and_then(|v| v.as_str()) {
        // executable_path는 보통 모듈의 server_executable + working_dir에서 자동 해석되지만,
        // 명시적으로 설정하면 override로 저장합니다.
        updated.executable_path = Some(executable_path.to_string());
        // working_dir이 미설정이면 executable_path의 부모 디렉토리로 자동 설정
        if updated.working_dir.is_none() {
            if let Some(parent) = std::path::Path::new(executable_path).parent() {
                updated.working_dir = Some(parent.to_string_lossy().to_string());
                tracing::info!(
                    "Auto-set working_dir to {} from executable_path",
                    parent.display()
                );
            }
        }
    }

    // protocol_mode 설정 (rest 또는 rcon)
    if let Some(protocol_mode) = settings.get("protocol_mode").and_then(|v| v.as_str()) {
        updated.protocol_mode = protocol_mode.to_string();
    }

    // server_version 설정
    if let Some(version) = settings.get("server_version").and_then(|v| v.as_str()) {
        updated.server_version = Some(version.to_string());
    }

    // ── 범용 extension_data 갱신 ──
    // 프론트엔드에서 { "extension_data": { "key": value, ... } } 형태로 전달
    // null 값은 해당 키 삭제, 빈 문자열도 삭제 처리
    if let Some(ext_updates) = settings.get("extension_data") {
        if let Some(obj) = ext_updates.as_object() {
            for (key, value) in obj {
                match value {
                    serde_json::Value::Null => {
                        updated.extension_data.remove(key);
                    }
                    serde_json::Value::String(s) if s.is_empty() => {
                        updated.extension_data.remove(key);
                    }
                    serde_json::Value::String(s) => {
                        // 숫자로 변환 가능하면 Number로 저장 (CPU 제한 등)
                        if let Ok(n) = s.parse::<f64>() {
                            updated.extension_data.insert(key.clone(), json!(n));
                        } else {
                            updated.extension_data.insert(key.clone(), value.clone());
                        }
                    }
                    _ => {
                        updated.extension_data.insert(key.clone(), value.clone());
                    }
                }
            }
        }
    }

    // 동적 모듈 설정 저장 (하드코딩 필드 이외의 모든 설정을 module_settings에 저장)
    if let Some(obj) = settings.as_object() {
        for (key, value) in obj {
            if !known_fields.contains(key.as_str()) {
                updated.module_settings.insert(key.clone(), value.clone());
            }
        }
    }

    tracing::info!(
        "Updating instance {} with settings: port={:?}, rcon_port={:?}, executable_path={:?}, protocol_mode={}, module_settings_count={}",
        id,
        updated.port,
        updated.rcon_port,
        updated.executable_path,
        updated.protocol_mode,
        updated.module_settings.len()
    );

    // 모든 설정을 server.properties에 동기화 (configure lifecycle 호출)
    let mut props_sync = serde_json::Map::new();
    if let Some(obj) = settings.as_object() {
        for (key, value) in obj {
            // protocol_mode, executable_path 등은 server.properties에 관련 없으므로 제외
            if key == "protocol_mode"
                || key == "executable_path"
                || key == "server_version"
                || key == "rest_host"
                || key == "rest_port"
                || key == "rest_username"
                || key == "rest_password"
                || key == "java_path"
                || key == "ram"
                || key == "use_aikar_flags"
                || key == "managed_start"
                || key == "graceful_stop"
                || key == "extension_data"
            {
                continue;
            }
            props_sync.insert(key.clone(), value.clone());
        }
    }

    // managed_start ↔ RCON: enable_rcon을 props_sync에 주입
    if let Some(rcon_on) = enable_rcon {
        props_sync.insert("enable_rcon".to_string(), json!(rcon_on));
    }
    // 인스턴스에 저장된 RCON 비밀번호가 있으면 props_sync에도 반영
    if let Some(pwd) = &updated.rcon_password {
        if !props_sync.contains_key("rcon_password") {
            props_sync.insert("rcon_password".to_string(), json!(pwd));
        }
    }

    // 저장
    let updated_clone = updated.clone();
    if let Err(e) = supervisor.instance_store.update(&id, updated) {
        let error = json!({ "error": format!("Failed to update instance: {}", e) });
        return (StatusCode::INTERNAL_SERVER_ERROR, Json(error)).into_response();
    }

    // 익스텐션 hook에 넘길 값을 챙기고 락을 놓는다. 아래 모듈·익스텐션 호출은 락 밖에서 (C12)
    let instance_dir = supervisor.instance_store.instance_dir(&id);
    // 모듈의 모든 익스텐션 설정을 범용으로 전달 (특정 익스텐션 이름 참조 없음)
    let module_extensions = supervisor.module_loader.get_module(&updated_clone.module_name)
        .ok()
        .map(|m| serde_json::to_value(&m.metadata.extensions).unwrap_or_default())
        .unwrap_or_else(|| serde_json::json!({}));
    drop(supervisor);

    // server.properties 동기화 (변경된 항목이 있을 때만)
    if !props_sync.is_empty() {
        tracing::info!(
            "Syncing settings to server.properties for instance {}: {:?}",
            id,
            props_sync
        );
        let props_value = Value::Object(props_sync);
        match crate::supervisor::Supervisor::manage_properties(&state.supervisor, &id, "write", Some(props_value)).await {
            Ok(_) => tracing::info!("server.properties synced successfully for instance {}", id),
            Err(e) => tracing::warn!(
                "Failed to sync server.properties for instance {}: {}",
                id,
                e
            ),
        }
    }

    // 인스턴스 설정 변경 시 Extension hook 디스패치 (extension_data가 있는 경우)
    // instance_dir + 모듈 익스텐션 설정을 범용으로 전달 → 각 extension이 자체 판단
    if !updated_clone.extension_data.is_empty() {
        let ext_mgr = state.extension_manager.clone();
        let ctx = serde_json::json!({
            "instance_id": &id,
            "instance_dir": instance_dir.to_string_lossy(),
            "instance": serde_json::to_value(&updated_clone).unwrap_or_default(),
            "module_extensions": module_extensions,
            "extension_data": &updated_clone.extension_data,
            "settings": &settings,
        });
        let _results = crate::extension::run_hook(&ext_mgr, "server.settings_changed", ctx).await;
    }

    // 게임 설정 파일의 비밀번호·포트가 바뀌었을 수 있다
    state.credentials.invalidate(&id);
    (StatusCode::OK, Json(json!({ "success": true }))).into_response()
}

/// GET /api/instance/:id/check-update - 게임 서버 업데이트 확인
///
/// 설치 방식 익스텐션(`install_method_<방식>`)으로 설치된 인스턴스에 대해
/// server.check_update hook을 디스패치하여 buildid 비교 결과를 반환합니다.
pub async fn check_server_update(
    Path(id): Path<String>,
    State(state): State<IPCServer>,
) -> impl IntoResponse {
    let supervisor = state.supervisor.read().await;

    let instance = match supervisor.instance_store.get(&id) {
        Some(inst) => inst.clone(),
        None => {
            return (
                StatusCode::NOT_FOUND,
                Json(json!({
                    "error": format!("Instance not found: {}", id),
                    "error_code": "instance_not_found",
                })),
            )
                .into_response();
        }
    };

    // extension_data 가 비어 있으면 hook 이 동작하지 않으므로 조기 반환
    if instance.extension_data.is_empty() {
        return (
            StatusCode::OK,
            Json(json!({
                "update_available": false,
                "reason": "no_extension_data",
            })),
        )
            .into_response();
    }

    let instance_dir = supervisor.instance_store.instance_dir(&id);

    // 모듈에서 install config 가져오기 (install_subdir, app_id 등)
    let install_config = supervisor
        .module_loader
        .get_module(&instance.module_name)
        .ok()
        .and_then(|m| m.metadata.install.clone());

    // install_dir 계산: instance_dir / install_subdir
    let install_subdir = install_config
        .as_ref()
        .and_then(|cfg| cfg.install_subdir.clone())
        .unwrap_or_else(|| "server".to_string());
    let install_dir = instance_dir.join(&install_subdir);
    let install_dir_str = install_dir.to_string_lossy().to_string();

    // app_id: extension_data(`<방식>_app_id`)에서 우선, 없으면 module config에서
    let app_id = install_config
        .as_ref()
        .and_then(|cfg| instance.extension_data.get(&format!("{}_app_id", cfg.method)))
        .and_then(|v| v.as_u64())
        .or_else(|| install_config.as_ref().and_then(|cfg| cfg.app_id.map(|id| id as u64)));

    // supervisor lock 해제 (hook 실행 중 필요 없음)
    drop(supervisor);

    let ctx = json!({
        "instance_id": &id,
        "instance_name": &instance.name,
        "module": &instance.module_name,
        "instance_dir": instance_dir.to_string_lossy(),
        "install_dir": &install_dir_str,
        "app_id": app_id,
        "extension_data": &instance.extension_data,
    });

    let results = crate::extension::run_hook(&state.extension_manager, "server.check_update", ctx).await;

    // 첫 번째 성공 결과를 클라이언트로 전달
    for (_ext_id, result) in &results {
        if let Ok(val) = result {
            return (StatusCode::OK, Json(val.clone())).into_response();
        }
    }

    // hook이 등록되지 않았거나 모두 condition 불일치
    (
        StatusCode::OK,
        Json(json!({
            "update_available": false,
            "reason": "no_hook_handled",
        })),
    )
        .into_response()
}

/// POST /api/instance/:id/apply-update - 게임 서버 업데이트 적용
///
/// SteamCMD install (== update)을 재실행하여 서버를 최신 빌드로 업데이트합니다.
/// 서버가 실행 중이면 업데이트할 수 없습니다.
pub async fn apply_server_update(
    Path(id): Path<String>,
    State(state): State<IPCServer>,
) -> impl IntoResponse {
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
        Err(r) => r,
    }
}

/// 업데이트를 백그라운드로 시작한다. 작업은 성공 여부를 돌려준다 (예약 업데이트가 기다린다)
#[allow(clippy::result_large_err)]
pub async fn begin_server_update(state: &IPCServer, id: &str) -> Result<tokio::task::JoinHandle<bool>, axum::response::Response> {
    let supervisor = state.supervisor.read().await;

    let instance = match supervisor.instance_store.get(id) {
        Some(inst) => inst.clone(),
        None => {
            return Err((
                StatusCode::NOT_FOUND,
                Json(json!({
                    "error": format!("Instance not found: {}", id),
                    "error_code": "instance_not_found",
                })),
            )
                .into_response());
        }
    };

    // 실행 중이면 업데이트 불가
    if supervisor.tracker.get_pid(id).is_ok() {
        return Err((
            StatusCode::CONFLICT,
            Json(json!({
                "error": "Cannot update while server is running. Stop the server first.",
                "error_code": "server_running",
            })),
        )
            .into_response());
    }

    // 모듈에서 install config 가져오기
    let module_info = supervisor
        .module_loader
        .get_module(&instance.module_name)
        .ok()
        .map(|m| (m.metadata.install.clone(), m.path.clone()));

    let (install_config, _module_path) = match module_info {
        Some((Some(cfg), path)) => (cfg, path),
        _ => {
            return Err((
                StatusCode::BAD_REQUEST,
                Json(json!({
                    "error": "Module install configuration not found.",
                    "error_code": "no_install_config",
                })),
            )
                .into_response());
        }
    };

    // 익스텐션이 설치한 인스턴스만 이 경로로 업데이트한다 (설치 방식 슬롯)
    let method = install_config.method.clone();
    let by_extension = !crate::instance::BUILTIN_INSTALL_METHODS.contains(&method.as_str())
        && instance.ext_enabled(&crate::instance::install_method_key(&method));
    if !by_extension {
        return Err((
            StatusCode::BAD_REQUEST,
            Json(json!({
                "error": "Only instances installed by an install-method extension can be updated this way.",
                "error_code": "update_unsupported",
            })),
        )
            .into_response());
    }

    let instance_dir = supervisor.instance_store.instance_dir(id);
    let install_subdir = install_config
        .install_subdir
        .clone()
        .unwrap_or_else(|| "server".to_string());
    let install_dir = instance_dir.join(&install_subdir);
    let install_dir_str = install_dir.to_string_lossy().to_string();

    let tracker = state.provision_tracker.clone();
    let supervisor_arc = state.supervisor.clone();
    let instance_name = instance.name.clone();
    let module_name_owned = instance.module_name.clone();

    // 프로비저닝 상태 등록 (기존 인스턴스 업데이트 — 실패해도 롤백 대상이 아니다)
    tracker.take_creating(&instance_name);
    tracker.update(
        &instance_name,
        crate::ipc::ProvisionProgress {
            step: 0,
            total: 1,
            label: method.clone(),
            message: "Updating server...".to_string(),
            done: false,
            error: None,
            percent: Some(0),
            steps: None,
        },
    );

    // supervisor lock 해제 후 백그라운드 태스크 실행
    drop(supervisor);

    let tracker_done = tracker.clone();
    let name_done = instance_name.clone();

    let backup_state = state.clone();
    let backup_id = id.to_string();
    Ok(tokio::spawn(async move {
        // ── Phase 0: 업데이트 전 백업 (A2). 백업할 것이 없으면 그냥 진행, 실패하면 업데이트하지 않는다 ──
        if let Err(r) = super::backup::take_backup(&backup_state, &backup_id, crate::backup::BackupKind::PreUpdate).await {
            let reason = crate::ipc::internal::into_internal(r).await.body["error"].as_str().unwrap_or("backup failed").to_string();
            tracing::error!("Pre-update backup failed for '{}': {}", name_done, reason);
            tracker_done.update(
                &name_done,
                crate::ipc::ProvisionProgress {
                    step: 0,
                    total: 1,
                    label: "done".to_string(),
                    message: "Update failed".to_string(),
                    done: true,
                    error: Some(format!("Pre-update backup failed: {}", reason)),
                    percent: Some(100),
                    steps: None,
                },
            );
            return false;
        }

        // ── Phase 1: supervisor lock을 최소 시간만 잡고 모듈 경로 복사 ──
        let lifecycle_path = {
            let sup = supervisor_arc.read().await;
            match sup.module_loader.get_module(&module_name_owned) {
                Ok(m) => m.entry_path(),
                Err(e) => {
                    tracker_done.update(
                        &name_done,
                        crate::ipc::ProvisionProgress {
                            step: 0,
                            total: 1,
                            label: "done".to_string(),
                            message: "Update failed".to_string(),
                            done: true,
                            error: Some(format!("Module not found: {}", e)),
                            percent: Some(100),
                            steps: None,
                        },
                    );
                    return false;
                }
            }
        };
        // supervisor read lock 해제됨

        // ── Phase 2: 플러그인 호출 (lock 없이 실행) ──
        let result = {
            let config = serde_json::json!({
                "install_dir": &install_dir_str,
                "accept_eula": true,
            });

            let tracker_progress = tracker_done.clone();
            let name_progress = name_done.clone();
            let on_progress = move |prog: crate::plugin::ExtensionProgress| {
                let pct = prog.percent.unwrap_or(0);
                let msg = prog.message.clone().unwrap_or_default();
                tracker_progress.update(
                    &name_progress,
                    crate::ipc::ProvisionProgress {
                        step: prog.step.unwrap_or(0),
                        total: prog.total.unwrap_or(1),
                        label: prog.label.unwrap_or_else(|| method.clone()),
                        message: msg,
                        done: false,
                        error: None,
                        percent: Some(pct),
                        steps: None,
                    },
                );
            };

            crate::plugin::run_plugin_with_progress_and_timeout(
                &lifecycle_path,
                "install_server",
                config,
                on_progress,
                1800, // 30분 — 대용량 다운로드 허용
            )
            .await
        };

        let updated = match result {
            Ok(val) => {
                let success = val
                    .get("success")
                    .and_then(|s| s.as_bool())
                    .unwrap_or(false);
                if success {
                    tracing::info!("Server update completed for '{}'", name_done);
                    tracker_done.update(
                        &name_done,
                        crate::ipc::ProvisionProgress {
                            step: 0,
                            total: 1,
                            label: "done".to_string(),
                            message: "Server updated successfully".to_string(),
                            done: true,
                            error: None,
                            percent: Some(100),
                            steps: None,
                        },
                    );
                } else {
                    let err_msg = val
                        .get("message")
                        .or_else(|| val.get("error"))
                        .and_then(|v| v.as_str())
                        .unwrap_or("Unknown error")
                        .to_string();
                    tracing::error!("Server update failed for '{}': {}", name_done, err_msg);
                    tracker_done.update(
                        &name_done,
                        crate::ipc::ProvisionProgress {
                            step: 0,
                            total: 1,
                            label: "done".to_string(),
                            message: "Update failed".to_string(),
                            done: true,
                            error: Some(err_msg),
                            percent: Some(100),
                            steps: None,
                        },
                    );
                }
                success
            }
            Err(e) => {
                tracing::error!("Server update error for '{}': {}", name_done, e);
                tracker_done.update(
                    &name_done,
                    crate::ipc::ProvisionProgress {
                        step: 0,
                        total: 1,
                        label: "done".to_string(),
                        message: "Update failed".to_string(),
                        done: true,
                        error: Some(format!("{}", e)),
                        percent: Some(100),
                        steps: None,
                    },
                );
                false
            }
        };

        // 5초 후 프로비저닝 tracker 자동 정리
        let tracker_cleanup = tracker_done.clone();
        let name_cleanup = name_done.clone();
        tokio::spawn(async move {
            tokio::time::sleep(std::time::Duration::from_secs(5)).await;
            tracker_cleanup.remove(&name_cleanup);
        });
        updated
    }))

}
