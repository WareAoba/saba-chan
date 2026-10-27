<div align="center">

<img src="saba-chan-gui/public/icon.png" width="140" alt="사바쨩" />

# 사바쨩 · Saba-chan

**게임 서버를 대신 돌봐 주는 나만의 서버 메이드**

마인크래프트, 팰월드, 프로젝트 좀보이드, 테라리아 서버를 하나의 앱에서 설치하고, 켜고, 고치고, 디스코드에서 친구들과 함께 다루세요.

[![License: MIT](https://img.shields.io/badge/license-MIT-7c3aed?style=flat-square)](LICENSE)
![Platform](https://img.shields.io/badge/platform-Windows%20%7C%20Linux%20%28daemon%29-3b82f6?style=flat-square)
![Core](https://img.shields.io/badge/core-Rust-06b6d4?style=flat-square)
![Languages](https://img.shields.io/badge/i18n-10%20languages-22c55e?style=flat-square)

[🌐 웹사이트](https://saba-chan.online) · [📦 모듈](https://github.com/WareAoba/saba-chan-modules) · [🧩 익스텐션](https://github.com/WareAoba/saba-chan-extensions) · [📚 아키텍처 문서](docs/architecture/00-overview.md)

</div>

> 이름은 일본어 "서버(サーバー)"와 "고등어(鯖, さば)"의 발음이 비슷한 데서 왔어요. 🐟

---

## 왜 사바쨩인가요?

게임 서버 하나 여는 데도 할 일이 많아요. 서버 파일 받기, 배치 파일 만들기, 설정 파일 뒤지기, 원격으로 켜려고 원격 데스크톱 붙이기, 밤중에 꺼진 서버 다시 켜기. 게임이 바뀌면 이걸 처음부터 다시 배워야 하죠.

사바쨩은 이 일을 **게임마다 쓰인 "설명서(모듈)"** 로 바꿔 둡니다. 게임을 고르면 설치·설정·실행 방법은 모듈이 알고 있고, 사바쨩이 그대로 해 줍니다.

| 😵 지금까지 | 🐟 사바쨩과 함께 |
|---|---|
| 게임마다 다른 실행법과 배치 파일 | 어떤 게임이든 같은 버튼, 같은 명령 |
| 설정 파일을 직접 열어 한 줄씩 수정 | 앱에서 항목별로 보고 고치기 |
| 원격으로 켜려면 포트 포워딩·원격 데스크톱 | 디스코드 채팅으로 켜고 끄기, 원격 제어용 포트 개방 필요 없음 |
| 서버가 여러 개면 창도 여러 개 | 모든 서버를 한 화면에서 |

## ✨ 기능

- 🖥️ **서버 통합 관리**: 여러 게임 서버를 만들고, 시작·정지하고, 상태와 콘솔 출력을 한곳에서 봅니다. 게임별 설정도 앱 안에서 항목으로 보고 바로 고칠 수 있어요.
- 👯 **같은 게임 서버를 여러 개**: 팰월드 서버 두 개, 좀보이드 서버 두 개도 나란히. 새 서버는 겹치지 않는 포트와 자기 폴더를 알아서 받아요.
- 🔁 **크래시 자동 복구**: 오류로 꺼진 서버만 골라서 다시 켜요. 내가 일부러 끈 서버는 건드리지 않고, 왜 꺼졌는지 종료 기록으로 남겨요.
- 🧭 **끊기지 않는 관리**: 서버마다 작은 감시 프로세스(`saba-runner`)가 붙어서, 사바쨩을 껐다 켜도 서버는 계속 돌고 콘솔과 로그도 이어집니다.
- 💬 **디스코드에서 제어**: 사바쨩 봇을 서버에 초대하고 PC와 연결하면, 채팅으로 서버를 켜고 끄고 상태를 확인할 수 있어요. 디스코드 서버마다 보여 줄 게임 서버를 고르고, 멤버마다 쓸 수 있는 명령을 정할 수 있어요. 명령어 별명도 직접 지을 수 있어요. 봇은 디스코드 릴레이에서 24시간 깨어 있고, 원격 제어를 위해 내 PC의 포트를 열 필요는 없습니다.
- 🛰️ **원격 데몬** (0.1.11부터): 다른 PC나 리눅스 서버에서 도는 사바쨩을 내 PC의 앱과 CLI로 그대로 다뤄요. 같은 디스코드 계정의 PC끼리만 짝지어지고, 연결은 TLS로 암호화돼요. 직접 닿지 않는 PC는 사바쨩 클라우드를 거쳐 이어져서 포트 포워딩이 꼭 필요하지 않아요.
- 📱 **사바쨩 웹** (0.1.11부터): 디스코드에서 `/사바쨩 웹`으로 받은 일회용 링크로, 휴대폰 브라우저에서 내 PC의 서버를 켜고 끄고 콘솔·설정·백업을 다뤄요. 설치도 로그인도 필요 없고, 세션은 15분이고 연장할 수 있어요.
- 🐧 **리눅스 서버** (0.1.11부터): 화면 없는 리눅스(x86_64)에 데몬과 CLI를 한 줄로 설치하고, 원격 데몬으로 다뤄요.
- 🔔 **알림** (0.1.11부터): 누가 어디서 서버를 켜고 껐는지, 예상하지 못한 시작·정지까지 앱·OS 알림·디스코드 알림 채널로 알려 줘요.
- ⌨️ **GUI도 CLI도**: 데스크톱 앱으로 하는 일은 모두 명령줄로도 됩니다(`saba-chan-cli start pal`). 인자 없이 실행하면 Tab 자동 완성이 되는 대화형 셸이 열리고, 스크립트용 `--json` 출력도 있어요. 자세히: [saba-chan-cli/README.md](saba-chan-cli/README.md)
- 📦 **모듈로 게임 추가**: 게임 하나 = 모듈 하나. 새 게임 지원은 Python 모듈 하나만 쓰면 되고, 적합성 검사 키트로 바로 확인할 수 있어요. Claude Code·Codex 같은 AI 에이전트에게 [모듈 메이커](https://saba-chan.online/ko/guide/module-maker)로 만들게 할 수도 있어요.
- 🧩 **익스텐션으로 기능 추가**: SteamCMD 자동 설치, 언리얼 엔진 설정 파일 편집 같은 기능을 필요할 때만 켜세요. 익스텐션은 앱 화면에 메뉴를, 게임 서버에 명령을 더할 수 있어요.
- 💾 **월드 백업**: 세이브를 스냅샷으로 저장하고 되돌려요. 주기와 보관 개수를 정해 두면 알아서 백업합니다.
- ⏰ **예약 작업**: 정기 재시작, 재시작 전 게임 안 공지, 백업을 시간표대로.
- 🧪 **모듈·익스텐션 안전 검사**: 모듈이 필요한 권한을 미리 밝히고, 공식 서명이 없는 코드는 설치할 때 검사한 뒤 허용된 폴더 안에서만 돌려요.
- 🌏 **10개 언어**: 한국어, English, 日本語, Deutsch, Español, Français, Português, Русский, 简体中文, 繁體中文.

### 지원 게임

| 게임 | 지원 내용 |
|---|---|
| **Minecraft** | 공식 서버 자동 다운로드, 콘솔 명령, 설정 편집 |
| **Palworld** | SteamCMD 자동 설치, REST API 연동 |
| **Project Zomboid** | SteamCMD 자동 설치, 콘솔 명령 |
| **Terraria** | 공식 서버 자동 다운로드, 콘솔 명령, Windows·Linux (0.1.11부터) |
| ➕ 더 많은 게임 | [모듈 저장소](https://github.com/WareAoba/saba-chan-modules)에서 직접 만들 수 있어요 |

## 🪶 가볍게 설계했어요

사바쨩의 모든 일은 **데몬** 하나가 합니다. 앱(GUI)과 터미널 도구(CLI)는 데몬에게 부탁만 하는 리모컨이에요.

<p align="center">
  <picture>
    <source media="(prefers-color-scheme: dark)" srcset="docs/images/architecture-dark.svg">
    <img src="docs/images/architecture-light.svg" alt="사바쨩 구조: GUI와 CLI는 이 PC 안에서 데몬을 조작하고, 데몬이 게임 서버·모듈·익스텐션을 다뤄요. 디스코드 명령은 클라우드 릴레이를 거치고, 연결은 내 PC가 먼저 바깥으로 열어요.">
  </picture>
</p>

- **GUI 없이도 동작합니다.** 모니터 없이 늘 켜 두는 서버 PC에서는 데몬만 켜 두고, 디스코드나 CLI, 다른 PC의 앱(원격 데몬)으로 다루면 됩니다.
- **필요한 것만 설치합니다.** 게임 모듈과 익스텐션은 고른 것만 받아요.
- **Rust 코어**라 오래 켜 둬도 부담이 적고, 서버 PC의 자원은 게임 서버에 돌아갑니다.
- **시스템을 건드리지 않아요**: 필요한 런타임(Python 등)은 사바쨩 데이터 폴더에 따로 받아서, 시스템 PATH나 다른 프로그램을 바꾸지 않아요.

## 📥 설치

1. [웹사이트](https://saba-chan.online)에서 설치기(`saba-chan-installer.exe`)를 받아 실행합니다. GitHub [Releases](https://github.com/WareAoba/saba-chan/releases)에도 같은 파일이 있어요.
2. 설치 위치와 처음 받을 게임 모듈을 고르고 설치를 누릅니다. 기본 위치는 `C:\Program Files\Saba-chan`이고, 다른 파일이 든 폴더를 고르면 그 안에 `Saba-chan` 폴더를 만들어 설치해요.

- **요구사항**: Windows 10 또는 11 (64비트), Microsoft Edge WebView2 런타임(Windows 11과 최신 Windows 10에는 들어 있어요), 인터넷 연결.
- **리눅스 서버** (0.1.11부터): 화면 없는 리눅스(x86_64)에는 데몬과 CLI만 설치해요. `curl`, `unzip`, `sha256sum`, `python3`가 필요해요.

  ```sh
  curl -fsSL https://raw.githubusercontent.com/WareAoba/saba-chan/main/install.sh | sh
  ```

  설치기가 최신 정식 릴리즈를 받아 릴리즈 해시와 서명된 매니페스트로 파일을 확인하고, 데몬을 systemd 사용자 서비스로 켭니다. 앱 화면은 Windows PC의 사바쨩에서 원격 데몬으로 써요. 자세히: [원격 데몬 사용법](https://saba-chan.online/ko/guide/remote-daemon)
- **데이터 위치**: 설정, 게임 서버 파일, 월드, 백업은 `%APPDATA%\saba-chan`(리눅스는 `~/.config/saba-chan`)에 저장돼요. 프로그램 폴더를 지워도 서버는 남아요.
- **업데이트**: 앱이 새 버전을 알려 주면 설정 화면에서 바로 적용합니다. 서명이 확인된 릴리즈만 받아요.
- **제거**: Windows 설정의 "설치된 앱"이나 앱의 설정 화면에서 제거합니다. "게임 서버와 설정 남기기"를 고르면 서버·월드·백업·설정이 남아서 다시 설치하면 그대로 이어져요. 고르지 않으면 모두 지워집니다.

## 🛡️ 보안

- **내 PC 밖으로 열리지 않아요.** 데몬은 내 컴퓨터 안(`127.0.0.1`)에서만 요청을 받고, 매번 접근 토큰을 확인합니다. 원격 데몬을 직접 켰을 때만 원격 포트(TLS)가 열리고, 같은 디스코드 계정으로 짝지은 PC만 들어올 수 있어요.
- **원격 제어용 포트는 필요 없어요.** 디스코드 연동은 내 PC가 디스코드 릴레이로 먼저 연결하는 방식이라, 원격 제어를 위해 공유기 설정을 할 필요가 없습니다. 친구가 게임에 접속하려면 게임 포트는 평소처럼 열어야 해요.
- **오가는 요청은 서명돼요.** 디스코드 릴레이와 주고받는 명령은 페어링할 때 받은 키로 서명해서, 남이 끼어들어 명령을 흉내 낼 수 없어요.
- **멤버마다 권한을 따로.** 디스코드에서 누가 어떤 명령을 쓸 수 있는지 정할 수 있고, 위험한 명령은 따로 표시돼요.
- **서명된 업데이트만 설치해요.** 앱·모듈·익스텐션 모두 공식 서명이 확인된 릴리즈에서만 받고, 내려받은 파일은 하나하나 해시로 검사합니다.
- **켜 둔 익스텐션만 불러와요.** 꺼 둔 익스텐션은 모듈 코드가 몰래 불러올 수도 없어요.
- **설정 파일이 깨져도 복구해요.** 설정은 파일 하나(`config.json`)에 저장되고, 파일이 깨지면 직전 버전으로 자동 복구합니다.
- **전부 오픈소스입니다.** 무엇을 하는지 코드로 직접 확인할 수 있어요.

## 🚧 준비 중인 기능

| 기능 | 설명 |
|---|---|
| 🍎 **macOS** | 지금은 Windows 앱과 리눅스 데몬(0.1.11부터)만 지원해요. |

## 🧱 구성요소

| 구성요소 | 위치 | 역할 |
|---|---|---|
| **데몬** (`saba-core`) | `src/` (Rust) | 서비스 본체. 모든 기능은 데몬을 통해 동작하고, 디스코드 릴레이 연결도 데몬이 맡습니다 |
| **GUI** | `saba-chan-gui/` (Tauri + React) | 데몬을 조작하는 데스크톱 앱 |
| **CLI** | `saba-chan-cli/` (Rust, clap) | 명령줄 도구 + 대화형 셸 |
| **러너** (`saba-runner`) | `runner/` (Rust) | 서버마다 붙는 감시 프로세스. 데몬이 다시 켜져도 콘솔·로그·종료 코드를 이어 줍니다 |
| **업데이터 / 인스톨러** | `updater/`, `installer/` (Rust, Tauri) | 서명된 릴리즈로 설치·업데이트 |
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

배포 파일은 GitHub Actions(`.github/workflows/build-windows.yml`)가 이 저장소의 소스로 빌드하고 서명합니다. 직접 빌드할 때는:

```powershell
# 전체 빌드 (서명하지 않음)
.\scripts\build-windows.ps1

# 데몬
cargo build --release

# CLI
cargo build --release -p saba-chan-cli

# GUI 개발 모드
cd saba-chan-gui; npm install; npm start
```

## 코드 서명 정책 · Code signing policy

Free code signing provided by [SignPath.io](https://signpath.io), certificate by [SignPath Foundation](https://signpath.org).

- **서명하는 파일**: 이 저장소의 소스로 GitHub Actions가 빌드한 Windows 실행 파일(`saba-core.exe`, `saba-runner.exe`, `saba-chan-cli.exe`, `saba-chan-gui.exe`, `saba-chan-updater.exe`, `saba-chan-installer.exe`)만 서명합니다. 빌드 과정은 [build-windows.yml](.github/workflows/build-windows.yml)에 공개돼 있어요.
- **Committers and reviewers**: [WareAoba](https://github.com/WareAoba)
- **Approvers**: [WareAoba](https://github.com/WareAoba) — 릴리스마다 서명 요청을 직접 승인합니다.
- **개인정보 · Privacy policy**: 사바쨩은 업데이트 확인(기본으로 켜져 있고 설정에서 끌 수 있어요. 익명 ID와 버전만 보냅니다)과, 사용자가 직접 연결한 디스코드 연동 말고는 다른 시스템으로 정보를 보내지 않습니다. 무엇을 보내고 얼마나 보관하는지는 [개인정보처리방침](https://saba-chan.online/legal/privacy)에 있어요. Saba-chan sends data only for update checks (on by default, can be turned off; an anonymous ID and version numbers) and for the Discord integration the user connects; see the [privacy policy](https://saba-chan.online/legal/privacy).

## 라이선스

[MIT License](LICENSE)
