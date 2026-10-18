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


> **2026-10-17 L3 이후**: 업데이트·모듈 목록·익스텐션 목록은 모두 릴레이의 서명된 릴리즈 manifest 하나에서 나온다. GitHub 업데이트 경로(`github.rs`, `api_base_url`, `github_owner`/`github_repo`, `include_prerelease`, 익스텐션 `manifest_url`, module.toml `[update]`, modules·extensions 저장소의 릴리즈 워크플로)는 없다. 근거 커밋: saba-chan 65e1763·8b840f7·60768a7, modules a26bdeb, extensions 301ff2c.

## 1. 모듈 발견과 로딩

### 1.1 탐색 위치

| 항목 | 값 | 근거 |
|---|---|---|
| 모듈 루트 | `SABA_MODULES_PATH` 환경변수. 없으면 `resolve_data_dir()/modules`, 즉 Windows는 `%APPDATA%/saba-chan/modules`, Linux/macOS는 `$HOME/.config/saba-chan/modules` (`SABA_DATA_DIR`로 override 가능) | `D:updater/src/constants.rs:55-101`, `D:src/plugin/mod.rs:76-82` |
| GUI가 데몬을 띄울 때 | GUI가 `SABA_MODULES_PATH`를 명시적으로 주입 | `D:saba-chan-gui/main.js:593-602` |
| 레거시 경로 | `Supervisor::run()`은 지워졌다. `"./modules"`는 테스트에만 남았다 | `D:src/supervisor/mod.rs:2272-` |

`ModuleLoader::discover_modules()` (`D:src/supervisor/module_loader.rs:1086-1153`)의 동작은 다음과 같다.

- 모듈 루트의 **1단계 엔트리만** 순회하고 재귀하지 않는다.
- **폴더 모듈**: `<dir>/module.toml`이 있는 디렉터리만 로드한다 (`:1126-1146`). 폴더 이름은 아무 것이나 되며, 식별자는 `[module].name`이다.
- **ZIP 모듈**: 확장자가 `.zip`인 파일이 대상이다 (`:1104-1124`, `load_module_from_zip` `:1161-1218`).
  - 이름이 정확히 `module.toml`인 엔트리 중 **가장 얕은 것**을 파싱한다 (`:1164-1175`). `[module].name`이 폴더 이름 하나가 아니면 거부한다 (`:1189-1191`).
  - 아카이브 전체를 tempdir에 풀었다가 `<modules>/.extracted/<name>/`로 복사한다. 이 과정에서 기존 디렉터리는 `remove_dir_all`로 지운다.
  - 캐시가 무효화될 때마다(`refresh_modules`) 다시 풀린다. 그동안 실행 중인 Python 프로세스가 그 디렉터리를 쓰고 있어도 삭제된다.
- 결과는 `RwLock<Option<Vec<LoadedModule>>>`에 캐시된다. `invalidate_cache()`(`:1080`)는 `/api/modules/refresh`, 모듈 설치·삭제 시에만 호출된다.
- 로드에 실패한 모듈은 `warn` 로그를 남기고 `load_errors`에 기록한다. `GET /api/modules`가 `failed[]`로 돌려준다 (`:1116-1144`, `D:src/ipc/handlers/server.rs:338-339`).
- `.py` 파일(`daemon_rcon.py`, `i18n.py`)이 같은 루트에 있어도 무시된다. 이 둘은 PYTHONPATH용 공용 라이브러리다 (§2.1).

**진입점 파일**: `[module].entry`가 데몬이 실행하는 Python 파일이다(보통 `lifecycle.py`). `LoadedModule::entry_path()`가 `<모듈 폴더>/<entry>`를 만들고(`D:src/supervisor/module_loader.rs:319-321`), 데몬의 모든 lifecycle 호출이 이 경로를 쓴다(예: `D:src/supervisor/mod.rs:598, 752, 970, 1329`). 로드할 때 `check_loadable()`이 이 파일이 실제로 있는지 확인하고, 없으면 모듈을 로드하지 않는다(`module_loader.rs:324-338`). 검증 단계는 `entry`가 비었거나 절대 경로이거나 모듈 폴더 밖을 가리키면 거부한다(`:1001-1007`). (qcza9f 기준)

### 1.2 module.toml 스키마 (Rust 역직렬화 기준, 전체)

파서는 `parse_module_toml()` → `serde_ignored::deserialize::<ModuleToml>` → `into_metadata()`이다 (`module_loader.rs:957-997, 806-953`). 이 구조체들이 계약 그 자체이고, 같은 구조체에서 JSON Schema가 나온다(`module_json_schema()` `:1049-1051`, `GET /api/modules/schema`, `saba-core module schema`). 모듈 저장소는 이것을 `M:schema/module.schema.json`으로 두고 `M:.github/workflows/check-modules.yml`에서 검사한다.

- **모르는 키·잘못된 값**: `[module] schema_version = 1`이면 모듈 로드가 실패한다. `schema_version`이 없으면(스키마 이전 모듈) `warnings`에만 남기고 로드한다. 데몬보다 새 버전이면 거부한다 (`:973-994`). 값 검사는 `entry` 경로, `risk` 값, `config.announce_command`·`backup.save_command`가 `[[commands.fields]]` 이름을 가리키는지다 (`:999-1033`). 세 실제 모듈은 모두 `schema_version = 1`이다.
- **알고 있는 키의 타입이 틀리면 모듈 전체 로드가 실패**한다. 예를 들어 settings의 `options`에 숫자를 넣으면 `Vec<String>`이라 실패한다.
- BOM: 파서가 앞의 UTF-8 BOM을 떼고 읽는다 (`:958`). 지금 `palworld/module.toml`에는 BOM이 없다.

표의 기호: ✔ = 필수(없으면 파싱 실패), ○ = 선택.

#### `[module]` (섹션 자체가 필수, `ModuleSection` `:463-490`)

| 키 | 타입 | 필수 | 기본값 / 비고 |
|---|---|---|---|
| `schema_version` | u32 | ○ | 지금은 1. 위 규칙 참조 |
| `name` | String | ✔ | 모듈 ID. 인스턴스의 `module_name`과 매칭됨 |
| `version` | String | ✔ | semver 형식 기대(검증 없음) |
| `entry` | String | ✔ | 실행할 Python 파일. 파일이 없으면 모듈 로드 실패 (위 1.1 참조) |
| `description` | String | ○ | None |
| `icon` | String | ○ | 모듈 폴더 기준 상대경로. `/api/modules`가 base64로 인코딩해 응답 (`D:src/ipc/handlers/server.rs:294-300`). MIME은 무조건 `image/png` |
| `log_pattern` | String(regex) | ○ | `(?P<level>…)` 명명 그룹 필수. 콘솔 레벨 파싱에 쓰임 (§3.3) |
| `dependencies` | Table<String,String> | ○ | `[module.dependencies]`, 값은 `">=x.y.z"`. 두 단계로 강제된다. ① 로드: `saba-core` 요구 버전이 데몬보다 높으면 모듈을 로드하지 않고 `GET /api/modules`의 `failed[]`에 `error_code: "module_dependency_unsatisfied"`, `missing_dependencies`와 함께 싣는다 (`module_loader.rs:324-338, 349-387`, `D:src/ipc/handlers/server.rs:338`). ② 시작: 인스턴스 시작 전에 `saba-core`와 켜진 익스텐션(`ext-<id>`) 요구를 다시 확인하고, 못 맞추면 `module_dependency_unsatisfied`와 `missing_dependencies`로 시작을 거부한다 (`D:src/supervisor/mod.rs:302-345, 388, 1153`). 다른 키(`gui`, `cli` 등)는 데몬이 알 수 없어 넘긴다 |
| `game_name`, `display_name`, `author` | String | ○ | 메타데이터에 실리고 `/api/modules` 응답에도 들어간다 (`D:src/ipc/mod.rs:255-260`) |

#### `[update]` (없어짐)
L3에서 지웠다. 모듈 업데이트는 릴리즈 manifest의 `module-<이름>` 항목으로 판정한다(§5). 옛 module.toml에 남아 있어도 모르는 섹션이라 조용히 무시된다.

#### `[protocols]` (`ProtocolsSection` `:524-534`)
| 키 | 타입 | 사용처 |
|---|---|---|
| `supported` | Vec<String> | `stdin`/`rest` (`rcon`도 문법상 받지만 데몬이 RCON을 쓰지 않는다). 포트 충돌 검사(`build_module_protocols_map`, `D:src/supervisor/mod.rs:224-235`), 인스턴스 생성 시 rest_port 기본값 결정(`instance.rs:97-101`), 명령·정지를 stdin으로 보낼지 판단(`mod.rs:762-764, 1493-1508`) |
| `default` | String | GUI로 전달만 함 (`ProtocolsInfo`) |
| `interaction_mode` | String | `console` 또는 `commands`. GUI가 콘솔 버튼 같은 UI를 고르는 데 쓴다 (`saba-chan-gui/src/components/ServerCard.js:417-421`, `App.js:494-500`). 데몬은 해석하지 않는다. 데몬은 모든 모듈을 기본 managed로 시작하고(`mod.rs:1479-1489`), 그 판정을 `GET /api/servers`의 `ServerInfo.managed`로 알려 준다(`D:src/ipc/mod.rs:208-210`, `server.rs:81`). 그런데 GUI는 이 값을 쓰지 않고 아직 `managed_start`가 없으면 `console`만 managed로 본다(`ServerCard.js:418-421`, `App.js:494-500`). 그래서 palworld 같은 `commands` 모듈은 데몬은 managed로, GUI(콘솔 버튼·종료 확인)는 non-managed로 다룬다 |

> **RCON 정책**: 보안상 saba-chan은 RCON을 쓰지 않는다. 데몬에는 RCON 클라이언트가 없고, 게임 명령은 모두 서버 콘솔(stdin)로 간다. `/api/instance/:id/rcon`은 이름만 남은 호환 경로로, 한 줄을 stdin에 쓰고 콘솔 출력을 약 1.5초 모아 `data.response`로 돌려준다(`D:src/ipc/handlers/command.rs:263-307`). 콘솔이 없는 서버(관리하지 않는 모드, runner 없이 붙은 프로세스)는 409 `no_command_path`를 받으므로 saba-chan에서 명령을 보낼 수 없다. 데몬이 재시작해도 saba-runner에 다시 붙어 stdin을 유지한다. minecraft는 모든 모드와 설치 시점에 `enable-rcon=false`를 강제하고, zomboid는 `RCONPassword`를 비운다. 두 모듈의 RCON 설정 항목, RCON 폴백(stop·status·command), `get_credentials`는 지워졌고 `supported = ["stdin"]`이다. palworld도 RCON 명령 경로(RCON 클라이언트, `protocol_mode=rcon` 분기, `rcon_enabled`·`rcon_port` 키 매핑, `rcon_password` 반환)를 지웠다. 명령은 늘 데몬 REST 경로로 가고, `configure()`는 저장할 때마다 `RCONEnabled=False`를 쓴다 (`M:palworld/lifecycle.py:933, 1224-`). 콘솔 명령은 `method = "console"` + `console_template`이다(예전 이름 `rcon`·`rcon_template`도 별칭으로 읽힌다).

#### `[config]` (`ConfigSection` `:537-554`)
| 키 | 타입 | 사용처 |
|---|---|---|
| `executable_path` | String | `/api/modules` 응답에 그대로 노출. 데몬은 이 값으로 실행하지 않음 |
| `server_executable` | String | `working_dir + server_executable`로 실행파일 자동 해석 (`mod.rs:243-283`), 설치 시 백업 대상(`:1781-1806`), 마이그레이션 (`instance.rs:508-515`) |
| `process_name` | String | 인스턴스 생성 시 `instance.process_name`에 복사 → auto-detect (`instance.rs:93`, `mod.rs:2154-2164`) |
| `default_port` | u16 | 인스턴스 생성 시 `instance.port` 기본값 |
| `stop_command` | String | managed stop 때 stdin으로 전송. 기본값 `"stop"` (`mod.rs:766`) |
| `stop_timeout` | u64(초) | managed graceful stop 대기 시간. 기본값 30 (`mod.rs:791`) |
| `announce_command` | String | 게임 안 공지에 쓰는 `[[commands.fields]]` 이름 (예약 재시작 예고 등) |

`java_path`·`ram` 같은 게임 설정은 `[config]`가 아니라 `[[settings.fields]]`에 있다. `[config]`의 모르는 키는 schema v1에서 로드 오류다.

