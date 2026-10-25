//! 콘솔 줄 칠하기. 토큰 구간은 데몬이 정한다(`?spans=1`, 서버 콘솔은 모듈 규칙, 데몬 로그는 데몬 규칙).
//! 여기서는 토큰 → 색과, 줄의 레벨·출처·시각을 그린다. 토큰 색은 `console` 명령(cmd/console.rs)과 같다

use super::data::{clock_fmt, s};
use ratatui::style::{Color, Modifier, Style};
use ratatui::text::{Line, Span};
use serde_json::Value;
use unicode_width::UnicodeWidthChar;

/// 토큰 → 색 (GUI 테마와 같은 이름)
pub fn token_style(token: &str) -> Style {
    let st = Style::default();
    match token {
        "error" => st.fg(Color::Red).add_modifier(Modifier::BOLD),
        "warn" => st.fg(Color::Yellow).add_modifier(Modifier::BOLD),
        "info" => st.fg(Color::Blue),
        "debug" | "dim" | "timestamp" => st.fg(Color::DarkGray),
        "success" => st.fg(Color::Green).add_modifier(Modifier::BOLD),
        "string" => st.fg(Color::Green),
        "number" => st.fg(Color::Indexed(215)),
        "keyword" | "command" => st.fg(Color::Magenta),
        "player" => st.fg(Color::Cyan).add_modifier(Modifier::BOLD),
        "selector" => st.fg(Color::Yellow),
        "ip" | "uuid" => st.fg(Color::Cyan),
        "path" => st.fg(Color::Indexed(116)),
        _ => st,
    }
}

/// 글을 토큰 구간(UTF-16 오프셋)대로 나눠 칠한다. 구간 밖은 `base`. 어긋난 구간은 버린다
pub fn painted(text: &str, spans: Option<&Value>, base: Style) -> Vec<Span<'static>> {
    let mut ranges: Vec<(usize, usize, Style)> = Vec::new();
    if let Some(spans) = spans.and_then(Value::as_array) {
        // UTF-16 오프셋 → 바이트 오프셋 표 (마지막 원소는 끝)
        let mut bytes = Vec::with_capacity(text.len() + 1);
        for (b, ch) in text.char_indices() {
            for _ in 0..ch.len_utf16() {
                bytes.push(b);
            }
        }
        bytes.push(text.len());
        let byte_at = |u: u64| bytes.get(usize::try_from(u).ok()?).copied();
        for sp in spans {
            let (Some(st), Some(en), Some(tok)) = (
                sp.get(0).and_then(Value::as_u64).and_then(byte_at),
                sp.get(1).and_then(Value::as_u64).and_then(byte_at),
                sp.get(2).and_then(Value::as_str),
            ) else {
                continue;
            };
            if st < en && ranges.last().is_none_or(|last| last.1 <= st) {
                ranges.push((st, en, base.patch(token_style(tok))));
            }
        }
    }
    let mut out = Vec::new();
    let mut at = 0;
    for (st, en, style) in ranges {
        if at < st {
            out.push(Span::styled(text[at..st].to_string(), base));
        }
        out.push(Span::styled(text[st..en].to_string(), style));
        at = en;
    }
    if at < text.len() || out.is_empty() {
        out.push(Span::styled(text[at..].to_string(), base));
    }
    out
}

/// 서버 콘솔 한 줄 (`GET /api/instance/:id/console` 줄). 사바쨩이 낸 줄은 `» `, 레벨이 줄 바탕색
pub fn console_line(l: &Value) -> Vec<Span<'static>> {
    let raw = s(l, "content");
    let text = crate::cmd::console::strip_terminal_controls(raw);
    let base = match (s(l, "source"), s(l, "level")) {
        ("system", _) => Style::default().fg(Color::Cyan),
        (_, "error") => Style::default().fg(Color::Red),
        (_, "warn") => Style::default().fg(Color::Yellow),
        (_, "debug") => Style::default().fg(Color::DarkGray),
        _ => Style::default(),
    };
    // 제어 문자를 지웠으면 데몬이 센 오프셋이 맞지 않는다: 토큰 없이 바탕색만
    let spans = (text == raw).then(|| l.get("spans")).flatten();
    let mut out = Vec::new();
    if s(l, "source") == "system" {
        out.push(Span::styled("» ", base));
    }
    out.extend(painted(&text, spans, base));
    out
}

/// 데몬 로그 한 줄 (`GET /api/daemon/console` 줄): `시각 레벨 [태그 또는 모듈] 글`
pub fn daemon_line(l: &Value) -> Vec<Span<'static>> {
    let dim = Style::default().fg(Color::DarkGray);
    let level = s(l, "level").to_ascii_lowercase();
    let level_style = match level.as_str() {
        "error" => Style::default().fg(Color::Red).add_modifier(Modifier::BOLD),
        "warn" => Style::default().fg(Color::Yellow).add_modifier(Modifier::BOLD),
        "info" => Style::default().fg(Color::Green),
        "debug" => Style::default().fg(Color::Blue),
        _ => dim,
    };
    let at = l.get("timestamp").and_then(Value::as_u64).map(|t| clock_fmt(t, "%H:%M:%S")).unwrap_or_default();
    let message = crate::cmd::console::strip_terminal_controls(s(l, "message"));
    let mut out = vec![Span::styled(format!("{at} "), dim), Span::styled(format!("{:<5} ", level.to_ascii_uppercase()), level_style)];
    // 글에 `[태그]`가 없으면 어디서 났는지(모듈 경로의 첫 칸)를 흐리게
    if !message.starts_with('[') {
        let target = s(l, "target");
        let short = target.strip_prefix("saba_core::").unwrap_or(target).split("::").next().unwrap_or("");
        if !short.is_empty() {
            out.push(Span::styled(format!("{short} "), dim));
        }
    }
    let base = match level.as_str() {
        "error" => Style::default().fg(Color::LightRed),
        "warn" => Style::default().fg(Color::LightYellow),
        _ => Style::default(),
    };
    let spans = (message == s(l, "message")).then(|| l.get("spans")).flatten();
    out.extend(painted(&message, spans, base));
    out
}

