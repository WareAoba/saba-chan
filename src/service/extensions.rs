//! 익스텐션 설치와 모듈이 요구하는 익스텐션 맞추기: 설치 API·모듈 설치·업데이트 뒤 기동이 같이 쓴다

use axum::{http::StatusCode, Json};
use serde_json::json;

use crate::extension::{ExtensionError, ExtensionManager};
use crate::ipc::IPCServer;

/// 인스턴스 이름과 extension_data (`running_only`면 실행 중인 것만)
pub(crate) async fn instance_ext_data(state: &IPCServer, running_only: bool) -> Vec<(String, std::collections::HashMap<String, serde_json::Value>)> {
    let sup = state.supervisor.read().await;
    let running_ids = sup.managed_store.running_instance_ids().await;
    sup.instance_store
        .list()
        .iter()
        .filter(|inst| !running_only || running_ids.contains(&inst.id))
        .map(|inst| (inst.name.clone(), inst.extension_data.clone()))
        .collect()
}

/// 받을 주소와 sha256을 정한다. 주소를 주지 않으면 서명된 카탈로그의 것을 쓴다.
/// 주소만 주면 카탈로그의 주소와 같을 때 카탈로그 해시를 쓰고, 다르면 해시 없이 받지 않는다 (`sha256_required`).
/// 주소와 해시를 함께 주면 그대로 쓴다 (카탈로그와 다르면 공식 설치로 기록하지 않는다).
pub(crate) fn install_source(
    id: &str,
    url: Option<String>,
    sha: Option<String>,
    remote: &anyhow::Result<Vec<crate::extension::RemoteExtensionInfo>>,
) -> Result<(String, Option<String>), (String, &'static str)> {
    let entry = remote.as_ref().ok().and_then(|r| r.iter().find(|e| e.id == id));
    let sha = sha.filter(|s| !s.is_empty());
    match (url.filter(|u| !u.is_empty()), sha) {
        (Some(url), Some(sha)) => Ok((url, Some(sha))),
        (Some(url), None) => match entry.filter(|e| e.download_url == url).and_then(|e| e.sha256.clone()) {
            Some(sha) => Ok((url, Some(sha))),
            None => Err((format!("Extension '{}': a download_url outside the catalog needs a sha256", id), "sha256_required")),
        },
        (None, _) => match (entry, remote) {
            (Some(e), _) => Ok((e.download_url.clone(), e.sha256.clone())),
            (None, Ok(_)) => Err((format!("Extension '{}' not found in manifest", id), "not_in_manifest")),
            (None, Err(e)) => Err((format!("Manifest fetch failed: {}", e), "manifest_fetch_failed")),
        },
    }
}

/// 익스텐션 하나를 (없는 의존 익스텐션과 함께) 받아 설치한다. 돌려주는 값은 설치한 id들.
/// 부르는 쪽이 `components_in_use`를 쥐고 있어야 한다
pub(crate) async fn install_with_dependencies(
    state: &IPCServer,
    ext_id: &str,
    explicit_url: Option<String>,
    explicit_sha: Option<String>,
) -> Result<Vec<String>, super::Failure> {
    let remote = ExtensionManager::fetch_remote().await;
    install_from(state, ext_id, explicit_url, explicit_sha, &remote).await
}

