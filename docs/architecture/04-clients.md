# 04. 클라이언트 계층: GUI · CLI · Discord Bot (server-chan) · 공유 상수 · 로케일

> 이 문서는 코드에서 직접 도출했다. 기존 docs/README는 신뢰하지 않았고, 코드와 어긋나는 주석이나 문서는 각 항목에서 따로 표시했다.
> 기준 원칙(프로젝트 오너): **daemon(saba-core)이 서비스의 주인이며 모든 기능은 daemon을 거친다. 인터페이스(GUI/CLI)는 혼자서 아무것도 하면 안 되고, 파일 편집도 daemon API를 거쳐야 한다.** modules는 Python 게임 서버 매뉴얼, plugins(=extensions)는 Python으로 작성되어 인터페이스에 컴포넌트를 주입하고, server-chan(discord_bot)은 daemon과 짝을 이뤄 Discord 봇을 돌린다.
> 이 원칙을 어기는 지점은 모두 **§7 "daemon 우회 지점 전수 목록"**에 정리했다. 이 문서에서 가장 중요한 결과물이다.

Daemon 라우터의 기준 위치: `src/ipc/mod.rs:639-817`(`router()`, 인증 미들웨어 `auth::auth_middleware`는 `:816`), `src/ipc/updates.rs:106-119`(`/api/updates/*`, 인증 layer **이전**에 `.merge()`로 붙어 같이 인증된다, `src/ipc/mod.rs:812-816`).

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
| IPC 토큰 인증 | `daemon.rs:122-286` | 토큰 파일(`constants::token_file_path()`, `lib.rs:95`)을 **직접 읽어** 데몬 요청에만 `X-Saba-Token`을 붙인다. 감사 로그 출처 `X-Saba-Client: gui`도 붙인다(`builder` `:193-204`, A10). 리다이렉트를 따라가지 않고 프록시도 쓰지 않는다(`:139-145`). 401이면 토큰 파일을 다시 읽고 1회 재시도한다. 단 `/api/relay/` 요청은 재시도하지 않는다(`send` `:235-254`) |
| 렌더러 → 데몬 통로 | `native.rs:17-42`, `daemon.rs:93-118` | 렌더러는 `daemon_request` 명령으로만 데몬에 닿는다. 경로는 `/api/…`와 `/health`만 받고 `.`·`..` 조각(`%2e` 포함)·`\`·공백·`#`는 막는다. 경로에 넣는 이름은 브리지가 `encodeURIComponent`로 넣는다. 셸이 직접 정하는 `/api/updates/launch-updater`·`/api/client/*`·`/api/daemon/shutdown`은 렌더러에 열지 않는다(확장 GUI 스크립트도 같은 웹뷰에서 돈다). 동의 기록(`/api/sandbox/unisolated/*`·`/api/sandbox/extension-gui/*`의 GET·DELETE 밖 메서드)도 막고 `sandbox_consent_allow`만 보낸다(`is_allowed_request`). 막힌 경로는 `{status:0, code:'EINVAL'}`. 결과는 성공·HTTP 오류 모두 `{status, data}`, 연결 실패는 `{status:0, error, code}`(`ECONNREFUSED`·`ETIMEDOUT`·`EOTHER`). `responseType`은 `json`·`text`·`base64` |
| 명령 권한 | `capabilities/default.json`, `capabilities/tray-popup.json`, `build.rs` | 창 `main`과 `popout-*`에만 이 앱의 명령(`allow-*`)과 `core:default`를 연다. 트레이 팝업 창 `tray-popup`은 `daemon_request`·`daemon_status`·`language_get`·`tray_action`·`tray_popup_fit`만 부른다(알림 `notify`는 없다: 팝업은 열 때마다 새로 떠서 지난 포트 충돌 알림을 다시 받는다). 명령 목록은 `build.rs`의 `COMMANDS`와 `lib.rs:118-149` `generate_handler!`가 짝을 이룬다 |
| 콘텐츠 보안 정책 | `tauri.conf.json` `app.security.csp` | 스크립트는 앱 자신과 `blob:`(익스텐션 번들)만, 인라인 스크립트·이벤트 속성은 막는다. 스타일은 인라인 허용(+Google Fonts), 이미지는 `data:`/`blob:`/https, 연결은 Tauri IPC만. 개발 실행(`devCsp`)은 vite HMR 때문에 느슨하다 |
| **Daemon spawn** | `start_daemon()` `lifecycle.rs:158-218` | debug는 `target/release`(없으면 `target/debug`)의 `saba-core(.exe)`, 배포는 설치 루트의 것을 `--spawned`로 띄운다(`daemon_path` `:129-138`). cwd는 설치 루트, env는 `RUST_LOG=info, SABA_LANG, SABA_IPC_PORT, SABA_INSTANCES_PATH=<data>/instances`. 모듈 폴더는 넣지 않는다(데몬이 `config.json` `paths`로 정한다, 01 §1.5). stdout/stderr는 GUI 로그로 넘긴다(`pipe_log` `:247-265`) |
| 분리 실행 | `detach()` `lifecycle.rs:222-244`, `:188-200` | Windows는 `CREATE_NO_WINDOW`·`CREATE_NEW_PROCESS_GROUP`·`CREATE_BREAKAWAY_FROM_JOB`로 띄운다. GUI를 띄운 쪽의 job이 breakaway를 막아 spawn이 거부되면 breakaway 없이 다시 띄운다. 그때는 GUI 로그에 경고를 남긴다. `daemon_status`의 `in_parent_job`이 true면 상단 바가 경고한다(`ParentJobNotice`: 코어를 띄운 프로그램이 꺼지면 데몬도 꺼질 수 있으니 시작 메뉴에서 다시 열라는 안내). 값은 데몬 `/health`의 `stops_with_launcher`(CLI가 job 안에서 띄운 데몬도 포함)이고, 그 키가 없는 옛 데몬이면 이 GUI가 breakaway 없이 띄운 데몬이 살아 있는지다. Unix는 `process_group(0)` |
| Daemon 재사용/기동 판단 | `ensure_daemon()` `lifecycle.rs:389-436` | `GET /health`가 200이면 기존 daemon을 재사용한다. 응답의 `token`이 `valid`가 아니거나(A6) 예전 데몬이 401을 주면 토큰을 다시 읽으며 최대 3회 확인한다(`reload_token_until_valid` `:345-361`). ECONNREFUSED/ETIMEDOUT이면 `start_daemon()` 뒤 500ms 간격으로 최대 20회(10초) 폴링한다(`wait_for_new_daemon` `:370-386`) |
| Daemon 종료 | `stop_daemon()` `lifecycle.rs:311-338`, `force_kill()` `:292-307` | 먼저 `POST /api/daemon/shutdown`을 보내고 최대 40초 기다린다(데몬 종료 hook 한도 30초보다 길게). 그래도 안 끝나면 **이 GUI가 띄운 PID만** 강제 종료한다(Windows `taskkill /PID … /F`, `/T` 없음. Unix `Child::kill`(SIGKILL)). 이름 기반 kill은 하지 않는다(`:309-310` 주석) |
| 클라이언트 등록/heartbeat | `lifecycle.rs:454-522` | `POST /api/client/register {kind:'gui', pid, installRoot}`(인스톨러가 이 설치의 GUI를 찾는 데 쓴다), 이후 30초마다 `POST /api/client/:id/heartbeat`. 404나 연결 거부면 다시 등록하고, 처음 등록이 실패해 id가 없으면 주기마다 등록을 다시 시도한다. 종료 시 `DELETE /api/client/:id/unregister[?shutdown=true]` |
| install_root 동기화 | `sync_install_root()` `lifecycle.rs:439-450` | `PUT /api/updates/config {install_root}` |
| 백그라운드 초기화 | `app_ready()` `lifecycle.rs:64-75`, `:77-116` | 렌더러가 리스너를 단 뒤 `app_ready`를 부르면 처음 한 번 `ensure_daemon` → 등록·heartbeat·이벤트·업데이트 확인을 시작하고 `status:update` `ready`를 보낸다. 데몬이 뜨지 않으면 `daemon_failed`만 보내고 멈춘다. `--after-update`면 첫 ready 뒤 `updates:completed`. `--open-server=<id>`(CLI의 `gui`가 붙인다)로 뜨거나 이미 뜬 GUI에 단일 인스턴스로 그 인자가 오면 `app:openServer {instanceId}`를 보낸다. 렌더러가 `app_ready`를 부르기 전이면 맡아 뒀다가 그때 보낸다(`lifecycle.rs` `open_server`) |
| 이벤트 스트림 | `events.rs:75-116`, `lifecycle.rs:526-537` | `GET /api/events`(SSE) 연결 하나를 유지하고 받은 이벤트를 `events:message {id, type, data}`로 보낸다. 끊기면 3초 뒤 `Last-Event-ID`로 다시 붙고, 401이면 토큰을 다시 읽는다 |
| 업데이트 알림 | `lifecycle.rs:539-581`, `events.rs:123-160` | 확인 주기는 데몬이 정한다. 셸은 10분마다 데몬이 캐시한 `GET /api/updates/status`를 읽어, 새 업데이트 목록일 때만 OS 알림을 띄우고 `updates:available`을 보낸다. 확인에 성공했는데 받을 것이 없으면 `count:0`으로 보내 렌더러가 남은 업데이트 알림을 지운다(수동 확인 `updaterCheck`도 같다). Locales는 알림에서 뺀다. 적용은 사용자가 고른다 |
| **업데이트 적용 시작** | `updater_launch_apply` `native.rs:81-102` | `POST /api/updates/launch-updater {relaunch:[current_exe()]}`만 보내고 500ms 뒤 `clean_quit()`한다. 업데이터는 데몬이 띄운다(`src/ipc/updates.rs:609-675`, 실행 파일은 `saba-chan-updater{EXE_SUFFIX}`, `updater_binary` `:747-754`) |
| **동의 기록** | `sandbox_consent_allow` `native.rs` | 렌더러가 `{kind:'unisolated'|'extension-gui', component}`를 넘기면 셸이 네이티브 확인 창(문구 `common.json` `consent.*`)을 띄우고, 허락하면 `POST /api/sandbox/<kind>/<component>`를 보내 데몬 본문을 돌려준다. 취소는 `{success:false, error_code:'canceled'}`. 구성요소 이름은 영숫자·`-`·`_`·`.`만 받는다 |
| **Installer spawn(언인스톨)** | `launch_uninstaller` `native.rs:151-184` | 설치 루트·`saba-chan-gui` 하위·상위·exe 폴더에서 인스톨러를 찾고, 없으면 Windows 레지스트리 `HKCU\…\Uninstall\Saba-chan` `InstallLocation`을 본다(`winreg`, `:119-126`). `--uninstall`로 분리 실행(`DETACHED_PROCESS`) 뒤 `clean_quit()` |
| 모의 릴레이(개발용) | — | GUI는 띄우지 않는다. 개발자가 `node scripts/mock-release-server.js`를 직접 실행하거나 `scripts/build-core.ps1 -MockRelay`(모의 서명 키를 믿는 개발 빌드, 배포 금지)로 쓴다 |
| 디스코드 연결 | `createApi.js:809-832` | 셸에 디스코드 코드는 없다. 브리지가 데몬의 릴레이 에이전트를 `/api/relay/agent{,/start,/stop}`로 켜고 끈다(§4) |
| 콘솔 팝아웃 창 | `window.rs:192-280` | Tauri 창 라벨 `popout-<hex(instanceId)>`(`popout_label` `:21-24`)에 `index.html?console-popout=<id>&name=`을 붙여 같은 React 번들을 연다. 700×450(최소 400×250), 프레임 없음, 항상 위. 이미 열려 있으면 포커스한다. 열림·닫힘·되돌리기 요청은 메인 창에 `console:popoutOpened/Closed/popinRequest`로 알린다 |
| 메인 창 | `create_main_window()` `window.rs:100-148` | 라벨 `main`, 프레임 없음, 페이지 로드 뒤 표시, 배경 `#f0f0f2`. 크기는 설정 `windowBounds`(작업 영역보다 크지 않게, 최소 780×min(840, max(560, 작업 영역 높이))). 이동·크기 변경이 멈추면 500ms 디바운스로 저장한다(최대화·최소화 중 제외, `schedule_save_bounds` `:159-188`) |
| 탐색 잠금 | `lock_navigation()` `window.rs:70-96` | 모든 창에서 앱 주소(`tauri://localhost`, `tauri.localhost`, 개발 서버) 밖으로의 이동과 새 창을 막고, http(s) 링크만 바깥 브라우저로 연다 |
| 트레이 | `tray.rs`, 팝업 `window.rs` `toggle_tray_popup`·`fit_tray_popup`, 화면 `components/shell/TrayPopup.js` | 아이콘을 누르면(왼쪽·오른쪽) 아이콘 옆에 테두리 없는 투명 창 `tray-popup`(`index.html?tray-popup=1`, 너비 320)을 띄운다. 렌더러가 잰 높이(`tray_popup_fit`)로 작업 영역 안에서 아이콘 위(아이콘이 화면 아래쪽일 때)나 아래에 놓고 보인다(`popup_position`). 포커스를 잃거나 Esc·버튼을 누르면 창을 없앤다(포커스를 잃고 닫힌 뒤 300ms 안의 아이콘 클릭은 다시 열지 않는다). 더블클릭은 메인 창. 팝업: 데몬 상태, 서버 목록(`serverList`, 3초마다, 시작·정지는 `instanceStart`·`instanceStop`), 창 열기, CLI로 전환(`clientsLaunchable().cli`일 때), 데몬 켜기/끄기, 완전히 종료. 셸 동작은 `tray_action` 하나(`tray::action`): CLI로 전환은 `POST /api/clients/launch {kind:'cli'}`가 성공하면 인터페이스만 종료, 데몬 끄기는 `stop_daemon()`(API 우선)이고 상주 GUI가 데몬을 따라 끝나지 않게 표시한다(끄지 못했거나 데몬이 어디서든 다시 뜨면 표시를 지운다), 데몬 켜기는 `start_daemon()`을 직접 부른다. 리눅스(앱인디케이터)는 아이콘 클릭을 알리지 않아 같은 동작의 OS 메뉴를 단다(`linux_menu`). 아이콘은 테마별 favicon(Windows는 작업 표시줄 테마 `SystemUsesLightTheme`, 창이 없으면 테마 이벤트가 오지 않아 heartbeat마다 다시 본다). 팝업 크기·간격은 아이콘이 있는 모니터의 배율로 정한다 |
| 상주 GUI (`--tray`) | `lib.rs` `run`, `lifecycle.rs` `begin_init`·`interface_only_quit`, 데몬 `tray_host.rs` | 데몬이 화면 있는 세션에서 띄우고 죽으면 되살린다(01 §1). `--tray`로 뜨면 메인 창 없이 트레이만 들고 렌더러 없이 초기화(데몬 확인·등록·heartbeat·업데이트 알림)를 시작한다. 이미 떠 있는 GUI에 `--tray`가 오면(단일 인스턴스) 아무것도 하지 않는다. 마지막 창이 닫혀도 프로세스는 끝나지 않는다(창이 없을 때 온 `RunEvent::ExitRequested{code:None}`만 막는다. 창이 남은 채 온 OS 종료 요청은 막지 않는다). 사용자가 보는 창(메인·팝아웃, 트레이 팝업은 빼고)이 없는 동안 등록·heartbeat에 `resident:true`를 싣고, 메인 창이 열리거나 닫히면 heartbeat를 바로 한 번 보낸다(`report_resident`). 창이 없고 데몬이 연결을 거부하면(CLI의 완전히 종료 등) 트레이에서 직접 끈 경우가 아니면 함께 끝난다(`should_follow_daemon_out`) |
| 종료 플로우 | `window.rs:139-142`, `app_close_response` `native.rs:63-78`, `lifecycle.rs:630-712` | 메인 창 닫기를 막고 `app:closeRequest`를 보낸 뒤 `ack`(받음 확인)와 `exit-interface`/`quit`/`cancel`을 받는다. 받음 확인도 없이 5초가 지난 뒤 OS 닫기(Alt+F4·작업 표시줄)가 다시 오면 렌더러가 멈춘 것으로 보고 `exit-interface`처럼 끝낸다(`window.rs` `renderer_unresponsive`). 1분보다 오래된 물음이나 `app_ready` 전의 물음은 렌더러가 못 받은 것으로 보고 다시 묻는다. 브리지는 구독자가 없을 때 온 `app:closeRequest`를 첫 구독자에게 한 번 넘긴다(`events.js`). 창 안의 X는 렌더러 버튼이라 멈추면 눌리지 않으며, 그때는 트레이 팝업이나 다시 실행이 렌더러 없이 다룬다. `exit-interface`(인터페이스만 종료)는 창(메인·팝아웃)을 모두 없애 웹뷰를 내리고 트레이로 남는다. 등록·데몬은 그대로이고, 트레이나 다시 실행으로 메인 창을 새로 만든다(`show_main`). `quit`은 unregister(shutdown=true) → `stop_daemon()` 뒤 끝낸다. 이 경로를 거치지 않고 끝나면 `on_exit`가 자기가 띄운 데몬만 정리한다. 종료 요청(`app.exit`)이 웹뷰가 죽어 닿지 않으면 1초 뒤 `on_exit`를 직접 부르고 프로세스를 끝낸다(`exit_app`) |
| 단일 인스턴스 | `lib.rs` `tauri_plugin_single_instance` | GUI는 한 번에 하나만 뜬다. 두 번째 실행은 창을 새로 만들지 않고 떠 있는 메인 창을 앞으로 가져온 뒤 끝난다. 여러 GUI가 한 데몬을 함께 쓰다 한쪽의 "완전히 종료"가 다른 쪽의 데몬을 꺼뜨리던 것을 막는다 |
| 로그 | `logging.rs:10-45` | `<data>/logs/saba-chan-<ts>.log`와 stderr에 같이 쓴다. 패닉도 남긴다 |
| 셸 i18n | `i18n.rs:1-62` | 트레이·알림·상태 문구는 `<installRoot>/locales/<lang>/common.json`을 먼저 읽고, 없거나 깨졌으면 exe에 `include_str!`로 넣은 10개 언어 사본을 쓴다. 없는 키는 영어 → 키 그대로. 언어를 바꾸면 다시 읽고 트레이를 갱신한다(`language_set` `native.rs:248-257`) |

