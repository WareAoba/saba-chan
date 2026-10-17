//! 데몬 응답/요청 JSON을 다루는 순수 헬퍼 — one-shot, 셸, TUI 공용.
//!
//! - [`parse_scalar`]: 사용자 입력 문자열 → JSON 스칼라
//! - 콘솔: [`console_entries`], [`entry_text`], [`max_entry_id`]
//! - 매니페스트: [`parse_extension_manifest`], [`parse_module_manifest`]

use serde_json::{json, Value};

/// "true"/"false"(대소문자 무시) → bool, 정수 → i64, 유한 실수 → f64, 그 외 → 문자열.
pub fn parse_scalar(raw: &str) -> Value {
    if raw.eq_ignore_ascii_case("true") {
        return Value::Bool(true);
    }
    if raw.eq_ignore_ascii_case("false") {
        return Value::Bool(false);
    }
    if let Ok(n) = raw.parse::<i64>() {
        return json!(n);
    }
    if let Some(n) = raw.parse::<f64>().ok().and_then(serde_json::Number::from_f64) {
        return Value::Number(n);
    }
    Value::String(raw.to_string())
}

// ─── 콘솔 ──────────────────────────────────────────────────────────────

/// 콘솔 응답의 항목 배열 (`{"lines": [...]}`). 다른 모양이면 빈 슬라이스.
pub fn console_entries(value: &Value) -> &[Value] {
    value
        .get("lines")
        .and_then(|v| v.as_array())
        .map(|a| a.as_slice())
        .unwrap_or(&[])
}

/// 콘솔 항목 하나의 표시 텍스트.
///
/// - `/api/instance/{id}/console` → `content`
/// - `/api/daemon/console`        → `[LEVEL] target message`
///
/// 알 수 없는 모양이면 `None`.
pub fn entry_text(entry: &Value) -> Option<String> {
    if let Some(s) = entry.as_str() {
        return Some(s.to_string());
    }
    if let Some(content) = entry.get("content").and_then(|v| v.as_str()) {
        return Some(content.to_string());
    }
    let message = entry.get("message").and_then(|v| v.as_str()).filter(|m| !m.is_empty())?;
    let level = entry.get("level").and_then(|v| v.as_str()).unwrap_or("info");
    let target = entry.get("target").and_then(|v| v.as_str()).unwrap_or("");
    Some(format!("[{}] {} {}", level.to_uppercase(), target, message))
}

/// 항목들의 최대 `id` (커서 기반 폴링용). 데몬 `since`는 `id >= since`로 거르므로
/// 다음 요청은 `max + 1`을 보낸다.
pub fn max_entry_id(entries: &[Value]) -> Option<u64> {
    entries.iter().filter_map(|e| e.get("id").and_then(|v| v.as_u64())).max()
}

// ─── 매니페스트 ────────────────────────────────────────────────────────

/// 원격 매니페스트 항목 (모듈/익스텐션 공용)
#[derive(Clone, Debug, PartialEq)]
pub struct ManifestItem {
    pub id: String,
    pub name: String,
    pub version: String,
    pub description: String,
}

fn manifest_error(data: &Value) -> String {
    data.get("error")
        .and_then(|v| v.as_str())
        .unwrap_or("manifest fetch failed")
        .to_string()
}

/// `GET /api/extensions/manifest` → `{success, extensions: [{id, name, version, description}], error?}`
pub fn parse_extension_manifest(data: &Value) -> Result<Vec<ManifestItem>, String> {
    if data.get("success").and_then(|v| v.as_bool()) == Some(false) {
        return Err(manifest_error(data));
    }
    let list = data.get("extensions").and_then(|v| v.as_array()).ok_or_else(|| manifest_error(data))?;
    Ok(list
        .iter()
        .filter_map(|e| {
            let id = e.get("id").and_then(|v| v.as_str())?.to_string();
            Some(ManifestItem {
                name: e.get("name").and_then(|v| v.as_str()).unwrap_or(&id).to_string(),
                version: e.get("version").and_then(|v| v.as_str()).unwrap_or("?").to_string(),
                description: e.get("description").and_then(|v| v.as_str()).unwrap_or("").to_string(),
                id,
            })
        })
        .collect())
}

