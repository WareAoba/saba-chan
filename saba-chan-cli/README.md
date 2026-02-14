# saba-chan-cli: �E�임 �E�벁E�E��E� �E�E���E�E�E�������이�E�

Saba-chan Core Daemon�E�E�E�어���기 �E�E�� �E�E��E�� CLI ����E��E��E�����E�E���E�. 
Windows, macOS, Linux�E��E �E�일���겁E�E�동���며, **Electron GUI�E� �E��E�E���일�E�E�E��E�����E�다**.

## 🚀 �E��E� �E�작

### �E�드

```bash
cd cli
cargo build --release
```

�E�성�E�E�E�이�E�리:
- **Windows**: `target/release/saba-chan-cli.exe`
- **macOS/Linux**: `target/release/saba-chan-cli`

### �E����E

#### 1�E�⃣ �E�����E REPL �E��E�E(�E�장) 🎯

���E��그�E� �E�작 �E�E�E�동�E��E�E�E�����E �E��E�롁E�E�E������E�다:

```bash
saba-chan-cli
```

#### 2�E�⃣ �E��E� �E�E���E� �E��E�E

����E�E�E�업�E�E�E�행���고 �E�E��합�E�다:

```bash
saba-chan-cli server "�ÁE�드�E?�h�E status
saba-chan-cli bot config prefix "!play"
saba-chan-cli alias list
```

#### 3�E�⃣ JSON �E�력

���E��그�E�밁E�E�는 �E�동���를 �E�E�� JSON ���식�E��E�E�E�력:

```bash
saba-chan-cli --json server "����E�드�E�벁E status
saba-chan-cli -j alias list
```

## 🎮 �E�E���E� �E�E��E�E��E��E�E

### �E�벁E�E�어 `server <�E�벁E��E�E�는 ID>`

�E��E�����E� �E��E�E���E�E�E�벁E�� �E�어����E�다 (`instances.json` �E��E�):

```bash
saba-chan-cli server "����E�드�E�벁E           # �E��E��E�E status �E�력
saba-chan-cli server "����E�드�E�벁E status    # �E�E�E ���인
saba-chan-cli server "����E�드�E�벁E start     # �E�작
saba-chan-cli server "����E�드�E�벁E stop      # �E�지
saba-chan-cli server "����E�드�E�벁E restart   # �E��E�작
saba-chan-cli server "����E�드�E�벁E exec "�E�E��"    # �E��E���� �E�E�� �E����E
saba-chan-cli server "����E�드�E�벁E rcon "�E�E��"    # RCON �E�E�� �E����E
saba-chan-cli server "����E�드�E�벁E rest "�E�E��"    # REST �E�E�� �E����E

# �E�는 UUID�E�도 �E��E�
saba-chan-cli server 68b29cef-e584-4bd0-91dc-771865e31e25 status
```

### �E��E�E�E��E� `module`

```bash
saba-chan-cli module list                      # �E��E�된 �E��E�E�E��E�E
saba-chan-cli module info <�E��E�몁E             # �E��E�E�E�보 �E����E
saba-chan-cli module reload                    # �E��E� �E��E�E�E��E�E�E�드
saba-chan-cli module path <�E��E�E               # �E��E�E�E�렉�E�E� �E��E�E(TODO)
saba-chan-cli module mount <�E��E�E              # �E�E�E��E�E�E�운��� (TODO)
saba-chan-cli module unmount <�E��E�몁E          # �E��E�E�E��E�운��� (TODO)
```

### �E��E� �E�어 `daemon`

```bash
saba-chan-cli daemon status                    # �E��E� �E�E�E ���인
saba-chan-cli daemon start                     # �E��E� �E�작 (TODO)
saba-chan-cli daemon stop                      # �E��E� �E�지 (TODO)
saba-chan-cli daemon restart                   # �E��E� �E��E�작 (TODO)
```

### Discord �E�E�E�어 `bot`

