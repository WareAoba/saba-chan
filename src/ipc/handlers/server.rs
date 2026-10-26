use super::LooseJson;
use axum::{
    extract::{Path, Query, State},
    http::StatusCode,
    response::IntoResponse,
    Json,
};
use serde_json::{json, Value};

use super::super::{
    IPCServer, ExtensionInfo, ExtensionListResponse, PortConflictInfo, ProtocolsInfo, ServerInfo, ServerListResponse,
    ServerStartRequest, ServerStopRequest,
};

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
    (StatusCode::OK, Json(json!({ "ok": true, "version": version, "pid": std::process::id(), "token": token, "proof": proof, "stops_with_launcher": stops_with_launcher() })))
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
    /// `keep`이면 포트 충돌 강제 정지 알림을 보여 주기만 하고 비우지 않는다 (알림을 사람에게 보이지 않는 조회용)
    pub notices: Option<String>,
    /// 이 번호 뒤의 포트 충돌 강제 정지 알림만 받는다. 읽어도 다른 클라이언트 몫을 지우지 않는다 (L50)
    pub notices_after: Option<u64>,
}

/// GET /api/servers - 모든 서버 목록 (인스턴스 기반). 클라이언트(GUI·CLI)가 부르며, 포트 충돌 강제 정지 알림을 읽어 비운다
pub async fn list_servers(State(state): State<IPCServer>, Query(q): Query<ListServersQuery>) -> axum::response::Response {
    let notices = match (q.notices_after, q.notices.as_deref()) {
        (Some(after), _) => Notices::After(after),
        (None, Some("keep")) => Notices::After(0),
        (None, _) => Notices::Legacy,
    };
    server_list(&state, notices).await
}

/// 목록에 실을 포트 충돌 강제 정지 알림
enum Notices {
    /// 번호가 이것보다 큰 알림 모두 (본 것으로 적지 않는다)
    After(u64),
    /// 매개변수 없는 옛 클라이언트: 이 방식으로 아직 보이지 않은 것만
    Legacy,
}

/// 데몬 안에서 부르는 서버 목록 (릴레이 하트비트·채팅). 강제 정지 알림은 클라이언트 몫이라 비우지 않는다
pub async fn list_servers_internal(state: &IPCServer) -> axum::response::Response {
    server_list(state, Notices::After(0)).await
}

async fn server_list(state: &IPCServer, notices: Notices) -> axum::response::Response {
    let state = state.clone();
    // ── Phase 1: supervisor lock을 최소 시간만 잡고 필요한 데이터 복사 ──
    struct InstanceSnapshot {
        id: String,
        name: String,
        module_name: String,
        executable_path: Option<String>,
        working_dir: Option<String>,
        port: Option<u16>,
        rest_host: Option<String>,
        rest_port: Option<u16>,
        rest_username: Option<String>,
        rest_password: Option<String>,
        protocol_mode: String,
        module_settings: std::collections::HashMap<String, serde_json::Value>,
        server_version: Option<String>,
        extension_data: std::collections::HashMap<String, serde_json::Value>,
        extra_args: Vec<String>,
        memory_limit_mb: Option<u64>,
        revision: String,
        instance_dir: String,
        linked_folder: Option<String>,
        restorable_settings: bool,
        process_patterns: Vec<String>,
        // Native 모드 데이터
        pid: Option<u32>,
        start_time: Option<u64>,
        managed: bool,
    }

    let (snapshots, all_instances, module_protocols): (Vec<InstanceSnapshot>, Vec<_>, _) = {
        let supervisor = state.supervisor.read().await;
        let instances = supervisor.instance_store.list();
        let all_instances = instances.to_vec();
        let module_protocols = supervisor.build_module_protocols_map();
        let snapshots =         instances.iter().map(|instance| {
            let pid = supervisor.tracker.get_pid(&instance.id).ok();
            let start_time = if pid.is_some() {
                supervisor.tracker.get_start_time(&instance.id).ok()
            } else {
                None
            };
            let process_patterns = supervisor.module_loader.get_module(&instance.module_name)
                .map(|m| m.metadata.process_patterns.clone())
                .unwrap_or_default();
            InstanceSnapshot {
                id: instance.id.clone(),
                name: instance.name.clone(),
                module_name: instance.module_name.clone(),
                executable_path: instance.executable_path.clone(),
                working_dir: instance.working_dir.clone(),
                port: instance.port,
                rest_host: instance.rest_host.clone(),
                rest_port: instance.rest_port,
                rest_username: instance.rest_username.clone(),
                rest_password: instance.rest_password.clone(),
                protocol_mode: instance.protocol_mode.clone(),
                module_settings: instance.module_settings.clone(),
                server_version: instance.server_version.clone(),
                extension_data: instance.extension_data.clone(),
                extra_args: instance.extra_args.clone(),
                memory_limit_mb: instance.memory_limit_mb,
                revision: instance.settings_revision(),
                instance_dir: supervisor.instance_store.instance_dir(&instance.id).to_string_lossy().to_string(),
                // 폴더 판정은 아래 락 밖에서
                linked_folder: None,
                restorable_settings: false,
                process_patterns,
                managed: supervisor.uses_managed_start(instance),
                pid,
                start_time,
            }
        }).collect();
        (snapshots, all_instances, module_protocols)
    };
    // supervisor read lock 해제됨 — 이후 extension dispatch에 의해 블로킹되어도
    // 다른 API 핸들러가 supervisor write lock을 획득할 수 있음

    // 가져온 서버 폴더 판정은 실제 경로를 찾는 막는 호출이라(꺼진 네트워크 드라이브면 오래 걸린다) 데몬 락 밖, 런타임 밖에서
    let snapshots = match tokio::task::spawn_blocking(move || {
        let mut snapshots = snapshots;
        for s in &mut snapshots {
            let instance_dir = std::path::Path::new(&s.instance_dir);
            s.linked_folder = crate::instance::migration::linked_folder(s.working_dir.as_deref(), instance_dir);
            s.restorable_settings = crate::instance::migration::saved(instance_dir);
        }
        snapshots
    })
    .await
    {
        Ok(snapshots) => snapshots,
        Err(e) => {
            let error = json!({ "success": false, "error": format!("Could not read the server folders: {}", e) });
            return (StatusCode::INTERNAL_SERVER_ERROR, Json(error)).into_response();
        }
    };

    // ── Phase 2: Extension hooks (lock 없이 실행) ──
    let mut servers = Vec::new();

    for instance in &snapshots {
        // ── Extension hook: server.list_enrich ──
        // Extension이 런타임 상태/통계를 제공할 수 있음 (TTL 캐시 적용)
        let mut ext_handled = false;
        let has_ext_hooks = !instance.extension_data.is_empty();
        if has_ext_hooks {
            // 캐시 확인 — TTL 내 결과가 있으면 Python 프로세스 스폰 생략
            let enrich_result = if let Some(cached_val) = state.extension_status_cache.get(&instance.id) {
                cached_val
            } else {
                let ext_mgr = state.extension_manager.clone();
                let ctx = serde_json::json!({
                    "instance_id": &instance.id,
                    "instance_name": &instance.name,
                    "module": &instance.module_name,
                    "extension_data": &instance.extension_data,
                    "instance_dir": &instance.instance_dir,
                    "process_patterns": &instance.process_patterns,
                });
                let results = crate::extension::run_hook_timed(&ext_mgr, "server.list_enrich", ctx, 10).await;
                // 첫 번째 성공 응답만 캐시한다 (실패는 다음 목록 요청에서 다시 시도)
                match results.into_iter().find_map(|(_id, r)| r.ok()) {
                    Some(result) => {
                        state.extension_status_cache.set(&instance.id, result.clone());
                        result
                    }
                    None => json!({"handled": false}),
                }
            };

            // handled: true인 경우만 Extension이 서버 정보를 제공
            let is_handled = enrich_result.get("handled")
                .and_then(|h| h.as_bool()) == Some(true);

            if is_handled {
                let res = &enrich_result;
                let ext_status = res.get("status").and_then(|s| s.as_str())
                    .unwrap_or("stopped").to_string();
                let provision_entry = state.provision_tracker.get(&instance.name);
                let provisioning = provision_entry.is_some();
                let actively_provisioning = provision_entry.as_ref().map(|p| !p.done).unwrap_or(false);
                let status = if actively_provisioning { "provisioning".to_string() } else { ext_status };
                let last_api = state.api_actions.get(&instance.name)
                    .or_else(|| state.api_actions.get(&instance.id));

                // Extension 런타임 상태를 extension_status에 저장
                let mut extension_status = std::collections::HashMap::new();
                // hook 응답에서 extension_id 필드가 있으면 해당 키로, 아니면 "default"
                let ext_key = res.get("extension_id")
                    .and_then(|v| v.as_str())
                    .unwrap_or("default")
                    .to_string();
                extension_status.insert(ext_key, enrich_result.clone());

                servers.push(ServerInfo {
                    id: instance.id.clone(),
                    name: instance.name.clone(),
                    module: instance.module_name.clone(),
                    status,
                    pid: None,
                    start_time: None,
                    executable_path: instance.executable_path.clone(),
                    working_dir: instance.working_dir.clone(),
                    port: instance.port,
                    rest_host: instance.rest_host.clone(),
                    rest_port: instance.rest_port,
                    rest_username: instance.rest_username.clone(),
                    rest_password: instance.rest_password.clone(),
                    protocol_mode: instance.protocol_mode.clone(),
                    module_settings: instance.module_settings.clone(),
                    server_version: instance.server_version.clone(),
                    last_api_action: last_api,
                    provisioning,
                    managed: instance.managed,
                    extension_status,
                    extension_data: instance.extension_data.clone(),
                    extra_args: instance.extra_args.clone(),
                    memory_limit_mb: instance.memory_limit_mb,
                    revision: instance.revision.clone(),
                    port_conflicts: vec![],
                    slots: Default::default(),
                    players: None,
                    unresponsive: None,
                    linked_folder: instance.linked_folder.clone(),
                    restorable_settings: instance.restorable_settings,
                });
                ext_handled = true;
            }
        }
        if ext_handled {
            continue;
        }

        // Native 모드: 스냅샷에서 PID 및 시작 시간 사용
        let (status, start_time) = if instance.pid.is_some() {
            ("running".to_string(), instance.start_time)
        } else {
            ("stopped".to_string(), None)
        };

        // API(CLI/Discord)를 통한 마지막 시작/정지 타임스탬프 조회
        let last_api = state.api_actions.get(&instance.name)
            .or_else(|| state.api_actions.get(&instance.id));

        // 프로비저닝 진행 여부 확인 (tracker entry 존재 = provisioning UI 표시)
        let provision_entry = state.provision_tracker.get(&instance.name);
        let provisioning = provision_entry.is_some();
        let actively_provisioning = provision_entry.as_ref().map(|p| !p.done).unwrap_or(false);

        // 프로비저닝 중이면 상태를 "provisioning"으로 오버라이드
        let status = if actively_provisioning { "provisioning".to_string() } else { status };

        servers.push(ServerInfo {
            id: instance.id.clone(),
            name: instance.name.clone(),
            module: instance.module_name.clone(),
            status,
            pid: instance.pid,
            start_time,
            executable_path: instance.executable_path.clone(),
            working_dir: instance.working_dir.clone(),
            port: instance.port,
            rest_host: instance.rest_host.clone(),
            rest_port: instance.rest_port,
            rest_username: instance.rest_username.clone(),
            rest_password: instance.rest_password.clone(),
            protocol_mode: instance.protocol_mode.clone(),
            module_settings: instance.module_settings.clone(),
            server_version: instance.server_version.clone(),
            last_api_action: last_api,
            provisioning,
            managed: instance.managed,
            extension_status: std::collections::HashMap::new(),
            extension_data: instance.extension_data.clone(),
            extra_args: instance.extra_args.clone(),
            memory_limit_mb: instance.memory_limit_mb,
            revision: instance.revision.clone(),
            port_conflicts: vec![],
            slots: Default::default(),
            players: None,
            unresponsive: None,
            linked_folder: instance.linked_folder.clone(),
            restorable_settings: instance.restorable_settings,
        });
    }

    // ── 데이터 slot (Q-7): 조건 판정과 값 찾기는 여기서 한 번만 ──
    {
        let ext_mgr = state.extension_manager.read().await;
        for srv in servers.iter_mut() {
            srv.slots = ext_mgr.server_slots(&srv.extension_data, &srv.extension_status);
        }
    }

    // ── 접속자 수 (Q-5): 모듈이 알려 준 마지막 값, 켜진 서버만 ──
    for srv in servers.iter_mut().filter(|s| s.status == "running") {
        srv.players = crate::supervisor::players::get(&srv.id);
        srv.unresponsive = crate::supervisor::players::unresponsive(&srv.id, srv.pid, std::time::Instant::now());
    }

    // ── 포트 충돌 계산: 시작 시 검사(check_port_conflicts)와 같은 기준 ──
    // 모듈이 지원하지 않는 프로토콜의 포트는 제외한다. 목록은 "켜면 부딪힐" 서버를
    // 미리 보여주는 용도라 정지된 서버끼리도 비교한다(시작 시에는 실행 중인 것만).
    {
        let every_id: std::collections::HashSet<String> =
            all_instances.iter().map(|i| i.id.clone()).collect();
        for srv in servers.iter_mut() {
            let Some(target) = all_instances.iter().find(|i| i.id == srv.id) else { continue };
            srv.port_conflicts = crate::validator::check_port_conflicts(
                target,
                &all_instances,
                &every_id,
                Some(&module_protocols),
            )
            .into_iter()
            .map(|c| PortConflictInfo {
                port: c.port,
                port_type: c.port_type,
                conflict_name: c.conflicting_instance_name,
                conflict_id: c.conflicting_instance_id,
                conflict_port_type: c.conflicting_port_type,
            })
            .collect();
        }
    }

    // 포트 충돌 강제 정지 이벤트 (짧은 read lock). 옛 클라이언트 요청만 본 것으로 적는다
    let port_conflict_stops = {
        let supervisor = state.supervisor.read().await;
        match notices {
            Notices::After(after) => supervisor.port_conflict_stops_after(after),
            Notices::Legacy => supervisor.drain_port_conflict_stops(),
        }
    };

    Json(ServerListResponse { servers, port_conflict_stops }).into_response()
}

