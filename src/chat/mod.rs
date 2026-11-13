//! 채팅 명령 처리기
//!
//! 디스코드 등 채팅에서 들어온 텍스트 명령("팰월드 시작", "mc players")을 해석해
//! 데몬 기능으로 실행하고, 사용자에게 돌려줄 한 줄 응답을 만듭니다.
//! 릴레이 에이전트(`relay_agent`)의 문장 명령 계획·실행이 이 처리기를 씁니다. 답 문장은 릴레이가 만든다 (`Text`).
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
pub mod text;
pub mod words;

use crate::config_store::BotConfig;
use crate::ipc::internal::InternalResponse;
use crate::module_loader::CommandField;
use aliases::ModuleAliases;
use api::{DaemonApi, ModuleView, ServerView};
use i18n::Catalog;
use permissions::Denied;
pub use text::Text;
use serde_json::{json, Value};
use std::collections::HashMap;

/// 처리기가 쓰는 봇 설정 일부
#[derive(Debug, Clone, Default)]
pub struct ChatSettings {
    /// 호스트 접두사 (0.1.12 전 값). 길드 값(`nodeSettings[guild].discord.prefixes`)이 먼저다. 둘 다 비면 응답 언어의 기본 접두사
    pub prefixes: Vec<String>,
    /// 길드별 설정
    pub node_settings: Value,
    /// 인스턴스 id → 서버·명령을 부르는 낱말 (`words`, 어느 길드에서나 같다)
    pub server_words: HashMap<String, words::InstanceWords>,
}

impl From<&BotConfig> for ChatSettings {
    fn from(cfg: &BotConfig) -> Self {
        Self {
            prefixes: cfg.prefixes.clone(),
            node_settings: cfg.node_settings.clone(),
            server_words: cfg.server_words.clone(),
        }
    }
}

/// 채팅 명령 한 건 (릴레이 문장 계획·문장 `check`가 만든다, `actions::chat_request`)
#[derive(Debug, Clone, Default)]
pub struct ChatRequest {
    pub text: String,
    /// 디스코드 길드 ID — 노드별 허용 인스턴스/멤버 권한 조회 키
    pub guild_id: Option<String>,
    /// 요청한 사용자 ID — 멤버 권한 조회 키
    pub user_id: Option<String>,
    /// 디스코드(릴레이)에서 온 요청이면 판정에 쓸 출처. None이면 판정하지 않는다
    pub remote: Option<permissions::Requester>,
}

/// 최상위 명령 (모듈 이름보다 먼저 확인)
enum TopLevel {
    Help,
    List,
}

