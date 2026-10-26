//! 켜짐 알림: 길드마다 고른 서버가 사바쨩을 거쳐 켜지면 그 길드의 알림 채널에 알린다 (봇 설정 `nodeSettings[guild].startNotice`).
//!
//! - `GET /api/guilds/:guild/start-notice` → `{guildId, instances, allowedInstances}` (`allowedInstances`가 null이면 모든 서버를 켤 수 있다)
//! - `PUT /api/instance/:id/start-notice/:guild` `{enabled}` → `{success, id, guildId, enabled, instances}`.
//!   액션 `instance.start_notice.set`(디스코드 `/<서버> 켜짐-알림`)도 이 라우트를 부른다.
//!
//! 기본은 아무 서버도 알리지 않는다. 길드가 쓸 서버를 정해 두었으면(`allowedInstances`) 그 서버만 켤 수 있다.
//! 무엇을 알릴지(디스코드·말하는 사바쨩이 켠 것, 사바쨩 밖에서 켜진 것은 빼고)는 릴레이가 이벤트 출처로 정한다.

use axum::{
    extract::{Path, State},
    http::StatusCode,
    response::{IntoResponse, Response},
    Json,
};
use serde_json::{json, Value};

use super::super::IPCServer;

pub const KEY: &str = "startNotice";

fn ids(node: Option<&Value>, key: &str) -> Option<Vec<String>> {
    Some(node?.get(key)?.as_array()?.iter().filter_map(|v| v.as_str().map(str::to_string)).collect())
}

/// `GET /api/guilds/:guild/start-notice`
pub async fn get(State(state): State<IPCServer>, Path(guild): Path<String>) -> Response {
    let bot = state.config_store.get_bot_config().await;
    let node = bot.node_settings.get(&guild);
    let instances = ids(node, KEY).unwrap_or_default();
    Json(json!({ "guildId": guild, "instances": instances, "allowedInstances": ids(node, "allowedInstances") })).into_response()
}

/// `PUT /api/instance/:id/start-notice/:guild` `{enabled}`
pub async fn put(State(state): State<IPCServer>, Path((id, guild)): Path<(String, String)>, Json(body): Json<Value>) -> Response {
    let Some(enabled) = body.get("enabled").and_then(Value::as_bool) else {
        return invalid("enabled", "invalid");
    };
    if state.supervisor.read().await.instance_store.get(&id).is_none() {
        return (StatusCode::NOT_FOUND, Json(json!({ "success": false, "error": format!("Instance not found: {}", id), "error_code": "not_found" }))).into_response();
    }
    // 길드가 쓸 서버를 고르는 것과 켜짐 알림을 켜는 것을 한 잠금 안에서 본다
    let saved = state
        .config_store
        .update_node_settings(&guild, |node| {
            let node_value = Value::Object(node.clone());
            if enabled && ids(Some(&node_value), "allowedInstances").is_some_and(|allowed| !allowed.contains(&id)) {
                return (Err(()), false);
            }
            let mut list = ids(Some(&node_value), KEY).unwrap_or_default();
            let before = list.clone();
            list.retain(|x| x != &id);
            if enabled {
                list.push(id.clone());
            }
            if list == before {
                return (Ok(list), false);
            }
            if list.is_empty() {
                node.remove(KEY);
            } else {
                node.insert(KEY.into(), json!(list));
            }
            (Ok(list), true)
        })
        .await;
    match saved {
        Ok(Ok(list)) => Json(json!({ "success": true, "id": id, "guildId": guild, "enabled": enabled, "instances": list })).into_response(),
        Ok(Err(())) => invalid("instance", "not_allowed"),
        Err(e) => (StatusCode::INTERNAL_SERVER_ERROR, Json(json!({ "success": false, "error": e.to_string(), "error_code": "save_failed" }))).into_response(),
    }
}

