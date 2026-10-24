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
pub mod permissions;
pub mod words;

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
    /// 비어 있으면 응답 언어의 기본 접두사 (`default_prefix`)
    pub prefixes: Vec<String>,
    /// 길드별 설정. 서버·명령을 부르는 낱말은 `nodeSettings[guild].instances` (`words`)
    pub node_settings: Value,
}

impl From<&BotConfig> for ChatSettings {
    fn from(cfg: &BotConfig) -> Self {
        Self {
            prefixes: cfg.prefixes.clone(),
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
    /// 감사 로그에 남길 표시 이름·서버 이름 (권한 판단에는 쓰지 않는다)
    #[serde(default, alias = "userName")]
    pub user_name: Option<String>,
    #[serde(default, alias = "guildName")]
    pub guild_name: Option<String>,
    /// 답을 쓸 언어 (릴레이가 실어 보낸 디스코드 길드 언어). 없으면 데몬 GUI 언어
    #[serde(default)]
    pub locale: Option<String>,
    /// 디스코드(릴레이)에서 온 요청이면 판정에 쓸 출처. None이면 로컬(GUI·CLI)이라 판정하지 않는다.
    /// HTTP 본문으로는 채울 수 없다 (릴레이 경로만 채운다)
    #[serde(skip)]
    pub remote: Option<permissions::Requester>,
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
    /// 요청한 길드의 인스턴스별 낱말 (로컬 요청이면 비어 있다)
    words: HashMap<String, words::InstanceWords>,
}

/// 처리 결과. `text`는 늘 사람이 읽을 문구다. 데몬이 입력을 알아보지 못했으면 `code`가 붙고,
/// 릴레이는 이 코드를 보고 모델 해석기에 정확한 명령을 물을 수 있다
#[derive(Debug, Clone, PartialEq)]
pub struct Reply {
    pub text: String,
    /// `UNKNOWN_COMMAND` | `INSTANCE_NOT_FOUND` | `PERMISSION_DENIED` | `AGREEMENT_REQUIRED`(`args.agreements`). None이면 처리된 응답(성공이든 실행 오류든)
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
    let aliases = ModuleAliases::build(&modules);
    let words = match (&req.remote, req.guild_id.as_deref()) {
        (Some(_), Some(guild)) => words::of_guild(&settings.node_settings, guild),
        _ => HashMap::new(),
    };
    let ctx = Ctx { api, settings, cat, req, modules, aliases, words };

    let text = strip_prefix(req.text.trim(), &ctx.prefixes());
    let args: Vec<&str> = text.split_whitespace().collect();

    if let Err(denied) = ctx.permit(None) {
        return ctx.permission_denied(denied);
    }

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
/// 긴 접두사부터 대소문자 없이 맞춰 본다. 글자로 끝나는 접두사 뒤에는 공백·쉼표·쌍점이 오거나 글이 끝나야 한다
/// (`사바쨩, 안녕`은 접두사, `사바쨩아`는 아니다). 기호로 끝나는 접두사(`!`)는 명령에 바로 붙어도 된다
fn strip_prefix<'t>(text: &'t str, prefixes: &[String]) -> &'t str {
    let mut sorted: Vec<&String> = prefixes.iter().filter(|p| !p.is_empty()).collect();
    sorted.sort_by_key(|p| std::cmp::Reverse(p.chars().count()));
    for prefix in sorted {
        let n = prefix.chars().count();
        if text.chars().count() < n {
            continue;
        }
        let end = text.char_indices().nth(n).map_or(text.len(), |(i, _)| i);
        let (head, rest) = text.split_at(end);
        if head.to_lowercase() != prefix.to_lowercase() {
            continue;
        }
        let symbolic = !prefix.chars().last().is_some_and(char::is_alphanumeric);
        if symbolic || rest.is_empty() || rest.starts_with(|c: char| c.is_whitespace() || c == ',' || c == ':') {
            return rest.trim_start_matches(|c: char| c.is_whitespace() || c == ',' || c == ':');
        }
    }
    text
}

impl<D: DaemonApi> Ctx<'_, D> {
    fn t(&self, key: &str, vars: &[(&str, &str)]) -> String {
        self.cat.t(key, vars)
    }

