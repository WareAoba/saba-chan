//! Handler for the daemon's runtime preflight state.
//!
//! GET `/api/runtime/preflight` returns the current snapshot the GUI's
//! preparing-ring renderer reads. Polled every ~500 ms by Electron main
//! while the daemon is booting.

use axum::extract::State;
use axum::response::IntoResponse;

use crate::ipc::IPCServer;

/// GET /api/runtime/preflight — current preflight progress.
pub async fn preflight_status(State(srv): State<IPCServer>) -> impl IntoResponse {
    let snap = srv.preflight_tracker.snapshot().await;
    axum::Json(snap)
}
