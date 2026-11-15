//! 저장 공간 API (설정 › 저장 공간)
//!
//! - `GET /api/system/storage?refresh=` — 내장 디스크와 디스크마다 사바쨩 파일(갈래별), 사바쨩 파일 목록.
//!   크기는 데몬이 백그라운드에서 재고(`measuring`), 다 잰 값을 준다. `refresh=true`면 바로 다시 잰다
//! - `POST /api/system/storage/move` `{kind: "server"|"modules"|"extensions", id?, mount?, target?, dryRun?}` —
//!   `mount`(드라이브)를 주면 데몬이 자리를 정하고, `target`(폴더)을 주면 그 폴더로. `dryRun`이면 자리·크기·남은 공간만 돌려준다.
//!   아니면 작업을 걸고 202. 거절은 `storage_<코드>`(서버) · `paths_<코드>`(모듈·익스텐션)
//! - `GET /api/system/storage/move` — 마지막 옮기기 작업 (`crate::storage::MoveJob`, 없으면 null)
//! - `POST /api/system/storage/move/cancel` — 복사·검사 중인 작업을 멈춘다 (원본은 그대로)

use axum::{
    extract::{Query, State},
    http::StatusCode,
    response::{IntoResponse, Response},
    Json,
};
use saba_chan_updater_lib::component_dirs::{self, Kind, Layout};
use saba_chan_updater_lib::constants;
use serde::Deserialize;
use serde_json::{json, Value};
use std::path::{Path, PathBuf};

use super::super::IPCServer;
use crate::storage::{self, Category, MoveJob, Volume};

#[derive(Deserialize, Default)]
pub struct StorageQuery {
    #[serde(default)]
    refresh: bool,
}

fn install_dir() -> Option<PathBuf> {
    std::env::current_exe()
        .ok()
        .and_then(|exe| exe.parent().map(Path::to_path_buf))
}

/// 잴 폴더들
fn plan_of(sup: &crate::supervisor::Supervisor) -> storage::Plan {
    let instances = sup
        .instance_store
        .list()
        .iter()
        .map(|inst| {
            let instance_dir = sup.instance_store.instance_dir(&inst.id);
            let metadata = sup.module_loader.get_module(&inst.module_name).ok().map(|m| m.metadata);
            storage::PlannedInstance {
                id: inst.id.clone(),
                name: inst.name.clone(),
                server_dir: crate::module_paths::server_dir(inst, &instance_dir, metadata.as_ref()),
                instance_dir,
            }
        })
        .collect();
    let instances_dir = sup
        .instance_store
        .state_root()
        .parent()
        .map(Path::to_path_buf)
        .unwrap_or_else(constants::resolve_instances_dir);
    storage::Plan {
        instances,
        modules: Some(component_dirs::resolve(Kind::Modules).0),
        extensions: Some(component_dirs::resolve(Kind::Extensions).0),
        data_dir: constants::resolve_data_dir(),
        install_dir: install_dir(),
        instances_dir,
    }
}

async fn volumes() -> Vec<Volume> {
    tokio::task::spawn_blocking(storage::internal_volumes)
        .await
        .unwrap_or_default()
}

/// GET /api/system/storage
pub async fn get_storage(State(state): State<IPCServer>, Query(q): Query<StorageQuery>) -> Response {
    let (plan, running) = {
        let sup = state.supervisor.read().await;
        (plan_of(&sup), sup.running_instances().await)
    };
    let volumes = volumes().await;
    let mounts = volumes.iter().map(|v| v.mount.clone()).collect();
    let (measured, measuring) = state.storage.get(q.refresh, move || plan, mounts);
    let volumes: Vec<Value> = volumes
        .iter()
        .map(|v| {
            let sums = storage::per_volume(&measured.items, &v.mount);
            let sum = |c| sums.get(&c).copied().unwrap_or(0);
            let mut out = json!(v);
            out["saba"] = json!({
                "servers": sum(Category::Servers),
                "backups": sum(Category::Backups),
                "components": sum(Category::Components),
                "app": sum(Category::App),
            });
            out
        })
        .collect();
    let items: Vec<Value> = measured
        .items
        .iter()
        .map(|item| {
            let mut out = json!(item);
            if item.kind == "server" {
                out["running"] = json!(item.id.as_ref().is_some_and(|id| running.contains(id)));
            }
            out
        })
        .collect();
    Json(json!({
        "success": true,
        "measuring": measuring,
        "measured_at_ms": measured.at_ms,
        "volumes": volumes,
        "items": items,
    }))
    .into_response()
}

