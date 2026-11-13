//! 서버 목록과 접속자: 화면·채팅·릴레이 하트비트가 같이 읽는 서버 상태

use axum::{http::StatusCode, response::IntoResponse, Json};
use serde_json::{json, Value};

use crate::ipc::{IPCServer, PortConflictInfo, ServerInfo, ServerListResponse};

/// 서버마다의 `server.list_enrich` 답 (서버 id → 답). TTL 캐시에 답이 있으면 Python을 띄우지 않는다.
/// 캐시에 없는 서버의 hook은 함께 돌린다: 차례로 돌리면 캐시가 끝날 때마다 목록이 (서버 수 × Python 기동 시간)만큼 늦는다.
/// 한 번에 `LIST_ENRICH_AT_ONCE`개까지만 돈다: hook의 시간 한도는 plugin 실행 자리를 기다리는 시간도 쓰므로, 자리 수(8)만큼
/// 한꺼번에 줄을 세우면 뒤에 선 서버가 돌아 보지도 못하고 시간 초과가 되고 그동안 다른 모듈 호출도 줄을 선다
const LIST_ENRICH_AT_ONCE: usize = 4;

async fn enrich_results<Fut>(cache: &crate::ipc::ExtensionStatusCache, contexts: Vec<(String, Value)>, run: impl Fn(Value) -> Fut) -> std::collections::HashMap<String, Value>
where
    Fut: std::future::Future<Output = Vec<(String, anyhow::Result<Value>)>>,
{
    let asks = contexts.into_iter().map(|(id, ctx)| {
        let run = &run;
        async move {
            if let Some(cached) = cache.get(&id) {
                return (id, cached);
            }
            // 첫 번째 성공 응답만 캐시한다 (실패는 다음 목록 요청에서 다시 시도)
            let result = match run(ctx).await.into_iter().find_map(|(_id, r)| r.ok()) {
                Some(result) => {
                    cache.set(&id, result.clone());
                    result
                }
                None => json!({"handled": false}),
            };
            (id, result)
        }
    });
    futures_util::StreamExt::collect(futures_util::StreamExt::buffer_unordered(futures_util::stream::iter(asks), LIST_ENRICH_AT_ONCE)).await
}

