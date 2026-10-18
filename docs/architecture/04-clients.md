# 04. 클라이언트 계층: GUI · CLI · Discord Bot (server-chan) · 공유 상수 · 로케일

> 이 문서는 코드에서 직접 도출했다. 기존 docs/README는 신뢰하지 않았고, 코드와 어긋나는 주석이나 문서는 각 항목에서 따로 표시했다.
> 기준 원칙(프로젝트 오너): **daemon(saba-core)이 서비스의 주인이며 모든 기능은 daemon을 거친다. 인터페이스(GUI/CLI)는 혼자서 아무것도 하면 안 되고, 파일 편집도 daemon API를 거쳐야 한다.** modules는 Python 게임 서버 매뉴얼, plugins(=extensions)는 Python으로 작성되어 인터페이스에 컴포넌트를 주입하고, server-chan(discord_bot)은 daemon과 짝을 이뤄 Discord 봇을 돌린다.
> 이 원칙을 어기는 지점은 모두 **§7 "daemon 우회 지점 전수 목록"**에 정리했다. 이 문서에서 가장 중요한 결과물이다.

Daemon 라우터의 기준 위치: `src/ipc/mod.rs:634-802`(`router()`, 인증 미들웨어 `auth::auth_middleware`는 `:801`), `src/ipc/updates.rs:98-111`(`/api/updates/*`, 인증 layer **이전**에 `.merge()`로 붙어 같이 인증된다, `src/ipc/mod.rs:797-801`).

---

## 1. GUI (saba-chan-gui, Tauri)

> 2026-10-18 기준 Tauri 2(플러그인 dialog·opener·notification), Vite 6.4, Vitest 5. Electron 메인 프로세스(main.js·preload.js)는 0.1.5에서 Tauri 셸로 바뀌었다. 프런트 빌드에는 여전히 Node `>=22.12`가 필요하다(`saba-chan-gui/package.json` `engines`). 산출물은 프런트(`build/`)를 넣은 exe 하나 `saba-chan-gui.exe`(Unix `saba-chan-gui`)다(`src-tauri/Cargo.toml` `[features] default = ["custom-protocol"]`, `tauri.conf.json` `bundle.active false`). Windows는 WebView2로 화면을 띄운다. 개발 실행은 `npm start`(vite 5173 + `cargo run -p saba-chan-gui --no-default-features`).

구성은 둘이다.
- **Tauri 셸** `saba-chan-gui/src-tauri/src`(Rust): 창·트레이·데몬 기동·토큰·이벤트 스트림 같은 네이티브 일만 한다(`lib.rs:1-4`).
- **JS 브리지** `saba-chan-gui/src/api`: `window.api`·`window.electron`을 예전 preload와 같은 이름·인자·결과로 만든다(`install.js:11-34`, `createApi.js:1-3`). `window.__TAURI__`가 있고 `window.api`가 없을 때만 설치하므로 테스트 mock과 브라우저 개발 mock(`src/dev`)은 그대로다(`install.js:12`). `src/index.js`가 가장 먼저 import한다. 데몬 응답 가공(상태별 오류 문구, `errorBody`·`daemonCall`·`daemonJson`)은 브리지가 한다(`daemon.js:35-62`).

### 1.1 Tauri 셸이 맡는 일 (`saba-chan-gui/src-tauri/src`)

| 책임 | 위치 | 설명 |
|---|---|---|
| 데이터 폴더 | `lib.rs:79`, `updater/src/constants.rs:55` | `constants::resolve_data_dir()`. `SABA_DATA_DIR`, Windows `%APPDATA%/saba-chan`, Unix `$HOME/.config/saba-chan`. 설치 루트는 debug면 저장소 루트, 배포면 exe 폴더다(`install_root()` `lib.rs:67-76`) |
| IPC 토큰 인증 | `daemon.rs:120-271` | 토큰 파일(`constants::token_file_path()`, `lib.rs:95`)을 **직접 읽어** 데몬 요청에만 `X-Saba-Token`을 붙인다. 감사 로그 출처 `X-Saba-Client: gui`도 붙인다(`builder` `:182-192`, A10). 리다이렉트를 따라가지 않고 프록시도 쓰지 않는다(`:129-134`). 401이면 토큰 파일을 다시 읽고 1회 재시도한다. 단 `/api/relay/` 요청은 재시도하지 않는다(`send` `:213-229`) |
| 렌더러 → 데몬 통로 | `native.rs:17-42`, `daemon.rs:93-116` | 렌더러는 `daemon_request` 명령으로만 데몬에 닿는다. 경로는 `/api/…`와 `/health`만 받고 `.`·`..` 조각(`%2e` 포함)·`\`·공백·`#`는 막는다. 경로에 넣는 이름은 브리지가 `encodeURIComponent`로 넣는다. 셸이 직접 정하는 `/api/updates/launch-updater`·`/api/client/*`·`/api/daemon/shutdown`은 렌더러에 열지 않는다(확장 GUI 스크립트도 같은 웹뷰에서 돈다). 막힌 경로는 `{status:0, code:'EINVAL'}`. 결과는 성공·HTTP 오류 모두 `{status, data}`, 연결 실패는 `{status:0, error, code}`(`ECONNREFUSED`·`ETIMEDOUT`·`EOTHER`). `responseType`은 `json`·`text`·`base64` |
| 명령 권한 | `capabilities/default.json`, `build.rs` | 창 `main`과 `popout-*`에만 이 앱의 명령(`allow-*`)과 `core:default`를 연다. 명령 목록은 `build.rs`의 `COMMANDS`와 `lib.rs:118-149` `generate_handler!`가 짝을 이룬다 |
| 콘텐츠 보안 정책 | `tauri.conf.json` `app.security.csp` | 스크립트는 앱 자신과 `blob:`(익스텐션 번들)만, 인라인 스크립트·이벤트 속성은 막는다. 스타일은 인라인 허용(+Google Fonts), 이미지는 `data:`/`blob:`/https, 연결은 Tauri IPC만. 개발 실행(`devCsp`)은 vite HMR 때문에 느슨하다 |
| **Daemon spawn** | `start_daemon()` `lifecycle.rs:158-215` | debug는 `target/release`(없으면 `target/debug`)의 `saba-core(.exe)`, 배포는 설치 루트의 것을 `--spawned`로 띄운다(`daemon_path` `:129-138`). cwd는 설치 루트, env는 `RUST_LOG=info, SABA_LANG, SABA_IPC_PORT, SABA_INSTANCES_PATH=<data>/instances, SABA_MODULES_PATH`(`constants::resolve_modules_dir()`). stdout/stderr는 GUI 로그로 넘긴다(`pipe_log` `:244-262`) |
| 분리 실행 | `detach()` `lifecycle.rs:219-241`, `:188-197` | Windows는 `CREATE_NO_WINDOW`·`CREATE_NEW_PROCESS_GROUP`·`CREATE_BREAKAWAY_FROM_JOB`로 띄운다. GUI를 띄운 쪽의 job이 breakaway를 막아 spawn이 거부되면 breakaway 없이 다시 띄운다. Unix는 `process_group(0)` |
| Daemon 재사용/기동 판단 | `ensure_daemon()` `lifecycle.rs:389-436` | `GET /health`가 200이면 기존 daemon을 재사용한다. 응답의 `token`이 `valid`가 아니거나(A6) 예전 데몬이 401을 주면 토큰을 다시 읽으며 최대 3회 확인한다(`reload_token_until_valid` `:342-359`). ECONNREFUSED/ETIMEDOUT이면 `start_daemon()` 뒤 500ms 간격으로 최대 20회(10초) 폴링한다(`wait_for_new_daemon` `:362-372`) |
| Daemon 종료 | `stop_daemon()` `lifecycle.rs:308-335`, `force_kill()` `:289-304` | 먼저 `POST /api/daemon/shutdown`을 보내고 최대 15초 기다린다. 그래도 안 끝나면 **이 GUI가 띄운 PID만** 강제 종료한다(Windows `taskkill /PID … /F`, `/T` 없음. Unix `Child::kill`(SIGKILL)). 이름 기반 kill은 하지 않는다(`:306-307` 주석) |
| 클라이언트 등록/heartbeat | `lifecycle.rs:440-506` | `POST /api/client/register {kind:'gui', pid, installRoot}`(인스톨러가 이 설치의 GUI를 찾는 데 쓴다), 이후 30초마다 `POST /api/client/:id/heartbeat`. 404나 연결 거부면 다시 등록하고, 처음 등록이 실패해 id가 없으면 주기마다 등록을 다시 시도한다. 종료 시 `DELETE /api/client/:id/unregister[?shutdown=true]` |
| install_root 동기화 | `sync_install_root()` `lifecycle.rs:439-450` | `PUT /api/updates/config {install_root}` |
| 백그라운드 초기화 | `app_ready()` `lifecycle.rs:64-75`, `:77-116` | 렌더러가 리스너를 단 뒤 `app_ready`를 부르면 처음 한 번 `ensure_daemon` → 등록·heartbeat·이벤트·업데이트 확인을 시작하고 `status:update` `ready`를 보낸다. 데몬이 뜨지 않으면 `daemon_failed`만 보내고 멈춘다. `--after-update`면 첫 ready 뒤 `updates:completed` |
| 이벤트 스트림 | `events.rs:75-116`, `lifecycle.rs:510-521` | `GET /api/events`(SSE) 연결 하나를 유지하고 받은 이벤트를 `events:message {id, type, data}`로 보낸다. 끊기면 3초 뒤 `Last-Event-ID`로 다시 붙고, 401이면 토큰을 다시 읽는다 |
| 업데이트 알림 | `lifecycle.rs:523-565`, `events.rs:123-160` | 확인 주기는 데몬이 정한다. 셸은 10분마다 데몬이 캐시한 `GET /api/updates/status`를 읽어, 새 업데이트 목록일 때만 OS 알림을 띄우고 `updates:available`을 보낸다. Locales는 알림에서 뺀다. 적용은 사용자가 고른다 |
| **업데이트 적용 시작** | `updater_launch_apply` `native.rs:81-102` | `POST /api/updates/launch-updater {relaunch:[current_exe()]}`만 보내고 500ms 뒤 `clean_quit()`한다. 업데이터는 데몬이 띄운다(`src/ipc/updates.rs:601-659`, 실행 파일은 `saba-chan-updater{EXE_SUFFIX}`, `updater_binary` `:662-669`) |
| **Installer spawn(언인스톨)** | `launch_uninstaller` `native.rs:151-184` | 설치 루트·`saba-chan-gui` 하위·상위·exe 폴더에서 인스톨러를 찾고, 없으면 Windows 레지스트리 `HKCU\…\Uninstall\Saba-chan` `InstallLocation`을 본다(`winreg`, `:119-126`). `--uninstall`로 분리 실행(`DETACHED_PROCESS`) 뒤 `clean_quit()` |
| 모의 릴레이(개발용) | — | GUI는 띄우지 않는다. 개발자가 `node scripts/mock-release-server.js`를 직접 실행하거나 `scripts/build-core.ps1 -MockRelay`(모의 서명 키를 믿는 개발 빌드, 배포 금지)로 쓴다 |
| 디스코드 연결 | `createApi.js:793-816` | 셸에 디스코드 코드는 없다. 브리지가 데몬의 릴레이 에이전트를 `/api/relay/agent{,/start,/stop}`로 켜고 끈다(§4) |
| 콘솔 팝아웃 창 | `window.rs:192-280` | Tauri 창 라벨 `popout-<hex(instanceId)>`(`popout_label` `:21-24`)에 `index.html?console-popout=<id>&name=`을 붙여 같은 React 번들을 연다. 700×450(최소 400×250), 프레임 없음, 항상 위. 이미 열려 있으면 포커스한다. 열림·닫힘·되돌리기 요청은 메인 창에 `console:popoutOpened/Closed/popinRequest`로 알린다 |
| 메인 창 | `create_main_window()` `window.rs:100-148` | 라벨 `main`, 프레임 없음, 페이지 로드 뒤 표시, 배경 `#f0f0f2`. 크기는 설정 `windowBounds`(작업 영역보다 크지 않게, 최소 780×min(840, max(560, 작업 영역 높이))). 이동·크기 변경이 멈추면 500ms 디바운스로 저장한다(최대화·최소화 중 제외, `schedule_save_bounds` `:159-188`) |
| 탐색 잠금 | `lock_navigation()` `window.rs:70-96` | 모든 창에서 앱 주소(`tauri://localhost`, `tauri.localhost`, 개발 서버) 밖으로의 이동과 새 창을 막고, http(s) 링크만 바깥 브라우저로 연다 |
| 트레이 | `tray.rs:23-172` (메뉴 `49-149`) | 창 열기, 터미널 열기(Windows만, `POST /api/clients/launch {kind:'cli', view:'shell'}`), 데몬 상태, 데몬 종료/시작, "인터페이스만 종료", "완전히 종료". "데몬 종료"는 `stop_daemon()`(API 우선), "데몬 시작"은 API가 아니라 `start_daemon()`을 직접 부른다. 왼쪽 클릭·더블클릭은 창 표시, 아이콘은 테마별 favicon |
| 종료 플로우 | `window.rs:139-142`, `app_close_response` `native.rs:63-78`, `lifecycle.rs:614-678` | 메인 창 닫기를 막고 `app:closeRequest`를 보낸 뒤 `exit-interface`/`quit`/`cancel`을 받는다. `exit-interface`는 unregister(shutdown=false) 뒤 데몬을 두고 끝낸다. `quit`은 unregister(shutdown=true) → `stop_daemon()`. 이 경로를 거치지 않고 끝나면 `on_exit`가 자기가 띄운 데몬만 정리한다(인터페이스만 종료한 경우 제외). 종료 요청(`app.exit`)이 웹뷰가 죽어 닿지 않으면 1초 뒤 `on_exit`를 직접 부르고 프로세스를 끝낸다(`exit_app`) |
| 단일 인스턴스 | `lib.rs` `tauri_plugin_single_instance` | GUI는 한 번에 하나만 뜬다. 두 번째 실행은 창을 새로 만들지 않고 떠 있는 메인 창을 앞으로 가져온 뒤 끝난다. 여러 GUI가 한 데몬을 함께 쓰다 한쪽의 "완전히 종료"가 다른 쪽의 데몬을 꺼뜨리던 것을 막는다 |
| 로그 | `logging.rs:10-45` | `<data>/logs/saba-chan-<ts>.log`와 stderr에 같이 쓴다. 패닉도 남긴다 |
| 셸 i18n | `i18n.rs:1-62` | 트레이·알림·상태 문구는 `<installRoot>/locales/<lang>/common.json`을 먼저 읽고, 없거나 깨졌으면 exe에 `include_str!`로 넣은 10개 언어 사본을 쓴다. 없는 키는 영어 → 키 그대로. 언어를 바꾸면 다시 읽고 트레이를 갱신한다(`language_set` `native.rs:248-257`) |

