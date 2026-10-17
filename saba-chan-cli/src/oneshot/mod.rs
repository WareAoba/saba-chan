//! One-shot (single-command) CLI mode.
//!
//! Provides scriptable, machine-readable access to all major daemon features
//! that the GUI exposes. Mirrors the daemon's REST surface as closely as
//! practical so that humans, shell scripts, and CI pipelines can drive the
//! daemon without launching the TUI.
//!
//! Behaviour rules
//! ---------------
//! * Stdout = command result (text or JSON). Stderr = diagnostics.
//! * Exit codes:
//!   * 0 — success
//!   * 1 — generic error (server returned 4xx/5xx, parse failure, …)
//!   * 2 — daemon offline / connection refused
//!   * 3 — bad usage / missing argument
//!   * 4 — target not found (instance/module/extension)
//! * JSON mode (`--json` global flag) prints the raw API response.
//! * No interactive prompts. Anything that needs confirmation requires
//!   `--yes`.

use clap::{Parser, Subcommand};
use serde_json::{json, Value};

use crate::client::DaemonClient;

// ─── Exit code constants ───────────────────────────────────────────────

pub const EXIT_OK: i32 = 0;
pub const EXIT_ERR: i32 = 1;
pub const EXIT_DAEMON_OFFLINE: i32 = 2;
pub const EXIT_BAD_USAGE: i32 = 3;
pub const EXIT_NOT_FOUND: i32 = 4;

// ─── Top-level CLI ─────────────────────────────────────────────────────

#[derive(Parser, Debug, Clone)]
#[command(
    name = "saba-chan-cli",
    about = "Saba-chan game-server daemon CLI (interactive TUI when run with no subcommand)",
    long_about = "Cross-platform CLI/TUI for the Saba-chan Core daemon.\n\
                  Run without any subcommand to launch the interactive TUI.\n\
                  Supply a subcommand to perform a single one-shot operation\n\
                  (suitable for shell scripts and CI).",
    version,
    propagate_version = true,
    disable_help_subcommand = true,
)]
pub struct Cli {
    /// Override the daemon base URL (default: http://127.0.0.1:<settings.json port>).
    #[arg(long, global = true, env = "SABA_DAEMON_URL")]
    pub daemon: Option<String>,

    /// Emit machine-readable JSON to stdout instead of human text.
    #[arg(long, short = 'j', global = true)]
    pub json: bool,

    /// Suppress the trailing newline / status decorations on success output.
    #[arg(long, short = 'q', global = true)]
    pub quiet: bool,

    /// Auto-confirm destructive operations (delete, remove, reset, shutdown).
    #[arg(long, short = 'y', global = true)]
    pub yes: bool,

    /// Set by the updater when it relaunches the CLI after an update. Ignored.
    #[arg(long = "after-update", global = true, hide = true)]
    pub after_update: bool,

    #[command(subcommand)]
    pub command: Option<Command>,
}

#[derive(Subcommand, Debug, Clone)]
pub enum Command {
    /// Launch the interactive TUI (default when no subcommand is given).
    Tui,

    /// Launch the inline REPL shell (Claude-Code-style; preserves scrollback).
    Shell {
        /// Pre-select a server instance so bare `start`/`stop`/`status` work.
        #[arg(long = "use", value_name = "INSTANCE")]
        use_instance: Option<String>,
    },

    /// Show overall daemon health, version, and instance/extension counts.
    Status,

    /// Manage server instances (start/stop/console/properties/etc.).
    #[command(subcommand)]
    Server(ServerCmd),

    /// Manage installed and remote modules.
    #[command(subcommand)]
    Module(ModuleCmd),

    /// Manage extensions (enable/disable/install/remove).
    #[command(subcommand)]
    Extension(ExtensionCmd),

    /// Control the Saba-Core daemon process (start/stop/restart/console).
    #[command(subcommand)]
    Daemon(DaemonCmd),

    /// Self-update operations (check, download, apply).
    #[command(subcommand)]
    Update(UpdateCmd),

