//! 릴레이 에이전트 — 클라우드 릴레이(오라클의 서버쨩 봇) ↔ 데몬 연결
//!
//! 디스코드 봇은 클라우드에서만 돌고, 데몬은 이 에이전트로 릴레이에 붙습니다.
//!   1. GET  /info              — 최소 에이전트 버전 확인
//!   2. POST /heartbeat (60초)  — 온라인 유지 + 서버/모듈/봇 설정 메타데이터 동기화
//!   3. GET  /poll (롱폴링)      — 디스코드에서 온 텍스트 명령 수신
//!   4. POST /result/:id        — 채팅 처리기(`crate::chat`) 결과 반환
//!   5. POST /events            — 디스코드 알림 채널(I3)에 띄울 이벤트 (`relay_event`)
//!
//! 인증은 노드 토큰(`<data_dir>/.node_token`, sbn_<id>.<secret>) HMAC 서명입니다.
//! 새 명령 종류는 `handle_payload`의 action 분기에 추가합니다.

use crate::ipc::handlers::relay::{load_node_token, parse_node_token, resolve_relay_url, signed_headers_with};
use crate::ipc::IPCServer;
use serde::Serialize;
use serde_json::{json, Value};
use std::sync::{Arc, Mutex};
use std::time::Duration;
use tokio_util::sync::CancellationToken;

pub const AGENT_VERSION: &str = env!("CARGO_PKG_VERSION");

const HEARTBEAT_INTERVAL: Duration = Duration::from_secs(60);
const POLL_TIMEOUT: Duration = Duration::from_secs(40); // 릴레이 롱폴 25초 + 여유
const RETRY_BASE: Duration = Duration::from_secs(3);
const RETRY_MAX: Duration = Duration::from_secs(60);
/// 알림 이벤트(I3): 처음 한 건이 오면 이만큼 모아서, 한 번에 이만큼까지 보낸다
const EVENT_GATHER: Duration = Duration::from_secs(2);
const EVENT_BATCH: usize = 20;
/// relay에 닿지 못하는 동안 들고 있는 최근 이벤트 수 (알림은 기록이 아니다)
const EVENT_PENDING: usize = 50;
/// relay에 `/events`가 없을 때(404) 다시 알아보기까지 쉬는 시간. 그동안의 이벤트는 버린다
const EVENT_PROBE: Duration = Duration::from_secs(600);

/// 세션 안에서 토큰 대체를 알리는 오류 표시 (밖으로 나가지 않는다)
const REPLACED: &str = "token_replaced";
/// 동시에 처리하는 릴레이 명령 수 (넘으면 끝나기를 기다렸다가 더 받는다)
const COMMANDS_IN_FLIGHT: usize = 4;

/// 에이전트 상태 (GET /api/relay/agent)
#[derive(Debug, Clone, Default, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct AgentStatus {
    pub running: bool,
    pub relay_url: Option<String>,
    pub has_token: bool,
    pub agent_version: &'static str,
    /// 마지막 하트비트 성공 시각 (epoch ms)
    pub last_heartbeat_at: Option<u64>,
    pub last_error: Option<String>,
    /// 끊긴 이유를 클라이언트가 현지화할 수 있게 주는 코드. `token_replaced`: 같은 디스코드 계정이 다른 PC를 페어링했다
    #[serde(skip_serializing_if = "Option::is_none")]
    pub last_error_code: Option<&'static str>,
    pub commands_handled: u64,
}

#[derive(Default)]
struct Inner {
    cancel: Option<CancellationToken>,
    status: AgentStatus,
    /// 시작할 때마다 늘어난다. 끝난 세션이 새 세션의 상태를 지우지 않게 한다
    generation: u64,
}

/// 에이전트 핸들 — 복제해도 같은 에이전트를 가리킵니다.
#[derive(Clone, Default)]
pub struct RelayAgent {
    inner: Arc<Mutex<Inner>>,
}

impl RelayAgent {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn status(&self) -> AgentStatus {
        let inner = self.lock();
        let mut status = inner.status.clone();
        status.agent_version = AGENT_VERSION;
        status.has_token = load_node_token().as_deref().and_then(parse_node_token).is_some();
        status
    }

    /// 에이전트를 시작합니다. 이미 돌고 있으면 그대로 둡니다.
    pub async fn start(&self, server: IPCServer) -> Result<(), String> {
        // 세션은 시작할 때의 주소·토큰 쌍만 쓴다: 주소나 토큰이 바뀌면 새 세션이 시작된다
        let Some(node_token) = load_node_token().filter(|t| parse_node_token(t).is_some()) else {
            return Err("node token missing or malformed — pair this node first".into());
        };
        let relay_url = {
            let bot = server.config_store.get_bot_config().await;
            let url = bot.cloud.relay_url.trim().to_string();
            resolve_relay_url(Some(url.as_str()))
        };
        let (cancel, generation) = {
            let mut inner = self.lock();
            if inner.cancel.is_some() {
                return Ok(());
            }
            let token = server.shutdown_token.child_token();
            inner.cancel = Some(token.clone());
            inner.generation += 1;
            inner.status = AgentStatus { running: true, relay_url: Some(relay_url.clone()), ..Default::default() };
            (token, inner.generation)
        };

        let agent = self.clone();
        tokio::spawn(async move {
            let client = match http_client() {
                Ok(c) => c,
                Err(e) => {
                    agent.finish(generation, Some(format!("http client: {}", e)));
                    return;
                }
            };
            let session = Session { agent: agent.clone(), server, client, base: relay_url, token: node_token, cancel: cancel.clone(), generation };
            if let Err(e) = session.check_version().await {
                tracing::error!("[RelayAgent] {}", e);
                agent.finish(generation, Some(e));
                return;
            }
            tracing::info!("[RelayAgent] started (relay={})", session.base);
            // 챗봇 설정은 데몬이 원본: 연결될 때마다 릴레이에 맞춘다
            // 세션이 끝나면(연결 해제·토큰 교체) 같이 멈춘다: 예전 토큰으로 설정을 올리지 않게
            let sync_server = session.server.clone();
            let sync_cancel = cancel.clone();
            tokio::spawn(async move {
                tokio::select! {
                    _ = sync_cancel.cancelled() => {}
                    _ = async {
                        crate::ipc::handlers::chatbot::sync_on_connect(&sync_server).await;
                        // 디스코드 길드 설정도 데몬이 원본 (I2)
                        crate::ipc::handlers::discord::sync_on_connect(&sync_server).await;
                    } => {}
                }
            });
            tokio::join!(session.heartbeat_loop(), session.poll_loop(), session.events_loop());
            agent.finish(generation, None);
            tracing::info!("[RelayAgent] stopped");
        });
        Ok(())
    }

    pub fn stop(&self) {
        let mut inner = self.lock();
        if let Some(cancel) = inner.cancel.take() {
            cancel.cancel();
        }
        inner.status.running = false;
    }

