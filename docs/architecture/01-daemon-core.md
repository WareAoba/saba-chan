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
   - CLI 모드는 CLI 대화형 셸이 터미널을 차지하므로 로그를 `<data_dir>/daemon.log` 파일로 보낸다(truncate, `main.rs:239-247`). 나머지 모드는 stderr와 `<data_dir>/logs/daemon.log`(이어 쓰기, 5 MiB를 넘으면 `daemon.log.1`로 넘김, `daemon_log::LogFile`)에 함께 보낸다. GUI가 띄운 데몬의 stderr는 GUI의 파이프라 GUI가 먼저 끝나면 남지 않기 때문이다. 이 모드는 패닉도 tracing을 거치지 않고 같은 파일에 `PANIC` 줄로 남긴다.
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
    - 프로세스 모니터: 2초 주기로 `process_monitor::get_running_processes_async()` 스냅샷을 찍고 `sup.monitor_processes_taken_at()`를 호출한다(`main.rs:416-463`). 스냅샷은 락 밖에서 찍으므로, 찍기 시작한 시각 뒤에 추적한 PID는 스냅샷에 없어도 끝났다고 보지 않고 다음 주기로 넘긴다. 이때 supervisor **write lock**을 잡는다. 크래시로 끝난 서버는 `take_pending_restarts`로 모아 락 밖에서 `server::auto_restart`로 다시 띄운다(`main.rs:456-461`). 작업 잠금이 잡혀 있으면(백업 중) 풀릴 때까지 최대 30분 기다리고, 그사이 사용자가 정지·시작하면 그만둔다.
    - 업데이트 주기 확인 `spawn_update_scheduler`(`main.rs:466`)와 인스턴스 예약 작업(백업·재시작·공지·업데이트) 실행기 `backup::spawn_scheduler`(`main.rs:468`, 30초 주기)를 띄운다. 데몬이 켜질 때 있던 작업은 데몬 시작부터, 그 뒤에 더하거나 고친 작업은 그때부터 세고, 밀린 차례(PC 절전 등)는 가장 최근 것 한 번만 실행하고, 그 바로 뒤 차례의 예고가 이미 시작됐으면 밀린 것은 건너뛰고 그 차례만 한다. 검사를 통과하지 못하는 작업(손으로 고친 `every_minutes: 0` 등)은 실행하지 않는다(`scheduler::Clock`).
    - 하트비트 reaper: 30초 주기로 90초 넘게 하트비트가 없는 클라이언트를 정리한다(`main.rs:483-497`, `handlers/client.rs:116-136`).
    - 렌더러 watchdog은 없다. 인터페이스가 모두 떠나도 데몬은 남는다. 다음에 켠 GUI·CLI가 다시 붙는다(C14).
    - 트레이 감독 `tray_host::run`(`src/tray_host.rs`, 부트 모드와 상관없이, 데몬 온리 포함): 화면이 있는 데스크톱 세션이고(`has_desktop`: Windows는 세션 0이 아니고 창 스테이션이 `WinSta0`, macOS는 늘, 그 밖은 `DISPLAY`·`WAYLAND_DISPLAY`) 데몬 옆에 `saba-chan-gui`가 있으면, IPC가 열린 뒤 3초부터 5초마다 트레이를 든 GUI가 있는지 본다. 등록된 GUI 클라이언트 중 프로세스가 살아 있는 것도, 이 감독이 띄운 그 프로세스(PID와 띄운 시각으로 본다. 등록을 못 했거나 잃었어도 센다)도 없으면 `saba-chan-gui --tray`(상주 GUI, 04 §1)를 띄운다(`IPC_BASE`는 `clients/launch`와 같다). 띄운 것이 30초 안에 사라지거나 띄우지 못하면 다음 시도까지 5초부터 두 배씩, 최대 5분 기다린다. `stop_requested`가 오거나 업데이터를 띄웠으면(`UpdateState::updater_launched`) 멈춘다. `SABA_NO_TRAY`가 비어 있지 않으면 띄우지 않는다(테스트 하네스). 데몬이 다시 띄우는 인터페이스는 이것뿐이다.
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
| `SABA_MODULES_PATH` | `component_dirs.rs` `Kind::env_var` | 모듈 디렉토리 (있으면 `config.json` `paths`보다 먼저, §1.5) |
| `SABA_EXTENSIONS_DIR` | `component_dirs.rs` `Kind::env_var` | 익스텐션 디렉토리 (같음) |
| `SABA_INSTANCES_PATH` | `constants.rs:119` | 인스턴스 루트 |
| `SABA_AUTH_DISABLED=1` | `ipc/auth.rs:120` | 토큰 검사 비활성화. **디버그 빌드에서만** 듣는다 |
| `SABA_PYTHON_URL`, `SABA_PYTHON_SHA256` | `updater/src/python_pin.rs:32-38` | 포터블 Python 미러 주소와 그 해시 |
| `SABA_UPDATER_PATH` | `ipc/updates.rs:746-748` | 업데이터 실행 파일 경로 |
| `SABA_LOCALES_DIR` | `chat/i18n.rs:109` | 채팅 응답 로케일 폴더 |
| `APPDATA`, `HOME` | 경로 해석 | 플랫폼 기본 경로 |

**데몬이 자식 프로세스에 넘기는 환경변수**

- Python 모듈/익스텐션: `PYTHONIOENCODING`, `PYTHONUNBUFFERED`, `PYTHONPATH`, `SABA_IPC_PORT`, `SABA_IPC_URL`, `SABA_TOKEN_PATH`, `SABA_LANG`(요청의 `Accept-Language`, 없으면 `config.json`의 `gui.language`, 비었으면 OS 언어, 실행마다 읽음), `SABA_ENABLED_EXTENSIONS`(`plugin/mod.rs:14-38, 304-322`), `SABA_EXTENSION_DATA_ROOT`(import된 익스텐션이 쓸 폴더의 뿌리, 02 §2.1), `SABA_JAVA_RUNTIMES`(데몬이 찾은 Java 런타임 `[{java, home, version, major, arch}]`, 02 §2.1). 토큰 값 자체는 넘기지 않는다. 공용 `modules/daemon_rcon.py`는 `SABA_TOKEN_PATH` 파일을 읽어 `X-Saba-Token`을 붙인다(`modules/daemon_rcon.py:48-54`).
- `SABA_LANG`는 GUI도 데몬에 넘기지만(`saba-chan-gui/src-tauri/src/lifecycle.rs:177`) **데몬 코어는 이 환경변수를 읽지 않는다.** 언어는 HTTP 요청에 `Accept-Language`가 있으면 그 요청 동안 첫 언어 태그(`ko-KR,ko;q=0.9` → `ko-KR`, 영문자·숫자·`-` 16자까지, `*`는 무시)를 쓰고(CLI `--lang`, 감사 출처 `Origin.lang`로 그 요청이 띄운 작업까지 이어진다), 없으면 `config.json`의 `gui.language`에서 읽고, 비었으면 OS 언어를 지원 언어로 맞춰 쓴다(`ConfigStore::ui_language`, `chat::i18n::os_language`. 중국어는 Hant·TW·HK·MO면 zh-TW, 그 밖은 zh-CN). 릴레이에 알리는 이 PC의 언어(하트비트·디스코드 설정의 `language`)는 요청 언어를 따르지 않는다(`ConfigStore::pc_language`).

### 1.4 디스코드 릴레이 연결 (`src/relay_agent.rs`)

로컬 Discord 봇 실행 경로(`node_env`, `discord_bot` ext-process)는 L1(7b5102f)에서 지웠다. 디스코드 봇은 클라우드 릴레이에서만 돌고, 데몬은 릴레이 에이전트로 붙는다.

1. IPC 서버가 뜬 뒤 `relay_agent::auto_start`를 띄운다(`ipc/mod.rs:860-866`). 수동 제어는 `/api/relay/agent{,/start,/stop}`. 페어링 완료(`/api/relay/pair/:code/status`가 `claimed`로 토큰을 저장)나 `PUT /api/config/node-token`(저장된 토큰과 다를 때만) 뒤에는 `relay_agent::restart`가 돌던 세션을 멈추고 새 토큰으로 바로 다시 연결한다. 돌고 있는 에이전트의 릴레이 주소가 `PUT`·`PATCH /api/config/bot`으로 바뀌어도 같다. 세션은 시작할 때 읽은 주소·토큰 쌍만 쓰고(`Session.token`, `signed_headers_with`), 끝난 세션은 요청을 보내지 않는다.
2. `GET /info`로 최소 에이전트 버전을 확인하고, 60초마다 `POST /heartbeat`로 서버·모듈·봇 설정 메타데이터를 올린다(`relay_agent.rs:23`).
3. `GET /poll`(롱폴링)로 디스코드 명령을 받아 채팅 처리기(`crate::chat`)로 실행하고 `POST /result/:id`로 돌려준다. 명령은 실행을 시작할 때 `POST /ack`로 알리고(릴레이가 404면 그 세션에서는 생략, 받아 두고 시작하지 못한 명령은 연결이 끝나면 잊어 릴레이가 다음 연결로 다시 보낸다. 이미 시작한 명령의 결과는 연결이 끝난 뒤에도 보낸다), 명령 id를 10분·1,000개까지 기억해 다시 온 명령은 실행하지 않는다(끝난 명령이면 기억한 결과를 다시 보낸다). 결과 전송이 연결 오류·5xx·408·429로 여섯 번 실패하면 명령 자리를 놓고 결과를 남긴다. 남긴 결과는 그 결과를 만든 릴레이 주소·노드 토큰의 세션만 16초 간격으로 다시 보낸다. 주소나 토큰이 다른 세션은 남긴 결과를 버린다. 남긴 결과의 명령이 다시 오면 실행하지 않고 ack만 한다. 남긴 결과가 100건이면 새 명령을 시작하지 않는다(받아 둔 명령은 ack하지 않는다). 그 밖의 4xx는 다시 보내지 않는다.
   - 같은 연결에서 `POST /events {events:[{id, type, data}]}`로 디스코드 알림 채널(I3)에 띄울 이벤트를 올린다(`relay_agent.rs` `events_loop`·`relay_event`). 올리는 것은 `instance.exit`(`crash`·`start_failed`·`unknown_exit`만), `instance.started`(`at, instanceId, instance, source, userName?, unexpected`만), `instance.stopped`(`at, instanceId, instance, source, userName?, kind`만), `backup.done`, `update.available`, `schedule.warn`, `instance.ready`(`at, instanceId, instance, ok, code?`만), `plan.done`(뒤에서 마친 여러 단계 실행: `runId, guildId?, ok, steps, stoppedAt`, 단계 줄에는 원격이라 오류 코드만)뿐이다. 원격으로 나가므로 경로·콘솔 줄·오류 원문은 뺀다(`evidence`는 `exit code …`·`signal …`일 때만, `backup.done`의 `error`는 빼고 `ok`만). 첫 건이 오면 2초 모아 20건씩 보내고, 닿지 못하면 백오프하며 최근 50건만 들고 있는다. 4xx(429 말고)는 그 묶음을 버리고 429·5xx·네트워크 오류는 백오프 뒤 다시 보내며, 404면(relay에 경로가 아직 없음) 그 묶음을 버리고 10분 동안 올리지 않은 뒤 다음 이벤트로 다시 알아본다.
4. 인증은 노드 토큰(`config.json` `node.token`) HMAC 서명이다(`relay_agent.rs:1-11`).

### 1.4.1 사바쨩 웹 원격 (`src/remote/`)

PC 주인이 디스코드에서 받은 일회용 링크로 브라우저(사바쨩 웹)를 열면, 브라우저 요청이 릴레이 웹소켓을 거쳐 데몬에 온다.

