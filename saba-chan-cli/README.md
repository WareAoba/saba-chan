# saba-chan-cli

사바쨩 데몬을 터미널에서 다루는 명령줄 도구입니다. GUI로 할 수 있는 일은 모두 여기서도 할 수 있고,
모든 동작은 데몬 API를 거칩니다 (CLI가 직접 서버 파일을 만지지 않습니다).

## 빠르게

```sh
saba-chan-cli                      # 터미널에서 인자 없이 → 대화형 셸
saba-chan-cli status               # 데몬·서버·디스코드·업데이트 한눈에
saba-chan-cli server add           # 새 서버 (필요한 것을 물어봐요)
saba-chan-cli start pal            # 이름은 일부만 적어도 돼요
saba-chan-cli console pal          # 콘솔 보면서 입력 (:q 나가기, 서버는 계속 돌아요)
saba-chan-cli run pal kick Steve   # 게임 명령 (모듈이 정의한 명령, 별명도 됨)
saba-chan-cli chat 팰월드 시작      # 디스코드와 같은 채팅 처리
```

데몬이 꺼져 있으면 켤지 물어봅니다. CLI를 닫아도 데몬과 서버는 계속 돕니다.

## 명령

| 갈래 | 하는 일 |
|---|---|
| `status`, `version` | 전체 상태, 설치된 부분의 버전 |
| `server` (`start` `stop` `restart` `console` `run`은 바로 써도 됨) | 목록·정보·추가·삭제·켜기·끄기·콘솔·게임 명령·설정(`settings`, `set`)·게임 설정 파일(`properties`)·RCON·REST·버전·업데이트·설치 진행·종료 기록·점검(`validate`, `diagnose`)·EULA·초기화·순서 |
| `module` | 게임 모듈 목록·정보·명령·설치할 수 있는 버전·받기·지우기·폴더 감지·서버 파일만 받기 |
| `extension` | 익스텐션 목록·켜기·끄기·받기·업데이트·지우기·설정 |
| `bot` | 디스코드 봇 연결: 페어링(코드 표시 → 디스코드에 입력)·연결·끊기·접두사·게임 별명·노드·멤버·설정 |
| `update` | 사바쨩 업데이트 확인·받기·적용·마지막 결과·무결성 확인·설정 |
| `config` | 앱 설정 보기·바꾸기 (`cli.키`는 CLI 전용 칸) |
| `daemon` | 데몬 켜기·끄기·다시 켜기·로그 |
| `shell`, `completions` | 대화형 셸, 셸 자동 완성 스크립트 |

`saba-chan-cli <명령> --help`로 자세한 사용법을 볼 수 있습니다.

게임 명령은 설치된 모듈이 정합니다 (`saba-chan-cli server commands pal`). 인자는 순서대로 주거나
`이름=값`으로 줍니다. 빠진 인자는 물어보고, 되돌릴 수 없는 명령은 확인을 받습니다.
정의되지 않은 줄은 콘솔에 그대로 보냅니다 (모듈이 허용할 때).

## 스크립트에서

- `--json`: stdout에는 JSON 하나만 나갑니다. 안내·진행 표시는 stderr로 갑니다.
- `--yes` (`-y`): 확인 질문에 예. 터미널이 아니면 묻지 않으므로, 확인이 필요한 명령은 `--yes`가 있어야 합니다.
- 종료 코드: `0` 성공 · `1` 실패 · `2` 사용법 오류·여러 개가 맞음 · `3` 데몬에 연결할 수 없음 · `4` 대상 없음 · `130` 취소
- 실패하면 `--json` 출력에 `error`, `error_code`, (있으면) `suggestions`가 들어갑니다.

## 설정과 환경 변수

| 무엇 | 어디서 |
|---|---|
| 데몬 포트 | `--port`, `SABA_IPC_PORT`, 설정 파일의 `ipcPort`, 기본 57474 |
| 언어 | `--lang`, `SABA_LANG`, 설정 파일의 `language`, 시스템 언어, 영어 |
| 데몬 실행 파일 | `SABA_CORE_PATH`, CLI와 같은 폴더 |
| 색 끄기 | `--no-color`, `NO_COLOR` |

문구는 `locales/<언어>/cli.json`에 있습니다. 영어·한국어는 실행 파일에 들어 있고, 없는 문구는 영어로 보입니다.
새 언어는 그 파일 하나만 더하면 됩니다.

## 익스텐션이 CLI에 더하는 것

익스텐션 manifest의 `cli.slots`로 화면에 내용을 끼울 수 있습니다 (데이터만, 코드 실행 없음).

- `InstanceList.badge` `{text, condition?}`: 서버 목록 이름 옆 표시. `condition`은 `instance.ext_data.<키>`.
- `InstanceDetail.status` `[{label, value_from}]`: 서버 정보에 줄 추가.
- `CreateInstance.options` `[{field, type, label, default?}]`: 서버를 추가할 때 묻고 `extension_data`에 저장.

## 빌드

```sh
cargo build -p saba-chan-cli --release
cargo test -p saba-chan-cli
```
