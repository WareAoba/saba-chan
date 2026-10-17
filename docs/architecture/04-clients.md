# 04. 클라이언트 계층: GUI · CLI · Discord Bot (server-chan) · 공유 상수 · 로케일

> 이 문서는 코드에서 직접 도출했다. 기존 docs/README는 신뢰하지 않았고, 코드와 어긋나는 주석이나 문서는 각 항목에서 따로 표시했다.
> 기준 원칙(프로젝트 오너): **daemon(saba-core)이 서비스의 주인이며 모든 기능은 daemon을 거친다. 인터페이스(GUI/CLI)는 혼자서 아무것도 하면 안 되고, 파일 편집도 daemon API를 거쳐야 한다.** modules는 Python 게임 서버 매뉴얼, plugins(=extensions)는 Python으로 작성되어 인터페이스에 컴포넌트를 주입하고, server-chan(discord_bot)은 daemon과 짝을 이뤄 Discord 봇을 돌린다.
> 이 원칙을 어기는 지점은 모두 **§7 "daemon 우회 지점 전수 목록"**에 정리했다. 이 문서에서 가장 중요한 결과물이다.

Daemon 라우터의 기준 위치: `src/ipc/mod.rs:572-668`(인증 미들웨어 `auth::auth_middleware` 적용 구간), `src/ipc/updates.rs:83-93`(`/api/updates/*`, 인증 layer **이후** `.merge()`로 붙음, `src/ipc/mod.rs:669`).

---

## 1. GUI (saba-chan-gui, Electron)

> 2026-10-17 기준 Electron 44, electron-builder 26, Vite 6.4, Vitest 5. 빌드에는 Node `>=22.12`가 필요하다(`saba-chan-gui/package.json` `engines`, 커밋 4ba8d26).

### 1.1 Main process가 맡는 일 (`saba-chan-gui/main.js`, 3451줄)