**인증 방식 요약**: 데몬이 시작할 때 UUID 토큰을 만들어 `<dataDir>/.ipc_token`(Unix 0600)에 쓰고(`src/ipc/auth.rs:27-63`), 모든 클라이언트는 이 파일을 읽어 `X-Saba-Token`을 보낸다. `Host`가 루프백이 아니면 403으로 막는다(`auth.rs:75-84, 98-101`). `SABA_AUTH_DISABLED=1`은 개발 빌드(`debug_assertions`)에서만 통하고, 토큰이 없으면 아무 요청도 통과시키지 않는다(`auth.rs:104-125`). 토큰은 바이트를 끝까지 비교하는 상수 시간 비교로 맞춘다(`auth.rs:86-89, 134`). `/api/updates/*`도 auth layer 안에 있어 인증 대상이다. `/health`만 토큰 없이 답하고, 보낸 토큰이 맞는지(`token: valid|invalid|missing`)를 알려 준다(A6).

### 1.2 window.api 전수표 (`src/api/createApi.js` → daemon endpoint)

`window.api.*`는 `createApi.js:89-1037`, `window.electron.*`는 `:1039-1051`에 있다. 데몬 호출은 모두 `daemon_request` 명령(§1.1)을 거친다. 네이티브 작업은 Rust 명령 이름을 적었다(계약 §2, `native.rs`). "라우트 존재"는 `src/ipc/mod.rs`와 `updates.rs` 기준으로 대조했다(숫자는 줄). `tests/api_contract.rs`가 `createApi.js`를 읽어 GUI가 부르는 경로가 데몬 라우트에 있는지 검사한다(`:60-69`). "미사용"은 renderer(`src/`, 테스트 제외)가 부르지 않는다는 뜻이다.

