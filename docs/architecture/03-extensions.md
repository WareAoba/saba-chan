# 03. 익스텐션(= 플러그인) 시스템 — 코드 기준 분석

> 기준: saba-chan 저장소 (daemon, GUI, CLI, discord_bot) + saba-chan-extensions 저장소 (docker, steamcmd, music, ue4_ini).
> 모든 내용은 코드에서 직접 추출했다. README / docs / copilot-instructions는 참고하지 않았다(오래된 문서로 간주).
> 경로 약어: `ext/` = `saba-chan/src/extension/mod.rs`, `plg/` = `saba-chan/src/plugin/mod.rs`, `EXT/` = `saba-chan-extensions/`.

---

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
| `crate::plugin` (`src/plugin/mod.rs`) | **개념이 아니라 실행기**다. "Python 파일 하나를 `python <file> <function>`로 띄우고 stdin JSON → stdout JSON을 주고받는 러너"일 뿐이다. **module의 `lifecycle.py`와 extension hook이 둘 다** 이 러너를 쓴다. | `plg/:77-258`. module 호출 예: `supervisor/mod.rs:489,717,830…`. extension 호출: `ext/:1174,1277` |
| `crate::extension` (`src/extension/mod.rs`) | 오너가 말하는 "plugin". manifest 기반이고, hook, GUI/CLI 슬롯, instance_fields, config_fields를 가진다. | `ext/:172-209` |
| `ipc::ExtensionInfo` / `ExtensionListResponse { modules }` | 이름은 extension이지만 **실제로는 module 정보**다(명명 혼동). | `src/ipc/mod.rs:261-290`, `handlers/server.rs:10,274` |
| GUI `selectedExtension` | AddServer 모달에서 **module 이름**을 담는 상태다. | `saba-chan-gui/src/components/Modals/AddInstanceNewServer.js:24,79` |
| `discord_bot/extensions/*.js` | Discord 봇 내부 기능 파일(music.js, rps.js 등)이다. daemon extension 시스템과 **별개인 제3의 "extension" 개념**이다. | `saba-chan/discord_bot/extensions/` |
| updater의 extension 업데이트 | `extension.toml` + `[update].github_repo`를 찾는 **또 다른 체계**다. 실제 익스텐션에는 `extension.toml`이 없다. | `updater/src/lib.rs:835-860,1100-1115` |

**결론**: 코드에서 "plugin"과 "extension"은 서로 다른 계층이다. `plugin`은 Python 프로세스 실행 프로토콜이고, `extension`은 그 위의 확장 시스템이다. 오너가 말하는 "plugin"은 코드의 `extension`에 해당하고, 코드의 `plugin` 모듈은 module과 extension이 공유하는 하위 런타임이다. `plg/:263-284`의 `run_extension`/`run_extension_with_progress`는 `extensions/<name>.py` 단일 파일이라는 옛 레이아웃을 가정한 `#[allow(dead_code)]` 잔재다.

---

## 1. Extension manifest.json 스키마 (Rust 역직렬화 기준)

### 1.1 `ExtensionManifest` (`ext/:172-209`)

| key | Rust 타입 | 필수/기본값 | 코드에서 실제 사용 여부 |
|---|---|---|---|
| `id` | `String` | **필수** | 사용. discovered 맵의 키. `mount()`는 `manifest.id == ext_id`를 검사한다(`ext/:760`). `discover()`/`rescan()`은 디렉토리명과 비교하지 않는다. |
| `name` | `String` | **필수** | 목록과 로그 |
| `version` | `String` | **필수** | 업데이트 비교(SemVer, 파싱 실패 시 문자열 비교 `ext/:1399`), 익스텐션 간 의존성 버전 검사 |
| `description` | `String` | `""` | 목록 |
| `author` | `String` | `""` | 목록 |
| `min_app_version` | `Option<String>` | `None` | **역직렬화만 하고 아무 데도 쓰지 않는다** (grep 결과 0건) |
| `dependencies` | `HashMap<String,String>` | `{}` | 커스텀 디시리얼라이저 `deserialize_dependencies` (`ext/:17-58`): 배열 `["a"]` → `{"a":"*"}`, 맵은 그대로 받는다. enable/disable/unmount 검증에 쓰인다(아래 3.3). |
| `python_modules` | `HashMap<String,String>` | `{}` | 모듈 키 → 익스텐션 디렉토리 기준 상대경로. hook 바인딩이 참조한다(`ext/:1145`). |
| `hooks` | `HashMap<String, HookBinding>` | `{}` | hook 이름 → 바인딩 |
| `gui` | `Option<GuiManifest>` | `None` | 번들/스타일 서빙에 쓰인다(`handlers/extension.rs:217-275`). |
| `cli` | `Option<CliManifest>` | `None` | `/api/extensions` 응답에 그대로 실린다. CLI TUI가 해석한다. |
| `module_config_section` | `Option<String>` | `None` | `should_parse_config_section()` (`ext/:1309`)만 읽는데, **이 함수 호출처가 0건이다(dead)**. module.toml의 `[docker]` 섹션은 module_loader가 하드코딩으로 파싱한다(`supervisor/module_loader.rs:267,693`). |
| `instance_fields` | `HashMap<String, FieldDef>` | `{}` | (a) disable/unmount 시 "사용 중" 판정(`ext/:822-853`), (b) 목록 응답 → CLI가 자동 설정 슬롯을 만든다. `all_instance_fields()` (`ext/:1317`)는 호출처 0건이다. **default 값을 인스턴스에 주입하는 코드는 없다.** |
| `config_fields` | `HashMap<String, FieldDef>` | `{}` | 목록 응답에만 실린다. 값 저장은 `extensionConfig.json`. 스키마 검증은 없다. |
| `i18n_dir` | `Option<String>` | `None` | `load_i18n()` → `<dir>/<i18n_dir>/<locale>.json` (`ext/:1501-1510`) |

### 1.2 하위 구조체

- **`HookBinding`** (`ext/:211-219`): `module: String`(필수, `python_modules`의 키), `function: String`(필수), `condition: Option<String>`, `async`(→`is_async: Option<bool>`).
  - **`async`는 읽는 코드가 0건이다.** 주석에는 "true면 tokio::spawn"이라고 되어 있지만 구현되지 않았다. post_create가 비동기로 도는 이유는 `handlers/instance.rs:318`이 직접 `tokio::spawn`하기 때문이다.
- **`GuiManifest`** (`ext/:221-231`): `bundle`, `styles`, `builtin: Option<bool>`, `slots: HashMap<String,String>`.
  - `builtin`은 daemon에서 읽지 않는다.
  - `slots`도 **GUI에서 읽지 않는다.** GUI는 번들의 `registerSlots()` 반환값만 쓴다(4절).
