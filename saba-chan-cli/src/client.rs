//! 데몬 HTTP API — CLI가 쓰는 모든 계약이 이 파일 한 곳에 있다
//!
//! 응답 정규화: HTTP 오류뿐 아니라 200인데 `ok:false`/`success:false`인 응답도 `CliError::Api`로 바꾼다.
//! 오류 문구는 데몬이 준 순서대로 `error_message`(모듈의 사용자용 문구) → `error` → `message`.

use crate::error::{CliError, Result};
use reqwest::Method;
use serde_json::{json, Value};
use std::sync::{Arc, RwLock};
use std::time::Duration;

/// 짧은 조회
const QUICK: Duration = Duration::from_secs(10);
/// 원격 목록 조회·모듈 호출처럼 몇 초 걸리는 것
const SLOW: Duration = Duration::from_secs(60);
/// 설치·다운로드
const LONG: Duration = Duration::from_secs(30 * 60);
/// 정지·재시작: 데몬은 인스턴스 `stop_timeout`(최대 3600초)까지 저장 후 종료를 기다린 뒤 답한다 (MK14, L74)
const STOP: Duration = Duration::from_secs(3600 + 10 * 60);

#[derive(Clone)]
pub struct Api {
    http: reqwest::Client,
    base: String,
    /// 데몬 포트 (데몬을 띄울 때 그대로 넘긴다)
    pub port: u16,
    token: Arc<RwLock<Option<String>>>,
    /// 데몬 신원 증명을 확인한 토큰. 같은 토큰이면 요청마다 다시 묻지 않는다
    verified: Arc<RwLock<Option<String>>>,
    /// 요청마다 싣는 `Accept-Language` (`--lang`·`SABA_LANG`으로 고른 언어). 없으면 데몬이 자기 설정을 따른다
    pub lang: Option<String>,
    /// 이 실행에서 마지막으로 보인 포트 충돌 정지 알림 id (L50)
    notices_seen: Arc<std::sync::atomic::AtomicU64>,
}

fn read_token() -> Option<String> {
    std::fs::read_to_string(saba_chan_updater_lib::constants::token_file_path())
        .ok()
        .map(|s| s.trim().to_string())
        .filter(|s| !s.is_empty())
}

/// 경로 조각 하나를 URL에 넣을 수 있게
pub fn seg(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    for b in s.bytes() {
        if b.is_ascii_alphanumeric() || matches!(b, b'-' | b'_' | b'.' | b'~') {
            out.push(b as char);
        } else {
            out.push_str(&format!("%{:02X}", b));
        }
    }
    out
}

impl Api {
    pub fn new(port: u16, lang: Option<String>) -> Self {
        let mut headers = reqwest::header::HeaderMap::new();
        if let Some(v) = lang.as_deref().and_then(|l| reqwest::header::HeaderValue::from_str(l).ok()) {
            headers.insert(reqwest::header::ACCEPT_LANGUAGE, v);
        }
        let http = reqwest::Client::builder()
            .connect_timeout(Duration::from_secs(2))
            .no_proxy()
            .default_headers(headers)
            .build()
            .expect("http client");
        Self {
            http,
            base: format!("http://127.0.0.1:{}", port),
            port,
            token: Arc::new(RwLock::new(read_token())),
            verified: Arc::default(),
            lang,
            notices_seen: Arc::default(),
        }
    }

    pub fn base_url(&self) -> &str {
        &self.base
    }

    async fn send(
        &self,
        method: &Method,
        path: &str,
        body: Option<&Value>,
        timeout: Duration,
    ) -> Result<reqwest::Response> {
        let mut req = self
            .http
            .request(method.clone(), format!("{}{}", self.base, path))
            .timeout(timeout);
        if let Some(t) = self.token.read().ok().and_then(|t| t.clone()) {
            req = req.header("X-Saba-Token", t);
        }
        // 감사 로그의 출처 (A10)
        req = req.header("X-Saba-Client", "cli");
        if let Some(b) = body {
            req = req.json(b);
        }
        req.send().await.map_err(|e| self.send_error(e, path))
    }

    fn send_error(&self, e: reqwest::Error, path: &str) -> anyhow::Error {
        if e.is_connect() {
            CliError::DaemonDown { url: self.base.clone() }.into()
        } else if e.is_timeout() {
            CliError::Failed(t!("err.timeout", path = path)).into()
        } else {
            anyhow::anyhow!(e)
        }
    }

    async fn daemon_identity_valid(&self) -> Result<bool> {
        let token = self.token.read().ok().and_then(|t| t.clone()).unwrap_or_default();
        if token.is_empty() { return Ok(false); }
        if self.verified.read().ok().is_some_and(|v| v.as_deref() == Some(token.as_str())) { return Ok(true); }
        let challenge = uuid::Uuid::new_v4().to_string();
        let response = self.http.get(format!("{}/health", self.base)).timeout(QUICK)
            .header("X-Saba-Challenge", &challenge).send().await.map_err(|e| self.send_error(e, "/health"))?;
        let body: Value = response.json().await?;
        let Some(proof) = body.get("proof").and_then(Value::as_str) else { return Ok(false) };
        use hmac::{Hmac, Mac};
        let mut mac = Hmac::<sha2::Sha256>::new_from_slice(token.as_bytes())?;
        mac.update(challenge.as_bytes());
        let valid = mac.verify_slice(&hex::decode(proof)?).is_ok();
        if valid {
            if let Ok(mut verified) = self.verified.write() { *verified = Some(token); }
        }
        Ok(valid)
    }

    pub async fn call(&self, method: Method, path: &str, body: Option<Value>, timeout: Duration) -> Result<Value> {
        if self.token.read().ok().and_then(|t| t.clone()).is_some() && !self.daemon_identity_valid().await? {
            if let Ok(mut token) = self.token.write() { *token = read_token(); }
            if !self.daemon_identity_valid().await? { anyhow::bail!("IPC endpoint identity check failed"); }
        }
        let mut resp = self.send(&method, path, body.as_ref(), timeout).await?;
        // 데몬이 다시 뜨면 토큰이 바뀐다 → 한 번 다시 읽고 재시도
        if resp.status() == reqwest::StatusCode::UNAUTHORIZED {
            let fresh = read_token();
            if let Ok(mut t) = self.token.write() {
                *t = fresh;
            }
            if self.token.read().ok().and_then(|t| t.clone()).is_some() && !self.daemon_identity_valid().await? {
                anyhow::bail!("IPC endpoint identity check failed");
            }
            resp = self.send(&method, path, body.as_ref(), timeout).await?;
        }
        let status = resp.status().as_u16();
        // 본문을 다 받지 못했으면 빈 응답으로 성공 처리하지 않는다
        let text = resp
            .text()
            .await
            .map_err(|e| CliError::Failed(t!("err.body_read", path = path, error = e)))?;
        let value: Value = if text.trim().is_empty() {
            Value::Null
        } else {
            serde_json::from_str(&text).unwrap_or_else(|_| Value::String(text.clone()))
        };
        check(status, value)
    }

    async fn get(&self, path: &str) -> Result<Value> {
        self.call(Method::GET, path, None, QUICK).await
    }
    async fn get_slow(&self, path: &str) -> Result<Value> {
        self.call(Method::GET, path, None, SLOW).await
    }
    async fn post(&self, path: &str, body: Value) -> Result<Value> {
        self.call(Method::POST, path, Some(body), SLOW).await
    }
    async fn post_long(&self, path: &str, body: Value) -> Result<Value> {
        self.call(Method::POST, path, Some(body), LONG).await
    }
    async fn put(&self, path: &str, body: Value) -> Result<Value> {
        self.call(Method::PUT, path, Some(body), QUICK).await
    }
    async fn put_slow(&self, path: &str, body: Value) -> Result<Value> {
        self.call(Method::PUT, path, Some(body), SLOW).await
    }
    async fn patch(&self, path: &str, body: Value) -> Result<Value> {
        self.call(Method::PATCH, path, Some(body), SLOW).await
    }
    async fn delete(&self, path: &str) -> Result<Value> {
        self.call(Method::DELETE, path, None, SLOW).await
    }

    // ── 데몬 ──

    /// GET /health — 락 없이 답하는 ping (`{ok, version, pid, token, stops_with_launcher}`)
    pub async fn health(&self) -> Result<Value> {
        // 연결 제한(2초)보다 길게 둔다: Windows는 닫힌 루프백 포트 거부에 2초쯤 걸려서,
        // 같으면 "데몬 꺼짐"이 아니라 "응답 늦음"으로 끝난다
        self.call(Method::GET, "/health", None, Duration::from_secs(5)).await
    }
    /// GET /api/version — 코어·모듈·익스텐션 버전
    pub async fn version(&self) -> Result<Value> {
        self.get("/api/version").await
    }
    /// GET /api/system/components — 릴리스 버전과 설치된 구성요소 `{version, components, lastUpdated}`
    pub async fn system_components(&self) -> Result<Value> {
        self.get("/api/system/components").await
    }
    /// GET /api/system/paths — 폴더 위치 `{data_dir, install_dir, default_install_dir, instances_dir, modules: {path, default, source}, extensions}`
    pub async fn paths(&self) -> Result<Value> {
        self.get("/api/system/paths").await
    }
    /// POST /api/system/paths/move `{kind, target}` — 파일을 복사·검증해 옮기므로 오래 걸릴 수 있다
    pub async fn move_dir(&self, kind: &str, target: &str) -> Result<Value> {
        self.post_long("/api/system/paths/move", json!({ "kind": kind, "target": target })).await
    }
    /// GET /api/system/autostart — 로그인할 때 자동 실행 `{supported, mode: "off"|"daemon"|"full"}`
    pub async fn autostart(&self) -> Result<Value> {
        self.get("/api/system/autostart").await
    }
    /// PUT /api/system/autostart `{mode}`
    pub async fn set_autostart(&self, mode: &str) -> Result<Value> {
        self.put("/api/system/autostart", json!({ "mode": mode })).await
    }
    /// GET /api/system/metrics — 시스템 모니터 표본
    pub async fn system_metrics(&self) -> Result<Value> {
        self.get("/api/system/metrics").await
    }
    /// POST /api/daemon/shutdown
    pub async fn shutdown(&self) -> Result<Value> {
        self.post("/api/daemon/shutdown", json!({})).await
    }
    /// GET /api/clients — 데몬에 붙어 있는 GUI 창들 `{clients: [{id, kind, pid, installRoot}]}`
    pub async fn clients(&self) -> Result<Value> {
        self.get("/api/clients").await
    }
    /// GET /api/daemon/console?since=&count=
    pub async fn daemon_console(&self, since: Option<u64>, count: Option<usize>) -> Result<Value> {
        self.daemon_console_tokens(since, count, false).await
    }

