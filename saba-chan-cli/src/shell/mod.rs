//! Inline REPL shell — Claude-Code-style interactive UX.
//!
//! Unlike `tui::run`, this mode does NOT switch the terminal into the
//! alternate-screen buffer. Output is appended to the existing scrollback so
//! users can `Ctrl+Shift+Up` / mouse-scroll back through history exactly like
//! a normal shell.
//!
//! Features
//! --------
//! * `rustyline` line editor: arrow-history, Ctrl+R reverse search, multi-byte
//!   safe, file-backed history.
//! * Slash commands handled in-process: `/help`, `/use <name>`, `/clear`,
//!   `/json on|off`, `/yes on|off`, `/exit`.
//! * Bare verbs (`start`, `stop`, `status`, `console`, `exec ...`, `rcon ...`)
//!   are auto-rewritten to `server <verb> <current_instance> ...` when an
//!   instance is selected via `/use`.
//! * Everything else is parsed by the existing `oneshot::Cli` clap definition
//!   and handed to `oneshot::runner::dispatch`, so the shell stays in lock-step
//!   with one-shot behaviour.

use std::path::PathBuf;

use anyhow::Context;
use clap::Parser;
use rustyline::error::ReadlineError;
use rustyline::history::FileHistory;
use rustyline::{Config, Editor};

use crate::client::DaemonClient;
use crate::oneshot;

/// Run the inline REPL until the user exits via `/exit`, EOF, or fatal error.
pub async fn run(
    client: DaemonClient,
    initial_json: bool,
    initial_yes: bool,
    initial_instance: Option<String>,
) -> anyhow::Result<()> {
    let handle = tokio::runtime::Handle::current();
    let history_path = history_file_path();
    tokio::task::spawn_blocking(move || {
        repl_blocking(client, initial_json, initial_yes, initial_instance, history_path, handle)
    })
    .await
    .context("shell task panicked")?
}

fn repl_blocking(
    client: DaemonClient,
    json: bool,
    yes: bool,
    initial_instance: Option<String>,
    history_path: PathBuf,
    handle: tokio::runtime::Handle,
) -> anyhow::Result<()> {
    let mut state = ShellState::new(json, yes, initial_instance);
    let config = Config::builder()
        .auto_add_history(false)
        .history_ignore_space(true)
        .completion_type(rustyline::CompletionType::List)
        .build();
    let mut rl: Editor<(), FileHistory> =
        Editor::with_config(config).context("rustyline init failed")?;
    if let Some(parent) = history_path.parent() {
        let _ = std::fs::create_dir_all(parent);
    }
    let _ = rl.load_history(&history_path);

    print_banner(&state);

    loop {
        let prompt = state.prompt();
        match rl.readline(&prompt) {
            Ok(line) => {
                let trimmed = line.trim();
                if trimmed.is_empty() {
                    continue;
                }
                let _ = rl.add_history_entry(trimmed);
                if let Some(action) = SlashAction::parse(trimmed) {
                    if matches!(action, SlashAction::Exit) {
                        break;
                    }
                    apply_slash(&mut state, action);
                    continue;
                }
                let argv = match build_argv(&state, trimmed) {
                    Ok(v) => v,
                    Err(e) => {
                        eprintln!("error: {}", e);
                        continue;
                    }
                };
                let cli = match oneshot::Cli::try_parse_from(&argv) {
                    Ok(c) => c,
                    Err(e) => {
                        // clap-style help/error output (already coloured if TTY).
                        eprintln!("{}", e);
                        continue;
                    }
                };
                if matches!(cli.command, Some(oneshot::Command::Tui))
                    || matches!(cli.command, Some(oneshot::Command::Shell { .. }))
                {
                    eprintln!("(cannot launch tui/shell from inside the shell)");
                    continue;
                }
                if cli.command.is_none() {
                    // Bare invocation inside shell is a no-op.
                    continue;
                }
                let code = handle.block_on(oneshot::runner::dispatch(cli, client.clone()));
                if code != oneshot::EXIT_OK && !state.quiet {
                    eprintln!("(exit code {})", code);
                }
            }
            Err(ReadlineError::Interrupted) => {
                // Ctrl+C on empty line — just clear the line and continue.
                continue;
            }
            Err(ReadlineError::Eof) => {
                // Ctrl+D — exit gracefully.
                break;
            }
            Err(e) => {
                eprintln!("readline error: {}", e);
                break;
            }
        }
    }

    let _ = rl.save_history(&history_path);
    Ok(())
}

// ─── State ─────────────────────────────────────────────────────────────

