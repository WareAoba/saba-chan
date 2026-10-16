# 04. 클라이언트 계층: GUI · CLI · Discord Bot (server-chan) · 공유 상수 · 로케일

> 이 문서는 코드에서 직접 도출했다. 기존 docs/README는 신뢰하지 않았고, 코드와 어긋나는 주석이나 문서는 각 항목에서 따로 표시했다.
> 기준 원칙(프로젝트 오너): **daemon(saba-core)이 서비스의 주인이며 모든 기능은 daemon을 거친다. 인터페이스(GUI/CLI)는 혼자서 아무것도 하면 안 되고, 파일 편집도 daemon API를 거쳐야 한다.** modules는 Python 게임 서버 매뉴얼, plugins(=extensions)는 Python으로 작성되어 인터페이스에 컴포넌트를 주입하고, server-chan(discord_bot)은 daemon과 짝을 이뤄 Discord 봇을 돌린다.
> 이 원칙을 어기는 지점은 모두 **§7 "daemon 우회 지점 전수 목록"**에 정리했다. 이 문서에서 가장 중요한 결과물이다.

Daemon 라우터의 기준 위치: `src/ipc/mod.rs:572-668`(인증 미들웨어 `auth::auth_middleware` 적용 구간), `src/ipc/updates.rs:83-93`(`/api/updates/*`, 인증 layer **이후** `.merge()`로 붙음, `src/ipc/mod.rs:669`).

---

## 1. GUI (saba-chan-gui, Electron)

### 1.1 Main process가 맡는 일 (`saba-chan-gui/main.js`, 3451줄)

| 책임 | 위치 | 설명 |
|---|---|---|
| userData 경로 고정 | main.js:25 | `app.setPath('userData', getSabaDataDir())`. Windows는 `%APPDATA%/saba-chan`, Unix는 `$HOME/.config/saba-chan`, `SABA_DATA_DIR`가 있으면 그 값을 쓴다 |
| IPC 토큰 인증 | main.js:51-195 | `.ipc_token` 파일을 **직접 읽어** `X-Saba-Token` 헤더에 넣는다. `http.request`를 monkey-patch해서(main.js:94-122) 127.0.0.1:ipcPort로 가는 모든 요청에 주입하고, axios 인터셉터(125-136)는 보조 수단이다. 401을 받으면 토큰 파일을 다시 읽고 1회 재시도한다(142-195). 단 `/api/relay/` 요청은 재시도하지 않는다 |
| **Daemon spawn** | `startDaemon()` main.js:507-604 | dev 모드는 `target/release/saba-core(.exe)`, prod 모드는 `getInstallRoot()/saba-core(.exe)`를 `--spawned` 인자로 실행한다. env는 `RUST_LOG=info, SABA_LANG, SABA_IPC_PORT, SABA_INSTANCES_PATH=<userData>/instances, SABA_MODULES_PATH`. stdout/stderr는 GUI 로그로 넘긴다 |
| Daemon 재사용/기동 판단 | `ensureDaemon()` main.js:907-986 | `GET /health`가 200이면 기존 daemon을 재사용한다. 401이면 토큰을 다시 읽으며 최대 3회 재시도한다. ECONNREFUSED/timeout이면 `startDaemon()`을 호출하고 500ms 간격으로 최대 8회 폴링한다 |
| Daemon 종료 | `stopDaemon()` main.js:607-674, `cleanQuit()` 824-904 | Windows는 `taskkill /PID … /F /T`, Unix는 SIGTERM을 보내고 2초 뒤 SIGKILL한다. 그 뒤 **프로세스 이름 기반 fallback**(`taskkill /IM saba-core.exe /F` 869, `pkill -f saba-core` 876)을 실행한다 |
| 클라이언트 등록/heartbeat | main.js:1014-1073 | `POST /api/client/register {kind:'gui'}`, 30초마다 `GET /api/ext-process/discord-bot/status`로 bot pid를 조회한 뒤 `POST /api/client/:id/heartbeat {bot_pid}`를 보낸다. 종료 시 `DELETE /api/client/:id/unregister[?shutdown=true]`를 호출한다 |
| install_root 동기화 | `syncInstallRoot()` main.js:989-1003 | `PUT /api/updates/config {install_root}` |
| 업데이트 백그라운드 체크 | main.js:1153-1268 | 3시간 주기로 `POST /api/updates/check`. 결과는 OS Notification과 renderer `updates:available` 이벤트로 알린다. 자동 다운로드는 없다("향후 구현 예정" 주석, 1223-1224) |
| **Updater exe spawn** | `updater:launchApply` main.js:2744-2777, `findUpdaterExe()` 1108-1150 | `saba-chan-updater.exe --apply --relaunch <gui exe>`를 **detached**로 실행한다(Windows는 `cmd /c start`로 Job Object를 회피, `spawnDetached` 766-787). 이후 `cleanQuit()`. 파일명이 `.exe`로 하드코딩되어 있어 Linux/macOS에서는 updater를 찾지 못한다 |
| **Installer spawn(언인스톨)** | `app:launchUninstaller` main.js:2891-2959 | 설치 경로를 직접 탐색하고, Windows는 `reg query HKCU\…\Uninstall\Saba-chan`도 본다(2920). 이후 `saba-chan-installer.exe --uninstall`을 detached로 실행한다 |
| **Mock release server spawn** | main.js:679-759 | `node scripts/mock-release-server.js --port 9876`(테스트/개발용). preload에 노출되어 있지만 renderer는 사용하지 않는다 |
| Discord bot 기동 | `discord:start` main.js:3183-3299 | **GUI가 bot을 직접 spawn하지는 않는다.** `POST /api/ext-process/discord-bot/start`로 daemon에게 node 프로세스 실행을 요청한다(§4.1) |
| 콘솔 팝아웃 창 | main.js:1865-1978 | 별도 `BrowserWindow`에 `?console-popout=<id>&name=` 쿼리를 붙여 같은 React 번들을 로드한다 |
| 트레이 | main.js:1366-1472 | 데몬 시작/종료, "인터페이스만 종료", "완전히 종료" 메뉴. 트레이 메뉴의 "데몬 시작"은 API가 아니라 `startDaemon()`을 직접 호출한다 |
| 종료 플로우 | main.js:1312-1361, 1352 | 창 close를 가로채 renderer에 `app:closeRequest`를 보내고, 선택 결과 `exit-interface`/`quit`/`cancel`을 받는다 |
| 로그 | main.js:223-282 | `<userData>/logs/saba-chan-<ts>.log`에 직접 쓴다 |
| 메인 프로세스 i18n | main.js:288-337 | `<installRoot>/locales/<lang>/common.json`을 직접 읽는다 |

**인증 방식 요약**: 데몬이 시작할 때 UUID 토큰을 만들어 `<dataDir>/.ipc_token`(Unix 0600)에 쓰고(`src/ipc/auth.rs:27-61`), 모든 클라이언트는 이 파일을 읽어 `X-Saba-Token`을 보낸다. `SABA_AUTH_DISABLED=1`이면 인증을 하지 않고, 토큰 파일이 없어도 통과한다(`auth.rs:82-101`). `/health`도 인증 대상이다(`mod.rs:574`가 layer 이전에 있음). **`/api/updates/*`는 auth layer 이후에 merge되므로 무인증으로 노출된다**(`mod.rs:667-669`). 이 부분은 daemon 쪽 이슈라 참고로만 적는다.

### 1.2 preload 채널 전수표 (`preload.js` → `ipcMain` → daemon endpoint)

`window.api.*`는 preload.js:3-138, `window.electron.*`는 preload.js:141-153에 있다. "Daemon 라우트 존재"는 `src/ipc/mod.rs`와 `updates.rs` 기준으로 대조했다.

