# Saba-CLI: ・護桷 ・罹ｲ・・・ｬ ・・ｹ・・・ｸ奓ｰ寬們擽・､

Saba-chan Core Daemon・・・懍牟﨑俾ｸｰ ・・復 ・・・復 CLI 增ｴ・ｼ・ｴ・ｸ孖ｸ・・笈・､. 
Windows, macOS, Linux・川・ ・呷攵﨑俾ｲ・・瀧徐﨑俯ｩｰ, **Electron GUI・ ・､・・甯護攵・・・ｵ・﨑ｩ・壱共**.

## 噫 ・・ｸ ・懍梠

### ・誤糖

```bash
cd cli
cargo build --release
```

・晧┳・・・肥擽・壱ｦｬ:
- **Windows**: `target/release/saba-cli.exe`
- **macOS/Linux**: `target/release/saba-cli`

### ・､嵂・

#### 1・鞘Ε ・嶹被・ REPL ・ｨ・・(・護棗) 識

嵓・｡懋ｷｸ・ｨ ・懍梠 ・・・尖徐・ｼ・・・嶹被・ ・ｨ・罹｡・・・桿﨑ｩ・壱共:

```bash
saba-cli
```

#### 2・鞘Ε ・ｨ・ｼ ・・ｹ・ｴ ・ｨ・・

孖ｹ・・・卓羅・・・倆哩﨑俾ｳ ・・｣醐鮒・壱共:

```bash
saba-cli server "甯ｰ・罷糖・罹ｲ・ status
saba-cli bot config prefix "!play"
saba-cli alias list
```

#### 3・鞘Ε JSON ・罹･

嵓・｡懋ｷｸ・俯ｰ・・尖株 ・尖徐嶹罷･ｼ ・・紛 JSON 嶸菩享・ｼ・・・罹･:

```bash
saba-cli --json server "甯ｰ・罷糖・罹ｲ・ status
saba-cli -j alias list
```

## 式 ・・ｹ・ｴ ・・・・・ｴ・・

### ・罹ｲ・・懍牟 `server <・罹ｲ・ｪ・・尖株 ID>`

・ｸ・､奓ｴ・､ ・ｴ・・愍・・・罹ｲ・･ｼ ・懍牟﨑ｩ・壱共 (`instances.json` ・ｸ・ｰ):

```bash
saba-cli server "甯ｰ・罷糖・罹ｲ・           # ・ｰ・ｸ・・ status ・罹･
saba-cli server "甯ｰ・罷糖・罹ｲ・ status    # ・・・ 嶹菩攤
saba-cli server "甯ｰ・罷糖・罹ｲ・ start     # ・懍梠
saba-cli server "甯ｰ・罷糖・罹ｲ・ stop      # ・卓ｧ
saba-cli server "甯ｰ・罷糖・罹ｲ・ restart   # ・ｬ・懍梠
saba-cli server "甯ｰ・罷糖・罹ｲ・ exec "・・ｹ"    # ・､・､奛 ・・ｹ ・､嵂・
saba-cli server "甯ｰ・罷糖・罹ｲ・ rcon "・・ｹ"    # RCON ・・ｹ ・､嵂・
saba-cli server "甯ｰ・罷糖・罹ｲ・ rest "・・ｹ"    # REST ・・ｹ ・､嵂・

# ・尖株 UUID・罹巡 ・・･
saba-cli server 68b29cef-e584-4bd0-91dc-771865e31e25 status
```

### ・ｨ・・・・ｬ `module`

```bash
saba-cli module list                      # ・､・俯頗 ・ｨ・・・ｩ・・
saba-cli module info <・ｨ・壱ｪ・             # ・ｨ・・・簿ｳｴ ・ｰ巐・
saba-cli module reload                    # ・ｨ・ ・ｨ・・・､・・・罹糖
saba-cli module path <・ｽ・・               # ・ｨ・・・罷駕・・ｬ ・､・・(TODO)
saba-cli module mount <・ｽ・・              # ・・・ｨ・・・溢垓孖ｸ (TODO)
saba-cli module unmount <・ｨ・壱ｪ・          # ・ｨ・・・ｸ・溢垓孖ｸ (TODO)
```

### ・ｰ・ｬ ・懍牟 `daemon`

```bash
saba-cli daemon status                    # ・ｰ・ｬ ・・・ 嶹菩攤
saba-cli daemon start                     # ・ｰ・ｬ ・懍梠 (TODO)
saba-cli daemon stop                      # ・ｰ・ｬ ・卓ｧ (TODO)
saba-cli daemon restart                   # ・ｰ・ｬ ・ｬ・懍梠 (TODO)
```

### Discord ・・・懍牟 `bot`

