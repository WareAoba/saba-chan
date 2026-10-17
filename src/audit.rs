//! 감사 로그 (A10): 누가(GUI·CLI·디스코드·예약) 어떤 서버에 무엇을 했는지
//!
//! `<data_dir>/audit.log`에 한 줄 JSON으로 덧붙인다. 1 MiB를 넘으면 `audit.log.1`로 넘기고 새로 쓴다
//! (둘만 남는다). 출처는 요청마다 task-local `Origin`에 담는다:
//! - HTTP 요청: `layer`가 `X-Saba-Client` 헤더(GUI는 `gui`, CLI는 `cli`, 없으면 `api`)로 정한다
//! - 디스코드(릴레이 에이전트): `discord` + 요청한 사용자·길드 id
//! - 예약 작업: `schedule`
//!
//! 로컬 클라이언트는 같은 토큰을 쓰므로 출처 헤더는 보안 경계가 아니라 기록용이다.

use serde::Serialize;
use serde_json::{json, Value};
use std::future::Future;
use std::io::Write;
use std::path::{Path, PathBuf};
use std::sync::Mutex;

/// 이 크기를 넘으면 `audit.log.1`로 넘긴다
const MAX_BYTES: u64 = 1024 * 1024;
/// 인자·콘솔 줄은 이만큼만 남긴다
const MAX_TEXT: usize = 300;

#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct Origin {
    pub source: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub user: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub guild: Option<String>,
}

impl Origin {
    pub fn new(source: &str) -> Self {
        Self { source: source.to_string(), user: None, guild: None }
    }

    /// `X-Saba-Client` 헤더 → 출처. 소문자·숫자·`-`만, 16자까지 (로그를 더럽히지 않게)
    pub fn from_headers(headers: &axum::http::HeaderMap) -> Self {
        let client = headers
            .get("X-Saba-Client")
            .and_then(|v| v.to_str().ok())
            .map(|v| v.trim().to_ascii_lowercase())
            .filter(|v| !v.is_empty() && v.len() <= 16 && v.chars().all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '-'));
        Self::new(client.as_deref().unwrap_or("api"))
    }
}

tokio::task_local! {
    static ORIGIN: Origin;
    static QUIET: ();
}

/// 안에서 일어나는 일은 기록하지 않는다 (재시작이 부르는 정지·시작처럼 바깥에서 한 건으로 남길 때)
pub async fn quietly<F: Future>(f: F) -> F::Output {
    QUIET.scope((), f).await
}

pub fn is_quiet() -> bool {
    QUIET.try_with(|_| ()).is_ok()
}

/// `origin`을 출처로 `f`를 실행한다
pub async fn with_origin<F: Future>(origin: Origin, f: F) -> F::Output {
    ORIGIN.scope(origin, f).await
}

/// 지금 요청의 출처 (정해진 게 없으면 데몬 자신)
pub fn current() -> Origin {
    ORIGIN.try_with(Clone::clone).unwrap_or_else(|_| Origin::new("daemon"))
}

/// HTTP 요청마다 출처를 정하는 미들웨어
pub async fn layer(req: axum::extract::Request, next: axum::middleware::Next) -> axum::response::Response {
    let origin = Origin::from_headers(req.headers());
    with_origin(origin, next.run(req)).await
}

/// 기록 한 건
#[derive(Debug, Clone, Default)]
pub struct Event<'a> {
    pub action: &'a str,
    pub instance_id: &'a str,
    pub instance: Option<&'a str>,
    pub command: Option<&'a str>,
    pub args: Option<&'a Value>,
    pub ok: bool,
    pub status: u16,
}

fn clip(s: &str) -> String {
    if s.chars().count() <= MAX_TEXT {
        s.to_string()
    } else {
        format!("{}…", s.chars().take(MAX_TEXT).collect::<String>())
    }
}

pub struct AuditLog {
    path: PathBuf,
    lock: Mutex<()>,
}

impl AuditLog {
    pub fn at(path: PathBuf) -> Self {
        Self { path, lock: Mutex::new(()) }
    }

    pub fn path(&self) -> &Path {
        &self.path
    }

    fn rotated(&self) -> PathBuf {
        let mut name = self.path.as_os_str().to_owned();
        name.push(".1");
        PathBuf::from(name)
    }