fn invalid(field: &str, reason: &str) -> Response {
    (StatusCode::BAD_REQUEST, Json(json!({ "success": false, "error": "invalid start notice", "error_code": "validation_failed", "details": [{ "field": field, "reason": reason }] }))).into_response()
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Arc;

    // 사용자 요청(2026-10-26): 켜짐 알림은 인스턴스마다 길드마다 정하고, 기본은 알리지 않으며, 길드가 쓰기로 한 서버만 켤 수 있다
    #[tokio::test]
    async fn start_notice_is_off_by_default_and_set_per_guild_and_instance() {
        let tmp = tempfile::tempdir().unwrap();
        let modules = tmp.path().join("modules");
        std::fs::create_dir_all(&modules).unwrap();
        let mut sup = crate::supervisor::Supervisor::new_with_instances_dir(modules.to_str().unwrap(), tmp.path().join("instances").to_str().unwrap());
        sup.initialize().await.unwrap();
        let (a, b) = (crate::instance::ServerInstance::new("survival", "plain"), crate::instance::ServerInstance::new("creative", "plain"));
        let (ia, ib) = (a.id.clone(), b.id.clone());
        sup.instance_store.add(a).unwrap();
        sup.instance_store.add(b).unwrap();
        let state = IPCServer::new(Arc::new(tokio::sync::RwLock::new(sup)), "127.0.0.1:0", crate::daemon_log::DaemonLogBuffer::new());
        let (g1, g2) = ("start-notice-g1", "start-notice-g2");
        let set = |id: String, guild: &'static str, body: Value| {
            let state = state.clone();
            async move { crate::ipc::internal::into_internal(put(State(state), Path((id, guild.to_string())), Json(body)).await).await }
        };
        let shown = |guild: &'static str| {
            let state = state.clone();
            async move { crate::ipc::internal::into_internal(get(State(state), Path(guild.to_string())).await).await.body["instances"].clone() }
        };

        assert_eq!(shown(g1).await, json!([]));
        let r = set(ia.clone(), g1, json!({ "enabled": true })).await;
        assert_eq!(r.status, StatusCode::OK);
        assert_eq!(r.body["instances"], json!([ia]));
        assert_eq!(shown(g1).await, json!([ia]));
        assert_eq!(shown(g2).await, json!([]), "another guild stays off");

        // 길드가 쓸 서버를 정해 두었으면 그 밖의 서버는 켤 수 없다. 끄기는 된다
        state
            .config_store
            .update_node_settings(g1, |n| {
                n.insert("allowedInstances".into(), json!([ia]));
                ((), true)
            })
            .await
            .unwrap();
        let state2 = state.clone();
        let view = crate::ipc::internal::into_internal(get(State(state2), Path(g1.to_string())).await).await.body;
        assert_eq!(view["allowedInstances"], json!([ia]), "the screens show only servers this guild can use");
        let r = set(ib.clone(), g1, json!({ "enabled": true })).await;
        assert_eq!(r.status, StatusCode::BAD_REQUEST);
        assert_eq!(r.body["details"][0]["reason"], "not_allowed");
        assert_eq!(set(ib.clone(), g1, json!({ "enabled": false })).await.status, StatusCode::OK);

        assert_eq!(set(ia.clone(), g1, json!({ "enabled": false })).await.body["instances"], json!([]));
        assert!(state.config_store.get_bot_config().await.node_settings[g1].get(KEY).is_none(), "an empty list drops the key");

        assert_eq!(set("nope".into(), g1, json!({ "enabled": true })).await.status, StatusCode::NOT_FOUND);
        assert_eq!(set(ia.clone(), g1, json!({ "enabled": "yes" })).await.status, StatusCode::BAD_REQUEST);

        // 봇 설정 일괄 저장은 이 키를 바꾸지 않는다
        set(ia.clone(), g1, json!({ "enabled": true })).await;
        state.config_store.set_bot_config_from_json(json!({ "nodeSettings": { g1: { "startNotice": [] } } })).await.unwrap();
        assert_eq!(shown(g1).await, json!([ia]));
    }
}
