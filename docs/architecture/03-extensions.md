# 03. 익스텐션(= 플러그인) 시스템 — 코드 기준 분석

> 기준: saba-chan 저장소 (daemon, GUI, CLI) + saba-chan-extensions 저장소 (steamcmd, ue4_ini).
> 모든 내용은 코드에서 직접 추출했다. README / docs / copilot-instructions는 참고하지 않았다(오래된 문서로 간주).
> 경로 약어: `ext/` = `saba-chan/src/extension/mod.rs`, `plg/` = `saba-chan/src/plugin/mod.rs`, `EXT/` = `saba-chan-extensions/`.

---


> **2026-10-17 L3 이후**: 업데이트·모듈 목록·익스텐션 목록은 모두 릴레이의 서명된 릴리즈 manifest 하나에서 나온다. GitHub 업데이트 경로(`github.rs`, `api_base_url`, `github_owner`/`github_repo`, `include_prerelease`, 익스텐션 `manifest_url`, module.toml `[update]`, modules·extensions 저장소의 릴리즈 워크플로)는 없다. 근거 커밋: saba-chan 65e1763·8b840f7·60768a7, modules a26bdeb, extensions 301ff2c.
>
> **현재 코드 기준 요약**: 로컬 Discord 봇(`discord_bot/`), docker·music 익스텐션, `ext_process` API는 모두 없어졌다. 익스텐션 저장소에는 `steamcmd`, `ue4_ini` 두 개만 있다. 활성 목록과 익스텐션 글로벌 설정은 `<data_dir>/config.json`의 `extensions.enabled`, `extensions.settings`에 저장된다.

## 0. 용어 정리: "plugin" vs "extension" (프로젝트 오너 프레이밍 대비)

프로젝트 오너가 정의한 구조는 다음과 같다.

- **daemon** = 서비스의 주인. 모든 기능은 daemon을 거친다.
- **interface** (GUI/CLI) = 스스로는 아무것도 못 한다.
- **module** = Python으로 된 게임 서버 "매뉴얼".
- **plugin** (코드상 이름은 **extension**) = Python. daemon에 기능을 붙이거나 interface에 컴포넌트를 주입한다. 자유롭게 커스터마이즈할 수 있다.
- **server-chan** = daemon과 함께 쓰는 Discord 봇.

코드에서 이 단어들이 실제로 어떻게 쓰이는지는 다음과 같다.

| 코드상 이름 | 실제 의미 | 근거 |
|---|---|---|
| `crate::plugin` (`src/plugin/mod.rs`) | **개념이 아니라 실행기**다. "Python 파일 하나를 `python <file> <function>`로 띄우고 stdin JSON → stdout JSON을 주고받는 러너"일 뿐이다. **module의 `lifecycle.py`와 extension hook이 둘 다** 이 러너를 쓴다. | `plg/:168-317`. module 호출 예: `supervisor/mod.rs:791,1077,1178…`. extension 호출: `ext/:1583` |
| `crate::extension` (`src/extension/mod.rs`) | 오너가 말하는 "plugin". manifest 기반이고, hook, 명령, GUI/CLI 슬롯, instance_fields, config_fields를 가진다. | `ext/:221-271` |
| `ipc::ExtensionInfo` / `ExtensionListResponse { modules }` | 이름은 extension이지만 **실제로는 module 정보**다(명명 혼동). | `src/ipc/mod.rs:236-295`, `handlers/server.rs:10,334-392` |
| GUI `selectedExtension` | AddServer 모달에서 **module 이름**을 담는 상태다. | `saba-chan-gui/src/components/Modals/AddInstanceNewServer.js:32,86` |
| ~~`discord_bot/extensions/*.js`~~ | → 해결: 로컬 Discord 봇이 저장소에서 빠져 제3의 "extension" 개념도 없다. | — |
| updater의 extension 업데이트 | 릴리즈 manifest의 `ext-<id>` 항목. 로컬 버전은 `extensions/*/manifest.json`의 id·version으로 읽는다. 데몬의 익스텐션 목록(`fetch_remote`)과 같은 manifest를 본다. | `updater/src/lib.rs:274-277,608-620`, `src/extension/mod.rs:1395` |

**결론**: 코드에서 "plugin"과 "extension"은 서로 다른 계층이다. `plugin`은 Python 프로세스 실행 프로토콜이고, `extension`은 그 위의 확장 시스템이다. 오너가 말하는 "plugin"은 코드의 `extension`에 해당하고, 코드의 `plugin` 모듈은 module과 extension이 공유하는 하위 런타임이다. 옛 레이아웃용 `run_extension`/`run_extension_with_progress`는 삭제되었다.

---

## 1. Extension manifest.json 스키마 (Rust 역직렬화 기준)

### 1.1 `ExtensionManifest` (`ext/:221-271`)

