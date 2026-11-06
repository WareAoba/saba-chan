//! 조작 가능 (사용자 지정 2026-11-06): 인스턴스마다 시작·정지·재시작·백업 만들기를 할 수 있는 사람 명단.
//!
//! - 한 줄은 게임 닉네임(필수)과 디스코드 계정(선택)이다. 디스코드를 이은 사람만 디스코드에서 조작한다.
//! - 디스코드 요청은 명단에 있거나 요구 레벨(기본 관리자, 길드 명령별 덮어쓰기)을 넘어야 한다 (`actions::authorize`).
//!   PC 앱·웹(주인)은 이 판정 밖이다.
//! - 인스턴스 설정 `controllers_only`(사바쨩 탭 "조작 가능한 사람만 접속")를 켜면 명단에 없는 닉네임이 접속할 때
//!   모듈 `[config] kick_command`로 내보낸다. 차단하지 않는다. 명단이 비면 켤 수 없다.
//! - 닉네임은 대소문자를 가리지 않고 비교한다 (마인크래프트 계정 이름이 그렇다).

use serde::{Deserialize, Serialize};
use serde_json::{json, Value};

use crate::ipc::IPCServer;

/// 명단에 넣을 수 있는 사람 수
pub const MAX_CONTROLLERS: usize = 200;
/// 닉네임 최대 길이 (문자). 접속 줄에서 읽는 이름도 64자에서 자른다 (`player_events::NAME_MAX`)
const NAME_MAX: usize = 64;

/// 명단에 있는 사람이 하는 일 (액션 id)
pub const CONTROLLED_ACTIONS: [&str; 4] = ["server.start", "server.stop", "server.restart", "backup.create"];

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Controller {
    /// 게임 닉네임
    pub name: String,
    /// 이은 디스코드 계정 (사용자 ID)
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub discord_id: Option<String>,
}

fn same_name(a: &str, b: &str) -> bool {
    a.to_lowercase() == b.to_lowercase()
}

/// PATCH 값 `controllers`를 읽는다: `[{name, discord_id?}]`. 앞뒤 공백을 지우고, 겹치는 닉네임·잘못된 값은 거절한다
pub fn parse(value: &Value) -> Result<Vec<Controller>, &'static str> {
    let Some(list) = value.as_array() else { return Err("controllers must be a list") };
    if list.len() > MAX_CONTROLLERS {
        return Err("controllers has too many people");
    }
    let mut out: Vec<Controller> = Vec::with_capacity(list.len());
    for item in list {
        let name = item.get("name").and_then(Value::as_str).map(str::trim).unwrap_or_default();
        if name.is_empty() || name.chars().count() > NAME_MAX || name.chars().any(char::is_control) {
            return Err("each controller needs a game nickname of at most 64 characters");
        }
        let discord_id = match item.get("discord_id") {
            None | Some(Value::Null) => None,
            Some(Value::String(s)) if s.trim().is_empty() => None,
            Some(Value::String(s)) if (15..=21).contains(&s.trim().len()) && s.trim().bytes().all(|b| b.is_ascii_digit()) => Some(s.trim().to_string()),
            _ => return Err("discord_id must be a Discord user ID"),
        };
        if out.iter().any(|c| same_name(&c.name, name)) {
            return Err("the same game nickname is listed twice");
        }
        out.push(Controller { name: name.to_string(), discord_id });
    }
    Ok(out)
}

/// 이 닉네임이 명단에 있는가
pub fn listed(list: &[Controller], name: &str) -> bool {
    list.iter().any(|c| same_name(&c.name, name.trim()))
}

/// 이 디스코드 사용자가 명단에 있는가
pub fn controls(list: &[Controller], discord_id: &str) -> bool {
    list.iter().any(|c| c.discord_id.as_deref() == Some(discord_id))
}

/// 저장 전 검사: 명단이 비면 "조작 가능한 사람만 접속"을 켤 수 없다 (켜면 모두 내보낸다)
pub fn check(list: &[Controller], only: bool) -> Result<(), &'static str> {
    if only && list.is_empty() {
        return Err("controllers_only needs at least one controller");
    }
    Ok(())
}

