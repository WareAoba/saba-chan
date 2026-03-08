# 🐟 Saba-chan (サバちゃん / 사바쨩)

> **모듈형 게임 서버 관리 플랫폼** — 여러 게임 서버를 하나의 앱에서 통합 관리

## 개요

**Saba-chan**은 Palworld, Minecraft, Project Zomboid 등 다양한 게임 서버를 한 곳에서 관리하기 위한 데스크톱 애플리케이션입니다.

> 이름의 유래: 일본어로 "서버(サーバー)"와 "고등어(鯖, さば)"의 발음이 비슷한 것에서 착안한 말장난입니다.

---

## 아키텍처

```
┌─────────────────┐     HTTP REST API  ┌──────────────────┐
│  Electron GUI   │ ◄────────────────► │   Core Daemon    │
│  (React 18)     │  127.0.0.1:57474   │   (Rust/Axum)    │
└─────────────────┘                    └────────┬─────────┘
                                                │
┌─────────────────┐     HTTP REST API           │
│   CLI (TUI)     │ ◄─────────────────────────►│
│  (Ratatui)      │                   ┌─────────┴──────────┐
└─────────────────┘                   │                    │
                                ┌─────▼──────┐     ┌──────▼──────┐
┌─────────────────┐             │   Modules  │     │  Instances  │
│  Discord Bot    │             │  (게임별)   │     │ (서버 설정)  │
│  (discord.js)   │             └────────────┘     └─────────────┘
└────────┬────────┘                    │
         │ IPC                  ┌──────▼──────┐
         └────────────────────► │ Extensions  │
                                │ (Docker 등) │
                                └─────────────┘
```

| 컴포넌트 | 기술 스택 | 역할 |
|----------|-----------|------|
| **Core Daemon** (`saba-core.exe`) | Rust, Axum | 모든 서버 관리 기능의 핵심. 백그라운드 상주 |
| **GUI** (`Saba-chan.exe`) | Electron 28, React 18, Vite, Zustand | 데스크톱 GUI |
| **CLI** (`saba-chan-cli.exe`) | Rust, Ratatui, Crossterm | 터미널 TUI |
| **Discord Bot** | discord.js 14 | 디스코드 채팅 기반 제어 |
| **Updater** (`saba-chan-updater.exe`) | Rust | 자동 업데이트 적용기 |

---

## 주요 기능

### 이중 인터페이스 (GUI + CLI)

데스크톱 GUI와 터미널 TUI 두 가지 인터페이스를 모두 지원합니다. 두 클라이언트는 동일한 Core Daemon에 연결되어 동시에 사용할 수 있습니다.

- **GUI**: 서버 카드 대시보드, 설정 모달, 콘솔 도킹 패널·팝업 창, 익스텐션 뱃지
- **CLI**: vim-like 키 바인딩(`j/k/Enter/Esc`), 레거시 커맨드 모드(`:명령어`), 20여 개의 TUI 화면

### 인스턴스 관리

하나의 게임을 여러 개의 독립 인스턴스로 분리하여 관리할 수 있습니다. 각 인스턴스는 UUID 기반 디렉토리에 독립적인 설정을 저장합니다.

- 인스턴스 생성 / 삭제 / 정렬
- 모듈이 정의한 설정 필드를 타입에 맞는 UI로 편집 (`text`, `number`, `boolean`, `select`, `password`, `file`, `folder`)
- 비밀번호(RCON/REST) 자동 생성 및 포트 충돌 사전 검사

### 서버 콘솔

서버의 실시간 stdout/stderr 출력을 확인하고, 서버에 직접 명령어를 전달할 수 있습니다.

- GUI: 메인 화면 도킹 패널 + 독립 팝업 창 분리 가능
- 모듈별 구문 강조(`syntax_highlight`) 지원
- RCON (Valve Source), REST API, stdin 등 모듈이 지원하는 프로토콜로 명령어 전송

### 모듈 시스템

