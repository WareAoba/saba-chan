//! 코어 업데이트의 단계(상단 바 알약)와 자동 적용 판정 — `GET /api/updates/status`의 `phase`
//!
//! - `idle` → (받을 앱 업데이트가 있고 막는 것이 있음) `waiting` → `applying`(업데이터를 띄움) → 새 데몬이 완료 마커를 읽어
//!   `done`(성공) 또는 `ready`(되돌림·미룸). 자동 적용이 안 되는 경우(GUI 창이 열려 있음, 실패한 판)도 `ready`다
//! - 모듈·익스텐션·번역 자동 교체는 단계를 바꾸지 않는다
//! - 막는 것은 한 곳(`daemon_work` + `Supervisor::core_swap_blockers`)에서 모은다
//!
//! 상태(마지막으로 본 릴리즈 id, 단계, 읽은 완료 마커)는 `<staging>/live-update.json`에 둔다

use crate::supervisor::core_swap::{CoreSwapBlocker, CoreSwapBlockerKind};
use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, HashMap};
use std::path::{Path, PathBuf};

/// `done`이 스스로 `idle`이 되기까지
pub const DONE_LINGERS: chrono::Duration = chrono::Duration::hours(24);

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PhaseState {
    #[default]
    Idle,
    Waiting,
    Applying,
    Done,
    Ready,
}

/// `ready`인 까닭
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ReadyReason {
    RolledBack,
    GuiWindowOpen,
    WaitPidTimeout,
    FailedRelease,
    /// 자동 적용을 끈 설정(`auto_apply=false`): 사용자가 다시 시작을 누를 때 적용한다
    Manual,
}

/// 알약이 보이는 단계 (`GET /api/updates/status`의 `phase`)
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct Phase {
    pub state: PhaseState,
    pub version: Option<String>,
    /// 이 단계가 된 때 (RFC 3339)
    pub since: Option<String>,
    #[serde(default)]
    pub waiting_for: Vec<CoreSwapBlocker>,
    pub reason: Option<ReadyReason>,
    /// 언어 코드 → 패치노트
    pub notes: Option<HashMap<String, String>>,
}

impl Phase {
    pub fn new(state: PhaseState, version: Option<String>, notes: Option<HashMap<String, String>>) -> Self {
        Self { state, version, since: Some(chrono::Utc::now().to_rfc3339()), waiting_for: Vec::new(), reason: None, notes }
    }

    pub fn ready(reason: ReadyReason, version: Option<String>, notes: Option<HashMap<String, String>>) -> Self {
        Self { reason: Some(reason), ..Self::new(PhaseState::Ready, version, notes) }
    }

    /// 지금 보일 단계: `done`은 하루가 지나면 `idle`
    pub fn visible(&self, now: chrono::DateTime<chrono::Utc>) -> Phase {
        let expired = self.state == PhaseState::Done
            && self
                .since
                .as_deref()
                .and_then(|s| chrono::DateTime::parse_from_rfc3339(s).ok())
                .is_none_or(|since| now - since.with_timezone(&chrono::Utc) >= DONE_LINGERS);
        if expired { Phase::default() } else { self.clone() }
    }
}

/// `<staging>/live-update.json`
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct LiveState {
    /// heartbeat `latestReleases` 중 이 데몬이 받는 채널의 값 (마지막으로 본 것)
    pub seen: BTreeMap<String, u64>,
    pub phase: Phase,
    /// 단계를 정할 때 이미 읽은 완료 마커의 `timestamp` (같은 마커로 단계를 다시 바꾸지 않는다)
    pub marker_seen: Option<String>,
}

impl LiveState {
    pub fn path() -> PathBuf {
        #[cfg(not(test))]
        return saba_chan_updater_lib::constants::resolve_staging_dir().join("live-update.json");
        // 시험은 실제 상태 파일을 건드리지 않는다
        #[cfg(test)]
        return std::env::temp_dir().join(format!("saba-live-update-test-{}.json", std::process::id()));
    }

    pub fn load_from(path: &Path) -> Self {
        std::fs::read_to_string(path).ok().and_then(|t| serde_json::from_str(&t).ok()).unwrap_or_default()
    }

