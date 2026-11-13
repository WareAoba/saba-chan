//! 릴레이에 원격 데몬 주소 올리기: 작업 하나가 올리려는 값(`Wanted`)을 따라 올리고, 실패하면 다시 올린다.
//!
//! 설정 적용(`apply`)과 공인 IP·공개 주소 변경(`readdress`)은 값만 바꾼다. 요청은 이 작업 하나가 하나씩 보내므로
//! 보내는 동안 바뀐 값은 그 요청이 끝난 뒤에 올라가고, 늦게 닿은 예전 주소가 새 주소를 덮지 않는다

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::time::Duration;

use tokio::sync::watch;
use tokio_util::sync::CancellationToken;

use super::relay::{self, Published};
use super::server::RelaySource;

/// 올리기에 실패한 뒤 처음 다시 올릴 때까지의 간격과, 간격의 상한
const RETRY_FIRST: Duration = if cfg!(test) { Duration::from_millis(10) } else { Duration::from_secs(5) };
const RETRY_MAX: Duration = Duration::from_secs(300);

/// 릴레이에 올리는 직접 연결 주소
#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) struct Endpoint {
    /// 설정한 주소, 비었으면 릴레이가 본 이 PC의 공인 IP (`None`: 주소 없이, 터널로만)
    pub address: Option<String>,
    pub port: u16,
    pub cert_sha256: String,
}

/// 작업이 따라가는 값
#[derive(Debug, Clone, Default)]
struct Wanted {
    /// 적용(`apply`)마다 늘린다: 같은 주소도 다시 올린다 (노드 토큰이 바뀌었을 수 있다)
    applied: u64,
    /// 원격 포트가 열려 있으면 올릴 것. 없으면 (전에 올렸을 때) 내린다
    endpoint: Option<Endpoint>,
    /// 닫았을 때 이 작업이 올린 적이 없어도 내리는가 (데몬이 다시 뜬 뒤에는 전에 올렸는지 모른다)
    withdraw: bool,
}

/// 작업이 마지막으로 끝낸 적용 번호와, 릴레이에 지금 주소를 올렸는지
#[derive(Debug, Clone, Copy, Default)]
struct Done {
    applied: u64,
    published: bool,
}

#[derive(Clone)]
pub(super) struct Publisher(Arc<Inner>);

struct Inner {
    wanted: watch::Sender<Wanted>,
    done: watch::Sender<Done>,
    /// 작업을 띄웠는가 (첫 적용 때 띄운다: 릴레이를 어디서 찾는지와 멈출 때를 그때 안다)
    running: AtomicBool,
}

impl Default for Publisher {
    fn default() -> Self {
        Self(Arc::new(Inner { wanted: watch::Sender::new(Wanted::default()), done: watch::Sender::new(Done::default()), running: AtomicBool::new(false) }))
    }
}

impl Publisher {
    /// 설정을 적용한 뒤: 원격 포트가 열려 있으면 `endpoint`를 올리고, 닫혔으면 전에 올렸거나 `withdraw`면 내린다.
    /// 이번 적용의 첫 결과(올렸는가)를 기다린다. 올리기에 실패하면 작업이 뒤에서 다시 올린다
    pub(super) async fn apply(&self, relay: &RelaySource, stop: &CancellationToken, endpoint: Option<Endpoint>, withdraw: bool) -> bool {
        if !self.0.running.swap(true, Ordering::SeqCst) {
            tokio::spawn(self.clone().run(relay.clone(), stop.clone()));
        }
        let mut done = self.0.done.subscribe();
        let mut applied = 0;
        self.0.wanted.send_modify(|w| {
            w.applied += 1;
            applied = w.applied;
            w.endpoint = endpoint;
            w.withdraw = withdraw;
        });
        tokio::select! {
            d = done.wait_for(|d| d.applied >= applied) => d.is_ok_and(|d| d.published),
            _ = stop.cancelled() => false,
        }
    }

    /// 올릴 주소만 바꾼다 (원격 포트가 열려 있을 때. 포트는 다시 열지 않는다). 작업이 뒤에서 올린다
    pub(super) fn readdress(&self, address: Option<String>) {
        self.0.wanted.send_if_modified(|w| match &mut w.endpoint {
            Some(ep) if ep.address != address => {
                ep.address = address;
                true
            }
            _ => false,
        });
    }

