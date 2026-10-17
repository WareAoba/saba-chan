# 02. 게임 모듈 시스템 (module.toml / lifecycle.py / Managed Process / 배포)

> 범위: `saba-chan` 데몬의 `src/supervisor/**`, `src/plugin/mod.rs`, `src/ipc/handlers/{managed,instance,server,command}.rs`, 공용 Python 유틸(`saba-chan/modules/`), 그리고 `saba-chan-modules` 리포(minecraft / palworld / zomboid / `__example`, 빌드 스크립트·워크플로).
> 원칙: 문서는 신뢰하지 않고 코드 기준으로 서술. 문서 주장과 코드가 다르면 표시함. 경로 약어: `D:` = saba-chan 저장소 루트, `M:` = saba-chan-modules 저장소 루트.

### 프로젝트 오너의 아키텍처 원칙 (판정 기준)

- **daemon** = 서비스의 주인. 모든 기능은 데몬을 거친다.
- **interface** (GUI/CLI) = 혼자서는 아무것도 못 한다.
- **module** = 게임 서버 운영법을 기술한 Python "매뉴얼".
- **plugin (= extension)** = Python. 데몬에 기능을 더하고 인터페이스에 컴포넌트를 주입한다.
- **server-chan** = 데몬과 함께 Discord 봇을 돌린다.

이 원칙을 어기는 지점은 각 절에 **[원칙 위반]** 으로 표시하고, §6.5에 다시 모았다.

---

## 1. 모듈 발견과 로딩

### 1.1 탐색 위치

| 항목 | 값 | 근거 |
|---|---|---|
| 모듈 루트 | `SABA_MODULES_PATH` 환경변수. 없으면 `resolve_data_dir()/modules`, 즉 Windows는 `%APPDATA%/saba-chan/modules`, Linux/macOS는 `$HOME/.config/saba-chan/modules` (`SABA_DATA_DIR`로 override 가능) | `D:updater/src/constants.rs:88-131`, `D:src/plugin/mod.rs:46-52` |
| GUI가 데몬을 띄울 때 | GUI가 `SABA_MODULES_PATH`를 명시적으로 주입 | `D:saba-chan-gui/main.js:556-563` |
| 레거시 경로 | `Supervisor::run()`이 `"./modules"`를 사용하지만 `#[allow(dead_code)]`라 쓰이지 않음 | `D:src/supervisor/mod.rs:1888-1895` |

`ModuleLoader::discover_modules()` (`D:src/supervisor/module_loader.rs:756-819`)의 동작은 다음과 같다.

- 모듈 루트의 **1단계 엔트리만** 순회하고 재귀하지 않는다.
- **폴더 모듈**: `<dir>/module.toml`이 있는 디렉터리만 로드한다 (`:794-813`). 폴더 이름은 아무 것이나 되며, 식별자는 `[module].name`이다.
- **ZIP 모듈**: 확장자가 `.zip`인 파일이 대상이다 (`:775-792`, `load_module_from_zip` `:836-885`).
  - 아카이브 엔트리 중 이름이 `module.toml`로 **끝나는 첫 엔트리**를 파싱한다 (`:845`). 그래서 `__example__module.toml`이나 하위 폴더의 toml도 잡힐 수 있다.
  - 아카이브 전체를 tempdir에 풀었다가 `<modules>/.extracted/<name>/`로 복사한다. 이 과정에서 기존 디렉터리는 `remove_dir_all`로 지운다.
  - 캐시가 무효화될 때마다(`refresh_modules`) 다시 풀린다. 그동안 실행 중인 Python 프로세스가 그 디렉터리를 쓰고 있어도 삭제된다.
- 결과는 `RwLock<Option<Vec<LoadedModule>>>`에 캐시된다. `invalidate_cache()`(`:750`)는 `/api/modules/refresh`, 모듈 설치·삭제 시에만 호출된다.
- 로드에 실패한 모듈은 `warn` 로그만 남기고 조용히 건너뛴다.
- `.py` 파일(`daemon_rcon.py`, `i18n.py`)이 같은 루트에 있어도 무시된다. 이 둘은 PYTHONPATH용 공용 라이브러리다 (§2.1).

**진입점 파일**: `[module].entry`는 필수 필드로 파싱되지만 **실제로는 쓰이지 않는다**. 데몬의 모든 호출 지점이 `format!("{}/lifecycle.py", module.path)`로 파일명을 하드코딩한다. 예시 위치는 `D:src/supervisor/mod.rs:487, 700, 809, 893, 1189, 1389, 1419, 1457, 1474, 1503, 1522, 1541, 1638`, `D:src/ipc/handlers/instance.rs:649`, `command.rs:85`이다. `entry = "main.py"`로 바꿔도 동작하지 않는다.

### 1.2 module.toml 스키마 (Rust 역직렬화 기준, 전체)

파서는 `parse_module_toml()` → `toml::from_str::<ModuleToml>` → `into_metadata()`이다 (`module_loader.rs:725-729, 584-720`).

- `#[serde(deny_unknown_fields)]`가 없다. 그래서 **모르는 키나 섹션은 모두 조용히 무시**된다.
- 반대로 **알고 있는 키의 타입이 틀리면 모듈 전체 로드가 실패**한다. 예를 들어 settings의 `options`에 숫자를 넣으면 `Vec<String>`이라 실패한다.
- BOM: `palworld/module.toml`은 UTF-8 BOM으로 시작한다. `fs::read_to_string`은 BOM을 그대로 두며, toml 파서가 BOM을 허용하는지는 **검증하지 못했다**. 빌드 스크립트는 BOM을 명시적으로 제거한다 (`M:.github/scripts/build_modules.py:98-100`).

표의 기호: ✔ = 필수(없으면 파싱 실패), ○ = 선택.

#### `[module]` (섹션 자체가 필수, `ModuleSection` `:278-297`)

| 키 | 타입 | 필수 | 기본값 / 비고 |
|---|---|---|---|
| `name` | String | ✔ | 모듈 ID. 인스턴스의 `module_name`과 매칭됨 |
| `version` | String | ✔ | semver 형식 기대(검증 없음) |
| `entry` | String | ✔ | **파싱만 하고 미사용** (위 1.1 참조) |
| `description` | String | ○ | None |
| `icon` | String | ○ | 모듈 폴더 기준 상대경로. `/api/modules`가 base64로 인코딩해 응답 (`D:src/ipc/handlers/server.rs:276-286`). MIME은 무조건 `image/png` |
| `log_pattern` | String(regex) | ○ | `(?P<level>…)` 명명 그룹 필수. 콘솔 레벨 파싱에 쓰임 (§3.3) |
| `dependencies` | Table<String,String> | ○ | `[module.dependencies]`. `check_dependencies()`는 `#[allow(dead_code)]`라 **호출하는 곳 없음** (`:135-161`) |
| `game_name` | String | ○ | `_game_name`으로 받아서 버림. **메타데이터에 실리지 않음** |
| `display_name` | String | ○ | `_display_name`으로 받아서 버림. 빌드 스크립트와 installer만 사용 |

#### `[update]` (`UpdateSection` `:299-304`)
| 키 | 타입 | 비고 |
|---|---|---|
| `github_repo` | String | 데몬 로더에서는 `_update`로 버림. **업데이터**(`D:updater/src/lib.rs:806-830`)가 별도로 raw toml을 읽어서 사용 |

#### `[protocols]` (`ProtocolsSection` `:306-314`)
| 키 | 타입 | 사용처 |
|---|---|---|
| `supported` | Vec<String> | `rcon`/`rest`/`stdin`. 포트 충돌 검사(`build_module_protocols_map`, `D:src/supervisor/mod.rs:163-173`), 인스턴스 생성 시 rcon_port/rest_port 기본값 결정(`instance.rs:142-147`) |
| `default` | String | GUI로 전달만 함 (`ProtocolsInfo`) |
| `interaction_mode` | String | `console` 또는 `commands`. GUI와 봇이 시작 방식·UI 결정에 사용 (데몬은 해석하지 않음) |