- **`CliManifest`** (`ext/:234-240`): `slots: HashMap<String, Value>`. 형태가 자유로운 JSON이다.
- **`FieldDef`** (`ext/:242-250`): `type`(필수), `default: Option<Value>`, `optional: Option<bool>`.

알 수 없는 키는 serde 기본 동작대로 조용히 무시된다(`deny_unknown_fields` 없음).

### 1.3 실제 manifest에는 있는데 코드가 무시하는 것

| 키 / 값 | 위치 | 상태 |
|---|---|---|
| `min_app_version: "0.1.0"` | 4개 manifest 전부 | 강제하지 않음 |
| `"async": true` | `EXT/docker/manifest.json` server.post_create | 무시됨 |
| `gui.slots` (docker 6개) | docker | GUI가 무시함(번들 `registerSlots()`와 내용이 같아 중복 정의) |
| `gui.builtin: true`, `gui.slots: {}` | music | daemon/GUI 모두 의미 없음. `gui`가 `Some`이라 `/api/extensions/music/gui`는 bundle이 없어 404를 반환한다. GUI `loadExtensionBundle`은 `ext.gui`가 truthy이면 요청을 보낸다(`ExtensionContext.js:32-38`). |
| `module_config_section: "docker"` | docker | dead 함수만 참조 |
| `config_fields.discord_token`, `default_volume` | music | 저장 API는 있지만 GUI에 IPC 바인딩이 없다. 봇은 `bot-config.json`을 쓴다. 사실상 미사용. |
| `"i18n_dir": null` | music | OK(None) |
| `condition: null` | docker/music daemon.* | OK(None) |
| hook 이름 `server.pre_create`, `server.stats`, `server.logs`, `server.install`, `server.update`, `module.parse_settings`, `module.write_settings` | docker/steamcmd/ue4-ini | **daemon이 발화하지 않는다**(2.3절) |

### 1.4 코드는 지원하는데 실제 manifest에서 안 쓰는 것

- `gui.styles`: CSS 서빙 경로(`handlers/extension.rs:249`)가 있지만 docker에 styles 항목이 없다.
- `dependencies`의 비-익스텐션 컴포넌트 키(`saba-core`, `gui` 등, `ext/:899-923`): 실제 manifest에서는 쓰지 않는다. 게다가 HTTP enable 경로는 `enable()` → `enable_with_versions(&HashMap::new())`(`ext/:856-858`)를 부르므로, **컴포넌트 의존성을 선언하면 HTTP로는 절대 활성화할 수 없다**(테스트 `ext/:2543-2560`이 이 동작을 확인한다). `enable_with_versions`를 버전 맵과 함께 부르는 프로덕션 호출처는 0건이다.
- `condition` 패턴은 `instance.ext_data.<key>` 하나뿐이다(`ext/:1070-1086`). 그 외 패턴은 warn을 남기고 false로 처리한다.

### 1.5 원격 레지스트리 스키마 (`RemoteExtensionInfo`, `ext/:292-354`)

```
{ "schema_version": u32(default 0), "generated_at": Option<String>,
  "extensions": { "<id>": { name*, version*, description, author, download_url, sha256?,
                            min_app_version?, asset?, install_dir?, dependencies(배열|맵),
                            has_gui, has_i18n, tags[], homepage? } } }
```

`id`는 맵 키에서 주입된다(`into_list`, `ext/:358-366`). 실제 `EXT/manifest.json`은 `build_extensions.py`가 생성하며 `tags`/`homepage`는 없다. `install_dir: "extensions/ue4-ini"`이지만 실제 설치 디렉토리는 `ue4_ini`다(하이픈→언더스코어, `ext/:462,1464`). `install_dir`과 `asset`은 daemon이 읽지 않는다.

---

## 2. Hook 시스템

### 2.1 Python 호출 프로토콜 (`plg/:115-258`)

- **명령**: `<venv python> <절대경로 module.py> <function>`. Python 경로는 `python_env::get_python_path()`(`plg/:125`)에서 얻는다. 매 호출마다 새 프로세스를 띄우는 short-lived 방식이다.
- **stdin**: context JSON 한 덩어리. 쓴 다음 바로 닫는다(`plg/:180-183`).
- **stdout**: 전체를 하나의 JSON으로 파싱한다. 파싱에 실패하면 Err(`plg/:233-241`). 종료 코드가 0이 아니어도 Err(`plg/:228-231`).
- **stderr**: 줄 단위로 스트리밍한다. `PROGRESS:{json}` 줄은 `ExtensionProgress{percent,message,step,total,label,steps}`(`plg/:9-20`)로 파싱해 콜백을 호출한다. 나머지 줄은 debug 로그로 남긴다.
- **env**:
  - `PYTHONIOENCODING=utf-8`, `PYTHONUNBUFFERED=1`
  - `PYTHONPATH = <extensions_dir의 부모>:<modules_dir>:<기존 PYTHONPATH>` (`plg/:153-173`). extensions 디렉토리의 **부모**를 넣으므로 `import extensions.steamcmd`가 동작한다. 단, 디렉토리명이 실제로 `extensions`일 때만 그렇다. `SABA_EXTENSIONS_DIR`을 다른 이름으로 지정하면 import가 깨진다.
  - `SABA_ENABLED_EXTENSIONS=<콤마 목록>`: **버그가 있다.** `APPDATA`가 있으면 `%APPDATA%/saba-chan/extensions_state.json`, 없으면 **cwd 기준 `./extensions_state.json`**을 읽는다(`plg/:140-151`). 실제 상태 파일 위치는 `constants::resolve_extensions_state_path()` = `resolve_data_dir()/extensions_state.json`(`updater/src/constants.rs:219`)이고, Linux에서는 `~/.config/saba-chan/`이다. 결과적으로 **Linux/macOS에서는 이 env가 대개 설정되지 않는다.** `SABA_DATA_DIR` 오버라이드도 무시된다. palworld/zomboid `lifecycle.py`가 이 env로 steamcmd 활성 여부를 판단하므로(`saba-chan-modules/palworld/lifecycle.py:1244`, `zomboid/lifecycle.py:1525`) Linux에서는 설치가 "extension_not_enabled"로 실패할 수 있다.
- **타임아웃**: 기본 `DEFAULT_PLUGIN_TIMEOUT_SECS = 120`(`plg/:55`). 초과하면 `child.kill()` 후 Err.

### 2.2 디스패처 (`ext/:1055-1306`)

- `hooks_for(name)`(`ext/:1056`): **enabled**이면서 해당 hook을 가진 익스텐션 목록. `discovered`가 `HashMap`이라 **실행 순서가 비결정적**이다. chain-of-responsibility의 우선순위를 정할 수 없다.
- 공통 처리:
  1. context에 `extension_config`(전체 `extensionConfig.json` 맵, 모든 익스텐션 것)를 주입한다(`ext/:1090-1100`). 다른 익스텐션의 설정(예: music의 `discord_token`)까지 모든 hook 프로세스에 노출된다.
  2. `context.extension_data`로 `condition`을 평가한다(bool, 0이 아닌 숫자, 비어 있지 않은 문자열이면 true).
  3. `python_modules[binding.module]` 경로로 실행한다.
  4. 결과에 `handled == true`가 있으면 **나머지 익스텐션을 건너뛴다**(`ext/:1193-1204`). 에러가 나면 계속 진행한다(graceful).
