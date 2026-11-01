//! 명령 결과를 보일 모양(`output`)으로 — 앱·웹·CLI·디스코드가 같은 결과를 그린다
//!
//! 서버가 값으로 답하는 명령(REST 조회: 정보·지표·설정)은 답이 객체라 클라이언트마다 다르게 풀거나
//! "실행됨"으로 숨겼다. 데몬이 껍데기(`url`·`status`·`response_text`)를 벗기고, 비밀 칸을 가리고,
//! 칸 이름을 붙여 셋 중 한 모양으로 준다:
//! - `{ "text": "..." }` — 글자로 온 답
//! - `{ "fields": [{ "key", "label", "value" }] }` — 이름: 값 목록
//! - `{ "table": { "key"?, "label"?, "columns": [{ "key", "label" }], "rows": [["..."]] } }` — 목록(표)
//!
//! 값은 모두 문자열이다. 보일 것이 없으면(빈 답) `output`이 없다. 콘솔 명령의 답은 서버 로그 줄이라 그대로 둔다.

use crate::supervisor::module_loader::SettingField;
use serde_json::{json, Map, Value};
use std::collections::HashMap;

/// 칸 이름: 모듈 locales `commands.<명령>.output.<키>` → 같은 키를 `file_key`로 가진 설정의 이름
/// (locales `settings.<이름>.label` → module.toml `label`). 보는 언어에 없으면 영어 문구를 쓴다
pub fn labels(command: &str, settings: &[SettingField], locale: &Value, fallback: &Value) -> HashMap<String, String> {
    let text = |pointer: &str| {
        locale
            .pointer(pointer)
            .and_then(Value::as_str)
            .or_else(|| fallback.pointer(pointer).and_then(Value::as_str))
            .map(str::to_string)
    };
    let mut out = HashMap::new();
    for field in settings {
        if let Some(key) = field.file_key.as_deref().filter(|k| !k.is_empty()) {
            let label = text(&format!("/settings/{}/label", field.name)).unwrap_or_else(|| field.label.clone());
            out.insert(key.to_string(), label);
        }
    }
    for source in [fallback, locale] {
        if let Some(map) = source
            .pointer(&format!("/commands/{}/output", command))
            .and_then(Value::as_object)
        {
            for (key, label) in map {
                if let Some(label) = label.as_str() {
                    out.insert(key.clone(), label.to_string());
                }
            }
        }
    }
    out
}

/// `/command` 응답 본문에서 보일 결과. 서버가 값으로 답한 REST 명령만 만든다
pub fn build(body: &Value, labels: &HashMap<String, String>) -> Option<Value> {
    if body.get("protocol").and_then(Value::as_str) != Some("rest") {
        return None;
    }
    let response = body.get("data")?.get("response")?;
    match response {
        Value::Object(map) if map.len() == 1 && map.get("raw").is_some_and(Value::is_string) => {
            text(map["raw"].as_str().unwrap_or_default())
        }
        Value::Object(map) if map.is_empty() => None,
        Value::Object(map) => match map.iter().next() {
            Some((key, Value::Array(items))) if map.len() == 1 => Some(table(Some(key), items, labels)),
            _ => Some(fields(map, labels)),
        },
        Value::Array(items) => Some(table(None, items, labels)),
        Value::String(s) => text(s),
        Value::Null => None,
        other => text(&other.to_string()),
    }
}

fn text(s: &str) -> Option<Value> {
    (!s.trim().is_empty()).then(|| json!({ "text": s.trim_end() }))
}

fn label_of(key: &str, labels: &HashMap<String, String>) -> String {
    labels.get(key).cloned().unwrap_or_else(|| key.to_string())
}

/// 이름: 값. 이름이 붙은 칸이 먼저, 그다음 이름이 없는 칸 (각각 키 순)
fn fields(map: &Map<String, Value>, labels: &HashMap<String, String>) -> Value {
    let mut keys: Vec<&String> = map.keys().collect();
    keys.sort_by_key(|k| !labels.contains_key(k.as_str()));
    let items: Vec<Value> = keys
        .into_iter()
        .map(|k| json!({ "key": k, "label": label_of(k, labels), "value": cell(k, &map[k]) }))
        .collect();
    json!({ "fields": items })
}

/// 줄 이름으로 앞에 두는 열
const ROW_NAMES: [&str; 2] = ["name", "id"];

fn table(key: Option<&String>, items: &[Value], labels: &HashMap<String, String>) -> Value {
    let mut columns: Vec<String> = Vec::new();
    for item in items {
        match item {
            Value::Object(row) => {
                for k in row.keys() {
                    if !columns.contains(k) {
                        columns.push(k.clone());
                    }
                }
            }
            _ if !columns.iter().any(String::is_empty) => columns.push(String::new()),
            _ => {}
        }
    }
    // 줄을 가리키는 열(name·id)이 앞에, 나머지는 키 순
    columns.sort_by_key(|k| {
        ROW_NAMES
            .iter()
            .position(|n| k.eq_ignore_ascii_case(n))
            .unwrap_or(ROW_NAMES.len())
    });
    let rows: Vec<Value> = items
        .iter()
        .map(|item| {
            let row: Vec<Value> = columns
                .iter()
                .map(|c| match item {
                    Value::Object(m) => Value::String(m.get(c).map(|v| cell(c, v)).unwrap_or_default()),
                    other if c.is_empty() => Value::String(cell("", other)),
                    _ => Value::String(String::new()),
                })
                .collect();
            Value::Array(row)
        })
        .collect();
    let columns: Vec<Value> = columns
        .iter()
        .map(|c| json!({ "key": c, "label": label_of(c, labels) }))
        .collect();
    let mut out = json!({ "columns": columns, "rows": rows });
    if let Some(key) = key {
        out["key"] = json!(key);
        out["label"] = json!(label_of(key, labels));
    }
    json!({ "table": out })
}