1. 릴레이가 명령 대기열로 `{action:'web_session'}`을 보내면 에이전트가 `remote::link::ensure`로 `<릴레이>/node/ws`(https→wss)에 `Authorization: Bearer <노드 토큰>`·`X-Saba-Client: 1`로 붙는다. 연결은 프로세스에 하나다. 에이전트가 멈추거나 토큰이 바뀌면 `remote::link::stop`으로 닫는다.
2. 붙자마자 `{t:"hello", version, protocol:1}`을 보낸다. 릴레이는 `{t:"open", sid, user}`·`{t:"close", sid}`로 세션을 알리고, `{t:"req", sid, id, method, path, body?}`로 요청을 보낸다. 데몬은 `{t:"res", sid, id, status, body}`로 답한다. 열지 않은 세션의 요청은 답하지 않는다. 동시에 8건까지 실행하고, 실행 중과 기다리는 것을 합해 32건을 넘으면 쌓지 않고 바로 429 `remote_busy`로 답한다. 한 메시지는 16 MiB까지다.
3. 데몬 이벤트 버스(`/api/events`와 같은 것)에서 `instance.*`·`module.*`·`action.run`만 `{t:"event", id, kind, data:{instanceId?}}`로 세션을 가리지 않고 한 번 보낸다(뒤처지면 `kind:"resync"`, `link::web_event`). 웹은 이벤트로 목록을 다시 읽기만 하므로 그 밖의 본문(경로·콘솔 줄·오류 원문)은 보내지 않는다(I3).
4. 세션이 하나도 없는 채로 60초가 지나면 닫는다. 세션이 남은 채 끊기면 1·2·4초 뒤 세 번까지 다시 붙는다(붙을 때마다 횟수를 새로 센다). 기다리는 중에 `stop`하면 다시 붙지 않는다. 25초마다 ping한다.
5. 요청은 `remote::exec::run`이 실행한다. `remote::gate::decide`가 라우트 표(`gate::ROUTES`)로 허용·거부를 정하고(표에 없으면 거부, 라우터처럼 리터럴 조각이 많은 패턴을 먼저 고른다), `..`·인코딩된 `/`·`\`·빈 조각이 있는 경로는 400 `remote_bad_path`, 거부는 403 `remote_not_allowed`다. 허용된 것도 `gate::blocked_fields`가 PC에서만 바꾸는 키를 막는다(403 `remote_field_blocked`, `fields`): 인스턴스 설정 `PATCH /api/instance/:id`의 `executable_path`·`working_dir`·`extra_args`·`process_name`·`auto_detect`·`rest_host`·`rest_port`·`rest_password`·`extension_data`와 그 모듈의 PC 전용 설정(`password`·`file`·`path` 타입, `format = "path"`), 인스턴스 만들기 `POST /api/instances`의 `executable_path`·`working_dir`·`migration_source`·`rest_password`·`extension_data`·`module_settings.<PC 전용 설정 또는 인스턴스 설정의 PC 전용 키>`, 플러그인 설치 `POST /api/extensions/:id/install`의 `download_url`·`sha256`(공식 목록에서만 받는다), 게임 설정 파일 쓰기 `PUT /api/instance/:id/properties`의 `settings.<키>`(인스턴스 설정과 같은 PC 전용 키와 그 모듈의 PC 전용 설정), REST 호출 `POST /api/instance/:id/rest`의 `rest_host`·`rest_port`·`use_https`(인스턴스에 저장한 주소로만 보낸다). 모듈 설정 칸을 찾는 인스턴스 id는 라우터처럼 퍼센트 인코딩을 풀어서 찾는다. 통과하면 `actions::call_as`로 라우터를 안에서 부르고(인증·감사 미들웨어를 그대로 거친다) 감사 출처는 `web`(+ 링크를 받은 디스코드 id)다. `web`은 HTTP 헤더로 흉내 낼 수 없는 출처다(`audit.rs` `INTERNAL_SOURCES`). 인스턴스를 담은 응답(`GET /api/instances`·`/api/instance/:id`·`/api/servers`, `gate::shows_instances`)은 `exec::hide_secrets`가 `rest_password`를 null로, 모듈의 `password` 타입 설정을 `module_settings`에서 빼고 돌려준다. 디스코드에만 거는 제한(원격 백업 개수, 원격 일정, 약관 동의는 PC에서만)은 `discord` 출처에만 걸리므로 웹에는 걸리지 않는다.
6. 허용 범위: 서버·인스턴스 관리(시작·정지·재시작, 콘솔·입력·로그, 명령·RCON·REST, 설정, 서버 설정 파일 쓰기, 게임 서버 업데이트, 진단, 약관 동의), 백업·일정, 멤버 권한과 길드별 노출 서버, 공식 목록의 모듈 설치와 플러그인 설치·켜기·끄기, 데몬 콘솔 읽기, 시스템 사용량, 액션 목록 읽기. 막는 것: 진단·감사 정보, 실행 줄 미리보기(`launch-preview`: 저장하지 않은 초안의 경로로 모듈이 프로그램을 부를 수 있다), 게임 설정 원문 읽기(`properties`·`settings/live`: 관리자 비밀번호가 들어 있다), 모듈 폴더 감지·삭제·게임 서버 직접 설치(`/api/module/:name/install`), 플러그인 다시 읽기·붙이기·설정·삭제·GUI 번들, 데몬·앱 설정, 노드 토큰, 데이터 옮기기, 경로, 클라이언트 등록·데몬 종료, 디스코드 설정·채널·릴레이 연결·페어링, 챗봇, 샌드박스 예외, 액션 실행(안에서 라우터를 다시 불러 표를 비켜 가므로), 업데이트, Python 점검. 라우터의 모든 라우트가 표에 분류되어 있는지 `gate::tests::every_route_is_classified`가 `ipc/mod.rs`·`ipc/updates.rs`를 읽어 확인한다(새 라우트는 분류해야 테스트를 지난다).

### 1.4.2 원격 데몬 포트 (`src/remote_daemon/`, D1·D1b·T1)

같은 주인의 다른 PC가 이 데몬에 TLS로 직접 붙는 원격 포트다(사용자 결정 2026-10-25 "직접 TLS 연결, 릴레이는 연결될 때까지만 중계", 릴레이 쪽은 05 §4.4 (j)). 1~4는 원격 PC 쪽(D1), 5~6은 붙는 쪽(D1b), 7은 GUI·CLI가 원격 데몬을 고르는 것(G1), 9는 공인 주소 없는 PC 터널(T1)이다. CLI는 `remote` 명령(04 §3.2)으로 다룬다.

1. 설정은 `config.json` `remote` 한 곳이다: `enabled`(기본 false), `port`(기본 `constants::DEFAULT_REMOTE_PORT` 57480, 사용자 확정 2026-10-26), `address`(릴레이에 알릴 공인 호스트 이름 또는 IP, 릴레이와 같은 규칙 `remote_daemon::valid_address`. 비우면 터널로만 붙는다, 9), `transport`(`direct`뿐. 직접 연결이 안 되면 늘 9의 터널로 붙으므로 고르는 값이 아니다), `clients[]`(짝 맺은 PC `{hostId, name, tokenHash, pairedAt}`, 토큰은 SHA-256만), `daemons[]`(이 PC가 붙는 원격 데몬 `{hostId, name, address, port, certSha256, token, pairedAt}`, 토큰은 보내야 하므로 노드 토큰처럼 원문). 내보내기에서 빼고(`LOCAL_SECTIONS`) 클라이언트 네임스페이스로 쓸 수 없다(`RESERVED_SECTIONS`).
2. `remote_daemon::apply`가 설정대로 연다·닫는다·다시 연다. 데몬이 IPC를 연 뒤, `PUT /api/remote/settings` 뒤, 노드 토큰이 바뀐 뒤(`adopt_node_token`) 부른다. 켜져 있으면 `<data_dir>/remote-cert.der`·`remote-key.der`(키는 주인만 읽는다)의 자체 서명 인증서로(없거나 못 쓰면 새로 만든다. 지문이 바뀐다) `[::]:<port>`(IPv6가 없으면 `0.0.0.0`)에 TLS(rustls, ring)로 받는다. 연결은 64개까지(한 곳(IP)에서 8개까지, 이 PC 안 루프백으로 붙는 릴레이 터널은 따로 16개), 핸드셰이크는 10초까지다. 열려 있으면 릴레이에 `PUT /api/remote/endpoint {address, port, certSha256}`를 노드 토큰 서명으로 올리고(`address`가 없으면 `null`. 실패하면 포트가 열려 있는 동안 5초부터 두 배씩, 5분 상한으로 다시 올린다. 주소 없는 endpoint를 모르는 예전 릴레이가 400이면 다시 보내지 않는다), 닫았으면 `DELETE`로 내린다(데몬이 다시 뜬 뒤에는 원격 포트 인증서가 있는 PC만). 결과는 `status {listening, certSha256?, published, error?}`(`certificate_failed`·`port_in_use`·`bind_failed`)다.
3. 원격 포트의 라우트는 로컬 IPC와 같다(`IPCServer::routes`). 인증층만 다르다(`remote_daemon::server::remote_auth`): IPC 토큰·루프백 Host 검사 대신 `Authorization: Bearer sbr_…`(짝 맺은 PC의 토큰, 해시를 상수 시간으로 비교)를 받는다. 모르는 토큰은 401 `remote_unauthorized`. `/api/config/node-token`과 7의 이 PC 전용 경로(`target::stays_local`: `/api/remote/*`, `/api/client*`, `/api/config/gui*`·`/api/config/client/*`, `/api/updates/*`, `/api/system/autostart`, `/api/daemon/shutdown`)는 토큰이 있어도 403 `remote_local_only`다. `/health`는 토큰 없이 받는다. 감사 출처는 `remote`(+ 그 PC 이름)이고 헤더로 흉내 낼 수 없다(`INTERNAL_SOURCES`).
4. 짝 맺기 `POST /remote/pair {ticketId, secret}`(토큰 없이, 1분에 한 곳(IP)에서 5번·모두 합쳐 20번까지, 넘으면 429 `too_many_attempts`. 본문은 1KB·머리 시간 제한 안에 와야 한다: 넘으면 413 `body_too_large`, 늦으면 408 `request_timeout`): 붙는 PC가 릴레이에서 받은 연결표를 내민다. 데몬은 설정한 릴레이에 `POST /api/remote/tickets/:id/redeem`을 노드 토큰 서명으로 한 번 묻고, 맞으면 그 PC(`fromHostId`) 앞으로 새 토큰 `sbr_…`을 내서 `{token}`으로 답한다. 같은 PC가 다시 짝을 맺으면 예전 토큰은 더는 통하지 않는다. 릴레이가 받지 않으면 401 `ticket_invalid`, 릴레이에 닿지 못하면 502 `relay_unreachable`, 이 PC가 릴레이에 연결돼 있지 않으면 503 `relay_not_paired`.
5. 붙기 `POST /api/remote/daemons {hostId}`(`remote_daemon::peer::connect`): 릴레이에서 그 PC 앞 연결표를 받고(`POST /api/remote/tickets`, 주소·지문이 같이 온다), 그 지문을 고정해(`tls::pinned_client_config`, 인증서 사슬은 보지 않는다) TLS로 붙어 `/remote/pair`로 토큰을 받아 `daemons[]`에 저장한다. 붙기·핸드셰이크는 10초까지다. 주소가 없거나 직접 붙지 못하면 9의 터널로 짝을 맺는다. 실패 코드: 같은 주인의 다른 PC가 아님 404 `remote_not_found`, 그 PC가 원격 포트를 올리지 않음 409 `remote_no_endpoint`, 이 PC가 릴레이에 연결 안 됨 503 `relay_not_paired`, 릴레이에 닿지 못함 502 `relay_unreachable`, 원격 데몬에 닿지 못함 502 `remote_unreachable`, 원격 데몬이 연결표를 받지 않음 502 `remote_rejected`.
6. 중계 `/api/remote/daemons/:hostId/proxy/<경로>`(GET·POST·PUT·PATCH·DELETE, `peer::forward`): `<경로>`가 `api/…`·`health`일 때만 풀지 않은 그대로(쿼리 포함) 저장한 원격 데몬에 보낸다. 넘기는 요청 머리는 `Content-Type`·`Accept`·`Accept-Language`·`Last-Event-ID`·`X-Saba-Client`뿐이고(이 PC의 IPC 토큰·쿠키는 넘기지 않는다) 받은 토큰을 `Authorization: Bearer`로 붙인다. 본문은 16MB까지 모아 보내고(413 `body_too_large`), 답은 머리가 4200초(정지 대기 3600초+여유, CLI와 같다) 안에 오면 본문을 오는 대로 넘긴다(이벤트 스트림). 붙은 뒤 답이 오지 않으면 504 `remote_timeout`이고 다시 보내지 않는다(원격 데몬이 이미 처리하고 있을 수 있다). 붙지 못했으면 릴레이 `GET /api/remote/peers`의 지금 주소·지문이 저장한 것과 다를 때만 저장을 고치고 한 번 더 해 본다(원격 PC의 IP가 바뀌었거나 인증서를 새로 만든 경우. 지문은 처음처럼 릴레이를 믿는다). 그래도 안 되면 502 `remote_unreachable`, 저장하지 않은 PC면 404 `remote_unknown`. 이 데몬이 멈추면 기다리던 답은 503 `shutting_down`으로 끝내고 넘기던 스트림도 닫는다(원격 이벤트 구독이 데몬 종료를 막지 않게). 직접 연결은 요청마다 새로 맺는다. 주소가 없거나 직접 붙지 못하면(주소를 고쳐 다시 붙는 것까지 포함해 각 시도에서) 9의 터널로 보낸다.
7. 고르기 `X-Saba-Target`(`remote_daemon::target`, G1): 로컬 IPC 요청(인증을 지난 뒤)에 이 머리가 있으면 6과 같은 중계로 그 원격 데몬에 보낸다. 값은 저장한 `hostId`나 이름(퍼센트 인코딩한 UTF-8)이다. 이름이 둘 이상에 맞으면 409 `remote_ambiguous`, 없으면 404 `remote_unknown`, 읽을 수 없으면 400 `bad_target`. 비었거나 `local`이면 이 데몬이 처리한다. `/api/…`가 아닌 경로(`/health` 포함)와 이 PC의 앱·연결에 관한 경로는 머리와 상관없이 이 데몬이 처리한다(`target::stays_local`): `/api/remote/*`, `/api/client/*`·`/api/clients*`, `/api/config/gui*`·`/api/config/client/*`, `/api/updates/*`, `/api/system/autostart`, `/api/daemon/shutdown`. `..`·`.` 조각이 있으면 400 `remote_bad_path`. 원격 포트와 안에서 부르는 라우터(`actions::call_as`)는 이 머리를 보지 않는다(중계를 잇지 않는다). 중계한 요청은 이 PC의 감사 로그에 남지 않고 원격 데몬이 출처 `remote`(+ 이 PC 이름)로 남긴다. CLI는 전역 `--daemon <이름>`(`SABA_DAEMON`)으로, GUI는 상단 바에서 고른 데몬(`gui.daemonTarget`)으로 싣는다(04 §1.1·§3.1).
8. 합쳐 보기 `GET /api/remote/servers`(`remote_daemon.rs` `all_servers_with`, G1c): 이 PC(`hostId: null`)와 저장한 원격 데몬마다의 서버 목록 `{daemons:[{hostId, name, ok, error_code?, servers}]}`. 이 PC는 `list_servers_internal`(포트 충돌 알림을 비우지 않는다), 원격 데몬은 동시에 6의 중계로 `GET /api/servers?notices=keep`을 묻고 본문까지 30초(옛 주소 붙기 10초 → 새 주소로 다시 붙기 10초를 넘는 값)가 넘으면 `remote_timeout`. 닿지 않은 데몬은 `ok:false`와 6의 실패 코드로 남는다. `/api/remote` 아래라 7의 머리와 상관없이 이 PC가 답한다. GUI 합쳐 보기(04 §1.2 `OtherDaemons`)와 CLI `remote servers`가 쓴다.
9. 터널 (`remote_daemon::tunnel`, T1, 릴레이 쪽 05 §4.4 (k), 사용자 결정 2026-10-26 "직접 연결 먼저, 안 되면 릴레이 터널"): 공인 주소가 없거나 직접 닿지 않는 원격 데몬에 릴레이를 거쳐 붙는다.
   - 붙는 쪽: 릴레이 `/node/tunnel?to=<hostId>` 웹소켓(노드 토큰, `X-Saba-Client: 1`)을 열고, 그 위에 지문을 고정한 TLS(ALPN `h2`)와 HTTP/2 연결을 맺는다. 릴레이 주소·원격 PC마다 연결 하나를 열어 두고 요청을 여러 개 싣는다(열어 둔 연결이 그새 끊겨 요청이 나가지 못하고 돌아오면 새 터널로 한 번 더 보낸다)(이벤트 스트림이 열려 있어도 같은 연결). 여는 데 20초까지이고, 끊겼으면 다음 요청이 다시 연다. 원격 데몬을 잊으면(`DELETE /api/remote/daemons/:hostId`) 그 PC까지의 연결을 놓는다(릴레이의 터널 수 한도를 쥐고 있지 않게). 이 연결이 살아 있으면 직접 연결보다 먼저 쓴다(NAT 뒤 PC에 직접 붙기를 요청마다 기다리지 않게).
   - 원격 쪽: 릴레이 명령 `remote_tunnel {tunnelId}`(`relay_agent.rs`)를 받으면 원격 포트가 열려 있을 때만 `/node/tunnel?accept=<tunnelId>`를 열고 이 PC의 원격 포트(`127.0.0.1`, 없으면 `::1`)에 TCP로 붙어 바이트를 잇는다. 원격 포트가 꺼져 있으면 `{success:false, error:"remote_off"}`. 웹소켓 바이너리 프레임 하나에 64KiB까지 싣는다.
   - 원격 포트는 루프백에서 ALPN `h2`로 붙은 연결만 HTTP/2로 받는다(터널). 그 밖은 HTTP/1.1이다(요청 머리 시간 제한이 HTTP/2에는 없다). 인증(3)과 짝 맺기(4)는 직접 연결과 같고, 짝 맺기 횟수 제한은 터널 연결을 모두 한 곳(루프백)으로 센다.
   - 릴레이가 아직 터널을 모르면(배포 전) 직접 연결만 된다.

### 1.5 파일/디렉토리 레이아웃 (`constants.rs`가 단일 진실 원천)

`<data_dir>`는 다음 순서로 정해진다.

1. `SABA_DATA_DIR`
2. Windows: `%APPDATA%/saba-chan`
3. Unix: `$HOME/.config/saba-chan`
4. 그 외: `./saba-chan`

```
<data_dir>/
  .ipc_token                IPC 인증 토큰 (uuid v4, Unix 0600)          auth.rs:27-43
  config.json (+ .bak)      모든 사용자 설정 (gui, relay, updates, boot, extensions, chatbot, node, cli 등)
                            updater/src/config_file.rs. `node.token`은 cloud relay 노드 토큰(sbn_<id>.<secret>,
                            handlers/relay.rs `load_node_token`·`write_node_token`). 내보내기 설정 절에서 뺀다
  settings.json 등           예전 개별 파일. 한 번 config.json으로 이관하고 원본은 두며 .migrated 사본을 남긴다.
                            예전 `.node_token`(0.1.8까지)은 `node.token`에 넣은 뒤 사본 없이 지운다(다시 생기면 예전 판이 새로 페어링한 것이라 그 값이 이긴다)
  remote-cert.der, remote-key.der   원격 데몬 포트의 자체 서명 인증서와 키(키는 주인만 읽는다)    remote_daemon/tls.rs
  daemon.lock (+ .owner)    단일 인스턴스 잠금과 실행 중 데몬 pid·주소    instance_lock.rs
  daemon.log                CLI 부트 모드에서만 생성
  logs/daemon.log (+ .1)    그 밖의 모드의 데몬 로그·패닉 (GUI 로그 saba-chan-<시각>.log와 같은 폴더)    daemon_log.rs
  python-guard/sitecustomize.py   활성 익스텐션만 import하게 하는 가드    plugin/mod.rs:113-125
  installed-manifest.json, updater-state.json, update-complete.json (updater)
  modules/<module>/module.toml, lifecycle.py, locales/*.json
  python-shared/i18n.py, daemon_rcon.py, saba_audit.py   데몬에 내장된 공용 Python 유틸리티 (plugin/mod.rs)
  extensions/<ext>/...
  instances/
    order.json
    <uuid>/instance.json, settings.json, logs/console.log, backups/, server/ (설치 대상)
    .saba-state/<uuid>/managed.pid, run/ (데몬만 쓰는 서버 실행 기록), .saba-state/legacy-adopted (예전 배치를 옮겨 받은 표시, 02 PID 영속화)
<exe_dir>/
  saba-chan-gui[.exe], saba-chan-cli[.exe]   부트 선택기가 스폰
  saba-chan-updater[.exe]   /api/updates/launch-updater가 스폰
```

- 모듈·익스텐션 폴더(`updater/src/component_dirs.rs` `resolve`)는 `SABA_MODULES_PATH`/`SABA_EXTENSIONS_DIR` → `config.json`의 `paths.modules`/`paths.extensions`(절대 경로만, 아니면 기본값) → `<data_dir>/modules`·`<data_dir>/extensions` 순이다. 기본 폴더 이름과 기본 설치 폴더(`default_install_dir`: Windows `%ProgramW6432%`(없으면 `%ProgramFiles%`, `C:\Program Files`)`\Saba-chan`, Unix `~/.local/share/saba-chan`)는 `constants.rs`에 한 번만 둔다.
- 사용자는 폴더 하나(X)를 고르고 실제 폴더는 `X/modules`, `X/extensions`다(익스텐션은 `import extensions.<id>` 때문에 이름이 `extensions`여야 한다). 고를 수 없는 폴더: 상대 경로, 드라이브 루트·홈 폴더, 데이터 폴더를 품는 폴더, 지금 폴더와 겹치거나 다른 종류 폴더와 겹치는 폴더(`target_for`, 오류 코드 `not_absolute`·`root_or_home`·`contains_data_dir`·`same_as_current`·`overlaps_current`·`overlaps_other`).
- 옮기기(`relocate`): 같은 볼륨이면 rename 한 번. 아니면 옆 `.<이름>.saba-moving`에 복사하고 파일마다 크기와 SHA-256을 맞춘 뒤 대상 이름으로 바꾸고, 그다음 `config.json`을 쓴다. 어느 단계든 실패하면 만든 것을 지우고 설정을 바꾸지 않는다. 원본은 설정을 쓴 뒤에만 지우고, 지우지 못한 것은 `left_behind`로 알린다. 대상이 비어 있지 않거나(`not_empty`), 쓸 수 없거나(`not_writable`), 원본에 링크가 있으면(`has_link`) 거절한다. 사바쨩이 만든 폴더에는 `.saba-chan-dir` 표시를 두고, 제거기는 표시가 있는 폴더만 지운다(05 §제거). 0.1.8·0.1.9 데몬은 `paths`를 읽지 않는다.
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

- **라우터 정의 위치**: `src/ipc/mod.rs:640-817` (axum 0.8), 업데이트 라우트는 `src/ipc/updates.rs:106-117`.
- **경로 파라미터 표기**: 코드는 axum 0.8의 `{param}` 문법을 쓴다(원격 문지기 `remote/gate.rs`의 표도 같다). 이 문서의 표는 예전처럼 `:param`으로 적는다.
- **인증 적용 범위**: `.layer(auth_middleware)`는 업데이트 라우터를 `.merge()`한 **뒤에** 달린다(`ipc/mod.rs:812-816`).
  - 따라서 `/api/updates/*`를 포함한 모든 라우트에 토큰이 필요하다. **`/health`만 예외**다(A6): 루프백 검사는 거치지만 토큰 없이 답한다. 토큰이 어긋나도 클라이언트가 데몬이 떠 있는지 알 수 있게 하려는 것이다.
  - GUI와 CLI는 토큰을 보내기 전에 `/health`에 임의 challenge를 보내고, 데몬의 토큰 기반 HMAC 응답을 검증한다. Windows 리스너는 `SO_EXCLUSIVEADDRUSE`로 포트를 독점 바인드한다.
  - 미들웨어는 토큰보다 먼저 `Host` 헤더가 루프백(`127.0.0.1`, `localhost`, `::1`)인지 보고, 아니면 본문 없는 403을 돌려준다(`auth.rs:90-117`). DNS 리바인딩 방어다.
- **에러 형식이 통일돼 있지 않다.** 다음 형식들이 섞여 있다.
  - `{"error": "..."}` (+ 선택적으로 `"error_code"`)
  - `{"ok": false, "error": ...}` (updates, modules manifest/install/remove, `/api/config/:kind/*`는 성공 시 ok)
  - `{"success": false, "error", "error_code", ...}` (익스텐션, 백업, 이름 변경 등)
  - 인증 실패: 본문 없는 401, Host 거부: 본문 없는 403
  - 일부 핸들러는 실패해도 HTTP 200을 돌려준다(modules manifest/install/remove, extensions manifest/install).
- **JSON 바디 파싱 실패**: axum 기본 거부 응답(400/415/422, 텍스트 본문)이 나간다. 본문을 생략할 수 있는 라우트(`LooseJson`, `ipc/handlers/mod.rs`)는 본문이 없거나 읽히지 않으면 본문 없이 처리한다. `/api/updates/download`·`apply`만 본문을 직접 읽어 `{ok:false, error, error_code:"invalid_body"}` 400을 준다.
- **식별자 혼용**: 예전 경로는 서버를 **이름**(`/api/server/:name/*`, `/api/provision-progress/:name`)으로, 나머지는 인스턴스 **UUID**(`/api/instance/:id/*`)로 가리킨다. `/api/instance/:id/start|stop|restart`는 id가 없으면 이름으로도 찾는다(`server.rs:667-671, 767-774`).

### 2.2 라우트 표

아래 표의 모든 라우트는 `X-Saba-Token` 헤더가 필요하다.

#### 헬스 / 서버 제어 (handlers/server.rs)

| Method | Path | Handler | 요청 | 응답 |
|---|---|---|---|---|
| GET | `/health` | `server.rs:19` `health_check` | 선택: `X-Saba-Challenge` | `{ok:true, version, pid, token, proof?, stops_with_launcher}`. 토큰 없이 부를 수 있다. `token`은 보낸 `X-Saba-Token`이 `valid`·`invalid`·`missing` 중 무엇인지(값은 드러내지 않는다). challenge가 있으면 `proof`는 토큰 기반 HMAC 응답이며 클라이언트가 데몬 신원을 확인하는 데 쓴다. `stops_with_launcher`는 데몬이 닫히면 함께 끝내는 job(kill-on-close) 안에서 도는지다(Windows만 true가 될 수 있다. GUI가 job breakaway를 거절당해 그대로 띄운 경우나 CLI가 그런 job 안에서 띄운 경우, L46. 바로 위 job만 본다). 클라이언트가 true일 때 데몬이 띄운 프로그램과 함께 끝날 수 있음을 알리는 데 쓴다(CLI는 `daemon start`·`daemon status`·`status`와 데몬을 켜 준 직후에 경고로 알린다. GUI는 `daemon_status`의 `in_parent_job`으로 받아 상단 바에 경고한다(`ParentJobNotice`, 04 §1.1)) |
| GET | `/api/diagnostics` | `diagnostics.rs:20` | - | A6, 인증 필요. `{ok, version, pid, uptime_secs, listen_addr, data_dir, token_file:{path, exists}, audit_log, extensions:{enabled, enabled_but_missing}, modules:{count}\|{error}, recent_problems:[최근 경고·오류 20개, 오래된 순]}` |
| GET | `/api/onboarding` | `diagnostics.rs` `onboarding` | - | J1-a 다음 할 일. `{steps:[{id, done, optional}], next}`. 단계 `module`(모듈 있음)·`server`(서버 있음)·`discord`(노드 토큰 있음)·`chatbot`(챗봇 API 키, 선택). `next`는 안 한 첫 필수 단계 id, 다 했으면 null. 문구는 클라이언트가 id로 고른다. relay 하트비트 `metadata.onboarding`에도 같은 값이 실린다 |
| GET | `/api/audit` | `diagnostics.rs` `audit` | `?limit=`(기본 100, 최대 1000)`&instance=<id>` | A10 감사 로그, 최신이 먼저: `{entries:[{ts, source, user?, userName?, guild?, guildName?, action, instanceId, instance?, command?, args?, ok, status}]}`. 비밀은 `***`로 가린다(아래 `audit.rs`). `action`은 `start`·`stop`·`restart`·`command`(`/command`·`/rcon`·`/rest`)·`stdin`·`properties`·`reset`, 예약 백업은 `backup`(출처 `schedule`, `status` 200 만듦·204 백업할 것 없음·409 서버가 꺼져 있어 건너뜀·그 밖은 실패), 액션 카탈로그의 관리 액션은 그 액션 id(예 `instance.settings.set`) |
| GET | `/api/events` | `events.rs` `stream` | 헤더 `Last-Event-ID` 또는 `?since=<n>` | P4 이벤트 스트림(SSE). 한 건은 `id`(데몬이 켜진 뒤 1부터)·`event`(종류)·`data`(`{at, ...}` JSON). 최근 200건을 메모리에 두고 `since` 뒤 것을 먼저 보낸다. 번호가 버퍼보다 오래됐거나 데몬 재시작으로 커졌거나 느려서 놓친 건이 있으면 `event: resync`. 15초마다 `:ping`. 종류: `action.run`(감사 로그 한 줄과 같은 모양, `ts` 대신 `at`), `instance.exit`(`instanceId, instance, kind, exitCode, signal, evidence, restartInSecs, restartAttempt, note, noteCode, noteArgs`, `restartAttempt`는 다시 띄우기로 했을 때만 숫자인 복구 시도 번호, `noteCode`·`noteArgs`는 `/api/instance/:id/events`의 `note_code`·`note_args`와 같다), `instance.started`(`instanceId, instance, source, userName?, unexpected`, 서버가 켜질 때마다(시작이 성공한 때, 자동 재시작 포함). `source`는 시킨 쪽: 요청의 감사 출처(`gui`·`cli`·`api`·`discord`·`web`·`remote`·`schedule`·`daemon`, 크래시 뒤 자동 재시작은 `restart`), 말하는 사바쨩(`via:"chatbot"`)은 `chatbot`. `userName`은 감사 출처의 이름(디스코드 표시 이름, 원격 PC 이름). 사용자·길드 id는 싣지 않는다. 사바쨩이 띄우지 않은 프로세스를 모니터가 찾아 붙였으면 `source:"external"`·`unexpected:true`(예기치 않은 시작). 데몬이 켜진 뒤 첫 모니터 순회에서 찾은 것(데몬이 켜질 때 이미 돌던 서버), 사바쨩이 띄우는 중(추적 전)에 먼저 본 것, 포트 충돌로 바로 끈 것은 알리지 않는다, `supervisor/mod.rs` `announce_started`), `instance.stopped`(`instanceId, instance, source, userName?, kind`, 정상 정지: 사바쨩을 거친 정지는 `kind:"user_stop"`과 정지를 시킨 쪽(`source`는 `instance.started`와 같다), 게임 안에서 끈 것(`in_game_stop`)은 `source:"game"`. 크래시·시작 실패·원인 모를 종료(작업 관리자 강제 종료 등)는 이 이벤트 없이 `instance.exit`만 낸다. 그러므로 예기치 않은 종료는 `instance.exit`의 `crash`·`unknown_exit`, 예기치 않은 시작은 `instance.started`의 `unexpected:true`뿐이다, `announce_stopped`), `instance.restart_failed`(`instanceId, instance, noteCode:"restart_failed", noteArgs:{reason}, restartAttempt`, 크래시 뒤 자동 재시작이 실패했을 때. 오류 원문이 있어 relay로 올리지 않는다), `instance.restarted`·`instance.restart_cancelled`(`instanceId, instance, restartAttempt`, 자동 재시작 대기가 끝나 서버를 다시 켰을 때 / 대기 중에 정지·시작·삭제돼 그만뒀을 때. `restartInSecs`가 있는 `instance.exit`부터 `restartAttempt`가 같은 이 둘이나 `instance.restart_failed`까지가 복구 시도 중이다(번호가 다른 끝 이벤트는 이전 시도의 것이다). relay로 올리지 않는다), `instance.port_moved`(`instanceId, instance, field:"rest_port", from, to`, 시작 직전 기본 REST 포트가 Windows 예약 범위에 들어 옮겨 저장했을 때(7.2). relay로 올리지 않는다), `provision.progress`(`instance` + `/api/provision-progress` 필드. 지금 붙은 클라이언트에게만 보내고 `id`가 없다: 버퍼에 남지 않아 다시 붙을 때의 기록을 밀어내지 않는다), `backup.done`(`instanceId, instance, kind, name?, ok, error?`, `name`은 백업 파일 이름), `update.available`(`components:[표시 이름]`, `/api/updates/check`의 `update_names`와 같다. 보이는 업데이트(구성요소·최신 버전)가 지난번과 달라졌을 때만), `schedule.warn`(`instanceId, instance, action:"restart"|"update", minutesLeft`, 예약 재시작·업데이트 예고 시각마다), `plan.done`(`runId, guildId?, ok, steps, stoppedAt`, 40초 안에 끝나지 않아 뒤에서 마친 여러 단계 실행), `instance.ready`(`instanceId, instance, ok, code?`, 시작 한 번(자동 재시작 포함)마다 한 번: 모듈의 `[extensions.saba-core] ready_patterns`가 콘솔에 찍혔거나 패턴이 없어 시작이 성공했을 때 `ok:true`, 준비 전에 끝나면 `ok:false`와 `code` `start_failed`·`stopped_while_starting`·`ready_timeout`(02 §`[extensions.<익스텐션 id>]`). relay로는 이 필드만 올린다), `instance.update_notice`(`instanceId, instance, kind:"module"|"server", module?, latest?`, `/api/instance/:id/start`로 새로 켰을 때(이미 켜져 있었으면 없다): `module`은 받아 둔 업데이트 확인 결과에 그 모듈의 새 버전이 있을 때 바로, `server`는 뒤에서 부른 `server.check_update` hook(`GET /api/instance/:id/check-update`와 같은 결과)이 `update_available:true`일 때. relay로는 이 필드만 올린다), `instance.player`(`instanceId, instance, player, change:"join"|"leave"`, managed 시작에서 모듈의 `player_join_patterns`·`player_leave_patterns`가 콘솔 줄에 맞을 때(02 같은 절). 이름만 싣고 relay로 올리지 않는다), `relay.link_request`(`requestId, guildId, guildName?, requestedBy?, userName?, expiresAt?`(ISO 8601 UTC 문자열), 디스코드에서 코드 없이 온 연결 요청. relay로 다시 올리지 않는다). 데몬 종료(`shutdown_token`)에 스트림도 끝난다 |
| GET | `/api/version` | `server.rs:306` | - | `{version, components:{saba-core, module-<이름>, ext-<id>, ...}}` |
| GET | `/api/servers` | `server.rs:26` `list_servers` | `?notices_after=<id>` 또는 `?notices=keep` | `{servers:[ServerInfo], port_conflict_stops?:[...]}` (아래 설명) |
| GET | `/api/server/:name/status` | `server.rs:620` | - | supervisor 결과 JSON 그대로. 404 시 `{error, error_code:"instance_not_found"}` |
| POST | `/api/server/:name/start` | `server.rs:881` | `{module: string (필수), config?: any}` | supervisor 결과(예약 범위를 피해 REST 포트를 옮겼으면 `port_moved`, 7.2). 에러 시 404/422/500 + `error_code`∈{instance_not_found, executable_missing, start_failed}, 409 `instance_busy`(같은 인스턴스의 백업·복원·업데이트 중) |
| POST | `/api/server/:name/stop` | `server.rs:938` | `{force?: bool}` (바디 필수) | supervisor 결과. 에러 시 `error_code`∈{instance_not_found, stop_failed}. 모듈이 저장 후 종료를 요청할 수 없으면 HTTP 200 `{success:false, error_code:"graceful_stop_unavailable"}`(서버는 그대로, `force:true`로 다시, 02 §3.4) |
| POST | `/api/instance/:id/start` | `server.rs:654` | - | 데몬이 고른 경로(managed/non-managed, `Supervisor::uses_managed_start`)의 응답 + `managed`(+ 옮긴 포트가 있으면 `port_moved`, 7.2)(+ 새로 켰고 받아 둔 업데이트 확인에 그 모듈의 새 버전이 있으면 `module_update {module, current, latest}`. 그때와 게임 서버에 새 빌드가 있을 때 `instance.update_notice`를 낸다, `server.rs` `announce_updates`). 404 `not_found`, 409 `agreement_required`(`agreements`: 동의하지 않은 모듈 약관, validate보다 먼저), 409 `preflight_failed`(모듈 validate의 critical 문제), 409 `instance_busy`(백업·복원·업데이트 중), 422 `memory_limit_reached`(`limit_mb`: 시작하는 중 러너가 메모리 상한으로 끊음. 종료 이력과 `instance.exit`에 `start_failed`로도 남는다), `success:false` `start_cancelled`(시작 준비 중에 정지·삭제가 들어와 띄우지 않았거나 띄운 것을 바로 껐다. 정지는 시작 요청을 받을 때(약관·validate 점검 전, `supervisor::START_SINCE`)의 실행 세대(`run_generation`)가 바뀐 것으로 안다. 익스텐션이 맡은 시작(`server.pre_start` handled)이 끝났을 때 정지가 들어와 있었으면 정지를 한 번 더 불러 익스텐션이 띄운 서버를 끈다) |
| POST | `/api/instance/:id/stop` | `server.rs:785` | 선택 `{force?}` | `/api/server/:name/stop`과 같다 |
| POST | `/api/instance/:id/restart` | `server.rs:796` | 선택 `{force?}` | 정지(non-managed는 정지 호출을 06 §6의 한도 `stop_timeout`+30초까지 하고 그 뒤 프로세스가 끝나기를 최대 2분 더 기다린다. managed는 모듈 호출 최대 120초 뒤 `stop_timeout`만큼 기다리고 강제 종료 5초) 뒤 시작 응답(`port_moved` 포함) + `stopped`. 작업 잠금이 잡혀 있으면(백업·복원 중) 풀릴 때까지 최대 30분 기다렸다 켠다(`backup::start_when_free`). 기다리는 시작은 따로 돌아 요청이 끊겨도 이어지고, 정지 단계(서버가 끝나기를 기다리는 동안)에 다른 정지·시작이 들어왔거나, 그사이 다른 쪽에서 켜거나 인스턴스를 지우면 409 `restart_cancelled`(`stopped` 포함). 기다린 뒤의 시작은 재시작이 확인한 실행 세대를 시작 기준(`supervisor::START_SINCE`)으로 써서, 확인과 시작 사이에 들어온 정지도 시작을 취소한다(`success:false` `start_cancelled`, 자동 재시작·예약 업데이트 뒤 시작도 같다, `backup::start_when_free`) |
| GET | `/api/instance/:id/events` | `server.rs:872` | - | `{events:[...]}` 최근 종료 이력 / 404. 한 건은 `at, kind, exit_code, signal, evidence, restart_in_secs, note`에 다시 띄우지 않은 이유가 있으면 `note_code`(`auto_restart_off` · `gave_up` + `note_args.count` · `restart_failed` + `note_args.reason`)·`note_args`. `note`는 영어 문장이라 화면 문구는 `note_code`로 고른다 |

`GET /api/servers` 응답의 `ServerInfo` 필드(`ipc/mod.rs:160-209`):

- 기본: `id, name, module, status, pid, start_time, executable_path, working_dir?, port`
- 접속 정보: `rest_host, rest_port, rest_username, rest_password, protocol_mode`
- 설정과 상태: `module_settings, server_version, last_api_action?, provisioning, managed, extension_status?, extension_data, port_conflicts?`
- `revision`: 인스턴스 설정의 내용 해시(`ServerInstance::settings_revision`, 예약 `schedule`·`backup_keep`은 뺀다. 그 둘은 `/schedule`의 revision이 따로 본다). 모든 revision은 `config_store::patch::revision`이 데몬이 켤 때마다 새로 정하는 키로 서명한 지문이라 데몬이 다시 뜨면 바뀐다(받은 쪽은 409를 받고 다시 읽는다). `PATCH /api/instance/:id`에 그대로 돌려보내면 그사이 바뀐 설정을 덮어쓰지 않는다
- `linked_folder?`: 인스턴스 폴더 밖의 서버 폴더(가져온 서버). 삭제해도 남는다. `restorable_settings`: 가져오기 전 게임 설정 파일을 보관해 두어 삭제(연결 해제) 때 되돌릴 수 있는가(`instance::migration`). 클라이언트는 이 둘로 삭제 확인을 "연결 해제"로 바꾼다.
- `slots?`: 익스텐션 데이터 slot을 데몬이 판정한 결과(`{"ServerCard.badge":[{extension,text}], "ServerDetail.status":[{extension,label,value}]}`, 없으면 키가 없다). 규칙은 03-extensions 4.3.
- `players?`: `{online, max?}`. `status`가 `running`이고 모듈이 `[status] players`로 알려 준 마지막 접속자 수(1분마다 갱신, `supervisor/players.rs`, 02 §1.2). 모르면 키가 없다.
- `unresponsive?`: 문자열. `status`가 `running`인데 그 접속자 함수가 5분(`players::UNRESPONSIVE_AFTER`) 넘게 계속 실패하거나 시간을 넘길 때, 모듈의 마지막 실패 이유(`message`)다. 프로세스는 살아 있지만 상태 질문에 답하지 않는다(예: 팰월드가 REST 포트를 열지 못하면 명령·정상 종료도 닿지 않는다). 한 번 답하면(수 없이 `{"success": true}`만 와도) 키가 없어진다. 기록은 pid별이라 재시작한 새 프로세스는 이전 실패를 잇지 않는다. 5분을 넘긴 뒤 첫 질문(1분마다)에서 데몬 로그에 경고를 한 번 남기고, 키는 그때부터 나온다(화면과 로그가 어긋나지 않게).
- `managed`는 saba-runner가 맡아 실행하는지다. 판정은 데몬의 `Supervisor::uses_managed_start` 하나이고(`server.rs:87`), 클라이언트는 이 값을 그대로 쓴다. non-managed면 데몬은 입출력을 잡지 않고, Windows에서는 서버를 새 콘솔 창에 띄운다(`CREATE_NEW_CONSOLE`, 데몬의 표준 핸들을 넘기지 않는다, `utils::spawn_in_new_console`). 실행 파일이 `.exe`·`.com`이 아니면(배치 파일 등) 예전처럼 창 없이 띄운다(`utils::new_console_program`).
- **REST 비밀번호가 평문으로 포함된다.**

`port_conflict_stops`는 포트 충돌로 멈춘 알림 목록이다. 알림마다 `id`(늘어나기만 하는 번호. 붙일 때의 Unix 시각(µs)보다 작지 않아 데몬을 다시 켜도 앞 번호보다 크다)와 `timestamp_ms`가 있고, 읽어도 지우지 않고 10분(`PORT_CONFLICT_STOPS_TTL_MS`)·최근 20개(`PORT_CONFLICT_STOPS_KEPT`)까지 둔다. 클라이언트는 `?notices_after=<본 마지막 id>`로 그 뒤 알림만 받는다(처음엔 0). 이렇게 읽으면 다른 클라이언트 몫을 지우지 않으므로 GUI와 CLI가 각자 본다(L50). `?notices=keep`은 `notices_after=0`과 같다(남은 알림 모두, 대기 루프·목록용). 매개변수가 없는 옛 클라이언트에는 그 방식으로 아직 보이지 않은 알림만 한 번 준다(예전 drain 동작, 알림은 지우지 않는다). 데몬 안의 목록 호출(릴레이 하트비트·채팅, `list_servers_internal`)은 남은 알림 모두를 본다(`server.rs` `server_list`).

`status` 값은 다음 중 하나다.

- `running` / `stopped`: PID 기반
- 익스텐션 `server.list_enrich` 훅이 `handled:true`를 돌려준 경우: 그 훅이 준 값
- `provisioning`: 프로비저닝 진행 중

#### 모듈 (handlers/server.rs, handlers/managed.rs, handlers/config.rs)

| Method | Path | Handler | 요청 | 응답 |
|---|---|---|---|---|
| GET | `/api/modules` | `server.rs:401` | - | `{modules:[ExtensionInfo], failed?:[...]}` (아래 설명). `failed[]` 항목은 `{path, error, module?, error_code?, missing_dependencies?, problems?, required?}`: 스키마에 맞지 않으면 `error_code:"module_schema_invalid"`와 `problems:[필드 이름과 이유]`, 데몬보다 새 스키마면 `"module_schema_unsupported"`와 `required:<스키마 버전>`(02 §1.2) |
| POST | `/api/modules/refresh` | `server.rs:408` | - | 위와 같음 |
| GET | `/api/modules/schema` | `server.rs:419` | - | module.toml JSON Schema |
| GET | `/api/maker` | `handlers/maker.rs` | - | 모듈 메이커 안내 `{ok, saba_core:{version, path, runner}, python:{path, version}, shared_libs, modules_dir, schema_versions, repo, guide}`. 읽기만 한다. 원격 웹(`remote/gate.rs`)에는 열지 않는다 |
| POST | `/api/modules/detect` | `server.rs:425` | `{path}` | `{matches:[{name, display_name}]}` (`dir_signatures`가 맞는 모듈) / 400 / 404 |
| GET | `/api/modules/manifest` | `server.rs:502` | - | `{ok, manifest}` 또는 `{ok:false,error}`. 항상 200 |
| POST | `/api/modules/manifest/:id/install` | `server.rs:525` | - | `{ok, module_id, version, extensions:{installed, enabled, failed}}` 또는 `{ok:false,error}`. 항상 200. 카탈로그 버전이 설치된 버전보다 낮으면 받지 않는다. 모듈이 요구하는 익스텐션을 받고 켠다(02-modules §5.2) |
| DELETE | `/api/modules/:id` | `server.rs:593` | - | `{ok, id}`. 항상 200 |
| GET | `/api/module/:name` | `server.rs:467` | - | `{name, version, description, path, metadata}` / 404 |
| GET | `/api/module/:name/versions` | `managed.rs:375` | query `include_snapshots, page=1, per_page=25` | supervisor 결과 |
| GET | `/api/module/:name/version/:version` | `managed.rs:404` | - | supervisor 결과 |
| POST | `/api/module/:name/install` | `managed.rs` `install_server_handler` | `{version?, install_dir*, instance_id?, server_executable?, accept_agreements?:[id], initial_settings?, background?}`(0.1.10 결정 3: `instance_id`면 모듈을 부르기 전에 그 인스턴스를 `pre_update` 종류로 백업하고(`[backup]`이 없으면 건너뛴다, 실패하면 설치하지 않고 422 `backup_failed` + `reason`), 설치가 `success:false`이거나 오류로 끝나면 그 백업을 안전 백업 없이 되돌린다(응답 `restored_backup`, 되돌리지 못하면 `kept_backup`). 성공하면 데몬이 그 인스턴스에 `server_version`과, 모듈이 알려 준 `executable_path`(예전 `jar_path`)·작업 폴더(`install_path`, 없으면 실행 파일 폴더)를 저장한다. 저장하지 못하면 500 `save_failed`. `background:true`와 `instance_id`를 함께 주면 검사(버전 고르기·잠금) 뒤 바로 `{success:true, provisioning:true, id, version}`을 돌려주고 진행·결과는 `/api/provision-progress/:인스턴스 이름`에 싣는다(끝나면 `done`, 실패는 `error`에 모듈의 `message`나 `provision.script_failed`·`save_failed`·`install_backup_failed` 문구. 모듈이 `warning`(예전 `java_warning`)을 주거나 실패했으면 사용자가 닫을 때까지 남고, 아니면 5초 뒤 지운다) (`server_executable`은 받을 서버 실행 파일 이름, 없으면 모듈 `server_executable`. `version`·`instance_id`가 문자열이 아니면 400 `invalid_format`. `instance_id`면 설치가 끝날 때까지 그 인스턴스 작업 잠금을 잡고(다른 작업 중이면 409 `instance_busy`, 켜져 있으면 409 `server_running`) 그 `module_settings`를 `install_server` `settings`로 넘긴다. 없는 인스턴스는 404 `not_found`, 다른 모듈의 인스턴스는 400 `invalid_format`. `version`을 비우면 모듈 `list_versions`에 그 설정을 넘겨 최신을 고르고, 못 고르면 422 `version_unknown`(까닭을 `error`에). 응답에 설치한 `version`을 싣는다. 모듈을 부르기 전에 받을 실행 파일과, `instance_id`면 그 인스턴스가 실제로 띄우는 파일(`executable_path`, 없으면 모듈 `server_executable`, `Supervisor::resolve_executable`)을, 실제 경로가 `install_dir` 안에 있을 때만(밖을 가리키는 링크는 빼고) 같은 폴더의 비어 있는 이름(`<파일 이름>.bak`, 이미 있으면 `<파일 이름>.1.bak`, …, 기존 파일은 덮지 않는다)으로 복사해 두고, 설치가 `success:false`이거나 모듈이 오류로 끝나면 둘 다 되돌린다(rename, 안 되면 복사. 심볼릭 링크가 그대로 있으면 링크를 둔 채 그 대상에 복사, 모듈이 링크를 바꿨으면 그 자리에 되돌린다). 성공하면 백업을 지운다. 백업을 만들지 못하면(디스크 오류, `.1.bak`~`.99.bak`까지 이름이 모두 참) 모듈을 부르지 않고 422 `success:false`, `error_code:"backup_failed"`, `file`(백업하지 못한 파일 이름), `message`(영어. 화면 문구는 `error_code`로 고른다, GUI·CLI 표시는 후속 작업). 되돌리지 못한 파일은 `.bak`을 남기고 응답 `restore_failed:[파일 이름]`과 같은 순서의 `kept_backups:[남긴 백업 이름]`으로 알린다. 모듈이 오류로 끝났으면 500 `error`에 모듈 오류와 `could not put back <파일> (kept as <백업 이름>)`이 함께 있고, 같은 `restore_failed`·`kept_backups` 칸도 싣는다) (`accept_agreements`는 원격이면 403 `agreement_local_only`, 모듈이 선언하지 않은 id면 400 `invalid_format`) | 200(success) / 422 / 404 / 400 / 500. `install_dir`를 쓰는 인스턴스(`working_dir`나 인스턴스 폴더가 같은 경로)는 설치를 마칠 때까지 작업 잠금을 잡고, 그 인스턴스가 다른 작업 중이면 409 `instance_busy`, 켜져 있으면 409 `server_running` |
| GET | `/api/module/:name/locales` | `config.rs:201` | - | `{<lang>: <json>}` |

- `ExtensionInfo`(이름과 달리 모듈 정보다, `ipc/mod.rs:237-287`) 필드: `name, version, description, schema_version, display_name?, game_name?, author?, warnings?, unmet_dependencies?, path, executable_path, server_executable?, icon(base64 data URL), interaction_mode, protocols{supported,default}, settings, commands, syntax_highlight, dir_signatures?, install?, module_aliases?, command_aliases?`
- `server_executable`: 모듈 `[config] server_executable`(서버 실행 파일 이름, 예 `server.jar`·`PalServer.exe`). 선언이 없으면 빠진다. 클라이언트는 `action_required: "executable_not_found"`의 파일 고르기 창에서 이 확장자로 파일 종류를 정한다.
- `functions`(MK1): 모듈이 예약 lifecycle 함수 `functions`로 밝힌 구현 함수 이름 배열, 모르면 `null`. 데몬은 데몬 시작·`POST /api/modules/refresh` 때 미리 묻고, 진입 파일마다 한 번만 부른다(15초 제한, 동시에 온 요청은 같은 실행의 답을 기다린다). 진입 파일의 수정 시각이 바뀌면(모듈 업데이트) 다시 묻는다. 목록 응답은 캐시에 없으면 2초까지만 기다리고 `null`로 답하며 묻기는 뒤에서 마저 한다. `functions`를 모르는 옛 모듈(`Unknown function`)은 `null`이고 다시 묻지 않는다. 실패(시간 초과·격리 차단 등)도 `null`이지만 60초 뒤 다시 묻는다. 클라이언트는 `null`이면 아무 기능도 숨기지 않고, 배열이면 빠진 선택 함수의 기능을 숨긴다: `reset_server`(서버 초기화), `list_versions`·`get_version_details`·`get_installed_version`(버전 목록·상세·설치된 버전), `diagnose_log`(로그 진단), `read_properties`·`reset_properties`·`configure`(게임 설정 읽기·초기화·저장), `import_settings`(설정 가져오기), `install_server`(설치·업데이트), `validate`(시작 전 점검), `config_files`(가져온 서버의 원래 설정 보관, 데몬만 부른다). 데몬도 같은 표(`plugin::OPTIONAL_MODULE_FUNCTIONS`)로, 목록이 있는데 빠진 선택 함수가 불리면 Python을 띄우지 않고 `{success:false, error_code:"not_supported", function, message}`(HTTP는 그 경로의 성공 응답과 같다)로 답한다.
- `/api/modules/manifest`는 최신 릴리즈 서명 manifest의 `module-*` 컴포넌트를 `{ok, manifest:{modules:{<이름>:{version, sha256, size_bytes, display_name, ...}}}}`로 돌려준다(`relay::latest_catalog`, `handlers/server.rs:499-522`).
- `/api/modules/manifest/:id/install`은 같은 manifest의 `module-<id>`를 R2에서 받아 SHA-256을 확인하고(`download_verified`), staging에 푼 뒤 데몬이 읽을 수 있는 모듈일 때만(`load_module_dir`) `modules/<id>`와 바꾸고(기존 폴더는 바꾼 뒤에 지운다) refresh한다(`:524-591`). `id`의 `..`, `/`, `\`는 거부한다. 받은 zip과 staging은 요청마다 다른 이름(`.tmp-module-<id>-<uuid>.zip`, `.staging-<id>-<uuid>`)이고, 같은 모듈의 설치·제거(`DELETE /api/modules/:id`)·업데이트 적용(`POST /api/updates/apply`의 모듈)은 한 번에 하나만 한다(id는 대소문자를 가리지 않는다). 설치는 잠금을 쥔 채 따로 돌아, 요청이 끊겨도 받은 파일을 치우고 잠금을 푸는 일이 작업 끝에 함께 일어난다. 다른 요청이 진행 중이면 `{ok:false, error_code:"module_busy"}`다.

#### 인스턴스 (handlers/instance.rs)

| Method | Path | Handler | 요청 | 응답 |
|---|---|---|---|---|
| GET | `/api/instances` | `instance.rs:12` | - | `[ServerInstance]` (§4.1, REST 비밀번호 평문 포함) |
| POST | `/api/instances` | `instance.rs:84` | `{name*, module_name*, executable_path?, working_dir?, migration_source?, migration_mode?:"link"|"move", port?, preset?, module_settings?, accept_agreements?:[id], extension_data?:{}}` (`accept_agreements`: 동의한 모듈 약관. 선언 안 된 id는 400 `invalid_format`, 원격이면 403 `agreement_local_only`. 가져오기(`migration_source`)면 만든 직후 lifecycle `accept_agreements`를 불러 모듈이 반영하게 한다. `extension_data`: 익스텐션 `AddServer.options`의 답. null이 아닌 값을 새 인스턴스에 넣고, 객체가 아니면 400 `invalid_format`) | 201 `{success, id, provisioning?, module_trust, warnings[]}`. 모듈이 `verified`가 아니면 `warnings`에 `{code:"module_unverified"\|"module_modified", module}` 한 건(알림일 뿐 만들기는 막지 않는다). 409 `working_dir_in_use {instance}`(`working_dir`가 다른 인스턴스의 서버 폴더나 인스턴스 폴더와 같거나 그 안·바깥: `InstanceStore::folder_user`), 409 `name_taken`(같은 이름의 인스턴스가 있음), 422 `extension_required`, 400 `preset_not_found`, 400 `invalid_format`(`migration_mode`가 link·move가 아님), 409 `move_cross_volume`·`move_in_use`·`move_failed`·`source_missing`·`destination_exists`(옮기기 실패, 아무것도 만들지 않음), 422 `module_path_outside`(가져오기의 `import_settings`가 경계 밖 경로를 돌려줌. 옮긴 서버 폴더는 원래 자리로 돌려놓고 그 폴더에 준 격리 실행 권한을 거두고, 인스턴스 폴더 안에서 물려받은 권한·낮은 수준 표시를 걷어 원래 자리의 상속 권한을 되찾게 한 뒤(`sandbox::adopt_parent`) 인스턴스를 지운다(`undo_import`). 지우기 전에 `server.pre_delete`를 부른다. 서버 폴더를 돌려놓지 못했거나 인스턴스를 지우지 못하면 인스턴스가 남고 `id`를 싣는다), 400/500 |
| PUT | `/api/instances/reorder` | `instance.rs:19` | `{order:[id]}` | `{success:true}` / 400 |
| GET | `/api/instance/:id` | `instance.rs:49` | - | `ServerInstance` + `agreements:[{id, label, url, accepted, accepted_at}]`(모듈 약관, 이름은 데몬 언어) + `hidden:[설정 이름]`(`visible_when`으로 지금 숨겨진 모듈 설정, 저장된 값(공통 칸 `port` 등은 인스턴스 칸의 값) 기준. 02 §1.2) + `revision`(`ServerInfo.revision`과 같다) / 404 `{error, error_code}` |
| DELETE | `/api/instance/:id` | `instance.rs` `delete_instance` | 쿼리 `keep_settings?`(bool) | `{success, linked_folder?, settings_restored?, settings_restore_failed?}` / 404 / 409 `instance_busy`(백업·복원·업데이트 중)·`server_running`(돌던 서버를 끝내지 못함: 아무것도 지우지 않는다) / 500(인스턴스 폴더를 데몬 전용 자리 `.saba-state/deleting/`로 옮기지 못함(연 파일 등, 2초 간격 3번): 설정 되돌리기·권한 거두기 전에 멈추고 아무것도 지우지 않는다, `InstanceStore::detach_dir`. 옮긴 폴더는 되돌리기·거두기를 마쳤다는 표시(`<폴더>.unlinked`)를 남긴 뒤 지우고, 남으면 다음 시작 때 지운다. 표시 없이 남은 폴더(그 사이 데몬이 멈춤)는 제자리로 돌려놓아 인스턴스가 다시 보이고 보관한 원래 설정도 남는다). 서버 폴더가 인스턴스 폴더 밖이면(가져온 서버, `instance::migration::linked_folder`) 그 폴더는 지우지 않고 연결만 끊는다: 가져올 때 보관한 원래 게임 설정 파일로 되돌리고(`keep_settings`면 두고, 지금 그 서버의 경계(`module_paths::Boundary`) 밖이거나 위 폴더가 없어진 자리는 건드리지 않고 `settings_restore_failed`로 센다), 격리 실행이 그 폴더·실행 파일 폴더에 남긴 권한과 낮은 무결성 표시를 거둔다(`sandbox::release`: 사바쨩 프로필들의 명시 ACE, 바로 위 폴더의 목록 권한(다른 인스턴스 폴더가 같은 위 폴더에 있으면 둔다), 실행 파일과 폴더에 직접 붙인 표시. 프로필은 남긴다). 응답의 `linked_folder`는 남은 폴더다 |
| PATCH | `/api/instance/:id` | `instance.rs` `update_instance_settings` | 키-값 객체. 선택 `revision`(GET·`ServerInfo`에서 받은 값)이 지금 값과 다르면 쓰지 않고 409 `settings_changed`(`revision`: 지금 값). 같은 인스턴스의 저장은 하나씩 한다(앞 저장의 게임 파일 쓰기와 hook이 끝난 뒤 다음 저장이 시작된다). 실제 값 읽기(`GET /settings/live`), 속성 쓰기·초기화(`/properties`), 백업 복원(파일 되돌리기와 다시 읽기)도 같은 인스턴스별 잠금을 쥐어, 쓰기 전의 파일 값으로 캐시를 되돌리지 않는다(`InstanceLock::files`, §작업 잠금). 업데이트·복원·설치처럼 서버 파일을 바꾸는 작업 중이면 409 `instance_busy`. `port`·`rest_port`는 1~65535의 수나 숫자 문자열이고 밖이면 400 `invalid_format`(`fields`에 그 키, 빈 문자열은 바꾸지 않음, `rest_port`의 null은 지우기). `working_dir`는 작업 폴더로(지금 값과 다르면: 이 인스턴스가 설치·업데이트·백업·복원 중일 때 409 `instance_busy`, 다른 인스턴스의 서버 폴더나 인스턴스 폴더와 같거나 그 안·바깥이면 409 `working_dir_in_use {instance}`). `extra_args`(문자열은 `chat::split_quoted`로 나눔, 또는 문자열 배열, null은 비우기)는 인스턴스의 추가 실행 인자로. `memory_limit_mb`(MiB 정수 256 이상. 글자는 숫자나 `4G`·`4096M`(`GB`·`MB`, 대소문자 무관). null·""·0은 제한 없음)는 메모리 상한(02 문서 §3.6), 그 밖이면 400 `invalid_format`(`fields:[{field:"memory_limit_mb"}]`). `stop_timeout`(정지 대기 초, 1~3600 정수, 숫자 글자도 받아 숫자로 저장. null·""·0은 지워 모듈 값으로)은 그 밖이면 400 `invalid_format`(`fields:[{field:"stop_timeout"}]`) | `{success:true, synced, sync_error?, ignored?, restart_required, revision}`(`revision`은 저장한 뒤의 값. `restart_required`는 서버가 켜져 있을 때 이번 저장으로 값이 바뀐 `restart_required` 모듈 설정 이름(공통 칸 이름이면 그 인스턴스 칸의 이전 값과 비교), 꺼져 있으면 빈 배열. 데몬은 스스로 재시작하지 않는다. `visible_when`으로 숨겨진 설정은 `required`를 보지 않는다(숨김은 저장된 값에 이번 요청을 얹어 계산). `synced:false`면 저장은 됐지만 모듈 `configure`가 게임 설정 파일에 쓰지 못함, `ignored`는 받고 쓰지 않은 RCON 키. `configure`에는 게임 설정만 간다: `port` 밖의 인스턴스 칸, `extension_data`, 데몬 설정(`DAEMON_SETTINGS`), 모듈이 사바쨩 그룹으로 선언한 설정은 빼고 넘긴다) / 400 `unknown_setting {fields}`(모듈이 설정 목록을 선언했는데 공통 필드·모듈 설정·데몬 설정 `auto_restart`·`restart_on_unknown_exit`·`managed_start`·`log_sessions_keep`·`stop_timeout` 어디에도 없는 키) / 400 `validation_failed`·`invalid_format`(모듈 `format`·`pattern`과 다름)`{details, fields}` / 404 / 500 |
| POST | `/api/instance/:id/launch-preview` | `instance.rs` `launch_preview` → `Supervisor::launch_preview` | `{settings?:{필드:값}, extra_args?: string\|string[]}` (extra_args가 없으면 저장된 것) | 실행 인자 미리보기. 서버를 띄우지 않고 익스텐션 hook도 부르지 않는다. 저장된 설정 위에 초안, 그 위에 추가 인자를 필드 `arg` 모양으로 읽은 값을 얹어 `get_launch_command`(`preview:true`)를 부른다. 필드에 `arg`를 선언하지 않은 모듈이면 모듈을 부르지 않고 `{success:true, has_launch_args:false, ...}`(command_line 없음)를 돌려준다. 성공 `{success:true, has_launch_args, command_line, program, args[], settings_from_args{}, extra_args[], extra_args_text(큰따옴표가 든 인자가 있으면 null: 배열로 저장), errors[{arg\|null, field, expected, got}]}`. password 필드 값과 인스턴스 `rest_password`는 `***`. 모듈 실패 `{success:false, error:"launch_command_failed"\|action_required, message?, ...같은 키}` / 404 / 500 `preview_failed`. `expected`는 `validator::expected_of` 코드(`memory_size`·`port`·`integer`·`path`·`pattern`·`number`·`boolean`·`enum`·`text`) |
| POST | `/api/instance/:id/rename` | `instance.rs:878` | `{name}` | `{success, id, name, previous}`. 설치 진행 표시(이름 기준)도 새 이름으로 옮기고, 새 이름에 남아 있던 옛 표시는 버린다. 인스턴스를 지울 때도 그 이름의 표시를 지운다(`ProvisionTracker::forget`) / 409 `name_taken`·`server_running`·`instance_busy`(설치·업데이트·백업·복원 중) / 404 |
| GET | `/api/provision-progress/:name` | `instance.rs:760` | - | `{active:false}` 또는 `{active:true, step,total,label,message,done,error,percent?,steps?}`. `error`는 모듈이 돌려준 `message`이거나, 스크립트가 비정상 종료·시간 초과면 `provision.script_failed` 문구다(스크립트 출력은 데몬 로그에만) |
| DELETE | `/api/provision-progress/:name` | `instance.rs:719` | - | `{success, rolled_back}`. **신규 생성**의 프로비저닝이 실패한 상태였을 때만 그 설치를 시작한 인스턴스를 ID로 찾아 삭제한다(그 사이 이름이 바뀌어도 같은 이름의 다른 인스턴스는 지우지 않는다. 다른 작업이 그 인스턴스를 쥐고 있으면 지우지 않는다). 아직 끝나지 않은 설치는 닫지 않는다(`rolled_back:false`, 아무것도 바꾸지 않음). 이름을 바꾸면 진행 표시도 새 이름으로 옮겨 간다 |
| GET | `/api/instance/:id/check-update` | `instance.rs` `server_update_status` | - | 훅 결과 또는 `{update_available:false, reason}`. `reason`: `update_unsupported`(설치 방식 익스텐션이 설치한 인스턴스가 아님: 다운로드·수동 설치. `apply-update`와 같은 조건 `installed_by_extension`), `no_extension_data`, `no_hook_handled` |
| POST | `/api/instance/:id/apply-update` | `instance.rs:1481` | - | `{success,id,provisioning:true}`. 409 `server_running`·`instance_busy`(백업·복원·업데이트 중, 업데이트는 끝날 때까지 작업 잠금을 잡는다), 400 `no_install_config`/`update_unsupported`(설치 방식 익스텐션이 설치한 인스턴스가 아님) |

`POST /api/instances` 동작 세부:

- 모듈의 `install.requires_extensions`가 활성·설치돼 있지 않으면 422 `extension_required`다(`instance.rs:233-272`).
- 다른 인스턴스가 같은 `working_dir`를 쓰면 409다(`instance.rs:292-305`). 포트를 주지 않으면 다른 인스턴스와 겹치지 않게 기본 포트를 옮긴다(`:320-338`). 포트를 주면 모듈이 선언한 설정 `port`도 그 값으로 맞춘다(`:311-319`, 설치가 `module_settings`로 게임 설정 파일을 쓴다). 1~65535가 아닌 포트는 400 `invalid_format`(`fields:[{field:"port"}]`, `:306-310`)이다.
- `preset`을 주면 모듈 `[[presets]]`에서 그 id의 `settings`를 바탕으로 깔고, 따로 보낸 `module_settings`가 그 위를 덮는다. 없는 id는 400 `preset_not_found`다. 원격(디스코드·챗봇)도 `instance.create`의 `preset`으로 고를 수 있다(프리셋에는 비밀번호·파일·경로 설정이 없다).
- `module_settings{}`를 주면 모듈이 선언한 설정 이름만(모르는 이름은 무시), select는 고를 수 있는 값만 받아 기본값보다 먼저 넣는다. 선언된 설정에 규칙에 맞지 않는 값이 오면 400 `invalid_format` `fields:[{field}]`다(`instance.rs` "만들 때 고른 설정"). GUI·CLI는 `install = true`인 설정(예: 마인크래프트 서버 종류)을 만들 때 묻는다. 프로비저닝 `install_server`는 이 인스턴스 설정을 `settings{}`로 받는다(업데이트 경로도 같다).
- `migration_source`가 있으면 데몬 락을 놓은 뒤 `import_settings`를 **동기로** 호출한다(`instance.rs:648-686`). 만들기 전체는 요청과 떨어진 작업으로 돌아(`tokio::spawn`), 클라이언트가 시간 초과로 끊어도 설정 가져오기·원래 설정 보관까지 마친다(큰 서버 폴더는 격리 실행 권한을 주는 데 수십 초가 든다. GUI는 가져오기면 시작처럼 30분, CLI는 `post_long`으로 기다린다). 결과의 `executable_path`(작업 폴더 기준, 폴더 안의 파일일 때만)를 실행 파일로 쓰고, `detected`는 응답에 그대로 붙인다(`{success, id, detected?, notices?}`). 기본(`migration_mode` 없음·`"link"`)은 서버 폴더를 옮기지 않고 그 자리에 연결한다(로그·백업·러너 세션은 인스턴스 폴더). `"move"`면 그 폴더를 인스턴스 폴더 안 `<install_subdir 또는 server>`(상대 경로의 보통 이름만, 첫 이름이 데몬 항목이면 409 `move_failed`, `migration::move_target`)로 이름 바꾸기로 옮겨 전속시킨다(`migration::move_into`, 목록에 넣기 전). 같은 드라이브 안에서만 하고 복사하지 않으며, 실패하면 인스턴스 폴더를 지우고 409 `error_code`로 까닭을 돌려준다. 옮긴 서버는 인스턴스와 함께 지워진다. 원래 설정 보관(`migration::save`)은 두 방식 모두 하되 인스턴스 폴더 안의 파일은 건너뛴다(옮긴 서버도 서버 폴더 밖 데이터 폴더의 설정은 지울 때 되돌린다).
- 가져온 설정을 저장한 뒤, 약관 반영(`accept_agreements`)과 첫 시작이 게임 설정 파일을 고치기 전에 모듈 `config_files`가 밝힌 파일 중 지금 있는 것을 인스턴스 폴더 `migration/`(데몬 항목)에 그대로 복사하고 `migration/originals.json`(`[{path, copy}]`)에 적는다(`instance::migration::save`. 복사하지 못한 파일은 경고만 남기고 빼며, 목록을 쓰지 못하면 `migration/`을 남기지 않는다). 그 서버의 경계(`module_paths::Boundary`) 밖 경로는 버린다. 함수가 없거나 실패하면 보관하지 않고 가져오기는 계속한다. 삭제(연결 해제)가 이 사본으로 되돌린다.
- `notices`: 가져온 폴더에서 미리 알릴 것(`instance::migration::notices`). `folder_not_writable`(그 폴더에 시험 파일을 만들 수 없음. 바로 지운다), `server_running`(실행 파일이나 작업 폴더가 그 폴더 아래인 프로세스가 있음: 사바쨩 밖에서 켠 서버. 인수는 보지 않는다). 알림일 뿐 만들기는 막지 않는다.
- 인스턴스를 저장한 직후 `server.post_create` 훅을 백그라운드로 보내고 결과를 기다리지 않는다(`instance.rs:357-370`). ctx는 `{instance_id, instance_dir, instance, module, migration}`이다.
- 모듈에 `install` 설정이 있으면 `lifecycle.py install_server`를 백그라운드로 실행한다(타임아웃 30분).

`PATCH /api/instance/:id` 동작 세부:

- 알려진 키(`port, rest_*, executable_path, protocol_mode, server_version, extension_data`)는 일급 필드로 반영한다. 예전 RCON 키(`rcon_port, rcon_password, enable_rcon`)는 받아도 어디에도 반영하지 않는다(`instance.rs:979-1000`). `rest_*`·`executable_path`에 null을 보내면 지정을 지워 기본값(모듈 실행 파일 해석, REST 기본값·실행 때 만드는 비밀번호)으로 되돌린다.
- 나머지 키는 전부 `module_settings`로 들어간다.
- 이후 락을 놓고 `manage_properties("write")`로 게임 설정 파일과 동기화하고, `extension_data`가 비어 있지 않으면 `server.settings_changed` 훅을 보낸다.

#### 명령 실행 (handlers/command.rs)

| Method | Path | Handler | 요청 | 응답 |
|---|---|---|---|---|
| POST | `/api/instance/:id/command` | `command.rs:16` | `{command*, args?}` | 실행한 경로의 응답 + `command`. 실패 시 `error_code`, 모듈 `[errors]` 문구 `error_message`. 404 / 400 / 409 `server_not_running` / 503 `console_input_full`(콘솔로 보내는 명령인데 서버가 입력을 읽지 않음, `/rcon`과 같다) |
| GET | `/api/instance/:id/commands` | `command.rs` `list_instance_commands` | - | `{commands:[{name,label,description,risk,inputs,source:"module"\|"extension",group?,...}], components?}`. `[commands] hook = true`인 모듈은 lifecycle `instance_commands()`가 준 인스턴스별 명령이 module.toml 명령 뒤에 `group`(예: 플러그인 이름)과 함께 붙고, `components`는 모듈이 판정한 구성요소를 그대로 넘긴다. 목록은 매번 새로 읽는다 |
| POST | `/api/instance/:id/rcon` | `command.rs:392` | `{command*}` | 콘솔 한 줄. 콘솔이 있으면 stdin으로 보낸다: `{success, message, data:{response,executed}\|null, protocol:"stdin"}`. 콘솔이 없고 모듈이 RCON만 받으면 RCON으로: `{success, message, data:{response}, protocol:"rcon"}` / 400 `invalid_argument` / 502 `rcon_unreachable`·`rcon_auth_failed`·`rcon_timeout`·`rcon_failed` (02 RCON 정책) / 404 / 409 `server_not_running`(RCON 경로인데 꺼짐) / 409 `no_command_path`(콘솔도 RCON도 없음) / 503 `console_input_full`(서버가 콘솔 입력을 읽지 않아 대기열 256줄이 참. 기다리지 않고 바로 돌려준다) |
| POST | `/api/instance/:id/rest` | `command.rs:487` | `{endpoint*, method?, body?, timeout?, rest_host?, rest_port?, use_https?}` | `{success, data, error, endpoint, method, host, port, protocol:"rest"}`(응답 없이 연결이 끊기면 `success:false`, `data.outcome:"unknown"`) / 400(`rest_port`가 1~65535 밖의 수면 `invalid_port`, 숫자가 아니면 없는 것으로 본다) / 500(게임 서버가 401·403으로 거절하면 `error_code:"auth_failed"`, `/command`가 모듈 `errors.auth_failed` 문구를 붙인다) |

#### Managed 프로세스와 모듈 기능 (handlers/managed.rs)

| Method | Path | Handler | 요청 | 응답 |
|---|---|---|---|---|
| POST | `/api/instance/:id/managed/start` | `managed.rs:24` | 선택 `{config?}` | supervisor 결과(옮긴 포트가 있으면 `port_moved`, 7.2) / 404 / 409 `instance_busy` / 422 `memory_limit_reached` / 500 |
| GET | `/api/instance/:id/console` | `managed.rs:92` | query `since, count, spans` | supervisor 결과 (`lines[{id,content,level}]`, `spans=1`이면 줄마다 `spans`, 02 §3.3). 서버가 메모리에 있으면 `epoch`(콘솔 버퍼 세대)와 `running`, `stdin_available`도 준다. `stdin_available`은 `/stdin`이 받는지와 같다: 모듈 `protocols.supported`에 `stdin`이 있고 프로세스의 stdin이 열려 있을 때만 true다(REST 전용 모듈은 `[launch] console = "pty"`로 러너가 stdin을 쥐어도 false, `Supervisor::get_console_output`). 줄 `id`는 버퍼마다 0부터라(서버를 다시 켜거나 데몬이 다시 붙을 때) `epoch`가 바뀌면 클라이언트는 이어 읽던 `since`를 버린다. 디스크 기록(`source:"disk"`)은 `since`를 무시하고 `epoch`가 없다 / 404 |
| GET | `/api/instance/:id/logs` | `logs.rs` `logs_handler` | query `source, lines, before` | 로그 전체 읽기(검수 9번). 원천은 러너 세션 파일(`<instance>/logs/sessions/*.log`, `log_sessions_keep`개)과, 러너 없이 직접 실행했을 때의 `logs/console.log`·회전 파일(`console`, `console.1`, `console.2`). 목록은 최근에 쓴 것부터다. `{success, sources:[{id, label, startedAt}], source, lines:[문자열], before?}`. 세션 `id`는 파일 이름(시작 시각 `YYYYMMDD-HHMMSS`, UTC), `label`은 PC 시간대 시각, `startedAt`은 unix 초(직접 실행 기록은 첫 줄의 시각). `source`가 없거나 `latest`면 가장 최근. 줄은 세션 파일 원문(`[시각] [SOURCE/LEVEL] 내용`)이고 끝에서부터 `lines`(기본 200, 최대 500)줄, 더 앞이 있으면 `before`(바이트 커서)를 다음 요청에 준다. 한 번도 켜지 않은 서버는 `sources:[]`, `source:null`. 없는 인스턴스·원천은 404 `not_found`, `lines`·`before`가 0 이상 정수가 아니면 400 `invalid_argument {field}` |
| GET | `/api/instance/:id/logs/export` | `logs.rs` `export_handler` | query `source, since, until` | 한 세션을 파일 하나로: `{success, fileName, bytes, truncated, content}`. `fileName`은 `<서버 이름>-<source>-<YYYYMMDD-HHMM>.log`(지금 PC 시각, 파일 이름에 못 쓰는 글자는 `_`). `since`·`until`(unix 초)로 줄 시각을 거르고 시각 없는 줄은 앞 줄을 따른다. `content`는 JSON 문자열로 쓴 크기(릴레이 `/result`로 갈 때의 크기)로 4 MiB까지, 넘으면 최근 쪽 온전한 줄만 남기고 `truncated:true`. 원격으로 나가며 가린 뒤에는 `bytes`도 가린 내용의 크기다. 원천이 없으면 404, `since`·`until`이 unix 초(0 이상 정수)가 아니면 400 `invalid_argument {field}` |
| POST | `/api/instance/:id/stdin` | `managed.rs:118` | `{command*}` | `/rcon`과 같은 응답(`via_stdin`) / 400 / 409 `stdin_not_supported`(모듈의 `protocols.supported`에 `stdin`이 없음, 예: REST 전용) / 503 `console_input_full` / 500 |
| POST | `/api/instance/:id/validate` | `managed.rs:139` | - | supervisor 결과 |
| POST | `/api/instance/:id/query/:name` | `managed.rs` `query_instance_handler` | `{args?: {입력: 값}}` | 모듈 `[[queries]]` 조회(X-2, 02 §`[[queries]]`): lifecycle `function`을 `validate`와 같은 config + `args`로 부른다. `{success, data}`(실패면 `success:false, error, error_code:"query_failed"`, Python이 실패하면 500, 디스코드에서 부르면 오류 원문 대신 `Query failed`). 404 `not_found`(인스턴스)·`unknown_query`, 400 `invalid_argument {field}`(선언에 없는 입력, 명령 입력과 같은 검사). 읽기 전용이라 감사에 남기지 않는다 |
| GET | `/api/instance/:id/players` | `managed.rs` `players_handler` | - | 접속자 수와 이름(디스코드 검수 13번). 모듈 `[status] players` 함수를 config `players_detail: true`로 바로 불러(10초 제한) `{success:true, online, max?, players:[{name}]}`을 준다. 항목은 `name`만 남긴다(IP·고유 ID는 버린다). 모듈에 `[status] players`가 없으면 200 `{success:false, error_code:"not_supported"}`, 서버가 꺼져 있으면 409 `server_not_running`, 모듈이 실패하면 200 `players_failed`. 액션 `server.players`(Query, safe, 모든 표면)가 이 라우트를 부르고, 인스턴스 액션 목록과 heartbeat `modules`는 `[status] players`를 선언한 모듈만 이 액션을 지원한다고 본다(`actions::module_support`). 챗봇 결과는 콘솔처럼 가린다. 원격 요약은 `👥 접속 3/20명` 뒤 줄마다 이름(10명까지, 나머지는 `…외 N명`) |
| GET | `/api/instance/:id/properties` | `managed.rs:209` | - | `manage_properties("read")` |
| PUT | `/api/instance/:id/properties` | `managed.rs:224` | `{settings}` | `manage_properties("write")` |
| GET | `/api/instance/:id/settings/live` | `managed.rs:246` | - | `manage_properties("import")` + `revision`(맞춘 뒤의 인스턴스 revision). 성공하면 인스턴스 필드도 파일 값으로 맞춘다 |
| POST | `/api/instance/:id/properties/reset` | `managed.rs:274` | - | `"reset"` |
| POST | `/api/instance/:id/server/reset` | `managed.rs:289` | - | `"reset_server"` (월드, 설정 삭제). 작업 잠금을 잡고, 다른 작업 중이면 409 `instance_busy`, 켜져 있으면 409 `server_running` |
| POST | `/api/instance/:id/agreements` | `managed.rs` `accept_agreements_handler` | `{accept:[id]}` | `{success, agreements:[{id, label, url, accepted, accepted_at}]}`. 설정 파일 작업 한 줄에 서고(§작업 잠금), 서버 파일을 바꾸는 작업 중이면 409 `instance_busy`. 모듈 `[[agreements]]` 동의를 기록하고 lifecycle `accept_agreements`를 부른다(02 문서). 400 `invalid_format`(선언 안 된 id), 403 `agreement_local_only`(원격), 404 `not_found` |
| POST | `/api/instance/:id/diagnose` | `managed.rs:357` | - | supervisor 결과 |
| GET | `/api/instance/:id/installed-version` | `managed.rs:420` | - | 결과 + `revision`(저장한 뒤의 인스턴스 revision). 감지에 성공하면 `server_version`에 자동 저장 |

#### 백업과 예약 작업 (handlers/backup.rs)

| Method | Path | Handler | 요청 | 응답 |
|---|---|---|---|---|
| GET/POST | `/api/instance/:id/backups` | `backup.rs:203/210` | - | `{success, backups, snapshots_supported}` / `{success, backup}`. 모듈이 `[backup] snapshots`를 선언하면 목록 끝에 게임이 스스로 만든 자동 백업(스냅샷)이 `{name: "game-snapshot:<id>", kind: "game-snapshot", created, readable: true}`로 붙는다(크기 없음. lifecycle `backup_snapshots()`가 실패하면 빼고 로그만 남긴다). POST 본문 `{source: "snapshot", snapshot?}`(또는 액션 칸 `from_snapshot: true`)이면 지금 월드 대신 그 스냅샷(없으면 가장 새것)을 `kind: "snapshot"` 백업으로 보관한다. 보관한 백업은 스냅샷 내용을 월드 자리 경로로 적어 복원하면 월드 자리로 돌아가고, 수동 백업처럼 정리하지 않는다. 스냅샷이 없으면 404 `snapshot_not_found`, 스냅샷 경로가 인스턴스 경계 밖이면 500 `module_path_outside`다. 항목은 `{name, kind, created, size, stored, readable}`: `size`는 담은 내용의 크기(복원하면 쓰는 바이트, 디스크 사용량이 아니다), `stored`는 그 백업을 만들 때 디스크에 새로 쓴 바이트다(증분 백업 MK16. 0.1.8까지 만든 백업은 둘 다 zip 크기). `readable:false`는 zip으로 열리지 않거나 매니페스트가 없는 백업, 또는 대상이 없는 링크다(복원할 수 없고, 지울 때까지 쓰지 않는 내용의 정리를 멈출 수 있다. 매니페스트 내용까지는 목록마다 읽지 않으므로 매니페스트만 깨진 백업은 `true`로 보인다). 액션 `backup.list` 요약과 `label`에는 `⚠️ 열 수 없음`이 붙는다. GUI·CLI 표시는 후속 작업이다. 400 `nothing_to_back_up`·`backup_unsupported`(모듈에 `[backup]` 없음), 500 `backup_failed`(`backup_paths` 실패). 원격(디스코드·챗봇) 요청은 백업을 정리하지 않으므로, 그 종류의 백업이 이미 50개(`REMOTE_BACKUP_CAP`)면 409 `backup_limit`로 거절한다(복원 전 안전 백업도 같다, D-B7) |
| DELETE | `/api/instance/:id/backups/:name` | `backup.rs:218` | - | `{success}` / 404 `backup_not_found` / 409 `backup_busy` / 400 `snapshot_not_deletable`(게임 자동 백업 `game-snapshot:<id>`는 게임이 관리한다) |
| POST | `/api/instance/:id/backups/:name/restore` | `backup.rs:232` | - | `{success, restored, pre_restore_backup, links_not_restored, settings_refreshed}`. `:name`이 `game-snapshot:<id>`면 그 게임 자동 백업을 잠깐 사바쨩 백업으로 적어 같은 길(안전 백업·되돌리기)로 월드 자리에 되돌리고 그 백업은 지운다. 게임의 스냅샷 폴더는 그대로 둔다. 복원 뒤 모듈 `import_settings`로 되돌린 게임 설정 파일을 다시 읽어 `module_settings`와 인스턴스 필드(포트·REST 비밀번호)를 파일에 맞춘다(`GET /settings/live`와 같은 `managed::refresh_settings_from_files`, MK17). `settings_refreshed`는 그 읽기가 성공했는지이고, 실패해도 복원은 끝난 것이다. `links_not_restored`는 백업한 폴더 안의 링크 중 되살리지 못한 수다(Windows 권한 등, 경로는 데몬 로그에만, 나머지 복원은 끝났다, D-B14). 409 `server_running`·`backup_busy`. 복원할 백업은 안전 백업 뒤 정리에서 뺀다(`backup.rs:244-246`). 백업은 만들 때의 절대 경로로 돌아가므로, 그 경로가 지금 서버 폴더·인스턴스 폴더·모듈이 지금 백업하는 경로(`[backup] paths`·`backup_paths` 훅이 가리키는 경로, 지금 없어도 된다)·훅의 `restore_roots` 밖이면 아무것도 바꾸지 않고 400 `restore_outside`다. 복원 전 안전 백업은 지금 설정의 경로와 함께 복원이 덮을 경로(예: 이름을 바꾸기 전 월드 폴더)도 담는다. 모듈이 `[backup]`을 더는 선언하지 않아도(모듈 업데이트로 빠짐) 덮을 경로는 안전 백업으로 남긴다(`bare_plan`) |
| GET/PUT | `/api/instance/:id/schedule` | `backup.rs` `get_schedule`/`put_schedule` | `{schedule:[...], backup_keep, revision?}` | GET은 `schedule, backup_keep, backup_supported, announce_supported`와 `revision`(일정·보관 수의 지문 문자열)을 준다. PUT에 GET에서 받은 `revision`을 실으면, 그사이 다른 곳에서 일정이나 보관 수가 바뀌었을 때 덮어쓰지 않고 409 `schedule_changed`로 거절한다(다시 읽고 고쳐 보낸다). `revision`이 없으면 예전처럼 받는다(목록 전체를 다시 보내는 클라이언트는 실어 보낸다, L51). 400 `validation_failed {details}`. `backup_keep`은 1 이상이다(자동 백업 정리는 방금 만든 백업을 지우지 않는다). 원격(디스코드·챗봇) 출처는 이미 있는 작업과 같지 않은 작업에 대해 켜진 `update` 작업을 받지 않고(끄는 것은 받는다) `every_minutes`가 30 이상이어야 하며, 바꾸는 `backup_keep`은 30 이하여야 한다. 챗봇을 거친 요청(감사 출처 `via:"chatbot"`, 릴레이 액션이 붙인다)은 `backup_keep`을 지금 값(없으면 10)보다 줄이지 못한다(줄이면 다음 백업이 옛 백업을 지운다). 백업 작업의 하루 실행 횟수도 지금 값(백업 작업이 없거나 하루 한 번보다 드물면 1)보다 늘리려면 보관 수가 덮는 기간(`backup_keep` ÷ 하루 횟수)이 줄지 않게 `backup_keep`을 함께 올려야 한다. 쓰기 잠금 안에서 지금 값과 견주므로 계획 확인 사이 PC에서 올린 값도 본다. 또 작업 종류별로 켜진 작업들이 하루에 도는 횟수(매일 시각 작업 1번, N분마다 ⌈1440/N⌉번)를 48번(30분에 한 번) 넘게 올리지 못한다(이 PC에서 이미 그보다 잦게 정했으면 늘리지만 못한다, `daily_runs`). 예약 `update`가 끈 서버는 업데이트 뒤 다시 켜는데, 작업 잠금이 잡혀 있으면(백업·복원 중) 풀릴 때까지 기다리고 그사이 켜거나 끄거나 지우면 그만둔다(`start_after_update`) |

인스턴스마다 작업 잠금 하나(`instance/lock.rs` `InstanceLock`)를 백업·복원·백업 삭제·업데이트(`begin_server_update`, 작업이 끝날 때까지. 서버가 도는 `working_dir`에 쓰므로 그 폴더를 쓰는 다른 인스턴스의 잠금도 함께 잡고, 그중 켜져 있는 것이 있으면 409 `server_running`)·시작(`start_server_handler`·`start_managed_handler`, 시작을 마칠 때까지)·인스턴스 삭제·서버 초기화(`reset_server_handler`)·버전 설치(`install_server_handler`, 그 폴더를 쓰는 인스턴스 전부, 설치를 마칠 때까지)가 함께 쓴다. 초기화와 버전 설치는 잠금을 잡은 뒤 실행 중이면 409 `server_running`으로 거절한다. 잡혀 있으면 백업 API는 409 `backup_busy`, 나머지는 409 `instance_busy`를 준다. 복원은 잠금을 잡은 뒤 실행 여부를 본다. 백업·복원·서버 초기화·버전 설치·인스턴스 삭제는 잠금을 쥔 채 따로 돌아(`audit::spawn`, 출처 유지) 요청이 끊겨도 파일 작업이 끝날 때까지 잠금이 남는다. 긴 작업은 둘로 나뉜다. 파일을 읽기만 하는 것(백업, 백업 삭제: `InstanceLock::take`)과 서버 파일을 바꾸는 것(그 밖 전부: `take_for_writes`, 생성·가져오기는 기다리지 않는 `try_take_for_writes`). 짧은 설정 파일 작업(설정 저장, `GET /settings/live`, 속성 쓰기·초기화, 약관 동의: `InstanceLock::files`)은 인스턴스마다 한 줄로 서고, 서버 파일을 바꾸는 작업 중이면 409 `instance_busy`로 바로 거절한다. 백업 중에는 그대로 한다(설정 파일은 원자적으로 바뀐다). 서버 파일을 바꾸는 작업은 잡을 때 진행 중인 설정 파일 작업이 끝나기를 기다리고, 끝날 때까지 새 것을 막는다.

#### 설정 (handlers/bot.rs, handlers/config.rs)

| Method | Path | Handler | 요청 | 응답 |
|---|---|---|---|---|
| GET | `/api/config/bot` | `bot.rs` `get_bot_config` | - | BotConfig JSON (`config.json`의 `relay`) + `revision`(저장된 값의 내용 해시, `config_store::patch::revision`) |
| PUT | `/api/config/bot` | `bot.rs` `save_bot_config` | 부분 JSON (최상위 키 단위로 얹는다. `revision`은 보지 않고 버린다) | `{success, message, revision, ignored?}`(`ignored`: 권한 키라 저장하지 않고 이전 값으로 둔 경로 `nodeSettings.<길드>.<키>` 목록, 이름 순. 요청에 그 키가 없거나 값이 지금 값과 같으면 싣지 않는다. 클라이언트는 이 목록을 보여 주고 권한 API를 안내한다) / 400 `{success:false, error, error_code}`(`invalid_relay_url`: 바뀐 `cloud.relayUrl`이 https나 루프백 http가 아니다, `invalid_request`: 그 밖의 형식·접두사 규칙) / 500 `{error}` |
| PATCH | `/api/config/bot` | `bot.rs` `patch_bot_config` | JSON Merge Patch(RFC 7386: 객체는 키마다 합치고 `null`은 그 키를 지운다, 배열·값은 바꾼다) + 선택 `revision` | PUT과 같은 응답. `revision`이 지금 값과 다르면 쓰지 않고 409 `{success:false, error_code:"settings_changed", revision}`(지금 revision). 권한 키 보호(`ignored`)는 PUT과 같다 |
| GET | `/api/config/gui` | `config.rs` `get_gui_config` | - | GuiSettings JSON + `revision` |
| PUT | `/api/config/gui` | `config.rs` `save_gui_config` | 부분 JSON (최상위 키 단위, `revision`은 보지 않는다) | `{ok:true, revision}` / 400 / 500. 저장에 성공하면 `supervisor.skip_port_check`도 갱신 |
| PATCH | `/api/config/gui` | `config.rs` `patch_gui_config` | JSON Merge Patch + 선택 `revision` | PUT과 같다. `revision`이 다르면 409 `{ok:false, error_code:"settings_changed", revision}` |
| GET | `/api/config/:kind/defaults` | `config.rs:47` | - | `{ok, defaults}` (`gui`, `bot`) / 404 |
| POST | `/api/config/:kind/reset` | `config.rs:59` | 선택 `{keys?}` | `{ok, config}` / 400. 봇 설정 초기화도 권한 키와 `cloud.hostId`(노드 토큰을 따름)는 남기고, 릴레이 주소가 바뀌면 에이전트를 다시 시작한다 |
| GET/PUT | `/api/config/client/:kind` | `config.rs:106/117` | 부분 객체(최상위 키 머지) | 그 네임스페이스 JSON (예: `cli`). 예약 이름(`schemaVersion`·`gui`·`relay`·`updates`·`boot`·`extensions`·`chatbot`·`sandbox`)은 400 |
| GET | `/api/config/node-token` | `config.rs:134` | - | `{token}` (없으면 `""`) |
| PUT | `/api/config/node-token` | `config.rs:144` | `{token}` | `{ok}`. 저장 뒤 릴레이 에이전트를 새 토큰으로 다시 시작한다 |
| DELETE | `/api/config/node-token` | `config.rs:180` | - | `{ok}`. 먼저 릴레이 에이전트를 멈춘다 |
| GET | `/api/remote/settings` | `remote_daemon.rs` `get_settings` | - | `{enabled, port, address, transport, status:{listening, certSha256?, published, error?}}`. §1.4.2. 원격 포트·사바쨩 웹에서는 막는다 |
| PUT | `/api/remote/settings` | `remote_daemon.rs` `put_settings` | `{enabled?, port?, address?}` | 저장하고 원격 포트를 다시 맞춘 뒤 GET과 같은 모양. 포트 0은 400 `invalid_port`, 주소 규칙이 맞지 않으면 400 `invalid_address`(빈 값은 지움) |
| GET | `/api/remote/clients` | `remote_daemon.rs` `list_clients` | - | `{clients:[{hostId, name, pairedAt}]}`. 이 데몬에 짝 맺은 PC |
| DELETE | `/api/remote/clients/:hostId` | `remote_daemon.rs` `revoke_client` | - | 204. 그 PC의 토큰이 바로 통하지 않는다. 없으면 404 `not_paired` |
| GET | `/api/remote/peers` | `remote_daemon.rs` `list_peers` | - | `{peers:[{hostId, name, status, hasEndpoint, saved}]}`. 릴레이가 아는 같은 주인의 다른 PC(`saved`: 이 PC가 붙는 원격 데몬). 503 `relay_not_paired`, 502 `relay_unreachable`. §1.4.2 5 |
| GET | `/api/remote/daemons` | `remote_daemon.rs` `list_daemons` | - | `{daemons:[{hostId, name, address, port, pairedAt}]}`(토큰은 보이지 않는다) |
| POST | `/api/remote/daemons` | `remote_daemon.rs` `connect_daemon` | `{hostId}` | `{hostId, name}`. 짝을 맺고 저장한다. 실패 코드는 §1.4.2 5 |
| DELETE | `/api/remote/daemons/:hostId` | `remote_daemon.rs` `forget_daemon` | - | 204. 없으면 404 `remote_unknown` |
| GET·POST·PUT·PATCH·DELETE | `/api/remote/daemons/:hostId/proxy/*rest` | `remote_daemon.rs` `proxy` | 원격 데몬 API 그대로 | 원격 데몬의 답 그대로. §1.4.2 6 |
| GET | `/api/remote/servers` | `remote_daemon.rs` `list_all_servers` | - | `{daemons:[{hostId, name, ok, error_code?, servers}]}`. §1.4.2 8 |
| POST | `/api/migration/export` | `migration.rs:184` | `{path(절대 경로), secrets?}` | `{success, path, secrets, instances:[{id,name}], config:[네임스페이스]}` / 400 `INVALID_PATH` / 500 `WRITE_FAILED`. A9: 데몬이 `path`에 zip(`saba-export.json`, `config.json`, `instances.json`, `secrets`면 `node_token`)을 쓴다. `updates`·`boot`·`extensions`·`sandbox`(동의 기록)·`schemaVersion`은 이 PC 것이라 뺀다(가져오기도 받지 않는다). `secrets`가 아니면 챗봇 API 키, REST 비밀번호, 모듈이 `password`로 선언한 설정을 뺀다(모듈을 읽지 못한 인스턴스는 모듈 설정을 모두 뺀다). 게임 서버 파일은 넣지 않는다 |
| POST | `/api/migration/import` | `migration.rs:257` | `{path, dryRun?, overwrite?}` | `{success, dryRun, appVersion, secrets, instances:{added, replaced, skipped:[{id,name,reason}]}, missingModules, config:[네임스페이스], nodeToken:"imported"\|"kept"\|null}` / 400 `INVALID_PATH`·`INVALID_ARCHIVE`·`INVALID_CONFIG`. 인스턴스는 같은 id가 있으면 `exists`(`overwrite`면 덮어씀, 켜져 있으면 `running`), 다른 인스턴스의 서버 폴더나 인스턴스 폴더와 같거나 그 안·바깥이면 `working_dir_in_use`, 이름이 같으면 `name_taken`, 설치·업데이트·백업·복원이 그 id를 쥐고 있으면(`BusyGuard`) `busy`로 건너뛴다(같은 묶음 안에서 앞서 더한 인스턴스도 본다. `dryRun`도 같은 판정). 설정은 지금 값 위에 최상위 키 머지(파일에 없는 API 키는 남는다). 설정은 다른 것을 바꾸기 전에 저장과 같은 규칙(봇 접두사 포함)으로 먼저 검사한다(`check_sections`, 잘못되면 아무것도 바꾸지 않고 `INVALID_CONFIG`). 노드 토큰을 가져오지 않으면 버려질 릴레이 연결 정보는 검사하지 않는다(`takes_node_token`). `gui`를 가져오면 실행 중인 포트 충돌 검사(`skip_port_check`)도 바로 맞춘다. 노드 토큰은 이 PC에 없을 때만 쓰고, 토큰을 실제로 가져올 때만 `relay.cloud`(relayUrl·hostId)·`cloudNodes`·`cloudMembers`도 바꾼다(아니면 이 PC 값 유지). `added`·`replaced` 항목에는 `working_dir`·`executable_path`가 붙는다 |
| GET | `/api/system/paths` | `paths.rs` `get_paths` | - | `{ok, data_dir, install_dir, default_install_dir, instances_dir, modules:{path,default,source}, extensions:{…}}`. `source`는 `"env"`·`"config"`·`"default"` (§1.5) |
| POST | `/api/system/paths/move` | `paths.rs` `move_dir` | `{kind:"modules"\|"extensions", target}` | 고른 폴더 아래로 옮기고 데몬이 새 폴더를 다시 읽는다(모듈 다시 읽기, 익스텐션 다시 찾기, 업데이트 매니저 폴더). 기본 위치가 아닌 곳으로 옮기면 설치기처럼 옮긴 폴더의 상속 권한을 끊는다(Windows, `component_dirs::relocate_component`, 실패하면 `paths_acl_failed`). 성공 `{ok:true, path, left_behind:[]}`. 실패는 200 `{ok:false, error_code:"paths_<코드>", error}`: §1.5의 코드, `unknown_kind`, `env_fixed`(환경변수로 정한 폴더), `busy`(`error`가 `install`·`update`·`instance <id>`: 모듈·익스텐션 설치·제거, 업데이트, 인스턴스 작업 중), `servers_running`(켜진 서버 id 목록). 설치·제거는 옮기는 동안 기다린다(`components_in_use`) |
| GET | `/api/system/browse` | `paths.rs` `browse` | 쿼리 `path`(전체 경로, 비우면 시작 자리), `files`(보일 파일 확장자, 쉼표로, `*`면 모두. 없으면 폴더만) | 데몬 PC의 폴더 보기(D2): 원격 데몬을 다루는 GUI·CLI가 그 PC의 경로를 고른다. `{ok:true, path, parent, home, sep, entries:[{name, path, dir}], truncated}`. 폴더가 먼저, 이름은 대소문자 없이 정렬한다. 숨긴 항목(`.`으로 시작하는 이름, Windows 숨김·시스템 속성)은 뺀다(경로를 직접 적으면 열린다). 목록에 빠진 것이 있으므로 GUI 저장 창은 저장할 경로 자체를 이 라우트로 물어 `browse_not_found`일 때만 확인 없이 저장한다(파일이면 `browse_not_a_folder`). 한 폴더에 5000개까지(넘으면 `truncated:true`). `path`가 비면 `path:null`과 시작 자리(Windows는 있는 드라이브 `X:\`, 그 밖은 `/`). 실패는 200 `{ok:false, error_code}`: `browse_bad_path`(전체 경로가 아니거나 `..`), `browse_not_found`, `browse_denied`, `browse_not_a_folder`. 원격 웹(`remote/gate.rs`)에는 열지 않는다. 원격 포트에서는 짝 맺은 PC가 다른 데몬 API처럼 부른다 |
| GET·PUT | `/api/system/autostart` | `autostart.rs` `get_autostart`·`put_autostart` | PUT `{mode:"off"\|"daemon"\|"full"}` | 로그인할 때 자동 실행. 설정 파일에 적지 않고 OS 등록 항목을 읽고 쓴다(`saba_chan_updater_lib::autostart`): Windows `HKCU\…\CurrentVersion\Run`의 `Saba-chan` 값, 리눅스 `$XDG_CONFIG_HOME/autostart/saba-chan.desktop`. 항목은 하나이고 PUT한 데몬의 설치 폴더 실행 파일을 가리킨다. `daemon`은 `saba-chan-gui --tray`(창 없이 트레이, 데몬을 띄운다), `full`은 `saba-chan-gui`. 데스크톱에서는 데몬을 직접 등록하지 않으므로(Windows는 콘솔 창이 뜬다, 로그인 자동 실행은 데스크톱 세션만 돈다) GUI가 없는 설치는 켤 수 없다. 예외는 GUI 없이 설치한 리눅스(화면 없는 서버, D3)의 `daemon`이다: 데몬을 systemd 사용자 서비스 `$XDG_CONFIG_HOME/systemd/user/saba-chan.service`(`ExecStart`=설치 폴더의 `saba-core`, `Restart=on-failure`: 멈춤 API로 끝나면 다시 띄우지 않는다, `KillMode=process`: 데몬이 끝나도 업데이터·게임 서버는 남는다, `WantedBy=default.target`)로 쓰고 `default.target.wants`에 링크해 켠다(`systemctl --user enable`과 같다, 다음 로그인·부팅부터). 링크가 없으면(`systemctl --user disable`) `off`로 보인다. 데스크톱 항목과 서비스 중 하나만 남긴다. 로그인하지 않고 부팅할 때 돌리려면 사용자가 `loginctl enable-linger`를 한 번 한다. 서비스로 돈 데몬(`/proc/self/cgroup`이 `saba-chan.service`)이 업데이트를 적용하면 업데이터는 데몬 대신(CLI가 데몬을 다시 띄우라고 했어도) `systemctl --user start saba-chan.service`로 다시 띄운다(`autostart::service_relaunch`, 직접 띄우면 서비스 밖에서 돈다). 업데이터는 `--after-update`를 설치 폴더의 프로그램에만 붙인다. 단위의 실행 파일 경로는 `%`만 `%%`로 적는다(systemd는 그 자리의 `$`를 풀지 않는다). Windows에서 작업 관리자로 끈 표시(`Explorer\StartupApproved\Run`)나 리눅스 데스크톱이 끈 표시(`Hidden=true`, `X-GNOME-Autostart-enabled=false`)가 있거나 등록된 프로그램이 없으면 `off`로 보이고, 다시 고르면 항목을 새로 써서 그 표시를 지운다. 응답 `{ok, supported, mode, service, lingering}`(macOS는 `supported:false`). `service`: 서비스로 등록됨, `lingering`: `/var/lib/systemd/linger/<사용자>`가 있음(리눅스가 아니면 `null`). 실패는 200 `{ok:false, error_code:"autostart_<코드>", error}`: `unknown_mode`, `gui_missing`, `write_failed`, `unsupported`, `read_failed`. 원격 데몬은 막는다 |
| GET | `/api/system/components` | `config.rs:241` | - | `{version, components:{key:ver}, lastUpdated}` (`installed-manifest.json` + 번들 `versions.toml`). `lastUpdated`는 설치 manifest를 마지막으로 쓴 시각의 unix 초를 담은 문자열이고, 없으면 null |
| GET | `/api/system/metrics` | `system.rs:52` | `?sizes=false` | 시스템 모니터 표본 `{sampled_at_ms, window_ms, cpu:{cores,total_percent,others_percent}, memory:{total_bytes,used_bytes,others_bytes}, disk_io:{read_bps,write_bps,source:"physical"|"process",per_instance:false}, network:{rx_bps,tx_bps,per_instance:false}, disks:[{mount,total_bytes,available_bytes,others_bytes,instances:[{id,bytes}],sizes_at_ms}], instances:[{id,name,running,process_count,cpu_percent,memory_bytes,disk_read_bps,disk_write_bps}]}` (§10) |
| GET | `/api/runtime/preflight` | `ipc/mod.rs:766` | - | `{phase, percent, complete, error, components:{python}}` |

#### 클라이언트와 데몬 수명 (handlers/client.rs)

| Method | Path | Handler | 요청 | 응답 |
|---|---|---|---|---|
| POST | `/api/client/register` | `client.rs:21` | `{kind:"gui"\|"cli", pid?, installRoot?, resident?}` | `{client_id, kind, heartbeat_interval_ms:30000, timeout_ms:90000}`. `resident`: 창 없이 트레이만 든 GUI |
| GET | `/api/clients` | `client.rs` `client_list` | - | `{clients:[{id, kind, pid, installRoot, resident}]}`. 인스톨러·제거기가 데몬을 끄기 전에 이 설치(`installRoot`)의 GUI PID를 받아, 임시 폴더에 풀린 포터블 GUI까지 끈다 |
| GET | `/api/clients/launch` | `client.rs` `client_launchable` | - | 이 PC에서 띄울 수 있는 인터페이스 `{cli, gui}`. GUI·CLI가 서로 전환하는 버튼을 보일지 정한다. `cli`는 Windows이고 데몬 옆에 `saba-chan-cli`가 있을 때, `gui`는 데스크톱 화면이 있고(`tray_host::has_desktop`: Windows는 대화형 세션, macOS는 늘, 그 밖은 `DISPLAY`·`WAYLAND_DISPLAY`) 데몬 옆에 `saba-chan-gui`가 있을 때 true |
| POST | `/api/clients/launch` | `client.rs` `client_launch` | `{kind:"cli"\|"gui", instance?, daemon?}` | GUI ↔ CLI 전환: 데몬이 다른 인터페이스를 띄우고, 부른 쪽은 성공하면 스스로 닫는다. 실행 줄은 정해진 모양뿐이다: CLI는 `tui` 또는 `tui --server=<instance-id>`(전체 화면 메뉴, 그 서버를 골라 연다)를 새 콘솔 창(Windows `CREATE_NEW_CONSOLE`)에, GUI는 인자 없이 또는 `--open-server=<instance-id>`로 띄운다. 인스턴스는 등록된 id만 받고 `--옵션=값` 한 칸으로 넘긴다. `daemon`(CLI만, GUI가 고른 원격 데몬, §1.4.2 7)이 있으면 `--daemon=<값>`을 덧붙이고, 저장한 원격 데몬이 아니면 404 `remote_unknown`·`remote_ambiguous`. 이때 인스턴스는 그 데몬에 있어 이 PC에서 찾지 않는다. 예전 본문의 `view`는 무시한다. CLI는 데몬의 표준 스트림을 넘기지 않고 `CreateProcessW`로 띄워 새 창의 콘솔을 쓴다(`boot_selector::spawn_cli_in_new_console`), GUI는 표준 입출력 없이 띄우고 끝나면 거둔다. 띄운 CLI는 완전히 종료(`unregister?shutdown=true`) 때 닫으려고 기억한다. 이 데몬이 설정 포트(`gui.ipcPort`)와 다른 포트(`SABA_IPC_PORT`)에서 들을 때만 그 주소를 `IPC_BASE`로 준다(`boot_selector::spawn_gui_with`, `client.rs` `gui_ipc_base`, GUI는 `SABA_IPC_PORT`를 읽지 않는다, 이미 떠 있으면 단일 인스턴스가 그 창을 앞으로 가져온다). `{success, pid}`. 400 `invalid_input {field, reason}`, 404 `not_found`(인스턴스)·`cli_not_found`·`gui_not_found`, 501 `unsupported_platform`(CLI는 Windows 밖, GUI는 데스크톱 화면이 없을 때), 500 `launch_failed`. 로컬 전용이라 액션 카탈로그에 없다 |
| GET | `/api/permissions/:guildId` | `permissions.rs` `get_permissions` | - | `{guildId, ownerDiscordId, relayOwnerDiscordId, defaultLevel, preset, chatbotCap, members, commandLevels, ladder:[{id, name?}], presets}` |
| PATCH | `/api/permissions/:guildId` | `permissions.rs` `update_permissions` | `{defaultLevel?, preset?, chatbotCap?, ownerDiscordId?, commandLevels?:{key: level\|null}, ladder?:[{id?, name?}]}` | 바뀐 전체(GET과 같은 모양). 하나라도 틀리면 저장하지 않고 400 `invalid_level`·`invalid_preset`·`invalid_command`·`invalid_member`·`invalid_ladder`·`unknown_key`. `ownerDiscordId`는 고정된 주인을 바꾸는 유일한 길이다(CLI `saba perm owner`). 키 순서와 상관없이 `ownerDiscordId` → `ladder` → `preset` → 나머지 순서로 적용한다(같은 요청에서 새 주인·새 단계를 쓸 수 있고, 같은 요청의 `commandLevels`는 프리셋 뒤에 남는다). `preset`을 보내면(지금과 같은 값이어도) `commandLevels`를 지운다 |
| PUT·DELETE | `/api/permissions/:guildId/members/:discordId` | `permissions.rs` `set_member`·`remove_member` | PUT `{level}`(그 길드 사다리에서 소유자를 뺀 단계) | 바뀐 전체. 주인이면 400 `owner_fixed`. DELETE는 기본 레벨로 되돌린다 |
| GET | `/api/sandbox/unisolated` | `sandbox.rs` `list_unisolated` | - | `{components:[{component, fingerprint, current}]}`. `current`가 false면 파일이 바뀌어 동의가 쓰이지 않는다. 릴레이로는 열지 않는다 |
| POST·DELETE | `/api/sandbox/unisolated/:component` | `sandbox.rs` `allow_unisolated`·`revoke_unisolated` | - | `{success, component, fingerprint}`(DELETE는 `fingerprint:null`). `component`는 `module-<이름>`·`ext-<이름>`. 없으면 404 `unknown_component`. 원격 출처(디스코드·웹 원격·짝지은 PC의 원격 데몬 포트)면 403 `consent_local_only`. 확인되지 않은 구성요소를 격리 없이 돌리기 동의(02 문서 §2.1) |
| GET | `/api/sandbox/extension-gui` | `sandbox.rs` `list_extension_gui` | - | unisolated와 같은 모양. 확인되지 않은 익스텐션의 GUI 번들 싣기 동의 목록 |
| POST·DELETE | `/api/sandbox/extension-gui/:component` | `sandbox.rs` `allow_extension_gui`·`revoke_extension_gui` | - | unisolated와 같은 응답·오류. `component`는 `ext-<id>`. 동의하면 그 익스텐션의 `/gui`·`/gui/styles`를 지금 파일 그대로일 때 서빙한다(03 문서 §4.1) |
| GET | `/api/actions` | `actions.rs` `list_actions` | query `surface?`(gui·cli·discord·chatbot), `instance?`, `locale?` | `{actions:[{id, kind, risk, surfaces, scope, input, titleKey, title, description?, route, cli}], locale}`. `cli`는 같은 일을 하는 CLI 한 줄(I4-c "CLI로 보기"): `saba-chan-cli do <id>`에 인스턴스 액션이면 ` --server <server>`, 필수 필드(모듈 명령·조회는 필수 입력)마다 ` <이름>=<이름>`. 꺾쇠 자리는 화면이 채운다. `instance`를 주면 그 서버 명령이 `server.command:<이름>`으로, 모듈 `[[queries]]`가 `server.query:<이름>`(`kind=query`, risk `safe`, 모든 표면, 제목·설명은 모듈 locales `queries.<이름>.*` → module.toml 값 → 이름, route `POST /api/instance/:id/query/:name`)으로 펼쳐진다. 조회 입력은 선언에 없는 키·범위·형식을 `invalid_input`으로 거부하고, 릴레이 `query`도 받는다. 챗봇으로 나가는 조회 결과는 콘솔과 같이 IP·비밀처럼 보이는 값을 가린다. 선언은 `src/actions/mod.rs` `ACTIONS` 하나(P2) |
| POST | `/api/actions/:id/plan` | `actions.rs` `plan_action` | `{input, instance?, locale?}` | `{success, planId, actionId, instance, summary:[{labelKey, label, value}], risk, needsConfirm, expiresAt}`. 부작용 없음. `needsConfirm`은 `danger`이거나, 챗봇 경로(`via_chatbot`)의 관리(admin) 액션일 때 true. 사람이 직접 친 caution 관리 명령은 false라 바로 `run`한다. 400 `invalid_input {field, reason}`, 403 `action_not_allowed`·`PERMISSION_DENIED {args}`·`NODE_NOT_ALLOWED {args:{instance}}`(이 길드의 `allowedInstances`에 없는 인스턴스), 404 `unknown_action`. 켜진 서버의 `backup.restore`는 409 `server_running`이다(검수 14번). 입력 스키마의 칸에 `optionsFrom {action, list, value, label}`이 있으면 그 값은 같은 인스턴스에서 `action`을 부른 결과의 `list` 배열 항목 중 `value` 키에서 고른다(`label` 키가 보일 이름). 지금은 `backup.restore`의 `name`(`backup.list`의 `backups[].name`, 이름은 `label`)이다. `backup.list` 액션 결과의 `backups[]` 항목에는 요청 언어·PC 시간대로 쓴 `label`(`2026-10-20 21:03 · 직접 · 120MB`)이 붙는다(라우트 `GET /api/instance/:id/backups`에는 없다, `summary::label_backups`) |
| POST | `/api/actions/:id/run` | `actions.rs` `run_action` | `{input, instance?}` 또는 `{planId}` | `{success, actionId, status, result, text?}`(`result`는 밑 라우트 응답 그대로. `text`는 원격 요청에만 붙는 사람용 요약, 05 relay `action`). 404 `plan_expired`, 403 `plan_not_yours`. 챗봇이 만든 확인 계획은 요청자 본인의 확인 표지(`Requester::confirmed_by_requester`, relay `confirmedBy`)가 없으면 409 `confirm_required`이고 계획은 남는다. 데몬 라우터를 안에서 부르므로 감사 기록은 원래 요청의 출처로 남는다. 여러 단계 계획은 `:id`가 `plan`(또는 빈 값). 원격 요청의 `instance.settings.set`이 성공하면 바꾸기 전 값으로 되돌리는 계획을 하나 더 만들어 `undoPlanId`·`undoExpiresAt`을 싣는다(같은 요청자, 10분, 전에 값이 없던 키는 빠진다). 원격 관리(admin) 실행은 길드당 최근 한 시간 `nodeSettings[guild].adminRunsPerHour`(기본 10)회까지이고 넘으면 429 `rate_limited {args:{perHour, retryInSecs}}`(여러 단계 계획은 관리 단계가 있을 때만 한 번). 확인을 기다리는 계획은 500개까지(넘으면 오래된 것부터 버린다). `instance.settings.set` 계획 요약의 `value`·`before`는 비밀값 키(어느 깊이든)를 `"***"`로 가리고, 묶음 값의 `before`는 바꾸는 칸만 싣는다. 원격 `instance.settings.set`은 묶음 값(객체·배열)을 `invalid_input`으로 거부한다. PATCH 액션의 선택 필드는 `null`(기본값으로 되돌리기)을 받는다. 원격에서 만든 서버(`instance.create`, 한 액션이든 여러 단계든)는 그 길드에 `allowedInstances` 목록이 있으면 목록에 더한다 |
| POST | `/api/actions/plan` | `actions.rs` `plan_steps` | `{steps:[{id, instance?, input}], locale?}` | 여러 단계 계획(I1, `src/actions/steps.rs`): 1~8단계. `instance`가 `@N`이면 N번째 단계(앞에 있는 `instance.create`)가 만들 서버이고, 그런 단계는 고정 액션 중 인스턴스를 id로 받는 것만 된다(`server.status`·모듈 명령·조회 안 됨). 단계마다 검증·판정하고 틀리면 그 오류에 `step`(1부터)과 `field`(`steps[n].<키>`)를 붙인다. 원격 검사는 새 서버의 모듈(`instance.create`의 `module_name`)로 한다. 응답은 한 액션 계획과 같고 `actionId:"plan"`, `steps:[{n, actionId, instance}]`, `summary`는 단계마다 `actions.field.step` 줄(`"N. 제목"`) 뒤에 그 단계 요약. 2단계 이상이면 늘 `needsConfirm`. 실행(`/api/actions/plan/run {planId}`)은 확인한 사람으로 단계를 다시 판정하고 순서대로 부르며 실패한 단계에서 멈춘다: `{success, actionId:"plan", runId, steps:[{n, actionId, instance?, status:"done"\|"failed"\|"skipped", code?, error?, undoPlanId?}], stoppedAt?}`. 원격이면 `error` 원문은 싣지 않는다. 새 서버를 쓰는 단계 앞에서는 그 서버 설치(프로비저닝)가 끝나기를 기다리고(최대 30분, 실패면 `provision_failed`), 실행 때 실제 id로 다시 풀고 다시 판정한다. 원격에서 만든 서버는 그 길드에 `allowedInstances` 목록이 있으면 목록에 더한다. 40초 안에 끝나지 않으면 `{success:true, running:true, runId, steps}`를 먼저 돌려주고 뒤에서 이어 가며, 끝나면 이벤트 `plan.done {runId, guildId?, ok, steps, stoppedAt}`. 같은 길드(로컬은 같은 출처)에서는 한 번에 하나(409 `plan_running`). `backup.restore` 단계는 앞에 같은 인스턴스의 `server.stop` 단계가 있으면 켜짐 검사를 계획 때 하지 않고 실행 때 라우트가 한다("먼저 끄고 복원"). 정지 단계는 서버가 끝난 뒤에 성공하고, 실패하거나 `graceful_stop_unavailable`이면 복원은 `skipped`다. `server.stop`은 `force` 칸이 없어 원격 계획에서 강제 종료를 받지 않는다 |
| GET | `/api/actions/runs/:runId` | `actions.rs` `run_status` | - | 여러 단계 실행의 진행. 끝났으면 `{done:true, result}`, 아니면 `{done:false, steps}`. 시작한 쪽만(403 `plan_not_yours`), 한 시간 둔다(404 `run_not_found`) |
| POST | `/api/client/:id/heartbeat` | `client.rs` `client_heartbeat` | `{resident?}`(없으면 false) | `{ok}` / 404. 등록된 GUI가 모두 `resident`면 업데이터가 그 GUI를 `--tray`로 다시 띄운다(`updates.rs` `relaunch_target`) |
| DELETE | `/api/client/:id/unregister` | `client.rs` `client_unregister` | query `shutdown=true\|false` | `{ok}` / 404. `shutdown=true`(완전히 종료): `clients/launch`로 띄운 CLI 가운데 아직 그 프로세스인 것(PID·띄운 시각)을 닫고(`ClientRegistry::close_launched_clis`), 남은 클라이언트가 없으면 데몬도 끝낸다 |
| POST | `/api/daemon/shutdown` | `client.rs:12` | - | `{ok, message}`. 200ms 뒤 `stop_requested` cancel |
| GET | `/api/daemon/console` | `client.rs` `daemon_console` | `since, count=200, spans` | `{lines:[{id,timestamp,level,target,message,spans?}], count, epoch}`. `spans=1`이면 `message`에 데몬 로그 규칙(`highlight.rs` `daemon_log`: 맨 앞 `[태그]`, 주소·IP·UUID, 한 덩어리 버전 번호(`v0.1.5`·`1.0.5.102999`·`0.1.11-beta.2`), 경로, 따옴표 이름, `키=`, 실패·성공 낱말, 단위 붙은 수·수, 화살표)으로 토큰 구간 `[[시작,끝,토큰]]`(UTF-16)을 붙인다. 토큰은 02 §`HighlightRuleToml`과 같은 이름이다. `id`는 데몬을 켤 때마다 1부터 다시 센다. `epoch`(버퍼를 만든 시각 밀리초, 데몬을 다시 켜면 커진다)가 바뀌면 클라이언트는 이어 읽던 `since`를 버리고 처음부터 읽는다(L55) |

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
| GET/PUT | `/api/extensions/:id/config` | `extension.rs` `get_extension_config`·`save_extension_config` | `{config}` / 객체 머지(null이면 키 삭제). PUT은 설치되지 않은(발견되지 않은) id면 저장하지 않고 404 `not_found` |
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
| GET | `/api/relay/host/:host_id/nodes` | `relay.rs` `list_host_nodes` | `/api/hosts/:id/nodes`. 답에 있는 노드 주인 `ownerDiscordId`를 아직 주인이 없는 길드에 고정한다(`pin_listed_owners`, 명령과 같은 `pin_owner` 규칙: 고정된 주인은 바꾸지 않고 다른 값은 `relayOwnerDiscordId`) |
| GET | `/api/relay/node/:guild_id/chatbot/models` | `relay.rs` `get_node_chatbot_models` | `/api/nodes/:id/chatbot/models` 서명 프록시. 릴레이 답을 그대로 전달한다: 200 `{provider, models, selected, effectiveModel}`, 409 `NO_API_KEY`, 422 `INVALID_API_KEY`·`KEY_PERMISSION`(키 권한 부족)·`QUOTA_EXCEEDED`, 503 `PROVIDER_UNAVAILABLE` |
| GET/PUT | `/api/relay/node/:guild_id/channels` | `relay.rs:261/282` | `/api/nodes/:id/channels` 서명 프록시. 챗봇 채널(여럿)·음악 채널(하나) 배정의 원본은 릴레이다. 응답 `{guildId, chatbot:[id], music:id\|null, botAvailable, channels:[{id,name,category}]}`, PUT 본문 `{chatbot?, music?}`(준 키만). 오류 본문 `{error: CODE, channelId?}`는 그대로 전달(객체가 아닌 본문은 데몬이 400 `INVALID_BODY`). 데몬 config의 `chatbot.guilds.*.channelIds`는 `ChatbotConfig::migrate`가 지운다 |
| GET | `/api/relay/node/:guild_id/members` | `relay.rs` `list_node_members` | `/api/nodes/:id/discord-members`(15초 제한). 실패하면 `/members`(권한을 등록한 멤버만)로 폴백한다. query `withStatus=1`이면 `{members, partial, reason?}`로 답한다: 폴백이면 `partial:true`, `reason`은 `bot_unavailable`·`relay_error`·`relay_unreachable`. 없으면 예전처럼 배열만 |
| GET | `/api/relay/node/:guild_id/legacy-members` | `relay.rs` `list_node_legacy_members` | `/api/nodes/:id/members` 서명 프록시. 릴레이 본문(`[{id, username, displayName, level}]`)을 그대로, 실패하면 오류 상태와 본문(빈 목록으로 바꾸지 않는다) |
| GET | `/api/relay/bot-invite` | `relay.rs:430-449` | query `relayUrl?`. relay `/api/bot-invite`(공개, 서명 없음)를 그대로 전달. 주소는 서명 요청과 같은 규칙(설정한 릴레이 또는 루프백)으로 정한다. 봇 초대 URL `{url}` |
| POST | `/api/relay/pair/initiate` | `relay.rs` `initiate_pairing` | `/api/pair/initiate`에 `{relayUrl?(GUI가 보낸 값), deviceName(이 PC 이름: `COMPUTERNAME`·`HOSTNAME`·`/etc/hostname`, 64자), currentToken?(저장된 노드 토큰. 요청이 설정한 릴레이로 갈 때만)}`만 보낸다 (서명 없음, `pairing_body`). 릴레이는 `currentToken`의 PC를 다시 연결한다(05 §4.4). 릴레이 주소가 https(또는 루프백 http)가 아니면 400 `{error, error_code:"invalid_relay_url"}`(`pairing_relay_url`): 요청한 `relayUrl`이 그렇거나, 요청이 없을 때 쓰는 저장된 `cloud.relayUrl`이 그렇다 |
| GET | `/api/relay/pair/:code/status` | `relay.rs` `poll_pairing_status` | `secret?, relayUrl?`. 릴레이 주소 규칙은 시작과 같다(아니면 400 `invalid_relay_url`). relay 응답을 그대로 돌려준다. `status:"claimed"`이면 `nodeToken`을 `.node_token`에 저장하고(같은 값이 이미 있으면 다시 쓰지 않고 relay agent도 다시 띄우지 않는다, `store_node_token`) `tokenSaved:bool`을 더한다. `false`면 토큰이 이 PC에 없다(수령 기한이 지나 `nodeToken:null`이거나 쓰기 실패): 클라이언트는 연결됨으로 보이지 않고 다시 페어링하게 한다 |
| GET | `/api/relay/link-requests` | `relay.rs` `list_link_requests` | relay `GET /api/link-requests` 서명 프록시. 디스코드에서 코드 없이 `/사바쨩 연결`한 요청 중 이 PC에 온 것(J1-b) |
| POST | `/api/relay/link-requests/:request_id` | `relay.rs` `answer_link_request` | `{accept:bool}` → relay `POST /api/link-requests/:id` 서명 프록시. 수락하면 relay가 그 길드를 이 PC에 묶는다. 응답은 relay 것 그대로 |
| GET | `/api/relay/agent` | `chat.rs:19` | 에이전트 상태 `{running, relayUrl, hasToken, agentVersion, lastHeartbeatAt, lastError, lastErrorCode?, minVersion?, commandsHandled}`. `lastErrorCode` `invalid_relay_url`: 저장된 릴레이 주소가 https나 루프백 http가 아니라 에이전트를 띄우지 않았다(데몬 시작은 그대로). `token_replaced`: relay가 401 `TOKEN_REPLACED`(같은 디스코드 계정이 다른 PC를 페어링)로 답해서 에이전트가 멈췄다. 이 PC에서 다시 페어링하면 새 토큰 저장이 에이전트를 다시 띄운다. `agent_too_old`(+`minVersion`): 릴레이가 이 PC 버전을 받지 않는다. 시작 때 `/info`의 `minAgentVersion`보다 낮으면 에이전트를 띄우지 않고, 연결 중 heartbeat·poll이 426 `AGENT_TOO_OLD {minVersion}`을 받으면 세션은 둔 채 poll을 heartbeat 주기(60초)로만 다시 묻는다. heartbeat가 성공하면 코드와 `minVersion`을 지운다 |
| GET·PATCH | `/api/discord/:guild_id/settings` | `discord.rs` `get_settings`·`patch_settings` | 디스코드 길드 설정(I2) `{guildId, locale, sleeping, music}`. PATCH는 바꿀 키만(`music` 안도 부분) → `{success, settings, sync:{guildId, ok, status?, error?}}`. 틀린 값 400 `invalid_input {field, reason}`, 페어링이 없으면 409 `not_paired`. 저장 전에 릴레이 `PATCH /api/nodes/:g/settings`로 보내고, 릴레이 값과 다른 키만 보낸다(같으면 보내지 않는다). 자동 언어로 바꿀 때는 지금 GUI 언어 `language`도 싣는다. 릴레이가 400이면 저장하지 않고, 닿지 않으면 저장한 뒤 `sync.ok:false`. 에이전트가 연결되면 길드마다 같은 방식으로 맞추고, 블록이 없는 길드는 릴레이 값을 한 번 가져와 원본으로 삼는다(`discord::sync_on_connect`). 그 전에 온 GET·PATCH도 블록이 없으면 릴레이 값에서 출발한다(못 읽으면 기본값). 읽고-바꾸고-저장하기는 한 번에 하나씩 한다. 액션 `discord.settings.set`(admin, 디스코드는 소유자, 챗봇 없음. 릴레이가 실어 준 디스코드 서버 주인 `guildOwnerDiscordId`가 요청자와 같으면 레벨과 상관없이 받는다: `Def.guild_owner`, 원격 데몬 R2b). 조회 액션은 쓰는 곳이 없어 지웠다(라우트 GET은 그대로). 원격 요청은 자기 길드만 다룬다(`guild`를 그 길드로 고정, 다르면 `invalid_input`) |
| POST | `/api/relay/agent/start`, `/stop` | `chat.rs:24/36` | 토큰이 없으면 409 `node_token_missing`, 저장된 `cloud.relayUrl`이 https나 루프백 http가 아니면 409 `invalid_relay_url`(예전에 저장된 값도 같다. 노드 토큰을 평문으로 보내지 않는다, `relay_agent::session_relay_url`) |
| POST | `/api/chat/command` | `chat.rs:10` | `{text, guild_id?, user_id?, locale?}` → `{reply, code, args}`. 답 문구는 `locale`, 없으면 GUI 언어. `code`는 입력을 알아보지 못했을 때만(`UNKNOWN_COMMAND`·`INSTANCE_NOT_FOUND`), 아니면 null |
| GET/PUT/DELETE | `/api/chatbot` | `chatbot.rs:78/265/287` | AI 챗봇 설정 (`config.json`의 `chatbot`). 바뀔 때마다 릴레이에 동기화, 클라이언트에는 `hasApiKey`·`pendingRelayDelete`만. GET은 바꿀 수 있는 키와 그 기본값(릴레이 기본값 표)을 `defaults`로도 준다(CLI `chatbot show`가 키 목록으로 쓴다). PUT은 릴레이 기본값 위에 전체 설정을 얹어 보낸다(`chatbot.rs:31-65`). 한 길드라도 값을 거절(400)하면 저장하지 않고 400 `validation_failed`를 주며, 남은 길드에는 보내지 않고 이미 받은 길드는 옛 값으로 되돌린다(`push_or_restore`). DELETE가 릴레이에 닿지 못하면 `pendingRelayDelete:true`로 남기고 다음 연결 때 다시 지운다(`:219-263`) |
| DELETE | `/api/chatbot/memory` | `chatbot.rs:333` | query `guildId?` |
| GET | `/api/instance/:id/words` | `words.rs` `get` | - | 그 서버를 부르는 낱말 `{id, name, module, defaultWords, words, commands:[{name, label, risk, builtin, moduleAliases, aliases}]}`. 낱말은 인스턴스에 딸려 어느 길드에서나 같다. `defaultWords`는 정하지 않아도 그 서버를 부르는 기본 호출명이다: 인스턴스 이름, 그리고 같은 모듈의 서버가 모든 서버 중 이것 하나뿐이면 게임명(아래 §4 `game_names`)·module.toml 별명·모듈 이름(대소문자만 다른 것은 하나로, 다른 모듈과 다투는 별명과 다른 서버의 이름·id·낱말은 뺀다, `chat::words::default_words`). 명령은 이름마다 한 줄이고, 모듈이 내장(start·stop·status)과 같은 이름을 정의하면 이름표·위험도는 모듈 선언을 쓴다(`builtin`은 true). 없는 인스턴스는 404 `not_found` |
| PUT | `/api/instance/:id/words` | `words.rs` `put` | `{words, commands:{<명령>:[..]}}` | `{success, id, words, commands}`. 규칙 위반은 400 `validation_failed {details:[{field, reason, word?, conflictWith?}]}`(`field`: `words[i]`·`commands.<명령>[i]`·`words`·`commands.<명령>`·`input`, `reason`: `invalid`·`reserved`·`conflict`(같은 서버 안에서 겹치면 `conflictWith`가 그 서버 이름)·`unknown_command`·`too_many`, `word`는 문제가 된 낱말). 서버 낱말은 모든 서버(길드가 쓰는 서버만이 아니다)의 이름·id·낱말과 겹치면 안 된다. 없는 인스턴스는 404. 빈 값이면 항목을 지운다. 검사와 저장은 설정 잠금 안에서 한다(`update_server_words`). 로컬 전용(릴레이 에이전트는 부르지 않고 액션 카탈로그에도 없다). 0.1.10의 `/api/guilds/:guild/instances*`는 없앴다 |
| PUT | `/api/guilds/:guild/allowed-instances` | `words.rs` `put_allowed` | `{instances: [id] \| null}` | `{guildId, allowedInstances: [id] \| null}`. 그 길드가 보고 부를 수 있는 서버(`nodeSettings[guild].allowedInstances`). null은 제한 없음(키 삭제), 빈 배열은 아무 서버도 못 쓴다. 겹친 id는 하나로 줄인다. 없는 인스턴스 id는 400 `validation_failed {details:[{field:"instances", reason:"unknown_instance", id}]}`, 모양이 틀리면 `reason:"invalid"`. 설정 잠금 안에서 저장한다(`update_node_settings`). 로컬 전용(액션 카탈로그에 없다) |
| GET·PATCH | `/api/chatbot/guilds/:guild` | `chatbot.rs` `get_guild`·`patch_guild` | PATCH `{설정 키: 값\|null}` | 길드 하나의 챗봇 설정(I2-b): `{success, guildId, settings, hasApiKey}`(`settings`는 릴레이 기본값 위에 실제 값). PATCH는 그 길드 값만 합쳐 `PUT /api/chatbot`과 같이 저장·동기화한다. 릴레이 기본값 표에 없는 키(`apiKey` 포함)는 400 `validation_failed`. 액션 `chatbot.settings.get`(safe, 디스코드는 소유자)·`chatbot.settings.set`(admin, 디스코드는 소유자, 챗봇 표면 없음: 챗봇이 자기 확인을 끄지 못하게). 원격은 자기 길드만 |


Relay 서명 방식(`relay.rs:115-161`):

- 헤더: `Authorization: Bearer <node_token>`, `x-request-timestamp`, `x-request-nonce`, `x-request-signature`.
- 서명값: `HMAC-SHA256(secret, "METHOD\npath\nts\nnonce\nbody")`.
- 고정 헤더 `X-Saba-Client: 1`이 모든 relay 요청에 붙는다. timeout은 5초다(`relay.rs:69-81`).

#### 업데이트 (ipc/updates.rs)

| Method | Path | Handler | 요청 | 응답 |
|---|---|---|---|---|
| GET | `/api/updates/status` | `updates.rs:129` | - | `{ok,last_check,next_check,checking,error,updates_available,components:[{component,display_name,current_version,latest_version,update_available,downloaded,installed}],profile}` (Locales 제외. `profile`은 설치 구성 `"desktop"`\|`"headless"`, `install-profile.json`, 05 §1) |
| POST | `/api/updates/check` | `updates.rs:169` | - | `{ok,updates_available,update_names,components(+release_notes),last_check}`. Locales 업데이트가 있으면 백그라운드에서 조용히 적용 |
| POST | `/api/updates/download` | `updates.rs:366` | 선택 `{components?:[key]}`. 본문이 없거나 공백이면 전부, 깨진 JSON은 400 `invalid_body`(`updates.rs:348-357`) | `{ok,downloaded,count,errors?}`. 전부 받기에서 일부가 실패해도 받은 것은 `downloaded`에, 실패한 것은 `errors[{component,error}]`에 싣고 `ok:false` |
| GET | `/api/updates/download/progress` | `updates.rs:450` | - | `{ok,component,bytes_received,total_bytes,active}` |
| POST | `/api/updates/apply` | `updates.rs:475` | download와 같은 본문 규칙 | `{ok,applied,needs_updater,requires_updater,errors}` |
| POST | `/api/updates/install` | `updates.rs` `install_updates` | - | 확인 → 전부 받기 → apply → `requires_updater`면 `launch-updater`까지 한 번에. apply 답에 `downloaded`, 받지 못한 것 `download_errors`, 업데이터를 띄웠으면 `updater_launched: true`. 액션 `update.apply`(디스코드 `/사바쨩 업데이트`, 주인만, 챗봇 표면 없음)가 이 경로다 |
| POST | `/api/updates/launch-updater` | `updates.rs:609` | 선택 `{relaunch:[exe, args...]}` | `{ok, updater_launched, daemon_stopping}`. 404(업데이터 없음) / 409(받은 업데이트 없음). 등록된 GUI(`/api/client/register`의 `pid`, 실행 파일 이름이 GUI일 때만)가 떠 있으면 다시 띄울 것을 그 GUI로 바꾸고, 종료 전에 3초 기다려 남은 GUI를 끈다. 헤드리스 설치면 `--headless`를 붙이고, 다시 띄울 것을 받지 않았으면 데몬 자신(`<saba-core> --spawned`)을 다시 띄우게 한다(`updater_args`) |
| GET/DELETE | `/api/updates/last-result` | `updates.rs` `get_last_result`/`clear_last_result` | GET `?lang=`, `?include_seen=true` | 마지막 적용 결과 `{result, version, notes, acknowledged}` / 확인 처리(마커는 남기고 `acknowledged`만 적는다, 05 참고) |
| GET | `/api/updates/integrity` | `updates.rs:814` | - | `{ok,checked_at,overall,total,verified,failed,skipped,components[...]}` |
| GET | `/api/updates/config` | `updates.rs:844` | - | `{ok, config, mock_relay}` |
| PUT | `/api/updates/config` | `updates.rs:859` | 부분 `{enabled,check_interval_hours(1-8),auto_download,install_root,relay_url,channel}` | `{ok, config}`. `channel`은 `stable`·`rc`·`beta`만 받고 그 밖은 400 `invalid_channel`(05 §1 채널). `config.json`의 `updates` 절에 저장한다(`:899-902, 936-942`). 비어 있지 않은 `relay_url`은 개발 빌드(`mock-relay`)만 받고, 릴리즈 빌드는 403 `relay_url_not_settable`(D7: 출처는 config.json에서만) |

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
| `user_settings` | BTreeSet<String> | 사용자가 직접 고른 설정 키. 외부 데이터 루트의 출처 판정에도 쓴다(02 §2.1) |
| `imported_settings` | BTreeSet<String> | 마이그레이션 `import_settings`가 판정한 path형 설정 키. 기본은 빈 집합이며 값이 실제 바뀌면 해당 출처를 지운다. 값은 `module_settings`에만 둔다 |
| `server_version` | Option<String> | |
| `extension_data` | HashMap<String,Value> | 설치 방식 익스텐션 키(`install_method_<방식>`, `<방식>_<이름>`, `instance/mod.rs:142-166`) 등 |
| `required_extensions` | Vec<String> | 모듈의 `install.requires_extensions`를 복사한다 |
| `schedule` | Vec<ScheduledTask> | 예약 작업 (백업·재시작·공지·업데이트) |
| `backup_keep` | Option<u32> | 자동 백업 보관 개수 (기본 10). 예약·업데이트 전·복원 전 백업을 종류마다 따로 센다. 수동 백업은 지우지 않는다. 원격(디스코드·챗봇) 요청이 만든 백업은 정리를 하지 않는다(원격 복원을 거듭해도 앞서 남긴 안전 백업이 밀려 지워지지 않는다. 넘친 것은 이 PC에서 만드는 다음 백업이 정리한다, D-B11) |

- RCON 필드(`rcon_port`, `rcon_password`)는 없다. 예전 `instance.json`·`settings.json`에 남은 RCON 키는 로드할 때 한 번 지우고 다시 쓴다(`instance/mod.rs:439-468, 483-500`).
- 시작할 때(`server.rs` `fill_passwords`, 두 시작 경로 공용) 모듈 `[protocols] supported`에 `rest`가 있으면 `ensure_passwords(generate)`가 비어 있는 REST 비밀번호를 16자 영숫자로 채운다. REST를 지원하지 않는 모듈의 서버에는 REST 계정을 만들지 않고, 예전 데몬이 만들어 둔 `rest_username`·`rest_password`는 지운다. `generate`는 `GuiSettings.autoGeneratePasswords`(기본 true)다. 꺼져 있으면 비워 두고, 모듈이 게임 설정의 값을 쓰거나 스스로 만들어 `credential_map`으로 데몬에 돌려준다(Palworld는 INI `AdminPassword`가 비면 임의 값을 써 넣는다). 이미 있는 비밀번호는 켜져 있어도 바꾸지 않는다.
- 비밀번호 생성은 `constants::generate_random_password`가 uuid v4 바이트에 `% 62`를 적용하는 방식이다(`constants.rs:188-201`). 모듈로 편향이 있고, 두 번째 uuid는 계산만 하고 쓰지 않는다.

### 4.2 영속화 (`InstanceStore`, `instance/mod.rs:238-477`)

- 루트는 `constants::resolve_instances_dir()`이며, 기본값은 `<data_dir>/instances`다.
- 저장 형식(JSON, pretty):
  - `<root>/order.json`: id 배열
  - `<root>/<id>/instance.json`: `module_settings`를 뺀 메타데이터
  - `<root>/<id>/settings.json`: `module_settings`. JSON이 아니면(손으로 고치다 깨진 경우) 빈 설정으로 올리고 경고를 남기며, 다음 저장이 덮기 전에 원본을 `settings.json.corrupt`(다른 내용이 이미 있으면 `.corrupt-<초>`, 같은 내용의 사본이 이미 있으면 새로 만들지 않는다, unix는 0600으로 만든다)로 남긴다. 사본을 남기지 못하면 그 인스턴스를 올리지 않는다(원본을 덮지 않게)
  - Unix에서는 두 파일 모두 0600으로 둔다.
- 파일을 통째로 바꾸는 저장은 모두 공용 crate `saba-atomic-file`(`atomic-file/src/lib.rs` `write`)을 쓴다. 같은 폴더의 고유 임시 파일 `.<이름>.<pid>-<n>.tmp`에 다 쓰고 fsync한 뒤 rename한다(Unix는 폴더도 fsync). `Mode::Plain`은 원래 파일의 권한을 잇고, `Mode::Secret`은 처음부터 주인만 읽는다(Unix 0600, Windows는 폴더 ACL을 물려받는다). 비밀 모드: `instance.json`·`settings.json`(`instance/mod.rs` `write_atomic_bytes`), `rcon.json`, `.node_token`, IPC 토큰, 러너 토큰, 업데이터 실행 표, `config.json`(챗봇 API 키). 일반 모드: `order.json`, `managed.pid`, 종료 이력, AppContainer 기록, 루프백 응답, 러너 상태 파일, 업데이터 기록·staging 파일. 인스턴스 폴더의 임시 파일 이름(`.`로 시작하고 `.tmp`로 끝남)은 데몬 항목으로 보아 모듈 설정 경로로 받지 않는다(`supervisor/mod.rs` `is_daemon_entry`).
- 노드 토큰은 `relay.rs`의 `node_token_path`·`write_node_token`(같은 값이면 쓰지 않음)·`adopt_node_token`(hostId를 맞추고 바뀌었으면 relay agent 재시작)으로만 쓴다. 페어링 완료, `PUT /api/config/node-token`, 가져오기가 모두 이것을 부른다(가져오기는 설정 절까지 저장한 뒤).
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
| 동작 | `autoRefresh`(true), `refreshInterval`(2000), `ipcPort`(57474), `consoleBufferSize`(2000), `autoGeneratePasswords`(true), `portConflictCheck`(true), `osNotifications`(true, GUI 셸의 중대 알림 OS 알림) |
| 테마 | `theme?`, `accentColor`("#667eea"), `accentSecondary`("#764ba2"), `useGradient`(true), `fontScale`(100), `enableTransitions`(true), `consoleSyntaxHighlight`(true), `consoleBgColor`, `consoleTextColor`, `sidebarCompact`(false), `windowOpacity`(100), `consoleFontScale`(100), `systemMonitorOpen`(true, 서버 페이지 시스템 모니터 펼침) |
| 창/언어 | `windowBounds?{width:1200,height:840,x?,y?}`, `language?` |
| 다룰 데몬 | `daemonTarget?`(GUI가 고른 원격 데몬 hostId, 없으면 이 PC. §1.4.2 7, 04 §1.1) |
| 서버 목록 보기 | `daemonView?`(`switch` 고른 데몬만, `unified` 다른 데몬의 서버도 데몬별로. 없으면 switch. §1.4.2 8, 04 §1.2) |

**BotConfig** (camelCase, `config.json`의 `relay`, `config_store/mod.rs:118-133`). 로컬 봇용 필드(`token`, `autoStart`, `mode`, `music*`)는 지워졌고, 옛 파일에 남아 있어도 무시된다.

- `prefixes`(문자열 목록, 기본 `[]`: 비어 있으면 릴레이가 응답 언어의 기본값을 쓴다. ko `사바쨩`, ja `サバちゃん`, 그 밖 `Saba-chan`). 각각 공백 없는 1~32자, 10개까지이고 어기면 `PUT /api/config/bot`이 400이다. 예전 `prefix` 문자열은 읽을 때 옮긴다(`"!saba"`·빈 값 → `[]`, 그 밖 → `[값]`, `BotConfig::normalize`). `cloud{relayUrl,hostId}` (relayUrl은 빈 값(기본 릴레이)·https·루프백 http(`localhost`·`127.0.0.1`·`[::1]`, 모의 릴레이)만 받는다(`config_store::relay_url_problem`). 저장·가져오기(`check_sections`)에서 값이 바뀔 때만 보고, 이미 저장된 값은 읽기와 다른 키 저장을 막지 않는다. hostId는 노드 토큰 `sbn_<hostId>.…`을 따른다: 페어링 완료·`PUT`·`DELETE /api/config/node-token`이 같이 맞춘다)
- `moduleAliases`, `commandAliases`, `instanceAliases`: 예전 전역 별명. 데몬이 시작할 때 아래 `serverWords`로 옮기고 비운다(`words::migrate_on_start`, `chat::words::migrate`). `instanceAliases[id]`는 그 서버 `words`로, 모듈 별명은 그 모듈 서버가 하나뿐일 때만 그 서버 `words`로(여럿이면 버리고 로그), `commandAliases[모듈]`은 그 모듈 서버마다 `commands`로 간다. 0.1.10의 길드별 낱말 `nodeSettings[guild].instances[id]`도 같은 때 길드 이름 순으로 그 서버 낱말에 합치고 길드에서 걷는다(지워진 인스턴스 것은 버린다). 다른 서버 이름·id와 겹치거나 다른 서버가 먼저(id 순) 가진 낱말은 버린다
- `nodeSettings`, `cloudNodes`, `cloudMembers`
- `nodeSettings[guildId]`의 디스코드 명령 권한 (P1, `chat/permissions.rs`). 원본은 이것 하나다.
  - 키: `ownerDiscordId`(고정된 주인), `relayOwnerDiscordId`(릴레이가 알려 온 다른 주인, 경고용), `ladder`(단계 사다리, 아래), `defaultLevel`(1), `members{discordId: 레벨}`(소유자 제외), `preset`("default"·"loose"·"strict"), `commandLevels{"<instanceId>/<command>": 레벨}`(금지 제외), `chatbotCap`(0), 예전 금지 목록 `memberPermissions`(읽기만. `{discordId: {instanceId: [명령]}}`에 있는 명령은 그 사람에게 주인(3) 레벨을 요구한다. 채팅 명령(`NodePermissions::check`)과 액션 카탈로그(`actions::authorize`, 모듈 명령 액션과 내장 `server.start|stop|restart|status`) 모두 같다).
  - 레벨: 소유자 3, 관리자 2, 유저 1, 챗봇 0, 금지 -1. 길드마다 `ladder`(높은 것부터 `[{id, name?}]`, 없으면 기본 다섯 단계)로 단계를 더하고(id 10~999, 이름 1~32자, 20개까지) 순서를 바꾸고 지울 수 있다(사용자 지정 2026-10-25). 소유자는 늘 맨 위, 금지는 늘 맨 아래, 그 사이에 하나 이상이다(아니면 `invalid_ladder`). 높고 낮음은 숫자가 아니라 사다리 순서다(`NodePermissions::meets`). 사다리를 바꿀 때 지운 단계의 멤버·`defaultLevel`·`chatbotCap`은 옛 사다리에서 바로 아래 남은 단계로(아래에 금지밖에 없으면 금지, 지워서 권한이 넓어지지 않게), 그 단계를 요구하던 `commandLevels`는 바로 위 남은 단계로 옮겨 저장한다(`apply_ladder`). 프리셋표와 관리 액션 값이 가리키는 기본 단계를 지웠으면 기본 단계 순서(소유자·관리자·유저·챗봇)에서 더 엄한 쪽 남은 단계를 쓴다(`preset_need`·`admin_need`). 요구 레벨은 `commandLevels` 덮어쓰기, 없으면 프리셋표[risk]다. 프리셋은 한 번 적용하는 틀이다: 프리셋을 바꾸면 `commandLevels`를 지워 모두 그 프리셋 값을 따른다(사용자 지정 2026-10-25). 채팅의 내장 명령(`start`·`stop`·`restart`·`status`)은 `<instanceId>/<command>`가 없으면 액션 키 `<instanceId>/server.<command>`도 본다(액션 경로와 같은 덮어쓰기, `NodePermissions::required`). default 프리셋은 safe 0, caution 1, danger 2이고, loose는 0/0/1, strict는 1/2/3이다. 내장 명령의 risk는 `status` safe, `start`·`stop` caution, 모듈 명령은 선언값(없으면 caution).
  - 액션 카탈로그(P2): 서버 명령 액션은 위와 같은 키(`<instanceId>/<command>`), 그 밖의 인스턴스 액션은 `<instanceId>/<action id>`로, 인스턴스를 받지 않는 액션은 `<action id>`로 덮어쓴다. 내장 `server.start`·`server.stop`·`server.restart`·`server.status`는 명령 경로·GUI 권한 편집기와 같은 `<instanceId>/start|stop|restart|status`를 먼저 보고, 없으면 `<instanceId>/<action id>`를 본다. `PATCH /api/permissions/:guildId`의 `commandLevels`는 `/`가 없는 키를 디스코드·챗봇에서 부를 수 있고 레벨이 코드에 고정되지 않았으며 인스턴스를 받지 않는 액션 id일 때만 받는다(아니면 `invalid_command`, `actions::overridable`). 지우기(`null`)는 예전에 저장된 키도 받는다. 관리 액션(`kind=admin`)의 프리셋 값은 느슨 2, 기본 2, 엄격 3이다(`actions::admin_level`). 디스코드의 `update.apply`는 덮어쓰기와 상관없이 소유자이고, `POST /api/updates/install`(받기·적용·업데이터 띄우기를 한 번에)을 부른다.
  - 판정: 릴레이에서 온 요청(`ChatRequest.remote`)만 판정한다. GUI·CLI 요청은 하지 않는다. 금지는 help·list도 못 쓴다. 챗봇 경로는 `min(작성자 레벨, chatbotCap)`이다(사다리 순서로 낮은 쪽). 거부는 `PERMISSION_DENIED {need, level, via?}`.
  - 설정이 없는 길드도 같은 기본값으로 판정한다(제한 없음이 아니다).
  - 이 키들은 권한 API로만 바뀐다. 봇 설정 저장(`PUT /api/config/bot`)은 이 키들을 이전 값으로 되돌리고, 바꾸려던 키를 응답 `ignored`로 알린다(`config_store::keep_permission_keys`).
- `nodeSettings[guildId].discord`: 디스코드 길드 설정(I2, `ipc/handlers/discord.rs`). 원본은 이것 하나고 릴레이 값은 사본이다.
  - `{locale: "auto"|언어 코드, sleeping, music: {enabled, ui, queueLines 1-20, refreshInterval 2000-10000, normalize}}`. 기본값은 `auto`, false, `{true, true, 5, 4000, true}`(릴레이 기본값과 같다).
  - `/api/discord/:guild/settings`로만 바뀐다. 봇 설정 저장은 이 블록도 되돌린다(`PERMISSION_KEYS`).
  - 데몬에 값이 없는 길드는 릴레이 값을 읽는다(릴레이에 없으면 기본값). 페어링했는데 릴레이를 읽지 못하면 GET은 기본값에 `loaded:false`를 붙여 답하고, PATCH는 503 `relay_unavailable`로 거절한다(기본값을 저장해 릴레이 값을 덮지 않게).
  - 릴레이에 닿지 않거나 5xx로 보내지 못한 설정은 저장해 두고, 다음에 하트비트가 성공하면 `sync_on_connect`로 다시 보낸다(`discord::take_pending_sync`).
- `nodeSettings[guildId].allowedInstances`: 그 길드가 보고 부를 수 있는 인스턴스 id 목록(없으면 제한 없음). `PUT /api/guilds/:guild/allowed-instances`로만 바뀐다. 봇 설정 일괄 저장·초기화는 이 값을 바꾸지 않는다(`PERMISSION_KEYS`).
- `serverWords[instanceId]`: 그 서버를 부르는 낱말 `{words:[..], commands:{<명령 정식 이름>:[..]}}`(`chat/words.rs`). 인스턴스에 딸려 어느 길드에서나 같다. `/api/instance/:id/words`로만 바뀐다(봇 설정 일괄 저장은 이 키를 되돌린다, `keep_permission_keys`). 낱말은 공백 없는 1~32자, 항목마다 10개까지이고, 다른 서버 이름·id·낱말, 모듈 이름·게임명·module.toml 별명, `help`·`도움`·`도움말`·`list`·`목록`과 겹치면 안 된다. 명령 낱말은 같은 서버의 다른 명령 이름·별명과 겹치면 안 되고, 명령은 내장(start·stop·status)이거나 모듈 정의에 있어야 한다. 채팅 명령은 요청한 길드와 상관없이 이 낱말로 서버와 명령을 찾는다(길드가 쓸 서버 `allowedInstances`는 그대로 거른다).
  - 채팅 처리기(릴레이에서 온 길드 요청만): 대상은 인스턴스 이름·id → 그 서버 `words` → 모듈 이름·게임명·module.toml 별명 순(게임명 `ModuleView.game_names`는 module.toml `display_name`과 모듈 locales마다의 `display_name`에서 띄어 쓰지 않은 것, `chat::api::game_names`. 그 길드가 쓰는 서버 중 그 모듈 서버가 하나일 때만 그 서버를 부른다, 사용자 요청 2026-10-25), 명령은 그 길드·서버의 `commands` → module.toml 별명 → 명령 이름 → 내장 순이다. 여러 서버가 걸리면 `errors.multiple_instances`로 답한다. 위험(`danger`) 명령은 그 서버만 가리키는 말(인스턴스 이름·id·그 서버 `words`)로 불러야 하고, 모듈 이름·게임명·모듈 별명으로 찾았으면 실행하지 않고 `errors.danger_needs_name`(서버 이름을 알려 준다)으로 답한다(권한 판정 뒤, `chat::Ctx::module_command`). 로컬 요청(GUI·CLI)은 사용자 낱말을 쓰지 않는다. 권한 키는 언제나 정식 이름(`<instance>/<command>`)이다.

**데몬이 실제로 쓰는 설정값은 일부뿐이다.**

- `portConflictCheck`: 기동할 때 `skip_port_check` 원자 변수를 초기화하고(`ipc/mod.rs:822-828`), PUT·reset이 성공하면 갱신한다(`config.rs:28, 88-91`). start 경로와 모니터가 이 값 하나만 본다(`supervisor/mod.rs:269, 2555`).
- `language`: Python 실행의 `SABA_LANG`, 채팅 응답, 예약 공지, 익스텐션 설명 언어에 쓴다(`plugin/mod.rs:28-36`, `chat/api.rs:129`, `backup.rs:402`, `extension.rs:74`).
- `ipcPort`: `SABA_IPC_PORT`가 없을 때 데몬 포트로 쓴다(`main.rs:285-296`).
- `autoGeneratePasswords`: 시작할 때 빈 REST 비밀번호를 채우는가(§ 인스턴스 비밀번호).

예전 `src/config/mod.rs`의 `GlobalConfig`는 지워졌다.

---

## 5. 프로토콜 계층 (`src/protocol/**`)과 명령 라우팅

### 5.1 RCON은 없다

- 데몬은 RCON을 쓰지 않는다. 예전 `protocol/rcon.rs`와 `rcon_pool.rs`는 지워졌다. 게임 명령은 모두 stdin으로 간다(§5.4, 02 RCON 정책).
- 인스턴스 모델에서도 `rcon_port`, `rcon_password` 필드를 지웠다. 예전 파일에 남은 RCON 키는 로드할 때 지우고(§4.1), PATCH로 와도 버린다(`instance.rs:979-1000, 1295-1297`). 모듈 config에도 RCON 키를 넘기지 않는다(`supervisor/mod.rs:1253-1262`).

### 5.2 REST (`protocol/rest.rs`)

- `reqwest::blocking`(동기)을 쓰고 timeout은 10초다. 시스템 프록시를 거치지 않고(`no_proxy`), TLS는 native-tls다. Basic Auth를 지원한다.
- 응답 없이 연결이 끊기면(EOF·reset·abort. 오류 종류로 보므로 Windows의 os error 10053·10054도 같다) 결과를 알 수 없다고 본다: `success:false`, `data.outcome:"unknown"`, `error`(`rest.rs` `closed_without_response`). 종료 명령처럼 서버가 끊을 수 있는 요청도 성공으로 보고하지 않는다.
- `verify_connection`은 실제 연결 없이 host와 port 유효성만 본다(`rest.rs:42-48`).
- 인증 로그에는 사용자 이름만 debug 레벨로 남긴다(`rest.rs:91`). 토큰·비밀번호 길이는 남기지 않는다.

### 5.3 ProtocolClient (`protocol/client.rs`)

- REST 한 가지만 남았다(`CommandType::Rest`). `/rest` 핸들러가 `new_rest`로 만든다(`command.rs:572`).

### 5.4 명령 라우팅

| 경로 | 동작 |
|---|---|
| `POST /api/instance/:id/command` | 모든 인터페이스의 단일 명령 경로다. 요청 해석·검증은 `crate::commands`가 한다. 모듈에 없는 이름이면 활성 익스텐션 명령을 먼저 본다(`command.rs:80-124`). 그다음 모듈 정의대로 `/rcon`(stdin), `/rest`, `lifecycle.py command`, 원시 stdin 중 하나로 보낸다(`command.rs:158-183`). 꺼져 있고 콘솔도 없으면 409 `server_not_running`이다. `Supervisor::execute_command`(`supervisor/mod.rs:1220-1266`)는 config `{command,args,protocol_mode,working_dir,rest_host,rest_port,pid,instance_id}`를 락 안에서 만들고, 락을 놓은 뒤 Python을 실행한다 |
| `POST /api/instance/:id/rcon` | 콘솔이 있으면 **stdin으로 보낸다.** 콘솔이 없고 모듈이 RCON만 받으면(`Supervisor::rcon_route`) 127.0.0.1 RCON으로 보낸다(02 RCON 정책). `routes_commands_to_stdin`(`supervisor/mod.rs:1704-1712`)이 콘솔 있는 서버(관리 모드, 재접속 후에도 saba-runner가 stdin 유지)를 확인하면 `via_stdin`(`command.rs:430-484`)이 한 줄을 쓰고 콘솔 출력을 최대 약 1.5초 모아 `data.response`로 돌려준다. 둘 다 없으면 409 `no_command_path`(`command.rs:392-`). 서버가 콘솔 입력을 읽지 않아 대기열(256줄)이 차 있으면 기다리지 않고 503 `console_input_full` |
| `POST /api/instance/:id/rest` | 접속 정보는 `get_credentials`(게임 설정 파일)에서 읽고 인스턴스별로 캐시한다(`CredentialsCache`, `ipc/mod.rs:302-328`; `command.rs:661-689`). 실패하면 캐시를 지운다. host와 port는 payload, instance, 모듈 기본값 순으로 정하고, 동기 `reqwest::blocking` 호출은 `spawn_blocking`으로 감싼다(`command.rs:614-621`) |
| `POST /api/instance/:id/stdin` | `/rcon`과 같은 `via_stdin` 경로다(`managed.rs:118-137`) |

---

## 6. 내장 런타임: python_env

### 6.1 Python (`src/python_env/mod.rs`)

- 버전: 포터블 CPython **3.12.8**(python-build-standalone 태그 `20250106`, `install_only_stripped`)이다(`updater/src/python_pin.rs:7-8`). 시스템 Python을 쓸 경우 3.10 이상이어야 한다(`python_env/mod.rs:45`).
- 탐색 순서: 이미 받은 포터블 Python, 시스템 Python, 자동 다운로드 순이다(`find_base_python`, `:209`). 찾은 인터프리터로 `<pyroot>/python-env` venv를 만들고 pip를 업그레이드한다(`ensure_venv`, `:114`).
- 시스템 Python은 `sys.executable`로 확인한 실제 인터프리터 파일 경로를 사용한다.
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
- 진행률: stderr의 `PROGRESS:{json}` 줄을 `ExtensionProgress{percent,message,step,total,label}`로 파싱해 콜백에 넘긴다. `LOOPBACK:`·`HOST:` 줄은 중계로 처리하고 로그에 넣지 않는다. 나머지 줄은 debug 로그로 남기며 아래 상한 안에서 보존한다.
- 타임아웃: 기본 120초(`DEFAULT_PLUGIN_TIMEOUT_SECS`), `install_server`는 1800초(`INSTALL_TIMEOUT_SECS`)다. 기한은 아래의 자리 대기·선조회·환경 준비·실행 전체를 포함한다.
- `PluginCall`(`:161-177`)은 데몬 락 안에서 호출을 만들고 락을 놓은 뒤 실행하기 위한 묶음이다.

**실행 자원 상한 (0.1.10)**

모듈·익스텐션 Python 호출(`plugin::run_plugin*`, 훅, `functions` 조회와 미리 읽기)에 적용한다. 신뢰 표시가 `verified`여도 같은 상한을 쓴다. 값은 설정 파일이 아닌 상수다.

| 대상 | 상한과 동작 |
|---|---|
| stdout | 4 MiB까지 모은다. 넘으면 프로세스 트리를 끝내고 `module_output_too_large`로 실패한다 |
| 일반 stderr | 계속 읽어 비운다. 마지막 200줄, 합계 64 KiB만 보존한다. 앞의 줄은 debug 로그로만 남긴다 |
| 설치 자리 | 함수 이름이 `install_server`인 호출만 쓴다. 동시에 2개다 |
| 일반 자리 | 그 밖의 모든 호출이 쓴다. 동시에 8개다. 설치 자리와 별개다 |

- 자리를 기다리는 시간도 호출 기한에 든다. 기한 안에 자리를 얻지 못하면 `module_timeout`, `phase:"queue"`다.
- 한 호출 안의 내부 선조회(`functions` 등)는 바깥 호출이 잡은 자리와 같은 절대 기한을 쓴다. 자리를 두 번 잡지 않는다.
- 기한은 자리 대기, 함수 목록 조회, `python_env::get_python_path`, 실행을 모두 포함한다. 환경 준비 중 기한을 넘으면 준비 상태와 그 자식 프로세스도 정리한다.
- 격리 실행 준비(권한 부여, 서버 시작 잠금 `LaunchGate` 대기)는 기한에 넣지 않는다(0.1.9와 같다). 서버 폴더 표시 걷기가 큰 폴더에서 수십 초 걸릴 수 있어서다. 격리 실행 준비 중에도 자리는 유지한다. 내부 조회·재진입·같은 함수 목록을 기다리는 호출에도 이 제외 시간을 적용한다.
- 기한을 넘거나 출력 상한에 걸리면 프로세스 트리를 끝내고 읽기 작업을 멈춘다. Unix는 프로세스 그룹을, Windows는 Job Object와 `taskkill /T`를 쓴다. 격리 실행은 AppContainer job을 쓴다. 부모가 먼저 끝난 손자도 정리 대상이다.
- Windows의 일반 실행과 환경 준비는 프로세스를 일시 정지 상태로 만들고 Job Object에 넣은 뒤 실행한다. Job 등록이나 실행 재개에 실패하면 프로세스를 끝내고 호출을 실패로 돌려준다. 실행기가 Job 등록 전에 자식을 띄우는 경합을 막는다.
- 모듈 호출 중 데몬이 다른 모듈을 다시 부르면 바깥 호출의 자리를 빌려 쓴다. 자리 없이 캐시만 쓰는 경로는 캐시가 없으면 실패한다. 자리가 모두 차도 재진입으로 교착하지 않아야 한다.
- 같은 작업의 재진입은 절대 기한도 함께 쓴다. 별도 HTTP 작업의 REST 자격증명 조회는 캐시를 먼저 쓴다. 캐시가 없으면 일반 자리를 즉시 시도하며, 자리가 없으면 `module_execution_failed`, `phase:"queue"`로 실패한다. 이 경로는 자리를 기다리지 않는다.
- 1분 접속자 수 갱신(`supervisor/players.rs`)은 자리를 기다리지 않는다. 바로 얻지 못한 인스턴스는 이번 분을 건너뛴다. 실패로 세지 않고 마지막 값도 지우지 않는다. 매 분 순회 시작점을 돌린다.

긴 원문을 돌려주는 진단이 4 MiB를 넘으면 `module_output_too_large`로 실패한다. 기존 진단이 실제로 이 상한을 넘는지는 아직 측정하지 않았다.

**모듈 호출 오류 (0.1.10)**

공통 자료형 `ModuleCallError`와 정규화 함수 하나로 모듈·익스텐션 호출 실패를 전달한다. 실행기 → supervisor → 훅 → HTTP 핸들러 → 비동기 설치 진행 상태에서도 필드를 문자열로 바꾸지 않는다. 각 라우트가 쓰던 HTTP 상태는 유지한다.

```json
{
  "success": false,
  "error_code": "module_timeout",
  "error": "minecraft.install_server did not finish in 1800 s",
  "module": "minecraft",
  "function": "install_server",
  "phase": "run",
  "stderr_tail": ["...마지막 줄들..."]
}
```

| `error_code` | 조건 |
|---|---|
| `module_timeout` | 호출 기한 초과. `phase`는 `queue`(자리 대기), `prepare`(환경 준비), `run`(실행) |
| `module_execution_failed` | 환경 준비·프로세스 생성·입출력 실패. `phase`로 실패 단계를 구분한다 |
| `module_crashed` | 종료 코드가 0이 아님 |
| `module_bad_output` | stdout에서 결과 JSON을 읽지 못함. SDK 결과 직렬화 실패도 포함한다 |
| `module_output_too_large` | stdout 4 MiB 초과 |
| `module_path_outside` | SDK `safe_join` 또는 데몬의 반환 경로 검사가 허용 루트 밖 경로를 거부함 |
| `module_exception` | SDK `run_lifecycle`이 함수 안의 예외를 잡음 |
| `isolation_unsupported` | 격리할 수 없는 플랫폼에서 확인되지 않았고 현재 파일 지문에 대한 비격리 동의도 없는 구성요소 |

- `stderr_tail`은 보존한 일반 stderr의 마지막 20줄이다. 시간 초과·강제 종료 때도 남긴다. 결과가 `module_exception`이면 traceback의 끝을 붙인다.
- 모듈이 스스로 돌려준 `success:false`, `error_code`, `message`는 유지한다. 데몬은 `module`·`function`만 더한다. `module_exception`에는 위의 `stderr_tail`도 붙인다.
- GUI·CLI의 로컬 응답은 오류 필드를 유지한다. 안내 문구 키는 locales `module_errors.<코드>`다. 기존 오류 표시 경로가 `module`·`function`·`stderr_tail`을 전달한다.
- 원격(챗봇·디스코드·릴레이) 응답에서는 `stderr_tail`을 뺀다. `error`·`message`·`path` 같은 문자열의 IP·비밀값을 가리고, 절대 경로는 `<path>`로 바꾼다. [06 §3](06-contracts-0.1.9.md)과 같은 경로 가리기 함수를 쓴다.

**반환 경로 경계 (0.1.10)**

`module_paths::Boundary`는 신뢰 표시와 관계없이 반환 경로를 검사한다. 상대 경로는 서버 폴더(`working_dir`, 없으면 `instance_dir/<install_subdir>`) 기준이다. 허용 루트는 서버 폴더, 데몬 항목을 뺀 인스턴스 폴더, 데몬이 모은 `instance_data_dirs`, [02 §2.1](02-modules.md#21-실행-방식-dsrcpluginmodrs266-463)의 선언 데이터 루트다. `instance::DAEMON_ENTRIES`와 그 하위 경로는 다른 허용 루트와 겹쳐도 거부한다.

- `backup_paths`의 `paths`·`restore_roots` 중 하나라도 벗어나면 백업·복원 전체가 `module_path_outside`로 실패한다. 일부만 담은 백업을 성공으로 남기지 않는다.
- 마이그레이션 `import_settings`의 `executable_path`, `install_server`의 `install_path`·`executable_path`도 같은 검사를 받는다. 빈 실행 경로는 값이 없는 것으로 본다.
- 마이그레이션 경로 위반은 422와 이미 생성한 인스턴스의 `id`를 돌려준다. 잘못된 실행 경로·설정은 저장하지 않으며 인스턴스를 자동으로 삭제하지 않는다.
- 읽기·쓰기 직전에 기존 경로 구성요소를 검사한다. 링크·정션 같은 중간 재분석 지점은 거부한다. Windows는 `sandbox::appcontainer`의 핸들 검사와 최종 경로 비교를 함께 쓴다. 아직 없는 대상은 가장 가까운 기존 조상을 검사한다.
- 복원 `backup::is_inside`도 이 실제 경로 함수를 쓴다. 별도 `guard_launch`와 `restore_outside` 정책은 유지한다.
- 익스텐션 `i18n_dir`은 [03 §1](03-extensions.md)의 폴더 내부 검사에 실패하면 locales를 비우고 경고한다.

**HOST 통로 (0.1.10)**

모든 Python 호출에 실행별 임시 폴더 `SABA_HOST_DIR`을 준다. 모듈은 stderr에 `HOST:{id, op, args}` 한 줄을 쓴다. 데몬은 `<SABA_HOST_DIR>/<id>.json`에 `{ok:true, data}` 또는 `{ok:false, error_code, error}`를 원자적으로 쓴다. SDK가 결과 파일을 읽고 지우며 실행이 끝나면 데몬이 폴더를 정리한다.

| 작업 | 인자 | 동작 |
|---|---|---|
| `instance.stdin` | `{command}` | 기존 인스턴스 stdin API와 같은 처리와 결과 |
| `instance.rest` | `{method, path, body?}` | `path`를 REST API의 `endpoint`로 옮기고 기존 처리와 결과를 사용 |

대상 인스턴스는 데몬이 만든 호출 문맥으로 정한다. 격리된 구성요소(확인되지 않았고 비격리 동의도 없는 것)의 실행은 그 인스턴스 모듈 자신일 때(실행한 진입 파일과 그 모듈의 진입 파일의 실제 경로가 같을 때. zip 모듈은 `.extracted` 아래 파일) `instance.rest`만 쓸 수 있다. 콘솔(`instance.stdin`), 익스텐션 hook, 다른 모듈의 요청은 `op_not_allowed`다(`host.rs` `Proxy::sandboxed`, `own_module`). 그래서 REST로만 저장·정지하는 게임(팰월드)도 격리 실행에서 정상 정지한다. `instance.rest`는 대상이 127.0.0.1(`localhost`·`::1`)이고, 데몬 API 포트가 아니고(`port_not_allowed`), 그 포트를 듣는 프로세스가 그 인스턴스 서버이거나 그 자식일 때만(`port_not_served`) 잇는다. 요청에서 인스턴스 id·PID·파일 경로를 받지 않는다. 인스턴스 문맥이 없으면 `no_instance`, 허용하지 않은 작업은 `op_not_allowed`다. 실행당 요청은 64건, 요청 시간은 30초, 보내기는 64 KiB, 받기는 1 MiB다. `LOOPBACK`과 요청 수를 함께 센다. 응답 상한을 넘으면 `response_too_large`로 실패하며 잘린 성공을 돌려주지 않는다. `HOST:` 줄과 그 연속 조각은 로그·stderr 진단에 남기지 않는다.

REST 자격증명 조회의 `get_credentials` 재진입은 바깥 호출의 자리와 절대 기한을 쓴다. 인스턴스 hook도 같은 문맥을 전달한다. 인스턴스 없는 목록·데몬 시작/종료 hook은 `no_instance`다. SDK의 HTTP 호환 경로와 일반 토큰 경로는 남아 있다.

**자식 프로세스 환경**

- `PYTHONPATH` = `<data_dir>/python-guard` + `<extensions_dir의 부모>` + `<data_dir>/python-shared` + 기존 값. `sitecustomize.py`는 표준 라이브러리 경로를 이 경로보다 먼저 두고 활성 익스텐션만 import되게 막는다.
- `SABA_ENABLED_EXTENSIONS` = `config.json`의 `extensions.enabled`를 쉼표로 연결한 값. 목록이 없으면 빈 문자열이다(`:317-322`).
- 데몬 연결 정보 `SABA_IPC_PORT`, `SABA_IPC_URL`, `SABA_TOKEN_PATH`, `SABA_LANG`(`daemon_env`, `:14-38`).
- 확인되지 않은 구성요소는 격리를 요구한다. Windows는 AppContainer를 쓰며, Linux·macOS는 현재 파일 지문에 대한 비격리 동의가 없으면 실행을 거부한다(`sandbox::isolation_for`, 02 문서 §2.1). 격리를 요구한 실행은 `SABA_TOKEN_PATH`를 빼고 임시·캐시 폴더를 구성요소별로 나눈다.
- `SABA_HOST_DIR`은 모든 호출에 있고, 실행 종료 후 정리한다. 대상 인스턴스는 이 환경 변수가 아니라 `PluginCall`의 데몬 측 문맥으로 정한다.
- 내장 공용 Python 유틸리티는 `<data_dir>/python-shared/`에 맞춰 쓴다. 모듈 폴더는 `PYTHONPATH`에 넣지 않는다. 익스텐션은 import 가드와 익스텐션 상위 폴더를 사용한다.

**모듈의 `lifecycle.py`가 응답해야 하는 함수** (코어에서 직접 호출하는 것만)

- `list_versions` → `{latest:{release}}` 또는 `{latest_release}`
- `install_server` → `{success, executable_path?, install_path?, message?}`
- `import_settings` → `{success, settings{}, executable_path?, detected?}`
- `config_files` → `{success, files:[절대 경로]}` (선택. 사바쨩이 고칠 수 있는 게임 설정 파일. 가져올 때 보관하고 연결 해제 때 되돌린다)
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
  - `text|password|file`: 문자열(숫자, 불리언도 허용). 줄바꿈(`\r`·`\n`)과 NUL이 든 문자열은 `InvalidFormat`이다(값은 설정 파일의 한 줄·실행 인자 하나가 된다)
  - 알 수 없는 타입은 통과시킨다.
  - `required==Some(true)`인데 값이 `null`이거나 빈 문자열이면 에러다.
- `PATCH /api/instance/:id`는 `validate_settings_patch`(`:281-297`)로 **요청에 들어 있는 필드만** 검사한다(`instance.rs:1027`). 보내지 않은 필수 필드는 오류가 아니다.
- `check_port_conflicts`(`:312`)는 주어진 인스턴스 집합과 포트가 겹치는지 검사한다. 모듈이 지원하는 프로토콜의 포트만 본다. supervisor의 start 경로와 모니터는 **실행 중인** 인스턴스만 넘긴다.
- 시작 전 검사(`Supervisor::port_conflict_response`)는 실행 중인 인스턴스와 겹치는 포트가 없으면 `start_check_ports`(같은 포트에 모듈 설정 중 `port = true`인 값을 더한 것)를 `ports_held_elsewhere`로 TCP·UDP 둘 다 본다. Windows는 포트를 묶지 않고 OS의 사용 중 포트 표(`GetExtendedTcpTable`의 듣는 포트, `GetExtendedUdpTable`의 묶인 포트)를 읽는다. IPv4는 주소와 상관없이, IPv6는 `[::]`에 묶인 것(dual-stack 서버)만 센다. 0.0.0.0에 포트를 열면 방화벽 허용 알림이 뜨기 때문이다. 시스템 예약 포트 범위(Hyper-V·WinNAT 제외 범위)는 이 표에 나오지 않으므로 `ports_reserved_by_windows`가 루프백에 TCP·UDP로 묶어 보고 권한 거부면 `"Port N (이름) is reserved by Windows (excluded port range); choose another port"`를 더한다. 범위는 프로토콜마다 따로 잡히므로 `rest_port`·`rcon_port`는 TCP 거부만으로, 그 밖의 포트는 TCP·UDP가 둘 다 거부될 때만 예약으로 본다. 새 인스턴스는 포트를 옮길 때(`shift_ports_clear_of`) 어느 한쪽이라도 거부되는 포트를 피하고, 사용자가 정하지 않은 REST 기본 포트가 예약 범위면 `PORT_SHIFT_STEP` 단위로 올라가며 예약 포트와 다른 인스턴스가 저장한 포트를 피해 옮긴다(`next_free_port`, 같은 이름의 모듈 설정 `rest_port`도 맞춘다). 이미 저장된 인스턴스도 시작할 때(`start_server`·`start_managed_server`, 떠 있지 않을 때) REST 포트가 모듈 기본값과 같고 TCP 예약 범위에 들면 같은 방법으로 옮겨 저장한다(`move_reserved_default_rest_port`, 0.1.6에서 만든 팰월드 8212). 기본값이 아닌 REST 포트는 사용자가 정한 것으로 보고 옮기지 않는다. 다른 OS는 0.0.0.0에 묶어 본다. 다른 프로그램이 잡은 포트도 같은 `port_conflict` 응답(HTTP 200, `success:false`, `conflicts:["Port N (이름) is in use by another program"]`)으로 돌려준다. 이 인스턴스가 이미 실행 중이면 보지 않는다.
- 시작 직전(`start_server`·`start_managed_server`) 모듈 기본 REST 포트를 그대로 저장한 인스턴스는, 그 포트가 Windows 예약 범위에 들면 예약 범위와 다른 인스턴스의 포트를 피해 옮겨 저장한다(`move_reserved_default_rest_port`). 옮겼으면 200 시작 응답(성공이든 `success:false`든)에 `port_moved: [{field:"rest_port", from, to}]`를 싣는다(HTTP 오류 응답에는 없다). 자동·예약 재시작, 업데이트·백업 뒤 시작처럼 응답을 사람이 보지 않는 경로도 있으므로, 옮길 때마다 이벤트 `instance.port_moved`(`instanceId, instance, field, from, to`)를 낸다.
- `GET /api/servers`의 `port_conflicts`도 같은 함수를 쓰되(`server.rs:251-275`), "켜면 부딪힐" 서버를 미리 보여 주려고 정지된 인스턴스까지 모두 넘긴다.

### 7.3 기타

- `process_monitor.rs`는 `sysinfo` 0.38로 전체 프로세스 스냅샷(`pid,name,exe,cmd`)을 만든다. `spawn_blocking`으로 감싼 async 버전도 있다.
- `daemon_log.rs`는 tracing Layer와 링 버퍼(10k)를 제공하고, `get_since(since,count=200)`로 조회한다.
- `audit.rs`(A10): 서버 시작·정지·재시작(`/api/instance/:id/*`, `/api/server/:name/*`, `/managed/start`), 게임 명령(`execute_command`, `/rcon`·`/rest` 라우트), 콘솔 입력(`/stdin`), 속성 쓰기·서버 초기화(`/properties` PUT, `/server/reset`), 예약 백업(`backup`, 서버가 꺼져 건너뛴 것 포함), 액션 카탈로그의 관리 액션(`actions::run`, `action`=액션 id, 입력은 가려서 `args`)을 `<data_dir>/audit.log`에 한 줄 JSON으로 남긴다(1 MiB에서 `audit.log.1`로 넘김, 인자·줄은 300자까지). 출처는 task-local `Origin`: HTTP 요청은 미들웨어가 `X-Saba-Client`(GUI `gui`, CLI `cli`, 없으면 `api`)로, 릴레이 에이전트는 `discord`+요청자·길드 id(릴레이가 `payload.userName`·`payload.guildName`을 보내면 그 이름도)로, 예약 작업은 `schedule`로, 크래시 뒤 자동 재시작(`ipc/handlers/server.rs` `auto_restart`)은 `restart`로 정한다. 데몬 안에서만 붙이는 출처(`INTERNAL_SOURCES`: `discord`·`schedule`·`daemon`·`web`·`remote`·`restart`·`game`·`external`)는 헤더로 받지 않고 `api`로 본다. 정지를 시킨 출처는 정지 표시(`Supervisor::stop_requested`)와 함께 두었다가 `instance.stopped`에 싣는다. 기록 전에 `audit::redact`가 비밀을 가린다: 모듈이 `password`로 선언했거나 이름에 pass·pwd·token·secret·apikey가 든 명령 인자, 그런 인자를 받는 명령의 콘솔 줄(첫 낱말이 이름·별명·`console_template` 첫 낱말과 같거나 이름부터 비밀스러운 줄)은 첫 낱말 뒤 전체, 인스턴스의 `password` 설정값·REST 비밀번호(4자 이상)는 어디에 나오든. 넘기기(rename)에 실패해도 기록은 이어서 남긴다. 재시작 안의 정지·시작은 `quietly`로 따로 남기지 않는다. 로컬 클라이언트는 같은 토큰을 쓰므로 출처는 기록용이지 보안 경계가 아니다.
- `events.rs`(P4): 데몬 이벤트 버스. `emit(kind, data)`가 번호를 붙여 최근 200건 버퍼와 broadcast 채널(256)에 같은 잠금 안에서 넣는다(구독 시작 때 빠지거나 겹치는 건이 없게). `emit_live`는 번호·버퍼 없이 지금 붙은 구독자에게만 보낸다(진행률). 내는 곳: `AuditLog::record`(`action.run`), 감독자 종료 판정(`instance.exit`, 정상 정지는 `instance.stopped`도. 모니터가 추적하던 non-managed PID가 정지 요청 없이 사라지면 `unknown_exit`: 콘솔이 없어 서버 창에서 끈 것과 강제 종료를 가를 수 없으므로 이유 모름으로 둔다), 감독자 시작(`instance.started`, managed·non-managed·익스텐션이 맡은 시작이 성공한 때와 모니터가 사바쨩 밖의 프로세스를 붙였을 때), 익스텐션·non-managed 정지 성공(`instance.stopped`), 시작 전 포트 옮기기 `move_default_rest_port_if`(`instance.port_moved`), `ProvisionTracker::update`(`provision.progress`, live), 백업 핸들러 `take_backup_locked`(`backup.done`, 수동·예약·업데이트 전·복원 전 모두), 업데이트 확인 `check_and_follow_up`(`update.available`), 예약 예고 `count_down`(`schedule.warn`), 시작·종료 판정 `supervisor::ready`(`instance.ready`), managed 콘솔 `supervisor::player_events`(`instance.player`). 파일에는 남기지 않는다.
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
  - 업데이터 실행은 클라이언트가 `POST /api/updates/launch-updater`로 데몬에 맡긴다(`updates.rs:598-675`). 데몬이 이번 적용에만 쓰는 실행 표(`launch_ticket::issue`)를 만들어 `--apply --wait-pid <pid> --launch-token <표>`로 업데이터를 detach로 띄우고, 0.5초 뒤 등록된 GUI를 끄고(스스로 끝날 틈 3초) `stop_requested`로 스스로 종료한다. GUI가 떠 있었으면 `--relaunch`는 그 GUI다(GUI가 뜨면서 데몬을 띄운다). 헤드리스 설치(`install-profile.json`)면 `--headless`를 붙여 창 없이 적용하게 하고, 다시 띄울 것이 없으면 데몬 자신을 `--spawned`로 다시 띄우게 한다.
- `/api/updates/*`도 다른 라우트처럼 토큰 인증을 거친다(`ipc/mod.rs:812-816`). `relay_url`을 바꿔도 manifest 서명 검증은 그대로라 다른 서버가 파일을 바꿔 끼울 수는 없다.

---

## 9. 불일치, 데드 코드, 반쯤 구현된 기능, 위험 지점

### 9.A 오너 원칙 위반 (인터페이스가 데몬을 우회하는 곳)

1. ~~**GUI가 `settings.json`을 직접 읽고 쓴다.**~~ → 해결: GUI는 `config.json`의 `gui`를 읽기만 직접 하고(Tauri 셸 `saba-chan-gui/src-tauri/src/settings.rs:71-84`), 저장은 데몬 API(`PUT /api/config/gui`)로 하며 데몬이 응답하지 않을 때만 파일에 직접 쓴다(`saba-chan-gui/src/api/createApi.js:57-74`, `src-tauri/src/native.rs:189-212`, `write_gui_section` `settings.rs:49-51`).
2. ~~**GUI가 relay 서버를 직접 호출한다.**~~ → 해결: GUI(Tauri 셸 `saba-chan-gui/src-tauri/src`, 브리지 `saba-chan-gui/src/api`)에 릴레이 직접 호출이 없다. 봇 설정·초대·페어링은 데몬의 `/api/relay/*`, `/api/config/bot`을 거친다.
3. ~~**봇 실행 로직이 GUI에 중복돼 있다.**~~ → 해결: 로컬 봇과 범용 `/api/ext-process/*`가 지워졌다(L1). 디스코드 연동은 데몬의 `relay_agent.rs` 하나뿐이다.
4. **GUI가 데몬의 경로 일부를 정한다.** `saba-chan-gui/src-tauri/src/lifecycle.rs`에서 `SABA_INSTANCES_PATH`, `SABA_IPC_PORT`를 GUI가 정해 주입한다. ~~`SABA_MODULES_PATH`~~ → 해결: 모듈 폴더는 데몬이 `config.json` `paths`로 정한다(§1.5).
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
- ~~`GuiSettings.autoGeneratePasswords`는 데몬에서 효과가 없다~~ → 해결: 시작할 때 빈 REST 비밀번호를 채울지 이 값으로 정한다. ~~`ipcPort`~~는 데몬 포트로 쓴다(§4.4).
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
  - `check-update`: 없는 인스턴스는 404, 설치 방식 익스텐션이 설치하지 않은 인스턴스는 `reason:"update_unsupported"`, `extension_data`가 비어 있으면 `update_available:false`.
- 인증은 `tests/api_contract.rs`가 실제 서버로 모든 라우트가 토큰 없이 401인지 확인한다. 단일 인스턴스는 `tests/single_instance.rs`, 릴레이 에이전트 왕복은 `tests/relay_agent_e2e.rs`가 본다.

## 10. 시스템 모니터 (`system_monitor.rs`)

- `GET /api/system/metrics`(`ipc/handlers/system.rs`)가 부를 때만 표본을 뜬다. 백그라운드 루프는 없고, 아무 클라이언트도 보지 않으면 비용이 없다. 0.9초 안의 재요청은 직전 표본을 준다(`MIN_SAMPLE_INTERVAL`). 직전 표본이 10초보다 오래됐으면 짧은 간격으로 두 번 떠서 비율을 만든다(`STALE_AFTER`).
- 인스턴스 몫 = `tracker.get_pid`의 PID와 그 자손 프로세스의 합(`attribute`). saba-runner 자체는 인스턴스 몫이 아니다. `cpu_percent`는 기계 전체 기준(코어 수로 나눔).
- CPU·메모리: 인스턴스 - 나머지(`others_*`) - 유휴(전체 - 사용). 전체 값은 인스턴스 합보다 작아지지 않게 맞춘다.
- 디스크 IO: 전체는 OS의 물리 디스크 카운터로 잰다(`system_monitor/physical_io.rs`; Windows는 PDH `\PhysicalDisk(_Total)\Disk Read/Write Bytes/sec`, Linux는 `/proc/diskstats`의 `/sys/block` 디스크, loop·dm·md 등 겹치는 장치 제외). `source:"physical"`. 프로세스 IO 카운터는 캐시에서 읽은 것까지 세서 실제 디스크와 수십 배 어긋나므로(0.1.4 PC 확인: 13.9MB/s 대 실제 0.35MB/s) 전체를 인스턴스별로 나누지 않는다(`per_instance:false`). 인스턴스의 `disk_read_bps`·`disk_write_bps`는 그 프로세스들의 IO 요청(캐시 포함)이다. 물리 카운터가 없으면 프로세스 IO 합을 쓰고 `source:"process"`. 유휴 몫은 클라이언트가 최근 최대값을 눈금으로 정한다.
- 네트워크: 인터페이스 합(루프백 제외)만 준다. 프로세스별 값은 싸게 얻을 길이 없다(Windows는 ETW·관리자 권한). `per_instance`는 항상 false.
- 디스크 용량: 데이터 폴더나 인스턴스 폴더(인스턴스 폴더 + `working_dir`)가 있는 디스크만, 데이터 폴더 디스크가 먼저. 인스턴스 몫은 폴더 크기를 별도 스레드에서 재고 10분 재사용한다(`DIR_SIZE_TTL`). `?sizes=false` 요청(GUI의 접힌 요약)은 새로 재지 않고 마지막 값을 쓴다. 아직 못 쟀으면 `sizes_at_ms`가 null.
- `?sizes=false` 요청은 물리 카운터가 있으면 프로세스 IO도 읽지 않는다: 그때 인스턴스 `disk_read_bps`·`disk_write_bps`는 0이다. 그 뒤의 전체 요청은 짧은 간격으로 새로 잰다. 물리 카운터 읽기가 한 번 실패하면 10초까지 직전 값을 쓴다. 3분 동안 요청이 없으면 표본기(sysinfo)를 버리고 다음 요청 때 다시 만든다(`IDLE_DROP_AFTER`).
