//! 화면에 보일 데몬 상태. 모두 데몬 API로 읽고, 여기서는 아무것도 바꾸지 않는다
//!
//! 상태(머리줄·목록)는 몇 초마다, 보고 있는 콘솔(서버 콘솔·데몬 로그)은 더 자주 끝부분만 읽는다.
//! 콘솔 줄은 데몬이 붙여 준 구문 강조 토큰(`spans`)과 함께 그대로 둔다 (칠하기는 `highlight`).

use crate::ctx::Ctx;
use serde_json::Value;

/// 콘솔·로그 끝에서 읽어 오는 줄 수
pub const TAIL: usize = 300;

/// 한 번 읽은 상태. `None`은 이번에 읽지 않았다는 뜻 (화면에 남은 값을 그대로 쓴다)
#[derive(Default)]
pub struct Fetched {
    pub health: Option<Value>,
    pub servers: Vec<Value>,
    pub metrics: Option<Value>,
    pub agent: Option<Value>,
    pub updates: Option<Value>,
    pub modules: Option<Vec<Value>>,
    pub extensions: Option<Vec<Value>>,
    pub launchable: Option<Value>,
}

/// 보고 있는 콘솔 한 번 읽은 것
pub enum View {
    /// (서버 id, 콘솔 끝 줄들)
    Console(String, Vec<Value>),
    DaemonLog(Vec<Value>),
}

/// 무엇을 더 읽을지 (지금 화면)
#[derive(Default, Clone)]
pub struct Want {
    pub modules: bool,
    pub extensions: bool,
    /// 띄울 수 있는 인터페이스를 아직 모른다
    pub launchable: bool,
}

/// 어느 콘솔을 볼지
#[derive(Clone, PartialEq)]
pub enum ViewWant {
    Console(String),
    DaemonLog,
}

/// 화면이 쓰는 데몬 상태
#[derive(Default)]
pub struct Snapshot {
    /// `GET /health`. None이면 데몬 꺼짐
    pub health: Option<Value>,
    pub servers: Vec<Value>,
    pub metrics: Option<Value>,
    /// `GET /api/relay/agent`
    pub agent: Option<Value>,
    /// `GET /api/updates/status`
    pub updates: Option<Value>,
    pub modules: Vec<Value>,
    pub extensions: Vec<Value>,
    /// `GET /api/clients/launch` (GUI로 전환 버튼을 보일지)
    pub launchable: Option<Value>,
    /// 데몬 로그 끝 줄들 (`{timestamp, level, target, message, spans}`)
    pub daemon_log: Vec<Value>,
    /// (서버 id, 콘솔 끝 줄들 `{content, source, level, spans}`)
    pub console: Option<(String, Vec<Value>)>,
}

impl Snapshot {
    pub fn apply(&mut self, f: Fetched) {
        self.health = f.health;
        self.servers = f.servers;
        if self.health.is_none() {
            self.metrics = None;
            self.agent = None;
            self.launchable = None;
        }
        if f.metrics.is_some() {
            self.metrics = f.metrics;
        }
        if f.agent.is_some() {
            self.agent = f.agent;
        }
        if f.updates.is_some() {
            self.updates = f.updates;
        }
        if let Some(m) = f.modules {
            self.modules = m;
        }
        if let Some(e) = f.extensions {
            self.extensions = e;
        }
        if f.launchable.is_some() {
            self.launchable = f.launchable;
        }
    }

    pub fn apply_view(&mut self, v: View) {
        match v {
            View::Console(id, lines) => self.console = Some((id, lines)),
            View::DaemonLog(lines) => self.daemon_log = lines,
        }
    }

    pub fn running(&self) -> usize {
        self.servers.iter().filter(|v| s(v, "status") == "running").count()
    }
}

pub fn s<'a>(v: &'a Value, k: &str) -> &'a str {
    v.get(k).and_then(Value::as_str).unwrap_or("")
}

/// 유닉스 초(밀리초도) → 이 PC 시계로 `fmt`
pub fn clock_fmt(secs: u64, fmt: &str) -> String {
    let secs = if secs > 10_000_000_000 { secs / 1000 } else { secs };
    chrono::DateTime::from_timestamp(secs as i64, 0)
        .map(|t| t.with_timezone(&chrono::Local).format(fmt).to_string())
        .unwrap_or_default()
}

/// 유닉스 초 → 이 PC 시계의 HH:MM
pub fn clock(secs: u64) -> String {
    clock_fmt(secs, "%H:%M")
}

/// 한 번 읽기: 데몬이 꺼져 있으면 나머지는 묻지 않는다
pub async fn fetch(ctx: &Ctx, want: &Want) -> Fetched {
    let Ok(health) = ctx.api.health().await else { return Fetched::default() };
    let api = &ctx.api;
    let (servers, metrics, agent, updates) = tokio::join!(api.servers(true), api.system_metrics(), api.agent(), api.update_status());
    let list = |v: Option<Value>, key: &str| v.and_then(|v| v.get(key).and_then(Value::as_array).cloned());
    let modules = match want.modules {
        true => list(api.modules().await.ok(), "modules"),
        false => None,
    };
    let extensions = match want.extensions {
        true => list(api.extensions().await.ok(), "extensions"),
        false => None,
    };
    let launchable = match want.launchable {
        true => api.launchable().await.ok(),
        false => None,
    };
    Fetched {
        health: Some(health),
        servers: servers.ok().and_then(|v| v.get("servers").and_then(Value::as_array).cloned()).unwrap_or_default(),
        metrics: metrics.ok(),
        agent: agent.ok(),
        updates: updates.ok(),
        modules,
        extensions,
        launchable,
    }
}

/// 보고 있는 콘솔의 끝부분 (토큰과 함께). 못 읽으면 None
pub async fn fetch_view(ctx: &Ctx, want: &ViewWant) -> Option<View> {
    let lines = |r: Value| r.get("lines").and_then(Value::as_array).cloned().unwrap_or_default();
    match want {
        ViewWant::Console(id) => match ctx.api.console_tokens(id, None, Some(TAIL), true).await {
            Ok(r) => Some(View::Console(id.clone(), lines(r))),
            // 한 번도 켠 적 없는 서버는 404: 빈 콘솔
            Err(e) if matches!(e.downcast_ref::<crate::error::CliError>(), Some(crate::error::CliError::Api { status: 404, .. })) => {
                Some(View::Console(id.clone(), Vec::new()))
            }
            Err(_) => None,
        },
        ViewWant::DaemonLog => ctx.api.daemon_console_tokens(None, Some(TAIL), true).await.ok().map(|r| View::DaemonLog(lines(r))),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    /// 데몬이 꺼지면 데몬에 딸린 값(사용량·디스코드·전환 가능 여부)을 지운다. 이번에 읽지 않은 목록은 남긴다
    #[test]
    fn values_tied_to_the_daemon_are_cleared_when_it_goes_away() {
        let mut snap = Snapshot::default();
        snap.apply(Fetched {
            health: Some(json!({})),
            launchable: Some(json!({ "gui": true })),
            modules: Some(vec![json!({ "name": "palworld" })]),
            ..Fetched::default()
        });
        snap.apply(Fetched { health: Some(json!({})), ..Fetched::default() });
        assert!(snap.launchable.is_some() && snap.modules.len() == 1);
        snap.apply(Fetched::default());
        assert!(snap.health.is_none() && snap.launchable.is_none() && snap.metrics.is_none());
    }
}