/// 값 하나를 보일 글자로. 이름이 비밀스러운 칸(password·token 등)은 가린다
fn cell(key: &str, v: &Value) -> String {
    if crate::audit::sensitive(key) && !v.is_null() {
        return "***".into();
    }
    match v {
        Value::String(s) => s.clone(),
        Value::Null => "-".into(),
        other => other.to_string(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn rest(response: Value) -> Value {
        json!({ "success": true, "protocol": "rest", "data": { "status": 200, "url": "http://127.0.0.1:8212/v1/api/metrics", "method": "GET", "response": response, "response_text": "..." } })
    }

    #[test]
    fn rest_object_becomes_named_fields_with_secrets_masked() {
        let labels = HashMap::from([("serverfps".to_string(), "서버 FPS".to_string())]);
        let out = build(
            &rest(json!({ "serverfps": 59, "uptime": 7234, "AdminPassword": "hunter2", "Difficulty": "None" })),
            &labels,
        )
        .unwrap();
        assert_eq!(
            out,
            json!({ "fields": [
                { "key": "serverfps", "label": "서버 FPS", "value": "59" },
                { "key": "AdminPassword", "label": "AdminPassword", "value": "***" },
                { "key": "Difficulty", "label": "Difficulty", "value": "None" },
                { "key": "uptime", "label": "uptime", "value": "7234" },
            ] })
        );
    }

    #[test]
    fn single_list_becomes_table() {
        let labels = HashMap::from([
            ("players".to_string(), "플레이어".to_string()),
            ("name".to_string(), "이름".to_string()),
        ]);
        let out = build(
            &rest(json!({ "players": [{ "name": "Kim", "level": 24 }, { "name": "Lee", "ping": 51.5 }] })),
            &labels,
        )
        .unwrap();
        assert_eq!(
            out,
            json!({ "table": {
                "key": "players", "label": "플레이어",
                "columns": [{ "key": "name", "label": "이름" }, { "key": "level", "label": "level" }, { "key": "ping", "label": "ping" }],
                "rows": [["Kim", "24", ""], ["Lee", "", "51.5"]],
            } })
        );
        // 빈 목록도 표다 (몇 명인지가 답이다)
        let empty = build(&rest(json!({ "players": [] })), &labels).unwrap();
        assert_eq!(empty["table"]["rows"], json!([]));
    }

    #[test]
    fn empty_or_non_rest_answers_have_no_output() {
        let none = HashMap::new();
        assert_eq!(build(&rest(json!({ "raw": "" })), &none), None);
        assert_eq!(build(&rest(json!({})), &none), None);
        assert_eq!(
            build(&rest(json!({ "raw": "Saved\n" })), &none),
            Some(json!({ "text": "Saved" }))
        );
        // 콘솔 명령의 답(서버 로그 줄)은 지금처럼 클라이언트가 보인다
        let console = json!({ "protocol": "stdin", "data": { "response": "Saved the game", "executed": "save-all" } });
        assert_eq!(build(&console, &none), None);
    }

    #[test]
    fn labels_come_from_command_output_then_settings_file_keys() {
        let settings = vec![
            SettingField {
                name: "max_players".into(),
                label: "Max Players".into(),
                file_key: Some("ServerPlayerMaxNum".into()),
                ..Default::default()
            },
            SettingField {
                name: "server_name".into(),
                label: "Server Name".into(),
                file_key: Some("ServerName".into()),
                ..Default::default()
            },
            SettingField {
                name: "rest_host".into(),
                label: "REST Host".into(),
                ..Default::default()
            },
        ];
        let ko = json!({ "settings": { "max_players": { "label": "최대 플레이어 수" } }, "commands": { "metrics": { "output": { "serverfps": "서버 FPS" } } } });
        let en =
            json!({ "commands": { "metrics": { "output": { "serverfps": "Server FPS", "uptime": "Uptime (s)" } } } });
        let got = labels("metrics", &settings, &ko, &en);
        assert_eq!(
            got.get("ServerPlayerMaxNum").map(String::as_str),
            Some("최대 플레이어 수")
        );
        assert_eq!(got.get("ServerName").map(String::as_str), Some("Server Name"));
        assert_eq!(got.get("serverfps").map(String::as_str), Some("서버 FPS"));
        assert_eq!(got.get("uptime").map(String::as_str), Some("Uptime (s)"));
        assert_eq!(got.len(), 4);
    }
}
