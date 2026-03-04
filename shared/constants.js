/**
 * 공유 상수 — JS 전체에서 사용하는 Single Source of Truth
 *
 * saba-chan-gui/main.js, renderer (i18n.js, useSettingsStore.js),
 * discord_bot, updater GUI 등에서 이 파일을 import하여 사용합니다.
 *
 * Rust 측 SSOT: updater/src/constants.rs
 */

const path = require('path');

const APP_NAME = 'saba-chan';
const GITHUB_OWNER = 'WareAoba';
const GITHUB_REPO = 'saba-chan';
const GITHUB_MODULES_REPO = 'saba-chan-modules';
const GITHUB_EXTENSIONS_REPO = 'saba-chan-extensions';

const DEFAULT_IPC_PORT = 57474;
const DEFAULT_DAEMON_URL = `http://127.0.0.1:${DEFAULT_IPC_PORT}`;

const SUPPORTED_LANGUAGES = ['en', 'ko', 'ja', 'zh-CN', 'zh-TW', 'es', 'pt-BR', 'ru', 'de', 'fr'];

/**
 * 앱 데이터 디렉토리 — Rust constants::resolve_data_dir() 과 동일 로직
 *
 * - SABA_DATA_DIR 환경변수 우선
 * - Windows: %APPDATA%/saba-chan
 * - Unix:    $HOME/.config/saba-chan
 */
function getSabaDataDir() {
    if (process.env.SABA_DATA_DIR) return process.env.SABA_DATA_DIR;
    if (process.platform === 'win32') {
        return path.join(process.env.APPDATA || '', APP_NAME);
    }
    return path.join(process.env.HOME || '', '.config', APP_NAME);
}

module.exports = {
    APP_NAME,
    GITHUB_OWNER,
    GITHUB_REPO,
    GITHUB_MODULES_REPO,
    GITHUB_EXTENSIONS_REPO,
    DEFAULT_IPC_PORT,
    DEFAULT_DAEMON_URL,
    SUPPORTED_LANGUAGES,
    getSabaDataDir,
};
