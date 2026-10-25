//! 디스코드 명령 권한 판정 (P1). 원본은 `config.json`의 `nodeSettings[guildId]` 하나다
//!
//! 레벨: 소유자(3) > 관리자(2) > 유저(1) > 챗봇(0) > 금지(-1). 숫자는 릴레이(server-chan `commandAccess.ts`)와 같다.
//! 길드마다 단계 사다리(`ladder`)를 바꿀 수 있다: 단계를 더하고(id ≥ 10) 순서를 바꾸고 지운다. 소유자는 늘 맨 위,
//! 금지는 늘 맨 아래다. 높고 낮음은 숫자가 아니라 사다리 순서로 견준다 (`meets`).
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

/// 기본 단계 (높은 것부터). 지운 기본 단계는 이 순서로 가까운 단계를 찾는다
const BUILTIN: [i64; 5] = [OWNER, ADMIN, USER, CHATBOT, BANNED];
/// 더한 단계의 첫 id
pub const CUSTOM_MIN: i64 = 10;
const CUSTOM_MAX: i64 = 999;
const MAX_CUSTOM: usize = 20;
const MAX_NAME: usize = 32;

/// 사다리 한 칸. 기본 단계는 이름이 없다 (로케일 문구). 더한 단계는 이름이 있다
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct Level {
    pub id: i64,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub name: Option<String>,
}

/// 사다리 바꾸기의 한 칸. id가 없으면 새 단계다 (데몬이 번호를 준다)
#[derive(Debug, Clone, PartialEq)]
pub struct LevelInput {
    pub id: Option<i64>,
    pub name: Option<String>,
}

pub fn default_ladder() -> Vec<Level> {
    BUILTIN.iter().map(|&id| Level { id, name: None }).collect()
}

/// 사다리 규칙: 소유자 맨 위, 금지 맨 아래, 그 사이에 하나 이상. id는 겹치지 않고 기본 단계이거나 10 이상.
/// 더한 단계는 10~999 id와 1~32자 이름이 있다
fn check_ladder(levels: &[Level]) -> Result<(), &'static str> {
    let ok = levels.len() >= 3
        && levels.first().map(|l| l.id) == Some(OWNER)
        && levels.last().map(|l| l.id) == Some(BANNED)
        && levels.iter().enumerate().all(|(i, l)| levels[..i].iter().all(|o| o.id != l.id))
        && levels.iter().all(|l| match &l.name {
            None => BUILTIN.contains(&l.id),
            Some(n) => (CUSTOM_MIN..=CUSTOM_MAX).contains(&l.id) && !n.is_empty() && n.chars().count() <= MAX_NAME,
        })
        && levels.iter().filter(|l| l.id >= CUSTOM_MIN).count() <= MAX_CUSTOM;
    ok.then_some(()).ok_or("invalid_ladder")
}

fn parse_ladder(v: Option<&Value>) -> Option<Vec<Level>> {
    let levels: Vec<Level> = v?
        .as_array()?
        .iter()
        .map(|l| {
            let id = l.get("id")?.as_i64()?;
            let name = l.get("name").and_then(Value::as_str).map(str::to_string);
            Some(Level { id, name })
        })
        .collect::<Option<_>>()?;
    check_ladder(&levels).ok().map(|_| levels)
}

pub const PRESETS: [&str; 3] = ["loose", "default", "strict"];

/// 액션(`server.<명령>`)이 따로 있는 내장 명령
// `players`는 `[status] players` 모듈의 채팅 접속자 조회 (`server.players`와 같은 키)
const BUILTIN_COMMANDS: [&str; 5] = ["start", "stop", "restart", "status", "players"];

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
    /// 단계 사다리 (높은 것부터)
    pub ladder: Vec<Level>,
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
            ladder: default_ladder(),
            default_level: USER,
            members: HashMap::new(),
            preset: "default".into(),
            command_levels: HashMap::new(),
            chatbot_cap: CHATBOT,
            legacy_denied: HashMap::new(),
        }
    }
}

impl NodePermissions {
    fn position(&self, id: i64) -> Option<usize> {
        self.ladder.iter().position(|l| l.id == id)
    }

    /// 이 사다리에 있는 단계인가
    pub fn has(&self, id: i64) -> bool {
        self.position(id).is_some()
    }

    /// 멤버에게 줄 수 있는 레벨 (소유자는 줄 수 없다)
    pub fn grantable(&self, level: i64) -> bool {
        level != OWNER && self.has(level)
    }