#### `[config]` (`ConfigSection` `:316-334`, `#[serde(flatten)] _extra`로 나머지 키 흡수)
| 키 | 타입 | 사용처 |
|---|---|---|
| `executable_path` | String | `/api/modules` 응답에 그대로 노출. 데몬은 이 값으로 실행하지 않음 |
| `server_executable` | String | `working_dir + server_executable`로 실행파일 자동 해석 (`mod.rs:182-210`), 설치 시 백업 대상(`:1543-1570`), 마이그레이션 (`instance.rs:637`) |
| `process_name` | String | 인스턴스 생성 시 `instance.process_name`에 복사 → auto-detect (`instance.rs:137`, `mod.rs:1780-1791`) |
| `default_port` | u16 | 인스턴스 생성 시 `instance.port` 기본값 |
| `stop_command` | String | managed stop 때 stdin으로 전송. 기본값 `"stop"` (`mod.rs:603-606`) |
| `stop_timeout` | u64(초) | managed graceful stop 대기 시간. 기본값 30 (`mod.rs:638-642`) |
| (기타) | any | `_extra`로 흡수 후 **버림**. 예: minecraft의 `java_path`, `server_jar`, `eula`, `rcon_port`, `ram` |

#### `[detection]` (`DetectionSection` `:336-348`)
| 키 | 타입 | 사용처 |
|---|---|---|
| `process_patterns` | Vec<String> | extension hook `server.status` 컨텍스트로만 전달 (`mod.rs:755-765`). 네이티브 auto-detect에는 **미사용** |
| `cmd_patterns` | Vec<String> | auto-detect 때 커맨드라인 필터 (`mod.rs:1782-1791`) |
| `dir_signatures` | Vec<String> | `/api/modules`로 노출. 매칭은 **GUI가 수행** (`D:saba-chan-gui/src/components/Modals/AddInstanceMigration.js:74-82`, 전부 일치해야 함) |
| `common_paths` | Vec<String> | `#[allow(dead_code)]`. **미사용** |
| `[detection.linux]` 하위 전부 | — | **구조체에 없음 → 무시**. 모든 실제 모듈이 `process_name`, `server_executable`, `process_patterns`, `dir_signatures`, `common_paths`를 Linux용으로 override하려 하지만 효과 없음 |

#### `[[settings.fields]]` (`SettingFieldToml` `:356-381` → `SettingField` `:221-234`)
| 키 | 타입 | 기본값 |
|---|---|---|
| `name` | String | ✔ |
| `type` | String | `"text"` (`default_field_type`) |
| `label` | String | 없으면 `name`으로 대체 |
| `description` | String | — |
| `required` | bool | — |
| `default` | toml::Value | — (`get_setting_default`가 Int/String/Float/Bool을 파싱) |
| `min`, `max`, `step` | f64 | — |
| `options` | **Vec<String>** | 문자열만 허용 |
| `group` | String | 관례상 `saba-chan` / `advanced` / 없음(basic) |

- `fields`가 비어 있으면 `settings = None`이 된다.
- 데몬이 직접 의미를 부여하는 setting 이름은 `rcon_port`(기본 25575), `rest_port`(기본 8212), `rest_host`(기본 127.0.0.1) 뿐이다 (`:103-116`).
- 실제 모듈에서 쓰이는 type 값: `text`, `number`, `boolean`, `select`, `password`, `file`.

#### `[[commands.fields]]` (`CommandFieldToml` `:531-550` → `CommandField` `:196-207`)
| 키 | 타입 | 비고 |
|---|---|---|
| `name` | String ✔ | |
| `label` | String | 없으면 `name` |
| `description` | String | |
| `method` | String | 주석에는 `"rest" | "rcon" | "both"`라고 적혀 있음 (`:201`). GUI와 봇이 실제로 분기하는 값은 `rcon` / `rest` / `dual` / 미지정 (§6) |
| `http_method` | String | REST 전용 |
| `endpoint_template` | String | REST 전용 |
| `rcon_template` | String | 예: `"kick {player}"` |
| `command_template` | String | **별칭**. `rcon_template`이 없을 때 이 값이 대입됨 (`:614`) |
| `inputs` | Vec<CommandInput> | |

`[[commands.fields.inputs]]` (`CommandInputToml` `:552-565`)의 허용 키: `name`✔, `label`, `type`, `required`, `placeholder`, `default`.
→ 실제 모듈은 `options`, `min`, `max`도 쓴다 (minecraft의 `weather`·`difficulty`·`gamemode`·`time`·`whitelist`·`give`, zomboid의 `setaccesslevel`·`createhorde`·`godmod`·`invisible`·`additem`·`addxp`, palworld의 `shutdown.waittime`). 하지만 **구조체에 해당 필드가 없어 버려진다**. 그 결과 `/api/modules`를 통해 받는 GUI·봇은 select 옵션과 숫자 범위를 받지 못한다. 문서(`M:docs/module-development-guide.md:874-897`)는 이 키들을 지원한다고 명시하므로 **문서와 코드가 다르다**.

#### `[[syntax_highlight.rules]]` (`HighlightRuleToml` `:573-582`)
`name`✔, `pattern`✔(regex, `(?P<hl>…)` 선택), `color`✔(CSS 색 또는 시맨틱 토큰), `bold`=false, `italic`=false. GUI에 그대로 전달되고 데몬은 해석하지 않는다.

#### `[install]` (`InstallSectionToml` `:384-404` → `ModuleInstallConfig` `:438-457`)
| 키 | 타입 | 기본값 |
|---|---|---|
| `method` | String | `"manual"` (`steamcmd` / `download` / `manual`) |
| `app_id` | u32 | — |
| `anonymous` | bool | `true` |
| `install_subdir` | String | — |
| `platform` | String | — |
| `download_url` | String | — |
| `beta` | String | — |
| `requires_extensions` | Vec<String> | `[]`. 인스턴스 시작 시 필수 익스텐션 검증에 사용 (`mod.rs:259-318`) |

→ zomboid는 `beta_branch = "unstable"`을 쓰는데, 로더가 아는 키는 `beta`라서 **무시된다**.

#### `[docker]` (`ContainerSectionToml` `:407-435`)
`image`, `working_dir`, `restart`, `command`, `entrypoint`, `user`, `ports[]`, `volumes[]`, `environment{}`, `dockerfile`, `cpu_limit`(f64), `memory_limit`.

- `image`가 있을 때만 `extensions["docker"]`에 JSON으로 들어간다 (`:673-697`). `restart` 기본값은 `"unless-stopped"`이다.
- `ContainerExtensionConfig.extra_options`는 toml에서 받을 방법이 없고, 항상 빈 맵이다.

#### `[credential_map]` (`HashMap<String,String>`)
데몬 키(`rest_password` / `rest_username` / `rcon_password`) → 게임 설정 키. 사용처는 managed start 후 동기화(`mod.rs:1245-1284`)와 import_settings 역매핑(`instance.rs:664-…`)이다. **non-managed `start_server`에서는 동기화하지 않는다**.

#### `[aliases]`
`module_aliases: Vec<String>`, `[aliases.commands.<cmd>] = { aliases = [...], description = "..." }`.

#### 로더가 모르는 섹션과 키 (실제 모듈에서 사용됨)

| 섹션·키 | 사용 모듈 | 실제 소비자 |
|---|---|---|
| `[errors]` | minecraft, palworld, zomboid | 로더는 무시. **Discord 봇**이 `/api/module/:name`의 `toml` raw 필드로 읽음 (`D:src/ipc/handlers/server.rs:389-408`, `D:discord_bot/core/processor.js:602-605`). 데몬이 원본 TOML 전체를 JSON으로 재노출하는 우회 경로 |
| `[detection.linux]` | 3개 모듈 전부 | 아무도 사용 안 함 (Linux에서 auto-detect·실행파일 해석이 Windows 값으로 동작) |
| `module.author` / `game_name` / `display_name` | 전부 | 빌드 스크립트, installer |
| `config.java_path` / `server_jar` / `eula` / `rcon_port` / `ram` (minecraft), `config.rcon_port` (zomboid) | | 미사용 |
| `install.beta_branch` (zomboid) | | 미사용 (오타성 불일치) |
| `commands.fields.inputs.options/min/max` | 전부 | 버려짐 |
| `detection.common_paths` | 전부 | 미사용 |

---

## 2. lifecycle.py 프로토콜

### 2.1 실행 방식 (`D:src/plugin/mod.rs:115-258`)

