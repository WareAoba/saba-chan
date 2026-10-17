//! 출력 — 사람용(색·표)과 스크립트용(`--json`) 두 갈래
//!
//! 규칙: `--json`이면 stdout에는 JSON 문서 하나만 나간다. 사람용 안내·진행 표시는 stderr로.
//! 여러 단계를 거치는 명령(데몬 재시작 등)도 마지막 결과(또는 오류) 하나만 낸다: 문서를 모아 두었다가
//! 명령이 끝날 때 `flush()`가 찍는다.
//! 사람 모드에서 결과는 stdout, 경고·오류는 stderr.

use comfy_table::{presets, ContentArrangement, Table};
use console::{style, StyledObject};
use serde_json::Value;

/// `--json`에서 아직 찍지 않은 마지막 문서
static PENDING: std::sync::Mutex<Option<Value>> = std::sync::Mutex::new(None);

fn emit(value: Value) {
    if let Ok(mut p) = PENDING.lock() {
        *p = Some(value);
    }
}

/// 모아 둔 JSON 문서를 찍는다 (명령 하나가 끝날 때)
pub fn flush() {
    if let Some(v) = PENDING.lock().ok().and_then(|mut p| p.take()) {
        println!("{}", serde_json::to_string_pretty(&v).unwrap_or_default());
    }
}

#[derive(Clone, Copy)]
pub struct Out {
    pub json: bool,
}

impl Out {
    pub fn new(json: bool, color: bool) -> Self {
        console::set_colors_enabled(color && !json);
        console::set_colors_enabled_stderr(color);
        Self { json }
    }

    /// 결과 데이터. JSON 모드면 그대로, 아니면 `human`이 그린다.
    pub fn data(&self, value: &Value, human: impl FnOnce(&Value)) {
        if self.json {
            emit(value.clone());
        } else {
            human(value);
        }
    }

    /// 동작 결과 한 줄 (✓). JSON 모드면 `value`를 낸다.
    pub fn done(&self, msg: &str, value: &Value) {
        if self.json {
            emit(value.clone());
        } else {
            println!("{} {}", style("✓").green().bold(), msg);
        }
    }

    /// 진행 중 안내 (stderr, JSON 모드에서도 사람이 보게)
    pub fn note(&self, msg: &str) {
        eprintln!("{} {}", style("·").dim(), msg);
    }

    pub fn warn(&self, msg: &str) {
        eprintln!("{} {}", style("!").yellow().bold(), msg);
    }

    pub fn hint(&self, msg: &str) {
        if !self.json {
            eprintln!("  {}", style(msg).dim());
        }
    }

    pub fn error(&self, err: &crate::error::CliError) {
        if self.json {
            emit(err.to_json());
            return;
        }
        eprintln!("{} {}", style("✗").red().bold(), err);
        if let crate::error::CliError::Api { body, .. } = err {
            for line in validation_lines(body) {
                eprintln!("  {}", line);
            }
        }
        if let Some(h) = err.hint() {
            eprintln!("  {}", style(h).dim());
        }
    }
}

/// 설정 검증 실패(`fields`)를 한 줄씩
fn validation_lines(body: &Value) -> Vec<String> {
    body.get("fields")
        .and_then(Value::as_array)
        .map(|fields| {
            fields
                .iter()
                .map(|f| format!("{}: {}", s(f, "field"), s(f, "message")))
                .collect()
        })
        .unwrap_or_default()
}

// ── 표·값 도우미 ──

pub fn table(headers: &[String]) -> Table {
    let mut t = Table::new();
    // 폭을 모르는 터미널(파이프, 크기 0인 pty)에서는 줄이지 않는다
    let known_width = console::Term::stdout().size_checked().is_some_and(|(_, w)| w >= 40);
    t.load_preset(presets::NOTHING)
        .set_content_arrangement(if known_width {
            ContentArrangement::Dynamic
        } else {
            ContentArrangement::Disabled
        })
        .set_header(headers.iter().map(|h| style(h).dim().to_string()));
    t
}

