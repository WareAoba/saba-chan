//! 대화형 셸 — 인자 없이 터미널에서 실행하면 열린다
//!
//! 한 줄 = 명령 한 번 (`start pal`, `server ls`). 앞에 `saba-chan-cli`를 붙이지 않는다.
//! 첫 단어가 명령이 아니면 디스코드와 같은 채팅 처리기로 보낸다 (`팰월드 시작`).
//! Tab으로 명령·서버 이름을 채운다. 셸을 닫아도 데몬과 서버는 계속 돈다.

use crate::cli::Cli;
use crate::ctx::Ctx;
use crate::error::{classify, Result};
use clap::FromArgMatches;
use rustyline::completion::{Completer, Pair};
use rustyline::error::ReadlineError;
use rustyline::highlight::Highlighter;
use rustyline::hint::Hinter;
use rustyline::history::DefaultHistory;
use rustyline::validate::Validator;
use rustyline::{Context, Editor, Helper};

pub struct ShellHelper {
    /// "server start" 같은 명령 경로
    paths: Vec<String>,
    /// 어디서나 채울 수 있는 단어 (서버·모듈 이름, 게임 명령)
    words: Vec<String>,
}

impl ShellHelper {
    fn candidates(&self, line: &str) -> (usize, Vec<String>) {
        let start = line.rfind(char::is_whitespace).map(|i| i + 1).unwrap_or(0);
        let (before, cur) = line.split_at(start);
        let before = before.split_whitespace().collect::<Vec<_>>().join(" ");
        let mut out: Vec<String> = self
            .paths
            .iter()
            .filter_map(|p| {
                let rest = if before.is_empty() {
                    Some(p.as_str())
                } else {
                    p.strip_prefix(&before)?.strip_prefix(' ')
                };
                rest.filter(|r| !r.contains(' ') && r.starts_with(cur))
                    .map(str::to_string)
            })
            .collect();
        if !before.is_empty() || out.is_empty() {
            out.extend(
                self.words
                    .iter()
                    .filter(|w| w.starts_with(cur) && !w.contains(char::is_whitespace))
                    .cloned(),
            );
        }
        out.sort();
        out.dedup();
        (start, out)
    }
}

impl Completer for ShellHelper {
    type Candidate = Pair;
    fn complete(&self, line: &str, pos: usize, _: &Context<'_>) -> rustyline::Result<(usize, Vec<Pair>)> {
        let (start, list) = self.candidates(&line[..pos]);
        Ok((
            start,
            list.into_iter()
                .map(|w| Pair {
                    display: w.clone(),
                    replacement: w,
                })
                .collect(),
        ))
    }
}

impl Hinter for ShellHelper {
    type Hint = String;
}
impl Highlighter for ShellHelper {}
impl Validator for ShellHelper {}
impl Helper for ShellHelper {}

pub type LineEditor = Editor<ShellHelper, DefaultHistory>;

/// 줄 편집기. `extra_words`는 Tab으로 채울 단어 (서버 이름, 게임 명령 등).
pub fn editor(extra_words: Vec<String>) -> Result<LineEditor> {
    let config = rustyline::Config::builder()
        .completion_type(rustyline::CompletionType::List)
        .auto_add_history(false)
        .max_history_size(500)?
        .build();
    let mut rl = Editor::with_config(config)?;
    rl.set_helper(Some(ShellHelper {
        paths: crate::cli::command_words(),
        words: extra_words,
    }));
    Ok(rl)
}

async fn names(ctx: &Ctx) -> Vec<String> {
    let mut out = Vec::new();
    if let Ok(servers) = crate::resolve::servers(ctx).await {
        out.extend(servers.into_iter().map(|s| s.name));
    }
    if let Ok(mods) = crate::resolve::modules(ctx).await {
        for m in &mods {
            if let Some(n) = m.get("name").and_then(|v| v.as_str()) {
                out.push(n.to_string());
            }
        }
    }
    out
}

