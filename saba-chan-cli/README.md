# Saba-CLI: 게임 서버 관리 명령줄 인터페이스

Saba-chan Core Daemon을 제어하기 위한 완전한 CLI 클라이언트입니다. 
Windows, macOS, Linux에서 동일하게 작동하며, **Electron GUI와 설정 파일을 공유합니다**.

## 🚀 빠른 시작

### 빌드

```bash
cd cli
cargo build --release
```

생성된 바이너리:
- **Windows**: `target/release/saba.exe`
- **macOS/Linux**: `target/release/saba`

### 실행

#### 1️⃣ 대화형 REPL 모드 (권장) 🎯

프로그램 시작 시 자동으로 대화형 모드로 진입합니다:

```bash
saba
```

#### 2️⃣ 단일 명령어 모드

특정 작업만 수행하고 종료합니다:

```bash
saba server "팰월드서버" status
saba bot config prefix "!play"
saba alias list
```

#### 3️⃣ JSON 출력

프로그래밍 또는 자동화를 위해 JSON 형식으로 출력:

```bash
saba --json server "팰월드서버" status
saba -j alias list
```

## 🎮 명령어 완전 가이드

### 서버 제어 `server <서버명 또는 ID>`

인스턴스 이름으로 서버를 제어합니다 (`instances.json` 참조):

```bash
saba server "팰월드서버"           # 기본값: status 출력
saba server "팰월드서버" status    # 상태 확인
saba server "팰월드서버" start     # 시작
saba server "팰월드서버" stop      # 중지
saba server "팰월드서버" restart   # 재시작
saba server "팰월드서버" exec "명령"    # 커스텀 명령 실행
saba server "팰월드서버" rcon "명령"    # RCON 명령 실행
saba server "팰월드서버" rest "명령"    # REST 명령 실행

# 또는 UUID로도 가능
saba server 68b29cef-e584-4bd0-91dc-771865e31e25 status
```

### 모듈 관리 `module`

```bash
saba module list                      # 설치된 모듈 목록
saba module info <모듈명>             # 모듈 정보 조회
saba module reload                    # 모든 모듈 다시 로드
saba module path <경로>               # 모듈 디렉토리 설정 (TODO)
saba module mount <경로>              # 새 모듈 마운트 (TODO)
saba module unmount <모듈명>          # 모듈 언마운트 (TODO)
```

### 데몬 제어 `daemon`

```bash
saba daemon status                    # 데몬 상태 확인
saba daemon start                     # 데몬 시작 (TODO)
saba daemon stop                      # 데몬 중지 (TODO)
saba daemon restart                   # 데몬 재시작 (TODO)
```

### Discord 봇 제어 `bot`

봇 관리 및 설정 (GUI と共有):

```bash
saba bot status                       # 봇 상태 확인 (TODO)
saba bot start                        # 봇 시작 (TODO)
saba bot stop                         # 봇 중지 (TODO)

saba bot config prefix "!play"        # 명령 프리픽스 변경
saba bot config alias "팰" "palworld" # 모듈 별명 추가
saba bot config remove-alias "팰"     # 모듈 별명 제거
saba bot config show                  # 현재 봇 설정 조회
```

### 별명 & 설정 `alias`

```bash
saba alias list                            # 모든 서버/모듈 별명 조회
saba alias module "팰" "palworld"         # 모듈 별명 추가
saba alias remove-module "팰"             # 모듈 별명 제거
```

**참고**: 서버 별명은 `instances.json`의 서버 이름으로 관리됩니다.

## 💾 파일 위치 및 호환성

### Windows
| 파일 | 위치 | 설명 |
|------|------|------|
| `bot-config.json` | `%APPDATA%\saba-chan\` | Discord 봇 설정 (GUI와 공유) |
| `instances.json` | 프로젝트 루트 또는 `config\` | 서버 인스턴스 정의 |
| `saba-cli.json` | `%APPDATA%\saba-chan\` | CLI 전용 설정 |

### Linux/macOS
| 파일 | 위치 | 설명 |
|------|------|------|
| `bot-config.json` | `~/.config/saba-chan/` | Discord 봇 설정 (GUI와 공유) |
| `instances.json` | 프로젝트 루트 또는 `config/` | 서버 인스턴스 정의 |
| `saba-cli.json` | `~/.config/saba-chan/` | CLI 전용 설정 |

### 파일 형식

**bot-config.json** (Electron GUI와 동일):
```json
{
  "prefix": "!saba",
  "moduleAliases": {
    "팰": "palworld",
    "마인": "minecraft"
  },
  "commandAliases": {}
}
```

**instances.json**:
```json
[
  {
    "id": "68b29cef-e584-4bd0-91dc-771865e31e25",
    "name": "팰월드서버",
    "module_name": "palworld",
    "executable_path": "D:\\PalServer\\PalServer.exe",
    "port": 8211,
    "rcon_port": 25575,
    "rcon_password": "xxxx",
    "protocol_mode": "rest"
  }
]
```

## ⚙️ 전역 설정

### Daemon URL 변경

기본값: `http://127.0.0.1:57474`

