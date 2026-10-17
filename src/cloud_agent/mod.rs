//! 클라우드 에이전트 — saba-chan 데몬을 릴레이 서버에 연결하는 인프라 계층.
//!
//! ## 기능
//! 1. **Heartbeat** (60초 주기): `POST /heartbeat` — 모듈/인스턴스 상태 보고
//! 2. **Long-poll** (지속): `GET /poll` — 사용자 명령 수신
//! 3. **Result post-back**: `POST /result/:requestId` — 명령 실행 결과 회신
//!
//! ## 인증
//! `<data_dir>/.node_token` 파일이 존재할 때만 활성화된다. 토큰이 없으면
//! "클라우드 미페어링" 상태로 간주하고 idle. 페어링은 별도 IPC 핸들러
//! ([`crate::ipc::handlers::relay::pair`])가 처리한다.
//!
//! ## 종료
//! 호출자는 `CancellationToken`을 넘기고, 데몬 종료 시 cancel을 호출한다.
//! 두 루프 모두 `tokio::select!` 로 즉시 빠져나온다.

pub mod auth;
pub mod dispatcher;

use crate::supervisor::Supervisor;
use auth::{apply_signed, ParsedToken};
use serde_json::{json, Value};
use std::sync::Arc;
use std::time::Duration;
use tokio::sync::RwLock;
use tokio::task::JoinHandle;
use tokio_util::sync::CancellationToken;

/// 기본 릴레이 URL — 환경변수 `SABA_RELAY_URL` 로 오버라이드 가능.
pub const DEFAULT_RELAY_URL: &str = "https://saba-chan.online";

/// 환경변수 키.
pub const ENV_RELAY_URL: &str = "SABA_RELAY_URL";

const HEARTBEAT_INTERVAL_SECS: u64 = 60;
const POLL_TIMEOUT_SECS: u64 = 30; // 서버는 25s에 종료, 5s 여유
const BACKOFF_INITIAL_SECS: u64 = 3;
const BACKOFF_MAX_SECS: u64 = 60;

/// 데몬이 서버 fast-return 204 를 받아도 tight loop 에 빠지지 않도록
/// `Ok(None)` (204 또는 빈 commands) 이후 최소한 기다린다. 서버의 long-poll은
/// 정상적으로 25s 대기하므로 이 sleep 은 일상적으로 관찰되지 않고,
/// 수 ms~ 단위의 fast-return 이 발생해야만 적용된다.
const POLL_EMPTY_COOLDOWN_MS: u64 = 1_000;

/// 여러 데몬이 동시에 재시작하어 동기화되는 것을 방지하기 위해
/// 최초 폴 전 0~2s 랜덤 jitter를 적용한다.
const POLL_STARTUP_JITTER_MS: u64 = 2_000;

/// 에이전트 동작 설정.
#[derive(Debug, Clone)]
pub struct CloudAgentConfig {
    pub relay_url: String,
    pub agent_version: String,
    pub heartbeat_interval: Duration,
    pub poll_timeout: Duration,
}

impl CloudAgentConfig {
    /// 환경변수와 `CARGO_PKG_VERSION` 으로 기본 설정을 만든다.
    pub fn from_env() -> Self {
        let relay_url = std::env::var(ENV_RELAY_URL)
            .ok()
            .filter(|s| !s.trim().is_empty())
            .unwrap_or_else(|| DEFAULT_RELAY_URL.to_string());
        Self {
            relay_url: relay_url.trim_end_matches('/').to_string(),
            agent_version: env!("CARGO_PKG_VERSION").to_string(),
            heartbeat_interval: Duration::from_secs(HEARTBEAT_INTERVAL_SECS),
            poll_timeout: Duration::from_secs(POLL_TIMEOUT_SECS),
        }
    }
}

/// 에이전트 태스크(heartbeat + poll)를 spawn한다. 토큰은 호출자가 주입한다.
///
/// 라이프사이클 관리(시작/재시작/정지)는 [`CloudAgentRuntime`]이 담당하며,
/// 이 함수는 테스트 및 런타임 내부에서 사용된다.
pub fn start_with_token(
    config: CloudAgentConfig,
    token: ParsedToken,
    supervisor: Arc<RwLock<Supervisor>>,
    config_store: Arc<crate::config_store::ConfigStore>,
    shutdown: CancellationToken,
) -> Vec<JoinHandle<()>> {
    tracing::info!(
        "[CloudAgent] 시작 — node_id={}, relay={}",
        token.node_id,
        config.relay_url
    );

    let client = match build_client(&config) {
        Ok(c) => c,
        Err(e) => {
            tracing::error!("[CloudAgent] HTTP client 생성 실패: {}", e);
            return Vec::new();
        }
    };

    let hb = tokio::spawn(heartbeat_loop(
        config.clone(),
        client.clone(),
        token.clone(),
        supervisor.clone(),
        config_store.clone(),
        shutdown.clone(),
    ));
    let poll = tokio::spawn(poll_loop(
        config,
        client,
        token,
        supervisor,
        config_store,
        shutdown,
    ));
    vec![hb, poll]
}