#[derive(Debug, Clone)]
pub struct ShellState {
    pub json: bool,
    pub yes: bool,
    pub quiet: bool,
    pub current_instance: Option<String>,
}

impl ShellState {
    pub fn new(json: bool, yes: bool, current_instance: Option<String>) -> Self {
        Self { json, yes, quiet: false, current_instance }
    }

    pub fn prompt(&self) -> String {
        match &self.current_instance {
            Some(name) => format!("saba [{}]> ", name),
            None => "saba> ".to_string(),
        }
    }
}

// ─── Slash commands ────────────────────────────────────────────────────

#[derive(Debug, PartialEq, Eq)]
pub enum SlashAction {
    Exit,
    Help,
    Use(Option<String>),
    Clear,
    Json(bool),
    Yes(bool),
    Status,
}

impl SlashAction {
    pub fn parse(line: &str) -> Option<Self> {
        let line = line.trim();
        if !line.starts_with('/') {
            return None;
        }
        let mut parts = line[1..].split_whitespace();
        let head = parts.next()?.to_lowercase();
        match head.as_str() {
            "exit" | "quit" | "q" => Some(SlashAction::Exit),
            "help" | "h" | "?" => Some(SlashAction::Help),
            "clear" | "cls" => Some(SlashAction::Clear),
            "use" => Some(SlashAction::Use(parts.next().map(|s| s.to_string()))),
            "json" => match parts.next().map(|s| s.to_lowercase()).as_deref() {
                Some("on") | Some("true") | Some("1") => Some(SlashAction::Json(true)),
                Some("off") | Some("false") | Some("0") => Some(SlashAction::Json(false)),
                _ => None,
            },
            "yes" => match parts.next().map(|s| s.to_lowercase()).as_deref() {
                Some("on") | Some("true") | Some("1") => Some(SlashAction::Yes(true)),
                Some("off") | Some("false") | Some("0") => Some(SlashAction::Yes(false)),
                _ => None,
            },
            "status" => Some(SlashAction::Status),
            _ => None,
        }
    }
}

fn apply_slash(state: &mut ShellState, action: SlashAction) {
    match action {
        SlashAction::Exit => unreachable!("handled by caller"),
        SlashAction::Help => print_help(),
        SlashAction::Clear => {
            // ANSI clear-screen + move cursor to home.
            print!("\x1b[2J\x1b[H");
            use std::io::Write;
            let _ = std::io::stdout().flush();
        }
        SlashAction::Use(name) => {
            state.current_instance = name.clone();
            match name {
                Some(n) => println!("(now targeting '{}')", n),
                None => println!("(cleared instance selection)"),
            }
        }
        SlashAction::Json(b) => {
            state.json = b;
            println!("(json output: {})", if b { "on" } else { "off" });
        }
        SlashAction::Yes(b) => {
            state.yes = b;
            println!(
                "(auto-confirm destructive ops: {})",
                if b { "on" } else { "off" }
            );
        }
        SlashAction::Status => {
            println!(
                "instance={}  json={}  yes={}",
                state.current_instance.as_deref().unwrap_or("<none>"),
                state.json,
                state.yes
            );
        }
    }
}

// ─── Argv construction ─────────────────────────────────────────────────

/// Verbs that are valid when an instance is pre-selected via `/use`.
const SERVER_SHORTCUTS: &[&str] = &[
    "start",
    "stop",
    "restart",
    "status",
    "info",
    "console",
    "exec",
    "rcon",
    "rest",
    "diagnose",
    "properties",
    "validate",
];

/// Tokenize a user line and rewrite shortcuts into full clap-compatible argv.
pub fn build_argv(state: &ShellState, line: &str) -> anyhow::Result<Vec<String>> {
    let raw = shell_words::split(line).context("invalid quoting")?;
    if raw.is_empty() {
        anyhow::bail!("empty command");
    }
    let head = raw[0].to_lowercase();
    let rest = &raw[1..];

    let mut out: Vec<String> = vec!["saba-chan-cli".to_string()];
    if state.json {
        out.push("--json".to_string());
    }
    if state.yes {
        out.push("--yes".to_string());
    }

    if SERVER_SHORTCUTS.contains(&head.as_str()) && state.current_instance.is_some() {
        let inst = state.current_instance.clone().expect("checked above");
        out.push("server".to_string());
        out.push(head);
        out.push(inst);
        out.extend(rest.iter().cloned());
    } else {
        out.push(head);
        out.extend(rest.iter().cloned());
    }
    Ok(out)
}

// ─── Banner / help ─────────────────────────────────────────────────────

