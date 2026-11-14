//! 디스코드 알림 채널로 보낼 이벤트: 고르기(`relay_event`)·모으기·보내기

use super::*;

impl Session {
    /// 데몬 이벤트 중 알림 채널에 띄울 것(`relay_event`)을 모아 `POST /events`로 올린다
    pub(super) async fn events_loop(&self) {
        use tokio::sync::broadcast::error::{RecvError, TryRecvError};
        let mut rx = crate::events::bus().subscribe(None).rx;
        let mut pending = std::collections::VecDeque::new();
        let mut failures: u32 = 0;
        let mut last_sent: Option<tokio::time::Instant> = None;
        loop {
            if pending.is_empty() {
                // 첫 건을 기다렸다가 잠깐 더 모은다
                let gather = tokio::time::sleep(EVENT_GATHER);
                tokio::pin!(gather);
                let mut gathering = false;
                loop {
                    tokio::select! {
                        _ = self.cancel.cancelled() => return,
                        _ = &mut gather, if gathering => break,
                        r = rx.recv() => match r {
                            Ok(e) => {
                                queue_event(&mut pending, &e);
                                if !gathering && !pending.is_empty() {
                                    gathering = true;
                                    gather.as_mut().reset(tokio::time::Instant::now() + EVENT_GATHER);
                                }
                            }
                            Err(RecvError::Lagged(_)) => {}
                            Err(RecvError::Closed) => return,
                        },
                    }
                }
            }
            if let Some(at) = last_sent.map(|t| t + EVENT_MIN_GAP) {
                tokio::select! {
                    _ = self.cancel.cancelled() => return,
                    _ = tokio::time::sleep_until(at) => {}
                }
            }
            // 다시 보내기 전에 그 사이 쌓인 것도 넣는다
            loop {
                match rx.try_recv() {
                    Ok(e) => queue_event(&mut pending, &e),
                    Err(TryRecvError::Lagged(_)) => {}
                    Err(_) => break,
                }
            }
            let batch: Vec<Value> = pending.iter().take(EVENT_BATCH).cloned().collect();
            let sent = async {
                let body = json!({ "events": batch }).to_string();
                self.send_signed(reqwest::Method::POST, "/events", Some(body)).await
            };
            last_sent = Some(tokio::time::Instant::now());
            let resp = tokio::select! {
                _ = self.cancel.cancelled() => return,
                r = sent => r,
            };
            match resp {
                // 4xx는 다시 보내도 같다: 그 묶음을 버린다 (429는 relay의 노드당 한도라 아래에서 쉬었다 다시 보낸다)
                Ok(r) if r.status().is_success() || (r.status().is_client_error() && r.status().as_u16() != 429) => {
                    if !r.status().is_success() {
                        tracing::warn!("[RelayAgent] events rejected: {}", r.status());
                    }
                    pending.drain(..batch.len());
                    failures = 0;
                }
                other => {
                    failures += 1;
                    let backoff = backoff(failures);
                    match other {
                        Ok(r) => tracing::warn!("[RelayAgent] events {}, retry in {}s", r.status(), backoff.as_secs()),
                        Err(e) => tracing::warn!("[RelayAgent] events failed ({}), retry in {}s", e, backoff.as_secs()),
                    }
                    self.pause(backoff).await;
                    if self.cancel.is_cancelled() {
                        return;
                    }
                }
            }
        }
    }
}

/// 올릴 이벤트면 대기열에 넣는다. 가장 오래된 것부터 버려 `EVENT_PENDING`건만 둔다.
/// 게임 채팅을 먼저 버린다: 채팅이 몰려도 크래시 같은 알림이 밀려나지 않게
pub(super) fn queue_event(pending: &mut std::collections::VecDeque<Value>, event: &crate::events::Event) {
    if let Some(v) = relay_event(event) {
        pending.push_back(v);
        while pending.len() > EVENT_PENDING {
            match pending.iter().position(|v| v["type"] == "instance.chat") {
                Some(i) => drop(pending.remove(i)),
                None => drop(pending.pop_front()),
            }
        }
    }
}

/// 이 이벤트가 오면 다음 하트비트를 기다리지 않고 곧 보낸다 — 서버가 켜지고 꺼진 것은 누가 어디서 시켰든
/// (디스코드·챗봇·GUI·CLI·예약·사바쨩 밖) 데몬이 알아챈 때 릴레이의 서버 목록과 상황판에 닿게 한다
pub(super) fn kicks_heartbeat(kind: &str) -> bool {
    matches!(kind, "instance.started" | "instance.stopped" | "instance.ready" | "instance.exit")
}

