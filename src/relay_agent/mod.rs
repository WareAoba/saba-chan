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
//! 새 명령 종류는 `handle_payload`(`poll.rs`)의 action 분기에 추가합니다.
//!
//! 파일: `heartbeat`(하트비트), `metadata`(하트비트에 싣는 항목), `poll`(명령 받기), `commands`(명령 실행),
//! `events`(알림 이벤트), `results`(ack·결과 보내기). 이 파일은 에이전트 핸들·상태·세션과 공용 도우미를 둔다.

use crate::relay_client::{load_node_token, parse_node_token, relay_client_with_timeout, resolve_relay_url, signed_request};
use crate::ipc::IPCServer;
use serde::Serialize;
use serde_json::{json, Value};
use std::sync::{Arc, Mutex};
use std::time::Duration;
use tokio_util::sync::CancellationToken;

mod commands;
mod events;
mod heartbeat;
mod metadata;
mod poll;
mod results;

use commands::*;
use events::*;
use metadata::*;
use poll::*;
use results::*;
pub use heartbeat::platform;
pub(crate) use events::relay_event;

pub const AGENT_VERSION: &str = env!("CARGO_PKG_VERSION");

const HEARTBEAT_INTERVAL: Duration = Duration::from_secs(60);
/// 서버가 켜지거나 꺼지면 이만큼 모았다가 다음 하트비트를 기다리지 않고 보낸다 (`kicks_heartbeat`).
/// 앞당긴 하트비트도 앞의 것과 이만큼은 띄운다 (릴레이 `POST /heartbeat` 한도)
const HEARTBEAT_KICK_DELAY: Duration = Duration::from_secs(2);
const HEARTBEAT_MIN_GAP: Duration = Duration::from_secs(15);
/// 마지막 하트비트 성공이 이보다 오래되면 붙어 있지 않은 것으로 본다 (하트비트 세 번)
const CONNECTED_WITHIN_MS: u64 = 3 * 60_000;
/// 공인 IPv4를 일시 실패로 얻지 못했을 때 다시 묻는 간격 (`friend_address`, 포트 확인 한도 분당 6회보다 넉넉히)
const IPV4_RETRY: Duration = if cfg!(test) { Duration::from_millis(50) } else { Duration::from_secs(5 * 60) };
const POLL_TIMEOUT: Duration = Duration::from_secs(40); // 릴레이 롱폴 25초 + 여유
const RETRY_BASE: Duration = Duration::from_secs(3);
const RETRY_MAX: Duration = Duration::from_secs(60);

/// 알림 이벤트(I3): 처음 한 건이 오면 같은 순간에 난 것만 이만큼 모아 곧 보내고, 한 번에 이만큼까지 보낸다
/// (사용자 11-02: 게임 채팅이 디스코드에 늦게 올라간다. 이벤트는 대부분 한 건씩 온다)
const EVENT_GATHER: Duration = Duration::from_millis(20);
/// 보내기 사이 최소 간격: relay `/events` 한도(노드당 초당 5건, 분당 300건) 안에 머문다. 이어서 오면 이 간격으로 묶여 간다
const EVENT_MIN_GAP: Duration = Duration::from_millis(200);
const EVENT_BATCH: usize = 20;
/// relay에 닿지 못하는 동안 들고 있는 최근 이벤트 수 (알림은 기록이 아니다)
const EVENT_PENDING: usize = 50;

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

/// `POST /ack` 한 번에 싣는 id 수 (릴레이 상한)
const ACK_BATCH: usize = 50;

