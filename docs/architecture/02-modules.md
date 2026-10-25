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
| 모듈 루트 | `SABA_MODULES_PATH` 환경변수 → `config.json` `paths.modules` → `resolve_data_dir()/modules`(Windows `%APPDATA%/saba-chan/modules`, Linux/macOS `$HOME/.config/saba-chan/modules`). 사용자가 GUI 설정·`saba-chan-cli config move modules`·설치기로 옮길 수 있다(01 §1.5) | `D:updater/src/component_dirs.rs` `resolve` |
| GUI가 데몬을 띄울 때 | 모듈 폴더를 주입하지 않는다. 데몬이 위 순서로 정한다 | `D:saba-chan-gui/src-tauri/src/lifecycle.rs` |
| 레거시 경로 | `Supervisor::run()`은 지워졌다. `"./modules"`는 테스트에만 남았다 | `D:src/supervisor/mod.rs:2633-` |

`ModuleLoader::discover_modules()` (`D:src/supervisor/module_loader.rs:1481-1548`)의 동작은 다음과 같다.

- 모듈 루트의 **1단계 엔트리만** 순회하고 재귀하지 않는다.
- **폴더 모듈**: `<dir>/module.toml`이 있는 디렉터리만 로드한다 (`:1521-1541`). 폴더 이름은 아무 것이나 되며, 식별자는 `[module].name`이다.
- **ZIP 모듈**: 확장자가 `.zip`인 파일이 대상이다. 다운로드는 128 MiB, 항목은 20,000개, 누적 해제 크기는 1 GiB로 제한한다. 압축 해제 중에도 실제 출력 크기를 센다.
  - 이름이 정확히 `module.toml`인 엔트리 중 **가장 얕은 것**을 파싱한다 (`:1559-1570`). `[module].name`이 폴더 이름 하나가 아니면 거부한다 (`:1584-1586`).
  - 아카이브 전체를 tempdir에 풀었다가 `<modules>/.extracted/<name>/`로 복사한다. 이 과정에서 기존 디렉터리는 `remove_dir_all`로 지운다.
  - 캐시가 무효화될 때마다(`refresh_modules`) 다시 풀린다. 그동안 실행 중인 Python 프로세스가 그 디렉터리를 쓰고 있어도 삭제된다.
- 결과는 `RwLock<Option<Vec<LoadedModule>>>`에 캐시된다. `invalidate_cache()`(`:1475`)는 `/api/modules/refresh`, 모듈 설치·삭제 시에만 호출된다.
- 로드에 실패한 모듈은 `warn` 로그를 남기고 `load_errors`에 기록한다. `GET /api/modules`가 `failed[]`로 돌려준다 (`:1511-1539`, `D:src/ipc/handlers/server.rs:391-392`).
- `.py` 파일(`daemon_rcon.py`, `i18n.py`)이 같은 루트에 있어도 무시된다. 이 둘은 PYTHONPATH용 공용 라이브러리다 (§2.1).

**진입점 파일**: `[module].entry`가 데몬이 실행하는 Python 파일이다(보통 `lifecycle.py`). `LoadedModule::entry_path()`가 `<모듈 폴더>/<entry>`를 만들고(`D:src/supervisor/module_loader.rs:462-464`), 데몬의 모든 lifecycle 호출이 이 경로를 쓴다(예: `D:src/supervisor/mod.rs:791, 947, 1166, 1519`). 로드할 때 `check_loadable()`이 이 파일이 실제로 있는지 확인하고, 없으면 모듈을 로드하지 않는다(`module_loader.rs:467-481`). 검증 단계는 `entry`가 비었거나 절대 경로이거나 모듈 폴더 밖을 가리키면(`..`·`.` 포함) 거부한다(`:1250-1256`, 익스텐션 `python_modules`와 같은 `utils::is_inner_relative_path`). (qcza9f 기준)

### 1.2 module.toml 스키마 (Rust 역직렬화 기준, 전체)

파서는 `parse_module_toml()` → `serde_ignored::deserialize::<ModuleToml>` → `into_metadata()`이다 (`module_loader.rs:1193-1246, 1037-1189`). 이 구조체들이 계약 그 자체이고, 같은 구조체에서 JSON Schema가 나온다(`module_json_schema()` `:1444-1446`, `GET /api/modules/schema`, `saba-core module schema`). 모듈 저장소는 이것을 `M:schema/module.schema.json`으로 두고 `M:.github/workflows/check-modules.yml`에서 검사한다.

- **스키마 2** (0.1.9, MK2·MK8): `schema_version = 2`는 스키마 1에 설정 타입 `list`·`multiline`과 설정 키 `min_items`·`max_items`·`max_length`·`restart_required`·`visible_when`을 더한 것이다(`MODULE_SCHEMA_VERSION = 2`, `SETTING_TYPES_V2`). 이 타입·키가 스키마 1 모듈에 있으면 로드에 실패하고, 스키마 이전 모듈은 경고만 남기고 이 키를 끄며, `list`·`multiline` 타입은 0.1.8처럼 `text`로 다룬다. 0.1.8 데몬은 스키마 2 모듈을 로드하지 않으므로 스키마 2 모듈은 `[module.dependencies] saba-core = ">=0.1.9"`도 적는다. 0.1.9로 내는 공식 모듈은 스키마 1이다. 표시 없는 모듈에게 권하는 버전은 1이다.
- **로드 실패 표시**: 스키마에 맞지 않으면 `failed[]`에 `error_code:"module_schema_invalid"`와 `problems`(`SchemaInvalid`. `schema_version`을 밝힌 모듈은 알려진 키의 타입 오류도 여기에 실린다), 데몬보다 새 스키마면 `"module_schema_unsupported"`와 `required`(`SchemaUnsupported`)가 실린다.
- **모르는 키·잘못된 값**: `[module] schema_version = 1`(또는 2)이면 모듈 로드가 실패한다. `schema_version`이 없으면(스키마 이전 모듈) `warnings`에만 남기고 로드한다. 데몬보다 새 버전이면 거부한다 (`:1222-1243`). 값 검사는 `entry` 경로, `risk` 값, `config.announce_command`·`backup.save_command`가 `[[commands.fields]]` 이름을 가리키는지다 (`:1248-1307`). 세 실제 모듈은 모두 `schema_version = 1`이다.
- **알고 있는 키의 타입이 틀리면 모듈 전체 로드가 실패**한다. 예를 들어 settings의 `options`에 숫자를 넣으면 `Vec<String>`이라 실패한다.
- BOM: 파서가 앞의 UTF-8 BOM을 떼고 읽는다 (`:1194`). 지금 `palworld/module.toml`에는 BOM이 없다.

표의 기호: ✔ = 필수(없으면 파싱 실패), ○ = 선택.

#### `[module]` (섹션 자체가 필수, `ModuleSection` `:664-691`)

| 키 | 타입 | 필수 | 기본값 / 비고 |
|---|---|---|---|
| `schema_version` | u32 | ○ | 1 또는 2. 위 규칙 참조 |
| `name` | String | ✔ | 모듈 ID. 인스턴스의 `module_name`과 매칭됨 |
| `version` | String | ✔ | semver 형식 기대(검증 없음) |
| `entry` | String | ✔ | 실행할 Python 파일. 파일이 없으면 모듈 로드 실패 (위 1.1 참조) |
| `description` | String | ○ | None |
| `icon` | String | ○ | 모듈 폴더 기준 상대경로. `/api/modules`가 base64로 인코딩해 응답 (`D:src/ipc/handlers/server.rs:342-348`). MIME은 무조건 `image/png` |
| `log_pattern` | String(regex) | ○ | `(?P<level>…)` 명명 그룹 필수. 콘솔 레벨 파싱에 쓰임 (§3.3) |
| `dependencies` | Table<String,String> | ○ | `[module.dependencies]`, 값은 `">=x.y.z"`. 두 단계로 강제된다. ① 로드: `saba-core` 요구 버전이 데몬보다 높으면 모듈을 로드하지 않고 `GET /api/modules`의 `failed[]`에 `error_code: "module_dependency_unsatisfied"`, `missing_dependencies`와 함께 싣는다 (`module_loader.rs:467-481, 508-546`, `D:src/ipc/handlers/server.rs:391`). ② 시작: 인스턴스 시작 전에 `saba-core`와 켜진 익스텐션(`ext-<id>`) 요구를 다시 확인하고, 못 맞추면 `module_dependency_unsatisfied`와 `missing_dependencies`, `module`(모듈 이름)로 시작을 거부한다 (`D:src/supervisor/mod.rs:501-544, 568, 1349`). 데몬보다 새 스키마라 로드되지 못한 모듈은 시작도 `module_schema_unsupported`와 `required`, `module`로 거부한다(로드 실패 항목은 `module`이 없으면 모듈 폴더 이름으로 찾는다, `load_failure`). 다른 키(`gui`, `cli` 등)는 데몬이 알 수 없어 넘긴다 |
| `game_name`, `display_name`, `author` | String | ○ | 메타데이터에 실리고 `/api/modules` 응답에도 들어간다 (`D:src/ipc/mod.rs:246-251`) |

#### `[update]` (없어짐)
L3에서 지웠다. 모듈 업데이트는 릴리즈 manifest의 `module-<이름>` 항목으로 판정한다(§5). 옛 module.toml에 남아 있어도 모르는 섹션이라 조용히 무시된다.

#### `[protocols]` (`ProtocolsSection` `:725-735`)
| 키 | 타입 | 사용처 |
|---|---|---|
| `supported` | Vec<String> | `stdin`/`rest`/`rcon`. `rcon`은 `stdin`·`rest`가 없을 때만 쓴다(아래 RCON 정책, `ModuleMetadata::rcon_only_port`). 이 셋 밖의 값은 로드 때 거부한다(`PROTOCOLS`, `value_problems`). 포트 충돌 검사(`build_module_protocols_map`, `D:src/supervisor/mod.rs:255-306`), 인스턴스 생성 시 rest_port 기본값 결정(`instance.rs:112-116`), 명령·정지를 stdin으로 보낼지 판단(`mod.rs:957-959, 1700-1722`) |
| `rcon_port` | u16 | 게임의 기본 RCON 포트. `supported`에 `rcon`만 있으면 필수다(없으면 로드 때 거부, `value_problems`). 데몬이 처음 시작할 때 이 값으로 인스턴스의 RCON 접속 정보를 만든다 |
| `default` | String | GUI로 전달만 함 (`ProtocolsInfo`). `supported`와 같은 값만 받는다 |
| `interaction_mode` | String | `console` 또는 `commands`. 데몬도 GUI도 해석하지 않는다. 데몬은 모든 모듈을 기본 managed로 시작하고(`Supervisor::uses_managed_start`), 그 판정을 `GET /api/servers`의 `ServerInfo.managed`로 알려 준다. GUI는 그 값을 따라(`utils/serverView.js` `isManaged`) managed 서버에 콘솔 탭을 보이고, 입력칸은 `protocols.supported`에 `stdin`이 있을 때만 연다(`interactionOf`). REST 전용 서버는 읽기 전용 콘솔과 명령 탭을 함께 보인다 |

> **RCON 정책**: 명령 경로의 우선순위는 STDIN → REST → RCON이다. 다른 경로가 있으면 RCON을 쓰지 않는다. 데몬의 RCON 클라이언트(`D:src/protocol/rcon.rs`, Source RCON 프로토콜)는 `[protocols] supported`에 `rcon`만 있고 `stdin`·`rest`가 없는 모듈(정말 RCON만 받는 서버)에만 쓰인다. 지금 저장소의 모듈은 모두 `stdin`이나 `rest`라 RCON을 쓰지 않는다. 그런 모듈의 인스턴스를 시작하면 데몬이 인스턴스 폴더에 `rcon.json`(`{port, password}`, 주인만 읽기, 원자적 쓰기)을 한 번 만든다(`Supervisor::ensure_rcon_access`). 포트는 `[protocols] rcon_port`부터 차례로 보아 다른 인스턴스의 RCON·게임·REST 포트가 아니고 지금 열 수 있는 첫 포트(최대 200개)다. 그래서 같은 모듈의 인스턴스 둘이 같은 포트를 받지 않는다. 비밀번호는 무작위 16자다. 준비하지 못하면 시작이 실패한다. 모듈이 업데이트로 `stdin`·`rest`를 갖게 되면 다음 시작 때 `rcon.json`을 지운다. 모듈 config에는 `rcon_port`·`rcon_password`로 이 값이 실린다(모듈 설정·요청의 같은 키보다 앞선다, `insert_instance_fields`). 모듈은 시작 때 이 값을 게임 설정에 쓰고 RCON을 127.0.0.1에서만 받게 한다. 데몬은 늘 `127.0.0.1:<port>`에만 접속한다. 비밀번호는 `rcon.json`에만 저장하고 인스턴스 API 응답·로그·알림에 넣지 않는다. 실행 인자 미리보기에서는 `***`로 가리고, config 로그는 키 이름으로 가린다. `rcon.json`은 데몬 항목(`DAEMON_ENTRIES`)이라 모듈 데이터 폴더로 넘기지 않는다. 콘솔 명령(`method = "console"`)과 정의에 없는 한 줄은 콘솔이 있으면 stdin으로, 없고 RCON 경로가 있으면 RCON으로 간다. 명령 뒤에 끝 표시(빈 RESPONSE_VALUE)를 보내 그 답으로 여러 패킷 답의 끝을 알고, 끝 표시에 답하지 않는 서버는 첫 답 뒤 0.3초 조용하면 끝으로 본다(시간 초과로 다시 보내 명령이 두 번 도는 일을 막는다). 답은 모은 바이트를 한 번에 UTF-8로 바꿔 `data.response`에 담고 `protocol: "rcon"`을 붙인다. 실패는 400 `invalid_argument`(빈 줄·여러 줄·4096바이트 초과) 또는 502 `rcon_unreachable`·`rcon_auth_failed`·`rcon_timeout`(접속·인증·답 전체 5초)·`rcon_failed`다. 오류 문구에는 비밀번호와 서버 답을 넣지 않는다. RCON 경로가 있어도 서버가 꺼져 있으면 접속하지 않고 409 `server_not_running`이다. RCON 경로가 없는 서버(콘솔 없음)는 예전처럼 409 `no_command_path`다. 서버가 콘솔 입력을 읽지 않아 대기열이 차 있으면 기다리지 않고 503 `console_input_full`을 받는다. 데몬이 재시작해도 saba-runner에 다시 붙어 stdin을 유지한다. 정지는 데몬이 RCON으로 `stop_command`를 보내지 않는다. RCON만 받는 모듈의 `stop()`이 `daemon_post(f"/api/instance/{instance_id}/rcon", {"command": <정지 명령>})`로 데몬 `/rcon`을 불러 정지 명령을 보낸다. 데몬이 4xx·5xx로 답하면 `daemon_post`가 `ConnectionError`를 내므로, 모듈은 그때 `graceful_stop_unavailable`로 답한다(MK10). 별도 RCON 도우미 함수는 없다. 한계: 정해진 RCON 포트를 바꾸는 API는 아직 없다. minecraft는 모든 모드와 설치 시점에 `enable-rcon=false`를 강제하고, zomboid는 `RCONPassword`를 비운다. 두 모듈의 RCON 설정 항목, RCON 폴백(stop·status·command), `get_credentials`는 지워졌고 `supported = ["stdin"]`이다. palworld도 RCON 명령 경로(RCON 클라이언트, `protocol_mode=rcon` 분기, `rcon_enabled`·`rcon_port` 키 매핑, `rcon_password` 반환)를 지웠다. 명령은 늘 데몬 REST 경로로 가고, `configure()`는 저장할 때마다 `RCONEnabled=False`를 쓴다 (`M:palworld/lifecycle.py:933, 1224-`). 콘솔 명령은 `method = "console"` + `console_template`이다(예전 이름 `rcon`·`rcon_template`도 별칭으로 읽힌다).

#### `[config]` (`ConfigSection` `:738-755`)
| 키 | 타입 | 사용처 |
|---|---|---|
| `executable_path` | String | `/api/modules` 응답에 그대로 노출. 데몬은 이 값으로 실행하지 않음 |
| `server_executable` | String | `working_dir + server_executable`로 실행파일 자동 해석 (`mod.rs:306-342`), 설치 시 백업 대상(`:2058-2083`), 마이그레이션 (`instance.rs:633-640`) |
| `process_name` | String | 인스턴스 생성 시 `instance.process_name`에 복사 → auto-detect (`instance.rs:109`, `mod.rs:2514-2524`) |
| `default_port` | u16 | 인스턴스 생성 시 `instance.port` 기본값 |
| `stop_command` | String | managed stop 때 stdin으로 전송. 기본값 `"stop"` (`mod.rs:961`) |
| `stop_timeout` | u64(초) | managed graceful stop 대기 시간. 기본값 30. 인스턴스 설정 `stop_timeout`이 있으면 그 값이 앞선다(§ 데몬이 읽는 setting) |
| `announce_command` | String | 게임 안 공지에 쓰는 `[[commands.fields]]` 이름 (예약 재시작 예고 등) |

`java_path`·`ram` 같은 게임 설정은 `[config]`가 아니라 `[[settings.fields]]`에 있다. `[config]`의 모르는 키는 schema v1에서 로드 오류다.

#### `[detection]` (`DetectionSection` `:759-773`)
| 키 | 타입 | 사용처 |
|---|---|---|
| `process_patterns` | Vec<String> | extension hook `server.status` 컨텍스트로만 전달 (`mod.rs:1115-1126`). 네이티브 auto-detect에는 **미사용** |
| `cmd_patterns` | Vec<String> | auto-detect 때 커맨드라인 필터 (`mod.rs:2515-2524`) |
| `dir_signatures` | Vec<String> | `/api/modules`로 노출. 데몬도 `POST /api/modules/detect`로 매칭한다(`server.rs:423-443`, 항목은 전부 있어야 함). 한 항목 안의 `|`는 그중 하나, `*`는 그 폴더 바로 아래 파일 이름 패턴이다(`"server.jar|paper-*.jar"`, `module_loader.rs` `signature_present`). `..`가 든 항목은 맞지 않는다. GUI 마이그레이션 화면도 이 결과만 쓴다: `migrationScanDir`가 이 API를 부르고(`saba-chan-gui/src/api/createApi.js:825-833`), 화면은 돌아온 `matches`로 모듈을 고른다(`AddInstanceMigration.js:75-76`) |
| `graceful_exit_patterns` | Vec<String> | 정상 종료 문구(정규식). 종료 판정에 쓴다 (§3.6) |
| `crash_artifacts` | Vec<String> | 크래시 때 생기는 파일. 종료 판정에 쓴다 (§3.6) |

`common_paths`와 `[detection.linux]`는 스키마에 없고, 실제 모듈에서도 지워졌다.

#### `[launch]` (`LaunchSection` `:694-702`)
`log_files`(서버가 파일에 쓰는 로그, saba-runner가 따라 읽어 콘솔에 합친다. 줄은 출력과 같이 `MAX_LINE`(64 KiB)에서 끊고, 파일이 줄어들거나 읽은 자리 앞의 바이트(끝 64바이트와 끝나지 않은 조각 전체 중 긴 쪽)가 달라지면 새로 쓴 파일로 보고 처음부터 읽으며 끝나지 않았던 앞 조각은 버린다, `runner/src/main.rs` `read_new`), `console`(`pipe` 기본 / `pty`). palworld가 `console = "pty"`를 쓴다. pty에서 runner는 받은 출력을 줄 단위가 아니라 읽은 만큼 처리하고, Windows ConPTY가 보내는 커서 위치 질의(`ESC[6n`, 두 번에 나뉘어 와도)에 `ESC[1;1R`로 답한다(답이 없으면 콘솔 클라이언트가 멈춘다).