�E�E�E��E� �E�E�E��E�E(GUI と共朁E:

```bash
saba-chan-cli bot status                       # �E�E�E�E�E ���인 (TODO)
saba-chan-cli bot start                        # �E�E�E�작 (TODO)
saba-chan-cli bot stop                         # �E�E�E�지 (TODO)

saba-chan-cli bot config prefix "!play"        # �E�E�� ���E������E� �E��E�
saba-chan-cli bot config alias "���" "palworld" # �E��E�E�E�E��E�E�가
saba-chan-cli bot config remove-alias "���"     # �E��E�E�E�E��E�E�거
saba-chan-cli bot config show                  # ���E�� �E�E�E��E�E�E����E
```

### �E�E��E& �E��E�E`alias`

```bash
saba-chan-cli alias list                            # �E��E� �E�벁E�E��E�E�E�E��E�E����E
saba-chan-cli alias module "���" "palworld"         # �E��E�E�E�E��E�E�가
saba-chan-cli alias remove-module "���"             # �E��E�E�E�E��E�E�거
```

**�E��E�**: �E�벁E�E�E��E�� `instances.json`�E�E�E�벁E�E��E�E���E�E�E��E��E��E�다.

## 💾 ���일 �E�E��E�E�E������성

### Windows
| ���일 | �E�E��E| �E��E�E|
|------|------|------|
| `bot-config.json` | `%APPDATA%\saba-chan\` | Discord �E�E�E��E�E(GUI�E� �E��E�) |
| `instances.json` | ���E��젝트 �E���� �E�는 `config\` | �E�벁E�E��E�����E� �E�의 |
| `saba-cli.json` | `%APPDATA%\saba-chan\` | CLI �E�E�� �E��E�E|

### Linux/macOS
| ���일 | �E�E��E| �E��E�E|
|------|------|------|
| `bot-config.json` | `~/.config/saba-chan/` | Discord �E�E�E��E�E(GUI�E� �E��E�) |
| `instances.json` | ���E��젝트 �E���� �E�는 `config/` | �E�벁E�E��E�����E� �E�의 |
| `saba-cli.json` | `~/.config/saba-chan/` | CLI �E�E�� �E��E�E|

### ���일 ���식

**bot-config.json** (Electron GUI�E� �E�일):
```json
{
  "prefix": "!saba-chan-cli",
  "moduleAliases": {
    "���": "palworld",
    "�E�인": "minecraft"
  },
  "commandAliases": {}
}
```

**instances.json**:
```json
[
  {
    "id": "68b29cef-e584-4bd0-91dc-771865e31e25",
    "name": "����E�드�E�벁E,
    "module_name": "palworld",
    "executable_path": "D:\\PalServer\\PalServer.exe",
    "port": 8211,
    "rcon_port": 25575,
    "rcon_password": "xxxx",
    "protocol_mode": "rest"
  }
]
```

## ⚙︁E�E�E�� �E��E�E

### Daemon URL �E��E�

�E��E��E�E `http://127.0.0.1:57474`

**�E�E���E�E�E��E�E**
```bash
saba-chan-cli --daemon http://my-server:57474 server "����E�드�E�벁E status
```

**�E��E�E���일:**
```bash
saba-chan-cli config show                           # ���E�� �E��E�E�E��E�
saba-chan-cli config set key value                 # �E��E�E�E��E�
saba-chan-cli config reset                         # �E��E��E�으�E�E�E�기���E
```

**saba-cli.json** (�E�동 �E�성):
```json
{
  "daemon_url": "http://127.0.0.1:57474"
}
```

## 💡 �E�����E �E��E�E(REPL)

```bash
saba-chan-cli
```

����E�E
✁E**�E��E�각E�E��E�터�E�E* - 2�E�마다 �E�E�E �E�동 �E��E�
✁E**�E��E� �E�E�� �E�E��** - �E�E���E� �E�E��E���E�E�시 �E�E�E ���인
✁E**�E�E�� ���스����E�** - �E�E�E�E�� ���살���롁E�E��E�E�E�E�� �E��E��E�
✁E**�E�시 �E��E�** - �E�E�� �E����E�E��E��E� �E�롁E���인

## 🔄 �E��E� �E�시

### �E�벁E�E�작/�E�지

```bash
# ����E�드 �E�벁E�E�작
saba-chan-cli server "����E�드�E�벁E start

# �E�인����E�프��� �E�벁E�E�지
saba-chan-cli server "my-minecraft-1" stop

# �E�벁E�E��E�작
saba-chan-cli server "����E�드�E�벁E restart
```

### RCON �E�E�� �E����E

```bash
# ����E�드 �E�벁E�� �E�시�E� �E�E�E
saba-chan-cli server "����E�드�E�벁E rcon "say Server restarts in 5 minutes!"
```

### �E�E�E��E�E

```bash
# �E�E�E�E�� ���E������E��E� !play�E�E�E��E�
saba-chan-cli bot config prefix "!play"

# �E��E�E�E�E��E�E�가 (�E�E�� "!��� info" �E�은 �E�으�E�E�E��E� �E��E�)
saba-chan-cli bot config alias "���" "palworld"

# ���E�� �E��E�E���인
saba-chan-cli bot config show
```

## 🐧 Linux/����E�리�E� �E�벁E���E�E�E��E�

GUI �E�E�� �E��E�E�� �E�벁E���E�도 �E�E��E�� �E�일���겁E�E�동����E�다:

```bash
# SSH�E�E�E��E
ssh user@server.com

# �E�����E �E��E�롁E�E��E�
saba-chan-cli

# �E��E� �E��E� �E��E� �E��E�
saba-chan-cli> server "����E�드�E�벁E start
saba-chan-cli> module reload
saba-chan-cli> bot config show
saba-chan-cli> exit
```

## 📋 �E�E���E� �E��E� �E�퍼�E��E�

| �E�주 | �E�E���E� | �E��E�E|
|------|--------|------|
| **�E�벁E* | `server <name> [status\|start\|stop\|restart]` | �E�벁E�E�어 |
| | `server <name> [exec\|rcon\|rest] <cmd>` | �E�E�� �E����E|
| **�E��E�E* | `module [list\|info\|reload]` | �E��E�E�E��E� |
| **�E�E* | `bot [status\|start\|stop]` | �E�E���E��세�E� |
| | `bot config [prefix\|alias\|show]` | �E�E�E��E�E|
| **�E�E��E* | `alias list` | �E��E� �E�E��E�E����E|
| | `alias module <alias> <module>` | �E��E�E�E�E��E�E�가 |
| **�E��E�E* | `config [show\|set\|reset]` | CLI �E��E�E|
| **Health** | `health` | Daemon �E�E�E |

## 🔧 �E�밁E

### ���E��젝트 �E��E�

```
saba-chan-cli/
├── src/
━E  ├── main.rs              # �E�E���E�E clap ���싱
━E  ├── client.rs            # Daemon API HTTP ����E��E��E����
━E  ├── alias.rs             # bot-config.json & instances.json �E��E�
━E  ├── commands/            # �E�E�E�E���E� �E����E
━E  ━E  ├── server.rs
━E  ━E  ├── module.rs
━E  ━E  ├── instance.rs
━E  ━E  ├── exec.rs
━E  ━E  └── config.rs
━E  ├── interactive/         # REPL �E��E�E
━E  ━E  ├── state.rs
━E  ━E  ├── repl.rs
━E  ━E  └── mod.rs
━E  └── utils/
━E      ├── table.rs
━E      └── mod.rs
└── Cargo.toml
```

### �E�존�E�

- **clap**: CLI ���싱
- **tokio**: �E�E���E� �E�����E�E
- **reqwest**: HTTP ����E��E��E����
- **rustyline**: �E�����E �E�E��
- **serde_json**: JSON �E�리

## 📝 �E��E��E��E�

���E��젝트�E�E�E��E��E��E��E� �E��E�E���E�.
