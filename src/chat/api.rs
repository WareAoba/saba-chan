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
    /// 게임명: module.toml `display_name`과 모듈 locales마다의 `display_name` (`game_names`). 모듈 별명처럼 서버를 부른다
    pub game_names: Vec<String>,
    /// 명령어 이름 → 별명 목록 (module.toml [aliases.commands])
    pub command_aliases: Vec<(String, Vec<String>)>,
    pub commands: Vec<CommandField>,
    /// [errors] 오류 코드 → 문구
    pub errors: HashMap<String, String>,
    /// [[presets]] 추천 설정 묶음 (heartbeat로 relay에 id·이름만 나간다)
    pub presets: Vec<crate::supervisor::module_loader::ModulePreset>,
    /// `[commands] failure_patterns`: 콘솔 답이 맞으면 실패
    pub failure_patterns: Vec<String>,
    /// 모듈 폴더 (`locales/<lang>.json`을 읽는다). 없으면 module.toml 문구만
    pub locale_dir: Option<std::path::PathBuf>,
    /// `[status] players`를 선언했다 (채팅 `players`가 접속자 조회)
    pub players: bool,
}

impl ModuleView {
    pub fn command(&self, name: &str) -> Option<&CommandField> {
        self.commands.iter().find(|c| c.name == name)
    }

    /// 명령의 module.toml 별칭 (없으면 빈 목록)
    pub fn aliases_of(&self, command: &str) -> &[String] {
        self.command_aliases.iter().find(|(c, _)| c == command).map(|(_, a)| a.as_slice()).unwrap_or(&[])
    }

    /// 모듈 locales의 문구 (`pointer`는 `/commands/say/done` 꼴). 그 언어에 없으면 영어
    pub fn text(&self, lang: &str, pointer: &str) -> Option<String> {
        let dir = self.locale_dir.as_ref()?;
        let read = |lang: &str| -> Option<String> {
            let raw = std::fs::read_to_string(dir.join("locales").join(format!("{}.json", lang))).ok()?;
            serde_json::from_str::<serde_json::Value>(&raw).ok()?.pointer(pointer)?.as_str().map(str::to_string)
        };
        read(lang).or_else(|| read("en"))
    }
}

/// 모듈의 게임명: module.toml `display_name`, 그다음 언어마다 모듈 locales의 `display_name`.
/// 명령의 첫 낱말로 부르므로 띄어 쓴 이름은 빼고, 대소문자만 다른 이름은 하나로 둔다
pub fn game_names(display_name: Option<&str>, module_dir: Option<&std::path::Path>) -> Vec<String> {
    let localized = crate::chat::i18n::languages().into_iter().filter_map(|lang| {
        let raw = std::fs::read_to_string(module_dir?.join("locales").join(format!("{}.json", lang))).ok()?;
        serde_json::from_str::<serde_json::Value>(&raw).ok()?.get("display_name")?.as_str().map(str::to_string)
    });
    let mut out: Vec<String> = Vec::new();
    for name in display_name.map(str::to_string).into_iter().chain(localized) {
        let name = name.trim();
        if !name.is_empty() && !name.chars().any(char::is_whitespace) && !out.iter().any(|n| crate::chat::words::same(n, name)) {
            out.push(name.to_string());
        }
    }
    out
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
    /// 접속자 (`GET /api/instance/:id/players`)
    async fn players(&self, server: &ServerView) -> InternalResponse;
}

impl DaemonApi for IPCServer {
    async fn servers(&self) -> Result<Vec<ServerView>, String> {
        let resp = into_internal(crate::ipc::handlers::server::list_servers_internal(self).await).await;
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
                let locale_dir = Some(std::path::PathBuf::from(&m.path));
                let meta = m.metadata;
                let mut command_aliases: Vec<(String, Vec<String>)> = meta
                    .command_aliases
                    .into_iter()
                    .map(|(cmd, info)| (cmd, info.aliases))
                    .collect();
                command_aliases.sort_by(|a, b| a.0.cmp(&b.0));
                ModuleView {
                    game_names: game_names(meta.display_name.as_deref(), locale_dir.as_deref()),
                    name: meta.name,
                    module_aliases: meta.module_aliases,
                    command_aliases,
                    failure_patterns: meta.commands.as_ref().map(|c| c.failure_patterns.clone()).unwrap_or_default(),
                    commands: meta.commands.map(|c| c.fields).unwrap_or_default(),
                    errors: meta.errors,
                    presets: meta.presets,
                    locale_dir,
                    players: meta.players_function.is_some(),
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

    async fn players(&self, server: &ServerView) -> InternalResponse {
        use axum::response::IntoResponse;
        into_internal(crate::ipc::handlers::managed::players_handler(Path(server.id.clone()), State(self.clone())).await.into_response()).await
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn game_names_come_from_each_locale_display_name() {
        let tmp = tempfile::tempdir().unwrap();
        let locales = tmp.path().join("locales");
        std::fs::create_dir_all(&locales).unwrap();
        std::fs::write(locales.join("ko.json"), r#"{"display_name": "마인크래프트"}"#).unwrap();
        std::fs::write(locales.join("en.json"), r#"{"display_name": "Minecraft"}"#).unwrap();
        std::fs::write(locales.join("ja.json"), r#"{"display_name": "minecraft"}"#).unwrap();
        std::fs::write(locales.join("de.json"), r#"{"display_name": "Mine Craft"}"#).unwrap();
        // 띄어 쓴 이름은 명령 첫 낱말이 될 수 없어 뺀다. 대소문자만 다르면 하나로
        let got = game_names(Some("Minecraft"), Some(tmp.path()));
        assert_eq!(got, vec!["Minecraft", "마인크래프트"]);
        assert_eq!(game_names(Some("Terraria"), None), vec!["Terraria"]);
    }
}