| preload 함수 (preload.js 줄) | 채널 | main.js 줄 | 호출 대상 | 라우트 존재 |
|---|---|---|---|---|
| serverList (4) | server:list | 1608 | GET /api/servers (port_conflict_stops가 있으면 OS 알림) | ✓ 576 |
| serverStart (5) | server:start | 1670 | POST /api/server/:name/start | ✓ 578 |
| serverStop (6) | server:stop | 1708 | POST /api/server/:name/stop | ✓ 579 |
| serverStatus (7) | server:status | 1738 | GET /api/server/:name/status | ✓ 577 |
| moduleList (8) | module:list | 1980 | GET /api/modules | ✓ 580 |
| moduleRefresh (9) | module:refresh | 1999 | POST /api/modules/refresh | ✓ 581 |
| moduleGetMetadata (10) | module:getMetadata | 2066 | GET /api/module/:name | ✓ 585 |
| moduleGetLocales (11) | module:getLocales | 2056 | GET /api/module/:name/locales | ✓ 621 |
| moduleListVersions (12) | module:listVersions | 1767 | GET /api/module/:name/versions?… | ✓ 610 |
| moduleInstallServer (13) | module:installServer | 1780 | POST /api/module/:name/install | ✓ 612 |
| instanceGetInstalledVersion (14) | instance:getInstalledVersion | 1792 | GET /api/instance/:id/installed-version | ✓ 613 |
| moduleManifest (16) | module:manifest | 2024 | GET /api/modules/manifest | ✓ 582 |
| moduleInstallFromManifest (17) | module:installFromManifest | 2033 | POST /api/modules/manifest/:id/install | ✓ 583 |
| moduleRemove (18) | module:remove | 2046 | DELETE /api/modules/:id | ✓ 584 |
| instanceCreate (19) | instance:create | 2091 | POST /api/instances | ✓ 587 |
| instanceProvisionProgress (20) | instance:provisionProgress | 2122 | GET /api/provision-progress/:name | ✓ 591 |
| instanceDismissProvision (21) | instance:dismissProvision | 2133 | DELETE /api/provision-progress/:name | ✓ 591 |
| instanceCheckUpdate (22) | instance:checkUpdate | 2144 | GET /api/instance/:id/check-update | ✓ 593 |
| instanceApplyUpdate (23) | instance:applyUpdate | 2156 | POST /api/instance/:id/apply-update | ✓ 594 |
| instanceDelete (24) | instance:delete | 2168 | DELETE /api/instance/:id | ✓ 589 |
| instanceReorder (25) | instance:reorder | 2197 | PUT /api/instances/reorder {order} | ✓ 588 |
| instanceUpdateSettings (26) | instance:updateSettings | 2209 | PATCH /api/instance/:id | ✓ 589 |
| instanceResetProperties (27) | instance:resetProperties | 1801 | POST /api/instance/:id/properties/reset | ✓ 605 (**renderer 미사용**) |
| instanceResetServer (28) | instance:resetServer | 1814 | POST /api/instance/:id/server/reset | ✓ 606 |
| executeCommand (29) | instance:executeCommand | 2281 | GET /api/instance/:id 후 method별로 POST …/rcon, …/rest, …/command 중 하나 | ✓ 596-598 |
| extensionList…extensionInitStatus (31-44) | extension:* | 2408-2588 | /api/extensions, /:id/enable, /disable, /i18n/:locale, /gui, /gui/styles, /icon, /manifest, /:id/install, DELETE /:id, /updates, /rescan, /init-status | ✓ 641-656 (확장 UI 로딩은 다른 문서에서 다룸) |
| managedStart (46) | managed:start | 1829 | POST /api/instance/:id/managed/start | ✓ 600 |
| managedConsole (47) | managed:console | 1842 | GET /api/instance/:id/console?since&count | ✓ 601 |
| managedStdin (48) | managed:stdin | 1853 | POST /api/instance/:id/stdin | ✓ 602 |
| consolePopout/FocusPopout/Popin/ToggleAlwaysOnTop (50-53) | console:* | 1865-1978 | Electron 창 조작만 (daemon 호출 없음) | n/a |
| updaterCheck (61) | updater:check | 2591 | POST /api/updates/check | ✓ updates.rs:86 |
| updaterStatus (62) | updater:status | 2687 | GET /api/updates/status | ✓ |
| updaterDownload (63) | updater:download | 2698 | POST /api/updates/download | ✓ |
| updaterDownloadProgress (64) | updater:downloadProgress | 2709 | GET /api/updates/download/progress | ✓ |
| updaterApply (65) | updater:apply | 2720 | POST /api/updates/apply | ✓ |
| updaterLaunchApply (66) | updater:launchApply | 2744 | **updater exe 직접 spawn** | n/a |
| updaterGetConfig (67) | updater:getConfig | 2780 | GET /api/updates/config | ✓ |
| updaterSetConfig (68) | updater:setConfig | 2790 | **POST** /api/updates/config | ✗ **daemon은 GET/PUT만 제공(updates.rs:91-92). 405가 나서 설정 변경과 SabaStorage의 Mock 토글이 동작하지 않는다** |
| mockServerStart/Stop/Status (70-72) | mockServer:* | 679-759 | `node` 프로세스 직접 spawn | n/a (renderer 미사용) |
| settingsLoad (79) | settings:load | 2859 | `daemonProcess`가 있으면 GET /api/config/gui, 없으면 **settings.json 직접 읽기** | ✓ 617 |
| settingsSave (80) | settings:save | 2965 | `daemonProcess`가 있으면 GET 후 merge해서 PUT /api/config/gui, 없으면 **settings.json 직접 쓰기** | ✓ 617 |
| settingsGetPath (81) | settings:getPath | 2988 | 로컬 경로 문자열 | n/a |
| getFixedPaths (83) | paths:getFixed | 2992 | 로컬 경로 계산 | n/a (renderer 미사용) |
| openFileDialog / openFolderDialog (85-86) | dialog:* | 3044/3073 | Electron 네이티브 다이얼로그 | n/a |
| migrationScanDir (88) | migration:scanDir | 3085 | POST /api/fs/scan-dir | ✓ 625 |
| discordBotStatus (90) | discord:status | 3149 | GET /api/ext-process/discord-bot/status | ✓ 630 |
| discordBotStart (91) | discord:start | 3183 | PUT /api/config/bot, **PATCH {relayUrl}/api/hosts/:hostId/bot-config(외부 직접)**, POST /api/node-env/setup, POST /api/ext-process/discord-bot/start | ✓ 615/659/628 |
| discordBotStop (92) | discord:stop | 3301 | POST /api/ext-process/discord-bot/stop | ✓ 629 |
| discordGuildMembers / Channels (93-94) | discord:guildMembers/Channels | 3159/3171 | POST …/discord-bot/stdin 후 GET …/discord-bot/console 폴링(`__IPC__:` 응답 찾기) | ✓ 631-632 |
| botConfigLoad (99) | botConfig:load | 3311 | GET /api/config/bot | ✓ 615 |
| botConfigSave (100) | botConfig:save | 3368 | GET 후 merge해서 PUT /api/config/bot (**await 없음**, 3397), cloud면 relay에 직접 PATCH(3406) | ✓ |
| syncGuiConfig (102) | guiConfig:sync | 2961 | **no-op**(`syncGuiConfigToDaemon` 501-503 본문이 비어 있음) | 죽은 채널 |
| saveNodeToken / loadNodeToken (104-105) | nodeToken:* | 3316/3320 | PUT/GET /api/config/node-token | ✓ 619 |
| relayCheckHostStatus… relayPollPairingStatus (107-111) | relay:* | 2620-2685 | GET /api/relay/host/:id/status, /nodes, GET /api/relay/node/:guild/members, POST /api/relay/pair/initiate, GET /api/relay/pair/:code/status | ✓ 661-665 |
| shellOpenPath (113) | shell:openPath | 3343 | Electron `shell.openPath` | n/a |
| logsGetPath / logsOpenFolder (115-116) | logs:* | 3325/3359 | 로컬 | n/a (renderer 미사용) |
| launchUninstaller (118) | app:launchUninstaller | 2891 | installer 직접 spawn | n/a |
| getAppVersion (119) | app:getVersion | 2872 | `app.getVersion()` | n/a |
| getComponentInfo (120) | app:getComponentInfo | 2876 | GET /api/system/components | ✓ 623 |
| closeResponse (124) | app:closeResponse (send) | 1352 | 종료 플로우 | n/a |
| daemonStatus (132) | daemon:status | 2810 | GET /health | ✓ 574 |
| daemonRestart (133) | daemon:restart | 2820 | **자기가 spawn한 daemon 프로세스를 kill한 뒤 `startDaemon()`.** API(`/api/daemon/shutdown`)를 거치지 않는다 | — |
| minimize/maximize/closeWindow (135-137) | window:* | 3428-3451 | 창 조작 | n/a |
| electron.getLanguage/setLanguage/getSystemLanguage (146-148) | language:* | 3000-3041 | 로컬 settings.json 읽기/쓰기. 언어가 바뀌면 bot을 정지시키고(`POST …/discord-bot/stop`) renderer에 `bot:relaunch`를 보낸다 | 부분 |
| electron.shell.openExternal (151) | shell:openExternal | 3330 | 외부 URL 열기 | n/a |

Main에서 renderer로 가는 push 이벤트: `status:update`(로딩 단계), `updates:available`, `updates:completed`, `bot:error`, `bot:relaunch`, `console:popoutOpened/Closed/popinRequest`, `app:closeRequest`.

