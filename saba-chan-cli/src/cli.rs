//! 명령 구조 (clap)
//!
//! 영어 설명은 여기 doc 주석, 번역은 `cli.json`의 `help.cmd.<경로>`·`help.arg.<인자>`.
//! `localized()`가 실행 시 바꿔 끼운다. 새 명령은 enum에 갈래 하나 + `cmd/`에 처리 함수 하나.

use clap::{Args, CommandFactory, Parser, Subcommand};

#[derive(Parser, Debug)]
#[command(
    name = "saba-chan-cli",
    version,
    about = "Saba-chan: run and manage your game servers from the terminal",
    after_help = "Run without a command to open the interactive shell."
)]
pub struct Cli {
    #[command(flatten)]
    pub global: Global,
    #[command(subcommand)]
    pub cmd: Option<Cmd>,
}

#[derive(Args, Debug, Clone, Default)]
pub struct Global {
    /// Print machine-readable JSON instead of text
    #[arg(long, global = true, display_order = 900)]
    pub json: bool,
    /// Answer yes to confirmation questions
    #[arg(short = 'y', long, global = true, display_order = 900)]
    pub yes: bool,
    /// Disable colors
    #[arg(long, global = true, display_order = 900)]
    pub no_color: bool,
    /// Language for messages (en, ko, ...)
    #[arg(long, global = true, display_order = 900, value_name = "LANG")]
    pub lang: Option<String>,
    /// Daemon port (default: from settings)
    #[arg(long, global = true, display_order = 900, value_name = "PORT")]
    pub port: Option<u16>,
}

#[derive(Subcommand, Debug)]
pub enum Cmd {
    /// Show daemon, servers, Discord and updates at a glance
    Status,
    /// Manage game servers
    #[command(subcommand, visible_alias = "servers")]
    Server(ServerCmd),
    /// Start a server
    Start(StartArgs),
    /// Stop a server
    Stop(StopArgs),
    /// Restart a server
    Restart(RestartArgs),
    /// Watch a server's console and type into it
    Console(ConsoleArgs),
    /// Run a game command on a server (e.g. `run survival say hello`)
    Run(RunArgs),
    /// Start, stop and inspect the Saba-chan daemon
    #[command(subcommand)]
    Daemon(DaemonCmd),
    /// Game modules: what games Saba-chan can run
    #[command(subcommand, visible_alias = "modules", alias = "game")]
    Module(ModuleCmd),
    /// Extensions that add features
    #[command(subcommand, visible_alias = "ext", alias = "extensions")]
    Extension(ExtCmd),
    /// Discord bot connection (cloud relay)
    #[command(subcommand, visible_alias = "discord")]
    Bot(BotCmd),
    /// World backups of a server
    #[command(subcommand, visible_alias = "backups")]
    Backup(BackupCmd),
    /// Scheduled tasks of a server (backup, restart, announce, update)
    #[command(subcommand, visible_alias = "schedules")]
    Schedule(ScheduleCmd),
    /// AI chatbot for Discord (API key, settings, memory)
    #[command(subcommand, visible_alias = "ai")]
    Chatbot(ChatbotCmd),
    /// Saba-chan updates
    #[command(subcommand, visible_alias = "updates")]
    Update(UpdateCmd),
    /// App settings
    #[command(subcommand, visible_alias = "settings")]
    Config(ConfigCmd),
    /// Send a chat-style command, the same way Discord does (e.g. `chat palworld start`)
    Chat {
        #[arg(
            required = true,
            trailing_var_arg = true,
            allow_hyphen_values = true,
            value_name = "TEXT"
        )]
        text: Vec<String>,
    },
    /// Versions of every installed part
    Version,
    /// Open the interactive shell
    Shell,
    /// Print a shell completion script
    Completions {
        #[arg(value_name = "SHELL")]
        shell: clap_complete::Shell,
    },
}

// ── backup / schedule / chatbot ──

