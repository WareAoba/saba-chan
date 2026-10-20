//! 디스코드 명령 권한 판정 (P1). 원본은 `config.json`의 `nodeSettings[guildId]` 하나다
//!
//! 레벨: 소유자(3) > 관리자(2) > 유저(1) > 챗봇(0) > 금지(-1). 숫자는 릴레이(server-chan `commandAccess.ts`)와 같다.
//! 명령마다 요구 레벨이 있다 (`commandLevels` 덮어쓰기, 없으면 프리셋표[risk]). 유효 레벨이 모자라면 거부한다.
//! 챗봇을 거친 요청의 유효 레벨은 min(작성자 레벨, `chatbotCap`)이다.

use serde::Serialize;
use serde_json::{json, Map, Value};
use std::collections::HashMap;

pub const OWNER: i64 = 3;
pub const ADMIN: i64 = 2;
pub const USER: i64 = 1;
pub const CHATBOT: i64 = 0;
pub const BANNED: i64 = -1;

/// 멤버에게 줄 수 있는 레벨 (소유자는 줄 수 없다)
pub fn grantable(level: i64) -> bool {
    (BANNED..=ADMIN).contains(&level)
}

/// 명령에 요구할 수 있는 레벨
pub fn requirable(level: i64) -> bool {
    (CHATBOT..=OWNER).contains(&level)
}

pub const PRESETS: [&str; 3] = ["loose", "default", "strict"];

/// 프리셋이 risk(safe/caution/danger)에 요구하는 레벨. 모르는 risk는 caution
pub fn preset_level(preset: &str, risk: &str) -> i64 {
    let (safe, caution, danger) = match preset {
        "loose" => (CHATBOT, CHATBOT, USER),
        "strict" => (USER, ADMIN, OWNER),
        _ => (CHATBOT, USER, ADMIN),
    };
    match risk {
        "safe" => safe,
        "danger" => danger,
        _ => caution,
    }
}

/// 프리셋표 전체 (`GET /api/permissions/:guildId`의 `presets`)
pub fn presets_table() -> Value {
    let mut out = Map::new();
    for p in PRESETS {
        out.insert(
            p.into(),
            json!({ "safe": preset_level(p, "safe"), "caution": preset_level(p, "caution"), "danger": preset_level(p, "danger") }),
        );
    }
    Value::Object(out)
}

/// 원격 요청자: 누가, 어느 길드에서, 챗봇을 거쳤는지
#[derive(Debug, Clone, Default, PartialEq)]
pub struct Requester {
    pub guild_id: Option<String>,
    pub user_id: Option<String>,
    /// 릴레이가 알려 준 노드 주인. 없으면 소유자가 없는 것으로 본다
    pub owner_id: Option<String>,
    pub via_chatbot: bool,
    /// 챗봇이 올린 확인을 누른 사람 (릴레이 `confirmedBy`, 누른 사람이 요청자일 때만 싣는다)
    pub confirmed_by: Option<String>,
}

impl Requester {
    /// 요청자 본인이 확인했는가 (릴레이의 말을 다시 견준다)
    pub fn confirmed_by_requester(&self) -> bool {
        self.confirmed_by.is_some() && self.confirmed_by == self.user_id
    }
}

/// 거부 이유 (`PERMISSION_DENIED`)
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct Denied {
    pub need: i64,
    pub level: i64,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub via: Option<&'static str>,
}

/// 한 길드의 권한 설정. 키가 없으면 기본값 (설정이 없는 길드도 같은 규칙으로 판정한다)
#[derive(Debug, Clone, PartialEq)]
pub struct NodePermissions {
    pub default_level: i64,
    pub members: HashMap<String, i64>,
    pub preset: String,
    pub command_levels: HashMap<String, i64>,
    pub chatbot_cap: i64,
    /// 예전 `memberPermissions[user][instanceId] = [명령...]` 금지 목록 (읽기만)
    legacy_denied: HashMap<String, HashMap<String, Vec<String>>>,
}

impl Default for NodePermissions {
    fn default() -> Self {
        Self {
            default_level: USER,
            members: HashMap::new(),
            preset: "default".into(),
            command_levels: HashMap::new(),
            chatbot_cap: CHATBOT,
            legacy_denied: HashMap::new(),
        }
    }
}

fn level_in(v: Option<&Value>, ok: fn(i64) -> bool) -> Option<i64> {
    v.and_then(Value::as_i64).filter(|l| ok(*l))
}