    /// Manage CLI-local settings (~/AppData/Roaming/saba-chan/cli-settings.json).
    #[command(subcommand)]
    Config(ConfigCmd),

    /// Module-name aliases used by the cloud-relay Discord bot (bot-config.json).
    #[command(subcommand)]
    Alias(AliasCmd),

    /// System-level inspection (component versions, build info).
    #[command(subcommand)]
    System(SystemCmd),
}

// ─── server ────────────────────────────────────────────────────────────

#[derive(Subcommand, Debug, Clone)]
pub enum ServerCmd {
    /// List configured instances.
    List,
    /// Show full instance metadata.
    Info { name: String },
    /// Show runtime status (running / stopped / port / pid).
    Status { name: String },
    /// Create a new instance entry.
    Create {
        /// Display name for the instance.
        name: String,
        /// Module to use (e.g. minecraft, palworld). Required unless --module-name is set.
        #[arg(long = "module", short = 'm')]
        module: String,
        /// Optional JSON object with extra fields merged into the create payload.
        #[arg(long, value_name = "JSON")]
        extra: Option<String>,
    },
    /// Install a server (download/extract the actual game files for an instance).
    Install {
        /// Module to install (e.g. minecraft).
        module: String,
        /// Game version to install (or "latest").
        ///
        /// The flag is `--game-version` to avoid colliding with clap's built-in
        /// `--version`; the field name is also `game_version` for the same reason.
        #[arg(long = "game-version", default_value = "latest")]
        game_version: String,
        /// Optional instance name to attach to. If omitted, the daemon picks a default.
        #[arg(long)]
        name: Option<String>,
        /// Optional JSON object merged into the install payload (for extra module-specific fields).
        #[arg(long, value_name = "JSON")]
        extra: Option<String>,
    },
    /// Update instance settings via PATCH. Use KEY=VALUE pairs or --body <JSON>.
    Set {
        name: String,
        /// `key=value` pairs. Numbers/bools auto-detected; everything else is string.
        #[arg(value_name = "KEY=VALUE", num_args = 0..)]
        pairs: Vec<String>,
        /// Pass a raw JSON object as the patch body instead of key=value pairs.
        ///
        /// (Distinct from the global `--json` output flag.)
        #[arg(long, value_name = "JSON", conflicts_with = "pairs")]
        body: Option<String>,
    },
    /// Reorder instances. Pass instance ids/names in desired order.
    Reorder {
        /// Instance names or ids in the desired display order.
        #[arg(required = true, num_args = 1..)]
        order: Vec<String>,
    },
    /// Write server.properties. Use KEY=VALUE pairs (merged) or --body <JSON> (replace).
    PropertiesWrite {
        name: String,
        /// `key=value` pairs to merge into the existing server.properties.
        #[arg(value_name = "KEY=VALUE", num_args = 0..)]
        pairs: Vec<String>,
        /// Replace the whole properties object with this JSON literal (skips merge).
        #[arg(long, value_name = "JSON", conflicts_with = "pairs")]
        body: Option<String>,
    },
    /// Start an instance.
    Start { name: String },
    /// Stop an instance.
    Stop {
        name: String,
        /// Force-kill the process instead of graceful shutdown.
        #[arg(long, short = 'f')]
        force: bool,
    },
    /// Stop and start an instance.
    Restart { name: String },
    /// Send text to the managed process stdin.
    Exec {
        name: String,
        /// Command line to send (joined with spaces).
        #[arg(required = true, num_args = 1..)]
        command: Vec<String>,
    },
    /// Execute an RCON command against the instance.
    Rcon {
        name: String,
        #[arg(required = true, num_args = 1..)]
        command: Vec<String>,
    },
    /// Execute a REST command against the instance.
    Rest {
        name: String,
        endpoint: String,
        /// HTTP method (default: GET).
        #[arg(long, default_value = "GET")]
        method: String,
        /// JSON body (verbatim).
        #[arg(long)]
        body: Option<String>,
    },
    /// Print recent console output.
    Console {
        name: String,
        /// Maximum number of lines to fetch.
        #[arg(long, default_value_t = 200)]
        count: u32,
        /// Stream new lines as they arrive (Ctrl+C to stop).
        #[arg(long, short = 'f')]
        follow: bool,
        /// Polling interval in milliseconds when --follow is set.
        #[arg(long, default_value_t = 1000)]
        interval: u64,
    },
    /// Run server diagnostics.
    Diagnose { name: String },
    /// Read server.properties.
    Properties { name: String },
    /// Reset server.properties to module defaults.
    PropertiesReset { name: String },
    /// Reset the server to its initial install state.
    Reset { name: String },
    /// Accept the EULA (Minecraft etc.).
    AcceptEula { name: String },
    /// Check for a server (game) update.
    CheckUpdate { name: String },
    /// Apply pending server (game) update.
    ApplyUpdate { name: String },
    /// Show the installed game version.
    InstalledVersion { name: String },
    /// Validate the instance configuration.
    Validate { name: String },
    /// Delete an instance (requires --yes).
    Delete { name: String },
}