/// 에이전트 상태 (GET /api/relay/agent)
#[derive(Debug, Clone, Default, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct AgentStatus {
    pub running: bool,
    /// 화면이 보일 상태 하나 (0.1.14-beta.2~, `agent_state`): `running`·`too_old`(릴레이가 이 앱 버전을 받지 않음, `minVersion`)·
    /// `rejected`(세션은 살아 있지만 하트비트를 거절당함, `heartbeatRejected`)·`replaced`(같은 계정이 다른 PC를 페어링)·
    /// `invalid_relay_url`(저장된 릴레이 주소를 쓸 수 없어 띄우지 않음)·`stopped`
    pub state: &'static str,
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
    /// 릴레이가 마지막 하트비트를 검증에서 거절한 이유. 다시 받아 주면 없다
    #[serde(skip_serializing_if = "Option::is_none")]
    pub heartbeat_rejected: Option<HeartbeatReject>,
    pub commands_handled: u64,
    /// 릴레이와 붙어 있다: 돌고 있고, 하트비트가 최근에 성공했고, 그 뒤로 실패가 없다.
    /// 화면은 릴레이에 따로 묻지 않고 이 값을 본다. 바뀌면 `relay.status` 이벤트가 나간다
    pub connected: bool,
    /// 릴레이가 하트비트 답에 실어 준, 이 PC가 접속해 온 공인 주소 (서버 주소 `server.address`). 모르면 없다
    #[serde(skip_serializing_if = "Option::is_none")]
    pub seen_address: Option<String>,
}

/// 에이전트 상태를 화면이 보일 값 하나로 (`AgentStatus::state`). GUI·CLI가 오류 코드를 따로 해석하지 않게 데몬이 정한다
fn agent_state(s: &AgentStatus) -> &'static str {
    match (s.running, s.last_error_code) {
        (_, Some(TOO_OLD)) => "too_old",
        (true, _) if s.heartbeat_rejected.is_some() => "rejected",
        (true, _) => "running",
        (false, Some("token_replaced")) => "replaced",
        (false, Some("invalid_relay_url")) => "invalid_relay_url",
        _ => "stopped",
    }
}

/// 릴레이가 하트비트를 거절한 이유 (400 답의 `reason`): 어느 칸이 맞지 않았는지, 개수·길이 상한이면 상한과 보낸 크기.
/// 문장은 클라이언트가 만든다
#[derive(Debug, Clone, PartialEq, Serialize, serde::Deserialize)]
pub struct HeartbeatReject {
    /// `TOO_BIG`(상한 초과) 또는 `INVALID`
    pub code: String,
    pub field: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub max: Option<u64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub actual: Option<u64>,
}

