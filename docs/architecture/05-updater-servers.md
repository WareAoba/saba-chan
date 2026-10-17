# 05. 업데이터 · 인스톨러 · 릴리즈 파이프라인 · server-chan (릴레이/챗봇/음악)

> 작성 기준: 코드만 근거로 삼았다(문서는 오래돼서 믿을 수 없다). 경로 표기 규칙은 다음과 같다.
> - `saba-chan/…` = saba-chan 저장소 루트
> - `server-chan(git)/…` = server-chan 저장소 루트 (HEAD `4c423e8 정리6`)
>
> 비밀값은 출력하지 않았다. 파일이 있다는 사실과 경로만 적는다.

---

## 0. 소유자 프레이밍과 전체 그림

소유자 프레이밍은 다음과 같다.
- **daemon(saba-core)이 서비스의 주인이다.** 모든 기능은 daemon을 거쳐 실행된다.
- **인터페이스(GUI/CLI)는 스스로 아무것도 하지 않는다.**
- **modules / plugins(=extensions)는 Python으로 만든다.**
- **server-chan은 daemon과 함께 Discord bot을 돌리고, 그 bot에 추가 기능을 제공한다.**

코드에서 확인한 실제 연결 구조는 아래와 같다. `cloud` 모드 기준이다.

```
 Discord 사용자
    │ (prefix 메시지 / 슬래시 /사바쨩)
    ▼
[server-chan relay 프로세스] ── 내장 Discord bot (discord.js, BOT_ENABLED=true)
    │  bot → POST /api/command (x-internal-secret, SELF_URL 루프백)
    │  PostgreSQL command_queue 에 적재 → pollWaiters.wake(hostId)
    ▼
GET /poll (long-poll 25s, HMAC 서명)  ◄──── [saba-chan discord_bot 프로세스 = relayAgent]
POST /result/:id                     ────►   (Discord에 로그인하지 않음)
POST /heartbeat (60s)                         │ mock Message → core/processor.js
                                              ▼
                                       daemon IPC http://127.0.0.1:57474 (X-Saba-Token)
                                              ▲
                     daemon이 discord_bot을 ext-process "discord-bot"으로 spawn
                     (src/main.rs:669 auto_start_discord_bot) — GUI도 같은 일을 한다(§6.1)

[GUI/CLI] ──► daemon /api/relay/* (pairing·host/node 조회 proxy, relay.rs) ──► relay /api/pair/*, /api/hosts/*, /api/nodes/*
```

- **local 모드**에서는 `discord_bot`이 사용자 본인의 `DISCORD_TOKEN`으로 Discord에 직접 로그인한다(`saba-chan/discord_bot/index.js:156-330`). server-chan은 관여하지 않는다.
- **cloud 모드**에서는 Discord를 향하는 bot이 server-chan 쪽의 중앙 bot이다(`server-chan(git)/relay-server/src/bot/index.ts`). saba-chan의 `discord_bot`은 명령을 실제로 실행하는 "relay agent" 역할만 한다(`discord_bot/core/relayAgent.js`).
- 운영 쪽 부가 구성요소(music-server, chatbot-server)는 server-chan 비공개 문서에서 다룬다(§5).

---

## 1. Updater

### 1.1 크레이트 구성

| 크레이트 | 경로 | 버전 | 역할 |
|---|---|---|---|
| `saba-chan-updater-lib` | `saba-chan/updater/` (`Cargo.toml` v0.1.1) | 0.1.1 | 실제 로직(`UpdateManager`). daemon이 **in-process로 링크**하고 updater exe도 쓴다 |
| `saba-chan-updater` (bin, Tauri) | `saba-chan/updater/gui/src-tauri/` | 0.1.1 (`tauri.conf.json:3`) | **apply 전용** GUI exe. `--apply` 없이 실행하면 종료한다 |
| 루트 workspace | `saba-chan/Cargo.toml:12` | — | members = `.`, `updater`, `updater/gui/src-tauri`, `installer/gui/src-tauri`, `saba-chan-cli` |

`updater/src/constants.rs`는 "모든 크레이트가 공유하는 SSOT"라고 스스로 설명한다(`constants.rs:1-4`). daemon도 `saba_chan_updater_lib::constants::*`로 data dir, 토큰 경로, IPC 포트(57474, `constants.rs:16-18`)를 가져다 쓴다. 즉 경로/상수 SSOT가 updater 라이브러리 안에 들어 있다.

**죽은 코드**: lib.rs 헤더(`lib.rs:11-17`)는 "v2 아키텍처: worker.rs / foreground.rs / queue.rs / scheduler.rs / ipc.rs"를 설명한다. 하지만 `BackgroundWorker`, `AutoCheckScheduler`, `ForegroundApplier`, `SelfUpdater`, `DownloadQueue`, `DaemonIpcClient`를 쓰는 곳은 workspace 어디에도 없다(grep으로 확인, 사용처 0건). 실제로 쓰이는 것은 `UpdateManager`와 `UpdateCompletionMarker`뿐이다. `updater/gui/src-tauri/src/cli.rs.removed`, `lib.rs.bak` 같은 잔재 파일도 남아 있다.

### 1.2 컴포넌트 모델

`enum Component` (`updater/src/lib.rs:98-107`)와 manifest key의 대응(`lib.rs:111-141`):

| Component | manifest key | 적용 방식 | 적용 우선순위 (`lib.rs:1400-1414`) |
|---|---|---|---|
| `Updater` | `updater` | `apply_binary_update("saba-chan-updater")` | 0 |
| `Module(name)` | `module-{name}` | `modules_dir/{name}`에 zip 해제. 백업은 `staging/{name}_backup` (`lib.rs:1947-1999`) | 1 |
| `Extension(name)` | `ext-{name}` | `extensions_dir`에 해제. 하이픈은 언더스코어로 변환(`lib.rs:425-444`, 2001-) | 1 |
| `Locales` | `locales` | `install_root/locales`에 해제 | 1 |
| `DiscordBot` | `discord_bot` | `discord_bot/` 교체. `node_modules`는 백업에서 복구(`lib.rs:2464-2580`) | 2 |
| `CoreDaemon` | `saba-core` | `apply_binary_update("saba-core")` | 3 |
| `Gui` / `Cli` | `gui` / `cli` | 프로세스가 돌고 있으면 5, 아니면 4 | 4/5 |