    /// 데몬 로그. `spans`면 줄마다 구문 강조 토큰을 붙여 받는다 (TUI는 색 코드를 끄고도 칠한다)
    pub async fn daemon_console_tokens(&self, since: Option<u64>, count: Option<usize>, spans: bool) -> Result<Value> {
        self.get(&with_query(
            "/api/daemon/console",
            &[
                ("since", since.map(|v| v.to_string())),
                ("count", count.map(|v| v.to_string())),
                ("spans", spans.then(|| "1".to_string())),
            ],
        ))
        .await
    }

    /// 이 PC에서 띄울 수 있는 인터페이스 `{cli, gui}` (GUI로 전환 버튼을 보일지)
    pub async fn launchable(&self) -> Result<Value> {
        self.get("/api/clients/launch").await
    }

    /// 데몬이 다른 인터페이스(`cli`·`gui`)를 띄운다. `instance`면 그 서버를 골라 연다
    pub async fn launch(&self, kind: &str, instance: Option<&str>) -> Result<Value> {
        self.post("/api/clients/launch", serde_json::json!({ "kind": kind, "instance": instance })).await
    }

    // ── 서버 (런타임 상태) ──

    /// GET /api/servers — 상태가 붙은 서버 목록 (익스텐션 hook 포함이라 느릴 수 있다).
    /// 포트 충돌로 멈춘 알림(`port_conflict_stops`)은 데몬에 남는다: 알림을 보여 주는 `status`는 이 실행에서 본 것 뒤만,
    /// 나머지는 `keep`으로 남은 것 모두를 받는다
    pub async fn servers(&self, keep_notices: bool) -> Result<Value> {
        let mut body = self.get_slow(&servers_path(keep_notices, &self.notices_seen)).await?;
        if !keep_notices {
            remember_notices(&self.notices_seen, &mut body);
        }
        Ok(body)
    }
    /// POST /api/instance/:id/start — managed 여부는 데몬이 고른다
    pub async fn start(&self, id: &str) -> Result<Value> {
        self.post_long(&format!("/api/instance/{}/start", seg(id)), json!({}))
            .await
    }
    /// POST /api/instance/:id/stop
    pub async fn stop(&self, id: &str, force: bool) -> Result<Value> {
        self.call(Method::POST, &format!("/api/instance/{}/stop", seg(id)), Some(json!({ "force": force })), STOP)
            .await
    }
    /// POST /api/instance/:id/restart — 멈추고(stop_timeout까지) 다시 켠다. 꺼져 있었으면 켜기만. 응답은 시작 응답 + `stopped`
    pub async fn restart(&self, id: &str, force: bool) -> Result<Value> {
        self.call(Method::POST, &format!("/api/instance/{}/restart", seg(id)), Some(json!({ "force": force })), STOP)
            .await
    }
    /// POST /api/instance/:id/rename — 409 name_taken / server_running
    pub async fn rename(&self, id: &str, name: &str) -> Result<Value> {
        self.post(&format!("/api/instance/{}/rename", seg(id)), json!({ "name": name }))
            .await
    }
    /// GET /api/instance/:id/commands — 모듈 + 켜진 익스텐션 명령
    pub async fn instance_commands(&self, id: &str) -> Result<Value> {
        self.get(&format!("/api/instance/{}/commands", seg(id))).await
    }
    /// GET /api/instance/:id/events — 종료 기록 (A1)
    pub async fn exit_events(&self, id: &str) -> Result<Value> {
        self.get(&format!("/api/instance/{}/events", seg(id))).await
    }

    // ── 인스턴스 (설정) ──

    /// GET /api/instances
    pub async fn instances(&self) -> Result<Vec<Value>> {
        Ok(self
            .get("/api/instances")
            .await?
            .as_array()
            .cloned()
            .unwrap_or_default())
    }
    /// POST /api/instances — `{name, module_name, port?, migration_source?, executable_path?, working_dir?, module_settings?}`
    pub async fn create_instance(&self, body: Value) -> Result<Value> {
        self.post_long("/api/instances", body).await
    }
    /// DELETE /api/instance/:id (`keep_settings`: 가져온 서버의 게임 설정 파일을 되돌리지 않는다)
    pub async fn delete_instance(&self, id: &str, keep_settings: bool) -> Result<Value> {
        let query = if keep_settings { "?keep_settings=true" } else { "" };
        self.delete(&format!("/api/instance/{}{}", seg(id), query)).await
    }
    /// PATCH /api/instance/:id — 공통 필드·모듈 설정·`extension_data` 부분 변경
    pub async fn update_instance(&self, id: &str, patch: Value) -> Result<Value> {
        self.patch(&format!("/api/instance/{}", seg(id)), patch).await
    }
    /// PUT /api/instances/reorder — `{order: [id...]}`
    pub async fn reorder(&self, ids: &[String]) -> Result<Value> {
        self.put("/api/instances/reorder", json!({ "order": ids })).await
    }
    /// GET /api/provision-progress/:name
    pub async fn provision_progress(&self, name: &str) -> Result<Value> {
        self.get(&format!("/api/provision-progress/{}", seg(name))).await
    }
    /// DELETE /api/provision-progress/:name — 실패한 새 서버면 데몬이 롤백한다
    pub async fn dismiss_provision(&self, name: &str) -> Result<Value> {
        self.delete(&format!("/api/provision-progress/{}", seg(name))).await
    }
    /// GET /api/instance/:id/check-update
    pub async fn check_server_update(&self, id: &str) -> Result<Value> {
        self.get_slow(&format!("/api/instance/{}/check-update", seg(id))).await
    }
    /// POST /api/instance/:id/apply-update
    pub async fn apply_server_update(&self, id: &str) -> Result<Value> {
        self.post_long(&format!("/api/instance/{}/apply-update", seg(id)), json!({}))
            .await
    }
    /// GET /api/instance/:id/installed-version
    pub async fn installed_version(&self, id: &str) -> Result<Value> {
        self.get_slow(&format!("/api/instance/{}/installed-version", seg(id)))
            .await
    }

    // ── 명령 ──

    /// POST /api/instance/:id/command — 모듈 명령 카탈로그로 해석된다 (이름·인자 또는 한 줄)
    pub async fn command(&self, id: &str, command: &str, args: Value) -> Result<Value> {
        self.post(
            &format!("/api/instance/{}/command", seg(id)),
            json!({ "command": command, "args": args }),
        )
        .await
    }
    /// POST /api/instance/:id/rest — `{endpoint, method, body?}`
    pub async fn rest(&self, id: &str, method: &str, endpoint: &str, body: Option<Value>) -> Result<Value> {
        let mut req = json!({ "endpoint": endpoint, "method": method });
        if let Some(b) = body {
            req["body"] = b;
        }
        self.post(&format!("/api/instance/{}/rest", seg(id)), req).await
    }
    /// POST /api/instance/:id/stdin — 콘솔 한 줄 그대로
    pub async fn stdin(&self, id: &str, line: &str) -> Result<Value> {
        self.post(&format!("/api/instance/{}/stdin", seg(id)), json!({ "command": line }))
            .await
    }
    /// POST /api/instance/:id/rcon `{command}` — 한 줄을 서버가 받는 길(콘솔, 없으면 RCON)로 보내고 답을 받는다
    pub async fn rcon(&self, id: &str, line: &str) -> Result<Value> {
        self.post(&format!("/api/instance/{}/rcon", seg(id)), json!({ "command": line }))
            .await
    }
    /// GET /api/instance/:id/players — `{online, max?, players: [{name}]}`. 꺼져 있으면 409 server_not_running
    pub async fn players(&self, id: &str) -> Result<Value> {
        self.get_slow(&format!("/api/instance/{}/players", seg(id))).await
    }
    /// GET /api/instance/:id/logs?source=&lines=&before= — 저장된 실행 로그 `{sources, source, lines, before?}`
    pub async fn logs(&self, id: &str, source: Option<&str>, lines: Option<usize>, before: Option<u64>) -> Result<Value> {
        self.get(&with_query(
            &format!("/api/instance/{}/logs", seg(id)),
            &[
                ("source", source.map(str::to_string)),
                ("lines", lines.map(|v| v.to_string())),
                ("before", before.map(|v| v.to_string())),
            ],
        ))
        .await
    }
    /// GET /api/instance/:id/logs/export?source= — 로그 한 실행분 전체 `{fileName, bytes, truncated, content}`
    pub async fn export_log(&self, id: &str, source: Option<&str>) -> Result<Value> {
        self.get_slow(&with_query(&format!("/api/instance/{}/logs/export", seg(id)), &[("source", source.map(str::to_string))]))
            .await
    }
    /// POST /api/instance/:id/query/:name `{args}` — 모듈이 선언한 조회 (`[[queries]]`) `{success, data}`
    pub async fn query(&self, id: &str, name: &str, args: Value) -> Result<Value> {
        self.post(&format!("/api/instance/{}/query/{}", seg(id), seg(name)), json!({ "args": args }))
            .await
    }
    /// GET /api/instance/:id/console?since=&count=&spans=1 (색을 쓸 때만 하이라이팅 구간을 받는다)
    pub async fn console(&self, id: &str, since: Option<u64>, count: Option<usize>) -> Result<Value> {
        self.console_tokens(id, since, count, console::colors_enabled()).await
    }

