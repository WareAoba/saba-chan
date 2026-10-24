//! CLI 실패의 종류와 종료 코드
//!
//! 0 성공 · 1 실패 · 2 사용법 오류 · 3 데몬에 연결할 수 없음 · 4 대상 없음 · 130 사용자가 취소

use serde_json::{json, Value};
use std::fmt;

/// 종료 코드와 그때 `error.code`에 오는 값. 1에는 데몬이 준 `error_code`도 온다 (`help --json`에 싣는다)
pub const EXIT_CODES: &[(i32, &[&str])] = &[
    (0, &[]),
    (1, &["failed", "partial"]),
    (2, &["usage", "ambiguous"]),
    (3, &["daemon_unreachable"]),
    (4, &["not_found"]),
    (130, &["cancelled"]),
];

#[derive(Debug, Clone)]
pub enum CliError {
    /// 데몬이 꺼져 있거나 포트가 다르다
    DaemonDown {
        url: String,
    },
    /// 데몬이 실패로 답했다 (HTTP 오류, 또는 200인데 `ok`/`success`가 false)
    Api {
        status: u16,
        code: Option<String>,
        message: String,
        body: Value,
    },
    /// 이름으로 찾지 못했다. `suggestions`는 비슷한 이름
    NotFound {
        what: String,
        query: String,
        suggestions: Vec<String>,
    },
    /// 여러 개가 맞는데 고를 수 없는 상황 (비대화형)
    Ambiguous {
        what: String,
        query: String,
        candidates: Vec<String>,
    },
    /// 입력이 모자라거나 틀렸다
    Usage(String),
    Cancelled,
    Failed(String),
    /// 일부만 됐다. `data`는 JSON 출력에 그대로 싣는 결과 (`{updated, failed}` 등)
    Partial {
        message: String,
        data: Value,
    },
}

impl CliError {
    pub fn exit_code(&self) -> i32 {
        match self {
            CliError::DaemonDown { .. } => 3,
            CliError::NotFound { .. } => 4,
            CliError::Usage(_) | CliError::Ambiguous { .. } => 2,
            CliError::Cancelled => 130,
            CliError::Api { .. } | CliError::Failed(_) | CliError::Partial { .. } => 1,
        }
    }

    pub fn code(&self) -> String {
        match self {
            CliError::DaemonDown { .. } => "daemon_unreachable".into(),
            CliError::Api { code, status, .. } => code.clone().unwrap_or_else(|| format!("http_{}", status)),
            CliError::NotFound { .. } => "not_found".into(),
            CliError::Ambiguous { .. } => "ambiguous".into(),
            CliError::Usage(_) => "usage".into(),
            CliError::Cancelled => "cancelled".into(),
            CliError::Failed(_) => "failed".into(),
            CliError::Partial { .. } => "partial".into(),
        }
    }

    /// 사람에게 보여 줄 다음 행동 한 줄
    pub fn hint(&self) -> Option<String> {
        match self {
            CliError::DaemonDown { .. } => Some(t!("err.daemon_down_hint")),
            CliError::NotFound { suggestions, .. } if !suggestions.is_empty() => {
                Some(t!("err.did_you_mean", names = suggestions.join(", ")))
            }
            CliError::Ambiguous { candidates, .. } => Some(t!("err.ambiguous_hint", names = candidates.join(", "))),
            CliError::Api { code, body, .. } => {
                let hints: Vec<String> = code
                    .as_deref()
                    .and_then(|c| api_hint(c, body))
                    .into_iter()
                    .chain(kept_backups(body))
                    .collect();
                (!hints.is_empty()).then(|| hints.join("\n  "))
            }
            _ => None,
        }
    }

    pub fn to_json(&self) -> Value {
        let mut v = json!({ "ok": false, "error": self.to_string(), "error_code": self.code() });
        match self {
            CliError::Api { status, body, .. } => {
                v["status"] = json!(status);
                v["response"] = body.clone();
            }
            CliError::NotFound { suggestions, .. } => v["suggestions"] = json!(suggestions),
            CliError::Ambiguous { candidates, .. } => v["candidates"] = json!(candidates),
            CliError::Partial { data, .. } => {
                for (k, x) in data.as_object().into_iter().flatten() {
                    v[k] = x.clone();
                }
            }
            _ => {}
        }
        v
    }
}