- `dispatch_hook` = `dispatch_hook_timed(…, 120)`(`ext/:1106-1112`).
- `dispatch_hook_with_progress`(`ext/:1226-1306`): 타임아웃 1800초. **첫 번째로 조건을 통과한 익스텐션 하나만 실행하고 무조건 `break`한다**(`ext/:1302`). 주석은 "progress 콜백이 1회 소비"라고 설명하지만, 실제로는 hook 자체가 1개만 실행된다.
- 결과 병합: 디스패처는 병합하지 않고 `Vec<(ext_id, Result<Value>)>`를 반환한다. 해석은 호출부마다 다르다(아래 표).
- 모든 호출부가 `extension_manager.read()` 락을 **Python 실행이 끝날 때까지** 쥐고 있다. 특히 `daemon.startup`(npm install, Docker 엔진 설치)과 post_create(최대 30분) 동안 write 락이 필요한 enable/disable/install/rescan이 블록된다.

### 2.3 daemon이 발화하는 hook 전수 목록

| hook | 발화 위치 | 트리거 / 경로 | 동기성 · 타임아웃 | context 키 | 결과 사용 |
|---|---|---|---|---|---|
| `daemon.startup` | `src/main.rs:162-178` | daemon 기동 직후 | `tokio::spawn` 백그라운드, 120s | `{}` (+extension_config) | 결과마다 `success`(기본 true), `message`, `error` → `ExtensionInitTracker`(`/api/extensions/init-status`) |
| `daemon.startup` (재발화) | `handlers/extension.rs:84-123` | `POST /api/extensions/:id/enable` 성공 후 | spawn, 120s | `{}` | **활성화된 모든 익스텐션의 startup hook을 다시 실행한 뒤** 결과만 해당 id로 필터링한다(`:101`). 다른 익스텐션의 startup이 중복 실행된다. |
| `daemon.shutdown` | `src/main.rs:425` | Ctrl-C 시그널 핸들러 | 동기(await), 120s | `instances[] {id,name,module,extension_data,instance_dir}` | `handled` 여부를 로그로만 남긴다 |
| `server.pre_start` | `supervisor/mod.rs:324` (non-managed start), `:1032` (managed start) | 서버 시작 | 동기, 120s | `instance_id, instance_dir, module, extension_data` (+managed 쪽에 `instance, config, managed:true`) | `handled:true` + `success!=true` → 에러를 반환한다. `success:true`이면 `log_follower{program,args,working_dir,description,interactive,strip_prefix,env_vars}`로 `ManagedProcess::spawn`(interactive) 또는 `spawn_log_follower`를 실행한다(`:353-420`, `:1066-1124`). 그 결과를 그대로 반환하고 기본 시작 로직을 건너뛴다. |
| `server.post_stop` | `supervisor/mod.rs:566` | `stop_server` **진입 직후**(이름은 post지만 실제로는 정지를 **대행**한다) | 동기, 120s | 위와 동일 | `handled:true`이면 cooldown을 기록하고 그대로 반환한다 |
| `server.status` | `supervisor/mod.rs:759` | `GET /api/server/:name/status` | 동기, 120s | + `process_patterns` | `handled:true`이면 그대로 반환한다 |
| `server.list_enrich` | `handlers/server.rs:113` | `GET /api/servers`, `extension_data`가 비어 있지 않은 인스턴스마다 **순차** 실행 | 동기, **10s**, 결과를 30s TTL 캐시(`ipc/mod.rs:559`) | `instance_id, instance_name, module, extension_data, instance_dir, process_patterns` | `handled`와 무관하게 첫 번째 Ok를 캐시한다. `handled:true`이면 `status`와 `extension_status[<res.extension_id or "default">]`를 채운다 |
| `server.post_create` | `handlers/instance.rs:320` | 인스턴스 생성, **`use_container==true`일 때만**(`:255`) | spawn, progress, **1800s** | `instance_id, instance_name, module, use_container, instance_dir, extension_data, instance, module_install{install, container}` | `PROGRESS` → `ProvisionTracker`. `Err`이거나 `success:false`이면 error로 기록한다 |
| `server.pre_delete` | `handlers/instance.rs:828` | 인스턴스 삭제 | 동기, 120s | `instance_id, instance_name, module, extension_data` | `handled`이면 2초 대기한다(파일 잠금 해제용) |
| `server.settings_changed` | `handlers/instance.rs:1225` | 인스턴스 설정 PATCH, `extension_data`가 비어 있지 않을 때 | 동기, 120s | `instance_id, instance_dir, instance, module_extensions, extension_data, settings` | 결과를 버린다(`_results`) |
| `server.check_update` | `handlers/instance.rs:1305` | `GET /api/instance/:id/check-update` | 동기, 120s | `instance_id, instance_name, module, instance_dir, install_dir, app_id, extension_data` | 첫 번째 Ok를 그대로 응답한다. 없으면 `{update_available:false, reason:"no_hook_handled"}` |

**manifest에 선언됐지만 daemon이 발화하지 않는 hook**: `server.pre_create`(docker), `server.stats`(docker), `server.logs`(docker), `server.install`/`server.update`(steamcmd), `module.parse_settings`/`module.write_settings`(ue4-ini). grep 결과 문자열 자체가 daemon 코드에 없다.

- 업데이트 적용(`POST /api/instance/:id/apply-update`, `handlers/instance.rs:1329-1500`)과 네이티브 프로비저닝(`:440-560`)은 hook을 거치지 않는다. **module의 `lifecycle.py install_server`를 직접 호출**하고, module이 `from extensions.steamcmd import SteamCMD`로 익스텐션을 Python 라이브러리처럼 import한다.

### 2.4 hook × extension 매트릭스

● = 선언했고 daemon이 발화함, ○ = 선언했지만 **발화되지 않음**, 빈칸 = 미선언

| hook | docker | steamcmd | music | ue4-ini |
|---|---|---|---|---|
| daemon.startup | ● `docker_engine.ensure` | | ● `music_deps.check_dependencies` | |
| daemon.shutdown | ● `compose_manager.shutdown_all` | | | |
| server.pre_create | ○ `pre_create` (no-op, `handled:false`, `compose_manager.py:736-745`) | | | |
| server.post_create | ● `provision` (`async:true`는 무시됨) | | | |
| server.pre_start | ● `start` | | | |
| server.post_stop | ● `stop` | | | |
| server.status | ● `status` | | | |
| server.stats | ○ `container_stats` | | | |
| server.pre_delete | ● `cleanup` | | | |
| server.settings_changed | ● `regenerate_compose` | | | |
| server.list_enrich | ● `enrich_server_info` | | | |
| server.logs | ○ `get_logs` | | | |
| server.install | | ○ `install` | | |
| server.update | | ○ `install` | | |
| server.check_update | | ● `check_update` | | |
| module.parse_settings | | | | ○ `parse_option_settings` |
| module.write_settings | | | | ○ `write_option_settings` |