- 알 수 없는 key(예: `installer`)는 업데이트 대상에서 빠진다(`lib.rs:690-697`). `from_manifest_key`는 모르는 key를 `Module`로 폴백한다(`lib.rs:144-146`).
- **로컬 버전 판정 순서**(`lib.rs:946-1010`): 먼저 `installed-manifest.json`(`%APPDATA%/saba-chan/installed-manifest.json`)을 본다. 거기 없으면 Cargo.toml/package.json/module.toml/extension.toml을 읽는다.
  - ⚠️ **버그**: `saba-core`의 폴백이 `env!("CARGO_PKG_VERSION")`이다(`lib.rs:956`). 이 매크로가 평가되는 곳은 **updater-lib 크레이트**라서 결과는 updater-lib 버전이지 core 버전이 아니다. 지금은 둘 다 0.1.1이라 문제가 드러나지 않았을 뿐이다.
  - "설치돼 있는데 버전을 모르면 최신으로 간주하고 installed-manifest에 기록"하는 로직이 있다(`lib.rs:702-715`). 거짓 양성을 막으려는 것이지만, 실제로 구버전이어도 영구히 최신으로 표시될 수 있다.

### 1.3 Release manifest 형식

GitHub Release asset `manifest.json`의 구조는 `github.rs`의 `ReleaseManifest`/`ComponentInfo`에 정의돼 있다.

```jsonc
{
  "release_version": "0.2.0",
  "components": {
    "saba-core": {
      "version": "0.2.0",
      "asset": "saba-core-windows-x64.zip",       // Option — 없으면 이번 릴리즈엔 바이너리 없음
      "asset_linux": "saba-core-linux-x64.tar.gz",// Option — 비Windows에서 우선
      "sha256": "…",                               // Option
      "install_dir": ".",                          // Option, install_root 기준
      "requires": { "saba-core": ">=0.3.0" }       // Option (의존성 체크용)
    }
  }
}
```

- 템플릿은 `saba-chan/release-manifest.json:1-52`다. 버전 0.1.0, 모든 `sha256: null`, `asset_linux`가 있고 `installer`가 포함돼 있다.
- **모듈 매니페스트**(saba-chan-modules 릴리즈 asset) 형식: `{ "schema_version":1, "modules": { "<name>": { "sha256":…, … } } }` (`integrity.rs:559-562`).
- **익스텐션 매니페스트** 형식: `{ "schema_version":1, "extensions": { "<name>": {…} } }` (`integrity.rs:516-519`).

### 1.4 어디서 가져오는가

| 용도 | URL | 근거 |
|---|---|---|
| Core 릴리즈 목록 | `GET https://api.github.com/repos/WareAoba/saba-chan/releases?per_page=30` | `github.rs:121-130`, `lib.rs:664` |
| Core manifest | 릴리즈 asset `manifest.json`의 `browser_download_url` | `github.rs:167-186` |
| 모듈 업데이트 확인 | `module.toml`의 `[update].github_repo`(모듈마다 별도 repo)에서 releases?per_page=5, tag `vX.Y.Z`, asset `module-{n}.zip`/`{n}.zip` | `lib.rs:745-835` |
| 익스텐션 업데이트 확인 | `extension.toml`의 `[update].github_repo`, asset `ext-{n}.zip`/`{n}.zip` | `lib.rs:838-920` |
| 무결성용 모듈 manifest | `WareAoba/saba-chan-modules` 최신 릴리즈 asset `manifest.json` | `lib.rs:512-521`, 542-556 |
| 무결성용 익스텐션 manifest | `WareAoba/saba-chan-extensions` 최신 릴리즈 asset `manifest.json` | `lib.rs:523-532` |
| (상수) 모듈 manifest | `https://github.com/WareAoba/saba-chan-modules/releases/latest/download/manifest.json` | `constants.rs:20-32` — daemon `src/ipc/handlers/server.rs:444,466`에서 사용 |
| (상수) 익스텐션 manifest | `https://raw.githubusercontent.com/WareAoba/saba-chan-extensions/main/manifest.json` (**릴리즈가 아니라 main 브랜치**) | `constants.rs:34-39` |

`api_base_url`로 mock 서버를 지정할 수 있다(`lib.rs:318-321`, `github.rs:101-118`). GitHub 인증 토큰은 쓰지 않는다. 그래서 비인증 rate limit(60회/시간)을 받는데, 체크 한 번에 core 1회, manifest N회, 모듈/익스텐션 repo마다 1회를 호출한다.

**Walk-back resolver**(`github.rs:265-400`): 최신 릴리즈 manifest로 목표 버전을 정한다. asset이 없는 컴포넌트는 이전 릴리즈들을 뒤져 **같은 버전**의 asset을 찾는다. Windows는 `asset`, 그 외 플랫폼은 `asset_linux ?? asset`을 쓴다(`github.rs:291-297`).

### 1.5 무결성 검증

- `UpdateManager::verify_integrity` (`lib.rs:491-540`): 서버 manifest 3종(core/modules/extensions)에서 기대 해시를 모은 뒤 `IntegrityChecker::verify_all`로 검사한다. 결과 상태는 `Verified`, `Tampered`, `NoHash`, `FileNotFound`, `Error` 중 하나다(`integrity.rs:48-59`).
- SHA-256은 **직접 구현**했다(`integrity.rs:321-455`, "순수 Rust SHA-256 구현"). sha2 크레이트를 쓰지 않는다. 테스트 벡터는 있다(`integrity.rs:797-830`).
- 해시 대상 파일(`integrity.rs:246-296`): core/cli/updater는 `install_root/<exe>`, gui는 `install_root/<install_dir>/saba-chan-gui(.exe)`, discord_bot은 `…/discord_bot/index.js`.
- ⚠️ **해시 의미 불일치(치명적)**: CI(`package-release.py:474-481`)가 manifest에 넣는 `sha256`은 **zip asset 파일의 해시**다. 반면 integrity checker가 해시하는 것은 **압축을 푼 exe나 index.js**다. 그래서 해시가 들어간 모든 컴포넌트가 항상 `Tampered`로 판정될 수밖에 없다.
- ⚠️ **다운로드할 때는 해시를 검증하지 않는다**: `download_available_updates`(`lib.rs:1133-1182`)도 `download_component`(`lib.rs:1189-1279`)도 `ResolvedComponent.sha256`을 무시한다. 인스톨러도 같다.
- daemon API는 `GET /api/updates/integrity`이다(`src/ipc/updates.rs:90,478`).

