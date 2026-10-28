//! 릴레이 에이전트 — 클라우드 릴레이(오라클의 서버쨩 봇) ↔ 데몬 연결
//!
//! 디스코드 봇은 클라우드에서만 돌고, 데몬은 이 에이전트로 릴레이에 붙습니다.
//!   1. GET  /info              — 최소 에이전트 버전 확인
//!   2. POST /heartbeat (60초)  — 온라인 유지 + 서버/모듈/봇 설정 메타데이터 동기화
//!   3. GET  /poll (롱폴링)      — 디스코드에서 온 텍스트 명령 수신
//!   4. POST /ack               — 받은 명령 id 알림 (릴레이는 ack한 호스트에만 답 없는 명령을 다시 보낸다)
//!   5. POST /result/:id        — 채팅 처리기(`crate::chat`) 결과 반환
//!   6. POST /events            — 디스코드 알림 채널(I3)에 띄울 이벤트 (`relay_event`)
//!
//! 인증은 노드 토큰(`config.json` `node.token`, sbn_<id>.<secret>) HMAC 서명입니다.
//!   7. `web_session` 명령    — 사바쨩 웹 세션이 열렸다: `crate::remote::link`가 릴레이 웹소켓에 붙는다
//!      `remote_tunnel` 명령  — 같은 주인의 PC가 터널을 열었다: `crate::remote_daemon::tunnel::accept`가 원격 포트에 잇는다
//!
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
/// 릴레이가 이 PC의 버전을 받지 않는다 (최소 버전 미달). 업데이트 전에는 다시 시도해도 같다
const TOO_OLD: &str = "agent_too_old";
/// 동시에 처리하는 릴레이 명령 수 (넘으면 끝나기를 기다렸다가 더 받는다)
const COMMANDS_IN_FLIGHT: usize = 4;
/// 받은 명령 id를 기억하는 기간과 개수. 릴레이가 같은 명령을 다시 보내도 한 번만 실행한다
const SEEN_FOR: Duration = Duration::from_secs(600);
const SEEN_MAX: usize = 1000;
/// 결과 전송(`POST /result/:id`)이 일시적으로 실패했을 때 다시 보내는 횟수. 그사이 그 명령의 실행 자리를 쥔다
const RESULT_RETRIES: u32 = 5;

/// 받지 않은 결과를 이만큼 들고 있으면 새 명령을 시작하지 않는다. 받아 둔 명령은 ack하지 않아 릴레이에 남는다
const UNSENT_MAX: usize = 100;

/// 결과를 다시 보내기 전 기다림: 1·2·4·8·16초 (시험에서는 짧게)
fn result_retry_delay(attempt: u32) -> Duration {
    let unit = if cfg!(test) { Duration::from_millis(10) } else { Duration::from_secs(1) };
    unit * 2u32.pow(attempt.saturating_sub(1).min(4))
}

/// `POST /ack` 한 번에 싣는 id 수 (릴레이 상한)
const ACK_BATCH: usize = 50;

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
    /// `agent_too_old`일 때 릴레이가 요구하는 최소 버전
    #[serde(skip_serializing_if = "Option::is_none")]
    pub min_version: Option<String>,
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
    /// 받은 명령 id. 다시 연결해도(토큰·주소 교체, 연결 해제 뒤 연결) 이어서 쓴다: 대기열에 남은 명령이 다시 와도 두 번 실행하지 않게
    seen: Arc<Mutex<SeenCommands>>,
    /// 다시 보내도 릴레이가 받지 않은 결과. 같은 주소·토큰의 세션이 이어서 보낸다
    unsent: Arc<Mutex<Vec<Unsent>>>,
    /// 보내는 중인 명령 결과 수. 데몬이 꺼질 때 이것이 끝나기를 잠깐 기다린다 (원격 업데이트 뒤 재시작 안내 등)
    posting: Arc<std::sync::atomic::AtomicUsize>,
}

/// 보내는 중인 결과 하나 (끝나면 수를 줄인다)
struct Posting(Arc<std::sync::atomic::AtomicUsize>);

impl Drop for Posting {
    fn drop(&mut self) {
        self.0.fetch_sub(1, std::sync::atomic::Ordering::SeqCst);
    }
}

/// 릴레이가 아직 받지 않은 명령 결과. 만든 세션의 주소·토큰을 함께 둔다: 다른 노드로 보내지 않게
struct Unsent {
    base: String,
    token: String,
    id: String,
    body: String,
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
    pub async fn start(&self, server: IPCServer) -> Result<(), StartError> {
        // 세션은 시작할 때의 주소·토큰 쌍만 쓴다: 주소나 토큰이 바뀌면 새 세션이 시작된다
        let Some(node_token) = load_node_token().filter(|t| parse_node_token(t).is_some()) else {
            return Err(StartError { code: "node_token_missing", message: "node token missing or malformed — pair this node first".into() });
        };
        let relay_url = match session_relay_url(&server.config_store.get_bot_config().await.cloud.relay_url) {
            Ok(url) => url,
            Err(e) => {
                // 돌던 세션은 없다: 상태에 이유를 남겨 클라이언트가 보이게 한다
                let mut inner = self.lock();
                if inner.cancel.is_none() {
                    inner.status.last_error = Some(e.message.clone());
                    inner.status.last_error_code = Some(e.code);
                }
                return Err(e);
            }
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
            let session = Session {
                agent: agent.clone(),
                server,
                client,
                base: relay_url,
                token: node_token,
                cancel: cancel.clone(),
                generation,
                seen: agent.seen.clone(),
                ack_supported: std::sync::atomic::AtomicBool::new(true),
            };
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
            tokio::join!(session.heartbeat_loop(), session.poll_loop(), session.events_loop(), session.unsent_loop());
            agent.finish(generation, None);
            tracing::info!("[RelayAgent] stopped");
        });
        Ok(())
    }

    /// 보내는 중인 명령 결과가 끝나기를 `max`까지 기다린다 (데몬 종료 직전). 다 끝났으면 true
    pub async fn settle(&self, max: std::time::Duration) -> bool {
        let until = tokio::time::Instant::now() + max;
        while self.posting.load(std::sync::atomic::Ordering::SeqCst) > 0 {
            if tokio::time::Instant::now() >= until {
                return false;
            }
            tokio::time::sleep(std::time::Duration::from_millis(50)).await;
        }
        true
    }

    pub fn stop(&self) {
        crate::remote::link::stop();
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
        tracing::info!("[RelayAgent] not started: {}", e.message);
    }
}

/// 에이전트를 띄우지 못한 이유. `code`는 API의 `error_code`다
#[derive(Debug)]
pub struct StartError {
    pub code: &'static str,
    pub message: String,
}

/// 세션의 릴레이 주소. 노드 토큰이 평문으로 나가지 않게 https(또는 루프백 http 모의 릴레이)만 쓴다.
/// 저장할 때 막는 규칙(`relay_url_problem`)을 예전에 저장된 값에도 적용한다
fn session_relay_url(configured: &str) -> Result<String, StartError> {
    match crate::config_store::relay_url_problem(configured) {
        Some(message) => Err(StartError { code: "invalid_relay_url", message }),
        None => Ok(resolve_relay_url(Some(configured.trim()))),
    }
}

/// relay가 이 노드 토큰을 같은 계정의 새 페어링으로 대체했다고 답했는가
fn token_replaced(status: reqwest::StatusCode, body: &Value) -> bool {
    status == reqwest::StatusCode::UNAUTHORIZED
        && body.get("error").and_then(|v| v.as_str()).is_some_and(|e| e.eq_ignore_ascii_case("TOKEN_REPLACED"))
}

/// relay가 이 PC의 버전이 최소 버전보다 낮다고 거절했는가 (426 `AGENT_TOO_OLD`). 요구하는 최소 버전을 돌려준다
fn agent_too_old(status: reqwest::StatusCode, body: &Value) -> Option<String> {
    let rejected = status == reqwest::StatusCode::UPGRADE_REQUIRED
        && body.get("error").and_then(|v| v.as_str()).is_some_and(|e| e.eq_ignore_ascii_case("AGENT_TOO_OLD"));
    rejected.then(|| body.get("minVersion").and_then(|v| v.as_str()).unwrap_or("").to_string())
}