    pub fn record(&self, origin: &Origin, event: &Event<'_>) {
        let ts = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_secs())
            .unwrap_or(0);
        let mut line = json!({
            "ts": ts,
            "source": origin.source,
            "action": event.action,
            "instanceId": event.instance_id,
            "ok": event.ok,
            "status": event.status,
        });
        let obj = line.as_object_mut().expect("object");
        for (key, value) in [("user", origin.user.as_deref()), ("guild", origin.guild.as_deref()), ("instance", event.instance), ("command", event.command)] {
            if let Some(v) = value {
                obj.insert(key.into(), Value::String(clip(v)));
            }
        }
        if let Some(args) = event.args.filter(|a| !a.is_null()) {
            obj.insert("args".into(), Value::String(clip(&args.to_string())));
        }
        let _guard = self.lock.lock().unwrap_or_else(|e| e.into_inner());
        if let Err(e) = self.append(&line) {
            tracing::warn!("[Audit] Could not write {}: {}", self.path.display(), e);
        }
    }

    fn append(&self, line: &Value) -> std::io::Result<()> {
        if let Some(parent) = self.path.parent() {
            std::fs::create_dir_all(parent)?;
        }
        if std::fs::metadata(&self.path).is_ok_and(|m| m.len() >= MAX_BYTES) {
            std::fs::rename(&self.path, self.rotated())?;
        }
        let mut f = std::fs::OpenOptions::new().create(true).append(true).open(&self.path)?;
        writeln!(f, "{}", line)
    }

    /// 최근 기록 (최신이 먼저). `instance_id`가 있으면 그 서버 것만
    pub fn recent(&self, limit: usize, instance_id: Option<&str>) -> Vec<Value> {
        let _guard = self.lock.lock().unwrap_or_else(|e| e.into_inner());
        let mut out = Vec::new();
        for path in [self.path.clone(), self.rotated()] {
            let Ok(text) = std::fs::read_to_string(&path) else { continue };
            for line in text.lines().rev() {
                let Ok(v) = serde_json::from_str::<Value>(line) else { continue };
                if instance_id.is_some_and(|id| v.get("instanceId").and_then(Value::as_str) != Some(id)) {
                    continue;
                }
                out.push(v);
                if out.len() >= limit {
                    return out;
                }
            }
        }
        out
    }
}

static GLOBAL: std::sync::OnceLock<AuditLog> = std::sync::OnceLock::new();

/// 데몬의 감사 로그 (`<data_dir>/audit.log`)
pub fn log() -> &'static AuditLog {
    GLOBAL.get_or_init(|| AuditLog::at(saba_chan_updater_lib::constants::resolve_data_dir().join("audit.log")))
}

/// 지금 출처로 한 건 남긴다
pub fn record(event: Event<'_>) {
    if !is_quiet() {
        log().record(&current(), &event);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn records_come_back_newest_first_and_survive_rotation() {
        let dir = tempfile::tempdir().unwrap();
        let log = AuditLog::at(dir.path().join("audit.log"));
        let cli = Origin::new("cli");
        let discord = Origin { source: "discord".into(), user: Some("42".into()), guild: Some("7".into()) };
        log.record(&cli, &Event { action: "start", instance_id: "a", instance: Some("마크"), ok: true, status: 200, ..Default::default() });
        let args = json!(["x".repeat(400)]);
        log.record(&discord, &Event { action: "command", instance_id: "b", command: Some("say"), args: Some(&args), ok: false, status: 400, ..Default::default() });

        let all = log.recent(10, None);
        assert_eq!(all.len(), 2);
        assert_eq!(all[0]["source"], "discord");
        assert_eq!(all[0]["user"], "42");
        assert_eq!(all[0]["guild"], "7");
        assert!(all[0]["args"].as_str().unwrap().chars().count() <= MAX_TEXT + 1);
        assert_eq!(all[1]["instance"], "마크");
        assert!(all[1].get("user").is_none());
        assert_eq!(log.recent(10, Some("a")).len(), 1);

        // 넘긴 파일도 읽는다
        std::fs::rename(log.path(), log.rotated()).unwrap();
        log.record(&cli, &Event { action: "stop", instance_id: "a", ok: true, status: 200, ..Default::default() });
        let all = log.recent(10, None);
        assert_eq!(all.iter().map(|v| v["action"].as_str().unwrap()).collect::<Vec<_>>(), ["stop", "command", "start"]);
    }

    #[tokio::test]
    async fn the_origin_follows_the_request_and_the_header_is_sanitized() {
        assert_eq!(current().source, "daemon");
        let seen = with_origin(Origin::new("schedule"), async { current().source }).await;
        assert_eq!(seen, "schedule");
        assert!(!is_quiet() && quietly(async { is_quiet() }).await);
        let mut h = axum::http::HeaderMap::new();
        assert_eq!(Origin::from_headers(&h).source, "api");
        h.insert("X-Saba-Client", "GUI".parse().unwrap());
        assert_eq!(Origin::from_headers(&h).source, "gui");
        h.insert("X-Saba-Client", "evil\"json".parse().unwrap());
        assert_eq!(Origin::from_headers(&h).source, "api");
    }
}
