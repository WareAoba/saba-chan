//! 폴더 위치 API
//!
//! - `GET /api/system/paths` — 데이터·설치·인스턴스·모듈·익스텐션 폴더의 지금 값과 기본값, 모듈·익스텐션 폴더의 출처
//! - `POST /api/system/paths/move` `{kind: "modules"|"extensions", target}` — 고른 폴더 아래로 옮긴다
//!   (`saba_chan_updater_lib::component_dirs`). 실패는 HTTP 200 `{ok: false, error_code: "paths_<PathError 코드>", error}`
//! - `GET /api/system/browse?path=&files=` — 이 PC의 폴더 안(폴더와 `files` 확장자의 파일). 원격 데몬을 다룰 때 GUI가 그 PC의 경로를 고른다(D2)

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
    let install_dir = std::env::current_exe()
        .ok()
        .and_then(|exe| exe.parent().map(Path::to_path_buf));
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
    (
        StatusCode::OK,
        Json(json!({ "ok": false, "error_code": code, "error": e.detail })),
    )
        .into_response()
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
    // 요청이 끊겨도(클라이언트 종료 등) 잠금부터 매니저 갱신까지 끝내도록 따로 돌린다:
    // 파일과 설정만 옮겨지고 모듈·익스텐션·업데이터가 옛 폴더를 보는 일이 없게
    let chosen = PathBuf::from(req.target.trim());
    let task = tokio::spawn(async move { move_component_dir(&state, kind, &chosen, Layout::current(), config).await });
    let moved = match task.await {
        Ok(moved) => moved,
        Err(e) => Err(PathError::new("copy_failed", e.to_string())),
    };
    match moved {
        Ok((path, moved)) => (
            StatusCode::OK,
            Json(json!({ "ok": true, "path": path, "left_behind": moved.left_behind })),
        )
            .into_response(),
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
    // 설치 폴더 안은 받지 않는다: 다음 설치가 그 설치 폴더를 거절하고, 제거기가 설치 폴더째 지운다 (설치기와 같은 규칙)
    if let Some(install) = std::env::current_exe().ok().and_then(|exe| exe.parent().map(Path::to_path_buf)) {
        let real = component_dirs::real_path;
        if saba_chan_updater_lib::paths_overlap(&install, &target) || saba_chan_updater_lib::paths_overlap(&real(&install), &real(&target)) {
            return Err(PathError::new("overlaps_install", target.display().to_string()));
        }
    }
    let source = layout.dir(kind).to_path_buf();

    let Ok(_dirs) = IN_USE.try_write() else {
        return Err(busy("install"));
    };
    // 업데이트를 받거나 적용하는 중이면 그 작업이 매니저를 쥐고 있다: 기다리지 않고 거절한다
    let Ok(mut updates) = state.update_state.manager.try_write() else {
        return Err(busy("update"));
    };
    let mut supervisor = state.supervisor.write().await;
    let running = supervisor.running_instances().await;
    if !running.is_empty() {
        return Err(PathError::new("servers_running", running.join(", ")));
    }
    // 백업·설치·초기화 같은 인스턴스 작업도 모듈을 부른다 (모듈 설치·제거는 `IN_USE`가 막는다)
    let instance_ids: Vec<String> = supervisor.instance_store.list().iter().map(|i| i.id.clone()).collect();
    if let Some(id) = instance_ids
        .iter()
        .find(|id| crate::instance::lock::InstanceLock::held(id))
    {
        return Err(PathError::new("busy", format!("instance {}", id)));
    }
    let mut extensions = state.extension_manager.write().await;

    let (src, dst) = (source.clone(), target.clone());
    let moved = tokio::task::spawn_blocking(move || {
        component_dirs::relocate_component(kind, &src, &dst, &layout.data_dir, || {
            component_dirs::commit_move(&config, kind, &src, &dst, &layout.data_dir)
        })
    })
    .await
    .map_err(|e| PathError::new("copy_failed", e.to_string()))??;

    tracing::info!(
        "[Paths] Moved {} from {} to {}",
        kind.dir_name(),
        source.display(),
        target.display()
    );
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

#[derive(Deserialize)]
pub struct BrowseQuery {
    #[serde(default)]
    path: String,
    /// 보일 파일 확장자 (쉼표로, `*`면 모두). 없으면 폴더만
    files: Option<String>,
}

/// 한 폴더에서 보이는 항목 상한 (넘으면 `truncated`)
const BROWSE_MAX: usize = 5000;

fn browse_failed(code: &str) -> serde_json::Value {
    json!({ "ok": false, "error_code": code })
}

fn home_dir() -> Option<PathBuf> {
    std::env::var_os(if cfg!(windows) { "USERPROFILE" } else { "HOME" }).map(PathBuf::from)
}

/// 시작 자리: Windows는 있는 드라이브, 그 밖은 `/`
fn roots() -> Vec<PathBuf> {
    if cfg!(windows) {
        (b'A'..=b'Z').map(|d| PathBuf::from(format!("{}:\\", d as char))).filter(|p| p.exists()).collect()
    } else {
        vec![PathBuf::from("/")]
    }
}

/// 고르기 목록에서 뺄 항목: `.`으로 시작하는 이름, Windows 숨김·시스템 속성 (`$Recycle.Bin`·`System Volume Information` 등).
/// 경로를 직접 적으면 열 수 있다
fn hidden(entry: &std::fs::DirEntry, name: &str) -> bool {
    #[cfg(windows)]
    {
        use std::os::windows::fs::MetadataExt;
        // FILE_ATTRIBUTE_HIDDEN | FILE_ATTRIBUTE_SYSTEM
        if entry.metadata().is_ok_and(|m| m.file_attributes() & (0x2 | 0x4) != 0) {
            return true;
        }
    }
    #[cfg(not(windows))]
    let _ = entry;
    name.starts_with('.')
}

pub(crate) fn browse_dir(path: &str, files: Option<&str>) -> serde_json::Value {
    let home = home_dir();
    if path.trim().is_empty() {
        let entries: Vec<_> = roots()
            .into_iter()
            .map(|p| json!({ "name": p.to_string_lossy(), "path": p, "dir": true }))
            .collect();
        return json!({ "ok": true, "path": null, "parent": null, "home": home, "sep": std::path::MAIN_SEPARATOR_STR, "entries": entries });
    }
    let dir = PathBuf::from(path);
    if !dir.is_absolute() || dir.components().any(|c| matches!(c, std::path::Component::ParentDir)) {
        return browse_failed("browse_bad_path");
    }
    let read = match std::fs::read_dir(&dir) {
        Ok(read) => read,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return browse_failed("browse_not_found"),
        Err(e) if e.kind() == std::io::ErrorKind::PermissionDenied => return browse_failed("browse_denied"),
        Err(_) => return browse_failed("browse_not_a_folder"),
    };
    let wanted: Option<Vec<String>> =
        files.map(|f| f.split(',').map(|e| e.trim().trim_start_matches('.').to_ascii_lowercase()).filter(|e| !e.is_empty()).collect());
    let mut entries: Vec<(bool, String, PathBuf)> = Vec::new();
    let mut truncated = false;
    for entry in read.flatten() {
        let name = entry.file_name().to_string_lossy().to_string();
        if hidden(&entry, &name) {
            continue;
        }
        let is_dir = entry.file_type().map(|t| t.is_dir()).unwrap_or(false) || entry.path().is_dir();
        let shown = is_dir
            || wanted.as_ref().is_some_and(|w| {
                w.iter().any(|e| e == "*")
                    || Path::new(&name).extension().is_some_and(|x| w.contains(&x.to_string_lossy().to_ascii_lowercase()))
            });
        if !shown {
            continue;
        }
        if entries.len() >= BROWSE_MAX {
            truncated = true;
            break;
        }
        entries.push((is_dir, name, entry.path()));
    }
    entries.sort_by(|a, b| b.0.cmp(&a.0).then_with(|| a.1.to_lowercase().cmp(&b.1.to_lowercase())));
    let entries: Vec<_> = entries.into_iter().map(|(dir, name, path)| json!({ "name": name, "path": path, "dir": dir })).collect();
    json!({
        "ok": true,
        "path": dir,
        "parent": dir.parent(),
        "home": home,
        "sep": std::path::MAIN_SEPARATOR_STR,
        "entries": entries,
        "truncated": truncated,
    })
}

/// GET /api/system/browse
pub async fn browse(axum::extract::Query(q): axum::extract::Query<BrowseQuery>) -> impl IntoResponse {
    let value = tokio::task::spawn_blocking(move || browse_dir(&q.path, q.files.as_deref()))
        .await
        .unwrap_or_else(|_| browse_failed("browse_failed"));
    Json(value)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Arc;
    use tokio::sync::RwLock;

    /// 원격 데몬을 다룰 때 GUI가 그 PC의 폴더·파일을 고른다 (D2): 폴더는 늘, 파일은 고른 확장자만
    #[test]
    fn browsing_lists_folders_and_only_the_asked_files() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::create_dir(dir.path().join("Worlds")).unwrap();
        std::fs::write(dir.path().join("backup.ZIP"), b"z").unwrap();
        std::fs::write(dir.path().join("notes.txt"), b"t").unwrap();
        let path = dir.path().to_string_lossy().to_string();

        let folders = browse_dir(&path, None);
        assert_eq!(folders["ok"], true, "{folders}");
        let names = |v: &serde_json::Value| -> Vec<(String, bool)> {
            v["entries"].as_array().unwrap().iter().map(|e| (e["name"].as_str().unwrap().to_string(), e["dir"].as_bool().unwrap())).collect()
        };
        assert_eq!(names(&folders), vec![("Worlds".to_string(), true)]);
        assert_eq!(folders["parent"].as_str().map(PathBuf::from), dir.path().parent().map(Path::to_path_buf));

        let zips = browse_dir(&path, Some("zip"));
        assert_eq!(names(&zips), vec![("Worlds".to_string(), true), ("backup.ZIP".to_string(), false)]);
        assert_eq!(names(&browse_dir(&path, Some("*"))).len(), 3);
        assert_eq!(
            zips["entries"][1]["path"].as_str().map(PathBuf::from),
            Some(dir.path().join("backup.ZIP"))
        );
    }

    /// 숨긴 항목(`.`으로 시작, Windows 숨김·시스템 속성. `$Recycle.Bin`·`System Volume Information` 등)은 고르기 목록에 보이지 않는다
    #[test]
    fn browsing_leaves_out_hidden_entries() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::create_dir(dir.path().join(".cache")).unwrap();
        std::fs::create_dir(dir.path().join("Worlds")).unwrap();
        std::fs::write(dir.path().join(".env.zip"), "").unwrap();
        #[cfg(windows)]
        {
            use std::os::windows::ffi::OsStrExt;
            std::fs::create_dir(dir.path().join("System Volume Information")).unwrap();
            let wide: Vec<u16> = dir.path().join("System Volume Information").as_os_str().encode_wide().chain([0]).collect();
            // FILE_ATTRIBUTE_HIDDEN | FILE_ATTRIBUTE_SYSTEM
            assert_ne!(unsafe { windows_sys::Win32::Storage::FileSystem::SetFileAttributesW(wide.as_ptr(), 0x2 | 0x4) }, 0);
        }
        let listed = browse_dir(&dir.path().to_string_lossy(), Some("*"));
        let names: Vec<&str> = listed["entries"].as_array().unwrap().iter().map(|e| e["name"].as_str().unwrap()).collect();
        assert_eq!(names, vec!["Worlds"]);
    }

    #[test]
    fn browsing_starts_at_the_roots_and_says_why_it_cannot_open_a_folder() {
        let roots = browse_dir("", None);
        assert_eq!(roots["ok"], true);
        assert!(roots["path"].is_null());
        assert!(!roots["entries"].as_array().unwrap().is_empty());
        assert_eq!(browse_dir("relative/dir", None)["error_code"], "browse_bad_path");
        let dir = tempfile::tempdir().unwrap();
        let gone = dir.path().join("gone").to_string_lossy().to_string();
        assert_eq!(browse_dir(&gone, None)["error_code"], "browse_not_found");
    }

    fn module(root: &Path, name: &str) {
        let dir = root.join(name);
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(
            dir.join("module.toml"),
            format!(
                "[module]\nname = \"{}\"\nversion = \"1.0.0\"\nentry = \"lifecycle.py\"\n",
                name
            ),
        )
        .unwrap();
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
        let state = IPCServer::new(
            Arc::new(RwLock::new(sup)),
            "127.0.0.1:0",
            crate::daemon_log::DaemonLogBuffer::new(),
        );
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
            let err = move_component_dir(&state, Kind::Modules, &chosen, layout.clone(), config.clone())
                .await
                .unwrap_err();
            assert_eq!((err.code, err.detail.as_str()), ("busy", "install"));
        }
        {
            let _updating = state.update_state.manager.write().await;
            let err = move_component_dir(&state, Kind::Modules, &chosen, layout.clone(), config.clone())
                .await
                .unwrap_err();
            assert_eq!((err.code, err.detail.as_str()), ("busy", "update"));
        }
        // Codex 리뷰 #544: 설치 폴더(데몬 실행 파일 폴더) 안은 설치기처럼 거절한다
        let install = std::env::current_exe().unwrap().parent().unwrap().join("games");
        let err = move_component_dir(&state, Kind::Modules, &install, layout.clone(), config.clone()).await.unwrap_err();
        assert_eq!(err.code, "overlaps_install");
        assert!(old.join("pathgame").join("module.toml").is_file());
        assert!(!chosen.exists());
        assert_eq!(
            config.peek_section(component_dirs::PATHS_SECTION),
            serde_json::Value::Null
        );

        let (path, moved) = move_component_dir(&state, Kind::Modules, &chosen, layout, config.clone())
            .await
            .unwrap();
        assert_eq!(path, chosen.join("modules"));
        assert!(moved.left_behind.is_empty());
        assert!(!old.exists());
        let sup = state.supervisor.read().await;
        assert_eq!(Path::new(sup.module_loader.modules_dir()), path);
        assert!(
            sup.list_modules()
                .unwrap()
                .iter()
                .any(|m| m.metadata.name == "pathgame"),
            "modules are read from the new folder"
        );
        assert_eq!(
            config.peek_section(component_dirs::PATHS_SECTION)["modules"],
            json!(path.to_string_lossy())
        );
        // 나중에 받는 모듈 업데이트도 새 폴더에 푼다
        let module = saba_chan_updater_lib::Component::Module("pathgame".into());
        assert_eq!(
            state
                .update_state
                .manager
                .read()
                .await
                .resolve_install_dir_for_test(&module, None),
            path.join("pathgame")
        );
    }
}
