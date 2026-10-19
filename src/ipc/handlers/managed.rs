use axum::{
    extract::{Path, State},
    http::StatusCode,
    response::IntoResponse,
    Json,
};
use serde_json::{json, Value};
use std::collections::HashMap;

use super::super::IPCServer;
use crate::supervisor::Supervisor;

/// POST /api/instance/:id/managed/start 라우트: 시작을 감사에 남긴다 (서버 시작 경로가 안에서 부를 때는 그쪽이 남긴다)
pub async fn start_managed_route(
    Path(id): Path<String>,
    State(state): State<IPCServer>,
    payload: Option<Json<serde_json::Value>>,
) -> axum::response::Response {
    let resp = start_managed_handler(Path(id.clone()), State(state.clone()), payload).await.into_response();
    super::audited(&state, "start", &id, None, None, resp).await
}

/// POST /api/instance/:id/managed/start — Start with managed process (stdin/stdout capture)
pub async fn start_managed_handler(
    Path(id): Path<String>,
    State(state): State<IPCServer>,
    payload: Option<Json<serde_json::Value>>,
) -> impl IntoResponse {
    // 백업·복원·업데이트가 서버 파일을 다루는 동안에는 켜지 않는다 (시작을 마칠 때까지 잡는다)
    let Some(_guard) = super::backup::BusyGuard::take(&id) else {
        return super::backup::instance_busy();
    };
    // ── 실행 전 비밀번호 자동 생성 (비어있으면 채움) ──
    {
        let mut supervisor = state.supervisor.write().await;
        if let Some(mut inst) = supervisor.instance_store.get(&id).cloned() {
            let changed = inst.ensure_passwords();

            if changed {
                if let Err(e) = supervisor.instance_store.update(&id, inst) {
                    tracing::warn!("Failed to save auto-generated passwords for {}: {}", id, e);
                }
            }
        }
    }

    let (module_name, config) = {
        let supervisor = state.supervisor.read().await;

        let instance = match supervisor.instance_store.get(&id) {
            Some(i) => i,
            None => {
                return (
                    StatusCode::NOT_FOUND,
                    Json(json!({"error": format!("Instance not found: {}", id)})),
                )
                    .into_response()
            }
        };

        // API 경유 시작 기록 (name과 id 모두)
        state.api_actions.record(&instance.name);
        state.api_actions.record(&id);

        let module_name = instance.module_name.clone();
        let payload_val = payload.map(|j| j.0).unwrap_or(json!({}));
        let config = payload_val.get("config").cloned().unwrap_or(json!({}));
        (module_name, config)
    };

    // 락은 start_managed_server가 단계마다 짧게 잡는다 (C12)
    match Supervisor::start_managed_server(&state.supervisor, &id, &module_name, config).await {
        Ok(result) => (StatusCode::OK, Json(result)).into_response(),
        Err(e) => match e.downcast_ref::<crate::supervisor::managed_process::MemoryLimitAtStart>() {
            Some(hit) => {
                let language = state.config_store.ui_language().await;
                let message = crate::chat::i18n::Catalog::for_language(&language)
                    .t("errors.memory_limit_at_start", &[("limit", &hit.limit_mb.to_string())]);
                (
                    StatusCode::UNPROCESSABLE_ENTITY,
                    Json(json!({ "success": false, "error": message, "error_code": "memory_limit_reached", "limit_mb": hit.limit_mb })),
                )
                    .into_response()
            }
            None => (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(json!({"error": e.to_string()})),
            )
                .into_response(),
        },
    }
}

/// GET /api/instance/:id/console?since=0&count=100&spans=1 — Get console output
/// (`spans=1`이면 줄마다 하이라이팅 토큰 구간을 붙인다)
pub async fn get_console_handler(
    Path(id): Path<String>,
    State(state): State<IPCServer>,
    axum::extract::Query(params): axum::extract::Query<HashMap<String, String>>,
) -> impl IntoResponse {
    let supervisor = state.supervisor.read().await;

    let since_id = params.get("since").and_then(|s| s.parse::<u64>().ok());
    let count = params.get("count").and_then(|c| c.parse::<usize>().ok());

    let result = if params.get("spans").is_some_and(|v| v == "1" || v == "true") {
        supervisor.get_console_output_with_spans(&id, since_id, count).await
    } else {
        supervisor.get_console_output(&id, since_id, count).await
    };
    match result {
        Ok(result) => (StatusCode::OK, Json(result)).into_response(),
        Err(e) => (
            StatusCode::NOT_FOUND,
            Json(json!({"error": e.to_string(), "error_code": "not_found"})),
        )
            .into_response(),
    }
}