#### `[detection]` (`DetectionSection` `:558-572`)
| 키 | 타입 | 사용처 |
|---|---|---|
| `process_patterns` | Vec<String> | extension hook `server.status` 컨텍스트로만 전달 (`mod.rs:919-930`). 네이티브 auto-detect에는 **미사용** |
| `cmd_patterns` | Vec<String> | auto-detect 때 커맨드라인 필터 (`mod.rs:2155-2164`) |
| `dir_signatures` | Vec<String> | `/api/modules`로 노출. 데몬도 `POST /api/modules/detect`로 매칭한다(`server.rs:370-390`, 전부 있어야 함). GUI 마이그레이션 화면도 이 결과만 쓴다: `migration:scanDir`가 이 API를 부르고(`saba-chan-gui/main.js:2735-2745`), 화면은 돌아온 `matches`로 모듈을 고른다(`AddInstanceMigration.js:73-74`) |
| `graceful_exit_patterns` | Vec<String> | 정상 종료 문구(정규식). 종료 판정에 쓴다 (§3.6) |
| `crash_artifacts` | Vec<String> | 크래시 때 생기는 파일. 종료 판정에 쓴다 (§3.6) |

`common_paths`와 `[detection.linux]`는 스키마에 없고, 실제 모듈에서도 지워졌다.

#### `[launch]` (`LaunchSection` `:493-501`)
`log_files`(서버가 파일에 쓰는 로그, saba-runner가 따라 읽어 콘솔에 합친다), `console`(`pipe` 기본 / `pty`). palworld가 `console = "pty"`를 쓴다.

#### `[backup]` (`ModuleBackup` `:113-128`)
`paths`(`{설정 이름}` 치환), `save_command`(백업 전에 실행할 명령 이름), `hook`(true면 lifecycle `backup_paths()`가 경로를 준다). 세 모듈 모두 선언한다.
- `[backup]`이 없는 모듈: 수동 백업만 400 `backup_unsupported`이고, 자동 백업(업데이트 전·예약·복원 전)은 건너뛴다. 그래서 업데이트가 막히지 않는다 (`D:src/ipc/handlers/backup.rs:74-84, 138-145`).
- `hook = true`인데 `backup_paths()`가 실패하면 500 `backup_failed`로 멈춘다. 경로를 모른 채 안전 백업 없이 업데이트·복원이 진행되지 않게 하려는 것이다 (`backup.rs:112-123`).
- 복원할 때 남기는 복원 전 백업이 오래된 자동 백업을 정리해도, 복원 대상 백업은 지우지 않는다 (`D:src/backup/mod.rs:200-210`, `backup.rs:221-222`). 백업 파일 이름에는 밀리초가 붙는다 (`backup/mod.rs:103-104`).

#### `[[settings.fields]]` (`SettingFieldToml` `:583-611` → `SettingField` `:284-301`)
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
| `port` | bool | `false`. 네트워크 포트 값이면 true. 같은 게임 인스턴스를 여럿 만들 때 데몬이 겹치지 않게 옮긴다 |
| `install` | bool | `false`. 서버를 만들 때 고르는 설정(받을 서버 파일이 달라지는 값). GUI 새 서버 화면과 CLI `server add`가 묻고 `POST /api/instances`의 `module_settings`로 보낸다. GUI는 select만 보인다 |
| `arg` | String 또는 Vec<String> | — 이 값이 실행 인자로 들어가는 모양. 공백으로 나눈 토큰, `{value}`는 많아야 한 번(`"-port={value}"`, `"-servername {value}"`, `["-Xmx{value}", "-Xms{value}"]`). `{value}`가 없으면 boolean 플래그(있으면 true). `{value}`뿐인 모양은 로드 오류. 실행 인자 미리보기가 사용자가 친 추가 인자를 이 필드로 되돌릴 때 쓴다(`src/launch_args.rs`). 실제 실행 줄은 여전히 `get_launch_command`가 만든다 |
| `format` | String | — 값 형식: `memory_size`(숫자+K/M/G 한 글자, `4G`·`1024M`), `port`(1~65535 정수), `integer`, `path`(비어 있지 않고 줄바꿈 없음). 모르는 이름은 로드 오류 |
| `pattern` | String | — 값 전체가 맞아야 하는 정규식(잘못된 정규식은 로드 오류). `format`과 함께 PATCH 저장과 미리보기에서 검사한다(`validator::validate_format`) |

- `fields`가 비어 있으면 `settings = None`이 된다.
- 데몬이 직접 의미를 부여하는 setting 이름은 `rest_port`(기본 8212), `rest_host`(기본 127.0.0.1) (`:158-166`), 시작 경로를 고르는 `managed_start`(`mod.rs:1479-1489`), 자동 재시작의 `auto_restart`·`restart_on_unknown_exit`(`mod.rs:1993-1994`), 세션 로그 보관 수 `log_sessions_keep`(`mod.rs:1436-1439`)이다.
- 실제 모듈에서 쓰이는 type 값: `text`, `number`, `boolean`, `select`, `password`, `file`.

#### `[[commands.fields]]` (`CommandFieldToml` `:714-734` → `CommandField` `:246-261`)
| 키 | 타입 | 비고 |
|---|---|---|
| `name` | String ✔ | |
| `label` | String | 없으면 `name` |
| `description` | String | |
| `method` | String | `console`(템플릿을 채워 콘솔 stdin으로. 예전 이름 `rcon`은 읽을 때 `console`로 바뀐다, `D:src/supervisor/module_loader.rs:836`) / `rest` / `module`(lifecycle `command()`, 예전 이름 `dual`·`both`도 받음) / 생략(=`module`). 해석은 데몬 `crate::commands`가 한다 (`D:src/commands/mod.rs:104-128`) |
| `http_method` | String | REST 전용. 기본 `GET` |
| `endpoint_template` | String | REST 전용. 없으면 `/<name>` |
| `console_template` | String | 예: `"kick {player}"`. 예전 이름 `rcon_template`도 별칭으로 읽는다 (`D:src/supervisor/module_loader.rs:256-257, 730`). 인자에 `\r`·`\n`·NUL이 있으면 400 `invalid_argument`로 거부하고, stdin 전송(`ManagedProcess::send_command`)도 여러 줄 명령을 거부한다. 값 하나가 콘솔 명령 두 줄이 되지 않게 한다 |
| `risk` | String | `safe` / `caution` / `danger`, 없으면 caution |
| `inputs` | Vec<CommandInput> | |

`command_template` 별칭은 없어졌다.

`[commands] hook = true`면 lifecycle `instance_commands()`가 인스턴스마다 명령을 더 준다(설치된 플러그인 명령처럼 서버 폴더에 따라 달라지는 것). 결과는 `{success, commands:[명령 필드와 같은 모양 + group?], components?}`이다. 데몬(`crate::commands::InstanceCommands`)은 module.toml 명령 뒤에 붙여 같은 방식으로 해석하고, module.toml의 이름·별명과 겹치는 것과 이름에 공백이 있는 것은 뺀다. 명령 실행은 module.toml에서 못 찾은 이름일 때만 이 목록을 보며 60초 동안 기억한 값을 쓰고, `GET /api/instance/:id/commands`는 매번 새로 읽는다(`D:src/ipc/handlers/command.rs` `instance_commands`). `fields` 없이 `hook`만 있어도 된다.

`[[commands.fields.inputs]]` (`CommandInputToml` `:738-757`)의 허용 키: `name`✔, `label`, `type`, `required`, `placeholder`, `default`, `min`, `max`, `options`. 데몬이 입력을 묶고 검사할 때 이 값을 쓴다 (`commands/mod.rs:133-`).

#### `[[syntax_highlight.rules]]` (`HighlightRuleToml` `:768-784`)
`name`✔, `pattern`✔(regex, `(?P<hl>…)` 선택), `color`✔(시맨틱 토큰), `background`, `bold`=false, `italic`=false, `underline`=false, `priority`=0. `/api/modules`로 GUI에 그대로 전달되고, 데몬은 Python 전용 정규식 문법에 경고만 남긴다 (`:786-804`).
- 데몬 토큰화(`src/supervisor/highlight.rs`): `GET /api/instance/:id/console?spans=1`이면 줄마다 `spans: [[start, end, token]]`를 붙인다. 오프셋은 `content`의 UTF-16 코드 단위, 오름차순·겹침 없음, 칠할 곳만 들어 있다. 디스크 폴백 줄에도 붙고, 4096바이트를 넘는 줄은 앞부분만 본다.
  - 토큰은 16개로 고정: `error, warn, info, debug, success, string, number, keyword, timestamp, player, command, selector, ip, uuid, path, dim` (`TOKENS`). `warning`→`warn`, `trace`→`debug`, `comment`→`dim`으로 바꾸고, 색 값(`#hex`)이나 모르는 이름의 규칙은 경고를 남기고 뺀다. `bold`·`italic`·`background`·`underline`은 spans에 싣지 않는다(굵기·색은 클라이언트 테마가 토큰별로 정한다).
  - 엔진은 Rust `regex`(선형 시간). lookaround(`(?=`, `(?!`, `(?<=`, `(?<!`)는 컴파일되지 않아 그 규칙은 빠진다. 모듈 적합성 검사가 이를 막는다(saba-chan-modules `conformance`).
  - 겹치면 priority가 큰 규칙, 같으면 먼저 선언된 규칙이 이긴다. 모듈 규칙 뒤에 공통 규칙(스택 프레임·`... N more`→dim, `Caused by:`→error, IPv4[:포트]→ip, UUID→uuid, 따옴표 문자열→string, 퍼센트→number)이 남은 칸만 칠한다.
  - 컴파일 결과는 모듈 이름별로 캐시하고, 규칙 지문이 바뀌면(모듈 업데이트) 다시 만든다.

#### `[install]` (`InstallSectionToml` `:618-640` → `ModuleInstallConfig` `:643-661`)
| 키 | 타입 | 기본값 |
|---|---|---|
| `method` | String | `"manual"` (`steamcmd` / `download` / `manual`) |
| `app_id` | u32 | — |
| `anonymous` | bool | `true` |
| `install_subdir` | String | — |
| `platform` | String | — |
| `download_url` | String | — |
| `beta` | String | — |
| `requires_extensions` | Vec<String> | `[]`. 인스턴스 생성·non-managed 시작·`/api/module/:name/install` 때 필수 익스텐션 검증에 사용 (`instance.rs:141-160`, `mod.rs:392-449`, `managed.rs:376-`) |

zomboid의 `beta_branch` 키는 지워졌다. 베타 설치는 lifecycle이 `use_beta` 설정을 보고 정한다 (`M:zomboid/lifecycle.py:1250, 1273`).

#### `[extensions.<익스텐션 id>]`
익스텐션이 읽는 모듈별 설정. 데몬은 내용을 보지 않고 `ModuleMetadata.extensions`에 그대로 넘긴다 (`:454-458, 913-919`). 예전 `[docker]` 섹션(`ContainerSectionToml`)은 Docker 익스텐션과 함께 지워졌다.

#### `[credential_map]` (`HashMap<String,String>`)
데몬 키(`rest_password` / `rest_username`) → 게임 설정 키. 사용처는 managed start 후 동기화(`mod.rs:1362-1427`), import_settings 결과 반영(`D:src/instance/mod.rs:154-186`), 설정 PATCH 때 역동기화(`instance.rs:886-913`)다. **non-managed `start_server`에서는 동기화하지 않는다**.

#### `[aliases]`
`module_aliases: Vec<String>`, `[aliases.commands.<cmd>] = { aliases = [...], description = "..." }`.

#### `[errors]`
오류 코드 → 사용자 문구. 메타데이터 `errors`에 실리고(`:91-94`), `POST /api/instance/:id/command`가 실패 응답에 `error_message`로 붙인다 (`D:src/ipc/handlers/command.rs:11-15`).


#### `[[presets]]` (`ModulePreset`)
서버를 만들 때 고르는 추천 설정 묶음(예: "친구용 소규모", "하드코어"). `id`(소문자·숫자·`-`·`_`, 모듈 안에서 겹치지 않음), `label?`, `description?`, `[presets.settings]`(`[[settings.fields]]` 이름 → 값). 값은 그 설정의 규칙(형식·범위·select 선택지)을 따라야 하고, `password`·`file`·`path` 형식 설정은 넣지 못한다. 어기면 스키마 v1 모듈은 로드에 실패한다. 표시 문구는 모듈 locales의 `presets.<id>.label`·`description`이 덮는다. `GET /api/modules`의 각 모듈 `presets`, heartbeat `modules[].presets`(`{id, label}`만)에 실리고, `POST /api/instances`의 `preset`으로 고른다(01 §`POST /api/instances`). CLI는 `server add --preset <id>`(주지 않으면 새 서버일 때 묻는다), `module info`가 id를 보여 준다.

