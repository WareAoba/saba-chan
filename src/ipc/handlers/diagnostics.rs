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
        "extensions": { "enabled": enabled, "enabled_but_missing": missing },
        "modules": match modules {
            Ok(count) => json!({ "count": count }),
            Err(e) => json!({ "error": e.to_string() }),
        },
        "recent_problems": problems,
    }))
}
