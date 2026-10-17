# 사바쨩 아키텍처 기준 문서 (코드 실측판)

> 이 폴더(`docs/architecture/`)는 **기존 README/docs를 버리고 코드를 직접 읽어서** 다시 만든 기준 문서다. 앞으로의 작업은 이 문서를 출발점으로 삼고, 코드가 바뀌면 이 문서도 같이 고친다(맨 아래 "문서 유지 규칙").

## 기준 스냅샷

| 저장소 | 커밋 | 커밋 날짜 |
|---|---|---|
| WareAoba/saba-chan | `77acaee` 정리14 | 2026-04-14 |
| WareAoba/saba-chan-modules | `0ab6035` 정리6 | 2026-03-07 |
| WareAoba/saba-chan-extensions | `e5df73f` | 2026-04-14 |
| WareAoba/server-chan | `4c423e8` 정리6 | 2026-04-14 |

- 모든 근거는 위 커밋의 GitHub `main`이다. 이후 커밋이 쌓이면 해당 절의 file:line을 다시 확인해야 한다.
- 읽기만 했고 실행·테스트는 하지 않았다. "버그"로 적힌 항목은 코드 판독에 근거한 판단이며, 고치기 전에 재현부터 하는 게 맞다.

## 문서 구성

| 파일 | 다루는 것 |
|---|---|
| [01-daemon-core.md](01-daemon-core.md) | 데몬 기동, **IPC HTTP API 전체 라우트 표(약 95개)**, 인증, 인스턴스 모델·영속화, 전역 설정, RCON/REST, 내장 Python/Node, plugin 실행기 |
| [02-modules.md](02-modules.md) | `module.toml` 전체 스키마(Rust 역직렬화 기준), `lifecycle.py` 호출 프로토콜과 함수 매트릭스, managed process, 게임별 기능, 모듈 배포 |
| [03-extensions.md](03-extensions.md) | 플러그인(코드상 extension) `manifest.json` 스키마, hook 전수 목록과 hook×익스텐션 매트릭스, 설치·활성화 수명주기, GUI 컴포넌트 주입(UMD/slot) 계약 |
| [04-clients.md](04-clients.md) | GUI(Electron) preload 채널 전수표, CLI(TUI), Discord 봇, 공유 상수·로케일, **인터페이스가 데몬을 우회하는 지점 전수 목록(§7)** |
| [05-updater-servers.md](05-updater-servers.md) | 업데이터·인스톨러·CI, server-chan relay API·DB·프로토콜 (운영 결함·비밀 파일은 server-chan 비공개 문서) |

---

## 1. 다섯 구성요소: 오너 정의와 코드상 실체

오너가 정한 역할(설계 원칙)과, 코드에서 그 역할을 맡은 실체를 나란히 둔다. 이후 모든 "위반" 판정은 왼쪽 열이 기준이다.

| 구성요소 | 오너 정의 | 코드상 실체 |
|---|---|---|
| **데몬** | 서비스 주체. 모든 기능은 여기로 작동 | `saba-chan/src` → 바이너리 `saba-core`. axum HTTP 서버, `127.0.0.1:57474`(`SABA_IPC_PORT`), 헤더 `X-Saba-Token` 인증 |
| **인터페이스** | 데몬 조작 도구. 혼자서는 아무것도 못함. 파일 수정도 데몬 API로 | `saba-chan-gui`(Electron+React), `saba-chan-cli`(Rust TUI). 둘 다 데몬 HTTP API 클라이언트 |
| **모듈** | 게임 서버 구동 "설명서", Python | `saba-chan-modules/<game>/{module.toml, lifecycle.py, locales/}`. 데몬이 `python lifecycle.py <함수명>`을 단발 실행(stdin JSON → stdout JSON) |
| **플러그인** | 데몬 기능 추가·인터페이스 컴포넌트 주입, Python | 코드에서는 **extension**이라 부름. `saba-chan-extensions/<name>/{manifest.json, *.py, gui/dist/*.umd.js}`. 데몬이 hook 시점에 같은 Python 실행기로 호출, GUI가 UMD 번들을 slot에 주입 |
| **서버쨩** | 데몬과 연계해 디스코드 봇 구동 + 봇 추가 기능 | 두 갈래로 나뉘어 있다(아래 §2). ① `saba-chan/discord_bot`(Node, 로컬 봇·relay agent) ② `server-chan/relay-server`(Fastify+PostgreSQL, 중앙 클라우드 봇 + 중계) |

