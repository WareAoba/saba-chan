# 01. 데몬 코어 (saba-core) 분석

> 범위: `src/main.rs`, `src/lib.rs`, `src/ipc/**`, `src/instance`, `src/instance_lock.rs`, `src/config_store`, `src/plugin`, `src/python_env`, `src/validator`, `src/process_monitor.rs`, `src/boot_selector.rs`, `src/daemon_log.rs`, `src/utils.rs`, `src/protocol/**`, `src/relay_agent.rs`, `tests/daemon_integration.rs`.
> 모든 내용은 코드에서 도출했다. 경로/상수 대부분은 공유 크레이트 `updater/src/constants.rs`(`saba_chan_updater_lib::constants`)에서 온다.
> Supervisor/module_loader, extension/mod.rs 내부는 다른 문서 담당이며, 여기서는 코어가 그것들을 어떻게 호출하는지만 적는다.


> **2026-10-17 L3 이후**: 업데이트·모듈 목록·익스텐션 목록은 모두 릴레이의 서명된 릴리즈 manifest 하나에서 나온다. GitHub 업데이트 경로(`github.rs`, `api_base_url`, `github_owner`/`github_repo`, `include_prerelease`, 익스텐션 `manifest_url`, module.toml `[update]`, modules·extensions 저장소의 릴리즈 워크플로)는 없다. 근거 커밋: saba-chan 65e1763·8b840f7·60768a7, modules a26bdeb, extensions 301ff2c.

## 0. 프로젝트 오너의 설계 원칙 (비교 기준)

오너가 밝힌 설계 의도는 아래와 같다.

- **데몬(saba-core)이 서비스의 주인이다.** 모든 기능은 데몬을 거친다.
- **인터페이스(GUI/CLI)는 스스로 아무것도 하지 않는다.** 파일 편집도 데몬 API를 거쳐야 한다.
- **모듈**은 게임 서버를 운영하는 Python "매뉴얼"이다(`lifecycle.py`).
- **플러그인(=익스텐션)**은 Python으로 작성된다. 데몬에 기능을 더하고 인터페이스에 컴포넌트를 주입한다.
- **server-chan**(Discord 봇)은 데몬과 함께 돌면서 데몬에 기능을 더한다.

§9.A에서 이 원칙을 어기는 지점을 따로 모았다.

---

## 1. 기동 시퀀스, 부트 모드, CLI 인자/환경변수, 포트, 파일 레이아웃

### 1.1 `main()` 흐름 (`src/main.rs:182-647`)

1. **인자 처리** (`main.rs:187-209`)
   - `module schema|validate <폴더>...`와 `extension validate <폴더>...`는 데몬을 띄우지 않고 검사 결과만 찍은 뒤 종료 코드(실패 1, 사용법 오류 2)로 끝난다(`main.rs:39-152`).
   - 나머지 인자는 `daemon_args`(`main.rs:168-180`)가 본다. `--spawned`/`--after-update`, `--version`/`-V`, `--help`/`-h`만 받는다.
   - 모르는 인자는 **데이터 폴더를 건드리기 전에** 사용법을 찍고 종료 코드 2로 끝낸다(`main.rs:205-208`).
2. **부트 모드 결정** (`main.rs:210-220`)
   - `--spawned` 인자가 있거나, stdin과 stdout 중 하나라도 터미널이 아니면 부트 선택기를 건너뛴다(`boot_mode = None`).
   - 그 외에는 `boot_selector::run()`이 `Gui | Cli | DaemonOnly` 중 하나를 고른다.
3. **로그 버퍼와 tracing 초기화** (`main.rs:222-278`)
   - `DaemonLogBuffer`(링 버퍼 10,000줄, `daemon_log.rs:21`)와 `DaemonLogLayer`를 모든 모드에 합성한다.
   - 필터는 `RUST_LOG`이며 기본값은 `info`다.
   - CLI 모드는 CLI 대화형 셸이 터미널을 차지하므로 로그를 `<data_dir>/daemon.log` 파일로 보낸다(truncate, `main.rs:239-247`). 나머지 모드는 stderr로 보낸다.
4. **IPC 포트 결정** (`main.rs:285-298`)
   - `SABA_IPC_PORT` 환경변수, 없으면 `config.json`의 `gui.ipcPort`, 둘 다 없으면 `DEFAULT_IPC_PORT = 57474`(`constants.rs:32`)다.
   - **바인드 주소는 `127.0.0.1`로 고정이다.** 포트는 `plugin::set_ipc_port`로 Python 실행 환경에도 넘긴다.
5. **단일 인스턴스 잠금** (`main.rs:300-315`): `<data_dir>/daemon.lock`에 OS 배타 잠금을 건다(`instance_lock.rs:69-72`). 이미 다른 데몬이 잡고 있으면 그 pid·주소를 찍고 종료 코드 2로 끝낸다. 잠금 파일을 열 수 없는 환경이면 경고만 하고 진행한다. §1.6 참조.
6. `launch_ticket::record_core_version`으로 코어 버전을 기록하고(`main.rs:318`), Python 런타임 준비를 백그라운드로 시작한다(`main.rs:320-324`, 진행은 `/api/runtime/preflight`).
7. **Supervisor 생성과 초기화** (`main.rs:328-343`)
   - `plugin::resolve_modules_dir()`로 모듈 경로를 정하고 `Supervisor::new()`, `initialize()` 순으로 호출한다.
   - `initialize()`는 모듈 탐색, 인스턴스 로드, 고아 managed 프로세스 재연결(`managed.pid`)을 한다(`supervisor/mod.rs:154-170`).
   - `server_version` 백필은 모듈 Python 호출이라 기동을 막지 않게 별도 task로 돈다(`main.rs:343`).
8. **IPC 토큰 생성** `ipc::auth::generate_and_save_token()` (`main.rs:346-349`). 실패해도 경고만 하고 진행한다.
9. **IPC 서버 객체 생성** (`main.rs:352`): `IPCServer::new`(`ipc/mod.rs:584-627`)가 ExtensionManager를 만들고 `discover()`를 호출한다. ConfigStore(`config.json`)도 이때 만든다.
10. Supervisor에 `extension_manager`와 `provision_tracker`를 주입한다(`main.rs:355-359`).
11. **백그라운드 태스크 기동**
    - 익스텐션 `daemon.startup` 훅 비동기 디스패치(`main.rs:364-410`). 진행 상황은 `ExtensionInitTracker`에 기록되고 `/api/extensions/init-status`로 노출된다.
    - 프로세스 모니터: 2초 주기로 `process_monitor::get_running_processes_async()` 스냅샷을 찍고 `sup.monitor_processes()`를 호출한다(`main.rs:416-463`). 이때 supervisor **write lock**을 잡는다. 크래시로 끝난 서버는 `take_pending_restarts`로 모아 락 밖에서 `server::auto_restart`로 다시 띄운다(`main.rs:456-461`).
    - 업데이트 주기 확인 `spawn_update_scheduler`(`main.rs:466`)와 인스턴스 예약 작업(백업·재시작·공지·업데이트) 실행기 `backup::spawn_scheduler`(`main.rs:468`, 30초 주기)를 띄운다.
    - 하트비트 reaper: 30초 주기로 90초 넘게 하트비트가 없는 클라이언트를 정리한다(`main.rs:483-497`, `handlers/client.rs:116-136`).
    - 렌더러 watchdog은 없다. 인터페이스가 모두 떠나도 데몬은 남고 GUI를 다시 띄우지 않는다. 다음에 켠 GUI·CLI가 다시 붙는다(C14).
    - DaemonOnly 모드: `daemon_log::daemon_terminal_loop`가 500ms마다 각 인스턴스 콘솔을 stderr에 출력한다(`daemon_log.rs:199`).
    - 종료 경로는 하나다(`main.rs:511-578`). Ctrl+C, SIGTERM(Unix) 또는 `stop_requested`(API `/api/daemon/shutdown`, `unregister?shutdown=true`, `launch-updater`)를 받으면 `daemon.shutdown` 훅(30초 상한), 클라이언트 정리, `shutdown_token` cancel 순으로 정리한다.
