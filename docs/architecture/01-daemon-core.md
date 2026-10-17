# 01. 데몬 코어 (saba-core) 분석

> 범위: `src/main.rs`, `src/lib.rs`, `src/ipc/**`, `src/instance`, `src/config`, `src/config_store`, `src/plugin`, `src/python_env`, `src/validator`, `src/process_monitor.rs`, `src/boot_selector.rs`, `src/daemon_log.rs`, `src/utils.rs`, `src/protocol/**`, `tests/daemon_integration.rs`.
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

### 1.1 `main()` 흐름 (`src/main.rs:28-527`)

1. **부트 모드 결정** (`main.rs:33-44`)
   - `--spawned` 인자가 있거나, stdin과 stdout이 둘 다 터미널이 아니면 부트 선택기를 건너뛴다(`boot_mode = None`).
   - 그 외에는 `boot_selector::run()`이 `Gui | Cli | DaemonOnly` 중 하나를 고른다.
   - **실제로 인식하는 CLI 인자는 `--spawned` 하나뿐이다.** 다른 인자 파싱은 없다.
2. **로그 버퍼와 tracing 초기화** (`main.rs:48-102`)
   - `DaemonLogBuffer`(링 버퍼 10,000줄, `daemon_log.rs:21`)와 `DaemonLogLayer`를 모든 모드에 합성한다.
   - 필터는 `RUST_LOG`이며 기본값은 `info`다.
   - CLI 모드는 TUI가 터미널을 차지하므로 로그를 `<data_dir>/daemon.log` 파일로 보낸다(truncate, `main.rs:63`). 나머지 모드는 stderr로 보낸다.
3. **무결성 검사** `run_integrity_check_on_startup()` (`main.rs:111, 577-655`)
   - `UpdateManager::verify_integrity()`가 릴레이에서 이 채널·플랫폼 최신 릴리즈의 서명된 manifest를 받아(`updater/src/lib.rs:437-463`) 설치된 컴포넌트의 SHA256을 비교한다. 백그라운드 task라 기동을 막지 않는다(`main.rs:214, 663`).
   - 결과는 로그로만 남긴다. 변조가 감지돼도 경고만 하고 기동은 계속된다(`main.rs:643-647`).
   - 릴레이에 닿지 않거나 게시된 릴리즈가 없으면 경고만 남기고 건너뛴다. 기동할 때마다 릴레이에 한 번 요청한다.
4. **`GlobalConfig::load()`** (`main.rs:114`): 하드코딩 기본값만 돌려준다. 실제로 쓰이지 않는 데드 코드다(§9).
5. **Supervisor 생성과 초기화** (`main.rs:119-127`)
   - `plugin::resolve_modules_dir()`로 모듈 경로를 정하고 `Supervisor::new()`, `initialize()` 순으로 호출한다.
   - `initialize()`는 모듈 탐색, 인스턴스 로드, 고아 managed 프로세스 재연결(`managed.pid`), `server_version` 백필을 한다(`supervisor/mod.rs:77-96`).
6. **IPC 토큰 생성** `ipc::auth::generate_and_save_token()` (`main.rs:130`). 실패해도 경고만 하고 진행한다.
7. **IPC 서버 객체 생성** (`main.rs:136-141`)
   - 포트는 `SABA_IPC_PORT` 환경변수, 없으면 `DEFAULT_IPC_PORT = 57474`(`constants.rs:41`)다.
   - **바인드 주소는 `127.0.0.1`로 고정이다.**
   - `IPCServer::new`(`ipc/mod.rs:528`)가 ExtensionManager를 만들고 `discover()`를 호출한다. ConfigStore도 이때 만든다.
8. Supervisor에 `extension_manager`와 `provision_tracker`를 주입한다(`main.rs:144-148`).
9. **백그라운드 태스크 기동**
   - 익스텐션 `daemon.startup` 훅 비동기 디스패치(`main.rs:156-200`). 진행 상황은 `ExtensionInitTracker`에 기록되고 `/api/extensions/init-status`로 노출된다.
   - Discord 봇 자동 시작 `auto_start_discord_bot` (`main.rs:205-213, 669-794`). 아래 1.4 참조.
   - 프로세스 모니터: 2초 주기로 `process_monitor::get_running_processes_async()` 스냅샷을 찍고 `sup.monitor_processes()`를 호출한다(`main.rs:221-258`). 이때 supervisor **write lock**을 잡는다.
   - 하트비트 reaper: 30초 주기로 90초 넘게 하트비트가 없는 클라이언트를 정리하고, 그 클라이언트의 봇 PID를 kill한다(`main.rs:263-279`, `handlers/client.rs:143`).
   - 렌더러 watchdog: DaemonOnly가 아닐 때만 돈다(`main.rs:299-392`).
     - 클라이언트가 한 번이라도 연결된 적이 있다가 모두 끊기면 15초 유예 후 GUI, CLI 순으로 재기동을 시도한다.
     - 재기동 후 60초 안에 재접속이 없으면 다시 시도하고, 최대 2회 실패하면 데몬이 **스스로 종료**한다.
   - DaemonOnly 모드: `daemon_log::daemon_terminal_loop`가 500ms마다 각 인스턴스 콘솔을 stderr에 출력한다(`daemon_log.rs:191`).
   - Ctrl+C 핸들러(`main.rs:400-458`): `daemon.shutdown` 훅, 클라이언트 봇 kill, `ExtProcessManager::shutdown_all()`, 토큰 cancel 순으로 정리한다.
10. **인터페이스 스폰** (`main.rs:463-518`)
    - IPC 포트가 열릴 때까지 최대 15초 기다린다(`boot_selector::wait_for_ipc_port`).
    - GUI 모드: `saba-chan-gui --spawned-by-daemon`을 detach로 띄우고 콘솔 창을 숨긴다(Windows만).
    - CLI 모드: `saba-chan-cli`를 stdio를 상속해 같은 콘솔에 붙인다.
    - 인터페이스 바이너리는 데몬 exe와 같은 디렉토리에서만 찾는다(`boot_selector.rs:454`).
11. **`ipc_server.start().await`** (`main.rs:521`): 이 호출이 블로킹 메인 루프다. 토큰이 cancel되면 graceful shutdown한다.

### 1.2 부트 선택기 (`src/boot_selector.rs`)

- 마지막 선택을 `<data_dir>/boot-config.json`에 `{"last_mode":"gui"|"cli"|"daemononly"}` 형태로 저장한다(`boot_selector.rs:59-82`). `daemon_only` 별칭도 받는다.
- 5초 카운트다운 동안 DEL 키를 누르면 메뉴가 열린다(↑/↓, 1-3, Enter).
- **키 입력은 Windows에서만 구현돼 있다.** 비 Windows에서는 `poll_key_fallback`이 그냥 sleep하므로(`boot_selector.rs:253-257`) 메뉴를 열 수 없고, 항상 마지막 모드로 자동 부팅한다. 화면의 "Press [DEL]" 안내만 남아 있다.

### 1.3 환경변수 목록 (코어가 읽는 것)

| 변수 | 위치 | 의미 |
|---|---|---|
| `SABA_IPC_PORT` | `main.rs:136` | IPC 포트 (기본 57474) |
| `RUST_LOG` | `main.rs:57` | tracing 필터 |
| `SABA_DATA_DIR` | `constants.rs:89`, `python_env/mod.rs:448` | 데이터 루트 |
| `SABA_TOKEN_PATH` | `constants.rs:113` | IPC 토큰 파일 경로 |
| `SABA_MODULES_PATH` | `constants.rs:125` | 모듈 디렉토리 |
| `SABA_EXTENSIONS_DIR` | `constants.rs:137` | 익스텐션 디렉토리 |
| `SABA_INSTANCES_PATH` | `constants.rs:149` | 인스턴스 루트 |
| `SABA_DISCORD_BOT_DIR` | `constants.rs:172` | discord_bot 디렉토리 |
| `SABA_AUTH_DISABLED=1` | `ipc/auth.rs:82` | 인증 미들웨어 비활성화 |
| `SABA_PYTHON_URL` | `python_env/mod.rs:262` | 포터블 Python 다운로드 URL 오버라이드 |
| `APPDATA`, `HOME` | 경로 해석 | 플랫폼 기본 경로 |

**데몬이 자식 프로세스에 넘기는 환경변수**

- Python 플러그인: `PYTHONIOENCODING`, `PYTHONUNBUFFERED`, `PYTHONPATH`, `SABA_ENABLED_EXTENSIONS`.
- 봇: `IPC_BASE`, `BOT_CONFIG_PATH`, `SABA_EXTENSIONS_DIR`, `SABA_LANG`, 그리고 모드에 따라 `DISCORD_TOKEN` 또는 `RELAY_URL`+`RELAY_NODE_TOKEN`.
- `SABA_LANG`는 GUI가 데몬에 넘기지만(`saba-chan-gui/main.js:559`) **데몬 코어는 이 값을 읽지 않는다.** 봇을 띄울 때는 `settings.json`의 `language`를 읽는다(`main.rs:797`).