    /// 세션이 끝났을 때. 그 사이 새 세션이 시작됐으면(세대가 다르면) 아무것도 건드리지 않는다
    fn finish(&self, generation: u64, error: Option<String>) {
        let mut inner = self.lock();
        if inner.generation != generation {
            return;
        }
        inner.cancel = None;
        inner.status.running = false;
        if error.is_some() {
            inner.status.last_error = error;
        }
    }

    fn lock(&self) -> std::sync::MutexGuard<'_, Inner> {
        self.inner.lock().unwrap_or_else(|e| e.into_inner())
    }

    /// 세션 상태 갱신. 끝난(세대가 지난) 세션의 늦은 응답은 버린다
    fn update(&self, generation: u64, f: impl FnOnce(&mut AgentStatus)) {
        let mut inner = self.lock();
        if inner.generation == generation && inner.cancel.is_some() {
            f(&mut inner.status)
        }
    }
}

/// 데몬 기동 시: 이 노드가 페어링돼 있으면(노드 토큰 존재) 에이전트를 띄운다
/// 새 토큰을 저장한 뒤: 돌던 세션을 멈추고 새 토큰으로 다시 연결한다 (페어링을 마치면 바로 디스코드 명령을 받게)
pub async fn restart(server: &IPCServer) {
    server.relay_agent.stop();
    auto_start(server).await;
}

pub async fn auto_start(server: &IPCServer) {
    if let Err(e) = server.relay_agent.start(server.clone()).await {
        tracing::info!("[RelayAgent] not started: {}", e);
    }
}

/// relay가 이 노드 토큰을 같은 계정의 새 페어링으로 대체했다고 답했는가
fn token_replaced(status: reqwest::StatusCode, body: &Value) -> bool {
    status == reqwest::StatusCode::UNAUTHORIZED
        && body.get("error").and_then(|v| v.as_str()).is_some_and(|e| e.eq_ignore_ascii_case("TOKEN_REPLACED"))
}

fn http_client() -> Result<reqwest::Client, reqwest::Error> {
    let mut headers = reqwest::header::HeaderMap::new();
    headers.insert("X-Saba-Client", reqwest::header::HeaderValue::from_static("1"));
    reqwest::Client::builder().timeout(POLL_TIMEOUT).default_headers(headers).build()
}

struct Session {
    agent: RelayAgent,
    server: IPCServer,
    client: reqwest::Client,
    base: String,
    token: String,
    cancel: CancellationToken,
    generation: u64,
}

impl Session {
    /// 서명된 요청 생성. body는 서명한 문자열 그대로 보낸다.
    fn signed(&self, method: reqwest::Method, path: &str, body: Option<String>) -> Result<reqwest::RequestBuilder, String> {
        // 끝난 세션(연결 해제·주소나 토큰 교체)은 아무것도 보내지 않는다
        if self.cancel.is_cancelled() {
            return Err("relay session ended".into());
        }
        let headers = signed_headers_with(&self.token, method.as_str(), path, body.as_deref()).ok_or("node token missing")?;
        let mut req = self.client.request(method, format!("{}{}", self.base, path));
        for (k, v) in headers {
            req = req.header(k, v);
        }
        if let Some(b) = body {
            req = req.header("Content-Type", "application/json").body(b);
        }
        Ok(req)
    }

    /// 서명된 요청을 보낸다. 보내는 중에 세션이 끝나면 그만둔다
    async fn send_signed(&self, method: reqwest::Method, path: &str, body: Option<String>) -> Result<reqwest::Response, String> {
        let req = self.signed(method, path, body)?;
        tokio::select! {
            _ = self.cancel.cancelled() => Err("relay session ended".into()),
            r = req.send() => r.map_err(|e| e.to_string()),
        }
    }

    /// 이 PC의 토큰이 다른 PC 페어링으로 대체됐다: 같은 토큰으로 다시 시도해도 소용없으니 멈춘다.
    /// 이 PC에서 다시 페어링하면 새 토큰 저장이 에이전트를 다시 띄운다
    fn give_up_replaced(&self) {
        tracing::warn!("[RelayAgent] node token was replaced by a newer pairing of the same Discord account; stopping");
        self.agent.update(self.generation, |s| {
            s.last_error = Some("node token replaced by a newer pairing".into());
            s.last_error_code = Some("token_replaced");
        });
        self.cancel.cancel();
    }

    async fn check_version(&self) -> Result<(), String> {
        let info: Value = match self.client.get(format!("{}/info", self.base)).send().await {
            Ok(r) if r.status().is_success() => r.json().await.unwrap_or(Value::Null),
            Ok(r) => {
                tracing::warn!("[RelayAgent] /info {} — skipping version check", r.status());
                return Ok(());
            }
            Err(e) => {
                tracing::warn!("[RelayAgent] /info failed — skipping version check: {}", e);
                return Ok(());
            }
        };
        if let Some(min) = info.get("minAgentVersion").and_then(|v| v.as_str()) {
            if semver_lt(AGENT_VERSION, min) {
                return Err(format!("agent {} is older than relay minimum {} — update saba-chan", AGENT_VERSION, min));
            }
        }
        Ok(())
    }

    async fn heartbeat_loop(&self) {
        loop {
            self.heartbeat().await;
            tokio::select! {
                _ = self.cancel.cancelled() => return,
                _ = tokio::time::sleep(HEARTBEAT_INTERVAL) => {}
            }
        }
    }

    async fn heartbeat(&self) {
        let body = json!({
            "agentVersion": AGENT_VERSION,
            "os": format!("{} {}", std::env::consts::OS, std::env::consts::ARCH),
            "metadata": self.metadata().await,
        });
        let result = async {
            let resp = self.send_signed(reqwest::Method::POST, "/heartbeat", Some(body.to_string())).await?;
            let status = resp.status();
            let data: Value = resp.json().await.unwrap_or(Value::Null);
            if token_replaced(status, &data) {
                return Err(REPLACED.to_string());
            }
            if !status.is_success() {
                let hint = if status.as_u16() == 401 || status.as_u16() == 403 { " (node token rejected)" } else { "" };
                return Err(format!("heartbeat {}{}: {}", status, hint, data.get("error").and_then(|v| v.as_str()).unwrap_or("")));
            }
            Ok(data)
        }
        .await;

        match result {
            Ok(data) => {
                if data.get("warning").and_then(|v| v.as_str()) == Some("UPDATE_REQUIRED") {
                    tracing::warn!("[RelayAgent] relay asks for agent update (min {})", data.get("minVersion").and_then(|v| v.as_str()).unwrap_or("?"));
                }
                self.agent.update(self.generation, |s| {
                    s.last_heartbeat_at = Some(now_ms());
                    s.last_error = None;
                });
                // 연결이 끊긴 사이 릴레이에 못 보낸 디스코드 길드 설정을 다시 보낸다
                if crate::ipc::handlers::discord::take_pending_sync() {
                    let (server, cancel) = (self.server.clone(), self.cancel.clone());
                    tokio::spawn(async move {
                        tokio::select! {
                            _ = cancel.cancelled() => {}
                            _ = crate::ipc::handlers::discord::sync_on_connect(&server) => {}
                        }
                    });
                }
            }
            Err(e) if e == REPLACED => self.give_up_replaced(),
            Err(e) => {
                tracing::warn!("[RelayAgent] {}", e);
                self.agent.update(self.generation, |s| s.last_error = Some(e));
            }
        }
    }

