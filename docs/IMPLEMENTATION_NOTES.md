# Implementation Notes

## Paused: Palworld REST/RCON tasks
- Enable `RESTAPIEnabled=True` in Palworld server config; set Basic Auth credentials.
- Store REST settings per instance (rest_host/port/username/password) in GUI; ensure `PalServer.exe` path is correct (current instance uses `Palworld.exe`, likely wrong).
- Core daemon: prefer REST (announce/info/players/metrics/kick/ban/unban), RCON fallback already wired. Verify with a running server once REST is enabled.
- Electron picks `target/debug/core_daemon.exe` now; release binary was renamed to `core_daemon.exe.bak` after being locked. Rebuild release later and swap back if needed.

## Discord bot gaps
- Slash commands are only stubbed; no registration/deployment to Discord. Need a deploy script or REST registration.
- `interactionCreate` assumes `/server <subcommand>` maps directly to `GET /api/server/{subcommand}`, but IPC routes are:
  - `GET /api/servers` (list)
  - `POST /api/server/:name/start`
  - `POST /api/server/:name/stop`
  - `GET /api/server/:name/status`
  Update axios calls and add required parameters (server name, module, etc.).
- Missing intents for message content/guilds/guild messages as needed; currently only Guilds & MessageContent.
- No error/log handling for IPC failures or timeouts; add user-friendly replies.
- Add `IPC_BASE`/`DISCORD_TOKEN` validation on startup; warn if missing.

## Immediate blockers to start servers
- Verify instance executable path points to Palworld dedicated server binary (e.g., `.../PalServer.exe`).
- Ensure `core_daemon` not already running/locking release binary before rebuilds.