    /// 서버 콘솔. `spans`면 줄마다 구문 강조 토큰을 붙여 받는다
    pub async fn console_tokens(&self, id: &str, since: Option<u64>, count: Option<usize>, spans: bool) -> Result<Value> {
        self.get(&with_query(
            &format!("/api/instance/{}/console", seg(id)),
            &[
                ("since", since.map(|v| v.to_string())),
                ("count", count.map(|v| v.to_string())),
                ("spans", spans.then(|| "1".to_string())),
            ],
        ))
        .await
    }

    // ── 인스턴스 도구 (모듈 lifecycle 호출) ──

    pub async fn validate(&self, id: &str) -> Result<Value> {
        self.post(&format!("/api/instance/{}/validate", seg(id)), json!({}))
            .await
    }
    pub async fn diagnose(&self, id: &str) -> Result<Value> {
        self.post(&format!("/api/instance/{}/diagnose", seg(id)), json!({}))
            .await
    }
    /// POST /api/instance/:id/launch-preview — 실행 줄 미리보기. extra가 없으면 저장된 추가 인자
    pub async fn launch_preview(&self, id: &str, extra: Option<&str>) -> Result<Value> {
        let mut body = json!({ "settings": {} });
        if let Some(extra) = extra {
            body["extra_args"] = json!(extra);
        }
        self.post(&format!("/api/instance/{}/launch-preview", seg(id)), body).await
    }
    /// GET /api/instance/:id — 레코드와 모듈 약관 `agreements`
    pub async fn instance(&self, id: &str) -> Result<Value> {
        self.get(&format!("/api/instance/{}", seg(id))).await
    }
    /// POST /api/instance/:id/agreements — `{accept:[id]}`
    pub async fn accept_agreements(&self, id: &str, ids: &[String]) -> Result<Value> {
        self.post(&format!("/api/instance/{}/agreements", seg(id)), json!({ "accept": ids }))
            .await
    }
    /// POST /api/instance/:id/server/reset — 서버 파일 초기화
    pub async fn reset_server(&self, id: &str) -> Result<Value> {
        self.post_long(&format!("/api/instance/{}/server/reset", seg(id)), json!({}))
            .await
    }
    /// GET /api/instance/:id/settings/live — 실제 설정 파일에서 읽는다 (캐시도 갱신) `{success, settings}`
    pub async fn live_settings(&self, id: &str) -> Result<Value> {
        self.get_slow(&format!("/api/instance/{}/settings/live", seg(id))).await
    }
    /// GET /api/runtime/preflight — 부팅 준비(Python 등) `{phase, percent, complete, error, components}`
    pub async fn preflight(&self) -> Result<Value> {
        self.get("/api/runtime/preflight").await
    }
    /// GET /api/diagnostics — 데몬 진단 (경로·토큰 파일·모듈·익스텐션·최근 경고)
    pub async fn diagnostics(&self) -> Result<Value> {
        self.get("/api/diagnostics").await
    }
    /// GET /api/onboarding — 다음 할 일 `{steps, next}`
    pub async fn onboarding(&self) -> Result<Value> {
        self.get("/api/onboarding").await
    }
    /// GET /api/instance/:id/properties — 게임 설정 파일
    pub async fn properties(&self, id: &str) -> Result<Value> {
        self.get_slow(&format!("/api/instance/{}/properties", seg(id))).await
    }
    /// PUT /api/instance/:id/properties — `{settings: {...}}`
    pub async fn write_properties(&self, id: &str, settings: Value) -> Result<Value> {
        self.call(
            Method::PUT,
            &format!("/api/instance/{}/properties", seg(id)),
            Some(json!({ "settings": settings })),
            SLOW,
        )
        .await
    }
    pub async fn reset_properties(&self, id: &str) -> Result<Value> {
        self.post(&format!("/api/instance/{}/properties/reset", seg(id)), json!({}))
            .await
    }

    // ── 모듈 ──

    /// GET /api/modules — `{modules: [...], failed: [...]}`
    pub async fn modules(&self) -> Result<Value> {
        self.get("/api/modules").await
    }
    pub async fn refresh_modules(&self) -> Result<Value> {
        self.post("/api/modules/refresh", json!({})).await
    }
    /// GET /api/modules/schema — module.toml JSON Schema
    /// GET /api/module/:name/audit — 코드 검사 `{declared, findings, undeclared}`
    pub async fn module_audit(&self, name: &str) -> Result<Value> {
        self.get_slow(&format!("/api/module/{}/audit", seg(name))).await
    }
    /// GET /api/extensions/:id/audit — 모듈 검사와 같은 모양
    pub async fn extension_audit(&self, id: &str) -> Result<Value> {
        self.get_slow(&format!("/api/extensions/{}/audit", seg(id))).await
    }
    pub async fn module_schema(&self) -> Result<Value> {
        self.get("/api/modules/schema").await
    }
    /// GET /api/maker — 모듈 메이커 안내: 이 PC의 경로와 데몬 계약
    pub async fn maker(&self) -> Result<Value> {
        self.get("/api/maker").await
    }
    /// POST /api/modules/detect — 폴더가 어떤 게임 서버인지
    pub async fn detect(&self, path: &str) -> Result<Value> {
        self.post("/api/modules/detect", json!({ "path": path })).await
    }
    /// GET /api/modules/manifest — 원격 모듈 목록
    pub async fn module_manifest(&self) -> Result<Value> {
        self.get_slow("/api/modules/manifest").await
    }
    pub async fn install_module(&self, id: &str) -> Result<Value> {
        self.post_long(&format!("/api/modules/manifest/{}/install", seg(id)), json!({}))
            .await
    }
    pub async fn remove_module(&self, id: &str) -> Result<Value> {
        self.delete(&format!("/api/modules/{}", seg(id))).await
    }
    /// GET /api/module/:name/locales — 모듈 문구 (라벨 번역)
    pub async fn module_locales(&self, name: &str) -> Result<Value> {
        self.get(&format!("/api/module/{}/locales", seg(name))).await
    }
    /// GET /api/module/:name — 모듈 하나의 전체 선언 `{name, version, description, path, metadata}`
    pub async fn module_metadata(&self, name: &str) -> Result<Value> {
        self.get(&format!("/api/module/{}", seg(name))).await
    }
    /// GET /api/module/:name/versions
    pub async fn module_versions(&self, name: &str, snapshots: bool, page: u32, per_page: u32) -> Result<Value> {
        self.get_slow(&format!(
            "/api/module/{}/versions?include_snapshots={}&page={}&per_page={}",
            seg(name),
            snapshots,
            page,
            per_page
        ))
        .await
    }
    pub async fn module_version(&self, name: &str, version: &str) -> Result<Value> {
        self.get_slow(&format!("/api/module/{}/version/{}", seg(name), seg(version)))
            .await
    }
    /// POST /api/module/:name/install — 서버 파일만 받는다 `{version, install_dir, server_executable?, accept_agreements?}`
    pub async fn install_server_files(&self, name: &str, body: Value) -> Result<Value> {
        self.post_long(&format!("/api/module/{}/install", seg(name)), body)
            .await
    }

    // ── 익스텐션 ──

    pub async fn extensions(&self) -> Result<Value> {
        // 설명은 CLI 표시 언어로 (데몬은 없으면 GUI 언어를 쓴다)
        self.get(&with_query(
            "/api/extensions",
            &[("lang", Some(crate::i18n::lang().to_string()))],
        ))
        .await
    }
    pub async fn extension_init_status(&self) -> Result<Value> {
        self.get("/api/extensions/init-status").await
    }
    pub async fn rescan_extensions(&self) -> Result<Value> {
        self.post("/api/extensions/rescan", json!({})).await
    }
    pub async fn extension_manifest(&self) -> Result<Value> {
        self.get_slow("/api/extensions/manifest").await
    }
    pub async fn extension_updates(&self) -> Result<Value> {
        self.get_slow("/api/extensions/updates").await
    }
    pub async fn enable_extension(&self, id: &str) -> Result<Value> {
        self.post(&format!("/api/extensions/{}/enable", seg(id)), json!({}))
            .await
    }
    pub async fn disable_extension(&self, id: &str) -> Result<Value> {
        self.post(&format!("/api/extensions/{}/disable", seg(id)), json!({}))
            .await
    }
    /// POST /api/extensions/:id/mount — 디스크에서 다시 읽어 등록
    pub async fn mount_extension(&self, id: &str) -> Result<Value> {
        self.post(&format!("/api/extensions/{}/mount", seg(id)), json!({}))
            .await
    }
    pub async fn unmount_extension(&self, id: &str) -> Result<Value> {
        self.post(&format!("/api/extensions/{}/unmount", seg(id)), json!({}))
            .await
    }
    /// POST /api/extensions/:id/install — 설치·업데이트 `{download_url?, sha256?}`
    pub async fn install_extension(&self, id: &str, body: Value) -> Result<Value> {
        self.post_long(&format!("/api/extensions/{}/install", seg(id)), body)
            .await
    }
    pub async fn remove_extension(&self, id: &str) -> Result<Value> {
        self.delete(&format!("/api/extensions/{}", seg(id))).await
    }
    pub async fn extension_config(&self, id: &str) -> Result<Value> {
        self.get(&format!("/api/extensions/{}/config", seg(id))).await
    }
    /// PUT /api/extensions/:id/config — 키 단위 머지, null이면 삭제
    pub async fn save_extension_config(&self, id: &str, values: Value) -> Result<Value> {
        self.put(&format!("/api/extensions/{}/config", seg(id)), values).await
    }

    // ── 설정 ──