| window.api 함수 (createApi.js 줄) | 호출 대상 | 라우트 존재 |
|---|---|---|
| serverList (90) | GET /api/servers (port_conflict_stops가 있으면 `notify` 명령으로 OS 알림). 서버마다 `managed`(runner가 맡는지, `Supervisor::uses_managed_start`)가 실린다(`ipc/mod.rs:208-210`, `handlers/server.rs:81`) | ✓ 643 |
| instanceStart (119) | POST /api/instance/:id/start (managed 여부와 포트 검사는 데몬이 정한다) | ✓ 646 |
| serverStop (139) | POST /api/server/:name/stop | ✓ 650 |
| serverStatus (160) | GET /api/server/:name/status | ✓ 644 |
| moduleList (181) | GET /api/modules | ✓ 651 |
| moduleRefresh (192) | POST /api/modules/refresh | ✓ 652 |
| moduleGetMetadata (212) | GET /api/module/:name | ✓ 659 |
| moduleAudit (225) | GET /api/module/:name/audit (위험 능력과 `[permissions]` 선언 비교) | ✓ 660 |
| instanceEvents (233) | GET /api/instance/:id/events (최근 종료 이력) | ✓ 647 (미사용) |
| moduleGetLocales (240) | GET /api/module/:name/locales | ✓ 712 |
| moduleListVersions (248) | GET /api/module/:name/versions?… | ✓ 695 |
| moduleInstallServer (260) | POST /api/module/:name/install | ✓ 697 |
| instanceGetInstalledVersion (268) | GET /api/instance/:id/installed-version | ✓ 698 |
| moduleManifest (275) | GET /api/modules/manifest | ✓ 656 |
| moduleInstallFromManifest (282) | POST /api/modules/manifest/:id/install | ✓ 657 |
| moduleRemove (289) | DELETE /api/modules/:id | ✓ 658 |
| instanceCreate (296) | POST /api/instances | ✓ 662 |
| instanceProvisionProgress (332) | GET /api/provision-progress/:name | ✓ 668 |
| instanceDismissProvision (339) | DELETE /api/provision-progress/:name | ✓ 668 |
| instanceCheckUpdate (346) | GET /api/instance/:id/check-update | ✓ 670 |
| instanceApplyUpdate (355) | POST /api/instance/:id/apply-update | ✓ 671 |
| instanceDelete (363) | DELETE /api/instance/:id | ✓ 665 |
| instanceReorder (394) | PUT /api/instances/reorder {order} | ✓ 663 |
| instanceUpdateSettings (408) | PATCH /api/instance/:id. 400이면 `{status, error, error_code, details, fields}`를 그대로 넘긴다(`invalid_format`이면 형식 오류 알림) | ✓ 665 |
| instanceLaunchPreview (452) | POST /api/instance/:id/launch-preview `{settings, extra_args?}` (extra_args를 안 넘기면 저장된 것) | ✓ 666 |
| instanceResetServer (457) | POST /api/instance/:id/server/reset | ✓ 686 |
| executeCommand (465) | POST /api/instance/:id/command {command, args}. 명령 해석과 라우팅(REST·stdin·모듈)은 데몬이 한다(464 주석). GUI는 `…/rcon`, `…/rest`를 직접 부르지 않는다 | ✓ 673 |
| instanceStop/Restart/Rename/Commands (493-498) | POST …/stop·…/restart {force}(restart는 150초 timeout), POST …/rename {name}, GET …/commands | ✓ 648, 649, 664, 674 (미사용) |
| instanceReadLiveSettings (500) | GET /api/instance/:id/settings/live | ✓ 685 (미사용) |
| runtimePreflight (501) | GET /api/runtime/preflight | ✓ 756 (미사용) |
| systemMetrics (503) | GET /api/system/metrics[?sizes=false] | ✓ 715 (`SystemMonitor`) |
| configDefaults / configReset (505-507) | GET /api/config/:kind/defaults, POST /api/config/:kind/reset {keys?} | ✓ 704-705 (미사용) |
| extensionList…extensionInitStatus (510-630) | /api/extensions, /:id/enable, /disable, /audit, /i18n/:locale, /gui, /gui/styles(`responseType:'text'`), /icon(`responseType:'base64'`), /manifest, /:id/install, DELETE /:id, /updates, /rescan, /init-status | ✓ 754-771 (확장 UI 로딩은 03 §4.1). `gui_allowed:false`면 번들·스타일을 요청하지 않고 사용자에게 한 번 묻는다(`ExtensionContext`). `/gui`가 403 `extension_gui_unapproved`면 `extensionGuiBundle`은 `{error_code:'extension_gui_unapproved'}`를 돌려주고 다시 묻는다 |
| managedConsole (634) | GET /api/instance/:id/console?since&count (4번째 인자 `{spans:true}`면 `&spans=1`) | ✓ 679 |
| managedStdin (642) | POST /api/instance/:id/stdin | ✓ 680 |
| consolePopout / consolePopoutList (651-652) | Rust `console_popout` / `console_popout_list` (열린 팝아웃 창의 instanceId 배열). daemon 호출 없음 | n/a |
| daemonConsole (653) | GET /api/daemon/console?since&count. `{lines:[{id, timestamp, level, target, message}], count}` 그대로 | ✓ 752 |
| consoleFocusPopout / Popin / ToggleAlwaysOnTop (658-661) | Rust `console_focus_popout` / `console_popin` / `console_toggle_always_on_top` | n/a |
| updaterCheck (676) | POST /api/updates/check | ✓ updates.rs:101 |
| updaterStatus (696) | GET /api/updates/status | ✓ 100 |
| updaterDownload (697) | POST /api/updates/download `{components}` (빈 배열이면 전부) | ✓ 102 |
| updaterDownloadProgress (705) | GET /api/updates/download/progress | ✓ 103 |
| updaterApply (707) | POST /api/updates/apply `{components}` (빈 배열이면 전부) | ✓ 104 |
| updaterLaunchApply (725) | Rust `updater_launch_apply` → POST /api/updates/launch-updater {relaunch:[current_exe]}. 업데이터는 데몬이 띄운다 | ✓ 105 |
| updaterLastResult (727) | GET/DELETE /api/updates/last-result (GET은 `?lang=`, 언어는 `language_get`) | ✓ 106 (미사용) |
| updaterGetConfig (737) | GET /api/updates/config | ✓ 108 |
| updaterSetConfig (738) | PUT /api/updates/config | ✓ 109 |
| settingsLoad (747) | GET /api/config/gui. 데몬이 응답하지 않을 때만 Rust `settings_local_load`(config.json `gui` 직접 읽기) | ✓ 702 |
| settingsSave (755) | GET /api/config/gui로 merge base를 읽고 PUT. 데몬이 **응답하지 않을 때만** Rust `settings_local_save`로 config.json `gui`에 직접 쓴다. 데몬이 오류 응답을 주면 저장 실패로 끝난다. 끝에 `refresh_ipc_base` | ✓ 702 |
| settingsGetPath (775) | Rust `settings_get_path` (`<dataDir>/config.json`) | n/a |
| openFileDialog / openFolderDialog (778-779) | Rust `dialog_open_file` / `dialog_open_folder` | n/a |
| migrationScanDir (782) | POST /api/modules/detect (`{ok, matches}`, 파일 목록 없음) | ✓ 655 |
| discordBotStatus / Start / Stop (793-816) | GET /api/relay/agent, POST /api/relay/agent/start·stop. Status는 `running`·`stopped`, 다른 PC가 같은 계정으로 페어링해 멈췄으면(`lastErrorCode: token_replaced`) `replaced`(연결 화면에 다시 페어링 안내). 노드 토큰이 없으면(`node_token_missing`) `cloud_token_not_found` | ✓ 787-789 |
| botConfigLoad (817) | GET /api/config/bot. 못 읽으면 기본값 대신 예외를 던진다(호출한 쪽이 기본값을 저장해 설정을 지우지 않게) | ✓ 700 |
| botConfigSave (826) | PUT /api/config/bot(최상위 키 단위 병합). 릴레이 동기화는 에이전트 하트비트가 맡는다 | ✓ 700 |
| saveNodeToken / loadNodeToken / deleteNodeToken (835-853) | PUT/GET/DELETE /api/config/node-token. DELETE는 데몬이 에이전트를 멈추고 토큰을 지운다 | ✓ 710 |
| auditList (855) | GET /api/audit?limit&instance (A10). `{entries}` 최신이 먼저 | ✓ 640 |
| permissionsGet (860) | GET /api/permissions/:guildId (P1) | ✓ 723-726 |
| permissionsUpdate (861) | PATCH /api/permissions/:guildId `{defaultLevel?, preset?, chatbotCap?, ownerDiscordId?, commandLevels?}` | ✓ 723-726 |
| permissionsSetMember (862) | PUT /api/permissions/:guildId/members/:discordId `{level}` | ✓ 727-730 |
| permissionsRemoveMember (863) | DELETE /api/permissions/:guildId/members/:discordId | ✓ 727-730 |
| sandboxListUnisolated (864) | GET /api/sandbox/unisolated `{components:[{component, fingerprint, current}]}` | ✓ 737 |
| sandboxAllowUnisolated (865) | POST /api/sandbox/unisolated/:component. `sandbox_unavailable` 응답의 `component`를 넘긴다 | ✓ 738-741 |
| sandboxRevokeUnisolated (866) | DELETE /api/sandbox/unisolated/:component | ✓ 738-741 |
| sandboxListExtensionGui / sandboxAllowExtensionGui / sandboxRevokeExtensionGui | GET /api/sandbox/extension-gui, POST·DELETE /api/sandbox/extension-gui/ext-<id>. 확인되지 않은 익스텐션의 GUI 번들 허락(묻는 창은 `ExtensionContext`, 목록·철회는 설정 > 고급 `ExtensionGuiSection`) | 데몬 #148 |
| actionsList (867) | GET /api/actions?surface=gui&instance?&locale? (P2) | ✓ 743 |
| actionsPlan (872) | POST /api/actions/:id/plan `{input, instance?}` | ✓ 746 |
| actionsRun (873) | POST /api/actions/:id/run `{input, instance?}` 또는 `{planId}` | ✓ 747 |
| discordSettingsGet (874) | GET /api/discord/:guild/settings (I2). `loaded:false`면 릴레이 값을 못 읽은 기본값이라 GUI는 편집을 막는다 | ✓ 732-735 |
| discordSettingsSet (875) | PATCH /api/discord/:guild/settings `{locale?, sleeping?, music?}` | ✓ 732-735 |
| openInTerminal (877) | POST /api/clients/launch `{kind:'cli', view, instance?}` (I4). 데몬이 새 콘솔 창에 CLI 콘솔·셸을 띄운다. 실패하면 데몬 본문(`error_code`)을 그대로 돌려준다. 트레이 메뉴 "터미널 열기"(Windows만)도 같은 라우트를 `view:'shell'`로 부른다(`tray.rs:433-446`) | ✓ 721 |
| onDaemonEvent (882) | 셸의 `events:message`(§1.1 이벤트 스트림)를 받는다. `type: 'resync'`면 목록을 다시 읽는다. 돌려주는 함수로 해제 (P4) | ✓ 641 |
| dataExport / dataImport (885-907) | Rust `dialog_save_file` / `dialog_open_file`로 경로를 고른 뒤 POST /api/migration/export `{path, secrets}` · /api/migration/import `{path, dryRun, overwrite}`. 취소하면 `{canceled:true}`. 가져오기 결과에는 `path`를 붙여 미리보기(`dryRun`) 뒤 같은 파일로 다시 부를 수 있다 | ✓ 707-708 (A9) |
| relayCheckHostStatus… relayBotInvite (910-949) | GET /api/relay/host/:id/status, /nodes, GET /api/relay/node/:guild/members?withStatus=1(`{members, partial, reason?}`, 20초 제한), GET /api/relay/bot-invite | ✓ 774-776, 781 |
| relayListLegacyMembers | GET /api/relay/node/:guild/legacy-members(릴레이에 남은 부여값 `[{id, username, displayName, level}]`, 권한 가져오기 후보). 실패하면 오류 본문 | ✓ |
| relayNodeChannels / relayNodeChannelsSave (950-953) | GET/PUT /api/relay/node/:guild/channels(`notify` 알림 채널 포함). 실패하면 데몬 본문(`{error, channelId?}`)을 그대로 돌려준다 | ✓ 777-780 |
| instanceAgreements (956) | POST /api/instance/:id/agreements `{accept:[id]}` (모듈 약관 동의) | ✓ 687 |
| onboarding (957) | GET /api/onboarding (J1-a 다음 할 일) | ✓ 639 |
| instanceBackups…instanceScheduleSave (958-970) | GET/POST …/backups, DELETE …/backups/:name, POST …/backups/:name/restore(만들기·복원은 10분 timeout), GET/PUT …/schedule | ✓ 690-693 (미사용) |
| chatbotGet/Save/Clear/ClearMemory (971-975) | GET/PUT/DELETE /api/chatbot, DELETE /api/chatbot/memory?guildId | ✓ 793-794 (미사용) |
| chatbotModels | GET /api/relay/node/:guildId/chatbot/models (릴레이 답 그대로: `{models, selected, effectiveModel}`, 409 NO_API_KEY·422·503) | ✓ 챗봇 화면 모델 드롭다운. 고른 모델을 대신 쓰는 중이면(`effectiveModel`≠`model`) 알리고 그 모델을 고른 상태로 둔다 |
| relayInitiatePairing / relayPollPairingStatus (976-990) | POST /api/relay/pair/initiate, GET /api/relay/pair/:code/status | ✓ 782-783 |
| relayLinkRequests / relayLinkRequestRespond (992-994) | GET /api/relay/link-requests, POST /api/relay/link-requests/:id `{accept}`. 디스코드에서 코드 없이 온 연결 요청(J1-b). 데몬 본문을 그대로 돌려준다(`daemonJson`) | ✓ 784-785 |
| shellOpenPath (997) | Rust `shell_open_path`. 디렉터리가 아니면 열지 않는다(`errors.path_not_folder`) | n/a |
| logsGetPath / logsOpenFolder (1001-1002) | Rust `logs_get_path` / `logs_open_folder` | n/a (미사용) |
| launchUninstaller (1003) | Rust `launch_uninstaller` (installer 직접 spawn) | n/a |
| getAppVersion (1004) | Rust `app_get_version` (패키지 버전) | n/a |
| getComponentInfo (1005) | GET /api/system/components (실패하면 `app_get_version`만) | ✓ 714 |
| onCloseRequest / closeResponse (1021-1025) | 셸의 `app:closeRequest`를 받고, Rust `app_close_response`로 답한다(기다리지 않음) | n/a |
| onStatusUpdate (1026) | 셸의 `status:update`. 브리지가 마지막 값을 기억해 늦게 붙은 콜백에도 준다(`events.js:137-177`) | n/a |
| daemonStatus (1030) | Rust `daemon_status` → GET /health | ✓ 637 |
| daemonRestart (1031) | Rust `daemon_restart`: `stop_daemon()`(API 종료, 안 되면 자기 PID만 kill) 후 `/health`가 멎을 때까지 기다렸다가 **`start_daemon()`으로 직접 spawn**(`lifecycle.rs:576-602`) | 부분 |
| minimize/maximize/closeWindow (1034-1036, electron 1040-1042) | Rust `window_minimize` / `window_maximize`(토글) / `window_close`. 호출한 창에 적용한다 | n/a |
| electron.getLanguage/setLanguage (1043-1044) | Rust `language_get`(config.json `gui.language`를 직접 읽는다) / `language_set`(`save_settings`로 API 우선 저장, 셸 번역·트레이 갱신. 데몬은 재시작하지 않는다, `native.rs:241-257`) | 부분 |
| electron.shell.openExternal (1046) | Rust `shell_open_external` (http(s)만) | n/a |

