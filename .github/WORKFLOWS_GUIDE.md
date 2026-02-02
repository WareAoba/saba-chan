# GitHub Actions Configuration

이 프로젝트는 4개의 GitHub Actions 워크플로우를 사용합니다.

## 📋 워크플로우

### 1. **test.yml** - 전체 테스트
**트리거**: 
- `main`, `develop` 브랜치에 push
- `main`, `develop` 브랜치로 Pull Request

**실행 내용**:
- ✅ Rust Daemon 테스트 (37개)
- ✅ Electron GUI 통합 테스트
- ✅ Discord Bot 통합 테스트

**실행 시간**: ~10-15분

---

### 2. **coverage.yml** - 코드 커버리지
**트리거**:
- `main` 브랜치에 push
- `main` 브랜치로 Pull Request

**실행 내용**:
- 📊 Rust 코드 커버리지 (Tarpaulin)
- 📊 JavaScript 코드 커버리지 (Jest)
- 📤 Codecov 업로드

**실행 시간**: ~15-20분

---

### 3. **quick-test.yml** - 빠른 테스트
**트리거**:
- `main` 외 브랜치에 push
- `electron_gui/`, `discord_bot/` 경로 변경 시

**실행 내용**:
- ⚡ JavaScript 테스트만 실행 (Rust 제외)

**실행 시간**: ~3-5분

---

### 4. **build.yml** - 릴리스 빌드
**트리거**:
- `main`, `develop` 브랜치에 push
- `v*` 태그 생성 시

**실행 내용**:
- 🏗️ Release 모드 빌드
- 🔍 Clippy 린트 검사
- 📦 바이너리 아티팩트 업로드
- 🚀 태그 시 GitHub Release 생성

**실행 시간**: ~8-12분

---

## 🎯 워크플로우 선택 가이드

### 개발 중 (feature 브랜치)
```
electron_gui/ 수정 → quick-test.yml (3분)
discord_bot/ 수정 → quick-test.yml (3분)
src/ 수정 → test.yml (15분)
```

### Pull Request 생성
```
main으로 PR → test.yml + coverage.yml 실행
develop으로 PR → test.yml 실행
```

### 릴리스
```
v1.0.0 태그 → build.yml → GitHub Release 자동 생성
```

---

## 📊 상태 뱃지

README.md에 추가:

```markdown
![Tests](https://github.com/your-username/saba-chan/workflows/Saba-chan%20Tests/badge.svg)
![Coverage](https://codecov.io/gh/your-username/saba-chan/branch/main/graph/badge.svg)
![Build](https://github.com/your-username/saba-chan/workflows/Build/badge.svg)
```

---

## ⚙️ 설정

### Codecov 설정 (선택사항)

1. [Codecov](https://codecov.io) 계정 생성
2. Repository 연결
3. 토큰 발급 (public repo는 불필요)

### Secrets 설정 (필요 시)

GitHub Repository → Settings → Secrets → Actions

```
CODECOV_TOKEN=your-token-here  (선택사항)
```

---

## 🚀 로컬에서 동일하게 실행

### 전체 테스트 (test.yml)
```powershell
cargo build --release
cargo test --test daemon_integration
cd electron_gui && npm test
cd discord_bot && npm test
```

### 커버리지 (coverage.yml)
```powershell
cargo tarpaulin --test daemon_integration --out Xml
cd electron_gui && npm test -- --coverage
cd discord_bot && npm test -- --coverage
```

### 빌드 (build.yml)
```powershell
cargo build --release
cargo clippy -- -D warnings
```

---

## 📈 최적화

### 캐싱
- ✅ Rust 의존성 캐싱 (~3분 절약)
- ✅ npm 의존성 캐싱 (~2분 절약)

### 병렬 실행
현재는 순차 실행이지만, 필요 시 `strategy.matrix`로 병렬화 가능:

```yaml
strategy:
  matrix:
    test: [rust, gui, bot]
```

### 타임아웃
- test.yml: 30분
- coverage.yml: 40분
- quick-test.yml: 15분
- build.yml: 30분

---

## 🔍 트러블슈팅

### 테스트 실패 시
1. Actions 탭에서 로그 확인
2. 실패한 스텝 클릭
3. 로컬에서 재현: `cargo test --test daemon_integration -- --nocapture`

### 캐시 무효화
```yaml
# cache key에 날짜 추가
key: ${{ runner.os }}-cargo-${{ hashFiles('**/Cargo.lock') }}-20260203
```

### Windows 관련 이슈
- 경로 구분자: `/` 사용 (PowerShell이 자동 변환)
- 긴 경로: `git config --system core.longpaths true` (이미 설정됨)

---

## 📋 체크리스트

커밋 전:
- [ ] `.\scripts\run-all-tests.ps1` 로컬 실행
- [ ] Clippy 경고 없음: `cargo clippy`
- [ ] 포맷팅 확인: `cargo fmt --check`

PR 생성 시:
- [ ] GitHub Actions 통과 확인
- [ ] Coverage 변화 확인 (Codecov 코멘트)
- [ ] 리뷰어 할당

릴리스 시:
- [ ] `v*.*.*` 태그 생성
- [ ] GitHub Release 자동 생성 확인
- [ ] 바이너리 다운로드 테스트

자동화된 CI/CD로 안정적인 개발을 보장합니다! 🚀
