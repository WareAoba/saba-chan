//! 모듈·익스텐션: 받은 판을 묻지 않고 바꾸는 반복(켜진 서버가 쓰는 것은 기다린다)과 판 고르기·고정 (`/api/updates/addons/*`)

use super::*;
use super::apply::{ApplyRequest, apply_updates_with, bad_body, restore_pending};

pub(super) fn is_addon(c: &Component) -> bool {
    matches!(c, Component::Module(_) | Component::Extension(_))
}

/// 켜진 서버가 쓰고 있어 미룬 모듈·익스텐션 교체를 다시 해 보는 간격
pub(super) const ADDON_HOLD_RETRY: std::time::Duration = std::time::Duration::from_secs(5 * 60);
/// 받지 못했거나 바꾸지 못한 모듈·익스텐션 교체를 다시 해 보는 간격 (잠깐의 네트워크 오류로 새 판이 다음 정기 확인까지 밀리지 않게).
/// 다 쓰면 다음 확인에 맡긴다
const ADDON_FAIL_RETRY: [std::time::Duration; 3] =
    [ADDON_HOLD_RETRY, std::time::Duration::from_secs(30 * 60), std::time::Duration::from_secs(30 * 60)];
pub(super) static ADDON_REPLACING: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);

/// 교체 한 차례가 남긴 것
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub(super) struct Leftover {
    /// 켜진 서버가 쓰거나 서버 시작·설치 중이라 미뤘다
    pub(super) held: bool,
    /// 받지 못했거나 바꾸지 못했다
    pub(super) failed: bool,
}

/// 남긴 것이 있으면 다음 차례까지 기다릴 시간. `failures`는 잇달아 실패한 차례 수
pub(super) fn addon_retry_wait(left: Leftover, failures: &mut usize) -> Option<std::time::Duration> {
    if !left.failed {
        *failures = 0;
        return left.held.then_some(ADDON_HOLD_RETRY);
    }
    let wait = ADDON_FAIL_RETRY.get(*failures).copied();
    *failures = failures.saturating_add(1);
    if left.held { Some(ADDON_HOLD_RETRY) } else { wait }
}

/// 다시 해 보려고 기다리는 교체를 깨운다: 그 사이의 확인(사용자가 누른 것 포함)이 남은 잠 뒤로 밀리지 않게
static ADDON_RECHECK: tokio::sync::Notify = tokio::sync::Notify::const_new();

/// 모듈·익스텐션 자동 교체: 새 판(지금 코어로 돌아가는 가장 새 판, 릴레이 `/v1/addons/catalog`)을 받아 바로 바꿔 끼운다.
/// 켜진 서버가 쓰는 것은 서버가 꺼질 때까지 5분마다 다시 보고, 받지 못했거나 바꾸지 못한 것은 5분·30분·30분 뒤에 다시 해 본다.
/// 기다리는 사이에 확인이 또 오면 바로 한 차례 돈다.
/// 사용자가 고친 것·직접 넣은 것은 덮어쓰지 않는다. 한 번에 하나만 돈다
pub(super) fn spawn_addon_replace(state: UpdateState) {
    use std::sync::atomic::Ordering;
    if ADDON_REPLACING.swap(true, Ordering::SeqCst) {
        ADDON_RECHECK.notify_one();
        return;
    }
    tokio::spawn(async move {
        replace_addons_until_done(
            || async {
                let left = replace_addons_once(&state).await;
                state.status_may_have_changed();
                left
            },
            &ADDON_RECHECK,
        )
        .await;
        ADDON_REPLACING.store(false, Ordering::SeqCst);
    });
}

/// 남긴 것이 없어질 때까지(또는 실패를 다 쓸 때까지) 차례를 돌린다. 차례 사이의 잠은 `recheck`가 깨운다
pub(super) async fn replace_addons_until_done<Fut: std::future::Future<Output = Leftover>>(mut once: impl FnMut() -> Fut, recheck: &tokio::sync::Notify) {
    let mut failures = 0;
    while let Some(wait) = addon_retry_wait(once().await, &mut failures) {
        tokio::select! {
            _ = tokio::time::sleep(wait) => {}
            _ = recheck.notified() => {}
        }
    }
}