/// `install_with_dependencies`의 본체 (카탈로그는 부른 쪽이 준다).
/// 본 익스텐션과 없는 의존 익스텐션을 모두 받아 검사(stage)한 뒤에야 바꿔 끼운다: 의존 익스텐션 하나를 받지 못하면
/// 아무것도 바꾸지 않고 실패로 답한다 (본 익스텐션만 새 판으로 바뀐 채 "설치 실패"로 남지 않게)
async fn install_from(
    state: &IPCServer,
    ext_id: &str,
    explicit_url: Option<String>,
    explicit_sha: Option<String>,
    remote: &anyhow::Result<Vec<crate::extension::RemoteExtensionInfo>>,
) -> Result<Vec<String>, super::Failure> {
    let ext_id = ext_id.to_string();
    let extensions_dir = state.extension_manager.read().await.extensions_dir().to_path_buf();
    let fail = |error: String, code: &str| {
        let status = match code {
            "sha256_required" => StatusCode::BAD_REQUEST,
            "not_in_manifest" => StatusCode::NOT_FOUND,
            "manifest_fetch_failed" => StatusCode::BAD_GATEWAY,
            "older_than_installed" => StatusCode::CONFLICT,
            _ => StatusCode::INTERNAL_SERVER_ERROR,
        };
        super::Failure::coded(status, code, error)
    };
    let refused = |(status, Json(body)): (StatusCode, Json<serde_json::Value>)| super::Failure::new(status, body);

    // 이미 설치된 것을 바꾸는 것은 버전 고르기·모듈 설치와 같은 규칙: 켜진 서버가 쓰거나 손으로 고친 익스텐션은 바꾸지 않는다
    let replacing = state.extension_manager.read().await.discovered(&ext_id).is_some();
    let component = saba_chan_updater_lib::Component::Extension(ext_id.clone());
    let in_use = |servers: Vec<String>| {
        let error = format!("Extension '{}' is used by a running or starting server; stop it first", ext_id);
        let mut refusal = super::Failure::coded(StatusCode::CONFLICT, "in_use", error);
        refusal.body["related"] = json!(servers);
        refusal
    };
    if replacing {
        let servers = crate::ipc::updates::servers_using(&state.update_state, &component).await;
        if !servers.is_empty() {
            return Err(in_use(servers));
        }
        let key = component.manifest_key();
        if let Some(code) = crate::ipc::updates::hand_modified(&key) {
            return Err(super::Failure::coded(StatusCode::CONFLICT, &code, format!("{} was changed by hand; it is not replaced", key)));
        }
    }

    // 받아 검사한 패키지: (id, 폴더, 공식 설치인가). 실패하면 모두 지운다
    let mut staged: Vec<(String, std::path::PathBuf, bool)> = Vec::new();
    let discard = |staged: &[(String, std::path::PathBuf, bool)]| {
        for (_, dir, _) in staged {
            let _ = std::fs::remove_dir_all(dir);
        }
    };
    let mut queue = vec![ext_id.clone()];
    while let Some(id) = queue.pop() {
        if staged.iter().any(|(s, ..)| *s == id) {
            continue;
        }
        let step = stage_one(state, &extensions_dir, &id, &ext_id, &explicit_url, &explicit_sha, remote).await;
        let (dir, official) = match step {
            Ok(done) => done,
            Err(Stage::Failed(error, code)) => {
                discard(&staged);
                return Err(fail(error, code));
            }
            Err(Stage::Refused(refusal)) => {
                discard(&staged);
                return Err(refused(refusal));
            }
        };
        // 의존 목록은 받은 패키지의 manifest에서 읽는다 (아직 설치하지 않았다)
        if let Ok(r) = remote {
            let manifest = std::fs::read_to_string(dir.join("manifest.json")).ok().and_then(|t| crate::extension::parse_manifest(&t).ok());
            if let Some((manifest, _)) = manifest {
                queue.extend(state.extension_manager.read().await.missing_dependencies(&manifest, r));
            }
        }
        staged.push((id, dir, official));
    }

    // 바꿀 때까지 서버 시작을 막고 다시 본다 (받는 사이 켜졌을 수 있다). 시작·설치 중이면 그 서버가 쓰는 중이다
    let _starts = if replacing {
        let gate = state.supervisor.read().await.addon_gate();
        let starts = gate.try_hold_starts();
        let servers = crate::ipc::updates::servers_using(&state.update_state, &component).await;
        if starts.is_none() || !servers.is_empty() {
            discard(&staged);
            return Err(in_use(servers));
        }
        starts
    } else {
        None
    };
    let running = instance_ext_data(state, true).await;
    let refs: Vec<(&str, &std::collections::HashMap<String, serde_json::Value>)> =
        running.iter().map(|(n, d)| (n.as_str(), d)).collect();
    let mut mgr = state.extension_manager.write().await;
    let mut installed: Vec<String> = Vec::new();
    for (i, (id, dir, official)) in staged.iter().enumerate() {
        if let Err(e) = mgr.install_staged(id, dir, &refs, *official) {
            discard(&staged[i + 1..]);
            let refusal = extension_err_response(&e);
            if refusal.1.get("error_code").and_then(|c| c.as_str()) == Some("internal") {
                return Err(fail(e.to_string(), "install_failed"));
            }
            return Err(refused(refusal));
        }
        installed.push(id.clone());
    }
    Ok(installed)
}