/// 받기(poll)가 실패한 뒤 다시 묻기까지. 버전 미달은 업데이트 전에는 같은 답이라 heartbeat 주기로만 묻는다
fn poll_retry(error: &str, failures: u32) -> Duration {
    if error == TOO_OLD {
        HEARTBEAT_INTERVAL
    } else {
        backoff(failures)
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
    token: String,
    cancel: CancellationToken,
    generation: u64,
    seen: Arc<Mutex<SeenCommands>>,
    /// 릴레이가 `/ack`를 모르면(404, 예전 릴레이) 이 세션에서는 더 보내지 않는다
    ack_supported: std::sync::atomic::AtomicBool,
}

/// 받은 명령을 처음 본 것인지, 아직 처리 중인지, 이미 끝났는지
#[derive(Debug, Clone, PartialEq)]
enum Received {
    New,
    /// 받아 두고 아직 시작하지 않았다
    Waiting,
    Running,
    Done(Value),
}

/// 최근에 받은 명령 id. 끝난 명령은 결과도 기억해 다시 보낼 수 있게 한다 (`SEEN_FOR` 동안, 최대 `SEEN_MAX`개).
/// 기다리거나 실행 중인 명령은 끝날 때까지(또는 세션이 끝나 `forget`할 때까지) 기억한다
#[derive(Default)]
struct SeenCommands {
    entries: std::collections::VecDeque<Seen>,
}

struct Seen {
    id: String,
    at: std::time::Instant,
    started: bool,
    result: Option<Value>,
}

impl SeenCommands {
    /// 받은 id를 기록하고, 이 명령을 어떻게 다룰지 돌려준다. 처음 본 id는 처리 중으로 기록한다
    /// 끝난 명령만 시간·개수로 잊는다. 기다리거나 실행 중인 명령은 작업량 상한 안에 있고, 잊으면 다시 오는 같은 명령을 또 실행한다
    fn receive(&mut self, id: &str, now: std::time::Instant) -> Received {
        self.entries.retain(|e| e.result.is_none() || now.saturating_duration_since(e.at) < SEEN_FOR);
        if let Some(e) = self.entries.iter().find(|e| e.id == id) {
            return match &e.result {
                Some(r) => Received::Done(r.clone()),
                None if e.started => Received::Running,
                None => Received::Waiting,
            };
        }
        self.entries.push_back(Seen { id: id.to_string(), at: now, started: false, result: None });
        let mut over = self.entries.len().saturating_sub(SEEN_MAX);
        self.entries.retain(|e| {
            let drop = over > 0 && e.result.is_some();
            over -= usize::from(drop);
            !drop
        });
        Received::New
    }

    /// 세션이 끝나 시작하지 않고 버린 명령: 다시 오면 다음 세션이 실행한다
    fn forget(&mut self, ids: &[&str]) {
        self.entries.retain(|e| !ids.contains(&e.id.as_str()));
    }

    fn start(&mut self, id: &str) {
        if let Some(e) = self.entries.iter_mut().find(|e| e.id == id) {
            e.started = true;
        }
    }

    /// 끝난 명령의 보관 기간은 끝난 때부터 센다. 받은 때부터 세면 오래 기다린 명령을 끝나자마자 잊는다
    fn finish(&mut self, id: &str, result: &Value) {
        if let Some(e) = self.entries.iter_mut().find(|e| e.id == id) {
            e.at = std::time::Instant::now();
            e.result = Some(result.clone());
        }
    }
}

/// 시작한 명령 id로 만든 `POST /ack` 본문들 (`ACK_BATCH`개씩)
fn ack_bodies(commands: &[Value]) -> Vec<String> {
    let ids: Vec<&str> = commands.iter().filter_map(|c| c.get("id").and_then(Value::as_str)).collect();
    ids.chunks(ACK_BATCH).map(|chunk| json!({ "ids": chunk }).to_string()).collect()
}

impl Session {
    /// 서명된 요청 생성. body는 서명한 문자열 그대로 보낸다.
    fn signed(&self, method: reqwest::Method, path: &str, body: Option<String>) -> Result<reqwest::RequestBuilder, String> {
        // 끝난 세션(연결 해제·주소나 토큰 교체)은 새로 아무것도 보내지 않는다 (예외: 이미 시작한 명령의 결과)
        if self.cancel.is_cancelled() {
            return Err("relay session ended".into());
        }
        self.request(method, path, body)
    }

    /// 이 세션의 주소·토큰으로 서명한 요청
    fn request(&self, method: reqwest::Method, path: &str, body: Option<String>) -> Result<reqwest::RequestBuilder, String> {
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
        crate::remote::link::stop();
        self.cancel.cancel();
    }

    /// 릴레이가 이 버전을 받지 않는다: 클라이언트가 업데이트를 안내하게 코드와 최소 버전을 남긴다
    fn report_too_old(&self, min: &str) {
        tracing::warn!("[RelayAgent] relay requires agent {} or later (this is {}) — update saba-chan", min, AGENT_VERSION);
        self.agent.update(self.generation, |s| {
            s.last_error = Some(format!("agent {} is older than relay minimum {} — update saba-chan", AGENT_VERSION, min));
            s.last_error_code = Some(TOO_OLD);
            s.min_version = Some(min.to_string());
        });
    }

    /// 상태에 마지막 오류를 남긴다. 세션이 끝난 뒤(연결 해제·토큰 대체)에는 끝난 이유를 덮어쓰지 않는다
    fn report_error(&self, e: String) {
        if !self.cancel.is_cancelled() {
            self.agent.update(self.generation, |s| s.last_error = Some(e));
        }
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
                self.report_too_old(min);
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
            if let Some(min) = agent_too_old(status, &data) {
                self.report_too_old(&min);
                return Err(TOO_OLD.to_string());
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
                    s.last_error_code = None;
                    s.min_version = None;
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
            // 이미 상태에 남겼다
            Err(e) if e == TOO_OLD => {}
            Err(e) => {
                tracing::warn!("[RelayAgent] {}", e);
                self.report_error(e);
            }
        }
    }

    /// 하트비트 메타데이터: 릴레이 봇이 자동완성·접두사 표시에 쓴다
    async fn metadata(&self) -> Value {
        use crate::chat::api::DaemonApi;
        let translate = self.server.config_store.get_bot_config().await.translate_commands;
        let servers: Vec<Value> = match crate::ipc::internal::into_internal(
            crate::ipc::handlers::server::list_servers_internal(&self.server).await,
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
        // 모듈 locales 파일은 하트비트마다 읽으므로 async 작업자 밖에서 읽는다
        let views = self.server.modules().await;
        let modules: Vec<Value> = tokio::task::spawn_blocking(move || views
            .into_iter()
            .map(|m| {
                // 번역한 명령 이름 (봇 설정 `translateCommands`, 기본 켬): 모듈 locales를 언어마다 한 번 읽는다
                let locales: std::collections::HashMap<&str, Value> = match (translate, &m.locale_dir) {
                    (true, Some(dir)) => crate::chat::i18n::languages()
                        .into_iter()
                        .filter_map(|l| {
                            let raw = std::fs::read_to_string(dir.join("locales").join(format!("{}.json", l))).ok()?;
                            Some((l, serde_json::from_str(&raw).ok()?))
                        })
                        .collect(),
                    _ => Default::default(),
                };
                let texts = |l: &str| locales.get(l).cloned().unwrap_or(Value::Null);
                let texts: Option<&dyn Fn(&str) -> Value> = if translate { Some(&texts) } else { None };
                json!({
                    "name": m.name,
                    // 게임명도 별명처럼 서버를 부른다: 릴레이가 위험 명령 확인에서 같은 서버를 찾도록 함께 보낸다
                    "module_aliases": m.game_names.iter().chain(&m.module_aliases).collect::<Vec<_>>(),
                    // 디스코드·챗봇에 내보내지 않는 명령(`remote = false`)은 싣지 않는다
                    "commands": { "fields": m.commands.iter().filter(|c| c.remote).map(|c| relay_command_entry(c, m.aliases_of(&c.name), texts)).collect::<Vec<_>>() },
                    "presets": m.presets.iter().map(|p| json!({ "id": p.id, "label": p.label })).collect::<Vec<_>>(),
                })
            })
            .collect())
        .await
        .unwrap_or_else(|e| if e.is_panic() { std::panic::resume_unwind(e.into_panic()) } else { Vec::new() });

        // 권한을 읽기 직전의 시각 (권한 변경 응답의 `permissionsAt`과 같은 PC 시계)
        let permissions_at = now_ms();
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
        // 길드별 켜짐 알림을 켠 서버 (`/api/instance/:id/start-notice/:guild`). 켠 길드만 싣는다
        if let Some(on) = ids_by_guild(&bot.node_settings, crate::ipc::handlers::start_notice::KEY) {
            meta["startNotice"] = on;
        }
        // 인스턴스별로 서버와 명령을 부르는 낱말 (어느 길드에서나 같다, 릴레이는 찾기·위험 확인에 쓴다).
        // 비어도 싣는다: 이 키가 있으면 릴레이는 옛 길드별 `instanceWords`·전역 별명을 보지 않는다
        meta["serverWords"] = crate::chat::words::heartbeat(&bot.server_words);
        // 권한 원본은 데몬이다: 릴레이는 이 값으로 바깥 검사용 캐시를 맞춘다
        meta["permissions"] = permissions_by_guild(&bot.node_settings);
        meta["permissionsAt"] = json!(permissions_at);
        // 되묻기는 데몬이 계획마다 정한다 (`actions::needs_confirm`, 문장 계획 포함): 릴레이는 스스로 위험을 판정하지 않고 `needsConfirm`을 따른다
        meta["confirmByDaemon"] = Value::Bool(true);
        // `/사바쨩 상태` 끝 줄의 다음 할 일 (J1-a, GUI·CLI와 같은 단계)
        meta["onboarding"] = crate::ipc::handlers::diagnostics::onboarding_now(&self.server).await;
        // 언어가 `auto`인 길드는 이 언어로 답한다 (I2)
        // (릴레이 스키마는 10자까지. 넘으면 heartbeat 전체가 거절되므로 싣지 않는다)
        if let Some(language) = Some(self.server.config_store.pc_language().await).filter(|l| l.len() <= 10) {
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
        type Side<'a> = std::pin::Pin<Box<dyn std::future::Future<Output = ()> + Send + 'a>>;
        let mut running = futures_util::stream::FuturesUnordered::new();
        // ack와 다시 온 명령의 결과 재전송: 명령 자리를 차지하지 않고 따로 돈다
        let mut side: futures_util::stream::FuturesUnordered<Side<'_>> = futures_util::stream::FuturesUnordered::new();
        let mut waiting: Vec<Value> = Vec::new();
        let mut next: Option<Poll<'_>> = Some(Box::pin(self.poll_once()));
        while !self.cancel.is_cancelled() {
            // 시작하는 명령만 ack한다: 받아 두고 시작하지 못한 명령은 연결이 끝나면 릴레이가 다음 연결로 다시 보낸다
            let mut started = Vec::new();
            let backlogged = self.unsent_full();
            while running.len() < COMMANDS_IN_FLIGHT && !waiting.is_empty() && !backlogged {
                let cmd = waiting.remove(0);
                if let Some(id) = cmd.get("id").and_then(Value::as_str) {
                    self.seen.lock().unwrap_or_else(|e| e.into_inner()).start(id);
                }
                started.push(json!({ "id": cmd.get("id").cloned().unwrap_or(Value::Null) }));
                running.push(self.run_command(cmd));
            }
            for body in ack_bodies(&started) {
                side.push(Box::pin(self.post_ack(body)));
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
                            let now = std::time::Instant::now();
                            let mut reack = Vec::new();
                            for cmd in commands {
                                let Some(id) = cmd.get("id").and_then(Value::as_str).map(str::to_string) else { continue };
                                // 결과를 아직 보내지 못한 명령: 다시 실행하지 않는다 (`unsent_loop`가 결과를 보낸다)
                                if self.has_unsent(&id) {
                                    reack.push(json!({ "id": id }));
                                    continue;
                                }
                                let received = self.seen.lock().unwrap_or_else(|e| e.into_inner()).receive(&id, now);
                                match received {
                                    Received::New => waiting.push(cmd),
                                    // 같은 명령을 다시 받았다: 실행하지 않는다. 실행 중이면 ack가 닿지 않은 것이니 다시 ack하고,
                                    // 끝난 것이면 결과만 다시 보낸다
                                    Received::Waiting => {}
                                    Received::Running => reack.push(json!({ "id": id })),
                                    Received::Done(result) => side.push(Box::pin(async move { self.post_result(&id, &result).await })),
                                }
                            }
                            for body in ack_bodies(&reack) {
                                side.push(Box::pin(self.post_ack(body)));
                            }
                        }
                        Err(e) if e == REPLACED => self.give_up_replaced(),
                        Err(e) => {
                            failures += 1;
                            let backoff = poll_retry(&e, failures);
                            tracing::warn!("[RelayAgent] poll failed ({}), retry in {}s", e, backoff.as_secs());
                            if e != TOO_OLD {
                                self.report_error(e);
                            }
                            next = Some(Box::pin(async move {
                                self.pause(backoff).await;
                                self.poll_once().await
                            }));
                        }
                    }
                }
                Some(()) = running.next(), if !running.is_empty() => {}
                Some(()) = side.next(), if !side.is_empty() => {}
                // 받지 않은 결과가 줄었는지 가끔 본다 (그동안 받아 둔 명령은 시작하지 않는다)
                _ = self.pause(result_retry_delay(RESULT_RETRIES)), if backlogged && !waiting.is_empty() => {}
                _ = self.cancel.cancelled() => {}
            }
        }
        // 연결 해제(취소) 뒤에는 받아 두고 시작하지 않은 명령은 바로 잊어 다음 연결이 다시 받으면 실행하게 하고,
        // 이미 시작한 것은 끝까지 마쳐 결과를 올린다
        {
            let ids: Vec<&str> = waiting.iter().filter_map(|c| c.get("id").and_then(Value::as_str)).collect();
            self.seen.lock().unwrap_or_else(|e| e.into_inner()).forget(&ids);
        }
        while running.next().await.is_some() {}
    }

    async fn run_command(&self, cmd: Value) {
        let Some(id) = cmd.get("id").and_then(Value::as_str).map(str::to_string) else { return };
        let result = self.handle_payload(&cmd).await;
        // 결과를 올리지 못해도 기억해 둔다: 릴레이가 명령을 다시 보내면 이 결과를 다시 올린다
        self.seen.lock().unwrap_or_else(|e| e.into_inner()).finish(&id, &result);
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
        if let Some(min) = agent_too_old(status, &body) {
            self.report_too_old(&min);
            return Err(TOO_OLD.to_string());
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
                let via_chatbot = req.remote.as_ref().is_some_and(|w| w.via_chatbot);
                let reply = crate::audit::with_origin(origin, crate::chat::api::run_with_daemon_settings(&self.server, &req)).await;
                raw_command_result(reply, via_chatbot)
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
                origin.via = who.via_chatbot.then(|| "chatbot".to_string());
                let caller = crate::actions::Caller {
                    surface: if who.via_chatbot { crate::actions::Surface::Chatbot } else { crate::actions::Surface::Discord },
                    remote: Some(who),
                    origin: origin.clone(),
                    locale: text("locale").unwrap_or_else(|| "en".into()),
                };
                let result = crate::audit::with_origin(origin, action_request(&self.server, action, &payload, &caller)).await;
                action_result(result, &caller.locale)
            }
            // 사바쨩 웹: 릴레이에 웹 세션이 열렸다. 릴레이 웹소켓에 붙는다 (세션이 없어지면 데몬이 끊는다)
            "web_session" => {
                crate::remote::link::ensure(self.server.clone(), &self.base, &self.token);
                json!({ "success": true, "data": { "connecting": true } })
            }
            // 원격 데몬 터널 (T1): 같은 주인의 PC가 릴레이를 거쳐 이 PC의 원격 포트에 붙는다
            "remote_tunnel" => {
                let id = payload.get("tunnelId").and_then(Value::as_str).unwrap_or("");
                if !crate::remote_daemon::relay::valid_ticket_id(id) {
                    return json!({ "success": false, "data": { "error": "invalid remote_tunnel" } });
                }
                if !crate::remote_daemon::status().await.listening {
                    return json!({ "success": false, "data": { "error": "remote_off" } });
                }
                let port = crate::remote_daemon::config::settings_in(&saba_chan_updater_lib::config_file::ConfigFile::default_location()).port;
                let relay = crate::remote_daemon::relay::Target { base: self.base.clone(), token: self.token.clone() };
                tokio::spawn(crate::remote_daemon::tunnel::accept(relay, id.to_string(), port));
                json!({ "success": true, "data": { "connecting": true } })
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
            confirmed_by: payload.get("confirmedBy").and_then(Value::as_str).map(str::to_string),
            guild_owner_id: cmd.get("guildOwnerDiscordId").and_then(Value::as_str).filter(|s| !s.is_empty()).map(str::to_string),
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

    /// 받은 명령을 알린다(`POST /ack`). 릴레이는 ack한 호스트에만 답 없는 명령을 다시 보낸다
    async fn post_ack(&self, body: String) {
        use std::sync::atomic::Ordering;
        if !self.ack_supported.load(Ordering::Relaxed) {
            return;
        }
        match self.send_signed(reqwest::Method::POST, "/ack", Some(body)).await {
            Ok(r) if r.status().is_success() => {}
            Ok(r) if r.status().as_u16() == 404 => {
                tracing::info!("[RelayAgent] relay has no /ack; not acking in this session");
                self.ack_supported.store(false, Ordering::Relaxed);
            }
            Ok(r) => tracing::warn!("[RelayAgent] ack rejected: {}", r.status()),
            Err(e) => tracing::warn!("[RelayAgent] ack failed: {}", e),
        }
    }

    async fn post_result(&self, id: &str, result: &Value) {
        // 실행한 명령의 결과는 연결이 끝난 뒤에도 끝까지 보낸다: ack한 명령이라 릴레이가 다시 보내 주지 않는다.
        // 연결 오류·5xx·408·429는 `RESULT_RETRIES`번까지 간격을 늘려 다시 보낸다. 그 밖의 4xx는 릴레이가 받지 않는 것이라 그만둔다
        let body = result.to_string();
        self.agent.posting.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
        let _posting = Posting(self.agent.posting.clone());
        for attempt in 0..=RESULT_RETRIES {
            if attempt > 0 {
                tokio::time::sleep(result_retry_delay(attempt)).await;
            }
            if self.send_result(id, &body, attempt).await {
                return;
            }
        }
        // 명령 자리는 놓고, 결과는 남겨 같은 주소·토큰의 세션이 이어서 보낸다
        tracing::warn!("[RelayAgent] result {} not delivered after {} tries; keeping it", id, RESULT_RETRIES + 1);
        self.keep_unsent(id, body);
    }

    /// 결과를 한 번 보낸다. 다시 보낼 일이 없으면(받았거나, 릴레이가 그 명령을 몰라 받지 않으면) `true`
    async fn send_result(&self, id: &str, body: &str, attempt: u32) -> bool {
        let path = format!("/result/{}", encode_segment(id));
        let sent = match self.request(reqwest::Method::POST, &path, Some(body.to_string())) {
            Ok(req) => req.send().await.map_err(|e| e.to_string()),
            Err(e) => Err(e),
        };
        let status = match sent {
            Ok(r) if r.status().is_success() => return true,
            Ok(r) => r.status(),
            Err(e) => {
                tracing::warn!("[RelayAgent] result {} failed (try {}): {}", id, attempt + 1, e);
                return false;
            }
        };
        let retry = status.is_server_error() || matches!(status.as_u16(), 408 | 429);
        tracing::warn!("[RelayAgent] result {} rejected (try {}): {}", id, attempt + 1, status);
        !retry
    }

    fn unsent(&self) -> std::sync::MutexGuard<'_, Vec<Unsent>> {
        self.agent.unsent.lock().unwrap_or_else(|e| e.into_inner())
    }

    fn keep_unsent(&self, id: &str, body: String) {
        let mut unsent = self.unsent();
        if !unsent.iter().any(|u| u.id == id && u.base == self.base && u.token == self.token) {
            unsent.push(Unsent { base: self.base.clone(), token: self.token.clone(), id: id.to_string(), body });
        }
    }

    fn has_unsent(&self, id: &str) -> bool {
        self.unsent().iter().any(|u| u.id == id && u.base == self.base && u.token == self.token)
    }

    fn unsent_full(&self) -> bool {
        self.unsent().len() >= UNSENT_MAX
    }

    /// 받지 않은 결과를 세션이 끝날 때까지 간격을 두고 다시 보낸다. 명령은 다시 실행하지 않는다.
    /// 다른 주소·토큰(다른 노드)이 만든 결과는 버린다
    async fn unsent_loop(&self) {
        while !self.cancel.is_cancelled() {
            self.pause(result_retry_delay(RESULT_RETRIES)).await;
            let due: Vec<(String, String)> = {
                let mut unsent = self.unsent();
                // 끝난 세션은 대기열을 정리하지 않는다: 기다리는 사이 시작한 새 세션(다른 주소·토큰)의 결과를 지우지 않게.
                // 잠근 채 보므로 새 세션은 이 정리가 끝난 뒤에야 결과를 넣는다 (새 세션은 이 세션을 끝낸 뒤 시작한다)
                if self.cancel.is_cancelled() {
                    break;
                }
                unsent.retain(|u| u.base == self.base && u.token == self.token);
                unsent.iter().map(|u| (u.id.clone(), u.body.clone())).collect()
            };
            for (id, body) in due {
                if self.cancel.is_cancelled() {
                    break;
                }
                if !self.send_result(&id, &body, RESULT_RETRIES + 1).await {
                    // 못 보낸 결과는 맨 뒤로 보낸다: 한 결과만 계속 거절돼도 다음 회차엔 뒤 결과부터 보낸다
                    let mut unsent = self.unsent();
                    if let Some(i) = unsent.iter().position(|u| u.id == id && u.base == self.base && u.token == self.token) {
                        let failed = unsent.remove(i);
                        unsent.push(failed);
                    }
                    break;
                }
                self.unsent().retain(|u| u.id != id || u.base != self.base || u.token != self.token);
            }
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
        "instance.ready" => pick(&["at", "instanceId", "instance", "ok", "code"]),
        // 서버가 켜지고 꺼진 것과 시킨 쪽. 사바쨩 밖의 시작은 unexpected (크래시·원인 모를 종료는 instance.exit)
        "instance.started" => pick(&["at", "instanceId", "instance", "source", "userName", "unexpected"]),
        "instance.stopped" => pick(&["at", "instanceId", "instance", "source", "userName", "kind"]),
        // 켠 서버의 모듈 새 버전·게임 서버 새 빌드 (시작을 시킨 사람에게만 알린다)
        "instance.update_notice" => pick(&["at", "instanceId", "instance", "kind", "module", "latest"]),
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
        // 문장 명령 ("terra-a 공지 안녕"): 데몬이 해석한다
        "plan" if payload.get("text").is_some() => {
            crate::actions::plan_text(server, payload["text"].as_str().unwrap_or_default(), caller).await
        }
        "plan" => crate::actions::plan(server, id, instance, &input, caller).await,
        "run_plan" => {
            let plan_id = payload.get("planId").and_then(Value::as_str).unwrap_or_default();
            crate::actions::run(server, id, instance, &input, Some(plan_id), caller).await
        }
        _ => crate::actions::query(server, id, instance, &input, caller).await,
    }
}

/// 릴레이 결과 모양: 성공 `{success:true, data}`, 실패 `{success:false, data:{code, error, args?, field?, reason?}}`
/// `locale`: 요청 언어. 문구가 있는 실패 코드면 짧은 이유(`reason`)를 함께 준다
fn action_result(result: Result<Value, crate::actions::ActionError>, locale: &str) -> Value {
    let mut out = match result {
        Ok(v) => json!({ "success": v.get("success").and_then(Value::as_bool).unwrap_or(true), "data": v }),
        Err(e) => {
            let mut data = json!({ "code": e.code, "error": e.message });
            if let Some(reason) = crate::chat::known_reason(&crate::chat::i18n::Catalog::for_language(locale), e.code, &e.data) {
                data["reason"] = json!(reason);
            }
            if let Value::Object(extra) = &e.data {
                for (k, v) in extra {
                    data[k] = v.clone();
                }
            }
            json!({ "success": false, "data": data })
        }
    };
    crate::actions::strip_local_details(&mut out);
    out
}

/// 알아보지 못한 입력은 실패와 코드로 돌려준다: 릴레이는 그 코드를 보고 모델 해석기에 정확한 명령을 묻는다.
/// 답 문구는 실행 오류 원문을 담을 수 있어 액션 결과와 같은 기준으로 로컬 정보를 뺀다 (I3).
/// 챗봇이 부른 것이면 모듈 답의 IP·비밀처럼 보이는 값도 가린다 (액션 경로의 콘솔·조회 결과와 같다)
fn raw_command_result(reply: crate::chat::Reply, via_chatbot: bool) -> Value {
    let mut out = match reply.code {
        Some(code) => json!({ "success": false, "data": { "code": code, "error": reply.text, "args": reply.args } }),
        None => json!({ "success": true, "data": { "text": reply.text } }),
    };
    crate::actions::strip_local_details(&mut out);
    if via_chatbot {
        crate::actions::mask_console(&mut out);
    }
    out
}

fn backoff(failures: u32) -> Duration {
    let factor = 2u32.saturating_pow(failures.saturating_sub(1).min(16));
    (RETRY_BASE * factor).min(RETRY_MAX)
}

/// a < b (x.y.z, 빠진 자리는 0). 프리릴리스(0.1.11-beta.1)는 업데이터와 같은 SemVer 순서로 비교한다
fn semver_lt(a: &str, b: &str) -> bool {
    use saba_chan_updater_lib::version::SemVer;
    if let (Some(x), Some(y)) = (SemVer::parse(a), SemVer::parse(b)) {
        return x < y;
    }
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
            // 저장을 마친 뒤의 시각: 릴레이는 이보다 먼저 읽은 heartbeat 권한을 버린다
            let mut data = permission_entry(&p);
            data["permissionsAt"] = json!(now_ms());
            json!({ "success": true, "data": data })
        }
        Ok(Err(code)) => fail(&code.to_uppercase()),
        Err(e) => json!({ "success": false, "data": { "code": "SAVE_FAILED", "error": e.to_string() } }),
    }
}

