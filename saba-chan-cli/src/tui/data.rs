//! 화면에 보일 데몬 상태. 모두 데몬 API로 읽고, 여기서는 아무것도 바꾸지 않는다
//!
//! 늘 보이는 것(머리줄·대시보드)은 매번, 지금 화면에만 필요한 것(모듈·익스텐션·로그·콘솔)은 그 화면일 때만 읽는다.

use crate::ctx::Ctx;
use serde_json::Value;
use std::collections::VecDeque;

/// 사용량 그래프에 남기는 표본 수
pub const HISTORY: usize = 60;
/// 최근 일 목록에 남기는 이벤트 수
pub const FEED: usize = 50;
/// 콘솔·로그 끝에서 읽어 오는 줄 수
pub const TAIL: usize = 200;

/// 한 번 읽은 결과. `None`은 이번에 읽지 않았다는 뜻 (화면에 남은 값을 그대로 쓴다)
#[derive(Default)]
pub struct Fetched {
    pub health: Option<Value>,
    pub servers: Vec<Value>,
    pub metrics: Option<Value>,
    pub agent: Option<Value>,
    pub updates: Option<Value>,
    pub modules: Option<Vec<Value>>,
    pub extensions: Option<Vec<Value>>,
    pub daemon_log: Option<Vec<String>>,
    /// (서버 id, 콘솔 끝 줄들)
    pub console: Option<(String, Vec<String>)>,
}

/// 무엇을 더 읽을지 (지금 화면)
#[derive(Default, Clone)]
pub struct Want {
    pub modules: bool,
    pub extensions: bool,
    pub daemon_log: bool,
    pub console_of: Option<String>,
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
    pub daemon_log: Vec<String>,
    pub console: Option<(String, Vec<String>)>,
    /// CPU·메모리 사용률(%) 기록
    pub cpu: VecDeque<u64>,
    pub mem: VecDeque<u64>,
    /// 최근 일 (시각, 글)
    pub feed: VecDeque<(String, String)>,
}

impl Snapshot {
    pub fn apply(&mut self, f: Fetched) {
        self.health = f.health;
        self.servers = f.servers;
        if self.health.is_none() {
            self.metrics = None;
            self.agent = None;
        }
        if let Some(m) = f.metrics {
            push(&mut self.cpu, percent(m.pointer("/cpu/total_percent").and_then(Value::as_f64)));
            let mem = match (m.pointer("/memory/used_bytes").and_then(Value::as_f64), m.pointer("/memory/total_bytes").and_then(Value::as_f64)) {
                (Some(u), Some(t)) if t > 0.0 => Some(u * 100.0 / t),
                _ => None,
            };
            push(&mut self.mem, percent(mem));
            self.metrics = Some(m);
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
        if let Some(l) = f.daemon_log {
            self.daemon_log = l;
        }
        if f.console.is_some() {
            self.console = f.console;
        }
    }

    pub fn running(&self) -> usize {
        self.servers.iter().filter(|v| s(v, "status") == "running").count()
    }

    /// 켜진 서버들의 접속자 합 (모르는 서버는 빼고)
    pub fn players_online(&self) -> Option<u64> {
        let counts: Vec<u64> = self
            .servers
            .iter()
            .filter_map(|v| v.get("players")?.get("online")?.as_u64())
            .collect();
        (!counts.is_empty()).then(|| counts.iter().sum())
    }

    /// 데몬 이벤트 하나를 최근 일에 더한다
    pub fn note(&mut self, kind: &str, data: &Value) {
        let at = data.get("at").and_then(Value::as_u64).map(clock).unwrap_or_else(|| clock(now()));
        let who = data
            .get("instance")
            .or_else(|| data.get("name"))
            .and_then(Value::as_str)
            .map(|n| self.name_of(n))
            .unwrap_or_default();
        let label = crate::i18n::try_t(&format!("tui.event.{}", kind.replace('.', "_"))).unwrap_or_else(|| kind.to_string());
        let mut text = if who.is_empty() { label } else { format!("{who} · {label}") };
        if let Some(a) = data.get("action").and_then(Value::as_str) {
            text = format!("{text} ({a})");
        }
        self.feed.push_front((at, text));
        self.feed.truncate(FEED);
    }

    /// id로 오는 서버를 이름으로
    fn name_of(&self, id_or_name: &str) -> String {
        self.servers
            .iter()
            .find(|v| s(v, "id") == id_or_name)
            .map(|v| s(v, "name").to_string())
            .unwrap_or_else(|| id_or_name.to_string())
    }
}

fn push(q: &mut VecDeque<u64>, v: Option<u64>) {
    if let Some(v) = v {
        q.push_back(v);
        while q.len() > HISTORY {
            q.pop_front();
        }
    }
}

fn percent(v: Option<f64>) -> Option<u64> {
    v.map(|p| p.clamp(0.0, 100.0).round() as u64)
}

pub fn s<'a>(v: &'a Value, k: &str) -> &'a str {
    v.get(k).and_then(Value::as_str).unwrap_or("")
}