    /// 하트비트 메타데이터: 릴레이 봇이 자동완성·접두사 표시에 쓴다
    async fn metadata(&self) -> Value {
        use crate::chat::api::DaemonApi;
        let servers: Vec<Value> = match crate::ipc::internal::into_internal(
            crate::ipc::handlers::server::list_servers(axum::extract::State(self.server.clone())).await,
        )
        .await
        .body
        .get("servers")
        {
            Some(Value::Array(list)) => {
                // `/사바쨩 상태`의 CPU·RAM (폴더 크기는 재지 않는다)
                let roots = crate::ipc::handlers::system::instance_roots(&self.server).await;
                let monitor = self.server.system_monitor.clone();
                let usage = tokio::task::spawn_blocking(move || monitor.sample(&roots, &[], false).instances).await.unwrap_or_default();
                list.iter()
                    .map(|s| relay_server_entry(s, usage.iter().find(|u| s.get("id").and_then(|v| v.as_str()) == Some(u.id.as_str()))))
                    .collect()
            }
            _ => Vec::new(),
        };
        let modules: Vec<Value> = self
            .server
            .modules()
            .await
            .into_iter()
            .map(|m| {
                json!({
                    "name": m.name,
                    "module_aliases": m.module_aliases,
                    "commands": { "fields": m.commands.iter().map(|c| relay_command_entry(c, m.aliases_of(&c.name))).collect::<Vec<_>>() },
                    "presets": m.presets.iter().map(|p| json!({ "id": p.id, "label": p.label })).collect::<Vec<_>>(),
                })
            })
            .collect();

        let bot = self.server.config_store.get_bot_config().await;
        // 릴레이 스키마: commandAliases는 모듈 → (명령 → 콤마 문자열)
        let command_aliases: serde_json::Map<String, Value> = bot
            .command_aliases
            .iter()
            .filter_map(|(module, cmds)| {
                let cmds: serde_json::Map<String, Value> = cmds
                    .as_object()?
                    .iter()
                    .filter_map(|(c, a)| a.as_str().map(|a| (c.clone(), Value::String(a.to_string()))))
                    .collect();
                Some((module.clone(), Value::Object(cmds)))
            })
            .collect();

        let mut meta = json!({
            "servers": servers,
            "modules": modules,
            "moduleDetails": {},
            "botConfig": {
                // 비어 있으면 릴레이가 응답 언어의 기본값을 쓴다
                "prefixes": bot.prefixes,
                "moduleAliases": bot.module_aliases,
                "commandAliases": command_aliases,
            },
        });
        // 옛 릴레이는 `prefix` 하나만 안다 (첫 접두사, 기본값이면 싣지 않는다)
        if let Some(first) = bot.prefixes.first() {
            meta["botConfig"]["prefix"] = json!(first);
        }
        // 길드별 허용 인스턴스. 목록이 있는 길드만 싣는다 (없으면 릴레이는 제한 없음으로 본다)
        if let Some(allowed) = allowed_instances_by_guild(&bot.node_settings) {
            meta["allowedInstances"] = allowed;
        }
        // 길드·인스턴스별로 서버와 명령을 부르는 낱말. 항목이 있는 길드만 (릴레이는 찾기·위험 확인에 쓴다)
        if let Some(words) = crate::chat::words::heartbeat(&bot.node_settings) {
            meta["instanceWords"] = words;
        }
        // 권한 원본은 데몬이다: 릴레이는 이 값으로 바깥 검사용 캐시를 맞춘다
        meta["permissions"] = permissions_by_guild(&bot.node_settings);
        // `/사바쨩 상태` 끝 줄의 다음 할 일 (J1-a, GUI·CLI와 같은 단계)
        meta["onboarding"] = crate::ipc::handlers::diagnostics::onboarding_now(&self.server).await;
        // 언어가 `auto`인 길드는 이 언어로 답한다 (I2)
        // (릴레이 스키마는 10자까지. 넘으면 heartbeat 전체가 거절되므로 싣지 않는다)
        if let Some(language) = Some(self.server.config_store.ui_language().await).filter(|l| l.len() <= 10) {
            meta["language"] = Value::String(language);
        }
        // 디스코드·챗봇에서 쓸 수 있는 액션 (P2). 상한을 넘으면 잘라 표시한다
        let loaded = self.server.supervisor.read().await.list_modules().unwrap_or_default();
        let (actions, truncated) = crate::actions::heartbeat_actions(&crate::chat::i18n::languages(), &loaded);
        meta["actions"] = Value::Array(actions);
        if truncated {
            meta["actionsTruncated"] = Value::Bool(true);
        }
        meta
    }

    /// 받은 명령은 동시에 처리하고 그동안에도 계속 받는다: 오래 걸리는 명령(재시작·백업 등) 하나가
    /// 뒤의 명령을 릴레이 대기 시간 너머로 붙잡지 않게 (한 번에 `COMMANDS_IN_FLIGHT`개까지)
    async fn poll_loop(&self) {
        use futures_util::StreamExt;
        type Poll<'a> = std::pin::Pin<Box<dyn std::future::Future<Output = Result<Vec<Value>, String>> + Send + 'a>>;
        let mut failures: u32 = 0;
        let mut running = futures_util::stream::FuturesUnordered::new();
        let mut waiting: Vec<Value> = Vec::new();
        let mut next: Option<Poll<'_>> = Some(Box::pin(self.poll_once()));
        while !self.cancel.is_cancelled() {
            while running.len() < COMMANDS_IN_FLIGHT && !waiting.is_empty() {
                running.push(self.run_command(waiting.remove(0)));
            }
            if next.is_none() && waiting.is_empty() && running.len() < COMMANDS_IN_FLIGHT {
                next = Some(Box::pin(self.poll_once()));
            }
            tokio::select! {
                polled = async { next.as_mut().expect("poll").await }, if next.is_some() => {
                    next = None;
                    match polled {
                        Ok(commands) => {
                            failures = 0;
                            waiting.extend(commands);
                        }
                        Err(e) if e == REPLACED => self.give_up_replaced(),
                        Err(e) => {
                            failures += 1;
                            let backoff = backoff(failures);
                            tracing::warn!("[RelayAgent] poll failed ({}), retry in {}s", e, backoff.as_secs());
                            self.agent.update(self.generation, |s| s.last_error = Some(e));
                            next = Some(Box::pin(async move {
                                self.pause(backoff).await;
                                self.poll_once().await
                            }));
                        }
                    }
                }
                Some(()) = running.next(), if !running.is_empty() => {}
            }
        }
        // 연결 해제(취소) 뒤에는 받아 두고 시작하지 않은 명령은 버리고, 이미 시작한 것은 끝까지 마친다
        while running.next().await.is_some() {}
    }