- **명령줄**: `<python_exe> <module_path>/lifecycle.py <function_name>`
  - Python 실행파일은 `python_env::get_python_path()`가 정한다(데몬이 관리하는 venv).
  - cwd는 지정하지 않으므로 **데몬의 cwd를 상속**한다.
- **stdin**: config 전체를 JSON 한 덩어리로 쓰고 바로 닫는다(`shutdown`).
- **stdout**: 프로세스 종료 후 전체를 읽어 **통째로** `serde_json::from_str`로 파싱한다. stdout에 JSON 외의 출력이 한 줄이라도 섞이면 `Invalid JSON from plugin` 에러가 난다.
- **stderr**: 줄 단위로 스트리밍된다.
  - `PROGRESS:{json}` 형식의 줄은 `ExtensionProgress {percent, message, step, total, label, steps}`로 파싱해 콜백에 넘긴다 (`:7-20, 185-205`). 설치 진행률 표시에 쓰인다.
  - 나머지 줄은 `debug` 로그로만 남는다.
- **종료 코드**: 0이 아니면, stdout에 `{"success":false,...}`가 있더라도 `Err("Plugin execution failed (exit ..)")`가 된다. 그래서 모듈이 `sys.exit(1)`로 내보낸 JSON 메시지는 호출자에게 구조화된 형태로 전달되지 않는다.
- **타임아웃**: 기본 `DEFAULT_PLUGIN_TIMEOUT_SECS = 120` (`:55`). 초과하면 `child.kill()`.
  - `install_server`를 프로비저닝 경로(`instance.rs:524-527`)와 업데이트 경로(`instance.rs:1486-1492`)에서 부를 때만 1800초를 쓴다.
  - **`POST /api/module/:name/install` → `Supervisor::install_server`는 기본 120초**다 (`mod.rs:1585`). Minecraft의 Java 다운로드와 jar 다운로드가 120초를 넘으면 실패한다.
- **환경변수** (`:128-174`):
  - `PYTHONIOENCODING=utf-8`, `PYTHONUNBUFFERED=1`
  - `PYTHONPATH` = `<extensions_dir의 부모>` + `<modules_dir>` + 기존 `PYTHONPATH`.
    - 그래서 `import extensions.steamcmd`, `import daemon_rcon`, `import i18n`이 가능하다.
    - extensions_dir가 디렉터리일 때만 설정된다. 다만 `resolve_extensions_dir()`가 디렉터리를 만들어 주므로 사실상 항상 설정된다.
  - `SABA_ENABLED_EXTENSIONS` = 활성 익스텐션 ID를 콤마로 연결한 문자열. 읽는 경로가 `%APPDATA%/saba-chan/extensions_state.json`이고, **APPDATA가 없으면 `./extensions_state.json`(cwd 상대)**이다 (`:140-146`). 그런데 `ExtensionManager`는 `constants::resolve_extensions_state_path()` = `resolve_data_dir()/extensions_state.json`에 쓴다 (`D:updater/src/constants.rs:218-220`). 따라서 **Linux/macOS, 그리고 `SABA_DATA_DIR`를 쓰는 경우에는 이 변수가 설정되지 않는다**. 그러면 palworld·zomboid의 `install_server`는 항상 "extension not enabled"를 돌려준다 (`M:palworld/lifecycle.py:1243-1250`, `M:zomboid/lifecycle.py:1523-1530`).
  - **`SABA_LANG`, `DAEMON_API_URL`, IPC 토큰은 주입하지 않는다.**
    - 문서(`docs/module-development-guide.md:1823-1831`, §5.4)는 데몬이 `SABA_LANG`과 `DAEMON_API_URL`을 주입한다고 하는데, 이는 코드와 다르다.
    - `SABA_LANG`은 GUI가 데몬을 띄울 때 데몬 프로세스 환경에 넣은 값이 상속될 때만 존재한다 (`saba-chan-gui/main.js:560`). CLI나 단독 실행이면 영어로 고정된다.
    - `DAEMON_API_URL`이 없으면 모듈은 `http://127.0.0.1:57474`로 하드코딩된 주소를 쓴다. GUI 설정의 `ipcPort`가 다르면 콜백이 깨진다.

### 2.2 모듈 측 디스패치 관례

모든 모듈이 같은 구조를 따른다 (`M:minecraft/lifecycle.py:1971-1994`, `palworld:1655-1701`, `zomboid:1713-1752`, `__example:534-555`).

1. `sys.argv[1]`을 읽는다.
2. stdin JSON을 읽는다. 비어 있으면 `{}`로 둔다.
3. `FUNCTIONS[name](config)`를 호출한다.
4. `print(json.dumps(result))`로 결과를 출력한다.

모르는 함수 이름이 오면 `{"success":false,"message":"Unknown function: X"}`를 돌려준다. 이때 종료 코드는 minecraft·palworld·example이 0, zomboid가 1이다.

**에러 관례 (코드에서 관찰한 것)**:
- 공통 응답: `success: bool`, `message: str`.
- `get_launch_command` 성공: `program`, `args[]`, `working_dir`, `env_vars{}`. 그 밖의 선택 키:
  - `rest_credentials{rest_username, rest_password}`
  - credential_map의 게임 키(예: palworld의 `"AdminPassword"`)를 최상위나 `module_settings{}` 아래에 둔다.
- `get_launch_command` 사용자 조치 필요: `action_required: "<code>"`를 넣으면 데몬이 결과를 그대로 GUI에 넘긴다 (`mod.rs:491-495, 1195-1199`). minecraft는 `server_jar_not_found` + `options:["update_path","install_new"]`를 쓴다.
- 익스텐션 누락: `error_code: "extension_required"`, `missing_extensions[]`.
- `status`: `status` 문자열과 `pid`를 데몬이 읽는다. 나머지 키는 버린다 (`mod.rs:830-841`).
- `install_server`: `success`, `executable_path`, `install_path`를 프로비저닝 경로가 읽는다 (`instance.rs:532-541`).
- `list_versions`: 데몬은 `latest.release` 또는 `latest_release`만 읽는다 (`instance.rs:466-474`).
- `get_installed_version`: `version`.
- `get_credentials`: `success`, `rcon_port`, `rcon_password`, `rest_username`, `rest_password` (`command.rs:101-140, 260-330`).
- `import_settings`: `success`, `settings{}`. `settings.port`는 `instance.port`로 동기화된다 (`instance.rs:653-663`).

### 2.3 데몬이 호출하는 함수 전체 목록과 전달 config

| 함수 | 호출 지점 | 데몬이 넣는 config 키 |
|---|---|---|
| `get_launch_command` | managed 시작 `mod.rs:1193`, non-managed 시작 `mod.rs:489` | (요청 payload.config) + `server_executable`, `server_jar`(없을 때), `working_dir`, `port`, `rcon_port`, `rcon_password`, **`module_settings`를 최상위로 펼침**, `rest_host`, `rest_port`, `rest_username`, `rest_password`, `protocol_mode`, `server_version`, `managed: true/false`. **`instance_id`는 없음** |
| `stop` | non-managed 정지만 `mod.rs:717` | `server_executable`, `server_jar`, `working_dir`, `pid`, `rcon_port`, `force`. **`instance_id`, `managed`, `module_settings`, `rcon_password` 없음** |
| `status` | non-managed 상태 조회 `mod.rs:830` | `server_executable`, `server_jar`, `working_dir`, `pid`, `rcon_port`, `rcon_password`, `module_settings`(펼침) |
| `command` | `POST /api/instance/:id/command` → `mod.rs:919` | `command`, `args`, `protocol_mode`, `working_dir`, `rcon_host`, `rcon_port`, `rest_host`, `rest_port`, `pid`, `instance_id` |
| `get_credentials` | `/rcon`, `/rest` 요청마다 1회씩 `command.rs:101, 260` | `working_dir` |
| `validate` | `POST /api/instance/:id/validate` `mod.rs:1404` | `server_jar`(=executable_path), `working_dir`, `port`, `extension_data` |
| `configure` | `PUT /properties`, 인스턴스 설정 PATCH 후 동기화 `mod.rs:1435-1447`, `instance.rs:1140-1200` | `working_dir`, `extension_data`, `settings{}` |
| `read_properties` | `GET /properties` | `working_dir`, `extension_data` |
| `reset_properties` | `POST /properties/reset` | 위와 같음 |
| `reset_server` | `POST /server/reset` | 위와 같음 |
| `accept_eula` | `POST /accept-eula` | `working_dir` |
| `diagnose_log` | `POST /diagnose` | `working_dir`, `log_lines[]`(managed일 때 최근 500줄) |
| `list_versions` | `GET /api/module/:name/versions`, 프로비저닝(download 방식) | `include_snapshots`, `page`, `per_page` 또는 `{}` |
| `get_version_details` | `GET /api/module/:name/version/:v` | `version` |
| `install_server` | `POST /api/module/:name/install`(120초), 프로비저닝·업데이트(1800초) | `version`, `install_dir`, `accept_eula`, `jar_name`, `initial_settings` 등 |
| `get_installed_version` | `GET /installed-version`, 데몬 초기화 시 backfill `mod.rs:1668-1711` | `install_dir`, `jar_name`(**기본값 `"server.jar"` 하드코딩**, `mod.rs:1651-1654`) |
| `import_settings` | 마이그레이션 인스턴스 생성 `instance.rs:653` | `working_dir` |

