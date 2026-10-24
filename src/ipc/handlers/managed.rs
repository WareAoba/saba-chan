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
    let Some(_guard) = super::backup::InstanceLock::take_for_writes(&id).await else {
        return super::backup::instance_busy();
    };
    super::server::fill_passwords(&state, &id).await;

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

/// GET /api/instance/:id/players — 접속자 수와 이름 (디스코드 검수 13번). 모듈 `[status] players` 함수를
/// `players_detail: true`로 바로 부른다(1분 갱신은 이 키 없이 수만 읽는다). 항목은 `{name}`만 남긴다
pub async fn players_handler(Path(id): Path<String>, State(state): State<IPCServer>) -> impl IntoResponse {
    let fail = |status: StatusCode, code: &str, error: String| (status, Json(json!({ "success": false, "error_code": code, "error": error }))).into_response();
    let call = {
        let sup = state.supervisor.read().await;
        let Some(instance) = sup.instance_store.get(&id) else {
            return fail(StatusCode::NOT_FOUND, "not_found", format!("Instance not found: {}", id));
        };
        let Some(function) = sup.module_loader.get_module(&instance.module_name).ok().and_then(|m| m.metadata.players_function) else {
            return fail(StatusCode::OK, "not_supported", "This server's module cannot list players".into());
        };
        if sup.tracker.get_pid(&id).is_err() {
            return fail(StatusCode::CONFLICT, "server_not_running", "The server is not running".into());
        }
        sup.instance_call(&id, &function)
    };
    let mut call = match call {
        Ok(c) => c,
        Err(e) => return fail(StatusCode::INTERNAL_SERVER_ERROR, "players_failed", e.to_string()),
    };
    if let serde_json::Value::Object(cfg) = call.config_mut() {
        cfg.insert("players_detail".to_string(), json!(true));
    }
    match call.timeout(crate::supervisor::players::CALL_TIMEOUT_SECS).run().await {
        Ok(result) => match crate::supervisor::players::parse(&result) {
            Some(count) => {
                let names: Vec<serde_json::Value> = result
                    .get("players")
                    .and_then(serde_json::Value::as_array)
                    .into_iter()
                    .flatten()
                    .filter_map(|p| p.get("name").and_then(serde_json::Value::as_str))
                    // 게임이 이름을 자유롭게 두는 경우: 제어 문자를 빼고 64자까지 (요약 줄이 깨지지 않게)
                    .map(|name| name.chars().filter(|c| !c.is_control()).take(64).collect::<String>())
                    .filter(|name| !name.is_empty())
                    .map(|name| json!({ "name": name }))
                    .collect();
                let mut body = json!({ "success": true, "online": count.online, "players": names });
                if let Some(max) = count.max {
                    body["max"] = json!(max);
                }
                (StatusCode::OK, Json(body)).into_response()
            }
            None => {
                let message = result.get("message").and_then(serde_json::Value::as_str).unwrap_or("The module did not report players");
                fail(StatusCode::OK, "players_failed", message.to_string())
            }
        },
        Err(e) => {
            tracing::warn!("Player list for '{}' failed: {}", id, e);
            fail(StatusCode::OK, "players_failed", "The module did not answer".into())
        }
    }
}

/// GET /api/instance/:id/properties — 게임 설정 파일 읽기
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

/// PUT /api/instance/:id/properties — 게임 설정 파일 쓰기
pub async fn write_properties_handler(
    Path(id): Path<String>,
    State(state): State<IPCServer>,
    Json(payload): Json<serde_json::Value>,
) -> impl IntoResponse {
    let settings = payload.get("settings").cloned();
    let Some(_files) = super::backup::InstanceLock::files(&id).await else {
        return super::backup::instance_busy();
    };
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
    let Some(_files) = super::backup::InstanceLock::files(&id).await else {
        return super::backup::instance_busy();
    };
    match refresh_settings_from_files(&state, &id).await {
        Ok(mut result) => {
            // 캐시를 맞춘 뒤의 revision: 설정 창이 연 때의 값으로 저장하면 이 읽기 때문에 409가 된다
            if let (Some(inst), true) = (state.supervisor.read().await.instance_store.get(&id), result.is_object()) {
                result["revision"] = json!(inst.settings_revision());
            }
            (StatusCode::OK, Json(result)).into_response()
        }
        Err(e) => (StatusCode::INTERNAL_SERVER_ERROR, Json(json!({ "success": false, "error": e.to_string() }))).into_response(),
    }
}

/// 게임 설정 파일을 모듈 `import_settings`로 읽어 `module_settings`와 인스턴스 필드(포트·비밀번호)를 맞춘다.
/// 호출자가 설정 파일 잠금(`InstanceLock::files`나 `take_for_writes`)을 쥐고 있어야 한다. 모듈의 답을 그대로 돌려준다. `success`가 아니거나 설정이 비어 있으면 아무것도 바꾸지 않는다
pub(crate) async fn refresh_settings_from_files(state: &IPCServer, id: &str) -> anyhow::Result<serde_json::Value> {
    let result = Supervisor::manage_properties(&state.supervisor, id, "import", None).await?;
    let ok = result.get("success").and_then(|v| v.as_bool()) == Some(true);
    let settings = result.get("settings").and_then(|v| v.as_object()).filter(|s| !s.is_empty());
    if let (true, Some(settings)) = (ok, settings) {
        let mut supervisor = state.supervisor.write().await;
        if let Some(mut inst) = supervisor.instance_store.get(id).cloned() {
            let (credential_map, types) = supervisor.module_loader.get_module(&inst.module_name)
                .map(|m| (m.metadata.credential_map.clone(), m.metadata.setting_types()))
                .unwrap_or_default();
            // 포트·비밀번호 같은 인스턴스 필드도 파일 값으로 맞춘다 (GUI는 그쪽을 먼저 읽는다)
            inst.apply_imported_settings(settings, &credential_map, &types);
            if let Err(e) = supervisor.instance_store.update(id, inst) {
                tracing::warn!("Settings refresh from game files failed for {}: {}", id, e);
            }
        }
    }
    Ok(result)
}

