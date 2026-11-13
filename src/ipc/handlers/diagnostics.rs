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
    /// 이 시각(unix 초)부터
    #[serde(default)]
    pub since: Option<u64>,
    /// 이 시각(unix 초) 전까지
    #[serde(default)]
    pub until: Option<u64>,
}

/// GET /api/audit?limit=&instance=&since=&until= — 감사 로그 (A10), 최신이 먼저. 기본 100건, 최대 1000건
pub async fn audit(axum::extract::Query(q): axum::extract::Query<AuditQuery>) -> impl IntoResponse {
    let limit = q.limit.unwrap_or(100).clamp(1, 1000);
    let entries = tokio::task::spawn_blocking(move || {
        crate::audit::log().query(&crate::audit::Filter { limit, instance_id: q.instance.as_deref(), since: q.since, until: q.until })
    })
    .await
    .unwrap_or_default();
    Json(json!({ "entries": entries }))
}

#[derive(serde::Deserialize)]
pub struct AuditExportRequest {
    /// 데몬이 CSV를 쓸 절대 경로
    pub path: String,
    #[serde(default)]
    pub instance: Option<String>,
    #[serde(default)]
    pub since: Option<u64>,
    #[serde(default)]
    pub until: Option<u64>,
}

/// POST /api/audit/export `{path, instance?, since?, until?}` — 기간의 기록을 모두 CSV로 `path`에 쓴다.
/// `{success, path, count}`. 경로가 절대 경로가 아니면 400 `INVALID_PATH`, 쓰지 못하면 500 `WRITE_FAILED`
pub async fn audit_export(Json(req): Json<AuditExportRequest>) -> axum::response::Response {
    use axum::http::StatusCode;
    let fail = |status: StatusCode, code: &str, message: String| {
        (status, Json(json!({ "success": false, "error": message, "error_code": code }))).into_response()
    };
    let path = std::path::PathBuf::from(req.path.trim());
    if req.path.trim().is_empty() || !path.is_absolute() {
        return fail(StatusCode::BAD_REQUEST, "INVALID_PATH", "path must be an absolute file path".into());
    }
    let written = tokio::task::spawn_blocking(move || {
        let entries = crate::audit::log().query(&crate::audit::Filter {
            instance_id: req.instance.as_deref(),
            since: req.since,
            until: req.until,
            ..Default::default()
        });
        std::fs::write(&path, crate::audit::to_csv(&entries)).map(|_| (path, entries.len()))
    })
    .await;
    match written {
        Ok(Ok((path, count))) => Json(json!({ "success": true, "path": path.to_string_lossy(), "count": count })).into_response(),
        Ok(Err(e)) => fail(StatusCode::INTERNAL_SERVER_ERROR, "WRITE_FAILED", format!("Could not write the audit export: {e}")),
        Err(e) => fail(StatusCode::INTERNAL_SERVER_ERROR, "WRITE_FAILED", e.to_string()),
    }
}

/// GET /api/onboarding — 처음 쓰는 사람의 다음 할 일 (J1-a). GUI 첫 화면과 CLI가 같은 단계를 보인다.
/// 문구는 클라이언트가 단계 `id`로 고른다
pub async fn onboarding(State(state): State<IPCServer>) -> impl IntoResponse {
    Json(crate::service::onboarding::now(&state).await)
}