    pub fn save_to(&self, path: &Path) {
        let saved = serde_json::to_vec_pretty(self)
            .map_err(std::io::Error::other)
            .and_then(|bytes| {
                if let Some(dir) = path.parent() {
                    std::fs::create_dir_all(dir)?;
                }
                saba_atomic_file::write(path, &bytes, saba_atomic_file::Mode::Plain)
            });
        if let Err(e) = saved {
            tracing::warn!("[Updates] Could not save {}: {}", path.display(), e);
        }
    }
}

/// 새 데몬이 뜬 뒤 완료 마커(업데이터가 남긴다)로 정하는 단계. 앱 구성요소를 바꾼 결과만 본다
/// (모듈·익스텐션만 바꾼 데몬의 기록은 단계를 바꾸지 않는다). 이미 읽은 마커면 None
pub fn phase_from_marker(marker: &serde_json::Value, marker_seen: Option<&str>) -> Option<Phase> {
    let stamp = marker.get("timestamp").and_then(serde_json::Value::as_str);
    if stamp.is_some() && stamp == marker_seen {
        return None;
    }
    let release = marker.get("release");
    // 보이는 판은 릴리즈 판이다. 되돌림 마커의 `version`은 되돌린 코어 판이라 릴리즈 요약이 없을 때만 쓴다
    let version = release
        .and_then(|r| r.get("version"))
        .and_then(serde_json::Value::as_str)
        .or_else(|| marker.get("version").and_then(serde_json::Value::as_str))
        .map(str::to_string);
    let notes: Option<HashMap<String, String>> =
        release.and_then(|r| r.get("notes")).and_then(|n| serde_json::from_value(n.clone()).ok()).filter(|n: &HashMap<_, _>| !n.is_empty());
    match marker.get("result").and_then(serde_json::Value::as_str) {
        Some("rolled_back") => return Some(Phase::ready(ReadyReason::RolledBack, version, notes)),
        // 파일을 바꾸지 않았다(이전 데몬이 끝나지 않음 `wait_pid_timeout`, 데이터 스냅샷 실패 `snapshot_failed`): 사용자가 다시 시작할 때까지 둔다
        Some("deferred") => return Some(Phase::ready(ReadyReason::WaitPidTimeout, version, notes)),
        // 사용자가 직전 버전으로 되돌렸다: 떠난 판의 완료·준비됨 알약은 걷는다 (되돌림 알림은 `last-result`가 한다)
        Some("reverted") => return Some(Phase::default()),
        _ => {}
    }
    let success = marker.get("success").and_then(serde_json::Value::as_bool) == Some(true);
    let app = marker
        .get("updated_components")
        .and_then(serde_json::Value::as_array)
        .is_some_and(|keys| keys.iter().filter_map(serde_json::Value::as_str).any(is_app_component));
    (success && app).then(|| Phase::new(PhaseState::Done, version, notes))
}

/// 데몬이 뜰 때: 새 완료 마커가 있으면 그것으로 단계를 정한다. 없으면 이전 데몬이 남긴 `waiting`·`applying`은 걷는다
/// (기다림은 다시 판정하고, 업데이터가 마커를 남기지 못했으면 이어서 보일 것이 없다). 바뀌었으면 true
pub fn adopt_marker(live: &mut LiveState, marker: Option<&serde_json::Value>) -> bool {
    if let Some(marker) = marker {
        if let Some(phase) = phase_from_marker(marker, live.marker_seen.as_deref()) {
            live.marker_seen = marker.get("timestamp").and_then(serde_json::Value::as_str).map(str::to_string);
            live.phase = phase;
            return true;
        }
    }
    if matches!(live.phase.state, PhaseState::Waiting | PhaseState::Applying) {
        live.phase = Phase::default();
        return true;
    }
    false
}

/// 데몬이 뜰 때 완료 마커를 읽는다(`adopt_marker`). 미룬 단계에 판이 없으면 받아 둔 릴리즈 판(`pending_release`)을 채운다
/// (같은 묶음을 다시 띄우지 않는 기준). 바뀌었는가와, 이 마커로 되돌림을 알았으면 되돌린 코어 판(마커의 `version`)을 준다
pub fn adopt_startup_marker(
    live: &mut LiveState,
    marker: Option<&serde_json::Value>,
    pending_release: impl FnOnce() -> Option<String>,
) -> (bool, Option<String>) {
    if !adopt_marker(live, marker) {
        return (false, None);
    }
    if live.phase.reason == Some(ReadyReason::WaitPidTimeout) && live.phase.version.is_none() {
        live.phase.version = pending_release();
    }
    let rolled_back = (live.phase.reason == Some(ReadyReason::RolledBack))
        .then(|| marker.and_then(|m| m.get("version")).and_then(serde_json::Value::as_str).map(str::to_string))
        .flatten();
    (true, rolled_back)
}