    async fn run_command(&self, cmd: Value) {
        let Some(id) = cmd.get("id").and_then(Value::as_str).map(str::to_string) else { return };
        let result = self.handle_payload(&cmd).await;
        self.post_result(&id, &result).await;
        self.agent.update(self.generation, |s| s.commands_handled += 1);
    }

    async fn pause(&self, d: Duration) {
        tokio::select! {
            _ = self.cancel.cancelled() => {}
            _ = tokio::time::sleep(d) => {}
        }
    }

    async fn poll_once(&self) -> Result<Vec<Value>, String> {
        let req = self.signed(reqwest::Method::GET, "/poll", None)?;
        let resp = tokio::select! {
            _ = self.cancel.cancelled() => return Ok(Vec::new()),
            r = req.send() => r.map_err(|e| e.to_string())?,
        };
        let status = resp.status();
        if status.as_u16() == 204 {
            return Ok(Vec::new());
        }
        let text = tokio::select! {
            _ = self.cancel.cancelled() => return Ok(Vec::new()),
            b = resp.text() => b.unwrap_or_default(),
        };
        let body = decode_poll(&text);
        if token_replaced(status, &body) {
            return Err(REPLACED.to_string());
        }
        if !status.is_success() {
            return Err(format!("poll {}: {}", status, body.get("error").and_then(|v| v.as_str()).unwrap_or("")));
        }
        Ok(body.get("commands").and_then(|v| v.as_array()).cloned().unwrap_or_default())
    }

    async fn handle_payload(&self, cmd: &Value) -> Value {
        let payload = cmd.get("payload").cloned().unwrap_or(Value::Null);
        let action = payload.get("action").and_then(|v| v.as_str()).unwrap_or("");
        match action {
            "raw_command" => {
                let text = payload.get("text").and_then(|v| v.as_str()).unwrap_or("");
                let name = |key: &str| {
                    payload.get(key).and_then(|v| v.as_str()).map(str::trim).filter(|s| !s.is_empty()).map(str::to_string)
                };
                let guild_id = cmd.get("guildId").and_then(|v| v.as_str()).map(str::to_string);
                let user_id = cmd.get("requestedBy").and_then(|v| v.as_str()).map(str::to_string);
                let remote = self.requester(cmd, &payload, guild_id.clone(), user_id.clone()).await;
                let req = crate::chat::ChatRequest {
                    text: text.to_string(),
                    guild_id,
                    user_id,
                    user_name: name("userName"),
                    guild_name: name("guildName"),
                    locale: name("locale"),
                    remote: Some(remote),
                };
                // 감사 로그 (A10): 디스코드에서 누가 보냈는지 (이름은 릴레이가 payload에 실어 보낸 것)
                let origin = crate::audit::Origin::new("discord").requested_by(&req);
                let reply = crate::audit::with_origin(origin, crate::chat::api::run_with_daemon_settings(&self.server, &req)).await;
                raw_command_result(reply)
            }
            "permission" => {
                let guild_id = cmd.get("guildId").and_then(|v| v.as_str()).map(str::to_string);
                let user_id = cmd.get("requestedBy").and_then(|v| v.as_str()).map(str::to_string);
                let who = self.requester(cmd, &payload, guild_id, user_id).await;
                let origin = crate::audit::Origin::new("discord");
                crate::audit::with_origin(origin, permission_action(&self.server, &who, &payload)).await
            }
            // 액션 카탈로그 (P2·P3): 계획·확인한 계획 실행·읽기 전용 조회
            "plan" | "run_plan" | "query" => {
                let guild_id = cmd.get("guildId").and_then(|v| v.as_str()).map(str::to_string);
                let user_id = cmd.get("requestedBy").and_then(|v| v.as_str()).map(str::to_string);
                let who = self.requester(cmd, &payload, guild_id.clone(), user_id.clone()).await;
                let text = |key: &str| payload.get(key).and_then(Value::as_str).map(str::to_string);
                let mut origin = crate::audit::Origin::new("discord");
                origin.user = user_id;
                origin.guild = guild_id;
                origin.user_name = text("userName");
                origin.guild_name = text("guildName");
                let caller = crate::actions::Caller {
                    surface: if who.via_chatbot { crate::actions::Surface::Chatbot } else { crate::actions::Surface::Discord },
                    remote: Some(who),
                    origin: origin.clone(),
                    locale: text("locale").unwrap_or_else(|| "en".into()),
                };
                let result = crate::audit::with_origin(origin, action_request(&self.server, action, &payload, &caller)).await;
                action_result(result)
            }
            // 디스코드에서 코드 없이 `/사바쨩 연결` (J1-b): PC 화면에 알리기만 한다. 수락은 PC에서 사람이 누른다
            "link_request" => match link_request_event(cmd, &payload) {
                Some(data) => {
                    crate::events::emit("relay.link_request", data);
                    json!({ "success": true, "data": { "notified": true } })
                }
                None => json!({ "success": false, "data": { "error": "invalid link_request" } }),
            },
            other => json!({ "success": false, "data": { "error": format!("Unknown action: {}", other) } }),
        }
    }

    /// 릴레이 명령의 요청자. 노드 주인은 처음 받은 값으로 고정하고(`nodeSettings[guild].ownerDiscordId`),
    /// 다른 값이 오면 무시하고 경고를 남긴다
    async fn requester(&self, cmd: &Value, payload: &Value, guild_id: Option<String>, user_id: Option<String>) -> crate::chat::permissions::Requester {
        let reported = cmd.get("ownerDiscordId").and_then(Value::as_str);
        let owner_id = match guild_id.as_deref() {
            Some(guild) => {
                let pinned = self
                    .server
                    .config_store
                    .update_node_settings(guild, |node| {
                        let (owner, changed, conflict) = crate::chat::permissions::pin_owner(node, reported);
                        ((owner, conflict), changed)
                    })
                    .await;
                match pinned {
                    Ok((owner, conflict)) => {
                        if let Some(conflict) = conflict {
                            tracing::warn!("[RelayAgent] guild {}: relay reported a different owner ({}); keeping the pinned owner", guild, conflict);
                        }
                        owner
                    }
                    Err(e) => {
                        // 고정할 수 없으면 주인을 가정하지 않는다
                        tracing::warn!("[RelayAgent] could not read node settings for {}: {}", guild, e);
                        None
                    }
                }
            }
            None => None,
        };
        crate::chat::permissions::Requester {
            guild_id,
            user_id,
            owner_id,
            via_chatbot: crate::chat::permissions::via_chatbot(payload.get("via").and_then(Value::as_str), reported.is_some()),
        }
    }