/// 실패한 설치가 되돌리지 못한 파일과 남긴 사본 (`restore_failed`·`kept_backups`는 같은 순서, 오류 코드와 상관없이 온다)
fn kept_backups(body: &Value) -> Option<String> {
    let names = |key: &str| body.get(key).and_then(Value::as_array).cloned().unwrap_or_default();
    let pairs: Vec<String> = names("restore_failed")
        .iter()
        .zip(names("kept_backups").iter())
        .filter_map(|(f, b)| Some(format!("{} → {}", f.as_str()?, b.as_str()?)))
        .collect();
    (!pairs.is_empty()).then(|| t!("err.hint.kept_backups", files = pairs.join(", ")))
}

/// 사바쨩 버전이 모자라 거절된 응답을 "업데이트가 필요해요" 문구로. 그런 거절이 아니면 None (GUI `utils/updateRequired.js`와 같은 규칙).
/// 릴레이(`agent_too_old`·`AGENT_TOO_OLD` + minVersion), 모듈(saba-core 의존성·더 새 스키마), 익스텐션(min_app_version·saba-core 의존성).
/// `name`이 비면 응답의 `module`·`extension`을 쓴다
pub fn update_required(body: &Value, name: &str) -> Option<String> {
    let s = |k: &str| body.get(k).and_then(Value::as_str).unwrap_or_default();
    let version = |requirement: &str| match requirement.trim().trim_start_matches(">=").trim() {
        "" => "?".to_string(),
        v => v.to_string(),
    };
    let related = |i: usize| body.get("related").and_then(|r| r.get(i)).and_then(Value::as_str).unwrap_or_default();
    let name = if name.is_empty() { [s("module"), s("extension")].into_iter().find(|n| !n.is_empty()).unwrap_or_default() } else { name };
    if s("lastErrorCode") == "agent_too_old" || s("error").eq_ignore_ascii_case("AGENT_TOO_OLD") {
        return Some(t!("update_required.relay", version = version(s("minVersion"))));
    }
    match s("error_code") {
        // 항목 꼴: "saba-core >=0.1.10 (installed 0.1.9)"
        "module_dependency_unsatisfied" => body
            .get("missing_dependencies")
            .and_then(Value::as_array)
            .into_iter()
            .flatten()
            .filter_map(Value::as_str)
            .find_map(|d| d.strip_prefix("saba-core "))
            .map(|d| t!("update_required.module", name = name, version = version(d.split(" (").next().unwrap_or_default()))),
        "module_schema_unsupported" => Some(t!("update_required.module_newer", name = name)),
        "app_version_unsatisfied" => Some(t!("update_required.extension", name = name, version = version(related(0)))),
        "component_version_unsatisfied" if related(0) == "saba-core" => {
            Some(t!("update_required.extension", name = name, version = version(related(1))))
        }
        _ => None,
    }
}

