//! 콘솔 조회의 `?wait=<초>`: 새 줄이 없으면 바로 빈 답을 주지 않고 줄이 생길 때까지 붙잡는다.
//! 클라이언트는 0.5초마다 빈 요청을 보내지 않고, 답을 받으면 곧바로 다음 요청을 건다.
//! `…/console/stream`(SSE)은 같은 일을 데몬 안에서 이어 한다: 클라이언트 대신 커서를 들고 [`read_waiting`]을 되풀이해,
//! 새 줄·상태가 생길 때마다 콘솔 조회와 같은 모양의 답을 한 건씩 흘려보낸다 ([`stream`])

use serde_json::Value;
use std::future::Future;
use std::time::Duration;
use tokio::sync::Notify;

/// 한 요청이 기다리는 가장 긴 시간
pub const MAX_WAIT: Duration = Duration::from_secs(25);

/// 쿼리의 `wait`(초)를 기다릴 시간으로. 없거나 0이면 기다리지 않는다
pub fn wait_of(params: &std::collections::HashMap<String, String>) -> Duration {
    params
        .get("wait")
        .and_then(|w| w.parse::<u64>().ok())
        .map(|s| Duration::from_secs(s).min(MAX_WAIT))
        .unwrap_or_default()
}

/// 화면이 다시 그려야 할 콘솔 상태: 버퍼 세대, 실행 여부, 출처(디스크 기록), 입력 가능 여부
fn shape(v: &Value) -> [Value; 4] {
    ["epoch", "running", "source", "stdin_available"].map(|k| v.get(k).cloned().unwrap_or(Value::Null))
}

/// `read`가 준 답에 새 줄(`lines`)이 있거나, 클라이언트가 아는 세대(`known_epoch`)와 다르거나,
/// 기다리는 사이 콘솔 상태가 바뀌면 그 답을 준다. 아니면 `changed`가 울릴 때마다 다시 읽고, `wait`가 지나면 마지막 답을 준다.
/// 기다린 답에는 `waits: true`를 붙인다 (클라이언트가 이 데몬이 붙잡는 것을 안다)
pub async fn read_waiting<F, Fut>(changed: &Notify, wait: Duration, known_epoch: Option<u64>, read: F) -> anyhow::Result<Value>
where
    F: FnMut() -> Fut,
    Fut: Future<Output = anyhow::Result<Value>>,
{
    read_waiting_from(changed, wait, known_epoch, None, read).await
}

/// [`read_waiting`]과 같되, 받는 쪽이 이미 아는 콘솔 상태(`known`)와 다르면 첫 읽기에서 바로 답한다
async fn read_waiting_from<F, Fut>(
    changed: &Notify,
    wait: Duration,
    known_epoch: Option<u64>,
    known: Option<[Value; 4]>,
    mut read: F,
) -> anyhow::Result<Value>
where
    F: FnMut() -> Fut,
    Fut: Future<Output = anyhow::Result<Value>>,
{
    let deadline = tokio::time::Instant::now() + wait;
    let mut first: Option<[Value; 4]> = known;
    loop {
        // 읽기 전에 알림을 걸어 둔다: 읽은 뒤 기다리기 전에 들어온 줄을 놓치지 않게
        let notified = changed.notified();
        tokio::pin!(notified);
        notified.as_mut().enable();
        let mut result = read().await?;
        if wait.is_zero() {
            return Ok(result);
        }
        let now_shape = shape(&result);
        let has_lines = result.get("lines").and_then(Value::as_array).is_some_and(|l| !l.is_empty());
        // 세대가 없는 답(꺼진 서버의 디스크 기록)은 세대로 가리지 않는다 (출처가 바뀐 것은 아래 `moved`가 본다)
        let other_epoch = known_epoch.is_some_and(|e| result.get("epoch").and_then(Value::as_u64).is_some_and(|now| now != e));
        let moved = first.as_ref().is_some_and(|s| *s != now_shape);
        if has_lines || other_epoch || moved || tokio::time::Instant::now() >= deadline {
            if let Some(obj) = result.as_object_mut() {
                obj.insert("waits".into(), Value::Bool(true));
            }
            return Ok(result);
        }
        first.get_or_insert(now_shape);
        let _ = tokio::time::timeout_at(deadline, notified).await;
    }
}

/// 스트림이 들고 다니는 읽을 자리: 다음 줄 번호, 아는 세대, 꺼진 서버의 디스크 기록을 이미 보냈는가
#[derive(Debug, Clone, Copy, Default, PartialEq)]
pub struct Cursor {
    pub since: Option<u64>,
    pub epoch: Option<u64>,
    pub disk_seen: bool,
}