/// `notices_after`: 이 번호 뒤의 포트 충돌 강제 정지 알림만 싣는다
pub(crate) async fn list(state: &IPCServer, notices_after: u64) -> axum::response::Response {
    let state = state.clone();
    // ── Phase 1: supervisor lock을 최소 시간만 잡고 필요한 데이터 복사 ──
    /// 목록 한 줄의 기본 모습(native 상태)과 hook·폴더 판정에 쓰는 값
    struct InstanceSnapshot {
        info: ServerInfo,
        instance_dir: String,
        process_patterns: Vec<String>,
    }

    let (snapshots, all_instances, module_protocols): (Vec<InstanceSnapshot>, Vec<_>, _) = {
        let supervisor = state.supervisor.read().await;
        let instances = supervisor.instance_store.list();
        let all_instances = instances.to_vec();
        let module_protocols = supervisor.rest_port_settings();
        let modules = supervisor.module_loader.modules().unwrap_or_default();
        let mut states = std::collections::HashMap::new();
        for instance in instances {
            states.insert(instance.id.as_str(), supervisor.tracked_state(&instance.id).await);
        }
        let snapshots =         instances.iter().map(|instance| {
            let (running, pid, start_time) = states[instance.id.as_str()];
            let module = modules.iter().find(|m| m.metadata.name == instance.module_name);
            let process_patterns = module.as_ref().map(|m| m.metadata.process_patterns.clone()).unwrap_or_default();
            let chat_patterns = module.as_ref().is_some_and(|m| !m.metadata.chat_patterns.is_empty());
            let kick_on_join = module.as_ref().is_some_and(|m| m.metadata.kick_command.is_some() && !m.metadata.player_join_patterns.is_empty());
            let managed = supervisor.uses_managed_start(instance);
            let counts_players = module.as_ref().is_some_and(|m| {
                m.metadata.players_function.is_some() || (managed && !m.metadata.player_join_patterns.is_empty() && !m.metadata.player_leave_patterns.is_empty())
            });
            InstanceSnapshot {
                info: ServerInfo {
                    id: instance.id.clone(),
                    name: instance.name.clone(),
                    module: instance.module_name.clone(),
                    // Native 모드: 스냅샷의 PID 및 시작 시간
                    status: if running { "running" } else { "stopped" }.to_string(),
                    pid,
                    start_time,
                    executable_path: instance.executable_path.clone(),
                    working_dir: instance.working_dir.clone(),
                    port: instance.port,
                    // REST 접속값은 모듈 설정에만 있다 (`[protocols.rest]`). 응답에는 예전 이름으로 풀어 싣는다
                    rest_host: module.as_ref().and_then(|m| m.metadata.rest_host(instance)),
                    rest_port: module.as_ref().and_then(|m| m.metadata.rest_port(instance)),
                    rest_username: module.as_ref().and_then(|m| m.metadata.rest()?.username.clone()),
                    rest_password: module.as_ref().and_then(|m| m.metadata.rest_password(instance)).map(str::to_string),
                    protocol_mode: instance.protocol_mode.clone(),
                    module_settings: instance.module_settings.clone(),
                    server_version: instance.server_version.clone(),
                    server_build: instance.server_build.clone(),
                    version_hidden: module
                        .as_ref()
                        .and_then(|m| m.metadata.settings.as_ref())
                        .map(|s| crate::validator::version_hidden(&s.fields, instance.server_version.as_deref()))
                        .unwrap_or_default(),
                    last_api_action: None,
                    provisioning: false,
                    managed,
                    extension_status: std::collections::HashMap::new(),
                    extension_data: instance.extension_data.clone(),
                    extra_args: instance.extra_args.clone(),
                    memory_limit_mb: instance.memory_limit_mb,
                    memory_limit_supported: crate::instance::MEMORY_LIMIT_ENFORCED,
                    auto_update: instance.auto_update,
                    branch_of: instance.branch_of.clone(),
                    game_chat: instance.game_chat,
                    game_chat_keyword: instance.game_chat_keyword.clone(),
                    access: instance.access,
                    controllers: instance.controllers.clone(),
                    controllers_only: instance.controllers_only,
                    kick_players: instance.kick_players.clone(),
                    game_chat_supported: chat_patterns && managed,
                    controllers_only_supported: kick_on_join && managed,
                    idle_stop_supported: counts_players,
                    idle_stopped: if running { None } else { crate::supervisor::idle::stopped(&instance.id) },
                    revision: instance.settings_revision(),
                    port_conflicts: vec![],
                    slots: Default::default(),
                    update_check: crate::service::server_update::last_check(&instance.id),
                    transition: crate::instance::transition::of(&instance.id).map(|k| k.as_str().to_string()),
                    players: None,
                    unresponsive: None,
                    unresponsive_minutes: None,
                    // 폴더 판정은 아래 락 밖에서
                    linked_folder: None,
                    restorable_settings: false,
                    stored: module
                        .and_then(|m| m.metadata.settings.as_ref())
                        .map(|s| crate::instance::stored_settings(instance, &s.fields))
                        .unwrap_or_default(),
                },
                instance_dir: supervisor.instance_store.instance_dir(&instance.id).to_string_lossy().to_string(),
                process_patterns,
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
            s.info.linked_folder = crate::instance::migration::linked_folder(s.info.working_dir.as_deref(), instance_dir);
            s.info.restorable_settings = crate::instance::migration::saved(instance_dir);
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
    // Extension이 런타임 상태/통계를 제공할 수 있음 (TTL 캐시 적용)
    let contexts = snapshots.iter().filter(|s| !s.info.extension_data.is_empty()).map(|s| {
        let ctx = serde_json::json!({
            "instance_id": &s.info.id,
            "instance_name": &s.info.name,
            "module": &s.info.module,
            "extension_data": &s.info.extension_data,
            "instance_dir": &s.instance_dir,
            "process_patterns": &s.process_patterns,
        });
        (s.info.id.clone(), ctx)
    }).collect();
    let ext_mgr = state.extension_manager.clone();
    let mut enriched = enrich_results(&state.extension_status_cache, contexts, |ctx| {
        let ext_mgr = ext_mgr.clone();
        async move { crate::extension::run_hook_timed(&ext_mgr, "server.list_enrich", ctx, 10).await }
    }).await;
    let mut servers = Vec::new();

    for InstanceSnapshot { info: mut server, .. } in snapshots {
        // ── Extension hook: server.list_enrich ──
        if let Some(enrich_result) = enriched.remove(&server.id) {
            // handled: true인 경우만 Extension이 서버 정보를 제공: 상태·PID·시작 시간·extension_status만 바꾼다
            let is_handled = enrich_result.get("handled")
                .and_then(|h| h.as_bool()) == Some(true);

            if is_handled {
                server.status = enrich_result.get("status").and_then(|s| s.as_str())
                    .unwrap_or("stopped").to_string();
                server.pid = None;
                server.start_time = None;
                // hook 응답에서 extension_id 필드가 있으면 해당 키로, 아니면 "default"
                let ext_key = enrich_result.get("extension_id")
                    .and_then(|v| v.as_str())
                    .unwrap_or("default")
                    .to_string();
                server.extension_status.insert(ext_key, enrich_result);
            }
        }

        // API(CLI/Discord)를 통한 마지막 시작/정지 타임스탬프 조회
        server.last_api_action = state.api_actions.get(&server.id);

        // 프로비저닝 진행 여부 확인 (tracker entry 존재 = provisioning UI 표시)
        let provision_entry = state.provision_tracker.get(&server.id);
        server.provisioning = provision_entry.is_some();
        // 프로비저닝 중이면 상태를 "provisioning"으로 오버라이드
        if provision_entry.as_ref().is_some_and(|p| !p.done) {
            server.status = "provisioning".to_string();
        }

        servers.push(server);
    }
    // ── 데이터 slot (Q-7): 조건 판정과 값 찾기는 여기서 한 번만 ──
    {
        let ext_mgr = state.extension_manager.read().await;
        for srv in servers.iter_mut() {
            srv.slots = ext_mgr.server_slots(&srv.extension_data, &srv.extension_status);
        }
    }

    // ── 접속자 수 (Q-5): 모듈이 알려 준 마지막 값, 켜진 서버만 ──
    let catalog = crate::chat::i18n::Catalog::for_language(&state.config_store.ui_language().await);
    let runtime = state.supervisor.read().await.runtime.clone();
    for srv in servers.iter_mut().filter(|s| s.status == "running") {
        srv.players = crate::supervisor::players::get(&runtime, &srv.id);
        // 응답 없음은 인스턴스 설정의 시간(`watchdog::rule_of`)이 지난 뒤에 알린다
        let after = all_instances.iter().find(|i| i.id == srv.id).map(|i| crate::supervisor::watchdog::rule_of(i).after).unwrap_or_default();
        if let Some((silent, reason)) = crate::supervisor::players::silent_for(&runtime, &srv.id, srv.pid, std::time::Instant::now()).filter(|(silent, _)| *silent >= after) {
            srv.unresponsive = Some(crate::supervisor::players::reason_text(&reason, &catalog));
            srv.unresponsive_minutes = Some(silent.as_secs() / 60);
        }
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
                &module_protocols,
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

    // 포트 충돌 강제 정지 이벤트 (짧은 read lock)
    let port_conflict_stops = state.supervisor.read().await.port_conflict_stops_after(notices_after);

    Json(ServerListResponse { servers, port_conflict_stops }).into_response()
}

/// 접속자 목록 (`GET /api/instance/:id/players`와 채팅이 같이 쓴다)
pub(crate) async fn players(state: &IPCServer, id: &str) -> axum::response::Response {
    let fail = |status: StatusCode, code: &str, error: String| (status, Json(json!({ "success": false, "error_code": code, "error": error }))).into_response();
    let call = {
        let sup = state.supervisor.read().await;
        let Some(instance) = sup.instance_store.get(&id) else {
            return fail(StatusCode::NOT_FOUND, "not_found", format!("Instance not found: {}", id));
        };
        let Some(function) = sup.module_loader.get_module(&instance.module_name).ok().and_then(|m| m.metadata.players_function) else {
            return fail(StatusCode::OK, "not_supported", "This server's module cannot list players".into());
        };
        if sup.tracker.get_pid(&id).is_err() {
            return fail(StatusCode::CONFLICT, "server_not_running", "The server is not running".into());
        }
        sup.instance_call(&id, &function)
    };
    let mut call = match call {
        Ok(c) => c,
        Err(e) => return fail(StatusCode::INTERNAL_SERVER_ERROR, "players_failed", e.to_string()),
    };
    if let serde_json::Value::Object(cfg) = call.config_mut() {
        cfg.insert("players_detail".to_string(), json!(true));
    }
    match call.timeout(crate::supervisor::players::CALL_TIMEOUT_SECS).run().await {
        Ok(result) => match crate::supervisor::players::parse(&result) {
            Some(count) => {
                let names: Vec<serde_json::Value> = result
                    .get("players")
                    .and_then(serde_json::Value::as_array)
                    .into_iter()
                    .flatten()
                    .filter_map(|p| p.get("name").and_then(serde_json::Value::as_str))
                    // 게임이 이름을 자유롭게 두는 경우: 제어 문자를 빼고 64자까지 (요약 줄이 깨지지 않게)
                    .map(|name| name.chars().filter(|c| !c.is_control()).take(64).collect::<String>())
                    .filter(|name| !name.is_empty())
                    .map(|name| json!({ "name": name }))
                    .collect();
                let mut body = json!({ "success": true, "online": count.online, "players": names });
                if let Some(max) = count.max {
                    body["max"] = json!(max);
                }
                (StatusCode::OK, Json(body)).into_response()
            }
            None => {
                if result.get("success").and_then(serde_json::Value::as_bool) == Some(false) {
                    return (StatusCode::OK, Json(result)).into_response();
                }
                let message = result.get("message").and_then(serde_json::Value::as_str).unwrap_or("The module did not report players");
                fail(StatusCode::OK, "players_failed", message.to_string())
            }
        },
        Err(e) => {
            tracing::warn!("Player list for '{}' failed: {}", id, e);
            (StatusCode::OK, Json(crate::plugin::error_response(&e))).into_response()
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// 캐시에 답이 없는 서버들의 `server.list_enrich`는 함께 돈다 (차례로 돌면 목록이 서버 수만큼 늦는다).
    /// 캐시에 있는 서버는 부르지 않고, 받은 답은 캐시에 남는다
    #[tokio::test]
    async fn list_enrich_hooks_for_uncached_servers_run_together() {
        let cache = crate::ipc::ExtensionStatusCache::new(30);
        cache.set("cached", json!({ "handled": true, "status": "running" }));
        let all_started = tokio::sync::Barrier::new(3);
        let run = |ctx: Value| {
            let all_started = &all_started;
            async move {
                assert_ne!(ctx["instance_id"], "cached", "a cached server is not asked again");
                all_started.wait().await;
                if ctx["instance_id"] == "failing" {
                    return vec![("docker".to_string(), Err(anyhow::anyhow!("hook failed")))];
                }
                vec![("docker".to_string(), Ok(json!({ "handled": true, "status": "starting", "for": ctx["instance_id"] })))]
            }
        };
        let contexts = ["a", "cached", "b", "failing"].map(|id| (id.to_string(), json!({ "instance_id": id }))).to_vec();
        let results = tokio::time::timeout(std::time::Duration::from_secs(5), enrich_results(&cache, contexts, run))
            .await
            .expect("the hooks must not wait for each other");
        assert_eq!(results["a"]["for"], "a");
        assert_eq!(results["b"]["for"], "b");
        assert_eq!(results["cached"]["status"], "running");
        assert_eq!(results["failing"], json!({ "handled": false }));
        assert_eq!(cache.get("a").unwrap()["for"], "a");
        assert!(cache.get("failing").is_none(), "a failed hook is asked again on the next list");
    }

    /// 서버가 많아도 `server.list_enrich`는 몇 개씩만 함께 돈다: plugin 실행 자리를 목록 요청 하나가 다 쥐지 않고,
    /// 뒤에 선 서버가 자리를 기다리다 hook 시간 한도를 넘기지 않는다
    #[tokio::test]
    async fn list_enrich_hooks_for_many_servers_do_not_all_start_at_once() {
        use std::sync::atomic::{AtomicUsize, Ordering};
        let cache = crate::ipc::ExtensionStatusCache::new(30);
        let (running, most) = (AtomicUsize::new(0), AtomicUsize::new(0));
        let run = |ctx: Value| {
            let (running, most) = (&running, &most);
            async move {
                most.fetch_max(running.fetch_add(1, Ordering::SeqCst) + 1, Ordering::SeqCst);
                tokio::time::sleep(std::time::Duration::from_millis(20)).await;
                running.fetch_sub(1, Ordering::SeqCst);
                vec![("docker".to_string(), Ok(json!({ "handled": true, "for": ctx["instance_id"] })))]
            }
        };
        let contexts: Vec<_> = (0..12).map(|i| (format!("s{i}"), json!({ "instance_id": format!("s{i}") }))).collect();
        let results = enrich_results(&cache, contexts, run).await;
        assert_eq!(results.len(), 12);
        assert_eq!(results["s11"]["for"], "s11");
        assert_eq!(most.load(Ordering::SeqCst), 4);
    }
}
