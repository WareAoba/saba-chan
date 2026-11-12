//! manifest `hooks`의 `event:<종류>`: 데몬 이벤트 버스(`crate::events`)에 난 일을 익스텐션 Python에 넘긴다 (03 §2.3).
//!
//! 버스를 듣는 디스패처는 하나이고, 익스텐션마다 줄 하나와 일꾼 하나를 둔다. 디스패처는 줄에 넣기만 하므로
//! 버스를 막지 않고, 한 익스텐션의 hook이 느리거나 실패해도 다른 익스텐션의 hook은 기다리지 않는다.

use super::ExtensionManager;
use crate::events::Event;
use serde_json::{json, Value};
use std::collections::{HashMap, VecDeque};
use std::future::Future;
use std::sync::{Arc, Mutex};
use tokio::sync::{broadcast, Notify, RwLock};

/// hook 이름 앞머리 (`event:backup.done`)
pub const PREFIX: &str = "event:";
/// 익스텐션 하나에 쌓아 두는 이벤트 수. 넘치면 가장 오래된 것을 버린다
const QUEUE_CAP: usize = 32;
/// hook을 부르는 작업 안에서 난 이벤트는 그 hook보다 한 단계 깊다. 이보다 깊은 것은 넘기지 않는다 (hook ↔ 이벤트 고리)
const MAX_DEPTH: u32 = 3;

/// `event:` 뒤가 이벤트 종류 모양(영문 소문자·숫자·`_`·`.`)인 hook 이름인가
pub fn is_event_hook(name: &str) -> bool {
    name.strip_prefix(PREFIX).is_some_and(|kind| {
        !kind.is_empty() && kind.chars().all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '_' || c == '.')
    })
}

/// 디스패처가 hook을 받을 익스텐션을 찾고 부르는 곳
pub trait Runner: Send + Sync + 'static {
    /// 이 hook을 가진 활성 익스텐션 id
    fn targets(&self, hook: &str) -> impl Future<Output = Vec<String>> + Send;
    /// 익스텐션 하나의 hook을 부르고 끝날 때까지 기다린다
    fn run(&self, ext_id: &str, hook: &str, context: Value) -> impl Future<Output = ()> + Send;
}

impl Runner for RwLock<ExtensionManager> {
    async fn targets(&self, hook: &str) -> Vec<String> {
        self.read().await.hooks_for(hook).iter().map(|(ext, _)| ext.manifest.id.clone()).collect()
    }

    async fn run(&self, ext_id: &str, hook: &str, context: Value) {
        // 실패는 `HookPlan::run`이 기록한다. 결과는 쓰지 않는다
        let _ = super::run_hook_to(self, ext_id, hook, context).await;
    }
}

/// 데몬 버스를 구독하고 디스패처를 띄운다 (데몬에 하나)
pub fn spawn(mgr: Arc<RwLock<ExtensionManager>>) -> tokio::task::JoinHandle<()> {
    dispatch(crate::events::bus().subscribe(None).rx, mgr)
}

struct Pending {
    hook: String,
    event: Arc<Event>,
}

#[derive(Default)]
struct Queue {
    items: VecDeque<Pending>,
    /// 넘쳐 버린 것을 알렸다. 줄이 빌 때까지 다시 알리지 않는다
    warned: bool,
}

#[derive(Default)]
struct Worker {
    queue: Mutex<Queue>,
    wake: Notify,
}