#### `[backup]` (`ModuleBackup` `:215-230`)
`paths`(`{설정 이름}` 치환. 경로 형식이 아닌 설정(`type="file"`·`format="path"`가 아닌 것) 값은 서버 폴더 안 상대 경로만 받고, 절대 경로나 `..`이면 그 경로를 백업·복원 범위에서 뺀다), `save_command`(백업 전에 실행할 명령 이름), `pause_command`·`resume_command`(켜진 서버에서 저장 전에 자동 저장을 멈추고 백업이 끝나면 실패해도 되돌리는 명령 이름. `pause_command`만 있으면 로드 실패), `exclude`(폴더 안에서 담지 않을 파일 이름 패턴, `*` 가능. 켜진 서버가 잠가 두는 파일용), `hook`(true면 lifecycle `backup_paths()`가 경로를 준다), `snapshots`(true면 lifecycle `backup_snapshots()`가 게임이 스스로 만든 자동 백업을 `{success, snapshots: [{id, created, paths: [{from, to}]}]}`(새것부터, `from`은 스냅샷 안의 경로, `to`는 그것이 되돌아갈 월드 자리)로 준다. 백업 목록에 함께 보이고, 사바쨩 백업으로 보관하거나 바로 복원할 수 있다. 01 §백업). 세 모듈 모두 선언한다. 세 명령 이름은 `[[commands.fields]]` 이름이어야 한다.
- 켜진 서버가 잠근 파일(Windows 공유·잠금 위반 32·33)을 읽지 못하면 백업은 409 `backup_file_locked`(`file`: 파일 이름만)로 실패한다. 반쪽 백업을 만들지 않는다 (`backup::LockedFile`). 백업 뒤 `resume_command`가 실패하면 백업은 남기되 500 `autosave_not_resumed`(`backup`: 백업 이름)로 실패를 알린다(서버를 다시 시작해야 자동 저장이 돌아온다). `remote = false` 명령은 `GET /api/instance/:id/commands`에 `remote:false`로 실리고, 액션 `server.command:<이름>`의 표면이 GUI·CLI뿐이다.
- `[backup]`이 없는 모듈: 수동 백업만 400 `backup_unsupported`이고, 자동 백업(업데이트 전·예약·복원 전)은 건너뛴다. 그래서 업데이트가 막히지 않는다 (`D:src/ipc/handlers/backup.rs:77-87, 141-148`).
- `hook = true`인데 `backup_paths()`가 실패하면 500으로 멈추며 모듈 호출 오류 필드를 유지한다. 경로를 모른 채 안전 백업 없이 업데이트·복원이 진행되지 않게 하려는 것이다 (`backup.rs:115-126`).
- 복원 전 백업은 지금 설정이 가리키는 경로와 함께 복원이 덮을 경로도 담는다(월드 이름을 바꾼 뒤 옛 백업을 복원해도 옛 월드의 진행분이 남는다). 서버 폴더를 옮겨 백업 경로가 지금 서버 폴더·인스턴스 폴더·백업 경로·훅의 `restore_roots` 밖이면 복원은 400 `restore_outside`다.
- 백업 형식(MK16, 0.1.9~): 파일 내용은 `backups/objects/<해시 앞 두 글자>/<sha256>`에 한 번만 두고(deflate), 백업 zip에는 매니페스트 `saba-backup-v2.json`(항목별 원래 절대 경로, 파일마다 상대 경로·해시·크기·수정 시각·내용 파일 크기, 하위 폴더, 링크)과 요약 `saba-backup-v2-summary.json`(`size`·`stored`)만 담는다. 가장 새 백업과 크기·수정 시각이 같은 파일(그 백업 시작 2초 전보다 먼저 바뀐 것)은 내용 파일의 크기가 적어 둔 그대로일 때만 다시 읽지 않는다. 같은 해시의 내용 파일이 있는데 크기가 다르면 망가진 것으로 보고 새로 쓴 것으로 바꾼다. 새로 쓴 내용 파일은 매니페스트를 쓰기 전에 몇 갈래로 나눠 한꺼번에 디스크에 내린다. 백업을 지우거나 정리하면 남은 백업이 쓰지 않는 내용을 치운다. 이때 목록이 아니라 폴더의 백업 이름을 단 파일을 모두 보고(그 이름의 폴더는 백업이 아니다), 열거나 읽지 못하는 백업이 하나라도 있으면 치우지 않는다(열리지 않거나 매니페스트가 없는 것, 대상이 없는 링크는 목록에 `readable:false`로 보인다). 백업 폴더(`backups/`)가 백업할 폴더 안에 있으면(서버 폴더가 인스턴스 폴더이고 경로가 `.` 등) 그 폴더는 담지 않는다. 판 2 zip은 혼자서는 내용이 없으므로 옮길 때는 `backups/` 전체(`objects/` 포함)를 함께 옮겨야 하고, 이름을 바꾼 zip은 백업으로 보지 않아 다음 정리가 그 내용을 지운다. 재사용은 크기·수정 시각만 보므로(rsync 빠른 검사와 같다) 수정 시각을 보존하며 같은 크기로 내용만 바꾸는 도구, 파일 시스템 시계가 데몬과 어긋난 네트워크 드라이브에서는 바뀐 내용을 놓칠 수 있다. Windows에서 백신이 막 쓴 임시 내용 파일을 잡고 있어 이름 바꾸기가 실패하면 그 백업 전체가 실패로 끝난다(성공으로 알리지 않는다). 복원은 내용의 해시·크기를 확인하고 맞지 않으면 실패해 되돌린다. 0.1.8까지의 백업(`saba-backup.json`과 파일을 zip에 담은 판 1)도 그대로 복원한다. 판 2에는 `saba-backup.json`이 없어 0.1.8 데몬은 복원을 거절한다(`backup/mod.rs`). 만들기·지우기·정리·복원은 백업 폴더마다 하나씩 돈다.
- 복원할 때 남기는 복원 전 백업이 오래된 자동 백업을 정리해도, 복원 대상 백업은 지우지 않는다 (`D:src/backup/mod.rs:200-210`, `backup.rs:230-231`). 백업 파일 이름에는 밀리초가 붙고, 같은 밀리초의 백업이 이미 있으면(종류가 달라도) 두 자리 번호를 붙여 이름순이 만든 순서가 된다 (`backup/mod.rs` `unique_name`). 다만 이름은 지역 시각이라 서머타임이 끝나는 시간에는 뒤바뀔 수 있어, 목록·정리의 순서는 만든 시각(판 2는 요약의 시작 시각, 판 1은 이름의 시각과 파일 수정 시각 중 이른 것. 판 1은 모두 0.1.8까지 만든 것이라 옮겼다 되돌려 수정 시각이 새로 찍혀도 판 2보다 새것으로 보지 않는다)으로 정하고 같으면 이름으로 정한다 (D-B12).

#### `[[settings.fields]]` (`SettingFieldToml` `:784-822` → `SettingField` `:393-444`)
| 키 | 타입 | 기본값 |
|---|---|---|
| `name` | String | ✔ |
| `type` | String | `"text"` (`default_field_type`). `text`·`password`·`number`·`file`·`select`·`boolean` 중 하나(`SETTING_TYPES`), 스키마 2는 `list`·`multiline`도(`SETTING_TYPES_V2`). 밖이면 스키마 v1·v2 모듈은 로드에 실패하고, 스키마 이전 모듈은 경고만 남긴다. `list`는 문자열 배열(API·저장·모듈 config 모두): `options`가 있으면 항목마다 그중 하나, `format`·`pattern`은 항목마다, 빈 항목·같은 값 두 번은 `invalid_format`, `default`는 문자열 배열(만들 때 채운다), 빈 문자열은 받지 않는다(빈 목록은 `[]`), 프리셋 값도 로드 때 같은 규칙으로 검사한다. `multiline`은 줄바꿈을 받는 문자열이고 `format`·`pattern`은 값 전체에 쓴다 |
| `label` | String | 없으면 `name`으로 대체 |
| `description` | String | — |
| `required` | bool | — |
| `default` | toml::Value | — (`get_setting_default`가 Int/String/Float/Bool을 파싱). 스키마 2 `list`의 기본값은 PATCH와 같은 검사(`validate_setting_value`: 항목 수·중복·`options`·형식)를 받고, 맞지 않으면 로드 오류 |
| `min`, `max`, `step` | f64 | — |
| `options` | **Vec<String>** | 문자열만 허용 |
| `group` | String | `saba-chan` / `advanced` / 없음(basic) 등. `saba-chan`은 사바쨩 설정(실행 방식·Java 경로·메모리·실행 인자처럼 게임 설정 파일에 없는 값)이라는 선언이다: 설정 PATCH 뒤 `configure`의 `settings`로 넘기지 않는다(MK6). 그 밖의 묶음은 표시용이다 |
| `port` | bool | `false`. 네트워크 포트 값이면 true. 같은 게임 인스턴스를 여럿 만들 때 데몬이 겹치지 않게 옮긴다 |
| `install` | bool | `false`. 서버를 만들 때 고르는 설정(받을 서버 파일이 달라지는 값). GUI 새 서버 화면과 CLI `server add`가 묻고 `POST /api/instances`의 `module_settings`로 보낸다. GUI는 select만 보인다 |
| `arg` | String 또는 Vec<String> | — 이 값이 실행 인자로 들어가는 모양. 공백으로 나눈 토큰, `{value}`는 많아야 한 번(`"-port={value}"`, `"-servername {value}"`, `["-Xmx{value}", "-Xms{value}"]`). `{value}`가 없으면 boolean 플래그(있으면 true). `{value}`뿐인 모양은 로드 오류. 실행 인자 미리보기가 사용자가 친 추가 인자를 이 필드로 되돌릴 때 쓴다(`src/launch_args.rs`). 실제 실행 줄은 여전히 `get_launch_command`가 만든다 |
| `format` | String | — 값 형식: `memory_size`(숫자+K/M/G 한 글자, `4G`·`1024M`), `port`(1~65535 정수), `integer`, `path`(비어 있지 않고 줄바꿈 없음). 모르는 이름은 로드 오류 |
| `pattern` | String | — 값 전체가 맞아야 하는 정규식(잘못된 정규식은 로드 오류). `format`과 함께 PATCH 저장과 미리보기에서 검사한다(`validator::validate_format`) |
| `min_items`, `max_items` | u32 | 스키마 2, `list`만. 항목 수 범위(밖이면 400 `validation_failed`) |
| `max_length` | u32 | 스키마 2, `multiline`만. 글자 수 상한 |
| `restart_required` | bool | 스키마 2, `false`. 켜져 있을 때 바꾸면 다시 켜야 적용된다. PATCH 응답 `restart_required`에 실린다(01) |
| `visible_when` | `{field, equals}` 또는 `{field, in=[...]}` | 스키마 2. 다른 설정 값에 따라 보일지. `field`는 같은 모듈의 설정이고 자기 자신·순환·`list`·`multiline`·`password`·`file` 설정이면 로드 오류, 값은 그 설정 타입에 맞아야 한다. 비교는 타입으로 바꾼 값끼리(`number`는 "25565"와 25565가 같다), 값이 없으면 `default`로. 숨긴 설정은 값을 지우지 않고 `required`를 보지 않는다. `GET /api/instance/:id`의 `hidden`(CLI)과 GUI의 같은 계산이 쓴다(`validator::hidden_fields`) |

- `fields`가 비어 있으면 `settings = None`이 된다.
- 옮기기(06 §8.1): 문자열로 저장된 값의 설정이 모듈 업데이트로 `list`가 되면, 데몬이 인스턴스를 읽을 때(시작, 모듈 다시 읽기) 쉼표로 나누고 앞뒤 공백·빈 항목을 버려 배열로 바꾼다(`validator::migrate_list_values`, `Supervisor::migrate_list_settings`). 파일에는 다음 저장 때 쓴다.
- `players_detail`·`stop_call_limit`은 데몬이 호출마다 정해 넣는 키다(`RESERVED_CALL_KEYS`). 같은 이름의 설정은 로드하되 경고하고, 그 값은 모듈 config에 싣지 않는다(`insert_instance_fields`).
- 데몬이 직접 의미를 부여하는 setting 이름은 `rest_port`(기본 8212), `rest_host`(기본 127.0.0.1) (`:259-267`), 시작 경로를 고르는 `managed_start`(`mod.rs:1690-1700`), 자동 재시작의 `auto_restart`·`restart_on_unknown_exit`(`mod.rs:2285-2286`), 세션 로그 보관 수 `log_sessions_keep`(`mod.rs:1628-1631`), 정지 대기 시간(초) `stop_timeout`(모듈 `[config] stop_timeout`보다 앞선다, 1~3600, `supervisor::stop_timeout_of`, MK14. 데몬이 기다리는 managed 정지에 쓴다. non-managed 정지는 모듈 `stop()` 호출 하나이고, 그 호출의 시간 제한이 `stop_timeout + 30`초다(`supervisor::module_stop_limit`))이다. 이 값들도 module_settings라서 모듈 함수 config에 펼쳐 들어간다. 모듈 `stop()`의 config에는 데몬이 실제로 쓸 `stop_timeout`(인스턴스 값, 모듈 값, 기본 30 순)과 그 호출의 시간 제한 `stop_call_limit`(초, non-managed면 `stop_timeout + 30`, managed면 120)이 늘 실린다. `stop_call_limit`이 없으면(0.1.8 데몬) 모듈은 120초 안에 끝나게 기다린다. non-managed 정지(`managed:false`)의 모듈은 이 값만큼 기다리고 자기 상한을 두지 않는다. managed 정지(`managed:true`)의 모듈은 저장·종료 요청만 보내고 바로 `success`를 돌려준다(기다림과 강제 종료는 데몬이 한다).
- 실제 모듈에서 쓰이는 type 값: `text`, `number`, `boolean`, `select`, `password`, `file` (0.1.9 공식 모듈은 스키마 1).

#### `[[commands.fields]]` (`CommandFieldToml` `:928-949` → `CommandField` `:352-370`)
| 키 | 타입 | 비고 |
|---|---|---|
| `name` | String ✔ | |
| `label` | String | 없으면 `name` |
| `description` | String | |
| `method` | String | `console`(템플릿을 채워 콘솔 stdin으로. 예전 이름 `rcon`은 읽을 때 `console`로 바뀐다, `D:src/supervisor/module_loader.rs:1067`) / `rest` / `module`(lifecycle `command()`, 예전 이름 `dual`·`both`도 받음) / 생략(=`module`). 해석은 데몬 `crate::commands`가 한다 (`D:src/commands/mod.rs:167-191`) |
| `http_method` | String | REST 전용. 기본 `GET` |
| `endpoint_template` | String | REST 전용. 없으면 `/<name>` |
| `console_template` | String | 예: `"kick {player}"`. 예전 이름 `rcon_template`도 별칭으로 읽는다 (`D:src/supervisor/module_loader.rs:362-363, 941`). 인자에 `\r`·`\n`·NUL이 있으면 400 `invalid_argument`로 거부하고, stdin 전송(`ManagedProcess::send_command`)도 여러 줄 명령을 거부한다. 값 하나가 콘솔 명령 두 줄이 되지 않게 한다. 값은 자기 자리만 채운다(`commands::console_place_problem`): 뒤에 템플릿 글자가 더 있는 자리의 값은 공백이 있으면 400 `invalid_argument`(`must be one word`), 따옴표로 감싼 자리(`"{player}"`)는 공백을 받되 따옴표가 있으면 400(`must not contain quotes`). 마지막 자리는 남은 말을 모두 받는다. 템플릿은 한 번만 채운다(`commands::render_template`): 값 안의 `{key}`는 다시 채우지 않는다. 중간 자리에 여러 단어가 필요하면 템플릿에서 따옴표로 감싼다 |
| `risk` | String | `safe` / `caution` / `danger`, 없으면 caution |
| `inputs` | Vec<CommandInput> | |
| `remote` | bool | 기본 true. false면 heartbeat 명령 목록에 싣지 않고 디스코드·챗봇 채팅 명령에서 없는 명령(`UNKNOWN_COMMAND`)이다. 데몬 기능이 같은 일을 하는 명령용(접속자 목록은 채팅 `players`) |
| `reply_pattern` | String | 콘솔 답에서 값을 뽑는 정규식(이름 있는 그룹). 맞고 모듈 locales `commands.<이름>.done`이 있으면 채팅 답이 그 문구에 그룹 값을 넣는다 (`{{그룹}}`) |

`command_template` 별칭은 없어졌다.

`[commands] failure_patterns`: 콘솔 답이 이 정규식과 맞으면 채팅 답이 실패로 알린다(`reason` 그룹이 있으면 그것이 이유). 정규식이 틀리면 로드 실패다.

채팅 명령 답(디스코드·`POST /api/chat/command`, `D:src/chat/mod.rs`)은 성공·실패와 짧은 이유만 쓴다. 성공은 `✅ **서버** · <명령 이름>`(이름은 모듈 locales `commands.<이름>.label` → `label`)이고 콘솔 답 원문은 싣지 않는다. 모듈·익스텐션의 `message`와 REST 결과 값은 아래 줄에 붙인다. 틀린 인자는 데몬 검사 결과의 `rule`(`missing`·`not_number`·`out_of_range`·`not_option`·`not_boolean`·`line_break`·`one_word`·`quote`·`too_many`)과 `params`(`value`·`min`·`max`·`options`)로 응답 언어 문구(`bot.json` `command.invalid.*`)를 쓴다. `POST /api/instance/:id/command`의 400 본문에도 `rule`·`params`가 실린다. 입력이 없는 명령에 말을 붙이면 400 `invalid_argument`(`too_many`)이고, `boolean` 칸은 `true/false/on/off/yes/no/1/0`만 받는다. 숫자 칸은 NaN·무한대를 받지 않는다. `[status] players`를 선언한 모듈은 채팅 `players`(모듈 별명 포함)가 접속자 조회(`GET /api/instance/:id/players`)다.

`[commands] hook = true`면 lifecycle `instance_commands()`가 인스턴스마다 명령을 더 준다(설치된 플러그인 명령처럼 서버 폴더에 따라 달라지는 것). 결과는 `{success, commands:[명령 필드와 같은 모양 + group?], components?}`이다. 데몬(`crate::commands::InstanceCommands`)은 module.toml 명령 뒤에 붙여 같은 방식으로 해석하고, module.toml의 이름·별명과 겹치는 것과 이름에 공백이 있는 것은 뺀다. 명령 실행은 module.toml에서 못 찾은 이름일 때만 이 목록을 보며 60초 동안 기억한 값을 쓰고, `GET /api/instance/:id/commands`는 매번 새로 읽는다(`D:src/ipc/handlers/command.rs` `instance_commands`). `fields` 없이 `hook`만 있어도 된다.

`[[commands.fields.inputs]]` (`CommandInputToml` `:953-972`)의 허용 키: `name`✔, `label`, `type`, `required`, `placeholder`, `default`, `min`, `max`, `options`. `type`은 `text`·`password`·`number`·`select`·`boolean` 중 하나다(`COMMAND_INPUT_TYPES`, 어기면 설정 `type`과 같이 처리한다). 데몬이 입력을 묶고 검사할 때 이 값을 쓴다 (`commands/mod.rs:196-`).

#### `[[syntax_highlight.rules]]` (`HighlightRuleToml` `:999-1015`)
`name`✔, `pattern`✔(regex, `(?P<hl>…)` 선택), `color`✔(시맨틱 토큰), `background`, `bold`=false, `italic`=false, `underline`=false, `priority`=0. `/api/modules`로 GUI에 그대로 전달되고, 데몬은 Python 전용 정규식 문법에 경고만 남긴다 (`:1017-1035`).
- 데몬 토큰화(`src/supervisor/highlight.rs`): `GET /api/instance/:id/console?spans=1`이면 줄마다 `spans: [[start, end, token]]`를 붙인다. 오프셋은 `content`의 UTF-16 코드 단위, 오름차순·겹침 없음, 칠할 곳만 들어 있다. 디스크 폴백 줄에도 붙고, 4096바이트를 넘는 줄은 앞부분만 본다.
  - 토큰은 16개로 고정: `error, warn, info, debug, success, string, number, keyword, timestamp, player, command, selector, ip, uuid, path, dim` (`TOKENS`). `warning`→`warn`, `trace`→`debug`, `comment`→`dim`으로 바꾸고, 색 값(`#hex`)이나 모르는 이름의 규칙은 경고를 남기고 뺀다. `bold`·`italic`·`background`·`underline`은 spans에 싣지 않는다(굵기·색은 클라이언트 테마가 토큰별로 정한다).
  - 엔진은 Rust `regex`(선형 시간). lookaround(`(?=`, `(?!`, `(?<=`, `(?<!`)는 컴파일되지 않아 그 규칙은 빠진다. 모듈 적합성 검사가 이를 막는다(saba-chan-modules `conformance`).
  - 겹치면 priority가 큰 규칙, 같으면 먼저 선언된 규칙이 이긴다. 모듈 규칙 뒤에 공통 규칙(스택 프레임·`... N more`→dim, `Caused by:`→error, IPv4[:포트]→ip, UUID→uuid, 따옴표 문자열→string, 퍼센트→number)이 남은 칸만 칠한다.
  - 컴파일 결과는 모듈 이름별로 캐시하고, 규칙 지문이 바뀌면(모듈 업데이트) 다시 만든다.

