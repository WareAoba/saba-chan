//! 채팅 명령 처리기
//!
//! 디스코드 등 채팅에서 들어온 텍스트 명령("팰월드 시작", "mc players")을 해석해
//! 데몬 기능으로 실행하고, 사용자에게 돌려줄 한 줄 응답을 만듭니다.
//! 릴레이 에이전트(`relay_agent`)와 `POST /api/chat/command`가 이 처리기를 씁니다.
//!
//! 처리 순서:
//!   1. 빈 입력 / help / 도움 → 도움말
//!   2. list / 목록 → 서버 목록
//!   3. 모듈(또는 인스턴스 이름)만 → 그 모듈의 명령어 안내
//!   4. 모듈 + 명령어 [+ 인자] → 내장(start/stop/status) 또는 module.toml 정의 명령 실행
//!
//! 새 최상위 명령은 `TopLevel`에, 새 실행 방식은 `Method`에 한 갈래 추가하면 됩니다.

pub mod aliases;
pub mod api;
pub mod format;
pub mod i18n;

use crate::config_store::BotConfig;
use crate::ipc::internal::InternalResponse;
use crate::supervisor::module_loader::CommandField;
use aliases::ModuleAliases;
use api::{DaemonApi, ModuleView, ServerView};
use i18n::Catalog;
use serde_json::{Map, Value};
use std::collections::HashMap;

/// 처리기가 쓰는 봇 설정 일부
#[derive(Debug, Clone, Default)]
pub struct ChatSettings {
    pub prefix: String,
    pub module_aliases: HashMap<String, String>,
    pub command_aliases: HashMap<String, Value>,
    pub node_settings: Value,
}

impl From<&BotConfig> for ChatSettings {
    fn from(cfg: &BotConfig) -> Self {
        Self {
            prefix: cfg.prefix.clone(),
            module_aliases: cfg.module_aliases.clone(),
            command_aliases: cfg.command_aliases.clone(),
            node_settings: cfg.node_settings.clone(),
        }
    }
}

/// 채팅 명령 한 건
#[derive(Debug, Clone, Default, serde::Deserialize)]
pub struct ChatRequest {
    pub text: String,
    /// 디스코드 길드 ID — 노드별 허용 인스턴스/멤버 권한 조회 키
    #[serde(default, alias = "guildId")]
    pub guild_id: Option<String>,
    /// 요청한 사용자 ID — 멤버 권한 조회 키
    #[serde(default, alias = "userId")]
    pub user_id: Option<String>,
}

/// 최상위 명령 (모듈 이름보다 먼저 확인)
enum TopLevel {
    Help,
    List,
}

impl TopLevel {
    fn parse(word: &str) -> Option<Self> {
        match word.to_lowercase().as_str() {
            "help" | "도움" | "도움말" => Some(Self::Help),
            "list" | "목록" => Some(Self::List),
            _ => None,
        }
    }
}

/// module.toml 명령의 실행 방식
enum Method<'a> {
    Rcon,
    Rest,
    /// 모듈 lifecycle.py의 명령 핸들러 (module.toml의 "dual"/"both")
    Module,
    Unsupported(&'a str),
}

impl<'a> Method<'a> {
    fn of(cmd: &'a CommandField) -> Self {
        match cmd.method.as_deref().unwrap_or("") {
            "rcon" => Self::Rcon,
            "rest" => Self::Rest,
            "dual" | "both" => Self::Module,
            other => Self::Unsupported(other),
        }
    }
}

/// 처리 문맥 — 한 요청 동안 쓰는 데이터 묶음
struct Ctx<'a, D: DaemonApi> {
    api: &'a D,
    settings: &'a ChatSettings,
    cat: &'a Catalog,
    req: &'a ChatRequest,
    modules: Vec<ModuleView>,
    aliases: ModuleAliases,
}