enum Stage {
    Failed(String, &'static str),
    Refused((StatusCode, Json<serde_json::Value>)),
}

/// 익스텐션 하나를 받아 검사한다: (검사한 폴더, 공식 설치인가)
async fn stage_one(
    state: &IPCServer,
    extensions_dir: &std::path::Path,
    id: &str,
    ext_id: &str,
    explicit_url: &Option<String>,
    explicit_sha: &Option<String>,
    remote: &anyhow::Result<Vec<crate::extension::RemoteExtensionInfo>>,
) -> Result<(std::path::PathBuf, bool), Stage> {
    let entry = remote.as_ref().ok().and_then(|r| r.iter().find(|e| e.id == id).cloned());
    let explicit = if id == ext_id { (explicit_url.clone(), explicit_sha.clone()) } else { (None, None) };
    let (url, sha) = install_source(id, explicit.0, explicit.1, remote).map_err(|(error, code)| Stage::Failed(error, code))?;

    // 카탈로그가 설치된 것보다 낮은 버전을 주면 받지 않는다
    if let Some(e) = entry.as_ref().filter(|e| e.download_url == url) {
        let installed = state.extension_manager.read().await.discovered(id).map(|d| d.manifest.version.clone());
        if let Some(installed) = installed.filter(|v| saba_chan_updater_lib::version::is_downgrade(v, &e.version)) {
            return Err(Stage::Failed(format!("Extension '{}' {} is older than the installed {}", id, e.version, installed), "older_than_installed"));
        }
    }

    let dir = match ExtensionManager::download_package(&url).await.and_then(|bytes| {
        ExtensionManager::stage_package(extensions_dir, id, &bytes, sha.as_deref())
    }) {
        Ok(p) => p,
        // 더 새 앱을 요구하는 패키지는 구조화된 이유(app_version_unsatisfied)로 답한다
        Err(e) if e.downcast_ref::<ExtensionError>().is_some_and(|x| x.error_code == "app_version_unsatisfied") => {
            return Err(Stage::Refused(naming_extension(extension_err_response(&e), id)))
        }
        Err(e) => return Err(Stage::Failed(format!("{:#}", e), "install_failed")),
    };
    // 카탈로그의 주소와 해시로 받았을 때만 공식 설치다
    let official = entry.as_ref().is_some_and(|e| e.download_url == url && e.sha256.is_some() && e.sha256 == sha);
    Ok((dir, official))
}

/// 모듈을 설치한 뒤 그 모듈이 요구하는 익스텐션을 맞춘다: 없으면 카탈로그에서 받고, 켠다 (`ExtensionManager::enable_for_module`).
/// 모듈 설치는 이미 끝났으므로 실패는 `failed`로만 알린다. 부르는 쪽이 `components_in_use`를 쥐고 있어야 한다
pub(crate) async fn prepare_for_module(
    state: &IPCServer,
    required: &[String],
    previously_required: &[String],
) -> serde_json::Value {
    let failure = |id: &str, body: &serde_json::Value| {
        let field = |k: &str| body.get(k).cloned().unwrap_or_default();
        // `related`: 켜기 실패의 관련 항목. 사바쨩이 모자라면 `required_core`에 요구 버전 (CLI·GUI의 "업데이트 필요" 문구)
        let mut entry = json!({ "id": id, "error": field("error"), "error_code": field("error_code"), "related": field("related") });
        if let Some(version) = body.get("required_core") {
            entry["required_core"] = version.clone();
        }
        entry
    };
    let mut installed: Vec<String> = Vec::new();
    let mut failed = Vec::new();
    let mut to_enable = Vec::new();
    for id in required {
        let (present, unknown_deps) = {
            let mgr = state.extension_manager.read().await;
            let deps = mgr.discovered(id).map(|e| {
                e.manifest.dependencies.keys().filter(|d| mgr.discovered(d).is_none()).cloned().collect::<Vec<_>>()
            });
            (deps.is_some(), deps.unwrap_or_default())
        };
        // 이미 있어도 의존 익스텐션이 빠졌을 수 있다 (꺼 둔 채 의존 익스텐션만 지운 경우): 카탈로그에 있는 것만 받는다
        if !unknown_deps.is_empty() {
            let missing = match ExtensionManager::fetch_remote().await {
                Ok(remote) => {
                    let mgr = state.extension_manager.read().await;
                    mgr.discovered(id).map(|e| mgr.missing_dependencies(&e.manifest, &remote)).unwrap_or_default()
                }
                Err(_) => Vec::new(),
            };
            for dep in missing {
                if installed.contains(&dep) {
                    continue;
                }
                match install_with_dependencies(state, &dep, None, None).await {
                    Ok(ids) => installed.extend(ids),
                    Err(refusal) => failed.push(failure(&dep, &refusal.body)),
                }
            }
        }
        if !present && !installed.contains(id) {
            match install_with_dependencies(state, id, None, None).await {
                Ok(ids) => installed.extend(ids),
                Err(refusal) => {
                    failed.push(failure(id, &refusal.body));
                    continue;
                }
            }
        }
        to_enable.push(id.clone());
    }
    let (enabled, not_enabled) =
        state.extension_manager.write().await.enable_for_module(&to_enable, previously_required, &installed);
    for (id, e) in not_enabled {
        failed.push(failure(&id, &extension_err_response(&e).1 .0));
    }
    for id in &enabled {
        dispatch_startup(state, id).await;
    }
    json!({ "installed": installed, "enabled": enabled, "failed": failed })
}

/// 업데이트로 바뀐 모듈이 쓰는 익스텐션을 맞춘다 (모듈 설치의 `prepare_for_module`과 같은 일). 완료 마커의 `module-*` 키를 보고,
/// 마커 하나에 한 번만 한다. 업데이터가 적용하면 데몬이 다시 뜨므로 기동 때, 데몬이 바로 적용하면 그 뒤에 부른다
/// (`UpdateState::modules_updated`). 옛 판이 요구하던 목록은 남지 않으므로 이미 깔린 익스텐션은 켜짐 상태를 그대로 두고
/// (사용자가 껐을 수 있다) 없는 것만 받아 켠다. 돌려주는 값: (모듈 id, `prepare_for_module`의 결과)
pub async fn prepare_after_update(state: &IPCServer, marker_path: &std::path::Path) -> Vec<(String, serde_json::Value)> {
    let Some(marker) = saba_chan_updater_lib::UpdateCompletionMarker::take_unprepared_at(marker_path) else {
        return Vec::new();
    };
    let modules: Vec<&str> = marker
        .updated_components
        .iter()
        .filter_map(|k| k.strip_prefix(saba_chan_updater_lib::relay::MODULE_PREFIX))
        .collect();
    if modules.is_empty() {
        return Vec::new();
    }
    let _dirs = crate::component_lock::in_use().await;
    let mut prepared = Vec::new();
    for module_id in modules {
        let required = match state.supervisor.read().await.module_loader.get_module(module_id) {
            Ok(module) => module.required_extensions(),
            Err(_) => continue,
        };
        if required.is_empty() {
            continue;
        }
        let installed: Vec<String> = {
            let mgr = state.extension_manager.read().await;
            required.iter().filter(|id| mgr.discovered(id).is_some()).cloned().collect()
        };
        let result = prepare_for_module(state, &required, &installed).await;
        tracing::info!("[Updates] Extensions for updated module '{}': {}", module_id, result);
        prepared.push((module_id.to_string(), result));
    }
    prepared
}

/// anyhow::Error 에서 ExtensionError를 추출하여 구조화된 응답을 반환.
/// ExtensionError가 아닌 경우 일반 500 INTERNAL_SERVER_ERROR.
pub(crate) fn extension_err_response(
    err: &anyhow::Error,
) -> (StatusCode, Json<serde_json::Value>) {
    if let Some(ext_err) = err.downcast_ref::<ExtensionError>() {
        let status = match ext_err.error_code.as_str() {
            "not_found" | "not_mounted" | "manifest_not_found" => StatusCode::NOT_FOUND,
            "dependency_missing" | "dependency_not_enabled" | "component_version_unsatisfied"
            | "app_version_unsatisfied" => {
                StatusCode::UNPROCESSABLE_ENTITY
            }
            "has_dependents" | "in_use" | "folder_in_use" => StatusCode::CONFLICT,
            "id_mismatch" => StatusCode::BAD_REQUEST,
            _ => StatusCode::INTERNAL_SERVER_ERROR,
        };
        let mut body = json!({
            "success": false,
            "error": ext_err.message,
            "error_code": ext_err.error_code,
            "related": ext_err.related,
        });
        if let Some(version) = ext_err.required_core() {
            body["required_core"] = json!(version);
        }
        (status, Json(body))
    } else {
        (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(json!({
                "success": false,
                "error": err.to_string(),
                "error_code": "internal",
            })),
        )
    }
}

/// 켜기·설치 실패에 그 익스텐션 id를 싣는다 (`extension`). 설치는 의존성도 받으므로 요청한 것과 다를 수 있다
pub(crate) fn naming_extension(
    (status, Json(mut body)): (StatusCode, Json<serde_json::Value>),
    ext_id: &str,
) -> (StatusCode, Json<serde_json::Value>) {
    body["extension"] = json!(ext_id);
    (status, Json(body))
}

/// 방금 켠 익스텐션에 daemon.startup 훅이 있으면 비동기로 돌린다 (npm install 등). 돌렸는지 돌려준다
pub(crate) async fn dispatch_startup(state: &IPCServer, ext_id: &str) -> bool {
    let has_startup_hook = {
        let mgr = state.extension_manager.read().await;
        mgr.hooks_for("daemon.startup")
            .iter()
            .any(|(ext, _)| ext.manifest.id == ext_id)
    };

    if has_startup_hook {
        let ext_mgr = state.extension_manager.clone();
        let init_tracker = state.extension_init_tracker.clone();
        let ext_id_clone = ext_id.to_string();
        tokio::spawn(async move {
            init_tracker.mark_started(&ext_id_clone, "Initializing...").await;
            // 방금 켠 익스텐션에만 보낸다 (다른 익스텐션의 startup을 다시 돌리지 않게)
            match crate::extension::run_hook_to(&ext_mgr, &ext_id_clone, "daemon.startup", serde_json::json!({})).await {
                Some(result) => record_startup(&init_tracker, &ext_id_clone, result).await,
                // 조건이 맞지 않아 실행하지 않았다
                None => init_tracker.mark_finished(&ext_id_clone, true, "OK").await,
            }
        });
    }
    has_startup_hook
}

/// daemon.startup 훅 결과 하나를 로그와 초기화 상태(`/api/extensions/init-status`)에 남긴다
pub async fn record_startup(init_tracker: &crate::ipc::ExtensionInitTracker, ext_id: &str, result: anyhow::Result<serde_json::Value>) {
    match result {
        Ok(val) => {
            let success = val.get("success").and_then(|s| s.as_bool()).unwrap_or(true);
            let msg = val.get("message").and_then(|m| m.as_str()).unwrap_or("OK");
            if success {
                tracing::info!("Extension '{}' startup complete: {}", ext_id, msg);
            } else {
                let err = val.get("error").and_then(|e| e.as_str()).unwrap_or("unknown");
                tracing::warn!("Extension '{}' startup failed: {}", ext_id, err);
            }
            init_tracker.mark_finished(ext_id, success, msg).await;
        }
        Err(e) => {
            tracing::error!("Extension '{}' startup error: {}", ext_id, e);
            init_tracker.mark_finished(ext_id, false, &e.to_string()).await;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Arc;
    use tokio::sync::RwLock;

    fn zip_package(manifest: serde_json::Value) -> Vec<u8> {
        use std::io::Write;
        let mut buf = std::io::Cursor::new(Vec::new());
        {
            let mut w = zip::ZipWriter::new(&mut buf);
            w.start_file("manifest.json", zip::write::SimpleFileOptions::default()).unwrap();
            w.write_all(manifest.to_string().as_bytes()).unwrap();
            w.finish().unwrap();
        }
        buf.into_inner()
    }

    /// 의존 익스텐션을 받지 못하면 본 익스텐션도 설치하지 않고 실패로 답한다 (본 익스텐션만 바뀐 채 "설치 실패"로 남았다)
    #[tokio::test]
    async fn a_dependency_that_fails_to_download_leaves_nothing_installed() {
        use tokio::io::{AsyncReadExt, AsyncWriteExt};
        let app = zip_package(json!({ "id": "app", "name": "App", "version": "1.0.0", "dependencies": { "dep": ">=1.0.0" } }));
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();
        let served = app.clone();
        tokio::spawn(async move {
            loop {
                let Ok((mut sock, _)) = listener.accept().await else { return };
                let mut buf = [0u8; 2048];
                let n = sock.read(&mut buf).await.unwrap_or(0);
                let response = if String::from_utf8_lossy(&buf[..n]).starts_with("GET /app.zip") {
                    let mut r = format!("HTTP/1.1 200 OK\r\nContent-Length: {}\r\nConnection: close\r\n\r\n", served.len()).into_bytes();
                    r.extend_from_slice(&served);
                    r
                } else {
                    b"HTTP/1.1 404 Not Found\r\nContent-Length: 0\r\nConnection: close\r\n\r\n".to_vec()
                };
                let _ = sock.write_all(&response).await;
            }
        });
        let tmp = tempfile::tempdir().unwrap();
        let extensions = tmp.path().join("extensions");
        std::fs::create_dir_all(&extensions).unwrap();
        let mut sup = crate::supervisor::Supervisor::new_with_instances_dir(tmp.path().join("modules").to_str().unwrap(), tmp.path().join("instances").to_str().unwrap());
        sup.extension_manager = Arc::new(RwLock::new(ExtensionManager::new_isolated(extensions.to_str().unwrap())));
        let state = IPCServer::new(Arc::new(RwLock::new(sup)), "127.0.0.1:0", crate::daemon_log::DaemonLogBuffer::new());
        let sha = |bytes: &[u8]| {
            use sha2::{Digest, Sha256};
            hex::encode(Sha256::digest(bytes))
        };
        let remote: anyhow::Result<Vec<crate::extension::RemoteExtensionInfo>> = Ok(serde_json::from_value(json!([
            { "id": "app", "name": "App", "version": "1.0.0", "download_url": format!("http://{addr}/app.zip"), "sha256": sha(&app) },
            { "id": "dep", "name": "Dep", "version": "1.0.0", "download_url": format!("http://{addr}/dep.zip"), "sha256": sha(b"gone") },
        ]))
        .unwrap());

        let result = install_from(&state, "app", None, None, &remote).await;
        assert!(result.is_err(), "the dependency could not be downloaded");
        assert!(state.extension_manager.read().await.discovered("app").is_none(), "app is not installed without its dependency");
        let left: Vec<_> = std::fs::read_dir(&extensions)
            .unwrap()
            .flatten()
            .map(|e| e.file_name().to_string_lossy().into_owned())
            .filter(|name| name == "app" || name.starts_with(".staging"))
            .collect();
        assert!(left.is_empty(), "no package folders are left behind: {left:?}");
    }

    /// 이미 설치된 익스텐션은 켜진 서버가 모듈 요구로 쓰거나 손으로 고쳤으면 바꾸지 않는다 (버전 고르기·모듈 설치와 같은 판정)
    #[tokio::test]
    async fn an_installed_extension_is_not_replaced_while_a_running_server_needs_it_or_after_hand_edits() {
        let id = format!("swap{}", std::process::id());
        let key = format!("{}{id}", saba_chan_updater_lib::relay::EXTENSION_PREFIX);
        let extensions = crate::plugin::resolve_extensions_dir();
        let dir = extensions.join(&id);
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(dir.join("manifest.json"), json!({ "id": id, "name": "Swap", "version": "1.0.0" }).to_string()).unwrap();
        saba_chan_updater_lib::integrity::record_install(&key, &[dir.join("manifest.json")]).unwrap();
        saba_chan_updater_lib::integrity::set_official(&key, true).unwrap();
        let tmp = tempfile::tempdir().unwrap();
        let module = tmp.path().join("modules/game");
        std::fs::create_dir_all(&module).unwrap();
        std::fs::write(module.join("lifecycle.py"), "").unwrap();
        let toml = format!("[module]\nschema_version = 1\nname = \"game\"\nversion = \"1.0.0\"\nentry = \"lifecycle.py\"\n\n[install]\nrequires_extensions = [\"{id}\"]\n");
        std::fs::write(module.join("module.toml"), toml).unwrap();
        let mut sup = crate::supervisor::Supervisor::new_with_instances_dir(tmp.path().join("modules").to_str().unwrap(), tmp.path().join("instances").to_str().unwrap());
        sup.initialize().await.unwrap();
        let mut mgr = ExtensionManager::new_isolated(extensions.to_str().unwrap());
        mgr.discover().unwrap();
        sup.extension_manager = Arc::new(RwLock::new(mgr));
        let inst = crate::instance::ServerInstance::new("srv", "game");
        let server = inst.id.clone();
        sup.instance_store.add(inst).unwrap();
        let state = IPCServer::new(Arc::new(RwLock::new(sup)), "127.0.0.1:0", crate::daemon_log::DaemonLogBuffer::new());
        let remote: anyhow::Result<Vec<crate::extension::RemoteExtensionInfo>> = Ok(serde_json::from_value(json!([
            { "id": id, "name": "Swap", "version": "1.1.0", "download_url": "http://127.0.0.1:9/swap.zip", "sha256": "00" },
        ]))
        .unwrap());

        state.supervisor.read().await.tracker.track(&server, std::process::id());
        let refused = install_from(&state, &id, None, None, &remote).await.unwrap_err();
        assert_eq!((refused.status, refused.body["error_code"].as_str(), &refused.body["related"]), (StatusCode::CONFLICT, Some("in_use"), &json!(["srv"])));

        let _ = state.supervisor.read().await.tracker.untrack(&server);
        std::fs::write(dir.join("my-notes.txt"), "mine").unwrap();
        let refused = install_from(&state, &id, None, None, &remote).await.unwrap_err();
        assert_eq!((refused.status, refused.body["error_code"].as_str()), (StatusCode::CONFLICT, Some("extension_modified")));
        assert!(dir.join("my-notes.txt").exists());
        let _ = std::fs::remove_dir_all(&dir);
    }
}
