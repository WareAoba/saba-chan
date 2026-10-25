//! 콘솔 줄 토큰화 — `GET /api/instance/:id/console?spans=1`
//!
//! 모듈 `[[syntax_highlight.rules]]`와 데몬 공통 규칙으로 줄을 의미 토큰 구간으로 나눈다.
//! 색은 클라이언트 테마가 토큰별로 정한다. 구간 오프셋은 UTF-16 코드 단위다(GUI가 그대로 `slice`).
//!
//! 겹치면 priority가 큰 규칙이 이기고, 같으면 먼저 선언된 규칙이 이긴다(이미 칠한 칸은 덮지 않는다).
//! 모듈 규칙이 먼저, 공통 규칙은 그 뒤에 남은 칸만 칠한다.

use super::module_loader::{HighlightRule, SyntaxHighlight};
use regex::Regex;
use std::collections::HashMap;
use std::hash::{Hash, Hasher};
use std::sync::{Arc, Mutex, OnceLock};

/// 클라이언트가 아는 토큰 전부
pub const TOKENS: [&str; 16] = [
    "error",
    "warn",
    "info",
    "debug",
    "success",
    "string",
    "number",
    "keyword",
    "timestamp",
    "player",
    "command",
    "selector",
    "ip",
    "uuid",
    "path",
    "dim",
];

/// 이 길이(바이트)를 넘는 줄은 앞부분만 토큰화한다
const MAX_LINE_BYTES: usize = 4096;

/// 공통 규칙: (패턴, 토큰). 모든 모듈 규칙보다 뒤에 칠한다
const BUILTIN_RULES: [(&str, &str); 7] = [
    (r"^\s+at \S+\(.*\)\s*$", "dim"),
    (r"^\s*\.\.\. \d+ more\s*$", "dim"),
    (r"^Caused by: .*", "error"),
    (r"\b(?P<hl>\d{1,3}(?:\.\d{1,3}){3}(?::\d{1,5})?)\b", "ip"),
    (
        r"\b[0-9a-fA-F]{8}-[0-9a-fA-F]{4}-[0-9a-fA-F]{4}-[0-9a-fA-F]{4}-[0-9a-fA-F]{12}\b",
        "uuid",
    ),
    (r#""[^"]*""#, "string"),
    (r"\b\d+(?:\.\d+)?%", "number"),
];

/// 모듈이 쓰는 이름 → 토큰. 옛 별칭을 받고, 색 값이나 모르는 이름은 None
pub fn normalize_token(color: &str) -> Option<&'static str> {
    let lower = color.trim().to_ascii_lowercase();
    let name = match lower.as_str() {
        "warning" => "warn",
        "trace" => "debug",
        "comment" => "dim",
        other => other,
    };
    TOKENS.iter().copied().find(|t| *t == name)
}

struct Compiled {
    regex: Regex,
    has_hl: bool,
    token: &'static str,
}

/// 한 모듈의 규칙을 컴파일한 것
pub struct Highlighter {
    rules: Vec<Compiled>,
}

impl Highlighter {
    /// 모듈 규칙 + 공통 규칙. 쓸 수 없는 모듈 규칙은 경고만 남기고 뺀다
    pub fn new(module: &str, rules: &[HighlightRule]) -> Self {
        let mut ordered: Vec<&HighlightRule> = rules.iter().collect();
        ordered.sort_by_key(|r| std::cmp::Reverse(r.priority)); // 안정 정렬: 같으면 선언 순서
        let mut compiled = Vec::with_capacity(ordered.len() + BUILTIN_RULES.len());
        for rule in ordered {
            let Some(token) = normalize_token(&rule.color) else {
                tracing::warn!(
                    "Module '{}' highlight rule '{}': '{}' is not a highlight token; rule skipped",
                    module,
                    rule.name,
                    rule.color
                );
                continue;
            };
            match Regex::new(&rule.pattern) {
                Ok(regex) => compiled.push(Compiled {
                    has_hl: regex.capture_names().any(|n| n == Some("hl")),
                    regex,
                    token,
                }),
                Err(e) => tracing::warn!(
                    "Module '{}' highlight rule '{}': pattern not supported, rule skipped: {}",
                    module,
                    rule.name,
                    e
                ),
            }
        }
        for (pattern, token) in BUILTIN_RULES {
            let regex = Regex::new(pattern).expect("builtin highlight rule");
            compiled.push(Compiled {
                has_hl: regex.capture_names().any(|n| n == Some("hl")),
                regex,
                token,
            });
        }
        Self { rules: compiled }
    }

