# saba-chan-cli

Saba-chan Core 데몬을 조작하기 위한 크로스플랫폼 CLI/TUI.

- **인터랙티브 TUI**: 인자 없이 실행하면 풀스크린 터미널 UI 진입.
- **단일 명령 모드 (one-shot)**: 서브커맨드를 주면 한 줄 실행 후 종료. 자동화/스크립트/CI에 적합.
- **머신 리더블 출력**: 모든 one-shot 명령은 `--json` 플래그로 JSON 출력 가능.
- **GUI 동등성**: GUI에서 가능한 데몬 조작은 거의 동일하게 CLI에서도 가능.

> 데이터(설정·로그·토큰)는 모두 `%APPDATA%\saba-chan` (Windows) /
> `~/.config/saba-chan` (Unix)에 저장됩니다. CLI는 GUI와 같은 경로를 공유합니다.

---

## 🚀 빌드 및 실행

```bash
cd saba-chan-cli
cargo build --release
```

산출물:
- Windows: `target/release/saba-chan-cli.exe`
- Linux/macOS: `target/release/saba-chan-cli`

### 실행 모드

```bash
# 1) 인터랙티브 TUI (기본; alt-screen 풀스크린)
saba-chan-cli

# 2) 인라인 REPL 셸 (스크롤백 보존, Claude Code 스타일)
saba-chan-cli shell
saba-chan-cli shell --use mc-1     # 시작부터 인스턴스 선택

# 3) 단일 명령 모드 (자동화)
saba-chan-cli server "팰월드 서버" status
saba-chan-cli --json module list
saba-chan-cli daemon status

# 4) 명시적 TUI 진입
saba-chan-cli tui
```

---

## 🌐 글로벌 옵션

| 플래그 | 환경변수 | 용도 |
|--------|----------|------|
| `--daemon <URL>` | `SABA_DAEMON_URL` | 데몬 베이스 URL 오버라이드 |
| `--json`, `-j` | — | 결과를 JSON으로 출력 (스크립트용) |
| `--quiet`, `-q` | — | 성공 시 장식 메시지 생략 |
| `--yes`, `-y` | — | `delete`/`remove`/`reset`/`shutdown` 등 파괴적 작업 자동 승인 |
| `-h`, `--help` | — | 도움말. 모든 서브커맨드에 적용 |
| `-V`, `--version` | — | 버전 |

---

## 🚦 종료 코드 (one-shot 전용)

| 코드 | 의미 |
|------|------|
| `0` | 성공 |
| `1` | 일반 에러 (서버 4xx/5xx, 잘못된 입력 거부 등) |
| `2` | 데몬 오프라인 / 연결 거부 |
| `3` | 사용 오류 (clap 거부) |
| `4` | 대상 없음 (인스턴스/모듈/익스텐션 미존재) |

JSON 모드에서는 stderr에 `{"ok": false, "error": "..."}` 형태가 출력됩니다.

---

## 🎮 명령어 레퍼런스

> 인스턴스 식별자는 **이름**(대소문자 무시) 또는 **UUID** 둘 다 허용합니다.

### 전체 상태

```bash
saba-chan-cli status
```

### 서버 인스턴스 `server`

```bash
saba-chan-cli server list                           # 인스턴스 목록 (테이블)
saba-chan-cli server info <name|id>                 # 인스턴스 메타데이터
saba-chan-cli server status <name>                  # 런타임 상태
saba-chan-cli server start <name>                   # 시작
saba-chan-cli server stop <name> [-f|--force]       # 정지 (-f 시 강제)
saba-chan-cli server restart <name>                 # 재시작
saba-chan-cli server exec <name> <command...>       # stdin 명령 전송
saba-chan-cli server rcon <name> <command...>       # RCON 명령
saba-chan-cli server rest <name> <endpoint>         # REST 명령
                  [--method GET] [--body '{...}']
saba-chan-cli server console <name> [--count N]     # 콘솔 출력
                  [-f|--follow] [--interval MS]     # 실시간 스트리밍 (Ctrl+C로 종료)
saba-chan-cli server diagnose <name>                # 진단
saba-chan-cli server properties <name>              # server.properties 조회
saba-chan-cli server properties-reset <name> --yes  # 프로퍼티 기본값 복원
saba-chan-cli server reset <name> --yes             # 서버 초기 상태 복원
saba-chan-cli server accept-eula <name>             # EULA 수락
saba-chan-cli server check-update <name>            # 게임 업데이트 확인
saba-chan-cli server apply-update <name>            # 게임 업데이트 적용
saba-chan-cli server installed-version <name>      # 설치된 게임 버전
saba-chan-cli server validate <name>                # 설정 검증
saba-chan-cli server delete <name> --yes            # 인스턴스 삭제
```