**renderer(src/, 테스트 제외)에서 쓰지 않는 preload API**: `instanceResetProperties`, `mockServerStart/Stop/Status`, `offUpdatesAvailable`, `offUpdateCompleted`, `getFixedPaths`, `offBotError`, `syncGuiConfig`, `logsGetPath`, `logsOpenFolder`, `getSystemLanguage`. 반대로 **renderer가 호출하지 않는 daemon 라우트**: `/api/instance/:id/validate`, `/properties`(GET/PUT), `/accept-eula`, `/diagnose`, `/api/module/:name/version/:version`, `/api/extensions/:id/mount|unmount|config`, `/api/daemon/shutdown`, `/api/daemon/console`, `/api/ext-processes`, `/api/updates/integrity`, `/api/node-env/status`. 이 중 일부는 CLI만 사용한다(§3.4).

### 1.3 GUI가 읽고 쓰는 설정 파일

| 파일 | 경로 | 접근 방식 | 스키마 |
|---|---|---|---|
| settings.json | `<dataDir>/settings.json` (main.js:432-435) | `loadSettings()`(437-477)는 **항상 파일을 직접 읽는다.** `saveSettings()`(480-498)는 `daemonProcess`가 null이면 파일에 직접 쓰고, 아니면 `PUT /api/config/gui` | 기본값(main.js:452-462): `autoRefresh, refreshInterval(2000), windowBounds{width,height,x,y}, language, ipcPort(57474), consoleBufferSize(2000), autoGeneratePasswords, portConflictCheck`. renderer가 추가로 저장하는 키(useSettingsStore.js:79-98): `accentColor, accentSecondary, useGradient, fontScale, enableTransitions, consoleSyntaxHighlight, consoleBgColor, consoleTextColor, sidebarCompact, consoleFontScale`. 레거시 키 `discordToken/discordAutoStart`는 App.js:418-436에서 bot-config로 마이그레이션한다 |
| bot-config.json | daemon이 관리(`/api/config/bot`) | GUI는 API만 사용 | `prefix, token, autoStart, moduleAliases, commandAliases, musicEnabled, musicChannelId, musicUISettings{queueLines,refreshInterval}, nodeSettings{<guildId>:{allowedInstances[], memberPermissions{<userId>:{<serverId>:[deniedCmd]}}}}, mode('local'/'cloud'), cloud{relayUrl,hostId}, cloudNodes[], cloudMembers{}` (main.js:3200-3217, 3376-3406) |
| node token | `/api/config/node-token` | API | `{token: "sbn_<nodeId>.<secret>"}` |
| .ipc_token | `<dataDir>/.ipc_token` 또는 `SABA_TOKEN_PATH` | 직접 읽기(필수 부트스트랩) | 평문 UUID |
| logs | `<dataDir>/logs/*.log` | 직접 쓰기 | — |
| renderer localStorage | Chromium 저장소 | 직접 | `i18nextLng`(i18n.js:100-139, SettingsModal.js:249), 테마 키(themeManager.js:50-120), 공지 목록(NoticeModal.js:15-25) |

**핵심 결함: `daemonProcess` 기준 분기.** `settings:load/save`(2860, 2967)와 `saveSettings`(483)는 "daemon이 떠 있는가"가 아니라 **"이 GUI가 daemon을 spawn했는가"**(`daemonProcess !== null`)로 분기한다. 그래서 daemon이 이미 떠 있던 경우에는 daemon이 살아 있어도 **GUI가 settings.json을 직접 읽고 쓴다.** 이런 경우는 CLI가 daemon을 띄웠을 때, "인터페이스만 종료" 후 GUI를 다시 열었을 때, `ensureDaemon`에서 기존 daemon을 재사용했을 때에 해당한다. daemon의 ConfigStore 캐시와 어긋날 수 있다.

### 1.4 화면과 모달별 기능 (`src/`)

- **App.js**(1110줄): 루트 컴포넌트. `ExtensionProvider`로 감싸고, `useServerActions`가 `refreshInterval` 주기로 `serverList`를 폴링한다. 종료 질문 모달, 로딩 화면(`status:update`), Discord bot 자동 시작(`useDiscordStore`, `autoStart`가 있을 때), 언어 변경 시 `bot:relaunch` 처리를 맡는다.
- **ServerCard.js**: 인스턴스 카드. 상태, 시작/정지, 콘솔, 설정, 명령 버튼과 REST host:port 표시가 있다. 게임 서버 업데이트 확인/적용(`instanceCheckUpdate/ApplyUpdate` 45-72), 네이티브 프로비저닝 진행률(`instanceProvisionProgress` 100, `NativeProvision.js`), 팝아웃 콘솔 포커스(442)를 제공한다. `useDragReorder`로 카드를 드래그해 순서를 바꾸면 `instanceReorder`가 호출된다.
- **ConsoleView / ConsoleDock / ConsoleWindow**(useConsole, useMultiConsole): managed 프로세스 콘솔. `managedConsole` 폴링, `managedStdin`, `executeCommand`를 쓰고, PiP 팝아웃과 always-on-top을 지원한다.
- **AddInstanceModal**: 진입점이 둘이다.
  - `AddInstanceNewServer.js`: 게임 모듈 카드 그리드에서 모듈을 고른 뒤 이름과 옵션을 입력하고 `instanceCreate`를 호출한다.
  - `AddInstanceMigration.js`: 폴더 선택 → `migrationScanDir`(daemon `/api/fs/scan-dir`) → 모듈 자동 감지(`dir_signatures`). 실패하면 `moduleManifest`에서 수동으로 고른다.
- **ServerSettingsModal.js + useServerSettings.js**: 모듈 `settings.fields` 스키마로 필드를 렌더링하고(`instanceUpdateSettings`), 버전 선택과 설치(`moduleListVersions`, `moduleInstallServer`, `instanceGetInstalledVersion`), 서버 리셋(`instanceResetServer`)을 제공한다. 인스턴스별 bot alias와 권한 편집도 여기서 하는데, bot-config을 직접 load/save한다(515-592).
- **CommandModal.js**: 모듈 `commands.fields` 기반 명령 힌트, 자동완성, 입력 폼. `executeCommand`(107)로 실행한다.
- **SettingsModal.js**: 탭은 `general / appearance / extensions / advanced`(335-353)이다. 버전과 컴포넌트 정보(`getComponentInfo`), 언어, 테마, IPC 포트, 언인스톨(`launchUninstaller`)이 있고, SabaStorage로 들어가는 진입점이 있다.
- **SabaStorage.js**(구 업데이트 센터, 1280줄): 탭은 `components / modules / extensions`(61-70)이다.
  - components 탭: updater check/download/progress/apply/launchApply. `discord_bot` 컴포넌트를 갱신할 때는 bot을 정지했다가 다시 시작한다(297-330).
  - modules 탭: 설치된 모듈과 `saba-chan-modules` 레지스트리(`moduleManifest`, `moduleInstallFromManifest`, `moduleRemove`).
  - extensions 탭: 확장 레지스트리.
  - devMode에서는 Mock 릴리스 서버 토글(196-215)을 제공한다. 이 토글은 `updaterSetConfig`를 쓰므로 실제로는 405로 실패한다.
- **DiscordBotModal.js**(1851줄): local/cloud 모드, 토큰, prefix, module/command alias, 음악(활성화, 전용 채널 `discordGuildChannels`, UI 설정), 노드(길드)별 `allowedInstances`와 멤버별 차단 명령(`memberPermissions`, 멤버 목록은 local이면 `discordGuildMembers`, cloud면 `relayListNodeMembers`)을 다룬다. Cloud 페어링은 `relayInitiatePairing`으로 시작해 `relayPollPairingStatus`로 진행 상황을 확인하고 `saveNodeToken`으로 마무리한다. 기본 relay는 `https://saba-chan.online`(12)이다. side-panel 모드가 있다.
- **BackgroundModal.js**: daemon 상태(`daemonStatus`)와 재시작(`daemonRestart`).
- **NoticeModal.js**: 앱 내 공지와 알림 목록(localStorage). 업데이트 모달로 연결된다.
- 공용 모달(Modals.js): Success/Failure/Notification/Question. TitleBar(frameless 창 컨트롤), Toast, CustomDropdown, Icon, ExtensionSlot(확장 UI 주입 지점. 세부 내용은 다른 문서).
- 상태 저장소(zustand): `useServerStore`, `useSettingsStore`(설정 변경 후 500ms debounce로 자동 저장, 160-172), `useDiscordStore`(bot 상태, 설정, 자동 시작), `useUIStore`(모달 표시).
- 테스트(`src/test/`): gui-e2e, settings-roundtrip, store-unit, discord-side-panel, console-window, theme-contrast. 모두 `window.api`를 mock한다(setupTests.js).

---

## 2. 공유 상수와 포트 (`shared/constants.js`)