    /// 릴레이에 지금 주소를 올렸는가 (다시 올리기가 올린 것 포함)
    pub(super) fn published(&self) -> bool {
        self.0.done.borrow().published
    }

    /// 올리려는 값을 따라간다. 실패하면 `RETRY_FIRST`부터 두 배씩(상한 `RETRY_MAX`) 다시 올리고, 그동안 값이 바뀌면 새 값으로 간다.
    /// 릴레이가 받지 않거나(`Refused`) 노드 토큰을 거절하면(`Unauthorized`, 토큰이 바뀌면 다시 적용한다) 값이 바뀔 때까지 그만둔다.
    /// `stop`(데몬 종료)이면 기다림 중에 끝난다. 보내는 중인 요청은 끝까지 기다린다
    async fn run(self, relay: RelaySource, stop: CancellationToken) {
        let mut wanted = self.0.wanted.subscribe();
        let (mut applied, mut published) = (0, false);
        'follow: loop {
            let want = wanted.borrow_and_update().clone();
            let fresh = want.applied != applied;
            applied = want.applied;
            match &want.endpoint {
                Some(ep) => {
                    // 새 적용은 처음부터 본다. 주소만 바뀐 것은 실패해도 앞서 올린 것이 릴레이에 남아 있다
                    if fresh {
                        published = false;
                    }
                    let outcome = publish(&relay, ep).await;
                    match (outcome, fresh) {
                        (Some(Published::Done), false) => tracing::info!("Published the remote daemon address the relay sees"),
                        (Some(Published::Refused), true) => tracing::warn!("The relay refused the remote daemon endpoint"),
                        (Some(Published::Unauthorized), true) => tracing::warn!("The relay rejected the node token; not publishing the remote daemon address"),
                        // 부팅 직후 네트워크가 아직이거나 릴레이가 잠시 받지 않으면, 포트가 열려 있는 동안 다시 올린다
                        (Some(Published::Failed), true) => tracing::warn!("Failed to publish the remote daemon address to the relay; retrying"),
                        _ => {}
                    }
                    published |= outcome == Some(Published::Done);
                    self.0.done.send_replace(Done { applied, published });
                    let (mut outcome, mut wait) = (outcome, RETRY_FIRST);
                    while outcome == Some(Published::Failed) {
                        tokio::select! {
                            _ = stop.cancelled() => return,
                            _ = wanted.changed() => continue 'follow,
                            _ = tokio::time::sleep(wait) => {}
                        }
                        wait = (wait * 2).min(RETRY_MAX);
                        // 그새 릴레이 연결이 없어졌으면 다음 간격에 다시 본다
                        outcome = publish(&relay, ep).await.or(Some(Published::Failed));
                        if outcome == Some(Published::Done) {
                            tracing::info!("Published the remote daemon address to the relay");
                            published = true;
                            self.0.done.send_replace(Done { applied, published });
                        }
                    }
                }
                None if fresh => {
                    if published || want.withdraw {
                        if let Some(t) = relay.target().await {
                            if !relay::withdraw(&t).await {
                                tracing::warn!("Failed to withdraw the remote daemon address from the relay");
                            }
                        }
                    }
                    published = false;
                    self.0.done.send_replace(Done { applied, published });
                }
                None => {}
            }
            tokio::select! {
                _ = stop.cancelled() => return,
                _ = wanted.changed() => {}
            }
        }
    }
}

