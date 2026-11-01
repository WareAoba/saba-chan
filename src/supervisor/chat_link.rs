//! 채팅 연동 (`/채팅연동`, 2026-11-01 사용자 요청): 디스코드 채널 하나와 켜진 서버의 게임 채팅을 잇는다.
//!
//! - 액션 `server.chat_link`(`POST /api/instance/:id/chat-link/:guild {channel, direction?}`)이 켜고 끈다. 한 길드는 한 서버에 연동 하나다.
//!   같은 채널에서 다시 부르면 끄고, 다른 방향을 주면 방향만 바꾼다. 다른 채널에서 부르면 그 채널로 옮긴다.
//! - 켜려면 서버가 켜져 있고 접속·퇴장 줄로 센 접속자가 1명 이상이어야 한다 (주기적인 접속자 명령은 보지 않는다).
//!   마지막 사람이 나가거나 프로세스가 끝나면 끄고 `instance.chat_link {instanceId, instance, guild, channel, active: false, reason}`을 낸다.
//! - 게임 → 디스코드: `player_events`가 채팅 줄마다 [`to_discord`]의 채널을 `instance.chat`의 `links`에 싣는다 ([`chat_event`]).
//!   인스턴스 설정 `game_chat`이 꺼져 있어도 간다.
//! - 디스코드 → 게임: 릴레이가 하트비트 서버 항목의 `chatLinks`([`list`])로 채널을 알고 액션 `server.chat`으로 보낸다.
//! - 상태는 메모리에만 둔다. 데몬이 다시 켜지면 콘솔을 처음부터 읽지 못해 접속자를 모르므로 연동도 없다.

use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use std::collections::HashMap;
use std::sync::{Mutex, OnceLock};

/// 연동 방향
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Direction {
    #[default]
    Both,
    ToDiscord,
    ToGame,
}

impl Direction {
    pub fn to_discord(self) -> bool {
        self != Direction::ToGame
    }

    pub fn to_game(self) -> bool {
        self != Direction::ToDiscord
    }

    /// 입력 값 (없음·null·""는 정하지 않음)
    pub fn parse(value: Option<&Value>) -> Result<Option<Self>, &'static str> {
        match value {
            None | Some(Value::Null) => Ok(None),
            Some(Value::String(s)) if s.trim().is_empty() => Ok(None),
            Some(v) => serde_json::from_value(v.clone()).map(Some).map_err(|_| "direction must be both, to_discord or to_game"),
        }
    }
}

/// 연동 하나: 길드의 채널과 방향
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct Link {
    pub guild: String,
    pub channel: String,
    pub direction: Direction,
}

/// 부른 결과
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum Outcome {
    On,
    Off,
    Changed,
}

/// 디스코드 ID (숫자 15~21자리)
pub fn is_discord_id(s: &str) -> bool {
    (15..=21).contains(&s.len()) && s.bytes().all(|b| b.is_ascii_digit())
}

fn links() -> &'static Mutex<HashMap<String, Vec<Link>>> {
    static LINKS: OnceLock<Mutex<HashMap<String, Vec<Link>>>> = OnceLock::new();
    LINKS.get_or_init(Default::default)
}

/// 켜고 끈다. 켜거나 방향을 바꿀 때만 `check`(새 방향)를 부르고, 실패하면 아무것도 바꾸지 않는다.
/// 돌려준 방향은 끈 경우 끄기 전 방향이다
pub fn toggle<E>(instance_id: &str, guild: &str, channel: &str, direction: Option<Direction>, check: impl FnOnce(Direction) -> Result<(), E>) -> Result<(Outcome, Direction), E> {
    let mut map = links().lock().unwrap_or_else(|e| e.into_inner());
    let list = map.entry(instance_id.to_string()).or_default();
    let existing = list.iter().position(|l| l.guild == guild);
    let result = match existing.map(|i| (i, list[i].clone())) {
        Some((i, old)) if old.channel == channel && direction.is_none_or(|d| d == old.direction) => {
            list.remove(i);
            (Outcome::Off, old.direction)
        }
        Some((i, old)) if old.channel == channel => {
            let d = direction.unwrap_or_default();
            check(d)?;
            list[i].direction = d;
            (Outcome::Changed, d)
        }
        _ => {
            let d = direction.unwrap_or_default();
            check(d)?;
            list.retain(|l| l.guild != guild);
            list.push(Link { guild: guild.to_string(), channel: channel.to_string(), direction: d });
            (Outcome::On, d)
        }
    };
    if list.is_empty() {
        map.remove(instance_id);
    }
    Ok(result)
}