/// POST /api/instance/:id/stdin — Send command to stdin
pub async fn send_stdin_handler(
    Path(id): Path<String>,
    State(state): State<IPCServer>,
    Json(payload): Json<serde_json::Value>,
) -> impl IntoResponse {
    // 콘솔 입력을 받지 않는 모듈(예: REST 전용)은 보낸 척하지 않는다: 클라이언트가 명령 API로 넘어가게
    let resp = if !state.supervisor.read().await.module_accepts_stdin(&id) {
        (
            StatusCode::CONFLICT,
            Json(json!({ "error": "This server does not take console input; use its commands instead", "error_code": "stdin_not_supported" })),
        )
            .into_response()
    } else {
        // 명령 경로와 같은 응답: 보낸 뒤 찍힌 콘솔 줄을 data.response로
        super::command::via_stdin(&state, &id, &payload).await.into_response()
    };
    let line = payload.get("command").or_else(|| payload.get("line")).and_then(|v| v.as_str()).map(str::to_string);
    super::audited(&state, "stdin", &id, line.as_deref(), None, resp).await
}

/// POST /api/instance/:id/validate — Validate prerequisites
pub async fn validate_instance_handler(
    Path(id): Path<String>,
    State(state): State<IPCServer>,
) -> impl IntoResponse {
    match Supervisor::validate_instance(&state.supervisor, &id).await {
        Ok(result) => (StatusCode::OK, Json(result)).into_response(),
        Err(e) => (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(json!({"error": e.to_string()})),
        )
            .into_response(),
    }
}

/// POST /api/instance/:id/query/:name `{args?: {입력: 값}}` — 모듈 `[[queries]]` 읽기 전용 조회 (X-2).
/// 입력은 명령 입력과 같은 규칙으로 검사한다. lifecycle 결과 `{success, data?, message?}` → `{success, data, error?}`
/// (`data`가 없으면 `success`·`message`를 뺀 나머지). 읽기만 하므로 감사에 남기지 않는다
pub async fn query_instance_handler(
    Path((id, name)): Path<(String, String)>,
    State(state): State<IPCServer>,
    payload: Option<Json<serde_json::Value>>,
) -> impl IntoResponse {
    let fail = |status: StatusCode, code: &str, error: String, field: Option<&str>| {
        (status, Json(json!({ "success": false, "error_code": code, "error": error, "field": field }))).into_response()
    };
    let query = {
        let sup = state.supervisor.read().await;
        let Some(instance) = sup.instance_store.get(&id) else {
            return fail(StatusCode::NOT_FOUND, "not_found", format!("Instance not found: {}", id), None);
        };
        sup.module_loader.get_module(&instance.module_name).ok().and_then(|m| m.metadata.queries.into_iter().find(|q| q.name == name))
    };
    let Some(query) = query else {
        return fail(StatusCode::NOT_FOUND, "unknown_query", format!("Unknown query: {}", name), None);
    };
    let args = match payload.as_ref().and_then(|p| p.get("args")) {
        None | Some(serde_json::Value::Null) => serde_json::Map::new(),
        Some(serde_json::Value::Object(m)) => m.clone(),
        Some(_) => return fail(StatusCode::BAD_REQUEST, "invalid_argument", "'args' must be an object".into(), Some("args")),
    };
    if let Some(key) = args.keys().find(|k| !query.inputs.iter().any(|i| &i.name == *k)) {
        return fail(StatusCode::BAD_REQUEST, "invalid_argument", format!("Unknown input '{}'", key), Some(key));
    }
    let args = match crate::commands::bind(&query.inputs, &crate::commands::Args::Named(args)) {
        Ok(a) => a,
        Err(e) => return fail(StatusCode::BAD_REQUEST, e.code, e.message, e.field.as_deref()),
    };
    match Supervisor::query_instance(&state.supervisor, &id, &query.function, args).await {
        Ok(serde_json::Value::Object(mut result)) => {
            let success = result.remove("success").and_then(|v| v.as_bool()).unwrap_or(true);
            let message = result.remove("message");
            let data = result.remove("data").unwrap_or(serde_json::Value::Object(result));
            let mut body = json!({ "success": success, "data": data });
            if !success {
                body["error"] = message.unwrap_or_else(|| json!("Query failed"));
                body["error_code"] = json!("query_failed");
            }
            (StatusCode::OK, Json(body)).into_response()
        }
        Ok(other) => (StatusCode::OK, Json(json!({ "success": true, "data": other }))).into_response(),
        Err(e) => {
            // 원문에는 PC 경로나 모듈이 받은 설정값이 섞일 수 있어 원격에는 내보내지 않는다
            tracing::warn!("Query '{}' on '{}' failed: {}", name, id, e);
            let message = if crate::audit::current().source == "discord" { "Query failed".to_string() } else { e.to_string() };
            fail(StatusCode::INTERNAL_SERVER_ERROR, "query_failed", message, None)
        }
    }
}