/// 칠한 줄을 표시 폭으로 접는다 (한글은 두 칸). 색은 접힌 줄에도 남는다
pub fn wrap_spans(spans: Vec<Span<'static>>, width: usize) -> Vec<Line<'static>> {
    if width == 0 {
        return vec![Line::from(spans)];
    }
    let mut lines: Vec<Line<'static>> = Vec::new();
    let mut cur: Vec<Span<'static>> = Vec::new();
    let mut w = 0;
    for span in spans {
        let style = span.style;
        let mut piece = String::new();
        for ch in span.content.chars() {
            let cw = ch.width().unwrap_or(0);
            if w + cw > width && (w > 0 || !piece.is_empty()) {
                if !piece.is_empty() {
                    cur.push(Span::styled(std::mem::take(&mut piece), style));
                }
                lines.push(Line::from(std::mem::take(&mut cur)));
                w = 0;
            }
            piece.push(ch);
            w += cw;
        }
        if !piece.is_empty() {
            cur.push(Span::styled(piece, style));
        }
    }
    lines.push(Line::from(cur));
    lines
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn styles(spans: &[Span]) -> Vec<(String, Style)> {
        spans.iter().map(|s| (s.content.to_string(), s.style)).collect()
    }

    /// 데몬이 준 토큰 구간대로 칠하고, 오프셋은 UTF-16이다 (한글·이모지 뒤도 맞는다)
    #[test]
    fn tokens_paint_their_ranges_in_utf16() {
        let spans = painted("안녕 😀 <철수> 하이", Some(&json!([[7, 9, "player"]])), Style::default());
        assert_eq!(styles(&spans)[1], ("철수".to_string(), token_style("player")));
        // 어긋난 구간·겹친 구간은 버린다
        let spans = painted("abc", Some(&json!([[1, 99, "error"], [0, 2, "info"], [1, 3, "warn"]])), Style::default());
        assert_eq!(styles(&spans), vec![("ab".to_string(), token_style("info")), ("c".to_string(), Style::default())]);
    }

    /// 서버 콘솔: 사바쨩 줄은 `»`, 오류 줄은 붉은 바탕색 위에 토큰. 제어 문자를 지웠으면 토큰을 쓰지 않는다
    #[test]
    fn console_lines_carry_source_level_and_tokens() {
        let sys = console_line(&json!({ "content": "서버를 켰어요", "source": "system", "level": "info" }));
        assert_eq!(sys[0].content, "» ");
        let err = console_line(&json!({ "content": "Steve left", "source": "stdout", "level": "error", "spans": [[0, 5, "player"]] }));
        assert_eq!(err[0].style, Style::default().fg(Color::Red).patch(token_style("player")));
        assert_eq!(err[1].style.fg, Some(Color::Red));
        let colored = console_line(&json!({ "content": "\u{1b}[32mSteve\u{1b}[0m left", "spans": [[0, 5, "player"]] }));
        assert_eq!(styles(&colored), vec![("Steve left".to_string(), Style::default())]);
    }

    /// 데몬 로그: 시각·레벨을 앞에, 태그가 없으면 모듈 이름을 흐리게
    #[test]
    fn daemon_lines_show_time_level_and_where_they_came_from() {
        let tagged = daemon_line(&json!({ "timestamp": 1_700_000_000_000u64, "level": "warn", "target": "saba_core::ipc", "message": "[Heartbeat] x", "spans": [[0, 11, "keyword"]] }));
        let text: String = tagged.iter().map(|s| s.content.to_string()).collect();
        assert!(text.contains("WARN  [Heartbeat] x"), "{text}");
        assert_eq!(tagged[1].style.fg, Some(Color::Yellow));
        assert!(!text.contains("ipc"), "a tag says where it came from already: {text}");
        let plain = daemon_line(&json!({ "timestamp": 1, "level": "info", "target": "saba_core::supervisor::managed_process", "message": "Loaded 3 modules" }));
        let text: String = plain.iter().map(|s| s.content.to_string()).collect();
        assert!(text.contains("INFO  supervisor Loaded 3 modules"), "{text}");
    }

    /// 접어도 색이 남고, 두 칸 글자는 두 칸으로 센다
    #[test]
    fn wrapping_keeps_styles_and_counts_wide_characters() {
        let red = Style::default().fg(Color::Red);
        let lines = wrap_spans(vec![Span::raw("가나"), Span::styled("다라마", red)], 4);
        assert_eq!(lines.len(), 3);
        assert_eq!(lines[1].spans[0].style, red);
        assert_eq!(lines.iter().map(|l| l.to_string()).collect::<Vec<_>>(), vec!["가나", "다라", "마"]);
        assert_eq!(wrap_spans(vec![Span::raw("")], 5).len(), 1);
    }
}
