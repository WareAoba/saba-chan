# Saba-chan 테스트 빠른 참조

## ⚡ 빠른 실행

```powershell
# 전체 테스트
.\scripts\run-all-tests.ps1

# JavaScript만 (빠름)
.\scripts\quick-test.ps1

# CI/CD 모드
.\scripts\ci-test.ps1
```

---

## 📊 테스트 통계

| 컴포넌트 | 테스트 수 | 실행 시간 |
|----------|-----------|----------|
| Rust Daemon | 37개 | ~20초 |
| Electron GUI | ~15개 | ~8초 |
| Discord Bot | ~20개 | ~7초 |
| **합계** | **~72개** | **~35초** |

---

## 🎯 개별 실행

```powershell
# Rust
cargo test --test daemon_integration

# GUI
cd electron_gui && npm test

# Bot
cd discord_bot && npm test
```

---

## ✅ 성공 기준

- [ ] Rust: 37/37 passed
- [ ] GUI: 모든 통합 테스트 통과
- [ ] Bot: 별명 해석 + E2E 통과
- [ ] 총 실행 시간 < 60초

문제 발생 시 [TEST_EXECUTION_GUIDE.md](TEST_EXECUTION_GUIDE.md) 참조
