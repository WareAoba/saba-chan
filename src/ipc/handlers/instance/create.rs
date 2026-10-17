use axum::{
    extract::State,
    http::StatusCode,
    response::IntoResponse,
    Json,
};
use serde_json::{json, Value};

use crate::ipc::IPCServer;

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

        // 익스텐션 모드 플래그 (예: 컨테이너 격리)
        let use_container_ext = payload
            .get("use_container")
            .and_then(|v| v.as_bool())
            .unwrap_or(false);

        // 컨테이너 격리 요청 시 해당 익스텐션이 활성화되어 있는지 검증
        if use_container_ext {
            let ext_mgr = state.extension_manager.read().await;
            if !ext_mgr.is_enabled("docker") {
                let error = json!({
                    "error": "Cannot create instance: the required extension is not enabled. Enable it in Settings → Extensions first.",
                    "error_code": "extension_required",
                    "extension_id": "docker",
                });
                return (StatusCode::UNPROCESSABLE_ENTITY, Json(error)).into_response();
            }
            drop(ext_mgr);
        }

        // extension_data 설정 (컨테이너 격리 플래그 → extension_data에 저장)
        if use_container_ext {
            instance.extension_data.insert(
                "docker_enabled".to_string(),
                serde_json::json!(true),
            );
        }

        // ── 네이티브(비-컨테이너) 인스턴스는 모듈당 1개 제한 ──
        if !use_container_ext {
            let existing = supervisor.instance_store.list()
                .iter()
                .find(|i| i.module_name == module && !i.ext_enabled("docker_enabled"));
            if let Some(existing_inst) = existing {
                let error = json!({
                    "error": format!(
                        "A native instance for module '{}' already exists: '{}'. Only one native instance per module is allowed. Use container isolation to run multiple instances.",
                        module, existing_inst.name
                    ),
                    "error_code": "duplicate_native_instance",
                    "existing_instance": existing_inst.name,
                });
                return (StatusCode::CONFLICT, Json(error)).into_response();
            }
        }

        // 모듈 정보에서 process_name, default_port, install/container config 가져오기
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
            // (docker command 템플릿의 {ram} 등이 해결되려면 이 값이 필요)
            if let Some(ref settings) = loaded_module.metadata.settings {
                for field in &settings.fields {
                    if !instance.module_settings.contains_key(&field.name) {
                        if let Some(ref default_val) = field.default {
                            let json_val = match default_val {
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
                loaded_module.metadata.extensions.get("docker").cloned(),  // 컨테이너 익스텐션 설정
                loaded_module.path.clone(),  // 모듈 디렉토리 경로 (마이그레이션 설정 import 시 사용)
                loaded_module.metadata.server_executable.clone(),  // 바이너리 파일명 (마이그레이션 시 executable_path 자동 계산)
            ))
        } else {
            None
        };

        // ── 모듈의 install.requires_extensions → 인스턴스 required_extensions 자동 채우기 ──
        if let Some((Some(ref install), _, _, _)) = module_install {
            for ext_id in &install.requires_extensions {
                if !instance.required_extensions.contains(ext_id) {
                    instance.required_extensions.push(ext_id.clone());
                }
            }
        }

        // ── 네이티브 인스턴스: 필수 익스텐션 활성화 검증 ──
        // (컨테이너 모드는 컨테이너 내부에서 처리하므로 스킵)
        if !use_container_ext && !instance.required_extensions.is_empty() {
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
        }

        let id = instance.id.clone();
        let instance_name = instance.name.clone();
        let module_name_owned = module.to_string();

        // 컨테이너 모드일 때: working_dir를 인스턴스 디렉토리의 server/ 하위 경로로 설정
        let instance_dir = supervisor.instance_store.instance_dir(&id);
        if use_container_ext {
            let server_dir = instance_dir.join("server");
            instance.working_dir = Some(server_dir.to_string_lossy().to_string());
        }

        // 인스턴스 저장 (프로비저닝은 비동기로 수행)
        match supervisor.instance_store.add(instance.clone()) {
            Ok(_) => {
                // ── Extension hook: server.post_create ──
                if use_container_ext {
                    let ext_mgr = state.extension_manager.clone();
                    let tracker = state.provision_tracker.clone();
                    let ctx = serde_json::json!({
                        "instance_id": &id,
                        "instance_name": &instance_name,
                        "module": &module_name_owned,
                        "use_container": use_container_ext,
                        "instance_dir": instance_dir.to_string_lossy(),
                        "extension_data": &instance.extension_data,
                        "instance": serde_json::to_value(&instance).unwrap_or_default(),
                        "module_install": module_install.as_ref().map(|(install, ext_container, _, _)| {
                            serde_json::json!({
                                "install": install,
                                "container": ext_container,
                            })
                        }),
                    });
                    let inst_clone = instance.clone();

                    // 초기 프로비저닝 상태 등록 (범용 — 스텝 정보는 extension이 제공)
                    tracker.update(&inst_clone.name, crate::ipc::ProvisionProgress {
                        step: 0,
                        total: 1,
                        label: "initializing".to_string(),
                        message: "Initializing...".to_string(),
                        done: false,
                        error: None,
                        percent: Some(0),
                        steps: None,
                    });

                    let tracker_cb = tracker.clone();
                    let name_cb = inst_clone.name.clone();
                    let steps_store = std::sync::Arc::new(std::sync::Mutex::new(None::<Vec<String>>));
                    let steps_write = steps_store.clone();
                    let steps_final = steps_store.clone();
                    let on_progress = move |prog: crate::plugin::ExtensionProgress| {
                        let pct = prog.percent.unwrap_or(0);
                        let msg = prog.message.clone().unwrap_or_default();
                        // extension이 steps 목록을 보내면 저장
                        if let Some(ref new_steps) = prog.steps {
                            if let Ok(mut s) = steps_write.lock() {
                                *s = Some(new_steps.clone());
                            }
                        }
                        let stored_steps = steps_store.lock().ok().and_then(|s| s.clone());
                        tracker_cb.update(&name_cb, crate::ipc::ProvisionProgress {
                            step: prog.step.unwrap_or(0),
                            total: prog.total.unwrap_or(1),
                            label: prog.label.unwrap_or_default(),
                            message: msg,
                            done: false,  // progress 콜백에서는 절대 done 설정하지 않음
                            error: None,
                            percent: Some(pct),
                            steps: stored_steps,
                        });
                    };

                    let tracker_done = tracker.clone();
                    let name_done = inst_clone.name.clone();
                    tokio::spawn(async move {
                        let mgr = ext_mgr.read().await;
                        let results = mgr.dispatch_hook_with_progress(
                            "server.post_create", ctx, on_progress,
                        ).await;
                        // 완료 또는 에러 상태 기록 (Err variant + Python success:false 모두 체크)
                        let err = results.iter().find_map(|(_, r)| match r {
                            Err(e) => Some(e.to_string()),
                            Ok(val) => {
                                if val.get("success").and_then(|s| s.as_bool()) == Some(false) {
                                    val.get("error")
                                        .and_then(|e| e.as_str())
                                        .map(|s| s.to_string())
                                        .or_else(|| Some("Extension reported failure".to_string()))
                                } else {
                                    None
                                }
                            }
                        });
                        let final_steps = steps_final.lock().ok().and_then(|s| s.clone());
                        if let Some(ref e) = err {
                            tracing::warn!("Provisioning failed for '{}': {}", inst_clone.name, e);
                        }
                        let has_error = err.is_some();
                        tracker_done.update(&name_done, crate::ipc::ProvisionProgress {
                            step: 0,
                            total: 1,
                            label: "done".to_string(),
                            message: if has_error { "Provisioning failed".to_string() } else { "Provisioning complete".to_string() },
                            done: true,
                            error: err,
                            percent: Some(100),
                            steps: final_steps,
                        });
                        tracing::info!("Extension post_create dispatched for '{}' (error={})", inst_clone.name, has_error);

                        // 성공 시 5초 후 tracker 자동 정리 → provisioning UI 자연스럽게 사라짐
                        // 에러 시에는 유지 → 사용자가 dismiss할 때까지 UI 표시
                        if !has_error {
                            let tracker_cleanup = tracker_done.clone();
                            let name_cleanup = name_done.clone();
                            tokio::spawn(async move {
                                tokio::time::sleep(std::time::Duration::from_secs(5)).await;
                                tracker_cleanup.remove(&name_cleanup);
                            });
                        }
                    });

                    let response = json!({
                        "success": true,
                        "id": id,
                        "provisioning": true,
                    });
                    (StatusCode::CREATED, Json(response)).into_response()
                } else if !migration_mode {
                    // ── Native provisioning: 서버 바이너리 설치 ──
                    // install 설정이 있는 모듈만 프로비저닝 (steamcmd / download)
                    let has_install = module_install.as_ref()
                        .and_then(|(install, _, _, _)| install.as_ref())
                        .is_some();

                    if has_install {
                        let install_config = module_install.as_ref()
                            .and_then(|(install, _, _, _)| install.clone())
                            .unwrap();

                        // ── SteamCMD extension_data 자동 주입 (신규 생성) ──
                        instance.apply_install_extension_data(
                            &install_config.method,
                            install_config.app_id,
                            install_config.anonymous,
                            install_config.beta.as_deref(),
                            install_config.platform.as_deref(),
                        );
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
                            "steamcmd" => "steamcmd",
                            _ => "download",
                        };
                        tracker.update(&instance_name, crate::ipc::ProvisionProgress {
                            step: 0,
                            total: 1,
                            label: provision_label.to_string(),
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
                                    Ok(m) => format!("{}/lifecycle.py", m.path),
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
                                    None // steamcmd 방식은 version 불필요
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
                                        label: prog.label.unwrap_or_else(|| provision_label.to_string()),
                                        message: msg,
                                        done: false,
                                        error: None,
                                        percent: Some(pct),
                                        steps: None,
                                    });
                                };

                                crate::plugin::run_plugin_with_progress_and_timeout(
                                    &module_path, "install_server", config, on_progress,
                                    1800, // 30분 — SteamCMD 대용량 다운로드 허용
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

                    // ── SteamCMD extension_data 자동 주입 (마이그레이션) ──
                    // 모듈이 steamcmd 방식일 경우, hook 조건이 올바르게 동작하도록
                    // install_method_steamcmd, steamcmd_app_id 등을 extension_data에 설정
                    if let Some((Some(ref install), _, _, _)) = module_install {
                        instance.apply_install_extension_data(
                            &install.method,
                            install.app_id,
                            install.anonymous,
                            install.beta.as_deref(),
                            install.platform.as_deref(),
                        );
                    }

                    // ── executable_path 자동 계산 ──
                    // migration_source(=working_dir)와 server_executable(바이너리 파일명)을 결합
                    // 프론트엔드가 모듈 기본값을 보낼 수 있으므로, migration에서는 항상 재계산
                    if let (Some(ref work_dir), Some((_, _, _, Some(ref exe_name)))) = (&instance.working_dir, &module_install) {
                        let resolved = std::path::Path::new(work_dir).join(exe_name);
                        if resolved.exists() {
                            instance.executable_path = Some(resolved.to_string_lossy().to_string());
                            tracing::info!("Migration: executable_path auto-resolved to '{}'", resolved.display());
                        } else {
                            tracing::warn!("Migration: expected executable '{}' not found in '{}'", exe_name, work_dir);
                        }
                    }

                    // ── 기존 INI 설정 import ──
                    // 임포트가 실패하면 module_settings 캐시는 스키마 기본값으로 남는다.
                    // 이 상태를 조용히 넘기면 GUI가 기본값을 실제 값처럼 표시/저장해
                    // 실제 게임 설정 파일을 덮어쓰는 사고로 이어지므로, 실패 여부를
                    // settings_import_pending 플래그로 인스턴스에 기록한다.
                    // (해제 경로: read_live_settings 성공 시 supervisor가 캐시 갱신과 함께 해제)
                    let mut import_succeeded = false;
                    if let Some((_, _, ref module_path, ref _server_exe)) = module_install {
                        let lifecycle_path = format!("{}/lifecycle.py", module_path);
                        let import_config = json!({
                            "working_dir": instance.working_dir,
                        });
                        match crate::plugin::run_plugin(&lifecycle_path, "import_settings", import_config).await {
                            Ok(result) => {
                                if result.get("success").and_then(|v| v.as_bool()).unwrap_or(false) {
                                    if let Some(settings) = result.get("settings").and_then(|v| v.as_object()) {
                                        import_succeeded = !settings.is_empty();
                                        for (key, value) in settings {
                                            instance.module_settings.insert(key.clone(), value.clone());
                                        }
                                        // port 등 core 필드 동기화
                                        if let Some(port) = settings.get("port").and_then(|v| v.as_u64()) {
                                            instance.port = Some(port as u16);
                                        }
                                        // credential_map 역매핑: import된 설정에서 daemon first-class 필드로 동기화
                                        // 예: Palworld의 credential_map { rest_password = "AdminPassword" }
                                        //   → settings에 "AdminPassword" 값이 있으면 instance.rest_password에 설정
                                        if let Ok(module) = supervisor.module_loader.get_module(&module_name_owned) {
                                            for (daemon_key, game_key) in &module.metadata.credential_map {
                                                // game_key(예: "AdminPassword")를 case-insensitive로 settings에서 탐색
                                                let game_key_lower = game_key.to_lowercase();
                                                let value = settings.iter()
                                                    .find(|(k, _)| k.to_lowercase() == game_key_lower || k.as_str() == game_key.as_str())
                                                    .and_then(|(_, v)| v.as_str());
                                                if let Some(pwd) = value {
                                                    match daemon_key.as_str() {
                                                        "rest_password" => instance.rest_password = Some(pwd.to_string()),
                                                        "rcon_password" => instance.rcon_password = Some(pwd.to_string()),
                                                        _ => {}
                                                    }
                                                }
                                            }
                                        }
                                        tracing::info!("Migration import_settings: {} settings imported for '{}'", settings.len(), instance_name);
                                    }
                                }
                            }
                            Err(e) => {
                                tracing::warn!("Migration import_settings failed for '{}': {} (non-fatal)", instance_name, e);
                            }
                        }
                    }
                    instance.settings_import_pending = !import_succeeded;
                    if instance.settings_import_pending {
                        tracing::warn!(
                            "Migration '{}': settings import did not succeed — module_settings cache holds schema defaults, flagged settings_import_pending",
                            instance_name
                        );
                    }
                    // 마이그레이션 변경사항 저장 (executable_path, import된 설정 등)
                    let _ = supervisor.instance_store.update(&id, instance.clone());
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