셸에서 renderer로 가는 push 이벤트는 Tauri 이벤트로, 메인 창(라벨 `main`)에만 보낸다(`emit_main` `lib.rs:60-64`). 팝아웃 창의 브리지는 구독하지 않는다(`install.js:19-21`). 브리지는 모듈을 읽을 때 8개 채널을 한 번 구독한 뒤(`events.js:124-133, 182-184`) `app_ready`를 부른다(`install.js:23-32`). 렌더러를 새로고침하면 `app_ready`가 다시 와서 셸이 마지막 `status:update`를 다시 보낸다(`lifecycle.rs:66-71`). 채널: `status:update`(로딩 단계 `{step, message, ts}`. `message`는 `common.json`의 `daemon.*`·`init.*`로 번역된다. 데몬이 10초 안에 응답하지 않으면 `step:'daemon_failed'`를 보내고 `ready`는 보내지 않는다. 그 뒤 `daemon_restart`가 성공하면 멈춘 초기화를 이어서 끝내고 `ready`를 보낸다. 실패하면 `{success:false, error}`), `events:message`, `updates:available`, `updates:completed`, `console:popoutOpened/Closed/popinRequest`, `app:closeRequest`.

**renderer(src/, 테스트 제외)에서 쓰지 않는 window.api**: 위 표의 "미사용" 항목(`instanceEvents`, `instanceStop/Restart/Rename/Commands`, `instanceReadLiveSettings`, `runtimePreflight`, `configDefaults/Reset`, `updaterLastResult`, 백업·예약 6개, `chatbot*` 4개, `logsGetPath`, `logsOpenFolder`)과 `loadNodeToken`, `deleteNodeToken`, `relayBotInvite`. 반대로 **GUI가 호출하지 않는 daemon 라우트**: `/api/server/:name/start`, GET `/api/instances`, GET `/api/instance/:id`, `/api/instance/:id/rcon`, `/rest`, `/managed/start`, `/validate`, `/properties`(GET/PUT), `/properties/reset`, `/diagnose`, `/api/module/:name/version/:version`, `/api/version`, `/api/modules/schema`, `/api/config/client/:kind`, `/api/extensions/:id/mount|unmount|config`, `/api/chat/command`, `/api/updates/integrity`. 이 중 일부는 CLI만 사용한다(§3).

### 1.3 GUI가 읽고 쓰는 설정 파일

모든 사용자 설정은 `<dataDir>/config.json` 하나에 네임스페이스별로 들어간다(`updater/src/config_file.rs:1-27`). 예전 개별 파일(`settings.json`, `bot-config.json` 등)은 데몬이 한 번 이관하고, 원본은 남긴 채 `.migrated` 사본을 만든다(`config_file.rs:37-45`).

| 파일 | 경로 | 접근 방식 | 스키마 |
|---|---|---|---|
| config.json `gui` | `<dataDir>/config.json` (`settings.rs:11-13`) | 셸 `load_settings()`(`settings.rs:81-94`, 명령 `settings_local_load`)는 **항상 파일을 직접 읽는다**(깨졌으면 `.bak`, `gui`가 아직 없으면 예전 `settings.json`, `:27-38`). 저장은 브리지 `saveSettings()`(`createApi.js:57-74`)와 셸 `save_settings()`(`native.rs:189-199`, 언어·창 크기)가 먼저 `PUT /api/config/gui`를 보내고, 데몬이 **응답하지 않을 때만** `write_gui_section()`(명령 `settings_local_save`)으로 `gui` 키만 바꾼다. 데몬과 같은 writer(`updater/src/config_file.rs` `ConfigFile::set_section`: tmp→fsync→rename, 직전 정상본은 `.bak`, 깨진 파일은 `.corrupt-<시각>`)를 쓴다 | 기본값(`settings.rs:63-78`): `autoRefresh, refreshInterval(2000), windowBounds{width,height,x,y}, language, ipcPort(57474), consoleBufferSize(2000), autoGeneratePasswords, portConflictCheck`. renderer가 추가로 저장하는 키(useSettingsStore.js:79-97): `accentColor, accentSecondary, useGradient, fontScale, enableTransitions, consoleSyntaxHighlight, consoleBgColor, consoleTextColor, sidebarCompact, consoleFontScale` |
| config.json `relay` | daemon이 관리(`/api/config/bot`, `src/config_store/mod.rs:208, 282`) | GUI는 API만 사용 | `prefixes[], cloud{relayUrl,hostId}, moduleAliases·commandAliases·instanceAliases(예전 전역 별명, 시작할 때 nodeSettings[guild].instances로 옮겨져 빈다), nodeSettings, cloudNodes[], cloudMembers{}`. 로컬 봇 필드(`token`, `autoStart`, `mode`, `music*`)는 없다(01 §4 BotConfig) |
| config.json `chatbot` | daemon이 관리(`/api/chatbot`) | API | 응답에는 API 키 대신 `hasApiKey`만 온다(`src/ipc/handlers/chatbot.rs:1-15`) |
| node token | `/api/config/node-token` | API | `{token: "sbn_<nodeId>.<secret>"}` |
| .ipc_token | `<dataDir>/.ipc_token` 또는 `SABA_TOKEN_PATH` | 직접 읽기(필수 부트스트랩) | 평문 UUID |
| logs | `<dataDir>/logs/*.log` | 직접 쓰기 | — |
| renderer localStorage | WebView 저장소(Windows WebView2) | 직접 | `i18nextLng`(i18n.js `getInitialLanguage`·`getCachedLanguage`, SettingsModal.js:249), 테마 키(themeManager.js:50-120), 공지 목록(NoticeModal.js:15-25) |

**쓰기 분기(해결됨).** 예전에는 `settings:load/save`와 `saveSettings`가 "이 GUI가 daemon을 spawn했는가"(`daemonProcess !== null`)로 분기해서, 이미 떠 있던 daemon에 붙으면 settings.json을 직접 썼다. 지금은 "daemon이 응답하는가"로 나눈다. 데몬이 응답하면 API로만 쓰고, 오류 응답이면 로컬로 넘어가지 않고 실패로 끝낸다(`createApi.js:57-74, 747-774`, `native.rs:189-199`). Tauri로 옮기면서도 이 규칙은 그대로다. 남은 직접 접근은 **읽기**다. `load_settings()`는 데몬 상태와 상관없이 파일을 읽고, `language_get`, `create_main_window`, `refresh_ipc_base`, 데몬 기동 env, 창 크기 저장(`window.rs:159-188`, 파일에서 읽어 merge한 뒤 API로 저장)이 이 값을 쓴다.

### 1.4 화면과 모달별 기능 (`src/`)