pub fn dispatch<R: Runner>(mut rx: broadcast::Receiver<Arc<Event>>, runner: Arc<R>) -> tokio::task::JoinHandle<()> {
    tokio::spawn(async move {
        let mut workers: HashMap<String, Arc<Worker>> = HashMap::new();
        loop {
            let event = match rx.recv().await {
                Ok(event) => event,
                Err(broadcast::error::RecvError::Lagged(n)) => {
                    tracing::warn!("[EventHooks] fell behind the event bus; {} event(s) were not handed to extensions", n);
                    continue;
                }
                Err(broadcast::error::RecvError::Closed) => return,
            };
            // 지금 붙은 클라이언트에게만 가는 것(진행률, 목록을 다시 읽으라는 알림)은 넘기지 않는다
            if !event.replay {
                continue;
            }
            let hook = format!("{PREFIX}{}", event.kind);
            let targets = runner.targets(&hook).await;
            if targets.is_empty() {
                continue;
            }
            if event.hook_depth > MAX_DEPTH {
                tracing::warn!("[EventHooks] '{}' came from a chain of {} event hooks; not handing it on (hook loop)", event.kind, event.hook_depth);
                continue;
            }
            for ext_id in targets {
                let worker = workers.entry(ext_id).or_insert_with_key(|id| start_worker(id.clone(), runner.clone()));
                worker.push(Pending { hook: hook.clone(), event: event.clone() });
            }
        }
    })
}

impl Worker {
    fn push(&self, item: Pending) {
        let mut queue = self.queue.lock().unwrap_or_else(|e| e.into_inner());
        if queue.items.len() >= QUEUE_CAP {
            queue.items.pop_front();
            if !std::mem::replace(&mut queue.warned, true) {
                tracing::warn!("[EventHooks] an extension is behind on '{}'; dropping its oldest waiting events", item.hook);
            }
        }
        queue.items.push_back(item);
        drop(queue);
        self.wake.notify_one();
    }

    fn pop(&self) -> Option<Pending> {
        let mut queue = self.queue.lock().unwrap_or_else(|e| e.into_inner());
        let next = queue.items.pop_front();
        if next.is_none() {
            queue.warned = false;
        }
        next
    }
}