#### `[[queries]]` (`QueryToml` → `ModuleQuery`)
읽기 전용 조회(예: 접속자 목록). 액션 `server.query:<name>`(`kind=query`, risk `safe`, 모든 표면)이 되어 GUI·CLI·디스코드·챗봇 도구로 쓰인다(01 §액션 카탈로그). 키: `name`(소문자·숫자·`-`·`_`, 모듈 안에서 겹치지 않음), `label?`, `description?`, `function`(lifecycle.py `FUNCTIONS`의 함수 이름, Python 식별자), `inputs`(`[[commands.fields]]`의 입력과 같은 모양: `name`·`type`·`required`·`label`·`placeholder`·`default`·`min`·`max`·`options`). 로드 때 검사: 이름 형식·중복, `function`이 식별자인지, 데몬이 정해진 뜻으로 부르며 상태를 바꾸는 함수(`start`·`stop`·`command`·`configure`·`get_launch_command`·`import_settings`·`install_server`·`reset_properties`·`reset_server`·`accept_agreements`)가 아닌지, 입력 이름 형식·중복, 입력 `type`이 `text`(기본)·`number`·`select`인지, `select`에 `options`가 있는지, `min ≤ max`. 어기면 스키마 v1 모듈은 로드에 실패하고, `schema_version`이 없는 모듈은 경고만 남기되 `[[queries]]`를 모두 끈다. 표시 문구는 모듈 locales의 `queries.<name>.label`·`description`이 덮는다(없으면 `label`·`description`, 그것도 없으면 `name`). `GET /api/modules`의 각 모듈 `queries`에 실린다.

```toml
[[queries]]
name = "players"
label = "접속자 목록"
description = "지금 접속한 플레이어"
function = "list_players"
inputs = [{ name = "limit", type = "number", min = 1, max = 100 }]
```

#### `[[agreements]]` (`ModuleAgreement`)
서버를 켜기 전에 사용자가 원문을 읽고 동의해야 하는 약관(예: 게임 EULA). `id`(소문자·숫자·`-`·`_`, 모듈 안에서 겹치지 않음), `url`(https 원문). 이름은 모듈 locales `agreements.<id>`(없으면 id). 데몬은 게임 이름을 보지 않는다.

```toml
[[agreements]]
id = "eula"
url = "https://aka.ms/MinecraftEULA"
```

- 동의는 인스턴스 레코드 `accepted_agreements`(`{id: 동의 시각 RFC3339}`)에 남는다. 만들 때 `POST /api/instances`의 `accept_agreements:[id]`, 만든 뒤 `POST /api/instance/:id/agreements {accept:[id]}`로 받는다. 원격(디스코드·챗봇)에서 온 동의는 403 `agreement_local_only`(사람이 직접 원문을 보고 동의해야 하는 법적 동의).
- 선언된 약관을 다 동의하지 않으면 켜기 전 점검이 409 `agreement_required`(`agreements:[{id, label, url, accepted, accepted_at}]`)로 막는다(01 문서 시작 라우트). 파일 내용(예: eula.txt)으로 동의를 추정하지 않는다.
- 설치·재설치·업데이트 때 모듈은 `accepted_agreements:[id]`를 받고, 그 안에 자기 id가 있을 때만 자기 파일에 동의를 쓴다. 동의를 받으면 데몬이 lifecycle `accept_agreements`를 부른다(§2.3).
---

## 2. lifecycle.py 프로토콜

### 2.1 실행 방식 (`D:src/plugin/mod.rs:168-318`)

- **명령줄**: `<python_exe> <module_path>/lifecycle.py <function_name>`
  - Python 실행파일은 `python_env::get_python_path()`가 정한다(데몬이 관리하는 venv).
  - cwd는 지정하지 않으므로 **데몬의 cwd를 상속**한다.
- **stdin**: config 전체를 JSON 한 덩어리로 쓰고 바로 닫는다(`shutdown`).
- **stdout**: 프로세스 종료 후 전체를 읽어 JSON으로 파싱한다. 전체가 JSON이 아니면 **마지막 비어 있지 않은 줄**을 JSON으로 본다 (`parse_plugin_output` `:323-332`). 앞에 찍힌 디버그 print는 괜찮지만, 마지막 줄이 JSON이 아니면 `Invalid JSON from plugin` 에러가 난다.
- **stderr**: 줄 단위로 스트리밍된다.
  - `PROGRESS:{json}` 형식의 줄은 `ExtensionProgress {percent, message, step, total, label}`로 파싱해 콜백에 넘긴다 (`:40-50, 245-262`). 설치 진행률 표시에 쓰인다.
  - 나머지 줄은 `debug` 로그로만 남는다.
- **종료 코드**: 0이 아니면, stdout에 `{"success":false,...}`가 있더라도 `Err("Plugin execution failed (exit ..)")`가 된다 (`:287-290`). 그래서 실제 모듈은 모르는 함수·예외도 JSON으로 답하고 종료 코드 0을 지킨다 (공용 `run_lifecycle`, `D:modules/daemon_rcon.py:405-`).
- **타임아웃**: 기본 `DEFAULT_PLUGIN_TIMEOUT_SECS = 120` (`:119`). 초과하면 `child.kill()`.
  - `install_server`를 프로비저닝 경로(`instance.rs:405-408`)와 업데이트 경로(`instance.rs:1365-1371`)에서 부를 때만 1800초를 쓴다.
  - **`POST /api/module/:name/install` → `Supervisor::install_server`는 기본 120초**다 (`mod.rs:1822`). Minecraft의 Java 다운로드와 jar 다운로드가 120초를 넘으면 실패한다.
- **확인되지 않은 구성요소 (Windows)**: 신뢰 표시가 `verified`가 아닌 모듈·익스텐션은 Python을 띄우기 전에 그 폴더를 검사한다(`D:src/sandbox/`).
  - 스크립트(`.py`·`.pth`·`.ps1`·`.bat` 등)는 AMSI(등록된 백신)로, 네이티브 코드와 묶음(`.pyd`·`.dll`·`.so`·`.exe`·`.zip`·`.egg`·`.whl`)은 Defender `MpCmdRun`으로 본다. `__pycache__`는 건너뛴다.
  - 찾으면 실행하지 않고 `{success:false, action_required:"malware_detected", message, file}`를 돌려준다.
  - `__pycache__` 밖에 `.pyc`·`.pyo`가 있으면 원본 없이 실행될 수 있어 `action_required:"unscannable_file"`로 막는다.
  - 결과는 sha256으로 `<data_dir>/scan-verdicts.json`에 남겨, 바뀌지 않은 파일은 다시 보지 않는다. 깨끗하다는 결과는 7일이 지나면 다시 검사한다(찾았다는 결과는 그대로 둔다). 파일 sha256은 데몬 실행 동안 경로·크기·수정 시각으로 기억해, 셋이 같으면 다시 읽지 않는다(설치 기록과 비교하는 신뢰 표시도 같다). 옛 형식 결과 파일은 찾았다는 결과만 그대로 옮긴다. 빈 파일은 검사할 내용이 없어 넘긴다.
  - 검사기가 없으면(AMSI 공급자, `MpCmdRun`) 그 검사를 건너뛰고 데몬 실행마다 한 번 경고를 남긴다.
  - 파일을 읽지 못하거나 한 파일의 검사가 실패하면(검사 스레드가 죽은 경우 포함) 실행하지 않고 `action_required:"scan_failed"`(`file`, `detail`)를 돌려준다. 나머지 파일 검사는 계속한다.
  - 검사를 통과하면 Python을 그 구성요소만 쓰는 AppContainer `saba-chan.untrusted.<키 sha256 앞 16자>`(능력 `internetClient`만) 안에서 Job Object에 넣어 띄운다(`sandbox/appcontainer.rs`).
    - 안에서는 포트를 열 수 없고 LAN·루프백(데몬 API, 로컬 게임 서버 REST)에 닿지 않는다.
    - 폴더 권한: 읽기는 venv와 그 기반 Python, 그 구성요소 폴더, `modules_dir`, `extensions_dir`, `python-guard`. 읽기/쓰기는 `<data_dir>/pycache-sandbox/<꼬리표>`, `<data_dir>/sandbox-temp/<꼬리표>`(꼬리표는 프로필 이름과 같은 16자), 그리고 모듈이면 config의 `instance_dir`·`working_dir`·`install_dir`(익스텐션은 이것들을 읽기만 하고 `<data_dir>/extension-data/<이름>`에 쓴다, 03 문서). `extensions_dir`의 부모와 인스턴스 폴더 키 각각의 바로 위 폴더에는 목록만 준다(`os.makedirs(..., exist_ok=True)`가 위 폴더부터 확인한다). 모듈이 쓸 인스턴스 폴더가 아직 없으면 데몬이 먼저 만든다. 데이터 폴더 자체와 그 위, 드라이브 루트, 사용자 폴더 루트에는 주지 않는다. 목록 권한도 드라이브 루트와 사용자 폴더와 그 위에는 주지 않는다. 표시를 지원하지 않는 볼륨이라 붙이지 못하면 경고만 남기고 계속한다(그 폴더 쓰기가 안 될 수 있다). 권한은 그 프로필 SID에 대한 폴더 ACL로 남는다(한 번 주면 다시 쓰지 않는다). 폴더 ACL 고치기는 데몬 안에서 한 번에 하나씩 한다(같은 폴더에 동시에 쓰면 한쪽 ACE가 사라진다). AppContainer 프로세스는 낮은 무결성 수준으로 돌고, 표시가 없는 보통 폴더는 중간 수준(낮은 수준의 쓰기 금지)이라 DACL만으로는 쓰지 못한다. 그래서 읽기/쓰기를 준 폴더에는 하위까지 상속되는 낮은 수준 표시(`ML;OICI;NW;;;LW`)도 붙인다(이미 있으면 다시 쓰지 않는다). 그 폴더는 같은 사용자의 다른 낮은 수준 프로세스도 DACL이 허용하는 만큼 쓸 수 있다. 그 폴더의 서버 실행 파일은 데몬이 중간 수준으로 띄우므로, 낮은 수준 프로세스가 바꿔 둔 파일이 중간 수준으로 실행될 수 있다. 쓰기를 줄 경로나 그 위 폴더가 심볼릭 링크·정션이면 쓰기를 주지 않고(읽기는 준다), ACL과 표시는 경로를 다시 찾지 않고 연 핸들에 쓴다(핸들의 최종 경로가 확인한 경로와 다르거나 재분석 지점이면 실행을 멈춘다). 프로필이 구성요소마다 달라서 한 구성요소가 받은 권한을 다른 구성요소가 쓰지 못한다. 예전의 공용 프로필 `saba-chan.untrusted`는 더 쓰지 않는다.
    - 환경: `SABA_TOKEN_PATH`를 뺀다. `PYTHONPYCACHEPREFIX`는 `pycache-sandbox/<꼬리표>`, `TEMP`·`TMP`는 `sandbox-temp/<꼬리표>`.
    - 시간 초과면 Job째 끝낸다.
  - 격리를 준비하지 못하면(오래된 Windows, ACL 오류 등) 격리 없이 돌리지 않고 `{success:false, action_required:"sandbox_unavailable", message, detail, component}`를 돌려준다. 사용자가 `POST /api/sandbox/unisolated/:component`로 동의하면 그 구성요소는 격리 없이 돈다. 동의는 `config.json` `sandbox.unisolated`에 파일 지문(상대 경로와 sha256)으로 남아, 파일이 하나라도 바뀌면 다시 묻는다.
  - Linux·macOS에서는 하지 않는다.
- **데몬 락**: lifecycle 호출은 데몬 락 밖에서 실행한다. 보통 `PluginCall`로 락 안에서 값을 챙기고 락을 놓은 뒤 부른다 (`:130-144`, C12).
- **환경변수** (`:18-37, 180-232`):
  - `PYTHONIOENCODING=utf-8`, `PYTHONUNBUFFERED=1`
  - `PYTHONPATH` = `<python-guard>` + `<extensions_dir의 부모>` + `<modules_dir>` + 기존 `PYTHONPATH`.
    - 그래서 `import extensions.steamcmd`, `import daemon_rcon`, `import i18n`이 가능하다.
    - `python-guard/sitecustomize.py`는 활성 목록에 없는 `extensions.<이름>` import를 막는다 (`:85-97`).
    - 데몬은 첫 실행 때 자기 안에 든 `i18n.py`·`daemon_rcon.py`를 modules 폴더에 맞춰 쓴다 (`ensure_shared_modules` `:100-117`).
  - `SABA_ENABLED_EXTENSIONS` = 활성 익스텐션 ID를 콤마로 연결한 문자열. `<data_dir>/config.json`의 `extensions.enabled`에서 읽는다 (`:198-205`). 목록이 없으면 빈 값이다.
  - `SABA_IPC_PORT`, `SABA_IPC_URL`(데몬 API 주소), `SABA_TOKEN_PATH`(`X-Saba-Token` 값이 든 파일), `SABA_LANG`(`config.json`의 `gui.language`, 실행마다 다시 읽음)을 넣는다 (`daemon_env` `:18-37`).
    - ~~문서는 `DAEMON_API_URL`을 주입한다고 한다~~ → 해결: 다시 쓴 개발 가이드는 `SABA_IPC_URL`, `SABA_TOKEN_PATH`, `SABA_LANG`이라고 적는다(`M:docs/module-development-guide.md:31`). `daemon_rcon.py`는 `DAEMON_API_URL`이 있으면 그것을, 없으면 `SABA_IPC_URL`을, 둘 다 없으면 `http://127.0.0.1:57474`를 쓴다 (`D:modules/daemon_rcon.py:53-59`).