impl Cursor {
    /// 받는 쪽이 이미 받은 데: `since`(다음 줄 번호), `epoch`(그 버퍼 세대), `disk=seen`(꺼진 서버의 디스크 기록을 받았다)
    pub fn from_query(params: &std::collections::HashMap<String, String>) -> Self {
        let num = |k: &str| params.get(k).and_then(|v| v.parse::<u64>().ok());
        Self { since: num("since"), epoch: num("epoch"), disk_seen: params.get("disk").is_some_and(|v| v == "seen") }
    }

    /// 보낸 답 다음 자리
    fn after(self, v: &Value) -> Self {
        // 꺼진 서버의 디스크 기록은 줄 번호가 실행 중 버퍼와 따로다: 자리를 잊고, 다음에 켜진 프로세스는 처음부터 읽는다
        if v.get("source").and_then(Value::as_str) == Some("disk") {
            return Self { since: None, epoch: None, disk_seen: true };
        }
        let last = v.get("lines").and_then(Value::as_array).and_then(|l| l.last()).and_then(|l| l.get("id")).and_then(Value::as_u64);
        Self {
            since: last.map(|id| id + 1).or(self.since),
            epoch: v.get("epoch").and_then(Value::as_u64).or(self.epoch),
            disk_seen: false,
        }
    }
}

/// 콘솔 스트림 (SSE): 처음 답(`start` 자리부터: 받는 쪽이 이미 받은 데까지)을 보내고, 새 줄이 생기거나 콘솔 상태가 바뀔 때마다 한 건씩 보낸다.
/// 한 건의 `data`는 콘솔 조회(`GET …/console`)의 답과 같은 모양이다. 버퍼 세대가 바뀌면(프로세스를 다시 켬)
/// 처음부터 다시 읽어 `reset: true`를 붙인다: 받은 쪽은 지금까지 받은 줄을 버리고 이 답으로 다시 그린다.
/// 읽다가 오류가 나면 `event: error`를 한 건 보내고 끝낸다. `stop`(데몬 종료)이면 끝낸다
pub fn stream<F, Fut>(
    changed: &'static Notify,
    stop: tokio_util::sync::CancellationToken,
    start: Cursor,
    read: F,
) -> impl futures_util::Stream<Item = Result<axum::response::sse::Event, std::convert::Infallible>>
where
    F: FnMut(Cursor) -> Fut + Send + 'static,
    Fut: Future<Output = anyhow::Result<Value>> + Send,
{
    use axum::response::sse::Event;
    use futures_util::StreamExt;
    updates(changed, stop, start, read).map(|u| {
        Ok(match u {
            Ok(v) => Event::default().data(v.to_string()),
            Err(e) => Event::default().event("error").data(e.to_string()),
        })
    })
}

/// 콘솔 스트림을 SSE 답으로. 연결을 살려 두는 주석 줄은 `/api/events`와 같은 간격이다
pub fn sse(
    updates: impl futures_util::Stream<Item = Result<axum::response::sse::Event, std::convert::Infallible>> + Send + 'static,
) -> axum::response::Response {
    use axum::response::sse::{KeepAlive, Sse};
    use axum::response::IntoResponse;
    Sse::new(updates).keep_alive(KeepAlive::new().interval(super::events::PING).text("ping")).into_response()
}

