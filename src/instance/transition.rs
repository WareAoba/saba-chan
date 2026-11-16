//! 시작·정지가 진행 중인 인스턴스. 서버 목록이 `transition`으로 싣고, 바뀌면 `servers.changed`를 낸다.
//! 화면마다 "시작하는 중"을 따로 들고 있지 않게 데몬이 하나로 쥔다 (누른 창 밖의 트레이·다른 창·CLI도 같은 값을 본다)

use std::collections::HashMap;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{LazyLock, Mutex};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Kind {
    Starting,
    Stopping,
}

impl Kind {
    pub fn as_str(self) -> &'static str {
        match self {
            Kind::Starting => "starting",
            Kind::Stopping => "stopping",
        }
    }
}

/// 인스턴스마다 진행 중인 요청 (번호, 종류). 나중에 시작한 것이 뒤에 있다
type Active = HashMap<String, Vec<(u64, Kind)>>;
static ACTIVE: LazyLock<Mutex<Active>> = LazyLock::new(Default::default);
static NEXT: AtomicU64 = AtomicU64::new(1);

/// 진행 중 표시. 겹친 요청이 있으면 가장 나중에 시작한 것을 보인다 (재시작은 정지 → 시작으로 바뀐다).
/// 놓으면 자기 것만 빠진다: 먼저 시작한 요청이 먼저 끝나도 끝난 요청의 표시가 남지 않는다
pub struct Transition {
    id: String,
    token: u64,
}

impl Transition {
    pub fn begin(id: &str, kind: Kind) -> Self {
        let token = NEXT.fetch_add(1, Ordering::Relaxed);
        let changed = {
            let mut map = ACTIVE.lock().unwrap_or_else(|e| e.into_inner());
            let list = map.entry(id.to_string()).or_default();
            let before = list.last().map(|(_, k)| *k);
            list.push((token, kind));
            before != Some(kind)
        };
        if changed {
            announce();
        }
        Self { id: id.to_string(), token }
    }
}

impl Drop for Transition {
    fn drop(&mut self) {
        let mut map = ACTIVE.lock().unwrap_or_else(|e| e.into_inner());
        let Some(list) = map.get_mut(&self.id) else { return };
        let before = list.last().map(|(_, k)| *k);
        list.retain(|(token, _)| *token != self.token);
        let now = list.last().map(|(_, k)| *k);
        if list.is_empty() {
            map.remove(&self.id);
        }
        drop(map);
        if now != before {
            announce();
        }
    }
}

/// 지금 진행 중인 것 (목록의 `transition`)
pub fn of(id: &str) -> Option<Kind> {
    ACTIVE.lock().unwrap_or_else(|e| e.into_inner()).get(id).and_then(|list| list.last()).map(|(_, k)| *k)
}

fn announce() {
    crate::events::emit_live("servers.changed", serde_json::json!({}));
}

#[cfg(test)]
mod tests {
    use super::*;

    /// 재시작은 정지하는 중 → 시작하는 중 → 끝으로 보인다
    #[test]
    fn a_restart_shows_stopping_then_starting_then_nothing() {
        let id = "transition-restart";
        assert_eq!(of(id), None);
        {
            let _stop = Transition::begin(id, Kind::Stopping);
            assert_eq!(of(id), Some(Kind::Stopping));
        }
        assert_eq!(of(id), None);
        {
            let _start = Transition::begin(id, Kind::Starting);
            assert_eq!(of(id), Some(Kind::Starting));
            {
                let _inner = Transition::begin(id, Kind::Stopping);
                assert_eq!(of(id), Some(Kind::Stopping));
            }
            assert_eq!(of(id), Some(Kind::Starting), "back to the outer one");
        }
        assert_eq!(of(id), None);
    }

    /// 겹친 요청이 시작한 순서와 다르게 끝나도 남은 요청의 표시만 남고, 다 끝나면 표시가 없다
    #[test]
    fn overlapping_requests_that_end_out_of_order_leave_no_stale_mark() {
        let id = "transition-overlap";
        let first = Transition::begin(id, Kind::Starting);
        let second = Transition::begin(id, Kind::Starting);
        drop(first);
        assert_eq!(of(id), Some(Kind::Starting), "the second start is still running");
        drop(second);
        assert_eq!(of(id), None, "both starts ended");

        // 시작하는 중에 정지가 들어오고, 시작이 먼저 끝난 뒤 정지가 끝났다
        let start = Transition::begin(id, Kind::Starting);
        let stop = Transition::begin(id, Kind::Stopping);
        drop(start);
        assert_eq!(of(id), Some(Kind::Stopping));
        drop(stop);
        assert_eq!(of(id), None, "a stopped server is not left as starting");
    }

    #[tokio::test]
    async fn every_change_tells_clients_to_reread_the_list() {
        let mut rx = crate::events::bus().subscribe(None).rx;
        {
            let _t = Transition::begin("transition-announce", Kind::Starting);
        }
        let mut n = 0;
        while let Ok(e) = rx.try_recv() {
            n += usize::from(e.kind == "servers.changed");
        }
        assert!(n >= 2, "begin and end ({n})");
    }
}