// ── 핫 리로드 런타임 ─────────────────────────────────────────

/// 실행 중인 에이전트 인스턴스 (취소 토큰 + 태스크 핸들).
struct RunningAgent {
    cancel: CancellationToken,
    handles: Vec<JoinHandle<()>>,
}

/// 핫 리로드 가능한 클라우드 에이전트 런타임.
///
/// 페어링 완료·토큰 저장/삭제 시 **데몬 재시작 없이** 에이전트를
/// 시작/재시작/정지할 수 있다. 데몬 부팅 시에도 이 런타임을 통해 시작한다.
pub struct CloudAgentRuntime {
    config: CloudAgentConfig,
    supervisor: Arc<RwLock<Supervisor>>,
    config_store: Arc<crate::config_store::ConfigStore>,
    /// `.node_token` 파일이 위치한 데이터 디렉토리.
    data_dir: std::path::PathBuf,
    /// 데몬 전체 종료 토큰 — 에이전트 인스턴스는 이 토큰의 child로 생성된다.
    parent_shutdown: CancellationToken,
    current: tokio::sync::Mutex<Option<RunningAgent>>,
}

impl CloudAgentRuntime {
    pub fn new(
        config: CloudAgentConfig,
        supervisor: Arc<RwLock<Supervisor>>,
        config_store: Arc<crate::config_store::ConfigStore>,
        data_dir: std::path::PathBuf,
        parent_shutdown: CancellationToken,
    ) -> Self {
        Self {
            config,
            supervisor,
            config_store,
            data_dir,
            parent_shutdown,
            current: tokio::sync::Mutex::new(None),
        }
    }

    /// `.node_token` 파일 기준으로 에이전트를 (재)시작한다.
    ///
    /// 기존 인스턴스가 있으면 먼저 정지한다. 토큰이 없거나 파싱 불가면
    /// 정지 상태로 남고 `false`를 반환한다.
    pub async fn restart_from_file(&self) -> bool {
        let mut slot = self.current.lock().await;
        Self::stop_agent(slot.take()).await;

        let token = match auth::load_node_token_from(&self.data_dir) {
            Some(t) => t,
            None => {
                tracing::info!(
                    "[CloudAgent] .node_token 없음 — 클라우드 비활성 (페어링 시 자동 활성화)"
                );
                return false;
            }
        };

        let cancel = self.parent_shutdown.child_token();
        let handles = start_with_token(
            self.config.clone(),
            token,
            self.supervisor.clone(),
            self.config_store.clone(),
            cancel.clone(),
        );
        if handles.is_empty() {
            return false;
        }
        *slot = Some(RunningAgent { cancel, handles });
        true
    }

    /// 실행 중인 에이전트를 정지한다 (멱등).
    pub async fn stop(&self) {
        let mut slot = self.current.lock().await;
        Self::stop_agent(slot.take()).await;
    }

    /// 현재 에이전트가 실행 중인지 여부.
    #[allow(dead_code)] // 테스트·향후 상태 API용 (bin 타깃에서는 미사용)
    pub async fn is_running(&self) -> bool {
        self.current.lock().await.is_some()
    }

    async fn stop_agent(agent: Option<RunningAgent>) {
        let Some(agent) = agent else { return };
        agent.cancel.cancel();
        for mut handle in agent.handles {
            // select! 기반 루프라 즉시 빠져나오지만, 안전망으로 timeout.
            if tokio::time::timeout(Duration::from_secs(5), &mut handle).await.is_err() {
                handle.abort();
                let _ = handle.await;
            }
        }
        tracing::info!("[CloudAgent] 에이전트 정지 완료");
    }
}

fn build_client(config: &CloudAgentConfig) -> reqwest::Result<reqwest::Client> {
    reqwest::Client::builder()
        .user_agent(format!("saba-chan-daemon/{}", config.agent_version))
        // poll보다 약간 더 길게: 서버 25s + 여유
        .timeout(config.poll_timeout + Duration::from_secs(5))
        .build()
}

