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
        // 전각 공백처럼 여러 바이트인 공백 뒤에서 자른다
        let start = line
            .char_indices()
            .rfind(|(_, c)| c.is_whitespace())
            .map(|(i, c)| i + c.len_utf8())
            .unwrap_or(0);
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
            // `run 서버 li<Tab>` → list (게임 명령도 채운다)
            out.extend(crate::catalog::Catalog::from_module(m, None).names());
        }
    }
    out.sort();
    out.dedup();
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

    tell!(
        "{}",
        console::style(t!("shell.welcome", version = env!("CARGO_PKG_VERSION"))).bold()
    );
    if let Err(e) = crate::cmd::status::run(&ctx).await {
        ctx.out.error(&classify(&e));
    }
    tell!("{}", crate::output::dim(&t!("shell.help_line")));

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
                tell!("{}", crate::output::dim(&t!("shell.ctrl_c")));
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
        // `# 메모`처럼 주석뿐인 줄은 단어가 없다
        let Some(first) = words.first().map(|w| w.trim_start_matches("saba-chan-cli").to_string()) else {
            continue;
        };
        let words: Vec<String> = if first.is_empty() { words[1..].to_vec() } else { words };
        if words.is_empty() {
            continue;
        }
        ctx.follow_daemon_port();
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
    tell!("{}", crate::output::dim(&t!("shell.bye")));
    Ok(())
}

/// 셸 한 줄을 명령 하나로 실행
async fn one(ctx: &Ctx, words: &[String]) -> Result<()> {
    let argv = std::iter::once("saba-chan-cli".to_string()).chain(words.iter().cloned());
    let matches = match crate::cli::parse(argv.map(Into::into).collect()) {
        Ok(m) => m,
        Err(e) => {
            // 도움말·버전도 여기로 온다. 셸은 끝내지 않고 보여만 준다.
            let _ = e.print();
            return Ok(());
        }
    };
    let cli = Cli::from_arg_matches(&matches)?;
    let mut ctx = ctx.clone();
    // 셸을 연 뒤에 준 `--port`는 이 줄에만 쓴다. `--lang`은 문구를 이미 불러와 바꿀 수 없다
    if let Some(port) = cli.global.port {
        ctx = ctx.on_port(port);
    }
    // `--daemon`도 이 줄에만 (`local`이면 이 PC)
    if let Some(daemon) = crate::cli::line_daemon(&matches) {
        ctx.api.target = Some(daemon).filter(|d| !d.trim().is_empty());
    }
    if cli.global.lang.as_deref().is_some_and(|l| crate::i18n::normalize(l) != crate::i18n::lang()) {
        ctx.out.warn(&t!("shell.lang_fixed", lang = crate::i18n::lang()));
    }
    if cli.global.json || cli.global.json_stream {
        ctx.out = crate::output::Out { json: true };
        ctx.ask.enabled = false;
    }
    if cli.global.yes {
        ctx.ask.yes = true;
    }
    match cli.cmd {
        None => Ok(()),
        Some(crate::cli::Cmd::Shell) => Ok(()),
        Some(cmd) => {
            // `--no-color`(와 `--json`의 표준 출력)는 이 줄에만, 끝나면 되돌린다
            let colors = (console::colors_enabled(), console::colors_enabled_stderr());
            if cli.global.no_color || cli.global.json || cli.global.json_stream {
                console::set_colors_enabled(false);
            }
            if cli.global.no_color {
                console::set_colors_enabled_stderr(false);
            }
            // `--json-stream`도 이 줄에만
            crate::output::set_stream(cli.global.json_stream);
            let stream = cli.global.json_stream.then(crate::progress::stream_lines);
            let r = crate::cmd::dispatch(&ctx, cmd).await;
            console::set_colors_enabled(colors.0);
            console::set_colors_enabled_stderr(colors.1);
            // `--json` 줄은 오류도 JSON으로
            let r = match r {
                Err(e) if ctx.out.json => {
                    ctx.out.error(&classify(&e));
                    Ok(())
                }
                r => r,
            };
            crate::output::flush();
            drop(stream);
            crate::output::set_stream(false);
            r
        }
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

    #[test]
    fn completes_after_a_full_width_space() {
        let (start, list) = helper().candidates("start\u{3000}pal");
        assert_eq!(start, "start\u{3000}".len());
        assert_eq!(list, vec!["palworld-1"]);
    }
}
