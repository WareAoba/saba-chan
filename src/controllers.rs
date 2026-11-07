//! 인스턴스 사용 모드와 명단 (사용자 지정 2026-11-08).
//!
//! - `access = shared`(서버 공용, 기본): 디스코드 요청은 그 길드의 레벨 권한을 따른다 (`actions::authorize`).
//!   `kick_players`에 적은 닉네임은 접속하면 내보낸다.
//! - `access = members`(전용 멤버): 길드 레벨과 따로 논다. `controllers`(전용 멤버 명단)에 디스코드를 이은 사람만
//!   이 인스턴스의 모든 조작·명령을 하고(금지 레벨은 막는다), 나머지는 레벨과 상관없이 거절한다. `controllers_only`를 켜면
//!   명단에 없는 닉네임이 접속할 때 내보낸다 (명단이 비면 켤 수 없다).
//! - 내보내기는 모듈 `[config] kick_command`로 한다. 차단하지 않는다. PC 앱·웹(주인)은 이 판정 밖이다.
//! - 명단 한 줄은 게임 닉네임(필수)과 디스코드 계정(선택)이다. 닉네임은 대소문자를 가리지 않고 비교한다.

use serde::{Deserialize, Serialize};
use serde_json::{json, Value};

use crate::ipc::IPCServer;

/// 명단에 넣을 수 있는 사람 수
pub const MAX_CONTROLLERS: usize = 200;
/// 닉네임 최대 길이 (문자). 접속 줄에서 읽는 이름도 64자에서 자른다 (`player_events::NAME_MAX`)
const NAME_MAX: usize = 64;

/// 권한 계단의 "조작 가능" 줄에 묶이는 일 (액션 id). 서버 공용에서는 이 줄 위 레벨이 한다
pub const CONTROLLED_ACTIONS: [&str; 4] = ["server.start", "server.stop", "server.restart", "backup.create"];

/// 인스턴스 사용 모드
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Access {
    /// 서버 공용: 길드 레벨 권한을 따른다
    #[default]
    Shared,
    /// 전용 멤버: 명단에 있는 사람만 쓴다
    Members,
}

impl Access {
    pub fn is_shared(&self) -> bool {
        *self == Access::Shared
    }

    /// PATCH 값 `access`: "shared" 또는 "members"
    pub fn parse(value: &Value) -> Result<Self, &'static str> {
        match value.as_str() {
            Some("shared") => Ok(Access::Shared),
            Some("members") => Ok(Access::Members),
            _ => Err("access must be shared or members"),
        }
    }
}

/// 디스코드 요청을 이 인스턴스에서 어떻게 볼지
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Gate {
    /// 길드 레벨 권한으로 판정한다 (서버 공용)
    Levels,
    /// 전용 멤버: 레벨을 보지 않고 허락한다
    Allow,
    /// 전용 멤버가 아니다: 레벨과 상관없이 거절한다
    Deny,
}

/// 디스코드 사용자(`user`) 요청의 판정. 말하는 사바쨩을 거친 요청은 전용 멤버여도 챗봇 상한(레벨 판정)을 받는다
pub fn gate_for(access: Access, list: &[Controller], user: Option<&str>, via_chatbot: bool) -> Gate {
    match access {
        Access::Shared => Gate::Levels,
        Access::Members => match user {
            Some(u) if controls(list, u) => {
                if via_chatbot {
                    Gate::Levels
                } else {
                    Gate::Allow
                }
            }
            _ => Gate::Deny,
        },
    }
}

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

/// PATCH 값 `kick_players`(서버 공용의 자동 강퇴 닉네임)를 읽는다: 문자열 목록. 앞뒤 공백을 지우고 겹치면 거절한다
pub fn parse_kick_list(value: &Value) -> Result<Vec<String>, &'static str> {
    let Some(list) = value.as_array() else { return Err("kick_players must be a list") };
    if list.len() > MAX_CONTROLLERS {
        return Err("kick_players has too many people");
    }
    let mut out: Vec<String> = Vec::with_capacity(list.len());
    for item in list {
        let name = item.as_str().map(str::trim).unwrap_or_default();
        if name.is_empty() || name.chars().count() > NAME_MAX || name.chars().any(char::is_control) {
            return Err("each kick_players entry needs a game nickname of at most 64 characters");
        }
        if out.iter().any(|n| same_name(n, name)) {
            return Err("the same game nickname is listed twice");
        }
        out.push(name.to_string());
    }
    Ok(out)
}

