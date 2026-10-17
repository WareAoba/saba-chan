//! `console` — 서버 콘솔 보기·입력, 데몬 로그 따라가기
//!
//! 데몬은 줄마다 순번(`id`)을 주고 `?since=<id>`(그 번호 포함)로 이어 받게 한다.
//! 서버가 다시 뜨면 순번이 0부터 다시 시작하고, 꺼져 있을 때는 디스크 로그(`source: "disk"`)를 주며
//! 이때는 `since`가 무시된다. 그래서 실행 여부·출처가 바뀌면 처음부터 다시 읽는다.
//!
//! 입력 모드에서는 친 줄을 게임 명령으로 보낸다 (정의된 명령이면 인자를 묶고, 아니면 콘솔 한 줄).
//! `:`로 시작하는 줄은 CLI가 처리한다 (`:help`).

use crate::cli::ConsoleArgs;
use crate::ctx::Ctx;
use crate::error::Result;
use crate::output::s;
use crate::resolve::{self, Server};
use console::style;
use rustyline::error::ReadlineError;
use rustyline::ExternalPrinter;
use serde_json::Value;
use std::time::Duration;

const POLL: Duration = Duration::from_millis(500);

pub async fn run(ctx: &Ctx, a: ConsoleArgs) -> Result<()> {
    let sv = resolve::server(ctx, a.server.as_deref()).await?;
    // 한 번도 켠 적 없는 서버는 데몬이 404를 준다 → 빈 콘솔로 본다
    let first = match ctx.api.console(&sv.id, None, Some(a.lines)).await {
        Err(e)
            if matches!(
                e.downcast_ref::<crate::error::CliError>(),
                Some(crate::error::CliError::Api { status: 404, .. })
            ) =>
        {
            serde_json::json!({ "lines": [], "running": false })
        }
        r => r?,
    };
    if ctx.out.json {
        ctx.out.data(&first, |_| {});
        return Ok(());
    }
    let lines = first
        .get("lines")
        .and_then(Value::as_array)
        .cloned()
        .unwrap_or_default();
    for l in &lines {
        println!("{}", format_line(l));
    }
    let cursor = Cursor::from(&first);

    let interactive = !a.follow && !a.print && ctx.ask.enabled;
    if a.print || (!a.follow && !interactive) {
        if lines.is_empty() {
            println!("{}", crate::output::dim(&t!("console.empty")));
        }
        return Ok(());
    }
    if first.get("running").and_then(Value::as_bool) != Some(true) {
        ctx.out.note(&t!("console.not_running", name = sv.name));
    }
    if interactive {
        attach(ctx, &sv, cursor).await
    } else {
        ctx.out.note(&t!("console.following", name = sv.name));
        follow_printing(ctx, &sv, cursor, |line| println!("{}", line)).await
    }
}

/// 다음에 어디서부터 읽을지
#[derive(Clone, Copy, Debug, Default)]
struct Cursor {
    next: Option<u64>,
    running: bool,
    disk: bool,
}

impl Cursor {
    fn from(resp: &Value) -> Self {
        let last = resp
            .get("lines")
            .and_then(Value::as_array)
            .and_then(|l| l.iter().filter_map(|x| x.get("id")?.as_u64()).max());
        Self {
            next: last.map(|id| id + 1),
            running: resp.get("running").and_then(Value::as_bool).unwrap_or(false),
            disk: s(resp, "source") == "disk",
        }
    }
}

/// 새 줄을 받아 `emit`으로 넘긴다. Ctrl+C로 끝.
async fn follow_printing(ctx: &Ctx, sv: &Server, mut cur: Cursor, emit: impl Fn(String)) -> Result<()> {
    loop {
        tokio::select! {
            _ = tokio::time::sleep(POLL) => {}
            _ = tokio::signal::ctrl_c() => return Ok(()),
        }
        cur = poll_once(ctx, sv, cur, &emit).await;
    }
}

async fn poll_once(ctx: &Ctx, sv: &Server, cur: Cursor, emit: &impl Fn(String)) -> Cursor {
    // 디스크 로그는 since를 무시하므로, 꺼져 있는 동안은 실행 여부만 본다
    let since = if cur.disk { None } else { cur.next };
    let Ok(resp) = ctx
        .api
        .console(&sv.id, since, if since.is_none() { Some(1) } else { None })
        .await
    else {
        return cur;
    };
    let fresh = Cursor::from(&resp);
    let restarted = fresh.running && (!cur.running || cur.disk);
    if restarted {
        // 새 프로세스: 순번이 새로 시작한다 → 최근 줄부터 다시
        if let Ok(recent) = ctx.api.console(&sv.id, Some(0), None).await {
            emit(style(t!("console.started")).cyan().dim().to_string());
            for l in recent.get("lines").and_then(Value::as_array).into_iter().flatten() {
                emit(format_line(l));
            }
            return Cursor::from(&recent);
        }
        return cur;
    }
    if cur.running && !fresh.running {
        emit(style(t!("console.stopped")).cyan().dim().to_string());
    }
    if since.is_some() {
        for l in resp.get("lines").and_then(Value::as_array).into_iter().flatten() {
            emit(format_line(l));
        }
    }
    Cursor {
        next: fresh.next.or(cur.next),
        running: fresh.running,
        disk: fresh.disk,
    }
}

