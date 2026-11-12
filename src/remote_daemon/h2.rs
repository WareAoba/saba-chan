//! 원격 데몬까지의 HTTP/2 연결: 원격 PC마다 하나를 열어 두고 요청을 모두 싣는다 (터널 T1·직접 연결 D1b가 같이 쓴다)

use std::collections::BTreeMap;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use axum::body::Body;
use tokio::io::{AsyncRead, AsyncWrite};

use super::tls;

pub type Sender = hyper::client::conn::http2::SendRequest<Body>;

/// 요청 하나의 답 본문을 원격 데몬이 앞서 보내 둘 수 있는 양. 받는 쪽(앱·웹)이 읽지 않는 스트림이 이만큼에서 멈춰도
/// 연결 전체 몫(`CONNECTION_WINDOW`)은 스트림 16개가 멈출 때까지 남아 다른 요청이 계속 간다
const STREAM_WINDOW: u32 = 1024 * 1024;
const CONNECTION_WINDOW: u32 = 16 * STREAM_WINDOW;

type Slot = Arc<tokio::sync::Mutex<Option<Sender>>>;
/// 잡아 둔 연결 자리. 쥐고 있는 동안 같은 원격 PC의 다른 요청은 기다린다
pub type Held = tokio::sync::OwnedMutexGuard<Option<Sender>>;

/// 키(원격 PC)마다 연결 하나. 여는 동안 같은 키의 다른 요청은 그 연결을 기다렸다가 같이 쓴다
pub struct Connections<K>(Mutex<BTreeMap<K, Slot>>);

impl<K: Ord> Connections<K> {
    pub const fn new() -> Self {
        Self(Mutex::new(BTreeMap::new()))
    }

    fn map(&self) -> std::sync::MutexGuard<'_, BTreeMap<K, Slot>> {
        self.0.lock().unwrap_or_else(|e| e.into_inner())
    }

    /// 그 키의 자리를 잡는다. 열린 연결이 있으면 `live`로 꺼내 쓰고, 없으면 새로 열어 넣는다
    pub async fn hold(&self, key: K) -> Held {
        let slot = self.map().entry(key).or_default().clone();
        slot.lock_owned().await
    }

    /// 연결을 놓는다. 그 위의 요청·스트림이 끝나면 연결이 닫힌다
    pub fn remove(&self, key: &K) {
        self.map().remove(key);
    }

    pub fn retain(&self, mut keep: impl FnMut(&K) -> bool) {
        self.map().retain(|k, _| keep(k));
    }
}

/// 잡은 자리에 아직 열린 연결
pub fn live(held: &Held) -> Option<Sender> {
    held.as_ref().filter(|s| !s.is_closed()).cloned()
}

/// 지문을 고정해 맺은 TLS 위에 HTTP/2 연결을 연다
pub async fn handshake<T>(io: T) -> Option<Sender>
where
    T: AsyncRead + AsyncWrite + Send + Unpin + 'static,
{
    let (send, conn) = hyper::client::conn::http2::Builder::new(hyper_util::rt::TokioExecutor::new())
        .timer(hyper_util::rt::TokioTimer::new())
        .keep_alive_interval(Duration::from_secs(25))
        .keep_alive_timeout(Duration::from_secs(20))
        .initial_stream_window_size(STREAM_WINDOW)
        .initial_connection_window_size(CONNECTION_WINDOW)
        .handshake(hyper_util::rt::TokioIo::new(io))
        .await
        .ok()?;
    tokio::spawn(conn);
    Some(send)
}

/// HTTP/2 요청은 절대 주소를 쓴다 (`:authority`)
pub fn absolute(req: &mut hyper::Request<Body>) {
    let path = req
        .uri()
        .path_and_query()
        .map(|p| p.as_str().to_string())
        .unwrap_or_else(|| "/".into());
    if let Ok(uri) = format!("https://{}{}", tls::SERVER_NAME, path).parse() {
        *req.uri_mut() = uri;
    }
    req.headers_mut().remove(hyper::header::HOST);
}