| 책임 | 위치 | 설명 |
|---|---|---|
| userData 경로 고정 | main.js:25 | `app.setPath('userData', getSabaDataDir())`. Windows는 `%APPDATA%/saba-chan`, Unix는 `$HOME/.config/saba-chan`, `SABA_DATA_DIR`가 있으면 그 값을 쓴다 |
| IPC 토큰 인증 | main.js:51-195 | `.ipc_token` 파일을 **직접 읽어** `X-Saba-Token` 헤더에 넣는다. `http.request`를 monkey-patch해서(main.js:94-122) 127.0.0.1:ipcPort로 가는 모든 요청에 주입하고, axios 인터셉터(125-136)는 보조 수단이다. 401을 받으면 토큰 파일을 다시 읽고 1회 재시도한다(142-195). 단 `/api/relay/` 요청은 재시도하지 않는다 |
| **Daemon spawn** | `startDaemon()` main.js:507-604 | dev 모드는 `target/release/saba-core(.exe)`, prod 모드는 `getInstallRoot()/saba-core(.exe)`를 `--spawned` 인자로 실행한다. env는 `RUST_LOG=info, SABA_LANG, SABA_IPC_PORT, SABA_INSTANCES_PATH=<userData>/instances, SABA_MODULES_PATH`. stdout/stderr는 GUI 로그로 넘긴다 |
| Daemon 재사용/기동 판단 | `ensureDaemon()` main.js:907-986 | `GET /health`가 200이면 기존 daemon을 재사용한다. 401이면 토큰을 다시 읽으며 최대 3회 재시도한다. ECONNREFUSED/timeout이면 `startDaemon()`을 호출하고 500ms 간격으로 최대 8회 폴링한다 |
| Daemon 종료 | `stopDaemon()` main.js:607-674, `cleanQuit()` 824-904 | Windows는 `taskkill /PID … /F /T`, Unix는 SIGTERM을 보내고 2초 뒤 SIGKILL한다. 그 뒤 **프로세스 이름 기반 fallback**(`taskkill /IM saba-core.exe /F` 869, `pkill -f saba-core` 876)을 실행한다 |
| 클라이언트 등록/heartbeat | main.js:1014-1073 | `POST /api/client/register {kind:'gui'}`, 이후 주기적으로 `POST /api/client/:id/heartbeat`를 보낸다(봇 pid 조회는 로컬 봇과 함께 없어졌다). 종료 시 `DELETE /api/client/:id/unregister[?shutdown=true]`를 호출한다 |
| install_root 동기화 | `syncInstallRoot()` main.js:989-1003 | `PUT /api/updates/config {install_root}` |
| 업데이트 백그라운드 체크 | main.js:1153-1268 | 3시간 주기로 `POST /api/updates/check`. 결과는 OS Notification과 renderer `updates:available` 이벤트로 알린다. 자동 다운로드는 없다("향후 구현 예정" 주석, 1223-1224) |
| **Updater exe spawn** | `updater:launchApply` main.js:2744-2777, `findUpdaterExe()` 1108-1150 | `saba-chan-updater.exe --apply --relaunch <gui exe>`를 **detached**로 실행한다(Windows는 `cmd /c start`로 Job Object를 회피, `spawnDetached` 766-787). 이후 `cleanQuit()`. 파일명이 `.exe`로 하드코딩되어 있어 Linux/macOS에서는 updater를 찾지 못한다 |
| **Installer spawn(언인스톨)** | `app:launchUninstaller` main.js:2891-2959 | 설치 경로를 직접 탐색하고, Windows는 `reg query HKCU\…\Uninstall\Saba-chan`도 본다(2920). 이후 `saba-chan-installer.exe --uninstall`을 detached로 실행한다 |
| **Mock release server spawn** | main.js:679-759 | `node scripts/mock-release-server.js --port 9876`(테스트/개발용). preload에 노출되어 있지만 renderer는 사용하지 않는다 |
| 디스코드 연결 | `discord:start/stop/status` main.js:2805-2833 | GUI는 봇을 띄우지 않는다. 데몬의 릴레이 에이전트를 `/api/relay/agent{,/start,/stop}`로 켜고 끈다(§4) |
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
| discordBotStatus / Start / Stop (96-98) | discord:status/start/stop | 2805-2833 | GET /api/relay/agent, POST /api/relay/agent/start·stop. 노드 토큰이 없으면 `cloud_token_not_found` | ✓ |
| botConfigLoad (99) | botConfig:load | 3311 | GET /api/config/bot | ✓ 615 |
| botConfigSave (101) | botConfig:save | 2898-2908 | PUT /api/config/bot(최상위 키 단위 병합). 릴레이 동기화는 에이전트 하트비트가 맡는다 | ✓ |
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
| electron.getLanguage/setLanguage/getSystemLanguage (146-148) | language:* | 3000-3041 | 로컬 settings.json 읽기/쓰기. 언어를 저장하고 번역을 다시 읽는다. 데몬은 재시작하지 않고, 로컬 봇 재시작 단계는 없어졌다(qcza9f main.js:2737-2747) | 부분 |
| electron.shell.openExternal (151) | shell:openExternal | 3330 | 외부 URL 열기 | n/a |

Main에서 renderer로 가는 push 이벤트: `status:update`(로딩 단계), `updates:available`, `updates:completed`, `bot:error`, `bot:relaunch`, `console:popoutOpened/Closed/popinRequest`, `app:closeRequest`.

**renderer(src/, 테스트 제외)에서 쓰지 않는 preload API**: `instanceResetProperties`, `mockServerStart/Stop/Status`, `offUpdatesAvailable`, `offUpdateCompleted`, `getFixedPaths`, `offBotError`, `syncGuiConfig`, `logsGetPath`, `logsOpenFolder`, `getSystemLanguage`. 반대로 **renderer가 호출하지 않는 daemon 라우트**: `/api/instance/:id/validate`, `/properties`(GET/PUT), `/accept-eula`, `/diagnose`, `/api/module/:name/version/:version`, `/api/extensions/:id/mount|unmount|config`, `/api/daemon/shutdown`, `/api/daemon/console`, `/api/ext-processes`, `/api/updates/integrity`, `/api/node-env/status`. 이 중 일부는 CLI만 사용한다(§3.4).

### 1.3 GUI가 읽고 쓰는 설정 파일

