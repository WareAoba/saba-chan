//! 명령 ack·결과 보내기와, 받지 않은 결과 다시 보내기

use super::*;

/// 결과를 다시 보내기 전 기다림: 1·2·4·8·16초 (시험에서는 짧게)
pub(super) fn result_retry_delay(attempt: u32) -> Duration {
    let unit = if cfg!(test) { Duration::from_millis(10) } else { Duration::from_secs(1) };
    unit * 2u32.pow(attempt.saturating_sub(1).min(4))
}

impl Session {
    /// 받은 명령을 알린다(`POST /ack`). 릴레이는 ack한 호스트에만 답 없는 명령을 다시 보낸다
    pub(super) async fn post_ack(&self, body: String) {
        match self.send_signed(reqwest::Method::POST, "/ack", Some(body)).await {
            Ok(r) if r.status().is_success() => {}
            Ok(r) => tracing::warn!("[RelayAgent] ack rejected: {}", r.status()),
            Err(e) => tracing::warn!("[RelayAgent] ack failed: {}", e),
        }
    }

    pub(super) async fn post_result(&self, id: &str, result: &Value) {
        // 실행한 명령의 결과는 연결이 끝난 뒤에도 끝까지 보낸다: ack한 명령이라 릴레이가 다시 보내 주지 않는다.
        // 연결 오류·5xx·408·429는 `RESULT_RETRIES`번까지 간격을 늘려 다시 보낸다. 그 밖의 4xx는 릴레이가 받지 않는 것이라 그만둔다
        let body = result.to_string();
        self.agent.posting.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
        let _posting = Posting(self.agent.posting.clone());
        for attempt in 0..=RESULT_RETRIES {
            if attempt > 0 {
                tokio::time::sleep(result_retry_delay(attempt)).await;
            }
            if self.send_result(id, &body, attempt).await {
                return;
            }
        }
        // 명령 자리는 놓고, 결과는 남겨 같은 주소·토큰의 세션이 이어서 보낸다
        tracing::warn!("[RelayAgent] result {} not delivered after {} tries; keeping it", id, RESULT_RETRIES + 1);
        self.keep_unsent(id, body);
    }

    /// 결과를 한 번 보낸다. 다시 보낼 일이 없으면(받았거나, 릴레이가 그 명령을 몰라 받지 않으면) `true`
    async fn send_result(&self, id: &str, body: &str, attempt: u32) -> bool {
        let path = format!("/result/{}", encode_segment(id));
        let sent = match self.request(reqwest::Method::POST, &path, Some(body.to_string())) {
            Ok(req) => req.send().await.map_err(|e| e.to_string()),
            Err(e) => Err(e),
        };
        let status = match sent {
            Ok(r) if r.status().is_success() => return true,
            Ok(r) => r.status(),
            Err(e) => {
                tracing::warn!("[RelayAgent] result {} failed (try {}): {}", id, attempt + 1, e);
                return false;
            }
        };
        let retry = status.is_server_error() || matches!(status.as_u16(), 408 | 429);
        tracing::warn!("[RelayAgent] result {} rejected (try {}): {}", id, attempt + 1, status);
        !retry
    }

    fn unsent(&self) -> std::sync::MutexGuard<'_, Vec<Unsent>> {
        self.agent.unsent.lock().unwrap_or_else(|e| e.into_inner())
    }

    pub(super) fn keep_unsent(&self, id: &str, body: String) {
        let mut unsent = self.unsent();
        if !unsent.iter().any(|u| u.id == id && u.base == self.base && u.token == self.token) {
            unsent.push(Unsent { base: self.base.clone(), token: self.token.clone(), id: id.to_string(), body });
        }
    }

    pub(super) fn has_unsent(&self, id: &str) -> bool {
        self.unsent().iter().any(|u| u.id == id && u.base == self.base && u.token == self.token)
    }

    pub(super) fn unsent_full(&self) -> bool {
        self.unsent().len() >= UNSENT_MAX
    }

    /// 받지 않은 결과를 세션이 끝날 때까지 간격을 두고 다시 보낸다. 명령은 다시 실행하지 않는다.
    /// 다른 주소·토큰(다른 노드)이 만든 결과는 버린다
    pub(super) async fn unsent_loop(&self) {
        while !self.cancel.is_cancelled() {
            self.pause(result_retry_delay(RESULT_RETRIES)).await;
            let due: Vec<(String, String)> = {
                let mut unsent = self.unsent();
                // 끝난 세션은 대기열을 정리하지 않는다: 기다리는 사이 시작한 새 세션(다른 주소·토큰)의 결과를 지우지 않게.
                // 잠근 채 보므로 새 세션은 이 정리가 끝난 뒤에야 결과를 넣는다 (새 세션은 이 세션을 끝낸 뒤 시작한다)
                if self.cancel.is_cancelled() {
                    break;
                }
                unsent.retain(|u| u.base == self.base && u.token == self.token);
                unsent.iter().map(|u| (u.id.clone(), u.body.clone())).collect()
            };
            for (id, body) in due {
                if self.cancel.is_cancelled() {
                    break;
                }
                if !self.send_result(&id, &body, RESULT_RETRIES + 1).await {
                    // 못 보낸 결과는 맨 뒤로 보낸다: 한 결과만 계속 거절돼도 다음 회차엔 뒤 결과부터 보낸다
                    let mut unsent = self.unsent();
                    if let Some(i) = unsent.iter().position(|u| u.id == id && u.base == self.base && u.token == self.token) {
                        let failed = unsent.remove(i);
                        unsent.push(failed);
                    }
                    break;
                }
                self.unsent().retain(|u| u.id != id || u.base != self.base || u.token != self.token);
            }
        }
    }
}