게임별 서버 관리 로직을 **Python 스크립트 + TOML 설정 파일**로 작성하는 플러그인 시스템입니다. 새 게임을 추가할 때 Core를 재컴파일하지 않아도 됩니다.

#### 지원 게임

| 게임 | 프로토콜 | 설치 방식 | 기본 포트 |
|------|----------|-----------|-----------|
| **Palworld** | REST API | SteamCMD (App ID: 2394010) | 8211 |
| **Minecraft** | RCON + stdin | 공식 다운로드 (mojang.com) | 25565 |
| **Project Zomboid** | RCON + stdin | SteamCMD (App ID: 380870) | 16261 |

> 커뮤니티에서 제작한 모듈을 원격 매니페스트에서 직접 검색하고 설치할 수 있습니다.  
> 모듈 개발에 대한 자세한 내용은 [모듈 개발 가이드](../saba-chan-modules/docs/module-development-guide.md)를 참조하세요.

### 익스텐션 시스템

특정 게임에 종속되지 않는 범용 확장 기능입니다. Hook 시스템으로 서버 생명주기의 다양한 지점에 개입하고, GUI/CLI에 UI 슬롯을 주입할 수 있습니다.

#### 내장 익스텐션

| ID | 이름 | 설명 |
|----|------|------|
| `docker` | Docker Isolation | Docker 컨테이너로 게임 서버를 격리하여 실행. GUI 뱃지 및 통계 슬롯 포함 |
| `steamcmd` | SteamCMD | SteamCMD 기반 서버 자동 설치 / 업데이트 |
| `music` | Music Bot | Discord 음성 채널 음악 재생 (yt-dlp + ffmpeg) |
| `ue4-ini` | UE4 INI Parser | Unreal Engine 4 INI OptionSettings 파싱 |

익스텐션은 GUI 설정 탭 또는 CLI `:ext enable/disable/install` 명령으로 관리합니다.

### Discord 봇

discord.js 14 기반으로 디스코드 채팅에서 게임 서버를 직접 제어합니다.

- **로컬 모드**: 봇이 Discord에 직접 로그인하여 IPC로 로컬 데몬에 명령 전달
- **클라우드 모드**: NAT / 포트 포워딩 없는 환경에서도 릴레이 서버를 통해 동작
- **별칭 시스템**: 모듈명과 명령어에 한국어 별칭 지정 가능 (`팔 켜` → `palworld start`)
- **별칭 핫 리로드**: `bot-config.json` 변경 시 봇 재시작 없이 즉시 반영
- **봇 익스텐션**: 음악 재생(`music.js`), 이스터 에그(`easter_eggs.js`), 가위바위보(`rps.js`)

```
!saba palworld status   → Palworld 서버 상태 확인
!saba minecraft stop    → Minecraft 서버 정지
!saba 팔 켜             → 별칭으로 Palworld 시작
```

기본 prefix는 `!saba`이며, `bot-config.json`에서 변경할 수 있습니다.

### 릴레이 서버 (클라우드 모드)

`server-chan`은 NAT 뒤의 로컬 사바쨩 인스턴스와 Discord 봇 사이를 중계하는 클라우드 서버입니다.

- **Pull 모델**: 로컬 노드가 릴레이에 접속하므로 포트 포워딩 불필요
- **보안**: argon2id 토큰 해싱, HMAC-SHA256 요청 서명
- **프라이버시**: 릴레이 서버는 payload를 해석하지 않으며 IP를 저장하지 않음
- 기술 스택: Fastify 5 + PostgreSQL 17

### 자동 업데이트 시스템

GitHub Releases 기반의 업데이트 시스템이 내장되어 있습니다.

- 3시간 간격으로 자동 업데이트 확인 (설정 변경 가능)
- 다운로드 파일의 **SHA256** 해시 무결성 검증
- 업데이트 대상: Core Daemon, CLI, GUI, Discord Bot, Updater, 다국어 파일, 모듈/익스텐션 단독 업데이트
- GUI/CLI의 업데이트 탭 또는 `:update check` / `:update download` / `:update apply` 명령으로 수동 제어

