//! Runtime preflight — the daemon's "self-healing install" pass.
//!
//! ## Phases
//!
//! 1. `python`   — ensure portable Python venv is healthy.
//! 2. `done`     — preflight is complete, daemon is ready to serve traffic.
//!    `failed`   — the Python bootstrap failed; `error` carries the reason.
//!
//! Each phase updates a [`PreflightTracker`] with `phase`, `message`,
//! `percent`, and `error` so other components can subscribe.

use std::sync::Arc;

use tokio::sync::RwLock;

/// Phase identifier reported to the GUI. Matches the step strings the
/// renderer uses for progress mapping (`saba-chan-gui/src/App.js`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Phase {
    /// Preflight has not started yet.
    Idle,
    /// Checking / setting up portable Python venv.
    Python,
    /// All checks passed. Daemon is ready.
    Done,
    /// Preflight failed. `error` contains a message.
    Failed,
}

impl Phase {
    /// Maps phase to the step string the GUI consumes.
    pub fn as_step(&self) -> &'static str {
        match self {
            Self::Idle => "preflight-idle",
            Self::Python => "preflight-python",
            Self::Done => "preflight-done",
            Self::Failed => "preflight-failed",
        }
    }

    /// Coarse percent (0-100) for the GUI's preparing ring.
    pub fn base_percent(&self) -> u8 {
        match self {
            Self::Idle => 0,
            Self::Python => 55,
            Self::Done => 100,
            Self::Failed => 0,
        }
    }
}

/// Snapshot of the preflight state — the JSON shape the
/// `/api/runtime/preflight` endpoint returns.
#[derive(Debug, Clone, serde::Serialize)]
pub struct PreflightSnapshot {
    pub phase: String,
    pub message: String,
    pub percent: u8,
    pub complete: bool,
    pub error: Option<String>,
    /// Per-component status keyed by `python`.
    /// Each value is `pending | running | ok | failed`.
    pub components: serde_json::Value,
}

#[derive(Debug)]
struct PreflightInner {
    phase: Phase,
    message: String,
    percent: u8,
    error: Option<String>,
    /// Component → status map. Key: "python".
    components: std::collections::BTreeMap<String, String>,
}

impl PreflightInner {
    fn new() -> Self {
        let mut components = std::collections::BTreeMap::new();
        components.insert("python".to_string(), "pending".to_string());
        Self {
            phase: Phase::Idle,
            message: "Waiting to start...".into(),
            percent: 0,
            error: None,
            components,
        }
    }

    fn snapshot(&self) -> PreflightSnapshot {
        let components: serde_json::Map<String, serde_json::Value> = self
            .components
            .iter()
            .map(|(k, v)| (k.clone(), serde_json::Value::String(v.clone())))
            .collect();
        PreflightSnapshot {
            phase: self.phase.as_step().to_string(),
            message: self.message.clone(),
            percent: self.percent,
            // ★ "complete" means the preflight reached a terminal state
            //   (success OR failure). Callers check `error` to distinguish.
            //   Done-only would force the GUI to re-implement the
            //   condition; this keeps it simple.
            complete: matches!(self.phase, Phase::Done | Phase::Failed),
            error: self.error.clone(),
            components: serde_json::Value::Object(components),
        }
    }
}

/// Shared, cheap-to-clone handle to preflight progress.
///
/// All mutators are async because we expose the inner state behind a
/// `tokio::RwLock` (consistent with other daemon-side trackers like
/// `ExtensionInitTracker`).
#[derive(Clone)]
pub struct PreflightTracker {
    inner: Arc<RwLock<PreflightInner>>,
}

impl Default for PreflightTracker {
    fn default() -> Self {
        Self::new()
    }
}

impl PreflightTracker {
    pub fn new() -> Self {
        Self {
            inner: Arc::new(RwLock::new(PreflightInner::new())),
        }
    }

    /// Atomic snapshot for the `/api/runtime/preflight` endpoint.
    pub async fn snapshot(&self) -> PreflightSnapshot {
        self.inner.read().await.snapshot()
    }

    /// Move to a new phase with that phase's base percent.
    pub async fn set_phase(&self, phase: Phase, message: impl Into<String>) {
        let mut s = self.inner.write().await;
        s.phase = phase;
        s.message = message.into();
        s.percent = phase.base_percent();
    }

    /// Move to a new phase with an explicit percent (overrides the
    /// phase's base — useful for finer-grained progress within a phase).
    pub async fn set_phase_with_pct(
        &self,
        phase: Phase,
        message: impl Into<String>,
        percent: u8,
    ) {
        let mut s = self.inner.write().await;
        s.phase = phase;
        s.message = message.into();
        s.percent = percent.min(100);
    }

    /// Mark a component as `ok | failed | running | pending`.
    pub async fn set_component(&self, key: &str, status: &str) {
        let mut s = self.inner.write().await;
        s.components.insert(key.to_string(), status.to_string());
    }

    /// Terminal: success.
    pub async fn finish_ok(&self) {
        let mut s = self.inner.write().await;
        s.phase = Phase::Done;
        s.percent = 100;
        s.message = "Ready".into();
        s.error = None;
    }

    /// Terminal: failure. Sets error.
    pub async fn finish_failed(&self, err: impl Into<String>) {
        let mut s = self.inner.write().await;
        s.phase = Phase::Failed;
        s.error = Some(err.into());
    }
}

