use super::LooseJson;
use axum::{
    extract::{Path, Query, State},
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
            // 06 §8.2: 저장된 값 기준으로 지금 숨겨진 설정 (CLI가 쓴다)
            let fields = supervisor.module_loader.get_module(&instance.module_name).ok().and_then(|m| m.metadata.settings.clone()).map(|s| s.fields).unwrap_or_default();
            body["hidden"] = json!(crate::validator::hidden_fields(&fields, |k| stored_setting(&body, instance, k)));
            body["revision"] = json!(instance.settings_revision());
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
) -> axum::response::Response {
    // 요청이 끊겨도(클라이언트 시간 초과) 만들기를 끝까지 마친다: 가져오기는 큰 서버 폴더에 권한을 주는 데 수십 초가 들 수 있고,
    // 중간에 멈추면 인스턴스만 남고 설정 가져오기·원래 설정 보관이 빠진다
    match tokio::spawn(create_instance_to_the_end(state, payload)).await {
        Ok(response) => response,
        Err(e) => {
            tracing::error!("Instance creation task failed: {}", e);
            (StatusCode::INTERNAL_SERVER_ERROR, Json(json!({ "error": "instance creation failed" }))).into_response()
        }
    }
}

async fn create_instance_to_the_end(state: IPCServer, payload: Value) -> axum::response::Response {
    create_instance_inner(state, payload).await.into_response()
}

