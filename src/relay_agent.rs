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
            let session = Session { agent: agent.clone(), server, client, base: relay_url, cancel: cancel.clone(), generation };
            if let Err(e) = session.check_version().await {
                tracing::error!("[RelayAgent] {}", e);
                agent.finish(generation, Some(e));
                return;
            }
            tracing::info!("[RelayAgent] started (relay={})", session.base);
            // 챗봇 설정은 데몬이 원본: 연결될 때마다 릴레이에 맞춘다
            let sync_server = session.server.clone();
            tokio::spawn(async move { crate::ipc::handlers::chatbot::sync_on_connect(&sync_server).await });
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

    /// 세션 상태 갱신. 끝난(세대가 지난) 세션의 늦은 응답은 버린다
    fn update(&self, generation: u64, f: impl FnOnce(&mut AgentStatus)) {
        let mut inner = self.lock();
        if inner.generation == generation && inner.cancel.is_some() {
            f(&mut inner.status)
        }
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
    generation: u64,
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
                self.agent.update(self.generation, |s| {
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

        let mut meta = json!({
            "servers": servers,
            "modules": modules,
            "moduleDetails": {},
            "botConfig": {
                "prefix": bot.prefix,
                "moduleAliases": bot.module_aliases,
                "commandAliases": command_aliases,
            },
        });
        // 길드별 허용 인스턴스. 목록이 있는 길드만 싣는다 (없으면 릴레이는 제한 없음으로 본다)
        if let Some(allowed) = allowed_instances_by_guild(&bot.node_settings) {
            meta["allowedInstances"] = allowed;
        }
        // 권한 원본은 데몬이다: 릴레이는 이 값으로 바깥 검사용 캐시를 맞춘다
        meta["permissions"] = permissions_by_guild(&bot.node_settings);
        meta
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
                    self.agent.update(self.generation, |s| s.last_error = Some(e));
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
        let text = tokio::select! {
            _ = self.cancel.cancelled() => return Ok(()),
            b = resp.text() => b.unwrap_or_default(),
        };
        let body = decode_poll(&text);
        if !status.is_success() {
            return Err(format!("poll {}: {}", status, body.get("error").and_then(|v| v.as_str()).unwrap_or("")));
        }
        if body.get("sleeping").and_then(|v| v.as_bool()) == Some(true) {
            self.agent.update(self.generation, |s| s.sleeping = true);
            return Ok(());
        }
        let commands = body.get("commands").and_then(|v| v.as_array()).cloned().unwrap_or_default();
        for cmd in commands {
            // 연결 해제(취소) 뒤에는 이미 받은 명령도 실행하지 않는다
            if self.cancel.is_cancelled() {
                break;
            }
            let Some(id) = cmd.get("id").and_then(|v| v.as_str()) else { continue };
            let result = self.handle_payload(&cmd).await;
            self.post_result(id, &result).await;
            self.agent.update(self.generation, |s| s.commands_handled += 1);
        }
        Ok(())
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

/// 알아보지 못한 입력은 실패와 코드로 돌려준다: 릴레이는 그 코드를 보고 모델 해석기에 정확한 명령을 묻는다
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

/// 길드별 `{defaultLevel, members}` (릴레이 바깥 검사 캐시용). 부여값을 가진 길드만
fn permissions_by_guild(node_settings: &Value) -> Value {
    let map: serde_json::Map<String, Value> = node_settings
        .as_object()
        .into_iter()
        .flatten()
        // 데몬이 아직 부여값을 갖지 않은 길드는 싣지 않는다 (릴레이에만 있던 부여값을 지우지 않게)
        .filter(|(_, node)| crate::chat::permissions::configured(Some(node)))
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn heartbeat_carries_permissions_only_for_guilds_the_daemon_configured() {
        let nodes = json!({
            "g1": { "members": { "a": 2, "bad": 7 }, "ownerDiscordId": "o" },
            "g2": { "ownerDiscordId": "o", "allowedInstances": ["x"] },
            "g3": { "defaultLevel": -1 }
        });
        assert_eq!(
            permissions_by_guild(&nodes),
            json!({ "g1": { "defaultLevel": 1, "members": { "a": 2 } }, "g3": { "defaultLevel": -1, "members": {} } })
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