### 2.2 모듈 측 디스패치 관례

모든 모듈이 같은 구조를 따른다 (`M:minecraft/lifecycle.py:1834-1876`, `palworld:1453-1507`, `zomboid:1432-1492`, `__example:478-511`).

1. `sys.argv[1]`을 읽는다.
2. stdin JSON을 읽는다. 비어 있으면 `{}`로 둔다.
3. `FUNCTIONS[name](config)`를 호출한다.
4. `print(json.dumps(result))`로 결과를 출력한다.

모르는 함수 이름이 오면 `{"success":false,"message":"Unknown function: X"}`를 돌려주고 종료 코드는 네 모듈 모두 0이다 (zomboid는 `return`, `M:zomboid/lifecycle.py:1466-1473`).

**에러 관례 (코드에서 관찰한 것)**:
- 공통 응답: `success: bool`, `message: str`.
- `get_launch_command` 성공: `program`, `args[]`, `working_dir`, `env_vars{}`. 그 밖의 선택 키:
  - `rest_credentials{rest_username, rest_password}`
  - credential_map의 게임 키(예: palworld의 `"AdminPassword"`)를 최상위나 `module_settings{}` 아래에 둔다.
  - `extra_args_placed: true`: 받은 `extra_args`를 모듈이 알맞은 자리(예: JVM 인자는 `-jar` 앞)에 넣었다. 없으면 데몬이 `args` 끝에 붙인다(`supervisor::launch_args`).
- `get_launch_command` 사용자 조치 필요: `action_required: "<code>"`를 넣으면 데몬이 결과를 그대로 GUI에 넘긴다 (`mod.rs:602-606, 1334-1337`). minecraft는 `server_jar_not_found` + `options:["update_path","install_new"]`를 쓴다.
- 익스텐션 누락: `error_code: "extension_required"`, `missing_extensions[]`.
- `status`: `status` 문자열과 `pid`를 데몬이 읽는다. 나머지 키는 버린다 (`mod.rs:986-1001`).
- `install_server`: `success`, `executable_path`, `install_path`를 프로비저닝 경로가 읽는다 (`instance.rs:415-433`).
- `list_versions`: 데몬은 `latest.release` 또는 `latest_release`만 읽는다 (`instance.rs:343-358`).
- `get_installed_version`: `version`.
- `get_credentials`: `success`, `rest_username`, `rest_password`. 데몬은 REST 자격증명만 쓴다 (`command.rs:450-452, 531-558`). 지금은 palworld만 구현한다. minecraft·zomboid는 RCON 값만 돌려주던 이 함수를 지웠다.
- `import_settings`: `success`, `settings{}`. `port`·`rest_port`와 credential_map의 게임 키(→ `rest_password`)는 인스턴스 필드로도 맞춘다 (`D:src/instance/mod.rs:154-186`). 마이그레이션에서는 `executable_path`(작업 폴더 기준 상대 경로, 모듈이 찾은 서버 실행 파일)와 `detected`(판정 결과, 데몬은 내용을 보지 않고 생성 응답에 넘긴다)도 읽는다.
- `instance_commands`: `success`, `commands[]`(명령 필드 + `group`), `components` (`[commands] hook = true`일 때).
- `install_server`: config에 인스턴스 설정 `settings{}`가 온다(프로비저닝·업데이트).
- `backup_paths`: `success`, `paths[]` (`[backup] hook = true`일 때).

### 2.3 데몬이 호출하는 함수 전체 목록과 전달 config

| 함수 | 호출 지점 | 데몬이 넣는 config 키 |
|---|---|---|
| `get_launch_command` | managed 시작 `mod.rs:1310-1330`, non-managed 시작 `mod.rs:580-599`, 실행 인자 미리보기(`Supervisor::launch_preview`) | (요청 payload.config) + `server_executable`, `server_jar`(없을 때), `working_dir`, `port`, **`module_settings`를 최상위로 펼침**, `rest_host`, `rest_port`, `rest_username`, `rest_password`, `protocol_mode`, `server_version`, `instance_id`, `extra_args`(사용자가 더한 인자, 문자열 배열), `managed: true/false` (`insert_instance_fields`). 미리보기에서는 `preview: true`도 온다: 이때 모듈은 설정 파일을 쓰는 등 부작용을 내지 않는다 |
| `stop` | non-managed 정지 `mod.rs:874-883`, stdin을 쓸 수 없는 managed 서버(REST 전용, stdin 없는 재부착)의 정지 `mod.rs:778-789` | `server_executable`, `server_jar`, `working_dir`, `module_settings`(펼침), `rest_*`, `protocol_mode`, `server_version`, `instance_id`(`insert_instance_fields`, `mod.rs:286-297`), `pid`, `force`, `managed`. 추적 중인 PID가 없으면 non-managed에서는 부르지 않는다 (`mod.rs:863-872`). 콘솔이 있는 managed 서버는 이 함수를 부르지 않고 데몬이 `stop_command`를 stdin으로 보낸 뒤 기다린다 (`mod.rs:756-776`) |
| `status` | non-managed 상태 조회 `mod.rs:964-982` | `server_executable`, `server_jar`, `working_dir`, `pid`, `module_settings`(펼침) |
| `command` | `POST /api/instance/:id/command`에서 명령의 `method`가 `module`일 때 → `mod.rs:1024-1083` | `command`, `args`, `protocol_mode`, `working_dir`, `rest_host`, `rest_port`, `pid`, `instance_id` |
| `get_credentials` | `/rest` 요청 시. 결과를 캐시하므로 캐시가 있으면 Python을 띄우지 않는다 (`command.rs:531-558`) | `working_dir` |
| `validate` | `POST /api/instance/:id/validate`, 그리고 `POST /api/instance/:id/start`의 시작 전 점검(`server.rs:618-655`) → `mod.rs:1579-1610` | `server_executable`·`server_jar`(=executable_path), `working_dir`, `port`, `extension_data`, 인스턴스 값(위와 같음), `running` |
| `configure` | `PUT /properties`, 인스턴스 설정 PATCH 후 동기화 (`instance.rs:991-1048`) → `manage_properties` `mod.rs:1614-1665` | `working_dir`, `extension_data`, 인스턴스 값, `port`, `settings{}` |
| `read_properties` | `GET /properties` | `working_dir`, `extension_data`, 인스턴스 값, `port` |
| `reset_properties` | `POST /properties/reset` | 위와 같음 |
| `reset_server` | `POST /server/reset` | 위와 같음 |
| `import_settings` | 마이그레이션 인스턴스 생성 `instance.rs:527-531`, `GET /api/instance/:id/settings/live` (`managed.rs:150-176`) | 마이그레이션은 `working_dir`만, live는 위와 같음 |
| `instance_commands` | `GET /api/instance/:id/commands`, module.toml에 없는 이름의 명령 실행 (`[commands] hook = true`일 때, `D:src/ipc/handlers/command.rs`) | `working_dir`, 인스턴스 값(`insert_instance_fields`) |
| `accept_agreements` | `POST /api/instance/:id/agreements` (동의를 기록한 뒤) | `validate`와 같은 config + `accepted_agreements:[id]`(지금까지 동의한 전부). 모듈은 자기 파일에 동의를 쓴다(예: minecraft eula.txt). 함수가 없으면 기록만 남는다 |
| `diagnose_log` | `POST /diagnose` | `working_dir`, `log_lines[]`(managed일 때 최근 500줄 중 이번 실행분, `mod.rs:1704-1712`) |
| `list_versions` | `GET /api/module/:name/versions`, 프로비저닝(download 방식) | `include_snapshots`, `page`, `per_page` 또는 `{}` |
| `get_version_details` | `GET /api/module/:name/version/:v` | `version` |
| `install_server` | `POST /api/module/:name/install`(120초), 프로비저닝·업데이트(1800초) | `version`, `install_dir`, `accepted_agreements`(동의한 `[[agreements]]` id), `jar_name`, `initial_settings` 등 |
| `get_installed_version` | `GET /installed-version`, 데몬 초기화 시 backfill `mod.rs:1909-1958` | `install_dir`, `jar_name`(**기본값 `"server.jar"` 하드코딩**, `mod.rs:1890-1893`) |
| `[[queries]]`의 `function` | `POST /api/instance/:id/query/:name`(액션 `server.query:<name>`) → `Supervisor::query_instance` | `validate`와 같은 config(`Supervisor::instance_call`) + `args{입력 이름: 값}`(명령 입력과 같이 검사·기본값 채움, 숫자는 숫자로). 돌려줄 것: `{"success": true, "data": <JSON>}` 또는 실패 `{"success": false, "message": "..."}`. `data`가 없으면 `success`·`message`를 뺀 나머지를 data로 본다. 읽기만 해야 한다(감사에 남지 않고 챗봇·일반 사용자도 부른다) |
| `backup_paths` | 백업 때 `[backup] hook = true`인 모듈 (`D:src/ipc/handlers/backup.rs:91-96`) | `working_dir`, `extension_data`, 인스턴스 값 |

모듈의 `start` 함수는 모든 모듈에서 지워졌다(FUNCTIONS에 없음). non-managed 모드도 `get_launch_command`로 데몬이 직접 spawn한다 (`mod.rs:580-652`).

**예전 개발 가이드의 "호출 순서"와 다른 점** (가이드는 a8b6c29에서 현재 데몬 기준으로 다시 쓰였다. 아래는 `__example__lifecycle.py:36-46` 주석과의 차이다):
- "시작 시 `validate → configure → get_launch_command`"라고 되어 있다. 실제로는 `POST /api/instance/:id/start`가 `validate`를 시작 전 점검으로 부르고(critical 문제만 막고, 점검이 실패하면 막지 않는다) 이어서 `get_launch_command`를 부른다. `configure`는 시작 때 부르지 않는다. `/managed/start`와 `/server/:name/start`를 직접 부르면 점검도 없다.
- "status 주기적 호출"이라고 되어 있지만, 실제로는 non-managed 인스턴스의 status API 요청 때만 호출된다. 모니터 루프는 PID 스냅샷만 본다.
- ~~"rcon_template이 있으면 데몬이 직접 RCON을 보낸다"~~ → 해결: 개발 가이드가 다시 쓰여 `method = "console"` + `console_template`을 콘솔(stdin)로 보낸다고 적는다(`M:docs/module-development-guide.md:182-200`, `D:src/commands/mod.rs:111-112`). `__example__lifecycle.py` 주석도 `method="console"`로 고쳐졌다(`:44`, e1d7743).
- ~~"config에 `instance_id`, `settings{}` 항상 포함"~~ → 부분 해결: `instance_id`는 인스턴스 필드를 넣는 호출(`insert_instance_fields`, `D:src/supervisor/mod.rs:286-297`)에 들어간다. `settings{}`는 여전히 `configure`에만 들어가고, 그 외 함수에서는 `module_settings`가 **최상위로 펼쳐져** 들어간다(템플릿 주석 `:58-63`은 아직 `settings`가 항상 온다고 적는다).

### 2.4 함수 구현 매트릭스 (FUNCTIONS 딕셔너리 기준)

✔ = 디스패치 테이블에 있음, – = 없음.

| 함수 (데몬 호출 여부) | minecraft | palworld | zomboid | __example |
|---|---|---|---|---|
| get_launch_command (호출) | ✔ | ✔ | ✔ | ✔ |
| stop (non-managed, stdin 없는 managed) | ✔ | ✔ | ✔ | ✔ |
| status (non-managed만) | ✔ | ✔ | ✔ | ✔ |
| command (호출) | ✔ | ✔ | ✔ | ✔ |
| get_credentials (호출) | – | ✔ | – | – |
| validate (호출) | ✔ | ✔ | ✔ | ✔ |
| configure (호출) | ✔ | ✔ | ✔ | ✔ |
| read_properties (호출) | ✔ | ✔ | ✔ | – |
| reset_properties (호출) | ✔ | ✔ | – | – |
| reset_server (호출) | ✔ | ✔ | ✔ | – |
| accept_agreements (호출) | ✔ | – | – | – |
| diagnose_log (호출) | ✔ | ✔ | ✔ | ✔(stub) |
| list_versions (호출) | ✔ | ✔ | – | – |
| get_version_details (호출) | ✔ | ✔ | – | – |
| install_server (호출) | ✔ | ✔ | ✔ | ✔ |
| get_installed_version (호출) | ✔ | – | – | – |
| import_settings (호출) | ✔ | ✔ | ✔ | ✔ |
| backup_paths (hook일 때 호출) | – | – | ✔ | – |