#[derive(Deserialize)]
pub struct MoveRequest {
    kind: String,
    id: Option<String>,
    mount: Option<String>,
    target: Option<String>,
    #[serde(default, rename = "dryRun")]
    dry_run: bool,
}

fn refused(code: &str, detail: impl Into<String>) -> Response {
    let status = if code == "storage_bad_request" {
        StatusCode::BAD_REQUEST
    } else {
        StatusCode::CONFLICT
    };
    super::error(status, code, detail)
}

/// 옮길 크기: 잰 값이 있으면 그것, 없으면 지금 잰다
async fn size_of(state: &IPCServer, kind: &str, id: Option<&str>, dir: &Path) -> u64 {
    if let Some(item) = state
        .storage
        .measured()
        .items
        .iter()
        .find(|i| i.kind == kind && i.id.as_deref() == id)
    {
        return item.bytes;
    }
    let dir = dir.to_path_buf();
    tokio::task::spawn_blocking(move || crate::system_monitor::dir_size(&dir))
        .await
        .unwrap_or(0)
}

/// `target`이 들어갈 디스크와 남은 공간
fn volume_for<'a>(volumes: &'a [Volume], path: &Path) -> Option<&'a Volume> {
    let mounts: Vec<String> = volumes.iter().map(|v| v.mount.clone()).collect();
    let mount = crate::system_monitor::mount_for(path, &mounts)?.to_string();
    volumes.iter().find(|v| v.mount == mount)
}

/// 같은 디스크 안의 옮기기는 이름만 바꾼다 (공간이 더 들지 않는다). 아니면 옮긴 뒤에도 `SPARE_BYTES`가 남아야 한다.
/// 들어갈 디스크를 모르면 확인하지 않는다 (`None`, 모자라면 복사가 실패한다). 모자라면 거절의 `detail`
async fn space_check(source: &Path, target: &Path, bytes: u64) -> Result<Option<u64>, String> {
    let (source, target) = (source.to_path_buf(), target.to_path_buf());
    let (from, to) = tokio::task::spawn_blocking(move || {
        let disks = storage::all_disks();
        (storage::space_at(&source, &disks), storage::space_at(&target, &disks))
    })
    .await
    .unwrap_or((None, None));
    let Some((to_mount, available)) = to else {
        return Ok(None);
    };
    let same = from.is_some_and(|(m, _)| m == to_mount);
    if !same && bytes.saturating_add(storage::SPARE_BYTES) > available {
        return Err(format!("{} needed, {} available", bytes, available));
    }
    Ok(Some(available))
}

/// POST /api/system/storage/move
pub async fn start_move(State(state): State<IPCServer>, Json(req): Json<MoveRequest>) -> Response {
    match req.kind.as_str() {
        "server" => start_server_move(state, req).await,
        other => match Kind::parse(other) {
            Some(kind) => start_component_move(state, kind, req).await,
            None => refused("storage_bad_request", format!("unknown kind {}", other)),
        },
    }
}

