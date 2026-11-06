//! 시스템 모니터 API
//!
//! - `GET /api/system/metrics` — 기계 전체 CPU·RAM·디스크 IO·디스크 용량·네트워크와 인스턴스별 몫
//!   (형식: `crate::system_monitor::MetricsSnapshot`). 부를 때만 표본을 뜬다.
//!   `?sizes=false`: 인스턴스 폴더 크기를 새로 재지 않는다 (요약만 보는 화면). 포트·서버별 네트워크(`net_trace`)는
//!   설정 `systemMonitorNetTrace`가 켜져 있을 때 `sizes=true`(펼친 화면) 요청이 오는 동안만 센다
//! - `GET /api/system/net-trace` — `{ok, access: "ok"|"needs_grant"|"needs_relogin"|"unsupported", enabled}`
//! - `POST /api/system/net-trace/grant` | `/revoke` — 이 PC에 관리자 확인 창을 띄워 지금 사용자에게
//!   커널 네트워크 이벤트 권한을 주거나 거둔다. 실패는 HTTP 200 `{ok: false, error_code: "net_trace_<코드>", error}`

use axum::{
    extract::{Query, State},
    http::StatusCode,
    response::IntoResponse,
    Json,
};
use serde::Deserialize;
use serde_json::json;
use std::path::PathBuf;

use super::super::IPCServer;
use crate::system_monitor::InstanceRoot;

#[derive(Deserialize)]
pub struct MetricsQuery {
    #[serde(default = "default_sizes")]
    sizes: bool,
}

fn default_sizes() -> bool {
    true
}

/// 표본을 뜰 인스턴스 목록 (프로세스 PID와 폴더)
pub(crate) async fn instance_roots(state: &IPCServer) -> Vec<InstanceRoot> {
    let supervisor = state.supervisor.read().await;
    supervisor
        .instance_store
        .list()
        .iter()
        .map(|inst| {
            let mut dirs = vec![supervisor.instance_store.instance_dir(&inst.id)];
            if let Some(wd) = inst.working_dir.as_deref().filter(|w| !w.is_empty()) {
                dirs.push(PathBuf::from(wd));
            }
            let module = supervisor.module_loader.get_module(&inst.module_name).ok();
            InstanceRoot {
                id: inst.id.clone(),
                name: inst.name.clone(),
                pid: supervisor.tracker.get_pid(&inst.id).ok(),
                dirs,
                ports: crate::port_check::public_ports(inst, module.as_ref().and_then(|m| m.metadata.settings.as_ref())),
            }
        })
        .collect()
}

pub async fn get_metrics(State(state): State<IPCServer>, Query(q): Query<MetricsQuery>) -> impl IntoResponse {
    let roots = instance_roots(&state).await;
    let data_dir = saba_chan_updater_lib::constants::resolve_data_dir();
    let monitor = state.system_monitor.clone();
    monitor.set_net_trace(state.config_store.get_gui_settings().await.system_monitor_net_trace);
    // sysinfo는 동기 시스템 콜이다
    match tokio::task::spawn_blocking(move || monitor.sample(&roots, &[data_dir], q.sizes)).await {
        Ok(snapshot) => (StatusCode::OK, Json(json!(snapshot))).into_response(),
        Err(e) => (StatusCode::INTERNAL_SERVER_ERROR, Json(json!({ "error": e.to_string() }))).into_response(),
    }
}

fn net_trace_status(enabled: bool) -> serde_json::Value {
    json!({ "ok": true, "access": crate::system_monitor::net_trace::access().as_str(), "enabled": enabled })
}

/// GET /api/system/net-trace
pub async fn get_net_trace(State(state): State<IPCServer>) -> impl IntoResponse {
    let enabled = state.config_store.get_gui_settings().await.system_monitor_net_trace;
    match tokio::task::spawn_blocking(move || net_trace_status(enabled)).await {
        Ok(v) => Json(v).into_response(),
        Err(e) => Json(json!({ "ok": false, "error_code": "net_trace_failed", "error": e.to_string() })).into_response(),
    }
}

/// POST /api/system/net-trace/grant
pub async fn grant_net_trace(State(state): State<IPCServer>) -> impl IntoResponse {
    change_net_trace_access(state, true).await
}

/// POST /api/system/net-trace/revoke
pub async fn revoke_net_trace(State(state): State<IPCServer>) -> impl IntoResponse {
    change_net_trace_access(state, false).await
}

/// 관리자 확인 창을 띄우고 끝날 때까지 기다린다 (확인 뒤 하위 명령은 최대 60초)
async fn change_net_trace_access(state: IPCServer, grant: bool) -> axum::response::Response {
    let enabled = state.config_store.get_gui_settings().await.system_monitor_net_trace;
    let done = tokio::task::spawn_blocking(move || {
        crate::system_monitor::net_trace::elevate(grant).map(|()| net_trace_status(enabled))
    })
    .await;
    state.system_monitor.net_trace_retry();
    // 거둔 뒤에도 이미 열린 세션은 계속 센다: 닫아서 다음 요청이 바뀐 권한으로 다시 열게 한다
    if !grant && matches!(done, Ok(Ok(_))) {
        state.system_monitor.stop_net_trace();
    }
    match done {
        Ok(Ok(v)) => Json(v).into_response(),
        Ok(Err(e)) => Json(json!({ "ok": false, "error_code": "net_trace_elevation_failed", "error": e })).into_response(),
        Err(e) => Json(json!({ "ok": false, "error_code": "net_trace_failed", "error": e.to_string() })).into_response(),
    }
}