/// 알림 채널(I3)에 올릴 모양 `{id, type, data}`. 올리지 않는 종류면 None.
/// 원격으로 나가므로 경로·콘솔 줄·오류 원문은 싣지 않는다. 격리된 익스텐션의 event hook도 이 모양만 받는다 (`extension::event_hooks`)
pub(crate) fn relay_event(event: &crate::events::Event) -> Option<Value> {
    let d = &event.data;
    let pick = |keys: &[&str]| -> serde_json::Map<String, Value> {
        keys.iter().filter_map(|k| d.get(*k).map(|v| (k.to_string(), v.clone()))).collect()
    };
    let data = match event.kind.as_str() {
        "instance.exit" => {
            let kind = d.get("kind").and_then(Value::as_str)?;
            if !["crash", "start_failed", "unknown_exit"].contains(&kind) {
                return None;
            }
            let mut v = pick(&["at", "instanceId", "instance", "kind", "exitCode", "signal", "restartInSecs", "note"]);
            // 종료 코드·신호·메모리 상한만. 콘솔 줄(`console: …`)과 크래시 파일 경로는 뺀다
            if let Some(evidence) = d.get("evidence").and_then(Value::as_str).filter(|e| crate::supervisor::crash::shareable_evidence(e)) {
                v.insert("evidence".into(), Value::from(evidence));
            }
            v
        }
        "backup.done" => pick(&["at", "instanceId", "instance", "kind", "ok", "name"]),
        "update.available" => pick(&["at", "components"]),
        "schedule.warn" => pick(&["at", "instanceId", "instance", "action", "minutesLeft"]),
        "instance.ready" => pick(&["at", "instanceId", "instance", "ok", "code"]),
        // 켜져 있지만 응답이 없다 (응답 없는 서버 감시). 실패 이유 원문은 싣지 않는다
        "instance.unresponsive" => pick(&["at", "instanceId", "instance", "minutes", "restarting"]),
        // 서버가 켜지고 꺼진 것과 시킨 쪽. 사바쨩 밖의 시작은 unexpected (크래시·원인 모를 종료는 instance.exit)
        "instance.started" => pick(&["at", "instanceId", "instance", "source", "userName", "unexpected"]),
        "instance.stopped" => pick(&["at", "instanceId", "instance", "source", "userName", "kind", "idleMinutes"]),
        // 켠 서버의 모듈 새 버전·게임 서버 새 빌드 (시작을 시킨 사람에게만 알린다)
        "instance.update_notice" => pick(&["at", "instanceId", "instance", "kind", "module", "latest"]),
        // 게임 서버 업데이트가 끝났다 (예약·앱·디스코드 어디서 시켰든)
        "instance.updated" => pick(&["at", "instanceId", "instance"]),
        // 자동 업데이트가 접속자 때문에 기다린다 (마지막으로 디스코드에서 켠 사람에게 알린다)
        "instance.update_waiting" => pick(&["at", "instanceId", "instance", "players"]),
        // 게임 서버 업데이트 실패. 오류 원문에는 로컬 경로가 섞이므로 싣지 않는다
        "instance.update_failed" => pick(&["at", "instanceId", "instance"]),
        // 디스코드에서 시작해 뒤에서 마친 여러 단계 실행 (I1). 원격 실행의 단계 줄에는 오류 원문이 없다(코드만)
        "plan.done" if d.get("guildId").is_some() => pick(&["at", "runId", "guildId", "ok", "steps", "stoppedAt"]),
        // 게임 채팅 전달: 사용자가 켠 서버의 채팅만 데몬이 다 만든 문장(`text`)으로 낸다 (`supervisor::game_chat`)
        // 채팅 연동 채널에 보낼 줄(`linkText`)과 그 채널(`links`)도 (`supervisor::chat_link`)
        "instance.chat" => pick(&["at", "instanceId", "instance", "text", "links", "linkText"]),
        // 채팅 연동이 저절로 꺼짐 (모두 나감·서버 꺼짐)
        "instance.chat_link" => pick(&["at", "instanceId", "instance", "guild", "channel", "active", "reason"]),
        _ => return None,
    };
    Some(json!({ "id": event.id, "type": event.kind, "data": data }))
}