/// 앱 구성요소 (모듈·익스텐션·번역이 아닌 것): 이것이 바뀌어야 알약이 움직인다
pub fn is_app_component(key: &str) -> bool {
    !(key.starts_with("module-") || key.starts_with("ext-") || key == "locales")
}

/// 바꾸려면 데몬이 내려가야 하는 구성요소 (`apply_updates_with`가 업데이터에 넘기는 것)
pub fn needs_restart(key: &str) -> bool {
    matches!(key, "saba-core" | "gui" | "updater")
}

/// 자동 적용 판정의 입력
pub struct PlanInput<'a> {
    /// 받아 둔 구성요소 키 (앱 구성요소만 본다)
    pub pending: &'a [String],
    /// 적용할 릴리즈 판
    pub version: Option<&'a str>,
    /// 받아 둔 묶음의 코어(`saba-core`) 판. 묶음에 코어가 없으면 None
    pub core_version: Option<&'a str>,
    /// 되돌린 코어 판 (`UpdateConfig::failed_release`, 설치기가 적는다)
    pub failed_release: Option<&'a str>,
    /// 사용자가 고른 되돌리기다 (`UpdateConfig::user_held`): 그 코어는 준비됨으로도 알리지 않는다
    pub user_held: bool,
    /// 이전 데몬이 끝나지 않아 미룬 릴리즈 판 (단계 `ready`/`wait_pid_timeout`의 판)
    pub deferred_release: Option<&'a str>,
    pub gui_window_open: bool,
    pub cli_running: bool,
    /// 데몬 작업 + 켜진 서버 (`daemon_work`, `core_swap_blockers`)
    pub blockers: Vec<CoreSwapBlocker>,
}

#[derive(Debug, Clone, PartialEq)]
pub enum Decision {
    /// 자동으로 적용할 앱 업데이트가 없다
    Nothing,
    Ready(ReadyReason),
    /// 사용자가 되돌려 고정한 코어: 적용하지 않고 준비됨으로도 두지 않는다 (더 새 코어가 오면 고정이 풀린다)
    Held,
    Wait(Vec<CoreSwapBlocker>),
    /// 이 키들을 적용한다 (켜진 CLI TUI가 있으면 `cli`는 다음 묶음으로 남는다)
    Apply(Vec<String>),
}

/// 받아 둔 앱 업데이트를 묻지 않고 적용할 수 있는가
pub fn decide(input: PlanInput<'_>) -> Decision {
    let mut targets: Vec<String> = input.pending.iter().filter(|k| is_app_component(k)).cloned().collect();
    if targets.is_empty() {
        return Decision::Nothing;
    }
    // 되돌린 코어는 다시 자동으로 적용하지 않는다 (13절): 사용자가 다시 시작을 누를 때까지.
    // 사용자가 고른 되돌리기는 고정이라 준비됨으로도 알리지 않는다
    if input.user_held {
        if is_held_core(input.failed_release, input.core_version) {
            return Decision::Held;
        }
    } else if is_rolled_back_core(input.failed_release, input.core_version) {
        return Decision::Ready(ReadyReason::FailedRelease);
    }
    // 이전 데몬이 끝나지 않아 미룬 묶음도 사용자가 다시 시작하거나 더 새 판이 올 때까지 다시 띄우지 않는다
    if input.deferred_release.is_some() && input.deferred_release == input.version {
        return Decision::Ready(ReadyReason::WaitPidTimeout);
    }
    // GUI 창이 떠 있는 채로 GUI를 바꾸면 창이 닫힌다: 사용자가 고를 때까지 묶음 전체를 둔다
    if input.gui_window_open && targets.iter().any(|k| k == "gui") {
        return Decision::Ready(ReadyReason::GuiWindowOpen);
    }
    if input.cli_running {
        targets.retain(|k| k != "cli");
        if targets.is_empty() {
            return Decision::Nothing;
        }
    }
    if targets.iter().any(|k| needs_restart(k)) && !input.blockers.is_empty() {
        return Decision::Wait(input.blockers);
    }
    Decision::Apply(targets)
}