・・・・ｬ ・・・､・・(GUI 縺ｨ蜈ｱ譛・:

```bash
saba-cli bot status                       # ・・・・・ 嶹菩攤 (TODO)
saba-cli bot start                        # ・・・懍梠 (TODO)
saba-cli bot stop                         # ・・・卓ｧ (TODO)

saba-cli bot config prefix "!play"        # ・・ｹ 嵓・ｦｬ嵓ｽ・､ ・・ｽ
saba-cli bot config alias "甯ｰ" "palworld" # ・ｨ・・・・ｪ・・緋ｰ
saba-cli bot config remove-alias "甯ｰ"     # ・ｨ・・・・ｪ・・懋ｱｰ
saba-cli bot config show                  # 嶸・椪 ・・・､・・・ｰ巐・
```

### ・・ｪ・& ・､・・`alias`

```bash
saba-cli alias list                            # ・ｨ・ ・罹ｲ・・ｨ・・・・ｪ・・ｰ巐・
saba-cli alias module "甯ｰ" "palworld"         # ・ｨ・・・・ｪ・・緋ｰ
saba-cli alias remove-module "甯ｰ"             # ・ｨ・・・・ｪ・・懋ｱｰ
```

**・ｸ・**: ・罹ｲ・・・ｪ・捩 `instances.json`・・・罹ｲ・・ｴ・・愍・・・・ｬ・ｩ・壱共.

## 沈 甯護攵 ・・ｹ・・・嶸ｸ嶹們┳

### Windows
| 甯護攵 | ・・ｹ・| ・､・・|
|------|------|------|
| `bot-config.json` | `%APPDATA%\saba-chan\` | Discord ・・・､・・(GUI・ ・ｵ・) |
| `instances.json` | 嵓・｡懍晨敢 ・ｨ孖ｸ ・尖株 `config\` | ・罹ｲ・・ｸ・､奓ｴ・､ ・菩攪 |
| `saba-cli.json` | `%APPDATA%\saba-chan\` | CLI ・・圸 ・､・・|

### Linux/macOS
| 甯護攵 | ・・ｹ・| ・､・・|
|------|------|------|
| `bot-config.json` | `~/.config/saba-chan/` | Discord ・・・､・・(GUI・ ・ｵ・) |
| `instances.json` | 嵓・｡懍晨敢 ・ｨ孖ｸ ・尖株 `config/` | ・罹ｲ・・ｸ・､奓ｴ・､ ・菩攪 |
| `saba-cli.json` | `~/.config/saba-chan/` | CLI ・・圸 ・､・・|

### 甯護攵 嶸菩享

**bot-config.json** (Electron GUI・ ・呷攵):
```json
{
  "prefix": "!saba-cli",
  "moduleAliases": {
    "甯ｰ": "palworld",
    "・溢攤": "minecraft"
  },
  "commandAliases": {}
}
```

**instances.json**:
```json
[
  {
    "id": "68b29cef-e584-4bd0-91dc-771865e31e25",
    "name": "甯ｰ・罷糖・罹ｲ・,
    "module_name": "palworld",
    "executable_path": "D:\\PalServer\\PalServer.exe",
    "port": 8211,
    "rcon_port": 25575,
    "rcon_password": "xxxx",
    "protocol_mode": "rest"
  }
]
```

## 笞呻ｸ・・・溜 ・､・・

### Daemon URL ・・ｽ

・ｰ・ｸ・・ `http://127.0.0.1:57474`

**・・ｹ・・・ｵ・・**
```bash
saba-cli --daemon http://my-server:57474 server "甯ｰ・罷糖・罹ｲ・ status
```

**・､・・甯護攵:**
```bash
saba-cli config show                           # 嶸・椪 ・､・・・ｴ・ｰ
saba-cli config set key value                 # ・､・・・・ｽ
saba-cli config reset                         # ・ｰ・ｸ・廷愍・・・一ｸｰ嶹・
```

**saba-cli.json** (・尖徐 ・晧┳):
```json
{
  "daemon_url": "http://127.0.0.1:57474"
}
```

## 庁 ・嶹被・ ・ｨ・・(REPL)

```bash
saba-cli
```

孖ｹ・・
笨・**・､・懋ｰ・・ｨ・逸┣・・* - 2・壱ｧ壱共 ・・・ ・尖徐 ・ｱ・
笨・**・・ｸ ・・ｹ ・・･** - ・・ｹ・ｴ ・・｣・弡・・餓亨 ・・・ 嶹菩攤
笨・**・・ｹ 德溢侃奝・ｬ** - ・・・・椈 嶹肥ざ岺罹｡・・ｴ・・・・ｹ ・ｬ・ｬ・ｩ
笨・**・餓亨 ・ｰ・ｼ** - ・・ｹ ・､嵂・・ｰ・ｼ・ｼ ・罷｡・嶹菩攤

## 売 ・ｬ・ｩ ・溢亨

### ・罹ｲ・・懍梠/・卓ｧ

```bash
# 甯ｰ・罷糖 ・罹ｲ・・懍梠
saba-cli server "甯ｰ・罷糖・罹ｲ・ start

# ・溢攤增ｬ・倆売孖ｸ ・罹ｲ・・卓ｧ
saba-cli server "my-minecraft-1" stop

# ・罹ｲ・・ｬ・懍梠
saba-cli server "甯ｰ・罷糖・罹ｲ・ restart
```

### RCON ・・ｹ ・､嵂・

```bash
# 甯ｰ・罷糖 ・罹ｲ・乱 ・肥亨・ ・・・
saba-cli server "甯ｰ・罷糖・罹ｲ・ rcon "say Server restarts in 5 minutes!"
```

### ・・・､・・

```bash
# ・・・・ｹ 嵓・ｦｬ嵓ｽ・､・ｼ !play・・・・ｽ
saba-cli bot config prefix "!play"

# ・ｨ・・・・ｪ・・緋ｰ (・・擽 "!甯ｰ info" ・呷捩 ・晧愍・・・ｬ・ｩ ・・･)
saba-cli bot config alias "甯ｰ" "palworld"

# 嶸・椪 ・､・・嶹菩攤
saba-cli bot config show
```

## 制 Linux/嵭､・罹ｦｬ・､ ・罹ｲ・乱・・・ｬ・ｩ

GUI ・・株 ・ｬ・・侃 ・罹ｲ・乱・罹巡 ・・・梭 ・呷攵﨑俾ｲ・・瀧徐﨑ｩ・壱共:

```bash
# SSH・・・卓・
ssh user@server.com

# ・嶹被・ ・ｨ・罹｡・・・ｬ
saba-cli

# ・ｨ・ ・ｰ・･ ・ｬ・ｩ ・・･
saba-cli> server "甯ｰ・罷糖・罹ｲ・ start
saba-cli> module reload
saba-cli> bot config show
saba-cli> exit
```

## 搭 ・・ｹ・ｴ ・・ｸ ・逸詐・ｰ・､

| ・肥｣ｼ | ・・ｹ・ｴ | ・､・・|
|------|--------|------|
| **・罹ｲ・* | `server <name> [status\|start\|stop\|restart]` | ・罹ｲ・・懍牟 |
| | `server <name> [exec\|rcon\|rest] <cmd>` | ・・ｹ ・､嵂・|
| **・ｨ・・* | `module [list\|info\|reload]` | ・ｨ・・・・ｬ |
| **・・* | `bot [status\|start\|stop]` | ・・嵓・｡懍┷・､ |
| | `bot config [prefix\|alias\|show]` | ・・・､・・|
| **・・ｪ・* | `alias list` | ・ｨ・ ・・ｪ・・ｰ巐・|
| | `alias module <alias> <module>` | ・ｨ・・・・ｪ・・緋ｰ |
| **・､・・* | `config [show\|set\|reset]` | CLI ・､・・|
| **Health** | `health` | Daemon ・・・ |

## 肌 ・罹ｰ・

### 嵓・｡懍晨敢 ・ｬ・ｰ

```
saba-chan-cli/
笏懌楳笏 src/
笏・  笏懌楳笏 main.rs              # ・・桿・・ clap 甯護恭
笏・  笏懌楳笏 client.rs            # Daemon API HTTP 增ｴ・ｼ・ｴ・ｸ孖ｸ
笏・  笏懌楳笏 alias.rs             # bot-config.json & instances.json ・・ｬ
笏・  笏懌楳笏 commands/            # ・・・・ｹ・ｴ ・ｬ嶸・
笏・  笏・  笏懌楳笏 server.rs
笏・  笏・  笏懌楳笏 module.rs
笏・  笏・  笏懌楳笏 instance.rs
笏・  笏・  笏懌楳笏 exec.rs
笏・  笏・  笏披楳笏 config.rs
笏・  笏懌楳笏 interactive/         # REPL ・ｨ・・
笏・  笏・  笏懌楳笏 state.rs
笏・  笏・  笏懌楳笏 repl.rs
笏・  笏・  笏披楳笏 mod.rs
笏・  笏披楳笏 utils/
笏・      笏懌楳笏 table.rs
笏・      笏披楳笏 mod.rs
笏披楳笏 Cargo.toml
```

### ・們｡ｴ・ｱ

- **clap**: CLI 甯護恭
- **tokio**: ・・徐・ｰ ・ｰ夋・・
- **reqwest**: HTTP 增ｴ・ｼ・ｴ・ｸ孖ｸ
- **rustyline**: ・嶹被・ ・・･
- **serde_json**: JSON ・俯ｦｬ

## 統 ・ｼ・ｴ・・､

嵓・｡懍晨敢・・・ｼ・ｴ・・､・ｼ ・ｰ・・笈・､.
