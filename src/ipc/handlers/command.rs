use axum::{
    extract::{Path, State},
    http::StatusCode,
    response::IntoResponse,
    Json,
};
use serde_json::{json, Value};

use super::super::{CommandRequest, IPCServer};

/// POST /api/instance/:id/command - 게임 명령 실행 (모든 인터페이스의 단일 경로)
///
/// 요청 해석·검증·라우팅은 `crate::commands`가 한다. 응답은 실제로 실행한 경로
/// (RCON·REST·stdin·모듈)의 응답 본문에 `command`를 더한 것이고, 실패하면
/// `error_code`와 모듈 `[errors]`의 사용자용 문구 `error_message`를 더한다.
pub async fn execute_command(
    Path(id): Path<String>,
    State(state): State<IPCServer>,
    Json(req): Json<CommandRequest>,
) -> axum::response::Response {
    let (command, args) = (req.command.clone(), req.args.clone());
    let resp = run_command(&id, &state, req).await;
    super::audited(&state, "command", &id, Some(&command), Some(&args), resp).await
}

async fn run_command(id: &str, state: &IPCServer, req: CommandRequest) -> axum::response::Response {
    let id = id.to_string();
    let state = state.clone();
    use crate::commands::{Args, Plan, Request};
    use crate::ipc::internal::{into_internal, InternalResponse};

    let request = Request::parse(&req.command, &req.args);
    if request.name.is_empty() {
        return command_reply(
            InternalResponse { status: StatusCode::BAD_REQUEST, body: json!({ "error": "Missing 'command' field" }) },
            "",
            &Default::default(),
        );
    }

    let (instance, module, running, stdin_route) = {
        let supervisor = state.supervisor.read().await;
        let Some(instance) = supervisor.instance_store.get(&id).cloned() else {
            return command_reply(
                InternalResponse {
                    status: StatusCode::NOT_FOUND,
                    body: json!({ "error": format!("Instance not found: {}", id), "error_code": "not_found" }),
                },
                &request.name,
                &Default::default(),
            );
        };
        let module = supervisor.module_loader.get_module(&instance.module_name).ok();
        let running = supervisor.tracker.get_pid(&instance.id).is_ok();
        let stdin_route = supervisor.routes_commands_to_stdin(&instance.id).await;
        (instance, module, running, stdin_route)
    };
    let errors = match &module {
        Some(module) => module_errors(module, &state.config_store.ui_language().await),
        None => Default::default(),
    };
    let metadata = module.map(|m| m.metadata);
    // module.toml에 없는 이름이면 인스턴스별 명령(설치된 플러그인 명령 등)에서도 찾는다
    let metadata = match metadata {
        Some(meta)
            if meta.commands.as_ref().is_some_and(|c| c.hook) && crate::commands::find(&meta, &request.name).is_none() =>
        {
            let extra = instance_commands(&state, &instance, false).await;
            Some(crate::commands::with_instance_commands(&meta, &extra))
        }
        other => other,
    };
    // 응답의 command는 별명이 아니라 정의된 이름 (정의에 없으면 입력 그대로)
    let name = metadata
        .as_ref()
        .and_then(|m| crate::commands::find(m, &request.name))
        .map(|f| f.name.clone())
        .unwrap_or_else(|| request.name.clone());

    // 모듈에 없는 이름이면 익스텐션 명령인지 본다 (서버가 꺼져 있어도 부를 수 있다)
    let module_has_it = metadata.as_ref().is_some_and(|m| crate::commands::find(m, &request.name).is_some());
    if !module_has_it {
        let context = json!({
            "instance_id": &instance.id,
            "instance_name": &instance.name,
            "module": &instance.module_name,
            "extension_data": &instance.extension_data,
            "running": running,
        });
        let planned = state.extension_manager.read().await.plan_command(&request.name, &request.args, context);
        if let Some(Err(e)) = &planned {
            return command_reply(
                InternalResponse {
                    status: StatusCode::BAD_REQUEST,
                    body: json!({ "error": e.message, "error_code": e.code, "field": e.field }),
                },
                &request.name,
                &errors,
            );
        }
        if let Some(Ok(plan)) = planned {
            let resp = match plan.run(crate::plugin::DEFAULT_PLUGIN_TIMEOUT_SECS).await.into_iter().next() {
                Some((ext_id, Ok(value))) => {
                    let failed = value.get("success").and_then(Value::as_bool) == Some(false);
                    let mut body = match value {
                        Value::Object(map) => map,
                        other => serde_json::Map::from_iter([("data".to_string(), other)]),
                    };
                    body.insert("protocol".into(), json!("extension"));
                    body.insert("extension".into(), json!(ext_id));
                    InternalResponse {
                        status: if failed { StatusCode::UNPROCESSABLE_ENTITY } else { StatusCode::OK },
                        body: Value::Object(body),
                    }
                }
                Some((ext_id, Err(e))) => InternalResponse {
                    status: StatusCode::INTERNAL_SERVER_ERROR,
                    body: json!({ "error": e.to_string(), "extension": ext_id }),
                },
                None => InternalResponse { status: StatusCode::INTERNAL_SERVER_ERROR, body: json!({ "error": "no handler" }) },
            };
            return command_reply(resp, &request.name, &errors);
        }
    }

    let plan = match &metadata {
        Some(meta) => crate::commands::plan(meta, &request),
        None => Ok(Plan::Raw { line: request.line.clone() }),
    };
    let plan = match plan {
        Ok(plan) => plan,
        Err(e) => {
            return command_reply(
                InternalResponse {
                    status: StatusCode::BAD_REQUEST,
                    body: json!({ "error": e.message, "error_code": e.code, "field": e.field }),
                },
                &name,
                &errors,
            )
        }
    };

    if !running && !stdin_route {
        return command_reply(
            InternalResponse {
                status: StatusCode::CONFLICT,
                body: json!({
                    "error": format!("Server '{}' is not running", instance.name),
                    "error_code": "server_not_running",
                }),
            },
            &name,
            &errors,
        );
    }

    let resp = match plan {
        Plan::Console { line } => {
            into_internal(execute_rcon_command(Path(id.clone()), State(state.clone()), Json(json!({ "command": line }))).await).await
        }
        Plan::Rest { endpoint, http_method, body } => {
            into_internal(
                execute_rest_command(
                    Path(id.clone()),
                    State(state.clone()),
                    Json(json!({ "endpoint": endpoint, "method": http_method, "body": body })),
                )
                .await,
            )
            .await
        }
        Plan::Module { command, args } => run_module_command(&state, &instance, &command, Value::Object(args)).await,
        Plan::Raw { line } if stdin_route => {
            into_internal(
                super::managed::send_stdin_handler(Path(id.clone()), State(state.clone()), Json(json!({ "command": line })))
                    .await,
            )
            .await
        }
        Plan::Raw { line } => match &request.args {
            Args::Named(args) if !args.is_empty() => {
                run_module_command(&state, &instance, &request.name, Value::Object(args.clone())).await
            }
            _ => run_module_command(&state, &instance, &line, json!({})).await,
        },
    };
    command_reply(resp, &name, &errors)
}

