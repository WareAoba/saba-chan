//! Human-readable formatters for one-shot CLI output.
//!
//! Keep these intentionally small and side-effect free so they remain easy
//! to unit-test. The runner is responsible for choosing what value to format.

use serde_json::Value;

/// Generic JSON-to-text fallback. Tries to render scalar values cleanly and
/// pretty-prints anything more complex.
pub fn value_to_text(v: &Value) -> String {
    match v {
        Value::Null => String::new(),
        Value::Bool(b) => b.to_string(),
        Value::Number(n) => n.to_string(),
        Value::String(s) => s.clone(),
        Value::Array(_) | Value::Object(_) => {
            serde_json::to_string_pretty(v).unwrap_or_else(|_| v.to_string())
        }
    }
}

/// Render a list of instances as a fixed-width table.
pub fn instances_table(instances: &[Value]) -> String {
    if instances.is_empty() {
        return "(no instances configured)".to_string();
    }

    let rows: Vec<(String, String, String, String)> = instances
        .iter()
        .map(|inst| {
            (
                inst.get("name").and_then(|v| v.as_str()).unwrap_or("?").to_string(),
                inst.get("module_name")
                    .or_else(|| inst.get("module"))
                    .and_then(|v| v.as_str())
                    .unwrap_or("-")
                    .to_string(),
                inst.get("port")
                    .and_then(|v| v.as_u64())
                    .map(|p| p.to_string())
                    .unwrap_or_else(|| "-".to_string()),
                inst.get("id").and_then(|v| v.as_str()).unwrap_or("?").to_string(),
            )
        })
        .collect();

    let name_w = rows.iter().map(|r| r.0.chars().count()).max().unwrap_or(4).max(4);
    let mod_w = rows.iter().map(|r| r.1.chars().count()).max().unwrap_or(6).max(6);
    let port_w = rows.iter().map(|r| r.2.chars().count()).max().unwrap_or(4).max(4);

    let mut out = String::new();
    out.push_str(&format!(
        "{:<name_w$}  {:<mod_w$}  {:>port_w$}  ID\n",
        "NAME", "MODULE", "PORT",
        name_w = name_w,
        mod_w = mod_w,
        port_w = port_w
    ));
    out.push_str(&format!(
        "{}  {}  {}  {}\n",
        "-".repeat(name_w),
        "-".repeat(mod_w),
        "-".repeat(port_w),
        "-".repeat(36)
    ));
    for (n, m, p, id) in rows {
        out.push_str(&format!(
            "{:<name_w$}  {:<mod_w$}  {:>port_w$}  {}\n",
            n, m, p, id,
            name_w = name_w, mod_w = mod_w, port_w = port_w
        ));
    }
    out.trim_end().to_string()
}

/// Render module list as a table.
pub fn modules_table(modules: &[Value]) -> String {
    if modules.is_empty() {
        return "(no modules installed)".to_string();
    }
    let rows: Vec<(String, String)> = modules
        .iter()
        .map(|m| {
            (
                m.get("name").and_then(|v| v.as_str()).unwrap_or("?").to_string(),
                m.get("version")
                    .and_then(|v| v.as_str())
                    .unwrap_or("-")
                    .to_string(),
            )
        })
        .collect();
    let name_w = rows.iter().map(|r| r.0.chars().count()).max().unwrap_or(4).max(4);

    let mut out = String::new();
    out.push_str(&format!(
        "{:<name_w$}  VERSION\n",
        "NAME", name_w = name_w
    ));
    out.push_str(&format!("{}  {}\n", "-".repeat(name_w), "-".repeat(8)));
    for (n, v) in rows {
        out.push_str(&format!("{:<name_w$}  {}\n", n, v, name_w = name_w));
    }
    out.trim_end().to_string()
}