/// 한 번 올린다. 릴레이에 연결하지 않았으면 `None`
async fn publish(relay: &RelaySource, ep: &Endpoint) -> Option<Published> {
    let target = relay.target().await?;
    Some(relay::publish(&target, ep.address.as_deref(), ep.port, &ep.cert_sha256).await)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::AtomicUsize;

    fn endpoint(address: Option<&str>) -> Option<Endpoint> {
        Some(Endpoint { address: address.map(str::to_string), port: 57480, cert_sha256: "ab".repeat(32) })
    }

    /// 릴레이 흉내: `/api/remote/endpoint` PUT을 `answer`(몇 번째 요청, 본문)로 답한다
    async fn relay_with<F, Fut>(answer: F) -> RelaySource
    where
        F: Fn(usize, serde_json::Value) -> Fut + Clone + Send + Sync + 'static,
        Fut: std::future::Future<Output = axum::http::StatusCode> + Send + 'static,
    {
        let calls = Arc::new(AtomicUsize::new(0));
        let app = axum::Router::new().route(
            "/api/remote/endpoint",
            axum::routing::put(move |axum::Json(body): axum::Json<serde_json::Value>| {
                let n = calls.fetch_add(1, Ordering::SeqCst);
                answer(n, body)
            }),
        );
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let base = format!("http://{}", listener.local_addr().unwrap());
        tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });
        RelaySource::Fixed(relay::Target { base, token: "sbn_host-b.secret".into() })
    }

    /// 릴레이 흉내: 처음 `fail`번은 503, 그 뒤로 204
    async fn flaky_relay(fail: usize) -> (RelaySource, Arc<AtomicUsize>) {
        let calls = Arc::new(AtomicUsize::new(0));
        let seen = calls.clone();
        let relay = relay_with(move |n, _| {
            seen.fetch_add(1, Ordering::SeqCst);
            async move { if n < fail { axum::http::StatusCode::SERVICE_UNAVAILABLE } else { axum::http::StatusCode::NO_CONTENT } }
        })
        .await;
        (relay, calls)
    }

    async fn eventually(what: &str, check: impl Fn() -> bool) {
        let started = std::time::Instant::now();
        while !check() {
            assert!(started.elapsed() < Duration::from_secs(5), "{what}");
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
    }

    /// 부팅 직후처럼 릴레이가 잠시 받지 않아도, 포트가 열려 있는 동안 다시 올려 결국 올린다
    #[tokio::test]
    async fn a_failed_publish_is_retried_until_the_relay_takes_it() {
        let (relay, calls) = flaky_relay(2).await;
        let publisher = Publisher::default();
        assert!(!publisher.apply(&relay, &CancellationToken::new(), endpoint(Some("saba.example.com")), false).await);
        eventually("published after retrying", || publisher.published()).await;
        assert_eq!(calls.load(Ordering::SeqCst), 3);
    }

    /// 릴레이가 본문을 받지 않으면(400) 같은 본문을 다시 보내지 않는다
    #[tokio::test]
    async fn a_relay_that_refuses_the_endpoint_is_not_asked_again() {
        let calls = Arc::new(AtomicUsize::new(0));
        let seen = calls.clone();
        let relay = relay_with(move |_, body| {
            seen.fetch_add(1, Ordering::SeqCst);
            async move { if body["address"].is_null() { axum::http::StatusCode::BAD_REQUEST } else { axum::http::StatusCode::NO_CONTENT } }
        })
        .await;
        let publisher = Publisher::default();
        assert!(!publisher.apply(&relay, &CancellationToken::new(), endpoint(None), false).await);
        tokio::time::sleep(Duration::from_millis(100)).await;
        assert_eq!(calls.load(Ordering::SeqCst), 1);
    }

    /// 릴레이가 이 노드 토큰을 받지 않으면(401, 같은 계정의 다른 PC가 페어링해 토큰이 대체됨 등) 다시 보내지 않는다.
    /// 같은 토큰으로는 계속 거절되고, 토큰이 바뀌어 다시 적용하면 같은 주소라도 다시 올린다
    #[tokio::test]
    async fn a_relay_that_rejects_the_node_token_is_asked_again_only_on_the_next_apply() {
        let calls = Arc::new(AtomicUsize::new(0));
        let seen = calls.clone();
        let relay = relay_with(move |_, _| {
            seen.fetch_add(1, Ordering::SeqCst);
            async { axum::http::StatusCode::UNAUTHORIZED }
        })
        .await;
        let (publisher, stop) = (Publisher::default(), CancellationToken::new());
        assert!(!publisher.apply(&relay, &stop, endpoint(Some("saba.example.com")), false).await);
        tokio::time::sleep(Duration::from_millis(100)).await;
        assert_eq!(calls.load(Ordering::SeqCst), 1);
        publisher.apply(&relay, &stop, endpoint(Some("saba.example.com")), false).await;
        assert_eq!(calls.load(Ordering::SeqCst), 2);
    }

    /// 부팅 직후 주소 없이 올리려다 실패해 다시 올리는 중에 릴레이가 본 공인 IP를 알게 되면, 나중 주소가 남는다:
    /// 다시 올리던 예전 주소(없음)가 새 주소를 덮지 않는다
    #[tokio::test]
    async fn an_address_learned_while_retrying_is_not_overwritten_by_the_stale_retry() {
        let up = Arc::new(AtomicBool::new(false));
        let last = Arc::new(std::sync::Mutex::new(None));
        let (online, record) = (up.clone(), last.clone());
        let relay = relay_with(move |_, body| {
            let ok = online.load(Ordering::SeqCst);
            if ok {
                *record.lock().unwrap() = Some(body["address"].clone());
            }
            async move { if ok { axum::http::StatusCode::NO_CONTENT } else { axum::http::StatusCode::SERVICE_UNAVAILABLE } }
        })
        .await;
        let publisher = Publisher::default();
        assert!(!publisher.apply(&relay, &CancellationToken::new(), endpoint(None), false).await);
        up.store(true, Ordering::SeqCst);
        publisher.readdress(Some("203.0.113.7".into()));
        eventually("published", || publisher.published()).await;
        tokio::time::sleep(Duration::from_millis(200)).await;
        assert_eq!(*last.lock().unwrap(), Some(serde_json::json!("203.0.113.7")), "the later address is what the relay keeps");
    }

    /// 주소가 바뀌면 보내던 요청이 끝난 뒤에 새 주소를 보낸다: 요청은 겹치지 않고, 릴레이에는 새 주소가 남는다
    #[tokio::test]
    async fn a_new_address_is_sent_after_the_request_in_flight() {
        let (sending, most) = (Arc::new(AtomicUsize::new(0)), Arc::new(AtomicUsize::new(0)));
        let last = Arc::new(std::sync::Mutex::new(None));
        let (now, peak, record) = (sending.clone(), most.clone(), last.clone());
        let relay = relay_with(move |_, body| {
            let (now, peak, record) = (now.clone(), peak.clone(), record.clone());
            async move {
                peak.fetch_max(now.fetch_add(1, Ordering::SeqCst) + 1, Ordering::SeqCst);
                tokio::time::sleep(Duration::from_millis(300)).await;
                *record.lock().unwrap() = Some(body["address"].clone());
                now.fetch_sub(1, Ordering::SeqCst);
                axum::http::StatusCode::NO_CONTENT
            }
        })
        .await;
        let (publisher, stop) = (Publisher::default(), CancellationToken::new());
        let applying = publisher.apply(&relay, &stop, endpoint(Some("198.51.100.1")), false);
        let (published, ()) = tokio::join!(applying, async {
            tokio::time::sleep(Duration::from_millis(100)).await;
            publisher.readdress(Some("203.0.113.7".into()));
        });
        assert!(published);
        eventually("the new address arrives", || *last.lock().unwrap() == Some(serde_json::json!("203.0.113.7"))).await;
        assert_eq!(most.load(Ordering::SeqCst), 1, "one request at a time");
    }

    /// 포트를 닫으면 다시 올리기를 멈춘다
    #[tokio::test]
    async fn retrying_stops_once_the_port_closes() {
        let (relay, calls) = flaky_relay(usize::MAX).await;
        let (publisher, stop) = (Publisher::default(), CancellationToken::new());
        publisher.apply(&relay, &stop, endpoint(Some("saba.example.com")), false).await;
        tokio::time::sleep(Duration::from_millis(100)).await;
        assert!(!publisher.apply(&relay, &stop, None, false).await);
        let after = calls.load(Ordering::SeqCst);
        tokio::time::sleep(Duration::from_millis(100)).await;
        assert_eq!(calls.load(Ordering::SeqCst), after, "no more attempts after the port closed");
    }

    /// 적용이 도는 동안(포트를 쥐고 올리기를 기다리는 동안) 물은 상태는 적용이 끝난 뒤의 것 하나다:
    /// 올렸는가와 포트가 열렸는가가 적용 전·후로 섞이지 않는다
    #[tokio::test]
    async fn status_asked_during_an_apply_is_the_state_after_it() {
        let remote = super::super::RemoteDaemon::default();
        let mut port = remote.port.lock().await;
        let asked = tokio::spawn({
            let remote = remote.clone();
            async move { remote.status().await }
        });
        for _ in 0..10 {
            tokio::task::yield_now().await;
        }
        remote.publisher.0.done.send_replace(Done { applied: 1, published: true });
        port.status.listening = true;
        drop(port);
        let status = asked.await.unwrap();
        assert_eq!((status.listening, status.published), (true, true));
    }
}
