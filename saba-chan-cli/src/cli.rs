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
    disable_help_subcommand = true,
    after_help = "Run without a command to open the full-screen menu (SABA_NO_TUI=1: the interactive shell)."
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
    /// Print progress and then the result as JSON lines (NDJSON); implies --json
    #[arg(long, global = true, display_order = 900)]
    pub json_stream: bool,
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
    /// Use a saved remote daemon (name or host id; `remote daemons`) through this PC's daemon
    #[arg(long, global = true, display_order = 900, value_name = "NAME", env = "SABA_DAEMON")]
    pub daemon: Option<String>,
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
    /// Talking Saba-chan, the AI for Discord (API key, settings, memory)
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
    /// CPU, memory, disk and network use, and each server's share
    Monitor(MonitorArgs),
    /// Who started, stopped or sent commands to servers, and from where (GUI, CLI, Discord, schedule)
    #[command(visible_alias = "activity")]
    Audit {
        server: Option<String>,
        /// How many entries
        #[arg(long, default_value_t = 30)]
        limit: usize,
        /// Keep printing new entries as they happen (Ctrl+C to stop)
        #[arg(short, long, conflicts_with = "export")]
        follow: bool,
        /// From this day: YYYY-MM-DD, today, or 7d (the last 7 days)
        #[arg(long, value_name = "DATE")]
        since: Option<String>,
        /// Up to the end of this day: YYYY-MM-DD or today
        #[arg(long, value_name = "DATE")]
        until: Option<String>,
        /// Save every entry of the period to this CSV file instead of listing them
        #[arg(long, value_name = "FILE")]
        export: Option<std::path::PathBuf>,
    },
    /// Follow daemon events as they happen (with --json: one JSON per line). Ctrl+C to stop
    Events {
        /// Only these events; `instance` also takes `instance.exit` and the like
        #[arg(long, value_name = "EVENT")]
        kind: Vec<String>,
        /// Only events of this server
        #[arg(long, short)]
        server: Option<String>,
        /// Start after this event id (replays what the daemon still keeps)
        #[arg(long, value_name = "ID")]
        since: Option<u64>,
    },
    /// Who may run which commands from Discord (levels per member, preset, overrides)
    #[command(subcommand, visible_alias = "permissions")]
    Perm(PermCmd),
    /// Isolated runs of unverified modules and extensions
    #[command(subcommand)]
    Sandbox(SandboxCmd),
    /// Remote daemon port: your other PCs connect to this daemon over TLS
    #[command(subcommand)]
    Remote(RemoteCmd),
    /// Run any daemon action by id (`saba do` alone lists them)
    Do {
        /// Action id, e.g. server.restart or server.command:say
        action: Option<String>,
        /// Server the action works on
        #[arg(long, short)]
        server: Option<String>,
        /// Only check the input and show what would happen
        #[arg(long)]
        plan: bool,
        /// Input as key=value (values that read as JSON stay JSON)
        #[arg(value_name = "KEY=VALUE")]
        input: Vec<String>,
        /// Several steps as JSON: [{"id": "...", "instance": "...", "input": {...}}, ...] (instead of ACTION)
        #[arg(long, value_name = "JSON", conflicts_with = "action")]
        steps: Option<String>,
        /// Progress of a multi-step run that continues in the background
        #[arg(long = "run", value_name = "RUN_ID", conflicts_with_all = ["action", "steps"])]
        run_id: Option<String>,
    },
    /// Versions of every installed part
    Version,
    /// Open the interactive shell
    Shell,
    /// Open the full-screen menu (the default when run without a command in a terminal)
    Tui {
        /// Server to open first
        #[arg(long)]
        server: Option<String>,
    },
    /// Switch to the desktop app: it opens on the same server and the full-screen menu closes
    Gui { server: Option<String> },
    /// Show help for a command. With --json: the command tree, arguments and exit codes
    Help {
        /// Command path, e.g. `server start`
        #[arg(value_name = "COMMAND")]
        command: Vec<String>,
    },
    /// Print a shell completion script
    Completions {
        #[arg(value_name = "SHELL")]
        shell: clap_complete::Shell,
    },
    /// Install Saba-chan on this PC without the window installer
    Install(InstallArgs),
    /// Remove Saba-chan from this PC. Settings and servers are kept unless removed
    Uninstall(UninstallArgs),
}

#[derive(Args, Debug)]
pub struct UninstallArgs {
    /// Remove settings and the Discord connection too
    #[arg(long)]
    pub remove_settings: bool,
    /// Remove game servers too (server files, worlds, backups)
    #[arg(long)]
    pub remove_instances: bool,
}

#[derive(Args, Debug)]
pub struct InstallArgs {
    /// Install folder (asked when omitted)
    #[arg(long, value_name = "PATH")]
    pub dir: Option<std::path::PathBuf>,
    /// Game modules to install, comma separated (asked when omitted)
    #[arg(long, value_name = "NAMES", num_args = 1..)]
    pub modules: Vec<String>,
    /// Install the desktop app too, even without a display
    #[arg(long, conflicts_with = "no_gui")]
    pub gui: bool,
    /// Install without the desktop app, even with a display
    #[arg(long)]
    pub no_gui: bool,
    /// Folder for modules and extensions (Saba-chan makes `modules` and `extensions` inside it). Installed files are moved
    #[arg(long, value_name = "PATH")]
    pub components_dir: Option<std::path::PathBuf>,
}

