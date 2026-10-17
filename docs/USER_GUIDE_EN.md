# Saba-chan (サバちゃん / 사바쨩) — Official User Guide

> **Modular Game Server Management Platform** — Manage multiple game servers from a unified GUI/CLI

> 💡 **Easier-to-read HTML version**: [guide.html](./guide.html) · [Landing page](./index.html)
>
> 🇰🇷 **한국어 원본**: [USER_GUIDE.md](./USER_GUIDE.md) (canonical reference)

---

## Table of Contents

1. [Introduction](#1-introduction)
2. [System Requirements](#2-system-requirements)
3. [Installation](#3-installation)
   - 3.1 [Installer-based Setup](#31-installer-based-setup)
   - 3.2 [Building from Source](#32-building-from-source)
4. [Quick Start](#4-quick-start)
5. [Architecture Overview](#5-architecture-overview)
6. [GUI Usage](#6-gui-usage)
7. [CLI Usage](#7-cli-usage)
8. [Instance Management](#8-instance-management)
9. [Module System](#9-module-system)
10. [Extension System](#10-extension-system)
11. [Discord Bot (Cloud Relay)](#11-discord-bot-cloud-relay)
12. [Configuration Reference](#12-configuration-reference)
13. [Update System](#13-update-system)
14. [Relay Server (Cloud Mode)](#14-relay-server-cloud-mode)
15. [REST API Reference](#15-rest-api-reference)
16. [Internationalization (i18n)](#16-internationalization-i18n)
17. [Filesystem Paths](#17-filesystem-paths)
18. [Troubleshooting](#18-troubleshooting)
19. [FAQ](#19-faq)

---

## 1. Introduction

**Saba-chan** (サバちゃん / 사바쨩) is a **modular game-server management platform** that lets you manage multiple game servers from a single interface.

> Origin of the name: a play on words — in Japanese, "server" (サーバー) and "mackerel" (鯖, さば) sound similar.

### Key Features

- **Multi-game support** — Palworld, Minecraft, Project Zomboid, and infinitely extensible via modules
- **Auto process detection** — automatically discovers running game servers
- **Module system** — add new games without recompiling Core
- **Dual interface** — Electron GUI desktop app + Rust TUI CLI
- **Resilient design** — daemon crashes do not affect running game servers
- **Auto updates** — built-in update system based on GitHub Releases
- **Discord bot integration** — manage servers directly from Discord
- **Extension system** — Docker isolation, SteamCMD automation, and other general-purpose features
- **10-language support** — Korean, English, Japanese, German, Spanish, French, Portuguese (BR), Russian, Chinese (Simplified/Traditional)

---

## 2. System Requirements

| Item | Requirement |
|------|-------------|
| **OS** | Windows 10 / 11 |
| **Rust** | latest stable (only when building from source) |
| **Node.js** | 18.0+ (GUI and Discord bot) |
| **Python** | 3.x (for module lifecycle execution) |
| **Disk** | 500MB minimum (excluding game servers) |
| **Network** | game-server ports must be open |

> **Note**: Rust is **not** required if you use the installer. Python and Node.js portable runtimes are bootstrapped automatically by Saba-chan.

---

## 3. Installation

### 3.1 Installer-based Setup

Saba-chan ships with a standalone installer (Tauri-based).

1. Download the latest installer from [GitHub Releases](https://github.com/WareAoba/saba-chan/releases).
2. Run the installer and choose an install path.
3. Launch Saba-chan from the desktop shortcut or Start menu.

### 3.2 Building from Source

#### Clone the repository

```bash
git clone https://github.com/WareAoba/saba-chan.git
cd saba-chan
```

#### Full build (Windows PowerShell)

```powershell
.\scripts\build-windows.ps1
```

This runs three build jobs **in parallel**:

1. **Rust workspace** — `cargo build --release --workspace`
   - Outputs: `saba-core.exe`, `saba-chan-cli.exe`, `saba-chan-updater.exe`, `saba-chan-installer.exe`
2. **Electron GUI** — `npm run build` (Vite) → `npm run package` (electron-builder)
   - Output: `Saba-chan.exe`
3. **Discord Bot** — `npm install --omit=dev` → directory copy

#### Individual builds

```bash
# Build Core daemon only
cargo build --release

# Run GUI in dev mode
cd saba-chan-gui
npm install
npm start

# Build CLI only
cargo build --release -p saba-chan-cli
```

---

## 4. Quick Start

### Step 1: Launch Saba-chan

Run the GUI (`Saba-chan.exe`) or CLI (`saba-chan-cli.exe`). The Core daemon starts automatically.

### Step 2: Create an Instance

1. Click **"+ Add Server"** in the GUI, or run `:instance create` in the CLI.
2. Select the game you want to manage (e.g. Palworld, Minecraft).
3. Enter an instance name and the server-executable path.

### Step 3: Configure the Server

- Set the executable (`executable_path`) and the working directory (`working_dir`).
- Optionally, set ports and RCON passwords.

### Step 4: Start the Server

Click the **Start (▶)** button in the GUI or trigger Start from the CLI.

### Step 5: Monitor

- Real-time server status
- Console output and command input
- Optional remote control through the Discord bot

---

## 5. Architecture Overview

### 5.1 Layout

```
┌─────────────────┐     HTTP API      ┌──────────────────┐
│  Electron GUI   │ ◄───────────────► │   Core Daemon    │
│  (React 18)     │   127.0.0.1:57474 │   (Rust/Axum)    │
└─────────────────┘                   └────────┬─────────┘
                                               │
┌─────────────────┐     HTTP API               │
│   CLI (TUI)     │ ◄─────────────────────────►│
│  (Ratatui)      │                            │
└─────────────────┘                  ┌─────────┴──────────┐
                                     │                    │
┌─────────────────┐           ┌──────▼──────┐     ┌──────▼──────┐
│  Discord Bot    │           │   Modules   │     │  Instances  │
│  (discord.js)   │           │ (per-game)  │     │  (configs)  │
└────────┬────────┘           └─────────────┘     └─────────────┘
         │                           │
         │ IPC                ┌──────▼──────┐
         └──────────────────► │ Extensions  │
                              │ (Docker etc)│
                              └─────────────┘
```

### 5.2 Core Daemon

`saba-core.exe` runs in the background and provides every server-management feature.

| Internal module | Role |
|-----------------|------|
| `supervisor/` | process management (start/stop/monitor), module loader, state machine |
| `ipc/` | Axum HTTP API server (port 57474) |
| `instance/` | directory-based instance store |
| `plugin/` | Python lifecycle module runner |
| `protocol/` | RCON (Valve Source) + REST API client |
| `extension/` | extension system (manifest parsing, hook dispatch) |
| `config/` | global settings |
| `validator/` | settings validation, port-conflict detection |
| `python_env/` | automatic Python venv bootstrap |
| `node_env/` | automatic Node.js portable runtime bootstrap |

### 5.3 Communication

#### Daemon ↔ GUI/CLI

- Protocol: HTTP REST API
- Default address: `http://127.0.0.1:57474`
- Auth: token from `.ipc_token` (`X-Saba-Token` header)

#### Daemon ↔ Modules

1. Daemon spawns `lifecycle.py <function_name>` as a child process
2. Config JSON is fed via **stdin**
3. Module writes the result as JSON to **stdout**
4. Logs go to **stderr**

#### Client Watchdog

- Each GUI/CLI registers via `POST /api/client/register` and sends a heartbeat every 30 seconds.
- When all clients disconnect → 15-second grace period → reconnect attempts → daemon shuts itself down if no client returns within 60 seconds.

| Constant | Value | Description |
|----------|-------|-------------|
| `DEFAULT_IPC_PORT` | 57474 | Default IPC server port |
| `MONITOR_INTERVAL_SECS` | 2 | Process monitoring period |
| `HEARTBEAT_REAPER_INTERVAL_SECS` | 30 | Client liveness check |
| `STOP_COOLDOWN_SECS` | 30 | Suppress auto-detect after Stop |

---

## 6. GUI Usage

The Saba-chan GUI is a desktop app built with **Electron 28 + React 18 + Vite + Zustand**.

### 6.1 Main Screen

When the GUI starts, a loading screen waits until the Core daemon connects, then the **dashboard** appears with all registered server instances rendered as **cards**. Each card shows:

- Server name and game type
- Current status (Running / Stopped / Error)
- Start/Stop buttons
- Extension badges (Docker, etc.)

### 6.2 Instance Management

#### Create a new instance

1. Click **"+ Add Server"** on the dashboard
2. **Select a game**: pick from installed modules
3. **Enter settings**: instance name, executable path, ports, etc.
4. **Done**: the new card appears on the dashboard

#### Server control

- **Start**: ▶ button on the card
- **Stop**: ■ button on the card
- **Settings**: click the card → settings modal
- **Delete**: context menu → Delete

#### Editing settings

Clicking a card opens the **Settings modal**. Each module defines its own setting fields, and the UI matches the field type:

| Field type | UI |
|------------|-----|
| `text` | text input |
| `number` | numeric input (min/max/step) |
| `boolean` | toggle switch |
| `select` | dropdown |
| `password` | masked input |
| `file` | file picker |
| `folder` | folder picker |

Fields are grouped:

- **basic** (empty string) — general
- **saba-chan** — Saba-chan-only options
- **advanced** — collapsed by default

### 6.3 Server Console

The **console view** displays real-time stdout/stderr and lets you send commands directly.

- **ConsoleView**: bottom dock panel on the dashboard
- **ConsoleWindow**: detachable popup
- Per-game syntax highlighting via the module `syntax_highlight` field

### 6.4 Settings

The settings modal (gear icon) controls global preferences:

| Tab | Items |
|-----|-------|
| **General** | language, auto-refresh, refresh interval |
| **Appearance** | UI theme |
| **Advanced** | IPC port, console buffer, password auto-generation, port conflict check |
| **Extensions** | manage installed extensions |

---

## 7. CLI Usage

The CLI (`saba-chan-cli.exe`) is an interactive TUI built on **Ratatui + Crossterm**.

### 7.1 TUI Screens

| Screen | Purpose |
|--------|---------|
| **Dashboard** | main view — daemon/bot status, server summary |
| **Servers** | instance list |
| **ServerDetail** | per-instance — status, start/stop/restart/delete |
| **ServerConsole** | live stdin/stdout |
| **ServerSettings** | vim-like settings editor |
| **ServerProperties** | edit server properties files |
| **Modules** | installed modules |
| **ModuleDetail** | module details |
| **ModuleManifest** | remote module manifest (installable modules) |
| **Bot** | Discord bot management |
| **BotAliases** | edit bot aliases |
| **Settings** | CLI/GUI settings |
| **Updates** | update management |
| **Daemon** | daemon controls |
| **Extensions** | extensions |
| **CreateInstance Step 1/2** | new-instance wizard |

### 7.2 Input Modes

| Mode | Description | Enter/Exit |
|------|-------------|-----------|
| **Normal** | menu navigation | default |
| **Command** | legacy command input | press `:` |
| **Editing** | vim-like field editing | Enter on a settings field |
| **Console** | server stdin input | auto inside console screen |
| **Confirm** | y/n dialog | y / n |
| **InlineInput / InlineSelect** | inline prompts | context-dependent |

#### Default Key Bindings

| Key | Action |
|-----|--------|
| `↑` / `k` | move up |
| `↓` / `j` | move down |
| `Enter` | select / confirm |
| `Esc` | back / cancel |
| `:` | enter command mode |
| `q` | quit |

### 7.3 Legacy Command Mode

Press `:` to enter command mode for direct text input.

```
: <command> [subcommand] [args...]
```

### 7.4 CLI Command Reference

#### Instances

| Command | Description |
|---------|-------------|
| `instance list` | list instances |
| `instance create` | new instance wizard |
| `instance delete <id>` | delete instance |
| `instance set <id> <key> <value>` | change setting |
| `instance reset <id>` | reset settings |
| `instance reorder` | reorder instances |

#### Modules

| Command | Description |
|---------|-------------|
| `module list` | list installed modules |
| `module info <name>` | module details |
| `module refresh` | reload modules |
| `module versions <name>` | list versions |
| `module install <name>` | install server |
| `module manifest` | view remote manifest |
| `module install-manifest <id>` | install module from manifest |
| `module remove <name>` | remove module |

#### Extensions

| Command | Description |
|---------|-------------|
| `extension list` / `ext list` | list extensions |
| `extension enable <id>` | enable |
| `extension disable <id>` | disable |
| `extension install <id>` | install |
| `extension remove <id>` | remove |
| `extension manifest` | view remote manifest |
| `extension rescan` | rescan |

#### Daemon

| Command | Description |
|---------|-------------|
| `daemon start` | start daemon |
| `daemon stop` | stop daemon |
| `daemon status` | daemon status |
| `daemon restart` | restart daemon |

#### Discord Bot

| Command | Description |
|---------|-------------|
| `bot start` | start bot |
| `bot stop` | stop bot |
| `bot status` | bot status |
| `bot token <token>` | set bot token |
| `bot prefix <prefix>` | set bot prefix |
| `bot mode <local\|cloud>` | switch mode |
| `bot relay <url>` | set relay URL |
| `bot node-token <token>` | set relay node token |

#### Settings

| Command | Description |
|---------|-------------|
| `config show` | show current settings |
| `config set <key> <value>` | change CLI setting |
| `config get <key>` | read setting |
| `config reset` | reset settings |
| `config system-language` | detect system language |

##### Modify GUI settings from CLI

| Command | Description |
|---------|-------------|
| `config gui language <lang>` | GUI language |
| `config gui auto_refresh <bool>` | auto-refresh |
| `config gui refresh_interval <ms>` | refresh interval |
| `config gui ipc_port <port>` | IPC port |
| `config gui console_buffer <n>` | console buffer size |
| `config gui auto_passwords <bool>` | password auto-generation |
| `config gui port_check <bool>` | port-conflict check |

#### Updates

| Command | Description |
|---------|-------------|
| `update check` | check updates |
| `update status` | update status |
| `update download` | download updates |
| `update apply` | apply updates |
| `update config` | view updater config |
| `update set <key> <value>` | change updater config |

#### Misc

| Command | Description |
|---------|-------------|
| `help` | show help |
| `menu` / `dashboard` | go to dashboard |
| `back` | previous screen |
| `exit` / `quit` / `q` | quit (also stops bot/daemon) |
| `<module> <command>` | run a module command directly |

---

## 8. Instance Management

### 8.1 What is an instance?

An **instance** is one game-server configuration unit. You may create many instances of the same game, each with its own settings.

Instance data is stored as a directory tree:

```
instances/
├── order.json                  ← instance ordering
├── <uuid>/
│   ├── instance.json           ← metadata (id, name, module, ports…)
│   └── settings.json           ← per-module dynamic settings
└── <uuid>/
    ├── instance.json
    └── settings.json
```

### 8.2 Creating an instance

#### From the GUI

1. Click **"+ Add Server"**
2. Select a game (from installed modules)
3. Enter required settings:
   - **Instance name**
   - **Executable path** (`executable_path`)
   - **Working directory** (`working_dir`)
4. Optional:
   - port number
   - RCON/REST password (auto-generated if left blank)
   - per-game settings

#### From the CLI

```
:instance create
```

Launches the create wizard (Step 1 → Step 2).

### 8.3 Instance configuration

Each instance has two config files:

- **instance.json** — instance metadata (name, module, ports, protocols)
- **settings.json** — dynamic settings defined by the module (executable path, per-game options, etc.)

Setting keys are declared in the module's `module.toml` under `[[settings.fields]]`.

### 8.4 Start / Stop / Restart

#### Start flow

1. Load instance settings
2. Call module `get_launch_command()` → produce the start command
3. Spawn `ManagedProcess` with stdin/stdout capture
4. Begin process monitoring (every 2 seconds)

#### Stop flow

1. Call module `stop_server()` (uses RCON / REST / stdin per protocol)
2. Confirm process termination
3. Suppress auto-detect for 30 seconds

### 8.5 Server-properties editor

Some modules expose direct editors for server-property files (e.g. `server.properties`).

- GUI: **Server Properties** tab
- CLI: **ServerProperties** screen

### 8.6 Sending commands to a server

While the server is running you can send commands directly:

| Method | Description |
|--------|-------------|
| **Console stdin** | type into the console screen |
| **RCON** | Valve Source RCON |
| **REST API** | HTTP REST (Palworld, etc.) |

Available methods depend on the module's supported protocols.

---

## 9. Module System

### 9.1 What is a module?

A module is a **Python script + TOML manifest** that manages the lifecycle of a specific game server. The Core daemon loads modules dynamically, so adding a new game does not require recompiling Core.

### 9.2 Supported games

| Game | Module | Protocol | Interaction | Install method | Default port |
|------|--------|----------|-------------|----------------|--------------|
| **Palworld** | `palworld` | REST API | `commands` | SteamCMD (App ID 2394010) | 8211 |
| **Minecraft** | `minecraft` | RCON + stdin | `console` | official download (mojang.com) | 25565 |
| **Project Zomboid** | `zomboid` | RCON + stdin | `console` | SteamCMD (App ID 380870) | 16261 |

> **Note**: install community/custom modules to support more games.

### 9.3 Installing & managing modules

#### Install from the remote manifest

```
:module manifest             # list available modules
:module install-manifest <id> # download + install a module
```

In the GUI, use the **Modules** tab → **Manifest** button.

#### Refresh modules

```
:module refresh
```

#### Remove a module

```
:module remove <name>
```

### 9.4 Module structure

```
mygame/
├── module.toml          # required — metadata + settings schema
├── lifecycle.py         # required — Python lifecycle script
├── icon.png             # recommended — icon shown in GUI
└── locales/             # recommended — translations
    ├── en.json          #   required (default)
    ├── ko.json
    ├── ja.json
    └── ...
```

#### `module.toml` sections

| Section | Description |
|---------|-------------|
| `[module]` | metadata — name, version, description, game_name, display_name, entry, icon |
| `[update]` | auto-update — github_repo |
| `[protocols]` | comm protocols — supported, default, interaction_mode |
| `[credential_map]` | credential sync — daemon key ↔ game key |
| `[config]` | execution defaults — executable_path, process_name, default_port, stop_command |
| `[install]` | server install — method (steamcmd/download/manual), app_id, download_url |
| `[docker]` | Docker — image, ports, volumes, env |
| `[detection]` | auto-detect — process_patterns, cmd_patterns, common_paths |
| `[settings]` | GUI/CLI settings field schema |
| `[commands]` | server commands |
| `[errors]` | error messages |
| `[aliases]` | aliases |
| `[syntax_highlight]` | console syntax highlighting |

#### `lifecycle.py` functions

| Function | Required | Description |
|----------|----------|-------------|
| `get_launch_command(config)` | ✅ | return start command |
| `get_status(config)` | ✅ | query status |
| `stop_server(config)` | ✅ | stop server |
| `execute_command(config)` |    | run server command |
| `get_available_versions(config)` |    | list versions |
| `install_server(config)` |    | install server |
| `read_settings(config)` |    | read server config files |
| `write_settings(config)` |    | write server config files |
| `diagnose(config)` |    | diagnostics |

> **Module development details**: see [the module-development guide](../saba-chan-modules/docs/module-development-guide.md).

---

## 10. Extension System

### 10.1 What is an extension?

Unlike modules, extensions provide **general-purpose functionality** that is not tied to a specific game. They can hook into many points of the server lifecycle and inject UI slots into the GUI/CLI.

### 10.2 Built-in extensions

| ID | Name | Description | GUI |
|----|------|-------------|-----|
| `docker` | Docker Isolation | run servers inside Docker containers | ✅ |
| `steamcmd` | SteamCMD | automate SteamCMD-based install/update | ❌ |
| `music` | Music Bot | Discord voice-channel music (yt-dlp + ffmpeg) | ❌ |
| `ue4-ini` | UE4 INI Parser | parse Unreal Engine 4 OptionSettings INI | ❌ |

### 10.3 Managing extensions

#### Enable / disable

Toggle in **GUI Settings → Extensions**, or run:

```
:ext enable <id>
:ext disable <id>
```

#### Install / remove

```
:ext manifest              # remote manifest
:ext install <id>          # install
:ext remove <id>           # remove
:ext rescan                # rescan extensions directory
```

### Hook points

Extensions can register for these hooks:

#### Daemon lifecycle

| Hook | When |
|------|------|
| `daemon.startup` | on daemon start |
| `daemon.shutdown` | on daemon stop |

#### Server lifecycle

| Hook | When |
|------|------|
| `server.pre_create` | before instance creation |
| `server.post_create` | after instance creation |
| `server.pre_start` | before server start |
| `server.post_stop` | after server stop |
| `server.pre_delete` | before instance deletion |
| `server.status` | on status query |
| `server.stats` | on stats query |
| `server.settings_changed` | on settings change |
| `server.list_enrich` | enrich server list info |
| `server.logs` | on log query |
| `server.install` | on server install |
| `server.update` | on server update |
| `server.check_update` | on update check |

### GUI/CLI slots

| GUI slot | CLI slot | Purpose |
|----------|----------|---------|
| `ServerCard.badge` | `InstanceList.badge` | server-list badges |
| `ServerCard.headerGauge` | — | header gauges |
| `ServerCard.expandedStats` | `InstanceDetail.status` | extended stats |
| `ServerCard.provision` | — | provisioning state |
| `ServerSettings.tab` | `InstanceSettings.fields` | settings tabs/fields |
| `AddServer.options` | `CreateInstance.options` | new-instance options |

---


## 11. Discord Bot (Cloud Relay)

The Discord bot is now hosted in the cloud (relay-server). Pair your instance via **Settings → Cloud → Generate Pairing Token**, then visit `https://saba-chan.app` to add the bot to your server.

For details on pairing, slash commands, prefix commands, aliases, and music features, see section [14. Relay Server (Cloud Mode)](#14-relay-server-cloud-mode).

---

## 12. Configuration Reference

### 12.1 Global config (`global.toml`)

Location: `config/global.toml`

```toml
ipc_socket = "./ipc.sock"
# log_buffer_size = 10000

[updater]
enabled = true
check_interval_hours = 3
auto_download = false
auto_apply = false
relay_base_url = "https://saba-chan.online"
channel = "stable"             # "stable" | "beta" | "nightly"
platform = "windows-x64"
# install_root = "."
```

### 12.2 GUI settings (`settings.json`)

Location: `%APPDATA%/saba-chan/settings.json`

| Key | Type | Default | Description |
|-----|------|---------|-------------|
| `language` | string | `"en"` | display language |
| `autoRefresh` | bool | `true` | auto status refresh |
| `refreshInterval` | number | `2000` | refresh interval (ms) |
| `ipcPort` | number | `57474` | IPC port (1024–65535) |
| `consoleBuffer` | number | `2000` | console buffer size |
| `autoGeneratePasswords` | bool | `true` | auto-generate empty RCON/REST passwords |
| `portConflictCheck` | bool | `true` | enable port-conflict check |

### 12.3 CLI settings (`cli-settings.json`)

Location: `%APPDATA%/saba-chan/cli-settings.json`

| Key | Alias | Type | Default | Description |
|-----|-------|------|---------|-------------|
| `language` | `lang` | string | `""` | display language (defaults to GUI setting if empty) |
| `auto_start` | `autostart` | bool | `true` | auto-start daemon when TUI launches |
| `refresh_interval` | `refresh` | u64 | `2` | refresh period (seconds, 1–60) |
| `bot_prefix` | `prefix` | string | `""` | override Discord bot prefix |

### 12.4 Bot config (`bot-config.json`)

Location: `%APPDATA%/saba-chan/bot-config.json`

```json
{
  "prefix": "사바쨩",
  "moduleAliases": {},
  "commandAliases": {},
  "musicEnabled": true,
  "musicChannelId": "",
  "musicUISettings": {
    "queueLines": 10,
    "refreshInterval": 5
  },
  "mode": "local",
  "cloud": {
    "relayUrl": "https://saba-chan.online",
    "musicServerUrl": "",
    "hostId": ""
  },
  "nodeSettings": {}
}
```

| Key | Type | Description |
|-----|------|-------------|
| `prefix` | string | bot command prefix (default `"사바쨩"`) |
| `moduleAliases` | object | module alias map (`{ "팔": "palworld" }`) |
| `commandAliases` | object | command alias map (`{ "켜": "start" }`) |
| `musicEnabled` | bool | enable music extension |
| `musicChannelId` | string | text channel ID for music UI (empty disables UI) |
| `musicUISettings.queueLines` | number | queue lines in embed |
| `musicUISettings.refreshInterval` | number | embed refresh (seconds) |
| `mode` | string | operating mode (`"local"` / `"cloud"`) |
| `cloud.relayUrl` | string | relay server URL (default `https://saba-chan.online`) |
| `cloud.musicServerUrl` | string | remote music-server URL (empty = local engine) |
| `cloud.hostId` | string | node ID registered on the relay (auto-extracted from token) |
| `nodeSettings` | object | per-Discord-guild instance filter and settings |

### 12.5 Environment variables

These override settings:

#### Saba-chan core

| Variable | Description |
|----------|-------------|
| `SABA_IPC_PORT` | override IPC port |
| `SABA_DATA_DIR` | data root (default `%APPDATA%\saba-chan`) |
| `SABA_MODULES_PATH` | modules directory |
| `SABA_INSTANCES_PATH` | instances directory |
| `SABA_EXTENSIONS_DIR` | extensions directory |
| `SABA_TOKEN_PATH` | IPC token file |
| `SABA_LANG` | display language |

#### Discord bot

| Variable | Description |
|----------|-------------|
| `DISCORD_TOKEN` | Discord bot token |
| `IPC_BASE` | Core daemon base URL (default `http://127.0.0.1:57474`) |
| `BOT_CONFIG_PATH` | path to `bot-config.json` |
| `RELAY_URL` | relay server URL (forces cloud mode when set) |
| `RELAY_NODE_TOKEN` | relay node token (`sbn_<nodeId>.<secret>`) |
| `MUSIC_SERVER_URL` | remote music-server URL |

#### Music / media paths

| Variable | Description |
|----------|-------------|
| `YT_DLP_PATH` | yt-dlp binary path |
| `FFMPEG_PATH` | ffmpeg binary path |
| `YT_COOKIES_PATH` | YouTube cookie file (Netscape format) |

---

## 13. Update System

Saba-chan ships an integrated auto-updater backed by GitHub Releases.

### 13.1 Auto updates

Defaults baked into the build:

| Setting | Default | Behavior |
|---------|---------|----------|
| `enabled` | `true` | enable update checks |
| `check_interval_hours` | `3` | check every 3 hours |
| `auto_download` | `false` | auto-download new versions |
| `auto_apply` | `false` | auto-apply after download |

### 13.2 Manual updates

GUI **Updates** tab, or CLI:

```
:update check
:update download
:update apply
:update status
```

### Update components

| Component | Description |
|-----------|-------------|
| `saba-core` | Core daemon |
| `cli` | CLI |
| `gui` | GUI |
| `updater` | updater itself |
| `locales` | localization files |
| `module-{name}` | individual module |
| `ext-{name}` | individual extension |

### Update flow

1. Query the relay server (`/v1/updates/check`) for the latest signed manifest
2. Parse component version/hash/URL from the relay manifest
3. Compare with local versions in `installed-manifest.json`
4. Download updates from R2 (auto or manual)
5. Apply (modules/extensions swap immediately; core/CLI/GUI need a restart)

### 13.3 Integrity verification

Downloaded files are verified by **SHA256**.

| State | Description |
|-------|-------------|
| `Verified` | hash match — OK |
| `Tampered` | hash mismatch — possibly corrupted/tampered |
| `NoHash` | no hash to compare |
| `FileNotFound` | file missing |
| `Error` | verification error |

---

## 14. Relay Server (Cloud Mode)

The relay server (server-chan) bridges Saba-chan nodes behind NAT with the Discord bot. The official instance runs at `https://saba-chan.online`.

### Architecture

```
Discord user
    │
    ▼
Relay server (server-chan)    ← Fastify 5 + PostgreSQL 17
    │
    ▼ (long poll)
Local Saba-chan node          ← bot in relay-agent mode
    │
    ▼ (IPC)
Local Core daemon
    │
    ▼
Result → Relay server → Discord response
```

### Principles

- **The central server never inspects the payload** (it's a courier)
- **Pull-based**: nodes connect *to* the relay (no port forwarding)
- **No IP storage**: privacy first
- **Security**: argon2id token hashing + HMAC-SHA256 request signing

### Token format

```
sbn_<nodeId>.<secret>
```

- The `sbn_` prefix and the dot are required.
- `nodeId` is the public ID known to the relay server.
- `secret` is the shared secret used to sign requests with HMAC-SHA256.

### HMAC-SHA256 request signing

Every request a node sends to the relay is signed over:

```
<METHOD>\n<URL>\n<timestamp>\n<nonce>\n<body>
```

Headers:

- `X-Saba-Node-Id`
- `X-Saba-Timestamp` (Unix ms)
- `X-Saba-Nonce` (one-time random)
- `X-Saba-Signature` (the HMAC value)

### Fatal auth errors (FATAL_AUTH_ERRORS)

If any of the following errors occur, the bot stops without retrying:

| Error | Meaning |
|-------|---------|
| `NODE_NOT_FOUND` | node ID not registered on the relay |
| `INVALID_TOKEN` | token secret mismatch |
| `INVALID_TOKEN_FORMAT` | token does not match `sbn_<id>.<secret>` |
| `MISSING_TOKEN` | empty token |
| `INVALID_SIGNATURE` | HMAC mismatch (tampered or clock-drift) |

### Cloud mode setup

1. Configure relay URL and credentials:

   ```
   :bot mode cloud
   :bot relay https://saba-chan.online
   :bot node-token sbn_<nodeId>.<secret>
   ```

2. (Optional) use a remote music-server:

   ```
   :config set cloud.musicServerUrl https://saba-chan.online/music
   ```

3. Start the bot — it now operates as a relay agent.

---

## 15. REST API Reference

The Core daemon serves an HTTP REST API at `http://127.0.0.1:57474`.

### 15.1 Authentication

Every API call requires the `X-Saba-Token` header.

```
X-Saba-Token: <contents of .ipc_token>
```

Token file: `%APPDATA%/saba-chan/.ipc_token`

On a 401 response, clients re-read the file and retry once.

### 15.2 Server API (runtime)

| Method | Endpoint | Description |
|--------|----------|-------------|
| `GET` | `/api/servers` | runtime server list |
| `GET` | `/api/server/{name}/status` | server status |
| `POST` | `/api/server/{name}/start` | start server |
| `POST` | `/api/server/{name}/stop` | stop server |

### 15.3 Instance API (configuration)

| Method | Endpoint | Description |
|--------|----------|-------------|
| `GET` | `/api/instances` | list instances |
| `GET` | `/api/instance/{id}` | instance details |
| `POST` | `/api/instances` | create instance |
| `PATCH` | `/api/instance/{id}` | update instance |
| `DELETE` | `/api/instance/{id}` | delete instance |
| `PUT` | `/api/instances/reorder` | reorder |
| `POST` | `/api/instance/{id}/validate` | validate settings |
| `GET` | `/api/instance/{id}/properties` | read server properties |
| `PUT` | `/api/instance/{id}/properties` | write server properties |
| `POST` | `/api/instance/{id}/accept-eula` | accept EULA |
| `POST` | `/api/instance/{id}/diagnose` | diagnose |
| `POST` | `/api/instance/{id}/server/reset` | reset server |
| `POST` | `/api/instance/{id}/properties/reset` | reset properties |

### 15.4 Console API

| Method | Endpoint | Description |
|--------|----------|-------------|
| `POST` | `/api/instance/{id}/managed/start` | managed start (capture stdin/stdout) |
| `GET` | `/api/instance/{id}/console` | get console output |
| `POST` | `/api/instance/{id}/stdin` | send stdin text |

### 15.5 Module API

| Method | Endpoint | Description |
|--------|----------|-------------|
| `GET` | `/api/modules` | list modules |
| `GET` | `/api/module/{name}` | module details |
| `POST` | `/api/modules/refresh` | refresh modules |
| `GET` | `/api/module/{name}/versions` | versions |
| `POST` | `/api/module/{name}/install` | install server |
| `GET` | `/api/modules/manifest` | remote manifest |
| `POST` | `/api/modules/manifest/{id}/install` | install from manifest |
| `DELETE` | `/api/modules/{id}` | remove module |

### 15.6 Extension API

| Method | Endpoint | Description |
|--------|----------|-------------|
| `GET` | `/api/extensions` | list extensions |
| `POST` | `/api/extensions/{id}/enable` | enable |
| `POST` | `/api/extensions/{id}/disable` | disable |
| `POST` | `/api/extensions/{id}/install` | install |
| `DELETE` | `/api/extensions/{id}` | remove |
| `GET` | `/api/extensions/manifest` | remote manifest |
| `GET` | `/api/extensions/updates` | check updates |
| `POST` | `/api/extensions/rescan` | rescan |
| `GET` | `/api/extensions/init-status` | init status |

### 15.7 Update API

| Method | Endpoint | Description |
|--------|----------|-------------|
| `POST` | `/api/updates/check` | manual check |
| `GET` | `/api/updates/status` | update status |
| `POST` | `/api/updates/download` | download |
| `POST` | `/api/updates/apply` | apply |
| `GET` | `/api/updates/config` | view updater config |
| `PUT` | `/api/updates/config` | change updater config |
| `GET` | `/api/install/status` | install state |
| `POST` | `/api/install/run` | first-run install |
| `POST` | `/api/install/component/{key}` | install one component |
| `GET` | `/api/install/progress` | install progress |

### 15.8 Bot / Client API

| Method | Endpoint | Description |
|--------|----------|-------------|
| `POST` | `/api/client/register` | register client |
| `POST` | `/api/client/{id}/heartbeat` | heartbeat |
| `DELETE` | `/api/client/{id}/unregister` | unregister |
| `GET` | `/api/config/bot` | get bot config |
| `PUT` | `/api/config/bot` | save bot config |
| `GET` | `/api/provision-progress/{name}` | provision state |
| `DELETE` | `/api/provision-progress/{name}` | clear provision state |

### Sample API call (PowerShell)

```powershell
# List servers
$token = Get-Content "$env:APPDATA\saba-chan\.ipc_token"
$headers = @{ "X-Saba-Token" = $token }
Invoke-RestMethod -Uri "http://127.0.0.1:57474/api/servers" -Headers $headers | ConvertTo-Json

# Create an instance
$body = @{
    name = "My Palworld Server"
    module = "palworld"
} | ConvertTo-Json
Invoke-RestMethod -Method POST -Uri "http://127.0.0.1:57474/api/instances" -Headers $headers -Body $body -ContentType "application/json"
```

---

## 16. Internationalization (i18n)

Saba-chan ships with 10 languages:

| Code | Language | Code | Language |
|------|----------|------|----------|
| `en` | English | `fr` | Français |
| `ko` | 한국어 | `pt-BR` | Português (Brasil) |
| `ja` | 日本語 | `ru` | Русский |
| `de` | Deutsch | `zh-CN` | 简体中文 |
| `es` | Español | `zh-TW` | 繁體中文 |

### Translation file layout

```
locales/
├── en/
│   ├── gui.json         ← GUI translations
│   ├── cli.json         ← CLI translations
│   └── common.json      ← shared translations
├── ko/
│   ├── gui.json
│   ├── cli.json
│   └── common.json
└── ...
```

### Language resolution order

1. Per-app config (CLI: `cli-settings.json`, GUI: `settings.json`)
2. `SABA_LANG` environment variable
3. Default `"en"`

### Module / extension translations

- Modules: `modules/{module}/locales/{lang}.json`
- Extensions: `extensions/{ext}/i18n/{lang}.json`

Missing keys fall back to English automatically.

---

## 17. Filesystem Paths

Windows defaults (override the root with `SABA_DATA_DIR`):

### Configs / data

| Path | Purpose |
|------|---------|
| `%APPDATA%\saba-chan\` | data root |
| `%APPDATA%\saba-chan\settings.json` | GUI config |
| `%APPDATA%\saba-chan\cli-settings.json` | CLI config |
| `%APPDATA%\saba-chan\bot-config.json` | Discord bot config |
| `%APPDATA%\saba-chan\.ipc_token` | IPC auth token |
| `%APPDATA%\saba-chan\instances\` | instance store |
| `%APPDATA%\saba-chan\modules\` | modules |
| `%APPDATA%\saba-chan\extensions\` | extensions |
| `%APPDATA%\saba-chan\logs\daemon.log` | core daemon log |
| `config\global.toml` | global config (inside the app dir) |

### Music / misc

| Path | Purpose |
|------|---------|
| `~\.saba-cookies\cookies.txt` | YouTube cookies (user home) |
| `%TEMP%\saba-chan\` | download / swap staging |

---

## 18. Troubleshooting

### Cannot connect to daemon

**Symptom**: GUI is stuck at the loading screen, or CLI shows a connection error.

**Steps**:
1. Confirm `saba-core.exe` is running.
2. Check whether the IPC port (default 57474) is in use by another program.
3. Make sure the firewall is not blocking local connections.
4. Verify `.ipc_token` exists.

### Server fails to start

**Steps**:
1. Verify `executable_path` points to a real executable.
2. Verify `working_dir` is a valid directory.
3. Check the server console for error messages.
4. Run `:instance validate <id>` (or the GUI diagnose dialog) to detect port conflicts.

### Module does not load

**Steps**:
1. Verify the modules directory path (`SABA_MODULES_PATH`).
2. Confirm both `module.toml` and `lifecycle.py` exist inside the module folder.
3. Run `:module refresh`.
4. Confirm Python 3.x is available.

### Discord bot is silent

**Steps**:
1. Confirm the bot token is set correctly.
2. Confirm the bot is running (`:bot status`).
3. Confirm the bot is invited to the Discord server.
4. Confirm the bot has Read/Send Messages permissions.
5. Verify the prefix.

### Update fails

**Steps**:
1. Check internet connectivity.
2. Confirm GitHub API is reachable.
3. Use `:update status` to inspect current state.
4. Retry: `:update check` → `:update download` → `:update apply`.

### Music bot does not play

**Symptom**: `<prefix> play <URL>` produces no audio or only an error.

**Steps**:
1. Confirm the bot has voice connect/speak permissions.
2. Local engine: ensure `yt-dlp` and `ffmpeg` are on PATH or pointed to via `YT_DLP_PATH` / `FFMPEG_PATH`.
3. Age-restricted/private content: provide a cookie file (see §11.6).
4. Remote engine: confirm `cloud.musicServerUrl` is correct and the music-server responds.
5. Confirm `musicEnabled: true` in `bot-config.json`.

### Cloud (relay) mode authentication failed

**Symptom**: bot start logs one of `INVALID_TOKEN`, `INVALID_TOKEN_FORMAT`, `NODE_NOT_FOUND`, `INVALID_SIGNATURE` and stops.

**Steps**:
1. Confirm the token in `RELAY_NODE_TOKEN` or `bot-config.json` is `sbn_<nodeId>.<secret>`.
2. If the relay re-issued your node, refresh the token.
3. For `INVALID_SIGNATURE`, ensure the system clock is in sync (NTP).
4. Confirm `RELAY_URL` is correct (official: `https://saba-chan.online`).

---

## 19. FAQ

### Q: If Saba-chan crashes, do my game servers go down?

**A:** No. Game servers run as independent processes; even if Saba-chan (Core daemon) crashes the games keep running. When Saba-chan restarts, it auto-detects the running servers.

### Q: Can I run multiple servers of the same game?

**A:** Yes. You can create as many instances as you like, each with its own port/directory configuration.

### Q: How do I add a new game?

**A:** Develop a module or install a community module. See the [module-development guide](../saba-chan-modules/docs/module-development-guide.md). You do **not** need to recompile Core.

### Q: Can I run the GUI and CLI at the same time?

**A:** Yes. Both connect to the same Core daemon and stay in sync.

### Q: Can I manage servers remotely?

**A:** Yes — through the Discord bot. Even from environments behind NAT (no port forwarding), cloud mode (relay server) lets you control remotely.

### Q: Can I isolate game servers using Docker?

**A:** Yes. Enable the Docker extension, set `docker_enabled` for the instance, and define CPU/memory limits.

### Q: How do I change the language?

**A:** GUI: Settings → General → Language. CLI: `:config set language <lang_code>`. Supported codes: `en`, `ko`, `ja`, `de`, `es`, `fr`, `pt-BR`, `ru`, `zh-CN`, `zh-TW`.

---

<p align="center">
  <strong>Saba-chan</strong> — Made with ❤️ and 🐟
</p>

<p align="center">
  <a href="https://github.com/WareAoba/saba-chan">GitHub</a> ·
  <a href="https://github.com/WareAoba/saba-chan/issues">Issue Tracker</a> ·
  MIT License
</p>