    /// GET/PUT /api/config/gui — 앱 전체 설정 (이름은 gui지만 GUI·CLI·데몬이 같이 쓴다)
    pub async fn app_config(&self) -> Result<Value> {
        self.get("/api/config/gui").await
    }
    pub async fn save_app_config(&self, full: Value) -> Result<Value> {
        self.put("/api/config/gui", full).await
    }
    /// PATCH /api/config/gui — merge patch (null은 지운다). `revision`을 실으면 그사이 바뀐 설정에는 쓰지 않고 409 `settings_changed`
    pub async fn patch_app_config(&self, patch: Value) -> Result<Value> {
        self.patch("/api/config/gui", patch).await
    }
    /// GET /api/audit — `{entries: [{ts, source, user?, guild?, action, instanceId, instance?, command?, args?, ok, status}]}` 최신이 먼저
    pub async fn audit(&self, instance: Option<&str>, limit: usize) -> Result<Value> {
        self.get(&with_query(
            "/api/audit",
            &[("limit", Some(limit.to_string())), ("instance", instance.map(str::to_string))],
        ))
        .await
    }
    /// GET /api/events — 데몬 이벤트 스트림 (P4, SSE). `since`는 마지막으로 받은 번호
    pub async fn events(&self, since: Option<u64>) -> Result<Events> {
        let mut resp = self.open_events(since).await?;
        // 데몬이 다시 뜨면 토큰이 바뀐다 → 한 번 다시 읽고 재시도 (`call`과 같다)
        if resp.status() == reqwest::StatusCode::UNAUTHORIZED {
            let fresh = read_token();
            if let Ok(mut t) = self.token.write() {
                *t = fresh;
            }
            resp = self.open_events(since).await?;
        }
        if !resp.status().is_success() {
            let status = resp.status().as_u16();
            let value = resp.json().await.unwrap_or(Value::Null);
            check(status, value)?;
            return Err(anyhow::anyhow!("GET /api/events: {}", status));
        }
        Ok(Events { resp, buf: Vec::new() })
    }

    async fn open_events(&self, since: Option<u64>) -> Result<reqwest::Response> {
        let mut req = self.http.get(format!("{}/api/events", self.base)).header("X-Saba-Client", "cli");
        if let Some(t) = self.token.read().ok().and_then(|t| t.clone()) {
            req = req.header("X-Saba-Token", t);
        }
        if let Some(n) = since {
            req = req.header("Last-Event-ID", n.to_string());
        }
        req.send().await.map_err(|e| -> anyhow::Error {
            if e.is_connect() {
                CliError::DaemonDown { url: self.base.clone() }.into()
            } else {
                anyhow::anyhow!(e)
            }
        })
    }
    /// GET /api/permissions/:guildId
    pub async fn permissions(&self, guild: &str) -> Result<Value> {
        self.get(&format!("/api/permissions/{}", seg(guild))).await
    }
    /// PATCH /api/permissions/:guildId `{defaultLevel?, preset?, chatbotCap?, ownerDiscordId?, commandLevels?}`
    pub async fn update_permissions(&self, guild: &str, patch: Value) -> Result<Value> {
        self.patch(&format!("/api/permissions/{}", seg(guild)), patch).await
    }
    /// PUT/DELETE /api/permissions/:guildId/members/:discordId (None이면 기본 레벨로)
    pub async fn set_member_level(&self, guild: &str, member: &str, level: Option<i64>) -> Result<Value> {
        let path = format!("/api/permissions/{}/members/{}", seg(guild), seg(member));
        match level {
            Some(level) => self.put(&path, json!({ "level": level })).await,
            None => self.delete(&path).await,
        }
    }
    /// GET /api/sandbox/{unisolated|extension-gui} — `{components:[{component, fingerprint, current}]}`
    pub async fn consents(&self, kind: &str) -> Result<Value> {
        self.get(&format!("/api/sandbox/{}", kind)).await
    }
    /// POST/DELETE /api/sandbox/{unisolated|extension-gui}/:component (격리 없이 돌리기, 익스텐션 GUI 번들 싣기 동의)
    pub async fn set_consent(&self, kind: &str, component: &str, allow: bool) -> Result<Value> {
        let path = format!("/api/sandbox/{}/{}", kind, seg(component));
        if allow {
            self.post(&path, json!({})).await
        } else {
            self.delete(&path).await
        }
    }
    /// GET·PUT /api/remote/settings — 원격 데몬 포트 (D1)
    pub async fn remote_settings(&self) -> Result<Value> {
        self.get("/api/remote/settings").await
    }
    pub async fn set_remote_settings(&self, body: Value) -> Result<Value> {
        self.put("/api/remote/settings", body).await
    }
    /// GET /api/remote/clients — 이 데몬에 짝 맺은 PC
    pub async fn remote_clients(&self) -> Result<Value> {
        self.get("/api/remote/clients").await
    }
    /// DELETE /api/remote/clients/:hostId
    pub async fn revoke_remote_client(&self, host_id: &str) -> Result<Value> {
        self.delete(&format!("/api/remote/clients/{}", seg(host_id))).await
    }
    /// GET /api/remote/peers — 릴레이에 있는 같은 주인의 다른 PC (D1b)
    pub async fn remote_peers(&self) -> Result<Value> {
        self.get("/api/remote/peers").await
    }
    /// GET·POST /api/remote/daemons — 이 데몬이 붙는 원격 데몬, 짝 맺기
    pub async fn remote_daemons(&self) -> Result<Value> {
        self.get("/api/remote/daemons").await
    }
    pub async fn connect_remote_daemon(&self, host_id: &str) -> Result<Value> {
        self.post("/api/remote/daemons", json!({ "hostId": host_id })).await
    }
    /// DELETE /api/remote/daemons/:hostId
    pub async fn forget_remote_daemon(&self, host_id: &str) -> Result<Value> {
        self.delete(&format!("/api/remote/daemons/{}", seg(host_id))).await
    }
    /// GET /api/actions?surface=cli&instance&locale (P2)
    pub async fn actions(&self, instance: Option<&str>) -> Result<Value> {
        let mut path = "/api/actions?surface=cli".to_string();
        if let Some(i) = instance {
            path.push_str(&format!("&instance={}", seg(i)));
        }
        path.push_str(&format!("&locale={}", seg(crate::i18n::lang())));
        self.get(&path).await
    }
    /// POST /api/actions/:id/plan·run `{input, instance?, locale}` 또는 `{planId, locale}`: 요약 문구는 CLI가 고른 언어로
    pub async fn action(&self, id: &str, step: &str, mut body: Value) -> Result<Value> {
        if let Some(m) = body.as_object_mut() {
            m.entry("locale").or_insert_with(|| json!(crate::i18n::lang()));
        }
        self.post_long(&format!("/api/actions/{}/{}", seg(id), step), body).await
    }
    /// POST /api/actions/plan `{steps: [{id, instance?, input}], locale}` — 여러 단계 계획. 실행은 `action("plan", "run", {planId})`
    pub async fn plan_steps(&self, steps: Value) -> Result<Value> {
        self.post("/api/actions/plan", json!({ "steps": steps, "locale": crate::i18n::lang() })).await
    }
    /// GET /api/actions/runs/:runId — 뒤에서 이어 가는 여러 단계 실행의 진행
    pub async fn action_run(&self, run_id: &str) -> Result<Value> {
        self.get(&format!("/api/actions/runs/{}", seg(run_id))).await
    }
    /// POST /api/migration/export — 데몬이 `path`에 zip을 쓴다
    pub async fn export_config(&self, path: &str, secrets: bool) -> Result<Value> {
        self.post_long("/api/migration/export", json!({ "path": path, "secrets": secrets }))
            .await
    }
    /// POST /api/migration/import — `{instances: {added, replaced, skipped}, missingModules, config, nodeToken}`
    pub async fn import_config(&self, path: &str, dry_run: bool, overwrite: bool) -> Result<Value> {
        self.post_long(
            "/api/migration/import",
            json!({ "path": path, "dryRun": dry_run, "overwrite": overwrite }),
        )
        .await
    }
    /// GET/PUT /api/config/client/cli — CLI 전용 네임스페이스 (최상위 키 머지)
    pub async fn cli_config(&self) -> Result<Value> {
        self.get("/api/config/client/cli").await
    }
    pub async fn save_cli_config(&self, patch: Value) -> Result<Value> {
        self.put("/api/config/client/cli", patch).await
    }
    /// GET/PUT /api/config/bot — 디스코드 연동 설정 (전체 저장)
    /// GET /api/config/:kind/defaults (gui | bot)
    pub async fn config_defaults(&self, kind: &str) -> Result<Value> {
        self.get(&format!("/api/config/{}/defaults", seg(kind))).await
    }
    /// POST /api/config/:kind/reset — `keys`(최상위 키)만, 비우면 전부 기본값으로
    /// 키가 없으면 전체 초기화 (`keys`를 빼고 보낸다. 빈 배열은 400)
    pub async fn config_reset(&self, kind: &str, keys: &[String]) -> Result<Value> {
        let body = if keys.is_empty() {
            json!({})
        } else {
            json!({ "keys": keys })
        };
        self.post(&format!("/api/config/{}/reset", seg(kind)), body).await
    }
    pub async fn bot_config(&self) -> Result<Value> {
        self.get("/api/config/bot").await
    }
    pub async fn save_bot_config(&self, full: Value) -> Result<Value> {
        self.put("/api/config/bot", full).await
    }
    /// PATCH /api/config/bot — merge patch (null은 지운다). `revision`을 실으면 그사이 바뀐 설정에는 쓰지 않고 409 `settings_changed`
    pub async fn patch_bot_config(&self, patch: Value) -> Result<Value> {
        self.patch("/api/config/bot", patch).await
    }
    pub async fn save_node_token(&self, token: &str) -> Result<Value> {
        self.put("/api/config/node-token", json!({ "token": token })).await
    }
    pub async fn delete_node_token(&self) -> Result<Value> {
        self.delete("/api/config/node-token").await
    }

    // ── 디스코드 릴레이 ──