/// 바꿀 수 있는 것을 바꾸고, 다음 차례로 남긴 것(켜진 서버 때문에 미룬 것, 실패한 것)을 돌려준다
pub(super) async fn replace_addons_once(state: &UpdateState) -> Leftover {
    // 미뤄 둔 사이 업데이트 확인을 껐으면 멈춘다. 사용자가 고른 판으로 바꾸기(`addon_pins`의 `pending_version`)는 마저 한다
    let enabled = state.manager.read().await.get_config().enabled;
    let chosen: std::collections::HashSet<String> =
        addon_pins::load().into_iter().filter(|(_, c)| c.pending_version.is_some()).map(|(k, _)| k).collect();
    if !enabled && chosen.is_empty() {
        return Leftover::default();
    }
    // 이 코어로 돌아가지 않는 판(릴리즈 manifest에만 있는 더 새 판)은 앱을 올릴 때까지 둔다: 적용이 requires 검사로 거절한다
    let pending: Vec<Component> = state.manager.read().await.get_status().components.into_iter()
        .filter(|c| c.update_available && is_addon(&c.component))
        .filter(|c| enabled || chosen.contains(&c.component.manifest_key()))
        .filter(|c| {
            let core = c.requires.as_ref().and_then(|r| r.get(crate::module_loader::CORE_COMPONENT));
            core.is_none_or(|req| saba_chan_updater_lib::version::satisfies(crate::module_loader::DAEMON_VERSION, req))
        })
        .map(|c| c.component)
        .collect();
    if pending.is_empty() {
        return Leftover::default();
    }
    let (busy_modules, busy_extensions) = busy_addons(state).await;
    let pending: Vec<Component> = pending.into_iter().filter(|c| replaceable_without_asking(&c.manifest_key())).collect();
    let (ready, held) = split_by_busy(pending, &busy_modules, &busy_extensions);
    if !held.is_empty() {
        tracing::info!("[Updates] Servers are using {:?}; replacing them after they stop", held.iter().map(Component::manifest_key).collect::<Vec<_>>());
    }
    if ready.is_empty() {
        return Leftover { held: !held.is_empty(), failed: false };
    }
    let mut failed = false;
    let mut downloaded = Vec::new();
    for comp in ready {
        // 받는 동안은 관리자를 잠그지 않는다: 상태 조회·업데이트 화면이 기다리지 않는다
        let result = saba_chan_updater_lib::download_shared(&state.manager, &comp).await;
        match result {
            Ok(_) => downloaded.push(comp),
            Err(e) => {
                failed = true;
                tracing::warn!("[Updates] Could not download {} for replacement: {}", comp.manifest_key(), e);
            }
        }
    }
    // 여기부터 교체가 끝날 때까지 서버 시작을 막는다: 사용 확인과 교체 사이에 켜진 서버가 쓰던 판을 바꾸지 않게
    // 시작·설치가 진행 중이면 기다리지 않고 다음 차례로 미룬다 (받은 파일은 남는다)
    let Some(_starts) = try_hold_starts(state).await else {
        tracing::info!("[Updates] A server is starting or installing; replacing modules and extensions later");
        return Leftover { held: true, failed };
    };
    // 받는 사이에 켠 서버가 쓰는 것은 바꾸지 않고 다음 차례로 미룬다
    let (busy_modules, busy_extensions) = busy_addons(state).await;
    let (ready, started) = split_by_busy(downloaded, &busy_modules, &busy_extensions);
    let held = !held.is_empty() || !started.is_empty();
    // 받는 사이에 사용자가 고치거나 넣은 파일이 있으면 두고 넘어간다: 교체가 폴더를 비운다.
    // 이 확인 뒤(적용이 업데이트 관리자를 기다리는 동안) 생긴 변경은 적용이 백업을 뜬 순간 한 번 더 보고 멈춘다 (`untouched_only`)
    let keys: Vec<String> = ready.iter().map(Component::manifest_key).filter(|k| replaceable_without_asking(k)).collect();
    if !keys.is_empty() {
        tracing::info!("[Updates] Replacing {:?} without asking", keys);
        let answer = crate::ipc::internal::into_internal(apply_updates_with(state.clone(), ApplyRequest { components: keys, untouched_only: true }).await.into_response()).await.body;
        // 바꾸지 못한 것(교체 순간 사용자 파일이 보였거나, 옮기지 못함)은 업데이트 목록에 남아 사용자가 고른다.
        // 다음 차례는 사용자 파일이 있는 것을 받기 전에 거르므로(`replaceable_without_asking`) 옮기지 못한 것만 다시 해 본다
        if let Some(errors) = answer.get("errors").and_then(Value::as_array).filter(|e| !e.is_empty()) {
            failed = true;
            tracing::warn!("[Updates] Some modules or extensions were not replaced: {:?}", errors);
        }
    }
    Leftover { held, failed }
}