impl NodePermissions {
    /// `nodeSettings[guildId]` 값에서 읽는다. 범위를 벗어난 값은 버리고 기본값을 쓴다
    pub fn from_node(node: Option<&Value>) -> Self {
        let mut p = Self::default();
        let Some(node) = node.and_then(Value::as_object) else { return p };
        if let Some(l) = level_in(node.get("defaultLevel"), grantable) {
            p.default_level = l;
        }
        if let Some(l) = level_in(node.get("chatbotCap"), grantable) {
            p.chatbot_cap = l;
        }
        if let Some(preset) = node.get("preset").and_then(Value::as_str).filter(|s| PRESETS.contains(s)) {
            p.preset = preset.into();
        }
        for (id, l) in node.get("members").and_then(Value::as_object).into_iter().flatten() {
            if let Some(l) = level_in(Some(l), grantable) {
                p.members.insert(id.clone(), l);
            }
        }
        for (key, l) in node.get("commandLevels").and_then(Value::as_object).into_iter().flatten() {
            if let Some(l) = level_in(Some(l), requirable) {
                p.command_levels.insert(key.clone(), l);
            }
        }
        for (user, per) in node.get("memberPermissions").and_then(Value::as_object).into_iter().flatten() {
            let per: HashMap<String, Vec<String>> = per
                .as_object()
                .into_iter()
                .flatten()
                .map(|(inst, cmds)| {
                    let cmds = cmds.as_array().into_iter().flatten().filter_map(Value::as_str).map(str::to_string).collect();
                    (inst.clone(), cmds)
                })
                .collect();
            p.legacy_denied.insert(user.clone(), per);
        }
        p
    }

    /// 작성자 레벨 (챗봇 상한 전)
    pub fn level_of(&self, who: &Requester) -> i64 {
        let Some(user) = who.user_id.as_deref() else { return self.default_level };
        if who.owner_id.as_deref() == Some(user) {
            return OWNER;
        }
        self.members.get(user).copied().unwrap_or(self.default_level)
    }

    /// 판정에 쓰는 레벨: 챗봇을 거쳤으면 상한을 씌운다
    pub fn effective_level(&self, who: &Requester) -> i64 {
        let level = self.level_of(who);
        if who.via_chatbot {
            level.min(self.chatbot_cap)
        } else {
            level
        }
    }

    /// 명령별 덮어쓰기 (`commandLevels[key]`)
    pub fn override_level(&self, key: &str) -> Option<i64> {
        self.command_levels.get(key).copied()
    }

    /// 이 명령의 요구 레벨
    pub fn required(&self, instance_id: &str, command: &str, risk: &str) -> i64 {
        self.command_levels
            .get(&format!("{}/{}", instance_id, command))
            .copied()
            .unwrap_or_else(|| preset_level(&self.preset, risk))
    }

    fn denied(&self, who: &Requester, need: i64) -> Denied {
        Denied { need, level: self.effective_level(who), via: who.via_chatbot.then_some("chatbot") }
    }

    /// help·list처럼 명령이 아닌 요청: 금지만 막는다
    pub fn check_basic(&self, who: &Requester) -> Result<(), Denied> {
        if self.effective_level(who) <= BANNED {
            return Err(self.denied(who, CHATBOT));
        }
        Ok(())
    }

    /// 이 사람에게 이 인스턴스의 이 명령을 막아 두었는가 (`memberPermissions`). 막았으면 주인만 쓸 수 있다
    pub fn member_denied(&self, who: &Requester, instance_id: &str, command: &str) -> bool {
        who.user_id
            .as_deref()
            .and_then(|u| self.legacy_denied.get(u))
            .and_then(|per| per.get(instance_id))
            .is_some_and(|cmds| cmds.iter().any(|c| c == command))
    }

    /// 서버 명령 하나
    pub fn check(&self, who: &Requester, instance_id: &str, command: &str, risk: &str) -> Result<(), Denied> {
        self.check_basic(who)?;
        let level = self.effective_level(who);
        let need = if self.member_denied(who, instance_id, command) { OWNER } else { self.required(instance_id, command, risk) };
        if level < need {
            return Err(self.denied(who, need));
        }
        Ok(())
    }

