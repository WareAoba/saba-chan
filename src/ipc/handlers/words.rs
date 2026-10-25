//! 인스턴스별 디스코드 낱말 API (봇 설정 `serverWords`)와 길드가 쓸 서버 API
//!
//! `GET`·`PUT /api/instance/:id/words`, 길드가 쓸 서버 목록 `PUT /api/guilds/:guild/allowed-instances`.
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
use crate::chat::words::{self, InstanceWords};
use super::super::IPCServer;

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

/// 낱말 항목: `{id, name, module, defaultWords, words, commands:[{name, label, risk, builtin, moduleAliases, aliases}]}`.
/// `defaultWords`는 사용자가 정하지 않아도 이 서버를 부르는 낱말 (`words::default_words`)
fn entry(server: &ServerView, default_words: Vec<String>, module: Option<&ModuleView>, mine: &InstanceWords) -> Value {
    json!({ "id": server.id, "name": server.name, "module": server.module, "defaultWords": default_words, "words": mine.words, "commands": command_rows(module, mine) })
}

fn not_found(id: &str) -> Response {
    (StatusCode::NOT_FOUND, Json(json!({ "success": false, "error": format!("Instance not found: {}", id), "error_code": "not_found" }))).into_response()
}

/// `GET /api/instance/:id/words`
pub async fn get(State(state): State<IPCServer>, Path(id): Path<String>) -> Response {
    let servers = match state.servers().await {
        Ok(s) => s,
        Err(e) => return (StatusCode::INTERNAL_SERVER_ERROR, Json(json!({ "success": false, "error": e }))).into_response(),
    };
    let Some(server) = servers.iter().find(|s| s.id == id) else { return not_found(&id) };
    let modules = state.modules().await;
    let bot = state.config_store.get_bot_config().await;
    let mine = bot.server_words.get(&id).cloned().unwrap_or_default();
    let defaults = words::default_words(server, &servers, &modules, &bot.server_words);
    Json(entry(server, defaults, modules.iter().find(|m| m.name == server.module), &mine)).into_response()
}

