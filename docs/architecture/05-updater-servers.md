# 05. 업데이터 · 인스톨러 · 릴리즈 파이프라인 · server-chan (릴레이/챗봇/음악)

> 작성 기준: 코드만 근거로 삼았다(문서는 오래돼서 믿을 수 없다). 경로 표기 규칙은 다음과 같다.
> - `saba-chan/…` = saba-chan 저장소 루트
> - `server-chan(git)/…` = server-chan 저장소 루트 (HEAD `4c423e8 정리6`)
>
> 비밀값은 출력하지 않았다. 파일이 있다는 사실과 경로만 적는다.


> **2026-10-17 L3 이후**: 업데이트·모듈 목록·익스텐션 목록은 모두 릴레이의 서명된 릴리즈 manifest 하나에서 나온다. GitHub 업데이트 경로(`github.rs`, `api_base_url`, `github_owner`/`github_repo`, `include_prerelease`, 익스텐션 `manifest_url`, module.toml `[update]`, modules·extensions 저장소의 릴리즈 워크플로)는 없다. 근거 커밋: saba-chan 65e1763·8b840f7·60768a7, modules a26bdeb, extensions 301ff2c.

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
GET /poll (long-poll 25s, HMAC 서명)  ◄──── [saba-core 안의 relay agent (src/relay_agent.rs)]
POST /result/:id                     ────►   (Discord에 로그인하지 않음)
POST /heartbeat (60s)                         │ raw_command → crate::chat (같은 프로세스)
                                              ▼
                                       daemon 핸들러를 직접 호출 (IPC 경유 없음)
                     노드 토큰이 있으면 daemon 기동 때 자동 시작
                     (relay_agent.rs:155-160, ipc/mod.rs:787-789), /api/relay/agent/start·stop

