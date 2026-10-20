# 0.1.9 합의 계약 (구현 중)

> 이 문서는 0.1.9에 넣기로 한 계약을 구현보다 먼저 적는다. 모듈·릴레이·GUI·CLI는 이 문서를 보고 시작한다.
> 데몬 구현이 머지되면 해당 절을 01·02·05 문서의 제자리로 옮기고 여기서는 지운다. 이름이나 모양을 바꿔야 하면 이 문서를 먼저 고친다.
> 근거: 사용자 디스코드 검수(2026-10-20, 9·13·14번), 릴레이 설계(`relay/discord-9-13-14-design.md`), R39 제안, 모듈 감사 MA3.

## 1. 디스코드에서 백업 복원 (검수 14번)

- 액션 `backup.restore`의 표면에 Discord를 더한다(GUI·CLI·Discord·챗봇). Kind admin, risk danger 그대로라 늘 확인을 거친다. 요구 레벨은 지금 규칙(`levels.admin`, 프리셋 strict면 3)이다.
- 입력 칸이 다른 액션의 결과에서 고르는 값이면 칸에 범용 표지 `optionsFrom`을 싣는다. heartbeat `actions[].input.properties.<칸>.optionsFrom`과 `GET /api/actions`의 `input`에 같이 싣는다.
  - 모양: `{action, list, value, label?}`. `action`이 부를 조회 액션(같은 인스턴스), `list`가 결과에서 항목 배열의 키, `value`가 칸에 넣을 항목 키, `label`이 사람에게 보일 항목 키다.
  - `backup.restore`의 `name`: `{action:"backup.list", list:"backups", value:"name", label:"label"}`.
  - 릴레이는 이 표지가 있는 칸을 자동완성으로 만든다(최근 것부터 25개). 다른 액션도 같은 표지를 쓸 수 있다.
- `backup.list` 액션 결과의 `backups[]` 항목마다 `label`을 더한다. 요청 언어와 PC 시간대로 `2026-10-20 21:03 · 수동 · 120MB` 같은 한 줄이다(종류 이름은 봇 locales). 라우트 `GET /api/instance/:id/backups`는 그대로다.
- 서버가 켜져 있으면 `plan(backup.restore)`가 계획을 만들지 않고 409 `server_running`으로 답한다. 여러 단계 계획에서 같은 인스턴스의 `server.stop` 단계가 앞에 있으면 계획 때는 이 검사를 하지 않고, 그 정지 단계가 끝난 뒤 복원 단계를 실행할 때 한다. 앞에 정지 단계가 없으면 계획 때 409다. 실행 때 검사(지금 라우트의 409)는 늘 남는다.
- 릴레이가 "먼저 끄고 복원"을 고르면 여러 단계 계획 `POST /api/actions/plan {steps:[{id:"server.stop", instance}, {id:"backup.restore", instance, input:{name}}]}`을 보낸다. 데몬은 다음을 보장한다.
  - 정지 단계는 서버 프로세스가 끝난 것을 확인한 뒤에 성공한다. 그 뒤에 복원이 시작된다.
  - 정지가 실패하거나 `graceful_stop_unavailable`(MK10)이면 정지 단계가 실패로 끝나고 복원은 `skipped`다.
  - 원격 계획의 `server.stop`은 `force`를 받지 않는다(강제 종료는 GUI·CLI만).
  - 정지 단계는 non-managed면 정지 호출 한도(`stop_timeout`+30초, 최대 3630초), managed면 모듈 호출(최대 120초) 뒤 `stop_timeout`만큼 종료를 기다리고 강제 종료(5초)까지라 최대 약 3725초가 걸릴 수 있다. 릴레이는 계획 실행 결과를 그만큼 기다리거나, 진행 중임을 먼저 알리고 결과를 뒤에 보낸다(05 문서의 액션 실행 절에 맞춘다).

## 2. 플레이어 목록 `server.players` (검수 13번)