fn start_worker<R: Runner>(ext_id: String, runner: Arc<R>) -> Arc<Worker> {
    let worker = Arc::new(Worker::default());
    let me = worker.clone();
    tokio::spawn(async move {
        loop {
            let Some(item) = me.pop() else {
                me.wake.notified().await;
                continue;
            };
            let context = json!({ "event": item.event.kind, "id": item.event.id, "data": item.event.data });
            let (runner, ext, depth) = (runner.clone(), ext_id.clone(), item.event.hook_depth + 1);
            // 따로 띄워 hook 하나가 패닉해도 이 익스텐션의 다음 이벤트는 이어진다
            let run = tokio::spawn(crate::events::within_hook(depth, async move { runner.run(&ext, &item.hook, context).await }));
            if let Err(e) = run.await {
                tracing::error!("[EventHooks] event hook of extension '{}' panicked: {}", ext_id, e);
            }
        }
    });
    worker
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::events::Bus;
    use tokio::sync::Semaphore;
    use tokio::time::{sleep, timeout, Duration};

    /// 익스텐션 → 받는 hook. `held`의 hook은 허가를 받아야 끝나고, `echo`의 hook은 받은 종류를 다시 낸다
    struct Fake {
        bus: &'static Bus,
        hooks: Vec<(&'static str, &'static str)>,
        calls: Mutex<Vec<(String, Value)>>,
        held: Option<(&'static str, Semaphore)>,
        echo: Option<&'static str>,
    }

    impl Fake {
        fn new(hooks: Vec<(&'static str, &'static str)>) -> Self {
            Self { bus: Box::leak(Box::new(Bus::new())), hooks, calls: Mutex::default(), held: None, echo: None }
        }

        fn calls_of(&self, ext: &str) -> Vec<Value> {
            self.calls.lock().unwrap().iter().filter(|(e, _)| e == ext).map(|(_, c)| c.clone()).collect()
        }

        async fn until(&self, ext: &str, n: usize) -> Vec<Value> {
            timeout(Duration::from_secs(5), async {
                loop {
                    let calls = self.calls_of(ext);
                    if calls.len() >= n {
                        return calls;
                    }
                    sleep(Duration::from_millis(5)).await;
                }
            })
            .await
            .unwrap_or_else(|_| panic!("{ext} got {:?}", self.calls_of(ext)))
        }
    }

    impl Runner for Fake {
        async fn targets(&self, hook: &str) -> Vec<String> {
            self.hooks.iter().filter(|(_, h)| *h == hook).map(|(e, _)| e.to_string()).collect()
        }

        async fn run(&self, ext_id: &str, hook: &str, context: Value) {
            self.calls.lock().unwrap().push((ext_id.to_string(), context));
            if self.echo == Some(ext_id) {
                self.bus.emit(hook.strip_prefix(PREFIX).unwrap(), json!({}));
            }
            if let Some((held, gate)) = &self.held {
                if *held == ext_id {
                    gate.acquire().await.unwrap().forget();
                }
            }
        }
    }

    fn start(fake: Fake) -> Arc<Fake> {
        let fake = Arc::new(fake);
        dispatch(fake.bus.subscribe(None).rx, fake.clone());
        fake
    }

    #[tokio::test]
    async fn an_event_hook_gets_the_kind_number_and_data_of_kept_events_only() {
        let fake = start(Fake::new(vec![("a", "event:backup.done")]));
        fake.bus.emit_live("backup.done", json!({ "live": true }));
        fake.bus.emit("other.kind", json!({}));
        fake.bus.emit("backup.done", json!({ "instanceId": "i1", "ok": true }));
        let calls = fake.until("a", 1).await;
        sleep(Duration::from_millis(50)).await;
        assert_eq!(fake.calls_of("a").len(), 1, "{:?}", fake.calls_of("a"));
        assert_eq!(calls[0]["event"], "backup.done");
        assert_eq!((calls[0]["data"]["instanceId"].as_str(), calls[0]["data"]["ok"].as_bool()), (Some("i1"), Some(true)));
        assert!(calls[0]["id"].is_u64() && calls[0]["data"]["at"].is_u64());
    }

    #[tokio::test]
    async fn a_stuck_hook_holds_up_only_its_own_extension_and_keeps_its_newest_events() {
        let mut fake = Fake::new(vec![("slow", "event:x"), ("fast", "event:x")]);
        fake.held = Some(("slow", Semaphore::new(0)));
        let fake = start(fake);
        let total = QUEUE_CAP + 8;
        for n in 0..total {
            fake.bus.emit("x", json!({ "n": n }));
            sleep(Duration::from_millis(2)).await;
        }
        let fast = fake.until("fast", total).await;
        assert_eq!(fast.iter().map(|c| c["data"]["n"].as_u64().unwrap() as usize).collect::<Vec<_>>(), (0..total).collect::<Vec<_>>());

        fake.held.as_ref().unwrap().1.add_permits(total);
        sleep(Duration::from_millis(100)).await;
        let slow: Vec<usize> = fake.calls_of("slow").iter().map(|c| c["data"]["n"].as_u64().unwrap() as usize).collect();
        assert!(slow.len() <= QUEUE_CAP + 1, "a stuck extension keeps at most its queue: {slow:?}");
        assert_eq!(slow.last(), Some(&(total - 1)), "the newest event is kept: {slow:?}");
    }

    #[tokio::test]
    async fn an_event_a_hook_causes_does_not_loop_forever() {
        let mut fake = Fake::new(vec![("loop", "event:x"), ("loop", "event:y")]);
        fake.echo = Some("loop");
        let fake = start(fake);
        fake.bus.emit("x", json!({}));
        fake.until("loop", MAX_DEPTH as usize + 1).await;
        sleep(Duration::from_millis(100)).await;
        assert_eq!(fake.calls_of("loop").len(), MAX_DEPTH as usize + 1, "the chain stops");

        // 고리를 끊은 뒤에도 hook 밖에서 난 이벤트는 그대로 받는다
        fake.bus.emit("y", json!({}));
        let calls = fake.until("loop", MAX_DEPTH as usize + 2).await;
        assert_eq!(calls[MAX_DEPTH as usize + 1]["event"], "y");
    }
}