/// 업데이터가 코어가 낀 묶음에서 새 코어를 기다리게 둔 모듈·익스텐션(완료 마커 `deferred`)을 마저 바꾼다.
/// 사용자가 고른 묶음이므로 자동 교체와 달리 업데이트 확인 설정·사용자 변경과 상관없이 업데이터처럼 바꾸고,
/// 켜진 서버가 쓰는 것은 서버가 꺼질 때까지 5분마다 다시 본다. 기동 때 한 번 띄운다
pub fn spawn_deferred_apply(state: UpdateState) {
    tokio::spawn(async move {
        let staging = saba_chan_updater_lib::constants::resolve_staging_dir();
        while let Some(wait) = apply_deferred_once(&state, &staging).await {
            tokio::time::sleep(wait).await;
        }
    });
}

/// 한 차례. 다음 차례까지 기다릴 시간을 돌려주고, 할 것이 없으면 None
pub(super) async fn apply_deferred_once(state: &UpdateState, staging: &std::path::Path) -> Option<std::time::Duration> {
    // 업데이터가 이 데몬을 지켜보는 동안은 바꾸지 않는다: 건강하지 않아 코어만 되돌리면 옛 코어가 거절하는 판이 남는다
    const WATCH_POLL: std::time::Duration = std::time::Duration::from_secs(2);
    if saba_chan_updater_lib::live_apply::watch_in_progress(staging) {
        return Some(WATCH_POLL);
    }
    // 되돌린(`rolled_back`) 뒤나 새 판을 그대로 둔(`left_in_place`) 마커는 하지 않는다: 받아 둔 채 다음 앱 업데이트를 기다린다
    let deferred = saba_chan_updater_lib::UpdateCompletionMarker::load().filter(|m| m.result.is_none())?.deferred;
    let pending: Vec<Component> = {
        let mut mgr = state.manager.write().await;
        restore_pending(&mut mgr);
        mgr.get_pending_components().into_iter().map(|c| c.component.clone()).filter(|c| deferred.contains(&c.manifest_key())).collect()
    };
    if pending.is_empty() {
        return None;
    }
    // 교체가 끝날 때까지 서버 시작을 막는다: 사용 확인과 교체 사이에 켜진 서버가 쓰던 판을 바꾸지 않게
    let Some(_starts) = try_hold_starts(state).await else {
        return Some(ADDON_HOLD_RETRY);
    };
    let (busy_modules, busy_extensions) = busy_addons(state).await;
    let (ready, held) = split_by_busy(pending, &busy_modules, &busy_extensions);
    if !ready.is_empty() {
        let keys: Vec<String> = ready.iter().map(Component::manifest_key).collect();
        tracing::info!("[Updates] Applying {:?}, which waited for this core", keys);
        // 결과는 `record_daemon_apply`가 그 묶음의 완료 마커에 더한다. 실패한 것은 받아 둔 채 업데이트 목록에 남는다
        let _ = apply_updates_with(state.clone(), ApplyRequest { components: keys, ..Default::default() }).await;
    }
    if !held.is_empty() {
        tracing::info!("[Updates] Servers are using {:?}; applying them after they stop", held.iter().map(Component::manifest_key).collect::<Vec<_>>());
        return Some(ADDON_HOLD_RETRY);
    }
    None
}