- 새 함수 이름을 만들지 않고 이미 있는 `[status] players`(02 §`[status]`, 접속자 수 함수)를 넓힌다. 그 함수의 반환에 선택 필드 `players:[{name}]`를 더한다: `{success:true, online:int, max?:int, players?:[{name}]}`. 이름 말고 IP·고유 ID는 싣지 않는다(실어도 데몬이 뺀다). 서버가 꺼져 있으면 지금처럼 실패를 돌려준다.
  - 지원 판정: 모듈이 `[status] players`를 선언하면 이 액션을 지원한다. 이름을 주지 않는 모듈은 수만 보인다. 따로 선언하는 키는 없다.
  - 데몬은 이름이 필요할 때(아래 라우트·액션)만 config에 `players_detail: true`를 싣는다. `players_detail`·`stop_call_limit`은 데몬이 config에 싣는 예약 이름이라 모듈 설정 이름으로 쓸 수 없다(0.1.9 로더는 그 이름의 설정을 경고하고 데몬 값으로 덮는다). 1분마다 도는 갱신에는 싣지 않는다. 이름을 콘솔 명령으로만 얻을 수 있는 모듈은 이 키가 있을 때만 그 명령을 보내고, 없으면 콘솔에 아무것도 찍지 않는 방법으로 수만 읽는다.
  - 통신은 모듈 규칙대로 STDIN → REST → RCON. 지원하는 게임과 출력 형식은 모듈 담당이 공식 문서로 확인한다.
- 데몬 라우트 `GET /api/instance/:id/players` → 그 함수를 `players_detail: true`로 바로 불러(10초 제한) `{success, online, max?, players:[{name}]}`(`players`는 `{name}`만, 없으면 빈 배열). `[status] players`가 없으면 200 `{success:false, error_code:"not_supported"}`, 서버가 꺼져 있으면 409 `server_not_running`. 1분마다 도는 접속자 수 갱신(`supervisor/players.rs`)은 이름을 기억하지 않는다.
- 액션 `server.players`: Query, safe, 모든 표면, 인스턴스 액션. 요약 `text`는 `접속 3/20명` 다음 줄마다 이름(10명까지, 남은 수는 `외 N명`). 봇 locales `actions.server.players.title`(ko 플레이어, en Players, ja プレイヤー …)과 `actions.summary.players*`.
- heartbeat `actions[]`의 `server.players` 항목에는 `modules:[이름]`(`[status] players`를 선언한 모듈)을 싣는다. 릴레이는 모듈이 목록에 있는 서버에만 하위 명령 `플레이어`를 만든다. `GET /api/actions?instance=`는 그 인스턴스의 모듈이 지원할 때만 싣는다.
- 챗봇에 가는 결과는 콘솔과 같이 IP·비밀값을 가린다(`mask_console`).

## 3. 로그 전체 읽기·내보내기 (검수 9번)

- 로그 원천은 인스턴스의 콘솔 기록이다: saba-runner가 남기는 세션 파일(`log_sessions_keep`개)과, 러너 없이 직접 실행했을 때의 `logs/console.log`·회전 파일. 모듈 `[launch] log_files`도 콘솔에 합쳐져 있다. 원천 id는 세션 파일 이름(확장자 뺀 것) 또는 `console`·`console.1`·`console.2`, `label`은 그 기록의 시작 시각이다. 목록은 최근에 쓴 것부터이고 맨 앞이 `latest`다.
- 라우트 `GET /api/instance/:id/logs?source=&lines=&before=`
  - 응답 `{sources:[{id, label, startedAt}], source, lines:[문자열], before?}`. `startedAt`은 세션 시작 unix 초라 GUI·CLI가 자기 시간대·언어로 다시 그릴 수 있다. `lines`는 기본 200, 최대 500. 끝에서부터 읽고 `before`(이전 응답이 준 커서)로 더 앞을 읽는다. 원천이 없으면 404 `not_found`.
- 라우트 `GET /api/instance/:id/logs/export?source=&since=&until=`
  - 응답 `{fileName, bytes, truncated, content}`. `fileName`은 `<서버 이름>-<source>-<YYYYMMDD-HHMM>.log`(파일 이름에 못 쓰는 글자는 `_`). `since`·`until`은 unix 초로 줄의 시각을 거른다(시각이 없는 줄은 앞 줄을 따른다).
  - `content`는 JSON 문자열로 쓴 크기(이스케이프 포함)로 최대 4 MiB다. 넘으면 최근 쪽 온전한 줄을 남기고 `truncated:true`. 릴레이 `/result`는 결과 봉투와 가린 뒤 늘어난 만큼의 여유를 두고 6 MiB 이상을 받는다.