### 1.6 다운로드 → apply → swap → rollback 흐름

1. **확인**: `POST /api/updates/check` → `check_for_updates()` (`updates.rs:142`, `lib.rs:573-645`). Locales 업데이트가 있으면 사용자에게 알리지 않고 백그라운드에서 다운로드·적용한다(`updates.rs:157-163`, `silent_apply_locales` 206).
2. **다운로드**: `POST /api/updates/download` → staging 디렉터리로 받는다.
   - Windows: `%APPDATA%/saba-chan/updates/`
   - Unix: `~/.cache/saba-chan/updates/` (`constants.rs:189-205`)
   - 진행률은 `GET /api/updates/download/progress`로 본다(`updates.rs:332`).
3. **apply 분기**: `POST /api/updates/apply {components:[…]}` (`updates.rs:353-475`)
   - 대상에 `Gui`, `Updater`, `CoreDaemon` 중 하나라도 있으면 **전부 updater exe에 맡긴다**(`updates.rs:392-400`). 이때 daemon은 `staging/pending.json`(`lib.rs:1477-1525`)과 `staging/apply-targets.json`(`lib.rs:1540-1565`)을 쓰고 `{requires_updater:true, needs_updater:[…]}`를 응답한다.
   - 그렇지 않으면 daemon이 직접 `apply_single_component`를 실행한다. 그 뒤 모듈은 `supervisor.refresh_modules()`, 익스텐션은 `extension_manager.rescan()`으로 hot-reload한다(`updates.rs:401-466`).
4. **updater exe 실행**은 daemon이 아니라 **GUI가 spawn한다**: `ipcMain 'updater:launchApply'` → `saba-chan-updater --apply --relaunch <gui exe>` → GUI 종료(`saba-chan-gui/main.js:2744-2775`). CLI의 `launch-apply`도 직접 spawn한다(`saba-chan-cli/src/tui/commands.rs:1134-1160`).
5. **updater exe** (`updater/gui/src-tauri/src/lib.rs`)
   - 인자는 `--apply [--relaunch <exe> [extra…]]`만 파싱한다(`lib.rs:362-381`). install_root는 자기 exe 위치로 정한다(`lib.rs:390-396`).
   - `load_pending_manifest()` → `apply-targets.json`의 key를 차례로 `apply_single_component()` → `UpdateCompletionMarker` 저장(`%APPDATA%/saba-chan/update-complete.json`) → `pending.json`/`apply-targets.json` 삭제 → 2초 뒤 `--after-update`를 붙여 relaunch(`lib.rs:73-178`, 300-321). 진행 상황은 Tauri 이벤트 `apply:progress`로 알린다.
   - Tauri 커맨드: `get_apply_mode`, `start_apply`, `get_preferred_language`, `get_theme`, `check_after_update`.
6. **바이너리 swap** (`lib.rs:2113-2204`, gui 2206-2300)
   - 대상 프로세스가 끝나기를 최대 15초 기다린다. 시간이 지나도 "attempting update anyway"로 그냥 진행한다.
   - Windows는 실행 중인 `X.exe`를 `X.exe.old`로 rename하고(재시도 5회, `lib.rs:2083-2111`) zip에서 새 파일을 쓴다. Linux는 `X.old`로 rename한다.
   - 적용이 끝나면 `cleanup_old_files()`가 install_root의 `*.old`를 지운다(`lib.rs:1428-1466`).
7. **rollback**
   - 모듈, 익스텐션, discord_bot은 staging에 `*_backup` 디렉터리 복사본을 남긴다. 하지만 **자동 복구 경로는 없다**. 예외는 discord_bot의 `node_modules` 복원 하나뿐이다(`lib.rs:2554-2575`).
   - 바이너리는 `.old`를 만들지만 적용 직후 `cleanup_old_files`가 지우므로 rollback 수단이 남지 않는다.
   - `prepare_daemon_update`(`lib.rs:2303-2405`)는 `_update_daemon.ps1`/`.sh`를 생성하는 함수인데 **호출하는 곳이 없다**(죽은 코드). 테스트 `test_daemon_update_script_content`만 이 함수를 참조한다.
8. **설치 버전 기록**: `update_installed_versions_batch` → `installed-manifest.json`. Windows는 레지스트리 `DisplayVersion`도 갱신한다(`lib.rs:1366-1381`, 1633).

### 1.7 daemon, GUI, CLI가 updater를 부르는 방식

**daemon HTTP API** (`saba-chan/src/ipc/updates.rs:83-94`, `ipc/mod.rs:669`에서 merge):

| Method | Path | 동작 |
|---|---|---|
| GET | `/api/updates/status` | 캐시된 상태. Locales는 숨긴다 |
| POST | `/api/updates/check` | GitHub를 조회한다 |
| POST | `/api/updates/download` | 다운로드 |
| GET | `/api/updates/download/progress` | 진행률 |
| POST | `/api/updates/apply` | body `{components:[]}`. 직접 적용하거나 updater 위임을 판단한다 |
| GET | `/api/updates/integrity` | SHA256 검증 |
| GET / **PUT** | `/api/updates/config` | 설정. **메모리에만 저장**되고 재시작하면 `UpdateConfig::default()`로 돌아간다(`updates.rs:530-570`, 579) |

