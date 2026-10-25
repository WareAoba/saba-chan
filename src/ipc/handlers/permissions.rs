//! 디스코드 명령 권한 설정 (P1): GUI·CLI가 `nodeSettings[guildId]`의 권한 키를 바꾼다
//!
//! 판정은 `crate::chat::permissions`가 한다. 봇 설정 저장(`PATCH /api/config/bot`)은 이 키들을 바꾸지 않는다.

use super::super::IPCServer;
use crate::chat::permissions::{apply, Change, LevelInput, NodePermissions};
use axum::{
    extract::{Path, State},
    http::StatusCode,
    response::{IntoResponse, Response},
    Json,
};
use serde_json::{json, Map, Value};

fn invalid(code: &str) -> Response {
    (StatusCode::BAD_REQUEST, Json(json!({ "error": format!("Invalid permission change: {}", code), "error_code": code }))).into_response()
}

async fn view(state: &IPCServer, guild_id: &str) -> Value {
    let bot = state.config_store.get_bot_config().await;
    let node = bot.node_settings.get(guild_id);
    NodePermissions::from_node(node).to_view(guild_id, node)
}

/// 바꾸기 여러 건을 한 번에 적용한다 (하나라도 틀리면 아무것도 저장하지 않는다)
async fn change(state: &IPCServer, guild_id: &str, changes: Vec<Change>) -> Response {
    let result = state
        .config_store
        .update_node_settings(guild_id, |node| {
            let mut next = node.clone();
            for c in &changes {
                // 주인 바꾸기가 먼저 오면 그 뒤 항목은 새 주인 기준으로 막는다
                let owner = next.get("ownerDiscordId").and_then(Value::as_str).map(str::to_string);
                if let Err(code) = apply(&mut next, c, owner.as_deref()) {
                    return (Err(code), false);
                }
            }
            let changed = next != *node;
            *node = next;
            (Ok(()), changed)
        })
        .await;
    match result {
        Ok(Ok(())) => Json(view(state, guild_id).await).into_response(),
        Ok(Err(code)) => invalid(code),
        Err(e) => (StatusCode::INTERNAL_SERVER_ERROR, Json(json!({ "error": e.to_string(), "error_code": "save_failed" }))).into_response(),
    }
}

/// GET /api/permissions/:guildId
pub async fn get_permissions(State(state): State<IPCServer>, Path(guild_id): Path<String>) -> Response {
    Json(view(&state, &guild_id).await).into_response()
}

/// `{defaultLevel?, preset?, chatbotCap?, ownerDiscordId?, commandLevels?: {key: level|null}, ladder?: [{id?, name?}]}` → 바꾸기 목록
fn changes_from(body: &Map<String, Value>) -> Result<Vec<Change>, &'static str> {
    let level = |v: &Value| v.as_i64().ok_or("invalid_level");
    let mut out = Vec::new();
    for (key, v) in body {
        match key.as_str() {
            "ownerDiscordId" => out.push(Change::Owner(v.as_str().ok_or("invalid_member")?.to_string())),
            "defaultLevel" => out.push(Change::DefaultLevel(level(v)?)),
            "chatbotCap" => out.push(Change::ChatbotCap(level(v)?)),
            "preset" => out.push(Change::Preset(v.as_str().ok_or("invalid_preset")?.to_string())),
            "ladder" => {
                let levels = v
                    .as_array()
                    .ok_or("invalid_ladder")?
                    .iter()
                    .map(|l| {
                        let id = match l.get("id") {
                            None | Some(Value::Null) => None,
                            Some(id) => Some(id.as_i64().ok_or("invalid_ladder")?),
                        };
                        let name = match l.get("name") {
                            None | Some(Value::Null) => None,
                            Some(n) => Some(n.as_str().ok_or("invalid_ladder")?.to_string()),
                        };
                        Ok(LevelInput { id, name })
                    })
                    .collect::<Result<Vec<_>, &'static str>>()?;
                out.push(Change::Ladder(levels));
            }
            "commandLevels" => {
                for (k, l) in v.as_object().ok_or("invalid_command")? {
                    let level = if l.is_null() { None } else { Some(level(l)?) };
                    out.push(Change::CommandLevel { key: k.clone(), level });
                }
            }
            _ => return Err("unknown_key"),
        }
    }
    // 주인 → 사다리 → 프리셋 → 나머지: 다른 항목이 새 주인·새 단계로 검사되고,
    // 명령별 레벨을 지우는 프리셋이 같은 요청의 명령 레벨을 지우지 않게
    out.sort_by_key(|c| match c {
        Change::Owner(_) => 0,
        Change::Ladder(_) => 1,
        Change::Preset(_) => 2,
        _ => 3,
    });
    Ok(out)
}