/// 접속 알림(`instance.player`)을 보고, 명단에 없는 닉네임을 내보낸다. 데몬이 켜질 때 한 번 띄운다
pub fn spawn_gate(state: IPCServer) {
    tokio::spawn(async move {
        let mut sub = crate::events::bus().subscribe(None);
        loop {
            let event = match sub.rx.recv().await {
                Ok(e) => e,
                Err(tokio::sync::broadcast::error::RecvError::Lagged(_)) => continue,
                Err(_) => return,
            };
            if event.kind != "instance.player" || event.data["change"] != "join" {
                continue;
            }
            let (Some(id), Some(player)) = (event.data["instanceId"].as_str(), event.data["player"].as_str()) else { continue };
            let (id, player) = (id.to_string(), player.to_string());
            let state = state.clone();
            tokio::spawn(async move { gate(&state, &id, &player).await });
        }
    });
}

/// 내보낼 사람이면 모듈의 내보내기 명령 이름과 그 입력 이름들
async fn kick_plan(state: &IPCServer, id: &str, player: &str) -> Option<(String, Vec<String>)> {
    let supervisor = state.supervisor.read().await;
    let instance = supervisor.instance_store.get(id)?;
    if !instance.controllers_only || instance.controllers.is_empty() || listed(&instance.controllers, player) {
        return None;
    }
    let module = supervisor.module_loader.get_module(&instance.module_name).ok()?;
    let Some(name) = module.metadata.kick_command.clone() else {
        tracing::warn!("[Controllers] {} 모듈에 kick_command가 없어 '{}'을(를) 내보내지 못했다", instance.module_name, player);
        return None;
    };
    let inputs = module.metadata.commands.iter().flat_map(|c| &c.fields).find(|f| f.name == name).map(|c| c.inputs.iter().map(|i| i.name.clone()).collect())?;
    Some((name, inputs))
}

/// 내보내기 명령의 인자: 첫 입력은 닉네임, 둘째 입력이 있으면 이유
pub fn kick_args(inputs: &[String], player: &str, reason: &str) -> Value {
    let mut args = serde_json::Map::new();
    if let Some(first) = inputs.first() {
        args.insert(first.clone(), json!(player));
    }
    if let Some(second) = inputs.get(1) {
        args.insert(second.clone(), json!(reason));
    }
    Value::Object(args)
}

async fn gate(state: &IPCServer, id: &str, player: &str) {
    let Some((command, inputs)) = kick_plan(state, id, player).await else { return };
    let language = state.config_store.ui_language().await;
    let reason = crate::chat::i18n::Catalog::for_language(&language).t("controllers.kick_reason", &[]);
    let ok = crate::ipc::handlers::backup::run_command(state, id, &command, kick_args(&inputs, player, &reason)).await;
    tracing::info!("[Controllers] 조작 가능에 없는 '{}'을(를) 내보냄: instance={} ok={}", player, id, ok);
    crate::events::emit("instance.player_kicked", json!({ "instanceId": id, "player": player, "success": ok }));
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_trims_and_rejects_bad_lists() {
        let list = parse(&json!([{ "name": " Steve " }, { "name": "Alex", "discord_id": "332007306403250178" }])).unwrap();
        assert_eq!(list[0], Controller { name: "Steve".into(), discord_id: None });
        assert_eq!(list[1].discord_id.as_deref(), Some("332007306403250178"));
        assert!(parse(&json!([{ "name": "Steve" }, { "name": "steve" }])).is_err(), "nicknames are compared without case");
        assert!(parse(&json!([{ "name": "" }])).is_err());
        assert!(parse(&json!([{ "name": "a\nb" }])).is_err());
        assert!(parse(&json!([{ "name": "x", "discord_id": "not-an-id" }])).is_err());
        assert_eq!(parse(&json!([{ "name": "x", "discord_id": "" }])).unwrap()[0].discord_id, None);
        assert!(parse(&json!("Steve")).is_err());
    }

    #[test]
    fn nicknames_match_without_case_and_discord_ids_match_exactly() {
        let list = parse(&json!([{ "name": "Aoba_0412", "discord_id": "332007306403250178" }])).unwrap();
        assert!(listed(&list, "aoba_0412"));
        assert!(!listed(&list, "griefer123"));
        assert!(controls(&list, "332007306403250178"));
        assert!(!controls(&list, "332007306403250179"));
    }

    #[test]
    fn the_join_gate_needs_someone_on_the_list() {
        assert!(check(&[], true).is_err());
        assert!(check(&[], false).is_ok());
    }

    #[test]
    fn kick_args_fill_the_nickname_then_the_reason() {
        let two = kick_args(&["player".into(), "reason".into()], "griefer", "not listed");
        assert_eq!(two, json!({ "player": "griefer", "reason": "not listed" }));
        assert_eq!(kick_args(&["userid".into()], "griefer", "x"), json!({ "userid": "griefer" }));
    }
}
