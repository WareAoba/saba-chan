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
GET /poll (long-poll 25s, 노드 토큰)  ◄──── [saba-core 안의 relay agent (src/relay_agent.rs)]
POST /result/:id                     ────►   (Discord에 로그인하지 않음)
POST /heartbeat (60s)                         │ raw_command → crate::chat (같은 프로세스)
POST /events (알림, I3)
                                              ▼
                                       daemon 핸들러를 직접 호출 (IPC 경유 없음)
                     노드 토큰이 있으면 daemon 기동 때 자동 시작
                     (relay_agent.rs:178-189, ipc/mod.rs:860-866), /api/relay/agent/start·stop

[GUI/CLI] ──► daemon /api/relay/* (pairing·host/node 조회 proxy, relay.rs) ──► relay /api/pair/*, /api/hosts/*, /api/nodes/*
```

- 로컬 Discord bot(`discord_bot/`, local 모드)은 없어졌다. Discord bot은 server-chan 쪽 중앙 bot 하나뿐이다(`server-chan/relay-server/src/bot/`).
- saba-chan 쪽 실행기는 daemon 안의 relay agent다. 받은 `raw_command`를 `crate::chat::api::run_with_daemon_settings`로 처리해 답한다(`relay_agent.rs:509-530`).
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

| Component | manifest key | 적용 방식 | 적용 우선순위 (`lib.rs:896-909`) |
|---|---|---|---|
| `Updater` | `updater` | `apply_binary_update("saba-chan-updater")` | 0 |
| `Module(name)` | `module-{name}` | `modules_dir/{name}`을 zip 내용으로 교체. 백업은 `staging/{name}_backup` (`apply.rs:7-13`, `replace_dir` 24-62) | 1 |
| `Extension(name)` | `ext-{name}` | `extensions_dir`에 교체. 하이픈은 언더스코어로 변환(`lib.rs:426-442`, `apply.rs:16-22`). 백업은 `{name}_ext_backup` | 1 |
| `Locales` | `locales` | `install_root/locales`에 해제(`lib.rs:855-858`) | 1 |
| `Installer` | `installer` | 설치 폴더의 `saba-chan-installer.exe`를 받은 exe로 교체(`apply.rs` `apply_installer_update`) | 1 |
| `CoreDaemon` | `saba-core` | `apply_binary_update("saba-core")` | 3 |
| `Gui` / `Cli` | `gui` / `cli` | 프로세스가 돌고 있으면 5, 아니면 4 | 4/5 |

- `DiscordBot` 컴포넌트는 로컬 bot과 함께 없어졌다. 우선순위 2는 비어 있다.
- 알 수 없는 key는 업데이트 대상에서 빠진다(`lib.rs` `try_from_manifest_key`). `installer`는 `Component::Installer`로, 설치 폴더의 `saba-chan-installer.exe`(제거기)를 받은 exe로 바꾸고 복사가 실패하면 원래 사본을 되돌린다(`apply.rs` `apply_installer_update`). `from_manifest_key`는 모르는 key를 `Module`로 폴백한다(`lib.rs:118-120`).
- 설치하지 않은 모듈·익스텐션은 업데이트 목록에 넣지 않는다. 새로 받는 것은 목록 화면의 몫이다(`lib.rs:515-518`).
- **로컬 버전 판정 순서**(`collect_local_versions`, `lib.rs:585-624`): 먼저 `installed-manifest.json`(`<data_dir>/installed-manifest.json`)을 본다. 앱 컴포넌트가 거기 없으면 빌드에 묶인 `versions.toml` 버전을 쓰고(설치기는 빼고. 0.1.7까지 기록하지 않아 깔려 있는데 기록이 없으면 옛 사본으로 보고 바꾼다, `fill_bundled_versions`·`unknown_version_needs_update`), 모듈은 `module.toml`, 익스텐션은 `manifest.json`의 id·version을 읽는다.
  - ~~`saba-core` 폴백이 updater-lib의 `CARGO_PKG_VERSION`이다~~ → 해결: 폴백은 `versions.toml`의 컴포넌트별 버전이다(`lib.rs:589-592`, `versions.rs`). daemon은 기동할 때 자기 버전을 installed-manifest에 적는다(`launch_ticket.rs:28-30` `record_core_version`).
  - "설치돼 있는데 버전을 모르면 최신으로 간주하고 installed-manifest에 기록"하는 로직이 있다(`lib.rs:519-523`). 거짓 양성을 막으려는 것이지만, 실제로 구버전이어도 영구히 최신으로 표시될 수 있다.

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

- 컴포넌트 키는 `saba-core`, `cli`, `gui`, `updater`, `installer`, `locales`, `module-<name>`, `ext-<id>`다. 업데이터가 모르는 키는 건너뛴다.
- `installer`는 0.1.5부터 zip이 아닌 `saba-chan-installer.exe` 그대로다(`versions.toml`, `.github/scripts/package-release.py`). 소개 페이지 다운로드(relay `/v1/updates/download/installer`)가 최신 stable manifest의 이 `url`로 보낸다. 설치기는 이 항목의 `version`만 읽고(2.5단계) 자기 자신을 설치 폴더에 복사하며 그 버전을 installed-manifest의 `installer`로 적는다. 업데이터는 이 기록이 manifest보다 낮으면 사본을 바꾼다. 0.1.4 이전 릴리스는 zip 그대로 둔다.
- `requires`(다른 컴포넌트 최소 버전)와 `meta`(목록 화면용)는 선택이다.
- 서명 검증 공개키는 바이너리에 들어 있다(`signature.rs:21`). 검증을 통과하지 못하거나, 플랫폼이나 채널이 요청과 다르거나, 현재보다 낮은 버전이면 거부한다(`relay.rs` `verify_release`). 서명이 맞아도 컴포넌트 키, `asset` 파일 이름, `install_dir`의 각 칸이 영문·숫자·`.`·`_`·`-`만으로 되어 있지 않거나 `.`으로 시작하면(`..`, 절대 경로, 드라이브 `:`) manifest 전체를 거부한다(`check_manifest_paths`). `module-`·`ext-` 키는 접두사를 뗀 이름도 같은 규칙을 지켜야 한다(`module-..`, `ext-` 거부). 이 값들은 그대로 모듈·익스텐션 폴더, 받을 파일, 설치 폴더 경로에 붙는다. 단 이미 최신이라 버전만으로 manifest를 받은 경우에는 채널이 달라도(같은 버전이 stable·beta 둘 다에 있으면 릴레이는 늦게 등록한 행을 준다) 그 버전이 지금 깔린 버전과 같으면 받아서 컴포넌트를 비교한다. check 응답의 `latest`는 서명 밖이라, 그 `version`이 서명된 manifest의 `version`과 다르면 거부한다(`bind_release`). 그래서 적용 결과에 싣는 릴리즈 버전은 서명된 값이다. 패치노트(`notes`)는 릴레이에 등록하는 값이라 서명 밖에 있다.

### 1.4 어디서 가져오는가

| 용도 | 요청 | 근거 |
|---|---|---|
| 새 버전 확인 | `GET {relay_url}/v1/updates/check?channel&platform&current&client_id`. 아무것도 설치되지 않았으면(설치기, 목록) `current=0.0.0`을 보낸다 | `relay.rs:117-120, 163-179` |
| manifest | `latest.manifestUrl` 또는 `GET /v1/updates/manifest/:version` | `relay.rs:181-188` |
| 파일 | manifest 컴포넌트의 `url`(Cloudflare R2), `sha256` 확인 | `download.rs:40`, `lib.rs:673, 727` |
| 진행 이벤트 | `POST /v1/updates/event` (익명, 실패해도 무시) | `relay.rs:191-196` |
| 모듈·익스텐션 목록 | 같은 manifest의 `module-*`/`ext-*` (`relay::latest_catalog`) | `relay.rs:109-115` |

- 기본 릴레이 주소는 `relay::DEFAULT_RELAY_URL`, 채널은 `stable`이다(`relay.rs:21-23`). `relay_url`에 `scripts/mock-release-server.js` 주소를 넣으면 모의 릴레이로 시험할 수 있다(`lib.rs:246-249`). 다만 모의 릴레이 서명 키는 `mock-relay` 기능으로 빌드한 개발용 바이너리만 믿는다(`signature.rs:26-28`, `updater/Cargo.toml` `[features]`).
- 코어가 최신이어도 같은 릴리즈 manifest를 받아 컴포넌트별로 비교한다(GUI·CLI·모듈이 뒤처졌을 수 있으므로, `relay.rs:198-223`).

### 1.5 무결성 검증

- `UpdateManager::verify_integrity` (`lib.rs:469-473`)는 `integrity::verify_installed`를 부른다. 업데이터가 적용할 때 zip에서 푼 파일마다 SHA-256을 `<data_dir>/installed-files.json`에 적어 두고(`record_install`, `integrity.rs:104-121`), 검사는 그 기록과 지금 파일만 비교한다. 네트워크를 쓰지 않는다(`integrity.rs:1-6`).
- 결과 상태는 `Verified`, `Tampered`, `FileNotFound`, `Error` 중 하나다(`integrity.rs:45-55`). 설치기로 처음 깐 코어 컴포넌트(모듈·익스텐션 밖)는 기록이 없어 결과에 나오지 않는다. 0.1.6까지의 설치기가 기록 없이 둔 모듈·익스텐션은 업데이트 확인이 같은 버전이라도(로컬이 더 새 버전이면 빼고) 업데이트 대상으로 올려(`UpdateManager::reinstall_from_catalog`), 적용하면 기록과 공식 표시가 생긴다.
- `installed-files.json`, `official-components.json`, `installed-manifest.json`은 같은 폴더의 임시 파일에 쓴 뒤 이름을 바꿔 통째로 바꾼다(`lib.rs` `write_atomic`). 고쳐 쓸 때 기존 파일을 읽거나 해석할 수 없으면 덮어쓰지 않고 오류를 낸다(다른 컴포넌트의 기록을 지우지 않게). 단 `installed-files.json`의 내용이 깨졌으면 `installed-files.json.corrupt-<시각>`으로 옮겨 두고 새로 쓴다(그대로 두면 기록이 영영 안 되어 모든 모듈·익스텐션이 다시 받기 대상으로 남는다). `installed-manifest.json`을 해석할 수 없으면 업데이터는 표 없는 `--apply`를 옛 데몬 전환 경로로 받아 주지 않는다(`launch_ticket.rs` `installed_core_predates_tickets`). 기록하는 쪽은 경고만 남기고 적용·설치는 그대로 성공한다.
- ~~**해시 의미 불일치**: manifest의 zip 해시를 압축을 푼 파일과 비교해 항상 `Tampered`가 된다~~ → 해결: manifest 해시는 다운로드 검증에만 쓰고, 무결성 검사는 설치 때 기록한 파일 해시를 본다(`integrity.rs:1-6`).
- 다운로드는 해시를 확인한다: `download_verified`가 받은 파일의 sha256을 manifest 값과 대조하고(`lib.rs:673, 727`), 적용 직전에 staging 파일을 다시 대조한다(`lib.rs:876-886`). 응답 머리나 다음 조각이 30초(`download::IDLE_TIMEOUT`) 동안 오지 않으면 끊고, manifest의 `size_bytes`보다 많이 오면 끊는다. `<이름>.part`에 받아 다 맞을 때만 이름을 바꾸므로, 다시 받다 실패해도 이미 받아 둔 파일은 그대로다(`download.rs`).
- daemon API는 `GET /api/updates/integrity`이다(`src/ipc/updates.rs:115, 814`).

### 1.6 다운로드 → apply → swap → rollback 흐름

1. **확인**: `POST /api/updates/check`와 daemon의 주기 확인이 같은 `check_and_follow_up`을 거친다(`updates.rs:169, 217-253`) → `check_for_updates()` (`lib.rs:482-557`). 릴레이 manifest 하나로 모든 컴포넌트를 비교한다. 이미 받아 둔 파일이 있고 최신 버전이 그대로면 적용 대기 상태를 이어 간다(`keep_staged`, 다운로드 사이에 주기 확인이 끼어도 적용할 것이 사라지지 않는다). Locales 업데이트가 있으면 사용자에게 알리지 않고 백그라운드에서 다운로드·적용한다(`updates.rs:225-231`, `silent_apply_locales` 279). Locales도 모듈처럼 백업한 뒤 폴더를 통째로 바꾼다(`replace_dir`, 실패하면 되돌린다). `auto_download`이면 나머지 업데이트도 미리 받아 두고 pending manifest를 저장한다(`updates.rs:234-250`). 여러 개를 받다 하나가 실패해도 나머지는 받고, 받은 것은 pending에 남긴 뒤 오류를 돌려준다. 상태의 `release_notes`는 GUI 언어(`config.json` `gui.language`, 없으면 영어)로 완료 마커와 같은 규칙(`notes_for`)으로 고른다. 적용은 언제나 사용자가 고른다.
2. **다운로드**: `POST /api/updates/download` → staging 디렉터리로 받는다.
   - Windows: `%APPDATA%/saba-chan/updates/`
   - Unix: `~/.cache/saba-chan/updates/` (`constants.rs:148-167`)
   - 진행률은 `GET /api/updates/download/progress`로 본다(`updates.rs:450`).
3. **apply 분기**: `POST /api/updates/apply {components:[…]}` (`updates.rs:475-596`). 본문이 없거나 공백이면 전체 대상, 깨진 JSON이면 400 `invalid_body`다(`:348-357`). download도 같다
   - 대상에 `Gui`, `Updater`, `CoreDaemon` 중 하나라도 있으면 **전부 updater exe에 맡긴다**(`updates.rs:511-520`). 이때 daemon은 `staging/pending.json`(`state.rs:19-45`)과 `staging/apply-targets.json`(`state.rs:109-116`)을 쓰고 `{requires_updater:true, needs_updater:[…]}`를 응답한다.
   - 그렇지 않으면 daemon이 직접 `apply_single_component`를 실행한다. 그 뒤 모듈은 `supervisor.refresh_modules()`, 익스텐션은 `extension_manager.rescan()`으로 hot-reload한다(`updates.rs:559-587`).
4. **updater exe 실행**은 데몬이 한다: `POST /api/updates/launch-updater {relaunch}` → 데몬이 일회용 실행 표를 끊고(`launch_ticket.rs:1-5`) `saba-chan-updater --apply --wait-pid <데몬 PID> --launch-token <표> [--relaunch …]`를 분리 실행한 뒤 스스로 종료한다(`src/ipc/updates.rs:609-675`). 그사이 다시 불리면 409 `updater_already_launched`로 거절한다. 표를 새로 끊으면 먼저 띄운 업데이터의 표가 무효가 되어 아무것도 적용되지 않은 채 데몬만 끝나기 때문이다. 그 전에 데몬에 등록된 GUI가 떠 있으면 끄고(GUI가 부른 경우는 스스로 끝날 3초를 기다린다), `--relaunch`를 그 GUI로 바꾼다. CLI·자동 업데이트로 시작해도 GUI 파일이 잠겨 교체가 실패하지 않는다. GUI(Tauri 셸 `updater_launch_apply`, `saba-chan-gui/src-tauri/src/native.rs:81-102`. `relaunch`는 자기 exe 경로 `current_exe()`)와 CLI(PR #6 기준 `update apply`, `saba-chan-cli/src/cmd/update.rs:249`, `client.rs:785-789`)는 이 API만 부른다.
5. **updater exe** (`updater/gui/src-tauri/src/lib.rs`)
   - 인자는 `--apply [--wait-pid <pid>] --launch-token <t> [--relaunch <exe> [extra…]]`다(`lib.rs:408-423`). 표가 없거나 맞지 않으면 적용하지 않고 끝낸다. 표를 모르는 옛 데몬(0.1.2 미만)이 띄운 경우만 표 없이 받는다(`lib.rs:427-448`). `--wait-pid`가 있으면 그 PID가 끝나기를 최대 30초 기다린다(`lib.rs:450-467`). install_root는 자기 exe 위치로 정한다(`lib.rs:471-477`).
   - `load_pending_manifest()` → `apply-targets.json`의 key를 차례로 `apply_single_component()`(`apply_target_keys`. 목록이 없을 때의 전체 적용처럼 첫 실패에서 멈추고, 남은 key는 `failed`에 "skipped after an earlier failure"로 적는다. 목록이 없으면 받아 둔 것 전체를 `pending_keys_in_apply_order()` 순서로 같은 `apply_target_keys`에 넘긴다. 매니페스트를 못 읽으면 실패 1건으로 알린다) → 성공·실패 모두 `UpdateCompletionMarker`(`<data_dir>/update-complete.json`, 실패 목록 `failed` 포함)를 저장 → 마커의 `release`는 다운로드 때 데몬이 `pending.json` 옆에 남긴 `pending-release.json`(릴리즈 버전·언어별 패치노트)에서 온다(`state.rs` `save_pending_manifest`/`pending_release`) → `clear_pending_manifest()`가 `pending.json`/`apply-targets.json`/`pending-release.json`과 받아 둔 압축 파일(`.zip`, `.tar.gz`, `.part`, 실패한 것 포함)을 지움(updater-lib `state.rs:84-104`) → 2초 뒤 `--after-update`를 붙여 relaunch(`lib.rs:73-216`, 312-350). 진행 상황은 Tauri 이벤트 `apply:progress`(`step`·`message`·`percent`·`applied`·`failed` 수)로 알린다. `message`는 진단용 영어 원문이고, 실패 화면은 `applied`·`failed` 수로 번역한 요약을 보인다. 다음 데몬이 결과를 `GET /api/updates/last-result`로 보여 준다(`updates.rs:787-792`).
   - relaunch는 Windows에서 `CREATE_BREAKAWAY_FROM_JOB`으로 띄우고, Job이 허용하지 않으면(os error 5) 그 플래그 없이 다시 띄운다(`lib.rs:346-362`). `--relaunch`가 없으면 설치 폴더의 GUI exe(`saba-chan-gui.exe`, 0.1.1 zip의 `Saba-chan.exe`)를 찾는다(`lib.rs:290-292`, updater-lib `lib.rs:279-285`). `--relaunch`가 GUI exe 이름(`saba-chan-gui.exe`, `Saba-chan.exe`)인데 그 파일이 없으면(적용 중 `.exe.old`로 비켜 난 경우) 설치 폴더의 GUI exe를 대신 띄우고 받은 인자를 그대로 넘긴다. 설치 폴더에서도 못 찾으면 받은 경로를 기다린다(`lib.rs` `relaunch_command`). GUI를 적용하면서 exe 이름이 바뀌면(0.1.1 `Saba-chan.exe` → `saba-chan-gui.exe`) 예전 exe는 `.exe.old`로 비켜 두고(다음 정리 때 지운다) 바탕화면·시작 메뉴 바로가기를 새 exe로 돌린다(`apply.rs` `retire_other_gui_exes`). 0.1.1 업데이터는 `saba-chan-gui.exe`만 찾으므로 0.1.1에서 올릴 때는 GUI가 첫 적용에서 빠지고, 새 데몬으로 한 번 더 적용해야 GUI까지 바뀐다(0.1.1 코드라 고칠 수 없다).
   - Tauri 커맨드: `get_apply_mode`, `start_apply`, `get_preferred_language`, `get_theme`, `check_after_update`.
6. **바이너리 swap** (`apply_binary_update` `apply.rs:94-177`, gui 184-230)
   - 대상 프로세스가 끝나기를 최대 15초 기다린다. 시간이 지나도 "attempting update anyway"로 그냥 진행한다.
   - Windows는 실행 중인 `X.exe`를 `X.exe.old`로 rename하고(재시도 5회, `apply.rs:64-92`, 366-382) zip에서 새 파일을 쓴다. `X.exe.old`가 아직 잡혀 있어 지울 수 없으면(지난 업데이트 뒤로 계속 도는 서버의 `saba-runner` 등) `X.exe.old.1`, `.old.2` … 중 비울 수 있는 이름을 쓴다(`apply.rs` `free_backup_path`). Linux는 알려진 바이너리만 `X.old`로 rename한다.
   - `cleanup_old_files()`는 install_root 바로 안의 사바쨩 실행 파일 `*.exe.old`·`*.exe.old.<n>`만 지운다. 적용 직후에 한 번, 아직 잡혀 있어 못 지운 것은 다음 `UpdateManager` 생성 때 지운다(`lib.rs:410-411, 837-840, 919-939`).
7. **rollback**
   - 모듈·익스텐션은 staging에 `*_backup` 복사본을 만들고, 교체 도중 실패하면 그 백업으로 되돌린다. 성공하면 백업을 지운다(`replace_dir`, `apply.rs:24-62`).
   - 바이너리와 GUI(설치 폴더 바로 안의 exe든 폴더 모드든)는 압축 해제 도중 실패하면 새로 생긴 파일을 지우고 비켜 둔 `.old`·`.update-bak`을 원래 이름으로 되돌린다. zip과 tar.gz가 같은 경로를 쓴다(`apply.rs` `extract_or_restore`). 적용이 끝난 뒤에 이전 판으로 돌아가는 경로는 없다.
   - ~~`prepare_daemon_update`는 호출하는 곳이 없다(죽은 코드)~~ → 해결: 함수가 지워졌다(grep 0건).
8. **설치 버전 기록**: `update_installed_versions_batch` → `installed-manifest.json`(`state.rs:173-193`). 코어가 바뀌면 Windows "설치된 앱" `DisplayVersion`도 갱신하는데, 이 설치 폴더의 항목만 고친다(`state.rs:199-207`, `uninstall_entry.rs:76-85`).

### 1.7 daemon, GUI, CLI가 updater를 부르는 방식

**daemon HTTP API** (`saba-chan/src/ipc/updates.rs:106-118`, `ipc/mod.rs:812`에서 merge):

| Method | Path | 동작 |
|---|---|---|
| GET | `/api/updates/status` | 캐시된 상태. Locales는 숨긴다 |
| POST | `/api/updates/check` | 릴레이를 조회한다 |
| POST | `/api/updates/download` | body `{components:[]}`. 본문이 없으면 전부 받는다 |
| GET | `/api/updates/download/progress` | 진행률 |
| POST | `/api/updates/apply` | body `{components:[]}`(없으면 전부). 직접 적용하거나 updater 위임을 판단한다 |
| POST | `/api/updates/launch-updater` | body `{relaunch:[exe, …]}`. 데몬이 updater를 띄우고 스스로 종료한다(`updates.rs:609-675`). 이미 띄웠으면 표를 새로 끊지 않고 409 `updater_already_launched` |
| GET / DELETE | `/api/updates/last-result` | updater나 데몬이 남긴 마지막 적용 결과 `{ok, result, version, notes, acknowledged}`. 재시작 없이 데몬이 바로 적용한 업데이트(모듈·익스텐션·설치기 등)도 같은 마커를 남긴다(`record_daemon_apply`, 릴리즈는 `applying_release`). `result`는 마커 그대로이고, `result.release`에 적용한 릴리즈의 `{version, released_at, notes:{언어:본문}}`이 있다(이전 업데이터가 남긴 마커에는 없음). `?lang=ko`이면 `notes`에 그 언어 패치노트를 고른다. 없으면 지역 변형을 떼고, 그다음 en→ko 순이다(`ReleaseSummary::notes_for`). DELETE는 확인 처리이며 마커를 지우지 않고 `acknowledged: true`만 적는다. GET은 확인한 결과를 `result: null`로 숨기고(GUI가 다시 띄우지 않게), `?include_seen=true`면 확인한 결과도 `acknowledged: true`와 함께 준다(CLI `update last`가 패치노트를 다시 볼 때, `updates.rs` `get_last_result`) |
| GET | `/api/updates/integrity` | 설치 때 기록한 파일 해시와 비교 |
| GET / **PUT** | `/api/updates/config` | 설정 `{enabled, check_interval_hours(1-8), auto_download, install_root, relay_url, channel}`(`auto_apply`는 f498525에서 빠졌다). `config.json`의 `updates` 절에 저장한다. GET은 `mock_relay`(모의 릴레이를 믿는 개발 빌드인지)도 준다(`updates.rs:844-942`). 업데이트 출처(`relay_url`)는 앱에서 바꾸지 않는다: 릴리즈 빌드의 PUT은 비우기만 받고 다른 값은 403 `relay_url_not_settable`(D7). 0.1.1이 남긴 예전 키 `relay_base_url`도 `relay_url`로 읽고, 빈 값은 기본 릴레이다(`updater/src/lib.rs` `UpdateConfig`) |

불일치:
- ~~GUI의 `updater:setConfig`가 POST로 보내 405가 난다~~ → 해결: PUT으로 보낸다(옛 Electron main.js:2505-2507, 지금은 `saba-chan-gui/src/api/createApi.js:781`).
- ~~CLI의 `apply_updates`와 `download_updates`는 body 없이 POST하는데 `Json` extractor가 거부할 수 있다~~ → 해결: PR #6 CLI는 `{components:[...]}` 본문을 늘 보낸다(`saba-chan-cli/src/client.rs:774-784`). 데몬도 본문을 바이트로 받아 비었으면 기본값(전체)으로 읽고, 깨진 JSON만 400으로 거부한다(`src/ipc/updates.rs:348-371, 475-480`).
- ~~CLI가 daemon에 없는 `/api/install*`을 호출한다~~ → 해결: PR #6 CLI에 그 호출이 없다(grep 0건).
- ~~CLI `launch-apply`가 넘기는 `--install-root`와 대상 인자를 updater가 무시한다~~ → 해결: PR #6 CLI는 `update apply`에서 데몬이 updater가 필요하다고 답하면 확인 후 `POST /api/updates/launch-updater {relaunch:[saba-core, --spawned]}`만 부른다(`saba-chan-cli/src/cmd/update.rs:237-249`, `client.rs:785-789`).
- ~~자동 확인 주기를 GUI의 `setInterval`이 돌린다~~ → 해결: daemon이 `spawn_update_scheduler`로 기동 30초 뒤부터 `check_interval_hours`마다 확인한다(`updates.rs:256-283`, `src/main.rs:466`). GUI는 10분마다 캐시된 `GET /api/updates/status`만 읽어 알린다(Tauri 셸 `saba-chan-gui/src-tauri/src/lifecycle.rs:539-581`, `events.rs:16`). ~~`auto_apply`는 어디에서도 쓰이지 않는다~~ → 해결: 설정에서 지웠다. `auto_download`만 남았다(`updates.rs:215-216`).

---

## 2. Installer (`saba-chan/installer/gui`, Tauri, v0.1.2)

모듈 구성: `lib.rs`, `release.rs`(릴리즈 받기, 업데이터와 같은 relay 경로), `runtime_bootstrap.rs`, `registry.rs`, `shortcuts.rs`, `uninstall.rs`.

**설치 위치**
- 기본 경로는 Windows `C:\Program Files\Saba-chan`, Unix `~/.local/share/saba-chan`(`installer/gui/src-tauri/src/lib.rs:797-814`).
- 고른 폴더가 비어 있지 않고 사바쨩 설치(`saba-core`나 GUI exe)도 아니면 그 아래 `Saba-chan` 폴더에 설치한다. 찾아보기·직접 입력 모두 `set_install_path`가 바꾼 경로를 돌려줘 화면에 보인다(`lib.rs` `dedicated_install_dir`).
- 설치기가 새로 만든 폴더에만 `.saba-chan-install` 표시 파일을 둔다(원래 있던 빈 폴더는 나중에 다른 파일이 생길 수 있어 표시하지 않는다). 제거기는 이 표시가 있을 때만 폴더를 통째로 지우고, 없으면(0.1.4 이전 설치, 사용자 파일이 있던 기존 설치 폴더) 사바쨩 실행 파일·`locales` 폴더·공유 모듈 파일만 지운 뒤 폴더가 비었을 때만 지운다(`uninstall.rs` `remove_install_dir`).
- 쓰기에 실패하면 UAC로 권한을 올려 디렉터리를 만든다(`elevate_create_dir`, `lib.rs:638-731`).
- 데이터는 `%APPDATA%/saba-chan`에 둔다(updater constants와 같다).
- 로그는 `%TEMP%/saba-chan-installer.log`에 남는다(`lib.rs:1001-1016`).

**`do_install` 단계** (`lib.rs:255-576`)

| 단계 | 내용 |
|---|---|
| 1 (0-5%) | install dir 생성, 쓰기 probe, 필요하면 권한 상승 |
| 1.5 | 덮어쓰기 전에 이 설치 폴더의 프로세스를 끈다(`uninstall.rs` `stop_daemon_before_install`). `saba-core`는 실행 파일 경로(받은 경로와 실제 경로 둘 다)로 찾고, 못 찾으면 설정된 포트의 `/health` PID를 보고 그 exe가 이 폴더 안이거나 경로를 못 읽을 때만 대상으로 삼는다. `/api/daemon/shutdown` 후 20초 안에 안 끝나면(요청이 거절되면 바로) 그 PID만 kill. 이어서 이 폴더의 GUI exe와 `saba-chan-cli`를 그 PID만 끈다(트리째 끄면 데몬이 남았을 때 그 아래 게임 서버까지 죽는다). 포터블 GUI는 실행기가 임시 폴더에 풀어 띄우므로, 실행기를 끄기 전에 그 자손(`descendants_in`)을 찾고, 데몬을 끄기 전에 `GET /api/clients`에서 `installRoot`가 이 폴더인 GUI PID를 받아 둔다. 그 PID가 아직 GUI 이름의 프로세스일 때만 끈다. 단계마다 `%TEMP%\saba-chan-installer.log`에 남긴다 |
| 2 (5-10%) | 릴레이에서 서명된 최신 manifest를 받아 검증한다(`release.rs:16-28` `fetch_latest`) |
| 2.5 | manifest의 `installer.version`이 현재 `CARGO_PKG_VERSION`보다 높으면 설치를 차단한다(`lib.rs:353-369`) |
| 3 (10-45%) | 업데이터가 아는 키 중 모듈·익스텐션을 뺀 컴포넌트(`installer` 제외)를 R2에서 받아 SHA-256을 확인하고 `install_dir/<install_dir>`에 푼다(`release.rs:35-45`, `lib.rs:371-425`). 하나라도 실패하면 설치를 멈춘다 |
| 3.5 | 공유 모듈(`i18n.py`, `daemon_rcon.py`)을 `%APPDATA%/saba-chan/modules/`로 복사(`lib.rs:856-884`) |
| 4 (45-55%) | 선택한 모듈을 manifest의 `module-<id>`에서, `requires_extensions`에 따른 익스텐션을 `ext-<id>`에서 R2로 받아 SHA-256을 확인하고 설치한다(`lib.rs:437-476`, 915-948). 푼 폴더는 데몬 카탈로그 설치와 같이 `installed-files.json`에 해시를, `official-components.json`에 키를 적어 신뢰 표시가 `verified`가 된다(`release.rs` `install_addon_zip`). 기록하지 못하면 그 구성요소는 `unverified`로 격리되므로 설치 경고로 보인다. 실패는 경고로 남긴다 |
| 5 (55-70%) | portable Python(python-build-standalone, 판과 SHA-256은 `updater/src/python_pin.rs`에 고정)과 venv (`runtime_bootstrap.rs:42-51`). 실패해도 치명적으로 처리하지 않는다 |
| 6 | 없음: 로컬 Discord bot이 없어져 portable Node·npm install 단계도 지워졌다 |
| 7-8 | `setup_config`, 언어 설정을 `config.json`의 `gui.language`에 저장(`lib.rs:886-911`) |
| 9 | `HKCU\Software\Microsoft\Windows\CurrentVersion\Uninstall\Saba-chan*` 등록. 키는 설치 폴더마다 따로다: 첫 설치본은 `Saba-chan`, 다른 폴더는 `Saba-chan-<폴더 해시 8자>`(`registry.rs:15-77`, `updater/src/uninstall_entry.rs:12-31`) |
| 10 | 바탕화면·시작 메뉴 바로가기. 대상은 설치 폴더의 GUI exe(`saba-chan-gui.exe` 또는 0.1.1의 `Saba-chan.exe`)다. 만들기 전에 언어별 옛 이름(`shortcuts::ALL_SHORTCUT_NAMES`)의 바로가기를 지우되, `.lnk` 대상이 이 설치 폴더 안인 것만 지우고 시작 메뉴 폴더는 비었을 때만 지운다(`shortcuts::remove_shortcuts_into`). 제거(uninstall)도 같은 함수를 쓴다 |
| 10.5 | `installed-manifest.json` 생성. updater 버전 판정의 1순위 소스다(`lib.rs:540-544`). 설치 폴더의 `saba-chan-installer.exe`가 지금 설치기와 같은 파일이면 `installer`에 자기 버전을 적는다(복사가 실패해 옛 사본이 남았으면 적지 않는다) |

레지스트리·바로가기·모듈·익스텐션처럼 실패해도 계속하는 단계는 완료 이벤트의 `warnings`·`log_path`로 보내고, 완료 화면이 목록과 로그 경로를 남겨 보여 준다(`installer.js` `enterComplete`). 설치를 멈추는 실패는 `step:'error'`에 `error_code`(`elevation_failed`·`fetch_failed`·`installer_outdated`·`download_failed`·`extract_failed`)와 `log_path`를 싣고, 화면은 코드별 번역 문구와 원문·로그 경로, [다시 시도](설정 시트로)를 보인다. "사바쨩 실행"이 실패하면 창을 닫지 않고 안내한다.

코어 zip을 풀 때 덮어쓸 exe가 잠겨 있으면(켜진 서버의 `saba-runner` 등) 업데이터와 같은 `move_aside_binary`로 `.exe.old`로 비켜 두고 계속한다.

Tauri 커맨드(`lib.rs:117-775`): `get_installer_state`, `set_install_path`, `set_language`, `set_shortcut_options`, `set_selected_modules`, `get_available_modules`(같은 manifest의 `module-*`, 폴백 없음), `fetch_latest_release`, `start_install`, `start_uninstall`, `get_app_mode`, `launch_app`, `get_preferred_language`, `browse_folder`. GUI는 `--uninstall` 인자로 installer를 uninstall 모드로 실행한다(`saba-chan-gui/src-tauri/src/native.rs:149-184`).

설치와 제거는 한 번에 하나만 돈다. 이미 돌고 있으면 `start_install`·`start_uninstall`이 `busy`로 거절하고 화면은 진행 중인 것을 그대로 둔다(`lib.rs` `WorkGuard`). 설치기가 새로 만든 폴더의 전용 표시(`uninstall::INSTALL_MARKER`)는 폴더를 만든 바로 뒤에 쓴다(받다 실패한 뒤 다시 시도해도 전용 폴더로 남는다). Python 런타임 준비나 공유 모듈(`i18n.py`·`daemon_rcon.py`) 복사가 실패하면 설치는 이어 가되 완료 화면 경고에 띄운다.

**제거** (`uninstall.rs`): 대상은 제거기 자신이 든 설치 폴더 하나다(`uninstall.rs` `target_dir`). 설정된 포트의 데몬이 이 폴더의 것이면 먼저 `GET /api/servers`에서 꺼져 있지 않은 서버마다 `POST /api/instance/:id/stop`으로 정상 종료를 요청해 기다린다(서버마다 최대 90초). 이어서 데몬 아래 프로세스(러너 없이 띄운 서버 등. 제거기 자신과 그 위, 부모보다 먼저 시작한 프로세스는 뺀다, `tree_of`)를 적어 두고 데몬에 `/api/daemon/shutdown`을 보내, 요청이 받아들여졌으면 끝날 때까지 최대 20초(마지막 목록 조회가 걸리면 그만큼 더) 기다린다. 목록을 못 읽으면 자식을 모르므로 정상 종료 대신 데몬을 트리째 끊는다. 그다음 그 폴더에서 실행 중인 사바쨩 프로세스만 끝내고(`uninstall.rs` `stop_saba_processes`), 적어 둔 데몬 아래 프로세스 중 같은 PID·이름·시작 시각으로 남은 것도 트리째 끝낸다. 끈 프로세스(GUI 포함)는 PID·이름·시작 시각으로 적어 두고 실제로 끝났는지 최대 10초 확인하며(기다리는 사이 PID가 다른 프로그램에 다시 쓰였으면 끝난 것으로 본다), 아직 같은 프로세스로 남았으면 한 번 더 끊고 다시 최대 10초 기다린다(`wait_for_exit`). 프로세스 목록을 못 읽은 회차는 끝나지 않은 것으로 센다. 목록 조회(PowerShell)와 `taskkill`은 한 번에 10초까지만 기다린다(`output_within`). 끝나기 전에 지우면 끝나는 중인 프로세스가 파일을 잡고 있거나 다시 쓴다. 포터블 GUI 실행기가 임시 폴더에 풀어 띄운 실제 GUI(실행기의 자손, 제거기 자신은 뺀다, `descendants_in`)와 데몬에 이 설치로 등록된 GUI(`GET /api/clients`)도 끝낸 뒤, 그 폴더의 레지스트리 항목만 지운다(`registry.rs:78-93`). `%APPDATA%/saba-chan`과 모듈 폴더는 설치본들이 함께 쓰므로 다른 설치본 항목이 남아 있으면 지우지 않는다(`uninstall.rs` `do_uninstall`의 `last_install`, `uninstall_entry.rs:42-48`). 제거기 옆에서 설치 폴더를 찾지 못하면 마지막 설치본으로 치지 않아 사용자 데이터를 지우지 않는다. `SABA_DATA_DIR`·`SABA_MODULES_PATH`·`SABA_EXTENSIONS_DIR`로 데이터 폴더를 바꿔 두었으면 그 폴더는 남의 것일 수 있어 지우지 않는다(`owned_data_dir`). 앱이 만들지 않는 `.keys`(업데이트 서명 키)는 어느 경우에도 지우지 않는다. `keep_settings`(화면 문구 "게임 서버와 설정 남기기")이면 다시 받거나 만들 수 있는 런타임·캐시(모듈, 익스텐션, Python 런타임, 업데이트, 로그, 무결성·검사 기록 등, `uninstall_entry.rs` `RUNTIME_ENTRIES`)만 지우고 그 밖의 것(설정, 인스턴스, 토큰, 앞으로 생길 파일)은 남긴다(`removable_data_entry`). 지운 뒤 지울 항목이 남아 있으면 1초 간격으로 두 번 더 지우고, 그래도 남은 것은 로그에 남기고 결과에 알린다: 화면은 완료 문구 대신 "일부 파일이 남았습니다"와 남은 경로를 보여 주고(`UninstallProgress.left`), 조용한 제거(`--uninstall --silent`)는 남은 경로를 찍고 종료 코드 1로 끝난다.

**문제점**
- 기본 설치 경로가 `Program Files`(관리자 권한 필요)인데 레지스트리는 HKCU에 쓴다. 더 큰 문제는 updater exe와 daemon이 **권한 상승 없이** `install_root`에 쓴다는 점이다. 기본 경로에 설치하면 업데이트 적용이 권한 문제로 실패할 수 있다(추정, 매니페스트의 requestedExecutionLevel은 확인하지 않음).
- 모듈·익스텐션 소스는 하나다: 설치기, 데몬 목록, 업데이터가 모두 같은 릴리즈 manifest를 본다.

---

## 3. CI / 릴리즈 파이프라인

### 3.1 saba-chan (`.github/workflows`)

| 파일 | 트리거 | 내용 |
|---|---|---|
| `ci.yml` | PR to main, develop / tag `v*` push / `workflow_dispatch` | 하나뿐인 검사 워크플로(옛 `test.yml`은 중복이라 b0b5631에서 지웠다). PR에서는 로컬(클라우드 작업·리뷰, PC 점검)에서 돌리지 않는 것과 릴리스를 깨뜨리는 것만 본다. `changes` job이 base와의 diff로 돌릴 job을 고른다. PR: `guard`(`versions.py check`, `.github/scripts` 패키징 시험, 항상) · `rust` Windows만(fixture 모듈 생성 → `cargo clippy --workspace -D warnings` → `cargo test --workspace`, 문서·릴리스 노트·GUI 화면 소스(`saba-chan-gui/src/`)만 바뀌면 건너뜀. `public/`은 트레이 아이콘을 `include_bytes!`로 묶으므로 건너뛰지 않는다) · `frontend`(GUI `npm ci` → `vite build`, updater/installer 스크립트 `node --check`, 프런트·locales 변경 시). 전체(태그 push, `workflow_dispatch`): 위에 더해 Ubuntu `rust`, GUI vitest, `e2e`(`tests/e2e/run-cloud.sh`, 모듈·익스텐션 `main` checkout). 같은 PR의 이전 실행은 취소한다. main push에는 돌지 않는다. Tauri GUI 셸(`saba-chan-gui/src-tauri`)은 workspace 멤버라 `rust`의 clippy·test에 들어간다 |
| `release.yml` | `release: [published, edited]`, `workflow_dispatch(tag_name)` | **빌드는 하지 않는다.** 사람이 Draft Release에 올린 raw exe/zip을 `gh release download`(릴리즈의 에셋이 크기까지 모두 받아지지 않으면 실패) → `package-release.py`로 표준 zip 재패키징 → `locales.zip`. 이번 릴리즈에 없는 구성요소는 이전 릴리즈에서 가져오는데, 태그 버전이 현재보다 낮은 것만 높은 순으로 보고 정식판은 prerelease(GitHub 표시나 `-pre` 태그)에서 가져오지 않는다. `saba-runner.exe`가 staging에도, 올린 saba-core zip 루트에도 없으면 실패한다. 태그는 `v1.2.3[-pre]` 꼴이어야 하고, GitHub prerelease인데 `versions.toml` 채널이 stable이면 실패한다. 그다음 R2 단계: 태그가 `versions.toml`의 release 버전과 다르면 실패 → 릴레이와 R2에 같은 버전이 있으면(확인 실패 포함) 실패(단 `edited` 이벤트로 다시 돈 실행은 릴레이에 이미 있으면 R2·릴레이 단계와 GitHub 에셋 교체·정리를 모두 건너뛴다. 같은 태그의 실행은 줄을 서고 진행 중인 것을 취소하지 않는다) → modules·extensions 저장소 `main`을 checkout → `scripts/versions.py manifest`가 모듈·익스텐션 zip까지 묶어 manifest 생성 → `release_signing` 예제를 키 없이 먼저 빌드하고, 키는 실행 단계에만 넘겨 `saba-updater-v1` 키 서명(번들 공개키로 검증까지) → R2 `releases/<version>/` 업로드 → 릴레이 `POST /admin/releases` 등록(`release-notes/<version>.json`의 `{언어: 본문}`을 `notes`로 싣는다. stable 채널은 이 파일이 없으면 등록 본문을 만들지 않고 실패한다(클라이언트는 받을 때의 노트를 적용 뒤 패치노트로 보여 준다). 다른 채널은 노트 없이 등록하고, 나중에 `PATCH /admin/releases/:id`로 고칠 수 있다. 본문은 한글을 `\uXXXX`로 이스케이프한 ASCII JSON이고 `--data-binary`로 보낸다, `scripts/versions.py:370-373`, `release.yml:248-251`). 마지막에 `scripts/changelog.py`로 만든 릴리즈 노트를 붙인다(`release.yml:1-48, 122-255, 294-304`) |

- GitHub Release는 사람이 raw 파일을 올리는 **입력 칸**으로만 쓴다. 업데이터·설치기는 GitHub를 보지 않는다.
- 실제 빌드는 로컬 Windows의 `scripts/build-windows.ps1`이 한다. 버전은 `versions.toml`에서 읽는다(`build-windows.ps1:27-28`). GUI 화면을 `npm run build`(vite)로 먼저 만들고(`:59-72`, Tauri GUI가 `build/`를 exe에 넣는다), 이어서 `cargo build --release --workspace`(`:98`)가 데몬·CLI·업데이터·인스톨러와 함께 `target/release/saba-chan-gui.exe`를 만든다(`:127`).
- `package-release.py`는 **windows-x64 asset만** 만든다. manifest도 `--platform windows-x64`로 만든다.
- 필수 구성요소는 `versions.toml`의 `[components.<키>] required = true`(지금은 `saba-core`, `gui`, `updater`) 한 곳에서 정한다. 이번 릴리즈에도 이전 릴리즈에도 없으면 `package-release.py`가 실패한다(`missing_required`). 설치기는 같은 파일을 빌드에 담아 읽고, manifest에 하나라도 없으면 기존 데몬을 끄기 전에 `release_incomplete`로 멈춘다(`installer/gui/src-tauri/src/release.rs`의 `required_components`).
- `package-release.py`는 zip을 표준 형태로 다시 담는다: 경로 구분자를 `/`로 바꾸고, GUI zip 루트의 앱 exe를 `saba-chan-gui.exe`로 바꾸며(Tauri 빌드는 이미 `saba-chan-gui.exe`라 그대로다. `Saba-chan.exe` 개명은 옛 Electron zip에만 해당한다), 다른 컴포넌트의 exe는 뺀다(`package-release.py:147-184`). 다만 컴포넌트의 `bundles`에 적은 exe는 남기므로 raw saba-core zip에 든 `saba-runner.exe`가 재포장에서 빠지지 않는다(`:58-59, 166-169`). saba-core zip에는 `saba-runner.exe`와 공유 모듈(`i18n.py`, `daemon_rcon.py`)이 같이 들어간다(`package-release.py:48-60`).

### 3.2 modules / extensions repo

- 두 저장소 모두 자체 릴리즈 워크플로가 없다. `saba-chan-modules`에는 검사용 `check-modules.yml`(PR은 conformance만, `workflow_dispatch`는 모듈 단위 pytest까지), `saba-chan-extensions`에는 ruff 치명 규칙·pytest 검사용 `ci.yml`(PR은 Windows만, `workflow_dispatch`는 Windows+Ubuntu)만 있다. 둘 다 main push에는 돌지 않는다.
- 둘 다 코어 `release.yml`이 `main`을 checkout해서 같은 릴리즈 manifest에 싣는다(§3.1, [02-modules.md](02-modules.md) §5, [03-extensions.md](03-extensions.md) §3.4).

---

## 4. Relay server (server-chan, 비공개 저장소)

> server-chan `main` 기준(2026-10-18). 프레임워크는 **Fastify 5**, DB는 PostgreSQL + drizzle이고, 기동할 때 `./drizzle`(0000–0022)을 자동 migrate한다(`src/index.ts`). 앱 조립은 `src/app.ts` `buildApp`, 디스코드 봇은 같은 프로세스의 `src/bot/`, 챗봇은 `src/chatbot/`이다. **단일 인스턴스 전제**다: poll·result 대기열, ack를 보낸 호스트 목록(`services/redelivery.ts`), 노드 토큰 검증 캐시, 레이트리밋 카운터, 봇 캐시(prefix·잠자기·언어), 음악·공지·챗봇 진행 상태는 프로세스 메모리에 있고 재시작하면 DB에서 다시 채우거나 사라진다(`src/index.ts` 머리 주석).

### 4.1 전역 동작

- 클라이언트 게이트(`app.ts` `clientGate`): `X-Saba-Client: 1`이 없는 브라우저 접속에 안내 페이지(403/404 HTML)를 보여 준다. 보안 장치가 아니라 안내용이다. `/admin`, `/api/bot-invite`, 컨테이너 프로브용 `/health`, 내부 스크립트 호출은 통과한다.
- CORS는 `app://`, `tauri://`, 웹사이트 origin, dev 모드의 localhost, `CORS_ORIGINS`만 허용한다.
- 레이트리밋은 전역 100/min이고 라우트별 값은 `middleware/rateLimit.ts` `ROUTE_RATE_LIMITS`에 있다. 노드 경로(`/poll`, `/heartbeat`, `/events`, `/ack`, `/result/:id`)는 한 번 검증된 토큰이면 노드 단위로, 아니면 IP로 센다.
- 프로덕션 필수 env: `RELAY_INTERNAL_SECRET`, `ADMIN_PAGE_KEY`, `DISCORD_TOKEN`, `DATABASE_URL`, `SELF_URL`. 없으면 기동하지 않는다.
- 프로덕션의 5xx 응답은 내부 메시지 없이 `INTERNAL_SERVER_ERROR`만 돌려준다.
- **인증 4종**
  - 노드 토큰 `authenticateNode`(`middleware/auth.ts`): `Authorization: Bearer sbn_<hostId>.<secret>`를 DB `hosts.token_hash`(argon2id)로 검증하고 5분 캐시한다. `req.node = {id: hostId, generation}`(`generation`은 그 호스트의 토큰을 바꾼 횟수, 메모리 기록). 검증하는 사이에 토큰이 바뀌었으면 DB를 한 번 다시 읽고 예전 결과는 캐시하지 않는다. 실패는 401 `MISSING_TOKEN`·`INVALID_TOKEN_FORMAT`·`NODE_NOT_FOUND`·`INVALID_TOKEN`이고, 같은 계정의 재페어링으로 토큰이 바뀐 지 7일 안이면 `INVALID_TOKEN` 대신 `TOKEN_REPLACED`다(메모리 기록이라 relay를 다시 켜면 `INVALID_TOKEN`으로 돌아간다). 데몬은 heartbeat나 poll에서 `TOKEN_REPLACED`를 받으면 에이전트를 멈추고 상태에 `lastErrorCode: "token_replaced"`를 남긴다(`relay_agent.rs`, CLI `bot status`가 현지화해 보인다). 요청마다 디스크의 토큰으로 서명하고 이 PC에서 다시 페어링하면 새 토큰 저장이 에이전트를 다시 띄우므로, 같은 PC의 재페어링은 따로 다루지 않는다. 다른 401은 기록하고 재시도한다. 데몬이 함께 보내는 `x-request-timestamp`·`x-request-nonce`·`x-request-signature`는 받아들이되 검증하지 않는다(키가 같은 요청의 토큰이라 TLS 위에서 더하는 보호가 없다).
  - 내부 시크릿 `verifyInternal`(`middleware/internalAuth.ts`): `x-internal-secret`을 `RELAY_INTERNAL_SECRET`과 상수시간 비교한다. 릴리즈 스크립트·봇 상태 조회용.
  - 노드 또는 내부 `authenticateNodeOrInternal`: 노드 토큰이면 자기 호스트(`isForeignHost`)와 그 호스트의 길드만 다룬다.
  - 관리자 키 `verifyAdmin`: `x-admin-key`를 `ADMIN_PAGE_KEY`와 비교한다. 키가 비어 있으면 항상 거부한다.
- 토큰 형식: `sbn_{hostId}.{nanoid(48)}`(`services/nodeToken.ts`). 코드에서 "node"와 "host"가 같은 대상을 가리키는 곳이 남아 있다(`req.node.id`는 hostId다).
- zod 스키마는 `middleware/schemas.ts` 한 곳에 있다.

### 4.2 HTTP API 표

| Method | Path | 인증 | Body / Query | 응답 | 위치 |
|---|---|---|---|---|---|
| GET | `/info` | — | — | `{name, version, apiVersion:2, minAgentVersion}`. `minAgentVersion`은 환경 변수 `MIN_AGENT_VERSION`(SemVer, 없으면 `0.1.0`, SemVer가 아니면 릴레이가 시작하지 않는다). 앱은 연결할 때 이 값보다 낮으면 스스로 멈춘다(`relay_agent.rs`) | `app.ts`·`services/agentVersion.ts` |
| GET | `/health` | — | — | `{status, uptime, db}` 200/503 | `app.ts` |
| GET | `/bot-status` | 내부 | — | 봇 연결 상태 | `app.ts` |
| GET | `/api/bot-invite` | — | — | `{clientId, inviteUrl, bot}` / 503 `BOT_NOT_CONFIGURED` | `app.ts` |
| POST | `/api/pair/initiate` | —(3/min) | `{relayUrl?}` | `{code(nanoid32), pollSecret(nanoid48), expiresAt, ttl:300}`. pollSecret은 sha256으로 저장 | `routes/pair.ts` |
| GET | `/api/pair/:code/status` | `X-Poll-Secret` 헤더(30/min) | — | `{status:'pending', expiresAt}` / `expired` / `{status:'claimed', hostId, nodeToken}`. nodeToken은 수령 기한(코드 만료와 클레임 뒤 2분 가운데 늦은 쪽)까지 다시 읽을 수 있고, 기한이 지나면 `null`이다. 헤더가 없으면 401 `MISSING_POLL_SECRET`, 틀리면 403 | `pair.ts` |
| GET | `/api/hosts/:hostId` | 노드(자기 호스트)/내부 | — | `{id, name, status, lastHeartbeat, agentVersion, createdAt}` | `routes/host.ts` |
| PATCH | `/api/hosts/:hostId/bot-config` | 노드/내부 | `{prefixes?, prefix?, moduleAliases?, commandAliases?}` (`prefixes`가 있으면 `prefix`보다 우선) | `{success, botConfig}`. 지금 데몬은 heartbeat `botConfig`로 보내므로 쓰지 않는다 | `host.ts` |
| GET | `/api/hosts/:hostId/nodes` | 노드/내부 | — | `[{guildId, hostId, guildName, locale, iconUrl, prefixes, prefixDefault, createdAt, updatedAt}]`. `iconUrl`은 봇이 그 길드에 있고 아이콘이 있을 때만(128px png, 없으면 null), `prefixes`는 지금 쓰는 접두사(기본값이면 응답 언어의 글자), `prefixDefault`는 기본값을 쓰는 중인지 | `routes/node.ts` |
| GET | `/api/nodes/:guildId/members` | 노드/내부(10/min) | — | `[{id, username, displayName, level}]`(`node_permissions` 캐시) | `node.ts` |
| GET | `/api/nodes/:guildId/discord-members` | 노드/내부(5/min) | — | 봇이 조회한 길드 멤버(각각 `avatarUrl` 64px png). 전체 요청은 길드마다 10분에 한 번이고 나머지는 봇 캐시로 답한다. 봇이 없으면 503 `BOT_UNAVAILABLE` | `node.ts` |
| GET/PATCH | `/api/nodes/:guildId/settings` | 노드/내부 | PATCH `{locale?: 'auto'\|태그, language?, sleeping?, musicEnabled?, musicUIEnabled?, musicSettings?: {queueLines 1-20, refreshInterval 2000-10000, normalize}}`(준 키만, `musicSettings`는 키별로 합친다) | `{guildId, locale, sleeping, musicEnabled, musicUIEnabled, musicSettings}`. 원본은 데몬 `nodeSettings[g].discord`(I2)이고 데몬만 보낸다. 디스코드의 `/언어`·`/코코낸내`·`/기상`·`/음악 기능\|플레이어`는 데몬 액션 `discord.settings.set`을 부른다 (server-chan #36). 확인한 뒤 노드가 다른 PC로 옮겨 갔으면 쓰지 않고 409 `NODE_MOVED` | `node.ts` |
| GET/PUT | `/api/nodes/:guildId/channels` | 노드/내부 | PUT `{chatbot?: string[](≤20), music?: string\|null, notify?: string\|null}` | `{guildId, chatbot, music, notify, botAvailable, channels:[{id,name,category}]}`. `notify`는 알림 채널(I3, server-chan #39). 400 `INVALID_BODY`·`CHANNEL_CONFLICT`·`UNKNOWN_CHANNEL`(`channelId` 포함), 배정할 때 봇이 없으면 503 `BOT_UNAVAILABLE`, 확인한 뒤 노드가 다른 PC로 옮겨 갔으면 409 `NODE_MOVED`. 한 채널은 한 종류만, 챗봇은 배정 채널에서만 답하고 음악 채널의 일반 메시지는 지운다 (server-chan PR #23) | `node.ts` |
| GET/PUT/DELETE | `/api/nodes/:guildId/chatbot` | 노드/내부 | PUT 설정 일부, `apiKey`: 문자열=교체, null=삭제, 생략=유지(128KB) | 설정 + `hasApiKey`(키 자체는 돌려주지 않는다) + `effectiveModel`(고른 모델을 쓸 수 없어 대신 쓰는 모델, 대체가 없으면 `model`과 같다). `triggerWords`·`sessionSeconds`·`ambientSeconds`는 받아도 무시한다(후속 대화 180초, 끼어들기 없음으로 고정). PUT·DELETE는 확인한 뒤 노드가 다른 PC로 옮겨 갔으면 쓰지 않고 409 `NODE_MOVED` | `chatbot/routes.ts` |
| GET | `/api/nodes/:guildId/chatbot/models` | 노드/내부(10/min) | — | `{provider, models, selected, effectiveModel}`: 길드에 저장된 사용자 키로 받은 대화 모델 목록(이름순, 10분 캐시). 409 `NO_API_KEY`, 422 `INVALID_API_KEY`·`QUOTA_EXCEEDED`, 503 `PROVIDER_UNAVAILABLE` | `chatbot/routes.ts` |
| DELETE | `/api/nodes/:guildId/chatbot/memory` | 노드/내부 | — | 이 길드의 장기 기억 삭제 | `chatbot/routes.ts` |
| GET | `/poll` | 노드(60/30s) | — | `{commands:[{id, payload, requestedBy, guildId, channelId}]}`(10건까지) / 204 / 401 `TOKEN_REPLACED`(대기 중 재페어링) | `routes/poll.ts` |
| POST | `/ack` | 노드 | `{ids: string[]}`(1~50개) | `{acked}`: 이 호스트에 전달된(`delivered`) 명령만 `acked`로 바꾼 수. 모르는 id·남의 id는 세지 않는다. 받으면 이 호스트에 다시 보내기를 켠다(heartbeat `agentVersion` 0.1.7 이상으로도 켠다, §4.4 (b) 3) | `routes/ack.ts` |
| POST | `/result/:requestId` | 노드 | `{success:boolean, data}` | `{status:'ok'}` / 404 / 409. `pending`·`delivered`·`acked`·`timeout` 상태에서 받는다 | `routes/result.ts` |
| POST | `/heartbeat` | 노드(4/min), 5MB | `{agentVersion?, os?, metadata?}`(§4.4 (c)) | `{status:'ok'}`. `agentVersion`이 `minAgentVersion`보다 낮으면(최소 버전을 `0.1.0`보다 올렸을 때는 `agentVersion`이 없어도) 426 `{error:'AGENT_TOO_OLD', minVersion, message}`이고 호스트를 오프라인으로 두며 `agentVersion`만 남긴다(`/사바쨩 상태`가 업데이트 안내를 보인다). 그 뒤 같은 노드의 다른 노드 인증 요청(poll·ack·result·events·link-requests)도 다음 heartbeat가 통과할 때까지 같은 426이다(`auth.ts`). `/info`·`/v1/updates/*`는 노드 인증을 쓰지 않아 막히지 않는다. DB에 쓰지 못하면 503 `DB_UPDATE_FAILED`(데몬은 상태 코드로 연결 오류를 남긴다). `metadata.language`가 바뀌면 그 PC 길드의 답 언어 캐시를 바로 비운다 | `routes/heartbeat.ts` |
| POST | `/events` | 노드(60/min), 256KB | `{events:[{id, type, data}]}`(20건까지) | `{accepted}`. 같은 호스트의 같은 `id`·`data.at`은 한 번만 받는다. 길드마다 따로 보내고, 디스코드 전송이 실패하면 2초·10초 뒤 다시 해 본다. 데몬은 2xx를 받은 이벤트를 다시 보내지 않으므로 끝내 실패한 길드는 그 알림을 놓친다(로그만) | `routes/events.ts`, `bot/notify.ts` |
| GET | `/api/link-requests` | 노드 | — | `{requests:[{requestId, guildId, guildName, requestedBy, userName, expiresAt}]}`(이 호스트의 대기 요청, `expiresAt`은 ISO 8601 UTC). 코드 없는 연결(J1-b) | `linkRequests.ts` |
| POST | `/api/link-requests/:id` | 노드 | `{accept: boolean}` | 수락 `{ok:true, guildId, guildName}`, 거절 `{ok:true, declined:true}`. 남의 것·없음 404 `NOT_FOUND`, 만료 410 `EXPIRED`, 묶을 수 없으면 409 `GUILD_TAKEN`·`HOST_NOT_FOUND`. 한 번만 쓴다(묶다가 DB 오류로 500이면 요청이 남아 다시 답할 수 있다) | `linkRequests.ts` |
| GET | `/v1/updates/check` | — | `channel`(stable·beta·nightly), `platform`, `current`, `client_id` | `{upToDate:false, latest:{version, channel, isCritical, releasedAt, minSupported, manifestUrl, notes}}` / `{upToDate:true, latest?}` (최신이면 `latest`는 요청한 채널에서 지금 버전의 릴리스를 가리킨다 — 업데이터가 컴포넌트를 비교한다. 그 채널에 지금 버전이 없으면 `{upToDate:true}`) | `routes/updates.ts`, §1.4 |
| GET | `/v1/updates/manifest/:version` | — | `release?` | 서명된 manifest 원문 | `updates.ts` |
| POST | `/v1/updates/event` | —, 4KB | `{anonId, phase, hostId?, fromVersion?, toVersion?, errorCode?, platform?}` | 204(익명 업데이트 텔레메트리) | `updates.ts` |
| GET | `/v1/updates/download/installer` | — (브라우저, 게이트 예외) | 없음 | 302 → 가장 높은 stable(전원 롤아웃, yanked 제외, manifest `platform`이 없거나 `windows-x64`) manifest의 `components.installer.url`(https만). 없으면 302 → `/not-found.html`. `cache-control: no-store`. 웹사이트 다운로드 버튼이 쓴다 | `updates.ts` |
| POST | `/admin/releases` | 내부, 256KB | `{version, channel, signedManifest, notes?, rolloutPercent?, isCritical?, minSupported?, assets[]}` | 201 `{id, version, channel, assets}` / 409 `RELEASE_ALREADY_EXISTS` | `updates.ts` |
| PATCH | `/admin/releases/:id` | 내부 | `{rolloutPercent?, yanked?, isCritical?, notes?}` | 바뀐 릴리즈 | `updates.ts` |
| GET | `/admin/api/service` | 관리자 키 | — | `{bot, connections, hosts, nodes, db}` | `routes/admin.ts` |
| GET/POST | `/admin/api/broadcast` | 관리자 키(POST 5/10min) | POST `{message, deleteAfterSeconds?}` | 202 `{ok, job}` / 409 `BROADCAST_IN_PROGRESS` / 503 `BOT_OFFLINE` | `routes/announcement.ts` |

데몬이 명령을 넣는 HTTP 라우트는 없다. 큐 INSERT는 봇이 같은 프로세스에서 `services/commandSubmission.ts` `submitCommand`로 한다. 페어링 클레임도 봇의 `/사바쨩 연결`이 `services/pairing.ts` `claimPairingCode`를 직접 부른다.

### 4.3 DB 스키마 (`relay-server/src/db/schema.ts`, migration 0000–0022)

| 테이블 | 주요 컬럼 |
|---|---|
| `users` | `discord_id` PK, username, display_name, is_banned, created_at, last_seen |
| `hosts` | `id` PK(nanoid12), `owner_discord_id` **UNIQUE**(사용자 1명당 host 1개) → users, name, `token_hash`(argon2id), status(online/offline), last_heartbeat, agent_version, `metadata` jsonb(heartbeat 캐시와 botConfig. 서버 항목은 `id`·`name`·`module`·`status`만 저장, server-chan #40), timestamps |
| `nodes` | `guild_id` PK, `host_id` → hosts(cascade), owner_discord_id, guild_name, locale, `locale_source`(auto/manual), music_enabled, music_ui_enabled, music_settings jsonb, is_sleep, `default_level`(데몬 값의 캐시), `notify_channel_id`(I3), timestamps |
| `node_permissions` | PK(guild_id, discord_id), level(-1=금지, 0=챗봇, 1=유저, 2=관리자), granted_by. 데몬 `nodeSettings[guild].members`의 캐시(릴레이 바깥 검사용) |
| `command_queue` | id PK, host_id, guild_id, payload jsonb, requested_by, channel_id, status(pending→delivered→acked→completed / timeout / error), created/delivered/completed/expires_at(+60s), result jsonb, `redeliveries`(ack 없이 다시 보낸 횟수, `0022_command_redeliveries`) |
| `pairing_codes` | code PK, expires_at(+5분), status, attempts, relay_url, claimed_by, host_id, node_token(수령 기한이 지나면 NULL. 읽을 때 또는 정리 작업이 지운다), claimed_at, poll_secret_hash |
| `announcement_settings` | guild_id PK, channel_id, subscribed_at |
| `audit_logs` | serial id, timestamp, user_discord_id, host_id, guild_id, action, detail, result. action `relay`(큐에 넣은 명령: `requestId`, `kind`, 명령은 서버·명령 이름과 인자 개수만, 액션은 id·instance·입력 키(참·거짓·숫자만 값, 여러 단계 계획은 단계마다 id·입력 키), 결과는 `queued` → `/result`가 성공이면 `success`, 실패면 데몬 코드로 바꾼다. 늦은 결과는 7일 안의 행에만 적는다)·`confirm`(확인 버튼 실행·취소)·`assign`(채널 배정, 앱에서 바꾸면 user `app`)·권한·페어링. 인자 값과 비밀은 남기지 않는다(`services/audit.ts`) |
| `releases` / `release_assets` | 버전·채널(채널·버전 쌍은 유일, `0021_releases_unique`)·롤아웃·isCritical·minSupported·signed_manifest·notes·yanked / 컴포넌트·플랫폼·파일·크기·sha256 (§1.3) |
| `update_events` | host_id, anon_id, from/to_version, phase, error_code, platform |
| `chatbot_settings` / `chatbot_memory` / `chatbot_records` | 길드 챗봇 설정(키는 `api_key_enc`로 암호화) / 채널별 요약 기억 / 길드별 기록(kind·key·value·만료) |

정리 스케줄러(`services/cleanup.ts`): 감사 30일, 끝난 큐 7일, 업데이트 이벤트 90일, 페어링 코드 24시간 뒤 삭제. 수령 기한이 지난 페어링 토큰은 바로 지운다. 만료 큐는 timeout으로, heartbeat가 3분 넘게 없는 host는 offline으로 바꾼다.

### 4.4 프로토콜 흐름

**(a) 페어링** (GUI → daemon proxy → relay → Discord 슬래시)
1. GUI `relay:initiatePairing` → daemon `POST /api/relay/pair/initiate` (`src/ipc/handlers/relay.rs`) → relay `POST /api/pair/initiate`. 응답은 `{code, pollSecret}`이다. 데몬은 https 릴레이(모의 릴레이는 루프백 http)와만 페어링한다. 아니면 relay에 보내지 않고 400 `invalid_relay_url`이다(`pairing_relay_url`, 상태 폴링도 같다).
2. 사용자가 Discord에서 `/사바쨩 연결 코드:<code>`를 실행한다 → relay 봇이 같은 프로세스에서 `claimPairingCode`를 부른다(`bot/commands/coreService.ts`, `services/pairing.ts`). users upsert, hosts 생성 또는 토큰 재발급(계정당 PC 하나. 3분 안에 heartbeat가 있던 PC를 대체하면 답에 그 사실을 붙인다), guild가 있으면 nodes 자동 생성, `pairing_codes.node_token`에 저장(수령 기한이 지나면 지운다). 길드가 이미 다른 사람(노드 주인이 아닌 사람)에게 묶여 있으면, 연결하는 사람이 디스코드 서버 주인일 때만 그 노드를 지우고 새로 만든다(옛 PC의 대기 명령도 지운다). 아니면 `GUILD_TAKEN`으로 거절하고 코드는 pending으로 남는다. `/사바쨩 연결해제`(노드 주인 또는 디스코드 서버 주인. 노드 주인은 잠근 노드 행으로 다시 확인한다)는 노드와 그 길드의 대기 명령을 지운다. 권한·배정·챗봇 설정은 cascade로 지워지고, 길드 슬래시 명령은 0개로 다시 등록된다(`unbindGuild`). 데몬의 `nodeSettings`는 남는다.
   - 코드 없이 `/사바쨩 연결`(서버 관리 권한, 길드에서만)을 치면 친 사람의 켜진 PC(3분 안 heartbeat)에 수락을 묻는다(J1-b). PC가 없으면 코드 안내, 이미 내 PC에 묶인 길드면 그렇다고 답하고, 남의 PC에 묶인 길드는 위와 같은 규칙이다. 요청(`requestId`, 10분)은 relay 메모리에 두고(재시작하면 사라진다, `services/linkRequests.ts`) 그 호스트 큐에 `{action:'link_request', requestId, guildId, guildName, userName, expiresAt}`을 넣는다(노드가 아직 없어 큐 항목 `guildId`는 비우고 payload에 싣는다). 데몬은 이것을 `relay.link_request` 이벤트로 GUI에 보이고, 답은 `POST /api/link-requests/:id`다. 수락하면 페어링의 노드 만들기와 같은 검사로 길드를 묶고(남의 PC에 묶인 길드를 옮길 서버 주인 여부는 수락할 때 봇 캐시로 다시 본다. 봇이 그 길드를 모르면 옮기지 않는다)(토큰은 새로 만들지 않는다, 감사 `link_accept`, `pairing.ts` `linkGuildToHost`) 요청한 채널에 짧게 알린다.
3. GUI가 daemon `GET /api/relay/pair/:code/status?secret=…`를 폴링한다(`relay.rs` `poll_pairing_status`). daemon은 relay에 `X-Poll-Secret` 헤더로 묻는다(#61). relay가 `claimed`와 함께 nodeToken을 돌려주면(응답을 잃어도 수령 기한 안의 다음 폴링이 다시 받는다) **daemon이 `<data_dir>/.node_token`에 저장**하고 응답에 `tokenSaved`를 더한다(같은 토큰이면 다시 쓰지 않는다). `tokenSaved:false`(토큰 `null`이나 쓰기 실패)면 GUI·CLI는 연결됨으로 보이지 않는다.
4. daemon `GET/PUT/DELETE /api/config/node-token`(`handlers/config.rs`)은 토큰 원문을 IPC로 그대로 반환한다.

**(b) 명령 실행** (cloud 모드)
1. Discord 사용자가 `<prefix> <text>`를 입력한다. 접두사는 길드별 캐시(heartbeat `botConfig.prefixes`, 옛 데몬은 `prefix`)이고, 비어 있으면(옛 `"!saba"`도) 응답 언어의 기본값(ko `사바쨩`, ja `サバちゃん`, 그 밖 `Saba-chan`)이다(`bot/state/nodeState.ts`). 챗봇 채널에서는 접두사 뒤가 서버 명령이나 데몬 최상위 명령(help·list 등)이 아니면 챗봇이 받는다. 길드 슬래시 명령(`/<서버> <명령>`)과 챗봇 `run_command`도 같은 경로로 간다.
2. `dispatchCommand`·`sendToDaemon`(`bot/commands/dispatch.ts`)이 requestId를 먼저 만들고 `resultWaiters.wait(id, 55s)`를 등록한 뒤(실행 중인 명령은 더 기다린다, 아래 3) `submitCommand`(`services/commandSubmission.ts`)로 큐에 넣는다. payload는 `{action:'raw_command', text, via:'direct'|'chatbot', voiceChannelId?, userName?, guildName?, locale?}`이고(`locale`은 답 문구를 쓸 디스코드 길드 언어, 없으면 데몬 GUI 언어), 큐 항목에는 노드 주인 `ownerDiscordId`도 실린다(P1). 데몬은 처음 받은 주인을 고정하고, 다른 값은 무시한다(`relay_agent.rs` `requester`, `chat::permissions::pin_owner`). `ownerDiscordId`가 있는데 `via`가 없으면 챗봇으로 본다. 두 이름은 감사 로그에 남길 디스코드 표시 이름·서버 이름(한 줄, 100자까지)이고, 권한 판단에는 쓰지 않는다(권한은 `requestedBy`·`guildId` id로만). `guildId`로 nodes를 조회해 hostId를 찾고, 큐에 INSERT한 뒤 `pollWaiters.wake(hostId)`를 호출한다.
3. daemon의 relay agent(`src/relay_agent.rs` `poll_loop`)가 `GET /poll`을 계속 돌리고 있다. 서버는 즉시 가장 오래된 pending 1건을 꺼내 `delivered`로 표시하고 반환한다(`poll.ts` `CLAIM_PER_POLL`). 데몬은 빈 실행 자리가 있을 때만 poll 하고 받은 명령을 시작하며 ack하므로, 한 번에 여러 건을 주면 자리가 모자란 명령이 ack 없이 데몬에 묶여 다시 보내기·시간 초과와 엇갈린다. 없으면 **25초 대기**(`poll.ts:6`, `pollWaiters.wait`) 후 다시 조회하고, 그래도 없으면 204를 준다. 클라이언트 연결이 끊기면 abort한다. 꺼낸 뒤 연결이 끊겼거나 응답을 끝까지 쓰지 못하면 그 명령을 `pending`으로 되돌린다(봇이 이미 시간 초과로 알려 다시 보내기 상한에 닿은 명령은 되돌리지 않는다). 응답을 다 쓴 뒤 데몬이 받지 못한 경우는 ack로 살린다: `POST /ack`를 한 번이라도 보냈거나 heartbeat의 `agentVersion`이 0.1.7 이상인(id로 중복을 거르는) 호스트에서, 전달된 뒤 15초 동안 ack도 결과도 없는 명령을 5초마다 훑어 `pending`으로 되돌리고 그 호스트의 poll을 깨운다. 한 명령은 2번까지 다시 보내고, 만료(60초)가 지나면 보내지 않는다(`services/redelivery.ts`, `routes/ack.ts`). 이 호스트 목록은 메모리에만 있어 릴레이를 다시 켜면 다음 heartbeat·ack 때 다시 켜진다. 재페어링하거나 heartbeat의 `agentVersion`이 0.1.7 미만이면 그 호스트를 목록에서 뺀다(재페어링은 hostId를 그대로 쓴다). 그 뒤 0.1.7 이상의 heartbeat나 ack가 다시 켠다. ack를 보내지 않는 예전 데몬에는 다시 보내지 않는다(id로 중복을 거르지 않아 두 번 실행될 수 있다). 봇은 결과를 55초 기다린다. 그때 아직 `pending`인 명령은 `timeout`으로 내려 늦게 전달되지 않게 하고 시간 초과로 답한다. 이미 전달된 명령은 다시 보내지 않는다. 그중 데몬이 실행을 시작했다고 알린(`acked`) 명령은 첫 서버 시작처럼 1분을 넘길 수 있어, 처음부터 10분까지 결과를 더 기다린다(디스코드 상호작용 토큰 15분 안). 한 상호작용에서 plan과 run_plan을 이어 보내는 슬래시·버튼 경로(`actionCommands.ts` `runCommandAction`, 확인 버튼)는 상호작용 시각 + 14분을 `deadline`으로 넘긴다. 연장 대기만 그 시각에서 멈추고 첫 55초는 그대로라, 최악이어도 상호작용 시각 + 14분 55초 안에 답한다. 더 기다리는 동안 15초마다 그 PC와의 연결을 보고, 호스트가 offline이거나 heartbeat가 3분 넘게 없거나 데몬 poll과 heartbeat가 함께 150초 넘게 없으면(heartbeat 한 번 실패는 끊김으로 보지 않는다)(실행 자리가 차면 poll만 멈추므로 poll 공백만으로는 판단하지 않는다. poll 시각은 릴레이 메모리에 있어, 다시 켠 뒤 poll을 아직 못 봤으면 이 기준은 건너뛴다) 기다림을 멈추고 연결이 끊겼다고 답한다(`HOST_LOST`, `cmd_host_lost`). 대기가 끝나는 순간 큐에 저장된 결과가 있으면 시간 초과 대신 그 결과로 답한다(`dispatch.ts` `sendToDaemon`). 같은 계정의 재페어링으로 토큰이 바뀌면(`hostReplaced`) 기다리던 poll을 깨워 401 `TOKEN_REPLACED`로 끝내고, 예전 토큰으로는 명령을 꺼내지 않는다. 잠자기는 길드 단위이고 릴레이가 그 길드의 명령을 받지 않는 것으로 처리한다(데몬 폴링은 멈추지 않는다). agent는 받은 명령을 4건까지 동시에 실행하고, 빈자리가 있는 동안에는 다음 poll을 계속 보낸다(`relay_agent.rs` `COMMANDS_IN_FLIGHT`). 4건이 모두 차거나 받아 두고 시작하지 못한 명령이 남아 있으면 poll을 보내지 않는다(heartbeat는 계속 보낸다). 그래서 오래 걸리는 명령 하나가 뒤 명령의 답을 막지 않고, 결과는 끝난 순서대로 `POST /result/:id`로 간다. 4건이 차면 새 명령은 빈자리가 날 때까지 기다린다. agent는 명령을 실행하기 시작할 때 그 id들로 `POST /ack {ids}`(50개씩)를 보내고, 릴레이가 404면(예전 릴레이) 그 세션에서는 보내지 않는다. 받은 명령 id는 10분 동안 1,000개까지 기억해서, 같은 id가 다시 오면 실행하지 않는다: 아직 처리 중(대기 포함)이면 그대로 두고, 끝난 명령이면 기억해 둔 결과를 `/result/:id`로 다시 보낸다. 실행 중인 명령이 다시 오면(ack가 닿지 않았다) ack를 다시 보내고, 대기 중인 명령은 시작할 때 보낸다(`relay_agent.rs` `SeenCommands`, `post_ack`). 기억은 에이전트 메모리에 있어 다시 연결해도 이어지고(데몬을 다시 켜면 사라진다), 세션이 끝나면 받아 두고 시작하지 않은(ack하지 않은) 명령은 바로 잊어서 릴레이가 다음 연결로 다시 보내면 실행한다. 이미 시작한 명령은 세션이 끝나도 끝까지 실행하고 결과를 보낸다.
4. agent는 받은 `raw_command`마다 `crate::chat::api::run_with_daemon_settings`를 같은 프로세스에서 실행한다(`relay_agent.rs:509-530`). 접두사·별명·길드별 설정은 daemon의 bot config에서 읽는다(`chat/api.rs` `run_with_daemon_settings`). 답 문구는 payload `locale`, 없으면 GUI 언어로 쓴다. 답 텍스트를 `{success, data:{text}}`로 담아 `POST /result/:id`로 보낸다. 입력을 알아보지 못했으면(인스턴스를 못 찾음, 정의에 없는 명령, 모르는 낱말) `{success:false, data:{code:"INSTANCE_NOT_FOUND"|"UNKNOWN_COMMAND", error:<현지화 문구>, args:{name?, command?, help?}}}`로 보내고(정의에 없는 명령이면 `help`에 그 모듈의 명령 안내를 담아 릴레이가 덧붙인다), 릴레이는 이 코드를 보고 모델 해석기에 정확한 명령을 한 번 묻는다(`relay_agent.rs` `raw_command_result`, `chat::Reply`). 모듈 이름만 쓰면 실패가 아니라 명령 안내다. 권한이 모자라면 `code:"PERMISSION_DENIED"`, `args:{need, level, via?}`다. 챗봇 경로(`via:'chatbot'`)에서 모듈이 `risk="danger"`로 선언한 명령(모르는 risk 값도 danger로 본다, `commands::risk_of`)은 요청자 본인이 확인한 것만 실행한다: 릴레이는 챗봇이 올린 확인 버튼을 요청자(`requestedBy`)가 눌렀을 때만 payload에 `confirmedBy:<누른 사람 Discord id>`를 싣고 같은 `raw_command`를 다시 보낸다. 데몬은 `confirmedBy`가 `requestedBy`와 같을 때만 통과시키고(`Requester::confirmed_by_requester`), 레벨·`chatbotCap` 판정은 그대로 한다. 없거나 다르면 `code:"action_not_allowed"`, `args:{command}`로 거절한다(`chat::Ctx::module_command`). 액션 경로(`server.command:*`)는 danger 명령도 챗봇 표면에 열려 있고 늘 확인 계획(`needsConfirm`)을 거친다(사용자 결정 2026-10-20). 권한·비밀값·`update.apply` 같은 챗봇 금지 액션은 그대로다. 챗봇 경로의 답은 로컬 정보를 뺀 뒤 IP와 비밀처럼 보이는 값도 가린다(`actions::mask_console`, `relay_agent.rs` `raw_command_result`). 액션 경로도 콘솔·모듈 조회·모듈 명령(`server.command:*`)의 결과를 챗봇에 같이 가린다(`actions::chatbot_masked`). 시작이 모듈 약관 동의를 기다리면 `code:"AGREEMENT_REQUIRED"`, `args:{agreements:[{id, label, url, accepted}]}`다(동의는 PC에서만 받는다, `chat::start`). 모르는 action은 `{success:false, data:{error:"Unknown action: <action>"}}`다(릴레이가 이 문구로 옛 데몬을 알아본다).
   - `{action:'plan'|'run_plan'|'query', id, input?, instance?, planId?, locale?, via, userName?, guildName?}` (P2·P3, `relay_agent.rs` `action_request`): 액션 카탈로그. `plan`에 `steps:[{id, instance?, input}]`가 있으면 여러 단계 계획(I1, 01 `POST /api/actions/plan`)이고 `run_plan`은 같다. 그 실행 결과가 `running:true`이면 끝은 이벤트 `plan.done`(`runId`)으로 온다. 원격 설정 변경의 실행 결과에는 되돌리기 `undoPlanId`가 있고 `run_plan`으로 보낸다. 관리 실행 횟수를 넘으면 `rate_limited {args:{perHour, retryInSecs}}`. 판정·검증은 데몬이 한다(`crate::actions`). `via:'chatbot'`이면 액션 선언의 `surfaces`에 chatbot이 없을 때 레벨과 상관없이 `action_not_allowed`다. 확인이 필요한 액션(danger·관리)은 `plan` → `run_plan`만 받고, 바로 부르면 `confirm_required`다. `planId`는 만든 `guildId`·`requestedBy`에 묶여 10분 살고(`plan_not_yours`·`plan_expired`), 챗봇이 만든 계획은 확인한 사람이 보내도 챗봇 상한으로 판정한다. `query`는 `kind=query` 액션(모듈 조회 `server.query:<이름>` 포함)만 받는다(`not_a_query`). 원격 요청에는 세 가지를 더 지킨다: 결과에서 비밀값(`*_password`·`*_token`·`*_secret`, 모듈 password 형식 필드)을 어느 깊이에서든 지운다. 결과의 PC 로컬 정보도 어느 깊이에서든 뺀다(I3, 알림 push와 같은 기준, `actions::strip_local_details`): `working_dir`·`executable_path`·`extra_args`·`extension_data`·`install_path` 키, 종료 근거 `evidence` 중 종료 코드·신호·메모리 상한이 아닌 것(콘솔 줄·크래시 파일 경로), `note_args.reason`(자동 재시작 실패 원문). 문자열 안의 절대 경로는 `<path>`로 바꾸고(따옴표로 싼 경로는 닫는 따옴표까지, 아니면 공백이 있을 수 있어 그 줄 끝까지), Python traceback은 마지막 줄만 남긴다. `instance.settings.set`·`instance.create` 입력은 비밀값과 실행 관련 키(`executable_path`·`working_dir`·`instance_dir`·`extra_args`·`extension_data`·`rest_host`·`memory_limit_mb`·`java_path`(모듈이 어떤 형식으로 선언하든), 모듈의 file 형식·path 형식 필드, 형식·정규식 없이 실행 인자가 되는 text 필드)를 어느 깊이에서든 `invalid_input`으로 거부한다. 길드에 `allowedInstances`가 있으면 그 밖의 인스턴스는 `NODE_NOT_ALLOWED`(`args.instance`)이고, 결과의 `servers`·`instances` 목록(어느 깊이든, 원소의 `id`. `instance.list`·`monitor.summary` 등)도 그 목록으로 거른다. 원격 실행 결과가 성공이면 데몬은 응답 최상위 `text`에 요청 `locale`의 사람용 요약(머리 줄 + 항목 10줄까지 + 남은 개수)을 싣는다: `instance.list`·`backup.list`(새것부터, 종류·시각·크기)·`instance.events`(새것부터)·`update.check`(업데이트가 있는 구성요소, 확인 실패면 없음)·`monitor.summary`(CPU·메모리, 실행 중 서버별), 모듈 조회 `server.query:*`(채팅 명령 결과와 같은 범용 포맷 `chat::format::format_response`). 시각은 PC 시간대다(모르면 `?`). 그 밖의 액션은 `text`가 없다(`actions/summary.rs`, 문구는 봇 locales `actions.summary.*`). 성공은 `{success, data:<plan·run 응답>}`(응답에 `text`(선택, 요청 `locale`로 쓴 사람용 요약)가 있으면 relay는 결과 JSON 대신 그 문구를 보인다, `actionResult.ts` `resultText`), 실패는 `{success:false, data:{code, error, args?, field?, reason?}}`. 문구는 `locale`로 만든다. 약관 동의 계약(데몬 쪽은 구현 중)에서 동의가 필요하면 `code:"agreement_required"`와 `data.agreements:[{id,label,url}]`(또는 `args.agreements`)를 싣기로 했다. relay는 동의를 대신 보내지 않고 "PC의 사바쨩에서 <이름>에 동의해야 켤 수 있어요"와 https 원문 링크를 보인다(`raw_command` 결과도 같다, `dispatch.ts` `agreementText`). `run_plan`이 밑 라우트의 409를 `data.result.error_code`로 감싸 와도 같다(`actions.ts`).
   - relay 쪽 결과 처리(P2 후속 계약, `bot/commands/actionResult.ts`): 여러 단계 계획 `{action:'plan', steps:[{id, instance?, input}]}`(1~8단계, `instance:"@N"`은 N번째 단계가 만들 서버)의 결과는 단계마다 ✅·❌(코드 문구)·⏭️ 한 줄로 보인다. `running:true`면 ⏳ 메시지를 보내고 `runId` → 그 메시지를 메모리에 1시간 들고 있다가 `plan.done` 이벤트가 오면 고친다. 들고 있지 않으면(재시작) 그 길드 알림 채널에 되돌리기 없이 보내고, 이 호스트에 묶인 길드가 아니면 버린다. 결과에 `undoPlanId`(한 액션) 또는 단계의 `undoPlanId`가 있으면 [되돌리기] 버튼을 붙이고, 요청한 사람만 누를 수 있으며 누르면 그 사람으로 `run_plan {planId: undoPlanId}`를 보내고 그 버튼 줄만 걷는다. 계획 요약의 `before`는 "이전 → 새 값"(`null`은 "없음")으로 보인다. `rate_limited`(`args.perHour`·`retryInSecs`)·`plan_running`·`provision_failed`·`provision_timeout`·`step_skipped`는 번역 문구가 있고, 틀린 단계의 `step`은 "N단계:"로 붙인다. 챗봇 도구 `plan_actions {steps:[{action, server, input}]}`(조회 단계는 받지 않는다)는 데몬 계획을 하나 만들고 늘 확인 버튼으로 돌린다(`chatbot/tools.ts`).
   - `{action:'link_request', requestId, guildName?, userName?, expiresAt?}` (J1-b): 이미 페어링한 사람이 다른 길드에서 코드 없이 `/사바쨩 연결`을 쳤을 때 relay가 그 사람의 PC로 보낸다. 데몬은 이벤트 `relay.link_request`를 내서 GUI·CLI에 알리기만 하고 `{success:true, data:{notified:true}}`로 답한다(`requestId`·`guildId`가 없으면 `success:false`). 수락·거절은 PC에서 사람이 `POST /api/relay/link-requests/:id`로 한다(01 Relay 프록시 표).
   - `{action:'permission', op:'grant'|'revoke'|'default_level', discordId?, level?, via}`: 디스코드 `/사바쨩 권한` 명령. 고정된 주인이 직접(`via:'direct'`) 보낸 것만 받아 `nodeSettings`에 쓰고 `{success:true, data:{defaultLevel, members, chatbotCap, levels, commandLevels, permissionsAt}}`(heartbeat `permissions[guild]`와 같은 모양)를 돌려준다(`permissionsAt`은 저장을 마친 직후의 PC 시각 Unix ms). 아니면 `data.code` `NOT_OWNER`·`OWNER_FIXED`·`INVALID_LEVEL`·`UNKNOWN_OP`·`MISSING_GUILD`.
5. relay는 상태를 completed로 바꾸고 `resultWaiters.wake(id)`를 호출한다. bot은 `result.data.text`(최대 1950자)로 Discord에 reply한다.
6. agent의 재시도 백오프는 3초에서 시작해 최대 60초다(`relay_agent.rs:25-26`).
7. 알림 채널(I3): agent가 데몬 이벤트 버스 중 `instance.exit`(크래시·시작 실패·원인 모를 종료), `backup.done`, `update.available`, `schedule.warn`, `plan.done`(뒤에서 마친 여러 단계 실행, 요청한 `guildId`가 있다)을 `POST /events {events:[{id, type, data}]}`로 올린다(노드 서명, 20건씩). 경로·콘솔 줄·오류 원문은 데몬이 뺀다. relay가 404면 10분 쉬고 다시 알아본다. relay는 노드당 분당 60건까지 받고(넘으면 429, agent는 백오프 뒤 다시 보낸다), 잠든 길드에는 보내지 않는다. 배정 종류 `알림`과 길드별 전달은 relay가 한다. 같은 경로의 `plan.done`은 알림이 아니라 그 실행의 결과 메시지를 고친다(위 relay 쪽 결과 처리).

**(c) Heartbeat**: 60초마다 `POST /heartbeat`(`relay_agent.rs:271-279`). metadata = `{servers, modules, moduleDetails:{}, botConfig:{prefixes, prefix?, moduleAliases, commandAliases}, allowedInstances?:{<guildId>:[instance id]}, instanceWords?:{<guildId>:{<instanceId>:{words, commands:{<명령>:[..]}}}}, permissions:{<guildId>:{defaultLevel, members, chatbotCap, levels:{safe, caution, danger, admin}, commandLevels:{<키>:<레벨>}, commandLevelsTruncated?}}, permissionsAt, actions:[{id, kind, risk, surfaces, scope, level?, title:{<locale>:…}, input}], actionsTruncated?, language?, onboarding}`이고(`language`는 GUI 언어, 10자까지. 언어가 `auto`인 길드가 따른다)(`onboarding`은 `GET /api/onboarding`과 같은 `{steps, next}`로 `/사바쨩 상태` 끝 줄에 다음 할 일을 보이는 데 쓴다, J1-a)(`actions`는 디스코드·챗봇 표면의 액션만, 64개·32768바이트까지, 넘으면 `actionsTruncated:true`. 전용 명령이 같은 액션을 부르는 것(`Def.remote_menu:false`: 길드 설정 `chatbot.settings.*`·`discord.settings.*`는 `/사바쨩`)은 싣지 않고, `input`에는 원격이면 데몬이 채우는 `guild` 칸을 두지 않는다. 데몬은 `input.properties.<칸>.title`에 `{<locale>: …}` 칸 제목을 싣는다(고정 액션은 봇 locales `actions.field.<칸>`, 모듈 조회는 모듈 locales `queries.<이름>.inputs.<칸>.label` → module.toml 입력 `label`. 문구가 없는 언어는 빠지고, 하나도 없으면 `title`이 없다. 32768바이트를 넘으면 칸 제목부터 모두 뺀다, `with_field_titles`). 고정 액션 뒤에 모듈 조회 `server.query:<이름>`이 모듈·이름 순으로 붙는다: `{id, kind:'query', risk:'safe', surfaces:['discord','chatbot'], scope:'instance', module, title:{<locale>:…}, description?:{<locale>:…}, input}`, `module`은 그 조회를 가진 모듈 이름이다. relay는 `module`이 있는 항목을 그 모듈의 서버에서만 보이고 부르며, 같은 `id`를 여러 모듈이 내면 하나로 합친다, `catalog.ts` `actionServers`)(relay는 디스코드 표면의 `scope:'instance'` 액션을 각 서버 슬래시 루트 `/<서버>` 아래 하위 명령으로 만든다. 기본 명령 start·stop·status와 같은 `server.start`·`server.stop`·`server.status`는 따로 만들지 않고 그 제목을 기본 명령의 언어별 이름으로 쓴다. 서버를 받지 않는 액션은 슬래시 명령으로 만들지 않는다(업데이트 확인·적용은 PC 앱·CLI에서만, 사용자 결정 2026-10-20), `guildCommands.ts`·`actionCommands.ts` `serverActions`. 입력 스키마 `properties.<필드>.title`(선택, 문자열 또는 `{<locale>:…}`)이 있으면 옵션 설명과 언어별 옵션 이름으로 쓴다)(`allowedInstances`는 `bot.node_settings[guild].allowedInstances` 목록이 있는 길드만, 없으면 제한 없음. `instanceWords`는 그 길드에서 서버·명령을 부르는 낱말이 있는 길드만 싣는다(길드 50, 길드당 서버 50까지, `chat::words::heartbeat`). 데몬이 원문을 해석하므로 relay는 이 낱말로 대상 서버와 명령을 찾아 자동완성·위험 확인(정식 명령 이름 기준)에 쓰면 된다. `permissions`는 `nodeSettings`에 있는 모든 길드(값이 없으면 `{defaultLevel:1, members:{}, chatbotCap:0, levels:{safe:0, caution:1, danger:2, admin:2}, commandLevels:{}}`)이고, 릴레이는 이 값으로 자기 캐시의 데몬 행을 맞춘다. `chatbotCap`·`commandLevels`는 `NodePermissions` 값 그대로이고(`commandLevels` 키는 `<instanceId>/<명령·액션 id>` 또는 전역 액션 id), `levels`는 그 길드 프리셋으로 푼 risk별 요구 레벨(`chat::permissions::preset_level`)과 관리 액션의 요구 레벨 `admin`(`actions::admin_level`, strict면 3)이라 릴레이가 프리셋 표를 따로 들지 않는다. `commandLevels`는 길드당 키 순으로 200개까지 싣고, 넘으면 `commandLevelsTruncated:true`다(그 길드는 릴레이가 사전 검사를 하지 말고 데몬 판정에 맡긴다). 릴레이 사전 검사는 `actions::authorize`와 같은 순서로 요구 레벨을 센다: 모듈 명령(`raw_command`)은 `commandLevels[<instanceId>/<명령>]` → 내장 명령(`start|stop|restart|status`)이면 `commandLevels[<instanceId>/server.<명령>]` → `levels[risk]`(데몬 `NodePermissions::required`와 같다, `chatbot/tools.ts` `requiredLevel`). 액션은 항목의 `level`(고정 레벨, 있으면 끝) → 내장 서버 액션(`server.start|stop|restart|status`)이면 `commandLevels[<instanceId>/<start|stop|restart|status>]` → `commandLevels[<instanceId>/<액션 id>]`(인스턴스가 없는 액션은 `commandLevels[<액션 id>]`) → `kind`가 `admin`이면 `levels.admin`, 아니면 `levels[risk]`. 실효 레벨은 `min(주인이면 3, 아니면 members[id] ?? defaultLevel, 챗봇 경로면 chatbotCap)`이다. 예전 금지 목록 `memberPermissions`(GUI에서 사람별로 막은 명령, 데몬은 주인 레벨을 요구한다)는 싣지 않으므로 사전 검사는 예측일 뿐이고, 통과해도 데몬이 거절할 수 있다. `levels.admin`이나 `chatbotCap`이 없는 heartbeat(예전 데몬)는 `commandLevelsTruncated`처럼 사전 검사를 건너뛰고 데몬 판정에 맡긴다. `actions` 항목의 `level`은 고정 요구 레벨이 있는 액션(`Def.discord_level`, 예: `update.apply` 3)에만 있다. heartbeat `actions`에 없는 액션(원격 메뉴에 싣지 않는 `chatbot.settings.*`·`discord.settings.set` 등 고정 레벨이 주인인 것)은 사전 검사를 하지 않는다. 사전 검사와 상관없이 데몬이 받은 `raw_command`·`action`을 같은 규칙으로 다시 판정한다(`chat::permissions` `check`, `actions::authorize`). `permission` action 응답 `data`도 같은 모양에 `permissionsAt`을 더한다. `permissionsAt`은 권한을 읽기 직전의 PC 시각 Unix ms다. `permission` action 응답의 `permissionsAt`과 같은 PC 시계라, 늦게 온 heartbeat가 방금 바꾼 레벨을 되돌리지 않게 한다. 릴레이는 `hostId:guildId`별로 마지막에 반영한 `permissionsAt`을 메모리에 둔다. heartbeat `permissions`의 permissionsAt이 그 값보다 작거나 같고 차이가 5분 안쪽이면, 그 길드만 건너뛴다. 5분보다 오래된 값은 시계 보정으로 보고 받는다(`permissions.ts` `claimApply`). permission 응답보다 늦게 읽은 heartbeat를 이미 반영했으면 응답 스냅샷으로 다시 덮지 않는다. 값이 없으면(옛 데몬) 그대로 반영한다), `modules[]`는 `{name, module_aliases, commands:{fields:[{name, label, description, risk, aliases?, inputs:[{name, label, type, required, min, max, options}]}]}, presets:[{id, label}]}`만(콘솔 템플릿·REST 엔드포인트는 싣지 않는다, `relay_command_entry`. `aliases`는 module.toml `[aliases.commands]`에 그 명령의 별칭이 있을 때만 싣는 문자열 목록으로, relay는 별칭으로 친 명령도 같은 위험도 확인을 거친다), 서버 목록의 각 줄은 relay가 읽는 `{id, name, module, status, usage?:{cpu, memMb}, players?:{online, max?}, statusLine?}`만 싣는다(설정값·경로·비밀번호는 싣지 않는다, `relay_agent.rs` `relay_server_entry`). `usage`는 실행 중일 때 하트비트마다 뜬 표본(`cpu`는 기계 전체 기준 %, 소수 한 자리), `players`는 실행 중이고 모듈이 `[status] players`로 알려 준 마지막 접속자 수(02 §1.2, 최대를 모르면 `max`가 없다), `statusLine`은 `handled:true`로 답한 익스텐션 `server.list_enrich` 결과의 `status_line`(제어 문자를 빼고 100자까지)이다(Q-5·X-1). 시작할 때 `GET /info`로 `minAgentVersion`을 비교한다(`relay_agent.rs:251-`).

**(d) Sleep/Wake**: 원본은 데몬 `nodeSettings[g].discord.sleeping`이다. 슬래시 `/사바쨩 코코낸내`/`기상`은 데몬 액션 `discord.settings.set`을 부르고, 데몬이 `PATCH /api/nodes/:g/settings {sleeping}`으로 relay의 `nodes.is_sleep`과 메모리 `_pausedGuilds`를 맞춘다(`bot/commands/discordSettings.ts` `applyNodeSettings`). 잠든 길드는 `/사바쨩` 하위 명령을 뺀 명령과 알림을 받지 않는다. 메시지는 모두 무시하고, 상호작용은 `/사바쨩` 하위 명령만 받으며 길드 서버 명령·버튼·모달에는 잠들어 있다는 안내만 보인다(`bot/gateway/gateway.ts`). 옛 데몬(액션을 모름)이면 relay에만 쓴다.

### 4.5 daemon 쪽 호출과 relay 서버 비교 (검증 결과)

| daemon 라우트 | relay 호출 | 인증 | 일치 |
|---|---|---|---|
| GET `/api/relay/host/:id/status` | GET `/api/hosts/:id` | 노드 토큰 | ✓ |
| GET `/api/relay/host/:id/nodes` | GET `/api/hosts/:id/nodes` | 노드 토큰 | ✓ |
| GET `/api/relay/node/:guild/members` | GET `/api/nodes/:g/discord-members`, 503이면 `/api/nodes/:g/members` | 노드 토큰 | ✓ |
| POST `/api/relay/pair/initiate` | POST `/api/pair/initiate` | 없음 | ✓ |
| GET `/api/relay/pair/:code/status` | GET `/api/pair/:code/status` | `X-Poll-Secret` | ✓. GUI가 데몬에 주는 `?secret=`을 데몬이 헤더로 옮겨 보낸다(saba-chan #61). relay는 server-chan c3a3425부터 헤더만 받는다 |
| GET `/api/relay/bot-invite` | GET `/api/bot-invite` | 없음 | ✓ |
| GET `/api/relay/agent`, POST `.../agent/start`·`stop` | — (daemon 안의 relay agent 상태·제어) | — | — |

- 데몬은 노드 요청에 `x-request-timestamp`·`x-request-nonce`·`x-request-signature`(`METHOD\npath\nts\nnonce\nbody` HMAC)를 붙이지만, relay는 Bearer 토큰만 검증한다(§4.1).
- 서명하는 프록시(host status·nodes·members)와 페어링·bot-invite는 `signed_base`를 거쳐, 요청한 주소가 설정된 릴레이와 같거나 루프백(모의 릴레이)일 때만 따르고 아니면 설정된 릴레이로 보낸다(`relay.rs` `signed_base`, 59a93b8).
- `/poll`, `/ack`, `/heartbeat`, `/result`, `/events`는 daemon 안의 relay agent가 부른다. 릴레이 주소는 bot config의 `cloud.relayUrl`, 토큰은 `<data_dir>/.node_token`에서 읽는다(`relay_agent.rs`).
- bot config(접두사·별명)는 GUI가 daemon에만 저장하고 agent heartbeat의 `botConfig`로 relay에 간다. relay는 heartbeat metadata를 `hosts.metadata`에 저장하고 봇이 거기서 prefix를 읽는다(`heartbeat.ts`, `bot/state/nodeState.ts`).
- 길드·인스턴스별 사용자 낱말: heartbeat metadata `instanceWords?:{<guildId>:{<instance id>:{words:[낱말], commands:{<명령 이름>:[낱말]}}}}`(항목이 있는 길드만, 길드 50·길드당 인스턴스 50까지. 넘으면 heartbeat를 400으로 거절하고, 낱말 규칙 1~32자·공백 없음·10개에 맞지 않는 낱말은 버린다, `schemas.ts`·`catalog.ts` `userWords`). relay는 접두사 명령을 원문 그대로 데몬에 보내고(별명 치환을 하지 않는다), 낱말은 위험 명령 확인과 챗봇 채널의 서버 명령 인식에만 쓴다. 대상은 서버 이름·ID → 그 길드의 서버 낱말 → 모듈 이름·별명 순으로 찾고, 명령 낱말은 그 인스턴스 명령의 `aliases` 앞에 붙는다(`findCommandTarget`·`commandsFor`·`riskOf`). `instanceWords`가 없는 옛 데몬은 `botConfig.moduleAliases`·`commandAliases`를 같은 자리에 쓴다.
- `/사바쨩 상태`는 호스트가 온라인이면 heartbeat `servers`(길드는 `allowedInstances`로 거른 것, 20개까지)를 한 줄씩 붙인다. 각 줄에는 실행 여부, `players`(👥 접속/최대), `usage`(CPU %·RAM MB), `statusLine`을 마크다운·멘션·가린 링크 문자를 빼고 넣는다(`bot/commands/coreService.ts`). PC 전체 자원 줄(`monitor.summary`)은 붙이지 않는다(사용자 결정 2026-10-20). 끝에는 heartbeat `metadata.onboarding {steps:[{id, done, optional}], next}`(J1-a)로 다음 단계 한 줄을 붙인다: `next`가 `module`·`server`면 그 안내, `next`가 null인데 `chatbot` 단계가 안 됐으면 API 키 선택 안내.
- `/사바쨩 설정`(노드 소유자, 본인에게만 보임)은 이 길드의 챗봇 설정 일부를 바꾼다. `query chatbot.settings.get`으로 읽고 켜기·명령 전 확인·웹 검색·기억·도구 묶음(`disabledTools`) 토글과 호출어 모달을 보인다. 누르면 데몬의 지금 값에서 뒤집어 `plan chatbot.settings.set` → (`needsConfirm`이 false면) 같은 요청자로 `run_plan`을 보내고 다시 읽어 고친다. API 키·채널은 다루지 않는다(`bot/commands/chatbotSettings.ts`).
- 권한 판정의 주체는 데몬이다(`chat/permissions.rs`, 01 문서 BotConfig 절). 금지(-1) 멤버는 relay 게이트웨이가 음악·챗봇을 포함한 봇 기능 전체에서 먼저 막는다. 슬래시·버튼에는 숨김 답으로, 봇에게 한 메시지에는 10분에 한 번 짧은 안내로 답한다(`bot/gateway/gateway.ts`, `commandAccess.ts` `isBanned`). 설정이 없는 길드도 기본 레벨(유저)로 판정한다. relay의 `node_permissions`·`nodes.default_level`은 데몬 값의 캐시이고 relay 바깥 검사(`commandAccess.ts`)에 쓴다. `/사바쨩 권한` 명령은 `permission` action으로 데몬에 보낸다. 카탈로그에서 risk가 danger인 prefix·길드 슬래시 명령은 바로 보내지 않고 확인 버튼을 띄운다. 요청한 사람만 누를 수 있고, 누른 사람 이름으로 `raw_command`를 보낸다. 레벨이 모자라면 확인 없이 거부 문구다(`actionCommands.ts` `dangerConfirm`). 챗봇 도구(`run_command`·`run_action`·`plan_actions`)는 모델이 도구를 부르게 한 새 메시지의 id(`request_message_id`)만 받고, 요청자는 릴레이가 그 메시지의 실제 작성자로 넣는다(문맥 메시지·봇 메시지는 `current_human_request_required`). `run_command`·`run_action`의 확인은 위험도로 정한다: safe는 바로, caution은 챗봇 설정 `confirmCommands`가 켜져 있으면 확인, danger는 늘 확인이다(액션은 데몬의 `needsConfirm`도 따르고, 조회는 확인 없이 실행한다). `plan_actions`는 늘 확인이다. 이번 묶음에 글을 쓴 사람이 여럿이면 위험도와 상관없이 확인을 거치고(조회는 `multiple_requesters`로 거절), 버튼은 요청자만 누른다. heartbeat `permissions[길드]`에 `chatbotCap`·`levels`가 있으면 릴레이는 요구 레벨((c)의 모듈 명령 순서: `commandLevels["<인스턴스>/<명령>"]`, 내장 명령이면 `commandLevels["<인스턴스>/server.<명령>"]`, 없으면 `levels[risk]`)과 챗봇 레벨 min(작성자 레벨, `chatbotCap`)을 비교해(`commandLevelsTruncated`이거나 `levels.admin`·`chatbotCap`이 없는 길드는 비교하지 않는다. `run_action`과 `plan_actions`의 단계도 (c)의 액션 순서로 세고, 앞 단계가 만들 서버 `@N`은 세지 않는다) 모자란 요청을 보내지 않고 `permission_denied`로 돌려주며, 이 값을 모델에도 보인다(`describe_commands`의 `required_level`, payload `requester_levels`). 최종 판정은 데몬이다(`chatbot/tools.ts`).

---

## 5. server-chan 운영 구성요소와 운영 결함

> 이 절(공유 폴더에만 있는 chatbot/music-server, 운영 배포 토폴로지)과 §6.3–§6.5(relay 결함, 비밀 파일, 누락 소스)는 운영 서버 보안과 직결돼 공개 저장소에 두지 않는다. 비공개 저장소 `WareAoba/server-chan`의 `docs/architecture-server.md`에서 관리한다.

---

## 6. 불일치 · 누락 소스 · 스텁 · 보안 문제

### 6.1 소유자 프레이밍("daemon이 주인, 인터페이스는 무력") 위반

1. ~~**GUI가 업데이트 주기를 직접 돌린다**~~ → 해결: daemon이 주기 확인을 한다(`src/ipc/updates.rs:256-283`). GUI는 캐시된 상태만 10분마다 읽는다(`saba-chan-gui/src-tauri/src/lifecycle.rs:539-581`).
2. ~~**GUI와 CLI가 updater exe를 직접 spawn한다**~~ → 해결: 둘 다 `POST /api/updates/launch-updater`만 부르고 updater는 daemon이 띄운다(`updates.rs:609-675`, `saba-chan-gui/src-tauri/src/native.rs:81-102`, `saba-chan-cli/src/cmd/update.rs:249`).
3. ~~**GUI가 relay를 직접 호출한다**(`PATCH /api/hosts/:id/bot-config`)~~ → 해결: GUI는 bot config를 daemon에 저장하고, relay 동기화는 agent heartbeat가 맡는다(`saba-chan-gui/src/api/createApi.js:865-874`, `relay_agent.rs:381-395`).
4. ~~**GUI가 Discord bot 실행 명령을 조립한다**, 실행 경로가 두 개~~ → 해결: 로컬 Discord bot이 없어졌다. relay agent는 daemon 안에서 노드 토큰이 있으면 기동 때 자동으로 시작한다(`relay_agent.rs:178-189`, `ipc/mod.rs:860-866`).
5. GUI가 daemon을 `--spawned`로 spawn하고(Tauri 셸 `saba-chan-gui/src-tauri/src/lifecycle.rs:158-218`) installer를 `--uninstall`로 실행한다(`native.rs:149-184`). 인터페이스가 프로세스 감독 역할을 맡고 있다.
6. ~~**extensions 용어 충돌**(`discord_bot/extensions/*.js`, `music.js`가 Python 익스텐션을 직접 spawn)~~ → 해결: `discord_bot/`이 저장소에서 없어졌다.
7. **server-chan과 프레이밍의 관계**
   - relay는 프레이밍에 맞는다. Discord를 향하는 bot은 server-chan 쪽 하나뿐이고, saba-chan은 daemon 안의 relay agent로 실행기 역할만 한다. "server-chan이 bot에 추가 기능을 준다"기보다는 **server-chan이 bot 그 자체**다.
   - music-server는 relay bot의 부가 기능이지만 소스가 없다.
   - chatbot-server는 daemon과도 relay와도 연결이 없는 **별개의 bot**이라 프레이밍을 벗어난다.
   - 업데이트도 relay가 SSOT다(§1.3-1.4). server-chan은 bot 지원을 넘어 배포 인프라(서명된 릴리즈 manifest 제공)까지 맡는다.

### 6.2 updater, installer, CI 불일치와 버그

| # | 내용 | 근거 |
|---|---|---|
| U1 | ~~manifest `sha256`은 zip 해시이고 integrity는 압축을 푼 파일 해시라서 항상 `Tampered`가 된다~~ → 해결: 적용 때 푼 파일 해시를 `installed-files.json`에 기록하고 그 기록과 비교한다 | `integrity.rs:1-6, 104-121` |
| U2 | ~~다운로드와 설치 때 sha256을 검증하지 않는다~~ 해결됨: 업데이터·설치기·데몬 모듈 설치 모두 `download_verified`로 확인한다 | `download.rs:40` |
| U3 | ~~**Zip-slip**: `entry.name()`을 그대로 `join`한다~~ → 해결: 모든 zip 해제가 `enclosed_name()`으로 검사하는 `extract_zip` 하나를 거치고, tar.gz는 `..`·절대 경로 엔트리를 건너뛴다 | updater `lib.rs:307-340`, `apply.rs:475-486`; installer `lib.rs:850-854` |
| U4 | ~~`saba-core` 버전 폴백이 updater-lib 버전이다~~ → 해결: 폴백은 빌드에 묶인 `versions.toml` 버전이다 | `lib.rs:587-590`, `versions.rs` |
| U5 | ~~`prepare_daemon_update`는 사용되지 않는다. rollback 경로가 없다~~ → 해결: 함수는 지워졌고, 적용 도중 실패하면 백업·`.old`로 되돌린다. 적용이 끝난 뒤 이전 판으로 돌아가는 경로는 여전히 없다 | `apply.rs:24-62, 142-167` |
| U6 | ~~v2 모듈(worker/foreground/queue/scheduler/ipc)이 죽은 코드다. `cli.rs.removed`, `lib.rs.bak` 잔재~~ → 해결: 지워졌다(e023a75). ~~헤더 주석만 남았다~~ → 해결: 헤더도 현재 파일 구성으로 바뀌었다 | `lib.rs:1-12` |
| U7 | ~~설정이 저장되지 않는다, GUI가 POST를 쓴다~~ → 해결: `config.json` `updates` 절, GUI도 PUT | `updates.rs:844-942`, `saba-chan-gui/src/api/createApi.js:754` |
| U8 | ~~CLI의 `/api/install/*`은 daemon에 없다~~(호출이 지워졌다). ~~CLI apply·download는 여전히 body 없는 POST다~~ → 해결: PR #6 CLI는 본문을 보내고, 데몬도 본문 없는 apply·download를 전체 대상으로 받는다 | `saba-chan-cli/src/client.rs:774-784`, `src/ipc/updates.rs:348-371` |
| U9 | ~~CLI `launch-apply`가 넘기는 `--install-root`와 대상 인자를 updater가 무시한다~~ → 해결: PR #6 CLI `update apply`는 daemon의 `launch-updater`만 부른다 | `saba-chan-cli/src/cmd/update.rs:237-249` |
| U10 | CI가 windows-x64 릴리즈만 만든다 | `package-release.py`, `release.yml` |
| U11 | ~~`generate-manifest.py`, `release-manifest.json` 템플릿~~ 지워졌다 | `.github/` |
| U12 | ~~모듈과 익스텐션 소스가 4갈래로 갈린다~~ 해결됨: 모두 같은 릴리즈 manifest | §1.4 |
| U13 | `Program Files` 기본 설치에 권한 상승 없이 업데이트를 적용한다 | installer `lib.rs:797-801` |
| U14 | ~~GitHub API 비인증 rate limit~~ 해당 없음: GitHub를 조회하지 않는다 | §1.4 |