    /// GET /api/relay/agent — `{running, relayUrl, hasToken, lastHeartbeatAt, lastError, commandsHandled}`
    pub async fn agent(&self) -> Result<Value> {
        self.get("/api/relay/agent").await
    }
    pub async fn agent_start(&self) -> Result<Value> {
        self.post("/api/relay/agent/start", json!({})).await
    }
    pub async fn agent_stop(&self) -> Result<Value> {
        self.post("/api/relay/agent/stop", json!({})).await
    }
    /// POST /api/relay/pair/initiate — `{code, expiresAt, pollSecret}`
    pub async fn pair_initiate(&self, relay_url: Option<&str>) -> Result<Value> {
        let body = match relay_url {
            Some(u) => json!({ "relayUrl": u }),
            None => json!({}),
        };
        self.post("/api/relay/pair/initiate", body).await
    }
    /// GET /api/relay/pair/:code/status — 성공(`claimed`)하면 데몬이 노드 토큰을 저장한다
    pub async fn pair_status(&self, code: &str, secret: &str, relay_url: Option<&str>) -> Result<Value> {
        self.get(&with_query(
            &format!("/api/relay/pair/{}/status", seg(code)),
            &[
                ("secret", Some(secret.to_string())),
                ("relayUrl", relay_url.map(str::to_string)),
            ],
        ))
        .await
    }
    pub async fn host_nodes(&self, host_id: &str, relay_url: Option<&str>) -> Result<Value> {
        self.get(&with_query(
            &format!("/api/relay/host/{}/nodes", seg(host_id)),
            &[("relayUrl", relay_url.map(str::to_string))],
        ))
        .await
    }
    /// GET /api/relay/node/:guild/members?withStatus=1 — `{members: [{id, username, displayName}], partial, reason?}`
    /// (`partial`: 디스코드 전체 목록을 못 받아 권한을 등록한 멤버만)
    pub async fn node_members(&self, guild_id: &str, relay_url: Option<&str>) -> Result<Value> {
        self.get(&with_query(
            &format!("/api/relay/node/{}/members", seg(guild_id)),
            &[("withStatus", Some("1".to_string())), ("relayUrl", relay_url.map(str::to_string))],
        ))
        .await
    }
    /// GET /api/relay/node/:guildId/legacy-members — 릴레이에 남은 권한 부여값 `[{id, username, displayName, level}]`
    pub async fn legacy_members(&self, guild_id: &str, relay_url: Option<&str>) -> Result<Value> {
        self.get(&with_query(
            &format!("/api/relay/node/{}/legacy-members", seg(guild_id)),
            &[("relayUrl", relay_url.map(str::to_string))],
        ))
        .await
    }
    /// GET /api/relay/host/:id/status — 릴레이가 이 PC(호스트)를 아는지. 실패하면 오류
    pub async fn host_status(&self, host_id: &str, relay_url: Option<&str>) -> Result<Value> {
        self.get(&with_query(
            &format!("/api/relay/host/{}/status", seg(host_id)),
            &[("relayUrl", relay_url.map(str::to_string))],
        ))
        .await
    }
    /// GET /api/relay/node/:guild/chatbot/models — `{provider, models: [이름], selected, effectiveModel}`
    /// (409 NO_API_KEY, 422 INVALID_API_KEY·KEY_PERMISSION·QUOTA_EXCEEDED, 503 PROVIDER_UNAVAILABLE)
    pub async fn chatbot_models(&self, guild_id: &str, relay_url: Option<&str>) -> Result<Value> {
        self.get(&with_query(
            &format!("/api/relay/node/{}/chatbot/models", seg(guild_id)),
            &[("relayUrl", relay_url.map(str::to_string))],
        ))
        .await
    }
    /// GET /api/relay/node/:guild/channels — `{chatbot: [id], music: id|null, botAvailable, channels: [{id, name, category}]}`
    pub async fn node_channels(&self, guild_id: &str, relay_url: Option<&str>) -> Result<Value> {
        self.get(&with_query(
            &format!("/api/relay/node/{}/channels", seg(guild_id)),
            &[("relayUrl", relay_url.map(str::to_string))],
        ))
        .await
    }
    /// PUT /api/relay/node/:guild/channels — `{chatbot?, music?}` 준 키만 바뀐다. 응답은 GET과 같다
    pub async fn set_node_channels(&self, guild_id: &str, body: Value, relay_url: Option<&str>) -> Result<Value> {
        self.put(
            &with_query(
                &format!("/api/relay/node/{}/channels", seg(guild_id)),
                &[("relayUrl", relay_url.map(str::to_string))],
            ),
            body,
        )
        .await
    }
    /// GET /api/discord/:guild/settings — `{guildId, locale, sleeping, music}` (I2)
    pub async fn discord_settings(&self, guild_id: &str) -> Result<Value> {
        self.get(&format!("/api/discord/{}/settings", seg(guild_id))).await
    }
    /// PATCH /api/discord/:guild/settings — 바꿀 키만 → `{success, settings, sync}`
    pub async fn set_discord_settings(&self, guild_id: &str, patch: Value) -> Result<Value> {
        self.patch(&format!("/api/discord/{}/settings", seg(guild_id)), patch).await
    }
    /// GET /api/instance/:id/words — `{id, name, module, words, commands: [{name, label, risk, builtin, moduleAliases, aliases}]}`
    pub async fn instance_words(&self, id: &str) -> Result<Value> {
        self.get(&format!("/api/instance/{}/words", seg(id))).await
    }
    /// PUT /api/instance/:id/words — `{words, commands}`. 규칙 위반은 400 `validation_failed {details}`
    pub async fn save_instance_words(&self, id: &str, body: Value) -> Result<Value> {
        self.put(&format!("/api/instance/{}/words", seg(id)), body).await
    }
    /// PUT /api/guilds/:guild/allowed-instances — `{instances: [id] | null}` (null은 제한 없음)
    pub async fn save_allowed_instances(&self, guild_id: &str, instances: Option<&[String]>) -> Result<Value> {
        self.put(&format!("/api/guilds/{}/allowed-instances", seg(guild_id)), json!({ "instances": instances })).await
    }
    // ── 백업·예약 ──

    /// GET /api/instance/:id/backups — `{backups: [{name, kind, created, size}]}`
    pub async fn backups(&self, id: &str) -> Result<Value> {
        self.get(&format!("/api/instance/{}/backups", seg(id))).await
    }
    /// POST /api/instance/:id/backups — `{backup}`. `body`: `{}`(지금 상태) 또는 `{source: "snapshot", snapshot?}`(게임 자동 백업 보관)
    pub async fn create_backup(&self, id: &str, body: Value) -> Result<Value> {
        self.post_long(&format!("/api/instance/{}/backups", seg(id)), body)
            .await
    }
    pub async fn delete_backup(&self, id: &str, name: &str) -> Result<Value> {
        self.delete(&format!("/api/instance/{}/backups/{}", seg(id), seg(name)))
            .await
    }
    /// POST …/backups/:name/restore — 켜져 있으면 409 `server_running`
    pub async fn restore_backup(&self, id: &str, name: &str) -> Result<Value> {
        self.post_long(
            &format!("/api/instance/{}/backups/{}/restore", seg(id), seg(name)),
            json!({}),
        )
        .await
    }
    /// GET /api/instance/:id/schedule — `{schedule, backup_keep, revision, backup_supported, announce_supported}`
    pub async fn schedule(&self, id: &str) -> Result<Value> {
        self.get(&format!("/api/instance/{}/schedule", seg(id))).await
    }
    /// PUT /api/instance/:id/schedule — `{schedule: [...], backup_keep?, revision?}` (목록 전체를 바꾼다. 읽은 뒤 바뀌었으면 409 `schedule_changed`)
    pub async fn save_schedule(&self, id: &str, body: Value) -> Result<Value> {
        self.put(&format!("/api/instance/{}/schedule", seg(id)), body).await
    }

    // ── AI 챗봇 ──

    /// GET /api/chatbot — `{settings, guilds, hasApiKey}` (키 자체는 오지 않는다)
    pub async fn chatbot(&self) -> Result<Value> {
        self.get("/api/chatbot").await
    }
    /// PUT /api/chatbot — `{settings?, guilds?, apiKey?: string|null}` (apiKey를 빼면 그대로)
    pub async fn save_chatbot(&self, patch: Value) -> Result<Value> {
        self.put_slow("/api/chatbot", patch).await
    }
    /// DELETE /api/chatbot — 설정과 키를 모두 지운다
    pub async fn clear_chatbot(&self) -> Result<Value> {
        self.delete("/api/chatbot").await
    }
    /// GET /api/chatbot/guilds/:guild — 디스코드 서버 하나에 실제로 쓰이는 값 (공통 값 + 그 서버 값) `{guildId, settings, hasApiKey}`
    pub async fn chatbot_guild(&self, guild: &str) -> Result<Value> {
        self.get(&format!("/api/chatbot/guilds/{}", seg(guild))).await
    }
    /// DELETE /api/chatbot/memory?guildId= — 장기 기억 (없으면 모든 길드)
    pub async fn forget_chatbot(&self, guild: Option<&str>) -> Result<Value> {
        self.delete(&with_query(
            "/api/chatbot/memory",
            &[("guildId", guild.map(str::to_string))],
        ))
        .await
    }

    /// GET /api/relay/bot-invite — 봇 초대 URL `{url}`
    pub async fn bot_invite(&self, relay_url: Option<&str>) -> Result<Value> {
        self.get(&with_query(
            "/api/relay/bot-invite",
            &[("relayUrl", relay_url.map(str::to_string))],
        ))
        .await
    }
    /// GET /api/relay/link-requests — 디스코드에서 코드 없이 온 연결 요청 (J1-b)
    pub async fn link_requests(&self, relay_url: Option<&str>) -> Result<Value> {
        self.get(&with_query("/api/relay/link-requests", &[("relayUrl", relay_url.map(str::to_string))])).await
    }
    /// POST /api/relay/link-requests/:id — 수락·거절
    pub async fn answer_link_request(&self, id: &str, accept: bool, relay_url: Option<&str>) -> Result<Value> {
        let path = format!("/api/relay/link-requests/{}", seg(id));
        self.post(&with_query(&path, &[("relayUrl", relay_url.map(str::to_string))]), json!({ "accept": accept })).await
    }
    /// POST /api/chat/command — 디스코드와 같은 텍스트 명령 처리 (`{reply}`)
    pub async fn chat(&self, text: &str) -> Result<Value> {
        self.post("/api/chat/command", json!({ "text": text })).await
    }

