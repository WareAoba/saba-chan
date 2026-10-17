//! `server.properties` (혹은 유사 key-value) 편집 화면.
//!
//! 데몬의 `read_properties` / `write_properties` API가 반환하는 JSON 객체와
//! 에디터 필드(`EditorField`) 표현 사이를 변환하는 순수 헬퍼를 제공한다.
//!
//! 실제 비동기 로드는 `mod.rs`의 `load_server_properties`가 처리한다.

use std::collections::HashMap;

use serde_json::{Map, Value};

/// properties 값(`Value`) 하나를 편집 가능한 문자열 표현으로 변환.
///
/// - `Bool(true/false)` → `"true"` / `"false"`
/// - 정수/실수 → 10진수 문자열
/// - `String` → 원본 문자열
/// - `Null` → 빈 문자열
/// - 그 외(배열/객체)는 디버그 직렬화
fn property_to_display(value: &Value) -> String {
    match value {
        Value::Null => String::new(),
        Value::String(s) => s.clone(),
        Value::Bool(b) => b.to_string(),
        Value::Number(n) => n.to_string(),
        other => other.to_string(),
    }
}

/// 문자열을 원래 타입에 최대한 근접하게 역직렬화한다.
///
/// 사용자가 편집한 값은 모두 `String`으로 들어오지만, `server.properties`는
/// bool/int를 타입으로 기대하는 키가 많아서 저장 시 복원이 필요하다.
///
/// - `"true"` / `"false"` → `Bool`
/// - `"123"` → `Number(i64)` (단, 숫자 범위 초과 시 `f64` fallback)
/// - `"3.14"` → `Number(f64)`
/// - 그 외 → `String`
/// - 빈 문자열 → `String("")` (Null 대신 빈 문자열을 유지 — properties 규칙)
fn property_from_display(raw: &str) -> Value {
    if raw == "true" {
        return Value::Bool(true);
    }
    if raw == "false" {
        return Value::Bool(false);
    }
    if let Ok(i) = raw.parse::<i64>() {
        // "0123" 같은 건 숫자로 파싱되지만 문자열로 보존해야 하는 키도 있음.
        // properties는 보통 leading zero가 없으므로 그대로 숫자 처리.
        return Value::Number(i.into());
    }
    if let Ok(f) = raw.parse::<f64>() {
        if let Some(n) = serde_json::Number::from_f64(f) {
            return Value::Number(n);
        }
    }
    Value::String(raw.to_string())
}

/// 타입 문자열을 추론한다 (UI에서 토글/숫자 힌트용).
fn infer_type(value: &Value) -> &'static str {
    match value {
        Value::Bool(_) => "bool",
        Value::Number(_) => "number",
        _ => "text",
    }
}

/// `read_properties` 응답(`{"key": value, ...}`)을 `EDITOR_FIELD:` 인코딩
/// 라인 목록으로 변환한다.
///
/// 키는 알파벳 순으로 정렬되어 매 호출마다 같은 순서로 렌더링된다.
///
/// 형식: `key|value|group|type|label|required|options`
pub fn properties_to_editor_lines(data: &Value) -> Vec<String> {
    let Some(obj) = data.as_object() else {
        return Vec::new();
    };

    let mut pairs: Vec<(&String, &Value)> = obj.iter().collect();
    pairs.sort_by(|a, b| a.0.cmp(b.0));

    pairs
        .into_iter()
        .map(|(key, value)| {
            let display = property_to_display(value);
            let ty = infer_type(value);
            // label은 key 그대로 노출, group은 "properties" 하나로 통일.
            format!("{}|{}|properties|{}|{}|false|", key, display, ty, key)
        })
        .collect()
}