    /// 명령에 요구할 수 있는 레벨
    pub fn requirable(&self, level: i64) -> bool {
        level != BANNED && self.has(level)
    }

    /// `level`이 `need` 이상인가 (사다리 순서). 사다리에 없는 값은 늘 모자라다
    pub fn meets(&self, level: i64, need: i64) -> bool {
        matches!((self.position(level), self.position(need)), (Some(l), Some(n)) if l <= n)
    }

    /// 지운 기본 단계를 가까운 단계로: 위로(요구 레벨은 엄한 쪽) 또는 아래로(멤버 레벨은 약한 쪽)
    fn nearest_builtin(&self, id: i64, up: bool) -> i64 {
        let at = BUILTIN.iter().position(|&b| b == id).unwrap_or(2);
        let found = if up { BUILTIN[..=at].iter().rev().find(|b| self.has(**b)) } else { BUILTIN[at..].iter().find(|b| self.has(**b)) };
        found.copied().unwrap_or(if up { OWNER } else { BANNED })
    }

    /// 프리셋이 이 risk에 요구하는 레벨 (그 기본 단계를 지웠으면 더 엄한 쪽)
    pub fn preset_need(&self, risk: &str) -> i64 {
        self.nearest_builtin(preset_level(&self.preset, risk), true)
    }

    /// 관리 액션의 요구 레벨 (`actions::admin_level`, 지웠으면 더 엄한 쪽)
    pub fn admin_need(&self) -> i64 {
        self.nearest_builtin(crate::actions::admin_level(&self.preset), true)
    }

    /// `nodeSettings[guildId]` 값에서 읽는다. 범위를 벗어난 값은 버리고 기본값을 쓴다
    pub fn from_node(node: Option<&Value>) -> Self {
        let mut p = Self::default();
        let Some(node) = node.and_then(Value::as_object) else { return p };
        if let Some(ladder) = parse_ladder(node.get("ladder")) {
            p.ladder = ladder;
            p.default_level = p.nearest_builtin(USER, false);
            p.chatbot_cap = p.nearest_builtin(CHATBOT, false);
        }
        let level_in = |v: Option<&Value>, ok: &dyn Fn(i64) -> bool| v.and_then(Value::as_i64).filter(|l| ok(*l));
        if let Some(l) = level_in(node.get("defaultLevel"), &|l| p.grantable(l)) {
            p.default_level = l;
        }
        if let Some(l) = level_in(node.get("chatbotCap"), &|l| p.grantable(l)) {
            p.chatbot_cap = l;
        }
        if let Some(preset) = node.get("preset").and_then(Value::as_str).filter(|s| PRESETS.contains(s)) {
            p.preset = preset.into();
        }
        let members: Vec<(String, i64)> = node
            .get("members")
            .and_then(Value::as_object)
            .into_iter()
            .flatten()
            .filter_map(|(id, l)| level_in(Some(l), &|l| p.grantable(l)).map(|l| (id.clone(), l)))
            .collect();
        let commands: Vec<(String, i64)> = node
            .get("commandLevels")
            .and_then(Value::as_object)
            .into_iter()
            .flatten()
            .filter_map(|(key, l)| level_in(Some(l), &|l| p.requirable(l)).map(|l| (key.clone(), l)))
            .collect();
        p.members.extend(members);
        p.command_levels.extend(commands);
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
        if who.via_chatbot && !self.meets(self.chatbot_cap, level) {
            self.chatbot_cap
        } else {
            level
        }
    }

    /// 명령별 덮어쓰기 (`commandLevels[key]`)
    pub fn override_level(&self, key: &str) -> Option<i64> {
        self.command_levels.get(key).copied()
    }

    /// 이 명령의 요구 레벨. 내장 명령은 `<instance>/<명령>`이 없으면 그 액션 키(`<instance>/server.<명령>`)도 본다:
    /// 액션 경로와 같은 덮어쓰기를 따른다 (D-A2)
    pub fn required(&self, instance_id: &str, command: &str, risk: &str) -> i64 {
        let action_key = || BUILTIN_COMMANDS.contains(&command).then(|| format!("{}/server.{}", instance_id, command));
        self.command_levels
            .get(&format!("{}/{}", instance_id, command))
            .or_else(|| action_key().and_then(|k| self.command_levels.get(&k)))
            .copied()
            .unwrap_or_else(|| self.preset_need(risk))
    }