/// [`stream`]의 한 건씩: 콘솔 답, 또는 끝내기 전 오류 본문
fn updates<F, Fut>(
    changed: &'static Notify,
    stop: tokio_util::sync::CancellationToken,
    start: Cursor,
    read: F,
) -> impl futures_util::Stream<Item = Result<Value, Value>>
where
    F: FnMut(Cursor) -> Fut + Send + 'static,
    Fut: Future<Output = anyhow::Result<Value>> + Send,
{
    struct State<F> {
        read: F,
        cursor: Cursor,
        sent: Option<[Value; 4]>,
        done: bool,
    }
    let init = State { read, cursor: start, sent: None, done: false };
    futures_util::stream::unfold(init, move |mut st| {
        let stop = stop.clone();
        async move {
            if st.done {
                return None;
            }
            loop {
                let cur = st.cursor;
                let first = st.sent.is_none();
                let wait = if first { Duration::ZERO } else { MAX_WAIT };
                let read = &mut st.read;
                let got = tokio::select! {
                    _ = stop.cancelled() => return None,
                    got = read_waiting_from(changed, wait, cur.epoch, st.sent.clone(), || read(cur)) => got,
                };
                let mut v = match got {
                    Ok(v) => v,
                    Err(e) => {
                        st.done = true;
                        let body = serde_json::json!({ "success": false, "error": e.to_string(), "error_code": "not_found" });
                        return Some((Err(body), st));
                    }
                };
                // 아는 세대(처음이면 받는 쪽이 준 것)와 다르면 줄 번호가 0부터다
                let restarted = cur.epoch.is_some_and(|e| v.get("epoch").and_then(Value::as_u64).is_some_and(|now| now != e));
                if restarted {
                    match (st.read)(Cursor { since: Some(0), epoch: None, disk_seen: false }).await {
                        Ok(again) => v = again,
                        Err(_) => continue,
                    }
                    v["reset"] = Value::Bool(true);
                    st.cursor = Cursor::default();
                }
                if let Some(obj) = v.as_object_mut() {
                    obj.remove("waits");
                }
                let now_shape = shape(&v);
                let has_lines = v.get("lines").and_then(Value::as_array).is_some_and(|l| !l.is_empty());
                st.cursor = st.cursor.after(&v);
                if first || restarted || has_lines || st.sent.as_ref() != Some(&now_shape) {
                    st.sent = Some(now_shape);
                    return Some((Ok(v), st));
                }
            }
        }
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;
    use std::sync::{Arc, Mutex};

    fn reader(state: Arc<Mutex<Value>>) -> impl FnMut() -> std::pin::Pin<Box<dyn Future<Output = anyhow::Result<Value>> + Send>> {
        move || {
            let v = state.lock().unwrap().clone();
            Box::pin(async move { Ok(v) })
        }
    }

    #[tokio::test]
    async fn waits_until_a_new_line_arrives_then_answers_with_it() {
        let changed = Arc::new(Notify::new());
        let state = Arc::new(Mutex::new(json!({"lines": [], "epoch": 7, "running": true})));
        let (c, s) = (changed.clone(), state.clone());
        tokio::spawn(async move {
            tokio::time::sleep(Duration::from_millis(150)).await;
            *s.lock().unwrap() = json!({"lines": [{"id": 5}], "epoch": 7, "running": true});
            c.notify_waiters();
        });
        let started = tokio::time::Instant::now();
        let got = read_waiting(&changed, Duration::from_millis(800), Some(7), reader(state)).await.unwrap();
        assert_eq!(got["lines"][0]["id"], 5);
        assert_eq!(got["waits"], true);
        assert!(started.elapsed() < Duration::from_millis(600), "줄이 들어오면 바로 답한다");
    }

    #[tokio::test]
    async fn an_empty_console_answers_empty_after_the_wait() {
        let changed = Notify::new();
        let state = Arc::new(Mutex::new(json!({"lines": [], "epoch": 7})));
        let started = tokio::time::Instant::now();
        let got = read_waiting(&changed, Duration::from_millis(800), Some(7), reader(state)).await.unwrap();
        assert_eq!(got["lines"], json!([]));
        assert_eq!(got["waits"], true);
        assert!(started.elapsed() >= Duration::from_millis(790));
    }

    #[tokio::test]
    async fn a_restarted_console_or_a_changed_state_answers_at_once() {
        let changed = Arc::new(Notify::new());
        // 클라이언트가 아는 세대와 다르면 줄이 없어도 바로 (다시 시작한 프로세스는 줄 번호가 0부터다)
        let state = Arc::new(Mutex::new(json!({"lines": [], "epoch": 8})));
        let started = tokio::time::Instant::now();
        read_waiting(&changed, Duration::from_millis(800), Some(7), reader(state)).await.unwrap();
        assert!(started.elapsed() < Duration::from_millis(400));

        // 꺼진 서버의 디스크 기록을 보다가 서버가 켜지면 (출처·세대가 바뀐다) 바로
        let state = Arc::new(Mutex::new(json!({"lines": [], "source": "disk", "running": false})));
        let (c, s) = (changed.clone(), state.clone());
        tokio::spawn(async move {
            tokio::time::sleep(Duration::from_millis(100)).await;
            *s.lock().unwrap() = json!({"lines": [], "epoch": 9, "running": true});
            c.notify_waiters();
        });
        let got = read_waiting(&changed, Duration::from_millis(800), None, reader(state)).await.unwrap();
        assert_eq!(got["epoch"], 9);
        assert!(started.elapsed() < Duration::from_millis(600));
    }

    /// 꺼진 서버의 디스크 기록(세대가 없다)은 클라이언트가 아는 세대와 다르다고 보지 않는다: 붙잡는다
    #[tokio::test]
    async fn a_disk_record_without_an_epoch_is_held() {
        let changed = Notify::new();
        let state = Arc::new(Mutex::new(json!({"lines": [], "source": "disk", "running": false})));
        let started = tokio::time::Instant::now();
        read_waiting(&changed, Duration::from_millis(300), Some(7), reader(state)).await.unwrap();
        assert!(started.elapsed() >= Duration::from_millis(290), "held {:?}", started.elapsed());
    }

    /// 커서대로 읽는 가짜 콘솔: id가 since 이상인 줄만 준다
    fn console(state: Arc<Mutex<Value>>) -> impl FnMut(Cursor) -> std::pin::Pin<Box<dyn Future<Output = anyhow::Result<Value>> + Send>> {
        move |cur: Cursor| {
            let mut v = state.lock().unwrap().clone();
            let since = cur.since.unwrap_or(0);
            if let Some(lines) = v["lines"].as_array_mut() {
                lines.retain(|l| l["id"].as_u64().unwrap_or(0) >= since);
            }
            Box::pin(async move { Ok(v) })
        }
    }

    async fn next(s: &mut (impl futures_util::Stream<Item = Result<Value, Value>> + Unpin)) -> Result<Value, Value> {
        use futures_util::StreamExt;
        tokio::time::timeout(Duration::from_secs(2), s.next()).await.expect("no event").expect("ended")
    }
    fn texts(v: &Value) -> Vec<&str> {
        v["lines"].as_array().unwrap().iter().map(|l| l["t"].as_str().unwrap()).collect()
    }

    #[tokio::test]
    async fn the_stream_sends_what_is_there_then_each_new_line_and_starts_over_when_the_process_restarts() {
        let changed: &'static Notify = Box::leak(Box::new(Notify::new()));
        let state = Arc::new(Mutex::new(json!({"lines": [{"id": 0, "t": "a"}], "epoch": 1, "running": true})));
        let stop = tokio_util::sync::CancellationToken::new();
        let mut s = Box::pin(updates(changed, stop.clone(), Cursor::default(), console(state.clone())));
        assert_eq!(texts(&next(&mut s).await.unwrap()), ["a"], "처음에는 지금 있는 줄");

        // 새 줄만 보낸다 (받은 줄을 다시 보내지 않는다)
        *state.lock().unwrap() = json!({"lines": [{"id": 0, "t": "a"}, {"id": 1, "t": "b"}], "epoch": 1, "running": true});
        changed.notify_waiters();
        assert_eq!(texts(&next(&mut s).await.unwrap()), ["b"]);

        // 다시 켜면 세대가 바뀌고 줄 번호가 0부터다: 처음부터 다시 보내고 reset을 붙인다
        *state.lock().unwrap() = json!({"lines": [{"id": 0, "t": "c"}], "epoch": 2, "running": true});
        changed.notify_waiters();
        let got = next(&mut s).await.unwrap();
        assert_eq!((texts(&got), &got["reset"]), (vec!["c"], &json!(true)));

        // 꺼지면 (상태가 바뀐다) 줄이 없어도 알린다
        *state.lock().unwrap() = json!({"lines": [], "epoch": 2, "running": false});
        changed.notify_waiters();
        assert_eq!(next(&mut s).await.unwrap()["running"], false);

        stop.cancel();
        use futures_util::StreamExt;
        assert!(s.next().await.is_none(), "데몬이 꺼지면 끝난다");
    }

    /// 이미 받은 데서 이어 받는다. 받는 쪽이 아는 세대가 지났으면 처음부터 다시 (reset)
    #[tokio::test]
    async fn the_stream_continues_from_where_the_client_is() {
        let changed: &'static Notify = Box::leak(Box::new(Notify::new()));
        let state = Arc::new(Mutex::new(json!({"lines": [{"id": 0, "t": "a"}, {"id": 1, "t": "b"}], "epoch": 1, "running": true})));
        let stop = tokio_util::sync::CancellationToken::new();
        let from = Cursor { since: Some(1), epoch: Some(1), disk_seen: false };
        let mut s = Box::pin(updates(changed, stop.clone(), from, console(state.clone())));
        let got = next(&mut s).await.unwrap();
        assert_eq!((texts(&got), got.get("reset")), (vec!["b"], None));

        let mut s = Box::pin(updates(changed, stop, Cursor { since: Some(5), epoch: Some(0), disk_seen: false }, console(state)));
        let got = next(&mut s).await.unwrap();
        assert_eq!((texts(&got), &got["reset"]), (vec!["a", "b"], &json!(true)));
    }

    #[tokio::test]
    async fn a_missing_console_sends_an_error_and_ends() {
        let changed: &'static Notify = Box::leak(Box::new(Notify::new()));
        let mut s = Box::pin(updates(changed, tokio_util::sync::CancellationToken::new(), Cursor::default(), |_| async {
            Err::<Value, _>(anyhow::anyhow!("no such instance"))
        }));
        assert_eq!(next(&mut s).await.unwrap_err()["error_code"], "not_found");
        use futures_util::StreamExt;
        assert!(s.next().await.is_none());
    }

    #[tokio::test]
    async fn no_wait_answers_as_before() {
        let changed = Notify::new();
        let state = Arc::new(Mutex::new(json!({"lines": []})));
        let got = read_waiting(&changed, Duration::ZERO, None, reader(state)).await.unwrap();
        assert!(got.get("waits").is_none(), "기다리지 않은 답은 예전과 같다");
    }
}