    /// 데몬 이벤트 중 알림 채널에 띄울 것(`relay_event`)을 모아 `POST /events`로 올린다.
    /// relay가 404면(아직 그 경로가 없음) `EVENT_PROBE` 동안 올리지 않는다
    async fn events_loop(&self) {
        use tokio::sync::broadcast::error::{RecvError, TryRecvError};
        let mut rx = crate::events::bus().subscribe(None).rx;
        let mut pending = std::collections::VecDeque::new();
        let mut failures: u32 = 0;
        loop {
            if pending.is_empty() {
                // 첫 건을 기다렸다가 잠깐 더 모은다
                let gather = tokio::time::sleep(EVENT_GATHER);
                tokio::pin!(gather);
                let mut gathering = false;
                loop {
                    tokio::select! {
                        _ = self.cancel.cancelled() => return,
                        _ = &mut gather, if gathering => break,
                        r = rx.recv() => match r {
                            Ok(e) => {
                                queue_event(&mut pending, &e);
                                if !gathering && !pending.is_empty() {
                                    gathering = true;
                                    gather.as_mut().reset(tokio::time::Instant::now() + EVENT_GATHER);
                                }
                            }
                            Err(RecvError::Lagged(_)) => {}
                            Err(RecvError::Closed) => return,
                        },
                    }
                }
            }
            // 다시 보내기 전에 그 사이 쌓인 것도 넣는다
            loop {
                match rx.try_recv() {
                    Ok(e) => queue_event(&mut pending, &e),
                    Err(TryRecvError::Lagged(_)) => {}
                    Err(_) => break,
                }
            }
            let batch: Vec<Value> = pending.iter().take(EVENT_BATCH).cloned().collect();
            let sent = async {
                let body = json!({ "events": batch }).to_string();
                self.send_signed(reqwest::Method::POST, "/events", Some(body)).await
            };
            let resp = tokio::select! {
                _ = self.cancel.cancelled() => return,
                r = sent => r,
            };
            match resp {
                // relay에 경로가 아직 없다: 잠시 쉬었다가 그 뒤 이벤트로 다시 알아본다 (relay가 나중에 배포될 수 있다)
                Ok(r) if r.status().as_u16() == 404 => {
                    tracing::info!("[RelayAgent] relay has no /events yet — retrying in {}s", EVENT_PROBE.as_secs());
                    pending.clear();
                    self.pause(EVENT_PROBE).await;
                    if self.cancel.is_cancelled() {
                        return;
                    }
                    rx = rx.resubscribe();
                }
                // 4xx는 다시 보내도 같다: 그 묶음을 버린다 (429는 relay의 노드당 한도라 아래에서 쉬었다 다시 보낸다)
                Ok(r) if r.status().is_success() || (r.status().is_client_error() && r.status().as_u16() != 429) => {
                    if !r.status().is_success() {
                        tracing::warn!("[RelayAgent] events rejected: {}", r.status());
                    }
                    pending.drain(..batch.len());
                    failures = 0;
                }
                other => {
                    failures += 1;
                    let backoff = backoff(failures);
                    match other {
                        Ok(r) => tracing::warn!("[RelayAgent] events {}, retry in {}s", r.status(), backoff.as_secs()),
                        Err(e) => tracing::warn!("[RelayAgent] events failed ({}), retry in {}s", e, backoff.as_secs()),
                    }
                    self.pause(backoff).await;
                    if self.cancel.is_cancelled() {
                        return;
                    }
                }
            }
        }
    }

    async fn post_result(&self, id: &str, result: &Value) {
        let path = format!("/result/{}", encode_segment(id));
        let sent = self.send_signed(reqwest::Method::POST, &path, Some(result.to_string())).await;
        match sent {
            Ok(r) if r.status().is_success() => {}
            Ok(r) => tracing::warn!("[RelayAgent] result {} rejected: {}", id, r.status()),
            Err(e) => tracing::warn!("[RelayAgent] result {} failed: {}", id, e),
        }
    }
}

/// 올릴 이벤트면 대기열에 넣는다. 가장 오래된 것부터 버려 `EVENT_PENDING`건만 둔다
fn queue_event(pending: &mut std::collections::VecDeque<Value>, event: &crate::events::Event) {
    if let Some(v) = relay_event(event) {
        pending.push_back(v);
        while pending.len() > EVENT_PENDING {
            pending.pop_front();
        }
    }
}

/// 알림 채널(I3)에 올릴 모양 `{id, type, data}`. 올리지 않는 종류면 None.
/// 원격으로 나가므로 경로·콘솔 줄·오류 원문은 싣지 않는다
fn relay_event(event: &crate::events::Event) -> Option<Value> {
    let d = &event.data;
    let pick = |keys: &[&str]| -> serde_json::Map<String, Value> {
        keys.iter().filter_map(|k| d.get(*k).map(|v| (k.to_string(), v.clone()))).collect()
    };
    let data = match event.kind.as_str() {
        "instance.exit" => {
            let kind = d.get("kind").and_then(Value::as_str)?;
            if !["crash", "start_failed", "unknown_exit"].contains(&kind) {
                return None;
            }
            let mut v = pick(&["at", "instanceId", "instance", "kind", "exitCode", "signal", "restartInSecs", "note"]);
            // 종료 코드·신호·메모리 상한만. 콘솔 줄(`console: …`)과 크래시 파일 경로는 뺀다
            if let Some(evidence) = d.get("evidence").and_then(Value::as_str).filter(|e| crate::supervisor::crash::shareable_evidence(e)) {
                v.insert("evidence".into(), Value::from(evidence));
            }
            v
        }
        "backup.done" => pick(&["at", "instanceId", "instance", "kind", "ok", "name"]),
        "update.available" => pick(&["at", "components"]),
        "schedule.warn" => pick(&["at", "instanceId", "instance", "action", "minutesLeft"]),
        // 디스코드에서 시작해 뒤에서 마친 여러 단계 실행 (I1). 원격 실행의 단계 줄에는 오류 원문이 없다(코드만)
        "plan.done" if d.get("guildId").is_some() => pick(&["at", "runId", "guildId", "ok", "steps", "stoppedAt"]),
        _ => return None,
    };
    Some(json!({ "id": event.id, "type": event.kind, "data": data }))
}

/// `link_request` → 데몬 이벤트 `relay.link_request` 본문. 요청 id와 길드가 없으면 None
fn link_request_event(cmd: &Value, payload: &Value) -> Option<Value> {
    let text = |v: Option<&Value>| v.and_then(Value::as_str).map(str::trim).filter(|s| !s.is_empty()).map(str::to_string);
    let request_id = text(payload.get("requestId"))?;
    let guild_id = text(payload.get("guildId").or(cmd.get("guildId")))?;
    Some(json!({
        "requestId": request_id,
        "guildId": guild_id,
        "guildName": text(payload.get("guildName")),
        "requestedBy": text(cmd.get("requestedBy")),
        "userName": text(payload.get("userName")),
        "expiresAt": text(payload.get("expiresAt")),
    }))
}

/// poll 응답 본문. 릴레이가 이모지를 반으로 잘라 짝 없는 서로게이트(`\ud83d`)가 오면 serde_json이
/// 본문 전체를 거부해 그 묶음의 명령이 다 사라진다: 짝 없는 서로게이트만 U+FFFD로 바꿔 다시 읽는다
fn decode_poll(text: &str) -> Value {
    match serde_json::from_str(text) {
        Ok(v) => v,
        Err(e) => match serde_json::from_str(&replace_lone_surrogates(text)) {
            Ok(v) => {
                tracing::warn!("[RelayAgent] poll body had broken text ({}); decoded with replacement characters", e);
                v
            }
            Err(e) => {
                tracing::warn!("[RelayAgent] poll body is not JSON ({}), {} bytes dropped", e, text.len());
                Value::Null
            }
        },
    }
}