**인증 방식 요약**: 데몬이 시작할 때 UUID 토큰을 만들어 `<dataDir>/.ipc_token`(Unix 0600)에 쓰고(`src/ipc/auth.rs:26-60`), 모든 클라이언트는 이 파일을 읽어 `X-Saba-Token`을 보낸다. `Host`가 루프백이 아니면 403으로 막는다(`auth.rs:90-101, 114-117`). `SABA_AUTH_DISABLED=1`은 개발 빌드(`debug_assertions`)에서만 통하고, 토큰이 없으면 아무 요청도 통과시키지 않는다(`auth.rs:120-133`). 토큰은 바이트를 끝까지 비교하는 상수 시간 비교로 맞춘다(`auth.rs:102-105, 142`). `/api/updates/*`도 auth layer 안에 있어 인증 대상이다. `/health`만 토큰 없이 답하고, 보낸 토큰이 맞는지(`token: valid|invalid|missing`)를 알려 준다(A6).

### 1.2 window.api 전수표 (`src/api/createApi.js` → daemon endpoint)

`window.api.*`는 `createApi.js:117-1152`, `window.electron.*`는 `:1153-1167`에 있다. 표의 줄 번호는 작성 당시 기준이라 어긋날 수 있다(함수 이름으로 찾는다). 데몬 호출은 모두 `daemon_request` 명령(§1.1)을 거친다. 네이티브 작업은 Rust 명령 이름을 적었다(계약 §2, `native.rs`). "라우트 존재"는 `src/ipc/mod.rs`와 `updates.rs` 기준으로 대조했다(숫자는 줄). `tests/api_contract.rs`가 `createApi.js`를 읽어 GUI가 부르는 경로가 데몬 라우트에 있는지 검사한다(`:60-69`). "미사용"은 renderer(`src/`, 테스트 제외)가 부르지 않는다는 뜻이다.

