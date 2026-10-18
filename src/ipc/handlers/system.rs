//! 시스템 모니터 API
//!
//! - `GET /api/system/metrics` — 기계 전체 CPU·RAM·디스크 IO·디스크 용량·네트워크와 인스턴스별 몫
//!   (형식: `crate::system_monitor::MetricsSnapshot`). 부를 때만 표본을 뜬다.
//!   `?sizes=false`: 인스턴스 폴더 크기를 새로 재지 않는다 (요약만 보는 화면)

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
            InstanceRoot {
                id: inst.id.clone(),
                name: inst.name.clone(),
                pid: supervisor.tracker.get_pid(&inst.id).ok(),
                dirs,
            }
        })
        .collect()
}

pub async fn get_metrics(State(state): State<IPCServer>, Query(q): Query<MetricsQuery>) -> impl IntoResponse {
    let roots = instance_roots(&state).await;
    let data_dir = saba_chan_updater_lib::constants::resolve_data_dir();
    let monitor = state.system_monitor.clone();
    // sysinfo는 동기 시스템 콜이다
    match tokio::task::spawn_blocking(move || monitor.sample(&roots, &[data_dir], q.sizes)).await {
        Ok(snapshot) => (StatusCode::OK, Json(json!(snapshot))).into_response(),
        Err(e) => (StatusCode::INTERNAL_SERVER_ERROR, Json(json!({ "error": e.to_string() }))).into_response(),
    }
}
