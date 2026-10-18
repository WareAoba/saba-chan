//! 익스텐션이 화면에 끼워 넣는 데이터 자리 (Q-7, GUI와 같은 이름)
//!
//! 데이터만 받는다 (코드 실행 없음). 조건 판정과 값 찾기는 데몬이 한 번만 한다:
//! - `ServerCard.badge`    서버마다 `GET /api/servers`의 `slots` → `[{extension, text}]` — 서버 목록 이름 옆
//! - `ServerDetail.status` 서버마다 `slots` → `[{extension, label, value}]` — 서버 정보에 줄 추가
//! - `AddServer.options`   `GET /api/extensions`의 `slots` → `[{field, type, label, default?}]` — 서버 추가 때 물어보고
//!   만들 때 `extension_data`로 보낸다

use serde_json::Value;

/// 서버 하나의 데몬이 판정한 slot 항목들
fn server_items<'a>(server: &'a Value, slot: &str) -> impl Iterator<Item = &'a Value> + 'a {
    server.get("slots").and_then(|s| s.get(slot)).and_then(Value::as_array).into_iter().flatten()
}

/// 서버 목록 뱃지들
pub fn badges(server: &Value) -> Vec<String> {
    server_items(server, "ServerCard.badge").filter_map(|e| e.get("text").and_then(Value::as_str).map(str::to_string)).collect()
}

/// 서버 정보에 더할 (라벨, 값)
pub fn detail_rows(server: &Value) -> Vec<(String, String)> {
    server_items(server, "ServerDetail.status")
        .filter_map(|e| Some((e.get("label")?.as_str()?.to_string(), e.get("value")?.as_str()?.to_string())))
        .collect()
}

/// 서버 추가 때 물어볼 칸 (`GET /api/extensions`의 켜진 익스텐션)
pub fn create_options(list: &[Value]) -> Vec<Value> {
    list.iter()
        .filter(|e| e.get("enabled").and_then(Value::as_bool) == Some(true))
        .filter_map(|ext| ext.pointer("/slots/AddServer.options").and_then(Value::as_array))
        .flatten()
        .cloned()
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn rows_come_from_the_daemon_slots() {
        let server = json!({ "slots": {
            "ServerCard.badge": [{ "extension": "box", "text": "📦" }],
            "ServerDetail.status": [{ "extension": "box", "label": "Box", "value": "up" }]
        } });
        assert_eq!(badges(&server), vec!["📦"]);
        assert_eq!(detail_rows(&server), vec![("Box".to_string(), "up".to_string())]);
        assert!(badges(&json!({})).is_empty());
    }

    #[test]
    fn create_options_only_from_enabled_extensions() {
        let list = vec![
            json!({ "id": "box", "enabled": true, "slots": { "AddServer.options": [{ "field": "boxed", "type": "boolean", "label": "Box" }] } }),
            json!({ "id": "off", "enabled": false, "slots": { "AddServer.options": [{ "field": "x" }] } }),
        ];
        let opts = create_options(&list);
        assert_eq!(opts, vec![json!({ "field": "boxed", "type": "boolean", "label": "Box" })]);
    }
}