/// 데몬 오류 코드별 안내. 새 코드는 여기 한 줄.
pub fn api_hint(code: &str, body: &Value) -> Option<String> {
    if code == "isolation_unsupported" {
        return crate::i18n::try_t("sandbox.isolation_unsupported.message")
            .or_else(|| crate::i18n::try_t("module_errors.isolation_unsupported"));
    }
    if let Some(text) = crate::i18n::try_t(&format!("module_errors.{code}")) {
        return Some(text);
    }
    let list = |key: &str| {
        body.get(key)
            .and_then(Value::as_array)
            .map(|a| a.iter().filter_map(Value::as_str).collect::<Vec<_>>().join(" "))
            .unwrap_or_default()
    };
    match code {
        "extension_required" => Some(t!("err.hint.extension_required", ids = list("missing_extensions"))),
        "extension_not_verified" => Some(t!("err.hint.extension_not_verified", ids = list("unverified_extensions"))),
        "module_dependency_unsatisfied" => {
            // 항목 꼴: "ext-steamcmd >=1.0.0 (not installed)"
            let deps: Vec<&str> = body
                .get("missing_dependencies")
                .and_then(Value::as_array)
                .into_iter()
                .flatten()
                .filter_map(Value::as_str)
                .collect();
            let ext = |missing: bool| {
                deps.iter()
                    .filter(|d| d.ends_with("(not installed)") == missing)
                    .filter_map(|d| d.split_whitespace().next()?.strip_prefix("ext-"))
                    .collect::<Vec<_>>()
                    .join(" ")
            };
            let (install, old) = (ext(true), ext(false));
            let mut hints = Vec::new();
            if !install.is_empty() {
                hints.push(t!("err.hint.extension_required", ids = install));
            }
            if !old.is_empty() || deps.iter().any(|d| d.starts_with("saba-core")) {
                hints.push(t!("err.hint.dependency_outdated"));
            }
            (!hints.is_empty()).then(|| hints.join("\n  "))
        }
        "port_conflict" => {
            let mut lines: Vec<String> = body
                .get("conflicts")
                .and_then(Value::as_array)
                .into_iter()
                .flatten()
                .filter_map(Value::as_str)
                .map(str::to_string)
                .collect();
            lines.push(t!("err.hint.port_conflict"));
            Some(lines.join("\n  "))
        }
        "server_not_running" => Some(t!("err.hint.server_not_running")),
        "server_running" => Some(t!("err.hint.server_running")),
        "rcon_unreachable" => Some(t!("err.hint.rcon_unreachable")),
        "rcon_auth_failed" => Some(t!("err.hint.rcon_auth_failed")),
        "rcon_timeout" => Some(t!("err.hint.rcon_timeout")),
        "rcon_failed" => Some(t!("err.hint.rcon_failed")),
        "node_token_missing" | "not_paired" | "node_not_found" | "missing_token" | "invalid_token" | "invalid_token_format" => Some(t!("err.hint.node_token_missing")),
        // 모듈이 실행 파일을 못 찾음: executable_not_found (MK7), 옛 모듈은 server_jar_not_found
        "executable_missing" | "executable_not_found" | "server_jar_not_found" => Some(t!("err.hint.executable_missing")),
        "update_unsupported" => Some(t!("err.hint.update_unsupported")),
        "backup_unsupported" => Some(t!("err.hint.backup_unsupported")),
        "nothing_to_back_up" => Some(t!("err.hint.nothing_to_back_up")),
        "backup_busy" => Some(t!("err.hint.backup_busy")),
        // 켜진 서버가 쥐고 있는 파일 (`file`: 그 이름)
        "backup_file_locked" => Some(t!("err.hint.backup_file_locked", file = body.get("file").and_then(Value::as_str).unwrap_or("?"))),
        // 설치 전 실행 파일 백업을 못 해 모듈을 부르지 않았다 (`file`: 그 파일 이름. 백업 만들기의 backup_failed에는 없다)
        "backup_failed" => body.get("file").and_then(Value::as_str).map(|f| t!("err.hint.install_backup_failed", file = f)),
        "no_command_path" => Some(t!("err.hint.no_command_path")),
        "invalid_relay_url" => Some(t!("err.hint.invalid_relay_url")),
        "agent_too_old" | "module_schema_unsupported" | "app_version_unsatisfied" => Some(t!("err.hint.agent_too_old")),
        "component_version_unsatisfied" => update_required(body, "").map(|_| t!("err.hint.agent_too_old")),
        "module_busy" => Some(t!("err.hint.module_busy")),
        // 폴더 옮기기 (`/api/system/paths/move`)
        c if c.starts_with("paths_") => crate::i18n::try_t(&format!("err.hint.{}", c)),
        "schedule_changed" => Some(t!("err.hint.schedule_changed")),
        "not_supported" => Some(t!("err.hint.not_supported")),
        "invalid_command" => Some(t!("err.hint.invalid_command")),
        // `details: [{field, reason, word?, conflictWith?}]` — 칸마다 한 줄
        "validation_failed" => {
            let lines: Vec<String> = body
                .get("details")
                .and_then(Value::as_array)
                .into_iter()
                .flatten()
                .filter_map(|d| {
                    let get = |k: &str| d.get(k).and_then(Value::as_str);
                    let field = get("field")?;
                    let reason = get("reason").unwrap_or_default();
                    let text = crate::i18n::try_t(&format!("err.hint.word.{}", reason)).unwrap_or_else(|| reason.to_string());
                    let mut line = match get("word") {
                        Some(w) => format!("{} \"{}\": {}", field, w, text),
                        None => format!("{}: {}", field, text),
                    };
                    if let Some(w) = get("conflictWith") {
                        line = format!("{} ({})", line, t!("err.hint.conflict_with", name = w));
                    }
                    Some(line)
                })
                .collect();
            (!lines.is_empty()).then(|| lines.join("\n  "))
        }
        _ => None,
    }
}

