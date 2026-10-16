# 🐟 Saba-chan (サバちゃん / 사바쨩)

> **모듈형 게임 서버 관리 플랫폼** — 여러 게임 서버를 하나의 앱에서 통합 관리

> 이름의 유래: 일본어로 "서버(サーバー)"와 "고등어(鯖, さば)"의 발음이 비슷한 것에서 착안한 말장난입니다.

## 구성요소

| 구성요소 | 위치 | 역할 |
|----------|------|------|
| **데몬** (`saba-core`) | `src/` (Rust, axum) | 서비스 주체. 모든 기능은 데몬을 통해 동작. `127.0.0.1:57474`, `X-Saba-Token` 인증 |
| **GUI** | `saba-chan-gui/` (Electron + React) | 데몬 HTTP API 클라이언트 |
| **CLI** | `saba-chan-cli/` (Rust TUI) | 데몬 HTTP API 클라이언트 |
| **Discord 봇** | `discord_bot/` (Node) | 로컬 모드 또는 relay(server-chan)를 통한 클라우드 모드 |
| **업데이터 / 인스톨러** | `updater/`, `installer/` (Rust, Tauri) | GitHub Releases 기반 업데이트·설치 |
| **모듈** | [saba-chan-modules](https://github.com/WareAoba/saba-chan-modules) | 게임별 서버 구동 "설명서" (`module.toml` + `lifecycle.py`) |
| **익스텐션(플러그인)** | [saba-chan-extensions](https://github.com/WareAoba/saba-chan-extensions) | 데몬 기능 추가, GUI 컴포넌트 주입 (Python + UMD 번들) |

## 문서

**[docs/architecture/](docs/architecture/00-overview.md)** 가 유일한 기준 문서입니다. 코드를 직접 읽어 작성했고 모든 주장에 `file:line` 근거가 달려 있습니다. 코드의 계약(API 라우트, `module.toml` 키, hook, manifest 필드)이 바뀌면 같은 커밋에서 해당 문서를 고칩니다.

| 문서 | 내용 |
|------|------|
| [00-overview](docs/architecture/00-overview.md) | 전체 구조, 핵심 계약 요약, 알려진 문제 |
| [01-daemon-core](docs/architecture/01-daemon-core.md) | 데몬 기동, IPC HTTP API 전체 라우트, 인증, 인스턴스 모델 |
| [02-modules](docs/architecture/02-modules.md) | `module.toml` 스키마, `lifecycle.py` 호출 프로토콜 |
| [03-extensions](docs/architecture/03-extensions.md) | 익스텐션 `manifest.json`, hook, GUI 주입 계약 |
| [04-clients](docs/architecture/04-clients.md) | GUI, CLI, Discord 봇, 로케일 |
| [05-updater-servers](docs/architecture/05-updater-servers.md) | 업데이터, 인스톨러, CI, relay |

## 소스에서 빌드 (Windows)

```powershell
# 전체 릴리스 빌드
.\scripts\build-windows.ps1

# 데몬
cargo build --release

# CLI
cargo build --release -p saba-chan-cli

# GUI 개발 모드
cd saba-chan-gui; npm install; npm start

# Discord 봇
cd discord_bot; npm install; npm start
```

## 라이선스

[MIT License](LICENSE)
