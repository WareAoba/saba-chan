//! 익스텐션이 CLI 화면에 끼워 넣는 자리 (manifest `cli.slots`)
//!
//! 데이터만 받는다 (코드 실행 없음). 지원하는 자리:
//! - `InstanceList.badge`     `{text, condition?}` — 서버 목록 이름 옆 표시
//! - `InstanceDetail.status`  `[{label, value_from}]` — 서버 정보에 줄 추가 (익스텐션 상태 → extension_data 순으로 찾음)
//! - `CreateInstance.options` `[{field, type, label, default?}]` — 서버 추가 때 물어보고 `extension_data`로 저장
//!
//! `condition`은 `instance.ext_data.<키>`가 참인지 본다. 새 자리는 `Slots`에 메서드 하나를 더하면 된다.

use serde_json::Value;

#[derive(Clone, Debug)]
pub struct SlotEntry {
    pub extension: String,
    pub data: Value,
}

#[derive(Clone, Debug, Default)]
pub struct Slots {
    entries: Vec<(String, SlotEntry)>,
}

impl Slots {
    /// `GET /api/extensions`의 목록에서 켜진 익스텐션의 `cli.slots`를 모은다
    pub fn from_extensions(list: &[Value]) -> Self {
        let mut entries = Vec::new();
        for ext in list
            .iter()
            .filter(|e| e.get("enabled").and_then(Value::as_bool) == Some(true))
        {
            let id = ext.get("id").and_then(Value::as_str).unwrap_or("").to_string();
            if let Some(slots) = ext.pointer("/cli/slots").and_then(Value::as_object) {
                for (slot, data) in slots {
                    // 배열이면 항목마다, 아니면 하나로
                    let items: Vec<Value> = match data {
                        Value::Array(a) => a.clone(),
                        other => vec![other.clone()],
                    };
                    for item in items {
                        entries.push((
                            slot.clone(),
                            SlotEntry {
                                extension: id.clone(),
                                data: item,
                            },
                        ));
                    }
                }
            }
        }
        Self { entries }
    }

    fn at<'a>(&'a self, slot: &'a str) -> impl Iterator<Item = &'a SlotEntry> + 'a {
        self.entries.iter().filter(move |(s, _)| s == slot).map(|(_, e)| e)
    }

    /// 서버 목록 뱃지들
    pub fn badges(&self, server: &Value) -> Vec<String> {
        self.at("InstanceList.badge")
            .filter(|e| condition_holds(e.data.get("condition").and_then(Value::as_str), server))
            .filter_map(|e| e.data.get("text").and_then(Value::as_str).map(str::to_string))
            .collect()
    }

    /// 서버 정보에 더할 (라벨, 값)
    pub fn detail_rows(&self, server: &Value) -> Vec<(String, String)> {
        self.at("InstanceDetail.status")
            .filter_map(|e| {
                let label = e.data.get("label").and_then(Value::as_str)?.to_string();
                let key = e.data.get("value_from").and_then(Value::as_str)?;
                let value = server
                    .pointer(&format!("/extension_status/{}/{}", e.extension, key))
                    .or_else(|| server.pointer(&format!("/extension_status/default/{}", key)))
                    .or_else(|| server.pointer(&format!("/extension_data/{}", key)))?;
                Some((label, crate::output::show(value)))
            })
            .collect()
    }

    /// 서버 추가 때 물어볼 칸
    pub fn create_options(&self) -> Vec<SlotEntry> {
        self.at("CreateInstance.options").cloned().collect()
    }
}

/// `instance.ext_data.<키>` — 값이 참(true, 0이 아닌 수, 비어 있지 않은 문자열)이면 참. 조건이 없으면 항상 참.
fn condition_holds(cond: Option<&str>, server: &Value) -> bool {
    let Some(cond) = cond else { return true };
    let Some(key) = cond.strip_prefix("instance.ext_data.") else {
        return false;
    };
    match server.pointer(&format!("/extension_data/{}", key)) {
        Some(Value::Bool(b)) => *b,
        Some(Value::Number(n)) => n.as_f64() != Some(0.0),
        Some(Value::String(s)) => !s.is_empty(),
        _ => false,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn exts() -> Vec<Value> {
        vec![
            json!({ "id": "box", "enabled": true, "cli": { "slots": {
                "InstanceList.badge": { "text": "📦", "condition": "instance.ext_data.boxed" },
                "InstanceDetail.status": [ { "label": "Box", "value_from": "box_state" } ]
            } } }),
            json!({ "id": "off", "enabled": false, "cli": { "slots": { "InstanceList.badge": { "text": "X" } } } }),
        ]
    }

    #[test]
    fn badge_follows_condition_and_enabled_flag() {
        let slots = Slots::from_extensions(&exts());
        assert_eq!(
            slots.badges(&json!({ "extension_data": { "boxed": true } })),
            vec!["📦"]
        );
        assert!(slots.badges(&json!({ "extension_data": {} })).is_empty());
    }

    #[test]
    fn detail_rows_read_extension_status() {
        let slots = Slots::from_extensions(&exts());
        let server = json!({ "extension_status": { "box": { "box_state": "up" } } });
        assert_eq!(slots.detail_rows(&server), vec![("Box".to_string(), "up".to_string())]);
    }
}