/// 인스턴스별 명령 기억 (인스턴스 id → 읽은 때, 명령). 판정에 Python을 띄우므로 명령 실행 때는
/// 잠깐 기억한 것을 쓰고, 목록 API는 매번 새로 읽는다 (플러그인을 넣고 빼면 바로 보이게).
static INSTANCE_COMMANDS: std::sync::OnceLock<
    std::sync::Mutex<std::collections::HashMap<String, (std::time::Instant, crate::commands::InstanceCommands)>>,
> = std::sync::OnceLock::new();
const INSTANCE_COMMANDS_TTL: std::time::Duration = std::time::Duration::from_secs(60);

/// 모듈 lifecycle `instance_commands()` (`[commands] hook = true`인 모듈만). 실패하면 빈 목록.
async fn instance_commands(
    state: &IPCServer,
    instance: &crate::instance::ServerInstance,
    fresh: bool,
) -> crate::commands::InstanceCommands {
    let cache = INSTANCE_COMMANDS.get_or_init(Default::default);
    if !fresh {
        if let Some((at, cached)) = cache.lock().ok().and_then(|c| c.get(&instance.id).cloned()) {
            if at.elapsed() < INSTANCE_COMMANDS_TTL {
                return cached;
            }
        }
    }
    let entry = {
        let supervisor = state.supervisor.read().await;
        match supervisor.module_loader.get_module(&instance.module_name) {
            Ok(m) if m.metadata.commands.as_ref().is_some_and(|c| c.hook) => m.entry_path(),
            _ => return Default::default(),
        }
    };
    let mut config = serde_json::Map::new();
    if let Some(dir) = &instance.working_dir {
        config.insert("working_dir".into(), json!(dir));
    }
    crate::supervisor::Supervisor::insert_instance_fields(&mut config, instance);
    let loaded = match crate::plugin::run_plugin(&entry, "instance_commands", Value::Object(config)).await {
        Ok(value) if value.get("success").and_then(Value::as_bool) != Some(false) => {
            crate::commands::InstanceCommands::parse(&value)
        }
        Ok(value) => {
            tracing::warn!("instance_commands failed for '{}': {}", instance.name, value["message"]);
            Default::default()
        }
        Err(e) => {
            tracing::warn!("instance_commands failed for '{}': {}", instance.name, e);
            Default::default()
        }
    };
    if let Ok(mut c) = cache.lock() {
        c.insert(instance.id.clone(), (std::time::Instant::now(), loaded.clone()));
    }
    loaded
}

