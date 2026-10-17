//! saba-chan-cli 라이브러리 — 바이너리(`main.rs`)와 통합 테스트(`tests/`)가 공유한다.
//!
//! 통합 테스트가 쓰는 `client`, `oneshot`만 공개한다. 나머지 모듈은 크레이트 내부
//! 전용이라 쓰이지 않는 코드는 컴파일러가 dead code로 알려 준다.

mod cli_config;
pub mod client;
mod config;
mod i18n;
mod ipc;
mod module_registry;
pub mod oneshot;
mod payload;
mod process;
mod shell;
mod tui;
mod update_flow;

use clap::Parser;

/// CLI 진입점. 서브커맨드가 없으면 TUI를 띄운다.
///
/// clap의 기본 parse-error exit code는 2(=EXIT_DAEMON_OFFLINE과 충돌)이므로
/// 문서화된 계약(0=ok, 1=err, 2=offline, 3=bad usage, 4=not found)에 맞게 매핑한다.
/// 도움말/버전 출력은 0으로 종료.
pub async fn run() -> anyhow::Result<()> {
    let cli = match oneshot::Cli::try_parse() {
        Ok(c) => c,
        Err(e) => {
            // clap의 print 함수가 stdout/stderr 분기까지 처리해 준다.
            let _ = e.print();
            let exit_code = match e.kind() {
                clap::error::ErrorKind::DisplayHelp
                | clap::error::ErrorKind::DisplayVersion
                | clap::error::ErrorKind::DisplayHelpOnMissingArgumentOrSubcommand => {
                    oneshot::EXIT_OK
                }
                _ => oneshot::EXIT_BAD_USAGE,
            };
            std::process::exit(exit_code);
        }
    };

    // --daemon URL이 있으면 우선, 없으면 settings.json의 IPC 포트.
    let base_url = cli.daemon.clone().unwrap_or_else(ipc::base_url);
    let client = client::DaemonClient::new(Some(&base_url))?;

    match cli.command {
        Some(oneshot::Command::Tui) | None => tui::run(client).await,
        Some(oneshot::Command::Shell { use_instance }) => {
            shell::run(client, cli.json, cli.yes, use_instance).await
        }
        Some(_) => {
            let code = oneshot::runner::dispatch(cli, client).await;
            std::process::exit(code);
        }
    }
}