**데몬이 절대 호출하지 않는 함수**:
- `start`: 모든 모듈이 구현하고 문서 §4.4도 "Non-managed 모드 직접 시작"이라고 설명한다. 하지만 non-managed 모드도 `get_launch_command`로 데몬이 직접 spawn한다 (`mod.rs:447-548`). 따라서 `start`는 **죽은 코드**다.

**문서의 "호출 순서"와 다른 점** (`docs/module-development-guide.md:96-125`, `__example__lifecycle.py:38-47`):
- "시작 시 `validate → configure → get_launch_command`"라고 되어 있지만, 데몬은 시작할 때 **`get_launch_command`만** 호출한다. validate와 configure는 별도 API 경로에서만 호출된다.
- "status 주기적 호출"이라고 되어 있지만, 실제로는 non-managed 인스턴스의 status API 요청 때만 호출된다. 모니터 루프는 PID 스냅샷만 본다.
- "rcon_template이 있으면 데몬이 직접 RCON을 보낸다"고 되어 있지만, 실제로는 **GUI와 봇이 템플릿을 치환한 뒤** `/rcon`을 호출한다 (§6.5).
- "config에 `instance_id`, `settings{}` 항상 포함"이라고 되어 있지만, `instance_id`는 `command`에만 들어가고 `settings{}`는 `configure`에만 들어간다. 그 외 함수에서는 `module_settings`가 **최상위로 펼쳐져** 들어간다.

### 2.4 함수 구현 매트릭스 (FUNCTIONS 딕셔너리 기준)

✔ = 디스패치 테이블에 있음, d = 함수는 정의돼 있으나 **FUNCTIONS에 없음**(호출하면 Unknown function), – = 없음.

| 함수 (데몬 호출 여부) | minecraft | palworld | zomboid | __example |
|---|---|---|---|---|
| get_launch_command (호출) | ✔ | ✔ | ✔ | ✔ |
| stop (non-managed만) | ✔ | ✔ | ✔ | ✔ |
| status (non-managed만) | ✔ | ✔ | ✔ | ✔ |
| command (호출) | ✔ | ✔ | ✔ | ✔ |
| get_credentials (호출) | ✔ | ✔ | ✔ | – |
| validate (호출) | ✔ | ✔ | ✔ | ✔ |
| configure (호출) | ✔ | ✔ | ✔ | ✔ |
| read_properties (호출) | ✔ | ✔ | ✔ | – |
| reset_properties (호출) | **d** (`:1181`) | ✔ | – | – |
| reset_server (호출) | **d** (`:1197`) | ✔ | ✔ | – |
| accept_eula (호출) | ✔ | ✔(no-op) | – | – |
| diagnose_log (호출) | ✔ | ✔ | ✔ | ✔(stub) |
| list_versions (호출) | ✔ | ✔ | – | – |
| get_version_details (호출) | ✔ | ✔ | – | – |
| install_server (호출) | ✔ | ✔ | ✔ | ✔(잘못된 API, §6) |
| get_installed_version (호출) | ✔ | – | – | – |
| import_settings (호출) | **d** (`:1131`) | **d** (`:1043`) | ✔ | ✔ |
| start (**미호출**) | ✔ | ✔ | ✔ | ✔(=get_launch_command) |

출처: `M:minecraft/lifecycle.py` FUNCTIONS 블록, `M:palworld/lifecycle.py:1675-1692`, `M:zomboid/lifecycle.py` FUNCTIONS 블록, `M:__example/__example__lifecycle.py` FUNCTIONS 블록.

**영향**:
- minecraft와 palworld의 마이그레이션 import는 항상 실패한다. 실패해도 로그만 남고 넘어간다.
- minecraft의 "설정 초기화"와 "서버 리셋" API는 항상 Unknown function을 돌려준다.
- 데몬 초기화 때의 backfill은 palworld와 zomboid에서 매번 실패한다(debug 로그).

### 2.5 공용 Python 유틸 (`D:modules/`)

- `i18n.py`: `I18n(module_dir)`가 `locales/<lang>.json`을 읽는다. 언어는 `SABA_LANG`으로 정하고 기본값은 `en`이다. 점 표기 키와 `{{var}}` 보간을 지원한다. `__example/i18n.py`는 같은 파일의 복사본이다.
- `daemon_rcon.py`: 데몬 HTTP `/api/instance/:id/rcon`을 감싸는 래퍼다 (`:43-100`).
  - 기본 URL은 `DAEMON_API_URL`, 없으면 `http://127.0.0.1:57474`이다.
  - **`X-Saba-Token` 헤더를 보내지 않는다.** 데몬은 토큰 파일이 있으면 모든 요청에 토큰을 요구한다 (`D:src/ipc/auth.rs:77-115`). 따라서 정상 배포 환경에서 모듈→데몬 콜백은 **401로 실패**할 가능성이 높다. palworld의 `execute_rest_via_daemon`(`:1559-`)와 zomboid의 `_send_stdin_command`(`:1261-1269`)도 같은 문제가 있다.
  - payload의 `rcon_port`와 `rcon_password`는 데몬 `/rcon` 핸들러가 **무시**한다. 핸들러는 `get_credentials`의 값만 쓴다 (`command.rs:134-157`).
  - `rcon_command(host,port,pw,cmd)`를 `instance_id` 없이 호출하면 **소켓으로 RCON에 직접 붙는다** (`:173-205`). minecraft와 zomboid는 instance_id를 넘기지 않으므로 사실상 직접 소켓 RCON을 쓴다. 문서 §6.1의 "데몬이 연결 관리"와 다르다.
- 배포: 코어 패키저가 `modules/i18n.py`와 `modules/daemon_rcon.py`를 데이터 디렉터리의 `modules/`에 넣는다 (`D:.github/scripts/package-release.py:56-57`). installer도 같은 일을 한다 (`installer/gui/src-tauri/src/lib.rs:1171`). 모듈 리포의 zip에는 들어가지 않는다(예외: `__example/i18n.py`).

---

## 3. Managed Process 모델

### 3.1 두 가지 실행 경로

| | managed (`POST /api/instance/:id/managed/start`) | non-managed (`POST /api/server/:name/start`) |
|---|---|---|
| 엔트리 | `Supervisor::start_managed_server` `mod.rs:946-1317` | `Supervisor::start_server` `mod.rs:225-549` |
| 사전 검사 | 중복 실행 방지(managed_store와 tracker PID 생존 확인), 포트 충돌, extension `server.pre_start` hook | 포트 충돌, `requires_extensions` 검증, `server.pre_start` hook |
| spawn | `ManagedProcess::spawn` — stdin, stdout, stderr 파이프 | `TokioCommand` — stdio 모두 `null`, 콘솔 없음 |
| 추적 키 | `tracker.track(instance.id, pid)` | `tracker.track(server_name, pid)` ← **키 불일치** (`mod.rs:540`) |
| PID 영속화 | `<instance_dir>/managed.pid` | 없음 |
| 자격증명 동기화 | `rest_credentials` 또는 `credential_map` | 없음 |

