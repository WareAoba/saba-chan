//! 출력 — 사람용(색·표)과 스크립트용(`--json`) 두 갈래
//!
//! 규칙: `--json`이면 stdout에는 JSON 문서 하나만 나간다. 사람용 안내·진행 표시는 stderr로.
//! 여러 단계를 거치는 명령(데몬 재시작 등)도 마지막 결과(또는 오류) 하나만 낸다: 문서를 모아 두었다가
//! 명령이 끝날 때 `flush()`가 찍는다.
//! 사람 모드에서 결과는 stdout, 경고·오류는 stderr.
//! 모든 줄은 `write_line` 한 창구를 지난다 (`say!`·`tell!`). TUI는 창구를 바꿔 끼워 결과를 패널에 그린다.

use comfy_table::{presets, ContentArrangement, Table};
use console::{style, StyledObject};
use serde_json::Value;

/// 한 줄을 결과(stdout)로
macro_rules! say {
    () => { $crate::output::write_line($crate::output::Stream::Out, "") };
    ($($arg:tt)*) => { $crate::output::write_line($crate::output::Stream::Out, &format!($($arg)*)) };
}

/// 한 줄을 안내(stderr)로
macro_rules! tell {
    () => { $crate::output::write_line($crate::output::Stream::Err, "") };
    ($($arg:tt)*) => { $crate::output::write_line($crate::output::Stream::Err, &format!($($arg)*)) };
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Stream {
    /// 결과
    Out,
    /// 안내·경고·오류
    Err,
}

pub type Sink = Box<dyn Fn(Stream, &str) + Send + Sync>;

/// 바꿔 끼운 창구. 없으면 터미널
static SINK: std::sync::RwLock<Option<Sink>> = std::sync::RwLock::new(None);

pub fn write_line(stream: Stream, text: &str) {
    if let Some(sink) = SINK.read().ok().as_ref().and_then(|s| s.as_ref()) {
        return sink(stream, text);
    }
    match stream {
        Stream::Out => println!("{}", text),
        Stream::Err => eprintln!("{}", text),
    }
}

/// 창구를 바꿔 끼웠는지 (TUI 안에서 도는 명령인지)
pub fn redirected() -> bool {
    SINK.read().map(|s| s.is_some()).unwrap_or(false)
}

/// 창구를 바꿔 끼운다. 놓으면 터미널로 돌아간다 (TUI가 끼운다)
#[must_use]
pub struct SinkGuard(());

impl SinkGuard {
    pub fn install(sink: Sink) -> Self {
        if let Ok(mut s) = SINK.write() {
            *s = Some(sink);
        }
        Self(())
    }
}

impl Drop for SinkGuard {
    fn drop(&mut self) {
        if let Ok(mut s) = SINK.write() {
            *s = None;
        }
    }
}

/// `--json`에서 아직 찍지 않은 마지막 문서
static PENDING: std::sync::Mutex<Option<Value>> = std::sync::Mutex::new(None);

fn emit(value: Value) {
    if let Ok(mut p) = PENDING.lock() {
        *p = Some(value);
    }
}

/// `--json-stream`: 결과도 진행 줄처럼 한 줄로
static STREAM: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);

pub fn set_stream(on: bool) {
    STREAM.store(on, std::sync::atomic::Ordering::Relaxed);
}

/// 따라가기 명령(`events`, `audit -f`)의 JSON 한 줄. 스트림 모드면 진행·결과 줄과 나눌 수 있게 `type:"event"`를 붙인다
pub fn event_line(mut v: Value) -> String {
    if STREAM.load(std::sync::atomic::Ordering::Relaxed) {
        if let Some(o) = v.as_object_mut() {
            o.insert("type".into(), "event".into());
        }
    }
    v.to_string()
}

/// 모아 둔 JSON 문서를 찍는다 (명령 하나가 끝날 때)
pub fn flush() {
    if let Some(v) = PENDING.lock().ok().and_then(|mut p| p.take()) {
        say!("{}", document(v, STREAM.load(std::sync::atomic::Ordering::Relaxed)));
    }
}

fn document(v: Value, stream: bool) -> String {
    if stream {
        serde_json::json!({ "type": "result", "data": v }).to_string()
    } else {
        serde_json::to_string_pretty(&v).unwrap_or_default()
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
            say!("{} {}", style("✓").green().bold(), msg);
        }
    }

    /// 진행 중 안내 (stderr, JSON 모드에서도 사람이 보게)
    pub fn note(&self, msg: &str) {
        tell!("{} {}", style("·").dim(), msg);
    }

    pub fn warn(&self, msg: &str) {
        tell!("{} {}", style("!").yellow().bold(), msg);
    }

    pub fn hint(&self, msg: &str) {
        if !self.json {
            tell!("  {}", style(msg).dim());
        }
    }

    pub fn error(&self, err: &crate::error::CliError) {
        if self.json {
            emit(err.to_json());
            return;
        }
        tell!("{} {}", style("✗").red().bold(), err);
        if let crate::error::CliError::Api { body, .. } = err {
            for line in validation_lines(body) {
                tell!("  {}", line);
            }
        }
        if let Some(h) = err.hint() {
            tell!("  {}", style(h).dim());
        }
    }
}