/// GET /api/modules - 모든 모듈 목록
/// 모듈 하나를 API 응답 모양으로 (아이콘은 base64 data URL로)
/// 지금 설치된 컴포넌트 버전 (`versions.toml`과 같은 키: saba-core, module-<이름>, ext-<id>, gui, cli ...).
/// 업데이터가 기록한 installed-manifest를 바탕으로, 데몬이 직접 아는 값(코어, 로드된 모듈, 발견된 익스텐션)으로 덮는다 (U11, M9).
pub async fn component_versions(state: &IPCServer) -> std::collections::BTreeMap<String, String> {
    let mut versions: std::collections::BTreeMap<String, String> =
        saba_chan_updater_lib::UpdateManager::load_installed_manifest().into_iter().collect();
    versions.insert(crate::supervisor::module_loader::CORE_COMPONENT.to_string(), crate::supervisor::module_loader::DAEMON_VERSION.to_string());
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
        "version": crate::supervisor::module_loader::DAEMON_VERSION,
        "components": component_versions(&state).await,
    }))
}

/// 모듈 폴더의 신뢰 표시 (설치 기록의 키는 폴더 이름)
pub(crate) fn module_trust(path: &str) -> saba_chan_updater_lib::integrity::Trust {
    let folder = std::path::Path::new(path).file_name().map(|n| n.to_string_lossy().to_string()).unwrap_or_default();
    saba_chan_updater_lib::integrity::trust_of(
        &format!("{}{}", saba_chan_updater_lib::relay::MODULE_PREFIX, folder),
        std::path::Path::new(path),
    )
}

/// 서명된 카탈로그에서 받지 않았거나 받은 뒤 바뀐 구성요소를 쓸 때 응답에 싣는 알림 (실행은 막지 않는다)
pub(crate) fn trust_warnings(kind: &str, id: &str, trust: saba_chan_updater_lib::integrity::Trust) -> Vec<serde_json::Value> {
    use saba_chan_updater_lib::integrity::Trust;
    let code = match trust {
        Trust::Verified => return Vec::new(),
        Trust::Modified => format!("{}_modified", kind),
        Trust::Unverified => format!("{}_unverified", kind),
    };
    // isolated: 이 플랫폼에서 격리된 환경(허용된 폴더만)에서 돈다
    vec![json!({ "code": code, kind: id, "isolated": crate::sandbox::ENFORCED })]
}

fn module_info(m: crate::supervisor::module_loader::LoadedModule, versions: &std::collections::BTreeMap<String, String>) -> ExtensionInfo {
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
    let trust = module_trust(&m.path);
    let platform_supported = m.metadata.supports_os(std::env::consts::OS);
    let meta = m.metadata;
    ExtensionInfo {
        platforms: meta.platforms,
        platform_supported,
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
        functions: None,
    }
}

async fn module_list_response(
    modules: anyhow::Result<Vec<crate::supervisor::module_loader::LoadedModule>>,
    failed: Vec<crate::supervisor::module_loader::ModuleLoadError>,
    versions: &std::collections::BTreeMap<String, String>,
) -> axum::response::Response {
    match modules {
        Ok(modules) => {
            // 구현한 함수 목록 (MK1): 캐시에 없으면 짧게만 기다린다(느린 모듈은 null, 묻기는 뒤에서 마저). 데몬 락 밖에서, 모듈끼리 동시에
            let asks: Vec<_> = modules.iter().map(|m| tokio::spawn(crate::plugin::module_functions_for_list(m.entry_path()))).collect();
            let mut modules: Vec<ExtensionInfo> = modules.into_iter().map(|m| module_info(m, versions)).collect();
            for (info, ask) in modules.iter_mut().zip(asks) {
                info.functions = ask.await.ok().flatten();
            }
            (StatusCode::OK, Json(ExtensionListResponse { modules, failed })).into_response()
        }
        Err(e) => {
            let error = json!({ "error": format!("Failed to list modules: {}", e) });
            (StatusCode::INTERNAL_SERVER_ERROR, Json(error)).into_response()
        }
    }
}

pub async fn list_modules(State(state): State<IPCServer>) -> impl IntoResponse {
    let versions = component_versions(&state).await;
    let (modules, failed) = {
        let supervisor = state.supervisor.read().await;
        (supervisor.list_modules(), supervisor.module_loader.load_errors())
    };
    module_list_response(modules, failed, &versions).await
}

/// POST /api/modules/refresh - 모듈 캐시를 새로고침하고 다시 발견
pub async fn refresh_modules(State(state): State<IPCServer>) -> impl IntoResponse {
    // 문자열로 저장된 값의 설정이 list가 되었으면 옮긴다 (06 §8.1)
    let modules = {
        let mut supervisor = state.supervisor.write().await;
        let modules = supervisor.refresh_modules();
        supervisor.migrate_list_settings();
        modules
    };
    if let Ok(m) = &modules {
        tracing::info!("Module cache refreshed. Found {} modules", m.len());
        crate::plugin::prefetch_module_functions(m.iter().map(|m| m.entry_path()).collect());
    }
    let versions = component_versions(&state).await;
    let failed = state.supervisor.read().await.module_loader.load_errors();
    module_list_response(modules, failed, &versions).await
}

/// GET /api/modules/schema - module.toml 스키마 v1 (JSON Schema)
pub async fn module_schema() -> impl IntoResponse {
    Json(crate::supervisor::module_loader::module_json_schema())
}