#[derive(Default)]
struct Inner {
    cancel: Option<CancellationToken>,
    /// 지금 세션의 릴레이 주소와 노드 토큰 (`notify_updating`이 같은 쌍으로 보낸다)
    endpoint: Option<(String, String)>,
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
    /// 업데이트로 내려간다고 릴레이에 알렸다 (`notify_updating`): 그 뒤로는 heartbeat를 보내지 않는다.
    /// 릴레이는 알린 뒤의 heartbeat를 새 데몬이 돌아온 것으로 보고 "업데이트 중" 창을 닫는다
    going_down: Arc<std::sync::atomic::AtomicBool>,
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
        // GUI가 5초마다 부른다: 설정 파일이 그대로면 다시 읽지 않는다
        status.has_token = crate::relay_client::peek_node_token().as_deref().and_then(parse_node_token).is_some();
        status.connected = connected(&status, now_ms());
        status.state = agent_state(&status);
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
                    let before = Shown::of(&inner.status);
                    inner.status.last_error = Some(e.message.clone());
                    inner.status.last_error_code = Some(e.code);
                    before.announce_if_changed(&inner.status);
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
            inner.endpoint = Some((relay_url.clone(), node_token.clone()));
            inner.generation += 1;
            let before = Shown::of(&inner.status);
            inner.status = AgentStatus { running: true, relay_url: Some(relay_url.clone()), ..Default::default() };
            before.announce_if_changed(&inner.status);
            (token, inner.generation)
        };

        let agent = self.clone();
        tokio::spawn(async move {
            let client = match relay_client_with_timeout(POLL_TIMEOUT) {
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
                metadata_sent: Mutex::new(None),
                linked_acks: Mutex::default(),
                chatbot_sync_due: std::sync::atomic::AtomicBool::new(true),
                seen_v4: Mutex::default(),
            };
            if let Err(e) = session.check_version().await {
                tracing::error!("[RelayAgent] {}", e);
                agent.finish(generation, Some(e));
                return;
            }
            tracing::info!("[RelayAgent] started (relay={})", session.base);
            // 디스코드 길드 설정과 챗봇 설정은 데몬이 원본 (I2): 연결될 때마다 릴레이에 맞춘다. 첫 하트비트 응답을 처리한 뒤에 보낸다:
            // 꺼져 있는 동안 다시 묶인 길드(`linkedGuilds`)는 릴레이 값을 먼저 받아야 옛 사본으로 덮지 않는다
            crate::service::discord::request_sync();
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

    /// 데몬이 업데이트로 내려가기 직전: 릴레이가 그동안 디스코드 명령에 "업데이트 중"으로 답하고 "PC 꺼짐"으로 보지 않게 한다
    /// (`POST /v1/host/updating`). 연결돼 있지 않으면 오류
    pub async fn notify_updating(&self, eta_seconds: u64) -> Result<(), String> {
        // 알리기 전에 막는다: 알린 뒤 옛 데몬의 heartbeat가 창을 닫지 않게
        self.going_down.store(true, std::sync::atomic::Ordering::SeqCst);
        let (base, token) = {
            let inner = self.lock();
            match (&inner.cancel, &inner.endpoint) {
                (Some(cancel), Some(endpoint)) if !cancel.is_cancelled() => endpoint.clone(),
                _ => return Err("relay not connected".into()),
            }
        };
        let client = relay_client_with_timeout(POLL_TIMEOUT).map_err(|e| e.to_string())?;
        let body = serde_json::json!({ "etaSeconds": eta_seconds }).to_string();
        let req = crate::relay_client::signed_request(&client, reqwest::Method::POST, &base, "/v1/host/updating", &token, Some(body))
            .ok_or("node token missing")?;
        let resp = tokio::time::timeout(std::time::Duration::from_secs(5), req.send())
            .await
            .map_err(|_| "timed out".to_string())?
            .map_err(|e| e.to_string())?;
        if !resp.status().is_success() {
            return Err(format!("relay answered {}", resp.status()));
        }
        Ok(())
    }

    /// 사용자가 사바쨩을 꺼서 데몬이 내려가기 직전: 릴레이가 3분을 기다리지 않고 바로 꺼짐으로 두고 "PC 연결 끊김" 알림을 보내지 않게 한다
    /// (`POST /v1/host/stopping`, 본문 없음). 업데이트로 내려가는 중이면 이미 알렸으므로 보내지 않는다. 실패해도 종료는 진행한다
    pub async fn notify_stopping(&self) {
        // 알린 뒤의 heartbeat가 다시 켜짐으로 돌리지 않게 먼저 막는다. 이미 막혀 있으면 업데이트 창이다
        if self.going_down.swap(true, std::sync::atomic::Ordering::SeqCst) {
            return;
        }
        let (base, token) = {
            let inner = self.lock();
            match (&inner.cancel, &inner.endpoint) {
                (Some(cancel), Some(endpoint)) if !cancel.is_cancelled() => endpoint.clone(),
                _ => return,
            }
        };
        let Ok(client) = relay_client_with_timeout(POLL_TIMEOUT) else { return };
        let Some(req) = crate::relay_client::signed_request(&client, reqwest::Method::POST, &base, "/v1/host/stopping", &token, None)
        else {
            return;
        };
        match tokio::time::timeout(std::time::Duration::from_secs(5), req.send()).await {
            Ok(Ok(resp)) if resp.status().is_success() => {}
            Ok(Ok(resp)) => tracing::warn!("[Shutdown] Relay answered {} to the stopping notice", resp.status()),
            Ok(Err(e)) => tracing::warn!("[Shutdown] Could not tell the relay we are stopping: {}", e),
            Err(_) => tracing::warn!("[Shutdown] Telling the relay we are stopping timed out"),
        }
    }

    pub fn stop(&self) {
        crate::remote::link::stop();
        let mut inner = self.lock();
        inner.endpoint = None;
        if let Some(cancel) = inner.cancel.take() {
            cancel.cancel();
        }
        let before = Shown::of(&inner.status);
        inner.status.running = false;
        before.announce_if_changed(&inner.status);
    }

    /// 세션이 끝났을 때. 그 사이 새 세션이 시작됐으면(세대가 다르면) 아무것도 건드리지 않는다
    fn finish(&self, generation: u64, error: Option<String>) {
        let mut inner = self.lock();
        if inner.generation != generation {
            return;
        }
        inner.cancel = None;
        inner.endpoint = None;
        let before = Shown::of(&inner.status);
        inner.status.running = false;
        if error.is_some() {
            inner.status.last_error = error;
        }
        before.announce_if_changed(&inner.status);
    }

    fn lock(&self) -> std::sync::MutexGuard<'_, Inner> {
        self.inner.lock().unwrap_or_else(|e| e.into_inner())
    }

    /// 지금 연결된 세션에서 릴레이가 본 이 PC의 공인 주소 (연결돼 있지 않으면 없다)
    pub fn seen_address(&self) -> Option<String> {
        let inner = self.lock();
        inner.status.seen_address.clone().filter(|_| inner.cancel.is_some())
    }

    #[cfg(test)]
    pub(crate) fn set_seen_address_for_test(&self, address: Option<&str>) {
        let mut inner = self.lock();
        inner.cancel.get_or_insert_with(CancellationToken::new);
        inner.status.seen_address = address.map(str::to_string);
    }

    /// 세션 상태 갱신. 끝난(세대가 지난) 세션의 늦은 응답은 버린다
    fn update(&self, generation: u64, f: impl FnOnce(&mut AgentStatus)) {
        let mut inner = self.lock();
        if inner.generation == generation && inner.cancel.is_some() {
            let before = Shown::of(&inner.status);
            f(&mut inner.status);
            before.announce_if_changed(&inner.status);
        }
    }
}