### 용어 충돌 (문서·코드 읽을 때 주의)

- `src/plugin/`은 오너가 말하는 "플러그인"이 **아니다.** 모듈과 익스텐션이 함께 쓰는 **Python 실행기**다. 오너의 "플러그인" = 코드의 `src/extension/` + `saba-chan-extensions`.
- `ipc::ExtensionInfo`, GUI의 `selectedExtension`은 이름과 달리 **모듈**을 가리킨다.
- `discord_bot/extensions/*.js`(music 등)는 Python 익스텐션과 별개인 **세 번째 "extension"** 개념(봇 내부 JS 기능)이다.
- 이 문서 세트에서는 오너 용어를 따르되 처음 나올 때 코드 이름을 병기한다: 플러그인(extension), 실행기(plugin).

## 2. 구성요소 간 연결 (코드 기준)

```
                ┌──────────────── 인터페이스 ────────────────┐
                │  GUI (Electron)        CLI (TUI)           │
                └────────┬───────────────────┬───────────────┘
                         │ HTTP + X-Saba-Token (127.0.0.1:57474)
                         ▼
   ┌──────────────────── 데몬 saba-core ───────────────────────┐
   │ IPC 라우터 ─ Supervisor ─ InstanceStore ─ ConfigStore      │
   │     │            │                                        │
   │     │            ├─ plugin 실행기: python <file>.py <fn>  ─┼─► 모듈 lifecycle.py
   │     │            │                                        ├─► 플러그인 hook (*.py)
   │     │            └─ managed process(stdin/stdout), RCON, REST ─► 게임 서버
   │     ├─ ext-process 매니저 ─► discord_bot(Node) 실행        │
   │     ├─ /api/relay/* 프록시 ─────────────────────────────────┼─► server-chan relay
   │     └─ /api/updates/* ─ updater lib ─► GitHub Releases     │
   └───────────────────────────────────────────────────────────┘
                         ▲ HTTP + X-Saba-Token
   discord_bot (Node) ───┘
     ├ local 모드: 사용자 봇 토큰으로 Discord 직접 로그인
     └ cloud 모드: Discord 로그인 없이 relay를 long-poll(/poll 25s, HMAC 서명)해서
                   받은 명령을 데몬에 실행시키고 /result, /heartbeat로 보고
   server-chan relay ── 중앙 Discord 봇(discord.js) ── PostgreSQL command_queue
```

- server-chan 운영 부가 구성요소(chatbot-server, music-server)와 운영 배포 구성은 server-chan 비공개 문서에서 다룬다.

## 3. 핵심 계약 요약 (상세는 각 문서)

| 계약 | 형식 | 상세 |
|---|---|---|
| 인터페이스 → 데몬 | HTTP JSON, `X-Saba-Token`(데몬이 `.ipc_token` 파일에 기록, 클라이언트가 읽음). 토큰 파일이 없으면 인증 생략(fail-open) | 01 §2, §3 |
| 데몬 → 모듈 | `python lifecycle.py <fn>`, stdin JSON(config) → stdout 마지막 JSON, stderr `PROGRESS:` 진행 보고, 기본 120초 타임아웃. 데몬이 부르는 함수 16종 | 02 §2 |
| 모듈 정의 | `module.toml`: `[module]`(필수) `[update]` `[protocols]` `[config]` `[detection]` `[[settings.fields]]` `[[commands.fields]]` `[install]` `[docker]` 등. 모르는 키는 조용히 버려짐 | 02 §1.2 |
| 데몬 → 플러그인 | 같은 실행기로 hook 함수 호출. 실제 발화 hook 10종(`daemon.startup/shutdown`, `server.pre_start/post_stop/status/list_enrich/post_create/pre_delete/settings_changed/check_update`) | 03 §2 |
| 플러그인 → GUI | GUI가 `/api/extensions/:id/gui` 번들을 로드, 전역 `window.SabaExt<Pascal>`의 `registerSlots()`로 slot에 컴포넌트 주입 | 03 §4 |
| 데몬 ↔ relay | 페어링·호스트/노드 조회는 데몬 `/api/relay/*` 프록시. 봇 relay agent는 relay에 직접 HMAC 서명 요청 | 05 §4 |
| 업데이트 | 데몬이 updater 라이브러리를 내장, GitHub Releases `manifest.json` 기준. GUI/코어/업데이터 자신은 `pending.json` 기록 후 인터페이스가 `saba-chan-updater --apply`를 실행 | 05 §1 |