async fn start_server_move(state: IPCServer, req: MoveRequest) -> Response {
    let Some(id) = req.id.clone() else {
        return refused("storage_bad_request", "id is required");
    };
    let volumes = volumes().await;
    let (instance, instance_dir, server_dir, subdir, others, running) = {
        let sup = state.supervisor.read().await;
        let Some(instance) = sup.instance_store.get(&id).cloned() else {
            return super::error(
                StatusCode::NOT_FOUND,
                "not_found",
                format!("Instance not found: {}", id),
            );
        };
        let instance_dir = sup.instance_store.instance_dir(&id);
        let metadata = sup
            .module_loader
            .get_module(&instance.module_name)
            .ok()
            .map(|m| m.metadata);
        let server_dir = crate::module_paths::server_dir(&instance, &instance_dir, metadata.as_ref());
        let subdir = metadata
            .as_ref()
            .and_then(|m| m.install.as_ref())
            .and_then(|i| i.install_subdir.clone())
            .unwrap_or_else(|| "server".to_string());
        let others = crate::service::instance::other_instance_folders(&sup, &id);
        let running = sup.is_running(&id).await;
        (instance, instance_dir, server_dir, subdir, others, running)
    };
    let target = match (
        req.target.as_deref().map(str::trim).filter(|t| !t.is_empty()),
        req.mount.as_deref(),
    ) {
        (Some(t), _) => PathBuf::from(t),
        (None, Some(mount)) => {
            // 직접 고른 폴더·가져온 서버는 인스턴스 폴더 안으로 들이지 않는다 (plan_server_move가 거절한다)
            let linked = crate::instance::migration::linked_folder(Some(&server_dir.to_string_lossy()), &instance_dir).is_some();
            let instance_mount = (!linked)
                .then(|| volume_for(&volumes, &instance_dir).map(|v| PathBuf::from(&v.mount)))
                .flatten();
            storage::default_server_target(
                Path::new(mount),
                &instance_dir,
                instance_mount.as_deref(),
                &subdir,
                &instance.name,
                &id,
            )
        }
        (None, None) => return refused("storage_bad_request", "mount or target is required"),
    };
    let plan = match storage::plan_server_move(&server_dir, &instance_dir, &target, &others, install_dir().as_deref()) {
        Ok(plan) => plan,
        Err(e) => return refused(&format!("storage_{}", e.code), e.detail),
    };
    if running {
        return refused("storage_server_running", instance.name);
    }
    let bytes = size_of(&state, "server", Some(&id), &server_dir).await;
    let available = match space_check(&plan.source, &plan.target, bytes).await {
        Ok(a) => a,
        Err(detail) => return refused("storage_no_space", detail),
    };
    if req.dry_run {
        return Json(json!({ "success": true, "target": plan.target, "bytes": bytes, "available_bytes": available }))
            .into_response();
    }

    let Some(lock) = crate::instance::lock::InstanceLock::take_for_writes(&id).await else {
        return super::backup::instance_busy();
    };
    let job = MoveJob {
        kind: "server".into(),
        id: Some(id.clone()),
        target: plan.target.clone(),
        phase: "copy",
        done_bytes: 0,
        total_bytes: bytes,
        error_code: None,
        error: None,
        left_behind: Vec::new(),
    };
    if !state.storage.begin(job.clone()) {
        return refused("storage_busy", "another move is running");
    }
    // 요청이 끊겨도 끝까지 한다 (옮긴 파일과 인스턴스 설정이 어긋나지 않게)
    let runner = state.clone();
    tokio::spawn(async move {
        let _lock = lock;
        let result = run_server_move(&runner, &id, &plan).await;
        finish(&runner, result);
    });
    (StatusCode::ACCEPTED, Json(json!({ "success": true, "job": job }))).into_response()
}