    /// 화면·API에 보이는 모양 (`GET /api/permissions/:guildId`)
    pub fn to_view(&self, guild_id: &str, node: Option<&Value>) -> Value {
        let text = |k: &str| node.and_then(|n| n.get(k)).and_then(Value::as_str);
        json!({
            "guildId": guild_id,
            "ownerDiscordId": text("ownerDiscordId"),
            "relayOwnerDiscordId": text("relayOwnerDiscordId"),
            "defaultLevel": self.default_level,
            "preset": self.preset,
            "chatbotCap": self.chatbot_cap,
            "members": self.members,
            "commandLevels": self.command_levels,
            "presets": presets_table(),
        })
    }
}

/// 챗봇 경로인가. `via`가 "direct"일 때만 직접 명령이다. P1을 아는 릴레이(`ownerDiscordId`를 싣는다)가
/// `via`를 빠뜨리면 챗봇으로 본다 (열린 쪽으로 처리하지 않는다). 두 필드가 다 없는 옛 릴레이만 직접 명령
pub fn via_chatbot(via: Option<&str>, owner_reported: bool) -> bool {
    match via {
        Some("direct") => false,
        Some(_) => true,
        None => owner_reported,
    }
}

/// 노드 주인을 고정한다 (처음 받은 값을 `ownerDiscordId`에 저장). 고정된 값과 다른 값이 오면 무시하고
/// `relayOwnerDiscordId`에 적는다. 돌려주는 것: (판정에 쓸 주인, 설정을 바꿨는가, 충돌한 값)
pub fn pin_owner(node: &mut Map<String, Value>, received: Option<&str>) -> (Option<String>, bool, Option<String>) {
    let received = received.map(str::trim).filter(|s| !s.is_empty());
    let pinned = node.get("ownerDiscordId").and_then(Value::as_str).map(str::to_string).filter(|s| !s.is_empty());
    match (pinned, received) {
        (Some(p), Some(r)) if p != r => {
            let changed = node.get("relayOwnerDiscordId").and_then(Value::as_str) != Some(r);
            node.insert("relayOwnerDiscordId".into(), json!(r));
            (Some(p), changed, Some(r.to_string()))
        }
        (Some(p), _) => (Some(p), false, None),
        (None, Some(r)) => {
            node.insert("ownerDiscordId".into(), json!(r));
            (Some(r.to_string()), true, None)
        }
        (None, None) => (None, false, None),
    }
}

/// 권한 바꾸기 요청 하나 (HTTP·디스코드 공통). 실패는 오류 코드
#[derive(Debug, Clone, PartialEq)]
pub enum Change {
    Member { discord_id: String, level: Option<i64> },
    DefaultLevel(i64),
    Preset(String),
    ChatbotCap(i64),
    CommandLevel { key: String, level: Option<i64> },
    /// 고정된 주인을 바꾼다 (GUI·CLI만). 충돌 표시도 지운다
    Owner(String),
}

