# 사바쨩 아키텍처 기준 문서 (코드 실측판)

> 이 폴더(`docs/architecture/`)는 **기존 README/docs를 버리고 코드를 직접 읽어서** 다시 만든 기준 문서다. 앞으로의 작업은 이 문서를 출발점으로 삼고, 코드가 바뀌면 이 문서도 같이 고친다(맨 아래 "문서 유지 규칙").


> **2026-10-17 L3 이후**: 업데이트·모듈 목록·익스텐션 목록은 모두 릴레이의 서명된 릴리즈 manifest 하나에서 나온다. GitHub 업데이트 경로(`github.rs`, `api_base_url`, `github_owner`/`github_repo`, `include_prerelease`, 익스텐션 `manifest_url`, module.toml `[update]`, modules·extensions 저장소의 릴리즈 워크플로)는 없다. 근거 커밋: saba-chan 65e1763·8b840f7·60768a7, modules a26bdeb, extensions 301ff2c.

> **서버쨩(릴레이 서버)**: 사용자에게 보이는 이름으로는 클라우드 릴레이를 "서버쨩"이라 부른다. 사바쨩의 언니 캐릭터이고, 디스코드 봇을 돌리며 PC의 데몬과 디스코드 사이를 중계한다. 코드·API·설정 키(`relay_*`, `/api/relay/*`)와 이 문서의 본문은 계속 "릴레이"라는 이름을 쓴다.

## 기준 스냅샷

| 저장소 | 브랜치 · 커밋 | 커밋 날짜 |
|---|---|---|
| WareAoba/saba-chan | `claude/project-thread-qcza9f` `1f0028a` (CLI 절은 `claude/cli-remake-b1wszk`) | 2026-10-17 |
| WareAoba/saba-chan-modules | `claude/project-thread-qcza9f` `e1d7743` | 2026-10-17 |
| WareAoba/saba-chan-extensions | `claude/project-thread-qcza9f` `359741f` | 2026-10-17 |
| WareAoba/server-chan | `main` `4c423e8` (릴레이 쪽은 다시 만드는 중이라 갱신하지 않았다) | 2026-04-14 |

- 근거는 위 커밋이다. 이후 커밋이 쌓이면 해당 절의 file:line을 다시 확인해야 한다.
- 읽기만 했고 실행·테스트는 하지 않았다. "버그"로 적힌 항목은 코드 판독에 근거한 판단이며, 고치기 전에 재현부터 하는 게 맞다.
- 고친 문제는 지우지 않고 `~~예전 내용~~ → 해결: 지금 동작 (file:line)`으로 남긴다.

## 문서 구성

| 파일 | 다루는 것 |
|---|---|
| [01-daemon-core.md](01-daemon-core.md) | 데몬 기동, **IPC HTTP API 전체 라우트 표(약 95개)**, 인증, 인스턴스 모델·영속화, 전역 설정(`config.json`), 명령 경로(stdin/REST), 내장 Python, plugin 실행기 |
| [02-modules.md](02-modules.md) | `module.toml` 전체 스키마(Rust 역직렬화 기준), `lifecycle.py` 호출 프로토콜과 함수 매트릭스, managed process, 게임별 기능, 모듈 배포 |
| [03-extensions.md](03-extensions.md) | 플러그인(코드상 extension) `manifest.json` 스키마, hook 전수 목록과 hook×익스텐션 매트릭스, 설치·활성화 수명주기, GUI 컴포넌트 주입(UMD/slot) 계약 |
| [04-clients.md](04-clients.md) | GUI(Tauri 셸 + `window.api` 브리지) 전수표, CLI(하위 명령·대화형 셸·`--json`), 디스코드 연동(데몬 쪽), 공유 상수·로케일, **인터페이스가 데몬을 우회하는 지점 전수 목록(§7)** |
| [05-updater-servers.md](05-updater-servers.md) | 업데이터·인스톨러·언인스톨러·CI, server-chan relay API·DB·프로토콜 (운영 결함·비밀 파일은 server-chan 비공개 문서) |
| [06-contracts-0.1.9.md](06-contracts-0.1.9.md) | 0.1.9에 넣기로 하고 아직 구현 중인 계약(디스코드 복원, 플레이어 목록, 로그 읽기·내보내기, 서버 준비 알림, 복원 허용 범위). 구현이 머지되면 01·02·05로 옮긴다 |

---

## 1. 다섯 구성요소: 오너 정의와 코드상 실체

오너가 정한 역할(설계 원칙)과, 코드에서 그 역할을 맡은 실체를 나란히 둔다. 이후 모든 "위반" 판정은 왼쪽 열이 기준이다.

