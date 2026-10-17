# Saba-chan 테스트 범위

테스트 성공은 아래 환경에서 확인한 계약을 뜻한다. 모의 API 테스트를 실제 게임 서버나 Discord 검증으로 해석하지 않는다.
실행 결과와 보안 점검 결과는 [변경 기록](../CHANGELOG.md)에서 관리한다.

## 실행

Windows에서는 `scripts/run-test.ps1`, Unix에서는 `scripts/run-test.sh`를 사용한다.
두 진입점은 `scripts/run-tests.py`를 실행한다. Python 3.11 이상과 Node.js 22.12 이상이 필요하다.
Python 경로를 지정하려면 `SABA_PYTHON`을 사용한다.

| 스위트 | 검사 | 필요한 환경 |
|---|---|---|
| `rust` | `cargo test --workspace --no-fail-fast` | Rust, 플랫폼 빌드 도구, Python |
| `gui` | 전체 Vitest | GUI npm 의존성, jsdom |
| `mcp` | 전체 Vitest, TypeScript 타입 검사 | MCP npm 의존성 |
| `python` | bridge와 실행기 unittest | Python 표준 라이브러리, 사용 가능한 셸 |
| `modules` | 게임 모듈 pytest | 형제 모듈 저장소, pytest |
| `extensions` | 익스텐션 pytest | 형제 익스텐션 저장소, pytest |
| `relay` | 릴레이 전체 Vitest | 형제 서버 저장소, 테스트 전용 PostgreSQL |
| `music` | 음악 서버 전체 Vitest | 형제 서버 저장소 |
| `appdata-sync` | 런타임 파일과 소스의 읽기 전용 비교 | 설치된 런타임, PowerShell |

기본 `all`은 `appdata-sync`를 제외한 스위트를 실행한다. 없는 형제 저장소는 `SKIP`으로 표시한다.
명령 실패는 종료 코드 1로 전달한다. 스킵한 검사와 통과한 검사는 구분한다.
Python 검사는 소스의 공유 bridge를 사용하고 데이터 경로와 임시 디렉터리를 격리한다.
서명 검증 테스트는 가짜 키와 로컬 HTTP 서버를 사용한다. 운영 비밀키는 읽지 않는다.

릴레이 테스트는 `TEST_DATABASE_URL`만 사용한다. 지정하지 않으면 `npm test`가 테스트용 Docker PostgreSQL을 준비한다.
운영 DB 주소를 테스트에 사용하지 않는다. 자세한 조건은 [기여 지침](../CONTRIBUTING.md)을 따른다.

## 계약과 실제 검증 경계

| 영역 | 자동 검사 근거 | 자동 검사에 포함되지 않는 범위 |
|---|---|---|
| 데몬 API·인스턴스 | `tests/daemon_integration.rs`, `tests/ipc_schema_contract.rs`, `src/instance/mod.rs` | 설치된 게임 서버의 전체 수명주기 |
| 인증·클라우드 작업 종료 | `src/ipc/auth.rs`, `src/cloud_agent/mod.rs`, `tests/cloud_agent_e2e.rs` | 운영 릴레이와 장시간 연결 |
| 프로세스·로그 | supervisor 단위 검사, `tests/e2e_cross_component.rs` | 실제 게임 프로세스의 모든 종료 조건 |
| RCON·REST | `tests/protocol_round_trip.rs`, `src/protocol/rcon.rs` | 실제 게임별 프로토콜 차이 |
| CLI | `saba-chan-cli/tests/`, TUI 단위 검사 | 실제 터미널에서의 모든 입력·화면 조합 |
| Electron GUI | `saba-chan-gui/src/test/`, Vite 빌드 | 실제 사용자 데이터와 운영 데몬의 전체 조작 |
| 콘솔·설정 경합 | `console-window.test.js`, `recovery-async.test.js`, `server-settings-live-guard.test.js` | 실제 포인터·화면 배율 조합 |
| Electron 경계·접근성 | `electron-main.test.js`, `ui-accessibility.test.js`, Biome | 실제 보조공학과 OS 권한 대화상자 |
| MCP·Python bridge | `saba-chan-mcp/test/`, `tests/test_daemon_rcon.py` | 외부 MCP 클라이언트의 설치·인증 |
| 게임 모듈 | 형제 저장소의 Minecraft·Palworld·Zomboid 계약 검사 | 실제 게임 자산 다운로드·서버 실행 |
| 익스텐션 | 형제 저장소의 Docker·SteamCMD·UE4 INI 검사 | 실제 컨테이너·Steam 인증·음성 재생 |
| 릴레이 | `server-chan/relay-server/test/`의 PostgreSQL·서명·nonce 검사 | 실제 Discord 길드·음성 연결 |
| 음악 서버 | `server-chan/music-server/test/` | 실제 YouTube·프록시·Discord 전송 |
| 업데이터 | `updater/src/tests.rs`, `tests/updater_*.rs` | 설치된 앱의 운영 업데이트·복구 |
| 인스톨러 | 경로 가드 단위 검사, `installer/gui/src-tauri/tests/`, JS 문법 검사 | 실제 권한 상승·설치·제거 |
| 업데이트 GUI | `updater/gui/tests/apply-listener.test.cjs` | 실제 Tauri 창의 전체 흐름 |
| 다국어 | `tests/i18n_locale_consistency.rs`, GUI 언어 검사 | 모든 번역의 의미·모든 화면 배치 |
| 자동 시작·서비스·사용자 백업 | 관련 코드 검토 | OS 등록·운영 백업 복원 실물 검증 |

## 변경 시 검사

동작을 수정하면 해당 결함을 재현하는 검사를 먼저 추가한다. 구조만 바꾸면 기존 동작 검사를 유지한다.
수정 영역의 검사를 통과한 뒤 영향 범위의 전체 검사를 실행한다.
Rust는 `cargo clippy --workspace --all-targets -- -D warnings`도 통과해야 한다.
GUI는 `npm run lint`와 `npm run build`, MCP와 서버는 `npm run build`를 실행한다.
의존성을 바꾸면 lockfile과 보안 검사 결과를 함께 확인한다.
검사를 스킵하면 이유와 실제 확인한 범위를 변경 기록에 남긴다.
