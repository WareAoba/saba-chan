//! GET /api/events — 데몬 이벤트 스트림 (P4, SSE). 버스는 `crate::events`

use axum::{
    extract::{Query, State},
    http::HeaderMap,
    response::sse::{Event, KeepAlive, Sse},
};
use futures_util::stream::{self, Stream};
use serde::Deserialize;
use std::convert::Infallible;
use tokio::sync::broadcast::error::RecvError;

/// 연결을 살려 두는 주석 줄 간격
const PING: std::time::Duration = std::time::Duration::from_secs(15);

#[derive(Deserialize)]
pub struct EventsQuery {
    since: Option<u64>,
}

fn sse(event: &crate::events::Event) -> Event {
    let out = Event::default().event(&event.kind).data(event.data.to_string());
    // 다시 보내지 않는 건(진행률)은 번호가 없다: 클라이언트의 마지막 번호를 바꾸지 않는다
    if event.replay { out.id(event.id.to_string()) } else { out }
}

fn resync() -> Event {
    Event::default().event("resync").data("{}")
}

/// `Last-Event-ID`(브라우저·EventSource 재연결) 또는 `?since=`. 데몬이 꺼질 때 스트림도 끝난다
/// (열린 연결이 정상 종료를 붙잡지 않게)
pub async fn stream(
    State(state): State<super::super::IPCServer>,
    headers: HeaderMap,
    Query(q): Query<EventsQuery>,
) -> Sse<impl Stream<Item = Result<Event, Infallible>>> {
    let since = headers
        .get("Last-Event-ID")
        .and_then(|v| v.to_str().ok())
        .and_then(|v| v.trim().parse().ok())
        .or(q.since);
    let sub = crate::events::bus().subscribe(since);

    let mut first: Vec<Event> = Vec::with_capacity(sub.backlog.len() + 1);
    if sub.resync {
        first.push(resync());
    }
    first.extend(sub.backlog.iter().map(|e| sse(e)));
    let last = sub.backlog.last().map_or(0, |e| e.id);

    let stop = state.shutdown_token.clone();
    let live = stream::unfold((sub.rx, last, stop), |(mut rx, mut last, stop)| async move {
        loop {
            let next = tokio::select! {
                _ = stop.cancelled() => return None,
                next = rx.recv() => next,
            };
            match next {
                Ok(event) if event.replay && event.id <= last => continue,
                Ok(event) if !event.replay => return Some((Ok(sse(&event)), (rx, last, stop))),
                Ok(event) => {
                    last = event.id;
                    return Some((Ok(sse(&event)), (rx, last, stop)));
                }
                // 뒤처져 놓친 건이 있다: 목록을 다시 읽게 한다
                Err(RecvError::Lagged(_)) => return Some((Ok(resync()), (rx, last, stop))),
                Err(RecvError::Closed) => return None,
            }
        }
    });
    let backlog: Vec<Result<Event, Infallible>> = first.into_iter().map(Ok).collect();
    Sse::new(futures_util::StreamExt::chain(stream::iter(backlog), live)).keep_alive(KeepAlive::new().interval(PING).text("ping"))
}
