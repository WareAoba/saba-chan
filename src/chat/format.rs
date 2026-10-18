//! 명령 결과(JSON)를 채팅 메시지 텍스트로 바꾸는 범용 포맷터

use super::i18n::Catalog;
use serde_json::Value;

pub fn format_response(data: &Value, cat: &Catalog) -> String {
    let done = || cat.t("responses.command_complete", &[]);
    match data {
        Value::Null => done(),
        Value::String(s) if s.is_empty() => done(),
        Value::String(s) => s.clone(),
        Value::Array(items) if items.is_empty() => cat.t("responses.empty_list", &[]),
        Value::Array(items) => format_array(items),
        Value::Object(map) if map.is_empty() => done(),
        Value::Object(map) => {
            // 콘솔 명령 결과 {response, executed}: 서버가 찍은 답만 보인다 (내부 키는 보이지 않는다)
            if let Some(text) = map.get("response").and_then(Value::as_str) {
                return if text.trim().is_empty() { done() } else { text.to_string() };
            }
            // 첫 번째 배열 필드가 있으면 그것을 목록으로 보여준다 (예: {"players": [...]})
            for (key, value) in map {
                if let Value::Array(items) = value {
                    if items.is_empty() {
                        return format!("📋 **{}**: (empty)", key);
                    }
                    return format!("📋 **{}** ({}):\n{}", key, items.len(), format_array(items));
                }
            }
            let lines: Vec<String> = map
                .iter()
                .filter(|(_, v)| !v.is_null())
                .map(|(k, v)| format!("• **{}**: {}", k, scalar(v)))
                .collect();
            if lines.is_empty() { done() } else { lines.join("\n") }
        }
        other => scalar(other),
    }
}

fn format_array(items: &[Value]) -> String {
    items
        .iter()
        .enumerate()
        .map(|(idx, item)| match item {
            Value::String(s) => format!("{}. {}", idx + 1, s),
            Value::Object(map) => {
                let name = ["name", "id", "userid"]
                    .iter()
                    .find_map(|k| map.get(*k).filter(|v| !v.is_null()).map(scalar))
                    .unwrap_or_else(|| format!("#{}", idx + 1));
                let extras: Vec<String> = map
                    .iter()
                    .filter(|(k, _)| *k != "name" && *k != "id")
                    .map(|(k, v)| format!("{}: {}", k, scalar(v)))
                    .collect();
                if extras.is_empty() {
                    format!("• **{}**", name)
                } else {
                    format!("• **{}** ({})", name, extras.join(", "))
                }
            }
            other => format!("• {}", scalar(other)),
        })
        .collect::<Vec<_>>()
        .join("\n")
}

/// 값 하나를 사람이 읽을 문자열로. 문자열은 따옴표 없이, 나머지는 JSON 표기.
fn scalar(v: &Value) -> String {
    match v {
        Value::String(s) => s.clone(),
        other => other.to_string(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn formats_common_shapes() {
        let cat = Catalog::for_language("en");
        assert_eq!(format_response(&json!("hi"), &cat), "hi");
        assert_eq!(format_response(&json!(["a", "b"]), &cat), "1. a\n2. b");
        let players = format_response(&json!({"players": [{"name": "Kim", "level": 3}]}), &cat);
        assert_eq!(players, "📋 **players** (1):\n• **Kim** (level: 3)");
        assert_eq!(format_response(&json!({"response": "There are 0 players", "executed": "list"}), &cat), "There are 0 players");
        // 서버가 아무것도 찍지 않은 콘솔 명령도 내부 키 대신 완료 문구
        assert_eq!(format_response(&json!({"response": "", "executed": "list"}), &cat), cat.t("responses.command_complete", &[]));
        assert_eq!(format_response(&Value::Null, &cat), cat.t("responses.command_complete", &[]));
    }
}