/// 그 서버의 연동 (하트비트 `chatLinks`)
pub fn list(instance_id: &str) -> Vec<Link> {
    links().lock().unwrap_or_else(|e| e.into_inner()).get(instance_id).cloned().unwrap_or_default()
}

/// 게임 채팅을 받을 연동 채널 (`instance.chat`의 `links`)
pub fn to_discord(instance_id: &str) -> Vec<Value> {
    list(instance_id).into_iter().filter(|l| l.direction.to_discord()).map(|l| json!({ "guild": l.guild, "channel": l.channel })).collect()
}

/// 그 서버의 연동을 모두 끄고 연동마다 `instance.chat_link`를 낸다 (`reason`: `empty` 모두 나감, `stopped` 프로세스 끝남)
pub fn end(instance_id: &str, instance: &str, reason: &str) {
    let removed = links().lock().unwrap_or_else(|e| e.into_inner()).remove(instance_id).unwrap_or_default();
    for link in removed {
        crate::events::emit("instance.chat_link", json!({ "instanceId": instance_id, "instance": instance, "guild": link.guild, "channel": link.channel, "active": false, "reason": reason }));
    }
}

/// 채팅 한 줄의 `instance.chat` 이벤트. 게임 채팅 채널에 보낼 줄(`text`, 인스턴스 설정 `game_chat`이 고른 것)과
/// 연동 채널에 보낼 줄(`linkText`, 채팅 전체)을 따로 싣는다. 둘 다 없으면 None
pub fn chat_event(instance_id: &str, instance: &str, player: &str, message: &str, settings: &super::game_chat::GameChat, links: Vec<Value>) -> Option<Value> {
    let text = settings.pick(message).map(|t| super::game_chat::compose(instance, player, t));
    let whole = message.trim();
    let link_text = (!links.is_empty() && !whole.is_empty()).then(|| super::game_chat::compose(instance, player, whole));
    if text.is_none() && link_text.is_none() {
        return None;
    }
    let mut ev = json!({ "instanceId": instance_id, "instance": instance });
    if let Some(text) = text {
        ev["text"] = json!(text);
    }
    if let Some(link_text) = link_text {
        ev["links"] = json!(links);
        ev["linkText"] = json!(link_text);
    }
    Some(ev)
}

#[cfg(test)]
mod tests {
    use super::super::game_chat::{GameChat, Mode};
    use super::*;

    const G: &str = "100000000000000001";
    const C1: &str = "200000000000000001";
    const C2: &str = "200000000000000002";

