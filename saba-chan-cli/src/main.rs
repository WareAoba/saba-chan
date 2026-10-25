//! saba-chan-cli — 사바쨩 데몬을 다루는 명령줄 도구
//!
//! 모든 동작은 데몬 API로 한다. 이 프로그램이 직접 만지는 로컬 파일은
//! `bootstrap`(포트·언어·토큰 위치, 데몬 실행 파일 찾기)뿐이다.

#[macro_use]
mod i18n;
#[macro_use]
mod output;

mod bootstrap;
mod catalog;
mod cli;
mod client;
mod cmd;
mod ctx;
mod error;
mod progress;
mod prompt;
mod resolve;
mod shell;
mod tui;
mod slots;

use clap::FromArgMatches;
use std::io::IsTerminal;

/// `--lang`은 도움말 번역 전에 알아야 하므로 먼저 훑는다
fn lang_flag() -> Option<String> {
    let args: Vec<String> = std::env::args().collect();
    args.iter().enumerate().find_map(|(i, a)| {
        a.strip_prefix("--lang=")
            .map(str::to_string)
            .or_else(|| (a == "--lang").then(|| args.get(i + 1).cloned()).flatten())
    })
}

/// 명령 처리는 큰 async 상태 기계라 디버그 빌드에서 스택을 많이 쓴다. 셸·메뉴 안에서 명령을 한 겹 더
/// 부르면 Windows 기본 주 스레드 스택(1MB)을 넘으므로, 넉넉한 스택의 스레드에서 돌린다
const STACK: usize = 16 * 1024 * 1024;

fn main() {
    let runtime = tokio::runtime::Builder::new_multi_thread()
        .enable_all()
        .thread_stack_size(STACK)
        .build()
        .expect("tokio runtime");
    let main = std::thread::Builder::new()
        .name("main".into())
        .stack_size(STACK)
        .spawn(move || runtime.block_on(run()))
        .expect("main thread");
    if main.join().is_err() {
        std::process::exit(101);
    }
}

async fn run() {
    // `| head`처럼 파이프가 먼저 닫혀도 패닉 대신 조용히 끝나게
    #[cfg(unix)]
    unsafe {
        libc::signal(libc::SIGPIPE, libc::SIG_DFL);
    }
    let flag = lang_flag();
    i18n::init(&bootstrap::language(flag.as_deref()));

    let matches = match cli::parse(std::env::args_os().collect()) {
        Ok(m) => m,
        Err(e) => e.exit(),
    };
    let parsed = match cli::Cli::from_arg_matches(&matches) {
        Ok(c) => c,
        Err(e) => e.exit(),
    };
    let g = parsed.global;
    // 언어를 직접 골랐을 때만 데몬에도 알린다: 아니면 데몬은 앱 설정 언어를 따른다.
    // 앞의 훑기는 `chat … --lang=ko` 같은 명령 본문도 잡으므로 실제로 인식된 전역 옵션을 쓴다
    let daemon_lang = bootstrap::requested_language(g.lang.as_deref()).map(|l| i18n::normalize(&l));
    let json = g.json || g.json_stream;
    // `--json-stream`: 진행은 한 줄씩, 결과는 마지막 한 줄
    output::set_stream(g.json_stream);
    let _stream = g.json_stream.then(progress::stream_lines);
    // 새로 띄울 데몬의 포트와, 지금 붙을 데몬의 포트(설정을 바꾼 뒤 옛 포트에 남은 데몬일 수 있다)
    let spawn_port = g.port.unwrap_or_else(bootstrap::ipc_port);
    let port = g.port.unwrap_or_else(|| bootstrap::live_port(spawn_port));
    let color = !g.no_color && std::env::var_os("NO_COLOR").is_none() && std::io::stdout().is_terminal();
    let ctx = ctx::Ctx {
        api: client::Api::new(port, daemon_lang),
        spawn_port,
        port_fixed: g.port.is_some(),
        out: output::Out::new(json, color),
        ask: prompt::Prompt::new(json, g.yes),
        in_shell: false,
        attached: false,
    };

    let cmd = match parsed.cmd {
        Some(c) => c,
        // 인자 없이 터미널에서 열면 셸, 아니면 상태 한 번
        // 터미널이면 전체 화면 메뉴. `SABA_NO_TUI=1`이면 예전 셸
        None if ctx.ask.enabled && std::io::stdout().is_terminal() => {
            if std::env::var_os("SABA_NO_TUI").is_some_and(|v| !v.is_empty() && v != "0") {
                cli::Cmd::Shell
            } else {
                cli::Cmd::Tui { server: None }
            }
        }
        None => cli::Cmd::Status,
    };
    let result = cmd::dispatch(&ctx, cmd).await;
    if let Err(e) = &result {
        ctx.out.error(&error::classify(e));
    }
    output::flush();
    if let Err(e) = result {
        std::process::exit(error::classify(&e).exit_code());
    }
}
