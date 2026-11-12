//! 콘솔 조회의 `?wait=<초>`: 새 줄이 없으면 바로 빈 답을 주지 않고 줄이 생길 때까지 붙잡는다.
//! 클라이언트는 0.5초마다 빈 요청을 보내지 않고, 답을 받으면 곧바로 다음 요청을 건다

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
pub async fn read_waiting<F, Fut>(changed: &Notify, wait: Duration, known_epoch: Option<u64>, mut read: F) -> anyhow::Result<Value>
where
    F: FnMut() -> Fut,
    Fut: Future<Output = anyhow::Result<Value>>,
{
    let deadline = tokio::time::Instant::now() + wait;
    let mut first: Option<[Value; 4]> = None;
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
        let other_epoch = known_epoch.is_some_and(|e| result.get("epoch").and_then(Value::as_u64) != Some(e));
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

    #[tokio::test]
    async fn no_wait_answers_as_before() {
        let changed = Notify::new();
        let state = Arc::new(Mutex::new(json!({"lines": []})));
        let got = read_waiting(&changed, Duration::ZERO, None, reader(state)).await.unwrap();
        assert!(got.get("waits").is_none(), "기다리지 않은 답은 예전과 같다");
    }
}
