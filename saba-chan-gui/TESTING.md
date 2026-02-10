# GUI 테스트 가이드

## 📊 테스트 현황

### ✅ 자동화된 테스트 (Jest)
```powershell
npm test                      # 모든 유닛/통합 테스트
npm test -- --watchAll=false  # Watch 모드 없이 실행
npm run test:coverage         # 커버리지 리포트
```

**테스트 스위트:**
- ✅ `App.test.js` - 앱 로직, 설정, 봇 자동실행 (25 tests)
- ✅ `main.test.js` - Electron 메인 프로세스 (9 tests)
- ⏭️ `integration.test.js` - E2E 테스트 (1 skipped)

**총 테스트:** 54개
- **통과:** 38개
- **실패:** 15개 (App.test.js 일부)
- **스킵:** 1개 (integration E2E)

### ⚠️ 알려진 문제

1. **App.test.js 일부 실패** (15 tests)
   - 원인: window.api.serverList 모킹 누락
   - 해결 방법: setupTests.js에 `serverList` 추가

2. **Integration 테스트 스킵**
   - 원인: Jest + axios ESM import 비호환
   - 대안: 수동 E2E 테스트 (아래 참고)

---

## 🔧 테스트 로그 개선

### 변경 사항
- ✅ 테스트 환경에서 디버깅 로그 억제
- ✅ 진행 상황 표시 (이모지 + 단계별 메시지)
- ✅ 타임아웃 증가 (5s → 10s)
- ✅ `setupTests.js`에서 로그 필터링

### 효과
**이전:**
```
console.log [Settings] Loaded: {...}
console.warn Attempt 1 failed, retrying...
console.warn Attempt 2 failed, retrying...
(수백 줄의 디버깅 로그...)
```

**개선 후:**
```
PASS src/test/main.test.js
FAIL src/test/App.test.js
  (에러만 표시)
Test Suites: 2 failed, 1 passed, 3 total
```

---

## 🚀 수동 E2E 테스트

Integration 테스트가 Jest와 호환되지 않아, 수동으로 E2E 테스트를 수행할 수 있습니다.

### 방법 1: GUI 앱 직접 실행

```powershell
# 1. Daemon 빌드 및 실행
cargo build --release
.\target\release\core_daemon.exe

# 2. 별도 터미널에서 GUI 앱 실행
cd saba-chan-gui
npm start

# 3. GUI에서 수동 테스트:
# - 서버 생성/삭제
# - 설정 변경
# - 봇 시작/중지
# - 모듈 로드
```

### 방법 2: 빠른 테스트 스크립트

```powershell
.\scripts\test-gui.ps1
```

이 스크립트는:
1. Daemon 빌드 확인
2. 빌드되지 않았다면 자동 빌드
3. GUI 앱 시작
4. 수동 테스트 안내

---

## 📝 CI/CD (GitHub Actions)

`.github/workflows/test.yml`에서 자동 실행:
- ✅ Rust Daemon 테스트 (7 tests)
- ✅ Electron GUI 테스트 (39 passing)
- ✅ Discord Bot 테스트
- ⏭️ Integration 테스트 스킵

**커버리지 리포트:** `.github/workflows/coverage.yml`

---

## 🛠️ 개발자 가이드

### 새 테스트 추가

**Unit Test (App.test.js):**
```javascript
test('새 기능 테스트', async () => {
    // Arrange
    global.window.api.someMethod = jest.fn().mockResolvedValue(
        { data: 'mocked' }
    );
    
    // Act
    render(<App />);
    
    // Assert
    await waitFor(() => {
        expect(mockApi.someMethod).toHaveBeenCalled();
    });
});
```

### 로그 필터링 수정

`setupTests.js`에서 억제할 로그 패턴 추가:
```javascript
console.log = (...args) => {
    const msg = args.join(' ');
    if (!msg.includes('YOUR_PATTERN')) {
        originalConsoleLog(...args);
    }
};
```

### 타임아웃 조정

특정 테스트가 느리면:
```javascript
test('느린 테스트', async () => {
    // ...
}, 15000); // 15초 타임아웃
```

---

## 📌 TODO

- [ ] App.test.js 실패 테스트 수정 (serverList 모킹)
- [ ] Integration 테스트를 Playwright/Cypress로 마이그레이션
- [ ] 커버리지 80% 이상 달성
- [ ] 스냅샷 테스트 추가 (UI 컴포넌트)

---

## 🔗 관련 문서

- [docs/TESTING.md](../docs/TESTING.md) - 전체 프로젝트 테스트 가이드
- [docs/GUI_TESTING.md](../docs/GUI_TESTING.md) - GUI 테스트 상세
- [scripts/test-gui.ps1](../scripts/test-gui.ps1) - 빠른 테스트 스크립트