/// JSON 문자열의 `\uXXXX` 중 짝이 없는 서로게이트를 `\ufffd`로
fn replace_lone_surrogates(text: &str) -> String {
    let hex = |s: &str| u16::from_str_radix(s, 16).ok();
    let bytes = text.as_bytes();
    let mut out = String::with_capacity(text.len());
    let mut i = 0;
    let mut last = 0;
    while i < bytes.len() {
        if bytes[i] != b'\\' {
            i += 1;
            continue;
        }
        // 이스케이프 하나: `\\`는 건너뛴다
        if bytes.get(i + 1) != Some(&b'u') {
            i += 2;
            continue;
        }
        let Some(unit) = text.get(i + 2..i + 6).and_then(hex) else {
            i += 2;
            continue;
        };
        let low_next = || {
            (text.get(i + 6..i + 8) == Some("\\u")).then(|| text.get(i + 8..i + 12).and_then(hex)).flatten()
        };
        match unit {
            0xD800..=0xDBFF if low_next().is_some_and(|l| (0xDC00..=0xDFFF).contains(&l)) => i += 12,
            0xD800..=0xDFFF => {
                out.push_str(&text[last..i]);
                out.push_str("\\ufffd");
                i += 6;
                last = i;
            }
            _ => i += 6,
        }
    }
    out.push_str(&text[last..]);
    out
}

/// 릴레이의 액션 요청 한 건
async fn action_request(server: &crate::ipc::IPCServer, action: &str, payload: &Value, caller: &crate::actions::Caller) -> Result<Value, crate::actions::ActionError> {
    let id = payload.get("id").and_then(Value::as_str).unwrap_or_default();
    let instance = payload.get("instance").and_then(Value::as_str);
    let input = payload.get("input").cloned().unwrap_or(Value::Null);
    match action {
        // 여러 단계 계획 (I1)
        "plan" if payload.get("steps").is_some() => crate::actions::plan_steps(server, &payload["steps"], caller).await,
        "plan" => crate::actions::plan(server, id, instance, &input, caller).await,
        "run_plan" => {
            let plan_id = payload.get("planId").and_then(Value::as_str).unwrap_or_default();
            crate::actions::run(server, id, instance, &input, Some(plan_id), caller).await
        }
        _ => crate::actions::query(server, id, instance, &input, caller).await,
    }
}

/// 릴레이 결과 모양: 성공 `{success:true, data}`, 실패 `{success:false, data:{code, error, args?, field?, reason?}}`
fn action_result(result: Result<Value, crate::actions::ActionError>) -> Value {
    match result {
        Ok(v) => json!({ "success": v.get("success").and_then(Value::as_bool).unwrap_or(true), "data": v }),
        Err(e) => {
            let mut data = json!({ "code": e.code, "error": e.message });
            if let Value::Object(extra) = &e.data {
                for (k, v) in extra {
                    data[k] = v.clone();
                }
            }
            json!({ "success": false, "data": data })
        }
    }
}

/// 알아보지 못한 입력은 실패와 코드로 돌려준다: 릴레이는 그 코드를 보고 모델 해석기에 정확한 명령을 묻는다
fn raw_command_result(reply: crate::chat::Reply) -> Value {
    match reply.code {
        Some(code) => json!({ "success": false, "data": { "code": code, "error": reply.text, "args": reply.args } }),
        None => json!({ "success": true, "data": { "text": reply.text } }),
    }
}

fn backoff(failures: u32) -> Duration {
    let factor = 2u32.saturating_pow(failures.saturating_sub(1).min(16));
    (RETRY_BASE * factor).min(RETRY_MAX)
}

/// a < b (x.y.z, 빠진 자리는 0)
fn semver_lt(a: &str, b: &str) -> bool {
    let parse = |s: &str| -> Vec<u64> { s.split('.').map(|p| p.trim().parse().unwrap_or(0)).collect() };
    let (pa, pb) = (parse(a), parse(b));
    for i in 0..3 {
        let (x, y) = (pa.get(i).copied().unwrap_or(0), pb.get(i).copied().unwrap_or(0));
        if x != y {
            return x < y;
        }
    }
    false
}

/// encodeURIComponent와 같은 경로 조각 인코딩 (서명 경로와 실제 경로가 같아야 함)
fn encode_segment(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    for b in s.bytes() {
        if b.is_ascii_alphanumeric() || b"-_.!~*'()".contains(&b) {
            out.push(b as char);
        } else {
            out.push_str(&format!("%{:02X}", b));
        }
    }
    out
}

fn now_ms() -> u64 {
    std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_millis() as u64).unwrap_or(0)
}

/// 디스코드 `/사바쨩 권한 부여·해제·기본`: 노드 주인만 바꿀 수 있다. 챗봇 경로는 받지 않는다
async fn permission_action(server: &crate::ipc::IPCServer, who: &crate::chat::permissions::Requester, payload: &Value) -> Value {
    use crate::chat::permissions::Change;
    let fail = |code: &str| json!({ "success": false, "data": { "code": code } });
    let Some(guild) = who.guild_id.as_deref() else { return fail("MISSING_GUILD") };
    if who.via_chatbot || who.owner_id.is_none() || who.owner_id != who.user_id {
        return fail("NOT_OWNER");
    }
    let discord_id = payload.get("discordId").and_then(Value::as_str).unwrap_or("").to_string();
    let level = payload.get("level").and_then(Value::as_i64);
    let change = match payload.get("op").and_then(Value::as_str) {
        Some("grant") => match level {
            Some(l) => Change::Member { discord_id, level: Some(l) },
            None => return fail("INVALID_LEVEL"),
        },
        Some("revoke") => Change::Member { discord_id, level: None },
        Some("default_level") => match level {
            Some(l) => Change::DefaultLevel(l),
            None => return fail("INVALID_LEVEL"),
        },
        _ => return fail("UNKNOWN_OP"),
    };
    let owner = who.owner_id.clone();
    let result = server
        .config_store
        .update_node_settings(guild, |node| match crate::chat::permissions::apply(node, &change, owner.as_deref()) {
            Ok(()) => (Ok(node.clone()), true),
            Err(code) => (Err(code), false),
        })
        .await;
    match result {
        Ok(Ok(node)) => {
            let p = crate::chat::permissions::NodePermissions::from_node(Some(&Value::Object(node)));
            json!({ "success": true, "data": { "defaultLevel": p.default_level, "members": p.members } })
        }
        Ok(Err(code)) => fail(&code.to_uppercase()),
        Err(e) => json!({ "success": false, "data": { "code": "SAVE_FAILED", "error": e.to_string() } }),
    }
}