| 상수 | 값 | 사용처 |
|---|---|---|
| APP_NAME | `saba-chan` | dataDir 이름 |
| GITHUB_OWNER/REPO/MODULES_REPO/EXTENSIONS_REPO | `WareAoba` / `saba-chan` / `saba-chan-modules` / `saba-chan-extensions` | JS 쪽에서는 거의 참조하지 않는다. SabaStorage.js:208-210이 `'WareAoba'`, `'saba-chan'`을 **리터럴로 중복 정의**한다 |
| DEFAULT_IPC_PORT | `57474` | main.js:18-20, useSettingsStore, bot |
| DEFAULT_DAEMON_URL | `http://127.0.0.1:57474` | |
| SUPPORTED_LANGUAGES | en, ko, ja, zh-CN, zh-TW, es, pt-BR, ru, de, fr | main.js. **중복 정의 위치**: `saba-chan-gui/src/i18n.js:36`, `discord_bot/i18n.js:10`, `discord_bot/utils/constants.js:46` |
| getSabaDataDir() | `SABA_DATA_DIR`, Windows는 `%APPDATA%/saba-chan`, 그 외는 `$HOME/.config/saba-chan` | |

- Rust 쪽 SSOT는 `updater/src/constants.rs`이고(constants.js:7 주석), CLI는 `saba_chan_updater_lib::constants`를 사용한다.
- `discord_bot/utils/constants.js`는 `../../shared`를 먼저 찾고 실패하면 `../shared`를 찾는 프록시다. 둘 다 없으면 상수를 **인라인으로 복제**해 쓴다(24-50).
- 그 밖의 포트:
  - mock release server 9876 (main.js:683, SabaStorage.js:202)
  - Vite dev 5173 (main.js:1320)
  - Palworld REST 기본 8212가 **bot 코어에 하드코딩**되어 있다(discord_bot/core/ipc.js:165). 모듈 중립 원칙에 어긋난다
  - Relay 기본 URL `https://saba-chan.online` (main.js:3251, DiscordBotModal.js:12)

---

## 3. CLI (saba-chan-cli, Rust TUI)

### 3.1 구조
- `main.rs`(15줄): `config::get_ipc_base_url()`로 포트를 정한다. 이 값은 **settings.json을 로컬에서 직접 읽어** `ipcPort`를 꺼낸다(config.rs:99-107). 그다음 `DaemonClient`를 만들고 `tui::run`을 호출한다. 서브커맨드 방식의 일회성 CLI는 없고 **TUI 전용**이다(TUI 안에 명령 입력줄이 있다).
- `client.rs`: async reqwest 클라이언트(기본 5초, long 120초). 토큰 파일을 읽어 `X-Saba-Token`을 보내고, 401이면 토큰을 다시 읽어 1회 재시도한다(67-104).
- `process.rs`: blocking. daemon/bot 수명주기를 관리한다.
- `config.rs`: daemon `/api/config/*`를 감싼 blocking 래퍼.
- `cli_config.rs`: CLI 전용 `cli-settings.json`.
- `module_registry.rs`: alias 매핑.
- `i18n.rs`: `locales/<lang>/cli.json`.
- `tui/`: app/mod/render/commands/screens.

### 3.2 TUI 화면 (`tui/app.rs:74-97`)
Dashboard, Servers, ServerDetail, ServerConsole, ServerSettings, ServerProperties, Modules, ModuleDetail, ModuleManifest, Bot, BotAliases, Settings, Updates, Daemon, Extensions, ExtensionList, ExtensionDetail, ExtensionManifest, CommandMode, CreateInstanceStep1/2.

명령 입력줄의 상위 명령(`tui/commands.rs`): `help, daemon start|stop|restart|status, bot start|stop|status|token|prefix|mode|relay|node-token, server/instance list|start|stop|status|create|delete|reorder|…, module list|refresh|versions|install|manifest|install-manifest|remove, extension list|enable|disable|install|remove|rescan, update check|status|download|apply|config|set|install|launch-apply, config get|set|reset|show, exit/quit/q`. 모듈 단축 명령도 받는다(`palworld start`, `팰월드 시작` 등. `module_registry`가 module.toml의 alias를 해석한다).

시작 시퀀스(`tui/mod.rs:85-140`):
1. 2초 뒤 `POST /api/client/register {kind:'cli'}`를 1회 시도하고, 성공하면 30초 주기로 heartbeat를 보낸다. **등록에 실패하면 다시 시도하지 않는다.** auto-start로 daemon이 늦게 뜨면 CLI는 등록되지 않은 채로 남는다.
2. `cli-settings.auto_start`가 켜져 있으면 1초 뒤 daemon이 없을 때 `process::start_daemon`을 실행하고, bot의 autoStart가 켜져 있으면 `start_bot`도 실행한다.
3. `exit`를 입력하면 **항상 bot과 daemon을 모두 정지한다**(commands.rs:35-60). GUI의 "인터페이스만 종료"에 해당하는 선택지가 없다.

### 3.3 CLI가 호출하는 daemon endpoint (`client.rs`)
대부분 GUI와 같다. CLI에만 있는 호출과 존재하지 않는 호출은 아래와 같다.

| 메서드 (client.rs 줄) | Endpoint | 상태 |
|---|---|---|
| validate_instance (253) | POST /api/instance/:id/validate | ✓ 603 (CLI 전용) |
| read/write_properties (258/263) | GET/PUT /api/instance/:id/properties | ✓ 604 (CLI 전용) |
| accept_eula (268), diagnose (273) | POST …/accept-eula, …/diagnose | ✓ 607-608 (CLI 전용) |
| get_version_details (305) | GET /api/module/:name/version/:v | ✓ 611 |
| get/save_extension_config (616/621) | /api/extensions/:id/config | ✓ 652 |
| shutdown_daemon (577), get_daemon_console (582) | /api/daemon/shutdown, /api/daemon/console | ✓ 638-639 |
| list_ext_processes (539) | GET /api/ext-processes | ✓ 627 |
| check_update_integrity (604) | GET /api/updates/integrity | ✓ |
| get_node_env_status (594) | GET /api/node-env/status | ✓ 658 |
| **get_install_status (384)** | GET /api/install/status | ✗ **라우트 없음** |
| **run_install (389)** | POST /api/install/run | ✗ **라우트 없음** (commands.rs:1130 `update install`) |
| **install_component (398)** | POST /api/install/component/:key | ✗ **라우트 없음** (commands.rs:1125) |
| **get_install_progress (406)** | GET /api/install/progress | ✗ **라우트 없음** |
| **set_update_config (505)** | **POST** /api/updates/config | ✗ **daemon은 PUT만 제공. 405** (commands.rs:1119, tui/mod.rs:907) |
| discord_guild_list (655), discord_guild_members (671) | **https://discord.com/api/v10 직접 호출** | dead code (`#[allow(dead_code)]` impl 30). 호출하는 곳이 없다 |

`src/`와 `updater/` 전체를 grep해도 `api/install`은 없다. 즉 CLI의 `update install …` 명령은 반드시 404가 난다.

### 3.4 config.rs / gui_config.rs / cli_config.rs
- **config.rs**: "모든 설정은 daemon API를 경유한다"(1-7)는 원칙을 따르는 blocking 래퍼다. `load_settings`는 `GET /api/config/gui`, `save_settings`는 `PUT`, `load/save_bot_config`는 `/api/config/bot`을 쓰고, node-token은 GET/PUT/DELETE를 쓴다.
  - CLI가 **GUI 설정(settings.json)을 편집하는 이유**: `ipcPort, language, autoRefresh, refreshInterval, consoleBufferSize, autoGeneratePasswords, portConflictCheck`가 GUI와 CLI, daemon이 공유하는 "앱 설정"이기 때문이다(config.rs:263-343. TUI Settings 화면과 `config set`에서 쓴다). 파일 이름은 `gui`지만 실질적으로는 전역 설정이다.
  - 예외: `get_ipc_port_local()`(99-107)은 settings.json을 직접 읽는다. 순환을 피하기 위한 부트스트랩으로 문서화되어 있다. `get_modules_path`/`get_extensions_path`(129-147, 345-363)는 디렉터리를 **로컬에서 직접 생성**한다. `scan_directory`(427-463)와 `get_module_locales`(468-500)는 daemon 호출이 실패하면 **로컬 파일시스템으로 fallback**한다.
- **gui_config.rs**: config.rs와 거의 같은 사본이다(diff 결과 `get_ipc_base_url`이 `get_ipc_port()`를 쓰는지 `get_ipc_port_local()`을 쓰는지만 다르다). **`main.rs`에 `mod gui_config;`가 없어서 컴파일되지 않는 죽은 파일**이다. 테스트도 돌지 않는다.
- **cli_config.rs**: `<dataDir>/cli-settings.json`을 `fs::read_to_string`과 `fs::write`로 **직접** 읽고 쓴다(52-68). 키는 `language`(비어 있으면 GUI 설정을 따름), `auto_start`(기본 true), `refresh_interval`(초, 기본 2), `bot_prefix`이다. daemon API에는 대응하는 저장소가 없다.