// ─── module ────────────────────────────────────────────────────────────

#[derive(Subcommand, Debug, Clone)]
pub enum ModuleCmd {
    /// List loaded modules.
    List,
    /// Show metadata for a module.
    Info { name: String },
    /// List available game versions for a module.
    Versions { name: String },
    /// Reload module manifests on the daemon.
    Refresh,
    /// Fetch the remote module manifest.
    Manifest,
    /// Install a module from the remote manifest.
    Install { id: String },
    /// Remove an installed module (requires --yes).
    Remove { id: String },
}

// ─── extension ─────────────────────────────────────────────────────────

#[derive(Subcommand, Debug, Clone)]
pub enum ExtensionCmd {
    /// List installed extensions.
    List,
    /// Enable an extension.
    Enable { id: String },
    /// Disable an extension.
    Disable { id: String },
    /// Fetch the remote extension manifest.
    Manifest,
    /// Install an extension from the manifest.
    Install { id: String },
    /// Remove an installed extension (requires --yes).
    Remove { id: String },
    /// Trigger a rescan of local extensions.
    Rescan,
    /// Check for available extension updates.
    CheckUpdates,
}

// ─── daemon ────────────────────────────────────────────────────────────

#[derive(Subcommand, Debug, Clone)]
pub enum DaemonCmd {
    /// Probe whether the daemon is reachable.
    Status,
    /// Spawn the Saba-Core daemon binary.
    Start,
    /// Gracefully stop the daemon (falls back to OS kill on failure).
    Stop,
    /// Stop then start the daemon.
    Restart,
    /// Print recent daemon log lines.
    Console {
        /// Cursor (last seen log id). Use 0 to fetch from the start of the buffer.
        #[arg(long, default_value_t = 0)]
        since: u64,
        /// Maximum number of entries to fetch.
        #[arg(long, default_value_t = 200)]
        count: u32,
        /// Stream new lines as they arrive (Ctrl+C to stop).
        #[arg(long, short = 'f')]
        follow: bool,
        /// Polling interval in milliseconds when --follow is set.
        #[arg(long, default_value_t = 1000)]
        interval: u64,
    },
    /// Trigger a graceful shutdown via the daemon API (requires --yes).
    Shutdown,
}

// ─── update ────────────────────────────────────────────────────────────

#[derive(Subcommand, Debug, Clone)]
pub enum UpdateCmd {
    /// Force an update check.
    Check,
    /// Show the cached update status.
    Status,
    /// Begin downloading available updates.
    Download,
    /// Apply downloaded updates (requires --yes for self-replace).
    Apply,
    /// Show the current download progress.
    Progress,
    /// Show updater configuration.
    ConfigShow,
    /// Set updater configuration value.
    ConfigSet { key: String, value: String },
}

// ─── config ────────────────────────────────────────────────────────────

