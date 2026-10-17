//! 멀티 콘솔 탭 모델
//!
//! GUI의 `ConsoleDock`/`ConsoleWindow`에 대응. `App`이 `TabSet`을 소유하며,
//! 각 [`Tab`]은 독립적인 링 버퍼, stdin, 스크롤 상태, 필터를 가진다.
//!
//! ## 폴링 구조
//!
//! 탭을 열 때 [`spawn_server_poller`]가 토큰으로 데몬을 주기적으로 폴링하면서
//! 새 라인을 `UnboundedSender<TabEvent>`로 메인 루프에 밀어 넣는다. 메인 루프는
//! 매 iteration마다 `try_recv` 루프로 수신해 [`TabSet::apply_event`]를 호출한다.
//!
//! 탭이 활성이 아니어도 폴러는 계속 돌며 라인을 누적한다 (`dirty` 플래그로 탭바에 표시).
//!
//! ## 수명
//!
//! [`Drop for Tab`]이 `JoinHandle::abort()`를 호출하므로 `close()` 시 폴러가 정리된다.
//! `App`이 drop될 때도 모든 탭이 drop되며 자동으로 폴러가 abort 된다.

use std::collections::VecDeque;
use std::time::Duration;

use serde_json::Value;
use tokio::sync::mpsc::UnboundedSender;
use tokio::task::JoinHandle;

use crate::client::DaemonClient;

pub type TabId = usize;

const BUFFER_CAP: usize = 2000;

/// Extract printable text + optional level from a single console entry.
///
/// Daemon endpoints disagree on the field name:
/// - `/api/instance/{id}/console`        → `content` (+ `level`)
/// - `/api/daemon/console`               → `message` (+ `level`)
/// - `/api/ext-process/.../console`      → `line`
///
/// Plain strings are returned as-is for backwards compatibility with any
/// endpoint that hasn't been migrated to the structured form.
pub(crate) fn extract_entry_text(entry: &Value) -> Option<String> {
    if let Some(s) = entry.as_str() {
        return Some(s.to_string());
    }
    entry
        .get("content")
        .or_else(|| entry.get("text"))
        .or_else(|| entry.get("line"))
        .or_else(|| entry.get("message"))
        .and_then(|v| v.as_str())
        .map(|s| s.to_string())
}

/// Map an entry's `level` field (`error`/`warn`/...) to a [`LogLine`] constructor.
pub(crate) fn make_log_line(entry: &Value, text: String) -> LogLine {
    match entry.get("level").and_then(|v| v.as_str()) {
        Some("error") => LogLine::error(text),
        Some("warn") | Some("warning") => LogLine::warn(text),
        _ => LogLine::info(text),
    }
}

// ═══════════════════════════════════════════════════════
// LogLine / LogLevel
// ═══════════════════════════════════════════════════════

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum LogLevel {
    Info,
    Warn,
    Error,
    /// 사용자 stdin 입력 에코
    Stdin,
}

#[derive(Clone, Debug)]
pub struct LogLine {
    pub text: String,
    pub level: LogLevel,
}

impl LogLine {
    pub fn info(text: impl Into<String>) -> Self {
        Self { text: text.into(), level: LogLevel::Info }
    }
    #[allow(dead_code)]
    pub fn warn(text: impl Into<String>) -> Self {
        Self { text: text.into(), level: LogLevel::Warn }
    }
    pub fn error(text: impl Into<String>) -> Self {
        Self { text: text.into(), level: LogLevel::Error }
    }
    pub fn stdin_echo(text: impl Into<String>) -> Self {
        Self { text: text.into(), level: LogLevel::Stdin }
    }
}

// ═══════════════════════════════════════════════════════
// TabSource / TabEvent
// ═══════════════════════════════════════════════════════

/// 탭이 어떤 로그 소스를 보여주는지 나타낸다.
///
/// Stage 1에서는 `Server`만 지원. Stage 2에서 `Daemon`·`Unified`·`ExtProcess`가 추가된다.
#[derive(Clone, Debug)]
#[allow(dead_code)]
pub enum TabSource {
    /// 서버 인스턴스 콘솔. `id`로 `get_console(id)` 폴링.
    /// `module`은 Stage 2에서 Unified 탭 필터링 등에 사용된다.
    Server { id: String, name: String, module: String },
    /// 데몬(Saba-Core) 트레이싱 로그. stdin 불가 (read-only).
    Daemon,
    /// 외부 익스텐션 프로세스 (e.g. music bot). `get_ext_process_console`.
    ExtProcess { name: String },
}