#[derive(Subcommand, Debug)]
pub enum BackupCmd {
    /// Backups of a server, newest first
    #[command(visible_alias = "ls")]
    List { server: Option<String> },
    /// Back up now
    #[command(visible_alias = "new")]
    Create { server: Option<String> },
    /// Delete one backup
    #[command(visible_alias = "rm")]
    Delete { server: String, backup: String },
    /// Put a backup back (the server must be stopped; asks to stop it)
    Restore { server: String, backup: String },
}

#[derive(Subcommand, Debug)]
pub enum ScheduleCmd {
    /// Show the schedule
    #[command(visible_alias = "ls")]
    Show { server: Option<String> },
    /// Add a task: backup, restart, announce or update
    Add {
        server: String,
        /// backup | restart | announce | update
        action: String,
        /// Repeat every N minutes (30, 30m, 2h)
        #[arg(long, value_name = "TIME")]
        every: Option<String>,
        /// Every day at this time (HH:MM, this PC's clock)
        #[arg(long, value_name = "HH:MM")]
        at: Option<String>,
        /// Warn players this many minutes before (restart/update), e.g. 10,5,1
        #[arg(long, value_name = "MINUTES", value_delimiter = ',')]
        warn: Vec<u32>,
        /// Message to announce
        #[arg(long)]
        message: Option<String>,
    },
    /// Remove task number N (see `schedule show`)
    #[command(visible_alias = "rm")]
    Remove { server: String, number: usize },
    /// Turn task N on
    On { server: String, number: usize },
    /// Turn task N off
    Off { server: String, number: usize },
    /// How many scheduled backups to keep
    Keep { server: String, count: u32 },
    /// Remove every task
    Clear { server: String },
}

#[derive(Subcommand, Debug)]
pub enum ChatbotCmd {
    /// Settings and whether an API key is saved
    Show,
    /// Change settings: KEY=VALUE ... (empty = default, [a,b] = list, @FILE = file text, e.g. memo=@notes.txt)
    Set {
        #[arg(value_name = "KEY=VALUE", required = true)]
        pairs: Vec<String>,
        /// Only for this Discord server (guild id)
        #[arg(long)]
        guild: Option<String>,
    },
    /// Save the API key (asked for without showing it; never pass it as an argument)
    Key {
        /// Delete the saved key instead
        #[arg(long)]
        delete: bool,
    },
    /// Delete all chatbot settings and the key
    Clear,
    /// Forget what the chatbot remembers (all Discord servers, or one)
    Forget {
        #[arg(long)]
        guild: Option<String>,
    },
}

// ── server ──