/// 에디터 변경사항(`editor_changes`: key → new string value) 맵을
/// `write_properties`가 기대하는 JSON 객체로 복원한다.
pub fn editor_changes_to_properties_json(changes: &HashMap<String, String>) -> Value {
    let mut map = Map::with_capacity(changes.len());
    for (key, raw) in changes {
        map.insert(key.clone(), property_from_display(raw));
    }
    Value::Object(map)
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn properties_to_editor_lines_sorted_alphabetically() {
        let data = json!({
            "zoo": "a",
            "apple": "b",
            "middle": "c",
        });
        let lines = properties_to_editor_lines(&data);
        assert_eq!(lines.len(), 3);
        assert!(lines[0].starts_with("apple|"));
        assert!(lines[1].starts_with("middle|"));
        assert!(lines[2].starts_with("zoo|"));
    }

    #[test]
    fn properties_to_editor_lines_handles_bool_int_string() {
        let data = json!({
            "pvp": true,
            "max-players": 20,
            "motd": "Hello world",
            "empty-key": null,
        });
        let lines = properties_to_editor_lines(&data);
        // 알파벳 정렬: empty-key, max-players, motd, pvp
        assert_eq!(lines.len(), 4);
        assert_eq!(lines[0], "empty-key||properties|text|empty-key|false|");
        assert_eq!(lines[1], "max-players|20|properties|number|max-players|false|");
        assert_eq!(lines[2], "motd|Hello world|properties|text|motd|false|");
        assert_eq!(lines[3], "pvp|true|properties|bool|pvp|false|");
    }

    #[test]
    fn properties_to_editor_lines_non_object_returns_empty() {
        assert!(properties_to_editor_lines(&json!(null)).is_empty());
        assert!(properties_to_editor_lines(&json!([1, 2, 3])).is_empty());
        assert!(properties_to_editor_lines(&json!("string")).is_empty());
    }

    #[test]
    fn editor_changes_to_properties_json_typed_conversion() {
        let mut changes = HashMap::new();
        changes.insert("pvp".to_string(), "false".to_string());
        changes.insert("max-players".to_string(), "42".to_string());
        changes.insert("view-distance".to_string(), "3.5".to_string());
        changes.insert("motd".to_string(), "New MOTD".to_string());
        changes.insert("blank".to_string(), "".to_string());

        let out = editor_changes_to_properties_json(&changes);
        let obj = out.as_object().unwrap();

        assert_eq!(obj["pvp"], Value::Bool(false));
        assert_eq!(obj["max-players"], Value::Number(42.into()));
        assert_eq!(
            obj["view-distance"]
                .as_f64()
                .expect("view-distance should be f64"),
            3.5
        );
        assert_eq!(obj["motd"], Value::String("New MOTD".to_string()));
        // 빈 문자열은 null이 아닌 String("") 로 저장되어야 한다 (properties 규칙).
        assert_eq!(obj["blank"], Value::String(String::new()));
    }

    #[test]
    fn editor_changes_empty_produces_empty_object() {
        let changes = HashMap::new();
        let out = editor_changes_to_properties_json(&changes);
        assert!(out.as_object().unwrap().is_empty());
    }

    #[test]
    fn roundtrip_preserves_primitive_types() {
        // read_properties → editor_lines → changes 맵 → write_properties JSON
        // 위 경로에서 bool/number/string 타입이 보존되는지 확인.
        let original = json!({
            "pvp": true,
            "max-players": 20,
            "motd": "Hi",
        });

        let lines = properties_to_editor_lines(&original);
        let mut changes = HashMap::new();
        for line in lines {
            // key|value|... 에서 key/value만 추출
            let parts: Vec<&str> = line.splitn(7, '|').collect();
            changes.insert(parts[0].to_string(), parts[1].to_string());
        }

        let restored = editor_changes_to_properties_json(&changes);
        let obj = restored.as_object().unwrap();
        assert_eq!(obj["pvp"], Value::Bool(true));
        assert_eq!(obj["max-players"], Value::Number(20.into()));
        assert_eq!(obj["motd"], Value::String("Hi".to_string()));
    }
}
