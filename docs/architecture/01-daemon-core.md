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

### 1.1 `main()` 흐름 (`src/main.rs:122-668`)

1. **인자 처리** (`main.rs:127-149`)
   - `module schema|validate <폴더>...`와 `extension validate <폴더>...`는 데몬을 띄우지 않고 검사 결과만 찍은 뒤 종료 코드(실패 1, 사용법 오류 2)로 끝난다(`main.rs:32-92`).
   - 나머지 인자는 `daemon_args`(`main.rs:108-120`)가 본다. `--spawned`/`--after-update`, `--version`/`-V`, `--help`/`-h`만 받는다.
   - 모르는 인자는 **데이터 폴더를 건드리기 전에** 사용법을 찍고 종료 코드 2로 끝낸다(`main.rs:145-148`).
2. **부트 모드 결정** (`main.rs:150-160`)
   - `--spawned` 인자가 있거나, stdin과 stdout 중 하나라도 터미널이 아니면 부트 선택기를 건너뛴다(`boot_mode = None`).
   - 그 외에는 `boot_selector::run()`이 `Gui | Cli | DaemonOnly` 중 하나를 고른다.
3. **로그 버퍼와 tracing 초기화** (`main.rs:162-218`)
   - `DaemonLogBuffer`(링 버퍼 10,000줄, `daemon_log.rs:21`)와 `DaemonLogLayer`를 모든 모드에 합성한다.
   - 필터는 `RUST_LOG`이며 기본값은 `info`다.
   - CLI 모드는 CLI 대화형 셸이 터미널을 차지하므로 로그를 `<data_dir>/daemon.log` 파일로 보낸다(truncate, `main.rs:179-187`). 나머지 모드는 stderr로 보낸다.
4. **IPC 포트 결정** (`main.rs:225-238`)
   - `SABA_IPC_PORT` 환경변수, 없으면 `config.json`의 `gui.ipcPort`, 둘 다 없으면 `DEFAULT_IPC_PORT = 57474`(`constants.rs:32`)다.
   - **바인드 주소는 `127.0.0.1`로 고정이다.** 포트는 `plugin::set_ipc_port`로 Python 실행 환경에도 넘긴다.
5. **단일 인스턴스 잠금** (`main.rs:240-255`): `<data_dir>/daemon.lock`에 OS 배타 잠금을 건다(`instance_lock.rs:69-72`). 이미 다른 데몬이 잡고 있으면 그 pid·주소를 찍고 종료 코드 2로 끝낸다. 잠금 파일을 열 수 없는 환경이면 경고만 하고 진행한다. §1.6 참조.
6. `launch_ticket::record_core_version`으로 코어 버전을 기록하고(`main.rs:258`), Python 런타임 준비를 백그라운드로 시작한다(`main.rs:260-264`, 진행은 `/api/runtime/preflight`).
7. **Supervisor 생성과 초기화** (`main.rs:268-278`)
   - `plugin::resolve_modules_dir()`로 모듈 경로를 정하고 `Supervisor::new()`, `initialize()` 순으로 호출한다.
   - `initialize()`는 모듈 탐색, 인스턴스 로드, 고아 managed 프로세스 재연결(`managed.pid`)을 한다(`supervisor/mod.rs:123-139`).
   - `server_version` 백필은 모듈 Python 호출이라 기동을 막지 않게 별도 task로 돈다(`main.rs:278`).
8. **IPC 토큰 생성** `ipc::auth::generate_and_save_token()` (`main.rs:281-284`). 실패해도 경고만 하고 진행한다.
9. **IPC 서버 객체 생성** (`main.rs:287`): `IPCServer::new`(`ipc/mod.rs:578-618`)가 ExtensionManager를 만들고 `discover()`를 호출한다. ConfigStore(`config.json`)도 이때 만든다.
10. Supervisor에 `extension_manager`와 `provision_tracker`를 주입한다(`main.rs:290-294`).
11. **백그라운드 태스크 기동**
    - 익스텐션 `daemon.startup` 훅 비동기 디스패치(`main.rs:299-345`). 진행 상황은 `ExtensionInitTracker`에 기록되고 `/api/extensions/init-status`로 노출된다.
    - 프로세스 모니터: 2초 주기로 `process_monitor::get_running_processes_async()` 스냅샷을 찍고 `sup.monitor_processes()`를 호출한다(`main.rs:351-398`). 이때 supervisor **write lock**을 잡는다. 크래시로 끝난 서버는 `take_pending_restarts`로 모아 락 밖에서 `server::auto_restart`로 다시 띄운다(`main.rs:391-396`).
    - 업데이트 주기 확인 `spawn_update_scheduler`(`main.rs:401`)와 인스턴스 예약 작업(백업·재시작·공지·업데이트) 실행기 `backup::spawn_scheduler`(`main.rs:403`, 30초 주기)를 띄운다.
    - 하트비트 reaper: 30초 주기로 90초 넘게 하트비트가 없는 클라이언트를 정리한다(`main.rs:405-424`, `handlers/client.rs:97-117`).
    - 렌더러 watchdog은 없다. 인터페이스가 모두 떠나도 데몬은 남고 GUI를 다시 띄우지 않는다. 다음에 켠 GUI·CLI가 다시 붙는다(C14).
    - DaemonOnly 모드: `daemon_log::daemon_terminal_loop`가 500ms마다 각 인스턴스 콘솔을 stderr에 출력한다(`daemon_log.rs:192`).
    - 종료 경로는 하나다(`main.rs:533-599`). Ctrl+C, SIGTERM(Unix) 또는 `stop_requested`(API `/api/daemon/shutdown`, `unregister?shutdown=true`, `launch-updater`)를 받으면 `daemon.shutdown` 훅(30초 상한), 클라이언트 정리, `shutdown_token` cancel 순으로 정리한다.
