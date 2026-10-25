//! 원격 데몬이 릴레이에 하는 일 (05 §4.4 (j)): 주소 올리기·내리기, 받은 연결표 확인. 노드 토큰으로 서명한다

use crate::ipc::handlers::relay::{relay_client, signed_headers_with};
use serde::Deserialize;
use serde_json::{json, Value};

/// 서명할 릴레이 주소와 노드 토큰
#[derive(Debug, Clone)]
pub struct Target {
    pub base: String,
    pub token: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Redeemed {
    pub from_host_id: String,
    pub from_host_name: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Failure {
    /// 릴레이가 연결표를 받지 않았다 (없음·만료·사용함·남의 것)
    Rejected,
    /// 릴레이에 닿지 못했거나 알 수 없는 답
    Unreachable,
}

async fn send(target: &Target, method: reqwest::Method, path: &str, body: Option<&Value>) -> Result<(u16, Value), Failure> {
    let client = relay_client().map_err(|_| Failure::Unreachable)?;
    let text = body.map(Value::to_string);
    let mut req = client.request(method.clone(), format!("{}{}", target.base, path));
    for (k, v) in signed_headers_with(&target.token, method.as_str(), path, text.as_deref()).ok_or(Failure::Unreachable)? {
        req = req.header(k, v);
    }
    if let Some(text) = text {
        req = req.header("Content-Type", "application/json").body(text);
    }
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

pub async fn publish(target: &Target, address: &str, port: u16, cert_sha256: &str) -> bool {
    let body = json!({ "address": address, "port": port, "certSha256": cert_sha256 });
    matches!(send(target, reqwest::Method::PUT, "/api/remote/endpoint", Some(&body)).await, Ok((204, _)))
}

pub async fn withdraw(target: &Target) -> bool {
    matches!(send(target, reqwest::Method::DELETE, "/api/remote/endpoint", None).await, Ok((204, _)))
}

/// 원격 PC의 직접 연결 주소와 고정할 지문
#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Endpoint {
    pub address: String,
    pub port: u16,
    pub cert_sha256: String,
}

/// 같은 주인의 다른 PC
#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Peer {
    pub host_id: String,
    pub name: String,
    #[serde(default)]
    pub status: String,
    pub endpoint: Option<Endpoint>,
}

pub async fn peers(target: &Target) -> Result<Vec<Peer>, Failure> {
    #[derive(Deserialize)]
    struct Peers {
        peers: Vec<Peer>,
    }
    match send(target, reqwest::Method::GET, "/api/remote/peers", None).await? {
        (200, data) => serde_json::from_value::<Peers>(data).map(|p| p.peers).map_err(|_| Failure::Unreachable),
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
    #[serde(flatten)]
    pub endpoint: Endpoint,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TicketFailure {
    /// 같은 주인의 다른 PC가 아니다 (없음·남의 것·자기 자신)
    NotFound,
    /// 그 PC가 주소를 올리지 않았다 (원격 포트가 꺼졌거나 공인 주소가 없다)
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