불일치:
- GUI의 `updater:setConfig`는 **POST**로 보낸다(`saba-chan-gui/main.js:2792`). 라우트는 PUT뿐이라 405가 난다. `syncInstallRoot`(`main.js:992`)는 PUT을 써서 정상이다.
- CLI의 `apply_updates`는 body 없이 POST한다(`saba-chan-cli/src/client.rs:371-373`, `post_empty`). axum의 `Json<ApplyRequest>` extractor는 Content-Type이 없으면 415로 거부할 가능성이 높다(미실행, 추정).
- CLI는 `/api/install/status`, `/api/install/run`, `/api/install/component/{key}`를 호출한다(`client.rs:382-400`). daemon에는 `/api/install*` 라우트가 **없다**(grep 0건). CLI의 `install` 계열은 동작하지 않는다.
- CLI `launch-apply`가 넘기는 `--install-root <root>`와 대상 인자(`commands.rs:1147`)를 updater exe는 **무시한다**(`--apply`/`--relaunch`만 파싱). 후보 경로도 `.exe`로 고정돼 있어 Windows 전용이다.
- 자동 확인 주기(`check_interval_hours=3`)를 실제로 돌리는 것은 daemon이 아니라 **GUI의 `setInterval`**이다(`main.js:1153` `UPDATE_CHECK_INTERVAL_MS`, 1250-1262). `auto_download`/`auto_apply` 설정은 daemon 어디에서도 쓰이지 않는다.

---

## 2. Installer (`saba-chan/installer/gui`, Tauri, v0.1.1)

모듈 구성: `lib.rs`(1503줄), `github.rs`, `runtime_bootstrap.rs`, `registry.rs`, `shortcuts.rs`, `uninstall.rs`.

**설치 위치**
- 기본 경로는 Windows `C:\Program Files\Saba-chan`, Unix `~/.local/share/saba-chan`(`installer/gui/src-tauri/src/lib.rs:1029-1046`).
- 쓰기에 실패하면 UAC로 권한을 올려 디렉터리를 만든다(`elevate_create_dir`, `lib.rs:919-980`).
- 데이터는 `%APPDATA%/saba-chan`에 둔다(updater constants와 같다).

**`do_install` 단계** (`lib.rs:384-875`)

| 단계 | 내용 |
|---|---|
| 1 (0-5%) | install dir 생성, 쓰기 probe, 필요하면 권한 상승 |
| 2 (5-10%) | `GET api.github.com/repos/{owner}/{repo}/releases?per_page=20` → tag manifest (`github.rs:44-128`) |
| 2.5 | manifest의 `installer.version`이 현재 `CARGO_PKG_VERSION`보다 높으면 설치를 차단한다(`lib.rs:489-505`) |
| 3 (10-45%) | `installer`를 뺀 core 컴포넌트 asset을 스트리밍으로 받아 `%TEMP%/saba-chan-installer`에 둔 뒤 `install_dir/<install_dir>`에 zip 해제. **sha256은 검증하지 않는다** |
| 3.5 | 공유 모듈(`i18n.py`, `daemon_rcon.py`)을 `%APPDATA%/saba-chan/modules/`로 복사 |
| 4 (45-55%) | **`saba-chan-modules` repo의 zipball(main 브랜치 소스 전체)**을 받아 선택된 모듈만 추출. `module.toml [install].requires_extensions`에 따라 익스텐션 manifest(raw main)에서 익스텐션을 설치(`lib.rs:621-700`, 1264-) |
| 5 (55-70%) | portable Python(`indygreg/python-build-standalone`)과 venv (`runtime_bootstrap.rs:321`). 실패해도 치명적으로 처리하지 않는다 |
| 6 (70-85%) | portable Node(`nodejs.org/dist`, `runtime_bootstrap.rs:346`)와 `discord_bot` npm install |
| 7-8 | `setup_config`, 언어 설정을 `settings.json`에 저장 |
| 9 | `HKCU\Software\Microsoft\Windows\CurrentVersion\Uninstall\Saba-chan` 등록(`registry.rs:3,11,22`) |
| 10 | 바탕화면·시작 메뉴 바로가기(`shortcuts.rs`) |
| 10.5 | `installed-manifest.json` 생성. updater 버전 판정의 1순위 소스다 |

Tauri 커맨드(`lib.rs:109-1015`): `get_installer_state`, `set_install_path`, `set_language`, `set_shortcut_options`, `set_selected_modules`, `get_available_modules`(GitHub Contents API와 raw `module.toml`, 실패 시 하드코딩 폴백), `fetch_latest_release`, `start_install`, `start_uninstall`, `get_app_mode`, `launch_app`, `get_preferred_language`, `browse_folder`. GUI는 `--uninstall` 인자로 installer를 uninstall 모드로 실행한다(`saba-chan-gui/main.js:2945`).

**문제점**
- 기본 설치 경로가 `Program Files`(관리자 권한 필요)인데 레지스트리는 HKCU에 쓴다. 더 큰 문제는 updater exe와 daemon이 **권한 상승 없이** `install_root`에 쓴다는 점이다. 기본 경로에 설치하면 업데이트 적용이 권한 문제로 실패할 수 있다(추정, 매니페스트의 requestedExecutionLevel은 확인하지 않음).
- 모듈 소스의 출처가 네 갈래로 서로 다르다.
  1. installer: modules repo **main zipball**
  2. daemon `server.rs:444-466`: modules repo **releases/latest manifest와 asset**
  3. updater 업데이트 확인: `module.toml`이 가리키는 **모듈별 repo**
  4. 무결성 검사: modules repo **최신 릴리즈 manifest**
- `installer/KNOWN_ISSUES.md`는 "2026-03-05 5건 모두 해결"이라고 적혀 있다. 해시 검증 미구현은 이 목록에 없다.

---

## 3. CI / 릴리즈 파이프라인

### 3.1 saba-chan (`.github/workflows`)

| 파일 | 트리거 | 내용 |
|---|---|---|
| `ci.yml` | push/PR to main, develop | Rust matrix(os): fixture 모듈 생성 → `cargo clippy --workspace -D warnings` → build → test. JS: GUI vitest, discord_bot test |
| `test.yml` | push/PR to main, develop | `ci.yml`과 거의 중복(`cargo build --release`, `cargo test --workspace`, vitest, npm test, artifact 업로드) |
| `release.yml` | `release: [published, edited]`, `workflow_dispatch(tag_name)` | **빌드는 하지 않는다.** 사람이 Draft Release에 직접 올린 raw exe/zip을 `gh release download` → `package-release.py`로 표준 zip 재패키징 → 빠진 컴포넌트는 이전 릴리즈에서 가져옴 → `locales.zip` 생성 → `manifest.json`(sha256 포함) 생성 → `--clobber` 업로드 → output에 없는 raw asset 삭제(`release.yml:43-147`) |

