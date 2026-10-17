# Contributing to Saba-chan

Saba-chan 프로젝트에 관심을 가져주셔서 감사합니다. 이 문서는 기여 방법과 규칙을 안내합니다.

## 리포지토리 구조

Saba-chan은 네 개의 저장소로 구성됩니다. 기여 대상에 따라 올바른 저장소에 PR을 제출하세요.

| 저장소 | 역할 |
|--------|------|
| [saba-chan](https://github.com/WareAoba/saba-chan) | Core Daemon · GUI · CLI · Updater · Installer |
| [saba-chan-modules](https://github.com/WareAoba/saba-chan-modules) | 게임별 lifecycle 모듈 (Palworld, Minecraft 등) |
| [saba-chan-extensions](https://github.com/WareAoba/saba-chan-extensions) | 범용 익스텐션 (Docker, SteamCMD 등) |
| [server-chan](https://github.com/WareAoba/server-chan) | 클라우드 릴레이 서버 + 공식 디스코드 봇 + 음악 서버 + 문서 사이트 |

## 개발 규칙

1. **TDD 원칙**: 기능 추가·수정 전 테스트를 먼저 작성하고 실패를 확인한 뒤 구현합니다.
2. **테스트 실행 의무**: PR 전 아래를 모두 실행하여 회귀가 없음을 확인합니다.
   ```powershell
   .\scripts\run-test.ps1
   ```
   `scripts/run-test.sh`도 같은 Python 실행기를 사용합니다. Python 3.11 이상과 Node.js 22.12 이상이 필요합니다.
   Python 경로가 자동으로 선택되지 않으면 `SABA_PYTHON`을 지정합니다.
   개별 스위트는 `rust|gui|mcp|python|modules|extensions|relay|music|appdata-sync`입니다.
   `rust`는 워크스페이스 전체를 한 번 실행합니다. `mcp`는 테스트와 타입 검사를 실행합니다.
   릴레이는 Docker 테스트 DB 또는 임시 PostgreSQL을 가리키는 `TEST_DATABASE_URL`이 필요합니다.
   형제 저장소가 없으면 해당 스위트는 `SKIP`으로 표시됩니다. 실패한 스위트가 있으면 종료 코드는 1입니다.
3. **AppData 반영**: 런타임을 갱신할 때 `scripts/check-appdata-sync.ps1 -Fix`를 사용합니다. 데이터 변경 없는 검사는 `-Suite appdata-sync`로 별도 실행합니다. 기본 테스트는 런타임을 동기화하지 않습니다.
4. **하드코딩 금지**: 경로·포트·IPC 토큰은 상수 모듈 또는 환경변수에서 가져옵니다.
5. **에러 처리**: 에러를 삼키지 않습니다. 데몬이 stdout/stderr로 수집합니다.
6. **함수 단일 책임**: 한 함수는 하나의 역할만 수행합니다.

## 모듈 기여

새 게임 모듈은 `saba-chan-modules` 저장소에 추가합니다. [모듈 개발 가이드](https://github.com/WareAoba/saba-chan-modules/blob/main/docs/module-development-guide.md)를 참조하세요.

필수 파일:
- `module.toml` — 메타데이터 · 설정 스키마
- `lifecycle.py` — 서버 수명주기 훅
- `test_lifecycle.py` — 계약 테스트 (`_KEY_MAP` ⊆ `DEFAULT_*.keys()` 등)
- `locales/{ko,en,ja,...}.json` — 번역

## 익스텐션 기여

새 익스텐션은 `saba-chan-extensions` 저장소에 추가합니다.

필수 파일:
- `manifest.json`
- Python 모듈 + `FUNCTIONS` / `_FUNCTIONS` 디스패치
- `test_*.py` — 계약 테스트

## PR 가이드

1. `main` 브랜치에서 feature 브랜치를 만듭니다.
2. 커밋 메시지는 명령형 영문 또는 간결한 한국어로 작성합니다.
3. PR 설명에 다음을 포함:
   - 변경 요약
   - 관련 이슈 번호
   - 실행한 테스트 스위트
   - 스크린샷 (GUI 변경 시)
4. CI(`.github/workflows/ci.yml`)가 통과해야 머지됩니다.

## 버그 리포트

GitHub Issues에 다음 정보를 포함하여 제출하세요.

- OS 및 Saba-chan 버전 (`설정 탭 → 정보` 또는 `saba-core.exe --version`)
- 재현 절차
- 기대 동작 vs 실제 동작
- 로그 (`%APPDATA%\saba-chan\logs\`)

## 보안 취약점

공개 이슈가 아닌 [SECURITY.md](SECURITY.md)에 따른 비공개 절차로 신고해주세요.

## 라이선스

기여하신 코드는 [MIT License](LICENSE) 조건으로 배포됩니다.