#[derive(Subcommand, Debug, Clone)]
pub enum ConfigCmd {
    /// Print all CLI settings (key/value pairs).
    Show,
    /// Print one setting value.
    Get { key: String },
    /// Set a CLI setting value.
    Set { key: String, value: String },
    /// Reset a setting to its default.
    Reset { key: String },
    /// Print the on-disk path of the CLI settings file.
    Path,
    /// Print the IPC base URL the CLI is using.
    DaemonUrl,
}

// ─── alias ─────────────────────────────────────────────────────────────

#[derive(Subcommand, Debug, Clone)]
pub enum AliasCmd {
    /// List server names and module aliases.
    List,
    /// Add or replace a module alias.
    Add { alias: String, module: String },
    /// Remove a module alias.
    Remove { alias: String },
}

// ─── system ────────────────────────────────────────────────────────────

#[derive(Subcommand, Debug, Clone)]
pub enum SystemCmd {
    /// List daemon/GUI/locale component versions.
    Components,
}

// ─── Output helpers ────────────────────────────────────────────────────

/// Shared output context. Methods write either JSON or human text to stdout
/// according to the global `--json` flag.
pub struct Output {
    pub json: bool,
    pub quiet: bool,
}

impl Output {
    pub fn new(json: bool, quiet: bool) -> Self {
        Self { json, quiet }
    }

    /// Print a successful result.
    pub fn print_value(&self, value: &Value) {
        if self.json {
            println!("{}", serde_json::to_string_pretty(value).unwrap_or_else(|_| value.to_string()));
        } else {
            println!("{}", crate::oneshot::format::value_to_text(value));
        }
    }

    /// Print a one-line success message (for void operations).
    pub fn print_success(&self, msg: &str) {
        if self.json {
            println!(
                "{}",
                serde_json::to_string_pretty(&json!({ "ok": true, "message": msg }))
                    .unwrap_or_default()
            );
        } else if !self.quiet {
            println!("{}", msg);
        }
    }

    /// Print a structured error (does not exit).
    pub fn print_error(&self, msg: &str) {
        if self.json {
            eprintln!(
                "{}",
                serde_json::to_string_pretty(&json!({ "ok": false, "error": msg }))
                    .unwrap_or_default()
            );
        } else {
            eprintln!("error: {}", msg);
        }
    }
}

// ─── Error classification ──────────────────────────────────────────────

/// Map an `anyhow::Error` to one of the documented exit codes.
///
/// 분류 우선순위:
///   1. 데몬이 HTTP 응답을 돌려준 경우(메시지에 "server returned" 포함):
///      - 404 / "not found" → `EXIT_NOT_FOUND`
///      - 그 외(4xx/5xx) → `EXIT_ERR`
///      - transport-level 키워드(os error 10061 등)가 응답 **본문**에
///        포함돼 있어도 `EXIT_DAEMON_OFFLINE`로 오분류하지 않는다.
///   2. HTTP 응답 자체를 받지 못한 경우(transport 실패): connection refused /
///      timeout / DNS 실패 / 명시적 "daemon is offline" 메시지 → `EXIT_DAEMON_OFFLINE`.
///   3. 그 외 → `EXIT_ERR`.
pub fn classify_error(err: &anyhow::Error) -> i32 {
    let msg = format!("{}", err).to_lowercase();
    let chain = err
        .chain()
        .map(|c| format!("{}", c).to_lowercase())
        .collect::<Vec<_>>()
        .join(" | ");

    let combined = format!("{} | {}", msg, chain);

    // 1) 데몬이 HTTP 응답을 줬다면 transport 키워드가 본문에 섞여 있어도 무시.
    if combined.contains("server returned") {
        if combined.contains("returned 404") || combined.contains(" not found") {
            return EXIT_NOT_FOUND;
        }
        return EXIT_ERR;
    }

    // 2) HTTP 응답을 못 받은 경우만 transport 레벨 휴리스틱 적용.
    if combined.contains("connection refused")
        || combined.contains("connect failed")
        || combined.contains("error sending request")
        || combined.contains("tcp connect error")
        || combined.contains("dns error")
        || combined.contains("timed out")
        || combined.contains("os error 10061")  // ECONNREFUSED on Windows
        || combined.contains("daemon is offline")
        || combined.contains("daemon is not reachable")
        || combined.contains("not reachable")
    {
        return EXIT_DAEMON_OFFLINE;
    }
    if combined.contains("returned 404") || combined.contains(" not found") {
        return EXIT_NOT_FOUND;
    }
    EXIT_ERR
}