    /// `[[start, end, token]]` (UTF-16 오프셋, 오름차순, 겹치지 않음)
    pub fn spans(&self, line: &str) -> Vec<(usize, usize, &'static str)> {
        let mut cut = line.len().min(MAX_LINE_BYTES);
        while !line.is_char_boundary(cut) {
            cut -= 1;
        }
        let text = &line[..cut];
        if text.is_empty() {
            return Vec::new();
        }

        // 바이트마다 칠한 규칙의 토큰
        let mut paint: Vec<Option<&'static str>> = vec![None; text.len()];
        for rule in &self.rules {
            let mut hit = |start: usize, end: usize| {
                for slot in &mut paint[start..end] {
                    slot.get_or_insert(rule.token);
                }
            };
            if rule.has_hl {
                for caps in rule.regex.captures_iter(text) {
                    match caps.name("hl") {
                        Some(m) => hit(m.start(), m.end()),
                        None => {
                            let m = caps.get(0).expect("whole match");
                            hit(m.start(), m.end());
                        }
                    }
                }
            } else {
                for m in rule.regex.find_iter(text) {
                    hit(m.start(), m.end());
                }
            }
        }

        // 바이트 구간 → UTF-16 구간 (같은 토큰이 이어지면 하나로)
        let mut spans: Vec<(usize, usize, &'static str)> = Vec::new();
        let mut utf16 = 0;
        for (byte, ch) in text.char_indices() {
            let width = ch.len_utf16();
            if let Some(token) = paint[byte] {
                match spans.last_mut() {
                    Some(last) if last.1 == utf16 && last.2 == token => last.1 += width,
                    _ => spans.push((utf16, utf16 + width, token)),
                }
            }
            utf16 += width;
        }
        spans
    }
}

/// 모듈 이름 → (규칙 지문, 컴파일 결과)
type HighlightCache = HashMap<String, (u64, Arc<Highlighter>)>;

/// 모듈 이름별 컴파일 캐시. 규칙이 바뀌면(모듈 업데이트) 지문이 달라져 다시 만든다
pub fn for_module(module: &str, highlight: Option<&SyntaxHighlight>) -> Arc<Highlighter> {
    static CACHE: OnceLock<Mutex<HighlightCache>> = OnceLock::new();
    let rules: &[HighlightRule] = highlight.map(|h| h.rules.as_slice()).unwrap_or(&[]);
    let mut hasher = std::collections::hash_map::DefaultHasher::new();
    for r in rules {
        (&r.name, &r.pattern, &r.color, r.priority).hash(&mut hasher);
    }
    let fingerprint = hasher.finish();

    let mut cache = CACHE
        .get_or_init(Default::default)
        .lock()
        .unwrap_or_else(|e| e.into_inner());
    if let Some((fp, hl)) = cache.get(module) {
        if *fp == fingerprint {
            return hl.clone();
        }
    }
    let hl = Arc::new(Highlighter::new(module, rules));
    cache.insert(module.to_string(), (fingerprint, hl.clone()));
    hl
}

/// 데몬 로그 규칙: (이름, 패턴, 토큰). daemon_log.rs가 tracing 메시지에 `키=값` 필드를 이어 붙인 줄을 칠한다.
/// 레벨·시각은 줄의 따로 있는 칸이라 여기서 칠하지 않는다. 앞에 둔 규칙이 이긴다
const DAEMON_LOG_RULES: [(&str, &str, &str); 14] = [
    ("tag", r"^\[[A-Za-z][\w .:/-]{0,40}\]", "keyword"),
    ("url", r"\bhttps?://[^\s'\x22]+", "path"),
    ("ip", r"\b(?P<hl>\d{1,3}(?:\.\d{1,3}){3}(?::\d{1,5})?)\b", "ip"),
    ("uuid", r"\b[0-9a-fA-F]{8}-[0-9a-fA-F]{4}-[0-9a-fA-F]{4}-[0-9a-fA-F]{4}-[0-9a-fA-F]{12}\b", "uuid"),
    // 버전은 한 덩어리 (`v0.1.5`, `1.0.5.102999`, `0.1.11-beta.2`). 점 하나짜리 소수(`12.5 ms`)는 아래 measure·number가 맡는다
    ("version", r"\b(?:[vV]\d+(?:\.\d+)+|\d+(?:\.\d+){2,})(?:-[0-9A-Za-z]+(?:\.[0-9A-Za-z]+)*)?\b", "number"),
    ("windows_path", r"\b[A-Za-z]:\\[^\s'\x22,;)]*", "path"),
    ("unix_path", r"(?:^|[\s(=\x22'])(?P<hl>/(?:[\w.@+-]+/)*[\w.@+-]+)", "path"),
    ("name", r"(?:^|[\s(=:,\[])(?P<hl>'[^'\n]{1,120}')", "string"),
    ("field", r"(?:^|\s)(?P<hl>[a-z_][a-z0-9_]*)=", "dim"),
    ("failure", r"\b(?:[Ff]ailed|[Ee]rror|[Cc]annot|[Cc]ould not|panicked|denied|refused|timed out|[Uu]nable to)\b", "error"),
    ("success", r"\b(?:[Ss]uccessfully|[Cc]ompleted|[Rr]eady|[Ss]tarted|[Cc]onnected|[Ll]oaded)\b", "success"),
    ("measure", r"\b\d+(?:\.\d+)?\s?(?:ms|s|KB|MB|GB|B|%)(?:\b|$)", "number"),
    ("number", r"\b\d+(?:\.\d+)*\b", "number"),
    ("arrow", r"→|->", "dim"),
];

/// 데몬 자체 로그(`GET /api/daemon/console?spans=1`)의 하이라이터
pub fn daemon_log() -> Arc<Highlighter> {
    static HL: OnceLock<Arc<Highlighter>> = OnceLock::new();
    HL.get_or_init(|| {
        let n = DAEMON_LOG_RULES.len() as i32;
        let rules: Vec<HighlightRule> = DAEMON_LOG_RULES
            .iter()
            .enumerate()
            .map(|(i, (name, pattern, token))| HighlightRule {
                name: (*name).into(),
                pattern: (*pattern).into(),
                color: (*token).into(),
                background: None,
                bold: false,
                italic: false,
                underline: false,
                priority: n - i as i32,
            })
            .collect();
        Arc::new(Highlighter::new("daemon", &rules))
    })
    .clone()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn rule(name: &str, pattern: &str, color: &str, priority: i32) -> HighlightRule {
        HighlightRule {
            name: name.into(),
            pattern: pattern.into(),
            color: color.into(),
            background: None,
            bold: false,
            italic: false,
            underline: false,
            priority,
        }
    }

    #[test]
    fn hl_group_paints_only_the_group() {
        let hl = Highlighter::new("m", &[rule("join", r"(?P<hl>\w+) joined the game", "player", 0)]);
        assert_eq!(hl.spans("Steve joined the game"), vec![(0, 5, "player")]);
    }

    #[test]
    fn higher_priority_wins_then_declaration_order() {
        let hl = Highlighter::new(
            "m",
            &[
                rule("low", r"abc", "info", 0),
                rule("high", r"b", "error", 10),
                rule("same_late", r"c", "warn", 0),
            ],
        );
        assert_eq!(hl.spans("abc"), vec![(0, 1, "info"), (1, 2, "error"), (2, 3, "info")]);
    }

    #[test]
    fn offsets_are_utf16_units() {
        // 한글은 UTF-8 3바이트, UTF-16 1단위. 이모지는 UTF-16 2단위
        let hl = Highlighter::new("m", &[rule("chat", r"<(?P<hl>\w+)>", "player", 0)]);
        assert_eq!(hl.spans("안녕 😀 <철수> 하이"), vec![(7, 9, "player")]);
    }

    #[test]
    fn aliases_normalize_and_colors_are_dropped() {
        assert_eq!(normalize_token("warning"), Some("warn"));
        assert_eq!(normalize_token("Comment"), Some("dim"));
        assert_eq!(normalize_token("#ff0000"), None);
        let hl = Highlighter::new("m", &[rule("hex", r"x", "#ff0000", 0), rule("ok", r"y", "warning", 0)]);
        assert_eq!(hl.spans("xy"), vec![(1, 2, "warn")]);
    }

    #[test]
    fn unsupported_pattern_is_skipped_not_fatal() {
        let hl = Highlighter::new(
            "m",
            &[rule("look", r"(?<=a)b", "error", 0), rule("ok", r"c", "info", 0)],
        );
        assert_eq!(hl.spans("abc"), vec![(2, 3, "info")]);
    }

    #[test]
    fn module_rules_beat_builtin_rules() {
        let hl = Highlighter::new("m", &[rule("addr", r"/(?P<hl>[\d.:]+)", "player", -50)]);
        assert_eq!(hl.spans("Steve[/10.0.0.5:49152] logged in"), vec![(7, 21, "player")]);
    }

    #[test]
    fn builtin_rules_cover_java_stack_and_common_values() {
        let hl = Highlighter::new("m", &[]);
        assert_eq!(hl.spans("\tat zombie.Foo.bar(Foo.java:75)"), vec![(0, 31, "dim")]);
        assert_eq!(hl.spans("Caused by: java.lang.Oops"), vec![(0, 25, "error")]);
        assert_eq!(hl.spans("from 127.0.0.1:25575 ok"), vec![(5, 20, "ip")]);
        assert_eq!(hl.spans(r#"Preparing level "world""#), vec![(16, 23, "string")]);
        assert_eq!(hl.spans("spawn area: 52%"), vec![(12, 15, "number")]);
    }

    #[test]
    fn long_lines_only_tokenize_the_head() {
        let hl = Highlighter::new("m", &[rule("x", r"X", "error", 0)]);
        let line = format!("{}X", "a".repeat(MAX_LINE_BYTES + 10));
        assert!(hl.spans(&line).is_empty());
        // 경계가 글자 중간이어도 자르기가 안전하다
        let multibyte = "가".repeat(MAX_LINE_BYTES);
        assert!(hl.spans(&multibyte).is_empty());
    }

    /// 데몬 로그는 `[태그] 글 'id' 경로 키=값 숫자` 모양이다 (daemon_log.rs가 tracing 메시지와 필드를 이어 붙인다)
    #[test]
    fn daemon_log_lines_are_tokenized() {
        let hl = daemon_log();
        let span = |line: &str, part: &str| {
            let at = line.find(part).expect("part");
            let units = |b: usize| line[..b].encode_utf16().count();
            (units(at), units(at + part.len()))
        };
        let token_of = |line: &str, part: &str| {
            let (st, en) = span(line, part);
            hl.spans(line).into_iter().find(|(a, b, _)| *a <= st && en <= *b).map(|(_, _, t)| t)
        };
        let l = "[Heartbeat] Active clients: 2";
        assert_eq!(token_of(l, "[Heartbeat]"), Some("keyword"));
        assert_eq!(token_of(l, "2"), Some("number"));
        let l = "Managed server 'pal-1' started with PID 4242";
        assert_eq!(token_of(l, "'pal-1'"), Some("string"));
        assert_eq!(token_of(l, "4242"), Some("number"));
        let l = r"Failed to save exit history for 'mc': C:\Users\me\saba\x.json is busy";
        assert_eq!(token_of(l, "Failed"), Some("error"));
        assert_eq!(token_of(l, r"C:\Users\me\saba\x.json"), Some("path"));
        let l = "IPC HTTP server starting on 127.0.0.1:57474 instance=3f2a0b1c-1111-2222-3333-444455556666";
        assert_eq!(token_of(l, "127.0.0.1:57474"), Some("ip"));
        assert_eq!(token_of(l, "instance"), Some("dim"));
        assert_eq!(token_of(l, "3f2a0b1c-1111-2222-3333-444455556666"), Some("uuid"));
        let l = "[Sandbox] granting \"x\" on /srv/saba/instances/a took 12 ms";
        assert_eq!(token_of(l, "/srv/saba/instances/a"), Some("path"));
        assert_eq!(token_of(l, "12 ms"), Some("number"));
        assert_eq!(token_of("[Updates] fetched https://example.com/a.json", "https://example.com/a.json"), Some("path"));
        // 낱말 속 작은따옴표는 이름이 아니다
        assert_eq!(token_of("the user's choice (it's fine)", "'s choice (it'"), None);
    }

    /// 버전 번호는 한 덩어리다: `v0.1.5`, `v1.0.5.102999`, `0.1.10-beta.2`가 중간에 끊기지 않는다 (IP로 읽지도 않는다)
    #[test]
    fn daemon_log_versions_are_one_token() {
        let hl = daemon_log();
        let whole = |line: &str, part: &str| {
            let at = line.find(part).expect("part");
            let units = |b: usize| line[..b].encode_utf16().count();
            let (st, en) = (units(at), units(at + part.len()));
            hl.spans(line).into_iter().any(|(a, b, t)| a == st && b == en && t == "number")
        };
        assert!(whole("Loaded module palworld v0.1.5", "v0.1.5"));
        assert!(whole("server version v1.0.5.102999 ready", "v1.0.5.102999"));
        assert!(whole("[Updates] core 0.1.10 -> 0.1.11-beta.2", "0.1.11-beta.2"));
        assert!(!whole("IPC on 127.0.0.1:57474", "127.0.0.1:57474"), "an address is still an ip");
    }

    #[test]
    fn cache_rebuilds_when_rules_change() {
        let a = SyntaxHighlight {
            rules: vec![rule("r", "a", "info", 0)],
        };
        let b = SyntaxHighlight {
            rules: vec![rule("r", "a", "error", 0)],
        };
        assert_eq!(for_module("cache-test", Some(&a)).spans("a"), vec![(0, 1, "info")]);
        assert_eq!(for_module("cache-test", Some(&b)).spans("a"), vec![(0, 1, "error")]);
    }
}
