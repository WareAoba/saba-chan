//! 시스템 모니터 API
//!
//! - `GET /api/system/metrics` — 기계 전체 CPU·RAM·디스크 IO·디스크 용량·네트워크와 인스턴스별 몫
//!   (형식: `crate::system_monitor::MetricsSnapshot`). 부를 때만 표본을 뜬다.

use axum::{extract::State, http::StatusCode, response::IntoResponse, Json};
use serde_json::json;
use std::path::PathBuf;

use super::super::IPCServer;
use crate::system_monitor::InstanceRoot;

pub async fn get_metrics(State(state): State<IPCServer>) -> impl IntoResponse {
    let roots: Vec<InstanceRoot> = {
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
    };
    let data_dir = saba_chan_updater_lib::constants::resolve_data_dir();
    let monitor = state.system_monitor.clone();
    // sysinfo는 동기 시스템 콜이다
    match tokio::task::spawn_blocking(move || monitor.sample(&roots, &[data_dir])).await {
        Ok(snapshot) => (StatusCode::OK, Json(json!(snapshot))).into_response(),
        Err(e) => (StatusCode::INTERNAL_SERVER_ERROR, Json(json!({ "error": e.to_string() }))).into_response(),
    }
}