/// 검증 실패를 한 줄씩: `fields: [{field, message}]` 또는 `details: ["..."]`
fn validation_lines(body: &Value) -> Vec<String> {
    let fields = body.get("fields").and_then(Value::as_array).into_iter().flatten();
    let details = body.get("details").and_then(Value::as_array).into_iter().flatten();
    fields
        .map(|f| format!("{}: {}", s(f, "field"), s(f, "message")))
        .chain(details.filter_map(|d| d.as_str().map(str::to_string)))
        .collect()
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
        say!("{}", line.strip_prefix(' ').unwrap_or(line).trim_end());
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
        say!("  {}{}  {}", style(k).dim(), " ".repeat(pad), v);
    }
}

pub fn heading(text: &str) {
    say!("{}", style(text).bold());
}

/// 서버 상태 표시. 새 상태 문자열이 와도 그대로 보인다.
pub fn status_badge(status: &str) -> String {
    let label = crate::i18n::try_t(&format!("status.{}", status)).unwrap_or_else(|| status.to_string());
    let dot: StyledObject<&str> = match status {
        "running" | "connected" => style("●").green(),
        "starting" | "provisioning" | "stopping" => style("◐").yellow(),
        "crashed" | "error" => style("●").red(),
        _ => style("○").dim(),
    };
    format!("{} {}", dot, label)
}