모든 docker 서버 hook의 condition은 `instance.ext_data.docker_enabled`, steamcmd는 `install_method_steamcmd`, ue4-ini는 `ue4_ini_enabled`다. `ue4_ini_enabled`를 설정하는 코드는 어디에도 없다.

### 2.5 Python 쪽 엔트리포인트 호환성

| 파일 | `__main__` 디스패치 | daemon context와 맞는지 |
|---|---|---|
| `EXT/docker/compose_manager.py:1160-1204` | `_FUNCTIONS[argv[1]](config)`. `config.wsl2_mode`가 있으면 WSL2 모드를 오버라이드한다 | 맞음 |
| `EXT/docker/docker_engine.py:962-993` | `FUNCTIONS = ensure,start_daemon,stop_daemon,status,info`. 예외는 `{success:false,message}`로 바꾼다. BOM을 제거한다 | daemon.startup ctx가 `{}`라서 기본 설정으로 동작한다. **엔진 다운로드가 120초를 넘으면 daemon이 kill한다**(provision 경로에서 직접 부를 때는 timeout 300을 넘기지만 hook 경로에는 없다) |
| `EXT/steamcmd/steamcmd.py:741-833` | `FUNCTIONS = ensure,install,update,status,check_update` | `check_update`는 `app_id, install_dir, branch`를 읽는다. daemon은 `app_id, install_dir`을 준다. OK |
| `EXT/music/music_deps.py:375-400` | `func(**config)` (kwargs 전개) | ctx `{}`+`extension_config` → `check_dependencies(**_kwargs)`. 반환값에 `success`/`message`가 없어서 daemon은 항상 성공 "OK"로 기록하고 `status:"degraded"`를 무시한다(`main.rs:182-183`) |
| `EXT/ue4_ini/ue4_ini.py` | **`__main__`가 없다.** 함수 시그니처도 `(ini_path, …)`라 hook 프로토콜과 맞지 않는다 | hook으로 실행하면 stdout이 비어 JSON 파싱 에러가 난다. 실제로는 module이 `from extensions.ue4_ini import …`로 쓰는 **순수 라이브러리**다 |

---

## 3. 익스텐션 수명주기

### 3.1 저장 위치

- 익스텐션 디렉토리: `resolve_extensions_dir()` = `$SABA_EXTENSIONS_DIR` 또는 `resolve_data_dir()/extensions` (`updater/src/constants.rs:136-143`). data_dir은 `$SABA_DATA_DIR`, Windows는 `%APPDATA%/saba-chan`, Unix는 `~/.config/saba-chan`이다(`:89-108`).
- 활성 상태: `<data_dir>/extensions_state.json`. 형식은 enabled id 배열이다(`ext/:1607-1664`).
- 익스텐션 글로벌 설정: `<data_dir>/extensionConfig.json`. 형식은 `{ext_id:{k:v}}`(`ext/:1547-1604`).
- 테스트 생성자 `new_isolated`는 위 두 파일을 extensions 디렉토리 안에 둔다(`ext/:420-424`).

### 3.2 발견 (discover / rescan / mount)

- `discover()`(`ext/:499-595`): 하위 디렉토리 중 `manifest.json`이 있는 것을 등록한다. 루트에 있는 `*.zip`은 `python_safe_dir_name(stem)` 폴더로 풀고 zip을 삭제한다(`ext/:599-646`). 폴더가 이미 있으면 **zip만 삭제하고 덮어쓰지 않는다**. **manifest.id와 디렉토리명의 일치는 검사하지 않는다.**
- `rescan()`(`ext/:696-745`): 기존 항목의 manifest를 다시 읽고 새 항목을 추가한다. 디스크에서 사라진 항목은 그대로 남긴다.
- `mount(id)`(`ext/:749-770`): `ext_dir_on_disk`(id → id 그대로 → 하이픈을 언더스코어로 바꾼 이름)로 디렉토리를 찾고, `manifest.id == id`를 검사한다.
- `IPCServer::new`(`src/ipc/mod.rs:534-542`)에서 discover를 한 번 실행한다. 상태 파일에 enabled로 되어 있지만 디스크에 없는 익스텐션은 `enabled_but_not_installed()`로 구분한다(`ext/:1021`).

### 3.3 enable / disable / remove / unmount

| 작업 | API | 동작 |
|---|---|---|
| enable | `POST /api/extensions/:id/enable` (`handlers/extension.rs:72`) | `enable_with_versions(id, {})`(`ext/:865-930`). dependencies 맵의 각 키를 확인한다. (a) discovered에 있는 익스텐션이면 **enabled 상태여야 하고**, `">=x"`이면 SemVer를 비교한다(`>=`만 지원, `^`/`~` 미지원). (b) 없으면 컴포넌트로 간주하는데, `"*"`이면 `dependency_missing`, 아니면 installed_versions가 비어 있으므로 실패한다. **의존성을 자동으로 활성화하지 않는다.** 성공하면 상태를 저장하고 `daemon.startup`을 재발화한다. |
| disable | `POST …/disable` | **실행 중인 인스턴스만** 검사한다(`handlers/extension.rs:134-144`). 역의존성(활성 익스텐션 중 이 id를 dependencies에 둔 것)이 있으면 409 `has_dependents`. 인스턴스의 `extension_data`에서 이 익스텐션의 instance_fields 키 중 하나라도 "truthy"(Bool(true)이거나 String/Number/Object/Array는 **값과 무관하게** 모두)이면 409 `in_use`(`ext/:822-853`). 숫자 0이나 빈 문자열도 "사용 중"으로 판정된다. |
| remove | `DELETE /api/extensions/:id` | 활성 상태면 disable 검사를 거친 뒤 discovered에서 제거하고, 디렉토리를 `remove_dir_all`하고, extensionConfig 항목을 삭제한다(`ext/:969-997`) |
| mount / unmount | `POST …/mount`, `…/unmount` | 디스크를 건드리지 않고 메모리에서만 등록/해제한다. unmount는 enabled에서도 제거한다 |
| rescan | `POST /api/extensions/rescan` | 위 3.2 |
| force_disable | (없음) | `ext/:962`. 호출처 0건 |

에러 코드 → HTTP 매핑: `handlers/extension.rs:25-57`. not_found/not_mounted/manifest_not_found는 404, dependency_*와 component_version_unsatisfied는 422, has_dependents/in_use는 409, id_mismatch는 400.

