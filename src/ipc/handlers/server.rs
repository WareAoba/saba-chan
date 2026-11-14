use super::LooseJson;
use axum::{
    extract::{Path, Query, State},
    http::StatusCode,
    response::IntoResponse,
    Json,
};
use serde_json::{json, Value};

use crate::service::lifecycle::{instance_not_found, start, user_restart, user_stop};

use super::super::{ExtensionInfo, ExtensionListResponse, IPCServer, ProtocolsInfo, ServerStopRequest};

/// GET /health - 데몬 활성 확인용 경량 ping
///
/// supervisor lock이나 디스크 I/O 없이 즉시 응답합니다.
/// `daemon:status` 폴링 및 데몬 ready 체크에 사용됩니다.
/// 토큰 없이 부를 수 있다(A6). `token`은 보낸 토큰이 맞는지(`valid`·`invalid`·`missing`)
pub async fn health_check(headers: axum::http::HeaderMap) -> impl IntoResponse {
    let version = env!("CARGO_PKG_VERSION");
    let token = super::super::auth::token_status(&headers);
    let challenge = headers.get("X-Saba-Challenge").and_then(|v| v.to_str().ok());
    let proof = challenge.and_then(crate::ipc::auth::identity_proof);
    (StatusCode::OK, Json(json!({ "ok": true, "version": version, "pid": std::process::id(), "started_at_ms": *crate::ipc::STARTED_AT_MS, "token": token, "proof": proof, "stops_with_launcher": stops_with_launcher() })))
}

/// 데몬이 닫히면 함께 끝내는 job 안에 있는가 (L46). GUI가 job breakaway를 거절당해 그대로 띄우면, GUI를 띄운 프로그램이
/// 끝날 때 데몬도 끝날 수 있다. 시작 뒤 바뀌지 않으므로 한 번만 본다. 바로 위 job만 본다 (중첩 job의 바깥 job은 알 수 없다)
pub fn stops_with_launcher() -> bool {
    static FOUND: std::sync::OnceLock<bool> = std::sync::OnceLock::new();
    *FOUND.get_or_init(|| {
        #[cfg(windows)]
        unsafe {
            use windows_sys::Win32::System::JobObjects::{JobObjectExtendedLimitInformation, QueryInformationJobObject, JOBOBJECT_EXTENDED_LIMIT_INFORMATION};
            let mut info: JOBOBJECT_EXTENDED_LIMIT_INFORMATION = std::mem::zeroed();
            // 핸들 없이 부르면 이 프로세스의 job을 본다. job 밖이면 실패한다
            let read = QueryInformationJobObject(
                std::ptr::null_mut(),
                JobObjectExtendedLimitInformation,
                &mut info as *mut _ as *mut _,
                std::mem::size_of::<JOBOBJECT_EXTENDED_LIMIT_INFORMATION>() as u32,
                std::ptr::null_mut(),
            );
            job_stops_with_it(read != 0, info.BasicLimitInformation.LimitFlags)
        }
        #[cfg(not(windows))]
        false
    })
}

/// `JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE` (winnt.h)
#[cfg(any(windows, test))]
const KILL_ON_JOB_CLOSE: u32 = 0x2000;

/// 이 프로세스가 든 job이 닫힐 때 함께 끝나는가. 이미 job 안에 있으면 breakaway 허용 여부(새로 띄우는 프로세스의 일)와 상관없다
#[cfg(any(windows, test))]
fn job_stops_with_it(in_job: bool, limit_flags: u32) -> bool {
    in_job && limit_flags & KILL_ON_JOB_CLOSE != 0
}

#[derive(serde::Deserialize, Default)]
pub struct ListServersQuery {
    /// 이 번호 뒤의 포트 충돌 강제 정지 알림만 받는다. 읽어도 다른 클라이언트 몫을 지우지 않는다 (L50)
    pub notices_after: Option<u64>,
}

/// GET /api/servers - 모든 서버 목록 (인스턴스 기반). 포트 충돌 강제 정지 알림은 `notices_after` 뒤의 것을 싣고,
/// 매개변수가 없거나 `notices=keep`이면 남은 것 모두를 싣는다. 읽어도 지우지 않는다
pub async fn list_servers(State(state): State<IPCServer>, Query(q): Query<ListServersQuery>) -> axum::response::Response {
    super::answer(crate::service::servers::list(&state, q.notices_after.unwrap_or(0)).await)
}

/// GET /api/modules - 모든 모듈 목록
/// 모듈 하나를 API 응답 모양으로 (아이콘은 base64 data URL로)
/// 지금 설치된 컴포넌트 버전 (`versions.toml`과 같은 키: saba-core, module-<이름>, ext-<id>, gui, cli ...).
/// 업데이터가 기록한 installed-manifest를 바탕으로, 데몬이 직접 아는 값(코어, 로드된 모듈, 발견된 익스텐션)으로 덮는다 (U11, M9).
pub async fn component_versions(state: &IPCServer) -> std::collections::BTreeMap<String, String> {
    let mut versions: std::collections::BTreeMap<String, String> =
        saba_chan_updater_lib::UpdateManager::load_installed_manifest().into_iter().collect();
    versions.insert(crate::module_loader::CORE_COMPONENT.to_string(), crate::module_loader::DAEMON_VERSION.to_string());
    if let Ok(modules) = state.supervisor.read().await.list_modules() {
        for m in modules {
            versions.insert(format!("module-{}", m.metadata.name), m.metadata.version);
        }
    }
    for ext in state.extension_manager.read().await.list() {
        versions.insert(format!("ext-{}", ext.id), ext.version);
    }
    versions
}

/// GET /api/version — 코어·모듈·익스텐션·인터페이스 버전을 한 번에
pub async fn version_handler(State(state): State<IPCServer>) -> impl IntoResponse {
    Json(json!({
        "version": crate::module_loader::DAEMON_VERSION,
        "components": component_versions(&state).await,
    }))
}

/// 모듈 목록 한 줄. 별명은 module.toml(언어 없는 낱말)에 `lang`(앱 언어)의 모듈 locales 별명을 더한다 (`chat::api::locale_aliases`)
fn module_info(m: crate::module_loader::LoadedModule, versions: &std::collections::BTreeMap<String, String>, lang: &str) -> ExtensionInfo {
    let installed: std::collections::HashMap<String, String> = versions.iter().map(|(k, v)| (k.clone(), v.clone())).collect();
    let unmet_dependencies = m
        .metadata
        .check_dependencies(&installed)
        .into_iter()
        .map(|(component, required, installed)| crate::ipc::UnmetDependency { component, required, installed })
        .collect();
    let icon_base64 = m.metadata.icon.as_ref().and_then(|icon_file| {
        let icon_path = std::path::Path::new(&m.path).join(icon_file);
        std::fs::read(&icon_path).ok().map(|data| {
            use base64::{engine::general_purpose, Engine as _};
            format!("data:image/png;base64,{}", general_purpose::STANDARD.encode(&data))
        })
    });
    let trust = crate::module_audit::module_trust(&m.path);
    let addon = addon_choice(&format!("{}{}", saba_chan_updater_lib::relay::MODULE_PREFIX, m.metadata.name), &m.metadata.version);
    let localized = crate::chat::api::locale_aliases(std::path::Path::new(&m.path), lang).unwrap_or_default();
    let unsupported_reason = m.metadata.unsupported_on(std::env::consts::OS, std::env::consts::ARCH).map(str::to_string);
    let mut meta = m.metadata;
    crate::chat::api::push_words(&mut meta.module_aliases, localized.module.into_iter());
    for (cmd, words) in localized.commands {
        crate::chat::api::push_words(&mut meta.command_aliases.entry(cmd).or_default().aliases, words.into_iter());
    }
    ExtensionInfo {
        platforms: meta.platforms,
        architectures: meta.architectures,
        platform_supported: unsupported_reason.is_none(),
        unsupported_reason,
        name: meta.name,
        version: meta.version,
        trust,
        description: meta.description,
        schema_version: meta.schema_version,
        display_name: meta.display_name,
        game_name: meta.game_name,
        author: meta.author,
        warnings: meta.warnings,
        unmet_dependencies,
        path: m.path,
        executable_path: meta.executable_path,
        server_executable: meta.server_executable,
        icon: icon_base64,
        interaction_mode: meta.interaction_mode,
        protocols: meta.protocols_supported.map(|supported| ProtocolsInfo {
            supported,
            default: meta.protocols_default,
        }),
        settings: meta.settings,
        commands: meta.commands,
        syntax_highlight: meta.syntax_highlight,
        dir_signatures: meta.dir_signatures,
        install: meta.install,
        module_aliases: meta.module_aliases,
        command_aliases: meta.command_aliases,
        presets: meta.presets,
        queries: meta.queries,
        agreements: meta.agreements,
        lists: meta.lists,
        functions: None,
        addon,
    }
}

/// 목록에 싣는 사용자가 고른 판 (`addon_pins`): 고정과, 깔린 판과 달라 아직 기다리는 고른 판 (`AddonChoice::as_installed`)
pub(crate) fn addon_choice(key: &str, installed: &str) -> saba_chan_updater_lib::addon_pins::AddonChoice {
    saba_chan_updater_lib::addon_pins::get(key).as_installed(installed)
}