/// Friendly hint appended to errors when the daemon is unreachable.
pub fn daemon_offline_hint() -> &'static str {
    "Saba-Core daemon is not reachable.\n\
     Try `saba-chan-cli daemon start` first, or set --daemon to the correct URL."
}

// ─── Lookup helpers ────────────────────────────────────────────────────

/// Resolve a user-supplied identifier (UUID or name) to an instance id.
/// Returns `Err(anyhow!("not found"))` if neither matches.
pub async fn resolve_instance_id(client: &DaemonClient, query: &str) -> anyhow::Result<String> {
    let instances = client.list_instances().await?;
    // Direct id match.
    for inst in &instances {
        if inst.get("id").and_then(|v| v.as_str()) == Some(query) {
            return Ok(query.to_string());
        }
    }
    // Case-insensitive name match.
    let lower = query.to_lowercase();
    for inst in &instances {
        if inst.get("name").and_then(|v| v.as_str()).map(|n| n.to_lowercase())
            == Some(lower.clone())
        {
            return inst
                .get("id")
                .and_then(|v| v.as_str())
                .map(|s| s.to_string())
                .ok_or_else(|| anyhow::anyhow!("instance has no id field"));
        }
    }
    anyhow::bail!("instance not found: {}", query)
}

/// Same as `resolve_instance_id` but also returns the full instance Value.
pub async fn resolve_instance(client: &DaemonClient, query: &str) -> anyhow::Result<Value> {
    let id = resolve_instance_id(client, query).await?;
    client.get_instance(&id).await
}

// ─── Submodules ────────────────────────────────────────────────────────

pub mod format;
pub mod runner;

#[cfg(test)]
mod tests {
    use super::*;
    use clap::Parser;

    #[test]
    fn parses_bare_invocation() {
        let cli = Cli::try_parse_from(["saba-chan-cli"]).unwrap();
        assert!(cli.command.is_none());
    }

    #[test]
    fn parses_status_subcommand() {
        let cli = Cli::try_parse_from(["saba-chan-cli", "status"]).unwrap();
        assert!(matches!(cli.command, Some(Command::Status)));
    }

    #[test]
    fn parses_global_json_flag() {
        let cli = Cli::try_parse_from(["saba-chan-cli", "--json", "status"]).unwrap();
        assert!(cli.json);
    }

    #[test]
    fn parses_server_start() {
        let cli = Cli::try_parse_from(["saba-chan-cli", "server", "start", "mc-1"]).unwrap();
        match cli.command {
            Some(Command::Server(ServerCmd::Start { name })) => assert_eq!(name, "mc-1"),
            other => panic!("unexpected: {:?}", other),
        }
    }

    #[test]
    fn parses_server_exec_multi_word() {
        let cli = Cli::try_parse_from([
            "saba-chan-cli", "server", "exec", "mc-1", "say", "hello", "world",
        ])
        .unwrap();
        match cli.command {
            Some(Command::Server(ServerCmd::Exec { name, command })) => {
                assert_eq!(name, "mc-1");
                assert_eq!(command, vec!["say", "hello", "world"]);
            }
            other => panic!("unexpected: {:?}", other),
        }
    }

    #[test]
    fn rejects_unknown_subcommand() {
        let res = Cli::try_parse_from(["saba-chan-cli", "wat"]);
        assert!(res.is_err());
    }