pub fn print_table(t: Table) {
    // comfy-table은 줄 앞에 공백 하나를 둔다
    for line in t.to_string().lines() {
        println!("{}", line.strip_prefix(' ').unwrap_or(line).trim_end());
    }
}

/// 이름: 값 목록 (라벨 폭 맞춤)
pub fn print_kv(rows: &[(String, String)]) {
    let width = rows
        .iter()
        .map(|(k, _)| console::measure_text_width(k))
        .max()
        .unwrap_or(0);
    for (k, v) in rows {
        let pad = width.saturating_sub(console::measure_text_width(k));
        println!("  {}{}  {}", style(k).dim(), " ".repeat(pad), v);
    }
}

pub fn heading(text: &str) {
    println!("{}", style(text).bold());
}

/// 서버 상태 표시. 새 상태 문자열이 와도 그대로 보인다.
pub fn status_badge(status: &str) -> String {
    let label = crate::i18n::try_t(&format!("status.{}", status)).unwrap_or_else(|| status.to_string());
    let dot: StyledObject<&str> = match status {
        "running" => style("●").green(),
        "starting" | "provisioning" | "stopping" => style("◐").yellow(),
        "crashed" | "error" => style("●").red(),
        _ => style("○").dim(),
    };
    format!("{} {}", dot, label)
}

pub fn yes_no(v: bool) -> String {
    if v {
        style(t!("common.yes")).green().to_string()
    } else {
        style(t!("common.no")).dim().to_string()
    }
}

/// 문자열 필드 (없으면 빈 문자열)
pub fn s<'a>(v: &'a Value, key: &str) -> &'a str {
    v.get(key).and_then(Value::as_str).unwrap_or("")
}

/// 값 하나를 사람이 읽는 문자열로
pub fn show(v: &Value) -> String {
    match v {
        Value::Null => style("—").dim().to_string(),
        Value::String(s) if s.is_empty() => style("\"\"").dim().to_string(),
        Value::String(s) => s.clone(),
        Value::Bool(b) => yes_no(*b),
        // 단순 값 목록은 쉼표로 (["rcon","stdin"] → rcon, stdin)
        Value::Array(a) if a.is_empty() => style("—").dim().to_string(),
        Value::Array(a) if a.iter().all(|x| !x.is_object() && !x.is_array()) => {
            a.iter().map(show).collect::<Vec<_>>().join(", ")
        }
        other => other.to_string(),
    }
}

pub fn dim(text: &str) -> String {
    style(text).dim().to_string()
}

pub fn bold(text: &str) -> String {
    style(text).bold().to_string()
}

/// 비밀번호처럼 보이는 값은 가린다
pub fn mask(key: &str, v: &Value) -> String {
    let k = key.to_ascii_lowercase();
    let secret = k.contains("password") || k.contains("token") || k.contains("secret");
    match v {
        Value::String(s) if secret && !s.is_empty() => "••••••".into(),
        _ => show(v),
    }
}

/// epoch 초 → "3분 전" 같은 상대 시각
pub fn ago(epoch_secs: u64) -> String {
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(epoch_secs);
    let d = now.saturating_sub(epoch_secs);
    let (n, unit) = match d {
        0..=59 => (d, "sec"),
        60..=3599 => (d / 60, "min"),
        3600..=86399 => (d / 3600, "hour"),
        _ => (d / 86400, "day"),
    };
    t!(&format!("time.{}_ago", unit), n = n)
}

/// 초 → "1시간 3분"
pub fn duration(secs: u64) -> String {
    let (h, m, s) = (secs / 3600, (secs % 3600) / 60, secs % 60);
    if h > 0 {
        t!("time.hm", h = h, m = m)
    } else if m > 0 {
        t!("time.ms", m = m, s = s)
    } else {
        t!("time.s", s = s)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn masks_secret_values_only() {
        assert_eq!(mask("rcon_password", &json!("abc")), "••••••");
        assert_eq!(mask("server_name", &json!("abc")), "abc");
        assert_eq!(mask("admin_password", &json!("")), show(&json!("")));
    }
}