- **App.js**(1057줄): 루트 컴포넌트. `ExtensionProvider`로 감싸고, `useServerActions`가 `refreshInterval` 주기로 `serverList`를 폴링한다. 종료 질문 모달, 로딩 화면(`status:update`), Discord 패널(창이 넓으면 side-panel, 60)과 릴레이 에이전트 시작/정지(`useDiscordStore`, 157-158)를 맡는다. 봇 자동 시작과 `bot:relaunch` 처리는 로컬 봇과 함께 없어졌다.
- **ServerCard.js**: 인스턴스 카드. 상태, 시작/정지, 콘솔, 설정, 명령 버튼과 REST host:port 표시가 있다. 게임 서버 업데이트 확인/적용(`instanceCheckUpdate/ApplyUpdate` 45-72), 네이티브 프로비저닝 진행률(`instanceProvisionProgress` 100, `NativeProvision.js`), 팝아웃 콘솔 포커스(442)를 제공한다. `useDragReorder`로 카드를 드래그해 순서를 바꾸면 `instanceReorder`가 호출된다.
- **ConsoleView / ConsoleDock / ConsoleWindow**(useMultiConsole): managed 프로세스 콘솔. `managedConsole` 폴링, `managedStdin`, `executeCommand`(stdin이 실패할 때 폴백)를 쓰고, PiP 팝아웃과 always-on-top을 지원한다.
- **AddInstanceModal**: 진입점이 둘이다.
  - `AddInstanceNewServer.js`: 게임 모듈 카드 그리드에서 모듈을 고른 뒤 이름과 옵션을 입력하고 `instanceCreate`를 호출한다. 모듈 설정 중 `install = true`인 select(예: 마인크래프트 서버 종류)를 함께 묻고 `module_settings`로 보낸다. 마이그레이션 생성 응답에 `detected`가 있으면 완료 안내에 서버 종류와 플러그인·모드를 덧붙인다(`useServerActions.js` `handleAddServer`). 명령 칩은 `group`(플러그인 이름)이 있으면 표시한다(`CommandModal.js`). CLI `server add`도 `install` 설정을 `--set`으로 받거나 묻고 생성 요청에 싣는다(`saba-chan-cli/src/cmd/server.rs`). 모듈에 `[[presets]]`가 있으면 `server add --preset <id>`로 고르거나 새 서버일 때 묻고 `preset`으로 보낸다.
  - `AddInstanceMigration.js`: 폴더 선택 → `migrationScanDir`(daemon `/api/modules/detect`, 파일 목록은 주지 않는다) → 모듈 자동 감지(`dir_signatures`). 실패하면 `moduleManifest`에서 수동으로 고른다.