pub fn format_line(l: &Value) -> String {
    let text = s(l, "content");
    let base = |t: &str| match (s(l, "source"), s(l, "level")) {
        ("system", _) => style(t).cyan().to_string(),
        (_, "error") => style(t).red().to_string(),
        (_, "warn") => style(t).yellow().to_string(),
        (_, "debug") => style(t).dim().to_string(),
        _ => t.to_string(),
    };
    let prefix = if s(l, "source") == "system" {
        base("» ")
    } else {
        String::new()
    };
    let spans = span_ranges(text, l.get("spans"));
    if spans.is_empty() {
        return format!("{}{}", prefix, base(text));
    }
    // 토큰 구간은 토큰 색, 나머지는 줄 레벨 색
    let mut out = prefix;
    let mut at = 0;
    for (start, end, token) in spans {
        out.push_str(&base(&text[at..start]));
        out.push_str(&token_style(token, &text[start..end]));
        at = end;
    }
    out.push_str(&base(&text[at..]));
    out
}

/// 데몬 `spans`(UTF-16 오프셋) → 바이트 구간. 어긋난 구간은 버린다
fn span_ranges<'a>(text: &str, spans: Option<&'a Value>) -> Vec<(usize, usize, &'a str)> {
    let Some(spans) = spans.and_then(Value::as_array) else {
        return Vec::new();
    };
    // UTF-16 오프셋 → 바이트 오프셋 표 (마지막 원소는 끝)
    let mut bytes = Vec::with_capacity(text.len() + 1);
    for (b, ch) in text.char_indices() {
        for _ in 0..ch.len_utf16() {
            bytes.push(b);
        }
    }
    bytes.push(text.len());
    let byte_at = |u: u64| bytes.get(usize::try_from(u).ok()?).copied();
    let mut out: Vec<(usize, usize, &str)> = Vec::new();
    for sp in spans {
        let (Some(st), Some(en), Some(tok)) = (
            sp.get(0).and_then(Value::as_u64).and_then(byte_at),
            sp.get(1).and_then(Value::as_u64).and_then(byte_at),
            sp.get(2).and_then(Value::as_str),
        ) else {
            continue;
        };
        if st < en && out.last().is_none_or(|last| last.1 <= st) {
            out.push((st, en, tok));
        }
    }
    out
}

/// 토큰 → 터미널 스타일 (GUI 테마와 같은 이름)
fn token_style(token: &str, t: &str) -> String {
    let st = style(t);
    match token {
        "error" => st.red().bold(),
        "warn" => st.yellow().bold(),
        "info" => st.blue(),
        "debug" | "dim" | "timestamp" => st.dim(),
        "success" => st.green().bold(),
        "string" => st.green(),
        "number" => st.color256(215),
        "keyword" | "command" => st.magenta(),
        "player" => st.cyan().bold(),
        "selector" => st.yellow(),
        "ip" | "uuid" => st.cyan(),
        "path" => st.color256(116),
        _ => st,
    }
    .to_string()
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn spans_convert_utf16_offsets_to_byte_ranges() {
        let text = "안녕 😀 <철수>";
        let spans = json!([[7, 9, "player"], [100, 101, "error"], [8, 9, "warn"]]);
        let got = span_ranges(text, Some(&spans));
        assert_eq!(got.len(), 1, "out-of-range and overlapping spans are dropped");
        let (st, en, tok) = got[0];
        assert_eq!((&text[st..en], tok), ("철수", "player"));
    }

    #[test]
    fn format_line_keeps_text_and_colors_spans() {
        console::set_colors_enabled(true);
        let line = json!({"content": "Steve joined", "source": "stdout", "level": "info", "spans": [[0, 5, "player"]]});
        let out = format_line(&line);
        assert!(out.starts_with("\u{1b}["), "the player token is styled: {:?}", out);
        assert_eq!(console::strip_ansi_codes(&out), "Steve joined");
        let plain = json!({"content": "x", "source": "system", "level": "info"});
        assert_eq!(console::strip_ansi_codes(&format_line(&plain)), "» x");
    }
}

