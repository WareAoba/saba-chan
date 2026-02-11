# Changelog

## [Unreleased] - 2026-02-11

### 🎨 앱 리소스 및 브랜딩

- **앱 아이콘**: `icon.png` → BrowserWindow 아이콘 + 메인 헤더 로고
- **파비콘**: `favicon.png` → 타이틀바 아이콘 + 시스템 트레이 아이콘 (다크/라이트 모드 대응)
- **언어별 로고**: `logo-en.png`, `logo-kr.png`, `logo-jp.png` — 현재 언어에 따라 자동 선택
- **일러스트**: `title.png` (로딩 화면), `panic.png` (에러 모달)
- **로딩 화면 리디자인**: 코닉 그라데이션(sky-blue→purple) 회전 링 + blur 글로우 + 언어별 로고

### 🖥 GUI 개선

- **서버 카드 확장**: 클릭 시 상세 정보 펼침 — PID, Uptime, Port, RCON, REST, Protocol
- **서버 카드 드래그 앤 드롭**: Pointer Events 기반으로 카드 순서 변경 + 백엔드 저장
  - 드래그 시 커서를 따라다니는 카드 + 다른 카드들이 부드럽게 자리 양보 (CSS Transform 애니메이션)
  - 슬롯 위치 스냅샷 기반 타겟 계산 — DOM 재배치 없이 시각적 이동만 수행
  - 드롭 시 최종 순서를 `instances.json`에 저장
- **윈도우 제약**: 최소 크기 780×840
- **헤더 여백 조정**: padding 28px 28px 20px
- **에러 모달 일러스트**: ✕ 텍스트 → `panic.png` 이미지로 교체

### ⚙ 데몬 API

- **서버 시작 시간 API**: `start_time` 필드 추가 — CLI/GUI 모두에서 업타임 표시
- **인스턴스 순서 변경 API**: `PUT /api/instances/reorder` — ID 배열 순서대로 재정렬 + 저장

### 🔧 CLI 개선

- **exit/quit 명령어**: 데몬에 graceful shutdown 요청 후 종료
- **대소문자 무시**: 명령어 입력 시 대소문자 구분 없이 인식
- **업타임 표시**: `status` 명령어에서 서버 가동 시간 표시

### 🐛 버그 수정

- **instances.json 경로**: 하드코딩된 경로 → `%APPDATA%/saba-chan/instances.json`으로 수정
- **BrowserWindow 아이콘**: `favicon.png`(16px) → `icon.png`(고해상도)로 변경

---

## [Unreleased] - 2026-02-10

### 🔧 프로젝트 구조 개편

- **디렉토리 이름 변경**: `electron_gui/` → `saba-chan-gui/`, `cli/` → `saba-chan-cli/`
- **통합 설정 디렉토리**: `%APPDATA%/saba-chan-gui/` → `%APPDATA%/saba-chan/` 로 일원화
  - GUI(Electron)와 CLI 모두 동일한 설정 디렉토리 사용
  - `settings.json`, `bot-config.json`, `cli-settings.json` 통합 관리
- **Electron userData 경로 오버라이드**: `app.setPath('userData', ...)` 로 Electron 기본 경로를 `%APPDATA%/saba-chan`으로 변경
- **기존 AppData 파일 마이그레이션**: `saba-chan-gui/` → `saba-chan/` 자동 이전

### 📦 참조 경로 업데이트 (30+ 파일)

- `saba-chan-cli/Cargo.toml`: 패키지 이름 `saba-cli` → `saba-chan-cli`
- `saba-chan-cli/src/gui_config.rs`: 설정 경로를 `saba-chan`으로 변경
- `saba-chan-cli/src/cli_config.rs`: 설정 경로를 `saba-chan`으로 변경
- `saba-chan-gui/main.js`: Electron userData 경로 오버라이드, 봇 경로 참조 수정
- `.gitignore`: 모든 빌드/출력 경로 업데이트
- `scripts/*.ps1`, `scripts/make-executable.sh`: 빌드 스크립트 경로 업데이트
- `.github/workflows/*.yml`: CI 워크플로우 경로 업데이트
- `README.md`, `TESTING.md`, `WORKFLOWS_GUIDE.md`: 문서 경로 업데이트

### 🧪 테스트 추가

CLI 크레이트에 단위 테스트를 새로 작성 (기존 0개 → **37개**):

- **cli_config** (5개): 기본값, set/get/reset, 직렬화 왕복, 키 목록/별명 검증
- **i18n** (5개): 점 표기법 키 해석, `{{var}}` 템플릿 치환, 로케일 폴백, 로드 동작
- **module_registry** (13개): 모듈 로드, 이름/별명 리졸브(한글 포함), 대소문자 무시, 명령어 별명, 빈/없는 디렉토리, 라이프사이클 상수
- **tui** (14개): `truncate_str` (한글/ASCII/혼합/경계/빈 문자열/zero max), `smart_scroll` (viewport 내/초과/빈 출력/높이 0)