/// 길드당 싣는 명령별 덮어쓰기 수. 넘으면 키 순으로 자르고 `commandLevelsTruncated`를 둔다 (릴레이는 데몬 판정에 맡긴다)
const COMMAND_LEVELS_PER_GUILD: usize = 200;

/// 길드별 `{defaultLevel, members, chatbotCap, levels, commandLevels}` (릴레이 바깥 검사용). `nodeSettings`에 있는 길드는
/// 값이 없어도 늘 싣는다 (부여값을 다 지운 길드도 릴레이가 데몬 행을 지우게). 릴레이에서만 부여한 행은 릴레이가 남긴다.
/// `levels`는 그 길드 프리셋으로 푼 risk별 요구 레벨이다. 최종 판정은 데몬이 다시 한다
fn permissions_by_guild(node_settings: &Value) -> Value {
    let map: serde_json::Map<String, Value> = node_settings
        .as_object()
        .into_iter()
        .flatten()
        .map(|(guild, node)| (guild.clone(), permission_entry(&crate::chat::permissions::NodePermissions::from_node(Some(node)))))
        .collect();
    Value::Object(map)
}

/// 한 길드의 권한 스냅샷 (heartbeat `permissions[guild]`, `permission` action 응답이 같은 모양)
fn permission_entry(p: &crate::chat::permissions::NodePermissions) -> Value {
    let mut keys: Vec<&String> = p.command_levels.keys().collect();
    keys.sort();
    let command_levels: serde_json::Map<String, Value> =
        keys.iter().take(COMMAND_LEVELS_PER_GUILD).map(|k| ((*k).clone(), json!(p.command_levels[*k]))).collect();
    let levels = json!({
        "safe": p.preset_need("safe"),
        "caution": p.preset_need("caution"),
        "danger": p.preset_need("danger"),
        // 관리(Kind::Admin) 액션은 risk 대신 이 레벨이다 (`actions::authorize`)
        "admin": p.admin_need(),
    });
    let mut entry = json!({
        "defaultLevel": p.default_level,
        "members": p.members,
        "chatbotCap": p.chatbot_cap,
        "levels": levels,
        "commandLevels": command_levels,
        // 단계 순서 (높은 것부터). 릴레이는 숫자가 아니라 이 순서로 견준다
        "ladder": p.ladder,
    });
    if keys.len() > COMMAND_LEVELS_PER_GUILD {
        entry["commandLevelsTruncated"] = json!(true);
    }
    entry
}