    #[test]
    fn classify_error_detects_connection_refused() {
        let err = anyhow::anyhow!("error sending request: connection refused (os error 10061)");
        assert_eq!(classify_error(&err), EXIT_DAEMON_OFFLINE);
    }

    #[test]
    fn classify_error_detects_not_found() {
        let err = anyhow::anyhow!("Server returned 404 Not Found: instance missing");
        assert_eq!(classify_error(&err), EXIT_NOT_FOUND);
    }

    #[test]
    fn classify_error_default_is_generic() {
        let err = anyhow::anyhow!("Server returned 500: internal");
        assert_eq!(classify_error(&err), EXIT_ERR);
    }

    #[test]
    fn classify_error_does_not_misclassify_5xx_with_inner_transport_error() {
        // 회귀: 데몬은 정상이지만 응답 본문에 transport 레벨 키워드(os error 10061 등)가
        // 포함될 수 있다 (예: RCON이 게임 포트 연결에 실패한 케이스).
        // "Server returned"가 있으면 HTTP 응답을 받은 것이므로 EXIT_DAEMON_OFFLINE이
        // 아닌 EXIT_ERR로 분류돼야 한다.
        let err = anyhow::anyhow!(
            "Server returned 500 Internal Server Error: \
             {{\"error\":\"RCON execution failed: Connection failed: \
             Failed to connect to 127.0.0.1:25575: ... os error 10061\"}}"
        );
        assert_eq!(classify_error(&err), EXIT_ERR);
    }

    #[test]
    fn classify_error_5xx_with_not_found_in_body_still_not_found() {
        // 데몬이 명시적으로 404를 돌려준 경우는 EXIT_NOT_FOUND.
        let err = anyhow::anyhow!("Server returned 404 Not Found: instance missing");
        assert_eq!(classify_error(&err), EXIT_NOT_FOUND);
    }

    #[test]
    fn classify_error_detects_offline_message() {
        // run_status가 직접 만드는 메시지도 EXIT_DAEMON_OFFLINE로 분류돼야 한다.
        let err = anyhow::anyhow!("Saba-Core daemon is offline (connection refused at http://127.0.0.1:9999)");
        assert_eq!(classify_error(&err), EXIT_DAEMON_OFFLINE);
    }

    #[test]
    fn classify_error_detects_not_reachable_hint() {
        let err = anyhow::anyhow!("the daemon is not reachable from cli");
        assert_eq!(classify_error(&err), EXIT_DAEMON_OFFLINE);
    }

    #[test]
    fn parses_shell_with_use() {
        let cli =
            Cli::try_parse_from(["saba-chan-cli", "shell", "--use", "mc-1"]).unwrap();
        match cli.command {
            Some(Command::Shell { use_instance }) => {
                assert_eq!(use_instance.as_deref(), Some("mc-1"));
            }
            other => panic!("unexpected: {:?}", other),
        }
    }

    #[test]
    fn parses_server_console_follow() {
        let cli =
            Cli::try_parse_from(["saba-chan-cli", "server", "console", "mc-1", "-f"]).unwrap();
        match cli.command {
            Some(Command::Server(ServerCmd::Console { follow, name, .. })) => {
                assert!(follow);
                assert_eq!(name, "mc-1");
            }
            other => panic!("unexpected: {:?}", other),
        }
    }

    #[test]
    fn parses_server_create_with_module() {
        let cli = Cli::try_parse_from([
            "saba-chan-cli", "server", "create", "mc-1", "--module", "minecraft",
        ])
        .unwrap();
        match cli.command {
            Some(Command::Server(ServerCmd::Create { name, module, extra })) => {
                assert_eq!(name, "mc-1");
                assert_eq!(module, "minecraft");
                assert!(extra.is_none());
            }
            other => panic!("unexpected: {:?}", other),
        }
    }

    #[test]
    fn parses_server_install_default_version() {
        let cli =
            Cli::try_parse_from(["saba-chan-cli", "server", "install", "minecraft"]).unwrap();
        match cli.command {
            Some(Command::Server(ServerCmd::Install { module, game_version, .. })) => {
                assert_eq!(module, "minecraft");
                assert_eq!(game_version, "latest");
            }
            other => panic!("unexpected: {:?}", other),
        }
    }