/// 이 닉네임이 접속하면 내보낼까: 서버 공용은 자동 강퇴 명단에 있으면, 전용 멤버는 켜 두었고 명단에 없으면
pub fn should_kick(access: Access, members: &[Controller], members_only_join: bool, kick_players: &[String], player: &str) -> bool {
    match access {
        Access::Shared => kick_players.iter().any(|n| same_name(n, player.trim())),
        Access::Members => members_only_join && !members.is_empty() && !listed(members, player),
    }
}

/// 이 닉네임이 명단에 있는가
pub fn listed(list: &[Controller], name: &str) -> bool {
    list.iter().any(|c| same_name(&c.name, name.trim()))
}

/// 이 디스코드 사용자가 명단에 있는가
pub fn controls(list: &[Controller], discord_id: &str) -> bool {
    list.iter().any(|c| c.discord_id.as_deref() == Some(discord_id))
}

/// 저장 전 검사: 명단이 비면 "명단에 없는 사람 자동 강퇴"를 켤 수 없다 (켜면 모두 내보낸다)
pub fn check(list: &[Controller], only: bool) -> Result<(), &'static str> {
    if only && list.is_empty() {
        return Err("controllers_only needs at least one controller");
    }
    Ok(())
}

/// 볼 사람: 접속 알림(`instance.player` join)의 한 명, 또는 다시 붙은 서버에 이미 들어와 있던 사람들
/// (`instance.players_present`)
pub fn gate_targets(kind: &str, data: &Value) -> Vec<(String, String)> {
    let Some(id) = data["instanceId"].as_str() else { return Vec::new() };
    let players: Vec<&str> = match kind {
        "instance.player" if data["change"] == "join" => data["player"].as_str().into_iter().collect(),
        "instance.players_present" => data["players"].as_array().into_iter().flatten().filter_map(Value::as_str).collect(),
        _ => Vec::new(),
    };
    players.into_iter().map(|p| (id.to_string(), p.to_string())).collect()
}

/// 접속 알림을 보고, 명단에 없는 닉네임을 내보낸다. 데몬이 켜질 때 한 번 띄운다.
/// 이 데몬이 켜진 뒤 낸 알림부터 본다: 다시 붙은 서버의 알림은 이 감시가 뜨기 전에 나올 수 있다
pub fn spawn_gate(state: IPCServer) {
    tokio::spawn(async move {
        let sub = crate::events::bus().subscribe(Some(0));
        let mut rx = sub.rx;
        let mut backlog = sub.backlog.into_iter();
        loop {
            let event = match backlog.next() {
                Some(e) => e,
                None => match rx.recv().await {
                    Ok(e) => e,
                    // 밀려 건너뛴 접속 알림이 있을 수 있다: 지금 들어와 있는 사람을 다시 본다
                    Err(tokio::sync::broadcast::error::RecvError::Lagged(_)) => {
                        sweep(&state, None);
                        continue;
                    }
                    Err(_) => return,
                },
            };
            for (id, player) in gate_targets(&event.kind, &event.data) {
                let state = state.clone();
                tokio::spawn(async move { gate(&state, &id, &player).await });
            }
        }
    });
}

/// 지금 들어와 있는 사람(접속·퇴장 줄로 센 명단)을 게이트에 한 번 넣는다. `instance_id`가 없으면 모든 서버.
/// 게이트가 모드와 명단을 다시 보므로 내보낼 사람이 아니면 그대로 둔다
pub fn sweep(state: &IPCServer, instance_id: Option<&str>) {
    for (id, player) in crate::supervisor::players::present(instance_id) {
        let state = state.clone();
        tokio::spawn(async move { gate(&state, &id, &player).await });
    }
}

/// 설정 저장 뒤 이미 들어와 있는 사람을 볼 때: 모드·강퇴 스위치·명단 중 하나가 바뀌었다 (내보낼지는 게이트가 본다)
pub fn sweep_needed(was: &crate::instance::ServerInstance, now: &crate::instance::ServerInstance) -> bool {
    was.access != now.access
        || was.controllers_only != now.controllers_only
        || was.controllers != now.controllers
        || was.kick_players != now.kick_players
}

