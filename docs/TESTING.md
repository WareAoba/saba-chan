# Saba-chan 통합 테스트 가이드

## 📋 목차
1. [테스트 개요](#테스트-개요)
2. [빠른 시작](#빠른-시작)
3. [Rust Daemon 테스트](#rust-daemon-테스트)
4. [Electron GUI 테스트](#electron-gui-테스트)
5. [Discord Bot 테스트](#discord-bot-테스트)
6. [통합 테스트](#통합-테스트)
7. [CI/CD 테스트](#cicd-테스트)

---

## 테스트 개요

### 전체 테스트 구조

```
프로젝트
├── Rust Daemon (37 테스트)
│   ├── API 테스트 (9개)
│   ├── 모듈 테스트 (8개)
│   ├── 스트레스 테스트 (9개)
│   └── 에러 처리 (11개)
│
├── Electron GUI (34 테스트)
│   ├── 단위 테스트 (29개)
│   └── 통합 테스트 (5개, 1 스킵)
│
└── Discord Bot (17 테스트)
    ├── 단위 테스트 (14개)
    └── 통합 테스트 (3개)
```

**📈 총 88개 테스트**

---

## 빠른 시작

### 모든 테스트 실행

```powershell
# PowerShell 스크립트로 전체 테스트 (권장)
.\scripts\test-integration.ps1

# 또는 수동으로
cargo test                    # Rust (30-40초)
cd electron_gui && npm test   # GUI (10-20초)
cd discord_bot && npm test    # Bot (5-10초)
```

### 빠른 테스트 (JavaScript만)

```powershell
# Rust 컴파일 없이 GUI와 Bot만
cd electron_gui && npm test -- --watchAll=false
cd discord_bot && npm test
```

---

## Rust Daemon 테스트

### 테스트 구조

```
tests/
├── daemon_integration.rs     # 메인 엔트리포인트
├── stress_test.rs            # 스트레스 테스트
└── daemon/
    ├── api_tests.rs          # HTTP API (9개)
    ├── module_tests.rs       # 모듈 로더 (8개)
    └── error_handling_tests.rs  # 에러 처리 (11개)
```

### 실행 방법

```powershell
# 모든 통합 테스트
cargo test

# 특정 카테고리만
cargo test api_tests
cargo test module_tests
cargo test error_handling

# 상세 출력
cargo test -- --nocapture

# 병렬 실행 제어
cargo test -- --test-threads=4
```

### 주요 테스트

#### 1. API 테스트 (api_tests.rs)

| 테스트 | 검증 내용 |
|--------|----------|
| `test_api_modules_list` | GET /api/modules |
| `test_api_servers_list` | GET /api/servers |
| `test_api_instance_crud` | 인스턴스 생성→조회→수정→삭제 |
| `test_api_bot_config` | Bot Config 읽기/쓰기 |
| `test_api_error_handling` | 404, 400 에러 응답 |
| `test_api_module_refresh` | POST /api/modules/refresh |
| `test_api_concurrent_requests` | 10개 동시 요청 처리 |

#### 2. 모듈 테스트 (module_tests.rs)

| 테스트 | 검증 내용 |
|--------|----------|
| `test_module_discovery` | modules/ 디렉토리 스캔 |
| `test_module_metadata_structure` | TOML 필드 유효성 |
| `test_module_refresh` | 캐시 무효화 및 재발견 |
| `test_python_plugin_detection` | Python 경로 탐지 |
| `test_python_plugin_execution` | lifecycle.py 실행 |
| `test_module_hot_reload` | 핫 리로드 일관성 |

#### 3. 에러 처리 (error_handling_tests.rs)

| 테스트 | 검증 내용 |
|--------|----------|
| `test_missing_module_toml` | TOML 파일 없음 |
| `test_malformed_toml` | 잘못된 TOML 포맷 |
| `test_python_plugin_failure` | Python 실행 실패 |
| `test_invalid_instance_id` | 존재하지 않는 ID |
| `test_corrupted_instances_json` | 손상된 JSON |

---

## Electron GUI 테스트

### 테스트 파일

```
electron_gui/src/
├── test/
│   ├── App.test.js           # React 컴포넌트 (26개)
│   ├── main.test.js          # Electron Main (8개)
│   └── integration.test.js   # E2E (1개, 스킵)
└── setupTests.js             # Jest 설정
```

### 실행 방법

```powershell
cd electron_gui

# 모든 테스트
npm test -- --watchAll=false

# Watch 모드 (자동 재실행)
npm test

# 커버리지 포함
npm test -- --coverage

# 특정 파일만
npm test App.test.js
npm test main.test.js
```

### 주요 테스트

#### 1. React 컴포넌트 (App.test.js)

```javascript
describe('App Component', () => {
  // 렌더링 테스트
  test('renders without crashing');
  test('displays server cards');
  test('updates server list');
  
  // 사용자 인터랙션
  test('opens add server modal');
  test('creates new server');
  test('deletes server');
  test('saves settings');
  
  // 에러 처리
  test('shows error on API failure');
  test('handles network timeout');
});
```

#### 2. Electron Main (main.test.js)

```javascript
describe('IPC Handlers', () => {
  test('getServers - returns server list');
  test('createServer - saves instance');
  test('deleteServer - removes instance');
  test('executeCommand - routes to daemon');
  test('error handling - invalid request');
});
```

#### 3. E2E 테스트 (integration.test.js)

**현재 상태**: `test.skip()` - axios ESM import 문제로 스킵
- Jest와 axios ESM 비호환
- 수동 E2E 테스트 권장 (실제 앱 실행)

---

## Discord Bot 테스트

### 테스트 파일

```
discord_bot/
├── test/
│   └── integration.test.js   # 통합 테스트 (17개)
└── utils/
    └── aliasResolver.test.js # (통합됨)
```

### 실행 방법

```powershell
cd discord_bot

# 모든 테스트
npm test

# 특정 파일만
npm test integration.test.js
```

### 주요 테스트

#### 별명 해석 & 통합 테스트

```javascript
describe('Bot Integration', () => {
  test('buildModuleAliasMap - pw → palworld');
  test('buildCommandAliasMap - 플레이어 → players');
  test('resolveAlias - full chain');
  test('parses Discord message');
  test('executes command via daemon');
  test('formats response');
});
```

---

## 통합 테스트

### E2E 워크플로우

**1. Daemon 시작**
```powershell
cargo run --release
```

**2. GUI 테스트**
```powershell
# GUI 앱 시작
cd electron_gui
npm start

# 테스트 시나리오:
# 1. 서버 추가 (Minecraft/Palworld)
# 2. 설정 저장
# 3. 명령어 실행 (💻 Command 버튼)
# 4. 서버 삭제
```

**3. Bot 테스트**
```powershell
# Discord 봇 시작
cd discord_bot
node index.js

# Discord에서 테스트:
# !saba palworld info
# !saba pw players
# !saba minecraft list
```

### 전체 시스템 플로우

```
Discord 메시지
  ↓
Discord Bot (Node.js)
  ↓ HTTP
Core Daemon (Rust)
  ↓ RCON/REST
Game Server
```

---

## CI/CD 테스트

### GitHub Actions

**.github/workflows/test.yml**
- ✅ Rust 테스트 (cargo test)
- ✅ GUI 테스트 (npm test)
- ✅ Bot 테스트 (npm test)
- ✅ 빌드 검증 (cargo build)

**.github/workflows/coverage.yml**
- 코드 커버리지 수집
- Codecov 업로드

**.github/workflows/quick-test.yml**
- PR용 빠른 테스트
- JavaScript만 실행

### 로컬 CI 시뮬레이션

```powershell
# PowerShell에서
.\scripts\test-integration.ps1

# 실행 내용:
# 1. 환경 정보 출력
# 2. Rust 테스트
# 3. GUI 테스트
# 4. Bot 테스트
# 5. 실행 시간 측정
```

---

## 문제 해결

### Rust 테스트 실패

**원인**: Daemon이 이미 실행 중
```powershell
# 해결: Daemon 종료
taskkill /F /IM core_daemon.exe
```

**원인**: 캐시 문제
```powershell
# 해결: 클린 빌드
cargo clean
cargo test
```

### GUI 테스트 타임아웃

**원인**: `instances.json` 잠금
```powershell
# 해결: 파일 권한 확인
icacls instances.json
```

**원인**: API 서버 미응답
```powershell
# 해결: Daemon 재시작
cargo run --release
```

### Bot 테스트 실패

**원인**: axios ESM import (integration.test.js)
- **현재**: test.skip()으로 스킵됨
- **해결**: 수동 E2E 테스트 권장

---

## 테스트 커버리지

### 현재 커버리지

| 컴포넌트 | 테스트 수 | 커버리지 |
|---------|----------|---------|
| Rust Daemon | 37 | ~85% |
| Electron GUI | 34 | ~70% |
| Discord Bot | 17 | ~60% |

### 미래 개선 계획

- [ ] GUI E2E 테스트 (Playwright/Puppeteer)
- [ ] Bot E2E 테스트 (Discord.js mocking)
- [ ] 시각적 회귀 테스트
- [ ] 성능 벤치마크

---

## 참고 자료

- **API 스펙**: [API_SPEC.md](API_SPEC.md)
- **프로젝트 가이드**: [PROJECT_GUIDE.md](PROJECT_GUIDE.md)
- **빠른 시작**: [QUICK_START.md](QUICK_START.md)
- **사용 가이드**: [USAGE_GUIDE.md](USAGE_GUIDE.md)