/// POST /api/modules/detect `{path}` - 이 폴더가 어느 모듈의 게임 서버 폴더인지
/// (`[detection] dir_signatures`가 전부 있는 모듈). 기존 서버 가져오기에서 쓴다.
pub async fn detect_module_dir(
    State(state): State<IPCServer>,
    Json(body): Json<serde_json::Value>,
) -> impl IntoResponse {
    let Some(path) = body.get("path").and_then(|v| v.as_str()) else {
        return (StatusCode::BAD_REQUEST, Json(json!({ "error": "path is required" }))).into_response();
    };
    let dir = std::path::Path::new(path);
    if !dir.is_dir() {
        return (StatusCode::NOT_FOUND, Json(json!({ "error": format!("not a folder: {}", path) }))).into_response();
    }
    let supervisor = state.supervisor.read().await;
    let modules = supervisor.list_modules().unwrap_or_default();
    let matches: Vec<_> = modules
        .iter()
        .filter(|m| m.matches_dir(dir))
        .map(|m| json!({ "name": m.metadata.name, "display_name": m.metadata.display_name }))
        .collect();
    (StatusCode::OK, Json(json!({ "matches": matches }))).into_response()
}

/// GET /api/module/:name - 모듈 메타데이터 조회 (별명 포함)
/// GET /api/module/:name/audit — 모듈 Python 코드 검사 (M12): `{declared, findings, undeclared}`.
/// `undeclared`가 비어 있지 않으면 모듈이 선언하지 않은 능력을 쓴다 (사용자에게 보여 줄 것)
pub async fn audit_module(Path(name): Path<String>, State(state): State<IPCServer>) -> impl IntoResponse {
    let module = state.supervisor.read().await.module_loader.get_module(&name).ok();
    let Some(module) = module else {
        return (StatusCode::NOT_FOUND, Json(json!({ "error": format!("Module '{}' not found", name), "error_code": "not_found" }))).into_response();
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
        Err(e) => (StatusCode::INTERNAL_SERVER_ERROR, Json(json!({ "error": format!("{:#}", e), "error_code": "audit_failed" }))).into_response(),
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
            None => (
                StatusCode::NOT_FOUND,
                Json(json!({ "error": format!("Module '{}' not found", name) })),
            )
                .into_response(),
        },
        Err(e) => (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(json!({ "error": format!("Failed to list modules: {}", e) })),
        )
            .into_response(),
    }
}

/// GET /api/modules/manifest - 설치할 수 있는 모듈 목록 (최신 릴리즈의 서명된 manifest, U9e)
///
/// `{ok, manifest: {modules: {<이름>: {version, display_name, description, ...}}}}`
pub async fn fetch_module_manifest() -> impl IntoResponse {
    match saba_chan_updater_lib::relay::latest_catalog(None, &crate::ipc::updates::load_updater_config().channel).await {
        Ok(catalog) => {
            let mut modules = serde_json::Map::new();
            if let Some(manifest) = &catalog {
                for (name, comp) in manifest.addons(saba_chan_updater_lib::relay::MODULE_PREFIX) {
                    let mut info = match &comp.meta {
                        Some(serde_json::Value::Object(meta)) => meta.clone(),
                        _ => serde_json::Map::new(),
                    };
                    info.insert("version".into(), json!(comp.version));
                    info.insert("sha256".into(), json!(comp.sha256));
                    info.insert("size_bytes".into(), json!(comp.size_bytes));
                    modules.insert(name.to_string(), serde_json::Value::Object(info));
                }
            }
            (StatusCode::OK, Json(json!({ "ok": true, "manifest": { "modules": modules } }))).into_response()
        }
        Err(e) => (StatusCode::OK, Json(json!({ "ok": false, "error": e.to_string() }))).into_response(),
    }
}