/// 지금 바꿀 것과 켜진 서버가 써서 미룰 것
pub(super) fn split_by_busy(
    pending: Vec<Component>,
    busy_modules: &std::collections::HashSet<String>,
    busy_extensions: &std::collections::HashSet<String>,
) -> (Vec<Component>, Vec<Component>) {
    pending.into_iter().partition(|c| match c {
        Component::Module(name) => !busy_modules.contains(name),
        Component::Extension(id) => !busy_extensions.contains(id),
        _ => false,
    })
}

/// 카탈로그 그대로인(Verified) 모듈·익스텐션만 묻지 않고 바꾼다. 사용자가 고쳤거나(Modified) 직접 넣은(Unverified) 것,
/// 설치 기록에 없는 파일(사용자가 넣은 설정·데이터)이 있는 것은 둔다: 교체가 폴더를 비워 사용자의 것을 지운다.
/// 그런 것은 업데이트 목록에 남아 사용자가 고를 수 있다
fn replaceable_without_asking(key: &str) -> bool {
    let Some(dir) = crate::sandbox::component_dir(key) else { return false };
    if !saba_chan_updater_lib::integrity::replaceable_in_place(key, &dir) {
        tracing::info!("[Updates] Not replacing {} automatically: its folder is not exactly what the catalog installed", key);
        return false;
    }
    true
}

/// 켜진 서버가 쓰는 모듈 이름과 익스텐션 id
async fn busy_addons(state: &UpdateState) -> (std::collections::HashSet<String>, std::collections::HashSet<String>) {
    let mut modules = std::collections::HashSet::new();
    let mut extensions = std::collections::HashSet::new();
    for user in running_addon_users(state).await {
        modules.insert(user.module);
        extensions.extend(user.extensions);
    }
    (modules, extensions)
}

/// 켜진 서버 하나가 쓰는 모듈과 익스텐션
struct AddonUser {
    name: String,
    module: String,
    extensions: std::collections::HashSet<String>,
}

/// 켜진 서버마다 쓰는 모듈·익스텐션
async fn running_addon_users(state: &UpdateState) -> Vec<AddonUser> {
    let mut users = Vec::new();
    let mut ext_data = Vec::new();
    if let Some(ref sup) = state.supervisor {
        let sup = sup.read().await;
        let running = sup.running_instances().await;
        for instance in sup.instance_store.list() {
            if running.contains(&instance.id) {
                let extensions = sup.effective_required_extensions(instance).into_iter().collect();
                users.push(AddonUser { name: instance.name.clone(), module: instance.module_name.clone(), extensions });
                ext_data.push((instance.id.clone(), instance.extension_data.clone()));
            }
        }
    }
    // 필수 목록에 없어도 켜진 인스턴스가 칸 값으로 쓰는 익스텐션 (손으로 교체할 때의 `in_use` 판정과 같다). 데몬 락을 놓은 뒤에 본다
    if let Some(ref mgr) = state.extension_manager {
        let mgr = mgr.read().await;
        for (user, (id, data)) in users.iter_mut().zip(&ext_data) {
            user.extensions.extend(mgr.extensions_in_use(&[(id.as_str(), data)]));
        }
    }
    users
}