/// GET /api/instance/:id/properties — Read server.properties
pub async fn read_properties_handler(
    Path(id): Path<String>,
    State(state): State<IPCServer>,
) -> impl IntoResponse {
    match Supervisor::manage_properties(&state.supervisor, &id, "read", None).await {
        Ok(result) => (StatusCode::OK, Json(result)).into_response(),
        Err(e) => (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(json!({"error": e.to_string()})),
        )
            .into_response(),
    }
}

/// PUT /api/instance/:id/properties — Update server.properties
pub async fn write_properties_handler(
    Path(id): Path<String>,
    State(state): State<IPCServer>,
    Json(payload): Json<serde_json::Value>,
) -> impl IntoResponse {
    let settings = payload.get("settings").cloned();
    let resp = match Supervisor::manage_properties(&state.supervisor, &id, "write", settings.clone()).await {
        Ok(result) => (StatusCode::OK, Json(result)).into_response(),
        Err(e) => (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(json!({"error": e.to_string()})),
        )
            .into_response(),
    };
    super::audited(&state, "properties", &id, None, settings.as_ref(), resp).await
}

/// GET /api/instance/:id/settings/live — 디스크의 실제 설정을 모듈 `import_settings`로 읽어 돌려준다.
///
/// `module_settings`는 마지막으로 saba-chan이 저장한 값의 캐시라 게임 안이나 파일에서 바꾼 값과
/// 어긋날 수 있다. 읽기에 성공하면 캐시도 실제 값으로 맞춘다. 실패하면 `success: false`를 그대로
/// 돌려주므로, 인터페이스는 그때 기본값으로 덮어쓰지 말아야 한다. Python은 락 밖에서 돈다.
pub async fn read_live_settings_handler(
    Path(id): Path<String>,
    State(state): State<IPCServer>,
) -> impl IntoResponse {
    match Supervisor::manage_properties(&state.supervisor, &id, "import", None).await {
        Ok(result) => {
            let ok = result.get("success").and_then(|v| v.as_bool()) == Some(true);
            let settings = result.get("settings").and_then(|v| v.as_object()).filter(|s| !s.is_empty());
            if let (true, Some(settings)) = (ok, settings) {
                let mut supervisor = state.supervisor.write().await;
                if let Some(mut inst) = supervisor.instance_store.get(&id).cloned() {
                    let (credential_map, types) = supervisor.module_loader.get_module(&inst.module_name)
                        .map(|m| (m.metadata.credential_map.clone(), m.metadata.setting_types()))
                        .unwrap_or_default();
                    // 포트·비밀번호 같은 인스턴스 필드도 파일 값으로 맞춘다 (GUI는 그쪽을 먼저 읽는다)
                    inst.apply_imported_settings(settings, &credential_map, &types);
                    if let Err(e) = supervisor.instance_store.update(&id, inst) {
                        tracing::warn!("Live settings read: cache refresh failed for {}: {}", id, e);
                    }
                }
            }
            (StatusCode::OK, Json(result)).into_response()
        }
        Err(e) => (StatusCode::INTERNAL_SERVER_ERROR, Json(json!({ "success": false, "error": e.to_string() }))).into_response(),
    }
}

/// POST /api/instance/:id/properties/reset — Reset server.properties to defaults
pub async fn reset_properties_handler(
    Path(id): Path<String>,
    State(state): State<IPCServer>,
) -> impl IntoResponse {
    match Supervisor::manage_properties(&state.supervisor, &id, "reset", None).await {
        Ok(result) => (StatusCode::OK, Json(result)).into_response(),
        Err(e) => (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(json!({"error": e.to_string()})),
        )
            .into_response(),
    }
}

/// 켜져 있는 서버의 파일을 바꾸는 작업을 거절하는 409
fn server_running() -> axum::response::Response {
    let error = json!({ "success": false, "error": "Stop the server first", "error_code": "server_running" });
    (StatusCode::CONFLICT, Json(error)).into_response()
}

