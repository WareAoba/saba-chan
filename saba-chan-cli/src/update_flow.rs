//! 자체 업데이트 흐름 — one-shot `update`, TUI 명령 모드, TUI Updates 화면 공용.
//!
//! 데몬 계약 (`src/ipc/updates.rs`):
//! - `GET /api/updates/status`, `POST /api/updates/check`
//!   → `{components: [{component, display_name, current_version, latest_version, update_available, downloaded}]}`
//! - `GET /api/updates/config` → `{ok, config: {...}}`, `PUT /api/updates/config` → 부분 갱신
//! - `POST /api/updates/apply` → `{ok, applied, errors, needs_updater, requires_updater, launch_token}`
//!   `requires_updater`면 데몬은 아무것도 적용하지 않았고, 클라이언트가 `launch_token`으로
//!   업데이터 exe를 실행해야 한다 (GUI/Core/Updater 컴포넌트).

use std::path::PathBuf;

use serde_json::Value;

use crate::client::DaemonClient;

/// 출력 한 줄의 성격 (TUI는 색상, one-shot은 stdout/stderr 구분에 사용)
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Tone {
    Ok,
    Info,
    Err,
    Text,
}

pub type Lines = Vec<(Tone, String)>;

fn str_or<'a>(v: &'a Value, key: &str, default: &'a str) -> &'a str {
    v.get(key).and_then(|x| x.as_str()).unwrap_or(default)
}

fn string_list(v: &Value, key: &str) -> Vec<String> {
    v.get(key)
        .and_then(|x| x.as_array())
        .map(|a| a.iter().filter_map(|s| s.as_str().map(String::from)).collect())
        .unwrap_or_default()
}

/// 상태/확인 응답 → 컴포넌트 목록 출력.
pub fn status_lines(status: &Value) -> Lines {
    let Some(comps) = status.get("components").and_then(|v| v.as_array()) else {
        return vec![(Tone::Text, status.to_string())];
    };
    let available = comps.iter().filter(|c| c["update_available"].as_bool().unwrap_or(false)).count();
    let mut lines = vec![if available > 0 {
        (Tone::Ok, format!("{} update(s) available:", available))
    } else {
        (Tone::Ok, "All components are up to date.".to_string())
    }];
    for c in comps {
        let avail = c["update_available"].as_bool().unwrap_or(false);
        let downloaded = if c["downloaded"].as_bool().unwrap_or(false) { " [downloaded]" } else { "" };
        lines.push((
            Tone::Text,
            format!(
                "  {} {:<20} {} → {}{}",
                if avail { "⬆" } else { "✓" },
                str_or(c, "component", "?"),
                str_or(c, "current_version", "?"),
                str_or(c, "latest_version", "?"),
                downloaded,
            ),
        ));
    }
    lines
}

/// `GET /api/updates/config` 응답(`{ok, config}`) → `key: value` 목록.
pub fn config_lines(resp: &Value) -> Lines {
    let mut lines = vec![(Tone::Ok, "Updater Config:".to_string())];
    if let Some(map) = resp.get("config").and_then(|v| v.as_object()) {
        for (k, v) in map {
            lines.push((Tone::Text, format!("  {}: {}", k, v)));
        }
    }
    lines
}

/// 업데이트를 적용하고 (원본 응답, 결과 출력 줄)을 돌려준다.
/// `requires_updater`면 업데이터를 실행한다.
pub async fn apply(client: &DaemonClient) -> anyhow::Result<(Value, Lines)> {
    let resp = client.apply_updates().await?;
    let lines = apply_outcome_lines(&resp, spawn_updater);
    Ok((resp, lines))
}

/// 적용 응답 해석 (업데이터 실행 함수 주입 — 테스트용).
fn apply_outcome_lines(resp: &Value, launch: impl FnOnce(&str) -> Result<PathBuf, String>) -> Lines {
    let mut lines = Lines::new();
    let applied = string_list(resp, "applied");
    if !applied.is_empty() {
        lines.push((Tone::Ok, format!("✓ Applied: {}", applied.join(", "))));
    }
    for e in string_list(resp, "errors") {
        lines.push((Tone::Err, format!("✗ {}", e)));
    }
    let needs = string_list(resp, "needs_updater");
    if resp["requires_updater"].as_bool().unwrap_or(false) {
        lines.push((Tone::Info, format!("Updater required for: {}", needs.join(", "))));
        match resp["launch_token"].as_str().filter(|t| !t.is_empty()) {
            Some(token) => match launch(token) {
                Ok(exe) => lines.push((Tone::Ok, format!("✓ Updater launched: {}", exe.display()))),
                Err(e) => lines.push((Tone::Err, format!("✗ {}", e))),
            },
            None => lines.push((Tone::Err, "✗ Launch token missing — daemon did not issue one.".into())),
        }
    } else if !needs.is_empty() {
        lines.push((Tone::Err, format!("✗ Updater could not be prepared for: {}", needs.join(", "))));
    }
    if lines.is_empty() {
        lines.push((Tone::Info, "Nothing to apply.".into()));
    }
    lines
}

