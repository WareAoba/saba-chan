//! `server access`: 디스코드에서 이 서버를 누가 쓰는지 (사용 모드, 전용 멤버 명단, 자동 강퇴)
//!
//! GUI 디스코드 › 서버 › 인스턴스 메뉴의 사용 모드와 같은 값이다. 데몬 `GET`·`PATCH /api/instance/:id`의
//! `access`·`controllers`·`controllers_only`·`kick_players`. 닉네임 규칙(길이·겹침)은 데몬이 검사한다.

use crate::ctx::Ctx;
use crate::error::{usage, Result};
use crate::output::{self, s};
use crate::resolve;
use serde_json::{json, Map, Value};

#[derive(Default)]
pub struct AccessChange {
    /// `shared` | `members`
    pub mode: Option<String>,
    /// 명단에 넣을 사람: `닉네임` 또는 `닉네임=디스코드ID` (같은 닉네임이면 디스코드 ID를 바꾼다)
    pub add_members: Vec<String>,
    pub remove_members: Vec<String>,
    /// 전용 멤버: 명단 밖 닉네임을 내보낸다
    pub kick_others: Option<bool>,
    /// 서버 공용: 접속하면 내보낼 닉네임
    pub kick: Vec<String>,
    pub unkick: Vec<String>,
}

impl AccessChange {
    fn is_empty(&self) -> bool {
        self.mode.is_none()
            && self.add_members.is_empty()
            && self.remove_members.is_empty()
            && self.kick_others.is_none()
            && self.kick.is_empty()
            && self.unkick.is_empty()
    }
}

fn same(a: &str, b: &str) -> bool {
    a.trim().to_lowercase() == b.trim().to_lowercase()
}

/// PATCH 본문: 준 칸만 싣는다. 명단은 지금 값에서 시작해 넣고 뺀다 (닉네임은 대소문자 무시)
fn access_patch(info: &Value, change: &AccessChange) -> Result<Map<String, Value>> {
    let mut patch = Map::new();
    if let Some(mode) = &change.mode {
        patch.insert("access".into(), json!(mode));
    }
    if !change.add_members.is_empty() || !change.remove_members.is_empty() {
        let mut list: Vec<Value> = info.get("controllers").and_then(Value::as_array).cloned().unwrap_or_default();
        for name in &change.remove_members {
            let before = list.len();
            list.retain(|c| !same(s(c, "name"), name));
            if list.len() == before {
                return Err(usage(t!("access.not_listed", name = name)));
            }
        }
        for spec in &change.add_members {
            let (name, discord_id) = match spec.split_once('=') {
                Some((n, id)) => (n.trim(), Some(id.trim()).filter(|id| !id.is_empty())),
                None => (spec.trim(), None),
            };
            let mut entry = json!({ "name": name });
            if let Some(id) = discord_id {
                entry["discord_id"] = json!(id);
            }
            match list.iter_mut().find(|c| same(s(c, "name"), name)) {
                Some(c) => *c = entry,
                None => list.push(entry),
            }
        }
        patch.insert("controllers".into(), Value::Array(list));
    }
    if let Some(on) = change.kick_others {
        patch.insert("controllers_only".into(), json!(on));
    }
    if !change.kick.is_empty() || !change.unkick.is_empty() {
        let mut list: Vec<String> = info
            .get("kick_players")
            .and_then(Value::as_array)
            .into_iter()
            .flatten()
            .filter_map(Value::as_str)
            .map(str::to_string)
            .collect();
        for name in &change.unkick {
            let before = list.len();
            list.retain(|n| !same(n, name));
            if list.len() == before {
                return Err(usage(t!("access.not_listed", name = name)));
            }
        }
        for name in &change.kick {
            if !list.iter().any(|n| same(n, name)) {
                list.push(name.trim().to_string());
            }
        }
        patch.insert("kick_players".into(), json!(list));
    }
    Ok(patch)
}