impl TopLevel {
    /// `help`·`list`, 그리고 답하는 언어의 말 (`top_level_words.<help|list>`, 쉼표로 나눈 목록. 영어 문구로 대신하지 않는다)
    fn parse(word: &str, cat: &Catalog) -> Option<Self> {
        let says = |name: &str| {
            let key = format!("top_level_words.{name}");
            let local = cat.t(&key, &[]);
            words::same(word, name) || (local != key && local.split(words::is_comma).any(|w| words::same(w.trim(), word)))
        };
        if says("help") {
            Some(Self::Help)
        } else if says("list") {
            Some(Self::List)
        } else {
            None
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
    /// 인스턴스별 낱말
    words: &'a HashMap<String, words::InstanceWords>,
}

/// 처리 결과. `text`는 늘 사람이 읽을 답이다 (문장은 릴레이가 만든다, `Text`). 데몬이 입력을 알아보지 못했으면 `code`가 붙고,
/// 릴레이는 이 코드를 보고 모델 해석기에 정확한 명령을 물을 수 있다
#[derive(Debug, Clone, PartialEq)]
pub struct Reply {
    pub text: Text,
    /// `UNKNOWN_COMMAND` | `INSTANCE_NOT_FOUND` | `PERMISSION_DENIED` | `AGREEMENT_REQUIRED`(`args.agreements`). None이면 처리된 응답(성공이든 실행 오류든)
    pub code: Option<&'static str>,
    pub args: Value,
}

impl Reply {
    fn unrecognized(code: &'static str, text: Text, args: Value) -> Self {
        Self { text, code: Some(code), args }
    }
}

impl From<Text> for Reply {
    fn from(text: Text) -> Self {
        Self { text, code: None, args: Value::Null }
    }
}

/// 해석 결과: 실행 없이 나온 답(도움말·목록·안내·거절), 또는 실행할 명령
#[derive(Debug)]
pub enum Interpreted {
    Reply(Reply),
    Run(Box<Target>),
}

/// 실행할 것으로 알아본 명령. 문장 계획(`actions::plan_text`)이 실행 때까지 들고 있는다
#[derive(Debug, Clone)]
pub struct Target {
    server: ServerView,
    exec: Exec,
    /// 사용자가 부른 서버·명령 낱말과 나머지 인자 원문 (답 문구와 인자 나누기에 쓴다)
    alias: String,
    cmd_alias: String,
    extra: String,
    risk: &'static str,
}

#[derive(Debug, Clone)]
enum Exec {
    Start,
    Stop,
    Status,
    Players,
    Defined(Box<CommandField>),
}

impl Target {
    pub fn instance(&self) -> &str {
        &self.server.id
    }

    pub fn risk(&self) -> &'static str {
        self.risk
    }

    /// 같은 일을 하는 액션 id (`server.start`·`server.command:<이름>` …)
    pub fn action_id(&self) -> String {
        match &self.exec {
            Exec::Start => "server.start".into(),
            Exec::Stop => "server.stop".into(),
            Exec::Status => "server.status".into(),
            Exec::Players => "server.players".into(),
            Exec::Defined(field) => format!("{}{}", crate::actions::COMMAND_PREFIX, field.name),
        }
    }
}

/// 실행한 명령의 답. `ok`는 명령이 해낸 것인지, `already_running`은 시작했는데 이미 켜져 있었는지
#[derive(Debug, Clone)]
pub struct Done {
    pub reply: Reply,
    pub ok: bool,
    pub already_running: bool,
}

impl Done {
    fn new(reply: impl Into<Reply>, ok: bool) -> Self {
        Self { reply: reply.into(), ok, already_running: false }
    }
}

/// 텍스트 명령을 처리하고 응답을 돌려줍니다. 실패해도 항상 응답을 만듭니다.
pub async fn process<D: DaemonApi>(api: &D, settings: &ChatSettings, cat: &Catalog, req: &ChatRequest) -> Reply {
    let ctx = Ctx::new(api, settings, cat, req).await;
    match ctx.interpret().await {
        Interpreted::Reply(reply) => reply,
        Interpreted::Run(target) => ctx.execute(&target).await.reply,
    }
}

/// 실행하지 않고 해석만 한다 (권한·위험 명령 판정 포함)
pub async fn interpret<D: DaemonApi>(api: &D, settings: &ChatSettings, cat: &Catalog, req: &ChatRequest) -> Interpreted {
    Ctx::new(api, settings, cat, req).await.interpret().await
}

/// 해석한 명령을 실행한다
pub async fn execute<D: DaemonApi>(api: &D, settings: &ChatSettings, cat: &Catalog, req: &ChatRequest, target: &Target) -> Done {
    Ctx::new(api, settings, cat, req).await.execute(target).await
}

/// 앞의 낱말 `n`개를 건너뛴 나머지 원문 (따옴표 안의 공백을 그대로 두려고 나누기 전 문자열을 쓴다)
#[cfg(test)]
fn skip_words(text: &str, n: usize) -> &str {
    let mut rest = text.trim_start();
    for _ in 0..n {
        let end = rest.find(char::is_whitespace).unwrap_or(rest.len());
        rest = rest[end..].trim_start();
    }
    rest
}

/// 글의 첫 낱말과 그 낱말이 끝난 자리 (바이트)
fn first_word(text: &str) -> Option<(&str, usize)> {
    let start = text.find(|c: char| !c.is_whitespace())?;
    let end = text[start..].find(char::is_whitespace).map_or(text.len(), |i| start + i);
    Some((&text[start..end], end))
}

/// 글 앞이 이 말과 낱말 단위로 맞으면 그 말이 끝난 자리 (바이트). 대소문자·공백 칸 수는 보지 않는다
fn phrase_end(text: &str, phrase: &str) -> Option<usize> {
    let mut rest_at = 0;
    let mut matched = false;
    for want in phrase.split_whitespace() {
        let (word, end) = first_word(&text[rest_at..])?;
        if !words::same(word, want) {
            return None;
        }
        rest_at += end;
        matched = true;
    }
    matched.then_some(rest_at)
}

/// 글 앞에서 가장 길게(낱말 수) 맞는 말과 그 말이 끝난 자리. 길이가 같으면 먼저 온 말 (부르는 쪽이 우선순위대로 넣는다)
fn longest_phrase<'p, T>(text: &str, phrases: impl IntoIterator<Item = (&'p str, T)>) -> Option<(&'p str, usize, T)> {
    let mut best: Option<(&str, usize, T, usize)> = None;
    for (phrase, tag) in phrases {
        let Some(end) = phrase_end(text, phrase) else { continue };
        let n = phrase.split_whitespace().count();
        if best.as_ref().is_none_or(|b| n > b.3) {
            best = Some((phrase, end, tag, n));
        }
    }
    best.map(|(p, end, tag, _)| (p, end, tag))
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

impl<'a, D: DaemonApi> Ctx<'a, D> {
    async fn new(api: &'a D, settings: &'a ChatSettings, cat: &'a Catalog, req: &'a ChatRequest) -> Self {
        // 별명·게임명은 답하는 언어의 것만 받는다 (module.toml의 언어 없는 낱말은 늘)
        let modules: Vec<ModuleView> = api.modules().await.into_iter().map(|m| m.in_language(Some(cat.language()))).collect();
        let aliases = ModuleAliases::build(&modules);
        Self { api, settings, cat, req, modules, aliases, words: &settings.server_words }
    }

    async fn interpret(&self) -> Interpreted {
        let text = strip_prefix(self.req.text.trim(), &self.prefixes());
        let reply = |r: Reply| Interpreted::Reply(r);

        if let Err(denied) = self.permit(None) {
            return reply(self.permission_denied(denied));
        }

        let Some((first, first_end)) = first_word(text) else {
            return reply(self.help().await.into());
        };
        match TopLevel::parse(first, self.cat) {
            Some(TopLevel::Help) => return reply(self.help().await.into()),
            Some(TopLevel::List) => return reply(self.list().await.into()),
            None => {}
        }

        let servers = match self.servers().await {
            Ok(s) => s,
            Err(e) => {
                tracing::warn!("[Chat] 서버 목록 조회 실패: {}", e);
                return reply(Text::seq(["❌ ".into(), self.t("messages.command_error", &[])]).into());
            }
        };

        // 서버를 부르는 말은 띄어 쓸 수 있다: 앞에서부터 가장 길게 맞는 말, 없으면 첫 낱말 (사용자 지정 2026-11-06)
        let (alias, alias_end) = longest_phrase(text, self.server_phrases(&servers).into_iter().map(|p| (p, ())))
            .map_or((first, first_end), |(p, end, ())| (p, end));
        let rest = text[alias_end..].trim_start();
        let module_known = self.aliases.is_known(alias);
        let mounted = self.is_mounted(alias, &servers);
        if rest.is_empty() {
            // 모듈 별명이나 인스턴스 이름이 실제 서버로 있어야 명령어 안내
            if !mounted {
                let command = text.trim();
                return reply(Reply::unrecognized("UNKNOWN_COMMAND", self.t("errors.unknown_input", &[]), json!({ "command": command })));
            }
            return reply(self.module_help(alias, &servers).into());
        }
        if module_known && !mounted {
            return reply(Reply::unrecognized("INSTANCE_NOT_FOUND", self.t("errors.unknown_input", &[]), json!({ "name": alias })));
        }
        let (cmd, cmd_end) = self.command_phrase(alias, rest, &servers);
        match self.module_command(alias, cmd, rest[cmd_end..].trim_start(), &servers) {
            Ok(target) => Interpreted::Run(Box::new(target)),
            Err(r) => reply(r),
        }
    }

    /// 서버를 부르는 말: 인스턴스 이름·id·서버 낱말, 모듈 이름·게임명·모듈 별명 (해석 순서는 `find_server`가 정한다)
    fn server_phrases<'x>(&'x self, servers: &'x [ServerView]) -> Vec<&'x str> {
        let mut out: Vec<&str> = Vec::new();
        for s in servers {
            out.push(&s.name);
            out.push(&s.id);
            out.extend(self.words.get(&s.id).into_iter().flat_map(|w| w.words.iter().map(String::as_str)));
        }
        out.extend(self.aliases.all());
        out
    }

    /// 나머지 글 앞에서 이 서버의 명령을 부르는 가장 긴 말과 그 말이 끝난 자리. 맞는 말이 없으면 첫 낱말 (모르는 명령 안내로 간다).
    /// 말은 그 서버 낱말 → 명령 이름·모듈 별명 → 내장 명령의 언어별 이름 → 화면에 보이는 명령 이름(언어별 이름표) 순으로 넣는다
    fn command_phrase<'x>(&'x self, alias: &str, rest: &'x str, servers: &'x [ServerView]) -> (&'x str, usize) {
        let fallback = || first_word(rest).unwrap_or((rest, rest.len()));
        let named = self.named_servers(alias, servers);
        let module_name = named.first().map(|s| s.module.clone()).unwrap_or_else(|| self.aliases.resolve(alias));
        let mut phrases: Vec<String> = Vec::new();
        for s in named.iter().copied().chain(servers.iter().filter(|s| named.is_empty() && s.module == module_name)) {
            phrases.extend(self.words.get(&s.id).into_iter().flat_map(|w| w.commands.values().flatten().cloned()));
        }
        let module = self.module(&module_name);
        if let Some(m) = module {
            phrases.extend(m.command_aliases.iter().flat_map(|(_, list)| list.iter().cloned()));
            phrases.extend(m.commands.iter().map(|c| c.name.clone()));
        }
        phrases.extend(aliases::BUILTIN_COMMANDS.iter().map(|b| b.to_string()));
        phrases.extend(self.builtin_names().into_iter().map(|(name, _)| name));
        if let Some(m) = module {
            phrases.extend(m.commands.iter().filter_map(|c| self.calling_label(m, c)));
        }
        match longest_phrase(rest, phrases.iter().map(|p| (p.as_str(), ()))) {
            Some((_, end, ())) => (rest[..end].trim(), end),
            None => fallback(),
        }
    }

    async fn execute(&self, target: &Target) -> Done {
        let server = &target.server;
        let Some(module) = self.module(&server.module) else {
            return Done::new(self.t("server.module_missing", &[("module", &server.module)]), false);
        };
        match &target.exec {
            Exec::Start => self.start(server).await,
            Exec::Stop => self.stop(server).await,
            Exec::Status => Done::new(self.status(server).await, true),
            Exec::Players => self.players(server).await,
            Exec::Defined(field) => self.run_defined(server, module, field, &target.alias, &target.cmd_alias, &target.extra).await,
        }
    }

    /// 봇 답장 문구 (문장은 릴레이가 만든다)
    fn t(&self, key: &str, vars: &[(&str, &str)]) -> Text {
        Text::key_with(key, vars)
    }

    /// 데몬이 아는 말 (`locales/<lang>/daemon.json`: 기본 접두사·내장 명령 이름처럼 입력을 알아보는 데 쓰는 것)
    fn word(&self, key: &str) -> String {
        self.cat.t(key, &[])
    }

    /// 맞춰 볼 접두사. 설정이 비어 있으면 응답 언어의 기본값 하나
    fn prefixes(&self) -> Vec<String> {
        let own = crate::service::discord::guild_prefixes(&self.settings.node_settings, self.req.guild_id.as_deref(), &self.settings.prefixes);
        match own.is_empty() {
            true => vec![self.word("default_prefix")],
            false => own,
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

    fn permission_denied(&self, denied: Denied) -> Reply {
        tracing::info!("[Chat] 권한 거부: user={:?} need={} level={}", self.req.user_id, denied.need, denied.level);
        // 전용 멤버 서버의 명단 밖 사람이면 그 안내를 한다
        let key = if denied.members_only { "errors.members_only" } else { "errors.permission_denied" };
        Reply::unrecognized("PERMISSION_DENIED", self.t(key, &[]), json!(denied))
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

    /// 도움말: 쓰는 법, 이 길드가 쓰는 서버와 그 서버를 부르는 말, 슬래시 명령 안내
    async fn help(&self) -> Text {
        let prefix = &self.prefix();
        let servers = self.servers().await.unwrap_or_else(|e| {
            tracing::warn!("[Chat] 도움말용 서버 목록 실패: {}", e);
            Vec::new()
        });

        let mut out: Vec<Text> = vec![format!("📖 **{}**\n", prefix).into(), self.t("help.usage_format", &[("prefix", prefix)]), "\n".into()];
        // 문장 명령으로 못 하는 일(권한·언어·웹 등)은 슬래시 명령으로 안내한다
        let hint = self.t("help.slash_hint", &[]);
        if servers.is_empty() {
            out.extend(["\n".into(), self.t("list.empty", &[]), "\n\n".into(), hint]);
            return Text::seq(out);
        }
        out.extend(["\n".into(), self.t("help.modules_header", &[]), "\n".into()]);
        for s in &servers {
            // 이 서버를 부르는 말: 기본 호출명(이름 다음) + 사용자가 정한 낱말
            let mut calls: Vec<String> = words::default_words(s, &servers, &self.modules, self.words).into_iter().skip(1).collect();
            for w in self.words.get(&s.id).into_iter().flat_map(|w| w.words.iter()) {
                if !calls.iter().any(|c| words::same(c, w)) {
                    calls.push(w.clone());
                }
            }
            out.push(match calls.is_empty() {
                true => format!("• **{}**\n", s.name).into(),
                false => format!("• **{}** ({})\n", s.name, calls.join(", ")).into(),
            });
        }
        out.extend(["\n".into(), hint]);
        Text::seq(out)
    }

    async fn list(&self) -> Text {
        let servers = match self.servers().await {
            Ok(s) => s,
            Err(e) => {
                tracing::warn!("[Chat] 목록 조회 실패: {}", e);
                return Text::seq(["❌ ".into(), self.t("messages.command_error", &[])]);
            }
        };
        if servers.is_empty() {
            return self.t("list.empty", &[]);
        }
        let lines = servers.iter().map(|s| {
            // 상태 문구에 아이콘이 들어 있다 (🟢 실행 중)
            let status = self.t(if s.is_running() { "status.running" } else { "status.stopped" }, &[]);
            self.t("list.item", &[("name", &s.name), ("module", &s.module)]).var("status", status)
        });
        Text::lines(std::iter::once(self.t("list.title", &[])).chain(lines))
    }

    fn alias_conflict(&self, alias: &str) -> Option<Text> {
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
    /// bool은 그 서버만 가리키는 말(인스턴스 이름·id·그 서버 낱말)로 찾았는지다. 모듈 이름·게임명·모듈 별명이면 false
    fn find_server<'s>(&self, alias: &str, servers: &'s [ServerView]) -> Result<(&'s ServerView, bool), Reply> {
        match self.named_servers(alias, servers).as_slice() {
            [one] => return Ok((one, true)),
            [first, ..] => return Err(self.t("errors.multiple_instances", &[("module", &first.module)]).into()),
            [] => {}
        }
        let module = self.aliases.resolve(alias);
        // 이름에 모듈 이름이 든 서버는 모듈을 부르는 말일 때만 찾는다: 모르는 말이 이름의 일부라고 그 서버를 부르지 않는다
        let known = self.aliases.is_known(alias);
        let matched: Vec<&ServerView> = servers.iter().filter(|s| s.module == module || (known && s.name.contains(&module))).collect();
        match matched.as_slice() {
            [one] => Ok((one, false)),
            [] => Err(Reply::unrecognized(
                "INSTANCE_NOT_FOUND",
                self.t("server.not_found", &[("alias", alias), ("resolved", &module)]),
                json!({ "name": alias }),
            )),
            _ => Err(self.t("errors.multiple_instances", &[("module", &module)]).into()),
        }
    }

    fn module_help(&self, alias: &str, servers: &[ServerView]) -> Text {
        // 인스턴스 이름·id·서버 낱말이 맞으면 별명 충돌은 보지 않는다 (이름이 별명보다 먼저)
        if let Some(msg) = self.alias_conflict(alias).filter(|_| self.named_servers(alias, servers).is_empty()) {
            return msg;
        }
        let prefix = &self.prefix();
        // 인스턴스 이름으로 불렀으면 그 인스턴스의 모듈을 안내한다
        let named = self.named_servers(alias, servers);
        let module_name = named.first().map(|s| s.module.clone()).unwrap_or_else(|| self.aliases.resolve(alias));
        let module = self.module(&module_name);
        let same_module: Vec<&ServerView> = servers.iter().filter(|s| s.module == module_name).collect();
        // 제목은 서버 이름 (한 서버를 가리킬 때), 아니면 게임 이름
        let title = match (named.as_slice(), same_module.as_slice()) {
            ([one], _) | ([], [one]) => one.name.clone(),
            _ => module.and_then(|m| m.text(self.cat.language(), "/module/display_name").or_else(|| m.display_name.clone())).unwrap_or_else(|| module_name.clone()),
        };

        let mut out: Vec<Text> = vec![self.t("modules.enter_command", &[]), "\n\n".into(), self.t("help.module_title", &[("module", &title)]), "\n".into()];
        for (cmd, key) in [("start", "modules.help_start"), ("stop", "modules.help_stop"), ("status", "modules.help_status")] {
            let word = self.builtin_word(cmd);
            out.extend([format!("• `{} {} {}` - ", prefix, alias, word).into(), self.t(key, &[]), "\n".into()]);
        }
        if let Some(module) = module {
            // 디스코드·챗봇에 내보내지 않는 명령은 원격 도움말에도 없다. 내장 명령과 같은 이름(stop 등)은 위에서 보였다
            let shown = module
                .commands
                .iter()
                .filter(|c| c.remote || self.req.remote.is_none())
                .filter(|c| !aliases::BUILTIN_COMMANDS.iter().any(|b| b.eq_ignore_ascii_case(&c.name)));
            for c in shown {
                let lang = self.cat.language();
                let inputs: Vec<String> = c
                    .inputs
                    .iter()
                    .map(|i| {
                        let label = module
                            .text(lang, &format!("/commands/{}/inputs/{}/label", c.name, i.name))
                            .or_else(|| i.label.clone().filter(|l| !l.is_empty()))
                            .unwrap_or_else(|| i.name.clone());
                        if i.required.unwrap_or(false) { format!("<{}>", label) } else { format!("[{}]", label) }
                    })
                    .collect();
                let word = self.command_word(module, c);
                let usage = if inputs.is_empty() { word.clone() } else { format!("{} {}", word, inputs.join(" ")) };
                let about = module
                    .text(lang, &format!("/commands/{}/description", c.name))
                    .or_else(|| c.description.clone().filter(|d| !d.is_empty()))
                    .unwrap_or_else(|| self.command_label(module, c));
                out.push(format!("• `{} {} {}` - {}\n", prefix, alias, usage, about).into());
            }
        }
        if same_module.len() > 1 && named.len() != 1 {
            out.extend(["\n\n".into(), self.t("errors.multiple_instances", &[("module", &module_name)])]);
        }
        Text::seq(out)
    }

    /// 도움말에 보일 내장 명령의 말: 응답 언어의 이름 (`actions.server.<명령>.name`, 영문이면 소문자)
    fn builtin_word(&self, cmd: &str) -> String {
        let key = format!("actions.server.{cmd}.name");
        match self.word(&key) {
            name if name == key => cmd.to_string(),
            name if name.is_ascii() => name.to_lowercase(),
            name => name,
        }
    }

    /// 도움말에 보일 명령의 말: 응답 언어의 명령 이름표가 그 명령을 부르면 그것, 아니면 명령 이름
    fn command_word(&self, module: &ModuleView, field: &CommandField) -> String {
        let label = self.command_label(module, field);
        let calls_it = aliases::find_command(Some(module), &label).is_none_or(|c| c == field.name);
        if !label.is_empty() && calls_it && !label.contains(words::is_comma) {
            return if label.is_ascii() { label.to_lowercase() } else { label };
        }
        field.name.clone()
    }

    // ── 모듈 + 명령어 ──

    /// 서버와 명령을 찾고 권한·위험 명령 규칙을 본다. 실행할 것이 아니면 답을 Err로
    fn module_command(&self, alias: &str, cmd_alias: &str, extra: &str, servers: &[ServerView]) -> Result<Target, Reply> {
        if let Some(msg) = self.alias_conflict(alias).filter(|_| self.named_servers(alias, servers).is_empty()) {
            return Err(msg.into());
        }
        let (server, by_name) = self.find_server(alias, servers)?;
        let Some(module) = self.module(&server.module) else {
            return Err(self.t("server.module_missing", &[("module", &server.module)]).into());
        };
        // 그 서버 낱말 → 명령 이름·모듈 별명 → 내장 명령의 언어별 이름 → 화면에 보이는 명령 이름 (`command_phrase`와 같은 순서)
        let command = words::command_for(self.words.get(&server.id), cmd_alias)
            .map(str::to_string)
            .or_else(|| aliases::find_command(Some(module), cmd_alias))
            .or_else(|| self.builtin_names().into_iter().find(|(n, _)| words::same(n, cmd_alias)).map(|(_, b)| b.to_string()))
            .or_else(|| module.commands.iter().find(|c| self.calling_label(module, c).is_some_and(|l| words::same(&l, cmd_alias))).map(|c| c.name.clone()))
            .unwrap_or_else(|| cmd_alias.to_string());

        // 내장 명령의 위험도: status·players·start·stop은 safe (start·stop은 유저 이상이 한다).
        // 모듈 명령은 선언값 (없으면 caution)
        let risk = match command.as_str() {
            "status" | "start" | "stop" => "safe",
            "players" if module.players => "safe",
            _ => crate::commands::risk_of(module.command(&command).and_then(|f| f.risk.as_deref())),
        };
        // 전용 멤버 서버는 길드 레벨과 따로 논다: 명단에 디스코드를 이은 사람만 모든 명령을 하고, 나머지는 거절한다.
        // 말하는 사바쨩을 거친 요청은 명단에 있어도 챗봇 상한(레벨 판정)을 받는다
        let gate = match &self.req.remote {
            Some(r) => crate::controllers::gate_for(server.access, &server.controllers, r.user_id.as_deref(), r.via_chatbot),
            None => crate::controllers::Gate::Allow,
        };
        match gate {
            crate::controllers::Gate::Allow => {}
            crate::controllers::Gate::Deny => {
                let Some(who) = &self.req.remote else { unreachable!() };
                let perms = permissions::NodePermissions::from_node(self.node_settings());
                let level = perms.effective_level(who);
                let via = who.via_chatbot.then_some("chatbot");
                return Err(self.permission_denied(Denied { need: permissions::USER, level, via, members_only: true }));
            }
            crate::controllers::Gate::Levels => {
                if let Err(denied) = self.permit(Some((&server.id, &command, risk))) {
                    return Err(self.permission_denied(denied));
                }
            }
        }
        // 위험 명령은 그 서버만 가리키는 말로 불러야 한다: 게임명·모듈 별명은 서버가 늘면 다른 서버를 가리킬 수 있다
        if risk == "danger" && !by_name {
            return Err(self.t("errors.danger_needs_name", &[("name", &server.name)]).into());
        }

        let exec = match command.as_str() {
            "start" => Exec::Start,
            "stop" => Exec::Stop,
            "status" => Exec::Status,
            "players" if module.players => Exec::Players,
            // 디스코드·챗봇에 내보내지 않는 명령은 거기서 없는 명령이다
            _ => match module.command(&command).filter(|f| f.remote || self.req.remote.is_none()) {
                Some(field) => Exec::Defined(Box::new(field.clone())),
                // 없는 명령이면 이 모듈에서 쓸 수 있는 명령을 같이 보여준다
                // 릴레이는 해석에 실패하면 자기 문구 뒤에 `args.help`를 붙인다
                None => {
                    let help = self.module_help(alias, servers);
                    return Err(Reply::unrecognized(
                        "UNKNOWN_COMMAND",
                        Text::seq([self.t("errors.command_not_found", &[]), "\n\n".into(), help.clone()]),
                        json!({ "command": cmd_alias, "name": server.name, "instance": server.id, "extra": extra, "help": help.to_json() }),
                    ));
                }
            },
        };
        Ok(Target { server: server.clone(), exec, alias: alias.into(), cmd_alias: cmd_alias.into(), extra: extra.into(), risk })
    }

    async fn start(&self, server: &ServerView) -> Done {
        let resp = self.remote_response(self.api.start(server).await);
        // 약관 동의는 PC에서만 받는다: 릴레이가 원문 링크와 함께 안내하도록 목록을 싣는다
        if resp.str_field("error_code") == Some("agreement_required") {
            let text = resp.str_field("error").map(Text::from).unwrap_or_else(|| self.t("errors.error_title", &[]));
            return Done::new(Reply::unrecognized("AGREEMENT_REQUIRED", Text::seq(["📜 ".into(), text]), json!({ "agreements": resp.body.get("agreements") })), false);
        }
        let started = resp.is_success() && resp.body.get("success").and_then(Value::as_bool) != Some(false) && resp.str_field("action_required").is_none();
        let already_running = started && resp.body.get("already_running").and_then(Value::as_bool) == Some(true);
        let moved = self.port_moved_lines(&resp);
        let text = match already_running {
            true => self.t("server.already_running", &[("name", &server.name)]),
            false => self.start_text(server, resp),
        };
        Done { reply: Text::lines(std::iter::once(text).chain(moved)).into(), ok: started, already_running }
    }

    /// 시작 전에 예약 범위를 피해 옮긴 포트 (`port_moved`, 시작이 실패해도 옮긴 값은 저장돼 있다)
    fn port_moved_lines(&self, resp: &InternalResponse) -> Vec<Text> {
        let moves = resp.body.get("port_moved").and_then(Value::as_array).cloned().unwrap_or_default();
        moves
            .iter()
            .filter_map(|m| Some(self.t("server.port_moved", &[("from", &m.get("from")?.to_string()), ("to", &m.get("to")?.to_string())])))
            .collect()
    }

    fn start_text(&self, server: &ServerView, resp: InternalResponse) -> Text {
        // 감독자가 시작 전에 거절한 것(포트 충돌 등)은 2xx가 아니어도 그 문구 한 줄이다 (아래 `success:false`와 같다)
        let refused = resp.str_field("error_code").and_then(crate::service::lifecycle::refused_status).is_some();
        if !resp.is_success() && !refused {
            let detail = resp.str_field("error").unwrap_or_default();
            tracing::warn!("[Chat] 시작 실패 {}: {}", server.name, detail);
            // 켜기 전 점검(preflight_failed)의 문제 문구에는 경로가 들어 있을 수 있어 PC 밖으로 내지 않는다.
            // 점검 코드와 모듈이 준 해결 방법(경로 없는 문구)만 보인다
            if resp.str_field("error_code") == Some("preflight_failed") {
                let code = resp.str_field("issue_code").unwrap_or("preflight_failed");
                let line = Text::seq(["❌ ".into(), self.t("errors.preflight_failed", &[("code", code)])]);
                return match resp.str_field("solution").filter(|s| !s.trim().is_empty()) {
                    Some(solution) => Text::seq([line, format!("\n💡 {}", solution).into()]),
                    None => line,
                };
            }
            return self.error_line(detail);
        }
        if resp.body.get("success").and_then(|v| v.as_bool()) == Some(false) {
            let msg = resp.str_field("message").or(resp.str_field("error")).map(Text::from).unwrap_or_else(|| self.t("errors.error_title", &[]));
            return Text::seq(["❌ ".into(), msg]);
        }
        if let Some(action) = resp.str_field("action_required") {
            let msg = resp.str_field("message").map(str::to_string).unwrap_or_else(|| format!("Action required: {}", action));
            return format!("⚠️ {}", msg).into();
        }
        self.t("server.start_complete", &[("name", &server.name)])
    }

    async fn stop(&self, server: &ServerView) -> Done {
        let resp = self.remote_response(self.api.stop(server).await);
        let failed = !resp.is_success() || resp.body.get("success").and_then(|v| v.as_bool()) == Some(false);
        if failed {
            let detail = resp.str_field("error").or(resp.str_field("message")).unwrap_or_default();
            tracing::warn!("[Chat] 정지 실패 {}: {}", server.name, detail);
            return Done::new(self.error_line(detail), false);
        }
        Done::new(self.t("server.stop_complete", &[("name", &server.name)]), true)
    }

    /// 접속자 조회 (`[status] players`). 이름은 액션 요약과 같은 모양
    async fn players(&self, server: &ServerView) -> Done {
        let resp = self.remote_response(self.api.players(server).await);
        let failed = !resp.is_success() || resp.body.get("success").and_then(Value::as_bool) == Some(false);
        // 켜지는 중이면 게임이 아직 답하지 못한다: 실패가 아니라 기다릴 일이다
        if failed && server.starting {
            return Done::new(self.t("server.still_starting", &[("name", &server.name)]), false);
        }
        if failed {
            let code = crate::commands::error_code(resp.status.as_u16(), &resp.body);
            return Done::new(self.failed_line(server, self.t("command.players", &[]), reason(&code, &resp.body)), false);
        }
        let summary = crate::actions::summary::text("server.players", &resp.body, self.cat).unwrap_or_else(|| "".into());
        Done::new(Text::seq([format!("**{}**\n", server.name).into(), summary]), true)
    }

    /// 상태 한 줄: 실행 여부, 켜져 있으면 접속자와 켜진 시간 (PID는 보이지 않는다, 사용자 지정 2026-11-06)
    async fn status(&self, server: &ServerView) -> Text {
        let state = match (server.is_running(), server.starting) {
            (true, true) => self.t("status.starting", &[]),
            (true, false) => self.t("status.running", &[]),
            (false, _) => self.t("status.stopped", &[]),
        };
        let mut parts = vec![state];
        if server.is_running() {
            let players = self.module(&server.module).is_some_and(|m| m.players) && !server.starting;
            if players {
                let resp = self.api.players(server).await;
                if resp.is_success() && resp.body.get("success").and_then(Value::as_bool) != Some(false) {
                    let online = resp.body.get("online").and_then(Value::as_u64).unwrap_or(0).to_string();
                    parts.push(match resp.body.get("max").and_then(Value::as_u64) {
                        Some(max) => self.t("actions.summary.players_max", &[("online", &online), ("max", &max.to_string())]),
                        None => self.t("actions.summary.players", &[("online", &online)]),
                    });
                }
            }
            if let Some(up) = server.start_time.and_then(|t| self.uptime(t)) {
                parts.push(up);
            }
        }
        self.t("server.status_check", &[("name", &server.name)]).var("status", Text::join(" · ", parts))
    }

    /// 켜진 시간 ("켜진 지 1시간 2분"). 시작 시각이 앞날이면 None
    fn uptime(&self, started: u64) -> Option<Text> {
        let now = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).ok()?.as_secs();
        let mins = now.checked_sub(started)? / 60;
        let (h, m) = ((mins / 60).to_string(), (mins % 60).to_string());
        Some(match mins >= 60 {
            true => self.t("server.uptime_hm", &[("h", &h), ("m", &m)]),
            false => self.t("server.uptime_m", &[("m", &m)]),
        })
    }

    /// 정의된 명령 실행 — 해석·검증·라우팅은 데몬 명령 경로(`crate::commands`)가 한다
    async fn run_defined(&self, server: &ServerView, module: &ModuleView, field: &CommandField, alias: &str, cmd_alias: &str, extra: &str) -> Done {
        let args = split_quoted(extra);
        let resp = self.remote_response(self.api.command(&server.id, &field.name, args).await);
        let (text, ok) = self.render_result(server, module, field, alias, cmd_alias, resp);
        Done::new(text, ok)
    }

    fn remote_response(&self, mut resp: InternalResponse) -> InternalResponse {
        if self.req.remote.is_some() {
            crate::actions::strip_local_details(&mut resp.body);
        }
        resp
    }

    /// 명령의 보일 이름: 모듈 locales `commands.<이름>.label` → module.toml label → 이름
    /// 답하는 언어의 내장 명령 이름 (`actions.server.<명령>.name`) → 내장 명령
    fn builtin_names(&self) -> Vec<(String, &'static str)> {
        aliases::BUILTIN_COMMANDS
            .iter()
            .filter_map(|b| {
                let key = format!("actions.server.{b}.name");
                let name = self.word(&key);
                (name != key).then_some((name, *b))
            })
            .collect()
    }

    /// 채팅이 명령 이름처럼 받는 이름표: 답하는 언어로 보이는 이름 (`command_label`). 쉼표가 든 이름표는 받지 않는다
    fn calling_label(&self, module: &ModuleView, field: &CommandField) -> Option<String> {
        let label = words::tidy(&self.command_label(module, field));
        (!label.is_empty() && !label.contains(words::is_comma)).then_some(label)
    }

    fn command_label(&self, module: &ModuleView, field: &CommandField) -> String {
        module
            .text(self.cat.language(), &format!("/commands/{}/label", field.name))
            .unwrap_or_else(|| if field.label.is_empty() { field.name.clone() } else { field.label.clone() })
    }

    fn done_line(&self, server: &ServerView, label: impl Into<Text>) -> Text {
        self.t("command.done", &[("name", &server.name)]).var("label", label)
    }

    fn failed_line(&self, server: &ServerView, label: impl Into<Text>, reason: impl Into<Text>) -> Text {
        self.t("command.failed", &[("name", &server.name)]).var("label", label).var("reason", reason)
    }

    /// `❌ 명령어 실행 오류: <자세히>` (자세히가 없으면 앞만)
    fn error_line(&self, detail: impl Into<Text>) -> Text {
        Text::seq(["❌ ".into(), self.error_detail(detail)])
    }

    fn error_detail(&self, detail: impl Into<Text>) -> Text {
        let title = self.t("errors.error_title", &[]);
        let detail = detail.into();
        match detail.is_empty() {
            true => title,
            false => Text::seq([title, ": ".into(), detail]),
        }
    }

    /// 성공한 명령의 답: 성공과 명령 이름만. 콘솔 원문은 싣지 않고, 모듈이 실패 모양·답 모양을 정했으면 그것을 따른다.
    /// 모듈·익스텐션이 사람에게 쓴 `message`와 REST 조회 결과(값 줄)는 아래에 붙인다
    /// bool은 성공인지 (모듈의 실패 모양에 맞으면 false)
    fn success_text(&self, server: &ServerView, module: &ModuleView, field: &CommandField, resp: &InternalResponse) -> (Text, bool) {
        let label = self.command_label(module, field);
        let data = resp.body.get("data").filter(|d| !d.is_null());
        let console = data.and_then(|d| d.get("response")).and_then(Value::as_str).unwrap_or("");
        for pattern in &module.failure_patterns {
            if let Some(caps) = regex::Regex::new(pattern).ok().and_then(|re| re.captures(console)) {
                let reason = caps.name("reason").map(|m| Text::from(m.as_str().trim())).unwrap_or_else(|| self.t("reasons.rejected", &[]));
                return (self.failed_line(server, label, reason), false);
            }
        }
        if let Some(caps) = field.reply_pattern.as_deref().and_then(|p| regex::Regex::new(p).ok()).and_then(|re| re.captures(console)) {
            if let Some(template) = module.text(self.cat.language(), &format!("/commands/{}/done", field.name)) {
                let re = regex::Regex::new(field.reply_pattern.as_deref().unwrap_or_default()).expect("compiled above");
                let vars: Vec<(&str, &str)> = re.capture_names().flatten().filter_map(|n| Some((n, caps.name(n)?.as_str()))).collect();
                return (self.done_line(server, i18n::interpolate(&template, &vars)), true);
            }
        }
        // 값으로 답한 REST 조회는 데몬이 정리한 결과(`commands::output`)를 보인다
        let lang = self.cat.language();
        let labels = crate::commands::output::labels(&field.name, &module.settings, &module.locale(lang), &module.locale("en"));
        if let Some(output) = crate::commands::output::build(&resp.body, &labels) {
            return (Text::lines([self.done_line(server, label), format::format_output(&output).into()]), true);
        }
        // 모듈·익스텐션 명령은 답을 `message`로 준다 (stdin의 message는 내부 문구)
        let message = match resp.str_field("protocol") {
            Some("module" | "extension") => resp.body.get("message").filter(|m| m.as_str().is_some_and(|t| !t.trim().is_empty())),
            _ => None,
        };
        let detail = match data.or(message) {
            // 콘솔 명령의 답은 게임 서버 로그 줄이라 보이지 않는다
            Some(d) if d.get("response").is_some() => None,
            Some(d) => Some(format::format_response(d)).filter(|t| !t.is_empty() && t.key_name() != Some(format::COMMAND_COMPLETE)),
            None => None,
        };
        let head = self.done_line(server, label);
        let text = match detail {
            Some(detail) => Text::lines([head, detail]),
            None => head,
        };
        (text, true)
    }

    /// 틀린 인자의 이유 (`rule`·`params`, 데몬 검사가 준다). 칸 이름은 모듈 locales `commands.<명령>.inputs.<칸>.label`.
    /// 릴레이가 모르는 규칙이면 일반 오류 줄로 보인다
    fn invalid_text(&self, module: &ModuleView, field: &CommandField, resp: &InternalResponse) -> Text {
        let fallback = self.error_detail(resp.str_field("error").unwrap_or_default());
        let Some(rule) = resp.str_field("rule") else { return Text::seq(["❌ ".into(), fallback]) };
        let params = resp.body.get("params").cloned().unwrap_or(Value::Null);
        let name = resp.str_field("field").unwrap_or_default();
        let label = module
            .text(self.cat.language(), &format!("/commands/{}/inputs/{}/label", field.name, name))
            .or_else(|| field.inputs.iter().find(|i| i.name == name).and_then(|i| i.label.clone()))
            .unwrap_or_else(|| name.to_string());
        let num = |key: &str| params.get(key).and_then(Value::as_f64).map(|n| n.to_string()).unwrap_or_else(|| "-".into());
        let options = params.get("options").and_then(Value::as_array).map(|o| o.iter().filter_map(Value::as_str).collect::<Vec<_>>().join(", ")).unwrap_or_default();
        let value = params.get("value").and_then(Value::as_str).unwrap_or_default();
        let text = self.t(&format!("command.invalid.{}", rule), &[("label", &label), ("value", value), ("min", &num("min")), ("max", &num("max")), ("options", &options)]);
        Text::seq(["❌ ".into(), text.or(fallback)])
    }

    /// 답과 성공 여부
    fn render_result(&self, server: &ServerView, module: &ModuleView, field: &CommandField, alias: &str, cmd_alias: &str, resp: InternalResponse) -> (Text, bool) {
        let failed = !resp.is_success() || resp.body.get("success").and_then(|v| v.as_bool()) == Some(false);
        if !failed {
            return self.success_text(server, module, field, &resp);
        }
        (self.failure_text(server, module, field, alias, cmd_alias, resp), false)
    }

    fn failure_text(&self, server: &ServerView, module: &ModuleView, field: &CommandField, alias: &str, cmd_alias: &str, resp: InternalResponse) -> Text {
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
                let error = module_text.map(Text::from).unwrap_or_else(|| self.t("server.not_running_default", &[]));
                self.t("server.not_running", &[("name", &server.name)]).var("error", error)
            }
            "no_command_path" => Text::seq(["❌ ".into(), self.t("errors.no_command_path", &[("name", &server.name)])]),
            "invalid_argument" => self.invalid_text(module, field, &resp),
            "auth_failed" => self.error_line(module_text.map(Text::from).unwrap_or_else(|| self.t("errors.auth_failed", &[]))),
            "not_found" => self.error_line(resp.str_field("error").map(Text::from).unwrap_or_else(|| self.t("errors.not_found", &[]))),
            _ => self.error_line(
                module_text
                    .or_else(|| module.errors.get("internal_server_error").cloned())
                    .map(Text::from)
                    .unwrap_or_else(|| self.t("errors.internal_server_error", &[])),
            ),
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

/// 실패 코드의 짧은 이유 (`reasons.<코드>`, 릴레이에 그 문구가 없으면 일반 문구). 액션 결과(`reason`)도 이것을 쓴다
pub fn reason(code: &str, body: &Value) -> Text {
    reason_or(code, body, Text::key("reasons.failed"))
}

/// 실패 코드의 짧은 이유. 릴레이에 그 코드 문구가 없으면 `fallback`
pub fn reason_or(code: &str, body: &Value, fallback: impl Into<Text>) -> Text {
    let file = body.get("file").and_then(Value::as_str).unwrap_or_default();
    Text::key(format!("reasons.{}", code)).var("file", file).or(fallback)
}

#[cfg(test)]
mod tests;