#[derive(Args, Debug)]
pub struct MonitorArgs {
    /// Keep refreshing until Ctrl+C
    #[arg(short, long)]
    pub watch: bool,
    /// Seconds between refreshes with --watch
    #[arg(long, default_value_t = 2, value_name = "SECONDS")]
    pub interval: u64,
}

// ── backup / schedule / chatbot ──

#[derive(Subcommand, Debug)]
pub enum BackupCmd {
    /// Backups of a server, newest first
    #[command(visible_alias = "ls")]
    List { server: Option<String> },
    /// Back up now (`--from-snapshot`: keep the game's latest automatic backup instead)
    #[command(visible_alias = "new")]
    Create {
        server: Option<String>,
        /// Keep the game's own latest automatic backup instead of copying the world now (modules with game snapshots)
        #[arg(long)]
        from_snapshot: bool,
    },
    /// Delete one backup (no name: pick from the list)
    #[command(visible_alias = "rm")]
    Delete { server: String, backup: Option<String> },
    /// Put a backup back (the server must be stopped; asks to stop it). No name: pick from the list
    Restore { server: String, backup: Option<String> },
}

#[derive(Subcommand, Debug)]
pub enum PermCmd {
    /// Levels, preset and overrides of one Discord server (name or id; without it: the only paired server)
    Show { guild: Option<String> },
    /// Commands per level: from which level each command can be used (one server with `--server`)
    Commands {
        guild: Option<String>,
        #[arg(long, value_name = "SERVER")]
        server: Option<String>,
    },
    /// Give a member (name or id) a level: admin, user, chatbot (Talking Saba-chan) or banned
    Grant { guild: String, member: String, level: String },
    /// Take a member's own level away (they get the default level)
    Revoke { guild: String, member: String },
    /// Level for members without their own level
    Default { guild: String, level: String },
    /// Command level preset: loose, default or strict
    Preset { guild: String, preset: String },
    /// Highest level a request through Talking Saba-chan can use
    Cap { guild: String, level: String },
    /// Level one command needs (`<server>/<command>`, server by name or id); leave the level out to go back to the preset
    Command { guild: String, key: String, level: Option<String> },
    /// Add, rename, remove or move a permission level of one Discord server (`perm show` lists them)
    #[command(subcommand)]
    Level(LevelCmd),
    /// Change the pinned owner of this Discord server
    Owner { guild: String, member: String },
    /// Bring levels still kept on the relay (from before) into this PC. No members: all of them, after asking
    Import { guild: String, members: Vec<String> },
}

#[derive(Subcommand, Debug)]
pub enum LevelCmd {
    /// Add a level (right below the owner, or right above `--above`)
    Add {
        guild: String,
        name: String,
        #[arg(long, value_name = "LEVEL")]
        above: Option<String>,
    },
    /// Rename an added level
    Rename { guild: String, level: String, name: String },
    /// Remove a level: its members move one level down, commands that needed it one level up
    Remove { guild: String, level: String },
    /// Put a level right above another one (the owner stays on top, banned at the bottom)
    Move {
        guild: String,
        level: String,
        #[arg(long, value_name = "LEVEL")]
        above: String,
    },
}

impl LevelCmd {
    pub fn guild(&self) -> &str {
        match self {
            LevelCmd::Add { guild, .. } | LevelCmd::Rename { guild, .. } | LevelCmd::Remove { guild, .. } | LevelCmd::Move { guild, .. } => guild,
        }
    }
}

#[derive(Subcommand, Debug)]
pub enum RemoteCmd {
    /// Remote port settings and state (listening, address sent to the relay, certificate fingerprint)
    Status,
    /// Turn the remote port on or off, change its port or the public address sent to the relay
    Set {
        #[arg(long, conflicts_with = "off")]
        on: bool,
        #[arg(long)]
        off: bool,
        #[arg(long)]
        port: Option<u16>,
        /// Public host name or IP of this PC (empty clears it)
        #[arg(long)]
        address: Option<String>,
    },
    /// Your PCs paired with this daemon
    #[command(visible_alias = "ls")]
    Clients,
    /// Unpair a PC: its token stops working at once
    Revoke { host_id: String },
    /// Your other PCs on the relay that this daemon can connect to
    Peers,
    /// Pair with the remote daemon on one of your PCs (`remote peers` shows the ids)
    Connect { host_id: String },
    /// Remote daemons this daemon connects to
    Daemons,
    /// Servers on this PC and on every remote daemon, by daemon (use one with `--daemon <name>`)
    Servers,
    /// Forget a remote daemon (pair again with `remote connect`)
    Forget { host_id: String },
}

#[derive(Subcommand, Debug)]
pub enum SandboxCmd {
    /// Consents given on this PC (run without isolation, extension GUI bundles)
    #[command(visible_alias = "ls")]
    List,
    /// Run this component without isolation when isolation cannot be prepared (asked again when its files change).
    /// `--gui`: load this extension's GUI bundle instead (component is the `ext-<id>` name shown by `sandbox list`)
    Allow {
        component: String,
        #[arg(long)]
        gui: bool,
    },
    /// Remove the consent (`--gui`: the GUI bundle consent)
    Revoke {
        component: String,
        #[arg(long)]
        gui: bool,
    },
}