| window.api 함수 (createApi.js 줄) | 호출 대상 | 라우트 존재 |
|---|---|---|
| serverList (90) | GET /api/servers?notices_after=<본 마지막 알림 id> (port_conflict_stops가 있으면 `notify` 명령으로 OS 알림. 받은 가장 큰 `id`를 기억해 다음 조회에 실어 같은 알림을 두 번 띄우지 않는다, L50). 서버마다 `managed`(runner가 맡는지, `Supervisor::uses_managed_start`)가 실린다(`ipc/mod.rs:185-187`, `handlers/server.rs:87`) | ✓ 643 |
| instanceStart (119) | POST /api/instance/:id/start (managed 여부와 포트 검사는 데몬이 정한다) | ✓ 646 |
| serverStop (139) | POST /api/server/:name/stop (70분 timeout, L74) | ✓ 650 |
| serverStatus (160) | GET /api/server/:name/status | ✓ 644 |
| moduleList (181) | GET /api/modules | ✓ 651 |
| moduleRefresh (192) | POST /api/modules/refresh | ✓ 652 |
| moduleGetMetadata (212) | GET /api/module/:name | ✓ 659 |
| moduleAudit (225) | GET /api/module/:name/audit (위험 능력과 `[permissions]` 선언 비교) | ✓ 660 |
| instanceEvents (233) | GET /api/instance/:id/events (최근 종료 이력) | ✓ 647 |
| moduleGetLocales (240) | GET /api/module/:name/locales | ✓ 712 |
| moduleListVersions (248) | GET /api/module/:name/versions?… | ✓ 695 |
| moduleInstallServer (260) | POST /api/module/:name/install. 설정 창의 버전 설치는 `instance_id`·`background:true`로 시작만 시키고 창을 닫는다(진행은 서버 목록의 설치 표시). 받은 실행 파일·작업 폴더 기록은 데몬이 한다 | ✓ 697 |
| instanceGetInstalledVersion (268) | GET /api/instance/:id/installed-version | ✓ 698 |
| makerInfo | GET /api/maker | ✓ |
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
| instanceUpdateSettings (408) | PATCH /api/instance/:id. 400이면 `{status, error, error_code, details, fields}`를 그대로 넘긴다(`invalid_format`이면 형식 오류 알림). 설정 창은 연 때의 `ServerInfo.revision`(그 뒤 실제 값 읽기·버전 감지·설치 저장이 준 `revision`이 있으면 그 값)을 싣고, 409 `settings_changed`면 다른 곳에서 바뀌었다고 알린 뒤 목록을 다시 읽는다 | ✓ 665 |
| instanceLaunchPreview (452) | POST /api/instance/:id/launch-preview `{settings, extra_args?}` (extra_args를 안 넘기면 저장된 것) | ✓ 666 |
| instanceResetServer (457) | POST /api/instance/:id/server/reset | ✓ 686 |
| executeCommand (465) | POST /api/instance/:id/command {command, args}. 명령 해석과 라우팅(REST·stdin·모듈)은 데몬이 한다(464 주석). GUI는 `…/rcon`, `…/rest`를 직접 부르지 않는다 | ✓ 673 |
| instanceStop/Restart/Rename/Commands (493-498) | POST …/stop·…/restart {force}(둘 다 70분 timeout: 데몬이 stop_timeout까지 기다린 뒤 답한다, L74), POST …/rename {name}, GET …/commands | ✓ 648, 649, 664, 674 |
| instanceValidate / instanceDiagnose | POST /api/instance/:id/validate · /diagnose `{}` (60초 timeout). 모듈 결과 `{success, issues:[{code, severity, message, solution}]}`를 그대로 돌려준다(서버 상세 `ServerCheck`) | ✓ 717, 724 |
| instanceReadLiveSettings (500) | GET /api/instance/:id/settings/live | ✓ 685 |
| runtimePreflight (501) | GET /api/runtime/preflight | ✓ 756 |
| systemMetrics (503) | GET /api/system/metrics[?sizes=false] | ✓ 715 (`SystemMonitor`) |
| configDefaults / configReset (505-507) | GET /api/config/:kind/defaults, POST /api/config/:kind/reset {keys?} | ✓ 704-705 (configDefaults 미사용) |
| extensionList…extensionInitStatus (510-630) | /api/extensions, /:id/enable, /disable, /audit, /i18n/:locale, /gui, /gui/styles(`responseType:'text'`), /icon(`responseType:'base64'`), /manifest, /:id/install, DELETE /:id, /updates, /rescan, /init-status | ✓ 754-771 (확장 UI 로딩은 03 §4.1). `gui_allowed:false`면 번들·스타일을 요청하지 않고 사용자에게 한 번 묻는다(`ExtensionContext`). `/gui`가 403 `extension_gui_unapproved`면 `extensionGuiBundle`은 `{error_code:'extension_gui_unapproved'}`를 돌려주고 다시 묻는다 |
| extensionConfigGet / extensionConfigSet | GET /api/extensions/:id/config(`{config}`) · PUT 같은 경로에 값 객체 그대로(키 단위 병합, null이면 삭제). 실패하면 데몬 본문을 그대로 돌려준다(설정 > 익스텐션 `ExtensionConfigForm`) | ✓ 807 |
| managedConsole (634) | GET /api/instance/:id/console?since&count (4번째 인자 `{spans:true}`면 `&spans=1`) | ✓ 679 |
| managedStdin (642) | POST /api/instance/:id/stdin | ✓ 680 |
| consolePopout / consolePopoutList (651-652) | Rust `console_popout` / `console_popout_list` (열린 팝아웃 창의 instanceId 배열). daemon 호출 없음 | n/a |
| daemonConsole (653) | GET /api/daemon/console?since&count. `{lines:[{id, timestamp, level, target, message}], count, epoch}` 그대로. 활동 줄의 데몬 콘솔(`ActivityBar.js` `DaemonConsole`)은 `epoch`가 바뀌면 받은 줄을 비우고 처음부터 다시 받는다(L55) | ✓ 752 |
| consoleFocusPopout / Popin / ToggleAlwaysOnTop (658-661) | Rust `console_focus_popout` / `console_popin` / `console_toggle_always_on_top` | n/a |
| trayAction / trayPopupFit | Rust `tray_action {action}`(async: 창 열기가 창을 새로 만든다)(`open_window`·`switch_to_cli`·`start_daemon`·`stop_daemon`·`quit_all`·`close`, 팝업을 닫고 `{ok}`) / `tray_popup_fit {height}`(트레이 팝업 크기·자리를 정하고 보인다). daemon 호출 없음 | n/a |
| updaterCheck (676) | POST /api/updates/check | ✓ updates.rs:109 |
| updaterStatus (696) | GET /api/updates/status | ✓ 100 |
| updaterDownload (697) | POST /api/updates/download `{components}` (빈 배열이면 전부) | ✓ 102 |
| updaterDownloadProgress (705) | GET /api/updates/download/progress | ✓ 103 |
| updaterApply (707) | POST /api/updates/apply `{components}` (빈 배열이면 전부) | ✓ 104 |
| updaterLaunchApply (725) | Rust `updater_launch_apply` → POST /api/updates/launch-updater {relaunch:[current_exe]}. 업데이터는 데몬이 띄운다 | ✓ 105 |
| updaterLastResult (727) | GET/DELETE /api/updates/last-result (GET은 `?lang=`, 언어는 `language_get`) | ✓ 106 |
| updaterGetConfig (737) | GET /api/updates/config | ✓ 108 |
| updaterSetConfig (738) | PUT /api/updates/config | ✓ 109 |
| settingsLoad (747) | GET /api/config/gui. 데몬이 응답하지 않을 때만 Rust `settings_local_load`(config.json `gui` 직접 읽기) | ✓ 702 |
| settingsSave (755) | 바꾼 키만 PUT /api/config/gui로 보낸다(데몬이 잠근 채 최상위 키로 합친다). 데몬이 **응답하지 않을 때만** Rust `settings_local_save`로 바꾼 키만 넘기고, 셸이 저장 직전 상태에 합쳐 config.json `gui`에 쓴다. 데몬이 오류 응답을 주면 저장 실패로 끝난다. 여러 번 부르면 차례로 처리한다. 연결 주소는 바꾸지 않는다: 바꾼 `ipcPort`는 셸이 데몬을 띄울 때(`lifecycle::start_daemon`) 적용한다 | ✓ 702 |
| settingsGetPath (775) | Rust `settings_get_path` (`<dataDir>/config.json`) | n/a |
| openFileDialog / openFolderDialog (778-779) | Rust `dialog_open_file` / `dialog_open_folder` | n/a |
| migrationScanDir (782) | POST /api/modules/detect (`{ok, matches}`, 파일 목록 없음) | ✓ 655 |
| discordBotStatus / Start / Stop (793-816) | GET /api/relay/agent, POST /api/relay/agent/start·stop. Status는 `running`·`stopped`, 다른 PC가 같은 계정으로 페어링해 멈췄으면(`lastErrorCode: token_replaced`) `replaced`(연결 화면에 다시 페어링 안내), 저장된 릴레이 주소가 거절돼 멈췄으면(`lastErrorCode: invalid_relay_url`) `invalid_relay_url`(연결 화면에 주소 형식 안내), 릴레이가 앱 버전을 거절하면(`lastErrorCode: agent_too_old`) `{status: 'too_old', running, minVersion}`(연결 화면에 `update_required.relay` 안내와 업데이트 화면을 여는 버튼, '실행 중'으로 보이지 않음). 노드 토큰이 없으면(`node_token_missing`) `cloud_token_not_found`, Start가 409 `invalid_relay_url`이면 `{error: 'invalid_relay_url'}`(`discord.invalid_relay_url` 문구) | ✓ 787-789 |
| botConfigLoad (817) | GET /api/config/bot. 못 읽으면 기본값 대신 예외를 던진다(호출한 쪽이 기본값을 저장해 설정을 지우지 않게) | ✓ 700 |
| botConfigSave (826) | PATCH /api/config/bot. 디스코드 저장은 마지막으로 저장한 값과 비교한 merge patch(`utils/mergePatch.js` `mergePatchDiff`, 지운 키는 `null`)만 보낸다. 칸마다 자동 저장하므로 `revision`은 싣지 않는다. 데몬 본문을 그대로 돌려준다: 권한 키라 저장하지 않은 경로는 `ignored`로 오고 화면이 알린다(CLI와 같다). 릴레이 동기화는 에이전트 하트비트가 맡는다 | ✓ 700 |
| saveNodeToken / loadNodeToken / deleteNodeToken (835-853) | PUT/GET/DELETE /api/config/node-token. DELETE는 데몬이 에이전트를 멈추고 토큰을 지운다 | ✓ 710 |
| auditList (855) | GET /api/audit?limit&instance (A10). `{entries}` 최신이 먼저 | ✓ 640 |
| permissionsGet (860) | GET /api/permissions/:guildId (P1) | ✓ 723-726 |
| permissionsUpdate (861) | PATCH /api/permissions/:guildId `{defaultLevel?, preset?, chatbotCap?, ownerDiscordId?, commandLevels?, ladder?}` | ✓ 723-726 |
| permissionsSetMember (862) | PUT /api/permissions/:guildId/members/:discordId `{level}` | ✓ 727-730 |
| permissionsRemoveMember (863) | DELETE /api/permissions/:guildId/members/:discordId | ✓ 727-730 |
| sandboxListUnisolated (864) | GET /api/sandbox/unisolated `{components:[{component, fingerprint, current}]}` | ✓ 737 |
| sandboxAllowUnisolated (865) | Rust `sandbox_consent_allow {kind:'unisolated'}` → 확인 창 → POST /api/sandbox/unisolated/:component. `sandbox_unavailable` 응답의 `component`를 넘긴다. 취소는 `error_code:'canceled'` | ✓ 738-741 |
| sandboxRevokeUnisolated (866) | DELETE /api/sandbox/unisolated/:component | ✓ 738-741 |
| sandboxListExtensionGui / sandboxAllowExtensionGui / sandboxRevokeExtensionGui | GET /api/sandbox/extension-gui, POST(Rust `sandbox_consent_allow {kind:'extension-gui'}` → 확인 창)·DELETE /api/sandbox/extension-gui/ext-<id>. 확인되지 않은 익스텐션의 GUI 번들 허락(묻는 창은 `ExtensionContext`, 목록·철회는 설정 > 고급 `ExtensionGuiSection`) | 데몬 #148 |
| actionsList (867) | GET /api/actions?surface=gui&instance?&locale? (P2) | ✓ 743 |
| actionsPlan (872) | POST /api/actions/:id/plan `{input, instance?}` | ✓ 746 |
| actionsRun (873) | POST /api/actions/:id/run `{input, instance?}` 또는 `{planId}` | ✓ 747 |
| instanceWords | GET /api/instance/:id/words (인스턴스별 부르는 말, 서버 상세의 디스코드 탭 `InstanceDiscordTab`이 부른다). `{id, name, module, words, commands:[…]}`를 그대로 돌려준다 | ✓ |
| instanceWordsSave | PUT /api/instance/:id/words `{words, commands}`. 400 `validation_failed {details:[{field, reason, word?, conflictWith?}]}`를 그대로 넘긴다. 성공하면 `{success, id, words, commands:{<명령>:[..]}}` | ✓ |
| guildAllowedInstancesSave | PUT /api/guilds/:guild/allowed-instances `{instances: [id] \| null}`(null = 제한 없음) → `{guildId, allowedInstances}`. 디스코드 화면의 인스턴스 탭이 바로 부른다(bot-config 일괄 저장으로는 바뀌지 않는다) | ✓ |
| discordSettingsGet (874) | GET /api/discord/:guild/settings (I2). `loaded:false`면 릴레이 값을 못 읽은 기본값이라 GUI는 편집을 막는다 | ✓ 732-735 |
| discordSettingsSet (875) | PATCH /api/discord/:guild/settings `{locale?, sleeping?, music?}` | ✓ 732-735 |
| clientsLaunchable / clientLaunch | GET /api/clients/launch → `{cli, gui}`, POST /api/clients/launch `{kind, instance?}`. 머리줄의 "CLI로 전환"(`shell/SwitchToCli.js`)은 `cli`가 true일 때만 보이고, 고른 서버(`activeServerId`)로 CLI를 띄운 뒤 성공하면 `closeResponse('exit-interface')`로 인터페이스만 끝낸다. 실패하면 알림만 하고 남는다. 예전 서버 머리의 "터미널로 열기"(openInTerminal)는 없앴다 | n/a |
| onDaemonEvent (882) | 셸의 `events:message`(§1.1 이벤트 스트림)를 받는다. `type: 'resync'`면 목록을 다시 읽는다. 돌려주는 함수로 해제 (P4) | ✓ 641 |
| dataExport / dataImport (885-907) | Rust `dialog_save_file` / `dialog_open_file`로 경로를 고른 뒤 POST /api/migration/export `{path, secrets}` · /api/migration/import `{path, dryRun, overwrite}`. 취소하면 `{canceled:true}`. 가져오기 결과에는 `path`를 붙여 미리보기(`dryRun`) 뒤 같은 파일로 다시 부를 수 있다 | ✓ 707-708 (A9) |
| relayCheckHostStatus… relayBotInvite (910-949) | GET /api/relay/host/:id/status, /nodes, GET /api/relay/node/:guild/members?withStatus=1(`{members, partial, reason?}`, 20초 제한), GET /api/relay/bot-invite | ✓ 774-776, 781 |
| relayListLegacyMembers | GET /api/relay/node/:guild/legacy-members(릴레이에 남은 부여값 `[{id, username, displayName, level}]`, 권한 가져오기 후보). 실패하면 오류 본문 | ✓ |
| relayNodeChannels / relayNodeChannelsSave (950-953) | GET/PUT /api/relay/node/:guild/channels(`notify` 알림 채널 포함). 실패하면 데몬 본문(`{error, channelId?}`)을 그대로 돌려준다 | ✓ 777-780 |
| instanceAgreements (956) | POST /api/instance/:id/agreements `{accept:[id]}` (모듈 약관 동의) | ✓ 687 |
| onboarding (957) | GET /api/onboarding (J1-a 다음 할 일) | ✓ 639 |
| daemonDiagnostics | GET /api/diagnostics(버전·pid·데이터 폴더·토큰 파일 유무·최근 경고). 실패하면 `{success:false, error}`(설정 > 고급 `DiagnosticsPanel`) | ✓ 674 |
| instanceBackups…instanceScheduleSave (958-970) | GET/POST …/backups(`instanceBackupCreate(id, body?)`: `{source:"snapshot"}`이면 게임 자동 백업 보관), DELETE …/backups/:name, POST …/backups/:name/restore(만들기·복원은 10분 timeout), GET/PUT …/schedule | ✓ 690-693 |
| chatbotGet/Save/Clear/ClearMemory (971-975) | GET/PUT/DELETE /api/chatbot, DELETE /api/chatbot/memory?guildId | ✓ 793-794 (chatbotClear 미사용) |
| chatbotModels | GET /api/relay/node/:guildId/chatbot/models (릴레이 답 그대로: `{models, selected, effectiveModel}`, 409 NO_API_KEY·422·503) | ✓ 챗봇 화면 모델 드롭다운. 고른 모델을 대신 쓰는 중이면(`effectiveModel`≠`model`) 알리고 그 모델을 고른 상태로 둔다 |
| relayInitiatePairing / relayPollPairingStatus (976-990) | POST /api/relay/pair/initiate, GET /api/relay/pair/:code/status | ✓ 782-783 |
| relayLinkRequests / relayLinkRequestRespond (992-994) | GET /api/relay/link-requests, POST /api/relay/link-requests/:id `{accept}`. 디스코드에서 코드 없이 온 연결 요청(J1-b). 데몬 본문을 그대로 돌려준다(`daemonJson`) | ✓ 784-785 |
| shellOpenPath (997) | Rust `shell_open_path`. 디렉터리가 아니면 열지 않는다(`errors.path_not_folder`) | n/a |
| logsGetPath / logsOpenFolder (1001-1002) | Rust `logs_get_path` / `logs_open_folder` | n/a (logsGetPath 미사용) |
| launchUninstaller (1003) | Rust `launch_uninstaller` (installer 직접 spawn) | n/a |
| getAppVersion (1004) | Rust `app_get_version` (패키지 버전) | n/a |
| systemPaths / systemPathsMove | GET /api/system/paths · POST /api/system/paths/move `{kind, target}`(시간 제한 30분). 설정 > 고급의 폴더 위치(`FolderPathsSection.js`)가 쓴다. 폴더 고르기는 `openFolderDialog`, 실패 문구는 `error_code`(`paths_*`)로 `gui.json` `folder_paths.err_*`에서 고른다 | ✓ (01 §2.2) |
| getComponentInfo (1005) | GET /api/system/components (실패하면 `app_get_version`만) | ✓ 714 |
| onOpenServer | 셸의 `app:openServer {instanceId}`(다른 인터페이스가 고른 서버)를 받는다. App은 `revealServer(instanceId)`로 그 서버를 연다(좁은 한 열 화면에서도 목록 대신 상세). 구독자가 없을 때 온 요청은 첫 구독자에게 한 번 넘긴다(`events.js`) | n/a |
| onCloseRequest / closeResponse (1021-1025) | 셸의 `app:closeRequest`를 받고, Rust `app_close_response`로 답한다(기다리지 않음). App은 받자마자 `ack`를 보낸다. 돌려주는 함수로 그 구독만 해제 | n/a |
| onStatusUpdate (1026) | 셸의 `status:update`. 브리지가 마지막 값을 기억해 늦게 붙은 콜백에도 준다(`events.js:137-177`). 돌려주는 함수로 그 구독만 해제 (채널 전체를 떼는 `off*`는 없다) | n/a |
| daemonStatus (1030) | Rust `daemon_status` → GET /health. 응답의 `token`이 `valid`(필드가 없으면 예전 데몬이라 유효로 본다)일 때만 `running: true`. 어긋나면 토큰 파일을 한 번 다시 읽어 재시도하고, 그래도 어긋나면 `running: false, error_code: "token_invalid"`. 실행 중이면 `in_parent_job`(데몬을 띄운 프로그램과 함께 끝날 수 있는가: `/health` `stops_with_launcher`, 키가 없는 옛 데몬이면 이 GUI가 job breakaway 없이 띄운 데몬이 아직 살아 있는가)도 준다 | ✓ 637 |
| daemonRestart (1031) | Rust `daemon_restart`: `stop_daemon()`(API 종료, 안 되면 자기 PID만 kill) 후 `/health`가 멎을 때까지 기다렸다가 **`start_daemon()`으로 직접 spawn**(`lifecycle.rs:592-618`) | 부분 |
| minimize/maximize/closeWindow (1034-1036, electron 1040-1042) | Rust `window_minimize` / `window_maximize`(토글) / `window_close`. 호출한 창에 적용한다 | n/a |
| electron.getLanguage/setLanguage (1043-1044) | Rust `language_get`(config.json `gui.language`를 직접 읽는다) / `language_set`(`save_settings`로 API 우선 저장, 셸 번역·트레이 갱신. 데몬은 재시작하지 않는다, `native.rs:241-257`) | 부분 |
| electron.shell.openExternal (1046) | Rust `shell_open_external` (http(s)만) | n/a |