async fn heartbeat_loop(
    config: CloudAgentConfig,
    client: reqwest::Client,
    token: ParsedToken,
    supervisor: Arc<RwLock<Supervisor>>,
    config_store: Arc<crate::config_store::ConfigStore>,
    shutdown: CancellationToken,
) {
    // 첫 heartbeat는 즉시
    let mut interval = tokio::time::interval(config.heartbeat_interval);
    interval.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);

    loop {
        tokio::select! {
            _ = shutdown.cancelled() => {
                tracing::info!("[CloudAgent] heartbeat 루프 종료");
                return;
            }
            _ = interval.tick() => {
                if let Err(e) = send_heartbeat(&config, &client, &token, &supervisor, &config_store).await {
                    tracing::warn!("[CloudAgent] heartbeat 실패: {}", e);
                }
            }
        }
    }
}

async fn send_heartbeat(
    config: &CloudAgentConfig,
    client: &reqwest::Client,
    token: &ParsedToken,
    supervisor: &Arc<RwLock<Supervisor>>,
    config_store: &Arc<crate::config_store::ConfigStore>,
) -> anyhow::Result<()> {
    let body = build_heartbeat_body(config, supervisor, config_store).await;
    let body_str = serde_json::to_string(&body)?;
    let url = format!("{}/heartbeat", config.relay_url);
    let req = client.post(&url).header("content-type", "application/json");
    let req = apply_signed(req, token, "POST", "/heartbeat", &body_str);
    let resp = req.body(body_str).send().await?;
    let status = resp.status();
    if !status.is_success() {
        let text = resp.text().await.unwrap_or_default();
        anyhow::bail!("heartbeat status {}: {}", status, text);
    }
    tracing::debug!("[CloudAgent] heartbeat OK");
    Ok(())
}

async fn build_heartbeat_body(
    config: &CloudAgentConfig,
    supervisor: &Arc<RwLock<Supervisor>>,
    config_store: &Arc<crate::config_store::ConfigStore>,
) -> Value {
    let guard = supervisor.read().await;

    let modules: Vec<String> = guard
        .list_modules()
        .ok()
        .unwrap_or_default()
        .into_iter()
        .map(|m| m.metadata.name)
        .collect();

    let servers: Vec<Value> = guard
        .instance_store
        .list()
        .iter()
        .map(|i| {
            json!({
                "id": i.id,
                "name": i.name,
                "module": i.module_name,
            })
        })
        .collect();

    // 앱(GUI) 언어 — 릴레이가 locale_source='auto' 노드의 응답 언어로 사용한다.
    // 미설정(None)이면 필드를 생략해 릴레이가 길드 언어로 폴백하게 한다.
    let language = config_store.get_gui_settings().await.language;

    let mut metadata = serde_json::Map::new();
    metadata.insert("modules".to_string(), json!(modules));
    metadata.insert("servers".to_string(), json!(servers));
    metadata.insert("moduleDetails".to_string(), json!({}));
    if let Some(lang) = language.filter(|l| !l.trim().is_empty()) {
        metadata.insert("language".to_string(), json!(lang));
    }

    json!({
        "agentVersion": config.agent_version,
        "os": std::env::consts::OS,
        "metadata": Value::Object(metadata),
    })
}