impl TabSource {
    #[allow(dead_code)]
    pub fn supports_stdin(&self) -> bool {
        match self {
            TabSource::Server { .. } | TabSource::ExtProcess { .. } => true,
            TabSource::Daemon => false,
        }
    }
}

/// 폴러 → 메인 루프 이벤트
#[derive(Debug)]
pub enum TabEvent {
    NewLines { tab_id: TabId, lines: Vec<LogLine> },
    /// 폴러가 비정상 종료된 경우 (수신 측에서 `poll_handle`을 None으로 리셋)
    #[allow(dead_code)]
    Closed { tab_id: TabId },
}

// ═══════════════════════════════════════════════════════
// Tab
// ═══════════════════════════════════════════════════════

pub struct Tab {
    pub id: TabId,
    pub title: String,
    pub source: TabSource,
    /// 링 버퍼. 선두(front)가 가장 오래된 라인.
    pub buffer: VecDeque<LogLine>,
    /// 위로 스크롤한 라인 수. 0이면 최신 라인(bottom)을 본다.
    pub scroll: usize,
    pub stdin: String,
    pub stdin_cursor: usize,
    /// 비활성 탭에 새 라인이 도착하면 true. 활성화되면 false로 리셋.
    pub dirty: bool,
    /// 부분 문자열 필터 (lowercase 포함). None이면 필터 없음.
    pub filter: Option<String>,
    poll_handle: Option<JoinHandle<()>>,
}

impl Tab {
    pub fn push_line(&mut self, line: LogLine) {
        if self.buffer.len() >= BUFFER_CAP {
            self.buffer.pop_front();
        }
        self.buffer.push_back(line);
    }

    /// 필터가 있으면 그에 매칭되는 라인만, 없으면 전체 반환.
    pub fn visible_lines(&self) -> Vec<&LogLine> {
        match &self.filter {
            Some(q) if !q.is_empty() => {
                let q_lower = q.to_lowercase();
                self.buffer.iter()
                    .filter(|l| l.text.to_lowercase().contains(&q_lower))
                    .collect()
            }
            _ => self.buffer.iter().collect(),
        }
    }
}

impl Drop for Tab {
    fn drop(&mut self) {
        if let Some(h) = self.poll_handle.take() {
            h.abort();
        }
    }
}

// ═══════════════════════════════════════════════════════
// TabSet
// ═══════════════════════════════════════════════════════

#[derive(Default)]
pub struct TabSet {
    pub tabs: Vec<Tab>,
    pub active: Option<TabId>,
    next_id: TabId,
}

impl TabSet {
    /// 새 탭을 연다. 동일한 소스(`TabSource`)의 탭이 이미 있으면 재사용.
    pub fn open(
        &mut self,
        source: TabSource,
        client: DaemonClient,
        tx: UnboundedSender<TabEvent>,
    ) -> TabId {
        if let Some(existing) = self.find_tab_for_source(&source) {
            self.active = Some(existing);
            if let Some(tab) = self.get_mut(existing) {
                tab.dirty = false;
            }
            return existing;
        }

        let id = self.next_id;
        self.next_id += 1;

        let (title, handle) = match &source {
            TabSource::Server { id: iid, name, .. } => {
                let title = format!("{} · {}", name, short_id(iid));
                let handle = spawn_server_poller(id, client, iid.clone(), tx);
                (title, handle)
            }
            TabSource::Daemon => {
                let title = "saba-core".to_string();
                let handle = spawn_daemon_poller(id, client, tx);
                (title, handle)
            }
            TabSource::ExtProcess { name } => {
                let title = format!("ext: {}", name);
                let handle = spawn_ext_process_poller(id, client, name.clone(), tx);
                (title, handle)
            }
        };

        let tab = Tab {
            id,
            title,
            source,
            buffer: VecDeque::with_capacity(256),
            scroll: 0,
            stdin: String::new(),
            stdin_cursor: 0,
            dirty: false,
            filter: None,
            poll_handle: Some(handle),
        };
        self.tabs.push(tab);
        self.active = Some(id);
        id
    }

    pub fn close(&mut self, id: TabId) {
        if let Some(pos) = self.tabs.iter().position(|t| t.id == id) {
            self.tabs.remove(pos);
        }
        if self.active == Some(id) {
            self.active = self.tabs.first().map(|t| t.id);
        }
    }