셸에서 renderer로 가는 push 이벤트는 Tauri 이벤트로, 메인 창(라벨 `main`)에만 보낸다(`emit_main` `lib.rs:60-64`). 팝아웃·트레이 팝업 창의 브리지는 구독하지 않고 `app_ready`도 부르지 않는다(`install.js`). 브리지는 모듈을 읽을 때 9개 채널을 한 번 구독한 뒤(`events.js:5-14, 63-65`) `app_ready`를 부른다(`install.js:23-32`). 렌더러를 새로고침하면 `app_ready`가 다시 와서 셸이 마지막 `status:update`를 다시 보낸다(`lifecycle.rs:66-71`). 채널: `status:update`(로딩 단계 `{step, message, ts}`. `message`는 `common.json`의 `daemon.*`·`init.*`로 번역된다. 데몬이 10초 안에 응답하지 않으면 `step:'daemon_failed'`를 보내고 `ready`는 보내지 않는다. 그 뒤 `daemon_restart`가 성공하면 멈춘 초기화를 이어서 끝내고 `ready`를 보낸다. 실패하면 `{success:false, error}`), `events:message`, `updates:available`, `updates:completed`, `console:popoutOpened/Closed/popinRequest`, `app:closeRequest`, `app:openServer`(`{instanceId}`).

**renderer(src/, 테스트 제외)에서 쓰지 않는 window.api**: `configDefaults`, `chatbotClear`, `logsGetPath`, `loadNodeToken`(2026-10-22 renderer 대조). 나머지 표 항목은 모두 화면에서 부른다. 반대로 **GUI가 호출하지 않는 daemon 라우트**: `/api/server/:name/start`, GET `/api/instances`, GET `/api/instance/:id`, `/api/instance/:id/rcon`, `/rest`, `/managed/start`, `/validate`, `/properties`(GET/PUT), `/properties/reset`, `/diagnose`, `/api/module/:name/version/:version`, `/api/version`, `/api/modules/schema`, `/api/config/client/:kind`, `/api/extensions/:id/mount|unmount|config`, `/api/chat/command`, `/api/updates/integrity`. 이 중 일부는 CLI만 사용한다(§3).

### 1.3 GUI가 읽고 쓰는 설정 파일

모든 사용자 설정은 `<dataDir>/config.json` 하나에 네임스페이스별로 들어간다(`updater/src/config_file.rs:1-27`). 예전 개별 파일(`settings.json`, `bot-config.json` 등)은 데몬이 한 번 이관하고, 원본은 남긴 채 `.migrated` 사본을 만든다(`config_file.rs:37-45`).

| 파일 | 경로 | 접근 방식 | 스키마 |
|---|---|---|---|
| config.json `gui` | `<dataDir>/config.json` (`settings.rs:12-14`) | 셸 `load_settings()`(`settings.rs:71-84`, 명령 `settings_local_load`)는 **항상 파일을 직접 읽는다**(깨졌으면 `.bak`, `gui`가 아직 없으면 예전 `settings.json`, `:28-39`). 저장은 브리지 `saveSettings()`(`createApi.js:57-74`, `PATCH`)와 셸 `save_settings()`(언어·창 크기, `PUT`)가 바꾼 키만 `/api/config/gui`로 보내고(데몬이 잠근 채 합친다), 데몬이 **응답하지 않을 때만** 지금 로컬 설정에 합쳐 `write_gui_section()`(명령 `settings_local_save`)으로 `gui` 키만 바꾼다. 데몬과 같은 writer(`updater/src/config_file.rs` `ConfigFile::set_section`: tmp→fsync→rename, 직전 정상본은 `.bak`, 깨진 파일은 `.corrupt-<시각>`)를 쓴다 | 기본값(`settings.rs:53-68`): `autoRefresh, refreshInterval(2000), windowBounds{width,height,x,y}, language, ipcPort(57474), consoleBufferSize(2000), autoGeneratePasswords, portConflictCheck`. renderer가 추가로 저장하는 키(useSettingsStore.js:120-140): `accentColor, accentSecondary, useGradient, fontScale, enableTransitions, consoleSyntaxHighlight, consoleBgColor, consoleTextColor, sidebarCompact, consoleFontScale` |
| config.json `relay` | daemon이 관리(`/api/config/bot`, `src/config_store/mod.rs:293, 393`) | GUI는 API만 사용 | `prefixes[], cloud{relayUrl,hostId}, translateCommands(기본 true, 슬래시 명령에 모듈 명령의 번역한 이름·설명. GUI 디스코드 > 봇 설정, CLI `bot config translateCommands=false`), moduleAliases·commandAliases·instanceAliases(예전 전역 별명, 시작할 때 nodeSettings[guild].instances로 옮겨져 빈다), nodeSettings, cloudNodes[], cloudMembers{}`. 로컬 봇 필드(`token`, `autoStart`, `mode`, `music*`)는 없다(01 §4 BotConfig) |
| config.json `chatbot` | daemon이 관리(`/api/chatbot`) | API | 응답에는 API 키 대신 `hasApiKey`만 온다(`src/ipc/handlers/chatbot.rs:1-15`) |
| node token | `/api/config/node-token` | API | `{token: "sbn_<nodeId>.<secret>"}` |
| .ipc_token | `<dataDir>/.ipc_token` 또는 `SABA_TOKEN_PATH` | 직접 읽기(필수 부트스트랩) | 평문 UUID |
| logs | `<dataDir>/logs/*.log` | 직접 쓰기 | — |
| renderer localStorage | WebView 저장소(Windows WebView2) | 직접 | `i18nextLng`(i18n.js `getInitialLanguage`·`getCachedLanguage`, SettingsModal.js:268), 테마 키(themeManager.js:51-121), 공지 목록(NoticeModal.js:18-28) |

**쓰기 분기(해결됨).** 예전에는 `settings:load/save`와 `saveSettings`가 "이 GUI가 daemon을 spawn했는가"(`daemonProcess !== null`)로 분기해서, 이미 떠 있던 daemon에 붙으면 settings.json을 직접 썼다. 지금은 "daemon이 응답하는가"로 나눈다. 데몬이 응답하면 API로만 쓰고, 오류 응답이면 로컬로 넘어가지 않고 실패로 끝낸다(`createApi.js:57-74, 763-790`, `native.rs:189-199`). Tauri로 옮기면서도 이 규칙은 그대로다. 남은 직접 접근은 **읽기**다. `load_settings()`는 데몬 상태와 상관없이 파일을 읽고, `language_get`, `create_main_window`, `refresh_ipc_base`, 데몬 기동 env, 창 크기 저장(`window.rs:159-188`, 파일에서 읽어 merge한 뒤 API로 저장)이 이 값을 쓴다.

### 1.4 화면과 모달별 기능 (`src/`)

- **App.js**(1057줄): 루트 컴포넌트. `ExtensionProvider`로 감싸고, `useServerActions`가 `refreshInterval` 주기로 `serverList`를 폴링한다. 종료 질문 모달, 로딩 화면(`status:update`), Discord 패널(창이 넓으면 side-panel, 60)과 릴레이 에이전트 시작/정지(`useDiscordStore`, 157-158)를 맡는다. 봇 자동 시작과 `bot:relaunch` 처리는 로컬 봇과 함께 없어졌다.
- **ServerCard.js**: 인스턴스 카드. 상태, 시작/정지, 콘솔, 설정, 명령 버튼과 REST host:port 표시가 있다. 게임 서버 업데이트 확인/적용(`instanceCheckUpdate/ApplyUpdate` 45-72), 네이티브 프로비저닝 진행률(`instanceProvisionProgress` 100, `NativeProvision.js`: 설치 중엔 요리하는 사바쨩과 진행 막대, 성공하면 접시에 서버 아이콘을 올린 완성 카드(`cooked`, 사용자가 닫을 때까지), 실패하면 쓰러진 사바쨩), 팝아웃 콘솔 포커스(442)를 제공한다. `useDragReorder`로 카드를 드래그해 순서를 바꾸면 `instanceReorder`가 호출된다.
- **ConsoleView / ConsoleDock / ConsoleWindow**(useMultiConsole): managed 프로세스 콘솔. `managedConsole` 폴링, `managedStdin`, `executeCommand`(stdin이 실패할 때 폴백)를 쓰고, PiP 팝아웃과 always-on-top을 지원한다.
- **AddInstanceModal**: 진입점이 둘이다.
  - `AddInstanceNewServer.js`: 게임 모듈 카드 그리드에서 모듈을 고른 뒤 이름과 옵션을 입력하고 `instanceCreate`를 호출한다. 모듈 설정 중 `install = true`인 select(예: 마인크래프트 서버 종류)를 함께 묻고 `module_settings`로 보낸다. 마이그레이션 생성 응답에 `detected`가 있으면 완료 안내에 서버 종류와 플러그인·모드를 덧붙인다(`useServerActions.js` `handleAddServer`). 명령 칩은 `group`(플러그인 이름)이 있으면 표시한다(`CommandModal.js`). CLI `server add`도 `install` 설정을 `--set`으로 받거나 묻고 생성 요청에 싣는다(`saba-chan-cli/src/cmd/server.rs`). 모듈에 `[[presets]]`가 있으면 `server add --preset <id>`로 고르거나 새 서버일 때 묻고 `preset`으로 보낸다.
  - `AddInstanceMigration.js`: 폴더 선택 → `migrationScanDir`(daemon `/api/modules/detect`, 파일 목록은 주지 않는다) → 모듈 자동 감지(`dir_signatures`). 실패하면 `moduleManifest`에서 수동으로 고른다.
