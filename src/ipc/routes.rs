//! HTTP 라우터 — 주제별 서브라우터 모음.
//!
//! `IPCServer::start()` 가 한 곳에 90개 넘는 `.route(...)` 를 직접 나열하던
//! 구조를 분리. 핸들러 함수 자체는 [`super::handlers`] 의 기존 위치 그대로이며,
//! 여기서는 라우트 ↔ 핸들러 매핑만 토픽별로 묶는다.
//!
//! 라우터를 추가/수정할 때는 동일 토픽의 함수 안에서만 작업하면 되도록 분리되어
//! 있어, 신규 엔드포인트 추가 시 충돌 영역이 좁다.

use axum::{
    routing::{delete, get, patch, post, put},
    Router,
};

use super::{auth, handlers, updates, IPCServer};

/// 인증된 ping + 서버(인스턴스) 단위 조회/제어 + 모듈 카탈로그.
fn server_routes() -> Router<IPCServer> {
    Router::new()
        .route("/api/ping", get(handlers::server::auth_ping))
        .route("/api/servers", get(handlers::server::list_servers))
        .route(
            "/api/server/:name/status",
            get(handlers::server::get_server_status),
        )
        .route(
            "/api/server/:name/start",
            post(handlers::server::start_server_handler),
        )
        .route(
            "/api/server/:name/stop",
            post(handlers::server::stop_server_handler),
        )
        .route("/api/modules", get(handlers::server::list_modules))
        .route(
            "/api/modules/refresh",
            post(handlers::server::refresh_modules),
        )
        .route(
            "/api/modules/manifest",
            get(handlers::server::fetch_module_manifest),
        )
        .route(
            "/api/modules/manifest/:id/install",
            post(handlers::server::install_module_from_manifest),
        )
        .route("/api/modules/:id", delete(handlers::server::remove_module))
        .route(
            "/api/module/:name",
            get(handlers::server::get_module_metadata),
        )
}

/// 인스턴스 CRUD + provisioning 진행 상태 + 게임서버 업데이트.
fn instance_routes() -> Router<IPCServer> {
    Router::new()
        .route(
            "/api/instances",
            get(handlers::instance::list_instances)
                .post(handlers::instance::create_instance),
        )
        .route(
            "/api/instances/reorder",
            put(handlers::instance::reorder_instances),
        )
        .route(
            "/api/instance/:id",
            get(handlers::instance::get_instance)
                .delete(handlers::instance::delete_instance)
                .patch(handlers::instance::update_instance_settings),
        )
        .route(
            "/api/provision-progress/:name",
            get(handlers::instance::get_provision_progress)
                .delete(handlers::instance::dismiss_provision_progress),
        )
        .route(
            "/api/instance/:id/check-update",
            get(handlers::instance::check_server_update),
        )
        .route(
            "/api/instance/:id/apply-update",
            post(handlers::instance::apply_server_update),
        )
}

/// AI 에이전트용 통합 명령 진입점 (이름/id 해석 + 동작 디스패치).
fn agent_routes() -> Router<IPCServer> {
    Router::new().route(
        "/api/agent/command",
        post(handlers::agent::agent_command),
    )
}

/// 명령 실행 (모듈 커맨드 / RCON / REST).
fn command_routes() -> Router<IPCServer> {
    Router::new()
        .route(
            "/api/instance/:id/command",
            post(handlers::command::execute_command),
        )
        .route(
            "/api/instance/:id/rcon",
            post(handlers::command::execute_rcon_command),
        )
        .route(
            "/api/instance/:id/rest",
            post(handlers::command::execute_rest_command),
        )
}

/// Managed 프로세스 (데몬이 stdio 캡처) + 서버 설치/버전 + 모듈 자체 기능 엔드포인트.
fn managed_routes() -> Router<IPCServer> {
    Router::new()
        .route(
            "/api/instance/:id/managed/start",
            post(handlers::managed::start_managed_handler),
        )
        .route(
            "/api/instance/:id/console",
            get(handlers::managed::get_console_handler),
        )
        .route(
            "/api/instance/:id/stdin",
            post(handlers::managed::send_stdin_handler),
        )
        .route(
            "/api/instance/:id/validate",
            post(handlers::managed::validate_instance_handler),
        )
        .route(
            "/api/instance/:id/properties",
            get(handlers::managed::read_properties_handler)
                .put(handlers::managed::write_properties_handler),
        )
        .route(
            "/api/instance/:id/settings/live",
            get(handlers::managed::read_live_settings_handler),
        )
        .route(
            "/api/instance/:id/properties/reset",
            post(handlers::managed::reset_properties_handler),
        )
        .route(
            "/api/instance/:id/server/reset",
            post(handlers::managed::reset_server_handler),
        )
        .route(
            "/api/instance/:id/accept-eula",
            post(handlers::managed::accept_eula_handler),
        )
        .route(
            "/api/instance/:id/diagnose",
            post(handlers::managed::diagnose_handler),
        )
        .route(
            "/api/module/:name/versions",
            get(handlers::managed::list_versions_handler),
        )
        .route(
            "/api/module/:name/version/:version",
            get(handlers::managed::get_version_details_handler),
        )
        .route(
            "/api/module/:name/install",
            post(handlers::managed::install_server_handler),
        )
        .route(
            "/api/instance/:id/installed-version",
            get(handlers::managed::get_installed_version_handler),
        )
}