async fn module_list_response(
    modules: anyhow::Result<Vec<crate::module_loader::LoadedModule>>,
    failed: Vec<crate::module_loader::ModuleLoadError>,
    versions: &std::collections::BTreeMap<String, String>,
    lang: &str,
) -> axum::response::Response {
    match modules {
        Ok(modules) => {
            // 구현한 함수 목록 (MK1): 캐시에 없으면 짧게만 기다린다(느린 모듈은 null, 묻기는 뒤에서 마저). 데몬 락 밖에서, 모듈끼리 동시에
            let asks: Vec<_> = modules.iter().map(|m| tokio::spawn(crate::plugin::module_functions_for_list(m.entry_path()))).collect();
            let mut modules: Vec<ExtensionInfo> = modules.into_iter().map(|m| module_info(m, versions, lang)).collect();
            for (info, ask) in modules.iter_mut().zip(asks) {
                info.functions = ask.await.ok().flatten();
            }
            (StatusCode::OK, Json(ExtensionListResponse { modules, failed })).into_response()
        }
        Err(e) => {
            super::error(StatusCode::INTERNAL_SERVER_ERROR, "internal_server_error", format!("Failed to list modules: {}", e))
        }
    }
}

pub async fn list_modules(State(state): State<IPCServer>) -> impl IntoResponse {
    let versions = component_versions(&state).await;
    let (modules, failed) = {
        let supervisor = state.supervisor.read().await;
        (supervisor.list_modules(), supervisor.module_loader.load_errors())
    };
    module_list_response(modules, failed, &versions, &state.config_store.ui_language().await).await
}

/// POST /api/modules/refresh - 모듈 캐시를 새로고침하고 다시 발견
pub async fn refresh_modules(State(state): State<IPCServer>) -> impl IntoResponse {
    // 문자열로 저장된 값의 설정이 list가 되었으면 옮긴다 (06 §8.1)
    let modules = {
        let mut supervisor = state.supervisor.write().await;
        let modules = supervisor.refresh_modules();
        supervisor.migrate_settings();
        modules
    };
    if let Ok(m) = &modules {
        tracing::info!("Module cache refreshed. Found {} modules", m.len());
        crate::plugin::prefetch_module_functions(m.iter().map(|m| m.entry_path()).collect());
    }
    let versions = component_versions(&state).await;
    let failed = state.supervisor.read().await.module_loader.load_errors();
    module_list_response(modules, failed, &versions, &state.config_store.ui_language().await).await
}

/// GET /api/modules/schema - module.toml 스키마 v1 (JSON Schema)
pub async fn module_schema() -> impl IntoResponse {
    Json(crate::module_loader::module_json_schema())
}

/// POST /api/modules/detect `{path}` - 이 폴더가 어느 모듈의 게임 서버 폴더인지
/// (`[detection] dir_signatures`가 전부 있는 모듈). 기존 서버 가져오기에서 쓴다.
pub async fn detect_module_dir(
    State(state): State<IPCServer>,
    Json(body): Json<serde_json::Value>,
) -> impl IntoResponse {
    let Some(path) = body.get("path").and_then(|v| v.as_str()) else {
        return super::error(StatusCode::BAD_REQUEST, "invalid_request", "path is required");
    };
    let dir = std::path::Path::new(path);
    if !dir.is_dir() {
        return super::error(StatusCode::NOT_FOUND, "not_found", format!("not a folder: {}", path));
    }
    let matches: Vec<_> = {
        let supervisor = state.supervisor.read().await;
        let modules = supervisor.list_modules().unwrap_or_default();
        modules
            .iter()
            .filter(|m| m.matches_dir(dir))
            .map(|m| json!({ "name": m.metadata.name, "display_name": m.metadata.display_name }))
            .collect()
    };
    // 가져오기 전에 알릴 것 (만들 때와 같은 `migration::notices`): 사바쨩 밖에서 켜 둔 서버, 쓸 수 없는 폴더.
    // 프로세스 목록 읽기는 막는 호출이라 런타임 밖에서
    let folder = dir.to_path_buf();
    let notices = tokio::task::spawn_blocking(move || crate::instance::migration::notices(&folder)).await.unwrap_or_default();
    (StatusCode::OK, Json(json!({ "matches": matches, "notices": notices }))).into_response()
}

/// GET /api/module/:name - 모듈 메타데이터 조회 (별명 포함)
/// GET /api/module/:name/audit — 모듈 Python 코드 검사 (M12): `{declared, findings, undeclared}`.
/// `undeclared`가 비어 있지 않으면 모듈이 선언하지 않은 능력을 쓴다 (사용자에게 보여 줄 것)
pub async fn audit_module(Path(name): Path<String>, State(state): State<IPCServer>) -> impl IntoResponse {
    let module = state.supervisor.read().await.module_loader.get_module(&name).ok();
    let Some(module) = module else {
        return super::error(StatusCode::NOT_FOUND, "not_found", format!("Module '{}' not found", name));
    };
    let found = match crate::python_env::get_python_path().await {
        Ok(python) => crate::module_audit::scan(&python, std::path::Path::new(&module.path)).await,
        Err(e) => Err(e),
    };
    match found {
        Ok(findings) => {
            let undeclared = crate::module_audit::undeclared(&module.metadata.permissions, &findings);
            Json(json!({ "success": true, "declared": module.metadata.permissions, "findings": findings, "undeclared": undeclared })).into_response()
        }
        Err(e) => super::error(StatusCode::INTERNAL_SERVER_ERROR, "audit_failed", format!("{:#}", e)),
    }
}

pub async fn get_module_metadata(
    Path(name): Path<String>,
    State(state): State<IPCServer>,
) -> impl IntoResponse {
    let supervisor = state.supervisor.read().await;
    match supervisor.list_modules() {
        Ok(modules) => match modules.iter().find(|m| m.metadata.name == name) {
            Some(module) => (
                StatusCode::OK,
                Json(json!({
                    "name": &module.metadata.name,
                    "version": &module.metadata.version,
                    "description": &module.metadata.description,
                    "path": &module.path,
                    "metadata": &module.metadata,
                })),
            )
                .into_response(),
            None => super::error(StatusCode::NOT_FOUND, "not_found", format!("Module '{}' not found", name)),
        },
        Err(e) => super::error(StatusCode::INTERNAL_SERVER_ERROR, "internal_server_error", format!("Failed to list modules: {}", e)),
    }
}

/// GET /api/modules/manifest - 설치할 수 있는 모듈 목록 (최신 릴리즈의 서명된 manifest, U9e)
///
/// `{success, manifest: {modules: {<이름>: {version, display_name, description, ...}}}}`
pub async fn fetch_module_manifest() -> impl IntoResponse {
    match saba_chan_updater_lib::relay::latest_catalog(None, &crate::ipc::updates::load_updater_config().channel).await {
        Ok(catalog) => {
            let mut modules = serde_json::Map::new();
            if let Some(manifest) = &catalog {
                for (name, comp) in manifest.addons(saba_chan_updater_lib::relay::MODULE_PREFIX) {
                    modules.insert(name.to_string(), catalog_module_info(comp, std::env::consts::OS, std::env::consts::ARCH));
                }
            }
            (StatusCode::OK, Json(json!({ "success": true, "manifest": { "modules": modules } }))).into_response()
        }
        Err(e) => super::error(StatusCode::BAD_GATEWAY, "manifest_fetch_failed", e.to_string()),
    }
}

/// 카탈로그 모듈 하나의 목록 정보: meta에 버전·해시·크기와, 이 OS·CPU에서 도는지(`platform_supported`,
/// 아니면 `unsupported_reason` `os`·`arch`)를 더한다. 설치된 모듈 목록과 같은 이름이다
fn catalog_module_info(comp: &saba_chan_updater_lib::relay::ManifestComponent, os: &str, arch: &str) -> Value {
    let mut info = match &comp.meta {
        Some(Value::Object(meta)) => meta.clone(),
        _ => serde_json::Map::new(),
    };
    let reason = crate::module_loader::catalog_unsupported_on(&Value::Object(info.clone()), os, arch);
    info.insert("version".into(), json!(comp.version));
    info.insert("sha256".into(), json!(comp.sha256));
    info.insert("size_bytes".into(), json!(comp.size_bytes));
    info.insert("platform_supported".into(), json!(reason.is_none()));
    if let Some(reason) = reason {
        info.insert("unsupported_reason".into(), json!(reason));
    }
    Value::Object(info)
}

/// POST /api/modules/manifest/:id/install - 최신 릴리즈에서 모듈을 받아 설치한다 (SHA-256 검증)
pub async fn install_module_from_manifest(
    Path(module_id): Path<String>,
    State(state): State<IPCServer>,
) -> impl IntoResponse {
    if !crate::utils::is_single_path_component(&module_id) {
        return super::error(StatusCode::BAD_REQUEST, "invalid_request", "Invalid module ID");
    }
    let Some(installing) = module_install_guard(&module_id) else {
        return module_busy(&module_id);
    };
    // 잠금을 쥔 채 따로 돌린다: 요청이 끊겨도 받은 파일 정리와 잠금 해제가 작업 끝에 함께 일어난다
    crate::audit::spawn(async move {
        let _installing = installing;
        install_module_locked(state, module_id).await
    })
    .await
    .unwrap_or_else(|e| super::error(StatusCode::INTERNAL_SERVER_ERROR, "module_install_failed", e.to_string()))
}