/// `dir`을 쓰는 인스턴스(working_dir 또는 인스턴스 폴더)의 작업 잠금을 모두 잡는다. `except`(이미 잠금을 쥔
/// 인스턴스)는 건너뛴다. 다른 작업 중이면 409 `instance_busy`, 켜져 있으면 409 `server_running`
#[allow(clippy::result_large_err)] // 오류는 바로 돌려줄 HTTP 응답
pub(super) async fn hold_other_instances_using(state: &IPCServer, dir: &str, except: Option<&str>) -> Result<Vec<super::backup::BusyGuard>, axum::response::Response> {
    let ids: Vec<String> = {
        let supervisor = state.supervisor.read().await;
        let dir = std::path::Path::new(dir);
        supervisor.instance_store.list().iter()
            .filter(|i| {
                i.working_dir.as_deref().is_some_and(|d| saba_chan_updater_lib::same_path(std::path::Path::new(d), dir))
                    || saba_chan_updater_lib::same_path(&supervisor.instance_store.instance_dir(&i.id), dir)
            })
            .filter(|i| except != Some(i.id.as_str()))
            .map(|i| i.id.clone())
            .collect()
    };
    let mut guards = Vec::new();
    for id in ids {
        let Some(guard) = super::backup::BusyGuard::take(&id) else {
            return Err(super::backup::instance_busy());
        };
        guards.push(guard);
        if super::backup::is_running(state, &id).await {
            return Err(server_running());
        }
    }
    Ok(guards)
}

/// POST /api/instance/:id/server/reset — Full server reset (delete worlds, configs, etc.)
/// 다른 작업 중이면 409 `instance_busy`, 켜져 있으면 409 `server_running`
pub async fn reset_server_handler(
    Path(id): Path<String>,
    State(state): State<IPCServer>,
) -> impl IntoResponse {
    let Some(guard) = super::backup::BusyGuard::take(&id) else {
        return super::backup::instance_busy();
    };
    if super::backup::is_running(&state, &id).await {
        return server_running();
    }
    // 요청이 끊겨도 모듈이 파일을 지우는 동안 잠금을 쥔다
    let reset = {
        let (state, id) = (state.clone(), id.clone());
        crate::audit::spawn(async move {
            let _guard = guard;
            Supervisor::manage_properties(&state.supervisor, &id, "reset_server", None).await.map_err(|e| e.to_string())
        })
    };
    let resp = match reset.await.unwrap_or_else(|e| Err(e.to_string())) {
        Ok(result) => (StatusCode::OK, Json(result)).into_response(),
        Err(e) => (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(json!({"error": e})),
        )
            .into_response(),
    };
    super::audited(&state, "reset", &id, None, None, resp).await
}

/// 요청 본문의 약관 id 목록 (`key`: 문자열 배열, 없으면 빈 목록). 동의는 이 PC(GUI·CLI)에서만 받는다:
/// 원격(디스코드·챗봇)에서 온 요청이 하나라도 담으면 403 `agreement_local_only`
pub(crate) fn requested_agreements(payload: &Value, key: &str) -> Result<Vec<String>, Box<axum::response::Response>> {
    let ids: Vec<String> = match payload.get(key) {
        None | Some(Value::Null) => Vec::new(),
        Some(Value::Array(items)) if items.iter().all(Value::is_string) => {
            items.iter().filter_map(Value::as_str).map(str::to_string).collect()
        }
        Some(_) => {
            let error = json!({ "error": format!("'{}' must be a list of agreement ids", key), "error_code": "invalid_format", "fields": [{"field": key}] });
            return Err(Box::new((StatusCode::BAD_REQUEST, Json(error)).into_response()));
        }
    };
    if !ids.is_empty() && crate::audit::current().source == "discord" {
        let error = json!({ "success": false, "error": "Agreements can only be accepted on this PC", "error_code": "agreement_local_only" });
        return Err(Box::new((StatusCode::FORBIDDEN, Json(error)).into_response()));
    }
    Ok(ids)
}