/// `bot.node_settings[guild].allowedInstances` → `{ guild: [instance id, ...] }`. 하나도 없으면 None
fn allowed_instances_by_guild(node_settings: &Value) -> Option<Value> {
    ids_by_guild(node_settings, "allowedInstances")
}

/// `bot.node_settings[guild].<key>`의 인스턴스 ID 목록 → `{ guild: [instance id, ...] }`. 목록이 있는 길드가 없으면 None
fn ids_by_guild(node_settings: &Value, key: &str) -> Option<Value> {
    let map: serde_json::Map<String, Value> = node_settings
        .as_object()?
        .iter()
        .filter_map(|(guild, cfg)| {
            let ids: Vec<Value> = cfg
                .get(key)?
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
/// `texts`: 언어 → 모듈 locales (번역한 명령 이름이 켜져 있을 때). 있으면 `commands.<이름>.label`·`description`과
/// 칸의 `inputs.<칸>.label`을 언어별 `titles`·`descriptions`로 싣는다 (릴레이 슬래시 명령의 언어별 이름·설명)
fn relay_command_entry(c: &crate::supervisor::module_loader::CommandField, aliases: &[String], texts: Option<&dyn Fn(&str) -> Value>) -> Value {
    let by_language = |pointer: &str| -> Option<serde_json::Map<String, Value>> {
        let texts = texts?;
        let map: serde_json::Map<String, Value> = crate::chat::i18n::languages()
            .into_iter()
            .filter_map(|l| Some((l.to_string(), json!(texts(l).pointer(pointer)?.as_str()?))))
            .collect();
        (!map.is_empty()).then_some(map)
    };
    let inputs: Vec<Value> = c
        .inputs
        .iter()
        .map(|i| {
            let mut input = json!({ "name": i.name, "label": i.label, "type": i.input_type, "required": i.required, "min": i.min, "max": i.max, "options": i.options });
            if let Some(titles) = by_language(&format!("/commands/{}/inputs/{}/label", c.name, i.name)) {
                input["titles"] = Value::Object(titles);
            }
            input
        })
        .collect();
    let mut entry = json!({ "name": c.name, "label": c.label, "description": c.description, "risk": c.risk, "inputs": inputs });
    if !aliases.is_empty() {
        entry["aliases"] = json!(aliases);
    }
    for (key, text) in [("titles", "label"), ("descriptions", "description")] {
        if let Some(map) = by_language(&format!("/commands/{}/{}", c.name, text)) {
            entry[key] = Value::Object(map);
        }
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

    /// 릴레이가 이 PC의 버전을 받지 않으면(최소 버전 미달) 상태에 `agent_too_old`와 요구 버전을 남겨
    /// GUI·CLI가 업데이트를 안내하게 하고, 받기는 업데이트 전에는 같은 답이므로 heartbeat 주기로만 다시 묻는다
    #[tokio::test]
    async fn a_relay_that_needs_a_newer_agent_is_reported_and_asked_slowly() {
        use axum::routing::{get, post};
        async fn too_old() -> (axum::http::StatusCode, axum::Json<Value>) {
            let body = json!({ "error": "AGENT_TOO_OLD", "minVersion": "99.0.0", "message": "Update Saba-chan to 99.0.0 or later." });
            (axum::http::StatusCode::UPGRADE_REQUIRED, axum::Json(body))
        }
        let app = axum::Router::new()
            .route("/info", get(|| async { axum::Json(json!({ "minAgentVersion": "99.0.0" })) }))
            .route("/heartbeat", post(too_old))
            .route("/poll", get(too_old));
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let base = format!("http://{}", listener.local_addr().unwrap());
        tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });

        let tmp = tempfile::tempdir().unwrap();
        let mut sup = crate::supervisor::Supervisor::new_with_instances_dir(
            tmp.path().join("modules").to_str().unwrap(),
            tmp.path().join("instances").to_str().unwrap(),
        );
        sup.initialize().await.unwrap();
        let server = crate::ipc::IPCServer::new(std::sync::Arc::new(tokio::sync::RwLock::new(sup)), "127.0.0.1:0", crate::daemon_log::DaemonLogBuffer::new());
        let agent = RelayAgent::new();
        let cancel = CancellationToken::new();
        {
            let mut inner = agent.lock();
            inner.cancel = Some(cancel.clone());
            inner.generation = 1;
            inner.status.running = true;
        }
        let session = Session {
            agent: agent.clone(),
            server,
            client: http_client().unwrap(),
            base,
            token: "sbn_host.secret".into(),
            cancel,
            generation: 1,
            seen: Arc::default(),
            ack_supported: std::sync::atomic::AtomicBool::new(true),
        };
        let reset = || agent.update(1, |s| *s = AgentStatus { running: true, ..Default::default() });
        let reported = |what: &str| {
            let status = agent.status();
            assert_eq!(status.last_error_code, Some("agent_too_old"), "{what}: {status:?}");
            assert_eq!(status.min_version.as_deref(), Some("99.0.0"), "{what}");
            let json = serde_json::to_value(&status).unwrap();
            assert_eq!((json["lastErrorCode"].clone(), json["minVersion"].clone()), (json!("agent_too_old"), json!("99.0.0")));
        };

        assert!(session.check_version().await.is_err());
        reported("/info");
        reset();
        session.heartbeat().await;
        reported("heartbeat");
        reset();
        let polled = session.poll_once().await.unwrap_err();
        reported("poll");
        assert_eq!(poll_retry(&polled, 1), HEARTBEAT_INTERVAL);
        assert_eq!(poll_retry("poll 502 Bad Gateway: ", 1), backoff(1), "other failures keep the short backoff");
    }

    /// 외부 감사: ack한 명령의 결과 전송이 한 번 실패하면(503 등) 릴레이가 다시 보내 주지 않아 결과가 사라졌다.
    /// 일시적인 실패는 몇 번 다시 보내고, 릴레이가 그 명령을 모르면(4xx) 그만둔다
    #[tokio::test]
    async fn a_result_the_relay_did_not_take_is_sent_again() {
        use axum::routing::post;
        use std::sync::atomic::{AtomicUsize, Ordering};
        static CALLS: AtomicUsize = AtomicUsize::new(0);
        async fn result(axum::extract::Path(id): axum::extract::Path<String>) -> axum::http::StatusCode {
            let n = CALLS.fetch_add(1, Ordering::SeqCst);
            match id.as_str() {
                "gone" => axum::http::StatusCode::NOT_FOUND,
                _ if n < 2 => axum::http::StatusCode::SERVICE_UNAVAILABLE,
                _ => axum::http::StatusCode::OK,
            }
        }
        let app = axum::Router::new().route("/result/{id}", post(result));
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let base = format!("http://{}", listener.local_addr().unwrap());
        tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });

        let tmp = tempfile::tempdir().unwrap();
        let mut sup = crate::supervisor::Supervisor::new_with_instances_dir(
            tmp.path().join("modules").to_str().unwrap(),
            tmp.path().join("instances").to_str().unwrap(),
        );
        sup.initialize().await.unwrap();
        let server = crate::ipc::IPCServer::new(std::sync::Arc::new(tokio::sync::RwLock::new(sup)), "127.0.0.1:0", crate::daemon_log::DaemonLogBuffer::new());
        let session = Session {
            agent: RelayAgent::new(),
            server,
            client: http_client().unwrap(),
            base,
            token: "sbn_host.secret".into(),
            cancel: CancellationToken::new(),
            generation: 1,
            seen: Arc::default(),
            ack_supported: std::sync::atomic::AtomicBool::new(true),
        };
        session.post_result("c1", &json!({ "success": true })).await;
        assert_eq!(CALLS.swap(0, Ordering::SeqCst), 3, "two 503s, then taken");
        session.post_result("gone", &json!({ "success": true })).await;
        assert_eq!(CALLS.load(Ordering::SeqCst), 1, "a command the relay no longer has is not sent again");
        assert!(session.agent.unsent.lock().unwrap().is_empty(), "a refused result is not kept");
    }

    /// 시험용 세션: 주어진 에이전트·주소·토큰으로
    async fn test_session(agent: &RelayAgent, base: &str, token: &str, tmp: &std::path::Path) -> Session {
        let mut sup = crate::supervisor::Supervisor::new_with_instances_dir(
            tmp.join("modules").to_str().unwrap(),
            tmp.join("instances").to_str().unwrap(),
        );
        sup.initialize().await.unwrap();
        let server = crate::ipc::IPCServer::new(std::sync::Arc::new(tokio::sync::RwLock::new(sup)), "127.0.0.1:0", crate::daemon_log::DaemonLogBuffer::new());
        Session {
            agent: agent.clone(),
            server,
            client: http_client().unwrap(),
            base: base.into(),
            token: token.into(),
            cancel: CancellationToken::new(),
            generation: 1,
            seen: Arc::default(),
            ack_supported: std::sync::atomic::AtomicBool::new(true),
        }
    }

    /// `unsent_loop`를 돌리다가 `done`이 참이 되거나 `wait`이 지나면 세션을 끝낸다
    async fn run_unsent(session: &Session, wait: Duration, done: impl Fn() -> bool) {
        tokio::join!(session.unsent_loop(), async {
            let until = tokio::time::Instant::now() + wait;
            while !done() && tokio::time::Instant::now() < until {
                tokio::time::sleep(Duration::from_millis(10)).await;
            }
            session.cancel.cancel();
        });
    }

    /// 외부 감사 10-23: 결과 전송이 여섯 번 다 실패하면 결과를 버렸다. ack한 명령이라 릴레이가 다시 보내 주지 않는다.
    /// 받지 않은 결과는 남겨 두고, 릴레이가 살아나면 명령을 다시 실행하지 않고 같은 결과를 보낸다
    #[tokio::test]
    async fn a_result_kept_after_every_retry_failed_is_sent_once_the_relay_recovers() {
        use axum::routing::post;
        use std::sync::atomic::{AtomicUsize, Ordering};
        static CALLS: AtomicUsize = AtomicUsize::new(0);
        static BODY: std::sync::Mutex<String> = std::sync::Mutex::new(String::new());
        async fn result(body: String) -> axum::http::StatusCode {
            if CALLS.fetch_add(1, Ordering::SeqCst) < 7 {
                return axum::http::StatusCode::SERVICE_UNAVAILABLE;
            }
            *BODY.lock().unwrap() = body;
            axum::http::StatusCode::OK
        }
        let app = axum::Router::new().route("/result/{id}", post(result));
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let base = format!("http://{}", listener.local_addr().unwrap());
        tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });

        let tmp = tempfile::tempdir().unwrap();
        let agent = RelayAgent::new();
        let session = test_session(&agent, &base, "sbn_host.secret", tmp.path()).await;
        session.post_result("c1", &json!({ "success": true, "n": 1 })).await;
        assert_eq!(CALLS.load(Ordering::SeqCst), (RESULT_RETRIES + 1) as usize);
        assert_eq!(agent.unsent.lock().unwrap().len(), 1, "kept after the last retry");
        run_unsent(&session, Duration::from_secs(5), || agent.unsent.lock().unwrap().is_empty()).await;
        assert!(agent.unsent.lock().unwrap().is_empty(), "sent once the relay took it");
        assert_eq!(CALLS.load(Ordering::SeqCst), 8);
        assert_eq!(serde_json::from_str::<Value>(&BODY.lock().unwrap()).unwrap(), json!({ "success": true, "n": 1 }));
    }

    /// Codex 리뷰 #553: 원격 업데이트 뒤 데몬이 0.5초 만에 꺼져, 첫 전송이 실패한 결과(재시작 안내)를 다시 보내기 전에 잃었다.
    /// 종료 전에 보내는 중인 결과가 끝나기를 기다린다
    #[tokio::test]
    async fn shutdown_waits_for_a_result_still_being_sent() {
        use axum::routing::post;
        use std::sync::atomic::{AtomicUsize, Ordering};
        static CALLS: AtomicUsize = AtomicUsize::new(0);
        async fn result() -> axum::http::StatusCode {
            if CALLS.fetch_add(1, Ordering::SeqCst) == 0 {
                return axum::http::StatusCode::SERVICE_UNAVAILABLE;
            }
            axum::http::StatusCode::OK
        }
        let app = axum::Router::new().route("/result/{id}", post(result));
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let base = format!("http://{}", listener.local_addr().unwrap());
        tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });

        let tmp = tempfile::tempdir().unwrap();
        let agent = RelayAgent::new();
        let session = Arc::new(test_session(&agent, &base, "sbn_host.secret", tmp.path()).await);
        assert!(agent.settle(Duration::from_millis(10)).await, "nothing to wait for");
        let sending = session.clone();
        let task = tokio::spawn(async move { sending.post_result("c1", &json!({ "success": true })).await });
        tokio::time::sleep(Duration::from_millis(5)).await;
        assert!(agent.settle(Duration::from_secs(5)).await);
        assert_eq!(CALLS.load(Ordering::SeqCst), 2, "the retry went out before settle returned");
        task.await.unwrap();
    }

    /// Codex 리뷰 #549: 대기열 맨 앞 결과만 계속 5xx를 받으면 매 회차 거기서 멈춰 뒤 결과를 보내지 않았다
    #[tokio::test]
    async fn one_result_the_relay_keeps_refusing_does_not_block_the_others() {
        use axum::routing::post;
        use std::sync::atomic::{AtomicUsize, Ordering};
        static OTHER: AtomicUsize = AtomicUsize::new(0);
        async fn result(axum::extract::Path(id): axum::extract::Path<String>) -> axum::http::StatusCode {
            if id == "c1" {
                return axum::http::StatusCode::INTERNAL_SERVER_ERROR;
            }
            OTHER.fetch_add(1, Ordering::SeqCst);
            axum::http::StatusCode::OK
        }
        let app = axum::Router::new().route("/result/{id}", post(result));
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let base = format!("http://{}", listener.local_addr().unwrap());
        tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });

        let tmp = tempfile::tempdir().unwrap();
        let agent = RelayAgent::new();
        let session = test_session(&agent, &base, "sbn_host.secret", tmp.path()).await;
        session.keep_unsent("c1", "{}".into());
        session.keep_unsent("c2", "{}".into());
        run_unsent(&session, Duration::from_secs(5), || OTHER.load(Ordering::SeqCst) > 0).await;
        assert_eq!(OTHER.load(Ordering::SeqCst), 1, "the result behind the refused one is sent");
        assert!(session.has_unsent("c1") && !session.has_unsent("c2"));
    }

    /// 받지 않은 결과는 그 결과를 만든 주소·토큰의 다음 세션만 보낸다. 다른 토큰(다른 노드)의 세션은 버린다
    #[tokio::test]
    async fn a_kept_result_goes_only_to_the_node_that_ran_the_command() {
        use axum::routing::post;
        use std::sync::atomic::{AtomicUsize, Ordering};
        static CALLS: AtomicUsize = AtomicUsize::new(0);
        let app = axum::Router::new().route(
            "/result/{id}",
            post(|| async {
                CALLS.fetch_add(1, Ordering::SeqCst);
                axum::http::StatusCode::OK
            }),
        );
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let base = format!("http://{}", listener.local_addr().unwrap());
        tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });

        let tmp = tempfile::tempdir().unwrap();
        let agent = RelayAgent::new();
        let first = test_session(&agent, &base, "sbn_host.secret", tmp.path()).await;
        first.keep_unsent("c1", "{}".into());
        first.cancel.cancel();
        // 같은 주소·토큰으로 다시 연결: 보낸다
        let again = test_session(&agent, &base, "sbn_host.secret", tmp.path()).await;
        run_unsent(&again, Duration::from_secs(5), || CALLS.load(Ordering::SeqCst) > 0).await;
        assert_eq!(CALLS.swap(0, Ordering::SeqCst), 1);
        // 다른 토큰·다른 주소: 보내지 않고 버린다
        first.keep_unsent("c2", "{}".into());
        let other_node = test_session(&agent, &base, "sbn_other.secret", tmp.path()).await;
        run_unsent(&other_node, result_retry_delay(RESULT_RETRIES) * 3, || false).await;
        let other_relay = test_session(&agent, "http://127.0.0.1:9", "sbn_host.secret", tmp.path()).await;
        run_unsent(&other_relay, result_retry_delay(RESULT_RETRIES) * 3, || false).await;
        assert_eq!(CALLS.load(Ordering::SeqCst), 0);
        assert!(agent.unsent.lock().unwrap().is_empty(), "another node's result is dropped");
    }

    /// 외부 감사: 끝난 세션의 `unsent_loop`가 기다림에서 깨어나 대기열을 정리하면서, 그 사이 시작한 새 세션(다른 토큰)의 결과를 지웠다
    #[tokio::test]
    async fn an_ended_session_does_not_drop_the_next_sessions_kept_results() {
        let tmp = tempfile::tempdir().unwrap();
        let agent = RelayAgent::new();
        let old = test_session(&agent, "http://127.0.0.1:9", "sbn_host.secret", tmp.path()).await;
        let new = test_session(&agent, "http://127.0.0.1:9", "sbn_other.secret", tmp.path()).await;
        // 옛 세션은 첫 차례에 기다림에 들어간다. 새 세션이 결과를 남긴 뒤 옛 세션이 끝난다
        tokio::join!(old.unsent_loop(), async {
            new.keep_unsent("c1", "{}".into());
            old.cancel.cancel();
        });
        assert!(new.has_unsent("c1"), "the next session's result is kept");
    }

    /// 결과를 아직 보내지 못한 명령을 릴레이가 다시 보내도(기억한 id가 지난 뒤에도) 다시 실행하지 않고 ack만 한다
    #[tokio::test]
    async fn a_command_whose_result_is_kept_is_not_run_again() {
        use axum::routing::{get, post};
        use std::sync::atomic::{AtomicUsize, Ordering};
        static RESULTS: AtomicUsize = AtomicUsize::new(0);
        static ACKS: AtomicUsize = AtomicUsize::new(0);
        let app = axum::Router::new()
            .route(
                "/poll",
                get(|| async {
                    tokio::time::sleep(Duration::from_millis(20)).await;
                    axum::Json(json!({ "commands": [{ "id": "c1", "payload": {} }] }))
                }),
            )
            .route(
                "/ack",
                post(|| async {
                    ACKS.fetch_add(1, Ordering::SeqCst);
                    axum::http::StatusCode::OK
                }),
            )
            .route(
                "/result/{id}",
                post(|| async {
                    RESULTS.fetch_add(1, Ordering::SeqCst);
                    axum::http::StatusCode::OK
                }),
            );
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let base = format!("http://{}", listener.local_addr().unwrap());
        tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });

        let tmp = tempfile::tempdir().unwrap();
        let agent = RelayAgent::new();
        let session = test_session(&agent, &base, "sbn_host.secret", tmp.path()).await;
        session.keep_unsent("c1", "{}".into());
        tokio::join!(session.poll_loop(), async {
            let until = tokio::time::Instant::now() + Duration::from_secs(5);
            while ACKS.load(Ordering::SeqCst) == 0 && tokio::time::Instant::now() < until {
                tokio::time::sleep(Duration::from_millis(10)).await;
            }
            tokio::time::sleep(Duration::from_millis(100)).await;
            session.cancel.cancel();
        });
        assert!(ACKS.load(Ordering::SeqCst) > 0, "the redelivered command is acked again");
        assert_eq!(RESULTS.load(Ordering::SeqCst), 0, "not run again (running would post a new result)");
        assert_eq!(agent.unsent.lock().unwrap().len(), 1);
    }

    /// 디스코드에서 바꾼 권한의 응답과 heartbeat는 같은 PC 시계의 `permissionsAt`을 싣는다.
    /// 릴레이는 바꾼 뒤보다 먼저 읽은 heartbeat 권한을 버린다 (늦게 와서 옛 레벨로 되돌리지 않게)
    #[tokio::test]
    async fn a_permission_change_reports_when_it_was_saved() {
        let tmp = tempfile::tempdir().unwrap();
        let mut sup = crate::supervisor::Supervisor::new_with_instances_dir(
            tmp.path().join("modules").to_str().unwrap(),
            tmp.path().join("instances").to_str().unwrap(),
        );
        sup.initialize().await.unwrap();
        let server = crate::ipc::IPCServer::new(std::sync::Arc::new(tokio::sync::RwLock::new(sup)), "127.0.0.1:0", crate::daemon_log::DaemonLogBuffer::new());
        let who = crate::chat::permissions::Requester {
            guild_id: Some("r8-permissions-at".into()),
            user_id: Some("owner".into()),
            owner_id: Some("owner".into()),
            via_chatbot: false,
            confirmed_by: None,
            guild_owner_id: None,
        };
        let before = now_ms();
        let res = permission_action(&server, &who, &json!({ "op": "default_level", "level": 1 })).await;
        assert_eq!(res["success"], true, "{res}");
        let at = res["data"]["permissionsAt"].as_u64().unwrap_or_else(|| panic!("no permissionsAt: {res}"));
        assert!(at >= before && at <= now_ms());
        // 응답은 heartbeat `permissions[guild]`와 같은 모양이다
        for key in ["defaultLevel", "members", "chatbotCap", "levels", "commandLevels", "ladder"] {
            assert!(res["data"].get(key).is_some(), "no {key}: {res}");
        }
    }

    /// 노드 토큰이 평문으로 나가지 않게: 이미 저장된 http 주소(루프백 제외)로는 세션을 열지 않는다
    #[test]
    fn a_session_only_opens_to_an_https_or_loopback_relay() {
        assert_eq!(session_relay_url("").unwrap(), "https://saba-chan.online");
        assert_eq!(session_relay_url(" https://my.relay/ ").unwrap(), "https://my.relay");
        assert_eq!(session_relay_url("http://127.0.0.1:9000").unwrap(), "http://127.0.0.1:9000");
        let err = session_relay_url("http://old.example").unwrap_err();
        assert_eq!(err.code, "invalid_relay_url");
    }

    #[test]
    fn heartbeat_commands_leave_templates_on_the_pc() {
        let field: crate::supervisor::module_loader::CommandField = serde_json::from_value(json!({
            "name": "kick", "label": "Kick", "method": "console", "console_template": "kick {player}",
            "endpoint_template": "/v1/kick", "risk": "caution",
            "inputs": [{ "name": "player", "type": "text", "required": true }]
        }))
        .unwrap();
        let entry = relay_command_entry(&field, &["추방".to_string()], None);
        assert_eq!(entry["name"], "kick");
        assert_eq!(entry["aliases"], json!(["추방"]));
        assert!(relay_command_entry(&field, &[], None).get("aliases").is_none());
        assert_eq!(entry["inputs"][0]["name"], "player");
        assert!(entry.get("console_template").is_none() && entry.get("endpoint_template").is_none() && entry.get("method").is_none());
    }

    /// 번역한 명령 이름이 켜져 있으면 모듈 locales의 이름·설명·칸 이름을 언어별로 싣는다 (슬래시 명령의 언어별 이름)
    #[test]
    fn heartbeat_commands_carry_translated_names_when_asked() {
        let field: crate::supervisor::module_loader::CommandField = serde_json::from_value(json!({
            "name": "save-all", "label": "Save World", "description": "Save the game world",
            "inputs": [{ "name": "flush", "type": "text" }]
        }))
        .unwrap();
        let texts = |lang: &str| -> Value {
            match lang {
                "ko" => json!({ "commands": { "save-all": { "label": "월드 저장", "description": "게임 월드를 저장합니다", "inputs": { "flush": { "label": "바로 쓰기" } } } } }),
                _ => Value::Null,
            }
        };
        let entry = relay_command_entry(&field, &[], Some(&texts));
        assert_eq!(entry["titles"]["ko"], "월드 저장");
        assert_eq!(entry["descriptions"]["ko"], "게임 월드를 저장합니다");
        assert_eq!(entry["inputs"][0]["titles"]["ko"], "바로 쓰기");
        assert!(entry["titles"].get("en").is_none(), "no text for a language → nothing (the relay shows the plain name)");
        assert!(relay_command_entry(&field, &[], None).get("titles").is_none(), "off → plain names only");
    }

    /// 사바쨩 웹: 릴레이가 세션을 열었다고 알리면 이 세션의 릴레이 주소·토큰으로 웹소켓을 연다
    #[tokio::test]
    async fn a_web_session_opens_the_relay_websocket() {
        let _one = crate::remote::link::TEST_LOCK.lock().await;
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let base = format!("http://{}", listener.local_addr().unwrap());
        let tmp = tempfile::tempdir().unwrap();
        let session = test_session(&RelayAgent::new(), &base, "sbn_host.secret", tmp.path()).await;
        let result = session.handle_payload(&json!({ "id": "c1", "payload": { "action": "web_session", "sessionId": "s1" } })).await;
        assert_eq!(result["success"], true, "{result:?}");
        let (tcp, _) = tokio::time::timeout(Duration::from_secs(5), listener.accept()).await.expect("the daemon dials the relay").unwrap();
        let mut path = String::new();
        let _ = tokio_tungstenite::accept_hdr_async(tcp, |req: &tokio_tungstenite::tungstenite::handshake::server::Request, res| {
            path = req.uri().path().to_string();
            Ok(res)
        })
        .await;
        assert_eq!(path, "/node/ws");
        crate::remote::link::stop();
    }

    /// 원격 데몬 터널: 원격 포트가 꺼져 있거나 터널 id가 이상하면 받지 않는다
    #[tokio::test]
    async fn a_tunnel_is_refused_while_the_remote_port_is_off() {
        let tmp = tempfile::tempdir().unwrap();
        let session = test_session(&RelayAgent::new(), "http://127.0.0.1:9", "sbn_host.secret", tmp.path()).await;
        let bad = session.handle_payload(&json!({ "id": "c1", "payload": { "action": "remote_tunnel", "tunnelId": "../x" } })).await;
        assert_eq!(bad["success"], false);
        let off = session.handle_payload(&json!({ "id": "c2", "payload": { "action": "remote_tunnel", "tunnelId": "t1" } })).await;
        assert_eq!((off["success"].as_bool(), off["data"]["error"].as_str()), (Some(false), Some("remote_off")));
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
            "g1": { "members": { "a": 2, "bad": 7 }, "ownerDiscordId": "o", "chatbotCap": 1, "commandLevels": { "i/stop": 3 } },
            "g2": { "ownerDiscordId": "o", "allowedInstances": ["x"], "preset": "strict" },
            "g3": { "defaultLevel": -1 }
        });
        // 릴레이가 요구 레벨을 미리 셀 수 있게 프리셋을 푼 risk별 레벨, 챗봇 상한, 명령별 덮어쓰기도 싣는다
        let default_levels = json!({ "safe": 0, "caution": 1, "danger": 2, "admin": 2 });
        let ladder = json!([{ "id": 3 }, { "id": 2 }, { "id": 1 }, { "id": 0 }, { "id": -1 }]);
        assert_eq!(
            permissions_by_guild(&nodes),
            json!({
                "g1": { "defaultLevel": 1, "members": { "a": 2 }, "chatbotCap": 1, "levels": default_levels, "commandLevels": { "i/stop": 3 }, "ladder": ladder },
                "g2": { "defaultLevel": 1, "members": {}, "chatbotCap": 0, "levels": { "safe": 1, "caution": 2, "danger": 3, "admin": 3 }, "commandLevels": {}, "ladder": ladder },
                "g3": { "defaultLevel": -1, "members": {}, "chatbotCap": 0, "levels": default_levels, "commandLevels": {}, "ladder": ladder }
            })
        );
        let many: serde_json::Map<String, Value> = (0..COMMAND_LEVELS_PER_GUILD + 5).map(|i| (format!("i/c{i:04}"), json!(2))).collect();
        let big = permissions_by_guild(&json!({ "g": { "commandLevels": many } }));
        assert_eq!(big["g"]["commandLevels"].as_object().unwrap().len(), COMMAND_LEVELS_PER_GUILD);
        assert_eq!(big["g"]["commandLevelsTruncated"], true);
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

    // 켜짐 알림을 켠 서버는 길드마다 싣는다. 켠 서버가 없는 길드는 싣지 않는다 (릴레이는 없으면 알리지 않는다)
    #[test]
    fn start_notice_is_sent_per_guild() {
        let settings = json!({
            "g1": { "startNotice": ["i-mc", 3] },
            "g2": { "allowedInstances": ["i-mc"] },
        });
        assert_eq!(ids_by_guild(&settings, "startNotice"), Some(json!({ "g1": ["i-mc"] })));
        assert_eq!(ids_by_guild(&json!({ "g2": {} }), "startNotice"), None);
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
            raw_command_result(unknown, false),
            json!({ "success": false, "data": { "code": "UNKNOWN_COMMAND", "error": "Unknown command", "args": { "command": "dance" } } })
        );
        assert_eq!(raw_command_result("done".to_string().into(), false), json!({ "success": true, "data": { "text": "done" } }));
    }

    /// 채팅 명령 답도 액션 결과와 같이 PC의 경로를 가린 채 나간다 (I3)
    #[test]
    fn raw_command_replies_keep_local_paths_on_the_pc() {
        let failed: crate::chat::Reply = "❌ 오류: Failed to start server: Failed to spawn process 'C:\\Users\\me\\java\\bin\\java.exe': not found".to_string().into();
        let out = raw_command_result(failed, false).to_string();
        assert!(!out.contains("Users") && out.contains("<path>"), "{out}");
        let denied = crate::chat::Reply {
            text: "no access to /home/me/servers/world".into(),
            code: Some("PERMISSION_DENIED"),
            args: json!({ "path": "D:\\Games\\mc" }),
        };
        let out = raw_command_result(denied, false).to_string();
        assert!(!out.contains("/home/me") && !out.contains("Games"), "{out}");
    }

    #[test]
    fn remote_action_module_errors_hide_diagnostics_and_sensitive_text() {
        let out = action_result(Err(crate::actions::ActionError {
            status: 500, code: "module_exception", message: "peer 203.0.113.7 token=hunter2".into(),
            data: json!({ "success": false, "module": "plain", "function": "start", "path": "/home/me/server", "stderr_tail": ["private traceback"] }),
        }), "en");
        assert!(out["data"].get("stderr_tail").is_none());
        assert_eq!(out["data"]["error"], "peer x.x.x.x token=***");
        assert_eq!(out["data"]["path"], "<path>");
    }

    /// 챗봇이 부른 채팅 명령의 답은 액션 경로의 콘솔·조회 결과처럼 IP와 비밀처럼 보이는 값을 가린다
    #[test]
    fn raw_command_replies_to_the_chatbot_hide_secrets() {
        let reply = || -> crate::chat::Reply { "joined from 10.0.0.5, password=hunter2".to_string().into() };
        let out = raw_command_result(reply(), true).to_string();
        assert!(!out.contains("10.0.0.5") && !out.contains("hunter2"), "{out}");
        assert!(raw_command_result(reply(), false).to_string().contains("hunter2"), "a person asking directly sees the module's reply");
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
        // R39 준비 알림은 계약 필드만 (디스코드 "다 켜졌어요")
        let ready = ev("instance.ready", json!({"at": 1, "instanceId": "i", "instance": "mc1", "ok": false, "code": "start_failed", "line": "C:\\x"}));
        assert_eq!(relay_event(&ready).unwrap()["data"], json!({"at": 1, "instanceId": "i", "instance": "mc1", "ok": false, "code": "start_failed"}));
        // 켠 서버의 업데이트 알림은 종류·모듈·새 버전만
        let notice = ev("instance.update_notice", json!({"at": 1, "instanceId": "i", "instance": "pal", "kind": "module", "module": "palworld", "latest": "1.1.0", "path": "C:\\x"}));
        assert_eq!(relay_event(&notice).unwrap()["data"], json!({"at": 1, "instanceId": "i", "instance": "pal", "kind": "module", "module": "palworld", "latest": "1.1.0"}));
        // 시작·정상 정지는 누가 시켰는지만 (사바쨩 밖의 시작은 unexpected)
        let started = ev("instance.started", json!({"at": 1, "instanceId": "i", "instance": "mc1", "source": "external", "unexpected": true, "path": "C:\\x"}));
        assert_eq!(relay_event(&started).unwrap()["data"], json!({"at": 1, "instanceId": "i", "instance": "mc1", "source": "external", "unexpected": true}));
        let stopped = ev("instance.stopped", json!({"at": 1, "instanceId": "i", "instance": "mc1", "source": "gui", "userName": "집 PC", "kind": "user_stop"}));
        assert_eq!(relay_event(&stopped).unwrap()["data"], json!({"at": 1, "instanceId": "i", "instance": "mc1", "source": "gui", "userName": "집 PC", "kind": "user_stop"}));
        // 여러 단계 실행은 디스코드에서 시작한 것만
        assert!(relay_event(&ev("plan.done", json!({"runId": "r", "ok": true}))).is_none());
        assert_eq!(relay_event(&ev("plan.done", json!({"runId": "r", "guildId": "g", "ok": true}))).unwrap()["data"]["guildId"], "g");
    }

    #[test]
    fn a_redelivered_command_runs_once_and_gets_its_result_again() {
        let mut seen = SeenCommands::default();
        let t = std::time::Instant::now();
        assert_eq!(seen.receive("c1", t), Received::New);
        seen.start("c1");
        // 처리 중에 다시 오면 실행하지 않는다 (끝날 때 결과가 한 번 나간다)
        assert_eq!(seen.receive("c1", t + Duration::from_secs(15)), Received::Running);
        seen.finish("c1", &json!({"ok": true}));
        assert_eq!(seen.receive("c1", t + Duration::from_secs(30)), Received::Done(json!({"ok": true})));
        assert_eq!(seen.receive("c2", t + Duration::from_secs(30)), Received::New);
    }

    /// 연결을 다시 맺어도 끝난 명령과 실행 중인 명령은 기억한다. 받아 두고 시작하지 않은 명령은 잊어서 다시 오면 실행한다
    #[test]
    fn a_reconnect_keeps_finished_commands_and_forgets_unfinished_ones() {
        let mut seen = SeenCommands::default();
        let t = std::time::Instant::now();
        seen.receive("done", t);
        seen.finish("done", &json!({"ok": true}));
        seen.receive("dropped", t);
        // 실행 중인 명령은 남는다
        seen.receive("running", t);
        seen.start("running");
        seen.forget(&["dropped"]);
        assert_eq!(seen.receive("done", t), Received::Done(json!({"ok": true})));
        assert_eq!(seen.receive("dropped", t), Received::New);
        assert_eq!(seen.receive("running", t), Received::Running);
    }

    /// 실행 중인 명령이 다시 오면(ack가 닿지 않았다) 다시 ack한다. 아직 시작하지 않은 명령은 시작할 때 ack한다
    #[test]
    fn a_redelivered_command_is_told_apart_as_waiting_or_running() {
        let mut seen = SeenCommands::default();
        let t = std::time::Instant::now();
        assert_eq!(seen.receive("c1", t), Received::New);
        assert_eq!(seen.receive("c1", t), Received::Waiting);
        seen.start("c1");
        assert_eq!(seen.receive("c1", t), Received::Running);
    }

    #[test]
    fn finished_commands_are_forgotten_after_ten_minutes_or_past_the_cap() {
        let mut seen = SeenCommands::default();
        let t = std::time::Instant::now();
        seen.receive("old", t);
        seen.finish("old", &json!({"ok": true}));
        assert_eq!(seen.receive("old", std::time::Instant::now() + SEEN_FOR), Received::New);

        let mut seen = SeenCommands::default();
        for i in 0..=SEEN_MAX {
            seen.receive(&format!("c{i}"), t);
            seen.finish(&format!("c{i}"), &json!({"ok": true}));
        }
        assert_eq!(seen.entries.len(), SEEN_MAX);
        assert_eq!(seen.receive("c0", t), Received::New);
        assert_eq!(seen.receive(&format!("c{SEEN_MAX}"), t), Received::Done(json!({"ok": true})));
    }

    /// 외부 감사: 대기열이 밀려 10분 넘게 기다린 명령을 시작한 뒤 다시 받으면, 시간 정리가 그 명령까지 잊어 또 실행했다
    #[test]
    fn waiting_or_running_commands_are_not_forgotten_by_time_or_cap() {
        let mut seen = SeenCommands::default();
        let t = std::time::Instant::now();
        seen.receive("waiting", t);
        seen.receive("running", t);
        seen.start("running");
        seen.receive("done", t);
        seen.finish("done", &json!({"ok": true}));
        let later = t + SEEN_FOR + Duration::from_secs(1);
        assert_eq!(seen.receive("waiting", later), Received::Waiting);
        assert_eq!(seen.receive("running", later), Received::Running);
        assert_eq!(seen.receive("done", later), Received::New, "a finished one is still forgotten");
        // 개수를 넘으면 끝난 명령부터 잊는다
        for i in 0..SEEN_MAX {
            seen.receive(&format!("c{i}"), later);
            seen.finish(&format!("c{i}"), &json!({"ok": true}));
        }
        assert_eq!(seen.receive("waiting", later), Received::Waiting);
        assert_eq!(seen.receive("running", later), Received::Running);
    }

    /// 독립 리뷰: 10분 넘게 기다린 명령이 끝나자마자 다시 오면, 받은 시각 기준으로 잊혀 또 실행했다
    #[test]
    fn a_long_waiting_command_is_kept_for_ten_minutes_after_it_finishes() {
        let mut seen = SeenCommands::default();
        let now = std::time::Instant::now();
        assert_eq!(seen.receive("c1", now - SEEN_FOR - Duration::from_secs(1)), Received::New);
        seen.start("c1");
        seen.finish("c1", &json!({"ok": true}));
        assert_eq!(seen.receive("c1", now), Received::Done(json!({"ok": true})));
    }

    #[test]
    fn acks_carry_every_started_id_in_batches_the_relay_accepts() {
        let commands: Vec<Value> = (0..120).map(|i| json!({"id": format!("c{i}")})).chain([json!({"payload": {}})]).collect();
        let bodies = ack_bodies(&commands);
        let ids: Vec<Vec<String>> = bodies.iter().map(|b| serde_json::from_str::<Value>(b).unwrap()["ids"].as_array().unwrap().iter().map(|v| v.as_str().unwrap().to_string()).collect()).collect();
        assert_eq!(ids.iter().map(Vec::len).collect::<Vec<_>>(), vec![50, 50, 20]);
        assert_eq!(ids.concat(), (0..120).map(|i| format!("c{i}")).collect::<Vec<_>>());
        assert!(ack_bodies(&[]).is_empty());
    }

    #[test]
    fn semver_compare() {
        assert!(semver_lt("0.1.0", "0.1.1"));
        assert!(!semver_lt("0.1.1", "0.1.0"));
        assert!(!semver_lt("1.0", "1.0.0"));
        assert!(semver_lt("0.9.9", "1"));
    }

    #[test]
    fn prerelease_agent_is_not_older_than_previous_release() {
        assert!(!semver_lt("0.1.11-beta.1", "0.1.10"));
        assert!(semver_lt("0.1.11-beta.1", "0.1.11"));
        assert!(semver_lt("0.1.11-beta.1", "0.1.11-beta.2"));
        assert!(!semver_lt("0.1.11-rc.1", "0.1.11-beta.2"));
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