| 구성요소 | 오너 정의 | 코드상 실체 |
|---|---|---|
| **데몬** | 서비스 주체. 모든 기능은 여기로 작동 | `saba-chan/src` → 바이너리 `saba-core`. axum HTTP 서버, `127.0.0.1:57474`(`SABA_IPC_PORT`), 헤더 `X-Saba-Token` 인증 |
| **인터페이스** | 데몬 조작 도구. 혼자서는 아무것도 못함. 파일 수정도 데몬 API로 | `saba-chan-gui`(Tauri+React), `saba-chan-cli`(Rust, 하위 명령 + 대화형 셸). 둘 다 데몬 HTTP API 클라이언트 |
| **모듈** | 게임 서버 구동 "설명서", Python | `saba-chan-modules/<game>/{module.toml, lifecycle.py, locales/}`. 데몬이 `python lifecycle.py <함수명>`을 단발 실행(stdin JSON → stdout JSON) |
| **플러그인** | 데몬 기능 추가·인터페이스 컴포넌트 주입, Python | 코드에서는 **extension**이라 부름. `saba-chan-extensions/<name>/{manifest.json, *.py, gui/dist/*.umd.js}`. 데몬이 hook 시점에 같은 Python 실행기로 호출, GUI가 UMD 번들을 slot에 주입 |
| **서버쨩** | 데몬과 연계해 디스코드 봇 구동 + 봇 추가 기능 | 클라우드 릴레이에서만 돈다. 데몬은 relay_agent로 바깥으로 연결한다(04 §4). 로컬 봇(`saba-chan/discord_bot`)은 L1에서 지웠다 |

### 용어 충돌 (문서·코드 읽을 때 주의)

- `src/plugin/`은 오너가 말하는 "플러그인"이 **아니다.** 모듈과 익스텐션이 함께 쓰는 **Python 실행기**다. 오너의 "플러그인" = 코드의 `src/extension/` + `saba-chan-extensions`.
- `ipc::ExtensionInfo`, GUI의 `selectedExtension`은 이름과 달리 **모듈**을 가리킨다.
- 이 문서 세트에서는 오너 용어를 따르되 처음 나올 때 코드 이름을 병기한다: 플러그인(extension), 실행기(plugin).

## 2. 구성요소 간 연결 (코드 기준)

```
                ┌──────────────── 인터페이스 ────────────────┐
                │  GUI (Tauri)           CLI (셸)            │
                └────────┬───────────────────┬───────────────┘
                         │ HTTP + X-Saba-Token (127.0.0.1:57474)
                         ▼
   ┌──────────────────── 데몬 saba-core ───────────────────────┐
   │ IPC 라우터 ─ Supervisor ─ InstanceStore ─ ConfigStore      │
   │     │            │                                        │
   │     │            ├─ plugin 실행기: python <file>.py <fn>  ─┼─► 모듈 lifecycle.py
   │     │            │                                        ├─► 플러그인 hook (*.py)
   │     │            └─ managed process(stdin/stdout), REST ─► 게임 서버
   │     ├─ relay_agent ─(롱폴링, HMAC 서명, 바깥으로만)────────┼─► 클라우드 릴레이 ─ 사바쨩 봇 ─ Discord
   │     ├─ /api/relay/* 프록시(페어링·봇 초대·노드 조회) ───────┼─► 클라우드 릴레이
   │     └─ /api/updates/* ─ updater lib ─► 릴레이 + R2         │
   └───────────────────────────────────────────────────────────┘
```

- 게임 명령은 언제나 stdin(서버 콘솔)으로 보낸다. 데몬에는 RCON 클라이언트가 없고, 모듈은 모든 모드에서 게임의 RCON을 끈다. 콘솔이 없는 서버(관리하지 않는 모드)에는 saba-chan이 명령을 보낼 수 없다(02 RCON 정책).
- 로컬 Discord 봇은 L1에서 지웠다. 봇은 클라우드 릴레이에서만 돌고 데몬은 relay_agent로 붙는다(04 §4).
- 릴레이 쪽 봇(게이트웨이·음악·챗봇)은 server-chan에서 다시 만드는 중이라 이 문서에서 다루지 않는다.

## 3. 핵심 계약 요약 (상세는 각 문서)