| 파일 | 경로 | 접근 방식 | 스키마 |
|---|---|---|---|
| settings.json | `<dataDir>/settings.json` (main.js:432-435) | `loadSettings()`(437-477)는 **항상 파일을 직접 읽는다.** `saveSettings()`(480-498)는 `daemonProcess`가 null이면 파일에 직접 쓰고, 아니면 `PUT /api/config/gui` | 기본값(main.js:452-462): `autoRefresh, refreshInterval(2000), windowBounds{width,height,x,y}, language, ipcPort(57474), consoleBufferSize(2000), autoGeneratePasswords, portConflictCheck`. renderer가 추가로 저장하는 키(useSettingsStore.js:79-98): `accentColor, accentSecondary, useGradient, fontScale, enableTransitions, consoleSyntaxHighlight, consoleBgColor, consoleTextColor, sidebarCompact, consoleFontScale`. 레거시 키 `discordToken/discordAutoStart`는 App.js:418-436에서 bot-config로 마이그레이션한다 |
| bot-config.json | daemon이 관리(`/api/config/bot`) | GUI는 API만 사용 | `prefix, cloud{relayUrl,hostId}, moduleAliases, commandAliases, instanceAliases{<인스턴스ID>:"별명, 별명"}, nodeSettings, cloudNodes[], cloudMembers{}`. 로컬 봇 필드(`token`, `autoStart`, `mode`, `music*`)는 없다(01 §4 BotConfig) |
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
  - components 탭: updater check/download/progress/apply/launchApply.
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
| DEFAULT_IPC_PORT | `57474` | main.js:18-20, useSettingsStore, bot |
| DEFAULT_DAEMON_URL | `http://127.0.0.1:57474` | |
| SUPPORTED_LANGUAGES | en, ko, ja, zh-CN, zh-TW, es, pt-BR, ru, de, fr | main.js. **중복 정의 위치**: `saba-chan-gui/src/i18n.js:36` |
| getSabaDataDir() | `SABA_DATA_DIR`, Windows는 `%APPDATA%/saba-chan`, 그 외는 `$HOME/.config/saba-chan` | |

- Rust 쪽 SSOT는 `updater/src/constants.rs`이고(constants.js:7 주석), CLI는 `saba_chan_updater_lib::constants`를 사용한다.
- 그 밖의 포트:
  - mock release server 9876 (main.js:683, SabaStorage.js:202)
  - Vite dev 5173 (main.js:1320)
  - Relay 기본 URL `https://saba-chan.online` (main.js:3251, DiscordBotModal.js:12)

---

## 3. CLI (saba-chan-cli, Rust)