/// 예약 작업의 때 (`schedule add`·`edit`)
#[derive(clap::Args, Debug, Default, Clone)]
pub struct ScheduleWhen {
    /// Repeat every N minutes (30, 30m, 2h)
    #[arg(long, value_name = "TIME")]
    pub every: Option<String>,
    /// At this time (HH:MM, the server PC's clock). Alone: every day
    #[arg(long, value_name = "HH:MM")]
    pub at: Option<String>,
    /// With --at: every N days
    #[arg(long, value_name = "DAYS", conflicts_with_all = ["on", "date", "daily"])]
    pub every_days: Option<u32>,
    /// With --every-days: count from this day (YYYY-MM-DD, default: today)
    #[arg(long, value_name = "YYYY-MM-DD")]
    pub from: Option<String>,
    /// With --at: on these weekdays, e.g. mon,wed,fri
    #[arg(long, value_name = "DAYS", value_delimiter = ',', conflicts_with_all = ["date", "daily"])]
    pub on: Vec<String>,
    /// With --at: once on this day (YYYY-MM-DD)
    #[arg(long, value_name = "YYYY-MM-DD", conflicts_with = "daily")]
    pub date: Option<String>,
    /// With --at: every day (drops --every-days, --on and --date in `edit`)
    #[arg(long)]
    pub daily: bool,
}