fn now() -> u64 {
    std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_secs()).unwrap_or(0)
}

/// 유닉스 초 → 이 PC 시계의 HH:MM
pub fn clock(secs: u64) -> String {
    // 밀리초로 온 값도 받는다
    let secs = if secs > 10_000_000_000 { secs / 1000 } else { secs };
    chrono::DateTime::from_timestamp(secs as i64, 0)
        .map(|t| t.with_timezone(&chrono::Local).format("%H:%M").to_string())
        .unwrap_or_default()
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
    let daemon_log = match want.daemon_log {
        true => api.daemon_console(None, Some(TAIL)).await.ok().map(|r| {
            r.get("lines")
                .and_then(Value::as_array)
                .into_iter()
                .flatten()
                .map(|l| {
                    let level = s(l, "level").to_uppercase();
                    let at = l.get("timestamp").and_then(Value::as_u64).map(clock).unwrap_or_default();
                    format!("{at} {level:<5} {}", s(l, "message"))
                })
                .collect()
        }),
        false => None,
    };
    let console = match &want.console_of {
        Some(id) => api.console(id, None, Some(TAIL)).await.ok().map(|r| {
            let lines = r.get("lines").and_then(Value::as_array).into_iter().flatten().map(|l| s(l, "content").to_string()).collect();
            (id.clone(), lines)
        }),
        None => None,
    };
    Fetched {
        health: Some(health),
        servers: servers.ok().and_then(|v| v.get("servers").and_then(Value::as_array).cloned()).unwrap_or_default(),
        metrics: metrics.ok(),
        agent: agent.ok(),
        updates: updates.ok(),
        modules,
        extensions,
        daemon_log,
        console,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    /// 사용량은 % 기록으로 쌓이고 오래된 것부터 버린다. 데몬이 꺼지면 사용량을 지운다
    #[test]
    fn usage_history_is_kept_and_cleared_when_the_daemon_goes_away() {
        let mut snap = Snapshot::default();
        for _ in 0..HISTORY + 5 {
            snap.apply(Fetched {
                health: Some(json!({})),
                metrics: Some(json!({ "cpu": { "total_percent": 12.4 }, "memory": { "used_bytes": 4, "total_bytes": 16 } })),
                ..Fetched::default()
            });
        }
        assert_eq!(snap.cpu.len(), HISTORY);
        assert_eq!(snap.cpu.back(), Some(&12));
        assert_eq!(snap.mem.back(), Some(&25));
        snap.apply(Fetched::default());
        assert!(snap.metrics.is_none() && snap.health.is_none());
    }

    /// 이벤트는 서버 이름으로 보이고 최근 것이 위에 온다
    #[test]
    fn events_read_with_server_names_newest_first() {
        crate::i18n::init("ko");
        let mut snap = Snapshot { servers: vec![json!({ "id": "abc", "name": "팰월드" })], ..Snapshot::default() };
        snap.note("instance.ready", &json!({ "instance": "abc", "at": 1_700_000_000 }));
        snap.note("backup.done", &json!({ "instanceId": "abc" }));
        assert!(snap.feed[1].1.starts_with("팰월드 · "), "{:?}", snap.feed);
        assert_eq!(snap.feed.len(), 2);
    }
}