/// 이 데몬의 서버 시작·설치를 막는다(`AddonSwapGate::try_hold_starts`). 감독자가 없으면 막을 시작도 없다
async fn try_hold_starts(state: &UpdateState) -> Option<tokio::sync::OwnedRwLockWriteGuard<()>> {
    let gate = match &state.supervisor {
        Some(sup) => sup.read().await.addon_gate(),
        None => crate::supervisor::AddonSwapGate::default(),
    };
    gate.try_hold_starts()
}

/// 켜진 서버 중 이 모듈·익스텐션을 쓰는 것의 이름
async fn servers_using(state: &UpdateState, component: &Component) -> Vec<String> {
    let uses = |user: &AddonUser| match component {
        Component::Module(name) => user.module == *name,
        Component::Extension(id) => user.extensions.contains(id),
        _ => false,
    };
    running_addon_users(state).await.into_iter().filter(|u| uses(u)).map(|u| u.name).collect()
}

// ── 판 고르기 (모듈·익스텐션) ──

/// 판 고르기가 받는 키 (`module-<이름>`, `ext-<id>`)
fn addon_component(key: &str) -> Option<Component> {
    Component::try_from_manifest_key(key).filter(is_addon)
}

fn addon_refusal(status: axum::http::StatusCode, code: &str, error: impl std::fmt::Display) -> axum::response::Response {
    crate::ipc::handlers::error(status, code, error.to_string())
}

fn not_an_addon(key: &str) -> axum::response::Response {
    addon_refusal(axum::http::StatusCode::BAD_REQUEST, "invalid_key", format!("'{key}' is not a module or an extension"))
}

/// 릴레이에 묻지 못했다 (받기 실패와 같은 502)
fn relay_unreachable(e: saba_chan_updater_lib::relay::RelayError) -> axum::response::Response {
    addon_refusal(axum::http::StatusCode::BAD_GATEWAY, "relay_unreachable", e)
}

/// 판 고르기 창의 목록: 고를 수 있는 판과 내린 판을 새 판부터. 가장 새 판(`latest`)은 내리지 않은 것 중에서
pub(super) fn addon_versions_body(
    key: &str,
    core: &str,
    installed: Option<&str>,
    list: &saba_chan_updater_lib::relay::AddonVersions,
    pinned: Option<&addon_pins::AddonPin>,
) -> Value {
    use saba_chan_updater_lib::version::SemVer;
    let mut rows: Vec<(&str, bool)> = list.versions.iter().map(|c| (c.version.as_str(), false)).chain(list.yanked.iter().map(|v| (v.as_str(), true))).collect();
    rows.sort_by(|a, b| SemVer::parse(b.0).cmp(&SemVer::parse(a.0)));
    let latest = list.versions.first().map(|c| c.version.as_str());
    let versions: Vec<Value> = rows
        .into_iter()
        .map(|(version, yanked)| json!({ "version": version, "installed": installed == Some(version), "latest": latest == Some(version), "yanked": yanked }))
        .collect();
    json!({ "success": true, "key": key, "core": core, "installed": installed, "versions": versions, "excluded": list.excluded, "pinned": pinned })
}

/// GET /api/updates/addons/{key}/versions — 이 코어로 돌아가는 판(서명 확인)과 내린 판, 더 새 코어가 필요해 뺀 판
pub(super) async fn get_addon_versions(State(state): State<UpdateState>, Path(key): Path<String>) -> axum::response::Response {
    if addon_component(&key).is_none() {
        return not_an_addon(&key);
    }
    let (ask, core, installed) = {
        let mgr = state.manager.read().await;
        (mgr.addon_versions(&key), mgr.addon_core(), mgr.installed_addon_version(&key))
    };
    match ask.await {
        Ok(list) => {
            let pinned = addon_pins::get(&key).as_installed(installed.as_deref().unwrap_or_default()).pinned;
            Json(addon_versions_body(&key, &core, installed.as_deref(), &list, pinned.as_ref())).into_response()
        }
        Err(e) => relay_unreachable(e),
    }
}

