# Changelog

이 프로젝트의 모든 주목할 만한 변경사항이 이 문서에 기록됩니다.

포맷은 [Keep a Changelog](https://keepachangelog.com/ko/1.1.0/)을 따르며,
버전 관리는 [Semantic Versioning](https://semver.org/lang/ko/)을 따릅니다.

## [Unreleased]

### Fixed

- IPC 토큰이 없거나 비어 있으면 인증을 거부한다. 토큰 조각과 명령·설정 본문을 로그에서 제거했다.
- 인스턴스 ID의 경로 탈출·중복·교체와 재정렬 중복을 차단했다. 익스텐션은 발견한 실제 경로에서 제거한다.
- 잘못된 날짜와 한글 로그 입력의 패닉을 막았다. RCON 연결 검사 후 명령 제한시간을 복원한다.
- 종료가 지연된 클라우드 작업을 취소하고 회수한다. Python bridge는 IPC 인증과 토큰 회전을 지원한다.
- 이전 폴링 응답이 새 연결·콘솔을 덮는 문제를 수정했다. 설정 조회·저장·설치·초기화 완료도 요청한 창에서만 반영한다.
- ZIP·tar 경로와 링크를 추출 전에 검사한다. 업데이트 실패 시 기존 모듈·익스텐션을 복원한다.
- 최초 설치도 공통 SHA256 검증을 사용한다. 제거 예약은 검증한 경로만 사용하며 Windows 환경변수 재확장을 막는다.
- 설치 화면의 구문 오류, 업데이트 이벤트 등록 순서, TUI 오류·패닉 후 터미널 복원을 수정했다.
- 버튼 유형·입력 라벨·키보드 조작을 보완했다. 콘솔 이동과 크기 조절은 키보드도 지원한다.

### Changed

- 압축 해제·디렉터리 교체·최초 설치 흐름과 모달·선택 입력·콘솔 렌더러의 중복을 합쳤다.
- PowerShell·Bash 테스트 진입점은 공통 Python 실행기를 사용한다. Rust 바이너리는 기존 라이브러리 모듈을 재사용한다.
- Electron 44와 Vitest 5를 적용했다. JavaScript 개발 환경은 Node.js 22.12 이상, CI는 Node.js 24를 사용한다.
- CI에 MCP 테스트·빌드, GUI lint·빌드, Python 및 설치·업데이트 스크립트 검사를 포함했다.

### Security

- 연동 릴레이의 nonce 보존 시간을 서명 유효창 전체보다 길게 유지한다. HMAC 검증 실패는 nonce를 소모하지 않는다.
- Electron 인증 헤더는 데몬에만 전달한다. 외부 탐색·리다이렉트·외부 프로토콜과 MCP 요청 경로를 제한한다.
- GUI 빌드는 인라인 스크립트를 차단하고 설치된 확장의 blob 스크립트는 허용한다. 개발 서버의 React 갱신 코드는 유지한다.
- 운영 서명 키에 의존하던 테스트를 가짜 키와 로컬 HTTP 서버로 바꿨다. Python 테스트의 데이터·임시 경로도 격리한다.
- 의존성 수정 후 RustSec 취약점은 16건에서 0건으로 줄었다. GUI·MCP·음악 서버의 전체 npm audit와 릴레이 운영 의존성 audit는 0건이다.
- 릴레이 개발 도구에는 중간 위험 경고 4건이 남았다. `drizzle-kit`의 기존 esbuild 변환 경로이며 운영 의존성에는 포함되지 않는다.
- RustSec 정보성 경고는 유지보수 중단 8건과 비건전성 4건이다. LRU의 해당 취약 함수는 호출하지 않으며 rand 경고의 조건 기능도 비활성이다. Linux 전용 glib 경고는 남아 있다.

### Validation (2026-10-15)

| 대상 | 결과 |
|---|---|
| Rust 전체 워크스페이스 | 896 통과, 4 제외. `--locked` 테스트·전체 대상 Clippy·개발 빌드 통과 |
| GUI | 270 통과. 폐기된 로컬 봇의 빈 검사 2건 제거. lint 오류 0, 빌드 통과 |
| MCP | 17 통과. TypeScript 빌드 통과 |
| 릴레이 서버 | 임시 PostgreSQL에서 435 통과. TypeScript 빌드 통과 |
| 음악 서버 | 348 통과. TypeScript 빌드 통과 |
| 게임 모듈·익스텐션 | 각각 60 통과·1 제외, 67 통과. 공통 실행기로 검사 |
| Python bridge·테스트 실행기 | 8 통과 |
| 설치·업데이트 JavaScript | 문법 검사 통과, 이벤트 등록 회귀 1 통과 |
| Electron 44 실제 렌더러 | 격리된 preload·빈 대시보드·확장 등록·CSS 주입 통과. 인라인 스크립트 차단, blob 실행 허용, 보안 경고·예상 밖 오류 0 |

릴레이 검사용 PostgreSQL은 작업 디렉터리에 격리했고 검사 후 종료했다. 운영 DB는 사용하지 않았다.
제외한 Rust 검사는 모의 서버가 필요한 2건, Palworld의 별도 UE4 INI 경로가 필요한 1건, 예제 doctest 1건이다. 모듈 1건은 설치된 데몬의 공유 모듈 경로가 필요하다.
GUI lint 경고 3건은 단계 전환 애니메이션 의존성과 모션 비활성화 CSS에 남아 있다. 주 번들 크기는 약 678 kB로 Vite 권고를 넘는다.
실제 게임 서버·Discord 연결·Linux 실행·설치·제거·릴리스 패키징·라이브 배포는 검증하지 않았다.
진단 초기에 테스트 격리 부족으로 로컬 IPC 토큰이 실패 로그에 노출됐다. 로그는 마스킹했으며 데몬 재시작을 통한 토큰 재발급을 권장한다.

### Removed
- **로컬 디스코드 봇 폐기**: `discord_bot/` Node.js 봇 프로세스와 관련 설정(`mode: "local"`, 사용자 측 Discord 토큰 입력 등) 모두 제거. 이제 디스코드 봇은 클라우드 릴레이 서버(`server-chan`) 안에서만 운영되며, 사용자는 GUI 페어링만으로 연결합니다.

## [0.1.1] - 2026-04-21

### Added
- 통합 테스트 러너 `scripts/run-test.ps1` (단일 진입점으로 전체 스위트 실행)
- AppData 동기화 검증 스크립트 `scripts/check-appdata-sync.ps1` (`-Fix` / `-Strict` 옵션)
- Updater E2E 자동화 (launch-ticket 일회성 / 만료 / spawner 매트릭스 / 네트워크 복원)
- IPC JSON Schema 계약 테스트 (daemon ↔ GUI ↔ bot 필드 drift 감지)
- Relay 서버 보안·동시성 E2E (Nonce 리플레이 / HMAC 변조 / 페어링 레이스 / SQL 인젝션)
- `CONTRIBUTING.md`, `SECURITY.md`, `CHANGELOG.md`
- Discord 봇 클라우드 모드(릴레이 경유) 지원

### Changed
- Updater `--spawned-by` 화이트리스트 강제 (`gui` / `cli` / `daemon`)
- Launch ticket 발급·소비 함수(`issue_at`/`consume_at`) 테스트용 공개
- 모듈/익스텐션 테스트 커버리지 확대 (palworld / zomboid / docker / music)

### Security
- Updater 체인 강화: 티켓 검증 실패 시 즉시 종료 + 티켓 파일 즉시 삭제
- 릴레이 서버 SQL 인젝션 / HMAC 변조 / 페어링 레이스 전역 검증

## [0.1.0] - 초기 릴리스

### Added
- Core Daemon (Rust/Axum, `127.0.0.1:57474`)
- GUI (Electron 28 + React 18 + Vite)
- CLI (Rust/Ratatui)
- Discord Bot (discord.js 14)
- Updater (GitHub Releases 기반, SHA256 검증)
- Installer (Tauri 기반)
- 내장 모듈: Palworld, Minecraft, Project Zomboid
- 내장 익스텐션: Docker, SteamCMD, Music, UE4 INI
- 10개 언어 번역 (ko, en, ja, de, es, fr, pt-BR, ru, zh-CN, zh-TW)
- 릴레이 서버 (Fastify 5 + PostgreSQL 17, HMAC + 토큰 + 타임스탬프 3단계 인증)

[Unreleased]: https://github.com/WareAoba/saba-chan/compare/v0.1.1...HEAD
[0.1.1]: https://github.com/WareAoba/saba-chan/compare/v0.1.0...v0.1.1
[0.1.0]: https://github.com/WareAoba/saba-chan/releases/tag/v0.1.0
