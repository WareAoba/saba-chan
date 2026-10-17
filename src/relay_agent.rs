//! 릴레이 에이전트 — 클라우드 릴레이(오라클의 서버쨩 봇) ↔ 데몬 연결
//!
//! 디스코드 봇은 클라우드에서만 돌고, 데몬은 이 에이전트로 릴레이에 붙습니다.
//!   1. GET  /info              — 최소 에이전트 버전 확인
//!   2. POST /heartbeat (60초)  — 온라인 유지 + 서버/모듈/봇 설정 메타데이터 동기화
//!   3. GET  /poll (롱폴링)      — 디스코드에서 온 텍스트 명령 수신
//!   4. POST /result/:id        — 채팅 처리기(`crate::chat`) 결과 반환
//!
//! 인증은 노드 토큰(`<data_dir>/.node_token`, sbn_<id>.<secret>) HMAC 서명입니다.
//! 새 명령 종류는 `handle_payload`의 action 분기에 추가합니다.

use crate::ipc::handlers::relay::{load_node_token, parse_node_token, resolve_relay_url, signed_headers};
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
const SLEEP_CHECK_INTERVAL: Duration = Duration::from_secs(30);

/// 에이전트 상태 (GET /api/relay/agent)
#[derive(Debug, Clone, Default, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct AgentStatus {
    pub running: bool,
    /// 릴레이가 이 호스트를 수면 상태로 둠 (폴링 일시정지)
    pub sleeping: bool,
    pub relay_url: Option<String>,
    pub has_token: bool,
    pub agent_version: &'static str,
    /// 마지막 하트비트 성공 시각 (epoch ms)
    pub last_heartbeat_at: Option<u64>,
    pub last_error: Option<String>,
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
        if load_node_token().as_deref().and_then(parse_node_token).is_none() {
            return Err("node token missing or malformed — pair this node first".into());
        }
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
            let session = Session { agent: agent.clone(), server, client, base: relay_url, cancel: cancel.clone() };
            if let Err(e) = session.check_version().await {
                tracing::error!("[RelayAgent] {}", e);
                agent.finish(generation, Some(e));
                return;
            }
            tracing::info!("[RelayAgent] started (relay={})", session.base);
            tokio::join!(session.heartbeat_loop(), session.poll_loop());
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
        inner.status.sleeping = false;
    }

    /// 세션이 끝났을 때. 그 사이 새 세션이 시작됐으면(세대가 다르면) 아무것도 건드리지 않는다
    fn finish(&self, generation: u64, error: Option<String>) {
        let mut inner = self.lock();
        if inner.generation != generation {
            return;
        }
        inner.cancel = None;
        inner.status.running = false;
        inner.status.sleeping = false;
        if error.is_some() {
            inner.status.last_error = error;
        }
    }

    fn lock(&self) -> std::sync::MutexGuard<'_, Inner> {
        self.inner.lock().unwrap_or_else(|e| e.into_inner())
    }

    fn update(&self, f: impl FnOnce(&mut AgentStatus)) {
        f(&mut self.lock().status)
    }
}

/// 데몬 기동 시: 이 노드가 페어링돼 있으면(노드 토큰 존재) 에이전트를 띄운다
pub async fn auto_start(server: &IPCServer) {
    if let Err(e) = server.relay_agent.start(server.clone()).await {
        tracing::info!("[RelayAgent] not started: {}", e);
    }
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
    cancel: CancellationToken,
}

impl Session {
    /// 서명된 요청 생성. body는 서명한 문자열 그대로 보낸다.
    fn signed(&self, method: reqwest::Method, path: &str, body: Option<String>) -> Result<reqwest::RequestBuilder, String> {
        let headers = signed_headers(method.as_str(), path, body.as_deref()).ok_or("node token missing")?;
        let mut req = self.client.request(method, format!("{}{}", self.base, path));
        for (k, v) in headers {
            req = req.header(k, v);
        }
        if let Some(b) = body {
            req = req.header("Content-Type", "application/json").body(b);
        }
        Ok(req)
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
            let resp = self.signed(reqwest::Method::POST, "/heartbeat", Some(body.to_string()))?.send().await.map_err(|e| e.to_string())?;
            let status = resp.status();
            let data: Value = resp.json().await.unwrap_or(Value::Null);
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
                let sleeping = data.get("sleeping").and_then(|v| v.as_bool()).unwrap_or(false);
                self.agent.update(|s| {
                    if s.sleeping != sleeping {
                        tracing::info!("[RelayAgent] relay {}", if sleeping { "sleeping — polling paused" } else { "awake — polling resumed" });
                    }
                    s.sleeping = sleeping;
                    s.last_heartbeat_at = Some(now_ms());
                    s.last_error = None;
                });
            }
            Err(e) => {
                tracing::warn!("[RelayAgent] {}", e);
                self.agent.update(|s| s.last_error = Some(e));
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
            Some(Value::Array(list)) => list
                .iter()
                .map(|s| {
                    let mut s = s.clone();
                    if let Some(obj) = s.as_object_mut() {
                        // 게임 서버 비밀번호는 릴레이에 보낼 이유가 없다
                        obj.remove("rcon_password");
                        obj.remove("rest_password");
                    }
                    s
                })
                .collect(),
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
                    "commands": { "fields": m.commands },
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

        json!({
            "servers": servers,
            "modules": modules,
            "moduleDetails": {},
            "botConfig": {
                "prefix": bot.prefix,
                "moduleAliases": bot.module_aliases,
                "commandAliases": command_aliases,
            },
        })
    }