- 실제 빌드는 로컬 Windows의 `scripts/build-windows.ps1`이 한다. 내용은 `cargo build --release --workspace`, GUI `npm run build`/`package`, discord_bot zip, locales 복사다(`build-windows.ps1:58-271`). `build-core.ps1`은 core를 GUI bin으로 복사하고, `build-docker-io.ps1`은 docker 익스텐션의 Go 바이너리 `saba-docker-io`를 빌드한다. `run-test.{ps1,sh}`는 형제 디렉터리 `../server-chan/relay-server`가 있으면 relay E2E까지 돌린다(`run-test.sh:46-50,123-141`).
- `package-release.py`의 `COMPONENTS`(`:47-112`)는 **windows-x64 asset만** 만들고 `asset_linux`는 만들지 않는다. 템플릿(`release-manifest.json`)과 updater(`github.rs:291`)는 Linux asset을 전제로 하므로 맞지 않는다. Linux에서는 Windows zip으로 폴백하게 된다.
- `.github/scripts/generate-manifest.py`는 어떤 workflow에서도 호출되지 않는다. `sha256: None`으로 고정하는(`:188`) 레거시 스크립트다. `release-manifest.json`의 `_comment`는 "버전은 CI 스크립트(generate-manifest.py)가 덮어쓴다"고 하지만 이 설명은 틀렸다.
- `release.yml`은 `ref: main`으로 checkout한다(`:74`). 그래서 태그 시점이 아니라 main의 locales와 버전 파일이 manifest에 들어간다.
- `.github/WORKFLOWS_GUIDE.md`는 "5개 워크플로우(coverage.yml 등)"라고 쓰지만 실제로는 3개다.

### 3.2 modules / extensions repo (소비자 쪽 기준 요약)

- `saba-chan-modules`: GitHub Release `latest`에 `manifest.json`(`{schema_version, modules:{name:{sha256,…}}}`)과 `module-<name>.zip`을 올린다(`constants.rs:20-32`, `integrity.rs:559-600`). 모듈마다 `module.toml`에 `[update] github_repo`가 있으면 그 repo의 tag `vX.Y.Z` 릴리즈를 업데이트 소스로 쓴다.
- `saba-chan-extensions`: updater integrity는 **릴리즈 asset** `manifest.json`을 보고, installer와 constants는 **raw main/manifest.json**을 본다. 둘 중 하나는 어긋나게 된다.
- 두 repo의 CI 정의(`.github/workflows`, `build_modules.py`, `build_extensions.py`)는 [02-modules.md](02-modules.md) §5와 [03-extensions.md](03-extensions.md) §3.4에서 다룬다.

---

## 4. Relay server (server-chan, git 저장소)

> ⚠️ 프레임워크는 **Fastify 5**다(`relay-server/package.json`: fastify, @fastify/helmet/cors/rate-limit, drizzle-orm+postgres, argon2, discord.js 14, zod). "Hono"가 아니다. DB는 PostgreSQL, ORM은 drizzle이며 기동할 때 `./drizzle`을 자동 migrate한다(`src/index.ts:113-120`).

### 4.1 전역 동작

- `onRequest` 훅(`src/index.ts:139-146`): `/health`를 빼고, `X-Saba-Client: 1` 헤더나 `x-internal-secret` 헤더(**값과 상관없이 존재만 확인**)가 없으면 444를 돌려준다. nginx의 `map $http_x_saba_client`(git `deploy/nginx/nginx.conf:31`)와 함께 2중 필터를 이룬다.
- CORS는 `app://`, `tauri://`, 그리고 dev 모드의 localhost만 허용한다(`index.ts:79-104`).
- rate limit은 전역 100/min이고, 라우트별 설정은 `middleware/rateLimit.ts:8-29`에 있다.
- 프로덕션 필수 env: `RELAY_INTERNAL_SECRET`, `DISCORD_TOKEN`, `DATABASE_URL`, `SELF_URL`(`index.ts:27-35`).
- **인증 3종**
  - `authenticateNode` (`middleware/auth.ts:65-158`): `Authorization: Bearer sbn_<hostId>.<secret>` → DB `hosts.token_hash`를 argon2id로 검증(5분 캐시) → `x-request-timestamp` ±30초 → `x-request-nonce` 재사용 검사(**선택 사항**: 없으면 경고만, `auth.ts:120-130`) → `x-request-signature = hex(HMAC-SHA256(secret, METHOD\nreq.url\nts\nnonce\nJSON.stringify(body)))`. `req.node={id:hostId}`를 설정한다.
  - `verifyInternal` (`middleware/internalAuth.ts:17-27`): `x-internal-secret`을 `RELAY_INTERNAL_SECRET`과 상수시간 비교한다. **시크릿을 설정하지 않으면 그냥 통과한다(dev)**.
  - `authenticateNodeOrInternal` (`auth.ts:174-201`): internal 헤더를 먼저 보고, 없으면 node 토큰을 본다. 둘 다 없으면 시크릿 미설정 시 통과, 아니면 401이다.
- 토큰 형식: `sbn_{nodeId}.{nanoid(48)}`. 여기서 nodeId는 사실 **hostId(nanoid 12)**다(`services/nodeToken.ts:9-20`). 같은 대상을 "node"와 "host"라는 두 이름으로 부르고 있다.

### 4.2 HTTP API 표 (git HEAD)