/// GET /api/instance/:id/commands — 이 인스턴스에서 쓸 수 있는 명령 전체 (모듈 + 인스턴스별 + 활성 익스텐션)
///
/// `{commands: [{name, label, description, risk, inputs, source: "module"|"extension", group?, extension?, aliases?}],
///   components?}` — 인스턴스별 명령(`[commands] hook`)은 source "module"에 `group`(예: 플러그인 이름)이 붙고,
/// `components`는 모듈이 판정한 구성요소(설치된 플러그인·모드 등)를 그대로 넘긴 것이다.
pub async fn list_instance_commands(Path(id): Path<String>, State(state): State<IPCServer>) -> axum::response::Response {
    let (instance, metadata) = {
        let supervisor = state.supervisor.read().await;
        let Some(instance) = supervisor.instance_store.get(&id).cloned() else {
            return (
                StatusCode::NOT_FOUND,
                Json(json!({ "error": format!("Instance not found: {}", id), "error_code": "not_found" })),
            )
                .into_response();
        };
        let metadata = supervisor.module_loader.get_module(&instance.module_name).ok().map(|m| m.metadata);
        (instance, metadata)
    };
    let mut commands = Vec::new();
    if let Some(meta) = &metadata {
        for field in meta.commands.as_ref().map(|c| c.fields.as_slice()).unwrap_or(&[]) {
            let aliases = meta.command_aliases.get(&field.name).map(|a| a.aliases.clone()).unwrap_or_default();
            commands.push(json!({
                "name": field.name,
                "label": field.label,
                "description": field.description,
                "risk": field.risk.as_deref().unwrap_or("caution"),
                "inputs": field.inputs,
                "source": "module",
                "aliases": aliases,
            }));
        }
    }
    let extra = match &metadata {
        Some(meta) if meta.commands.as_ref().is_some_and(|c| c.hook) => Some(instance_commands(&state, &instance, true).await),
        _ => None,
    };
    if let (Some(meta), Some(extra)) = (&metadata, &extra) {
        for field in &extra.fields {
            if crate::commands::find(meta, &field.name).is_some() {
                continue; // module.toml이 먼저다
            }
            commands.push(json!({
                "name": field.name,
                "label": field.label,
                "description": field.description,
                "risk": field.risk.as_deref().unwrap_or("caution"),
                "inputs": field.inputs,
                "source": "module",
                "group": extra.groups.get(&field.name),
            }));
        }
    }
    let mgr = state.extension_manager.read().await;
    for (ext_id, cmd) in mgr.commands_for(&instance.extension_data) {
        if commands.iter().any(|c| c["name"].as_str().is_some_and(|n| n.eq_ignore_ascii_case(&cmd.name))) {
            continue; // 모듈 명령이 먼저다
        }
        commands.push(json!({
            "name": cmd.name,
            "label": cmd.label.as_deref().unwrap_or(&cmd.name),
            "description": cmd.description,
            "risk": cmd.risk.as_deref().unwrap_or("caution"),
            "inputs": cmd.inputs,
            "source": "extension",
            "extension": ext_id,
        }));
    }
    let mut body = json!({ "commands": commands });
    if let Some(extra) = extra.filter(|e| !e.components.is_null()) {
        body["components"] = extra.components;
    }
    Json(body).into_response()
}