- 액션: `server.logs.read`(Query, safe), `server.logs.export`(Query, safe: `server.logs.read`와 같은 가린 읽기 전용이고, 말하는 사바쨩 상한 기본값 0에서도 "로그 파일 줘"가 되게). 둘 다 GUI·CLI·챗봇 표면이고 디스코드 메뉴에는 없다.
- 가리기: 원격(챗봇·디스코드)으로 나가는 모든 줄은 콘솔 꼬리와 같은 가리기(IP·비밀값, 절대 경로 → `<path>`)를 거친다. 내보내기도 같다. GUI·CLI(로컬)는 원문이다.
- 릴레이 규칙(05 문서에 같은 줄을 둔다): `/result` 본문은 `{success, data:{success, actionId, status, result}}`이고, 그 `data.result`가 `{fileName, bytes, truncated, content}` 모양이면 릴레이는 `content`를 파일로 첨부하고 모델에는 `content`를 주지 않는다. 모델에는 `{ok, fileName, bytes, truncated}`만 간다.

## 4. 서버 준비 알림 `instance.ready` (R39)

- 모듈은 서버가 접속을 받기 시작할 때 콘솔에 찍히는 문구를 `module.toml`의 `[extensions.saba-core] ready_patterns`(정규식 목록)로 선언할 수 있다. 같은 표의 `ready_timeout`(초, 기본 600)은 그 문구를 기다리는 시간이다.
  - `[extensions.saba-core]`는 데몬이 읽는 예약 표다. 0.1.8 로더는 `[extensions.*]`의 내용을 보지 않으므로 이 키가 있는 모듈도 0.1.8에서 그대로 로드된다(`[detection]`에 두면 스키마 v1 0.1.8 로더가 모르는 키로 거부한다).
  - 이 표는 버전 사이를 잇는 자리다. 0.1.9부터 데몬은 이 표의 모르는 키(더 새 데몬의 키)를 거부하지 않고 경고만 남긴다. 그래서 다음 버전이 키를 더해도 0.1.9가 그 모듈을 읽는다.
  - `saba-core`라는 id의 익스텐션은 만들 수 없다. 데몬은 그 id의 익스텐션을 설치·인식하지 않는다.
  - 값 검사(아는 키의 값이 틀리면 그 항목만 버리고 경고한다): `ready_patterns`는 문자열 배열이고, 정규식이 아닌 항목은 버리고 그 모듈에 경고만 남긴다(로드는 실패하지 않는다). `ready_timeout`이 0이거나 숫자가 아니면 경고를 남기고 기본값을 쓴다.
- 데몬 이벤트 `instance.ready {at, instanceId, instance, ok, code?}`를 시작 한 번(자동 재시작 포함)마다 한 번 낸다.
  - `ok:true`: `ready_patterns` 중 하나가 콘솔 줄과 맞았을 때. 선언이 없으면 시작이 성공한 때(managed는 프로세스가 뜨고 시작 가드가 풀린 때, non-managed는 모듈 `start` 성공).
  - `ok:false`: 준비되기 전에 끝났을 때. `code`는 `start_failed`(시작 실패나 준비 전에 프로세스가 끝남), `stopped_while_starting`(준비 전에 사용자가 정지), `ready_timeout`(`ready_timeout`이 지남). 원문·경로·콘솔 줄은 싣지 않는다.
  - 데몬이 띄운 프로세스가 알림 전에 끝나 이미 치워졌으면 `start_failed`다. `ready_timeout`이 시계로 나타낼 수 없을 만큼 크면 시간 초과 없이 기다린다.
  - 감독자가 시작을 시도하지 않은 요청(사전 점검에 막힘, 다른 작업 중, 이미 켜짐, 시작 중)에는 알림이 없다. 자동 재시작이 사전 점검에 막히면 `instance.restart_failed`·`instance.restart_cancelled`가 끝을 알린다.
