//! 데몬 이벤트 스트림 (P4): 데몬에서 일어난 일을 GUI·CLI가 폴링 없이 받는다 (`GET /api/events`, SSE)
//!
//! 최근 `KEPT`건만 메모리에 두고 파일에는 남기지 않는다. 번호는 데몬이 켜진 뒤 1부터 늘어난다.
//! 다시 붙는 클라이언트는 마지막 번호를 주고 그 뒤 것을 먼저 받는다. 놓친 건이 있으면
//! (버퍼보다 오래된 번호, 데몬 재시작, 느려서 뒤처짐) `resync`를 받고 필요한 목록을 다시 읽는다.

use serde_json::Value;
use std::collections::VecDeque;
use std::sync::{Arc, Mutex};
use tokio::sync::broadcast;

/// 메모리에 두는 최근 이벤트 수
const KEPT: usize = 200;
/// 구독자 한 명이 이만큼 뒤처지면 `resync`
const CHANNEL: usize = 256;

#[derive(Debug, Clone, PartialEq)]
pub struct Event {
    pub id: u64,
    pub kind: String,
    /// `{at, ...}`
    pub data: Value,
}

/// 구독을 시작할 때 받는 것: 먼저 보낼 것들과 이어서 받을 채널
pub struct Subscription {
    /// 놓친 건이 있어 클라이언트가 목록을 다시 읽어야 한다
    pub resync: bool,
    pub backlog: Vec<Arc<Event>>,
    pub rx: broadcast::Receiver<Arc<Event>>,
}

struct Inner {
    next: u64,
    recent: VecDeque<Arc<Event>>,
}

pub struct Bus {
    inner: Mutex<Inner>,
    tx: broadcast::Sender<Arc<Event>>,
}

impl Default for Bus {
    fn default() -> Self {
        Self::new()
    }
}

impl Bus {
    pub fn new() -> Self {
        Self { inner: Mutex::new(Inner { next: 1, recent: VecDeque::new() }), tx: broadcast::channel(CHANNEL).0 }
    }

    /// 한 건 낸다. `data`가 객체이면 `at`(unix 초)을 붙인다
    pub fn emit(&self, kind: &str, mut data: Value) {
        if let Some(obj) = data.as_object_mut() {
            obj.insert("at".into(), Value::from(saba_runner::now()));
        }
        let mut inner = self.inner.lock().unwrap_or_else(|e| e.into_inner());
        let event = Arc::new(Event { id: inner.next, kind: kind.to_string(), data });
        inner.next += 1;
        inner.recent.push_back(event.clone());
        while inner.recent.len() > KEPT {
            inner.recent.pop_front();
        }
        // 버퍼에 넣는 것과 보내는 것을 같은 잠금 안에서 해야 구독 시작 때 빠지거나 겹치는 건이 없다
        let _ = self.tx.send(event);
    }

    /// `since`(마지막으로 받은 번호) 뒤부터 구독한다. 없으면 지금부터
    pub fn subscribe(&self, since: Option<u64>) -> Subscription {
        let inner = self.inner.lock().unwrap_or_else(|e| e.into_inner());
        let rx = self.tx.subscribe();
        let Some(since) = since else {
            return Subscription { resync: false, backlog: Vec::new(), rx };
        };
        let last = inner.next - 1;
        if since > last {
            // 데몬이 다시 켜져 번호가 처음부터다
            return Subscription { resync: true, backlog: inner.recent.iter().cloned().collect(), rx };
        }
        let oldest = inner.recent.front().map_or(inner.next, |e| e.id);
        Subscription {
            resync: since + 1 < oldest,
            backlog: inner.recent.iter().filter(|e| e.id > since).cloned().collect(),
            rx,
        }
    }
}

static GLOBAL: std::sync::OnceLock<Bus> = std::sync::OnceLock::new();

/// 데몬의 이벤트 버스
pub fn bus() -> &'static Bus {
    GLOBAL.get_or_init(Bus::new)
}

/// 데몬 버스로 한 건 낸다
pub fn emit(kind: &str, data: Value) {
    bus().emit(kind, data);
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn ids(events: &[Arc<Event>]) -> Vec<u64> {
        events.iter().map(|e| e.id).collect()
    }

    #[test]
    fn a_client_that_comes_back_gets_what_it_missed_in_order() {
        let bus = Bus::new();
        for n in 0..5 {
            bus.emit("action.run", json!({ "n": n }));
        }
        let sub = bus.subscribe(Some(2));
        assert!(!sub.resync);
        assert_eq!(ids(&sub.backlog), vec![3, 4, 5]);
        assert!(sub.backlog[0].data["at"].is_u64(), "every event carries its time");

        let fresh = bus.subscribe(None);
        assert!(!fresh.resync && fresh.backlog.is_empty(), "a new client starts from now");
        assert!(bus.subscribe(Some(5)).backlog.is_empty());
    }

    #[test]
    fn a_gap_tells_the_client_to_reload() {
        let bus = Bus::new();
        for _ in 0..KEPT + 10 {
            bus.emit("action.run", json!({}));
        }
        let old = bus.subscribe(Some(3));
        assert!(old.resync, "events 4..=10 were dropped from the buffer");
        assert_eq!(old.backlog.len(), KEPT);
        assert!(!bus.subscribe(Some(10)).resync, "11 is the oldest kept, so nothing was missed");

        let restarted = Bus::new();
        restarted.emit("action.run", json!({}));
        let sub = restarted.subscribe(Some(50));
        assert!(sub.resync, "a number from before a daemon restart");
        assert_eq!(ids(&sub.backlog), vec![1]);
    }

    #[tokio::test]
    async fn events_after_subscribing_arrive_on_the_channel() {
        let bus = Bus::new();
        bus.emit("action.run", json!({ "n": 1 }));
        let mut sub = bus.subscribe(Some(0));
        bus.emit("instance.exit", json!({ "n": 2 }));
        assert_eq!(ids(&sub.backlog), vec![1]);
        let next = sub.rx.recv().await.unwrap();
        assert_eq!((next.id, next.kind.as_str()), (2, "instance.exit"));
    }
}