/// `node`(`nodeSettings[guildId]` 객체)에 바꾸기를 적용한다. `owner_id`는 강등·부여할 수 없다
pub fn apply(node: &mut Map<String, Value>, change: &Change, owner_id: Option<&str>) -> Result<(), &'static str> {
    fn sub<'m>(node: &'m mut Map<String, Value>, key: &str) -> &'m mut Map<String, Value> {
        let slot = node.entry(key).or_insert_with(|| json!({}));
        if !slot.is_object() {
            *slot = json!({});
        }
        slot.as_object_mut().expect("object")
    }
    match change {
        Change::Member { discord_id, level } => {
            if discord_id.trim().is_empty() {
                return Err("invalid_member");
            }
            if owner_id == Some(discord_id.as_str()) {
                return Err("owner_fixed");
            }
            match level {
                Some(l) if !grantable(*l) => return Err("invalid_level"),
                Some(l) => {
                    sub(node, "members").insert(discord_id.clone(), json!(l));
                }
                None => {
                    sub(node, "members").remove(discord_id);
                }
            }
        }
        Change::DefaultLevel(l) | Change::ChatbotCap(l) if !grantable(*l) => return Err("invalid_level"),
        Change::DefaultLevel(l) => {
            node.insert("defaultLevel".into(), json!(l));
        }
        Change::ChatbotCap(l) => {
            node.insert("chatbotCap".into(), json!(l));
        }
        Change::Preset(p) if !PRESETS.contains(&p.as_str()) => return Err("invalid_preset"),
        Change::Preset(p) => {
            node.insert("preset".into(), json!(p));
        }
        Change::Owner(id) if id.trim().is_empty() => return Err("invalid_member"),
        Change::Owner(id) => {
            node.insert("ownerDiscordId".into(), json!(id.trim()));
            node.remove("relayOwnerDiscordId");
            if let Some(members) = node.get_mut("members").and_then(Value::as_object_mut) {
                members.remove(id.trim());
            }
        }
        Change::CommandLevel { key, level } => {
            // `<instance>/<명령·액션 id>` 또는 전역 액션 덮어쓰기 `<action id>`. 지우기는 예전에 저장된 키도 받는다
            let well_formed = match key.split_once('/') {
                Some((i, c)) => !i.is_empty() && !c.is_empty(),
                None => level.is_none() || crate::actions::overridable(key),
            };
            if !well_formed {
                return Err("invalid_command");
            }
            match level {
                Some(l) if !requirable(*l) => return Err("invalid_level"),
                Some(l) => {
                    sub(node, "commandLevels").insert(key.clone(), json!(l));
                }
                None => {
                    sub(node, "commandLevels").remove(key);
                }
            }
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn who(user: &str) -> Requester {
        Requester { guild_id: Some("g".into()), user_id: Some(user.into()), owner_id: Some("own".into()), via_chatbot: false, ..Default::default() }
    }

    #[test]
    fn members_without_a_level_get_the_default_and_banned_runs_nothing() {
        let p = NodePermissions::from_node(Some(&json!({ "members": { "a": 2, "b": -1, "bad": 9 } })));
        assert_eq!(p.level_of(&who("x")), USER);
        assert_eq!(p.level_of(&who("bad")), USER);
        assert!(p.check(&who("x"), "i", "stop", "caution").is_ok());
        assert_eq!(p.check(&who("x"), "i", "op", "danger"), Err(Denied { need: ADMIN, level: USER, via: None }));
        assert!(p.check(&who("a"), "i", "op", "danger").is_ok());
        assert!(p.check_basic(&who("b")).is_err());
        assert!(p.check(&who("b"), "i", "status", "safe").is_err());

        // 설정이 없는 길드도 같은 기본 규칙
        let none = NodePermissions::from_node(None);
        assert!(none.check(&who("x"), "i", "op", "danger").is_err());
        // 기본 레벨을 금지로 바꾸면 레벨 없는 멤버는 아무것도 못 한다
        let closed = NodePermissions::from_node(Some(&json!({ "defaultLevel": -1 })));
        assert!(closed.check_basic(&who("x")).is_err());
        assert!(closed.check(&who("own"), "i", "op", "danger").is_ok());
    }

    #[test]
    fn presets_overrides_and_the_chatbot_cap() {
        let p = NodePermissions::from_node(Some(&json!({
            "preset": "strict",
            "members": { "a": 2 },
            "commandLevels": { "i/kick": 1, "i/bad": -1 }
        })));
        assert_eq!(p.required("i", "op", "danger"), OWNER);
        assert_eq!(p.required("i", "kick", "danger"), USER);
        assert_eq!(p.required("i", "bad", "caution"), ADMIN);
        assert!(p.check(&who("a"), "i", "op", "danger").is_err());
        assert!(p.check(&who("own"), "i", "op", "danger").is_ok());

        // 챗봇을 거치면 관리자·소유자도 상한(챗봇) 안에서만
        let p = NodePermissions::from_node(None);
        let mut bot = who("own");
        bot.via_chatbot = true;
        assert!(p.check(&bot, "i", "status", "safe").is_ok());
        assert_eq!(p.check(&bot, "i", "stop", "caution"), Err(Denied { need: USER, level: CHATBOT, via: Some("chatbot") }));
        // 상한보다 낮은 작성자는 그대로 낮다
        let p = NodePermissions::from_node(Some(&json!({ "chatbotCap": 2, "members": { "z": 0 } })));
        let mut low = who("z");
        low.via_chatbot = true;
        assert_eq!(p.effective_level(&low), CHATBOT);
    }

    #[test]
    fn legacy_deny_lists_still_block_and_the_owner_is_not_assumed() {
        let p = NodePermissions::from_node(Some(&json!({ "memberPermissions": { "u": { "i": ["stop"] } } })));
        assert!(p.check(&who("u"), "i", "stop", "caution").is_err());
        assert!(p.check(&who("u"), "j", "stop", "caution").is_ok());
        // 릴레이가 주인을 알려 주지 않으면 누구도 소유자가 아니다
        let mut anon = who("own");
        anon.owner_id = None;
        assert_eq!(p.level_of(&anon), USER);
    }

    #[test]
    fn a_relay_that_reports_the_owner_but_drops_via_is_treated_as_the_chatbot() {
        assert!(!via_chatbot(Some("direct"), true));
        assert!(via_chatbot(Some("chatbot"), true));
        assert!(via_chatbot(None, true));
        assert!(via_chatbot(Some("other"), false));
        // 옛 릴레이 (두 필드 다 없음)
        assert!(!via_chatbot(None, false));
    }

    #[test]
    fn the_first_owner_is_pinned_and_a_different_one_is_ignored() {
        let mut node = Map::new();
        assert_eq!(pin_owner(&mut node, None), (None, false, None));
        assert_eq!(pin_owner(&mut node, Some("own")), (Some("own".into()), true, None));
        assert_eq!(pin_owner(&mut node, Some("own")), (Some("own".into()), false, None));
        assert_eq!(pin_owner(&mut node, Some("evil")), (Some("own".into()), true, Some("evil".into())));
        assert_eq!(pin_owner(&mut node, Some("evil")), (Some("own".into()), false, Some("evil".into())));
        assert_eq!(node["ownerDiscordId"], "own");
        // GUI·CLI에서만 바꾼다
        apply(&mut node, &Change::Owner("evil".into()), None).unwrap();
        assert_eq!(pin_owner(&mut node, Some("evil")), (Some("evil".into()), false, None));
        assert!(node.get("relayOwnerDiscordId").is_none());
    }

    #[test]
    fn changes_keep_the_owner_fixed_and_levels_in_range() {
        let mut node = Map::new();
        let owner = Some("own");
        assert_eq!(apply(&mut node, &Change::Member { discord_id: "own".into(), level: Some(-1) }, owner), Err("owner_fixed"));
        assert_eq!(apply(&mut node, &Change::Member { discord_id: "a".into(), level: Some(3) }, owner), Err("invalid_level"));
        apply(&mut node, &Change::Member { discord_id: "a".into(), level: Some(2) }, owner).unwrap();
        apply(&mut node, &Change::DefaultLevel(-1), owner).unwrap();
        assert_eq!(apply(&mut node, &Change::Preset("open".into()), owner), Err("invalid_preset"));
        assert_eq!(apply(&mut node, &Change::CommandLevel { key: "kick".into(), level: Some(1) }, owner), Err("invalid_command"));
        apply(&mut node, &Change::CommandLevel { key: "i/kick".into(), level: Some(3) }, owner).unwrap();
        // 전역 액션 덮어쓰기는 원격에서 부를 수 있고 레벨이 고정되지 않은 액션 id만
        for fixed in ["update.apply", "instance.delete"] {
            assert_eq!(apply(&mut node, &Change::CommandLevel { key: fixed.into(), level: Some(0) }, owner), Err("invalid_command"));
        }
        // 인스턴스 액션은 `<instance>/<action id>`로만 판정하므로 전역 키는 받지 않는다. 예전에 저장된 것은 지울 수 있다
        for per_instance in ["server.stop", "backup.restore"] {
            assert_eq!(apply(&mut node, &Change::CommandLevel { key: per_instance.into(), level: Some(3) }, owner), Err("invalid_command"));
            apply(&mut node, &Change::CommandLevel { key: per_instance.into(), level: None }, owner).unwrap();
        }
        apply(&mut node, &Change::CommandLevel { key: "instance.create".into(), level: Some(3) }, owner).unwrap();
        apply(&mut node, &Change::CommandLevel { key: "instance.create".into(), level: None }, owner).unwrap();
        let p = NodePermissions::from_node(Some(&Value::Object(node.clone())));
        assert_eq!((p.members.get("a"), p.default_level, p.command_levels.get("i/kick")), (Some(&2), BANNED, Some(&3)));
        apply(&mut node, &Change::Member { discord_id: "a".into(), level: None }, owner).unwrap();
        apply(&mut node, &Change::CommandLevel { key: "i/kick".into(), level: None }, owner).unwrap();
        assert_eq!(node["members"], json!({}));
        assert_eq!(node["commandLevels"], json!({}));
    }
}
