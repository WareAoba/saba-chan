//! 폴더 위치 API
//!
//! - `GET /api/system/paths` — 데이터·설치·인스턴스·모듈·익스텐션 폴더의 지금 값과 기본값, 모듈·익스텐션 폴더의 출처
//! - `POST /api/system/paths/move` `{kind: "modules"|"extensions", target}` — 고른 폴더 아래로 옮긴다
//!   (`saba_chan_updater_lib::component_dirs`). 실패는 HTTP 200 `{ok: false, error_code: "paths_<PathError 코드>", error}`

use axum::{extract::State, http::StatusCode, response::IntoResponse, Json};
use saba_chan_updater_lib::component_dirs::{self, Kind, Layout, PathError};
use saba_chan_updater_lib::constants;
use serde::Deserialize;
use serde_json::json;
use std::path::{Path, PathBuf};

use super::super::IPCServer;

/// 모듈·익스텐션 폴더에 파일을 쓰는 작업(설치·제거)과 폴더 옮기기를 가른다.
/// 작업은 읽기 잠금을 쥐고 하고, 옮기기는 쓰기 잠금을 바로 잡지 못하면 거절한다
static IN_USE: tokio::sync::RwLock<()> = tokio::sync::RwLock::const_new(());

/// 모듈·익스텐션 폴더에 쓰는 동안 쥔다 (옮기는 중이면 끝나기를 기다린다)
pub(crate) async fn components_in_use() -> tokio::sync::RwLockReadGuard<'static, ()> {
    IN_USE.read().await
}

fn dir_json(kind: Kind, data_dir: &Path) -> serde_json::Value {
    let (path, source) = component_dirs::resolve(kind);
    json!({ "path": path, "default": component_dirs::default_dir(kind, data_dir), "source": source })
}

/// GET /api/system/paths
pub async fn get_paths(State(_state): State<IPCServer>) -> impl IntoResponse {
    let data_dir = constants::resolve_data_dir();
    let install_dir = std::env::current_exe().ok().and_then(|exe| exe.parent().map(Path::to_path_buf));
    Json(json!({
        "ok": true,
        "data_dir": data_dir,
        "install_dir": install_dir,
        "default_install_dir": constants::default_install_dir(),
        "instances_dir": constants::resolve_instances_dir(),
        "modules": dir_json(Kind::Modules, &data_dir),
        "extensions": dir_json(Kind::Extensions, &data_dir),
    }))
}

#[derive(Deserialize)]
pub struct MoveRequest {
    kind: String,
    target: String,
}

fn refused(e: PathError) -> axum::response::Response {
    let code = format!("paths_{}", e.code);
    (StatusCode::OK, Json(json!({ "ok": false, "error_code": code, "error": e.detail }))).into_response()
}

/// POST /api/system/paths/move
pub async fn move_dir(State(state): State<IPCServer>, Json(req): Json<MoveRequest>) -> impl IntoResponse {
    let Some(kind) = Kind::parse(&req.kind) else {
        return refused(PathError::new("unknown_kind", req.kind));
    };
    if component_dirs::resolve(kind).1 == component_dirs::Source::Env {
        return refused(PathError::new("env_fixed", kind.env_var()));
    }
    let config = saba_chan_updater_lib::config_file::ConfigFile::default_location();
    match move_component_dir(&state, kind, Path::new(req.target.trim()), Layout::current(), config).await {
        Ok((path, moved)) => {
            (StatusCode::OK, Json(json!({ "ok": true, "path": path, "left_behind": moved.left_behind }))).into_response()
        }
        Err(e) => refused(e),
    }
}