    // ── 업데이트 ──

    pub async fn update_status(&self) -> Result<Value> {
        self.get("/api/updates/status").await
    }
    pub async fn update_check(&self) -> Result<Value> {
        self.post_long("/api/updates/check", json!({})).await
    }
    pub async fn update_download(&self, components: &[String]) -> Result<Value> {
        self.post_long("/api/updates/download", json!({ "components": components }))
            .await
    }
    pub async fn update_progress(&self) -> Result<Value> {
        self.get("/api/updates/download/progress").await
    }
    pub async fn update_apply(&self, components: &[String]) -> Result<Value> {
        self.post_long("/api/updates/apply", json!({ "components": components }))
            .await
    }
    /// POST /api/updates/launch-updater — 데몬이 업데이터를 띄우고 스스로 종료한다
    pub async fn launch_updater(&self, relaunch: &[String]) -> Result<Value> {
        self.post("/api/updates/launch-updater", json!({ "relaunch": relaunch }))
            .await
    }
    /// GET /api/updates/last-result?lang&include_seen=true — `{result, version, notes, acknowledged}` (notes: 그 언어의 패치노트)
    pub async fn update_last_result(&self, lang: &str) -> Result<Value> {
        self.get(&last_result_path(lang)).await
    }
    pub async fn clear_update_last_result(&self) -> Result<Value> {
        self.delete("/api/updates/last-result").await
    }
    pub async fn update_integrity(&self) -> Result<Value> {
        self.get_slow("/api/updates/integrity").await
    }
    pub async fn update_config(&self) -> Result<Value> {
        self.get("/api/updates/config").await
    }
    pub async fn save_update_config(&self, patch: Value) -> Result<Value> {
        self.put("/api/updates/config", patch).await
    }
}

fn servers_path(keep_notices: bool, seen: &std::sync::atomic::AtomicU64) -> String {
    if keep_notices {
        "/api/servers?notices=keep".into()
    } else {
        format!("/api/servers?notices_after={}", seen.load(std::sync::atomic::Ordering::Relaxed))
    }
}

/// 받은 알림 중 가장 큰 id를 본 것으로 적고, 겹친 조회로 이미 본 것(id가 커서 이하)은 거른다 (GUI serverList와 같은 규칙)
fn remember_notices(seen: &std::sync::atomic::AtomicU64, body: &mut Value) {
    let Some(list) = body.get_mut("port_conflict_stops").and_then(Value::as_array_mut) else {
        return;
    };
    let newest = list.iter().filter_map(|n| n.get("id").and_then(Value::as_u64)).max().unwrap_or(0);
    let before = seen.fetch_max(newest, std::sync::atomic::Ordering::Relaxed);
    // id가 없는 알림(옛 데몬)은 그대로 둔다
    list.retain(|n| n.get("id").and_then(Value::as_u64).is_none_or(|id| id > before));
}

/// `update last`는 다른 클라이언트(GUI)가 이미 확인한 결과도 다시 본다: 패치노트를 CLI로 다시 읽는 길 (L70)
fn last_result_path(lang: &str) -> String {
    with_query("/api/updates/last-result", &[("lang", Some(lang.to_string())), ("include_seen", Some("true".into()))])
}

fn with_query(path: &str, params: &[(&str, Option<String>)]) -> String {
    let q: Vec<String> = params
        .iter()
        .filter_map(|(k, v)| v.as_ref().map(|v| format!("{}={}", k, seg(v))))
        .collect();
    if q.is_empty() {
        path.to_string()
    } else {
        format!("{}?{}", path, q.join("&"))
    }
}

/// 일부만 된 작업의 결과(2xx에 `ok`/`success: false`)는 오류 대신 본문을 돌려준다.
/// 무엇이 되고 무엇이 실패했는지는 부른 명령이 보이고 실패로 끝낸다 (데몬이 오류 문구 없이 결과만 싣는다)
pub fn partial(r: Result<Value>) -> Result<Value> {
    match r {
        Err(e) => match e.downcast::<CliError>() {
            Ok(CliError::Api { status, body, .. }) if (200..300).contains(&status) => Ok(body),
            Ok(other) => Err(other.into()),
            Err(e) => Err(e),
        },
        ok => ok,
    }
}

/// 상태 코드와 본문으로 성공·실패를 정한다
pub fn check(status: u16, body: Value) -> Result<Value> {
    let flag_false = |k: &str| body.get(k).and_then(Value::as_bool) == Some(false);
    if (200..300).contains(&status) && !flag_false("ok") && !flag_false("success") {
        return Ok(body);
    }
    let pick = |k: &str| {
        body.get(k)
            .and_then(Value::as_str)
            .filter(|s| !s.is_empty())
            .map(str::to_string)
    };
    // 데몬이 그대로 넘긴 릴레이 본문은 `error`가 코드(`NODE_NOT_FOUND`)이고 error_code가 없다
    let relay_code = pick("error").filter(|e| e.len() > 2 && e.bytes().all(|b| b.is_ascii_uppercase() || b.is_ascii_digit() || b == b'_'));
    let relay_code = if status == 429 && pick("error_code").is_none() { Some("RATE_LIMITED".to_string()) } else { relay_code };
    if let (None, Some(rc)) = (pick("error_code"), relay_code) {
        let code = rc.to_ascii_lowercase();
        // 같은 뜻의 릴레이 코드는 한 문장으로 묶는다
        let key = match code.as_str() {
            "bot_offline" | "bot_not_ready" | "service_unavailable" => "bot_unavailable",
            "invalid_channel" => "unknown_channel",
            other => other,
        };
        let message = match key {
            // 릴레이가 요구하는 최소 버전을 그대로 보인다
            "agent_too_old" => crate::error::update_required(&body, ""),
            key => crate::i18n::try_t(&format!("err.relay.{}", key)),
        }
        .unwrap_or_else(|| {
            // 모르는 코드는 릴레이가 붙인 설명이 있으면 같이 보인다
            let other = t!("err.relay.other", code = rc);
            match pick("message") {
                Some(detail) => format!("{} {}", other, detail),
                None => other,
            }
        });
        return Err(CliError::Api { status, code: Some(code), message, body }.into());
    }
    // 모듈이 사용자 조치를 바라는 실패(`action_required`, 예: 실행 파일 없음)는 그 코드가 오류 코드다
    let code = pick("error_code").or_else(|| pick("action_required"));
    // 내부 오류일 때 error_message는 뭉뚱그린 문구라 구체적인 error가 낫다
    let generic = code.as_deref() == Some("internal_server_error") && pick("error").is_some();
    let message = pick("error_message")
        .filter(|_| !generic)
        .or_else(|| pick("error").filter(|e| Some(e) != code.as_ref()))
        .or_else(|| pick("message"))
        .or_else(|| pick("error"))
        .or_else(|| body.as_str().map(str::to_string))
        .unwrap_or_else(|| format!("HTTP {}", status));
    // 사바쨩이 낮아 거절됐으면(모듈·익스텐션이 더 새 앱을 요구) 일반 실패 문구 대신 업데이트가 필요하다고 보인다
    let message = crate::error::update_required(&body, "").unwrap_or(message);
    Err(CliError::Api {
        status,
        code,
        message,
        body,
    }
    .into())
}

#[cfg(test)]
mod tests {
    use super::*;

    /// GUI가 패치노트를 열어 확인한 뒤에도 `update last`가 결과와 노트를 보인다 (L70)
    #[test]
    fn the_last_result_includes_one_already_seen_elsewhere() {
        assert_eq!(last_result_path("ko"), "/api/updates/last-result?lang=ko&include_seen=true");
    }

    fn api_err(r: Result<Value>) -> CliError {
        r.unwrap_err().downcast::<CliError>().unwrap()
    }

    /// 포트 충돌 정지 알림은 본 것 뒤만 받는다: 다른 클라이언트(GUI)가 읽어도 사라지지 않고,
    /// 셸에서 `status`를 다시 불러도 이미 보인 알림을 또 보이지 않는다 (L50)
    #[test]
    fn port_conflict_notices_are_read_after_the_last_one_seen() {
        let seen = std::sync::atomic::AtomicU64::new(0);
        assert_eq!(servers_path(false, &seen), "/api/servers?notices_after=0");
        let mut first = json!({ "port_conflict_stops": [{ "id": 9 }, { "id": 5 }] });
        remember_notices(&seen, &mut first);
        assert_eq!(first["port_conflict_stops"].as_array().unwrap().len(), 2);
        assert_eq!(servers_path(false, &seen), "/api/servers?notices_after=9");
        // 겹친 조회가 같은 알림을 늦게 받아도 본 것은 다시 보이지 않는다
        let mut late = json!({ "port_conflict_stops": [{ "id": 9 }, { "id": 12 }] });
        remember_notices(&seen, &mut late);
        assert_eq!(late["port_conflict_stops"], json!([{ "id": 12 }]));
        remember_notices(&seen, &mut json!({ "port_conflict_stops": [] }));
        assert_eq!(servers_path(false, &seen), "/api/servers?notices_after=12");
        assert_eq!(servers_path(true, &seen), "/api/servers?notices=keep");
    }

    /// 정지·재시작 요청은 데몬이 가장 긴 정지 대기(인스턴스 stop_timeout 최대 3600초)를 다 기다릴 때까지 끊지 않는다 (L74)
    #[test]
    fn a_stop_request_outlasts_the_longest_stop_timeout() {
        assert!(STOP > Duration::from_secs(3600 + 5), "{STOP:?}");
    }