#[derive(Subcommand, Debug)]
pub enum ScheduleCmd {
    /// Show the schedule
    #[command(visible_alias = "ls")]
    Show { server: Option<String> },
    /// Add a task: start, stop, restart, save, backup, announce, update or command
    Add {
        server: String,
        /// start | stop | restart | save | backup | announce | update | command
        action: String,
        #[command(flatten)]
        when: ScheduleWhen,
        /// Warn players this many minutes before (restart/update/stop), e.g. 10,5,1
        #[arg(long, value_name = "MINUTES", value_delimiter = ',')]
        warn: Vec<u32>,
        /// Message to announce
        #[arg(long)]
        message: Option<String>,
        /// Game command to run (command; see `server commands`)
        #[arg(long, value_name = "NAME")]
        command: Option<String>,
        /// Game command input, NAME=VALUE (repeat for more)
        #[arg(long = "arg", value_name = "NAME=VALUE")]
        args: Vec<String>,
    },
    /// Change task number N in place: only the options given change (see `schedule show`)
    Edit {
        server: String,
        number: usize,
        #[command(flatten)]
        when: ScheduleWhen,
        /// New warnings in minutes before (restart/update/stop), e.g. 10,5,1
        #[arg(long, value_name = "MINUTES", value_delimiter = ',', conflicts_with = "no_warn")]
        warn: Vec<u32>,
        /// Remove the warnings
        #[arg(long)]
        no_warn: bool,
        /// New message to announce
        #[arg(long)]
        message: Option<String>,
        /// New game command (command; a different command drops the old inputs unless --arg is given)
        #[arg(long, value_name = "NAME")]
        command: Option<String>,
        /// New game command inputs, NAME=VALUE (replaces all inputs)
        #[arg(long = "arg", value_name = "NAME=VALUE")]
        args: Vec<String>,
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
    Show {
        /// What one Discord server actually uses (common values + its own), name or id
        #[arg(long)]
        guild: Option<String>,
    },
    /// Change settings: KEY=VALUE ... (shared: only model; everything else per server with --guild, its keys and values: `chatbot show GUILD`; e.g. --guild G effort=high; empty = default, [a,b] = list, @FILE = file text, e.g. memo=@notes.txt)
    Set {
        #[arg(value_name = "KEY=VALUE", required = true)]
        pairs: Vec<String>,
        /// Only for this Discord server (name or id)
        #[arg(long)]
        guild: Option<String>,
    },
    /// Save the API key (asked for without showing it; never pass it as an argument)
    Key {
        /// Delete the saved key instead
        #[arg(long)]
        delete: bool,
    },
    /// Delete all Talking Saba-chan settings and the key
    Clear,
    /// Models the saved API key can use (name or id of the Discord server; without it: the only paired server)
    Models { guild: Option<String> },
    /// Remaining credit (only providers that report it for a regular key) and this month's usage of one Discord server
    Credit { guild: Option<String> },
    /// Tools by category and whether each is on in one Discord server (change: `chatbot set tools=[lol,pubg] --guild ...`)
    Tools { guild: Option<String> },
    /// Print the server prompt template (save it and use `chatbot set memo=@FILE --guild ...`)
    Template {
        /// Language of the template (default: the app language)
        #[arg(long)]
        locale: Option<String>,
    },
    /// Forget what Talking Saba-chan remembers (all Discord servers, or one)
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
    /// Delete a server (an imported server only loses its link; its folder stays)
    #[command(visible_alias = "rm", alias = "delete")]
    Remove {
        server: Option<String>,
        /// Imported server: keep the game settings files as they are now instead of restoring them to before the import
        #[arg(long)]
        keep_settings: bool,
    },
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
    /// Who is online
    Players { server: Option<String> },
    /// Check from the internet whether friends can reach this server (port forwarding)
    #[command(name = "port-check", visible_alias = "ports")]
    PortCheck { server: Option<String> },
    /// Send one line the way the server takes commands (console, else RCON) and show its reply
    Rcon {
        server: String,
        #[arg(required = true, trailing_var_arg = true, allow_hyphen_values = true, value_name = "LINE")]
        line: Vec<String>,
    },
    /// Saved log of past runs (the console keeps only the current one)
    Log {
        server: Option<String>,
        /// Which run (`--runs` lists them), default the latest
        #[arg(long)]
        source: Option<String>,
        /// Last N lines
        #[arg(short = 'n', long)]
        lines: Option<usize>,
        /// List the saved runs
        #[arg(long, conflicts_with_all = ["lines", "all"])]
        runs: bool,
        /// The whole run (to save: `> file.log`)
        #[arg(long)]
        all: bool,
    },
    /// Read-only lookups the game module offers (lists them when none is given)
    Query {
        server: String,
        /// Query name
        name: Option<String>,
        /// Input as KEY=VALUE
        #[arg(value_name = "KEY=VALUE")]
        args: Vec<String>,
    },
    /// What Discord chat calls this server and its commands (the same in every Discord server). No options: show
    Discord {
        server: Option<String>,
        /// Words for the server, comma-separated or repeated (replace the current ones). `--word ""` removes them
        #[arg(long, value_delimiter = ',', requires = "server", conflicts_with = "clear")]
        word: Option<Vec<String>>,
        /// Words for one command: COMMAND=WORD,WORD (replace that command's words, `COMMAND=` removes them). Repeatable
        #[arg(long, value_name = "COMMAND=WORDS", requires = "server", conflicts_with = "clear")]
        command: Vec<String>,
        /// Remove every word of this server
        #[arg(long, requires = "server")]
        clear: bool,
    },
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
    /// Show the exact launch line the daemon would run (starts nothing)
    LaunchPreview {
        server: Option<String>,
        /// Extra launch args to try (quoted like a shell); saved ones are used when omitted
        #[arg(long = "args", allow_hyphen_values = true)]
        extra_args: Option<String>,
    },
    /// Read and accept the license agreements the server's module asks for (e.g. a game EULA)
    Agree { server: Option<String> },
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
    /// With `--from`: move the folder into saba-chan instead of linking it in place (same drive only)
    #[arg(long = "move", requires = "from")]
    pub move_folder: bool,
    /// Game port (`--port` is the daemon's)
    #[arg(long = "game-port", value_name = "PORT")]
    pub game_port: Option<u16>,
    /// Server version to install (games that download their server; default: the latest release)
    #[arg(long = "game-version", value_name = "VERSION", conflicts_with = "from")]
    pub game_version: Option<String>,
    /// Install into this empty folder instead of saba-chan's own (its files stay when the server is removed)
    #[arg(long, value_name = "DIR", conflicts_with = "from")]
    pub dir: Option<String>,
    /// Let the server run on its own: saba-chan starts it and watches whether it runs (no saba-chan console)
    #[arg(long)]
    pub unmanaged: bool,
    /// Start from one of the game's presets (`saba module info` lists them)
    #[arg(long, value_name = "ID")]
    pub preset: Option<String>,
    /// Extra settings KEY=VALUE (repeatable)
    #[arg(long = "set", value_name = "KEY=VALUE")]
    pub set: Vec<String>,
    /// Start it as soon as it is ready
    #[arg(long)]
    pub start: bool,
    /// Don't wait for the install to finish
    #[arg(long)]
    pub no_wait: bool,
    /// Agreement id the module declares (`[[agreements]]`, e.g. a game EULA), repeatable. Only pass ones you have read.
    /// `--yes` never accepts an agreement
    #[arg(long = "accept", value_name = "ID")]
    pub accept: Vec<String>,
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
    /// Self-check to paste when asking for help: version, paths, token file, modules, extensions, recent warnings
    Diagnostics,
    /// The daemon's own log
    Logs {
        #[arg(short = 'n', long, default_value_t = 100)]
        lines: usize,
        #[arg(short = 'f', long)]
        follow: bool,
    },
    /// Desktop app windows connected to the daemon
    Clients,
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
    /// Make a new game module with your AI agent: install commands, a first prompt, and this PC's paths
    Maker {
        /// Game to put in the first prompt
        game: Option<String>,
    },
    /// Check what a module's Python code can do against what it declares
    Audit { module: Option<String> },
    /// Download server files only, into a folder you choose
    InstallFiles {
        module: String,
        #[arg(long)]
        version: String,
        #[arg(long, value_name = "DIR")]
        dir: String,
        /// Server executable file name to download as (default: the module's `server_executable`)
        #[arg(long, alias = "jar", value_name = "FILE")]
        executable: Option<String>,
        /// Agreement id the module declares (`[[agreements]]`), repeatable. Only pass ones you have read
        #[arg(long = "accept", value_name = "ID")]
        accept: Vec<String>,
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
    /// Check what an extension's Python code can do against what it declares
    Audit { extension: Option<String> },
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
    /// Erase everything the relay keeps about your Discord account (opens a consent page), then disconnect this PC
    Erase,
    /// The AI agent (remote MCP) linked to your Discord account. It can manage every PC of the account
    Agent {
        /// Unlink the agent (it loses access at once)
        #[arg(long)]
        unlink: bool,
    },
    /// Set the node token by hand (from a pairing done elsewhere). Asked for without showing it, or one line on stdin; never an argument
    Token,
    /// Show or change the chat prefixes of a Discord server: `prefix`, `prefix A B`, `prefix --default`
    Prefix {
        values: Vec<String>,
        /// Go back to the default prefix of the app language
        #[arg(long, conflicts_with = "values")]
        default: bool,
        /// The Discord server, by name or id (without it: the only paired server)
        #[arg(long)]
        guild: Option<String>,
    },
    /// Translated command names in the slash commands of a Discord server. No options: show
    Translate {
        guild: Option<String>,
        /// Show translated command names
        #[arg(long, conflicts_with = "off")]
        on: bool,
        /// Show only the names in the module
        #[arg(long)]
        off: bool,
    },
    /// Discord servers paired with this PC
    Nodes,
    /// Members of a paired Discord server (name or id; without it: the only paired server)
    Members { guild: Option<String> },
    /// Talking Saba-chan channels (several), the Singing Saba-chan channel (one), the notification channel and the game chat channel of a paired Discord server. No options: show
    Channels {
        /// Without it: every paired Discord server (changing needs exactly one)
        guild: Option<String>,
        /// Channels Talking Saba-chan talks in, by name or id (comma-separated). Replaces the current list
        #[arg(long, value_delimiter = ',', conflicts_with = "no_chatbot")]
        chatbot: Vec<String>,
        /// Unassign every Talking Saba-chan channel
        #[arg(long)]
        no_chatbot: bool,
        /// The Singing Saba-chan channel, by name or id. Other messages in it get deleted
        #[arg(long, conflicts_with = "no_music")]
        music: Option<String>,
        /// Unassign the Singing Saba-chan channel
        #[arg(long)]
        no_music: bool,
        /// The notification channel (crashes, backups, updates, scheduled restarts), by name or id
        #[arg(long, conflicts_with = "no_notify")]
        notify: Option<String>,
        /// Unassign the notification channel
        #[arg(long)]
        no_notify: bool,
        /// The game chat channel (in-game chat of servers with game chat relay on), by name or id
        #[arg(long, conflicts_with = "no_game_chat")]
        game_chat: Option<String>,
        /// Unassign the game chat channel
        #[arg(long)]
        no_game_chat: bool,
    },
    /// The language the bot answers in on a Discord server (`auto` follows this PC's language). No value: show
    Lang { guild: String, value: Option<String> },
    /// Put the bot to sleep on a Discord server (it ignores commands until woken)
    Sleep { guild: Option<String> },
    /// Wake the bot on a Discord server
    Wake { guild: Option<String> },
    /// Singing Saba-chan settings of a Discord server. No options: show
    Music {
        guild: Option<String>,
        /// Turn Singing Saba-chan on
        #[arg(long, conflicts_with = "off")]
        on: bool,
        /// Turn Singing Saba-chan off
        #[arg(long)]
        off: bool,
        /// Show the player panel
        #[arg(long, value_name = "on|off", value_parser = on_off)]
        ui: Option<bool>,
        /// Queue lines in the player panel (1-20)
        #[arg(long, value_name = "N")]
        queue_lines: Option<i64>,
        /// Player panel refresh in milliseconds (2000-10000)
        #[arg(long, value_name = "MS")]
        refresh: Option<i64>,
        /// Even out loudness between tracks
        #[arg(long, value_name = "on|off", value_parser = on_off)]
        normalize: Option<bool>,
    },
    /// When the bot asks with a Discord button before running (all, caution, danger, none). No options: show
    Confirm {
        guild: Option<String>,
        /// Requests to the bot (prefix and slash commands)
        #[arg(long, value_name = "LEVEL", value_parser = ["all", "caution", "danger", "none"])]
        direct: Option<String>,
        /// Commands Talking Saba-chan picks (its admin tasks always ask)
        #[arg(long, value_name = "LEVEL", value_parser = ["all", "caution", "danger", "none"])]
        chatbot: Option<String>,
    },
    /// Link to invite the Saba-chan bot to a Discord server
    Invite,
    /// Connection requests sent from Discord with `/사바쨩 연결` and no code. No id: show. ID: accept, `--decline`: refuse
    Link {
        id: Option<String>,
        #[arg(long, requires = "id")]
        decline: bool,
    },
    /// Servers a Discord server can see and run. No servers: show. `--all`: every server
    Allow {
        guild: String,
        /// Servers by name or id (replace the current list)
        #[arg(conflicts_with = "all")]
        servers: Vec<String>,
        /// No limit: every server of this PC
        #[arg(long)]
        all: bool,
    },
    /// Post in the Discord server's alert channel when Saba-chan starts a server (off by default).
    /// No server: show. Server with `--on`/`--off`: change
    Notice {
        guild: String,
        /// Server by name or id
        server: Option<String>,
        /// Turn the start alert on
        #[arg(long, requires = "server", conflicts_with = "off")]
        on: bool,
        /// Turn the start alert off
        #[arg(long, requires = "server")]
        off: bool,
    },
    /// Bot settings back to their defaults (all when no key is named)
    Reset { keys: Vec<String> },

    /// Show or change a bot setting: KEY=VALUE (dotted keys as shown with no arguments, lists comma-separated)
    Config {
        #[arg(value_name = "KEY=VALUE")]
        pairs: Vec<String>,
    },
}

/// `on`·`off` (그리고 true·false)
fn on_off(s: &str) -> Result<bool, String> {
    match s.to_ascii_lowercase().as_str() {
        "on" | "true" => Ok(true),
        "off" | "false" => Ok(false),
        _ => Err("on or off".into()),
    }
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
    /// Save settings and the server list to a zip, to move to another PC (game files not included)
    Export {
        file: std::path::PathBuf,
        /// Also save passwords, the Talking Saba-chan API key and the bot pairing
        #[arg(long)]
        secrets: bool,
    },
    /// Where Saba-chan keeps its data, modules and extensions, and the default folders
    #[command(visible_alias = "dirs")]
    Paths,
    /// Move modules or extensions to another folder. Installed files are moved; servers must be stopped
    Move {
        /// What to move
        #[arg(value_enum)]
        kind: DirKind,
        /// Folder to use. Saba-chan makes a `modules` or `extensions` folder inside it
        folder: std::path::PathBuf,
    },
    /// Folders and files on the daemon's PC (with --daemon, the remote PC), to pick paths there. Drives or `/` without a path
    #[command(visible_alias = "ls")]
    Browse {
        /// Full path of the folder to show
        path: Option<String>,
        /// Also show files with these extensions (comma separated, `*` for all)
        #[arg(long, value_name = "EXT")]
        files: Option<String>,
    },
    /// What starts when you sign in to this PC: off, daemon (daemon and tray only) or full (with the GUI window). Shows the current value without a mode
    Autostart {
        #[arg(value_enum)]
        mode: Option<AutostartMode>,
    },
    /// Bring in settings and servers from an export zip
    Import {
        file: std::path::PathBuf,
        /// Only show what would change
        #[arg(long)]
        dry_run: bool,
        /// Replace servers that already exist (running ones are skipped)
        #[arg(long)]
        overwrite: bool,
    },
}

/// `config move`로 옮길 수 있는 폴더 (데몬 `/api/system/paths/move`의 `kind`)
#[derive(clap::ValueEnum, Clone, Copy, Debug, PartialEq, Eq)]
pub enum DirKind {
    Modules,
    Extensions,
}

impl DirKind {
    pub fn as_str(self) -> &'static str {
        match self {
            DirKind::Modules => "modules",
            DirKind::Extensions => "extensions",
        }
    }
}

/// `config autostart`의 모드 (데몬 `/api/system/autostart`의 `mode`)
#[derive(clap::ValueEnum, Clone, Copy, Debug, PartialEq, Eq)]
pub enum AutostartMode {
    Off,
    Daemon,
    Full,
}

impl AutostartMode {
    pub fn as_str(self) -> &'static str {
        match self {
            AutostartMode::Off => "off",
            AutostartMode::Daemon => "daemon",
            AutostartMode::Full => "full",
        }
    }
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

/// 인자를 읽는다. 맨 위 `help`는 우리 명령이라 clap의 하위 `help`가 꺼진다:
/// `server help start`처럼 하위 명령 자리에 온 `help`는 맨 앞으로 옮겨 다시 읽는다
/// 이 줄에 `--daemon`을 적었을 때만 그 값 (셸·TUI 한 줄). `SABA_DAEMON`에서 다시 채운 값은 시작할 때 고른 대상을 덮지 않는다
pub fn line_daemon(matches: &clap::ArgMatches) -> Option<String> {
    (matches.value_source("daemon") == Some(clap::parser::ValueSource::CommandLine))
        .then(|| matches.get_one::<String>("daemon").cloned())
        .flatten()
}

pub fn parse(args: Vec<std::ffi::OsString>) -> Result<clap::ArgMatches, clap::Error> {
    match localized().try_get_matches_from(&args) {
        Err(e) if e.kind() == clap::error::ErrorKind::InvalidSubcommand && args.iter().skip(1).any(|a| a == "help") => {
            let mut moved: Vec<std::ffi::OsString> = args.iter().filter(|a| *a != "help").cloned().collect();
            moved.insert(1.min(moved.len()), "help".into());
            // 옮긴 경로가 실제 명령일 때만: 오타면 clap의 원래 제안("혹시 start?")을 살린다
            let real = |m: &clap::ArgMatches| {
                let path: Vec<String> = m.subcommand_matches("help").and_then(|h| h.get_many::<String>("command")).into_iter().flatten().cloned().collect();
                find(&Cli::command(), &path).is_some()
            };
            localized().try_get_matches_from(moved).ok().filter(real).ok_or(e)
        }
        r => r,
    }
}

/// 이름이나 별칭으로 하위 명령 경로를 따라간다. 없으면 None
pub fn find<'c>(root: &'c clap::Command, path: &[String]) -> Option<&'c clap::Command> {
    path.iter().try_fold(root, |cmd, word| cmd.find_subcommand(word))
}