/// 내보낼 사람이면 모듈의 내보내기 명령 이름과 그 입력 이름들
async fn kick_plan(state: &IPCServer, id: &str, player: &str) -> Option<(String, Vec<String>)> {
    let supervisor = state.supervisor.read().await;
    let instance = supervisor.instance_store.get(id)?;
    if !should_kick(instance.access, &instance.controllers, instance.controllers_only, &instance.kick_players, player) {
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
    tracing::info!("[Controllers] 자동 강퇴 '{}': instance={} ok={}", player, id, ok);
    crate::events::emit("instance.player_kicked", json!({ "instanceId": id, "player": player, "success": ok }));
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_gate_looks_at_joins_and_at_players_already_in_after_a_reattach() {
        let join = json!({ "instanceId": "i", "player": "Steve", "change": "join" });
        assert_eq!(gate_targets("instance.player", &join), [("i".to_string(), "Steve".to_string())]);
        assert!(gate_targets("instance.player", &json!({ "instanceId": "i", "player": "Steve", "change": "leave" })).is_empty());
        let present = json!({ "instanceId": "i", "players": ["Alex", "Bob"] });
        assert_eq!(gate_targets("instance.players_present", &present).len(), 2);
        assert!(gate_targets("instance.chat", &join).is_empty());
    }

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
    fn people_already_in_are_checked_when_the_mode_switch_or_a_list_changes() {
        let was = crate::instance::ServerInstance::new("a", "game");
        let mut now = was.clone();
        assert!(!sweep_needed(&was, &now), "nothing changed");
        now.access = Access::Members;
        assert!(sweep_needed(&was, &now), "mode changed");
        let mut kicks = was.clone();
        kicks.kick_players = vec!["griefer".into()];
        assert!(sweep_needed(&was, &kicks), "kick list changed");
    }

    #[test]
    fn shared_servers_kick_only_the_named_players_and_members_servers_kick_everyone_else_when_on() {
        let members = parse(&json!([{ "name": "Steve" }])).unwrap();
        let kicks = vec!["Griefer".to_string()];
        assert!(should_kick(Access::Shared, &members, true, &kicks, "griefer"));
        assert!(!should_kick(Access::Shared, &members, true, &kicks, "Alex"), "shared ignores the members list");
        assert!(should_kick(Access::Members, &members, true, &[], "Alex"));
        assert!(!should_kick(Access::Members, &members, true, &[], "steve"));
        assert!(!should_kick(Access::Members, &members, false, &kicks, "Griefer"), "switch off: nobody, not even the shared list");
        assert!(parse_kick_list(&json!([" a ", "A"])).is_err());
        assert_eq!(parse_kick_list(&json!([" a "])).unwrap(), ["a"]);
        assert!(parse_kick_list(&json!([""])).is_err());
    }

    #[test]
    fn members_mode_lets_only_listed_discord_users_in_and_ignores_levels() {
        let members = parse(&json!([{ "name": "Steve", "discord_id": "332007306403250178" }])).unwrap();
        assert_eq!(gate_for(Access::Shared, &members, Some("1"), false), Gate::Levels);
        assert_eq!(gate_for(Access::Members, &members, Some("332007306403250178"), false), Gate::Allow);
        assert_eq!(gate_for(Access::Members, &members, Some("332007306403250178"), true), Gate::Levels, "chatbot cap still applies");
        assert_eq!(gate_for(Access::Members, &members, Some("1"), false), Gate::Deny);
        assert_eq!(gate_for(Access::Members, &members, None, false), Gate::Deny);
        assert_eq!(Access::parse(&json!("members")), Ok(Access::Members));
        assert!(Access::parse(&json!("all")).is_err());
    }

    #[test]
    fn kick_args_fill_the_nickname_then_the_reason() {
        let two = kick_args(&["player".into(), "reason".into()], "griefer", "not listed");
        assert_eq!(two, json!({ "player": "griefer", "reason": "not listed" }));
        assert_eq!(kick_args(&["userid".into()], "griefer", "x"), json!({ "userid": "griefer" }));
    }
}