### 3.4 원격 레지스트리 · 설치 · 업데이트

- URL: `DEFAULT_MANIFEST_URL = "https://raw.githubusercontent.com/WareAoba/saba-chan-extensions/main/manifest.json"`, 하드코딩(`ext/:390-392`). `constants::extensions_manifest_url()`(`updater/src/constants.rs:63`)과 값은 같지만 그 함수를 쓰지 않는다(주석에 TODO). `set_manifest_url()`은 호출처 0건이다. module 레지스트리는 `releases/latest/download/manifest.json`을 쓰므로 **extension과 module의 레지스트리 소스 방식이 다르다**(raw main 브랜치 vs 릴리즈 에셋).
- `GET /api/extensions/manifest`: 원격 목록과 `check_updates_against`를 반환한다(`handlers/extension.rs:302`).
- `GET /api/extensions/updates`: 업데이트 목록만 반환한다. `ExtensionUpdateInfo.downloaded/installed`는 항상 false다(`ext/:378-381`, 미구현 필드).
- `POST /api/extensions/:id/install` (body `{download_url?, sha256?}`): body가 없으면 manifest를 fetch해서 URL을 찾는다. `install_from_url`(`ext/:1431-1498`)은 zip을 받아 `extensions/<id>.zip`에 저장한 뒤 **기존 폴더를 삭제하고** 압축을 풀고 mount한다(실패하면 rescan).
  - **sha256 검증은 미구현이다**(`_expected_sha256`, 주석 "v0.2 예정"). GUI도 sha256을 넘기지 않는다(`SabaStorage.js:1164,1249`).
  - `min_app_version`과 `dependencies`는 검사하지 않는다. docker를 설치해도 steamcmd는 설치되지 않는다.
  - 업데이트는 같은 install 엔드포인트로 덮어쓴다. **실행 중이거나 enabled인 익스텐션이어도 아무 검사 없이 디렉토리를 지운다.** enabled 상태는 id 기준이라 유지된다. 설치 후 자동 enable은 없다.
- 별도 경로: `ipc/updates.rs:454`는 업데이트를 적용한 뒤 `rescan()`을 부른다. updater 쪽 익스텐션 업데이트(`updater/src/lib.rs:835-1115, 2001`)는 `extension.toml`을 기준으로 하는데, 실제 익스텐션은 모두 `manifest.json`만 가지므로 **이 경로는 사실상 dead**이고 업데이트 체계가 둘로 갈려 있다.
- 배포(`EXT/.github/workflows/release-extensions.yml`, `EXT/.github/scripts/build_extensions.py`):
  - 루트에서 `.`이나 `_`로 시작하지 않고 `manifest.json`이 있는 디렉토리를 찾는다. `extension-{id}.zip`으로 압축하며 `__pycache__`, `node_modules`, `*.pyc`, `test_*`, `package-lock.json`은 제외한다. sha256을 계산하고, `has_gui = bool(gui.slots)`, `has_i18n = i18n_dir is not None`으로 정한다. 생성한 `manifest.json`을 릴리즈하고 main에 커밋한다(`[skip ci]`).
  - **기존 릴리즈를 모두 삭제한다**(workflow "Delete previous releases"). 그래서 버전 롤백이나 고정이 불가능하고 `latest/download`만 존재한다.
  - zip에 들어가는 원치 않는 파일: `docker/docker/dockerd.pid`(커밋된 런타임 산출물), `music/.deps-resolved.json`(`.gitignore`에 있지만 커밋됨, 개발자 로컬 경로가 담겼을 수 있음), `docker/saba-docker-io/saba-docker-io`(Go 바이너리 커밋), `docker/gui/src/*`. GUI 번들 빌드 스크립트(package.json, esbuild 설정)가 리포에 없다. `docker.umd.js`는 커밋된 산출물을 그대로 배포한다.

### 3.5 의존성 처리 (music 의존성, node)

- 익스텐션 간 의존성: docker는 `{"steamcmd": ">=0.1.0"}`을 가진다. 그래서 steamcmd를 먼저 enable해야 docker를 enable할 수 있고, docker가 enabled인 동안 steamcmd를 disable할 수 없다. 런타임에서는 `compose_manager.provision`이 `../steamcmd/steamcmd.py`를 `importlib`로 **파일 경로로 직접 로드**한다(`compose_manager.py:808-825`).
- music: `daemon.startup` → `music_deps.check_dependencies`가 npm 패키지(`EXT/music/package.json`)를 `npm install`하고 ffmpeg/yt-dlp/opus를 확인한 뒤 결과를 `.deps-resolved.json`에 기록한다(`music_deps.py:91-160`). portable npm/ffmpeg는 data_dir 아래에서 탐색한다(`:40-90`).
  - 실제 음악 기능 코드는 이 익스텐션 패키지에 **없고**, `saba-chan/discord_bot/extensions/music.js`에 있다. music.js는 `.deps-resolved.json`을 읽거나, 없으면 **Node 봇이 Python `music_deps.py`를 직접 spawn한다**(`discord_bot/extensions/music.js:33-100`). daemon을 우회하는 경로다. 또 `../../../saba-chan-extensions/music/music_deps.py`라는 개발 리포 경로까지 후보로 둔다(`:40`).
- node: daemon에 `node_env::find_or_bootstrap()`(`src/main.rs:711`)과 `/api/node-env/setup`이 있다. 익스텐션 manifest에는 node 의존성을 선언할 방법이 없다.
- Python 의존성: 익스텐션별 requirements 선언이 없다. 공용 venv를 쓴다.

---

## 4. GUI 익스텐션 계약

### 4.1 로딩 경로 (Electron)

1. Renderer `window.api.extensionList()` → main.js `ipcMain.handle('extension:list')` → `GET /api/extensions`(`saba-chan-gui/main.js:2408`, `preload.js:31-44`). 모든 호출은 main 프로세스가 axios로 대행한다(토큰 인증 포함).
2. `ExtensionProvider`(`src/contexts/ExtensionContext.js:93-189`)가 enabled 익스텐션마다:
   - `window.api.extensionGuiBundle(id)` → `GET /api/extensions/:id/gui`로 JS 소스 문자열을 받는다. **Blob URL `<script>`**로 실행한다(`:23-57`).
   - 전역 이름 규칙: `window["SabaExt" + PascalCase(id)]`. `-`와 `_` 구분자를 제거하고 다음 글자를 대문자로 바꾼다(`:75-77`). docker → `SabaExtDocker`. 번들이 정확히 이 전역을 만들어야 한다(`docker.umd.js`의 첫 줄이 `var SabaExtDocker = (() => {…`).
   - 스타일: `GET …/gui/styles`가 있으면 `<style data-extension=id>`로 넣는다.
   - 모듈의 **`registerSlots(): { [slotId]: Component | Component[] }`**를 호출해 슬롯 레지스트리에 병합한다. manifest의 `gui.slots`는 쓰지 않는다.
   - 버전이 바뀌면 전역을 삭제하고 style 태그를 제거한 뒤 다시 로드한다(핫로드, `:146-156`).
