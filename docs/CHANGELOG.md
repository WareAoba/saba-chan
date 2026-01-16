# Changelog

## [Unreleased] - 2026-01-17

### 🎯 주요 기능 추가

#### Discord Bot 별칭 시스템
- **모듈별 별칭 관리**: 각 게임 모듈(Palworld, Minecraft)에 대한 사용자 정의 별칭 설정
- **명령어 별칭**: start, stop, status 등 명령어에 대한 한글/커스텀 별칭 지원
- **GUI 통합**: Settings 모달에 "Discord 별칭" 탭 추가하여 직관적인 설정 UI 제공
- **영구 저장**: `discord_bot/bot-config.json`에 별칭 저장 및 로드
- **대소문자 무시**: Discord 명령어 대소문자 구분 없이 처리
- **도움말 개선**: 사용자가 설정한 별칭만 표시하여 간결성 향상

#### 시스템 트레이 통합
- **백그라운드 실행**: GUI 창을 닫아도 데몬 계속 실행
- **트레이 메뉴**: 데몬 시작/정지, 창 열기, 완전 종료 등
- **종료 확인 다이얼로그**: 창 닫기 시 QuestionModal로 선택지 제공
  - GUI만 닫기 (백그라운드 실행)
  - 완전히 종료 (데몬 포함)
  - 취소

#### GUI 구조 개선
- **Settings 모달 탭 구조**: 일반 설정 / Discord 별칭 탭 분리
- **모듈별 별칭 관리 섹션 제거**: Settings 모달 내부로 통합하여 UX 개선
- **별칭 저장/초기화**: 모듈별 독립적인 저장 및 기본값 복원

### 🐛 버그 수정

#### 서버 정지 기능
- **문제**: PID 기반 종료가 불안정 (stale PID, 프로세스 추적 실패)
- **해결**: 실행 파일명 기반 종료로 변경 (`taskkill /F /T /IM <process_name>`)
- **영향**: Palworld, Minecraft 모듈의 `lifecycle.py` 수정

#### Discord Bot 크래시
- **문제**: `bot-config.json`의 중첩 객체 구조에서 `.toLowerCase()` 호출 시 TypeError
- **해결**: `getCommandAliases()`에서 중첩 구조를 평탄화하여 `{alias: command}` 형태로 변환
- **추가**: `resolveAlias()` 함수에 타입 체크 추가

#### 설정 저장 문제
- **문제**: 프로그램 재시작 시 Discord 별칭 설정이 사라짐
- **해결**: `settings.json`과 `bot-config.json`의 역할 분리
  - `settings.json`: GUI 전역 설정 (autoRefresh, discordToken 등)
  - `bot-config.json`: Discord 별칭만 저장
- **수정**: `loadSettings()`에서 별칭 로드 제거, `loadBotConfig()` 별도 호출

#### 빈 문자열 처리
- **문제**: 사용자가 별칭을 비워도 저장되지 않음 (falsy check)
- **해결**: `module.name in discordModuleAliases` 키 존재 여부로 판단하도록 수정

### 🔧 코드 정리 및 리팩토링

#### 모달 컴포넌트 통합
- **변경 전**: `SuccessModal.js`, `FailureModal.js`, `NotificationModal.js`, `QuestionModal.js` 4개 파일
- **변경 후**: `Modals.js` 하나로 통합 (named exports 사용)
- **영향**: `App.js` import 문 간소화

#### 디렉토리 구조 개선
```
생성:
- scripts/                  # 실행 스크립트 통합
- docs/archive/             # 레거시 문서 보관

이동:
- make-executable.sh        → scripts/
- test_rcon.py              → docs/archive/
- RCON_TEST.md              → docs/archive/
- RCON_COMPLETION.md        → docs/archive/
```

#### .gitignore 업데이트
```gitignore
# Python 캐시 추가
__pycache__/
*.py[cod]
*$py.class
*.so
.Python
*.pyc
```

#### 문서 업데이트
- `README.md`: 프로젝트 구조 섹션 업데이트 (실제 디렉토리 구조 반영)

### 🏗️ 아키텍처 변경

#### Discord 별칭 데이터 흐름
```
module.toml (기본값)
    ↓
GUI Settings Tab (사용자 입력)
    ↓
bot-config.json (영구 저장)
    ↓
Discord Bot (런타임 병합)
```

#### IPC 통신 확장
- **추가 API**:
  - `GET /api/config/bot`: Discord bot 설정 로드
  - `PUT /api/config/bot`: Discord bot 설정 저장
  - `GET /api/module/:name`: 모듈 메타데이터 (별칭 포함) 조회
  
- **Preload Bridge**:
  - `botConfigLoad()`, `botConfigSave()`
  - `onCloseRequest()`, `closeResponse()` (앱 종료 처리)

### 📊 파일 변경 통계

#### 신규 생성
- `electron_gui/src/Modals.js` (통합 모달)
- `scripts/` 디렉토리
- `docs/archive/` 디렉토리

#### 주요 수정
- `src/ipc/mod.rs`: BotConfig 구조체 및 API 추가
- `discord_bot/index.js`: 별칭 시스템 및 도움말 개선
- `electron_gui/src/App.js`: Settings 탭, 별칭 관리 UI, 종료 다이얼로그
- `electron_gui/main.js`: 시스템 트레이, 종료 이벤트 처리
- `modules/*/lifecycle.py`: 프로세스 종료 방식 변경
- `modules/*/module.toml`: game_name, display_name, 명령어 설명 추가

#### 삭제 가능 (통합됨)
- `electron_gui/src/SuccessModal.js`
- `electron_gui/src/FailureModal.js`
- `electron_gui/src/NotificationModal.js`
- `electron_gui/src/QuestionModal.js`

### 🎨 UI/UX 개선

- **Settings 모달**: 일반 설정 / Discord 별칭 탭으로 분리
- **별칭 입력 필드**: 실시간 badge 프리뷰, placeholder 힌트
- **기본값 표시**: 사용자가 비운 필드는 기본값 badge로 표시
- **도움말**: 사용자 설정 별칭만 표시하여 간결성 향상
- **시스템 트레이**: 아이콘 및 컨텍스트 메뉴 추가
- **종료 다이얼로그**: 3가지 선택지 (GUI만 닫기/완전 종료/취소)

### 📝 Known Issues

- ~~서버 정지 기능 불안정~~ ✅ 해결 (실행 파일명 기반으로 변경)
- ~~Discord 별칭 저장 안됨~~ ✅ 해결 (bot-config.json 분리)
- ~~탭 전환 시 별칭 초기화~~ ✅ 해결 (키 존재 여부 체크)

### 🔜 다음 작업 예정

- [ ] 프로덕션 빌드 테스트
- [ ] Discord 봇 end-to-end 테스트
- [ ] 시스템 트레이 아이콘 커스터마이징
- [ ] 에러 핸들링 강화
- [ ] 사용자 문서 업데이트

---

## 기술 스택

- **Core Daemon**: Rust (Tokio, Axum)
- **GUI**: Electron + React 18
- **Discord Bot**: Node.js + discord.js
- **IPC**: REST API (HTTP/JSON)
- **설정 저장**: JSON 파일 (instances.json, bot-config.json, settings.json)