| key | Rust 타입 | 필수/기본값 | 코드에서 실제 사용 여부 |
|---|---|---|---|
| `schema_version` | `u32` | `0` | `parse_manifest()`(`ext/:374-421`): `1`이면 모르는 키·발화되지 않는 hook·없는 python_modules 참조가 오류, `0`이면 같은 문제를 경고로만 남기고, 더 큰 값은 거부한다. 현재 두 manifest 모두 `1`이다. |
| `id` | `String` | **필수** | discovered 맵의 키. 영문·숫자·`-`·`_`만 허용한다(`ext/:359-368`). 스캔 시 폴더 이름이 id이거나 하이픈을 밑줄로 바꾼 이름이어야 한다(`ext/:763-769`). `mount()`는 `manifest.id == ext_id`를 검사한다(`ext/:811`). |
| `name` | `String` | **필수** | 목록과 로그 |
| `version` | `String` | **필수** | 업데이트 비교(SemVer, 파싱 실패 시 문자열 비교 `ext/:1335-1342`), 익스텐션 간 의존성 버전 검사 |
| `description` / `descriptions` | `String` / `HashMap` | `""` / `{}` | 목록. `GET /api/extensions?lang=`이 GUI 언어에 맞는 `descriptions` 항목으로 바꿔 준다(`handlers/extension.rs:67-91`). |
| `author` | `String` | `""` | 목록 |
| `min_app_version` | `Option<String>` | `None` | enable(`ext/:929-933`), 패키지 설치(`ext/:1405`)에서 강제하고, 목록·원격 목록에 `incompatible`로 표시한다(`ext/:1108,1289`). |
| `dependencies` | `HashMap<String,String>` | `{}` | 커스텀 디시리얼라이저 `deserialize_dependencies` (`ext/:56-97`): 배열 `["a"]` → `{"a":"*"}`, 맵은 그대로 받는다. enable/disable/unmount 검증에 쓰인다(아래 3.3). |
| `python_modules` | `HashMap<String,String>` | `{}` | 모듈 키 → 익스텐션 디렉토리 기준 상대경로. hook 바인딩과 명령이 참조한다(`ext/:1165,1240`). 값은 폴더 안 상대 경로만 받는다(모듈 `entry`와 같은 `utils::is_inner_relative_path`): 빈 값, 절대 경로, `..`·`.`가 든 경로는 `schema_version`과 무관하게 manifest 오류다. 신뢰 판정(`sandbox::component_in`)도 `..`·`.`가 든 진입점은 어느 구성요소에도 넣지 않는다. |
| `hooks` | `HashMap<String, HookBinding>` | `{}` | hook 이름 → 바인딩. 옛 이름 `server.post_stop`은 `server.stop`으로 바꿔 읽는다(`ext/:15-17,40-51`). |
| `gui` | `Option<GuiManifest>` | `None` | 번들/스타일 서빙에 쓰인다(`handlers/extension.rs:240-334`). |
| `cli` | `Option<CliManifest>` | `None` | `/api/extensions` 응답에 그대로 실린다. `cli.slots`는 데이터 slot 선언이다(4.3). 인터페이스는 이 원본 대신 정규화된 `slots`를 읽는다. |
| `module_config_section` | `Option<String>` | `None` | **읽는 코드가 없다(dead 필드)**. module.toml의 익스텐션별 설정은 범용 `[extensions.<id>]` 섹션으로 받아 그대로 넘긴다(`supervisor/module_loader.rs:618-622,1139-1144`). |
| `instance_fields` | `HashMap<String, FieldDef>` | `{}` | (a) disable/remove/unmount/install 시 "사용 중" 판정(`ext/:873-902`), (b) ~~목록 응답 → CLI가 자동 설정 슬롯을 만든다~~(PR #6 CLI는 쓰지 않는다). **default 값을 인스턴스에 주입하는 코드는 없다.** |
| `config_fields` | `HashMap<String, FieldDef>` | `{}` | 목록 응답에만 실린다. 값 저장은 `config.json`의 `extensions.settings.<id>`. 스키마 검증은 없다. |
| `i18n_dir` | `Option<String>` | `None` | `load_i18n()` → `<dir>/<i18n_dir>/<locale>.json`. `i18n_dir`은 `..` 없는 상대 경로이며 실제 경로도 익스텐션 폴더 안이어야 한다. 중간 링크·정션도 거부한다. 위반하면 locales를 비우고 경고한다. locale은 한 단계 경로 이름이어야 한다(`ext/:1457-1469`). |
| `priority` | `i32` | `100` | 같은 hook·명령을 여러 익스텐션이 가질 때의 순서. 작을수록 먼저, 같으면 id 순(`ext/:265-267,1116-1127`). |
| `commands` | `Vec<ExtensionCommand>` | `[]` | 인스턴스 명령 카탈로그에 더하는 명령(`ext/:273-293`). `GET /api/instance/:id/commands`에 실리고(모듈 명령이 우선, `handlers/command.rs:296-300`), `POST /api/instance/:id/command`가 `plan_command`로 실행한다(`handlers/command.rs:90-102`, `ext/:1143-1176`). |
| `permissions` | `ModulePermissions` | 모두 false | 코드가 쓰는 위험한 능력 선언. 모듈 `[permissions]`와 같은 표다(02 문서 §1). 실행을 막거나 풀지 않고, `GET /api/extensions/:id/audit`(모듈 audit과 같은 모양)와 CLI 도구 `saba-core extension audit <폴더>...`가 코드와 대조해 선언하지 않은 능력을 알린다. |

### 1.2 하위 구조체

- **`HookBinding`** (`ext/:299-305`): `module: String`(필수, `python_modules`의 키), `function: String`(필수), `condition: Option<String>`.
  - ~~`async`(→`is_async`)는 읽는 코드가 0건이다.~~ → 해결: 필드가 없어졌다. schema v1 manifest에 `async`를 쓰면 모르는 키 오류다.
- **`GuiManifest`** (`ext/:307-317`): `bundle`, `styles`, `builtin: Option<bool>`, `slots: HashMap<String,String>`.
  - `builtin`은 daemon에서 읽지 않는다.
  - `slots`도 **GUI에서 읽지 않는다.** GUI는 번들의 `registerSlots()` 반환값만 쓴다(4절).
- **`CliManifest`** (`ext/:320-326`): `slots: HashMap<String, Value>`. 데이터 slot 선언(4.3). 값은 항목 하나 또는 배열이다.
- **`FieldDef`** (`ext/:328-336`): `type`(필수), `default: Option<Value>`, `optional: Option<bool>`.
- **`ExtensionCommand`** (`ext/:275-293`): `name`, `label?`, `description?`, `risk?`(safe/caution/danger), `inputs[]`(모듈 명령과 같은 `CommandInput`), `module`, `function`, `condition?`.

알 수 없는 키는 `serde_ignored`로 모아 schema v1이면 오류, v0이면 경고(`warnings`, 목록 응답에 실림)로 처리한다(`ext/:374-421`).

### 1.3 실제 manifest에는 있는데 코드가 무시하는 것

| 키 / 값 | 위치 | 상태 |
|---|---|---|
| `min_app_version: "0.1.0"` | 2개 manifest 전부 | ~~강제하지 않음~~ → 해결: enable·설치에서 강제한다(`ext/:929-933,1405`). |
| `instance_fields.ue4_ini_enabled`, `ue4_ini_section` | `EXT/ue4_ini/manifest.json` | 이 값을 설정하는 코드가 어디에도 없다. "사용 중" 판정에만 쓰이므로 사실상 항상 미사용이다. |
| ~~docker의 `"async": true`, `gui.slots`, `module_config_section: "docker"`, music의 `gui.builtin`, `config_fields.discord_token` 등~~ | — | → 해결: docker·music 익스텐션이 저장소에서 제거되었다. |
| ~~발화되지 않는 hook 이름(`server.pre_create`, `server.stats`, `server.logs`, `server.install`, `server.update`, `module.parse_settings`, `module.write_settings`)~~ | — | → 해결: 현재 manifest에는 없다. 이런 이름을 쓰면 `parse_manifest`가 schema v1에서 거부한다(`ext/:387-390`). |

### 1.4 코드는 지원하는데 실제 manifest에서 안 쓰는 것

- `gui` 전체: 현재 두 익스텐션 모두 `gui`가 없다. CSS 서빙 경로(`handlers/extension.rs:309-334`)와 번들 경로가 있지만 쓰는 익스텐션이 없다.
- `commands`, `priority`, `i18n_dir`, `config_fields`, `cli`: 현재 manifest에서 쓰지 않는다.
- `dependencies`의 비-익스텐션 컴포넌트 키: HTTP enable은 `enable()`이 `{"saba-core": APP_VERSION}`을 넘기므로(`ext/:905-908`) `saba-core` 버전 요구는 검사된다. 그 밖의 컴포넌트 키(`gui` 등)에 버전을 쓰면 설치 버전을 모르므로 활성화할 수 없다(`ext/:960-984`).
- `condition` 패턴은 `instance.ext_data.<key>` 하나뿐이다(`ext/:1179-1195`). 그 외 패턴은 warn을 남기고 false로 처리한다.

### 1.5 원격 목록 항목 (`RemoteExtensionInfo`, `ext/:476-518`)

```
릴리즈 manifest 컴포넌트 "ext-<id>": { version, url, sha256, filename, install_dir?,
  meta: { name, description, author, min_app_version?, dependencies(배열|맵), has_gui, has_i18n } }
→ RemoteExtensionInfo { id, name, version, description, author, download_url(=url), sha256, asset(=filename),
                        install_dir?, min_app_version?, incompatible?, dependencies, has_gui, has_i18n, tags[], homepage? }
```

`from_release`(`ext/:1295-1324`)가 `meta`를 바탕으로 `version`·`download_url`·`sha256`·`asset`·`install_dir`을 덮어쓰고, id는 컴포넌트 키에서 `ext-`를 뗀 값이다. `meta`는 코어 릴리즈 워크플로의 `versions.py`가 익스텐션 manifest에서 만든다(`D:scripts/versions.py:246-267`). `tags`/`homepage`는 넣지 않는다. `install_dir`은 폴더 이름을 쓰므로 `extensions/ue4_ini`다. `install_dir`과 `asset`은 daemon이 읽지 않는다(설치 폴더는 `ext_dir_on_disk`가 정한다, `ext/:596-616`).

---

## 2. Hook 시스템

### 2.1 Python 호출 프로토콜 (`plg/:168-332`)

- **명령**: `<venv python> <절대경로 module.py> <function>`. Python 경로는 `python_env::get_python_path()`(`plg/:178`)에서 얻는다. 매 호출마다 새 프로세스를 띄우는 short-lived 방식이다.
- **stdin**: context JSON 한 덩어리. 쓴 다음 바로 닫는다(`plg/:239-242`).
- **stdout**: 최대 4 MiB를 모아 하나의 JSON으로 파싱한다. 실패하면 비어 있지 않은 마지막 줄을 JSON으로 파싱한다(앞의 print 무시). 둘 다 실패하면 `module_bad_output`, 상한 초과면 `module_output_too_large`, 종료 코드가 0이 아니면 `module_crashed`다.
- **stderr**: 줄 단위로 스트리밍한다. `PROGRESS:{json}` 줄은 `ExtensionProgress{percent,message,step,total,label}`로 파싱해 콜백을 호출하고 `LOOPBACK:`·`HOST:`는 중계로 처리하고 그 줄을 로그에 넣지 않는다. 나머지는 계속 비우고 마지막 200줄, 합계 64 KiB를 보존한다. 앞의 줄은 debug 로그로만 남긴다.
- **env**:
  - `PYTHONIOENCODING=utf-8`, `PYTHONUNBUFFERED=1`
  - 데몬 연결 정보 `SABA_IPC_PORT`, `SABA_IPC_URL`, `SABA_TOKEN_PATH`(토큰 파일 경로, 토큰 자체는 넣지 않음), `SABA_LANG`(요청의 `Accept-Language`, 없으면 config.json `gui.language`) (`plg/:14-37`).
  - `PYTHONPATH = <import 가드 폴더>:<extensions_dir의 부모>:<data_dir>/python-shared:<기존 PYTHONPATH>`. extensions 디렉토리의 **부모**를 넣으므로 `import extensions.steamcmd`가 동작한다. `sitecustomize.py`는 표준 라이브러리 경로를 먼저 두고, 활성 익스텐션 외의 `extensions.<id>` import를 거부한다.
  - `SABA_ENABLED_EXTENSIONS=<콤마 목록>`: ~~`%APPDATA%`나 cwd의 `extensions_state.json`을 읽어 Linux/macOS에서 대개 설정되지 않는다.~~ → 해결: `config.json`의 `extensions.enabled`에서 읽고, 목록이 없으면 빈 값으로 항상 설정한다(`plg/:200-205`).
  - import 가드: 데이터 폴더의 `python-guard/sitecustomize.py`(`plg/:84-96`, `src/plugin/sitecustomize.py`)가 `SABA_ENABLED_EXTENSIONS`에 없는 `extensions.<이름>` import를 `ModuleNotFoundError`로 막는다(`-`와 `_`는 같게 본다).
  - 공용 모듈 `i18n.py`, `daemon_rcon.py`를 modules 디렉토리에 내장본과 같게 맞춘다(`plg/:98-116,217-224`).
- **확인되지 않은 익스텐션**: 플랫폼과 관계없이 모듈과 같은 거부·동의 정책을 따른다(02 문서 §2.1). Windows는 AppContainer를 쓴다. Linux·macOS는 격리할 수 없으므로 현재 파일 지문에 대한 동의가 없으면 `sandbox_unavailable`·`isolation_unsupported`와 `component`를 돌려준다. 모듈이 import하는 익스텐션도 각각 자기 동의가 있어야 격리 없이 실행할 수 있다. 모듈의 동의로 다른 익스텐션까지 승인하지 않는다.
  - hook은 확인된 모듈의 인스턴스에도 불리므로 config의 `instance_dir`·`working_dir`·`install_dir`은 읽기만 준다. 쓸 수 있는 곳은 익스텐션 전용 `<data_dir>/extension-data/<이름>`이고 경로는 환경 변수 `SABA_EXTENSION_DATA_DIR`로 받는다. 모듈이 익스텐션을 import해 쓸 때(SteamCMD 등)는 `SABA_EXTENSION_DATA_ROOT/<id>`에 쓴다(격리된 모듈은 그 모듈만의 뿌리를 받는다, 02 §2.1). 인스턴스 폴더에 써야 하는 익스텐션은 비격리 동의가 있어야 한다. 데몬의 비밀 파일과 `backups/`는 상속을 끊어 두어 이 읽기 권한이 닿지 않는다(02 §2.1).
  - `server.pre_start`가 돌려준 `log_follower.program`도 모듈의 실행 대상과 같이 검사한다(인스턴스 폴더·사용자가 정한 서버 폴더 안, 셸 거부, `env_vars`의 `PATH`·`PATHEXT` 버림). 막히면 시작 결과가 `launch_outside_instance`다.
- **타임아웃과 자리**: 기본 `DEFAULT_PLUGIN_TIMEOUT_SECS = 120`이다. 함수 이름이 `install_server`면 설치 자리 2개를 쓰고, 훅을 포함한 나머지 호출은 일반 자리 8개를 쓴다. 기한은 자리 대기·내부 선조회·Python 환경 준비·실행 전체를 포함한다. 내부 선조회는 바깥 자리와 절대 기한을 재사용한다. 재진입은 바깥 자리를 빌리거나 캐시만 쓰고 캐시가 없으면 실패한다. 상한·정리 규칙은 [01 §7.1](01-daemon-core.md#71-srcpluginmodrs-python-모듈익스텐션-실행기-단발-프로세스)을 따른다.
- **오류 필드**: 실행기와 훅은 [01 §7.1](01-daemon-core.md#71-srcpluginmodrs-python-모듈익스텐션-실행기-단발-프로세스)의 공통 오류를 유지한다. `error_code`·`module`·`function`·`phase`·`stderr_tail`을 문자열 하나로 바꾸지 않는다. 로컬은 stderr 끝을 보존하고 원격은 빼며, 원격 문자열의 IP·비밀값·절대 경로를 가린다. 모듈이 정한 실패 코드와 메시지는 유지한다.

### 2.2 디스패처 (`ext/:1213-1256`, `ext/:1557-1628`)

- `hooks_for(name)`(`ext/:1116-1127`): **enabled**이면서 해당 hook을 가진 익스텐션 목록. ~~`HashMap`이라 실행 순서가 비결정적이다.~~ → 해결: `priority`, 그다음 id 순으로 정렬한다.
- `plan_hook(name, ctx, only)`(`ext/:1213-1256`): 매니저 락 안에서 실행 목록만 정한다.
  1. 부를 익스텐션마다 context를 따로 만들고, `extension_config`에는 그 익스텐션 자신의 설정만 `{<id>: {k: v}}` 형태로 넣는다(`ext/:1303-1312`). 다른 익스텐션의 설정은 넘기지 않는다. 명령(`plan_command`)도 같다.
  2. `context.extension_data`로 `condition`을 평가한다(bool, 0이 아닌 숫자, 비어 있지 않은 문자열이면 true).
  3. `python_modules[binding.module]` 경로를 정한다. 없는 모듈이면 그 호출은 Err 결과가 된다.
- `HookPlan::run(timeout)`(`ext/:1572-1600`): **락 밖에서** 순서대로 실행한다. 결과에 `handled == true`가 있으면 **나머지 익스텐션을 건너뛴다**. 에러가 나면 기록하고 계속 진행한다(graceful).
- `run_hook` = `run_hook_timed(…, 120)`(`ext/:1605-1617`). `run_hook_to(ext_id, …)`(`ext/:1620-1628`)는 익스텐션 하나에만 보낸다.
- ~~`dispatch_hook_with_progress`는 첫 익스텐션 하나만 실행하고 `break`한다.~~ → 해결: progress 디스패처가 없어졌다. hook 경로에는 progress 콜백이 없다.
- 결과 병합: 디스패처는 병합하지 않고 `Vec<(ext_id, Result<Value>)>`를 반환한다. 해석은 호출부마다 다르다(아래 표).
- ~~모든 호출부가 `extension_manager.read()` 락을 Python 실행이 끝날 때까지 쥐고 있다.~~ → 해결: 락은 `plan_hook` 동안만 잡고 Python은 락 밖에서 실행한다(`ext/:1603-1617`). 긴 hook 동안에도 enable/disable/install/rescan이 막히지 않는다.

### 2.3 daemon이 발화하는 hook 전수 목록

`FIRED_HOOKS`(`ext/:23-34`)가 이 목록을 코드로 갖고 있고, manifest 검증이 이 목록을 기준으로 한다.

| hook | 발화 위치 | 트리거 / 경로 | 동기성 · 타임아웃 | context 키 | 결과 사용 |
|---|---|---|---|---|---|
| `daemon.startup` | `src/main.rs:365-410` | daemon 기동 직후 | `tokio::spawn` 백그라운드, 120s | `{}` (+extension_config) | 결과마다 `success`(기본 true), `message`, `error` → `ExtensionInitTracker`(`/api/extensions/init-status`) |
| `daemon.startup` (재발화) | `handlers/extension.rs:108-145` | `POST /api/extensions/:id/enable` 성공 후 | spawn, 120s | `{}` | ~~모든 익스텐션의 startup을 다시 실행~~ → 해결: `run_hook_to`로 **방금 켠 익스텐션에만** 보낸다(`:122-123`). |
| `daemon.shutdown` | `src/main.rs:525-567` | 종료 경로(Ctrl-C/SIGTERM, 종료 API, 워치독) | 동기(await), hook 120s, 전체 30s 상한(`:545-554`) | `instances[] {id,name,module,extension_data,instance_dir}` | `handled` 여부를 로그로만 남긴다 |
| `server.pre_start` | `supervisor/mod.rs:643-660` (non-managed start), `:1368-1387` (managed start) | 서버 시작 | 동기, 120s | `instance_id, instance_dir, module, extension_data` (+managed 쪽에 `instance, config, managed:true`. `instance`와 `config`의 비밀 값은 `server.post_create`와 같이 `***`로 가린다(D-E2)) | `handled:true` + `success!=true` → 에러를 반환한다. `success:true`이면 `log_follower{program,args,working_dir,description,interactive,strip_prefix,env_vars}`로 `ManagedProcess::spawn`(interactive) 또는 `spawn_log_follower`를 실행한다(`:682-753`, `:1409-1480`). 그 결과를 그대로 반환하고 기본 시작 로직을 건너뛴다. |
| `server.stop` (옛 이름 `server.post_stop`) | `supervisor/mod.rs:908-932` | `stop_server` 진입 직후, 정지 전 | 동기, 120s | 위와 동일 | `handled:true`이면 cooldown을 기록하고 그대로 반환한다. 익스텐션이 Err이거나 `success:false`이면 실패를 반환한다 |
| `server.status` | `supervisor/mod.rs:1131-1142` | `GET /api/server/:name/status` | 동기, 120s | + `process_patterns` | `handled:true`이면 그대로 반환한다 |
| `server.list_enrich` | `handlers/server.rs:100-153` | `GET /api/servers`, `extension_data`가 비어 있지 않은 인스턴스마다 **순차** 실행 | 동기, **10s**, 결과를 30s TTL 캐시(`ipc/mod.rs:617`) | `instance_id, instance_name, module, extension_data, instance_dir, process_patterns` | `handled`와 무관하게 첫 번째 Ok를 캐시한다(실패는 캐시하지 않음). `handled:true`이면 `status`와 `extension_status[<res.extension_id or "default">]`를 채운다. `handled:true` 결과의 `status_line`(짧은 한 줄, 경로·비밀값 금지)은 디스코드 `/사바쨩 상태`에 나간다(05 §4 heartbeat `statusLine`) |
| `server.post_create` | `handlers/instance.rs:357-370` | 인스턴스 생성(`POST /api/instances`), `instance_store.add` 성공 직후 | `tokio::spawn` 백그라운드, 120s. 결과를 기다리지 않고 바로 네이티브 프로비저닝으로 넘어간다 | `instance_id, instance_dir, instance, module, migration`. `instance`의 REST 비밀번호와 비밀 설정 값(모듈이 password로 선언했거나 이름에 pass·token·secret 같은 낱말이 든 것)은 `***`로 가린다(D-E2) | 결과를 버린다. 프로비저닝과 동시에 돈다 |
| `server.pre_delete` | `handlers/instance.rs:826-846` | 인스턴스 삭제 | 동기, 120s | `instance_id, instance_name, module, extension_data` | `handled`이면 2초 대기한다(파일 잠금 해제용) |
| `server.settings_changed` | `handlers/instance.rs:1350-1364` | 인스턴스 설정 PATCH, `extension_data`가 비어 있지 않을 때 | 동기, 120s | `instance_id, instance_dir, instance, module_extensions(module.toml [extensions.*] 전체), extension_data, settings`. `instance`와 `settings`의 비밀 값은 `server.post_create`와 같이 `***`로 가린다(D-E2) | 결과를 버린다(`_results`) |
| `server.check_update` | `handlers/instance.rs` `server_update_status` | `GET /api/instance/:id/check-update`, 그리고 `/api/instance/:id/start`로 새로 켠 뒤 뒤에서(`update_available:true`면 `instance.update_notice {kind:"server"}`, `server.rs` `announce_updates`) | 동기(시작 뒤에는 spawn), 120s | `instance_id, instance_name, module, instance_dir, install_dir, app_id, extension_data` | 첫 번째 Ok를 그대로 응답한다. 없으면 `{update_available:false, reason:"no_hook_handled"}`. `app_id`는 `extension_data["<방식>_app_id"]`, 없으면 module.toml `[install].app_id`(`:1433-1437`) |
| `command.<name>` (익스텐션 명령) | `handlers/command.rs:90-102` | `POST /api/instance/:id/command`에서 모듈에 없는 명령 | 동기, 120s | `instance_id, instance_name, module, extension_data, running, args` | 결과를 명령 응답으로 돌려준다 |

~~**manifest에 선언됐지만 daemon이 발화하지 않는 hook** 7종~~ → 해결: 현재 manifest에는 없고, 새로 선언하면 `parse_manifest`가 v1에서 거부, v0에서 경고한다(`ext/:387-390`).

- 업데이트 적용(`POST /api/instance/:id/apply-update`, `handlers/instance.rs:1477-1761`)과 네이티브 프로비저닝(`:371-624`)은 hook을 거치지 않는다. **module의 `lifecycle.py install_server`를 직접 호출**하고, module이 `from extensions.steamcmd import SteamCMD`로 익스텐션을 Python 라이브러리처럼 import한다. apply-update는 `install_method_<방식>`이 켜진 인스턴스만 받는다(`:1547-1560`).

### 2.4 hook × extension 매트릭스

● = 선언했고 daemon이 발화함, 빈칸 = 미선언

| hook | steamcmd | ue4-ini |
|---|---|---|
| server.check_update | ● `check_update` | |

ue4-ini는 hook을 선언하지 않는다. steamcmd의 condition은 `instance.ext_data.install_method_steamcmd`다. docker·music 열은 익스텐션 제거로 없어졌다.

### 2.5 Python 쪽 엔트리포인트 호환성

| 파일 | `__main__` 디스패치 | daemon context와 맞는지 |
|---|---|---|
| ~~`EXT/docker/*.py`, `EXT/music/music_deps.py`~~ | — | → 해결: 익스텐션이 제거되었다 |
| `EXT/steamcmd/steamcmd.py:879-915` | `FUNCTIONS = check_update, ensure, status`. `_run_cli`는 실패도 `{success:false,error,message}`로 출력하고 0으로 종료한다. BOM을 제거한다(`:891-912`) | `check_update`는 `app_id, install_dir`, 브랜치는 `branch` → `extension_data.steamcmd_beta` → `"public"` 순으로 읽는다(`:846-876`). daemon은 `app_id, install_dir, extension_data`를 준다. OK |
| `EXT/ue4_ini/ue4_ini.py` | **`__main__`가 없다.** 함수 시그니처도 `(ini_path, …)`라 hook 프로토콜과 맞지 않는다 | hook을 선언하지 않으므로 문제없다. module이 `from extensions.ue4_ini import …`로 쓰는 **순수 라이브러리**다 |

---

## 3. 익스텐션 수명주기

### 3.1 저장 위치

- 익스텐션 디렉토리: `resolve_extensions_dir()` = `$SABA_EXTENSIONS_DIR` → `config.json` `paths.extensions` → `resolve_data_dir()/extensions` (`updater/src/component_dirs.rs` `resolve`). 옮길 때도 폴더 이름은 `extensions`로 둔다(01 §1.5). data_dir은 `$SABA_DATA_DIR`, Windows는 `%APPDATA%/saba-chan`, Unix는 `~/.config/saba-chan`이다(`:55-77`).
- 활성 상태: `<data_dir>/config.json`의 `extensions.enabled`. 형식은 정렬된 enabled id 배열이다(`ext/:1500-1521`).
- 익스텐션 글로벌 설정: 같은 파일의 `extensions.settings`. 형식은 `{ext_id:{k:v}}`(`ext/:1523-1549`). 옛 `extensions_state.json`, `extensionConfig.json`은 한 번 이 두 키로 옮겨진다(`updater/src/config_file.rs:43-44`).
- 테스트 생성자 `new_isolated`는 config.json을 extensions 디렉토리 안에 둔다(`ext/:557-560`).

### 3.2 발견 (discover / rescan / mount)

- `discover()`(`ext/:623-648`): 루트에 있는 `*.zip`을 먼저 푼 뒤 `scan()`을 실행한다. 다운로드와 ZIP 파일은 128 MiB, 항목은 10,000개, 누적 해제 크기는 512 MiB로 제한한다. 모듈 ZIP과 익스텐션 패키지는 같은 해제기(`src/archive.rs`)로 푼다: 폴더 밖을 가리키는 경로와 링크 항목은 거부하고, 권한 비트(SUID 등)는 옮기지 않는다. zip은 `.staging-<id>`에 풀어 manifest를 검사하고, 기존 폴더가 있으면 **새 내용으로 바꿔 끼운 뒤** zip을 삭제한다(`ext/:652-721`).
- `scan()`(`ext/:737-783`): 하위 디렉토리 중 `manifest.json`이 있는 것(`.`으로 시작하는 임시 폴더 제외)을 등록한다. **폴더 이름이 manifest.id(또는 하이픈→밑줄)와 다르면 건너뛰고**, 로드 실패와 함께 `load_errors`에 남긴다. 이 목록은 `GET /api/extensions`의 `failed`로 나간다(`handlers/extension.rs:85-90`).
- `rescan()`(`ext/:792-796`): `scan()`과 같다. 기존 항목의 manifest를 다시 읽고 새 항목을 추가한다. 디스크에서 사라진 항목은 그대로 남긴다.
- `mount(id)`(`ext/:800-821`): `ext_dir_on_disk`(discovered → id 그대로 → 하이픈을 언더스코어로 바꾼 이름)로 디렉토리를 찾고, `manifest.id == id`를 검사한다.
- `IPCServer::new`(`src/ipc/mod.rs:590-598`)에서 discover를 한 번 실행한다. 상태에 enabled로 되어 있지만 디스크에 없는 익스텐션은 `enabled_but_not_installed()`로 구분한다(`ext/:1076`).

### 3.3 enable / disable / remove / unmount

| 작업 | API | 동작 |
|---|---|---|
| enable | `POST /api/extensions/:id/enable` (`handlers/extension.rs:97-159`) | `enable()` → `enable_with_versions(id, {"saba-core": APP_VERSION})`(`ext/:905-991`). `min_app_version`을 먼저 확인한다. dependencies 맵의 각 키를 확인한다. (a) discovered에 있는 익스텐션이면 **enabled 상태여야 하고**, `">=x"`이면 SemVer를 비교한다(`>=`만 지원, `^`/`~` 미지원). (b) 없으면 컴포넌트로 간주하는데, `"*"`이면 `dependency_missing`, 아니면 설치 버전(`saba-core`만 앎)과 비교한다. **의존성을 자동으로 활성화하지 않는다.** 성공하면 상태를 저장하고 그 익스텐션의 `daemon.startup`만 발화한다. 응답에 신뢰 표시 `trust`를 싣고, `verified`가 아니면 `warnings[{code:"extension_unverified"\|"extension_modified", extension}]`도 싣는다(알림일 뿐 켜기는 막지 않는다). |
| disable | `POST …/disable` | **실행 중인 인스턴스만** 검사한다(`handlers/extension.rs:166-167`). 역의존성(활성 익스텐션 중 이 id를 dependencies에 둔 것)이 있으면 409 `has_dependents`. 인스턴스의 `extension_data`에서 이 익스텐션의 instance_fields 키 중 하나라도 값이 있으면(true, 비어 있지 않은 문자열·배열·객체, 0이 아닌 숫자) 409 `in_use`(`ext/:873-902`). |
| remove | `DELETE /api/extensions/:id` | **정지된 인스턴스까지** 사용 여부를 검사하고(`handlers/extension.rs:494-495`), 활성 상태면 disable 검사를 거친 뒤 discovered에서 제거하고, 디렉토리를 `remove_dir_all`하고, `extensions.settings` 항목을 삭제한다(`ext/:1023-1056`) |
| mount / unmount | `POST …/mount`, `…/unmount` | 디스크를 건드리지 않고 메모리에서만 등록/해제한다. unmount는 실행 중인 인스턴스만 검사하고 enabled에서도 제거한다 |
| rescan | `POST /api/extensions/rescan` | 위 3.2 |
| ~~force_disable~~ | — | → 해결: 함수가 삭제되었다 |

켜기(`POST /api/extensions/:id/enable`) 실패 본문에는 그 익스텐션 id `extension`이 실린다(`naming_extension`). 설치에서 패키지의 `min_app_version`을 못 맞추면 `install_failed` 대신 `app_version_unsatisfied`(`related[0]` = 요구 버전, `extension`)로 답한다(`stage_package`). 에러 코드 → HTTP 매핑: `handlers/extension.rs:26-59`. not_found/not_mounted/manifest_not_found는 404, dependency_*·component_version_unsatisfied·app_version_unsatisfied는 422, has_dependents/in_use는 409, id_mismatch는 400.

### 3.4 원격 목록 · 설치 · 업데이트

- **출처**: 최신 릴리즈의 서명된 manifest에 든 `ext-<id>` 컴포넌트다. `ExtensionManager::fetch_remote()`가 `relay::latest_catalog`를 부르고 `from_release`가 `meta`(이름·설명·작성자·`min_app_version`·의존성)와 `version`, `download_url`(R2), `sha256`, `asset`을 목록 항목으로 만든다(`ext/:1283-1324`). `manifest_url`, `set_manifest_url`, raw `main/manifest.json`은 없다. 앱보다 새 버전을 요구하면 `incompatible`로 표시한다.
- `GET /api/extensions/manifest`: 원격 목록과 `check_updates_against` 결과를 돌려준다(`handlers/extension.rs:361-382`).
- `GET /api/extensions/updates`: 업데이트 목록만 돌려준다(`handlers/extension.rs:468-487`).
- `POST /api/extensions/:id/install` (body `{download_url?, sha256?}`): body가 없거나 `{}`이면 원격 목록(서명된 카탈로그)에서 URL과 sha256을 찾는다. `download_url`만 주면 그 id의 카탈로그 URL과 같을 때만 카탈로그 sha256으로 받고, 다르면 해시 확인 없이 받지 않고 `sha256_required`로 거부한다. 둘 다 주면 그대로 받되, 카탈로그의 URL·해시와 같을 때만 공식 설치로 기록한다(`install_source`, `handlers/extension.rs:396-422`). 받기·검사·압축 해제는 락 없이 `stage_package`로 하고(sha256(있으면), manifest 스키마, id, `min_app_version` 확인, `ext/:1376-1415`), 폴더 교체만 write 락 안에서 `install_staged`로 한다. 실행 중인 인스턴스가 쓰는 익스텐션이면 `in_use`로 거부하고, 옛 폴더는 새 폴더가 자리 잡은 뒤에 지우며 실패하면 되돌린다(`ext/:695-721,1419-1440`). 빠진 익스텐션 의존성도 차례로 함께 설치한다(`handlers/extension.rs:400-465`). 원격 목록에서 받을 때 그 버전이 설치된 버전보다 낮으면 `older_than_installed`로 거부한다(예전 서명 manifest 재전송 방지).
- 업데이터의 일괄 업데이트(`/api/updates/*`)는 설치된 `ext-*`만 비교하고, 적용 뒤 `rescan()`으로 다시 읽는다(`src/ipc/updates.rs:575-577`).
- 배포: 익스텐션 저장소에는 릴리즈 워크플로도 생성된 `manifest.json`도 없다(CI는 pytest·ruff만, `E:.github/workflows/ci.yml`). 코어 릴리즈 워크플로가 `saba-chan-extensions`의 `main`을 checkout하고 `versions.py manifest --extensions-repo`가 `ext-<id>.zip`을 만들어 같은 릴리즈 manifest에 싣는다(`D:.github/workflows/release.yml:185-215`, `D:scripts/versions.py:246-267`). 지금 저장소에는 `steamcmd`, `ue4_ini` 두 개만 있다.

### 3.5 의존성 처리

- 익스텐션 간 의존성: 현재 두 manifest 모두 `dependencies: []`다. ~~docker → steamcmd 의존성과 `compose_manager`의 파일 경로 import~~ → 해결: docker 익스텐션 제거.
- ~~music: `daemon.startup`에서 npm install, 실제 기능은 `discord_bot/extensions/music.js`~~ → 해결: music 익스텐션과 로컬 Discord 봇이 제거되었다.
- node: 데몬에는 Node 런타임 관리가 없다(L1에서 `node_env` 삭제). 익스텐션 manifest에도 node 의존성을 선언할 방법이 없다.
- Python 의존성: 익스텐션별 requirements 선언이 없다. 공용 venv를 쓴다.

---

## 4. GUI 익스텐션 계약

### 4.1 로딩 경로 (Tauri)

1. Renderer `window.api.extensionList()` → JS 브리지 `saba-chan-gui/src/api/createApi.js:542-549` → Tauri 명령 `daemon_request` → `GET /api/extensions`. 확장 관련 메서드는 `createApi.js:524-646`에 있다. 데몬 요청은 Tauri 셸이 대행한다(토큰 인증 포함, `src-tauri/src/native.rs:17-42`, `daemon.rs:235-254`).
2. `ExtensionProvider`(`src/contexts/ExtensionContext.js:100-215`)가 enabled 익스텐션마다:
   - `window.api.extensionGuiBundle(id)` → `GET /api/extensions/:id/gui`(`responseType:'text'`, `createApi.js:584-598`)로 JS 소스 문자열을 받는다. **Blob URL `<script>`**로 실행한다(`:29-64`). `ext.gui`가 없으면 요청하지 않는다(`:38-40`).
   - 확인되지 않았고 이 PC에서 허락하지 않은 익스텐션(목록의 `gui_allowed:false`, 또는 번들 요청이 403 `extension_gui_unapproved`)은 번들·스타일을 요청하지 않고 한 번 묻는다. 허락하면 `POST /api/sandbox/extension-gui/ext-<id>` 뒤 목록을 다시 받아 로드한다. 거절하면 그 실행에서는 다시 묻지 않고, 설정 > 고급 `ExtensionGuiSection`에서 허락하거나 지운다.
   - 전역 이름 규칙: `window["SabaExt" + PascalCase(id)]`. `-`와 `_` 구분자를 제거하고 다음 글자를 대문자로 바꾼다(`:82-84`). steamcmd → `SabaExtSteamcmd`. 번들이 정확히 이 전역을 만들어야 한다.
   - 스타일: `GET …/gui/styles`가 있으면 `<style data-extension=id>`로 넣는다(`:66-73`).
   - 모듈의 **`registerSlots(): { [slotId]: Component | Component[] }`**를 호출해 슬롯 레지스트리에 병합한다. manifest의 `gui.slots`는 쓰지 않는다.
   - 버전이 바뀌면 전역을 삭제하고 style 태그를 제거한 뒤 다시 로드한다(핫로드, `:164-171`). 로드됨으로 기억했는데 전역이 없어도 다시 로드한다.
   - 실행이 실패한 번들은 자기가 남긴 전역만 되돌린다. 같은 확장을 겹쳐 읽은 다른 로드가 성공해 등록한 전역은 지우지 않는다.
3. React 공유: 호스트가 `window.React`와 `window.ReactDOM`을 전역으로 노출하고(`src/index.js:16-17`), 번들은 `window.React`를 참조한다(external 처리). 그 밖의 호스트 API(토스트, 데몬 API, 스토어 등)는 노출하지 않는다. 확장 컴포넌트는 props만으로 동작해야 한다.
4. 보안: 번들은 렌더러 컨텍스트에서 **그대로 eval**되므로 `window.api`·`window.electron`(브리지의 모든 메서드)과 `window.__TAURI__`(`withGlobalTauri`)에 접근할 수 있다. 즉 번들은 AppContainer 밖에서 데몬 API 전체를 부른다. 셸 쪽 경계는 `daemon_request`의 경로 제한(`/api/…`·`/health`만, 셸이 직접 정하는 `/api/updates/launch-updater`·`/api/client/*`·`/api/daemon/shutdown`은 거부, 04 문서 §1.1)과 capability가 연 명령 목록(`src-tauri/capabilities/default.json`)뿐이다.
   - 그래서 데몬은 번들(`/gui`)과 스타일(`/gui/styles`)을 **확인된(`verified`) 익스텐션이거나 사용자가 이 PC에서 허락한 익스텐션에만** 서빙한다(`extension::gui_allowed`, 플랫폼과 무관). 아니면 403 `{error_code:"extension_gui_unapproved", component:"ext-<id>"}`. manifest의 `gui.bundle`·`gui.styles`는 익스텐션 폴더 기준 상대 경로(`..`·절대 경로 없음)이고, 링크를 거치지 않는 일반 파일이어야 한다. 아니면 404(허락은 폴더 안 파일의 지문에 묶이기 때문이다).
   - `GET /api/extensions` 항목에 GUI 번들이 있으면 `gui_allowed: bool`이 실린다. false면 GUI는 번들을 요청하지 말고 사용자에게 물은 뒤 `POST /api/sandbox/extension-gui/ext-<id>`로 기록한다(원격 출처는 403 `consent_local_only`). 동의는 `config.json` `sandbox.extension_gui`에 익스텐션 폴더 전체의 파일 지문으로 남아, 파일이 하나라도 바뀌면 다시 묻는다. 철회는 `DELETE` 같은 경로.
   - 웹뷰 CSP(04 §1)는 스크립트를 앱 자신과 `blob:`으로만 열고 `unsafe-eval`이 없으므로, 번들 안에서 `eval`·`new Function`을 쓰면 실행되지 않는다.
   - 받아들인 위험: 허락한 번들은 데몬 API 전체(동의 라우트 포함)를 부를 수 있다. 허락은 그 익스텐션을 GUI에서 믿는다는 뜻이다.

현재 `gui`를 가진 익스텐션은 없으므로 이 경로는 실제로 쓰이지 않는다.

### 4.2 슬롯과 props (호스트 `<ExtensionSlot slotId … {...props}>`, `components/ExtensionSlot.js`)

| slotId | 호스트 위치 | 전달 props | 구현 익스텐션 |
|---|---|---|---|
| `ServerCard.badge` | `servers/ServerRow.js:64` | `server` | 없음 |
| `ServerCard.headerGauge` | `servers/ServerDetail.js:201` | `server` | 없음 |
| `ServerCard.provision` | `servers/ServerDetail.js:247` (`server.provisioning`일 때) | `server, provisionProgress, onDismiss, t` | 없음 |
| `ServerCard.expandedStats` | `servers/ServerDetail.js:467` | `server, t` | 없음 |
| `ServerSettings.tab` | `ServerSettingsModal.js:588` | `server, activeTab, setActiveTab, settings, onSettingsChange, t` | 없음 |
| `AddServer.options` | `AddInstanceNewServer.js:326` | `t` | 없음 |

- `server` 객체는 `/api/servers`의 `ServerInfo`다. 여기서 `extension_data`와 `extension_status[<extension_id>]`(list_enrich 결과)를 컴포넌트가 읽는다.
- 슬롯 이름은 호스트에 하드코딩되어 있고 검증하는 장치는 없다. 번들 없이 데이터만 선언하는 slot은 4.3이다.

### 4.3 데이터 slot (manifest `cli.slots`, GUI·CLI 공통)

코드 없이 manifest에 데이터만 적는 slot. 이름은 GUI slot 이름과 같다. 옛 CLI 이름은 별칭으로 받아 새 이름으로 바꾼다(한 릴리스 뒤 제거, `SLOT_ALIASES`).

| 이름 | 옛 이름 | manifest 형태 | 인터페이스가 읽는 곳 |
|---|---|---|---|
| `ServerCard.badge` | `InstanceList.badge` | `{text, condition?}` | `GET /api/servers` 각 서버 `slots["ServerCard.badge"]` = `[{extension, text}]` |
| `ServerDetail.status` | `InstanceDetail.status` | `[{label, value_from}]` | 같은 곳 `slots["ServerDetail.status"]` = `[{extension, label, value}]` |
| `AddServer.options` | `CreateInstance.options` | `[{field, type, label, default?}]` | `GET /api/extensions` 각 익스텐션 `slots["AddServer.options"]` |

- 판정은 데몬이 한 번만 한다(`ExtensionManager::server_slots`, `src/extension/mod.rs`). 켜진 익스텐션만, priority·id 순. `condition`은 `evaluate_condition`(`instance.ext_data.<키>`가 참). `value`는 `extension_status[<ext>][value_from]` → `extension_status.default[value_from]` → `extension_data[value_from]` 순으로 찾고, 없거나 `value_from`이 비밀값 이름(`password`·`token`·`secret`이거나 `_password` 등으로 끝남)이면 그 줄을 뺀다(목록은 원격으로도 나간다). 문자열이 아니면 JSON 문자열로 바꾼다. 항목이 없으면 `slots` 키 자체가 없다.
- `/api/extensions`의 `slots`는 `normalized_slots(cli)`: 통일 이름 키, 값은 늘 배열. 옛 이름과 새 이름을 함께 쓰면 이어 붙인다.
- `AddServer.options`의 답은 `POST /api/instances` 본문 `extension_data: {field: value}`로 보낸다. 데몬이 null이 아닌 값을 새 인스턴스 `extension_data`에 넣는다(객체가 아니면 400 `invalid_format`). 설치 방식이 주는 값이 뒤에서 덮는다.
- CLI(`saba-chan-cli/src/slots.rs`)는 이 결과를 그대로 출력한다. GUI는 같은 자리에서 번들 컴포넌트와 데이터 slot을 함께 그린다.
- ~~`AddServer.options`의 `use_container` 의미가 호스트와 daemon에 하드코딩된 docker 전용 계약이다.~~ → 해결: 슬롯은 `t`만 받고, daemon `create_instance`에 `use_container`가 없다.

### 4.3 i18n

- 로더: `window.api.extensionI18n(id, lang)` → `GET /api/extensions/:id/i18n/:locale` → `i18n.addResourceBundle(lang, "ext_<id>", …)`(`ExtensionContext.js:89-98`). 현재 언어와 `en`을 로드한다. **언어가 바뀌어도 다시 로드하지 않는다**(effect 의존성이 `[extensions]`뿐, `:215`).
- ~~docker 컴포넌트가 호스트 `gui` 네임스페이스의 `server_settings.docker_*` 키를 쓰는 위반 사례~~ → 해결: docker 익스텐션이 제거되었고 `locales/*/gui.json`에 docker 키가 없다.

### 4.4 GUI에서 노출되지 않는 API

- `GET/PUT /api/extensions/:id/config`(글로벌 설정, `config_fields`)에 대한 `window.api` 메서드(`createApi.js`)가 **없다**. CLI `saba-chan-cli/src/client.rs:470-476`에만 있다(`extension config`, PR #6 기준).
- `extension:icon` → SabaStorage(`SabaStorage.js:995`), `extension:initStatus` → `hooks/useExtensionInitStatus.js`.

---

## 5. 익스텐션별 기능 요약 · module과의 연결

### 5.1 docker

~~포터블 Docker Engine, compose 기반 컨테이너 격리, 6개 GUI 슬롯, CLI 슬롯~~ → 해결: docker 익스텐션은 저장소에서 제거되었고, core의 docker 전용 코드도 함께 없어졌다.

### 5.2 steamcmd (v0.1.0)

- `SteamCMD` 클래스(`steamcmd.py:151-622`): 포터블 SteamCMD 탐색/설치, `install(...)`, `update`, buildid·브랜치 비교로 `check_update`. 압축 해제는 경로 탈출을 막는다(`:799-827`).
- daemon hook으로 실제로 쓰이는 것은 `server.check_update` 하나뿐이다. 설치는 module의 `lifecycle.py install_server`가 `from extensions.steamcmd import SteamCMD`로 직접 수행한다(`M:palworld/lifecycle.py:1175`, `M:zomboid/lifecycle.py:1457`).
- instance_fields `install_method_steamcmd`, `steamcmd_app_id` 등은 daemon이 module의 `[install] method`를 보고 **범용 규칙**(`install_method_<방식>`, `<방식>_<이름>`)으로 채운다(`src/instance/mod.rs:142-166` `apply_install_extension_data`). core에 steamcmd 이름은 없다.

### 5.3 music

~~`music_deps.py`와 `package.json`, 재생 로직은 `discord_bot/extensions/music.js`~~ → 해결: music 익스텐션과 로컬 Discord 봇이 제거되었다.

### 5.4 ue4-ini (v0.1.0, 디렉토리명 `ue4_ini`)

- `parse_option_settings(ini_path)`와 `write_option_settings(ini_path, props, section)`를 제공하는 **순수 Python 라이브러리**다(`ue4_ini.py:23,163`). hook은 선언하지 않는다.
- palworld가 `requires_extensions = ["steamcmd", "ue4-ini"]`로 요구한다(`M:palworld/module.toml:51`). ~~최상위에서 무조건 import해서 ue4-ini가 없으면 lifecycle 전체가 ImportError로 실패하고, disable해도 import가 막히지 않는다.~~ → 해결: palworld는 `try/except ImportError`로 import하고 INI를 건드리는 함수만 `extension_required`를 돌려준다(`M:palworld/lifecycle.py:32-37`). disable하면 import 가드가 import를 막는다(2.1).

### 5.5 module → extension 참조 방식

1. `module.toml [install] requires_extensions = [...]`. 모듈을 설치하면(데몬 `POST /api/modules/manifest/:id/install`, 설치기) 요구하는 익스텐션을 받고 켠다(02-modules §5.2). 인스턴스 생성 시 `instance.required_extensions`에 복사된다(`handlers/instance.rs:233-240`). 생성 시점에 검증하고(`:242-272`), 시작 시점에 다시 검증한다(`supervisor/mod.rs:572-630`, `installed_and_enabled_set`). module 기능 엔드포인트에서도 검사한다(`handlers/managed.rs:527-562`). `[module.dependencies]`의 `ext-<id>` 버전 요구도 시작 시 검사한다(`supervisor/mod.rs:519-539`).
2. `module.toml [extensions.<id>]` 섹션은 `metadata.extensions["<id>"]`로 그대로 파싱된다(`supervisor/module_loader.rs:618-622`). settings_changed context의 `module_extensions`로 전달된다.
3. Python 레벨: `from extensions.<pkg> import …`. PYTHONPATH 주입과 import 가드에 의존하고, module은 `SABA_ENABLED_EXTENSIONS`로 자체 검사도 한다(`M:palworld/lifecycle.py:1165`, `M:zomboid/lifecycle.py:1447`).
4. ~~`__example__lifecycle.py`의 `from extensions.steamcmd import install_game`은 존재하지 않는 함수다.~~ → 해결: 예제는 `from extensions.steamcmd import SteamCMD`를 쓴다(`M:__example/__example__lifecycle.py:405`).

---

## 6. ext_process: 장기 실행 외부 프로세스 계약

~~`src/ipc/handlers/ext_process.rs`, `/api/ext-process/*`: Discord 봇 실행용 범용 프로세스 매니저~~ → 해결: 로컬 Discord 봇 제거와 함께 `ext_process` 핸들러와 라우트가 없어졌다(`src/ipc/mod.rs` 라우트 목록에 없음). Discord 연동은 `relay_agent.rs`가 릴레이와 통신한다.

---

## 7. 불일치 · dead code · 스텁 · 원칙 위반

### 7.1 Dead code / 미구현

1. ~~`HookBinding.is_async`: 읽는 곳이 없다.~~ → 해결: 필드 삭제 (`ext/:299-305`).
2. ~~`min_app_version`: 강제하지 않는다.~~ → 해결: enable·설치에서 강제하고 목록에 `incompatible`로 표시한다(`ext/:929-933,1405,1108`).
3. ~~`should_parse_config_section`, `all_instance_fields`, `force_disable`, `enabled_set`, `is_newer_version`, `replace_extension_config`와 `impl` 전체 `#[allow(dead_code)]`~~ → 해결: 함수들이 삭제되었다. 남은 dead는 읽는 코드가 없는 `module_config_section` 필드(`ext/:254-256`)다.
4. ~~`install_from_url`의 sha256 검증: 스텁이다.~~ → 해결: `stage_package`가 sha256을 비교한다(`ext/:1383-1389`).
5. ~~`ExtensionUpdateInfo.downloaded/installed`: 항상 false다.~~ → 해결: 필드 삭제, 대신 `incompatible`(`ext/:524-531`).
6. ~~`plugin::run_extension*`: 옛 레이아웃이다.~~ → 해결: 삭제.
7. ~~updater의 `extension.toml` 기반 익스텐션 업데이트~~ → 해결: `manifest.json`의 id·version을 읽는다(`updater/src/lib.rs:274-277,608-620`).
8. ~~발화되지 않는 hook 7종과 그 Python 함수들~~ → 해결: manifest에서 빠졌고 검증이 막는다. ~~`server.post_create`는 `FIRED_HOOKS`에 있지만 발화하는 코드가 없다~~ → 해결: 인스턴스 저장 직후 백그라운드로 발화한다(`handlers/instance.rs:357-370`). `FIRED_HOOKS`의 이름마다 `src/` 어딘가에서 발화하는지 테스트가 검사한다(`ext/:1632-1649`).
9. `ue4_ini_enabled` instance_field: 설정하는 경로가 없다.

### 7.2 버그 · 위험

1. ~~`SABA_ENABLED_EXTENSIONS` 상태 파일 경로 불일치~~ → 해결: config.json `extensions.enabled`에서 읽는다(`plg/:200-205`).
2. ~~hook 실행 순서가 HashMap에 의존해 비결정적이다.~~ → 해결: `priority`, id 순(`ext/:1116-1127`).
3. ~~`dispatch_hook_with_progress`는 익스텐션 1개만 실행한다.~~ → 해결: 함수 삭제.
4. ~~enable할 때 모든 익스텐션의 `daemon.startup`이 재실행된다.~~ → 해결: `run_hook_to`로 해당 익스텐션만(`handlers/extension.rs:122-123`).
5. ~~긴 hook 동안 `extension_manager` read 락을 유지한다.~~ → 해결: 계획만 락 안에서, 실행은 락 밖에서(`ext/:1603-1617`).
6. `daemon.startup` 120초 타임아웃(`main.rs:388` → `run_hook`)이 긴 설치 작업에는 짧을 수 있다.
7. ~~`server.post_stop`이라는 이름과 실제 의미가 다르다.~~ → 해결: `server.stop`으로 바꾸고 옛 이름은 별칭으로 읽는다(`ext/:15-17`, `supervisor/mod.rs:908-911`).
8. `server.list_enrich`는 인스턴스마다 순차로 Python을 실행한다(10초 × N). TTL 30초 캐시에 `handled:false`도 캐싱한다(`handlers/server.rs:119-128`).
9. ~~`instances_using_ext`가 숫자 0, 빈 문자열도 "사용 중"으로 판정하고, 정지된 인스턴스가 있어도 remove할 수 있다.~~ → 해결: 빈 값은 미사용으로 보고(`ext/:891-898`), remove는 정지된 인스턴스까지 검사한다(`handlers/extension.rs:494-495`). disable은 여전히 실행 중인 인스턴스만 보므로, 정지된 인스턴스가 쓰는 익스텐션을 끈 뒤 시작하면 `extension_required` 에러가 난다.
10. ~~`save_extension_config` 주석은 "null을 전달하면 키 제거"라고 하지만, 실제로는 null 값을 저장한다.~~ → 해결: null인 키는 지워 기본값으로 돌린다(`ext/:1590-1606`). `get_extension_config` 핸들러 주석은 아직 `extensionConfig.json`이라고 적혀 있다(`:573`).
11. ~~컴포넌트 의존성(`saba-core` 등)을 선언하면 HTTP enable이 불가능하다.~~ → 해결: `saba-core`는 데몬 버전으로 검사한다(`ext/:905-908`). 다른 컴포넌트 키는 여전히 버전을 쓰면 켤 수 없다.
12. ~~install/update가 실행 중이거나 enabled인 익스텐션 디렉토리를 검사 없이 삭제한다. 무결성 검증·의존성 자동 설치가 없다.~~ → 해결: 실행 중 사용이면 거부, staging 후 교체·롤백, sha256 확인, 빠진 의존성 자동 설치(`ext/:1376-1440`, `handlers/extension.rs:460-462`).
13. ~~`discover()`/`rescan()`이 manifest.id와 디렉토리명의 일치를 검사하지 않고, zip 추출은 기존 폴더가 있으면 새 zip을 버린다.~~ → 해결: 스캔이 이름을 검사하고(`ext/:763-769`), zip은 기존 폴더를 바꿔 끼운다(`ext/:652-683`).
14. ~~모든 hook context에 전체 `extensions.settings`(타 익스텐션 비밀값 포함)가 주입된다.~~ → 해결: 익스텐션마다 자기 설정만 넣는다(`ext/:1303-1312`).
15. GUI 번들을 Blob script로 무검증 실행하고, `window.api`에 전체 접근할 수 있다.
16. GUI i18n이 언어 변경 시 다시 로드되지 않는다(`ExtensionContext.js:215`).
17. ~~리포에 런타임 산출물(`dockerd.pid`, `.deps-resolved.json`, Go 바이너리)이 커밋되어 있다.~~ → 해결: 해당 익스텐션이 제거되었고, 저장소에는 소스·테스트·`icon.png`만 있다.
18. ~~릴리즈 워크플로가 기존 릴리즈를 모두 삭제한다.~~ → 해결: 익스텐션 저장소의 릴리즈 워크플로가 없어졌다(extensions 301ff2c). 배포는 코어 릴리즈가 맡는다(3.4).
19. ~~CLI 도움말은 `extension [...|registry|...]`라고 하지만 실제 서브커맨드는 `manifest`다~~ → 해결: PR #6 CLI의 하위 명령 목록과 도움말은 clap 정의에서 나오므로 어긋나지 않는다. 카탈로그 조회는 `extension available`이다(`saba-chan-cli/src/cli.rs:557-558`, `cmd/extension.rs:30,251`).
20. ~~`server.post_create`가 `FIRED_HOOKS`에 있어 manifest 검증을 통과하지만 실제로는 발화되지 않는다~~ → 해결: 생성 시 발화한다(7.1-8, `handlers/instance.rs:357-370`).

### 7.3 "daemon이 주인 / 플러그인은 자유롭게 커스터마이즈" 원칙 위반 (하드코딩된 플러그인 지식)

Core(daemon/GUI/CLI)가 특정 익스텐션을 이름으로 아는 곳들이다. 플러그인을 자유롭게 교체하거나 추가할 수 없게 만든다.

| 위치 | 하드코딩 내용 |
|---|---|
| ~~`supervisor/mod.rs` `ext_enabled("docker_enabled")`~~ | → 해결: 삭제 |
| ~~`handlers/server.rs` stop 후 `docker_enabled` untrack 생략~~ | → 해결: 삭제 |
| ~~`handlers/instance.rs` `use_container` → `docker_enabled` 주입~~ | → 해결: 삭제 |
| ~~`handlers/instance.rs` `metadata.extensions.get("docker")`~~ | → 해결: 모듈의 `[extensions.*]` 전체를 `module_extensions`로 넘긴다(`handlers/instance.rs:1313-1323`) |
| ~~`supervisor/module_loader.rs` `[docker]` 전용 구조체~~ | → 해결: 범용 `[extensions.<id>]`(`module_loader.rs:618-622`) |
| ~~`src/instance/mod.rs` steamcmd instance_field 이름~~ | → 해결: `install_method_<방식>`, `<방식>_<이름>` 범용 규칙(`src/instance/mod.rs:142-166`) |
| ~~`handlers/instance.rs` check_update에서 `steamcmd_app_id` 읽기~~ | → 해결: `<방식>_app_id`(`handlers/instance.rs:1433-1437`) |
| ~~GUI `DiscordBotModal.js` `e.id==='music'`~~ | → 해결: 삭제 |
| ~~GUI `AddInstanceNewServer.js` `options.use_container` 계약~~ | → 해결: 슬롯이 `t`만 받는다(`AddInstanceNewServer.js:326`) |
| ~~호스트 `locales/*/gui.json` docker UI 문자열~~ | → 해결: 삭제 |
| ~~`discord_bot/extensions/music.js`~~ | → 해결: 봇 제거 |
| ~~익스텐션 → 익스텐션 파일 경로 import (docker → steamcmd)~~ | → 해결: docker 제거 |
| module → extension | module이 `from extensions.x import`로 직접 의존한다. ~~disable해도 import는 막히지 않는다~~ → 해결: import 가드가 비활성 익스텐션 import를 막는다(`plg/:84-96,211-214`) |

daemon을 우회하는 interface 동작:
- ~~GUI가 봇 실행 command와 env를 조립해 `/api/ext-process/discord-bot/start`로 보낸다.~~ → 해결: ext_process와 로컬 봇이 없어졌다.
- ~~CLI는 `get_extensions_path()`로 익스텐션 경로를 자체 해석한다~~ → 해결: PR #6 CLI에는 경로 해석이 없고 익스텐션 목록은 `GET /api/extensions`로만 받는다(`saba-chan-cli/src/resolve.rs:212-220`).

### 7.4 GUI/CLI 슬롯 체계 비대칭

- GUI: 번들의 `registerSlots()`가 기준이고 manifest의 `gui.slots`는 무시한다.
- ~~CLI: manifest의 `cli.slots`만 읽고 이름이 GUI와 다르며, `condition` 판정을 CLI `condition_holds`가 따로 한다~~ → 해결(Q-7): 데이터 slot은 GUI 이름으로 통일했고 판정은 데몬이 한 번만 한다(4.3). 예전 `InstanceSettings.fields` 자동 파생과 `EXT_SLOTS_INIT` 라인 프로토콜은 없다.