/// POST /api/instance/:id/agreements `{accept:[id]}` — 모듈 약관 동의 (이 PC에서만)
pub async fn accept_agreements_handler(
    Path(id): Path<String>,
    State(state): State<IPCServer>,
    Json(payload): Json<Value>,
) -> axum::response::Response {
    let ids = match requested_agreements(&payload, "accept") {
        Ok(ids) => ids,
        Err(resp) => return *resp,
    };
    match Supervisor::accept_agreements(&state.supervisor, &id, &ids).await {
        Ok(Ok(module_result)) => {
            if module_result.get("success").and_then(Value::as_bool) == Some(false) {
                let message = module_result.get("message").and_then(Value::as_str).unwrap_or("Module failed to apply the agreement");
                return (StatusCode::INTERNAL_SERVER_ERROR, Json(json!({ "success": false, "error": message }))).into_response();
            }
            let language = state.config_store.ui_language().await;
            let sup = state.supervisor.read().await;
            let agreements = sup.instance_store.get(&id).map(|i| sup.agreement_views(i, &language)).unwrap_or_default();
            (StatusCode::OK, Json(json!({ "success": true, "agreements": agreements }))).into_response()
        }
        Ok(Err(unknown)) => {
            let error = json!({ "error": format!("This server's module has no agreement '{}'", unknown), "error_code": "invalid_format", "fields": [{"field": "accept"}] });
            (StatusCode::BAD_REQUEST, Json(error)).into_response()
        }
        Err(e) if e.to_string().starts_with("Instance not found") => {
            (StatusCode::NOT_FOUND, Json(json!({ "error": e.to_string(), "error_code": "not_found" }))).into_response()
        }
        Err(e) => (StatusCode::INTERNAL_SERVER_ERROR, Json(json!({ "error": e.to_string() }))).into_response(),
    }
}

/// POST /api/instance/:id/diagnose — Diagnose errors
pub async fn diagnose_handler(
    Path(id): Path<String>,
    State(state): State<IPCServer>,
) -> impl IntoResponse {
    match Supervisor::diagnose_instance(&state.supervisor, &id).await {
        Ok(result) => (StatusCode::OK, Json(result)).into_response(),
        Err(e) => (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(json!({"error": e.to_string()})),
        )
            .into_response(),
    }
}

// ─── Server Installation Handlers ────────────────────────────

/// GET /api/module/:name/versions?include_snapshots=false&page=1&per_page=25
/// List available Minecraft server versions from Mojang
pub async fn list_versions_handler(
    Path(module_name): Path<String>,
    State(state): State<IPCServer>,
    axum::extract::Query(params): axum::extract::Query<HashMap<String, String>>,
) -> impl IntoResponse {
    let include_snapshots = params
        .get("include_snapshots")
        .map(|v| v == "true" || v == "1")
        .unwrap_or(false);
    let page = params
        .get("page")
        .and_then(|v| v.parse::<u32>().ok())
        .unwrap_or(1);
    let per_page = params
        .get("per_page")
        .and_then(|v| v.parse::<u32>().ok())
        .unwrap_or(25);

    match Supervisor::list_versions(&state.supervisor, &module_name, include_snapshots, page, per_page).await {
        Ok(result) => (StatusCode::OK, Json(result)).into_response(),
        Err(e) => (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(json!({"error": e.to_string()})),
        )
            .into_response(),
    }
}

/// GET /api/module/:name/version/:version — Get detailed info for a specific version
pub async fn get_version_details_handler(
    Path((module_name, version)): Path<(String, String)>,
    State(state): State<IPCServer>,
) -> impl IntoResponse {
    match Supervisor::get_version_details(&state.supervisor, &module_name, &version).await {
        Ok(result) => (StatusCode::OK, Json(result)).into_response(),
        Err(e) => (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(json!({"error": e.to_string()})),
        )
            .into_response(),
    }
}

/// GET /api/instance/:id/installed-version — Detect installed server version from binary
/// 감지 성공 시 instance.server_version에 자동 저장하여 이후 리스트에서 바로 표시
pub async fn get_installed_version_handler(
    Path(id): Path<String>,
    State(state): State<IPCServer>,
) -> impl IntoResponse {
    // Phase 1: 감지 (모듈 호출은 락 밖에서)
    let (_module_name, result) = {
        let found = state.supervisor.read().await.instance_store.list().iter().find(|i| i.id == id).cloned();
        let instance = match found {
            Some(i) => i,
            None => {
                return (
                    StatusCode::NOT_FOUND,
                    Json(json!({"error": format!("Instance '{}' not found", id)})),
                )
                    .into_response()
            }
        };

        let module_name = instance.module_name.clone();
        let res = Supervisor::get_installed_version(&state.supervisor, &module_name, &id).await;
        (module_name, res)
    };

    match result {
        Ok(result) => {
            // Phase 2: 감지 성공 시 write lock으로 인스턴스에 버전 저장
            if result.get("success").and_then(|s| s.as_bool()) == Some(true) {
                if let Some(version) = result.get("version").and_then(|v| v.as_str()) {
                    let mut supervisor = state.supervisor.write().await;
                    if let Some(mut instance) = supervisor.instance_store.get(&id).cloned() {
                        let already_set = instance.server_version.as_deref() == Some(version);
                        if !already_set {
                            instance.server_version = Some(version.to_string());
                            if let Err(e) = supervisor.instance_store.update(&id, instance) {
                                tracing::warn!("Failed to persist detected version for '{}': {}", id, e);
                            } else {
                                tracing::info!("Auto-saved detected server version '{}' for instance '{}'", version, id);
                            }
                        }
                    }
                }
            }
            (StatusCode::OK, Json(result)).into_response()
        }
        Err(e) => (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(json!({"error": e.to_string()})),
        )
            .into_response(),
    }
}