    pub fn close_active(&mut self) -> Option<TabId> {
        let id = self.active?;
        self.close(id);
        Some(id)
    }

    pub fn cycle_next(&mut self) {
        if self.tabs.is_empty() { return; }
        let cur = self.active.and_then(|id| self.tabs.iter().position(|t| t.id == id)).unwrap_or(0);
        let next = (cur + 1) % self.tabs.len();
        self.set_active(self.tabs[next].id);
    }

    pub fn cycle_prev(&mut self) {
        if self.tabs.is_empty() { return; }
        let cur = self.active.and_then(|id| self.tabs.iter().position(|t| t.id == id)).unwrap_or(0);
        let prev = if cur == 0 { self.tabs.len() - 1 } else { cur - 1 };
        self.set_active(self.tabs[prev].id);
    }

    pub fn set_active(&mut self, id: TabId) {
        if self.tabs.iter().any(|t| t.id == id) {
            self.active = Some(id);
            if let Some(tab) = self.get_mut(id) {
                tab.dirty = false;
            }
        }
    }

    pub fn active_tab(&self) -> Option<&Tab> {
        self.active.and_then(|id| self.tabs.iter().find(|t| t.id == id))
    }

    pub fn active_tab_mut(&mut self) -> Option<&mut Tab> {
        self.active.and_then(|id| self.tabs.iter_mut().find(|t| t.id == id))
    }

    #[allow(dead_code)]
    pub fn get(&self, id: TabId) -> Option<&Tab> {
        self.tabs.iter().find(|t| t.id == id)
    }

    pub fn get_mut(&mut self, id: TabId) -> Option<&mut Tab> {
        self.tabs.iter_mut().find(|t| t.id == id)
    }

    /// 서버 id로 탭을 찾는다 (재사용/매칭용).
    #[allow(dead_code)]
    pub fn find_server_tab(&self, server_id: &str) -> Option<TabId> {
        self.tabs.iter().find_map(|t| match &t.source {
            TabSource::Server { id, .. } if id == server_id => Some(t.id),
            _ => None,
        })
    }

    fn find_tab_for_source(&self, src: &TabSource) -> Option<TabId> {
        self.tabs.iter().find_map(|t| match (&t.source, src) {
            (TabSource::Server { id: a, .. }, TabSource::Server { id: b, .. }) if a == b => Some(t.id),
            (TabSource::Daemon, TabSource::Daemon) => Some(t.id),
            (TabSource::ExtProcess { name: a }, TabSource::ExtProcess { name: b }) if a == b => Some(t.id),
            _ => None,
        })
    }

    pub fn apply_event(&mut self, ev: TabEvent) {
        match ev {
            TabEvent::NewLines { tab_id, lines } => {
                let is_active = self.active == Some(tab_id);
                if let Some(tab) = self.get_mut(tab_id) {
                    for l in lines { tab.push_line(l); }
                    if !is_active { tab.dirty = true; }
                }
            }
            TabEvent::Closed { tab_id } => {
                if let Some(tab) = self.get_mut(tab_id) {
                    tab.poll_handle = None;
                }
            }
        }
    }
}

// ═══════════════════════════════════════════════════════
// 세션 복원 (Stage 6)
// ═══════════════════════════════════════════════════════

/// `App::restore_recent_tabs` 본체 — `TabSet`·`DaemonClient`만으로 호출되어
/// 순수 유닛 테스트가 가능하도록 분리되어 있다.
pub fn restore_tabs_into(
    tabs: &mut TabSet,
    client: &DaemonClient,
    tx: &UnboundedSender<TabEvent>,
    recent: &[super::session::RecentTab],
    valid_server_ids: Option<&std::collections::HashSet<String>>,
) -> usize {
    let mut count = 0usize;
    for rt in recent {
        let src = match rt.clone() {
            super::session::RecentTab::Server { id, name, module } => {
                if let Some(set) = valid_server_ids {
                    if !set.contains(&id) { continue; }
                }
                TabSource::Server { id, name, module }
            }
            super::session::RecentTab::Daemon => TabSource::Daemon,
            super::session::RecentTab::ExtProcess { name } => {
                TabSource::ExtProcess { name }
            }
        };
        tabs.open(src, client.clone(), tx.clone());
        count += 1;
    }
    count
}

// ═══════════════════════════════════════════════════════
// 폴링 태스크
// ═══════════════════════════════════════════════════════
fn short_id(id: &str) -> String {
    let taken: String = id.chars().take(8).collect();
    taken
}

