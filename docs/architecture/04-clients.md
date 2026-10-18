# 04. 클라이언트 계층: GUI · CLI · Discord Bot (server-chan) · 공유 상수 · 로케일

> 이 문서는 코드에서 직접 도출했다. 기존 docs/README는 신뢰하지 않았고, 코드와 어긋나는 주석이나 문서는 각 항목에서 따로 표시했다.
> 기준 원칙(프로젝트 오너): **daemon(saba-core)이 서비스의 주인이며 모든 기능은 daemon을 거친다. 인터페이스(GUI/CLI)는 혼자서 아무것도 하면 안 되고, 파일 편집도 daemon API를 거쳐야 한다.** modules는 Python 게임 서버 매뉴얼, plugins(=extensions)는 Python으로 작성되어 인터페이스에 컴포넌트를 주입하고, server-chan(discord_bot)은 daemon과 짝을 이뤄 Discord 봇을 돌린다.
> 이 원칙을 어기는 지점은 모두 **§7 "daemon 우회 지점 전수 목록"**에 정리했다. 이 문서에서 가장 중요한 결과물이다.

Daemon 라우터의 기준 위치: `src/ipc/mod.rs:640-758`(인증 미들웨어 `auth::auth_middleware` 적용 구간), `src/ipc/updates.rs:97-110`(`/api/updates/*`, 인증 layer **이전**에 `.merge()`로 붙어 같이 인증된다, `src/ipc/mod.rs:756-758`).

---

## 1. GUI (saba-chan-gui, Electron)

> 2026-10-17 기준 Electron 44, electron-builder 26, Vite 6.4, Vitest 5. 빌드에는 Node `>=22.12`가 필요하다(`saba-chan-gui/package.json` `engines`, 커밋 4ba8d26).

### 1.1 Main process가 맡는 일 (`saba-chan-gui/main.js`, 2895줄)

| 책임 | 위치 | 설명 |
|---|---|---|
| userData 경로 고정 | main.js:32 | `app.setPath('userData', getSabaDataDir())`. Windows는 `%APPDATA%/saba-chan`, Unix는 `$HOME/.config/saba-chan`, `SABA_DATA_DIR`가 있으면 그 값을 쓴다 |
| IPC 토큰 인증 | main.js:58-212 | `.ipc_token` 파일을 **직접 읽어** `X-Saba-Token` 헤더에 넣는다. axios 인터셉터(104-117)가 데몬으로 가는 요청에만 토큰을 붙이고 리다이렉트를 따라가지 않는다(`isDaemonRequest` 93-101). 같은 인터셉터가 감사 로그 출처 `X-Saba-Client: gui`도 붙인다(A10). Node 전역 `http.request`는 건드리지 않는다. 종료 시 unregister만 `http.request`를 직접 쓰며 토큰을 그 자리에서 붙인다. 401을 받으면 토큰 파일을 다시 읽고 1회 재시도한다(124-174). 단 `/api/relay/` 요청은 재시도하지 않는다 |
| 렌더러 격리 | `rendererWebPreferences()` main.js:1038-1045, `lockNavigation()` 1058-1072 | 모든 창이 `nodeIntegration:false, contextIsolation:true, sandbox:true`로 뜬다. 앱 밖 주소로의 이동과 새 창은 막고, http(s) 링크만 바깥 브라우저로 연다 |
| **Daemon spawn** | `startDaemon()` main.js:549-657 | dev 모드는 `target/release/saba-core(.exe)`, prod 모드는 `getInstallRoot()/saba-core(.exe)`를 `--spawned` 인자로 실행한다(608). env는 `RUST_LOG=info, SABA_LANG, SABA_IPC_PORT, SABA_INSTANCES_PATH=<userData>/instances, SABA_MODULES_PATH`. stdout/stderr는 GUI 로그로 넘긴다 |
| Daemon 재사용/기동 판단 | `ensureDaemon()` main.js:772-851 | `GET /health`가 200이면 기존 daemon을 재사용한다. 응답의 `token`이 `valid`가 아니거나(A6) 예전 데몬이 401을 주면 토큰을 다시 읽으며 최대 3회 재시도한다(`reloadTokenUntilValid`). ECONNREFUSED/timeout이면 `startDaemon()`을 호출하고 500ms 간격으로 최대 8회 폴링한다 |
| Daemon 종료 | `stopDaemon()` main.js:660-675, `forceKillDaemon()` 688-704, `cleanQuit()` 736-769 | 먼저 `POST /api/daemon/shutdown`을 보내고 최대 15초 기다린다. 그래도 안 끝나면 **이 GUI가 띄운 PID만** 강제 종료한다(Windows `taskkill /PID … /F`, `/T` 없음. Unix SIGKILL). 이름 기반 kill은 하지 않는다(659 주석) |
| 클라이언트 등록/heartbeat | main.js:874-917 | `POST /api/client/register {kind:'gui', pid, installRoot}`(인스톨러가 이 설치의 GUI를 찾는 데 쓴다), 이후 30초마다 `POST /api/client/:id/heartbeat`를 보낸다. 종료 시 `DELETE /api/client/:id/unregister[?shutdown=true]`를 호출한다 |
| install_root 동기화 | `syncInstallRoot()` main.js:854-868 | `PUT /api/updates/config {install_root}` |
| 업데이트 알림 | main.js:939-1035 | 확인 주기는 데몬이 정한다. GUI는 10분마다 데몬이 캐시한 `GET /api/updates/status`를 읽어, 새 업데이트 목록일 때만 OS Notification을 띄우고 renderer에 `updates:available`을 보낸다. Locales는 알림에서 뺀다. 적용은 사용자가 고르고, `auto_download`이면 데몬이 미리 받아 둔다(1015) |
| **업데이트 적용 시작** | `updater:launchApply` main.js:2461-2482 | `POST /api/updates/launch-updater {relaunch:[<gui exe>]}`만 보내고 `cleanQuit()`한다. 업데이터는 데몬이 띄운다(`src/ipc/updates.rs:570-639`, 실행 파일은 `saba-chan-updater{EXE_SUFFIX}`, 642-649) |
| **Installer spawn(언인스톨)** | `app:launchUninstaller` main.js:2599-2667 | 설치 경로를 직접 탐색하고, Windows는 `reg query HKCU\…\Uninstall\Saba-chan`도 본다(2629). 이후 `saba-chan-installer.exe --uninstall`을 detached로 실행한다(2653) |
| 모의 릴레이(개발용) | — | GUI는 더 이상 띄우지 않는다(`mockServer:*` IPC와 preload 항목은 f6ca7b6에서 삭제). 개발자가 `node scripts/mock-release-server.js`를 직접 실행하거나 `scripts/build-core.ps1 -MockRelay`(모의 서명 키를 믿는 개발 빌드, 배포 금지)로 쓴다 |
| 디스코드 연결 | `discord:start/stop/status` main.js:2764-2792 | GUI는 봇을 띄우지 않는다. 데몬의 릴레이 에이전트를 `/api/relay/agent{,/start,/stop}`로 켜고 끈다(§4) |
| 콘솔 팝아웃 창 | main.js:1609-1721 | 별도 `BrowserWindow`에 `?console-popout=<id>&name=` 쿼리를 붙여 같은 React 번들을 로드한다(1639) |
| 트레이 | main.js:1188-1274 | 데몬 시작/종료, "인터페이스만 종료", "완전히 종료" 메뉴. "데몬 종료"는 `stopDaemon()`(API 우선), "데몬 시작"은 API가 아니라 `startDaemon()`을 직접 호출한다 |
| 종료 플로우 | main.js:1110-1115, 1154-1165 | 창 close를 가로채 renderer에 `app:closeRequest`를 보내고, 선택 결과 `exit-interface`/`quit`/`cancel`을 받는다 |
| 로그 | main.js:245-305 | `<userData>/logs/saba-chan-<ts>.log`에 직접 쓴다 |
| 메인 프로세스 i18n | main.js:308-333 | `<installRoot>/locales/<lang>/common.json`을 직접 읽는다 |