    #[test]
    fn parses_server_install_explicit_version() {
        let cli = Cli::try_parse_from([
            "saba-chan-cli", "server", "install", "minecraft", "--game-version", "1.21.4",
        ])
        .unwrap();
        match cli.command {
            Some(Command::Server(ServerCmd::Install { game_version, .. })) => {
                assert_eq!(game_version, "1.21.4");
            }
            other => panic!("unexpected: {:?}", other),
        }
    }

    #[test]
    fn parses_server_set_pairs() {
        let cli = Cli::try_parse_from([
            "saba-chan-cli", "server", "set", "mc-1", "port=25565", "max_players=20",
        ])
        .unwrap();
        match cli.command {
            Some(Command::Server(ServerCmd::Set { name, pairs, body })) => {
                assert_eq!(name, "mc-1");
                assert_eq!(pairs, vec!["port=25565", "max_players=20"]);
                assert!(body.is_none());
            }
            other => panic!("unexpected: {:?}", other),
        }
    }

    #[test]
    fn parses_server_set_body_object() {
        let cli = Cli::try_parse_from([
            "saba-chan-cli", "server", "set", "mc-1",
            "--body", r#"{"port":25577}"#,
        ])
        .unwrap();
        match cli.command {
            Some(Command::Server(ServerCmd::Set { name, pairs, body })) => {
                assert_eq!(name, "mc-1");
                assert!(pairs.is_empty());
                assert_eq!(body.as_deref(), Some(r#"{"port":25577}"#));
            }
            other => panic!("unexpected: {:?}", other),
        }
    }

    #[test]
    fn parses_server_reorder() {
        let cli = Cli::try_parse_from([
            "saba-chan-cli", "server", "reorder", "mc-2", "mc-1",
        ])
        .unwrap();
        match cli.command {
            Some(Command::Server(ServerCmd::Reorder { order })) => {
                assert_eq!(order, vec!["mc-2", "mc-1"]);
            }
            other => panic!("unexpected: {:?}", other),
        }
    }

    #[test]
    fn parses_server_properties_write() {
        let cli = Cli::try_parse_from([
            "saba-chan-cli", "server", "properties-write", "mc-1", "max-players=20",
        ])
        .unwrap();
        match cli.command {
            Some(Command::Server(ServerCmd::PropertiesWrite { name, pairs, .. })) => {
                assert_eq!(name, "mc-1");
                assert_eq!(pairs, vec!["max-players=20"]);
            }
            other => panic!("unexpected: {:?}", other),
        }
    }

    /// 업데이터가 CLI를 재실행할 때 `--after-update`를 덧붙인다. clap이 거절하면 exit 3.
    #[test]
    fn accepts_hidden_after_update_flag() {
        let cli = Cli::try_parse_from(["saba-chan-cli", "--after-update"]).unwrap();
        assert!(cli.command.is_none());
        let cli = Cli::try_parse_from(["saba-chan-cli", "status", "--after-update"]).unwrap();
        assert!(matches!(cli.command, Some(Command::Status)));
    }

    #[test]
    fn parses_system_components() {
        let cli =
            Cli::try_parse_from(["saba-chan-cli", "system", "components"]).unwrap();
        assert!(matches!(cli.command, Some(Command::System(SystemCmd::Components))));
    }

    #[test]
    fn parses_daemon_console_follow_with_interval() {
        let cli = Cli::try_parse_from([
            "saba-chan-cli",
            "daemon",
            "console",
            "--follow",
            "--interval",
            "500",
        ])
        .unwrap();
        match cli.command {
            Some(Command::Daemon(DaemonCmd::Console { follow, interval, .. })) => {
                assert!(follow);
                assert_eq!(interval, 500);
            }
            other => panic!("unexpected: {:?}", other),
        }
    }
}