#[derive(Deserialize)]
struct PinRequest {
    version: String,
}

/// POST /api/updates/addons/{key}/pin `{version}` — 고른 판을 받아(서명·sha256 확인) 바꾸고, 그 순간 가장 새 판보다 새 판이 나올 때까지 고정한다
pub(super) async fn pin_addon(State(state): State<UpdateState>, Path(key): Path<String>, raw: axum::body::Bytes) -> axum::response::Response {
    let body: PinRequest = match serde_json::from_slice(&raw) {
        Ok(b) => b,
        Err(e) => return bad_body(format!("invalid JSON body: {}", e)),
    };
    let Some(component) = addon_component(&key) else { return not_an_addon(&key) };
    let ask = state.manager.read().await.addon_versions(&key);
    let list = match ask.await {
        Ok(list) => list,
        Err(e) => return relay_unreachable(e),
    };
    let conflict = axum::http::StatusCode::CONFLICT;
    let Some(entry) = list.find(&body.version).cloned() else {
        let why = if list.yanked.contains(&body.version) {
            "it was yanked"
        } else if list.excluded.contains(&body.version) {
            "it needs a newer saba-chan"
        } else {
            "it is not offered"
        };
        return addon_refusal(conflict, "version_unavailable", format!("{} {} cannot be chosen: {}", key, body.version, why));
    };
    // 고정은 지금 이 코어에 내는 가장 새 판(보통 바꾸기 전 판)보다 새 판이 나올 때까지. 가장 새 판을 고르면 고정하지 않는다 (고정 풀기와 같다)
    let until = list.versions.first().map(|c| c.version.clone()).unwrap_or_else(|| entry.version.clone());
    let pin = (entry.version != until).then(|| addon_pins::AddonPin { version: entry.version.clone(), until_newer_than: until });
    switch_addon(&state, &component, &entry, pin).await
}

/// DELETE /api/updates/addons/{key}/pin — 고정을 풀고 지금 가장 새 판으로 바로 바꾼다
pub(super) async fn unpin_addon(State(state): State<UpdateState>, Path(key): Path<String>) -> axum::response::Response {
    let Some(component) = addon_component(&key) else { return not_an_addon(&key) };
    let ask = state.manager.read().await.addon_versions(&key);
    let list = match ask.await {
        Ok(list) => list,
        Err(e) => return relay_unreachable(e),
    };
    let Some(newest) = list.versions.first().cloned() else {
        return addon_refusal(axum::http::StatusCode::CONFLICT, "version_unavailable", format!("no version of {} is offered for this saba-chan", key));
    };
    switch_addon(&state, &component, &newest, None).await
}

/// 판 고르기가 거절하는 사용자 폴더: 자동 교체처럼 카탈로그 그대로가 아닌 폴더는 덮어쓰지 않는다. 오류 코드는 신뢰 표시의 것
/// (`module_modified`·`module_unverified`, 익스텐션은 `extension_…`)
pub(crate) fn hand_modified(key: &str) -> Option<String> {
    use saba_chan_updater_lib::integrity::{self, Trust};
    if replaceable_without_asking(key) {
        return None;
    }
    let kind = if key.starts_with(saba_chan_updater_lib::relay::MODULE_PREFIX) { "module" } else { "extension" };
    let trust = crate::sandbox::component_dir(key).map(|dir| integrity::trust_of(key, &dir));
    Some(format!("{}_{}", kind, if trust == Some(Trust::Unverified) { "unverified" } else { "modified" }))
}