/// 모듈 lifecycle.py `command()` 실행
async fn run_module_command(
    state: &IPCServer,
    instance: &crate::instance::ServerInstance,
    command: &str,
    args: Value,
) -> crate::ipc::internal::InternalResponse {
    use crate::ipc::internal::InternalResponse;
    match crate::supervisor::Supervisor::execute_command(&state.supervisor, &instance.id, &instance.module_name, command, args).await {
        Ok(message) => InternalResponse {
            status: StatusCode::OK,
            body: json!({ "success": true, "message": message, "protocol": "module" }),
        },
        Err(e) => InternalResponse { status: StatusCode::INTERNAL_SERVER_ERROR, body: json!({ "error": e.to_string() }) },
    }
}

/// 모듈 `[errors]` 문구: 코드마다 모듈 locales `errors.<code>`(GUI 언어)가 있으면 그 문구, 없으면 module.toml 값
fn module_errors(module: &crate::supervisor::module_loader::LoadedModule, lang: &str) -> std::collections::HashMap<String, String> {
    let locale = crate::actions::module_locale(module, lang);
    module
        .metadata
        .errors
        .iter()
        .map(|(code, declared)| {
            let text = locale.pointer(&format!("/errors/{}", code)).and_then(Value::as_str).unwrap_or(declared);
            (code.clone(), text.to_string())
        })
        .collect()
}

/// 실행 결과에 명령 이름과(실패 시) 오류 코드·모듈 문구를 붙인다
fn command_reply(
    resp: crate::ipc::internal::InternalResponse,
    command: &str,
    errors: &std::collections::HashMap<String, String>,
) -> axum::response::Response {
    let mut body = match resp.body {
        Value::Object(map) => map,
        Value::Null => serde_json::Map::new(),
        other => {
            let mut map = serde_json::Map::new();
            map.insert("data".into(), other);
            map
        }
    };
    body.insert("command".into(), json!(command));
    if !resp.status.is_success() {
        let code = crate::commands::error_code(resp.status.as_u16(), &Value::Object(body.clone()));
        if let Some(message) = errors.get(&code) {
            body.insert("error_message".into(), json!(message));
        }
        body.insert("error_code".into(), json!(code));
        body.entry("success").or_insert(json!(false));
    }
    (resp.status, Json(Value::Object(body))).into_response()
}

/// POST /api/instance/:id/rcon 라우트: 콘솔 명령을 감사에 남긴다 (안에서 부르는 `execute_rcon_command`는 부른 쪽이 남긴다)
pub async fn rcon_route(Path(id): Path<String>, State(state): State<IPCServer>, Json(payload): Json<Value>) -> axum::response::Response {
    let command = payload.get("command").and_then(Value::as_str).map(str::to_string);
    let resp = execute_rcon_command(Path(id.clone()), State(state.clone()), Json(payload)).await.into_response();
    super::audited(&state, "command", &id, command.as_deref(), None, resp).await
}

/// POST /api/instance/:id/rest 라우트: REST 명령을 감사에 남긴다
pub async fn rest_route(Path(id): Path<String>, State(state): State<IPCServer>, Json(payload): Json<Value>) -> axum::response::Response {
    let endpoint = payload.get("endpoint").and_then(Value::as_str).map(str::to_string);
    let body = payload.get("body").cloned();
    let resp = execute_rest_command(Path(id.clone()), State(state.clone()), Json(payload)).await.into_response();
    super::audited(&state, "command", &id, endpoint.as_deref(), body.as_ref(), resp).await
}

/// POST /api/instance/:id/rcon - RCON 명령어 실행
pub async fn execute_rcon_command(
    Path(id): Path<String>,
    State(state): State<IPCServer>,
    Json(payload): Json<serde_json::Value>,
) -> impl IntoResponse {
    let supervisor = state.supervisor.read().await;

    // 인스턴스 확인 (lock 해제 전에 clone)
    let instance = match supervisor.instance_store.get(&id).cloned() {
        Some(instance) => instance,
        None => {
            return (
                StatusCode::NOT_FOUND,
                Json(json!({
                    "error": format!("Instance not found: {}", id)
                })),
            )
                .into_response();
        }
    };

    // 명령은 언제나 stdin으로 보낸다 (콘솔 출력을 잠깐 모아 답으로 돌려준다). RCON은 보안상 쓰지 않는다.
    // GUI·디스코드·모듈 역호출이 모두 이 엔드포인트를 쓴다. 경로 이름(`/rcon`)은 예전 호출자와의 호환용이다.
    let stdin_ok = supervisor.routes_commands_to_stdin(&id).await;
    drop(supervisor);
    if stdin_ok {
        return via_stdin(&state, &id, &payload).await;
    }
    (
        StatusCode::CONFLICT,
        Json(json!({
            "error": format!("'{}' has no saba-chan console attached, so it can't take commands: restart it in managed mode from saba-chan", instance.name),
            "error_code": "no_command_path",
        })),
    )
        .into_response()
}