    /// 받은 요청 머리를 그대로 돌려주는 한 번짜리 데몬
    async fn request_head(lang: Option<&str>) -> String {
        use tokio::io::{AsyncReadExt, AsyncWriteExt};
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let port = listener.local_addr().unwrap().port();
        let server = tokio::spawn(async move {
            let (mut sock, _) = listener.accept().await.unwrap();
            // 머리가 여러 조각으로 올 수 있으니 빈 줄까지 읽는다
            let (mut head, mut buf) = (Vec::new(), [0; 1024]);
            while !head.windows(4).any(|w| w == b"\r\n\r\n") {
                let n = sock.read(&mut buf).await.unwrap();
                assert!(n > 0, "connection closed before the request head ended");
                head.extend_from_slice(&buf[..n]);
            }
            sock.write_all(b"HTTP/1.1 200 OK\r\ncontent-length: 2\r\n\r\n{}").await.unwrap();
            String::from_utf8_lossy(&head).to_ascii_lowercase()
        });
        let api = Api::new(port, lang.map(str::to_string));
        *api.token.write().unwrap() = None;
        api.call(Method::GET, "/api/servers", None, QUICK).await.unwrap();
        server.await.unwrap()
    }

    /// 토큰 파일이 남은 채 데몬이 꺼져 있으면 신원 확인의 연결 실패도 "데몬 꺼짐"이다 (자동 시작·안내가 그것을 본다)
    #[tokio::test]
    async fn a_stopped_daemon_with_a_leftover_token_is_reported_as_down() {
        let port = std::net::TcpListener::bind("127.0.0.1:0").unwrap().local_addr().unwrap().port();
        let api = Api::new(port, None);
        *api.token.write().unwrap() = Some("leftover".into());
        let e = api.call(Method::GET, "/api/servers", None, QUICK).await.unwrap_err();
        assert!(matches!(e.downcast_ref::<CliError>(), Some(CliError::DaemonDown { .. })), "{e:#}");
    }

    /// 한 번 받은 요청의 본문 (JSON)
    async fn request_body(send: impl FnOnce(Api) -> std::pin::Pin<Box<dyn std::future::Future<Output = Result<Value>> + Send>>) -> Value {
        use tokio::io::{AsyncReadExt, AsyncWriteExt};
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let port = listener.local_addr().unwrap().port();
        let server = tokio::spawn(async move {
            let (mut sock, _) = listener.accept().await.unwrap();
            let (mut req, mut buf) = (Vec::new(), [0; 4096]);
            loop {
                let n = sock.read(&mut buf).await.unwrap();
                assert!(n > 0, "connection closed before the request ended");
                req.extend_from_slice(&buf[..n]);
                let text = String::from_utf8_lossy(&req).to_string();
                if let Some((head, body)) = text.split_once("\r\n\r\n") {
                    let len = head
                        .lines()
                        .find_map(|l| l.to_ascii_lowercase().strip_prefix("content-length:").map(|v| v.trim().parse::<usize>().unwrap()))
                        .unwrap_or(0);
                    if body.len() >= len {
                        sock.write_all(b"HTTP/1.1 200 OK\r\ncontent-length: 2\r\n\r\n{}").await.unwrap();
                        return serde_json::from_str::<Value>(body).unwrap();
                    }
                }
            }
        });
        let api = Api::new(port, None);
        *api.token.write().unwrap() = None;
        send(api).await.unwrap();
        server.await.unwrap()
    }

    /// 액션 계획·여러 단계 계획은 요약 문구를 CLI가 고른 언어로 받게 본문에 `locale`을 싣는다 (데몬은 머리가 아니라 본문을 본다)
    #[tokio::test]
    async fn action_plans_ask_for_the_cli_language() {
        // 언어는 프로세스에 한 번 정해지는데, 다른 테스트가 도중에 정할 수 있다: 요청 앞뒤 값 중 하나면 된다
        let either = |body: &Value, before: &str| {
            let sent = body["locale"].as_str().unwrap_or_default().to_string();
            assert!(sent == before || sent == crate::i18n::lang(), "{sent}");
        };
        let before = crate::i18n::lang();
        let one = request_body(|api| Box::pin(async move { api.action("server.stop", "plan", json!({ "input": {} })).await })).await;
        either(&one, before);
        let before = crate::i18n::lang();
        let many = request_body(|api| Box::pin(async move { api.plan_steps(json!([])).await })).await;
        either(&many, before);
    }

    /// `--lang`을 주면 데몬 문구도 그 언어로 오게 요청마다 `Accept-Language`를 싣는다. 안 주면 데몬 설정을 따른다
    #[tokio::test]
    async fn the_chosen_language_goes_to_the_daemon() {
        assert!(request_head(Some("ko")).await.contains("\r\naccept-language: ko\r\n"));
        assert!(!request_head(None).await.contains("accept-language"));
    }

    /// 일부만 된 작업(2xx에 `ok`/`success: false`, 예: 업데이트 받기·적용, 챗봇 기억 지우기)은 본문을 명령에 돌려준다:
    /// 무엇이 되고 무엇이 실패했는지는 명령이 보이고 실패로 끝낸다. 2xx가 아닌 실패는 그대로 오류다
    #[test]
    fn a_partly_done_result_is_handed_back_to_the_command() {
        let body = json!({ "ok": false, "applied": ["gui"], "errors": [{ "component": "cli", "error": "boom" }] });
        assert_eq!(partial(check(200, body.clone())).unwrap(), body);
        assert!(partial(check(500, json!({ "error": "boom" }))).is_err());
        assert_eq!(partial(check(200, json!({ "ok": true }))).unwrap(), json!({ "ok": true }));
    }

    /// 모듈이 실행 파일을 못 찾은 시작 실패(`action_required`)는 그 코드로 다루고 다시 설치하라고 안내한다 (MK7, 옛 server_jar_not_found 포함)
    #[test]
    fn a_missing_executable_is_reported_by_its_action_code() {
        crate::i18n::init("en");
        for code in ["executable_not_found", "server_jar_not_found"] {
            let e = api_err(check(200, json!({ "success": false, "action_required": code, "message": "server.jar not found" })));
            assert_eq!(e.code(), code);
            assert_eq!(e.to_string(), "server.jar not found");
            assert!(e.hint().is_some_and(|h| h.contains("server update")), "{}", code);
        }
    }

    #[test]
    fn relay_error_codes_are_shown_as_sentences() {
        crate::i18n::init("ko");
        let e = api_err(check(404, json!({ "error": "NODE_NOT_FOUND" })));
        assert_eq!(e.code(), "node_not_found");
        assert!(!e.to_string().contains("NODE_NOT_FOUND") && e.hint().is_some(), "{} / {:?}", e, e.hint());
        let e = api_err(check(429, json!({ "statusCode": 429, "error": "Too Many Requests", "message": "Rate limit exceeded, retry in 1 minute" })));
        assert_eq!(e.code(), "rate_limited");
        assert!(!e.to_string().contains("Rate limit"), "{}", e);
        // 모르는 코드도 코드 원문만 덩그러니 나오지 않는다
        let e = api_err(check(400, json!({ "error": "SOMETHING_NEW" })));
        assert!(e.to_string() != "SOMETHING_NEW" && e.to_string().contains("SOMETHING_NEW"), "{}", e);
        let e = api_err(check(400, json!({ "error": "SOMETHING_NEW", "message": "the relay says why" })));
        assert!(e.to_string().contains("the relay says why"), "{}", e);
        // 봇이 꺼져 있다는 여러 코드는 같은 문장
        let offline = api_err(check(503, json!({ "error": "BOT_OFFLINE" }))).to_string();
        assert_eq!(offline, api_err(check(503, json!({ "error": "BOT_NOT_READY" }))).to_string());
        assert_eq!(offline, crate::i18n::try_t("err.relay.bot_unavailable").unwrap());
        // 확인한 뒤 그 디스코드 서버가 다른 PC로 옮겨 갔다 (릴레이 409, 감사 R6)
        let moved = api_err(check(409, json!({ "error": "NODE_MOVED" })));
        assert_eq!(moved.to_string(), crate::i18n::try_t("err.relay.node_moved").unwrap_or_default(), "{}", moved);
    }

    // 릴레이가 최소 버전보다 낮은 앱을 거절하면(426) 올려야 할 버전과 확인 명령을 알려 준다
    #[test]
    fn a_relay_that_wants_a_newer_app_names_the_version() {
        crate::i18n::init("ko");
        let e = api_err(check(426, json!({ "error": "AGENT_TOO_OLD", "minVersion": "0.1.9", "message": "Update Saba-chan to 0.1.9 or later." })));
        assert_eq!(e.code(), "agent_too_old");
        assert!(e.to_string().starts_with("업데이트가 필요해요"), "{}", e);
        assert!(e.to_string().contains("0.1.9") && !e.to_string().contains("AGENT_TOO_OLD"), "{}", e);
        assert!(e.hint().is_some_and(|h| h.contains("saba-chan-cli update check")), "{:?}", e.hint());
    }

    // 모듈·익스텐션이 더 새 사바쨩을 요구해 거절되면 일반 실패 대신 "업데이트가 필요해요"와 그 이름·버전을 보인다
    #[test]
    fn a_dependency_that_wants_a_newer_app_asks_for_an_update() {
        crate::i18n::init("ko");
        let module = api_err(check(200, json!({
            "success": false, "error": "module_dependency_unsatisfied", "error_code": "module_dependency_unsatisfied",
            "message": "Cannot start 'A': module 'palworld' needs saba-core >=0.1.10 (installed 0.1.9)",
            "module": "palworld", "missing_dependencies": ["saba-core >=0.1.10 (installed 0.1.9)"],
        })));
        assert_eq!(module.code(), "module_dependency_unsatisfied");
        let text = module.to_string();
        assert!(text.starts_with("업데이트가 필요해요") && text.contains("palworld") && text.contains("0.1.10") && !text.contains(">="), "{text}");
        assert!(module.hint().is_some_and(|h| h.contains("saba-chan-cli update check")), "{:?}", module.hint());

        let newer = api_err(check(200, json!({ "success": false, "error_code": "module_schema_unsupported", "module": "ark", "required": 3 }))).to_string();
        assert!(newer.starts_with("업데이트가 필요해요") && newer.contains("ark"), "{newer}");

        let ext = api_err(check(422, json!({
            "success": false, "error": "raw", "error_code": "app_version_unsatisfied", "related": ["0.1.10"], "extension": "steamcmd",
        })));
        let text = ext.to_string();
        assert!(text.starts_with("업데이트가 필요해요") && text.contains("steamcmd") && text.contains("0.1.10"), "{text}");
        assert!(ext.hint().is_some_and(|h| h.contains("saba-chan-cli update check")), "{:?}", ext.hint());

        // 익스텐션만 모자라면 업데이트 안내가 아니다
        let only_ext = api_err(check(200, json!({
            "success": false, "error_code": "module_dependency_unsatisfied", "message": "needs ext",
            "module": "palworld", "missing_dependencies": ["ext-steamcmd >=1.0.0 (not installed)"],
        })));
        assert_eq!(only_ext.to_string(), "needs ext");
    }