    async fn poll_loop(&self) {
        let mut failures: u32 = 0;
        while !self.cancel.is_cancelled() {
            if self.agent.status().sleeping {
                self.pause(SLEEP_CHECK_INTERVAL).await;
                continue;
            }
            match self.poll_once().await {
                Ok(()) => failures = 0,
                Err(e) => {
                    failures += 1;
                    let backoff = backoff(failures);
                    tracing::warn!("[RelayAgent] poll failed ({}), retry in {}s", e, backoff.as_secs());
                    self.agent.update(|s| s.last_error = Some(e));
                    self.pause(backoff).await;
                }
            }
        }
    }

    async fn pause(&self, d: Duration) {
        tokio::select! {
            _ = self.cancel.cancelled() => {}
            _ = tokio::time::sleep(d) => {}
        }
    }

    async fn poll_once(&self) -> Result<(), String> {
        let req = self.signed(reqwest::Method::GET, "/poll", None)?;
        let resp = tokio::select! {
            _ = self.cancel.cancelled() => return Ok(()),
            r = req.send() => r.map_err(|e| e.to_string())?,
        };
        let status = resp.status();
        if status.as_u16() == 204 {
            return Ok(());
        }
        let body: Value = resp.json().await.unwrap_or(Value::Null);
        if !status.is_success() {
            return Err(format!("poll {}: {}", status, body.get("error").and_then(|v| v.as_str()).unwrap_or("")));
        }
        if body.get("sleeping").and_then(|v| v.as_bool()) == Some(true) {
            self.agent.update(|s| s.sleeping = true);
            return Ok(());
        }
        let commands = body.get("commands").and_then(|v| v.as_array()).cloned().unwrap_or_default();
        for cmd in commands {
            let Some(id) = cmd.get("id").and_then(|v| v.as_str()) else { continue };
            let result = self.handle_payload(&cmd).await;
            self.post_result(id, &result).await;
            self.agent.update(|s| s.commands_handled += 1);
        }
        Ok(())
    }

    async fn handle_payload(&self, cmd: &Value) -> Value {
        let payload = cmd.get("payload").cloned().unwrap_or(Value::Null);
        let action = payload.get("action").and_then(|v| v.as_str()).unwrap_or("");
        match action {
            "raw_command" => {
                let text = payload.get("text").and_then(|v| v.as_str()).unwrap_or("");
                let req = crate::chat::ChatRequest {
                    text: text.to_string(),
                    guild_id: cmd.get("guildId").and_then(|v| v.as_str()).map(str::to_string),
                    user_id: cmd.get("requestedBy").and_then(|v| v.as_str()).map(str::to_string),
                };
                let reply = crate::chat::api::run_with_daemon_settings(&self.server, &req).await;
                json!({ "success": true, "data": { "text": reply } })
            }
            other => json!({ "success": false, "data": { "error": format!("Unknown action: {}", other) } }),
        }
    }

    async fn post_result(&self, id: &str, result: &Value) {
        let path = format!("/result/{}", encode_segment(id));
        let sent = async { self.signed(reqwest::Method::POST, &path, Some(result.to_string()))?.send().await.map_err(|e| e.to_string()) }.await;
        match sent {
            Ok(r) if r.status().is_success() => {}
            Ok(r) => tracing::warn!("[RelayAgent] result {} rejected: {}", id, r.status()),
            Err(e) => tracing::warn!("[RelayAgent] result {} failed: {}", id, e),
        }
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

#[cfg(test)]
mod tests {
    use super::*;

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