/// 길드별 `{defaultLevel, members}` (릴레이 바깥 검사 캐시용). `nodeSettings`에 있는 길드는 값이 없어도 늘 싣는다
/// (부여값을 다 지운 길드도 릴레이가 데몬 행을 지우게). 릴레이에서만 부여한 행은 릴레이가 남긴다
fn permissions_by_guild(node_settings: &Value) -> Value {
    let map: serde_json::Map<String, Value> = node_settings
        .as_object()
        .into_iter()
        .flatten()
        .map(|(guild, node)| {
            let p = crate::chat::permissions::NodePermissions::from_node(Some(node));
            (guild.clone(), json!({ "defaultLevel": p.default_level, "members": p.members }))
        })
        .collect();
    Value::Object(map)
}

/// `bot.node_settings[guild].allowedInstances` → `{ guild: [instance id, ...] }`. 하나도 없으면 None
fn allowed_instances_by_guild(node_settings: &Value) -> Option<Value> {
    let map: serde_json::Map<String, Value> = node_settings
        .as_object()?
        .iter()
        .filter_map(|(guild, cfg)| {
            let ids: Vec<Value> = cfg
                .get("allowedInstances")?
                .as_array()?
                .iter()
                .filter(|v| v.is_string())
                .cloned()
                .collect();
            Some((guild.clone(), Value::Array(ids)))
        })
        .collect();
    (!map.is_empty()).then_some(Value::Object(map))
}

/// 하트비트의 모듈 명령 한 줄: relay가 슬래시·챗봇 도구를 만드는 데 쓰는 필드만 (템플릿·엔드포인트는 PC 안에서만 쓴다).
/// `aliases`는 module.toml `[aliases.commands]`의 별칭 — 별칭으로 친 명령도 relay가 위험도 확인을 거치게 한다
fn relay_command_entry(c: &crate::supervisor::module_loader::CommandField, aliases: &[String]) -> Value {
    let inputs: Vec<Value> = c
        .inputs
        .iter()
        .map(|i| json!({ "name": i.name, "label": i.label, "type": i.input_type, "required": i.required, "min": i.min, "max": i.max, "options": i.options }))
        .collect();
    let mut entry = json!({ "name": c.name, "label": c.label, "description": c.description, "risk": c.risk, "inputs": inputs });
    if !aliases.is_empty() {
        entry["aliases"] = json!(aliases);
    }
    entry
}

/// 익스텐션 상태 줄 길이 상한 (글자)
const STATUS_LINE_MAX: usize = 100;