## 4. 원칙 위반 종합 (오너 정의 기준)

### 4.1 인터페이스가 데몬을 우회 ("인터페이스는 혼자 아무것도 못한다" 위반)

1. GUI가 `settings.json`을 직접 읽고 쓴다. 분기 기준이 "데몬이 살아 있나"가 아니라 "이 GUI가 데몬을 spawn했나"라서 데몬이 떠 있어도 직접 쓰는 경우가 생긴다(04 §7 G2–G4).
2. GUI가 relay에 bot-config를 직접 PATCH한다. 데몬 `relay.rs` 주석의 "GUI/CLI는 relay에 직접 접근 금지"와 정면 충돌하고, 실제로 항상 실패한다(04 G17).
3. 게임 명령 해석(rcon/rest 분기, 템플릿 치환, 기본 endpoint)을 GUI와 봇이 **각각** 구현한다. 데몬은 raw `/rcon`, `/rest`만 제공한다(02 §6.5 #1).
4. managed/native 시작 경로 선택, 기존 폴더→모듈 판정을 인터페이스가 한다(02 §6.5 #2, #3).
5. 디스코드 봇 실행 명령을 GUI와 데몬(`auto_start_discord_bot`)이 따로 조립하고, GUI가 Discord 토큰을 데몬으로 다시 넘긴다(01 §9.A-3).
6. CLI: 데몬 미응답 시 로컬 fs 폴백, `module.toml` 직접 파싱, 자체 설정 파일 `cli-settings.json`(04 C3–C7, 02 §6.5 #4).
7. 업데이트 3시간 타이머를 GUI가 돌리고, 업데이터 exe를 GUI/CLI가 직접 실행한다(05 §6.1).
8. 봇이 `bot-config.json`을 파일로 읽고, music 기능이 Python(`music_deps.py`)·yt-dlp·ffmpeg를 데몬 없이 직접 실행한다(04 B5, B6).

부트스트랩상 불가피한 것(토큰 파일 읽기, 최초 데몬 spawn)은 예외다. 데몬 **종료**는 `/api/daemon/shutdown`이 있으므로 OS kill 대신 API로 바꿀 수 있다.

### 4.2 데몬이 게임·플러그인 지식을 하드코딩 ("모듈=설명서, 플러그인=자유 교체" 위반)

- 플러그인 이름 하드코딩: `docker`, `docker_enabled`, `steamcmd_*`, `music`가 supervisor·instance·module_loader(`[docker]` 섹션)에 박혀 있다. docker UI 문자열도 호스트 `locales/*/gui.json`에 있다(03 §7.3, 01 §9.A-8).
- 게임 지식 하드코딩: `java_path`, `ram`, `server.jar`, `server.properties`, `accept_eula`(Minecraft) 등이 데몬 공용 API에 있다. GUI도 `module === 'palworld'`로 분기한다(02 §6.5 #6–#8).
- 모듈이 플러그인을 `from extensions.x import`로 직접 import하고, 모듈이 데몬 HTTP를 역호출한다(인증 없이, 포트 하드코딩). 플러그인 비활성화로 막을 수 없다(02 §6.5 #9, #10).

## 5. 우선순위 높은 문제 (판독 기준, 재현 전)

### 보안
- 데몬 `/api/updates/*`가 인증 미들웨어 바깥에 붙어 있다(`src/ipc/mod.rs:667-669`). 세부는 01 §9.B.
- 운영 서버(server-chan)와 비밀 파일 관련 항목은 공개 저장소에 두지 않고 server-chan 비공개 문서에서 관리한다.

### 기능이 깨져 있을 가능성이 큰 것
- 업데이터 무결성 검사: CI는 zip 해시를 넣는데 검사기는 압축 해제된 파일을 해시한다. 해시가 있는 모든 컴포넌트가 "Tampered"로 나온다(05 §1.5). 모듈 업데이트는 릴리즈 태그를 버전으로 비교해 감지되지 않을 가능성이 크다(02 §5).
- GUI `updater:setConfig`와 CLI가 `/api/updates/config`에 POST를 보내는데 데몬은 GET/PUT만 받는다 → 405. CLI의 `/api/install/*`는 데몬에 라우트가 없다(04 §6.1).
- 모듈 → 데몬 역호출(`daemon_rcon` 등)이 토큰을 안 보내서 인증이 켜진 환경에서 401. Zomboid stdin 호출은 페이로드 키도 틀림(`input` vs `command`)(02 §5 요약).
- minecraft `import_settings`/`reset_properties`, palworld `import_settings`가 정의만 되고 `FUNCTIONS` 디스패치 표에 없어 항상 "Unknown function"(02 §2.4).
- Linux/macOS에서 `SABA_ENABLED_EXTENSIONS` 경로가 틀려 플러그인 활성 여부를 모듈이 못 읽는다(`src/plugin/mod.rs:140`).
- 크래시 감지·자동 재시작 없음(상태 머신이 사실상 dead code). managed 모드에서 Minecraft·Zomboid는 RCON을 끄는데 명령은 RCON으로 보낸다(02 §3).
- 업데이트 실패 직후 5초 안에 진행 표시를 닫으면 기존 인스턴스가 삭제될 수 있다(01 §9.B-10).
- GUI 종료 시 봇 정지 요청이 스코프 오류로 전송되지 않는다(`main.js:1536/1559`).

### 구조 정리 대상
- 단일 인스턴스 보장 없음 → 두 번째 데몬이 `.ipc_token`을 덮어써 기존 데몬이 모든 요청에 401(01 §1.6).
- 긴 Python 호출 동안 supervisor/익스텐션 락을 잡아 다른 요청이 막힌다(01 §9.B-13, 03 §7.2).
- hook 실행 순서가 HashMap 순서라 비결정적이고, `handled`가 체인을 끊는다(03 §7.2).
- manifest에 선언됐지만 발화되지 않는 hook 7종, 읽히지 않는 manifest 필드(`async`, `min_app_version`, `gui.slots` 등), 무시되는 `module.toml` 키(`entry`, `[detection.linux]`, 명령 입력 `options/min/max`)(02 §1.2, 03 §1.3).
- 릴리즈 CI가 매번 이전 릴리즈를 전부 삭제하고, 런타임 파일(`dockerd.pid`, `.deps-resolved.json`)이 배포 zip에 섞인다(03 §7.2).

## 6. 문서 유지 규칙

이 문서가 계속 "믿을 수 있는" 상태로 남으려면:

1. **근거는 코드뿐**이다. 모든 계약 주장에는 `file:line`을 단다. 문서끼리 서로를 근거로 삼지 않는다.
2. 계약(라우트, `module.toml` 키, hook 이름, manifest 필드, relay API)을 바꾸는 커밋은 같은 커밋에서 해당 절을 고친다.
3. 고친 문제는 §5와 각 문서의 "불일치" 절에서 지우고 커밋 해시를 남긴다.
4. 기준 스냅샷 표의 커밋을 갱신한다. 표보다 오래된 줄 번호는 다시 확인 전까지 "대략"으로 취급한다.
5. 이 세트가 기존 README/docs를 대체한다. 여기 없는 내용을 다른 문서에서 가져와 사실로 쓰지 않는다.