12. **인터페이스 스폰** (`main.rs:601-659`)
    - IPC 포트가 열릴 때까지 최대 15초 기다린다(`boot_selector::wait_for_ipc_port`).
    - GUI 모드: `saba-chan-gui --spawned-by-daemon`을 detach로 띄우고 콘솔 창을 숨긴다(Windows만).
    - CLI 모드: `saba-chan-cli`를 stdio를 상속해 같은 콘솔에 붙인다. 인자 없이 터미널에서 열린 CLI는 대화형 셸로 들어간다(`saba-chan-cli/src/main.rs:65-69`, PR #6 기준).
    - 인터페이스 바이너리는 데몬 exe와 같은 디렉토리에서만 찾는다(`boot_selector.rs:445-462`).
13. **`ipc_server.start().await`** (`main.rs:662`): 이 호출이 블로킹 메인 루프다. 바인드에 성공하면 릴레이 에이전트 자동 시작을 띄우고(`ipc/mod.rs:787-789`), `shutdown_token`이 cancel되면 graceful shutdown한다.

### 1.2 부트 선택기 (`src/boot_selector.rs`)

- 마지막 선택을 `config.json`의 `boot` 네임스페이스에 `{"last_mode":"gui"|"cli"|"daemononly"}` 형태로 저장한다(`boot_selector.rs:58-71`). `daemon_only` 별칭도 받는다.
- 5초 카운트다운 동안 DEL 키를 누르면 메뉴가 열린다(↑/↓, 1-3, Enter).
- **키 입력은 Windows에서만 구현돼 있다.** 비 Windows에서는 `poll_key_fallback`이 그냥 sleep하므로(`boot_selector.rs:244-248`) 메뉴를 열 수 없고, 항상 마지막 모드로 자동 부팅한다. 화면의 "Press [DEL]" 안내(`:308`)만 남아 있다.

### 1.3 환경변수 목록 (코어가 읽는 것)

| 변수 | 위치 | 의미 |
|---|---|---|
| `SABA_IPC_PORT` | `main.rs:226` | IPC 포트 (없으면 `gui.ipcPort`, 기본 57474) |
| `RUST_LOG` | `main.rs:173` | tracing 필터 |
| `SABA_DATA_DIR` | `constants.rs:56`, `python_env/mod.rs:398` | 데이터 루트 |
| `SABA_TOKEN_PATH` | `constants.rs:83` | IPC 토큰 파일 경로 |
| `SABA_MODULES_PATH` | `constants.rs:95` | 모듈 디렉토리 |
| `SABA_EXTENSIONS_DIR` | `constants.rs:107` | 익스텐션 디렉토리 |
| `SABA_INSTANCES_PATH` | `constants.rs:119` | 인스턴스 루트 |
| `SABA_AUTH_DISABLED=1` | `ipc/auth.rs:104` | 토큰 검사 비활성화. **디버그 빌드에서만** 듣는다 |
| `SABA_PYTHON_URL`, `SABA_PYTHON_SHA256` | `updater/src/python_pin.rs:32-38` | 포터블 Python 미러 주소와 그 해시 |
| `SABA_UPDATER_PATH` | `ipc/updates.rs:641-643` | 업데이터 실행 파일 경로 |
| `SABA_LOCALES_DIR` | `chat/i18n.rs:95` | 채팅 응답 로케일 폴더 |
| `APPDATA`, `HOME` | 경로 해석 | 플랫폼 기본 경로 |

**데몬이 자식 프로세스에 넘기는 환경변수**

- Python 모듈/익스텐션: `PYTHONIOENCODING`, `PYTHONUNBUFFERED`, `PYTHONPATH`, `SABA_IPC_PORT`, `SABA_IPC_URL`, `SABA_TOKEN_PATH`, `SABA_LANG`(`config.json`의 `gui.language`, 실행마다 읽음), `SABA_ENABLED_EXTENSIONS`(`plugin/mod.rs:14-37, 187-205`). 토큰 값 자체는 넘기지 않는다. 공용 `modules/daemon_rcon.py`는 `SABA_TOKEN_PATH` 파일을 읽어 `X-Saba-Token`을 붙인다(`modules/daemon_rcon.py:65-71`).
- `SABA_LANG`는 GUI도 데몬에 넘기지만(`saba-chan-gui/main.js:598`) **데몬 코어는 이 환경변수를 읽지 않는다.** 언어는 `config.json`의 `gui.language`에서 읽는다(`plugin/mod.rs:28-35`, `chat/api.rs:119`).

### 1.4 디스코드 릴레이 연결 (`src/relay_agent.rs`)

로컬 Discord 봇 실행 경로(`node_env`, `discord_bot` ext-process)는 L1(7b5102f)에서 지웠다. 디스코드 봇은 클라우드 릴레이에서만 돌고, 데몬은 릴레이 에이전트로 붙는다.

1. IPC 서버가 뜬 뒤 `relay_agent::auto_start`를 띄운다(`ipc/mod.rs:787-789`). 수동 제어는 `/api/relay/agent{,/start,/stop}`.
2. `GET /info`로 최소 에이전트 버전을 확인하고, 60초마다 `POST /heartbeat`로 서버·모듈·봇 설정 메타데이터를 올린다(`relay_agent.rs:22`).
3. `GET /poll`(롱폴링)로 디스코드 명령을 받아 채팅 처리기(`crate::chat`)로 실행하고 `POST /result/:id`로 돌려준다.
4. 인증은 노드 토큰(`<data_dir>/.node_token`) HMAC 서명이다(`relay_agent.rs:1-10`).

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
  python-guard/sitecustomize.py   활성 익스텐션만 import하게 하는 가드    plugin/mod.rs:84-96
  installed-manifest.json, updater-state.json, update-complete.json (updater)
  modules/<module>/module.toml, lifecycle.py, locales/*.json
  modules/i18n.py, daemon_rcon.py   데몬에 내장된 공용 Python 유틸리티 (plugin/mod.rs:98-116)
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
- 잠금은 토큰 생성·Supervisor 초기화보다 먼저 잡는다(`main.rs:240-255`). 그래서 두 번째 데몬은 `.ipc_token`을 덮어쓰기 전에 종료 코드 2로 끝난다.
- TCP 바인드는 2초 간격으로 10회 재시도한다(`ipc/mod.rs:760-811`). `SO_REUSEADDR`는 **Unix에서만** 켠다. Windows의 `SO_REUSEADDR`는 이미 리슨 중인 포트에도 바인드를 허용하므로 켜지 않는다(`ipc/mod.rs:772-775`).
- 잠금 파일을 열 수 없는 환경(읽기 전용 등)에서는 잠금 없이 진행하므로, 그때는 바인드만이 방어선이다.

---

## 2. IPC HTTP API 전체 표

### 2.1 공통 규약

- **라우터 정의 위치**: `src/ipc/mod.rs:640-758` (axum 0.7), 업데이트 라우트는 `src/ipc/updates.rs:98-109`.
- **경로 파라미터 표기**: axum 0.7의 `:param` 문법을 쓴다.
- **인증 적용 범위**: `.layer(auth_middleware)`는 업데이트 라우터를 `.merge()`한 **뒤에** 달린다(`ipc/mod.rs:756-758`).
  - 따라서 `/api/updates/*`를 포함한 모든 라우트에 토큰이 필요하다. **`/health`만 예외**다(A6): 루프백 검사는 거치지만 토큰 없이 답한다. 토큰이 어긋나도 클라이언트가 데몬이 떠 있는지 알 수 있게 하려는 것이다.
  - 미들웨어는 토큰보다 먼저 `Host` 헤더가 루프백(`127.0.0.1`, `localhost`, `::1`)인지 보고, 아니면 본문 없는 403을 돌려준다(`auth.rs:74-101`). DNS 리바인딩 방어다.
- **에러 형식이 통일돼 있지 않다.** 다음 형식들이 섞여 있다.
  - `{"error": "..."}` (+ 선택적으로 `"error_code"`)
  - `{"ok": false, "error": ...}` (updates, modules manifest/install/remove, `/api/config/:kind/*`는 성공 시 ok)
  - `{"success": false, "error", "error_code", ...}` (익스텐션, 백업, 이름 변경 등)
  - 인증 실패: 본문 없는 401, Host 거부: 본문 없는 403
  - 일부 핸들러는 실패해도 HTTP 200을 돌려준다(modules manifest/install/remove, extensions manifest/install).
- **JSON 바디 파싱 실패**: axum 기본 거부 응답(400/415/422, 텍스트 본문)이 나간다. `/api/updates/download`·`apply`만 본문을 직접 읽어 `{ok:false, error, error_code:"invalid_body"}` 400을 준다.
- **식별자 혼용**: 예전 경로는 서버를 **이름**(`/api/server/:name/*`, `/api/provision-progress/:name`)으로, 나머지는 인스턴스 **UUID**(`/api/instance/:id/*`)로 가리킨다. `/api/instance/:id/start|stop|restart`는 id가 없으면 이름으로도 찾는다(`server.rs:573-577, 658-665`).

### 2.2 라우트 표

아래 표의 모든 라우트는 `X-Saba-Token` 헤더가 필요하다.

#### 헬스 / 서버 제어 (handlers/server.rs)

| Method | Path | Handler | 요청 | 응답 |
|---|---|---|---|---|
| GET | `/health` | `server.rs:19` `health_check` | - | `{ok:true, version, pid, token}`. 토큰 없이 부를 수 있다. `token`은 보낸 `X-Saba-Token`이 `valid`·`invalid`·`missing` 중 무엇인지(값은 드러내지 않는다) |
| GET | `/api/diagnostics` | `diagnostics.rs:20` | - | A6, 인증 필요. `{ok, version, pid, uptime_secs, listen_addr, data_dir, token_file:{path, exists}, audit_log, extensions:{enabled, enabled_but_missing}, modules:{count}\|{error}, recent_problems:[최근 경고·오류 20개, 오래된 순]}` |
| GET | `/api/audit` | `diagnostics.rs` `audit` | `?limit=`(기본 100, 최대 1000)`&instance=<id>` | A10 감사 로그, 최신이 먼저: `{entries:[{ts, source, user?, guild?, action, instanceId, instance?, command?, args?, ok, status}]}`. `action`은 `start`·`stop`·`restart`·`command`·`stdin` |
| GET | `/api/version` | `server.rs:279` | - | `{version, components:{saba-core, module-<이름>, ext-<id>, ...}}` |
| GET | `/api/servers` | `server.rs:24` `list_servers` | - | `{servers:[ServerInfo], port_conflict_stops?:[...]}` (아래 설명) |
| GET | `/api/server/:name/status` | `server.rs:534` | - | supervisor 결과 JSON 그대로. 404 시 `{error, error_code:"instance_not_found"}` |
| POST | `/api/server/:name/start` | `server.rs:762` | `{module: string (필수), config?: any}` | supervisor 결과. 에러 시 404/422/500 + `error_code`∈{instance_not_found, executable_missing, start_failed} |
| POST | `/api/server/:name/stop` | `server.rs:818` | `{force?: bool}` (바디 필수) | supervisor 결과. 에러 시 `error_code`∈{instance_not_found, stop_failed} |
| POST | `/api/instance/:id/start` | `server.rs:568` | - | 데몬이 고른 경로(managed/non-managed, `Supervisor::uses_managed_start`)의 응답 + `managed`. 404 `not_found`, 409 `preflight_failed`(모듈 validate의 critical 문제) |
| POST | `/api/instance/:id/stop` | `server.rs:676` | 선택 `{force?}` | `/api/server/:name/stop`과 같다 |
| POST | `/api/instance/:id/restart` | `server.rs:687` | 선택 `{force?}` | 정지(최대 2분 대기) 뒤 시작 응답 + `stopped` |
| GET | `/api/instance/:id/events` | `server.rs:753` | - | `{events:[...]}` 최근 종료 이력 / 404 |

`GET /api/servers` 응답의 `ServerInfo` 필드(`ipc/mod.rs:183-220`):

- 기본: `id, name, module, status, pid, start_time, executable_path, working_dir?, port`
- 접속 정보: `rest_host, rest_port, rest_username, rest_password, protocol_mode`
- 설정과 상태: `module_settings, server_version, last_api_action?, provisioning, managed, extension_status?, extension_data, port_conflicts?`
- `managed`는 saba-runner가 맡아 실행하는지다. 판정은 데몬의 `Supervisor::uses_managed_start` 하나이고(`server.rs:81`), 클라이언트는 이 값을 그대로 쓴다.
- **REST 비밀번호가 평문으로 포함된다.**

`port_conflict_stops`는 한 번 읽으면 비워지는 drain 큐다(`server.rs:250-254`).

`status` 값은 다음 중 하나다.

- `running` / `stopped`: PID 기반
- 익스텐션 `server.list_enrich` 훅이 `handled:true`를 돌려준 경우: 그 훅이 준 값
- `provisioning`: 프로비저닝 진행 중

#### 모듈 (handlers/server.rs, handlers/managed.rs, handlers/config.rs)

| Method | Path | Handler | 요청 | 응답 |
|---|---|---|---|---|
| GET | `/api/modules` | `server.rs:348` | - | `{modules:[ExtensionInfo], failed?:[...]}` (아래 설명) |
| POST | `/api/modules/refresh` | `server.rs:355` | - | 위와 같음 |
| GET | `/api/modules/schema` | `server.rs:366` | - | module.toml JSON Schema |
| POST | `/api/modules/detect` | `server.rs:372` | `{path}` | `{matches:[{name, display_name}]}` (`dir_signatures`가 맞는 모듈) / 400 / 404 |
| GET | `/api/modules/manifest` | `server.rs:429` | - | `{ok, manifest}` 또는 `{ok:false,error}`. 항상 200 |
| POST | `/api/modules/manifest/:id/install` | `server.rs:452` | - | `{ok, module_id, version}` 또는 `{ok:false,error}`. 항상 200 |
| DELETE | `/api/modules/:id` | `server.rs:508` | - | `{ok, id}`. 항상 200 |
| GET | `/api/module/:name` | `server.rs:394` | - | `{name, version, description, path, metadata}` / 404 |
| GET | `/api/module/:name/versions` | `managed.rs:242` | query `include_snapshots, page=1, per_page=25` | supervisor 결과 |
| GET | `/api/module/:name/version/:version` | `managed.rs:271` | - | supervisor 결과 |
| POST | `/api/module/:name/install` | `managed.rs:342` | `{version*, install_dir*, jar_name?, accept_eula?, initial_settings?}` | 200(success) / 422 / 404 / 400 / 500 |
| GET | `/api/module/:name/locales` | `config.rs:193` | - | `{<lang>: <json>}` |

- `ExtensionInfo`(이름과 달리 모듈 정보다, `ipc/mod.rs:248-287`) 필드: `name, version, description, schema_version, display_name?, game_name?, author?, warnings?, unmet_dependencies?, path, executable_path, icon(base64 data URL), interaction_mode, protocols{supported,default}, settings, commands, syntax_highlight, dir_signatures?, install?, module_aliases?, command_aliases?`
- `/api/modules/manifest`는 최신 릴리즈 서명 manifest의 `module-*` 컴포넌트를 `{ok, manifest:{modules:{<이름>:{version, sha256, size_bytes, display_name, ...}}}}`로 돌려준다(`relay::latest_catalog`, `handlers/server.rs:426-449`).
- `/api/modules/manifest/:id/install`은 같은 manifest의 `module-<id>`를 R2에서 받아 SHA-256을 확인하고(`download_verified`), staging에 푼 뒤 `module.toml`이 있을 때만 `modules/<id>`와 바꾸고 refresh한다(`:451-506`). `id`의 `..`, `/`, `\`는 거부한다.

#### 인스턴스 (handlers/instance.rs)

| Method | Path | Handler | 요청 | 응답 |
|---|---|---|---|---|
| GET | `/api/instances` | `instance.rs:12` | - | `[ServerInstance]` (§4.1, REST 비밀번호 평문 포함) |
| POST | `/api/instances` | `instance.rs:71` | `{name*, module_name*, executable_path?, working_dir?, migration_source?, port?}` | 201 `{success, id, provisioning?}`. 409 `working_dir_in_use`, 422 `extension_required`, 400/500 |
| PUT | `/api/instances/reorder` | `instance.rs:19` | `{order:[id]}` | `{success:true}` / 400 |
| GET | `/api/instance/:id` | `instance.rs:49` | - | `ServerInstance` / 404 `{error, error_code}` |
| DELETE | `/api/instance/:id` | `instance.rs:635` | - | `{success}` / 404 / 500 |
| PATCH | `/api/instance/:id` | `instance.rs` `update_instance_settings` | 임의 키-값 객체. `extra_args`(문자열은 `chat::split_quoted`로 나눔, 또는 문자열 배열, null은 비우기)는 인스턴스의 추가 실행 인자로 | `{success:true}` / 400 `validation_failed`·`invalid_format`(모듈 `format`·`pattern`과 다름)`{details, fields}` / 404 / 500 |
| POST | `/api/instance/:id/launch-preview` | `instance.rs` `launch_preview` → `Supervisor::launch_preview` | `{settings?:{필드:값}, extra_args?: string\|string[]}` (extra_args가 없으면 저장된 것) | 실행 인자 미리보기. 서버를 띄우지 않고 익스텐션 hook도 부르지 않는다. 저장된 설정 위에 초안, 그 위에 추가 인자를 필드 `arg` 모양으로 읽은 값을 얹어 `get_launch_command`(`preview:true`)를 부른다. 성공 `{success:true, has_launch_args, command_line, program, args[], settings_from_args{}, extra_args[], extra_args_text, errors[{arg\|null, field, expected, got}]}`. password 필드 값은 `***`. 모듈 실패 `{success:false, error:"launch_command_failed"\|action_required, message?, ...같은 키}` / 404 / 500 `preview_failed`. `expected`는 `validator::expected_of` 코드(`memory_size`·`port`·`integer`·`path`·`pattern`·`number`·`boolean`·`enum`·`text`) |
| POST | `/api/instance/:id/rename` | `instance.rs:724` | `{name}` | `{success, id, name, previous}` / 409 `name_taken`·`server_running` / 404 |
| GET | `/api/provision-progress/:name` | `instance.rs:606` | - | `{active:false}` 또는 `{active:true, step,total,label,message,done,error,percent?,steps?}` |
| DELETE | `/api/provision-progress/:name` | `instance.rs:565` | - | `{success, rolled_back}`. **신규 생성**의 프로비저닝이 실패한 상태였을 때만 인스턴스를 삭제한다 |
| GET | `/api/instance/:id/check-update` | `instance.rs:1077` | - | 훅 결과 또는 `{update_available:false, reason}` |
| POST | `/api/instance/:id/apply-update` | `instance.rs:1170` | - | `{success,id,provisioning:true}`. 409 `server_running`, 400 `no_install_config`/`update_unsupported`(설치 방식 익스텐션이 설치한 인스턴스가 아님) |

`POST /api/instances` 동작 세부:

- 모듈의 `install.requires_extensions`가 활성·설치돼 있지 않으면 422 `extension_required`다(`instance.rs:141-180`).
- 다른 인스턴스가 같은 `working_dir`를 쓰면 409다(`instance.rs:200-213`). 포트를 주지 않으면 다른 인스턴스와 겹치지 않게 기본 포트를 옮긴다(`:217-234`).
- `migration_source`가 있으면 데몬 락을 놓은 뒤 `import_settings`를 **동기로** 호출한다(`instance.rs:523-547`).
- 인스턴스를 저장한 직후 `server.post_create` 훅을 백그라운드로 보내고 결과를 기다리지 않는다(`instance.rs:244-257`). ctx는 `{instance_id, instance_dir, instance, module, migration}`이다.
- 모듈에 `install` 설정이 있으면 `lifecycle.py install_server`를 백그라운드로 실행한다(타임아웃 30분).

`PATCH /api/instance/:id` 동작 세부:

- 알려진 키(`port, rest_*, executable_path, protocol_mode, server_version, extension_data`)는 일급 필드로 반영한다. 예전 RCON 키(`rcon_port, rcon_password, enable_rcon`)는 받아도 어디에도 반영하지 않는다(`instance.rs:822-836`).
- 나머지 키는 전부 `module_settings`로 들어간다.
- 이후 락을 놓고 `manage_properties("write")`로 게임 설정 파일과 동기화하고, `extension_data`가 비어 있지 않으면 `server.settings_changed` 훅을 보낸다.

#### 명령 실행 (handlers/command.rs)

| Method | Path | Handler | 요청 | 응답 |
|---|---|---|---|---|
| POST | `/api/instance/:id/command` | `command.rs:16` | `{command*, args?}` | 실행한 경로의 응답 + `command`. 실패 시 `error_code`, 모듈 `[errors]` 문구 `error_message`. 404 / 400 / 409 `server_not_running` |
| GET | `/api/instance/:id/commands` | `command.rs:172` | - | `{commands:[{name,label,description,risk,inputs,source:"module"\|"extension",...}]}` |
| POST | `/api/instance/:id/rcon` | `command.rs:263` | `{command*}` | 이름만 남은 호환 경로. 언제나 stdin으로 보낸다: `{success, message, data:{response,executed}\|null, protocol:"stdin"}` / 404 / 409 `no_command_path`(콘솔 없음) |
| POST | `/api/instance/:id/rest` | `command.rs:358` | `{endpoint*, method?, body?, timeout?, rest_host?, rest_port?, use_https?}` | `{success, data, error, endpoint, method, host, port, protocol:"rest"}` / 400 / 500 |

#### Managed 프로세스와 모듈 기능 (handlers/managed.rs)

| Method | Path | Handler | 요청 | 응답 |
|---|---|---|---|---|
| POST | `/api/instance/:id/managed/start` | `managed.rs:14` | 선택 `{config?}` | supervisor 결과 / 404 / 500 |
| GET | `/api/instance/:id/console` | `managed.rs:69` | query `since, count` | supervisor 결과 (`lines[{id,content,level}]`) / 404 |
| POST | `/api/instance/:id/stdin` | `managed.rs:90` | `{command*}` | `/rcon`과 같은 응답(`via_stdin`) / 400 / 500 |
| POST | `/api/instance/:id/validate` | `managed.rs:100` | - | supervisor 결과 |
| GET | `/api/instance/:id/properties` | `managed.rs:115` | - | `manage_properties("read")` |
| PUT | `/api/instance/:id/properties` | `managed.rs:130` | `{settings}` | `manage_properties("write")` |
| GET | `/api/instance/:id/settings/live` | `managed.rs:151` | - | `manage_properties("import")`. 성공하면 인스턴스 필드도 파일 값으로 맞춘다 |
| POST | `/api/instance/:id/properties/reset` | `managed.rs:179` | - | `"reset"` |
| POST | `/api/instance/:id/server/reset` | `managed.rs:194` | - | `"reset_server"` (월드, 설정 삭제) |
| POST | `/api/instance/:id/accept-eula` | `managed.rs:209` | - | supervisor 결과 |
| POST | `/api/instance/:id/diagnose` | `managed.rs:224` | - | supervisor 결과 |
| GET | `/api/instance/:id/installed-version` | `managed.rs:287` | - | 결과. 감지에 성공하면 `server_version`에 자동 저장 |

#### 백업과 예약 작업 (handlers/backup.rs)

| Method | Path | Handler | 요청 | 응답 |
|---|---|---|---|---|
| GET/POST | `/api/instance/:id/backups` | `backup.rs:183/190` | - | `{success, backups}` / `{success, backup}`. 400 `nothing_to_back_up`·`backup_unsupported`(모듈에 `[backup]` 없음), 500 `backup_failed`(`backup_paths` 실패) |
| DELETE | `/api/instance/:id/backups/:name` | `backup.rs:198` | - | `{success}` / 404 `backup_not_found` |
| POST | `/api/instance/:id/backups/:name/restore` | `backup.rs:209` | - | `{success, restored, pre_restore_backup}`. 409 `server_running`·`backup_busy`. 복원할 백업은 안전 백업 뒤 정리에서 뺀다(`backup.rs:221-222`) |
| GET/PUT | `/api/instance/:id/schedule` | `backup.rs:242/258` | `{schedule:[...], backup_keep}` | 400 `validation_failed` |

#### 설정 (handlers/bot.rs, handlers/config.rs)

| Method | Path | Handler | 요청 | 응답 |
|---|---|---|---|---|
| GET | `/api/config/bot` | `bot.rs:12` | - | BotConfig JSON (`config.json`의 `relay`) |
| PUT | `/api/config/bot` | `bot.rs:18` | 부분 JSON (얕은 머지) | `{success, message}` / 500 `{error}` |
| GET | `/api/config/gui` | `config.rs:16` | - | GuiSettings JSON |
| PUT | `/api/config/gui` | `config.rs:22` | 부분 JSON | `{ok:true}` / 400 / 500. 저장에 성공하면 `supervisor.skip_port_check`도 갱신 |
| GET | `/api/config/:kind/defaults` | `config.rs:48` | - | `{ok, defaults}` (`gui`, `bot`) / 404 |
| POST | `/api/config/:kind/reset` | `config.rs:60` | 선택 `{keys?}` | `{ok, config}` / 400 |
| GET/PUT | `/api/config/client/:kind` | `config.rs:107/118` | 부분 객체(최상위 키 머지) | 그 네임스페이스 JSON (예: `cli`). 예약 이름은 400 |
| GET | `/api/config/node-token` | `config.rs:135` | - | `{token}` (없으면 `""`) |
| PUT | `/api/config/node-token` | `config.rs:145` | `{token}` | `{ok}` |
| DELETE | `/api/config/node-token` | `config.rs:173` | - | `{ok}`. 먼저 릴레이 에이전트를 멈춘다 |
| POST | `/api/migration/export` | `migration.rs:173` | `{path(절대 경로), secrets?}` | `{success, path, secrets, instances:[{id,name}], config:[네임스페이스]}` / 400 `INVALID_PATH` / 500 `WRITE_FAILED`. A9: 데몬이 `path`에 zip(`saba-export.json`, `config.json`, `instances.json`, `secrets`면 `node_token`)을 쓴다. `updates`·`boot`·`extensions`·`schemaVersion`은 이 PC 것이라 뺀다. `secrets`가 아니면 챗봇 API 키, REST 비밀번호, 모듈이 `password`로 선언한 설정을 뺀다. 게임 서버 파일은 넣지 않는다 |
| POST | `/api/migration/import` | `migration.rs:246` | `{path, dryRun?, overwrite?}` | `{success, dryRun, appVersion, secrets, instances:{added, replaced, skipped:[{id,name,reason}]}, missingModules, config:[네임스페이스], nodeToken:"imported"\|"kept"\|null}` / 400 `INVALID_PATH`·`INVALID_ARCHIVE`·`INVALID_CONFIG`. 인스턴스는 같은 id가 있으면 `exists`(`overwrite`면 덮어씀, 켜져 있으면 `running`), 다른 인스턴스와 폴더가 같으면 `working_dir_in_use`로 건너뛴다. 설정은 지금 값 위에 최상위 키 머지(파일에 없는 API 키는 남는다). 설정은 다른 것을 바꾸기 전에 먼저 검사한다(`check_sections`, 잘못되면 아무것도 바꾸지 않고 `INVALID_CONFIG`). 노드 토큰은 이 PC에 없을 때만 쓰고, 토큰을 실제로 가져올 때만 `relay.cloud`(relayUrl·hostId)·`cloudNodes`·`cloudMembers`도 바꾼다(아니면 이 PC 값 유지). `added`·`replaced` 항목에는 `working_dir`·`executable_path`가 붙는다 |
| GET | `/api/system/components` | `config.rs:233` | - | `{version, components:{key:ver}, lastUpdated}` (`installed-manifest.json` + 번들 `versions.toml`) |
| GET | `/api/system/metrics` | `system.rs:30` | `?sizes=false` | 시스템 모니터 표본 `{sampled_at_ms, window_ms, cpu:{cores,total_percent,others_percent}, memory:{total_bytes,used_bytes,others_bytes}, disk_io:{read_bps,write_bps,source:"physical"|"process",per_instance:false}, network:{rx_bps,tx_bps,per_instance:false}, disks:[{mount,total_bytes,available_bytes,others_bytes,instances:[{id,bytes}],sizes_at_ms}], instances:[{id,name,running,process_count,cpu_percent,memory_bytes,disk_read_bps,disk_write_bps}]}` (§10) |
| GET | `/api/runtime/preflight` | `ipc/mod.rs:696` | - | `{phase, percent, complete, error, components:{python}}` |

#### 클라이언트와 데몬 수명 (handlers/client.rs)

| Method | Path | Handler | 요청 | 응답 |
|---|---|---|---|---|
| POST | `/api/client/register` | `client.rs:21` | `{kind:"gui"\|"cli", pid?, installRoot?}` | `{client_id, kind, heartbeat_interval_ms:30000, timeout_ms:90000}` |
| GET | `/api/clients` | `client.rs` `client_list` | - | `{clients:[{id, kind, pid, installRoot}]}`. 인스톨러·제거기가 데몬을 끄기 전에 이 설치(`installRoot`)의 GUI PID를 받아, 임시 폴더에 풀린 포터블 GUI까지 끈다 |
| POST | `/api/client/:id/heartbeat` | `client.rs:51` | 본문은 무시 | `{ok}` / 404 |
| DELETE | `/api/client/:id/unregister` | `client.rs:69` | query `shutdown=true\|false` | `{ok}` / 404 |
| POST | `/api/daemon/shutdown` | `client.rs:12` | - | `{ok, message}`. 200ms 뒤 `stop_requested` cancel |
| GET | `/api/daemon/console` | `client.rs:120` | `since, count=200` | `{lines:[{id,timestamp,level,target,message}], count}` |

`unregister`에 `shutdown=true`를 주고 남은 클라이언트가 0이면 데몬도 종료한다(`client.rs:80-84`). 둘 다 `request_shutdown`(`ipc/mod.rs:621-627`)을 거쳐 §1.1의 종료 경로로 간다.

#### 익스텐션 (handlers/extension.rs; 구현은 `extension/mod.rs`)

| Method | Path | Handler | 비고 |
|---|---|---|---|
| GET | `/api/extensions` | `extension.rs:67` | `{extensions:[...]}`. 설명은 `?lang=`(없으면 GUI 언어) |
| GET | `/api/extensions/init-status` | `extension.rs:452` | `{initializing, in_progress:{id:msg}, completed:[{ext_id,success,message,timestamp}]}` |
| POST | `/api/extensions/rescan` | `extension.rs:170` | `{success, newly_found}` |
| GET | `/api/extensions/manifest` | `extension.rs:313` | `{success, extensions, updates}` (원격) |
| GET | `/api/extensions/updates` | `extension.rs:410` | `{success, updates, count}` |
| POST | `/api/extensions/:id/enable` | `extension.rs:96` | `{success,id,startup_dispatched}`. `daemon.startup` 훅을 비동기 실행 |
| POST | `/api/extensions/:id/disable` | `extension.rs:150` | 실행 중인 managed 인스턴스 참조 검사 |
| POST | `/api/extensions/:id/mount` | `extension.rs:187` | |
| POST | `/api/extensions/:id/unmount` | `extension.rs:199` | |
| POST | `/api/extensions/:id/install` | `extension.rs:352` | 선택 `{download_url, sha256}`. 없으면 manifest에서 찾는다. 빠진 의존성도 함께 설치, `{success, id, installed}` |
| DELETE | `/api/extensions/:id` | `extension.rs:432` | |
| GET/PUT | `/api/extensions/:id/config` | `extension.rs:491/504` | `{config}` / 객체 머지(null이면 키 삭제) |
| GET | `/api/extensions/:id/gui` | `extension.rs:228` | UMD JS 번들 (application/javascript) |
| GET | `/api/extensions/:id/gui/styles` | `extension.rs:260` | text/css |
| GET | `/api/extensions/:id/icon` | `extension.rs:459` | icon.png |
| GET | `/api/extensions/:id/i18n/:locale` | `extension.rs:289` | JSON |

- 익스텐션 에러 형식: `{success:false, error, error_code, related}`.
- `error_code`별 상태 코드: `not_found|not_mounted|manifest_not_found`는 404, 의존성·버전 계열은 422, `has_dependents|in_use`는 409, `id_mismatch`는 400(`extension.rs:25-58`).
- `/gui`, `/gui/styles`, `/icon` 라우트가 바로 **플러그인이 인터페이스에 컴포넌트를 주입하는 통로**다. GUI가 이 번들을 받아 렌더한다.

#### Relay 프록시, 에이전트, 채팅

| Method | Path | Handler | 비고 |
|---|---|---|---|
| GET | `/api/relay/host/:host_id/status` | `relay.rs:211` | query `relayUrl?`. relay `/api/hosts/:id` 프록시 (HMAC 서명). 서명 요청의 `relayUrl`은 설정한 릴레이와 같거나 루프백일 때만 따른다(`signed_relay_url`, `relay.rs:45-67`) |
| GET | `/api/relay/host/:host_id/nodes` | `relay.rs:223` | `/api/hosts/:id/nodes` |
| GET/PUT | `/api/relay/node/:guild_id/channels` | `relay.rs:237/246` | `/api/nodes/:id/channels` 서명 프록시. 챗봇 채널(여럿)·음악 채널(하나) 배정의 원본은 릴레이다. 응답 `{guildId, chatbot:[id], music:id\|null, botAvailable, channels:[{id,name,category}]}`, PUT 본문 `{chatbot?, music?}`(준 키만). 오류 본문 `{error: CODE, channelId?}`는 그대로 전달(객체가 아닌 본문은 데몬이 400 `INVALID_BODY`). 데몬 config의 `chatbot.guilds.*.channelIds`는 `ChatbotConfig::migrate`가 지운다 |
| GET | `/api/relay/node/:guild_id/members` | `relay.rs:236` | `/api/nodes/:id/discord-members`, 503이면 `/members`로 폴백 |
| GET | `/api/relay/bot-invite` | `relay.rs:267-286` | query `relayUrl?`. relay `/api/bot-invite`(공개, 서명 없음)를 그대로 전달. 주소는 서명 요청과 같은 규칙(설정한 릴레이 또는 루프백)으로 정한다. 봇 초대 URL `{url}` |
| POST | `/api/relay/pair/initiate` | `relay.rs:288` | 바디를 그대로 `/api/pair/initiate`에 전달 (서명 없음) |
| GET | `/api/relay/pair/:code/status` | `relay.rs:321` | `secret?, relayUrl?`. `status:"claimed"`이면 `nodeToken`을 `.node_token`에 자동 저장 |
| GET | `/api/relay/agent` | `chat.rs:16` | 에이전트 상태 `{running, sleeping, relayUrl, hasToken, ...}` |
| POST | `/api/relay/agent/start`, `/stop` | `chat.rs:21/33` | 토큰이 없으면 409 `node_token_missing` |
| POST | `/api/chat/command` | `chat.rs:10` | `{text, guild_id?, user_id?}` → `{reply, code, args}`. `code`는 입력을 알아보지 못했을 때만(`UNKNOWN_COMMAND`·`INSTANCE_NOT_FOUND`), 아니면 null |
| GET/PUT/DELETE | `/api/chatbot` | `chatbot.rs:70/245/267` | AI 챗봇 설정 (`config.json`의 `chatbot`). 바뀔 때마다 릴레이에 동기화, 클라이언트에는 `hasApiKey`·`pendingRelayDelete`만. PUT은 릴레이 기본값 위에 전체 설정을 얹어 보낸다(`chatbot.rs:33-59`). DELETE가 릴레이에 닿지 못하면 `pendingRelayDelete:true`로 남기고 다음 연결 때 다시 지운다(`:199-243`) |
| DELETE | `/api/chatbot/memory` | `chatbot.rs:283` | query `guildId?` |

Relay 서명 방식(`relay.rs:100-137`):

- 헤더: `Authorization: Bearer <node_token>`, `x-request-timestamp`, `x-request-nonce`, `x-request-signature`.
- 서명값: `HMAC-SHA256(secret, "METHOD\npath\nts\nnonce\nbody")`.
- 고정 헤더 `X-Saba-Client: 1`이 모든 relay 요청에 붙는다. timeout은 5초다(`relay.rs:69-81`).

#### 업데이트 (ipc/updates.rs)

| Method | Path | Handler | 요청 | 응답 |
|---|---|---|---|---|
| GET | `/api/updates/status` | `updates.rs:121` | - | `{ok,last_check,next_check,checking,error,updates_available,components:[{component,display_name,current_version,latest_version,update_available,downloaded,installed}]}` (Locales 제외) |
| POST | `/api/updates/check` | `updates.rs:161` | - | `{ok,updates_available,update_names,components(+release_notes),last_check}`. Locales 업데이트가 있으면 백그라운드에서 조용히 적용 |
| POST | `/api/updates/download` | `updates.rs:338` | 선택 `{components?:[key]}`. 본문이 없거나 공백이면 전부, 깨진 JSON은 400 `invalid_body`(`updates.rs:320-329`) | `{ok,downloaded,count,errors?}` |
| GET | `/api/updates/download/progress` | `updates.rs:422` | - | `{ok,component,bytes_received,total_bytes,active}` |
| POST | `/api/updates/apply` | `updates.rs:447` | download와 같은 본문 규칙 | `{ok,applied,needs_updater,requires_updater,errors}` |
| POST | `/api/updates/launch-updater` | `updates.rs:581` | 선택 `{relaunch:[exe, args...]}` | `{ok, updater_launched, daemon_stopping}`. 404(업데이터 없음) / 409(받은 업데이트 없음) |
| GET/DELETE | `/api/updates/last-result` | `updates.rs` `get_last_result`/`clear_last_result` | GET `?lang=` | 마지막 적용 결과 `{result, version, notes}` / 확인 처리 |
| GET | `/api/updates/integrity` | `updates.rs:697` | - | `{ok,checked_at,overall,total,verified,failed,skipped,components[...]}` |
| GET | `/api/updates/config` | `updates.rs:727` | - | `{ok, config, mock_relay}` |
| PUT | `/api/updates/config` | `updates.rs:742` | 부분 `{enabled,check_interval_hours(1-8),auto_download,install_root,relay_url,channel}` | `{ok, config}`. `config.json`의 `updates` 절에 저장한다(`:770-772, 800-806`). 비어 있지 않은 `relay_url`은 개발 빌드(`mock-relay`)만 받고, 릴리즈 빌드는 403 `relay_url_not_settable`(D7: 출처는 config.json에서만) |

---

## 3. 인증 모델 (`src/ipc/auth.rs`)

- **토큰 생성**: 데몬이 기동할 때마다 `uuid::Uuid::new_v4().to_string()`(36자)를 새로 만든다(`auth.rs:28`).
- **저장**
  - `constants::token_file_path()`, 기본값은 `<data_dir>/.ipc_token`이고 `SABA_TOKEN_PATH`로 바꿀 수 있다.
  - Unix에서는 퍼미션을 0600으로 둔다.
  - 프로세스 전역 `static CACHED_TOKEN: RwLock<Option<String>>`에도 캐시한다(`auth.rs:17`).
- **Host 검사**: 토큰 검사보다 먼저, `Host` 헤더가 루프백이 아니면 403을 돌려준다. 헤더가 없으면 통과시킨다(`auth.rs:74-101`).
- **검증**: 헤더 `X-Saba-Token` 값과 정확히 일치하는지 비교한다(`auth.rs:134`).
  - 상수 시간 비교다(`constant_time_eq`, `auth.rs:86-89`). 길이 차이는 숨기지 않는다(UUID 고정 길이).
  - 불일치하면 본문 없이 401을 돌려준다.
  - 실패 로그는 30초에 한 번으로 제한하고, 토큰 앞 8자를 로그에 남긴다.
- **우회 경로**
  - `SABA_AUTH_DISABLED=1`이면 토큰 검사를 건너뛴다. **디버그 빌드에서만** 동작하며(`auth.rs:103-106`), 통합 테스트가 이 방식을 쓴다(`tests/daemon_integration.rs:111`).
  - 캐시도 없고 토큰 파일도 없으면 **거부한다**(fail-closed).
  - `/health`는 루프백 검사 뒤 토큰 검사 없이 통과한다(A6).
- **토큰을 쓰는 클라이언트**: GUI(`saba-chan-gui/main.js:69,180`, 파일 재읽기 포함), CLI(`saba-chan-cli/src/client.rs:28-33`, 401이면 파일을 다시 읽고 한 번 재시도 `:97-103`. PR #6 기준), Python 모듈(`modules/daemon_rcon.py:65-71`, `SABA_TOKEN_PATH`). 모두 토큰 파일을 직접 읽어 헤더에 넣는다.
- **CORS 레이어가 없다.** 브라우저에서 오는 교차 출처 요청은 사전 요청(preflight)에서 막히고, 단순 요청도 토큰이 없으면 401이다. 모든 라우트가 인증을 거치므로 바디 없는 POST를 CSRF로 부를 수 있던 예전 `/api/updates/*` 경로도 막혔다.

---

## 4. 인스턴스 모델

### 4.1 `ServerInstance` (`src/instance/mod.rs:24-62`)

| 필드 | 타입 | 비고 |
|---|---|---|
| `id` | String | uuid v4 |
| `name` | String | 생성 시 유일성 검사는 없고, 이름 변경(`/rename`)만 중복을 막는다. `/api/server/:name/*`는 첫 번째로 일치하는 인스턴스를 쓴다 |
| `module_name` | String | |
| `executable_path`, `working_dir` | Option<String> | |
| `auto_detect` | bool | 기본 true |
| `process_name` | Option<String> | 모듈 metadata에서 채운다 |
| `port` | Option<u16> | |
| `rest_host`, `rest_port`, `rest_username`, `rest_password` | Option | `rest_username`은 항상 "admin"으로 강제한다(`instance/mod.rs:198-203`) |
| `protocol_mode` | String | 기본 `"auto"` |
| `module_settings` | HashMap<String,Value> | settings.json으로 분리 저장 |
| `server_version` | Option<String> | |
| `extension_data` | HashMap<String,Value> | 설치 방식 익스텐션 키(`install_method_<방식>`, `<방식>_<이름>`, `instance/mod.rs:124-148`) 등 |
| `required_extensions` | Vec<String> | 모듈의 `install.requires_extensions`를 복사한다 |
| `schedule` | Vec<ScheduledTask> | 예약 작업 (백업·재시작·공지·업데이트) |
| `backup_keep` | Option<u32> | 자동 백업 보관 개수 (기본 10) |

- RCON 필드(`rcon_port`, `rcon_password`)는 없다. 예전 `instance.json`·`settings.json`에 남은 RCON 키는 로드할 때 한 번 지우고 다시 쓴다(`instance/mod.rs:419-448, 463-480`).
- `ensure_passwords()`(`instance/mod.rs:190`)는 시작할 때(`server.rs:784`, `managed.rs:23`) 비어 있는 REST 비밀번호를 16자 영숫자로 채운다.
- 비밀번호 생성은 `constants::generate_random_password`가 uuid v4 바이트에 `% 62`를 적용하는 방식이다(`constants.rs:188-201`). 모듈로 편향이 있고, 두 번째 uuid는 계산만 하고 쓰지 않는다.
- `GuiSettings.autoGeneratePasswords`는 이 동작에 **반영되지 않는다.**

### 4.2 영속화 (`InstanceStore`, `instance/mod.rs:218-457`)

- 루트는 `constants::resolve_instances_dir()`이며, 기본값은 `<data_dir>/instances`다.
- 저장 형식(JSON, pretty):
  - `<root>/order.json`: id 배열
  - `<root>/<id>/instance.json`: `module_settings`를 뺀 메타데이터
  - `<root>/<id>/settings.json`: `module_settings`
  - Unix에서는 두 파일 모두 0600으로 둔다.
- `instance.json`·`settings.json`은 임시 파일에 쓰고 rename하는 원자적 쓰기다(`write_atomic`, `instance/mod.rs:482-493`). `order.json`은 그냥 덮어쓴다(`:452-456`).
- 쓰거나 지울 폴더는 id가 경로 한 칸이고 심볼릭 링크가 아니어야 한다(`checked_dir`, `:379-389`).
- 로드할 때 `order.json` 순서로 정렬하고, 목록에 없는 인스턴스는 뒤에 붙인다.
- `remove()`는 `<root>/<id>` 디렉토리를 통째로 지운다(`:323-334`). 기본 설치 위치인 `server/` 하위 게임 서버 파일과 `backups/`도 **함께 지워진다.**

### 4.3 생명주기

1. **생성**: `POST /api/instances`. 두 갈래로 나뉜다.
   - 네이티브 설치: `download` 방식은 `lifecycle.py list_versions`, 이어서 `install_server`. 그 밖의 방식은 같은 이름의 설치 방식 익스텐션이 `extension_data`를 보고 맡는다.
   - 마이그레이션: `import_settings`
2. **프로비저닝 추적**: `ProvisionTracker`가 **인스턴스 이름**을 키로 쓴다(`ipc/mod.rs:379-428`).
   - 성공하면 5초 뒤 자동으로 지운다.
   - 실패하면 사용자가 dismiss할 때까지 남겨 두고, dismiss하면 **신규 생성이었던 경우만**(`mark_creating`/`take_creating`) 인스턴스를 롤백(삭제)한다.
3. **시작**: 클라이언트는 `/api/instance/:id/start`를 쓴다. 데몬이 `uses_managed_start`(`supervisor/mod.rs:1479-1488`, 기본 managed)로 경로를 고르고, 모듈 validate의 critical 문제가 있으면 막는다.
   - `/api/server/:name/start`: native/supervisor `start_server`
   - `/api/instance/:id/managed/start`: stdio를 캡처하는 managed 방식
4. **모니터링**: 2초 루프(`main.rs:351`)가 PID와 패턴을 매칭해 auto-detect하고, 크래시를 감지하면 자동 재시작을 예약한다. 세부는 supervisor 담당 문서에 있다.
5. **정지**: `/api/instance/:id/stop` 또는 `/api/server/:name/stop`(`stop_instance`, `server.rs:838`). 성공하면 tracker에서 id로 untrack한다(`server.rs:874`).
6. **삭제**: 실행 중이면 force kill(2초 대기), `server.pre_delete` 훅, 디렉토리 삭제(최대 3회 재시도) 순으로 진행한다(`instance.rs:635-721`).

### 4.4 config_store와 전역 설정 스키마 (`src/config_store/mod.rs`)

- 모든 사용자 설정은 `<data_dir>/config.json` 하나에 네임스페이스별로 들어간다(`updater/src/config_file.rs:1-27`). 저장은 임시 파일 → fsync → 직전 파일을 `config.json.bak`으로 복사 → rename이고, 깨진 파일은 `.bak`에서 복구한다. 예전 개별 파일(`settings.json`, `bot-config.json`, `cli-settings.json`, `update-config.json`, `boot-config.json`, `extensions_state.json`, `extensionConfig.json`)은 해당 키가 없을 때 한 번 이관하고, 원본은 그대로 두며 `.migrated` 사본을 남긴다(`config_file.rs:37-45, 155-182`).
- `ConfigStore`는 그중 `gui`와 `relay`(봇 설정) 네임스페이스를 메모리에 캐시하고 쓸 때 `config.json`에 flush한다(`config_store/mod.rs:193-288`). 기동할 때 한 번 로드한다(`ipc/mod.rs:594-596`). `chatbot`은 캐시 없이 읽고 쓰며, 그 밖의 클라이언트 네임스페이스(`cli` 등)는 JSON 그대로 최상위 키 머지한다(`:352-387`).
- PUT 동작: 현재 값을 JSON으로 바꾼 뒤 요청값을 **얕게** 머지하고(`merge_json`, `:418`), 강타입으로 역직렬화해 저장한다.
- 스키마에 없는 키는 **저장할 때 버려진다.** 예를 들어 레거시 `discordToken`이나 GUI가 임의로 넣은 키가 그렇다.

**GuiSettings** (camelCase, `config_store/mod.rs:26-56`)

| 그룹 | 필드 (기본값) |
|---|---|
| 동작 | `autoRefresh`(true), `refreshInterval`(2000), `ipcPort`(57474), `consoleBufferSize`(2000), `autoGeneratePasswords`(true), `portConflictCheck`(true) |
| 테마 | `theme?`, `accentColor`("#667eea"), `accentSecondary`("#764ba2"), `useGradient`(true), `fontScale`(100), `enableTransitions`(true), `consoleSyntaxHighlight`(true), `consoleBgColor`, `consoleTextColor`, `sidebarCompact`(false), `windowOpacity`(100), `consoleFontScale`(100), `systemMonitorOpen`(true, 서버 페이지 시스템 모니터 펼침) |
| 창/언어 | `windowBounds?{width:1200,height:840,x?,y?}`, `language?` |

**BotConfig** (camelCase, `config.json`의 `relay`, `config_store/mod.rs:115-125`). 로컬 봇용 필드(`token`, `autoStart`, `mode`, `music*`)는 지워졌고, 옛 파일에 남아 있어도 무시된다.

- `prefix`("!saba"), `cloud{relayUrl,hostId}`
- `moduleAliases`, `commandAliases`
- `instanceAliases`: 인스턴스 ID → 그 서버를 부르는 별명 목록(`"크리, cr"`). 같은 게임 서버가 여럿일 때 하나를 고른다. 채팅 처리기는 인스턴스 이름 → 인스턴스 별명 → 모듈 별명 순으로 찾고, 여러 서버가 걸리면 `errors.multiple_instances`로 답한다(`src/chat/mod.rs:256-285`)
- `nodeSettings`, `cloudNodes`, `cloudMembers`

**데몬이 실제로 쓰는 설정값은 일부뿐이다.**

- `portConflictCheck`: 기동할 때 `skip_port_check` 원자 변수를 초기화하고(`ipc/mod.rs:632-638`), PUT·reset이 성공하면 갱신한다(`config.rs:29, 89-92`). start 경로와 모니터가 이 값 하나만 본다(`supervisor/mod.rs:365, 1159, 2194`).
- `language`: Python 실행의 `SABA_LANG`, 채팅 응답, 예약 공지, 익스텐션 설명 언어에 쓴다(`plugin/mod.rs:28-35`, `chat/api.rs:119`, `backup.rs:375`, `extension.rs:73`).
- `ipcPort`: `SABA_IPC_PORT`가 없을 때 데몬 포트로 쓴다(`main.rs:225-236`).
- `autoGeneratePasswords`: 미사용.

예전 `src/config/mod.rs`의 `GlobalConfig`는 지워졌다.

---

## 5. 프로토콜 계층 (`src/protocol/**`)과 명령 라우팅

### 5.1 RCON은 없다

- 데몬은 RCON을 쓰지 않는다. 예전 `protocol/rcon.rs`와 `rcon_pool.rs`는 지워졌다. 게임 명령은 모두 stdin으로 간다(§5.4, 02 RCON 정책).
- 인스턴스 모델에서도 `rcon_port`, `rcon_password` 필드를 지웠다. 예전 파일에 남은 RCON 키는 로드할 때 지우고(§4.1), PATCH로 와도 버린다(`instance.rs:822-836, 1009-1011`). 모듈 config에도 RCON 키를 넘기지 않는다(`supervisor/mod.rs:1057-1066`).

### 5.2 REST (`protocol/rest.rs`)

- `ureq`(동기)를 쓰고 timeout은 10초다. Basic Auth를 지원한다.
- 연결이 EOF나 reset으로 끊기면 성공으로 간주한다. Palworld 대응 코드다(`rest.rs:100-119`).
- `verify_connection`은 실제 연결 없이 host와 port 유효성만 본다(`rest.rs:42-48`).
- Basic 토큰 앞 8자와 비밀번호 길이를 **info 레벨로 로그에 남긴다**(`rest.rs:86`).

### 5.3 ProtocolClient (`protocol/client.rs`)

- REST 한 가지만 남았다(`CommandType::Rest`). `/rest` 핸들러가 `new_rest`로 만든다(`command.rs:442`).

### 5.4 명령 라우팅

| 경로 | 동작 |
|---|---|
| `POST /api/instance/:id/command` | 모든 인터페이스의 단일 명령 경로다. 요청 해석·검증은 `crate::commands`가 한다. 모듈에 없는 이름이면 활성 익스텐션 명령을 먼저 본다(`command.rs:58-102`). 그다음 모듈 정의대로 `/rcon`(stdin), `/rest`, `lifecycle.py command`, 원시 stdin 중 하나로 보낸다(`command.rs:136-161`). 꺼져 있고 콘솔도 없으면 409 `server_not_running`이다. `Supervisor::execute_command`(`supervisor/mod.rs:1024-1070`)는 config `{command,args,protocol_mode,working_dir,rest_host,rest_port,pid,instance_id}`를 락 안에서 만들고, 락을 놓은 뒤 Python을 실행한다 |
| `POST /api/instance/:id/rcon` | 이름은 호환용이고 **언제나 stdin으로 보낸다.** `routes_commands_to_stdin`(`supervisor/mod.rs:1493-1508`)이 콘솔 있는 서버(관리 모드, 재접속 후에도 saba-runner가 stdin 유지)를 확인하면 `via_stdin`(`command.rs:308-356`)이 한 줄을 쓰고 콘솔 출력을 최대 약 1.5초 모아 `data.response`로 돌려준다. 콘솔이 없으면 409 `no_command_path`(`command.rs:263-306`) |
| `POST /api/instance/:id/rest` | 접속 정보는 `get_credentials`(게임 설정 파일)에서 읽고 인스턴스별로 캐시한다(`CredentialsCache`, `ipc/mod.rs:302-328`; `command.rs:532-560`). 실패하면 캐시를 지운다. host와 port는 payload, instance, 모듈 기본값 순으로 정하고, 동기 ureq 호출은 `spawn_blocking`으로 감싼다(`command.rs:485-492`) |
| `POST /api/instance/:id/stdin` | `/rcon`과 같은 `via_stdin` 경로다(`managed.rs:90-98`) |

---

## 6. 내장 런타임: python_env

### 6.1 Python (`src/python_env/mod.rs`)

- 버전: 포터블 CPython **3.12.8**(python-build-standalone 태그 `20250106`, `install_only_stripped`)이다(`updater/src/python_pin.rs:7-8`). 시스템 Python을 쓸 경우 3.10 이상이어야 한다(`python_env/mod.rs:45`).
- 탐색 순서: 이미 받은 포터블 Python, 시스템 Python, 자동 다운로드 순이다(`find_base_python`, `:209`). 찾은 인터프리터로 `<pyroot>/python-env` venv를 만들고 pip를 업그레이드한다(`ensure_venv`, `:114`).
- 성공한 결과는 `OnceCell`에 캐시하고, 실패는 캐시하지 않아 다음 호출이 다시 시도한다(`get_python_path`, `:56`). 모든 `run_plugin` 호출이 여기를 거친다(`plugin/mod.rs:178`). 데몬은 기동하자마자 미리 준비를 시작하고(`main.rs:260-264`), 진행은 `/api/runtime/preflight`(`preflight_snapshot`, `python_env/mod.rs:86-106`)로 보인다.
- 다운로드 URL: `https://github.com/indygreg/python-build-standalone/releases/download/{tag}/cpython-{ver}+{tag}-{triple}-install_only_stripped.tar.gz`(`python_pin.rs:41-49`). 지원 triple은 win x64, linux x64/arm64, mac x64/arm64다. `SABA_PYTHON_URL`로 미러를 쓸 수 있다.
- 다운로드 방식: Windows는 PowerShell `Invoke-WebRequest`, 그 외는 `curl`이다. 압축 해제는 `tar`를 쓴다. 풀기 전에 triple별로 고정한 SHA-256과 비교하고 다르면 지운다(`python_env/mod.rs:257-261`, `python_pin.rs:11-17, 53-61`). 미러를 쓰면 `SABA_PYTHON_SHA256`이 있을 때만 검사한다.
- **`<pyroot>`는 `constants::resolve_data_dir()`와 다르다**(`python_env/mod.rs:396-433`). 순서는 다음과 같다.
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
- 결과: stdout 전체, 또는 마지막 비지 않은 줄이 JSON이어야 한다(앞에 찍힌 print는 무시, `parse_plugin_output`, `:322-331`). 종료 코드가 0이 아니면 Err다. 파싱에 실패해도 Err다.
- 진행률: stderr의 `PROGRESS:{json}` 줄을 `ExtensionProgress{percent,message,step,total,label}`로 파싱해 콜백에 넘긴다(`:39-50, 244-264`). 그 밖의 stderr는 debug 로그로만 남긴다.
- 타임아웃: 기본 120초(`DEFAULT_PLUGIN_TIMEOUT_SECS`, `:119`), install 계열은 1800초다. 시간이 넘으면 `child.kill()`한다.
- `PluginCall`(`:130-146`)은 데몬 락 안에서 호출을 만들고 락을 놓은 뒤 실행하기 위한 묶음이다.

**자식 프로세스 환경**

- `PYTHONPATH` = `<data_dir>/python-guard` + `<extensions_dir의 부모>` + `<modules_dir>` + 기존 값(`:196-233`). 앞의 둘은 `extensions_dir`가 디렉토리일 때만 넣는다. `python-guard/sitecustomize.py`는 활성 익스텐션만 import되게 막는다(`:84-96`).
- `SABA_ENABLED_EXTENSIONS` = `config.json`의 `extensions.enabled`를 쉼표로 연결한 값. 목록이 없으면 빈 문자열이다(`:200-205`).
- 데몬 연결 정보 `SABA_IPC_PORT`, `SABA_IPC_URL`, `SABA_TOKEN_PATH`, `SABA_LANG`(`daemon_env`, `:14-37`).
- 첫 실행 때 내장 공용 유틸리티(`i18n.py`, `daemon_rcon.py`)를 모듈 폴더에 맞춰 쓴다(`:98-116, 218-224`).

**모듈의 `lifecycle.py`가 응답해야 하는 함수** (코어에서 직접 호출하는 것만)

- `list_versions` → `{latest:{release}}` 또는 `{latest_release}`
- `install_server` → `{success, executable_path?, install_path?, message?}`
- `import_settings` → `{success, settings{}}`
- `get_credentials` → `{success, rest_username?, rest_password?, ...}`. 데몬은 REST 값만 쓴다(`command.rs:450-455`)
- `command` → `{success, message}`
- 나머지(start/stop/status/properties/eula/diagnose 등)는 supervisor가 호출한다.

예전 `run_extension*`(dead code)은 지워졌다. 익스텐션 훅 실행은 `extension/mod.rs`가 맡는다.

### 7.2 `src/validator/mod.rs`

- `validate_setting_value`(`:70`)는 module.toml `SettingField`의 `field_type`으로 값을 검사한다.
  - `number`: min/max 범위
  - `boolean`: true/false 또는 문자열 "true"/"false"
  - `select`: `options`에 포함되는지
  - `text|password|file`: 문자열(숫자, 불리언도 허용)
  - 알 수 없는 타입은 통과시킨다.
  - `required==Some(true)`인데 값이 `null`이거나 빈 문자열이면 에러다.
- `PATCH /api/instance/:id`는 `validate_settings_patch`(`:222-238`)로 **요청에 들어 있는 필드만** 검사한다(`instance.rs:779`). 보내지 않은 필수 필드는 오류가 아니다.
- `check_port_conflicts`(`:253`)는 주어진 인스턴스 집합과 포트가 겹치는지 검사한다. 모듈이 지원하는 프로토콜의 포트만 본다. supervisor의 start 경로와 모니터는 **실행 중인** 인스턴스만 넘긴다.
- `GET /api/servers`의 `port_conflicts`도 같은 함수를 쓰되(`server.rs:224-248`), "켜면 부딪힐" 서버를 미리 보여 주려고 정지된 인스턴스까지 모두 넘긴다.

### 7.3 기타

- `process_monitor.rs`는 `sysinfo` 0.30으로 전체 프로세스 스냅샷(`pid,name,exe,cmd`)을 만든다. `spawn_blocking`으로 감싼 async 버전도 있다.
- `daemon_log.rs`는 tracing Layer와 링 버퍼(10k)를 제공하고, `get_since(since,count=200)`로 조회한다.
- `audit.rs`(A10): 서버 시작·정지·재시작(`/api/instance/:id/*`, `/api/server/:name/*`), 게임 명령(`execute_command`), 콘솔 입력(`/stdin`)을 `<data_dir>/audit.log`에 한 줄 JSON으로 남긴다(1 MiB에서 `audit.log.1`로 넘김, 인자·줄은 300자까지). 출처는 task-local `Origin`: HTTP 요청은 미들웨어가 `X-Saba-Client`(GUI `gui`, CLI `cli`, 없으면 `api`)로, 릴레이 에이전트는 `discord`+요청자·길드 id로, 예약 작업은 `schedule`로 정한다. 재시작 안의 정지·시작은 `quietly`로 따로 남기지 않는다. 로컬 클라이언트는 같은 토큰을 쓰므로 출처는 기록용이지 보안 경계가 아니다.
- 콘솔 프로그램을 부를 때는 창을 숨긴다: 데몬은 `utils::apply_creation_flags`(tokio), 업데이터·인스톨러와 std `Command`는 `saba_chan_updater_lib::process::hide_console`. 분리 실행(`DETACHED_PROCESS`)된 데몬이나 GUI 프로그램(인스톨러·업데이터)이 콘솔 프로그램을 플래그 없이 부르면 부를 때마다 검은 창이 뜬다.
- `utils.rs`는 Windows `CREATE_NO_WINDOW` 플래그와 경로 한 칸 이름 검사(`is_single_path_component`)를 다룬다(`utils.rs:8-20`).

---

## 8. updates.rs: 업데이트 엔드포인트와 외부 URL

- `UpdateState`가 `UpdateManager`(updater 크레이트)를 `Arc<RwLock>`으로 감싼다. 설정은 `config.json`의 `updates` 절에서 읽고(`load_updater_config`, `updates.rs:786-798`), 없거나 깨졌으면 `UpdateConfig::default()`를 쓴다.
  - 필드(`updater/src/lib.rs:236-264`): `enabled`(기본 true), `check_interval_hours`(3), `auto_download`(false), `install_root`, `relay_url`(None이면 `https://saba-chan.online`), `channel`(`stable`).
- `PUT /api/updates/config`는 바꾼 설정을 `config.json`에 저장한다(`updates.rs:770-772`).
- 주기 확인은 데몬의 `spawn_update_scheduler`가 한다(`updates.rs:247`). `check_interval_hours`마다 확인하고 `auto_download`이면 받아 둔다.
- **접속하는 외부 URL**
  - 릴레이: `GET {relay_url}/v1/updates/check?channel&platform&current&client_id`, `GET /v1/updates/manifest/:version`, `POST /v1/updates/event`(익명 진행 이벤트). 모든 요청에 `X-Saba-Client: 1` 헤더(`updater/src/relay.rs:1-25, 160-195`).
  - manifest의 각 컴포넌트 `url`(R2 공개 주소). 받을 때 `sha256`을 확인한다(`updater/src/download.rs:40`).
  - manifest는 번들된 공개키로 Ed25519 서명을 검증하고, 플랫폼이 다르거나 현재보다 낮은 버전이면 거부한다(`relay.rs:233-252`, `signature.rs`).
  - `/api/modules/manifest*`, 익스텐션 목록(`ExtensionManager::fetch_remote`), 설치기는 같은 함수 `relay::latest_catalog`를 쓴다(`relay.rs:107-115`). 데몬 쪽 호출은 모두 `None`을 넘기므로 기본 릴레이와 `stable` 채널을 쓰며 `relay_url` 설정을 보지 않는다(`server.rs:430, 460`, `extension/mod.rs:1284`).
  - Python 런타임만 예외로 `github.com/indygreg/python-build-standalone` 릴리즈에서 받는다(고정 해시 검사, §6).
- **apply 규칙**(`updates.rs:447-568`)
  - 대상에 GUI, Updater, CoreDaemon 중 하나라도 있으면 **전부** 업데이터 exe에 맡긴다. 이때 `needs_updater`를 채우고 `requires_updater:true`를 돌려준다.
  - 그렇지 않으면 데몬이 직접 적용하고, 모듈은 refresh, 익스텐션은 rescan으로 핫 리로드한다.
  - 업데이터 실행은 클라이언트가 `POST /api/updates/launch-updater`로 데몬에 맡긴다(`updates.rs:570-639`). 데몬이 이번 적용에만 쓰는 실행 표(`launch_ticket::issue`)를 만들어 `--apply --wait-pid <pid> --launch-token <표>`로 업데이터를 detach로 띄우고, 0.5초 뒤 `stop_requested`로 스스로 종료한다.
- `/api/updates/*`도 다른 라우트처럼 토큰 인증을 거친다(`ipc/mod.rs:756-758`). `relay_url`을 바꿔도 manifest 서명 검증은 그대로라 다른 서버가 파일을 바꿔 끼울 수는 없다.

---

## 9. 불일치, 데드 코드, 반쯤 구현된 기능, 위험 지점

### 9.A 오너 원칙 위반 (인터페이스가 데몬을 우회하는 곳)

1. ~~**GUI가 `settings.json`을 직접 읽고 쓴다.**~~ → 해결: GUI는 `config.json`의 `gui`를 읽기만 직접 하고(`saba-chan-gui/main.js:510-521`), 저장은 데몬 API(`PUT /api/config/gui`)로 하며 데몬이 응답하지 않을 때만 파일에 직접 쓴다(`main.js:526-545`, `writeGuiSection` `:485-496`).
2. ~~**GUI가 relay 서버를 직접 호출한다.**~~ → 해결: `main.js`에 릴레이 직접 호출이 없다. 봇 설정·초대·페어링은 데몬의 `/api/relay/*`, `/api/config/bot`을 거친다.
3. ~~**봇 실행 로직이 GUI에 중복돼 있다.**~~ → 해결: 로컬 봇과 범용 `/api/ext-process/*`가 지워졌다(L1). 디스코드 연동은 데몬의 `relay_agent.rs` 하나뿐이다.
4. **GUI가 데몬의 경로 정책을 결정한다.** `main.js:596-603`에서 `SABA_INSTANCES_PATH`, `SABA_MODULES_PATH`, `SABA_IPC_PORT`를 GUI가 정해 주입한다.
5. **CLI가 로컬 파일을 직접 읽는다.** (CLI 참조는 PR #6 리메이크 기준)
   - 데몬을 찾고 띄우는 `saba-chan-cli/src/bootstrap.rs`만 `config.json`의 `gui` 섹션을 직접 읽는다(`ipcPort`, `language`, `:14-46`). 파일 머리 주석이 이를 유일한 예외로 적어 둔다(`:1-6`).
   - ~~데몬은 `SABA_IPC_PORT`를 먼저 보므로 포트가 다를 수 있다~~ → 해결: CLI도 `SABA_IPC_PORT` > `gui.ipcPort` > 기본값 순으로 읽어 데몬(`main.rs:225-236`)과 같다(`bootstrap.rs:18-25`). 다만 GUI가 데몬에만 넣어 준 환경변수는 따로 띄운 CLI가 보지 못한다.
   - ~~`scan_directory` 로컬 fs 폴백, `get_module_locales` 로컬 폴백, 컴파일되지 않는 `gui_config.rs` 사본~~ → 해결: 예전 `config.rs`·`gui_config.rs`가 리메이크에서 지워졌다. CLI 문구(`locales/<lang>/cli.json`)는 로컬에서 읽는다(`i18n.rs:38`).
6. ~~**Discord 봇(server-chan)이 `bot-config.json`을 파일 경로(`BOT_CONFIG_PATH`)로 받는다.**~~ → 해결: 로컬 봇이 없다. 릴레이 에이전트가 하트비트로 봇 설정을 올린다(§1.4).
7. **토큰 파일 직접 읽기**(GUI, CLI, Python 모듈)는 인증 부트스트랩 구조상 피할 수 없으므로 예외로 본다.
8. ~~**반대 방향 위반: 코어 안에 특정 익스텐션 이름이 하드코딩돼 있다.**~~ → 해결: `docker`·steamcmd 분기가 코어에서 빠졌다. 설치 방식은 `install_method_<방식>` 슬롯으로 범용화됐다(`instance/mod.rs:67-148`, `instance.rs:1240-1253`). 이름은 테스트에만 남아 있다.

### 9.B 보안과 정합성

1. ~~`/api/updates/*`에 인증이 없다~~ 해결됨: 업데이트 라우터도 인증 미들웨어 안에 있다(`ipc/mod.rs:756-758`). 업데이트 출처 설정은 `relay_url`뿐이고, 서명되지 않은 manifest는 받지 않는다.
2. ~~**`/health`는 인증이 필요하다.**~~ → 해결(A6): `/health`는 토큰 없이 답하고 보낸 토큰의 상태(`token`)만 알려 준다. 자세한 진단은 인증이 필요한 `/api/diagnostics`.
3. ~~**토큰이 없으면 fail-open이다**~~ → 해결: 캐시도 파일도 없으면 401로 거부한다(`auth.rs:115-124`). ~~비교가 상수 시간이 아니다~~ → 해결: `constant_time_eq`로 끝까지 비교한다(`auth.rs:86-89, 134`).
4. ~~**단일 인스턴스 보장이 없다.**~~ → 해결: `daemon.lock` 배타 잠금을 토큰 생성 전에 잡고(`main.rs:240-255`), Windows에서는 `SO_REUSEADDR`를 켜지 않는다(`ipc/mod.rs:772-775`). §1.6.
5. ~~`/api/ext-process/:name/start`는 임의 명령을 실행~~(라우트 삭제). ~~`/api/fs/scan-dir`는 임의 디렉토리를 열람한다~~ → 해결: 지웠다. 마이그레이션은 게임 폴더 판정(`POST /api/modules/detect`)만 쓴다(D2).
6. ~~**relay 토큰 유출 가능성**: `?relayUrl=`로 임의 호스트에 `Authorization: Bearer <node_token>`을 보낼 수 있다~~ → 해결: 서명 요청(host status·nodes·members)은 `signed_relay_url`로 설정한 릴레이(없으면 기본)만 쓰고, 요청 주소가 그와 같거나 루프백(모의 릴레이)일 때만 따른다(`relay.rs:45-67, 211-241`). 챗봇 동기화도 설정한 릴레이만 쓴다(`chatbot.rs:150-153`). ~~서명 없는 bot-invite·pair는 `relayUrl`을 그대로 따른다~~ → 해결: 페어링(시작·상태)과 bot-invite도 같은 규칙을 쓴다(`relay.rs:271, 299, 327`, 59a93b8).
7. ~~**`POST /api/modules/manifest/:id/install`에 경로 검사가 없다**~~ → 해결: `..`, `/`, `\`를 거부한다(`server.rs:457-459`). `get_module_locales`는 여전히 이름을 그대로 `join`한다(`config.rs:201-203`).
8. **비밀값이 평문으로 응답에 실린다.** `GET /api/instances`, `/api/servers`에는 REST 비밀번호가 들어 있다. ~~`/api/config/bot`의 Discord 토큰~~(BotConfig에서 `token` 삭제). REST Basic 토큰 앞부분과 비밀번호 길이는 로그에 남는다(`rest.rs:86`).
9. **익스텐션 설치에 사용자 지정 `download_url`을 허용하고 sha256은 선택**이다(`extension.rs:357-375`). ~~포터블 Python/Node 다운로드에 체크섬 검증이 없다~~ → 해결: Python은 고정 해시로 검사하고(§6.1), Node 다운로드는 지워졌다.
10. ~~**프로비저닝 dismiss 롤백 경쟁**~~ → 해결: 롤백은 신규 생성으로 표시된 이름(`mark_creating`)일 때만 한다. 기존 인스턴스의 업데이트 실패는 진행 표시만 지운다(`instance.rs:565-603`, `ipc/mod.rs:379-410`).
11. ~~**`/api/daemon/shutdown`과 `unregister?shutdown=true`는 토큰만 cancel한다**~~ → 해결: 둘 다 `stop_requested`를 cancel하고, main의 단일 종료 경로가 `daemon.shutdown` 훅과 클라이언트 정리를 거친다(`ipc/mod.rs:621-627`, `main.rs:533-599`).
12. ~~**`SABA_ENABLED_EXTENSIONS` 경로 해석 버그**~~ → 해결: `config.json`의 `extensions.enabled`를 `ConfigFile::default_location()`(`SABA_DATA_DIR` 반영)으로 읽는다(`plugin/mod.rs:200-205`).
13. ~~**락을 오래 잡는 문제**~~ → 해결(C12): supervisor 함수는 `&RwLock`을 받아 단계마다 짧게 잠그고 Python은 락 밖에서 실행한다(`PluginCall`, `supervisor/mod.rs:1024-1070`). 마이그레이션 `import_settings`(`instance.rs:523-547`), managed 시작(`managed.rs:56`), 익스텐션 다운로드(`extension.rs:382-393`, write lock 전에 받음)도 락 밖이다. 2초 모니터는 여전히 `monitor_processes` 동안 write lock을 잡는다(`main.rs:371-372`).
14. ~~**REST 명령이 async 핸들러 안에서 동기로 실행된다**~~ → 해결: `spawn_blocking`으로 실행한다(`command.rs:485-492`).
15. ~~**ext-process 출력이 stderr로 새어 나간다.**~~ → 해결: `ext_process.rs`가 지워졌다.

### 9.C 데드 코드와 반쯤 구현된 기능

- ~~`src/config/mod.rs`의 `GlobalConfig`는 미사용이다~~ → 해결: 지워졌다.
- ~~`handlers/python_env.rs`의 세 핸들러는 라우트가 없다~~ → 해결: 핸들러와 `pip_install*`가 지워졌다. 런타임 상태는 `/api/runtime/preflight`가 보인다.
- ~~`plugin::run_extension*`, `RestClient::with_basic_auth`, `ServerResponse::success/error`, `CommandResponse`, `BotConfig`(ipc/mod.rs), `ExtensionInitTracker::is_initializing`, `InstanceStore::save`, `ServerInstance::ext_f64`, `process_monitor::find_by_name*` 동기 버전 미사용~~ → 해결: 모두 지워졌다(`ext_str`는 테스트 전용, `instance/mod.rs:110-113`).
- ~~업데이트 설정의 `check_interval_hours`, `auto_download`, `auto_apply`는 저장만 되고 아무 동작이 없다~~ → 해결: `config.json`에 저장되고, 데몬 스케줄러가 주기와 `auto_download`를 쓴다(`updates.rs:247`, `:770-772`). `auto_apply`는 설정에서 지워졌고 적용은 언제나 사용자가 고른다(`updater/src/lib.rs:236-251`).
- ~~`skip_port_check` 계열이 따로 논다~~ → 해결: 기동 시 `gui.portConflictCheck`로 초기화하고, start 경로와 모니터가 같은 원자 변수 하나만 본다(§4.4).
- `GuiSettings.autoGeneratePasswords`는 데몬에서 효과가 없다. ~~`ipcPort`~~는 데몬 포트로 쓴다(§4.4).
- 비 Windows 부트 선택기는 키 입력을 지원하지 않는데도 DEL 안내를 표시한다(`boot_selector.rs:244-248, 308`).
- `ServerStartRequest.module`이 필수 필드다(`ipc/mod.rs:155-159`). 인스턴스에 이미 모듈 정보가 있는데도 `/api/server/:name/start` 호출자가 다시 보내야 한다. `/api/instance/:id/start`는 이 값을 데몬이 채운다(`server.rs:602`).
- `python_env`의 데이터 디렉토리가 `constants::resolve_data_dir()`와 어긋난다(§6). 같은 파일 상단 주석은 `<data_dir>`라고 적고 있다.
- `install_server` 진행 콜백은 `step/total`을 그대로 넘기지만, 완료될 때는 `step:0,total:1`로 덮어쓴다(`instance.rs:440-449`). 백그라운드 설치의 진행 단계 정보가 사라진다.
- ~~`CLAUDE`/문서 주장 검증: `src/ipc/updates.rs:13`의 doc-claim "`/api/updates/apply` — 업데이터 exe 스폰하여 적용"은 코드와 다르다~~ → 해결: 모듈 머리 주석이 "`requires_updater`로 돌려주고 실제 적용은 `launch-updater`가 한다"로 바뀌었다(`updates.rs:13-14`). 핸들러 주석(`updates.rs:437`)에는 "업데이터 exe를 스폰하여 적용"이 아직 남아 있다. 그 밖의 README/docs 내용은 이 문서에서 근거로 쓰지 않았다.

### 9.D 테스트 계약 (`tests/daemon_integration.rs`)

- `boot_ipc()`(`:110-145`)는 `SABA_AUTH_DISABLED=1`로 인증을 끄고(디버그 빌드라 동작), 임시 instances 디렉토리와 `./modules`를 쓴다.
  - `IPCServer::new`는 `<data_dir>`의 ConfigStore와 익스텐션 디렉토리를 읽지만, 테스트 빌드에서는 dev-dependency의 `test-isolation` 기능으로 `<data_dir>`가 프로세스별 임시 폴더가 된다(`updater/src/constants.rs:127-141`, `Cargo.toml:77`). 사용자 데이터 디렉토리에 닿지 않는다.
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