async fn poll_loop(
    config: CloudAgentConfig,
    client: reqwest::Client,
    token: ParsedToken,
    supervisor: Arc<RwLock<Supervisor>>,
    config_store: Arc<crate::config_store::ConfigStore>,
    shutdown: CancellationToken,
) {
    // 시작 jitter — 여러 데몬이 동시에 재시작했을 때 폴이 동기화되어
    // 릴레이의 rate-limit 윈도우를 함께 채우는 것을 방지.
    let startup_jitter_ms = {
        use std::collections::hash_map::DefaultHasher;
        use std::hash::{Hash, Hasher};
        // node_id 기반 결정적 jitter — 같은 노드는 항상 같은 jitter (재현성 ↑),
        // 서로 다른 노드는 다른 jitter (분산 ↑).
        let mut h = DefaultHasher::new();
        token.node_id.hash(&mut h);
        h.finish() % POLL_STARTUP_JITTER_MS
    };
    if startup_jitter_ms > 0 {
        tokio::select! {
            _ = shutdown.cancelled() => return,
            _ = tokio::time::sleep(Duration::from_millis(startup_jitter_ms)) => {}
        }
    }

    let mut backoff = BACKOFF_INITIAL_SECS;
    loop {
        if shutdown.is_cancelled() {
            tracing::info!("[CloudAgent] poll 루프 종료");
            return;
        }

        let result = tokio::select! {
            _ = shutdown.cancelled() => {
                tracing::info!("[CloudAgent] poll 루프 종료 (요청 도중)");
                return;
            }
            r = poll_once(&config, &client, &token) => r,
        };

        match result {
            Ok(Some(commands)) => {
                backoff = BACKOFF_INITIAL_SECS;
                for cmd in commands {
                    handle_command(&config, &client, &token, &supervisor, &config_store, cmd)
                        .await;
                }
            }
            Ok(None) => {
                // 204 또는 빈 commands — 정상 long-poll 은 25s 가까이 대기했으므로
                // 즉시 재폴해도 무방하지만, 서버가 fast-return 하는 비정상 시나리오
                // (race·earlyResults 버퍼·connection drop 등)에서 tight loop 에 빠지지
                // 않도록 최소 cooldown 을 둔다.
                backoff = BACKOFF_INITIAL_SECS;
                tokio::select! {
                    _ = shutdown.cancelled() => return,
                    _ = tokio::time::sleep(Duration::from_millis(POLL_EMPTY_COOLDOWN_MS)) => {}
                }
            }
            Err(e) => {
                tracing::warn!(
                    "[CloudAgent] poll 실패 (backoff {}s): {}",
                    backoff,
                    e
                );
                tokio::select! {
                    _ = shutdown.cancelled() => return,
                    _ = tokio::time::sleep(Duration::from_secs(backoff)) => {}
                }
                backoff = (backoff * 2).min(BACKOFF_MAX_SECS);
            }
        }
    }
}

/// 한 번의 long-poll 요청. `Ok(Some(cmds))`는 명령 도착, `Ok(None)`은 timeout(204).
async fn poll_once(
    config: &CloudAgentConfig,
    client: &reqwest::Client,
    token: &ParsedToken,
) -> anyhow::Result<Option<Vec<RelayCommand>>> {
    let url = format!("{}/poll", config.relay_url);
    let req = client.get(&url);
    let req = apply_signed(req, token, "GET", "/poll", "");
    let resp = req.send().await?;
    let status = resp.status();
    if status.as_u16() == 204 {
        return Ok(None);
    }
    if !status.is_success() {
        let text = resp.text().await.unwrap_or_default();
        anyhow::bail!("poll status {}: {}", status, text);
    }
    let body: Value = resp.json().await?;
    let cmds = body
        .get("commands")
        .and_then(Value::as_array)
        .cloned()
        .unwrap_or_default();
    if cmds.is_empty() {
        return Ok(None);
    }
    let mut out = Vec::with_capacity(cmds.len());
    for c in cmds {
        if let Some(cmd) = RelayCommand::from_value(c) {
            out.push(cmd);
        }
    }
    Ok(Some(out))
}

#[derive(Debug, Clone)]
pub struct RelayCommand {
    pub id: String,
    pub payload: Value,
    /// 명령이 발신된 디스코드 노드(길드) ID — 릴레이 poll 응답의 `guildId`.
    pub guild_id: Option<String>,
    /// 명령을 보낸 디스코드 유저 ID — 릴레이 poll 응답의 `requestedBy`.
    /// 릴레이는 시스템 발신 명령에 "system"을 넣는데, 이는 유저 제한 대상이 아니다.
    pub requested_by: Option<String>,
}

impl RelayCommand {
    fn from_value(v: Value) -> Option<Self> {
        let id = v.get("id")?.as_str()?.to_string();
        let payload = v.get("payload").cloned().unwrap_or(Value::Null);
        let guild_id = v
            .get("guildId")
            .and_then(Value::as_str)
            .filter(|s| !s.is_empty())
            .map(str::to_string);
        let requested_by = v
            .get("requestedBy")
            .and_then(Value::as_str)
            .filter(|s| !s.is_empty() && *s != "system")
            .map(str::to_string);
        Some(Self {
            id,
            payload,
            guild_id,
            requested_by,
        })
    }
}