pub async fn run(ctx: &Ctx) -> Result<()> {
    let mut ctx = ctx.clone();
    ctx.in_shell = true;
    // 셸 안에서는 Ctrl+C가 셸을 죽이지 않게 한다 (명령마다 따로 받는다)
    tokio::spawn(async {
        loop {
            if tokio::signal::ctrl_c().await.is_err() {
                break;
            }
        }
    });

    eprintln!(
        "{}",
        console::style(t!("shell.welcome", version = env!("CARGO_PKG_VERSION"))).bold()
    );
    if let Err(e) = crate::cmd::status::run(&ctx).await {
        ctx.out.error(&classify(&e));
    }
    eprintln!("{}", crate::output::dim(&t!("shell.help_line")));

    let mut rl = editor(names(&ctx).await)?;
    let top: Vec<String> = crate::cli::command_words()
        .into_iter()
        .filter(|w| !w.contains(' '))
        .collect();
    let prompt = format!("{} ", console::style("saba>").cyan().bold());
    let mut last_ctrl_c = false;
    loop {
        let line = tokio::task::block_in_place(|| rl.readline(&prompt));
        let line = match line {
            Ok(l) => {
                last_ctrl_c = false;
                l
            }
            Err(ReadlineError::Interrupted) => {
                if last_ctrl_c {
                    break;
                }
                last_ctrl_c = true;
                eprintln!("{}", crate::output::dim(&t!("shell.ctrl_c")));
                continue;
            }
            Err(ReadlineError::Eof) => break,
            Err(e) => return Err(e.into()),
        };
        let text = line.trim();
        if text.is_empty() {
            continue;
        }
        let _ = rl.add_history_entry(text);
        if matches!(text, "exit" | "quit" | "q" | "종료" | "나가기") {
            break;
        }
        let words = match shlex::split(text) {
            Some(w) => w,
            None => {
                ctx.out.warn(&t!("shell.bad_quotes"));
                continue;
            }
        };
        let first = words[0].trim_start_matches("saba-chan-cli").to_string();
        let words: Vec<String> = if first.is_empty() { words[1..].to_vec() } else { words };
        if words.is_empty() {
            continue;
        }
        let is_cmd = top.contains(&words[0]) || words[0] == "help" || words[0].starts_with('-');
        let result = if is_cmd {
            one(&ctx, &words).await
        } else {
            crate::cmd::chat(&ctx, text).await
        };
        if let Err(e) = result {
            ctx.out.error(&classify(&e));
        }
        // 서버를 더하거나 지웠을 수 있으니 이름 목록을 새로
        if matches!(words[0].as_str(), "server" | "servers" | "module" | "modules" | "game") {
            if let Some(h) = rl.helper_mut() {
                h.words = names(&ctx).await;
            }
        }
    }
    eprintln!("{}", crate::output::dim(&t!("shell.bye")));
    Ok(())
}

/// 셸 한 줄을 명령 하나로 실행
async fn one(ctx: &Ctx, words: &[String]) -> Result<()> {
    let argv = std::iter::once("saba-chan-cli".to_string()).chain(words.iter().cloned());
    let matches = match crate::cli::localized().try_get_matches_from(argv) {
        Ok(m) => m,
        Err(e) => {
            // 도움말·버전도 여기로 온다. 셸은 끝내지 않고 보여만 준다.
            let _ = e.print();
            return Ok(());
        }
    };
    let cli = Cli::from_arg_matches(&matches)?;
    let mut ctx = ctx.clone();
    if cli.global.json {
        ctx.out = crate::output::Out { json: true };
        ctx.ask.enabled = false;
    }
    if cli.global.yes {
        ctx.ask.yes = true;
    }
    match cli.cmd {
        None => Ok(()),
        Some(crate::cli::Cmd::Shell) => Ok(()),
        Some(cmd) => crate::cmd::dispatch(&ctx, cmd).await,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn helper() -> ShellHelper {
        ShellHelper {
            paths: vec![
                "server".into(),
                "server start".into(),
                "server stop".into(),
                "status".into(),
                "start".into(),
            ],
            words: vec!["palworld-1".into()],
        }
    }

    #[test]
    fn completes_top_level_and_subcommands() {
        let h = helper();
        assert_eq!(h.candidates("st").1, vec!["start", "status"]);
        assert_eq!(h.candidates("server st").1, vec!["start", "stop"]);
        let (start, list) = h.candidates("start pal");
        assert_eq!(start, 6);
        assert_eq!(list, vec!["palworld-1"]);
    }
}