/// 데몬 트레이싱 로그 폴러. 800ms 주기로 `since` 커서 증분 폴.
fn spawn_daemon_poller(
    tab_id: TabId,
    client: DaemonClient,
    tx: UnboundedSender<TabEvent>,
) -> JoinHandle<()> {
    tokio::spawn(async move {
        let mut since: u64 = 0;
        tokio::time::sleep(Duration::from_millis(200)).await;
        loop {
            if let Ok(data) = client.get_daemon_console(Some(since), Some(100)).await {
                if let Some(arr) = data.get("lines").and_then(|v| v.as_array()) {
                    let mut fresh: Vec<LogLine> = Vec::with_capacity(arr.len());
                    for entry in arr {
                        let id = entry.get("id").and_then(|v| v.as_u64()).unwrap_or(0);
                        let level = entry.get("level").and_then(|v| v.as_str()).unwrap_or("info");
                        let target = entry.get("target").and_then(|v| v.as_str()).unwrap_or("");
                        let msg = entry.get("message").and_then(|v| v.as_str()).unwrap_or("");
                        if msg.is_empty() { continue; }
                        if id >= since { since = id + 1; }
                        let text = format!("[{}] {} {}", level.to_uppercase(), target, msg);
                        let line = match level {
                            "error" => LogLine::error(text),
                            "warn" => LogLine::warn(text),
                            _ => LogLine::info(text),
                        };
                        fresh.push(line);
                    }
                    if !fresh.is_empty()
                        && tx.send(TabEvent::NewLines { tab_id, lines: fresh }).is_err()
                    {
                        return;
                    }
                }
            }
            tokio::time::sleep(Duration::from_millis(800)).await;
        }
    })
}

/// 외부 프로세스(ext-process) 콘솔 폴러. 서버 폴러와 동일한 전략으로 증분 fetch.
fn spawn_ext_process_poller(
    tab_id: TabId,
    client: DaemonClient,
    process_name: String,
    tx: UnboundedSender<TabEvent>,
) -> JoinHandle<()> {
    tokio::spawn(async move {
        let mut last_count: usize = 0;
        tokio::time::sleep(Duration::from_millis(200)).await;
        loop {
            if let Ok(data) = client.get_ext_process_console(&process_name).await {
                let entries: Vec<Value> = if let Some(arr) = data.get("lines").and_then(|v| v.as_array()) {
                    arr.clone()
                } else if let Some(s) = data.get("output").and_then(|v| v.as_str()) {
                    s.lines().map(|l| Value::String(l.to_string())).collect()
                } else {
                    vec![]
                };
                if entries.len() < last_count { last_count = 0; }
                if entries.len() > last_count {
                    let fresh: Vec<LogLine> = entries[last_count..]
                        .iter()
                        .filter_map(|e| extract_entry_text(e).map(|t| make_log_line(e, t)))
                        .collect();
                    if !fresh.is_empty()
                        && tx.send(TabEvent::NewLines { tab_id, lines: fresh }).is_err()
                    {
                        return;
                    }
                    last_count = entries.len();
                }
            }
            tokio::time::sleep(Duration::from_millis(1500)).await;
        }
    })
}

/// 서버 인스턴스 콘솔 폴러. 1.5초 주기로 `get_console` 호출, 증분 라인만 전송.
fn spawn_server_poller(
    tab_id: TabId,
    client: DaemonClient,
    instance_id: String,
    tx: UnboundedSender<TabEvent>,
) -> JoinHandle<()> {
    tokio::spawn(async move {
        let mut last_count: usize = 0;
        // 첫 요청은 짧은 지연 후 (UI가 탭을 먼저 그리도록)
        tokio::time::sleep(Duration::from_millis(200)).await;
        loop {
            match client.get_console(&instance_id).await {
                Ok(data) => {
                    let entries: Vec<Value> = if let Some(arr) = data.get("lines").and_then(|v| v.as_array()) {
                        arr.clone()
                    } else if let Some(s) = data.get("output").and_then(|v| v.as_str()) {
                        s.lines().map(|l| Value::String(l.to_string())).collect()
                    } else {
                        vec![]
                    };
                    // 서버 재시작 등으로 라인이 줄면 처음부터 다시
                    if entries.len() < last_count {
                        last_count = 0;
                    }
                    if entries.len() > last_count {
                        let fresh: Vec<LogLine> = entries[last_count..]
                            .iter()
                            .filter_map(|e| extract_entry_text(e).map(|t| make_log_line(e, t)))
                            .collect();
                        if !fresh.is_empty()
                            && tx.send(TabEvent::NewLines { tab_id, lines: fresh }).is_err()
                        {
                            return; // receiver dropped
                        }
                        last_count = entries.len();
                    }
                }
                Err(_) => {
                    // 일시 오류는 무시 (다음 주기에 재시도)
                }
            }
            tokio::time::sleep(Duration::from_millis(1500)).await;
        }
    })
}

