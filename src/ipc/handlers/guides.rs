//! 가이드 (`crate::guides`): 목록·완료·new!·보던 단계. 화면 강조는 GUI가 한다

use super::super::IPCServer;
use super::LooseJson;
use axum::{
    extract::{Path, Query, State},
    http::StatusCode,
    response::{IntoResponse, Response},
    Json,
};
use serde::Deserialize;
use serde_json::{json, Value};

#[derive(Debug, Deserialize, Default)]
pub struct ListQuery {
    locale: Option<String>,
}

#[derive(Debug, Deserialize, Default)]
pub struct PatchBody {
    #[serde(default)]
    opened: bool,
    step: Option<usize>,
    #[serde(default)]
    finished: bool,
}

fn state_of(state: &IPCServer) -> crate::guides::State {
    crate::guides::State::from_section(&state.config_store.get_client_section(crate::guides::SECTION).unwrap_or(Value::Null))
}

/// 완료 조건을 판단할 지금 상태
async fn facts(state: &IPCServer) -> crate::guides::Facts {
    let onboarding = super::diagnostics::onboarding_now(state).await;
    let step_done = |id: &str| {
        onboarding["steps"].as_array().is_some_and(|a| a.iter().any(|s| s["id"] == id && s["done"] == true))
    };
    let sup = state.supervisor.read().await;
    let instances = sup.instance_store.list();
    let dirs: Vec<_> = instances.iter().map(|i| crate::backup::backups_dir(&sup.instance_store.instance_dir(&i.id))).collect();
    let backup_any = tokio::task::spawn_blocking(move || {
        dirs.iter().any(|d| {
            std::fs::read_dir(d).into_iter().flatten().flatten().any(|e| e.path().extension().is_some_and(|x| x == "zip"))
        })
    })
    .await
    .unwrap_or(false);
    crate::guides::Facts {
        paired: step_done("discord"),
        modules: sup.list_modules().map(|m| m.len()).unwrap_or(0),
        servers: instances.len(),
        started: step_done("start"),
        chatbot_key: step_done("chatbot"),
        backup_any,
        schedule_any: instances.iter().any(|i| !i.schedule.is_empty()),
        game_chat_on: instances.iter().any(|i| !i.game_chat.is_off()),
        onboarding,
    }
}

/// 서버에서 가이드를 다시 받는 간격 (목록을 볼 때 이만큼 지났으면 뒤에서 받는다)
const SYNC_EVERY: std::time::Duration = std::time::Duration::from_secs(6 * 60 * 60);
/// 받기에 실패했으면 이만큼 뒤에 다시 받는다
const RETRY_AFTER: std::time::Duration = std::time::Duration::from_secs(10 * 60);
/// 받는 응답 크기 상한
const SYNC_MAX_BYTES: usize = 2 * 1024 * 1024;
/// 다음에 받아도 되는 때 (받는 중에는 SYNC_EVERY 뒤로 잡아 겹쳐 받지 않는다)
static NEXT_SYNC: std::sync::Mutex<Option<std::time::Instant>> = std::sync::Mutex::new(None);

/// 받을 차례면 다음 차례를 SYNC_EVERY 뒤로 잡고 true
fn claim_sync(next: &std::sync::Mutex<Option<std::time::Instant>>, now: std::time::Instant) -> bool {
    let mut next = next.lock().unwrap_or_else(|e| e.into_inner());
    if next.is_some_and(|t| now < t) {
        return false;
    }
    *next = Some(now + SYNC_EVERY);
    true
}

/// 받기에 실패하면 RETRY_AFTER 뒤에 다시
fn sync_failed(next: &std::sync::Mutex<Option<std::time::Instant>>, now: std::time::Instant) {
    *next.lock().unwrap_or_else(|e| e.into_inner()) = Some(now + RETRY_AFTER);
}

/// 응답을 읽으면서 상한을 넘으면 끊는다
async fn read_capped(mut res: reqwest::Response, max: usize) -> Option<Vec<u8>> {
    if res.content_length().is_some_and(|n| n > max as u64) {
        return None;
    }
    let mut buf = Vec::new();
    while let Some(chunk) = res.chunk().await.ok()? {
        if buf.len() + chunk.len() > max {
            return None;
        }
        buf.extend_from_slice(&chunk);
    }
    Some(buf)
}