#[derive(Subcommand, Debug)]
pub enum ServerCmd {
    /// List servers and their state
    #[command(visible_alias = "ls")]
    List,
    /// Details of one server
    #[command(visible_alias = "show")]
    Info { server: Option<String> },
    /// Add a new server (asks what it needs)
    #[command(visible_alias = "new", alias = "create")]
    Add(AddArgs),
    /// Delete a server
    #[command(visible_alias = "rm", alias = "delete")]
    Remove { server: Option<String> },
    /// Start a server
    Start(StartArgs),
    /// Stop a server
    Stop(StopArgs),
    /// Restart a server
    Restart(RestartArgs),
    /// Watch the console and type into it
    #[command(visible_alias = "logs")]
    Console(ConsoleArgs),
    /// Run a game command (lists commands when none is given)
    #[command(visible_alias = "exec")]
    Run(RunArgs),
    /// Game commands this server understands
    Commands { server: Option<String> },
    /// Send one raw line to the server console
    Send {
        server: String,
        #[arg(
            required = true,
            trailing_var_arg = true,
            allow_hyphen_values = true,
            value_name = "LINE"
        )]
        line: Vec<String>,
    },
    /// Show a server's settings
    Settings { server: Option<String> },
    /// Change settings: KEY=VALUE ... (ext.KEY for extension data, empty value clears)
    Set {
        server: String,
        #[arg(required = true, value_name = "KEY=VALUE")]
        pairs: Vec<String>,
    },
    /// The game's own config file (server.properties, ini ...)
    #[command(visible_alias = "props")]
    Properties {
        server: Option<String>,
        #[command(subcommand)]
        action: Option<PropsCmd>,
    },
    /// Call the game's REST API directly
    Rest {
        server: String,
        /// GET, POST, ...
        method: String,
        /// e.g. /v1/api/info
        endpoint: String,
        /// JSON body
        #[arg(long, value_name = "JSON")]
        body: Option<String>,
    },
    /// Installed game server version
    Version { server: Option<String> },
    /// Update the game server files (SteamCMD servers)
    Update {
        server: Option<String>,
        /// Only check, don't install
        #[arg(long)]
        check: bool,
    },
    /// Install progress of a new server
    Progress {
        server: String,
        /// Clear a finished or failed progress entry (a failed new server is removed)
        #[arg(long)]
        dismiss: bool,
    },
    /// Why the server stopped (crash, user stop ...)
    #[command(visible_alias = "exits")]
    History { server: Option<String> },
    /// Check the server can start (files, Java, ports ...)
    Validate { server: Option<String> },
    /// Look for known problems in the logs
    Diagnose { server: Option<String> },
    /// Accept the game's EULA
    AcceptEula { server: Option<String> },
    /// Wipe the server files and start fresh
    Reset { server: Option<String> },
    /// Rename a server
    Rename { server: String, name: String },
    /// Change the list order: give server names in the order you want
    Reorder {
        #[arg(required = true)]
        servers: Vec<String>,
    },
}

#[derive(Subcommand, Debug)]
pub enum PropsCmd {
    /// Show the file's values
    Show,
    /// Change values: KEY=VALUE ...
    Set {
        #[arg(required = true, value_name = "KEY=VALUE")]
        pairs: Vec<String>,
    },
    /// Restore the module's defaults
    Reset,
}

#[derive(Args, Debug, Clone, Default)]
pub struct AddArgs {
    /// Name for the new server
    pub name: Option<String>,
    /// Game module (name or alias)
    #[arg(short = 'm', long = "module", visible_alias = "game")]
    pub module: Option<String>,
    /// Use an existing server folder instead of installing
    #[arg(long, value_name = "DIR")]
    pub from: Option<String>,
    /// Game port (`--port` is the daemon's)
    #[arg(long = "game-port", value_name = "PORT")]
    pub game_port: Option<u16>,
    /// Extra settings KEY=VALUE (repeatable)
    #[arg(long = "set", value_name = "KEY=VALUE")]
    pub set: Vec<String>,
    /// Start it as soon as it is ready
    #[arg(long)]
    pub start: bool,
    /// Don't wait for the install to finish
    #[arg(long)]
    pub no_wait: bool,
}

#[derive(Args, Debug, Clone, Default)]
pub struct StartArgs {
    pub server: Option<String>,
    /// Open the console after starting
    #[arg(short = 'c', long)]
    pub console: bool,
    /// Return right away instead of waiting until it runs
    #[arg(long)]
    pub no_wait: bool,
}

#[derive(Args, Debug, Clone, Default)]
pub struct StopArgs {
    pub server: Option<String>,
    /// Kill the process instead of a graceful stop
    #[arg(long)]
    pub force: bool,
    /// Return right away instead of waiting until it stops
    #[arg(long)]
    pub no_wait: bool,
}

#[derive(Args, Debug, Clone, Default)]
pub struct RestartArgs {
    pub server: Option<String>,
    /// Kill the process instead of a graceful stop
    #[arg(long)]
    pub force: bool,
}

#[derive(Args, Debug, Clone, Default)]
pub struct ConsoleArgs {
    pub server: Option<String>,
    /// How many recent lines to show first
    #[arg(short = 'n', long, default_value_t = 50)]
    pub lines: usize,
    /// Keep printing new lines (read-only)
    #[arg(short = 'f', long, conflicts_with = "print")]
    pub follow: bool,
    /// Print recent lines and exit
    #[arg(long)]
    pub print: bool,
}

