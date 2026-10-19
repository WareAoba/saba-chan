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
    let language = state.config_store.ui_language().await;
    let supervisor = state.supervisor.read().await;

    match supervisor.instance_store.get(&id) {
        Some(instance) => {
            // 모듈 약관과 동의 여부 (`agreements`)를 붙인다
            let mut body = serde_json::to_value(instance).unwrap_or_default();
            body["agreements"] = json!(supervisor.agreement_views(instance, &language));
            (StatusCode::OK, Json(body)).into_response()
        }
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

/// 사용자가 정한 `java_path`를 그 모듈의 격리 실행에 연다 (폴더 권한 고치기는 오래 걸릴 수 있어 따로)
fn open_user_runtime(entry: String, instance: &crate::instance::ServerInstance) {
    let Some(java) = instance.module_settings.get("java_path").and_then(Value::as_str).filter(|_| instance.user_settings.contains("java_path")) else { return };
    let java = java.to_string();
    tokio::task::spawn_blocking(move || crate::sandbox::open_user_runtime(std::path::Path::new(&entry), &java));
}

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
        // 이름으로 서버를 찾는 클라이언트가 있어 같은 이름은 둘 두지 않는다 (rename과 같은 409)
        if supervisor.instance_store.list().iter().any(|i| i.name == name) {
            let error = json!({ "success": false, "error": format!("An instance named '{}' already exists", name), "error_code": "name_taken" });
            return (StatusCode::CONFLICT, Json(error)).into_response();
        }
        let mut instance = crate::instance::ServerInstance::new(name, module);

        // 모듈 정보에서 process_name, default_port, install 설정 가져오기
        let mut module_trust = None;
        let module_install = if let Ok(loaded_module) = supervisor.module_loader.get_module(module) {
            module_trust = Some(super::server::module_trust(&loaded_module.path));
            instance.process_name = loaded_module.metadata.process_name.clone();
            if instance.port.is_none() {
                instance.port = loaded_module.metadata.default_port;
            }
            // rest_port 기본값은 모듈이 REST를 지원할 때만 설정
            let supported = loaded_module.metadata.protocols_supported.as_deref().unwrap_or(&[]);
            if instance.rest_port.is_none() && supported.iter().any(|s| s == "rest") {
                instance.rest_port = Some(loaded_module.metadata.default_rest_port());
            }

            // 고른 프리셋(`[[presets]]`)의 설정이 바탕이 되고, 따로 보낸 설정이 그 위를 덮는다
            let mut chosen = serde_json::Map::new();
            if let Some(id) = payload.get("preset").and_then(|v| v.as_str()).filter(|p| !p.is_empty()) {
                match loaded_module.metadata.presets.iter().find(|p| p.id == id) {
                    Some(preset) => chosen.extend(preset.settings.clone()),
                    None => {
                        let error = json!({ "error": format!("Module '{}' has no preset '{}'", module, id), "error_code": "preset_not_found" });
                        return (StatusCode::BAD_REQUEST, Json(error)).into_response();
                    }
                }
            }
            if let Some(given) = payload.get("module_settings").and_then(|v| v.as_object()) {
                chosen.extend(given.clone());
            }

            // 만들 때 동의한 약관 (`[[agreements]]` id). 이 PC에서만 받는다
            let accepted = match super::managed::requested_agreements(&payload, "accept_agreements") {
                Ok(ids) => ids,
                Err(resp) => return *resp,
            };
            if let Some(unknown) = accepted.iter().find(|id| !loaded_module.metadata.agreements.iter().any(|a| &a.id == *id)) {
                let error = json!({ "error": format!("Module '{}' has no agreement '{}'", module, unknown), "error_code": "invalid_format", "fields": [{"field": "accept_agreements"}] });
                return (StatusCode::BAD_REQUEST, Json(error)).into_response();
            }
            let now = chrono::Utc::now().to_rfc3339();
            instance.accepted_agreements = accepted.into_iter().map(|id| (id, now.clone())).collect();

            // 만들 때 고른 설정 (예: 마인크래프트 서버 종류 — 설치할 jar가 달라진다).
            // 모듈이 선언한 이름만, select는 고를 수 있는 값만 받는다
            if let Some(settings) = loaded_module.metadata.settings.as_ref() {
                for field in &settings.fields {
                    let Some(value) = chosen.get(&field.name).filter(|v| !v.is_null()) else { continue };
                    let allowed = match (&field.options, value.as_str()) {
                        (Some(options), Some(v)) => options.iter().any(|o| o == v),
                        (Some(_), None) => false,
                        (None, _) => !value.is_object() && !value.is_array(),
                    };
                    if !allowed || crate::validator::validate_setting_value(field, Some(value)).is_err() {
                        // 조용히 버리면 프리셋·기본값이 남아 고른 것과 다른 서버가 된다
                        let error = json!({ "error": format!("module_settings.{} is not a valid value", field.name), "error_code": "invalid_format", "fields": [{"field": field.name}] });
                        return (StatusCode::BAD_REQUEST, Json(error)).into_response();
                    }
                    instance.module_settings.insert(field.name.clone(), value.clone());
                    // 사용자가 보낸 값이 모듈 기본값·프리셋과 다를 때만 사용자가 정한 값이다
                    // (만들기 화면은 기본값을 채워 보낸다)
                    let from_module = payload.get("preset").and_then(|v| v.as_str())
                        .and_then(|id| loaded_module.metadata.presets.iter().find(|p| p.id == id))
                        .and_then(|p| p.settings.get(&field.name))
                        .is_some_and(|v| v == value)
                        || field.default.as_ref().and_then(|d| d.as_str()).is_some_and(|d| {
                            let dir = supervisor.instance_store.instance_dir(&instance.id);
                            let substituted = d.replace(INSTANCE_DIR_PLACEHOLDER, &dir.to_string_lossy());
                            value.as_str().is_some_and(|v| v == d || v == substituted)
                        });
                    let given = payload.get("module_settings").and_then(|v| v.get(&field.name)).is_some_and(|v| v == value);
                    if given && !from_module {
                        instance.user_settings.insert(field.name.clone());
                    }
                }
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

            open_user_runtime(loaded_module.entry_path(), &instance);

            Some((
                loaded_module.metadata.install.clone(),
                loaded_module.entry_path(),  // 모듈 진입 파일 (마이그레이션 설정 import 시 사용)
                loaded_module.metadata.server_executable.clone(),  // 바이너리 파일명 (마이그레이션 시 executable_path 자동 계산)
            ))
        } else {
            None
        };

        // ── 익스텐션이 만들 때 물은 칸 (`AddServer.options` → `extension_data`, Q-7). 설치 방식 값은 뒤에서 덮는다 ──
        match payload.get("extension_data") {
            None | Some(Value::Null) => {}
            Some(Value::Object(given)) => {
                for (key, value) in given.iter().filter(|(_, v)| !v.is_null()) {
                    instance.extension_data.insert(key.clone(), value.clone());
                }
            }
            Some(_) => {
                let error = json!({ "error": "extension_data must be an object", "error_code": "invalid_format", "fields": [{"field": "extension_data"}] });
                return (StatusCode::BAD_REQUEST, Json(error)).into_response();
            }
        }

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
        // 한 폴더에 서버 둘이면 서로의 월드·설정을 덮어쓴다
        if let Some(dir) = instance.working_dir.as_deref() {
            let taken = supervisor.instance_store.list().iter().find(|o| {
                o.working_dir.as_deref().is_some_and(|d| saba_chan_updater_lib::same_path(std::path::Path::new(d), std::path::Path::new(dir)))
            });
            if let Some(other) = taken {
                let error = json!({
                    "success": false,
                    "error": format!("Server '{}' already uses this folder", other.name),
                    "error_code": "working_dir_in_use",
                    "instance": other.name,
                });
                return (StatusCode::CONFLICT, Json(error)).into_response();
            }
        }
        let given_port = payload.get("port").filter(|v| !v.is_null());
        if given_port.is_some_and(|v| v.as_u64().and_then(|p| u16::try_from(p).ok()).filter(|&p| p > 0).is_none()) {
            let error = json!({ "error": "port must be a number from 1 to 65535", "error_code": "invalid_format", "fields": [{"field": "port"}] });
            return (StatusCode::BAD_REQUEST, Json(error)).into_response();
        }
        if let Some(port) = given_port.and_then(|v| v.as_u64()).and_then(|p| u16::try_from(p).ok()) {
            instance.port = Some(port);
            // 모듈이 `port` 설정을 선언했으면 그것도 맞춘다 (설치가 module_settings로 게임 설정 파일을 쓴다)
            let declared = supervisor.module_loader.get_module(module).ok()
                .and_then(|m| m.metadata.settings.as_ref().map(|s| s.fields.iter().any(|f| f.name == "port")))
                .unwrap_or(false);
            if declared {
                instance.module_settings.insert("port".to_string(), json!(port));
            }
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
        // 미검증·변경된 모듈로 만들면 알림만 싣는다 (`module_trust`, `warnings`)
        let module_name_owned = instance.module_name.clone();
        let created = move |mut body: Value| {
            if let Some(trust) = module_trust {
                body["module_trust"] = json!(trust);
                body["warnings"] = json!(super::server::trust_warnings("module", &module_name_owned, trust));
            }
            (StatusCode::CREATED, Json(body)).into_response()
        };
        let instance_name = instance.name.clone();
        let module_name_owned = module.to_string();
        let instance_dir = supervisor.instance_store.instance_dir(&id);

        // 인스턴스 저장 (프로비저닝은 비동기로 수행)
        match supervisor.instance_store.add(instance.clone()) {
            Ok(_) => {
                // ── Extension hook: server.post_create (결과를 기다리지 않는다) ──
                {
                    let ext_mgr = state.extension_manager.clone();
                    let ctx = serde_json::json!({
                        "instance_id": &id,
                        "instance_dir": instance_dir.to_string_lossy(),
                        "instance": serde_json::to_value(&instance).unwrap_or_default(),
                        "module": &module_name_owned,
                        "migration": migration_mode,
                    });
                    tokio::spawn(async move {
                        let _ = crate::extension::run_hook(&ext_mgr, "server.post_create", ctx).await;
                    });
                }
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
                        // 진행 문구는 데몬 언어로 (모듈이 보내는 문구는 SABA_LANG으로 이미 번역돼 온다)
                        let text = crate::chat::i18n::Catalog::for_language(&state.config_store.ui_language().await);

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
                            message: text.t("provision.installing", &[]),
                            done: false,
                            error: None,
                            percent: Some(0),
                            steps: None,
                        });

                        let tracker_done = tracker.clone();
                        let name_done = instance_name.clone();
                        let id_done = id.clone();
                        // install_server가 보는 인스턴스 설정 (서버 종류처럼 받을 파일을 바꾸는 값)
                        let install_settings = instance.module_settings.clone();
                        let accepted_agreements: Vec<String> = instance.accepted_agreements.keys().cloned().collect();

                        // Drop supervisor lock before spawning background task
                        drop(supervisor);
                        // 설치가 끝날 때까지 업데이트·삭제·복원·시작이 같은 폴더에 끼어들지 않게 작업 잠금을 쥔다
                        // (방금 만든 인스턴스라 다른 작업이 쥐고 있을 수 없다)
                        let install_guard = super::backup::BusyGuard::take(&id);

                        tokio::spawn(async move {
                            let mut install_guard = install_guard;
                            // ── Phase 1: supervisor lock을 최소 시간만 잡고 모듈 경로 복사 ──
                            let module_path = {
                                let sup = supervisor_arc.read().await;
                                match sup.module_loader.get_module(&module_name_owned) {
                                    Ok(m) => m.entry_path(),
                                    Err(e) => {
                                        drop(install_guard.take()); // 끝났다고 알리기 전에 놓는다 (알림을 본 삭제가 막히지 않게)
                                        tracker_done.update(&name_done, crate::ipc::ProvisionProgress {
                                            step: 0, total: 1,
                                            label: "done".to_string(),
                                            message: text.t("provision.install_failed", &[]),
                                            done: true,
                                            error: Some(text.t("provision.module_missing", &[("error", &e.to_string())])),
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
                                    drop(install_guard.take()); // 끝났다고 알리기 전에 놓는다 (알림을 본 삭제가 막히지 않게)
                                    tracker_done.update(&name_done, crate::ipc::ProvisionProgress {
                                        step: 0, total: 1,
                                        label: "done".to_string(),
                                        message: text.t("provision.install_failed", &[]),
                                        done: true,
                                        error: Some(text.t("provision.no_latest_version", &[])),
                                        percent: Some(100),
                                        steps: None,
                                    });
                                    return;
                                }

                                let mut config = serde_json::json!({
                                    "install_dir": &install_dir_str,
                                    "accepted_agreements": &accepted_agreements,
                                    "settings": &install_settings,
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
                                    crate::plugin::INSTALL_TIMEOUT_SECS,
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
                                                    // 확인되지 않은 모듈은 인스턴스 폴더 안만 (서버 폴더는 다음 격리 실행에 쓰기를 받는다)
                                                    let root = sup.instance_store.instance_dir(&id_done);
                                                    if crate::sandbox::accepts_module_dir(std::path::Path::new(&module_path), ip, &root) {
                                                        updated.working_dir = Some(ip.clone());
                                                    } else {
                                                        tracing::warn!("Ignoring install_path outside the instance folder from unverified module: {}", ip);
                                                    }
                                                }
                                                let _ = sup.instance_store.update(&id_done, updated);
                                            }
                                        }

                                        tracing::info!("Native provisioning completed for '{}'", name_done);
                                        drop(install_guard.take()); // 끝났다고 알리기 전에 놓는다 (알림을 본 삭제가 막히지 않게)
                                        tracker_done.update(&name_done, crate::ipc::ProvisionProgress {
                                            step: 0, total: 1,
                                            label: "done".to_string(),
                                            message: text.t("provision.installed", &[]),
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
                                            .map(str::to_string)
                                            .unwrap_or_else(|| text.t("provision.install_failed", &[]));
                                        tracing::warn!("Native provisioning failed for '{}': {}", name_done, err_msg);
                                        drop(install_guard.take()); // 끝났다고 알리기 전에 놓는다 (알림을 본 삭제가 막히지 않게)
                                        tracker_done.update(&name_done, crate::ipc::ProvisionProgress {
                                            step: 0, total: 1,
                                            label: "done".to_string(),
                                            message: text.t("provision.install_failed", &[]),
                                            done: true,
                                            error: Some(err_msg),
                                            percent: Some(100),
                                            steps: None,
                                        });
                                    }
                                }
                                Err(e) => {
                                    tracing::warn!("Native provisioning error for '{}': {}", name_done, e);
                                    drop(install_guard.take()); // 끝났다고 알리기 전에 놓는다 (알림을 본 삭제가 막히지 않게)
                                    tracker_done.update(&name_done, crate::ipc::ProvisionProgress {
                                        step: 0, total: 1,
                                        label: "done".to_string(),
                                        message: text.t("provision.install_failed", &[]),
                                        done: true,
                                        // 스크립트 출력(stderr·경로)은 로그에만 남긴다
                                        error: Some(text.t("provision.script_failed", &[])),
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
                        created(response)
                    } else {
                        created(json!({ "success": true, "id": id }))
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
                    let mut detected: Option<Value> = None;
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
                                    // 모듈이 서버 파일을 직접 찾았으면 (예: paper-1.21.4.jar) 그것을 쓴다
                                    if let (Some(rel), Some(dir)) = (
                                        result.get("executable_path").and_then(|v| v.as_str()),
                                        instance.working_dir.as_deref(),
                                    ) {
                                        // `..`·심볼릭 링크로 폴더 밖을 가리키지 않게 정규화한 경로로 비교한다
                                        let resolved = std::path::Path::new(dir).join(rel);
                                        let inside = match (resolved.canonicalize(), std::path::Path::new(dir).canonicalize()) {
                                            (Ok(file), Ok(root)) => file.starts_with(root),
                                            _ => false,
                                        };
                                        if resolved.is_file() && inside {
                                            instance.executable_path = Some(resolved.to_string_lossy().to_string());
                                        }
                                    }
                                    detected = result.get("detected").cloned();
                                }
                            }
                            Err(e) => {
                                tracing::warn!("Migration import_settings failed for '{}': {} (non-fatal)", instance_name, e);
                            }
                        }
                    }
                    // 마이그레이션 변경사항 저장 (executable_path, import된 설정 등)
                    let _ = state.supervisor.write().await.instance_store.update(&id, instance.clone());
                    // 가져온 서버는 설치를 거치지 않으므로, 만들 때 동의한 약관을 모듈이 지금 반영한다 (예: eula.txt)
                    if !instance.accepted_agreements.is_empty() {
                        let ids: Vec<String> = instance.accepted_agreements.keys().cloned().collect();
                        match crate::supervisor::Supervisor::accept_agreements(&state.supervisor, &id, &ids).await {
                            Ok(Ok(result)) if result.get("success").and_then(|v| v.as_bool()) != Some(false) => {}
                            other => tracing::warn!("Migration: module did not apply agreements for '{}': {:?}", instance_name, other),
                        }
                    }
                    // 모듈이 판정한 것(서버 종류, 설치된 플러그인 등)을 그대로 돌려준다
                    let mut response = json!({ "success": true, "id": id });
                    if let Some(detected) = detected {
                        response["detected"] = detected;
                    }
                    created(response)
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
    // 백업·복원·업데이트 중에는 폴더를 지우지 않는다
    let Some(guard) = super::backup::BusyGuard::take(&id) else {
        return super::backup::instance_busy();
    };
    // 요청이 끊겨도 정지·삭제를 끝까지 한다: 추적을 뺀 채 멈추면 돌던 서버가 추적 밖에 남는다
    let work = crate::audit::spawn(async move {
        let _guard = guard;
        delete_instance_held(state, id).await
    });
    work.await.unwrap_or_else(|e| {
        let error = json!({ "success": false, "error": format!("Failed to delete instance: {}", e) });
        (StatusCode::INTERNAL_SERVER_ERROR, Json(error)).into_response()
    })
}

/// 작업 잠금을 잡은 뒤의 인스턴스 삭제
async fn delete_instance_held(state: IPCServer, id: String) -> axum::response::Response {
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
            Some(m) if m.is_running() => m.force_kill(),
            Some(_) => Ok(()),
            None => force_kill_tree(pid),
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

/// POST /api/instance/:id/launch-preview — 실행 인자 미리보기 `{settings?, extra_args?}`.
/// 서버를 띄우지도 파일을 고치지도 않는다. 응답 모양은 `Supervisor::launch_preview`
pub async fn launch_preview(
    State(state): State<IPCServer>,
    Path(id): Path<String>,
    body: Option<Json<Value>>,
) -> impl IntoResponse {
    let body = body.map(|Json(b)| b).unwrap_or(Value::Null);
    let draft = match body.get("settings") {
        None | Some(Value::Null) => serde_json::Map::new(),
        Some(Value::Object(m)) => m.clone(),
        Some(_) => return (StatusCode::BAD_REQUEST, Json(json!({ "error": "invalid_body", "message": "settings must be an object" }))).into_response(),
    };
    // 보내지 않으면 저장된 추가 인자로 본다
    let extra = match body.get("extra_args").map(extra_args_from) {
        None => None,
        Some(Some(args)) => Some(args),
        Some(None) => {
            return (StatusCode::BAD_REQUEST, Json(json!({ "error": "invalid_format", "error_code": "invalid_format", "message": "extra_args must be a string or an array of strings" }))).into_response()
        }
    };
    match crate::supervisor::Supervisor::launch_preview(&state.supervisor, &id, &draft, extra.as_deref()).await {
        Ok(v) => (StatusCode::OK, Json(v)).into_response(),
        Err(e) if e.to_string().contains("not found") => (StatusCode::NOT_FOUND, Json(json!({ "error": e.to_string() }))).into_response(),
        Err(e) => (StatusCode::INTERNAL_SERVER_ERROR, Json(json!({ "success": false, "error": "preview_failed", "message": e.to_string() }))).into_response(),
    }
}

/// 이보다 작은 메모리 상한은 서버가 뜨지도 못하므로 받지 않는다
const MIN_MEMORY_LIMIT_MB: u64 = 256;

/// PATCH의 `memory_limit_mb`: 비움(null·""·0)은 제한 없음, 아니면 `MIN_MEMORY_LIMIT_MB` 이상의 MiB 정수.
/// 글자는 숫자(MiB)나 `4G`·`4096M`(`GB`·`MB`도, 대소문자 무관)
fn memory_limit_from(value: &Value) -> Result<Option<u64>, ()> {
    let n = match value {
        Value::Null => return Ok(None),
        Value::String(s) if s.trim().is_empty() => return Ok(None),
        Value::String(s) => {
            let t = s.trim().to_ascii_uppercase();
            let t = t.strip_suffix('B').unwrap_or(&t);
            let (digits, scale) = match t.strip_suffix('G') {
                Some(d) => (d, 1024),
                None => (t.strip_suffix('M').unwrap_or(t), 1),
            };
            digits.trim().parse::<u64>().ok().and_then(|n| n.checked_mul(scale)).ok_or(())?
        }
        Value::Number(n) => n.as_u64().ok_or(())?,
        _ => return Err(()),
    };
    match n {
        0 => Ok(None),
        n if n >= MIN_MEMORY_LIMIT_MB => Ok(Some(n)),
        _ => Err(()),
    }
}

/// 추가 실행 인자 입력: 문자열이면 `chat::split_quoted`로 나누고, 배열이면 문자열만. null은 비우기.
/// 줄바꿈·NUL이 든 인자는 받지 않는다
pub(crate) fn extra_args_from(value: &Value) -> Option<Vec<String>> {
    let args: Vec<String> = match value {
        Value::Null => Vec::new(),
        Value::String(s) => crate::chat::split_quoted(s),
        Value::Array(items) => items.iter().map(|v| v.as_str().map(str::to_string)).collect::<Option<_>>()?,
        _ => return None,
    };
    (!args.iter().any(|a| a.contains(['\r', '\n', '\0']))).then_some(args)
}

/// PATCH가 모듈 설정이 아니라 인스턴스 필드로 받는 이름 (그 밖의 이름은 `module_settings`로)
const KNOWN_FIELDS: &[&str] = &[
    "port",
    "rest_host",
    "rest_port",
    "rest_username",
    "rest_password",
    "executable_path",
    "working_dir",
    "protocol_mode",
    "server_version",
    "extension_data",
    "extra_args",
    "memory_limit_mb",
    // RCON은 보안상 쓰지 않는다: 예전 GUI·설정이 보내도 모듈 설정이나 게임 파일로 넘기지 않는다
    "rcon_port",
    "rcon_password",
    "enable_rcon",
];

/// 받지만 쓰지 않는 이름 (응답 `ignored`)
const IGNORED_FIELDS: &[&str] = &["rcon_port", "rcon_password", "enable_rcon"];

/// 모듈이 선언하지 않아도 데몬이 `module_settings`에서 읽는 설정
const DAEMON_SETTING_KEYS: &[&str] = &["auto_restart", "restart_on_unknown_exit", "managed_start", "log_sessions_keep"];

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
                    // 형식(`format`·`pattern`)이 틀렸으면 GUI가 "잘못된 형식입니다"를 띄운다
                    let code = if errors.iter().any(|e| e.error_type == crate::validator::ValidationErrorType::InvalidFormat) {
                        "invalid_format"
                    } else {
                        "validation_failed"
                    };
                    let error = json!({
                        "error": code,
                        "error_code": code,
                        "message": "Settings validation failed",
                        "details": error_details,
                        "fields": fields,
                    });
                    return (StatusCode::BAD_REQUEST, Json(error)).into_response();
                }
            }
        }
    }

    // ── 모르는 키 거절 (오타가 조용히 module_settings에 쌓이지 않게) ──
    // 모듈이 설정 목록을 선언했을 때만 본다. 공통 필드와 데몬이 읽는 설정(`DAEMON_SETTING_KEYS`)은 받는다
    if let (Some(settings_obj), Ok(module)) = (settings.as_object(), supervisor.module_loader.get_module(&instance.module_name)) {
        if let Some(ref settings_meta) = module.metadata.settings {
            let unknown: Vec<&String> = settings_obj
                .keys()
                .filter(|k| !KNOWN_FIELDS.contains(&k.as_str()) && !DAEMON_SETTING_KEYS.contains(&k.as_str()))
                .filter(|k| !settings_meta.fields.iter().any(|f| &f.name == *k))
                .collect();
            if !unknown.is_empty() {
                let fields: Vec<Value> = unknown.iter().map(|k| json!({ "field": k })).collect();
                let error = json!({
                    "error": format!("Unknown setting: {}", unknown.iter().map(|k| k.as_str()).collect::<Vec<_>>().join(", ")),
                    "error_code": "unknown_setting",
                    "fields": fields,
                });
                return (StatusCode::BAD_REQUEST, Json(error)).into_response();
            }
        }
    }

    // 설정값 업데이트
    let mut updated = instance.clone();

    // working_dir이 null인데 executable_path가 있으면 자동 보정 (이번 요청이 executable_path를 지우면 하지 않는다)
    if updated.working_dir.is_none() && !settings.get("executable_path").is_some_and(Value::is_null) {
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

    // NOTE: REST 비밀번호 자동 생성은 인스턴스 실행 시점에 supervisor에서 처리합니다.
    // REST 설정과 executable_path는 null이면 지정을 지워 기본값으로 되돌린다
    let cleared = |key: &str| settings.get(key).is_some_and(Value::is_null);
    if cleared("rest_host") {
        updated.rest_host = None;
    }
    if cleared("rest_port") {
        updated.rest_port = None;
    }
    if cleared("rest_username") {
        updated.rest_username = None;
    }
    if cleared("rest_password") {
        updated.rest_password = None;
    }

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
    // 설정 필드에 없는 추가 실행 인자: 문자열(따옴표로 묶기)이나 문자열 배열
    if let Some(extra) = settings.get("extra_args") {
        match extra_args_from(extra) {
            Some(args) => updated.extra_args = args,
            None => {
                let error = json!({ "error": "invalid_format", "error_code": "invalid_format", "message": "extra_args must be a string or an array of strings", "fields": [{"field": "extra_args"}] });
                return (StatusCode::BAD_REQUEST, Json(error)).into_response();
            }
        }
    }
    // 메모리 상한(MiB): 비우거나(null·"") 0이면 제한 없음
    if let Some(limit) = settings.get("memory_limit_mb") {
        match memory_limit_from(limit) {
            Ok(v) => updated.memory_limit_mb = v,
            Err(()) => {
                let error = json!({ "error": "invalid_format", "error_code": "invalid_format", "message": format!("memory_limit_mb must be empty or a whole number of MiB of at least {}", MIN_MEMORY_LIMIT_MB), "fields": [{"field": "memory_limit_mb"}] });
                return (StatusCode::BAD_REQUEST, Json(error)).into_response();
            }
        }
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
                        _ => {}
                    }
                }
            }
        }
    }

    if cleared("executable_path") {
        updated.executable_path = None;
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

    // 작업 폴더 (예: GUI가 서버 파일을 설치한 뒤 그 폴더로)
    if let Some(dir) = settings.get("working_dir").and_then(|v| v.as_str()).filter(|d| !d.trim().is_empty()) {
        updated.working_dir = Some(dir.to_string());
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
            if !KNOWN_FIELDS.contains(&key.as_str()) {
                // 설정 화면은 고치지 않은 값도 함께 보낸다: 값이 바뀐 키만 사용자가 정한 것으로 기록한다
                if updated.module_settings.get(key) != Some(value) {
                    updated.user_settings.insert(key.clone());
                }
                updated.module_settings.insert(key.clone(), value.clone());
            }
        }
    }

    tracing::info!(
        "Updating instance {} with settings: port={:?}, executable_path={:?}, protocol_mode={}, module_settings_count={}",
        id,
        updated.port,
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
                || key == "working_dir"
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
                || key == "rcon_port"
                || key == "rcon_password"
                || key == "enable_rcon"
            {
                continue;
            }
            props_sync.insert(key.clone(), value.clone());
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
    let module = supervisor.module_loader.get_module(&updated_clone.module_name).ok();
    if settings.get("java_path").is_some() {
        if let Some(m) = &module {
            open_user_runtime(m.entry_path(), &updated_clone);
        }
    }
    let module_extensions = module
        .map(|m| serde_json::to_value(&m.metadata.extensions).unwrap_or_default())
        .unwrap_or_else(|| serde_json::json!({}));
    drop(supervisor);

    // server.properties 동기화 (변경된 항목이 있을 때만). 실패해도 저장은 남기고 응답에 싣는다
    let mut sync_error: Option<String> = None;
    if !props_sync.is_empty() {
        // 값에는 비밀번호가 들 수 있어 키만 남긴다
        tracing::info!(
            "Syncing settings to server.properties for instance {}: {:?}",
            id,
            props_sync.keys().collect::<Vec<_>>()
        );
        let props_value = Value::Object(props_sync);
        match crate::supervisor::Supervisor::manage_properties(&state.supervisor, &id, "write", Some(props_value)).await {
            // 모듈에 configure가 없으면 쓸 게임 설정 파일도 없다
            Ok(r) if r.get("success").and_then(|v| v.as_bool()) == Some(false)
                && !r.get("message").and_then(|v| v.as_str()).is_some_and(|m| m.starts_with("Unknown function")) => {
                let message = r.get("message").or(r.get("error")).and_then(|v| v.as_str()).unwrap_or("configure failed");
                tracing::warn!("Module could not write game settings for instance {}: {}", id, message);
                sync_error = Some(message.to_string());
            }
            Ok(_) => tracing::info!("server.properties synced successfully for instance {}", id),
            Err(e) => {
                tracing::warn!("Failed to sync server.properties for instance {}: {}", id, e);
                sync_error = Some(e.to_string());
            }
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
    let mut body = json!({ "success": true, "synced": sync_error.is_none() });
    if let Some(e) = sync_error {
        body["sync_error"] = json!(e);
    }
    // RCON 키처럼 받되 쓰지 않는 것
    let ignored: Vec<&String> = settings.as_object().into_iter().flat_map(|o| o.keys()).filter(|k| IGNORED_FIELDS.contains(&k.as_str())).collect();
    if !ignored.is_empty() {
        body["ignored"] = json!(ignored);
    }
    (StatusCode::OK, Json(body)).into_response()
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

    let install_dir = install_dir_of(&instance, &instance_dir, install_config.as_ref());
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
    // 업데이트가 끝날 때까지 시작·복원·삭제를 막는다 (작업 안으로 넘긴다)
    let Some(guard) = super::backup::BusyGuard::take(id) else {
        return Err(super::backup::instance_busy());
    };
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
    let install_dir = install_dir_of(&instance, &instance_dir, Some(&install_config));
    let install_dir_str = install_dir.to_string_lossy().to_string();
    // 같은 서버 폴더를 쓰는 다른 인스턴스가 켜져 있거나 작업 중이면 덮어쓰지 않는다 (업데이트가 끝날 때까지 잡는다)
    drop(supervisor);
    let others = super::managed::hold_other_instances_using(state, &install_dir_str, Some(id)).await?;
    let supervisor = state.supervisor.read().await;

    let tracker = state.provision_tracker.clone();
    let supervisor_arc = state.supervisor.clone();
    let text = crate::chat::i18n::Catalog::for_language(&state.config_store.ui_language().await);
    let instance_name = instance.name.clone();
    let module_name_owned = instance.module_name.clone();
    let install_settings = instance.module_settings.clone();
    let accepted_agreements: Vec<String> = instance.accepted_agreements.keys().cloned().collect();

    // 프로비저닝 상태 등록 (기존 인스턴스 업데이트 — 실패해도 롤백 대상이 아니다)
    tracker.take_creating(&instance_name);
    tracker.update(
        &instance_name,
        crate::ipc::ProvisionProgress {
            step: 0,
            total: 1,
            label: method.clone(),
            message: text.t("provision.updating", &[]),
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
        let _guard = guard;
        let _others = others;
        // ── Phase 0: 업데이트 전 백업 (A2). 백업할 것이 없으면 그냥 진행, 실패하면 업데이트하지 않는다 ──
        if let Err(r) = super::backup::take_backup_held(&backup_state, &backup_id, crate::backup::BackupKind::PreUpdate, &_guard).await {
            let reason = crate::ipc::internal::into_internal(r).await.body["error"].as_str().unwrap_or("backup failed").to_string();
            tracing::error!("Pre-update backup failed for '{}': {}", name_done, reason);
            tracker_done.update(
                &name_done,
                crate::ipc::ProvisionProgress {
                    step: 0,
                    total: 1,
                    label: "done".to_string(),
                    message: text.t("provision.update_failed", &[]),
                    done: true,
                    error: Some(text.t("provision.backup_failed", &[("reason", &reason)])),
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
                            message: text.t("provision.update_failed", &[]),
                            done: true,
                            error: Some(text.t("provision.module_missing", &[("error", &e.to_string())])),
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
                "accepted_agreements": &accepted_agreements,
                "settings": &install_settings,
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
                crate::plugin::INSTALL_TIMEOUT_SECS,
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
                            message: text.t("provision.updated", &[]),
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
                        .map(str::to_string)
                        .unwrap_or_else(|| text.t("provision.update_failed", &[]));
                    tracing::error!("Server update failed for '{}': {}", name_done, err_msg);
                    tracker_done.update(
                        &name_done,
                        crate::ipc::ProvisionProgress {
                            step: 0,
                            total: 1,
                            label: "done".to_string(),
                            message: text.t("provision.update_failed", &[]),
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
                        message: text.t("provision.update_failed", &[]),
                        done: true,
                        // 스크립트 출력(stderr·경로)은 로그에만 남긴다
                        error: Some(text.t("provision.script_failed", &[])),
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
    use super::memory_limit_from;
    use serde_json::json;

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

    #[tokio::test]
    async fn a_crashed_install_script_reports_a_short_message_not_its_output() {
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
        std::fs::write(dir.join("lifecycle.py"), "import sys\nsys.stderr.write('Traceback: /home/me/secret.py line 3\\n')\nsys.exit(1)\n").unwrap();
        let mut sup = crate::supervisor::Supervisor::new_with_instances_dir(
            modules.to_str().unwrap(),
            tmp.path().join("instances").to_str().unwrap(),
        );
        sup.initialize().await.unwrap();
        let state = crate::ipc::IPCServer::new(std::sync::Arc::new(tokio::sync::RwLock::new(sup)), "127.0.0.1:0", crate::daemon_log::DaemonLogBuffer::new());
        let made = crate::ipc::internal::into_internal(super::create_instance(State(state.clone()), Json(json!({ "name": "a", "module_name": "game" }))).await).await;
        assert!(made.status.is_success(), "{:?}", made.body);

        let mut progress = None;
        for _ in 0..200 {
            progress = state.provision_tracker.get("a").filter(|p| p.done);
            if progress.is_some() {
                break;
            }
            tokio::time::sleep(std::time::Duration::from_millis(50)).await;
        }
        let error = progress.expect("provisioning finished").error.unwrap();
        let text = crate::chat::i18n::Catalog::for_language(&state.config_store.ui_language().await);
        assert_eq!(error, text.t("provision.script_failed", &[]));
        assert!(!error.contains("Traceback") && !error.contains("stderr"));
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
        assert_eq!(saved.body["sync_error"], "server.properties is read-only");
        assert_eq!(saved.body["ignored"], json!(["enable_rcon"]));
        let instance = state.supervisor.read().await.instance_store.get(&id).cloned().unwrap();
        assert_eq!(instance.module_settings["motd"], json!("hi"), "the setting is kept even when the game file write fails");
    }

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

    // 업데이트는 서버가 도는 working_dir에 쓴다: 같은 폴더를 쓰는 다른 인스턴스가 작업 중이면 덮어쓰지 않는다
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
        sup.initialize().await.unwrap();
        let shared = tmp.path().join("PalServer").to_string_lossy().to_string();
        let mut ids = Vec::new();
        for name in ["a", "b"] {
            let mut inst = crate::instance::ServerInstance::new(name, "game");
            inst.working_dir = Some(shared.clone());
            inst.extension_data.insert(crate::instance::install_method_key("steamcmd"), json!(true));
            ids.push(inst.id.clone());
            sup.instance_store.add(inst).unwrap();
        }
        let state = crate::ipc::IPCServer::new(std::sync::Arc::new(tokio::sync::RwLock::new(sup)), "127.0.0.1:0", crate::daemon_log::DaemonLogBuffer::new());
        let other = super::super::backup::BusyGuard::take(&ids[1]).expect("free");
        let refused = super::begin_server_update(&state, &ids[0]).await.err().expect("the other instance is busy");
        let body = crate::ipc::internal::into_internal(refused).await.body;
        assert_eq!(body["error_code"], "instance_busy");
        drop(other);
        assert!(super::super::backup::BusyGuard::take(&ids[0]).is_some(), "a refused update must not keep its own lock");
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
        std::fs::write(dir.join("lifecycle.py"), "import json, sys, time\ntime.sleep(3)\nprint(json.dumps({'success': True}))\n").unwrap();
        let mut sup = crate::supervisor::Supervisor::new_with_instances_dir(
            modules.to_str().unwrap(),
            tmp.path().join("instances").to_str().unwrap(),
        );
        sup.initialize().await.unwrap();
        let state = crate::ipc::IPCServer::new(std::sync::Arc::new(tokio::sync::RwLock::new(sup)), "127.0.0.1:0", crate::daemon_log::DaemonLogBuffer::new());
        let created = crate::ipc::internal::into_internal(super::create_instance(State(state.clone()), Json(json!({ "name": "a", "module_name": "game" }))).await).await;
        let id = created.body["id"].as_str().unwrap().to_string();
        assert!(super::super::backup::BusyGuard::take(&id).is_none(), "the install holds the lock");
        for _ in 0..200 {
            if let Some(_guard) = super::super::backup::BusyGuard::take(&id) {
                return;
            }
            tokio::time::sleep(std::time::Duration::from_millis(100)).await;
        }
        panic!("the lock was never released after the install");
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

    #[tokio::test]
    async fn null_clears_the_executable_and_rest_overrides() {
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
        sup.initialize().await.unwrap();
        let state = crate::ipc::IPCServer::new(std::sync::Arc::new(tokio::sync::RwLock::new(sup)), "127.0.0.1:0", crate::daemon_log::DaemonLogBuffer::new());
        let made = crate::ipc::internal::into_internal(super::create_instance(State(state.clone()), Json(json!({ "name": "a", "module_name": "game" }))).await).await;
        let id = made.body["id"].as_str().unwrap().to_string();
        let patch = |body: serde_json::Value| {
            let (state, id) = (state.clone(), id.clone());
            async move { crate::ipc::internal::into_internal(super::update_instance_settings(State(state), Path(id), Json(body)).await).await }
        };
        let exe = tmp.path().join("server").join("run.sh");
        let set = json!({ "executable_path": exe.to_str().unwrap(), "rest_host": "10.0.0.2", "rest_port": 9000, "rest_username": "admin", "rest_password": "hunter22" });
        assert!(patch(set).await.status.is_success());
        let cleared = json!({ "executable_path": null, "rest_host": null, "rest_port": null, "rest_username": null, "rest_password": null });
        assert!(patch(cleared).await.status.is_success());
        let inst = state.supervisor.read().await.instance_store.get(&id).cloned().unwrap();
        assert_eq!(inst.executable_path, None);
        assert_eq!((inst.rest_host, inst.rest_port, inst.rest_username, inst.rest_password), (None, None, None, None));
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
