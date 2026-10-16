<div align="center">

<img src="saba-chan-gui/public/icon.png" width="140" alt="사바쨩" />

# 사바쨩 · Saba-chan

**게임 서버를 대신 돌봐 주는 나만의 서버 메이드**

마인크래프트, 팰월드, 프로젝트 좀보이드 서버를 하나의 앱에서 설치하고, 켜고, 고치고, 디스코드에서 친구들과 함께 다루세요.

[![License: MIT](https://img.shields.io/badge/license-MIT-7c3aed?style=flat-square)](LICENSE)
![Platform](https://img.shields.io/badge/platform-Windows-3b82f6?style=flat-square)
![Core](https://img.shields.io/badge/core-Rust-06b6d4?style=flat-square)
![Languages](https://img.shields.io/badge/i18n-10%20languages-22c55e?style=flat-square)

[🌐 웹사이트](https://saba-chan.online) · [📦 모듈](https://github.com/WareAoba/saba-chan-modules) · [🧩 익스텐션](https://github.com/WareAoba/saba-chan-extensions) · [📚 아키텍처 문서](docs/architecture/00-overview.md)

</div>

> 이름은 일본어 "서버(サーバー)"와 "고등어(鯖, さば)"의 발음이 비슷한 데서 왔어요. 🐟

---

## 왜 사바쨩인가요?

게임 서버 하나 여는 데도 할 일이 많아요. 서버 파일 받기, 배치 파일 만들기, 설정 파일 뒤지기, 포트 열기, 밤중에 꺼진 서버 다시 켜기. 게임이 바뀌면 이걸 처음부터 다시 배워야 하죠.

사바쨩은 이 일을 **게임마다 쓰인 "설명서(모듈)"** 로 바꿔 둡니다. 게임을 고르면 설치·설정·실행 방법은 모듈이 알고 있고, 사바쨩이 그대로 해 줍니다.

| 😵 지금까지 | 🐟 사바쨩과 함께 |
|---|---|
| 게임마다 다른 실행법과 배치 파일 | 어떤 게임이든 같은 버튼, 같은 명령 |
| 설정 파일을 직접 열어 한 줄씩 수정 | 앱에서 항목별로 보고 고치기 |
| 원격으로 켜려면 포트 포워딩·원격 데스크톱 | 디스코드 채팅으로 켜고 끄기, 원격 제어용 포트 개방 필요 없음 |
| 서버가 여러 개면 창도 여러 개 | 모든 서버를 한 화면에서 |

## ✨ 기능

- 🖥️ **서버 통합 관리**: 여러 게임 서버를 만들고, 시작·정지하고, 상태와 콘솔 출력을 한곳에서 봅니다.
- ⚙️ **설정을 쉽게**: 게임별 설정을 앱 안에서 항목으로 보여 주고 바로 고칠 수 있어요.
- 💬 **디스코드에서 제어**: 사바쨩 봇을 서버에 초대하고 PC와 연결하면, 채팅으로 서버를 켜고 끄고 상태를 확인할 수 있어요. 봇은 디스코드 릴레이에서 24시간 깨어 있고, 원격 제어를 위해 내 PC의 포트를 열 필요는 없습니다.
- 📦 **모듈로 게임 추가**: 게임 하나 = 모듈 하나. 새 게임 지원은 Python 모듈 하나만 쓰면 됩니다.
- 🧩 **익스텐션으로 기능 추가**: SteamCMD 자동 설치, Docker 격리 실행, 언리얼 엔진 설정 파일 편집 같은 기능을 필요할 때만 켜세요. 익스텐션은 앱 화면에 자기 메뉴도 추가할 수 있어요.
- 🌏 **10개 언어**: 한국어, English, 日本語, Deutsch, Español, Français, Português, Русский, 简体中文, 繁體中文.

### 지원 게임

| 게임 | 지원 내용 |
|---|---|
| **Minecraft** | 공식 서버 자동 다운로드, 콘솔·RCON 명령, 설정 편집 |
| **Palworld** | SteamCMD 자동 설치, REST API 연동 |
| **Project Zomboid** | SteamCMD 자동 설치, 콘솔·RCON 명령 |
| ➕ 더 많은 게임 | [모듈 저장소](https://github.com/WareAoba/saba-chan-modules)에서 직접 만들 수 있어요 |

## 🪶 가볍게 설계했어요

사바쨩의 모든 일은 **데몬** 하나가 합니다. 앱(GUI)과 터미널 도구(CLI)는 데몬에게 부탁만 하는 리모컨이에요.

```
  GUI (선택)     CLI (선택)     디스코드 (릴레이 경유)
        \            |            /
         ▼           ▼           ▼
   ┌──────────────────────────────────┐
   │  데몬 · Rust로 만든 작은 백그라운드 서비스  │
   └──────────────────────────────────┘
         │                    │
      모듈 (게임별 설명서)     익스텐션 (추가 기능)
```

- **GUI 없이도 완전히 동작합니다.** 모니터 없이 늘 켜 두는 서버 PC에서는 데몬만 켜 두고, 디스코드나 CLI로 다루면 됩니다.
- **필요한 것만 설치합니다.** 게임 모듈과 익스텐션은 고른 것만 받아요.
- **Rust 코어**라 오래 켜 둬도 부담이 적고, 서버 PC의 자원은 게임 서버에 돌아갑니다.
- **포터블 설치**: 필요한 런타임을 사바쨩 폴더 안에 담아서, 시스템 PATH나 다른 프로그램을 건드리지 않아요.

## 🛡️ 안심하고 쓰세요

- **내 PC 밖으로 열리지 않아요.** 데몬은 내 컴퓨터 안(`127.0.0.1`)에서만 요청을 받고, 매번 접근 토큰을 확인합니다.
- **원격 제어용 포트는 필요 없어요.** 디스코드 연동은 내 PC가 디스코드 릴레이로 먼저 연결하는 방식이라, 원격 제어를 위해 공유기 설정을 할 필요가 없습니다. 친구가 게임에 접속하려면 게임 포트는 평소처럼 열어야 해요.
- **오가는 요청은 서명돼요.** 디스코드 릴레이와 주고받는 명령은 페어링할 때 받은 키로 서명해서, 남이 끼어들어 명령을 흉내 낼 수 없어요.
- **서버를 격리할 수 있어요.** Docker 익스텐션을 켜면 게임 서버를 컨테이너 안에서 따로 돌립니다.
- **전부 오픈소스입니다.** 무엇을 하는지 코드로 직접 확인할 수 있어요.

## 🚧 준비 중인 기능

| 기능 | 설명 |
|---|---|
| 🔁 **크래시 자동 복구** | 진짜로 죽은 서버만 골라서 다시 켭니다. 내가 일부러 끈 서버는 건드리지 않아요. |
| 💾 **월드 자동 백업** | 정해 둔 주기로 세이브를 스냅샷으로 저장하고, 보관 개수도 고를 수 있어요. 업데이트 전에도 자동으로 백업합니다. |
| ⏰ **예약 작업** | 정기 재시작, 재시작 전 게임 내 공지, 백업을 시간표대로. |
| 🧭 **모든 서버를 끝까지 관리** | 서버마다 작은 감시 프로세스를 붙여, 앱을 껐다 켜도 콘솔과 로그가 끊기지 않아요. 직접 관리하고 싶다면 기존 방식도 그대로 고를 수 있습니다. |
| 🔏 **서명된 자동 업데이트** | 공식 서명이 확인된 업데이트만 설치하고, 결과를 앱에서 바로 보여 줍니다. |
| 🧪 **모듈·익스텐션 안전 검사** | 모듈이 필요한 권한(네트워크, 프로그램 실행 등)을 미리 밝히고, 설치할 때 코드를 검사해 선언과 다르면 알려 줍니다. 공식 저장소 것은 "검증됨" 표시. |
| 🗂️ **설정 파일 하나로** | 흩어진 설정을 파일 하나에 안전하게 저장하고, 망가지면 직전 버전으로 되돌립니다. |
| 🛠️ **모듈 메이커** | 게임 정보만 알려 주면 새 게임 모듈의 뼈대를 만들어 주는 도구. |
| 🤖 **자연어 명령 (구상 중)** | 내 AI API 키로 "서버 켜 줘, 다들 들어오면 공지해 줘" 같은 말을 명령으로 바꿔 줍니다. |

## 🧱 구성요소

| 구성요소 | 위치 | 역할 |
|---|---|---|
| **데몬** (`saba-core`) | `src/` (Rust) | 서비스 본체. 모든 기능은 데몬을 통해 동작하고, 디스코드 릴레이 연결도 데몬이 맡습니다 |
| **GUI** | `saba-chan-gui/` (Electron + React) | 데몬을 조작하는 데스크톱 앱 |
| **CLI** | `saba-chan-cli/` (Rust TUI) | 데몬을 조작하는 터미널 앱 |
| **업데이터 / 인스톨러** | `updater/`, `installer/` (Rust, Tauri) | 설치와 업데이트 |
| **모듈** | [saba-chan-modules](https://github.com/WareAoba/saba-chan-modules) | 게임별 서버 구동 설명서 (`module.toml` + `lifecycle.py`) |
| **익스텐션** | [saba-chan-extensions](https://github.com/WareAoba/saba-chan-extensions) | 데몬 기능 추가, GUI 화면 주입 (Python) |

## 📚 문서

**[docs/architecture/](docs/architecture/00-overview.md)** 가 유일한 기준 문서입니다. 코드를 직접 읽어 작성했고 모든 주장에 `file:line` 근거가 달려 있습니다. 코드의 계약(API 라우트, `module.toml` 키, hook, manifest 필드)이 바뀌면 같은 커밋에서 해당 문서를 고칩니다.

| 문서 | 내용 |
|------|------|
| [00-overview](docs/architecture/00-overview.md) | 전체 구조, 핵심 계약 요약, 알려진 문제 |
| [01-daemon-core](docs/architecture/01-daemon-core.md) | 데몬 기동, IPC HTTP API 전체 라우트, 인증, 인스턴스 모델 |
| [02-modules](docs/architecture/02-modules.md) | `module.toml` 스키마, `lifecycle.py` 호출 프로토콜 |
| [03-extensions](docs/architecture/03-extensions.md) | 익스텐션 `manifest.json`, hook, GUI 주입 계약 |
| [04-clients](docs/architecture/04-clients.md) | GUI, CLI, 로케일 |
| [05-updater-servers](docs/architecture/05-updater-servers.md) | 업데이터, 인스톨러, CI, 디스코드 릴레이 |

## 🔨 소스에서 빌드 (Windows)

```powershell
# 전체 릴리스 빌드
.\scripts\build-windows.ps1

# 데몬
cargo build --release

# CLI
cargo build --release -p saba-chan-cli

# GUI 개발 모드
cd saba-chan-gui; npm install; npm start
```

## 라이선스

[MIT License](LICENSE)