/// 앱을 낸 뒤 생긴 가이드를 릴레이(`GET /v1/guides`)에서 받아 받은 가이드 폴더를 맞춘다. 실패하면 있던 것을 그대로 두고 조금 뒤 다시 받는다
fn spawn_sync(state: &IPCServer) {
    if !claim_sync(&NEXT_SYNC, std::time::Instant::now()) {
        return;
    }
    let state = state.clone();
    tokio::spawn(async move {
        #[derive(Deserialize)]
        struct Body {
            guides: Vec<crate::guides::Served>,
        }
        let base = super::chatbot::relay_base(&state).await;
        let failed = || sync_failed(&NEXT_SYNC, std::time::Instant::now());
        let Ok(http) = super::relay::client() else { return failed() };
        let url = format!("{}/v1/guides?core={}", base, env!("CARGO_PKG_VERSION"));
        let res = match http.get(url).timeout(std::time::Duration::from_secs(15)).send().await {
            Ok(r) if r.status().is_success() => r,
            // 옛 릴레이(404)·끊김: 조금 뒤 다시
            _ => return failed(),
        };
        let Some(bytes) = read_capped(res, SYNC_MAX_BYTES).await else {
            tracing::warn!("Served guides are too large or were cut off; keeping the current ones");
            return failed();
        };
        let Ok(body) = serde_json::from_slice::<Body>(&bytes) else {
            tracing::warn!("Could not read the served guides; keeping the current ones");
            return failed();
        };
        let dir = crate::guides::installed_dir();
        match tokio::task::spawn_blocking(move || crate::guides::install(&dir, &body.guides)).await {
            Ok(Ok(n)) if n > 0 => {
                tracing::info!("Guides changed: {} added, updated or removed", n);
                // 열린 앱이 목록을 다시 읽게 (웹 원격에는 보내지 않는 종류)
                crate::events::emit("guides.updated", json!({ "count": n }));
            }
            Ok(Err(e)) => tracing::warn!("Could not save the served guides: {}", e),
            _ => {}
        }
    });
}

/// GET /api/guides?locale — 가이드 목록 (문구는 그 언어, 없으면 영어). 받은 가이드는 6시간마다 뒤에서 다시 받는다 (실패하면 10분 뒤)
pub async fn list(State(state): State<IPCServer>, Query(q): Query<ListQuery>) -> Response {
    spawn_sync(&state);
    let lang = super::actions::locale(&state, q.locale).await;
    let lang = saba_chan_updater_lib::constants::resolve_locale(&lang).unwrap_or_else(|| "en".into());
    let guides = crate::guides::load(Some(&crate::guides::installed_dir()));
    let facts = facts(&state).await;
    let (body, remember) = crate::guides::view(&guides, &state_of(&state), &facts, &lang);
    if let Some(known) = remember {
        // 처음 본 때 있던 가이드는 new!가 아니다. 남기지 못하면 다음에 다시 남긴다
        if let Err(e) = state.config_store.merge_client_section(crate::guides::SECTION, json!({ "known": known })) {
            tracing::warn!("Could not remember the guides seen so far: {}", e);
        }
    }
    Json(body).into_response()
}

/// POST /api/guides/:id `{opened?, step?, finished?}` — 열었음(new! 끄기)·보던 단계·끝까지 봄
pub async fn update(State(state): State<IPCServer>, Path(id): Path<String>, LooseJson(body): LooseJson<PatchBody>) -> Response {
    let body = body.map(|b| b.0).unwrap_or_default();
    if !crate::guides::load(Some(&crate::guides::installed_dir())).iter().any(|g| g.id == id) {
        return (StatusCode::NOT_FOUND, Json(json!({ "ok": false, "error": "unknown guide", "error_code": "not_found" }))).into_response();
    }
    let patch = crate::guides::patch(&state_of(&state), &id, body.opened, body.step, body.finished);
    if patch.as_object().is_some_and(|o| o.is_empty()) {
        return Json(json!({ "ok": true })).into_response();
    }
    match state.config_store.merge_client_section(crate::guides::SECTION, patch) {
        Ok(_) => Json(json!({ "ok": true })).into_response(),
        Err(e) => (StatusCode::INTERNAL_SERVER_ERROR, Json(json!({ "ok": false, "error": e.to_string() }))).into_response(),
    }
}

#[cfg(test)]
mod sync_tests {
    use super::*;
    use std::time::{Duration, Instant};

    #[test]
    fn a_failed_sync_retries_soon_not_after_six_hours() {
        let next = std::sync::Mutex::new(None);
        let t0 = Instant::now();
        assert!(claim_sync(&next, t0));
        // 받는 중이거나 막 받았으면 다시 받지 않는다
        assert!(!claim_sync(&next, t0 + Duration::from_secs(60)));
        sync_failed(&next, t0);
        assert!(!claim_sync(&next, t0 + RETRY_AFTER - Duration::from_secs(1)));
        assert!(claim_sync(&next, t0 + RETRY_AFTER));
    }
}
