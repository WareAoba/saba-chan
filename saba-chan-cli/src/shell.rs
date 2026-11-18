//! 대화형 셸 — 인자 없이 터미널에서 실행하면 열린다
//!
//! 한 줄 = 명령 한 번 (`start pal`, `server ls`). 앞에 `saba-chan-cli`를 붙이지 않는다.
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
    // 셸 안에서는 Ctrl+C가 셸을 죽이지 않게 한다 (실행 중인 명령은 `interruptible`이 멈춘다)
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
        let words = match split(text) {
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
        if let Err(e) = interruptible(one(&ctx, &words), tokio::signal::ctrl_c()).await {
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

/// 명령 줄을 낱말로 나눈다 (셸·TUI 명령 줄). 따옴표가 맞지 않으면 None.
/// 작은따옴표 안은 그대로, 큰따옴표는 묶기만 한다. 역슬래시는 Windows 명령 줄처럼 따옴표 바로 앞에서만 뜻이 있어
/// (`\"` → 따옴표 글자, `\\"` → 역슬래시 하나와 닫는 따옴표) `C:\Servers\Pal` 같은 경로는 그대로 간다.
/// 낱말 첫머리의 `#`부터는 주석이다
pub fn split(line: &str) -> Option<Vec<String>> {
    let mut words = Vec::new();
    let mut word: Option<String> = None;
    let mut double = false;
    let mut chars = line.chars().peekable();
    while let Some(c) = chars.next() {
        match c {
            '\\' => {
                let mut n = 1;
                while chars.next_if_eq(&'\\').is_some() {
                    n += 1;
                }
                let w = word.get_or_insert_with(String::new);
                let before_quote = matches!(chars.peek(), Some('"')) || (!double && chars.peek() == Some(&'\''));
                if before_quote {
                    w.extend(std::iter::repeat_n('\\', n / 2));
                    if n % 2 == 1 {
                        w.extend(chars.next());
                    }
                } else {
                    w.extend(std::iter::repeat_n('\\', n));
                }
            }
            '"' => {
                double = !double;
                word.get_or_insert_with(String::new);
            }
            _ if double => word.get_or_insert_with(String::new).push(c),
            '\'' => {
                let w = word.get_or_insert_with(String::new);
                loop {
                    match chars.next()? {
                        '\'' => break,
                        c => w.push(c),
                    }
                }
            }
            '#' if word.is_none() => break,
            c if c.is_whitespace() => words.extend(word.take()),
            c => word.get_or_insert_with(String::new).push(c),
        }
    }
    if double {
        return None;
    }
    words.extend(word);
    Some(words)
}

/// 명령 줄에 넣을 낱말 (`split`과 짝): 글자·숫자와 `-_./:=,@+`만이면 그대로, 아니면 작은따옴표로 (한글 이름도 그대로 보이게)
pub fn quote(s: &str) -> String {
    if !s.is_empty() && s.chars().all(|c| c.is_alphanumeric() || "-_./:=,@+".contains(c)) && !s.starts_with('-') {
        return s.to_string();
    }
    // 작은따옴표 글자는 따옴표를 닫고 큰따옴표로 넣는다
    format!("'{}'", s.replace('\'', "'\"'\"'"))
}

/// Ctrl+C를 받은 명령이 마무리할 틈 (따라가기 끝내기, 원격 설치 취소 요청)
const WRAP_UP: std::time::Duration = std::time::Duration::from_secs(1);

/// 명령 하나를 Ctrl+C(`interrupt`)와 겨룬다: 눌리면 그 명령만 취소로 끝낸다 (셸은 남는다).
/// 스스로 Ctrl+C를 받는 명령은 `WRAP_UP` 안에 끝내면 그 결과를 쓴다
async fn interruptible<T>(run: impl Future<Output = Result<()>>, interrupt: impl Future<Output = T>) -> Result<()> {
    tokio::pin!(run);
    tokio::select! {
        r = &mut run => r,
        _ = interrupt => tokio::time::timeout(WRAP_UP, run).await.unwrap_or_else(|_| Err(crate::error::CliError::Cancelled.into())),
    }
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

    /// Windows 경로의 역슬래시는 그대로 간다. 따옴표로 묶은 공백 있는 인자, 주석뿐인 줄은 전과 같다
    #[test]
    fn backslashes_in_windows_paths_are_kept() {
        let w = |line: &str| split(line).unwrap();
        assert_eq!(w(r"server add pal --from C:\Servers\Palworld"), ["server", "add", "pal", "--from", r"C:\Servers\Palworld"]);
        assert_eq!(w(r"backup import mc \\nas\backups\mc.zip"), ["backup", "import", "mc", r"\\nas\backups\mc.zip"]);
        assert_eq!(w(r#"server add pal --dir "D:\My Games\pal""#), ["server", "add", "pal", "--dir", r"D:\My Games\pal"]);
        // 닫는 따옴표 바로 앞의 역슬래시는 Windows 명령 줄처럼 두 개를 하나로 쓴다
        assert_eq!(w(r#"--dir "C:\Games\\""#), ["--dir", r"C:\Games\"]);
        assert_eq!(w(r#"server run mc 'say "hi there"' "it's" a\"b"#), ["server", "run", "mc", r#"say "hi there""#, "it's", r#"a"b"#]);
        assert_eq!(w("set motd= x='' "), ["set", "motd=", "x="]);
        assert_eq!(w("# 메모"), Vec::<String>::new());
        assert_eq!(w("start pal # 메모"), ["start", "pal"]);
        assert_eq!(split(r#"say "unclosed"#), None);
        assert_eq!(split("say 'unclosed"), None);
    }

    /// 셸에서 Ctrl+C를 누르면 응답을 기다리던 명령은 취소로 끝나고(셸은 남는다), 스스로 Ctrl+C를 받아 마무리하는 명령은 그 결과를 쓴다
    #[tokio::test]
    async fn ctrl_c_ends_the_running_command_but_lets_it_wrap_up() {
        let pressed = async {};
        let err = interruptible(std::future::pending::<Result<()>>(), pressed).await.unwrap_err();
        assert!(matches!(err.downcast::<crate::error::CliError>().unwrap(), crate::error::CliError::Cancelled));
        let wraps_up = async {
            tokio::time::sleep(std::time::Duration::from_millis(50)).await;
            Ok(())
        };
        assert!(interruptible(wraps_up, async {}).await.is_ok());
        assert!(interruptible(async { Ok(()) }, std::future::pending::<()>()).await.is_ok());
    }

    /// `quote`로 만든 낱말은 `split`이 그대로 되읽는다 (TUI 폼·마법사가 만드는 줄)
    #[test]
    fn quoted_words_split_back_unchanged() {
        for word in ["mc", "팰월드 1", r"C:\Servers\Pal\", "it's", r#"say "hi""#, "", "-3", r#"a b'c\"d"#, "$HOME", r"\\nas\x", "#tag"] {
            let line = format!("server set {} k={}", quote(word), quote(word));
            assert_eq!(split(&line).unwrap(), ["server", "set", word, &format!("k={word}")], "{line}");
        }
    }

    #[test]
    fn completes_after_a_full_width_space() {
        let (start, list) = helper().candidates("start\u{3000}pal");
        assert_eq!(start, "start\u{3000}".len());
        assert_eq!(list, vec!["palworld-1"]);
    }
}