/// 서버 폴더를 옮기고 인스턴스 경로를 바꾼다. 잠금(`InstanceLock`)을 쥔 채 부른다
async fn run_server_move(
    state: &IPCServer,
    id: &str,
    plan: &storage::ServerMove,
) -> Result<Vec<PathBuf>, component_dirs::PathError> {
    // 잠금을 잡는 사이 시작했을 수 있다
    if state.supervisor.read().await.is_running(id).await {
        return Err(component_dirs::PathError::new("server_running", id.to_string()));
    }
    let (state2, id2, plan2) = (state.clone(), id.to_string(), plan.clone());
    let handle = tokio::runtime::Handle::current();
    let moved = tokio::task::spawn_blocking(move || {
        let progress = state2.storage.clone();
        let watch = move |phase: component_dirs::Phase, bytes: u64| {
            progress.update(|job| {
                job.phase = match phase {
                    component_dirs::Phase::Copy => "copy",
                    component_dirs::Phase::Verify => "verify",
                };
                job.done_bytes = bytes;
            });
            !progress.canceled()
        };
        component_dirs::relocate_watched(
            &plan2.source,
            &plan2.target,
            || {
                handle.block_on(async {
                    let mut sup = state2.supervisor.write().await;
                    let mut instance = sup
                        .instance_store
                        .get(&id2)
                        .cloned()
                        .ok_or_else(|| std::io::Error::other("instance removed"))?;
                    storage::rebase_instance(&mut instance, &plan2.source, &plan2.target);
                    sup.instance_store
                        .update(&id2, instance)
                        .map_err(|e| std::io::Error::other(e.to_string()))
                })
            },
            &watch,
        )
    })
    .await
    .map_err(|e| component_dirs::PathError::new("copy_failed", e.to_string()))??;
    // 직접 고른 폴더·가져온 서버는 옮겨도 사용자 것이다: 사바쨩 표시를 떼서 인스턴스를 지울 때 남긴다
    if plan.linked {
        let marker = plan.target.join(component_dirs::OWNED_MARKER);
        if let Err(e) = std::fs::remove_file(&marker)
            && e.kind() != std::io::ErrorKind::NotFound
        {
            tracing::warn!("[Storage] Could not remove {}: {}", marker.display(), e);
        }
    }
    tracing::info!(
        "[Storage] Moved the server folder of {} from {} to {}",
        id,
        plan.source.display(),
        plan.target.display()
    );
    Ok(moved.left_behind)
}

fn finish(state: &IPCServer, result: Result<Vec<PathBuf>, component_dirs::PathError>) {
    state.storage.invalidate();
    state.storage.update(|job| match result {
        Ok(left_behind) => {
            job.phase = "done";
            job.done_bytes = job.total_bytes;
            job.left_behind = left_behind;
        }
        Err(e) => {
            if e.code != "canceled" {
                tracing::warn!(
                    "[Storage] Could not move to {}: {} {}",
                    job.target.display(),
                    e.code,
                    e.detail
                );
            }
            job.phase = if e.code == "canceled" { "canceled" } else { "failed" };
            job.error_code = Some(e.code.to_string());
            job.error = Some(e.detail);
        }
    });
}

async fn start_component_move(state: IPCServer, kind: Kind, req: MoveRequest) -> Response {
    if component_dirs::resolve(kind).1 == component_dirs::Source::Env {
        return refused("paths_env_fixed", kind.env_var());
    }
    let layout = Layout::current();
    let volumes = volumes().await;
    // 드라이브를 고르면: 데이터 폴더가 있는 드라이브면 기본 자리, 아니면 `<드라이브>/saba-chan`
    let chosen = match (
        req.target.as_deref().map(str::trim).filter(|t| !t.is_empty()),
        req.mount.as_deref(),
    ) {
        (Some(t), _) => PathBuf::from(t),
        (None, Some(mount)) => {
            let data_mount = volume_for(&volumes, &layout.data_dir).map(|v| v.mount.clone());
            if data_mount.as_deref() == Some(mount) {
                layout.data_dir.clone()
            } else {
                Path::new(mount).join(constants::APP_NAME)
            }
        }
        (None, None) => return refused("storage_bad_request", "mount or target is required"),
    };
    let target = match component_dirs::target_for(kind, &chosen, &layout) {
        Ok(t) => t,
        Err(e) => return refused(&format!("paths_{}", e.code), e.detail),
    };
    let source = layout.dir(kind).to_path_buf();
    let bytes = size_of(&state, kind.dir_name(), None, &source).await;
    let available = match space_check(&source, &target, bytes).await {
        Ok(a) => a,
        Err(detail) => return refused("storage_no_space", detail),
    };
    if req.dry_run {
        return Json(json!({ "success": true, "target": target, "bytes": bytes, "available_bytes": available }))
            .into_response();
    }
    let job = MoveJob {
        kind: kind.dir_name().into(),
        id: None,
        target: target.clone(),
        phase: "moving",
        done_bytes: 0,
        total_bytes: bytes,
        error_code: None,
        error: None,
        left_behind: Vec::new(),
    };
    if !state.storage.begin(job.clone()) {
        return refused("storage_busy", "another move is running");
    }
    let runner = state.clone();
    tokio::spawn(async move {
        let config = saba_chan_updater_lib::config_file::ConfigFile::default_location();
        let result = super::paths::move_component_dir(&runner, kind, &chosen, layout, config)
            .await
            .map(|(_, moved)| moved.left_behind);
        finish(&runner, result);
    });
    (StatusCode::ACCEPTED, Json(json!({ "success": true, "job": job }))).into_response()
}