### 3.5 process.rs: CLI가 직접 하는 프로세스 관리
- `start_daemon`(192-231): `find_daemon_binary()`로 바이너리를 찾아 `--spawned`와 env `RUST_LOG, SABA_MODULES_PATH, SABA_INSTANCES_PATH, SABA_LANG`을 넘기고 **detached spawn**한다(spawn_detached 159-174). `SABA_IPC_PORT`는 넘기지 않는다. GUI(main.js:563)와 다른 점이다.
  - 버그: `SABA_LANG`은 `config::get_language()`에서 오는데(204), 이 함수는 daemon API를 호출한다. 그런데 이 시점에는 daemon이 아직 없어서 **항상 "en"으로 fallback**된다.
- `stop_daemon`(234-260): `POST /api/daemon/shutdown`을 보내고, 실패하면 `force_kill_daemon`으로 **`taskkill /IM saba-core.exe /F` 또는 `pkill -f saba-core`**를 실행한다(267-290). 프로세스 이름 기반이라 같은 이름의 프로세스를 모두 죽인다.
- `check_daemon_running`(98-106): TCP connect로 확인한다.
- `start_bot`(380-412): `POST /api/node-env/setup`으로 node 경로를 받고, `POST /api/ext-process/discord-bot/start`에 `{command:node, args:[index.js], cwd, env}`를 보낸다.
  - 버그: **Discord 토큰을 `settings.discordToken`에서 읽는다**(process.rs:325). GUI는 토큰을 bot-config의 `token`에 저장하고, settings에 남아 있던 값은 마이그레이션으로 옮긴다(App.js:418-436). 따라서 GUI로 설정한 사용자는 CLI `bot start`에서 "Discord token not set" 오류를 받는다. 같은 CLI의 `config::get_discord_token()`(config.rs:118)은 bot-config에서 읽어 서로 모순된다.
  - **cloud 모드를 지원하지 않는다**: `RELAY_URL`과 `RELAY_NODE_TOKEN`을 넘기지 않고 항상 `DISCORD_TOKEN`을 넘긴다(build_bot_env 347-360).
- `launch-apply`(commands.rs:1134-1165): updater exe를 직접 spawn한다. 인자는 `--apply --install-root <root> [targets]`이다. GUI는 `--apply --relaunch <gui>`를 쓴다. 두 클라이언트가 updater에 다른 인자 규약을 쓰므로 updater 쪽 문서와 대조해야 한다.

### 3.6 module_registry.rs
`try_load_from_daemon`(70-…)은 `GET /api/modules`를 호출해 alias 맵을 만든다. 실패하면 **`<modules>/*/module.toml`을 직접 읽어 파싱한다**(235-275 부근). 토큰 파일도 따로 직접 읽는다(76-79).

### 3.7 GUI 대비 기능 격차 (CLI가 못 하는 것과 CLI만 하는 것)