| Method | Path | Auth | Body / Query | Response | 위치 |
|---|---|---|---|---|---|
| GET | `/info` | (X-Saba-Client) | — | `{name, version, apiVersion:2, minAgentVersion:'0.1.0'}` | `index.ts:157-162` |
| GET | `/health` | 없음 | — | `{status, uptime, db}` 200/503 | `index.ts:165-176` |
| GET | `/bot-status` | internal | — | `{connected,…}` | `index.ts:179` |
| POST | `/api/pair/initiate` | 없음(rate 10/min) | `{relayUrl?}` | `{code(nanoid32), pollSecret(nanoid48), expiresAt, ttl:300}`. pollSecret은 sha256으로 저장 | `routes/pair.ts:17-50` |
| GET | `/api/pair/:code/status` | pollSecret (`X-Poll-Secret` 헤더 우선, `?secret=` 하위호환) | — | `pending`이면 `{status, expiresAt}` / `expired` / `claimed`이면 `{status, hostId, nodeToken}`. **nodeToken은 원자적 UPDATE로 1회만 반환** | `pair.ts:54-136` |
| POST | `/api/pair/:code/claim` | internal | `{discordId, username, name?, guildId?, guildName?, locale?}` | `{ok, hostId, guildId, nodeCreated, message}`. 트랜잭션 + `FOR UPDATE`, 5회 시도 제한. 기존 host는 토큰을 재발급한다 | `pair.ts:139-266` |
| DELETE | `/api/pair/cleanup` | internal | — | `{cleaned:true}` | `pair.ts:269-285` |
| POST | `/api/hosts/:hostId/rotate-token` | internal | `{discordId}`(owner여야 함) | `{hostId, token, warning}` | `routes/host.ts:11-45` |
| GET | `/api/hosts/:hostId` | node(자기 host만) 또는 internal | — | `{id,name,status,lastHeartbeat,agentVersion,createdAt}` | `host.ts:48-71` |
| GET | `/api/hosts/:hostId/metadata` | node/internal | — | heartbeat metadata(JSON) | `host.ts:74-93` |
| PATCH | `/api/hosts/:hostId/bot-config` | internal | `{prefix?, moduleAliases?, commandAliases?}` | `{success, botConfig}` | `host.ts:96-128` |
| POST | `/api/nodes` | internal | `{guildId, guildName?, hostId, ownerDiscordId}` | `{guildId,hostId,status:'created'}` / 403 / 409 | `routes/node.ts:30-74` |
| GET | `/api/nodes/:guildId` | internal | — | node row | `node.ts:77-93` |
| GET | `/api/nodes/:guildId/metadata` | internal | — | `{hostId, hostStatus, …metadata}` | `node.ts:96-121` |
| GET | `/api/hosts/:hostId/nodes` | node(자기 host)/internal | — | node[] | `node.ts:124-145` |
| GET | `/api/nodes/:guildId/members` | node(자기 host의 길드)/internal | — | `[{id,username,displayName,level}]` (node_permissions) | `node.ts:148-184` |
| GET | `/api/nodes/:guildId/discord-members` | node/internal | — | bot이 실시간으로 조회한 길드 멤버. bot이 없으면 503 `BOT_UNAVAILABLE` | `node.ts:187-214` |
| DELETE | `/api/nodes/:guildId` | internal | `{discordId}`(owner) | `{status:'deleted'}` | `node.ts:217-237` |
| POST | `/api/command` | internal | `{hostId?, guildId?, payload(1–50 keys), requestedBy?, channelId?, interactionToken?, requestId?}` | `{requestId, status:'queued'}`. host가 offline이거나 heartbeat가 3분 넘게 없으면 503 `HOST_OFFLINE` | `routes/command.ts:20-99` |
| GET | `/api/command/:requestId/status` | internal | — | `{id,status,result,createdAt,completedAt}` | `command.ts:102-122` |
| GET | `/poll` | node(HMAC) | — | `{commands:[{id,payload,requestedBy,guildId,channelId}]}` / 204 / `{sleeping:true}` | `routes/poll.ts:8-112` |
| POST | `/result/:requestId` | node(HMAC) | `{success:boolean, data:any(≤1MB)}` | `{status:'ok'}` / 404 / 409 | `routes/result.ts:6-85` |
| POST | `/heartbeat` | node(HMAC), body ≤5MB, 4/min | `{agentVersion?, os?, metadata?{modules[], servers[], moduleDetails{}, botConfig?}}` | `{status:'ok', sleeping, warning?:'UPDATE_REQUIRED', minVersion?}` | `routes/heartbeat.ts:23-73` |

zod 스키마는 `middleware/schemas.ts` 한 곳에 모여 있다.

### 4.3 DB 스키마 (`relay-server/src/db/schema.ts`, migration 0000-0007)

| 테이블 | 주요 컬럼 |
|---|---|
| `users` | `discord_id` PK, username, display_name, is_banned, created_at, last_seen |
| `hosts` | `id` PK(nanoid12), `owner_discord_id` **UNIQUE**(사용자 1명당 host 1개) → users, name, `token_hash`(argon2id), status(online/offline), last_heartbeat, agent_version, `metadata` jsonb(heartbeat 캐시와 botConfig), timestamps |
| `nodes` | `guild_id` PK, `host_id` → hosts (cascade), owner_discord_id, guild_name, locale(default 'en') |
| `node_permissions` | PK(guild_id, discord_id), level(1=USER, 2=ADMIN), granted_by |
| `command_queue` | id PK, host_id, guild_id, payload jsonb, requested_by, channel_id, interaction_token, status(pending→delivered→completed / timeout), created/delivered/completed/expires_at(+60s), result jsonb |
| `pairing_codes` | code PK, expires_at(+5분), status, attempts, relay_url, claimed_by, host_id, **node_token(평문, 수령 즉시 NULL)**, claimed_at, poll_secret_hash |
| `audit_logs` | serial id, timestamp, user_discord_id, host_id, guild_id, action, detail, result |

정리 스케줄러(`services/cleanup.ts:16-48`): audit 30일, 완료된 큐 7일, 만료 큐는 timeout 처리, heartbeat가 3분 넘게 없는 host는 offline, pairing 코드는 1일 뒤 삭제.

### 4.4 프로토콜 흐름

**(a) 페어링** (GUI → daemon proxy → relay → Discord 슬래시 → relay)
1. GUI `relay:initiatePairing` → daemon `POST /api/relay/pair/initiate` (`src/ipc/handlers/relay.rs:300-336`) → relay `POST /api/pair/initiate`. 응답은 `{code, pollSecret}`이다.
2. 사용자가 Discord에서 `/사바쨩 연결 코드:<code>`를 실행한다 → relay 내장 bot `handlePair`가 `SELF_URL/api/pair/:code/claim`을 x-internal-secret으로 루프백 호출한다(`bot/index.ts:479-529`). users upsert, hosts 생성 또는 토큰 재발급, guild가 있으면 nodes 자동 생성, `pairing_codes.node_token`에 평문 저장.
3. GUI가 daemon `GET /api/relay/pair/:code/status?secret=…`를 폴링한다(`relay.rs:349-404`). relay가 `claimed`와 함께 nodeToken을 1회 돌려주면 **daemon이 `%APPDATA%/saba-chan/.node_token`에 자동 저장**한다(`relay.rs:374-387`).
4. daemon `GET/PUT/DELETE /api/config/node-token`(`ipc/mod.rs:619`, `handlers/config.rs:47-`)은 토큰 원문을 IPC로 그대로 반환한다.

