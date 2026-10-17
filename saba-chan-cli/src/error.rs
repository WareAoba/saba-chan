//! CLI 실패의 종류와 종료 코드
//!
//! 0 성공 · 1 실패 · 2 사용법 오류 · 3 데몬에 연결할 수 없음 · 4 대상 없음 · 130 사용자가 취소

use serde_json::{json, Value};
use std::fmt;

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
}

impl CliError {
    pub fn exit_code(&self) -> i32 {
        match self {
            CliError::DaemonDown { .. } => 3,
            CliError::NotFound { .. } => 4,
            CliError::Usage(_) | CliError::Ambiguous { .. } => 2,
            CliError::Cancelled => 130,
            CliError::Api { .. } | CliError::Failed(_) => 1,
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
            CliError::Api {
                code: Some(code), body, ..
            } => api_hint(code, body),
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
            _ => {}
        }
        v
    }
}

/// 데몬 오류 코드별 안내. 새 코드는 여기 한 줄.
pub fn api_hint(code: &str, body: &Value) -> Option<String> {
    let list = |key: &str| {
        body.get(key)
            .and_then(Value::as_array)
            .map(|a| a.iter().filter_map(Value::as_str).collect::<Vec<_>>().join(" "))
            .unwrap_or_default()
    };
    match code {
        "extension_required" => Some(t!("err.hint.extension_required", ids = list("missing_extensions"))),
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
        "node_token_missing" | "not_paired" => Some(t!("err.hint.node_token_missing")),
        "executable_missing" => Some(t!("err.hint.executable_missing")),
        "update_unsupported" => Some(t!("err.hint.update_unsupported")),
        "backup_unsupported" => Some(t!("err.hint.backup_unsupported")),
        "nothing_to_back_up" => Some(t!("err.hint.nothing_to_back_up")),
        "backup_busy" => Some(t!("err.hint.backup_busy")),
        "no_command_path" => Some(t!("err.hint.no_command_path")),
        "validation_failed" => None,
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
            CliError::Usage(m) | CliError::Failed(m) => write!(f, "{}", m),
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