/// `help --json`: 명령 트리·인자·종료 코드. 전역 플래그는 `globalArgs`에 한 번만 싣는다
pub fn schema(root: &clap::Command, path: &[String]) -> Option<serde_json::Value> {
    use serde_json::{json, Value};
    fn text(s: Option<&clap::builder::StyledStr>) -> Value {
        s.map_or(Value::Null, |s| json!(s.to_string()))
    }
    fn arg(a: &clap::Arg) -> Value {
        let takes_value = a.get_action().takes_values();
        json!({
            "name": a.get_id().as_str(),
            "long": a.get_long(),
            "short": a.get_short().map(String::from),
            "positional": a.is_positional(),
            "required": a.is_required_set(),
            "takesValue": takes_value,
            "multiple": matches!(a.get_action(), clap::ArgAction::Append | clap::ArgAction::Count)
                || a.get_num_args().is_some_and(|n| n.max_values() > 1),
            // 켜고 끄는 플래그는 값 이름·기본값(false)이 뜻이 없다
            "valueName": a.get_value_names().and_then(|v| v.first()).filter(|_| takes_value).map(|v| v.as_str()),
            "default": if takes_value { a.get_default_values().iter().map(|v| v.to_string_lossy()).collect::<Vec<_>>() } else { vec![] },
            "choices": if takes_value { a.get_possible_values().iter().map(|v| v.get_name().to_string()).collect::<Vec<_>>() } else { vec![] },
            "help": text(a.get_help()),
        })
    }
    fn node(cmd: &clap::Command, path: &str, top: bool) -> Value {
        let args: Vec<Value> = cmd
            .get_arguments()
            .filter(|a| !a.is_global_set() && !a.is_hide_set() && !matches!(a.get_id().as_str(), "help" | "version"))
            .map(arg)
            .collect();
        // clap이 붙이는 하위 `help`는 뺀다. 맨 위 `help`만 우리 명령이다
        let subs: Vec<Value> = cmd
            .get_subcommands()
            .filter(|s| !s.is_hide_set() && (top || s.get_name() != "help"))
            .map(|s| node(s, format!("{} {}", path, s.get_name()).trim(), false))
            .collect();
        json!({
            "path": path,
            "name": cmd.get_name(),
            "aliases": cmd.get_visible_aliases().collect::<Vec<_>>(),
            "about": text(cmd.get_about()),
            "args": args,
            "subcommands": subs,
        })
    }
    let target = find(root, path)?;
    // 별칭으로 찾아도 경로는 본 이름으로
    let canonical: Vec<&str> = (1..=path.len()).filter_map(|n| find(root, &path[..n]).map(|c| c.get_name())).collect();
    let globals: Vec<Value> = root.get_arguments().filter(|a| a.is_global_set()).map(arg).collect();
    let exit_codes: Vec<Value> = crate::error::EXIT_CODES
        .iter()
        .map(|(exit, codes)| json!({ "exit": exit, "codes": codes }))
        .collect();
    Some(json!({
        "name": root.get_name(),
        "version": env!("CARGO_PKG_VERSION"),
        "command": node(target, &canonical.join(" "), path.is_empty()),
        "globalArgs": globals,
        "exitCodes": exit_codes,
        "notes": "--json prints one JSON document on stdout. A failure prints {\"ok\":false,\"error\",\"error_code\"} and exits with a code below; for exit 1 error_code may also be the daemon's own code.",
    }))
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

    /// 명령 본문 안의 `--lang=ko`는 전역 옵션이 아니다: 데몬 언어를 바꾸지 않게 실제 전역 옵션만 쓴다
    #[test]
    fn only_a_daemon_written_on_the_line_changes_the_target() {
        let m = |args: &[&str]| parse(std::iter::once("saba-chan-cli").chain(args.iter().copied()).map(Into::into).collect()).unwrap();
        assert_eq!(line_daemon(&m(&["--daemon", "test", "status"])).as_deref(), Some("test"));
        assert_eq!(line_daemon(&m(&["status", "--daemon=local"])).as_deref(), Some("local"));
        assert_eq!(line_daemon(&m(&["status"])), None);
    }

    #[test]
    fn a_lang_inside_command_text_is_not_the_global_option() {
        let p = |args: &[&str]| Cli::try_parse_from(std::iter::once("saba-chan-cli").chain(args.iter().copied())).unwrap();
        assert_eq!(p(&["chat", "mc", "say", "--lang=ko"]).global.lang, None);
        assert_eq!(p(&["--lang", "ko", "chat", "mc", "say", "hi"]).global.lang.as_deref(), Some("ko"));
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
        match p(&["server", "discord", "mc", "--word", "크리,크", "--command", "start=켜,on", "--command", "stop="]).unwrap().cmd {
            Some(Cmd::Server(ServerCmd::Discord { server, word, command, .. })) => {
                assert_eq!(server.as_deref(), Some("mc"));
                assert_eq!(word, Some(vec!["크리".to_string(), "크".to_string()]));
                assert_eq!(command, vec!["start=켜,on", "stop="]);
            }
            _ => panic!("server discord did not parse"),
        }
        assert!(p(&["server", "discord", "--word", "크리"]).is_err(), "--word needs a server");
        // 낱말은 인스턴스에 딸린다: 디스코드 서버를 고르지 않는다
        assert!(p(&["server", "discord", "mc", "--guild", "123"]).is_err());
        // 서버마다 부르는 말은 서버 명령으로 옮겼다
        assert!(p(&["bot", "words", "123"]).is_err());
        assert!(p(&["bot", "alias"]).is_err(), "old alias command is gone");
        assert!(matches!(p(&["bot", "channels"]).unwrap().cmd, Some(Cmd::Bot(BotCmd::Channels { guild: None, .. }))));
        assert!(matches!(p(&["bot", "members"]).unwrap().cmd, Some(Cmd::Bot(BotCmd::Members { guild: None }))));
        // 토큰은 셸 기록에 남지 않게 인자로 받지 않는다 (C15)
        assert!(p(&["bot", "token", "secret-token"]).is_err());
        assert!(matches!(p(&["bot", "token"]).unwrap().cmd, Some(Cmd::Bot(BotCmd::Token))));
        assert!(matches!(p(&["bot", "erase"]).unwrap().cmd, Some(Cmd::Bot(BotCmd::Erase))));
        assert!(matches!(p(&["bot", "agent", "--unlink"]).unwrap().cmd, Some(Cmd::Bot(BotCmd::Agent { unlink: true }))));
        assert!(p(&["--json", "status"]).unwrap().global.json);
        assert!(p(&["status", "--json"]).unwrap().global.json);
    }

    /// AI가 명령을 추측하지 않게 명령 트리·인자·종료 코드를 JSON으로 낸다
    #[test]
    fn help_json_describes_the_command_tree() {
        let p = |args: &[&str]| Cli::try_parse_from(std::iter::once("saba-chan-cli").chain(args.iter().copied()));
        let h = p(&["help", "--json", "server", "start"]).unwrap();
        assert!(h.global.json);
        assert!(matches!(h.cmd, Some(Cmd::Help { command }) if command == ["server", "start"]));

        let all = schema(&Cli::command(), &[]).unwrap();
        fn paths(node: &serde_json::Value, out: &mut Vec<String>) {
            for s in node["subcommands"].as_array().unwrap() {
                out.push(s["path"].as_str().unwrap().to_string());
                paths(s, out);
            }
        }
        let mut seen = Vec::new();
        paths(&all["command"], &mut seen);
        fn names(cmd: &clap::Command, prefix: &str, out: &mut Vec<String>) {
            for s in cmd.get_subcommands().filter(|s| s.get_name() != "help") {
                let p = format!("{} {}", prefix, s.get_name()).trim().to_string();
                out.push(p.clone());
                names(s, &p, out);
            }
        }
        let mut words = Vec::new();
        names(&Cli::command(), "", &mut words);
        assert!(words.len() > 50 && seen.iter().any(|p| p == "help"));
        for w in &words {
            assert!(seen.contains(w), "{} is not described", w);
        }
        assert!(all["globalArgs"].as_array().unwrap().iter().any(|a| a["long"] == "json"));
        assert!(all["exitCodes"].as_array().unwrap().iter().any(|e| e["exit"] == 3 && e["codes"][0] == "daemon_unreachable"));

        // 별칭으로도 찾는다
        let start = schema(&Cli::command(), &["servers".into(), "start".into()]).unwrap();
        assert_eq!(start["command"]["path"], "server start");
        let server = start["command"]["args"].as_array().unwrap().iter().find(|a| a["name"] == "server").unwrap();
        assert_eq!(server["positional"], true);
        assert!(start["command"]["args"].as_array().unwrap().iter().all(|a| a["long"] != "json"), "global flags are listed once");
        assert_eq!(schema(&Cli::command(), &["nope".into()]), None);

        // 예전 모양 `server help start`도 같은 명령이 된다
        crate::i18n::init("en");
        let m = parse(["saba-chan-cli", "server", "help", "start"].map(Into::into).to_vec()).unwrap();
        assert!(matches!(<Cli as clap::FromArgMatches>::from_arg_matches(&m).unwrap().cmd, Some(Cmd::Help { command }) if command == ["server", "start"]));
        // 오타는 옮겨 읽지 않고 clap의 원래 오류(제안 포함)를 낸다
        let typo = parse(["saba-chan-cli", "server", "strat", "help"].map(Into::into).to_vec()).unwrap_err();
        assert_eq!(typo.kind(), clap::error::ErrorKind::InvalidSubcommand);
    }

    #[test]
    fn folder_moves_and_install_location_parse() {
        let cli = Cli::try_parse_from(["saba-chan-cli", "config", "move", "extensions", "D:/Games/Saba"]).unwrap();
        assert!(matches!(cli.cmd, Some(Cmd::Config(ConfigCmd::Move { kind: DirKind::Extensions, folder })) if folder == std::path::Path::new("D:/Games/Saba")));
        assert!(Cli::try_parse_from(["saba-chan-cli", "config", "move", "instances", "x"]).is_err(), "only modules and extensions move");
        let cli = Cli::try_parse_from(["saba-chan-cli", "--daemon", "srv", "config", "ls", "/srv/game", "--files", "jar,zip"]).unwrap();
        assert!(matches!(cli.cmd, Some(Cmd::Config(ConfigCmd::Browse { path: Some(p), files: Some(f) })) if p == "/srv/game" && f == "jar,zip"));
        let cli = Cli::try_parse_from(["saba-chan-cli", "install", "--components-dir", "/srv/saba"]).unwrap();
        assert!(matches!(cli.cmd, Some(Cmd::Install(a)) if a.components_dir.as_deref() == Some(std::path::Path::new("/srv/saba"))));
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
                if crate::i18n::try_t_in("ko", &format!("help.cmd.{}", p)).is_none() {
                    missing.push(p.clone());
                }
                walk(s, &p, missing);
            }
        }
        let mut missing = Vec::new();
        walk(&Cli::command(), "", &mut missing);
        assert!(missing.is_empty(), "help.cmd keys missing: {:?}", missing);
    }
    /// `server add --preset`은 게임의 추천 설정 id다. 권한 프리셋(`perm preset`)의 설명을 빌려 쓰면 안 된다 (L73)
    #[test]
    fn server_add_preset_is_explained_as_a_game_preset() {
        crate::i18n::init("ko");
        let cmd = localized();
        let help = |path: &[&str]| {
            let mut c = &cmd;
            for name in &path[..path.len() - 1] {
                c = c.find_subcommand(name).unwrap();
            }
            let arg = c.get_arguments().find(|a| a.get_id() == path[path.len() - 1]).unwrap();
            arg.get_help().map(|h| h.to_string()).unwrap_or_default()
        };
        assert!(!help(&["server", "add", "preset"]).contains("loose"));
        assert!(help(&["perm", "preset", "preset"]).contains("loose"));
    }

    #[test]
    fn every_argument_has_a_korean_help_line() {
        crate::i18n::init("ko");
        fn walk(cmd: &clap::Command, path: &str, missing: &mut Vec<String>) {
            for a in cmd.get_arguments() {
                let id = a.get_id().as_str();
                if matches!(id, "help" | "version") || a.is_hide_set() {
                    continue;
                }
                let found = crate::i18n::try_t_in("ko", &format!("help.arg.{}.{}", path, id))
                    .or_else(|| crate::i18n::try_t_in("ko", &format!("help.arg.{}", id)));
                if found.is_none() {
                    missing.push(format!("{} --{}", path, id));
                }
            }
            for s in cmd.get_subcommands().filter(|s| s.get_name() != "help") {
                let p = if path.is_empty() { s.get_name().to_string() } else { format!("{}.{}", path, s.get_name()) };
                walk(s, &p, missing);
            }
        }
        let mut missing = Vec::new();
        walk(&Cli::command(), "", &mut missing);
        assert!(missing.is_empty(), "help.arg keys missing: {:?}", missing);
    }
}
