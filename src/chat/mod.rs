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
use serde_json::{json, Value};
use std::collections::HashMap;

/// 처리기가 쓰는 봇 설정 일부
#[derive(Debug, Clone, Default)]
pub struct ChatSettings {
    pub prefix: String,
    pub module_aliases: HashMap<String, String>,
    pub command_aliases: HashMap<String, Value>,
    pub instance_aliases: HashMap<String, String>,
    pub node_settings: Value,
}

impl From<&BotConfig> for ChatSettings {
    fn from(cfg: &BotConfig) -> Self {
        Self {
            prefix: cfg.prefix.clone(),
            module_aliases: cfg.module_aliases.clone(),
            command_aliases: cfg.command_aliases.clone(),
            instance_aliases: cfg.instance_aliases.clone(),
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

/// 처리 문맥 — 한 요청 동안 쓰는 데이터 묶음
struct Ctx<'a, D: DaemonApi> {
    api: &'a D,
    settings: &'a ChatSettings,
    cat: &'a Catalog,
    req: &'a ChatRequest,
    modules: Vec<ModuleView>,
    aliases: ModuleAliases,
}

/// 처리 결과. `text`는 늘 사람이 읽을 문구다. 데몬이 입력을 알아보지 못했으면 `code`가 붙고,
/// 릴레이는 이 코드를 보고 모델 해석기에 정확한 명령을 물을 수 있다
#[derive(Debug, Clone, PartialEq)]
pub struct Reply {
    pub text: String,
    /// `UNKNOWN_COMMAND` | `INSTANCE_NOT_FOUND`. None이면 처리된 응답(성공이든 실행 오류든)
    pub code: Option<&'static str>,
    pub args: Value,
}

impl Reply {
    fn unrecognized(code: &'static str, text: String, args: Value) -> Self {
        Self { text, code: Some(code), args }
    }
}

impl From<String> for Reply {
    fn from(text: String) -> Self {
        Self { text, code: None, args: Value::Null }
    }
}

/// 텍스트 명령을 처리하고 응답을 돌려줍니다. 실패해도 항상 응답을 만듭니다.
pub async fn process<D: DaemonApi>(api: &D, settings: &ChatSettings, cat: &Catalog, req: &ChatRequest) -> Reply {
    let modules = api.modules().await;
    let aliases = ModuleAliases::build(&modules, &settings.module_aliases);
    let ctx = Ctx { api, settings, cat, req, modules, aliases };

    let text = strip_prefix(req.text.trim(), &settings.prefix);
    let args: Vec<&str> = text.split_whitespace().collect();

    let Some(first) = args.first().copied() else {
        return ctx.help().await.into();
    };
    match TopLevel::parse(first) {
        Some(TopLevel::Help) => return ctx.help().await.into(),
        Some(TopLevel::List) => return ctx.list().await.into(),
        None => {}
    }

    let servers = match ctx.servers().await {
        Ok(s) => s,
        Err(e) => {
            tracing::warn!("[Chat] 서버 목록 조회 실패: {}", e);
            return format!("❌ {}", cat.t("messages.command_error", &[])).into();
        }
    };

    let module_known = ctx.aliases.is_known(first);
    let mounted = ctx.is_mounted(first, &servers);
    match args.get(1) {
        None => {
            // 모듈 별명이나 인스턴스 이름이 실제 서버로 있어야 명령어 안내
            if !mounted {
                let command = text.trim();
                return Reply::unrecognized("UNKNOWN_COMMAND", cat.t("errors.unknown_input", &[]), json!({ "command": command }));
            }
            ctx.module_help(first, &servers).into()
        }
        Some(cmd) => {
            if module_known && !mounted {
                return Reply::unrecognized("INSTANCE_NOT_FOUND", cat.t("errors.unknown_input", &[]), json!({ "name": first }));
            }
            ctx.module_command(first, cmd, skip_words(text, 2), &servers).await
        }
    }
}

/// 앞의 낱말 `n`개를 건너뛴 나머지 원문 (따옴표 안의 공백을 그대로 두려고 나누기 전 문자열을 쓴다)
fn skip_words(text: &str, n: usize) -> &str {
    let mut rest = text.trim_start();
    for _ in 0..n {
        let end = rest.find(char::is_whitespace).unwrap_or(rest.len());
        rest = rest[end..].trim_start();
    }
    rest
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

    /// 인스턴스 이름이나 인스턴스 별명으로 부른 서버들
    fn named_servers<'s>(&self, alias: &str, servers: &'s [ServerView]) -> Vec<&'s ServerView> {
        let by_name: Vec<&ServerView> = servers.iter().filter(|s| s.name.to_lowercase() == alias.to_lowercase()).collect();
        if !by_name.is_empty() {
            return by_name;
        }
        servers
            .iter()
            .filter(|s| {
                self.settings
                    .instance_aliases
                    .get(&s.id)
                    .is_some_and(|list| aliases::alias_listed(list, alias))
            })
            .collect()
    }