/// POST /api/instance/:id/properties/reset — 게임 설정 파일을 기본값으로
pub async fn reset_properties_handler(
    Path(id): Path<String>,
    State(state): State<IPCServer>,
) -> impl IntoResponse {
    let Some(_files) = super::backup::InstanceLock::files(&id).await else {
        return super::backup::instance_busy();
    };
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

/// `dir`과 겹치는 폴더(working_dir 또는 인스턴스 폴더, 그 안이나 바깥)를 쓰는 인스턴스의 작업 잠금을 모두 잡는다.
/// `except`(이미 잠금을 쥔 인스턴스)는 건너뛴다. 다른 작업 중이면 409 `instance_busy`, 켜져 있으면 409 `server_running`
#[allow(clippy::result_large_err)] // 오류는 바로 돌려줄 HTTP 응답
pub(super) async fn hold_other_instances_using(state: &IPCServer, dir: &str, except: Option<&str>) -> Result<Vec<super::backup::InstanceLock>, axum::response::Response> {
    let ids: Vec<String> = {
        let supervisor = state.supervisor.read().await;
        let dir = std::path::Path::new(dir);
        supervisor.instance_store.list().iter()
            .filter(|i| {
                i.working_dir.as_deref().is_some_and(|d| saba_chan_updater_lib::paths_overlap(std::path::Path::new(d), dir))
                    || saba_chan_updater_lib::paths_overlap(&supervisor.instance_store.instance_dir(&i.id), dir)
            })
            .filter(|i| except != Some(i.id.as_str()))
            .map(|i| i.id.clone())
            .collect()
    };
    let mut guards = Vec::new();
    for id in ids {
        let Some(guard) = super::backup::InstanceLock::take_for_writes(&id).await else {
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
    let Some(guard) = super::backup::InstanceLock::take_for_writes(&id).await else {
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
    // 모듈이 eula.txt 같은 게임 파일을 쓴다: 설정 저장과 한 줄로, 서버 파일을 바꾸는 작업 중에는 거절
    let Some(_files) = super::backup::InstanceLock::files(&id).await else {
        return super::backup::instance_busy();
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
    let (_module_name, seen, result) = {
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
        (module_name, instance.server_version, res)
    };

    match result {
        Ok(result) => {
            // Phase 2: 감지 성공 시 write lock으로 인스턴스에 버전 저장.
            // 설치·업데이트가 인스턴스를 잡고 있으면 파일이 바뀌는 중이라 저장하지 않는다 (D-S12)
            if result.get("success").and_then(|s| s.as_bool()) == Some(true) {
                if let Some(version) = result.get("version").and_then(|v| v.as_str()) {
                    if !super::backup::InstanceLock::held(&id) {
                        match state.supervisor.write().await.save_detected_version(&id, seen.as_deref(), version) {
                            Ok(true) => tracing::info!("Auto-saved detected server version '{}' for instance '{}'", version, id),
                            Ok(false) => {}
                            Err(e) => tracing::warn!("Failed to persist detected version for '{}': {}", id, e),
                        }
                    }
                }
            }
            let mut result = result;
            // 저장한 버전까지 반영한 revision (settings/live와 같다)
            if let (Some(inst), true) = (state.supervisor.read().await.instance_store.get(&id), result.is_object()) {
                result["revision"] = json!(inst.settings_revision());
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
///         "server_executable": "server.jar", "accept_agreements": ["eula"], "initial_settings": {...} }
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

    // 서버 실행 파일 이름 (없으면 모듈 `server_executable`)
    let executable = payload.get("server_executable").and_then(|v| v.as_str());
    let accepted_agreements = match requested_agreements(&payload, "accept_agreements") {
        Ok(ids) => ids,
        Err(resp) => return *resp,
    };
    let initial_settings = payload.get("initial_settings").cloned();
    // 어느 인스턴스의 설치인가: 그 설정(서버 종류 등)으로 버전을 고르고 설치한다. 그 인스턴스의 데이터 폴더를
    // 모듈에 넘기므로 설치가 끝날 때까지 잡아 둔다(켜져 있거나 백업·복원·삭제 중이면 409). 설정은 잡은 뒤에 읽는다
    let (instance, own_guard, instance_name) = match instance_id.as_deref() {
        Some(id) => {
            match state.supervisor.read().await.instance_store.get(id) {
                Some(inst) if inst.module_name == module_name => {}
                Some(_) => {
                    let error = json!({ "error": format!("Instance '{}' is not a '{}' server", id, module_name), "error_code": "invalid_format", "fields": [{"field": "instance_id"}] });
                    return (StatusCode::BAD_REQUEST, Json(error)).into_response();
                }
                None => return (StatusCode::NOT_FOUND, Json(json!({ "error": format!("Instance not found: {}", id), "error_code": "not_found" }))).into_response(),
            }
            let Some(guard) = super::backup::InstanceLock::take_for_writes(id).await else {
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
            (Some((json!(&inst.module_settings), Supervisor::instance_data_dirs(inst, &dir), sup.resolve_executable(inst, &module_name))), Some(guard), Some(inst.name.clone()))
        }
        None => (None, None, None),
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
            let settings = instance.as_ref().map(|(s, _, _)| s.clone()).unwrap_or_else(|| json!({}));
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
    let others = match hold_other_instances_using(&state, &install_dir, instance_id.as_deref()).await {
        Ok(guards) => guards,
        Err(resp) => return resp,
    };
    // 인스턴스의 버전 설치는 뒤에서 돌릴 수 있다: 설치 전 백업이 길어도 요청 한도에 걸리지 않게 (진행은 provision-progress)
    let background = instance_id.is_some() && payload.get("background").and_then(Value::as_bool) == Some(true);
    let executable = executable.map(str::to_string);
    let keep_executable = instance.as_ref().and_then(|(_, _, exe)| exe.clone());
    let text = crate::chat::i18n::Catalog::for_language(&state.config_store.ui_language().await);
    if let (true, Some(name)) = (background, &instance_name) {
        state.provision_tracker.take_creating(name);
        state.provision_tracker.update(name, version_progress(text.t("provision.version_installing", &[("version", &version)]), false, None));
    }
    let install = {
        let state = state.clone();
        let version = version.clone();
        let instance_id = instance_id.clone();
        crate::audit::spawn(async move {
            let _others = others;
            let own = instance_id.as_deref().zip(own_guard.as_ref());
            // 설치 전 백업 (결정 3): 백업할 것이 없으면 그냥 진행하고, 실패하면 설치하지 않는다
            let before = match own {
                Some((id, guard)) => match super::backup::take_backup_held(&state, id, crate::backup::BackupKind::PreUpdate, guard).await {
                    Ok(made) => made.map(|b| b.name),
                    Err(r) => {
                        let reason = crate::ipc::internal::into_internal(r).await.body["error"].as_str().unwrap_or("backup failed").to_string();
                        return Err(json!({ "success": false, "error": format!("Pre-install backup failed: {}", reason), "error_code": "backup_failed", "reason": reason }));
                    }
                },
                None => None,
            };
            let result = Supervisor::install_server(
                &state.supervisor,
                &module_name,
                &version,
                &install_dir,
                executable.as_deref(),
                &accepted_agreements,
                initial_settings,
                instance.map(|(settings, dirs, _)| (settings, dirs)),
                keep_executable.as_deref(),
            )
            .await
            .map_err(|e| {
                // 되돌리지 못한 파일은 422와 같은 칸으로도 싣는다
                let mut body = json!({ "error": format!("{:#}", e) });
                if let Some(put_back) = e.downcast_ref::<crate::supervisor::PutBackFailed>() {
                    body["restore_failed"] = json!(put_back.files);
                    body["kept_backups"] = json!(put_back.kept);
                }
                body
            });
            let Some((id, guard)) = own else {
                return result;
            };
            let succeeded = result.as_ref().is_ok_and(|r| r.get("success").and_then(Value::as_bool) == Some(true));
            let mut result = result;
            if let (true, Ok(installed)) = (succeeded, &result) {
                // 받은 파일로 켜도록 버전·실행 파일·작업 폴더를 함께 맞춘다 (클라이언트가 따로 저장하지 않는다)
                let saved = save_installed(&mut *state.supervisor.write().await, id, &version, installed, &install_dir);
                let Err(e) = saved else {
                    return result;
                };
                // 기록과 파일이 어긋나지 않게 아래에서 설치 전 백업으로 되돌린다
                tracing::error!("Installed '{}' but could not save it: {:#}", id, e);
                result = Err(json!({ "success": false, "error": format!("The server files were installed, but the server details could not be saved: {}", e), "error_code": "save_failed" }));
            }
            // 실패: 설치 전 백업으로 되돌린다
            if let Some(name) = before {
                let restored = super::backup::restore_held(&state, id, &name, guard, false).await;
                let body = match &mut result {
                    Ok(body) | Err(body) => body,
                };
                if body.is_object() {
                    match restored {
                        Ok(_) => body["restored_backup"] = json!(name),
                        Err(r) => {
                            let reason = crate::ipc::internal::into_internal(r).await.body["error"].as_str().unwrap_or("restore failed").to_string();
                            tracing::error!("Could not put back the pre-install backup '{}' for '{}': {}", name, id, reason);
                            body["kept_backup"] = json!(name);
                        }
                    }
                }
            }
            result
        })
    };
    let version_label = version.clone();
    let finish = move |outcome: Result<Result<Value, Value>, tokio::task::JoinError>| match outcome.unwrap_or_else(|e| Err(json!({ "error": e.to_string() }))) {
        Ok(mut result) => {
            // 버전을 비운 클라이언트도 무엇을 설치했는지 안다
            if let Some(obj) = result.as_object_mut() {
                obj.entry("version").or_insert_with(|| json!(version));
            }
            Ok(result)
        }
        Err(body) => Err(body),
    };
    if let (true, Some(name)) = (background, instance_name) {
        let tracker = state.provision_tracker.clone();
        tokio::spawn(async move {
            // 주의가 붙었거나 실패했으면 사용자가 닫을 때까지 남긴다
            let mut keep = true;
            let progress = match finish(install.await) {
                Ok(r) if r.get("success").and_then(Value::as_bool) == Some(true) => {
                    // 모듈이 붙인 주의 한 줄 (`java_warning`은 예전 이름)
                    let warning = ["warning", "java_warning"].iter().find_map(|k| r.get(*k).and_then(Value::as_str)).filter(|w| !w.is_empty());
                    keep = warning.is_some();
                    let message = match warning {
                        Some(w) => format!("{}\n⚠️ {}", text.t("provision.version_installed", &[]), w),
                        None => text.t("provision.version_installed", &[]),
                    };
                    version_progress(message, true, None)
                }
                // 모듈이 정한 실패 문구는 그대로, 스크립트 오류(stderr·경로)는 로그에만
                Ok(r) => version_progress(
                    text.t("provision.install_failed", &[]),
                    true,
                    Some(r.get("message").and_then(Value::as_str).map(str::to_string).unwrap_or_else(|| text.t("provision.install_failed", &[]))),
                ),
                Err(body) => {
                    let error = match body["error_code"].as_str() {
                        Some("backup_failed") => text.t("provision.install_backup_failed", &[("reason", body["reason"].as_str().unwrap_or_default())]),
                        Some("save_failed") => text.t("provision.save_failed", &[]),
                        _ => text.t("provision.script_failed", &[]),
                    };
                    version_progress(text.t("provision.install_failed", &[]), true, Some(error))
                }
            };
            tracker.update(&name, progress);
            if !keep {
                tokio::time::sleep(std::time::Duration::from_secs(5)).await;
                tracker.remove(&name);
            }
        });
        let body = json!({ "success": true, "provisioning": true, "id": instance_id, "version": version_label });
        return (StatusCode::OK, Json(body)).into_response();
    }
    match finish(install.await) {
        Ok(result) => {
            let status = if result.get("success").and_then(|s| s.as_bool()) == Some(true) {
                StatusCode::OK
            } else {
                StatusCode::UNPROCESSABLE_ENTITY
            };
            (status, Json(result)).into_response()
        }
        Err(body) => {
            let status = if body["error_code"] == "backup_failed" { StatusCode::UNPROCESSABLE_ENTITY } else { StatusCode::INTERNAL_SERVER_ERROR };
            (status, Json(body)).into_response()
        }
    }
}

/// 버전 설치의 진행 표시 한 줄 (인스턴스 업데이트와 같은 모양)
fn version_progress(message: String, done: bool, error: Option<String>) -> crate::ipc::ProvisionProgress {
    crate::ipc::ProvisionProgress {
        step: 0,
        total: 1,
        label: if done { "done".to_string() } else { "install".to_string() },
        message,
        done,
        error,
        percent: done.then_some(100),
        steps: None,
    }
}

/// 설치가 끝난 인스턴스에 버전과 받은 실행 파일·작업 폴더를 저장한다. 모듈이 경로를 알려 주지 않으면 버전만
fn save_installed(sup: &mut Supervisor, id: &str, version: &str, result: &Value, install_dir: &str) -> anyhow::Result<()> {
    let Some(mut inst) = sup.instance_store.get(id).cloned() else {
        anyhow::bail!("instance {} is gone", id);
    };
    inst.server_version = Some(version.to_string());
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

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Arc;
    use tokio::sync::RwLock;

    /// 플레이어 목록 (디스코드 검수 13번): 모듈 `[status] players` 함수를 `players_detail`로 불러 이름까지 받는다.
    /// 이름 말고 다른 값(IP 등)은 싣지 않는다. 함수가 없으면 not_supported, 꺼진 서버는 409
    #[tokio::test]
    async fn players_come_from_the_status_function_with_names_only() {
        let tmp = tempfile::tempdir().unwrap();
        let modules = tmp.path().join("modules");
        for (name, status) in [("game", "\n[status]\nplayers = \"player_count\"\n"), ("plain", "")] {
            let dir = modules.join(name);
            std::fs::create_dir_all(&dir).unwrap();
            std::fs::write(dir.join("module.toml"), format!("[module]\nschema_version = 1\nname = \"{name}\"\nversion = \"1.0.0\"\nentry = \"lifecycle.py\"\n{status}")).unwrap();
            std::fs::write(
                dir.join("lifecycle.py"),
                "import json, sys\nconfig = json.load(sys.stdin)\n\
                 names = [{'name': 'Steve', 'ip': '10.0.0.5'}, {'name': 'Al\\nex', 'uuid': 'x'}, {'name': 'x' * 70}] if config.get('players_detail') else []\n\
                 print(json.dumps({'success': True, 'online': 2, 'max': 20, 'players': names}))\n",
            )
            .unwrap();
        }
        let mut sup = crate::supervisor::Supervisor::new_with_instances_dir(modules.to_str().unwrap(), tmp.path().join("instances").to_str().unwrap());
        sup.initialize().await.unwrap();
        for (id, module) in [("with", "game"), ("without", "plain")] {
            let mut inst = crate::instance::ServerInstance::new(id, module);
            inst.id = id.into();
            sup.instance_store.add(inst).unwrap();
        }
        let state = IPCServer::new(Arc::new(RwLock::new(sup)), "127.0.0.1:0", crate::daemon_log::DaemonLogBuffer::new());
        let players = |id: &'static str| {
            let state = state.clone();
            async move { crate::ipc::internal::into_internal(players_handler(Path(id.into()), State(state)).await.into_response()).await }
        };

        let off = players("with").await;
        assert_eq!((off.status, off.body["error_code"].as_str()), (StatusCode::CONFLICT, Some("server_not_running")), "{}", off.body);
        state.supervisor.read().await.tracker.track("with", std::process::id()).unwrap();
        let on = players("with").await;
        assert_eq!(on.body, json!({ "success": true, "online": 2, "max": 20, "players": [{ "name": "Steve" }, { "name": "Alex" }, { "name": "x".repeat(64) }] }));
        let _ = state.supervisor.read().await.tracker.untrack("with");

        let unsupported = players("without").await;
        assert_eq!((unsupported.status, unsupported.body["error_code"].as_str()), (StatusCode::OK, Some("not_supported")), "{}", unsupported.body);
        assert_eq!(players("missing").await.status, StatusCode::NOT_FOUND);
    }

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
        let busy = super::super::backup::InstanceLock::take(&id).unwrap();
        let held = install(json!({ "install_dir": tmp.path().join("elsewhere"), "instance_id": id })).await;
        assert_eq!(held.status, StatusCode::CONFLICT, "{}", held.body);
        drop(busy);
        // 버전을 정해 보내면 그대로 쓴다 (예전 클라이언트)
        let pinned = install(json!({ "install_dir": files, "instance_id": id, "version": "1.0" })).await;
        assert_eq!(pinned.status, StatusCode::UNPROCESSABLE_ENTITY, "{}", pinned.body);
    }

    // Codex 리뷰 #539 P1: 확인되지 않은 모듈이 돌려준 `install_path`(없으면 실행 파일의 폴더)를 그대로 작업 폴더로 저장해,
    // 다음 격리 실행에서 인스턴스 밖 폴더가 쓰기를 받았다. 인스턴스 폴더 안이 아니면 설치 폴더를 둔다
    #[cfg(windows)]
    #[tokio::test]
    async fn an_unverified_module_cannot_point_the_server_folder_outside_the_instance() {
        let tmp = tempfile::tempdir().unwrap();
        let modules = tmp.path().join("modules");
        let dir = modules.join("game");
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(dir.join("module.toml"), "[module]\nschema_version = 1\nname = \"game\"\nversion = \"1.0.0\"\nentry = \"lifecycle.py\"\n").unwrap();
        std::fs::write(dir.join("lifecycle.py"), "").unwrap();
        let mut sup = crate::supervisor::Supervisor::new_with_instances_dir(modules.to_str().unwrap(), tmp.path().join("instances").to_str().unwrap());
        sup.initialize().await.unwrap();
        let inst = crate::instance::ServerInstance::new("srv", "game");
        let id = inst.id.clone();
        sup.instance_store.add(inst).unwrap();
        let root = sup.instance_store.instance_dir(&id);
        let inside = root.join("server");
        let outside = tmp.path().join("elsewhere");
        for d in [&inside, &outside] {
            std::fs::create_dir_all(d).unwrap();
        }
        let install_dir = inside.to_string_lossy().into_owned();
        let saved_dir = |sup: &Supervisor| sup.instance_store.get(&id).unwrap().working_dir.clone().unwrap();

        let exe = outside.join("server.exe").to_string_lossy().into_owned();
        save_installed(&mut sup, &id, "1.0", &json!({ "executable_path": exe }), &install_dir).unwrap();
        assert_eq!(saved_dir(&sup), install_dir, "the executable's folder outside the instance is not taken");
        save_installed(&mut sup, &id, "1.0", &json!({ "executable_path": exe, "install_path": outside }), &install_dir).unwrap();
        assert_eq!(saved_dir(&sup), install_dir, "install_path outside the instance is not taken");
        let nested = inside.join("bin");
        std::fs::create_dir_all(&nested).unwrap();
        save_installed(&mut sup, &id, "1.0", &json!({ "executable_path": exe, "install_path": nested }), &install_dir).unwrap();
        assert_eq!(saved_dir(&sup), nested.to_string_lossy(), "a folder inside the instance is kept");
    }

    // 외부 감사 P1: 업데이트 전 백업이 모듈 `server_executable`(server.jar)만 담아, 모듈이 인스턴스의 실제 실행 파일
    // (예: Fabric 런처)을 바꾸다 실패하면 그 파일은 되돌리지 못했다. 인스턴스 실행 파일도 함께 백업하고 되돌린다
    #[tokio::test]
    async fn a_failed_install_puts_back_the_instance_executable_too() {
        let tmp = tempfile::tempdir().unwrap();
        // 데몬의 모듈 폴더 아래에 둔다: 그 밖의 모듈은 Windows 격리 실행에서 설치 폴더 쓰기를 받지 못한다
        let modules = crate::plugin::resolve_modules_dir();
        let module = tempfile::Builder::new().prefix("put-back-exe-").tempdir_in(&modules).unwrap();
        let dir = module.path();
        let game = dir.file_name().unwrap().to_string_lossy().into_owned();
        std::fs::write(dir.join("module.toml"), format!("[module]\nschema_version = 1\nname = \"{game}\"\nversion = \"1.0.0\"\nentry = \"lifecycle.py\"\n\n[config]\nserver_executable = \"server.jar\"\n")).unwrap();
        // 두 파일을 반쯤 바꾼 뒤 실패한다 (한 번은 success=false, 한 번은 오류로 끝남)
        std::fs::write(
            dir.join("lifecycle.py"),
            "import json, os, sys\nconfig = json.load(sys.stdin)\nd = config['install_dir']\n\
             for name in ('server.jar', 'launch.jar'):\n    open(os.path.join(d, name), 'w').write('half')\n\
             if config.get('version') == 'crash':\n    sys.exit(3)\n\
             print(json.dumps({'success': False, 'message': 'download failed'}))\n",
        )
        .unwrap();
        let mut sup = crate::supervisor::Supervisor::new_with_instances_dir(modules.to_str().unwrap(), tmp.path().join("instances").to_str().unwrap());
        sup.initialize().await.unwrap();
        let files = tmp.path().join("files");
        std::fs::create_dir_all(&files).unwrap();
        let mut inst = crate::instance::ServerInstance::new("srv", &game);
        inst.executable_path = Some(files.join("launch.jar").to_string_lossy().into_owned());
        let id = inst.id.clone();
        sup.instance_store.add(inst).unwrap();
        let state = IPCServer::new(Arc::new(RwLock::new(sup)), "127.0.0.1:0", crate::daemon_log::DaemonLogBuffer::new());
        for version in ["1.1", "crash"] {
            for name in ["server.jar", "launch.jar"] {
                std::fs::write(files.join(name), "old").unwrap();
            }
            let body = json!({ "install_dir": files, "instance_id": id, "version": version });
            let done = crate::ipc::internal::into_internal(install_server_handler(Path(game.clone()), State(state.clone()), Json(body)).await.into_response()).await;
            assert!(!done.status.is_success() || done.body["success"] == false, "{}", done.body);
            for name in ["server.jar", "launch.jar"] {
                assert_eq!(std::fs::read_to_string(files.join(name)).unwrap(), "old", "{version}: {name}");
            }
            let left: Vec<_> = std::fs::read_dir(&files).unwrap().map(|e| e.unwrap().file_name()).filter(|n| n.to_string_lossy().ends_with(".bak")).collect();
            assert!(left.is_empty(), "{version}: {left:?}");
        }
    }

    // 되돌리기가 실패하면 백업을 남기고 응답에 알린다. 사용자의 기존 .bak는 덮지 않는다. 설치 폴더 밖 실행 파일은 건드리지 않는다.
    // 심볼릭 링크는 링크를 둔 채 대상 내용을 되돌린다
    #[tokio::test]
    async fn an_install_that_cannot_be_put_back_keeps_the_backup_and_says_so() {
        let tmp = tempfile::tempdir().unwrap();
        // 데몬의 모듈 폴더 아래에 둔다: 그 밖의 모듈은 Windows 격리 실행에서 설치 폴더 쓰기를 받지 못한다
        let modules = crate::plugin::resolve_modules_dir();
        let module = tempfile::Builder::new().prefix("put-back-").tempdir_in(&modules).unwrap();
        let dir = module.path();
        let game = dir.file_name().unwrap().to_string_lossy().into_owned();
        std::fs::write(dir.join("module.toml"), format!("[module]\nschema_version = 1\nname = \"{game}\"\nversion = \"1.0.0\"\nentry = \"lifecycle.py\"\n\n[config]\nserver_executable = \"PalServer\"\n")).unwrap();
        // write: 실행 파일을 고친 뒤 실패, replace: 새 파일로 바꿔 넣은 뒤 실패, break: 실행 파일 자리를 폴더로 바꿔 두고 실패(어떤 방법으로도 되돌릴 수 없다), 그 밖: 아무것도 안 하고 실패
        std::fs::write(
            dir.join("lifecycle.py"),
            "import json, os, sys\nconfig = json.load(sys.stdin)\nexe = os.path.join(config['install_dir'], 'PalServer')\n\
             if config['version'] == 'write':\n    open(exe, 'w').write('new')\n\
             elif config['version'] == 'replace':\n    open(exe + '.part', 'w').write('new')\n    os.replace(exe + '.part', exe)\n\
             elif config['version'] in ('break', 'crash'):\n    os.remove(exe)\n    os.makedirs(os.path.join(exe, 'x'))\n\
             if config['version'] == 'crash':\n    raise RuntimeError('boom')\n\
             print(json.dumps({'success': False, 'message': 'download failed'}))\n",
        )
        .unwrap();
        let mut sup = crate::supervisor::Supervisor::new_with_instances_dir(modules.to_str().unwrap(), tmp.path().join("instances").to_str().unwrap());
        sup.initialize().await.unwrap();
        let files = tmp.path().join("files");
        let elsewhere = tmp.path().join("elsewhere");
        std::fs::create_dir_all(&files).unwrap();
        std::fs::create_dir_all(&elsewhere).unwrap();
        std::fs::write(elsewhere.join("tool.exe"), "tool").unwrap();
        std::fs::write(elsewhere.join("tool.exe.bak"), "user backup").unwrap();
        let mut instance = |name: &str, exe: std::path::PathBuf| {
            let mut inst = crate::instance::ServerInstance::new(name, &game);
            inst.executable_path = Some(exe.to_string_lossy().into_owned());
            let id = inst.id.clone();
            sup.instance_store.add(inst).unwrap();
            id
        };
        let inside = instance("srv", files.join("PalServer"));
        let outside = instance("other", elsewhere.join("tool.exe"));
        let state = IPCServer::new(Arc::new(RwLock::new(sup)), "127.0.0.1:0", crate::daemon_log::DaemonLogBuffer::new());
        let install = |id: &str, version: &str| {
            let body = json!({ "install_dir": files, "instance_id": id, "version": version });
            let (state, game) = (state.clone(), game.clone());
            async move { crate::ipc::internal::into_internal(install_server_handler(Path(game), State(state), Json(body)).await.into_response()).await }
        };
        let backups = || -> Vec<String> {
            let mut names: Vec<String> = std::fs::read_dir(&files).unwrap().map(|e| e.unwrap().file_name().to_string_lossy().into_owned()).filter(|n| n.ends_with(".bak")).collect();
            names.sort();
            names
        };

        // 사용자가 둔 PalServer.bak는 백업 자리로 쓰지 않는다: 설치가 실패해도 그대로 남고 실행 파일은 되돌아온다
        std::fs::write(files.join("PalServer"), "old").unwrap();
        std::fs::write(files.join("PalServer.bak"), "user backup").unwrap();
        let done = install(&inside, "write").await;
        assert_eq!(done.body["success"], false, "{}", done.body);
        assert_eq!(std::fs::read_to_string(files.join("PalServer")).unwrap(), "old");
        assert_eq!(std::fs::read_to_string(files.join("PalServer.bak")).unwrap(), "user backup");
        assert_eq!(backups(), ["PalServer.bak"]);

        // 설치 폴더 밖의 실행 파일과 그 옆의 사용자 파일은 그대로다
        let done = install(&outside, "keep").await;
        assert_eq!(done.body["success"], false, "{}", done.body);
        assert_eq!(std::fs::read_to_string(elsewhere.join("tool.exe.bak")).unwrap(), "user backup");
        assert_eq!(std::fs::read_to_string(elsewhere.join("tool.exe")).unwrap(), "tool");

        // 되돌리지 못하면 백업을 남기고 그 파일을 알린다
        let done = install(&inside, "break").await;
        assert_eq!(done.body["success"], false, "{}", done.body);
        assert_eq!(done.body["restore_failed"], json!(["PalServer"]), "{}", done.body);
        // 어느 .bak이 이번 시도 직전 사본인지도 알린다 (607 P3-2)
        assert_eq!(done.body["kept_backups"], json!(["PalServer.1.bak"]), "{}", done.body);
        assert_eq!(backups(), ["PalServer.1.bak", "PalServer.bak"]);
        assert_eq!(std::fs::read_to_string(files.join("PalServer.1.bak")).unwrap(), "old");
        assert_eq!(std::fs::read_to_string(files.join("PalServer.bak")).unwrap(), "user backup");

        // 심볼릭 링크로 둔 실행 파일은 링크를 둔 채 대상 내용이 돌아온다
        #[cfg(unix)]
        {
            std::fs::remove_dir_all(files.join("PalServer")).unwrap();
            std::fs::write(files.join("real"), "old").unwrap();
            std::os::unix::fs::symlink(files.join("real"), files.join("PalServer")).unwrap();
            let done = install(&inside, "write").await;
            assert_eq!(done.body["success"], false, "{}", done.body);
            assert!(std::fs::symlink_metadata(files.join("PalServer")).unwrap().file_type().is_symlink());
            assert_eq!(std::fs::read_to_string(files.join("real")).unwrap(), "old");

            // 모듈이 링크를 새 파일로 바꿔 넣었으면 그 자리에 옛 내용을 되돌린다
            let done = install(&inside, "replace").await;
            assert_eq!(done.body["restore_failed"], Value::Null, "{}", done.body);
            assert_eq!(std::fs::read_to_string(files.join("PalServer")).unwrap(), "old");

            // 링크 자리를 폴더로 바꿨으면 되돌리지 못했다고 알린다
            std::fs::remove_file(files.join("PalServer")).unwrap();
            std::os::unix::fs::symlink(files.join("real"), files.join("PalServer")).unwrap();
            let done = install(&inside, "break").await;
            assert_eq!(done.body["restore_failed"], json!(["PalServer"]), "{}", done.body);

            // 설치 폴더 밖을 가리키는 링크는 백업하지 않는다: 링크도 밖의 파일도 그대로다
            std::fs::remove_dir_all(files.join("PalServer")).unwrap();
            std::fs::write(elsewhere.join("outside"), "old").unwrap();
            std::os::unix::fs::symlink(elsewhere.join("outside"), files.join("PalServer")).unwrap();
            let before = std::fs::metadata(elsewhere.join("outside")).unwrap().modified().unwrap();
            std::thread::sleep(std::time::Duration::from_millis(20));
            let done = install(&inside, "keep").await;
            assert_eq!(done.body["restore_failed"], Value::Null, "{}", done.body);
            assert_eq!(std::fs::metadata(elsewhere.join("outside")).unwrap().modified().unwrap(), before);
            assert_eq!(std::fs::read_to_string(elsewhere.join("outside")).unwrap(), "old");
            assert!(std::fs::symlink_metadata(files.join("PalServer")).unwrap().file_type().is_symlink());
            assert_eq!(backups(), ["PalServer.1.bak", "PalServer.2.bak", "PalServer.bak"], "only the failed restores keep a backup");
        }

        // 모듈이 오류로 끝나고 되돌리지도 못하면 500 `error`에 모듈 오류와 남긴 백업 이름이 함께 있다 (600 P3-2)
        let reset = || {
            let exe = files.join("PalServer");
            let _ = std::fs::remove_dir_all(&exe);
            let _ = std::fs::remove_file(&exe);
            std::fs::write(&exe, "old").unwrap();
        };
        reset();
        let before = backups();
        let done = install(&inside, "crash").await;
        let error = done.body["error"].as_str().unwrap_or_default();
        let kept: Vec<String> = backups().into_iter().filter(|b| !before.contains(b)).collect();
        assert_eq!(kept.len(), 1, "{}", done.body);
        assert!(error.contains("boom") && error.contains(&kept[0]), "{}", done.body);
        // 422와 같은 칸으로도 알린다: 클라이언트가 원문을 풀지 않고 현지화 문구를 고른다 (리뷰 631 P3-1)
        assert_eq!((&done.body["restore_failed"], &done.body["kept_backups"]), (&json!(["PalServer"]), &json!(kept)), "{}", done.body);

        // 백업할 이름이 모두 차 있으면 설치하지 않고 어느 파일을 백업하지 못했는지 알린다 (607 P3-3)
        reset();
        for n in 1..100 {
            std::fs::write(files.join(format!("PalServer.{n}.bak")), "x").unwrap();
        }
        let done = install(&inside, "write").await;
        assert_eq!((done.body["error_code"].as_str(), done.body["file"].as_str()), (Some("backup_failed"), Some("PalServer")), "{}", done.body);
        assert_eq!(std::fs::read_to_string(files.join("PalServer")).unwrap(), "old", "the module never ran");
    }

    // 0.1.10 결정 3: 버전 설치는 뒤에서 돌고(GUI 요청 한도에 걸리지 않게) 설치 전에 백업한다. 실패하면 그 백업으로
    // 되돌리고, 성공하면 데몬이 버전·실행 파일·작업 폴더를 함께 저장한다(클라이언트가 따로 맞추지 않는다)
    #[tokio::test]
    async fn a_background_version_install_backs_up_first_and_puts_it_back_on_failure() {
        let tmp = tempfile::tempdir().unwrap();
        // 데몬의 모듈 폴더 아래에 둔다: 그 밖의 모듈은 Windows 격리 실행에서 설치 폴더 쓰기를 받지 못한다
        let modules = crate::plugin::resolve_modules_dir();
        let module = tempfile::Builder::new().prefix("version-install-").tempdir_in(&modules).unwrap();
        let dir = module.path();
        let game = dir.file_name().unwrap().to_string_lossy().into_owned();
        std::fs::write(dir.join("module.toml"), format!("[module]\nschema_version = 1\nname = \"{game}\"\nversion = \"1.0.0\"\nentry = \"lifecycle.py\"\n\n[backup]\npaths = [\"world\"]\n")).unwrap();
        // 월드를 망가뜨린 뒤, bad면 실패하고 아니면 새 폴더에 받았다고 답한다
        std::fs::write(
            dir.join("lifecycle.py"),
            "import json, os, sys\nconfig = json.load(sys.stdin)\nd = config['install_dir']\n\
             open(os.path.join(d, 'world', 'level.dat'), 'w').write('broken')\n\
             if config.get('version') == 'bad':\n    print(json.dumps({'success': False, 'message': 'download failed'}))\n\
             else:\n    new = os.path.join(d, 'v2')\n    os.makedirs(new, exist_ok=True)\n    open(os.path.join(d, 'world', 'level.dat'), 'w').write('migrated')\n    print(json.dumps({'success': True, 'install_path': new, 'executable_path': os.path.join(new, 'server.bin'), 'warning': 'Java 21 needed'}))\n",
        )
        .unwrap();
        let mut sup = crate::supervisor::Supervisor::new_with_instances_dir(modules.to_str().unwrap(), tmp.path().join("instances").to_str().unwrap());
        sup.initialize().await.unwrap();
        let files = tmp.path().join("files");
        std::fs::create_dir_all(files.join("world")).unwrap();
        std::fs::write(files.join("world").join("level.dat"), "old").unwrap();
        let mut inst = crate::instance::ServerInstance::new("srv", &game);
        inst.working_dir = Some(files.to_string_lossy().into_owned());
        let id = inst.id.clone();
        sup.instance_store.add(inst).unwrap();
        let state = IPCServer::new(Arc::new(RwLock::new(sup)), "127.0.0.1:0", crate::daemon_log::DaemonLogBuffer::new());
        let install = |version: &str| {
            let body = json!({ "install_dir": files, "instance_id": id, "version": version, "background": true });
            let (state, game) = (state.clone(), game.clone());
            async move { crate::ipc::internal::into_internal(install_server_handler(Path(game), State(state), Json(body)).await.into_response()).await }
        };
        let finished = || async {
            for _ in 0..2400 {
                if let Some(p) = state.provision_tracker.get("srv").filter(|p| p.done) {
                    return p;
                }
                tokio::time::sleep(std::time::Duration::from_millis(50)).await;
            }
            panic!("the install never finished");
        };

        let started = install("bad").await;
        assert_eq!((started.status, &started.body["provisioning"]), (StatusCode::OK, &json!(true)), "{}", started.body);
        let failed = finished().await;
        assert_eq!(failed.error.as_deref(), Some("download failed"), "{failed:?}");
        assert_eq!(std::fs::read_to_string(files.join("world").join("level.dat")).unwrap(), "old", "the world comes back from the pre-install backup");
        assert_eq!(state.supervisor.read().await.instance_store.get(&id).unwrap().server_version, None);
        state.provision_tracker.remove("srv");

        install("2.0").await;
        let done = finished().await;
        assert_eq!(done.error, None, "{done:?}");
        // 모듈이 붙인 주의(필요한 Java 등)는 끝난 표시에 남는다
        assert!(done.message.contains("Java 21 needed"), "{done:?}");
        let inst = state.supervisor.read().await.instance_store.get(&id).cloned().unwrap();
        assert_eq!(inst.server_version.as_deref(), Some("2.0"));
        assert_eq!(inst.working_dir, Some(files.join("v2").to_string_lossy().into_owned()));
        assert_eq!(inst.executable_path, Some(files.join("v2").join("server.bin").to_string_lossy().into_owned()));
        assert_eq!(std::fs::read_to_string(files.join("world").join("level.dat")).unwrap(), "migrated");
    }

    // D-S12: 설치 버전 감지는 모듈 호출 동안 잠금을 쥐지 않아, 그사이 저장된 새 버전(설치·업데이트 뒤 설정)을
    // 감지가 끝난 뒤 옛 값으로 덮을 수 있었다. 감지 사이에 값이 바뀌었거나 인스턴스가 작업 중이면 저장하지 않는다
    #[tokio::test]
    async fn a_version_detected_while_the_server_changes_does_not_overwrite_it() {
        let tmp = tempfile::tempdir().unwrap();
        let modules = tmp.path().join("modules");
        let dir = modules.join("game");
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(dir.join("module.toml"), "[module]\nschema_version = 1\nname = \"game\"\nversion = \"1.0.0\"\nentry = \"lifecycle.py\"\n").unwrap();
        std::fs::write(
            dir.join("lifecycle.py"),
            "import json, sys, time\njson.load(sys.stdin)\ntime.sleep(0.6)\nprint(json.dumps({'success': True, 'version': '1.0'}))\n",
        )
        .unwrap();
        let mut sup = crate::supervisor::Supervisor::new_with_instances_dir(modules.to_str().unwrap(), tmp.path().join("instances").to_str().unwrap());
        sup.initialize().await.unwrap();
        let inst = crate::instance::ServerInstance::new("srv", "game");
        let id = inst.id.clone();
        sup.instance_store.add(inst).unwrap();
        let state = IPCServer::new(Arc::new(RwLock::new(sup)), "127.0.0.1:0", crate::daemon_log::DaemonLogBuffer::new());
        let detect = || {
            let (state, id) = (state.clone(), id.clone());
            async move { crate::ipc::internal::into_internal(get_installed_version_handler(Path(id), State(state)).await.into_response()).await }
        };
        let saved = || {
            let (state, id) = (state.clone(), id.clone());
            async move { state.supervisor.read().await.instance_store.get(&id).unwrap().server_version.clone() }
        };
        let set = |version: &'static str| {
            let (state, id) = (state.clone(), id.clone());
            async move {
                let mut sup = state.supervisor.write().await;
                let mut inst = sup.instance_store.get(&id).cloned().unwrap();
                inst.server_version = Some(version.into());
                sup.instance_store.update(&id, inst).unwrap();
            }
        };

        // 감지하는 사이 새 버전이 저장되면 그 값을 남긴다
        let running = tokio::spawn(detect());
        tokio::time::sleep(std::time::Duration::from_millis(200)).await;
        set("2.0").await;
        let done = running.await.unwrap();
        assert_eq!(done.body["version"], "1.0", "{}", done.body);
        assert_eq!(saved().await.as_deref(), Some("2.0"), "a stale detection must not overwrite a newer version");

        // 설치·업데이트 같은 작업이 인스턴스를 잡고 있으면 감지 결과는 알려 주되 저장하지 않는다
        let busy = super::super::backup::InstanceLock::take(&id).unwrap();
        let held = detect().await;
        assert_eq!(held.body["version"], "1.0", "{}", held.body);
        assert_eq!(saved().await.as_deref(), Some("2.0"), "files may be mid-change while the instance is busy");
        drop(busy);

        // 아무 일도 없으면 감지한 값을 저장한다
        detect().await;
        assert_eq!(saved().await.as_deref(), Some("1.0"));
    }
}
