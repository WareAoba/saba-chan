//! 길드·인스턴스별 디스코드 낱말 API (`nodeSettings[guild].instances`)
//!
//! `GET /api/guilds/:guild/instances`, `PUT /api/guilds/:guild/instances/:id/words`,
//! 길드가 쓸 서버 목록 `PUT /api/guilds/:guild/allowed-instances`.
//! 로컬 클라이언트(GUI·CLI)용이다. 릴레이 에이전트는 이 라우트를 부르지 않고, 액션 카탈로그에도 없다.

use axum::{
    extract::{Path, State},
    http::StatusCode,
    response::{IntoResponse, Response},
    Json,
};
use serde_json::{json, Value};
use std::collections::HashMap;

use crate::chat::api::{DaemonApi, ModuleView, ServerView};
use crate::chat::aliases::BUILTIN_COMMANDS;
use crate::chat::words::{self, InstanceWords, INSTANCES_KEY};
use super::super::IPCServer;

/// 길드가 쓰는 서버: `allowedInstances`가 있으면 그것만, 없으면 전부
fn guild_servers(node_settings: &Value, guild: &str, servers: &[ServerView]) -> Vec<ServerView> {
    let allowed: Option<Vec<&str>> = node_settings
        .get(guild)
        .and_then(|n| n.get("allowedInstances"))
        .and_then(Value::as_array)
        .map(|a| a.iter().filter_map(Value::as_str).collect());
    servers.iter().filter(|s| allowed.as_ref().is_none_or(|a| a.contains(&s.id.as_str()))).cloned().collect()
}

fn command_rows(module: Option<&ModuleView>, mine: &InstanceWords) -> Vec<Value> {
    // 모듈이 내장과 같은 이름을 정의하면 한 줄로: 이름표·위험도는 모듈 선언을 쓴다
    let defines = |name: &str| module.is_some_and(|m| m.commands.iter().any(|c| c.name.eq_ignore_ascii_case(name)));
    let builtin = BUILTIN_COMMANDS
        .iter()
        .filter(|c| !defines(c))
        .map(|c| (c.to_string(), String::new(), if *c == "status" { "safe" } else { "caution" }.to_string(), true));
    let defined = module.into_iter().flat_map(|m| {
        m.commands
            .iter()
            .map(|c| (c.name.clone(), c.label.clone(), c.risk.clone().unwrap_or_else(|| "caution".into()), BUILTIN_COMMANDS.iter().any(|b| b.eq_ignore_ascii_case(&c.name))))
    });
    builtin
        .chain(defined)
        .map(|(name, label, risk, builtin)| {
            json!({
                "name": name,
                "label": label,
                "risk": risk,
                "builtin": builtin,
                "moduleAliases": module.map(|m| m.aliases_of(&name).to_vec()).unwrap_or_default(),
                "aliases": mine.commands.get(&name).cloned().unwrap_or_default(),
            })
        })
        .collect()
}

/// `GET /api/guilds/:guild/instances`
pub async fn list(State(state): State<IPCServer>, Path(guild): Path<String>) -> Response {
    let servers = match state.servers().await {
        Ok(s) => s,
        Err(e) => return (StatusCode::INTERNAL_SERVER_ERROR, Json(json!({ "success": false, "error": e }))).into_response(),
    };
    let modules = state.modules().await;
    let bot = state.config_store.get_bot_config().await;
    let table = words::of_guild(&bot.node_settings, &guild);
    let rows: Vec<Value> = guild_servers(&bot.node_settings, &guild, &servers)
        .iter()
        .map(|s| {
            let mine = table.get(&s.id).cloned().unwrap_or_default();
            let module = modules.iter().find(|m| m.name == s.module);
            json!({ "id": s.id, "name": s.name, "module": s.module, "words": mine.words, "commands": command_rows(module, &mine) })
        })
        .collect();
    Json(json!({ "guildId": guild, "instances": rows })).into_response()
}