12. **인터페이스 스폰** (`main.rs:580-638`)
    - IPC 포트가 열릴 때까지 최대 15초 기다린다(`boot_selector::wait_for_ipc_port`).
    - GUI 모드: `saba-chan-gui --spawned-by-daemon`을 detach로 띄우고 콘솔 창을 숨긴다(Windows만).
    - CLI 모드: `saba-chan-cli`를 stdio를 상속해 같은 콘솔에 붙인다. 인자 없이 터미널에서 열린 CLI는 대화형 셸로 들어간다(`saba-chan-cli/src/main.rs:128-132`, PR #6 기준).
    - 인터페이스 바이너리는 데몬 exe와 같은 디렉토리에서만 찾는다(`boot_selector.rs:445-462`).
13. **`ipc_server.start().await`** (`main.rs:641`): 이 호출이 블로킹 메인 루프다. 바인드에 성공하면 릴레이 에이전트 자동 시작을 띄우고(`ipc/mod.rs:860-866`), `shutdown_token`이 cancel되면 graceful shutdown한다.

### 1.2 부트 선택기 (`src/boot_selector.rs`)

- 마지막 선택을 `config.json`의 `boot` 네임스페이스에 `{"last_mode":"gui"|"cli"|"daemononly"}` 형태로 저장한다(`boot_selector.rs:58-71`). `daemon_only` 별칭도 받는다.
- 5초 카운트다운 동안 DEL 키를 누르면 메뉴가 열린다(↑/↓, 1-3, Enter).
- **키 입력은 Windows에서만 구현돼 있다.** 비 Windows에서는 `poll_key_fallback`이 그냥 sleep하므로(`boot_selector.rs:244-248`) 메뉴를 열 수 없고, 항상 마지막 모드로 자동 부팅한다. 화면의 "Press [DEL]" 안내(`:308`)만 남아 있다.

### 1.3 환경변수 목록 (코어가 읽는 것)

| 변수 | 위치 | 의미 |
|---|---|---|
| `SABA_IPC_PORT` | `main.rs:286` | IPC 포트 (없으면 `gui.ipcPort`, 기본 57474) |
| `RUST_LOG` | `main.rs:233` | tracing 필터 |
| `SABA_DATA_DIR` | `constants.rs:56`, `python_env/mod.rs:397` | 데이터 루트 |
| `SABA_TOKEN_PATH` | `constants.rs:83` | IPC 토큰 파일 경로 |
| `SABA_MODULES_PATH` | `constants.rs:95` | 모듈 디렉토리 |
| `SABA_EXTENSIONS_DIR` | `constants.rs:107` | 익스텐션 디렉토리 |
| `SABA_INSTANCES_PATH` | `constants.rs:119` | 인스턴스 루트 |
| `SABA_AUTH_DISABLED=1` | `ipc/auth.rs:120` | 토큰 검사 비활성화. **디버그 빌드에서만** 듣는다 |
| `SABA_PYTHON_URL`, `SABA_PYTHON_SHA256` | `updater/src/python_pin.rs:32-38` | 포터블 Python 미러 주소와 그 해시 |
| `SABA_UPDATER_PATH` | `ipc/updates.rs:746-748` | 업데이터 실행 파일 경로 |
| `SABA_LOCALES_DIR` | `chat/i18n.rs:109` | 채팅 응답 로케일 폴더 |
| `APPDATA`, `HOME` | 경로 해석 | 플랫폼 기본 경로 |

**데몬이 자식 프로세스에 넘기는 환경변수**

- Python 모듈/익스텐션: `PYTHONIOENCODING`, `PYTHONUNBUFFERED`, `PYTHONPATH`, `SABA_IPC_PORT`, `SABA_IPC_URL`, `SABA_TOKEN_PATH`, `SABA_LANG`(`config.json`의 `gui.language`, 비었으면 OS 언어, 실행마다 읽음), `SABA_ENABLED_EXTENSIONS`(`plugin/mod.rs:14-38, 304-322`), `SABA_JAVA_RUNTIMES`(데몬이 찾은 Java 런타임 `[{java, home, version, major, arch}]`, 02 §2.1). 토큰 값 자체는 넘기지 않는다. 공용 `modules/daemon_rcon.py`는 `SABA_TOKEN_PATH` 파일을 읽어 `X-Saba-Token`을 붙인다(`modules/daemon_rcon.py:48-54`).
- `SABA_LANG`는 GUI도 데몬에 넘기지만(`saba-chan-gui/src-tauri/src/lifecycle.rs:177`) **데몬 코어는 이 환경변수를 읽지 않는다.** 언어는 `config.json`의 `gui.language`에서 읽고, 비었으면 OS 언어를 지원 언어로 맞춰 쓴다(`ConfigStore::ui_language`, `chat::i18n::os_language`. 중국어는 Hant·TW·HK·MO면 zh-TW, 그 밖은 zh-CN).

### 1.4 디스코드 릴레이 연결 (`src/relay_agent.rs`)

로컬 Discord 봇 실행 경로(`node_env`, `discord_bot` ext-process)는 L1(7b5102f)에서 지웠다. 디스코드 봇은 클라우드 릴레이에서만 돌고, 데몬은 릴레이 에이전트로 붙는다.

1. IPC 서버가 뜬 뒤 `relay_agent::auto_start`를 띄운다(`ipc/mod.rs:860-866`). 수동 제어는 `/api/relay/agent{,/start,/stop}`. 페어링 완료(`/api/relay/pair/:code/status`가 `claimed`로 토큰을 저장)나 `PUT /api/config/node-token`(저장된 토큰과 다를 때만) 뒤에는 `relay_agent::restart`가 돌던 세션을 멈추고 새 토큰으로 바로 다시 연결한다. 돌고 있는 에이전트의 릴레이 주소가 `PUT /api/config/bot`으로 바뀌어도 같다. 세션은 시작할 때 읽은 주소·토큰 쌍만 쓰고(`Session.token`, `signed_headers_with`), 끝난 세션은 요청을 보내지 않는다.
2. `GET /info`로 최소 에이전트 버전을 확인하고, 60초마다 `POST /heartbeat`로 서버·모듈·봇 설정 메타데이터를 올린다(`relay_agent.rs:23`).
3. `GET /poll`(롱폴링)로 디스코드 명령을 받아 채팅 처리기(`crate::chat`)로 실행하고 `POST /result/:id`로 돌려준다.
   - 같은 연결에서 `POST /events {events:[{id, type, data}]}`로 디스코드 알림 채널(I3)에 띄울 이벤트를 올린다(`relay_agent.rs` `events_loop`·`relay_event`). 올리는 것은 `instance.exit`(`crash`·`unknown_exit`만), `backup.done`, `update.available`, `schedule.warn`, `plan.done`(뒤에서 마친 여러 단계 실행: `runId, guildId?, ok, steps, stoppedAt`, 단계 줄에는 원격이라 오류 코드만)뿐이다. 원격으로 나가므로 경로·콘솔 줄·오류 원문은 뺀다(`evidence`는 `exit code …`·`signal …`일 때만, `backup.done`의 `error`는 빼고 `ok`만). 첫 건이 오면 2초 모아 20건씩 보내고, 닿지 못하면 백오프하며 최근 50건만 들고 있는다. 4xx(429 말고)는 그 묶음을 버리고 429·5xx·네트워크 오류는 백오프 뒤 다시 보내며, 404면(relay에 경로가 아직 없음) 그 묶음을 버리고 10분 동안 올리지 않은 뒤 다음 이벤트로 다시 알아본다.
4. 인증은 노드 토큰(`<data_dir>/.node_token`) HMAC 서명이다(`relay_agent.rs:1-11`).

### 1.5 파일/디렉토리 레이아웃 (`constants.rs`가 단일 진실 원천)

`<data_dir>`는 다음 순서로 정해진다.

1. `SABA_DATA_DIR`
2. Windows: `%APPDATA%/saba-chan`
3. Unix: `$HOME/.config/saba-chan`
4. 그 외: `./saba-chan`

```
<data_dir>/
  .ipc_token                IPC 인증 토큰 (uuid v4, Unix 0600)          auth.rs:27-43
  .node_token               cloud relay 노드 토큰 (sbn_<id>.<secret>)   handlers/config.rs, relay.rs
  config.json (+ .bak)      모든 사용자 설정 (gui, relay, updates, boot, extensions, chatbot, cli 등)
                            updater/src/config_file.rs
  settings.json 등           예전 개별 파일. 한 번 config.json으로 이관하고 원본은 두며 .migrated 사본을 남긴다
  daemon.lock (+ .owner)    단일 인스턴스 잠금과 실행 중 데몬 pid·주소    instance_lock.rs
  daemon.log                CLI 부트 모드에서만 생성
  python-guard/sitecustomize.py   활성 익스텐션만 import하게 하는 가드    plugin/mod.rs:113-125
  installed-manifest.json, updater-state.json, update-complete.json (updater)
  modules/<module>/module.toml, lifecycle.py, locales/*.json
  modules/i18n.py, daemon_rcon.py   데몬에 내장된 공용 Python 유틸리티 (plugin/mod.rs:127-147)
  extensions/<ext>/...
  instances/
    order.json
    <uuid>/instance.json, settings.json, managed.pid, logs/console.log, backups/, server/ (설치 대상)
<exe_dir>/
  saba-chan-gui[.exe], saba-chan-cli[.exe]   부트 선택기가 스폰
  saba-chan-updater[.exe]   /api/updates/launch-updater가 스폰
```

- 업데이트 스테이징 디렉토리: Windows는 `<data_dir>/updates`, Unix는 `~/.cache/saba-chan/updates`(`constants.rs:149-166`).
- Python 런타임은 **위의 `<data_dir>`와 다른 경로**를 쓴다(§6).

### 1.6 단일 인스턴스 처리

- `<data_dir>/daemon.lock`에 OS 배타 잠금을 건다(`instance_lock.rs:1-8, 75-99`). 잠금은 프로세스가 끝나면 OS가 풀어 주므로 비정상 종료 뒤에도 남지 않는다. 실행 중 데몬의 pid와 주소는 옆의 `daemon.lock.owner`에 쓴다.
- 잠금은 토큰 생성·Supervisor 초기화보다 먼저 잡는다(`main.rs:300-315`). 그래서 두 번째 데몬은 `.ipc_token`을 덮어쓰기 전에 종료 코드 2로 끝난다.
- TCP 바인드는 2초 간격으로 10회 재시도한다(`ipc/mod.rs:833-888`). `SO_REUSEADDR`는 **Unix에서만** 켠다. Windows의 `SO_REUSEADDR`는 이미 리슨 중인 포트에도 바인드를 허용하므로 켜지 않는다(`ipc/mod.rs:845-848`).
- 잠금 파일을 열 수 없는 환경(읽기 전용 등)에서는 잠금 없이 진행하므로, 그때는 바인드만이 방어선이다.

---

## 2. IPC HTTP API 전체 표

### 2.1 공통 규약

- **라우터 정의 위치**: `src/ipc/mod.rs:640-817` (axum 0.7), 업데이트 라우트는 `src/ipc/updates.rs:106-117`.
- **경로 파라미터 표기**: axum 0.7의 `:param` 문법을 쓴다.
- **인증 적용 범위**: `.layer(auth_middleware)`는 업데이트 라우터를 `.merge()`한 **뒤에** 달린다(`ipc/mod.rs:812-816`).
  - 따라서 `/api/updates/*`를 포함한 모든 라우트에 토큰이 필요하다. **`/health`만 예외**다(A6): 루프백 검사는 거치지만 토큰 없이 답한다. 토큰이 어긋나도 클라이언트가 데몬이 떠 있는지 알 수 있게 하려는 것이다.
  - 미들웨어는 토큰보다 먼저 `Host` 헤더가 루프백(`127.0.0.1`, `localhost`, `::1`)인지 보고, 아니면 본문 없는 403을 돌려준다(`auth.rs:90-117`). DNS 리바인딩 방어다.
- **에러 형식이 통일돼 있지 않다.** 다음 형식들이 섞여 있다.
  - `{"error": "..."}` (+ 선택적으로 `"error_code"`)
  - `{"ok": false, "error": ...}` (updates, modules manifest/install/remove, `/api/config/:kind/*`는 성공 시 ok)
  - `{"success": false, "error", "error_code", ...}` (익스텐션, 백업, 이름 변경 등)
  - 인증 실패: 본문 없는 401, Host 거부: 본문 없는 403
  - 일부 핸들러는 실패해도 HTTP 200을 돌려준다(modules manifest/install/remove, extensions manifest/install).
- **JSON 바디 파싱 실패**: axum 기본 거부 응답(400/415/422, 텍스트 본문)이 나간다. `/api/updates/download`·`apply`만 본문을 직접 읽어 `{ok:false, error, error_code:"invalid_body"}` 400을 준다.
- **식별자 혼용**: 예전 경로는 서버를 **이름**(`/api/server/:name/*`, `/api/provision-progress/:name`)으로, 나머지는 인스턴스 **UUID**(`/api/instance/:id/*`)로 가리킨다. `/api/instance/:id/start|stop|restart`는 id가 없으면 이름으로도 찾는다(`server.rs:667-671, 767-774`).

### 2.2 라우트 표

아래 표의 모든 라우트는 `X-Saba-Token` 헤더가 필요하다.

#### 헬스 / 서버 제어 (handlers/server.rs)

| Method | Path | Handler | 요청 | 응답 |
|---|---|---|---|---|
| GET | `/health` | `server.rs:19` `health_check` | - | `{ok:true, version, pid, token}`. 토큰 없이 부를 수 있다. `token`은 보낸 `X-Saba-Token`이 `valid`·`invalid`·`missing` 중 무엇인지(값은 드러내지 않는다) |
| GET | `/api/diagnostics` | `diagnostics.rs:20` | - | A6, 인증 필요. `{ok, version, pid, uptime_secs, listen_addr, data_dir, token_file:{path, exists}, audit_log, extensions:{enabled, enabled_but_missing}, modules:{count}\|{error}, recent_problems:[최근 경고·오류 20개, 오래된 순]}` |
| GET | `/api/onboarding` | `diagnostics.rs` `onboarding` | - | J1-a 다음 할 일. `{steps:[{id, done, optional}], next}`. 단계 `module`(모듈 있음)·`server`(서버 있음)·`discord`(노드 토큰 있음)·`chatbot`(챗봇 API 키, 선택). `next`는 안 한 첫 필수 단계 id, 다 했으면 null. 문구는 클라이언트가 id로 고른다. relay 하트비트 `metadata.onboarding`에도 같은 값이 실린다 |
| GET | `/api/audit` | `diagnostics.rs` `audit` | `?limit=`(기본 100, 최대 1000)`&instance=<id>` | A10 감사 로그, 최신이 먼저: `{entries:[{ts, source, user?, userName?, guild?, guildName?, action, instanceId, instance?, command?, args?, ok, status}]}`. 비밀은 `***`로 가린다(아래 `audit.rs`). `action`은 `start`·`stop`·`restart`·`command`(`/command`·`/rcon`·`/rest`)·`stdin`·`properties`·`reset`, 예약 백업은 `backup`(출처 `schedule`, `status` 200 만듦·204 백업할 것 없음·409 서버가 꺼져 있어 건너뜀·그 밖은 실패), 액션 카탈로그의 관리 액션은 그 액션 id(예 `instance.settings.set`) |
| GET | `/api/events` | `events.rs` `stream` | 헤더 `Last-Event-ID` 또는 `?since=<n>` | P4 이벤트 스트림(SSE). 한 건은 `id`(데몬이 켜진 뒤 1부터)·`event`(종류)·`data`(`{at, ...}` JSON). 최근 200건을 메모리에 두고 `since` 뒤 것을 먼저 보낸다. 번호가 버퍼보다 오래됐거나 데몬 재시작으로 커졌거나 느려서 놓친 건이 있으면 `event: resync`. 15초마다 `:ping`. 종류: `action.run`(감사 로그 한 줄과 같은 모양, `ts` 대신 `at`), `instance.exit`(`instanceId, instance, kind, exitCode, signal, evidence, restartInSecs, note, noteCode, noteArgs`, `noteCode`·`noteArgs`는 `/api/instance/:id/events`의 `note_code`·`note_args`와 같다), `instance.restart_failed`(`instanceId, instance, noteCode:"restart_failed", noteArgs:{reason}`, 크래시 뒤 자동 재시작이 실패했을 때. 오류 원문이 있어 relay로 올리지 않는다), `provision.progress`(`instance` + `/api/provision-progress` 필드. 지금 붙은 클라이언트에게만 보내고 `id`가 없다: 버퍼에 남지 않아 다시 붙을 때의 기록을 밀어내지 않는다), `backup.done`(`instanceId, instance, kind, name?, ok, error?`, `name`은 백업 파일 이름), `update.available`(`components:[표시 이름]`, `/api/updates/check`의 `update_names`와 같다. 보이는 업데이트(구성요소·최신 버전)가 지난번과 달라졌을 때만), `schedule.warn`(`instanceId, instance, action:"restart"|"update", minutesLeft`, 예약 재시작·업데이트 예고 시각마다), `plan.done`(`runId, guildId?, ok, steps, stoppedAt`, 40초 안에 끝나지 않아 뒤에서 마친 여러 단계 실행), `relay.link_request`(`requestId, guildId, guildName?, requestedBy?, userName?, expiresAt?`(ISO 8601 UTC 문자열), 디스코드에서 코드 없이 온 연결 요청. relay로 다시 올리지 않는다). 데몬 종료(`shutdown_token`)에 스트림도 끝난다 |
| GET | `/api/version` | `server.rs:306` | - | `{version, components:{saba-core, module-<이름>, ext-<id>, ...}}` |
| GET | `/api/servers` | `server.rs:26` `list_servers` | - | `{servers:[ServerInfo], port_conflict_stops?:[...]}` (아래 설명) |
| GET | `/api/server/:name/status` | `server.rs:620` | - | supervisor 결과 JSON 그대로. 404 시 `{error, error_code:"instance_not_found"}` |
| POST | `/api/server/:name/start` | `server.rs:881` | `{module: string (필수), config?: any}` | supervisor 결과. 에러 시 404/422/500 + `error_code`∈{instance_not_found, executable_missing, start_failed}, 409 `instance_busy`(같은 인스턴스의 백업·복원·업데이트 중) |
| POST | `/api/server/:name/stop` | `server.rs:938` | `{force?: bool}` (바디 필수) | supervisor 결과. 에러 시 `error_code`∈{instance_not_found, stop_failed} |
| POST | `/api/instance/:id/start` | `server.rs:654` | - | 데몬이 고른 경로(managed/non-managed, `Supervisor::uses_managed_start`)의 응답 + `managed`. 404 `not_found`, 409 `agreement_required`(`agreements`: 동의하지 않은 모듈 약관, validate보다 먼저), 409 `preflight_failed`(모듈 validate의 critical 문제), 409 `instance_busy`(백업·복원·업데이트 중), 422 `memory_limit_reached`(`limit_mb`: 시작하는 중 러너가 메모리 상한으로 끊음. 종료 이력과 `instance.exit`에 `start_failed`로도 남는다) |
| POST | `/api/instance/:id/stop` | `server.rs:785` | 선택 `{force?}` | `/api/server/:name/stop`과 같다 |
| POST | `/api/instance/:id/restart` | `server.rs:796` | 선택 `{force?}` | 정지(최대 2분 대기) 뒤 시작 응답 + `stopped` |
| GET | `/api/instance/:id/events` | `server.rs:872` | - | `{events:[...]}` 최근 종료 이력 / 404. 한 건은 `at, kind, exit_code, signal, evidence, restart_in_secs, note`에 다시 띄우지 않은 이유가 있으면 `note_code`(`auto_restart_off` · `gave_up` + `note_args.count` · `restart_failed` + `note_args.reason`)·`note_args`. `note`는 영어 문장이라 화면 문구는 `note_code`로 고른다 |

`GET /api/servers` 응답의 `ServerInfo` 필드(`ipc/mod.rs:160-209`):

- 기본: `id, name, module, status, pid, start_time, executable_path, working_dir?, port`
- 접속 정보: `rest_host, rest_port, rest_username, rest_password, protocol_mode`
- 설정과 상태: `module_settings, server_version, last_api_action?, provisioning, managed, extension_status?, extension_data, port_conflicts?`
- `slots?`: 익스텐션 데이터 slot을 데몬이 판정한 결과(`{"ServerCard.badge":[{extension,text}], "ServerDetail.status":[{extension,label,value}]}`, 없으면 키가 없다). 규칙은 03-extensions 4.3.
- `players?`: `{online, max?}`. `status`가 `running`이고 모듈이 `[status] players`로 알려 준 마지막 접속자 수(1분마다 갱신, `supervisor/players.rs`, 02 §1.2). 모르면 키가 없다.
- `managed`는 saba-runner가 맡아 실행하는지다. 판정은 데몬의 `Supervisor::uses_managed_start` 하나이고(`server.rs:87`), 클라이언트는 이 값을 그대로 쓴다.
- **REST 비밀번호가 평문으로 포함된다.**

`port_conflict_stops`는 한 번 읽으면 비워지는 drain 큐다(`server.rs:277-281`).

`status` 값은 다음 중 하나다.

- `running` / `stopped`: PID 기반
- 익스텐션 `server.list_enrich` 훅이 `handled:true`를 돌려준 경우: 그 훅이 준 값
- `provisioning`: 프로비저닝 진행 중

#### 모듈 (handlers/server.rs, handlers/managed.rs, handlers/config.rs)

| Method | Path | Handler | 요청 | 응답 |
|---|---|---|---|---|
| GET | `/api/modules` | `server.rs:401` | - | `{modules:[ExtensionInfo], failed?:[...]}` (아래 설명) |
| POST | `/api/modules/refresh` | `server.rs:408` | - | 위와 같음 |
| GET | `/api/modules/schema` | `server.rs:419` | - | module.toml JSON Schema |
| POST | `/api/modules/detect` | `server.rs:425` | `{path}` | `{matches:[{name, display_name}]}` (`dir_signatures`가 맞는 모듈) / 400 / 404 |
| GET | `/api/modules/manifest` | `server.rs:502` | - | `{ok, manifest}` 또는 `{ok:false,error}`. 항상 200 |
| POST | `/api/modules/manifest/:id/install` | `server.rs:525` | - | `{ok, module_id, version}` 또는 `{ok:false,error}`. 항상 200. 카탈로그 버전이 설치된 버전보다 낮으면 받지 않는다 |
| DELETE | `/api/modules/:id` | `server.rs:593` | - | `{ok, id}`. 항상 200 |
| GET | `/api/module/:name` | `server.rs:467` | - | `{name, version, description, path, metadata}` / 404 |
| GET | `/api/module/:name/versions` | `managed.rs:375` | query `include_snapshots, page=1, per_page=25` | supervisor 결과 |
| GET | `/api/module/:name/version/:version` | `managed.rs:404` | - | supervisor 결과 |
| POST | `/api/module/:name/install` | `managed.rs:475` | `{version*, install_dir*, jar_name?, accept_agreements?:[id], initial_settings?}` (`accept_agreements`는 원격이면 403 `agreement_local_only`, 모듈이 선언하지 않은 id면 400 `invalid_format`) | 200(success) / 422 / 404 / 400 / 500 |
| GET | `/api/module/:name/locales` | `config.rs:201` | - | `{<lang>: <json>}` |

- `ExtensionInfo`(이름과 달리 모듈 정보다, `ipc/mod.rs:237-287`) 필드: `name, version, description, schema_version, display_name?, game_name?, author?, warnings?, unmet_dependencies?, path, executable_path, icon(base64 data URL), interaction_mode, protocols{supported,default}, settings, commands, syntax_highlight, dir_signatures?, install?, module_aliases?, command_aliases?`
- `/api/modules/manifest`는 최신 릴리즈 서명 manifest의 `module-*` 컴포넌트를 `{ok, manifest:{modules:{<이름>:{version, sha256, size_bytes, display_name, ...}}}}`로 돌려준다(`relay::latest_catalog`, `handlers/server.rs:499-522`).
- `/api/modules/manifest/:id/install`은 같은 manifest의 `module-<id>`를 R2에서 받아 SHA-256을 확인하고(`download_verified`), staging에 푼 뒤 데몬이 읽을 수 있는 모듈일 때만(`load_module_dir`) `modules/<id>`와 바꾸고(기존 폴더는 바꾼 뒤에 지운다) refresh한다(`:524-591`). `id`의 `..`, `/`, `\`는 거부한다.

#### 인스턴스 (handlers/instance.rs)

| Method | Path | Handler | 요청 | 응답 |
|---|---|---|---|---|
| GET | `/api/instances` | `instance.rs:12` | - | `[ServerInstance]` (§4.1, REST 비밀번호 평문 포함) |
| POST | `/api/instances` | `instance.rs:84` | `{name*, module_name*, executable_path?, working_dir?, migration_source?, port?, preset?, module_settings?, accept_agreements?:[id], extension_data?:{}}` (`accept_agreements`: 동의한 모듈 약관. 선언 안 된 id는 400 `invalid_format`, 원격이면 403 `agreement_local_only`. 가져오기(`migration_source`)면 만든 직후 lifecycle `accept_agreements`를 불러 모듈이 반영하게 한다. `extension_data`: 익스텐션 `AddServer.options`의 답. null이 아닌 값을 새 인스턴스에 넣고, 객체가 아니면 400 `invalid_format`) | 201 `{success, id, provisioning?, module_trust, warnings[]}`. 모듈이 `verified`가 아니면 `warnings`에 `{code:"module_unverified"\|"module_modified", module}` 한 건(알림일 뿐 만들기는 막지 않는다). 409 `working_dir_in_use`, 422 `extension_required`, 400 `preset_not_found`, 400/500 |
| PUT | `/api/instances/reorder` | `instance.rs:19` | `{order:[id]}` | `{success:true}` / 400 |
| GET | `/api/instance/:id` | `instance.rs:49` | - | `ServerInstance` + `agreements:[{id, label, url, accepted, accepted_at}]`(모듈 약관, 이름은 데몬 언어) / 404 `{error, error_code}` |
| DELETE | `/api/instance/:id` | `instance.rs:789` | - | `{success}` / 404 / 409 `instance_busy`(백업·복원·업데이트 중)·`server_running`(돌던 서버를 끝내지 못함: 아무것도 지우지 않는다) / 500 |
| PATCH | `/api/instance/:id` | `instance.rs` `update_instance_settings` | 키-값 객체. `working_dir`는 작업 폴더로. `extra_args`(문자열은 `chat::split_quoted`로 나눔, 또는 문자열 배열, null은 비우기)는 인스턴스의 추가 실행 인자로. `memory_limit_mb`(MiB 정수 256 이상. 글자는 숫자나 `4G`·`4096M`(`GB`·`MB`, 대소문자 무관). null·""·0은 제한 없음)는 메모리 상한(02 문서 §3.6), 그 밖이면 400 `invalid_format`(`fields:[{field:"memory_limit_mb"}]`) | `{success:true, synced, sync_error?, ignored?}`(`synced:false`면 저장은 됐지만 모듈 `configure`가 게임 설정 파일에 쓰지 못함, `ignored`는 받고 쓰지 않은 RCON 키) / 400 `unknown_setting {fields}`(모듈이 설정 목록을 선언했는데 공통 필드·모듈 설정·데몬 설정 `auto_restart`·`restart_on_unknown_exit`·`managed_start`·`log_sessions_keep` 어디에도 없는 키) / 400 `validation_failed`·`invalid_format`(모듈 `format`·`pattern`과 다름)`{details, fields}` / 404 / 500 |
| POST | `/api/instance/:id/launch-preview` | `instance.rs` `launch_preview` → `Supervisor::launch_preview` | `{settings?:{필드:값}, extra_args?: string\|string[]}` (extra_args가 없으면 저장된 것) | 실행 인자 미리보기. 서버를 띄우지 않고 익스텐션 hook도 부르지 않는다. 저장된 설정 위에 초안, 그 위에 추가 인자를 필드 `arg` 모양으로 읽은 값을 얹어 `get_launch_command`(`preview:true`)를 부른다. 필드에 `arg`를 선언하지 않은 모듈이면 모듈을 부르지 않고 `{success:true, has_launch_args:false, ...}`(command_line 없음)를 돌려준다. 성공 `{success:true, has_launch_args, command_line, program, args[], settings_from_args{}, extra_args[], extra_args_text(큰따옴표가 든 인자가 있으면 null: 배열로 저장), errors[{arg\|null, field, expected, got}]}`. password 필드 값과 인스턴스 `rest_password`는 `***`. 모듈 실패 `{success:false, error:"launch_command_failed"\|action_required, message?, ...같은 키}` / 404 / 500 `preview_failed`. `expected`는 `validator::expected_of` 코드(`memory_size`·`port`·`integer`·`path`·`pattern`·`number`·`boolean`·`enum`·`text`) |
| POST | `/api/instance/:id/rename` | `instance.rs:878` | `{name}` | `{success, id, name, previous}` / 409 `name_taken`·`server_running` / 404 |
| GET | `/api/provision-progress/:name` | `instance.rs:760` | - | `{active:false}` 또는 `{active:true, step,total,label,message,done,error,percent?,steps?}`. `error`는 모듈이 돌려준 `message`이거나, 스크립트가 비정상 종료·시간 초과면 `provision.script_failed` 문구다(스크립트 출력은 데몬 로그에만) |
| DELETE | `/api/provision-progress/:name` | `instance.rs:719` | - | `{success, rolled_back}`. **신규 생성**의 프로비저닝이 실패한 상태였을 때만 인스턴스를 삭제한다 |
| GET | `/api/instance/:id/check-update` | `instance.rs:1384` | - | 훅 결과 또는 `{update_available:false, reason}` |
| POST | `/api/instance/:id/apply-update` | `instance.rs:1481` | - | `{success,id,provisioning:true}`. 409 `server_running`·`instance_busy`(백업·복원·업데이트 중, 업데이트는 끝날 때까지 작업 잠금을 잡는다), 400 `no_install_config`/`update_unsupported`(설치 방식 익스텐션이 설치한 인스턴스가 아님) |

`POST /api/instances` 동작 세부:

- 모듈의 `install.requires_extensions`가 활성·설치돼 있지 않으면 422 `extension_required`다(`instance.rs:233-272`).
- 다른 인스턴스가 같은 `working_dir`를 쓰면 409다(`instance.rs:292-305`). 포트를 주지 않으면 다른 인스턴스와 겹치지 않게 기본 포트를 옮긴다(`:320-338`). 포트를 주면 모듈이 선언한 설정 `port`도 그 값으로 맞춘다(`:311-319`, 설치가 `module_settings`로 게임 설정 파일을 쓴다). 1~65535가 아닌 포트는 400 `invalid_format`(`fields:[{field:"port"}]`, `:306-310`)이다.
- `preset`을 주면 모듈 `[[presets]]`에서 그 id의 `settings`를 바탕으로 깔고, 따로 보낸 `module_settings`가 그 위를 덮는다. 없는 id는 400 `preset_not_found`다. 원격(디스코드·챗봇)도 `instance.create`의 `preset`으로 고를 수 있다(프리셋에는 비밀번호·파일·경로 설정이 없다).
- `module_settings{}`를 주면 모듈이 선언한 설정 이름만(모르는 이름은 무시), select는 고를 수 있는 값만 받아 기본값보다 먼저 넣는다. 선언된 설정에 규칙에 맞지 않는 값이 오면 400 `invalid_format` `fields:[{field}]`다(`instance.rs` "만들 때 고른 설정"). GUI·CLI는 `install = true`인 설정(예: 마인크래프트 서버 종류)을 만들 때 묻는다. 프로비저닝 `install_server`는 이 인스턴스 설정을 `settings{}`로 받는다(업데이트 경로도 같다).
- `migration_source`가 있으면 데몬 락을 놓은 뒤 `import_settings`를 **동기로** 호출한다(`instance.rs:648-686`). 결과의 `executable_path`(작업 폴더 기준, 폴더 안의 파일일 때만)를 실행 파일로 쓰고, `detected`는 응답에 그대로 붙인다(`{success, id, detected?}`).
- 인스턴스를 저장한 직후 `server.post_create` 훅을 백그라운드로 보내고 결과를 기다리지 않는다(`instance.rs:357-370`). ctx는 `{instance_id, instance_dir, instance, module, migration}`이다.
- 모듈에 `install` 설정이 있으면 `lifecycle.py install_server`를 백그라운드로 실행한다(타임아웃 30분).

`PATCH /api/instance/:id` 동작 세부:

- 알려진 키(`port, rest_*, executable_path, protocol_mode, server_version, extension_data`)는 일급 필드로 반영한다. 예전 RCON 키(`rcon_port, rcon_password, enable_rcon`)는 받아도 어디에도 반영하지 않는다(`instance.rs:979-1000`). `rest_*`·`executable_path`에 null을 보내면 지정을 지워 기본값(모듈 실행 파일 해석, REST 기본값·실행 때 만드는 비밀번호)으로 되돌린다.
- 나머지 키는 전부 `module_settings`로 들어간다.
- 이후 락을 놓고 `manage_properties("write")`로 게임 설정 파일과 동기화하고, `extension_data`가 비어 있지 않으면 `server.settings_changed` 훅을 보낸다.

#### 명령 실행 (handlers/command.rs)

| Method | Path | Handler | 요청 | 응답 |
|---|---|---|---|---|
| POST | `/api/instance/:id/command` | `command.rs:16` | `{command*, args?}` | 실행한 경로의 응답 + `command`. 실패 시 `error_code`, 모듈 `[errors]` 문구 `error_message`. 404 / 400 / 409 `server_not_running` |
| GET | `/api/instance/:id/commands` | `command.rs` `list_instance_commands` | - | `{commands:[{name,label,description,risk,inputs,source:"module"\|"extension",group?,...}], components?}`. `[commands] hook = true`인 모듈은 lifecycle `instance_commands()`가 준 인스턴스별 명령이 module.toml 명령 뒤에 `group`(예: 플러그인 이름)과 함께 붙고, `components`는 모듈이 판정한 구성요소를 그대로 넘긴다. 목록은 매번 새로 읽는다 |
| POST | `/api/instance/:id/rcon` | `command.rs:392` | `{command*}` | 이름만 남은 호환 경로. 언제나 stdin으로 보낸다: `{success, message, data:{response,executed}\|null, protocol:"stdin"}` / 404 / 409 `no_command_path`(콘솔 없음) |
| POST | `/api/instance/:id/rest` | `command.rs:487` | `{endpoint*, method?, body?, timeout?, rest_host?, rest_port?, use_https?}` | `{success, data, error, endpoint, method, host, port, protocol:"rest"}`(응답 없이 연결이 끊기면 `success:false`, `data.outcome:"unknown"`) / 400 / 500 |

#### Managed 프로세스와 모듈 기능 (handlers/managed.rs)

| Method | Path | Handler | 요청 | 응답 |
|---|---|---|---|---|
| POST | `/api/instance/:id/managed/start` | `managed.rs:24` | 선택 `{config?}` | supervisor 결과 / 404 / 409 `instance_busy` / 422 `memory_limit_reached` / 500 |
| GET | `/api/instance/:id/console` | `managed.rs:92` | query `since, count, spans` | supervisor 결과 (`lines[{id,content,level}]`, `spans=1`이면 줄마다 `spans`, 02 §3.3) / 404 |
| POST | `/api/instance/:id/stdin` | `managed.rs:118` | `{command*}` | `/rcon`과 같은 응답(`via_stdin`) / 400 / 409 `stdin_not_supported`(모듈의 `protocols.supported`에 `stdin`이 없음, 예: REST 전용) / 500 |
| POST | `/api/instance/:id/validate` | `managed.rs:139` | - | supervisor 결과 |
| POST | `/api/instance/:id/query/:name` | `managed.rs` `query_instance_handler` | `{args?: {입력: 값}}` | 모듈 `[[queries]]` 조회(X-2, 02 §`[[queries]]`): lifecycle `function`을 `validate`와 같은 config + `args`로 부른다. `{success, data}`(실패면 `success:false, error, error_code:"query_failed"`, Python이 실패하면 500, 디스코드에서 부르면 오류 원문 대신 `Query failed`). 404 `not_found`(인스턴스)·`unknown_query`, 400 `invalid_argument {field}`(선언에 없는 입력, 명령 입력과 같은 검사). 읽기 전용이라 감사에 남기지 않는다 |
| GET | `/api/instance/:id/properties` | `managed.rs:209` | - | `manage_properties("read")` |
| PUT | `/api/instance/:id/properties` | `managed.rs:224` | `{settings}` | `manage_properties("write")` |
| GET | `/api/instance/:id/settings/live` | `managed.rs:246` | - | `manage_properties("import")`. 성공하면 인스턴스 필드도 파일 값으로 맞춘다 |
| POST | `/api/instance/:id/properties/reset` | `managed.rs:274` | - | `"reset"` |
| POST | `/api/instance/:id/server/reset` | `managed.rs:289` | - | `"reset_server"` (월드, 설정 삭제) |
| POST | `/api/instance/:id/agreements` | `managed.rs` `accept_agreements_handler` | `{accept:[id]}` | `{success, agreements:[{id, label, url, accepted, accepted_at}]}`. 모듈 `[[agreements]]` 동의를 기록하고 lifecycle `accept_agreements`를 부른다(02 문서). 400 `invalid_format`(선언 안 된 id), 403 `agreement_local_only`(원격), 404 `not_found` |
| POST | `/api/instance/:id/diagnose` | `managed.rs:357` | - | supervisor 결과 |
| GET | `/api/instance/:id/installed-version` | `managed.rs:420` | - | 결과. 감지에 성공하면 `server_version`에 자동 저장 |

#### 백업과 예약 작업 (handlers/backup.rs)

| Method | Path | Handler | 요청 | 응답 |
|---|---|---|---|---|
| GET/POST | `/api/instance/:id/backups` | `backup.rs:203/210` | - | `{success, backups}` / `{success, backup}`. 400 `nothing_to_back_up`·`backup_unsupported`(모듈에 `[backup]` 없음), 500 `backup_failed`(`backup_paths` 실패) |
| DELETE | `/api/instance/:id/backups/:name` | `backup.rs:218` | - | `{success}` / 404 `backup_not_found` / 409 `backup_busy` |
| POST | `/api/instance/:id/backups/:name/restore` | `backup.rs:232` | - | `{success, restored, pre_restore_backup}`. 409 `server_running`·`backup_busy`. 복원할 백업은 안전 백업 뒤 정리에서 뺀다(`backup.rs:244-246`) |
| GET/PUT | `/api/instance/:id/schedule` | `backup.rs` `get_schedule`/`put_schedule` | `{schedule:[...], backup_keep}` | 400 `validation_failed {details}`. `backup_keep`은 1 이상이다(자동 백업 정리는 방금 만든 백업을 지우지 않는다). 원격(디스코드·챗봇) 출처는 이미 있는 작업과 같지 않은 작업에 대해 켜진 `update` 작업을 받지 않고(끄는 것은 받는다) `every_minutes`가 30 이상이어야 하며, 바꾸는 `backup_keep`은 30 이하여야 한다 |

인스턴스마다 작업 잠금 하나(`backup.rs` `BusyGuard`)를 백업·복원·백업 삭제·업데이트(`begin_server_update`, 작업이 끝날 때까지)·시작(`start_server_handler`·`start_managed_handler`, 시작을 마칠 때까지)·인스턴스 삭제가 함께 쓴다. 잡혀 있으면 백업 API는 409 `backup_busy`, 나머지는 409 `instance_busy`를 준다. 복원은 잠금을 잡은 뒤 실행 여부를 본다.

#### 설정 (handlers/bot.rs, handlers/config.rs)

| Method | Path | Handler | 요청 | 응답 |
|---|---|---|---|---|
| GET | `/api/config/bot` | `bot.rs:12` | - | BotConfig JSON (`config.json`의 `relay`) |
| PUT | `/api/config/bot` | `bot.rs:18` | 부분 JSON (얕은 머지) | `{success, message}` / 500 `{error}` |
| GET | `/api/config/gui` | `config.rs:15` | - | GuiSettings JSON |
| PUT | `/api/config/gui` | `config.rs:21` | 부분 JSON | `{ok:true}` / 400 / 500. 저장에 성공하면 `supervisor.skip_port_check`도 갱신 |
| GET | `/api/config/:kind/defaults` | `config.rs:47` | - | `{ok, defaults}` (`gui`, `bot`) / 404 |
| POST | `/api/config/:kind/reset` | `config.rs:59` | 선택 `{keys?}` | `{ok, config}` / 400 |
| GET/PUT | `/api/config/client/:kind` | `config.rs:106/117` | 부분 객체(최상위 키 머지) | 그 네임스페이스 JSON (예: `cli`). 예약 이름은 400 |
| GET | `/api/config/node-token` | `config.rs:134` | - | `{token}` (없으면 `""`) |
| PUT | `/api/config/node-token` | `config.rs:144` | `{token}` | `{ok}`. 저장 뒤 릴레이 에이전트를 새 토큰으로 다시 시작한다 |
| DELETE | `/api/config/node-token` | `config.rs:180` | - | `{ok}`. 먼저 릴레이 에이전트를 멈춘다 |
| POST | `/api/migration/export` | `migration.rs:184` | `{path(절대 경로), secrets?}` | `{success, path, secrets, instances:[{id,name}], config:[네임스페이스]}` / 400 `INVALID_PATH` / 500 `WRITE_FAILED`. A9: 데몬이 `path`에 zip(`saba-export.json`, `config.json`, `instances.json`, `secrets`면 `node_token`)을 쓴다. `updates`·`boot`·`extensions`·`schemaVersion`은 이 PC 것이라 뺀다. `secrets`가 아니면 챗봇 API 키, REST 비밀번호, 모듈이 `password`로 선언한 설정을 뺀다(모듈을 읽지 못한 인스턴스는 모듈 설정을 모두 뺀다). 게임 서버 파일은 넣지 않는다 |
| POST | `/api/migration/import` | `migration.rs:257` | `{path, dryRun?, overwrite?}` | `{success, dryRun, appVersion, secrets, instances:{added, replaced, skipped:[{id,name,reason}]}, missingModules, config:[네임스페이스], nodeToken:"imported"\|"kept"\|null}` / 400 `INVALID_PATH`·`INVALID_ARCHIVE`·`INVALID_CONFIG`. 인스턴스는 같은 id가 있으면 `exists`(`overwrite`면 덮어씀, 켜져 있으면 `running`), 다른 인스턴스와 폴더가 같으면 `working_dir_in_use`로 건너뛴다. 설정은 지금 값 위에 최상위 키 머지(파일에 없는 API 키는 남는다). 설정은 다른 것을 바꾸기 전에 먼저 검사한다(`check_sections`, 잘못되면 아무것도 바꾸지 않고 `INVALID_CONFIG`). 노드 토큰은 이 PC에 없을 때만 쓰고, 토큰을 실제로 가져올 때만 `relay.cloud`(relayUrl·hostId)·`cloudNodes`·`cloudMembers`도 바꾼다(아니면 이 PC 값 유지). `added`·`replaced` 항목에는 `working_dir`·`executable_path`가 붙는다 |
| GET | `/api/system/components` | `config.rs:241` | - | `{version, components:{key:ver}, lastUpdated}` (`installed-manifest.json` + 번들 `versions.toml`) |
| GET | `/api/system/metrics` | `system.rs:52` | `?sizes=false` | 시스템 모니터 표본 `{sampled_at_ms, window_ms, cpu:{cores,total_percent,others_percent}, memory:{total_bytes,used_bytes,others_bytes}, disk_io:{read_bps,write_bps,source:"physical"|"process",per_instance:false}, network:{rx_bps,tx_bps,per_instance:false}, disks:[{mount,total_bytes,available_bytes,others_bytes,instances:[{id,bytes}],sizes_at_ms}], instances:[{id,name,running,process_count,cpu_percent,memory_bytes,disk_read_bps,disk_write_bps}]}` (§10) |
| GET | `/api/runtime/preflight` | `ipc/mod.rs:766` | - | `{phase, percent, complete, error, components:{python}}` |

#### 클라이언트와 데몬 수명 (handlers/client.rs)

| Method | Path | Handler | 요청 | 응답 |
|---|---|---|---|---|
| POST | `/api/client/register` | `client.rs:21` | `{kind:"gui"\|"cli", pid?, installRoot?}` | `{client_id, kind, heartbeat_interval_ms:30000, timeout_ms:90000}` |
| GET | `/api/clients` | `client.rs` `client_list` | - | `{clients:[{id, kind, pid, installRoot}]}`. 인스톨러·제거기가 데몬을 끄기 전에 이 설치(`installRoot`)의 GUI PID를 받아, 임시 폴더에 풀린 포터블 GUI까지 끈다 |
| POST | `/api/clients/launch` | `client.rs` `client_launch` | `{kind:"cli", view:"console"\|"shell", instance?}` | I4 터미널로 열기: 데몬 옆의 `saba-chan-cli`를 새 콘솔 창(Windows `CREATE_NEW_CONSOLE`)에 띄운다. 실행 줄은 `console -- <instance-id>`(등록된 id만) 또는 인자 없음(셸) 둘뿐이다. 데몬의 표준 스트림을 넘기지 않고 `CreateProcessW`로 띄워 CLI가 새 창의 콘솔을 쓴다(`boot_selector::spawn_cli_in_new_console`). `{success, pid}`. 400 `invalid_input {field, reason}`, 404 `not_found`(인스턴스)·`cli_not_found`, 501 `unsupported_platform`(Windows 밖), 500 `launch_failed`. 로컬 전용이라 액션 카탈로그에 없다 |
| GET | `/api/permissions/:guildId` | `permissions.rs` `get_permissions` | - | `{guildId, ownerDiscordId, relayOwnerDiscordId, defaultLevel, preset, chatbotCap, members, commandLevels, presets}` |
| PATCH | `/api/permissions/:guildId` | `permissions.rs` `update_permissions` | `{defaultLevel?, preset?, chatbotCap?, ownerDiscordId?, commandLevels?:{key: level\|null}}` | 바뀐 전체(GET과 같은 모양). 하나라도 틀리면 저장하지 않고 400 `invalid_level`·`invalid_preset`·`invalid_command`·`invalid_member`·`unknown_key`. `ownerDiscordId`는 고정된 주인을 바꾸는 유일한 길이다(CLI `saba perm owner`) |
| PUT·DELETE | `/api/permissions/:guildId/members/:discordId` | `permissions.rs` `set_member`·`remove_member` | PUT `{level: -1..2}` | 바뀐 전체. 주인이면 400 `owner_fixed`. DELETE는 기본 레벨로 되돌린다 |
| GET | `/api/sandbox/unisolated` | `sandbox.rs` `list_unisolated` | - | `{components:[{component, fingerprint, current}]}`. `current`가 false면 파일이 바뀌어 동의가 쓰이지 않는다. 릴레이로는 열지 않는다 |
| POST·DELETE | `/api/sandbox/unisolated/:component` | `sandbox.rs` `allow_unisolated`·`revoke_unisolated` | - | `{success, component, fingerprint}`(DELETE는 `fingerprint:null`). `component`는 `module-<이름>`·`ext-<이름>`. 없으면 404 `unknown_component`. 원격(디스코드) 출처면 403 `consent_local_only`. 확인되지 않은 구성요소를 격리 없이 돌리기 동의(02 문서 §2.1) |
| GET | `/api/sandbox/extension-gui` | `sandbox.rs` `list_extension_gui` | - | unisolated와 같은 모양. 확인되지 않은 익스텐션의 GUI 번들 싣기 동의 목록 |
| POST·DELETE | `/api/sandbox/extension-gui/:component` | `sandbox.rs` `allow_extension_gui`·`revoke_extension_gui` | - | unisolated와 같은 응답·오류. `component`는 `ext-<id>`. 동의하면 그 익스텐션의 `/gui`·`/gui/styles`를 지금 파일 그대로일 때 서빙한다(03 문서 §4.1) |
| GET | `/api/actions` | `actions.rs` `list_actions` | query `surface?`(gui·cli·discord·chatbot), `instance?`, `locale?` | `{actions:[{id, kind, risk, surfaces, scope, input, titleKey, title, description?, route, cli}], locale}`. `cli`는 같은 일을 하는 CLI 한 줄(I4-c "CLI로 보기"): `saba-chan-cli do <id>`에 인스턴스 액션이면 ` --server <server>`, 필수 필드(모듈 명령·조회는 필수 입력)마다 ` <이름>=<이름>`. 꺾쇠 자리는 화면이 채운다. `instance`를 주면 그 서버 명령이 `server.command:<이름>`으로, 모듈 `[[queries]]`가 `server.query:<이름>`(`kind=query`, risk `safe`, 모든 표면, 제목·설명은 모듈 locales `queries.<이름>.*` → module.toml 값 → 이름, route `POST /api/instance/:id/query/:name`)으로 펼쳐진다. 조회 입력은 선언에 없는 키·범위·형식을 `invalid_input`으로 거부하고, 릴레이 `query`도 받는다. 챗봇으로 나가는 조회 결과는 콘솔과 같이 IP·비밀처럼 보이는 값을 가린다. 선언은 `src/actions/mod.rs` `ACTIONS` 하나(P2) |
| POST | `/api/actions/:id/plan` | `actions.rs` `plan_action` | `{input, instance?, locale?}` | `{success, planId, actionId, instance, summary:[{labelKey, label, value}], risk, needsConfirm, expiresAt}`. 부작용 없음. `needsConfirm`은 `danger`이거나, 챗봇 경로(`via_chatbot`)의 관리(admin) 액션일 때 true. 사람이 직접 친 caution 관리 명령은 false라 바로 `run`한다. 400 `invalid_input {field, reason}`, 403 `action_not_allowed`·`PERMISSION_DENIED {args}`·`NODE_NOT_ALLOWED {args:{instance}}`(이 길드의 `allowedInstances`에 없는 인스턴스), 404 `unknown_action` |
| POST | `/api/actions/:id/run` | `actions.rs` `run_action` | `{input, instance?}` 또는 `{planId}` | `{success, actionId, status, result}`(`result`는 밑 라우트 응답 그대로). 404 `plan_expired`, 403 `plan_not_yours`. 데몬 라우터를 안에서 부르므로 감사 기록은 원래 요청의 출처로 남는다. 여러 단계 계획은 `:id`가 `plan`(또는 빈 값). 원격 요청의 `instance.settings.set`이 성공하면 바꾸기 전 값으로 되돌리는 계획을 하나 더 만들어 `undoPlanId`·`undoExpiresAt`을 싣는다(같은 요청자, 10분, 전에 값이 없던 키는 빠진다). 원격 관리(admin) 실행은 길드당 최근 한 시간 `nodeSettings[guild].adminRunsPerHour`(기본 10)회까지이고 넘으면 429 `rate_limited {args:{perHour, retryInSecs}}`(여러 단계 계획은 관리 단계가 있을 때만 한 번). 확인을 기다리는 계획은 500개까지(넘으면 오래된 것부터 버린다). `instance.settings.set` 계획 요약의 `value`·`before`는 비밀값 키(어느 깊이든)를 `"***"`로 가리고, 묶음 값의 `before`는 바꾸는 칸만 싣는다. 원격 `instance.settings.set`은 묶음 값(객체·배열)을 `invalid_input`으로 거부한다. PATCH 액션의 선택 필드는 `null`(기본값으로 되돌리기)을 받는다. 원격에서 만든 서버(`instance.create`, 한 액션이든 여러 단계든)는 그 길드에 `allowedInstances` 목록이 있으면 목록에 더한다 |
| POST | `/api/actions/plan` | `actions.rs` `plan_steps` | `{steps:[{id, instance?, input}], locale?}` | 여러 단계 계획(I1, `src/actions/steps.rs`): 1~8단계. `instance`가 `@N`이면 N번째 단계(앞에 있는 `instance.create`)가 만들 서버이고, 그런 단계는 고정 액션 중 인스턴스를 id로 받는 것만 된다(`server.status`·모듈 명령·조회 안 됨). 단계마다 검증·판정하고 틀리면 그 오류에 `step`(1부터)과 `field`(`steps[n].<키>`)를 붙인다. 원격 검사는 새 서버의 모듈(`instance.create`의 `module_name`)로 한다. 응답은 한 액션 계획과 같고 `actionId:"plan"`, `steps:[{n, actionId, instance}]`, `summary`는 단계마다 `actions.field.step` 줄(`"N. 제목"`) 뒤에 그 단계 요약. 2단계 이상이면 늘 `needsConfirm`. 실행(`/api/actions/plan/run {planId}`)은 확인한 사람으로 단계를 다시 판정하고 순서대로 부르며 실패한 단계에서 멈춘다: `{success, actionId:"plan", runId, steps:[{n, actionId, instance?, status:"done"\|"failed"\|"skipped", code?, error?, undoPlanId?}], stoppedAt?}`. 원격이면 `error` 원문은 싣지 않는다. 새 서버를 쓰는 단계 앞에서는 그 서버 설치(프로비저닝)가 끝나기를 기다리고(최대 30분, 실패면 `provision_failed`), 실행 때 실제 id로 다시 풀고 다시 판정한다. 원격에서 만든 서버는 그 길드에 `allowedInstances` 목록이 있으면 목록에 더한다. 40초 안에 끝나지 않으면 `{success:true, running:true, runId, steps}`를 먼저 돌려주고 뒤에서 이어 가며, 끝나면 이벤트 `plan.done {runId, guildId?, ok, steps, stoppedAt}`. 같은 길드(로컬은 같은 출처)에서는 한 번에 하나(409 `plan_running`) |
| GET | `/api/actions/runs/:runId` | `actions.rs` `run_status` | - | 여러 단계 실행의 진행. 끝났으면 `{done:true, result}`, 아니면 `{done:false, steps}`. 시작한 쪽만(403 `plan_not_yours`), 한 시간 둔다(404 `run_not_found`) |
| POST | `/api/client/:id/heartbeat` | `client.rs:70` | 본문은 무시 | `{ok}` / 404 |
| DELETE | `/api/client/:id/unregister` | `client.rs:88` | query `shutdown=true\|false` | `{ok}` / 404 |
| POST | `/api/daemon/shutdown` | `client.rs:12` | - | `{ok, message}`. 200ms 뒤 `stop_requested` cancel |
| GET | `/api/daemon/console` | `client.rs:139` | `since, count=200` | `{lines:[{id,timestamp,level,target,message}], count}` |

`unregister`에 `shutdown=true`를 주고 남은 클라이언트가 0이면 데몬도 종료한다(`client.rs:99-103`). 둘 다 `request_shutdown`(`ipc/mod.rs:630-636`)을 거쳐 §1.1의 종료 경로로 간다.

#### 익스텐션 (handlers/extension.rs; 구현은 `extension/mod.rs`)

| Method | Path | Handler | 비고 |
|---|---|---|---|
| GET | `/api/extensions` | `extension.rs:68` | `{extensions:[...]}`. 설명은 `?lang=`(없으면 GUI 언어) |
| GET | `/api/extensions/init-status` | `extension.rs:513` | `{initializing, in_progress:{id:msg}, completed:[{ext_id,success,message,timestamp}]}` |
| POST | `/api/extensions/rescan` | `extension.rs:182` | `{success, newly_found}` |
| GET | `/api/extensions/manifest` | `extension.rs:361` | `{success, extensions, updates}` (원격) |
| GET | `/api/extensions/updates` | `extension.rs:468` | `{success, updates, count}` |
| POST | `/api/extensions/:id/enable` | `extension.rs:97` | `{success,id,startup_dispatched,trust,warnings[]}`. `warnings`는 `verified`가 아닐 때 `{code:"extension_unverified"\|"extension_modified", extension}`(막지 않는다). `daemon.startup` 훅을 비동기 실행 |
| POST | `/api/extensions/:id/disable` | `extension.rs:162` | 실행 중인 managed 인스턴스 참조 검사 |
| POST | `/api/extensions/:id/mount` | `extension.rs:199` | |
| POST | `/api/extensions/:id/unmount` | `extension.rs:211` | |
| POST | `/api/extensions/:id/install` | `extension.rs:425` | 선택 `{download_url, sha256}`. 비어 있으면(`{}`) manifest의 URL과 sha256을 쓴다. `download_url`만 주면 manifest의 URL과 같을 때만 manifest의 sha256으로 받고, 다르면 `sha256_required`로 거부한다(`extension.rs:396-422`). 빠진 의존성도 함께 설치, `{success, id, installed}`. manifest 버전이 설치된 버전보다 낮으면 `older_than_installed` |
| DELETE | `/api/extensions/:id` | `extension.rs:490` | |
| GET/PUT | `/api/extensions/:id/config` | `extension.rs:574/587` | `{config}` / 객체 머지(null이면 키 삭제) |
| GET | `/api/extensions/:id/gui` | `extension.rs` `serve_gui_bundle` | UMD JS 번들 (application/javascript). 확인된(`verified`) 익스텐션이거나 사용자가 이 PC에서 허락한 것만(`sandbox.extension_gui`), 아니면 403 `{error_code:"extension_gui_unapproved", component:"ext-<id>"}` |
| GET | `/api/extensions/:id/gui/styles` | `extension.rs` `serve_gui_styles` | text/css. 번들과 같은 조건 |
| GET | `/api/extensions/:id/icon` | `extension.rs:520` | icon.png |
| GET | `/api/extensions/:id/i18n/:locale` | `extension.rs:337` | JSON |

- 익스텐션 에러 형식: `{success:false, error, error_code, related}`.
- `error_code`별 상태 코드: `not_found|not_mounted|manifest_not_found`는 404, 의존성·버전 계열은 422, `has_dependents|in_use`는 409, `id_mismatch`는 400(`extension.rs:26-59`).
- `/gui`, `/gui/styles`, `/icon` 라우트가 바로 **플러그인이 인터페이스에 컴포넌트를 주입하는 통로**다. GUI가 이 번들을 받아 렌더한다.

#### Relay 프록시, 에이전트, 채팅

| Method | Path | Handler | 비고 |
|---|---|---|---|
| GET | `/api/relay/host/:host_id/status` | `relay.rs:235` | query `relayUrl?`. relay `/api/hosts/:id` 프록시 (HMAC 서명). 서명 요청의 `relayUrl`은 설정한 릴레이와 같거나 루프백일 때만 따른다(`signed_relay_url`, `relay.rs:46-67`) |
| GET | `/api/relay/host/:host_id/nodes` | `relay.rs:247` | `/api/hosts/:id/nodes` |
| GET | `/api/relay/node/:guild_id/chatbot/models` | `relay.rs` `get_node_chatbot_models` | `/api/nodes/:id/chatbot/models` 서명 프록시. 릴레이 답을 그대로 전달한다: 200 `{provider, models, selected, effectiveModel}`, 409 `NO_API_KEY`, 422 `INVALID_API_KEY`·`QUOTA_EXCEEDED`, 503 `PROVIDER_UNAVAILABLE` |
| GET/PUT | `/api/relay/node/:guild_id/channels` | `relay.rs:261/282` | `/api/nodes/:id/channels` 서명 프록시. 챗봇 채널(여럿)·음악 채널(하나) 배정의 원본은 릴레이다. 응답 `{guildId, chatbot:[id], music:id\|null, botAvailable, channels:[{id,name,category}]}`, PUT 본문 `{chatbot?, music?}`(준 키만). 오류 본문 `{error: CODE, channelId?}`는 그대로 전달(객체가 아닌 본문은 데몬이 400 `INVALID_BODY`). 데몬 config의 `chatbot.guilds.*.channelIds`는 `ChatbotConfig::migrate`가 지운다 |
| GET | `/api/relay/node/:guild_id/members` | `relay.rs` `list_node_members` | `/api/nodes/:id/discord-members`(15초 제한). 실패하면 `/members`(권한을 등록한 멤버만)로 폴백한다. query `withStatus=1`이면 `{members, partial, reason?}`로 답한다: 폴백이면 `partial:true`, `reason`은 `bot_unavailable`·`relay_error`·`relay_unreachable`. 없으면 예전처럼 배열만 |
| GET | `/api/relay/node/:guild_id/legacy-members` | `relay.rs` `list_node_legacy_members` | `/api/nodes/:id/members` 서명 프록시. 릴레이 본문(`[{id, username, displayName, level}]`)을 그대로, 실패하면 오류 상태와 본문(빈 목록으로 바꾸지 않는다) |
| GET | `/api/relay/bot-invite` | `relay.rs:430-449` | query `relayUrl?`. relay `/api/bot-invite`(공개, 서명 없음)를 그대로 전달. 주소는 서명 요청과 같은 규칙(설정한 릴레이 또는 루프백)으로 정한다. 봇 초대 URL `{url}` |
| POST | `/api/relay/pair/initiate` | `relay.rs:451` | 바디를 그대로 `/api/pair/initiate`에 전달 (서명 없음) |
| GET | `/api/relay/pair/:code/status` | `relay.rs:485` | `secret?, relayUrl?`. `status:"claimed"`이면 `nodeToken`을 `.node_token`에 자동 저장 |
| GET | `/api/relay/link-requests` | `relay.rs` `list_link_requests` | relay `GET /api/link-requests` 서명 프록시. 디스코드에서 코드 없이 `/사바쨩 연결`한 요청 중 이 PC에 온 것(J1-b) |
| POST | `/api/relay/link-requests/:request_id` | `relay.rs` `answer_link_request` | `{accept:bool}` → relay `POST /api/link-requests/:id` 서명 프록시. 수락하면 relay가 그 길드를 이 PC에 묶는다. 응답은 relay 것 그대로 |
| GET | `/api/relay/agent` | `chat.rs:19` | 에이전트 상태 `{running, relayUrl, hasToken, agentVersion, lastHeartbeatAt, lastError, lastErrorCode?, commandsHandled}`. `lastErrorCode` `token_replaced`: relay가 401 `TOKEN_REPLACED`(같은 디스코드 계정이 다른 PC를 페어링)로 답해서 에이전트가 멈췄다. 이 PC에서 다시 페어링하면 새 토큰 저장이 에이전트를 다시 띄운다 |
| GET·PATCH | `/api/discord/:guild_id/settings` | `discord.rs` `get_settings`·`patch_settings` | 디스코드 길드 설정(I2) `{guildId, locale, sleeping, music}`. PATCH는 바꿀 키만(`music` 안도 부분) → `{success, settings, sync:{guildId, ok, status?, error?}}`. 틀린 값 400 `invalid_input {field, reason}`, 페어링이 없으면 409 `not_paired`. 저장 전에 릴레이 `PATCH /api/nodes/:g/settings`로 보내고, 릴레이 값과 다른 키만 보낸다(같으면 보내지 않는다). 자동 언어로 바꿀 때는 지금 GUI 언어 `language`도 싣는다. 릴레이가 400이면 저장하지 않고, 닿지 않으면 저장한 뒤 `sync.ok:false`. 에이전트가 연결되면 길드마다 같은 방식으로 맞추고, 블록이 없는 길드는 릴레이 값을 한 번 가져와 원본으로 삼는다(`discord::sync_on_connect`). 그 전에 온 GET·PATCH도 블록이 없으면 릴레이 값에서 출발한다(못 읽으면 기본값). 읽고-바꾸고-저장하기는 한 번에 하나씩 한다. 액션 `discord.settings.get`(safe)·`discord.settings.set`(admin, 디스코드는 소유자, 챗봇 없음). 원격 요청은 자기 길드만 다룬다(`guild`를 그 길드로 고정, 다르면 `invalid_input`) |
| POST | `/api/relay/agent/start`, `/stop` | `chat.rs:24/36` | 토큰이 없으면 409 `node_token_missing` |
| POST | `/api/chat/command` | `chat.rs:10` | `{text, guild_id?, user_id?, locale?}` → `{reply, code, args}`. 답 문구는 `locale`, 없으면 GUI 언어. `code`는 입력을 알아보지 못했을 때만(`UNKNOWN_COMMAND`·`INSTANCE_NOT_FOUND`), 아니면 null |
| GET/PUT/DELETE | `/api/chatbot` | `chatbot.rs:78/265/287` | AI 챗봇 설정 (`config.json`의 `chatbot`). 바뀔 때마다 릴레이에 동기화, 클라이언트에는 `hasApiKey`·`pendingRelayDelete`만. PUT은 릴레이 기본값 위에 전체 설정을 얹어 보낸다(`chatbot.rs:31-65`). DELETE가 릴레이에 닿지 못하면 `pendingRelayDelete:true`로 남기고 다음 연결 때 다시 지운다(`:219-263`) |
| DELETE | `/api/chatbot/memory` | `chatbot.rs:333` | query `guildId?` |
| GET | `/api/guilds/:guild/instances` | `guild_words.rs` `list` | - | 그 길드가 쓰는 서버(`allowedInstances`, 없으면 전부)마다 `{id, name, module, words, commands:[{name, label, risk, builtin, moduleAliases, aliases}]}`. `{guildId, instances}`. 명령은 이름마다 한 줄이고, 모듈이 내장(start·stop·status)과 같은 이름을 정의하면 이름표·위험도는 모듈 선언을 쓴다(`builtin`은 true) |
| PUT | `/api/guilds/:guild/instances/:id/words` | `guild_words.rs` `put` | `{words, commands:{<명령>:[..]}}` | `{success, id, words, commands}`. 규칙 위반은 400 `validation_failed {details:[{field, reason, word?, conflictWith?}]}`(`field`: `words[i]`·`commands.<명령>[i]`·`words`·`commands.<명령>`·`input`, `reason`: `invalid`·`reserved`·`conflict`(같은 서버 안에서 겹치면 `conflictWith`가 그 서버 이름)·`unknown_command`·`too_many`, `word`는 문제가 된 낱말), 그 길드가 쓰지 않는 서버는 404. 빈 값이면 항목을 지운다. 로컬 전용(릴레이 에이전트는 부르지 않고 액션 카탈로그에도 없다) |
| PUT | `/api/guilds/:guild/allowed-instances` | `guild_words.rs` `put_allowed` | `{instances: [id] \| null}` | `{guildId, allowedInstances: [id] \| null}`. 그 길드가 보고 부를 수 있는 서버(`nodeSettings[guild].allowedInstances`). null은 제한 없음(키 삭제), 빈 배열은 아무 서버도 못 쓴다. 겹친 id는 하나로 줄인다. 없는 인스턴스 id는 400 `validation_failed {details:[{field:"instances", reason:"unknown_instance", id}]}`, 모양이 틀리면 `reason:"invalid"`. 설정 잠금 안에서 저장한다(`update_node_settings`). 로컬 전용(액션 카탈로그에 없다) |
| GET·PATCH | `/api/chatbot/guilds/:guild` | `chatbot.rs` `get_guild`·`patch_guild` | PATCH `{설정 키: 값\|null}` | 길드 하나의 챗봇 설정(I2-b): `{success, guildId, settings, hasApiKey}`(`settings`는 릴레이 기본값 위에 실제 값). PATCH는 그 길드 값만 합쳐 `PUT /api/chatbot`과 같이 저장·동기화한다. 릴레이 기본값 표에 없는 키(`apiKey` 포함)는 400 `validation_failed`. 액션 `chatbot.settings.get`(safe, 디스코드는 소유자)·`chatbot.settings.set`(admin, 디스코드는 소유자, 챗봇 표면 없음: 챗봇이 자기 확인을 끄지 못하게). 원격은 자기 길드만 |


Relay 서명 방식(`relay.rs:115-161`):

- 헤더: `Authorization: Bearer <node_token>`, `x-request-timestamp`, `x-request-nonce`, `x-request-signature`.
- 서명값: `HMAC-SHA256(secret, "METHOD\npath\nts\nnonce\nbody")`.
- 고정 헤더 `X-Saba-Client: 1`이 모든 relay 요청에 붙는다. timeout은 5초다(`relay.rs:69-81`).

#### 업데이트 (ipc/updates.rs)

| Method | Path | Handler | 요청 | 응답 |
|---|---|---|---|---|
| GET | `/api/updates/status` | `updates.rs:129` | - | `{ok,last_check,next_check,checking,error,updates_available,components:[{component,display_name,current_version,latest_version,update_available,downloaded,installed}]}` (Locales 제외) |
| POST | `/api/updates/check` | `updates.rs:169` | - | `{ok,updates_available,update_names,components(+release_notes),last_check}`. Locales 업데이트가 있으면 백그라운드에서 조용히 적용 |
| POST | `/api/updates/download` | `updates.rs:366` | 선택 `{components?:[key]}`. 본문이 없거나 공백이면 전부, 깨진 JSON은 400 `invalid_body`(`updates.rs:348-357`) | `{ok,downloaded,count,errors?}` |
| GET | `/api/updates/download/progress` | `updates.rs:450` | - | `{ok,component,bytes_received,total_bytes,active}` |
| POST | `/api/updates/apply` | `updates.rs:475` | download와 같은 본문 규칙 | `{ok,applied,needs_updater,requires_updater,errors}` |
| POST | `/api/updates/launch-updater` | `updates.rs:609` | 선택 `{relaunch:[exe, args...]}` | `{ok, updater_launched, daemon_stopping}`. 404(업데이터 없음) / 409(받은 업데이트 없음). 등록된 GUI(`/api/client/register`의 `pid`, 실행 파일 이름이 GUI일 때만)가 떠 있으면 다시 띄울 것을 그 GUI로 바꾸고, 종료 전에 3초 기다려 남은 GUI를 끈다 |
| GET/DELETE | `/api/updates/last-result` | `updates.rs` `get_last_result`/`clear_last_result` | GET `?lang=` | 마지막 적용 결과 `{result, version, notes}` / 확인 처리 |
| GET | `/api/updates/integrity` | `updates.rs:814` | - | `{ok,checked_at,overall,total,verified,failed,skipped,components[...]}` |
| GET | `/api/updates/config` | `updates.rs:844` | - | `{ok, config, mock_relay}` |
| PUT | `/api/updates/config` | `updates.rs:859` | 부분 `{enabled,check_interval_hours(1-8),auto_download,install_root,relay_url,channel}` | `{ok, config}`. `config.json`의 `updates` 절에 저장한다(`:899-902, 936-942`). 비어 있지 않은 `relay_url`은 개발 빌드(`mock-relay`)만 받고, 릴리즈 빌드는 403 `relay_url_not_settable`(D7: 출처는 config.json에서만) |

---

## 3. 인증 모델 (`src/ipc/auth.rs`)

- **토큰 생성**: 데몬이 기동할 때마다 `uuid::Uuid::new_v4().to_string()`(36자)를 새로 만든다(`auth.rs:28`).
- **저장**
  - `constants::token_file_path()`, 기본값은 `<data_dir>/.ipc_token`이고 `SABA_TOKEN_PATH`로 바꿀 수 있다.
  - Unix에서는 퍼미션을 0600으로 둔다.
  - 프로세스 전역 `static CACHED_TOKEN: RwLock<Option<String>>`에도 캐시한다(`auth.rs:17`).
- **Host 검사**: 토큰 검사보다 먼저, `Host` 헤더가 루프백이 아니면 403을 돌려준다. 헤더가 없으면 통과시킨다(`auth.rs:90-117`).
- **검증**: 헤더 `X-Saba-Token` 값과 정확히 일치하는지 비교한다(`auth.rs:142`).
  - 상수 시간 비교다(`constant_time_eq`, `auth.rs:102-105`). 길이 차이는 숨기지 않는다(UUID 고정 길이).
  - 불일치하면 본문 없이 401을 돌려준다.
  - 실패 로그는 30초에 한 번으로 제한하고, 토큰은 남기지 않는다(헤더가 없었는지 틀렸는지만).
- **우회 경로**
  - `SABA_AUTH_DISABLED=1`이면 토큰 검사를 건너뛴다. **디버그 빌드에서만** 동작하며(`auth.rs:119-122`), 통합 테스트가 이 방식을 쓴다(`tests/daemon_integration.rs:111`).
  - 캐시도 없고 토큰 파일도 없으면 **거부한다**(fail-closed).
  - `/health`는 루프백 검사 뒤 토큰 검사 없이 통과한다(A6).
- **토큰을 쓰는 클라이언트**: GUI(Tauri 셸 `saba-chan-gui/src-tauri/src/daemon.rs:154-181`, 401이면 파일을 다시 읽고 한 번 재시도 `:228-244`), CLI(`saba-chan-cli/src/client.rs:28-33`, 401이면 파일을 다시 읽고 한 번 재시도 `:97-106`. PR #6 기준), Python 모듈(`modules/daemon_rcon.py:48-54`, `SABA_TOKEN_PATH`). 모두 토큰 파일을 직접 읽어 헤더에 넣는다.
- **CORS 레이어가 없다.** 브라우저에서 오는 교차 출처 요청은 사전 요청(preflight)에서 막히고, 단순 요청도 토큰이 없으면 401이다. 모든 라우트가 인증을 거치므로 바디 없는 POST를 CSRF로 부를 수 있던 예전 `/api/updates/*` 경로도 막혔다.

---

## 4. 인스턴스 모델

### 4.1 `ServerInstance` (`src/instance/mod.rs:24-76`)

| 필드 | 타입 | 비고 |
|---|---|---|
| `id` | String | uuid v4 |
| `name` | String | 생성 시 유일성 검사는 없고, 이름 변경(`/rename`)만 중복을 막는다. `/api/server/:name/*`는 첫 번째로 일치하는 인스턴스를 쓴다 |
| `module_name` | String | |
| `executable_path`, `working_dir` | Option<String> | |
| `auto_detect` | bool | 기본 true |
| `process_name` | Option<String> | 모듈 metadata에서 채운다 |
| `port` | Option<u16> | |
| `rest_host`, `rest_port`, `rest_username`, `rest_password` | Option | `rest_username`은 항상 "admin"으로 강제한다(`instance/mod.rs:218-223`) |
| `protocol_mode` | String | 기본 `"auto"` |
| `module_settings` | HashMap<String,Value> | settings.json으로 분리 저장 |
| `server_version` | Option<String> | |
| `extension_data` | HashMap<String,Value> | 설치 방식 익스텐션 키(`install_method_<방식>`, `<방식>_<이름>`, `instance/mod.rs:142-166`) 등 |
| `required_extensions` | Vec<String> | 모듈의 `install.requires_extensions`를 복사한다 |
| `schedule` | Vec<ScheduledTask> | 예약 작업 (백업·재시작·공지·업데이트) |
| `backup_keep` | Option<u32> | 자동 백업 보관 개수 (기본 10) |

- RCON 필드(`rcon_port`, `rcon_password`)는 없다. 예전 `instance.json`·`settings.json`에 남은 RCON 키는 로드할 때 한 번 지우고 다시 쓴다(`instance/mod.rs:439-468, 483-500`).
- `ensure_passwords()`(`instance/mod.rs:210`)는 시작할 때(`server.rs:903`, `managed.rs:33`) 비어 있는 REST 비밀번호를 16자 영숫자로 채운다.
- 비밀번호 생성은 `constants::generate_random_password`가 uuid v4 바이트에 `% 62`를 적용하는 방식이다(`constants.rs:188-201`). 모듈로 편향이 있고, 두 번째 uuid는 계산만 하고 쓰지 않는다.
- `GuiSettings.autoGeneratePasswords`는 이 동작에 **반영되지 않는다.**

### 4.2 영속화 (`InstanceStore`, `instance/mod.rs:238-477`)

- 루트는 `constants::resolve_instances_dir()`이며, 기본값은 `<data_dir>/instances`다.
- 저장 형식(JSON, pretty):
  - `<root>/order.json`: id 배열
  - `<root>/<id>/instance.json`: `module_settings`를 뺀 메타데이터
  - `<root>/<id>/settings.json`: `module_settings`
  - Unix에서는 두 파일 모두 0600으로 둔다.
- `instance.json`·`settings.json`은 임시 파일에 쓰고 rename하는 원자적 쓰기다(`write_atomic`, `instance/mod.rs:502-513`). `order.json`은 그냥 덮어쓴다(`:472-476`).
- 쓰거나 지울 폴더는 id가 경로 한 칸이고 심볼릭 링크가 아니어야 한다(`checked_dir`, `:399-409`).
- 로드할 때 `order.json` 순서로 정렬하고, 목록에 없는 인스턴스는 뒤에 붙인다.
- `remove()`는 `<root>/<id>` 디렉토리를 통째로 지운다(`:343-354`). 기본 설치 위치인 `server/` 하위 게임 서버 파일과 `backups/`도 **함께 지워진다.**

### 4.3 생명주기

1. **생성**: `POST /api/instances`. 두 갈래로 나뉜다.
   - 네이티브 설치: `download` 방식은 `lifecycle.py list_versions`, 이어서 `install_server`. 그 밖의 방식은 같은 이름의 설치 방식 익스텐션이 `extension_data`를 보고 맡는다.
   - 마이그레이션: `import_settings`
2. **프로비저닝 추적**: `ProvisionTracker`가 **인스턴스 이름**을 키로 쓴다(`ipc/mod.rs:379-432`).
   - 성공하면 5초 뒤 자동으로 지운다.
   - 실패하면 사용자가 dismiss할 때까지 남겨 두고, dismiss하면 **신규 생성이었던 경우만**(`mark_creating`/`take_creating`) 인스턴스를 롤백(삭제)한다.
3. **시작**: 클라이언트는 `/api/instance/:id/start`를 쓴다. 데몬이 `uses_managed_start`(`supervisor/mod.rs:1690-1699`, 기본 managed)로 경로를 고르고, 모듈 validate의 critical 문제가 있으면 막는다.
   - `/api/server/:name/start`: native/supervisor `start_server`
   - `/api/instance/:id/managed/start`: stdio를 캡처하는 managed 방식
4. **모니터링**: 2초 루프(`main.rs:416`)가 PID와 패턴을 매칭해 auto-detect하고, 크래시를 감지하면 자동 재시작을 예약한다. 세부는 supervisor 담당 문서에 있다.
5. **정지**: `/api/instance/:id/stop` 또는 `/api/server/:name/stop`(`stop_instance`, `server.rs:958`). 성공하면 tracker에서 id로 untrack한다(`server.rs:999`).
6. **삭제**: 실행 중이면 force kill(2초 대기), `server.pre_delete` 훅, 디렉토리 삭제(최대 3회 재시도) 순으로 진행한다(`instance.rs:789-875`).

### 4.4 config_store와 전역 설정 스키마 (`src/config_store/mod.rs`)

- 모든 사용자 설정은 `<data_dir>/config.json` 하나에 네임스페이스별로 들어간다(`updater/src/config_file.rs:1-27`). 저장은 임시 파일 → fsync → 직전 파일을 `config.json.bak`으로 복사 → rename이고, 깨진 파일은 `.bak`에서 복구한다. 예전 개별 파일(`settings.json`, `bot-config.json`, `cli-settings.json`, `update-config.json`, `boot-config.json`, `extensions_state.json`, `extensionConfig.json`)은 해당 키가 없을 때 한 번 이관하고, 원본은 그대로 두며 `.migrated` 사본을 남긴다(`config_file.rs:37-45, 155-182`).
- `ConfigStore`는 그중 `gui`와 `relay`(봇 설정) 네임스페이스를 메모리에 캐시하고 쓸 때 `config.json`에 flush한다(`config_store/mod.rs:232-436`). 기동할 때 한 번 로드한다(`ipc/mod.rs:600-602`). `chatbot`은 캐시 없이 읽고 쓰며, 그 밖의 클라이언트 네임스페이스(`cli` 등)는 JSON 그대로 최상위 키 머지한다(`:576-611`).
- `sandbox.unisolated{"module-<이름>"·"ext-<이름>": 파일 지문}`: 확인되지 않은 구성요소를 격리 없이 돌리기 동의. `POST·DELETE /api/sandbox/unisolated/:component`만 바꾼다(02 문서 §2.1).
- `sandbox.extension_gui{"ext-<id>": 파일 지문}`: 확인되지 않은 익스텐션의 GUI 번들 싣기 동의. `POST·DELETE /api/sandbox/extension-gui/:component`만 바꾼다(03 문서 §4.1).
- PUT 동작: 현재 값을 JSON으로 바꾼 뒤 요청값을 **얕게** 머지하고(`merge_json`, `:642`), 강타입으로 역직렬화해 저장한다.
- 스키마에 없는 키는 **저장할 때 버려진다.** 예를 들어 레거시 `discordToken`이나 GUI가 임의로 넣은 키가 그렇다.

**GuiSettings** (camelCase, `config_store/mod.rs:26-58`)

| 그룹 | 필드 (기본값) |
|---|---|
| 동작 | `autoRefresh`(true), `refreshInterval`(2000), `ipcPort`(57474), `consoleBufferSize`(2000), `autoGeneratePasswords`(true), `portConflictCheck`(true) |
| 테마 | `theme?`, `accentColor`("#667eea"), `accentSecondary`("#764ba2"), `useGradient`(true), `fontScale`(100), `enableTransitions`(true), `consoleSyntaxHighlight`(true), `consoleBgColor`, `consoleTextColor`, `sidebarCompact`(false), `windowOpacity`(100), `consoleFontScale`(100), `systemMonitorOpen`(true, 서버 페이지 시스템 모니터 펼침) |
| 창/언어 | `windowBounds?{width:1200,height:840,x?,y?}`, `language?` |

**BotConfig** (camelCase, `config.json`의 `relay`, `config_store/mod.rs:118-133`). 로컬 봇용 필드(`token`, `autoStart`, `mode`, `music*`)는 지워졌고, 옛 파일에 남아 있어도 무시된다.

- `prefixes`(문자열 목록, 기본 `[]`: 비어 있으면 릴레이가 응답 언어의 기본값을 쓴다. ko `사바쨩`, ja `サバちゃん`, 그 밖 `Saba-chan`). 각각 공백 없는 1~32자, 10개까지이고 어기면 `PUT /api/config/bot`이 400이다. 예전 `prefix` 문자열은 읽을 때 옮긴다(`"!saba"`·빈 값 → `[]`, 그 밖 → `[값]`, `BotConfig::normalize`). `cloud{relayUrl,hostId}` (hostId는 노드 토큰 `sbn_<hostId>.…`을 따른다: 페어링 완료·`PUT`·`DELETE /api/config/node-token`이 같이 맞춘다)
- `moduleAliases`, `commandAliases`, `instanceAliases`: 예전 전역 별명. 데몬이 시작할 때 길드를 하나라도 알면(`nodeSettings` 키, `cloudNodes[].guildId`) 아래 `nodeSettings[guild].instances`로 옮기고 비운다(`guild_words::migrate_on_start`, `chat::words::migrate`). `instanceAliases[id]`는 그 서버를 쓰는 길드마다 `words`로, 모듈 별명은 그 길드에 그 모듈 서버가 하나뿐일 때만 그 서버 `words`로(여럿이면 버리고 로그), `commandAliases[모듈]`은 그 모듈 서버마다 `commands`로 간다. 다른 서버 이름·낱말과 겹치는 낱말은 버린다
- `nodeSettings`, `cloudNodes`, `cloudMembers`
- `nodeSettings[guildId]`의 디스코드 명령 권한 (P1, `chat/permissions.rs`). 원본은 이것 하나다.
  - 키: `ownerDiscordId`(고정된 주인), `relayOwnerDiscordId`(릴레이가 알려 온 다른 주인, 경고용), `defaultLevel`(1), `members{discordId: -1..2}`, `preset`("default"·"loose"·"strict"), `commandLevels{"<instanceId>/<command>": 0..3}`, `chatbotCap`(0), 예전 금지 목록 `memberPermissions`(읽기만).
  - 레벨: 소유자 3, 관리자 2, 유저 1, 챗봇 0, 금지 -1. 요구 레벨은 `commandLevels` 덮어쓰기, 없으면 프리셋표[risk]다. default 프리셋은 safe 0, caution 1, danger 2이고, loose는 0/0/1, strict는 1/2/3이다. 내장 명령의 risk는 `status` safe, `start`·`stop` caution, 모듈 명령은 선언값(없으면 caution).
  - 액션 카탈로그(P2): 서버 명령 액션은 위와 같은 키(`<instanceId>/<command>`), 그 밖의 액션은 `<action id>`·`<instanceId>/<action id>`로 덮어쓴다. 내장 `server.start`·`server.stop`·`server.restart`·`server.status`는 명령 경로·GUI 권한 편집기와 같은 `<instanceId>/start|stop|restart|status`를 먼저 보고, 없으면 `<instanceId>/<action id>`를 본다. `PATCH /api/permissions/:guildId`의 `commandLevels`는 `/`가 없는 키를 디스코드·챗봇에서 부를 수 있고 레벨이 코드에 고정되지 않은 액션 id일 때만 받는다(아니면 `invalid_command`, `actions::overridable`). 관리 액션(`kind=admin`)의 프리셋 값은 느슨 2, 기본 2, 엄격 3이다(`actions::admin_level`). 디스코드의 `update.apply`는 덮어쓰기와 상관없이 소유자다.
  - 판정: 릴레이에서 온 요청(`ChatRequest.remote`)만 판정한다. GUI·CLI 요청은 하지 않는다. 금지는 help·list도 못 쓴다. 챗봇 경로는 `min(작성자 레벨, chatbotCap)`이다. 거부는 `PERMISSION_DENIED {need, level, via?}`.
  - 설정이 없는 길드도 같은 기본값으로 판정한다(제한 없음이 아니다).
  - 이 키들은 권한 API로만 바뀐다. 봇 설정 저장(`PATCH /api/config/bot`)은 이 키들을 이전 값으로 되돌린다(`config_store::keep_permission_keys`).
- `nodeSettings[guildId].discord`: 디스코드 길드 설정(I2, `ipc/handlers/discord.rs`). 원본은 이것 하나고 릴레이 값은 사본이다.
  - `{locale: "auto"|언어 코드, sleeping, music: {enabled, ui, queueLines 1-20, refreshInterval 2000-10000, normalize}}`. 기본값은 `auto`, false, `{true, true, 5, 4000, true}`(릴레이 기본값과 같다).
  - `/api/discord/:guild/settings`로만 바뀐다. 봇 설정 저장은 이 블록도 되돌린다(`PERMISSION_KEYS`).
  - 데몬에 값이 없는 길드는 릴레이 값을 읽는다(릴레이에 없으면 기본값). 페어링했는데 릴레이를 읽지 못하면 GET은 기본값에 `loaded:false`를 붙여 답하고, PATCH는 503 `relay_unavailable`로 거절한다(기본값을 저장해 릴레이 값을 덮지 않게).
  - 릴레이에 닿지 않거나 5xx로 보내지 못한 설정은 저장해 두고, 다음에 하트비트가 성공하면 `sync_on_connect`로 다시 보낸다(`discord::take_pending_sync`).
- `nodeSettings[guildId].instances[instanceId]`: 그 길드에서 그 서버를 부르는 낱말 `{words:[..], commands:{<명령 정식 이름>:[..]}}`(`chat/words.rs`). `/api/guilds/:guild/instances*`로만 바뀐다(`PERMISSION_KEYS`). 낱말은 공백 없는 1~32자, 항목마다 10개까지이고, 같은 길드의 다른 서버 이름·id·낱말, 모듈 이름·module.toml 별명, `help`·`도움`·`도움말`·`list`·`목록`과 겹치면 안 된다. 명령 낱말은 같은 서버의 다른 명령 이름·별명과 겹치면 안 되고, 명령은 내장(start·stop·status)이거나 모듈 정의에 있어야 한다.
  - 채팅 처리기(릴레이에서 온 길드 요청만): 대상은 인스턴스 이름·id → 그 길드 `words` → 모듈 이름·module.toml 별명 순, 명령은 그 길드·서버의 `commands` → module.toml 별명 → 명령 이름 → 내장 순이다. 여러 서버가 걸리면 `errors.multiple_instances`로 답한다. 로컬 요청(GUI·CLI)은 사용자 낱말을 쓰지 않는다. 권한 키는 언제나 정식 이름(`<instance>/<command>`)이다.

**데몬이 실제로 쓰는 설정값은 일부뿐이다.**

- `portConflictCheck`: 기동할 때 `skip_port_check` 원자 변수를 초기화하고(`ipc/mod.rs:822-828`), PUT·reset이 성공하면 갱신한다(`config.rs:28, 88-91`). start 경로와 모니터가 이 값 하나만 본다(`supervisor/mod.rs:269, 2555`).
- `language`: Python 실행의 `SABA_LANG`, 채팅 응답, 예약 공지, 익스텐션 설명 언어에 쓴다(`plugin/mod.rs:28-36`, `chat/api.rs:129`, `backup.rs:402`, `extension.rs:74`).
- `ipcPort`: `SABA_IPC_PORT`가 없을 때 데몬 포트로 쓴다(`main.rs:285-296`).
- `autoGeneratePasswords`: 미사용.

예전 `src/config/mod.rs`의 `GlobalConfig`는 지워졌다.

---

## 5. 프로토콜 계층 (`src/protocol/**`)과 명령 라우팅

### 5.1 RCON은 없다

- 데몬은 RCON을 쓰지 않는다. 예전 `protocol/rcon.rs`와 `rcon_pool.rs`는 지워졌다. 게임 명령은 모두 stdin으로 간다(§5.4, 02 RCON 정책).
- 인스턴스 모델에서도 `rcon_port`, `rcon_password` 필드를 지웠다. 예전 파일에 남은 RCON 키는 로드할 때 지우고(§4.1), PATCH로 와도 버린다(`instance.rs:979-1000, 1295-1297`). 모듈 config에도 RCON 키를 넘기지 않는다(`supervisor/mod.rs:1253-1262`).

### 5.2 REST (`protocol/rest.rs`)

- `ureq`(동기)를 쓰고 timeout은 10초다. Basic Auth를 지원한다.
- 응답 없이 연결이 EOF나 reset으로 끊기면 결과를 알 수 없다고 본다: `success:false`, `data.outcome:"unknown"`, `error`(`rest.rs:105-122`). 종료 명령처럼 서버가 끊을 수 있는 요청도 성공으로 보고하지 않는다.
- `verify_connection`은 실제 연결 없이 host와 port 유효성만 본다(`rest.rs:42-48`).
- 인증 로그에는 사용자 이름만 debug 레벨로 남긴다(`rest.rs:91`). 토큰·비밀번호 길이는 남기지 않는다.

### 5.3 ProtocolClient (`protocol/client.rs`)

- REST 한 가지만 남았다(`CommandType::Rest`). `/rest` 핸들러가 `new_rest`로 만든다(`command.rs:572`).

### 5.4 명령 라우팅

| 경로 | 동작 |
|---|---|
| `POST /api/instance/:id/command` | 모든 인터페이스의 단일 명령 경로다. 요청 해석·검증은 `crate::commands`가 한다. 모듈에 없는 이름이면 활성 익스텐션 명령을 먼저 본다(`command.rs:80-124`). 그다음 모듈 정의대로 `/rcon`(stdin), `/rest`, `lifecycle.py command`, 원시 stdin 중 하나로 보낸다(`command.rs:158-183`). 꺼져 있고 콘솔도 없으면 409 `server_not_running`이다. `Supervisor::execute_command`(`supervisor/mod.rs:1220-1266`)는 config `{command,args,protocol_mode,working_dir,rest_host,rest_port,pid,instance_id}`를 락 안에서 만들고, 락을 놓은 뒤 Python을 실행한다 |
| `POST /api/instance/:id/rcon` | 이름은 호환용이고 **언제나 stdin으로 보낸다.** `routes_commands_to_stdin`(`supervisor/mod.rs:1704-1712`)이 콘솔 있는 서버(관리 모드, 재접속 후에도 saba-runner가 stdin 유지)를 확인하면 `via_stdin`(`command.rs:430-484`)이 한 줄을 쓰고 콘솔 출력을 최대 약 1.5초 모아 `data.response`로 돌려준다. 콘솔이 없으면 409 `no_command_path`(`command.rs:392-435`) |
| `POST /api/instance/:id/rest` | 접속 정보는 `get_credentials`(게임 설정 파일)에서 읽고 인스턴스별로 캐시한다(`CredentialsCache`, `ipc/mod.rs:302-328`; `command.rs:661-689`). 실패하면 캐시를 지운다. host와 port는 payload, instance, 모듈 기본값 순으로 정하고, 동기 ureq 호출은 `spawn_blocking`으로 감싼다(`command.rs:614-621`) |
| `POST /api/instance/:id/stdin` | `/rcon`과 같은 `via_stdin` 경로다(`managed.rs:118-137`) |

---

## 6. 내장 런타임: python_env

### 6.1 Python (`src/python_env/mod.rs`)

- 버전: 포터블 CPython **3.12.8**(python-build-standalone 태그 `20250106`, `install_only_stripped`)이다(`updater/src/python_pin.rs:7-8`). 시스템 Python을 쓸 경우 3.10 이상이어야 한다(`python_env/mod.rs:45`).
- 탐색 순서: 이미 받은 포터블 Python, 시스템 Python, 자동 다운로드 순이다(`find_base_python`, `:209`). 찾은 인터프리터로 `<pyroot>/python-env` venv를 만들고 pip를 업그레이드한다(`ensure_venv`, `:114`).
- 성공한 결과는 `OnceCell`에 캐시하고, 실패는 캐시하지 않아 다음 호출이 다시 시도한다(`get_python_path`, `:56`). 모든 `run_plugin` 호출이 여기를 거친다(`plugin/mod.rs:282`). 데몬은 기동하자마자 미리 준비를 시작하고(`main.rs:320-324`), 진행은 `/api/runtime/preflight`(`preflight_snapshot`, `python_env/mod.rs:86-106`)로 보인다.
- 다운로드 URL: `https://github.com/indygreg/python-build-standalone/releases/download/{tag}/cpython-{ver}+{tag}-{triple}-install_only_stripped.tar.gz`(`python_pin.rs:41-49`). 지원 triple은 win x64, linux x64/arm64, mac x64/arm64다. `SABA_PYTHON_URL`로 미러를 쓸 수 있다.
- 다운로드 방식: Windows는 PowerShell `Invoke-WebRequest`, 그 외는 `curl`이다. 압축 해제는 `tar`를 쓴다. 풀기 전에 triple별로 고정한 SHA-256과 비교하고 다르면 지운다(`python_env/mod.rs:257-261`, `python_pin.rs:11-17, 53-61`). 미러를 쓰면 `SABA_PYTHON_SHA256`이 있을 때만 검사한다.
- **`<pyroot>`는 `constants::resolve_data_dir()`와 다르다**(`python_env/mod.rs:395-432`). 순서는 다음과 같다.
  1. `SABA_DATA_DIR`
  2. **exe 디렉토리에 쓰기 권한이 있으면 exe 디렉토리**
  3. Windows `%APPDATA%/saba-chan`, Linux `~/.local/share/saba-chan`, macOS `~/Library/Application Support/saba-chan`
- 결과적으로 Linux에서 데이터는 `~/.config/saba-chan`에, 런타임은 `~/.local/share/saba-chan` 또는 exe 옆에 흩어진다.
- 예전 `status()`와 `/api/python-env/*` 핸들러는 지워졌다.

### 6.2 Node

데몬은 더 이상 Node를 받거나 찾지 않는다. `node_env`와 `/api/node-env/*`는 로컬 봇과 함께 L1(7b5102f)에서 지웠다. Node는 GUI 빌드에만 쓰며 `>=22.12`가 필요하다(`saba-chan-gui/package.json` `engines`, 04 §1).

---

## 7. plugin/과 validator/

### 7.1 `src/plugin/mod.rs`: Python 모듈/익스텐션 실행기 (단발 프로세스)

**호출 계약**

- 실행 형태: `<venv python> <module_path> <function>`. stdin으로 config JSON을 한 번 쓰고 닫는다.
- 결과: stdout 전체, 또는 마지막 비지 않은 줄이 JSON이어야 한다(앞에 찍힌 print는 무시, `parse_plugin_output`, `:467-476`). 종료 코드가 0이 아니면 Err다. 파싱에 실패해도 Err다.
- 진행률: stderr의 `PROGRESS:{json}` 줄을 `ExtensionProgress{percent,message,step,total,label}`로 파싱해 콜백에 넘긴다(`:68-79, 391-410`). 그 밖의 stderr는 debug 로그로만 남긴다.
- 타임아웃: 기본 120초(`DEFAULT_PLUGIN_TIMEOUT_SECS`, `:150`), install 계열은 1800초다. 기한은 stdin 쓰기부터 종료 대기, stdout·stderr 수집까지 한 번에 잰다. 넘기면 프로세스 트리(Unix는 프로세스 그룹, Windows는 `taskkill /T`·AppContainer job)를 끝내고 읽기 작업을 멈춘 뒤 Err (`plugin/mod.rs:421-488`).
- `PluginCall`(`:161-177`)은 데몬 락 안에서 호출을 만들고 락을 놓은 뒤 실행하기 위한 묶음이다.

**자식 프로세스 환경**

- `PYTHONPATH` = `<data_dir>/python-guard` + `<extensions_dir의 부모>` + `<modules_dir>` + 기존 값(`:313-350`). 앞의 둘은 `extensions_dir`가 디렉토리일 때만 넣는다. `python-guard/sitecustomize.py`는 활성 익스텐션만 import되게 막는다(`:113-125`).
- `SABA_ENABLED_EXTENSIONS` = `config.json`의 `extensions.enabled`를 쉼표로 연결한 값. 목록이 없으면 빈 문자열이다(`:317-322`).
- 데몬 연결 정보 `SABA_IPC_PORT`, `SABA_IPC_URL`, `SABA_TOKEN_PATH`, `SABA_LANG`(`daemon_env`, `:14-38`).
- 확인되지 않은 구성요소(Windows)는 AppContainer 안에서 띄우고 `SABA_TOKEN_PATH`를 빼며 `PYTHONPYCACHEPREFIX`·`TEMP`·`TMP`를 격리용 폴더로 바꾼다(`sandbox::isolation_for`, 02 문서 §2.1).
- 첫 실행 때 내장 공용 유틸리티(`i18n.py`, `daemon_rcon.py`)를 모듈 폴더에 맞춰 쓴다(`:127-147, 335-341`).

**모듈의 `lifecycle.py`가 응답해야 하는 함수** (코어에서 직접 호출하는 것만)

- `list_versions` → `{latest:{release}}` 또는 `{latest_release}`
- `install_server` → `{success, executable_path?, install_path?, message?}`
- `import_settings` → `{success, settings{}, executable_path?, detected?}`
- `instance_commands` → `{success, commands:[명령 필드 + group?], components?}` (`[commands] hook = true`일 때)
- `get_credentials` → `{success, rest_username?, rest_password?, ...}`. 데몬은 REST 값만 쓴다(`command.rs:579-584`)
- `command` → `{success, message}`
- 나머지(start/stop/status/properties/eula/diagnose 등)는 supervisor가 호출한다.

예전 `run_extension*`(dead code)은 지워졌다. 익스텐션 훅 실행은 `extension/mod.rs`가 맡는다.

### 7.2 `src/validator/mod.rs`

- `validate_setting_value`(`:72`)는 module.toml `SettingField`의 `field_type`으로 값을 검사한다.
  - `number`: min/max 범위
  - `boolean`: true/false 또는 문자열 "true"/"false"
  - `select`: `options`에 포함되는지
  - `text|password|file`: 문자열(숫자, 불리언도 허용)
  - 알 수 없는 타입은 통과시킨다.
  - `required==Some(true)`인데 값이 `null`이거나 빈 문자열이면 에러다.
- `PATCH /api/instance/:id`는 `validate_settings_patch`(`:281-297`)로 **요청에 들어 있는 필드만** 검사한다(`instance.rs:1027`). 보내지 않은 필수 필드는 오류가 아니다.
- `check_port_conflicts`(`:312`)는 주어진 인스턴스 집합과 포트가 겹치는지 검사한다. 모듈이 지원하는 프로토콜의 포트만 본다. supervisor의 start 경로와 모니터는 **실행 중인** 인스턴스만 넘긴다.
- 시작 전 검사(`Supervisor::port_conflict_response`)는 실행 중인 인스턴스와 겹치는 포트가 없으면 `start_check_ports`(같은 포트에 모듈 설정 중 `port = true`인 값을 더한 것)를 `ports_held_elsewhere`로 TCP·UDP 둘 다 본다. Windows는 포트를 묶지 않고 OS의 사용 중 포트 표(`GetExtendedTcpTable`의 듣는 포트, `GetExtendedUdpTable`의 묶인 포트)를 읽는다. IPv4는 주소와 상관없이, IPv6는 `[::]`에 묶인 것(dual-stack 서버)만 센다. 0.0.0.0에 포트를 열면 방화벽 허용 알림이 뜨기 때문이다. 그래서 Windows에서는 시스템 예약 포트 범위는 이 검사로 잡지 않는다. 다른 OS는 0.0.0.0에 묶어 본다. 다른 프로그램이 잡은 포트도 같은 `port_conflict` 응답(HTTP 200, `success:false`, `conflicts:["Port N (이름) is in use by another program"]`)으로 돌려준다. 이 인스턴스가 이미 실행 중이면 보지 않는다.
- `GET /api/servers`의 `port_conflicts`도 같은 함수를 쓰되(`server.rs:251-275`), "켜면 부딪힐" 서버를 미리 보여 주려고 정지된 인스턴스까지 모두 넘긴다.

### 7.3 기타

- `process_monitor.rs`는 `sysinfo` 0.30으로 전체 프로세스 스냅샷(`pid,name,exe,cmd`)을 만든다. `spawn_blocking`으로 감싼 async 버전도 있다.
- `daemon_log.rs`는 tracing Layer와 링 버퍼(10k)를 제공하고, `get_since(since,count=200)`로 조회한다.
- `audit.rs`(A10): 서버 시작·정지·재시작(`/api/instance/:id/*`, `/api/server/:name/*`, `/managed/start`), 게임 명령(`execute_command`, `/rcon`·`/rest` 라우트), 콘솔 입력(`/stdin`), 속성 쓰기·서버 초기화(`/properties` PUT, `/server/reset`), 예약 백업(`backup`, 서버가 꺼져 건너뛴 것 포함), 액션 카탈로그의 관리 액션(`actions::run`, `action`=액션 id, 입력은 가려서 `args`)을 `<data_dir>/audit.log`에 한 줄 JSON으로 남긴다(1 MiB에서 `audit.log.1`로 넘김, 인자·줄은 300자까지). 출처는 task-local `Origin`: HTTP 요청은 미들웨어가 `X-Saba-Client`(GUI `gui`, CLI `cli`, 없으면 `api`)로, 릴레이 에이전트는 `discord`+요청자·길드 id(릴레이가 `payload.userName`·`payload.guildName`을 보내면 그 이름도)로, 예약 작업은 `schedule`로 정한다. `discord`·`schedule`·`daemon`은 헤더로 받지 않고 `api`로 본다. 기록 전에 `audit::redact`가 비밀을 가린다: 모듈이 `password`로 선언했거나 이름에 pass·pwd·token·secret·apikey가 든 명령 인자, 그런 인자를 받는 명령의 콘솔 줄(첫 낱말이 이름·별명·`console_template` 첫 낱말과 같거나 이름부터 비밀스러운 줄)은 첫 낱말 뒤 전체, 인스턴스의 `password` 설정값·REST 비밀번호(4자 이상)는 어디에 나오든. 넘기기(rename)에 실패해도 기록은 이어서 남긴다. 재시작 안의 정지·시작은 `quietly`로 따로 남기지 않는다. 로컬 클라이언트는 같은 토큰을 쓰므로 출처는 기록용이지 보안 경계가 아니다.
- `events.rs`(P4): 데몬 이벤트 버스. `emit(kind, data)`가 번호를 붙여 최근 200건 버퍼와 broadcast 채널(256)에 같은 잠금 안에서 넣는다(구독 시작 때 빠지거나 겹치는 건이 없게). `emit_live`는 번호·버퍼 없이 지금 붙은 구독자에게만 보낸다(진행률). 내는 곳: `AuditLog::record`(`action.run`), 감독자 종료 판정(`instance.exit`), `ProvisionTracker::update`(`provision.progress`, live), 백업 핸들러 `take_backup_locked`(`backup.done`, 수동·예약·업데이트 전·복원 전 모두), 업데이트 확인 `check_and_follow_up`(`update.available`), 예약 예고 `count_down`(`schedule.warn`). 파일에는 남기지 않는다.
- 콘솔 프로그램을 부를 때는 창을 숨긴다: 데몬은 `utils::apply_creation_flags`(tokio), 업데이터·인스톨러와 std `Command`는 `saba_chan_updater_lib::process::hide_console`. 분리 실행(`DETACHED_PROCESS`)된 데몬이나 GUI 프로그램(인스톨러·업데이터)이 콘솔 프로그램을 플래그 없이 부르면 부를 때마다 검은 창이 뜬다.
- `utils.rs`는 Windows `CREATE_NO_WINDOW` 플래그와 경로 한 칸 이름 검사(`is_single_path_component`)를 다룬다(`utils.rs:8-20`).

---

## 8. updates.rs: 업데이트 엔드포인트와 외부 URL

- `UpdateState`가 `UpdateManager`(updater 크레이트)를 `Arc<RwLock>`으로 감싼다. 설정은 `config.json`의 `updates` 절에서 읽고(`load_updater_config`, `updates.rs:916-934`), 없거나 깨졌으면 `UpdateConfig::default()`를 쓴다.
  - 필드(`updater/src/lib.rs:236-271`): `enabled`(기본 true), `check_interval_hours`(3), `auto_download`(false), `install_root`, `relay_url`(None이면 `https://saba-chan.online`), `channel`(`stable`).
- `PUT /api/updates/config`는 바꾼 설정을 `config.json`에 저장한다(`updates.rs:900-902`).
- 주기 확인은 데몬의 `spawn_update_scheduler`가 한다(`updates.rs:256`). `check_interval_hours`마다 확인하고 `auto_download`이면 받아 둔다.
- **접속하는 외부 URL**
  - 릴레이: `GET {relay_url}/v1/updates/check?channel&platform&current&client_id`, `GET /v1/updates/manifest/:version`, `POST /v1/updates/event`(익명 진행 이벤트). 모든 요청에 `X-Saba-Client: 1` 헤더(`updater/src/relay.rs:1-25, 160-195`).
  - manifest의 각 컴포넌트 `url`(R2 공개 주소). 받을 때 `sha256`을 확인한다(`updater/src/download.rs:40`).
  - manifest는 번들된 공개키로 Ed25519 서명을 검증하고, 플랫폼이 다르거나 현재보다 낮은 버전이면 거부한다(`relay.rs:233-252`, `signature.rs`).
  - `/api/modules/manifest*`, 익스텐션 목록(`ExtensionManager::fetch_remote`), 설치기는 같은 함수 `relay::latest_catalog`를 쓴다(`relay.rs:107-115`). 데몬 쪽 호출은 모두 `None`을 넘기므로 기본 릴레이와 `stable` 채널을 쓰며 `relay_url` 설정을 보지 않는다(`server.rs:503, 533`, `extension/mod.rs:1396`).
  - Python 런타임만 예외로 `github.com/indygreg/python-build-standalone` 릴리즈에서 받는다(고정 해시 검사, §6).
- **apply 규칙**(`updates.rs:475-596`)
  - 대상에 GUI, Updater, CoreDaemon 중 하나라도 있으면 **전부** 업데이터 exe에 맡긴다. 이때 `needs_updater`를 채우고 `requires_updater:true`를 돌려준다.
  - 그렇지 않으면 데몬이 직접 적용하고, 모듈은 refresh, 익스텐션은 rescan으로 핫 리로드한다.
  - 업데이터 실행은 클라이언트가 `POST /api/updates/launch-updater`로 데몬에 맡긴다(`updates.rs:598-675`). 데몬이 이번 적용에만 쓰는 실행 표(`launch_ticket::issue`)를 만들어 `--apply --wait-pid <pid> --launch-token <표>`로 업데이터를 detach로 띄우고, 0.5초 뒤 등록된 GUI를 끄고(스스로 끝날 틈 3초) `stop_requested`로 스스로 종료한다. GUI가 떠 있었으면 `--relaunch`는 그 GUI다(GUI가 뜨면서 데몬을 띄운다).
- `/api/updates/*`도 다른 라우트처럼 토큰 인증을 거친다(`ipc/mod.rs:812-816`). `relay_url`을 바꿔도 manifest 서명 검증은 그대로라 다른 서버가 파일을 바꿔 끼울 수는 없다.

---

## 9. 불일치, 데드 코드, 반쯤 구현된 기능, 위험 지점

### 9.A 오너 원칙 위반 (인터페이스가 데몬을 우회하는 곳)

1. ~~**GUI가 `settings.json`을 직접 읽고 쓴다.**~~ → 해결: GUI는 `config.json`의 `gui`를 읽기만 직접 하고(Tauri 셸 `saba-chan-gui/src-tauri/src/settings.rs:71-84`), 저장은 데몬 API(`PUT /api/config/gui`)로 하며 데몬이 응답하지 않을 때만 파일에 직접 쓴다(`saba-chan-gui/src/api/createApi.js:57-74`, `src-tauri/src/native.rs:189-212`, `write_gui_section` `settings.rs:49-51`).
2. ~~**GUI가 relay 서버를 직접 호출한다.**~~ → 해결: GUI(Tauri 셸 `saba-chan-gui/src-tauri/src`, 브리지 `saba-chan-gui/src/api`)에 릴레이 직접 호출이 없다. 봇 설정·초대·페어링은 데몬의 `/api/relay/*`, `/api/config/bot`을 거친다.
3. ~~**봇 실행 로직이 GUI에 중복돼 있다.**~~ → 해결: 로컬 봇과 범용 `/api/ext-process/*`가 지워졌다(L1). 디스코드 연동은 데몬의 `relay_agent.rs` 하나뿐이다.
4. **GUI가 데몬의 경로 정책을 결정한다.** `saba-chan-gui/src-tauri/src/lifecycle.rs:173-183`에서 `SABA_INSTANCES_PATH`, `SABA_MODULES_PATH`, `SABA_IPC_PORT`를 GUI가 정해 주입한다.
5. **CLI가 로컬 파일을 직접 읽는다.** (CLI 참조는 PR #6 리메이크 기준)
   - 데몬을 찾고 띄우는 `saba-chan-cli/src/bootstrap.rs`만 `config.json`의 `gui` 섹션을 직접 읽는다(`ipcPort`, `language`, `:14-46`). 파일 머리 주석이 이를 유일한 예외로 적어 둔다(`:1-6`).
   - ~~데몬은 `SABA_IPC_PORT`를 먼저 보므로 포트가 다를 수 있다~~ → 해결: CLI도 `SABA_IPC_PORT` > `gui.ipcPort` > 기본값 순으로 읽어 데몬(`main.rs:285-296`)과 같다(`bootstrap.rs:18-25`). 다만 GUI가 데몬에만 넣어 준 환경변수는 따로 띄운 CLI가 보지 못한다.
   - ~~`scan_directory` 로컬 fs 폴백, `get_module_locales` 로컬 폴백, 컴파일되지 않는 `gui_config.rs` 사본~~ → 해결: 예전 `config.rs`·`gui_config.rs`가 리메이크에서 지워졌다. CLI 문구(`locales/<lang>/cli.json`)는 로컬에서 읽는다(`i18n.rs:38`).
6. ~~**Discord 봇(server-chan)이 `bot-config.json`을 파일 경로(`BOT_CONFIG_PATH`)로 받는다.**~~ → 해결: 로컬 봇이 없다. 릴레이 에이전트가 하트비트로 봇 설정을 올린다(§1.4).
7. **토큰 파일 직접 읽기**(GUI, CLI, Python 모듈)는 인증 부트스트랩 구조상 피할 수 없으므로 예외로 본다.
8. ~~**반대 방향 위반: 코어 안에 특정 익스텐션 이름이 하드코딩돼 있다.**~~ → 해결: `docker`·steamcmd 분기가 코어에서 빠졌다. 설치 방식은 `install_method_<방식>` 슬롯으로 범용화됐다(`instance/mod.rs:81-166`, `instance.rs:1547-1560`). 이름은 테스트에만 남아 있다.

### 9.B 보안과 정합성

1. ~~`/api/updates/*`에 인증이 없다~~ 해결됨: 업데이트 라우터도 인증 미들웨어 안에 있다(`ipc/mod.rs:812-816`). 업데이트 출처 설정은 `relay_url`뿐이고, 서명되지 않은 manifest는 받지 않는다.
2. ~~**`/health`는 인증이 필요하다.**~~ → 해결(A6): `/health`는 토큰 없이 답하고 보낸 토큰의 상태(`token`)만 알려 준다. 자세한 진단은 인증이 필요한 `/api/diagnostics`.
3. ~~**토큰이 없으면 fail-open이다**~~ → 해결: 캐시도 파일도 없으면 401로 거부한다(`auth.rs:129-133`). ~~비교가 상수 시간이 아니다~~ → 해결: `constant_time_eq`로 끝까지 비교한다(`auth.rs:102-105, 142`).
4. ~~**단일 인스턴스 보장이 없다.**~~ → 해결: `daemon.lock` 배타 잠금을 토큰 생성 전에 잡고(`main.rs:300-315`), Windows에서는 `SO_REUSEADDR`를 켜지 않는다(`ipc/mod.rs:845-848`). §1.6.
5. ~~`/api/ext-process/:name/start`는 임의 명령을 실행~~(라우트 삭제). ~~`/api/fs/scan-dir`는 임의 디렉토리를 열람한다~~ → 해결: 지웠다. 마이그레이션은 게임 폴더 판정(`POST /api/modules/detect`)만 쓴다(D2).
6. ~~**relay 토큰 유출 가능성**: `?relayUrl=`로 임의 호스트에 `Authorization: Bearer <node_token>`을 보낼 수 있다~~ → 해결: 서명 요청(host status·nodes·members)은 `signed_relay_url`로 설정한 릴레이(없으면 기본)만 쓰고, 요청 주소가 그와 같거나 루프백(모의 릴레이)일 때만 따른다(`relay.rs:46-67, 235-259`). 챗봇 동기화도 설정한 릴레이만 쓴다(`chatbot.rs:170-173`). ~~서명 없는 bot-invite·pair는 `relayUrl`을 그대로 따른다~~ → 해결: 페어링(시작·상태)과 bot-invite도 같은 규칙을 쓴다(`relay.rs:434, 462, 490`, 59a93b8).
7. ~~**`POST /api/modules/manifest/:id/install`에 경로 검사가 없다**~~ → 해결: `..`, `/`, `\`를 거부한다(`server.rs:530-532`). `get_module_locales`는 여전히 이름을 그대로 `join`한다(`config.rs:209-211`).
8. **비밀값이 평문으로 응답에 실린다.** `GET /api/instances`, `/api/servers`에는 REST 비밀번호가 들어 있다. ~~`/api/config/bot`의 Discord 토큰~~(BotConfig에서 `token` 삭제).
9. **익스텐션 설치에 사용자 지정 `download_url`을 허용하고 sha256은 선택**이다(`extension.rs:405-423`). ~~포터블 Python/Node 다운로드에 체크섬 검증이 없다~~ → 해결: Python은 고정 해시로 검사하고(§6.1), Node 다운로드는 지워졌다.
10. ~~**프로비저닝 dismiss 롤백 경쟁**~~ → 해결: 롤백은 신규 생성으로 표시된 이름(`mark_creating`)일 때만 한다. 기존 인스턴스의 업데이트 실패는 진행 표시만 지운다(`instance.rs:719-757`, `ipc/mod.rs:379-410`).
11. ~~**`/api/daemon/shutdown`과 `unregister?shutdown=true`는 토큰만 cancel한다**~~ → 해결: 둘 다 `stop_requested`를 cancel하고, main의 단일 종료 경로가 `daemon.shutdown` 훅과 클라이언트 정리를 거친다(`ipc/mod.rs:630-636`, `main.rs:511-578`).
12. ~~**`SABA_ENABLED_EXTENSIONS` 경로 해석 버그**~~ → 해결: `config.json`의 `extensions.enabled`를 `ConfigFile::default_location()`(`SABA_DATA_DIR` 반영)으로 읽는다(`plugin/mod.rs:317-322`).
13. ~~**락을 오래 잡는 문제**~~ → 해결(C12): supervisor 함수는 `&RwLock`을 받아 단계마다 짧게 잠그고 Python은 락 밖에서 실행한다(`PluginCall`, `supervisor/mod.rs:1220-1266`). 마이그레이션 `import_settings`(`instance.rs:648-686`), managed 시작(`managed.rs:68`), 익스텐션 다운로드(`extension.rs:438-449`, write lock 전에 받음)도 락 밖이다. 2초 모니터는 여전히 `monitor_processes` 동안 write lock을 잡는다(`main.rs:436-437`).
14. ~~**REST 명령이 async 핸들러 안에서 동기로 실행된다**~~ → 해결: `spawn_blocking`으로 실행한다(`command.rs:614-621`).
15. ~~**ext-process 출력이 stderr로 새어 나간다.**~~ → 해결: `ext_process.rs`가 지워졌다.

### 9.C 데드 코드와 반쯤 구현된 기능

- ~~`src/config/mod.rs`의 `GlobalConfig`는 미사용이다~~ → 해결: 지워졌다.
- ~~`handlers/python_env.rs`의 세 핸들러는 라우트가 없다~~ → 해결: 핸들러와 `pip_install*`가 지워졌다. 런타임 상태는 `/api/runtime/preflight`가 보인다.
- ~~`plugin::run_extension*`, `RestClient::with_basic_auth`, `ServerResponse::success/error`, `CommandResponse`, `BotConfig`(ipc/mod.rs), `ExtensionInitTracker::is_initializing`, `InstanceStore::save`, `ServerInstance::ext_f64`, `process_monitor::find_by_name*` 동기 버전 미사용~~ → 해결: 모두 지워졌다(`ext_str`는 테스트 전용, `instance/mod.rs:128-131`).
- ~~업데이트 설정의 `check_interval_hours`, `auto_download`, `auto_apply`는 저장만 되고 아무 동작이 없다~~ → 해결: `config.json`에 저장되고, 데몬 스케줄러가 주기와 `auto_download`를 쓴다(`updates.rs:256`, `:899-902`). `auto_apply`는 설정에서 지워졌고 적용은 언제나 사용자가 고른다(`updater/src/lib.rs:236-252`).
- ~~`skip_port_check` 계열이 따로 논다~~ → 해결: 기동 시 `gui.portConflictCheck`로 초기화하고, start 경로와 모니터가 같은 원자 변수 하나만 본다(§4.4).
- `GuiSettings.autoGeneratePasswords`는 데몬에서 효과가 없다. ~~`ipcPort`~~는 데몬 포트로 쓴다(§4.4).
- 비 Windows 부트 선택기는 키 입력을 지원하지 않는데도 DEL 안내를 표시한다(`boot_selector.rs:244-248, 308`).
- `ServerStartRequest.module`이 필수 필드다(`ipc/mod.rs:132-136`). 인스턴스에 이미 모듈 정보가 있는데도 `/api/server/:name/start` 호출자가 다시 보내야 한다. `/api/instance/:id/start`는 이 값을 데몬이 채운다(`server.rs:696`).
- `python_env`의 데이터 디렉토리가 `constants::resolve_data_dir()`와 어긋난다(§6). 같은 파일 상단 주석은 `<data_dir>`라고 적고 있다.
- `install_server` 진행 콜백은 `step/total`을 그대로 넘기지만, 완료될 때는 `step:0,total:1`로 덮어쓴다(`instance.rs:565-574`). 백그라운드 설치의 진행 단계 정보가 사라진다.
- ~~`CLAUDE`/문서 주장 검증: `src/ipc/updates.rs:13`의 doc-claim "`/api/updates/apply` — 업데이터 exe 스폰하여 적용"은 코드와 다르다~~ → 해결: 모듈 머리 주석이 "`requires_updater`로 돌려주고 실제 적용은 `launch-updater`가 한다"로 바뀌었다(`updates.rs:13-14`). 핸들러 주석(`updates.rs:465`)에는 "업데이터 exe를 스폰하여 적용"이 아직 남아 있다. 그 밖의 README/docs 내용은 이 문서에서 근거로 쓰지 않았다.

### 9.D 테스트 계약 (`tests/daemon_integration.rs`)

- `boot_ipc()`(`:110-145`)는 `SABA_AUTH_DISABLED=1`로 인증을 끄고(디버그 빌드라 동작), 임시 instances 디렉토리와 `./modules`를 쓴다.
  - `IPCServer::new`는 `<data_dir>`의 ConfigStore와 익스텐션 디렉토리를 읽지만, 테스트 빌드에서는 dev-dependency의 `test-isolation` 기능으로 `<data_dir>`가 프로세스별 임시 폴더가 된다(`updater/src/constants.rs:127-141`, `Cargo.toml:83`). 사용자 데이터 디렉토리에 닿지 않는다.
- 테스트가 기대하는 계약
  - 존재하지 않는 인스턴스: `GET /api/instance/:id`와 `POST .../command`가 **404**를 주고 `error` 필드를 포함해야 한다(`:426-462`).
  - 생성한 인스턴스가 `/api/servers`에 나타나야 한다.
  - 삭제한 뒤 GET하면 404여야 한다.
  - `check-update`: 없는 인스턴스는 404, `extension_data`가 비어 있으면 `update_available:false`.
- 인증은 `tests/api_contract.rs`가 실제 서버로 모든 라우트가 토큰 없이 401인지 확인한다. 단일 인스턴스는 `tests/single_instance.rs`, 릴레이 에이전트 왕복은 `tests/relay_agent_e2e.rs`가 본다.

## 10. 시스템 모니터 (`system_monitor.rs`)

- `GET /api/system/metrics`(`ipc/handlers/system.rs`)가 부를 때만 표본을 뜬다. 백그라운드 루프는 없고, 아무 클라이언트도 보지 않으면 비용이 없다. 0.9초 안의 재요청은 직전 표본을 준다(`MIN_SAMPLE_INTERVAL`). 직전 표본이 10초보다 오래됐으면 짧은 간격으로 두 번 떠서 비율을 만든다(`STALE_AFTER`).
- 인스턴스 몫 = `tracker.get_pid`의 PID와 그 자손 프로세스의 합(`attribute`). saba-runner 자체는 인스턴스 몫이 아니다. `cpu_percent`는 기계 전체 기준(코어 수로 나눔).
- CPU·메모리: 인스턴스 - 나머지(`others_*`) - 유휴(전체 - 사용). 전체 값은 인스턴스 합보다 작아지지 않게 맞춘다.
- 디스크 IO: 전체는 OS의 물리 디스크 카운터로 잰다(`system_monitor/physical_io.rs`; Windows는 PDH `\PhysicalDisk(_Total)\Disk Read/Write Bytes/sec`, Linux는 `/proc/diskstats`의 `/sys/block` 디스크, loop·dm·md 등 겹치는 장치 제외). `source:"physical"`. 프로세스 IO 카운터는 캐시에서 읽은 것까지 세서 실제 디스크와 수십 배 어긋나므로(0.1.4 PC 확인: 13.9MB/s 대 실제 0.35MB/s) 전체를 인스턴스별로 나누지 않는다(`per_instance:false`). 인스턴스의 `disk_read_bps`·`disk_write_bps`는 그 프로세스들의 IO 요청(캐시 포함)이다. 물리 카운터가 없으면 프로세스 IO 합을 쓰고 `source:"process"`. 유휴 몫은 클라이언트가 최근 최대값을 눈금으로 정한다.
- 네트워크: 인터페이스 합(루프백 제외)만 준다. 프로세스별 값은 싸게 얻을 길이 없다(Windows는 ETW·관리자 권한). `per_instance`는 항상 false.
- 디스크 용량: 데이터 폴더나 인스턴스 폴더(인스턴스 폴더 + `working_dir`)가 있는 디스크만, 데이터 폴더 디스크가 먼저. 인스턴스 몫은 폴더 크기를 별도 스레드에서 재고 10분 재사용한다(`DIR_SIZE_TTL`). `?sizes=false` 요청(GUI의 접힌 요약)은 새로 재지 않고 마지막 값을 쓴다. 아직 못 쟀으면 `sizes_at_ms`가 null.
- `?sizes=false` 요청은 물리 카운터가 있으면 프로세스 IO도 읽지 않는다: 그때 인스턴스 `disk_read_bps`·`disk_write_bps`는 0이다. 그 뒤의 전체 요청은 짧은 간격으로 새로 잰다. 물리 카운터 읽기가 한 번 실패하면 10초까지 직전 값을 쓴다. 3분 동안 요청이 없으면 표본기(sysinfo)를 버리고 다음 요청 때 다시 만든다(`IDLE_DROP_AFTER`).