**(b) 명령 실행** (cloud 모드)
1. Discord 사용자가 `<prefix> <text>`를 입력한다. prefix는 길드별 캐시이고 기본값은 `BOT_PREFIX` env 또는 `'사바쨩'`이다(`bot/index.ts:230,238-281`).
2. `forwardCommand`(`bot/index.ts:730-800`)가 requestId를 먼저 만들고 `resultWaiters.wait(id, 30s)`를 등록한 뒤 `POST /api/command`를 부른다. payload는 `{action:'raw_command', text}`다. `guildId`로 nodes를 조회해 hostId를 찾고, 큐에 INSERT한 뒤 `pollWaiters.wake(hostId)`를 호출한다.
3. agent(`discord_bot/core/relayAgent.js:257-345`)가 `GET /poll`을 계속 돌리고 있다. 서버는 즉시 pending 최대 10건을 조회해 `delivered`로 표시하고 반환한다. 없으면 **25초 대기**(`poll.ts:6`, `pollWaiters.wait`) 후 다시 조회하고, 그래도 없으면 204를 준다. 클라이언트 연결이 끊기면 abort한다. host가 paused 상태면 `{sleeping:true}`를 준다.
4. agent는 받은 명령마다 `processRelayCommand`를 실행한다. mock `Message`를 만들고(content = 로컬 bot-config의 prefix + text, `author.id=requestedBy`) 로컬 `core/processor.js`로 처리한다. processor는 daemon IPC(`IPC_BASE`, `X-Saba-Token` 헤더, `core/ipc.js:25,60-66`)를 호출한다. 마지막 reply 텍스트를 `{success, data:{text}}`로 담아 `POST /result/:id`로 보낸다.
5. relay는 상태를 completed로 바꾸고 `resultWaiters.wake(id)`를 호출한다. bot은 `result.data.text`(최대 1950자)로 Discord에 reply한다.
6. agent의 재시도 백오프는 3초에서 시작해 최대 60초다. sleeping 상태면 30초마다 확인한다.

**(c) Heartbeat**: 60초마다 `POST /heartbeat`(`relayAgent.js:136-180`). metadata = `{servers: ipc.getServers(), modules: ipc.getModules(), moduleDetails:{}}`. 응답의 `sleeping`으로 폴링을 멈추거나 다시 시작한다. 시작할 때 `GET /info`로 `minAgentVersion`을 비교한다.

**(d) Sleep/Wake**: 슬래시 `/사바쨩 코코낸내`/`기상`이 메모리의 `_pausedHosts`/`_pausedGuilds` Set을 토글한다(`bot/index.ts:76-81,647-720`).

### 4.5 daemon 쪽 호출과 relay 서버 비교 (검증 결과)

| daemon 라우트 (`ipc/mod.rs:660-665`) | relay 호출 | 서명 | 일치 |
|---|---|---|---|
| GET `/api/relay/host/:id/status` | GET `/api/hosts/:id` | HMAC(Bearer, ts, nonce, sig) | ✓ (`relay.rs:155-197`) |
| GET `/api/relay/host/:id/nodes` | GET `/api/hosts/:id/nodes` | HMAC | ✓ |
| GET `/api/relay/node/:guild/members` | GET `/api/nodes/:g/discord-members`, 503이면 `/api/nodes/:g/members` | HMAC | ✓ (discord-members는 rate limit 5/min) |
| POST `/api/relay/pair/initiate` | POST `/api/pair/initiate` | 없음 | ✓ |
| GET `/api/relay/pair/:code/status` | GET `/api/pair/:code/status?secret=` | 없음 | ✓. 하위호환 query 방식이라 pollSecret이 nginx access log에 남는다. 헤더 방식으로 바꾸는 편이 낫다 |

- 서명 payload 형식(`METHOD\npath\nts\nnonce\nbody`)은 Rust(`relay.rs:78-112`), JS(`relayAgent.js:58-74`), 서버(`auth.ts:139-148`)가 같다. 서버는 `req.url`(query 포함)로 서명을 확인한다. daemon은 query 없이 서명하는데, 실제로 서명하는 요청에 query를 붙이지 않으므로 현재는 문제가 없다.
- daemon proxy의 `relayUrl` query/body override(`relay.rs:36-42`, `RelayQueryParams`)는 **임의의 URL로 Bearer node token과 HMAC 헤더를 보낼 수 있게 한다**. 로컬 IPC 호출자라면 누구든 node token을 외부로 유출시킬 수 있는 경로다.
- `/poll`, `/heartbeat`, `/result`는 daemon이 아니라 **discord_bot 프로세스**가 부른다. daemon은 bot을 spawn할 때 `RELAY_URL`과 `RELAY_NODE_TOKEN`을 env로 주입한다(`src/main.rs:748-769`).
- relay의 `PATCH /api/hosts/:id/bot-config`(internal 전용)를 daemon은 호출하지 않는다. **GUI가 relay로 직접** 호출한다(§6). `X-Saba-Client`와 internal secret이 없으므로 relay hook에서 444가 나 **항상 실패**한다. 결과적으로 relay bot의 길드별 prefix는 언제나 기본값이다. agent heartbeat도 `botConfig`를 보내지 않는다(`relayAgent.js:139-143`).
- `result.ts:54-80`의 Discord interaction followup은 `interactionToken`이 있을 때만 동작한다. 그런데 `forwardCommand`가 이 값을 넣지 않으므로 죽은 경로다. 또 `data.message/status`를 읽는 반면 agent는 `data.text`를 보내서 필드도 맞지 않는다.
- `node_permissions`(`/사바쨩 권한부여/권한해제`, `bot/index.ts:531-645`)는 relay에서 **목록 조회에만 쓰인다**. 권한을 실제로 강제하는 곳은 로컬 processor의 차단 목록(`discord_bot/core/processor.js:363-372`)뿐이고 이 목록은 기본 허용이다. relay 쪽에서 부여한 권한은 실행 권한에 아무 영향이 없다.