/// 텍스트 명령을 처리하고 응답 문자열을 돌려줍니다. 실패해도 항상 응답을 만듭니다.
pub async fn process<D: DaemonApi>(api: &D, settings: &ChatSettings, cat: &Catalog, req: &ChatRequest) -> String {
    let modules = api.modules().await;
    let aliases = ModuleAliases::build(&modules, &settings.module_aliases);
    let ctx = Ctx { api, settings, cat, req, modules, aliases };

    let text = strip_prefix(req.text.trim(), &settings.prefix);
    let args: Vec<&str> = text.split_whitespace().collect();

    let Some(first) = args.first().copied() else {
        return ctx.help().await;
    };
    match TopLevel::parse(first) {
        Some(TopLevel::Help) => return ctx.help().await,
        Some(TopLevel::List) => return ctx.list().await,
        None => {}
    }

    let servers = match ctx.servers().await {
        Ok(s) => s,
        Err(e) => {
            tracing::warn!("[Chat] 서버 목록 조회 실패: {}", e);
            return format!("❌ {}", cat.t("messages.command_error", &[]));
        }
    };

    let module_known = ctx.aliases.is_known(first);
    let mounted = ctx.is_mounted(first, &servers);
    match args.get(1) {
        None => {
            // 모듈 별명이나 인스턴스 이름이 실제 서버로 있어야 명령어 안내
            if !mounted {
                return cat.t("errors.unknown_input", &[]);
            }
            ctx.module_help(first, &servers)
        }
        Some(cmd) => {
            if module_known && !mounted {
                return cat.t("errors.unknown_input", &[]);
            }
            ctx.module_command(first, cmd, &args[2..], &servers).await
        }
    }
}

/// 릴레이는 접두사 없이 보내지만, 사용자가 붙여 보낸 경우도 받아 준다
fn strip_prefix<'t>(text: &'t str, prefix: &str) -> &'t str {
    if !prefix.is_empty() {
        if let Some(rest) = text.strip_prefix(prefix) {
            return rest.trim_start();
        }
    }
    text
}