> 이 절은 CLI 리메이크(PR #6, 브랜치 `claude/cli-remake-b1wszk`) 기준이다. 예전 TUI(`tui/`, `process.rs`, `config.rs`, `gui_config.rs`, `cli_config.rs`, `module_registry.rs`)는 이 리메이크에서 모두 지워졌다.

### 3.1 쓰는 방법 세 가지
- **한 번 실행하는 하위 명령**: clap 기반(`cli.rs`). `status`, `start|stop|restart <서버>`, `console`, `run <서버> <게임 명령>`, `server …`(list, info, add, remove, settings, set, properties, rcon, rest, update, history, validate, diagnose, accept-eula, rename 등), `daemon …`, `module …`, `extension …`, `bot …`, `update …`, `config …`, `chat <문장>`, `version`, `completions <셸>` (`cli.rs:40-100`).
- **대화형 셸**: 인자 없이 터미널에서 실행하면 열린다(`main.rs:66`, `shell.rs`). 한 줄이 명령 하나이고 Tab으로 명령·서버 이름·게임 명령을 채운다. 첫 단어가 명령이 아니면 디스코드와 같은 채팅 처리기로 보낸다(`shell.rs:1-5, 183`). 셸을 닫아도 데몬과 서버는 계속 돈다.
- **스크립트용 `--json`**: stdout에는 JSON 문서 하나만 나가고, 안내와 진행 표시는 stderr로 간다(`output.rs:1-6`). 종료 코드: 0 성공, 1 실패, 2 사용법·모호함, 3 데몬 응답 없음, 4 대상 없음, 130 취소(`error.rs:40-46`). 전역 옵션은 `--json`, `--yes`, `--lang`, `--port`, `--no-color`.

### 3.2 구조
- `client.rs`: 데몬 HTTP 클라이언트. 토큰 파일을 읽어 `X-Saba-Token`을 보내고, 401이면 토큰을 다시 읽어 1회 재시도한다(`client.rs:28-34, 95-110`). 업데이터 실행도 데몬 API로 요청한다(`launch_updater` `:555`).
- `bootstrap.rs`: 데몬이 뜨기 전에 필요한 것만 직접 한다. `config.json`의 `gui.ipcPort`·`gui.language` 읽기(우선순위 `SABA_IPC_PORT` > 파일 > 기본값, `:18-28`), `saba-core` 실행 파일 찾기와 분리 실행(`:49-90`), 정상 종료가 안 될 때 **그 PID만** 강제 종료(`:92`).
- `ctx.rs`: 데몬이 필요한 명령 앞에서 `ensure_daemon`이 데몬을 확인하고 없으면 띄운다(`:21, 40-60`).
- `catalog.rs`: 게임 명령을 하드코딩하지 않는다. `GET /api/modules`의 `commands`·`command_aliases`와 `GET /api/instance/:id/commands`로 목록·사용법·입력 칸을 만든다. 인자 검사의 최종 판단은 데몬이 한다.
- `slots.rs`: 익스텐션 manifest의 `cli.slots`(데이터만, 코드 실행 없음). 지원 자리는 `InstanceList.badge`, `InstanceDetail.status`, `CreateInstance.options`.
- `i18n.rs`: 설치 폴더의 `locales/<lang>/cli.json`을 먼저 읽고, 없으면 바이너리에 넣어 둔 en/ko를 쓴다(`:11-12, 36-45`).
- `cmd/`: 명령 갈래마다 파일 하나(bot, config, console, daemon, extension, module, server, status, update).

사용자 안내서는 `saba-chan-cli/README.md`에 있다.

## 4. 디스코드 연동 (데몬 쪽)

> 로컬 Discord 봇 실행(`discord_bot/`, Node, ext-process)은 레거시이며 L1(7b5102f)에서 지웠다. 디스코드 봇 자체는 클라우드 릴레이에서 돌고, 그 쪽(게이트웨이·음악·챗봇)은 지금 server-chan에서 다시 만드는 중이라 여기서 다루지 않는다. 이 절은 데몬 쪽의 안정된 계약만 적는다.

### 4.1 구조

```
Discord ─ 릴레이의 사바쨩 봇 ─(롱폴링, HMAC 서명)─ 데몬 relay_agent ─ 채팅 처리기(crate::chat) ─ 데몬 내부 API
```

- 데몬이 먼저 릴레이로 나가서 연결한다. 데몬은 들어오는 연결을 받지 않는다.
- 페어링으로 받은 노드 토큰(`sbn_<id>.<secret>`)이 있어야 에이전트가 뜬다. 페어링은 `/api/relay/pair/initiate`, `/api/relay/pair/:code/status`(01 §4 Relay 프록시)로 하고, 토큰은 `/api/config/node-token`으로 다룬다.

### 4.2 릴레이 에이전트 (`src/relay_agent.rs`)

| 단계 | 요청 | 내용 |
|---|---|---|
| 버전 확인 | `GET /info` | 릴레이가 요구하는 최소 에이전트 버전 확인 |
| 하트비트 | `POST /heartbeat` (60초) | `agentVersion`, `os`, `metadata`(서버·모듈·봇 설정). 릴레이가 `UPDATE_REQUIRED`를 주면 경고 로그(`relay_agent.rs:218-240`) |
| 명령 받기 | `GET /poll` (롱폴링) | 디스코드에서 온 명령. 현재 `payload.action`은 `raw_command` 하나다 |
| 결과 보내기 | `POST /result/:id` | 채팅 처리기의 응답 `{success, data:{text}}` (`:389-400`) |

- 모든 요청은 노드 토큰으로 HMAC 서명하고 `X-Saba-Client: 1` 헤더를 붙인다(`relay_agent.rs:159-163`, `handlers/relay.rs` `signed_headers`).
- 데몬이 뜨면 `auto_start`가 에이전트를 켠다(토큰이 없으면 조용히 건너뜀, `ipc/mod.rs:783`, `relay_agent.rs:153-157`).
- 상태 `GET /api/relay/agent`: `{running, sleeping, relay_url, has_token, agent_version, last_heartbeat_at, last_error, commands_handled}` (`relay_agent.rs:31-42`). 켜고 끄기는 `POST /api/relay/agent/start`, `/stop`.
- 새 명령 종류는 `handle_payload`의 action 분기에 더한다.

### 4.3 채팅 처리기 (`src/chat/`)

- 디스코드 명령과 CLI `chat`, `POST /api/chat/command {text, guild_id?, user_id?} → {reply}`가 모두 같은 처리기를 쓴다(`handlers/chat.rs:9-13`).
- 대상 서버 찾기: 인스턴스 이름 → `instanceAliases`(인스턴스 ID → `"별명, 별명"`) → 모듈 별명(`moduleAliases`) 순이다. 같은 게임 서버가 여럿인데 하나로 좁혀지지 않으면 `errors.multiple_instances`로 답한다(`chat/mod.rs:256-285`).
- 명령 별명은 `commandAliases`, 접두사는 `prefix`(기본 `!saba`)다. 응답 문구는 `locales/<lang>/bot.json`을 바이너리에 넣어 쓴다(`chat/i18n.rs`).
- 노드(디스코드 서버)별 허용 인스턴스와 멤버별 명령 권한은 `nodeSettings`에 있다.

### 4.4 봇 초대

- `GET /api/relay/bot-invite`(query `relayUrl?`)가 릴레이의 공개 `/api/bot-invite` 응답 `{url}`을 그대로 돌려준다. 공개 엔드포인트라 서명하지 않는다(`handlers/relay.rs:240-259`).

## 5. 로케일 구조 (`locales/<lang>/{common,gui,bot,cli}.json`, 10개 언어)

| 파일 | 최상위 키(en) | 소비자 |
|---|---|---|
| common.json (leaf 43개) | app_name, success/error/…, daemon, modules, server, network, port_conflict 등 21개 | GUI renderer(`common` ns), **GUI main 프로세스**(알림과 status 메시지) |
| gui.json (en leaf 707개) | title, menu, title_bar, modals, settings_modal, add_instance_modal, migration_modal, command_modal, server_*, discord_bot, discord_modal, notice_modal, updates, extensions, saba_storage, validation, context_menu 등 29개 | GUI renderer(`gui` ns) |
| bot.json | 채팅 응답 문구 | 데몬 채팅 처리기(`src/chat/i18n.rs`, 빌드 때 포함). GUI renderer도 import한다 |
| cli.json (en leaf 200개) | welcome, status, help, *_cmd, menu, screen 등 16개 | CLI `i18n.rs`. `<root>/locales` 또는 실행 파일 옆의 `locales`를 런타임에 읽는다(64-84) |

- GUI renderer는 10개 언어의 common, gui, bot JSON을 **빌드 시 정적으로 import**한다(i18n.js:3-33). 언어 소스는 `window.electron.getLanguage()`가 1순위, localStorage `i18nextLng`가 2순위다(92-139). 확장 i18n은 daemon `/api/extensions/:id/i18n/:locale`에서 받는다.
- **번역 누락**(en 기준 diff):
  - gui.json: de, es, fr, pt-BR, ru, zh-CN, zh-TW는 691개로 **16개가 빠져 있다**(`console.dock_focus/dock_restore/minimize/pin/popin/unpin`, `discord_modal.music_channel_*`, `discord_modal.music_normalize`). ja는 697개다.
  - cli.json: ko를 제외한 8개 언어가 168개로 **32~33개가 빠져 있다**(`screen.bot_*`, `screen.ext_*`, `screen.settings_*`, `screen.update_*` 등).
- 하드코딩된 한국어 문자열이 i18n을 거치지 않는 곳:
  - main.js의 트레이 메뉴(1427-1468), 업데이트 알림(1196-1197), 오류 매핑(2226-2241, 2373-2400), `preloadLightData`(1009-1010)

---

## 6. 불일치, 죽은 코드, stub, 버그 (file:line)

### 6.1 존재하지 않는 endpoint나 잘못된 HTTP 메서드 호출
(예전 TUI CLI의 항목들은 CLI 리메이크에서 코드째 사라져 뺐다. §3 참조.)
1. `saba-chan-gui/main.js:2792`: `updater:setConfig`가 `axios.post('/api/updates/config')`를 호출한다. daemon은 GET/PUT만 제공하므로(updates.rs:91-92) 405가 난다. 영향을 받는 곳은 SabaStorage의 자동 체크 on/off(201)와 Mock 토글(196-215)이다.

### 6.2 죽은 코드와 stub
- `saba-chan-gui/main.js:501-503` `syncGuiConfigToDaemon`(no-op)과 이를 부르는 `guiConfig:sync` 채널(2961), `runBackgroundInit`의 호출(1091-1094).
- `saba-chan-gui/main.js:1005-1011` `preloadLightData`: 상태 메시지만 보낸다(주석에서 레거시라고 인정함).
- renderer가 쓰지 않는 preload API 12개(§1.2). `mockServer:*` 핸들러 3개.
- main.js:1223-1224: auto_download/auto_apply는 "향후 구현 예정" 상태다.

### 6.3 기능 버그
- **before-quit bot 정지 요청 누락**: main.js:1536(`const currentPort`가 if와 try 블록 안에 선언됨)과 1559(블록 밖에서 참조) 때문에 ReferenceError가 나고 조용히 무시된다.
- **SabaStorage bot 재시작 실패**: SabaStorage.js:316, 445가 `settings?.discordToken`을 읽는다. 이 키는 bot-config으로 옮겨져 비어 있으므로 업데이트 후 bot 재시작이 항상 "no token"으로 끝난다. cloud 모드 정보도 넘기지 않는다.
- **settings 경로 분기 기준 오류**: main.js:483, 2860, 2967이 `daemonProcess`(내가 spawn했는가)로 판단한다. daemon이 살아 있는데도 settings.json을 직접 쓰는 경우가 생긴다(§1.3).
- **resolver 주석과 코드 불일치**: resolver.js:57(1분)과 60(5초).
- **updater exe 이름 하드코딩**: main.js:1113-1148, commands.rs:1137-1142에서 `.exe`로 고정되어 있어 비 Windows 환경에서 launch-apply가 불가능하다.

---

## 7. ★ Daemon 우회 지점 전수 목록 (GUI · CLI)

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
| G17 | — | — | ~~relay `PATCH /api/hosts/:hostId/bot-config` 직접 호출~~ 없어짐: 봇 설정은 `PUT /api/config/bot`, 릴레이 동기화는 에이전트 하트비트 | |
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

### 7.3 CLI (`saba-chan-cli/src`, PR #6 기준)
예전 TUI의 우회 지점 C3~C7, C11~C13(로컬 fallback, `cli-settings.json`, updater 직접 실행 등)과 이름 기반 kill은 리메이크에서 사라졌다. 남은 것은 부트스트랩뿐이다.

| # | 위치 | 분류 | 내용 | 비고 |
|---|---|---|---|---|
| C1 | client.rs:28-34 | FS-R | `.ipc_token` 읽기 (한 곳) | 불가피 |
| C2 | bootstrap.rs:18-28 | FS-R/CFG | `config.json`에서 `gui.ipcPort`·`gui.language` 읽기 | 불가피 |
| C8 | i18n.rs:36-45 | FS-R | 설치 폴더 `locales/<lang>/cli.json` 읽기 (없으면 내장본) | |
| C9 | bootstrap.rs:49-90 | PROC | saba-core 분리 실행 | 불가피(부트스트랩) |
| C10 | bootstrap.rs:92, cmd/daemon.rs:102 | PROC | API 종료가 안 될 때 데몬 PID만 강제 종료 | 이름 기반 kill 아님 |

### 7.4 디스코드 봇
로컬 봇이 없어져 이 목록(B1~B11)은 사라졌다. 릴레이와의 통신은 데몬의 relay_agent만 한다(§4).

### 7.5 요약 판정
- **원칙 위반 정도가 가장 큰 항목**:
  - G2, G3, G4: GUI가 settings.json을 직접 읽고 쓴다. 분기 기준이 잘못되어 daemon이 살아 있어도 이 경로를 탄다.
- **부트스트랩상 불가피한 항목**(토큰 파일, ipcPort, daemon 최초 spawn): G1, G7, C1, C2, C9. 다만 daemon **종료**(G8~G12)는 `/api/daemon/shutdown`이 있으므로 API로 대체할 수 있다. 이름 기반 kill(G9)은 특히 위험하다.
- **daemon에 API가 없어서 우회하는 항목**: updater 실행(G14), 언인스톨러(G16).