/// `entry` 판으로 바꾸고 `pin`을 남긴다 (`None`이면 고정을 푼다). 받기·검증·교체는 자동 교체와 같은 길(`download_component`,
/// `apply_updates_with`의 `untouched_only`)이고, 켜진 서버가 쓰면 자동 교체 반복이 서버가 꺼진 뒤 바꾼다.
/// 실패하면 고른 것과 상태를 그 전으로 되돌린다
async fn switch_addon(
    state: &UpdateState,
    component: &Component,
    entry: &saba_chan_updater_lib::ManifestComponent,
    pin: Option<addon_pins::AddonPin>,
) -> axum::response::Response {
    use axum::http::StatusCode;
    let _touched = StatusTouched(state);
    let key = component.manifest_key();
    let version = entry.version.clone();
    let previous = addon_pins::get(&key);
    let record = |choice: addon_pins::AddonChoice| {
        if let Err(e) = addon_pins::update(&key, |c| *c = choice) {
            tracing::warn!("[Updates] Could not save the chosen version of {}: {}", key, e);
        }
    };
    let answer = |state: &str, waiting_for: Vec<String>| Json(json!({ "success": true, "state": state, "version": version, "waiting_for": waiting_for })).into_response();
    // 이미 깔린 판이면 고정만 바꾼다. 켜진 서버를 기다리던 다른 판은 버린다
    if state.manager.read().await.installed_addon_version(&key).as_deref() == Some(version.as_str()) {
        state.manager.write().await.settle_addon(&key, entry);
        record(addon_pins::AddonChoice { pinned: pin, pending_version: None });
        return answer("applied", Vec::new());
    }
    // 요구 조건은 깔린 것과 대조한다 (코어는 릴레이가 걸렀다). 맞지 않으면 받지 않는다
    let unmet = state.manager.read().await.unmet_addon_requires(&key, &entry.requires.clone().unwrap_or_default());
    if !unmet.is_empty() {
        return addon_refusal(StatusCode::CONFLICT, "requires_unmet", unmet.iter().map(|i| i.message.as_str()).collect::<Vec<_>>().join("; "));
    }
    if let Some(code) = hand_modified(&key) {
        return addon_refusal(StatusCode::CONFLICT, &code, format!("{} was changed by hand; it is not replaced", key));
    }
    let staged = state.manager.write().await.stage_addon(&key, entry);
    let downloaded = saba_chan_updater_lib::download_shared(&state.manager, component).await;
    if let Err(e) = downloaded {
        state.manager.write().await.unstage_addon(staged);
        use saba_chan_updater_lib::download::DownloadError;
        let unverified = matches!(e.downcast_ref::<DownloadError>(), Some(DownloadError::Mismatch { .. } | DownloadError::BadHash(_) | DownloadError::TooLarge { .. }));
        let code = if unverified { "verify_failed" } else { "download_failed" };
        return addon_refusal(StatusCode::BAD_GATEWAY, code, e);
    }
    record(addon_pins::AddonChoice { pinned: pin, pending_version: Some(version.clone()) });
    // 여기부터 바꿀 때까지 서버 시작을 막는다 (자동 교체와 같다). 시작·설치 중이거나 켜진 서버가 쓰면 서버가 꺼진 뒤 자동 교체 반복이 바꾼다
    let starts = try_hold_starts(state).await;
    let waiting_for = if starts.is_some() { servers_using(state, component).await } else { Vec::new() };
    if starts.is_none() || !waiting_for.is_empty() {
        drop(starts);
        spawn_addon_replace(state.clone());
        return answer("waiting", waiting_for);
    }
    let applied = crate::ipc::internal::into_internal(
        apply_updates_with(state.clone(), ApplyRequest { components: vec![key.clone()], untouched_only: true }).await.into_response(),
    )
    .await
    .body;
    drop(starts);
    if let Some(errors) = applied.get("errors").and_then(Value::as_array).filter(|e| !e.is_empty()) {
        state.manager.write().await.unstage_addon(staged);
        record(previous);
        let code = hand_modified(&key).unwrap_or_else(|| "apply_failed".to_string());
        return addon_refusal(StatusCode::CONFLICT, &code, errors.iter().filter_map(Value::as_str).collect::<Vec<_>>().join("; "));
    }
    answer("applied", Vec::new())
}