**명령줄 옵션:**
```bash
saba --daemon http://my-server:57474 server "팰월드서버" status
```

**설정 파일:**
```bash
saba config show                           # 현재 설정 보기
saba config set key value                 # 설정 변경
saba config reset                         # 기본값으로 초기화
```

**saba-cli.json** (자동 생성):
```json
{
  "daemon_url": "http://127.0.0.1:57474"
}
```

## 💡 대화형 모드 (REPL)

```bash
saba
```

특징:
✅ **실시간 모니터링** - 2초마다 상태 자동 갱신
✅ **빠른 명령 입력** - 명령어 완료 후 즉시 상태 확인
✅ **명령 히스토리** - 위/아래 화살표로 이전 명령 재사용
✅ **즉시 결과** - 명령 실행 결과를 바로 확인

## 🔄 사용 예시

### 서버 시작/중지

```bash
# 팰월드 서버 시작
saba server "팰월드서버" start

# 마인크래프트 서버 중지
saba server "my-minecraft-1" stop

# 서버 재시작
saba server "팰월드서버" restart
```

### RCON 명령 실행

```bash
# 팰월드 서버에 메시지 전송
saba server "팰월드서버" rcon "say Server restarts in 5 minutes!"
```

### 봇 설정

```bash
# 봇 명령 프리픽스를 !play로 변경
saba bot config prefix "!play"

# 모듈 별명 추가 (봇이 "!팰 info" 같은 식으로 사용 가능)
saba bot config alias "팰" "palworld"

# 현재 설정 확인
saba bot config show
```

## 🐧 Linux/헤드리스 서버에서 사용

GUI 없는 리눅스 서버에서도 완전히 동일하게 작동합니다:

```bash
# SSH로 접속
ssh user@server.com

# 대화형 모드로 관리
saba

# 모든 기능 사용 가능
saba> server "팰월드서버" start
saba> module reload
saba> bot config show
saba> exit
```

## 📋 명령어 빠른 레퍼런스

| 범주 | 명령어 | 설명 |
|------|--------|------|
| **서버** | `server <name> [status\|start\|stop\|restart]` | 서버 제어 |
| | `server <name> [exec\|rcon\|rest] <cmd>` | 명령 실행 |
| **모듈** | `module [list\|info\|reload]` | 모듈 관리 |
| **봇** | `bot [status\|start\|stop]` | 봇 프로세스 |
| | `bot config [prefix\|alias\|show]` | 봇 설정 |
| **별명** | `alias list` | 모든 별명 조회 |
| | `alias module <alias> <module>` | 모듈 별명 추가 |
| **설정** | `config [show\|set\|reset]` | CLI 설정 |
| **Health** | `health` | Daemon 상태 |

## 🔧 개발

### 프로젝트 구조

```
saba-chan-cli/
├── src/
│   ├── main.rs              # 진입점, clap 파싱
│   ├── client.rs            # Daemon API HTTP 클라이언트
│   ├── alias.rs             # bot-config.json & instances.json 관리
│   ├── commands/            # 각 명령어 구현
│   │   ├── server.rs
│   │   ├── module.rs
│   │   ├── instance.rs
│   │   ├── exec.rs
│   │   └── config.rs
│   ├── interactive/         # REPL 모드
│   │   ├── state.rs
│   │   ├── repl.rs
│   │   └── mod.rs
│   └── utils/
│       ├── table.rs
│       └── mod.rs
└── Cargo.toml
```

### 의존성

- **clap**: CLI 파싱
- **tokio**: 비동기 런타임
- **reqwest**: HTTP 클라이언트
- **rustyline**: 대화형 입력
- **serde_json**: JSON 처리

## 📝 라이선스

프로젝트의 라이선스를 따릅니다.