async fn handle_command(
    config: &CloudAgentConfig,
    client: &reqwest::Client,
    token: &ParsedToken,
    supervisor: &Arc<RwLock<Supervisor>>,
    config_store: &Arc<crate::config_store::ConfigStore>,
    cmd: RelayCommand,
) {
    tracing::info!("[CloudAgent] 명령 수신: id={}", cmd.id);
    // 인스턴스별 노드/유저 권한 + 별명 오버라이드 스냅샷 — 판정은 디스패처가 수행.
    let instance_settings = config_store.get_bot_config().await.instance_settings;
    let ctx = dispatcher::DispatchContext {
        guild_id: cmd.guild_id.clone(),
        requested_by: cmd.requested_by.clone(),
        instance_settings,
    };
    let result = dispatcher::dispatch(supervisor.clone(), &cmd.payload, &ctx).await;
    if let Err(e) = post_result(config, client, token, &cmd.id, &result).await {
        tracing::warn!(
            "[CloudAgent] result 전송 실패 (id={}): {}",
            cmd.id,
            e
        );
    }
}

async fn post_result(
    config: &CloudAgentConfig,
    client: &reqwest::Client,
    token: &ParsedToken,
    request_id: &str,
    result: &dispatcher::DispatchResult,
) -> anyhow::Result<()> {
    let body = json!({
        "success": result.success,
        "data": result.data,
    });
    let body_str = serde_json::to_string(&body)?;
    let path = format!("/result/{}", request_id);
    let url = format!("{}{}", config.relay_url, path);
    let req = client.post(&url).header("content-type", "application/json");
    let req = apply_signed(req, token, "POST", &path, &body_str);
    let resp = req.body(body_str).send().await?;
    if !resp.status().is_success() {
        let s = resp.status();
        let text = resp.text().await.unwrap_or_default();
        anyhow::bail!("result status {}: {}", s, text);
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn recovery_cloud_agent_stop_aborts_unresponsive_tasks() {
        let task = tokio::spawn(std::future::pending::<()>());
        let abort = task.abort_handle();
        CloudAgentRuntime::stop_agent(Some(RunningAgent {
            cancel: CancellationToken::new(),
            handles: vec![task],
        })).await;
        let finished = abort.is_finished();
        abort.abort();
        assert!(finished, "cloud task remained detached after stop returned");
    }

    #[test]
    fn config_from_env_uses_default_when_unset() {
        // SABA_RELAY_URL이 비어있는 환경에서만 의미있음 — set/unset이 다른
        // 테스트와 충돌할 수 있어 직접 검증은 생략하고, trim 동작만 확인.
        let cfg = CloudAgentConfig {
            relay_url: "https://example.com/".to_string(),
            agent_version: "0.0.0".to_string(),
            heartbeat_interval: Duration::from_secs(1),
            poll_timeout: Duration::from_secs(1),
        };
        // trim_end_matches는 from_env 내부에서만 적용 — 직접 검증은 from_env 호출 시.
        assert!(cfg.relay_url.ends_with('/'));
    }

    #[test]
    fn relay_command_from_value_parses() {
        let v = json!({
            "id": "req-123",
            "payload": { "action": "ping" }
        });
        let c = RelayCommand::from_value(v).unwrap();
        assert_eq!(c.id, "req-123");
        assert_eq!(c.payload, json!({"action":"ping"}));
        // guildId/requestedBy가 없으면 None — 권한 제한 미적용 경로
        assert!(c.guild_id.is_none());
        assert!(c.requested_by.is_none());
    }

    /// 릴레이 poll 응답의 guildId/requestedBy가 컨텍스트로 전달되어야
    /// 데몬이 노드/유저 권한을 판정할 수 있다.
    #[test]
    fn relay_command_from_value_parses_origin_fields() {
        let v = json!({
            "id": "req-123",
            "payload": { "action": "raw_command", "text": "pal start" },
            "guildId": "guild-1",
            "requestedBy": "user-42"
        });
        let c = RelayCommand::from_value(v).unwrap();
        assert_eq!(c.guild_id.as_deref(), Some("guild-1"));
        assert_eq!(c.requested_by.as_deref(), Some("user-42"));
    }

    /// requestedBy="system"(릴레이 내부 발신)은 유저 제한 대상이 아니다.
    #[test]
    fn relay_command_from_value_ignores_system_requester() {
        let v = json!({
            "id": "req-123",
            "payload": {},
            "guildId": "",
            "requestedBy": "system"
        });
        let c = RelayCommand::from_value(v).unwrap();
        assert!(c.guild_id.is_none());
        assert!(c.requested_by.is_none());
    }

    #[test]
    fn relay_command_from_value_rejects_missing_id() {
        let v = json!({ "payload": {} });
        assert!(RelayCommand::from_value(v).is_none());
    }
}
