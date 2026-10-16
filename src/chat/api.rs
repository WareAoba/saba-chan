//! 채팅 처리기가 데몬에게 바라는 기능 (슬롯)
//!
//! 처리기는 이 트레이트만 알고, 실제 구현은 IPCServer(기존 핸들러 재사용)가
//! 맡습니다. 테스트에서는 가짜 구현을 꽂아 처리기만 따로 검증합니다.

use crate::ipc::internal::{into_internal, InternalResponse};
use crate::ipc::{CommandRequest, IPCServer, ServerStartRequest, ServerStopRequest};
use crate::supervisor::module_loader::CommandField;
use axum::extract::{Path, State};
use axum::Json;
use serde::Deserialize;
use serde_json::{json, Value};
use std::collections::HashMap;

/// 채팅 처리에 필요한 인스턴스 정보
#[derive(Debug, Clone, Deserialize)]
pub struct ServerView {
    pub id: String,
    pub name: String,
    pub module: String,
    pub status: String,
    #[serde(default)]
    pub pid: Option<u32>,
    #[serde(default)]
    pub module_settings: HashMap<String, Value>,
}

impl ServerView {
    pub fn is_running(&self) -> bool {
        self.status == "running"
    }
}

/// 채팅 처리에 필요한 모듈 정보
#[derive(Debug, Clone, Default)]
pub struct ModuleView {
    pub name: String,
    pub interaction_mode: Option<String>,
    pub module_aliases: Vec<String>,
    /// 명령어 이름 → 별명 목록 (module.toml [aliases.commands])
    pub command_aliases: Vec<(String, Vec<String>)>,
    pub commands: Vec<CommandField>,
    /// [errors] 오류 코드 → 문구
    pub errors: HashMap<String, String>,
}

impl ModuleView {
    pub fn command(&self, name: &str) -> Option<&CommandField> {
        self.commands.iter().find(|c| c.name == name)
    }
}

/// 처리기가 쓰는 데몬 기능. 응답은 HTTP 핸들러와 같은 (상태, JSON) 형태.
#[allow(async_fn_in_trait)]
pub trait DaemonApi {
    async fn servers(&self) -> Result<Vec<ServerView>, String>;
    async fn modules(&self) -> Vec<ModuleView>;
    async fn start(&self, server: &ServerView, managed: bool) -> InternalResponse;
    async fn stop(&self, server: &ServerView) -> InternalResponse;
    async fn rcon(&self, instance_id: &str, command: &str) -> InternalResponse;
    async fn rest(&self, instance_id: &str, endpoint: &str, http_method: &str, body: Value) -> InternalResponse;
    async fn module_command(&self, instance_id: &str, command: &str, args: Value) -> InternalResponse;
}

impl DaemonApi for IPCServer {
    async fn servers(&self) -> Result<Vec<ServerView>, String> {
        let resp = into_internal(crate::ipc::handlers::server::list_servers(State(self.clone())).await).await;
        if !resp.is_success() {
            return Err(resp.str_field("error").unwrap_or("failed to list servers").to_string());
        }
        let servers = resp.body.get("servers").cloned().unwrap_or(json!([]));
        serde_json::from_value(servers).map_err(|e| e.to_string())
    }

    async fn modules(&self) -> Vec<ModuleView> {
        let supervisor = self.supervisor.read().await;
        let modules = match supervisor.list_modules() {
            Ok(m) => m,
            Err(e) => {
                tracing::warn!("[Chat] 모듈 목록 조회 실패: {}", e);
                return Vec::new();
            }
        };
        modules
            .into_iter()
            .map(|m| {
                let meta = m.metadata;
                let mut command_aliases: Vec<(String, Vec<String>)> = meta
                    .command_aliases
                    .into_iter()
                    .map(|(cmd, info)| (cmd, info.aliases))
                    .collect();
                command_aliases.sort_by(|a, b| a.0.cmp(&b.0));
                ModuleView {
                    name: meta.name,
                    interaction_mode: meta.interaction_mode,
                    module_aliases: meta.module_aliases,
                    command_aliases,
                    commands: meta.commands.map(|c| c.fields).unwrap_or_default(),
                    errors: meta.errors,
                }
            })
            .collect()
    }

    async fn start(&self, server: &ServerView, managed: bool) -> InternalResponse {
        use crate::ipc::handlers::{managed::start_managed_handler, server::start_server_handler};
        if managed {
            into_internal(start_managed_handler(Path(server.id.clone()), State(self.clone()), Some(Json(json!({})))).await).await
        } else {
            let req = ServerStartRequest { module: server.module.clone(), config: json!({}) };
            into_internal(start_server_handler(Path(server.name.clone()), State(self.clone()), Json(req)).await).await
        }
    }

    async fn stop(&self, server: &ServerView) -> InternalResponse {
        let req = ServerStopRequest { force: false };
        into_internal(crate::ipc::handlers::server::stop_server_handler(Path(server.name.clone()), State(self.clone()), Json(req)).await).await
    }

    async fn rcon(&self, instance_id: &str, command: &str) -> InternalResponse {
        let body = json!({ "command": command });
        into_internal(crate::ipc::handlers::command::execute_rcon_command(Path(instance_id.to_string()), State(self.clone()), Json(body)).await).await
    }

    async fn rest(&self, instance_id: &str, endpoint: &str, http_method: &str, body: Value) -> InternalResponse {
        // 호스트/포트는 넘기지 않는다 — 핸들러가 인스턴스 설정 → 모듈 기본값 순으로 채운다
        let payload = json!({ "endpoint": endpoint, "method": http_method, "body": body });
        into_internal(crate::ipc::handlers::command::execute_rest_command(Path(instance_id.to_string()), State(self.clone()), Json(payload)).await).await
    }

    async fn module_command(&self, instance_id: &str, command: &str, args: Value) -> InternalResponse {
        let req = CommandRequest { command: command.to_string(), args };
        into_internal(crate::ipc::handlers::command::execute_command(Path(instance_id.to_string()), State(self.clone()), Json(req)).await).await
    }
}

/// 데몬의 현재 봇 설정·언어로 채팅 명령 한 건을 처리합니다.
pub async fn run_with_daemon_settings(server: &IPCServer, req: &super::ChatRequest) -> String {
    let bot = server.config_store.get_bot_config().await;
    let language = server.config_store.get_gui_settings().await.language.unwrap_or_else(|| "en".into());
    let catalog = super::i18n::Catalog::for_language(&language);
    super::process(server, &super::ChatSettings::from(&bot), &catalog, req).await
}
