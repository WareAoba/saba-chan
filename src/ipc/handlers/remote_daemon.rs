//! 원격 데몬 설정과 짝 맺은 PC (D1, `crate::remote_daemon`). 로컬 전용: 원격 포트에서는 막는다
//!
//!   GET    /api/remote/settings            {enabled, port, address, transport, status, hostId, thisPc} hostId = 이 PC의 릴레이 PC id (없으면 "")
//!                                          thisPc = {platform, agentVersion, publicIp} 이 PC 카드 (publicIp: 릴레이가 본 공인 IP, 모르면 null)
//!   PUT    /api/remote/settings            {enabled?, port?, address?} → 저장하고 원격 포트를 다시 맞춘다
//!   GET    /api/remote/clients             {clients:[{hostId, name, ownerName, label, pairedAt}]}
//!   DELETE /api/remote/clients/:hostId     짝을 푼다 (그 PC의 토큰이 더는 통하지 않는다)
//!
//! 붙는 쪽 (D1b, `remote_daemon::peer`):
//!   GET    /api/remote/peers                         {peers:[{hostId, name, ownerName, label, status, hasEndpoint, saved, platform, agentVersion,
//!                                                     createdAt, lastSeenAt, publicIp, route}]} 같은 주인의 다른 PC (릴레이). 읽을 때 릴레이 목록에 맞춘다
//!                                                     (`remote_daemon::sync_with_relay`). route = 저장한 PC에 지금 붙는 길 `direct`|`relay` (저장 안 했으면 null)
//!   GET    /api/remote/daemons                       {daemons:[{hostId, name, ownerName, label, address, port, pairedAt}]} 이 PC가 붙는 원격 데몬
//!   POST   /api/remote/daemons {hostId}              짝을 맺고 저장한다 → {hostId, name, label}
//!   label = 화면에 보일 이름 `디스코드 이름(PC 이름)` (`config::label`)
//!   DELETE /api/remote/daemons/:hostId               잊는다. 그 PC에도 짝을 풀라고 알린다 (`DELETE /remote/self`, 닿지 않으면 그대로 잊는다)
//!   POST   /api/remote/daemons/:hostId/remove {uninstall, keepSettings, keepInstances, removePc}
//!                                                    PC 카드의 지우기: 그 PC에서 사바쨩 제거(`/api/system/uninstall`) → 서버쨩 내 PC 목록에서 빼기 → 잊기
//!                                                    → {uninstalled, removedFromRelay}
//!   POST   /api/remote/claim {code}                  다른 PC가 낸 페어링 코드를 이 PC 주인의 것으로 받고 그 PC와 짝을 맺는다 → {hostId, name, label}
//!   POST   /api/remote/install {host, user, port?, linger?, steamLibs?}  이 PC의 ssh로 그 PC에 설치하고 짝을 맺는다 → {id}
//!   GET    /api/remote/install/:id?since=N           {phase, output, offset, prompt, error_code, hostId, name} (output은 since 뒤만)
//!   POST   /api/remote/install/:id/input {text}      ssh·sudo 물음에 답한다 (남기지 않는다)
//!   DELETE /api/remote/install/:id                   그만두거나(진행 중) 끝난 설치를 치운다
//!   ANY    /api/remote/daemons/:hostId/proxy/*path   원격 데몬의 `/path`(`api/…`·`health`)에 그대로 보낸다
//!   GET    /api/remote/servers                       {daemons:[{hostId, name, label, ok, error_code?, servers}]} 이 PC(hostId null)와 원격 데몬마다의 서버 (G1c 합쳐 보기)

use axum::{
    body::Body,
    extract::{Path, State},
    http::{Request, StatusCode},
    response::{IntoResponse, Response},
    Json,
};
use saba_chan_updater_lib::config_file::ConfigFile;
use serde::Deserialize;
use serde_json::{json, Value};

use crate::ipc::IPCServer;
use crate::remote_daemon::{self, config, peer};

fn bad(code: &str, message: &str) -> Response {
    (StatusCode::BAD_REQUEST, Json(json!({ "error": message, "error_code": code }))).into_response()
}