/// 관리 프로세스의 stdin으로 보낸다. 서버의 답은 콘솔로만 나오므로, 보낸 뒤 잠깐 새로 찍힌 줄을
/// 모아 `data.response`로 돌려준다 (채팅·CLI가 결과를 보여 줄 수 있게)
pub(crate) async fn via_stdin(state: &IPCServer, id: &str, payload: &serde_json::Value) -> axum::response::Response {
    use crate::supervisor::managed_process::LogSource;
    let Some(command) = payload.get("command").and_then(|v| v.as_str()) else {
        return (StatusCode::BAD_REQUEST, Json(json!({ "error": "Missing 'command' field" }))).into_response();
    };
    let proc = state.supervisor.read().await.managed_store.get(id).await;
    // 보내기 직전의 커서: 그 뒤에 나온 줄만 답으로 모은다
    let before = match &proc {
        Some(p) => p.console_cursor().await,
        None => 0,
    };
    let sent = state.supervisor.read().await.send_stdin_command(id, command).await;
    let message = match sent {
        Ok(m) => m,
        Err(e) if e.downcast_ref::<crate::supervisor::managed_process::StdinFull>().is_some() => {
            return (
                StatusCode::SERVICE_UNAVAILABLE,
                Json(json!({ "error": format!("stdin command failed: {}", e), "error_code": "console_input_full" })),
            )
                .into_response()
        }
        Err(e) => {
            return (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(json!({ "error": format!("stdin command failed: {}", e) })),
            )
                .into_response()
        }
    };
    let mut lines: Vec<String> = Vec::new();
    if let Some(p) = proc {
        // 최대 1.5초. 답이 오기 시작한 뒤 0.4초 동안 더 없으면 끝
        let mut quiet = 0;
        for _ in 0..15 {
            tokio::time::sleep(std::time::Duration::from_millis(100)).await;
            let new: Vec<String> = p
                .get_console_since(before)
                .await
                .into_iter()
                .filter(|l| l.source != LogSource::System && l.content.trim() != command.trim())
                .map(|l| l.content)
                .collect();
            if new.len() > lines.len() {
                lines = new;
                quiet = 0;
            } else if !lines.is_empty() {
                quiet += 1;
                if quiet >= 4 {
                    break;
                }
            }
        }
    }
    let data = if lines.is_empty() { Value::Null } else { json!({ "response": lines.join("\n"), "executed": command }) };
    (
        StatusCode::OK,
        Json(json!({ "success": true, "message": message, "data": data, "protocol": "stdin" })),
    )
        .into_response()
}