/// 옵션 없이: 사용 모드와 명단을 보여 준다. 바꿀 것이 있으면 지금 값에서 시작해 저장한다
pub async fn run(ctx: &Ctx, server: Option<&str>, change: AccessChange) -> Result<()> {
    let sv = resolve::server(ctx, server).await?;
    let info = ctx.api.instance(&sv.id).await?;
    if change.is_empty() {
        let view = json!({
            "access": info.get("access").and_then(Value::as_str).unwrap_or("shared"),
            "controllers": info.get("controllers").cloned().unwrap_or_else(|| json!([])),
            "controllers_only": info.get("controllers_only").and_then(Value::as_bool).unwrap_or(false),
            "kick_players": info.get("kick_players").cloned().unwrap_or_else(|| json!([])),
        });
        ctx.out.data(&view, |v| {
            let members = s(v, "access") == "members";
            let none = || output::dim(&t!("access.none"));
            let list = |key: &str, show: &dyn Fn(&Value) -> String| {
                let items: Vec<String> = v[key].as_array().into_iter().flatten().map(show).collect();
                if items.is_empty() { none() } else { items.join(", ") }
            };
            let mut rows = vec![(t!("access.mode"), t!(if members { "access.members" } else { "access.shared" }))];
            if members {
                rows.push((
                    t!("access.member_list"),
                    list("controllers", &|c| match c.get("discord_id").and_then(Value::as_str) {
                        Some(id) => format!("{} {}", s(c, "name"), output::dim(&format!("({id})"))),
                        None => s(c, "name").to_string(),
                    }),
                ));
                rows.push((t!("access.kick_others"), output::yes_no(v["controllers_only"].as_bool().unwrap_or(false))));
            } else {
                rows.push((t!("access.kick_list"), list("kick_players", &|n| n.as_str().unwrap_or("").to_string())));
            }
            output::print_kv(&rows);
        });
        return Ok(());
    }
    let patch = access_patch(&info, &change)?;
    let r = ctx.api.update_instance(&sv.id, Value::Object(patch)).await?;
    ctx.out.done(&t!("access.saved", server = sv.name), &r);
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn info() -> Value {
        json!({
            "access": "members",
            "controllers": [{ "name": "Steve", "discord_id": "123456789012345678" }, { "name": "Alex" }],
            "controllers_only": true,
            "kick_players": ["Griefer"],
        })
    }

    /// 넣고 빼면 나머지 명단은 그대로다. 같은 닉네임(대소문자 무시)을 다시 넣으면 디스코드 ID만 바뀐다
    #[test]
    fn members_are_added_and_removed_from_the_current_list() {
        let change = AccessChange { add_members: vec!["alex=987654321098765432".into(), "Notch".into()], remove_members: vec!["STEVE".into()], ..Default::default() };
        let patch = access_patch(&info(), &change).unwrap();
        assert_eq!(Value::Object(patch), json!({ "controllers": [{ "name": "alex", "discord_id": "987654321098765432" }, { "name": "Notch" }] }));
    }

    #[test]
    fn kick_names_are_added_once_and_removed() {
        let change = AccessChange { kick: vec!["griefer".into(), "Troll".into()], ..Default::default() };
        assert_eq!(access_patch(&info(), &change).unwrap()["kick_players"], json!(["Griefer", "Troll"]));
        let change = AccessChange { unkick: vec!["GRIEFER".into()], ..Default::default() };
        assert_eq!(access_patch(&info(), &change).unwrap()["kick_players"], json!([]));
    }

    /// 모드·스위치는 준 것만 싣는다 (명단은 건드리지 않는다)
    #[test]
    fn only_given_values_are_sent() {
        let change = AccessChange { mode: Some("shared".into()), kick_others: Some(false), ..Default::default() };
        assert_eq!(Value::Object(access_patch(&info(), &change).unwrap()), json!({ "access": "shared", "controllers_only": false }));
    }

    /// 명단에 없는 사람을 빼라고 하면 아무것도 저장하지 않고 알린다 (오타)
    #[test]
    fn removing_someone_not_listed_is_refused() {
        crate::i18n::init("en");
        let change = AccessChange { remove_members: vec!["Herobrine".into()], ..Default::default() };
        assert!(access_patch(&info(), &change).is_err());
        let change = AccessChange { unkick: vec!["Herobrine".into()], ..Default::default() };
        assert!(access_patch(&info(), &change).is_err());
    }
}