fn print_banner(state: &ShellState) {
    println!("saba-chan-cli shell — type /help for commands, /exit to quit.");
    if let Some(name) = &state.current_instance {
        println!("targeting instance: {}", name);
    }
}

fn print_help() {
    println!(
        "Slash commands:\n  \
         /help                 show this help\n  \
         /use <name>           target a server (enables bare start/stop/console/...)\n  \
         /use                  clear instance selection\n  \
         /status               show shell state (current instance, flags)\n  \
         /json on|off          toggle JSON output\n  \
         /yes on|off           toggle auto-confirm for destructive ops\n  \
         /clear                clear the screen\n  \
         /exit, /quit          leave the shell (Ctrl+D also works)\n\n\
         Anything else is parsed as a saba-chan-cli command. With an instance\n\
         selected, the shortcuts {{start, stop, restart, status, info, console,\n\
         exec, rcon, rest, diagnose, properties, validate}} are forwarded to\n\
         `server <verb> <instance> ...` automatically.\n\n\
         Live console:  console -f      (Ctrl+C to stop streaming)"
    );
}

// ─── History file ──────────────────────────────────────────────────────

fn history_file_path() -> PathBuf {
    saba_chan_updater_lib::constants::resolve_data_dir().join("shell-history.txt")
}

#[cfg(test)]
mod tests {
    use super::*;

    fn st_with_instance(inst: Option<&str>) -> ShellState {
        ShellState::new(false, false, inst.map(|s| s.to_string()))
    }

    #[test]
    fn slash_parses_exit_aliases() {
        assert_eq!(SlashAction::parse("/exit"), Some(SlashAction::Exit));
        assert_eq!(SlashAction::parse("/quit"), Some(SlashAction::Exit));
        assert_eq!(SlashAction::parse("/q"), Some(SlashAction::Exit));
    }

    #[test]
    fn slash_use_captures_name() {
        assert_eq!(
            SlashAction::parse("/use mc-1"),
            Some(SlashAction::Use(Some("mc-1".into())))
        );
        assert_eq!(SlashAction::parse("/use"), Some(SlashAction::Use(None)));
    }

    #[test]
    fn slash_json_toggle() {
        assert_eq!(SlashAction::parse("/json on"), Some(SlashAction::Json(true)));
        assert_eq!(
            SlashAction::parse("/json off"),
            Some(SlashAction::Json(false))
        );
        assert_eq!(SlashAction::parse("/json maybe"), None);
    }

    #[test]
    fn non_slash_input_is_none() {
        assert_eq!(SlashAction::parse("server list"), None);
        assert_eq!(SlashAction::parse(""), None);
    }

    #[test]
    fn build_argv_passes_through_full_command() {
        let st = st_with_instance(None);
        let argv = build_argv(&st, "module list").unwrap();
        assert_eq!(argv, vec!["saba-chan-cli", "module", "list"]);
    }

    #[test]
    fn build_argv_propagates_global_flags() {
        let mut st = st_with_instance(None);
        st.json = true;
        st.yes = true;
        let argv = build_argv(&st, "status").unwrap();
        assert_eq!(
            argv,
            vec!["saba-chan-cli", "--json", "--yes", "status"]
        );
    }

    #[test]
    fn build_argv_rewrites_shortcut_with_selected_instance() {
        let st = st_with_instance(Some("mc-1"));
        let argv = build_argv(&st, "start").unwrap();
        assert_eq!(argv, vec!["saba-chan-cli", "server", "start", "mc-1"]);
    }

    #[test]
    fn build_argv_shortcut_appends_user_args() {
        let st = st_with_instance(Some("mc-1"));
        let argv = build_argv(&st, "exec say hello world").unwrap();
        assert_eq!(
            argv,
            vec!["saba-chan-cli", "server", "exec", "mc-1", "say", "hello", "world"]
        );
    }

    #[test]
    fn build_argv_shortcut_without_instance_falls_through_to_clap() {
        // With no instance selected, shortcut verbs are passed through verbatim
        // so clap can decide (e.g. `status` is a top-level command, but
        // `start` is not — which yields a clap usage error at dispatch time).
        let st = st_with_instance(None);
        let argv = build_argv(&st, "start").unwrap();
        assert_eq!(argv, vec!["saba-chan-cli", "start"]);
    }

    #[test]
    fn build_argv_handles_quoted_strings() {
        let st = st_with_instance(None);
        let argv = build_argv(&st, "server start \"my server\"").unwrap();
        assert_eq!(
            argv,
            vec!["saba-chan-cli", "server", "start", "my server"]
        );
    }
}