    fn denied(&self, who: &Requester, need: i64) -> Denied {
        Denied { need, level: self.effective_level(who), via: who.via_chatbot.then_some("chatbot") }
    }

    /// help·list처럼 명령이 아닌 요청: 금지만 막는다
    pub fn check_basic(&self, who: &Requester) -> Result<(), Denied> {
        let level = self.effective_level(who);
        if level == BANNED || !self.has(level) {
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
        if !self.meets(level, need) {
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
            "ladder": self.ladder,
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
    /// 단계 사다리 전체 (높은 것부터, GUI·CLI·웹). 지운 단계의 멤버·기본 레벨·챗봇 상한은 바로 아래 단계로,
    /// 명령 요구 레벨은 바로 위 단계로 옮긴다
    Ladder(Vec<LevelInput>),
}

/// 새 사다리를 저장하고 지운 단계를 쓰던 값을 옮긴다
fn apply_ladder(node: &mut Map<String, Value>, input: &[LevelInput]) -> Result<(), &'static str> {
    let old = NodePermissions::from_node(Some(&Value::Object(node.clone())));
    let mut next_id = old.ladder.iter().map(|l| l.id).chain(input.iter().filter_map(|l| l.id)).max().unwrap_or(0).max(CUSTOM_MIN - 1);
    let ladder: Vec<Level> = input
        .iter()
        .map(|l| {
            let id = l.id.unwrap_or_else(|| {
                next_id += 1;
                next_id
            });
            // 기본 단계 이름은 로케일이 정한다
            let name = (id >= CUSTOM_MIN).then(|| l.name.as_deref().unwrap_or("").trim().to_string());
            Level { id, name }
        })
        .collect();
    check_ladder(&ladder)?;
    let kept = |id: i64| ladder.iter().any(|l| l.id == id);
    let pos = |id: i64| old.position(id).unwrap_or(old.ladder.len() - 1);
    let down = |id: i64| old.ladder[pos(id)..].iter().map(|l| l.id).find(|&l| kept(l)).unwrap_or(BANNED);
    let up = |id: i64| old.ladder[..=pos(id)].iter().rev().map(|l| l.id).find(|&l| kept(l)).unwrap_or(OWNER);
    // 권한을 넓히지 않는다: 멤버는 늘 아래로 (아래에 금지밖에 없으면 금지), 명령은 늘 위로
    let members: Map<String, Value> = old.members.iter().map(|(k, l)| (k.clone(), json!(down(*l)))).collect();
    let commands: Map<String, Value> = old.command_levels.iter().map(|(k, l)| (k.clone(), json!(up(*l)))).collect();
    if !members.is_empty() || node.contains_key("members") {
        node.insert("members".into(), Value::Object(members));
    }
    if !commands.is_empty() || node.contains_key("commandLevels") {
        node.insert("commandLevels".into(), Value::Object(commands));
    }
    node.insert("defaultLevel".into(), json!(down(old.default_level)));
    node.insert("chatbotCap".into(), json!(down(old.chatbot_cap)));
    node.insert("ladder".into(), json!(ladder));
    Ok(())
}

/// `node`(`nodeSettings[guildId]` 객체)에 바꾸기를 적용한다. `owner_id`는 강등·부여할 수 없다
pub fn apply(node: &mut Map<String, Value>, change: &Change, owner_id: Option<&str>) -> Result<(), &'static str> {
    let perms = NodePermissions::from_node(Some(&Value::Object(node.clone())));
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
                Some(l) if !perms.grantable(*l) => return Err("invalid_level"),
                Some(l) => {
                    sub(node, "members").insert(discord_id.clone(), json!(l));
                }
                None => {
                    sub(node, "members").remove(discord_id);
                }
            }
        }
        Change::DefaultLevel(l) | Change::ChatbotCap(l) if !perms.grantable(*l) => return Err("invalid_level"),
        Change::DefaultLevel(l) => {
            node.insert("defaultLevel".into(), json!(l));
        }
        Change::ChatbotCap(l) => {
            node.insert("chatbotCap".into(), json!(l));
        }
        Change::Preset(p) if !PRESETS.contains(&p.as_str()) => return Err("invalid_preset"),
        // 프리셋은 한 번 적용하는 틀이다: 고르면 명령별 레벨을 지워 모두 프리셋 값을 따른다 (사용자 지정 2026-10-25)
        Change::Preset(p) => {
            node.insert("preset".into(), json!(p));
            node.remove("commandLevels");
        }
        Change::Ladder(levels) => apply_ladder(node, levels)?,
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
                Some(l) if !perms.requirable(*l) => return Err("invalid_level"),
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

    // D-A2: 내장 명령은 액션 경로처럼 `<instance>/server.<명령>` 덮어쓰기도 본다. `<instance>/<명령>`이 있으면 그쪽이 먼저다
    #[test]
    fn built_in_commands_also_take_the_level_set_for_their_actions() {
        let p = NodePermissions::from_node(Some(&json!({ "preset": "loose", "defaultLevel": 2, "commandLevels": { "i/server.stop": 3, "i/server.start": 3, "i/start": 1 } })));
        assert_eq!(p.check(&who("x"), "i", "stop", "caution"), Err(Denied { need: OWNER, level: ADMIN, via: None }));
        assert!(p.check(&who("x"), "i", "start", "caution").is_ok(), "the command's own key wins");
        assert!(p.check(&who("x"), "j", "stop", "caution").is_ok(), "only that instance");
        assert_eq!(p.required("i", "kick", "caution"), preset_level("loose", "caution"), "module commands have no action key");
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

    /// 채팅 `players`도 액션 `server.players`의 명령별 레벨을 따른다 (같은 조회에 같은 제한)
    #[test]
    fn chat_players_follows_the_server_players_level() {
        let node = serde_json::json!({ "commandLevels": { "i/server.players": 3 } });
        let perms = NodePermissions::from_node(Some(&node));
        assert_eq!(perms.required("i", "players", "safe"), 3);
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

    fn ladder(ids: &[(i64, Option<&str>)]) -> Change {
        Change::Ladder(ids.iter().map(|(id, name)| LevelInput { id: Some(*id), name: name.map(str::to_string) }).collect())
    }

    // 사용자 지정 2026-10-25: 길드마다 단계를 더하고 순서를 바꾸고 지울 수 있다. 판정은 숫자가 아니라 사다리 순서다
    #[test]
    fn a_guild_ladder_decides_the_order_of_levels() {
        let mut node = Map::new();
        let owner = Some("own");
        // 새 단계 (id 없음) 는 데몬이 10 부터 번호를 준다. 유저 위·관리자 아래
        let add = Change::Ladder(vec![
            LevelInput { id: Some(OWNER), name: None },
            LevelInput { id: Some(ADMIN), name: None },
            LevelInput { id: None, name: Some(" VIP ".into()) },
            LevelInput { id: Some(USER), name: None },
            LevelInput { id: Some(CHATBOT), name: None },
            LevelInput { id: Some(BANNED), name: None },
        ]);
        apply(&mut node, &add, owner).unwrap();
        assert_eq!(node["ladder"][2], json!({ "id": 10, "name": "VIP" }));
        apply(&mut node, &Change::Member { discord_id: "v".into(), level: Some(10) }, owner).unwrap();
        apply(&mut node, &Change::CommandLevel { key: "i/kick".into(), level: Some(10) }, owner).unwrap();
        let p = NodePermissions::from_node(Some(&Value::Object(node.clone())));
        assert!(p.check(&who("v"), "i", "kick", "danger").is_ok());
        assert_eq!(p.check(&who("x"), "i", "kick", "danger"), Err(Denied { need: 10, level: USER, via: None }));
        assert_eq!(p.check(&who("v"), "i", "op", "danger"), Err(Denied { need: ADMIN, level: 10, via: None }));

        // 관리자를 유저 아래로 내리면 관리자는 유저보다 약하다
        apply(&mut node, &ladder(&[(OWNER, None), (10, Some("VIP")), (USER, None), (ADMIN, None), (CHATBOT, None), (BANNED, None)]), owner).unwrap();
        let p = NodePermissions::from_node(Some(&Value::Object(node.clone())));
        assert!(p.meets(USER, ADMIN));
        assert!(!p.meets(ADMIN, USER));
        // 챗봇 상한도 순서로 자른다
        let mut bot = who("v");
        bot.via_chatbot = true;
        assert_eq!(p.effective_level(&bot), CHATBOT);
    }

    #[test]
    fn deleting_a_level_moves_members_down_and_commands_up() {
        let mut node = Map::new();
        let owner = Some("own");
        apply(&mut node, &ladder(&[(OWNER, None), (ADMIN, None), (10, Some("VIP")), (USER, None), (CHATBOT, None), (BANNED, None)]), owner).unwrap();
        for (id, l) in [("v", 10), ("u", USER), ("a", ADMIN)] {
            apply(&mut node, &Change::Member { discord_id: id.into(), level: Some(l) }, owner).unwrap();
        }
        apply(&mut node, &Change::DefaultLevel(10), owner).unwrap();
        apply(&mut node, &Change::CommandLevel { key: "i/kick".into(), level: Some(10) }, owner).unwrap();
        apply(&mut node, &Change::CommandLevel { key: "i/say".into(), level: Some(USER) }, owner).unwrap();
        // VIP 와 유저를 지운다
        apply(&mut node, &ladder(&[(OWNER, None), (ADMIN, None), (CHATBOT, None), (BANNED, None)]), owner).unwrap();
        let p = NodePermissions::from_node(Some(&Value::Object(node.clone())));
        assert_eq!((p.members["v"], p.members["u"], p.members["a"]), (CHATBOT, CHATBOT, ADMIN));
        assert_eq!(p.default_level, CHATBOT);
        assert_eq!((p.command_levels["i/kick"], p.command_levels["i/say"]), (ADMIN, ADMIN));
        // 지워서 권한이 넓어지지 않는다: 아래에 금지밖에 없으면 금지
        let mut low = node.clone();
        apply(&mut low, &ladder(&[(OWNER, None), (ADMIN, None), (BANNED, None)]), owner).unwrap();
        let q = NodePermissions::from_node(Some(&Value::Object(low)));
        assert_eq!((q.members["v"], q.members["a"], q.default_level, q.chatbot_cap), (BANNED, ADMIN, BANNED, BANNED));
        // 지운 기본 단계는 프리셋도 더 엄한 쪽으로 푼다 (기본 프리셋 caution = 유저 → 관리자)
        assert_eq!(p.required("i", "stop", "caution"), ADMIN);
        assert_eq!(p.preset_need("caution"), ADMIN);
        // 지운 단계는 줄 수 없다
        assert_eq!(apply(&mut node, &Change::Member { discord_id: "x".into(), level: Some(USER) }, owner), Err("invalid_level"));
    }

    #[test]
    fn ladders_keep_the_owner_on_top_and_banned_at_the_bottom() {
        let mut node = Map::new();
        let bad = [
            ladder(&[(ADMIN, None), (OWNER, None), (BANNED, None)]),
            ladder(&[(OWNER, None), (BANNED, None), (USER, None)]),
            ladder(&[(OWNER, None), (BANNED, None)]),
            ladder(&[(OWNER, None), (USER, None), (USER, None), (BANNED, None)]),
            ladder(&[(OWNER, None), (10, Some("")), (BANNED, None)]),
            ladder(&[(OWNER, None), (10, None), (BANNED, None)]),
            ladder(&[(OWNER, None), (5, Some("x")), (BANNED, None)]),
            ladder(&[(OWNER, None), (10, Some(&"x".repeat(33))), (BANNED, None)]),
        ];
        for change in &bad {
            assert_eq!(apply(&mut node, change, None), Err("invalid_ladder"), "{:?}", change);
        }
        // 이미 있는 id 를 이름만 바꾼다. 기본 단계 이름은 로케일이라 버린다
        apply(&mut node, &ladder(&[(OWNER, Some("boss")), (10, Some("VIP")), (USER, None), (BANNED, None)]), None).unwrap();
        apply(&mut node, &ladder(&[(OWNER, None), (10, Some("단골")), (USER, None), (BANNED, None)]), None).unwrap();
        let p = NodePermissions::from_node(Some(&Value::Object(node)));
        assert_eq!(p.ladder[0], Level { id: OWNER, name: None });
        assert_eq!(p.ladder[1].name.as_deref(), Some("단골"));
        // 저장된 사다리가 틀렸으면 기본 사다리
        assert_eq!(NodePermissions::from_node(Some(&json!({ "ladder": [{ "id": 2 }] }))).ladder, default_ladder());
    }

    // 사용자 지정 2026-10-25: 프리셋은 한 번 적용하는 틀이다. 고르면 명령별 레벨을 지우고 모두 프리셋 값을 따른다
    #[test]
    fn choosing_a_preset_resets_command_levels() {
        let mut node = Map::new();
        apply(&mut node, &Change::CommandLevel { key: "i/kick".into(), level: Some(OWNER) }, None).unwrap();
        apply(&mut node, &Change::Preset("strict".into()), None).unwrap();
        let p = NodePermissions::from_node(Some(&Value::Object(node)));
        assert!(p.command_levels.is_empty());
        assert_eq!(p.required("i", "kick", "caution"), ADMIN);
    }
}