    /// 맞춰 볼 접두사. 설정이 비어 있으면 응답 언어의 기본값 하나
    fn prefixes(&self) -> Vec<String> {
        match self.settings.prefixes.is_empty() {
            true => vec![self.t("default_prefix", &[])],
            false => self.settings.prefixes.clone(),
        }
    }

    /// 도움말에 보여 줄 접두사 (첫 번째)
    fn prefix(&self) -> String {
        self.prefixes().swap_remove(0)
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

    /// 원격 요청의 권한 판정 (`command`가 None이면 help·list 같은 기본 요청: 금지만 막는다).
    /// 로컬 요청은 PC 주인이므로 판정하지 않는다
    fn permit(&self, command: Option<(&str, &str, &str)>) -> Result<(), permissions::Denied> {
        let Some(who) = &self.req.remote else { return Ok(()) };
        let perms = permissions::NodePermissions::from_node(self.node_settings());
        match command {
            None => perms.check_basic(who),
            Some((instance, command, risk)) => perms.check(who, instance, command, risk),
        }
    }

    fn permission_denied(&self, denied: permissions::Denied) -> Reply {
        tracing::info!("[Chat] 권한 거부: user={:?} need={} level={}", self.req.user_id, denied.need, denied.level);
        Reply::unrecognized("PERMISSION_DENIED", self.t("errors.permission_denied", &[]), json!(denied))
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
        servers.iter().any(|s| s.module == module) || !self.named_servers(alias, servers).is_empty()
    }

    // ── 도움말 / 목록 ──

    async fn help(&self) -> String {
        let prefix = &self.prefix();
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
                // 상태 문구에 아이콘이 들어 있다 (🟢 실행 중)
                let status = self.t(if s.is_running() { "status.running" } else { "status.stopped" }, &[]);
                self.t("list.item", &[("name", &s.name), ("module", &s.module), ("status", &status)])
            })
            .collect();
        format!("{}\n{}", self.t("list.title", &[]), lines.join("\n"))
    }

    fn alias_conflict(&self, alias: &str) -> Option<String> {
        let modules = self.aliases.conflict(alias)?.join(", ");
        Some(self.t("errors.alias_conflict", &[("alias", alias), ("modules", &modules)]))
    }

    /// 인스턴스 이름·id나 그 길드에서 그 서버를 부르는 낱말로 부른 서버들
    fn named_servers<'s>(&self, alias: &str, servers: &'s [ServerView]) -> Vec<&'s ServerView> {
        let by_name: Vec<&ServerView> = servers.iter().filter(|s| words::same(&s.name, alias) || s.id == alias).collect();
        if !by_name.is_empty() {
            return by_name;
        }
        servers
            .iter()
            .filter(|s| self.words.get(&s.id).is_some_and(|w| w.words.iter().any(|word| words::same(word, alias))))
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
        let prefix = &self.prefix();
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
        let command = words::command_for(self.words.get(&server.id), cmd_alias)
            .map(str::to_string)
            .unwrap_or_else(|| aliases::resolve_command(Some(module), cmd_alias));

        // 내장 명령의 위험도: status·players는 safe, start·stop은 caution. 모듈 명령은 선언값 (없으면 caution)
        let risk = match command.as_str() {
            "status" => "safe",
            "players" if module.players => "safe",
            "start" | "stop" => "caution",
            _ => crate::commands::risk_of(module.command(&command).and_then(|f| f.risk.as_deref())),
        };
        if let Err(denied) = self.permit(Some((&server.id, &command, risk))) {
            return self.permission_denied(denied);
        }
        // 챗봇 경로의 danger 명령은 요청자 본인이 확인한 것만 (릴레이 확인 버튼 → `confirmedBy`). 액션 경로는 확인 계획이 같은 일을 한다
        if risk == "danger" && self.req.remote.as_ref().is_some_and(|w| w.via_chatbot && !w.confirmed_by_requester()) {
            return Reply::unrecognized("action_not_allowed", self.t("errors.chatbot_not_allowed", &[]), json!({ "command": command }));
        }

        match command.as_str() {
            "start" => self.start(server).await,
            "stop" => self.stop(server).await.into(),
            "status" => self.status(server).into(),
            "players" if module.players => self.players(server).await.into(),
            // 디스코드·챗봇에 내보내지 않는 명령은 거기서 없는 명령이다
            _ => match module.command(&command).filter(|f| f.remote || self.req.remote.is_none()) {
                Some(field) => self.run_defined(server, module, field, alias, cmd_alias, extra).await.into(),
                // 없는 명령이면 이 모듈에서 쓸 수 있는 명령을 같이 보여준다
                // 릴레이는 해석에 실패하면 자기 문구 뒤에 `args.help`를 붙인다
                None => {
                    let help = self.module_help(alias, servers);
                    Reply::unrecognized(
                        "UNKNOWN_COMMAND",
                        format!("{}\n\n{}", self.t("errors.command_not_found", &[]), help),
                        json!({ "command": cmd_alias, "name": server.name, "help": help }),
                    )
                }
            },
        }
    }

    async fn start(&self, server: &ServerView) -> Reply {
        let resp = self.remote_response(self.api.start(server).await);
        // 약관 동의는 PC에서만 받는다: 릴레이가 원문 링크와 함께 안내하도록 목록을 싣는다
        if resp.str_field("error_code") == Some("agreement_required") {
            let text = resp.str_field("error").map(str::to_string).unwrap_or_else(|| self.t("errors.error_title", &[]));
            return Reply::unrecognized("AGREEMENT_REQUIRED", format!("📜 {}", text), json!({ "agreements": resp.body.get("agreements") }));
        }
        let moved = self.port_moved_lines(&resp);
        let text = self.start_text(server, resp);
        if moved.is_empty() { text.into() } else { format!("{}\n{}", text, moved.join("\n")).into() }
    }

    /// 시작 전에 예약 범위를 피해 옮긴 포트 (`port_moved`, 시작이 실패해도 옮긴 값은 저장돼 있다)
    fn port_moved_lines(&self, resp: &InternalResponse) -> Vec<String> {
        let moves = resp.body.get("port_moved").and_then(Value::as_array).cloned().unwrap_or_default();
        moves
            .iter()
            .filter_map(|m| Some(self.t("server.port_moved", &[("from", &m.get("from")?.to_string()), ("to", &m.get("to")?.to_string())])))
            .collect()
    }

    fn start_text(&self, server: &ServerView, resp: InternalResponse) -> String {
        if !resp.is_success() {
            let detail = resp.str_field("error").unwrap_or_default();
            tracing::warn!("[Chat] 시작 실패 {}: {}", server.name, detail);
            // 켜기 전 점검(preflight_failed)의 문제 문구에는 경로가 들어 있을 수 있어 PC 밖으로 내지 않는다.
            // 점검 코드와 모듈이 준 해결 방법(경로 없는 문구)만 보인다
            if resp.str_field("error_code") == Some("preflight_failed") {
                let code = resp.str_field("issue_code").unwrap_or("preflight_failed");
                let line = format!("❌ {}", self.t("errors.preflight_failed", &[("code", code)]));
                return match resp.str_field("solution").filter(|s| !s.trim().is_empty()) {
                    Some(solution) => format!("{}\n💡 {}", line, solution),
                    None => line,
                };
            }
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
        let resp = self.remote_response(self.api.stop(server).await);
        let failed = !resp.is_success() || resp.body.get("success").and_then(|v| v.as_bool()) == Some(false);
        // 저장 후 종료를 요청할 길이 없어 서버를 그대로 두었다 (MK10). 강제 종료는 이 PC에서만 고른다
        if resp.str_field("error_code") == Some("graceful_stop_unavailable") {
            return self.t("server.graceful_stop_unavailable", &[("name", &server.name)]);
        }
        if failed {
            let detail = resp.str_field("error").or(resp.str_field("message")).unwrap_or_default();
            tracing::warn!("[Chat] 정지 실패 {}: {}", server.name, detail);
            return error_line(&self.t("errors.error_title", &[]), detail);
        }
        self.t("server.stop_complete", &[("name", &server.name)])
    }

    /// 접속자 조회 (`[status] players`). 이름은 액션 요약과 같은 모양
    async fn players(&self, server: &ServerView) -> String {
        let resp = self.remote_response(self.api.players(server).await);
        let failed = !resp.is_success() || resp.body.get("success").and_then(Value::as_bool) == Some(false);
        if failed {
            let code = crate::commands::error_code(resp.status.as_u16(), &resp.body);
            return self.failed_line(server, &self.t("command.players", &[]), &reason(self.cat, &code, &resp.body));
        }
        let summary = crate::actions::summary::text("server.players", &resp.body, self.cat).unwrap_or_default();
        format!("**{}**\n{}", server.name, summary)
    }

    fn status(&self, server: &ServerView) -> String {
        let status = if server.is_running() { self.t("status.running", &[]) } else { self.t("status.stopped", &[]) };
        let pid = server.pid.map(|p| format!("PID: {}", p)).unwrap_or_default();
        self.t("server.status_check", &[("name", &server.name), ("status", &status), ("pid_info", &pid)]).trim_end().to_string()
    }

    /// 정의된 명령 실행 — 해석·검증·라우팅은 데몬 명령 경로(`crate::commands`)가 한다
    async fn run_defined(&self, server: &ServerView, module: &ModuleView, field: &CommandField, alias: &str, cmd_alias: &str, extra: &str) -> String {
        let args = split_quoted(extra);
        let resp = self.remote_response(self.api.command(&server.id, &field.name, args).await);
        self.render_result(server, module, field, alias, cmd_alias, resp)
    }

    fn remote_response(&self, mut resp: InternalResponse) -> InternalResponse {
        if self.req.remote.is_some() {
            crate::actions::strip_local_details(&mut resp.body);
        }
        resp
    }

    /// 명령의 보일 이름: 모듈 locales `commands.<이름>.label` → module.toml label → 이름
    fn command_label(&self, module: &ModuleView, field: &CommandField) -> String {
        module
            .text(self.cat.language(), &format!("/commands/{}/label", field.name))
            .unwrap_or_else(|| if field.label.is_empty() { field.name.clone() } else { field.label.clone() })
    }

    fn done_line(&self, server: &ServerView, text: &str) -> String {
        self.t("command.done", &[("name", &server.name), ("label", text)])
    }

    fn failed_line(&self, server: &ServerView, label: &str, reason: &str) -> String {
        self.t("command.failed", &[("name", &server.name), ("label", label), ("reason", reason)])
    }

    /// 성공한 명령의 답: 성공과 명령 이름만. 콘솔 원문은 싣지 않고, 모듈이 실패 모양·답 모양을 정했으면 그것을 따른다.
    /// 모듈·익스텐션이 사람에게 쓴 `message`와 REST 조회 결과(값 줄)는 아래에 붙인다
    fn success_text(&self, server: &ServerView, module: &ModuleView, field: &CommandField, resp: &InternalResponse) -> String {
        let label = self.command_label(module, field);
        let data = resp.body.get("data").filter(|d| !d.is_null());
        let console = data.and_then(|d| d.get("response")).and_then(Value::as_str).unwrap_or("");
        for pattern in &module.failure_patterns {
            if let Some(caps) = regex::Regex::new(pattern).ok().and_then(|re| re.captures(console)) {
                let reason = caps.name("reason").map(|m| m.as_str().trim().to_string()).unwrap_or_else(|| self.t("reasons.rejected", &[]));
                return self.failed_line(server, &label, &reason);
            }
        }
        if let Some(caps) = field.reply_pattern.as_deref().and_then(|p| regex::Regex::new(p).ok()).and_then(|re| re.captures(console)) {
            if let Some(template) = module.text(self.cat.language(), &format!("/commands/{}/done", field.name)) {
                let re = regex::Regex::new(field.reply_pattern.as_deref().unwrap_or_default()).expect("compiled above");
                let vars: Vec<(&str, &str)> = re.capture_names().flatten().filter_map(|n| Some((n, caps.name(n)?.as_str()))).collect();
                return self.done_line(server, &i18n::interpolate(&template, &vars));
            }
        }
        // 모듈·익스텐션 명령은 답을 `message`로 준다 (stdin의 message는 내부 문구)
        let message = match resp.str_field("protocol") {
            Some("module" | "extension") => resp.body.get("message").filter(|m| m.as_str().is_some_and(|t| !t.trim().is_empty())),
            _ => None,
        };
        let detail = match data.or(message) {
            // 콘솔 명령의 답은 게임 서버 로그 줄이라 보이지 않는다
            Some(d) if d.get("response").is_some() => None,
            Some(d) => Some(format::format_response(d, self.cat)).filter(|t| !t.trim().is_empty() && *t != self.t("responses.command_complete", &[])),
            None => None,
        };
        let head = self.done_line(server, &label);
        match detail {
            Some(detail) => format!("{}\n{}", head, detail),
            None => head,
        }
    }

    /// 틀린 인자의 이유 (`rule`·`params`, 데몬 검사가 준다). 칸 이름은 모듈 locales `commands.<명령>.inputs.<칸>.label`
    fn invalid_text(&self, module: &ModuleView, field: &CommandField, resp: &InternalResponse) -> Option<String> {
        let rule = resp.str_field("rule")?;
        let params = resp.body.get("params").cloned().unwrap_or(Value::Null);
        let name = resp.str_field("field").unwrap_or_default();
        let label = module
            .text(self.cat.language(), &format!("/commands/{}/inputs/{}/label", field.name, name))
            .or_else(|| field.inputs.iter().find(|i| i.name == name).and_then(|i| i.label.clone()))
            .unwrap_or_else(|| name.to_string());
        let num = |key: &str| params.get(key).and_then(Value::as_f64).map(|n| n.to_string()).unwrap_or_else(|| "-".into());
        let options = params.get("options").and_then(Value::as_array).map(|o| o.iter().filter_map(Value::as_str).collect::<Vec<_>>().join(", ")).unwrap_or_default();
        let value = params.get("value").and_then(Value::as_str).unwrap_or_default();
        let key = format!("command.invalid.{}", rule);
        let text = self.t(&key, &[("label", &label), ("value", value), ("min", &num("min")), ("max", &num("max")), ("options", &options)]);
        (text != key).then(|| format!("❌ {}", text))
    }

    fn render_result(&self, server: &ServerView, module: &ModuleView, field: &CommandField, alias: &str, cmd_alias: &str, resp: InternalResponse) -> String {
        let failed = !resp.is_success() || resp.body.get("success").and_then(|v| v.as_bool()) == Some(false);
        if !failed {
            return self.success_text(server, module, field, &resp);
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
                    &[("arg_name", name), ("prefix", &self.prefix()), ("alias", alias), ("command", cmd_alias), ("description", &description)],
                )
            }
            "server_not_running" => {
                let error = module_text.unwrap_or_else(|| self.t("server.not_running_default", &[]));
                self.t("server.not_running", &[("name", &server.name), ("error", &error)])
            }
            "no_command_path" => format!("❌ {}", self.t("errors.no_command_path", &[("name", &server.name)])),
            "invalid_argument" => self
                .invalid_text(module, field, &resp)
                .unwrap_or_else(|| error_line(&self.t("errors.error_title", &[]), resp.str_field("error").unwrap_or_default())),
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

/// 실패 코드의 짧은 이유 (응답 언어). 모르는 코드면 일반 문구. 액션 결과(`reason`)도 이것을 쓴다
pub fn reason(cat: &Catalog, code: &str, body: &Value) -> String {
    known_reason(cat, code, body).unwrap_or_else(|| cat.t("reasons.failed", &[]))
}

/// 문구가 있는 실패 코드만
pub fn known_reason(cat: &Catalog, code: &str, body: &Value) -> Option<String> {
    let file = body.get("file").and_then(Value::as_str).unwrap_or_default();
    let key = format!("reasons.{}", code);
    let text = cat.t(&key, &[("file", file)]);
    (text != key).then_some(text)
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