/// POST /api/instance/:id/rest - REST API 명령어 실행
pub async fn execute_rest_command(
    Path(id): Path<String>,
    State(state): State<IPCServer>,
    Json(payload): Json<serde_json::Value>,
) -> impl IntoResponse {
    let supervisor = state.supervisor.read().await;

    // 인스턴스 확인 (lock 해제 전에 clone)
    let instance = match supervisor.instance_store.get(&id).cloned() {
        Some(instance) => instance,
        None => {
            return (
                StatusCode::NOT_FOUND,
                Json(json!({
                    "error": format!("Instance not found: {}", id)
                })),
            )
                .into_response();
        }
    };

    // 모듈에서 기본값 가져오기
    let (default_rest_port, default_rest_host, module_path) =
        match supervisor.module_loader.get_module(&instance.module_name) {
            Ok(module) => (
                module.metadata.default_rest_port(),
                module.metadata.default_rest_host(),
                Some(module.entry_path()),
            ),
            Err(_) => (8212, "127.0.0.1".to_string(), None),
        };

    // 게임 설정 파일에서 실제 credential을 읽어옴 (메모리 동기화 문제 방지)
    let instance_dir = supervisor.instance_store.instance_dir(&id);
    let working_dir = instance.working_dir.clone()
        .unwrap_or_else(|| instance_dir.join("server").to_string_lossy().to_string());

    // supervisor lock 해제 — run_plugin 전에 release
    drop(supervisor);

    let file_credentials = instance_credentials(&state, &id, module_path.as_deref(), &working_dir).await;

    // REST 엔드포인트 추출
    let endpoint = match payload.get("endpoint").and_then(|v| v.as_str()) {
        Some(ep) => ep,
        None => {
            return (
                StatusCode::BAD_REQUEST,
                Json(json!({
                    "error": "Missing 'endpoint' field"
                })),
            )
                .into_response();
        }
    };

    // REST 정보 확인 - payload에서 먼저 찾고, 없으면 instance에서 찾음, 그래도 없으면 모듈 기본값
    let rest_host = payload
        .get("rest_host")
        .and_then(|v| v.as_str())
        .map(|s| s.to_string())
        .or_else(|| instance.rest_host.clone())
        .unwrap_or(default_rest_host);

    let rest_port = payload
        .get("rest_port")
        .and_then(|v| v.as_u64())
        .map(|p| p as u16)
        .or(instance.rest_port)
        .unwrap_or(default_rest_port);

    let use_https = payload
        .get("use_https")
        .and_then(|v| v.as_bool())
        .unwrap_or(false);

    tracing::info!(
        "REST command for instance {}: host={}:{} endpoint={}",
        id,
        rest_host,
        rest_port,
        endpoint
    );

    // REST 클라이언트 생성
    let mut client = crate::protocol::client::ProtocolClient::new_rest(
        rest_host.to_string(),
        rest_port,
        use_https,
    );

    // Basic Auth — 게임 설정 파일에서만 읽음
    if let Some(ref creds) = file_credentials {
        let username = creds.get("rest_username").and_then(|v| v.as_str()).unwrap_or("admin");
        if let Some(pass) = creds.get("rest_password").and_then(|v| v.as_str()) {
            client = client.with_basic_auth(username.to_string(), pass.to_string());
        }
    }

    // 메서드 결정
    let method = payload
        .get("method")
        .and_then(|v| v.as_str())
        .unwrap_or("GET");

    let http_method = match method.to_uppercase().as_str() {
        "POST" => crate::protocol::HttpMethod::Post,
        "PUT" => crate::protocol::HttpMethod::Put,
        "DELETE" => crate::protocol::HttpMethod::Delete,
        _ => crate::protocol::HttpMethod::Get,
    };

    // 명령어 구성 - endpoint는 모듈에서 완전한 형식으로 전달됨
    let cmd = crate::protocol::ServerCommand {
        command_type: crate::protocol::CommandType::Rest,
        command: None,
        endpoint: Some(endpoint.to_string()),
        method: Some(http_method),
        body: payload.get("body").cloned(),
        timeout_secs: payload.get("timeout").and_then(|v| v.as_u64()),
    };

    // 연결 확인과 요청은 블로킹 I/O라 async 런타임 밖에서 실행한다
    enum RestFailure {
        Connect(String),
        Execute(crate::protocol::ProtocolError),
    }
    let outcome = tokio::task::spawn_blocking(move || {
        client
            .connect_rest(std::time::Duration::from_secs(5))
            .map_err(|e| RestFailure::Connect(e.to_string()))?;
        client.execute(cmd).map_err(RestFailure::Execute)
    })
    .await
    .unwrap_or_else(|e| Err(RestFailure::Execute(crate::protocol::ProtocolError::Protocol(format!("task failed: {}", e)))));

    // 인증 실패 등으로 접속 정보가 바뀌었을 수 있다 — 다음 명령은 설정 파일을 다시 읽는다
    if !matches!(&outcome, Ok(response) if response.success) {
        state.credentials.invalidate(&id);
    }

    match outcome {
        Err(RestFailure::Connect(e)) => (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(json!({
                "error": format!("REST connection failed: {}", e)
            })),
        )
            .into_response(),
        Ok(response) => (
            StatusCode::OK,
            Json(json!({
                "success": response.success,
                "data": response.data,
                "error": response.error,
                "endpoint": endpoint,
                "method": method,
                "host": rest_host,
                "port": rest_port,
                "protocol": "rest"
            })),
        )
            .into_response(),
        Err(RestFailure::Execute(e)) => (StatusCode::INTERNAL_SERVER_ERROR, Json(rest_execute_failure(&e))).into_response(),
    }
}