    fn ok(_: Direction) -> Result<(), &'static str> {
        Ok(())
    }

    // 사용자 요청(2026-11-01): 같은 명령으로 켜고 끈다. 방향을 바꿔 부르면 끄지 않고 방향만 바꾼다
    #[test]
    fn the_same_call_toggles_and_another_direction_only_changes_it() {
        let id = "link-toggle";
        assert_eq!(toggle(id, G, C1, None, ok), Ok((Outcome::On, Direction::Both)));
        assert_eq!(toggle(id, G, C1, Some(Direction::ToDiscord), ok), Ok((Outcome::Changed, Direction::ToDiscord)));
        assert_eq!(list(id), vec![Link { guild: G.into(), channel: C1.into(), direction: Direction::ToDiscord }]);
        assert_eq!(toggle(id, G, C1, Some(Direction::ToDiscord), ok), Ok((Outcome::Off, Direction::ToDiscord)));
        assert!(list(id).is_empty());
        toggle(id, G, C1, Some(Direction::ToGame), ok).unwrap();
        assert_eq!(toggle(id, G, C1, None, ok), Ok((Outcome::Off, Direction::ToGame)), "no direction turns it off");
    }

    #[test]
    fn calling_from_another_channel_moves_the_guilds_link() {
        let id = "link-move";
        toggle(id, G, C1, None, ok).unwrap();
        assert_eq!(toggle(id, G, C2, None, ok), Ok((Outcome::On, Direction::Both)));
        assert_eq!(list(id).len(), 1);
        assert_eq!(list(id)[0].channel, C2);
        toggle(id, "100000000000000002", C1, None, ok).unwrap();
        assert_eq!(list(id).len(), 2, "another guild keeps its own link");
    }

    // 켤 수 없으면(접속자 없음 등) 아무것도 바꾸지 않는다. 끄기는 따지지 않는다
    #[test]
    fn a_failed_check_changes_nothing_and_turning_off_is_not_checked() {
        let id = "link-check";
        let deny = |_: Direction| Err("no_players");
        assert_eq!(toggle(id, G, C1, None, deny), Err("no_players"));
        assert!(list(id).is_empty());
        toggle(id, G, C1, None, ok).unwrap();
        assert_eq!(toggle(id, G, C1, Some(Direction::ToGame), deny), Err("no_players"));
        assert_eq!(list(id)[0].direction, Direction::Both);
        assert_eq!(toggle(id, G, C1, None, deny), Ok((Outcome::Off, Direction::Both)));
    }

    #[test]
    fn only_links_that_receive_game_chat_are_carried_and_end_clears_all() {
        let id = "link-end";
        toggle(id, G, C1, Some(Direction::ToGame), ok).unwrap();
        assert!(to_discord(id).is_empty());
        toggle(id, "100000000000000002", C2, None, ok).unwrap();
        assert_eq!(to_discord(id), vec![json!({ "guild": "100000000000000002", "channel": C2 })]);
        end(id, "s", "empty");
        assert!(list(id).is_empty());
    }

    // 게임 채팅 전달을 꺼 둔 서버도 연동 채널로는 채팅 전체가 간다. 감지 단어 모드의 게임 채팅 채널 줄은 단어를 뺀다
    #[test]
    fn the_event_carries_the_game_chat_line_and_the_whole_line_for_links_separately() {
        let off = GameChat::default();
        assert_eq!(chat_event("i", "s", "Steve", "hi", &off, vec![]), None);
        let links = vec![json!({ "guild": G, "channel": C1 })];
        let ev = chat_event("i", "s", "Steve", " hi ", &off, links.clone()).unwrap();
        assert_eq!(ev, json!({ "instanceId": "i", "instance": "s", "links": links, "linkText": "\\[s\\] **Steve**: hi" }));
        let kw = GameChat { mode: Mode::Keyword, keyword: "!d".into() };
        let ev = chat_event("i", "s", "Steve", "!d hi", &kw, links).unwrap();
        assert_eq!(ev["text"], "\\[s\\] **Steve**: hi");
        assert_eq!(ev["linkText"], "\\[s\\] **Steve**: !d hi");
        let ev = chat_event("i", "s", "Steve", "hi", &GameChat { mode: Mode::All, keyword: String::new() }, vec![]).unwrap();
        assert_eq!(ev, json!({ "instanceId": "i", "instance": "s", "text": "\\[s\\] **Steve**: hi" }), "without links the event is as before");
    }

    #[test]
    fn inputs_are_checked() {
        assert_eq!(Direction::parse(None), Ok(None));
        assert_eq!(Direction::parse(Some(&json!(""))), Ok(None));
        assert_eq!(Direction::parse(Some(&json!("to_game"))), Ok(Some(Direction::ToGame)));
        assert!(Direction::parse(Some(&json!("sideways"))).is_err());
        assert!(is_discord_id(G));
        assert!(!is_discord_id("12a45678901234567"));
        assert!(!is_discord_id("123"));
    }
}