/// 업데이터 exe를 `--apply --spawned-by cli --launch-token <t> --relaunch <cli>`로 실행한다.
/// 업데이터는 CLI를 다시 띄울 때 `--after-update`를 덧붙인다 (CLI는 이 인자를 무시한다).
fn spawn_updater(token: &str) -> Result<PathBuf, String> {
    let exe_name = if cfg!(target_os = "windows") { "saba-chan-updater.exe" } else { "saba-chan-updater" };
    let root = crate::process::find_project_root().unwrap_or_default();
    let mut candidates: Vec<PathBuf> = Vec::new();
    if let Some(dir) = std::env::var_os("PORTABLE_EXECUTABLE_DIR") {
        candidates.push(PathBuf::from(dir).join(exe_name));
    }
    if let Some(dir) = std::env::current_exe().ok().and_then(|p| p.parent().map(|d| d.to_path_buf())) {
        candidates.push(dir.join(exe_name));
    }
    candidates.push(root.join(exe_name));
    candidates.push(root.join("target").join("release").join(exe_name));
    candidates.push(root.join("target").join("debug").join(exe_name));

    let exe = candidates.iter().find(|p| p.exists()).ok_or_else(|| {
        let tried: Vec<String> = candidates.iter().map(|p| p.display().to_string()).collect();
        format!("Updater executable not found. Searched: {}", tried.join(" | "))
    })?;

    let mut cmd = std::process::Command::new(exe);
    cmd.args(["--apply", "--spawned-by", "cli", "--launch-token", token]);
    if let Ok(current) = std::env::current_exe() {
        cmd.arg("--relaunch").arg(current);
    }
    cmd.stdin(std::process::Stdio::null())
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .spawn()
        .map_err(|e| format!("Failed to launch updater: {}", e))?;
    Ok(exe.clone())
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn apply_launches_updater_when_required() {
        let resp = json!({
            "ok": true, "applied": [], "errors": [],
            "needs_updater": ["saba-core", "gui"], "requires_updater": true, "launch_token": "tok"
        });
        let mut seen = None;
        let lines = apply_outcome_lines(&resp, |t| {
            seen = Some(t.to_string());
            Ok(PathBuf::from("updater.exe"))
        });
        assert_eq!(seen.as_deref(), Some("tok"));
        assert!(lines.iter().any(|(tone, l)| *tone == Tone::Ok && l.contains("Updater launched")));
    }

    #[test]
    fn apply_direct_reports_applied_and_errors_without_launch() {
        let resp = json!({
            "ok": false, "applied": ["module-minecraft"], "errors": ["x: failed"],
            "needs_updater": [], "requires_updater": false, "launch_token": null
        });
        let lines = apply_outcome_lines(&resp, |_| panic!("must not launch"));
        assert_eq!(lines[0], (Tone::Ok, "✓ Applied: module-minecraft".into()));
        assert_eq!(lines[1], (Tone::Err, "✗ x: failed".into()));
    }

    #[test]
    fn apply_reports_missing_token() {
        let resp = json!({ "requires_updater": true, "needs_updater": ["gui"], "launch_token": null });
        let lines = apply_outcome_lines(&resp, |_| panic!("must not launch"));
        assert!(lines.iter().any(|(tone, l)| *tone == Tone::Err && l.contains("token")));
    }

    #[test]
    fn config_lines_read_nested_config_object() {
        let lines = config_lines(&json!({ "ok": true, "config": { "enabled": true } }));
        assert_eq!(lines[1], (Tone::Text, "  enabled: true".into()));
        assert_eq!(lines.len(), 2);
    }

    #[test]
    fn status_lines_mark_available_updates() {
        let v = json!({ "components": [
            { "component": "gui", "current_version": "1", "latest_version": "2", "update_available": true, "downloaded": true },
            { "component": "cli", "current_version": "1", "latest_version": "1", "update_available": false }
        ]});
        let lines = status_lines(&v);
        assert_eq!(lines[0].1, "1 update(s) available:");
        assert!(lines[1].1.contains("⬆") && lines[1].1.contains("[downloaded]"));
        assert!(lines[2].1.contains("✓"));
    }
}
