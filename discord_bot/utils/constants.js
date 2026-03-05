/**
 * shared/constants.js 프록시
 *
 * SSOT: shared/constants.js (프로젝트 루트)
 *
 * 개발 환경에서는 ../../shared/constants.js 를 그대로 참조합니다.
 * 프로덕션(설치 폴더)에서는 discord_bot이 설치 루트 바로 아래에 위치하므로
 * 상대경로 ../../shared/constants 가 설치 루트 바깥을 가리킵니다.
 * 이 프록시가 양쪽을 모두 시도하여 해결합니다.
 *
 * 경로 탐색 순서:
 *   1) ../../shared/constants  (개발 환경: 프로젝트 루트)
 *   2) ../shared/constants     (프로덕션: 설치 루트/shared/)
 */

let constants;

try {
    constants = require('../../shared/constants');
} catch (_) {
    try {
        constants = require('../shared/constants');
    } catch (__) {
        // 어디서도 찾을 수 없으면 인라인 fallback (SSOT 복제 — 최후 수단)
        const path = require('path');
        const APP_NAME = 'saba-chan';
        const DEFAULT_IPC_PORT = 57474;
        const DEFAULT_DAEMON_URL = `http://127.0.0.1:${DEFAULT_IPC_PORT}`;

        function getSabaDataDir() {
            if (process.env.SABA_DATA_DIR) return process.env.SABA_DATA_DIR;
            if (process.platform === 'win32') {
                return path.join(process.env.APPDATA || '', APP_NAME);
            }
            return path.join(process.env.HOME || '', '.config', APP_NAME);
        }

        constants = {
            APP_NAME,
            GITHUB_OWNER: 'WareAoba',
            GITHUB_REPO: 'saba-chan',
            GITHUB_MODULES_REPO: 'saba-chan-modules',
            GITHUB_EXTENSIONS_REPO: 'saba-chan-extensions',
            DEFAULT_IPC_PORT,
            DEFAULT_DAEMON_URL,
            SUPPORTED_LANGUAGES: ['en', 'ko', 'ja', 'zh-CN', 'zh-TW', 'es', 'pt-BR', 'ru', 'de', 'fr'],
            getSabaDataDir,
        };

        console.warn('[constants] shared/constants.js not found — using inline fallback');
    }
}

module.exports = constants;