| 기능 | GUI | CLI |
|---|---|---|
| 인스턴스 CRUD, 시작/정지, 콘솔, 명령, 설정 | ✓ | ✓ |
| 마이그레이션(폴더 스캔 후 감지) | ✓ | 부분 (scan_directory만 있고 자동 감지 플로우는 없다) |
| Cloud 모드 bot 기동 | ✓ | ✗ (env 미전달) |
| Cloud 페어링(relay/pair/*), 노드 및 멤버 목록 | ✓ | ✗ (`bot relay url/hostid`로 설정만 편집하고, relay API는 호출하지 않는다) |
| 노드별 allowedInstances, memberPermissions 편집 | ✓ | ✗ |
| 음악 채널 설정 | ✓ | 화면 키(`screen.bot_music`)만 있다 |
| 확장 GUI 번들 주입 | ✓ | ✗ (구조상 해당 없음) |
| properties 직접 편집, EULA 동의, diagnose, validate | ✗ | ✓ |
| daemon 콘솔 로그 보기, daemon API shutdown | ✗ | ✓ |
| 업데이트 integrity 검사 | ✗ | ✓ |
| "인터페이스만 종료" | ✓ | ✗ (exit하면 daemon까지 종료) |

---

## 4. Discord Bot (discord_bot/, "server-chan")

### 4.1 기동 경로
bot을 **직접 spawn하는 클라이언트는 없다.** GUI와 CLI 모두 daemon의 범용 외부 프로세스 API를 쓴다.

```
POST /api/ext-process/discord-bot/start
{ command: <node path from POST /api/node-env/setup or "node">,
  args: ["<installRoot>/discord_bot/index.js"], cwd: "<…>/discord_bot",
  env: { IPC_BASE, SABA_LANG, BOT_CONFIG_PATH, SABA_EXTENSIONS_DIR,
         DISCORD_TOKEN            (local)
         | RELAY_URL, RELAY_NODE_TOKEN (cloud) },
  meta: { mode } }
```
(GUI는 main.js:3238-3269, CLI는 process.rs:347-412. 요청 구조체는 `src/ipc/handlers/ext_process.rs:144-157`.)

- GUI는 bot 디렉터리를 `path.join(__dirname,'..','discord_bot')`로 정한다(main.js:3238). 패키징된 앱에서 `__dirname`이 asar 내부일 수 있어 경로가 깨질 가능성이 있다. CLI는 `current_exe()` 기준이다.
- **`BOT_CONFIG_PATH`는 bot 코드 어디에서도 읽지 않는다**(grep 결과 0건). bot은 설정을 `GET /api/config/bot`으로 받는다(core/ipc.js:179-182). 이 env는 죽은 값이다. `discord_bot/bot-config.json`(16줄, `hostId: "soKGFF7t62XT"` 포함) 역시 pkg asset에만 포함되고 런타임에서는 읽지 않는 잔존물이다.
- 3초 뒤 상태를 확인해 정지되어 있으면, console 버퍼에서 에러 줄을 골라 renderer에 `bot:error`로 보낸다(main.js:3272-3292).
- 종료: GUI `discord:stop`, 언어 변경(main.js:3018), `before-quit`(main.js:1552-1571)에서 정지 요청을 보낸다.
  - **버그**: `before-quit`의 bot 정지 요청은 블록 스코프 밖의 `currentPort`(1536에서 `const`로 선언됨)를 1559에서 참조한다. 그래서 ReferenceError가 나고 try/catch에 삼켜져 **요청이 전송되지 않는다.** 실제 정리는 `cleanQuit`의 `unregister?shutdown=true`에 의존한다.

### 4.2 Daemon과의 통신 (`core/ipc.js`)
- base는 `IPC_BASE` env이고 기본값은 57474이다. 토큰은 `SABA_TOKEN` env 또는 `.ipc_token`을 직접 읽는다(24-48). 401이면 다시 읽고 재시도한다(71-108).
- 사용하는 endpoint:
  - `GET /api/servers`, `GET /api/modules`, `GET /api/module/:name`(→ `.toml`), `GET /api/config/bot`
  - 시작: `POST /api/instance/:id/managed/start` 또는 `POST /api/server/:name/start {module, config:{}}`. 어느 쪽을 쓸지는 `determineUseManaged`가 instance의 `module_settings.managed_start`나 모듈의 `interaction_mode=='console'`로 정한다(processor.js:438-446).
  - 정지: `POST /api/server/:name/stop {force:false}`(35초 timeout).
  - 명령: `POST /api/instance/:id/stdin`, `/rcon`, `/rest`, `/command`.
  - `sendRestCommand`는 `rest_username/rest_password`를 기본값 `'admin'`과 `''`로 채워 보낸다(ipc.js:158-169). 자격증명을 bot이 지정하는 셈이다.
- GUI와 bot 사이 양방향 IPC(local 모드만): GUI가 `POST /api/ext-process/discord-bot/stdin {message: JSON}`을 보내면, bot은 stdin 한 줄을 JSON으로 파싱해(index.js:204-228) `getGuildMembers`/`getGuildChannels`/`shutdown`을 처리하고 stdout에 `__IPC__:{json}`을 쓴다. GUI는 `GET …/console`을 500ms 간격으로 폴링해 `id`가 맞는 줄을 찾는다(main.js:3105-3147).
  - 취약점: console 응답은 기본 최근 200줄이고(`ext_process.rs:419-423`), GUI는 `lastLineCount-5`부터만 스캔한다. 버퍼가 200줄로 찬 뒤에는 폴링 사이에 로그가 6줄 이상 들어오면 응답을 놓쳐 TIMEOUT이 날 수 있다.

### 4.3 명령 체계 (prefix 기반 메시지 명령. **slash command는 없다**)
- 진입점은 `messageCreate`에서 `processor.process`로 이어진다(index.js:232).
- prefix 기본값은 `!saba`이고 bot-config `prefix`로 바꿀 수 있다. `checkMultiGuildBlock`이 있어서 **local 모드에서 bot이 2개 이상의 길드에 들어가 있으면 모든 명령을 거부한다**(processor.js:27-35).
- 처리 순서(processor.js:76-164):
  0. 전용 음악 채널이면 prefix 없이 처리한다.
  1. `resolver.reloadConfigIfChanged()`: 마지막 로드 후 5초가 지났으면 `/api/config/bot`을 다시 읽는다. 주석(resolver.js:57)은 "1분"이라고 되어 있지만 코드는 5000ms다.
  2. 확장 파이프라인(core/handler.js:16-39): `Music:Shortcut`(prefix 뒤에 YouTube URL이나 음악 명령) → `Music` → `EasterEgg`(easter_eggs.js 트리거 `할건해야제`, `갈래말래` 등) → `RPS`(트리거 `가위바위보/rps/ㄱㅂㅂ`, 버튼 UI).
  3. 빈 입력, `help`, `도움`이면 도움말(모듈 목록과 alias)을 보여준다.
  4. `list`, `목록`이면 서버 목록을 보여준다(`nodeSettings[guild].allowedInstances`로 필터링).
  5. `<모듈 alias>`만 오면 그 모듈의 명령 도움말을 보여준다.
  6. `<모듈 alias> <명령 alias> [args…]`이면 `memberPermissions` 차단 목록을 확인한 뒤 내장 `start/stop/status`이거나 module.toml에 정의된 명령을 method(rcon/rest/dual/기본)에 맞춰 호출한다. 정의되지 않은 명령은 raw로 stdin이나 rcon에 보낸다(processor.js:506-536).
- Alias는 `utils/aliasResolver.js`가 module.toml의 alias와 bot-config `moduleAliases/commandAliases`를 합친다. 충돌을 검사하며, 충돌하면 `bot:errors.alias_conflict`를 응답한다.
- i18n(`discord_bot/i18n.js`): 시작할 때 `locales/<lang>/{common,bot}.json`을 **파일로 직접 읽는다**. `lng`는 `SABA_LANG`이고 defaultNS는 `bot`이다. 언어를 바꾸면 프로세스를 재시작해야 한다(GUI가 `bot:relaunch`로 처리).
- `resolver.ensureGuildMetadata`(resolver.js:129-131)는 **본문이 빈 stub**이다. 길드별 메타데이터 API(`ipc.getModules(guildId)`의 인자)도 무시된다.

### 4.4 음악 확장 (`extensions/music.js` 3114줄 + `musicChannelUI.js` 487줄)
- 의존성: `@discordjs/voice`, opusscript/sodium-native, play-dl(검색과 메타데이터), yt-dlp(스트림), ffmpeg.
  - 의존성 해석: 먼저 `.deps-resolved.json`(SABA_EXTENSIONS_DIR/music, dataDir/extensions/music)을 읽는다. 없으면 **격리된 Python(`<installRoot>/python-env` 또는 `.venv`)으로 `music_deps.py check_dependencies`를 `spawnSync`로 직접 실행한다**(music.js:50-127). 확장 디렉터리의 `node_modules`를 `NODE_PATH`에 주입하고(292-300), `FFMPEG_PATH` env를 직접 바꾼다.
- 명령(`MUSIC_COMMANDS` 607-623): `play, search, pause, resume, skip, stop, queue, np, volume, shuffle, loop, loopoff, radio(자동재생/추천), normalize(loudnorm 2-pass), help`.
  - 범용 alias(629-645): `p, find, s/next, leave/disconnect/dc, q/list, nowplaying/now, vol/v, random, repeat, unloop, autoplay, norm/loudnorm`. 모듈 alias는 `music`, `dj`. 언어별 alias는 `bot:music_commands`에서 가져온다.
  - 바로가기: prefix 뒤에 YouTube URL만 붙여도 재생한다.
  - `musicEnabled === false`이면 모두 비활성화된다(849, 928, 2956).
- 기능: 길드별 큐, 볼륨(기본 0.5), 5분 idle 시 자동 퇴장, 음성 채널이 비면 퇴장(`handleVoiceStateUpdate`), 다음 곡 prefetch, radio 모드(시드 기반 추천 5곡 보충), loudnorm 정규화(`I=-14:TP=-1:LRA=11`), 오류 분류(연령 제한, 지역 차단, 저작권, 도구 미설치 등).
  - 버튼 인터랙션: `music_vol_down/up, prev, next, pause_resume, radio, normalize`(2260-2348).
- **전용 음악 채널 UI**(musicChannelUI.js): 지정한 텍스트 채널 하나를 점유한다. 진행바, 큐, 버튼을 3~5초마다 갱신하고, prefix 없이 검색어를 받으며, 응답은 10초 뒤 지운다. `musicChannelId`가 바뀌면 resolver 콜백으로 다시 초기화한다(index.js:305-312).
- relay(cloud) 모드에서 들어온 mock 메시지에는 `member`/`guild`가 없어서 음악 명령을 처리할 수 없다(`isRelayMessage` 802-804).

### 4.5 Relay/Cloud 모드: 정확한 프로토콜 (`core/relayAgent.js`)
`RELAY_URL`과 `RELAY_NODE_TOKEN`이 모두 있으면 **Discord에 로그인하지 않는 "relay agent" 모드**로 동작한다(index.js:24, 156-187). Discord 게이트웨이는 server-chan relay 서버가 맡고, 로컬 bot은 명령 실행기 역할만 한다.

- 토큰 형식은 `sbn_<nodeId>.<secret>`이다(relayAgent.js:43-47). 오류 메시지(358)에는 `<hostId>`라고 되어 있어 표기가 맞지 않는다.
- **모든 요청 서명**(58-73):
  ```
  Authorization: Bearer <RELAY_NODE_TOKEN>
  X-Saba-Client: 1
  x-request-timestamp: <unix sec>
  x-request-nonce: <uuid>
  x-request-signature: hex(HMAC-SHA256(secret,
        METHOD \n urlPath \n ts \n nonce \n JSON(body)|''))
  ```
- 엔드포인트:
  1. `GET {RELAY}/info`(서명 없음): `{version, minAgentVersion}`. agent 버전(package.json `0.1.1`)이 최소 버전보다 낮으면 시작을 중단한다(101-132).
  2. `POST {RELAY}/heartbeat` body `{agentVersion, os, metadata:{servers, modules, moduleDetails:{}}}`. 60초 주기다(136-181). 응답의 `sleeping:true`면 폴링을 멈추고, `warning:'UPDATE_REQUIRED'`면 경고만 남긴다. heartbeat는 매번 daemon의 `/api/servers`와 `/api/modules` 결과 **전체**를 relay로 업로드한다.
  3. `GET {RELAY}/poll`(롱폴링): 204면 명령 없음이고, 200이면 `{commands:[{id, payload:{action:'raw_command', text}, requestedBy, guildId, channelId}], sleeping?}`이다. 실패하면 3초에서 60초까지 지수 백오프한다. sleeping 상태면 30초 간격으로 대기한다(257-339).
  4. 받은 명령마다 `createMockMessage`(prefix와 text를 붙인 가짜 discord.js Message)를 만들어 `processor.process`로 **local 모드와 같은 처리 체인**을 태운다. 마지막 reply 텍스트를 결과로 쓴다(185-233).
  5. `POST {RELAY}/result/:requestId` body `{success, data:{text}|{error}}`(237-253).
- GUI 쪽 cloud 지원: 페어링, 노드 목록, 멤버 목록은 daemon `/api/relay/*` 프록시를 거친다. 그러나 **bot-config 동기화(`PATCH {relayUrl}/api/hosts/:hostId/bot-config {prefix, moduleAliases, commandAliases}`)는 GUI main이 relay에 직접 axios로 호출한다**(main.js:3216, 3406). daemon에 이 프록시가 없기 때문이다. preload.js:106의 "all relay comms go through daemon" 주석은 코드와 맞지 않는다.

### 4.6 테스트
`discord_bot/test/`: `music.test.js`(107 케이스), `integration.test.js`(31), `ipc-timeout.test.js`(9). jest.
`package.json`의 `"discord.js": "^14.0.0"`이고 lockfile은 14.25.1이다. `client.once('clientReady')`(index.js:291)는 14.22 이상에서만 발생하는 이벤트이므로, 범위 하한 `^14.0.0`은 실제 요구사항과 맞지 않는다.

---

## 5. 로케일 구조 (`locales/<lang>/{common,gui,bot,cli}.json`, 10개 언어)

| 파일 | 최상위 키(en) | 소비자 |
|---|---|---|
| common.json (leaf 43개) | app_name, success/error/…, daemon, modules, server, network, port_conflict 등 21개 | GUI renderer(`common` ns), **GUI main 프로세스**(main.js:294-310, 알림과 status 메시지), bot(`common` ns) |
| gui.json (en leaf 707개) | title, menu, title_bar, modals, settings_modal, add_instance_modal, migration_modal, command_modal, server_*, discord_bot, discord_modal, notice_modal, updates, extensions, saba_storage, validation, context_menu 등 29개 | GUI renderer(`gui` ns) |
| bot.json (leaf 184개) | commands, help, modules, list, status, server, command, responses, errors, messages, cloud, relay, music, music_commands, music_ui, rps | bot(defaultNS `bot`). GUI renderer도 import한다(i18n.js:3-33. DiscordBotModal의 음악 alias 표시용) |
| cli.json (en leaf 200개) | welcome, status, help, *_cmd, menu, screen 등 16개 | CLI `i18n.rs`. `<root>/locales` 또는 실행 파일 옆의 `locales`를 런타임에 읽는다(64-84) |

- GUI renderer는 10개 언어의 common, gui, bot JSON을 **빌드 시 정적으로 import**한다(i18n.js:3-33). 언어 소스는 `window.electron.getLanguage()`가 1순위, localStorage `i18nextLng`가 2순위다(92-139). 확장 i18n은 daemon `/api/extensions/:id/i18n/:locale`에서 받는다.
- **번역 누락**(en 기준 diff):
  - gui.json: de, es, fr, pt-BR, ru, zh-CN, zh-TW는 691개로 **16개가 빠져 있다**(`console.dock_focus/dock_restore/minimize/pin/popin/unpin`, `discord_modal.music_channel_*`, `discord_modal.music_normalize`). ja는 697개다.
  - cli.json: ko를 제외한 8개 언어가 168개로 **32~33개가 빠져 있다**(`screen.bot_*`, `screen.ext_*`, `screen.settings_*`, `screen.update_*` 등).
- 하드코딩된 한국어 문자열이 i18n을 거치지 않는 곳:
  - main.js의 트레이 메뉴(1427-1468), 업데이트 알림(1196-1197), 오류 매핑(2226-2241, 2373-2400), `preloadLightData`(1009-1010)
  - processor.js의 `defaultValue`(371)
  - easter_eggs 트리거

---

## 6. 불일치, 죽은 코드, stub, 버그 (file:line)

### 6.1 존재하지 않는 endpoint나 잘못된 HTTP 메서드 호출
1. `saba-chan-gui/main.js:2792`: `updater:setConfig`가 `axios.post('/api/updates/config')`를 호출한다. daemon은 GET/PUT만 제공하므로(updates.rs:91-92) 405가 난다. 영향을 받는 곳은 SabaStorage의 자동 체크 on/off(201)와 Mock 토글(196-215)이다.
2. `saba-chan-cli/src/client.rs:505-506`: `set_update_config`도 POST를 쓴다. 같은 405이고 `update set`과 Updates 화면이 영향을 받는다.
3. `saba-chan-cli/src/client.rs:384-407`: `/api/install/status|run|component/:key|progress` 4개는 daemon과 updater 어디에도 라우트가 없다. `update install` 명령이 실패한다(commands.rs:1125-1133).

### 6.2 죽은 코드와 stub
- `saba-chan-cli/src/gui_config.rs` 전체(1086줄): `mod` 선언이 없어 컴파일되지 않는 config.rs 사본이다.
- `saba-chan-cli/src/client.rs:655-684`: Discord REST를 직접 호출하는 함수 2개. 호출하는 곳이 없다.
- `saba-chan-gui/main.js:501-503` `syncGuiConfigToDaemon`(no-op)과 이를 부르는 `guiConfig:sync` 채널(2961), `runBackgroundInit`의 호출(1091-1094).
- `saba-chan-gui/main.js:1005-1011` `preloadLightData`: 상태 메시지만 보낸다(주석에서 레거시라고 인정함).
- `discord_bot/core/resolver.js:129-131` `ensureGuildMetadata` 빈 stub. `ipc.getModules(guildId)`의 인자도 무시된다.
- `BOT_CONFIG_PATH` env(main.js:3242, process.rs:358, 364)를 읽는 코드가 없다. `discord_bot/bot-config.json`은 런타임에서 쓰이지 않는다.
- renderer가 쓰지 않는 preload API 12개(§1.2). `mockServer:*` 핸들러 3개.
- main.js:1223-1224: auto_download/auto_apply는 "향후 구현 예정" 상태다.
- `discord_bot/core/relayAgent.js:410-418` `getStatus`: export만 되어 있고 사용하는 곳이 없다.

### 6.3 기능 버그
- **before-quit bot 정지 요청 누락**: main.js:1536(`const currentPort`가 if와 try 블록 안에 선언됨)과 1559(블록 밖에서 참조) 때문에 ReferenceError가 나고 조용히 무시된다.
- **SabaStorage bot 재시작 실패**: SabaStorage.js:316, 445가 `settings?.discordToken`을 읽는다. 이 키는 bot-config으로 옮겨져 비어 있으므로 업데이트 후 bot 재시작이 항상 "no token"으로 끝난다. cloud 모드 정보도 넘기지 않는다.
- **CLI bot start 토큰 소스 오류**: process.rs:325는 `settings.discordToken`을 읽고, config.rs:118은 `botConfig.token`을 읽는다. cloud 모드는 지원하지 않는다(347-360).
- **CLI가 띄운 daemon 언어가 항상 en**: process.rs:213이 daemon 기동 전에 daemon API로 언어를 조회한다.
- **CLI daemon 기동 시 `SABA_IPC_PORT` 미전달**: process.rs:216-221. 사용자 지정 포트를 쓰면 daemon이 기본 포트로 뜨고 CLI는 지정 포트로 접속하게 된다. daemon이 settings에서 포트를 직접 읽는지는 daemon 문서와 대조해야 한다.
- **settings 경로 분기 기준 오류**: main.js:483, 2860, 2967이 `daemonProcess`(내가 spawn했는가)로 판단한다. daemon이 살아 있는데도 settings.json을 직접 쓰는 경우가 생긴다(§1.3).
- **botConfig:save 비동기 누락**: main.js:3397에서 `saveBotConfig`를 await하지 않는다. 실패해도 `{success:true}`를 반환한다. 또 `cloudNodes`/`cloudMembers`가 입력에 없으면 `[]`/`{}`로 **덮어써서 지워진다**(3392-3396). 같은 파일의 `discord:start`(3205-3206)는 기존 값을 보존한다.
- **CLI 등록 재시도 없음**: tui/mod.rs:93-111.
- **resolver 주석과 코드 불일치**: resolver.js:57(1분)과 60(5초).
- **Palworld 기본값 하드코딩**: discord_bot/core/ipc.js:164-167(`8212`, `admin`).
- **updater exe 이름 하드코딩**: main.js:1113-1148, commands.rs:1137-1142에서 `.exe`로 고정되어 있어 비 Windows 환경에서 launch-apply가 불가능하다.

---

## 7. ★ Daemon 우회 지점 전수 목록 (GUI · CLI · discord_bot)

> 기준: "인터페이스는 혼자서 아무것도 못 한다. 파일 편집도 daemon API를 거친다."
> 분류: **[FS-R]** 직접 파일 읽기 / **[FS-W]** 직접 파일·디렉터리 쓰기 / **[PROC]** 프로세스 spawn·kill / **[EXT]** 외부 서비스 직접 호출 / **[CFG]** 설정 파일 직접 편집 / **[OS]** OS 레지스트리·셸
> "불가피" 표시는 부트스트랩상 daemon 없이는 할 수 없는 항목이다. 그래도 목록에는 포함했다.

### 7.1 GUI main process (`saba-chan-gui/main.js`)
| # | 위치 | 분류 | 내용 | 비고 |
|---|---|---|---|---|
| G1 | 62, 160 | FS-R | `.ipc_token` 읽기 | 불가피(인증 부트스트랩) |
| G2 | 437-477 (443-444) | FS-R/CFG | `loadSettings()`는 daemon 상태와 상관없이 **항상** settings.json을 직접 읽는다. `getLanguage`, `createWindow`, `refreshIpcBase`, `language:get` 등 모든 소비자가 이 함수를 쓴다 | ipcPort 부트스트랩만 불가피하고, 나머지는 API로 대체할 수 있다 |
| G3 | 483-494 (488) | FS-W/CFG | `saveSettings()`: `daemonProcess`가 null이면 settings.json을 직접 쓴다. windowBounds 저장(1297), `language:set`→`setLanguage`(3005, 428-432), `settings:save` fallback(2981-2985)이 이 경로를 탄다 | §6.3 분기 버그와 결합됨 |
| G4 | 2859-2869 | FS-R/CFG | `settings:load` 로컬 fallback | 위와 같음 |
| G5 | 223-234 | FS-W | 로그 디렉터리 생성과 로그 파일 쓰기 | GUI 자체 로그 |
| G6 | 288-311 | FS-R | `locales/<lang>/common.json` 직접 읽기 | |
| G7 | 507-604 (570) | PROC | **saba-core spawn** (`--spawned`, env 주입) | 불가피(daemon 부재 시 부트스트랩) |
| G8 | 607-674 (634) | PROC | daemon kill (`taskkill /PID /F /T`, SIGTERM→SIGKILL) | API `/api/daemon/shutdown` 대신 OS kill을 쓴다 |
| G9 | 844-878 (854, 869, 876) | PROC | cleanQuit에서 강제 kill과 **이름 기반 `taskkill /IM saba-core.exe` 및 `pkill -f saba-core`** | 다른 사용자나 다른 인스턴스의 daemon까지 죽일 수 있다 |
| G10 | 2820-2856 | PROC | `daemon:restart`: 프로세스 kill 후 startDaemon | API를 쓰지 않는다 |
| G11 | 1443-1456 | PROC | 트레이 메뉴의 데몬 시작/종료 | |
| G12 | 1597-1604 | PROC | `process.on('exit')`에서 daemon SIGKILL | |
| G13 | 679-759 (694) | PROC | `node scripts/mock-release-server.js` spawn | 개발용 |
| G14 | 766-787, 2744-2777 (2769) | PROC | **updater exe detached spawn**(`--apply --relaunch`) | 업데이트 적용은 daemon `/api/updates/apply`로 하고, 실행 파일 교체는 클라이언트가 updater를 띄워서 한다 |
| G15 | 1108-1150 | FS-R | updater exe 경로 탐색(`existsSync`) | |
| G16 | 2891-2959 (2920, 2945) | OS/PROC | `reg query HKCU\…\Uninstall\Saba-chan`, installer `--uninstall` spawn | |
| G17 | 3216, 3406 | EXT | **relay 서버 `PATCH {relayUrl}/api/hosts/:hostId/bot-config` 직접 호출**(daemon 프록시 없음) | preload.js:106 주석과 모순 |
| G18 | 3330-3366 | OS | `shell.openExternal`, `shell.openPath`(로그 폴더, 인스턴스 폴더) | UI 성격 |
| G19 | 3044-3083 | OS | 네이티브 파일/폴더 다이얼로그 | UI 성격(경로 선택만 하고, 실제 스캔은 daemon) |
| G20 | 94-122 | — | `http.request` 전역 monkey-patch | 우회는 아니지만 모든 Node HTTP에 영향을 준다 |

### 7.2 GUI renderer (`saba-chan-gui/src`)
| # | 위치 | 분류 | 내용 |
|---|---|---|---|
| R1 | i18n.js:100-139, SettingsModal.js:249 | FS-W(localStorage) | 언어 캐시 `i18nextLng` |
| R2 | utils/themeManager.js:50-120 | FS-W(localStorage) | 테마와 커스텀 테마. settings.json에도 accent 계열을 저장하므로 저장소가 둘로 갈린다 |
| R3 | Modals/NoticeModal.js:15-25 | FS-W(localStorage) | 공지와 알림 기록 |
| R4 | i18n.js:3-33 | 빌드타임 | 로케일 JSON 정적 번들. 런타임 locale 업데이트(daemon의 Locales 컴포넌트 자동 적용, main.js:1172-1176)가 renderer에는 반영되지 않는다 |

(renderer는 `fetch`, `axios`, 직접 IP 접근을 하지 않는다. 모두 preload를 거친다.)

### 7.3 CLI (`saba-chan-cli/src`)
| # | 위치 | 분류 | 내용 | 비고 |
|---|---|---|---|---|
| C1 | config.rs:16-22, client.rs(`read_ipc_token`), process.rs:296-301, module_registry.rs:76-79 | FS-R | `.ipc_token` 읽기(4곳에 중복 구현) | 불가피 |
| C2 | config.rs:99-107 | FS-R/CFG | settings.json에서 `ipcPort` 직접 읽기 | 불가피(문서화됨) |
| C3 | config.rs:129-147, 345-363 | FS-W | modules/extensions 디렉터리 `create_dir_all` | daemon 책임이어야 한다 |
| C4 | config.rs:427-463 | FS-R | `scan_directory` 로컬 fallback(`read_dir`) | |
| C5 | config.rs:468-500 | FS-R | `get_module_locales` 로컬 fallback(모듈 locales 직접 읽기) | |
| C6 | module_registry.rs:235-275 부근 | FS-R | daemon이 실패하면 `<modules>/*/module.toml` 직접 파싱 | |
| C7 | cli_config.rs:52-68 | FS-R/FS-W/CFG | `cli-settings.json` 직접 읽기/쓰기 | daemon에 대응하는 저장소가 없다 |
| C8 | i18n.rs:64-90 | FS-R | `locales/<lang>/cli.json` 직접 읽기 | |
| C9 | process.rs:192-231 (215-223) | PROC | **saba-core detached spawn** | 불가피(부트스트랩) |
| C10 | process.rs:262-290 | PROC | **`taskkill /IM saba-core.exe /F` 및 `pkill -f saba-core`**(API shutdown이 실패했을 때) | 이름 기반 강제 종료 |
| C11 | commands.rs:1134-1165 (1150) | PROC | updater exe 직접 spawn(`--apply --install-root`) | |
| C12 | client.rs:655-684 | EXT | Discord REST(`discord.com/api/v10`) 직접 호출 | dead code |
| C13 | commands.rs:35-60 | PROC(간접) | exit 시 bot과 daemon 무조건 정지 | 정책 문제 |

### 7.4 discord_bot (server-chan)
| # | 위치 | 분류 | 내용 | 비고 |
|---|---|---|---|---|
| B1 | core/ipc.js:24-48, 82 | FS-R | `.ipc_token` 직접 읽기 | 불가피 |
| B2 | i18n.js:6-32 | FS-R | `locales/*/{common,bot}.json` 직접 읽기 | |
| B3 | utils/constants.js:16-52 | — | shared 상수를 찾지 못하면 인라인 복제 사용 | SSOT 위반 위험 |
| B4 | index.js:192-333 | EXT | Discord Gateway 로그인(discord.js) | 역할상 정당(server-chan 본연의 일) |
| B5 | core/relayAgent.js:103, 151, 240, 269 | EXT | **server-chan relay(`/info`, `/heartbeat`, `/poll`, `/result/:id`) 직접 HTTP 호출**. daemon `/api/relay/*` 프록시를 쓰지 않는다 | daemon의 servers와 modules 전체를 relay에 업로드한다(140-148) |
| B6 | extensions/music.js:50-127 (92) | PROC | 격리 Python으로 `music_deps.py check_dependencies` **spawnSync**. 결과 파일 `.deps-resolved.json`이 만들어진다(FS-W, Python 쪽) | 의존성 설치는 확장/daemon 책임이어야 한다 |
| B7 | extensions/music.js:129-190, 143-144 | FS-R | `.deps-resolved.json`, 확장 `package.json`, `node_modules` 존재 확인과 직접 읽기 | |
| B8 | extensions/music.js:292-300, 214-269 | ENV | `process.env.NODE_PATH`, `FFMPEG_PATH` 전역 변경(cleanup 3100-3111에서 복원) | |
| B9 | extensions/music.js:313-360, 1178-, 1274-1550 | PROC | `ffmpeg`, `yt-dlp` spawn(검증, 메타데이터, 스트림, 2-pass loudnorm) | 기능 본질상 필요 |
| B10 | extensions/music.js:1090-1170 | EXT | play-dl을 통한 YouTube 검색과 정보 조회 | 기능 본질상 필요 |
| B11 | core/ipc.js:158-169 | — | REST 자격증명 기본값(`admin`/`''`, 포트 8212)을 bot이 주입 | daemon과 모듈의 책임 침범 |

### 7.5 요약 판정
- **원칙 위반 정도가 가장 큰 항목**:
  - G2, G3, G4: GUI가 settings.json을 직접 읽고 쓴다. 분기 기준이 잘못되어 daemon이 살아 있어도 이 경로를 탄다.
  - G17: GUI가 relay에 직접 PATCH한다.
  - C7: CLI 전용 설정 파일이 daemon 밖에 있다.
  - C3~C6: CLI의 로컬 fallback.
  - B5: bot이 relay와 직접 통신한다. daemon에 relay 프록시가 있는데도 쓰지 않는다.
  - B6: bot이 의존성 설치용 Python을 직접 실행한다.
- **부트스트랩상 불가피한 항목**(토큰 파일, ipcPort, daemon 최초 spawn): G1, G7, C1, C2, C9, B1. 다만 daemon **종료**(G8~G12, C10)는 `/api/daemon/shutdown`이 있으므로 API로 대체할 수 있다. 이름 기반 kill(G9, C10)은 특히 위험하다.
- **daemon에 API가 없어서 우회하는 항목**: relay bot-config PATCH(G17), relay agent 통신(B5), CLI 설정 저장소(C7), updater 실행(G14, C11), 언인스톨러(G16).