impl<D: DaemonApi> Ctx<'_, D> {
    fn t(&self, key: &str, vars: &[(&str, &str)]) -> String {
        self.cat.t(key, vars)
    }

    // ── 노드 설정 (허용 인스턴스 / 멤버 권한) ──

    /// 길드(노드)별 설정. 설정이 없는 길드는 제한 없음.
    fn node_settings(&self) -> Option<&Value> {
        let guild = self.req.guild_id.as_deref()?;
        self.settings.node_settings.as_object()?.get(guild)
    }

    fn allowed_instances(&self) -> Option<Vec<&str>> {
        let list = self.node_settings()?.get("allowedInstances")?.as_array()?;
        Some(list.iter().filter_map(|v| v.as_str()).collect())
    }

    /// 사용자가 이 인스턴스에서 막힌 명령인지
    fn is_denied(&self, server_id: &str, command: &str) -> bool {
        let Some(user) = self.req.user_id.as_deref() else { return false };
        let Some(perms) = self.node_settings().and_then(|c| c.get("memberPermissions")).and_then(|p| p.get(user)) else {
            return false;
        };
        perms
            .get(server_id)
            .and_then(|v| v.as_array())
            .is_some_and(|denied| denied.iter().any(|c| c.as_str() == Some(command)))
    }

    async fn servers(&self) -> Result<Vec<ServerView>, String> {
        let servers = self.api.servers().await?;
        Ok(match self.allowed_instances() {
            Some(allowed) => servers.into_iter().filter(|s| allowed.contains(&s.id.as_str())).collect(),
            None => servers,
        })
    }

    fn module(&self, name: &str) -> Option<&ModuleView> {
        self.modules.iter().find(|m| m.name == name)
    }

    fn module_errors(&self, name: &str) -> Option<&HashMap<String, String>> {
        self.module(name).map(|m| &m.errors)
    }

    fn is_mounted(&self, alias: &str, servers: &[ServerView]) -> bool {
        let module = self.aliases.resolve(alias);
        servers.iter().any(|s| s.module == module || s.name.eq_ignore_ascii_case(alias) || s.name.to_lowercase() == alias.to_lowercase())
    }

    // ── 도움말 / 목록 ──

    async fn help(&self) -> String {
        let prefix = &self.settings.prefix;
        let mut mounted: Vec<String> = Vec::new();
        match self.servers().await {
            Ok(servers) => {
                for s in servers {
                    if !mounted.contains(&s.module) {
                        mounted.push(s.module);
                    }
                }
            }
            Err(e) => tracing::warn!("[Chat] 도움말용 서버 목록 실패: {}", e),
        }

        let mut out = format!("📖 **{}**\n{}\n", prefix, self.t("help.usage_format", &[("prefix", prefix)]));
        if mounted.is_empty() {
            out.push('\n');
            out.push_str(&self.t("help.no_modules", &[]));
            return out;
        }
        out.push('\n');
        out.push_str(&self.t("help.modules_header", &[]));
        out.push('\n');
        for m in &mounted {
            let aliases = self.aliases.aliases_of(m);
            if aliases.is_empty() {
                out.push_str(&format!("• **{}**\n", m));
            } else {
                out.push_str(&format!("• **{}** ({})\n", m, aliases.join(", ")));
            }
        }
        out
    }

    async fn list(&self) -> String {
        let servers = match self.servers().await {
            Ok(s) => s,
            Err(e) => {
                tracing::warn!("[Chat] 목록 조회 실패: {}", e);
                return format!("❌ {}", self.t("messages.command_error", &[]));
            }
        };
        if servers.is_empty() {
            return self.t("list.empty", &[]);
        }
        let lines: Vec<String> = servers
            .iter()
            .map(|s| {
                let (icon, status) = if s.is_running() {
                    ("🟢", self.t("status.running", &[]))
                } else {
                    ("⚪", self.t("status.stopped", &[]))
                };
                self.t("list.item", &[("name", &s.name), ("module", &s.module), ("status", &status), ("status_icon", icon)])
            })
            .collect();
        format!("{}\n{}", self.t("list.title", &[]), lines.join("\n"))
    }

    fn alias_conflict(&self, alias: &str) -> Option<String> {
        let modules = self.aliases.conflict(alias)?.join(", ");
        Some(self.t("errors.alias_conflict", &[("alias", alias), ("modules", &modules)]))
    }

    /// 별명/인스턴스 이름으로 대상 서버를 찾는다. 찾지 못하면 사용자에게 줄 응답을 Err로.
    fn find_server<'s>(&self, alias: &str, servers: &'s [ServerView]) -> Result<&'s ServerView, String> {
        if let Some(s) = servers.iter().find(|s| s.name.to_lowercase() == alias.to_lowercase()) {
            return Ok(s);
        }
        let module = self.aliases.resolve(alias);
        let matched: Vec<&ServerView> = servers.iter().filter(|s| s.module == module || s.name.contains(&module)).collect();
        match matched.as_slice() {
            [one] => Ok(one),
            [] => Err(self.t("server.not_found", &[("alias", alias), ("resolved", &module)])),
            _ => Err(self.t("errors.multiple_instances", &[("module", &module)])),
        }
    }

    fn module_help(&self, alias: &str, servers: &[ServerView]) -> String {
        if let Some(msg) = self.alias_conflict(alias) {
            return msg;
        }
        let prefix = &self.settings.prefix;
        // 인스턴스 이름으로 불렀으면 그 인스턴스의 모듈을 안내한다
        let module_name = servers
            .iter()
            .find(|s| s.name.to_lowercase() == alias.to_lowercase())
            .map(|s| s.module.clone())
            .unwrap_or_else(|| self.aliases.resolve(alias));

        let mut out = format!(
            "{}\n\n{}\n",
            self.t("modules.enter_command", &[]),
            self.t("help.module_title", &[("module", &module_name)])
        );
        for (cmd, key) in [("start", "modules.help_start"), ("stop", "modules.help_stop"), ("status", "modules.help_status")] {
            out.push_str(&format!("• `{} {} {}` - {}\n", prefix, alias, cmd, self.t(key, &[])));
        }
        if let Some(module) = self.module(&module_name) {
            if !module.commands.is_empty() {
                out.push_str(&format!("\n{}\n", self.t("modules.help_rest_title", &[])));
                for c in &module.commands {
                    let inputs: Vec<String> = c
                        .inputs
                        .iter()
                        .map(|i| if i.required.unwrap_or(false) { format!("<{}>", i.name) } else { format!("[{}]", i.name) })
                        .collect();
                    let usage = if inputs.is_empty() { c.name.clone() } else { format!("{} {}", c.name, inputs.join(" ")) };
                    let label = if c.label.is_empty() { &c.name } else { &c.label };
                    out.push_str(&format!("• `{} {} {}` - {}\n", prefix, alias, usage, label));
                }
            }
        }
        let same_module = servers.iter().filter(|s| s.module == module_name).count();
        if same_module > 1 && servers.iter().all(|s| s.name.to_lowercase() != alias.to_lowercase()) {
            out.push_str("\n\n");
            out.push_str(&self.t("errors.multiple_instances", &[("module", &module_name)]));
        }
        out
    }

    // ── 모듈 + 명령어 ──

    async fn module_command(&self, alias: &str, cmd_alias: &str, extra: &[&str], servers: &[ServerView]) -> String {
        if let Some(msg) = self.alias_conflict(alias) {
            return msg;
        }
        let server = match self.find_server(alias, servers) {
            Ok(s) => s,
            Err(msg) => return msg,
        };
        let Some(module) = self.module(&server.module) else {
            return self.t("server.module_missing", &[("module", &server.module)]);
        };
        let command = aliases::resolve_command(Some(module), &self.settings.command_aliases, cmd_alias);

        if self.is_denied(&server.id, &command) {
            tracing::info!("[Chat] 권한 거부: user={:?} server={} command={}", self.req.user_id, server.id, command);
            return self.t("errors.permission_denied", &[]);
        }

        match command.as_str() {
            "start" => self.start(server, module).await,
            "stop" => self.stop(server).await,
            "status" => self.status(server),
            _ => match module.command(&command) {
                Some(field) => self.run_defined(server, module, field, alias, cmd_alias, extra).await,
                None => self.t("errors.command_not_found", &[]),
            },
        }
    }

    async fn start(&self, server: &ServerView, module: &ModuleView) -> String {
        let resp = self.api.start(server, use_managed_start(server, module)).await;
        if !resp.is_success() {
            let detail = resp.str_field("error").unwrap_or_default();
            tracing::warn!("[Chat] 시작 실패 {}: {}", server.name, detail);
            return error_line(&self.t("errors.error_title", &[]), detail);
        }
        if resp.body.get("success").and_then(|v| v.as_bool()) == Some(false) {
            let msg = resp.str_field("message").or(resp.str_field("error")).map(str::to_string).unwrap_or_else(|| self.t("errors.error_title", &[]));
            return format!("❌ {}", msg);
        }
        if let Some(action) = resp.str_field("action_required") {
            let msg = resp.str_field("message").map(str::to_string).unwrap_or_else(|| format!("Action required: {}", action));
            return format!("⚠️ {}", msg);
        }
        self.t("server.start_complete", &[("name", &server.name)])
    }

    async fn stop(&self, server: &ServerView) -> String {
        let resp = self.api.stop(server).await;
        let failed = !resp.is_success() || resp.body.get("success").and_then(|v| v.as_bool()) == Some(false);
        if failed {
            let detail = resp.str_field("error").or(resp.str_field("message")).unwrap_or_default();
            tracing::warn!("[Chat] 정지 실패 {}: {}", server.name, detail);
            return error_line(&self.t("errors.error_title", &[]), detail);
        }
        self.t("server.stop_complete", &[("name", &server.name)])
    }

    fn status(&self, server: &ServerView) -> String {
        let status = if server.is_running() { self.t("status.running", &[]) } else { self.t("status.stopped", &[]) };
        let pid = server.pid.map(|p| format!("PID: {}", p)).unwrap_or_default();
        self.t("server.status_check", &[("name", &server.name), ("status", &status), ("pid_info", &pid)]).trim_end().to_string()
    }

    async fn run_defined(&self, server: &ServerView, module: &ModuleView, field: &CommandField, alias: &str, cmd_alias: &str, extra: &[&str]) -> String {
        let method = Method::of(field);
        if let Method::Unsupported(m) = method {
            let m = if m.is_empty() { "unknown" } else { m };
            return format!("❓ {}: {}", self.t("messages.command_error", &[]), m);
        }

        if !server.is_running() {
            let default = self.t("server.not_running_default", &[]);
            let error = module.errors.get("server_not_running").cloned().unwrap_or(default);
            return self.t("server.not_running", &[("name", &server.name), ("error", &error)]);
        }

        let inputs = match bind_inputs(field, extra) {
            Ok(v) => v,
            Err(missing) => {
                let description = if missing.label.as_deref().unwrap_or("").is_empty() { missing.name.clone() } else { missing.label.clone().unwrap_or_default() };
                return self.t(
                    "command.missing_required",
                    &[("arg_name", &missing.name), ("prefix", &self.settings.prefix), ("alias", alias), ("command", cmd_alias), ("description", &description)],
                );
            }
        };

        let resp = match method {
            Method::Rcon => {
                let line = render_rcon(field.rcon_template.as_deref().unwrap_or(&field.name), &inputs);
                self.api.rcon(&server.id, &line).await
            }
            Method::Rest => {
                let endpoint = field.endpoint_template.clone().unwrap_or_else(|| format!("/v1/api/{}", field.name));
                let http_method = field.http_method.as_deref().unwrap_or("GET").to_uppercase();
                self.api.rest(&server.id, &endpoint, &http_method, Value::Object(inputs)).await
            }
            Method::Module => self.api.module_command(&server.id, &field.name, Value::Object(inputs)).await,
            Method::Unsupported(_) => unreachable!("handled above"),
        };
        self.render_result(server, &module.name, &field.name, resp)
    }

    fn render_result(&self, server: &ServerView, module: &str, command: &str, resp: InternalResponse) -> String {
        let errors = self.module_errors(module);
        let module_msg = |code: &str| errors.and_then(|e| e.get(code)).cloned();

        if !resp.is_success() {
            let mapped = match resp.status.as_u16() {
                401 | 403 => module_msg("auth_failed").unwrap_or_else(|| self.t("errors.auth_failed", &[])),
                404 => resp.str_field("error").map(str::to_string).unwrap_or_else(|| self.t("errors.not_found", &[])),
                503 => module_msg("server_not_running").unwrap_or_else(|| self.t("errors.service_unavailable", &[])),
                _ => resp
                    .str_field("error_code")
                    .and_then(module_msg)
                    .or_else(|| module_msg("internal_server_error"))
                    .unwrap_or_else(|| self.t("errors.internal_server_error", &[])),
            };
            if let Some(detail) = resp.str_field("error") {
                tracing::warn!("[Chat] {} {} 실패 ({}): {}", server.name, command, resp.status, detail);
            }
            return format!("❌ {}: {}", self.t("errors.error_title", &[]), mapped);
        }

        if resp.body.get("success").and_then(|v| v.as_bool()) == Some(true) {
            let text = format::format_response(resp.body.get("data").unwrap_or(&Value::Null), self.cat);
            return self.t("command.execute_complete", &[("name", &server.name), ("command", command), ("response", &text)]);
        }
        let raw = resp.str_field("error").map(str::to_string).unwrap_or_else(|| self.t("errors.unknown", &[]));
        let friendly = resp.str_field("error_code").and_then(module_msg).unwrap_or(raw);
        self.t("command.execute_failed", &[("name", &server.name), ("error", &friendly)])
    }
}