#### `[install]` (`InstallSectionToml` `:829-851` → `ModuleInstallConfig` `:854-872`)
| 키 | 타입 | 기본값 |
|---|---|---|
| `method` | String | `"manual"` (`steamcmd` / `download` / `manual`) |
| `app_id` | u32 | — |
| `anonymous` | bool | `true` |
| `install_subdir` | String | — |
| `platform` | String | — |
| `download_url` | String | — |
| `beta` | String | — |
| `requires_extensions` | Vec<String> | `[]`. 인스턴스 생성·non-managed 시작·`/api/module/:name/install` 때 필수 익스텐션 검증에 사용 (`instance.rs:233-252`, `mod.rs:572-629`, `managed.rs:526-`) |

zomboid의 `beta_branch` 키는 지워졌다. lifecycle에는 `use_beta`를 보고 베타를 받는 분기가 남아 있지만(`M:zomboid/lifecycle.py:1342`), module.toml에 그 설정이 없고 데몬은 `install_server`에 설정을 최상위로 펼치지 않아(`settings{}`로 준다) 지금은 쓰이지 않는다.

#### `[extensions.<익스텐션 id>]`
익스텐션이 읽는 모듈별 설정. 데몬은 내용을 보지 않고 `ModuleMetadata.extensions`에 그대로 넘긴다 (`:618-622, 1139-1145`). 예전 `[docker]` 섹션(`ContainerSectionToml`)은 Docker 익스텐션과 함께 지워졌다.

`[extensions.saba-core]`는 데몬이 읽는 예약 표다(R39, 0.1.9). 이 이름은 데몬 몫이라 `saba-core`라는 id의 익스텐션은 설치·인식하지 않는다(`extension::is_valid_extension_id`). 0.1.8 로더도 `[extensions.*]`는 내용을 보지 않으므로 이 키가 있는 모듈을 그대로 로드한다.
이 표는 버전 사이를 잇는 자리라, 이 데몬이 모르는 키(더 새 데몬의 키)가 있어도 모듈을 거부하지 않고 경고만 남긴다(`DAEMON_TABLE_KEYS`). 아는 키의 값이 틀리면 그 항목만 버리고 경고한다.
- `ready_patterns` (Vec<String>, 정규식): 콘솔에 이 중 하나가 찍히면 서버가 준비된 것이다. 데몬이 콘솔을 읽는 시작(managed)에만 쓰인다. 켜지는 중에 들어온 정지는 이 문구가 찍힌 뒤 `stop_command`를 보낸다(§3.4). 정규식이 아니거나 목록이 아니면 그 항목만 버리고 모듈 경고(`warnings`)를 남긴다(로드는 실패하지 않는다).
- `ready_timeout` (u64, 초, 기본 600, 최대 900): 시작 뒤 이 시간 안에 준비 문구가 없으면 `ready_timeout`으로 닫는다. 0이거나 숫자가 아니면 경고를 남기고 기본값을 쓴다. 900보다 크면 경고를 남기고 900을 쓴다(디스코드가 시작을 시킨 채널을 15분 기억한다, 05 §4.4).
- `player_join_patterns`·`player_leave_patterns` (Vec<String>, 정규식, MK8): 콘솔 줄이 맞으면 그 플레이어가 들어오거나 나간 것이다. 각 정규식에 이름 그룹 `(?P<name>…)`이 있어야 하고, 없거나 정규식이 아니면 그 항목만 버리고 경고한다.

시작 한 번(자동 재시작 포함)마다 `instance.ready {instanceId, instance, ok, code?}` 이벤트가 한 번 나온다(`supervisor/ready.rs`, 01 §`GET /api/events`). 패턴이 없거나 콘솔을 읽지 않는 시작은 시작이 성공한 때 바로 `ok:true`다. 준비되기 전에 끝나면 `ok:false`와 `code`: `start_failed`, `stopped_while_starting`, `ready_timeout`. 데몬이 띄운 프로세스가 준비 문구 전에 끝나 이미 치워졌으면 `start_failed`다. 감독자가 시작을 시도하지 않은 요청(사전 점검에 막힘, 다른 작업 중, 이미 켜짐, 시작 중)에는 알림이 없다. relay로도 올린다(01 §relay 이벤트).

접속·퇴장 문구가 있으면 managed 시작마다 프로세스가 끝날 때까지 콘솔을 보고, 맞는 줄마다 `instance.player {instanceId, instance, player, change:"join"|"leave"}`를 낸다(`supervisor/player_events.rs`). 시스템 줄은 보지 않고, `name` 그룹만 싣는다(64자에서 자르고 제어 문자를 뺀다). non-managed 시작에는 없고, relay로 올리지 않는다. 1분 접속자 수 갱신(`supervisor/players.rs`)과는 따로다.

#### `[credential_map]` (`HashMap<String,String>`)
데몬 키(`rest_password` / `rest_username`) → 게임 설정 키. 사용처는 managed start 후 동기화(`mod.rs:1554-1619`), import_settings 결과 반영(`D:src/instance/mod.rs:172-206`), 설정 PATCH 때 역동기화(`instance.rs:1162-1189`)다. **non-managed `start_server`에서는 동기화하지 않는다**.

#### `[aliases]`
`module_aliases: Vec<String>`, `[aliases.commands.<cmd>] = { aliases = [...], description = "..." }`.

#### `[errors]`
오류 코드 → 사용자 문구. 메타데이터 `errors`에 실리고(`:94-97`), `POST /api/instance/:id/command`가 실패 응답에 `error_message`로 붙인다 (`D:src/ipc/handlers/command.rs:11-15`). 문구는 GUI 언어의 모듈 locales `errors.<code>`가 있으면 그것을, 없으면 module.toml 값을 쓴다 (`module_errors`).


#### `[[presets]]` (`ModulePreset`)
서버를 만들 때 고르는 추천 설정 묶음(예: "친구용 소규모", "하드코어"). `id`(소문자·숫자·`-`·`_`, 모듈 안에서 겹치지 않음), `label?`, `description?`, `[presets.settings]`(`[[settings.fields]]` 이름 → 값). 값은 그 설정의 규칙(형식·범위·select 선택지)을 따라야 하고, `password`·`file`·`path` 형식 설정은 넣지 못한다. 어기면 스키마 v1 모듈은 로드에 실패한다. 표시 문구는 모듈 locales의 `presets.<id>.label`·`description`이 덮는다. `GET /api/modules`의 각 모듈 `presets`, heartbeat `modules[].presets`(`{id, label}`만)에 실리고, `POST /api/instances`의 `preset`으로 고른다(01 §`POST /api/instances`). CLI는 `server add --preset <id>`(주지 않으면 새 서버일 때 묻는다), `module info`가 id를 보여 준다.

#### `[[queries]]` (`QueryToml` → `ModuleQuery`)
읽기 전용 조회(예: 접속자 목록). 액션 `server.query:<name>`(`kind=query`, risk `safe`, 모든 표면)이 되어 GUI·CLI·디스코드·챗봇 도구로 쓰인다(01 §액션 카탈로그). 키: `name`(소문자·숫자·`-`·`_`, 모듈 안에서 겹치지 않음), `label?`, `description?`, `function`(lifecycle.py `FUNCTIONS`의 함수 이름, Python 식별자), `inputs`(`[[commands.fields]]`의 입력과 같은 모양: `name`·`type`·`required`·`label`·`placeholder`·`default`·`min`·`max`·`options`). 로드 때 검사: 이름 형식·중복, `function`이 식별자인지, 데몬이 정해진 뜻으로 부르며 상태를 바꾸는 함수(`start`·`stop`·`command`·`configure`·`get_launch_command`·`import_settings`·`install_server`·`reset_properties`·`reset_server`·`accept_agreements`·`functions`)가 아닌지, 입력 이름 형식·중복, 입력 `type`이 `text`(기본)·`number`·`select`인지, `select`에 `options`가 있는지, `min ≤ max`. 어기면 스키마 v1 모듈은 로드에 실패하고, `schema_version`이 없는 모듈은 경고만 남기되 `[[queries]]`를 모두 끈다. 표시 문구는 모듈 locales의 `queries.<name>.label`·`description`이 덮는다(없으면 `label`·`description`, 그것도 없으면 `name`). `GET /api/modules`의 각 모듈 `queries`에 실린다.

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

#### `[permissions]` (`ModulePermissions`)
모듈 Python 코드가 쓰는 위험한 능력의 선언(M12). 모두 bool, 기본 false. 선언은 실행을 막거나 풀지 않는다: 격리(§2.1)는 신뢰 표시로만 정해진다. 검사기가 코드에서 찾은 능력 중 선언하지 않은 것을 알린다.

| 키 | 뜻 |
|---|---|
| `network` | 네트워크 접속(socket, urllib, requests 등). 데몬 API 호출(`daemon_rcon`)은 들지 않는다 |
| `spawn_process` | 다른 프로그램 실행·프로세스 신호(subprocess, os.system, os.kill 등) |
| `native_code` | 네이티브 코드(ctypes, cffi, winreg) |
| `dynamic_code` | 실행 중에 만든 코드 실행(eval, exec, compile, 동적 `__import__`, pickle, importlib) |
| `env_secrets` | 환경 변수 전체 읽기(`dict(os.environ)` 등) |
| `fs_outside_instance` | 서버 작업 폴더 밖 파일 쓰기(정적 검사로 찾지 못한다: 선언만) |

- 검사: `GET /api/module/:name/audit` → `{success, declared, findings, undeclared}`(실패 500 `audit_failed`), CLI 도구 `saba-core module audit <모듈 폴더>...`. 검사기는 데몬이 modules 폴더에 쓰는 `saba_audit.py`(`module_audit::SCRIPT`)다. 모르는 권한 이름은 `declares`가 None을 돌려 선언 없음으로 세지 않는다.
- 새 권한은 `ModulePermissions`, `PERMISSIONS`, `saba_audit.py` 표에 한 줄씩 더한다.

```toml
[permissions]
network = true
spawn_process = true
```

#### `[status]` (`StatusSection`)
켜진 서버의 상태를 더 알려 주는 함수(Q-5). 지금 키는 `players` 하나다: 접속자 수를 돌려주는 lifecycle 함수 이름(`[[queries]]`의 `function`과 같은 검사: Python 식별자, 상태를 바꾸는 함수가 아님. 어기면 스키마 v1은 로드 실패, 스키마 이전 모듈은 경고만 남기고 부르지 않는다). 데몬은 켜진(트래커에 PID가 있는) 서버마다 1분에 한 번, 10초 제한으로 부르고 마지막 값을 기억한다(`supervisor/players.rs`). 실패하거나 서버가 꺼지면 값을 지운다. 값은 `GET /api/servers`의 `players`와 relay 하트비트 `servers[].players`(05 §4)로 나간다. 같은 프로세스에서 5분 넘게 계속 실패하면(시간 초과 포함) 마지막 실패 `message`가 `GET /api/servers`의 `unresponsive`로 나가고 GUI에 응답 없음 경고로 보인다(01 §ServerInfo). 수를 알 수 없는 설정이면 실패 대신 `{"success": true}`를 돌려줘야 경고가 뜨지 않는다. 같은 함수가 플레이어 목록(`GET /api/instance/:id/players`, 액션 `server.players`)도 맡는다: 데몬은 그때만 config에 `players_detail: true`를 싣고, 모듈은 그때 `players:[{name}]`를 더해 돌려준다. 이름을 콘솔 명령으로만 얻는 모듈은 이 키가 있을 때만 그 명령을 보내서 1분 갱신이 콘솔을 어지럽히지 않게 한다. 함수 계약은 §2.3.

```toml
[status]
players = "player_count"
```
---

## 2. lifecycle.py 프로토콜

### 2.1 실행 방식 (`D:src/plugin/mod.rs:266-463`)

- **명령줄**: `<python_exe> <module_path>/lifecycle.py <function_name>`
  - Python 실행파일은 `python_env::get_python_path()`가 정한다(데몬이 관리하는 venv).
  - cwd는 지정하지 않으므로 **데몬의 cwd를 상속**한다.
- **stdin**: config 전체를 JSON 한 덩어리로 쓰고 바로 닫는다(`shutdown`).
- **stdout**: 최대 4 MiB를 모아 JSON으로 파싱한다. 전체가 JSON이 아니면 **마지막 비어 있지 않은 줄**을 JSON으로 본다(`parse_plugin_output`). JSON을 읽지 못하면 `module_bad_output`, 상한을 넘으면 `module_output_too_large`다.
- **stderr**: 줄 단위로 스트리밍된다.
  - `PROGRESS:{json}` 형식의 줄은 `ExtensionProgress {percent, message, step, total, label}`로 파싱해 콜백에 넘긴다 (`:69-79, 391-408`). 설치 진행률 표시에 쓰인다.
  - `LOOPBACK:` 줄은 중계로 처리한다. 나머지는 계속 읽어 비우고 마지막 200줄, 합계 64 KiB를 보존한다. 앞의 줄은 `debug` 로그로만 남는다.