3. React 공유: 호스트가 `window.React`와 `window.ReactDOM`을 전역으로 노출하고(`src/index.js:11-13`), 번들은 `window.React`를 참조한다(external 처리). 그 밖의 호스트 API(토스트, 데몬 API, 스토어 등)는 노출하지 않는다. 확장 컴포넌트는 props만으로 동작해야 한다.
4. 보안: 번들은 렌더러 컨텍스트에서 **그대로 eval**되므로 `window.api`(preload가 노출한 모든 IPC)에 접근할 수 있다. 무결성 검증(sha256)도 없다.

### 4.2 슬롯과 props (호스트 `<ExtensionSlot slotId … {...props}>`, `components/ExtensionSlot.js`)

| slotId | 호스트 위치 | 전달 props | docker 컴포넌트 |
|---|---|---|---|
| `ServerCard.badge` | `ServerCard.js:166` | `server` | DockerBadge |
| `ServerCard.headerGauge` | `ServerCard.js:229` | `server` | DockerMiniGauge |
| `ServerCard.provision` | `ServerCard.js:306` (`server.provisioning`일 때) | `server, provisionProgress, onDismiss, t` | DockerProvision |
| `ServerCard.expandedStats` | `ServerCard.js:337` | `server, t` | DockerStatsRow |
| `ServerSettings.tab` | `ServerSettingsModal.js:710` | `server, activeTab, setActiveTab, settings, onSettingsChange, t` | DockerTab |
| `AddServer.options` | `AddInstanceNewServer.js:247` | `options{use_container}, onOptionsChange, t` | DockerToggle |

- `server` 객체는 `/api/servers`의 `ServerInfo`다. 여기서 `extension_data`와 `extension_status[<extension_id>]`(list_enrich 결과)를 컴포넌트가 읽는다.
- 슬롯 이름은 호스트에 하드코딩되어 있고, 슬롯 목록을 문서화하거나 검증하는 장치가 없다. CLI 쪽 대응표는 `saba-chan-cli/src/tui/app.rs:276-290`에 주석으로만 있다.
- `AddServer.options`의 `use_container` 의미는 호스트 상태(`useContainerIsolation`)와 daemon(`create_instance`의 `use_container` → `docker_enabled`, `handlers/instance.rs:88-114`)에 하드코딩되어 있다. 범용 슬롯 계약이 아니라 docker 전용 계약이다.

### 4.3 i18n

- 로더: `window.api.extensionI18n(id, lang)` → `GET /api/extensions/:id/i18n/:locale` → `i18n.addResourceBundle(lang, "ext_<id>", …)`(`ExtensionContext.js:82-91`). 현재 언어와 `en`을 로드한다. **언어가 바뀌어도 다시 로드하지 않는다**(effect 의존성이 `[extensions]`뿐).
- 실제로는 docker 컴포넌트가 호스트에서 받은 `t`로 `add_server_modal.docker_isolation`, `server_settings.docker_*` 같은 **호스트 `gui` 네임스페이스 키**를 쓴다. 이 키들은 `saba-chan/locales/*/gui.json`(10개 언어)에 정의되어 있다. 익스텐션 자체 `EXT/docker/i18n/{en,ja,ko}.json`의 `ext_docker` 네임스페이스는 사실상 쓰이지 않는다. → **플러그인 전용 문자열이 호스트에 하드코딩된 위반 사례**다.

### 4.4 GUI에서 노출되지 않는 API

- `GET/PUT /api/extensions/:id/config`(글로벌 설정, `config_fields`)에 대한 preload/main IPC 바인딩이 **없다**. CLI `client.rs:615-622`에만 있다.
- `extension:icon` → SabaStorage(`SabaStorage.js:1065`), `extension:initStatus` → `hooks/useExtensionInitStatus.js`.

---

## 5. 익스텐션별 기능 요약 · module과의 연결

### 5.1 docker (v0.1.0, 의존성 steamcmd>=0.1.0)

- **docker_engine.py**(993줄): 포터블 Docker Engine을 관리한다. Windows는 WSL2 안에 `/opt/saba-chan/docker`를 설치하고, 그 외는 로컬 `base_dir`를 쓴다. `ensure`(설치 + 데몬 기동, `daemon_ready`/`wsl_mode`를 반환), `start_daemon`, `stop_daemon`, `status`, `info`. 진행률은 `PROGRESS:`로 보낸다.
- **compose_manager.py**(1204줄):
  - `provision`(post_create): 3단계 `docker_engine → steamcmd(또는 download) → compose`. module.toml `[docker]` 섹션(`module_install.container`)을 템플릿 치환해 `docker-compose.yml`을 생성한다(`_generate_compose_yaml`, `:1034`). SteamCMD 진행률은 40~66% 구간으로 리매핑한다.
  - `start`: `compose up`을 실행하고 `log_follower`를 반환한다. 우선 `saba-docker-io` 브리지(interactive, Go 바이너리 `saba-docker-io/main.go`: `docker logs --tail 200` 후 `docker attach`)를 쓰고, 없으면 `compose logs --follow`를 쓴다.
  - `stop`(compose stop), `cleanup`(compose down), `status`(docker top과 process_patterns로 판정), `enrich_server_info`(`extension_id:"docker"`, status, memory/cpu), `shutdown_all`(daemon 종료 시 모든 인스턴스), `regenerate_compose`(설정 변경).
  - 발화되지 않는 함수: `container_stats`, `get_logs`, `pre_create`(no-op).
- **GUI**: 6개 슬롯 컴포넌트와 `MemoryGauge`.
- **CLI 슬롯**: `InstanceList.badge`(🐳), `InstanceDetail.menu`(open_ext_settings), `InstanceDetail.status`(value_from `container_status`), `CreateInstance.options`(docker_enabled).
- **instance_fields**: `docker_enabled`, `docker_cpu_limit`, `docker_memory_limit`.

### 5.2 steamcmd (v0.1.0)

- `SteamCMD` 클래스(`steamcmd.py:141-640`): 포터블 SteamCMD 탐색/설치, `install(app_id, install_dir, anonymous, beta, platform, validate)`, buildid 비교로 `check_update`.
- daemon hook으로 실제로 쓰이는 것은 `server.check_update` 하나뿐이다. 설치는 module의 `lifecycle.py install_server`가 `from extensions.steamcmd import SteamCMD`로 직접 수행한다(`saba-chan-modules/palworld/lifecycle.py:1254`, `zomboid/lifecycle.py:1535`). docker는 파일 경로로 import한다.
- instance_fields `install_method_steamcmd`, `steamcmd_app_id` 등은 daemon이 module의 `[install] method="steamcmd"`를 보고 **하드코딩으로 주입**한다(`src/instance/mod.rs:126-170` `apply_install_extension_data`).