### 모듈 `module`

```bash
saba-chan-cli module list                            # 설치 모듈
saba-chan-cli module info <name>                     # 메타데이터
saba-chan-cli module versions <name>                 # 게임 버전 목록
saba-chan-cli module refresh                         # 매니페스트 재로드
saba-chan-cli module manifest                        # 원격 매니페스트
saba-chan-cli module install <id>                    # 매니페스트에서 설치
saba-chan-cli module remove <id> --yes               # 모듈 삭제
```

### 익스텐션 `extension`

```bash
saba-chan-cli extension list
saba-chan-cli extension enable <id>
saba-chan-cli extension disable <id>
saba-chan-cli extension manifest
saba-chan-cli extension install <id>
saba-chan-cli extension remove <id> --yes
saba-chan-cli extension rescan
saba-chan-cli extension check-updates
```

### Discord 봇 `bot`

디스코드 봇은 클라우드 릴레이 서버에서 운영되므로 사용자 측에서 봇 프로세스를 직접 시작/정지하지 않습니다. CLI 는 봇 메타데이터(prefix, 별명 등) 와 페어링 상태만 다룹니다.

```bash
saba-chan-cli bot status                                  # 페어링 상태 + 메타데이터
saba-chan-cli bot config show                             # 봇 설정 출력
saba-chan-cli bot config prefix <value>                   # 명령 접두사
saba-chan-cli bot config music <on|off>                   # 음악 명령 허용 여부
```

### 데몬 `daemon`

```bash
saba-chan-cli daemon status                                  # 도달 가능 여부
saba-chan-cli daemon start                                   # 바이너리 spawn
saba-chan-cli daemon stop                                    # graceful 종료
saba-chan-cli daemon restart
saba-chan-cli daemon console [--since N] [--count N]         # 데몬 로그
                  [-f|--follow] [--interval MS]              # 실시간 스트리밍
saba-chan-cli daemon shutdown --yes                          # API graceful shutdown
```

### 자체 업데이트 `update`

```bash
saba-chan-cli update check                                   # 업데이트 확인
saba-chan-cli update status
saba-chan-cli update download
saba-chan-cli update apply --yes
saba-chan-cli update progress
saba-chan-cli update config-show
saba-chan-cli update config-set <key> <value>
```

### CLI 설정 `config`

```bash
saba-chan-cli config show                  # 모든 CLI 설정
saba-chan-cli config get <key>
saba-chan-cli config set <key> <value>
saba-chan-cli config reset <key>
saba-chan-cli config path                  # cli-settings.json 경로
saba-chan-cli config daemon-url            # 사용 중인 IPC URL
```

### 별칭 `alias`

```bash
saba-chan-cli alias list                   # 인스턴스 + 모듈 별칭
saba-chan-cli alias add <alias> <module>   # 모듈 별칭 추가
saba-chan-cli alias remove <alias>
```

---

## 💾 데이터 위치

| 파일 | Windows | Linux/macOS | 용도 |
|------|---------|-------------|------|
| `bot-config.json` | `%APPDATA%\saba-chan\` | `~/.config/saba-chan/` | 봇 메타데이터 (prefix · 별명 · 노드 권한). 페어링 후 릴레이에 동기화 |
| `settings.json` | `%APPDATA%\saba-chan\` | `~/.config/saba-chan/` | GUI 공통 설정 |
| `cli-settings.json` | `%APPDATA%\saba-chan\` | `~/.config/saba-chan/` | CLI 전용 설정 |
| `instances.json` | `%APPDATA%\saba-chan\` | `~/.config/saba-chan/` | 인스턴스 정의 |
| `.ipc_token` | `%APPDATA%\saba-chan\` | `~/.config/saba-chan/` | IPC 인증 토큰 |
| `.node_token` | `%APPDATA%\saba-chan\` | `~/.config/saba-chan/` | 클라우드 릴레이 페어링 토큰 |

---

## 🔄 자동화 예시

```bash
# 데몬이 응답하지 않으면 자동 기동
saba-chan-cli daemon status >/dev/null 2>&1 || saba-chan-cli daemon start

# CI에서 안전한 정지 + 재시작
saba-chan-cli server stop "mc-1" -f
saba-chan-cli server start "mc-1"

