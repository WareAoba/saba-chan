use axum::{
    extract::{Path, State},
    http::StatusCode,
    response::IntoResponse,
    Json,
};
use serde_json::json;

use super::super::{CommandRequest, IPCServer};

/// POST /api/instance/:id/command - 명령어 실행
pub async fn execute_command(
    Path(id): Path<String>,
    State(state): State<IPCServer>,
    Json(req): Json<CommandRequest>,
) -> impl IntoResponse {
    let supervisor = state.supervisor.read().await;

    // 인스턴스 확인
    let instance = match supervisor.instance_store.get(&id) {
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

    // Supervisor를 통해 명령어 전달
    let result = supervisor
        .execute_command(&instance.id, &instance.module_name, &req.command, req.args)
        .await;

    match result {
        Ok(message) => (
            StatusCode::OK,
            Json(json!({
                "success": true,
                "message": message
            })),
        )
            .into_response(),
        Err(e) => (
            StatusCode::BAD_REQUEST,
            Json(json!({
                "error": e.to_string()
            })),
        )
            .into_response(),
    }
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

    // 모듈에서 기본값 가져오기
    let (default_rcon_port, _default_rest_port, module_path) =
        match supervisor.module_loader.get_module(&instance.module_name) {
            Ok(module) => (
                module.metadata.default_rcon_port(),
                module.metadata.default_rest_port(),
                Some(format!("{}/lifecycle.py", module.path)),
            ),
            Err(_) => (25575, 8212, None),
        };

    // 게임 설정 파일에서 실제 credential을 읽어옴
    let instance_dir = supervisor.instance_store.instance_dir(&id);
    let working_dir = instance.working_dir.clone()
        .unwrap_or_else(|| instance_dir.join("server").to_string_lossy().to_string());

    // supervisor lock 해제 — run_plugin 전에 release
    drop(supervisor);

    let file_credentials = if let Some(ref mp) = module_path {
        tracing::info!("get_credentials(rcon): calling plugin at {} with working_dir={}", mp, working_dir);
        let cred_config = json!({ "working_dir": &working_dir });
        match crate::plugin::run_plugin(mp, "get_credentials", cred_config).await {
            Ok(creds) if creds.get("success").and_then(|v| v.as_bool()).unwrap_or(false) => {
                tracing::info!("get_credentials(rcon): success");
                Some(creds)
            }
            Ok(creds) => {
                tracing::warn!("get_credentials(rcon) returned non-success: {:?}", creds);
                None
            }
            Err(e) => {
                tracing::warn!("get_credentials(rcon) call failed: {}", e);
                None
            }
        }
    } else {
        tracing::warn!("get_credentials(rcon): no module_path, skipping");
        None
    };

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

    // RCON 클라이언트 생성 및 실행 (연결 실패 시 최대 2회 재시도)
    let rcon_timeout = std::time::Duration::from_secs(5);
    let mut last_error = String::new();

    for attempt in 0..3 {
        if attempt > 0 {
            tracing::info!(
                "RCON retry attempt {} for command '{}'",
                attempt + 1,
                command
            );
            tokio::time::sleep(std::time::Duration::from_millis(500)).await;
        }

        let mut client = crate::protocol::client::ProtocolClient::new_rcon(
            rcon_host.clone(),
            rcon_port,
            rcon_password.clone(),
        );

        match client.connect_rcon(rcon_timeout) {
            Ok(_) => {
                let cmd = crate::protocol::ServerCommand {
                    command_type: crate::protocol::CommandType::Rcon,
                    command: Some(command.to_string()),
                    endpoint: None,
                    method: None,
                    body: None,
                    timeout_secs: payload.get("timeout").and_then(|v| v.as_u64()),
                };

                match client.execute(cmd) {
                    Ok(response) => {
                        return (
                            StatusCode::OK,
                            Json(json!({
                                "success": response.success,
                                "data": response.data,
                                "error": response.error,
                                "command": command,
                                "host": rcon_host,
                                "port": rcon_port,
                                "protocol": "rcon"
                            })),
                        )
                            .into_response();
                    }
                    Err(e) => {
                        last_error = format!("RCON execution failed: {}", e);
                        tracing::warn!("{} (attempt {})", last_error, attempt + 1);
                    }
                }
            }
            Err(e) => {
                last_error = format!("RCON connection failed: {}", e);
                tracing::warn!("{} (attempt {})", last_error, attempt + 1);
            }
        }
    }

    (
        StatusCode::INTERNAL_SERVER_ERROR,
        Json(json!({ "error": last_error })),
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
                Some(format!("{}/lifecycle.py", module.path)),
            ),
            Err(_) => (8212, "127.0.0.1".to_string(), None),
        };

    // 게임 설정 파일에서 실제 credential을 읽어옴 (메모리 동기화 문제 방지)
    let instance_dir = supervisor.instance_store.instance_dir(&id);
    let working_dir = instance.working_dir.clone()
        .unwrap_or_else(|| instance_dir.join("server").to_string_lossy().to_string());

    // supervisor lock 해제 — run_plugin 전에 release
    drop(supervisor);

    let file_credentials = if let Some(ref mp) = module_path {
        tracing::info!("get_credentials: calling plugin at {} with working_dir={}", mp, working_dir);
        let cred_config = json!({ "working_dir": &working_dir });
        match crate::plugin::run_plugin(mp, "get_credentials", cred_config).await {
            Ok(creds) if creds.get("success").and_then(|v| v.as_bool()).unwrap_or(false) => {
                tracing::info!("get_credentials: success, rest_password={}",
                    creds.get("rest_password").and_then(|v| v.as_str()).unwrap_or("(none)").chars().take(4).collect::<String>() + "...");
                Some(creds)
            }
            Ok(creds) => {
                tracing::warn!("get_credentials returned non-success: {:?}", creds);
                None
            }
            Err(e) => {
                tracing::warn!("get_credentials call failed: {}", e);
                None
            }
        }
    } else {
        tracing::warn!("get_credentials: no module_path, skipping");
        None
    };

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

    // REST 연결 검증
    if let Err(e) = client.connect_rest(std::time::Duration::from_secs(5)) {
        return (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(json!({
                "error": format!("REST connection failed: {}", e)
            })),
        )
            .into_response();
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

    match client.execute(cmd) {
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
        Err(e) => (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(json!({
                "error": format!("REST execution failed: {}", e)
            })),
        )
            .into_response(),
    }
}