/// 카탈로그 모듈의 `requires`를 못 맞춘 거절. 로드 실패·시작 응답과 같은 코드와 목록 꼴(`"saba-core >=0.1.10 (installed 0.1.9)"`)
fn unmet_requires_body(module_id: &str, unmet: &[saba_chan_updater_lib::DependencyIssue]) -> Value {
    let required_core = unmet
        .iter()
        .find(|i| i.required_component == "saba-core")
        .map(|i| i.required_version.trim().trim_start_matches(">=").trim().to_string());
    let mut body = json!({
        "success": false,
        "error": unmet.iter().map(|i| i.message.as_str()).collect::<Vec<_>>().join("; "),
        "error_code": "module_dependency_unsatisfied",
        "module": module_id,
        "missing_dependencies": unmet
            .iter()
            .map(|i| format!("{} {} ({})", i.required_component, i.required_version,
                i.installed_version.as_ref().map_or("not installed".to_string(), |v| format!("installed {v}"))))
            .collect::<Vec<_>>(),
    });
    if let Some(version) = required_core {
        body["required_core"] = json!(version);
    }
    body
}

async fn install_module_locked(state: IPCServer, module_id: String) -> axum::response::Response {
    let catalog = match saba_chan_updater_lib::relay::latest_catalog(None, &crate::ipc::updates::load_updater_config().channel).await {
        Ok(Some(m)) => m,
        Ok(None) => return super::error(StatusCode::NOT_FOUND, "not_in_manifest", "No release is published"),
        Err(e) => return super::error(StatusCode::BAD_GATEWAY, "manifest_fetch_failed", e.to_string()),
    };
    let Some(comp) = catalog
        .components
        .get(&format!("{}{}", saba_chan_updater_lib::relay::MODULE_PREFIX, module_id))
        .cloned()
    else {
        return super::error(StatusCode::NOT_FOUND, "not_in_manifest", format!("Module '{}' is not in the latest release", module_id));
    };
    // 설치된 것보다 낮은 버전은 받지 않는다
    let previous = state.supervisor.read().await.module_loader.get_module(&module_id).ok();
    let previously_required = previous.as_ref().map(crate::module_loader::LoadedModule::required_extensions).unwrap_or_default();
    let installed = previous.map(|m| m.metadata.version.clone());
    if let Some(installed) = installed.filter(|v| saba_chan_updater_lib::version::is_downgrade(v, &comp.version)) {
        return super::error(StatusCode::CONFLICT, "older_than_installed", format!("Module '{}' {} is older than the installed {}", module_id, comp.version, installed));
    }
    // 새 코어를 요구하는 판은 받지 않는다 (업데이터 적용과 같은 검사)
    if let Some(requires) = &comp.requires {
        let installed: std::collections::HashMap<String, String> = component_versions(&state).await.into_iter().collect();
        let key = format!("{}{}", saba_chan_updater_lib::relay::MODULE_PREFIX, module_id);
        let unmet = saba_chan_updater_lib::unmet_requires(&key, requires, &installed);
        if !unmet.is_empty() {
            return (StatusCode::UNPROCESSABLE_ENTITY, Json(unmet_requires_body(&module_id, &unmet))).into_response();
        }
    }

    // 여기부터 모듈 폴더에 쓴다: 폴더를 옮기는 중이면 끝나기를 기다리고, 끝날 때까지 옮기지 못하게 한다
    let _dirs = crate::component_lock::in_use().await;
    let modules_dir = {
        let supervisor = state.supervisor.read().await;
        std::path::PathBuf::from(supervisor.module_loader.modules_dir())
    };
    let (zip_path, staging) = module_install_paths(&modules_dir, &module_id);
    let target_dir = modules_dir.join(&module_id);

    let result: anyhow::Result<()> = async {
        let http = saba_chan_updater_lib::tls::http_client_builder().build()?;
        const MODULE_DOWNLOAD_LIMIT: u64 = 128 * 1024 * 1024;
        let max_bytes = Some(comp.size_bytes.unwrap_or(MODULE_DOWNLOAD_LIMIT).min(MODULE_DOWNLOAD_LIMIT));
        saba_chan_updater_lib::download::download_verified(&http, &comp.url, &zip_path, Some(&comp.sha256), max_bytes, None).await?;
        let _ = std::fs::remove_dir_all(&staging);
        saba_chan_updater_lib::extract_zip(&zip_path, &staging, |_| Ok(()))?;
        install_staged_module(&staging, &target_dir)?;
        // 서명된 카탈로그에서 받은 것: 신뢰 표시가 "verified"가 된다 (파일이 그대로인 동안)
        let key = format!("{}{}", saba_chan_updater_lib::relay::MODULE_PREFIX, module_id);
        if let Err(e) = saba_chan_updater_lib::integrity::record_dir(&key, &target_dir)
            .and_then(|_| saba_chan_updater_lib::integrity::set_official(&key, true))
        {
            tracing::warn!("Could not record installed files for module '{}': {}", module_id, e);
        }
        Ok(())
    }
    .await;
    let _ = std::fs::remove_file(&zip_path);
    let _ = std::fs::remove_dir_all(&staging);
    if let Err(e) = result {
        return super::error(StatusCode::INTERNAL_SERVER_ERROR, "module_install_failed", format!("{:#}", e));
    }
    let required = {
        let mut supervisor = state.supervisor.write().await;
        let _ = supervisor.refresh_modules();
        supervisor.migrate_settings();
        supervisor.module_loader.get_module(&module_id).ok().as_ref().map(crate::module_loader::LoadedModule::required_extensions).unwrap_or_default()
    };
    // 이 모듈이 쓰는 익스텐션을 받고 켠다 (새로 필요해진 것만 켠다: 예전 판부터 쓰던 것은 사용자가 껐을 수 있다)
    let extensions = crate::service::extensions::prepare_for_module(&state, &required, &previously_required).await;
    (StatusCode::OK, Json(json!({ "success": true, "module_id": module_id, "version": comp.version, "extensions": extensions })))
        .into_response()
}

/// 모듈 하나의 설치·제거는 한 번에 하나만 (같은 모듈을 GUI·CLI가 동시에 설치하면 서로의 파일을 지운다).
/// 키는 소문자로: Windows에서는 대소문자만 다른 id가 같은 폴더다
pub(crate) fn module_install_guard(module_id: &str) -> Option<crate::instance::lock::InstanceLock> {
    crate::instance::lock::InstanceLock::take(&format!("module-install:{}", module_id.to_lowercase())).map(|g| g.purpose(crate::instance::lock::Purpose::Provision))
}

pub(crate) fn module_busy(module_id: &str) -> axum::response::Response {
    let error = format!("Module '{}' is already being installed or removed", module_id);
    super::error(StatusCode::CONFLICT, "module_busy", error)
}

/// 설치 요청마다 따로 쓰는 받은 zip과 압축 해제 폴더 (`.tmp-`·`.staging-`로 시작해 남으면 다음 기동이 지운다)
fn module_install_paths(modules_dir: &std::path::Path, module_id: &str) -> (std::path::PathBuf, std::path::PathBuf) {
    let tag = uuid::Uuid::new_v4().simple().to_string();
    (
        modules_dir.join(format!(".tmp-module-{}-{}.zip", module_id, tag)),
        modules_dir.join(format!(".staging-{}-{}", module_id, tag)),
    )
}

/// 풀어 둔 모듈 패키지를 데몬과 같은 규칙으로 읽어 본 뒤 설치 폴더와 바꾼다.
/// 기존 폴더는 새 폴더가 자리를 잡을 때까지 옆에 두고, 바꾸다 실패하면 되돌린다
fn install_staged_module(staging: &std::path::Path, target_dir: &std::path::Path) -> anyhow::Result<()> {
    crate::module_loader::load_module_dir(staging).map_err(|e| e.context("package is not a loadable module"))?;
    crate::utils::swap_dir(staging, target_dir)
}

