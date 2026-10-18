//! 데몬 상태 진단 (A6). 인증 실패·포트 충돌·토큰 불일치를 사용자가 알아볼 수 있게
//! 데이터 경로, 토큰 파일, 익스텐션·모듈 상태, 최근 경고·오류를 한 번에 돌려준다.

use axum::{extract::State, response::IntoResponse, Json};
use serde_json::json;

use super::super::IPCServer;

/// 진단에 싣는 최근 경고·오류 줄 수
const RECENT_PROBLEMS: usize = 20;

static STARTED: std::sync::OnceLock<std::time::Instant> = std::sync::OnceLock::new();

/// IPC 서버가 뜰 때 한 번 부른다 (가동 시간 기준점)
pub fn mark_started() {
    STARTED.get_or_init(std::time::Instant::now);
}

/// GET /api/diagnostics — 인증 필요
pub async fn diagnostics(State(state): State<IPCServer>) -> impl IntoResponse {
    let data_dir = saba_chan_updater_lib::constants::resolve_data_dir();
    let token_file = saba_chan_updater_lib::constants::token_file_path();

    let (enabled, missing) = {
        let ext = state.extension_manager.read().await;
        let mut enabled: Vec<String> = ext.installed_and_enabled_set().into_iter().collect();
        let mut missing = ext.enabled_but_not_installed();
        enabled.sort();
        missing.sort();
        (enabled, missing)
    };
    let modules = state.supervisor.read().await.list_modules().map(|m| m.len());

    let problems = state.daemon_log_buffer.recent_where(RECENT_PROBLEMS, |e| {
        e.level.eq_ignore_ascii_case("warn") || e.level.eq_ignore_ascii_case("error")
    });

    Json(json!({
        "ok": true,
        "version": env!("CARGO_PKG_VERSION"),
        "pid": std::process::id(),
        "uptime_secs": STARTED.get().map(|t| t.elapsed().as_secs()),
        "listen_addr": state.listen_addr,
        "data_dir": data_dir,
        "token_file": { "path": token_file, "exists": token_file.is_file() },
        "audit_log": crate::audit::log().path(),
        "extensions": { "enabled": enabled, "enabled_but_missing": missing },
        "modules": match modules {
            Ok(count) => json!({ "count": count }),
            Err(e) => json!({ "error": e.to_string() }),
        },
        "recent_problems": problems,
    }))
}

#[derive(serde::Deserialize)]
pub struct AuditQuery {
    #[serde(default)]
    pub limit: Option<usize>,
    /// 이 인스턴스 id의 기록만
    #[serde(default)]
    pub instance: Option<String>,
}

/// GET /api/audit?limit=&instance= — 감사 로그 (A10), 최신이 먼저. 기본 100건, 최대 1000건
pub async fn audit(axum::extract::Query(q): axum::extract::Query<AuditQuery>) -> impl IntoResponse {
    let limit = q.limit.unwrap_or(100).clamp(1, 1000);
    let entries = tokio::task::spawn_blocking(move || crate::audit::log().recent(limit, q.instance.as_deref()))
        .await
        .unwrap_or_default();
    Json(json!({ "entries": entries }))
}

/// GET /api/onboarding — 처음 쓰는 사람의 다음 할 일 (J1-a). GUI 첫 화면·CLI·디스코드가 같은 단계를 보인다.
/// 문구는 클라이언트가 단계 `id`로 고른다
pub async fn onboarding(State(state): State<IPCServer>) -> impl IntoResponse {
    let (modules, servers) = {
        let sup = state.supervisor.read().await;
        (sup.list_modules().map(|m| m.len()).unwrap_or(0), sup.instance_store.list().len())
    };
    let paired = state.relay_agent.status().has_token;
    let chatbot = state.config_store.get_chatbot().api_key.is_some_and(|k| !k.trim().is_empty());
    Json(onboarding_steps(modules, servers, paired, chatbot))
}

/// `{steps:[{id, done, optional}], next}`: `next`는 아직 안 한 첫 필수 단계 (다 했으면 null)
fn onboarding_steps(modules: usize, servers: usize, paired: bool, chatbot: bool) -> serde_json::Value {
    let steps = [("module", modules > 0, false), ("server", servers > 0, false), ("discord", paired, false), ("chatbot", chatbot, true)];
    let next = steps.iter().find(|(_, done, optional)| !done && !optional).map(|(id, _, _)| *id);
    json!({
        "steps": steps.iter().map(|(id, done, optional)| json!({ "id": id, "done": done, "optional": optional })).collect::<Vec<_>>(),
        "next": next,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn onboarding_points_at_the_first_required_step_left() {
        assert_eq!(onboarding_steps(0, 0, false, false)["next"], "module");
        assert_eq!(onboarding_steps(2, 0, true, true)["next"], "server");
        assert_eq!(onboarding_steps(2, 1, false, true)["next"], "discord");
        let done = onboarding_steps(2, 1, true, false);
        assert!(done["next"].is_null(), "the chatbot is optional");
        assert_eq!(done["steps"][3], json!({ "id": "chatbot", "done": false, "optional": true }));
    }
}