/// 옮기기 본체. 무엇도 돌지 않을 때만, 폴더를 쓰는 모든 것을 잠근 채 옮기고 새 폴더를 다시 읽는다
async fn move_component_dir(
    state: &IPCServer,
    kind: Kind,
    chosen: &Path,
    layout: Layout,
    config: saba_chan_updater_lib::config_file::ConfigFile,
) -> Result<(PathBuf, component_dirs::Relocated), PathError> {
    let busy = |what: &str| PathError::new("busy", what.to_string());
    let target = component_dirs::target_for(kind, chosen, &layout)?;
    let source = layout.dir(kind).to_path_buf();

    let Ok(_dirs) = IN_USE.try_write() else { return Err(busy("install")) };
    // 업데이트를 받거나 적용하는 중이면 그 작업이 매니저를 쥐고 있다: 기다리지 않고 거절한다
    let Ok(mut updates) = state.update_state.manager.try_write() else { return Err(busy("update")) };
    let mut supervisor = state.supervisor.write().await;
    let running = supervisor.running_instances().await;
    if !running.is_empty() {
        return Err(PathError::new("servers_running", running.join(", ")));
    }
    // 백업·설치·초기화 같은 인스턴스 작업도 모듈을 부른다 (모듈 설치·제거는 `IN_USE`가 막는다)
    let instance_ids: Vec<String> = supervisor.instance_store.list().iter().map(|i| i.id.clone()).collect();
    if let Some(id) = instance_ids.iter().find(|id| crate::instance::lock::InstanceLock::held(id)) {
        return Err(PathError::new("busy", format!("instance {}", id)));
    }
    let mut extensions = state.extension_manager.write().await;

    let (src, dst) = (source.clone(), target.clone());
    let moved = tokio::task::spawn_blocking(move || {
        component_dirs::relocate(&src, &dst, || component_dirs::save_choice(&config, kind, &dst, &layout.data_dir))
    })
    .await
    .map_err(|e| PathError::new("copy_failed", e.to_string()))??;

    tracing::info!("[Paths] Moved {} from {} to {}", kind.dir_name(), source.display(), target.display());
    match kind {
        Kind::Modules => {
            supervisor.module_loader.set_modules_dir(&target.to_string_lossy());
            if let Err(e) = supervisor.refresh_modules() {
                tracing::warn!("[Paths] Could not reload modules from {}: {}", target.display(), e);
            }
            supervisor.migrate_list_settings();
        }
        Kind::Extensions => {
            if let Err(e) = extensions.set_extensions_dir(target.clone()) {
                tracing::warn!("[Paths] Could not rescan extensions in {}: {}", target.display(), e);
            }
        }
    }
    updates.set_component_dir(kind, &target);
    for left in &moved.left_behind {
        tracing::warn!("[Paths] Could not remove {} after the move", left.display());
    }
    Ok((target, moved))
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Arc;
    use tokio::sync::RwLock;

    fn module(root: &Path, name: &str) {
        let dir = root.join(name);
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(dir.join("module.toml"), format!("[module]\nname = \"{}\"\nversion = \"1.0.0\"\nentry = \"lifecycle.py\"\n", name)).unwrap();
        std::fs::write(dir.join("lifecycle.py"), "").unwrap();
    }

    async fn setup(tmp: &Path) -> (IPCServer, Layout, saba_chan_updater_lib::config_file::ConfigFile) {
        let data = tmp.join("data");
        let layout = Layout {
            data_dir: data.clone(),
            home: None,
            modules: data.join("modules"),
            extensions: data.join("extensions"),
        };
        module(&layout.modules, "pathgame");
        let mut sup = crate::supervisor::Supervisor::new_with_instances_dir(
            layout.modules.to_str().unwrap(),
            data.join("instances").to_str().unwrap(),
        );
        sup.initialize().await.unwrap();
        let state = IPCServer::new(Arc::new(RwLock::new(sup)), "127.0.0.1:0", crate::daemon_log::DaemonLogBuffer::new());
        (state, layout, saba_chan_updater_lib::config_file::ConfigFile::at(&data))
    }

    // 옮기기 잠금(`IN_USE`)은 프로세스에 하나라 두 경우를 한 테스트에서 차례로 본다
    #[tokio::test]
    async fn moves_only_when_nothing_uses_the_folder_then_switches_the_daemon() {
        let tmp = tempfile::tempdir().unwrap();
        let (state, layout, config) = setup(tmp.path()).await;
        let old = layout.modules.clone();
        let chosen = tmp.path().join("games");
        {
            let _installing = components_in_use().await;
            let err = move_component_dir(&state, Kind::Modules, &chosen, layout.clone(), config.clone()).await.unwrap_err();
            assert_eq!((err.code, err.detail.as_str()), ("busy", "install"));
        }
        {
            let _updating = state.update_state.manager.write().await;
            let err = move_component_dir(&state, Kind::Modules, &chosen, layout.clone(), config.clone()).await.unwrap_err();
            assert_eq!((err.code, err.detail.as_str()), ("busy", "update"));
        }
        assert!(old.join("pathgame").join("module.toml").is_file());
        assert!(!chosen.exists());
        assert_eq!(config.peek_section(component_dirs::PATHS_SECTION), serde_json::Value::Null);

        let (path, moved) = move_component_dir(&state, Kind::Modules, &chosen, layout, config.clone()).await.unwrap();
        assert_eq!(path, chosen.join("modules"));
        assert!(moved.left_behind.is_empty());
        assert!(!old.exists());
        let sup = state.supervisor.read().await;
        assert_eq!(Path::new(sup.module_loader.modules_dir()), path);
        assert!(sup.list_modules().unwrap().iter().any(|m| m.metadata.name == "pathgame"), "modules are read from the new folder");
        assert_eq!(config.peek_section(component_dirs::PATHS_SECTION)["modules"], json!(path.to_string_lossy()));
        // 나중에 받는 모듈 업데이트도 새 폴더에 푼다
        let module = saba_chan_updater_lib::Component::Module("pathgame".into());
        assert_eq!(state.update_state.manager.read().await.resolve_install_dir_for_test(&module, None), path.join("pathgame"));
    }
}
