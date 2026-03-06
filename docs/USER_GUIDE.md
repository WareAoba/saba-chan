# Saba-chan (サバちゃん / 사바쨩) 공식 사용자 가이드

> **모듈형 게임 서버 관리 플랫폼** — 여러 게임 서버를 하나의 GUI/CLI에서 통합 관리

---

## 목차

1. [소개](#1-소개)
2. [시스템 요구사항](#2-시스템-요구사항)
3. [설치](#3-설치)
   - 3.1 [인스톨러를 이용한 설치](#31-인스톨러를-이용한-설치)
   - 3.2 [소스에서 빌드](#32-소스에서-빌드)
4. [빠른 시작](#4-빠른-시작)
5. [아키텍처 개요](#5-아키텍처-개요)
   - 5.1 [전체 구조](#51-전체-구조)
   - 5.2 [Core Daemon](#52-core-daemon)
   - 5.3 [통신 방식](#53-통신-방식)
6. [GUI 사용법](#6-gui-사용법)
   - 6.1 [메인 화면](#61-메인-화면)
   - 6.2 [인스턴스 관리](#62-인스턴스-관리)
   - 6.3 [서버 콘솔](#63-서버-콘솔)
   - 6.4 [설정](#64-설정)
7. [CLI 사용법](#7-cli-사용법)
   - 7.1 [TUI 화면 구성](#71-tui-화면-구성)
   - 7.2 [입력 모드](#72-입력-모드)
   - 7.3 [레거시 커맨드 모드](#73-레거시-커맨드-모드)
   - 7.4 [CLI 명령어 레퍼런스](#74-cli-명령어-레퍼런스)
8. [인스턴스 관리](#8-인스턴스-관리)
   - 8.1 [인스턴스란?](#81-인스턴스란)
   - 8.2 [인스턴스 생성](#82-인스턴스-생성)
   - 8.3 [인스턴스 설정](#83-인스턴스-설정)
   - 8.4 [서버 시작 / 정지 / 재시작](#84-서버-시작--정지--재시작)
   - 8.5 [서버 프로퍼티 편집](#85-서버-프로퍼티-편집)
   - 8.6 [서버 명령어 실행](#86-서버-명령어-실행)
9. [모듈 시스템](#9-모듈-시스템)
   - 9.1 [모듈이란?](#91-모듈이란)
   - 9.2 [지원 게임](#92-지원-게임)
   - 9.3 [모듈 설치 및 관리](#93-모듈-설치-및-관리)
   - 9.4 [모듈 구조](#94-모듈-구조)
10. [익스텐션 시스템](#10-익스텐션-시스템)
    - 10.1 [익스텐션이란?](#101-익스텐션이란)
    - 10.2 [내장 익스텐션](#102-내장-익스텐션)
    - 10.3 [익스텐션 관리](#103-익스텐션-관리)
11. [Discord 봇](#11-discord-봇)
    - 11.1 [개요](#111-개요)
    - 11.2 [봇 설정](#112-봇-설정)
    - 11.3 [동작 모드](#113-동작-모드)
    - 11.4 [봇 명령어](#114-봇-명령어)
    - 11.5 [별칭 시스템](#115-별칭-시스템)
    - 11.6 [봇 익스텐션](#116-봇-익스텐션)
12. [설정 레퍼런스](#12-설정-레퍼런스)
    - 12.1 [전역 설정 (global.toml)](#121-전역-설정-globaltoml)
    - 12.2 [GUI 설정 (settings.json)](#122-gui-설정-settingsjson)
    - 12.3 [CLI 설정 (cli-settings.json)](#123-cli-설정-cli-settingsjson)
    - 12.4 [봇 설정 (bot-config.json)](#124-봇-설정-bot-configjson)
    - 12.5 [환경 변수](#125-환경-변수)
13. [업데이트 시스템](#13-업데이트-시스템)
    - 13.1 [자동 업데이트](#131-자동-업데이트)
    - 13.2 [수동 업데이트](#132-수동-업데이트)
    - 13.3 [무결성 검증](#133-무결성-검증)
14. [릴레이 서버 (클라우드 모드)](#14-릴레이-서버-클라우드-모드)
15. [REST API 레퍼런스](#15-rest-api-레퍼런스)
    - 15.1 [인증](#151-인증)
    - 15.2 [서버 API](#152-서버-api)
    - 15.3 [인스턴스 API](#153-인스턴스-api)
    - 15.4 [콘솔 API](#154-콘솔-api)
    - 15.5 [모듈 API](#155-모듈-api)
    - 15.6 [익스텐션 API](#156-익스텐션-api)
    - 15.7 [업데이트 API](#157-업데이트-api)
    - 15.8 [봇 / 클라이언트 API](#158-봇--클라이언트-api)
16. [다국어 지원 (i18n)](#16-다국어-지원-i18n)
17. [파일 시스템 경로](#17-파일-시스템-경로)
18. [문제 해결](#18-문제-해결)
19. [FAQ](#19-faq)

---

## 1. 소개

**Saba-chan** (サバちゃん / 사바쨩)은 여러 게임 서버를 통합적으로 관리할 수 있는 **모듈형 게임 서버 관리 플랫폼**입니다.

> 이름의 유래: 일본어로 "서버(サーバー)"와 "고등어(鯖, さば)"의 발음이 비슷한 것에서 착안한 말장난입니다.

### 핵심 특징

- **다중 게임 지원** — Palworld, Minecraft, Project Zomboid 등 모듈로 무한 확장 가능
- **자동 프로세스 감지** — 실행 중인 게임 서버를 자동으로 탐지
- **모듈 시스템** — 새 게임 추가 시 Core 재컴파일 불필요
- **이중 인터페이스** — Electron GUI 데스크톱 앱 + Rust TUI CLI
- **안전한 설계** — Daemon 크래시가 게임 서버에 영향을 주지 않음
- **자동 업데이트** — GitHub Releases 기반 자동 업데이트 시스템
- **Discord 봇 통합** — 디스코드에서 서버를 직접 제어
- **익스텐션 시스템** — Docker 격리, SteamCMD 자동화 등 범용 확장 기능
- **10개 언어 지원** — 한국어, 영어, 일본어, 독일어, 스페인어, 프랑스어, 포르투갈어(BR), 러시아어, 중국어(간체/번체)

---

## 2. 시스템 요구사항

| 항목 | 요구사항 |
|------|---------|
| **OS** | Windows 10 / 11 |
| **Rust** | 최신 stable (소스 빌드 시) |
| **Node.js** | 18.0 이상 (GUI 및 Discord 봇) |
| **Python** | 3.x (모듈 lifecycle 실행용) |
| **디스크** | 최소 500MB (게임 서버 제외) |
| **네트워크** | 게임 서버 포트 개방 필요 |

> **참고**: 인스톨러로 설치하는 경우 Rust는 필요하지 않습니다. Python과 Node.js는 사바쨩이 자동으로 포터블 환경을 부트스트랩합니다.

---

## 3. 설치

### 3.1 인스톨러를 이용한 설치

사바쨩은 독립형 설치 프로그램(Tauri 기반)을 제공합니다.

1. [GitHub Releases](https://github.com/WareAoba/saba-chan/releases)에서 최신 인스톨러를 다운로드합니다.
2. 인스톨러를 실행하고 설치 경로를 지정합니다.
3. 설치가 완료되면 바탕화면 또는 시작 메뉴에서 Saba-chan을 실행합니다.

### 3.2 소스에서 빌드

#### 저장소 클론

```bash
git clone https://github.com/WareAoba/saba-chan.git
cd saba-chan
```

#### 전체 빌드 (Windows PowerShell)

```powershell
.\scripts\build-windows.ps1
```

이 스크립트는 3개의 빌드 작업을 **병렬**로 실행합니다:

1. **Rust workspace** — `cargo build --release --workspace`
   - 출력: `saba-core.exe`, `saba-chan-cli.exe`, `saba-chan-updater.exe`, `saba-chan-installer.exe`
2. **Electron GUI** — `npm run build` (Vite) → `npm run package` (electron-builder)
   - 출력: `Saba-chan.exe`
3. **Discord Bot** — `npm install --omit=dev` → 디렉토리 복사

#### 개별 빌드

```bash
# Core Daemon만 빌드
cargo build --release

# GUI 개발 모드
cd saba-chan-gui
npm install
npm start

# CLI만 빌드
cargo build --release -p saba-chan-cli
```

---

## 4. 빠른 시작

### Step 1: 사바쨩 실행

GUI(`Saba-chan.exe`) 또는 CLI(`saba-chan-cli.exe`)를 실행합니다. Core Daemon이 자동으로 시작됩니다.

### Step 2: 인스턴스 생성

1. GUI에서 **"+ 서버 추가"** 버튼을 클릭하거나, CLI에서 `:instance create` 명령어를 입력합니다.
2. 관리할 게임을 선택합니다 (예: Palworld, Minecraft).
3. 인스턴스 이름과 서버 실행 파일 경로를 설정합니다.

### Step 3: 서버 설정

- 서버 실행 파일(`executable_path`)과 작업 디렉토리(`working_dir`)를 지정합니다.
- 필요에 따라 포트, RCON 비밀번호 등을 설정합니다.

### Step 4: 서버 시작

GUI에서 **시작(▶)** 버튼을 클릭하거나, CLI에서 해당 서버를 선택 후 시작합니다.

### Step 5: 모니터링

- 실시간 서버 상태 확인
- 콘솔 출력 확인 및 명령어 입력
- Discord 봇을 통한 원격 제어 (선택)

---

## 5. 아키텍처 개요

### 5.1 전체 구조

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
│  (discord.js)   │           │  (게임별)    │     │ (서버 설정)  │
└────────┬────────┘           └─────────────┘     └─────────────┘
         │                           │
         │ IPC                ┌──────▼──────┐
         └──────────────────► │ Extensions  │
                              │ (Docker 등) │
                              └─────────────┘
```

### 5.2 Core Daemon

Core Daemon(`saba-core.exe`)은 사바쨩의 핵심으로, 백그라운드에서 실행되며 모든 서버 관리 기능을 제공합니다.

| 내부 모듈 | 역할 |
|-----------|------|
| `supervisor/` | 프로세스 관리 (시작/정지/모니터링), 모듈 로더, 상태 머신 |
| `ipc/` | Axum HTTP API 서버 (포트 57474) |
| `instance/` | 디렉토리 기반 인스턴스 저장소 |
| `plugin/` | Python lifecycle 모듈 실행 |
| `protocol/` | RCON (Valve Source) + REST API 클라이언트 |
| `extension/` | 익스텐션 시스템 (manifest.json 파싱, Hook 디스패치) |
| `config/` | 전역 설정 관리 |
| `validator/` | 설정값 검증, 포트 충돌 검사 |
| `python_env/` | Python venv 자동 부트스트랩 |
| `node_env/` | Node.js 포터블 환경 자동 다운로드 |

### 5.3 통신 방식

#### Daemon ↔ GUI/CLI

- 프로토콜: HTTP REST API
- 기본 주소: `http://127.0.0.1:57474`
- 인증: `.ipc_token` 파일 기반 토큰 (`X-Saba-Token` 헤더)

#### Daemon ↔ 모듈

1. 데몬이 `lifecycle.py <function_name>`을 자식 프로세스로 실행
2. config JSON을 **stdin**으로 전달
3. 모듈은 결과를 **stdout**에 JSON으로 출력
4. 로그는 **stderr**로 출력

#### 클라이언트 Watchdog

- GUI/CLI가 데몬에 등록(`POST /api/client/register`)하고, 30초마다 하트비트를 전송합니다.
- 모든 클라이언트 연결이 끊기면 → 15초 Grace Period → 재접속 시도 → 60초 내 재접속 없으면 데몬 자체 종료

| 상수 | 값 | 설명 |
|------|----|------|
| `DEFAULT_IPC_PORT` | 57474 | 기본 IPC 서버 포트 |
| `MONITOR_INTERVAL_SECS` | 2 | 프로세스 모니터링 주기 |
| `HEARTBEAT_REAPER_INTERVAL_SECS` | 30 | 클라이언트 생존 확인 주기 |
| `STOP_COOLDOWN_SECS` | 30 | Stop 후 auto-detect 억제 시간 |

---

## 6. GUI 사용법

사바쨩 GUI는 **Electron 28 + React 18 + Vite + Zustand**으로 구축된 데스크톱 애플리케이션입니다.

### 6.1 메인 화면

GUI를 실행하면 로딩 화면이 표시되며 Core Daemon에 연결될 때까지 대기합니다. 연결이 완료되면 **대시보드**(메인 화면)가 나타납니다.

메인 화면에는 등록된 모든 서버 인스턴스가 **카드** 형태로 표시됩니다. 각 카드에는 다음 정보가 포함됩니다:
- 서버 이름 및 게임 종류
- 현재 상태 (실행 중 / 정지 / 오류)
- 시작/정지 버튼
- 익스텐션 뱃지 (Docker 등)

### 6.2 인스턴스 관리

#### 새 인스턴스 생성

1. 메인 화면에서 **"+ 서버 추가"** 버튼 클릭
2. **게임 선택**: 설치된 모듈 목록에서 게임을 선택
3. **설정 입력**: 인스턴스 이름, 서버 실행 파일 경로, 포트 등 설정
4. **생성 완료**: 메인 화면에 새 카드가 추가됨

#### 서버 제어

- **시작**: 카드의 ▶ 버튼 클릭
- **정지**: 카드의 ■ 버튼 클릭
- **설정**: 카드 클릭 → 설정 모달
- **삭제**: 컨텍스트 메뉴에서 삭제

#### 설정 편집

서버 카드를 클릭하면 **설정 모달**이 열립니다. 각 모듈이 정의한 설정 필드가 표시되며, 타입에 따라 적절한 입력 UI가 제공됩니다:

| 필드 타입 | UI |
|-----------|-----|
| `text` | 텍스트 입력 |
| `number` | 숫자 입력 (min/max/step) |
| `boolean` | 토글 스위치 |
| `select` | 드롭다운 |
| `password` | 마스킹된 입력 |
| `file` | 파일 선택 다이얼로그 |
| `folder` | 폴더 선택 다이얼로그 |

설정 필드는 그룹으로 분류됩니다:
- **기본** (빈 문자열) — 일반 설정
- **saba-chan** — 사바쨩 전용 설정
- **advanced** — 고급 설정 (기본적으로 접힌 상태)

### 6.3 서버 콘솔

**콘솔 뷰**에서 서버의 실시간 stdout/stderr 출력을 확인하고, 서버에 직접 명령어를 입력할 수 있습니다.

- **ConsoleView**: 메인 화면 하단 도킹 패널
- **ConsoleWindow**: 독립 팝업 창으로 분리 가능
- 모듈의 `syntax_highlight` 설정에 따라 게임별 구문 강조가 적용됩니다.

### 6.4 설정

**설정 모달** (톱니바퀴 아이콘)에서 전역 설정을 변경할 수 있습니다:

| 탭 | 설정 항목 |
|----|----------|
| **일반** | 언어, 자동 새로고침, 새로고침 간격 |
| **외관** | UI 테마 관련 설정 |
| **고급** | IPC 포트, 콘솔 버퍼 크기, 비밀번호 자동 생성, 포트 충돌 검사 |
| **익스텐션** | 설치된 익스텐션 관리 |

---

## 7. CLI 사용법

사바쨩 CLI(`saba-chan-cli.exe`)는 **Ratatui + Crossterm** 기반의 대화형 터미널 인터페이스(TUI)입니다.

### 7.1 TUI 화면 구성

CLI는 여러 화면(Screen) 간을 탐색하는 구조입니다:

| 화면 | 설명 |
|------|------|
| **Dashboard** | 메인 대시보드 — 데몬/봇 상태, 서버 목록 요약 |
| **Servers** | 인스턴스 목록 |
| **ServerDetail** | 인스턴스 상세 — 상태, 시작/정지/재시작/삭제 |
| **ServerConsole** | 서버 콘솔 — stdin/stdout 실시간 |
| **ServerSettings** | 서버 설정 에디터 (vim-like) |
| **ServerProperties** | 서버 프로퍼티 파일 편집 |
| **Modules** | 설치된 모듈 목록 |
| **ModuleDetail** | 모듈 상세 정보 |
| **ModuleManifest** | 원격 모듈 매니페스트 (설치 가능한 모듈) |
| **Bot** | Discord 봇 관리 |
| **BotAliases** | 봇 별칭 편집 |
| **Settings** | CLI/GUI 설정 |
| **Updates** | 업데이트 관리 |
| **Daemon** | Core Daemon 관리 |
| **Extensions** | 익스텐션 관리 |
| **ExtensionList** | 설치된 익스텐션 목록 |
| **ExtensionDetail** | 익스텐션 상세 |
| **ExtensionManifest** | 원격 익스텐션 매니페스트 |
| **CreateInstanceStep1** | 인스턴스 생성 — 게임 선택 |
| **CreateInstanceStep2** | 인스턴스 생성 — 설정 입력 |

### 7.2 입력 모드

| 모드 | 설명 | 진입/종료 |
|------|------|----------|
| **Normal** | 메뉴 탐색 | 기본 모드 |
| **Command** | 레거시 명령어 입력 | `:` 키로 진입 |
| **Editing** | vim-like 필드 편집 | 설정 화면에서 Enter |
| **Console** | 서버 콘솔 stdin 입력 | 콘솔 화면에서 자동 |
| **Confirm** | 확인 대화상자 | y/n |
| **InlineInput** | 인라인 텍스트 입력 | 특정 동작 시 |
| **InlineSelect** | 인라인 선택 | 특정 동작 시 |

#### 기본 키 바인딩

| 키 | 동작 |
|----|------|
| `↑` / `k` | 위로 이동 |
| `↓` / `j` | 아래로 이동 |
| `Enter` | 선택 / 확인 |
| `Esc` | 뒤로 가기 / 취소 |
| `:` | 커맨드 모드 진입 |
| `q` | 종료 |

### 7.3 레거시 커맨드 모드

`:` 키를 누르면 레거시 커맨드 모드에 진입합니다. 텍스트 기반으로 명령어를 직접 입력할 수 있습니다.

```
: <명령어> [서브커맨드] [인자...]
```

### 7.4 CLI 명령어 레퍼런스

#### 인스턴스 관리

| 명령어 | 설명 |
|--------|------|
| `instance list` | 인스턴스 목록 |
| `instance create` | 새 인스턴스 생성 |
| `instance delete <id>` | 인스턴스 삭제 |
| `instance set <id> <key> <value>` | 설정 변경 |
| `instance reset <id>` | 설정 초기화 |
| `instance reorder` | 인스턴스 순서 변경 |

#### 모듈 관리

| 명령어 | 설명 |
|--------|------|
| `module list` | 설치된 모듈 목록 |
| `module info <name>` | 모듈 상세 정보 |
| `module refresh` | 모듈 새로고침 |
| `module versions <name>` | 버전 목록 |
| `module install <name>` | 서버 설치 |
| `module manifest` | 원격 매니페스트 조회 |
| `module install-manifest <id>` | 매니페스트에서 모듈 설치 |
| `module remove <name>` | 모듈 삭제 |

#### 익스텐션 관리

| 명령어 | 설명 |
|--------|------|
| `extension list` / `ext list` | 익스텐션 목록 |
| `extension enable <id>` | 활성화 |
| `extension disable <id>` | 비활성화 |
| `extension install <id>` | 설치 |
| `extension remove <id>` | 삭제 |
| `extension manifest` | 원격 매니페스트 조회 |
| `extension rescan` | 재스캔 |

#### 데몬 관리

| 명령어 | 설명 |
|--------|------|
| `daemon start` | 데몬 시작 |
| `daemon stop` | 데몬 정지 |
| `daemon status` | 데몬 상태 |
| `daemon restart` | 데몬 재시작 |

#### Discord 봇 관리

| 명령어 | 설명 |
|--------|------|
| `bot start` | 봇 시작 |
| `bot stop` | 봇 정지 |
| `bot status` | 봇 상태 |
| `bot token <token>` | 봇 토큰 설정 |
| `bot prefix <prefix>` | 봇 prefix 설정 |
| `bot mode <local\|cloud>` | 동작 모드 변경 |
| `bot relay <url>` | 릴레이 서버 URL 설정 |
| `bot node-token <token>` | 릴레이 노드 토큰 설정 |

#### 설정

| 명령어 | 설명 |
|--------|------|
| `config show` | 현재 설정 표시 |
| `config set <key> <value>` | CLI 설정 변경 |
| `config get <key>` | 설정 값 조회 |
| `config reset` | 설정 초기화 |
| `config system-language` | 시스템 언어 감지 |

##### GUI 설정 변경 (CLI에서)

| 명령어 | 설명 |
|--------|------|
| `config gui language <lang>` | GUI 언어 |
| `config gui token <token>` | Discord 토큰 |
| `config gui discord_auto <bool>` | Discord 자동 시작 |
| `config gui auto_refresh <bool>` | 자동 새로고침 |
| `config gui refresh_interval <ms>` | 새로고침 간격 |
| `config gui ipc_port <port>` | IPC 포트 |
| `config gui console_buffer <n>` | 콘솔 버퍼 크기 |
| `config gui auto_passwords <bool>` | 비밀번호 자동 생성 |
| `config gui port_check <bool>` | 포트 충돌 검사 |

#### 업데이트

| 명령어 | 설명 |
|--------|------|
| `update check` | 업데이트 확인 |
| `update status` | 업데이트 상태 |
| `update download` | 업데이트 다운로드 |
| `update apply` | 업데이트 적용 |
| `update config` | 업데이터 설정 조회 |
| `update set <key> <value>` | 업데이터 설정 변경 |

#### 기타

| 명령어 | 설명 |
|--------|------|
| `help` | 도움말 |
| `menu` / `dashboard` | 대시보드로 이동 |
| `back` | 이전 화면 |
| `exit` / `quit` / `q` | 종료 (봇/데몬도 정지) |
| `<모듈명> <명령어>` | 모듈 명령어 직접 실행 |

---

## 8. 인스턴스 관리

### 8.1 인스턴스란?

**인스턴스**는 하나의 게임 서버 설정 단위입니다. 같은 게임의 서버를 여러 개 생성할 수 있으며, 각 인스턴스는 독립적인 설정을 가집니다.

인스턴스 데이터는 디렉토리 기반으로 저장됩니다:

```
instances/
├── order.json                  ← 인스턴스 정렬 순서
├── <uuid>/
│   ├── instance.json           ← 메타데이터 (id, name, module, ports…)
│   └── settings.json           ← 모듈별 동적 게임 설정
└── <uuid>/
    ├── instance.json
    └── settings.json
```

### 8.2 인스턴스 생성

#### GUI에서 생성

1. **"+ 서버 추가"** 버튼 클릭
2. 게임 선택 (설치된 모듈 목록)
3. 필수 설정 입력:
   - **인스턴스 이름**: 구분용 이름
   - **서버 실행 파일 경로** (`executable_path`): 게임 서버 실행 파일의 전체 경로
   - **작업 디렉토리** (`working_dir`): 서버가 실행될 디렉토리
4. 선택적 설정:
   - 포트 번호
   - RCON/REST 비밀번호 (자동 생성 가능)
   - 게임별 추가 설정

#### CLI에서 생성

```
:instance create
```

인스턴스 생성 위자드(Step 1 → Step 2)가 시작됩니다.

### 8.3 인스턴스 설정

각 인스턴스는 두 가지 설정 파일을 가집니다:

- **instance.json** — 인스턴스 메타데이터 (이름, 모듈, 포트, 프로토콜 정보)
- **settings.json** — 모듈이 정의한 동적 게임 설정 (실행 파일 경로, 게임별 옵션 등)

설정 키는 모듈의 `module.toml` 파일에서 `[[settings.fields]]`로 정의됩니다.

### 8.4 서버 시작 / 정지 / 재시작

#### 시작 흐름

1. 인스턴스 설정 로드
2. 모듈의 `get_launch_command()` 호출 → 시작 명령어 생성
3. ManagedProcess로 서버 시작 (stdin/stdout 캡처)
4. 프로세스 모니터링 시작 (2초 간격)

#### 정지 흐름

1. 모듈의 `stop_server()` 호출 (RCON/REST/stdin 등 프로토콜별 정지)
2. 프로세스 종료 확인
3. 30초간 자동 감지 억제 (재감지 방지)

### 8.5 서버 프로퍼티 편집

일부 모듈은 게임 서버의 프로퍼티 파일(예: `server.properties`)을 직접 편집할 수 있는 기능을 제공합니다.

- GUI: **서버 프로퍼티** 탭
- CLI: **ServerProperties** 화면

### 8.6 서버 명령어 실행

서버가 실행 중일 때 게임 서버에 직접 명령어를 보낼 수 있습니다:

| 방식 | 설명 |
|------|------|
| **콘솔 stdin** | 콘솔 화면에서 직접 입력 (Console 모드) |
| **RCON** | Valve Source RCON 프로토콜 |
| **REST API** | HTTP REST 요청 (Palworld 등) |

모듈이 지원하는 프로토콜에 따라 사용 가능한 방식이 달라집니다.

---

## 9. 모듈 시스템

### 9.1 모듈이란?

모듈은 특정 게임 서버의 생명주기를 관리하는 **Python 스크립트 + TOML 설정 파일**의 묶음입니다. Core Daemon은 모듈을 다이나믹하게 로드하므로, 새 게임을 추가할 때 Core를 재컴파일할 필요가 없습니다.

### 9.2 지원 게임

| 게임 | 모듈 이름 | 프로토콜 | 상호작용 모드 | 설치 방식 | 기본 포트 |
|------|-----------|----------|--------------|-----------|-----------|
| **Palworld** | `palworld` | REST API | `commands` | SteamCMD (App ID: 2394010) | 8211 |
| **Minecraft** | `minecraft` | RCON + stdin | `console` | 공식 다운로드 (mojang.com) | 25565 |
| **Project Zomboid** | `zomboid` | RCON + stdin | `console` | SteamCMD (App ID: 380870) | 16261 |

> **참고**: 커뮤니티 및 직접 개발한 모듈을 추가로 설치하여 더 많은 게임을 지원할 수 있습니다.

### 9.3 모듈 설치 및 관리

#### 원격 매니페스트에서 설치

```
:module manifest             # 사용 가능한 모듈 목록 확인
:module install-manifest <id> # 원격에서 모듈 다운로드 및 설치
```

GUI에서는 **모듈** 탭 → **매니페스트** 버튼을 통해 접근할 수 있습니다.

#### 모듈 새로고침

모듈 파일을 수동으로 변경한 후 적용하려면:

```
:module refresh
```

#### 모듈 삭제

```
:module remove <name>
```

### 9.4 모듈 구조

```
mygame/
├── module.toml          # 필수 — 메타데이터 및 설정 스키마
├── lifecycle.py         # 필수 — 생명주기 관리 Python 스크립트
├── icon.png             # 권장 — GUI에 표시될 아이콘
└── locales/             # 권장 — 다국어 번역 파일
    ├── en.json          #   필수 (기본 언어)
    ├── ko.json
    ├── ja.json
    └── ...
```

#### module.toml 주요 섹션

| 섹션 | 설명 |
|------|------|
| `[module]` | 메타데이터 — name, version, description, game_name, display_name, entry, icon |
| `[update]` | 자동 업데이트 — github_repo |
| `[protocols]` | 통신 프로토콜 — supported, default, interaction_mode |
| `[credential_map]` | 자격증명 동기화 — 데몬 키 ↔ 게임 키 매핑 |
| `[config]` | 기본 실행 설정 — executable_path, process_name, default_port, stop_command |
| `[install]` | 서버 설치 — method (steamcmd/download/manual), app_id, download_url |
| `[docker]` | Docker 컨테이너 설정 — image, ports, volumes, environment |
| `[detection]` | 자동 감지 — process_patterns, cmd_patterns, common_paths |
| `[settings]` | GUI/CLI 설정 필드 정의 |
| `[commands]` | 서버 명령어 정의 |
| `[errors]` | 에러 메시지 |
| `[aliases]` | 별칭 |
| `[syntax_highlight]` | 콘솔 구문 강조 |

#### lifecycle.py 함수

| 함수 | 필수 | 설명 |
|------|------|------|
| `get_launch_command(config)` | ✅ | 서버 시작 명령어 반환 |
| `get_status(config)` | ✅ | 서버 상태 조회 |
| `stop_server(config)` | ✅ | 서버 정지 |
| `execute_command(config)` | | 서버 명령어 실행 |
| `get_available_versions(config)` | | 사용 가능한 버전 목록 |
| `install_server(config)` | | 서버 설치 |
| `read_settings(config)` | | 서버 설정 파일 읽기 |
| `write_settings(config)` | | 서버 설정 파일 쓰기 |
| `diagnose(config)` | | 서버 진단 |

> **모듈 개발에 대한 자세한 내용**: [모듈 개발 가이드](../saba-chan-modules/docs/module-development-guide.md) 참조

---

## 10. 익스텐션 시스템

### 10.1 익스텐션이란?

익스텐션은 모듈과 달리 **특정 게임에 종속되지 않는 범용 확장 기능**입니다. Hook 시스템을 통해 서버 생명주기의 다양한 지점에 개입할 수 있으며, GUI/CLI에 UI 슬롯을 주입할 수 있습니다.

### 10.2 내장 익스텐션

| ID | 이름 | 설명 | GUI 포함 |
|----|------|------|---------|
| `docker` | Docker Isolation | Docker 컨테이너로 게임 서버를 격리하여 실행 | ✅ |
| `steamcmd` | SteamCMD | SteamCMD 기반 서버 설치/업데이트 자동화 | ❌ |
| `music` | Music Bot | Discord 음성 채널 음악 재생 (yt-dlp + ffmpeg) | ❌ |
| `ue4-ini` | UE4 INI Parser | Unreal Engine 4 INI OptionSettings 파싱 | ❌ |

### 10.3 익스텐션 관리

#### 활성화 / 비활성화

GUI 설정 → 익스텐션 탭에서 토글하거나, CLI에서:

```
:ext enable <id>
:ext disable <id>
```

#### 설치 / 삭제

```
:ext manifest              # 원격 매니페스트에서 설치 가능한 익스텐션 확인
:ext install <id>          # 설치
:ext remove <id>           # 삭제
:ext rescan                # 익스텐션 디렉토리 재스캔
```

### Hook 시스템

익스텐션은 다음 Hook 포인트에서 실행될 수 있습니다:

#### 데몬 수명주기

| Hook | 시점 |
|------|------|
| `daemon.startup` | 데몬 시작 시 |
| `daemon.shutdown` | 데몬 종료 시 |

#### 서버 수명주기

| Hook | 시점 |
|------|------|
| `server.pre_create` | 인스턴스 생성 전 |
| `server.post_create` | 인스턴스 생성 후 |
| `server.pre_start` | 서버 시작 전 |
| `server.post_stop` | 서버 정지 후 |
| `server.pre_delete` | 인스턴스 삭제 전 |
| `server.status` | 상태 조회 시 |
| `server.stats` | 통계 조회 시 |
| `server.settings_changed` | 설정 변경 시 |
| `server.list_enrich` | 서버 목록 정보 보강 시 |
| `server.logs` | 로그 조회 시 |
| `server.install` | 서버 설치 시 |
| `server.update` | 서버 업데이트 시 |
| `server.check_update` | 업데이트 확인 시 |

### GUI/CLI 슬롯

익스텐션은 다음 위치에 UI를 주입할 수 있습니다:

| GUI 슬롯 | CLI 슬롯 | 용도 |
|----------|---------|------|
| `ServerCard.badge` | `InstanceList.badge` | 인스턴스 목록 뱃지 |
| `ServerCard.headerGauge` | — | 헤더 게이지 |
| `ServerCard.expandedStats` | `InstanceDetail.status` | 확장 통계 |
| `ServerCard.provision` | — | 프로비저닝 상태 |
| `ServerSettings.tab` | `InstanceSettings.fields` | 설정 탭/필드 |
| `AddServer.options` | `CreateInstance.options` | 인스턴스 생성 옵션 |

---

## 11. Discord 봇

### 11.1 개요

사바쨩에는 Discord 봇이 내장되어 있어, 디스코드 채팅에서 직접 게임 서버를 제어할 수 있습니다. **discord.js 14** 기반으로 구현되어 있습니다.

### 11.2 봇 설정

#### Discord 봇 토큰 설정

1. [Discord Developer Portal](https://discord.com/developers/applications)에서 봇을 생성합니다.
2. 봇 토큰을 복사합니다.
3. GUI 설정 또는 CLI에서 토큰을 등록합니다:
   - GUI: 설정 → Discord 봇 → 토큰 입력
   - CLI: `:bot token <YOUR_TOKEN>`

#### 봇 초대

Discord 봇을 서버에 초대하려면 적절한 권한(메시지 읽기/쓰기)을 가진 OAuth2 URL을 생성하여 초대합니다.

#### 봇 시작/정지

- GUI: 타이틀바의 봇 아이콘 클릭
- CLI: `:bot start` / `:bot stop`
- 자동 시작: 설정에서 `discordAutoStart: true`로 설정

### 11.3 동작 모드

| 모드 | 설명 | 조건 |
|------|------|------|
| **로컬 (local)** | Discord에 직접 로그인하여 메시지를 처리하고, 로컬 데몬 IPC로 명령 실행 | 기본 모드 |
| **클라우드 (cloud)** | 릴레이 서버를 통해 중계. Discord 로그인 없이 릴레이 서버 폴링 | `RELAY_URL` + `RELAY_NODE_TOKEN` 설정 시 |

**클라우드 모드**는 포트 포워딩이 불가능한 환경(NAT 뒤)에서도 Discord 봇을 사용할 수 있게 해줍니다.

### 11.4 봇 명령어

명령어 형식:

```
<prefix> <모듈명> <명령어> [인자...]
```

기본 prefix는 `사바쨩`입니다.

#### 내장 명령어

| 명령어 | 설명 |
|--------|------|
| `<prefix>` 또는 `<prefix> help` | 도움말 표시 |
| `<prefix> list` / `<prefix> 목록` | 서버 목록 |

#### 모듈 명령어

| 명령어 | 설명 |
|--------|------|
| `<prefix> <모듈> start` | 서버 시작 |
| `<prefix> <모듈> stop` | 서버 정지 |
| `<prefix> <모듈> status` | 서버 상태 확인 |
| `<prefix> <모듈> <명령어> [인자]` | 모듈별 명령어 실행 |

예시:
```
사바쨩 palworld status
사바쨩 minecraft start
사바쨩 zomboid stop
```

### 11.5 별칭 시스템

명령어를 간소화하기 위한 별칭 시스템이 지원됩니다:

- **모듈 별칭**: `bot-config.json`의 `moduleAliases` + 모듈 `[aliases]` 섹션
- **명령어 별칭**: `bot-config.json`의 `commandAliases` + 모듈별 별칭

예시 (`bot-config.json`):
```json
{
  "moduleAliases": {
    "팔": "palworld",
    "마크": "minecraft"
  },
  "commandAliases": {
    "켜": "start",
    "꺼": "stop"
  }
}
```

이렇게 설정하면:
```
사바쨩 팔 켜         → 사바쨩 palworld start
사바쨩 마크 꺼       → 사바쨩 minecraft stop
```

별칭 설정 파일이 변경되면 **핫 리로드**됩니다 (봇 재시작 불필요).

#### 길드(노드)별 설정

`bot-config.json`의 `nodeSettings`로 Discord 길드별 인스턴스를 필터링할 수 있습니다.

### 11.6 봇 익스텐션

| 익스텐션 | 설명 |
|----------|------|
| **music.js** | 음악 재생 (로컬 모드 전용, yt-dlp + ffmpeg 필요) |
| **easter_eggs.js** | 이스터 에그 |
| **rps.js** | 가위바위보 미니게임 |

---

## 12. 설정 레퍼런스

### 12.1 전역 설정 (global.toml)

위치: `config/global.toml`

```toml
ipc_socket = "./ipc.sock"
# log_buffer_size = 10000       # 서버당 최대 로그 줄 수

[updater]
enabled = true
check_interval_hours = 3         # 업데이트 확인 간격 (시간 단위)
auto_download = false            # 새 버전 발견 시 자동 다운로드
auto_apply = false               # 다운로드 완료 시 자동 교체
github_owner = "WareAoba"
github_repo = "saba-chan"
include_prerelease = false       # 프리릴리스 포함 여부
# install_root = "."             # 설치 루트 디렉터리
```

### 12.2 GUI 설정 (settings.json)

위치: `%APPDATA%/saba-chan/settings.json`

| 키 | 타입 | 기본값 | 설명 |
|----|------|--------|------|
| `language` | string | `"en"` | 표시 언어 |
| `discordToken` | string | `""` | Discord 봇 토큰 |
| `discordAutoStart` | bool | `false` | Discord 봇 자동 시작 |
| `autoRefresh` | bool | `true` | 상태 자동 새로고침 |
| `refreshInterval` | number | `2000` | 새로고침 간격 (ms) |
| `ipcPort` | number | `57474` | IPC 포트 번호 (1024~65535) |
| `consoleBuffer` | number | `2000` | 콘솔 버퍼 크기 |
| `autoGeneratePasswords` | bool | `true` | 빈 RCON/REST 비밀번호 자동 생성 |
| `portConflictCheck` | bool | `true` | 포트 충돌 검사 활성화 |

### 12.3 CLI 설정 (cli-settings.json)

위치: `%APPDATA%/saba-chan/cli-settings.json`

| 키 | 별칭 | 타입 | 기본값 | 설명 |
|----|------|------|--------|------|
| `language` | `lang` | string | `""` | 표시 언어 (비어있으면 GUI 설정 따름) |
| `auto_start` | `autostart` | bool | `true` | TUI 시작 시 데몬/봇 자동 시작 |
| `refresh_interval` | `refresh` | u64 | `2` | 상태 새로고침 간격 (초, 1~60) |
| `bot_prefix` | `prefix` | string | `""` | Discord 봇 prefix 오버라이드 |

### 12.4 봇 설정 (bot-config.json)

위치: `%APPDATA%/saba-chan/bot-config.json`

```json
{
  "prefix": "사바쨩",
  "moduleAliases": {},
  "commandAliases": {},
  "musicEnabled": true,
  "mode": "local",
  "cloud": {
    "relayUrl": "",
    "hostId": ""
  },
  "nodeSettings": {}
}
```

| 키 | 타입 | 설명 |
|----|------|------|
| `prefix` | string | 봇 명령어 접두어 |
| `moduleAliases` | object | 모듈 별칭 매핑 (`{ "팔": "palworld" }`) |
| `commandAliases` | object | 명령어 별칭 매핑 (`{ "켜": "start" }`) |
| `musicEnabled` | bool | 음악 익스텐션 활성화 |
| `mode` | string | 동작 모드 (`"local"` / `"cloud"`) |
| `cloud.relayUrl` | string | 릴레이 서버 URL |
| `cloud.hostId` | string | 호스트 ID |
| `nodeSettings` | object | 길드별 설정 |

### 12.5 환경 변수

다음 환경 변수로 설정을 오버라이드할 수 있습니다:

| 변수 | 설명 |
|------|------|
| `SABA_IPC_PORT` | IPC 포트 오버라이드 |
| `SABA_MODULES_PATH` | 모듈 디렉토리 경로 |
| `SABA_INSTANCES_PATH` | 인스턴스 저장소 경로 |
| `SABA_LANG` | 표시 언어 |
| `SABA_TOKEN_PATH` | IPC 토큰 파일 경로 |
| `SABA_EXTENSIONS_DIR` | 익스텐션 디렉토리 경로 |
| `DISCORD_TOKEN` | Discord 봇 토큰 |
| `IPC_BASE` | IPC base URL |
| `BOT_CONFIG_PATH` | bot-config.json 경로 |
| `RELAY_URL` | 릴레이 서버 URL (클라우드 모드 활성화) |
| `RELAY_NODE_TOKEN` | 릴레이 노드 토큰 |

---

## 13. 업데이트 시스템

사바쨩은 GitHub Releases 기반의 자동 업데이트 시스템을 내장하고 있습니다.

### 13.1 자동 업데이트

업데이트 설정은 코드에 내장되어 있습니다:

| 설정 | 기본값 | 동작 |
|------|--------|------|
| `enabled` | `true` | 업데이트 확인 활성화 |
| `check_interval_hours` | `3` | 3시간 간격으로 자동 확인 |
| `auto_download` | `false` | 새 버전 발견 시 자동 다운로드 |
| `auto_apply` | `false` | 다운로드 완료 시 자동 적용 |

### 13.2 수동 업데이트

GUI의 **업데이트** 탭 또는 CLI에서:

```
:update check          # 업데이트 확인
:update download       # 다운로드
:update apply          # 적용
:update status         # 현재 상태 조회
```

### 업데이트 대상 컴포넌트

| 컴포넌트 | 설명 |
|----------|------|
| `saba-core` | Core Daemon |
| `cli` | CLI |
| `gui` | GUI |
| `updater` | Updater 자체 |
| `discord_bot` | Discord Bot |
| `locales` | 다국어 파일 |
| `module-{name}` | 개별 모듈 |
| `ext-{name}` | 개별 익스텐션 |

### 업데이트 흐름

1. GitHub Releases에서 최신 버전 정보 조회
2. `release-manifest.json`에서 각 컴포넌트의 버전/해시/URL 파싱
3. 로컬 설치 버전과 비교
4. 업데이트가 있으면 다운로드 (자동 또는 수동)
5. 적용 (모듈/익스텐션은 즉시 교체, 코어/CLI/GUI는 재시작 필요)

### 13.3 무결성 검증

다운로드된 파일은 **SHA256** 해시로 무결성을 검증합니다.

| 상태 | 설명 |
|------|------|
| `Verified` | 해시 일치 — 정상 |
| `Tampered` | 해시 불일치 — 변조 의심 |
| `NoHash` | 비교할 해시 없음 |
| `FileNotFound` | 파일을 찾을 수 없음 |
| `Error` | 검증 중 오류 |

---

## 14. 릴레이 서버 (클라우드 모드)

릴레이 서버(server-chan)는 **NAT 뒤에 있는 사바쨩 노드**와 **Discord 봇** 간의 통신을 중계하는 클라우드 서비스입니다.

### 아키텍처

```
Discord 사용자
    │
    ▼
릴레이 서버 (server-chan)    ← Fastify 5 + PostgreSQL 17
    │
    ▼ (Long Poll)
로컬 사바쨩 노드              ← 봇 (릴레이 에이전트 모드)
    │
    ▼ (IPC)
로컬 Core Daemon
    │
    ▼
결과 → 릴레이 서버 → Discord 응답
```

### 주요 원칙

- **중앙 서버는 payload를 해석하지 않음** (택배 기사 역할)
- **Pull 모델**: 노드가 릴레이 서버에 접속 (포트 포워딩 불필요)
- **IP 정보 미저장**: 프라이버시 보호
- **보안**: argon2id 토큰 해싱, HMAC-SHA256 요청 서명

### 클라우드 모드 설정

1. 릴레이 서버 URL과 인증 정보를 봇 설정에 등록:
   ```
   :bot mode cloud
   :bot relay https://relay.example.com
   :bot node-token <YOUR_NODE_TOKEN>
   ```
2. 봇을 시작하면 릴레이 에이전트 모드로 동작합니다.

---

## 15. REST API 레퍼런스

Core Daemon은 `http://127.0.0.1:57474`에서 HTTP REST API를 제공합니다.

### 15.1 인증

모든 API 요청에는 `X-Saba-Token` 헤더가 필요합니다.

```
X-Saba-Token: <.ipc_token 파일의 내용>
```

토큰 파일 위치: `%APPDATA%/saba-chan/.ipc_token`

401 응답을 받으면 토큰 파일을 다시 읽어 갱신 후 1회 재시도합니다.

### 15.2 서버 API (런타임)

| Method | Endpoint | 설명 |
|--------|----------|------|
| `GET` | `/api/servers` | 서버 런타임 상태 목록 |
| `GET` | `/api/server/{name}/status` | 서버 상태 조회 |
| `POST` | `/api/server/{name}/start` | 서버 시작 |
| `POST` | `/api/server/{name}/stop` | 서버 정지 |

### 15.3 인스턴스 API (설정)

| Method | Endpoint | 설명 |
|--------|----------|------|
| `GET` | `/api/instances` | 인스턴스 목록 |
| `GET` | `/api/instance/{id}` | 인스턴스 상세 |
| `POST` | `/api/instances` | 인스턴스 생성 |
| `PATCH` | `/api/instance/{id}` | 인스턴스 설정 업데이트 |
| `DELETE` | `/api/instance/{id}` | 인스턴스 삭제 |
| `PUT` | `/api/instances/reorder` | 인스턴스 순서 변경 |
| `POST` | `/api/instance/{id}/validate` | 설정 검증 |
| `GET` | `/api/instance/{id}/properties` | 서버 프로퍼티 읽기 |
| `PUT` | `/api/instance/{id}/properties` | 서버 프로퍼티 쓰기 |
| `POST` | `/api/instance/{id}/accept-eula` | EULA 수락 |
| `POST` | `/api/instance/{id}/diagnose` | 서버 진단 |
| `POST` | `/api/instance/{id}/server/reset` | 서버 리셋 |
| `POST` | `/api/instance/{id}/properties/reset` | 프로퍼티 리셋 |

### 15.4 콘솔 API

| Method | Endpoint | 설명 |
|--------|----------|------|
| `POST` | `/api/instance/{id}/managed/start` | 관리형 서버 시작 (stdin/stdout 캡처) |
| `GET` | `/api/instance/{id}/console` | 콘솔 출력 가져오기 |
| `POST` | `/api/instance/{id}/stdin` | stdin 텍스트 전송 |

### 15.5 모듈 API

| Method | Endpoint | 설명 |
|--------|----------|------|
| `GET` | `/api/modules` | 모듈 목록 |
| `GET` | `/api/module/{name}` | 모듈 상세 정보 |
| `POST` | `/api/modules/refresh` | 모듈 새로고침 |
| `GET` | `/api/module/{name}/versions` | 버전 목록 |
| `POST` | `/api/module/{name}/install` | 서버 설치 |
| `GET` | `/api/modules/manifest` | 원격 모듈 매니페스트 |
| `POST` | `/api/modules/manifest/{id}/install` | 매니페스트에서 모듈 설치 |
| `DELETE` | `/api/modules/{id}` | 모듈 삭제 |

### 15.6 익스텐션 API

| Method | Endpoint | 설명 |
|--------|----------|------|
| `GET` | `/api/extensions` | 익스텐션 목록 |
| `POST` | `/api/extensions/{id}/enable` | 활성화 |
| `POST` | `/api/extensions/{id}/disable` | 비활성화 |
| `POST` | `/api/extensions/{id}/install` | 설치 |
| `DELETE` | `/api/extensions/{id}` | 삭제 |
| `GET` | `/api/extensions/manifest` | 원격 매니페스트 |
| `GET` | `/api/extensions/updates` | 업데이트 확인 |
| `POST` | `/api/extensions/rescan` | 재스캔 |
| `GET` | `/api/extensions/init-status` | 초기화 상태 |

### 15.7 업데이트 API

| Method | Endpoint | 설명 |
|--------|----------|------|
| `POST` | `/api/updates/check` | 업데이트 수동 확인 |
| `GET` | `/api/updates/status` | 업데이트 상태 |
| `POST` | `/api/updates/download` | 업데이트 다운로드 |
| `POST` | `/api/updates/apply` | 업데이트 적용 |
| `GET` | `/api/updates/config` | 업데이터 설정 조회 |
| `POST` | `/api/updates/config` | 업데이터 설정 변경 |
| `GET` | `/api/install/status` | 설치 상태 |
| `POST` | `/api/install/run` | 최초 설치 |
| `POST` | `/api/install/component/{key}` | 특정 컴포넌트 설치 |
| `GET` | `/api/install/progress` | 설치 진행 상태 |

### 15.8 봇 / 클라이언트 API

| Method | Endpoint | 설명 |
|--------|----------|------|
| `POST` | `/api/client/register` | 클라이언트 등록 |
| `POST` | `/api/client/{id}/heartbeat` | 하트비트 |
| `DELETE` | `/api/client/{id}/unregister` | 클라이언트 해제 |
| `GET` | `/api/config/bot` | 봇 설정 조회 |
| `PUT` | `/api/config/bot` | 봇 설정 저장 |
| `GET` | `/api/provision-progress/{name}` | 프로비저닝 상태 |
| `DELETE` | `/api/provision-progress/{name}` | 프로비저닝 해제 |

### API 테스트 예시 (PowerShell)

```powershell
# 서버 목록 조회
$token = Get-Content "$env:APPDATA\saba-chan\.ipc_token"
$headers = @{ "X-Saba-Token" = $token }
Invoke-RestMethod -Uri "http://127.0.0.1:57474/api/servers" -Headers $headers | ConvertTo-Json

# 인스턴스 생성
$body = @{
    name = "My Palworld Server"
    module = "palworld"
} | ConvertTo-Json
Invoke-RestMethod -Method POST -Uri "http://127.0.0.1:57474/api/instances" -Headers $headers -Body $body -ContentType "application/json"
```

---

## 16. 다국어 지원 (i18n)

사바쨩은 10개 언어를 지원합니다:

| 코드 | 언어 | 코드 | 언어 |
|------|------|------|------|
| `en` | English | `fr` | Français |
| `ko` | 한국어 | `pt-BR` | Português (Brasil) |
| `ja` | 日本語 | `ru` | Русский |
| `de` | Deutsch | `zh-CN` | 简体中文 |
| `es` | Español | `zh-TW` | 繁體中文 |

### 번역 파일 구조

```
locales/
├── en/
│   ├── gui.json         ← GUI 번역
│   ├── cli.json         ← CLI 번역
│   ├── bot.json         ← Discord 봇 번역
│   └── common.json      ← 공통 번역
├── ko/
│   ├── gui.json
│   ├── cli.json
│   ├── bot.json
│   └── common.json
└── ...
```

### 언어 설정 우선순위

1. 개별 설정 파일 (CLI: `cli-settings.json`, GUI: `settings.json`)
2. 환경 변수 `SABA_LANG`
3. 기본값 `"en"`

### 모듈 / 익스텐션 번역

- 모듈: `modules/{module}/locales/{lang}.json`
- 익스텐션: `extensions/{ext}/i18n/{lang}.json`

---

## 17. 파일 시스템 경로

Windows 기준:

| 경로 | 용도 |
|------|------|
| `%APPDATA%\saba-chan\` | 설정 및 데이터 루트 디렉토리 |
| `%APPDATA%\saba-chan\settings.json` | GUI 설정 |
| `%APPDATA%\saba-chan\cli-settings.json` | CLI 설정 |
| `%APPDATA%\saba-chan\bot-config.json` | Discord 봇 설정 |
| `%APPDATA%\saba-chan\.ipc_token` | IPC 인증 토큰 |
| `%APPDATA%\saba-chan\instances\` | 인스턴스 저장소 (디렉토리 기반) |
| `%APPDATA%\saba-chan\modules\` | 모듈 디렉토리 |
| `%APPDATA%\saba-chan\extensions\` | 익스텐션 디렉토리 |
| `config\global.toml` | 전역 설정 (앱 디렉토리 내) |

---

## 18. 문제 해결

### 데몬에 연결할 수 없음

**증상**: GUI 로딩 화면에서 멈추거나, CLI에서 연결 오류

**해결 방법**:
1. `saba-core.exe`가 실행 중인지 확인
2. IPC 포트(기본 57474)가 다른 프로그램에 의해 사용 중인지 확인
3. 방화벽이 로컬 연결을 차단하고 있지 않은지 확인
4. `.ipc_token` 파일이 존재하는지 확인

### 서버가 시작되지 않음

**증상**: 시작 버튼을 눌러도 서버가 실행되지 않음

**해결 방법**:
1. `executable_path`가 올바른 실행 파일을 가리키는지 확인
2. `working_dir`이 유효한 디렉토리인지 확인
3. 서버 콘솔에서 에러 메시지 확인
4. 포트 충돌 여부 확인 (`:instance validate <id>` 또는 GUI 진단)

### 모듈이 로드되지 않음

**증상**: 모듈 목록에 게임이 표시되지 않음

**해결 방법**:
1. 모듈 디렉토리 경로가 올바른지 확인 (`SABA_MODULES_PATH` 환경 변수)
2. `module.toml`과 `lifecycle.py`가 모듈 디렉토리에 있는지 확인
3. `:module refresh` 실행
4. Python 3.x가 설치되어 있는지 확인

### Discord 봇이 응답하지 않음

**해결 방법**:
1. 봇 토큰이 올바르게 설정되어 있는지 확인
2. 봇이 실행 중인지 확인 (`:bot status`)
3. 봇이 해당 Discord 서버에 초대되어 있는지 확인
4. 봇에 메시지 읽기/쓰기 권한이 있는지 확인
5. prefix가 올바른지 확인

### 업데이트 실패

**해결 방법**:
1. 인터넷 연결 확인
2. GitHub API 접근 가능 여부 확인
3. `:update status`로 현재 상태 확인
4. 수동으로 다시 시도: `:update check` → `:update download` → `:update apply`

---

## 19. FAQ

### Q: 사바쨩이 크래시하면 게임 서버도 꺼지나요?

**A:** 아닙니다. 사바쨩은 게임 서버를 독립 프로세스로 관리하므로, 사바쨩(Core Daemon)이 크래시하더라도 게임 서버는 계속 실행됩니다. 사바쨩을 다시 시작하면 실행 중인 서버를 자동으로 재감지합니다.

### Q: 같은 게임의 서버를 여러 개 운영할 수 있나요?

**A:** 네. 같은 모듈로 여러 인스턴스를 생성할 수 있습니다. 각 인스턴스는 독립적인 설정(포트, 디렉토리 등)을 가집니다.

### Q: 새로운 게임을 추가하려면 어떻게 하나요?

**A:** 모듈을 개발하거나 커뮤니티 모듈을 설치하면 됩니다. 모듈 개발에 대한 자세한 내용은 [모듈 개발 가이드](../saba-chan-modules/docs/module-development-guide.md)를 참조하세요. Core를 재컴파일할 필요는 없습니다.

### Q: GUI와 CLI를 동시에 사용할 수 있나요?

**A:** 네. 둘 다 같은 Core Daemon에 연결되므로 동시 사용이 가능합니다. 각 클라이언트는 독립적으로 데몬에 등록되어 동기화됩니다.

### Q: 원격에서 서버를 관리할 수 있나요?

**A:** Discord 봇을 통해 디스코드 채팅에서 원격으로 서버를 제어할 수 있습니다. NAT 뒤에 있어 포트 포워딩이 불가능한 경우에도 클라우드 모드(릴레이 서버)를 사용하면 원격 제어가 가능합니다.

### Q: Docker로 게임 서버를 격리하여 실행할 수 있나요?

**A:** Docker 익스텐션을 활성화하면 게임 서버를 Docker 컨테이너에서 격리하여 실행할 수 있습니다. 인스턴스 설정에서 `docker_enabled`를 켜고, CPU/메모리 제한을 설정할 수 있습니다.

### Q: 지원하는 언어를 변경하려면?

**A:** GUI 설정 → 일반 → 언어에서 변경하거나, CLI에서 `:config set language <lang_code>` 명령어로 변경할 수 있습니다. 지원 코드: `en`, `ko`, `ja`, `de`, `es`, `fr`, `pt-BR`, `ru`, `zh-CN`, `zh-TW`

---

<p align="center">
  <strong>Saba-chan</strong> — Made with ❤️ and 🐟
</p>

<p align="center">
  <a href="https://github.com/WareAoba/saba-chan">GitHub</a> · 
  <a href="https://github.com/WareAoba/saba-chan/issues">Issue Tracker</a> · 
  MIT License
</p>