- **ServerSettingsModal.js + useServerSettings.js**: 모듈 `settings.fields` 스키마로 필드를 렌더링하고(`instanceUpdateSettings`), 버전 선택과 설치(`moduleListVersions`, `moduleInstallServer`, `instanceGetInstalledVersion`), 서버 리셋(`instanceResetServer`)을 제공한다. 인스턴스별 bot alias와 권한 편집도 여기서 하는데, bot-config을 직접 load/save한다(515-592).
- **CommandModal.js**: 모듈 `commands.fields` 기반 명령 힌트, 자동완성, 입력 폼. `executeCommand`(82)로 실행한다.
- **SettingsModal.js**: 탭은 `general / appearance / extensions / advanced`(335-353)이다. 버전과 컴포넌트 정보(`getComponentInfo`), 언어, 테마, IPC 포트, 언인스톨(`launchUninstaller`)이 있고, SabaStorage로 들어가는 진입점이 있다.
- **SabaStorage.js**(구 업데이트 센터, 1197줄): 탭은 `components / modules / extensions`(53-70)이다.
  - components 탭: updater check/download/progress/apply/launchApply.
  - modules 탭: 설치된 모듈과 `saba-chan-modules` 레지스트리(`moduleManifest`, `moduleInstallFromManifest`, `moduleRemove`). 맨 아래 `ModuleMakerCard`("직접 만들기")는 `makerInfo`가 준 저장소로 에이전트별 설치 줄과 첫 부탁을 보여 준다. 서버 추가 창의 게임 고르기에도 같은 카드가 있다.
  - extensions 탭: 확장 레지스트리.
  - devMode의 Mock 릴리스 서버 토글(`handleToggleMock`): qcza9f의 옛 renderer는 L3에서 없어진 `api_base_url`/`github_*`를 보내 효과가 없다. GUI 리메이크(PR #5, fd26f4e)에서 고쳤다: `relay_url`만 보내고, `mock_relay`로 빌드한 개발판에서만 보인다.
- **DiscordBotModal.js**(1134줄): 클라우드 릴레이 전용이다(local 모드, 봇 토큰, 음악 설정은 없어졌다). prefix, 노드(길드)별 `allowedInstances`와 멤버별 차단 명령(`memberPermissions`, 멤버 목록은 `relayListNodeMembers`, 125)을 다룬다. 페어링은 `relayInitiatePairing`(351)으로 시작해 `relayPollPairingStatus`(364)로 진행 상황을 확인하고 `saveNodeToken`(373)으로 마무리한다. 기본 relay는 `https://saba-chan.online`(9)이다. side-panel 모드가 있다(845-848). 명령 별명은 ServerSettingsModal에서 편집한다.
- **BackgroundModal.js**: daemon 상태(`daemonStatus`)와 재시작(`daemonRestart`).
- **NoticeModal.js**: 앱 내 공지와 알림 목록(localStorage). 업데이트 모달로 연결된다.
- 공용 모달(Modals.js): Success/Failure/Notification/Question. TitleBar(frameless 창 컨트롤), Toast, CustomDropdown, Icon, ExtensionSlot(확장 UI 주입 지점. 세부 내용은 다른 문서).
- 상태 저장소(zustand): `useServerStore`, `useSettingsStore`(설정 변경 후 500ms debounce로 자동 저장, 154-163), `useDiscordStore`(에이전트 상태, 봇 설정, 시작/정지), `useUIStore`(모달 표시).
- 테스트(`src/test/`): gui-e2e, settings-roundtrip, store-unit, discord-side-panel, console-window, theme-contrast. 모두 `window.api`를 mock한다(setupTests.js). `tauri-bridge.test.js`는 가짜 `window.__TAURI__`로 브리지 설치·응답 가공·이벤트를 검사한다.


### 1.5 사바쨩 웹 (`saba-chan-web`, React + Vite)

PC 주인이 폰 브라우저로 서버·인스턴스를 관리하는 원격 화면이다. 디스코드 `/사바쨩 웹`이 준 일회용 링크(`https://<릴레이>/remote/#<토큰>`)로 열고, 릴레이 웹소켓 `/remote/ws`로 데몬에 요청을 보낸다. 릴레이 쪽 계약은 05 §4.4(f), 데몬 쪽 문지기는 01 §1.4.1.

- 배포: server-chan `deploy/deploy-remote.ps1`이 이 디렉터리를 `npx vite build`해 `dist/`를 서버의 `remote-web`으로 올리고, nginx가 `/remote/`로 낸다(`base: '/remote/'`). 이 디렉터리나 함께 빌드되는 `saba-chan-gui/src`·`public`, `locales`에 커밋하지 않은 변경이 있으면 배포를 멈추고, 빌드한 커밋은 배포 복구 자료의 `sha`에 남는다.
- 세션(`src/session.js`): `hello{link}` → `ready{sid,key,expiresAt,hostId}`. 끊기면 25초 안에 `resume{sid,key}`로 다시 붙고(릴레이 유예 30초), 페이지를 떠나면(`pagehide`) `bye`. 요청은 `req{id,method,path,body}` → `res{id,status,body}`, 130초가 지나면 `remote_timeout`으로 끝낸다. 연장은 `extend`.
- 브리지(`src/api.js`): GUI `createApi({invoke, hub}).api`를 그대로 쓰고 `invoke('daemon_request')`만 세션 요청으로 바꾼다. 다른 Tauri 명령(창·파일 대화상자·프로세스)은 `not_supported`로 막는다. 응답 가공과 오류 문구는 GUI와 같다. 릴레이 `event{kind,data}`는 `onDaemonEvent`로 `{type,data}`가 되어 온다.
- GUI에서 가져오는 것은 `createApi`, `events`, `serverView`, `localizedText`, `commandRisk`, 권한 단계 계산 `components/discord/levels`, `tokens.css`, 아이콘 `components/Icon`, 드롭다운 `components/CustomDropdown`(웹 `components/Dropdown.jsx`가 감싼다)뿐이다(`src/test/guiImports.test.js`). 화면은 따로 만든다. GUI에 없는 아이콘만 `src/components/icons.jsx`에 둔다. 로고·사바쨩 그림은 GUI `public/`의 파일을 `@art` 별칭으로 가져온다(`src/art.js`의 목록만 빌드에 들어간다).
- 주기 읽기(서버 목록 4초, 콘솔 1.5초, PC 사용량·데몬 로그 3초, 설치 진행 2초)는 `components/poll.js` `startPoll`로 한다: 앞 요청이 끝난 뒤에 다음을 잡고, 탭이 숨어 있으면 쉬었다가 보이면 바로 읽는다.
- 화면: 서버(목록·추가·설치 진행과 실패(`provision-progress`, 실패를 닫으면 데몬이 새로 만들던 서버를 지운다)·시작/정지/재시작·EULA 동의·콘솔·설정·백업·삭제), 권한(`relayListHostNodes(hostId)` → 길드별 `permissionsGet`, 기본 레벨·멤버 레벨과 단계 사다리 편집(위·아래 버튼), 멤버는 실제 레벨 높은 순(사다리 순서)·같은 레벨은 이름순, 디스코드 프로필 사진), 모듈(공식 매니페스트 설치, 플러그인 켜기/끄기/설치), 더보기(PC 사용량, 데몬 로그). 설정의 경로·파일·비밀번호 칸은 PC에서만 바꾼다(열쇠 아이콘, 비밀번호는 데몬이 응답에서 뺀다). 확인 창은 GUI와 같은 경우에만 띄운다.
- 문구: `locales/<lang>/web.json`(`web` ns)과 `common.json`(`common` ns, 브리지 오류 문구). 10개 언어 키·자리 표시자 일치는 `src/test/locales.test.js`가 검사한다.

---

## 2. 공유 상수와 포트 (`updater/src/constants.rs`, `shared/constants.js`)

`shared/constants.js`를 import하는 코드는 이제 없다. 옛 Electron main.js만 `require`했고, 지금은 주석으로만 언급된다(`saba-chan-gui/src/stores/useSettingsStore.js:4`, `updater/gui/src/updater.js:9`). Tauri 셸과 CLI는 Rust 쪽 SSOT `saba_chan_updater_lib::constants`(`updater/src/constants.rs`)를 쓴다.

| 상수 | 값 | 사용처 |
|---|---|---|
| APP_NAME | `saba-chan` | dataDir 이름(`constants.rs:22`) |
| DEFAULT_IPC_PORT | `57474` | `constants.rs:32`. 셸 `settings.rs:8, 98-105`, CLI. renderer는 사본을 둔다(`useSettingsStore.js:4`) |
| DEFAULT_DAEMON_URL | `http://127.0.0.1:57474` | shared/constants.js에만 있고 쓰는 곳이 없다 |
| SUPPORTED_LANGUAGES | en, ko, ja, zh-CN, zh-TW, es, pt-BR, ru, de, fr | `constants.rs:42`. 셸은 OS 로캘을 이 목록에 맞춘다(`settings.rs:127-150`). **중복 정의 위치**: `saba-chan-gui/src/i18n.js:9`, `updater/gui/src/updater.js:10` |
| resolve_data_dir() (JS `getSabaDataDir()`) | `SABA_DATA_DIR`, Windows는 `%APPDATA%/saba-chan`, 그 외는 `$HOME/.config/saba-chan` | `constants.rs:55`. 셸 `lib.rs:79` |

- 그 밖의 포트:
  - 모의 릴레이 `scripts/mock-release-server.js` 9876 (개발용, GUI가 띄우지 않음)
  - Vite dev 5173 (`vite.config.js:24`, `tauri.conf.json` `devUrl`, 탐색 잠금은 이 출처를 앱 주소로 본다 `window.rs:74-78`)
  - Relay 기본 URL `https://saba-chan.online` (renderer `components/discord/DiscordPage.js:21`, 데몬 `src/ipc/handlers/relay.rs:41`). Tauri 셸에는 없다

---

## 3. CLI (saba-chan-cli, Rust)

> 이 절은 CLI 리메이크(PR #6, 브랜치 `claude/cli-remake-b1wszk`) 기준이다. 예전 TUI(`tui/`, `process.rs`, `config.rs`, `gui_config.rs`, `cli_config.rs`, `module_registry.rs`)는 이 리메이크에서 모두 지워졌다.

### 3.1 쓰는 방법 세 가지
- **한 번 실행하는 하위 명령**: clap 기반(`cli.rs`). `status`, `start|stop|restart <서버>`, `console`, `run <서버> <게임 명령>`, `server …`(list, info, add, remove, commands, send, settings, set, properties, rest, update, history, validate, diagnose, agree, reset, rename, reorder 등), `daemon …`, `module …`, `extension …`, `bot …`, `backup …`, `schedule …`, `chatbot …`, `update …`, `config …`, `chat <문장>`, `monitor [--watch] [--interval 초]`, `version`, `shell`, `tui [--server <서버>]`, `gui [서버]`(GUI로 전환: `POST /api/clients/launch {kind:'gui'}`로 그 서버를 연 GUI를 띄운다), `completions <셸>` (`cli.rs:41-138`). 콘솔에 한 줄을 그대로 보내는 명령은 `server send`다(`rcon` 하위 명령은 없다). `GET /api/servers`의 `port_conflict_stops`(데몬에 10분 남는 알림)는 `status`만 `?notices_after=<이 실행에서 본 마지막 id>`로 받아 보여 준다. 새 실행은 0부터라 남은 알림을 언제 일인지와 함께 다시 보이고, 셸 안에서 다시 부르면 본 것 뒤만 보인다(L50). 서버 목록·상태 대기 같은 나머지 호출은 `?notices=keep`이다(`client.rs` `servers`). `schedule`의 추가·고치기·지우기·켜기·끄기·보관 수는 읽을 때 받은 `revision`을 PUT에 실어, 그사이 다른 곳에서 바뀐 일정이면 409 `schedule_changed` 안내로 끝난다(`schedule clear`만 싣지 않는다, `cmd/schedule.rs` `save_body`).
- **무인 설치**: `saba-chan-cli install [--dir 폴더] [--components-dir 폴더] [--modules a,b] [--gui|--no-gui]`는 데몬이 아직 없는 PC에 창 없이 설치한다(`cmd/install.rs`). 데몬 API 대신 창 인스톨러와 같은 `saba-chan-installer-core`를 부른다(05 §2). 화면이 없으면(리눅스 `DISPLAY`·`WAYLAND_DISPLAY`가 모두 없음, Windows는 Server Core(레지스트리 `InstallationType`)이거나 `SSH_CONNECTION`이 있거나 `SESSIONNAME`이 없음) 헤드리스 구성, 있으면 데스크톱 구성이고 `--gui`·`--no-gui`가 감지보다 먼저다. 폴더와 모듈은 인자가 없으면 묻고, 요약을 한 번 확인한 뒤 설치한다(`--yes`·`--json-stream`). 설치는 대상 폴더의 CLI를 모두 끄므로, 그 폴더 안에서 실행된 CLI는 시작 전에 사용법 오류로 거절한다. 다른 폴더에 설치본이 이미 있으면(05 §2 설치 위치) 그 폴더를 알리고 사용법 오류로 거절한다. 쓸 수 없는 설치 폴더(`check_install_dir`)와 모듈·익스텐션 위치(`--components-dir`, `check_components_dir`)도 받기 전에 사용법 오류로 거절한다. 어느 설치의 것인지 모르는 데몬이 떠 있으면 끄지 않고 먼저 끄라고 안내하며 멈춘다. 받을 릴리스는 실행한 플랫폼 것(`relay::current_platform`)이다. 지금 릴리스는 `windows-x64`만 게시하므로(05 §3.1) 다른 플랫폼에서는 받기 단계에서 "게시된 릴리스 없음"으로 끝난다. 실패하면 설치 라이브러리 기록을 창 인스톨러와 같은 파일(`log_file_path`)에 덧붙이고 그 경로를 안내한다(파일을 열지 못하면 경로를 안내하지 않는다). 끝나면 데몬을 켜는 명령을 붙여 넣어 실행할 수 있는 꼴로 보인다(Windows는 `pushd "폴더"`와 `.\saba-chan-cli daemon start` 두 줄).
- **제거**: `saba-chan-cli uninstall [--remove-settings] [--remove-instances]`는 이 CLI가 든 설치 폴더의 사바쨩을 명령창에서 지운다(`cmd/uninstall.rs`). 창 제거기와 같은 `saba-chan-installer-core` `uninstall::do_uninstall`을 부르므로 지우는 범위와 남기는 규칙(05 §2 제거)이 같다. 아무것도 붙이지 않으면 설정과 게임 서버를 남긴다. 한 번 확인한 뒤 지우고(`--yes`로 건너뜀), 지우지 못한 항목이 있으면 그 경로와 함께 실패로 끝난다. 설치 폴더에서 실행된 CLI가 아니면 지울 것이 없다고 알린다. 창에서 고르는 화면은 없다(실행 방식은 부르는 쪽이 정한다).
- **전체 화면 메뉴(TUI)**: 인자 없이 터미널에서 실행하면 열린다(`saba-chan-cli tui`도 같다, `tui/`). `tui --server <id>`는 그 서버를 골라 연다(GUI의 "CLI로 전환"이 붙인다). 화면은 위 탭 줄(서버·게임 모듈·익스텐션·디스코드·업데이트·데몬)과 왼쪽 목록, 오른쪽 상세다. 서버 탭의 상세는 한 줄 정보, 자주 쓰는 버튼, 그 아래 큰 콘솔과 입력 줄이다. 드물게 쓰는 버튼은 [더 보기] 창에 모은다(`tui/app.rs` `page_buttons`, `When::More`). 화살표는 놓인 자리대로 움직인다: 탭 줄 ←→는 페이지, ↓는 목록, 목록 →는 버튼, 버튼 ↓는 콘솔 입력, Esc는 한 칸 뒤로. 콘솔 입력은 `console` 연결과 같이 게임 명령 표를 거쳐 보낸다. 버튼은 같은 뜻의 명령 한 줄로 실행해 메뉴에서 되는 일은 명령으로도 되고, 모든 명령은 `:`(또는 `/`) 명령 줄에서 Tab 완성과 함께 돈다. 한 줄 결과는 아래 알림 줄에, 여러 줄은 결과 서랍(F2)에 보인다. 출력(`output` 창구), 진행(`progress` 창구), 확인 질문(`prompt` 창구)을 바꿔 끼워 결과 서랍, 아래 작업 줄, 화면 안 대화상자에 그린다. 데몬 상태·서버 목록은 3초마다, 보고 있는 서버 콘솔(`GET /api/instance/:id/console?spans=1`)이나 데몬 로그(`GET /api/daemon/console?spans=1`)는 1초마다 끝 300줄을 읽어 데몬이 준 토큰으로 칠한다(`tui/highlight.rs`, 토큰 색은 `console` 명령과 같다). `GET /api/clients/launch`의 `gui`가 true면 탭 줄 오른쪽에 "GUI로 전환"(F9)이 보이고, 고른 서버로 GUI를 띄운 뒤 메뉴를 닫는다. Ctrl+C는 실행 중인 명령을 멈춘다(대화상자에서는 취소, 명령 줄에서는 줄 닫기). 화면을 통째로 쓰거나 프로그램을 끝내는 명령(`shell`, `tui`, `gui`, `completions`, `install`, `monitor --watch`, `update apply`)은 안에서 돌리지 않고, `console`·`server console`은 `--follow`로 읽기만 한다. 이 판단은 줄을 명령 표로 읽은 결과로 하므로 별칭이나 앞에 둔 전역 옵션도 같다(`tui/app.rs` `refuse`·`adapt`). 명령 작업이 패닉하면 결과 서랍에 알리고 실행 중 표시를 푼다. 터미널이 아니거나 `--json`이면 열지 않는다(인자 없으면 `status`, `tui`로 부르면 사용법 오류). 폭 60칸 미만이면 넓혀 달라는 안내만 보이고, 명령이 묻는 중이면 질문은 그린다. 명령 처리는 디버그 빌드에서 스택을 많이 써서 16MB 스택 스레드에서 돈다(`main.rs` `STACK`, Windows 주 스레드 기본 1MB).
- **대화형 셸**: `saba-chan-cli shell`, 또는 `SABA_NO_TUI=1`로 인자 없이 터미널에서 실행하면 열린다(`main.rs`, `shell.rs`). 한 줄이 명령 하나이고 Tab으로 명령·서버 이름·게임 명령을 채운다(게임 명령은 `catalog`에서, `shell.rs:109`). 첫 단어가 명령이 아니면 디스코드와 같은 채팅 처리기로 보낸다(`shell.rs:1-5, 187`). 셸을 닫아도 데몬과 서버는 계속 돈다.
- **스크립트용 `--json`**: stdout에는 JSON 문서 하나만 나가고, 안내와 진행 표시는 stderr로 간다(`output.rs:1-6`). 종료 코드: 0 성공, 1 실패, 2 사용법·모호함, 3 데몬 응답 없음, 4 대상 없음, 130 취소(`error.rs`의 `EXIT_CODES`). `help --json [명령…]`은 명령 트리(경로·별칭·설명), 인자(이름·플래그·위치·필수·값·선택지), 전역 옵션, 종료 코드 표를 JSON 하나로 낸다(`cli::schema`). 데몬 없이 되고, 별칭으로 찾아도 경로는 본 이름이다. 맨 위 `help`는 이 명령이라 clap의 하위 `help`는 꺼지고, `server help start`처럼 하위 자리에 온 `help`는 `cli::parse`가 맨 앞으로 옮겨 읽는다. 사람용 출력은 모두 `output::write_line` 한 창구(`say!`·`tell!`)를 지난다. 명령 코드가 `println!`으로 직접 찍지 않는 것은 시험이 지킨다(`no_command_talks_to_the_terminal_directly`). 진행 표시(`progress::Progress`)와 질문(`prompt::Asker`)도 같은 방식으로 바꿔 끼울 수 있고, `indicatif`·`dialoguer`는 각 파일 밖에서 쓰지 않는다. TUI가 이 창구들을 바꿔 끼워 명령 결과는 패널에, 진행은 작업 줄에, 질문은 대화상자에 그린다. `--json-stream`은 `--json`을 포함하고, 진행 표시를 막대 대신 stdout에 `{type:"progress", id, kind?, message?, position?, length?, done?}` 한 줄씩 내며(같은 막대의 위치는 250ms 간격, `progress::stream_lines`), `events`·`audit -f`의 줄에는 `type:"event"`가 붙고, 마지막 줄에 `{type:"result", data}`로 `--json`과 같은 문서(실패면 `{ok:false, error, error_code}`)를 싣는다. 전역 옵션은 `--json`, `--json-stream`, `--yes`, `--lang`, `--port`, `--no-color`.

### 3.2 구조
- `client.rs`: 데몬 HTTP 클라이언트. 토큰 파일을 읽어 `X-Saba-Token`을 보내고, 401이면 토큰을 다시 읽어 1회 재시도한다(`client.rs:28-33, 97-106`). 업데이터 실행도 데몬 API로 요청한다(`launch_updater` `:785-789`).
- `bootstrap.rs`: 데몬이 뜨기 전에 필요한 것만 직접 한다. `config.json`의 `gui.ipcPort`·`gui.language` 읽기(포트는 `SABA_IPC_PORT` > 파일 > 기본값, 언어는 `--lang` > `SABA_LANG` > 파일 > 시스템 로캘 > en, `:18-46`. `--lang`이나 `SABA_LANG`으로 직접 골랐을 때만 그 언어를 요청마다 `Accept-Language`로 실어(`client.rs` `Api::new`) 데몬 문구와 그 요청이 부르는 모듈도 같은 언어로 오게 한다. 고르지 않았으면 싣지 않아 데몬이 앱 설정 언어를 따른다), 설정 포트에 데몬이 없을 때 실행 중인 데몬이 남긴 `<dataDir>/daemon.lock.owner`(`src/instance_lock.rs`)의 포트에서 그 데몬(같은 pid)이 토큰 없는 `/health`로 답하면 그 데몬에 붙기(`live_port`: 포트를 바꾸고 데몬을 아직 다시 켜지 않았을 때 그 데몬을 끄고 새 포트로 다시 켤 수 있게 한다. 새 데몬은 언제나 `--port` 또는 설정 포트로 띄운다. 셸은 `--port` 없이 열었으면 줄마다 설정 포트와 붙을 포트를 다시 찾고, 한 줄에 준 `--port`는 그 줄의 붙을 곳과 띄울 곳 모두에 쓴다), `saba-core` 실행 파일 찾기와 분리 실행(띄운 데몬이 응답 전에 끝나면 시간 초과를 기다리지 않고 종료 코드를 알린다), 정상 종료가 안 될 때 **그 PID만** 강제 종료(`:92`).
- `ctx.rs`: 데몬이 필요한 명령 앞에서 `ensure_daemon`이 데몬을 확인한다. 꺼져 있으면 물을 수 있을 때(대화형, `--yes` 아님) 켤지 묻고 `start_daemon`으로 띄운다(`:22-41, 43-61`).
- `catalog.rs`: 게임 명령을 하드코딩하지 않는다. `GET /api/modules`의 `commands`·`command_aliases`와 `GET /api/instance/:id/commands`로 목록·사용법·입력 칸을 만든다(`:1-9`). 인자 검사의 최종 판단은 데몬이 한다. 명령의 `method`는 데몬이 준 값을 그대로 보여 줄 뿐 분기하지 않는다(`server commands --json`, `cmd/server.rs:983`). 데몬은 콘솔 명령의 method를 `console`(`console_template`)이라 부르고 예전 이름 `rcon`·`rcon_template`도 별칭으로 받는다(09b8e4f, 02 §1.2).
- `slots.rs`: 데이터 slot(03-extensions 4.3)을 출력만 한다. 서버의 `slots`(`ServerCard.badge`, `ServerDetail.status`)와 익스텐션의 `slots["AddServer.options"]`를 읽고, 답은 서버를 만들 때 `extension_data`로 보낸다.
- `i18n.rs`: 설치 폴더의 `locales/<lang>/cli.json`을 먼저 읽고, 없으면 바이너리에 넣어 둔 en/ko를 쓴다(`:11-12, 36-52`). clap 도움말은 `help.cmd.*`·`help.arg.*` 키가 있으면 그 문구로 바꿔 끼운다(`cli.rs:811-839`).
- 모듈이 구현하지 않은 기능(MK1): `GET /api/modules`의 `functions`가 배열이면 빠진 선택 함수를 쓰는 명령(`cmd/module.rs` `OPTIONAL`: `server reset`·`server version`·`server diagnose`·`server validate`·`server properties [set|reset]`·`module versions`·`module install-files`)을 `module info`·`server info`의 "지원하지 않음" 줄로 보이고(`server info --json`은 `unsupported`), `server reset`·`server properties reset`은 확인을 묻기 전에 `not_supported`로 멈춘다. 데몬의 `not_supported` 답에는 안내 한 줄이 붙는다(`error.rs` `api_hint`). `functions`가 null이거나 없으면 아무것도 숨기지 않는다.
- 실행 파일 이름(MK7): `module install-files --executable FILE`(예전 `--jar`는 별칭)은 `POST /api/module/:name/install`에 `server_executable`로 보낸다. 시작 실패의 `action_required`(`client.rs` `check`)는 `error_code`가 없을 때 오류 코드가 되고, `executable_not_found`(옛 `server_jar_not_found`)에는 다시 설치하라는 안내가 붙는다(`error.rs` `api_hint`).
- 스키마 2 설정(MK2·MK8, 06 §7·§8): `server set`은 `list` 설정 값을 쉼표로 나누거나 JSON 배열로 받아 문자열 배열로 보내고(선택지가 있으면 항목마다 맞춘다, 빈 값은 빈 목록), `multiline` 값의 `\n`을 줄바꿈으로, `\\`를 `\`로 바꿔 보낸다. `server settings`는 `GET /api/instance/:id`의 `hidden`에 든 설정을 빼고 그 이름을 끝에 한 줄로 보이며, 목록은 쉼표로, 여러 줄은 ⏎로 이어 보이고, `restart_required` 설정에 "바꾸면 다시 시작"을 붙인다. 저장 응답의 `restart_required`가 비어 있지 않으면 그 이름과 재시작 명령을 보인다(키가 없는 옛 데몬이면 켜져 있을 때 늘 안내). `module list`는 `failed[]`의 `module_schema_invalid`면 "읽지 못했어요"와 `problems`를 한 줄씩, `module_schema_unsupported`면 필요한 형식 `required`를 보인다.
- 사람이 직접 하는 동의(약관 `server agree`·`server add`, 격리 없이 돌리기·익스텐션 GUI `sandbox allow`)는 `prompt.rs` `agreed`·`consent`로만 받는다. `--yes`·`--json`은 동의가 아니고, 물을 수 없으면 터미널에서 하라는 오류다. 묻지 않고 동의하는 길은 읽은 약관 id를 대는 `server add --accept ID`뿐이다.
- 이름으로 대상 찾기(`resolve.rs`): 정확히 같음 → 대소문자 무시 → 앞부분 → 포함 순서로 찾는다. 지우기·초기화(`server remove`·`server reset`·`module remove`·`extension remove`)와 권한 대상 멤버(`perm`)는 앞의 두 단계만 쓴다(`find_exact`). 앞부분·포함으로만 맞는 이름은 고르지 않고 제안으로 보인다. 이 명령들은 이름 없이 물을 수 없으면(`--json`, 파이프) 대상이 하나뿐이어도 고르지 않고 이름을 대라고 한다.
- 일부만 된 결과: 데몬이 2xx에 `ok`/`success: false`와 결과만 싣는 응답(`update download`·`update apply`의 `errors`, `chatbot forget`의 `sync`, `server set`·`server add --set`의 `synced: false`)은 `client::partial`로 본문을 받아 된 것과 실패한 것을 보이고 `partial`(종료 코드 1)로 끝낸다. 구성요소를 고르지 않은 받기가 `errors` 없이 `{ok:false, error}`로 실패해도 실패로 끝낸다. 적용에 실패한 구성요소가 있어도 코어 업데이트가 있으면 GUI처럼 경고하고 업데이터를 띄우며, 종료 코드 1로 실패를 남긴다. 업데이터를 띄운 뒤에는 `GET /api/updates/last-result`가 적용 전과 다른 결과(시각)를 줄 때까지 2초마다 묻고(최대 15분, 그 사이 데몬이 꺼져 있는 오류는 기다림의 일부다) 결과와 패치노트를 보인 뒤 끝낸다. 결과가 실패이거나 15분이 지나면 종료 코드 1이다(`cmd/update.rs` `wait_for_result`, 06 §9). `monitor --watch --json`은 표본마다 JSON 한 줄을 낸다(`events`와 같다).
- `cmd/`: 명령 갈래마다 파일 하나(backup, bot, chatbot, config, console, daemon, extension, module, schedule, server, status, update).
- 콘솔 색: 터미널이 색을 쓸 때만 `?spans=1`을 붙여(`client.rs` `console`) 데몬이 준 토큰 구간을 토큰별 ANSI 스타일로 칠하고, 나머지는 줄 `level` 색을 쓴다(`cmd/console.rs` `format_line`, `token_style`). 토큰은 02 §`HighlightRuleToml`.

- `bot pair [--relay-url URL]`: 페어링 코드를 받아 보여 주고(디스코드 `/사바쨩 연결`·`/saba connect`에 넣는다) claimed까지 기다린다. 데몬은 설정한 릴레이와만 페어링하므로, `--relay-url`이 설정과 다르면 봇 설정 `cloud.relayUrl`을 먼저 바꾼 뒤 페어링한다. `--json`만 주면 코드만 내고 끝, `--yes`면 기다린다.
- `bot channels GUILD [--chatbot 이름,…] [--no-chatbot] [--music 이름] [--no-music] [--notify 이름] [--no-notify]`: 채널 이름이나 id를 GET 응답의 `channels`로 id로 바꿔 PUT한다. `--notify`는 알림 채널(I3: 크래시·백업·업데이트·예약 예고)이다. 음악 채널을 새로 정할 때는 메시지가 지워진다고 확인을 받는다.
- `bot link [ID] [--decline]`: 디스코드에서 코드 없이 `/사바쨩 연결`로 온 연결 요청(J1-b). ID 없이 부르면 목록(`GET /api/relay/link-requests`), ID면 수락, `--decline`이면 거절(`POST /api/relay/link-requests/:id`). `bot pair`는 끝에 GUI 디스코드 화면에서도 볼 수 있다는 안내를 붙인다(J1-c).
- `bot allow GUILD [서버…] [--all]`: 그 디스코드 서버가 보고 부를 수 있는 서버(`nodeSettings[guild].allowedInstances`, Q-4). 서버 없이 부르면 보기, `--all`은 키를 지워 제한 없음. `PUT /api/guilds/:guild/allowed-instances`로 저장한다(`--all`이면 `instances:null`). 없는 서버 id는 데몬이 400으로 거절한다.
- `server discord [SERVER] [--word 낱말,…] [--command 명령=낱말,…] [--clear]`: 디스코드에서 이 서버와 그 명령을 부르는 낱말(봇 설정 `serverWords[id]`, 인스턴스에 딸려 어느 디스코드 서버에서나 같다, `cmd/discord_words.rs`). GUI 서버 상세의 디스코드 탭과 같다. 옵션 없이 부르면 `GET /api/instance/:id/words`의 서버 낱말(괄호는 기본 호출명 `defaultWords`)과 명령별 낱말(괄호는 module.toml 별명)을 보여 준다. GUI 디스코드 탭도 서버 낱말 칸에 `defaultWords`를 "기본:"으로 보인다. 바꿀 때는 지금 값에서 시작해 `--word`(서버 낱말 통째, `--word ""`는 지움)와 `--command`(그 명령만, `명령=`은 지움)만 바꾸고 `{words, commands}` 전체를 `PUT /api/instance/:id/words`로 보낸다. `--clear`는 둘 다 비운다. 낱말 규칙·충돌은 데몬이 검사하고, 400 `validation_failed`의 `details`는 칸마다 한 줄로 보인다(`error.rs` `api_hint`). 0.1.10의 `--guild`(디스코드 서버마다 따로 정하기)는 없앴다. 예전 `bot words`와 전역 별명 명령 `bot alias`·`bot server-alias`·`bot word`도 없다.
- `bot lang GUILD [auto|코드]`,
 `bot sleep GUILD`, `bot wake GUILD`, `bot music GUILD [--on|--off] [--ui on|off] [--queue-lines N] [--refresh MS] [--normalize on|off]`: 디스코드 길드 설정(I2) `GET/PATCH /api/discord/:guild/settings`. 값이 없으면 보기만 한다. 봇에 닿지 않아 `sync.ok:false`면 다음 연결 때 보낸다고 알린다.
- `server launch-preview [SERVER] [--args "..."]`: `POST /api/instance/:id/launch-preview`의 `command_line`을 보여 주고, 형식이 틀린 인자는 "잘못된 형식입니다"로 알린다. `--args`가 없으면 저장된 추가 인자로 본다.
- `audit [SERVER] [--limit N] [-f]`(A10, 별명 `activity`): `GET /api/audit`를 표로 보여 준다. `-f`는 목록을 읽기 전에 `GET /api/events`에 붙고, 이어서 `action.run`을 한 줄씩 따라간다(서버를 주면 그 서버 것만, `--json`이면 data 한 줄씩). 끊기면 2초 뒤 마지막 번호로 다시 붙는다. CLI 요청에는 `X-Saba-Client: cli`가 붙는다.
- `events [--kind EVENT…] [--server S] [--since ID]`: `GET /api/events`를 한 줄씩 흘려보낸다(`--json`이면 `{id, event, data}` 한 줄씩, NDJSON). `--kind instance`는 `instance.exit` 같은 아래 이벤트도 받는다. 서버를 주면 `data.instanceId`를 id와, `instanceId`가 없는 `provision.progress`는 `instance`(인스턴스 이름)를 이름과 비교해 거른다. `resync`는 거르지 않는다(받은 쪽이 목록을 다시 읽는다). 끊기면 2초 뒤 마지막 번호로 다시 붙는다.
- `do [ACTION] [--server S] [--plan] [KEY=VALUE…]`(P2): 액션 카탈로그. 액션 없이 부르면 `GET /api/actions?surface=cli` 표. 있으면 `plan`을 부르고, `needsConfirm`이면 요약을 보이고 묻은 뒤 `planId`로 `run`한다. `--plan`은 요약만. 값이 JSON으로 읽히면(숫자·참거짓·배열·객체) 그대로 보낸다.
- `config set KEY=VALUE…`(앱 칸)·`bot config KEY=VALUE…`: 고친 경로만 담은 merge patch(빈 값은 `null`로 지우기)를 읽은 때의 `revision`과 함께 `PATCH /api/config/gui`·`PATCH /api/config/bot`으로 보낸다(`cmd/mod.rs` `config_patch`). 같은 최상위 키 아래 다른 값(다른 길드의 `nodeSettings` 등)을 읽은 때의 값으로 덮지 않고, 그사이 바뀌었으면 409 `settings_changed`로 끝난다. `cli.` 칸은 그대로 `PUT /api/config/client/cli`. 목록 출력에서 `revision`은 뺀다
- `config paths`(별명 `dirs`): `GET /api/system/paths`의 데이터·설치·인스턴스·모듈·익스텐션 폴더와 기본값·출처를 보인다. `config move modules|extensions FOLDER`: 요약을 한 번 확인한 뒤 `POST /api/system/paths/move`로 옮기고, 남은 원본(`left_behind`)이 있으면 알린다. 실패 코드 `paths_*`는 `err.hint.paths_*` 문구로 보인다(`error.rs` `api_hint`).
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
- AI 챗봇 설정의 원본은 데몬 config.json `chatbot`이다. `GET/PUT/DELETE /api/chatbot`, `DELETE /api/chatbot/memory?guildId=`로 다루고, 바뀔 때마다와 에이전트가 연결될 때 이 호스트의 길드마다 릴레이에 PUT해서 맞춘다. 릴레이 PUT은 받은 키만 합치므로 데몬은 늘 전체 설정을 보내고, 지운 키는 릴레이 기본값으로 채운다(`handlers/chatbot.rs:31-65`). DELETE가 릴레이에 닿지 못하면 `pendingRelayDelete: true`로 답하고 다음 연결 때 다시 지운다(`:218-238, 240-255, 287-295`). API 키는 데몬과 릴레이만 알고, 클라이언트에는 `hasApiKey`만 준다(`handlers/chatbot.rs:1-15`, `mod.rs:643-644`).

### 4.2 릴레이 에이전트 (`src/relay_agent.rs`)

| 단계 | 요청 | 내용 |
|---|---|---|
| 버전 확인 | `GET /info` | 릴레이가 요구하는 최소 에이전트 버전 확인 |
| 하트비트 | `POST /heartbeat` (60초) | `agentVersion`, `os`, `metadata`(서버·모듈·봇 설정). 릴레이가 `UPDATE_REQUIRED`를 주면 경고 로그를 남긴다(`relay_agent.rs:281-327`). 지금 릴레이는 경고 대신 426 `AGENT_TOO_OLD`로 거부하고(05 §3 `/heartbeat`), 데몬은 이를 하트비트 오류로 남긴다. 잠자기는 길드 설정(I2)이라 폴링은 쉬지 않는다. 게임 서버 비밀번호(`rcon_password`, `rest_password`)는 메타데이터에서 뺀다(273-276) |
| 명령 받기 | `GET /poll` (롱폴링) | 디스코드에서 온 명령. 현재 `payload.action`은 `raw_command` 하나다 |
| 결과 보내기 | `POST /result/:id` | 채팅 처리기의 응답 `{success, data:{text}}` (`:505-567`) |

- 모든 요청은 노드 토큰으로 HMAC 서명하고 `X-Saba-Client: 1` 헤더를 붙인다(`relay_agent.rs:197-201, 215-229`, `handlers/relay.rs:117` `signed_headers`). 노드 토큰을 싣는 프록시 요청(`/api/relay/host/*`, `/node/*`)은 `?relayUrl=`이 설정된 릴레이와 같거나 루프백일 때만 따르고, 아니면 설정된 릴레이로 보낸다(`handlers/relay.rs:46-67`).
- 데몬이 뜨면 `auto_start`가 에이전트를 켠다(토큰이 없으면 info 로그만 남기고 건너뜀, `ipc/mod.rs:860-866`, `relay_agent.rs:178-189`). 에이전트가 연결되면 AI 챗봇 설정도 릴레이에 맞춘다(`relay_agent.rs:131`).
- 상태 `GET /api/relay/agent`: `{running, relay_url, has_token, agent_version, last_heartbeat_at, last_error, commands_handled}` (`relay_agent.rs:43-55`). 켜고 끄기는 `POST /api/relay/agent/start`, `/stop`.
- 새 명령 종류는 `handle_payload`의 action 분기에 더한다.

### 4.3 채팅 처리기 (`src/chat/`)

- 디스코드 명령과 CLI `chat`, `POST /api/chat/command {text, guild_id?, user_id?} → {reply, code, args}`가 모두 같은 처리기를 쓴다(`handlers/chat.rs:9-16`).
- 대상 서버 찾기: 인스턴스 이름·id → 그 서버를 부르는 낱말(`serverWords[id].words`) → 모듈 이름·게임명(모듈 locales `display_name`)·module.toml 별명 순이다. 같은 게임 서버가 여럿인데 하나로 좁혀지지 않으면 `errors.multiple_instances`로 답한다. 위험 명령은 모듈 이름·게임명·별명으로 부르면 `errors.danger_needs_name`으로 거절한다(`chat/mod.rs` `named_servers`·`find_server`, 01 §4).
- 명령 낱말은 그 길드·서버의 `commands`, 그다음 module.toml 별명이고, 접두사는 `prefixes`다. 맞춰 볼 때 대소문자를 가리지 않고 긴 것부터 보며, 접두사 뒤가 공백·쉼표·쌍점이거나 글 끝이어야 한다. 도움말에는 첫 접두사를, 비어 있으면 언어별 `default_prefix`(bot.json)를 쓴다. 응답 문구는 `locales/<lang>/bot.json`을 바이너리에 넣어 쓴다(`chat/i18n.rs`).
- 노드(디스코드 서버)별 허용 인스턴스와 멤버별 명령 권한은 `nodeSettings`에 있다.

### 4.4 봇 초대

- `GET /api/relay/bot-invite`(query `relayUrl?`)가 릴레이의 공개 `/api/bot-invite` 응답 `{url}`을 그대로 돌려준다. 공개 엔드포인트라 서명하지 않지만, 주소는 서명 요청과 같이 설정한 릴레이나 루프백만 쓴다(`handlers/relay.rs:428-450`).

## 5. 로케일 구조 (`locales/<lang>/{common,gui,bot,cli,web}.json`, 10개 언어)

| 파일 | 최상위 키(en) | 소비자 |
|---|---|---|
| common.json (leaf 80개) | app_name, success/error/…, daemon, modules, server, network, port_conflict, init, tray, updates, errors 등 25개 | GUI renderer(`common` ns, JS 브리지의 오류 문구 `createApi.js:9`), **Tauri 셸**(트레이·알림·status 메시지, `i18n.rs`. 설치 폴더 파일 우선, 없으면 exe 내장본) |
| gui.json (en leaf 677개) | title, menu, title_bar, modals, settings_modal, add_instance_modal, migration_modal, command_modal, server_*, discord_bot, discord_modal, notice_modal, updates, extensions, saba_storage, validation, context_menu 등 29개 | GUI renderer(`gui` ns) |
| bot.json | 채팅 응답 문구 | 데몬 채팅 처리기(`src/chat/i18n.rs`, 빌드 때 포함). GUI renderer도 import한다 |
| cli.json (PR #6 기준 en leaf 402개) | server, daemon, module, ext, bot, backup, schedule, chatbot, err, prompt 등 39개 | CLI `i18n.rs`. 설치 폴더의 `locales/<lang>/cli.json`을 먼저 읽고, 없으면 내장 en/ko를 쓴다(`i18n.rs:11-12, 36-52`). ko만 도움말 번역 `help`(leaf 183개)가 더 있어 585개다. 나머지 언어의 도움말은 clap 기본 영어다. qcza9f에는 아직 예전 TUI용 cli.json(en 200개, `screen`·`menu` 등 16개)이 있다 |
| web.json | app_name, tabs, gate, session, servers, console, settings, backups, perm, modules, plugins, more, errors 등 | 사바쨩 웹(`web` ns, §1.5). 10개 언어 모두 en과 같은 키 |

- GUI renderer는 영어 common, gui, bot JSON만 번들에 넣고, 다른 언어는 그 언어로 바꿀 때 `import.meta.glob` 청크로 불러온다(i18n.js의 `localeBackend`). 첫 렌더는 캐시된 언어 파일을 읽은 뒤로 미룬다(`i18nReady`, index.js). 언어 소스는 `window.electron.getLanguage()`가 1순위, localStorage `i18nextLng`가 2순위다. 확장 i18n은 daemon `/api/extensions/:id/i18n/:locale`에서 받는다.
- **업데이트가 필요해요**: 사바쨩이 낮아 거절된 경우(릴레이 `agent_too_old`, 모듈 `module_dependency_unsatisfied`의 `saba-core` 요구·`module_schema_unsupported`, 익스텐션 `app_version_unsatisfied`·`component_version_unsatisfied`의 `saba-core`)는 gui.json·cli.json의 `update_required.*`(title·relay·module·module_newer·extension, GUI는 check도) 한 벌로 보인다. 규칙은 GUI `utils/updateRequired.js`와 CLI `error::update_required`가 같다. 이름은 GUI는 부른 쪽이, CLI는 응답의 `module`·`extension`을 쓴다.
- **번역 누락**(en 기준 diff):
  - gui.json: de, es, fr, pt-BR, ru, zh-CN, zh-TW는 661개로 **16개가 빠져 있다**(`console.dock_focus/dock_restore/minimize/pin/popin/unpin`, `discord_modal.music_channel_*`, `discord_modal.music_normalize`). ja는 667개로 `discord_modal.music_*` 10개가 빠져 있다.
  - cli.json: qcza9f의 예전 파일은 ko를 제외한 8개 언어가 168개로 33개가 빠져 있다(`screen.bot_*` 등). PR #6의 새 파일은 10개 언어 모두 en 키가 빠짐없이 있다.
- 하드코딩된 한국어 문자열: 옛 Electron main.js의 트레이·알림·오류 매핑·로딩 문구는 Tauri로 옮기며 모두 `common.json` 키(`tray.*`, `updates.*`, `init.*`, `daemon.*`, `errors.*`, `port_conflict.*`)를 거친다(셸 `AppState::t` `lib.rs:40-42`, 브리지 `t` `createApi.js:9`). 영어로 남은 문구: `createApi.js:126`(401 안내), `native.rs:154`(`Installer not found`), `lifecycle.rs:617`·`native.rs:52-53`(데몬 상태 메시지), 팝아웃 창 제목 `Console — <이름>`(`window.rs:207`)

---

## 6. 불일치, 죽은 코드, stub, 버그 (file:line)

### 6.1 존재하지 않는 endpoint나 잘못된 HTTP 메서드 호출
(예전 TUI CLI의 항목들은 CLI 리메이크에서 코드째 사라져 뺐다. §3 참조.)
1. ~~`updater:setConfig`가 POST를 써서 405~~ 해결됨: 이제 PUT이다(옛 Electron main.js:2505, 지금은 `createApi.js:781`).
2. ~~CLI `update download`/`update apply`가 본문 없이 POST하는데 데몬은 JSON 본문을 요구했다~~ → 해결: PR #6 CLI는 `{components:[...]}`를 보낸다(`saba-chan-cli/src/client.rs:774-784`). 데몬도 본문이 없거나 공백이면 전체로 받고, 깨진 JSON은 400 `invalid_body`로 거부한다(`src/ipc/updates.rs:348-371, 475-480`).

### 6.2 죽은 코드와 stub
- ~~(옛 Electron main.js:501-503) `syncGuiConfigToDaemon`(no-op)과 이를 부르는 `guiConfig:sync` 채널~~ → 해결: 함수, 채널, preload 항목이 모두 없어졌고, Tauri 브리지에도 없다.
- ~~(옛 Electron main.js:1005-1011) `preloadLightData`~~ → 해결: 없어졌다. 셸의 `finish_background_init`이 로딩 상태 문구만 보낸다(`lifecycle.rs:93-116`).
- renderer가 쓰지 않는 window.api(§1.2).
- ~~(옛 Electron main.js:1223-1224) auto_download/auto_apply는 "향후 구현 예정"~~ → 해결: `auto_download`이면 데몬이 확인 때 미리 내려받아 둔다(`src/ipc/updates.rs:215-216, 236`). 동작하지 않던 `auto_apply`는 설정에서 빠졌고, 적용은 언제나 사용자가 고른다.

### 6.3 기능 버그
- ~~**before-quit bot 정지 요청 누락**: (옛 Electron main.js:1536, 1559) `currentPort` 범위 오류로 ReferenceError~~ → 해결: 비정상 종료 경로(`on_exit` `lifecycle.rs:690-712`)는 봇을 건드리지 않고 unregister와 자기 데몬 정리만 한다.
- ~~**SabaStorage bot 재시작 실패**: SabaStorage.js:318, 445가 `settings?.discordToken`을 읽음~~ → 해결: 로컬 봇이 없어져 SabaStorage는 봇을 재시작하지 않고, renderer에 `discordToken`을 읽는 곳이 없다.
- ~~**settings 경로 분기 기준 오류**: (옛 Electron main.js:483, 2860, 2967) `daemonProcess`로 판단~~ → 해결: 데몬이 응답하는지로 나누고, 응답하면 API로만 쓴다(`createApi.js:57-74, 789-816`, `native.rs:189-199`). 읽기 직접 접근은 남아 있다(§1.3).
- ~~**resolver 주석과 코드 불일치**: resolver.js:57(1분)과 60(5초)~~ → 해결: `resolver.js`는 로컬 봇(`discord_bot/`)과 함께 L1(7b5102f)에서 지워졌다.
- ~~**updater exe 이름 하드코딩**: (옛 Electron main.js:1113-1148), 예전 TUI `commands.rs:1137-1142`~~ → 해결: GUI와 CLI는 `POST /api/updates/launch-updater`만 보내고, 데몬이 `saba-chan-updater{EXE_SUFFIX}`를 찾아 띄운다(`src/ipc/updates.rs:747-754`).

---

## 7. ★ Daemon 우회 지점 전수 목록 (GUI · CLI)

> 기준: "인터페이스는 혼자서 아무것도 못 한다. 파일 편집도 daemon API를 거친다."
> 분류: **[FS-R]** 직접 파일 읽기 / **[FS-W]** 직접 파일·디렉터리 쓰기 / **[PROC]** 프로세스 spawn·kill / **[EXT]** 외부 서비스 직접 호출 / **[CFG]** 설정 파일 직접 편집 / **[OS]** OS 레지스트리·셸
> "불가피" 표시는 부트스트랩상 daemon 없이는 할 수 없는 항목이다. 그래도 목록에는 포함했다.

### 7.1 GUI Tauri 셸 (`saba-chan-gui/src-tauri/src`)
| # | 위치 | 분류 | 내용 | 비고 |
|---|---|---|---|---|
| G1 | daemon.rs:154-181 | FS-R | `.ipc_token` 읽기 | 불가피(인증 부트스트랩) |
| G2 | settings.rs:28-39, 71-84 | FS-R/CFG | `load_settings()`는 daemon 상태와 상관없이 **항상** config.json `gui`(깨졌으면 `.bak`, 없으면 예전 settings.json)를 직접 읽는다. 기동 시 포트·언어(`lib.rs:94-96`), `create_main_window`, 데몬 기동 env, `refresh_ipc_base`, 창 크기 저장, `language_get`, `settings_local_load`(브리지의 settings 폴백)가 이 함수를 쓴다 | ipcPort 부트스트랩만 불가피하고, 나머지는 API로 대체할 수 있다 |
| G3 | settings.rs:47-61, native.rs:189-227 | FS-W/CFG | `write_gui_section()`: 데몬이 **응답하지 않을 때만** config.json `gui`를 직접 쓴다(데몬과 같은 `ConfigFile` writer). 셸 `save_settings()`(windowBounds 저장 `window.rs:186`, `language_set`)와 브리지가 부르는 `settings_local_save`(`createApi.js:67-73`)가 이 경로를 탄다 | 부트스트랩 전용. 예전 `daemonProcess` 분기 버그는 해결(§6.3) |
| G4 | createApi.js:763-770 | FS-R/CFG | `settingsLoad`: API 실패 시 `settings_local_load`로 로컬 폴백 | 위와 같음 |
| G5 | logging.rs:10-45 | FS-W | 로그 디렉터리 생성과 로그 파일 쓰기 | GUI 자체 로그 |
| G6 | i18n.rs:50-62 | FS-R | `locales/<lang>/common.json` 직접 읽기(없으면 exe 내장본) | |
| G7 | lifecycle.rs:158-244, 389-436 | PROC | **saba-core spawn** (`--spawned`, env 주입). 콘솔 창 없이 분리해 띄운다(Windows `CREATE_NO_WINDOW`·`CREATE_NEW_PROCESS_GROUP`·`CREATE_BREAKAWAY_FROM_JOB`, job이 막으면 breakaway 없이 다시, Unix `process_group(0)`). 로그는 GUI가 사는 동안 파이프로 받는다. 인터페이스만 종료하면(`interface_only_quit` `:630-644`) 자식 핸들만 놓아 데몬이 남는다. GUI가 죽거나 강제 종료돼도 데몬은 남고, 다음 GUI 실행 때 `ensure_daemon`이 `/health`로 떠 있는 데몬을 찾아 다시 붙는다. 데몬을 끄는 것은 전체 종료(`clean_quit`, G8)뿐이다 | 불가피(daemon 부재 시 부트스트랩) |
| G8 | lifecycle.rs:292-338 | PROC | daemon 종료: `POST /api/daemon/shutdown` 후 40초 안에 안 끝나면 **자기 PID만** kill(`taskkill /PID /F`, SIGKILL) | API 우선, kill은 대비책 |
| G9 | — | — | ~~cleanQuit의 이름 기반 `taskkill /IM saba-core.exe`·`pkill -f saba-core`~~ → 해결: `clean_quit`(`lifecycle.rs:647-674`)은 unregister(shutdown=true) 후 `stop_daemon()`만 부르고, 이름 기반 kill은 없다(`:309-310` 주석) | |
| G10 | lifecycle.rs:592-618 | PROC | `daemon_restart`: `stop_daemon()`(API 우선) 후 `start_daemon()`으로 직접 spawn | 재기동 API가 없다 |
| G11 | tray.rs:447-459 | PROC | 트레이 메뉴: "데몬 종료"는 `stop_daemon()`, "데몬 시작"은 `start_daemon()` 직접 호출 | |
| G12 | lifecycle.rs:690-712 | PROC | `RunEvent::Exit`(`clean_quit`을 거치지 않은 경로)에서 자기 데몬을 `force_kill`. 인터페이스만 종료한 경우(`keep_daemon_alive`)는 건너뛴다 | 자기가 띄운 프로세스만 |
| G13 | — | — | ~~`node scripts/mock-release-server.js` spawn~~ 없어짐(f6ca7b6). 개발자가 직접 실행한다 | |
| G14 | — | — | ~~updater exe detached spawn(`--apply --relaunch`)~~ → 해결: GUI는 `POST /api/updates/launch-updater {relaunch}`만 보내고 업데이터는 데몬이 띄운다(`native.rs:81-102`, `src/ipc/updates.rs:609-675`) | |
| G15 | — | — | ~~updater exe 경로 탐색(`existsSync`)~~ → 해결: 데몬의 `updater_binary()`가 찾는다(`updates.rs:747-754`) | |
| G16 | native.rs:151-184 | OS/PROC | 레지스트리 `HKCU\…\Uninstall\Saba-chan` `InstallLocation` 읽기(`winreg`), installer `--uninstall` 분리 spawn | |
| G17 | — | — | ~~relay `PATCH /api/hosts/:hostId/bot-config` 직접 호출~~ 없어짐: 봇 설정은 `PUT /api/config/bot`, 릴레이 동기화는 에이전트 하트비트 | |
| G18 | native.rs:342-395, window.rs:63-96 | OS | `shell_open_external`·탐색 잠금(http(s)만 바깥 브라우저), `shell_open_path`·`logs_open_folder`(로그 폴더, 인스턴스 폴더) | UI 성격 |
| G19 | native.rs:304-338 | OS | 네이티브 파일/폴더/저장 다이얼로그(tauri-plugin-dialog) | UI 성격(경로 선택만 하고, 실제 스캔·내보내기는 daemon) |
| G20 | — | — | ~~`http.request` 전역 monkey-patch~~ → 해결(D12): 토큰은 데몬 클라이언트(`daemon.rs:184-194`)가 데몬 요청에만 붙인다 | |

### 7.2 GUI renderer (`saba-chan-gui/src`)
| # | 위치 | 분류 | 내용 |
|---|---|---|---|
| R1 | i18n.js:42-81, SettingsModal.js:268 | FS-W(localStorage) | 언어 캐시 `i18nextLng` |
| R2 | utils/themeManager.js:51-121 | FS-W(localStorage) | 테마와 커스텀 테마. config.json `gui`에도 accent 계열을 저장하므로 저장소가 둘로 갈린다 |
| R3 | Modals/NoticeModal.js:18-28 | FS-W(localStorage) | 공지와 알림 기록 |
| R4 | i18n.js:3-30 | 빌드타임 | 로케일 JSON 정적 번들. 런타임 locale 업데이트(daemon의 Locales 컴포넌트 자동 적용. 셸 알림에서는 빼기만 한다, `events.rs:144-147`)가 renderer에는 반영되지 않는다. 셸 문구는 설치 폴더 파일을 먼저 읽어 반영된다(`i18n.rs:50-58`) |
| R5 | components/ServerCard.js:419-421, App.js:494-501 | 규칙 중복 | managed 여부를 `module_settings.managed_start`와 모듈 `interaction_mode`로 다시 계산한다. 데몬은 `GET /api/servers`의 `managed`로 판정을 주는데(`handlers/server.rs:87`), 규칙이 다르다: 값이 없으면 데몬은 늘 managed(`supervisor/mod.rs:1690-1699`), renderer는 `console` 모드 모듈만 managed로 본다 |

(renderer는 `fetch`, `axios`, 직접 IP 접근을 하지 않는다. 모두 `window.api`(브리지 → `daemon_request` 명령)를 거친다.)

### 7.3 CLI (`saba-chan-cli/src`, PR #6 기준)
예전 TUI의 우회 지점 C3~C7, C11~C13(로컬 fallback, `cli-settings.json`, updater 직접 실행 등)과 이름 기반 kill은 리메이크에서 사라졌다. 남은 것은 부트스트랩뿐이다.

| # | 위치 | 분류 | 내용 | 비고 |
|---|---|---|---|---|
| C1 | client.rs:28-33 | FS-R | `.ipc_token` 읽기 (한 곳) | 불가피 |
| C2 | bootstrap.rs:14-46 | FS-R/CFG | `config.json`에서 `gui.ipcPort`·`gui.language` 읽기 | 불가피 |
| C2b | bootstrap.rs `live_port` | FS-R | `daemon.lock.owner`에서 실행 중 데몬의 pid·포트 읽기(설정 포트에 데몬이 없을 때만, `/health`의 pid로 확인) | 불가피(데몬을 찾는 단계) |
| C8 | i18n.rs:36-52 | FS-R | 설치 폴더 `locales/<lang>/cli.json` 읽기 (없으면 내장본) | |
| C9 | bootstrap.rs:49-90 | PROC | saba-core 분리 실행 | 불가피(부트스트랩) |
| C10 | bootstrap.rs:92, cmd/daemon.rs:102 | PROC | API 종료가 안 될 때 데몬 PID만 강제 종료 | 이름 기반 kill 아님 |
| C14 | cmd/chatbot.rs:109-114 | FS-R | `chatbot set KEY=@<파일>`이면 사용자가 지정한 파일 내용을 값으로 읽어 API로 보낸다 | 사용자 입력 편의, 데몬 파일 아님 |
| C15 | cmd/server.rs:297-305 | FS-R | `server add`에서 폴더 중복 검사용 `canonicalize` | 경로 정규화만 |

### 7.4 디스코드 봇
로컬 봇이 없어져 이 목록(B1~B11)은 사라졌다. 릴레이와의 통신은 데몬의 relay_agent만 한다(§4).

### 7.5 요약 판정
- **원칙 위반 정도가 가장 큰 항목**:
  - G2: GUI가 daemon 상태와 상관없이 config.json `gui`를 직접 읽는다. 쓰기(G3, G4)는 daemon이 응답하지 않을 때만 하므로 부트스트랩 범위로 줄었다.
- **부트스트랩상 불가피한 항목**(토큰 파일, ipcPort, daemon 최초 spawn): G1, G7, C1, C2, C9. daemon **종료**(G8, G11)는 이제 `/api/daemon/shutdown`을 먼저 쓰고, 강제 종료는 자기가 띄운 PID에만 한다(G12도 같다). 이름 기반 kill(G9)은 없어졌다.
- **daemon에 API가 없어서 우회하는 항목**: daemon 재기동(G10), 언인스톨러(G16). updater 실행(G14)은 daemon API로 옮겨졌다.