### ✅ 테스트 결과

| 영역 | 테스트 수 | 결과 |
|------|----------|------|
| CLI 단위 테스트 (`cargo test`) | 37 | **PASS** |
| 데몬 통합 테스트 (`daemon_integration`) | 7 | **PASS** |
| GUI 테스트 (`vitest`) | 34 (+1 skipped E2E) | **PASS** |
| **합계** | **78** | **ALL PASS** |

### 🏗 GUI 컴포넌트 리팩토링

- 컴포넌트를 폴더별로 분리: `Modals/`, `StatusBar/`, `TitleBar/`, `Toast/`
- 아이콘 컴포넌트 (`Icon.js`) 추가 — SVG 기반 아이콘 시스템
- 에러 핸들링 유틸리티 (`errorHandler.js`) 추가
- 설정 관리 유틸리티 (`settingsManager.js`) 추가
- i18n 초기화 로직 개선 — localStorage → Electron 설정 → 시스템 언어 폴백 체인


## [Unreleased] - 2026-02-07

#### 국제화(i18n) 시스템 도입 (첫 구현)
- **i18next + react-i18next 의존성 추가**:
  - `i18next` (25.8.4), `react-i18next` (16.5.4), `i18next-browser-languagedetector` (8.2.0) 설치
  - 다국어 번역 파일 기반 동적 언어 전환 기능 구현
- **GUI 로케일 시스템** ([src/i18n.js](electron_gui/src/i18n.js)):
  - 영어(en), 한국어(ko) 지원으로 시작
  - localStorage, Electron settings.json에서 저장된 언어 로드
  - 시스템 언어 감지 및 자동 선택 기능
  - `/locales/{en,ko}/{common.json, gui.json}` 구조