    #[test]
    fn internal_error_shows_the_specific_cause() {
        let e = api_err(check(
            500,
            json!({ "error": "RCON execution failed: refused", "error_code": "internal_server_error", "error_message": "Internal server error occurred" }),
        ));
        match e {
            CliError::Api { message, .. } => assert_eq!(message, "RCON execution failed: refused"),
            _ => panic!(),
        }
    }

    #[test]
    fn ok_false_with_200_is_an_error() {
        let e = api_err(check(200, json!({ "ok": false, "error": "Module not found" })));
        assert!(matches!(e, CliError::Api { status: 200, ref message, .. } if message == "Module not found"));
    }

    #[test]
    fn module_message_wins_over_raw_error() {
        let e = api_err(check(
            409,
            json!({ "error": "Server 'a' is not running", "error_code": "server_not_running", "error_message": "서버가 꺼져 있어요" }),
        ));
        match e {
            CliError::Api { code, message, .. } => {
                assert_eq!(code.as_deref(), Some("server_not_running"));
                assert_eq!(message, "서버가 꺼져 있어요");
            }
            _ => unreachable!(),
        }
    }

    #[test]
    fn validation_error_uses_message_not_code() {
        let e = api_err(check(
            400,
            json!({ "error": "validation_failed", "error_code": "validation_failed", "message": "Settings validation failed" }),
        ));
        assert_eq!(e.to_string(), "Settings validation failed");
    }

    #[test]
    fn success_passes_through() {
        assert_eq!(check(201, json!({ "success": true, "id": "x" })).unwrap()["id"], "x");
        assert!(check(200, Value::Null).is_ok());
    }

    #[test]
    fn encodes_path_segments_and_queries() {
        assert_eq!(seg("내 서버/1"), "%EB%82%B4%20%EC%84%9C%EB%B2%84%2F1");
        assert_eq!(with_query("/x", &[("a", Some("1".into())), ("b", None)]), "/x?a=1");
    }
}

/// 이벤트 한 건 (`GET /api/events`)
#[derive(Debug, Clone, PartialEq)]
pub struct DaemonEvent {
    pub id: Option<u64>,
    pub kind: String,
    pub data: Value,
}

/// 열린 이벤트 스트림
pub struct Events {
    resp: reqwest::Response,
    /// 아직 다 오지 않은 바이트. 덩어리 경계에서 한글이 잘려도 블록이 끝난 뒤에 디코딩한다
    buf: Vec<u8>,
}

impl Events {
    /// 다음 이벤트. 데몬이 연결을 닫으면 None
    pub async fn next(&mut self) -> Result<Option<DaemonEvent>> {
        loop {
            if let Some(event) = take_event(&mut self.buf) {
                return Ok(Some(event));
            }
            match self.resp.chunk().await? {
                Some(chunk) => self.buf.extend_from_slice(&chunk),
                None => return Ok(None),
            }
        }
    }
}

/// 버퍼에서 끝난 블록을 꺼내 이벤트로. 끝난 블록이 없으면 None (남은 바이트는 그대로)
fn take_event(buf: &mut Vec<u8>) -> Option<DaemonEvent> {
    while let Some(end) = buf.windows(2).position(|w| w == b"\n\n") {
        let block: Vec<u8> = buf.drain(..end + 2).collect();
        if let Some(event) = parse_sse(&String::from_utf8_lossy(&block)) {
            return Some(event);
        }
    }
    None
}

/// SSE 한 덩어리 → 이벤트. 주석(ping)뿐이거나 data가 JSON이 아니면 None
fn parse_sse(block: &str) -> Option<DaemonEvent> {
    let (mut id, mut kind, mut data) = (None, "message".to_string(), Vec::new());
    for line in block.lines().filter(|l| !l.is_empty() && !l.starts_with(':')) {
        let (field, value) = line.split_once(':').map_or((line, ""), |(f, v)| (f, v.strip_prefix(' ').unwrap_or(v)));
        match field {
            "id" => id = value.parse().ok(),
            "event" => kind = value.to_string(),
            "data" => data.push(value),
            _ => {}
        }
    }
    if data.is_empty() {
        return None;
    }
    let data = serde_json::from_str(&data.join("\n")).ok()?;
    Some(DaemonEvent { id, kind, data })
}

#[cfg(test)]
mod route_coverage {
    /// 데몬 라우트 중 CLI가 부르지 않는 것과 그 까닭. 여기 없는 라우트는 모두 CLI(그래서 TUI)에서 닿아야 한다
    const NOT_FOR_CLI: &[(&str, &str)] = &[
        ("/api/client/register", "GUI 창 수명 (CLI는 한 번 돌고 끝난다)"),
        ("/api/updates/install", "원격(디스코드 `/사바쨩 업데이트`)용 한 번에 받고 적용 (CLI `update`는 단계마다 진행을 보인다)"),
        ("/api/client/{id}/heartbeat", "GUI 창 수명"),
        ("/api/client/{id}/unregister", "GUI 창 수명"),
        ("/api/extensions/{id}/gui", "GUI 번들"),
        ("/api/extensions/{id}/gui/styles", "GUI 번들"),
        ("/api/extensions/{id}/icon", "GUI 그림"),
        ("/api/extensions/{id}/i18n/{locale}", "GUI 번들 문구"),
        ("/api/server/{name}/status", "이름 기반 옛 길 (CLI는 인스턴스 id 길)"),
        ("/api/server/{name}/start", "이름 기반 옛 길"),
        ("/api/server/{name}/stop", "이름 기반 옛 길"),
        ("/api/instance/{id}/managed/start", "start가 관리 모드를 데몬에 맡긴다"),
        ("/api/remote/daemons/{host_id}/proxy/{*rest}", "원격 데몬 중계: 데몬 API 전체를 그 데몬으로 넘기는 길 (원격 데몬 고르기는 G1)"),
    ];

    /// 라우터 소스에서 `.route("…"` 경로를 모은다 (여러 줄에 걸친 것도)
    fn routes(src: &str) -> Vec<String> {
        let mut out = Vec::new();
        let mut rest = src;
        while let Some(i) = rest.find(".route(") {
            rest = &rest[i + 7..];
            let start = rest.find('"').unwrap() + 1;
            let end = start + rest[start..].find('"').unwrap();
            out.push(rest[start..end].to_string());
        }
        out
    }

    /// CLI 소스의 `"/api…"`·`"/health"` 문자열 (쿼리는 뗀다)
    fn calls(src: &str) -> Vec<String> {
        src.split('"')
            .map(|s| s.strip_prefix("{}").unwrap_or(s))
            .filter(|s| (s.starts_with("/api/") || *s == "/health") && !s.contains(' '))
            .map(|s| s.split('?').next().unwrap().to_string())
            .collect()
    }

    /// `{x}`와 `{}`는 아무 한 조각
    fn same(route: &str, call: &str) -> bool {
        let (r, c): (Vec<&str>, Vec<&str>) = (route.split('/').collect(), call.split('/').collect());
        r.len() == c.len() && r.iter().zip(&c).all(|(a, b)| a == b || a.starts_with('{') || *b == "{}")
    }

    #[test]
    fn every_daemon_route_is_reachable_from_the_cli() {
        let mut all = routes(include_str!("../../src/ipc/mod.rs"));
        all.extend(routes(include_str!("../../src/ipc/updates.rs")));
        let used = calls(include_str!("client.rs"));
        assert!(all.len() > 100, "router parse: {}", all.len());
        let missing: Vec<&String> = all
            .iter()
            .filter(|r| !NOT_FOR_CLI.iter().any(|(n, _)| n == r) && !used.iter().any(|c| same(r, c)))
            .collect();
        assert!(missing.is_empty(), "routes the CLI cannot reach: {missing:#?}");
        // 빼 둔 목록도 실제 라우트여야 한다 (지운 라우트가 남지 않게)
        for (n, _) in NOT_FOR_CLI {
            assert!(all.iter().any(|r| r == n), "{n} is not a daemon route any more");
        }
    }
}

#[cfg(test)]
mod sse_tests {
    use super::*;

    #[test]
    fn blocks_become_events_and_pings_are_skipped() {
        let e = parse_sse("id: 7\nevent: action.run\ndata: {\"ok\":true}\n\n").unwrap();
        assert_eq!(e, DaemonEvent { id: Some(7), kind: "action.run".into(), data: serde_json::json!({ "ok": true }) });
        assert_eq!(parse_sse(":ping\n\n"), None);
        assert_eq!(parse_sse("event: resync\ndata: {}\n\n").map(|e| (e.id, e.kind)), Some((None, "resync".into())));
    }

    #[test]
    fn korean_split_across_chunks_stays_whole() {
        let raw = "data: {\"line\":\"서버\"}\n\n".as_bytes();
        // '서'의 바이트 중간에서 자른다
        let cut = raw.iter().position(|&b| b >= 0x80).unwrap() + 1;
        let mut buf = raw[..cut].to_vec();
        assert_eq!(take_event(&mut buf), None);
        buf.extend_from_slice(&raw[cut..]);
        assert_eq!(take_event(&mut buf).unwrap().data, serde_json::json!({ "line": "서버" }));
        assert!(buf.is_empty());
    }
}