/// `PUT /api/instance/:id/words` `{words, commands}`
pub async fn put(State(state): State<IPCServer>, Path(id): Path<String>, Json(body): Json<Value>) -> Response {
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
    let Some(instance) = servers.iter().find(|s| s.id == id).cloned() else { return not_found(&id) };
    let modules = state.modules().await;
    let module = modules.iter().find(|m| m.name == instance.module);
    // 검사와 저장을 설정 잠금 안에서 한다: 동시에 온 두 요청이 같은 낱말을 둘 다 통과시키지 않게.
    // 낱말은 어느 길드에서나 쓰이므로 모든 서버와 겹치지 않아야 한다
    let saved = state
        .config_store
        .update_server_words(|table| {
            let others: HashMap<String, InstanceWords> = table.iter().filter(|(k, _)| *k != &id).map(|(k, v)| (k.clone(), v.clone())).collect();
            let problems = words::validate(&instance, module, &want, &servers, &modules, &others);
            if !problems.is_empty() {
                return (Err(problems), false);
            }
            let changed = if want.is_empty() { table.remove(&id).is_some() } else { table.insert(id.clone(), want.clone()).as_ref() != Some(&want) };
            (Ok(()), changed)
        })
        .await;
    match saved {
        Ok(Ok(())) => Json(json!({ "success": true, "id": id, "words": want.words, "commands": want.commands })).into_response(),
        Ok(Err(problems)) => validation(problems.into_iter().map(|p| serde_json::to_value(p).unwrap_or_default()).collect()),
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

/// 시작할 때 한 번: 예전 낱말(0.1.10의 길드별 낱말, 그 전의 전역 별명)을 인스턴스 낱말로 옮긴다
pub async fn migrate_on_start(state: &IPCServer) {
    let bot = state.config_store.get_bot_config().await;
    let guild_words = bot.node_settings.as_object().is_some_and(|m| m.values().any(|n| n.get(words::INSTANCES_KEY).is_some()));
    if !guild_words && bot.module_aliases.is_empty() && bot.command_aliases.is_empty() && bot.instance_aliases.is_empty() {
        return;
    }
    let servers = match state.servers().await {
        Ok(s) => s,
        Err(e) => {
            tracing::warn!("[Chat] word migration skipped: {}", e);
            return;
        }
    };
    // 옮길 값은 저장 잠금 안에서 지금 설정으로 만든다 (그사이 바뀐 길드 권한을 덮지 않게)
    let moved = state
        .config_store
        .finish_words_migration(|bot| {
            let moved = words::migrate(&bot.node_settings, &servers, &bot.server_words, &bot.instance_aliases, &bot.module_aliases, &bot.command_aliases)?;
            let count = moved.server_words.len();
            Some((moved, count))
        })
        .await;
    match moved {
        Ok(Some(count)) => tracing::info!("[Chat] moved old discord words to {} instance(s)", count),
        Ok(None) => {}
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
            remote: true,
            reply_pattern: None,
        };
        let module = ModuleView { name: "minecraft".into(), commands: vec![stop], ..Default::default() };
        let rows = command_rows(Some(&module), &InstanceWords::default());
        // 실행 경로처럼 대소문자는 가리지 않는다
        let stops: Vec<&Value> = rows.iter().filter(|r| r["name"].as_str().is_some_and(|n| n.eq_ignore_ascii_case("stop"))).collect();
        assert_eq!(stops.len(), 1, "{rows:?}");
        assert_eq!((stops[0]["label"].as_str(), stops[0]["risk"].as_str(), stops[0]["builtin"].as_bool()), (Some("서버 끄기"), Some("danger"), Some(true)));
        assert_eq!(rows.len(), BUILTIN_COMMANDS.len());
    }

    /// 옛 전역 별명 옮기기가 그사이 바뀐 길드 권한을 덮어쓰지 않는다: 옮길 값은 저장 잠금 안에서 지금 설정으로 만든다 (D-F5)
    #[tokio::test(flavor = "multi_thread", worker_threads = 4)]
    async fn moving_old_aliases_keeps_a_concurrent_permission_change() {
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
        let guild = "alias-migration-race-guild";
        for round in 0..200u32 {
            state.config_store.set_bot_config_from_json(json!({ "instanceAliases": { id.clone(): format!("w{round}") } })).await.unwrap();
            let (a, b) = (state.clone(), state.clone());
            let migrate = tokio::spawn(async move { migrate_on_start(&a).await });
            let grant = tokio::spawn(async move {
                b.config_store
                    .update_node_settings(guild, |node| {
                        node.insert("members".into(), json!({ "x": round }));
                        ((), true)
                    })
                    .await
            });
            migrate.await.unwrap();
            grant.await.unwrap().unwrap();
            let bot = state.config_store.get_bot_config().await;
            assert_eq!(bot.node_settings[guild]["members"]["x"], round, "round {}", round);
        }
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

    /// 낱말은 인스턴스에 딸린다: 길드 없이 저장하고, 모든 서버와 겹치지 않아야 하며, 봇 설정 일괄 저장이 덮지 않는다
    #[tokio::test]
    async fn words_belong_to_the_instance() {
        use std::sync::Arc;
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
        let put_words = |id: String, body: Value| {
            let state = state.clone();
            async move { crate::ipc::internal::into_internal(put(State(state), Path(id), Json(body)).await).await }
        };

        let r = put_words(ia.clone(), json!({ "words": [" 서바 "], "commands": { "start": ["켜"] } })).await;
        assert_eq!(r.status, StatusCode::OK, "{:?}", r.body);
        let got = crate::ipc::internal::into_internal(get(State(state.clone()), Path(ia.clone())).await).await;
        assert_eq!(got.body["words"], json!(["서바"]));
        // 같은 모듈의 서버가 둘이라 기본 호출명은 인스턴스 이름뿐
        assert_eq!(got.body["defaultWords"], json!(["survival"]));
        assert_eq!(got.body["commands"].as_array().unwrap().iter().find(|c| c["name"] == "start").unwrap()["aliases"], json!(["켜"]));

        // 다른 서버의 낱말·이름과 겹치면 거절한다 (어느 길드가 어느 서버를 쓰는지와 상관없다)
        let r = put_words(ib.clone(), json!({ "words": ["서바", "survival"] })).await;
        assert_eq!(r.status, StatusCode::BAD_REQUEST);
        let reasons: Vec<&str> = r.body["details"].as_array().unwrap().iter().map(|d| d["reason"].as_str().unwrap()).collect();
        assert_eq!(reasons, ["conflict", "conflict"]);
        assert_eq!(put_words("nope".into(), json!({ "words": [] })).await.status, StatusCode::NOT_FOUND);

        // 봇 설정을 통째로 저장해도 낱말은 낱말 API로만 바뀐다
        state.config_store.set_bot_config_from_json(json!({ "serverWords": {} })).await.unwrap();
        assert_eq!(state.config_store.get_bot_config().await.server_words[&ia].words, vec!["서바"]);
        // 빈 값이면 항목을 지운다
        assert_eq!(put_words(ia.clone(), json!({ "words": [], "commands": {} })).await.status, StatusCode::OK);
        assert!(!state.config_store.get_bot_config().await.server_words.contains_key(&ia));
    }
}
