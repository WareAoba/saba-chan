/**
 * Electron GUI E2E 통합 테스트
 * 실제 Daemon과 통신하여 전체 동작 흐름을 검증
 * 
 * NOTE: axios ESM import 문제로 인해 Jest에서 실행할 수 없음
 * 대신 scripts/test-gui.ps1를 사용하여 수동 테스트 가능
 * 
 * TODO: Jest-compatible E2E framework로 마이그레이션 예정
 * - Playwright
 * - Puppeteer
 * - Cypress
 */

describe('Integration Tests (Skipped)', () => {
    test.skip('E2E tests require manual execution', () => {
        // 이 테스트 파일은 Jest와 호환되지 않습니다
        // scripts/test-gui.ps1 실행 또는
        // 직접 데몬 시작 후 GUI 앱 실행으로 테스트하세요
    });
});