- `requires_extensions` 검증은 **non-managed 경로에만 있다** (`mod.rs:259-318`). managed 경로에는 빠져 있다.
- non-managed 프로세스를 이름으로 추적하므로 몇 가지가 어긋난다.
  - `monitor_processes`는 `instance.id`로 조회한다(`:1738`). 그래서 이름으로 추적된 non-managed 프로세스는 생존 확인 대상에서 빠진다.
  - 포트 충돌 검사의 `running_ids`(`:239-243`)에도 잡히지 않는다.
  - `stop_server`와 `get_server_status`는 `name`과 `id`를 둘 다 시도해서 이 문제를 우회한다 (`:706, 803-805`).

**잠금 문제**: `start_managed_handler`는 **supervisor write lock을 잡은 채로** `start_managed_server`를 실행한다 (`D:src/ipc/handlers/managed.rs:61-66`). 이 안에서 Python `get_launch_command`(최대 120초)가 실행되므로, 그동안 모든 IPC 읽기와 모니터가 블록된다.

### 3.2 상태 머신 (state_machine.rs)

- `State { Stopped, Starting, Running, Stopping, Crashed }`가 정의돼 있다.
- 허용 전이는 Stopped→Starting, Starting→Running/Crashed, Running→Stopping/Crashed, Stopping→Stopped/Crashed, Crashed→Stopped이다 (`D:src/supervisor/state_machine.rs:38-50`).
- 하지만 파일 헤더에 "Supervisor 통합 예정"이라고 적혀 있고 `#![allow(dead_code)]`가 붙어 있다 (`:1-5`). **어디서도 사용하지 않는다.** 테스트 외에 참조가 없다.

데몬이 실제로 노출하는 "상태"는 다음과 같다.
- managed: `running`/`stopped` 두 값이다. `watch::channel<bool>`의 `is_running()`으로 정한다 (`mod.rs:787-798`).
- non-managed: 모듈 `status()`가 돌려준 문자열을 그대로 쓴다. Python이 실패하면 `"unknown"`이다.
- extension hook `server.status`가 `handled: true`를 돌려주면 그 결과가 우선한다.

`ProcessTracker` (`D:src/supervisor/process.rs`)는 `ProcessStatus {Running, Stopped, Crashed}`를 기록하지만, 아래 메서드는 전부 `#[allow(dead_code)]`이고 **호출되지 않는다** (`:132-219`).
- `get_status`
- `mark_crashed`
- `terminate`(SIGTERM/TerminateProcess)

실제 종료에는 `force_kill_pid`(SIGKILL / `taskkill /F`)만 쓰인다.

### 3.3 콘솔 캡처와 로그 파싱 (`managed_process.rs`)

- **링 버퍼**: `LogBuffer`. 기본 10,000줄(`:28`). 단조 증가하는 `id`를 붙여 `get_since(id)`와 `get_recent(n)`으로 폴링한다.
- **브로드캐스트**: `broadcast::channel<LogLine>(2048)`. `subscribe()`가 있지만 IPC는 폴링(`GET /console?since=`) 방식이다.
- **stdout/stderr 리더**: 각각 tokio task로 줄 단위로 읽는다 (`:258-288`). stderr 줄은 레벨이 Info로 파싱되더라도 **Warn으로 올린다**.
- **레벨 파싱**: `parse_log_level(line, log_pattern)`이 `(?P<level>)` 캡처를 매핑한다 (`:703-717`).
  - ERROR/FATAL → Error, WARN/WARNING → Warn, DEBUG/TRACE → Debug, 그 외 → Info.
  - 패턴이 없거나 매치되지 않으면 Info다.
  - 정규식이 틀리면 warn 로그를 남기고 패턴 없이 동작한다.
  - zomboid 패턴의 `LOG` 같은 값도 Info가 된다.
- **디스크 영속화**: `<instance_dir>/logs/console.log`에 append한다. 포맷은 `[datetime] [source/level] content`이다 (`:730-744, 796-866`). 10MB가 되면 로테이션하고 2개(`.1`, `.2`)를 보관한다. writer가 죽으면 콘솔에 경고 줄을 넣는다.
- **데몬 재시작 시 재부착**: `initialize → reattach_orphaned_managed` (`mod.rs:106-159`).
  - `managed.pid`의 PID가 살아 있으면 `create_reattached_stub`을 만든다. 이 스텁은 디스크 로그를 로드하고, 5초마다 PID 생존을 폴링하며, **stdin을 쓸 수 없다**.
  - 이후 stop은 강제 kill만 가능하다 (`mod.rs:615`).
  - 다만 스텁은 원래 프로세스의 stdout을 잃었으므로 **새 로그를 더 수집하지 못한다**. 주석의 "Continues writing new log lines to disk"는 시스템 메시지에만 해당한다.
- **disk fallback**: managed 프로세스가 없으면 `get_console_output`이 디스크 로그를 읽는다. 이때 `since_id`는 무시된다 (`mod.rs:1358-1378`).
- **log follower**: extension(Docker 등)이 `server.pre_start`에서 `log_follower{program,args,working_dir,interactive,strip_prefix,env_vars}`를 돌려주면 `spawn_log_follower`(읽기 전용) 또는 `spawn`(양방향)으로 `docker logs -f` 같은 프로세스를 managed_store에 등록한다 (`mod.rs:352-421, 1054-1123`).

### 3.4 콘솔 입력, 정지, 강제 종료

- **stdin**: `POST /api/instance/:id/stdin` `{command}`가 `send_command`를 부른다. mpsc(256)를 거쳐 writer task가 개행을 붙여 쓴다 (`managed.rs:94-125`, `managed_process.rs:290-305, 343-346`).
- **managed stop** (`mod.rs:608-695`):
  1. `force`이거나 stdin을 쓸 수 없으면 `force_kill_pid`를 한다.
  2. 그렇지 않으면 stdin으로 `stop_command`를 보낸다.
  3. 0.5초 간격으로 `is_running`을 확인한다. 기다리는 시간은 graceful이면 `stop_timeout`(기본 30초), force면 5초다.
  4. 시간이 지나면 SIGKILL을 보내고 2초 더 기다린다.
  5. `managed_store`에서 제거하고, `managed.pid`를 삭제하고, 30초짜리 stop cooldown(`STOP_COOLDOWN_SECS`)을 등록한다.
  - **Python `stop()`은 호출되지 않는다.** 그래서 minecraft의 RCON graceful stop, zomboid의 save→quit 시퀀스 같은 모듈 로직은 managed 모드에서 쓰이지 않는다.
  - palworld는 stdin이 없는 REST 전용 서버인데도 `stop_command = "Shutdown"`을 stdin으로 보낸다. 효과가 없을 가능성이 높아서, 30초 대기 후 강제 종료로 끝날 것으로 보인다.
- **non-managed stop**: Python `stop()`에 `pid`와 `force`를 넘긴다. 모듈이 RCON, REST, kill을 직접 처리한다. `success=false`이면 Err가 된다.
- **extension 위임**: `server.post_stop` hook이 `handled`를 돌려주면 그대로 반환한다. 이 hook은 이름과 달리 **정지 전에** dispatch된다 (`mod.rs:562-581`). 이름과 동작이 맞지 않는다.

### 3.5 RCON/REST 연결

- 데몬에는 `protocol/rcon.rs`, `rcon_pool.rs`, `rest.rs`가 있다.
- **RCON 경로**: `/api/instance/:id/rcon`. 매 요청마다 Python `get_credentials`를 spawn해 게임 설정 파일(server.properties, servertest.ini 등)에서 포트와 비밀번호를 읽는다. 그다음 `rcon_pool.execute`를 `spawn_blocking`으로 실행하고 타임아웃은 5초다 (`command.rs:58-213`).
  - RCON 명령 하나마다 Python 프로세스가 하나 뜬다.
  - 비밀번호를 찾지 못하면 400을 돌려준다. 인스턴스에 저장된 `rcon_password`는 대체값으로 쓰지 않는다.
- **REST 경로**: `/api/instance/:id/rest`도 같은 방식이다. Basic Auth의 username 기본값은 `"admin"`이다.
- **managed와 RCON 정책 충돌**:
  - minecraft는 managed 모드에서 `enable-rcon=false`를 강제한다 (`M:minecraft/lifecycle.py:971-1010`, `:1096-1097`).
  - zomboid도 managed 모드에서는 RCON을 선택 사항(기본 비활성)으로 둔다 (`M:zomboid/lifecycle.py:784-830`).
  - 그런데 두 모듈의 `[[commands.fields]]`는 전부 `method = "rcon"`이다. 그래서 GUI 명령 버튼은 `/rcon`을 호출하고, managed 모드에서 실패한다.