/// 릴레이와 붙어 있는가 (`AgentStatus::connected`)
fn connected(s: &AgentStatus, now: u64) -> bool {
    s.running && s.last_error.is_none() && s.last_heartbeat_at.is_some_and(|at| now.saturating_sub(at) < CONNECTED_WITHIN_MS)
}

/// 화면이 보이는 연결 상태. 하트비트 시각처럼 늘 바뀌는 값은 빼고, 이것이 바뀔 때만 알린다
#[derive(PartialEq)]
struct Shown {
    running: bool,
    heard: bool,
    error: Option<String>,
    code: Option<&'static str>,
}

impl Shown {
    fn of(s: &AgentStatus) -> Self {
        Self { running: s.running, heard: s.last_heartbeat_at.is_some(), error: s.last_error.clone(), code: s.last_error_code }
    }

    /// 바뀌었으면 `relay.status`를 낸다 (지금 붙은 클라이언트에게만: 다시 붙으면 상태를 새로 읽는다)
    fn announce_if_changed(self, after: &AgentStatus) {
        if self != Self::of(after) {
            crate::events::emit_live("relay.status", json!({}));
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

struct Session {
    agent: RelayAgent,
    server: IPCServer,
    client: reqwest::Client,
    base: String,
    token: String,
    cancel: CancellationToken,
    generation: u64,
    seen: Arc<Mutex<SeenCommands>>,
    /// 릴레이가 메타데이터 전체를 받아 둔 지문 (`heartbeat_metadata`). 없으면 다음 하트비트는 전체를 보낸다
    metadata_sent: Mutex<Option<String>>,
    /// 적용했지만 릴레이가 아직 확인 응답을 받지 못한 새로 묶인 길드의 연결 id (`linkedGuildsAckIds`)
    linked_acks: Mutex<Vec<String>>,
    /// 첫 하트비트 응답 뒤 챗봇 설정을 릴레이에 맞춘다
    chatbot_sync_due: std::sync::atomic::AtomicBool,
    /// 릴레이가 본 IPv6를 IPv4로 다시 물은 결과 (`friend_address`). 같은 IPv6에는 답을 얻은 뒤로 다시 묻지 않는다
    seen_v4: Mutex<Option<SeenV4>>,
}

#[derive(Clone)]
struct SeenV4 {
    v6: String,
    /// 그때 얻은 공인 IPv4
    v4: Option<String>,
    /// 일시 실패였으면 다시 물을 때
    retry_at: Option<std::time::Instant>,
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
        signed_request(&self.client, method, &self.base, path, &self.token, body).ok_or_else(|| "node token missing".into())
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
}

fn backoff(failures: u32) -> Duration {
    crate::utils::exp_backoff(RETRY_BASE, RETRY_MAX, failures)
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

#[cfg(test)]
mod tests;