#[derive(Args, Debug, Clone, Default)]
pub struct RunArgs {
    pub server: Option<String>,
    /// Command name or alias
    pub command: Option<String>,
    /// Arguments: in order, or NAME=VALUE
    #[arg(trailing_var_arg = true, allow_hyphen_values = true)]
    pub args: Vec<String>,
}

// ── daemon ──

#[derive(Subcommand, Debug)]
pub enum DaemonCmd {
    /// Is the daemon running?
    Status,
    /// Start the daemon in the background
    Start,
    /// Stop the daemon (game servers keep their own state)
    Stop {
        /// Kill it if it doesn't stop in time
        #[arg(long)]
        force: bool,
    },
    /// Stop and start the daemon
    Restart {
        #[arg(long)]
        force: bool,
    },
    /// The daemon's own log
    Logs {
        #[arg(short = 'n', long, default_value_t = 100)]
        lines: usize,
        #[arg(short = 'f', long)]
        follow: bool,
    },
}

// ── module ──

#[derive(Subcommand, Debug)]
pub enum ModuleCmd {
    /// Installed game modules
    #[command(visible_alias = "ls")]
    List,
    /// Details of a module
    #[command(visible_alias = "show")]
    Info { module: Option<String> },
    /// Game commands a module defines
    Commands { module: Option<String> },
    /// Game server versions you can install
    Versions {
        module: Option<String>,
        /// Show details of one version
        version: Option<String>,
        #[arg(long)]
        snapshots: bool,
        #[arg(long, default_value_t = 1)]
        page: u32,
        #[arg(long, default_value_t = 25)]
        per_page: u32,
    },
    /// Modules available to download
    Available,
    /// Download and install a module
    Install { id: String },
    /// Delete an installed module
    #[command(visible_alias = "rm")]
    Remove { id: String },
    /// Reload modules from disk
    Refresh,
    /// Which game is in this folder?
    Detect { dir: String },
    /// JSON Schema for module.toml (for module authors)
    Schema,
    /// Download server files only, into a folder you choose
    InstallFiles {
        module: String,
        #[arg(long)]
        version: String,
        #[arg(long, value_name = "DIR")]
        dir: String,
        #[arg(long)]
        jar: Option<String>,
        #[arg(long)]
        accept_eula: bool,
    },
}

// ── extension ──

#[derive(Subcommand, Debug)]
pub enum ExtCmd {
    /// Installed extensions
    #[command(visible_alias = "ls")]
    List,
    /// Details of an extension
    #[command(visible_alias = "show")]
    Info { extension: Option<String> },
    /// Turn an extension on
    Enable { extension: Option<String> },
    /// Turn an extension off
    Disable { extension: Option<String> },
    /// Extensions available to download
    Available,
    /// Download and install (or update) extensions
    Install {
        /// One or more extension ids
        #[arg(required = true, num_args = 1..)]
        ids: Vec<String>,
        /// Install from this URL instead of the catalog (one id only)
        #[arg(long)]
        url: Option<String>,
        #[arg(long)]
        sha256: Option<String>,
    },
    /// Update installed extensions (all when none is named)
    Update { extension: Option<String> },
    /// Delete an extension
    #[command(visible_alias = "rm")]
    Remove { extension: Option<String> },
    /// Show or change an extension's settings: KEY=VALUE ...
    Config {
        extension: Option<String>,
        #[arg(value_name = "KEY=VALUE")]
        pairs: Vec<String>,
    },
    /// Look for new extension folders
    Rescan,
    /// Startup progress of extensions
    Status,
    /// Load one extension folder from disk
    Mount { id: String },
    /// Unload an extension without deleting it
    Unmount { id: String },
}

// ── bot ──