// ═══════════════════════════════════════════════════════
// 테스트
// ═══════════════════════════════════════════════════════

#[cfg(test)]
mod tests {
    use super::*;

    fn dummy_source(id: &str) -> TabSource {
        TabSource::Server {
            id: id.to_string(),
            name: format!("srv-{}", id),
            module: "test".into(),
        }
    }

    #[test]
    fn push_line_respects_buffer_cap() {
        let mut tab = Tab {
            id: 0,
            title: "t".into(),
            source: dummy_source("s1"),
            buffer: VecDeque::new(),
            scroll: 0,
            stdin: String::new(),
            stdin_cursor: 0,
            dirty: false,
            filter: None,
            poll_handle: None,
        };
        for i in 0..(BUFFER_CAP + 50) {
            tab.push_line(LogLine::info(format!("line {}", i)));
        }
        assert_eq!(tab.buffer.len(), BUFFER_CAP);
        // 가장 오래된 50줄은 drop되어야 함
        assert_eq!(tab.buffer.front().unwrap().text, "line 50");
    }

    #[test]
    fn visible_lines_respects_filter() {
        let mut tab = Tab {
            id: 0,
            title: "t".into(),
            source: dummy_source("s1"),
            buffer: VecDeque::new(),
            scroll: 0,
            stdin: String::new(),
            stdin_cursor: 0,
            dirty: false,
            filter: None,
            poll_handle: None,
        };
        tab.push_line(LogLine::info("hello world"));
        tab.push_line(LogLine::error("ERROR: boom"));
        tab.push_line(LogLine::info("another"));

        // 필터 없음
        assert_eq!(tab.visible_lines().len(), 3);

        // 필터: "error" (대소문자 무시)
        tab.filter = Some("Error".into());
        let v = tab.visible_lines();
        assert_eq!(v.len(), 1);
        assert_eq!(v[0].text, "ERROR: boom");

        // 빈 필터
        tab.filter = Some(String::new());
        assert_eq!(tab.visible_lines().len(), 3);
    }

    #[tokio::test(flavor = "current_thread")]
    async fn tabset_reuses_same_server_tab() {
        let (tx, _rx) = tokio::sync::mpsc::unbounded_channel();
        // `DaemonClient::new`는 기본 IPC 포트로 초기화만 하고 네트워크 요청은
        // 폴러가 호출될 때만 발생한다. 폴러가 실패해도 TabSet 상태에는 영향이 없다.
        let client = DaemonClient::new(None).unwrap();
        let mut set = TabSet::default();
        let id1 = set.open(dummy_source("s1"), client.clone(), tx.clone());
        let id2 = set.open(dummy_source("s1"), client.clone(), tx.clone());
        assert_eq!(id1, id2);
        assert_eq!(set.tabs.len(), 1);

        let id3 = set.open(dummy_source("s2"), client, tx);
        assert_ne!(id3, id1);
        assert_eq!(set.tabs.len(), 2);
        assert_eq!(set.active, Some(id3));
    }

    #[tokio::test(flavor = "current_thread")]
    async fn tabset_cycle_and_close() {
        let (tx, _rx) = tokio::sync::mpsc::unbounded_channel();
        let client = DaemonClient::new(None).unwrap();
        let mut set = TabSet::default();
        let a = set.open(dummy_source("a"), client.clone(), tx.clone());
        let b = set.open(dummy_source("b"), client.clone(), tx.clone());
        let c = set.open(dummy_source("c"), client, tx);
        assert_eq!(set.active, Some(c));

        set.cycle_next(); // c → a
        assert_eq!(set.active, Some(a));
        set.cycle_prev(); // a → c
        assert_eq!(set.active, Some(c));
        set.cycle_prev(); // c → b
        assert_eq!(set.active, Some(b));

        set.close_active(); // close b → active must switch
        assert_eq!(set.tabs.len(), 2);
        assert!(set.active.is_some());
        assert_ne!(set.active, Some(b));
    }

