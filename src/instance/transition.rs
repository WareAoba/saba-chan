//! 시작·정지가 진행 중인 인스턴스. 서버 목록이 `transition`으로 싣고, 바뀌면 `servers.changed`를 낸다.
//! 화면마다 "시작하는 중"을 따로 들고 있지 않게 데몬이 하나로 쥔다 (누른 창 밖의 트레이·다른 창·CLI도 같은 값을 본다)

use std::collections::HashMap;
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

static ACTIVE: LazyLock<Mutex<HashMap<String, Kind>>> = LazyLock::new(Default::default);

/// 진행 중 표시. 놓으면 앞의 표시로 돌아간다 (재시작은 정지 → 시작으로 바뀐다)
pub struct Transition {
    id: String,
    before: Option<Kind>,
}

impl Transition {
    pub fn begin(id: &str, kind: Kind) -> Self {
        let before = ACTIVE.lock().unwrap_or_else(|e| e.into_inner()).insert(id.to_string(), kind);
        if before != Some(kind) {
            announce();
        }
        Self { id: id.to_string(), before }
    }
}

impl Drop for Transition {
    fn drop(&mut self) {
        let mut map = ACTIVE.lock().unwrap_or_else(|e| e.into_inner());
        let now = map.get(&self.id).copied();
        match self.before {
            Some(kind) => map.insert(self.id.clone(), kind),
            None => map.remove(&self.id),
        };
        drop(map);
        if now != self.before {
            announce();
        }
    }
}

/// 지금 진행 중인 것 (목록의 `transition`)
pub fn of(id: &str) -> Option<Kind> {
    ACTIVE.lock().unwrap_or_else(|e| e.into_inner()).get(id).copied()
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