fn view(settings: &config::Settings, status: &remote_daemon::Status) -> Value {
    let mut v = serde_json::to_value(settings).unwrap_or_default();
    v["status"] = serde_json::to_value(status).unwrap_or_default();
    v
}

pub async fn get_settings(State(state): State<IPCServer>) -> Json<Value> {
    let settings = config::settings_in(&ConfigFile::default_location());
    let mut v = view(&settings, &remote_daemon::status().await);
    v["thisPc"] = json!({
        "platform": crate::relay_agent::platform(),
        "agentVersion": crate::relay_agent::AGENT_VERSION,
        "publicIp": state.relay_agent.seen_address(),
    });
    // 원격 데몬의 이벤트 중 이 PC가 시킨 것(`remoteHost`)을 가리는 데 쓴다 (셸 효과음)
    let host = crate::ipc::handlers::relay::load_node_token();
    v["hostId"] = json!(host.as_deref().and_then(|t| crate::ipc::handlers::relay::parse_node_token(t.trim())).map_or("", |(id, _)| id));
    Json(v)
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SettingsPatch {
    enabled: Option<bool>,
    port: Option<u16>,
    address: Option<String>,
}

pub async fn put_settings(State(state): State<IPCServer>, Json(patch): Json<SettingsPatch>) -> Response {
    let file = ConfigFile::default_location();
    let mut settings = config::settings_in(&file);
    if let Some(port) = patch.port {
        if port == 0 {
            return bad("invalid_port", "Port must be between 1 and 65535");
        }
        settings.port = port;
    }
    if let Some(address) = patch.address {
        let address = address.trim().to_string();
        if !address.is_empty() && !remote_daemon::valid_address(&address) {
            return bad("invalid_address", "Address must be a host name or an IP address");
        }
        settings.address = address;
    }
    if let Some(enabled) = patch.enabled {
        settings.enabled = enabled;
    }
    if let Err(e) = config::save_settings_in(&file, &settings) {
        return (StatusCode::INTERNAL_SERVER_ERROR, Json(json!({ "error": e.to_string(), "error_code": "save_failed" }))).into_response();
    }
    let status = remote_daemon::apply(&state).await;
    Json(view(&settings, &status)).into_response()
}

pub async fn list_clients() -> Json<Value> {
    let clients: Vec<Value> = config::clients_in(&ConfigFile::default_location())
        .into_iter()
        .map(|c| json!({ "hostId": c.host_id, "label": c.label(), "name": c.name, "ownerName": c.owner_name, "pairedAt": c.paired_at }))
        .collect();
    Json(json!({ "clients": clients }))
}

pub async fn revoke_client(Path(host_id): Path<String>) -> Response {
    let file = ConfigFile::default_location();
    match config::revoke_in(&file, &host_id) {
        Ok(true) => {
            remote_daemon::reload_clients(&file);
            StatusCode::NO_CONTENT.into_response()
        }
        Ok(false) => (StatusCode::NOT_FOUND, Json(json!({ "error": "Not paired", "error_code": "not_paired" }))).into_response(),
        Err(e) => (StatusCode::INTERNAL_SERVER_ERROR, Json(json!({ "error": e.to_string(), "error_code": "save_failed" }))).into_response(),
    }
}

async fn relay_target(state: &IPCServer) -> Option<remote_daemon::relay::Target> {
    remote_daemon::server::RelaySource::Configured(state.clone()).target().await
}

pub async fn list_peers(State(state): State<IPCServer>) -> Response {
    let Some(target) = relay_target(&state).await else {
        return peer::error(StatusCode::SERVICE_UNAVAILABLE, "relay_not_paired");
    };
    let file = ConfigFile::default_location();
    // 릴레이의 목록이 기준이다: 이름을 맞추고, 빠진 PC의 짝을 풀고, 바뀐 주소를 따른다
    let Ok(peers) = remote_daemon::sync_with_relay(&file, &target).await else {
        return peer::error(StatusCode::BAD_GATEWAY, "relay_unreachable");
    };
    let saved = config::daemons_in(&file);
    let peers: Vec<Value> = peers
        .into_iter()
        .map(|p| {
            let saved = saved.iter().find(|d| d.host_id == p.host_id);
            json!({
                "hostId": p.host_id,
                "label": config::label(&p.owner_name, &p.name),
                "name": p.name,
                "ownerName": p.owner_name,
                "status": p.status,
                "hasEndpoint": p.endpoint.is_some(),
                "saved": saved.is_some(),
                "platform": p.platform,
                "agentVersion": p.agent_version,
                "createdAt": p.created_at,
                "lastSeenAt": p.last_seen_at,
                "publicIp": p.public_ip,
                "pairedAt": saved.map(|d| d.paired_at.clone()),
                "route": saved.map(peer::route_of),
            })
        })
        .collect();
    Json(json!({ "peers": peers })).into_response()
}

pub async fn list_daemons() -> Json<Value> {
    let daemons: Vec<Value> = config::daemons_in(&ConfigFile::default_location())
        .into_iter()
        .map(|d| {
            json!({
                "hostId": d.host_id,
                "label": d.label(),
                "name": d.name,
                "ownerName": d.owner_name,
                "address": d.address,
                "port": d.port,
                "pairedAt": d.paired_at,
            })
        })
        .collect();
    Json(json!({ "daemons": daemons }))
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ConnectBody {
    host_id: String,
}

pub async fn connect_daemon(State(state): State<IPCServer>, Json(body): Json<ConnectBody>) -> Response {
    let host_id = body.host_id.trim();
    if host_id.is_empty() {
        return bad("invalid_host", "hostId is required");
    }
    match peer::connect(&ConfigFile::default_location(), relay_target(&state).await, host_id).await {
        Ok(d) => Json(json!({ "hostId": d.host_id, "name": d.name, "label": d.label() })).into_response(),
        Err(e) => peer::error(e.status(), e.code()),
    }
}

/// 그 PC에 짝을 풀라고 알리는 시간. 닿지 않아도 이 PC에서는 잊는다 (그 PC는 서버쨩 목록 동기화나 다음 짝 맺기에서 정리된다)
const TELL_REMOTE_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(8);

async fn tell_remote_to_unpair(file: &ConfigFile, state: &IPCServer, host_id: &str) {
    let req = Request::delete("/remote/self").body(Body::empty()).unwrap_or_default();
    let ask = peer::forward(file, relay_target(state).await, host_id, "/remote/self", req, state.shutdown_token.clone());
    match tokio::time::timeout(TELL_REMOTE_TIMEOUT, ask).await {
        Ok(res) if res.status().is_success() => {}
        _ => tracing::info!("Could not tell remote daemon {} to unpair; forgetting it here only", host_id),
    }
}

/// 이 PC에서 잊는다. 실패하면 그 답
fn forget_here(file: &ConfigFile, host_id: &str) -> Option<Response> {
    match config::forget_daemon_in(file, host_id) {
        Ok(true) => {
            crate::remote_daemon::tunnel::close(host_id);
            None
        }
        Ok(false) => Some(peer::error(StatusCode::NOT_FOUND, "remote_unknown")),
        Err(_) => Some(peer::error(StatusCode::INTERNAL_SERVER_ERROR, "save_failed")),
    }
}

pub async fn forget_daemon(State(state): State<IPCServer>, Path(host_id): Path<String>) -> Response {
    let file = ConfigFile::default_location();
    if !config::daemons_in(&file).iter().any(|d| d.host_id == host_id) {
        return peer::error(StatusCode::NOT_FOUND, "remote_unknown");
    }
    tell_remote_to_unpair(&file, &state, &host_id).await;
    forget_here(&file, &host_id).unwrap_or_else(|| StatusCode::NO_CONTENT.into_response())
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct RemoveBody {
    #[serde(default)]
    uninstall: bool,
    #[serde(default = "yes")]
    keep_settings: bool,
    #[serde(default = "yes")]
    keep_instances: bool,
    #[serde(default)]
    remove_pc: bool,
}

fn yes() -> bool {
    true
}

pub async fn remove_daemon(State(state): State<IPCServer>, Path(host_id): Path<String>, Json(body): Json<RemoveBody>) -> Response {
    let file = ConfigFile::default_location();
    let saved = config::daemons_in(&file).iter().any(|d| d.host_id == host_id);
    let relay = relay_target(&state).await;
    let mut uninstalled = false;
    if body.uninstall {
        if !saved {
            return peer::error(StatusCode::NOT_FOUND, "remote_unknown");
        }
        let req = Request::post("/api/system/uninstall")
            .header(axum::http::header::CONTENT_TYPE, "application/json")
            .body(Body::from(json!({ "keepSettings": body.keep_settings, "keepInstances": body.keep_instances }).to_string()))
            .unwrap_or_default();
        let res = peer::forward(&file, relay.clone(), &host_id, "/api/system/uninstall", req, state.shutdown_token.clone()).await;
        if !res.status().is_success() {
            // 지우지 못했으면 잊지 않는다 (다시 해 볼 수 있게). 그 PC의 오류 코드를 그대로 알린다
            let status = res.status();
            let code = body_json(res).await.get("error_code").and_then(Value::as_str).unwrap_or("remote_uninstall_failed").to_string();
            return peer::error(if status.is_client_error() || status.is_server_error() { status } else { StatusCode::BAD_GATEWAY }, &code);
        }
        uninstalled = true;
    }
    // 설정을 남기고 지우면 그 PC의 짝 목록도 남는다: 지우기가 시작되기 전에 이 PC의 짝을 풀게 한다
    if saved {
        tell_remote_to_unpair(&file, &state, &host_id).await;
    }
    let mut removed_from_relay = false;
    if body.remove_pc {
        let Some(target) = relay.as_ref() else { return peer::error(StatusCode::SERVICE_UNAVAILABLE, "relay_not_paired") };
        match remote_daemon::relay::remove_peer(target, &host_id).await {
            Ok(removed) => removed_from_relay = removed,
            Err(_) => return peer::error(StatusCode::BAD_GATEWAY, "relay_unreachable"),
        }
    }
    if saved {
        if let Some(res) = forget_here(&file, &host_id) {
            return res;
        }
    }
    Json(json!({ "uninstalled": uninstalled, "removedFromRelay": removed_from_relay })).into_response()
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ClaimBody {
    code: String,
}

pub async fn claim(State(state): State<IPCServer>, Json(body): Json<ClaimBody>) -> Response {
    let code = body.code.trim();
    if !remote_daemon::onboard::valid_code(code) {
        return bad("invalid_code", "Pairing code is not valid");
    }
    match remote_daemon::onboard::claim_and_connect(&ConfigFile::default_location(), relay_target(&state).await, code, &state.shutdown_token).await {
        Ok(d) => Json(json!({ "hostId": d.host_id, "name": d.name, "label": d.label() })).into_response(),
        Err(e) => peer::error(e.status(), e.code()),
    }
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct InstallBody {
    host: String,
    user: String,
    #[serde(default)]
    port: Option<u16>,
    #[serde(default)]
    linger: bool,
    #[serde(default)]
    steam_libs: bool,
}

pub async fn start_install(State(state): State<IPCServer>, Json(body): Json<InstallBody>) -> Response {
    use remote_daemon::onboard::{self, InstallOptions, StartError};
    let host = body.host.trim();
    if !remote_daemon::valid_address(host) {
        return bad("invalid_host", "Host must be a host name or an IP address");
    }
    let user = body.user.trim();
    if !onboard::valid_user(user) {
        return bad("invalid_user", "User name is not valid");
    }
    let port = body.port.unwrap_or(22);
    if port == 0 {
        return bad("invalid_port", "Port must be between 1 and 65535");
    }
    let options = InstallOptions { host: host.to_string(), user: user.to_string(), port, linger: body.linger, steam_libs: body.steam_libs };
    match onboard::start_install(ConfigFile::default_location(), relay_target(&state).await, &options, state.shutdown_token.clone()) {
        Ok(id) => Json(json!({ "id": id })).into_response(),
        Err(StartError::SshMissing) => peer::error(StatusCode::CONFLICT, "ssh_missing"),
        Err(StartError::SpawnFailed) => peer::error(StatusCode::INTERNAL_SERVER_ERROR, "ssh_spawn_failed"),
    }
}

#[derive(Deserialize)]
pub struct SinceQuery {
    #[serde(default)]
    since: usize,
}

pub async fn get_install(Path(id): Path<String>, axum::extract::Query(q): axum::extract::Query<SinceQuery>) -> Response {
    match remote_daemon::onboard::install(&id) {
        Some(s) => Json(s.view(q.since)).into_response(),
        None => peer::error(StatusCode::NOT_FOUND, "install_unknown"),
    }
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub struct InputBody {
    text: String,
}

pub async fn install_input(Path(id): Path<String>, Json(body): Json<InputBody>) -> Response {
    if body.text.len() > 1024 || body.text.contains(['\r', '\n']) {
        return bad("invalid_input", "Answer must be one line");
    }
    match remote_daemon::onboard::install(&id) {
        Some(s) if s.answer(&body.text) => StatusCode::NO_CONTENT.into_response(),
        Some(_) => peer::error(StatusCode::CONFLICT, "install_finished"),
        None => peer::error(StatusCode::NOT_FOUND, "install_unknown"),
    }
}

pub async fn cancel_install(Path(id): Path<String>) -> Response {
    match remote_daemon::onboard::remove_install(&id) {
        Some(s) => {
            s.cancel();
            StatusCode::NO_CONTENT.into_response()
        }
        None => peer::error(StatusCode::NOT_FOUND, "install_unknown"),
    }
}

/// 원격 데몬 하나의 서버 목록을 기다리는 시간. 하나가 늦어도 합쳐 보기 전체가 멈추지 않게.
/// 옛 주소 붙기(10초) → 릴레이에서 새 주소 → 다시 붙기(10초)까지는 끝나야 한다 (`peer::forward`)
const REMOTE_LIST_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(30);

pub async fn list_all_servers(State(state): State<IPCServer>) -> Json<Value> {
    Json(all_servers_with(&ConfigFile::default_location(), &state, REMOTE_LIST_TIMEOUT).await)
}

async fn body_json(res: Response) -> Value {
    let bytes = axum::body::to_bytes(res.into_body(), 16 * 1024 * 1024).await.unwrap_or_default();
    serde_json::from_slice(&bytes).unwrap_or(Value::Null)
}

/// 이 PC와 저장한 원격 데몬마다의 서버 목록. 원격 데몬은 동시에 묻고, 포트 충돌 알림은 비우지 않는다(`notices=keep`)
async fn all_servers_with(file: &ConfigFile, state: &IPCServer, timeout: std::time::Duration) -> Value {
    let local = body_json(crate::ipc::handlers::server::list_servers_internal(state).await).await;
    let mut daemons = vec![json!({ "hostId": null, "name": "", "label": "", "ok": true, "servers": local["servers"].as_array().cloned().unwrap_or_default() })];
    let relay = relay_target(state).await;
    let asks = config::daemons_in(file).into_iter().map(|d| {
        let relay = relay.clone();
        async move {
            let req = Request::get("/api/servers").body(Body::empty()).unwrap_or_default();
            // 본문까지 시간 안에 받는다 (머리만 오고 연결이 끊기면 본문을 기다리며 매달린다)
            let ask = async {
                let res = peer::forward(file, relay, &d.host_id, "/api/servers?notices=keep", req, state.shutdown_token.clone()).await;
                (res.status().is_success(), body_json(res).await)
            };
            let (ok, body) = tokio::time::timeout(timeout, ask)
                .await
                .unwrap_or_else(|_| (false, json!({ "error_code": "remote_timeout" })));
            let mut entry = json!({ "hostId": d.host_id, "name": d.name, "label": d.label(), "ok": ok, "servers": [] });
            if ok {
                entry["servers"] = Value::Array(body["servers"].as_array().cloned().unwrap_or_default());
            } else {
                entry["error_code"] = body.get("error_code").cloned().unwrap_or_else(|| json!("remote_unreachable"));
            }
            entry
        }
    });
    daemons.extend(futures_util::future::join_all(asks).await);
    json!({ "daemons": daemons })
}

pub async fn proxy(State(state): State<IPCServer>, Path((host_id, _)): Path<(String, String)>, req: Request<Body>) -> Response {
    // 풀지 않은 경로 그대로 넘긴다 (원격 데몬이 다시 푼다)
    // `/api/remote/daemons/<hostId>/proxy/<rest>`
    let mut parts = req.uri().path().splitn(7, '/').skip(5);
    let rest = match (parts.next(), parts.next()) {
        (Some("proxy"), Some(rest)) => rest,
        _ => "",
    };
    let Some(path) = peer::forward_path(rest, req.uri().query()) else {
        return peer::error(StatusCode::NOT_FOUND, "not_found");
    };
    peer::forward(&ConfigFile::default_location(), relay_target(&state).await, &host_id, &path, req, state.shutdown_token.clone()).await
}

#[cfg(test)]
mod tests {
    use saba_chan_updater_lib::config_file::ConfigFile;
    use serde_json::{json, Value};

    #[test]
    fn only_daemon_api_paths_are_forwarded() {
        assert_eq!(crate::remote_daemon::peer::forward_path("api/instances", Some("a=1")).as_deref(), Some("/api/instances?a=1"));
        assert_eq!(crate::remote_daemon::peer::forward_path("health", None).as_deref(), Some("/health"));
        // 이름에 `/`가 든 서버는 이름 그대로 넘긴다
        assert_eq!(
            crate::remote_daemon::peer::forward_path("api/server/Survival%201%2F2/stop", None).as_deref(),
            Some("/api/server/Survival%201%2F2/stop")
        );
        for bad in ["remote/pair", "api/../remote/pair", "api/%2e%2e/remote/pair", "api/a%2F..%2Fb", "api%2Fservers", "", "apix", "api"] {
            assert_eq!(crate::remote_daemon::peer::forward_path(bad, None), None, "{bad}");
        }
    }

    /// 합쳐 보기 (G1c): 이 PC 서버와 저장한 원격 데몬마다의 서버. 닿지 않는 데몬은 ok:false와 오류 코드로 남는다
    #[tokio::test]
    async fn every_daemons_servers_come_in_one_list_and_an_unreachable_one_says_why() {
        use crate::remote_daemon::config;
        let tmp = tempfile::tempdir().unwrap();
        let file = ConfigFile::at(tmp.path());
        let supervisor = std::sync::Arc::new(tokio::sync::RwLock::new(crate::supervisor::Supervisor::new_with_instances_dir(
            tmp.path().join("modules").to_str().unwrap(),
            tmp.path().join("instances").to_str().unwrap(),
        )));
        let state = crate::ipc::IPCServer::new(supervisor, "127.0.0.1:0", crate::daemon_log::DaemonLogBuffer::new());

        let all = super::all_servers_with(&file, &state, std::time::Duration::from_secs(5)).await;
        assert_eq!(all["daemons"].as_array().unwrap().len(), 1, "{all}");
        assert_eq!(all["daemons"][0]["hostId"], Value::Null);
        assert_eq!(all["daemons"][0]["ok"], true);
        assert!(all["daemons"][0]["servers"].is_array());

        // 아무도 받지 않는 포트
        let port = std::net::TcpListener::bind("127.0.0.1:0").unwrap().local_addr().unwrap().port();
        config::save_daemon_in(
            &file,
            config::Daemon {
                host_id: "host-b".into(),
                name: "게임 PC".into(),
                owner_name: String::new(),
                address: "127.0.0.1".into(),
                port,
                cert_sha256: "ab".repeat(32),
                token: "sbr_x".into(),
                paired_at: String::new(),
            },
        )
        .unwrap();
        let all = super::all_servers_with(&file, &state, std::time::Duration::from_secs(5)).await;
        let remote = &all["daemons"][1];
        assert_eq!(remote["hostId"], "host-b");
        assert_eq!(remote["name"], "게임 PC");
        assert_eq!(remote["ok"], false);
        assert_eq!(remote["error_code"], "remote_unreachable");
        assert_eq!(remote["servers"], json!([]));
    }
}