    /// 별명/인스턴스 이름으로 대상 서버를 찾는다. 찾지 못하면 사용자에게 줄 응답을 Err로.
    fn find_server<'s>(&self, alias: &str, servers: &'s [ServerView]) -> Result<&'s ServerView, Reply> {
        match self.named_servers(alias, servers).as_slice() {
            [one] => return Ok(one),
            [first, ..] => return Err(self.t("errors.multiple_instances", &[("module", &first.module)]).into()),
            [] => {}
        }
        let module = self.aliases.resolve(alias);
        let matched: Vec<&ServerView> = servers.iter().filter(|s| s.module == module || s.name.contains(&module)).collect();
        match matched.as_slice() {
            [one] => Ok(one),
            [] => Err(Reply::unrecognized(
                "INSTANCE_NOT_FOUND",
                self.t("server.not_found", &[("alias", alias), ("resolved", &module)]),
                json!({ "name": alias }),
            )),
            _ => Err(self.t("errors.multiple_instances", &[("module", &module)]).into()),
        }
    }

    fn module_help(&self, alias: &str, servers: &[ServerView]) -> String {
        if let Some(msg) = self.alias_conflict(alias) {
            return msg;
        }
        let prefix = &self.settings.prefix;
        // 인스턴스 이름으로 불렀으면 그 인스턴스의 모듈을 안내한다
        let named = self.named_servers(alias, servers);
        let module_name = named.first().map(|s| s.module.clone()).unwrap_or_else(|| self.aliases.resolve(alias));

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
        if same_module > 1 && named.len() != 1 {
            out.push_str("\n\n");
            out.push_str(&self.t("errors.multiple_instances", &[("module", &module_name)]));
        }
        out
    }

    // ── 모듈 + 명령어 ──

    async fn module_command(&self, alias: &str, cmd_alias: &str, extra: &str, servers: &[ServerView]) -> Reply {
        if let Some(msg) = self.alias_conflict(alias) {
            return msg.into();
        }
        let server = match self.find_server(alias, servers) {
            Ok(s) => s,
            Err(reply) => return reply,
        };
        let Some(module) = self.module(&server.module) else {
            return self.t("server.module_missing", &[("module", &server.module)]).into();
        };
        let command = aliases::resolve_command(Some(module), &self.settings.command_aliases, cmd_alias);

        if self.is_denied(&server.id, &command) {
            tracing::info!("[Chat] 권한 거부: user={:?} server={} command={}", self.req.user_id, server.id, command);
            return self.t("errors.permission_denied", &[]).into();
        }

        match command.as_str() {
            "start" => self.start(server).await.into(),
            "stop" => self.stop(server).await.into(),
            "status" => self.status(server).into(),
            _ => match module.command(&command) {
                Some(field) => self.run_defined(server, module, field, alias, cmd_alias, extra).await.into(),
                // 없는 명령이면 이 모듈에서 쓸 수 있는 명령을 같이 보여준다
                None => Reply::unrecognized(
                    "UNKNOWN_COMMAND",
                    format!("{}\n\n{}", self.t("errors.command_not_found", &[]), self.module_help(alias, servers)),
                    json!({ "command": cmd_alias, "name": server.name }),
                ),
            },
        }
    }

    async fn start(&self, server: &ServerView) -> String {
        let resp = self.api.start(server).await;
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

    /// 정의된 명령 실행 — 해석·검증·라우팅은 데몬 명령 경로(`crate::commands`)가 한다
    async fn run_defined(&self, server: &ServerView, module: &ModuleView, field: &CommandField, alias: &str, cmd_alias: &str, extra: &str) -> String {
        let args = split_quoted(extra);
        let resp = self.api.command(&server.id, &field.name, args).await;
        self.render_result(server, module, field, alias, cmd_alias, resp)
    }

    fn render_result(&self, server: &ServerView, module: &ModuleView, field: &CommandField, alias: &str, cmd_alias: &str, resp: InternalResponse) -> String {
        let failed = !resp.is_success() || resp.body.get("success").and_then(|v| v.as_bool()) == Some(false);
        if !failed {
            let text = format::format_response(resp.body.get("data").unwrap_or(&Value::Null), self.cat);
            return self.t("command.execute_complete", &[("name", &server.name), ("command", &field.name), ("response", &text)]);
        }

        let code = crate::commands::error_code(resp.status.as_u16(), &resp.body);
        let module_text = resp
            .str_field("error_message")
            .map(str::to_string)
            .or_else(|| module.errors.get(&code).cloned());
        if let Some(detail) = resp.str_field("error") {
            tracing::warn!("[Chat] {} {} 실패 ({}): {}", server.name, field.name, resp.status, detail);
        }

        match code.as_str() {
            "missing_argument" => {
                let name = resp.str_field("field").unwrap_or_default();
                let description = field
                    .inputs
                    .iter()
                    .find(|i| i.name == name)
                    .and_then(|i| i.label.clone().filter(|l| !l.is_empty()))
                    .unwrap_or_else(|| name.to_string());
                self.t(
                    "command.missing_required",
                    &[("arg_name", name), ("prefix", &self.settings.prefix), ("alias", alias), ("command", cmd_alias), ("description", &description)],
                )
            }
            "server_not_running" => {
                let error = module_text.unwrap_or_else(|| self.t("server.not_running_default", &[]));
                self.t("server.not_running", &[("name", &server.name), ("error", &error)])
            }
            "invalid_argument" => error_line(&self.t("errors.error_title", &[]), resp.str_field("error").unwrap_or_default()),
            "auth_failed" => format!("❌ {}: {}", self.t("errors.error_title", &[]), module_text.unwrap_or_else(|| self.t("errors.auth_failed", &[]))),
            "not_found" => format!(
                "❌ {}: {}",
                self.t("errors.error_title", &[]),
                resp.str_field("error").map(str::to_string).unwrap_or_else(|| self.t("errors.not_found", &[]))
            ),
            _ => {
                let text = module_text
                    .or_else(|| module.errors.get("internal_server_error").cloned())
                    .unwrap_or_else(|| self.t("errors.internal_server_error", &[]));
                format!("❌ {}: {}", self.t("errors.error_title", &[]), text)
            }
        }
    }
}