    // ── Stage 6: 세션 복원 ──

    #[tokio::test(flavor = "current_thread")]
    async fn restore_tabs_into_reopens_all_kinds() {
        use super::super::session::RecentTab;
        let (tx, _rx) = tokio::sync::mpsc::unbounded_channel();
        let client = DaemonClient::new(None).unwrap();
        let mut set = TabSet::default();
        let recent = vec![
            RecentTab::Daemon,
            RecentTab::Server { id: "s1".into(), name: "srv".into(), module: "palworld".into() },
            RecentTab::ExtProcess { name: "music".into() },
        ];
        let n = restore_tabs_into(&mut set, &client, &tx, &recent, None);
        assert_eq!(n, 3);
        assert_eq!(set.tabs.len(), 3);
        assert!(matches!(set.tabs[0].source, TabSource::Daemon));
        assert!(matches!(set.tabs[1].source, TabSource::Server { .. }));
        assert!(matches!(set.tabs[2].source, TabSource::ExtProcess { .. }));
    }

    #[tokio::test(flavor = "current_thread")]
    async fn restore_tabs_into_filters_missing_servers() {
        use super::super::session::RecentTab;
        let (tx, _rx) = tokio::sync::mpsc::unbounded_channel();
        let client = DaemonClient::new(None).unwrap();
        let mut set = TabSet::default();
        let recent = vec![
            RecentTab::Server { id: "alive".into(), name: "a".into(), module: "m".into() },
            RecentTab::Server { id: "gone".into(), name: "g".into(), module: "m".into() },
            RecentTab::Daemon,
        ];
        let valid: std::collections::HashSet<String> =
            std::iter::once("alive".to_string()).collect();
        let n = restore_tabs_into(&mut set, &client, &tx, &recent, Some(&valid));
        // "gone" 은 건너뛰고 "alive" + Daemon 만 복원
        assert_eq!(n, 2);
        assert_eq!(set.tabs.len(), 2);
        assert!(matches!(&set.tabs[0].source, TabSource::Server { id, .. } if id == "alive"));
        assert!(matches!(set.tabs[1].source, TabSource::Daemon));
    }

    #[tokio::test(flavor = "current_thread")]
    async fn restore_tabs_into_none_validates_means_accept_all() {
        use super::super::session::RecentTab;
        let (tx, _rx) = tokio::sync::mpsc::unbounded_channel();
        let client = DaemonClient::new(None).unwrap();
        let mut set = TabSet::default();
        let recent = vec![
            RecentTab::Server { id: "x".into(), name: "x".into(), module: "m".into() },
        ];
        let n = restore_tabs_into(&mut set, &client, &tx, &recent, None);
        // 데몬 오프라인(valid=None) 시에도 서버 탭은 복원됨
        assert_eq!(n, 1);
        assert_eq!(set.tabs.len(), 1);
    }

    /// Regression: server-console pollers used to assume `lines` was a
    /// `Vec<String>`, but the daemon returns objects with `content` (server),
    /// `message` (daemon), or `line` (ext-process). The old code silently
    /// emitted empty strings — leaving every console tab blank.
    #[test]
    fn extract_entry_text_handles_each_endpoint_shape() {
        use serde_json::json;
        // server console
        assert_eq!(
            extract_entry_text(&json!({"id": 1, "content": "hello", "level": "info"})),
            Some("hello".into())
        );
        // daemon console
        assert_eq!(
            extract_entry_text(&json!({"id": 2, "message": "warn msg", "level": "warn"})),
            Some("warn msg".into())
        );
        // ext-process console
        assert_eq!(
            extract_entry_text(&json!({"id": 3, "line": "[stdout] ok"})),
            Some("[stdout] ok".into())
        );
        // legacy string entry
        assert_eq!(
            extract_entry_text(&json!("plain")),
            Some("plain".into())
        );
        // unknown shape returns None (skipped, not blank)
        assert_eq!(extract_entry_text(&json!({"foo": "bar"})), None);
    }

    #[test]
    fn make_log_line_maps_level_field() {
        use serde_json::json;
        let err = make_log_line(&json!({"level": "error"}), "boom".into());
        assert_eq!(err.level, LogLevel::Error);
        let warn = make_log_line(&json!({"level": "warn"}), "uh".into());
        assert_eq!(warn.level, LogLevel::Warn);
        let info = make_log_line(&json!({}), "ok".into());
        assert_eq!(info.level, LogLevel::Info);
    }
}