### 1.4 디스코드 릴레이 연결 (`src/relay_agent.rs`)

로컬 Discord 봇 실행 경로(`node_env`, `discord_bot` ext-process)는 L1(7b5102f)에서 지웠다. 디스코드 봇은 클라우드 릴레이에서만 돌고, 데몬은 릴레이 에이전트로 붙는다.

1. IPC 서버가 뜬 뒤 `relay_agent::auto_start`를 띄운다(`ipc/mod.rs:783`). 수동 제어는 `/api/relay/agent{,/start,/stop}`.
2. `GET /info`로 최소 에이전트 버전을 확인하고, 60초마다 `POST /heartbeat`로 서버·모듈·봇 설정 메타데이터를 올린다.
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
  .ipc_token                IPC 인증 토큰 (uuid v4, Unix 0600)          auth.rs:27
  .node_token               cloud relay 노드 토큰 (sbn_<id>.<secret>)   handlers/config.rs, relay.rs
  settings.json             GUI 설정 (ConfigStore GuiSettings)
  bot-config.json           봇 설정 (ConfigStore BotConfig)
  boot-config.json          부트 선택기 마지막 모드
  daemon.log                CLI 부트 모드에서만 생성
  extensions_state.json     활성 익스텐션 ID 배열 (ExtensionManager)
  extensionConfig.json      익스텐션 글로벌 설정
  installed-manifest.json, updater-state.json, update-complete.json (updater)
  modules/<module>/module.toml, lifecycle.py, locales/*.json
  extensions/<ext>/...
  instances/
    order.json
    <uuid>/instance.json, settings.json, managed.pid, logs/console.log, server/ (설치 대상)
<exe_dir>/
  release-manifest.json     /api/system/components 가 읽음 (config.rs:332)
  discord_bot/index.js      (exe 옆에 없으면 cwd 기준 ./discord_bot)
  saba-chan-gui[.exe], saba-chan-cli[.exe]   watchdog·부트 선택기가 스폰
```

- 업데이트 스테이징 디렉토리: Windows는 `<data_dir>/updates`, Unix는 `~/.cache/saba-chan/updates`(`constants.rs:191`).
- Python 런타임은 **위의 `<data_dir>`와 다른 경로**를 쓴다(§6).

### 1.6 단일 인스턴스 처리: 없음

- 락 파일이나 뮤텍스 같은 단일 인스턴스 보장 장치가 없다.
- 유일한 방어선은 TCP 바인드다. `socket2`로 `SO_REUSEADDR`를 켜고 2초 간격으로 10회 재시도한다(`ipc/mod.rs:671-715`).
- 문제 1: **두 번째 데몬은 바인드를 시도하기 전에 이미 `.ipc_token`을 덮어쓴다**(`main.rs:130`이 `start()`보다 먼저 실행된다).
  - 첫 번째 데몬은 메모리에 캐시된 옛 토큰으로 검증한다(`auth.rs:89-92`).
  - 그래서 파일에서 새 토큰을 읽은 클라이언트는 첫 번째 데몬에게 **401을 받는다.**
  - 두 번째 데몬은 20초 동안 재시도하다 종료하지만, 그동안 무결성 검사 네트워크 호출, Supervisor 초기화, 봇 자동 시작 시도까지 이미 수행한다.
- 문제 2: Windows의 `SO_REUSEADDR`는 이미 리슨 중인 포트에도 바인드를 허용하는 의미다. 그래서 **Windows에서는 두 데몬이 같은 포트에 동시에 바인드될 수 있다.**

---

## 2. IPC HTTP API 전체 표

### 2.1 공통 규약

- **라우터 정의 위치**: `src/ipc/mod.rs:572-669` (axum 0.7).
- **경로 파라미터 표기**: axum 0.7의 `:param` 문법을 쓴다.
- **인증 적용 범위**: `.layer(auth_middleware)`는 그 앞에서 `.route()`로 등록한 라우트에만 적용된다.
  - `/api/updates/*` 라우터는 레이어를 단 **뒤에** `.merge()`된다(`ipc/mod.rs:669`).
  - 따라서 **`/api/updates/*`에는 인증이 없다.**
  - 반대로 `/health`는 레이어 앞에서 등록되므로 **토큰이 필요하다.** "경량 ping"이라는 주석과 다르다.
- **에러 형식이 통일돼 있지 않다.** 다음 형식들이 섞여 있다.
  - `{"error": "..."}` (+ 선택적으로 `"error_code"`)
  - `{"ok": false, "error": ...}` (updates, modules manifest/install/remove, `/api/fs/scan-dir`는 성공 시 ok)
  - `{"success": false, "error", "error_code", "related"}` (익스텐션)
  - 인증 실패: 본문 없는 401 (`auth.rs:150`)
  - 일부 핸들러는 실패해도 HTTP 200을 돌려준다(modules manifest/install/remove, extensions manifest/install, updates).
- **JSON 바디 파싱 실패**: axum 기본 거부 응답(400/415/422, 텍스트 본문)이 나간다.
- **식별자 혼용**: 같은 API 안에서 서버는 **이름**(`/api/server/:name/*`, `/api/provision-progress/:name`)으로, 인스턴스는 **UUID**(`/api/instance/:id/*`)로 가리킨다.

### 2.2 라우트 표

아래 표에서 "Auth"는 `X-Saba-Token` 헤더가 필요한지를 뜻한다. ✅는 필요, ❌는 불필요다.

#### 헬스 / 서버 제어 (handlers/server.rs)

| Method | Path | Handler | Auth | 요청 | 응답 |
|---|---|---|---|---|---|
| GET | `/health` | `server.rs:18` `health_check` | ✅ (주석과 달리) | - | `{ok:true, version}` |
| GET | `/api/servers` | `server.rs:24` `list_servers` | ✅ | - | `{servers:[ServerInfo], port_conflict_stops?:[...]}` (아래 설명) |
| GET | `/api/server/:name/status` | `server.rs:545` | ✅ | - | supervisor 결과 JSON 그대로. 404 시 `{error, error_code:"instance_not_found"}` |
| POST | `/api/server/:name/start` | `server.rs:579` | ✅ | `{module: string (필수), config?: any}` | supervisor 결과. 에러 시 404/422/500 + `error_code`∈{instance_not_found, executable_missing, start_failed} |
| POST | `/api/server/:name/stop` | `server.rs:638` | ✅ | `{force?: bool}` (바디 필수) | supervisor 결과. 에러 시 `error_code`∈{instance_not_found, stop_failed} |

`GET /api/servers` 응답의 `ServerInfo` 필드(`ipc/mod.rs:207-243`):

- 기본: `id, name, module, status, pid, start_time, executable_path, working_dir?, port`
- 접속 정보: `rcon_port, rcon_password, rest_host, rest_port, rest_username, rest_password, protocol_mode`
- 설정과 상태: `module_settings, server_version, last_api_action?, provisioning, extension_status?, extension_data, port_conflicts?`
- **비밀번호가 평문으로 포함된다.**

`port_conflict_stops`는 한 번 읽으면 비워지는 drain 큐다(`server.rs:260-263`).

`status` 값은 다음 중 하나다.

- `running` / `stopped`: PID 기반
- 익스텐션 `server.list_enrich` 훅이 `handled:true`를 돌려준 경우: 그 훅이 준 값
- `provisioning`: 프로비저닝 진행 중

#### 모듈 (handlers/server.rs, handlers/managed.rs, handlers/config.rs)

| Method | Path | Handler | Auth | 요청 | 응답 |
|---|---|---|---|---|---|
| GET | `/api/modules` | `server.rs:269` | ✅ | - | `{modules:[ExtensionInfo]}` (아래 설명) |
| POST | `/api/modules/refresh` | `server.rs:323` | ✅ | - | 위와 같음 |
| GET | `/api/modules/manifest` | `server.rs:443` | ✅ | - | `{ok, manifest}` 또는 `{ok:false,error}`. 항상 200 |
| POST | `/api/modules/manifest/:id/install` | `server.rs:460` | ✅ | - | `{ok, module_id}` 또는 `{ok:false,error}`. 항상 200 |
| DELETE | `/api/modules/:id` | `server.rs:519` | ✅ | - | `{ok, id}`. 항상 200 |
| GET | `/api/module/:name` | `server.rs:381` | ✅ | - | `{name, version, description, path, metadata, toml?}` / 404 |
| GET | `/api/module/:name/versions` | `managed.rs:257` | ✅ | query `include_snapshots, page=1, per_page=25` | supervisor 결과 |
| GET | `/api/module/:name/version/:version` | `managed.rs:291` | ✅ | - | supervisor 결과 |
| POST | `/api/module/:name/install` | `managed.rs:368` | ✅ | `{version*, install_dir*, jar_name?, accept_eula?, initial_settings?}` | 200(success) / 422 / 404 / 400 / 500 |
| GET | `/api/module/:name/locales` | `config.rs:102` | ✅ | - | `{<lang>: <json>}` |

- `ExtensionInfo`(이름과 달리 모듈 정보다) 필드: `name, version, description, path, executable_path, icon(base64 data URL), interaction_mode, protocols{supported,default}, settings, commands, syntax_highlight, dir_signatures?, install?, module_aliases?, command_aliases?`
- `/api/modules/manifest`는 최신 릴리즈 서명 manifest의 `module-*` 컴포넌트를 `{ok, manifest:{modules:{<이름>:{version, sha256, size_bytes, display_name, ...}}}}`로 돌려준다(`relay::latest_catalog`, `handlers/server.rs:430-452`).
- `/api/modules/manifest/:id/install`은 같은 manifest의 `module-<id>`를 R2에서 받아 SHA-256을 확인하고(`download_verified`), staging에 푼 뒤 `module.toml`이 있을 때만 `modules/<id>`와 바꾸고 refresh한다(`:455-516`). `id`의 `..`, `/`, `\`는 거부한다.

#### 인스턴스 (handlers/instance.rs)

| Method | Path | Handler | Auth | 요청 | 응답 |
|---|---|---|---|---|---|
| GET | `/api/instances` | `instance.rs:12` | ✅ | - | `[ServerInstance]` (§4.1, 비밀번호 평문 포함) |
| POST | `/api/instances` | `instance.rs:68` | ✅ | `{name*, module_name*, use_container?, executable_path?, working_dir?, migration_source?, port?}` | 201 `{success, id, provisioning?}`. 409 `duplicate_native_instance`, 422 `extension_required`, 400/500 |
| PUT | `/api/instances/reorder` | `instance.rs:19` | ✅ | `{order:[id]}` | `{success:true}` / 400 |
| GET | `/api/instance/:id` | `instance.rs:49` | ✅ | - | `ServerInstance` / 404 `{error, error_code}` |
| DELETE | `/api/instance/:id` | `instance.rs:779` | ✅ | - | `{success}` / 404 / 500 |
| PATCH | `/api/instance/:id` | `instance.rs:869` | ✅ | 임의 키-값 객체 | `{success:true}` / 400 `validation_failed{details}` / 404 / 500 |
| GET | `/api/provision-progress/:name` | `instance.rs:750` | ✅ | - | `{active:false}` 또는 `{active:true, step,total,label,message,done,error,percent?,steps?}` |
| DELETE | `/api/provision-progress/:name` | `instance.rs:711` | ✅ | - | `{success, rolled_back}`. 프로비저닝에 실패한 상태였으면 **인스턴스를 삭제**한다 |
| GET | `/api/instance/:id/check-update` | `instance.rs:1235` | ✅ | - | 훅 결과 또는 `{update_available:false, reason}` |
| POST | `/api/instance/:id/apply-update` | `instance.rs:1329` | ✅ | - | `{success,id,provisioning:true}`. 409 `server_running`, 400 `not_steamcmd`/`no_install_config` |

`POST /api/instances` 동작 세부:

- `use_container:true`이면 `"docker"` 익스텐션이 활성화돼 있어야 한다. 이후 `server.post_create` 훅을 비동기로 실행한다.
- 컨테이너가 아니면 모듈당 1개만 허용한다.
- `migration_source`가 있으면 `import_settings` 플러그인을 **동기로** 호출한다.
- 모듈에 `install` 설정이 있으면 `lifecycle.py install_server`를 백그라운드로 실행한다(타임아웃 30분).

`PATCH /api/instance/:id` 동작 세부:

- 알려진 키(`port, rcon_port, rcon_password, rest_*, executable_path, protocol_mode, server_version, extension_data`)는 일급 필드로 반영한다.
- 나머지 키는 전부 `module_settings`로 들어간다.
- 이후 `manage_properties("write")`로 게임 설정 파일과 동기화하고, `extension_data`가 비어 있지 않으면 `server.settings_changed` 훅을 보낸다.

#### 명령 실행 (handlers/command.rs)

| Method | Path | Handler | Auth | 요청 | 응답 |
|---|---|---|---|---|---|
| POST | `/api/instance/:id/command` | `command.rs:12` | ✅ | `{command*, args?}` | `{success:true, message}` / 404 / 400 `{error}` |
| POST | `/api/instance/:id/rcon` | `command.rs:58` | ✅ | `{command*}` | `{success, data:{response,executed}, error:null, command, host, port, protocol:"rcon"}` / 400 / 500 |
| POST | `/api/instance/:id/rest` | `command.rs:217` | ✅ | `{endpoint*, method?, body?, timeout?, rest_host?, rest_port?, use_https?}` | `{success, data, error, endpoint, method, host, port, protocol:"rest"}` / 400 / 500 |

#### Managed 프로세스와 모듈 기능 (handlers/managed.rs)

| Method | Path | Handler | Auth | 요청 | 응답 |
|---|---|---|---|---|---|
| POST | `/api/instance/:id/managed/start` | `managed.rs:13` | ✅ | 선택 `{config?}` | supervisor 결과 / 404 / 500 |
| GET | `/api/instance/:id/console` | `managed.rs:73` | ✅ | query `since, count` | supervisor 결과 (`lines[{id,content,level}]`) / 404 |
| POST | `/api/instance/:id/stdin` | `managed.rs:94` | ✅ | `{command*}` | `{success, message}` / 400 / 500 |
| POST | `/api/instance/:id/validate` | `managed.rs:127` | ✅ | - | supervisor 결과 |
| GET | `/api/instance/:id/properties` | `managed.rs:144` | ✅ | - | `manage_properties("read")` |
| PUT | `/api/instance/:id/properties` | `managed.rs:161` | ✅ | `{settings}` | `manage_properties("write")` |
| POST | `/api/instance/:id/properties/reset` | `managed.rs:183` | ✅ | - | `"reset"` |
| POST | `/api/instance/:id/server/reset` | `managed.rs:200` | ✅ | - | `"reset_server"` (월드, 설정 삭제) |
| POST | `/api/instance/:id/accept-eula` | `managed.rs:220` | ✅ | - | supervisor 결과 |
| POST | `/api/instance/:id/diagnose` | `managed.rs:237` | ✅ | - | supervisor 결과 |
| GET | `/api/instance/:id/installed-version` | `managed.rs:312` | ✅ | - | 결과. 감지에 성공하면 `server_version`에 자동 저장 |

#### 설정 (handlers/bot.rs, handlers/config.rs)

| Method | Path | Handler | Auth | 요청 | 응답 |
|---|---|---|---|---|---|
| GET | `/api/config/bot` | `bot.rs:12` | ✅ | - | BotConfig JSON (**Discord 토큰 평문 포함**) |
| PUT | `/api/config/bot` | `bot.rs:18` | ✅ | 부분 JSON (얕은 머지) | `{success, message}` / 500 `{error}` |
| GET | `/api/config/gui` | `config.rs:16` | ✅ | - | GuiSettings JSON |
| PUT | `/api/config/gui` | `config.rs:22` | ✅ | 부분 JSON | `{ok:true}` / 500. `portConflictCheck`가 있으면 `supervisor.skip_port_check`도 갱신 |
| GET | `/api/config/node-token` | `config.rs:47` | ✅ | - | `{token}` (없으면 `""`) |
| PUT | `/api/config/node-token` | `config.rs:57` | ✅ | `{token}` | `{ok}` |
| DELETE | `/api/config/node-token` | `config.rs:83` | ✅ | - | `{ok}` |
| GET | `/api/system/components` | `config.rs:139` | ✅ | - | `{version, components:{key:ver}, lastUpdated}` (`<exe_dir>/release-manifest.json`) |
| POST | `/api/fs/scan-dir` | `config.rs:205` | ✅ | `{path}` | `{ok, files, dirs}` / 404 / 500. **임의 경로 열람 가능** |

#### 범용 외부 프로세스 (handlers/ext_process.rs)

| Method | Path | Handler | Auth | 요청 | 응답 |
|---|---|---|---|---|---|
| GET | `/api/ext-processes` | `ext_process.rs:495` | ✅ | - | `{processes:[{name,status,pid,meta}]}` |
| POST | `/api/ext-process/:name/start` | `ext_process.rs:287` | ✅ | `{command*, args?, cwd?, env?, meta?}` | `{ok, pid, name}` / 409 / 500. **임의 명령 실행** |
| POST | `/api/ext-process/:name/stop` | `ext_process.rs:312` | ✅ | - | `{ok}`. stdin으로 `{"type":"shutdown"}`을 보내고 1초 뒤 kill -9 / taskkill |
| GET | `/api/ext-process/:name/status` | `ext_process.rs:381` | ✅ | - | `{name,status:running/stopped/unknown,pid,meta}` |
| GET | `/api/ext-process/:name/console` | `ext_process.rs:413` | ✅ | `since, count=200` | `{name, lines:[{id,line}], running}` |
| POST | `/api/ext-process/:name/stdin` | `ext_process.rs:463` | ✅ | `{message}` | `{ok}` / 400 / 500 |

#### 클라이언트와 데몬 수명 (handlers/client.rs)

| Method | Path | Handler | Auth | 요청 | 응답 |
|---|---|---|---|---|---|
| POST | `/api/client/register` | `client.rs:25` | ✅ | `{kind:"gui"\|"cli"}` | `{client_id, kind, heartbeat_interval_ms:30000, timeout_ms:90000}` |
| POST | `/api/client/:id/heartbeat` | `client.rs:55` | ✅ | `{bot_pid?}` (바디 필수) | `{ok}` / 404 |
| DELETE | `/api/client/:id/unregister` | `client.rs:79` | ✅ | query `shutdown=true\|false` | `{ok}` / 404 |
| POST | `/api/daemon/shutdown` | `client.rs:12` | ✅ | - | `{ok, message}`. 200ms 뒤 토큰 cancel |
| GET | `/api/daemon/console` | `client.rs:169` | ✅ | `since, count=200` | `{lines:[{id,timestamp,level,target,message}], count}` |

`unregister`에 `shutdown=true`를 주면, 그 클라이언트의 봇을 kill하고 남은 클라이언트가 0이면 데몬도 종료한다.

#### 익스텐션 (handlers/extension.rs; 구현은 `extension/mod.rs`)

| Method | Path | Handler | Auth | 비고 |
|---|---|---|---|---|
| GET | `/api/extensions` | `extension.rs:60` | ✅ | `{extensions:[...]}` |
| GET | `/api/extensions/init-status` | `extension.rs:453` | ✅ | `{initializing, in_progress:{id:msg}, completed:[{ext_id,success,message,timestamp}]}` |
| POST | `/api/extensions/rescan` | `extension.rs:159` | ✅ | `{success, newly_found}` |
| GET | `/api/extensions/manifest` | `extension.rs:302` | ✅ | `{success, extensions, updates}` (원격) |
| GET | `/api/extensions/updates` | `extension.rs:400` | ✅ | `{success, updates, count}` |
| POST | `/api/extensions/:id/enable` | `extension.rs:72` | ✅ | `{success,id,startup_dispatched}`. `daemon.startup` 훅을 비동기 실행 |
| POST | `/api/extensions/:id/disable` | `extension.rs:130` | ✅ | 실행 중인 managed 인스턴스 참조 검사 |
| POST | `/api/extensions/:id/mount` | `extension.rs:176` | ✅ | |
| POST | `/api/extensions/:id/unmount` | `extension.rs:188` | ✅ | |
| POST | `/api/extensions/:id/install` | `extension.rs:333` | ✅ | 선택 `{download_url, sha256}`. 없으면 manifest에서 찾는다 |
| DELETE | `/api/extensions/:id` | `extension.rs:424` | ✅ | |
| GET/PUT | `/api/extensions/:id/config` | `extension.rs:492/505` | ✅ | `{config}` / 객체 머지(null이면 키 삭제) |
| GET | `/api/extensions/:id/gui` | `extension.rs:217` | ✅ | UMD JS 번들 (application/javascript) |
| GET | `/api/extensions/:id/gui/styles` | `extension.rs:249` | ✅ | text/css |
| GET | `/api/extensions/:id/icon` | `extension.rs:460` | ✅ | icon.png |
| GET | `/api/extensions/:id/i18n/:locale` | `extension.rs:278` | ✅ | JSON |

- 익스텐션 에러 형식: `{success:false, error, error_code, related}`.
- `error_code`별 상태 코드: `not_found|not_mounted|manifest_not_found`는 404, 의존성 계열은 422, `has_dependents|in_use`는 409, `id_mismatch`는 400(`extension.rs:25-57`).
- `/gui`, `/gui/styles`, `/icon` 라우트가 바로 **플러그인이 인터페이스에 컴포넌트를 주입하는 통로**다. GUI가 이 번들을 받아 렌더한다.

#### Relay 프록시

| Method | Path | Handler | Auth | 비고 |
|---|---|---|---|---|
| GET | `/api/relay/host/:host_id/status` | `relay.rs:187` | ✅ | query `relayUrl?`. relay `/api/hosts/:id` 프록시 (HMAC 서명) |
| GET | `/api/relay/host/:host_id/nodes` | `relay.rs:199` | ✅ | `/api/hosts/:id/nodes` |
| GET | `/api/relay/node/:guild_id/members` | `relay.rs:212` | ✅ | `/api/nodes/:id/discord-members`, 503이면 `/members`로 폴백 |
| GET | `/api/relay/bot-invite` | `relay.rs:240-259` | ✅ | query `relayUrl?`. relay `/api/bot-invite`(공개, 서명 없음)를 그대로 전달. 봇 초대 URL `{url}` |
| POST | `/api/relay/pair/initiate` | `relay.rs:264` | ✅ | 바디를 그대로 `/api/pair/initiate`에 전달 (서명 없음) |
| GET | `/api/relay/pair/:code/status` | `relay.rs:297` | ✅ | `secret?, relayUrl?`. `status:"claimed"`이면 `nodeToken`을 `.node_token`에 자동 저장 |

Relay 서명 방식(`relay.rs:78-113`):

- 헤더: `Authorization: Bearer <node_token>`, `x-request-timestamp`, `x-request-nonce`, `x-request-signature`.
- 서명값: `HMAC-SHA256(secret, "METHOD\npath\nts\nnonce\nbody")`.
- 고정 헤더 `X-Saba-Client: 1`이 모든 relay 요청에 붙는다. timeout은 5초다.

#### 업데이트 (ipc/updates.rs), **인증 없음**

| Method | Path | Handler | 요청 | 응답 |
|---|---|---|---|---|
| GET | `/api/updates/status` | `updates.rs:104` | - | `{ok,last_check,next_check,checking,error,updates_available,components:[{component,display_name,current_version,latest_version,update_available,downloaded,installed}]}` (Locales 제외) |
| POST | `/api/updates/check` | `updates.rs:144` | - | `{ok,updates_available,update_names,components(+release_notes),last_check}`. Locales 업데이트가 있으면 백그라운드에서 조용히 적용 |
| POST | `/api/updates/download` | `updates.rs:252` | `{components?:[key]}` | `{ok,downloaded,count,errors?}` |
| GET | `/api/updates/download/progress` | `updates.rs:332` | - | `{ok,component,bytes_received,total_bytes,active}` |
| POST | `/api/updates/apply` | `updates.rs:357` | `{components?:[key]}` | `{ok,applied,needs_updater,requires_updater,errors}` |
| GET | `/api/updates/integrity` | `updates.rs:478` | - | `{ok,checked_at,overall,total,verified,failed,skipped,components[...]}` |
| GET | `/api/updates/config` | `updates.rs:518` | - | `{ok, config}` |
| PUT | `/api/updates/config` | `updates.rs:730` | 부분 `{enabled,check_interval_hours(1-8),auto_download,auto_apply,install_root,relay_url,channel}` | `{ok, config}`. `config.json`의 `updates` 절에 저장한다(`:761, 791-797`) |

`python_env` 핸들러(`handlers/python_env.rs`의 `/api/python-env/status|setup|pip-install`)는 **함수만 있고 라우터에 등록되지 않았다**(`#[allow(dead_code)]`, §9).

---

## 3. 인증 모델 (`src/ipc/auth.rs`)

- **토큰 생성**: 데몬이 기동할 때마다 `uuid::Uuid::new_v4().to_string()`(36자)를 새로 만든다(`auth.rs:28`).
- **저장**
  - `constants::token_file_path()`, 기본값은 `<data_dir>/.ipc_token`이고 `SABA_TOKEN_PATH`로 바꿀 수 있다.
  - Unix에서는 퍼미션을 0600으로 둔다.
  - 프로세스 전역 `static CACHED_TOKEN: RwLock<Option<String>>`에도 캐시한다(`auth.rs:17`).
- **검증**: 헤더 `X-Saba-Token` 값과 정확히 일치하는지 비교한다(`auth.rs:106-112`).
  - 상수 시간 비교가 아니다.
  - 불일치하면 본문 없이 401을 돌려준다.
  - 실패 로그는 30초에 한 번으로 제한하고, 토큰 앞 8자를 로그에 남긴다.
- **우회 경로**
  - `SABA_AUTH_DISABLED=1`이면 전체를 통과시킨다. 통합 테스트가 이 방식을 쓴다(`tests/daemon_integration.rs:111`).
  - 캐시도 없고 토큰 파일도 없으면 "하위 호환"이라며 **인증 없이 통과**시킨다(`auth.rs:95-101`).
  - 토큰 생성이 실패해도(`main.rs:132`) 메모리 캐시가 비어 있으므로, 이전 실행의 파일이 남아 있지 않다면 사실상 인증이 꺼진다.
- **토큰을 쓰는 클라이언트**: GUI(`saba-chan-gui/main.js:62,160`, 파일 재읽기 포함), CLI(`saba-chan-cli/src/config.rs:16`), Discord 봇(`discord_bot/core/ipc.js:33,82`). 셋 다 토큰 파일을 직접 읽어 헤더에 넣는다. 401을 받으면 파일을 다시 읽는다.
- **CORS 레이어가 없다.** 브라우저에서 오는 교차 출처 요청은 사전 요청(preflight)에서 막힌다. 단, **인증이 없는 `/api/updates/*`는 단순 POST(바디 없는 `check`, `download`, `apply`)를 CSRF 방식으로 호출당할 수 있다.**
  - `apply`에 바디가 없으면 axum `Json` 추출기가 Content-Type 오류로 거부하므로 그 경우는 막힌다.
  - 바디가 없는 `POST /api/updates/check`는 받아들여진다.

---

## 4. 인스턴스 모델

### 4.1 `ServerInstance` (`src/instance/mod.rs:23-58`)

| 필드 | 타입 | 비고 |
|---|---|---|
| `id` | String | uuid v4 |
| `name` | String | 유일성 검사 없음. `/api/server/:name/*`는 첫 번째로 일치하는 인스턴스를 쓴다 |
| `module_name` | String | |
| `executable_path`, `working_dir` | Option<String> | |
| `auto_detect` | bool | 기본 true |
| `process_name` | Option<String> | 모듈 metadata에서 채운다 |
| `port`, `rcon_port` | Option<u16> | |
| `rcon_password` | Option<String> | 평문 |
| `rest_host`, `rest_port`, `rest_username`, `rest_password` | Option | `rest_username`은 항상 "admin"으로 강제한다(`instance/mod.rs:204`) |
| `protocol_mode` | String | 기본 `"auto"` |
| `module_settings` | HashMap<String,Value> | settings.json으로 분리 저장 |
| `server_version` | Option<String> | |
| `extension_data` | HashMap<String,Value> | `docker_enabled`, `install_method_steamcmd`, `steamcmd_app_id`, `steamcmd_anonymous`, `steamcmd_beta`, `steamcmd_platform` 등 |
| `required_extensions` | Vec<String> | 모듈의 `install.requires_extensions`를 복사한다 |

- `ensure_passwords()`(`instance/mod.rs:189`)는 시작할 때(`server.rs:601`, `managed.rs:22`) 비어 있는 RCON/REST 비밀번호를 16자 영숫자로 채운다.
- 비밀번호 생성은 `constants::generate_random_password`가 uuid v4 바이트에 `% 62`를 적용하는 방식이다(`constants.rs:240-253`). 모듈로 편향이 있고, 두 번째 uuid는 계산만 하고 쓰지 않는다.
- `GuiSettings.autoGeneratePasswords`는 이 동작에 **반영되지 않는다.**

### 4.2 영속화 (`InstanceStore`, `instance/mod.rs:223-434`)

- 루트는 `constants::resolve_instances_dir()`(`supervisor/mod.rs:56`)이며, 기본값은 `<data_dir>/instances`다.
- 저장 형식(JSON, pretty):
  - `<root>/order.json`: id 배열
  - `<root>/<id>/instance.json`: `module_settings`를 뺀 메타데이터
  - `<root>/<id>/settings.json`: `module_settings`
  - Unix에서는 두 파일 모두 0600으로 둔다.
- 로드할 때 `order.json` 순서로 정렬하고, 목록에 없는 인스턴스는 뒤에 붙인다(`instance/mod.rs:243-302`).
- `remove()`는 `<root>/<id>` 디렉토리를 통째로 지운다. 기본 설치 위치인 `server/` 하위 게임 서버 파일도 **함께 지워진다.**
- 메모리 캐시와 디스크를 동시에 갱신한다. 원자적 쓰기(임시 파일 후 rename)는 없다.

### 4.3 생명주기

1. **생성**: `POST /api/instances`. 세 갈래로 나뉜다.
   - 컨테이너: `server.post_create` 훅
   - 네이티브 설치: `lifecycle.py list_versions`, 이어서 `install_server`
   - 마이그레이션: `import_settings`
2. **프로비저닝 추적**: `ProvisionTracker`가 **인스턴스 이름**을 키로 쓴다(`ipc/mod.rs:345`).
   - 성공하면 5초 뒤 자동으로 지운다.
   - 실패하면 사용자가 dismiss할 때까지 남겨 두고, dismiss하면 인스턴스를 롤백(삭제)한다.
3. **시작**: 경로가 두 개다.
   - `/api/server/:name/start`: native/supervisor `start_server`
   - `/api/instance/:id/managed/start`: stdio를 캡처하는 managed 방식
4. **모니터링**: 2초 루프(`main.rs:221`)가 PID와 패턴을 매칭해 auto-detect한다. 세부는 supervisor 담당 문서에 있다.
5. **정지**: `/api/server/:name/stop`. 성공하면 RCON 풀에서 연결을 지우고 tracker에서 untrack한다. 단, `docker_enabled`인 인스턴스는 untrack을 건너뛴다(`server.rs:683`).
6. **삭제**: 실행 중이면 force kill(2초 대기), `server.pre_delete` 훅, 디렉토리 삭제(최대 3회 재시도) 순으로 진행한다(`instance.rs:779-866`).

### 4.4 config_store와 전역 설정 스키마 (`src/config_store/mod.rs`)

- `ConfigStore`는 `settings.json`과 `bot-config.json`을 메모리에 캐시하고 쓸 때 파일로 flush한다(`config_store/mod.rs:176-275`). 기동할 때 한 번 로드한다(`ipc/mod.rs:546`).
- PUT 동작: 현재 값을 JSON으로 바꾼 뒤 요청값을 **얕게** 머지하고(`merge_json`, `:314`), 강타입으로 역직렬화해 저장한다.
- 스키마에 없는 키는 **저장할 때 버려진다.** 예를 들어 레거시 `discordToken`이나 GUI가 임의로 넣은 키가 그렇다.

**GuiSettings** (camelCase, `config_store/mod.rs:23-48`)

| 그룹 | 필드 (기본값) |
|---|---|
| 동작 | `autoRefresh`(true), `refreshInterval`(2000), `ipcPort`(57474), `consoleBufferSize`(2000), `autoGeneratePasswords`(true), `portConflictCheck`(true) |
| 테마 | `accentColor`("#667eea"), `accentSecondary`("#764ba2"), `useGradient`(true), `fontScale`(100), `enableTransitions`(true), `consoleSyntaxHighlight`(true), `consoleBgColor`, `consoleTextColor`, `sidebarCompact`(false), `windowOpacity`(100) |
| 창/언어 | `windowBounds?{width:1200,height:840,x?,y?}`, `language?` |

**BotConfig** (camelCase, `config_store/mod.rs:115-128`). 로컬 봇용 필드(`token`, `autoStart`, `mode`, `music*`)는 지워졌고, 옛 파일에 남아 있어도 무시된다.

- `prefix`("!saba"), `cloud{relayUrl,hostId}`
- `moduleAliases`, `commandAliases`
- `instanceAliases`: 인스턴스 ID → 그 서버를 부르는 별명 목록(`"크리, cr"`). 같은 게임 서버가 여럿일 때 하나를 고른다. 채팅 처리기는 인스턴스 이름 → 인스턴스 별명 → 모듈 별명 순으로 찾고, 여러 서버가 걸리면 `errors.multiple_instances`로 답한다(`src/chat/mod.rs:256-285`)
- `nodeSettings`, `cloudNodes`, `cloudMembers`

**데몬이 실제로 쓰는 설정값은 일부뿐이다.**

- `portConflictCheck`: PUT할 때 `skip_port_check` 원자 변수를 갱신한다. 이 변수를 읽는 곳은 모니터 루프 한 곳뿐이다(`supervisor/mod.rs:1805`). 기동할 때 `settings.json`에서 초기화하지 않는다.
- `language`: 봇 자동 시작에서 쓴다.
- `ipcPort`: **데몬은 읽지 않는다.** GUI가 이 값을 `SABA_IPC_PORT`로 넘겨 스폰할 때만 반영된다(`saba-chan-gui/main.js:556-560`).
- `autoGeneratePasswords`: 미사용.

`src/config/mod.rs`의 `GlobalConfig{ipc_socket:"./ipc.sock", log_buffer_size}`는 어디서도 쓰지 않는 잔재다.

---

## 5. 프로토콜 계층 (`src/protocol/**`)과 명령 라우팅

### 5.1 RCON (`protocol/rcon.rs`)

- Source RCON 방식을 동기 `std::net::TcpStream`으로 구현했다.
- 패킷 구조: `[size][id][type][payload\0\0]`. 전체를 한 번의 write로 보낸다(`rcon.rs:143-173`).
- 인증 응답 처리
  - 1개 또는 2개 응답을 모두 받는다. 두 번째 응답은 500ms 타임아웃으로 기다린다.
  - 인증 후 소켓이 살아 있는지 peek로 확인한다(`rcon.rs:64-131`).
- 제약
  - 응답이 4096바이트를 넘으면 에러로 처리한다(`rcon.rs:200`). 여러 패킷으로 나뉜 응답은 지원하지 않는다.
  - `TcpStream::connect`에 연결 타임아웃이 없다(`rcon.rs:37`).

### 5.2 RconPool (`protocol/rcon_pool.rs`)

- instance_id마다 연결 하나를 재사용한다. host, port, password가 바뀌거나 연결이 끊기거나 명령이 실패하면 재연결한다.
- `std::sync::Mutex`를 **명령 실행 I/O(최대 5초)가 끝날 때까지** 잡는다(`rcon_pool.rs:52-91`). 그래서 모든 인스턴스의 RCON 요청이 직렬화된다. 호출 쪽은 `spawn_blocking`으로 감싼다(`command.rs:166`).

### 5.3 REST (`protocol/rest.rs`)

- `ureq`(동기)를 쓰고 timeout은 10초다. Basic Auth를 지원한다.
- 연결이 EOF나 reset으로 끊기면 성공으로 간주한다. Palworld 대응 코드다(`rest.rs:110-127`).
- `verify_connection`은 실제 연결 없이 host와 port 유효성만 본다(`rest.rs:50-56`).
- Basic 토큰 앞 8자와 비밀번호 길이를 **info 레벨로 로그에 남긴다**(`rest.rs:94`).

### 5.4 ProtocolClient (`protocol/client.rs`)

- `Rcon`, `Rest`, `Both`(RCON 실패 시 REST로 폴백) 세 모드를 지원한다.
- 실제로는 `new_rest`만 쓰인다(`command.rs:323`). `new_rcon`, `new_both`, `connect_all`은 `#[allow(dead_code)]`다.

### 5.5 명령 라우팅

| 경로 | 동작 |
|---|---|
| `POST /api/instance/:id/command` | `Supervisor::execute_command`(`supervisor/mod.rs:865`)가 `lifecycle.py command`를 config `{command,args,protocol_mode,working_dir,rcon_host:"127.0.0.1",rcon_port,rest_host,rest_port,pid,instance_id}`로 실행한다. **실제 RCON/REST 호출은 Python 모듈이 한다.** `docker_enabled` 인스턴스는 거부한다(`:884`) |
| `POST /api/instance/:id/rcon` | `lifecycle.py get_credentials`(working_dir)로 **게임 설정 파일에서** `rcon_port`, `rcon_password`를 읽고, `127.0.0.1`로 RconPool을 실행한다. 인스턴스에 저장된 `rcon_password`는 무시하며, 포트만 폴백으로 쓴다(`command.rs:137-157`) |
| `POST /api/instance/:id/rest` | `get_credentials`로 `rest_username`, `rest_password`를 얻는다. host와 port는 payload, instance, 모듈 기본값 순으로 정하고 ProtocolClient REST로 실행한다. **동기 ureq를 async 핸들러 안에서 직접 호출하므로**(`command.rs:371`) tokio 워커 스레드를 최대 10초 막는다 |
| `POST /api/instance/:id/stdin` | managed 프로세스의 stdin에 쓴다(supervisor) |

---

## 6. 내장 런타임: python_env

### 6.1 Python (`src/python_env/mod.rs`)

- 버전: 포터블 CPython **3.12.8**(python-build-standalone 태그 `20250106`, `install_only_stripped`)이다. 시스템 Python을 쓸 경우 3.10 이상이어야 한다(`:42-46`).
- 탐색 순서: 이미 받은 포터블 Python, 시스템 Python, 자동 다운로드 순이다(`find_base_python`, `:236`). 찾은 인터프리터로 `<pyroot>/python-env` venv를 만들고 pip를 업그레이드한다(`ensure_venv`, `:70`).
- 결과는 `OnceCell`에 캐시한다(`get_python_path`, `:57`). 모든 `run_plugin` 호출이 여기를 거친다(`plugin/mod.rs:125`).
- 다운로드 URL: `https://github.com/indygreg/python-build-standalone/releases/download/{tag}/cpython-{ver}+{tag}-{triple}-install_only_stripped.tar.gz`. 지원 triple은 win x64, linux x64/arm64, mac x64/arm64다.
- 다운로드 방식: Windows는 PowerShell `Invoke-WebRequest`, 그 외는 `curl`이다. 압축 해제는 `tar`를 쓴다. **체크섬 검증은 없다.**
- **`<pyroot>`는 `constants::resolve_data_dir()`와 다르다**(`python_env/mod.rs:446-484`). 순서는 다음과 같다.
  1. `SABA_DATA_DIR`
  2. **exe 디렉토리에 쓰기 권한이 있으면 exe 디렉토리**
  3. Windows `%APPDATA%/saba-chan`, Linux `~/.local/share/saba-chan`, macOS `~/Library/Application Support/saba-chan`
- 결과적으로 Linux에서 데이터는 `~/.config/saba-chan`에, 런타임은 `~/.local/share/saba-chan` 또는 exe 옆에 흩어진다.
- `status()`(`:184`) 응답 필드: `{available, venv_dir, venv_python, portable_python_installed, portable_python_path, python_version_bundled, python_version?, portable_download_url?, system_python}`. 다만 HTTP 라우트는 없다.

### 6.2 Node

데몬은 더 이상 Node를 받거나 찾지 않는다. `node_env`와 `/api/node-env/*`는 로컬 봇과 함께 L1(7b5102f)에서 지웠다. Node는 GUI 빌드에만 쓰며 `>=22.12`가 필요하다(`saba-chan-gui/package.json` `engines`, 04 §1).

---

## 7. plugin/과 validator/

### 7.1 `src/plugin/mod.rs`: Python 모듈/익스텐션 실행기 (단발 프로세스)

**호출 계약**

- 실행 형태: `<venv python> <module_path> <function>`. stdin으로 config JSON을 한 번 쓰고 닫는다.
- 결과: **stdout 전체가 하나의 JSON**이어야 한다. 종료 코드가 0이 아니면 Err다. 파싱에 실패해도 Err다.
- 진행률: stderr의 `PROGRESS:{json}` 줄을 `ExtensionProgress{percent,message,step,total,label,steps}`로 파싱해 콜백에 넘긴다(`:7-20, 185-205`). 그 밖의 stderr는 debug 로그로만 남긴다.
- 타임아웃: 기본 120초(`DEFAULT_PLUGIN_TIMEOUT_SECS`), install 계열은 1800초다. 시간이 넘으면 `child.kill()`한다.

**자식 프로세스 환경**

- `PYTHONPATH` = `<extensions_dir의 부모>` + `<modules_dir>` + 기존 값(`:153-173`). `extensions_dir`가 디렉토리일 때만 설정한다.
- `SABA_ENABLED_EXTENSIONS` = 쉼표로 연결한 활성 익스텐션 목록. 이 경로를 **잘못 해석한다**(§9 B-12).

**모듈의 `lifecycle.py`가 응답해야 하는 함수** (코어에서 직접 호출하는 것만)

- `list_versions` → `{latest:{release}}` 또는 `{latest_release}`
- `install_server` → `{success, executable_path?, install_path?, message?}`
- `import_settings` → `{success, settings{}}`
- `get_credentials` → `{success, rcon_port?, rcon_password?, rest_username?, rest_password?}`
- `command` → `{success, message}`
- 나머지(start/stop/status/properties/eula/diagnose 등)는 supervisor가 호출한다.

`run_extension*`(`extensions/<name>.py` 경로)는 dead code다. 실제 익스텐션 훅 실행은 `extension/mod.rs`가 맡는다.

### 7.2 `src/validator/mod.rs`

- `validate_setting_value`, `validate_all_settings`(`:70, 230`)는 module.toml `SettingField`의 `field_type`으로 값을 검사한다.
  - `number`: min/max 범위
  - `boolean`: true/false 또는 문자열 "true"/"false"
  - `select`: `options`에 포함되는지
  - `text|password|file`: 문자열(숫자, 불리언도 허용)
  - 알 수 없는 타입은 통과시킨다.
  - `required==Some(true)`인데 값이 없거나 빈 문자열이면 에러다.
- 사용처는 `PATCH /api/instance/:id` 하나뿐이다(`instance.rs:889`).
  - 문제: `validate_all_settings`는 **모듈의 모든 필드를 검사**한다. 그래서 부분 PATCH에 required 필드가 빠져 있으면 400을 받는다. 부분 업데이트가 사실상 불가능하다.
- `check_port_conflicts`(`:261`)는 **실행 중인** 다른 인스턴스와 포트가 겹치는지 검사한다. 모듈이 지원하는 프로토콜의 포트만 본다. supervisor의 start 경로와 모니터에서 쓴다.
- `GET /api/servers`의 `port_conflicts` 계산(`server.rs:225-257`)은 이 함수를 쓰지 않는다. **실행 여부와 프로토콜 지원 여부를 따지지 않고 모든 인스턴스를 비교**하므로 판정 기준이 다르다.

### 7.3 기타

- `process_monitor.rs`는 `sysinfo` 0.30으로 전체 프로세스 스냅샷(`pid,name,exe,cmd`)을 만든다. `spawn_blocking`으로 감싼 async 버전도 있다.
- `daemon_log.rs`는 tracing Layer와 링 버퍼(10k)를 제공하고, `get_since(since,count=200)`로 조회한다.
- `utils.rs`는 Windows `CREATE_NO_WINDOW` 플래그만 다룬다.

---

## 8. updates.rs: 업데이트 엔드포인트와 외부 URL

- `UpdateState`가 `UpdateManager`(updater 크레이트)를 `Arc<RwLock>`으로 감싼다. 설정은 `config.json`의 `updates` 절에서 읽고(`load_updater_config`, `updates.rs:777-789`), 없거나 깨졌으면 `UpdateConfig::default()`를 쓴다.
  - 필드(`updater/src/lib.rs:251-266`): `enabled`(기본 true), `check_interval_hours`(3), `auto_download`(false), `auto_apply`(false), `install_root`, `relay_url`(None이면 `https://saba-chan.online`), `channel`(`stable`).
- `PUT /api/updates/config`는 바꾼 설정을 `config.json`에 저장한다(`updates.rs:761`).
- 주기 확인은 데몬의 `spawn_update_scheduler`가 한다(`updates.rs:246`). `check_interval_hours`마다 확인하고 `auto_download`이면 받아 둔다.
- **접속하는 외부 URL**
  - 릴레이: `GET {relay_url}/v1/updates/check?channel&platform&current&client_id`, `GET /v1/updates/manifest/:version`, `POST /v1/updates/event`(익명 진행 이벤트). 모든 요청에 `X-Saba-Client: 1` 헤더(`updater/src/relay.rs:1-25, 158-192`).
  - manifest의 각 컴포넌트 `url`(R2 공개 주소). 받을 때 `sha256`을 확인한다(`updater/src/download.rs:40`).
  - manifest는 번들된 공개키로 Ed25519 서명을 검증하고, 플랫폼이 다르거나 현재보다 낮은 버전이면 거부한다(`relay.rs:229-250`, `signature.rs`).
  - `/api/modules/manifest*`, 익스텐션 목록(`ExtensionManager::fetch_remote`), 설치기는 같은 함수 `relay::latest_catalog`를 쓴다(`relay.rs:109-115`). 이 함수는 기본 릴레이와 `stable` 채널을 쓰며 `relay_url` 설정을 보지 않는다.
  - Python 런타임만 예외로 `github.com/indygreg/python-build-standalone` 릴리즈에서 받는다(`python_env`).
- **apply 규칙**(`updates.rs:357-475`)
  - 대상에 GUI, Updater, CoreDaemon 중 하나라도 있으면 **전부** 업데이터 exe에 맡긴다. 이때 `needs_updater`를 채우고 `save_pending_manifest`, `save_updater_apply_targets`를 호출한다.
  - 그렇지 않으면 데몬이 직접 적용하고, 모듈은 refresh, 익스텐션은 rescan으로 핫 리로드한다.
  - 파일 상단 주석에는 "업데이터 exe 스폰"이라고 돼 있지만, **데몬은 업데이터를 스폰하지 않는다.** `requires_updater:true`를 돌려줄 뿐이고, 실행은 클라이언트 몫이다.
- `/api/updates/*`도 다른 라우트처럼 토큰 인증을 거친다(`ipc/mod.rs:749-751`). `relay_url`을 바꿔도 manifest 서명 검증은 그대로라 다른 서버가 파일을 바꿔 끼울 수는 없다.

---

## 9. 불일치, 데드 코드, 반쯤 구현된 기능, 위험 지점

### 9.A 오너 원칙 위반 (인터페이스가 데몬을 우회하는 곳)

1. **GUI가 `settings.json`을 직접 읽고 쓴다.**
   - `saba-chan-gui/main.js:440-460`의 `loadSettings`가 직접 읽는다.
   - `main.js:483-490`은 "데몬이 아직 시작되지 않은 부트스트랩 단계"라는 이유로 `fs.writeFileSync`로 직접 쓴다.
   - 데몬 ConfigStore는 기동할 때 한 번만 로드하므로, 데몬이 떠 있는 동안 GUI가 파일을 직접 쓰면 메모리 캐시와 어긋날 수 있다.
2. **GUI가 relay 서버를 직접 호출한다.** `main.js:3216, 3406`의 `axios.patch(${relayUrl}/api/hosts/${hostId}/bot-config)`가 그렇다. 이는 데몬 `relay.rs:3-4`의 "GUI/CLI must not access the relay server directly"와 정면으로 모순된다.
3. **봇 실행 로직이 GUI에 중복돼 있다.**
   - GUI가 직접 명령, 인자, 환경변수(`DISCORD_TOKEN`, `RELAY_NODE_TOKEN`, `BOT_CONFIG_PATH` 등)를 조립해 범용 `/api/ext-process/:name/start`로 보낸다(`main.js:3227-3260`).
   - 같은 로직이 데몬의 `auto_start_discord_bot`(`main.rs:669`)에도 있다. 두 구현이 따로 진화할 위험이 있다. 데몬 쪽은 `discord_bot` 경로를 `constants::resolve_discord_bot_dir()`로, GUI 쪽은 `__dirname/../discord_bot`으로 정한다.
   - 비밀값(Discord 토큰)이 인터페이스를 거쳐 다시 데몬으로 전달된다.
4. **GUI가 데몬의 경로 정책을 결정한다.** `main.js:556-563`에서 `SABA_INSTANCES_PATH`, `SABA_MODULES_PATH`, `SABA_IPC_PORT`를 GUI가 정해 주입한다.
5. **CLI가 로컬 파일을 직접 읽는다.**
   - `saba-chan-cli/src/config.rs:99`의 `get_ipc_port_local`이 `settings.json`의 `ipcPort`를 직접 읽는다. 주석에 "유일한 로컬 파일 직접 읽기"라고 적혀 있다.
   - 데몬이 실제로 쓰는 포트(`SABA_IPC_PORT`)와 다를 수 있다.
   - `scan_directory`와 `get_module_locales`는 데몬에 연결되지 않으면 로컬 fs로 폴백한다(`config.rs:427-466, 468-499`).
   - `saba-chan-cli/src/gui_config.rs`는 `config.rs`와 거의 같은 사본인데 `main.rs`에 `mod`로 선언돼 있지 않다. 컴파일되지 않는 잔재다.
6. **Discord 봇(server-chan)이 `bot-config.json`을 파일 경로(`BOT_CONFIG_PATH`)로 받는다.** `/api/config/bot`을 쓰지 않는다. music 익스텐션도 `SABA_EXTENSIONS_DIR`를 기준으로 파일을 직접 다룬다(`discord_bot/extensions/music.js:37-161`).
7. **토큰 파일 직접 읽기**(GUI, CLI, 봇)는 인증 부트스트랩 구조상 피할 수 없으므로 예외로 본다.
8. **반대 방향 위반: 코어 안에 특정 익스텐션 이름이 하드코딩돼 있다.** "익스텐션이 기능을 더한다"는 원칙과 달리, 코어가 특정 익스텐션을 알고 있다.
   - `"docker"`, `"docker_enabled"`: `instance.rs:98,112,121,171`, `server.rs:683`, `supervisor/mod.rs:884`
   - steamcmd 키: `instance/mod.rs:121-185`, `instance.rs:1287,1364`
   - `extension/mod.rs`의 "범용" 주석과 맞지 않는다.

### 9.B 보안과 정합성

1. ~~`/api/updates/*`에 인증이 없다~~ 해결됨: 업데이트 라우터도 인증 미들웨어 안에 있다(`ipc/mod.rs:749-751`). 업데이트 출처 설정은 `relay_url`뿐이고, 서명되지 않은 manifest는 받지 않는다.
2. **`/health`는 인증이 필요하다.** 문서와 주석이 말하는 "경량 ping"과 다르다. 클라이언트의 ready 체크가 토큰을 읽기 전에 실행되면 401을 받는다.
3. **토큰이 없으면 fail-open이다**(`auth.rs:95-101`). 비교도 상수 시간이 아니다.
4. **단일 인스턴스 보장이 없다.** 그 결과 토큰 덮어쓰기와 Windows `SO_REUSEADDR` 이중 바인드가 생긴다(§1.6).
5. **`/api/ext-process/:name/start`는 임의 명령을 실행**하고, **`/api/fs/scan-dir`는 임의 디렉토리를 열람**한다. 토큰만 있으면 쓸 수 있으며, 설계상 의도된 기능이지만 위험 범위가 넓다.
6. **relay 토큰 유출 가능성**: `?relayUrl=`을 호출자가 지정할 수 있는데, 서명 헤더에는 `Authorization: Bearer <node_token>` 원문이 들어간다(`relay.rs:38-43, 107-108`). 따라서 임의 호스트로 노드 토큰을 보낼 수 있다.
7. **`POST /api/modules/manifest/:id/install`에 경로 검사가 없다**(`server.rs:460-516`). `remove_module`(`:524`)에는 있다. `get_module_locales`도 이름을 그대로 `join`한다(`config.rs:296`).
8. **비밀값이 평문으로 응답에 실린다.** `GET /api/instances`, `/api/servers`에는 RCON/REST 비밀번호가, `/api/config/bot`에는 Discord 토큰이 들어 있다. REST Basic 토큰 앞부분과 REST 비밀번호 앞 4자는 로그에 남는다(`rest.rs:94`, `command.rs:262-263`).
9. **익스텐션 설치에 사용자 지정 `download_url`을 허용하고 sha256은 선택**이다(`extension.rs:339-349`). 포터블 Python/Node 다운로드에는 체크섬 검증이 아예 없다.
10. **프로비저닝 dismiss 롤백 경쟁**: `apply-update`가 실패한 직후 5초 안에 `DELETE /api/provision-progress/:name`을 호출하면, 기존 인스턴스가 "프로비저닝 실패"로 간주돼 **삭제된다**(`instance.rs:716-744`, `1558-1564`).
11. **`/api/daemon/shutdown`과 `unregister?shutdown=true`는 토큰만 cancel한다**(`client.rs:17-20, 100-104`). Ctrl+C 경로가 하는 정리(`daemon.shutdown` 훅, ext-process `shutdown_all`, 봇 kill, `main.rs:400-458`)를 **거치지 않는다.** daemon-only 모드에서 API로 종료하면 봇 같은 자식 프로세스가 고아로 남을 수 있다.
12. **`SABA_ENABLED_EXTENSIONS` 경로 해석 버그**: `plugin/mod.rs:140-146`이 `APPDATA`를 직접 쓰고, 없으면 `./extensions_state.json`(cwd 기준)을 본다.
    - 실제 파일은 `constants::resolve_extensions_state_path()`(`extension/mod.rs:452`)에 있다.
    - 그래서 Linux/macOS나 `SABA_DATA_DIR`를 쓰는 환경에서는 이 변수가 **설정되지 않는다.**
13. **락을 오래 잡는 문제**
    - `execute_command`, `manage_properties`, `diagnose`, `install_server` 등은 supervisor **read lock**을 잡은 채 Python 플러그인을 실행한다(최대 120초 이상, `command.rs:17-36`, `managed.rs` 전반).
    - `create_instance`의 마이그레이션 분기는 **write lock**을 잡은 채 `import_settings`를 실행한다(`instance.rs:72, 653`).
    - `start_managed_handler`도 write lock을 잡고 시작한다(`managed.rs:57`).
    - 2초 주기 모니터가 write lock을 기다리는 동안 다른 요청도 줄줄이 막힌다.
    - `install_extension`은 익스텐션 매니저 write lock을 잡은 채 네트워크 다운로드를 한다(`extension.rs:376-377`).
14. **REST 명령이 async 핸들러 안에서 동기로 실행된다**(`command.rs:371`). `spawn_blocking`이 없다.
15. **ext-process 출력이 stderr로 새어 나간다.** stdout/stderr를 `eprintln!`으로 그대로 내보낸다(`ext_process.rs:230, 248, 266`). CLI 부트 모드에서는 tracing을 파일로 돌렸는데도 이 출력이 TUI 화면을 덮을 수 있다.

### 9.C 데드 코드와 반쯤 구현된 기능

- `src/config/mod.rs`의 `GlobalConfig`(`ipc.sock` 언급 포함)는 미사용이다(`main.rs:114-115`).
- `handlers/python_env.rs`의 세 핸들러는 라우트가 없다. `python_env::pip_install`, `pip_install_requirements`도 `#[allow(dead_code)]`다.
- `plugin::run_extension*`, `ProtocolClient::new_rcon/new_both/connect_all/connect_rcon`, `RestClient::with_basic_auth`, `ServerResponse::success/error`, `CommandResponse`, `BotConfig`(ipc/mod.rs:188), `ExtensionInitTracker::is_initializing`, `InstanceStore::save`, `ServerInstance::ext_f64/ext_str` 일부, `process_monitor::find_by_name*` 동기 버전은 모두 미사용이다.
- 업데이트 설정의 `check_interval_hours`, `auto_download`, `auto_apply`는 저장만 되고 아무 동작이 없다. 설정도 재시작하면 사라진다.
- `skip_port_check` 계열이 따로 논다.
  - GUI 설정은 `Supervisor.skip_port_check` 원자 변수로 가는데, 이 변수는 모니터만 읽는다.
  - start 경로는 요청 바디 `config.skip_port_check`를 읽는다(`supervisor/mod.rs:236, 996`).
  - 기동할 때는 `settings.json`에서 초기화하지 않는다.
- `GuiSettings.ipcPort`와 `autoGeneratePasswords`는 데몬에서 효과가 없다(§4.4).
- 비 Windows 부트 선택기는 키 입력을 지원하지 않는데도 DEL 안내를 표시한다(`boot_selector.rs:253, 317`).
- `ServerStartRequest.module`이 필수 필드다(`ipc/mod.rs:160-164`). 인스턴스에 이미 모듈 정보가 있는데도 클라이언트가 다시 보내야 하므로 중복이다.
- `python_env`의 데이터 디렉토리가 `constants::resolve_data_dir()`와 어긋난다(§6). 같은 파일 상단 주석은 `<data_dir>`라고 적고 있다.
- `install_server` 진행 콜백은 `step/total`을 그대로 넘기지만, 완료될 때는 `step:0,total:1`로 덮어쓴다. 백그라운드 설치의 진행 단계 정보가 사라진다.
- `CLAUDE`/문서 주장 검증: `src/ipc/updates.rs:10`의 doc-claim "업데이터 exe 스폰하여 적용"은 **코드와 다르다.** 응답 플래그만 돌려준다. 그 밖의 README/docs 내용은 이 문서에서 근거로 쓰지 않았다.

### 9.D 테스트 계약 (`tests/daemon_integration.rs`)

- `boot_ipc()`(`:110-145`)는 `SABA_AUTH_DISABLED=1`로 인증을 끄고, 임시 instances 디렉토리와 `./modules`를 쓴다.
  - 그러나 `IPCServer::new`는 실제 `<data_dir>`의 ConfigStore와 익스텐션 디렉토리를 그대로 읽고 만든다. **테스트가 사용자 데이터 디렉토리에 닿는다**(`ipc/mod.rs:534-546`).
- 테스트가 기대하는 계약
  - 존재하지 않는 인스턴스: `GET /api/instance/:id`와 `POST .../command`가 **404**를 주고 `error` 필드를 포함해야 한다(`:426-457`).
  - 생성한 인스턴스가 `/api/servers`에 나타나야 한다.
  - 삭제한 뒤 GET하면 404여야 한다.
  - `check-update`: 없는 인스턴스는 404, `extension_data`가 비어 있으면 `update_available:false`.
  - `client unregister?shutdown=false`는 봇을 유지하고, `shutdown=true`는 봇 PID를 kill해야 한다(`:740-848`).
- 인증, `/health`, `/api/updates/*`, relay, ext-process에 대한 테스트는 **없다.**