- `SupervisorError`(`error.rs`)는 HTTP 상태 코드 매핑을 갖추고 있지만 `#[allow(dead_code)]`이고 **쓰이지 않는다**. 모든 핸들러가 `anyhow` + 수동 JSON을 쓴다.

### 3.6 크래시 감지와 자동 재시작

- **자동 재시작은 없다.** 관련 코드가 없다.
- **크래시 판정도 없다.**
  - 프로세스가 끝나면 waiter task가 `"Process exited with <status>"`를 System 줄(Info 레벨)로 남기고 `running=false`로 바꾼다 (`managed_process.rs:308-321`). 종료 코드는 구분하지 않는다.
  - `monitor_processes`(`mod.rs:1718-1864`)가 하는 일은 다음과 같다.
    1. `managed_store.cleanup_dead()`로 죽은 항목을 제거하고 `managed.pid`를 삭제한 뒤 cooldown을 등록한다.
    2. 추적 중인 PID가 스냅샷에 없으면 untrack한다.
    3. `auto_detect`가 켜진 인스턴스는 `process_name`(+`cmd_patterns`)으로 외부 프로세스를 찾아 추적한다. 포트가 충돌하면 그 프로세스를 **강제 kill**하고 `PortConflictStopEvent`를 쌓는다.
  - `Supervisor::monitor()`는 로그 한 줄만 찍는 stub이다 (`:1866-1870`).
- `process_name`은 Windows 값(`java.exe`)이다. `[detection.linux]`이 무시되므로 **Linux에서 minecraft와 zomboid의 auto-detect는 매치에 실패한다**.

---

## 4. 모듈별 기능 요약

공통 사항:
- 모든 모듈은 version 0.1.0, author WareAoba다.
- `locales/` 아래에 10개 언어 JSON이 있다: `de, en, es, fr, ja, ko, pt-BR, ru, zh-CN, zh-TW`.
- `[aliases]`(한국어 별칭)와 `[errors]`(봇 친화 메시지)가 있다.

### 4.1 minecraft (`M:minecraft/`, lifecycle 1,994줄, toml 1,078줄)

| 항목 | 내용 |
|---|---|
| protocols | `supported=["rcon","stdin"]`, default rcon, `interaction_mode=console` |
| 설치 | `install.method="download"`. `ServerInstaller`(`:1597-1870`)가 Mojang `version_manifest_v2`(`:1594`)에서 버전 목록을 받아 jar를 내려받는다. Java는 `JavaDetector`와 `JavaDownloader`(Adoptium API `:278`)로 감지하거나 `<working_dir>/java/`에 번들로 받는다 |
| 실행 | `get_launch_command`가 `java -Xmx/-Xms{ram}`, 선택적 Aikar flags, `jvm_args`, `-jar server.jar nogui`를 만든다. 시작 전에 RCON 정책을 강제한다 (managed면 RCON 끔, native면 켜고 비밀번호를 동기화) |
| 설정 | `ServerPropertiesManager`가 `server.properties`를 읽고 쓴다. 모듈 폴더에 동봉된 `server.properties`를 기본값으로 쓴다 (`_load_default_properties` `:501`). settings 69개(saba-chan 5 / advanced 43 / basic 21) |
| 명령 | 17개, 전부 `method="rcon"`: say, give, save-all, list, weather, difficulty, gamemode, time, tp, kick, ban, pardon, op, deop, whitelist, seed, stop |
| 기타 | `EulaManager`(accept_eula), `ErrorDetector`(diagnose_log), `MinecraftPing`(SLP 기반 status), `get_installed_version`(jar 내부 `version.json` 파싱), syntax_highlight 규칙 28개 |
| 미등록 | `import_settings`, `reset_properties`, `reset_server` |
| 동봉 파일 | `_sample_log.txt`, `test_lifecycle.py`(zip에서 제외), `__pycache__`가 리포에 커밋돼 있음 |

### 4.2 palworld (`M:palworld/`, lifecycle 1,701줄, toml 680줄)

| 항목 | 내용 |
|---|---|
| protocols | `supported=["rest"]`, `interaction_mode=commands`, `credential_map.rest_password="AdminPassword"` |
| 설치 | `steamcmd`, app_id 2394010, `requires_extensions=["steamcmd","ue4-ini"]`. `extensions.steamcmd.SteamCMD`를 사용한다 |
| 실행 | `PalServer.exe --port=<port>`. 시작 전 `_enforce_rest_policy`로 INI에 RESTAPIEnabled와 AdminPassword를 보장하고, `AdminPassword`를 최상위로 돌려주어 데몬이 rest_password를 동기화하게 한다 (`:388-434`) |
| 설정 | `PalWorldSettings.ini`의 `OptionSettings=(...)`를 **`extensions.ue4_ini`** 익스텐션으로 파싱한다. 동봉된 `DefaultPalWorldSettings.ini`가 기본값이다. settings 45개 |
| 명령 | 8개, 전부 `method="rest"`(`/v1/api/...`): announce, info, players, save, shutdown, kick, ban, unban. `resolve_player_id`가 닉네임을 SteamID로 해석한다 (`:141`) |
| 의존성 문제 | 최상위 `from extensions.ue4_ini import ...`(`:27`) 때문에 **ue4-ini 익스텐션이 없으면 lifecycle.py 전체가 import 단계에서 실패**한다. `validate`나 `status`도 마찬가지다. 모듈이 플러그인에 하드 의존한다 |
| 미등록 | `import_settings`, `get_installed_version` |
| 기타 | `accept_eula`는 no-op, `list_versions`는 Steam 단일 버전 |

### 4.3 zomboid (`M:zomboid/`, lifecycle 1,753줄, toml 1,463줄)

| 항목 | 내용 |
|---|---|
| protocols | `supported=["rcon","stdin"]`, console, `stop_command="quit"` |
| 설치 | `steamcmd`, app_id 380870. `beta_branch="unstable"`은 로더가 무시한다(§1.2). steamcmd 익스텐션을 쓰고, 없으면 시스템 steamcmd(`_find_steamcmd`)로 대체한다 |
| 실행 | `StartServer64.bat`나 `start-server.sh`를 **파싱**해서(`_parse_bat`, `_parse_sh`, `_parse_java_cmdline` `:544-707`) 번들 Java, classpath, native lib을 찾는다. 그다음 `java ... zombie.network.GameServer -servername <name> -cachedir=...`를 직접 구성한다 |
| 설정 | `IniManager`가 `Zomboid/Server/<servername>.ini`를 다룬다. `_INI_KEY_MAP`(saba 키 → INI 키). settings 118개 |
| 명령 | 23개, 전부 rcon: servermsg, players, save, kick, banuser, unbanuser, adduser, setaccesslevel, teleport, teleportto, createhorde, godmod, invisible, startrain, stoprain, chopper, gunshot, additem, addxp, showoptions, reloadoptions, changeoption, quit |
| 버그 | `stop()`이 `config["managed"]` 기본값을 True로 보고 daemon `/stdin`을 호출한다. 그런데 (1) 데몬은 `stop`을 non-managed에서만 호출하고 `managed`나 `instance_id` 키를 넘기지 않는다. (2) payload 키가 `{"input": ...}`인데 데몬은 `command`를 요구한다(`managed.rs:101`) → 400. (3) 토큰 헤더가 없다 |
| 미구현 | `accept_eula`, `list_versions`, `get_version_details`, `get_installed_version`, `reset_properties` |

### 4.4 __example (`M:__example/`)

- 파일명이 `__example__module.toml`, `__example__lifecycle.py`다. 로더는 정확히 `module.toml`과 `lifecycle.py`를 요구하므로 **이름을 바꿔야** 동작한다. 폴더명도 `_`로 시작해 빌드에서 제외된다(의도된 것).
- `name = "mygame"`인데 폴더명은 `__example`이다. toml 주석은 "폴더명과 동일"을 요구한다(예시 값).
- 템플릿 주석에 있는 "데몬 호출 시점" 설명(`:38-47`)과 config 키 설명(`:58-80`)은 §2.3과 같은 이유로 **사실과 다르다**.
- `install_server`가 `from extensions.steamcmd import install_game`을 쓰는데, 실제 모듈들은 `SteamCMD` 클래스를 쓴다. 이 API가 존재하는지 **확인되지 않았다**.
- `get_credentials`, `read_properties`가 없다. 그래서 이 템플릿으로 만든 RCON 모듈은 `/rcon` 명령이 항상 "RCON password not configured"로 실패한다 (`command.rs:145-156`).
- settings에 `managed_start`와 `graceful_stop`이 있다. `managed_start`는 GUI와 봇이 시작 경로를 고르는 데 쓴다. `graceful_stop`은 데몬 어디에서도 읽지 않는다(문서는 stop config에 들어간다고 주장).