/// 보면서 입력하기. 새 줄은 입력 줄을 건드리지 않고 위에 찍힌다.
async fn attach(ctx: &Ctx, sv: &Server, cur: Cursor) -> Result<()> {
    let ctx = &Ctx {
        attached: true,
        ..ctx.clone()
    };
    let cat = crate::cmd::server::catalog_for_server(ctx, sv)
        .await
        .unwrap_or_default();
    let mut rl = crate::shell::editor(cat.names())?;
    let printer = rl.create_external_printer()?;

    // 프린터는 일반 스레드에 두고, 비동기 폴링은 채널로 줄만 넘긴다
    let (tx, rx) = std::sync::mpsc::channel::<String>();
    let printer_thread = std::thread::spawn(move || {
        let mut printer = printer;
        while let Ok(line) = rx.recv() {
            if printer.print(line).is_err() {
                break;
            }
        }
    });
    let poll_ctx = ctx.clone();
    let poll_sv = sv.clone();
    let poller = tokio::spawn(async move {
        let mut cur = cur;
        let emit = |line: String| {
            let _ = tx.send(line);
        };
        loop {
            tokio::time::sleep(POLL).await;
            cur = poll_once(&poll_ctx, &poll_sv, cur, &emit).await;
        }
    });

    eprintln!("{}", style(t!("console.attach_help", name = sv.name)).dim());
    let prompt = format!("{}> ", sv.name);
    loop {
        let line = tokio::task::block_in_place(|| rl.readline(&prompt));
        let line = match line {
            Ok(l) => l,
            Err(ReadlineError::Interrupted | ReadlineError::Eof) => break,
            Err(e) => {
                ctx.out.warn(&e.to_string());
                break;
            }
        };
        let text = line.trim();
        if text.is_empty() {
            continue;
        }
        let _ = rl.add_history_entry(text);
        if let Some(meta) = text.strip_prefix(':') {
            match meta.trim() {
                "q" | "quit" | "exit" => break,
                "help" | "h" | "?" => eprintln!("{}", t!("console.meta_help")),
                "commands" | "c" => {
                    let _ = crate::cmd::server::print_commands(ctx, &sv.module, &cat);
                }
                "stop" => report(
                    ctx,
                    crate::cmd::server::stop(
                        ctx,
                        crate::cli::StopArgs {
                            server: Some(sv.id.clone()),
                            no_wait: true,
                            ..Default::default()
                        },
                    )
                    .await,
                ),
                "start" => report(
                    ctx,
                    crate::cmd::server::start(
                        ctx,
                        crate::cli::StartArgs {
                            server: Some(sv.id.clone()),
                            no_wait: true,
                            ..Default::default()
                        },
                    )
                    .await,
                ),
                other => ctx.out.warn(&t!("console.unknown_meta", cmd = other)),
            }
            continue;
        }
        let words = shlex::split(text).unwrap_or_else(|| text.split_whitespace().map(str::to_string).collect());
        let (name, rest) = words
            .split_first()
            .map(|(n, r)| (n.clone(), r.to_vec()))
            .unwrap_or_default();
        report(ctx, crate::cmd::server::exec(ctx, sv, &cat, &name, &rest).await);
    }
    poller.abort();
    drop(rl);
    let _ = printer_thread.join();
    Ok(())
}

fn report(ctx: &Ctx, r: Result<()>) {
    if let Err(e) = r {
        ctx.out.error(&crate::error::classify(&e));
    }
}

/// `daemon logs`
pub async fn daemon_logs(ctx: &Ctx, lines: usize, follow: bool) -> Result<()> {
    let first = ctx.api.daemon_console(None, Some(lines)).await?;
    if ctx.out.json {
        ctx.out.data(&first, |_| {});
        return Ok(());
    }
    let print = |l: &Value| {
        let lvl = s(l, "level");
        let lvl_s = match lvl {
            "error" => style(format!("{:5}", lvl.to_uppercase())).red().to_string(),
            "warn" => style(format!("{:5}", lvl.to_uppercase())).yellow().to_string(),
            _ => style(format!("{:5}", lvl.to_uppercase())).dim().to_string(),
        };
        println!("{} {}", lvl_s, s(l, "message"));
    };
    let mut next = None;
    for l in first.get("lines").and_then(Value::as_array).into_iter().flatten() {
        print(l);
        next = l.get("id").and_then(Value::as_u64).map(|id| id + 1);
    }
    if !follow {
        return Ok(());
    }
    loop {
        tokio::select! {
            _ = tokio::time::sleep(POLL) => {}
            _ = tokio::signal::ctrl_c() => return Ok(()),
        }
        let Ok(r) = ctx.api.daemon_console(Some(next.unwrap_or(0)), Some(500)).await else {
            continue;
        };
        for l in r.get("lines").and_then(Value::as_array).into_iter().flatten() {
            print(l);
            next = l.get("id").and_then(Value::as_u64).map(|id| id + 1);
        }
    }
}