/// 관리형(stdin 캡처) 시작 여부: 인스턴스 설정 managed_start가 우선, 없으면 콘솔형 모듈이면 관리형.
pub fn use_managed_start(server: &ServerView, module: &ModuleView) -> bool {
    match server.module_settings.get("managed_start") {
        Some(Value::Bool(b)) => *b,
        Some(Value::String(s)) if s == "true" => true,
        Some(Value::String(s)) if s == "false" => false,
        _ => module.interaction_mode.as_deref() == Some("console"),
    }
}

/// 인자를 명령 입력 칸에 차례로 넣는다. 마지막 칸은 남은 인자를 모두 받는다
/// (예: "say hello world"의 메시지). 필수 칸이 비면 그 칸을 Err로.
fn bind_inputs<'f>(field: &'f CommandField, extra: &[&str]) -> Result<Map<String, Value>, &'f crate::supervisor::module_loader::CommandInput> {
    let mut out = Map::new();
    let last = field.inputs.len().saturating_sub(1);
    for (i, input) in field.inputs.iter().enumerate() {
        let value = if i == last && extra.len() > i { Some(extra[i..].join(" ")) } else { extra.get(i).map(|s| s.to_string()) };
        match value {
            Some(v) => {
                out.insert(input.name.clone(), Value::String(v));
            }
            None if input.required.unwrap_or(false) => return Err(input),
            None => {}
        }
    }
    Ok(out)
}

/// RCON 템플릿 채우기: {key}를 값으로, 남은 {key} 자리는 앞 공백과 함께 지운다
fn render_rcon(template: &str, inputs: &Map<String, Value>) -> String {
    let mut line = template.to_string();
    for (key, value) in inputs {
        let v = value.as_str().map(str::to_string).unwrap_or_else(|| value.to_string());
        line = line.replacen(&format!("{{{}}}", key), &v, 1);
    }
    static LEFTOVER: std::sync::OnceLock<regex::Regex> = std::sync::OnceLock::new();
    let re = LEFTOVER.get_or_init(|| regex::Regex::new(r"\s*\{\w+\}").expect("valid regex"));
    re.replace_all(&line, "").trim().to_string()
}

fn error_line(title: &str, detail: &str) -> String {
    if detail.is_empty() {
        format!("❌ {}", title)
    } else {
        format!("❌ {}: {}", title, detail)
    }
}

#[cfg(test)]
mod tests;