- **종료 코드**: 0이 아니면 stdout의 JSON과 관계없이 `module_crashed`다. 공용 `run_lifecycle`은 모르는 함수·예외·직렬화 실패도 JSON으로 답하고 종료 코드 0을 지킨다.
- **타임아웃**: 기본 `DEFAULT_PLUGIN_TIMEOUT_SECS = 120`이다. 기한은 자리 대기, 내부 `functions` 조회, Python 환경 준비, 실행과 출력 수집 전체를 포함한다. 기한·출력 상한에 걸리면 프로세스 트리를 끝내고 읽기 작업을 멈춘다. 동시 실행 자리와 단계별 오류는 [01 §7.1](01-daemon-core.md#71-srcpluginmodrs-python-모듈익스텐션-실행기-단발-프로세스)을 따른다.
  - `install_server`는 어디서 부르든(프로비저닝, 업데이트, `POST /api/module/:name/install` → `Supervisor::install_server`) 1800초(`plugin::INSTALL_TIMEOUT_SECS`)를 쓴다.
  - 프로비저닝 설치는 끝날 때까지 인스턴스 작업 잠금(`InstanceLock`)을 쥔다: 그동안 시작·업데이트·삭제·복원은 `instance_busy`다.
- **확인되지 않은 구성요소**: 신뢰 표시가 `verified`가 아닌 모듈·익스텐션의 Python은 격리를 요구한다(`D:src/sandbox/`). Windows는 AppContainer를 쓴다. Linux·macOS는 격리가 없으므로 기본으로 실행하지 않는다. 이때 응답은 `{success:false, action_required:"sandbox_unavailable", error_code:"isolation_unsupported", component, message}`다. 백신 검사는 하지 않으며 예전 `scan-verdicts.json`도 읽지 않는다. 신뢰 키는 모듈 `module-<폴더 이름>`, 익스텐션 `ext-<manifest id>`다.
  - 비격리 동의는 기존 `POST /api/sandbox/unisolated/:component`와 `config.json`의 `sandbox.unisolated`를 쓴다. 이 PC의 현재 파일 지문(상대 경로와 sha256)에만 유효하다. `__pycache__` 안의 실행 코드도 예외가 아니므로 모든 실파일을 포함한다. 그 폴더가 있던 옛 동의는 다시 받아야 할 수 있다. 데몬의 정상 Python 캐시는 구성요소 밖에 둔다. 파일이 바뀌면 다시 동의해야 한다. GUI·CLI는 기존 동의 안내와 `sandbox.isolation_unsupported.message`를 쓴다. 게임 서버 프로세스의 격리 정책은 바꾸지 않는다.
  - Python을 그 구성요소만 쓰는 AppContainer `saba-chan.untrusted.<키 sha256 앞 16자>`(능력 `internetClient`만. 루프백 예외를 주지 않으므로 같은 PC의 데몬 API·로컬 프록시에는 닿지 못한다) 안에서 Job Object에 넣어 띄운다(`sandbox/appcontainer.rs`).
    - 안에서는 포트를 열 수 없고 LAN·루프백(데몬 API, 로컬 게임 서버 REST)에 닿지 않는다.
    - 폴더 권한: 읽기는 venv와 그 기반 Python, 그 구성요소 폴더, `python-shared`, `extensions_dir`, `python-guard`, 그리고 데몬이 아는 Java 런타임 설치 폴더(`sandbox::runtime_homes`: 데몬 환경의 `JAVA_HOME`, 데몬 PATH의 모든 `java`, 알려진 설치 위치의 JDK·JRE(`SABA_JAVA_RUNTIMES`와 같은 목록). 링크를 따라간 실제 실행 파일이 `<폴더>/bin/java(w)`이고 그 폴더에 `release` 파일이 있을 때만. 모듈은 `java -version`으로 런타임을 확인하는데 AppContainer는 Program Files 밖 폴더를 읽지 못한다). 이 런타임 폴더는 권한을 주지 못하면(관리자가 설치한 폴더 등) 경고만 하고 빼고 띄운다. 모든 구성요소가 함께 읽는 이 폴더들(venv와 기반 Python, `python-shared`, `extensions_dir`과 그 부모의 목록, `python-guard`, Java 런타임)은 프로필 SID가 아니라 ALL APPLICATION PACKAGES(`S-1-15-2-1`)에 한 번만 준다(`Grants::shared`). 프로필마다 주면 그 폴더 DACL에 구성요소 수만큼 ACE가 쌓여 64KB 한도에 닿고, 그 뒤로는 모든 격리 실행이 `build ACL … os error 87`로 멈춘다. 함께 쓰는 권한이라 런타임 기록(`sandbox-grants.json`)에는 남기지 않고 거두지도 않는다. 사용자가 정한 `java_path`(인스턴스 `user_settings`에 있는 것, 아래)는 설정을 저장할 때 같은 판정으로 그 구성요소 프로필에 읽기를 준다(`sandbox::open_user_runtime`. 실행 때 받는 config의 값은 모듈 기본값과 구별할 수 없어 쓰지 않는다). `java_path`는 파일 시스템에 묻기 전에 로컬 드라이브의 절대 경로이고 `..`가 없는지 본다(UNC·장치 경로는 거부, `\\?\C:\` 같은 로컬 드라이브 경로는 받는다, `sandbox::plain_local_path`). 읽기/쓰기는 `<data_dir>/pycache-sandbox/<꼬리표>`, `<data_dir>/sandbox-temp/<꼬리표>`(꼬리표는 프로필 이름과 같은 16자), 그리고 모듈이면 config의 `instance_dir`·`working_dir`·`install_dir`와 `instance_data_dirs`(이 폴더 키들은 데몬만 정하고 모듈 설정의 같은 키는 config에 싣지 않는다. `import_settings`로 아무 경로나 넣을 수 있어서다. `instance_data_dirs`는 데몬이 인스턴스를 가리키는 호출(처음 설치하는 `install_server` 포함)마다 모듈 설정 값 중 그 인스턴스 폴더 안을 가리키는 경로를 인스턴스 폴더 바로 아래 항목 단위로 모은 목록이다(`Supervisor::insert_instance_fields`): 좀보이드 `cache_dir`처럼 서버 폴더 옆에 둔 모듈 데이터를 쓸 수 있게 한다. 인스턴스 폴더 자체, `..`가 든 경로, 데몬 항목 `instance::DAEMON_ENTRIES`(`instance.json`·`settings.json`·`managed.pid`·`run`·`logs`·`backups`·`rcon.json`. 대소문자(유니코드)와 끝의 점·공백을 가리지 않고, `~`·`:`가 든 이름도 같은 항목으로 본다, `is_daemon_entry`)와 이미 있는 파일은 넣지 않는다. 익스텐션은 이것들을 읽기만 하고 `<data_dir>/extension-data/<이름>`에 쓴다, 03 문서), 그리고 이 실행이 import한 익스텐션이 쓸 `<data_dir>/sandbox-data/<꼬리표>/extension-data`(`SABA_EXTENSION_DATA_ROOT`, 아래). `extensions_dir`의 부모와 인스턴스 폴더 키 각각의 바로 위 폴더에는 목록만 준다(`os.makedirs(..., exist_ok=True)`가 위 폴더부터 확인한다). 모듈이 쓸 인스턴스 폴더가 아직 없으면 데몬이 먼저 만든다. 인스턴스 폴더 권한은 하위로 상속되므로, 비밀이 든 데몬 항목(`instance.json`·`settings.json`·`rcon.json`)과 `backups/`는 상속을 끊은 보호 DACL(데몬 사용자·SYSTEM·Administrators)로 둔다: 비밀 파일은 저장할 때마다(`saba_atomic_file::Mode::Secret`), 백업 폴더는 백업을 만들 때, 둘 다 격리 실행에 권한을 주기 직전에도(`sandbox::protect_daemon_entries`) 끊는다. 데이터 폴더 자체와 그 위, 드라이브 루트, 사용자 폴더 루트에는 주지 않는다. 목록 권한도 드라이브 루트와 사용자 폴더와 그 위에는 주지 않는다. 표시를 바꾸려면 그 폴더의 WRITE_OWNER가 있어야 하는데, 사용자가 `C:\` 바로 아래 만든 폴더(`C:\Games` 등)는 상속받은 "수정" 권한뿐이라 소유자에게도 없다. 이때는 소유자가 데몬 사용자일 때만, 소유자가 원래 가진 DACL 고치기 권한으로 그 폴더에 OWNER RIGHTS(`S-1-3-4`) `READ_CONTROL`·`WRITE_DAC`·`WRITE_OWNER` ACE를 잠깐 더해 핸들을 열고, 표시를 붙이기 전에 원래 DACL로 되돌린다(ACE는 남지 않는다). 그래서 사용자가 서버 폴더로 고른 `C:\` 바로 아래 폴더도 그 구성요소가 실제로 쓸 수 있게 된다(쓰기를 준 다른 폴더와 같다). 표시를 지원하지 않는 볼륨이라 붙이지 못하면 경고만 남기고 계속한다(그 폴더 쓰기가 안 될 수 있다). 권한은 그 프로필 SID에 대한 폴더 ACL로 남는다(한 번 주면 다시 쓰지 않는다. 서버 실행 파일의 낮은 수준은 아래 "서버 폴더의 낮은 수준 표시 걷기"에서 서버를 띄우기 전에 덮인다). 런타임 폴더 읽기 권한(데몬 런타임, 사용자 `java_path`) 중 그 프로필 SID의 명시 ACE로 열린 것(지금 더한 것, 예전 데몬이 더해 둔 것. 데몬이 더하는 권한·상속 표시와 똑같은 ACE만 데몬 것으로 보고, 관리자가 따로 준 다른 모양의 ACE는 기록하지 않는다. 위 폴더에서 상속받았거나 ALL APPLICATION PACKAGES로 이미 읽을 수 있는 폴더는 이 폴더에서 거둘 것이 없어 빼고, 판정은 격리 실행마다 `[Sandbox] runtime …` 로그 한 줄)은 `<data_dir>/sandbox-grants.json`(프로필 → 폴더)에 기록하고, 데몬이 시작할 때 지금 데몬이 아는 런타임도 아니고 그 프로필 모듈의 인스턴스가 쓰는 사용자 `java_path`도 아닌 것은 거둔다(`sandbox::reconcile_runtimes`, 그 SID의 명시 ACE를 뺀다). 폴더가 남아 있는데 거두지 못한 것은 기록에 두고 다음 시작 때 다시 시도한다. 폴더 ACL 고치기는 데몬 안에서 한 번에 하나씩 한다(같은 폴더에 동시에 쓰면 한쪽 ACE가 사라진다). AppContainer 프로세스는 낮은 무결성 수준으로 돌고, 표시가 없는 보통 폴더는 중간 수준(낮은 수준의 쓰기 금지)이라 DACL만으로는 쓰지 못한다. 그래서 읽기/쓰기를 준 폴더에는 하위까지 상속되는 낮은 수준 표시(`ML;OICI;NW;;;LW`)도 붙인다(이미 있으면 다시 쓰지 않는다). 그 폴더는 같은 사용자의 다른 낮은 수준 프로세스도 DACL이 허용하는 만큼 쓸 수 있다. 그 폴더의 서버 실행 파일은 데몬이 중간 수준으로 띄우므로, 낮은 수준 프로세스가 바꿔 둔 파일이 중간 수준으로 실행될 수 있다. 쓰기를 줄 경로나 그 위 폴더가 심볼릭 링크·정션이면 쓰기를 주지 않고(읽기는 준다), ACL과 표시는 경로를 다시 찾지 않고 연 핸들에 쓴다(핸들의 최종 경로가 확인한 경로와 다르거나 링크류 재분석 지점이면 실행을 멈춘다. 압축(WOF)·중복 제거처럼 이름 대리 표시가 없는 재분석 지점은 같은 파일이라 보통처럼 다룬다). 프로필이 구성요소마다 달라서 한 구성요소가 받은 권한을 다른 구성요소가 쓰지 못한다. 예전의 공용 프로필 `saba-chan.untrusted`는 더 쓰지 않는다. 제거기는 데몬을 끈 뒤 설치 폴더를 지우기 전에 `saba-core sandbox reset`을 부른다(`sandbox::reset`): 인스턴스 기록의 인스턴스 폴더·작업 폴더·실행 파일 폴더에서 낮은 수준 표시를 덮고, 이 사용자의 AppContainer 매핑(`HKCU\Software\Classes\Local Settings\…\AppContainer\Mappings`의 `Moniker`) 중 사바쨩 프로필(`saba-chan.untrusted.*`와 예전 공용 프로필, `is_saba_profile`)마다 그 폴더들과 바로 위 폴더, 데몬 Python의 기반 설치, `sandbox-grants.json`에 기록된 런타임 폴더에서 그 SID의 명시 ACE를 거둔 뒤(그 ACE가 있는 폴더만 고친다) `DeleteAppContainerProfile`로 프로필을 지운다(프로필 폴더 `%LOCALAPPDATA%\Packages\<이름>`과 매핑도 지워진다). 인스턴스를 남겨도 그 폴더의 권한·표시는 거둔다(다시 설치하면 격리 실행 때 다시 준다). 거두지 못한 것은 `FAIL  <경로>: <이유>` 줄로 찍고 종료 코드 1이며, 제거기는 그 줄을 남은 항목으로 보인다.
    - 환경: `SABA_TOKEN_PATH`를 뺀다. `PYTHONPYCACHEPREFIX`는 `pycache-sandbox/<꼬리표>`, `TEMP`·`TMP`는 `sandbox-temp/<꼬리표>`.
    - 시간 초과면 Job째 끝낸다.
  - 격리를 준비하지 못하면(오래된 Windows, ACL 오류 등) 격리 없이 돌리지 않고 `{success:false, action_required:"sandbox_unavailable", message, detail, component}`를 돌려준다. 사용자가 `POST /api/sandbox/unisolated/:component`로 동의하면 그 구성요소는 격리 없이 돈다. 동의는 `config.json` `sandbox.unisolated`에 파일 지문(상대 경로와 sha256)으로 남아, 파일이 하나라도 바뀌면 다시 묻는다.
  - Linux·macOS에서는 하지 않는다.
  - 격리되는 것은 lifecycle Python 실행(설치·설정·상태 스크립트)뿐이다. **게임 서버 프로세스는 격리되지 않는다**: 데몬이 사용자 권한(중간 무결성)으로 띄우고, 확인되지 않은 모듈이면 띄울 대상의 위치만 본다(아래 "확인되지 않은 모듈의 실행 대상"). 백신 검사도 하지 않으므로, 확인되지 않은 모듈이 설치 중 서버 폴더에 둔 실행 파일은 그대로 사용자 권한으로 돈다. 서버 폴더를 격리 실행의 쓰기 범위에서 빼는 구조 변경은 0.1.8 과제다.
- **데몬 락**: lifecycle 호출은 데몬 락 밖에서 실행한다. 보통 `PluginCall`로 락 안에서 값을 챙기고 락을 놓은 뒤 부른다 (`:161-175`, C12).
- **환경변수** (`:18-38, 284-349`):
  - `PYTHONIOENCODING=utf-8`, `PYTHONUNBUFFERED=1`
  - `PYTHONPATH` = `<python-guard>` + `<extensions_dir의 부모>` + `<data_dir>/python-shared` + 기존 `PYTHONPATH`.
    - 그래서 `import extensions.steamcmd`, `import daemon_rcon`, `import i18n`이 가능하다.
    - `python-guard/sitecustomize.py`는 활성 목록에 없는 `extensions.<이름>` import를 막는다 (`:114-126`).
    - 격리 없이 도는 실행은 플랫폼과 관계없이 확인된 익스텐션 또는 그 익스텐션의 현재 지문에 따로 비격리 동의한 익스텐션만 import한다. 모듈의 동의는 다른 익스텐션을 승인하지 않는다. 차단한 항목은 `SABA_UNVERIFIED_EXTENSIONS`에 넣는다. 격리된 실행은 활성 익스텐션을 같은 격리 안에서 import한다. 빠진 항목이 모두 미검증 차단 항목이면 모듈의 `extension_required`를 `extension_not_verified`로 바꾼 뒤 호출 진단 필드를 붙인다.
    - `sitecustomize.py`는 표준 라이브러리 경로를 데몬이 추가한 경로보다 먼저 둔다.
  - `SABA_ENABLED_EXTENSIONS` = 활성 익스텐션 ID를 콤마로 연결한 문자열. `<data_dir>/config.json`의 `extensions.enabled`에서 읽는다 (`:315-322`). 목록이 없으면 빈 값이다.
  - `SABA_IPC_PORT`, `SABA_IPC_URL`(데몬 API 주소), `SABA_TOKEN_PATH`(`X-Saba-Token` 값이 든 파일), `SABA_LANG`(요청의 `Accept-Language`, 없으면 `config.json`의 `gui.language`, 비었으면 OS 언어, 실행마다 다시 읽음)을 넣는다 (`daemon_env` `:18-38`).
  - `SABA_EXTENSION_DATA_ROOT` = 모듈이 import한 익스텐션(SteamCMD 등)이 받은 파일을 `<뿌리>/<익스텐션 id>`에 두는 뿌리(`sandbox::extension_data_root`). 보통은 `<data_dir>/extension-data`, 격리 실행은 구성요소마다 따로인 `<data_dir>/sandbox-data/<꼬리표>/extension-data`다(공용 폴더에 쓰게 하면 확인된 실행이 그 파일을 격리 없이 실행할 수 있다). 데몬이 만들고 격리 실행에는 쓰기를 준다.
  - `SABA_LOOPBACK_DIR` = 격리 실행에만 둔다. AppContainer는 127.0.0.1에 닿지 못하므로(능력은 `internetClient`뿐) 공용 도우미 `daemon_rcon.loopback_exchange`·`loopback_http`가 이 값이 있으면 직접 연결하지 않고 데몬에 맡긴다(`plugin/loopback.rs`). 도우미는 stderr에 `LOOPBACK:{id, port, data(base64), timeout, idle_ms?}` 한 줄을 쓰고, 데몬은 `127.0.0.1:port`와 주고받은 결과를 `<SABA_LOOPBACK_DIR>/<id>.json`(`{ok:true, data}` 또는 `{ok:false, error}`)으로 둔다. 도우미가 읽고 지운다. 허용 포트는 데몬이 만든 config의 `port`·`rest_port`뿐이고(다른 `*_port` 모듈 설정은 모듈이 `import_settings`로 바꿀 수 있어 넣지 않는다), 밖이면 `port_not_allowed`로 거절하고 로그만 남긴다. 그 포트에서 듣는 프로세스가 데몬이 추적하는 그 인스턴스의 서버이거나 그 자식이 아니면(서버가 꺼져 있을 때 포함) `port_not_served`로 거절한다. 서버 PID는 데몬이 `PluginCall::serving`으로 넘긴 값이고 config의 `pid`는 쓰지 않는다. 상한은 보내기 64 KiB, 받기 1 MiB, 요청당 30초, `idle_ms` 2초, 한 실행 64건이고, 넘친 요청은 `too_many_requests`로 바로 거절한다. 받기는 서버가 닫을 때까지이고, `idle_ms`가 있으면 답이 온 뒤 그만큼 조용할 때 끝난다(마인크래프트 상태 핑처럼 닫지 않는 프로토콜). `LOOPBACK:` 줄은 인증 머리글이 들 수 있어 로그에 남기지 않는다. 격리 준비에 실패해 사용자 동의로 격리 없이 도는 실행에는 두지 않는다.
  - `SABA_JAVA_RUNTIMES` = 데몬이 찾은 Java 런타임의 JSON 배열 `[{java, home, version, major, arch}]`(`sandbox::java_runtimes`). 순서는 데몬 환경의 `JAVA_HOME`, PATH의 모든 `java`, 알려진 설치 위치(Windows는 Program Files 두 곳, 그 밖은 `/usr/lib`의 `<공급자>/<jdk>`, 공급자 폴더는 `JAVA_VENDOR_DIRS`)다. `release` 파일이 있는 실제 JDK·JRE만 들고(Oracle `javapath` 같은 바로가기는 빠진다), `version`은 그 `JAVA_VERSION`, `major`는 주 버전(`1.8.0_381`→8), `arch`는 `release`의 `OS_ARCH`(`amd64`·`x86` 등, 없으면 빈 문자열)다. 모듈은 필요한 주 버전 이상의 64비트 런타임을 여기서 고르고, 격리 실행에서도 이 폴더들은 읽을 수 있다. 사용자가 정한 `java_path`는 따로 config로 온다.
    - ~~문서는 `DAEMON_API_URL`을 주입한다고 한다~~ → 해결: 다시 쓴 개발 가이드는 `SABA_IPC_URL`, `SABA_TOKEN_PATH`, `SABA_LANG`이라고 적는다(`M:docs/module-development-guide.md:31`). `daemon_rcon.py`는 `DAEMON_API_URL`이 있으면 그것을, 없으면 `SABA_IPC_URL`을, 둘 다 없으면 `http://127.0.0.1:57474`를 쓴다 (`D:modules/daemon_rcon.py:36-42`).

**반환 경로와 `data_roots` (0.1.10)**

모듈이 돌려준 백업·복원·설치·마이그레이션 경로는 [01 §7.1](01-daemon-core.md#71-srcpluginmodrs-python-모듈익스텐션-실행기-단발-프로세스)의 데몬 측 실제 경로 검사를 받는다. `verified`인 모듈도 예외가 아니다.

서버 폴더 밖의 데이터가 필요하면 `module.toml`의 `[extensions.saba-core]` 예약 표에 `data_roots` 문자열 배열을 선언한다. 값은 `{home}` 또는 path형 설정 자리로 시작해야 한다. path형은 `type = "file"` 또는 `format = "path"`다. `{home}`은 확인된 모듈만 쓸 수 있다. 데몬 데이터 폴더와 데몬 전용 인스턴스 항목은 경계에서 거부한다. 다른 절대 경로, `..`, 다른 자리, 중간의 추가 자리·드라이브는 그 항목만 버리고 경고한다.

```toml
[extensions.saba-core]
data_roots = ["{cache_dir}", "{data_home}", "{home}/Zomboid"]
```

`{home}`은 확인된 모듈에 한해 데몬 사용자의 홈 폴더를 가리킨다. 미확인 모듈은 이 루트를 받지 않는다. `{설정}`은 사용자가 고른 값(`user_settings`) 또는 마이그레이션·시작 후 보충의 `import_settings`가 판정한 path형 값만 루트로 쓴다. 가져온 값의 출처 키는 인스턴스의 `imported_settings` 집합으로 저장한다. 값은 `module_settings` 한곳에만 둔다. 일반 재조회가 값을 바꾸면 그 키의 출처 기록을 지운다. 같은 값을 다시 읽었을 때는 기록을 유지한다. 모듈 기본값만으로 외부 폴더 접근을 허용하지 않는다. 데몬에 게임 이름 분기는 없다.

`data_roots`의 `{설정}` 값이 없고 그 설정의 보충을 시도한 적이 없으면 데몬은 시작 뒤 `import_settings`를 호출한다. 호출은 인스턴스별로 누락 설정을 모아 잠금 밖에서 일반 호출 제한으로 실행한다. `{home}`은 보충 대상이 아니다. 데몬은 반환한 누락 루트 설정만 저장하고 `imported_settings`에 기록한다. 기존 값과 사용자가 정한 키는 저장 직전에도 확인해 보존한다. 호출 전 `instance.json`의 `data_roots_backfill_attempted` 집합에 시도할 설정 이름을 저장한다. 이 집합이 비면 필드를 생략한다. 같은 설정의 빈 결과·실패·중단은 다음 시작에 반복하지 않는다. 모듈 업데이트로 새 루트 설정이 추가되면 아직 시도하지 않은 설정만 보충한다. 실패는 로그에 남기고 데몬 기동은 계속한다.

좀보이드 모듈은 옛 외부 INI 배치에서 부모의 `Saves`·`db`를 유지하도록 `import_settings`에서 `data_home`을 돌려준다. 데몬의 시작 후 보충은 기존 인스턴스에도 이 값을 저장한다. 현재 선택한 INI 폴더는 이 계약으로 바꾸지 않는다.

### 2.2 모듈 측 디스패치 관례

세 모듈과 `__example` 모두 모듈 최상위에 `FUNCTIONS` 표를 두고 진입점은 공용 `daemon_rcon.run_lifecycle(FUNCTIONS)`이다. SDK 최소 버전은 §2.5의 표를 따른다. palworld는 표의 함수마다 `ExtensionMissing`을 `error_code: "extension_required"`로 바꾸는 감싸기를 더해 넘긴다.

1. `sys.argv[1]`을 읽는다.
2. stdin JSON을 읽는다. 비어 있으면 `{}`로 두고, 해석할 수 없으면 함수를 부르지 않고 `{"success":false,"message":"Invalid JSON config on stdin: ..."}`로 답한다.
3. `FUNCTIONS[name](config)`를 호출한다. `PathOutside`는 `module_path_outside`와 `message`·`path`로 답한다. 그 밖의 함수 예외는 `module_exception`과 `message`로 답하고 traceback을 stderr에 쓴다.
4. 결과 JSON 한 줄을 stdout에 쓴다. 직렬화할 수 없는 값은 `success:false`, `error_code:"module_bad_output"`, `message`로 바꾼다.

모르는 함수 이름이 오면 `{"success":false,"message":"Unknown function: X","available":[...]}`를 돌려준다. 어느 경우든 종료 코드는 0이다. 적합성 키트가 깨진 stdin과 예외 경로를 검사한다(`M:conformance/test_conformance.py` `test_bad_stdin_is_answered_not_run`, `test_a_crashing_function_still_answers_json`).

예약 함수 `functions`(MK1): 모듈이 구현한 lifecycle 함수 이름(`FUNCTIONS`의 키, `functions` 자신은 빼고)을 `{"success": true, "functions": [...]}`로 돌려준다. 공용 진입점 `daemon_rcon.run_lifecycle(FUNCTIONS)`은 표에 `functions`가 없으면 이렇게 자동으로 답한다. 데몬은 이 목록을 `GET /api/modules`의 `functions`로 내보내고, 목록에 없는 선택 함수는 부르지 않고 `not_supported`로 답한다(01 §`GET /api/modules`). 답하지 못하는 옛 모듈은 목록을 모르는 것으로 다뤄 아무것도 숨기지 않는다. `functions`는 `[[queries]]`·`[status] players`의 `function`으로 쓸 수 없다.

**에러 관례 (코드에서 관찰한 것)**:
- 공통 응답: `success: bool`, `message: str`.
- 데몬 호출 실패의 `error_code`·`module`·`function`·`phase`·`stderr_tail`은 [01 §7.1](01-daemon-core.md#71-srcpluginmodrs-python-모듈익스텐션-실행기-단발-프로세스)을 따른다. supervisor·훅·HTTP·비동기 설치 상태까지 필드를 유지한다. 모듈이 정한 실패 코드와 메시지는 바꾸지 않는다.
- `get_launch_command` 성공: `program`, `args[]`, `working_dir`, `env_vars{}`. 그 밖의 선택 키:
  - `rest_credentials{rest_username, rest_password}`
  - credential_map의 게임 키(예: palworld의 `"AdminPassword"`)를 최상위나 `module_settings{}` 아래에 둔다.
  - `extra_args_placed: true`: 받은 `extra_args`를 모듈이 알맞은 자리(예: JVM 인자는 `-jar` 앞)에 넣었다. 없으면 데몬이 `args` 끝에 붙인다(`supervisor::launch_args`).
- `get_launch_command` 사용자 조치 필요: `action_required: "<code>"`를 넣으면 데몬이 결과를 그대로 GUI에 넘긴다 (`mod.rs:795-799, 1524-1527`). minecraft는 `server_jar_not_found` + `options:["update_path","install_new"]`를 쓴다.
- 익스텐션 누락: `error_code: "extension_required"`, `missing_extensions[]`.
- 데몬은 그 실행에서 확인되지 않아 import를 막은 익스텐션(`SABA_UNVERIFIED_EXTENSIONS`)만으로 `missing_extensions`가 채워진 `extension_required`를 `error_code: "extension_not_verified"`, `unverified_extensions[]`, 영문 `message`로 바꿔 돌려준다(`plugin/mod.rs` `explain_unverified_extensions`). 모듈이 import 실패를 삼켜도 설치·활성화 안내 대신 "파일이 바뀌어 확인되지 않음"이 보이게 하려는 것이다.
- `status`: `status` 문자열과 `pid`를 데몬이 읽는다. 나머지 키는 버린다 (`mod.rs:1182-1197`).
- `install_server`: `success`, `executable_path`, `install_path`를 프로비저닝 경로가 읽는다 (`instance.rs:531-559`) `install_path`는 인스턴스의 `working_dir`가 되는데, 확인되지 않은 모듈(Windows)이 돌려준 값은 실제 경로가 인스턴스 폴더 안의 폴더일 때만 받는다(파일은 받지 않는다)(`sandbox::accepts_module_dir`. 그 폴더는 다음 격리 실행에 쓰기를 받는다).
- `get_installed_version`: `version`.
- `get_credentials`: `success`, `rest_username`, `rest_password`. 데몬은 REST 자격증명만 쓴다 (`command.rs:579-581, 660-687`). 지금은 palworld만 구현한다. minecraft·zomboid는 RCON 값만 돌려주던 이 함수를 지웠다.
- `import_settings`: `success`, `settings{}`. `port`·`rest_port`와 credential_map의 게임 키(→ `rest_password`)는 인스턴스 필드로도 맞춘다 (`D:src/instance/mod.rs:172-206`). 마이그레이션에서는 `executable_path`(작업 폴더 기준 상대 경로, 모듈이 찾은 서버 실행 파일)와 `detected`(판정 결과, 데몬은 내용을 보지 않고 생성 응답에 넘긴다)도 읽는다.
- `instance_commands`: `success`, `commands[]`(명령 필드 + `group`), `components` (`[commands] hook = true`일 때).
- `install_server`: config에 인스턴스 설정 `settings{}`가 온다(프로비저닝·업데이트, `instance_id`를 준 `POST /api/module/:name/install`).
- `list_versions`: config `settings{}`(인스턴스 설정)로 그 설정에서 받을 수 있는 최신을 `latest.release`(예전 모양 `latest_release`)에 준다. 예: Minecraft `server_type`이면 Paper·Folia는 안정 빌드가 나온 가장 새 버전. 데몬은 처음 설치와 버전을 비운 `POST /api/module/:name/install`에서 이 값만 읽는다(`Supervisor::latest_version`. 없으면 모듈의 `message`를 까닭으로 싣는다).
- `backup_paths`: `success`, `paths[]`, 선택 `restore_roots[]` (`[backup] hook = true`일 때). `restore_roots`는 `paths`와 같이 해석하는 복원 허용 범위다. 그 폴더 안을 담은 옛 백업(데이터 폴더를 통째로 담던 예전 모듈의 백업 등)도 복원되게 하고, 백업 대상에는 넣지 않는다 (MA3).

### 2.3 데몬이 호출하는 함수 전체 목록과 전달 config

| 함수 | 호출 지점 | 데몬이 넣는 config 키 |
|---|---|---|
| `get_launch_command` | managed 시작 `mod.rs:1500-1520`, non-managed 시작 `mod.rs:773-792`, 실행 인자 미리보기(`Supervisor::launch_preview`) | (요청 payload.config) + `server_executable`, `working_dir`, `port`, **`module_settings`를 최상위로 펼침**, `rest_host`, `rest_port`, `rest_username`, `rest_password`, `protocol_mode`, `server_version`, `instance_id`, `extra_args`(사용자가 더한 인자, 문자열 배열), `managed: true/false` (`insert_instance_fields`). 미리보기에서는 `preview: true`도 온다: 이때 모듈은 설정 파일을 쓰는 등 부작용을 내지 않는다 |
| `stop` | non-managed 정지 `mod.rs:1068-1077`, stdin을 쓸 수 없는 managed 서버(REST 전용, stdin 없는 재부착)의 정지 `mod.rs:971-982` | `server_executable`, `working_dir`, `module_settings`(펼침), `rest_*`, `protocol_mode`, `server_version`, `instance_id`(`insert_instance_fields`, `mod.rs:481-498`), `pid`, `force`, `managed`. 추적 중인 PID가 없으면 non-managed에서는 부르지 않는다 (`mod.rs:1057-1066`). 콘솔이 있는 managed 서버는 이 함수를 부르지 않고 데몬이 `stop_command`를 stdin으로 보낸 뒤 기다린다 (`mod.rs:949-969`) |
| `status` | non-managed 상태 조회 `mod.rs:1160-1178` | `server_executable`, `working_dir`, `pid`, `module_settings`(펼침) |
| `command` | `POST /api/instance/:id/command`에서 명령의 `method`가 `module`일 때 → `mod.rs:1220-1279` | `command`, `args`, `protocol_mode`, `working_dir`, `rest_host`, `rest_port`, `pid`, `instance_id` |
| `get_credentials` | `/rest` 요청 시. 결과를 캐시하므로 캐시가 있으면 Python을 띄우지 않는다 (`command.rs:660-687`) | `working_dir` |
| `validate` | `POST /api/instance/:id/validate`, 그리고 `POST /api/instance/:id/start`의 시작 전 점검(`server.rs:712-764`) → `mod.rs:1818-1859` | `server_executable`(=executable_path), `working_dir`, `port`, `extension_data`, 인스턴스 값(위와 같음), `running` |
| `configure` | `PUT /properties`, 인스턴스 설정 PATCH 후 동기화 (`instance.rs:1276-1349`) → `manage_properties` `mod.rs:1865-1916` | `working_dir`, `extension_data`, 인스턴스 값, `port`, `settings{}`. PATCH 동기화의 `settings`에는 데몬 필드(`protocol_mode`·`executable_path`·`working_dir`·`server_version`·`extension_data`·`managed_start`·`rest_*`·`rcon_*`·`enable_rcon`)와 모듈이 `group = "saba-chan"`으로 선언한 사바쨩 설정이 빠진다(MK6) |
| `read_properties` | `GET /properties` | `working_dir`, `extension_data`, 인스턴스 값, `port` |
| `reset_properties` | `POST /properties/reset` | 위와 같음 |
| `reset_server` | `POST /server/reset` | 위와 같음 |
| `import_settings` | 마이그레이션 인스턴스 생성 `instance.rs:653-657`, `GET /api/instance/:id/settings/live` (`managed.rs:245-271`) | 마이그레이션은 `working_dir`만, live는 위와 같음 |
| `instance_commands` | `GET /api/instance/:id/commands`, module.toml에 없는 이름의 명령 실행 (`[commands] hook = true`일 때, `D:src/ipc/handlers/command.rs`) | `working_dir`, 인스턴스 값(`insert_instance_fields`) |
| `accept_agreements` | `POST /api/instance/:id/agreements` (동의를 기록한 뒤) | `validate`와 같은 config + `accepted_agreements:[id]`(지금까지 동의한 전부). 모듈은 자기 파일에 동의를 쓴다(예: minecraft eula.txt). 함수가 없으면 기록만 남는다 |
| `diagnose_log` | `POST /diagnose` | `working_dir`, `log_lines[]`(managed일 때 최근 500줄 중 이번 실행분, `mod.rs:1982-1990`) |
| `list_versions` | `GET /api/module/:name/versions`, 프로비저닝(download 방식), 버전을 비운 `POST /api/module/:name/install` | `include_snapshots`, `page`, `per_page` 또는 최신을 고를 때 `{settings}` |
| `get_version_details` | `GET /api/module/:name/version/:v` | `version` |
| `install_server` | `POST /api/module/:name/install`, 프로비저닝·업데이트(모두 1800초) | `version`, `install_dir`, `accepted_agreements`(동의한 `[[agreements]]` id), `settings`(프로비저닝 때, 그리고 `POST /api/module/:name/install`에 `instance_id`가 있을 때 그 인스턴스 `module_settings`), `instance_data_dirs`(같은 때), `server_executable`(요청 값, 없으면 모듈 `server_executable`), `initial_settings` 등 |
| `get_installed_version` | `GET /installed-version`, 데몬 초기화 시 backfill `mod.rs:2200-2249` | `validate`와 같은 config(`Supervisor::instance_call`: `server_executable`, `working_dir` 등). `server_executable`은 시작과 같이 `resolve_executable`로 정한다(인스턴스 경로, 옛 `module_settings.executable_path`, `working_dir` + 모듈 `server_executable` 순) |
| `[[queries]]`의 `function` | `POST /api/instance/:id/query/:name`(액션 `server.query:<name>`) → `Supervisor::query_instance` | `validate`와 같은 config(`Supervisor::instance_call`) + `args{입력 이름: 값}`(명령 입력과 같이 검사·기본값 채움, 숫자는 숫자로). 돌려줄 것: `{"success": true, "data": <JSON>}` 또는 실패 `{"success": false, "message": "..."}`. `data`가 없으면 `success`·`message`를 뺀 나머지를 data로 본다. 읽기만 해야 한다(감사에 남지 않고 챗봇·일반 사용자도 부른다) |
| `[status] players`의 함수 | 켜진 서버마다 1분에 한 번(`supervisor::players::refresh`, 10초 제한), 그리고 플레이어 목록 요청 때(`GET /api/instance/:id/players`·액션 `server.players`, 10초 제한) | `validate`와 같은 config. 목록 요청 때만 config에 `players_detail: true`가 더 실린다. 돌려줄 것: `{"success": true, "online": <0 이상 정수>, "max": <정수>?, "players": [{"name": "..."}]?}`(`max`가 없거나 0이면 최대를 모른다고 본다. `players`는 `players_detail`이 있을 때만 의미가 있고, 이름 말고 다른 필드는 데몬이 뺀다. 이름은 제어 문자를 빼고 64자로 자른다). 실패 `{"success": false, "message": "..."}`이거나 정수가 아니면 값을 비운다(0으로 보이지 않는다). 실패가 5분 넘게 이어지면 `message`가 `unresponsive`로 GUI에 보인다. 서버는 정상인데 수를 알 수 없으면(예: 상태 응답을 끈 설정) `online` 없이 `{"success": true}`를 돌려준다: 값은 비우고 실패로 세지 않는다. 읽기만 해야 한다 |
| `backup_paths` | 백업 때 `[backup] hook = true`인 모듈 (`D:src/ipc/handlers/backup.rs:94-99`) | `working_dir`, `extension_data`, 인스턴스 값 |

모듈의 `start` 함수는 모든 모듈에서 지워졌다(FUNCTIONS에 없음). non-managed 모드도 `get_launch_command`로 데몬이 직접 spawn한다 (`mod.rs:773-847`).

**예전 개발 가이드의 "호출 순서"와 다른 점** (가이드는 a8b6c29에서 현재 데몬 기준으로 다시 쓰였다. 아래는 `__example__lifecycle.py:36-46` 주석과의 차이다):
- "시작 시 `validate → configure → get_launch_command`"라고 되어 있다. 실제로는 `POST /api/instance/:id/start`가 `validate`를 시작 전 점검으로 부르고(critical 문제만 막고, 점검이 실패하면 막지 않는다) 이어서 `get_launch_command`를 부른다. `configure`는 시작 때 부르지 않는다. `/managed/start`와 `/server/:name/start`를 직접 부르면 점검도 없다.
- "status 주기적 호출"이라고 되어 있지만, 실제로는 non-managed 인스턴스의 status API 요청 때만 호출된다. 모니터 루프는 PID 스냅샷만 본다.
- ~~"rcon_template이 있으면 데몬이 직접 RCON을 보낸다"~~ → 해결: 개발 가이드가 다시 쓰여 `method = "console"` + `console_template`을 콘솔(stdin)로 보낸다고 적는다(`M:docs/module-development-guide.md:182-200`, `D:src/commands/mod.rs:173-174`). `__example__lifecycle.py` 주석도 `method="console"`로 고쳐졌다(`:44`, e1d7743).
- ~~"config에 `instance_id`, `settings{}` 항상 포함"~~ → 부분 해결: `instance_id`는 인스턴스 필드를 넣는 호출(`insert_instance_fields`, `D:src/supervisor/mod.rs:481-498`)에 들어간다. 그 호출에서 `rest_port`는 인스턴스 값이 모듈 설정의 같은 키보다 먼저 들어간다(데몬이 접속하는 포트가 기준). `settings{}`는 여전히 `configure`에만 들어가고, 그 외 함수에서는 `module_settings`가 **최상위로 펼쳐져** 들어간다(템플릿 주석 `:58-63`은 아직 `settings`가 항상 온다고 적는다).

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
| backup_paths (hook일 때 호출) | ✔ | ✔ | ✔ | – |
| player_count (`[status] players`) | ✔ | ✔ | – | – |
| instance_commands (호출) | ✔ | – | – | – |

출처: 각 lifecycle의 최상위 `FUNCTIONS` 표 (2026-10-22 대조).

**영향**:
- 데몬 초기화 때의 backfill은 palworld와 zomboid에서 매번 실패한다(debug 로그).
- zomboid의 "설정 초기화"와 EULA, 버전 목록 API는 Unknown function을 돌려준다.

### 2.5 공용 Python 유틸 (`<data_dir>/python-shared/`)

- `i18n.py`: `I18n(module_dir)`가 `locales/<lang>.json`을 읽는다. 언어는 `SABA_LANG`으로 정하고 기본값은 `en`이다. 점 표기 키와 `{{var}}` 보간을 지원한다. `__example/i18n.py`는 같은 파일의 복사본이다.
- `daemon_rcon.py`: 모듈 공용 헬퍼다. 이름은 예전 `from daemon_rcon import ...` 줄을 살리려고 남겼고 RCON 소켓은 열지 않는다. 임의 경로용 `daemon_post`, stdin 전송(`daemon_stdin_command`), 루프백 중계, PID 확인·강제 종료, 설정값 조회·형 변환, 원자적 파일 저장(`atomic_write`), `run_lifecycle` 진입점이 들어 있다 (`D:modules/daemon_rcon.py:9-16`). RCON 명령 전용 함수는 없다. RCON만 받는 모듈은 `daemon_post`로 `/api/instance/:id/rcon`을 부른다(RCON 정책).
  - 주소는 `DAEMON_API_URL` → `SABA_IPC_URL` → `http://127.0.0.1:57474` 순서다 (`:40-46`).
  - `SABA_TOKEN_PATH` 파일의 토큰을 `X-Saba-Token` 헤더로 보낸다 (`:49-61`). palworld(`execute_rest_via_daemon` `M:palworld/lifecycle.py:1414-`)는 `daemon_post`를, minecraft·zomboid·__example은 공용 `daemon_stdin_command`(`/stdin`에 `{"command"}`, `D:modules/daemon_rcon.py:87-89`)와 PID 헬퍼 `is_process_alive`·`wait_for_exit`·`force_kill`을 쓴다 (`M:zomboid/lifecycle.py:36`).
  - `atomic_write(path, data)`(saba-core 0.1.10부터)는 같은 폴더의 고유 임시 파일(`.<이름>.*.tmp`)에 쓰고 fsync한 뒤 바꾼다. 원래 파일이 있으면 그 권한을 잇는다(Windows `ReplaceFileW`로 DACL, POSIX는 권한 비트). 격리된 모듈처럼 DACL을 옮길 권한이 없어 `ReplaceFileW`가 거부되면, 같은 폴더에 만든 임시 파일과 원래 파일의 소유자·DACL·무결성 표시와 EFS 암호화 여부가 똑같을 때만 `os.replace`로 바꾼다(둘 다 폴더 권한만 물려받은 경우). 다르면 거부한다. 모듈이 게임 설정 파일을 쓸 때 쓴다.
  - payload의 `rcon_port`와 `rcon_password`는 데몬 `/rcon` 핸들러가 무시한다. RCON 접속에는 데몬이 만든 `rcon.json` 값만 쓴다(RCON 정책).
- 배포: 데몬 실행 파일에 공용 파일이 들어 있다. 첫 lifecycle 실행 때 데몬 전용 `<data_dir>/python-shared/`에 쓴다. 이 경로만 공용 유틸리티의 import 경로로 쓴다. 모듈 폴더는 `PYTHONPATH`에 넣지 않는다. 적합성 키트의 `--shared-libs`에는 `<data_dir>/python-shared/`를 지정한다.

**경로 도우미와 SDK 버전 (0.1.10)**

`daemon_rcon.safe_join(root, *parts)`는 이름 조각을 합친 실제 경로가 `root` 안이면 `Path`로 돌려준다. 절대 경로, 드라이브, `..`, 구분자가 든 이름 조각과 루트 밖으로 나가는 링크는 `PathOutside`다. `run_lifecycle`은 이 예외를 `{success:false, error_code:"module_path_outside", message, path}`로 바꾼다.

`daemon_rcon.py` 머리의 `SINCE`가 공용 SDK 이름의 최소 버전 표다. `i18n`의 이름도 같은 표에 넣는다.

| 공개 이름 | 처음 제공한 코어 버전 |
|---|---|
| `i18n.I18n` | `0.1.1` |
| `daemon_post`, `daemon_stdin_command` | `0.1.2` (릴리스 커밋 기준) |
| `is_process_alive`, `wait_for_exit`, `force_kill` | `0.1.2` (릴리스 커밋 기준) |
| `config_value`, `load_setting_types`, `coerce_setting`, `run_lifecycle` | `0.1.2` (릴리스 커밋 기준) |
| `loopback_exchange`, `loopback_http` | `0.1.8` (릴리스 커밋 기준) |
| `safe_join` | `0.1.10` |
| `PathOutside` | `0.1.10` |
| `daemon_rest` | `0.1.10` |

이력 근거: 공개 태그 `test23`(`f493b78e`, 코어 `0.1.1`)에 `i18n.I18n`이 있다. 기존 통신·설정·프로세스 함수는 `97db8a0e`(`0.1.2` 릴리스 커밋)에 있다. 루프백 함수는 `1862c5c0`(`0.1.7` 릴리스 커밋)에는 없고 `77333cab`(`0.1.8` 릴리스 커밋)에 있다. 공개 origin에는 `test`~`test23` 태그만 있다. 따라서 `0.1.2`·`0.1.8` 값은 릴리스 커밋에 근거한 보수적 값이며 최초 공개 배포 태그로 확정한 값은 아니다. 이 미확정 부분은 07 §4에 남긴다.

`saba_audit.py`는 `daemon_rcon`·`i18n`의 import와 별칭 사용을 표와 비교한다. 선언이 없거나 낮으면 `sdk_version` finding으로 실패하며 함수 이름·현재 선언·필요 버전을 알린다. 모듈은 `[module.dependencies] saba-core = ">=<버전>"`으로 최소 버전을 선언한다. 범위·상한은 지원하지 않는다. 익스텐션 감사의 기존 호환성은 manifest의 코어 의존성도 같은 방법으로 읽어 유지한다.

데몬·CLI 감사는 Rust가 TOML 선언을 읽어 검사기에 넘기므로 Python 3.10도 지원한다. 단독 `saba_audit.py`로 `module.toml`을 읽으려면 `tomllib`이 필요하다. Python 3.10에서 이 단독 실행은 명확히 실패하며 `saba-core module audit` 사용을 안내한다. 선언 누락 finding으로 잘못 바꾸지 않는다.

`daemon_stdin_command`와 `daemon_rest(instance_id, method, path, *, body=None, timeout=10)`는 `SABA_HOST_DIR`이 있으면 [01 §7.1](01-daemon-core.md#71-srcpluginmodrs-python-모듈익스텐션-실행기-단발-프로세스)의 HOST 통로를 쓴다. 대상은 인자로 받은 `instance_id`가 아니라 데몬의 호출 문맥이다. 환경 변수가 없으면 기존 HTTP API를 쓴다. `daemon_rest`의 HTTP fallback은 `path`를 `endpoint`로 보낸다. HOST 실패는 `error_code` 속성이 있는 `ConnectionError`다. 일반 토큰 경로 `SABA_TOKEN_PATH`와 `daemon_post`는 옛 모듈 호환용으로 남아 있다. 공식 모듈의 SDK 전환은 part 3에서 한다.

기존 `run_lifecycle`의 최초 제공 버전은 올리지 않는다. 0.1.10부터 `PathOutside`를 별도로 바꾸고, 일반 함수 예외에 `module_exception`을 넣는다. 결과 직렬화 실패에는 `module_bad_output`으로 답하며 종료 코드 0을 유지한다.

---

## 3. Managed Process 모델

### 3.1 두 가지 실행 경로

시작 경로는 데몬이 고른다. `POST /api/instance/:id/start`가 `Supervisor::uses_managed_start`(인스턴스 설정 `managed_start`, 없으면 managed)로 경로를 정하고, 시작 전 점검(§2.3 `validate`)을 거쳐 아래 둘 중 하나를 부른다 (`D:src/ipc/handlers/server.rs:649-710`, `D:src/supervisor/mod.rs:1690-1700`). GUI는 이 경로만 부른다 (`D:saba-chan-gui/src/api/createApi.js:133-136`).

| | managed (`POST /api/instance/:id/managed/start`) | non-managed (`POST /api/server/:name/start`) |
|---|---|---|
| 엔트리 | `Supervisor::start_managed_server` `mod.rs:1298-1686` | `Supervisor::start_server` `mod.rs:550-855` |
| 사전 검사 | 중복 실행 방지(managed_store와 tracker PID 생존 확인), 모듈 의존성, 포트 충돌, 시작 중 표시(`StartGuard`), extension `server.pre_start` hook | 포트 충돌, 모듈 의존성, `requires_extensions` 검증, 시작 중 표시, `server.pre_start` hook |
| spawn | saba-runner가 있으면 `ManagedProcess::spawn_with_runner`(러너가 10초 안에 `runner.json`을 쓰지 못하면 러너와 그 자식을 끝내고 시작 실패), 없으면 `ManagedProcess::spawn`(stdin, stdout, stderr 파이프) (`mod.rs:1622-1660`) | `TokioCommand` — stdio 모두 `null`, 콘솔 없음 |
| 추적 키 | `tracker.track(instance.id, pid)` | `tracker.track(instance.id, pid)` (`mod.rs:846-847`) |
| PID 영속화 | `<instance_dir>/managed.pid`, runner면 `<instance_dir>/run/` | 없음 |
| 자격증명 동기화 | `rest_credentials` 또는 `credential_map` | 없음 |

- `requires_extensions` 검증은 **non-managed 경로에만 있다** (`mod.rs:572-629`). managed 경로는 `[module.dependencies]`의 `ext-<id>` 검사만 한다.
- 두 경로 모두 인스턴스 ID로 추적하므로 모니터·포트 충돌 검사·정지·상태가 같은 키를 쓴다.
- **확인되지 않은 모듈의 실행 대상 (Windows)**: `get_launch_command`가 돌려준 `program`은 두 경로 모두 띄우기 전에 검사한다(`Supervisor::guard_launch`, `sandbox::check_launch`). 신뢰 표시가 `verified`인 모듈은 검사하지 않는다.
  - 허용: 실제 경로(링크·`..`를 푼 것)가 인스턴스 폴더나 사용자가 정한 서버 폴더(`working_dir`) 안에 있는 파일. 모듈이 돌려준 `working_dir`은 허용 위치로 쓰지 않는다.
  - 경로가 있는 상대 `program`은 데몬 폴더가 아니라 모듈이 돌려준 `working_dir` 기준으로 풀어 검사하고, 검사한 그 절대 경로로 띄운다.
  - 허용: 경로 없는 `java`·`javaw`, 그리고 사용자가 정한 설정 `java_path`(인스턴스 `user_settings`에 있고 로컬 드라이브 절대 경로인 것)가 가리키는 java. 인스턴스 `user_settings`는 사용자가 직접 정한 `module_settings` 키다: 만들 때 보낸 값이 모듈 기본값(`{instance_dir}`을 바꾼 값 포함)·고른 프리셋과 다를 때, 설정 저장(`PATCH`)에서 값이 바뀐 키만 기록하고, 게임 파일에서 읽어 온 값(`import_settings`)은 기록에서 뺀다. 원격(디스코드·챗봇)은 `java_path`를 바꿀 수 없다(05 문서 액션 원격 규칙). 이 기록이 생기기 전에 만든 인스턴스는 `user_settings`가 비어 있어, 사용자가 정한 `java_path`도 데몬의 `JAVA_HOME`·PATH JDK가 아니면 `launch_outside_instance`가 된다(기본값과 구별할 수 없어 자동으로 옮기지 않는다). 서버 설정에서 Java 경로를 다시 골라 저장하면 기록된다(값이 바뀐 저장만 기록하므로 같은 경로면 한 번 비워 저장한 뒤 다시 고른다. 오류 문구도 다시 고르라고 안내한다). 실행 대상·인자 파일 검사는 모듈이 준 경로가 로컬 드라이브 절대 경로가 아니면 파일 시스템에 묻지 않고 거부하거나 건너뛴다(`plain_local_path`). 인자는 그대로, `@파일`, `-옵션=값`·`-옵션:값`의 값, 그리고 Windows에서는 `;`로 나눈 경로 목록(클래스 경로 `-cp a.jar;b.jar`, `-Djava.class.path=…`)의 항목마다 본다(`sandbox::argument_files`). Java가 인자처럼 읽는 것도 같이 본다: `env_vars`의 `JAVA_TOOL_OPTIONS`·`JDK_JAVA_OPTIONS`·`_JAVA_OPTIONS`, `CLASSPATH`(경로 목록), 그리고 인자·이 변수들이 가리키는 인자 파일(`@파일`, `-XX:VMOptionsFile=파일`)의 내용(한 단계만. 로컬 경로가 아니거나 1 MiB를 넘거나 읽지 못하면 막는다)(`sandbox::indirect_arguments`). 낱말은 Java처럼 나눈다: 따옴표(`"`·`'`)는 낱말 중간에서도 벗기고, 따옴표 안 `\`를 이스케이프로 보는 규칙과 글자 그대로 보는 규칙 둘 다로 나눈 낱말을 모두 본다(`java_tokens`). 폴더·와일드카드(`-cp .`, `lib\*`)는 인스턴스 폴더 안이면 막지 않는다: 확인되지 않은 모듈이 서버 폴더에 둔 코드는 서버로 돌 수 있다(§2.1 격리 범위, 게임 서버는 격리하지 않는다). 데몬의 `JAVA_HOME`·PATH가 가리키는 JDK의 `bin/java(w)`도 인정한다(§2.1의 읽기를 주는 런타임과 같은 판정, `sandbox::daemon_runtimes`).
  - 모듈이 돌려준 `env_vars`의 `PATH`·`PATHEXT`(대소문자 무시)는 버린다. 경로 없는 `java`는 데몬의 PATH로 찾는다.
  - 거부: 셸·스크립트 호스트(`cmd`, `powershell`, `wscript`, `mshta`, `rundll32`, `regsvr32` 등)와 `.bat`·`.cmd`·`.ps1`·`.vbs` 같은 스크립트는 어디 있든 거부한다.
  - 거부하면 `{success:false, action_required:"launch_outside_instance", message, file}`를 돌려준다.
  - 실행 인자가 가리키는 파일(인자 그대로, `@파일`, `-옵션=파일`·`-옵션:파일`, 상대 경로는 작업 폴더 기준)도 로컬 드라이브 경로여야 한다. 아니면 `launch_outside_instance`다.
- **서버 폴더의 낮은 수준 표시 걷기 (Windows)**: 격리 실행에 쓰기를 준 폴더에는 낮은 수준 표시가 붙고, 그 안에 받은 서버 파일(Java 등)도 표시를 상속한다. 낮은 수준 표시가 걸린 실행 파일로 띄운 프로세스는 낮은 무결성으로 돌아 뜨지 못한다(예: `0xC0000135`, `0xC0000022`). 프로세스 수준은 실행 파일의 표시만 따르므로, 데몬은 서버를 띄우기 전에(모듈이 띄우든 익스텐션 시작 훅이 띄우든, 모든 실행이 지나는 `sandbox::guard_launch_for`에서) 신뢰 표시와 상관없이 인스턴스 폴더와 사용자가 정한 서버 폴더, 그 아래 폴더의 실행 파일(`.exe`·`.com`, 대소문자 무시) 중 낮은 수준이 걸린 것에 중간 수준 표시를 직접 붙인다(`sandbox::restore_server_folders` → `appcontainer::clear_low_labels`). 띄울 파일 자체가 그 폴더 안에 있으면 확장자와 상관없이 그 파일도 덮는다(확장자가 다른 PE도 Windows가 띄운다, `appcontainer::clear_low_label_of`, L38). 직접 붙인 표시는 상속받은 표시보다 앞에 와서 그 파일의 수준이 되고, 위 폴더 표시가 다시 퍼져도 남는다. 폴더와 다른 파일의 표시는 그대로 둔다: 폴더 표시를 걷으면 모든 하위 파일에 상속 계산이 퍼지고 다음 격리 실행이 다시 붙이며 또 퍼져, 큰 서버 폴더(파일 4만 개)는 시작마다 수십 초가 걸렸다. 그래서 격리된 모듈은 서버 설정 파일 등을 계속 쓸 수 있고, 중간 수준이 된 실행 파일은 낮은 수준 프로세스가 고치지 못한다(지우고 새로 만든 파일은 상속 표시를 받아 다음 시작 때 다시 덮인다). 0.1.6까지 격리되어 표시가 남은 폴더도 다음 시작 때 고쳐진다. 덮기부터 서버 프로세스를 띄울 때까지는 격리 실행이 권한을 주지 못하게 잠근다(`sandbox::LaunchGate`, 격리 실행은 권한을 주기 전에 같은 잠금을 잡는다). 한 폴더에서 실패해도 나머지를 마저 보고, 낮은 수준이 걸린 실행 파일을 덮지 못했거나 그 표시를 읽지 못했거나 폴더 목록을 읽지 못했으면(실행 파일이 남았는지 알 수 없다) 띄우지 않고 `{success:false, action_required:"server_folder_label", file, detail}`을 돌려준다. 볼륨이 파일 보안 정보를 두지 않으면(FAT·exFAT, `GetVolumeInformationW`에 `FILE_PERSISTENT_ACLS` 없음) 표시가 있을 수 없으므로 넘어간다. 걷기는 실제 경로(`canonicalize`, `\\?\` 긴 이름)에서 시작하고, 각 경로를 열 때 `GetFinalPathNameByHandleW` 결과와 비교해 중간 폴더가 정션이면 고치지 않는다(`\\?\` 머리·짧은 이름(8.3)·대소문자·끝 구분자는 같게 본다, `appcontainer::same_final_path`). 링크가 아닌 재분석 지점(목록의 속성으로 거른다)은 실행 파일이 아니어도 열어 보고(압축(WOF)·중복 제거 파일·폴더는 보통처럼 다룬다), 폴더 자체나 그 아래 파일·폴더가 동기화 폴더(OneDrive 등)의 클라우드 파일(재분석 태그 `0x9000_?01A`)이면 실행 파일 여부와 상관없이 막되 `action_required: "server_in_sync_folder"`(file은 그 파일)로 알린다. 사용자 결정으로 동기화 폴더 아래 서버는 지원하지 않는다(폴더를 옮기라고 안내).

**잠금**: `start_managed_handler`는 비밀번호 자동 생성 때만 write lock을 잡고, `start_managed_server`는 검사·준비·등록 때만 락을 짧게 잡는다. 익스텐션 hook, `get_launch_command`, 프로세스 띄우기는 락 밖에서 한다 (`D:src/ipc/handlers/managed.rs:29-68`, `mod.rs:1289-1297`, C12).

### 3.2 상태와 종료 판정

- `state_machine.rs`는 지워졌다.
- 데몬이 노출하는 "상태"는 다음과 같다.
  - managed: `running`/`stopped` 두 값이다. `watch::channel<bool>`의 `is_running()`으로 정한다 (`mod.rs:1146-1158`).
  - non-managed: 모듈 `status()`가 돌려준 문자열을 그대로 쓴다. Python이 실패하면 `"unknown"`이다.
  - extension hook `server.status`가 `handled: true`를 돌려주면 그 결과가 우선한다.
- managed 서버가 끝나면 `judge_exit`가 `user_stop` / `crash` / `start_failed` / `in_game_stop` / `unknown_exit`로 가르고 기록한다 (`D:src/supervisor/crash.rs`). 이력은 `GET /api/instance/:id/events`로 본다. 이력은 인스턴스 폴더의 `exit-events.json`에도 저장해서 데몬을 다시 켜도 남는다.
- `ProcessTracker`(`D:src/supervisor/process.rs`)는 PID와 시작 시각만 기록한다(`track`, `track_since`, `replace_pid`, `untrack`). 예전의 `ProcessStatus`, `get_status`, `mark_crashed`, `terminate`는 지워졌다. 강제 종료는 `ManagedProcess::force_kill`(서버는 `force_kill_tree`, runner는 `force_kill_pid`, `managed_process.rs:234-239`)이다. Windows의 `force_kill_tree`는 `taskkill /F /T`로 자식까지 끝낸다: runner 없이 띄운 런처형 서버(PalServer.exe)도 실제 서버가 남지 않는다. `force_kill_pid`는 그 PID 하나만 끝낸다(업데이트 때 GUI를 닫는 `close_guis`는 GUI가 띄운 데몬·업데이터를 남겨야 한다).

### 3.3 콘솔 캡처와 로그 파싱 (`managed_process.rs`)

- **링 버퍼**: `LogBuffer`. 기본 10,000줄(`:33`). 단조 증가하는 `id`를 붙여 `get_since(id)`와 `get_recent(n)`으로 폴링한다. IPC는 폴링(`GET /console?since=`) 방식이다. 예전의 broadcast 채널과 `subscribe`, `wait_for_exit`는 지워졌다.
- **stdout/stderr 리더**: 각각 tokio task로 줄 단위로 읽는다 (`:313-342`). stderr 줄은 레벨이 Info로 파싱되더라도 **Warn으로 올린다**.
- **레벨 파싱**: `parse_log_level(line, log_pattern)`이 `(?P<level>)` 캡처를 매핑한다 (`:761-775`).
  - ERROR/FATAL → Error, WARN/WARNING → Warn, DEBUG/TRACE → Debug, 그 외 → Info.
  - 패턴이 없거나 매치되지 않으면 Info다.
  - 정규식이 틀리면 warn 로그를 남기고 패턴 없이 동작한다.
  - zomboid 패턴의 `LOG` 같은 값도 Info가 된다.
- **디스크 영속화**: 직접 실행이면 `<instance_dir>/logs/console.log`에 append한다. 포맷은 `[datetime] [source/level] content`이다 (`:788-803, 878-940`). 10MB가 되면 로테이션하고 2개(`.1`, `.2`)를 보관한다. writer가 죽으면 콘솔에 경고 줄을 넣는다. saba-runner로 띄우면 runner가 `<instance_dir>/logs/sessions/<시각>.log`에 쓰고, 기본 20개(`log_sessions_keep`)를 남긴다 (`managed_process/runner.rs:24-25, 133-138`).
- **데몬 재시작 시 재부착**: `initialize → reattach_orphaned_managed` (`mod.rs:180-252`).
  - saba-runner가 쥔 서버면 `attach_runner`가 콘솔 기록, stdin, 종료 코드를 그대로 다시 잡는다 (`managed_process/runner.rs:198-224`).
  - 기록한 PID가 아직 그 프로세스인지는 살아 있는지에 더해 시작 시각으로 본다(`process::is_same_process`): 기록한 때(runner는 `runner.json`의 `started_at`, `managed.pid`는 파일 수정 시각)보다 10초 넘게 늦게 시작한 프로세스는 재부팅 뒤 같은 PID를 받은 다른 프로세스로 보고 붙지 않는다(정지할 때 그 프로세스를 끄지 않게). 시작 시각을 읽지 못하면 살아 있는지만 본다.
  - runner가 없고 `managed.pid`의 PID만 살아 있으면 `create_reattached_stub`을 만든다. 이 스텁은 디스크 로그를 로드하고, 5초마다 PID 생존을 폴링하며, **stdin을 쓸 수 없고 새 출력도 받지 못한다** (`managed_process.rs:618-682`).
- **disk fallback**: managed 프로세스가 없으면 `get_console_output`이 가장 최근 디스크 로그(세션 로그 또는 `console.log`)를 읽는다. 이때 `since_id`는 무시된다 (`mod.rs:1763-1781`).
- **log follower**: 익스텐션이 `server.pre_start`에서 `log_follower{program,args,working_dir,interactive,strip_prefix,env_vars}`를 돌려주면 `spawn_log_follower`(읽기 전용) 또는 `spawn`(양방향)으로 그 프로세스를 managed_store에 등록한다. 종료 판정 대상은 아니다 (`mod.rs:684-761, 1411-1488`).

### 3.4 콘솔 입력, 정지, 강제 종료

- **stdin**: `POST /api/instance/:id/stdin` `{command}`가 `/rcon`과 같은 `via_stdin`을 부르고, 보낸 뒤 찍힌 콘솔 줄을 `data.response`로 돌려준다 (`managed.rs:117-136`, `command.rs:432-`). 쓰기는 mpsc(256)를 거쳐 writer task가 개행을 붙여 쓴다 (`managed_process.rs:285, 344-351, 408-415`).
- **managed stop** (`mod.rs:951-1026`):
  1. `force`이면 `force_kill`을 한다.
  2. 모듈이 `stdin`을 지원하고 stdin이 열려 있으면 `stop_command`를 stdin으로 보낸다. 그 시작이 아직 `ready_patterns`를 기다리는 중이면(켜지는 중) 준비 문구가 찍히거나 프로세스가 끝날 때까지, 길어야 4번의 대기 시간만큼 들고 있다가 보낸다. 준비 전에 받은 명령을 실패하는 서버가 있어서다(minecraft 26.3은 `An unexpected error occurred while trying to execute that command`만 찍고 계속 돈다). 테스트 `a_stop_while_starting_waits_for_the_ready_line`.
  3. 그 밖(REST 전용 게임, stdin 없는 스텁)이면 모듈 `stop()`에 `pid`, `managed: true`, `force: false`를 넘긴다. 모듈이 `{success:false, error_code:"graceful_stop_unavailable"}`로 답하면 기다리거나 강제 종료하지 않고 그 응답을 돌려준다(아래).
  4. 0.5초 간격으로 `is_running`을 확인한다. 기다리는 시간은 graceful이면 인스턴스 설정 `stop_timeout`, 모듈 `stop_timeout`, 기본 30초 순이고, force면 5초다.
  5. 시간이 지나면 강제 종료하고 2초 더 기다린다.
  6. `managed_store`에서 제거하고 종료를 판정·기록하고, `managed.pid`를 삭제하고, 30초짜리 stop cooldown(`STOP_COOLDOWN_SECS`)을 등록한다.
  - palworld는 stdin이 없어 3번 경로로 간다. 모듈 `stop()`이 REST로 저장·종료를 요청한다 (`M:palworld/lifecycle.py:399-426`).
- **non-managed stop**: 추적 중인 PID가 있을 때만 Python `stop()`에 `pid`와 `force`를 넘긴다. 모듈이 REST나 kill을 직접 처리한다. `success=false`이면 Err가 된다 (`mod.rs:1056-1099`). minecraft·zomboid는 일반 정지에서 바로 죽이지 않고 콘솔 정지(콘솔이 없을 때 불리므로 보통 실패, §6.3 #8) → POSIX면 SIGTERM → 30초 대기 → 강제 종료 순서를 밟는다. Windows처럼 보낼 정상 종료 신호가 없으면 끝내지 않고 `graceful_stop_unavailable`로 답한다(아래).
- **저장 후 종료를 요청할 길이 없을 때** (MK10): 콘솔도 REST도 없는 서버(runner 없이 다시 붙은 서버 등)에서 `force: false`인 `stop()`은 서버를 끝내지 말고 `{success:false, error_code:"graceful_stop_unavailable", message}`를 돌려준다(message는 모듈 locales). 데몬은 두 경로(managed 3번, non-managed) 모두 서버를 그대로 두고 HTTP 200 `{success:false, server, error_code:"graceful_stop_unavailable", error}`로 답한다. 클라이언트는 저장 없이 끝난다는 것을 알리고 사용자가 고르면 `force: true`로 다시 요청한다. 예약 작업(재시작·업데이트)은 강제 종료하지 않고 그만둔다.
- **extension 위임**: `server.stop` hook은 정지 전에 불린다. `handled`를 돌려주면 그대로 반환하고, 정지를 맡을 익스텐션이 실패하면 실패를 돌려준다 (`mod.rs:908-933`).

### 3.5 명령 경로 (stdin/REST/RCON)

- 데몬의 프로토콜 계층에는 REST(`rest.rs`, `client.rs`)와 RCON만 받는 모듈용 RCON 클라이언트(`rcon.rs`)가 있다.
- **명령 해석**: `POST /api/instance/:id/command`가 모든 인터페이스의 단일 경로다. `crate::commands`가 module.toml 정의로 이름·별명·입력을 해석하고 `method`에 따라 콘솔·REST·모듈 `command()`로 보낸다. 정의에 없는 명령은 한 줄 그대로 콘솔로 간다 (`D:src/commands/mod.rs:1-17`, `command.rs:11-`).
- **콘솔 경로**: `/api/instance/:id/rcon`과 `/stdin`은 서버 콘솔(stdin)에 쓰고, 콘솔 출력을 잠깐 모아 답으로 돌려준다. `/rcon`은 콘솔이 없고 모듈이 RCON만 받으면 RCON으로 보낸다. 둘 다 없으면 409 `no_command_path`, 서버가 입력을 읽지 않아 대기열이 차 있으면 503 `console_input_full` (위 RCON 정책).
- **REST 경로**: `/api/instance/:id/rest`. `get_credentials`(캐시)로 게임 설정 파일의 계정을 읽는다. Basic Auth의 username 기본값은 `"admin"`이다.
- **RCON 끄기**: minecraft는 모든 모드와 설치 때 `enable-rcon=false`를 강제한다 (`M:minecraft/lifecycle.py:526, 990-995, 1712`). zomboid는 `RCONPassword`를 비운다 (`M:zomboid/lifecycle.py:817-827`). palworld는 실행 전과 `configure()` 저장마다 `RCONEnabled=False`를 쓴다 (`M:palworld/lifecycle.py:256-258, 933`).
- `SupervisorError`(`error.rs`)는 지워졌다. 핸들러는 `anyhow` + 수동 JSON을 쓴다.

### 3.6 크래시 감지와 자동 재시작

- **크래시 판정** (`D:src/supervisor/crash.rs`): 데몬이 정지를 보냈으면 `user_stop`, 메모리 상한을 넘어 러너가 끝냈으면 `crash`(근거 `memory limit N MiB reached`, 아래 메모리 상한), 예외 종료 코드·크래시 시그널·이번 실행 중 생긴 `crash_artifacts` 파일이 있으면 `crash`, 콘솔 끝 50줄에 `graceful_exit_patterns`가 있으면 `in_game_stop`, 시작 뒤 30초(`STARTUP_GRACE`) 안에 끝나면 `start_failed`(근거는 걸린 시간과 콘솔 마지막 줄, 포트 충돌 같은 시작 실패. 데몬 재시작 뒤 PID로 다시 붙은 서버는 시작 시각을 모르므로 빼고, 러너로 다시 붙은 서버는 러너가 띄운 시각을 쓴다), 종료 코드 0이면 `in_game_stop`, 그 밖은 `unknown_exit`다. 흔적이 없으면 크래시로 보지 않는다.
- **자동 재시작**: `auto_restart`(기본 켬)가 켜져 있으면 `crash`를 재시작한다. `restart_on_unknown_exit`(기본 끔)까지 켜면 `unknown_exit`도 재시작한다. `start_failed`는 다시 켜도 같은 이유로 실패하므로 재시작하지 않는다. 첫 대기 5초에서 두 배씩 늘고, 10분 안에 3번을 넘으면 포기한다 (`crash.rs:22-33`, `mod.rs:2285-2305`). 데몬 루프가 예약을 가져가 대기 뒤 `POST /api/instance/:id/start`와 같은 경로로 시작한다 (`D:src/main.rs:456-461`, `server.rs:854-870`). 대기하는 동안 정지 요청이나 시작이 있었으면(인스턴스별 실행 세대가 예약 때와 다르면) 그만둔다. 판정과 재시작 여부는 콘솔에 System 줄로 남는다.
- `monitor_processes`(`mod.rs:2435-2615`)가 하는 일은 다음과 같다.
  1. `managed_store.cleanup_dead()`로 죽은 항목을 꺼내 종료를 판정하고, `managed.pid`를 삭제한 뒤 cooldown을 등록한다.
  2. 추적 중인 PID가 스냅샷에 없으면 untrack한다. saba-runner가 아직 서버를 돌리면(런처가 끝나고 실제 서버가 남은 경우) runner가 보는 PID로 옮긴다. runner는 띄운 프로세스가 30초 안에 **정상 종료(코드 0)**하고 job에 프로세스를 남겼을 때만 남은 것을 서버로 따라간다. 실패 코드로 끝났으면(예: `0xC0000135` DLL 없음) 시작 실패로 끝낸다.
  3. `auto_detect`가 켜진 인스턴스는 `process_name`(+`cmd_patterns`)으로 외부 프로세스를 찾고, 실행 파일·작업 폴더·명령줄 경로가 이 인스턴스 폴더(사용자 서버 폴더, 실행 파일 폴더, 인스턴스 폴더) 아래에 있는 것만 추적한다. 같은 모듈 인스턴스가 하나뿐이어도 폴더가 맞지 않는 같은 이름 프로세스(다른 데이터 폴더의 `java.exe` 등)는 잡지 않고, 여럿이면 더 깊은 폴더 쪽이 주인이다(`process_monitor::pick_owned_process`). 포트가 충돌하면 그 프로세스를 **강제 kill**하고 `PortConflictStopEvent`를 쌓는다.
- **메모리 상한** (`memory_limit_mb`, 인스턴스 레코드, MiB): 비우면 제한 없음. 데몬이 saba-runner에 `--memory-limit <MiB>`로 넘기고, 러너는 Windows Job에 `JOB_OBJECT_LIMIT_JOB_MEMORY`를 건다(서버와 그 자식의 커밋 메모리 합, `runner/src/job.rs`). 넘으면 할당이 실패하고 Job이 완료 포트로 알린다. 러너는 그때 Job을 끝내고(종료 코드 8) `exit.json`에 `memory_limit_mb`를 남긴다. 데몬은 이것을 `crash`로 판정하므로 `auto_restart`면 다시 켠다. Java 서버면 상한을 `-Xmx`보다 1~2GB 크게 잡는다(힙 밖 메모리). Windows만 적용하고, 다른 OS나 러너 없이 띄운 서버는 로그에 한 줄 남기고 제한 없이 돈다. 원격(디스코드·챗봇) 설정 변경으로는 바꿀 수 없다.
- `Supervisor::monitor()` stub은 지워졌다.
- `process_name`은 Windows 값(`java.exe`)이고 Linux용 override가 없다. 이름 비교는 부분 문자열(`"java".contains("java.exe")`는 거짓)이라 **Linux에서 minecraft와 zomboid의 auto-detect는 매치에 실패한다** (`D:src/process_monitor.rs:122-127`).

---

## 4. 모듈별 기능 요약

공통 사항:
- 모든 모듈은 version 0.1.0, author WareAoba다.
- `locales/` 아래에 10개 언어 JSON이 있다: `de, en, es, fr, ja, ko, pt-BR, ru, zh-CN, zh-TW`.
- `[aliases]`(한국어 별칭), `[errors]`(사용자용 오류 문구), `[backup]`, `config.announce_command`가 있다. 모두 `schema_version = 1`이다.

### 4.1 minecraft (`M:minecraft/`, lifecycle 2,337줄, toml 1,303줄)

| 항목 | 내용 |
|---|---|
| protocols | `supported=["stdin"]`, default stdin, `interaction_mode=console` |
| 서버 종류 | 설정 `server_type`(`install = true`): vanilla, paper, folia, purpur, pufferfish, spigot, craftbukkit, fabric, quilt, forge, neoforge. 판정·설치 소스·실행 대상은 `M:minecraft/server_types.py`의 표(`SERVER_TYPES`, versions.list 접두사, Main-Class, 폴더 흔적)만 읽는다. 판정은 jar 안의 `META-INF/versions.list`(번들된 서버 jar 이름)와 MANIFEST Main-Class, 그다음 폴더 흔적(`config/paper-global.yml`, `purpur.yml`, `.fabric/`, `libraries/net/neoforged/neoforge/<버전>/` 등)으로 한다. 설치는 vanilla(Mojang), paper·folia(PaperMC Fill v3, 안정 채널, sha256 검사), purpur(Purpur API, md5), fabric(Fabric meta 서버 런처 → `fabric-server-launch.jar`)만 되고 나머지는 가져오기로 추가한다. 1.17+ Forge/NeoForge는 `@libraries/…/<버전>/win_args.txt`(또는 unix)로 띄운다. 고른 종류와 서버 파일이 다르면 `validate`가 `SERVER_TYPE_MISMATCH` 경고를 낸다 |
| 플러그인·모드 | `M:minecraft/plugins.py`가 `plugins/*.jar`·`mods/*.jar`의 `paper-plugin.yml`·`plugin.yml`·`fabric.mod.json`·`quilt.mod.json`·`META-INF/(neoforge.)mods.toml`을 읽어 판정한다(파일 이름은 보지 않는다, `.jar.disabled`는 꺼짐). `instance_commands()`가 plugin.yml에 선언된 명령을 자유 입력 한 칸(`{args}`)으로, `M:minecraft/plugin_catalog/<slug>.json`(플러그인 id `match`가 키, 공식 문서·소스 기준)의 명령을 입력 칸까지 준다. 한국어 문구는 `plugin_catalog/locales/ko.json`, 없으면 데이터의 영어. 카탈로그 22종 740개(EssentialsX·Spawn·Chat·Discord, LuckPerms, WorldEdit, WorldGuard, Multiverse Core·Portals·NetherPortals·Inventories, CoreProtect, GriefPrevention, Geyser, Floodgate, ViaVersion, PlaceholderAPI, Vault, spark, Chunky, ChunkyBorder, Citizens). 콘솔에서 못 쓰는 명령은 각 파일 `player_only`에 이름만 적는다 |
| 설치 | `install.method="download"`. `ServerInstaller`(`:1480-1750`)가 Mojang `version_manifest_v2`(`:1477`)에서 버전 목록을 받아 jar를 내려받는다. Java는 `JavaDetector`와 `JavaDownloader`(Adoptium API `:284`)로 감지하거나 `<working_dir>/java/`에 번들로 받는다 |
| 실행 | `get_launch_command`가 `java -Xmx/-Xms{ram}`, 선택적 Aikar flags, `jvm_args`, 실행 대상(`server_types.launch_target`: `-jar <서버 jar> nogui` 또는 Forge/NeoForge 인자 파일)을 만든다. 시작 전에 `enable-rcon=false`를 강제한다(모든 모드) |
| 설정 | `ServerPropertiesManager`가 `server.properties`를 읽고 쓴다. 모듈 폴더에 동봉된 `server.properties`를 기본값으로 쓴다 (`_load_default_properties` `:507`). settings 66개(saba-chan 5 / advanced 40 / basic 21) |
| 명령 | `[commands] hook = true`(설치된 플러그인·모드 명령은 위 행). module.toml에는 17개, 전부 `method="console"`(콘솔 stdin): say, give, save-all, list, weather, difficulty, gamemode, time, tp, kick, ban, pardon, op, deop, whitelist, seed, stop |
| 정지 | managed는 데몬이 stdin으로 `stop`을 보낸다. 모듈 `stop()`은 force가 아니면 콘솔 `stop`(`instance_id` 필요) → SIGTERM(POSIX) 순서로 요청하고, non-managed면 `stop_timeout`만큼 기다린 뒤에만 `force_kill`한다. 콘솔 없이 다시 붙은 managed 서버(`managed:true`)면 요청만 보내고 바로 돌려준다(기다림과 강제 종료는 데몬). 둘 다 보낼 수 없으면(콘솔 없는 Windows) 끝내지 않고 `graceful_stop_unavailable`로 답한다(§3.4). ~~RCON `stop` 시도 후 실패하면 바로 강제 종료~~ → 해결. `command()`도 `daemon_stdin_command`로 콘솔에 보낸다 (`:1425-`) |
| 기타 | `EulaManager`(accept_agreements), `ErrorDetector`(diagnose_log), `MinecraftPing`(SLP 기반 status), `get_installed_version`(jar 내부 `version.json` 파싱), syntax_highlight 규칙 40개(Vanilla·Forge `[시각] [스레드/레벨]`과 Paper 콘솔 `[시각 레벨]`을 모두 받는다). 색은 모두 GUI 팔레트 토큰이다(`saba-chan-gui/src/utils/syntaxHighlight.js:24-41`). ~~팔레트에 없는 `notice`·`critical`을 써서 색이 안 나왔다~~ → 해결: `info`·`error`로 바꿨다 (zomboid의 `duration`도 `number`로) |
| 미등록 | `get_credentials`만 없다(RCON 값만 돌려주던 것이라 지웠다). `import_settings`, `reset_properties`, `reset_server`는 등록됨 |
| 동봉 파일 | `_sample_log.txt`, `test_lifecycle.py`가 리포에 있고 zip에도 들어간다. 패키저는 `__pycache__`·`tests` 폴더와 `.pyc`만 뺀다 (`D:scripts/versions.py:196-208`) |

### 4.2 palworld (`M:palworld/`, lifecycle 1,637줄, toml 1,351줄)

| 항목 | 내용 |
|---|---|
| protocols | `supported=["rest"]`, `interaction_mode=commands`, `credential_map.rest_password="AdminPassword"` |
| 설치 | `steamcmd`, app_id 2394010, `requires_extensions=["steamcmd","ue4-ini"]`. `extensions.steamcmd.SteamCMD`를 사용한다 |
| 실행 | `--port=<port>`. 실제 서버 `Pal/Binaries/Win64/PalServer-Win64-Shipping-Cmd.exe`가 있으면 런처 대신 그것을 직접 띄운다(`:322-331`). `[launch] console = "pty"`. 시작 전 `_enforce_rest_policy`로 INI에 RESTAPIEnabled와 AdminPassword를 보장하고, `AdminPassword`를 최상위로 돌려주어 데몬이 rest_password를 동기화하게 한다 (`:216-346`) |
| 정지 | stdin이 없어 데몬이 모듈 `stop()`을 부른다. REST로 저장·종료를 요청하고 60초 안에 안 끝나면 강제 종료한다 (`:399-426`). REST를 쓸 수 없으면 끝내지 않고 `graceful_stop_unavailable`로 답한다(§3.4) |
| 설정 | `PalWorldSettings.ini`의 `OptionSettings=(...)`를 **`extensions.ue4_ini`** 익스텐션으로 파싱한다. 동봉된 `DefaultPalWorldSettings.ini`가 기본값이다. settings 117개. 목록 값 키(`_LIST_KEYS`, 지금은 `DenyTechnologyList`)는 화면의 `PALBOX,RepairBench`를 INI에 `("PALBOX","RepairBench")`로 쓰고 읽을 때 쉼표 형태로 되돌린다 (`:884-904, 927-928, 975-977`). `configure()`는 저장마다 `RCONEnabled=False`를 쓴다 (`:933`) |
| 명령 | 10개, 전부 `method="rest"`(`/v1/api/...`): announce, info, players, metrics, settings, save, shutdown, kick, ban, unban. `resolve_player_id`가 닉네임을 SteamID로 해석한다 (`:112`). `protocol_mode`와 상관없이 늘 REST로 보낸다. ~~`protocol_mode=rcon`이면 데몬 `/rcon`으로 보내는 RCON 명령 경로~~ → 해결: 지워졌다 (`:1224-`) |
| 의존성 | `extensions.ue4_ini`는 `try`로 불러온다(`:33-36`). 익스텐션이 없으면 모듈은 로드되고, INI를 만지는 함수만 `extension_required`를 돌려준다 (`:742-757, 1494-1502`) |
| 미등록 | `get_installed_version` |
| 기타 | `list_versions`는 Steam 단일 버전 |

### 4.3 zomboid (`M:zomboid/`, lifecycle 1,610줄, toml 1,580줄)

| 항목 | 내용 |
|---|---|
| protocols | `supported=["stdin"]`, console, `stop_command="quit"`. `RCONPassword`를 비워 RCON을 끈다 |
| 설치 | `steamcmd`, app_id 380870. steamcmd 익스텐션이 켜져 있지 않으면 `extension_required`를 돌려준다(`:1255-1261`). 익스텐션을 쓰고, 없으면 시스템 steamcmd(`_find_steamcmd`)로 대체한다 |
| 실행 | `StartServer64.bat`나 `start-server.sh`를 **파싱**해서(`_parse_bat`, `_parse_sh`, `_parse_java_cmdline` `:607-741`) 번들 Java, classpath, native lib을 찾는다. 그다음 `java ... zombie.network.GameServer -servername <name> -cachedir=...`를 직접 구성한다 |
| 설정 | `IniManager`가 `Zomboid/Server/<servername>.ini`를 다룬다. `_INI_KEY_MAP`(saba 키 → INI 키). settings 117개. `[backup] hook = true`라 `backup_paths()`가 세이브 위치를 계산한다 |
| 명령 | 23개, 전부 `console`(콘솔 stdin): servermsg, players, save, kick, banuser, unbanuser, adduser, setaccesslevel, teleport, teleportto, createhorde, godmod, invisible, startrain, stoprain, chopper, gunshot, additem, addxp, showoptions, reloadoptions, changeoption, quit |
| 정지 | managed는 데몬이 stdin으로 `quit`을 보낸다. 모듈 `stop()`(non-managed, stdin 없는 managed)은 force가 아니면 콘솔 `save`→`quit`(`instance_id` 필요) → SIGTERM(POSIX) 순서로 요청하고, non-managed면 `stop_timeout`만큼 기다린 뒤에만 강제 종료한다. managed(`managed:true`)면 요청만 보내고 바로 돌려준다(기다림과 강제 종료는 데몬). 둘 다 보낼 수 없으면(콘솔 없는 Windows) 끝내지 않고 `graceful_stop_unavailable`로 답한다(§3.4). PID 확인·종료는 공용 헬퍼를 쓰고, `status()`는 PID 생존만 본다 (`:1020-1092`). ~~예전 버그(managed 기본 True, `input` 키, 토큰 없음)~~ → 해결. ~~`tasklist`·`os.kill`로 직접 확인·종료, RCON 폴백(stop·status·command)~~ → 해결: 지워졌다 |
| 미구현 | `list_versions`, `get_version_details`, `get_installed_version`, `reset_properties` |

### 4.4 __example (`M:__example/`)

- 파일명이 `__example__module.toml`, `__example__lifecycle.py`다. 로더는 `module.toml`과 `entry`(여기서는 `lifecycle.py`)를 요구하므로 **이름을 바꿔야** 동작한다. 폴더명도 `_`로 시작해 빌드에서 제외된다(의도된 것).
- `name = "mygame"`인데 폴더명은 `__example`이다. toml 주석은 "폴더명과 동일"을 요구한다(예시 값).
- 템플릿 주석에 있는 "데몬 호출 시점" 설명(`:36-50`)과 config 키 설명(`:56-75`)은 §2.3과 같은 이유로 **사실과 다르다**(예: `instance_id`·`settings` 항상 포함). `rcon_port`·`rcon_password` 예시는 지워졌다.
- RCON 대신 stdin을 가르친다: `protocols.supported = ["stdin"]`, RCON 설정·자격증명 예시 삭제, 공용 `daemon_stdin_command`·`wait_for_exit`·`force_kill`을 쓰고 대기 후에만 강제 종료한다 (`__example__lifecycle.py:92-97, 236-256`, `__example__module.toml:58-60`). 진입점은 공용 `run_lifecycle()`이고, 주석의 명령 방식도 `method="console"`로 맞췄다(e1d7743). `stop()`의 콘솔 정지는 콘솔이 없을 때 불리므로 보통 실패하고 대기 후 강제 종료로 넘어간다(§6.3 #8).
- `install_server`는 실제 모듈처럼 `extensions.steamcmd.SteamCMD` 클래스를 쓴다 (`:394-395`).
- `get_credentials`, `read_properties`가 없다. 명령은 stdin으로 가므로 콘솔 모듈에는 지장이 없지만, REST 모듈을 만들면 `get_credentials`를 직접 추가해야 한다.
- settings에 `managed_start`와 `graceful_stop`이 있다. `managed_start`는 데몬이 시작 경로를 고르는 데 쓴다(§3.1). `graceful_stop`은 데몬도 예시 `stop()`도 읽지 않고(설정 동기화에서 빼기만 한다, `instance.rs:1293`), GUI만 정지 방식을 고를 때 읽는다 (`saba-chan-gui/src/hooks/useServerActions.js:618`).

### 4.5 i18n과 locales

- 모듈 측: `I18n(MODULE_DIR)`이 `locales/<SABA_LANG>.json`을 읽고, 없으면 `en`으로 대체한다. 키 구조는 `module`, `settings`, `commands`, `presets`, `queries`, `agreements`, `errors`, `messages`, `rcon`, `reset` 등이다(minecraft ko.json 기준). `queries.<name>.label`·`description`과 `[errors]` 코드의 `errors.<code>`는 데몬도 읽는다: 액션 목록(`GET /api/actions?instance=`)과 heartbeat `actions`의 조회 제목·설명(`actions::module_locale`, `<모듈 폴더>/locales/<lang>.json`). `reset.label`·`reset.description`은 서버 초기화(`reset_server`)가 무엇을 지우고 남기는지 GUI 초기화 화면에 보여 주는 문구다(모듈마다 다르다: minecraft는 JAR·eula.txt만 남김, palworld는 설정만 기본값으로, zomboid는 월드 세이브만 지움).
- 인터페이스 측: `GET /api/module/:name/locales`가 `<modules_dir>/<name>/locales/*.json`을 하나의 맵으로 묶어 돌려준다 (`D:src/ipc/handlers/config.rs:200-234`). GUI는 이를 `mod_<name>` 네임스페이스로 쓴다. 이 경로 계산은 `module.path`가 아니라 **모듈 이름으로 만든 경로**라서, ZIP 모듈(`.extracted/<name>`)이나 폴더명이 name과 다른 모듈에서는 locales가 비어 나온다.

---

## 5. 패키징과 배포

### 5.1 릴리즈에 싣기 (`D:scripts/versions.py manifest`, `D:.github/workflows/release.yml`)

- 모듈 저장소에는 릴리즈 워크플로가 없다. 검사용 `M:.github/workflows/check-modules.yml`(스키마·적합성 검사)만 있다.
- 코어 릴리즈 워크플로가 `saba-chan-modules`의 `main`을 checkout하고(`release.yml:175-182`), `versions.py manifest --modules-repo`가 모듈마다 `module-<name>.zip`을 만든다(`versions.py:217-245`, `_`·`.`로 시작하는 폴더는 뺀다). 버전은 각 `module.toml`의 `[module].version`, 목록 화면용 정보(`display_name`, `description`, `game_name`, `requires_extensions`, 아이콘 URL)는 `meta`에 들어간다.
- 이 zip들은 코어·GUI·CLI와 **같은 릴리즈 manifest**(`components`의 `module-<name>` 키)에 실려 서명되고, R2 `releases/<version>/`에 올라간다. 그래서 모듈 목록과 버전은 릴리즈 단위로 고정된다.

### 5.2 데몬의 목록·설치

- **`GET /api/modules/manifest`**: `relay::latest_catalog`로 최신 릴리즈 manifest를 받아 `module-*` 항목을 `{modules:{<이름>:{version, sha256, size_bytes, ...meta}}}`로 돌려준다 (`D:src/ipc/handlers/server.rs:502-522`).
- **`POST /api/modules/manifest/:id/install`** (`server.rs:525-590`):
  1. `id`에 `..`, `/`, `\`가 있으면 거부한다.
  2. manifest의 `module-<id>`가 없으면 실패한다.
  - 설치된 모듈 버전이 manifest 버전보다 높으면 받지 않는다(`version::is_downgrade`). 릴레이가 예전 서명 manifest를 다시 보내 옛 버전을 깔게 하는 것을 막는다.
  3. R2 URL에서 `<modules>/.tmp-module-<id>.zip`로 받으며 SHA-256을 확인한다(`download_verified`).
  4. `.staging-<id>`에 풀고 데몬과 같은 규칙(`load_module_dir`: `module.toml` 파싱, 진입 파일, `saba-core` 요구 버전)으로 읽힐 때만 기존 `<modules>/<id>`와 바꾼다(`install_staged_module`). 기존 폴더는 `.old-<id>`로 옮겨 두었다가 새 폴더가 자리를 잡으면 지우고, 옮기다 실패하면 되돌린다. 모듈 탐색은 점으로 시작하는 항목을 건너뛰고, 데몬은 기동할 때 모듈·익스텐션 폴더의 `.staging-*`·`.old-*`·`.tmp-*`를 지운다(`utils::remove_install_leftovers`, 설치 도중 끝났을 때 남은 것). 단일 실행 잠금을 못 잡고 계속 가는 경우에는 정리하지 않는다. 또 `.old-<id>`가 있는데 `<id>`가 없으면(옛 폴더를 옮긴 뒤 새 폴더를 두기 전에 끝났다) 지우지 않고 `<id>`로 되돌린다. 그래서 업데이트해도 지워진 파일이 남지 않는다.
  - manifest의 `requires`(예: `{"saba-core": ">=0.1.10"}`)를 못 맞추면 받지 않고 `{ok:false, error, error_code:"module_dependency_unsatisfied", module, missing_dependencies:["saba-core >=0.1.10 (installed 0.1.9)"]}`로 답한다(로드 실패·시작 응답과 같은 꼴, `unmet_requires_body`).
  - 실패도 HTTP 200 `{ok:false, error}`로 돌려준다.
- **신뢰 표시 알림**: 서명된 카탈로그에서 받지 않았거나(`unverified`) 받은 뒤 바뀐(`modified`) 모듈로 인스턴스를 만들면 `POST /api/instances` 응답에 `module_trust`와 `warnings[{code, module}]`가 실린다. 알림일 뿐 만들기·실행은 막지 않는다(`server.rs` `trust_warnings`). CLI는 경고 줄로 보인다.
- **`DELETE /api/modules/:id`**: `..`·`/`·`\`가 든 `id`는 거부하고, `<modules>/<id>`를 `remove_dir_all`로 지운다. 여기서 `id`는 **디렉터리명**이다 (`server.rs:593-617`).

### 5.3 업데이터

- `check_for_updates`가 같은 manifest에서 **설치된 모듈만** 골라 버전을 비교한다(`D:updater/src/lib.rs:482-557`). 새 모듈 설치는 목록 화면(§5.2)의 몫이다.
- 무결성 검사는 업데이터가 적용할 때 zip에서 푼 파일마다 기록한 SHA-256(`<data_dir>/installed-files.json`)과 디스크 파일을 비교한다. manifest의 zip 해시와는 비교하지 않는다 (`D:updater/src/integrity.rs:1-6, 105-152, 235-239`). 설치기로 처음 깐 모듈은 기록이 없어 결과에 나오지 않는다.
- **installer**: 모듈 목록도 같은 manifest의 `module-*`에서 만든다(`installer/gui/src-tauri/src/lib.rs` `get_available_modules`, `installer/core/src/release.rs` `modules`). 하드코딩 폴백은 없고, 받지 못하면 화면이 알린다.

---

## 6. 불일치, 죽은 코드, 미완성, 원칙 위반

### 6.1 로더와 모듈·문서 사이의 불일치
1. ~~`[module].entry`가 쓰이지 않는다~~ 해결됨: 데몬이 `entry_path()`로 쓰고 로드 때 파일 존재를 확인한다 (§1.1).
2. ~~command `inputs`의 `options`, `min`, `max`가 버려진다~~ → 해결: `CommandInputToml`·`CommandInput`이 받고 데몬이 입력 검사에 쓴다 (`module_loader.rs:373-390, 953-972`).
3. ~~`[detection.linux]`, `install.beta_branch`, `detection.common_paths`, `config.*` 추가 키, `[errors]`, `game_name`, `display_name`이 무시된다~~ → 해결: schema v1이 모르는 키를 로드 오류로 내고, 모듈에서 그 키들이 지워졌다. `[errors]`·`game_name`·`display_name`은 메타데이터에 실린다 (`module_loader.rs:1193-1246`).
4. ~~`CommandField.method` 주석은 `both`인데 인터페이스 분기는 `dual`이다~~ → 해결: 데몬 `crate::commands`가 `console`/`rest`/`module`(`rcon`·`dual`·`both` 별칭)을 해석하고 인터페이스는 분기하지 않는다 (`D:src/commands/mod.rs:167-191`).
5. ~~`/api/modules` 응답에 `game_name`과 `display_name`이 없다~~ → 해결: `ExtensionInfo`에 둘 다 있다 (`D:src/ipc/mod.rs:246-249`).
6. ~~ZIP 로딩에서 `ends_with("module.toml")`로 첫 엔트리를 고른다~~ → 해결: 이름이 정확히 `module.toml`인 가장 얕은 엔트리를 고른다 (`module_loader.rs:1559-1570`). `.extracted` 재추출 문제와, locales·설치 경로가 `<modules>/<name>`을 가정하는 문제는 남았다 (§1.1, §4.5).

### 6.2 데몬 측 죽은 코드와 stub
| 대상 | 위치 |
|---|---|
| ~~`state_machine.rs` 전체~~ → 해결: 파일이 지워졌다 | — |
| ~~`SupervisorError` 전체~~ → 해결: `error.rs`가 지워졌다 | — |
| ~~`ProcessTracker::get_status`, `mark_crashed`, `terminate`, `ProcessStatus::Crashed`~~ → 해결: 지워졌다 | `process.rs` |
| ~~`Supervisor::monitor()` stub, `run()`~~ → 해결: 지워졌다 | `mod.rs` |
| ~~`ModuleMetadata::steam_app_id`, `has_extension_config`~~ → 해결: 지워졌다 | `module_loader.rs` |
| ~~`ModuleLoader.get_module`의 잘못된 `#[allow(dead_code)]`~~ → 해결: 속성이 없다 | `module_loader.rs:1616` |
| ~~`LoadedModule.is_zip` 미사용~~ → 해결: 필드가 지워졌다 | `module_loader.rs:455-458` |
| ~~`plugin::run_extension*`~~ → 해결: 지워졌다 | `plugin/mod.rs` |
| ~~`ManagedProcess::wait_for_exit`, `subscribe`~~ → 해결: broadcast 채널과 함께 지워졌다 | `managed_process.rs` |

### 6.3 모듈 측 미완성과 버그
1. ~~FUNCTIONS 누락: minecraft의 `import_settings`, `reset_properties`, `reset_server`, palworld의 `import_settings`~~ → 해결: 모두 등록됐다 (§2.4).
2. ~~모든 모듈의 `start()`가 죽은 코드다~~ → 해결: 모든 모듈에서 지워졌다.
3. ~~모듈→데몬 HTTP 콜백이 인증 토큰을 보내지 않고 포트도 57474로 하드코딩돼 있다~~ → 해결: `daemon_post`가 `SABA_TOKEN_PATH`의 토큰을 `X-Saba-Token`으로 보내고 주소는 `SABA_IPC_URL`을 쓴다. palworld·zomboid도 이것을 쓴다 (`D:modules/daemon_rcon.py:36-82`).
4. ~~zomboid `stop`의 stdin 콜백은 payload 키가 틀리고(`input` vs `command`), managed 판정도 틀렸다~~ → 해결: 공용 `daemon_stdin_command`(`{"command": ...}`)를 쓰고 `managed` 분기는 없어졌다 (`M:zomboid/lifecycle.py:1052-1092`).
5. ~~palworld가 최상위에서 `extensions.ue4_ini`를 import해서 익스텐션에 하드 의존한다~~ → 해결: `try`로 불러오고 없으면 해당 함수만 `extension_required`를 돌려준다 (`M:palworld/lifecycle.py:33-36, 742-757`).
6. ~~`__example`의 `install_server`가 쓰는 steamcmd API(`install_game`)가 실제 모듈과 다르다~~ → 해결: `SteamCMD` 클래스를 쓴다 (`M:__example/__example__lifecycle.py:394-395`).
7. ~~`get_installed_version`의 `jar_name` 기본값이 `"server.jar"`다~~ → 해결(MK7): `validate`와 같은 config를 받는다.
8. ~~데몬의 `stop` config에 `instance_id`가 없어 모듈 `stop()`의 콘솔 정지가 실행되지 않는다~~ → 해결: 모듈 호출에 `instance_id`가 들어간다(`D:src/supervisor/mod.rs:494-495`, 59a93b8). 정지 경로는 이렇다. 콘솔이 있는 managed 서버는 데몬이 `stop_command`를 stdin으로 보내고 끝나기를 기다린다(`mod.rs:949-969`, 테스트 `normal_stop_asks_the_console_and_waits`). 모듈 `stop()`은 콘솔이 없을 때(non-managed, REST 전용, stdin 없는 재부착)만 불리므로, 그 안의 콘솔 정지 시도는 보통 실패하고 SIGTERM(POSIX)이나 대기 후 강제 종료로 넘어간다.

### 6.4 데몬 프로토콜과 운영 문제
1. ~~`SABA_ENABLED_EXTENSIONS`의 상태 파일 경로가 `ExtensionManager`와 다르다~~ → 해결: 둘 다 `<data_dir>/config.json`의 `extensions.enabled`를 읽는다 (`plugin/mod.rs:315-322`).
2. ~~데몬이 `SABA_LANG`과 `DAEMON_API_URL`을 주입하지 않는다~~ → 해결: `SABA_IPC_PORT`, `SABA_IPC_URL`, `SABA_TOKEN_PATH`, `SABA_LANG`을 넣는다 (`plugin/mod.rs:18-38`). 이름이 `DAEMON_API_URL`이 아니라 `SABA_IPC_URL`이라 모듈 문서와 다르다 (§2.1).
3. ~~`/api/module/:name/install` 경로의 타임아웃이 120초다~~ → 해결: 1800초 (`plugin::INSTALL_TIMEOUT_SECS`).
4. ~~non-managed는 이름으로, 나머지는 ID로 추적한다~~ → 해결: non-managed도 인스턴스 ID로 추적한다 (`mod.rs:846-847`).
5. ~~`managed/start`가 write lock을 잡은 채로 Python을 호출한다~~ → 해결: 시작은 단계마다 락을 짧게 잡고 Python은 락 밖에서 부른다 (`managed.rs:67-68`, C12).
6. ~~`server.post_stop` hook을 정지 전에 dispatch한다~~ → 해결: hook 이름이 `server.stop`이고 정지 전에 부른다고 명시돼 있다 (`mod.rs:908-911`).
7. ~~managed 모드에서는 RCON을 끄는데, 명령은 RCON으로 라우팅된다~~ → 해결: 데몬은 RCON을 쓰지 않고 명령은 모두 stdin으로 간다(RCON 정책).
8. ~~RCON/REST 명령 하나마다 `get_credentials` Python 프로세스를 띄운다~~ → 해결: REST만 쓰고 결과를 캐시한다(`command.rs:660-687`).
9. ~~모듈 설치에 sha256 검증, 경로 검증, 클린 설치가 없고, 업데이터의 버전 판정과 무결성 해시 대상이 맞지 않는다~~ → 해결: 설치는 SHA-256 검증·id 검사·staging 교체를 하고(`server.rs:525-590`), 버전은 manifest의 semver로 비교하며(`D:updater/src/lib.rs:482-557`), 무결성은 설치 때 기록한 파일별 해시를 본다 (`D:updater/src/integrity.rs:1-6`).

### 6.5 [원칙 위반] 모듈 지식이 데몬 바깥이나 데몬 하드코딩에 있는 곳

| # | 위치 | 내용 | 원칙 |
|---|---|---|---|
| 1 | ~~GUI·봇의 명령 실행 로직~~ | ~~`method` 분기, 템플릿 치환, 입력 검증을 인터페이스가 중복 구현~~ → 해결: `POST /api/instance/:id/command` 하나로 모이고 데몬 `crate::commands`가 해석한다. GUI는 그 경로만 부른다 (`D:src/commands/mod.rs:1-17`, `saba-chan-gui/src/api/createApi.js:479-485`). 로컬 Discord 봇은 지워졌다 | — |
| 2 | ~~GUI·봇의 시작 경로 선택~~ | ~~인터페이스가 `/managed/start`와 `/server/:name/start`를 고른다~~ → 해결: `POST /api/instance/:id/start`에서 데몬이 고른다 (`server.rs:649-710`, `saba-chan-gui/src/api/createApi.js:133-135`) | — |
| 3 | — | ~~`dir_signatures` 매칭을 GUI 마이그레이션 화면이 직접 한다~~ → 해결: `migrationScanDir`가 데몬 `POST /api/modules/detect`(`server.rs:423-443`)를 부르고 화면은 그 `matches`만 쓴다(`saba-chan-gui/src/api/createApi.js:825-833`, `AddInstanceMigration.js:75-76`) | |
| 4 | ~~`saba-chan-cli/src/module_registry.rs`의 fs 폴백~~ | ~~CLI가 데몬 응답 실패 시 `module.toml`을 파일시스템에서 직접 파싱~~ → 해결: PR #6 CLI는 모듈 목록을 `GET /api/modules`로만 받는다(`saba-chan-cli/src/resolve.rs:185-193`, `cmd/module.rs:108-109`). 실패하면 오류로 끝나고 로컬 폴백이 없다 | — |
| 5 | ~~raw TOML로 `[errors]` 읽기~~ | ~~봇이 `/api/module/:name`의 raw TOML 덤프를 읽는다~~ → 해결: `[errors]`는 메타데이터 필드이고 `/command`가 `error_message`로 붙인다. `/api/module/:name`은 메타데이터를 돌려준다 (`server.rs:467-487`) | — |
| 6 | ~~`D:src/ipc/handlers/instance.rs:1276-1303`~~ | ~~설정 동기화 제외 목록에 Minecraft 전용 키가 데몬에 하드코딩~~ → 해결(MK6): 데몬 필드만 데몬이 알고, 나머지는 모듈이 `group = "saba-chan"`으로 선언한 설정을 뺀다 | — |
| 7 | ~~`D:src/supervisor/mod.rs`~~ | ~~`server_jar` 키 주입, `jar_name="server.jar"` 기본값~~ → 해결(MK7): 모듈에는 `server_executable`·`working_dir`만 넘기고, 설치 요청·config의 실행 파일 이름도 `server_executable`이다. 함수 이름 `manage_properties`·라우트 `/properties`는 남았지만 게임 설정 파일 일반을 뜻한다 | — |
| 8 | `saba-chan-gui/src/components/ServerSettingsModal.js:354,363` (`module === 'palworld'`), `useServerSettings.js:275` ("Install Minecraft …" 문자열) | GUI가 특정 게임 이름으로 분기 | 인터페이스에 게임 지식 |
| 9 | `M:palworld/lifecycle.py:33-36`, `zomboid`/`palworld` `install_server` | 모듈이 플러그인(`extensions.*`)을 Python import로 직접 사용. 활성 여부도 스스로 `SABA_ENABLED_EXTENSIONS`로 확인. 데몬은 import 가드로 꺼진 익스텐션 import만 막는다 (`D:src/plugin/mod.rs:114-126`) | 플러그인 기능 주입은 데몬이 중개해야 하는데, 모듈과 플러그인이 직접 결합 |
| 10 | `modules/daemon_rcon.py`, palworld, zomboid 콜백 | 모듈이 데몬 HTTP API를 역호출(rcon/rest/stdin). ~~인증 없이 호출하고 포트도 하드코딩~~ → 토큰과 `SABA_IPC_URL`을 쓴다 | "매뉴얼"인 모듈이 능동 클라이언트로 동작. 경계가 모호함 |
| 11 | ~~`installer/gui/src-tauri/src/lib.rs`의 하드코딩 모듈 목록~~ | → 해결: 목록은 릴리즈 manifest에서만 만든다 (`lib.rs:169-188`) | — |

---

### 부록: 핵심 수치
- 모듈 크기(2026-10-22): minecraft lifecycle 2,337줄 / toml 1,303줄, palworld 1,637 / 1,351, zomboid 1,610 / 1,580, example 520 / 408.
- settings 필드 수: minecraft 66, palworld 117, zomboid 117, example 9. commands 수: 17 / 10 / 23 / 3.
- 데몬의 lifecycle 호출 함수: 19종(§2.3). 모듈의 `start`는 지워졌다.