### 4.5 i18n과 locales

- 모듈 측: `I18n(MODULE_DIR)`이 `locales/<SABA_LANG>.json`을 읽고, 없으면 `en`으로 대체한다. 키 구조는 `module`, `settings`, `commands`, `errors`, `messages`, `rcon` 등이다(minecraft ko.json 기준).
- 인터페이스 측: `GET /api/module/:name/locales`가 `<modules_dir>/<name>/locales/*.json`을 하나의 맵으로 묶어 돌려준다 (`D:src/ipc/handlers/config.rs:102-134`). GUI는 이를 `mod_<name>` 네임스페이스로 쓴다. 이 경로 계산은 `module.path`가 아니라 **모듈 이름으로 만든 경로**라서, ZIP 모듈(`.extracted/<name>`)이나 폴더명이 name과 다른 모듈에서는 locales가 비어 나온다.

---

## 5. 패키징과 배포

### 5.1 `build_modules.py` (`M:.github/scripts/build_modules.py`)

- **탐색**: 리포 루트의 1단계 디렉터리 중 `module.toml`이 있고 이름이 `.`이나 `_`로 시작하지 않는 것 (`:83-90`). 그래서 `__example`은 제외된다.
- **파싱**: `tomllib`(BOM 제거)을 쓰고, 없으면 `[module]` 섹션만 읽는 정규식 파서로 대체한다. 추출 항목은 `name`, `version`, `description`, `display_name`, `game_name`, `entry`다.
- **zip**: `module-<name>.zip`. 모듈 디렉터리 기준 **평탄한 상대경로**로 묶는다(최상위 폴더 없음). `__pycache__`, `.pytest_cache`, `.git`, `*.pyc/.pyo`, `test_*`는 제외한다. `_sample_log.txt`, `.ini`, `.properties` 등은 포함된다.
- **manifest.json** (`:303-320`):
  ```json
  { "schema_version": 1, "generated_at": "...",
    "modules": { "<name>": { "version", "asset": "module-<name>.zip", "sha256": "<zip sha256>",
                             "install_dir": "modules/<name>", "display_name", "description", "game_name" } } }
  ```
- **변경 감지**: 이전 릴리스의 manifest와 version과 sha256을 비교한다. 변경이 없으면 릴리스를 만들지 않는다. `FORCE_RELEASE`로 강제할 수 있다.
- **태그**: `modules-YYYYMMDD-HHMMSS`.

### 5.2 워크플로 (`M:.github/workflows/release-modules.yml`)

- **트리거**: `main` push 중 `*/module.toml`, `*/lifecycle.py`, `*/i18n.py`, `*/locales/**`, `*/icon.png`, `**/*.ini`, `**/*.properties` 경로, 또는 수동 실행.
  - 다른 파일(예: 추가 `.py` 헬퍼, `_sample_log.txt`)만 바꾸면 릴리스가 생기지 않는다.
- **순서**: 이전 manifest를 받는다(`gh release download`) → 빌드한다 → **기존 릴리스를 모두 삭제한다**(`gh release list --limit 100 | gh release delete --cleanup-tag`) → 새 릴리스를 만든다(manifest.json과 *.zip).
  - 그 결과 항상 **릴리스가 하나만** 존재한다. 롤백이나 과거 버전 고정은 불가능하다.

### 5.3 데몬과 업데이터의 다운로드·설치

- **URL**: owner는 `WareAoba`, repo는 `saba-chan-modules`다 (`D:updater/src/constants.rs:25-60`).
  - manifest: `https://github.com/WareAoba/saba-chan-modules/releases/latest/download/manifest.json`
  - 에셋: `https://github.com/WareAoba/saba-chan-modules/releases/latest/download/module-<id>.zip`
- **`GET /api/modules/manifest`**: 위 manifest를 프록시해서 돌려준다 (`server.rs:443-457`).
- **`POST /api/modules/manifest/:id/install`** (`server.rs:460-516`):
  1. `module-<id>.zip`을 `<modules>/_tmp_module_<id>.zip`로 받는다.
  2. `<modules>/<id>/`에 `archive.extract`로 푼다. zip이 평탄한 구조라서 결과가 맞게 나온다.
  3. 임시 zip을 지우고 캐시를 새로고침한다.
  - **manifest의 sha256을 검증하지 않는다.**
  - `module_id`의 경로를 검증하지 않는다. `remove_module`(`:526`)에는 `..`, `/`, `\` 검사가 있지만 install에는 없다.
  - 기존 디렉터리를 정리하지 않고 덮어써서 풀기 때문에, 업데이트하면 삭제된 파일이 남는다.
  - HTTP 에러도 200 `{ok:false}`로 돌려준다.
- **`DELETE /api/modules/:id`**: `<modules>/<id>`를 `remove_dir_all`로 지운다. 여기서 `id`는 **디렉터리명**이다.
- **업데이터(코어 업데이트 흐름)** (`D:updater/src/lib.rs`):
  - `discover_module_repos()`가 로컬 `module.toml`의 `[update].github_repo`를 읽는다(`:806-830`). 그 리포의 **최신 릴리스 태그**를 `trim_start_matches('v')`해서 latest_version으로 쓴다(`:742-803`).
  - 그런데 모듈 리포의 태그는 `modules-20261016-...`라서 SemVer로 파싱되지 않는다. `compare_versions`는 `(None, _) => false`이므로(`:921-929`) **모듈 자동 업데이트는 영원히 "업데이트 없음"으로 판정될 가능성이 높다**. 모듈별 버전은 manifest에 있지만 이 경로는 manifest를 읽지 않는다.
  - 3개 모듈이 모두 같은 `github_repo`를 가리키므로 같은 릴리스를 세 번 조회한다.
  - lib.rs:32의 doc 예시 manifest 형식(`"module-minecraft": {...}` + `components`)은 실제 build_modules.py 출력(`modules.<name>`)과 다르다.
  - 무결성 검사(`integrity::collect_hashes_from_module_manifest`, `D:updater/src/integrity.rs:562-603`)는 manifest의 sha256(**zip 파일의 해시**)을 쓴다. 그런데 비교 대상은 `modules/<name>/lifecycle.py` (`integrity.rs:274-275`)라서 **원리상 항상 불일치한다**. 구현을 직접 실행해 보지는 않았다.
- **installer**: 원격 module.toml을 받아 표시 정보를 만든다. 원격 실패 시 minecraft, palworld, zomboid 하드코딩 목록으로 대체한다 (`installer/gui/src-tauri/src/lib.rs:169-195`).

---

## 6. 불일치, 죽은 코드, 미완성, 원칙 위반

### 6.1 로더와 모듈·문서 사이의 불일치
1. `[module].entry`가 필수인데 사용되지 않고, `lifecycle.py`가 하드코딩돼 있다 (§1.1).
2. command `inputs`의 `options`, `min`, `max`가 버려진다. 문서는 지원한다고 한다 (`module_loader.rs:552-565` vs `docs:874-897`).
3. `[detection.linux]` 전체, `install.beta_branch`, `detection.common_paths`, `config.*` 추가 키, `[errors]`(데몬 메타 기준), `game_name`, `display_name`이 무시된다.
4. `CommandField.method` 주석은 `both`인데 실제 인터페이스 분기는 `dual`이다 (`module_loader.rs:201` vs `saba-chan-gui/main.js:2343`, `discord_bot/core/processor.js:585`). 현재 어떤 모듈도 `dual`을 쓰지 않는다.
5. `/api/modules` 응답(`ExtensionInfo`, `D:src/ipc/mod.rs:262-286`)에 `game_name`과 `display_name`이 없다. 그런데 GUI 마이그레이션 화면은 `ext.game_name`을 참조한다 (`AddInstanceMigration.js:83-84`).
6. ZIP 로딩에서 `ends_with("module.toml")`로 첫 엔트리를 고르는 문제와 `.extracted` 재추출 문제가 있다. locales와 설치 경로는 `<modules>/<name>`을 가정한다 (§1.1, §4.5).

### 6.2 데몬 측 죽은 코드와 stub
| 대상 | 위치 |
|---|---|
| `state_machine.rs` 전체 (`#![allow(dead_code)]`, "통합 예정") | `state_machine.rs:1-61` |
| `SupervisorError` 전체 | `error.rs:8,35` |
| `ProcessTracker::get_status`, `mark_crashed`, `terminate`, `ProcessStatus::Crashed` | `process.rs:132-219` |
| `Supervisor::monitor()` (로그만 찍는 stub), `run()` | `mod.rs:1866-1895` |
| `ModuleMetadata::steam_app_id`, `has_extension_config`, `check_dependencies` | `module_loader.rs:119-161` |
| `ModuleLoader.get_module`에 `#[allow(dead_code)]`가 붙어 있지만 실제로 광범위하게 사용됨 (주석 오류) | `module_loader.rs:888` |
| `LoadedModule.is_zip` 미사용 | `:240` |
| `plugin::run_extension*` `#[allow(dead_code)]` | `plugin/mod.rs:265-284` |
| `ManagedProcess::wait_for_exit`, `subscribe` — IPC에서 사용하지 않음 | `managed_process.rs:522-545` |