/// DELETE /api/modules/:id — 모듈 제거 (디렉토리 삭제 + 캐시 갱신)
pub async fn remove_module(
    Path(module_id): Path<String>,
    State(state): State<IPCServer>,
) -> impl IntoResponse {
    // 경로 탐색 방지
    if !crate::utils::is_single_path_component(&module_id) {
        return super::error(StatusCode::BAD_REQUEST, "invalid_request", "Invalid module ID");
    }
    let Some(_removing) = module_install_guard(&module_id) else {
        return module_busy(&module_id);
    };
    let _dirs = crate::component_lock::in_use().await;
    let modules_dir = {
        let supervisor = state.supervisor.read().await;
        // 켜진 서버가 쓰는 모듈을 지우면 그 서버를 정지할 수 없다 (EE9). Windows 경로는 대소문자를 가리지 않는다
        for instance in supervisor.instance_store.list() {
            if instance.module_name.eq_ignore_ascii_case(&module_id) && supervisor.is_running(&instance.id).await {
                let error = format!("Module '{}' is used by a running server; stop it first", module_id);
                return super::error(StatusCode::CONFLICT, "module_in_use", error);
            }
        }
        let modules_dir = std::path::PathBuf::from(supervisor.module_loader.modules_dir());
        // `<modules>/<id>` 폴더가 아니면 찾은 모듈의 원본(modules 폴더에 놓은 ZIP)을 지운다
        let zip = supervisor.module_loader.discover_modules().ok().and_then(|modules| {
            modules
                .into_iter()
                .find(|m| m.metadata.name.eq_ignore_ascii_case(&module_id))
                .map(|m| (std::path::PathBuf::from(m.source), std::path::PathBuf::from(m.path)))
                .filter(|(source, _)| source.is_file() && source.parent() == Some(modules_dir.as_path()))
        });
        (modules_dir, zip)
    };
    let (modules_dir, zip) = modules_dir;
    let module_path = modules_dir.join(&module_id);
    let removed = if module_path.is_dir() {
        crate::plugin::stop_workers(Some(&module_path));
        std::fs::remove_dir_all(&module_path)
    } else if let Some((zip, extracted)) = zip {
        crate::plugin::stop_workers(Some(&extracted));
        std::fs::remove_file(&zip).map(|()| {
            // 풀어 둔 사본은 다시 만들 수 있다: 지우지 못해도 제거는 끝났다
            if extracted.starts_with(modules_dir.join(".extracted")) {
                let _ = std::fs::remove_dir_all(&extracted);
            }
        })
    } else {
        return super::error(StatusCode::NOT_FOUND, "not_found", "Module not found");
    };
    if let Err(e) = removed {
        return super::error(StatusCode::INTERNAL_SERVER_ERROR, "remove_failed", e.to_string());
    }
    let _ = saba_chan_updater_lib::integrity::set_official(&format!("{}{}", saba_chan_updater_lib::relay::MODULE_PREFIX, module_id), false);
    // 캐시 갱신 (write lock)
    let mut supervisor = state.supervisor.write().await;
    let _ = supervisor.refresh_modules();
    supervisor.migrate_settings();
    (StatusCode::OK, Json(json!({ "success": true, "id": module_id }))).into_response()
}

/// GET /api/instance/:id/status — id로 상태 조회 (없으면 이름으로 찾는다). 못 찾았으면 404 `instance_not_found`
pub async fn get_instance_status(
    Path(id): Path<String>,
    State(state): State<IPCServer>,
) -> axum::response::Response {
    let instance = {
        let supervisor = state.supervisor.read().await;
        supervisor
            .instance_store
            .find(&id)
            .map(|i| (i.id.clone(), i.module_name.clone()))
    };
    if let Some((id, module_name)) = instance {
        match crate::supervisor::Supervisor::get_server_status(&state.supervisor, &id, &module_name).await {
            Ok(result) => (StatusCode::OK, Json(result)).into_response(),
            Err(e) => {
                super::error(StatusCode::INTERNAL_SERVER_ERROR, "internal_server_error", format!("Failed to get status: {}", e))
            }
        }
    } else {
        super::error(StatusCode::NOT_FOUND, "instance_not_found", format!("Server '{}' not found", id))
    }
}

/// POST /api/instance/:id/start — 서버 시작 (`start`)
pub async fn start_instance_handler(
    Path(id): Path<String>,
    State(state): State<IPCServer>,
) -> axum::response::Response {
    super::answer(start(&state, &id).await)
}

/// POST /api/instance/:id/stop — id로 정지 (`{force?}`)
pub async fn stop_instance_handler(
    Path(id): Path<String>,
    State(state): State<IPCServer>,
    LooseJson(body): LooseJson<ServerStopRequest>,
) -> axum::response::Response {
    let force = body.map(|Json(b)| b.force).unwrap_or(false);
    super::answer(user_stop(&state, &id, force).await)
}

/// POST /api/instance/:id/restart — 정지(`{force?}`) 뒤 시작 (`restart`)
pub async fn restart_instance_handler(
    Path(id): Path<String>,
    State(state): State<IPCServer>,
    LooseJson(body): LooseJson<ServerStopRequest>,
) -> axum::response::Response {
    let force = body.map(|Json(b)| b.force).unwrap_or(false);
    super::answer(user_restart(&state, &id, force).await)
}