/// POST /api/module/:name/install — Install a server
/// Body: { "version": "1.21.11", "install_dir": "/path/to/server",
///         "jar_name": "server.jar", "accept_agreements": ["eula"], "initial_settings": {...} }
pub async fn install_server_handler(
    Path(module_name): Path<String>,
    State(state): State<IPCServer>,
    Json(payload): Json<serde_json::Value>,
) -> impl IntoResponse {
    // 있으면 문자열이어야 한다 (형식이 틀린 값을 "비움"으로 받아 다른 버전을 설치하지 않게)
    let text_field = |field: &str| match payload.get(field) {
        None => Ok(None),
        Some(Value::String(v)) => Ok(Some(v.clone())),
        Some(_) => {
            let error = json!({ "error": format!("'{}' must be a string", field), "error_code": "invalid_format", "fields": [{"field": field}] });
            Err(Box::new((StatusCode::BAD_REQUEST, Json(error)).into_response()))
        }
    };
    // 비우면 아래에서 그 인스턴스 설정으로 최신을 고른다
    let version = match text_field("version") {
        Ok(v) => v.filter(|v| !v.is_empty()),
        Err(resp) => return *resp,
    };
    let instance_id = match text_field("instance_id") {
        Ok(v) => v,
        Err(resp) => return *resp,
    };

    let install_dir = match payload.get("install_dir").and_then(|v| v.as_str()) {
        Some(v) => v.to_string(),
        None => {
            return (
                StatusCode::BAD_REQUEST,
                Json(json!({"error": "Missing 'install_dir' field"})),
            )
                .into_response()
        }
    };

    let jar_name = payload.get("jar_name").and_then(|v| v.as_str());
    let accepted_agreements = match requested_agreements(&payload, "accept_agreements") {
        Ok(ids) => ids,
        Err(resp) => return *resp,
    };
    let initial_settings = payload.get("initial_settings").cloned();
    // 어느 인스턴스의 설치인가: 그 설정(서버 종류 등)으로 버전을 고르고 설치한다. 그 인스턴스의 데이터 폴더를
    // 모듈에 넘기므로 설치가 끝날 때까지 잡아 둔다(켜져 있거나 백업·복원·삭제 중이면 409). 설정은 잡은 뒤에 읽는다
    let (instance, own_guard) = match instance_id.as_deref() {
        Some(id) => {
            match state.supervisor.read().await.instance_store.get(id) {
                Some(inst) if inst.module_name == module_name => {}
                Some(_) => {
                    let error = json!({ "error": format!("Instance '{}' is not a '{}' server", id, module_name), "error_code": "invalid_format", "fields": [{"field": "instance_id"}] });
                    return (StatusCode::BAD_REQUEST, Json(error)).into_response();
                }
                None => return (StatusCode::NOT_FOUND, Json(json!({ "error": format!("Instance not found: {}", id), "error_code": "not_found" }))).into_response(),
            }
            let Some(guard) = super::backup::BusyGuard::take(id) else {
                return super::backup::instance_busy();
            };
            if super::backup::is_running(&state, id).await {
                return server_running();
            }
            let sup = state.supervisor.read().await;
            // 잡기 전에 지워졌을 수 있다
            let Some(inst) = sup.instance_store.get(id) else {
                return (StatusCode::NOT_FOUND, Json(json!({ "error": format!("Instance not found: {}", id), "error_code": "not_found" }))).into_response();
            };
            let dir = sup.instance_store.instance_dir(id);
            (Some((json!(&inst.module_settings), Supervisor::instance_data_dirs(inst, &dir))), Some(guard))
        }
        None => (None, None),
    };

    // ── 필수 익스텐션 의존성 검증 ──
    {
        let found = state.supervisor.read().await.module_loader.get_module(&module_name);
        let module = match found {
            Ok(m) => m,
            Err(e) => {
                return (
                    StatusCode::NOT_FOUND,
                    Json(json!({"error": format!("Module '{}' not found: {}", module_name, e)})),
                )
                    .into_response()
            }
        };
        if let Some(unknown) = accepted_agreements.iter().find(|id| !module.metadata.agreements.iter().any(|a| &a.id == *id)) {
            let error = json!({ "error": format!("Module '{}' has no agreement '{}'", module_name, unknown), "error_code": "invalid_format", "fields": [{"field": "accept_agreements"}] });
            return (StatusCode::BAD_REQUEST, Json(error)).into_response();
        }
        if let Some(ref install_cfg) = module.metadata.install {
            if !install_cfg.requires_extensions.is_empty() {
                let ext_mgr = state.extension_manager.read().await;
                let ready = ext_mgr.installed_and_enabled_set();
                let enabled_not_installed = ext_mgr.enabled_but_not_installed();
                drop(ext_mgr);

                let missing: Vec<String> = install_cfg
                    .requires_extensions
                    .iter()
                    .filter(|ext_id| !ready.contains(ext_id.as_str()))
                    .cloned()
                    .collect();

                if !missing.is_empty() {
                    let not_installed: Vec<&String> = missing
                        .iter()
                        .filter(|id| enabled_not_installed.contains(id))
                        .collect();
                    let msg = if !not_installed.is_empty() {
                        format!(
                            "Cannot install server: required extension(s) not installed: {}. Install them in Settings → Extensions first.",
                            not_installed.iter().map(|s| s.as_str()).collect::<Vec<_>>().join(", ")
                        )
                    } else {
                        format!(
                            "Cannot install server: required extension(s) not enabled: {}. Enable them in Settings → Extensions first.",
                            missing.join(", ")
                        )
                    };
                    return (
                        StatusCode::UNPROCESSABLE_ENTITY,
                        Json(json!({
                            "error": msg,
                            "error_code": "extension_required",
                            "missing_extensions": missing,
                            "not_installed": not_installed,
                        })),
                    )
                        .into_response();
                }
            }
        }
    }

    let version = match version {
        Some(v) => v,
        None => {
            let module_path = match state.supervisor.read().await.module_loader.get_module(&module_name) {
                Ok(m) => m.entry_path(),
                Err(e) => return (StatusCode::NOT_FOUND, Json(json!({"error": format!("Module '{}' not found: {}", module_name, e)}))).into_response(),
            };
            let settings = instance.as_ref().map(|(s, _)| s.clone()).unwrap_or_else(|| json!({}));
            match Supervisor::latest_version(&module_path, &settings).await {
                Ok(v) => v,
                Err(e) => {
                    let error = json!({ "error": format!("Could not find the latest version of this server: {}", e), "error_code": "version_unknown" });
                    return (StatusCode::UNPROCESSABLE_ENTITY, Json(error)).into_response();
                }
            }
        }
    };

    // 이 폴더를 쓰는 서버가 켜져 있거나 백업·복원 중이면 파일을 덮어쓰지 않는다.
    // 설치를 마칠 때까지 잡고, 요청이 끊겨도 잠금이 설치와 함께 남게 따로 돌린다
    let mut guards = match hold_other_instances_using(&state, &install_dir, instance_id.as_deref()).await {
        Ok(guards) => guards,
        Err(resp) => return resp,
    };
    guards.extend(own_guard);
    let jar_name = jar_name.map(str::to_string);
    let install = {
        let state = state.clone();
        let version = version.clone();
        crate::audit::spawn(async move {
            let _guards = guards;
            Supervisor::install_server(
                &state.supervisor,
                &module_name,
                &version,
                &install_dir,
                jar_name.as_deref(),
                &accepted_agreements,
                initial_settings,
                instance,
            )
            .await
            .map_err(|e| e.to_string())
        })
    };
    match install.await.unwrap_or_else(|e| Err(e.to_string())) {
        Ok(mut result) => {
            // 버전을 비운 클라이언트도 무엇을 설치했는지 안다
            if let Some(obj) = result.as_object_mut() {
                obj.entry("version").or_insert_with(|| json!(version));
            }
            let status = if result.get("success").and_then(|s| s.as_bool()) == Some(true) {
                StatusCode::OK
            } else {
                StatusCode::UNPROCESSABLE_ENTITY
            };
            (status, Json(result)).into_response()
        }
        Err(e) => (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(json!({"error": e.to_string()})),
        )
            .into_response(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Arc;
    use tokio::sync::RwLock;

    // 실사용(L23): GUI "새로 설치"가 버전을 직접 골라 인스턴스 설정 없이 보내, 종류가 다른 서버(예: 다른 배포판)의
    // 파일을 받을 수 있었다. 버전을 비우면 데몬이 그 인스턴스 설정으로 최신을 고르고, 설치에도 그 설정을 넘긴다
    #[tokio::test]
    async fn an_install_without_a_version_takes_the_latest_for_the_instance_settings() {
        let tmp = tempfile::tempdir().unwrap();
        let modules = tmp.path().join("modules");
        let dir = modules.join("game");
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(dir.join("module.toml"), "[module]\nschema_version = 1\nname = \"game\"\nversion = \"1.0.0\"\nentry = \"lifecycle.py\"\n").unwrap();
        // 종류가 "b"일 때만 최신이 2.0이고, 설치는 그 버전과 그 종류로 불렸을 때만 성공한다
        std::fs::write(
            dir.join("lifecycle.py"),
            "import json, sys\nconfig = json.load(sys.stdin)\nkind = (config.get('settings') or {}).get('kind')\n\
             if sys.argv[1] == 'list_versions':\n    print(json.dumps({'success': True, 'latest': {'release': '2.0' if kind == 'b' else '1.0'}}))\n\
             else:\n    ok = config.get('version') == '2.0' and kind == 'b'\n    print(json.dumps({'success': ok, 'message': 'version %s kind %s' % (config.get('version'), kind)}))\n",
        )
        .unwrap();
        let mut sup = crate::supervisor::Supervisor::new_with_instances_dir(modules.to_str().unwrap(), tmp.path().join("instances").to_str().unwrap());
        sup.initialize().await.unwrap();
        let mut inst = crate::instance::ServerInstance::new("srv", "game");
        inst.module_settings.insert("kind".into(), json!("b"));
        let id = inst.id.clone();
        sup.instance_store.add(inst).unwrap();
        let state = IPCServer::new(Arc::new(RwLock::new(sup)), "127.0.0.1:0", crate::daemon_log::DaemonLogBuffer::new());
        let install = |body: Value| {
            let state = state.clone();
            async move { crate::ipc::internal::into_internal(install_server_handler(Path("game".into()), State(state), Json(body)).await.into_response()).await }
        };
        let files = tmp.path().join("files");

        let done = install(json!({ "install_dir": files, "instance_id": id })).await;
        assert!(done.status.is_success(), "{}", done.body);
        assert_eq!(done.body["version"], "2.0", "the client learns which version the daemon chose");
        // 없는 인스턴스, 다른 모듈의 인스턴스, 문자열이 아닌 값은 받지 않는다
        let missing = install(json!({ "install_dir": files, "instance_id": "nope" })).await;
        assert_eq!(missing.status, StatusCode::NOT_FOUND, "{}", missing.body);
        let foreign = crate::instance::ServerInstance::new("other", "elsewhere");
        let foreign_id = foreign.id.clone();
        state.supervisor.write().await.instance_store.add(foreign).unwrap();
        let other = install(json!({ "install_dir": files, "instance_id": foreign_id })).await;
        assert_eq!(other.status, StatusCode::BAD_REQUEST, "{}", other.body);
        for body in [json!({ "install_dir": files, "instance_id": 123 }), json!({ "install_dir": files, "version": 1.2 })] {
            let wrong = install(body).await;
            assert_eq!(wrong.status, StatusCode::BAD_REQUEST, "{}", wrong.body);
            assert_eq!(wrong.body["error_code"], "invalid_format");
        }
        // 그 인스턴스가 다른 작업(백업·복원·삭제 등)에 잡혀 있으면 그 데이터 폴더를 모듈에 넘기지 않는다
        let busy = super::super::backup::BusyGuard::take(&id).unwrap();
        let held = install(json!({ "install_dir": tmp.path().join("elsewhere"), "instance_id": id })).await;
        assert_eq!(held.status, StatusCode::CONFLICT, "{}", held.body);
        drop(busy);
        // 버전을 정해 보내면 그대로 쓴다 (예전 클라이언트)
        let pinned = install(json!({ "install_dir": files, "instance_id": id, "version": "1.0" })).await;
        assert_eq!(pinned.status, StatusCode::UNPROCESSABLE_ENTITY, "{}", pinned.body);
    }
}