/// `PUT /api/guilds/:guild/instances/:id/words` `{words, commands}`
pub async fn put(State(state): State<IPCServer>, Path((guild, id)): Path<(String, String)>, Json(body): Json<Value>) -> Response {
    let Ok(mut want) = serde_json::from_value::<InstanceWords>(body) else {
        return validation(vec![json!({ "field": "input", "reason": "invalid" })]);
    };
    want.words = want.words.iter().map(|w| w.trim().to_string()).collect();
    for list in want.commands.values_mut() {
        *list = list.iter().map(|w| w.trim().to_string()).collect();
    }
    want.commands.retain(|_, l| !l.is_empty());

    let servers = match state.servers().await {
        Ok(s) => s,
        Err(e) => return (StatusCode::INTERNAL_SERVER_ERROR, Json(json!({ "success": false, "error": e }))).into_response(),
    };
    let modules = state.modules().await;
    let value = serde_json::to_value(&want).unwrap_or_default();
    // 검사와 저장을 설정 잠금 안에서 한다: 동시에 온 두 요청이 같은 낱말을 둘 다 통과시키지 않게
    let saved = state
        .config_store
        .update_node_settings(&guild, |node| {
            let node_settings = json!({ guild.as_str(): Value::Object(node.clone()) });
            let in_guild = guild_servers(&node_settings, &guild, &servers);
            let Some(instance) = in_guild.iter().find(|s| s.id == id) else {
                return (Err(None), false);
            };
            let module = modules.iter().find(|m| m.name == instance.module);
            let others: HashMap<String, InstanceWords> = words::of_guild(&node_settings, &guild).into_iter().filter(|(k, _)| k != &id).collect();
            let problems = words::validate(instance, module, &want, &in_guild, &modules, &others);
            if !problems.is_empty() {
                return (Err(Some(problems)), false);
            }
            let table = node.entry(INSTANCES_KEY).or_insert_with(|| json!({}));
            if !table.is_object() {
                *table = json!({});
            }
            let table = table.as_object_mut().expect("object");
            let changed = if want.is_empty() { table.remove(&id).is_some() } else { table.insert(id.clone(), value.clone()).as_ref() != Some(&value) };
            if table.is_empty() {
                node.remove(INSTANCES_KEY);
            }
            (Ok(()), changed)
        })
        .await;
    let saved = match saved {
        Ok(Err(None)) => {
            return (StatusCode::NOT_FOUND, Json(json!({ "success": false, "error": "instance not used in this guild", "error_code": "not_found" }))).into_response();
        }
        Ok(Err(Some(problems))) => return validation(problems.into_iter().map(|p| serde_json::to_value(p).unwrap_or_default()).collect()),
        Ok(Ok(())) => Ok(()),
        Err(e) => Err(e),
    };
    match saved {
        Ok(()) => Json(json!({ "success": true, "id": id, "words": want.words, "commands": want.commands })).into_response(),
        Err(e) => (StatusCode::INTERNAL_SERVER_ERROR, Json(json!({ "success": false, "error": e.to_string(), "error_code": "save_failed" }))).into_response(),
    }
}

/// `PUT /api/guilds/:guild/allowed-instances` `{instances: [id] | null}`. null은 제한 없음(키 삭제), 빈 배열은 아무 서버도 못 쓴다.
/// bot-config 일괄 저장은 이 키를 바꾸지 않는다 (`PERMISSION_KEYS`)
pub async fn put_allowed(State(state): State<IPCServer>, Path(guild): Path<String>, Json(body): Json<Value>) -> Response {
    let want: Option<Vec<String>> = match body.as_object().and_then(|o| o.get("instances")) {
        Some(Value::Null) => None,
        Some(Value::Array(list)) if list.iter().all(Value::is_string) => {
            let mut ids: Vec<String> = Vec::new();
            for id in list.iter().filter_map(Value::as_str) {
                if !ids.iter().any(|x| x == id) {
                    ids.push(id.to_string());
                }
            }
            Some(ids)
        }
        _ => return validation(vec![json!({ "field": "instances", "reason": "invalid" })]),
    };
    if let Some(ids) = &want {
        let supervisor = state.supervisor.read().await;
        let unknown: Vec<Value> = ids
            .iter()
            .filter(|id| supervisor.instance_store.get(id).is_none())
            .map(|id| json!({ "field": "instances", "reason": "unknown_instance", "id": id }))
            .collect();
        if !unknown.is_empty() {
            return validation(unknown);
        }
    }
    let value = want.as_ref().map(|ids| json!(ids));
    let saved = state
        .config_store
        .update_node_settings(&guild, |node| {
            let changed = match &value {
                None => node.remove("allowedInstances").is_some(),
                Some(v) => node.insert("allowedInstances".into(), v.clone()).as_ref() != Some(v),
            };
            ((), changed)
        })
        .await;
    match saved {
        Ok(()) => Json(json!({ "guildId": guild, "allowedInstances": want })).into_response(),
        Err(e) => (StatusCode::INTERNAL_SERVER_ERROR, Json(json!({ "success": false, "error": e.to_string(), "error_code": "save_failed" }))).into_response(),
    }
}

fn validation(details: Vec<Value>) -> Response {
    (StatusCode::BAD_REQUEST, Json(json!({ "success": false, "error": "invalid words", "error_code": "validation_failed", "details": details }))).into_response()
}