/// 받을 코어(`core_version`)가 되돌린 코어(`failed_release`)인가.
/// 부분 릴리즈는 안 바뀐 코어를 지난 판 번호 그대로 싣으므로 릴리즈 판이 아니라 코어 판으로 견준다
pub fn is_rolled_back_core(failed_release: Option<&str>, core_version: Option<&str>) -> bool {
    failed_release.is_some() && failed_release == core_version
}

/// 받을 코어(`core_version`)가 사용자 고정에 묶이는가: 고정 기준(`failed_release`)과 그보다 옛 코어.
/// 기준은 되돌릴 때 알던 가장 새 코어라, 그 판이 내려가 릴레이가 그보다 옛 판을 주어도 묶인다
pub fn is_held_core(failed_release: Option<&str>, core_version: Option<&str>) -> bool {
    use saba_chan_updater_lib::version::SemVer;
    match (failed_release, core_version) {
        (Some(held), Some(core)) => held == core || matches!((SemVer::parse(held), SemVer::parse(core)), (Some(h), Some(c)) if c <= h),
        _ => false,
    }
}

/// 되돌린 코어 판보다 새 코어가 왔으면 그 기록은 지운다 (`offered`: 받아 둔 묶음의 코어 판)
pub fn failed_release_outdated(failed: &str, offered: &str) -> bool {
    use saba_chan_updater_lib::version::SemVer;
    matches!((SemVer::parse(failed), SemVer::parse(offered)), (Some(f), Some(o)) if o > f)
}

/// 데몬 작업 (백업·서버 설치·게임 서버 업데이트·모듈 호출). 인스턴스 이름은 `name_of`로 찾는다
pub fn daemon_work(name_of: impl Fn(&str) -> Option<String>) -> Vec<CoreSwapBlocker> {
    use crate::instance::lock::{InstanceLock, Purpose};
    let mut work = Vec::new();
    for (key, purpose) in InstanceLock::busy() {
        if let Some(module) = key.strip_prefix("module-install:") {
            work.push(CoreSwapBlocker { kind: CoreSwapBlockerKind::Provision, instance_id: String::new(), name: module.to_string() });
            continue;
        }
        let kind = match purpose {
            Some(Purpose::Backup) => CoreSwapBlockerKind::Backup,
            Some(Purpose::Provision) => CoreSwapBlockerKind::Provision,
            Some(Purpose::ServerUpdate) => CoreSwapBlockerKind::ServerUpdate,
            None => CoreSwapBlockerKind::ModuleCall,
        };
        let name = name_of(&key).unwrap_or_else(|| key.clone());
        work.push(CoreSwapBlocker { kind, instance_id: key, name });
    }
    for (instance, module) in crate::plugin::calls_in_flight() {
        let instance_id = instance.unwrap_or_default();
        // 같은 인스턴스의 긴 작업이 이미 있으면 그 줄로 충분하다
        if !instance_id.is_empty() && work.iter().any(|w| w.instance_id == instance_id) {
            continue;
        }
        if work.iter().any(|w| w.kind == CoreSwapBlockerKind::ModuleCall && w.instance_id == instance_id && w.name == module) {
            continue;
        }
        let name = (!instance_id.is_empty()).then(|| name_of(&instance_id)).flatten().unwrap_or(module);
        work.push(CoreSwapBlocker { kind: CoreSwapBlockerKind::ModuleCall, instance_id, name });
    }
    work
}