// ═══════════════════════════════════════════════════════════════
//  Phase implementations
// ═══════════════════════════════════════════════════════════════

/// Result of a phase: ok or with details about the failure.
#[derive(Debug)]
pub enum PhaseOutcome {
    Ok,
    Failed(String),
}

/// Phase 2: Python venv.
pub async fn phase_python(tracker: &PreflightTracker) -> PhaseOutcome {
    tracker
        .set_phase(Phase::Python, "Checking Python environment...")
        .await;
    tracker.set_component("python", "running").await;
    match crate::python_env::get_python_path().await {
        Ok(path) => {
            tracing::info!("[preflight] Python venv OK: {}", path.display());
            tracker.set_component("python", "ok").await;
            PhaseOutcome::Ok
        }
        Err(e) => {
            tracker.set_component("python", "failed").await;
            // 데몬 자체는 계속 동작하지만 모듈/익스텐션은 Python 없이 실행할 수 없다.
            tracing::warn!("[preflight] Python bootstrap failed: {}", e);
            PhaseOutcome::Failed(format!("Python bootstrap failed: {}", e))
        }
    }
}

// ═══════════════════════════════════════════════════════════════
//  Orchestrator
// ═══════════════════════════════════════════════════════════════

/// 데몬 기동 시 백그라운드 태스크로 실행한다.
///
/// 실패하면 `preflight-failed` + `error`로 종료해 GUI가 원인을 표시할 수 있게 한다.
pub async fn run(tracker: PreflightTracker) {
    tracing::info!("[preflight] starting");
    match phase_python(&tracker).await {
        PhaseOutcome::Ok => tracker.finish_ok().await,
        PhaseOutcome::Failed(e) => tracker.finish_failed(e).await,
    }
    tracing::info!("[preflight] done");
}

// ═══════════════════════════════════════════════════════════════
//  Tests
// ═══════════════════════════════════════════════════════════════

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn tracker_starts_in_idle_phase() {
        let t = PreflightTracker::new();
        let snap = t.snapshot().await;
        assert_eq!(snap.phase, "preflight-idle");
        assert_eq!(snap.percent, 0);
        assert!(!snap.complete);
        assert!(snap.error.is_none());
    }

    #[tokio::test]
    async fn tracker_set_phase_updates_percent_to_base() {
        let t = PreflightTracker::new();
        t.set_phase(Phase::Python, "checking").await;
        let snap = t.snapshot().await;
        assert_eq!(snap.phase, "preflight-python");
        assert_eq!(snap.message, "checking");
        assert_eq!(snap.percent, Phase::Python.base_percent());
    }

    #[tokio::test]
    async fn tracker_set_phase_with_pct_clamps_to_100() {
        let t = PreflightTracker::new();
        t.set_phase_with_pct(Phase::Python, "x", 200).await;
        assert_eq!(t.snapshot().await.percent, 100);
    }

    #[tokio::test]
    async fn tracker_components_default_pending() {
        let t = PreflightTracker::new();
        let snap = t.snapshot().await;
        for k in ["python"] {
            assert_eq!(
                snap.components.get(k).and_then(|v| v.as_str()),
                Some("pending"),
                "{} default",
                k
            );
        }
    }

    #[tokio::test]
    async fn tracker_set_component_updates_status() {
        let t = PreflightTracker::new();
        t.set_component("python", "failed").await;
        let snap = t.snapshot().await;
        assert_eq!(snap.components["python"], "failed");
        assert!(snap.components.get("node").is_none(), "Node 런타임은 더 이상 관리하지 않는다");
    }

    #[tokio::test]
    async fn tracker_finish_ok_sets_terminal_state() {
        let t = PreflightTracker::new();
        t.finish_ok().await;
        let snap = t.snapshot().await;
        assert_eq!(snap.phase, "preflight-done");
        assert_eq!(snap.percent, 100);
        assert!(snap.complete);
        assert!(snap.error.is_none());
    }

    #[tokio::test]
    async fn tracker_finish_failed_sets_error_and_completes() {
        let t = PreflightTracker::new();
        t.finish_failed("disk full").await;
        let snap = t.snapshot().await;
        assert_eq!(snap.phase, "preflight-failed");
        assert_eq!(snap.error.as_deref(), Some("disk full"));
        // ★ Failed is a terminal state — `complete` must be true so the
        //   GUI's wait-loop exits and surfaces the error to the user.
        assert!(snap.complete);
    }

    #[test]
    fn phase_step_strings_match_gui_expectations() {
        // ★ The renderer (saba-chan-gui/src/App.js) maps these exact
        //   strings to progress messages. Renaming any of them silently
        //   breaks the loading ring — guard with a hard assertion.
        assert_eq!(Phase::Idle.as_step(), "preflight-idle");
        assert_eq!(Phase::Python.as_step(), "preflight-python");
        assert_eq!(Phase::Done.as_step(), "preflight-done");
        assert_eq!(Phase::Failed.as_step(), "preflight-failed");
    }

    #[test]
    fn phase_base_percents_are_monotonic() {
        // The GUI renders a progress bar that should never go backwards.
        assert!(Phase::Python.base_percent() <= Phase::Done.base_percent());
    }
}