/// 정의된 명령의 인자를 나눈다. 공백으로 나누되, 큰따옴표로 시작해 큰따옴표로 끝나는 덩어리는 한 칸이다
/// (`give "Steve Jobs" diamond`). 낱말 중간의 따옴표(`{"text":"a"}`)는 그대로 둔다.
/// 정의에 없는 명령은 원문 그대로 콘솔로 가므로 이 규칙을 쓰지 않는다
pub fn split_quoted(text: &str) -> Vec<String> {
    let mut out = Vec::new();
    let mut rest = text.trim_start();
    while !rest.is_empty() {
        let end = rest.find(char::is_whitespace).unwrap_or(rest.len());
        let word = &rest[..end];
        // 따옴표로 시작한 낱말이면, 공백 앞에 오는 닫는 따옴표(또는 끝)까지를 원문 그대로 한 칸으로
        if word.len() > 1 && word.starts_with('"') && word.ends_with('"') {
            out.push(word[1..word.len() - 1].to_string());
            rest = rest[end..].trim_start();
            continue;
        }
        if word.starts_with('"') {
            let body = &rest[1..];
            let close = body.char_indices().find(|&(i, c)| {
                c == '"' && body[i + 1..].chars().next().is_none_or(char::is_whitespace)
            });
            if let Some((i, _)) = close {
                out.push(body[..i].to_string());
                rest = body[i + 1..].trim_start();
                continue;
            }
        }
        out.push(word.to_string());
        rest = rest[end..].trim_start();
    }
    out
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