/// 하트비트의 서버 한 줄: relay가 읽는 필드만 싣는다. 설정값·경로·비밀번호는 PC 밖으로 나가지 않는다.
/// 실행 중이면 `usage{cpu, memMb}`와 모듈이 알려 준 `players{online, max?}`(Q-5), 익스텐션이 `server.list_enrich`에서 `status_line`을 주면 `statusLine`(X-1)
fn relay_server_entry(server: &Value, usage: Option<&crate::system_monitor::InstanceMetrics>) -> Value {
    let mut entry = serde_json::Map::new();
    for key in ["id", "name", "module", "status"] {
        if let Some(v) = server.get(key) {
            entry.insert(key.to_string(), v.clone());
        }
    }
    if let Some(u) = usage.filter(|u| u.running) {
        let cpu = (f64::from(u.cpu_percent) * 10.0).round() / 10.0;
        entry.insert("usage".into(), json!({ "cpu": cpu, "memMb": u.memory_bytes / (1024 * 1024) }));
    }
    let players = server.get("players").filter(|_| server.get("status").and_then(Value::as_str) == Some("running"));
    if let Some(count) = players.and_then(crate::supervisor::players::parse) {
        entry.insert("players".into(), json!(count));
    }
    // 익스텐션 id 순으로 첫 번째 상태 줄. 제어 문자를 빼고 한 줄로 자른다
    let line = server.get("extension_status").and_then(|v| v.as_object()).and_then(|statuses| {
        let mut ids: Vec<&String> = statuses.keys().collect();
        ids.sort();
        ids.into_iter().find_map(|id| {
            let text: String = statuses[id].get("status_line")?.as_str()?.chars().filter(|c| !c.is_control()).take(STATUS_LINE_MAX).collect();
            Some(text.trim().to_string()).filter(|t| !t.is_empty())
        })
    });
    if let Some(line) = line {
        entry.insert("statusLine".into(), json!(line));
    }
    Value::Object(entry)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn heartbeat_commands_leave_templates_on_the_pc() {
        let field: crate::supervisor::module_loader::CommandField = serde_json::from_value(json!({
            "name": "kick", "label": "Kick", "method": "console", "console_template": "kick {player}",
            "endpoint_template": "/v1/kick", "risk": "caution",
            "inputs": [{ "name": "player", "type": "text", "required": true }]
        }))
        .unwrap();
        let entry = relay_command_entry(&field, &["추방".to_string()]);
        assert_eq!(entry["name"], "kick");
        assert_eq!(entry["aliases"], json!(["추방"]));
        assert!(relay_command_entry(&field, &[]).get("aliases").is_none());
        assert_eq!(entry["inputs"][0]["name"], "player");
        assert!(entry.get("console_template").is_none() && entry.get("endpoint_template").is_none() && entry.get("method").is_none());
    }

    #[test]
    fn a_link_request_from_discord_only_reaches_the_pc_screen() {
        let cmd = json!({ "guildId": "g1", "requestedBy": "u1" });
        let payload = json!({ "action": "link_request", "requestId": "r1", "guildName": "길드", "userName": "주인", "expiresAt": "2026-10-18T08:00:00Z" });
        let data = link_request_event(&cmd, &payload).unwrap();
        assert_eq!(data, json!({ "requestId": "r1", "guildId": "g1", "guildName": "길드", "requestedBy": "u1", "userName": "주인", "expiresAt": "2026-10-18T08:00:00Z" }));
        assert!(link_request_event(&cmd, &json!({ "action": "link_request" })).is_none(), "no request id");
        assert!(link_request_event(&json!({}), &json!({ "requestId": "r1" })).is_none(), "no guild");
        let event = crate::events::Event { id: 1, kind: "relay.link_request".into(), data, replay: true };
        assert!(relay_event(&event).is_none(), "never sent back to the relay");
    }

    #[test]
    fn heartbeat_servers_carry_only_what_the_relay_reads() {
        let server = json!({
            "id": "i1", "name": "마크", "module": "minecraft", "status": "running",
            "executable_path": "C:/srv/java.exe", "working_dir": "C:/srv",
            "module_settings": { "rcon_secret": "x" }, "rest_password": "y"
        });
        assert_eq!(relay_server_entry(&server, None), json!({ "id": "i1", "name": "마크", "module": "minecraft", "status": "running" }));
    }

    #[test]
    fn heartbeat_servers_add_usage_and_an_extension_status_line() {
        let server = json!({
            "id": "i1", "name": "마크", "module": "minecraft", "status": "running",
            "extension_status": { "zz": { "status_line": "later" }, "docker": { "status_line": "  컨테이너 실행 중\n", "path": "C:/x" } },
            "players": { "online": 3, "max": 20 }
        });
        let usage = crate::system_monitor::InstanceMetrics {
            id: "i1".into(), name: "마크".into(), running: true, process_count: 1,
            cpu_percent: 12.345, memory_bytes: 3 * 1024 * 1024 * 1024, disk_read_bps: 0, disk_write_bps: 0,
        };
        let entry = relay_server_entry(&server, Some(&usage));
        assert_eq!(entry["usage"], json!({ "cpu": 12.3, "memMb": 3072 }));
        assert_eq!(entry["statusLine"], "컨테이너 실행 중");
        assert_eq!(entry["players"], json!({ "online": 3, "max": 20 }));
        let mut off = server.clone();
        off["status"] = json!("stopped");
        assert!(relay_server_entry(&off, None).get("players").is_none(), "a stopped server has no players");
        assert!(entry.get("extension_status").is_none());
        let stopped = crate::system_monitor::InstanceMetrics { running: false, ..usage };
        assert!(relay_server_entry(&server, Some(&stopped)).get("usage").is_none());
    }

    #[test]
    fn heartbeat_carries_permissions_for_every_known_guild() {
        let nodes = json!({
            "g1": { "members": { "a": 2, "bad": 7 }, "ownerDiscordId": "o" },
            "g2": { "ownerDiscordId": "o", "allowedInstances": ["x"] },
            "g3": { "defaultLevel": -1 }
        });
        assert_eq!(
            permissions_by_guild(&nodes),
            json!({
                "g1": { "defaultLevel": 1, "members": { "a": 2 } },
                "g2": { "defaultLevel": 1, "members": {} },
                "g3": { "defaultLevel": -1, "members": {} }
            })
        );
    }

    #[test]
    fn a_half_emoji_does_not_drop_the_whole_poll_batch() {
        let body = r#"{"commands":[{"id":"1","payload":{"text":"say \ud83d"}},{"id":"2","payload":{"text":"ok 😀 \\ud83d"}}]}"#;
        let v = decode_poll(body);
        let cmds = v["commands"].as_array().unwrap();
        assert_eq!(cmds.len(), 2);
        assert_eq!(cmds[0]["payload"]["text"], "say \u{fffd}");
        // 짝이 맞는 이모지와 이스케이프된 역슬래시 뒤의 글자는 그대로
        assert_eq!(cmds[1]["payload"]["text"], "ok 😀 \\ud83d");
        assert_eq!(decode_poll("not json"), Value::Null);
    }

    #[test]
    fn allowed_instances_are_sent_only_for_guilds_that_list_them() {
        let settings = json!({
            "g1": { "allowedInstances": ["i-mc", 3, "i-pal"] },
            "g2": { "memberPermissions": {} },
            "g3": { "allowedInstances": [] },
        });
        assert_eq!(
            allowed_instances_by_guild(&settings),
            Some(json!({ "g1": ["i-mc", "i-pal"], "g3": [] }))
        );
        assert_eq!(allowed_instances_by_guild(&json!({ "g2": {} })), None);
        assert_eq!(allowed_instances_by_guild(&json!(null)), None);
    }

    #[test]
    fn finished_old_session_leaves_new_session_alone() {
        let agent = RelayAgent::new();
        {
            let mut inner = agent.lock();
            inner.generation = 2;
            inner.cancel = Some(CancellationToken::new());
            inner.status.running = true;
        }
        agent.finish(1, Some("old".into()));
        assert!(agent.lock().cancel.is_some() && agent.lock().status.running);
        agent.finish(2, None);
        assert!(agent.lock().cancel.is_none() && !agent.lock().status.running);
    }

    /// 릴레이는 실패 + `data.code`(UNKNOWN_COMMAND 등)를 보고 해석기를 부른다. 문구는 `data.error`
    #[test]
    fn unrecognized_raw_commands_fail_with_a_code() {
        let unknown = crate::chat::Reply {
            text: "Unknown command".into(),
            code: Some("UNKNOWN_COMMAND"),
            args: json!({ "command": "dance" }),
        };
        assert_eq!(
            raw_command_result(unknown),
            json!({ "success": false, "data": { "code": "UNKNOWN_COMMAND", "error": "Unknown command", "args": { "command": "dance" } } })
        );
        assert_eq!(raw_command_result("done".to_string().into()), json!({ "success": true, "data": { "text": "done" } }));
    }

    #[test]
    fn only_notifications_go_to_the_relay_without_paths_or_console_lines() {
        let ev = |kind: &str, data: Value| crate::events::Event { id: 7, kind: kind.into(), data, replay: true };
        let crash = ev(
            "instance.exit",
            json!({"at": 1, "instanceId": "i", "instance": "마크", "kind": "crash", "exitCode": 1, "signal": null, "evidence": "crash file C:\\Users\\me\\crash.txt", "restartInSecs": 10, "note": null}),
        );
        let sent = relay_event(&crash).unwrap();
        assert_eq!(sent["type"], "instance.exit");
        assert_eq!(sent["id"], 7);
        assert_eq!(sent["data"]["restartInSecs"], 10);
        assert!(sent["data"].get("evidence").is_none());

        let coded = ev("instance.exit", json!({"kind": "unknown_exit", "evidence": "exit code 3"}));
        assert_eq!(relay_event(&coded).unwrap()["data"]["evidence"], "exit code 3");
        let console = ev("instance.exit", json!({"kind": "crash", "evidence": "console: joined from 10.0.0.2"}));
        assert!(relay_event(&console).unwrap()["data"].get("evidence").is_none());
        assert!(relay_event(&ev("instance.exit", json!({"kind": "user_stop"}))).is_none());

        let failed = ev("backup.done", json!({"instanceId": "i", "instance": "mc1", "kind": "manual", "ok": false, "error": "D:\\saves locked"}));
        assert_eq!(relay_event(&failed).unwrap()["data"], json!({"instanceId": "i", "instance": "mc1", "kind": "manual", "ok": false}));
        assert!(relay_event(&ev("action.run", json!({"action": "server.start"}))).is_none());
        assert!(relay_event(&ev("provision.progress", json!({}))).is_none());
        // 여러 단계 실행은 디스코드에서 시작한 것만
        assert!(relay_event(&ev("plan.done", json!({"runId": "r", "ok": true}))).is_none());
        assert_eq!(relay_event(&ev("plan.done", json!({"runId": "r", "guildId": "g", "ok": true}))).unwrap()["data"]["guildId"], "g");
    }

    #[test]
    fn semver_compare() {
        assert!(semver_lt("0.1.0", "0.1.1"));
        assert!(!semver_lt("0.1.1", "0.1.0"));
        assert!(!semver_lt("1.0", "1.0.0"));
        assert!(semver_lt("0.9.9", "1"));
    }

    #[test]
    fn backoff_doubles_up_to_cap() {
        assert_eq!(backoff(1), Duration::from_secs(3));
        assert_eq!(backoff(2), Duration::from_secs(6));
        assert_eq!(backoff(10), RETRY_MAX);
        assert_eq!(backoff(100), RETRY_MAX);
    }

    #[test]
    fn encodes_like_encode_uri_component() {
        assert_eq!(encode_segment("abc-123"), "abc-123");
        assert_eq!(encode_segment("a b/c"), "a%20b%2Fc");
    }
}
