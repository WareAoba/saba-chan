//! 원격 데몬이 릴레이에 하는 일 (05 §4.4 (j)): 주소 올리기·내리기, 받은 연결표 확인. 노드 토큰으로 서명한다

use crate::ipc::handlers::relay::{relay_client, signed_request};
use serde::Deserialize;
use serde_json::{json, Value};

/// 서명할 릴레이 주소와 노드 토큰
#[derive(Debug, Clone)]
pub struct Target {
    pub base: String,
    pub token: String,
}

impl Target {
    /// 노드 토큰을 실어도 되는 주소인가: https 또는 루프백 http(모의 릴레이)만.
    /// 저장할 때 막는 규칙(`relay_url_problem`)을 예전에 저장된 값에도 적용한다
    pub fn sendable(&self) -> bool {
        crate::config_store::relay_url_problem(&self.base).is_none() && !self.base.trim().is_empty()
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Redeemed {
    pub from_host_id: String,
    pub from_host_name: String,
    /// 그 PC 주인의 디스코드 이름 (모르면 빈 값)
    #[serde(default)]
    pub from_owner_name: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Failure {
    /// 릴레이가 연결표를 받지 않았다 (없음·만료·사용함·남의 것)
    Rejected,
    /// 릴레이에 닿지 못했거나 알 수 없는 답
    Unreachable,
}

async fn send(target: &Target, method: reqwest::Method, path: &str, body: Option<&Value>) -> Result<(u16, Value), Failure> {
    if !target.sendable() {
        tracing::warn!("Not sending the node token to relay {}: it must be https (http only for localhost)", target.base);
        return Err(Failure::Unreachable);
    }
    let client = relay_client().map_err(|_| Failure::Unreachable)?;
    let text = body.map(Value::to_string);
    let req = signed_request(&client, method, &target.base, path, &target.token, text).ok_or(Failure::Unreachable)?;
    let res = req.send().await.map_err(|_| Failure::Unreachable)?;
    let status = res.status().as_u16();
    let data = res.json::<Value>().await.unwrap_or(Value::Null);
    Ok((status, data))
}

/// 릴레이 연결표 id: nanoid 문자만 (경로에 그대로 넣는다)
pub fn valid_ticket_id(id: &str) -> bool {
    (1..=64).contains(&id.len()) && id.bytes().all(|b| b.is_ascii_alphanumeric() || b == b'_' || b == b'-')
}

pub async fn redeem(target: &Target, ticket_id: &str, secret: &str) -> Result<Redeemed, Failure> {
    if !valid_ticket_id(ticket_id) {
        return Err(Failure::Rejected);
    }
    let path = format!("/api/remote/tickets/{ticket_id}/redeem");
    match send(target, reqwest::Method::POST, &path, Some(&json!({ "secret": secret }))).await? {
        (200, data) => serde_json::from_value(data).map_err(|_| Failure::Unreachable),
        (400 | 404 | 410, _) => Err(Failure::Rejected),
        _ => Err(Failure::Unreachable),
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Published {
    Done,
    /// 릴레이가 본문을 받지 않는다 (400): 같은 본문으로는 다시 보내지 않는다
    Refused,
    /// 릴레이가 이 노드 토큰을 받지 않는다 (401·403: 토큰이 대체되거나 지워짐): 같은 토큰으로는 다시 보내지 않는다
    Unauthorized,
    Failed,
}

/// 주소가 없으면(`None`) 터널로만 붙는 PC로 올린다 (T1)
pub async fn publish(target: &Target, address: Option<&str>, port: u16, cert_sha256: &str) -> Published {
    let body = json!({ "address": address, "port": port, "certSha256": cert_sha256 });
    match send(target, reqwest::Method::PUT, "/api/remote/endpoint", Some(&body)).await {
        Ok((204, _)) => Published::Done,
        Ok((400, _)) => Published::Refused,
        Ok((401 | 403, _)) => Published::Unauthorized,
        _ => Published::Failed,
    }
}

pub async fn withdraw(target: &Target) -> bool {
    matches!(send(target, reqwest::Method::DELETE, "/api/remote/endpoint", None).await, Ok((204, _)))
}

/// 원격 PC의 직접 연결 주소와 고정할 지문. 주소가 비었으면(릴레이의 `null`) 터널로만 붙는다
#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Endpoint {
    #[serde(default, deserialize_with = "null_as_empty")]
    pub address: String,
    pub port: u16,
    pub cert_sha256: String,
}

fn null_as_empty<'de, D: serde::Deserializer<'de>>(d: D) -> Result<String, D::Error> {
    Ok(Option::<String>::deserialize(d)?.unwrap_or_default())
}

/// 같은 주인의 다른 PC
#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Peer {
    pub host_id: String,
    pub name: String,
    #[serde(default)]
    pub owner_name: String,
    #[serde(default)]
    pub status: String,
    pub endpoint: Option<Endpoint>,
    /// OS·CPU·배포판 `{os, distro:{id, version, name}}` (heartbeat). 아직 heartbeat 전이면 없다
    #[serde(default)]
    pub platform: Option<Value>,
    #[serde(default)]
    pub agent_version: Option<String>,
    /// 서버쨩에 처음 연결한 때·마지막 heartbeat (ISO)
    #[serde(default)]
    pub created_at: Option<String>,
    #[serde(default)]
    pub last_seen_at: Option<String>,
    /// 그 PC가 서버쨩에 접속해 온 공인 IP (릴레이는 메모리에만 둔다)
    #[serde(default)]
    pub public_ip: Option<String>,
}

/// 이 PC (릴레이 내 PC 목록의 `self`, 옛 릴레이는 주지 않는다)
#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ThisPc {
    pub host_id: String,
    pub name: String,
}

/// 내 PC 목록: 이 PC와 같은 주인의 다른 PC
#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
pub struct PeerList {
    #[serde(default, rename = "self")]
    pub this_pc: Option<ThisPc>,
    pub peers: Vec<Peer>,
}

pub async fn peer_list(target: &Target) -> Result<PeerList, Failure> {
    match send(target, reqwest::Method::GET, "/api/remote/peers", None).await? {
        (200, data) => serde_json::from_value::<PeerList>(data).map_err(|_| Failure::Unreachable),
        _ => Err(Failure::Unreachable),
    }
}

pub async fn peers(target: &Target) -> Result<Vec<Peer>, Failure> {
    peer_list(target).await.map(|l| l.peers)
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RenameFailure {
    /// 같은 주인의 PC가 아니다 (없음·남의 것)
    NotFound,
    /// 정리하면 빈 이름
    Empty,
    Unreachable,
}

/// 같은 주인의 PC 하나(이 PC 포함)의 이름을 바꾼다 (디스코드 `/사바쨩 내pc`와 같은 값). 릴레이가 정리해 남긴 이름을 준다
pub async fn rename(target: &Target, host_id: &str, name: &str) -> Result<String, RenameFailure> {
    if !valid_ticket_id(host_id) {
        return Err(RenameFailure::NotFound);
    }
    match send(target, reqwest::Method::PUT, &format!("/api/remote/hosts/{host_id}/name"), Some(&json!({ "name": name }))).await {
        Ok((200, data)) => data["name"].as_str().map(str::to_string).ok_or(RenameFailure::Unreachable),
        Ok((400, _)) => Err(RenameFailure::Empty),
        Ok((404, _)) => Err(RenameFailure::NotFound),
        _ => Err(RenameFailure::Unreachable),
    }
}

/// 주인의 가이드 진행도 (내 PC 모두가 함께 본다). 옛 릴레이(404)·끊김은 `Unreachable`
pub async fn guide_state(target: &Target) -> Result<crate::guides::State, Failure> {
    match send(target, reqwest::Method::GET, "/api/remote/guides", None).await? {
        (200, data) => serde_json::from_value(data["state"].clone()).map_err(|_| Failure::Unreachable),
        _ => Err(Failure::Unreachable),
    }
}

/// 바뀐 진행도(`config.json guides` 칸 모양)를 보내 합치고, 합친 진행도를 받는다
pub async fn merge_guide_state(target: &Target, patch: &Value) -> Result<crate::guides::State, Failure> {
    match send(target, reqwest::Method::POST, "/api/remote/guides", Some(patch)).await? {
        (200, data) => serde_json::from_value(data["state"].clone()).map_err(|_| Failure::Unreachable),
        _ => Err(Failure::Unreachable),
    }
}

/// 페어링 코드를 디스코드 대신 받은 결과: 같은 주인의 새 PC
#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Claimed {
    pub host_id: String,
    #[serde(default)]
    pub name: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ClaimFailure {
    /// 없는 코드
    NotFound,
    /// 이미 누가 받은 코드
    AlreadyClaimed,
    Expired,
    Unreachable,
}

/// 다른 PC(새로 설치한 리눅스 등)가 낸 페어링 코드를 이 PC 주인의 것으로 받는다 (디스코드 `/사바쨩 연결`과 같다)
pub async fn claim(target: &Target, code: &str) -> Result<Claimed, ClaimFailure> {
    match send(target, reqwest::Method::POST, "/api/remote/pair-claim", Some(&json!({ "code": code }))).await {
        Ok((200, data)) => serde_json::from_value(data).map_err(|_| ClaimFailure::Unreachable),
        Ok((400 | 404, _)) => Err(ClaimFailure::NotFound),
        Ok((409, _)) => Err(ClaimFailure::AlreadyClaimed),
        Ok((410, _)) => Err(ClaimFailure::Expired),
        _ => Err(ClaimFailure::Unreachable),
    }
}

/// 같은 주인의 다른 PC 하나를 서버쨩에서 뺀다. 빠졌거나 이미 없으면 true
pub async fn remove_peer(target: &Target, host_id: &str) -> Result<bool, Failure> {
    if !valid_ticket_id(host_id) {
        return Ok(false);
    }
    match send(target, reqwest::Method::DELETE, &format!("/api/remote/peers/{host_id}"), None).await? {
        (204, _) => Ok(true),
        (404, _) => Ok(false),
        _ => Err(Failure::Unreachable),
    }
}

/// 원격 PC 앞으로 받은 연결표
#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Ticket {
    pub ticket_id: String,
    pub secret: String,
    pub name: String,
    #[serde(default)]
    pub owner_name: String,
    #[serde(flatten)]
    pub endpoint: Endpoint,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TicketFailure {
    /// 같은 주인의 다른 PC가 아니다 (없음·남의 것·자기 자신)
    NotFound,
    /// 그 PC가 원격 포트를 올리지 않았다 (꺼져 있다)
    NoEndpoint,
    Unreachable,
}

pub async fn ticket(target: &Target, to_host_id: &str) -> Result<Ticket, TicketFailure> {
    match send(target, reqwest::Method::POST, "/api/remote/tickets", Some(&json!({ "toHostId": to_host_id }))).await {
        Ok((201, data)) => serde_json::from_value(data).map_err(|_| TicketFailure::Unreachable),
        Ok((404, _)) => Err(TicketFailure::NotFound),
        Ok((409, _)) => Err(TicketFailure::NoEndpoint),
        _ => Err(TicketFailure::Unreachable),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use axum::{extract::Path, http::StatusCode, routing::get, Json, Router};
    use saba_chan_updater_lib::config_file::ConfigFile;

    /// 릴레이 흉내: 내 PC 목록(이 PC 이름 포함), 이름 바꾸기, 가이드 진행도
    async fn relay() -> Target {
        let app = Router::new()
            .route(
                "/api/remote/peers",
                get(|| async { Json(json!({ "self": { "hostId": "host-a", "name": "작업용" }, "peers": [{ "hostId": "host-b", "name": "맥북", "ownerName": "kim" }] })) }),
            )
            .route(
                "/api/remote/hosts/{id}/name",
                axum::routing::put(|Path(id): Path<String>, Json(b): Json<Value>| async move {
                    let name = b["name"].as_str().unwrap_or_default().trim().to_string();
                    match (id.as_str(), name.is_empty()) {
                        (_, true) => (StatusCode::BAD_REQUEST, Json(json!({ "error": "EMPTY_NAME" }))),
                        ("host-b", _) => (StatusCode::OK, Json(json!({ "hostId": id, "name": name }))),
                        _ => (StatusCode::NOT_FOUND, Json(json!({ "error": "NOT_FOUND" }))),
                    }
                }),
            )
            .route(
                "/api/remote/guides",
                get(|| async { Json(json!({ "state": { "known": null, "finished": ["music"], "achieved": [], "step": {} } })) }).post(
                    |Json(b): Json<Value>| async move {
                        Json(json!({ "state": { "known": b["known"], "finished": ["music"], "achieved": b["achieved"].as_array().cloned().unwrap_or_default(), "step": {} } }))
                    },
                ),
            );
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let base = format!("http://{}", listener.local_addr().unwrap());
        tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });
        Target { base, token: "sbn_host-a.secret".into() }
    }

    /// PC 이름은 어느 PC에서든 바꾼다 (사용자 11-12). 이 PC 이름도 내 PC 목록에서 받아 둔다
    #[tokio::test]
    async fn any_of_my_pcs_can_be_renamed_and_this_pcs_name_is_remembered() {
        let target = relay().await;
        assert_eq!(rename(&target, "host-b", " 맥북 프로 ").await, Ok("맥북 프로".into()));
        assert_eq!(rename(&target, "host-b", "  ").await, Err(RenameFailure::Empty));
        assert_eq!(rename(&target, "host-x", "x").await, Err(RenameFailure::NotFound));
        assert_eq!(rename(&target, "../x", "x").await, Err(RenameFailure::NotFound));

        let dir = tempfile::tempdir().unwrap();
        let file = ConfigFile::at(dir.path());
        let list = crate::remote_daemon::RemoteDaemon::default().sync_with_relay(&file, &target).await.unwrap();
        assert_eq!(list.peers.len(), 1);
        assert_eq!(crate::remote_daemon::config::this_name_in(&file), "작업용");
    }

    #[tokio::test]
    async fn guide_progress_is_read_and_merged_on_the_relay() {
        let target = relay().await;
        assert_eq!(guide_state(&target).await.unwrap().finished, vec!["music".to_string()]);
        let merged = merge_guide_state(&target, &json!({ "achieved": ["backup"] })).await.unwrap();
        assert_eq!(merged.achieved, vec!["backup".to_string()]);
    }

    /// 노드 토큰은 https(또는 루프백 http 모의 릴레이)로만 보낸다: 예전에 저장된 평문 http 주소에는 보내지 않는다
    #[tokio::test]
    async fn the_node_token_is_not_sent_to_a_plain_http_relay() {
        // 시험은 루프백에만 묶는다 (tests/test_binds.rs). 127.0.0.2를 못 묶는 OS(macOS)는 127.0.0.1로 받는다: 그래도 127.0.0.2로 간 요청은 닿지 않는다
        let listener = std::net::TcpListener::bind("127.0.0.2:0").or_else(|_| std::net::TcpListener::bind("127.0.0.1:0")).unwrap();
        listener.set_nonblocking(true).unwrap();
        let port = listener.local_addr().unwrap().port();
        // 127.0.0.2도 이 PC지만 허용한 루프백 이름이 아니다: 바깥 평문 주소와 같게 다룬다
        let target = Target { base: format!("http://127.0.0.2:{port}"), token: "sbn_host-a.secret".into() };
        assert!(!target.sendable());
        assert_eq!(peers(&target).await, Err(Failure::Unreachable));
        assert!(listener.accept().is_err(), "the plain-http relay was contacted");
        assert!(Target { base: "https://saba-chan.online".into(), token: String::new() }.sendable());
        assert!(Target { base: "http://127.0.0.1:9".into(), token: String::new() }.sendable());
    }
}