/// PATCH /api/permissions/:guildId
pub async fn update_permissions(State(state): State<IPCServer>, Path(guild_id): Path<String>, Json(body): Json<Value>) -> Response {
    let Some(body) = body.as_object() else { return invalid("invalid_body") };
    match changes_from(body) {
        Ok(changes) => change(&state, &guild_id, changes).await,
        Err(code) => invalid(code),
    }
}

/// PUT /api/permissions/:guildId/members/:discordId `{level}`
pub async fn set_member(
    State(state): State<IPCServer>,
    Path((guild_id, discord_id)): Path<(String, String)>,
    Json(body): Json<Value>,
) -> Response {
    let Some(level) = body.get("level").and_then(Value::as_i64) else { return invalid("invalid_level") };
    change(&state, &guild_id, vec![Change::Member { discord_id, level: Some(level) }]).await
}

/// DELETE /api/permissions/:guildId/members/:discordId (기본 레벨로)
pub async fn remove_member(State(state): State<IPCServer>, Path((guild_id, discord_id)): Path<(String, String)>) -> Response {
    change(&state, &guild_id, vec![Change::Member { discord_id, level: None }]).await
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn patch_bodies_put_the_owner_first_and_reject_unknown_keys() {
        let body = json!({ "defaultLevel": 0, "ownerDiscordId": "o", "commandLevels": { "i/kick": 2, "i/op": null } });
        let changes = changes_from(body.as_object().unwrap()).unwrap();
        assert_eq!(changes[0], Change::Owner("o".into()));
        assert!(changes.contains(&Change::CommandLevel { key: "i/op".into(), level: None }));
        assert_eq!(changes_from(json!({ "members": {} }).as_object().unwrap()), Err("unknown_key"));
        assert_eq!(changes_from(json!({ "defaultLevel": "1" }).as_object().unwrap()), Err("invalid_level"));
        // 사다리는 주인 다음, 다른 항목보다 먼저
        let body = json!({ "defaultLevel": 10, "ladder": [{ "id": 3 }, { "name": "VIP" }, { "id": -1 }], "ownerDiscordId": "o" });
        let changes = changes_from(body.as_object().unwrap()).unwrap();
        assert_eq!(changes[1], Change::Ladder(vec![
            LevelInput { id: Some(3), name: None },
            LevelInput { id: None, name: Some("VIP".into()) },
            LevelInput { id: Some(-1), name: None },
        ]));
        assert_eq!(changes_from(json!({ "ladder": [{ "id": "3" }] }).as_object().unwrap()), Err("invalid_ladder"));
        // 프리셋은 명령별 레벨을 지우므로, 같은 요청의 명령 레벨보다 먼저 (키 순서와 무관)
        let body = json!({ "commandLevels": { "i/kick": 3 }, "preset": "strict", "ladder": [{ "id": 3 }, { "id": -1 }, { "id": 1 }] });
        let changes = changes_from(body.as_object().unwrap()).unwrap();
        assert!(matches!(changes[0], Change::Ladder(_)));
        assert_eq!(changes[1], Change::Preset("strict".into()));
        assert_eq!(changes[2], Change::CommandLevel { key: "i/kick".into(), level: Some(3) });
    }
}