#[derive(Subcommand, Debug)]
pub enum BotCmd {
    /// Is this PC connected to the Discord bot?
    Status,
    /// Connect to the Discord bot relay
    Start,
    /// Disconnect from the relay
    Stop,
    /// Pair this PC with your Discord server (shows a code to enter in Discord)
    Pair {
        #[arg(long, value_name = "URL")]
        relay_url: Option<String>,
    },
    /// Forget this PC's pairing
    Unpair,
    /// Set the node token by hand (from a pairing done elsewhere)
    Token { token: Option<String> },
    /// Show or change the chat prefix
    Prefix { value: Option<String> },
    /// Short names for games in chat: `alias`, `alias add NAME MODULE`, `alias remove NAME`
    Alias {
        #[command(subcommand)]
        action: Option<AliasCmd>,
    },
    /// Discord servers paired with this PC
    Nodes,
    /// Members of a paired Discord server
    Members { guild: String },
    /// Link to invite the Saba-chan bot to a Discord server
    Invite,
    /// Chat names for one server (e.g. "크리" for a second Minecraft server). No names: show
    #[command(name = "server-alias")]
    ServerAlias {
        server: Option<String>,
        /// New names (replace the old ones)
        names: Vec<String>,
        /// Remove this server's names
        #[arg(long)]
        clear: bool,
    },
    /// Bot settings back to their defaults (all when no key is named)
    Reset { keys: Vec<String> },
    /// Show or change any bot setting: KEY=VALUE (dotted keys, JSON values)
    Config {
        #[arg(value_name = "KEY=VALUE")]
        pairs: Vec<String>,
    },
}

#[derive(Subcommand, Debug)]
pub enum AliasCmd {
    List,
    Add {
        alias: String,
        module: String,
    },
    #[command(visible_alias = "rm")]
    Remove {
        alias: String,
    },
}

// ── update ──

#[derive(Subcommand, Debug)]
pub enum UpdateCmd {
    /// What is installed and what is new
    Status,
    /// Check for updates now
    Check,
    /// Download updates (all when none is named)
    Download { components: Vec<String> },
    /// Install downloaded updates
    Apply { components: Vec<String> },
    /// Result of the last update
    Last {
        #[arg(long)]
        clear: bool,
    },
    /// Verify installed files against the release
    Integrity,
    /// Show or change update settings: KEY=VALUE ...
    Config {
        #[arg(value_name = "KEY=VALUE")]
        pairs: Vec<String>,
    },
}

// ── config ──

#[derive(Subcommand, Debug)]
pub enum ConfigCmd {
    /// All settings
    #[command(visible_alias = "list")]
    Show,
    /// One setting
    Get { key: String },
    /// Change settings: KEY=VALUE ... (`cli.KEY` for CLI-only settings)
    Set {
        #[arg(required = true, value_name = "KEY=VALUE")]
        pairs: Vec<String>,
    },
    /// Default value of every setting
    Defaults,
    /// Back to the default values (all settings when no key is named)
    Reset { keys: Vec<String> },
}

/// 번역된 도움말을 끼운 clap 명령
pub fn localized() -> clap::Command {
    fn walk(cmd: clap::Command, path: &str) -> clap::Command {
        let mut cmd = cmd;
        if let Some(about) = crate::i18n::try_t(&format!("help.cmd.{}", if path.is_empty() { "_root" } else { path })) {
            cmd = cmd.about(about);
        }
        // mut_arg는 인자를 맨 뒤로 옮긴다. 위치 인자는 순서가 곧 뜻이라 번호를 먼저 박아 둔다.
        let positionals: Vec<String> = cmd.get_positionals().map(|a| a.get_id().to_string()).collect();
        for (i, id) in positionals.iter().enumerate() {
            cmd = cmd.mut_arg(id.as_str(), |a| a.index(i + 1));
        }
        let ids: Vec<String> = cmd.get_arguments().map(|a| a.get_id().to_string()).collect();
        for id in ids {
            let text = crate::i18n::try_t(&format!("help.arg.{}.{}", path, id))
                .or_else(|| crate::i18n::try_t(&format!("help.arg.{}", id)));
            if let Some(text) = text {
                cmd = cmd.mut_arg(id.as_str(), |a| a.help(text));
            }
        }
        let names: Vec<String> = cmd.get_subcommands().map(|s| s.get_name().to_string()).collect();
        for name in names {
            let sub_path = if path.is_empty() {
                name.clone()
            } else {
                format!("{}.{}", path, name)
            };
            cmd = cmd.mut_subcommand(name.as_str(), |s| walk(s, &sub_path));
        }
        cmd
    }
    let mut cmd = walk(Cli::command(), "");
    if let Some(after) = crate::i18n::try_t("help.after") {
        cmd = cmd.after_help(after);
    }
    cmd
}