/// REST 요청 실패의 응답 본문. 게임 서버가 401·403으로 거절하면(관리자 비밀번호가 다름) `auth_failed`로 나눠
/// `/command`가 모듈의 `errors.auth_failed` 문구를 붙이게 한다 (데몬 토큰 실패와 섞이지 않게 HTTP 상태는 500 그대로)
fn rest_execute_failure(e: &crate::protocol::ProtocolError) -> Value {
    let mut body = json!({ "error": format!("REST execution failed: {}", e) });
    if matches!(e, crate::protocol::ProtocolError::HttpStatus(401 | 403, _)) {
        body["error_code"] = json!("auth_failed");
    }
    body
}

/// 게임 설정 파일의 접속 정보 (lifecycle.py `get_credentials`). 캐시가 있으면 Python을 띄우지 않는다.
async fn instance_credentials(
    state: &IPCServer,
    instance_id: &str,
    module_path: Option<&str>,
    working_dir: &str,
) -> Option<Value> {
    if let Some(creds) = state.credentials.get(instance_id) {
        return Some(creds);
    }
    let Some(module_path) = module_path else {
        tracing::warn!("get_credentials: no module_path for {}, skipping", instance_id);
        return None;
    };
    let config = json!({ "working_dir": working_dir });
    match crate::plugin::run_plugin(module_path, "get_credentials", config).await {
        Ok(creds) if creds.get("success").and_then(|v| v.as_bool()).unwrap_or(false) => {
            state.credentials.put(instance_id, creds.clone());
            Some(creds)
        }
        Ok(creds) => {
            tracing::warn!("get_credentials returned non-success for {}: {:?}", instance_id, creds.get("message"));
            None
        }
        Err(e) => {
            tracing::warn!("get_credentials call failed for {}: {}", instance_id, e);
            None
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// 팰월드 관리자 비밀번호가 틀려 REST가 401로 거절되면 일반 "내부 오류" 대신 모듈의 인증 실패 문구가 붙는다
    #[tokio::test]
    async fn a_rest_command_the_game_rejects_gets_the_modules_auth_text() {
        use crate::protocol::ProtocolError;
        let errors: std::collections::HashMap<String, String> = [
            ("auth_failed".to_string(), "비밀번호를 확인하세요".to_string()),
            ("internal_server_error".to_string(), "내부 오류".to_string()),
        ]
        .into();
        let reply = |e: ProtocolError| {
            command_reply(
                crate::ipc::internal::InternalResponse { status: StatusCode::INTERNAL_SERVER_ERROR, body: rest_execute_failure(&e) },
                "info",
                &errors,
            )
        };
        let body = |r: axum::response::Response| async move {
            serde_json::from_slice::<Value>(&axum::body::to_bytes(r.into_body(), usize::MAX).await.unwrap()).unwrap()
        };
        let rejected = body(reply(ProtocolError::HttpStatus(401, "HTTP GET x failed with status 401: ".into()))).await;
        assert_eq!(rejected["error_code"], "auth_failed");
        assert_eq!(rejected["error_message"], "비밀번호를 확인하세요");
        let other = body(reply(ProtocolError::HttpStatus(500, "boom".into()))).await;
        assert_eq!(other["error_code"], "internal_server_error");
    }

    #[test]
    fn module_error_text_follows_the_gui_language() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::create_dir_all(dir.path().join("locales")).unwrap();
        std::fs::write(dir.path().join("locales/ko.json"), r#"{"errors": {"server_not_running": "서버가 꺼져 있어요", "timeout": {"detail": "x"}}}"#).unwrap();
        let metadata = crate::supervisor::module_loader::parse_module_toml(
            "[module]\nname = \"m\"\nversion = \"1\"\nentry = \"lifecycle.py\"\n[errors]\nserver_not_running = \"Server is off\"\ntimeout = \"Timed out\"\n",
        )
        .unwrap();
        let module = crate::supervisor::module_loader::LoadedModule { metadata, path: dir.path().to_string_lossy().into() };

        let ko = module_errors(&module, "ko");
        assert_eq!(ko["server_not_running"], "서버가 꺼져 있어요");
        // 번역이 없거나 문자열이 아니면 module.toml 문구
        assert_eq!(ko["timeout"], "Timed out");
        assert_eq!(module_errors(&module, "ja")["server_not_running"], "Server is off");
    }
}