/// GET /api/system/storage/move
pub async fn get_move(State(state): State<IPCServer>) -> Response {
    Json(json!({ "success": true, "job": state.storage.job() })).into_response()
}

/// POST /api/system/storage/move/cancel
pub async fn cancel_move(State(state): State<IPCServer>) -> Response {
    state.storage.cancel();
    Json(json!({ "success": true })).into_response()
}

#[cfg(test)]
mod tests {
    use super::*;
    use axum::extract::{Path as AxPath, Query as AxQuery, State as AxState};

    async fn state_with_server(tmp: &Path) -> (IPCServer, String, PathBuf) {
        let modules = tmp.join("modules");
        std::fs::create_dir_all(modules.join("game")).unwrap();
        std::fs::write(
            modules.join("game").join("module.toml"),
            "[module]\nschema_version = 1\nname = \"game\"\nversion = \"1.0.0\"\nentry = \"lifecycle.py\"\n",
        )
        .unwrap();
        std::fs::write(modules.join("game").join("lifecycle.py"), "").unwrap();
        let mut sup = crate::supervisor::Supervisor::new_with_instances_dir(
            modules.to_str().unwrap(),
            tmp.join("instances").to_str().unwrap(),
        );
        for module in sup.module_loader.discover_modules().unwrap() {
            crate::sandbox::consent_test_component(Path::new(&module.entry_path()));
        }
        sup.initialize().await.unwrap();
        let mut inst = crate::instance::ServerInstance::new("생존", "game");
        let id = inst.id.clone();
        let server = sup.instance_store.instance_dir(&id).join("server");
        std::fs::create_dir_all(server.join("world")).unwrap();
        std::fs::write(server.join("world").join("level.dat"), vec![7u8; 4096]).unwrap();
        std::fs::write(server.join("server.jar"), b"jar").unwrap();
        inst.working_dir = Some(server.to_string_lossy().into_owned());
        inst.executable_path = Some(server.join("server.jar").to_string_lossy().into_owned());
        sup.instance_store.add(inst).unwrap();
        let state = IPCServer::new(
            std::sync::Arc::new(tokio::sync::RwLock::new(sup)),
            "127.0.0.1:0",
            crate::daemon_log::DaemonLogBuffer::new(),
        );
        (state, id, server)
    }

    async fn call(state: &IPCServer, body: Value) -> crate::ipc::internal::InternalResponse {
        let req: MoveRequest = serde_json::from_value(body).unwrap();
        crate::ipc::internal::into_internal(start_move(AxState(state.clone()), Json(req)).await).await
    }

    async fn wait_job(state: &IPCServer) -> MoveJob {
        for _ in 0..200 {
            if let Some(job) = state.storage.job().filter(MoveJob::finished) {
                return job;
            }
            tokio::time::sleep(std::time::Duration::from_millis(20)).await;
        }
        panic!("move did not finish: {:?}", state.storage.job());
    }