### 6.3 모듈 측 미완성과 버그
1. FUNCTIONS 누락 (§2.4): minecraft의 `import_settings`, `reset_properties`, `reset_server`, palworld의 `import_settings`.
2. 모든 모듈의 `start()`가 죽은 코드다.
3. 모듈→데몬 HTTP 콜백(daemon_rcon, palworld REST, zomboid stdin)이 인증 토큰을 보내지 않는다. 포트도 57474로 하드코딩돼 있다.
4. zomboid `stop`의 stdin 콜백은 payload 키가 틀리고(`input` vs `command`), managed 판정도 틀렸다.
5. palworld가 최상위에서 `extensions.ue4_ini`를 import해서 익스텐션에 하드 의존한다.
6. `__example`의 `install_server`가 쓰는 steamcmd API(`install_game`)가 실제 모듈의 API(`SteamCMD` 클래스)와 다르다.
7. `get_installed_version`의 `jar_name` 기본값이 `"server.jar"`다. 데몬 코드 안의 Minecraft 전용 가정이다 (`mod.rs:1651-1654`).

### 6.4 데몬 프로토콜과 운영 문제
1. `SABA_ENABLED_EXTENSIONS`의 상태 파일 경로가 `ExtensionManager`와 다르다. Linux에서는 cwd 기준이 된다 (`plugin/mod.rs:140-146` vs `constants.rs:218-220`).
2. 데몬이 `SABA_LANG`과 `DAEMON_API_URL`을 주입하지 않는다(문서는 주입한다고 주장).
3. `/api/module/:name/install` 경로의 타임아웃이 120초다.
4. non-managed는 이름으로, 나머지는 ID로 추적한다 (`mod.rs:540`).
5. `managed/start`가 write lock을 잡은 채로 Python을 호출한다 (`managed.rs:61`).
6. `server.post_stop` hook을 정지 전에 dispatch한다.
7. managed 모드에서는 RCON을 끄는데, 명령은 RCON으로 라우팅된다 (§3.5).
8. RCON/REST 명령 하나마다 `get_credentials` Python 프로세스를 띄운다.
9. 모듈 설치에 sha256 검증, 경로 검증, 클린 설치가 없다. 업데이터의 태그→semver 판정은 실패하고, 무결성 해시 대상도 맞지 않는다.

### 6.5 [원칙 위반] 모듈 지식이 데몬 바깥이나 데몬 하드코딩에 있는 곳

| # | 위치 | 내용 | 원칙 |
|---|---|---|---|
| 1 | `saba-chan-gui/main.js:2281-2360`, `discord_bot/core/processor.js:577-594` | **명령 실행 로직이 인터페이스에 있다.** `method`에 따라 rcon/rest/dual 분기, `rcon_template`의 `{var}` 치환, 미치환 토큰 제거, `endpoint_template`/`http_method` 기본값(`/v1/api/<cmd>` — Palworld 전용 경로를 범용 기본값으로 씀), 입력 검증을 GUI와 봇이 **각각 중복 구현**한다. 데몬은 raw `/rcon`과 `/rest`만 제공한다 | 인터페이스는 스스로 아무것도 하면 안 됨. 모듈 명령 해석은 데몬(또는 모듈)이 맡아야 함 |
| 2 | `saba-chan-gui/src/hooks/useServerActions.js:230-237`, `discord_bot/core/processor.js:440-442` | managed와 native 중 어느 경로로 시작할지(`module_settings.managed_start`, `interaction_mode`)를 **인터페이스가 결정**해 `/managed/start`나 `/server/:name/start`를 고른다 | 시작 정책은 데몬 소관 |
| 3 | `AddInstanceMigration.js:74-82` | `dir_signatures` 매칭, 즉 기존 서버 폴더 → 모듈 판정을 GUI가 수행 | 데몬 기능이어야 함 |
| 4 | `D:saba-chan-cli/src/module_registry.rs:218-261` | CLI가 데몬 응답 실패 시 `module.toml`을 **파일시스템에서 직접 파싱**하는 대체 경로를 가짐 | 인터페이스가 데몬을 우회 |
| 5 | `D:src/ipc/handlers/server.rs:389-408` + 봇 `processor.js:602-605` | 데몬 메타데이터에 `[errors]`가 없어서, 봇이 `/api/module/:name`의 **raw TOML 덤프**를 읽어 에러 메시지를 직접 매핑 | 스키마 우회. 데몬이 정식 필드로 제공해야 함 |
| 6 | `D:src/ipc/handlers/instance.rs:1145-1167` | 설정 동기화 제외 목록에 `java_path`, `ram`, `use_aikar_flags`(Minecraft 전용 키)와 `managed_start`, `graceful_stop`이 **데몬에 하드코딩**돼 있고, `enable_rcon` 주입도 있음. 주석·로그는 "server.properties"(Minecraft 파일명) | 게임별 지식이 데몬에 있음 (모듈 매뉴얼 소관) |
| 7 | `D:src/supervisor/mod.rs:216, 1393, 1651-1654, 1452-1466` | `server_jar` 키 주입, `jar_name="server.jar"` 기본값, `accept_eula`/`manage_properties`라는 Minecraft식 명명이 데몬 공용 API에 들어가 있음 | 동일 |
| 8 | `saba-chan-gui/src/components/ServerSettingsModal.js:354,363` (`module === 'palworld'`), `useServerSettings.js:256` ("Install Minecraft …" 문자열) | GUI가 특정 게임 이름으로 분기 | 인터페이스에 게임 지식 |
| 9 | `M:palworld/lifecycle.py:27`, `zomboid`/`palworld` `install_server` | 모듈이 플러그인(`extensions.*`)을 Python import로 직접 사용. 활성 여부도 스스로 `SABA_ENABLED_EXTENSIONS`로 확인 | 플러그인 기능 주입은 데몬이 중개해야 하는데, 모듈과 플러그인이 직접 결합 |
| 10 | `modules/daemon_rcon.py`, palworld, zomboid 콜백 | 모듈이 데몬 HTTP API를 역호출(rcon/rest/stdin). 인증 없이 호출하고 포트도 하드코딩 | "매뉴얼"인 모듈이 능동 클라이언트로 동작. 경계가 모호하고 현재 인증 때문에 깨짐 |
| 11 | `installer/gui/src-tauri/src/lib.rs:169-195` | 설치기에 모듈 목록(minecraft, palworld, zomboid)이 하드코딩된 대체값 | 경미 |

---

### 부록: 핵심 수치
- 모듈 크기: minecraft lifecycle 1,994줄 / toml 1,078줄, palworld 1,701 / 680, zomboid 1,753 / 1,463, example 555 / 367.
- settings 필드 수: minecraft 69, palworld 45, zomboid 118, example 11. commands 수: 17 / 8 / 23 / 3.
- 데몬의 lifecycle 호출 함수: 16종(§2.3). 모듈이 구현하지만 데몬이 호출하지 않는 함수: `start` 1종.