### 5.3 music (v0.1.0)

- 내용물은 `music_deps.py`(의존성 검사/설치)와 `package.json`(Node 패키지 목록)뿐이다. 재생 로직은 `saba-chan/discord_bot/extensions/music.js`와 `musicChannelUI.js`에 있다.
- GUI는 `extensions.some(e => e.id==='music' && e.enabled)`로 Discord 모달의 음악 설정을 켜고 끈다(`DiscordBotModal.js:70-72`). 음악 설정 UI는 호스트에 하드코딩되어 있다(`gui.builtin:true`의 실질적 의미).

### 5.4 ue4-ini (v0.1.0, 디렉토리명 `ue4_ini`)

- `parse_option_settings(ini_path)`와 `write_option_settings(ini_path, props, section)`를 제공하는 **순수 Python 라이브러리**다. hook 선언은 dead다(2.5절).
- palworld가 `requires_extensions = ["steamcmd", "ue4-ini"]`로 요구하고, `lifecycle.py:27`에서 **모듈 최상위에서 무조건 import**한다. ue4-ini가 설치되지 않았으면 palworld lifecycle 전체(status/stop 포함)가 ImportError로 실패한다. `SABA_ENABLED_EXTENSIONS` 검사보다 앞서 import하므로 disable해도 import는 계속 동작한다(디스크에 있는 한). 즉 "disable"이 Python import를 막지 못한다.

### 5.5 module → extension 참조 방식

1. `module.toml [install] requires_extensions = [...]`. 인스턴스 생성 시 `instance.required_extensions`에 복사된다(`handlers/instance.rs:178-185`). 네이티브 인스턴스만 생성 시점에 검증하고(`:187-221`), 시작 시점에는 다시 검증한다(`supervisor/mod.rs:259-317`, `installed_and_enabled_set`). module 기능 엔드포인트에서도 검사한다(`handlers/managed.rs:416-430`).
2. `module.toml [docker]` 섹션은 `metadata.extensions["docker"]`로 파싱된다(module_loader가 하드코딩). post_create와 settings_changed context로 전달된다.
3. Python 레벨: `from extensions.<pkg> import …`. PYTHONPATH 주입에 의존하고, `SABA_ENABLED_EXTENSIONS`로 자체 검사한다.
4. `saba-chan-modules/__example/__example__lifecycle.py:408`의 `from extensions.steamcmd import install_game`은 **존재하지 않는 함수**다(예제가 오래됨).

---

## 6. ext_process: 장기 실행 외부 프로세스 계약 (`src/ipc/handlers/ext_process.rs`)

- 목적: 이름 기반 범용 프로세스 매니저. 실제 사용자는 **Discord 봇(`discord-bot`) 하나뿐**이다. 익스텐션 manifest와 **아무 연관이 없다**(manifest에 프로세스 선언 필드가 없음).
- API(`src/ipc/mod.rs:627-632`):
  - `GET /api/ext-processes` → `{processes:[{name,status,pid,meta}]}`
  - `POST /api/ext-process/:name/start` body `StartProcessRequest{command, args[], cwd?, env{}, meta}`(`:144-158`) → `{ok,pid,name}`. 이미 실행 중이면 에러.
  - `POST …/stop`: stdin에 `{"type":"shutdown"}` 줄을 쓰고, 1초 뒤 PID가 같으면 `kill -9` / `taskkill /F /T`(`:336-377`).
  - `GET …/status` → `{name,status,pid,meta}`
  - `GET …/console?since=<id>&count=<n=200>` → `{name, lines:[{id,line}], running}`. 링버퍼는 100,000줄이다(`:58-68`). 각 줄 앞에 `[stdout]`, `[stderr]`, `[system]`을 붙인다.
  - `POST …/stdin`: 한 줄을 전송한다(`\n`을 자동으로 붙임).
- 프로토콜 관례: stdout에서 `__IPC__:` 접두 줄은 daemon 터미널 echo에서만 숨기고 버퍼에는 저장한다(`:229-231`). 종료하면 pid=None과 `[system] Process exited`를 기록한다. daemon이 종료될 때 `shutdown_all`(`:89-130`, `main.rs:446-449`)을 호출한다.
- **보안/설계 지적**: 인증된 클라이언트라면 **임의의 command/args/env로 프로세스를 실행**할 수 있다(auth 미들웨어 `ipc/mod.rs:667`만 있음). 실행할 명령은 interface(GUI `main.js:3226-3269`)가 조립한다. daemon에도 같은 조립 로직이 중복되어 있다(`main.rs` `auto_start_discord_bot` `:669-760`, node 경로·env 구성). "interface는 스스로 아무것도 못 한다 / daemon이 주인"이라는 원칙에서 보면, 봇의 실행 사양을 GUI가 결정한다는 점에서 경계가 흐리다.

---

## 7. 불일치 · dead code · 스텁 · 원칙 위반

### 7.1 Dead code / 미구현

1. `HookBinding.is_async`(`ext/:217-218`): 읽는 곳이 없다.
2. `min_app_version`(`ext/:182,311`): 강제하지 않는다.
3. `should_parse_config_section`(`ext/:1309`), `all_instance_fields`(`:1317`), `force_disable`(`:962`), `enabled_set`(`:1005`), `is_newer_version`(`:1420`), `set_manifest_url`(`:1353`), `replace_extension_config`(`:1542`), `enable_with_versions`(외부 호출 0건): `impl` 전체에 `#[allow(dead_code)]`가 걸려 있다(`ext/:407`).
4. `install_from_url`의 sha256 검증: 스텁이다(`ext/:1435,1439`).
5. `ExtensionUpdateInfo.downloaded/installed`: 항상 false다(`ext/:378-381,1408-1409`).
6. `plugin::run_extension*`(`plg/:263-284`): 옛 레이아웃이다.
7. updater의 `extension.toml` 기반 익스텐션 업데이트(`updater/src/lib.rs:835-1115`): 실제 익스텐션과 형식이 맞지 않는다.
8. 발화되지 않는 hook 7종(2.3절)과 그 Python 함수들(`pre_create`는 no-op 스텁).
9. `ue4_ini_enabled` instance_field: 설정하는 경로가 없다.

### 7.2 버그 · 위험