/// GUI/봇/시스템 설정, 노드 토큰, 모듈 로케일, 디렉토리 스캔.
fn config_routes() -> Router<IPCServer> {
    Router::new()
        .route(
            "/api/config/bot",
            get(handlers::bot::get_bot_config).put(handlers::bot::save_bot_config),
        )
        .route(
            "/api/config/gui",
            get(handlers::config::get_gui_config).put(handlers::config::save_gui_config),
        )
        .route(
            "/api/config/node-token",
            get(handlers::config::get_node_token)
                .put(handlers::config::save_node_token)
                .delete(handlers::config::delete_node_token),
        )
        .route(
            "/api/module/:name/locales",
            get(handlers::config::get_module_locales),
        )
        .route(
            "/api/system/components",
            get(handlers::config::get_system_components),
        )
        .route("/api/fs/scan-dir", post(handlers::config::scan_directory))
}

/// 클라이언트(GUI/CLI) 등록·heartbeat·해제 + 데몬 lifecycle.
fn client_routes() -> Router<IPCServer> {
    Router::new()
        .route(
            "/api/client/register",
            post(handlers::client::client_register),
        )
        .route(
            "/api/client/:id/heartbeat",
            post(handlers::client::client_heartbeat),
        )
        .route(
            "/api/client/:id/unregister",
            delete(handlers::client::client_unregister),
        )
        .route(
            "/api/daemon/shutdown",
            post(handlers::client::daemon_shutdown),
        )
        .route(
            "/api/daemon/console",
            get(handlers::client::daemon_console),
        )
}

/// 익스텐션 매니저 (목록·활성화·설치·번들 서빙).
fn extension_routes() -> Router<IPCServer> {
    Router::new()
        .route(
            "/api/extensions",
            get(handlers::extension::list_extensions),
        )
        .route(
            "/api/extensions/init-status",
            get(handlers::extension::extension_init_status),
        )
        .route(
            "/api/extensions/rescan",
            post(handlers::extension::rescan_extensions),
        )
        .route(
            "/api/extensions/manifest",
            get(handlers::extension::fetch_manifest),
        )
        .route(
            "/api/extensions/updates",
            get(handlers::extension::check_extension_updates),
        )
        .route(
            "/api/extensions/:id/enable",
            post(handlers::extension::enable_extension),
        )
        .route(
            "/api/extensions/:id/disable",
            post(handlers::extension::disable_extension),
        )
        .route(
            "/api/extensions/:id/install",
            post(handlers::extension::install_extension),
        )
        .route(
            "/api/extensions/:id",
            delete(handlers::extension::remove_extension),
        )
        .route(
            "/api/extensions/:id/config",
            get(handlers::extension::get_extension_config)
                .put(handlers::extension::save_extension_config),
        )
        .route(
            "/api/extensions/:id/gui",
            get(handlers::extension::serve_gui_bundle),
        )
        .route(
            "/api/extensions/:id/gui/styles",
            get(handlers::extension::serve_gui_styles),
        )
        .route(
            "/api/extensions/:id/icon",
            get(handlers::extension::serve_icon),
        )
        .route(
            "/api/extensions/:id/i18n/:locale",
            get(handlers::extension::serve_i18n),
        )
}

/// 데몬 자체 런타임 preflight (포터블 Python).
fn runtime_routes() -> Router<IPCServer> {
    Router::new()
        .route(
            "/api/runtime/preflight",
            get(handlers::runtime::preflight_status),
        )
}

/// 클라우드 모드 — 릴레이 서버 프록시 (호스트/노드/페어링).
fn relay_routes() -> Router<IPCServer> {
    Router::new()
        .route(
            "/api/relay/host/:host_id/status",
            get(handlers::relay::check_host_status),
        )
        .route(
            "/api/relay/host/:host_id/nodes",
            get(handlers::relay::list_host_nodes),
        )
        .route(
            "/api/relay/host/:host_id/bot-config",
            patch(handlers::relay::sync_bot_config),
        )
        .route(
            "/api/relay/node/:guild_id/members",
            get(handlers::relay::list_node_members),
        )
        .route(
            "/api/relay/node/:guild_id/settings",
            get(handlers::relay::get_node_settings)
                .patch(handlers::relay::update_node_settings),
        )
        .route(
            "/api/relay/pair/initiate",
            post(handlers::relay::initiate_pairing),
        )
        .route(
            "/api/relay/pair/:code/status",
            get(handlers::relay::poll_pairing_status),
        )
        .route(
            "/api/relay/bot-invite",
            get(handlers::relay::get_bot_invite),
        )
}

/// 인증이 필요한 모든 토픽별 라우터를 머지.
///
/// `auth_middleware` 적용 + state 주입 전 단계의 라우터를 반환한다.
fn authenticated_routes() -> Router<IPCServer> {
    Router::new()
        .merge(server_routes())
        .merge(instance_routes())
        .merge(agent_routes())
        .merge(command_routes())
        .merge(managed_routes())
        .merge(config_routes())
        .merge(client_routes())
        .merge(extension_routes())
        .merge(runtime_routes())
        .merge(relay_routes())
}

/// 데몬 IPC HTTP 라우터 전체를 빌드한다.
///
/// 구성 (바깥 → 안):
/// 1. `host_guard_middleware` — 모든 요청에서 루프백이 아닌 `Host` 거부 (DNS rebinding 방어)
/// 2. `GET /health` — 인증 없음. 데몬이 listen 중인지 확인하는 용도라 버전만 반환한다.
/// 3. `auth_middleware` — 그 밖의 모든 라우트 (`/api/updates/*` 포함)에 `X-Saba-Token` 요구
pub fn build_router(state: IPCServer) -> Router {
    let update_state = state.update_state.clone();

    Router::new()
        .merge(authenticated_routes().with_state(state))
        .merge(updates::updates_router(update_state))
        .layer(axum::middleware::from_fn(auth::auth_middleware))
        .route("/health", get(handlers::server::health_check))
        .layer(axum::middleware::from_fn(auth::host_guard_middleware))
}