/// 모듈·익스텐션 출처 표시 (`trust`). 새 값이 와도 그대로 보인다.
pub fn trust_badge(trust: &str) -> String {
    if trust.is_empty() {
        return String::new();
    }
    let label = crate::i18n::try_t(&format!("trust.{}", trust)).unwrap_or_else(|| trust.to_string());
    match trust {
        "verified" => style(format!("✓ {}", label)).green().to_string(),
        "modified" => style(format!("! {}", label)).yellow().to_string(),
        _ => style(format!("? {}", label)).dim().to_string(),
    }
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
        // 단순 값 목록은 쉼표로 (["console","rest"] → console, rest)
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

/// UTC ISO-8601 시각(`2026-10-19T17:02:57.000Z`) → epoch 초. 다른 모양이면 None
pub fn iso_epoch(iso: &str) -> Option<u64> {
    let (date, time) = iso.trim().strip_suffix('Z')?.split_once('T')?;
    let mut d = date.splitn(3, '-').map(|p| p.parse::<i64>().ok());
    let (y, mo, day) = (d.next()??, d.next()??, d.next()??);
    let mut t = time.split('.').next()?.splitn(3, ':').map(|p| p.parse::<i64>().ok());
    let (h, mi, s) = (t.next()??, t.next()??, t.next()??);
    // 그레고리력 날짜 → 1970-01-01부터의 일수
    let (y, m) = if mo <= 2 { (y - 1, mo + 9) } else { (y, mo - 3) };
    let era = y.div_euclid(400);
    let yoe = y - era * 400;
    let doy = (153 * m + 2) / 5 + day - 1;
    let days = era * 146097 + yoe * 365 + yoe / 4 - yoe / 100 + doy - 719468;
    u64::try_from(days * 86400 + h * 3600 + mi * 60 + s).ok()
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

/// 데몬이 알려 준 접속자 수 → "3/20" (최대를 모르면 "3")
pub fn players(server: &Value) -> Option<String> {
    let p = server.get("players")?;
    let online = p.get("online")?.as_u64()?;
    Some(match p.get("max").and_then(Value::as_u64) {
        Some(max) => format!("{online}/{max}"),
        None => online.to_string(),
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    /// `--json-stream`의 마지막 줄은 결과(실패면 오류 문서)를 한 줄로 싣는다
    #[test]
    fn stream_result_is_one_line() {
        let line = document(json!({ "ok": false, "error_code": "usage" }), true);
        assert!(!line.contains('\n'));
        assert_eq!(
            serde_json::from_str::<Value>(&line).unwrap(),
            json!({ "type": "result", "data": { "ok": false, "error_code": "usage" } })
        );
        assert!(document(json!({ "a": 1 }), false).contains('\n'));
    }

    /// 사람용 출력은 모두 한 창구를 지난다: 나중에 TUI가 창구를 바꿔 끼우면 명령 결과를 패널에 그릴 수 있다
    #[test]
    fn human_output_goes_through_the_installed_sink() {
        let seen = std::sync::Arc::new(std::sync::Mutex::new(Vec::<(Stream, String)>::new()));
        let rec = seen.clone();
        let _guard = SinkGuard::install(Box::new(move |stream, text| {
            rec.lock()
                .unwrap()
                .push((stream, console::strip_ansi_codes(text).to_string()))
        }));
        let out = Out { json: false };
        out.done("done-msg", &json!({}));
        out.note("note-msg");
        say!("say {}", 1);
        tell!("tell {}", 2);
        say!();
        heading("head");
        print_kv(&[("k".into(), "v".into())]);
        let mut t = table(&["col".into()]);
        t.add_row(vec!["cell"]);
        print_table(t);
        let got = seen.lock().unwrap().clone();
        let has = |stream: Stream, needle: &str| got.iter().any(|(s, l)| *s == stream && l.contains(needle));
        assert!(has(Stream::Out, "done-msg"));
        assert!(has(Stream::Err, "note-msg"));
        assert!(has(Stream::Out, "say 1"));
        assert!(has(Stream::Err, "tell 2"));
        assert!(got.contains(&(Stream::Out, String::new())));
        assert!(has(Stream::Out, "head"));
        assert!(has(Stream::Out, "k  v"));
        assert!(has(Stream::Out, "cell"));
    }

    /// 명령 코드가 터미널에 직접 찍거나(막대·질문 포함) 하면 TUI 패널을 우회한다.
    /// 출력은 `say!`·`tell!`, 진행 표시는 `progress`, 질문은 `prompt`만 쓴다
    #[test]
    fn no_command_talks_to_the_terminal_directly() {
        let rules: [(&str, &str); 6] = [
            ("println!(", "output.rs"),
            ("eprintln!(", "output.rs"),
            ("print!(", "output.rs"),
            ("eprint!(", "output.rs"),
            ("indicatif", "progress.rs"),
            ("dialoguer", "prompt.rs"),
        ];
        let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("src");
        let mut stack = vec![root];
        let mut bad = Vec::new();
        while let Some(dir) = stack.pop() {
            for e in std::fs::read_dir(&dir).unwrap().flatten() {
                let path = e.path();
                if path.is_dir() {
                    stack.push(path);
                    continue;
                }
                if path.extension().and_then(|x| x.to_str()) != Some("rs") {
                    continue;
                }
                // 시험 모듈은 터미널에 쓰지 않으므로 보지 않는다 (이 시험의 규칙 문자열도 거기 있다)
                let source = std::fs::read_to_string(&path).unwrap();
                let body = source.split("#[cfg(test)]").next().unwrap_or("");
                for (i, line) in body.lines().enumerate() {
                    let code = line.split("//").next().unwrap_or("");
                    if rules.iter().any(|(m, home)| code.contains(m) && !path.ends_with(home)) {
                        bad.push(format!("{}:{}", path.display(), i + 1));
                    }
                }
            }
        }
        assert!(bad.is_empty(), "정해진 창구 밖에서 터미널을 직접 씀: {bad:#?}");
    }

    /// 디스코드 연결 상태 `connected`도 켜진 점으로 (빈 원이면 끊긴 것처럼 보인다)
    #[test]
    fn a_connected_badge_has_a_filled_dot() {
        assert!(console::strip_ansi_codes(&status_badge("connected")).starts_with('●'));
        assert!(console::strip_ansi_codes(&status_badge("disconnected")).starts_with('○'));
    }

    #[test]
    fn iso_times_from_the_relay_become_epoch_seconds() {
        assert_eq!(iso_epoch("1970-01-02T00:00:00.000Z"), Some(86400));
        assert_eq!(iso_epoch("2026-10-19T17:02:57.000Z"), Some(1792429377));
        assert_eq!(iso_epoch("2026-10-19T17:02:57Z"), Some(1792429377));
        assert_eq!(iso_epoch("2026-10-19 17:02"), None);
    }

    #[test]
    fn masks_secret_values_only() {
        assert_eq!(mask("rcon_password", &json!("abc")), "••••••");
        assert_eq!(mask("server_name", &json!("abc")), "abc");
        assert_eq!(mask("admin_password", &json!("")), show(&json!("")));
    }

    #[test]
    fn shows_players_only_when_the_daemon_knows_them() {
        assert_eq!(
            players(&json!({ "players": { "online": 3, "max": 20 } })).as_deref(),
            Some("3/20")
        );
        assert_eq!(players(&json!({ "players": { "online": 0 } })).as_deref(), Some("0"));
        assert_eq!(players(&json!({ "status": "running" })), None);
    }
}