1. `SABA_ENABLED_EXTENSIONS` 상태 파일 경로 불일치(`plg/:140-151` vs `constants.rs:219`). Linux에서 module의 steamcmd 검사가 실패할 수 있다.
2. hook 실행 순서가 HashMap에 의존해 비결정적이다(`ext/:1058`). 그런데 `handled`가 체인을 끊는다.
3. `dispatch_hook_with_progress`는 익스텐션 1개만 실행한다(`ext/:1302`).
4. enable할 때 모든 익스텐션의 `daemon.startup`이 재실행된다(`handlers/extension.rs:99`). npm install이나 엔진 체크가 중복된다.
5. 긴 hook 동안 `extension_manager` read 락을 유지한다. post_create(30분) 동안 enable/disable/install/rescan(write)이 블록된다(`handlers/instance.rs:319`, `main.rs:177`).
6. `daemon.startup` 120초 타임아웃이 Docker 엔진 다운로드나 npm install에 비해 짧을 수 있다.
7. `server.post_stop`이라는 이름과 실제 의미(정지 대행, pre)가 다르다(`supervisor/mod.rs:566`).
8. `server.list_enrich`는 인스턴스마다 순차로 Python을 실행한다(10초 × N). TTL 30초 캐시에 `handled:false`도 캐싱한다.
9. `instances_using_ext`는 숫자 0, 빈 문자열, `false`가 아닌 모든 Object/Array를 "사용 중"으로 판정한다(`ext/:841-848`). disable은 **실행 중인** 인스턴스만 보므로 정지된 docker 인스턴스가 있어도 docker를 disable하거나 **remove**할 수 있다. 이후 시작하면 extension_required 에러가 난다.
10. `save_extension_config` 주석은 "null을 전달하면 키 제거"라고 하지만, 실제로는 null 값을 저장한다(`handlers/extension.rs:504` vs `ext/:1531-1539`).
11. 컴포넌트 의존성(`saba-core` 등)을 선언하면 HTTP enable이 불가능하다(`ext/:856-858`).
12. install/update가 실행 중이거나 enabled인 익스텐션 디렉토리를 검사 없이 삭제한다(`ext/:1466-1469`). 무결성 검증이 없고, 의존성을 자동 설치하지 않는다.
13. `discover()`/`rescan()`이 manifest.id와 디렉토리명의 일치를 검사하지 않는다(mount만 검사, `ext/:760`). zip 추출은 기존 폴더가 있으면 새 zip을 **버린다**(`ext/:607-613`).
14. 모든 hook context에 전체 `extensionConfig.json`(타 익스텐션 비밀값 포함)이 주입된다(`ext/:1090-1100`).
15. GUI 번들을 Blob script로 무검증 실행하고, `window.api`에 전체 접근할 수 있다.
16. GUI i18n이 언어 변경 시 다시 로드되지 않는다(`ExtensionContext.js:189`).
17. 리포에 런타임 산출물이 커밋되어 배포 zip에 들어간다: `docker/docker/dockerd.pid`, `music/.deps-resolved.json`, Go 바이너리. GUI 번들 빌드 설정이 없다.
18. 릴리즈 워크플로가 기존 릴리즈를 모두 삭제한다. 버전 고정이나 롤백이 불가능하다.
19. CLI 도움말은 `extension [...|registry|...]`라고 하지만 실제 서브커맨드는 `manifest`다(`saba-chan-cli/src/tui/commands.rs:742` vs `:139,996`).

### 7.3 "daemon이 주인 / 플러그인은 자유롭게 커스터마이즈" 원칙 위반 (하드코딩된 플러그인 지식)

Core(daemon/GUI/CLI)가 특정 익스텐션을 이름으로 아는 곳들이다. 플러그인을 자유롭게 교체하거나 추가할 수 없게 만든다.

| 위치 | 하드코딩 내용 |
|---|---|
| `supervisor/mod.rs:429,584,777,884,1131` | `instance.ext_enabled("docker_enabled")`. hook이 처리하지 않으면 에러나 최소 응답을 반환한다 |
| `handlers/server.rs:683` | stop 후 `docker_enabled`이면 ProcessTracker untrack을 생략한다 |
| `handlers/instance.rs:88-114` | `use_container` → `is_enabled("docker")` 검사, `docker_enabled=true` 주입, "네이티브는 모듈당 1개" 규칙 |
| `handlers/instance.rs:171` | `metadata.extensions.get("docker")` |
| `supervisor/module_loader.rs:267,693` | module.toml `[docker]` 섹션 전용 구조체(`ContainerSectionToml`)와 `"docker"` 키. manifest의 `module_config_section`을 무시한다 |
| `src/instance/mod.rs:126-170` | `install_method_steamcmd`, `steamcmd_app_id` 등 steamcmd instance_field 이름을 core가 직접 쓴다 |
| `handlers/instance.rs:1290-1294` | check_update에서 `extension_data.steamcmd_app_id` 읽기 |
| GUI `DiscordBotModal.js:70-72` | `e.id==='music'` |
| GUI `AddInstanceNewServer.js:247-252` | `options.use_container` 계약 |
| 호스트 `locales/*/gui.json` | docker UI 문자열(`server_settings.docker_*`, `add_server_modal.docker_*`) |
| `discord_bot/extensions/music.js` | music 기능 본체가 core 리포(봇)에 있다. 봇이 Python `music_deps.py`를 daemon 없이 직접 실행한다(`:33-100`) |
| 익스텐션 → 익스텐션 | docker `provision`이 `../steamcmd/steamcmd.py`를 파일 경로로 직접 import한다(`compose_manager.py:808-825`). daemon의 dependency 메커니즘을 거치지 않는다 |
| module → extension | module이 `from extensions.x import`로 직접 의존한다. disable해도 import는 막히지 않는다(palworld ue4_ini 최상위 import) |

daemon을 우회하는 interface 동작:
- GUI(`main.js:3226-3269`)가 봇 실행 command와 env를 조립해 `/api/ext-process/discord-bot/start`로 보낸다. daemon의 `auto_start_discord_bot`(`main.rs:669+`)과 로직이 중복된다.
- CLI는 `get_extensions_path()`(`saba-chan-cli/src/config.rs:345`)로 익스텐션 경로를 자체 해석한다(표시 용도).

### 7.4 GUI/CLI 슬롯 체계 비대칭

- GUI: 번들의 `registerSlots()`가 기준이고 manifest의 `gui.slots`는 무시한다.
- CLI: manifest의 `cli.slots`(데이터 기반)가 기준이다(`saba-chan-cli/src/tui/mod.rs:1129-1150`, `tui/app.rs:323-385`). instance_fields가 있으면 `InstanceSettings.fields` 슬롯을 자동 파생한다. CLI는 `/api/extensions` 결과를 내부 출력 버퍼의 `EXT_SLOTS_INIT:<json>` 라인 프로토콜로 전달해 슬롯을 만든다(`tui/mod.rs:155-158,1038`).
- CLI badge의 `condition` 평가는 CLI 쪽 `evaluate_badge_condition`이 따로 구현한다(`tui/screens/servers.rs:31-35`). daemon의 `evaluate_condition`과 중복 구현이다.