- relay_event 허용 목록에 넣는다(`instance.ready`는 위 필드만). 지금 `instance.exit`(start_failed)는 그대로 나간다.

## 5. 복원 허용 범위 `restore_roots` (모듈 감사 MA3)

- 모듈 `backup_paths` 훅의 반환에 선택 필드 `restore_roots:[경로]`를 더한다. 경로 해석(작업 폴더 기준 상대, `..`·절대 경로 규칙)은 `paths`와 같다.
- 복원은 백업에 든 경로가 서버 폴더·인스턴스 폴더·지금 `paths`의 대상·`restore_roots` 중 하나의 안에 있으면 허용한다. 그 밖이면 지금처럼 400 `restore_outside`다.
- `restore_roots`는 백업 대상에 넣지 않는다(무엇을 담을지는 `paths`가 정한다). 데이터 폴더 단위로 옛 백업이 담겨 있는 모듈(좀보이드 `Server/`)이 쓴다.

## 6. non-managed 정지 대기 시간 (모듈 #37과 합의)

- 데몬은 모듈 `stop()`의 config에 이번 정지에 쓸 대기 시간 `stop_timeout`(초)과 그 호출의 시간 제한 `stop_call_limit`(초, non-managed면 `stop_timeout + 30`, managed면 120)을 늘 싣는다. 모듈은 `stop_call_limit`의 30초 앞까지만 기다린다. 키가 없으면(0.1.8 데몬) 지금처럼 90초 상한을 지킨다(modules #40). 값은 인스턴스 설정 `stop_timeout`, 모듈 `[config] stop_timeout`, 기본 30 순이다(`supervisor::stop_timeout_of`, 1~3600).
- non-managed 정지(`managed:false`)의 모듈 `stop()` 호출 시간 제한은 `stop_timeout + 30`초다(지금의 고정 120초 대신). 모듈은 자기 상한(90초 등)을 두지 않고 `stop_timeout`만큼 기다린 뒤 결과를 돌려준다. 30초는 저장 명령·프로세스 정리에 쓰는 여유다. 0.1.8 데몬(고정 120초)과 섞이지 않게 모듈의 상한 제거는 이 데몬 변경이 든 릴리스(0.1.9)에 같이 싣는다.
- managed 정지(`managed:true`)에서 모듈 `stop()`은 저장·종료 요청만 보내고 기다리지 않고 바로 `success`를 돌려준다. 끝나기를 기다리는 일과 시간이 지난 뒤 강제 종료는 데몬이 `stop_timeout`으로 한다. 이 호출의 시간 제한은 지금처럼 기본값(120초)이다.

## 7. 설정 타입 목록과 스키마 2 (MK2)

- 0.1.8까지의 규칙(#374)은 그대로다. 스키마 1 모듈의 설정 `type`은 `text`·`password`·`number`·`file`·`select`·`boolean`(`SETTING_TYPES`), 명령 입력 `type`은 `text`·`password`·`number`·`select`·`boolean`(`COMMAND_INPUT_TYPES`) 중 하나이고, 밖이면 로드에 실패한다(스키마 이전 모듈은 경고).
- 0.1.9는 `schema_version = 2`를 받는다. 스키마 2는 스키마 1에 8절의 키·타입을 더한 것이다. 8절의 키·타입은 스키마 2 모듈에서만 받고, 스키마 1 모듈에 있으면 모르는 키·타입으로 로드에 실패한다(쓰는 쪽이 버전 표시를 잊지 않게).
  - 0.1.8 데몬은 스키마 2 모듈을 지금 규칙대로 `module.toml schema_version 2 is not supported by this saba-chan (supports 1); update saba-chan`으로 로드하지 않는다. 값 모양이 바뀌는(목록) 설정을 0.1.8이 잘못 다루는 것보다 낫다.
  - 0.1.9로 내는 모듈은 모두 스키마 1로 둔다(게이트 7: 0.1.9 모듈이 0.1.8 데몬에서 로드돼야 하고, 0.1.8 업데이터는 `requires`를 보지 않는다). 스키마 2 모듈은 0.1.10부터 낸다. 0.1.9의 스키마 2 지원은 그때를 위해 미리 넣는 것이다. 8.3의 키는 예약 표에 있어 스키마 1인 0.1.9 모듈도 쓴다.
  - 스키마 2 모듈은 `[module.dependencies] saba-core = ">=0.1.9"`도 적고, 릴리스 매니페스트의 모듈 항목 `requires`에 같은 값을 싣는다. 0.1.9 업데이터는 `requires`를 못 맞춘 모듈 업데이트를 코어보다 먼저 적용하지 않는다(같이 받으면 코어 먼저).
- 로드 실패를 화면에 보인다: `GET /api/modules`의 `failed[]` 항목에 `error_code:"module_schema_invalid"`와 `problems:[문자열]`(필드 이름과 무엇이 틀렸는지, 경로 없음)을 싣는다. 스키마 버전이 높아 못 읽으면 `error_code:"module_schema_unsupported"`, `required:<버전>`. GUI·CLI는 모듈 목록에 "이 모듈을 읽지 못했습니다"와 `problems`를 보인다(locales `modules.load_failed.*`).

## 8. 설정 표현력과 플레이어 접속·퇴장 이벤트 (MK8)

### 8.1 새 설정 타입 (스키마 2)

- `list`: 문자열 목록. API·저장·모듈 config 모두 JSON 문자열 배열이다. `default`는 TOML 문자열 배열.
  - `options`가 있으면 각 항목이 그중 하나여야 한다(여러 개 고르기). 없으면 자유 입력 항목이다.
  - `format`·`pattern`은 항목마다 검사한다. 빈 문자열 항목과 같은 값 두 번은 400 `invalid_format`.
  - `min_items`·`max_items`(정수, 선택): 항목 수 범위.
  - 옮기기: 저장된 값이 문자열인 설정이 모듈 업데이트로 `list`가 되면, 데몬이 인스턴스를 읽을 때 쉼표로 나누고 앞뒤 공백·빈 항목을 버려 배열로 바꾼다. 바꾼 값은 다음 저장 때 파일에 쓴다(읽기만으로 파일을 고치지 않는다). 항목 안의 쉼표는 구분할 수 없어 나뉜다. 그런 값을 쓰던 모듈은 `list`로 바꾸지 말고 새 설정 이름을 쓴다. 게임 파일에 쓰는 모양(팰월드 `("A","B")` 등)은 모듈 `configure`가 정한다.
- `multiline`: 줄바꿈이 있는 문자열. `format`·`pattern`은 값 전체에 쓴다(`pattern`은 여러 줄 모드 없이 그대로). `max_length`(정수, 선택)로 글자 수 상한.
- 명령 입력 `type`에는 0.1.9에서 새 타입을 더하지 않는다.

### 8.2 새 설정 키 (스키마 2)

- `restart_required = true`(bool, 기본 false): 서버가 켜져 있을 때 바꾸면 다시 켜야 적용되는 설정이다.
  - `PATCH /api/instance/:id`(설정 저장) 응답에 `restart_required:[설정 이름]`을 싣는다. 이번 저장에서 값이 바뀐 이 키의 모듈 설정(모듈이 선언한 설정만, `memory_limit_mb` 같은 인스턴스 칸은 아니다)이고, 서버가 켜져 있을 때만 채운다(꺼져 있으면 빈 배열). GUI·CLI는 "다시 시작해야 적용됩니다"를 보이고 재시작 버튼을 함께 낸다. 데몬이 스스로 재시작하지 않는다.
  - `GET /api/modules`의 필드에 그대로 실려 GUI가 설정 옆에 표시를 단다.
- `visible_when`(표, 선택): 다른 설정 값에 따라 이 설정을 보일지. `{ field = "<설정 이름>", equals = <값> }` 또는 `{ field = "<설정 이름>", in = [<값>…] }`.
  - 로드 검사: `field`가 같은 모듈의 설정이어야 하고 자기 자신이나 순환이면 안 된다. `field`가 `list`·`multiline`·`password`·`file` 설정이면 안 된다. 값은 그 설정의 타입에 맞아야 한다. 어기면 로드 실패(7절 `problems`).
  - 비교는 설정 타입으로 바꾼 값끼리 한다: `boolean`은 true/false, `number`는 수 값("25565"와 25565는 같다), `select`·`text`는 문자열 그대로. 값이 없으면 그 설정의 `default`로 비교한다.
  - 숨긴 설정은 값을 지우지 않는다. 숨겨진 동안 `required` 검사를 하지 않는다. 숨겨져 있어도 값이 오면 지금처럼 검사해 저장한다.
  - `GET /api/instance/:id`의 설정 응답에 `hidden:[설정 이름]`(저장된 값 기준)을 싣는다. CLI는 이 값을 쓴다. GUI는 `GET /api/modules`의 `visible_when`으로 입력 중 값에 같은 규칙을 계산한다. 저장 검사(`required`를 건너뛸지)는 데몬이 보낸 값 기준으로 한다.

### 8.3 플레이어 접속·퇴장 이벤트

- 모듈은 `[extensions.saba-core]`(4절과 같은 예약 표)에 `player_join_patterns`·`player_leave_patterns`(정규식 목록)를 선언할 수 있다. 각 정규식은 이름 그룹 `(?P<name>…)`이 있어야 하고, 없거나 정규식이 아니면 그 항목만 버리고 경고를 남긴다. 표에 두는 까닭은 4절과 같다(0.1.8도 로드).
- 데몬이 콘솔을 읽는 시작(managed)에서 콘솔 줄이 맞으면 이벤트 `instance.player {at, instanceId, instance, player, change:"join"|"leave"}`를 낸다. 시스템 줄(`LogSource::System`)은 보지 않는다. non-managed 시작에서는 0.1.9에 이 이벤트가 없다.
- 이름 말고 IP·고유 ID는 싣지 않는다. 모듈 패턴이 다른 그룹을 잡아도 `name`만 쓴다. 이름은 64자에서 자르고 제어 문자를 뺀다.
- relay로 올리지 않는다(0.1.9). GUI·CLI 이벤트 스트림(`events --kind instance`)과 TUI 접속자 표시가 쓴다. 1분 접속자 수 갱신(`supervisor/players.rs`)은 그대로다.

## 9. 무인 설치·업데이트의 데몬 몫 (사용자 결정 2026-10-20, 설계 문서 4단계)

- 설치 구성 `profile`(`"desktop"`|`"headless"`)은 installed-manifest 옆의 `install-profile.json`(`{"profile":"headless"}`)에 기록된다(설계 2단계, 인스톨러·`saba-chan-cli install`이 쓴다). installed-manifest는 구성요소 → 버전 맵이라 섞지 않는다(0.1.8이 그 맵을 구성요소 목록으로 그대로 보인다). 파일이 없거나 읽지 못하면 `desktop`이다(0.1.8까지의 설치).
- `GET /api/updates/status` 응답에 `profile`을 싣는다. GUI·CLI는 이 값으로 GUI 업데이트 안내를 보일지 정한다(헤드리스면 GUI 항목이 애초에 없다).
- `POST /api/updates/launch-updater`는 `profile`이 `headless`이면 다음처럼 바뀐다.
  - 업데이터 인자에 `--headless`를 더한다(`--apply --wait-pid … --launch-token … --headless`). 업데이터는 Tauri 없이 적용하고 지금과 같은 완료 표시를 남긴다.
  - 다시 띄울 대상은 요청의 `relaunch`가 있으면 그것, 없으면 데몬 자신(지금 실행 파일과 실행 인자)이다. 떠 있는 GUI를 찾아 바꿔 넣는 동작(`relaunch_target`)과 업데이터의 GUI 찾기 예비 동작은 쓰지 않는다.
  - 응답은 지금과 같다(`{ok, updater_launched, daemon_stopping}`).
- `desktop`이면 지금 동작 그대로다.
- CLI는 `update apply` 뒤 데몬이 다시 응답할 때까지(`GET /api/updates/last-result`가 이번 적용 결과를 줄 때까지) 기다렸다가 결과와 패치노트를 보인다. 데몬 API는 새로 늘지 않는다.
- 데몬이 서비스(systemd·Windows 서비스)로 돌 때 다시 띄우기는 리눅스 배포 단계(이 설계 다음)에서 정한다.