**인증 방식 요약**: 데몬이 시작할 때 UUID 토큰을 만들어 `<dataDir>/.ipc_token`(Unix 0600)에 쓰고(`src/ipc/auth.rs:27-63`), 모든 클라이언트는 이 파일을 읽어 `X-Saba-Token`을 보낸다. `Host`가 루프백이 아니면 403으로 막는다(`auth.rs:75-84, 98-101`). `SABA_AUTH_DISABLED=1`은 개발 빌드(`debug_assertions`)에서만 통하고, 토큰이 없으면 아무 요청도 통과시키지 않는다(`auth.rs:104-125`). 토큰은 바이트를 끝까지 비교하는 상수 시간 비교로 맞춘다(`auth.rs:86-89, 134`). `/api/updates/*`도 auth layer 안에 있어 인증 대상이다. `/health`만 토큰 없이 답하고, 보낸 토큰이 맞는지(`token: valid|invalid|missing`)를 알려 준다(A6).

### 1.2 preload 채널 전수표 (`preload.js` → `ipcMain` → daemon endpoint)

`window.api.*`는 preload.js:3-144, `window.electron.*`는 preload.js:147-158에 있다. "Daemon 라우트 존재"는 `src/ipc/mod.rs`와 `updates.rs` 기준으로 대조했다. "미사용"은 qcza9f의 renderer(`src/`, 테스트 제외)가 부르지 않는다는 뜻이다(새 채널 대부분은 GUI 리메이크 PR #5용).

| preload 함수 (preload.js 줄) | 채널 | main.js 줄 | 호출 대상 | 라우트 존재 |
|---|---|---|---|---|
| serverList (4) | server:list | 1390 | GET /api/servers (port_conflict_stops가 있으면 OS 알림). 서버마다 `managed`(runner가 맡는지, `Supervisor::uses_managed_start`)가 실린다(`ipc/mod.rs:208-210`, `handlers/server.rs:81`) | ✓ 644 |
| instanceStart (5) | instance:start | 1453 | POST /api/instance/:id/start (managed 여부와 포트 검사는 데몬이 정한다) | ✓ 647 |
| serverStop (6) | server:stop | 1480 | POST /api/server/:name/stop | ✓ 651 |
| serverStatus (7) | server:status | 1510 | GET /api/server/:name/status | ✓ 645 |
| moduleList (8) | module:list | 1723 | GET /api/modules | ✓ 652 |
| moduleRefresh (9) | module:refresh | 1742 | POST /api/modules/refresh | ✓ 653 |
| moduleGetMetadata (10) | module:getMetadata | 1829 | GET /api/module/:name | ✓ 660 |
| instanceEvents (12) | instance:events | 1820 | GET /api/instance/:id/events (최근 종료 이력) | ✓ 648 (미사용) |
| moduleGetLocales (13) | module:getLocales | 1799 | GET /api/module/:name/locales | ✓ 707 |
| moduleListVersions (14) | module:listVersions | 1539 | GET /api/module/:name/versions?… | ✓ 693 |
| moduleInstallServer (15) | module:installServer | 1552 | POST /api/module/:name/install | ✓ 695 |
| instanceGetInstalledVersion (16) | instance:getInstalledVersion | 1564 | GET /api/instance/:id/installed-version | ✓ 696 |
| moduleManifest (18) | module:manifest | 1767 | GET /api/modules/manifest | ✓ 657 |
| moduleInstallFromManifest (19) | module:installFromManifest | 1776 | POST /api/modules/manifest/:id/install | ✓ 658 |
| moduleRemove (20) | module:remove | 1789 | DELETE /api/modules/:id | ✓ 659 |
| instanceCreate (21) | instance:create | 1854 | POST /api/instances | ✓ 662 |
| instanceProvisionProgress (22) | instance:provisionProgress | 1887 | GET /api/provision-progress/:name | ✓ 667 |
| instanceDismissProvision (23) | instance:dismissProvision | 1898 | DELETE /api/provision-progress/:name | ✓ 667 |
| instanceCheckUpdate (24) | instance:checkUpdate | 1909 | GET /api/instance/:id/check-update | ✓ 669 |
| instanceApplyUpdate (25) | instance:applyUpdate | 1921 | POST /api/instance/:id/apply-update | ✓ 670 |
| instanceDelete (26) | instance:delete | 1933 | DELETE /api/instance/:id | ✓ 665 |
| instanceReorder (27) | instance:reorder | 1962 | PUT /api/instances/reorder {order} | ✓ 663 |
| instanceUpdateSettings (28) | instance:updateSettings | 1982 | PATCH /api/instance/:id. 400이면 `{status, error, error_code, details, fields}`를 그대로 넘긴다(`invalid_format`이면 형식 오류 알림) | ✓ 665 |
| instanceLaunchPreview (31) | instance:launchPreview | 1966 | POST /api/instance/:id/launch-preview `{settings, extra_args?}` (extra_args를 안 넘기면 저장된 것) | ✓ (LaunchPreview.js) |
| instanceResetServer (29) | instance:resetServer | 1573 | POST /api/instance/:id/server/reset | ✓ 684 |
| executeCommand (30) | instance:executeCommand | 2014 | POST /api/instance/:id/command {command, args}. 명령 해석과 라우팅(REST·stdin·모듈)은 데몬이 한다(2012 주석). GUI는 `…/rcon`, `…/rest`를 직접 부르지 않는다 | ✓ 672 |
| instanceStop/Restart/Rename/Commands (31-34) | instance:stop/restart/rename/commands | 2054-2065 | POST …/stop·…/restart {force}(restart는 150초 timeout), POST …/rename {name}, GET …/commands | ✓ 649, 650, 664, 673 (미사용) |
| instanceReadLiveSettings (35) | instance:readLiveSettings | 2068 | GET /api/instance/:id/settings/live | ✓ 683 (미사용) |
| runtimePreflight (36) | runtime:preflight | 2072 | GET /api/runtime/preflight | ✓ 723 (미사용) |
| configDefaults / configReset (37-38) | config:defaults/reset | 2074-2079 | GET /api/config/:kind/defaults, POST /api/config/:kind/reset {keys?} | ✓ 702-703 (미사용) |
| extensionList…extensionInitStatus (40-53) | extension:* | 2084-2263 | /api/extensions, /:id/enable, /disable, /i18n/:locale, /gui, /gui/styles, /icon, /manifest, /:id/install, DELETE /:id, /updates, /rescan, /init-status | ✓ 721-737 (확장 UI 로딩은 다른 문서에서 다룸) |
| managedConsole (55) | managed:console | 1588 | GET /api/instance/:id/console?since&count (4번째 인자 `{spans:true}`면 `&spans=1`) | ✓ 678 |
| managedStdin (56) | managed:stdin | 1599 | POST /api/instance/:id/stdin | ✓ 679 |
| consolePopout/FocusPopout/Popin/ToggleAlwaysOnTop (58-61) | console:* | 1611-1721 | Electron 창 조작만 (daemon 호출 없음) | n/a |
| daemonConsole (67) | daemon:console | 1600 | GET /api/daemon/console?since&count. `{lines:[{id, timestamp, level, target, message}], count}` 그대로 | ✓ |
| consolePopoutList (65) | console:popoutList | 1600 | 열린(파괴되지 않은) 팝아웃 창의 instanceId 배열. daemon 호출 없음 | n/a |
| updaterCheck (69) | updater:check | 2267 | POST /api/updates/check | ✓ updates.rs:101 |
| updaterStatus (70) | updater:status | 2405 | GET /api/updates/status | ✓ 100 |
| updaterDownload (71) | updater:download | 2416 | POST /api/updates/download (본문이 없으면 전부) | ✓ 102 |
| updaterDownloadProgress (72) | updater:downloadProgress | 2427 | GET /api/updates/download/progress | ✓ 103 |
| updaterApply (73) | updater:apply | 2438 | POST /api/updates/apply (본문이 없으면 전부) | ✓ 104 |
| updaterLaunchApply (74) | updater:launchApply | 2461 | POST /api/updates/launch-updater {relaunch}. 업데이터는 데몬이 띄운다 | ✓ 105 |
| updaterLastResult (75) | updater:lastResult | 2484 | GET/DELETE /api/updates/last-result | ✓ 106 (미사용) |
| updaterGetConfig (76) | updater:getConfig | 2495 | GET /api/updates/config | ✓ 108 |
| updaterSetConfig (77) | updater:setConfig | 2505 | PUT /api/updates/config | ✓ 109 |
| settingsLoad (82) | settings:load | 2569 | GET /api/config/gui. 데몬이 응답하지 않을 때만 config.json `gui`를 직접 읽는다(`loadSettings`) | ✓ 700 |
| settingsSave (83) | settings:save | 2669 | GET /api/config/gui로 merge base를 읽고 PUT. 데몬이 응답하지 않을 때만 config.json `gui`에 직접 쓴다. 데몬이 오류 응답을 주면 저장 실패로 끝난다 | ✓ 700 |
| settingsGetPath (84) | settings:getPath | 2687 | 로컬 경로 문자열(`<dataDir>/config.json`) | n/a |
| openFileDialog / openFolderDialog (87-88) | dialog:* | 2709/2738 | Electron 네이티브 다이얼로그 | n/a |
| migrationScanDir (90) | migration:scanDir | 2780 | POST /api/modules/detect (`{ok, matches}`, 파일 목록 없음) | ✓ 711 |
| discordBotStatus / Start / Stop (92-94) | discord:status/start/stop | 2764-2792 | GET /api/relay/agent, POST /api/relay/agent/start·stop. 노드 토큰이 없으면(`node_token_missing`) `cloud_token_not_found` | ✓ 747-749 |
| botConfigLoad (96) | botConfig:load | 2794 | GET /api/config/bot | ✓ 698 |
| botConfigSave (97) | botConfig:save | 2857 | PUT /api/config/bot(최상위 키 단위 병합). 릴레이 동기화는 에이전트 하트비트가 맡는다 | ✓ 698 |
| saveNodeToken / loadNodeToken / deleteNodeToken (100-102) | nodeToken:* | 2799/2803/2808 | PUT/GET/DELETE /api/config/node-token. DELETE는 데몬이 에이전트를 멈추고 토큰을 지운다 | ✓ 705 |
| auditList (107) | audit:list | 2336 | GET /api/audit?limit&instance (A10). `{entries}` 최신이 먼저 | ✓ (A10) |
| permissionsGet | permissions:get | - | GET /api/permissions/:guildId (P1) | - |
| permissionsUpdate | permissions:update | - | PATCH /api/permissions/:guildId `{defaultLevel?, preset?, chatbotCap?, ownerDiscordId?, commandLevels?}` | - |
| permissionsSetMember | permissions:setMember | - | PUT /api/permissions/:guildId/members/:discordId `{level}` | - |
| permissionsRemoveMember | permissions:removeMember | - | DELETE /api/permissions/:guildId/members/:discordId | - |
| sandboxListUnisolated | sandbox:listUnisolated | - | GET /api/sandbox/unisolated `{components:[{component, fingerprint, current}]}` | - |
| sandboxAllowUnisolated | sandbox:allowUnisolated | - | POST /api/sandbox/unisolated/:component. `sandbox_unavailable` 응답의 `component`를 넘긴다 | - |
| sandboxRevokeUnisolated | sandbox:revokeUnisolated | - | DELETE /api/sandbox/unisolated/:component | - |
| actionsList | actions:list | - | GET /api/actions?surface=gui&instance?&locale? (P2) | - |
| actionsPlan | actions:plan | - | POST /api/actions/:id/plan `{input, instance?}` | - |
| actionsRun | actions:run | - | POST /api/actions/:id/run `{input, instance?}` 또는 `{planId}` | - |
| discordSettingsGet | discord:settings:get | - | GET /api/discord/:guild/settings (I2) | - |
| discordSettingsSet | discord:settings:set | - | PATCH /api/discord/:guild/settings `{locale?, sleeping?, music?}` | - |
| openInTerminal | clients:launch | - | POST /api/clients/launch `{kind:'cli', view, instance?}` (I4). 데몬이 새 콘솔 창에 CLI 콘솔·셸을 띄운다. 실패하면 데몬 본문(`error_code`)을 그대로 돌려준다. 트레이 메뉴 "터미널 열기"(Windows만)도 같은 라우트를 `view:'shell'`로 부른다 | - |
| onDaemonEvent | events:message (main → 창) | - | main.js가 GET /api/events(SSE) 연결 하나를 유지하고 받은 이벤트를 `{id, type, data}`로 보낸다. 끊기면 3초 뒤 마지막 번호로 다시 붙는다. `type: 'resync'`면 목록을 다시 읽는다. 돌려주는 함수로 해제 (P4) | - |
| dataExport / dataImport (105-106) | dataTransfer:export/import | 2800/2809 | 저장·열기 다이얼로그로 경로를 고른 뒤 POST /api/migration/export `{path, secrets}` · /api/migration/import `{path, dryRun, overwrite}`. 취소하면 `{canceled:true}`. 가져오기 결과에는 `path`를 붙여 미리보기(`dryRun`) 뒤 같은 파일로 다시 부를 수 있다 | ✓ (A9) |
| relayCheckHostStatus… relayBotInvite (104-107) | relay:* | 2296-2344 | GET /api/relay/host/:id/status, /nodes, GET /api/relay/node/:guild/members, GET /api/relay/bot-invite | ✓ 740-743 |
| relayNodeChannels / relayNodeChannelsSave (113-114) | relay:nodeChannels/nodeChannelsSave | 2320/2323 | GET/PUT /api/relay/node/:guild/channels(`notify` 알림 채널 포함). 실패하면 데몬 본문(`{error, channelId?}`)을 그대로 돌려준다 | ✓ |
| instanceAgreements (108) | instance:agreements | 2568 | POST /api/instance/:id/agreements `{accept:[id]}` | 모듈 약관 동의 |
| onboarding | onboarding:get | 2569 | GET /api/onboarding (J1-a 다음 할 일) | - |
| instanceBackups…instanceScheduleSave (109-114) | instance:backup*/schedule* | 2359-2368 | GET/POST …/backups, DELETE …/backups/:name, POST …/backups/:name/restore(만들기·복원은 10분 timeout), GET/PUT …/schedule | ✓ 688-691 (미사용) |
| chatbotGet/Save/Clear/ClearMemory (115-118) | chatbot:* | 2370-2375 | GET/PUT/DELETE /api/chatbot, DELETE /api/chatbot/memory?guildId | ✓ 753-754 (미사용) |
| relayInitiatePairing / relayPollPairingStatus (119-120) | relay:* | 2377/2388 | POST /api/relay/pair/initiate, GET /api/relay/pair/:code/status | ✓ 744-745 |
| shellOpenPath (122) | shell:openPath | 2832 | Electron `shell.openPath` | n/a |
| logsGetPath / logsOpenFolder (124-125) | logs:* | 2811/2848 | 로컬 | n/a (미사용) |
| launchUninstaller (127) | app:launchUninstaller | 2599 | installer 직접 spawn | n/a |
| getAppVersion (128) | app:getVersion | 2580 | `app.getVersion()` | n/a |
| getComponentInfo (129) | app:getComponentInfo | 2584 | GET /api/system/components | ✓ 709 |
| systemMetrics (37) | system:metrics | 2049 | GET /api/system/metrics | ✓ 683 (`SystemMonitor`) |
| closeResponse (133) | app:closeResponse (send) | 1154 | 종료 플로우 | n/a |
| daemonStatus (138) | daemon:status | 2515 | GET /health | ✓ 642 |
| daemonRestart (139) | daemon:restart | 2525 | `stopDaemon()`(API 종료, 안 되면 자기 PID만 kill) 후 `/health`가 멎을 때까지 기다렸다가 **`startDaemon()`으로 직접 spawn** | 부분 |
| minimize/maximize/closeWindow (141-143) | window:* | 2872-2895 | 창 조작 | n/a |
| electron.getLanguage/setLanguage (152-153) | language:get/set | 2692/2696 | get은 config.json `gui.language`를 직접 읽는다(`loadSettings`). set은 `saveSettings`(API 우선)로 저장하고 번역을 다시 읽는다. 데몬은 재시작하지 않는다 | 부분 |
| electron.shell.openExternal (156) | shell:openExternal | 2816 | 외부 URL 열기(http(s)만) | n/a |

Main에서 renderer로 가는 push 이벤트: `status:update`(로딩 단계 `{step, message, ts}`. `message`는 `locales/<lang>/common.json`의 `daemon.*`·`init.*`로 번역된다. 데몬이 10초 안에 응답하지 않으면 `step:'daemon_failed'`를 보내고 `ready`는 보내지 않는다. 그 뒤 `daemon:restart`가 성공하면 멈춘 초기화를 이어서 끝내고 `ready`를 보낸다. 실패하면 `{success:false, error}`), `updates:available`, `updates:completed`, `console:popoutOpened/Closed/popinRequest`, `app:closeRequest`.

**renderer(src/, 테스트 제외)에서 쓰지 않는 preload API**: 위 표의 "미사용" 항목(`instanceEvents`, `instanceStop/Restart/Rename/Commands`, `instanceReadLiveSettings`, `runtimePreflight`, `configDefaults/Reset`, `updaterLastResult`, `loadNodeToken`, `deleteNodeToken`, `relayBotInvite`, `instanceAcceptEula`, 백업·예약 6개, `chatbot*` 4개, `logsGetPath`, `logsOpenFolder`). 반대로 **main.js가 호출하지 않는 daemon 라우트**: `/api/server/:name/start`, GET `/api/instances`, GET `/api/instance/:id`, `/api/instance/:id/rcon`, `/rest`, `/managed/start`, `/validate`, `/properties`(GET/PUT), `/properties/reset`, `/diagnose`, `/api/module/:name/version/:version`, `/api/version`, `/api/modules/schema`, `/api/config/client/:kind`, `/api/extensions/:id/mount|unmount|config`, `/api/daemon/console`, `/api/chat/command`, `/api/updates/integrity`. 이 중 일부는 CLI만 사용한다(§3).

### 1.3 GUI가 읽고 쓰는 설정 파일

모든 사용자 설정은 `<dataDir>/config.json` 하나에 네임스페이스별로 들어간다(`updater/src/config_file.rs:1-27`). 예전 개별 파일(`settings.json`, `bot-config.json` 등)은 데몬이 한 번 이관하고, 원본은 남긴 채 `.migrated` 사본을 만든다(`config_file.rs:37-45`).

| 파일 | 경로 | 접근 방식 | 스키마 |
|---|---|---|---|
| config.json `gui` | `<dataDir>/config.json` (main.js:458-461) | `loadSettings()`(511-523)는 **항상 파일을 직접 읽는다**(`gui`가 아직 없으면 예전 `settings.json`). `saveSettings()`(528-547)는 먼저 `PUT /api/config/gui`를 보내고, 데몬이 **응답하지 않을 때만** `writeGuiSection()`(485-496)으로 `gui` 키만 바꿔 tmp→rename으로 쓴다(직전 파일은 `.bak`) | 기본값(main.js:498-509): `autoRefresh, refreshInterval(2000), windowBounds{width,height,x,y}, language, ipcPort(57474), consoleBufferSize(2000), autoGeneratePasswords, portConflictCheck`. renderer가 추가로 저장하는 키(useSettingsStore.js:79-97): `accentColor, accentSecondary, useGradient, fontScale, enableTransitions, consoleSyntaxHighlight, consoleBgColor, consoleTextColor, sidebarCompact, consoleFontScale` |
| config.json `relay` | daemon이 관리(`/api/config/bot`, `src/config_store/mod.rs:208, 282`) | GUI는 API만 사용 | `prefix, cloud{relayUrl,hostId}, moduleAliases, commandAliases, instanceAliases{<인스턴스ID>:"별명, 별명"}, nodeSettings, cloudNodes[], cloudMembers{}`. 로컬 봇 필드(`token`, `autoStart`, `mode`, `music*`)는 없다(01 §4 BotConfig) |
| config.json `chatbot` | daemon이 관리(`/api/chatbot`) | API | 응답에는 API 키 대신 `hasApiKey`만 온다(main.js:2369 주석, `src/ipc/handlers/chatbot.rs:1-15`) |
| node token | `/api/config/node-token` | API | `{token: "sbn_<nodeId>.<secret>"}` |
| .ipc_token | `<dataDir>/.ipc_token` 또는 `SABA_TOKEN_PATH` | 직접 읽기(필수 부트스트랩) | 평문 UUID |
| logs | `<dataDir>/logs/*.log` | 직접 쓰기 | — |
| renderer localStorage | Chromium 저장소 | 직접 | `i18nextLng`(i18n.js:100-139, SettingsModal.js:249), 테마 키(themeManager.js:50-120), 공지 목록(NoticeModal.js:15-25) |

**쓰기 분기(해결됨).** 예전에는 `settings:load/save`와 `saveSettings`가 "이 GUI가 daemon을 spawn했는가"(`daemonProcess !== null`)로 분기해서, 이미 떠 있던 daemon에 붙으면 settings.json을 직접 썼다. 지금은 "daemon이 응답하는가"로 나눈다. 데몬이 응답하면 API로만 쓰고, 오류 응답이면 로컬로 넘어가지 않고 실패로 끝낸다(main.js:525-547, 2569-2685). 남은 직접 접근은 **읽기**다. `loadSettings()`는 데몬 상태와 상관없이 파일을 읽고, `getLanguage`, `createWindow`, 창 크기 저장(1094-1100, 파일에서 읽어 merge한 뒤 API로 저장)이 이 값을 쓴다.

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
- 테스트(`src/test/`): gui-e2e, settings-roundtrip, store-unit, discord-side-panel, console-window, theme-contrast. 모두 `window.api`를 mock한다(setupTests.js).

---

## 2. 공유 상수와 포트 (`shared/constants.js`)

| 상수 | 값 | 사용처 |
|---|---|---|
| APP_NAME | `saba-chan` | dataDir 이름 |
| DEFAULT_IPC_PORT | `57474` | main.js:18-20, useSettingsStore |
| DEFAULT_DAEMON_URL | `http://127.0.0.1:57474` | |
| SUPPORTED_LANGUAGES | en, ko, ja, zh-CN, zh-TW, es, pt-BR, ru, de, fr | main.js. **중복 정의 위치**: `saba-chan-gui/src/i18n.js:36` |
| getSabaDataDir() | `SABA_DATA_DIR`, Windows는 `%APPDATA%/saba-chan`, 그 외는 `$HOME/.config/saba-chan` | |

- Rust 쪽 SSOT는 `updater/src/constants.rs`이고(constants.js:7 주석), CLI는 `saba_chan_updater_lib::constants`를 사용한다.
- 그 밖의 포트:
  - 모의 릴레이 `scripts/mock-release-server.js` 9876 (개발용, GUI가 띄우지 않음)
  - Vite dev 5173 (main.js:1059, 1122)
  - Relay 기본 URL `https://saba-chan.online` (DiscordBotModal.js:9, 데몬 `src/ipc/handlers/relay.rs:41`). main.js에는 없다

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
- `slots.rs`: 익스텐션 manifest의 `cli.slots`(데이터만, 코드 실행 없음). 지원 자리는 `InstanceList.badge`, `InstanceDetail.status`, `CreateInstance.options`.
- `i18n.rs`: 설치 폴더의 `locales/<lang>/cli.json`을 먼저 읽고, 없으면 바이너리에 넣어 둔 en/ko를 쓴다(`:11-12, 36-45`). clap 도움말은 `help.cmd.*`·`help.arg.*` 키가 있으면 그 문구로 바꿔 끼운다(`cli.rs:617-645`).
- `cmd/`: 명령 갈래마다 파일 하나(backup, bot, chatbot, config, console, daemon, extension, module, schedule, server, status, update).
- 콘솔 색: 터미널이 색을 쓸 때만 `?spans=1`을 붙여(`client.rs` `console`) 데몬이 준 토큰 구간을 토큰별 ANSI 스타일로 칠하고, 나머지는 줄 `level` 색을 쓴다(`cmd/console.rs` `format_line`, `token_style`). 토큰은 02 §`HighlightRuleToml`.

- `bot pair [--relay-url URL]`: 페어링 코드를 받아 보여 주고(디스코드 `/사바쨩 연결`·`/saba connect`에 넣는다) claimed까지 기다린다. 데몬은 설정한 릴레이와만 페어링하므로, `--relay-url`이 설정과 다르면 봇 설정 `cloud.relayUrl`을 먼저 바꾼 뒤 페어링한다. `--json`만 주면 코드만 내고 끝, `--yes`면 기다린다.
- `bot channels GUILD [--chatbot 이름,…] [--no-chatbot] [--music 이름] [--no-music] [--notify 이름] [--no-notify]`: 채널 이름이나 id를 GET 응답의 `channels`로 id로 바꿔 PUT한다. `--notify`는 알림 채널(I3: 크래시·백업·업데이트·예약 예고)이다. 음악 채널을 새로 정할 때는 메시지가 지워진다고 확인을 받는다.
- `bot lang GUILD [auto|코드]`, `bot sleep GUILD`, `bot wake GUILD`, `bot music GUILD [--on|--off] [--ui on|off] [--queue-lines N] [--refresh MS] [--normalize on|off]`: 디스코드 길드 설정(I2) `GET/PATCH /api/discord/:guild/settings`. 값이 없으면 보기만 한다. 봇에 닿지 않아 `sync.ok:false`면 다음 연결 때 보낸다고 알린다.
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
- 대상 서버 찾기: 인스턴스 이름 → `instanceAliases`(인스턴스 ID → `"별명, 별명"`) → 모듈 별명(`moduleAliases`) 순이다. 같은 게임 서버가 여럿인데 하나로 좁혀지지 않으면 `errors.multiple_instances`로 답한다(`chat/mod.rs:257-288`).
- 명령 별명은 `commandAliases`, 접두사는 `prefix`(기본 `!saba`)다. 응답 문구는 `locales/<lang>/bot.json`을 바이너리에 넣어 쓴다(`chat/i18n.rs`).
- 노드(디스코드 서버)별 허용 인스턴스와 멤버별 명령 권한은 `nodeSettings`에 있다.

### 4.4 봇 초대

- `GET /api/relay/bot-invite`(query `relayUrl?`)가 릴레이의 공개 `/api/bot-invite` 응답 `{url}`을 그대로 돌려준다. 공개 엔드포인트라 서명하지 않지만, 주소는 서명 요청과 같이 설정한 릴레이나 루프백만 쓴다(`handlers/relay.rs:265-287`).

## 5. 로케일 구조 (`locales/<lang>/{common,gui,bot,cli}.json`, 10개 언어)

| 파일 | 최상위 키(en) | 소비자 |
|---|---|---|
| common.json (leaf 43개) | app_name, success/error/…, daemon, modules, server, network, port_conflict 등 21개 | GUI renderer(`common` ns), **GUI main 프로세스**(알림과 status 메시지) |
| gui.json (en leaf 677개) | title, menu, title_bar, modals, settings_modal, add_instance_modal, migration_modal, command_modal, server_*, discord_bot, discord_modal, notice_modal, updates, extensions, saba_storage, validation, context_menu 등 29개 | GUI renderer(`gui` ns) |
| bot.json | 채팅 응답 문구 | 데몬 채팅 처리기(`src/chat/i18n.rs`, 빌드 때 포함). GUI renderer도 import한다 |
| cli.json (PR #6 기준 en leaf 402개) | server, daemon, module, ext, bot, backup, schedule, chatbot, err, prompt 등 39개 | CLI `i18n.rs`. 설치 폴더의 `locales/<lang>/cli.json`을 먼저 읽고, 없으면 내장 en/ko를 쓴다(`i18n.rs:11-12, 36-45`). ko만 도움말 번역 `help`(leaf 183개)가 더 있어 585개다. 나머지 언어의 도움말은 clap 기본 영어다. qcza9f에는 아직 예전 TUI용 cli.json(en 200개, `screen`·`menu` 등 16개)이 있다 |

- GUI renderer는 10개 언어의 common, gui, bot JSON을 **빌드 시 정적으로 import**한다(i18n.js:3-33). 언어 소스는 `window.electron.getLanguage()`가 1순위, localStorage `i18nextLng`가 2순위다(92-139). 확장 i18n은 daemon `/api/extensions/:id/i18n/:locale`에서 받는다.
- **번역 누락**(en 기준 diff):
  - gui.json: de, es, fr, pt-BR, ru, zh-CN, zh-TW는 661개로 **16개가 빠져 있다**(`console.dock_focus/dock_restore/minimize/pin/popin/unpin`, `discord_modal.music_channel_*`, `discord_modal.music_normalize`). ja는 667개로 `discord_modal.music_*` 10개가 빠져 있다.
  - cli.json: qcza9f의 예전 파일은 ko를 제외한 8개 언어가 168개로 33개가 빠져 있다(`screen.bot_*` 등). PR #6의 새 파일은 10개 언어 모두 en 키가 빠짐없이 있다.
- 하드코딩된 한국어 문자열이 i18n을 거치지 않는 곳:
  - main.js의 트레이 메뉴(1202, 1229-1266), 업데이트 알림(988-989, 1310), 오류 매핑(인스턴스 생성·삭제·설정·명령, 1868-2037), 로딩 상태 문구(`runBackgroundInit` 920-934, 1303), shell·logs 오류(2812-2854)

---

## 6. 불일치, 죽은 코드, stub, 버그 (file:line)

### 6.1 존재하지 않는 endpoint나 잘못된 HTTP 메서드 호출
(예전 TUI CLI의 항목들은 CLI 리메이크에서 코드째 사라져 뺐다. §3 참조.)
1. ~~`updater:setConfig`가 POST를 써서 405~~ 해결됨: 이제 PUT이다(qcza9f main.js:2505).
2. ~~CLI `update download`/`update apply`가 본문 없이 POST하는데 데몬은 JSON 본문을 요구했다~~ → 해결: PR #6 CLI는 `{components:[...]}`를 보낸다(`saba-chan-cli/src/client.rs:609-619`). 데몬도 본문이 없거나 공백이면 전체로 받고, 깨진 JSON은 400 `invalid_body`로 거부한다(`src/ipc/updates.rs:320-343, 447-452`).

### 6.2 죽은 코드와 stub
- ~~`saba-chan-gui/main.js:501-503` `syncGuiConfigToDaemon`(no-op)과 이를 부르는 `guiConfig:sync` 채널~~ → 해결: 함수, 채널, preload 항목이 모두 없어졌다(preload.js:98은 주석만 남음).
- ~~`saba-chan-gui/main.js:1005-1011` `preloadLightData`~~ → 해결: 없어졌다. `runBackgroundInit`이 로딩 상태 문구만 직접 보낸다(main.js:919-937).
- renderer가 쓰지 않는 preload API(§1.2).
- ~~main.js:1223-1224: auto_download/auto_apply는 "향후 구현 예정"~~ → 해결: `auto_download`이면 데몬이 확인 때 미리 내려받아 둔다(`src/ipc/updates.rs:207-208, 227`). 동작하지 않던 `auto_apply`는 설정에서 빠졌고, 적용은 언제나 사용자가 고른다.

### 6.3 기능 버그
- ~~**before-quit bot 정지 요청 누락**: main.js:1536과 1559의 `currentPort` 범위 오류로 ReferenceError~~ → 해결: before-quit은 봇을 건드리지 않고, `currentPort`를 선언한 같은 try 블록 안에서 unregister만 보낸다(main.js:1336-1351).
- ~~**SabaStorage bot 재시작 실패**: SabaStorage.js:316, 445가 `settings?.discordToken`을 읽음~~ → 해결: 로컬 봇이 없어져 SabaStorage는 봇을 재시작하지 않고, renderer에 `discordToken`을 읽는 곳이 없다.
- ~~**settings 경로 분기 기준 오류**: main.js:483, 2860, 2967이 `daemonProcess`로 판단~~ → 해결: 데몬이 응답하는지로 나누고, 응답하면 API로만 쓴다(main.js:528-547, 2569-2577, 2669-2685). 읽기 직접 접근은 남아 있다(§1.3).
- ~~**resolver 주석과 코드 불일치**: resolver.js:57(1분)과 60(5초)~~ → 해결: `resolver.js`는 로컬 봇(`discord_bot/`)과 함께 L1(7b5102f)에서 지워졌다.
- ~~**updater exe 이름 하드코딩**: main.js:1113-1148, 예전 TUI `commands.rs:1137-1142`~~ → 해결: GUI와 CLI는 `POST /api/updates/launch-updater`만 보내고, 데몬이 `saba-chan-updater{EXE_SUFFIX}`를 찾아 띄운다(`src/ipc/updates.rs:642-649`).

---

## 7. ★ Daemon 우회 지점 전수 목록 (GUI · CLI)

> 기준: "인터페이스는 혼자서 아무것도 못 한다. 파일 편집도 daemon API를 거친다."
> 분류: **[FS-R]** 직접 파일 읽기 / **[FS-W]** 직접 파일·디렉터리 쓰기 / **[PROC]** 프로세스 spawn·kill / **[EXT]** 외부 서비스 직접 호출 / **[CFG]** 설정 파일 직접 편집 / **[OS]** OS 레지스트리·셸
> "불가피" 표시는 부트스트랩상 daemon 없이는 할 수 없는 항목이다. 그래도 목록에는 포함했다.

### 7.1 GUI main process (`saba-chan-gui/main.js`)
| # | 위치 | 분류 | 내용 | 비고 |
|---|---|---|---|---|
| G1 | 58-84, 177 | FS-R | `.ipc_token` 읽기 | 불가피(인증 부트스트랩) |
| G2 | 471-483, 511-523 | FS-R/CFG | `loadSettings()`는 daemon 상태와 상관없이 **항상** config.json `gui`(없으면 예전 settings.json)를 직접 읽는다. `getLanguage`, `createWindow`, `refreshIpcBase`, 창 크기 저장, `language:get`, settings 폴백이 이 함수를 쓴다 | ipcPort 부트스트랩만 불가피하고, 나머지는 API로 대체할 수 있다 |
| G3 | 485-496, 528-547 | FS-W/CFG | `saveSettings()`: 데몬이 **응답하지 않을 때만** `writeGuiSection()`으로 config.json `gui`를 직접 쓴다(다른 키 보존, tmp→rename, `.bak`). windowBounds 저장(1094-1100), `language:set`→`setLanguage`(448-452), `settings:save`(2669-2685)가 이 경로를 탄다 | 부트스트랩 전용. 예전 `daemonProcess` 분기 버그는 해결(§6.3) |
| G4 | 2569-2577 | FS-R/CFG | `settings:load`: API 실패 시 로컬 폴백 | 위와 같음 |
| G5 | 245-298 | FS-W | 로그 디렉터리 생성과 로그 파일 쓰기 | GUI 자체 로그 |
| G6 | 313-333 | FS-R | `locales/<lang>/common.json` 직접 읽기 | |
| G7 | 549-657 (608-617), 783-797 | PROC | **saba-core spawn** (`--spawned`, env 주입). `detached: true`, `windowsHide: true`로 분리해 띄운다(콘솔 창 없음, 로그는 GUI가 사는 동안 파이프로 받는다). Windows에서 분리하지 않은 자식은 Electron의 kill-on-close 잡에 묶여 GUI와 함께 죽기 때문이다. 인터페이스만 종료하면(`interfaceOnlyQuit`, 706-740) 파이프를 놓고 `unref()`해 데몬이 남는다. GUI가 죽거나 강제 종료돼도 데몬은 남고, 다음 GUI 실행 때 `ensureDaemon`이 `/health`로 떠 있는 데몬을 찾아 다시 붙는다(783-797). 데몬을 끄는 것은 전체 종료(cleanQuit, G8)뿐이다 | 불가피(daemon 부재 시 부트스트랩) |
| G8 | 660-704 (689-703) | PROC | daemon 종료: `POST /api/daemon/shutdown` 후 15초 안에 안 끝나면 **자기 PID만** kill(`taskkill /PID /F`, SIGKILL) | API 우선, kill은 대비책 |
| G9 | — | — | ~~cleanQuit의 이름 기반 `taskkill /IM saba-core.exe`·`pkill -f saba-core`~~ → 해결: cleanQuit(736-769)은 unregister(shutdown=true) 후 `stopDaemon()`만 부르고, 이름 기반 kill은 없다(659 주석) | |
| G10 | 2525-2566 | PROC | `daemon:restart`: `stopDaemon()`(API 우선) 후 `startDaemon()`으로 직접 spawn | 재기동 API가 없다 |
| G11 | 1241-1256 | PROC | 트레이 메뉴: "데몬 종료"는 `stopDaemon()`, "데몬 시작"은 `startDaemon()` 직접 호출 | |
| G12 | 1364-1368, 1385-1395 | PROC | before-quit(cleanQuit을 거치지 않은 경로)에서 자기 데몬 `forceKillDaemon`, `process.on('exit')`에서 SIGKILL. 인터페이스만 종료한 경우(`keepDaemonAlive`)는 둘 다 건너뛴다 | 둘 다 자기가 띄운 프로세스만 |
| G13 | — | — | ~~`node scripts/mock-release-server.js` spawn~~ 없어짐(f6ca7b6). 개발자가 직접 실행한다 | |
| G14 | — | — | ~~updater exe detached spawn(`--apply --relaunch`)~~ → 해결: GUI는 `POST /api/updates/launch-updater {relaunch}`만 보내고 업데이터는 데몬이 띄운다(main.js:2461-2482, `src/ipc/updates.rs:570-639`) | |
| G15 | — | — | ~~updater exe 경로 탐색(`existsSync`)~~ → 해결: 데몬의 `updater_binary()`가 찾는다(`updates.rs:642-649`) | |
| G16 | 2599-2667 (2629, 2653) | OS/PROC | `reg query HKCU\…\Uninstall\Saba-chan`, installer `--uninstall` spawn | |
| G17 | — | — | ~~relay `PATCH /api/hosts/:hostId/bot-config` 직접 호출~~ 없어짐: 봇 설정은 `PUT /api/config/bot`, 릴레이 동기화는 에이전트 하트비트 | |
| G18 | 2816-2855 | OS | `shell.openExternal`(http(s)만), `shell.openPath`(로그 폴더, 인스턴스 폴더) | UI 성격 |
| G19 | 2709-2747 | OS | 네이티브 파일/폴더 다이얼로그 | UI 성격(경로 선택만 하고, 실제 스캔은 daemon) |
| G20 | — | — | ~~`http.request` 전역 monkey-patch~~ → 해결(D12): 토큰은 axios 인터셉터가 데몬 요청에만 붙인다 | |

### 7.2 GUI renderer (`saba-chan-gui/src`)
| # | 위치 | 분류 | 내용 |
|---|---|---|---|
| R1 | i18n.js:100-139, SettingsModal.js:249 | FS-W(localStorage) | 언어 캐시 `i18nextLng` |
| R2 | utils/themeManager.js:50-120 | FS-W(localStorage) | 테마와 커스텀 테마. config.json `gui`에도 accent 계열을 저장하므로 저장소가 둘로 갈린다 |
| R3 | Modals/NoticeModal.js:15-25 | FS-W(localStorage) | 공지와 알림 기록 |
| R4 | i18n.js:3-33 | 빌드타임 | 로케일 JSON 정적 번들. 런타임 locale 업데이트(daemon의 Locales 컴포넌트 자동 적용. GUI 알림에서는 빼기만 한다, main.js:961-969)가 renderer에는 반영되지 않는다 |
| R5 | components/ServerCard.js:419-421, App.js:494-501 | 규칙 중복 | managed 여부를 `module_settings.managed_start`와 모듈 `interaction_mode`로 다시 계산한다. 데몬은 `GET /api/servers`의 `managed`로 판정을 주는데(`handlers/server.rs:81`), 규칙이 다르다: 값이 없으면 데몬은 늘 managed(`supervisor/mod.rs:1479-1488`), renderer는 `console` 모드 모듈만 managed로 본다 |

(renderer는 `fetch`, `axios`, 직접 IP 접근을 하지 않는다. 모두 preload를 거친다.)

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