/// `GET /api/modules/manifest` → `{ok, manifest: {modules: {<id>: {version, display_name, ...}}}, error?}`
pub fn parse_module_manifest(data: &Value) -> Result<Vec<ManifestItem>, String> {
    if data.get("ok").and_then(|v| v.as_bool()) == Some(false) {
        return Err(manifest_error(data));
    }
    let modules = data
        .get("manifest")
        .and_then(|m| m.get("modules"))
        .and_then(|v| v.as_object())
        .ok_or_else(|| manifest_error(data))?;
    let mut items: Vec<ManifestItem> = modules
        .iter()
        .map(|(id, m)| ManifestItem {
            id: id.clone(),
            name: m.get("display_name").and_then(|v| v.as_str()).unwrap_or(id).to_string(),
            version: m.get("version").and_then(|v| v.as_str()).unwrap_or("?").to_string(),
            description: m.get("description").and_then(|v| v.as_str()).unwrap_or("").to_string(),
        })
        .collect();
    items.sort_by(|a, b| a.id.cmp(&b.id));
    Ok(items)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_scalar_handles_bool_int_float_string() {
        assert_eq!(parse_scalar("true"), Value::Bool(true));
        assert_eq!(parse_scalar("FALSE"), Value::Bool(false));
        assert_eq!(parse_scalar("42"), json!(42));
        assert_eq!(parse_scalar("3.25"), json!(3.25));
        assert_eq!(parse_scalar("hello"), Value::String("hello".into()));
        assert_eq!(parse_scalar(""), Value::String(String::new()));
        assert_eq!(parse_scalar("NaN"), Value::String("NaN".into()));
    }

    #[test]
    fn entry_text_handles_server_and_daemon_shapes() {
        assert_eq!(entry_text(&json!({"id": 1, "content": "hello"})), Some("hello".into()));
        assert_eq!(
            entry_text(&json!({"id": 2, "level": "warn", "target": "saba_core::ipc", "message": "a:b"})),
            Some("[WARN] saba_core::ipc a:b".into())
        );
        assert_eq!(entry_text(&json!("plain")), Some("plain".into()));
        assert_eq!(entry_text(&json!({"foo": "bar"})), None);
    }

    #[test]
    fn max_entry_id_picks_highest() {
        let v = json!({"lines": [{"id": 3}, {"id": 9}, {"id": 5}]});
        assert_eq!(max_entry_id(console_entries(&v)), Some(9));
        assert_eq!(max_entry_id(console_entries(&json!({}))), None);
    }

    #[test]
    fn module_manifest_reads_nested_object_map() {
        let v = json!({"ok": true, "manifest": {"release_version": "1", "modules": {
            "palworld": {"version": "1.2.0", "display_name": "palworld"},
            "minecraft": {"version": "2.0.0", "display_name": "minecraft"}
        }}});
        let items = parse_module_manifest(&v).unwrap();
        assert_eq!(items.len(), 2);
        assert_eq!(items[0].id, "minecraft");
        assert_eq!(items[0].version, "2.0.0");
    }

    #[test]
    fn module_manifest_reports_error() {
        let v = json!({"ok": false, "error": "relay down"});
        assert_eq!(parse_module_manifest(&v), Err("relay down".into()));
    }

    #[test]
    fn extension_manifest_reads_list_and_error() {
        let v = json!({"success": true, "extensions": [
            {"id": "docker", "name": "Docker", "version": "0.3.0", "description": "d"}
        ], "updates": []});
        let items = parse_extension_manifest(&v).unwrap();
        assert_eq!(items[0].name, "Docker");
        let e = json!({"success": false, "error": "x", "extensions": []});
        assert_eq!(parse_extension_manifest(&e), Err("x".into()));
    }
}
