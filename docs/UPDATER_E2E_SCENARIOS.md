# Updater E2E Manual Test Scenarios

업데이터/인스톨러 엔드투엔드 수동 검증 절차. 릴리즈 직전 및 업데이터/데몬 변경 시 반드시 수행.

## 준비

1. `scripts/build-windows.ps1`로 프로덕션 바이너리 빌드
2. 테스트 전용 `AppData`: `$env:APPDATA="$env:TEMP\saba-e2e"` 설정 후 각 시나리오 실행
3. 빌드된 exe를 격리된 폴더에 복사

## 시나리오 1 — GUI 자동 업데이트 (요구 경로)

**목적**: `requires_updater=true` 컴포넌트(GUI/코어/업데이터)의 자동 체이닝 검증.

1. GUI 실행 → 설정 → 업데이트 확인
2. 구버전 매니페스트를 강제로 주입: `settings.json`의 설치된 GUI 버전을 낮춤
3. "업데이트 확인" → 다운로드 → 적용 클릭
4. **기대 결과**:
   - 데몬이 `requires_updater=true` + `launch_token` 응답
   - main 프로세스가 토큰 캐시, 렌더러 응답에서 `launch_token` 누락
   - 업데이터 exe가 `--spawned-by gui --launch-token <uuid>`로 스폰
   - GUI 자동 종료 → 업데이터 창 출현 → 파일 교체 → GUI 재실행
   - `AppData/saba-chan/updates/launch-ticket.json` 파일 **삭제됨**

## 시나리오 2 — CLI 자동 체이닝

**목적**: `update apply`가 `requires_updater=true`일 때 자동으로 업데이터를 스폰.

1. CLI 진입 (`saba-chan-cli.exe`) → `:update check` → `:update download gui saba-core`
2. `:update apply` 실행
3. **기대 결과**:
   - 터미널에 `✓ Applied` 출력
   - `Updater required for: gui, saba-core. Launching updater...` 출력
   - `✓ Updater launched: ...` 출력
   - 업데이터 창이 뜨고 CLI는 계속 동작 (업데이터가 CLI 종료 후 재실행)

## 시나리오 3 — 업데이터 직접 실행 차단

**목적**: 플래그/토큰 없이 실행 시 거부되는지 검증.

1. 업데이터 exe를 탐색기에서 더블클릭 → **"missing --apply" 에러 후 종료**
2. `saba-chan-updater.exe --apply` → **"missing --spawned-by" 거부**
3. `saba-chan-updater.exe --apply --spawned-by gui` → **"missing --launch-token" 거부**
4. `saba-chan-updater.exe --apply --spawned-by gui --launch-token fake-123` → **"launch ticket not found" 또는 "token mismatch" 거부**

## 시나리오 4 — 티켓 일회성

**목적**: 티켓이 단 한 번만 사용되는지 검증.

1. GUI에서 업데이트 적용 시작 → 업데이터 스폰
2. 업데이터 실행 도중 같은 토큰으로 **두 번째** 업데이터 실행 시도
3. **기대 결과**: 두 번째 인스턴스는 `launch ticket not found`로 즉시 종료

## 시나리오 5 — 티켓 만료

**목적**: 발급 120초 후 티켓이 만료되는지 검증.

1. 데몬 API 직접 호출: `POST /api/updates/apply` → `launch_token` 획득
2. 121초 이상 대기
3. 업데이터를 토큰과 함께 스폰
4. **기대 결과**: `launch ticket expired` 출력 후 종료

## 시나리오 6 — 혼합 적용 (모듈 + GUI)

**목적**: 데몬 직접 적용 + 업데이터 위임이 올바르게 분기되는지.

1. 모듈(minecraft)과 GUI를 동시에 업데이트 대상으로 다운로드
2. `apply` 호출
3. **기대 결과**:
   - 전체 타겟이 `needs_updater`로 위임됨 (데몬은 직접 적용하지 않음)
   - 업데이터가 우선순위대로 적용: Updater → 모듈 → CoreDaemon → GUI

## 시나리오 7 — 네트워크 단절 시 복원

1. 다운로드 중 Wi-Fi 차단 → 재시도/실패 처리 확인
2. 적용 중 프로세스 강제 종료 → 재실행 시 pending 매니페스트 복원

## 결과 기록

각 시나리오 수행 후 아래 항목을 기록:

- [ ] 1. GUI 자동 업데이트
- [ ] 2. CLI 자동 체이닝
- [ ] 3. 직접 실행 차단 (4개 케이스)
- [ ] 4. 티켓 일회성
- [ ] 5. 티켓 만료
- [ ] 6. 혼합 적용
- [ ] 7. 네트워크 단절 복원

실패 시: `%TEMP%\saba-updater.log` + `AppData\saba-chan\logs\*.log` 첨부.