---

## 5. server-chan 운영 구성요소와 운영 결함

> 이 절(공유 폴더에만 있는 chatbot/music-server, 운영 배포 토폴로지)과 §6.3–§6.5(relay 결함, 비밀 파일, 누락 소스)는 운영 서버 보안과 직결돼 공개 저장소에 두지 않는다. 비공개 저장소 `WareAoba/server-chan`의 `docs/architecture-server.md`에서 관리한다.

---

## 6. 불일치 · 누락 소스 · 스텁 · 보안 문제

### 6.1 소유자 프레이밍("daemon이 주인, 인터페이스는 무력") 위반

1. **GUI가 업데이트 주기를 직접 돌린다**: `setInterval(checkForUpdates, 3h)` (`saba-chan-gui/main.js:1153,1250-1262`). daemon 쪽 scheduler/worker(`updater/src/scheduler.rs`, `worker.rs`)는 죽은 코드다.
2. **GUI와 CLI가 updater exe를 직접 spawn한다**: `main.js:2744-2775`, `saba-chan-cli/src/tui/commands.rs:1134-1160`. daemon은 파일(`pending.json`/`apply-targets.json`)만 남긴다.
3. **GUI가 relay를 직접 호출한다**: `axios.patch(${relayUrl}/api/hosts/${hostId}/bot-config)` (`main.js:3216`). `relay.rs:3-4`의 "GUI/CLI must not access the relay server directly"와 정면으로 충돌하고, internal 전용 엔드포인트라 항상 실패한다.
4. **GUI가 Discord bot 실행 명령을 조립한다**: `.node_token`을 직접 읽고(`main.js:377`, 3247) node 경로와 botDir을 정한 뒤 `DISCORD_TOKEN`/`RELAY_NODE_TOKEN`을 env에 넣어 범용 `/api/ext-process/discord-bot/start`에 보낸다(`main.js:3227-3266`). daemon에도 같은 역할의 `auto_start_discord_bot`(`src/main.rs:669-790`)이 있어 **실행 경로가 두 개**다.
5. GUI가 daemon을 spawn하고(`main.js:570`) installer를 `--uninstall`로 실행한다(`main.js:2945`). 인터페이스가 프로세스 감독 역할을 맡고 있다.
6. **extensions 용어 충돌**: daemon의 extensions는 Python 플러그인(`%APPDATA%/saba-chan/extensions`, `extension.toml`)인데, `discord_bot/extensions/{music,rps,easter_eggs}.js`는 bot 안의 JS 모듈이다. `music.js`는 Python 익스텐션(`SABA_EXTENSIONS_DIR/music/music_deps.py`)을 직접 spawn한다(`music.js:37-92`). bot이 daemon을 거치지 않고 Python 익스텐션을 실행하는 셈이다.
7. **server-chan과 프레이밍의 관계**
   - relay는 프레이밍에 맞는다. 다만 cloud 모드에서는 Discord를 향하는 bot이 server-chan 쪽이고, saba-chan `discord_bot`은 실행기로만 동작한다. "server-chan이 bot에 추가 기능을 준다"기보다는 **cloud 모드에서는 server-chan이 bot 그 자체**다.
   - music-server는 relay bot의 부가 기능이지만 소스가 없다.
   - chatbot-server는 daemon과도 relay와도 연결이 없는 **별개의 bot**이라 프레이밍을 벗어난다.
   - updater 청사진은 relay를 업데이트 SSOT로 삼으려 한다. 그렇게 되면 server-chan이 bot 지원을 넘어 배포 인프라까지 맡게 된다.

### 6.2 updater, installer, CI 불일치와 버그

| # | 내용 | 근거 |
|---|---|---|
| U1 | manifest `sha256`은 zip 해시이고 integrity는 압축을 푼 파일 해시라서 항상 `Tampered`가 된다 | `package-release.py:474-481` ↔ `integrity.rs:246-296` |
| U2 | 다운로드와 설치 때 sha256을 검증하지 않는다(updater, installer 모두) | `lib.rs:1133-1279`, installer `lib.rs:512-560` |
| U3 | **Zip-slip**: `entry.name()`을 그대로 `join`한다(`enclosed_name` 미사용) | updater `lib.rs:1976,2028,2166,2501,3004`; installer `lib.rs:1096` |
| U4 | `saba-core` 버전 폴백이 updater-lib 버전이다 | `lib.rs:956` |
| U5 | `prepare_daemon_update`는 사용되지 않는다. rollback 경로가 없고 `.old`는 적용 직후 삭제된다 | `lib.rs:2303`, 1428-1466 |
| U6 | v2 모듈(worker/foreground/queue/scheduler/ipc 대부분)이 죽은 코드다. `cli.rs.removed`, `lib.rs.bak` 잔재 | `lib.rs:11-17,46-55` |
| U7 | `/api/updates/config` 설정이 저장되지 않고 재시작하면 초기화된다. GUI는 POST를 쓰는데 라우트는 PUT이다 | `updates.rs:92,579`; `main.js:2792` |
| U8 | CLI의 `/api/install/*`은 daemon에 없다. CLI apply는 body 없는 POST다 | `saba-chan-cli/src/client.rs:371-400` |
| U9 | CLI `launch-apply`가 넘기는 `--install-root`와 대상 인자를 updater가 무시한다 | `commands.rs:1147` ↔ updater gui `lib.rs:362-381` |
| U10 | CI가 Linux asset을 만들지 않는다(`asset_linux` 없음) | `package-release.py:47-112` |
| U11 | `generate-manifest.py`는 사용되지 않는다. `release-manifest.json`의 주석이 틀렸다. (삭제된) `WORKFLOWS_GUIDE.md`는 5개라고 했지만 실제로는 3개다 | `.github/` |
| U12 | 모듈과 익스텐션 소스가 4갈래로 갈린다(main zipball / latest release / 모듈별 repo / raw main) | §2, §1.4 |
| U13 | `Program Files` 기본 설치에 권한 상승 없이 업데이트를 적용한다 | installer `lib.rs:1029-1033` |
| U14 | GitHub API를 인증 없이 호출해 rate limit(60/h)에 걸릴 위험이 있다 | `github.rs:101-140` |