async fn create_instance_inner(state: IPCServer, payload: Value) -> impl IntoResponse {
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
        // REST 포트를 모듈 기본값으로 채웠는가 (사용자가 정하지 않은 값만 예약 범위 밖으로 옮긴다)
        let mut rest_port_defaulted = false;

        // 모듈 정보에서 process_name, default_port, install 설정 가져오기
        let mut module_trust = None;
        let module_install = if let Ok(loaded_module) = supervisor.module_loader.get_module(module) {
            if !loaded_module.metadata.supports_os(std::env::consts::OS) {
                let platforms = loaded_module.metadata.platforms.clone().unwrap_or_default();
                let body = crate::supervisor::module_loader::platform_unsupported_body(module, &platforms);
                return (StatusCode::CONFLICT, Json(body)).into_response();
            }
            module_trust = Some(super::server::module_trust(&loaded_module.path));
            instance.process_name = loaded_module.metadata.process_name.clone();
            if instance.port.is_none() {
                instance.port = loaded_module.metadata.default_port;
            }
            // rest_port 기본값은 모듈이 REST를 지원할 때만 설정
            let supported = loaded_module.metadata.protocols_supported.as_deref().unwrap_or(&[]);
            if instance.rest_port.is_none() && supported.iter().any(|s| s == "rest") {
                instance.rest_port = Some(loaded_module.metadata.default_rest_port());
                rest_port_defaulted = true;
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
            if chosen.get("rest_port").is_some_and(|v| !v.is_null()) {
                rest_port_defaulted = false;
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
                    let allowed = field.field_type == "list" || match (&field.options, value.as_str()) {
                        (Some(options), Some(v)) => options.iter().any(|o| o == v),
                        (Some(_), None) => false,
                        (None, _) => !value.is_object() && !value.is_array(),
                    };
                    // 새로 설치하는 서버는 모듈이 설치할 수 있는 값만 (가져오는 서버는 판정된 값 그대로)
                    let installable = payload.get("migration_source").is_some()
                        || field.install_options.as_ref().is_none_or(|ok| value.as_str().is_some_and(|v| ok.iter().any(|o| o == v)));
                    if allowed && !installable {
                        let error = json!({ "error": format!("module_settings.{} cannot be installed by saba-chan", field.name), "error_code": "not_installable", "fields": [{"field": field.name}] });
                        return (StatusCode::BAD_REQUEST, Json(error)).into_response();
                    }
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
                                // list 기본값 (06 §8.1: TOML 문자열 배열)
                                toml::Value::Array(_) if field.field_type == "list" => match serde_json::to_value(default_val) {
                                    Ok(v) => v,
                                    Err(_) => continue,
                                },
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

        // ── 앱 설정의 새 서버 기본값. 기본값과 같으면 비워 둬 지금처럼 읽을 때 기본값을 쓴다 ──
        let app = state.config_store.get_gui_settings().await;
        if app.new_server_backup_keep as usize != crate::ipc::handlers::backup::DEFAULT_KEEP {
            instance.backup_keep = Some(app.new_server_backup_keep);
        }
        if !app.new_server_auto_restart && !instance.module_settings.contains_key("auto_restart") {
            instance.module_settings.insert("auto_restart".into(), json!(false));
        }

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
        // `migration_mode = "move"`면 그 폴더를 인스턴스 폴더 안으로 옮겨 전속시킨다 (기본 "link": 그 자리에 연결)
        let move_mode = match payload.get("migration_mode").and_then(|v| v.as_str()) {
            None | Some("link") => false,
            Some("move") => true,
            Some(_) => {
                let error = json!({ "error": "migration_mode must be \"link\" or \"move\"", "error_code": "invalid_format", "fields": [{"field": "migration_mode"}] });
                return (StatusCode::BAD_REQUEST, Json(error)).into_response();
            }
        };
        let mut moved_from: Option<std::path::PathBuf> = None;
        let migration_mode = if let Some(src) = payload.get("migration_source").and_then(|v| v.as_str()) {
            // 상대 경로는 데몬의 작업 폴더 기준으로 풀린다: 보내는 쪽이 절대 경로로 바꿔 보내야 한다
            if !std::path::Path::new(src).is_absolute() {
                let error = json!({ "error": "migration_source must be an absolute path", "error_code": "invalid_format", "fields": [{"field": "migration_source"}] });
                return (StatusCode::BAD_REQUEST, Json(error)).into_response();
            }
            if move_mode {
                // 옮길 폴더도 다른 인스턴스가 쓰는 폴더와 겹치면 안 된다
                if let Some(other) = supervisor.instance_store.folder_user(std::path::Path::new(src), &instance.id) {
                    let error = json!({ "success": false, "error": format!("Server '{}' already uses this folder", other.name), "error_code": "working_dir_in_use", "instance": other.name });
                    return (StatusCode::CONFLICT, Json(error)).into_response();
                }
                let subdir = module_install.as_ref().and_then(|(install, _, _)| install.as_ref()).and_then(|i| i.install_subdir.clone()).unwrap_or_else(|| "server".to_string());
                let Some(dest) = crate::instance::migration::move_target(&supervisor.instance_store.instance_dir(&instance.id), &subdir) else {
                    tracing::warn!("Migration: module install_subdir '{}' is not a folder inside the instance, not moving", subdir);
                    let error = json!({ "success": false, "error": "The module's install folder is not inside the instance folder", "error_code": "move_failed" });
                    return (StatusCode::CONFLICT, Json(error)).into_response();
                };
                instance.working_dir = Some(dest.to_string_lossy().into_owned());
                moved_from = Some(std::path::PathBuf::from(src));
            } else if instance.working_dir.is_none() {
                instance.working_dir = Some(src.to_string());
            }
            true
        } else {
            false
        };
        // 한 폴더에 서버 둘이면 서로의 월드·설정을 덮어쓴다
        if let Some(dir) = instance.working_dir.as_deref() {
            if let Some(other) = supervisor.instance_store.folder_user(std::path::Path::new(dir), &instance.id) {
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
        // 다른 인스턴스가 저장해 둔 포트. 새 인스턴스의 포트(기본값으로 채운 REST 포트 포함)는 이것과 겹치지 않게 고른다
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
        // Windows가 예약한 포트(예: 팰월드 REST 8212가 Hyper-V 범위 안)도 피한다: 서버가 그 포트를 열지 못한다
        let taken_or_reserved = |p: u16| taken.contains(&p) || crate::validator::port_reserved_by_windows(p);
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
            let fields = port_fields_of(module);
            let fields: Vec<&str> = fields.iter().map(String::as_str).collect();
            if let Some(delta) = crate::validator::shift_ports_clear_of(&mut instance, &fields, taken_or_reserved) {
                tracing::info!("Instance '{}': ports moved by +{} to stay clear of other instances and reserved ports", instance.name, delta);
            }
        }
        // 기본 REST 포트가 Windows 예약 범위에 들면(팰월드 8212가 Hyper-V 범위 8135-8234 안에 든 PC) 서버가 REST를
        // 열지 못한다: 게임 포트를 직접 정했어도, 사용자가 정하지 않은 REST 포트는 예약 범위와 다른 인스턴스의
        // 포트를 피해 옮긴다. 설치가 모듈 설정 값으로 게임 설정 파일을 쓰므로 같은 이름의 모듈 설정도 맞춘다
        if let Some(default) = instance.rest_port.filter(|&p| rest_port_defaulted && crate::validator::port_reserved_by_windows(p)) {
            if let Some(free) = crate::validator::next_free_port(default, taken_or_reserved) {
                tracing::info!("Instance '{}': REST port {} is reserved by Windows, using {}", instance.name, default, free);
                instance.rest_port = Some(free);
                if instance.module_settings.contains_key("rest_port") {
                    instance.module_settings.insert("rest_port".to_string(), json!(free));
                }
            }
        }

        let id = instance.id.clone();
        // 미검증·변경된 모듈로 만들면 알림만 싣는다 (`module_trust`, `warnings`)
        let module_name_owned = instance.module_name.clone();
        let check_state = state.clone();
        let created = move |mut body: Value| {
            if let Some(trust) = module_trust {
                body["module_trust"] = json!(trust);
                body["warnings"] = json!(super::server::trust_warnings("module", &module_name_owned, trust));
            }
            // 자동 포트포워딩 확인 (설정이 켜져 있을 때만). 만들기 잠금이 풀린 뒤에 돈다
            if let Some(id) = body["id"].as_str().map(str::to_string) {
                tokio::spawn(async move { crate::port_check::auto_check(&check_state, vec![id]).await });
            }
            (StatusCode::CREATED, Json(body)).into_response()
        };
        let instance_name = instance.name.clone();
        let module_name_owned = module.to_string();
        let instance_dir = supervisor.instance_store.instance_dir(&id);

        // 설치가 끝날 때까지 업데이트·삭제·복원·시작이 같은 폴더에 끼어들지 않게 작업 잠금을 쥔다.
        // 가져오기는 이 요청이 끝날 때까지 쥔다: 격리된 `import_settings`가 끝나면 그때 없던 데몬 항목 자리를 지우므로
        // 그 사이 시작·설정 저장·백업이 데몬 항목을 만들지 않게 한다.
        // 목록에 보이기 전에 잡는다 (보인 뒤에 잡으면 그 틈의 삭제가 먼저 잡아 설치가 지워지는 폴더로 돈다)
        let install_guard = super::backup::InstanceLock::try_take_for_writes(&id);

        // 가져오기 "옮기기": 목록에 보이기 전에 옮긴다 (실패하면 아무것도 만들지 않는다)
        if let (Some(src), Some(dest)) = (&moved_from, instance.working_dir.as_deref()) {
            if let Err(code) = crate::instance::migration::move_into(src, std::path::Path::new(dest)) {
                let _ = std::fs::remove_dir_all(&instance_dir);
                let error = json!({ "success": false, "error": format!("Could not move the server folder into saba-chan ({})", code), "error_code": code });
                return (StatusCode::CONFLICT, Json(error)).into_response();
            }
        }

        // 인스턴스 저장 (프로비저닝은 비동기로 수행)
        match supervisor.instance_store.add(instance.clone()) {
            Ok(_) => {
                // ── Extension hook: server.post_create (결과를 기다리지 않는다) ──
                {
                    let ext_mgr = state.extension_manager.clone();
                    let meta = supervisor.module_loader.get_module(&instance.module_name).ok().map(|m| m.metadata);
                    let ctx = serde_json::json!({
                        "instance_id": &id,
                        "instance_dir": instance_dir.to_string_lossy(),
                        "instance": instance_for_hooks(meta.as_ref(), &instance),
                        "module": &module_name_owned,
                        "migration": migration_mode,
                    });
                    let host_context = Some(state.host_context(&id));
                    tokio::spawn(async move {
                        let _ = crate::plugin::with_instance(host_context, crate::extension::run_hook(&ext_mgr, "server.post_create", ctx)).await;
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
                        tracker.mark_creating(&instance_name, &id);
                        tracker.update(&instance_name, crate::ipc::ProvisionProgress {
                            step: 0,
                            total: 1,
                            label: provision_label.clone(),
                            message: text.t("provision.installing", &[]),
                            done: false,
                            error: None,
                            percent: Some(0),
                            steps: None, details: Default::default(),
                        });

                        let tracker_done = tracker.clone();
                        let name_done = instance_name.clone();
                        let id_done = id.clone();
                        // install_server가 보는 인스턴스 설정 (서버 종류처럼 받을 파일을 바꾸는 값)
                        let install_settings = instance.module_settings.clone();
                        let accepted_agreements: Vec<String> = instance.accepted_agreements.keys().cloned().collect();
                        // 설치 중에 데이터 폴더에 첫 설정을 쓰는 모듈(좀보이드 첫 INI)도 격리 실행에서 쓸 수 있게 넘긴다
                        let instance_data_dirs = crate::supervisor::Supervisor::instance_data_dirs(&instance, &instance_dir);
                        let host_context = state.host_context(&id);

                        // Drop supervisor lock before spawning background task
                        drop(supervisor);

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
                            let result = {
                                // download 방식 모듈은 먼저 list_versions로 최신 버전 조회.
                                // 인스턴스 설정(서버 종류 등)을 넘겨, 그 설정으로 받을 수 있는 최신을 고르게 한다
                                let version = if install_config.method == "download" {
                                    match crate::supervisor::Supervisor::latest_version(&module_path, &serde_json::json!(&install_settings)).await {
                                        Ok(version) => Some(version),
                                        Err(e) => {
                                            drop(install_guard.take());
                                            tracker_done.update(&name_done, crate::ipc::ProvisionProgress {
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

                                let mut config = serde_json::json!({
                                    "install_dir": &install_dir_str,
                                    "accepted_agreements": &accepted_agreements,
                                    "settings": &install_settings,
                                    crate::sandbox::INSTANCE_DATA_DIRS: &instance_data_dirs,
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
                                        steps: None, details: Default::default(),
                                    });
                                };

                                crate::plugin::PluginCall::new(&module_path, "install_server", config)
                                    .in_instance(Some(host_context))
                                    .timeout(crate::plugin::INSTALL_TIMEOUT_SECS)
                                    .run_with_progress(Box::new(on_progress)).await
                            };

                            let result = match result {
                                Ok(mut value) if value.get("success").and_then(Value::as_bool) == Some(true) => {
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

                                        drop(install_guard.take()); // 끝났다고 알리기 전에 놓는다 (알림을 본 삭제가 막히지 않게)
                                        // 실행 경로를 저장하지 못했으면 설치했다고 알리지 않는다 (시작할 실행 파일을 모른다)
                                        if let Err(e) = saved {
                                            tracing::warn!("Native provisioning for '{}' finished but the instance could not be saved: {}", name_done, e);
                                            tracker_done.update(&name_done, crate::ipc::ProvisionProgress {
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
                                        tracker_done.update(&name_done, crate::ipc::ProvisionProgress {
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
                                            steps: None, details: Default::default(),
                                        }.with_failure(val));
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
                                        error: Some(text.t("provision.script_failed", &[])),
                                        percent: Some(100),
                                        steps: None, details: Default::default(),
                                    }.with_failure(crate::plugin::error_response(&e)));
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
                    let metadata = supervisor.module_loader.get_module(&module_name_owned).ok().map(|m| m.metadata);
                    let (credential_map, setting_types) = metadata.as_ref().map(|m| (m.credential_map.clone(), m.setting_types())).unwrap_or_default();
                    drop(supervisor);
                    let mut detected: Option<Value> = None;
                    if let Some((_, ref lifecycle_path, ref _server_exe)) = module_install {
                        match migration_import_call(lifecycle_path, instance.working_dir.as_deref(), &instance_dir).in_instance(Some(state.host_context(&id))).run().await {
                            Ok(mut result) => {
                                if result.get("success").and_then(|v| v.as_bool()).unwrap_or(false) {
                                    let mut imported = instance.clone();
                                    if let Some(settings) = result.get("settings").and_then(|v| v.as_object()) {
                                        imported.apply_imported_settings(settings, &credential_map, &setting_types);
                                        for field in metadata.iter().flat_map(|m| m.settings.iter()).flat_map(|s| &s.fields) {
                                            if (field.field_type == "file" || field.format.as_deref() == Some("path")) && settings.contains_key(&field.name) {
                                                imported.imported_settings.insert(field.name.clone());
                                            }
                                        }
                                        tracing::info!("Migration import_settings: {} settings imported for '{}'", settings.len(), instance_name);
                                    }
                                    let boundary = crate::module_paths::Boundary::for_instance(&imported, &instance_dir, metadata.as_ref());
                                    if let Err(e) = boundary.returned_paths(&mut result, &module_name_owned, "import_settings") {
                                        let mut failure = crate::plugin::error_response(&e);
                                        // 거두지 못했으면(옮긴 폴더를 돌려놓지 못함) 남은 인스턴스를 알린다
                                        if !undo_import(&state, &instance, moved_from.as_deref()).await {
                                            failure["id"] = json!(&id);
                                        }
                                        return (StatusCode::UNPROCESSABLE_ENTITY, Json(failure)).into_response();
                                    }
                                    instance = imported;
                                    // 모듈이 서버 파일을 직접 찾았으면 (예: paper-1.21.4.jar) 그것을 쓴다
                                    if let Some(path) = result.get("executable_path").and_then(|v| v.as_str()).filter(|p| !p.trim().is_empty()) {
                                        let resolved = std::path::Path::new(path);
                                        if resolved.is_file() {
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
                    // 해제할 때 되돌릴 원래 게임 설정 파일. 약관 반영·첫 시작이 파일을 고치기 전에 보관한다
                    // (옮긴 서버도 서버 폴더 밖 데이터 폴더의 설정은 남으므로 보관한다)
                    save_migration_originals(&state, &instance, &instance_dir, metadata.as_ref()).await;
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
                    // 가져온 폴더에서 미리 알릴 것: 쓸 수 없는 폴더, 사바쨩 밖에서 켜 둔 서버
                    // 프로세스 목록 읽기는 막는 호출이라 런타임 밖에서
                    let folder = instance.working_dir.clone().map(std::path::PathBuf::from);
                    let notices = match folder {
                        Some(folder) => tokio::task::spawn_blocking(move || crate::instance::migration::notices(&folder)).await.unwrap_or_default(),
                        None => Vec::new(),
                    };
                    if !notices.is_empty() {
                        response["notices"] = json!(notices);
                    }
                    created(response)
                }
            }
            Err(e) => {
                // 옮긴 서버 폴더는 제자리로 돌려놓는다 (저장하지 못한 인스턴스 폴더에 남기지 않게)
                if let (Some(src), Some(dest)) = (&moved_from, instance.working_dir.as_deref()) {
                    if let Err(back) = std::fs::rename(dest, src) {
                        tracing::error!("Could not move the server folder back to {} after a failed create: {}", src.display(), back);
                    }
                }
                let error = json!({ "error": format!("Failed to create instance: {}", e) });
                (StatusCode::INTERNAL_SERVER_ERROR, Json(error)).into_response()
            }
        }
    } else {
        let error = json!({ "error": "Invalid request" });
        (StatusCode::BAD_REQUEST, Json(error)).into_response()
    }
}

/// 가져오기의 모듈 `import_settings` 호출. instance_dir: 모듈이 서버 폴더 대신 인스턴스 폴더에 둘 것(가져온 서버의
/// 포터블 런타임 등)의 자리. 런타임을 받을 수 있으므로 설치와 같은 시간 한도를 주고, 이 호출만 인스턴스 폴더에 쓴다
/// (`sandbox::MIGRATION_IMPORT`)
fn migration_import_call(lifecycle_path: &str, working_dir: Option<&str>, instance_dir: &std::path::Path) -> crate::plugin::PluginCall {
    let config = json!({
        "working_dir": working_dir,
        "instance_dir": instance_dir,
        crate::sandbox::MIGRATION_IMPORT: true,
    });
    crate::plugin::PluginCall::new(lifecycle_path, "import_settings", config).timeout(crate::plugin::INSTALL_TIMEOUT_SECS)
}

/// 가져온 서버의 게임 설정 파일(모듈 `config_files`)을 인스턴스 폴더에 보관한다 (`instance::migration`).
/// 모듈이 모르거나 실패하면 보관하지 않는다: 가져오기는 막지 않고, 해제할 때 되돌릴 것이 없을 뿐이다
async fn save_migration_originals(
    state: &IPCServer,
    instance: &crate::instance::ServerInstance,
    instance_dir: &std::path::Path,
    metadata: Option<&crate::supervisor::module_loader::ModuleMetadata>,
) {
    let files = match crate::supervisor::Supervisor::manage_properties(&state.supervisor, &instance.id, "config_files", None).await {
        Ok(result) if result.get("success").and_then(Value::as_bool) == Some(true) => result,
        Ok(result) => {
            tracing::info!("Migration: module did not list its config files for '{}': {}", instance.name, result.get("error_code").and_then(serde_json::Value::as_str).unwrap_or("failed"));
            return;
        }
        Err(e) => {
            tracing::warn!("Migration: could not ask the module for its config files for '{}': {}", instance.name, e);
            return;
        }
    };
    // 모듈이 밝힌 경로는 그 서버의 폴더(서버 폴더·인스턴스 폴더·선언한 데이터 폴더) 안의 것만 보관한다
    let boundary = crate::module_paths::Boundary::for_instance(instance, instance_dir, metadata);
    let files: Vec<std::path::PathBuf> = files.get("files").and_then(Value::as_array).into_iter().flatten()
        .filter_map(Value::as_str)
        .filter_map(|f| boundary.resolve(std::path::Path::new(f)))
        .collect();
    match crate::instance::migration::save(instance_dir, &files) {
        Ok(n) => tracing::info!("Migration: kept {} original config file(s) of '{}' to restore when unlinked", n, instance.name),
        Err(e) => tracing::warn!("Migration: could not keep the original config files of '{}': {}", instance.name, e),
    }
}

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
fn other_instance_folders(supervisor: &crate::supervisor::Supervisor, id: &str) -> Vec<std::path::PathBuf> {
    supervisor.instance_store.list().iter()
        .filter(|i| i.id != id)
        .flat_map(|i| [Some(supervisor.instance_store.instance_dir(&i.id)), i.working_dir.as_deref().map(std::path::PathBuf::from)])
        .flatten()
        .collect()
}

/// 인스턴스를 지우기 전 익스텐션 hook `server.pre_delete`. 익스텐션이 정리했으면 파일 잠금이 풀리기를 잠깐 기다린다
async fn pre_delete_hook(state: &IPCServer, instance: &crate::instance::ServerInstance) {
    let ext_mgr = state.extension_manager.clone();
    let ctx = serde_json::json!({
        "instance_id": &instance.id,
        "instance_name": &instance.name,
        "module": &instance.module_name,
        "extension_data": &instance.extension_data,
    });
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

/// 가져오기가 실패한 새 인스턴스를 거둔다: 옮겨 온 서버 폴더는 원래 자리로 돌려놓고, 그 서버 폴더에 준 격리 실행의
/// 권한·표시를 거둔 뒤 인스턴스를 지운다. 돌려놓지 못하면 서버 파일이 든 인스턴스를 그대로 둔다(지우면 서버도 지워진다).
/// 인스턴스를 지웠으면 true
async fn undo_import(state: &IPCServer, instance: &crate::instance::ServerInstance, moved_from: Option<&std::path::Path>) -> bool {
    // 만들 때 server.post_create를 알렸으니 지울 때도 알린다
    pre_delete_hook(state, instance).await;
    let folder = match (moved_from, instance.working_dir.as_deref()) {
        (Some(src), Some(dest)) => {
            // Windows 파일 잠금 대비 재시도 (가져오기 실행기·백신이 잠깐 쥐고 있을 수 있다)
            let mut moved = None;
            for attempt in 0..3 {
                match std::fs::rename(dest, src) {
                    Ok(()) => {
                        moved = Some(src.to_path_buf());
                        break;
                    }
                    Err(e) => tracing::warn!("Could not move the server folder back to {} after a failed import ({}/3): {}", src.display(), attempt + 1, e),
                }
                if attempt < 2 {
                    tokio::time::sleep(std::time::Duration::from_secs(2)).await;
                }
            }
            if moved.is_none() {
                tracing::error!("Could not move the server folder back to {} after a failed import", src.display());
                return false;
            }
            moved
        }
        (None, dir) => dir.filter(|d| !d.trim().is_empty()).map(std::path::PathBuf::from),
        (Some(_), None) => None,
    };
    if let Some(folder) = folder {
        let others = other_instance_folders(&*state.supervisor.read().await, &instance.id);
        // 실행 파일 폴더는 그 서버 폴더 안일 때만 (옮겨 왔으면 옮기기 전 자리로 바꿔 본다)
        let exe_dir = instance.executable_path.as_deref().and_then(|p| std::path::Path::new(p).parent()).map(|d| match (moved_from, instance.working_dir.as_deref()) {
            (Some(_), Some(dest)) => d.strip_prefix(dest).map(|rel| folder.join(rel)).unwrap_or_else(|_| d.to_path_buf()),
            _ => d.to_path_buf(),
        });
        let dirs: Vec<std::path::PathBuf> = std::iter::once(folder.clone()).chain(exe_dir.filter(|d| d != &folder && crate::module_paths::is_inside(d, std::slice::from_ref(&folder)))).collect();
        let name = instance.name.clone();
        let moved_back = moved_from.is_some().then(|| folder.clone());
        let _ = tokio::task::spawn_blocking(move || {
            if let Err(failed) = crate::sandbox::release(&dirs, &others) {
                tracing::warn!("Could not take back sandbox permissions on the server folder of '{}': {}", name, failed.join("; "));
            }
            // 인스턴스 폴더 안에서 물려받은 권한·표시를 걷고 원래 자리의 권한을 되찾는다
            if let Some(folder) = moved_back {
                if let Err(e) = crate::sandbox::adopt_parent(&folder) {
                    tracing::warn!("Could not restore the permissions of the server folder of '{}': {}", name, e);
                }
            }
        })
        .await;
    }
    // Windows 파일 잠금 대비 재시도 (삭제와 같다)
    for attempt in 0..3 {
        match state.supervisor.write().await.instance_store.remove(&instance.id) {
            Ok(()) => return true,
            Err(e) => tracing::warn!("Could not remove '{}' after a failed import ({}/3): {}", instance.name, attempt + 1, e),
        }
        if attempt < 2 {
            tokio::time::sleep(std::time::Duration::from_secs(2)).await;
        }
    }
    false
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
    let Some(progress) = state.provision_tracker.get(&name).filter(|p| p.done) else {
        return (StatusCode::OK, Json(json!({ "success": true, "rolled_back": false }))).into_response();
    };
    let had_error = progress.error.is_some();
    let creating = state.provision_tracker.take_creating(&name);

    state.provision_tracker.remove(&name);

    // 신규 생성이 실패했을 때만 롤백 — 그 설치의 인스턴스를 ID로 지운다.
    // 이름으로 찾으면 그 사이 이름을 바꾸고 같은 이름으로 만든 다른 인스턴스가 지워진다
    let rollback = creating.filter(|_| had_error);
    // 실제로 지웠을 때만 rolled_back (건너뛰거나 실패하면 인스턴스가 남아 있다)
    let mut rolled_back = false;
    if let Some(id) = rollback {
        // 다른 작업(업데이트·복원·삭제)이 그 폴더를 쓰는 중이면 지우지 않는다
        match super::backup::InstanceLock::take_for_writes(&id).await {
            None => tracing::warn!("Skipping rollback of '{}' (id={}): another task holds the instance", name, id),
            Some(_guard) => {
                let mut supervisor = state.supervisor.write().await;
                if supervisor.instance_store.get(&id).is_some() {
                    match supervisor.instance_store.remove(&id) {
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

/// GET /api/provision-progress/:name - 프로비저닝 진행 상태 (서버 이름 기준)
pub async fn get_provision_progress(
    Path(name): Path<String>,
    State(state): State<IPCServer>,
) -> impl IntoResponse {
    if let Some(progress) = state.provision_tracker.get(&name) {
        let mut resp = serde_json::to_value(progress).expect("provision progress contains JSON fields");
        resp["active"] = json!(true);
        (StatusCode::OK, Json(resp)).into_response()
    } else {
        (StatusCode::OK, Json(json!({
            "active": false,
        }))).into_response()
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
    let Some(guard) = super::backup::InstanceLock::take_for_writes(&id).await else {
        return super::backup::instance_busy();
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

    // 연결 해제를 마쳤다는 표시가 없으면 다음 시작 때 인스턴스를 되돌린다 (그 사이 데몬이 멈춘 경우)
    if let Some(dir) = &detached {
        if let Err(e) = crate::instance::InstanceStore::mark_unlinked(dir) {
            tracing::warn!("{:#}", e);
        }
    }
    let removed = state.supervisor.write().await.instance_store.remove(&id);
    if let Some(dir) = detached {
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
            state.provision_tracker.forget(&instance.name);
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
            }
            (StatusCode::OK, Json(response)).into_response()
        }
        Err(e) => {
            let error = json!({ "success": false, "error": format!("Failed to delete instance: {}", e) });
            (StatusCode::INTERNAL_SERVER_ERROR, Json(error)).into_response()
        }
    }
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
    // 설치·업데이트·복원 중에는 바꾸지 않는다: 진행 표시와 실패 정리가 이름으로 이어진다
    let Some(_guard) = super::backup::InstanceLock::take_for_writes(&id).await else {
        return super::backup::instance_busy();
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
    let running = supervisor.is_running(&id).await;
    if running {
        return fail(StatusCode::CONFLICT, "Stop the server before renaming it".into(), "server_running");
    }
    let previous = std::mem::replace(&mut instance.name, new_name.to_string());
    if let Err(e) = supervisor.instance_store.update(&id, instance) {
        return fail(StatusCode::INTERNAL_SERVER_ERROR, e.to_string(), "internal");
    }
    // 실패한 설치의 진행 표시는 이름으로 이어지므로 같은 잠금 안에서 함께 옮긴다
    state.provision_tracker.rename(&previous, new_name);
    Json(json!({ "success": true, "id": id, "name": new_name, "previous": previous })).into_response()
}

/// POST /api/instance/:id/launch-preview — 실행 인자 미리보기 `{settings?, extra_args?}`.
/// 서버를 띄우지도 파일을 고치지도 않는다. 응답 모양은 `Supervisor::launch_preview`
pub async fn launch_preview(
    State(state): State<IPCServer>,
    Path(id): Path<String>,
    LooseJson(body): LooseJson<Value>,
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
        Err(e) if e.to_string().contains("not found") => (StatusCode::NOT_FOUND, Json(crate::plugin::error_response(&e))).into_response(),
        Err(e) => (StatusCode::INTERNAL_SERVER_ERROR, Json(crate::plugin::error_response(&e))).into_response(),
    }
}

/// PATCH의 `stop_timeout`(정지 대기 초): 비움(null·""·0)은 모듈 값으로 되돌리기, 아니면 1~3600 정수 (숫자 글자도 받는다)
fn stop_timeout_from(value: &Value) -> Result<Option<u64>, ()> {
    let n = match value {
        Value::Null => return Ok(None),
        Value::String(s) if s.trim().is_empty() => return Ok(None),
        Value::String(s) => s.trim().parse::<u64>().map_err(|_| ())?,
        Value::Number(n) => n.as_u64().ok_or(())?,
        _ => return Err(()),
    };
    match n {
        0 => Ok(None),
        1..=3600 => Ok(Some(n)),
        _ => Err(()),
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

/// 설정의 저장된 값: 공통 칸(`constants::INSTANCE_FIELDS`, 예: port)은 PATCH가 인스턴스 칸에 쓰므로 그 값을,
/// 아니면 `module_settings`를 본다. `view`는 그 인스턴스를 JSON으로 펼친 것
pub(crate) fn stored_setting(view: &Value, instance: &crate::instance::ServerInstance, key: &str) -> Option<Value> {
    let own = saba_chan_updater_lib::constants::INSTANCE_FIELDS.iter().any(|(k, _)| *k == key);
    own.then(|| view.get(key).filter(|v| !v.is_null()).cloned()).flatten().or_else(|| instance.module_settings.get(key).cloned())
}

/// PATCH가 모듈 설정이 아니라 인스턴스 필드로 받는 이름 (그 밖의 이름은 `module_settings`로):
/// 공통 칸(`constants::INSTANCE_FIELDS`)과 익스텐션 값, 받지만 쓰지 않는 이름
fn is_known_field(key: &str) -> bool {
    saba_chan_updater_lib::constants::INSTANCE_FIELDS.iter().any(|(k, _)| *k == key) || key == "extension_data" || IGNORED_FIELDS.contains(&key)
}

/// 받지만 쓰지 않는 이름 (응답 `ignored`).
/// RCON은 보안상 쓰지 않는다: 예전 GUI·설정이 보내도 모듈 설정이나 게임 파일로 넘기지 않는다
const IGNORED_FIELDS: &[&str] = &["rcon_port", "rcon_password", "enable_rcon"];

/// 모듈이 선언하지 않아도 데몬이 `module_settings`에서 읽는 설정
fn is_daemon_setting(key: &str) -> bool {
    saba_chan_updater_lib::constants::DAEMON_SETTINGS.iter().any(|(k, _)| *k == key)
}

/// 포트 값: 1~65535의 수 또는 그런 숫자 문자열. 다른 값은 `None`
fn port_value(value: &Value) -> Option<u16> {
    match value {
        Value::Number(n) => n.as_u64(),
        Value::String(s) => s.trim().parse::<u64>().ok(),
        _ => None,
    }
    .and_then(|p| u16::try_from(p).ok())
    .filter(|&p| p > 0)
}

/// PATCH /api/instance/:id - 인스턴스 설정 업데이트 (보낸 키만). `revision`(GET에서 받은 값)을 주면
/// 그사이 다른 곳에서 바뀐 인스턴스에는 쓰지 않고 409 `settings_changed`로 거절한다
pub async fn update_instance_settings(
    State(state): State<IPCServer>,
    Path(id): Path<String>,
    Json(mut settings): Json<serde_json::Value>,
) -> impl IntoResponse {
    let revision = crate::config_store::patch::take_revision(&mut settings);
    // 서버 폴더를 옮기는 저장은 설치·업데이트·백업·복원이 그 폴더를 쓰지 않을 때만 (업데이트는 시작할 때
    // 계산한 폴더로 다른 인스턴스를 잠그고 받는다). 같은 값을 다시 보내는 저장은 막지 않는다.
    // 그 밖의 저장은 설정 파일 줄에 선다: 겹치면 늦게 끝난 게임 파일 쓰기가 나중 저장을 덮는다. 서버 파일을 바꾸는 작업 중이면 거절한다
    let moves_folder = {
        let supervisor = state.supervisor.read().await;
        supervisor.instance_store.get(&id).is_some_and(|i| settings.get("working_dir").is_some_and(|v| v.as_str() != i.working_dir.as_deref()))
    };
    let (_task, _saving) = if moves_folder {
        match super::backup::InstanceLock::take_for_writes(&id).await {
            Some(task) => (Some(task), None),
            None => return super::backup::instance_busy(),
        }
    } else {
        match super::backup::InstanceLock::files(&id).await {
            Some(files) => (None, Some(files)),
            None => return super::backup::instance_busy(),
        }
    };
    let mut supervisor = state.supervisor.write().await;

    // 인스턴스 찾기
    let instance = match supervisor.instance_store.get(&id) {
        Some(inst) => inst,
        None => {
            let error = json!({ "error": "Instance not found" });
            return (StatusCode::NOT_FOUND, Json(error)).into_response();
        }
    };

    let now = instance.settings_revision();
    if revision.is_some_and(|seen| seen != now) {
        let error = json!({ "success": false, "error": "The server settings were changed elsewhere; read them again", "error_code": "settings_changed", "revision": now });
        return (StatusCode::CONFLICT, Json(error)).into_response();
    }

    // 잠근 뒤 다시 본다: 폴더를 옮기는 다른 저장·작업은 위 잠금을 지나야 한다
    let moves_folder = settings.get("working_dir").is_some_and(|v| v.as_str() != instance.working_dir.as_deref());
    if moves_folder && _task.is_none() {
        return super::backup::instance_busy();
    }
    // 다른 인스턴스가 쓰는 폴더(서버 폴더나 인스턴스 폴더, 그 안이나 바깥)로는 옮기지 않는다: 만들 때처럼 한 폴더에 서버 둘을 두지 않는다.
    // 그 인스턴스의 서버·설치·업데이트가 옮긴 서버의 파일을 덮어쓴다
    if let Some(dir) = settings.get("working_dir").and_then(|v| v.as_str()).filter(|d| moves_folder && !d.is_empty()) {
        if let Some(other) = supervisor.instance_store.folder_user(std::path::Path::new(dir), &id) {
            let error = json!({
                "success": false,
                "error": format!("Server '{}' already uses this folder", other.name),
                "error_code": "working_dir_in_use",
                "instance": other.name,
            });
            return (StatusCode::CONFLICT, Json(error)).into_response();
        }
    }

    // ── 설정값 타입/범위 스키마 검증 ──
    if let Some(settings_obj) = settings.as_object() {
        if let Ok(module) = supervisor.module_loader.get_module(&instance.module_name) {
            if let Some(ref settings_meta) = module.metadata.settings {
                // 부분 변경이므로 보낸 필드만 검증한다
                // 숨겨진 설정은 required를 보지 않는다 (06 §8.2). 숨김은 저장된 값에 이번 요청을 얹어 계산한다
                let view = serde_json::to_value(instance).unwrap_or_default();
                let hidden = crate::validator::hidden_fields(&settings_meta.fields, |k| settings_obj.get(k).cloned().or_else(|| stored_setting(&view, instance, k)));
                let errors: Vec<_> = crate::validator::validate_settings_patch(
                    &settings_meta.fields,
                    settings_obj,
                )
                .into_iter()
                .filter(|e| !(e.error_type == crate::validator::ValidationErrorType::Required && hidden.contains(&e.field)))
                .collect();
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
    // 모듈이 설정 목록을 선언했을 때만 본다. 공통 필드와 데몬이 읽는 설정(`constants::DAEMON_SETTINGS`)은 받는다
    if let (Some(settings_obj), Ok(module)) = (settings.as_object(), supervisor.module_loader.get_module(&instance.module_name)) {
        if let Some(ref settings_meta) = module.metadata.settings {
            let unknown: Vec<&String> = settings_obj
                .keys()
                .filter(|k| !is_known_field(k) && !is_daemon_setting(k))
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
    // port·rest_port: 1~65535의 숫자 또는 숫자 문자열 (범위 밖 값을 다른 숫자로 저장하지 않는다)
    for key in ["port", "rest_port"] {
        // null(rest_port 지우기)과 빈 문자열(바꾸지 않음)은 예전처럼 받는다
        let given = settings.get(key).filter(|v| !v.is_null() && v.as_str() != Some(""));
        if given.is_some_and(|v| port_value(v).is_none()) {
            let error = json!({ "error": format!("{} must be a number from 1 to 65535", key), "error_code": "invalid_format", "fields": [{"field": key}] });
            return (StatusCode::BAD_REQUEST, Json(error)).into_response();
        }
    }
    if let Some(port) = settings.get("port").and_then(port_value) {
        updated.port = Some(port);
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
    if let Some(rest_port) = settings.get("rest_port").and_then(port_value) {
        updated.rest_port = Some(rest_port);
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
    // 정지 대기 시간(초): 비우거나(null·"") 0이면 모듈 값을 쓴다
    let stop_timeout = match settings.get("stop_timeout").map(stop_timeout_from) {
        Some(Err(())) => {
            let error = json!({ "error": "invalid_format", "error_code": "invalid_format", "message": "stop_timeout must be empty or a whole number of seconds from 1 to 3600", "fields": [{"field": "stop_timeout"}] });
            return (StatusCode::BAD_REQUEST, Json(error)).into_response();
        }
        other => other.map(Result::unwrap_or_default),
    };
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

    // 06 §8.2: 켜져 있을 때 값이 바뀐 restart_required 모듈 설정. 데몬은 스스로 재시작하지 않는다
    let running = supervisor.is_running(&id).await;
    let restart_required: Vec<String> = if running {
        // 공통 칸(port 등)은 PATCH 전 인스턴스 칸의 값과, 그 밖에는 module_settings와 비교한다
        let before = serde_json::to_value(instance).unwrap_or_default();
        supervisor.module_loader.get_module(&updated.module_name).ok()
            .and_then(|m| m.metadata.settings.clone())
            .map(|s| s.fields)
            .unwrap_or_default()
            .into_iter()
            .filter(|f| f.restart_required && settings.get(&f.name).is_some_and(|v| stored_setting(&before, instance, &f.name).as_ref() != Some(v)))
            .map(|f| f.name)
            .collect()
    } else {
        Vec::new()
    };

    // 동적 모듈 설정 저장 (하드코딩 필드 이외의 모든 설정을 module_settings에 저장)
    if let Some(obj) = settings.as_object() {
        for (key, value) in obj {
            if !is_known_field(key) {
                // 설정 화면은 고치지 않은 값도 함께 보낸다: 값이 바뀐 키만 사용자가 정한 것으로 기록한다
                if updated.module_settings.get(key) != Some(value) {
                    updated.user_settings.insert(key.clone());
                    updated.imported_settings.remove(key);
                }
                updated.module_settings.insert(key.clone(), value.clone());
            }
        }
    }
    match stop_timeout {
        Some(Some(n)) => {
            updated.module_settings.insert("stop_timeout".into(), json!(n));
        }
        Some(None) => {
            updated.module_settings.remove("stop_timeout");
            updated.user_settings.remove("stop_timeout");
        }
        None => {}
    }

    tracing::info!(
        "Updating instance {} with settings: port={:?}, executable_path={:?}, protocol_mode={}, module_settings_count={}",
        id,
        updated.port,
        updated.executable_path,
        updated.protocol_mode,
        updated.module_settings.len()
    );

    // 게임 설정 파일로 넘길 값 (configure lifecycle 호출). 데몬 필드, 데몬이 읽는 설정(DAEMON_SETTINGS),
    // 모듈이 사바쨩 설정으로 선언한 값은 뺀다
    const DAEMON_FIELDS: &[&str] = &[
        "protocol_mode", "executable_path", "working_dir", "server_version", "extension_data", "extra_args", "memory_limit_mb",
        "rest_host", "rest_port", "rest_username", "rest_password", "rcon_port", "rcon_password", "enable_rcon",
    ];
    let saba_fields: Vec<String> = supervisor.module_loader.get_module(&updated.module_name).ok()
        .and_then(|m| m.metadata.settings.as_ref().map(|s| s.saba_fields().into_iter().map(str::to_string).collect()))
        .unwrap_or_default();
    let mut props_sync = serde_json::Map::new();
    if let Some(obj) = settings.as_object() {
        for (key, value) in obj {
            if DAEMON_FIELDS.contains(&key.as_str()) || is_daemon_setting(key) || saba_fields.contains(key) {
                continue;
            }
            props_sync.insert(key.clone(), value.clone());
        }
    }

    // 저장
    let updated_clone = updated.clone();
    let new_revision = updated_clone.settings_revision();
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
    let hook_instance = instance_for_hooks(module.as_ref().map(|m| &m.metadata), &updated_clone);
    let hook_settings = settings_for_hooks(module.as_ref().map(|m| &m.metadata), &settings);
    let module_extensions = module
        .map(|m| serde_json::to_value(&m.metadata.extensions).unwrap_or_default())
        .unwrap_or_else(|| serde_json::json!({}));
    drop(supervisor);

    // 게임 설정 파일 동기화 (변경된 항목이 있을 때만). 실패해도 저장은 남기고 응답에 싣는다
    let mut sync_error: Option<Value> = None;
    if !props_sync.is_empty() {
        // 값에는 비밀번호가 들 수 있어 키만 남긴다
        tracing::info!(
            "Syncing settings to the game config for instance {}: {:?}",
            id,
            props_sync.keys().collect::<Vec<_>>()
        );
        let props_value = Value::Object(props_sync);
        match crate::supervisor::Supervisor::manage_properties(&state.supervisor, &id, "write", Some(props_value)).await {
            // 모듈에 configure가 없으면 쓸 게임 설정 파일도 없다
            Ok(r) if r.get("success").and_then(|v| v.as_bool()) == Some(false)
                && r.get("error_code").and_then(|v| v.as_str()) != Some("not_supported")
                && !r.get("message").and_then(|v| v.as_str()).is_some_and(|m| m.starts_with("Unknown function")) => {
                let message = r.get("message").or(r.get("error")).and_then(|v| v.as_str()).unwrap_or("configure failed");
                tracing::warn!("Module could not write game settings for instance {}: {}", id, message);
                sync_error = Some(r);
            }
            Ok(_) => tracing::info!("Game config synced for instance {}", id),
            Err(e) => {
                tracing::warn!("Failed to sync the game config for instance {}: {}", id, e);
                sync_error = Some(crate::plugin::error_response(&e));
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
            "instance": hook_instance,
            "module_extensions": module_extensions,
            "extension_data": &updated_clone.extension_data,
            "settings": hook_settings,
        });
        let _results = crate::plugin::with_instance(Some(state.host_context(&id)), crate::extension::run_hook(&ext_mgr, "server.settings_changed", ctx)).await;
    }

    // 게임 설정 파일의 비밀번호·포트가 바뀌었을 수 있다
    state.credentials.invalidate(&id);
    let mut body = json!({ "success": true, "synced": sync_error.is_none(), "restart_required": restart_required, "revision": new_revision });
    if let Some(e) = sync_error {
        body["sync_error"] = e.get("message").or_else(|| e.get("error")).cloned().unwrap_or(Value::Null);
        // 설정은 이미 저장했다: 게임 파일 동기화 실패의 칸(오류 코드 등)만 덧붙이고 성공 표시는 덮지 않는다 (Codex 리뷰 #550)
        if let (Some(body), Some(fields)) = (body.as_object_mut(), e.as_object()) {
            body.extend(fields.iter().filter(|(k, _)| k.as_str() != "success").map(|(k, v)| (k.clone(), v.clone())));
        }
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
        "instance_id": id,
        "instance_name": &instance.name,
        "module": &instance.module_name,
        "instance_dir": instance_dir.to_string_lossy(),
        "install_dir": &install_dir_str,
        "app_id": app_id,
        "extension_data": &instance.extension_data,
    });

    let results = crate::plugin::with_instance(Some(state.host_context(id)), crate::extension::run_hook(&state.extension_manager, "server.check_update", ctx)).await;

    // 첫 번째 성공 결과를 클라이언트로 전달
    if let Some(val) = results.iter().find_map(|(_, result)| result.as_ref().ok()) {
        return Some(val.clone());
    }
    // 모두 실패했으면 그 오류를 구조화해 돌려준다
    if let Some(error) = results.iter().find_map(|(_, result)| result.as_ref().err()) {
        return Some(crate::plugin::error_response(error));
    }

    // hook이 등록되지 않았거나 모두 condition 불일치
    Some(json!({
        "update_available": false,
        "reason": "no_hook_handled",
    }))
}

/// 설치 방식 익스텐션(`install_method_<방식>`)으로 설치한 인스턴스인가. 이런 서버만 사바쨩이 업데이트를 확인·적용한다
fn installed_by_extension(instance: &crate::instance::ServerInstance, method: &str) -> bool {
    !crate::instance::BUILTIN_INSTALL_METHODS.contains(&method) && instance.ext_enabled(&crate::instance::install_method_key(method))
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
    let Some(guard) = super::backup::InstanceLock::take_for_writes(id).await else {
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
    if supervisor.is_running(id).await {
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
    if !installed_by_extension(&instance, &method) {
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
    let module = supervisor.module_loader.get_module(&instance.module_name).ok();
    let boundary = crate::module_paths::Boundary::for_instance(&instance, &instance_dir, module.as_ref().map(|m| &m.metadata));
    let host_context = state.host_context(id);

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
            steps: None, details: Default::default(),
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
            let failure = crate::ipc::internal::into_internal(r).await.body;
            let reason = failure["error"].as_str().unwrap_or("backup failed").to_string();
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
                    steps: None, details: Default::default(),
                }.with_failure(failure),
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
                        steps: None, details: Default::default(),
                    },
                );
            };

            crate::plugin::PluginCall::new(&lifecycle_path, "install_server", config)
                .in_instance(Some(host_context)).timeout(crate::plugin::INSTALL_TIMEOUT_SECS)
                .run_with_progress(Box::new(on_progress)).await
                .and_then(|mut value| {
                    if value.get("success").and_then(Value::as_bool) == Some(true) {
                        boundary.returned_paths(&mut value, &module_name_owned, "install_server")?;
                    }
                    Ok(value)
                })
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
                            steps: None, details: Default::default(),
                        }.with_failure(val.clone()),
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

/// 익스텐션 hook에 넘기는 인스턴스: REST 비밀번호, 모듈이 password로 선언했거나 이름이 비밀스러운 설정 값은 가린다 (D-E2)
fn instance_for_hooks(module: Option<&crate::supervisor::module_loader::ModuleMetadata>, instance: &crate::instance::ServerInstance) -> Value {
    settings_for_hooks(module, &serde_json::to_value(instance).unwrap_or_default())
}

/// hook에 넘기는 설정 값에서 같은 기준으로 비밀을 가린다. 키 이름으로만 가린다:
/// 인스턴스를 넘기면 비밀값과 같은 글자가 이름·경로에서도 지워진다
fn settings_for_hooks(module: Option<&crate::supervisor::module_loader::ModuleMetadata>, settings: &Value) -> Value {
    crate::audit::redact(module, None, None, Some(settings)).1.unwrap_or_default()
}

#[cfg(test)]
mod tests {
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
        assert!(super::super::backup::InstanceLock::take(&id).is_none(), "the import holds the lock");
        let saved = crate::ipc::internal::into_internal(super::update_instance_settings(State(state.clone()), Path(id.clone()), Json(json!({ "motd": "theirs" }))).await).await;
        assert_eq!(saved.status, 409, "{}", saved.body);
        let created = crate::ipc::internal::into_internal(request.await.unwrap()).await;
        assert_eq!(created.status, 201, "{}", created.body);
        assert!(super::super::backup::InstanceLock::take(&id).is_some(), "the lock is released once the import is done");
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

    use super::{memory_limit_from, stop_timeout_from};
    use serde_json::json;

    /// 가져오기의 `import_settings`는 포터블 런타임을 받을 수 있어 설치와 같은 시간 한도를 받고,
    /// 이 호출만 인스턴스 폴더에 쓴다는 표시를 싣는다 (기본 120초면 느린 회선에서 가져온 설정을 모두 잃는다)
    #[test]
    fn the_migration_import_waits_like_an_install_and_is_marked() {
        let call = super::migration_import_call("game/lifecycle.py", Some("/srv/game"), std::path::Path::new("/data/instances/a"));
        assert_eq!(call.timeout_secs(), crate::plugin::INSTALL_TIMEOUT_SECS);
        assert_eq!(call.config()[crate::sandbox::MIGRATION_IMPORT], json!(true));
        assert_eq!(call.config()["working_dir"], json!("/srv/game"));
        assert_eq!(call.config()["instance_dir"], json!("/data/instances/a"));
    }

    /// 설치가 끝났다는 표시를 기다린다. 상한은 멈춘 설치를 알아채려는 것이라, 병렬 테스트로 느려진 PC·CI에서도 넉넉히 둔다
    async fn provisioned(state: &crate::ipc::IPCServer, name: &str) -> Option<crate::ipc::ProvisionProgress> {
        for _ in 0..2400 {
            if let Some(progress) = state.provision_tracker.get(name).filter(|p| p.done) {
                return Some(progress);
            }
            tokio::time::sleep(std::time::Duration::from_millis(50)).await;
        }
        None
    }

    /// 익스텐션 hook에 넘기는 인스턴스는 REST 비밀번호와 비밀스러운 설정 값을 가리고, 나머지는 그대로 둔다 (D-E2)
    #[test]
    fn hooks_get_the_instance_without_its_secrets() {
        let mut inst = crate::instance::ServerInstance::new("srv", "game");
        inst.rest_password = Some("hunter2-rest".into());
        inst.module_settings.insert("server_password".into(), json!("hunter2-join"));
        inst.module_settings.insert("max_players".into(), json!(16));
        let value = super::instance_for_hooks(None, &inst);
        let text = value.to_string();
        assert!(!text.contains("hunter2"), "{text}");
        assert_eq!(value["module_settings"]["max_players"], 16);
        assert_eq!(value["name"], "srv");
        let settings = super::settings_for_hooks(None, &json!({ "rest_password": "hunter2-new", "port": 7777 }));
        assert!(!settings.to_string().contains("hunter2"), "{settings}");
        assert_eq!(settings["port"], 7777);
        // 비밀값과 같은 글자의 이름·경로는 비밀이 아니다: 값 대신 키 이름으로만 가린다
        let mut same = crate::instance::ServerInstance::new("palworld", "game");
        same.working_dir = Some("C:/srv/palworld/data".into());
        same.rest_password = Some("palworld".into());
        let value = super::instance_for_hooks(None, &same);
        assert_eq!(value["name"], "palworld");
        assert_eq!(value["working_dir"], "C:/srv/palworld/data");
        assert_ne!(value["rest_password"], "palworld");
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
        let live = crate::ipc::internal::into_internal(super::super::managed::read_live_settings_handler(Path(id.clone()), State(state.clone())).await).await;
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

        let update = super::super::backup::InstanceLock::take_for_writes(&id).await.unwrap();
        let refused = save("during update").await;
        assert_eq!(refused.status, axum::http::StatusCode::CONFLICT);
        assert_eq!(refused.body["error_code"], "instance_busy");
        let agreement = crate::ipc::internal::into_internal(
            super::super::managed::accept_agreements_handler(Path(id.clone()), State(state.clone()), Json(json!({ "accept": ["eula"] }))).await,
        )
        .await;
        assert_eq!(agreement.status, axum::http::StatusCode::CONFLICT);
        drop(update);

        let backup = super::super::backup::InstanceLock::take(&id).unwrap();
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

        for (key, value) in [("port", json!(65537)), ("port", json!(0)), ("port", json!("70000")), ("rest_port", json!(65537)), ("rest_port", json!("x"))] {
            let bad = patch(json!({ key: value.clone() })).await;
            assert_eq!((bad.status.as_u16(), bad.body["error_code"].as_str(), bad.body["fields"][0]["field"].as_str()), (400, Some("invalid_format"), Some(key)), "{key}={value}");
        }
        let inst = state.supervisor.read().await.instance_store.get(&id).cloned().unwrap();
        assert_ne!(inst.port, Some(1));
        assert_eq!(inst.rest_port, None);
        assert!(patch(json!({ "port": 25570, "rest_port": "8213" })).await.status.is_success());
        let inst = state.supervisor.read().await.instance_store.get(&id).cloned().unwrap();
        assert_eq!((inst.port, inst.rest_port), (Some(25570), Some(8213)));
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
             [[settings.fields]]\nname = \"rest_host\"\ntype = \"text\"\nrestart_required = true\n\n\
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
        sup.migrate_list_settings();
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
        // 인스턴스 칸에 저장되는 설정(rest_host)은 그 칸의 지금 값과 비교한다
        let running = patch(json!({ "rest_host": "10.0.0.2" })).await;
        assert_eq!(running.body["restart_required"], json!(["rest_host"]));
        let running = patch(json!({ "rest_host": "10.0.0.2" })).await;
        assert_eq!(running.body["restart_required"], json!([]), "saving the same value again asks for nothing");
        state.supervisor.read().await.tracker.untrack(&id).unwrap();
        let stored = std::fs::read_to_string(state.supervisor.read().await.instance_store.instance_dir(&id).join("settings.json")).unwrap();
        assert!(stored.contains("\"Alex\""), "{stored}");
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
            "stop_timeout": 90, "memory_limit_mb": "", "extra_args": "-Dx=1",
        });
        let saved = crate::ipc::internal::into_internal(super::update_instance_settings(State(state), Path(id), Json(body)).await).await;
        assert!(saved.status.is_success(), "{:?}", saved.body);
        // 받은 키는 실패 메시지로 돌려받는다: Windows에서 확인되지 않은 모듈은 격리되어 모듈 폴더에 쓸 수 없다
        let configured: Vec<String> = serde_json::from_str(saved.body["sync_error"].as_str().unwrap()).unwrap();
        assert_eq!(configured, vec!["motd", "ram"], "a saba-chan setting and daemon fields stay out; a game setting named like another game's launcher setting goes in");
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
        let other = super::super::backup::InstanceLock::take(&ids[1]).expect("free");
        let refused = super::begin_server_update(&state, &ids[0]).await.err().expect("the other instance is busy");
        let body = crate::ipc::internal::into_internal(refused).await.body;
        assert_eq!(body["error_code"], "instance_busy");
        drop(other);
        assert!(super::super::backup::InstanceLock::take(&ids[0]).is_some(), "a refused update must not keep its own lock");
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

        let busy = super::super::backup::InstanceLock::take(&id).unwrap();
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
        assert!(super::super::backup::InstanceLock::take(&id).is_none(), "the install holds the lock");
        // 시간이 아니라 설치가 끝났다는 표시를 기다린다. 잠금은 그 표시보다 먼저 풀린다
        let done = provisioned(&state, "a").await;
        let done = done.expect("the install never finished");
        assert!(done.error.is_none(), "{:?}", done.error);
        assert!(super::super::backup::InstanceLock::take(&id).is_some(), "the lock is released once the install is done");
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
        assert!(state.provision_tracker.get("a").is_some_and(|p| p.error.is_some()), "the install should have failed");

        // 실패 뒤 dismiss 전에 이름을 바꾸고, 그 이름으로 다른 인스턴스를 만든다
        let renamed = crate::ipc::internal::into_internal(super::rename_instance(State(state.clone()), Path(a.clone()), Json(json!({ "name": "b" }))).await).await;
        assert_eq!(renamed.status, 200, "{}", renamed.body);
        let c = crate::ipc::internal::into_internal(super::create_instance(State(state.clone()), Json(json!({ "name": "a", "module_name": "plain" }))).await).await;
        let c = c.body["id"].as_str().unwrap().to_string();

        // 실패 표시는 이름과 함께 옮겨 간다: 같은 이름의 새 인스턴스가 설치 중으로 보이거나 그 배너로 A가 지워지지 않게
        assert!(state.provision_tracker.get("a").is_none(), "the new instance with the old name is not provisioning");
        let other = crate::ipc::internal::into_internal(super::dismiss_provision_progress(Path("a".to_string()), State(state.clone())).await).await;
        assert_eq!(other.body["rolled_back"], false);
        assert!(state.supervisor.read().await.instance_store.get(&a).is_some(), "dismissing another name's banner keeps A");
        let dismissed = crate::ipc::internal::into_internal(super::dismiss_provision_progress(Path("b".to_string()), State(state.clone())).await).await;
        assert_eq!(dismissed.body["rolled_back"], true);
        let sup = state.supervisor.read().await;
        assert!(sup.instance_store.get(&c).is_some(), "another instance with the old name must survive");
        assert!(sup.instance_store.get(&a).is_none(), "the failed install is rolled back");
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
}
