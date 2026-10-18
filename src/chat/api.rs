//! 채팅 처리기가 데몬에게 바라는 기능 (슬롯)
//!
//! 처리기는 이 트레이트만 알고, 실제 구현은 IPCServer(기존 핸들러 재사용)가
//! 맡습니다. 테스트에서는 가짜 구현을 꽂아 처리기만 따로 검증합니다.

use crate::ipc::internal::{into_internal, InternalResponse};
use crate::ipc::{CommandRequest, IPCServer};
use crate::supervisor::module_loader::CommandField;
use axum::extract::{Path, State};
use axum::Json;
use serde::Deserialize;
use serde_json::json;
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
    pub module_aliases: Vec<String>,
    /// 명령어 이름 → 별명 목록 (module.toml [aliases.commands])
    pub command_aliases: Vec<(String, Vec<String>)>,
    pub commands: Vec<CommandField>,
    /// [errors] 오류 코드 → 문구
    pub errors: HashMap<String, String>,
    /// [[presets]] 추천 설정 묶음 (heartbeat로 relay에 id·이름만 나간다)
    pub presets: Vec<crate::supervisor::module_loader::ModulePreset>,
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
    /// 서버 시작 — 경로(managed 여부)는 데몬이 고른다
    async fn start(&self, server: &ServerView) -> InternalResponse;
    async fn stop(&self, server: &ServerView) -> InternalResponse;
    /// 게임 명령 (`POST /api/instance/:id/command`와 같은 경로). 인자는 순서대로.
    async fn command(&self, instance_id: &str, command: &str, args: Vec<String>) -> InternalResponse;
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
                    module_aliases: meta.module_aliases,
                    command_aliases,
                    commands: meta.commands.map(|c| c.fields).unwrap_or_default(),
                    errors: meta.errors,
                    presets: meta.presets,
                }
            })
            .collect()
    }

    async fn start(&self, server: &ServerView) -> InternalResponse {
        into_internal(crate::ipc::handlers::server::start_instance_handler(Path(server.id.clone()), State(self.clone())).await).await
    }

    async fn stop(&self, server: &ServerView) -> InternalResponse {
        into_internal(crate::ipc::handlers::server::stop_instance(self.clone(), &server.id, false).await).await
    }

    async fn command(&self, instance_id: &str, command: &str, args: Vec<String>) -> InternalResponse {
        let req = CommandRequest { command: command.to_string(), args: json!(args) };
        into_internal(crate::ipc::handlers::command::execute_command(Path(instance_id.to_string()), State(self.clone()), Json(req)).await).await
    }
}

/// 데몬의 현재 봇 설정으로 채팅 명령 한 건을 처리합니다. 언어는 요청의 `locale`, 없으면 GUI 언어.
pub async fn run_with_daemon_settings(server: &IPCServer, req: &super::ChatRequest) -> super::Reply {
    let bot = server.config_store.get_bot_config().await;
    let language = match &req.locale {
        Some(locale) => locale.clone(),
        None => server.config_store.ui_language().await,
    };
    let catalog = super::i18n::Catalog::for_language(&language);
    super::process(server, &super::ChatSettings::from(&bot), &catalog, req).await
}