### 다국어 지원

10개 언어(한국어, 영어, 일본어, 독일어, 스페인어, 프랑스어, 포르투갈어(BR), 러시아어, 중국어 간체/번체)를 지원합니다.

### REST API

Core Daemon은 `http://127.0.0.1:57474`에서 REST API를 제공합니다. 모든 요청에는 `X-Saba-Token` 헤더(`.ipc_token` 파일 기반)가 필요합니다.

주요 엔드포인트: 서버 시작/정지/상태, 인스턴스 CRUD, 콘솔 I/O, 모듈/익스텐션/업데이트 관리, 봇 제어.

```powershell
# 예시: 서버 목록 조회
$token = Get-Content "$env:APPDATA\saba-chan\.ipc_token"
Invoke-RestMethod -Uri "http://127.0.0.1:57474/api/servers" -Headers @{ "X-Saba-Token" = $token }
```

전체 API 명세는 [사용자 가이드 § REST API 레퍼런스](docs/USER_GUIDE.md#15-rest-api-레퍼런스)를 참조하세요.

---

## 설치

### 인스톨러 사용 (권장)

[GitHub Releases](https://github.com/WareAoba/saba-chan/releases)에서 최신 인스톨러를 다운로드합니다. Python과 Node.js는 사바쨩이 포터블 환경을 자동으로 부트스트랩하므로 별도 설치가 필요하지 않습니다.

### 소스에서 빌드

#### 요구사항

| 항목 | 버전 |
|------|------|
| **OS** | Windows 10 / 11 |
| **Rust** | 최신 stable |
| **Node.js** | 18.0 이상 |
| **Python** | 3.x |

#### 전체 빌드

```powershell
.\scripts\build-windows.ps1
```

Core Daemon (Rust workspace), GUI (Vite + electron-builder), Discord Bot (npm)을 병렬로 빌드합니다.

출력 바이너리: `saba-core.exe`, `saba-chan-cli.exe`, `saba-chan-updater.exe`, `Saba-chan.exe`

#### 개별 빌드 / 개발 모드

```powershell
# Core Daemon
cargo build --release

# GUI 개발 모드
cd saba-chan-gui; npm install; npm start

# CLI
cargo build --release -p saba-chan-cli

# Discord Bot
cd discord_bot; npm install; npm start
```

---

## 프로젝트 구조

```
saba-chan/
├── src/                    # Core Daemon (Rust)
│   ├── ipc/                # Axum HTTP API 서버 (포트 57474)
│   ├── supervisor/         # 프로세스 관리 및 모듈 로더
│   ├── instance/           # 인스턴스 저장소 (UUID 기반)
│   ├── plugin/             # Python lifecycle 실행기
│   ├── protocol/           # RCON / REST 클라이언트
│   ├── extension/          # 익스텐션 Hook 디스패처
│   ├── python_env/         # Python venv 자동 부트스트랩
│   └── node_env/           # Node.js 포터블 환경 다운로더
├── saba-chan-cli/          # CLI (Rust, Ratatui)
├── saba-chan-gui/          # GUI (Electron 28 + React 18 + Vite)
├── discord_bot/            # Discord Bot (discord.js 14)
├── updater/                # Updater (Rust)
├── installer/              # 인스톨러 (Tauri)
├── modules/                # 게임 모듈 (saba-chan-modules 참조)
├── locales/                # 다국어 번역 파일 (10개 언어)
└── docs/
    └── USER_GUIDE.md       # 공식 사용자 가이드
```

---

## 문서

모든 기능 사용법, 설정 레퍼런스, REST API 명세, CLI 명령어 레퍼런스, FAQ는 아래 가이드에서 확인할 수 있습니다.

**[📖 공식 사용자 가이드 (docs/USER_GUIDE.md)](docs/USER_GUIDE.md)**

---

## 라이선스

[MIT License](LICENSE)