/// heartbeat `latestReleases` 중 이 데몬이 보는 값: 고른 채널이 받는 채널, 같은 판 베타 규칙이면 베타까지
pub fn watched_release_ids(latest: &serde_json::Value, channel: &str, current: &str) -> BTreeMap<String, u64> {
    use saba_chan_updater_lib::relay::{channel_includes, stable_wait_base};
    let same_release_beta = stable_wait_base(channel, current).is_some();
    latest
        .as_object()
        .map(|ids| {
            ids.iter()
                .filter(|(c, _)| channel_includes(channel, c) || (same_release_beta && c.as_str() == "beta"))
                .filter_map(|(c, id)| Some((c.clone(), id.as_u64()?)))
                .collect()
        })
        .unwrap_or_default()
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn blocker(kind: CoreSwapBlockerKind) -> CoreSwapBlocker {
        CoreSwapBlocker { kind, instance_id: "i1".into(), name: "월드".into() }
    }

    fn input<'a>(pending: &'a [String]) -> PlanInput<'a> {
        PlanInput {
            pending,
            version: Some("0.1.14"),
            core_version: Some("0.1.14"),
            failed_release: None,
            user_held: false,
            deferred_release: None,
            gui_window_open: false,
            cli_running: false,
            blockers: Vec::new(),
        }
    }

    fn keys(list: &[&str]) -> Vec<String> {
        list.iter().map(|s| s.to_string()).collect()
    }

    /// 켜진 서버가 있어도 코어를 적용한다. 막는 것이 하나라도 있으면 기다린다
    #[test]
    fn the_core_waits_only_for_blockers() {
        let pending = keys(&["saba-core", "cli", "module-palworld"]);
        assert_eq!(decide(input(&pending)), Decision::Apply(keys(&["saba-core", "cli"])));
        for kind in [CoreSwapBlockerKind::Backup, CoreSwapBlockerKind::NonRunnerServer, CoreSwapBlockerKind::ExtensionBridge, CoreSwapBlockerKind::RunnerFormat] {
            let got = decide(PlanInput { blockers: vec![blocker(kind)], ..input(&pending) });
            assert_eq!(got, Decision::Wait(vec![blocker(kind)]));
        }
        // 데몬이 내려가지 않는 묶음(CLI만)은 막는 것을 보지 않는다
        let cli = keys(&["cli"]);
        assert_eq!(decide(PlanInput { blockers: vec![blocker(CoreSwapBlockerKind::Backup)], ..input(&cli) }), Decision::Apply(cli.clone()));
        // 모듈·번역만이면 단계와 상관없다
        assert_eq!(decide(input(&keys(&["module-palworld", "locales"]))), Decision::Nothing);
    }

    /// 13절: 되돌린 판은 다시 자동 적용하지 않는다. 더 새 판이 오면 기록을 지운다
    #[test]
    fn a_rolled_back_release_is_not_applied_again() {
        let pending = keys(&["saba-core"]);
        let got = decide(PlanInput { failed_release: Some("0.1.14"), ..input(&pending) });
        assert_eq!(got, Decision::Ready(ReadyReason::FailedRelease));
        let newer = decide(PlanInput { failed_release: Some("0.1.13"), ..input(&pending) });
        assert_eq!(newer, Decision::Apply(pending.clone()));
        assert!(failed_release_outdated("0.1.14-beta.2", "0.1.14-beta.3"));
        assert!(!failed_release_outdated("0.1.14", "0.1.14"));
        assert!(!failed_release_outdated("0.1.14", "0.1.13"));
    }

    /// 부분 릴리즈: 릴리즈 0.1.15가 코어 0.1.14를 그대로 싣는다. 되돌린 코어 0.1.14는 릴리즈 판이 달라도 다시 적용하지 않고,
    /// 코어가 그대로인 더 새 릴리즈도 그 기록을 지우지 않는다
    #[test]
    fn a_rolled_back_core_reused_by_a_partial_release_is_not_applied_again() {
        let pending = keys(&["saba-core", "gui"]);
        let partial = PlanInput { version: Some("0.1.15"), core_version: Some("0.1.14"), failed_release: Some("0.1.14"), ..input(&pending) };
        assert_eq!(decide(partial), Decision::Ready(ReadyReason::FailedRelease));
        assert!(!failed_release_outdated("0.1.14", "0.1.14"), "the same core in a newer release keeps the record");
        // 코어가 없는 묶음(GUI만)은 되돌린 코어와 상관없다
        let gui = keys(&["gui"]);
        assert_eq!(decide(PlanInput { version: Some("0.1.16"), core_version: None, failed_release: Some("0.1.14"), ..input(&gui) }), Decision::Apply(gui.clone()));
    }

    /// 사용자가 고른 되돌리기는 고정이다: 되돌린 코어를 다시 받아 두어도 준비됨으로 알리지 않는다.
    /// 업데이트가 실패해 되돌린 코어는 전처럼 준비됨이고, 더 새 코어는 고정과 상관없이 적용한다
    #[test]
    fn a_core_the_user_rolled_back_from_is_held_without_a_ready_phase() {
        let pending = keys(&["saba-core", "gui"]);
        assert_eq!(decide(PlanInput { failed_release: Some("0.1.14"), user_held: true, ..input(&pending) }), Decision::Held);
        assert_eq!(decide(PlanInput { failed_release: Some("0.1.14"), ..input(&pending) }), Decision::Ready(ReadyReason::FailedRelease));
        assert_eq!(decide(PlanInput { failed_release: Some("0.1.13"), user_held: true, ..input(&pending) }), Decision::Apply(pending.clone()));
        // 고정 기준이 받을 코어보다 새 판이어도(되돌릴 때 이미 알던 판이 내려갔다) 묶인다. 업데이트 실패의 기록은 같은 판만 막는다
        assert_eq!(decide(PlanInput { failed_release: Some("0.1.15"), user_held: true, ..input(&pending) }), Decision::Held);
        assert_eq!(decide(PlanInput { failed_release: Some("0.1.15"), ..input(&pending) }), Decision::Apply(pending.clone()));
    }

    /// 이전 데몬이 끝나지 않아 미룬 묶음은 같은 릴리즈면 다시 띄우지 않는다. 더 새 릴리즈면 적용한다
    #[test]
    fn a_deferred_release_is_not_applied_again_automatically() {
        let pending = keys(&["saba-core"]);
        assert_eq!(decide(PlanInput { deferred_release: Some("0.1.14"), ..input(&pending) }), Decision::Ready(ReadyReason::WaitPidTimeout));
        assert_eq!(decide(PlanInput { deferred_release: Some("0.1.13"), ..input(&pending) }), Decision::Apply(pending.clone()));
    }

    /// GUI가 바뀌는데 창이 열려 있으면 묶음 전체가 준비됨. 트레이에만 있으면 적용한다. CLI TUI는 CLI만 다음으로
    #[test]
    fn an_open_gui_window_or_tui_holds_its_own_part() {
        let pending = keys(&["saba-core", "gui", "cli"]);
        assert_eq!(decide(PlanInput { gui_window_open: true, ..input(&pending) }), Decision::Ready(ReadyReason::GuiWindowOpen));
        assert_eq!(decide(input(&pending)), Decision::Apply(pending.clone()));
        assert_eq!(decide(PlanInput { cli_running: true, ..input(&pending) }), Decision::Apply(keys(&["saba-core", "gui"])));
        assert_eq!(decide(PlanInput { cli_running: true, ..input(&keys(&["cli"])) }), Decision::Nothing);
        // GUI가 바뀌지 않으면 창이 열려 있어도 된다
        assert_eq!(decide(PlanInput { gui_window_open: true, ..input(&keys(&["saba-core"])) }), Decision::Apply(keys(&["saba-core"])));
    }

    /// 새 데몬은 완료 마커로 단계를 정한다: 성공은 done, 되돌림·미룸은 ready. 같은 마커는 한 번만
    #[test]
    fn the_new_daemon_takes_its_phase_from_the_completion_marker() {
        let release = json!({ "version": "0.1.14", "notes": { "ko": "고침" } });
        let done = json!({ "timestamp": "t1", "updated_components": ["saba-core", "gui"], "success": true, "release": release });
        let phase = phase_from_marker(&done, None).unwrap();
        assert_eq!((phase.state, phase.version.as_deref()), (PhaseState::Done, Some("0.1.14")));
        assert_eq!(phase.notes.unwrap()["ko"], "고침");
        assert!(phase_from_marker(&done, Some("t1")).is_none(), "a marker read once does not set the phase again");

        let rolled = json!({ "timestamp": "t2", "updated_components": [], "success": false, "result": "rolled_back", "version": "0.1.14" });
        let phase = phase_from_marker(&rolled, Some("t1")).unwrap();
        assert_eq!((phase.state, phase.reason, phase.version.as_deref()), (PhaseState::Ready, Some(ReadyReason::RolledBack), Some("0.1.14")));
        // 되돌린 코어 판(0.1.14)이 아니라 릴리즈 판(0.1.15)을 보인다
        let partial = json!({ "timestamp": "t5", "success": false, "result": "rolled_back", "version": "0.1.14", "release": { "version": "0.1.15" } });
        assert_eq!(phase_from_marker(&partial, None).unwrap().version.as_deref(), Some("0.1.15"));
        let deferred = json!({ "timestamp": "t3", "updated_components": [], "success": false, "result": "deferred", "reason": "wait_pid_timeout" });
        assert_eq!(phase_from_marker(&deferred, None).unwrap().reason, Some(ReadyReason::WaitPidTimeout));

        // 모듈·익스텐션만 바꾼 기록은 알약을 움직이지 않는다
        let addons = json!({ "timestamp": "t4", "updated_components": ["module-palworld", "locales"], "success": true });
        assert!(phase_from_marker(&addons, None).is_none());
    }

    /// 사용자가 직전 버전으로 되돌리면(`reverted`) 되돌리기 전 판의 '업데이트 완료' 알약을 걷는다
    #[test]
    fn a_user_rollback_clears_the_done_pill_of_the_version_it_left() {
        let mut live = LiveState { phase: Phase::new(PhaseState::Done, Some("0.1.15".into()), None), ..Default::default() };
        let reverted = json!({ "timestamp": "t9", "updated_components": [], "success": true, "result": "reverted", "version": "0.1.14", "replaced_by": "0.1.15" });
        assert_eq!(adopt_startup_marker(&mut live, Some(&reverted), || None), (true, None));
        assert_eq!(live.phase, Phase::default());
        assert_eq!(live.marker_seen.as_deref(), Some("t9"));
    }

    /// 이전 데몬이 적용을 시작한 채 끝났는데 마커가 없으면 단계를 걷는다. 이미 읽은 마커는 단계를 되살리지 않는다
    #[test]
    fn a_new_daemon_clears_a_stale_applying_phase() {
        let mut live = LiveState { phase: Phase::new(PhaseState::Applying, Some("0.1.14".into()), None), ..Default::default() };
        assert!(adopt_marker(&mut live, None));
        assert_eq!(live.phase, Phase::default());
        let done = json!({ "timestamp": "t1", "updated_components": ["saba-core"], "success": true, "release": { "version": "0.1.14" } });
        assert!(adopt_marker(&mut live, Some(&done)));
        assert_eq!((live.phase.state, live.marker_seen.as_deref()), (PhaseState::Done, Some("t1")));
        live.phase = Phase::default();
        assert!(!adopt_marker(&mut live, Some(&done)));
        assert_eq!(live.phase, Phase::default());
    }

    /// 뜰 때: 되돌림이면 되돌린 코어 판(릴리즈 판이 아님)을 실패 기록 후보로, 미룸이면 받아 둔 릴리즈 판을 단계에 채운다
    #[test]
    fn startup_marker_names_the_rolled_back_core_and_the_deferred_release() {
        let mut live = LiveState::default();
        let rolled = json!({ "timestamp": "t1", "success": false, "result": "rolled_back", "version": "0.1.14", "release": { "version": "0.1.15" } });
        assert_eq!(adopt_startup_marker(&mut live, Some(&rolled), || None), (true, Some("0.1.14".to_string())));
        assert_eq!(live.phase.version.as_deref(), Some("0.1.15"));

        let mut live = LiveState::default();
        let deferred = json!({ "timestamp": "t2", "success": false, "result": "deferred", "reason": "wait_pid_timeout" });
        assert_eq!(adopt_startup_marker(&mut live, Some(&deferred), || Some("0.1.15".into())), (true, None));
        assert_eq!((live.phase.reason, live.phase.version.as_deref()), (Some(ReadyReason::WaitPidTimeout), Some("0.1.15")));
        assert_eq!(adopt_startup_marker(&mut live, Some(&deferred), || None), (false, None), "read once");
    }

    #[test]
    fn done_turns_idle_after_a_day() {
        let now = chrono::Utc::now();
        let mut phase = Phase::new(PhaseState::Done, Some("0.1.14".into()), None);
        assert_eq!(phase.visible(now).state, PhaseState::Done);
        phase.since = Some((now - chrono::Duration::hours(25)).to_rfc3339());
        assert_eq!(phase.visible(now), Phase::default());
        // 다른 단계는 시간으로 사라지지 않는다
        phase.state = PhaseState::Ready;
        assert_eq!(phase.visible(now).state, PhaseState::Ready);
    }

    /// 상태 응답의 모양 (GUI가 읽는다)
    #[test]
    fn the_phase_serializes_as_the_gui_reads_it() {
        let mut phase = Phase::new(PhaseState::Waiting, Some("0.1.14".into()), None);
        phase.waiting_for = vec![blocker(CoreSwapBlockerKind::ServerUpdate), blocker(CoreSwapBlockerKind::ModuleCall)];
        let v = serde_json::to_value(&phase).unwrap();
        assert_eq!(v["state"], "waiting");
        assert_eq!(v["waiting_for"][0], json!({ "kind": "server_update", "instance_id": "i1", "name": "월드" }));
        assert_eq!(v["waiting_for"][1]["kind"], "module_call");
        assert_eq!(v["reason"], serde_json::Value::Null);
        assert_eq!(serde_json::to_value(Phase::ready(ReadyReason::GuiWindowOpen, None, None)).unwrap()["reason"], "gui_window_open");
    }

    /// heartbeat 판 번호: 고른 채널이 받는 채널만, 정식 채널의 베타 판이면 베타까지 본다
    #[test]
    fn heartbeat_release_ids_are_watched_for_the_channels_this_daemon_takes() {
        let ids = json!({ "stable": 10, "rc": 11, "beta": 14 });
        let got = |channel: &str, current: &str| watched_release_ids(&ids, channel, current).into_iter().collect::<Vec<_>>();
        assert_eq!(got("stable", "0.1.13"), vec![("stable".to_string(), 10)]);
        assert_eq!(got("stable", "0.1.14-beta.2"), vec![("beta".to_string(), 14), ("stable".to_string(), 10)]);
        assert_eq!(got("beta", "0.1.13").len(), 3);
    }

    /// 데몬 작업은 한 곳에서: 이름표 붙은 작업 잠금, 모듈 설치, 진행 중인 모듈 호출
    #[tokio::test]
    async fn daemon_work_reads_instance_locks_and_module_installs() {
        use crate::instance::lock::{InstanceLock, Purpose};
        let id = format!("phase-test-{}", std::process::id());
        let names = |k: &str| (k == id).then(|| "내 서버".to_string());
        let backup = InstanceLock::take(&id).unwrap().purpose(Purpose::Backup);
        let found = daemon_work(names);
        assert!(found.iter().any(|w| w.kind == CoreSwapBlockerKind::Backup && w.instance_id == id && w.name == "내 서버"), "{found:?}");
        drop(backup);
        let update = InstanceLock::take_for_writes(&id).await.unwrap().purpose(Purpose::ServerUpdate);
        assert!(daemon_work(names).iter().any(|w| w.kind == CoreSwapBlockerKind::ServerUpdate && w.instance_id == id));
        drop(update);
        let other = InstanceLock::take(&id).unwrap();
        assert!(daemon_work(names).iter().any(|w| w.kind == CoreSwapBlockerKind::ModuleCall && w.instance_id == id), "an unlabeled task calls the module");
        drop(other);
        assert!(!daemon_work(names).iter().any(|w| w.instance_id == id));
        let module = format!("phase-mod-{}", std::process::id());
        let install = InstanceLock::take(&format!("module-install:{module}")).unwrap();
        assert!(daemon_work(names).iter().any(|w| w.kind == CoreSwapBlockerKind::Provision && w.name == module));
        drop(install);
    }

    #[test]
    fn the_state_file_round_trips() {
        let tmp = tempfile::tempdir().unwrap();
        let path = tmp.path().join("live-update.json");
        assert_eq!(LiveState::load_from(&path), LiveState::default());
        let state = LiveState {
            seen: BTreeMap::from([("stable".to_string(), 12)]),
            phase: Phase::ready(ReadyReason::RolledBack, Some("0.1.14".into()), None),
            marker_seen: Some("t".into()),
        };
        state.save_to(&path);
        assert_eq!(LiveState::load_from(&path), state);
    }
}