- **메인 프로세스 번역 시스템** ([main.js#L19-L35](electron_gui/main.js#L19-L35)):
  - `loadTranslations()` 함수로 common.json 로드
  - `t(key, variables)` 함수로 간단한 메시지 번역 제공
  - 데몬 시작/설정 로드 시 현재 언어 기반 메시지 표시
- **IPC 언어 관리 핸들러**:
  - `language:get` - 저장된 언어 반환
  - `language:set` - 새로운 언어 저장 및 데몬 재시작 ([main.js#L1183-L1215](electron_gui/main.js#L1183-L1215))
  - `language:getSystem` - 시스템 언어 반환
  - 설정 저장/로드 API와 통합 ([main.js#L140-L155](electron_gui/main.js#L140-L155))
- **Settings 모달 언어 선택 UI** ([SettingsModal.js](electron_gui/src/components/Modals/SettingsModal.js)):
  - 언어 선택 드롭다운 (English, 한국어)
  - 언어 변경 시 localStorage, Electron 설정, i18n 모두 동기화
  - 즉시 UI 업데이트
- **데몬 및 Python 모듈 언어 지원**:
  - `SABA_LANG` 환경 변수로 현재 언어를 데몬/모듈에 전달 ([main.js#L237-L240](electron_gui/main.js#L237-L240))
  - Python 모듈의 i18n 초기화에 사용
  - Daemon의 메시지도 현재 언어로 표시됨
- **모듈 로케일 시스템** ([modules/{palworld,minecraft}/i18n.py](modules/)):
  - `I18n` 클래스 구현 (Python)
  - 각 모듈별 `/locales/{en,ko}.json` 파일
  - `SABA_LANG` 환경 변수 기반 언어 자동 선택
  - Error/message 텍스트를 모두 번역 가능하게 구조화

#### 언어 변경 시 동기화: Discord 봇 자동 재시작
- **IPC 핸들러 확장** ([main.js#L1183-L1215](electron_gui/main.js#L1183-L1215)):
  - `language:set` 핸들러에 Discord 봇 자동 재시작 로직 추가
  - 데몬 + 봇 모두 새로운 언어 설정으로 재시작됨
  - 앱의 언어 변경 → mainWindow에 `bot:relaunch` 이벤트 발송
- **React 앱의 봇 재시작 처리** ([App.js#L581-L598](electron_gui/src/App.js#L581-L598)):
  - `onBotRelaunch` 이벤트 리스너 등록
  - 봇 설정(토큰, 프레픽스, 에이리어스)을 다시 로드하여 재시작
  - 사용자 개입 없이 자동화됨
- **IPC 브릿지 추가** ([preload.js#L30-31](electron_gui/preload.js#L30-31)):
  - `window.api.onBotRelaunch(callback)` 메서드 노출
  - 메인 프로세스와 React 앱 간의 언어 변경 신호 전달

---

## [Unreleased] - 2026-02-04
### 🚀 크로스 플랫폼 지원 및 빌드 시스템 현대화
- **z-index 계층 구조 재정렬** ([Modals.css](electron_gui/src/components/Modals/Modals.css))
  - 에러/확인/질문 모달: 1000 → **9000** (최상위)
  - 설정 모달: 9000 → **2000** (중간)
  - 디스코드/백그라운드 모달: **2000** (중간)
  - 토스트 알림: 9998 → **10000** (최상위)
  - StatusBar: **9999** (유지)
  - 계층 순서: Toast(10000) > StatusBar(9999) > 에러 모달(9000) > 설정 모달(2000)

#### 백그라운드 데몬 모니터링 강화
- **실시간 상태 인디케이터 구현** ([App.js](electron_gui/src/App.js))
  - `backgroundDaemonStatus` 상태 추가 (`checking | running | stopped | error`)
  - 5초마다 `window.api.daemonStatus()` 호출하여 자동 업데이트
  - Background 버튼의 인디케이터가 데몬 상태를 실시간 반영
- **BackgroundModal 개선** ([BackgroundModal.js](electron_gui/src/components/Modals/BackgroundModal.js))
  - 모달 열릴 때 2초마다 상태 확인
  - 상태별 동적 표시: Running(초록) / Stopped(빨강) / Checking(노랑, 펄스 애니메이션)
  - `.status-checking` CSS 클래스 추가 ([App.css](electron_gui/src/App.css#L1077-L1081))
- **데몬 재시작 기능 추가**
  - `daemon:restart` IPC 핸들러 구현 ([main.js#L1057-L1076](electron_gui/main.js#L1057-L1076))
  - `window.api.daemonRestart()` API 노출 ([preload.js#L36](electron_gui/preload.js#L36))
  - 데몬 종료 → 1초 대기 → 재시작 → 2초 대기 로직
  - BackgroundModal에 "🔄 Daemon 재시작" 버튼 추가 (stopped/error 상태일 때만 표시)
  - 보라색 그라데이션 버튼 스타일 ([Modals.css#L958-L986](electron_gui/src/components/Modals/Modals.css#L958-L986))

#### 에러 메시지 사용자 친화화
- **전역 에러 번역 함수 구현** ([App.js#L30-L100](electron_gui/src/App.js#L30-L100))
  - `translateError()` 함수로 50여가지 에러 패턴 변환
  - 파일 경로 오류: "Executable not found" → "게임 서버 실행 파일을 찾을 수 없습니다. 설정에서 경로를 확인해주세요."
  - 네트워크 오류: "ECONNREFUSED" → "데몬에 연결할 수 없습니다. 데몬이 실행 중인지 확인해주세요."
  - 프로세스 오류: "Process not found" → "프로세스를 찾을 수 없습니다. 서버가 비정상 종료되었을 수 있습니다."
  - Discord 오류: "Invalid token" → "Discord 봇 토큰이 올바르지 않습니다. 토큰을 확인해주세요."
- **전역 적용**
  - Discord 봇 시작/정지
  - 모듈/서버 목록 조회
  - 서버 시작/정지/상태 조회
  - 인스턴스 추가/삭제/설정 저장
  - 별명 저장/초기화
  - 모든 토스트 알림 및 모달 메시지

---

## [Unreleased] - 2026-02-04
### � 크로스 플랫폼 지원 및 빌드 시스템 현대화

#### Rust 크로스 플랫폼 마이그레이션
- **ProcessMonitor 개선**: PowerShell 의존성 제거, sysinfo 크레이트로 전환
  - Windows 전용 PowerShell 명령 제거 (Get-Process, tasklist 등)
  - `sysinfo::System`을 사용한 크로스 플랫폼 프로세스 모니터링
  - PID 확인, 프로세스 목록 조회 모두 네이티브 API 사용
- **PathDetector 확장**: Linux/macOS 게임 경로 지원 추가
  - Windows: Steam, Epic Games 경로 유지
  - Linux: `~/.steam/steam`, `~/.var/app/com.valvesoftware.Steam` (Flatpak)
  - macOS: `~/Library/Application Support/Steam`
- **Process Termination**: 플랫폼별 프로세스 종료 구현
  - Windows: `winapi` 크레이트 (`TerminateProcess`, `PROCESS_TERMINATE`)
  - Unix: `nix` 크레이트 (`kill()`, SIGTERM/SIGKILL 시그널)
- **Electron GUI**: `taskkill` 명령 크로스 플랫폼 처리 강화
  - Windows/Unix 분기 처리 개선 ([main.js#L95-L110](electron_gui/main.js#L95-L110))

#### Vite/Vitest 빌드 시스템 마이그레이션
- **Webpack → Vite 전환**:
  - `react-scripts` 제거, `vite` 6.0.7 설치
  - `vite.config.js` 생성 (JSX 로더, 포트 5173 설정)
  - 개발 서버 포트: localhost:3000 → localhost:5173
  - esbuild 기반 빌드로 성능 향상
- **Jest → Vitest 마이그레이션**:
  - 모든 테스트 파일 변환 (App.test.js, integration.test.js, main.test.js)
  - `jest.fn()` → `vi.fn()`, `jest.useFakeTimers()` → `vi.useFakeTimers()`
  - `setupTests.js`를 Vitest 방식으로 변경
  - 34개 테스트 모두 통과 (1개 스킵)
- **package.json 스크립트 업데이트**:
  - `dev`: `vite` (기존 `react-scripts start`)
  - `test`: `vitest` (기존 `react-scripts test`)
  - `build`: `vite build` (기존 `react-scripts build`)
- **의존성 정리**: 72개 패키지 추가, 1098개 제거, 총 551개 패키지

#### Cargo.toml 의존성 추가
```toml
[dependencies]
sysinfo = "0.30"  # 크로스 플랫폼 시스템 정보

[target.'cfg(unix)'.dependencies]
nix = { version = "0.27", features = ["signal"] }  # Unix 시그널 처리

[target.'cfg(windows)'.dependencies]
winapi = { version = "0.3", features = ["processthreadsapi", "handleapi"] }
```

#### 빌드 검증
- **Rust**: `cargo build --release` 성공 (38.32초, 6.2MB 바이너리)
- **Electron GUI**: Vite 개발 서버 정상 실행 (localhost:5173)
- **전체 테스트**: 
  - Rust: 91개 테스트 통과
  - GUI: 34개 테스트 통과 (Vitest)
  - Bot: 17개 테스트 통과
  - **총 142개 테스트 통과** 🎉

---

### �🔧 CI/CD 및 코드 품질 개선

#### Clippy 에러 수정 (15개 해결)
- **unit struct default() 제거**: ProcessMonitor, ProcessManager에서 `Self::default()` → `Self` 변경
- **dead_code 경고 처리**: 미사용 함수/구조체에 `#[allow(dead_code)]` 추가
  - PluginManager, PathDetector, ProcessManager::execute_command
  - ProtocolError variants (TimeoutError, ConfigError, Unknown)
  - ServerResponse::error, RestClient::with_basic_auth
  - ProtocolClient::new_both, connect_all
- **enum variant 이름 충돌 해결**: `ProtocolError::ProtocolError` → `ProtocolError::Protocol`
- **코드 스타일 개선**:
  - needless return 제거 (5곳)
  - needless_borrows 수정
  - matches! 매크로 사용
  - or_else → or 변경 (2곳)
  - std::ptr 미사용 import 제거
- **결과**: `cargo clippy -- -D warnings` 통과 ✅

#### GitHub Actions 캐시 문제 해결
- **문제**: npm ci 실행 시 `yaml@2.8.2` 캐시 불일치 오류
- **해결**: 모든 워크플로우에서 `npm ci` 전 `rm -rf node_modules` 추가
  - test.yml, quick-test.yml, coverage.yml 수정
  - 깨끗한 상태에서 의존성 설치 보장

#### 테스트 결과
- **Rust**: 51개 테스트 통과 (Unit 42개 + Integration 7개 + Stress 2개)
- **GUI**: 34개 테스트 통과 (1개 스킵)
- **Discord Bot**: 17개 테스트 통과
- **총 91개 테스트 통과** 🎉

---

## [Unreleased] - 2026-02-03
### 🎨 GUI 대규모 리팩토링

#### 게임 아이콘 시스템 구현 ([module_loader.rs](src/supervisor/module_loader.rs), [mod.rs](src/ipc/mod.rs), [App.js](electron_gui/src/App.js))
- **모듈별 게임 아이콘 지원**:
  - `ModuleMetadata`에 `icon` 필드 추가 (module.toml에서 읽음)
  - `ModuleInfo`에 `icon` 필드 추가 (base64 인코딩된 이미지 데이터)
  - module.toml에 `icon = "icon.png"` 설정 추가
- **아이콘 로딩 및 전송**:
  - 백엔드에서 모듈 폴더의 `icon.png` 파일을 읽어 base64로 인코딩
  - `/api/modules` 응답에 `data:image/png;base64,...` 형식으로 포함
  - 프론트엔드에서 base64 이미지를 `<img>` 태그로 직접 표시
- **폴백 처리**: 아이콘이 없으면 gamepad 아이콘 placeholder 표시
- **구현 위치**: 
  - 각 모듈 폴더에 `icon.png` 배치 (예: `modules/palworld/icon.png`)
  - `list_modules`와 `refresh_modules`에서 base64 인코딩 처리

#### 서버 카드 UI 완전 재설계 ([App.js](electron_gui/src/App.js), [App.css](electron_gui/src/App.css))
- **게임 아이콘 영역 추가**: 각 서버 카드에 게임 아이콘 표시 (40x40px)
- **상태 버튼 개선**:
  - 인디케이터 + 텍스트 조합으로 변경
  - 호버 시 텍스트만 변경 (실행중 ↔ 정지, 정지중 ↔ 실행)
  - 인디케이터 위치 고정 (min-width로 레이아웃 안정화)
- **카드 접기/펼치기 기능**:
  - 헤더 클릭 시 상세 정보 및 액션 버튼 표시/숨김
  - max-height 트랜지션 (0.4s ease) 적용
  - expanded 상태를 fetchServers 시에도 보존
- **액션 아이콘 버튼 재배치**: 설정, 정보, 명령어/삭제 아이콘을 카드 하단에 배치
- **스타일 최적화**:
  - 여백 대폭 축소 (padding 12px, gap 최소화)
  - 폰트 크기 조정 (서버 이름 16px, 게임 이름 12px)
  - 호버 시 배경 fill 제거, transform만 적용

#### 레이아웃 구조 리팩토링
- **main 태그 추가**: 스크롤 영역을 명확히 분리 (.app-main)
- **그리드 레이아웃 개선**:
  - 카드 최대 2개 제한 (max-width: 1200px)
  - 각 카드 높이 독립성 보장 (align-items: start)
  - 최소 카드 폭 480px로 증가
- **스크롤 영역 최적화**:
  - App 컨테이너: height 100vh, overflow hidden
  - app-main: flex 1, overflow-y auto
  - 호버 시 카드 상단 잘림 방지 (padding-top 10px)

#### AddServerModal 개선
- 서버 추가를 모달 방식으로 변경
- 모듈 선택 후 서버 이름 입력하는 2단계 프로세스

#### 버그 수정
- **fetchServers 시 expanded 상태 보존**: 자동 새로고침 시 펼친 카드가 닫히던 문제 해결
- **중복 CSS 제거**: .server-list 중복 정의 제거
- **창 최소 크기 제한**: 400x500으로 설정 ([main.js](electron_gui/main.js))

---

## [Unreleased] - 2026-02-01
### 🎮 Palworld 플레이어 ID 자동 변환 기능

#### kick/ban/unban 명령어 개선 ([lifecycle.py](modules/palworld/lifecycle.py))
- **닉네임 → Steam ID 자동 변환**: 사용자가 닉네임을 입력하면 자동으로 Steam ID로 변환
  - `accountName` (Steam 계정 이름) 검색 지원
  - `name` (게임 내 캐릭터 이름) 검색 지원
  - 정확 일치 및 부분 일치 검색 지원
- **직접 REST 요청**: Daemon 데드락 방지를 위해 Palworld 서버에 직접 요청
  - `resolve_player_id()` 함수 개선
  - `execute_rest_direct()` 함수 추가

#### GUI 명령어 라우팅 변경 ([main.js](electron_gui/main.js))
- **플레이어 명령어 분기 처리**: kick, ban, unban 명령어를 `/api/instance/:id/command` 엔드포인트로 라우팅
- **Python 모듈 연동**: 플레이어 ID 변환 로직이 있는 Python 모듈을 통해 명령어 실행

#### 사용 예시
```
kick KimchiMayo        → steam_76561199507076069 으로 자동 변환 후 kick 실행
kick 김마무            → 캐릭터 이름으로도 검색 가능
ban KimchiMayo         → 동일하게 자동 변환 후 ban 실행
kick steam_76561199507076069  → Steam ID 직접 입력도 가능
```

---

## [Unreleased] - 2026-01-20
### 🤖 Discord 봇 개선

#### 메시지 수정 방식 적용 ([discord_bot/index.js](discord_bot/index.js))
- **변경 전**: "⏳ 실행 중..." 메시지와 "✅ 완료!" 메시지가 각각 전송됨 (2개)
- **변경 후**: "⏳ 실행 중..." 메시지가 "✅ 완료!"로 수정됨 (1개)
- **적용 범위**: start, stop, REST 명령어 모두
- **구현**: `message.reply()` 반환값을 저장 후 `.edit()`로 메시지 내용 갱신

#### 중복 메시지 처리 방지 ([discord_bot/index.js](discord_bot/index.js))
- **문제**: Discord.js 이벤트 중복으로 같은 명령이 두 번 실행되는 현상
- **해결**: `processedMessages` Set으로 메시지 ID 캐싱, 5초 TTL로 중복 필터링
- **효과**: 동일 메시지에 대한 중복 API 호출 방지

### 🎨 GUI 개선

#### 모듈 명령어 별명 UI 확장 ([App.js](electron_gui/src/App.js))
- **변경 전**: `[aliases.commands]`에 정의된 명령어만 별명 편집 가능
- **변경 후**: `[commands.fields]`의 REST 명령어도 별명 지정 가능
- **표시 정보**: 명령어 영문명 + 한글 라벨 (예: `announce (공지사항 전송)`)
- **대상 명령어**: announce, info, players, metrics, settings, save, shutdown, kick, ban, unban 등

#### CSS 스타일 추가 ([App.css](electron_gui/src/App.css))
- `.cmd-label` 스타일: 명령어 한글 라벨 표시용

---
### � Discord 봇 REST 명령어 지원 추가

#### module.toml commands 연동 ([discord_bot/index.js](discord_bot/index.js))
- **명령어 자동 로드**: `/api/modules`에서 commands 필드 파싱하여 사용 가능한 명령어 목록 구축
- **REST 명령어 실행**: `!saba palworld players`, `!saba pw info` 등 REST API 호출 지원
- **입력 파라미터 처리**: 필수 인자 검증 및 누락 시 안내 메시지 표시
- **응답 포맷팅**: players, info, metrics 등 주요 명령어 결과를 읽기 쉬운 형식으로 출력

#### 도움말 시스템 개선
- `!saba` - 전체 도움말 + 모듈별 사용 가능 명령어 표시
- `!saba <모듈>` - 해당 모듈의 모든 명령어 목록 및 사용법 표시
- `!saba <모듈> <명령어>` - 명령어 실행

#### 사용 예시
```
!saba palworld players     → 접속 중인 플레이어 목록
!saba pw info              → 서버 정보 조회
!saba palworld announce 안녕하세요  → 공지 전송
!saba pw kick steam_xxxxx  → 플레이어 강퇴
```

---

### �🧪 테스트 강화

#### Rust 백엔드 테스트 추가 ([ipc/mod.rs](src/ipc/mod.rs))
- **ModuleInfo 직렬화 테스트**: commands 필드 포함 여부 검증
- **ModuleListResponse 테스트**: 다중 명령어, http_method, inputs 필드 검증
- **HTTP 메서드 파싱 테스트**: GET/POST/PUT/DELETE 및 기본값 처리 확인
- **CommandInput 직렬화 테스트**: 필수 필드와 type 필드 직렬화 검증
- **테스트 결과**: 6개 테스트 모두 통과 ✅

#### React 프론트엔드 테스트 추가 ([App.test.js](electron_gui/src/test/App.test.js))
- **safeShowToast 안전 호출 테스트**: window.showToast 미정의 시 에러 방지 확인
- **모듈 목록 API 응답 테스트**: commands 필드 포함 및 http_method 검증
- **REST 명령어 실행 테스트**: GET/POST 메서드 및 body 전송 검증
- **연결 실패 테스트**: 서버 목록/모듈 로드 실패 시 토스트 표시 확인
- **테스트 결과**: 25개 테스트 모두 통과 ✅

### 🐛 버그 수정

#### 서버 종료 후 상태 즉시 반영 안 되는 문제 해결 ([ipc/mod.rs](src/ipc/mod.rs))
- **문제**: GUI에서 서버 종료해도 상태가 "running"으로 유지됨
- **원인**: `stop_server_handler`가 프로세스 종료 후 tracker에서 untrack하지 않음
- **해결**: 서버 종료 성공 시 즉시 `tracker.untrack()` 호출하여 상태 즉시 반영
- **결과**: 서버 종료 후 바로 "stopped" 상태로 전환, Stop 버튼이 Start로 변경됨

#### safeShowToast 헬퍼 추가 ([App.js](electron_gui/src/App.js))
- **문제**: Toast 컴포넌트 마운트 전 `window.showToast` 호출 시 에러 발생
- **해결**: `safeShowToast()` 래퍼 함수로 안전한 호출 보장
- **영향 범위**: Discord 봇 시작/정지, 모듈 로드, 서버 목록 업데이트

#### Settings 반복 저장 문제 해결 ([App.js](electron_gui/src/App.js))
- **문제**: 설정이 매 렌더마다 반복 저장됨
- **원인**: useEffect 의존성과 handleStartDiscordBot에서의 중복 저장 호출
- **해결**: 
  - useRef로 이전 값 추적하여 실제 변경 시에만 저장
  - handleStartDiscordBot에서 불필요한 saveCurrentSettings() 호출 제거

---

## [Unreleased] - 2026-01-19

### 🎮 REST 명령어 시스템 완성

#### HTTP 클라이언트 실제 구현
- **ureq 기반 실제 HTTP 요청**: 스텁 코드를 실제 HTTP 클라이언트로 교체 ([ipc/mod.rs](src/ipc/mod.rs))
- **Basic Auth 지원**: 사용자명/비밀번호 인증으로 Palworld REST API 호출
- **응답 텍스트 캡처**: `response_text` 필드 추가로 서버 응답 메시지 GUI에 표시

#### 모듈 명령어 메타데이터 전달 수정 (핵심 버그 수정)
- **ModuleInfo 구조체 확장** ([ipc/mod.rs](src/ipc/mod.rs)):
  - `commands: Option<ModuleCommands>` 필드 추가
  - `list_modules`, `refresh_modules` 함수에서 commands 매핑 추가
- **문제**: GUI에서 `commandMetadata`가 undefined로 전달되어 HTTP 메서드 정보 누락
- **원인**: API 응답에 commands 필드가 포함되지 않았음
- **해결**: ModuleInfo에 commands 필드 추가하여 React까지 메타데이터 전달

#### module.toml 명령어 정의 완성
- **Palworld 모듈** ([modules/palworld/module.toml](modules/palworld/module.toml)):
  - 10개 REST 엔드포인트 완전 정의 (info, players, metrics, settings, announce, save, shutdown, kick, ban, unban)
  - `http_method` 필드로 GET/POST 구분
  - `inputs` 스키마로 명령어 파라미터 정의

#### 입력 검증 레이어 추가
- **React 검증**: CommandModal에서 필수 필드 검증
- **Node.js 검증**: main.js에서 타입 및 기본값 처리

---

### 🔧 백엔드 안정성 개선 (간헐적 종료 문제 해결)

#### ProcessMonitor 강화
- **안전한 오류 처리**: PowerShell 명령 실패 시 Panic 대신 빈 목록 반환 ([process_monitor.rs](src/process_monitor.rs))
- **CSV 파싱 오류 복원력**: 파싱 실패 줄은 무시하고 계속 진행
- **상세 로깅**: PowerShell 오류 시 경고 레벨로 로깅 추가

#### 모니터링 루프 강화
- **오류 카운팅 및 자동 리셋** ([main.rs](src/main.rs)): 
  - 연속 10회 이상 오류 시 자동 리셋하여 무한 루프 방지
  - 오류 횟수 추적으로 시스템 상태 모니터링
- **로그 반복 방지**: 첫 3회 오류와 이후 10회마다만 로깅하여 로그 폭증 방지
- **세밀한 모니터 통계**: 추적 서버 수, 자동 감지 수 등 상세 정보 로깅

#### ProcessTracker 안전성 개선
- **뮤텍스 데드락 방지** ([process.rs](src/supervisor/process.rs)):
  - 모든 `.unwrap()` 호출 → `match` 패턴으로 변경
  - 잠금 획득 실패 시 로깅 후 정적 오류 반환
  - 데드락 시 Panic 대신 에러 값 반환
- **모든 메서드에 안전한 오류 처리**:
  - `track()`, `get_status()`, `get_pid()`, `untrack()` 등 모든 함수 업데이트

#### 코드 품질 개선
- **경고 제거**:
  - `mut` 불필요한 변수 제거 ([supervisor/mod.rs](src/supervisor/mod.rs))
  - snake_case 필드명 적용: `moduleAliases` → `module_aliases` ([ipc/mod.rs](src/ipc/mod.rs))
  - serde rename으로 이전 필드명 호환성 유지
- **테스트 강화**:
  - 모든 ProcessTracker 메서드 단위 테스트 추가
  - 16개 라이브러리 테스트 모두 통과 ✅

#### 스트레스 테스트 추가
- **안정성 시뮬레이션** ([tests/stress_test.rs](tests/stress_test.rs)):
  - test_concurrent_mutex_access: 20 스레드 × 100회 = 2,000회 동시 뮤텍스 획득 ✅
  - test_process_detection_loop: 50회 PowerShell 호출 → 리소스 누수 없음 ✅
  - test_error_recovery_in_parsing: CSV 파싱 오류 복원력 ✅
  - test_error_logging_throttle: 50회 오류 → 13-15개 로그만 출력 ✅
  - test_thread_safe_hashmap_access: 10 스레드 × 100회 = 1,000회 HashMap 수정 ✅
  - test_memory_allocation_cleanup: 10,000개 × 1KB 할당/해제 → 메모리 누수 없음 ✅
  - test_no_panic_on_common_errors: 모든 에러 상황에서 Panic 없음 ✅
- **테스트 결과**: 7개 테스트 모두 통과 (약 9.5초)

#### 문서 추가
- [BACKEND_TESTING_GUIDE.md](docs/BACKEND_TESTING_GUIDE.md) 작성:
  - 개선사항 요약
  - 테스트 실행 방법 및 결과
  - 모니터링 명령어
  - 다음 단계 로드맵

---

### 🎨 GUI 개선

#### 로딩 화면 추가
- **초기 로딩 화면**: Daemon 준비 전 표시되는 로딩 화면 추가 ([App.js](src/App.js) / [App.css](src/App.css))
  - 🐟 물고기 로고 + 플로팅 애니메이션 (`@keyframes float`)
  - 진행률 바 (0% → 100%) + 상태 메시지 (초기화 시작, 데몬 준비, 모듈 로드, 인스턴스 로드, 준비 완료)
  - 팁 표시 영역 (무작위 팁 순환)
  - State 추가: `daemonReady`, `initStatus`, `initProgress`
  - IPC 이벤트: `status:update` 리스너로 진행상황 실시간 업데이트
- **서버 카드 초기화 로딩**: 서버 상태 안정화 대기 중 오버레이 표시 ([App.js](src/App.js) / [App.css](src/App.css))
  - 반투명 흰색 배경 + blur 효과 (`.servers-initializing-overlay`)
  - 스피너 + "서버 상태 확인 중..." 텍스트
  - State 추가: `serversInitializing`
  - 타이머: ready 상태 수신 후 3.5초 후 자동 제거
  - 조건부 렌더링: `serversInitializing && servers.length > 0` 일 때만 표시

#### 앱 구동 방식 개선 (Architecture 변경)
- **변경 전**: 
  - Rust Daemon 완료 대기 → GUI 창 렌더링
  - 프로세스: Daemon 시작 → Daemon 초기화 (3-5초) → GUI 표시 (추가 2-3초)
  - 총 소요 시간: 5-7초
- **변경 후**:
  - GUI 창 즉시 렌더링 (로딩 화면) → Daemon 백그라운드 초기화
  - 프로세스: GUI 창 표시 (즉시) → 로딩 화면 표시 → Daemon IPC 통신으로 진행률 업데이트
  - 총 소요 시간: 1-2초 (체감 개선)
  - 이점: 사용자 반응성 향상, 진행 상황 시각화

#### CSS 애니메이션 개선
- **Spinner 애니메이션**: `.loading-spinner` 회전 (`@keyframes spin`)
  - 1.5초 순환, 선형 무한 반복
- **로고 플로팅 애니메이션**: `.loading-logo` 상하 움직임 (`@keyframes float`)
  - 3초 순환, ease-in-out
- **진행률 바 애니메이션**: `.loading-progress-fill` 너비 변경
  - 0.3초 transition, ease-out

### 🔧 기술적 변경

#### Fluent Icons 시도 후 롤백 (`package.json` / `package-lock.json`)
- `@fluentui/react-icons` 패키지 설치 시도
  - npm으로 설치 후 React 컴포넌트에 적용
  - 이모지 아이콘 → Fluent UI 아이콘 변경 시도
- 문제 발생: webpack 컴파일 실패
  - 여러 아이콘 이름 오류 (예: 잘못된 심볼 참조)
  - 대소문자 구분 문제로 인한 모듈 로드 실패
- **최종 해결**: 패키지 제거 및 이모지 유지
  - 아이콘 통합이 필요한 추후 프로젝트에서는 Lucide React 검토 필요
  - 현재는 이모지로 충분한 시각적 구별성 확보

#### GitHub Actions 수정 (`.github/workflows/test.yml`)
- **문제**: 
  - `npm ci`: package.json과 package-lock.json 불일치 오류
  - npm 캐시: 이전 버전의 오래된 패키지 참조
- **해결**:
  - `npm ci` → `npm install` 변경
  - npm 캐시 설정 제거 (`cache: 'npm'` 라인 삭제)
  - cache-dependency-path 설정 제거
- **결과**: CI/CD 파이프라인 성공 (테스트 실행 가능)

### 🧪 테스트 추가

#### 로딩 화면 테스트 ([App.test.js](src/test/App.test.js))
- **초기 로딩 화면 표시 테스트**: 
  - `daemonReady=false` 상태에서 로딩 화면 렌더링 확인
  - 초기화 메시지 표시 검증
- **ready 상태 전환 테스트**: 
  - `onStatusUpdate` 콜백으로 `ready` 상태 수신 시뮬레이션
  - 600ms 후 `daemonReady=true` 전환 확인
  - 로딩 화면 사라짐 및 메인 UI 표시 검증
- **서버 카드 초기화 로딩 타이머 테스트**: 
  - `jest.useFakeTimers()` 사용
  - ready 상태 수신 후 3.5초 경과 시뮬레이션
  - `serversInitializing=false` 전환 확인
  - 오버레이 제거 검증

---

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

---