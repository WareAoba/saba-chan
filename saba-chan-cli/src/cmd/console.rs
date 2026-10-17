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

/// 명령을 보내기 전 콘솔 위치. `echo_reply`와 짝.
pub async fn mark(ctx: &Ctx, sv: &Server) -> Option<u64> {
    if ctx.out.json || ctx.attached {
        return None;
    }
    let resp = ctx.api.console(&sv.id, None, Some(1)).await.ok()?;
    let cur = Cursor::from(&resp);
    (cur.running && !cur.disk).then_some(cur.next.unwrap_or(0))
}

/// 명령 뒤에 서버가 콘솔에 찍은 답을 잠깐 모아 보여 준다 (stdin 명령은 응답이 콘솔로만 온다)
pub async fn echo_reply(ctx: &Ctx, sv: &Server, from: Option<u64>) {
    let Some(mut next) = from else { return };
    let mut quiet = 0;
    // 줄이 오다가 잠깐 멎으면 끝. 아무것도 안 오면 1.5초까지 기다린다.
    for _ in 0..10 {
        tokio::time::sleep(Duration::from_millis(150)).await;
        let Ok(resp) = ctx.api.console(&sv.id, Some(next), None).await else {
            return;
        };
        let lines: Vec<Value> = resp.get("lines").and_then(Value::as_array).cloned().unwrap_or_default();
        if lines.is_empty() {
            quiet += 1;
            if quiet >= 2 && next > from.unwrap_or(0) {
                return;
            }
            continue;
        }
        quiet = 0;
        for l in &lines {
            println!("  {}", format_line(l));
        }
        if let Some(c) = Cursor::from(&resp).next {
            next = c;
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
    match (s(l, "source"), s(l, "level")) {
        ("system", _) => style(format!("» {}", text)).cyan().to_string(),
        (_, "error") => style(text).red().to_string(),
        (_, "warn") => style(text).yellow().to_string(),
        (_, "debug") => style(text).dim().to_string(),
        _ => text.to_string(),
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