/// 셸 자동완성·검사용 명령 이름 목록 (경로 포함)
pub fn command_words() -> Vec<String> {
    fn walk(cmd: &clap::Command, prefix: &str, out: &mut Vec<String>) {
        for s in cmd.get_subcommands() {
            let path = if prefix.is_empty() {
                s.get_name().to_string()
            } else {
                format!("{} {}", prefix, s.get_name())
            };
            out.push(path.clone());
            for a in s.get_all_aliases() {
                out.push(if prefix.is_empty() {
                    a.to_string()
                } else {
                    format!("{} {}", prefix, a)
                });
            }
            walk(s, &path, out);
        }
    }
    let mut out = Vec::new();
    walk(&Cli::command(), "", &mut out);
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn clap_definition_is_valid() {
        Cli::command().debug_assert();
        // 도움말을 바꿔 끼워도 위치 인자 순서가 그대로여야 한다
        localized().debug_assert();
    }

    #[test]
    fn shortcuts_and_aliases_parse() {
        let p = |args: &[&str]| Cli::try_parse_from(std::iter::once("saba-chan-cli").chain(args.iter().copied()));
        assert!(matches!(
            p(&["start", "pal"]).unwrap().cmd,
            Some(Cmd::Start(StartArgs { server: Some(_), .. }))
        ));
        assert!(matches!(
            p(&["servers", "ls"]).unwrap().cmd,
            Some(Cmd::Server(ServerCmd::List))
        ));
        assert!(matches!(
            p(&["ext", "ls"]).unwrap().cmd,
            Some(Cmd::Extension(ExtCmd::List))
        ));
        assert!(matches!(
            p(&["game", "list"]).unwrap().cmd,
            Some(Cmd::Module(ModuleCmd::List))
        ));
        let run = p(&["run", "mc", "give", "Steve", "--weird", "x=1"]).unwrap();
        match run.cmd {
            Some(Cmd::Run(r)) => {
                assert_eq!(r.command.as_deref(), Some("give"));
                assert_eq!(r.args, vec!["Steve", "--weird", "x=1"]);
            }
            _ => panic!("run did not parse"),
        }
        assert!(p(&["--json", "status"]).unwrap().global.json);
        assert!(p(&["status", "--json"]).unwrap().global.json);
    }

    #[test]
    fn every_command_has_a_korean_help_line() {
        crate::i18n::init("ko");
        fn walk(cmd: &clap::Command, path: &str, missing: &mut Vec<String>) {
            for s in cmd.get_subcommands() {
                if s.get_name() == "help" {
                    continue;
                }
                let p = if path.is_empty() {
                    s.get_name().to_string()
                } else {
                    format!("{}.{}", path, s.get_name())
                };
                if crate::i18n::try_t(&format!("help.cmd.{}", p)).is_none() {
                    missing.push(p.clone());
                }
                walk(s, &p, missing);
            }
        }
        let mut missing = Vec::new();
        walk(&Cli::command(), "", &mut missing);
        assert!(missing.is_empty(), "help.cmd keys missing: {:?}", missing);
    }
}
