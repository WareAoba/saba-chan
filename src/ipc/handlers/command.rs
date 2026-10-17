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

    let (instance, metadata, running, stdin_route) = {
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
        let metadata = supervisor.module_loader.get_module(&instance.module_name).ok().map(|m| m.metadata);
        let running = supervisor.tracker.get_pid(&instance.id).is_ok();
        let stdin_route = supervisor.routes_commands_to_stdin(&instance.id).await;
        (instance, metadata, running, stdin_route)
    };
    let errors = metadata.as_ref().map(|m| m.errors.clone()).unwrap_or_default();
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
        Plan::Rcon { line } => {
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

/// GET /api/instance/:id/commands — 이 인스턴스에서 쓸 수 있는 명령 전체 (모듈 + 활성 익스텐션)
///
/// `{commands: [{name, label, description, risk, inputs, source: "module"|"extension", extension?, aliases?}]}`
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
    Json(json!({ "commands": commands })).into_response()
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

    // 관리 모드면 stdin으로도 보낼 수 있다. RCON을 지원하는 게임은 RCON을 먼저 쓴다 (서버의 답이 돌아온다).
    // RCON이 안 되면 (아직 켜지는 중 등) stdin으로 보낸다. GUI·디스코드·모듈 역호출이 모두 이 엔드포인트를 쓴다.
    let stdin_ok = supervisor.routes_commands_to_stdin(&id).await;
    let rcon_supported = supervisor
        .module_loader
        .get_module(&instance.module_name)
        .ok()
        .and_then(|m| m.metadata.protocols_supported)
        .is_some_and(|p| p.iter().any(|s| s == "rcon"));
    if stdin_ok && !rcon_supported {
        drop(supervisor);
        return via_stdin(&state, &id, &payload).await;
    }

    // 모듈에서 기본값 가져오기
    let (default_rcon_port, _default_rest_port, module_path) =
        match supervisor.module_loader.get_module(&instance.module_name) {
            Ok(module) => (
                module.metadata.default_rcon_port(),
                module.metadata.default_rest_port(),
                Some(module.entry_path()),
            ),
            Err(_) => (25575, 8212, None),
        };

    // 게임 설정 파일에서 실제 credential을 읽어옴
    let instance_dir = supervisor.instance_store.instance_dir(&id);
    let working_dir = instance.working_dir.clone()
        .unwrap_or_else(|| instance_dir.join("server").to_string_lossy().to_string());

    // supervisor lock 해제 — run_plugin 전에 release
    drop(supervisor);

    let file_credentials = instance_credentials(&state, &id, module_path.as_deref(), &working_dir).await;

    // RCON 커맨드 추출
    let command = match payload.get("command").and_then(|v| v.as_str()) {
        Some(cmd) => cmd,
        None => {
            return (
                StatusCode::BAD_REQUEST,
                Json(json!({
                    "error": "Missing 'command' field"
                })),
            )
                .into_response();
        }
    };

    // RCON 정보 — 게임 설정 파일에서만 읽음
    let rcon_host = "127.0.0.1".to_string();

    let rcon_port = file_credentials.as_ref()
        .and_then(|c| c.get("rcon_port"))
        .and_then(|v| v.as_u64())
        .map(|p| p as u16)
        .or(instance.rcon_port)
        .unwrap_or(default_rcon_port);

    let rcon_password = match file_credentials.as_ref()
        .and_then(|c| c.get("rcon_password"))
        .and_then(|v| v.as_str()) {
        Some(pass) => pass.to_string(),
        None if stdin_ok => return via_stdin(&state, &id, &payload).await,
        None => {
            return (
                StatusCode::BAD_REQUEST,
                Json(json!({
                    "error": "RCON password not configured (not found in game config file)"
                })),
            )
                .into_response();
        }
    };

    // RCON 풀을 통해 명령어 실행 (연결 재사용, 실패 시 자동 재연결)
    let rcon_timeout = std::time::Duration::from_secs(5);
    let rcon_pool = state.rcon_pool.clone();
    let instance_id = id.clone();
    let command_owned = command.to_string();

    // RCON은 동기 I/O이므로 spawn_blocking으로 실행
    let result = tokio::task::spawn_blocking(move || {
        rcon_pool.execute(
            &instance_id,
            &rcon_host,
            rcon_port,
            &rcon_password,
            &command_owned,
            rcon_timeout,
        )
    })
    .await;

    match result {
        Ok(Ok(response_text)) => {
            let cmd_str = payload.get("command").and_then(|v| v.as_str()).unwrap_or("");
            (
                StatusCode::OK,
                Json(json!({
                    "success": true,
                    "data": { "response": response_text, "executed": cmd_str },
                    "error": null,
                    "command": cmd_str,
                    "host": "127.0.0.1",
                    "port": rcon_port,
                    "protocol": "rcon"
                })),
            )
                .into_response()
        }
        Ok(Err(e)) => {
            // 비밀번호·포트가 바뀌었을 수 있다 — 다음 명령은 설정 파일을 다시 읽는다
            state.credentials.invalidate(&id);
            // 보내지도 못했을 때만 stdin으로 (이미 간 명령을 또 보내면 두 번 실행된다)
            if stdin_ok && !matches!(e, crate::protocol::ProtocolError::CommandError(_)) {
                tracing::debug!("RCON failed for '{}' ({}), sending via stdin", id, e);
                return via_stdin(&state, &id, &payload).await;
            }
            let error_msg = format!("RCON execution failed: {}", e);
            tracing::warn!("{}", error_msg);
            (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(json!({ "error": error_msg })),
            )
                .into_response()
        }
        Err(e) => {
            let error_msg = format!("RCON task failed: {}", e);
            tracing::error!("{}", error_msg);
            (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(json!({ "error": error_msg })),
            )
                .into_response()
        }
    }
}

/// 관리 프로세스의 stdin으로 보낸다. 서버의 답은 콘솔로만 나오므로, 보낸 뒤 잠깐 새로 찍힌 줄을
/// 모아 `data.response`로 돌려준다 (채팅·CLI가 결과를 보여 줄 수 있게)
pub(crate) async fn via_stdin(state: &IPCServer, id: &str, payload: &serde_json::Value) -> axum::response::Response {
    use crate::supervisor::managed_process::LogSource;
    let Some(command) = payload.get("command").and_then(|v| v.as_str()) else {
        return (StatusCode::BAD_REQUEST, Json(json!({ "error": "Missing 'command' field" }))).into_response();
    };
    let proc = state.supervisor.read().await.managed_store.get(id).await;
    let before = match &proc {
        Some(p) => p.get_recent_console(1).await.last().map(|l| l.id).unwrap_or(0),
        None => 0,
    };
    let sent = state.supervisor.read().await.send_stdin_command(id, command).await;
    let message = match sent {
        Ok(m) => m,
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
        Execute(String),
    }
    let outcome = tokio::task::spawn_blocking(move || {
        client
            .connect_rest(std::time::Duration::from_secs(5))
            .map_err(|e| RestFailure::Connect(e.to_string()))?;
        client.execute(cmd).map_err(|e| RestFailure::Execute(e.to_string()))
    })
    .await
    .unwrap_or_else(|e| Err(RestFailure::Execute(format!("task failed: {}", e))));

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
        Err(RestFailure::Execute(e)) => (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(json!({
                "error": format!("REST execution failed: {}", e)
            })),
        )
            .into_response(),
    }
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