/// 시작할 때 한 번: 예전 전역 별명을 길드·인스턴스 낱말로 옮긴다 (길드를 아직 모르면 다음 시작에 다시)
pub async fn migrate_on_start(state: &IPCServer) {
    let bot = state.config_store.get_bot_config().await;
    if bot.module_aliases.is_empty() && bot.command_aliases.is_empty() && bot.instance_aliases.is_empty() {
        return;
    }
    let mut guilds: Vec<String> = bot.node_settings.as_object().map(|m| m.keys().cloned().collect()).unwrap_or_default();
    for g in bot.cloud_nodes.iter().filter_map(|n| n.get("guildId").and_then(Value::as_str)) {
        if !guilds.iter().any(|x| x == g) {
            guilds.push(g.to_string());
        }
    }
    let servers = match state.servers().await {
        Ok(s) => s,
        Err(e) => {
            tracing::warn!("[Chat] word migration skipped: {}", e);
            return;
        }
    };
    let moved = words::migrate(
        &bot.node_settings,
        &guilds,
        |g| guild_servers(&bot.node_settings, g, &servers),
        &bot.instance_aliases,
        &bot.module_aliases,
        &bot.command_aliases,
    );
    let Some(node_settings) = moved else { return };
    match state.config_store.finish_words_migration(node_settings).await {
        Ok(()) => tracing::info!("[Chat] moved global aliases to {} guild(s)", guilds.len()),
        Err(e) => tracing::warn!("[Chat] word migration not saved: {}", e),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::supervisor::module_loader::CommandField;

    #[test]
    fn a_module_command_named_like_a_builtin_is_one_row() {
        let stop = CommandField {
            name: "Stop".into(),
            label: "서버 끄기".into(),
            description: None,
            method: Some("console".into()),
            http_method: None,
            endpoint_template: None,
            console_template: Some("stop".into()),
            risk: Some("danger".into()),
            inputs: vec![],
        };
        let module = ModuleView { name: "minecraft".into(), commands: vec![stop], ..Default::default() };
        let rows = command_rows(Some(&module), &InstanceWords::default());
        // 실행 경로처럼 대소문자는 가리지 않는다
        let stops: Vec<&Value> = rows.iter().filter(|r| r["name"].as_str().is_some_and(|n| n.eq_ignore_ascii_case("stop"))).collect();
        assert_eq!(stops.len(), 1, "{rows:?}");
        assert_eq!((stops[0]["label"].as_str(), stops[0]["risk"].as_str(), stops[0]["builtin"].as_bool()), (Some("서버 끄기"), Some("danger"), Some(true)));
        assert_eq!(rows.len(), BUILTIN_COMMANDS.len());
    }

    #[tokio::test]
    async fn allowed_instances_are_set_per_guild_and_checked() {
        use std::sync::Arc;
        let tmp = tempfile::tempdir().unwrap();
        let modules = tmp.path().join("modules");
        std::fs::create_dir_all(&modules).unwrap();
        let mut sup = crate::supervisor::Supervisor::new_with_instances_dir(modules.to_str().unwrap(), tmp.path().join("instances").to_str().unwrap());
        sup.initialize().await.unwrap();
        let inst = crate::instance::ServerInstance::new("srv", "plain");
        let id = inst.id.clone();
        sup.instance_store.add(inst).unwrap();
        let state = IPCServer::new(Arc::new(tokio::sync::RwLock::new(sup)), "127.0.0.1:0", crate::daemon_log::DaemonLogBuffer::new());
        let guild = "allowed-instances-test-guild";
        let put = |body: Value| {
            let state = state.clone();
            async move { crate::ipc::internal::into_internal(put_allowed(State(state), Path(guild.to_string()), Json(body)).await).await }
        };
        let stored = || async { state.config_store.get_bot_config().await.node_settings[guild].get("allowedInstances").cloned() };

        let r = put(json!({ "instances": [id, id] })).await;
        assert_eq!(r.status, StatusCode::OK);
        assert_eq!(r.body, json!({ "guildId": guild, "allowedInstances": [id] }));
        assert_eq!(stored().await, Some(json!([id])));

        let r = put(json!({ "instances": ["nope"] })).await;
        assert_eq!(r.status, StatusCode::BAD_REQUEST);
        assert_eq!(r.body["details"], json!([{ "field": "instances", "reason": "unknown_instance", "id": "nope" }]));
        assert_eq!(put(json!({ "instances": "x" })).await.body["details"][0]["reason"], "invalid");
        assert_eq!(put(json!({})).await.status, StatusCode::BAD_REQUEST);

        assert_eq!(put(json!({ "instances": [] })).await.body["allowedInstances"], json!([]));
        assert_eq!(stored().await, Some(json!([])));
        assert_eq!(put(json!({ "instances": null })).await.body["allowedInstances"], Value::Null);
        assert_eq!(stored().await, None);
    }
}