    /// 서버 폴더를 다른 자리로 옮기면 파일과 인스턴스 경로가 함께 가고, 사바쨩 것으로 남아 인스턴스와 함께 지워진다
    #[tokio::test]
    async fn a_server_folder_moves_with_its_instance_paths_and_is_deleted_with_it() {
        let tmp = tempfile::tempdir().unwrap();
        let (state, id, server) = state_with_server(tmp.path()).await;
        let target = tmp.path().join("d").join("saba-chan").join("servers").join("생존");

        let preview = call(
            &state,
            json!({ "kind": "server", "id": id, "target": target, "dryRun": true }),
        )
        .await;
        assert_eq!(preview.status, 200, "{}", preview.body);
        assert_eq!(preview.body["target"], json!(target));
        assert_eq!(preview.body["bytes"], json!(4096 + 3));
        assert!(server.join("server.jar").is_file(), "a dry run changes nothing");

        let started = call(&state, json!({ "kind": "server", "id": id, "target": target })).await;
        assert_eq!(started.status, 202, "{}", started.body);
        let job = wait_job(&state).await;
        assert_eq!((job.phase, job.done_bytes), ("done", 4096 + 3), "{:?}", job);

        let inst = state.supervisor.read().await.instance_store.get(&id).cloned().unwrap();
        assert_eq!(inst.working_dir.as_deref(), Some(target.to_string_lossy().as_ref()));
        assert_eq!(
            inst.executable_path.as_deref(),
            Some(target.join("server.jar").to_string_lossy().as_ref())
        );
        assert_eq!(
            std::fs::read(target.join("world").join("level.dat")).unwrap().len(),
            4096
        );
        assert!(!server.exists());
        // 화면의 "연결된 폴더"가 아니다 (지우면 함께 지운다)
        let instance_dir = state.supervisor.read().await.instance_store.instance_dir(&id);
        assert_eq!(
            crate::instance::migration::linked_folder(inst.working_dir.as_deref(), &instance_dir),
            None
        );

        let deleted = crate::ipc::internal::into_internal(
            super::super::instance::delete_instance(
                AxPath(id.clone()),
                AxState(state.clone()),
                AxQuery(Default::default()),
            )
            .await
            .into_response(),
        )
        .await;
        assert_eq!(deleted.status, 200, "{}", deleted.body);
        assert!(!target.exists(), "the moved server folder goes with the instance");
    }

    /// 사용자가 고른 폴더(사바쨩 표시 없음)는 옮겨도 사용자 것이라 인스턴스를 지워도 남는다
    #[tokio::test]
    async fn a_folder_the_user_picked_stays_theirs_after_a_move() {
        let tmp = tempfile::tempdir().unwrap();
        let (state, id, _) = state_with_server(tmp.path()).await;
        let picked = tmp.path().join("mine");
        std::fs::create_dir_all(&picked).unwrap();
        std::fs::write(picked.join("server.jar"), b"jar").unwrap();
        {
            let mut sup = state.supervisor.write().await;
            let mut inst = sup.instance_store.get(&id).cloned().unwrap();
            inst.working_dir = Some(picked.to_string_lossy().into_owned());
            inst.executable_path = None;
            sup.instance_store.update(&id, inst).unwrap();
        }
        let target = tmp.path().join("d").join("mine");
        let started = call(&state, json!({ "kind": "server", "id": id, "target": target })).await;
        assert_eq!(started.status, 202, "{}", started.body);
        assert_eq!(wait_job(&state).await.phase, "done");
        assert!(!target.join(component_dirs::OWNED_MARKER).exists());

        let deleted = crate::ipc::internal::into_internal(
            super::super::instance::delete_instance(
                AxPath(id.clone()),
                AxState(state.clone()),
                AxQuery(Default::default()),
            )
            .await
            .into_response(),
        )
        .await;
        assert_eq!(deleted.status, 200, "{}", deleted.body);
        assert!(target.join("server.jar").is_file());
    }

    #[tokio::test]
    async fn a_move_is_refused_before_anything_changes() {
        let tmp = tempfile::tempdir().unwrap();
        let (state, id, server) = state_with_server(tmp.path()).await;
        let into_itself = call(
            &state,
            json!({ "kind": "server", "id": id, "target": server.join("x") }),
        )
        .await;
        assert_eq!(
            (into_itself.status.as_u16(), into_itself.body["error_code"].clone()),
            (409, json!("storage_overlaps_current"))
        );
        let nothing = call(&state, json!({ "kind": "server", "id": id })).await;
        assert_eq!(nothing.status, 400);
        let unknown = call(&state, json!({ "kind": "server", "id": "nope", "mount": "/" })).await;
        assert_eq!(unknown.status, 404);
        assert!(server.join("server.jar").is_file());
        assert!(state.storage.job().is_none());
    }
}
