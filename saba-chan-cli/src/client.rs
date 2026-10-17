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

#[derive(Clone)]
pub struct Api {
    http: reqwest::Client,
    base: String,
    /// 데몬 포트 (데몬을 띄울 때 그대로 넘긴다)
    pub port: u16,
    token: Arc<RwLock<Option<String>>>,
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
    pub fn new(port: u16) -> Self {
        let http = reqwest::Client::builder()
            .connect_timeout(Duration::from_secs(2))
            .no_proxy()
            .build()
            .expect("http client");
        Self {
            http,
            base: format!("http://127.0.0.1:{}", port),
            port,
            token: Arc::new(RwLock::new(read_token())),
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
        req.send().await.map_err(|e| {
            if e.is_connect() {
                CliError::DaemonDown { url: self.base.clone() }.into()
            } else if e.is_timeout() {
                CliError::Failed(t!("err.timeout", path = path)).into()
            } else {
                anyhow::anyhow!(e)
            }
        })
    }

    pub async fn call(&self, method: Method, path: &str, body: Option<Value>, timeout: Duration) -> Result<Value> {
        let mut resp = self.send(&method, path, body.as_ref(), timeout).await?;
        // 데몬이 다시 뜨면 토큰이 바뀐다 → 한 번 다시 읽고 재시도
        if resp.status() == reqwest::StatusCode::UNAUTHORIZED {
            let fresh = read_token();
            if let Ok(mut t) = self.token.write() {
                *t = fresh;
            }
            resp = self.send(&method, path, body.as_ref(), timeout).await?;
        }
        let status = resp.status().as_u16();
        let text = resp.text().await.unwrap_or_default();
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

    /// GET /health — 락 없이 답하는 ping (`{ok, version, pid}`)
    pub async fn health(&self) -> Result<Value> {
        self.call(Method::GET, "/health", None, Duration::from_secs(2)).await
    }
    /// GET /api/version — 코어·모듈·익스텐션 버전
    pub async fn version(&self) -> Result<Value> {
        self.get("/api/version").await
    }
    /// GET /api/system/components — 릴리스 버전과 설치된 구성요소 `{version, components, lastUpdated}`
    pub async fn system_components(&self) -> Result<Value> {
        self.get("/api/system/components").await
    }
    /// GET /api/system/metrics — 시스템 모니터 표본
    pub async fn system_metrics(&self) -> Result<Value> {
        self.get("/api/system/metrics").await
    }
    /// POST /api/daemon/shutdown
    pub async fn shutdown(&self) -> Result<Value> {
        self.post("/api/daemon/shutdown", json!({})).await
    }
    /// GET /api/daemon/console?since=&count=
    pub async fn daemon_console(&self, since: Option<u64>, count: Option<usize>) -> Result<Value> {
        self.get(&with_query(
            "/api/daemon/console",
            &[
                ("since", since.map(|v| v.to_string())),
                ("count", count.map(|v| v.to_string())),
            ],
        ))
        .await
    }

    // ── 서버 (런타임 상태) ──

    /// GET /api/servers — 상태가 붙은 서버 목록 (익스텐션 hook 포함이라 느릴 수 있다)
    pub async fn servers(&self) -> Result<Value> {
        self.get_slow("/api/servers").await
    }
    /// POST /api/instance/:id/start — managed 여부는 데몬이 고른다
    pub async fn start(&self, id: &str) -> Result<Value> {
        self.post_long(&format!("/api/instance/{}/start", seg(id)), json!({}))
            .await
    }
    /// POST /api/instance/:id/stop
    pub async fn stop(&self, id: &str, force: bool) -> Result<Value> {
        self.post_long(&format!("/api/instance/{}/stop", seg(id)), json!({ "force": force }))
            .await
    }
    /// POST /api/instance/:id/restart — 멈추고(최대 2분) 다시 켠다. 꺼져 있었으면 켜기만. 응답은 시작 응답 + `stopped`
    pub async fn restart(&self, id: &str, force: bool) -> Result<Value> {
        self.post_long(&format!("/api/instance/{}/restart", seg(id)), json!({ "force": force }))
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
    /// DELETE /api/instance/:id
    pub async fn delete_instance(&self, id: &str) -> Result<Value> {
        self.delete(&format!("/api/instance/{}", seg(id))).await
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
    /// GET /api/instance/:id/console?since=&count=&spans=1 (색을 쓸 때만 하이라이팅 구간을 받는다)
    pub async fn console(&self, id: &str, since: Option<u64>, count: Option<usize>) -> Result<Value> {
        self.get(&with_query(
            &format!("/api/instance/{}/console", seg(id)),
            &[
                ("since", since.map(|v| v.to_string())),
                ("count", count.map(|v| v.to_string())),
                ("spans", console::colors_enabled().then(|| "1".to_string())),
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
    pub async fn accept_eula(&self, id: &str) -> Result<Value> {
        self.post(&format!("/api/instance/{}/accept-eula", seg(id)), json!({}))
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
    /// POST /api/module/:name/install — 서버 파일만 받는다 `{version, install_dir, jar_name?, accept_eula?}`
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
    /// GET /api/audit — `{entries: [{ts, source, user?, guild?, action, instanceId, instance?, command?, args?, ok, status}]}` 최신이 먼저
    pub async fn audit(&self, instance: Option<&str>, limit: usize) -> Result<Value> {
        self.get(&with_query(
            "/api/audit",
            &[("limit", Some(limit.to_string())), ("instance", instance.map(str::to_string))],
        ))
        .await
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
    /// POST/DELETE /api/sandbox/unisolated/:component (격리 없이 돌리기 동의)
    pub async fn set_unisolated(&self, component: &str, allow: bool) -> Result<Value> {
        let path = format!("/api/sandbox/unisolated/{}", seg(component));
        if allow {
            self.post(&path, json!({})).await
        } else {
            self.delete(&path).await
        }
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
    /// POST /api/actions/:id/plan·run `{input, instance?}` 또는 `{planId}`
    pub async fn action(&self, id: &str, step: &str, body: Value) -> Result<Value> {
        self.post_long(&format!("/api/actions/{}/{}", seg(id), step), body).await
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
    pub async fn node_members(&self, guild_id: &str, relay_url: Option<&str>) -> Result<Value> {
        self.get(&with_query(
            &format!("/api/relay/node/{}/members", seg(guild_id)),
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
    // ── 백업·예약 ──

    /// GET /api/instance/:id/backups — `{backups: [{name, kind, created, size}]}`
    pub async fn backups(&self, id: &str) -> Result<Value> {
        self.get(&format!("/api/instance/{}/backups", seg(id))).await
    }
    /// POST /api/instance/:id/backups — `{backup}`
    pub async fn create_backup(&self, id: &str) -> Result<Value> {
        self.post_long(&format!("/api/instance/{}/backups", seg(id)), json!({}))
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
    /// GET /api/instance/:id/schedule — `{schedule, backup_keep, backup_supported, announce_supported}`
    pub async fn schedule(&self, id: &str) -> Result<Value> {
        self.get(&format!("/api/instance/{}/schedule", seg(id))).await
    }
    /// PUT /api/instance/:id/schedule — `{schedule: [...], backup_keep?}` (목록 전체를 바꾼다)
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
    pub async fn update_last_result(&self) -> Result<Value> {
        self.get("/api/updates/last-result").await
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
    let code = pick("error_code");
    // 내부 오류일 때 error_message는 뭉뚱그린 문구라 구체적인 error가 낫다
    let generic = code.as_deref() == Some("internal_server_error") && pick("error").is_some();
    let message = pick("error_message")
        .filter(|_| !generic)
        .or_else(|| pick("error").filter(|e| Some(e) != code.as_ref()))
        .or_else(|| pick("message"))
        .or_else(|| pick("error"))
        .or_else(|| body.as_str().map(str::to_string))
        .unwrap_or_else(|| format!("HTTP {}", status));
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

    fn api_err(r: Result<Value>) -> CliError {
        r.unwrap_err().downcast::<CliError>().unwrap()
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