/// POST /api/modules/manifest/:id/install - 최신 릴리즈에서 모듈을 받아 설치한다 (SHA-256 검증)
pub async fn install_module_from_manifest(
    Path(module_id): Path<String>,
    State(state): State<IPCServer>,
) -> impl IntoResponse {
    let fail = |error: String| (StatusCode::OK, Json(json!({ "ok": false, "error": error }))).into_response();
    if !crate::utils::is_single_path_component(&module_id) {
        return fail("Invalid module ID".into());
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
    .unwrap_or_else(|e| fail(e.to_string()))
}

/// 카탈로그 모듈의 `requires`를 못 맞춘 거절. 로드 실패·시작 응답과 같은 코드와 목록 꼴(`"saba-core >=0.1.10 (installed 0.1.9)"`)
fn unmet_requires_body(module_id: &str, unmet: &[saba_chan_updater_lib::DependencyIssue]) -> Value {
    json!({
        "ok": false,
        "error": unmet.iter().map(|i| i.message.as_str()).collect::<Vec<_>>().join("; "),
        "error_code": "module_dependency_unsatisfied",
        "module": module_id,
        "missing_dependencies": unmet
            .iter()
            .map(|i| format!("{} {} ({})", i.required_component, i.required_version,
                i.installed_version.as_ref().map_or("not installed".to_string(), |v| format!("installed {v}"))))
            .collect::<Vec<_>>(),
    })
}

async fn install_module_locked(state: IPCServer, module_id: String) -> axum::response::Response {
    let fail = |error: String| (StatusCode::OK, Json(json!({ "ok": false, "error": error }))).into_response();
    let catalog = match saba_chan_updater_lib::relay::latest_catalog(None, &crate::ipc::updates::load_updater_config().channel).await {
        Ok(Some(m)) => m,
        Ok(None) => return fail("No release is published".into()),
        Err(e) => return fail(e.to_string()),
    };
    let Some(comp) = catalog
        .components
        .get(&format!("{}{}", saba_chan_updater_lib::relay::MODULE_PREFIX, module_id))
        .cloned()
    else {
        return fail(format!("Module '{}' is not in the latest release", module_id));
    };
    // 설치된 것보다 낮은 버전은 받지 않는다
    let previous = state.supervisor.read().await.module_loader.get_module(&module_id).ok();
    let previously_required = previous.as_ref().map(required_extensions).unwrap_or_default();
    let installed = previous.map(|m| m.metadata.version.clone());
    if let Some(installed) = installed.filter(|v| saba_chan_updater_lib::version::is_downgrade(v, &comp.version)) {
        return fail(format!("Module '{}' {} is older than the installed {}", module_id, comp.version, installed));
    }
    // 새 코어를 요구하는 판은 받지 않는다 (업데이터 적용과 같은 검사)
    if let Some(requires) = &comp.requires {
        let installed: std::collections::HashMap<String, String> = component_versions(&state).await.into_iter().collect();
        let key = format!("{}{}", saba_chan_updater_lib::relay::MODULE_PREFIX, module_id);
        let unmet = saba_chan_updater_lib::unmet_requires(&key, requires, &installed);
        if !unmet.is_empty() {
            return (StatusCode::OK, Json(unmet_requires_body(&module_id, &unmet))).into_response();
        }
    }

    // 여기부터 모듈 폴더에 쓴다: 폴더를 옮기는 중이면 끝나기를 기다리고, 끝날 때까지 옮기지 못하게 한다
    let _dirs = super::paths::components_in_use().await;
    let modules_dir = {
        let supervisor = state.supervisor.read().await;
        std::path::PathBuf::from(supervisor.module_loader.modules_dir())
    };
    let (zip_path, staging) = module_install_paths(&modules_dir, &module_id);
    let target_dir = modules_dir.join(&module_id);

    let result: anyhow::Result<()> = async {
        let http = reqwest::Client::new();
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
        return fail(format!("{:#}", e));
    }
    let required = {
        let mut supervisor = state.supervisor.write().await;
        let _ = supervisor.refresh_modules();
        supervisor.migrate_list_settings();
        supervisor.module_loader.get_module(&module_id).ok().as_ref().map(required_extensions).unwrap_or_default()
    };
    // 이 모듈이 쓰는 익스텐션을 받고 켠다 (새로 필요해진 것만 켠다: 예전 판부터 쓰던 것은 사용자가 껐을 수 있다)
    let extensions = super::extension::prepare_for_module(&state, &required, &previously_required).await;
    (StatusCode::OK, Json(json!({ "ok": true, "module_id": module_id, "version": comp.version, "extensions": extensions })))
        .into_response()
}

/// 모듈 `[install] requires_extensions`
pub(crate) fn required_extensions(module: &crate::supervisor::module_loader::LoadedModule) -> Vec<String> {
    module.metadata.install.as_ref().map(|i| i.requires_extensions.clone()).unwrap_or_default()
}

/// 모듈 하나의 설치·제거는 한 번에 하나만 (같은 모듈을 GUI·CLI가 동시에 설치하면 서로의 파일을 지운다).
/// 키는 소문자로: Windows에서는 대소문자만 다른 id가 같은 폴더다
pub(crate) fn module_install_guard(module_id: &str) -> Option<super::backup::InstanceLock> {
    super::backup::InstanceLock::take(&format!("module-install:{}", module_id.to_lowercase()))
}

pub(crate) fn module_busy(module_id: &str) -> axum::response::Response {
    let error = format!("Module '{}' is already being installed or removed", module_id);
    (StatusCode::OK, Json(json!({ "ok": false, "error": error, "error_code": "module_busy" }))).into_response()
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
    crate::supervisor::module_loader::load_module_dir(staging).map_err(|e| e.context("package is not a loadable module"))?;
    crate::utils::swap_dir(staging, target_dir)
}

/// DELETE /api/modules/:id — 모듈 제거 (디렉토리 삭제 + 캐시 갱신)
pub async fn remove_module(
    Path(module_id): Path<String>,
    State(state): State<IPCServer>,
) -> impl IntoResponse {
    // 경로 탐색 방지
    if !crate::utils::is_single_path_component(&module_id) {
        return (StatusCode::OK, Json(json!({ "ok": false, "error": "Invalid module ID" }))).into_response();
    }
    let Some(_removing) = module_install_guard(&module_id) else {
        return module_busy(&module_id);
    };
    let _dirs = super::paths::components_in_use().await;
    let modules_dir = {
        let supervisor = state.supervisor.read().await;
        std::path::PathBuf::from(supervisor.module_loader.modules_dir())
    };
    let module_path = modules_dir.join(&module_id);
    if !module_path.exists() {
        return (StatusCode::OK, Json(json!({ "ok": false, "error": "Module not found" }))).into_response();
    }
    if let Err(e) = std::fs::remove_dir_all(&module_path) {
        return (StatusCode::OK, Json(json!({ "ok": false, "error": e.to_string() }))).into_response();
    }
    let _ = saba_chan_updater_lib::integrity::set_official(&format!("{}{}", saba_chan_updater_lib::relay::MODULE_PREFIX, module_id), false);
    // 캐시 갱신 (write lock)
    let mut supervisor = state.supervisor.write().await;
    let _ = supervisor.refresh_modules();
    supervisor.migrate_list_settings();
    (StatusCode::OK, Json(json!({ "ok": true, "id": module_id }))).into_response()
}

/// GET /api/server/:name/status - 서버 상태 조회
pub async fn get_server_status(
    Path(name): Path<String>,
    State(state): State<IPCServer>,
) -> impl IntoResponse {
    // instance에서 모듈명 조회
    let module_name = state.supervisor.read().await
        .instance_store
        .list()
        .iter()
        .find(|i| i.name == name)
        .map(|i| i.module_name.clone());

    if let Some(module_name) = module_name {
        match crate::supervisor::Supervisor::get_server_status(&state.supervisor, &name, &module_name).await {
            Ok(result) => (StatusCode::OK, Json(result)).into_response(),
            Err(e) => {
                let error = json!({ "error": format!("Failed to get status: {}", e) });
                (StatusCode::INTERNAL_SERVER_ERROR, Json(error)).into_response()
            }
        }
    } else {
        let error = json!({
            "error": format!("Server '{}' not found", name),
            "error_code": "instance_not_found",
        });
        (StatusCode::NOT_FOUND, Json(error)).into_response()
    }
}

/// POST /api/instance/:id/start — 서버 시작. 경로(managed / non-managed)는 데몬이 고른다.
///
/// `:id`는 인스턴스 ID, 없으면 이름으로 찾는다(CLI·채팅처럼 이름만 아는 쪽용).
/// 정책은 `Supervisor::uses_managed_start`, 포트 충돌 검사는 전역 설정 하나다.
/// 응답은 고른 경로의 응답 그대로에 `managed`를 더한 것이다.
pub async fn start_instance_handler(
    Path(id): Path<String>,
    State(state): State<IPCServer>,
) -> axum::response::Response {
    let resp = start_instance(Path(id.clone()), State(state.clone())).await;
    super::audited(&state, "start", &id, None, None, resp).await
}

async fn start_instance(
    Path(id): Path<String>,
    State(state): State<IPCServer>,
) -> axum::response::Response {
    let picked = {
        let supervisor = state.supervisor.read().await;
        supervisor
            .instance_store
            .get(&id)
            .or_else(|| supervisor.instance_store.list().iter().find(|i| i.name == id))
            .cloned()
            .map(|inst| {
                let managed = supervisor.uses_managed_start(&inst);
                (inst, managed)
            })
    };
    let Some((instance, managed)) = picked else {
        return (
            StatusCode::NOT_FOUND,
            Json(json!({ "error": format!("Instance not found: {}", id), "error_code": "not_found" })),
        )
            .into_response();
    };

    // 점검 중에 들어온 정지도 시작을 취소하도록 세대를 점검 전에 잡는다.
    // 재시작처럼 부른 쪽이 이미 확인한 세대가 있으면 그 값을 쓴다 (`start_when_free`)
    let since = match crate::supervisor::START_SINCE.try_with(|g| *g) {
        Ok(checked) => checked,
        Err(_) => state.supervisor.read().await.run_generation_of(&instance.id),
    };
    crate::supervisor::START_SINCE.scope(since, start_checked(state, instance, managed)).await
}

async fn start_checked(state: IPCServer, instance: crate::instance::ServerInstance, managed: bool) -> axum::response::Response {
    if let Some(blocked) = preflight(&state, &instance.id).await {
        return blocked;
    }

    let resp = if managed {
        crate::ipc::internal::into_internal(
            super::managed::start_managed_handler(Path(instance.id.clone()), State(state.clone()), LooseJson(None)).await,
        )
        .await
    } else {
        let req = ServerStartRequest { module: instance.module_name.clone(), config: json!({}) };
        // id로 넘긴다 (이름은 겹칠 수 있다)
        crate::ipc::internal::into_internal(start_server_handler(Path(instance.id.clone()), State(state.clone()), Json(req)).await)
            .await
    };
    let mut body = match resp.body {
        Value::Object(map) => map,
        other => {
            let mut map = serde_json::Map::new();
            map.insert("data".into(), other);
            map
        }
    };
    body.insert("managed".into(), json!(managed));
    (resp.status, Json(announce_updates(&state, &instance.id, Value::Object(body)).await)).into_response()
}

/// 켜기 전 점검: 동의하지 않은 모듈 약관이 있거나, 모듈 validate가 critical 문제(EULA 미동의, 서버 파일 없음 등)를 내면 켜지 않는다.
/// 이미 켜져 있거나, 모듈에 validate가 없거나, 점검 자체가 실패하면 막지 않는다
async fn preflight(state: &IPCServer, id: &str) -> Option<axum::response::Response> {
    {
        let supervisor = state.supervisor.read().await;
        if supervisor.is_running(id).await {
            return None;
        }
    }
    // 모듈 약관(`[[agreements]]`)을 모두 동의해야 켠다. 파이썬을 부르지 않는 검사라 validate보다 먼저
    let language = state.config_store.ui_language().await;
    let missing = state.supervisor.read().await.missing_agreements(id, &language);
    if !missing.is_empty() {
        let names: Vec<&str> = missing.iter().filter_map(|a| a["label"].as_str()).collect();
        let message = crate::chat::i18n::Catalog::for_language(&language).t("errors.agreement_required", &[("names", &names.join(", "))]);
        tracing::info!("Start of '{}' blocked: agreements not accepted", id);
        return Some(
            (
                StatusCode::CONFLICT,
                Json(json!({ "success": false, "error": message, "error_code": "agreement_required", "agreements": missing })),
            )
                .into_response(),
        );
    }
    let result = match crate::supervisor::Supervisor::validate_instance(&state.supervisor, id).await {
        Ok(v) => v,
        Err(e) => {
            tracing::warn!("Pre-start check for '{}' failed to run: {}", id, e);
            return None;
        }
    };
    let issues = result.get("issues").and_then(Value::as_array)?;
    let critical: Vec<&Value> = issues.iter().filter(|i| i.get("severity").and_then(Value::as_str) == Some("critical")).collect();
    let first = critical.first()?;
    let code = first.get("code").and_then(Value::as_str).unwrap_or("preflight_failed");
    let message = first.get("message").and_then(Value::as_str).unwrap_or("Pre-start check failed");
    tracing::info!("Start of '{}' blocked by pre-start check: {}", id, code);
    Some(
        (
            StatusCode::CONFLICT,
            Json(json!({
                "success": false,
                "error": message,
                "error_code": "preflight_failed",
                "issue_code": code,
                "solution": first.get("solution"),
                "issues": critical,
            })),
        )
            .into_response(),
    )
}

/// id(또는 고유한 이름)로 인스턴스를 찾아 (id, 이름)을 돌려준다
async fn instance_key_of(state: &IPCServer, key: &str) -> Option<(String, String)> {
    let supervisor = state.supervisor.read().await;
    supervisor
        .instance_store
        .get(key)
        .or_else(|| supervisor.instance_store.list().iter().find(|i| i.name == key))
        .map(|i| (i.id.clone(), i.name.clone()))
}

fn instance_not_found(id: &str) -> axum::response::Response {
    (
        StatusCode::NOT_FOUND,
        Json(json!({ "error": format!("Instance not found: {}", id), "error_code": "not_found" })),
    )
        .into_response()
}

/// POST /api/instance/:id/stop — id로 정지 (`{force?}`). 응답은 `/api/server/:name/stop`과 같다
pub async fn stop_instance_handler(
    Path(id): Path<String>,
    State(state): State<IPCServer>,
    LooseJson(body): LooseJson<ServerStopRequest>,
) -> axum::response::Response {
    let force = body.map(|Json(b)| b.force).unwrap_or(false);
    stop_instance(state, &id, force).await
}

/// POST /api/instance/:id/restart — 정지(`{force?}`) 뒤 시작. 꺼져 있으면 시작만 한다.
/// 응답은 시작 응답에 `stopped`(정지했는가)를 더한 것. 정지가 실패하면 그 응답을 그대로 준다
pub async fn restart_instance_handler(
    Path(id): Path<String>,
    State(state): State<IPCServer>,
    LooseJson(body): LooseJson<ServerStopRequest>,
) -> axum::response::Response {
    // 안에서 부르는 정지·시작은 따로 남기지 않는다
    let resp = crate::audit::quietly(restart_instance(Path(id.clone()), State(state.clone()), LooseJson(body))).await;
    super::audited(&state, "restart", &id, None, None, resp).await
}

async fn restart_instance(
    Path(id): Path<String>,
    State(state): State<IPCServer>,
    LooseJson(body): LooseJson<ServerStopRequest>,
) -> axum::response::Response {
    let picked = {
        let supervisor = state.supervisor.read().await;
        supervisor
            .instance_store
            .get(&id)
            .or_else(|| supervisor.instance_store.list().iter().find(|i| i.name == id))
            .map(|i| i.id.clone())
    };
    let Some(inst_id) = picked else {
        return instance_not_found(&id);
    };
    let force = body.map(|Json(b)| b.force).unwrap_or(false);
    // 이 재시작의 정지가 올린 세대보다 더 올랐으면 그사이 다른 정지·시작이 들어온 것이다 (D-S8).
    // 기준은 정지가 알려 준 값이다: 정지 전에 따로 읽으면 그 틈에 끼어든 자동 재시작 시작까지 세어 스스로 취소한다 (D-S10)
    let before_stop = state.supervisor.read().await.run_generation_of(&inst_id);
    let running;
    let stopped_at;
    {
        // 데몬이 추적하지 않아도 익스텐션이 맡은 서버일 수 있다: 정지를 먼저 맡긴다
        let stopping = async {
            let resp = stop_instance(state.clone(), &inst_id, force).await;
            (resp, crate::supervisor::STOPPED_AT.with(|at| at.get()))
        };
        let (stop, at) = crate::supervisor::STOPPED_AT.scope(std::cell::Cell::new(None), stopping).await;
        stopped_at = at.unwrap_or(before_stop + 1);
        let stop = crate::ipc::internal::into_internal(stop).await;
        // 꺼져 있던 일반 인스턴스는 정지가 아무것도 하지 않고 was_running=false를 준다
        let ok = stop.status.is_success() && stop.body.get("success").and_then(Value::as_bool) != Some(false);
        if !ok {
            return (stop.status, Json(stop.body)).into_response();
        }
        running = stop.body.get("was_running").and_then(Value::as_bool) != Some(false);
        // 정지 응답이 프로세스가 끝나기 전에 올 수 있다: 끝날 때까지 기다린다 (최대 2분)
        for _ in 0..240 {
            let alive = {
                let supervisor = state.supervisor.read().await;
                supervisor.is_running(&inst_id).await
            };
            if !alive {
                break;
            }
            tokio::time::sleep(std::time::Duration::from_millis(500)).await;
        }
    }
    // 백업·복원이 잠금을 쥐고 있으면 끝나기를 기다렸다 켠다. 요청이 끊겨도 꺼진 채 남지 않게 따로 돌린다.
    // 그사이 다른 쪽에서 켜거나 인스턴스를 지우면 그만둔다
    let generation = state.supervisor.read().await.run_generation_of(&inst_id);
    if generation > stopped_at {
        let error = json!({ "success": false, "error": "The server was stopped or started while restarting", "error_code": "restart_cancelled", "stopped": running });
        return (StatusCode::CONFLICT, Json(error)).into_response();
    }
    let start = crate::audit::spawn(async move {
        let wanted = || async {
            let supervisor = state.supervisor.read().await;
            supervisor.instance_store.get(&inst_id).is_some() && supervisor.run_generation_of(&inst_id) == generation
        };
        super::backup::start_when_free(&state, &inst_id, generation, wanted).await
    })
    .await;
    let start = match start {
        Ok(Some(start)) => start,
        Ok(None) => {
            let error = json!({ "success": false, "error": "The server was started or removed while waiting", "error_code": "restart_cancelled", "stopped": running });
            return (StatusCode::CONFLICT, Json(error)).into_response();
        }
        Err(e) => return (StatusCode::INTERNAL_SERVER_ERROR, Json(json!({ "success": false, "error": e.to_string() }))).into_response(),
    };
    let mut body = match start.body {
        Value::Object(map) => map,
        other => serde_json::Map::from_iter([("data".to_string(), other)]),
    };
    body.insert("stopped".into(), json!(running));
    (start.status, Json(Value::Object(body))).into_response()
}

/// 크래시 뒤 자동 재시작 (A1). 대기하는 동안 사용자가 정지·시작했으면 그만둔다.
pub async fn auto_restart(state: IPCServer, instance_id: String, delay: std::time::Duration, generation: u64) {
    tokio::time::sleep(delay).await;
    if !state.supervisor.read().await.restart_still_wanted(&instance_id, generation).await {
        tracing::info!("Auto restart of '{}' skipped: stopped or started meanwhile", instance_id);
        state.supervisor.read().await.record_restart_outcome(&instance_id, generation, false);
        return;
    }
    tracing::info!("Auto restarting '{}' after a crash", instance_id);
    // 백업이 잠금을 쥐고 있으면 끝나기를 기다린다 (기다리는 동안 사용자가 정지·시작하면 그만둔다)
    let wanted = || async { state.supervisor.read().await.restart_still_wanted(&instance_id, generation).await };
    // 다시 띄운 시작의 출처는 자동 재시작이다 (`instance.started`·감사 로그)
    let restart = crate::audit::Origin::new("restart");
    let Some(resp) = crate::audit::with_origin(restart, super::backup::start_when_free(&state, &instance_id, generation, wanted)).await else {
        tracing::info!("Auto restart of '{}' skipped: stopped or started meanwhile", instance_id);
        state.supervisor.read().await.record_restart_outcome(&instance_id, generation, false);
        return;
    };
    let ok = resp.status.is_success() && resp.body.get("success").and_then(Value::as_bool) != Some(false);
    if !ok {
        let reason = resp.body.get("error").or_else(|| resp.body.get("message")).and_then(Value::as_str).unwrap_or("start failed").to_string();
        tracing::warn!("Auto restart of '{}' failed: {}", instance_id, reason);
        state.supervisor.write().await.record_restart_failure(&instance_id, generation, reason);
    } else {
        state.supervisor.read().await.record_restart_outcome(&instance_id, generation, true);
    }
}

/// GET /api/instance/:id/events — 최근 종료 이력 (사용자 정지, 게임 안 정지, 크래시, 원인 모를 종료)
pub async fn instance_events_handler(Path(id): Path<String>, State(state): State<IPCServer>) -> impl IntoResponse {
    let supervisor = state.supervisor.read().await;
    if supervisor.instance_store.get(&id).is_none() {
        return (StatusCode::NOT_FOUND, Json(json!({ "error": format!("Instance not found: {}", id), "error_code": "not_found" }))).into_response();
    }
    Json(json!({ "events": supervisor.exit_events(&id) })).into_response()
}

/// POST /api/server/:name/start - 서버 시작
pub async fn start_server_handler(
    Path(name): Path<String>,
    State(state): State<IPCServer>,
    Json(payload): Json<ServerStartRequest>,
) -> impl IntoResponse {
    // API 경유 시작 기록 (GUI 외부 시작 감지용)
    state.api_actions.record(&name);
    // 익스텐션 상태 캐시 무효화 — 시작 직후 fresh 상태 반영
    let inst_id = {
        let sup = state.supervisor.read().await;
        let id = sup.instance_store.get(&name).or_else(|| sup.instance_store.list().iter().find(|i| i.name == name)).map(|i| i.id.clone());
        if let Some(id) = &id {
            state.extension_status_cache.invalidate(id);
        }
        id
    };
    // 백업·복원·업데이트가 서버 파일을 다루는 동안에는 켜지 않는다 (시작을 마칠 때까지 잡는다)
    let _guard = match inst_id.as_deref() {
        Some(id) => match super::backup::InstanceLock::take_for_writes(id).await {
            None => return super::audited(&state, "start", &name, None, None, super::backup::instance_busy()).await,
            guard => guard,
        },
        None => None,
    };

    fill_passwords(&state, &name).await;

    let resp = match crate::supervisor::Supervisor::start_server(&state.supervisor, &name, &payload.module, payload.config)
        .await
    {
        Ok(result) => (StatusCode::OK, Json(result)).into_response(),
        Err(e) => {
            let msg = e.to_string();
            let (code, error_code) = if msg.contains("not found") {
                (StatusCode::NOT_FOUND, "instance_not_found")
            } else if msg.contains("jar") || msg.contains("executable") {
                (StatusCode::UNPROCESSABLE_ENTITY, "executable_missing")
            } else {
                (StatusCode::INTERNAL_SERVER_ERROR, "start_failed")
            };
            let error = if e.downcast_ref::<crate::plugin::ModuleCallError>().is_some() {
                crate::plugin::error_response(&e)
            } else { json!({
                "error": format!("Failed to start server: {}", msg),
                "error_code": error_code,
            }) };
            (code, Json(error)).into_response()
        }
    };
    super::audited(&state, "start", &name, None, None, resp).await
}

/// 받아 둔 업데이트 확인 결과에서 이 모듈의 새 버전 `{module, current, latest}`
fn module_update_of(status: &saba_chan_updater_lib::UpdateStatus, module: &str) -> Option<Value> {
    use saba_chan_updater_lib::Component;
    status
        .components
        .iter()
        .find(|c| c.update_available && matches!(&c.component, Component::Module(m) if m == module))
        .map(|c| json!({ "module": module, "current": c.current_version, "latest": c.latest_version }))
}

/// 새로 켠 서버의 업데이트 알림 `instance.update_notice {instanceId, instance, kind, module?, latest?}`.
/// - `module`: 모듈에 새 버전이 있다 (받아 둔 확인 결과라 바로 낸다). 시작 답에도 `module_update`로 싣는다
/// - `server`: 게임 서버에 새 빌드가 있다 (`server.check_update` hook, 오래 걸릴 수 있어 뒤에서)
async fn announce_updates(state: &IPCServer, id: &str, mut result: Value) -> Value {
    let fresh = result.get("success").and_then(Value::as_bool) == Some(true)
        && result.get("already_running").and_then(Value::as_bool) != Some(true);
    if !fresh {
        return result;
    }
    let Some((name, module)) = state.supervisor.read().await.instance_store.get(id).map(|i| (i.name.clone(), i.module_name.clone())) else {
        return result;
    };
    // 받거나 적용하는 중이면 매니저가 잠겨 있다: 덧붙이는 안내 때문에 시작 답을 기다리게 하지 않는다
    let status = state.update_state.manager.try_read().ok().map(|m| m.get_status());
    if let Some(update) = status.as_ref().and_then(|s| module_update_of(s, &module)) {
        crate::events::emit(
            "instance.update_notice",
            json!({ "instanceId": id, "instance": name, "kind": "module", "module": module, "latest": update["latest"] }),
        );
        result["module_update"] = update;
    }
    let (state, id) = (state.clone(), id.to_string());
    tokio::spawn(async move {
        let found = super::instance::server_update_status(&state, &id).await;
        if found.and_then(|s| s.get("update_available").and_then(Value::as_bool)) == Some(true) {
            crate::events::emit("instance.update_notice", json!({ "instanceId": id, "instance": name, "kind": "server" }));
        }
    });
    result
}

/// POST /api/server/:name/stop - 서버 중지
pub async fn stop_server_handler(
    Path(name): Path<String>,
    State(state): State<IPCServer>,
    Json(payload): Json<ServerStopRequest>,
) -> axum::response::Response {
    let id = {
        let supervisor = state.supervisor.read().await;
        supervisor.instance_store.list().iter().find(|i| i.name == name).map(|i| i.id.clone())
    };
    match id {
        Some(id) => stop_instance(state, &id, payload.force).await,
        None => (
            StatusCode::NOT_FOUND,
            Json(json!({ "error": format!("Server '{}' not found", name), "error_code": "instance_not_found" })),
        )
            .into_response(),
    }
}

/// 인스턴스 하나를 id로 정지한다. 이름은 겹칠 수 있으니 정지 경로는 모두 id를 쓴다
pub async fn stop_instance(state: IPCServer, key: &str, force: bool) -> axum::response::Response {
    let resp = stop_instance_unrecorded(state.clone(), key, force).await;
    super::audited(&state, "stop", key, None, None, resp).await
}

async fn stop_instance_unrecorded(state: IPCServer, key: &str, force: bool) -> axum::response::Response {
    let payload = ServerStopRequest { force };
    let Some((id, name)) = instance_key_of(&state, key).await else {
        return instance_not_found(key);
    };
    let id = id.as_str();
    // API 경유 정지 기록 (GUI 외부 정지 감지용)
    state.api_actions.record(&name);
    // 익스텐션 상태 캐시 무효화 — 정지 직후 fresh 상태 반영
    {
        let sup = state.supervisor.read().await;
        if sup.instance_store.get(id).is_some() {
            state.extension_status_cache.invalidate(id);
        }
    }

    // instance에서 모듈명과 ID 조회 (read lock으로 조회 후 즉시 해제)
    let instance = {
        let supervisor = state.supervisor.read().await;
        supervisor.instance_store.get(id).cloned()
    };

    if let Some(inst) = instance {
        // instance id로도 API 액션 기록
        state.api_actions.record(&inst.id);
        // 락은 stop_server가 단계마다 짧게 잡는다 (C12)
        match crate::supervisor::Supervisor::stop_server(&state.supervisor, &inst.id, &inst.module_name, payload.force).await {
            Ok(result) => {
                // 실제 종료 성공 시에만 tracker에서 제거
                let success = result
                    .get("success")
                    .and_then(|s| s.as_bool())
                    .unwrap_or(false);
                if success {
                    // tracker는 id로만 찾는다: 이름이 다른 인스턴스의 id와 같아도 그쪽을 건드리지 않는다
                    let supervisor = state.supervisor.read().await;
                    if let Err(e) = supervisor.tracker.untrack(&inst.id) {
                        tracing::warn!("Failed to untrack stopped server '{}': {}", name, e);
                    } else {
                        tracing::info!("Server '{}' untracked from process tracker", name);
                    }
                }
                (StatusCode::OK, Json(result)).into_response()
            }
            Err(e) => {
                let msg = e.to_string();
                let (code, error_code) = if msg.contains("not found") {
                    (StatusCode::NOT_FOUND, "instance_not_found")
                } else {
                    (StatusCode::INTERNAL_SERVER_ERROR, "stop_failed")
                };
                let error = if e.downcast_ref::<crate::plugin::ModuleCallError>().is_some() {
                    crate::plugin::error_response(&e)
                } else { json!({
                    "error": format!("Failed to stop server: {}", msg),
                    "error_code": error_code,
                }) };
                (code, Json(error)).into_response()
            }
        }
    } else {
        let error = json!({
            "error": format!("Server '{}' not found", name),
            "error_code": "instance_not_found",
        });
        (StatusCode::NOT_FOUND, Json(error)).into_response()
    }
}

// NOTE: sync_gui_config は config::save_gui_config に統合されました (portConflictCheck 同期含む)
/// 실행 전 비밀번호: 빈 REST 비밀번호는 GUI "비밀번호 자동 생성"(`autoGeneratePasswords`, 기본 켬)이 켜져 있을 때만 채운다.
/// 꺼져 있으면 비워 두고, 모듈이 게임 설정의 값을 쓰거나 스스로 만든다(`credential_map`으로 데몬에 돌아온다).
/// REST를 지원하지 않는 모듈(`[protocols] supported`에 rest가 없음)의 서버에는 REST 계정을 만들지 않고, 남아 있던 것은 지운다. `key`: 인스턴스 id 또는 이름
pub(super) async fn fill_passwords(state: &IPCServer, key: &str) {
    let generate = state.config_store.get_gui_settings().await.auto_generate_passwords;
    let mut supervisor = state.supervisor.write().await;
    let found = supervisor.instance_store.get(key).or_else(|| supervisor.instance_store.list().iter().find(|i| i.name == key)).cloned();
    let Some(mut inst) = found else { return };
    let changed = match supervisor.module_loader.get_module(&inst.module_name).map(|m| m.metadata.supports_protocol("rest")) {
        Ok(true) => inst.ensure_passwords(generate),
        // 예전 데몬이 만들어 둔 계정도 치운다 (REST 없는 모듈은 이 값을 쓰지 않는다)
        Ok(false) if inst.rest_username.is_some() || inst.rest_password.is_some() => {
            (inst.rest_username, inst.rest_password) = (None, None);
            true
        }
        _ => false,
    };
    if changed {
        let id = inst.id.clone();
        if let Err(e) = supervisor.instance_store.update(&id, inst) {
            tracing::warn!("Failed to save auto-generated passwords for {}: {}", key, e);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Arc;
    use tokio::sync::RwLock;

    /// L46: 데몬이 자기를 띄운 프로그램과 함께 끝나는 job 안에 있는지 /health에 싣는다 (GUI·CLI가 알린다)
    #[tokio::test]
    async fn health_says_whether_the_daemon_stops_with_its_launcher() {
        let done = crate::ipc::internal::into_internal(health_check(axum::http::HeaderMap::new()).await.into_response()).await;
        assert!(done.body["stops_with_launcher"].is_boolean(), "{}", done.body);
        #[cfg(not(windows))]
        assert_eq!(done.body["stops_with_launcher"], false);
        // job 밖이면 false, kill-on-close job 안이면 breakaway를 허용하든 말든 true (BREAKAWAY_OK 0x800, SILENT_BREAKAWAY_OK 0x1000)
        assert!(!job_stops_with_it(false, KILL_ON_JOB_CLOSE));
        assert!(!job_stops_with_it(true, 0x800 | 0x1000));
        assert!(job_stops_with_it(true, KILL_ON_JOB_CLOSE));
        assert!(job_stops_with_it(true, KILL_ON_JOB_CLOSE | 0x800));
        assert!(job_stops_with_it(true, KILL_ON_JOB_CLOSE | 0x1000));
    }

    /// 켜려는 서버의 모듈에 새 버전이 있으면 시작 답과 알림에 싣는다 (받아 둔 업데이트 확인 결과, 다른 모듈·없는 새 버전은 아니다)
    #[test]
    fn a_newer_module_version_is_found_from_the_last_update_check() {
        let component = |module: &str, available: bool| {
            json!({ "component": { "module": module }, "current_version": "1.0.0", "latest_version": "1.1.0", "update_available": available,
                    "download_url": null, "asset_name": null, "release_notes": null, "published_at": null, "downloaded": false, "downloaded_path": null, "installed": true })
        };
        let status: saba_chan_updater_lib::UpdateStatus = serde_json::from_value(json!({
            "last_check": null, "next_check": null, "checking": false, "error": null,
            "components": [component("palworld", true), component("minecraft", false)],
        }))
        .unwrap();
        assert_eq!(module_update_of(&status, "palworld"), Some(json!({ "module": "palworld", "current": "1.0.0", "latest": "1.1.0" })));
        assert_eq!(module_update_of(&status, "minecraft"), None);
        assert_eq!(module_update_of(&status, "ark"), None);
    }

    /// GUI "비밀번호 자동 생성"을 끄면 시작할 때 빈 REST 비밀번호를 채우지 않는다. 켜져 있으면(기본) 채운다.
    /// REST를 지원하지 않는 모듈의 서버에는 REST 계정을 만들지 않는다 (테라리아 점검: 설정 화면에 쓸모없는 값이 보였다)
    #[tokio::test]
    async fn rest_passwords_are_generated_only_when_the_setting_is_on() {
        let tmp = tempfile::tempdir().unwrap();
        let modules = tmp.path().join("modules");
        for (name, supported) in [("game", "rest"), ("console", "stdin")] {
            let dir = modules.join(name);
            std::fs::create_dir_all(&dir).unwrap();
            std::fs::write(dir.join("module.toml"), format!("[module]\nschema_version = 1\nname = \"{name}\"\nversion = \"1.0.0\"\nentry = \"lifecycle.py\"\n\n[protocols]\nsupported = [\"{supported}\"]\n")).unwrap();
            std::fs::write(dir.join("lifecycle.py"), "").unwrap();
        }
        let mut sup = crate::supervisor::Supervisor::new_with_instances_dir(modules.to_str().unwrap(), tmp.path().join("instances").to_str().unwrap());
        sup.initialize().await.unwrap();
        let mut state = IPCServer::new(Arc::new(RwLock::new(sup)), "127.0.0.1:0", crate::daemon_log::DaemonLogBuffer::new());
        state.config_store = Arc::new(crate::config_store::ConfigStore::new(&tmp.path().join("data")));
        let inst = crate::instance::ServerInstance::new("srv", "game");
        let id = inst.id.clone();
        let mut plain = crate::instance::ServerInstance::new("terra", "console");
        // 0.1.10이 만들어 둔 값
        (plain.rest_username, plain.rest_password) = (Some("admin".into()), Some("left-over".into()));
        let plain_id = plain.id.clone();
        state.supervisor.write().await.instance_store.add(inst).unwrap();
        state.supervisor.write().await.instance_store.add(plain).unwrap();
        let rest = |id: String| {
            let state = state.clone();
            async move {
                let sup = state.supervisor.read().await;
                let i = sup.instance_store.get(&id).unwrap();
                (i.rest_username.clone(), i.rest_password.clone())
            }
        };

        state.config_store.set_gui_settings_from_json(json!({ "autoGeneratePasswords": false })).await.unwrap();
        fill_passwords(&state, &id).await;
        assert_eq!(rest(id.clone()).await.1, None, "the setting is off");
        state.config_store.set_gui_settings_from_json(json!({ "autoGeneratePasswords": true })).await.unwrap();
        fill_passwords(&state, "srv").await;
        assert_eq!(rest(id.clone()).await.1.map(|p| p.len()), Some(16), "found by name too");
        fill_passwords(&state, &plain_id).await;
        assert_eq!(rest(plain_id).await, (None, None), "a module without REST keeps no REST account");
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

    /// 재시작의 정지 단계(서버가 끝나기를 기다리는 동안)에 들어온 정지도 재시작을 취소한다 (D-S8)
    #[cfg(unix)]
    #[tokio::test]
    async fn a_stop_during_a_restart_keeps_the_server_down() {
        let tmp = tempfile::tempdir().unwrap();
        let modules = tmp.path().join("modules");
        let dir = modules.join("game");
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(
            dir.join("module.toml"),
            "[module]\nschema_version = 1\nname = \"game\"\nversion = \"1.0.0\"\nentry = \"lifecycle.py\"\n\n[config]\nstop_timeout = 3\n",
        )
        .unwrap();
        // 정상 종료 요청(TERM)을 무시해 정지가 stop_timeout까지 걸리는 서버
        std::fs::write(
            dir.join("lifecycle.py"),
            "import json, sys\n\
             if sys.argv[1] == 'get_launch_command':\n    \
             print(json.dumps({'success': True, 'program': '/bin/sh', 'args': ['-c', \"trap '' TERM; sleep 60\"], 'working_dir': '.'}))\n\
             else:\n    print(json.dumps({'success': True}))\n",
        )
        .unwrap();
        let mut sup = crate::supervisor::Supervisor::new_with_instances_dir(modules.to_str().unwrap(), tmp.path().join("instances").to_str().unwrap());
        sup.initialize().await.unwrap();
        let mut inst = crate::instance::ServerInstance::new("srv", "game");
        inst.id = "stop-during-restart".into();
        sup.instance_store.add(inst).unwrap();
        let state = IPCServer::new(Arc::new(RwLock::new(sup)), "127.0.0.1:0", crate::daemon_log::DaemonLogBuffer::new());
        let started = crate::ipc::internal::into_internal(start_instance(Path("stop-during-restart".into()), State(state.clone())).await).await.body;
        assert_ne!(started["success"], false, "{started}");

        let restarter = {
            let state = state.clone();
            tokio::spawn(async move {
                crate::ipc::internal::into_internal(restart_instance(Path("stop-during-restart".into()), State(state), super::super::LooseJson(None)).await).await.body
            })
        };
        tokio::time::sleep(std::time::Duration::from_millis(1000)).await;
        let _ = crate::supervisor::Supervisor::stop_server(&state.supervisor, "stop-during-restart", "game", false).await;

        let restarted = restarter.await.unwrap();
        assert_eq!(restarted["error_code"], "restart_cancelled", "{restarted}");
        let s = state.supervisor.read().await;
        assert!(s.tracker.get_pid("stop-during-restart").is_err(), "nothing is tracked");
        assert!(s.managed_store.get("stop-during-restart").await.is_none(), "the restart did not bring the server back");
    }

    /// 시작 전 점검(모듈 validate) 중에 들어온 정지도 시작을 취소한다: 그 정지는 PID가 없어 바로 성공으로 끝난다
    #[cfg(unix)]
    #[tokio::test]
    async fn a_stop_during_the_pre_start_check_keeps_the_server_down() {
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
            "import json, sys, time\n\
             if sys.argv[1] == 'validate':\n    time.sleep(2)\n    print(json.dumps({'success': True, 'issues': []}))\n\
             elif sys.argv[1] == 'get_launch_command':\n    \
             print(json.dumps({'success': True, 'program': '/bin/sh', 'args': ['-c', 'sleep 30'], 'working_dir': '.'}))\n\
             else:\n    print(json.dumps({'success': True}))\n",
        )
        .unwrap();
        let mut sup = crate::supervisor::Supervisor::new_with_instances_dir(modules.to_str().unwrap(), tmp.path().join("instances").to_str().unwrap());
        sup.initialize().await.unwrap();
        let mut inst = crate::instance::ServerInstance::new("srv", "game");
        inst.id = "stop-during-check".into();
        sup.instance_store.add(inst).unwrap();
        let state = IPCServer::new(Arc::new(RwLock::new(sup)), "127.0.0.1:0", crate::daemon_log::DaemonLogBuffer::new());

        let starter = {
            let state = state.clone();
            tokio::spawn(async move {
                crate::ipc::internal::into_internal(start_instance(Path("stop-during-check".into()), State(state)).await).await.body
            })
        };
        tokio::time::sleep(std::time::Duration::from_millis(700)).await;
        let stopped = crate::supervisor::Supervisor::stop_server(&state.supervisor, "stop-during-check", "game", false).await.unwrap();
        assert_ne!(stopped["success"], false, "{stopped}");

        let started = starter.await.unwrap();
        assert_eq!(started["error_code"], "start_cancelled", "{started}");
        let s = state.supervisor.read().await;
        assert!(s.tracker.get_pid("stop-during-check").is_err(), "nothing is tracked");
        assert!(s.managed_store.get("stop-during-check").await.is_none(), "no server was left running");
    }

    /// 재시작·자동 재시작은 세대를 확인한 뒤 시작한다. 확인과 시작 사이에 들어온 정지도 시작을 취소해야 한다:
    /// 시작은 확인한 세대를 기준으로 삼는다(새로 읽으면 그 정지가 기준에 들어가 버린다)
    #[cfg(unix)]
    #[tokio::test]
    async fn a_stop_between_the_restart_check_and_its_start_keeps_the_server_down() {
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
        let mut sup = crate::supervisor::Supervisor::new_with_instances_dir(modules.to_str().unwrap(), tmp.path().join("instances").to_str().unwrap());
        sup.initialize().await.unwrap();
        let mut inst = crate::instance::ServerInstance::new("srv", "game");
        inst.id = "stop-after-check".into();
        sup.instance_store.add(inst).unwrap();
        let state = IPCServer::new(Arc::new(RwLock::new(sup)), "127.0.0.1:0", crate::daemon_log::DaemonLogBuffer::new());

        let checked = state.supervisor.read().await.run_generation_of("stop-after-check");
        let stopped = crate::supervisor::Supervisor::stop_server(&state.supervisor, "stop-after-check", "game", false).await.unwrap();
        assert_ne!(stopped["success"], false, "{stopped}");
        let started = super::super::backup::start_when_free(&state, "stop-after-check", checked, || async { true }).await.unwrap().body;

        assert_eq!(started["error_code"], "start_cancelled", "{started}");
        let s = state.supervisor.read().await;
        assert!(s.tracker.get_pid("stop-after-check").is_err(), "nothing is tracked");
        assert!(s.managed_store.get("stop-after-check").await.is_none(), "no server was left running");
    }

    /// 정지 대기 시간은 인스턴스 설정 `stop_timeout`(초)이 모듈 값보다 앞선다: 큰 월드는 늘리고, 여기서는 줄여 바로 끝나는지 본다 (MK14)
    #[cfg(unix)]
    #[tokio::test]
    async fn an_instance_stop_timeout_overrides_the_module_one() {
        let tmp = tempfile::tempdir().unwrap();
        let modules = tmp.path().join("modules");
        let dir = modules.join("game");
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(
            dir.join("module.toml"),
            "[module]\nschema_version = 1\nname = \"game\"\nversion = \"1.0.0\"\nentry = \"lifecycle.py\"\n[config]\nstop_timeout = 60\n",
        )
        .unwrap();
        // stop()은 아무것도 하지 않는다: 서버는 대기 시간이 지나 강제 종료돼야 끝난다
        std::fs::write(
            dir.join("lifecycle.py"),
            "import json, sys\n\
             if sys.argv[1] == 'get_launch_command':\n    \
             print(json.dumps({'success': True, 'program': '/bin/sh', 'args': ['-c', 'sleep 60'], 'working_dir': '.'}))\n\
             else:\n    print(json.dumps({'success': True, 'issues': []}))\n",
        )
        .unwrap();
        let mut sup = crate::supervisor::Supervisor::new_with_instances_dir(modules.to_str().unwrap(), tmp.path().join("instances").to_str().unwrap());
        sup.initialize().await.unwrap();
        let mut inst = crate::instance::ServerInstance::new("srv", "game");
        inst.id = "short-stop".into();
        inst.module_settings.insert("stop_timeout".into(), json!(1));
        sup.instance_store.add(inst).unwrap();
        let state = IPCServer::new(Arc::new(RwLock::new(sup)), "127.0.0.1:0", crate::daemon_log::DaemonLogBuffer::new());
        let started = crate::ipc::internal::into_internal(start_instance(Path("short-stop".into()), State(state.clone())).await).await.body;
        assert_eq!(started["success"], true, "{started}");

        let begun = std::time::Instant::now();
        let stopped = crate::ipc::internal::into_internal(stop_instance(state.clone(), "short-stop", false).await).await.body;
        assert_eq!(stopped["success"], true, "{stopped}");
        assert!(begun.elapsed() < std::time::Duration::from_secs(15), "waited {:?}, not the instance's 1s", begun.elapsed());
    }

    /// 콘솔이 없어 저장 후 종료를 요청할 길이 없으면 모듈 `stop()`은 `graceful_stop_unavailable`로 답한다. 데몬은 그때
    /// 서버를 죽이지 않고 그 코드를 돌려준다: 강제 종료(저장 없음)는 사용자가 `force`로 고른다 (MK10)
    #[cfg(unix)]
    #[tokio::test]
    async fn a_server_that_cannot_save_and_exit_is_not_killed_by_a_plain_stop() {
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
            "import json, os, signal, sys\n\
             config = json.load(sys.stdin)\n\
             if sys.argv[1] == 'get_launch_command':\n    \
             print(json.dumps({'success': True, 'program': '/bin/sh', 'args': ['-c', 'sleep 30'], 'working_dir': '.'}))\n\
             elif sys.argv[1] == 'stop' and not config.get('force'):\n    \
             print(json.dumps({'success': False, 'error_code': 'graceful_stop_unavailable', 'message': 'no console'}))\n\
             elif sys.argv[1] == 'stop':\n    os.kill(config['pid'], signal.SIGKILL)\n    print(json.dumps({'success': True}))\n\
             else:\n    print(json.dumps({'success': True, 'issues': []}))\n",
        )
        .unwrap();
        let mut sup = crate::supervisor::Supervisor::new_with_instances_dir(modules.to_str().unwrap(), tmp.path().join("instances").to_str().unwrap());
        sup.initialize().await.unwrap();
        for id in ["no-console-managed", "no-console-native"] {
            let mut inst = crate::instance::ServerInstance::new(id, "game");
            inst.id = id.into();
            sup.instance_store.add(inst).unwrap();
        }
        let state = IPCServer::new(Arc::new(RwLock::new(sup)), "127.0.0.1:0", crate::daemon_log::DaemonLogBuffer::new());
        let stop = |id: &'static str, force: bool| {
            let state = state.clone();
            async move { crate::ipc::internal::into_internal(stop_instance(state, id, force).await).await.body }
        };

        // managed인데 모듈이 stdin을 받지 않는다: 데몬은 모듈 stop()에 맡긴다
        let started = crate::ipc::internal::into_internal(start_instance(Path("no-console-managed".into()), State(state.clone())).await).await.body;
        assert_eq!(started["success"], true, "{started}");
        let refused = stop("no-console-managed", false).await;
        assert_eq!((refused["success"].clone(), refused["error_code"].clone()), (json!(false), json!("graceful_stop_unavailable")), "{refused}");
        let managed = state.supervisor.read().await.managed_store.get("no-console-managed").await.expect("still managed");
        assert!(managed.is_running(), "a plain stop does not kill it");
        assert_eq!(stop("no-console-managed", true).await["success"], true);

        // non-managed(데몬이 PID만 안다): 모듈 stop()의 답을 그대로 전한다
        let mut child = std::process::Command::new("sleep").arg("30").spawn().unwrap();
        state.supervisor.read().await.tracker.track("no-console-native", child.id()).unwrap();
        let refused = stop("no-console-native", false).await;
        assert_eq!(refused["error_code"], "graceful_stop_unavailable", "{refused}");
        assert!(child.try_wait().unwrap().is_none(), "a plain stop does not kill it");
        assert_eq!(stop("no-console-native", true).await["success"], true);
        assert!(child.wait().is_ok());
    }

    /// non-managed 정지에서 모듈 `stop()`은 이번 정지에 쓸 대기 시간(`stop_timeout`)과 그 호출의 한도(`stop_call_limit`,
    /// `stop_timeout + 30`초)를 config로 받는다. 인스턴스 값이 없으면 모듈 값이다 (06 계약 6절)
    #[cfg(unix)]
    #[tokio::test]
    async fn a_module_stop_is_told_how_long_to_wait() {
        let tmp = tempfile::tempdir().unwrap();
        let modules = tmp.path().join("modules");
        let dir = modules.join("game");
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(
            dir.join("module.toml"),
            "[module]\nschema_version = 1\nname = \"game\"\nversion = \"1.0.0\"\nentry = \"lifecycle.py\"\n\n[config]\nstop_timeout = 200\n",
        )
        .unwrap();
        let seen = tmp.path().join("seen.txt");
        std::fs::write(
            dir.join("lifecycle.py"),
            format!(
                "import json, os, signal, sys\n\
                 config = json.load(sys.stdin)\n\
                 if sys.argv[1] == 'stop':\n    \
                 open({seen:?}, 'w').write('%s %s' % (config.get('stop_timeout'), config.get('stop_call_limit')))\n    \
                 os.kill(config['pid'], signal.SIGKILL)\n    print(json.dumps({{'success': True}}))\n\
                 else:\n    print(json.dumps({{'success': True}}))\n",
                seen = seen.to_string_lossy()
            ),
        )
        .unwrap();
        let mut sup = crate::supervisor::Supervisor::new_with_instances_dir(modules.to_str().unwrap(), tmp.path().join("instances").to_str().unwrap());
        sup.initialize().await.unwrap();
        let mut inst = crate::instance::ServerInstance::new("native", "game");
        inst.id = "native".into();
        sup.instance_store.add(inst).unwrap();
        let state = IPCServer::new(Arc::new(RwLock::new(sup)), "127.0.0.1:0", crate::daemon_log::DaemonLogBuffer::new());

        let mut child = std::process::Command::new("sleep").arg("30").spawn().unwrap();
        state.supervisor.read().await.tracker.track("native", child.id()).unwrap();
        let stopped = crate::ipc::internal::into_internal(stop_instance(state.clone(), "native", false).await).await.body;
        assert_eq!(stopped["success"], true, "{stopped}");
        assert!(child.wait().is_ok());
        assert_eq!(std::fs::read_to_string(&seen).unwrap(), "200 230");
    }

    #[test]
    fn only_unofficial_or_changed_components_get_a_notice() {
        use saba_chan_updater_lib::integrity::Trust;
        assert!(trust_warnings("module", "m", Trust::Verified).is_empty());
        assert_eq!(trust_warnings("module", "m", Trust::Unverified), [json!({"code": "module_unverified", "module": "m", "isolated": cfg!(windows)})]);
        assert_eq!(trust_warnings("extension", "e", Trust::Modified), [json!({"code": "extension_modified", "extension": "e", "isolated": cfg!(windows)})]);
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
        let (status, refused) = body(start_instance(Path(id.clone()), State(state.clone())).await).await;
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

    /// 포트 충돌 강제 정지 알림은 클라이언트가 읽을 몫이다: 데몬 안 호출(릴레이 하트비트·채팅)은 비우지 않는다 (D-S3)
    #[tokio::test]
    async fn internal_server_lists_leave_forced_stop_notices_for_the_clients() {
        let tmp = tempfile::tempdir().unwrap();
        let mut sup = crate::supervisor::Supervisor::new_with_instances_dir(
            tmp.path().join("modules").to_str().unwrap(),
            tmp.path().join("instances").to_str().unwrap(),
        );
        sup.initialize().await.unwrap();
        sup.record_port_conflict_stops([crate::supervisor::PortConflictStopEvent {
            id: 0,
            stopped_name: "b".into(),
            stopped_id: "b-id".into(),
            existing_name: "a".into(),
            existing_id: "a-id".into(),
            port: 25565,
            timestamp_ms: std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap().as_millis() as u64,
        }]);
        let state = IPCServer::new(Arc::new(RwLock::new(sup)), "127.0.0.1:0", crate::daemon_log::DaemonLogBuffer::new());
        let stops = |body: Value| body["port_conflict_stops"].as_array().map(Vec::len).unwrap_or(0);

        let internal = crate::ipc::internal::into_internal(list_servers_internal(&state).await).await.body;
        assert_eq!(stops(internal), 1, "internal callers still see it");
        // 대기 루프·목록 같은 클라이언트 조회는 `notices=keep`으로 비우지 않고 본다
        let keep = Query(ListServersQuery { notices: Some("keep".into()), ..Default::default() });
        let peek = crate::ipc::internal::into_internal(list_servers(State(state.clone()), keep).await).await.body;
        assert_eq!(stops(peek), 1, "notices=keep shows it without taking it");
        let client = crate::ipc::internal::into_internal(list_servers(State(state.clone()), Query(ListServersQuery::default())).await).await.body;
        assert_eq!(stops(client), 1, "the client gets the notice the heartbeat did not take");
        let again = crate::ipc::internal::into_internal(list_servers(State(state), Query(ListServersQuery::default())).await).await.body;
        assert_eq!(stops(again), 0, "a client reads it once");
    }

    /// 강제 정지 알림은 한 클라이언트가 읽어도 사라지지 않는다: 알림마다 id가 있고 `notices_after`로 본 것 뒤만 받는다.
    /// 옛 클라이언트(매개변수 없음)가 먼저 읽어도 새 클라이언트는 그대로 본다. 오래된 알림은 지운다 (L50)
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

        let legacy = list(ListServersQuery::default()).await;
        assert_eq!(legacy.len(), 1, "an old notice is gone: {legacy:?}");
        let seen = list(after(0)).await;
        assert_eq!(seen.len(), 1, "a client that read nothing still sees it after the legacy read");
        let id = seen[0]["id"].as_u64().expect("notices carry an id");
        assert!(id > 0);
        assert!(list(after(id)).await.is_empty(), "nothing after what the client has seen");
        assert!(list(ListServersQuery::default()).await.is_empty(), "the legacy read still shows a notice once");

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
        let kept = sup.drain_port_conflict_stops();
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
        assert_eq!((body["ok"].clone(), body["module"].clone()), (json!(false), json!("future")));
        assert_eq!(body["error_code"], "module_dependency_unsatisfied", "{body}");
        assert_eq!(body["missing_dependencies"], json!(["saba-core >=99.0.0 (installed 0.1.9)"]), "{body}");
        let missing = saba_chan_updater_lib::unmet_requires("module-future", &requires, &Default::default());
        assert_eq!(unmet_requires_body("future", &missing)["missing_dependencies"], json!(["saba-core >=99.0.0 (not installed)"]));
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
        let held = module_install_guard("plain").expect("free");
        let body = crate::ipc::internal::into_internal(
            install_module_from_manifest(Path("plain".into()), State(state.clone())).await.into_response(),
        )
        .await
        .body;
        assert_eq!(body["error_code"], "module_busy", "{body}");
        let removed = crate::ipc::internal::into_internal(remove_module(Path("plain".into()), State(state)).await.into_response()).await.body;
        assert_eq!(removed["error_code"], "module_busy", "{removed}");
        drop(held);
        assert!(module_install_guard("plain").is_some(), "refused requests must not keep the lock");

        let (zip_a, staging_a) = module_install_paths(tmp.path(), "plain");
        let (zip_b, staging_b) = module_install_paths(tmp.path(), "plain");
        assert!(zip_a != zip_b && staging_a != staging_b);
        let name = staging_a.file_name().unwrap().to_string_lossy().to_string();
        assert!(name.starts_with(".staging-plain-") && zip_a.file_name().unwrap().to_string_lossy().starts_with(".tmp-module-plain-"));
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
}
