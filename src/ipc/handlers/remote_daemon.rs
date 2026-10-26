//! 원격 데몬 설정과 짝 맺은 PC (D1, `crate::remote_daemon`). 로컬 전용: 원격 포트에서는 막는다
//!
//!   GET    /api/remote/settings            {enabled, port, address, transport, status}
//!   PUT    /api/remote/settings            {enabled?, port?, address?} → 저장하고 원격 포트를 다시 맞춘다
//!   GET    /api/remote/clients             {clients:[{hostId, name, pairedAt}]}
//!   DELETE /api/remote/clients/:hostId     짝을 푼다 (그 PC의 토큰이 더는 통하지 않는다)
//!
//! 붙는 쪽 (D1b, `remote_daemon::peer`):
//!   GET    /api/remote/peers                         {peers:[{hostId, name, status, hasEndpoint, saved}]} 같은 주인의 다른 PC (릴레이)
//!   GET    /api/remote/daemons                       {daemons:[{hostId, name, address, port, pairedAt}]} 이 PC가 붙는 원격 데몬
//!   POST   /api/remote/daemons {hostId}              짝을 맺고 저장한다 → {hostId, name}
//!   DELETE /api/remote/daemons/:hostId               잊는다
//!   ANY    /api/remote/daemons/:hostId/proxy/*path   원격 데몬의 `/path`(`api/…`·`health`)에 그대로 보낸다
//!   GET    /api/remote/servers                       {daemons:[{hostId, name, ok, error_code?, servers}]} 이 PC(hostId null)와 원격 데몬마다의 서버 (G1c 합쳐 보기)

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

pub async fn get_settings() -> Json<Value> {
    let settings = config::settings_in(&ConfigFile::default_location());
    Json(view(&settings, &remote_daemon::status().await))
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
        .map(|c| json!({ "hostId": c.host_id, "name": c.name, "pairedAt": c.paired_at }))
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
    let Ok(peers) = remote_daemon::relay::peers(&target).await else {
        return peer::error(StatusCode::BAD_GATEWAY, "relay_unreachable");
    };
    let saved = config::daemons_in(&ConfigFile::default_location());
    let peers: Vec<Value> = peers
        .into_iter()
        .map(|p| {
            let is_saved = saved.iter().any(|d| d.host_id == p.host_id);
            json!({ "hostId": p.host_id, "name": p.name, "status": p.status, "hasEndpoint": p.endpoint.is_some(), "saved": is_saved })
        })
        .collect();
    Json(json!({ "peers": peers })).into_response()
}

pub async fn list_daemons() -> Json<Value> {
    let daemons: Vec<Value> = config::daemons_in(&ConfigFile::default_location())
        .into_iter()
        .map(|d| json!({ "hostId": d.host_id, "name": d.name, "address": d.address, "port": d.port, "pairedAt": d.paired_at }))
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
        Ok(d) => Json(json!({ "hostId": d.host_id, "name": d.name })).into_response(),
        Err(e) => peer::error(e.status(), e.code()),
    }
}

pub async fn forget_daemon(Path(host_id): Path<String>) -> Response {
    match config::forget_daemon_in(&ConfigFile::default_location(), &host_id) {
        Ok(true) => {
            crate::remote_daemon::tunnel::close(&host_id);
            StatusCode::NO_CONTENT.into_response()
        }
        Ok(false) => peer::error(StatusCode::NOT_FOUND, "remote_unknown"),
        Err(_) => peer::error(StatusCode::INTERNAL_SERVER_ERROR, "save_failed"),
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
    let mut daemons = vec![json!({ "hostId": null, "name": "", "ok": true, "servers": local["servers"].as_array().cloned().unwrap_or_default() })];
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
            let mut entry = json!({ "hostId": d.host_id, "name": d.name, "ok": ok, "servers": [] });
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
        for bad in ["remote/pair", "api/../remote/pair", "api/%2e%2e/remote/pair", "api/a%2Fb", "", "apix"] {
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