/// GET /api/instance/:id/events — 최근 종료 이력 (사용자 정지, 게임 안 정지, 크래시, 원인 모를 종료)
pub async fn instance_events_handler(Path(id): Path<String>, State(state): State<IPCServer>) -> impl IntoResponse {
    let supervisor = state.supervisor.read().await;
    if supervisor.instance_store.get(&id).is_none() {
        return instance_not_found(&id).into_response();
    }
    Json(json!({ "events": supervisor.exit_events(&id) })).into_response()
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Arc;
    use tokio::sync::RwLock;

    /// L46: 데몬이 자기를 띄운 프로그램과 함께 끝나는 job 안에 있는지 /health에 싣는다 (GUI·CLI가 알린다)
    /// 받기 전 목록도 이 OS·CPU에서 도는지 싣는다 (맥에서 x86_64 전용 모듈을 받은 뒤에야 막히지 않게)
    #[test]
    fn catalog_modules_say_whether_they_run_here() {
        let comp = |meta: Value| -> saba_chan_updater_lib::relay::ManifestComponent {
            serde_json::from_value(json!({"version": "1.0.0", "asset": "module-g.zip", "sha256": "ab", "url": "https://r2/g.zip", "size_bytes": 7, "meta": meta})).unwrap()
        };
        let x86 = catalog_module_info(&comp(json!({"display_name": "G", "architectures": ["x86_64"]})), "macos", "aarch64");
        assert_eq!((x86["platform_supported"].clone(), x86["unsupported_reason"].clone()), (json!(false), json!("arch")));
        assert_eq!((x86["display_name"].clone(), x86["version"].clone(), x86["size_bytes"].clone()), (json!("G"), json!("1.0.0"), json!(7)));
        let win = catalog_module_info(&comp(json!({"platforms": ["windows", "linux"]})), "macos", "aarch64");
        assert_eq!(win["unsupported_reason"], "os");
        let any = catalog_module_info(&comp(json!({})), "macos", "aarch64");
        assert_eq!(any["platform_supported"], true);
        assert!(any.get("unsupported_reason").is_none());
    }

    #[tokio::test]
    async fn health_says_whether_the_daemon_stops_with_its_launcher() {
        let done = crate::ipc::internal::into_internal(health_check(axum::http::HeaderMap::new()).await.into_response()).await;
        assert!(done.body["stops_with_launcher"].is_boolean(), "{}", done.body);
        // 업데이터가 새 데몬인지 가리는 기동 시각 (Unix ms, 지금보다 앞)
        let started = done.body["started_at_ms"].as_u64().expect("started_at_ms");
        assert!(started > 0 && started <= saba_chan_updater_lib::live_apply::now_ms(), "{}", done.body);
        #[cfg(not(windows))]
        assert_eq!(done.body["stops_with_launcher"], false);
        // job 밖이면 false, kill-on-close job 안이면 breakaway를 허용하든 말든 true (BREAKAWAY_OK 0x800, SILENT_BREAKAWAY_OK 0x1000)
        assert!(!job_stops_with_it(false, KILL_ON_JOB_CLOSE));
        assert!(!job_stops_with_it(true, 0x800 | 0x1000));
        assert!(job_stops_with_it(true, KILL_ON_JOB_CLOSE));
        assert!(job_stops_with_it(true, KILL_ON_JOB_CLOSE | 0x800));
        assert!(job_stops_with_it(true, KILL_ON_JOB_CLOSE | 0x1000));
    }

    /// 상태는 인스턴스 id로도 묻는다(이름이 바뀌어도 끊기지 않게). 이름으로 불러도 같은 답, 없으면 404
    #[tokio::test]
    async fn status_is_asked_by_instance_id_or_name() {
        let tmp = tempfile::tempdir().unwrap();
        let dir = tmp.path().join("modules").join("game");
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(dir.join("module.toml"), "[module]\nschema_version = 1\nname = \"game\"\nversion = \"1.0.0\"\nentry = \"lifecycle.py\"\n").unwrap();
        std::fs::write(dir.join("lifecycle.py"), "import json\nprint(json.dumps({'success': True, 'status': 'stopped'}))\n").unwrap();
        let modules = tmp.path().join("modules");
        let mut sup = crate::supervisor::Supervisor::new_with_instances_dir(modules.to_str().unwrap(), tmp.path().join("instances").to_str().unwrap());
        sup.initialize().await.unwrap();
        let state = IPCServer::new(Arc::new(RwLock::new(sup)), "127.0.0.1:0", crate::daemon_log::DaemonLogBuffer::new());
        let inst = crate::instance::ServerInstance::new("srv", "game");
        let id = inst.id.clone();
        state.supervisor.write().await.instance_store.add(inst).unwrap();
        let ask = |key: String| {
            let state = state.clone();
            async move { crate::ipc::internal::into_internal(get_instance_status(Path(key), State(state)).await.into_response()).await }
        };

        // 격리 실행이 없는 곳에서는 모듈 status()가 실패 모양으로 답한다. 어느 쪽이든 두 길의 답이 같아야 한다
        let by_id = ask(id.clone()).await;
        assert_eq!(by_id.status, StatusCode::OK, "{}", by_id.body);
        let by_name = ask("srv".into()).await;
        assert_eq!(by_name.body, by_id.body);
        let missing = ask("nope".into()).await;
        assert_eq!(missing.status, StatusCode::NOT_FOUND);
        assert_eq!(missing.body["error_code"], "instance_not_found");
    }

    /// 클라이언트가 실행 파일 고르기 창의 파일 종류를 모듈 선언(`[config] server_executable`)에서 알 수 있다
    #[tokio::test]
    async fn the_module_list_carries_the_declared_server_executable() {
        let tmp = tempfile::tempdir().unwrap();
        let modules = tmp.path().join("modules");
        for (name, config) in [("game", "\n[config]\nserver_executable = \"server.jar\"\n"), ("plain", "")] {
            let dir = modules.join(name);
            std::fs::create_dir_all(&dir).unwrap();
            std::fs::write(dir.join("module.toml"), format!("[module]\nschema_version = 1\nname = \"{name}\"\nversion = \"1.0.0\"\nentry = \"lifecycle.py\"\n{config}")).unwrap();
            std::fs::write(dir.join("lifecycle.py"), "").unwrap();
        }
        let mut sup = crate::supervisor::Supervisor::new_with_instances_dir(modules.to_str().unwrap(), tmp.path().join("instances").to_str().unwrap());
        sup.initialize().await.unwrap();
        let state = IPCServer::new(Arc::new(RwLock::new(sup)), "127.0.0.1:0", crate::daemon_log::DaemonLogBuffer::new());
        let listed = crate::ipc::internal::into_internal(list_modules(State(state)).await).await.body;
        let module = |name: &str| listed["modules"].as_array().unwrap().iter().find(|m| m["name"] == name).cloned().unwrap();
        assert_eq!(module("game")["server_executable"], "server.jar", "{listed}");
        assert!(module("plain").get("server_executable").is_none(), "{listed}");
    }

    /// Windows 폴더 이름은 대소문자를 가리지 않는다: 대소문자만 다른 id도 같은 모듈 폴더다
    #[test]
    fn module_ids_that_differ_only_in_case_share_one_install_lock() {
        let held = module_install_guard("CaseMod");
        assert!(held.is_some());
        assert!(module_install_guard("casemod").is_none());
    }

    /// Windows는 끝의 "."·공백을 떼고 폴더를 찾는다: `game.`은 `game`과 같은 폴더라 잠금을 피해 가지 못하게 거절한다
    #[tokio::test]
    async fn module_ids_that_alias_another_folder_are_refused() {
        let tmp = tempfile::tempdir().unwrap();
        let modules = tmp.path().join("modules");
        std::fs::create_dir_all(&modules).unwrap();
        let sup = crate::supervisor::Supervisor::new_with_instances_dir(modules.to_str().unwrap(), tmp.path().join("instances").to_str().unwrap());
        let state = IPCServer::new(Arc::new(RwLock::new(sup)), "127.0.0.1:0", crate::daemon_log::DaemonLogBuffer::new());
        for id in ["game.", "game "] {
            let removed = crate::ipc::internal::into_internal(remove_module(Path(id.into()), State(state.clone())).await).await.body;
            assert_eq!(removed["error"], "Invalid module ID", "{id:?}: {removed}");
            let installed = crate::ipc::internal::into_internal(install_module_from_manifest(Path(id.into()), State(state.clone())).await).await.body;
            assert_eq!(installed["error"], "Invalid module ID", "{id:?}: {installed}");
        }
    }

    /// 서버가 끝났는데 모니터가 아직 치우지 않은 사이, 목록은 추적 중인 PID만 보고 "실행 중"이라 하고
    /// 개별 상태 조회는 "꺼짐"이라 했다. 두 조회는 같은 판정을 쓴다
    #[cfg(unix)]
    #[tokio::test]
    async fn the_server_list_and_the_status_of_one_server_agree_right_after_it_exits() {
        let tmp = tempfile::tempdir().unwrap();
        let modules = tmp.path().join("modules");
        let dir = modules.join("game");
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(
            dir.join("module.toml"),
            "[module]\nschema_version = 1\nname = \"game\"\nversion = \"1.0.0\"\nentry = \"lifecycle.py\"\n",
        )
        .unwrap();
        std::fs::write(
            dir.join("lifecycle.py"),
            "import json, sys\n\
             if sys.argv[1] == 'get_launch_command':\n    \
             print(json.dumps({'success': True, 'program': '/bin/sh', 'args': ['-c', 'sleep 30'], 'working_dir': '.'}))\n\
             else:\n    print(json.dumps({'success': True, 'issues': []}))\n",
        )
        .unwrap();
        crate::sandbox::consent_test_component(&dir.join("lifecycle.py"));
        let mut sup = crate::supervisor::Supervisor::new_with_instances_dir(modules.to_str().unwrap(), tmp.path().join("instances").to_str().unwrap());
        sup.initialize().await.unwrap();
        let mut inst = crate::instance::ServerInstance::new("srv", "game");
        inst.id = "just-exited".into();
        sup.instance_store.add(inst).unwrap();
        let state = IPCServer::new(Arc::new(RwLock::new(sup)), "127.0.0.1:0", crate::daemon_log::DaemonLogBuffer::new());
        let started = crate::ipc::internal::InternalResponse::from(start(&state, "just-exited").await).body;
        assert_eq!(started["success"], true, "{started}");

        let both = || async {
            let list = crate::ipc::internal::InternalResponse::from(crate::service::servers::list(&state, 0).await).body;
            let one = crate::supervisor::Supervisor::get_server_status(&state.supervisor, "just-exited", "game").await.unwrap();
            (list["servers"][0]["status"].clone(), one["status"].clone())
        };
        assert_eq!(both().await, (json!("running"), json!("running")));

        // 모니터가 돌기 전: managed 프로세스는 끝났고 추적 PID는 남아 있다
        let managed = state.supervisor.read().await.managed_store.get("just-exited").await.unwrap();
        managed.force_kill().await.unwrap();
        assert!(crate::supervisor::process::wait_gone(std::time::Duration::from_secs(10), || managed.is_running()).await);
        assert_eq!(both().await, (json!("stopped"), json!("stopped")));
    }

    #[test]
    fn only_unofficial_or_changed_components_get_a_notice() {
        use saba_chan_updater_lib::integrity::Trust;
        assert!(crate::module_audit::trust_warnings("module", "m", Trust::Verified).is_empty());
        assert_eq!(crate::module_audit::trust_warnings("module", "m", Trust::Unverified), [json!({"code": "module_unverified", "module": "m", "isolated": cfg!(windows)})]);
        assert_eq!(crate::module_audit::trust_warnings("extension", "e", Trust::Modified), [json!({"code": "extension_modified", "extension": "e", "isolated": cfg!(windows)})]);
    }

    #[tokio::test]
    async fn imported_server_applies_the_agreements_accepted_at_creation() {
        let tmp = tempfile::tempdir().unwrap();
        // 데몬의 모듈 폴더 아래에 둔다: 그 밖의 모듈은 Windows 격리 실행에서 인스턴스 폴더 쓰기를 받지 못한다
        // 이름이 겹치지 않는 폴더를 새로 만들어 쓰고 지운다 (`SABA_MODULES_PATH`의 기존 모듈을 건드리지 않게)
        let modules = crate::plugin::resolve_modules_dir();
        let module = tempfile::Builder::new().prefix("import-agreement-").tempdir_in(&modules).unwrap();
        let dir = module.path();
        let name = dir.file_name().unwrap().to_string_lossy().into_owned();
        std::fs::write(
            dir.join("module.toml"),
            format!("[module]\nschema_version = 1\nname = \"{name}\"\nversion = \"1.0.0\"\nentry = \"lifecycle.py\"\n\n[[agreements]]\nid = \"eula\"\nurl = \"https://example.com/eula\"\n"),
        )
        .unwrap();
        std::fs::write(
            dir.join("lifecycle.py"),
            "import json, os, sys\ncfg = json.load(sys.stdin)\nif sys.argv[1] == 'accept_agreements':\n    open(os.path.join(cfg['working_dir'], 'agreed.txt'), 'w').write(','.join(cfg['accepted_agreements']))\nprint(json.dumps({'success': True}))\n",
        )
        .unwrap();
        crate::sandbox::consent_test_component(&dir.join("lifecycle.py"));
        let existing = tmp.path().join("existing");
        std::fs::create_dir_all(&existing).unwrap();
        let mut sup = crate::supervisor::Supervisor::new_with_instances_dir(
            modules.to_str().unwrap(),
            tmp.path().join("instances").to_str().unwrap(),
        );
        sup.initialize().await.unwrap();
        let state = IPCServer::new(Arc::new(RwLock::new(sup)), "127.0.0.1:0", crate::daemon_log::DaemonLogBuffer::new());
        let req = json!({ "name": "old", "module_name": name, "migration_source": existing, "accept_agreements": ["eula"] });
        let resp = super::super::instance::create_instance(State(state.clone()), Json(req)).await.into_response();
        assert!(resp.status().is_success());
        assert_eq!(std::fs::read_to_string(existing.join("agreed.txt")).unwrap(), "eula");
    }

    #[tokio::test]
    async fn server_list_says_which_servers_the_runner_manages() {
        let tmp = tempfile::tempdir().unwrap();
        let modules = tmp.path().join("modules");
        let dir = modules.join("plain");
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(dir.join("module.toml"), "[module]\nschema_version = 1\nname = \"plain\"\nversion = \"1.0.0\"\nentry = \"lifecycle.py\"\n").unwrap();
        std::fs::write(dir.join("lifecycle.py"), "").unwrap();
        let mut sup = crate::supervisor::Supervisor::new_with_instances_dir(
            modules.to_str().unwrap(),
            tmp.path().join("instances").to_str().unwrap(),
        );
        sup.initialize().await.unwrap();
        let default = crate::instance::ServerInstance::new("default", "plain");
        let mut direct = crate::instance::ServerInstance::new("direct", "plain");
        direct.module_settings.insert("managed_start".into(), json!(false));
        sup.instance_store.add(default).unwrap();
        sup.instance_store.add(direct).unwrap();
        let state = IPCServer::new(Arc::new(RwLock::new(sup)), "127.0.0.1:0", crate::daemon_log::DaemonLogBuffer::new());

        let body = crate::ipc::internal::into_internal(list_servers(State(state), Query(ListServersQuery::default())).await).await.body;
        let managed = |name: &str| {
            body["servers"].as_array().unwrap().iter().find(|s| s["name"] == name).unwrap()["managed"].clone()
        };
        assert_eq!(managed("default"), json!(true), "managed is the default (C8b)");
        assert_eq!(managed("direct"), json!(false));
    }

    /// 설정 칸마다 저장된 값(`stored`)은 데몬이 고른다: 공통 칸(port)은 인스턴스 칸, 나머지는 module_settings (F12)
    #[tokio::test]
    async fn server_list_and_instance_carry_the_stored_value_of_each_setting() {
        let tmp = tempfile::tempdir().unwrap();
        let modules = tmp.path().join("modules");
        let dir = modules.join("game");
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(
            dir.join("module.toml"),
            "[module]\nschema_version = 1\nname = \"game\"\nversion = \"1.0.0\"\nentry = \"lifecycle.py\"\n\n\
             [[settings.fields]]\nname = \"port\"\ntype = \"number\"\n\n[[settings.fields]]\nname = \"motd\"\ntype = \"text\"\n\n\
             [[settings.fields]]\nname = \"seed\"\ntype = \"text\"\n",
        )
        .unwrap();
        std::fs::write(dir.join("lifecycle.py"), "").unwrap();
        let mut sup = crate::supervisor::Supervisor::new_with_instances_dir(modules.to_str().unwrap(), tmp.path().join("instances").to_str().unwrap());
        sup.initialize().await.unwrap();
        let mut inst = crate::instance::ServerInstance::new("a", "game");
        inst.port = Some(26000);
        // 낡은 사본: 공통 칸은 인스턴스 칸이 진짜다
        inst.module_settings.insert("port".into(), json!(25565));
        inst.module_settings.insert("motd".into(), json!("hi"));
        let id = inst.id.clone();
        sup.instance_store.add(inst).unwrap();
        let state = IPCServer::new(Arc::new(RwLock::new(sup)), "127.0.0.1:0", crate::daemon_log::DaemonLogBuffer::new());

        let expected = json!({ "port": 26000, "motd": "hi" });
        let list = crate::ipc::internal::into_internal(list_servers(State(state.clone()), Query(ListServersQuery::default())).await).await.body;
        assert_eq!(list["servers"][0]["stored"], expected, "seed has no stored value");
        let one = crate::ipc::internal::into_internal(crate::ipc::handlers::instance::get_instance(axum::extract::Path(id), State(state)).await.into_response()).await.body;
        assert_eq!(one["stored"], expected);
    }

    /// 익스텐션이 맡았다고 답한 서버(`server.list_enrich`의 handled)는 상태·PID·시작 시간·extension_status만 익스텐션 답을 따르고
    /// 나머지 칸은 맡지 않은 서버와 같다
    #[tokio::test]
    async fn server_list_takes_only_the_runtime_state_from_an_extension_that_handles_the_server() {
        let tmp = tempfile::tempdir().unwrap();
        let mut sup = crate::supervisor::Supervisor::new_with_instances_dir(
            tmp.path().join("modules").to_str().unwrap(),
            tmp.path().join("instances").to_str().unwrap(),
        );
        sup.initialize().await.unwrap();
        let mut handled = crate::instance::ServerInstance::new("handled", "plain");
        handled.port = Some(25565);
        handled.extension_data.insert("docker_enabled".into(), json!(true));
        let mut declined = crate::instance::ServerInstance::new("declined", "plain");
        declined.port = Some(25566);
        declined.extension_data.insert("docker_enabled".into(), json!(true));
        let (handled_id, declined_id) = (handled.id.clone(), declined.id.clone());
        sup.instance_store.add(handled).unwrap();
        sup.instance_store.add(declined).unwrap();
        let state = IPCServer::new(Arc::new(RwLock::new(sup)), "127.0.0.1:0", crate::daemon_log::DaemonLogBuffer::new());
        // 캐시에 둔 답을 쓰므로 hook을 실제로 부르지 않는다
        state.extension_status_cache.set(&handled_id, json!({ "handled": true, "status": "starting", "extension_id": "docker", "memory_percent": 25.5 }));
        state.extension_status_cache.set(&declined_id, json!({ "handled": false }));

        let body = crate::ipc::internal::into_internal(list_servers(State(state), Query(ListServersQuery::default())).await).await.body;
        let server = |name: &str| body["servers"].as_array().unwrap().iter().find(|s| s["name"] == name).unwrap().clone();
        let handled = server("handled");
        assert_eq!(handled["status"], json!("starting"));
        assert_eq!(handled["pid"], Value::Null);
        assert_eq!(handled["extension_status"]["docker"]["memory_percent"], json!(25.5));
        assert_eq!(handled["port"], json!(25565));
        assert_eq!(handled["module"], json!("plain"));
        assert_eq!(handled["extension_data"]["docker_enabled"], json!(true));
        assert_eq!(handled["provisioning"], json!(false));
        let declined = server("declined");
        assert_eq!(declined["status"], json!("stopped"));
        assert!(declined.get("extension_status").is_none());
        assert_eq!(declined["port"], json!(25566));
        let without_status: Vec<&str> = ["status", "extension_status", "id", "name", "port", "revision", "pid", "start_time"].to_vec();
        let rest = |v: &Value| {
            let mut v = v.clone();
            for k in &without_status {
                v.as_object_mut().unwrap().remove(*k);
            }
            v
        };
        assert_eq!(rest(&handled), rest(&declined), "the other fields come from the same place");
    }

    /// 목록은 인스턴스 폴더 밖 서버 폴더(가져온 서버)와 되돌릴 원래 설정이 있는지를 알린다
    #[tokio::test]
    async fn server_list_reports_linked_folders() {
        let tmp = tempfile::tempdir().unwrap();
        let mut sup = crate::supervisor::Supervisor::new_with_instances_dir(
            tmp.path().join("modules").to_str().unwrap(),
            tmp.path().join("instances").to_str().unwrap(),
        );
        sup.initialize().await.unwrap();
        let outside = tmp.path().join("server");
        let mut linked = crate::instance::ServerInstance::new("linked", "plain");
        linked.working_dir = Some(outside.to_string_lossy().into_owned());
        let mut inside = crate::instance::ServerInstance::new("inside", "plain");
        inside.working_dir = Some(sup.instance_store.instance_dir(&inside.id).join("server").to_string_lossy().into_owned());
        let linked_id = linked.id.clone();
        sup.instance_store.add(linked).unwrap();
        sup.instance_store.add(inside).unwrap();
        std::fs::create_dir_all(sup.instance_store.instance_dir(&linked_id).join(crate::instance::migration::DIR)).unwrap();
        std::fs::write(sup.instance_store.instance_dir(&linked_id).join(crate::instance::migration::DIR).join(crate::instance::migration::MANIFEST), "{\"files\":[]}").unwrap();
        let state = IPCServer::new(Arc::new(RwLock::new(sup)), "127.0.0.1:0", crate::daemon_log::DaemonLogBuffer::new());

        let body = crate::ipc::internal::into_internal(list_servers(State(state), Query(ListServersQuery::default())).await).await.body;
        let server = |name: &str| body["servers"].as_array().unwrap().iter().find(|s| s["name"] == name).unwrap().clone();
        assert_eq!(server("linked")["linked_folder"], json!(outside.to_string_lossy()));
        assert_eq!(server("linked")["restorable_settings"], json!(true));
        assert!(server("inside").get("linked_folder").is_none_or(|v| v.is_null()));
        assert_eq!(server("inside")["restorable_settings"], json!(false));
    }

    /// 강제 정지 알림은 한 클라이언트가 읽어도 사라지지 않는다: 알림마다 id가 있고 `notices_after`로 본 것 뒤만 받는다.
    /// 매개변수 없이 읽으면(데몬 안의 액션 `instance.list` 등) 남은 것을 모두 보고 아무것도 본 것으로 적지 않는다 (D12). 오래된 알림은 지운다 (L50)
    #[tokio::test]
    async fn every_client_sees_forced_stop_notices_by_id() {
        let tmp = tempfile::tempdir().unwrap();
        let sup = crate::supervisor::Supervisor::new_with_instances_dir(
            tmp.path().join("modules").to_str().unwrap(),
            tmp.path().join("instances").to_str().unwrap(),
        );
        let now = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap().as_millis() as u64;
        let event = |port: u16, timestamp_ms: u64| crate::supervisor::PortConflictStopEvent {
            id: 0,
            stopped_name: "b".into(),
            stopped_id: "b-id".into(),
            existing_name: "a".into(),
            existing_id: "a-id".into(),
            port,
            timestamp_ms,
        };
        sup.record_port_conflict_stops([event(1, now - crate::supervisor::PORT_CONFLICT_STOPS_TTL_MS - 1000), event(2, now)]);
        let state = IPCServer::new(Arc::new(RwLock::new(sup)), "127.0.0.1:0", crate::daemon_log::DaemonLogBuffer::new());
        let list = |q: ListServersQuery| {
            let state = state.clone();
            async move {
                let body = crate::ipc::internal::into_internal(list_servers(State(state), Query(q)).await).await.body;
                body["port_conflict_stops"].as_array().cloned().unwrap_or_default()
            }
        };
        let after = |id: u64| ListServersQuery { notices_after: Some(id), ..Default::default() };

        let plain = list(ListServersQuery::default()).await;
        assert_eq!(plain.len(), 1, "an old notice is gone: {plain:?}");
        let seen = list(after(0)).await;
        assert_eq!(seen.len(), 1, "a client that read nothing still sees it after a plain read");
        let id = seen[0]["id"].as_u64().expect("notices carry an id");
        assert!(id > 0);
        assert!(list(after(id)).await.is_empty(), "nothing after what the client has seen");
        assert_eq!(list(ListServersQuery::default()).await.len(), 1, "a plain read shows what is left every time");

        state.supervisor.read().await.record_port_conflict_stops([event(3, now)]);
        let fresh = list(after(id)).await;
        assert_eq!(fresh.len(), 1);
        assert_eq!(fresh[0]["port"], 3);
        assert!(fresh[0]["id"].as_u64().unwrap() > id);
    }

    /// 아무 클라이언트도 읽지 않는 헤드리스 데몬에서도 알림이 끝없이 쌓이지 않는다
    #[tokio::test]
    async fn forced_stop_notices_keep_only_the_latest() {
        let tmp = tempfile::tempdir().unwrap();
        let sup = crate::supervisor::Supervisor::new_with_instances_dir(
            tmp.path().join("modules").to_str().unwrap(),
            tmp.path().join("instances").to_str().unwrap(),
        );
        let now = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap().as_millis() as u64;
        let event = |port: u16| crate::supervisor::PortConflictStopEvent {
            id: 0,
            stopped_name: "b".into(),
            stopped_id: "b-id".into(),
            existing_name: "a".into(),
            existing_id: "a-id".into(),
            port,
            timestamp_ms: now,
        };
        for port in 0..30 {
            sup.record_port_conflict_stops([event(port)]);
        }
        let kept = sup.port_conflict_stops_after(0);
        assert_eq!(kept.len(), crate::supervisor::PORT_CONFLICT_STOPS_KEPT);
        assert_eq!(kept.last().unwrap().port, 29, "the newest is kept");
    }

    /// 새 코어를 요구하는 카탈로그 모듈은 로드 실패·시작 응답과 같은 코드와 목록으로 거절한다 (클라이언트가 "업데이트가 필요해요"로 보인다)
    #[test]
    fn catalog_module_needing_newer_core_is_refused_with_dependency_code() {
        let requires = std::collections::HashMap::from([("saba-core".to_string(), ">=99.0.0".to_string())]);
        let installed = std::collections::HashMap::from([("saba-core".to_string(), "0.1.9".to_string())]);
        let unmet = saba_chan_updater_lib::unmet_requires("module-future", &requires, &installed);
        let body = unmet_requires_body("future", &unmet);
        assert_eq!((body["success"].clone(), body["module"].clone()), (json!(false), json!("future")));
        assert_eq!(body["error_code"], "module_dependency_unsatisfied", "{body}");
        assert_eq!(body["missing_dependencies"], json!(["saba-core >=99.0.0 (installed 0.1.9)"]), "{body}");
        assert_eq!(body["required_core"], "99.0.0", "{body}");
        let missing = saba_chan_updater_lib::unmet_requires("module-future", &requires, &Default::default());
        assert_eq!(unmet_requires_body("future", &missing)["missing_dependencies"], json!(["saba-core >=99.0.0 (not installed)"]));
    }

    /// 켜진 서버가 쓰는 모듈은 지우지 않는다: 지우면 그 서버를 정지할 수 없게 된다 (EE9)
    #[tokio::test]
    async fn a_module_used_by_a_running_server_is_not_removed() {
        let tmp = tempfile::tempdir().unwrap();
        let modules = tmp.path().join("modules");
        let dir = modules.join("plain");
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(dir.join("module.toml"), "[module]\nschema_version = 1\nname = \"plain\"\nversion = \"1.0.0\"\nentry = \"lifecycle.py\"\n").unwrap();
        std::fs::write(dir.join("lifecycle.py"), "").unwrap();
        let mut sup = crate::supervisor::Supervisor::new_with_instances_dir(modules.to_str().unwrap(), tmp.path().join("instances").to_str().unwrap());
        sup.initialize().await.unwrap();
        let inst = crate::instance::ServerInstance::new("srv", "plain");
        let id = inst.id.clone();
        sup.instance_store.add(inst).unwrap();
        sup.tracker.track(&id, std::process::id()).unwrap();
        let state = IPCServer::new(Arc::new(RwLock::new(sup)), "127.0.0.1:0", crate::daemon_log::DaemonLogBuffer::new());

        let refused = crate::ipc::internal::into_internal(remove_module(Path("plain".into()), State(state.clone())).await.into_response()).await.body;
        assert_eq!(refused["error_code"], "module_in_use", "{refused}");
        assert!(dir.exists(), "the module folder must stay");
        // Windows 경로는 대소문자를 가리지 않는다: 다른 대소문자로 불러도 같은 폴더다
        let upper = crate::ipc::internal::into_internal(remove_module(Path("PLAIN".into()), State(state.clone())).await.into_response()).await.body;
        assert_eq!(upper["error_code"], "module_in_use", "{upper}");

        state.supervisor.read().await.tracker.untrack(&id).unwrap();
        let removed = crate::ipc::internal::into_internal(remove_module(Path("plain".into()), State(state)).await.into_response()).await.body;
        assert_eq!(removed["success"], true, "{removed}");
    }

    /// modules 폴더에 놓은 ZIP 모듈도 제거 API로 지운다 (찾은 모듈의 실제 원본을 지운다)
    #[tokio::test]
    async fn a_zip_module_is_removed_by_the_api() {
        use std::io::Write;
        let tmp = tempfile::tempdir().unwrap();
        let modules = tmp.path().join("modules");
        std::fs::create_dir_all(&modules).unwrap();
        // 설치 문은 프로세스에 하나다: 다른 시험과 겹치지 않는 모듈 id를 쓴다
        let zip_path = modules.join("zip-mod-1.0.zip");
        let mut writer = zip::ZipWriter::new(std::fs::File::create(&zip_path).unwrap());
        let options = zip::write::SimpleFileOptions::default();
        writer.start_file("module.toml", options).unwrap();
        writer.write_all(b"[module]\nschema_version = 1\nname = \"zip-mod\"\nversion = \"1.0.0\"\nentry = \"lifecycle.py\"\n").unwrap();
        writer.start_file("lifecycle.py", options).unwrap();
        writer.finish().unwrap();
        let mut sup = crate::supervisor::Supervisor::new_with_instances_dir(modules.to_str().unwrap(), tmp.path().join("instances").to_str().unwrap());
        sup.initialize().await.unwrap();
        let state = IPCServer::new(Arc::new(RwLock::new(sup)), "127.0.0.1:0", crate::daemon_log::DaemonLogBuffer::new());

        let removed = crate::ipc::internal::into_internal(remove_module(Path("zip-mod".into()), State(state.clone())).await.into_response()).await.body;
        assert_eq!(removed["success"], true, "{removed}");
        assert!(!zip_path.exists());
        assert!(!modules.join(".extracted/zip-mod").exists());
        assert!(state.supervisor.read().await.module_loader.get_module("zip-mod").is_err());
    }

    /// 같은 모듈을 동시에 두 번 설치하지 않고, 설치마다 다른 임시 경로를 쓴다 (D-S2)
    #[tokio::test]
    async fn a_module_is_installed_by_one_request_at_a_time() {
        let tmp = tempfile::tempdir().unwrap();
        let sup = crate::supervisor::Supervisor::new_with_instances_dir(
            tmp.path().join("modules").to_str().unwrap(),
            tmp.path().join("instances").to_str().unwrap(),
        );
        let state = IPCServer::new(Arc::new(RwLock::new(sup)), "127.0.0.1:0", crate::daemon_log::DaemonLogBuffer::new());
        // 다른 시험이 쓰는 모듈 id와 겹치면(설치 문은 프로세스에 하나다) 그쪽이 module_busy를 받는다
        let module = "busy-mod";
        let held = module_install_guard(module).expect("free");
        let body = crate::ipc::internal::into_internal(
            install_module_from_manifest(Path(module.into()), State(state.clone())).await.into_response(),
        )
        .await
        .body;
        assert_eq!(body["error_code"], "module_busy", "{body}");
        let removed = crate::ipc::internal::into_internal(remove_module(Path(module.into()), State(state)).await.into_response()).await.body;
        assert_eq!(removed["error_code"], "module_busy", "{removed}");
        drop(held);
        assert!(module_install_guard(module).is_some(), "refused requests must not keep the lock");

        let (zip_a, staging_a) = module_install_paths(tmp.path(), module);
        let (zip_b, staging_b) = module_install_paths(tmp.path(), module);
        assert!(zip_a != zip_b && staging_a != staging_b);
        let name = staging_a.file_name().unwrap().to_string_lossy().to_string();
        assert!(name.starts_with(".staging-busy-mod-") && zip_a.file_name().unwrap().to_string_lossy().starts_with(".tmp-module-busy-mod-"));
    }

    #[test]
    fn a_module_package_that_does_not_load_keeps_the_installed_module() {
        let tmp = tempfile::tempdir().unwrap();
        let toml = "[module]\nschema_version = 1\nname = \"plain\"\nversion = \"1.0.0\"\nentry = \"lifecycle.py\"\n";
        let target = tmp.path().join("plain");
        std::fs::create_dir_all(&target).unwrap();
        std::fs::write(target.join("module.toml"), toml).unwrap();
        std::fs::write(target.join("lifecycle.py"), "old").unwrap();
        // 진입 파일이 빠진 패키지
        let staging = tmp.path().join(".staging-plain");
        std::fs::create_dir_all(&staging).unwrap();
        std::fs::write(staging.join("module.toml"), toml.replace("1.0.0", "2.0.0")).unwrap();
        assert!(install_staged_module(&staging, &target).is_err());
        assert_eq!(std::fs::read_to_string(target.join("lifecycle.py")).unwrap(), "old");

        std::fs::write(staging.join("lifecycle.py"), "new").unwrap();
        install_staged_module(&staging, &target).unwrap();
        assert_eq!(std::fs::read_to_string(target.join("lifecycle.py")).unwrap(), "new");
        assert!(!staging.exists());
        assert_eq!(std::fs::read_dir(tmp.path()).unwrap().count(), 1, "the old folder is removed after the swap");
    }

    /// 가져오기 전 폴더 검사에서 사바쨩 밖에서 켜 둔 서버를 미리 알린다 (가져오기를 누르기 전에 끄라고 보인다)
    #[cfg(unix)]
    #[tokio::test]
    async fn detecting_a_folder_warns_about_a_server_already_running_from_it() {
        use axum::{extract::State, Json};
        let tmp = tempfile::tempdir().unwrap();
        let server = tmp.path().join("server");
        std::fs::create_dir_all(&server).unwrap();
        let sup = crate::supervisor::Supervisor::new_with_instances_dir(tmp.path().join("modules").to_str().unwrap(), tmp.path().join("instances").to_str().unwrap());
        let state = crate::ipc::IPCServer::new(std::sync::Arc::new(tokio::sync::RwLock::new(sup)), "127.0.0.1:0", crate::daemon_log::DaemonLogBuffer::new());
        let detect = |state: crate::ipc::IPCServer, path: std::path::PathBuf| async move {
            crate::ipc::internal::into_internal(super::detect_module_dir(State(state), Json(json!({ "path": path }))).await.into_response()).await
        };
        let quiet = detect(state.clone(), server.clone()).await;
        assert_eq!(quiet.body["notices"], json!([]), "{}", quiet.body);
        let mut outside = std::process::Command::new("sleep").arg("30").current_dir(&server).spawn().unwrap();
        // 링크를 거친 경로로 골라도 알아본다 (macOS 임시 폴더 /var는 /private/var의 링크라 프로세스 경로와 다르다)
        let linked = tmp.path().join("linked");
        std::os::unix::fs::symlink(&server, &linked).unwrap();
        let busy = detect(state, linked).await;
        outside.kill().ok();
        outside.wait().ok();
        assert_eq!(busy.body["notices"], json!(["server_running"]), "{}", busy.body);
    }

    #[tokio::test]
    async fn start_waits_for_module_agreements_and_only_this_pc_can_accept() {
        let tmp = tempfile::tempdir().unwrap();
        let modules = tmp.path().join("modules");
        let dir = modules.join("game");
        std::fs::create_dir_all(dir.join("locales")).unwrap();
        std::fs::write(
            dir.join("module.toml"),
            "[module]\nschema_version = 1\nname = \"game\"\nversion = \"1.0.0\"\nentry = \"lifecycle.py\"\n\n[[agreements]]\nid = \"eula\"\nurl = \"https://example.com/eula\"\n",
        )
        .unwrap();
        std::fs::write(dir.join("lifecycle.py"), "").unwrap();
        std::fs::write(dir.join("locales").join("en.json"), r#"{ "agreements": { "eula": "Game EULA" } }"#).unwrap();
        let mut sup = crate::supervisor::Supervisor::new_with_instances_dir(
            modules.to_str().unwrap(),
            tmp.path().join("instances").to_str().unwrap(),
        );
        sup.initialize().await.unwrap();
        let state = IPCServer::new(Arc::new(RwLock::new(sup)), "127.0.0.1:0", crate::daemon_log::DaemonLogBuffer::new());
        let body = |resp: axum::response::Response| async move {
            let status = resp.status();
            let bytes = axum::body::to_bytes(resp.into_body(), usize::MAX).await.unwrap();
            (status, serde_json::from_slice::<Value>(&bytes).unwrap())
        };

        // 만들 때 동의하지 않았으면 켜지 않는다
        let created = super::super::instance::create_instance(State(state.clone()), Json(json!({ "name": "a", "module_name": "game" }))).await.into_response();
        let (status, created) = body(created).await;
        assert!(status.is_success(), "{created}");
        let id = created["id"].as_str().unwrap().to_string();
        let (status, refused) = body(start_instance_handler(Path(id.clone()), State(state.clone())).await).await;
        assert_eq!(status, StatusCode::CONFLICT);
        assert_eq!(refused["error_code"], "agreement_required");
        assert_eq!(refused["agreements"][0]["id"], "eula");
        assert_eq!(refused["agreements"][0]["url"], "https://example.com/eula");
        assert!(refused["error"].as_str().unwrap().contains(refused["agreements"][0]["label"].as_str().unwrap()));

        // 만들 때 동의하면 기록된다. 모르는 약관은 400
        let ok = super::super::instance::create_instance(State(state.clone()), Json(json!({ "name": "b", "module_name": "game", "accept_agreements": ["eula"] }))).await.into_response();
        let (_, ok) = body(ok).await;
        let b = ok["id"].as_str().unwrap().to_string();
        assert!(state.supervisor.read().await.missing_agreements(&b, "en").is_empty());
        let bad = super::super::instance::create_instance(State(state.clone()), Json(json!({ "name": "c", "module_name": "game", "accept_agreements": ["tos"] }))).await.into_response();
        assert_eq!(bad.status(), StatusCode::BAD_REQUEST);
        let install = json!({ "version": "1", "install_dir": tmp.path().join("files"), "accept_agreements": ["tos"] });
        let bad = super::super::managed::install_server_handler(Path("game".into()), State(state.clone()), Json(install)).await.into_response();
        assert_eq!(bad.status(), StatusCode::BAD_REQUEST);

        // 원격(디스코드·챗봇)에서는 동의를 받지 않는다
        let remote = crate::audit::with_origin(
            crate::audit::Origin::new("discord"),
            super::super::managed::accept_agreements_handler(Path(id.clone()), State(state.clone()), Json(json!({ "accept": ["eula"] }))),
        )
        .await;
        let (status, remote) = body(remote).await;
        assert_eq!((status, remote["error_code"].as_str()), (StatusCode::FORBIDDEN, Some("agreement_local_only")));
        assert_eq!(state.supervisor.read().await.missing_agreements(&id, "en").len(), 1);
    }
}