/// Render extension list as a table.
pub fn extensions_table(extensions: &[Value]) -> String {
    if extensions.is_empty() {
        return "(no extensions installed)".to_string();
    }
    let rows: Vec<(String, String, String)> = extensions
        .iter()
        .map(|e| {
            (
                e.get("id")
                    .or_else(|| e.get("name"))
                    .and_then(|v| v.as_str())
                    .unwrap_or("?")
                    .to_string(),
                e.get("version").and_then(|v| v.as_str()).unwrap_or("-").to_string(),
                if e.get("enabled").and_then(|v| v.as_bool()).unwrap_or(false) {
                    "enabled".into()
                } else {
                    "disabled".into()
                },
            )
        })
        .collect();
    let id_w = rows.iter().map(|r| r.0.chars().count()).max().unwrap_or(2).max(2);
    let mut out = String::new();
    out.push_str(&format!(
        "{:<id_w$}  VERSION   STATE\n",
        "ID",
        id_w = id_w
    ));
    out.push_str(&format!(
        "{}  {}  {}\n",
        "-".repeat(id_w),
        "-".repeat(8),
        "-".repeat(8)
    ));
    for (id, v, s) in rows {
        out.push_str(&format!(
            "{:<id_w$}  {:<8}  {}\n",
            id, v, s,
            id_w = id_w
        ));
    }
    out.trim_end().to_string()
}

/// Render console entries as plain text lines (newest last), at most `max`.
pub fn console_lines(value: &Value, max: usize) -> String {
    let entries = crate::payload::console_entries(value);
    if entries.is_empty() {
        return "(no output)".to_string();
    }
    let skip = entries.len().saturating_sub(max);
    entries[skip..]
        .iter()
        .map(|e| crate::payload::entry_text(e).unwrap_or_else(|| value_to_text(e)))
        .collect::<Vec<_>>()
        .join("\n")
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn empty_instances_renders_placeholder() {
        assert_eq!(instances_table(&[]), "(no instances configured)");
    }

    #[test]
    fn instances_table_has_header_and_row() {
        let arr = vec![json!({
            "id": "abc-123",
            "name": "mc-1",
            "module_name": "minecraft",
            "port": 25565
        })];
        let out = instances_table(&arr);
        assert!(out.contains("NAME"));
        assert!(out.contains("MODULE"));
        assert!(out.contains("mc-1"));
        assert!(out.contains("minecraft"));
        assert!(out.contains("25565"));
        assert!(out.contains("abc-123"));
    }

    #[test]
    fn console_lines_joins_entries() {
        let v = json!({
            "lines": [
                {"content": "line one"},
                {"content": "line two"},
            ]
        });
        assert_eq!(console_lines(&v, 10), "line one\nline two");
    }

    #[test]
    fn console_lines_caps_max() {
        let v = json!({"lines": [
            {"content": "a"}, {"content": "b"}, {"content": "c"}, {"content": "d"}
        ]});
        assert_eq!(console_lines(&v, 2), "c\nd");
    }

    /// Regression: server console endpoint returns objects with `content`
    /// (not `text`/`line`/`message`). Previously the formatter dumped the
    /// raw JSON for every entry.
    #[test]
    fn console_lines_extracts_content_field_from_lines_array() {
        let v = json!({
            "lines": [
                {"id": 0, "content": "hello world", "level": "info", "source": "stdout"},
                {"id": 1, "content": "second line", "level": "info", "source": "stdout"},
            ]
        });
        assert_eq!(console_lines(&v, 10), "hello world\nsecond line");
    }

    #[test]
    fn value_to_text_renders_scalars() {
        assert_eq!(value_to_text(&json!(42)), "42");
        assert_eq!(value_to_text(&json!("hi")), "hi");
        assert_eq!(value_to_text(&json!(true)), "true");
        assert_eq!(value_to_text(&json!(null)), "");
    }

    #[test]
    fn extensions_table_marks_disabled() {
        let arr = vec![json!({"id": "ext-foo", "version": "1.0.0", "enabled": false})];
        let out = extensions_table(&arr);
        assert!(out.contains("ext-foo"));
        assert!(out.contains("disabled"));
    }
}