출처: `M:minecraft/lifecycle.py:1834-1851`, `M:palworld/lifecycle.py:1473-1490`, `M:zomboid/lifecycle.py:1432-1445`, `M:__example/__example__lifecycle.py:478-488`.

**영향**:
- 데몬 초기화 때의 backfill은 palworld와 zomboid에서 매번 실패한다(debug 로그).
- zomboid의 "설정 초기화"와 EULA, 버전 목록 API는 Unknown function을 돌려준다.

### 2.5 공용 Python 유틸 (`D:modules/`)

- `i18n.py`: `I18n(module_dir)`가 `locales/<lang>.json`을 읽는다. 언어는 `SABA_LANG`으로 정하고 기본값은 `en`이다. 점 표기 키와 `{{var}}` 보간을 지원한다. `__example/i18n.py`는 같은 파일의 복사본이다.
- `daemon_rcon.py`: 데몬 HTTP `/api/instance/:id/rcon`(실제로는 stdin)을 감싸는 래퍼다. 이름만 RCON이고 RCON 소켓은 열지 않는다. 임의 경로용 `daemon_post`, stdin 전송, PID 확인·강제 종료, 설정값 조회·형 변환, `run_lifecycle` 진입점 같은 공용 헬퍼도 들어 있다 (`D:modules/daemon_rcon.py:25-34`).
  - 주소는 `DAEMON_API_URL` → `SABA_IPC_URL` → `http://127.0.0.1:57474` 순서다 (`:53-59`).
  - `SABA_TOKEN_PATH` 파일의 토큰을 `X-Saba-Token` 헤더로 보낸다 (`:62-74`). palworld(`execute_rest_via_daemon` `M:palworld/lifecycle.py:1414-`)는 `daemon_post`를, minecraft·zomboid·__example은 공용 `daemon_stdin_command`(`/stdin`에 `{"command"}`, `D:modules/daemon_rcon.py:228-230`)와 PID 헬퍼 `is_process_alive`·`wait_for_exit`·`force_kill`을 쓴다 (`M:zomboid/lifecycle.py:36`).
  - payload의 `rcon_port`와 `rcon_password`는 데몬 `/rcon` 핸들러가 무시한다.
  - 예전 호환 함수 `rcon_command(host,port,pw,cmd)`는 `instance_id`가 있으면 데몬 경로로 보내고, 없으면 `ConnectionError`를 낸다. 직접 소켓 RCON 폴백은 지워졌다.
- 배포: 데몬 실행 파일에 두 파일이 들어 있고, 첫 lifecycle 실행 때 modules 폴더에 맞춰 쓴다 (`D:src/plugin/mod.rs:100-117`). 코어 패키저도 둘을 싣고(`D:.github/scripts/package-release.py:55`), installer가 데이터 디렉터리의 `modules/`에 복사한다 (`installer/gui/src-tauri/src/lib.rs:403-405, 810-830`). 모듈 리포의 zip에는 들어가지 않는다(리포에는 `__example/i18n.py` 복사본만 있다).

---

## 3. Managed Process 모델

### 3.1 두 가지 실행 경로

시작 경로는 데몬이 고른다. `POST /api/instance/:id/start`가 `Supervisor::uses_managed_start`(인스턴스 설정 `managed_start`, 없으면 managed)로 경로를 정하고, 시작 전 점검(§2.3 `validate`)을 거쳐 아래 둘 중 하나를 부른다 (`D:src/ipc/handlers/server.rs:563-616`, `D:src/supervisor/mod.rs:1479-1489`). GUI는 이 경로만 부른다 (`D:saba-chan-gui/main.js:1455`).

| | managed (`POST /api/instance/:id/managed/start`) | non-managed (`POST /api/server/:name/start`) |
|---|---|---|
| 엔트리 | `Supervisor::start_managed_server` `mod.rs:1102-1475` | `Supervisor::start_server` `mod.rs:351-660` |
| 사전 검사 | 중복 실행 방지(managed_store와 tracker PID 생존 확인), 모듈 의존성, 포트 충돌, 시작 중 표시(`StartGuard`), extension `server.pre_start` hook | 포트 충돌, 모듈 의존성, `requires_extensions` 검증, 시작 중 표시, `server.pre_start` hook |
| spawn | saba-runner가 있으면 `ManagedProcess::spawn_with_runner`, 없으면 `ManagedProcess::spawn`(stdin, stdout, stderr 파이프) (`mod.rs:1430-1449`) | `TokioCommand` — stdio 모두 `null`, 콘솔 없음 |
| 추적 키 | `tracker.track(instance.id, pid)` | `tracker.track(instance.id, pid)` (`mod.rs:651-652`) |
| PID 영속화 | `<instance_dir>/managed.pid`, runner면 `<instance_dir>/run/` | 없음 |
| 자격증명 동기화 | `rest_credentials` 또는 `credential_map` | 없음 |

- `requires_extensions` 검증은 **non-managed 경로에만 있다** (`mod.rs:392-449`). managed 경로는 `[module.dependencies]`의 `ext-<id>` 검사만 한다.
- 두 경로 모두 인스턴스 ID로 추적하므로 모니터·포트 충돌 검사·정지·상태가 같은 키를 쓴다.
- **확인되지 않은 모듈의 실행 대상 (Windows)**: `get_launch_command`가 돌려준 `program`은 두 경로 모두 띄우기 전에 검사한다(`Supervisor::guard_launch`, `sandbox::check_launch`). 신뢰 표시가 `verified`인 모듈은 검사하지 않는다.
  - 허용: 실제 경로(링크·`..`를 푼 것)가 인스턴스 폴더나 사용자가 정한 서버 폴더(`working_dir`) 안에 있는 파일. 모듈이 돌려준 `working_dir`은 허용 위치로 쓰지 않는다.
  - 경로가 있는 상대 `program`은 데몬 폴더가 아니라 모듈이 돌려준 `working_dir` 기준으로 풀어 검사하고, 검사한 그 절대 경로로 띄운다.
  - 허용: 경로 없는 `java`·`javaw`, 그리고 설정 `java_path`가 가리키는 java. `java_path`가 모듈 `module.toml`의 기본값과 같으면 사용자가 정한 것이 아니므로 인정하지 않는다.
  - 모듈이 돌려준 `env_vars`의 `PATH`·`PATHEXT`(대소문자 무시)는 버린다. 경로 없는 `java`는 데몬의 PATH로 찾는다.
  - 거부: 셸·스크립트 호스트(`cmd`, `powershell`, `wscript`, `mshta`, `rundll32`, `regsvr32` 등)와 `.bat`·`.cmd`·`.ps1`·`.vbs` 같은 스크립트는 어디 있든 거부한다.
  - 거부하면 `{success:false, action_required:"launch_outside_instance", message, file}`를 돌려준다.
  - 통과한 절대 경로 실행 파일과, 실행 인자가 가리키는 있는 파일(인자 그대로, `@파일`, `-옵션=파일`·`-옵션:파일`, 상대 경로는 작업 폴더 기준. 예: `-jar server.jar`)은 처음 띄우기 전과 바뀐 뒤에 Defender `MpCmdRun -Scan -ScanType 3`으로 검사한다. 찾으면 `malware_detected`로 막는다. 결과는 sha256으로 남기고(위 §2.1과 같은 7일·크기·수정 시각 규칙), MpCmdRun이 없으면 건너뛴다. 파일을 읽지 못하면 `scan_failed`로 막는다.

**잠금**: `start_managed_handler`는 비밀번호 자동 생성 때만 write lock을 잡고, `start_managed_server`는 검사·준비·등록 때만 락을 짧게 잡는다. 익스텐션 hook, `get_launch_command`, 프로세스 띄우기는 락 밖에서 한다 (`D:src/ipc/handlers/managed.rs:19-58`, `mod.rs:1093-1101`, C12).

### 3.2 상태와 종료 판정

- `state_machine.rs`는 지워졌다.
- 데몬이 노출하는 "상태"는 다음과 같다.
  - managed: `running`/`stopped` 두 값이다. `watch::channel<bool>`의 `is_running()`으로 정한다 (`mod.rs:950-962`).
  - non-managed: 모듈 `status()`가 돌려준 문자열을 그대로 쓴다. Python이 실패하면 `"unknown"`이다.
  - extension hook `server.status`가 `handled: true`를 돌려주면 그 결과가 우선한다.
- managed 서버가 끝나면 `judge_exit`가 `user_stop` / `crash` / `in_game_stop` / `unknown_exit`로 가르고 기록한다 (`mod.rs:1961-2029`, `D:src/supervisor/crash.rs:1-13`). 이력은 `GET /api/instance/:id/events`로 본다.
- `ProcessTracker`(`D:src/supervisor/process.rs`)는 PID와 시작 시각만 기록한다(`track`, `track_since`, `replace_pid`, `untrack`). 예전의 `ProcessStatus`, `get_status`, `mark_crashed`, `terminate`는 지워졌다. 강제 종료는 `ManagedProcess::force_kill`(`force_kill_pid`로 서버와 runner를 함께 죽인다, `managed_process.rs:230-235`)이다.

### 3.3 콘솔 캡처와 로그 파싱 (`managed_process.rs`)

- **링 버퍼**: `LogBuffer`. 기본 10,000줄(`:33`). 단조 증가하는 `id`를 붙여 `get_since(id)`와 `get_recent(n)`으로 폴링한다. IPC는 폴링(`GET /console?since=`) 방식이다. 예전의 broadcast 채널과 `subscribe`, `wait_for_exit`는 지워졌다.
- **stdout/stderr 리더**: 각각 tokio task로 줄 단위로 읽는다 (`:309-338`). stderr 줄은 레벨이 Info로 파싱되더라도 **Warn으로 올린다**.
- **레벨 파싱**: `parse_log_level(line, log_pattern)`이 `(?P<level>)` 캡처를 매핑한다 (`:747-761`).
  - ERROR/FATAL → Error, WARN/WARNING → Warn, DEBUG/TRACE → Debug, 그 외 → Info.
  - 패턴이 없거나 매치되지 않으면 Info다.
  - 정규식이 틀리면 warn 로그를 남기고 패턴 없이 동작한다.
  - zomboid 패턴의 `LOG` 같은 값도 Info가 된다.
- **디스크 영속화**: 직접 실행이면 `<instance_dir>/logs/console.log`에 append한다. 포맷은 `[datetime] [source/level] content`이다 (`:774-789, 864-926`). 10MB가 되면 로테이션하고 2개(`.1`, `.2`)를 보관한다. writer가 죽으면 콘솔에 경고 줄을 넣는다. saba-runner로 띄우면 runner가 `<instance_dir>/logs/sessions/<시각>.log`에 쓰고, 기본 20개(`log_sessions_keep`)를 남긴다 (`managed_process/runner.rs:24-25, 105-110`).
- **데몬 재시작 시 재부착**: `initialize → reattach_orphaned_managed` (`mod.rs:149-221`).
  - saba-runner가 쥔 서버면 `attach_runner`가 콘솔 기록, stdin, 종료 코드를 그대로 다시 잡는다 (`managed_process/runner.rs:169-195`).
  - runner가 없고 `managed.pid`의 PID만 살아 있으면 `create_reattached_stub`을 만든다. 이 스텁은 디스크 로그를 로드하고, 5초마다 PID 생존을 폴링하며, **stdin을 쓸 수 없고 새 출력도 받지 못한다** (`managed_process.rs:605-668`).
- **disk fallback**: managed 프로세스가 없으면 `get_console_output`이 가장 최근 디스크 로그(세션 로그 또는 `console.log`)를 읽는다. 이때 `since_id`는 무시된다 (`mod.rs:1549-1567`).
- **log follower**: 익스텐션이 `server.pre_start`에서 `log_follower{program,args,working_dir,interactive,strip_prefix,env_vars}`를 돌려주면 `spawn_log_follower`(읽기 전용) 또는 `spawn`(양방향)으로 그 프로세스를 managed_store에 등록한다. 종료 판정 대상은 아니다 (`mod.rs:504-568, 1234-1298`).

### 3.4 콘솔 입력, 정지, 강제 종료

- **stdin**: `POST /api/instance/:id/stdin` `{command}`가 `/rcon`과 같은 `via_stdin`을 부르고, 보낸 뒤 찍힌 콘솔 줄을 `data.response`로 돌려준다 (`managed.rs:89-97`, `command.rs:303-`). 쓰기는 mpsc(256)를 거쳐 writer task가 개행을 붙여 쓴다 (`managed_process.rs:281, 340-347, 400-418`).
- **managed stop** (`mod.rs:756-830`):
  1. `force`이면 `force_kill`을 한다.
  2. 모듈이 `stdin`을 지원하고 stdin이 열려 있으면 `stop_command`를 stdin으로 보낸다.
  3. 그 밖(REST 전용 게임, stdin 없는 스텁)이면 모듈 `stop()`에 `pid`, `managed: true`, `force: false`를 넘긴다.
  4. 0.5초 간격으로 `is_running`을 확인한다. 기다리는 시간은 graceful이면 `stop_timeout`(기본 30초), force면 5초다.
  5. 시간이 지나면 강제 종료하고 2초 더 기다린다.
  6. `managed_store`에서 제거하고 종료를 판정·기록하고, `managed.pid`를 삭제하고, 30초짜리 stop cooldown(`STOP_COOLDOWN_SECS`)을 등록한다.
  - palworld는 stdin이 없어 3번 경로로 간다. 모듈 `stop()`이 REST로 저장·종료를 요청한다 (`M:palworld/lifecycle.py:399-426`).