[GUI/CLI] ──► daemon /api/relay/* (pairing·host/node 조회 proxy, relay.rs) ──► relay /api/pair/*, /api/hosts/*, /api/nodes/*
```

- 로컬 Discord bot(`discord_bot/`, local 모드)은 없어졌다. Discord bot은 server-chan 쪽 중앙 bot 하나뿐이다(`server-chan(git)/relay-server/src/bot/index.ts`).
- saba-chan 쪽 실행기는 daemon 안의 relay agent다. 받은 `raw_command`를 `crate::chat::api::run_with_daemon_settings`로 처리해 답한다(`relay_agent.rs:388-404`).
- 운영 쪽 부가 구성요소(music-server, chatbot-server)는 server-chan 비공개 문서에서 다룬다(§5).

---

## 1. Updater

### 1.1 크레이트 구성

| 크레이트 | 경로 | 버전 | 역할 |
|---|---|---|---|
| `saba-chan-updater-lib` | `saba-chan/updater/` (`Cargo.toml` v0.1.2) | 0.1.2 | 실제 로직(`UpdateManager`). daemon이 **in-process로 링크**하고 updater exe·installer도 쓴다 |
| `saba-chan-updater` (bin, Tauri) | `saba-chan/updater/gui/src-tauri/` | 0.1.2 (`tauri.conf.json:3`) | **apply 전용** GUI exe. `--apply` 없이 실행하면 종료한다 |
| 루트 workspace | `saba-chan/Cargo.toml:12` | — | members = `.`, `runner`, `updater`, `updater/gui/src-tauri`, `installer/gui/src-tauri`, `saba-chan-cli` |

버전의 원천은 저장소 루트 `versions.toml` 하나다. `scripts/versions.py sync`가 각 파일에 써 넣고 `check`가 CI에서 어긋남을 잡는다(`versions.toml:1-14`). updater-lib는 이 파일을 컴파일 때 그대로 묶는다(`updater/src/versions.rs:1-9`).

`updater/src/constants.rs`는 "모든 크레이트가 공유하는 SSOT"라고 스스로 설명한다(`constants.rs:1-4`). daemon도 `saba_chan_updater_lib::constants::*`로 data dir, 토큰 경로, IPC 포트(57474, `constants.rs:31-35`)를 가져다 쓴다. 즉 경로/상수 SSOT가 updater 라이브러리 안에 들어 있다.

~~**죽은 코드**: v2 모듈(worker/foreground/queue/scheduler/ipc)과 `cli.rs.removed`, `lib.rs.bak` 잔재~~ → 해결: 모듈과 잔재 파일은 지워졌다(e023a75, `lib.rs:15-31`의 `mod` 목록). ~~lib.rs 헤더 주석만 아직 "v2 아키텍처"를 설명한다~~ → 해결: 헤더는 지금 파일 구성을 적는다(`lib.rs:1-12`).

updater-lib 파일 구성(859c3fd에서 나눔): `lib.rs`는 컴포넌트 정의와 `UpdateManager`의 확인·다운로드·적용 흐름, `state.rs`는 적용 대기 목록(`pending.json`, `apply-targets.json`)과 설치 버전 기록, `apply.rs`는 폴더·바이너리·GUI 교체와 압축 해제, `time.rs`는 chrono 없이 만드는 ISO 8601 시각 문자열 도우미다.

### 1.2 컴포넌트 모델

`enum Component` (`updater/src/lib.rs:75-83`)와 manifest key의 대응(`lib.rs:87-120`):

| Component | manifest key | 적용 방식 | 적용 우선순위 (`lib.rs:863-876`) |
|---|---|---|---|
| `Updater` | `updater` | `apply_binary_update("saba-chan-updater")` | 0 |
| `Module(name)` | `module-{name}` | `modules_dir/{name}`을 zip 내용으로 교체. 백업은 `staging/{name}_backup` (`apply.rs:7-13`, `replace_dir` 24-62) | 1 |
| `Extension(name)` | `ext-{name}` | `extensions_dir`에 교체. 하이픈은 언더스코어로 변환(`lib.rs:411-427`, `apply.rs:16-22`). 백업은 `{name}_ext_backup` | 1 |
| `Locales` | `locales` | `install_root/locales`에 해제(`lib.rs:825-828`) | 1 |
| `CoreDaemon` | `saba-core` | `apply_binary_update("saba-core")` | 3 |
| `Gui` / `Cli` | `gui` / `cli` | 프로세스가 돌고 있으면 5, 아니면 4 | 4/5 |

- `DiscordBot` 컴포넌트는 로컬 bot과 함께 없어졌다. 우선순위 2는 비어 있다.
- 알 수 없는 key(예: `installer`)는 업데이트 대상에서 빠진다(`lib.rs:493-496`). `from_manifest_key`는 모르는 key를 `Module`로 폴백한다(`lib.rs:118-120`).
- 설치하지 않은 모듈·익스텐션은 업데이트 목록에 넣지 않는다. 새로 받는 것은 목록 화면의 몫이다(`lib.rs:499-502`).
- **로컬 버전 판정 순서**(`collect_local_versions`, `lib.rs:555-594`): 먼저 `installed-manifest.json`(`<data_dir>/installed-manifest.json`)을 본다. 앱 컴포넌트가 거기 없으면 빌드에 묶인 `versions.toml` 버전을 쓰고, 모듈은 `module.toml`, 익스텐션은 `manifest.json`의 id·version을 읽는다.
  - ~~`saba-core` 폴백이 updater-lib의 `CARGO_PKG_VERSION`이다~~ → 해결: 폴백은 `versions.toml`의 컴포넌트별 버전이다(`lib.rs:559-562`, `versions.rs`). daemon은 기동할 때 자기 버전을 installed-manifest에 적는다(`launch_ticket.rs:28-30` `record_core_version`).
  - "설치돼 있는데 버전을 모르면 최신으로 간주하고 installed-manifest에 기록"하는 로직이 있다(`lib.rs:503-507`). 거짓 양성을 막으려는 것이지만, 실제로 구버전이어도 영구히 최신으로 표시될 수 있다.

### 1.3 Release manifest 형식

릴레이가 주는 서명된 manifest 하나가 유일한 출처다(`updater/src/relay.rs:52-88`). 형식:

```jsonc
{
  "manifest": {
    "version": "0.1.1", "channel": "stable", "platform": "windows-x64",
    "released_at": "...", "is_critical": false,
    "components": {
      "saba-core":        { "version": "0.1.1", "asset": "saba-core-windows-x64.zip", "size_bytes": 0, "sha256": "…", "url": "<R2 공개 URL>", "install_dir": "." },
      "module-palworld":  { "version": "…", "asset": "module-palworld.zip", "sha256": "…", "url": "…", "install_dir": "modules/palworld", "meta": { "display_name": "…" } },
      "ext-steamcmd":     { "version": "…", "asset": "ext-steamcmd.zip", "sha256": "…", "url": "…", "meta": { "name": "…", "min_app_version": "…" } }
    }
  },
  "keyId": "saba-updater-v1", "signature": "<Ed25519>"
}
```

- 컴포넌트 키는 `saba-core`, `cli`, `gui`, `updater`, `locales`, `module-<name>`, `ext-<id>`다. 업데이터가 모르는 키(예: `installer`)는 건너뛴다.
- `requires`(다른 컴포넌트 최소 버전)와 `meta`(목록 화면용)는 선택이다.
- 서명 검증 공개키는 바이너리에 들어 있다(`signature.rs:21`). 검증을 통과하지 못하거나, 플랫폼이 다르거나, 현재보다 낮은 버전이면 거부한다(`relay.rs:233-251`).

### 1.4 어디서 가져오는가

| 용도 | 요청 | 근거 |
|---|---|---|
| 새 버전 확인 | `GET {relay_url}/v1/updates/check?channel&platform&current&client_id`. 아무것도 설치되지 않았으면(설치기, 목록) `current=0.0.0`을 보낸다 | `relay.rs:117-120, 163-179` |
| manifest | `latest.manifestUrl` 또는 `GET /v1/updates/manifest/:version` | `relay.rs:181-188` |
| 파일 | manifest 컴포넌트의 `url`(Cloudflare R2), `sha256` 확인 | `download.rs:40`, `lib.rs:643, 697` |
| 진행 이벤트 | `POST /v1/updates/event` (익명, 실패해도 무시) | `relay.rs:191-196` |
| 모듈·익스텐션 목록 | 같은 manifest의 `module-*`/`ext-*` (`relay::latest_catalog`) | `relay.rs:109-115` |

- 기본 릴레이 주소는 `relay::DEFAULT_RELAY_URL`, 채널은 `stable`이다(`relay.rs:21-23`). `relay_url`에 `scripts/mock-release-server.js` 주소를 넣으면 모의 릴레이로 시험할 수 있다(`lib.rs:246-248`). 다만 모의 릴레이 서명 키는 `mock-relay` 기능으로 빌드한 개발용 바이너리만 믿는다(`signature.rs:26-28`, `updater/Cargo.toml` `[features]`).
- 코어가 최신이어도 같은 릴리즈 manifest를 받아 컴포넌트별로 비교한다(GUI·CLI·모듈이 뒤처졌을 수 있으므로, `relay.rs:198-223`).

### 1.5 무결성 검증

- `UpdateManager::verify_integrity` (`lib.rs:454-458`)는 `integrity::verify_installed`를 부른다. 업데이터가 적용할 때 zip에서 푼 파일마다 SHA-256을 `<data_dir>/installed-files.json`에 적어 두고(`record_install`, `integrity.rs:100-117`), 검사는 그 기록과 지금 파일만 비교한다. 네트워크를 쓰지 않는다(`integrity.rs:1-6`).
- 결과 상태는 `Verified`, `Tampered`, `FileNotFound`, `Error` 중 하나다(`integrity.rs:41-51`). 설치기로 처음 깐 컴포넌트는 기록이 없어 결과에 나오지 않는다.
- ~~**해시 의미 불일치**: manifest의 zip 해시를 압축을 푼 파일과 비교해 항상 `Tampered`가 된다~~ → 해결: manifest 해시는 다운로드 검증에만 쓰고, 무결성 검사는 설치 때 기록한 파일 해시를 본다(`integrity.rs:1-6`).
- 다운로드는 해시를 확인한다: `download_verified`가 받은 파일의 sha256을 manifest 값과 대조하고(`lib.rs:643, 697`), 적용 직전에 staging 파일을 다시 대조한다(`lib.rs:843-853`).
- daemon API는 `GET /api/updates/integrity`이다(`src/ipc/updates.rs:107, 697`).

### 1.6 다운로드 → apply → swap → rollback 흐름

1. **확인**: `POST /api/updates/check`와 daemon의 주기 확인이 같은 `check_and_follow_up`을 거친다(`updates.rs:161, 209-244`) → `check_for_updates()` (`lib.rs:467-539`). 릴레이 manifest 하나로 모든 컴포넌트를 비교한다. Locales 업데이트가 있으면 사용자에게 알리지 않고 백그라운드에서 다운로드·적용한다(`updates.rs:216-222`, `silent_apply_locales` 279). `auto_download`이면 나머지 업데이트도 미리 받아 두고 pending manifest를 저장한다(`updates.rs:225-241`). 적용은 언제나 사용자가 고른다.
2. **다운로드**: `POST /api/updates/download` → staging 디렉터리로 받는다.
   - Windows: `%APPDATA%/saba-chan/updates/`
   - Unix: `~/.cache/saba-chan/updates/` (`constants.rs:148-167`)
   - 진행률은 `GET /api/updates/download/progress`로 본다(`updates.rs:422`).
3. **apply 분기**: `POST /api/updates/apply {components:[…]}` (`updates.rs:447-568`). 본문이 없거나 공백이면 전체 대상, 깨진 JSON이면 400 `invalid_body`다(`:320-329`). download도 같다
   - 대상에 `Gui`, `Updater`, `CoreDaemon` 중 하나라도 있으면 **전부 updater exe에 맡긴다**(`updates.rs:483-492`). 이때 daemon은 `staging/pending.json`(`state.rs:15-30`)과 `staging/apply-targets.json`(`state.rs:87-94`)을 쓰고 `{requires_updater:true, needs_updater:[…]}`를 응답한다.
   - 그렇지 않으면 daemon이 직접 `apply_single_component`를 실행한다. 그 뒤 모듈은 `supervisor.refresh_modules()`, 익스텐션은 `extension_manager.rescan()`으로 hot-reload한다(`updates.rs:531-559`).
4. **updater exe 실행**은 데몬이 한다: `POST /api/updates/launch-updater {relaunch}` → 데몬이 일회용 실행 표를 끊고(`launch_ticket.rs:1-5`) `saba-chan-updater --apply --wait-pid <데몬 PID> --launch-token <표> [--relaunch …]`를 분리 실행한 뒤 스스로 종료한다(`src/ipc/updates.rs:581-639`). GUI(`main.js:2458-2471`)와 CLI(PR #6 기준 `update apply`, `saba-chan-cli/src/cmd/update.rs:249`, `client.rs:620-624`)는 이 API만 부른다.
5. **updater exe** (`updater/gui/src-tauri/src/lib.rs`)
   - 인자는 `--apply [--wait-pid <pid>] --launch-token <t> [--relaunch <exe> [extra…]]`다(`lib.rs:388-403`). 표가 없거나 맞지 않으면 적용하지 않고 끝낸다. 표를 모르는 옛 데몬(0.1.2 미만)이 띄운 경우만 표 없이 받는다(`lib.rs:407-428`). `--wait-pid`가 있으면 그 PID가 끝나기를 최대 30초 기다린다(`lib.rs:430-447`). install_root는 자기 exe 위치로 정한다(`lib.rs:451-457`).
   - `load_pending_manifest()` → `apply-targets.json`의 key를 차례로 `apply_single_component()` → 성공·실패 모두 `UpdateCompletionMarker`(`<data_dir>/update-complete.json`, 실패 목록 `failed` 포함)를 저장 → `clear_pending_manifest()`가 `pending.json`/`apply-targets.json`과 받아 둔 압축 파일(`.zip`, `.tar.gz`, `.part`, 실패한 것 포함)을 지움(updater-lib `state.rs:63-82`) → 2초 뒤 `--after-update`를 붙여 relaunch(`lib.rs:73-198`, 312-350). 진행 상황은 Tauri 이벤트 `apply:progress`로 알린다. 다음 데몬이 결과를 `GET /api/updates/last-result`로 보여 준다(`updates.rs:682-694`).
   - relaunch는 Windows에서 `CREATE_BREAKAWAY_FROM_JOB`으로 띄우고, Job이 허용하지 않으면(os error 5) 그 플래그 없이 다시 띄운다(`lib.rs:326-342`). `--relaunch`가 없으면 설치 폴더의 GUI exe(`saba-chan-gui.exe`, 0.1.1 zip의 `Saba-chan.exe`)를 찾는다(`lib.rs:271-274`, updater-lib `lib.rs:272-278`). GUI를 적용하면서 exe 이름이 바뀌면(0.1.1 `Saba-chan.exe` → `saba-chan-gui.exe`) 예전 exe는 `.exe.old`로 비켜 두고(다음 정리 때 지운다) 바탕화면·시작 메뉴 바로가기를 새 exe로 돌린다(`apply.rs` `retire_other_gui_exes`). 0.1.1 업데이터는 `saba-chan-gui.exe`만 찾으므로 0.1.1에서 올릴 때는 GUI가 첫 적용에서 빠지고, 새 데몬으로 한 번 더 적용해야 GUI까지 바뀐다(0.1.1 코드라 고칠 수 없다).
   - Tauri 커맨드: `get_apply_mode`, `start_apply`, `get_preferred_language`, `get_theme`, `check_after_update`.
6. **바이너리 swap** (`apply_binary_update` `apply.rs:94-181`, gui 184-230)
   - 대상 프로세스가 끝나기를 최대 15초 기다린다. 시간이 지나도 "attempting update anyway"로 그냥 진행한다.
   - Windows는 실행 중인 `X.exe`를 `X.exe.old`로 rename하고(재시도 5회, `apply.rs:64-92`, 366-382) zip에서 새 파일을 쓴다. Linux는 알려진 바이너리만 `X.old`로 rename한다.
   - `cleanup_old_files()`는 install_root 바로 안의 사바쨩 실행 파일 `*.exe.old`만 지운다. 적용 직후에 한 번, 아직 잡혀 있어 못 지운 것은 다음 `UpdateManager` 생성 때 지운다(`lib.rs:395-396, 807-810, 886-902`).
7. **rollback**
   - 모듈·익스텐션은 staging에 `*_backup` 복사본을 만들고, 교체 도중 실패하면 그 백업으로 되돌린다. 성공하면 백업을 지운다(`replace_dir`, `apply.rs:24-62`).
   - 바이너리는 압축 해제 도중 실패하면 비켜 둔 `.old`를 원래 이름으로 되돌린다(`apply.rs:142-171`). 적용이 끝난 뒤에 이전 판으로 돌아가는 경로는 없다.
   - ~~`prepare_daemon_update`는 호출하는 곳이 없다(죽은 코드)~~ → 해결: 함수가 지워졌다(grep 0건).
8. **설치 버전 기록**: `update_installed_versions_batch` → `installed-manifest.json`(`state.rs:151-171`). 코어가 바뀌면 Windows "설치된 앱" `DisplayVersion`도 갱신하는데, 이 설치 폴더의 항목만 고친다(`state.rs:177-185`, `uninstall_entry.rs:76-85`).

### 1.7 daemon, GUI, CLI가 updater를 부르는 방식

**daemon HTTP API** (`saba-chan/src/ipc/updates.rs:98-110`, `ipc/mod.rs:756`에서 merge):

| Method | Path | 동작 |
|---|---|---|
| GET | `/api/updates/status` | 캐시된 상태. Locales는 숨긴다 |
| POST | `/api/updates/check` | 릴레이를 조회한다 |
| POST | `/api/updates/download` | body `{components:[]}`. 본문이 없으면 전부 받는다 |
| GET | `/api/updates/download/progress` | 진행률 |
| POST | `/api/updates/apply` | body `{components:[]}`(없으면 전부). 직접 적용하거나 updater 위임을 판단한다 |
| POST | `/api/updates/launch-updater` | body `{relaunch:[exe, …]}`. 데몬이 updater를 띄우고 스스로 종료한다(`updates.rs:581-639`) |
| GET / DELETE | `/api/updates/last-result` | updater가 남긴 마지막 적용 결과. DELETE는 확인 처리(`updates.rs:682-694`) |
| GET | `/api/updates/integrity` | 설치 때 기록한 파일 해시와 비교 |
| GET / **PUT** | `/api/updates/config` | 설정 `{enabled, check_interval_hours(1-8), auto_download, install_root, relay_url, channel}`(`auto_apply`는 f498525에서 빠졌다). `config.json`의 `updates` 절에 저장한다. GET은 `mock_relay`(모의 릴레이를 믿는 개발 빌드인지)도 준다(`updates.rs:727-806`). 업데이트 출처(`relay_url`)는 앱에서 바꾸지 않는다: 릴리즈 빌드의 PUT은 비우기만 받고 다른 값은 403 `relay_url_not_settable`(D7). 0.1.1이 남긴 예전 키 `relay_base_url`도 `relay_url`로 읽고, 빈 값은 기본 릴레이다(`updater/src/lib.rs` `UpdateConfig`) |

불일치:
- ~~GUI의 `updater:setConfig`가 POST로 보내 405가 난다~~ → 해결: PUT으로 보낸다(`saba-chan-gui/main.js:2505-2507`).
- ~~CLI의 `apply_updates`와 `download_updates`는 body 없이 POST하는데 `Json` extractor가 거부할 수 있다~~ → 해결: PR #6 CLI는 `{components:[...]}` 본문을 늘 보낸다(`saba-chan-cli/src/client.rs:609-619`). 데몬도 본문을 바이트로 받아 비었으면 기본값(전체)으로 읽고, 깨진 JSON만 400으로 거부한다(`src/ipc/updates.rs:320-343, 447-452`).
- ~~CLI가 daemon에 없는 `/api/install*`을 호출한다~~ → 해결: PR #6 CLI에 그 호출이 없다(grep 0건).
- ~~CLI `launch-apply`가 넘기는 `--install-root`와 대상 인자를 updater가 무시한다~~ → 해결: PR #6 CLI는 `update apply`에서 데몬이 updater가 필요하다고 답하면 확인 후 `POST /api/updates/launch-updater {relaunch:[saba-core, --spawned]}`만 부른다(`saba-chan-cli/src/cmd/update.rs:237-249`, `client.rs:620-624`).
- ~~자동 확인 주기를 GUI의 `setInterval`이 돌린다~~ → 해결: daemon이 `spawn_update_scheduler`로 기동 30초 뒤부터 `check_interval_hours`마다 확인한다(`updates.rs:247-274`, `src/main.rs:401`). GUI는 10분마다 캐시된 `GET /api/updates/status`만 읽어 알린다(`main.js:940-1028`). ~~`auto_apply`는 어디에서도 쓰이지 않는다~~ → 해결: 설정에서 지웠다. `auto_download`만 남았다(`updates.rs:207-208`).

---

## 2. Installer (`saba-chan/installer/gui`, Tauri, v0.1.2)

모듈 구성: `lib.rs`, `release.rs`(릴리즈 받기, 업데이터와 같은 relay 경로), `runtime_bootstrap.rs`, `registry.rs`, `shortcuts.rs`, `uninstall.rs`.

**설치 위치**
- 기본 경로는 Windows `C:\Program Files\Saba-chan`, Unix `~/.local/share/saba-chan`(`installer/gui/src-tauri/src/lib.rs:752-769`).
- 쓰기에 실패하면 UAC로 권한을 올려 디렉터리를 만든다(`elevate_create_dir`, `lib.rs:607-700`).
- 데이터는 `%APPDATA%/saba-chan`에 둔다(updater constants와 같다).
- 로그는 `%TEMP%/saba-chan-installer.log`에 남는다(`lib.rs:954-969`).

**`do_install` 단계** (`lib.rs:245-550`)

| 단계 | 내용 |
|---|---|
| 1 (0-5%) | install dir 생성, 쓰기 probe, 필요하면 권한 상승 |
| 2 (5-10%) | 릴레이에서 서명된 최신 manifest를 받아 검증한다(`release.rs:16-28` `fetch_latest`) |
| 2.5 | manifest의 `installer.version`이 현재 `CARGO_PKG_VERSION`보다 높으면 설치를 차단한다(`lib.rs:330-345`) |
| 3 (10-45%) | 업데이터가 아는 키 중 모듈·익스텐션을 뺀 컴포넌트(`installer` 제외)를 R2에서 받아 SHA-256을 확인하고 `install_dir/<install_dir>`에 푼다(`release.rs:35-45`, `lib.rs:347-401`). 하나라도 실패하면 설치를 멈춘다 |
| 3.5 | 공유 모듈(`i18n.py`, `daemon_rcon.py`)을 `%APPDATA%/saba-chan/modules/`로 복사(`lib.rs:810-838`) |
| 4 (45-55%) | 선택한 모듈을 manifest의 `module-<id>`에서, `requires_extensions`에 따른 익스텐션을 `ext-<id>`에서 R2로 받아 SHA-256을 확인하고 설치한다(`lib.rs:407-450`, 915-948). 실패는 경고로 남긴다 |
| 5 (55-70%) | portable Python(python-build-standalone, 판과 SHA-256은 `updater/src/python_pin.rs`에 고정)과 venv (`runtime_bootstrap.rs:42-51`). 실패해도 치명적으로 처리하지 않는다 |
| 6 | 없음: 로컬 Discord bot이 없어져 portable Node·npm install 단계도 지워졌다 |
| 7-8 | `setup_config`, 언어 설정을 `config.json`의 `gui.language`에 저장(`lib.rs:840-865`) |
| 9 | `HKCU\Software\Microsoft\Windows\CurrentVersion\Uninstall\Saba-chan*` 등록. 키는 설치 폴더마다 따로다: 첫 설치본은 `Saba-chan`, 다른 폴더는 `Saba-chan-<폴더 해시 8자>`(`registry.rs:15-77`, `updater/src/uninstall_entry.rs:12-31`) |
| 10 | 바탕화면·시작 메뉴 바로가기. 대상은 설치 폴더의 GUI exe(`saba-chan-gui.exe` 또는 0.1.1의 `Saba-chan.exe`)다(`lib.rs:486-515`) |
| 10.5 | `installed-manifest.json` 생성. updater 버전 판정의 1순위 소스다(`lib.rs:517-521`) |

레지스트리·바로가기처럼 실패해도 계속하는 단계는 완료 화면에 실패 목록과 로그 경로로 보여 준다(`lib.rs:277-284, 523-546`).

Tauri 커맨드(`lib.rs:108-730`): `get_installer_state`, `set_install_path`, `set_language`, `set_shortcut_options`, `set_selected_modules`, `get_available_modules`(같은 manifest의 `module-*`, 폴백 없음), `fetch_latest_release`, `start_install`, `start_uninstall`, `get_app_mode`, `launch_app`, `get_preferred_language`, `browse_folder`. GUI는 `--uninstall` 인자로 installer를 uninstall 모드로 실행한다(`saba-chan-gui/main.js:2653`).

**제거** (`uninstall.rs`): 대상은 제거기 자신이 든 설치 폴더 하나다(`uninstall.rs:251-256`). 그 폴더에서 실행 중인 사바쨩 프로세스만 끝내고(`uninstall.rs` `stop_saba_processes`), 포터블 GUI 실행기가 임시 폴더에 풀어 띄운 실제 GUI(실행기의 자손, 제거기 자신은 뺀다)도 끝낸 뒤(`descendants_in`), 그 폴더의 레지스트리 항목만 지운다(`registry.rs:78-93`). `%APPDATA%/saba-chan`과 모듈 폴더는 설치본들이 함께 쓰므로 다른 설치본 항목이 남아 있으면 지우지 않는다(`uninstall.rs:79-84, 117-142`, `uninstall_entry.rs:42-48`). `keep_settings`이면 `config.json`·`instances`·이관 전 설정 파일만 남긴다(`uninstall.rs:407-449`).

**문제점**
- 기본 설치 경로가 `Program Files`(관리자 권한 필요)인데 레지스트리는 HKCU에 쓴다. 더 큰 문제는 updater exe와 daemon이 **권한 상승 없이** `install_root`에 쓴다는 점이다. 기본 경로에 설치하면 업데이트 적용이 권한 문제로 실패할 수 있다(추정, 매니페스트의 requestedExecutionLevel은 확인하지 않음).
- 모듈·익스텐션 소스는 하나다: 설치기, 데몬 목록, 업데이터가 모두 같은 릴리즈 manifest를 본다.

---

## 3. CI / 릴리즈 파이프라인

### 3.1 saba-chan (`.github/workflows`)

| 파일 | 트리거 | 내용 |
|---|---|---|
| `ci.yml` | push/PR to main, develop, tag `v*` | 하나뿐인 검사 워크플로(옛 `test.yml`은 중복이라 b0b5631에서 지웠다). Rust matrix(os): `versions.py check` → fixture 모듈 생성 → `cargo clippy --workspace -D warnings` → build → test. JS(Node 22): GUI `npm ci` → vitest → `vite build`, 그리고 main/preload/updater/installer 스크립트 `node --check` |
| `release.yml` | `release: [published, edited]`, `workflow_dispatch(tag_name)` | **빌드는 하지 않는다.** 사람이 Draft Release에 올린 raw exe/zip을 `gh release download` → `package-release.py`로 표준 zip 재패키징 → `locales.zip`. 그다음 R2 단계: 태그가 `versions.toml`의 release 버전과 다르면 실패 → 릴레이와 R2에 같은 버전이 있으면(확인 실패 포함) 실패 → modules·extensions 저장소 `main`을 checkout → `scripts/versions.py manifest`가 모듈·익스텐션 zip까지 묶어 manifest 생성 → `release_signing` 예제로 `saba-updater-v1` 키 서명(번들 공개키로 검증까지) → R2 `releases/<version>/` 업로드 → 릴레이 `POST /admin/releases` 등록(본문은 한글을 `\uXXXX`로 이스케이프한 ASCII JSON이고 `--data-binary`로 보낸다, `scripts/versions.py:351-354`, `release.yml:226-229`). 마지막에 `scripts/changelog.py`로 만든 릴리즈 노트를 붙인다(`release.yml:1-48, 120-233, 272-282`) |

- GitHub Release는 사람이 raw 파일을 올리는 **입력 칸**으로만 쓴다. 업데이터·설치기는 GitHub를 보지 않는다.
- 실제 빌드는 로컬 Windows의 `scripts/build-windows.ps1`이 한다. 버전은 `versions.toml`에서 읽는다(`build-windows.ps1:27-28`).
- `package-release.py`는 **windows-x64 asset만** 만든다. manifest도 `--platform windows-x64`로 만든다.
- `package-release.py`는 zip을 표준 형태로 다시 담는다: 경로 구분자를 `/`로 바꾸고, GUI zip 루트의 앱 exe(Electron의 `Saba-chan.exe`)를 `saba-chan-gui.exe`로 바꾸며, 다른 컴포넌트의 exe는 뺀다(`package-release.py:146-183`). 다만 컴포넌트의 `bundles`에 적은 exe는 남기므로 raw saba-core zip에 든 `saba-runner.exe`가 재포장에서 빠지지 않는다(`:58-59, 165-168`). saba-core zip에는 `saba-runner.exe`와 공유 모듈(`i18n.py`, `daemon_rcon.py`)이 같이 들어간다(`package-release.py:48-60`).

### 3.2 modules / extensions repo

- 두 저장소 모두 자체 릴리즈 워크플로가 없다. `saba-chan-modules`에는 검사용 `check-modules.yml`, `saba-chan-extensions`에는 pytest·ruff 검사용 `ci.yml`만 있다.
- 둘 다 코어 `release.yml`이 `main`을 checkout해서 같은 릴리즈 manifest에 싣는다(§3.1, [02-modules.md](02-modules.md) §5, [03-extensions.md](03-extensions.md) §3.4).

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
1. GUI `relay:initiatePairing` → daemon `POST /api/relay/pair/initiate` (`src/ipc/handlers/relay.rs:288-309`) → relay `POST /api/pair/initiate`. 응답은 `{code, pollSecret}`이다.
2. 사용자가 Discord에서 `/사바쨩 연결 코드:<code>`를 실행한다 → relay 내장 bot `handlePair`가 `SELF_URL/api/pair/:code/claim`을 x-internal-secret으로 루프백 호출한다(`bot/index.ts:479-529`). users upsert, hosts 생성 또는 토큰 재발급, guild가 있으면 nodes 자동 생성, `pairing_codes.node_token`에 평문 저장.
3. GUI가 daemon `GET /api/relay/pair/:code/status?secret=…`를 폴링한다(`relay.rs:321-358`). relay가 `claimed`와 함께 nodeToken을 1회 돌려주면 **daemon이 `%APPDATA%/saba-chan/.node_token`에 자동 저장**한다(`relay.rs:341-355`).
4. daemon `GET/PUT/DELETE /api/config/node-token`(`ipc/mod.rs:705`, `handlers/config.rs:135-`)은 토큰 원문을 IPC로 그대로 반환한다.

**(b) 명령 실행** (cloud 모드)
1. Discord 사용자가 `<prefix> <text>`를 입력한다. prefix는 길드별 캐시이고 기본값은 `BOT_PREFIX` env 또는 `'사바쨩'`이다(`bot/index.ts:230,238-281`).
2. `forwardCommand`(`bot/index.ts:730-800`)가 requestId를 먼저 만들고 `resultWaiters.wait(id, 30s)`를 등록한 뒤 `POST /api/command`를 부른다. payload는 `{action:'raw_command', text}`다. `guildId`로 nodes를 조회해 hostId를 찾고, 큐에 INSERT한 뒤 `pollWaiters.wake(hostId)`를 호출한다.
3. daemon의 relay agent(`src/relay_agent.rs:326-386`)가 `GET /poll`을 계속 돌리고 있다. 서버는 즉시 pending 최대 10건을 조회해 `delivered`로 표시하고 반환한다. 없으면 **25초 대기**(`poll.ts:6`, `pollWaiters.wait`) 후 다시 조회하고, 그래도 없으면 204를 준다. 클라이언트 연결이 끊기면 abort한다. host가 paused 상태면 `{sleeping:true}`를 준다.
4. agent는 받은 `raw_command`마다 `crate::chat::api::run_with_daemon_settings`를 같은 프로세스에서 실행한다(`relay_agent.rs:388-404`). 접두사·별명·길드별 설정은 daemon의 bot config에서 읽는다(`chat/api.rs:117-122`). 답 텍스트를 `{success, data:{text}}`로 담아 `POST /result/:id`로 보낸다. 입력을 알아보지 못했으면(인스턴스를 못 찾음, 정의에 없는 명령, 모르는 낱말) `{success:false, data:{code:"INSTANCE_NOT_FOUND"|"UNKNOWN_COMMAND", error:<현지화 문구>, args:{name?, command?, help?}}}`로 보내고(정의에 없는 명령이면 `help`에 그 모듈의 명령 안내를 담아 릴레이가 덧붙인다), 릴레이는 이 코드를 보고 모델 해석기에 정확한 명령을 한 번 묻는다(`relay_agent.rs` `raw_command_result`, `chat::Reply`). 모듈 이름만 쓰면 실패가 아니라 명령 안내다. 모르는 action은 `success:false`다.
5. relay는 상태를 completed로 바꾸고 `resultWaiters.wake(id)`를 호출한다. bot은 `result.data.text`(최대 1950자)로 Discord에 reply한다.
6. agent의 재시도 백오프는 3초에서 시작해 최대 60초다. sleeping 상태면 30초마다 확인한다(`relay_agent.rs:22-26, 417-420`).

**(c) Heartbeat**: 60초마다 `POST /heartbeat`(`relay_agent.rs:211-260`). metadata = `{servers, modules, moduleDetails:{}, botConfig:{prefix, moduleAliases, commandAliases}, allowedInstances?:{<guildId>:[instance id]}}`이고(`allowedInstances`는 `bot.node_settings[guild].allowedInstances` 목록이 있는 길드만, 없으면 제한 없음), 서버 목록에서 `rcon_password`/`rest_password`는 뺀다(`relay_agent.rs:262-324`). 응답의 `sleeping`으로 폴링을 멈추거나 다시 시작한다. 시작할 때 `GET /info`로 `minAgentVersion`을 비교한다(`relay_agent.rs:191-`).

**(d) Sleep/Wake**: 슬래시 `/사바쨩 코코낸내`/`기상`이 메모리의 `_pausedHosts`/`_pausedGuilds` Set을 토글한다(`bot/index.ts:76-81,647-720`).

### 4.5 daemon 쪽 호출과 relay 서버 비교 (검증 결과)

| daemon 라우트 (`ipc/mod.rs:740-749`) | relay 호출 | 서명 | 일치 |
|---|---|---|---|
| GET `/api/relay/host/:id/status` | GET `/api/hosts/:id` | HMAC(Bearer, ts, nonce, sig) | ✓ (`relay.rs:188-219`) |
| GET `/api/relay/host/:id/nodes` | GET `/api/hosts/:id/nodes` | HMAC | ✓ |
| GET `/api/relay/node/:guild/members` | GET `/api/nodes/:g/discord-members`, 503이면 `/api/nodes/:g/members` | HMAC | ✓ (discord-members는 rate limit 5/min) |
| POST `/api/relay/pair/initiate` | POST `/api/pair/initiate` | 없음 | ✓ |
| GET `/api/relay/pair/:code/status` | GET `/api/pair/:code/status?secret=` | 없음 | ✓. 하위호환 query 방식이라 pollSecret이 nginx access log에 남는다. 헤더 방식으로 바꾸는 편이 낫다 |
| GET `/api/relay/bot-invite` | GET `/api/bot-invite` | 없음 | ✗ git HEAD의 relay에는 이 라우트가 없다(`relay.rs:265-284`) |
| GET `/api/relay/agent`, POST `.../agent/start`·`stop` | — (daemon 안의 relay agent 상태·제어) | — | — |

- 서명 payload 형식(`METHOD\npath\nts\nnonce\nbody`)은 Rust(`relay.rs:102-137`, relay agent도 같은 `signed_headers`를 쓴다, `relay_agent.rs:179-189`)와 서버(`auth.ts:139-148`)가 같다. 서버는 `req.url`(query 포함)로 서명을 확인한다. daemon은 query 없이 서명하는데, 실제로 서명하는 요청에 query를 붙이지 않으므로 현재는 문제가 없다.
- ~~daemon proxy의 `relayUrl` query/body override는 **임의의 URL로 Bearer node token과 HMAC 헤더를 보낼 수 있게 한다**~~ → 해결: 서명하는 프록시(host status·nodes·members)는 `signed_base`를 거쳐, 요청한 주소가 설정된 릴레이와 같거나 루프백(모의 릴레이)일 때만 따르고 아니면 설정된 릴레이로 보낸다(`relay.rs:45-67, 216, 228, 241`). ~~서명하지 않는 pairing·bot-invite는 `relayUrl`을 그대로 따라, 다른 릴레이가 준 노드 토큰을 저장할 수 있다~~ → 해결: 페어링 시작·상태와 bot-invite도 `signed_base`를 거친다(`relay.rs:271, 299, 327`, 59a93b8).
- `/poll`, `/heartbeat`, `/result`는 daemon 안의 relay agent가 부른다. 릴레이 주소는 bot config의 `cloud.relayUrl`, 토큰은 `<data_dir>/.node_token`에서 읽는다(`relay_agent.rs:72-80`).
- ~~relay의 `PATCH /api/hosts/:id/bot-config`를 GUI가 직접 호출해 항상 실패하고, 길드별 prefix가 언제나 기본값이다~~ → 해결: GUI는 bot config를 daemon에만 저장하고(`main.js:2857-2867`), agent heartbeat가 `botConfig`를 보낸다(`relay_agent.rs:313-321`). relay는 heartbeat metadata를 `hosts.metadata`에 저장하고 bot이 거기서 prefix를 읽는다(`heartbeat.ts:51-54`, `bot/index.ts:237-271`).
- `result.ts:54-80`의 Discord interaction followup은 `interactionToken`이 있을 때만 동작한다. 그런데 `forwardCommand`가 이 값을 넣지 않으므로 죽은 경로다. 또 `data.message/status`를 읽는 반면 agent는 `data.text`를 보내서 필드도 맞지 않는다(`relay_agent.rs:399`).
- `node_permissions`(`/사바쨩 권한부여/권한해제`, `bot/index.ts:531-645`)는 relay에서 **목록 조회에만 쓰인다**. 권한을 실제로 강제하는 곳은 daemon 채팅 처리기의 길드별 설정(bot config `nodeSettings`의 `allowedInstances`, `memberPermissions`)뿐이고, 설정이 없으면 제한이 없다(`src/chat/mod.rs:149-178, 345-348`). relay 쪽에서 부여한 권한은 실행 권한에 아무 영향이 없다.

---

## 5. server-chan 운영 구성요소와 운영 결함

> 이 절(공유 폴더에만 있는 chatbot/music-server, 운영 배포 토폴로지)과 §6.3–§6.5(relay 결함, 비밀 파일, 누락 소스)는 운영 서버 보안과 직결돼 공개 저장소에 두지 않는다. 비공개 저장소 `WareAoba/server-chan`의 `docs/architecture-server.md`에서 관리한다.

---

## 6. 불일치 · 누락 소스 · 스텁 · 보안 문제

### 6.1 소유자 프레이밍("daemon이 주인, 인터페이스는 무력") 위반

1. ~~**GUI가 업데이트 주기를 직접 돌린다**~~ → 해결: daemon이 주기 확인을 한다(`src/ipc/updates.rs:247-274`). GUI는 캐시된 상태만 10분마다 읽는다(`saba-chan-gui/main.js:940-1028`).
2. ~~**GUI와 CLI가 updater exe를 직접 spawn한다**~~ → 해결: 둘 다 `POST /api/updates/launch-updater`만 부르고 updater는 daemon이 띄운다(`updates.rs:581-639`, `main.js:2458-2471`, `saba-chan-cli/src/cmd/update.rs:249`).
3. ~~**GUI가 relay를 직접 호출한다**(`PATCH /api/hosts/:id/bot-config`)~~ → 해결: GUI는 bot config를 daemon에 저장하고, relay 동기화는 agent heartbeat가 맡는다(`main.js:2857-2867`, `relay_agent.rs:313-321`).
4. ~~**GUI가 Discord bot 실행 명령을 조립한다**, 실행 경로가 두 개~~ → 해결: 로컬 Discord bot이 없어졌다. relay agent는 daemon 안에서 노드 토큰이 있으면 기동 때 자동으로 시작한다(`relay_agent.rs:155-160`, `ipc/mod.rs:787-789`).
5. GUI가 daemon을 `--spawned`로 spawn하고(`main.js:608`) installer를 `--uninstall`로 실행한다(`main.js:2653`). 인터페이스가 프로세스 감독 역할을 맡고 있다.
6. ~~**extensions 용어 충돌**(`discord_bot/extensions/*.js`, `music.js`가 Python 익스텐션을 직접 spawn)~~ → 해결: `discord_bot/`이 저장소에서 없어졌다.
7. **server-chan과 프레이밍의 관계**
   - relay는 프레이밍에 맞는다. Discord를 향하는 bot은 server-chan 쪽 하나뿐이고, saba-chan은 daemon 안의 relay agent로 실행기 역할만 한다. "server-chan이 bot에 추가 기능을 준다"기보다는 **server-chan이 bot 그 자체**다.
   - music-server는 relay bot의 부가 기능이지만 소스가 없다.
   - chatbot-server는 daemon과도 relay와도 연결이 없는 **별개의 bot**이라 프레이밍을 벗어난다.
   - 업데이트도 relay가 SSOT다(§1.3-1.4). server-chan은 bot 지원을 넘어 배포 인프라(서명된 릴리즈 manifest 제공)까지 맡는다.

### 6.2 updater, installer, CI 불일치와 버그

| # | 내용 | 근거 |
|---|---|---|
| U1 | ~~manifest `sha256`은 zip 해시이고 integrity는 압축을 푼 파일 해시라서 항상 `Tampered`가 된다~~ → 해결: 적용 때 푼 파일 해시를 `installed-files.json`에 기록하고 그 기록과 비교한다 | `integrity.rs:1-6, 100-117` |
| U2 | ~~다운로드와 설치 때 sha256을 검증하지 않는다~~ 해결됨: 업데이터·설치기·데몬 모듈 설치 모두 `download_verified`로 확인한다 | `download.rs:40` |
| U3 | ~~**Zip-slip**: `entry.name()`을 그대로 `join`한다~~ → 해결: 모든 zip 해제가 `enclosed_name()`으로 검사하는 `extract_zip` 하나를 거치고, tar.gz는 `..`·절대 경로 엔트리를 건너뛴다 | updater `lib.rs:295-328`, `apply.rs:423-434`; installer `lib.rs:805-808` |
| U4 | ~~`saba-core` 버전 폴백이 updater-lib 버전이다~~ → 해결: 폴백은 빌드에 묶인 `versions.toml` 버전이다 | `lib.rs:559-562`, `versions.rs` |
| U5 | ~~`prepare_daemon_update`는 사용되지 않는다. rollback 경로가 없다~~ → 해결: 함수는 지워졌고, 적용 도중 실패하면 백업·`.old`로 되돌린다. 적용이 끝난 뒤 이전 판으로 돌아가는 경로는 여전히 없다 | `apply.rs:24-62, 142-171` |
| U6 | ~~v2 모듈(worker/foreground/queue/scheduler/ipc)이 죽은 코드다. `cli.rs.removed`, `lib.rs.bak` 잔재~~ → 해결: 지워졌다(e023a75). ~~헤더 주석만 남았다~~ → 해결: 헤더도 현재 파일 구성으로 바뀌었다 | `lib.rs:1-12` |
| U7 | ~~설정이 저장되지 않는다, GUI가 POST를 쓴다~~ → 해결: `config.json` `updates` 절, GUI도 PUT | `updates.rs:727-806`, `main.js:2505-2507` |
| U8 | ~~CLI의 `/api/install/*`은 daemon에 없다~~(호출이 지워졌다). ~~CLI apply·download는 여전히 body 없는 POST다~~ → 해결: PR #6 CLI는 본문을 보내고, 데몬도 본문 없는 apply·download를 전체 대상으로 받는다 | `saba-chan-cli/src/client.rs:609-619`, `src/ipc/updates.rs:320-343` |
| U9 | ~~CLI `launch-apply`가 넘기는 `--install-root`와 대상 인자를 updater가 무시한다~~ → 해결: PR #6 CLI `update apply`는 daemon의 `launch-updater`만 부른다 | `saba-chan-cli/src/cmd/update.rs:237-249` |
| U10 | CI가 windows-x64 릴리즈만 만든다 | `package-release.py`, `release.yml` |
| U11 | ~~`generate-manifest.py`, `release-manifest.json` 템플릿~~ 지워졌다 | `.github/` |
| U12 | ~~모듈과 익스텐션 소스가 4갈래로 갈린다~~ 해결됨: 모두 같은 릴리즈 manifest | §1.4 |
| U13 | `Program Files` 기본 설치에 권한 상승 없이 업데이트를 적용한다 | installer `lib.rs:752-756` |
| U14 | ~~GitHub API 비인증 rate limit~~ 해당 없음: GitHub를 조회하지 않는다 | §1.4 |