- **ServerSettingsModal.js + useServerSettings.js**: 모듈 `settings.fields` 스키마로 필드를 렌더링하고(`instanceUpdateSettings`), 버전 선택과 설치(`moduleListVersions`, `moduleInstallServer`, `instanceGetInstalledVersion`), 서버 리셋(`instanceResetServer`)을 제공한다. 인스턴스별 bot alias와 권한 편집도 여기서 하는데, bot-config을 직접 load/save한다(515-592).
- **CommandModal.js**: 모듈 `commands.fields` 기반 명령 힌트, 자동완성, 입력 폼. `executeCommand`(82)로 실행한다.
- **SettingsModal.js**: 탭은 `general / appearance / extensions / advanced`(335-353)이다. 버전과 컴포넌트 정보(`getComponentInfo`), 언어, 테마, IPC 포트, 언인스톨(`launchUninstaller`)이 있고, SabaStorage로 들어가는 진입점이 있다.
- **SabaStorage.js**(구 업데이트 센터, 1197줄): 탭은 `components / modules / extensions`(53-70)이다.
  - components 탭: updater check/download/progress/apply/launchApply.
  - modules 탭: 설치된 모듈과 `saba-chan-modules` 레지스트리(`moduleManifest`, `moduleInstallFromManifest`, `moduleRemove`).
  - extensions 탭: 확장 레지스트리.
  - devMode의 Mock 릴리스 서버 토글(`handleToggleMock`): qcza9f의 옛 renderer는 L3에서 없어진 `api_base_url`/`github_*`를 보내 효과가 없다. GUI 리메이크(PR #5, fd26f4e)에서 고쳤다: `relay_url`만 보내고, `mock_relay`로 빌드한 개발판에서만 보인다.
- **DiscordBotModal.js**(1134줄): 클라우드 릴레이 전용이다(local 모드, 봇 토큰, 음악 설정은 없어졌다). prefix, 노드(길드)별 `allowedInstances`와 멤버별 차단 명령(`memberPermissions`, 멤버 목록은 `relayListNodeMembers`, 125)을 다룬다. 페어링은 `relayInitiatePairing`(351)으로 시작해 `relayPollPairingStatus`(364)로 진행 상황을 확인하고 `saveNodeToken`(373)으로 마무리한다. 기본 relay는 `https://saba-chan.online`(9)이다. side-panel 모드가 있다(845-848). 명령 별명은 ServerSettingsModal에서 편집한다.
- **BackgroundModal.js**: daemon 상태(`daemonStatus`)와 재시작(`daemonRestart`).
- **NoticeModal.js**: 앱 내 공지와 알림 목록(localStorage). 업데이트 모달로 연결된다.
- 공용 모달(Modals.js): Success/Failure/Notification/Question. TitleBar(frameless 창 컨트롤), Toast, CustomDropdown, Icon, ExtensionSlot(확장 UI 주입 지점. 세부 내용은 다른 문서).
- 상태 저장소(zustand): `useServerStore`, `useSettingsStore`(설정 변경 후 500ms debounce로 자동 저장, 154-163), `useDiscordStore`(에이전트 상태, 봇 설정, 시작/정지), `useUIStore`(모달 표시).
- 테스트(`src/test/`): gui-e2e, settings-roundtrip, store-unit, discord-side-panel, console-window, theme-contrast. 모두 `window.api`를 mock한다(setupTests.js). `tauri-bridge.test.js`는 가짜 `window.__TAURI__`로 브리지 설치·응답 가공·이벤트를 검사한다.

---

## 2. 공유 상수와 포트 (`updater/src/constants.rs`, `shared/constants.js`)

`shared/constants.js`를 import하는 코드는 이제 없다. 옛 Electron main.js만 `require`했고, 지금은 주석으로만 언급된다(`saba-chan-gui/src/stores/useSettingsStore.js:4`, `updater/gui/src/updater.js:9`). Tauri 셸과 CLI는 Rust 쪽 SSOT `saba_chan_updater_lib::constants`(`updater/src/constants.rs`)를 쓴다.

| 상수 | 값 | 사용처 |
|---|---|---|
| APP_NAME | `saba-chan` | dataDir 이름(`constants.rs:22`) |
| DEFAULT_IPC_PORT | `57474` | `constants.rs:32`. 셸 `settings.rs:7, 108-115`, CLI. renderer는 사본을 둔다(`useSettingsStore.js:4`) |
| DEFAULT_DAEMON_URL | `http://127.0.0.1:57474` | shared/constants.js에만 있고 쓰는 곳이 없다 |
| SUPPORTED_LANGUAGES | en, ko, ja, zh-CN, zh-TW, es, pt-BR, ru, de, fr | `constants.rs:42`. 셸은 OS 로캘을 이 목록에 맞춘다(`settings.rs:137-148`). **중복 정의 위치**: `saba-chan-gui/src/i18n.js:9`, `updater/gui/src/updater.js:10` |
| resolve_data_dir() (JS `getSabaDataDir()`) | `SABA_DATA_DIR`, Windows는 `%APPDATA%/saba-chan`, 그 외는 `$HOME/.config/saba-chan` | `constants.rs:55`. 셸 `lib.rs:79` |

- 그 밖의 포트:
  - 모의 릴레이 `scripts/mock-release-server.js` 9876 (개발용, GUI가 띄우지 않음)
  - Vite dev 5173 (`vite.config.js:24`, `tauri.conf.json` `devUrl`, 탐색 잠금은 이 출처를 앱 주소로 본다 `window.rs:74-78`)
  - Relay 기본 URL `https://saba-chan.online` (renderer `components/discord/DiscordPage.js:18`, 데몬 `src/ipc/handlers/relay.rs:41`). Tauri 셸에는 없다

---

## 3. CLI (saba-chan-cli, Rust)

> 이 절은 CLI 리메이크(PR #6, 브랜치 `claude/cli-remake-b1wszk`) 기준이다. 예전 TUI(`tui/`, `process.rs`, `config.rs`, `gui_config.rs`, `cli_config.rs`, `module_registry.rs`)는 이 리메이크에서 모두 지워졌다.

### 3.1 쓰는 방법 세 가지
- **한 번 실행하는 하위 명령**: clap 기반(`cli.rs`). `status`, `start|stop|restart <서버>`, `console`, `run <서버> <게임 명령>`, `server …`(list, info, add, remove, commands, send, settings, set, properties, rest, update, history, validate, diagnose, agree, reset, rename, reorder 등), `daemon …`, `module …`, `extension …`, `bot …`, `backup …`, `schedule …`, `chatbot …`, `update …`, `config …`, `chat <문장>`, `monitor [--watch] [--interval 초]`, `version`, `shell`, `completions <셸>` (`cli.rs:41-105`). 콘솔에 한 줄을 그대로 보내는 명령은 `server send`다(`rcon` 하위 명령은 없다).
- **대화형 셸**: 인자 없이 터미널에서 실행하면 열린다(`main.rs:67`, `shell.rs`). 한 줄이 명령 하나이고 Tab으로 명령·서버 이름·게임 명령을 채운다(게임 명령은 `catalog`에서, `shell.rs:109`). 첫 단어가 명령이 아니면 디스코드와 같은 채팅 처리기로 보낸다(`shell.rs:1-5, 187`). 셸을 닫아도 데몬과 서버는 계속 돈다.
- **스크립트용 `--json`**: stdout에는 JSON 문서 하나만 나가고, 안내와 진행 표시는 stderr로 간다(`output.rs:1-6`). 종료 코드: 0 성공, 1 실패, 2 사용법·모호함, 3 데몬 응답 없음, 4 대상 없음, 130 취소(`error.rs:39-47`). 전역 옵션은 `--json`, `--yes`, `--lang`, `--port`, `--no-color`.

### 3.2 구조
- `client.rs`: 데몬 HTTP 클라이언트. 토큰 파일을 읽어 `X-Saba-Token`을 보내고, 401이면 토큰을 다시 읽어 1회 재시도한다(`client.rs:28-34, 95-104`). 업데이터 실행도 데몬 API로 요청한다(`launch_updater` `:620-624`).
- `bootstrap.rs`: 데몬이 뜨기 전에 필요한 것만 직접 한다. `config.json`의 `gui.ipcPort`·`gui.language` 읽기(포트는 `SABA_IPC_PORT` > 파일 > 기본값, 언어는 `--lang` > `SABA_LANG` > 파일 > 시스템 로캘 > en, `:18-46`), `saba-core` 실행 파일 찾기와 분리 실행(`:49-90`), 정상 종료가 안 될 때 **그 PID만** 강제 종료(`:92`).
- `ctx.rs`: 데몬이 필요한 명령 앞에서 `ensure_daemon`이 데몬을 확인한다. 꺼져 있으면 물을 수 있을 때(대화형, `--yes` 아님) 켤지 묻고 `start_daemon`으로 띄운다(`:22-41, 43-61`).
- `catalog.rs`: 게임 명령을 하드코딩하지 않는다. `GET /api/modules`의 `commands`·`command_aliases`와 `GET /api/instance/:id/commands`로 목록·사용법·입력 칸을 만든다(`:1-9`). 인자 검사의 최종 판단은 데몬이 한다. 명령의 `method`는 데몬이 준 값을 그대로 보여 줄 뿐 분기하지 않는다(`server commands --json`, `cmd/server.rs:931`). 데몬은 콘솔 명령의 method를 `console`(`console_template`)이라 부르고 예전 이름 `rcon`·`rcon_template`도 별칭으로 받는다(09b8e4f, 02 §1.2).
- `slots.rs`: 데이터 slot(03-extensions 4.3)을 출력만 한다. 서버의 `slots`(`ServerCard.badge`, `ServerDetail.status`)와 익스텐션의 `slots["AddServer.options"]`를 읽고, 답은 서버를 만들 때 `extension_data`로 보낸다.
- `i18n.rs`: 설치 폴더의 `locales/<lang>/cli.json`을 먼저 읽고, 없으면 바이너리에 넣어 둔 en/ko를 쓴다(`:11-12, 36-45`). clap 도움말은 `help.cmd.*`·`help.arg.*` 키가 있으면 그 문구로 바꿔 끼운다(`cli.rs:617-645`).
- `cmd/`: 명령 갈래마다 파일 하나(backup, bot, chatbot, config, console, daemon, extension, module, schedule, server, status, update).
- 콘솔 색: 터미널이 색을 쓸 때만 `?spans=1`을 붙여(`client.rs` `console`) 데몬이 준 토큰 구간을 토큰별 ANSI 스타일로 칠하고, 나머지는 줄 `level` 색을 쓴다(`cmd/console.rs` `format_line`, `token_style`). 토큰은 02 §`HighlightRuleToml`.

- `bot pair [--relay-url URL]`: 페어링 코드를 받아 보여 주고(디스코드 `/사바쨩 연결`·`/saba connect`에 넣는다) claimed까지 기다린다. 데몬은 설정한 릴레이와만 페어링하므로, `--relay-url`이 설정과 다르면 봇 설정 `cloud.relayUrl`을 먼저 바꾼 뒤 페어링한다. `--json`만 주면 코드만 내고 끝, `--yes`면 기다린다.
- `bot channels GUILD [--chatbot 이름,…] [--no-chatbot] [--music 이름] [--no-music] [--notify 이름] [--no-notify]`: 채널 이름이나 id를 GET 응답의 `channels`로 id로 바꿔 PUT한다. `--notify`는 알림 채널(I3: 크래시·백업·업데이트·예약 예고)이다. 음악 채널을 새로 정할 때는 메시지가 지워진다고 확인을 받는다.
- `bot link [ID] [--decline]`: 디스코드에서 코드 없이 `/사바쨩 연결`로 온 연결 요청(J1-b). ID 없이 부르면 목록(`GET /api/relay/link-requests`), ID면 수락, `--decline`이면 거절(`POST /api/relay/link-requests/:id`). `bot pair`는 끝에 GUI 디스코드 화면에서도 볼 수 있다는 안내를 붙인다(J1-c).
- `bot allow GUILD [서버…] [--all]`: 그 디스코드 서버가 보고 부를 수 있는 서버(`nodeSettings[guild].allowedInstances`, Q-4). 서버 없이 부르면 보기, `--all`은 키를 지워 제한 없음. `bot word [MODULE] [COMMAND] [말…] [--clear]`: 채팅에서 명령 대신 쓰는 말(`commandAliases`, 모듈 → 명령 → "말1, 말2"). 둘 다 `nodeSettings`·`commandAliases` 맵을 통째로 `PUT /api/config/bot`에 보낸다(권한 키는 데몬이 지킨다).
- `bot lang GUILD [auto|코드]`,
 `bot sleep GUILD`, `bot wake GUILD`, `bot music GUILD [--on|--off] [--ui on|off] [--queue-lines N] [--refresh MS] [--normalize on|off]`: 디스코드 길드 설정(I2) `GET/PATCH /api/discord/:guild/settings`. 값이 없으면 보기만 한다. 봇에 닿지 않아 `sync.ok:false`면 다음 연결 때 보낸다고 알린다.
- `server launch-preview [SERVER] [--args "..."]`: `POST /api/instance/:id/launch-preview`의 `command_line`을 보여 주고, 형식이 틀린 인자는 "잘못된 형식입니다"로 알린다. `--args`가 없으면 저장된 추가 인자로 본다.
- `audit [SERVER] [--limit N] [-f]`(A10, 별명 `activity`): `GET /api/audit`를 표로 보여 준다. `-f`는 목록을 읽기 전에 `GET /api/events`에 붙고, 이어서 `action.run`을 한 줄씩 따라간다(서버를 주면 그 서버 것만, `--json`이면 data 한 줄씩). 끊기면 2초 뒤 마지막 번호로 다시 붙는다. CLI 요청에는 `X-Saba-Client: cli`가 붙는다.
- `do [ACTION] [--server S] [--plan] [KEY=VALUE…]`(P2): 액션 카탈로그. 액션 없이 부르면 `GET /api/actions?surface=cli` 표. 있으면 `plan`을 부르고, `needsConfirm`이면 요약을 보이고 묻은 뒤 `planId`로 `run`한다. `--plan`은 요약만. 값이 JSON으로 읽히면(숫자·참거짓·배열·객체) 그대로 보낸다.
- `config export FILE [--secrets]`·`config import FILE [--dry-run] [--overwrite]`(A9): 경로를 절대 경로로 바꿔 `/api/migration/*`에 넘긴다. 파일은 데몬이 읽고 쓴다.

사용자 안내서는 `saba-chan-cli/README.md`에 있다.

## 4. 디스코드 연동 (데몬 쪽)

> 로컬 Discord 봇 실행(`discord_bot/`, Node, ext-process)은 레거시이며 L1(7b5102f)에서 지웠다. 디스코드 봇 자체는 클라우드 릴레이에서 돌고, 그 쪽(게이트웨이·음악·챗봇)은 지금 server-chan에서 다시 만드는 중이라 여기서 다루지 않는다. 이 절은 데몬 쪽의 안정된 계약만 적는다.

### 4.1 구조

```
Discord ─ 릴레이의 사바쨩 봇 ─(롱폴링, HMAC 서명)─ 데몬 relay_agent ─ 채팅 처리기(crate::chat) ─ 데몬 내부 API
```

- 데몬이 먼저 릴레이로 나가서 연결한다. 데몬은 들어오는 연결을 받지 않는다.
- 페어링으로 받은 노드 토큰(`sbn_<id>.<secret>`)이 있어야 에이전트가 뜬다. 페어링은 `/api/relay/pair/initiate`, `/api/relay/pair/:code/status`(01 §4 Relay 프록시)로 하고, 토큰은 `/api/config/node-token`으로 다룬다.
- AI 챗봇 설정의 원본은 데몬 config.json `chatbot`이다. `GET/PUT/DELETE /api/chatbot`, `DELETE /api/chatbot/memory?guildId=`로 다루고, 바뀔 때마다와 에이전트가 연결될 때 이 호스트의 길드마다 릴레이에 PUT해서 맞춘다. 릴레이 PUT은 받은 키만 합치므로 데몬은 늘 전체 설정을 보내고, 지운 키는 릴레이 기본값으로 채운다(`handlers/chatbot.rs:31-59`). DELETE가 릴레이에 닿지 못하면 `pendingRelayDelete: true`로 답하고 다음 연결 때 다시 지운다(`:198-218, 220-235, 267-275`). API 키는 데몬과 릴레이만 알고, 클라이언트에는 `hasApiKey`만 준다(`handlers/chatbot.rs:1-15`, `mod.rs:753-754`).

### 4.2 릴레이 에이전트 (`src/relay_agent.rs`)

| 단계 | 요청 | 내용 |
|---|---|---|
| 버전 확인 | `GET /info` | 릴레이가 요구하는 최소 에이전트 버전 확인 |
| 하트비트 | `POST /heartbeat` (60초) | `agentVersion`, `os`, `metadata`(서버·모듈·봇 설정). 릴레이가 `UPDATE_REQUIRED`를 주면 경고 로그를 남긴다(`relay_agent.rs:221-258`). 잠자기는 길드 설정(I2)이라 폴링은 쉬지 않는다. 게임 서버 비밀번호(`rcon_password`, `rest_password`)는 메타데이터에서 뺀다(273-276) |
| 명령 받기 | `GET /poll` (롱폴링) | 디스코드에서 온 명령. 현재 `payload.action`은 `raw_command` 하나다 |
| 결과 보내기 | `POST /result/:id` | 채팅 처리기의 응답 `{success, data:{text}}` (`:388-403`) |

- 모든 요청은 노드 토큰으로 HMAC 서명하고 `X-Saba-Client: 1` 헤더를 붙인다(`relay_agent.rs:162-166, 179-189`, `handlers/relay.rs:102` `signed_headers`). 노드 토큰을 싣는 프록시 요청(`/api/relay/host/*`, `/node/*`)은 `?relayUrl=`이 설정된 릴레이와 같거나 루프백일 때만 따르고, 아니면 설정된 릴레이로 보낸다(`handlers/relay.rs:45-67`).
- 데몬이 뜨면 `auto_start`가 에이전트를 켠다(토큰이 없으면 info 로그만 남기고 건너뜀, `ipc/mod.rs:787-789`, `relay_agent.rs:155-160`). 에이전트가 연결되면 AI 챗봇 설정도 릴레이에 맞춘다(`relay_agent.rs:111`).
- 상태 `GET /api/relay/agent`: `{running, relay_url, has_token, agent_version, last_heartbeat_at, last_error, commands_handled}` (`relay_agent.rs:31-42`). 켜고 끄기는 `POST /api/relay/agent/start`, `/stop`.
- 새 명령 종류는 `handle_payload`의 action 분기에 더한다.

### 4.3 채팅 처리기 (`src/chat/`)

- 디스코드 명령과 CLI `chat`, `POST /api/chat/command {text, guild_id?, user_id?} → {reply, code, args}`가 모두 같은 처리기를 쓴다(`handlers/chat.rs:9-13`).
- 대상 서버 찾기: 인스턴스 이름·id → 그 길드에서 그 서버를 부르는 낱말(`nodeSettings[guild].instances[id].words`) → 모듈 이름·module.toml 별명 순이다. 같은 게임 서버가 여럿인데 하나로 좁혀지지 않으면 `errors.multiple_instances`로 답한다(`chat/mod.rs` `named_servers`·`find_server`, 01 §4).
- 명령 낱말은 그 길드·서버의 `commands`, 그다음 module.toml 별명이고, 접두사는 `prefixes`다. 맞춰 볼 때 대소문자를 가리지 않고 긴 것부터 보며, 접두사 뒤가 공백·쉼표·쌍점이거나 글 끝이어야 한다. 도움말에는 첫 접두사를, 비어 있으면 언어별 `default_prefix`(bot.json)를 쓴다. 응답 문구는 `locales/<lang>/bot.json`을 바이너리에 넣어 쓴다(`chat/i18n.rs`).
- 노드(디스코드 서버)별 허용 인스턴스와 멤버별 명령 권한은 `nodeSettings`에 있다.

### 4.4 봇 초대

- `GET /api/relay/bot-invite`(query `relayUrl?`)가 릴레이의 공개 `/api/bot-invite` 응답 `{url}`을 그대로 돌려준다. 공개 엔드포인트라 서명하지 않지만, 주소는 서명 요청과 같이 설정한 릴레이나 루프백만 쓴다(`handlers/relay.rs:265-287`).

## 5. 로케일 구조 (`locales/<lang>/{common,gui,bot,cli}.json`, 10개 언어)

| 파일 | 최상위 키(en) | 소비자 |
|---|---|---|
| common.json (leaf 80개) | app_name, success/error/…, daemon, modules, server, network, port_conflict, init, tray, updates, errors 등 25개 | GUI renderer(`common` ns, JS 브리지의 오류 문구 `createApi.js:9`), **Tauri 셸**(트레이·알림·status 메시지, `i18n.rs`. 설치 폴더 파일 우선, 없으면 exe 내장본) |
| gui.json (en leaf 677개) | title, menu, title_bar, modals, settings_modal, add_instance_modal, migration_modal, command_modal, server_*, discord_bot, discord_modal, notice_modal, updates, extensions, saba_storage, validation, context_menu 등 29개 | GUI renderer(`gui` ns) |
| bot.json | 채팅 응답 문구 | 데몬 채팅 처리기(`src/chat/i18n.rs`, 빌드 때 포함). GUI renderer도 import한다 |
| cli.json (PR #6 기준 en leaf 402개) | server, daemon, module, ext, bot, backup, schedule, chatbot, err, prompt 등 39개 | CLI `i18n.rs`. 설치 폴더의 `locales/<lang>/cli.json`을 먼저 읽고, 없으면 내장 en/ko를 쓴다(`i18n.rs:11-12, 36-45`). ko만 도움말 번역 `help`(leaf 183개)가 더 있어 585개다. 나머지 언어의 도움말은 clap 기본 영어다. qcza9f에는 아직 예전 TUI용 cli.json(en 200개, `screen`·`menu` 등 16개)이 있다 |

- GUI renderer는 영어 common, gui, bot JSON만 번들에 넣고, 다른 언어는 그 언어로 바꿀 때 `import.meta.glob` 청크로 불러온다(i18n.js의 `localeBackend`). 첫 렌더는 캐시된 언어 파일을 읽은 뒤로 미룬다(`i18nReady`, index.js). 언어 소스는 `window.electron.getLanguage()`가 1순위, localStorage `i18nextLng`가 2순위다. 확장 i18n은 daemon `/api/extensions/:id/i18n/:locale`에서 받는다.
- **번역 누락**(en 기준 diff):
  - gui.json: de, es, fr, pt-BR, ru, zh-CN, zh-TW는 661개로 **16개가 빠져 있다**(`console.dock_focus/dock_restore/minimize/pin/popin/unpin`, `discord_modal.music_channel_*`, `discord_modal.music_normalize`). ja는 667개로 `discord_modal.music_*` 10개가 빠져 있다.
  - cli.json: qcza9f의 예전 파일은 ko를 제외한 8개 언어가 168개로 33개가 빠져 있다(`screen.bot_*` 등). PR #6의 새 파일은 10개 언어 모두 en 키가 빠짐없이 있다.
- 하드코딩된 한국어 문자열: 옛 Electron main.js의 트레이·알림·오류 매핑·로딩 문구는 Tauri로 옮기며 모두 `common.json` 키(`tray.*`, `updates.*`, `init.*`, `daemon.*`, `errors.*`, `port_conflict.*`)를 거친다(셸 `AppState::t` `lib.rs:40-42`, 브리지 `t` `createApi.js:9`). 영어로 남은 문구: `createApi.js:112`(401 안내), `native.rs:154`(`Installer not found`), `lifecycle.rs:601`·`native.rs:52-53`(데몬 상태 메시지), 팝아웃 창 제목 `Console — <이름>`(`window.rs:207`)

---

## 6. 불일치, 죽은 코드, stub, 버그 (file:line)

### 6.1 존재하지 않는 endpoint나 잘못된 HTTP 메서드 호출
(예전 TUI CLI의 항목들은 CLI 리메이크에서 코드째 사라져 뺐다. §3 참조.)
1. ~~`updater:setConfig`가 POST를 써서 405~~ 해결됨: 이제 PUT이다(옛 Electron main.js:2505, 지금은 `createApi.js:738`).
2. ~~CLI `update download`/`update apply`가 본문 없이 POST하는데 데몬은 JSON 본문을 요구했다~~ → 해결: PR #6 CLI는 `{components:[...]}`를 보낸다(`saba-chan-cli/src/client.rs:609-619`). 데몬도 본문이 없거나 공백이면 전체로 받고, 깨진 JSON은 400 `invalid_body`로 거부한다(`src/ipc/updates.rs:320-343, 447-452`).

### 6.2 죽은 코드와 stub
- ~~(옛 Electron main.js:501-503) `syncGuiConfigToDaemon`(no-op)과 이를 부르는 `guiConfig:sync` 채널~~ → 해결: 함수, 채널, preload 항목이 모두 없어졌고, Tauri 브리지에도 없다.
- ~~(옛 Electron main.js:1005-1011) `preloadLightData`~~ → 해결: 없어졌다. 셸의 `finish_background_init`이 로딩 상태 문구만 보낸다(`lifecycle.rs:93-116`).
- renderer가 쓰지 않는 window.api(§1.2).
- ~~(옛 Electron main.js:1223-1224) auto_download/auto_apply는 "향후 구현 예정"~~ → 해결: `auto_download`이면 데몬이 확인 때 미리 내려받아 둔다(`src/ipc/updates.rs:207-208, 227`). 동작하지 않던 `auto_apply`는 설정에서 빠졌고, 적용은 언제나 사용자가 고른다.

### 6.3 기능 버그
- ~~**before-quit bot 정지 요청 누락**: (옛 Electron main.js:1536, 1559) `currentPort` 범위 오류로 ReferenceError~~ → 해결: 비정상 종료 경로(`on_exit` `lifecycle.rs:656-678`)는 봇을 건드리지 않고 unregister와 자기 데몬 정리만 한다.
- ~~**SabaStorage bot 재시작 실패**: SabaStorage.js:316, 445가 `settings?.discordToken`을 읽음~~ → 해결: 로컬 봇이 없어져 SabaStorage는 봇을 재시작하지 않고, renderer에 `discordToken`을 읽는 곳이 없다.
- ~~**settings 경로 분기 기준 오류**: (옛 Electron main.js:483, 2860, 2967) `daemonProcess`로 판단~~ → 해결: 데몬이 응답하는지로 나누고, 응답하면 API로만 쓴다(`createApi.js:57-74, 747-774`, `native.rs:189-199`). 읽기 직접 접근은 남아 있다(§1.3).
- ~~**resolver 주석과 코드 불일치**: resolver.js:57(1분)과 60(5초)~~ → 해결: `resolver.js`는 로컬 봇(`discord_bot/`)과 함께 L1(7b5102f)에서 지워졌다.
- ~~**updater exe 이름 하드코딩**: (옛 Electron main.js:1113-1148), 예전 TUI `commands.rs:1137-1142`~~ → 해결: GUI와 CLI는 `POST /api/updates/launch-updater`만 보내고, 데몬이 `saba-chan-updater{EXE_SUFFIX}`를 찾아 띄운다(`src/ipc/updates.rs:662-669`).

---

## 7. ★ Daemon 우회 지점 전수 목록 (GUI · CLI)

> 기준: "인터페이스는 혼자서 아무것도 못 한다. 파일 편집도 daemon API를 거친다."
> 분류: **[FS-R]** 직접 파일 읽기 / **[FS-W]** 직접 파일·디렉터리 쓰기 / **[PROC]** 프로세스 spawn·kill / **[EXT]** 외부 서비스 직접 호출 / **[CFG]** 설정 파일 직접 편집 / **[OS]** OS 레지스트리·셸
> "불가피" 표시는 부트스트랩상 daemon 없이는 할 수 없는 항목이다. 그래도 목록에는 포함했다.

### 7.1 GUI Tauri 셸 (`saba-chan-gui/src-tauri/src`)
| # | 위치 | 분류 | 내용 | 비고 |
|---|---|---|---|---|
| G1 | daemon.rs:152-179 | FS-R | `.ipc_token` 읽기 | 불가피(인증 부트스트랩) |
| G2 | settings.rs:27-38, 81-94 | FS-R/CFG | `load_settings()`는 daemon 상태와 상관없이 **항상** config.json `gui`(깨졌으면 `.bak`, 없으면 예전 settings.json)를 직접 읽는다. 기동 시 포트·언어(`lib.rs:94-96`), `create_main_window`, 데몬 기동 env, `refresh_ipc_base`, 창 크기 저장, `language_get`, `settings_local_load`(브리지의 settings 폴백)가 이 함수를 쓴다 | ipcPort 부트스트랩만 불가피하고, 나머지는 API로 대체할 수 있다 |
| G3 | settings.rs:47-61, native.rs:189-227 | FS-W/CFG | `write_gui_section()`: 데몬이 **응답하지 않을 때만** config.json `gui`를 직접 쓴다(데몬과 같은 `ConfigFile` writer). 셸 `save_settings()`(windowBounds 저장 `window.rs:186`, `language_set`)와 브리지가 부르는 `settings_local_save`(`createApi.js:67-73`)가 이 경로를 탄다 | 부트스트랩 전용. 예전 `daemonProcess` 분기 버그는 해결(§6.3) |
| G4 | createApi.js:747-754 | FS-R/CFG | `settingsLoad`: API 실패 시 `settings_local_load`로 로컬 폴백 | 위와 같음 |
| G5 | logging.rs:10-45 | FS-W | 로그 디렉터리 생성과 로그 파일 쓰기 | GUI 자체 로그 |
| G6 | i18n.rs:50-62 | FS-R | `locales/<lang>/common.json` 직접 읽기(없으면 exe 내장본) | |
| G7 | lifecycle.rs:158-241, 375-422 | PROC | **saba-core spawn** (`--spawned`, env 주입). 콘솔 창 없이 분리해 띄운다(Windows `CREATE_NO_WINDOW`·`CREATE_NEW_PROCESS_GROUP`·`CREATE_BREAKAWAY_FROM_JOB`, job이 막으면 breakaway 없이 다시, Unix `process_group(0)`). 로그는 GUI가 사는 동안 파이프로 받는다. 인터페이스만 종료하면(`interface_only_quit` `:614-628`) 자식 핸들만 놓아 데몬이 남는다. GUI가 죽거나 강제 종료돼도 데몬은 남고, 다음 GUI 실행 때 `ensure_daemon`이 `/health`로 떠 있는 데몬을 찾아 다시 붙는다. 데몬을 끄는 것은 전체 종료(`clean_quit`, G8)뿐이다 | 불가피(daemon 부재 시 부트스트랩) |
| G8 | lifecycle.rs:289-335 | PROC | daemon 종료: `POST /api/daemon/shutdown` 후 15초 안에 안 끝나면 **자기 PID만** kill(`taskkill /PID /F`, SIGKILL) | API 우선, kill은 대비책 |
| G9 | — | — | ~~cleanQuit의 이름 기반 `taskkill /IM saba-core.exe`·`pkill -f saba-core`~~ → 해결: `clean_quit`(`lifecycle.rs:631-644`)은 unregister(shutdown=true) 후 `stop_daemon()`만 부르고, 이름 기반 kill은 없다(`:306-307` 주석) | |
| G10 | lifecycle.rs:576-602 | PROC | `daemon_restart`: `stop_daemon()`(API 우선) 후 `start_daemon()`으로 직접 spawn | 재기동 API가 없다 |
| G11 | tray.rs:447-459 | PROC | 트레이 메뉴: "데몬 종료"는 `stop_daemon()`, "데몬 시작"은 `start_daemon()` 직접 호출 | |
| G12 | lifecycle.rs:656-678 | PROC | `RunEvent::Exit`(`clean_quit`을 거치지 않은 경로)에서 자기 데몬을 `force_kill`. 인터페이스만 종료한 경우(`keep_daemon_alive`)는 건너뛴다 | 자기가 띄운 프로세스만 |
| G13 | — | — | ~~`node scripts/mock-release-server.js` spawn~~ 없어짐(f6ca7b6). 개발자가 직접 실행한다 | |
| G14 | — | — | ~~updater exe detached spawn(`--apply --relaunch`)~~ → 해결: GUI는 `POST /api/updates/launch-updater {relaunch}`만 보내고 업데이터는 데몬이 띄운다(`native.rs:81-102`, `src/ipc/updates.rs:601-659`) | |
| G15 | — | — | ~~updater exe 경로 탐색(`existsSync`)~~ → 해결: 데몬의 `updater_binary()`가 찾는다(`updates.rs:662-669`) | |
| G16 | native.rs:151-184 | OS/PROC | 레지스트리 `HKCU\…\Uninstall\Saba-chan` `InstallLocation` 읽기(`winreg`), installer `--uninstall` 분리 spawn | |
| G17 | — | — | ~~relay `PATCH /api/hosts/:hostId/bot-config` 직접 호출~~ 없어짐: 봇 설정은 `PUT /api/config/bot`, 릴레이 동기화는 에이전트 하트비트 | |
| G18 | native.rs:338-383, window.rs:63-96 | OS | `shell_open_external`·탐색 잠금(http(s)만 바깥 브라우저), `shell_open_path`·`logs_open_folder`(로그 폴더, 인스턴스 폴더) | UI 성격 |
| G19 | native.rs:300-334 | OS | 네이티브 파일/폴더/저장 다이얼로그(tauri-plugin-dialog) | UI 성격(경로 선택만 하고, 실제 스캔·내보내기는 daemon) |
| G20 | — | — | ~~`http.request` 전역 monkey-patch~~ → 해결(D12): 토큰은 데몬 클라이언트(`daemon.rs:182-192`)가 데몬 요청에만 붙인다 | |

### 7.2 GUI renderer (`saba-chan-gui/src`)
| # | 위치 | 분류 | 내용 |
|---|---|---|---|
| R1 | i18n.js:100-139, SettingsModal.js:249 | FS-W(localStorage) | 언어 캐시 `i18nextLng` |
| R2 | utils/themeManager.js:50-120 | FS-W(localStorage) | 테마와 커스텀 테마. config.json `gui`에도 accent 계열을 저장하므로 저장소가 둘로 갈린다 |
| R3 | Modals/NoticeModal.js:15-25 | FS-W(localStorage) | 공지와 알림 기록 |
| R4 | i18n.js:3-33 | 빌드타임 | 로케일 JSON 정적 번들. 런타임 locale 업데이트(daemon의 Locales 컴포넌트 자동 적용. 셸 알림에서는 빼기만 한다, `events.rs:144-147`)가 renderer에는 반영되지 않는다. 셸 문구는 설치 폴더 파일을 먼저 읽어 반영된다(`i18n.rs:50-58`) |
| R5 | components/ServerCard.js:419-421, App.js:494-501 | 규칙 중복 | managed 여부를 `module_settings.managed_start`와 모듈 `interaction_mode`로 다시 계산한다. 데몬은 `GET /api/servers`의 `managed`로 판정을 주는데(`handlers/server.rs:81`), 규칙이 다르다: 값이 없으면 데몬은 늘 managed(`supervisor/mod.rs:1479-1488`), renderer는 `console` 모드 모듈만 managed로 본다 |

(renderer는 `fetch`, `axios`, 직접 IP 접근을 하지 않는다. 모두 `window.api`(브리지 → `daemon_request` 명령)를 거친다.)

### 7.3 CLI (`saba-chan-cli/src`, PR #6 기준)
예전 TUI의 우회 지점 C3~C7, C11~C13(로컬 fallback, `cli-settings.json`, updater 직접 실행 등)과 이름 기반 kill은 리메이크에서 사라졌다. 남은 것은 부트스트랩뿐이다.

| # | 위치 | 분류 | 내용 | 비고 |
|---|---|---|---|---|
| C1 | client.rs:28-34 | FS-R | `.ipc_token` 읽기 (한 곳) | 불가피 |
| C2 | bootstrap.rs:14-46 | FS-R/CFG | `config.json`에서 `gui.ipcPort`·`gui.language` 읽기 | 불가피 |
| C8 | i18n.rs:36-45 | FS-R | 설치 폴더 `locales/<lang>/cli.json` 읽기 (없으면 내장본) | |
| C9 | bootstrap.rs:49-90 | PROC | saba-core 분리 실행 | 불가피(부트스트랩) |
| C10 | bootstrap.rs:92, cmd/daemon.rs:102 | PROC | API 종료가 안 될 때 데몬 PID만 강제 종료 | 이름 기반 kill 아님 |
| C14 | cmd/chatbot.rs:109-114 | FS-R | `chatbot set KEY=@<파일>`이면 사용자가 지정한 파일 내용을 값으로 읽어 API로 보낸다 | 사용자 입력 편의, 데몬 파일 아님 |
| C15 | cmd/server.rs:302-310 | FS-R | `server add`에서 폴더 중복 검사용 `canonicalize` | 경로 정규화만 |

### 7.4 디스코드 봇
로컬 봇이 없어져 이 목록(B1~B11)은 사라졌다. 릴레이와의 통신은 데몬의 relay_agent만 한다(§4).

### 7.5 요약 판정
- **원칙 위반 정도가 가장 큰 항목**:
  - G2: GUI가 daemon 상태와 상관없이 config.json `gui`를 직접 읽는다. 쓰기(G3, G4)는 daemon이 응답하지 않을 때만 하므로 부트스트랩 범위로 줄었다.
- **부트스트랩상 불가피한 항목**(토큰 파일, ipcPort, daemon 최초 spawn): G1, G7, C1, C2, C9. daemon **종료**(G8, G11)는 이제 `/api/daemon/shutdown`을 먼저 쓰고, 강제 종료는 자기가 띄운 PID에만 한다(G12도 같다). 이름 기반 kill(G9)은 없어졌다.
- **daemon에 API가 없어서 우회하는 항목**: daemon 재기동(G10), 언인스톨러(G16). updater 실행(G14)은 daemon API로 옮겨졌다.