- **non-managed stop**: 추적 중인 PID가 있을 때만 Python `stop()`에 `pid`와 `force`를 넘긴다. 모듈이 REST나 kill을 직접 처리한다. `success=false`이면 Err가 된다 (`mod.rs:860-903`). minecraft·zomboid는 일반 정지에서 바로 죽이지 않고 콘솔 정지(콘솔이 없을 때 불리므로 보통 실패, §6.3 #8) → POSIX면 SIGTERM → 30초 대기 → 강제 종료 순서를 밟는다. Windows에는 보낼 정상 종료 신호가 없어 바로 강제 종료한다.
- **extension 위임**: `server.stop` hook은 정지 전에 불린다. `handled`를 돌려주면 그대로 반환하고, 정지를 맡을 익스텐션이 실패하면 실패를 돌려준다 (`mod.rs:713-738`).

### 3.5 명령 경로 (stdin/REST)

- 데몬의 프로토콜 계층에는 REST(`rest.rs`, `client.rs`)만 남았다. RCON 코드는 지워졌다.
- **명령 해석**: `POST /api/instance/:id/command`가 모든 인터페이스의 단일 경로다. `crate::commands`가 module.toml 정의로 이름·별명·입력을 해석하고 `method`에 따라 콘솔·REST·모듈 `command()`로 보낸다. 정의에 없는 명령은 한 줄 그대로 콘솔로 간다 (`D:src/commands/mod.rs:1-17`, `command.rs:11-`).
- **콘솔 경로**: `/api/instance/:id/rcon`과 `/stdin`은 서버 콘솔(stdin)에 쓰고, 콘솔 출력을 잠깐 모아 답으로 돌려준다. 콘솔이 없으면 409 `no_command_path` (위 RCON 정책).
- **REST 경로**: `/api/instance/:id/rest`. `get_credentials`(캐시)로 게임 설정 파일의 계정을 읽는다. Basic Auth의 username 기본값은 `"admin"`이다.
- **RCON 끄기**: minecraft는 모든 모드와 설치 때 `enable-rcon=false`를 강제한다 (`M:minecraft/lifecycle.py:526, 990-995, 1712`). zomboid는 `RCONPassword`를 비운다 (`M:zomboid/lifecycle.py:817-827`). palworld는 실행 전과 `configure()` 저장마다 `RCONEnabled=False`를 쓴다 (`M:palworld/lifecycle.py:256-258, 933`).
- `SupervisorError`(`error.rs`)는 지워졌다. 핸들러는 `anyhow` + 수동 JSON을 쓴다.

### 3.6 크래시 감지와 자동 재시작

- **크래시 판정** (`D:src/supervisor/crash.rs`): 데몬이 정지를 보냈으면 `user_stop`, 메모리 상한을 넘어 러너가 끝냈으면 `crash`(근거 `memory limit N MiB reached`, 아래 메모리 상한), 예외 종료 코드·크래시 시그널·이번 실행 중 생긴 `crash_artifacts` 파일이 있으면 `crash`, 콘솔 끝 50줄에 `graceful_exit_patterns`가 있거나 종료 코드 0이면 `in_game_stop`, 그 밖은 `unknown_exit`다. 흔적이 없으면 크래시로 보지 않는다.
- **자동 재시작**: `auto_restart`(기본 켬)가 켜져 있으면 `crash`를 재시작한다. `restart_on_unknown_exit`(기본 끔)까지 켜면 `unknown_exit`도 재시작한다 (`crash.rs:109-111`). 첫 대기 5초에서 두 배씩 늘고, 10분 안에 3번을 넘으면 포기한다 (`crash.rs:21-30`, `mod.rs:1993-2009`). 데몬 루프가 예약을 가져가 대기 뒤 `POST /api/instance/:id/start`와 같은 경로로 시작한다 (`D:src/main.rs:391-396`, `server.rs:735-751`). 판정과 재시작 여부는 콘솔에 System 줄로 남는다.
- `monitor_processes`(`mod.rs:2074-2254`)가 하는 일은 다음과 같다.
  1. `managed_store.cleanup_dead()`로 죽은 항목을 꺼내 종료를 판정하고, `managed.pid`를 삭제한 뒤 cooldown을 등록한다.
  2. 추적 중인 PID가 스냅샷에 없으면 untrack한다. saba-runner가 아직 서버를 돌리면(런처가 끝나고 실제 서버가 남은 경우) runner가 보는 PID로 옮긴다.
  3. `auto_detect`가 켜진 인스턴스는 `process_name`(+`cmd_patterns`)으로 외부 프로세스를 찾고, 같은 게임 인스턴스가 여럿이면 폴더로 주인을 가려 추적한다. 포트가 충돌하면 그 프로세스를 **강제 kill**하고 `PortConflictStopEvent`를 쌓는다.
- **메모리 상한** (`memory_limit_mb`, 인스턴스 레코드, MiB): 비우면 제한 없음. 데몬이 saba-runner에 `--memory-limit <MiB>`로 넘기고, 러너는 Windows Job에 `JOB_OBJECT_LIMIT_JOB_MEMORY`를 건다(서버와 그 자식의 커밋 메모리 합, `runner/src/job.rs`). 넘으면 할당이 실패하고 Job이 완료 포트로 알린다. 러너는 그때 Job을 끝내고(종료 코드 8) `exit.json`에 `memory_limit_mb`를 남긴다. 데몬은 이것을 `crash`로 판정하므로 `auto_restart`면 다시 켠다. Java 서버면 상한을 `-Xmx`보다 1~2GB 크게 잡는다(힙 밖 메모리). Windows만 적용하고, 다른 OS나 러너 없이 띄운 서버는 로그에 한 줄 남기고 제한 없이 돈다. 원격(디스코드·챗봇) 설정 변경으로는 바꿀 수 없다.
- `Supervisor::monitor()` stub은 지워졌다.
- `process_name`은 Windows 값(`java.exe`)이고 Linux용 override가 없다. 이름 비교는 부분 문자열(`"java".contains("java.exe")`는 거짓)이라 **Linux에서 minecraft와 zomboid의 auto-detect는 매치에 실패한다** (`D:src/process_monitor.rs:125-130`).

---

## 4. 모듈별 기능 요약

공통 사항:
- 모든 모듈은 version 0.1.0, author WareAoba다.
- `locales/` 아래에 10개 언어 JSON이 있다: `de, en, es, fr, ja, ko, pt-BR, ru, zh-CN, zh-TW`.
- `[aliases]`(한국어 별칭), `[errors]`(사용자용 오류 문구), `[backup]`, `config.announce_command`가 있다. 모두 `schema_version = 1`이다.

### 4.1 minecraft (`M:minecraft/`, lifecycle 1,876줄, toml 1,138줄)

| 항목 | 내용 |
|---|---|
| protocols | `supported=["stdin"]`, default stdin, `interaction_mode=console` |
| 서버 종류 | 설정 `server_type`(`install = true`): vanilla, paper, folia, purpur, pufferfish, spigot, craftbukkit, fabric, quilt, forge, neoforge. 판정·설치 소스·실행 대상은 `M:minecraft/server_types.py`의 표(`SERVER_TYPES`, versions.list 접두사, Main-Class, 폴더 흔적)만 읽는다. 판정은 jar 안의 `META-INF/versions.list`(번들된 서버 jar 이름)와 MANIFEST Main-Class, 그다음 폴더 흔적(`config/paper-global.yml`, `purpur.yml`, `.fabric/`, `libraries/net/neoforged/neoforge/<버전>/` 등)으로 한다. 설치는 vanilla(Mojang), paper·folia(PaperMC Fill v3, 안정 채널, sha256 검사), purpur(Purpur API, md5), fabric(Fabric meta 서버 런처 → `fabric-server-launch.jar`)만 되고 나머지는 가져오기로 추가한다. 1.17+ Forge/NeoForge는 `@libraries/…/<버전>/win_args.txt`(또는 unix)로 띄운다. 고른 종류와 서버 파일이 다르면 `validate`가 `SERVER_TYPE_MISMATCH` 경고를 낸다 |
| 플러그인·모드 | `M:minecraft/plugins.py`가 `plugins/*.jar`·`mods/*.jar`의 `paper-plugin.yml`·`plugin.yml`·`fabric.mod.json`·`quilt.mod.json`·`META-INF/(neoforge.)mods.toml`을 읽어 판정한다(파일 이름은 보지 않는다, `.jar.disabled`는 꺼짐). `instance_commands()`가 plugin.yml에 선언된 명령을 자유 입력 한 칸(`{args}`)으로, `M:minecraft/plugin_catalog/<slug>.json`(플러그인 id `match`가 키, 공식 문서·소스 기준)의 명령을 입력 칸까지 준다. 한국어 문구는 `plugin_catalog/locales/ko.json`, 없으면 데이터의 영어. 카탈로그 22종 740개(EssentialsX·Spawn·Chat·Discord, LuckPerms, WorldEdit, WorldGuard, Multiverse Core·Portals·NetherPortals·Inventories, CoreProtect, GriefPrevention, Geyser, Floodgate, ViaVersion, PlaceholderAPI, Vault, spark, Chunky, ChunkyBorder, Citizens). 콘솔에서 못 쓰는 명령은 각 파일 `player_only`에 이름만 적는다 |
| 설치 | `install.method="download"`. `ServerInstaller`(`:1480-1750`)가 Mojang `version_manifest_v2`(`:1477`)에서 버전 목록을 받아 jar를 내려받는다. Java는 `JavaDetector`와 `JavaDownloader`(Adoptium API `:284`)로 감지하거나 `<working_dir>/java/`에 번들로 받는다 |
| 실행 | `get_launch_command`가 `java -Xmx/-Xms{ram}`, 선택적 Aikar flags, `jvm_args`, 실행 대상(`server_types.launch_target`: `-jar <서버 jar> nogui` 또는 Forge/NeoForge 인자 파일)을 만든다. 시작 전에 `enable-rcon=false`를 강제한다(모든 모드) |
| 설정 | `ServerPropertiesManager`가 `server.properties`를 읽고 쓴다. 모듈 폴더에 동봉된 `server.properties`를 기본값으로 쓴다 (`_load_default_properties` `:507`). settings 66개(saba-chan 5 / advanced 40 / basic 21) |
| 명령 | `[commands] hook = true`(설치된 플러그인·모드 명령은 위 행). module.toml에는 17개, 전부 `method="console"`(콘솔 stdin): say, give, save-all, list, weather, difficulty, gamemode, time, tp, kick, ban, pardon, op, deop, whitelist, seed, stop |
| 정지 | managed는 데몬이 stdin으로 `stop`을 보낸다. 모듈 `stop()`은 force가 아니면 콘솔 `stop`(`instance_id` 필요) → SIGTERM(POSIX) 순서로 요청하고 30초 기다린 뒤에만 `force_kill`한다 (`:1266-1318`). ~~RCON `stop` 시도 후 실패하면 바로 강제 종료~~ → 해결. `command()`도 `daemon_stdin_command`로 콘솔에 보낸다 (`:1425-`) |
| 기타 | `EulaManager`(accept_agreements), `ErrorDetector`(diagnose_log), `MinecraftPing`(SLP 기반 status), `get_installed_version`(jar 내부 `version.json` 파싱), syntax_highlight 규칙 40개(Vanilla·Forge `[시각] [스레드/레벨]`과 Paper 콘솔 `[시각 레벨]`을 모두 받는다). 색은 모두 GUI 팔레트 토큰이다(`saba-chan-gui/src/utils/syntaxHighlight.js:16-36`). ~~팔레트에 없는 `notice`·`critical`을 써서 색이 안 나왔다~~ → 해결: `info`·`error`로 바꿨다 (zomboid의 `duration`도 `number`로) |
| 미등록 | `get_credentials`만 없다(RCON 값만 돌려주던 것이라 지웠다). `import_settings`, `reset_properties`, `reset_server`는 등록됨 |
| 동봉 파일 | `_sample_log.txt`, `test_lifecycle.py`가 리포에 있고 zip에도 들어간다. 패키저는 `__pycache__`·`tests` 폴더와 `.pyc`만 뺀다 (`D:scripts/versions.py:195-207`) |

### 4.2 palworld (`M:palworld/`, lifecycle 1,507줄, toml 1,338줄)

| 항목 | 내용 |
|---|---|
| protocols | `supported=["rest"]`, `interaction_mode=commands`, `credential_map.rest_password="AdminPassword"` |
| 설치 | `steamcmd`, app_id 2394010, `requires_extensions=["steamcmd","ue4-ini"]`. `extensions.steamcmd.SteamCMD`를 사용한다 |
| 실행 | `--port=<port>`. 실제 서버 `Pal/Binaries/Win64/PalServer-Win64-Shipping-Cmd.exe`가 있으면 런처 대신 그것을 직접 띄운다(`:322-331`). `[launch] console = "pty"`. 시작 전 `_enforce_rest_policy`로 INI에 RESTAPIEnabled와 AdminPassword를 보장하고, `AdminPassword`를 최상위로 돌려주어 데몬이 rest_password를 동기화하게 한다 (`:216-346`) |
| 정지 | stdin이 없어 데몬이 모듈 `stop()`을 부른다. REST로 저장·종료를 요청하고 60초 안에 안 끝나면 강제 종료한다 (`:399-426`) |
| 설정 | `PalWorldSettings.ini`의 `OptionSettings=(...)`를 **`extensions.ue4_ini`** 익스텐션으로 파싱한다. 동봉된 `DefaultPalWorldSettings.ini`가 기본값이다. settings 117개. 목록 값 키(`_LIST_KEYS`, 지금은 `DenyTechnologyList`)는 화면의 `PALBOX,RepairBench`를 INI에 `("PALBOX","RepairBench")`로 쓰고 읽을 때 쉼표 형태로 되돌린다 (`:884-904, 927-928, 975-977`). `configure()`는 저장마다 `RCONEnabled=False`를 쓴다 (`:933`) |
| 명령 | 10개, 전부 `method="rest"`(`/v1/api/...`): announce, info, players, metrics, settings, save, shutdown, kick, ban, unban. `resolve_player_id`가 닉네임을 SteamID로 해석한다 (`:112`). `protocol_mode`와 상관없이 늘 REST로 보낸다. ~~`protocol_mode=rcon`이면 데몬 `/rcon`으로 보내는 RCON 명령 경로~~ → 해결: 지워졌다 (`:1224-`) |
| 의존성 | `extensions.ue4_ini`는 `try`로 불러온다(`:33-36`). 익스텐션이 없으면 모듈은 로드되고, INI를 만지는 함수만 `extension_required`를 돌려준다 (`:742-757, 1494-1502`) |
| 미등록 | `get_installed_version` |
| 기타 | `list_versions`는 Steam 단일 버전 |

### 4.3 zomboid (`M:zomboid/`, lifecycle 1,492줄, toml 1,554줄)

| 항목 | 내용 |
|---|---|
| protocols | `supported=["stdin"]`, console, `stop_command="quit"`. `RCONPassword`를 비워 RCON을 끈다 |
| 설치 | `steamcmd`, app_id 380870. steamcmd 익스텐션이 켜져 있지 않으면 `extension_required`를 돌려준다(`:1255-1261`). 익스텐션을 쓰고, 없으면 시스템 steamcmd(`_find_steamcmd`)로 대체한다 |
| 실행 | `StartServer64.bat`나 `start-server.sh`를 **파싱**해서(`_parse_bat`, `_parse_sh`, `_parse_java_cmdline` `:607-741`) 번들 Java, classpath, native lib을 찾는다. 그다음 `java ... zombie.network.GameServer -servername <name> -cachedir=...`를 직접 구성한다 |
| 설정 | `IniManager`가 `Zomboid/Server/<servername>.ini`를 다룬다. `_INI_KEY_MAP`(saba 키 → INI 키). settings 117개. `[backup] hook = true`라 `backup_paths()`가 세이브 위치를 계산한다 |
| 명령 | 23개, 전부 `console`(콘솔 stdin): servermsg, players, save, kick, banuser, unbanuser, adduser, setaccesslevel, teleport, teleportto, createhorde, godmod, invisible, startrain, stoprain, chopper, gunshot, additem, addxp, showoptions, reloadoptions, changeoption, quit |
| 정지 | managed는 데몬이 stdin으로 `quit`을 보낸다. 모듈 `stop()`(non-managed, stdin 없는 managed)은 force가 아니면 콘솔 `save`→`quit`(`instance_id` 필요) → SIGTERM(POSIX) 순서로 요청하고 30초 기다린 뒤에만 강제 종료한다. PID 확인·종료는 공용 헬퍼를 쓰고, `status()`는 PID 생존만 본다 (`:1020-1092`). ~~예전 버그(managed 기본 True, `input` 키, 토큰 없음)~~ → 해결. ~~`tasklist`·`os.kill`로 직접 확인·종료, RCON 폴백(stop·status·command)~~ → 해결: 지워졌다 |
| 미구현 | `list_versions`, `get_version_details`, `get_installed_version`, `reset_properties` |

### 4.4 __example (`M:__example/`)

- 파일명이 `__example__module.toml`, `__example__lifecycle.py`다. 로더는 `module.toml`과 `entry`(여기서는 `lifecycle.py`)를 요구하므로 **이름을 바꿔야** 동작한다. 폴더명도 `_`로 시작해 빌드에서 제외된다(의도된 것).
- `name = "mygame"`인데 폴더명은 `__example`이다. toml 주석은 "폴더명과 동일"을 요구한다(예시 값).
- 템플릿 주석에 있는 "데몬 호출 시점" 설명(`:36-50`)과 config 키 설명(`:56-75`)은 §2.3과 같은 이유로 **사실과 다르다**(예: `instance_id`·`settings` 항상 포함). `rcon_port`·`rcon_password` 예시는 지워졌다.
- RCON 대신 stdin을 가르친다: `protocols.supported = ["stdin"]`, RCON 설정·자격증명 예시 삭제, 공용 `daemon_stdin_command`·`wait_for_exit`·`force_kill`을 쓰고 대기 후에만 강제 종료한다 (`__example__lifecycle.py:92-97, 236-256`, `__example__module.toml:58-60`). 진입점은 공용 `run_lifecycle()`이고, 주석의 명령 방식도 `method="console"`로 맞췄다(e1d7743). `stop()`의 콘솔 정지는 콘솔이 없을 때 불리므로 보통 실패하고 대기 후 강제 종료로 넘어간다(§6.3 #8).
- `install_server`는 실제 모듈처럼 `extensions.steamcmd.SteamCMD` 클래스를 쓴다 (`:394-395`).
- `get_credentials`, `read_properties`가 없다. 명령은 stdin으로 가므로 콘솔 모듈에는 지장이 없지만, REST 모듈을 만들면 `get_credentials`를 직접 추가해야 한다.
- settings에 `managed_start`와 `graceful_stop`이 있다. `managed_start`는 데몬이 시작 경로를 고르는 데 쓴다(§3.1). `graceful_stop`은 데몬도 예시 `stop()`도 읽지 않고(설정 동기화에서 빼기만 한다, `instance.rs:1007`), GUI만 정지 방식을 고를 때 읽는다 (`saba-chan-gui/src/hooks/useServerActions.js:480`).

### 4.5 i18n과 locales

- 모듈 측: `I18n(MODULE_DIR)`이 `locales/<SABA_LANG>.json`을 읽고, 없으면 `en`으로 대체한다. 키 구조는 `module`, `settings`, `commands`, `presets`, `queries`, `agreements`, `errors`, `messages`, `rcon`, `reset` 등이다(minecraft ko.json 기준). `queries.<name>.label`·`description`은 데몬도 읽는다: 액션 목록(`GET /api/actions?instance=`)과 heartbeat `actions`의 조회 제목·설명(`actions::module_locale`, `<모듈 폴더>/locales/<lang>.json`). `reset.label`·`reset.description`은 서버 초기화(`reset_server`)가 무엇을 지우고 남기는지 GUI 초기화 화면에 보여 주는 문구다(모듈마다 다르다: minecraft는 JAR·eula.txt만 남김, palworld는 설정만 기본값으로, zomboid는 월드 세이브만 지움).
- 인터페이스 측: `GET /api/module/:name/locales`가 `<modules_dir>/<name>/locales/*.json`을 하나의 맵으로 묶어 돌려준다 (`D:src/ipc/handlers/config.rs:192-226`). GUI는 이를 `mod_<name>` 네임스페이스로 쓴다. 이 경로 계산은 `module.path`가 아니라 **모듈 이름으로 만든 경로**라서, ZIP 모듈(`.extracted/<name>`)이나 폴더명이 name과 다른 모듈에서는 locales가 비어 나온다.

---

## 5. 패키징과 배포

### 5.1 릴리즈에 싣기 (`D:scripts/versions.py manifest`, `D:.github/workflows/release.yml`)

- 모듈 저장소에는 릴리즈 워크플로가 없다. 검사용 `M:.github/workflows/check-modules.yml`(스키마·적합성 검사)만 있다.
- 코어 릴리즈 워크플로가 `saba-chan-modules`의 `main`을 checkout하고(`release.yml:163-170`), `versions.py manifest --modules-repo`가 모듈마다 `module-<name>.zip`을 만든다(`versions.py:216-244`, `_`·`.`로 시작하는 폴더는 뺀다). 버전은 각 `module.toml`의 `[module].version`, 목록 화면용 정보(`display_name`, `description`, `game_name`, `requires_extensions`, 아이콘 URL)는 `meta`에 들어간다.
- 이 zip들은 코어·GUI·CLI와 **같은 릴리즈 manifest**(`components`의 `module-<name>` 키)에 실려 서명되고, R2 `releases/<version>/`에 올라간다. 그래서 모듈 목록과 버전은 릴리즈 단위로 고정된다.

### 5.2 데몬의 목록·설치

- **`GET /api/modules/manifest`**: `relay::latest_catalog`로 최신 릴리즈 manifest를 받아 `module-*` 항목을 `{modules:{<이름>:{version, sha256, size_bytes, ...meta}}}`로 돌려준다 (`D:src/ipc/handlers/server.rs:429-449`).
- **`POST /api/modules/manifest/:id/install`** (`server.rs:452-505`):
  1. `id`에 `..`, `/`, `\`가 있으면 거부한다.
  2. manifest의 `module-<id>`가 없으면 실패한다.
  - 설치된 모듈 버전이 manifest 버전보다 높으면 받지 않는다(`version::is_downgrade`). 릴레이가 예전 서명 manifest를 다시 보내 옛 버전을 깔게 하는 것을 막는다.
  3. R2 URL에서 `<modules>/_tmp_module_<id>.zip`로 받으며 SHA-256을 확인한다(`download_verified`).
  4. `.staging-<id>`에 풀고 `module.toml`이 있을 때만 기존 `<modules>/<id>`를 지우고 바꾼다. 그래서 업데이트해도 지워진 파일이 남지 않는다.
  - 실패도 HTTP 200 `{ok:false, error}`로 돌려준다.
- **신뢰 표시 알림**: 서명된 카탈로그에서 받지 않았거나(`unverified`) 받은 뒤 바뀐(`modified`) 모듈로 인스턴스를 만들면 `POST /api/instances` 응답에 `module_trust`와 `warnings[{code, module}]`가 실린다. 알림일 뿐 만들기·실행은 막지 않는다(`server.rs` `trust_warnings`). CLI는 경고 줄로 보인다.
- **`DELETE /api/modules/:id`**: `..`·`/`·`\`가 든 `id`는 거부하고, `<modules>/<id>`를 `remove_dir_all`로 지운다. 여기서 `id`는 **디렉터리명**이다 (`server.rs:508-531`).

### 5.3 업데이터

- `check_for_updates`가 같은 manifest에서 **설치된 모듈만** 골라 버전을 비교한다(`D:updater/src/lib.rs:467-539`). 새 모듈 설치는 목록 화면(§5.2)의 몫이다.
- 무결성 검사는 업데이터가 적용할 때 zip에서 푼 파일마다 기록한 SHA-256(`<data_dir>/installed-files.json`)과 디스크 파일을 비교한다. manifest의 zip 해시와는 비교하지 않는다 (`D:updater/src/integrity.rs:1-6, 101-143`). 설치기로 처음 깐 모듈은 기록이 없어 결과에 나오지 않는다.
- **installer**: 모듈 목록도 같은 manifest의 `module-*`에서 만든다(`installer/gui/src-tauri/src/lib.rs:159-178`, `release.rs`). 하드코딩 폴백은 없고, 받지 못하면 화면이 알린다.

---

## 6. 불일치, 죽은 코드, 미완성, 원칙 위반

### 6.1 로더와 모듈·문서 사이의 불일치
1. ~~`[module].entry`가 쓰이지 않는다~~ 해결됨: 데몬이 `entry_path()`로 쓰고 로드 때 파일 존재를 확인한다 (§1.1).
2. ~~command `inputs`의 `options`, `min`, `max`가 버려진다~~ → 해결: `CommandInputToml`·`CommandInput`이 받고 데몬이 입력 검사에 쓴다 (`module_loader.rs:264-281, 738-757`).
3. ~~`[detection.linux]`, `install.beta_branch`, `detection.common_paths`, `config.*` 추가 키, `[errors]`, `game_name`, `display_name`이 무시된다~~ → 해결: schema v1이 모르는 키를 로드 오류로 내고, 모듈에서 그 키들이 지워졌다. `[errors]`·`game_name`·`display_name`은 메타데이터에 실린다 (`module_loader.rs:957-997`).
4. ~~`CommandField.method` 주석은 `both`인데 인터페이스 분기는 `dual`이다~~ → 해결: 데몬 `crate::commands`가 `console`/`rest`/`module`(`rcon`·`dual`·`both` 별칭)을 해석하고 인터페이스는 분기하지 않는다 (`D:src/commands/mod.rs:104-128`).
5. ~~`/api/modules` 응답에 `game_name`과 `display_name`이 없다~~ → 해결: `ExtensionInfo`에 둘 다 있다 (`D:src/ipc/mod.rs:255-258`).
6. ~~ZIP 로딩에서 `ends_with("module.toml")`로 첫 엔트리를 고른다~~ → 해결: 이름이 정확히 `module.toml`인 가장 얕은 엔트리를 고른다 (`module_loader.rs:1164-1175`). `.extracted` 재추출 문제와, locales·설치 경로가 `<modules>/<name>`을 가정하는 문제는 남았다 (§1.1, §4.5).

### 6.2 데몬 측 죽은 코드와 stub
| 대상 | 위치 |
|---|---|
| ~~`state_machine.rs` 전체~~ → 해결: 파일이 지워졌다 | — |
| ~~`SupervisorError` 전체~~ → 해결: `error.rs`가 지워졌다 | — |
| ~~`ProcessTracker::get_status`, `mark_crashed`, `terminate`, `ProcessStatus::Crashed`~~ → 해결: 지워졌다 | `process.rs` |
| ~~`Supervisor::monitor()` stub, `run()`~~ → 해결: 지워졌다 | `mod.rs` |
| ~~`ModuleMetadata::steam_app_id`, `has_extension_config`~~ → 해결: 지워졌다 | `module_loader.rs` |
| ~~`ModuleLoader.get_module`의 잘못된 `#[allow(dead_code)]`~~ → 해결: 속성이 없다 | `module_loader.rs:1221` |
| ~~`LoadedModule.is_zip` 미사용~~ → 해결: 필드가 지워졌다 | `module_loader.rs:312-315` |
| ~~`plugin::run_extension*`~~ → 해결: 지워졌다 | `plugin/mod.rs` |
| ~~`ManagedProcess::wait_for_exit`, `subscribe`~~ → 해결: broadcast 채널과 함께 지워졌다 | `managed_process.rs` |

### 6.3 모듈 측 미완성과 버그
1. ~~FUNCTIONS 누락: minecraft의 `import_settings`, `reset_properties`, `reset_server`, palworld의 `import_settings`~~ → 해결: 모두 등록됐다 (§2.4).
2. ~~모든 모듈의 `start()`가 죽은 코드다~~ → 해결: 모든 모듈에서 지워졌다.
3. ~~모듈→데몬 HTTP 콜백이 인증 토큰을 보내지 않고 포트도 57474로 하드코딩돼 있다~~ → 해결: `daemon_post`가 `SABA_TOKEN_PATH`의 토큰을 `X-Saba-Token`으로 보내고 주소는 `SABA_IPC_URL`을 쓴다. palworld·zomboid도 이것을 쓴다 (`D:modules/daemon_rcon.py:53-99`).
4. ~~zomboid `stop`의 stdin 콜백은 payload 키가 틀리고(`input` vs `command`), managed 판정도 틀렸다~~ → 해결: 공용 `daemon_stdin_command`(`{"command": ...}`)를 쓰고 `managed` 분기는 없어졌다 (`M:zomboid/lifecycle.py:1052-1092`).
5. ~~palworld가 최상위에서 `extensions.ue4_ini`를 import해서 익스텐션에 하드 의존한다~~ → 해결: `try`로 불러오고 없으면 해당 함수만 `extension_required`를 돌려준다 (`M:palworld/lifecycle.py:33-36, 742-757`).
6. ~~`__example`의 `install_server`가 쓰는 steamcmd API(`install_game`)가 실제 모듈과 다르다~~ → 해결: `SteamCMD` 클래스를 쓴다 (`M:__example/__example__lifecycle.py:394-395`).
7. `get_installed_version`의 `jar_name` 기본값이 `"server.jar"`다. 데몬 코드 안의 Minecraft 전용 가정이다 (`mod.rs:1890-1893`).
8. ~~데몬의 `stop` config에 `instance_id`가 없어 모듈 `stop()`의 콘솔 정지가 실행되지 않는다~~ → 해결: 모듈 호출에 `instance_id`가 들어간다(`D:src/supervisor/mod.rs:296-297`, 59a93b8). 정지 경로는 이렇다. 콘솔이 있는 managed 서버는 데몬이 `stop_command`를 stdin으로 보내고 끝나기를 기다린다(`mod.rs:756-776`, 테스트 `normal_stop_asks_the_console_and_waits`). 모듈 `stop()`은 콘솔이 없을 때(non-managed, REST 전용, stdin 없는 재부착)만 불리므로, 그 안의 콘솔 정지 시도는 보통 실패하고 SIGTERM(POSIX)이나 대기 후 강제 종료로 넘어간다.

### 6.4 데몬 프로토콜과 운영 문제
1. ~~`SABA_ENABLED_EXTENSIONS`의 상태 파일 경로가 `ExtensionManager`와 다르다~~ → 해결: 둘 다 `<data_dir>/config.json`의 `extensions.enabled`를 읽는다 (`plugin/mod.rs:198-205`).
2. ~~데몬이 `SABA_LANG`과 `DAEMON_API_URL`을 주입하지 않는다~~ → 해결: `SABA_IPC_PORT`, `SABA_IPC_URL`, `SABA_TOKEN_PATH`, `SABA_LANG`을 넣는다 (`plugin/mod.rs:18-37`). 이름이 `DAEMON_API_URL`이 아니라 `SABA_IPC_URL`이라 모듈 문서와 다르다 (§2.1).
3. `/api/module/:name/install` 경로의 타임아웃이 120초다 (`mod.rs:1822`).
4. ~~non-managed는 이름으로, 나머지는 ID로 추적한다~~ → 해결: non-managed도 인스턴스 ID로 추적한다 (`mod.rs:651-652`).
5. ~~`managed/start`가 write lock을 잡은 채로 Python을 호출한다~~ → 해결: 시작은 단계마다 락을 짧게 잡고 Python은 락 밖에서 부른다 (`managed.rs:57-58`, C12).
6. ~~`server.post_stop` hook을 정지 전에 dispatch한다~~ → 해결: hook 이름이 `server.stop`이고 정지 전에 부른다고 명시돼 있다 (`mod.rs:713-716`).
7. ~~managed 모드에서는 RCON을 끄는데, 명령은 RCON으로 라우팅된다~~ → 해결: 데몬은 RCON을 쓰지 않고 명령은 모두 stdin으로 간다(RCON 정책).
8. ~~RCON/REST 명령 하나마다 `get_credentials` Python 프로세스를 띄운다~~ → 해결: REST만 쓰고 결과를 캐시한다(`command.rs:531-558`).
9. ~~모듈 설치에 sha256 검증, 경로 검증, 클린 설치가 없고, 업데이터의 버전 판정과 무결성 해시 대상이 맞지 않는다~~ → 해결: 설치는 SHA-256 검증·id 검사·staging 교체를 하고(`server.rs:452-505`), 버전은 manifest의 semver로 비교하며(`D:updater/src/lib.rs:467-539`), 무결성은 설치 때 기록한 파일별 해시를 본다 (`D:updater/src/integrity.rs:1-6`).

### 6.5 [원칙 위반] 모듈 지식이 데몬 바깥이나 데몬 하드코딩에 있는 곳

| # | 위치 | 내용 | 원칙 |
|---|---|---|---|
| 1 | ~~GUI·봇의 명령 실행 로직~~ | ~~`method` 분기, 템플릿 치환, 입력 검증을 인터페이스가 중복 구현~~ → 해결: `POST /api/instance/:id/command` 하나로 모이고 데몬 `crate::commands`가 해석한다. GUI는 그 경로만 부른다 (`D:src/commands/mod.rs:1-17`, `saba-chan-gui/main.js:2014-2020`). 로컬 Discord 봇은 지워졌다 | — |
| 2 | ~~GUI·봇의 시작 경로 선택~~ | ~~인터페이스가 `/managed/start`와 `/server/:name/start`를 고른다~~ → 해결: `POST /api/instance/:id/start`에서 데몬이 고른다 (`server.rs:563-616`, `saba-chan-gui/main.js:1455`) | — |
| 3 | — | ~~`dir_signatures` 매칭을 GUI 마이그레이션 화면이 직접 한다~~ → 해결: `migration:scanDir`가 데몬 `POST /api/modules/detect`(`server.rs:370-390`)를 부르고 화면은 그 `matches`만 쓴다(`saba-chan-gui/main.js:2735-2745`, `AddInstanceMigration.js:73-74`) | |
| 4 | ~~`saba-chan-cli/src/module_registry.rs`의 fs 폴백~~ | ~~CLI가 데몬 응답 실패 시 `module.toml`을 파일시스템에서 직접 파싱~~ → 해결: PR #6 CLI는 모듈 목록을 `GET /api/modules`로만 받는다(`saba-chan-cli/src/resolve.rs:184-193`, `cmd/module.rs:99-100`). 실패하면 오류로 끝나고 로컬 폴백이 없다 | — |
| 5 | ~~raw TOML로 `[errors]` 읽기~~ | ~~봇이 `/api/module/:name`의 raw TOML 덤프를 읽는다~~ → 해결: `[errors]`는 메타데이터 필드이고 `/command`가 `error_message`로 붙인다. `/api/module/:name`은 메타데이터를 돌려준다 (`server.rs:394-414`) | — |
| 6 | `D:src/ipc/handlers/instance.rs:991-1017` | 설정 동기화 제외 목록에 `java_path`, `ram`, `use_aikar_flags`(Minecraft 전용 키)와 `managed_start`, `graceful_stop`이 **데몬에 하드코딩**돼 있다. `rcon_port`·`rcon_password`·`enable_rcon`은 게임 파일로 넘기지 않으려고 뺀다. 주석·로그는 "server.properties"(Minecraft 파일명) | 게임별 지식이 데몬에 있음 (모듈 매뉴얼 소관) |
| 7 | `D:src/supervisor/mod.rs:277, 1592, 1890-1893, 1614, 1668` | `server_jar` 키 주입, `jar_name="server.jar"` 기본값, `manage_properties`라는 Minecraft식 명명이 데몬 공용 API에 들어가 있음 | 동일 |
| 8 | `saba-chan-gui/src/components/ServerSettingsModal.js:354,363` (`module === 'palworld'`), `useServerSettings.js:256` ("Install Minecraft …" 문자열) | GUI가 특정 게임 이름으로 분기 | 인터페이스에 게임 지식 |
| 9 | `M:palworld/lifecycle.py:33-36`, `zomboid`/`palworld` `install_server` | 모듈이 플러그인(`extensions.*`)을 Python import로 직접 사용. 활성 여부도 스스로 `SABA_ENABLED_EXTENSIONS`로 확인. 데몬은 import 가드로 꺼진 익스텐션 import만 막는다 (`D:src/plugin/mod.rs:85-97`) | 플러그인 기능 주입은 데몬이 중개해야 하는데, 모듈과 플러그인이 직접 결합 |
| 10 | `modules/daemon_rcon.py`, palworld, zomboid 콜백 | 모듈이 데몬 HTTP API를 역호출(rcon/rest/stdin). ~~인증 없이 호출하고 포트도 하드코딩~~ → 토큰과 `SABA_IPC_URL`을 쓴다 | "매뉴얼"인 모듈이 능동 클라이언트로 동작. 경계가 모호함 |
| 11 | ~~`installer/gui/src-tauri/src/lib.rs`의 하드코딩 모듈 목록~~ | → 해결: 목록은 릴리즈 manifest에서만 만든다 (`lib.rs:159-178`) | — |

---

### 부록: 핵심 수치
- 모듈 크기: minecraft lifecycle 1,876줄 / toml 1,138줄, palworld 1,507 / 1,338, zomboid 1,492 / 1,554, example 511 / 336.
- settings 필드 수: minecraft 66, palworld 117, zomboid 117, example 9. commands 수: 17 / 10 / 23 / 3.
- 데몬의 lifecycle 호출 함수: 17종(§2.3). 모듈의 `start`는 지워졌다.