impl fmt::Display for CliError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            CliError::DaemonDown { url } => write!(f, "{}", t!("err.daemon_down", url = url)),
            CliError::Api { message, .. } => write!(f, "{}", message),
            CliError::NotFound { what, query, .. } => write!(f, "{}", t!("err.not_found", what = what, query = query)),
            CliError::Ambiguous { what, query, .. } => write!(f, "{}", t!("err.ambiguous", what = what, query = query)),
            CliError::Usage(m) | CliError::Failed(m) | CliError::Partial { message: m, .. } => write!(f, "{}", m),
            CliError::Cancelled => write!(f, "{}", t!("err.cancelled")),
        }
    }
}

impl std::error::Error for CliError {}

pub type Result<T> = std::result::Result<T, anyhow::Error>;

pub fn usage(msg: impl Into<String>) -> anyhow::Error {
    CliError::Usage(msg.into()).into()
}

pub fn failed(msg: impl Into<String>) -> anyhow::Error {
    CliError::Failed(msg.into()).into()
}

/// anyhow 오류에서 CliError를 꺼낸다 (없으면 일반 실패로 감싼다)
pub fn classify(err: &anyhow::Error) -> std::borrow::Cow<'_, CliError> {
    match err.downcast_ref::<CliError>() {
        Some(e) => std::borrow::Cow::Borrowed(e),
        None => std::borrow::Cow::Owned(CliError::Failed(format!("{:#}", err))),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn isolation_unsupported_uses_the_sandbox_guidance() {
        crate::i18n::init("en");
        for lang in ["en", "ko"] {
            assert!(crate::i18n::try_t_in(lang, "sandbox.isolation_unsupported.message").is_some(), "{lang}");
        }
        assert_eq!(api_hint("isolation_unsupported", &json!({})), crate::i18n::try_t("sandbox.isolation_unsupported.message"));
    }

    #[test]
    fn module_call_failures_have_localized_guidance() {
        crate::i18n::init("en");
        for code in ["module_timeout", "module_execution_failed", "module_crashed", "module_bad_output", "module_output_too_large", "module_path_outside", "module_exception", "isolation_unsupported"] {
            let hint = api_hint(code, &json!({})).unwrap_or_default();
            assert!(!hint.is_empty() && !hint.contains("module_errors."), "{code}: {hint}");
            for lang in ["en", "ko"] {
                assert!(crate::i18n::try_t_in(lang, &format!("module_errors.{code}")).is_some(), "{lang}: {code}");
            }
        }
    }

    /// 데몬이 거절한 릴레이 주소는 쓸 수 있는 주소를 알려 준다 (R31)
    #[test]
    fn a_refused_relay_url_says_which_addresses_are_allowed() {
        crate::i18n::init("en");
        let hint = api_hint("invalid_relay_url", &json!({})).unwrap();
        assert!(hint.contains("https://") && hint.contains("localhost"), "{}", hint);
    }

    /// 모듈 설치·삭제가 겹치거나 권한 키 모양이 틀리면 원문 코드 대신 할 일을 알려 준다
    #[test]
    fn busy_modules_and_malformed_command_keys_get_a_hint() {
        crate::i18n::init("en");
        for code in ["module_busy", "invalid_command", "not_supported", "schedule_changed"] {
            let hint = api_hint(code, &json!({})).unwrap_or_default();
            assert!(!hint.is_empty() && !hint.contains("err.hint"), "{}: {}", code, hint);
        }
        // 언어는 다른 시험이 먼저 정할 수 있어 모양만 본다
        assert!(api_hint("invalid_command", &json!({})).unwrap().contains(">/<"));
    }

    /// RCON만 받는 서버의 명령 실패(데몬 #457)는 데몬 영어 문구만 남기지 않고 할 일을 알려 준다
    #[test]
    fn rcon_failures_say_what_to_do() {
        crate::i18n::init("en");
        for code in ["rcon_unreachable", "rcon_auth_failed", "rcon_timeout", "rcon_failed"] {
            let hint = api_hint(code, &json!({ "protocol": "rcon" })).unwrap_or_default();
            assert!(!hint.is_empty() && !hint.contains("err.hint"), "{}: {}", code, hint);
        }
        // 시간 초과는 서버가 이미 실행했을 수 있다: 다시 보내라고 하지 않고 콘솔에서 확인하게 한다 (지급·차단이 두 번 되지 않게)
        let hint = api_hint("rcon_timeout", &json!({ "protocol": "rcon" })).unwrap();
        assert!(hint.contains("saba-chan-cli console") && !hint.to_lowercase().contains("try again"), "{}", hint);
    }

    /// `help --json`의 종료 코드 표가 실제 종료 코드와 맞는다
    #[test]
    fn exit_code_table_matches_every_error() {
        let all = [
            CliError::DaemonDown { url: String::new() },
            CliError::NotFound { what: String::new(), query: String::new(), suggestions: vec![] },
            CliError::Ambiguous { what: String::new(), query: String::new(), candidates: vec![] },
            CliError::Usage(String::new()),
            CliError::Cancelled,
            CliError::Failed(String::new()),
            CliError::Partial { message: String::new(), data: json!({}) },
        ];
        for e in all {
            let row = EXIT_CODES.iter().find(|(exit, _)| *exit == e.exit_code()).unwrap();
            assert!(row.1.contains(&e.code().as_str()), "{} not listed under {}", e.code(), row.0);
        }
        let api = CliError::Api { status: 409, code: Some("backup_busy".into()), message: String::new(), body: json!({}) };
        assert_eq!(api.exit_code(), 1);
    }

    /// 설치 전 백업을 못 해 설치하지 않았으면 어느 파일인지, 되돌리지 못한 파일은 남긴 사본 이름을 알린다 (데몬 #533)
    #[test]
    fn a_failed_install_names_the_file_it_could_not_back_up_or_put_back() {
        crate::i18n::init("en");
        let hint = api_hint("backup_failed", &json!({ "file": "PalServer" })).unwrap();
        assert!(hint.contains("PalServer") && !hint.contains("err.hint"), "{}", hint);
        assert_eq!(api_hint("backup_failed", &json!({})), None, "a backup that failed for other reasons has no file");

        let body = json!({ "success": false, "restore_failed": ["PalServer", "java"], "kept_backups": ["PalServer.1.bak", "java.bak"] });
        let api = |code: Option<&str>| CliError::Api { status: 422, code: code.map(str::to_string), message: String::new(), body: body.clone() };
        for e in [api(None), api(Some("download_failed"))] {
            let hint = e.hint().unwrap_or_default();
            assert!(hint.contains("PalServer → PalServer.1.bak") && hint.contains("java → java.bak"), "{}", hint);
        }
    }

    #[test]
    fn validation_details_become_one_line_per_field() {
        crate::i18n::init("en");
        let body = json!({ "details": [
            { "field": "words[0]", "reason": "conflict", "word": "크리", "conflictWith": "survival" },
            { "field": "commands.kick", "reason": "too_many" },
            { "field": "input", "reason": "something_new" }
        ] });
        let hint = api_hint("validation_failed", &body).unwrap();
        let lines: Vec<&str> = hint.split("\n  ").collect();
        assert_eq!(lines.len(), 3);
        assert!(lines[0].starts_with("words[0] \"크리\": ") && lines[0].contains("survival"), "{}", lines[0]);
        assert!(lines[1].starts_with("commands.kick: ") && !lines[1].ends_with("too_many"), "reason is localized: {}", lines[1]);
        assert_eq!(lines[2], "input: something_new");
        assert_eq!(api_hint("validation_failed", &json!({})), None);
    }
}