# JSON 파이프라인
saba-chan-cli --json status | jq '.instances'
saba-chan-cli --json server list | jq '.[].name'
```

---

## 💬 인라인 셸 (`shell`)

`saba-chan-cli shell` — 일반 터미널을 그대로 두고 프롬프트만 띄우는 REPL.
스크롤백이 그대로 남아 있어 SSH 위에서도 자연스럽게 사용 가능합니다.

### 슬래시 명령

| 명령 | 동작 |
|------|------|
| `/help` | 도움말 |
| `/use <name>` | 인스턴스 선택. 이후 바로 `start`, `stop`, `console -f` 등 가능 |
| `/use` (인자 없음) | 선택 해제 |
| `/json on\|off` | JSON 출력 토글 |
| `/yes on\|off` | 파괴적 명령 자동 승인 토글 |
| `/status` | 현재 셸 상태(인스턴스, 플래그) |
| `/clear` | 화면 클리어 |
| `/exit`, `/quit` | 종료 (Ctrl+D 도 가능) |

### 단축 동사 (`/use` 후 활성화)

`start` · `stop` · `restart` · `status` · `info` · `console` · `exec` ·
`rcon` · `rest` · `diagnose` · `properties` · `validate`
→ 자동으로 `server <verb> <selected> ...` 로 확장.

### 예시 세션

```text
$ saba-chan-cli shell
saba-chan-cli shell — type /help for commands, /exit to quit.
saba> /use mc-1
(now targeting 'mc-1')
saba [mc-1]> status
PID: 4242  port: 25565  state: running
saba [mc-1]> console -f
[12:34:56] Player joined the game
^C
(stopped)
saba [mc-1]> exec say hello world
ok
saba [mc-1]> /exit
```

히스토리는 `%APPDATA%\saba-chan\shell-history.txt` (또는
`~/.config/saba-chan/shell-history.txt`)에 저장되며, 화살표 ↑/↓ 와 Ctrl+R
역검색으로 불러올 수 있습니다.

---

## 🎛 인터랙티브 TUI 기능

`saba-chan-cli` 또는 `saba-chan-cli tui` 실행 시:

- 대시보드 → Servers / Modules / Extensions / Bot / Settings / Updates / Daemon
- 라이브 상태 갱신 (1초 폴링, 데몬 오프라인 시 마지막 캐시 유지)
- 슬래시 명령(`:server start ...` 등) 및 화살표 히스토리
- Tab 자동완성 (서브커맨드 → 인스턴스/모듈 이름)
- 다국어(한/영/일/중/스/포/러/독/프) 지원

---

## 🧪 테스트

```bash
cargo test -p saba-chan-cli
```

- 186 단위 테스트 (TUI 상태, autocomplete, slash, theme, properties, …)
- 3 컴포넌트 인테그레이션 (실제 IPCServer 부팅 후 client 라운드트립)
- 6 one-shot E2E (dispatcher가 EXIT_OK / EXIT_NOT_FOUND / EXIT_DAEMON_OFFLINE
  을 정확히 반환하는지 검증)
- one-shot 모듈 단위 테스트 (clap 파서, 에러 분류, 포맷터)

---

## 🔧 프로젝트 구조

```
saba-chan-cli/
├── src/
│   ├── main.rs              # 엔트리: clap 파싱 → TUI 또는 one-shot 디스패처
│   ├── lib.rs               # tests/ 재사용용 모듈 공개
│   ├── client.rs            # DaemonClient (HTTP)
│   ├── config.rs            # GUI 공유 설정 로드
│   ├── cli_config.rs        # CLI 전용 설정 (cli-settings.json)
│   ├── i18n.rs              # 다국어 리졸버
│   ├── module_registry.rs   # 모듈 메타 캐시
│   ├── process.rs           # 데몬 프로세스 라이프사이클
│   ├── oneshot/
│   │   ├── mod.rs           # clap 정의 + Output + 에러 분류
│   │   ├── format.rs        # 인스턴스/모듈/익스텐션 테이블 포맷터
│   │   └── runner.rs        # 서브커맨드 디스패처
│   └── tui/                 # ratatui 기반 인터랙티브 UI
└── tests/
    ├── client_round_trip.rs # 컴포넌트 인테그레이션
    └── oneshot_e2e.rs       # one-shot 디스패처 E2E
```

---

## 📝 기여

[CONTRIBUTING.md](../CONTRIBUTING.md)를 참고하세요.