| 계약 | 형식 | 상세 |
|---|---|---|
| 인터페이스 → 데몬 | HTTP JSON, `X-Saba-Token`(데몬이 `.ipc_token` 파일에 기록, 클라이언트가 읽음). `/api/updates/*`를 포함한 모든 라우트가 토큰을 요구하고, 토큰이 없으면 거부한다. loopback이 아닌 Host는 403. 데몬은 `daemon.lock`으로 한 개만 뜬다 | 01 §1.6, §2, §3 |
| 설정 | 사용자 설정은 `<data_dir>/config.json` 하나(네임스페이스 `gui`, `relay`, `updates`, `boot`, `extensions`, `chatbot`, 클라이언트별 영역). 원자적 저장과 `.bak` 복구, 예전 개별 파일은 한 번 이관하고 `.migrated` 사본을 남긴다 | 01 §4.4 |
| 데몬 → 모듈 | `python lifecycle.py <fn>`, stdin JSON(config) → stdout JSON(전체 또는 마지막 줄), stderr `PROGRESS:` 진행 보고. 데몬이 부르는 함수 17종. 환경변수 `SABA_IPC_URL`, `SABA_HOST_DIR`, `SABA_LANG`(IPC 토큰은 주지 않는다), `SABA_ENABLED_EXTENSIONS` | 02 §2 |
| 모듈 정의 | `module.toml`: `[module]`(필수) `[protocols]` `[config]` `[detection]` `[launch]` `[backup]` `[errors]` `[extensions.<id>]` `[[settings.fields]]` `[[commands.fields]]` `[install]`. `schema_version = 1`이면 잘못된 값에서 로드가 실패하고(모르는 키는 경고 뒤 무시, 0.1.11부터), 같은 구조체에서 JSON Schema를 내보낸다 | 02 §1.2 |
| 게임 명령 | 우선순위 stdin(서버 콘솔) → REST → RCON. RCON은 `rcon`만 받는 모듈에만 쓴다(127.0.0.1, 데몬이 만든 인스턴스별 비밀번호). 모듈의 콘솔 명령은 `method = "console"` + `console_template`(예전 이름 `rcon`은 별칭). `/api/instance/:id/rcon`은 이름만 남은 호환 경로로 콘솔 출력을 잠깐 모아 돌려준다. 콘솔 없는 서버는 409 `no_command_path`. palworld는 REST(`/rest`)만 쓴다. 정지도 RCON 없이 콘솔 명령이나 REST로 한다 | 01 §5.4, 02 RCON 정책 |
| 데몬 → 플러그인 | 같은 실행기로 hook 함수 호출. `FIRED_HOOKS` 목록(`daemon.startup/shutdown`, `server.pre_start/stop/status/list_enrich/post_create/pre_delete/settings_changed/check_update`, 익스텐션 명령 `command.<name>`). 목록의 hook이 모두 발화되는지 테스트가 확인한다. 순서는 `priority`, id 순 | 03 §2 |
| 플러그인 → GUI | GUI가 `/api/extensions/:id/gui` 번들을 로드, 전역 `window.SabaExt<Pascal>`의 `registerSlots()`로 slot에 컴포넌트 주입 | 03 §4 |
| 데몬 ↔ relay | 데몬 안의 relay_agent가 바깥으로 롱폴링하고 HMAC 서명으로 요청한다. 페어링·봇 초대·노드 조회는 데몬 `/api/relay/*` 프록시 | 04 §4, 05 §4 |
| 업데이트 | 데몬이 updater 라이브러리를 내장. 릴레이에서 Ed25519 서명 manifest를 받아 검증하고, 파일은 R2에서 SHA-256을 확인하며 받는다. 모듈(`module-*`)·익스텐션(`ext-*`)도 같은 manifest에 든다. 확인 주기는 데몬 스케줄러가 돈다. GUI/코어/업데이터 자신은 데몬이 `saba-chan-updater --apply`를 일회용 티켓과 함께 띄운다(`POST /api/updates/launch-updater`). 설치 후 무결성은 적용 때 기록한 파일별 해시로 비교한다 | 05 §1 |

## 4. 원칙 위반 종합 (오너 정의 기준)

### 4.1 인터페이스가 데몬을 우회 ("인터페이스는 혼자 아무것도 못한다" 위반)

