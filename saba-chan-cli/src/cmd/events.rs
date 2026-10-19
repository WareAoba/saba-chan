//! `events` — 데몬 이벤트 스트림(`GET /api/events`)을 한 줄씩 흘려보낸다
//!
//! `--json`이면 이벤트마다 JSON 한 줄(NDJSON). AI·스크립트가 설치·재시작이 끝났는지 폴링하지 않고 기다린다.
//! `resync`는 놓친 이벤트가 있다는 뜻이라 거르지 않고 내보낸다: 받은 쪽이 목록을 다시 읽는다.

use crate::client::DaemonEvent;
use crate::ctx::Ctx;
use crate::error::Result;
use crate::output::{self, s};
use serde_json::json;

/// 스트림이 끊겼을 때 다시 붙기 전 기다림
const RETRY: std::time::Duration = std::time::Duration::from_secs(2);

pub async fn run(ctx: &Ctx, kinds: &[String], server: Option<&str>, since: Option<u64>) -> Result<()> {
    let id = match server {
        Some(q) => Some(crate::resolve::server(ctx, Some(q)).await?.id),
        None => None,
    };
    let mut stream = ctx.api.events(since).await?;
    let mut last = since;
    loop {
        match stream.next().await {
            Ok(Some(event)) => {
                last = event.id.or(last);
                if wanted(&event, kinds, id.as_deref()) {
                    say!("{}", line(&event, ctx.out.json));
                }
            }
            // 데몬이 다시 켜지는 중일 수 있다: 마지막 번호로 다시 붙는다
            Ok(None) | Err(_) => loop {
                tokio::time::sleep(RETRY).await;
                if let Ok(fresh) = ctx.api.events(last).await {
                    stream = fresh;
                    break;
                }
            },
        }
    }
}

/// `--kind instance`는 `instance.exit`처럼 그 아래 이벤트를 모두 받는다. 서버를 주면 그 서버 것만
fn wanted(event: &DaemonEvent, kinds: &[String], server: Option<&str>) -> bool {
    if event.kind == "resync" {
        return true;
    }
    let kind_ok = kinds.is_empty()
        || kinds.iter().any(|k| {
            event.kind == *k
                || event
                    .kind
                    .strip_prefix(k.as_str())
                    .is_some_and(|rest| rest.starts_with('.'))
        });
    // `provision.progress`만 `instanceId` 없이 `instance`에 id를 싣는다
    let of = |id: &str| match s(&event.data, "instanceId") {
        "" => s(&event.data, "instance") == id,
        mine => mine == id,
    };
    kind_ok && server.is_none_or(of)
}

fn line(event: &DaemonEvent, json: bool) -> String {
    if json {
        json!({ "id": event.id, "event": event.kind, "data": event.data }).to_string()
    } else {
        format!("{}  {}", output::bold(&event.kind), event.data)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ev(kind: &str, instance: &str) -> DaemonEvent {
        DaemonEvent {
            id: Some(7),
            kind: kind.into(),
            data: json!({ "instanceId": instance }),
        }
    }

    #[test]
    fn kinds_match_whole_names_or_their_children() {
        let kinds = vec!["instance".to_string(), "backup.done".to_string()];
        assert!(wanted(&ev("instance.exit", "a"), &kinds, None));
        assert!(wanted(&ev("backup.done", "a"), &kinds, None));
        assert!(
            !wanted(&ev("instances.x", "a"), &kinds, None),
            "a prefix must end at a dot"
        );
        assert!(!wanted(&ev("plan.done", "a"), &kinds, None));
        assert!(wanted(&ev("plan.done", "a"), &[], None));
        assert!(!wanted(&ev("instance.exit", "b"), &kinds, Some("a")));
        let progress = DaemonEvent {
            id: None,
            kind: "provision.progress".into(),
            data: json!({ "instance": "a", "percent": 3 }),
        };
        assert!(wanted(&progress, &[], Some("a")) && !wanted(&progress, &[], Some("b")));
        // 놓친 이벤트 알림은 거르지 않는다
        assert!(wanted(&ev("resync", ""), &kinds, Some("a")));
    }

    #[test]
    fn json_lines_carry_id_kind_and_data() {
        let v: serde_json::Value = serde_json::from_str(&line(&ev("instance.exit", "a"), true)).unwrap();
        assert_eq!(
            v,
            json!({ "id": 7, "event": "instance.exit", "data": { "instanceId": "a" } })
        );
    }
}