1. GUI가 데몬 상태와 상관없이 `config.json`의 `gui`를 직접 읽는다. 쓰기는 데몬이 응답하지 않을 때(부트스트랩)만 한다(04 §7 G2–G4).
2. ~~GUI가 relay에 bot-config를 직접 PATCH한다~~ → 해결: 없어졌다(04 §7).
3. 기존 서버 폴더 → 모듈 판정을 GUI 마이그레이션 화면이 아직 직접 한다. 게임 명령 해석은 데몬(`/api/instance/:id/command`, `src/commands`)으로 옮겨졌다(02 §6.5).
4. ~~디스코드 봇 실행 명령을 GUI와 데몬이 따로 조립한다~~ 없어짐: 로컬 봇이 L1에서 지워졌다.
5. ~~CLI 로컬 fs 폴백·자체 설정 파일~~ CLI 리메이크(PR #6)에서 해결됨. 남은 것은 부트스트랩뿐이다(04 §7.3).
6. ~~업데이트 타이머와 업데이터 실행을 GUI가 한다~~ → 해결: 확인은 데몬 스케줄러가, 업데이터 실행은 데몬이 한다(05 §1.7).
7. 데몬 재기동과 언인스톨러 실행은 데몬 API가 없어 GUI가 직접 한다(04 §7.5).

부트스트랩상 불가피한 것(토큰 파일 읽기, 최초 데몬 spawn)은 예외다. GUI는 데몬을 끌 때 `/api/daemon/shutdown`을 먼저 쓰고, 강제 종료는 자기가 띄운 PID에만 한다.

### 4.2 데몬이 게임·플러그인 지식을 하드코딩 ("모듈=설명서, 플러그인=자유 교체" 위반)

- ~~플러그인 이름 하드코딩(`docker`, `steamcmd_*`, `music`)~~ → 해결: docker·music은 지워졌고, 설치 방식 필드는 `install_method_<방식>`, `<방식>_<이름>` 범용 규칙이 됐다(03 §7.3).
- 게임 지식 하드코딩: `java_path`, `ram`, `server.jar` 등이 데몬 공용 코드에 남아 있다. GUI도 `module === 'palworld'`로 분기한다(02 §6.5 #6–#8).
- 모듈이 플러그인을 `from extensions.x import`로 직접 import한다. 비활성 익스텐션 import는 실행기의 import 가드가 막는다. 모듈→데몬 역호출은 이제 토큰을 보낸다(02 §6.5 #9, #10).

## 5. 남은 문제 (판독 기준, 재현 전)

### 보안
- ~~`/health`도 토큰을 요구한다~~ → 해결: `/health`는 토큰 없이 최소 상태만, 진단은 인증이 필요한 `/api/diagnostics`(A6, 01 §2.2). ~~`/api/fs/scan-dir`는 아무 디렉터리나 나열한다~~ → 해결: 라우트를 지우고 게임 폴더 판정(`POST /api/modules/detect`)만 남겼다(D2). ~~토큰 비교가 상수 시간이 아니다~~, ~~`?relayUrl=`로 노드 토큰을 임의 호스트에 보낼 수 있다~~ → 해결: 상수 시간 비교, 설정된 릴레이나 loopback으로만 보낸다(01 §9.B).
- 운영 서버(server-chan)와 비밀 파일 관련 항목은 공개 저장소에 두지 않고 server-chan 비공개 문서에서 관리한다.

### 기능
- ~~업데이터 무결성 검사가 항상 "Tampered"~~, ~~GUI `updater:setConfig` 405~~, ~~모듈 역호출 401~~, ~~`FUNCTIONS` 표 누락~~, ~~`SABA_ENABLED_EXTENSIONS` 경로~~, ~~크래시 감지 없음~~, ~~명령이 꺼진 RCON으로 감~~, ~~업데이트 직후 인스턴스 삭제 위험~~, ~~GUI 종료 시 봇 정지 오류~~ → 모두 해결(각 문서의 해결 표시 참고).
- ~~`server.post_create`가 발화되지 않는다~~, ~~`auto_apply`가 동작하지 않는다~~(설정이 지워졌다), ~~CLI의 본문 없는 `apply`·`download` 요청이 거부된다~~ → 해결(03 §2.3, 05 §1.7).
- 데몬은 `/api/servers`의 `managed`로 관리 모드 여부를 알려 주고 GUI는 그 값을 따른다. REST 전용 서버(palworld)도 관리 모드라 GUI가 읽기 전용 콘솔로 서버 출력을 보인다(02 §1.2).
- 정지: 콘솔이 있는 managed 서버는 데몬이 `stop_command`를 stdin으로 보내고 기다린다. 모듈 `stop()`은 콘솔이 없을 때만 불린다(02 §6.3 #8).

### 구조 정리 대상
- ~~단일 인스턴스 보장 없음~~, ~~긴 Python 호출 동안 락 점유~~, ~~hook 순서 비결정~~, ~~발화되지 않는 hook·읽히지 않는 manifest 필드 대부분~~, ~~릴리즈 CI가 이전 릴리즈 삭제~~ → 해결.
- manifest의 `module_config_section`은 여전히 읽히지 않는다(03 §7.1). `server.list_enrich`는 인스턴스마다 순차로 돈다(03 §7.2).

## 6. 문서 유지 규칙

이 문서가 계속 "믿을 수 있는" 상태로 남으려면:

1. **근거는 코드뿐**이다. 모든 계약 주장에는 `file:line`을 단다. 문서끼리 서로를 근거로 삼지 않는다.
2. 계약(라우트, `module.toml` 키, hook 이름, manifest 필드, relay API)을 바꾸는 커밋은 같은 커밋에서 해당 절을 고친다.
3. 고친 문제는 §5와 각 문서의 "불일치" 절에서 지우고 커밋 해시를 남긴다.
4. 기준 스냅샷 표의 커밋을 갱신한다. 표보다 오래된 줄 번호는 다시 확인 전까지 "대략"으로 취급한다.
5. 이 세트가 기존 README/docs를 대체한다. 여기 없는 내용을 다른 문서에서 가져와 사실로 쓰지 않는다.
