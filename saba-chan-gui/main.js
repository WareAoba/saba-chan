const {
    app,
    BrowserWindow,
    Menu,
    ipcMain,
    Tray,
    nativeImage,
    nativeTheme,
    Notification,
    dialog,
    shell,
} = require('electron');
const path = require('path');
const axios = require('axios');
const { spawn, execSync } = require('child_process');
const fs = require('fs');
const http = require('http');
const { DEFAULT_IPC_PORT, DEFAULT_DAEMON_URL, SUPPORTED_LANGUAGES, getSabaDataDir } = require('../shared/constants');

const IPC_PORT_DEFAULT = DEFAULT_IPC_PORT;
let IPC_BASE = process.env.IPC_BASE || DEFAULT_DAEMON_URL; // Core Daemon endpoint — updated from settings after app ready

/** 데몬이 준 오류 본문(error_code, details 등)을 버리지 않고 렌더러에 넘긴다 */
function errorBody(err, extra = {}) {
    const data = err.response?.data;
    const body = data && typeof data === 'object' && !Array.isArray(data) ? data : {};
    return { ...body, ...extra, error: body.error || err.message };
}

// ── 고정 경로: shared/constants.js SSOT ──
// Electron userData를 saba-chan으로 통일 — ready 전에 호출해야 Chromium 내부 데이터도 이 경로 사용
app.setPath('userData', getSabaDataDir());

function getFixedModulesPath() {
    return process.env.SABA_MODULES_PATH || path.join(getSabaDataDir(), 'modules');
}
function getFixedExtensionsPath() {
    return process.env.SABA_EXTENSIONS_DIR || path.join(getSabaDataDir(), 'extensions');
}

function refreshIpcBase() {
    if (process.env.IPC_BASE) return; // 환경변수가 설정되면 그것을 우선
    try {
        const s = loadSettings();
        const port = s.ipcPort || IPC_PORT_DEFAULT;
        IPC_BASE = `http://127.0.0.1:${port}`;
    } catch (_) {
        /* app not ready yet */
    }
}

// 네트워크 호출 기본 타임아웃 (ms). 대부분의 API는 빠르게 응답하지만,
// 서버 JAR 다운로드 등 오래 걸리는 호출은 개별 timeout을 지정합니다.
axios.defaults.timeout = 5000;

// ── IPC 토큰 인증 ──────────────────────
// 데몬이 시작 시 생성하는 .ipc_token 파일을 읽어서 데몬 요청에만 X-Saba-Token 헤더로 포함
function getIpcTokenPath() {
    if (process.env.SABA_TOKEN_PATH) return process.env.SABA_TOKEN_PATH;
    return path.join(getSabaDataDir(), '.ipc_token');
}

// ── 토큰을 전용 변수로 관리 (axios.defaults.headers.common에 의존하지 않음) ──
let _cachedIpcToken = '';

function loadIpcToken() {
    try {
        const tokenPath = getIpcTokenPath();
        const token = fs.readFileSync(tokenPath, 'utf-8').trim();
        if (token) {
            const prev = _cachedIpcToken;
            _cachedIpcToken = token;
            if (prev !== token) {
                console.log(
                    `[Auth] IPC token loaded: ${token.substring(0, 8)}… from ${tokenPath}` +
                        (prev ? ` (was: ${prev.substring(0, 8)}…)` : ' (first load)'),
                );
            }
            return true;
        }
    } catch (err) {
        console.warn('[Auth] IPC token not found, auth may fail:', err.message);
    }
    return false;
}

function getIpcToken() {
    if (!_cachedIpcToken) loadIpcToken();
    return _cachedIpcToken;
}

/** 데몬으로 가는 요청인지 (토큰은 데몬에만 보낸다) */
function isDaemonRequest(config) {
    try {
        const url = new URL(config.url || '', config.baseURL || undefined);
        const ipcPort = parseInt((settings && settings.ipcPort) || IPC_PORT_DEFAULT, 10);
        return (url.hostname === '127.0.0.1' || url.hostname === 'localhost') && parseInt(url.port, 10) === ipcPort;
    } catch (_) {
        return false;
    }
}

// ── axios 인터셉터: 데몬 요청에만 토큰을 붙인다 (Node 전역 http는 건드리지 않는다) ──
axios.interceptors.request.use((config) => {
    // 데몬 요청에만 토큰을 붙이고, 리다이렉트는 따라가지 않는다 (토큰이 다른 곳으로 새지 않게)
    if (!isDaemonRequest(config)) return config;
    config.maxRedirects = 0;
    const token = getIpcToken();
    if (config.headers) {
        // 감사 로그의 출처 (A10)
        const put = (k, v) => (typeof config.headers.set === 'function' ? config.headers.set(k, v) : (config.headers[k] = v));
        if (token) put('X-Saba-Token', token);
        put('X-Saba-Client', 'gui');
    }
    return config;
});

// ── 401 응답 시 토큰 자동 재로드 + 재시도 인터셉터 ──
// 데몬 재시작으로 토큰이 갱신된 경우 자동 복구
// Promise 큐로 직렬화하여 동시 401에 대해 한 번만 갱신
// ★ 릴레이 프록시 요청(/api/relay/)의 401은 릴레이 서버 인증 실패이므로 제외
let _tokenRefreshPromise = null;
axios.interceptors.response.use(
    (response) => response,
    async (error) => {
        const originalRequest = error.config;
        if (error.response && error.response.status === 401 && !originalRequest._retried) {
            // 릴레이 프록시 요청은 데몬 IPC 토큰과 무관 → 재시도하지 않음
            const reqUrl = originalRequest.url || '';
            if (reqUrl.includes('/api/relay/')) {
                return Promise.reject(error);
            }

            originalRequest._retried = true;

            // 이미 갱신 중이면 같은 Promise를 대기
            if (!_tokenRefreshPromise) {
                _tokenRefreshPromise = (async () => {
                    try {
                        const tokenPath = getIpcTokenPath();
                        const newToken = fs.readFileSync(tokenPath, 'utf-8').trim();
                        if (newToken) {
                            _cachedIpcToken = newToken;
                            console.log(`[Auth] Token refreshed after 401: ${newToken.substring(0, 8)}…`);
                            return newToken;
                        }
                    } catch (_) {
                        /* 토큰 파일 읽기 실패 */
                    }
                    return null;
                })();

                // 300ms 후 Promise 리셋 (다음 배치의 401에 대해 다시 갱신 가능)
                _tokenRefreshPromise.finally(() => {
                    setTimeout(() => {
                        _tokenRefreshPromise = null;
                    }, 300);
                });
            }

            const refreshedToken = await _tokenRefreshPromise;
            if (refreshedToken) {
                if (typeof originalRequest.headers?.set === 'function') {
                    originalRequest.headers.set('X-Saba-Token', refreshedToken);
                } else {
                    originalRequest.headers['X-Saba-Token'] = refreshedToken;
                }
                return axios(originalRequest);
            }
        }
        return Promise.reject(error);
    },
);

let mainWindow;
let daemonProcess = null;
let settings = null;
let tray = null;
let translations = {}; // 번역 객체 캐시
let keepDaemonAlive = false; // 인터페이스만 종료 시 데몬 유지 플래그

// ========== 설치 루트 경로 ==========
// Portable exe: PORTABLE_EXECUTABLE_DIR (원본 exe 디렉토리)
// 일반 패키징: exe 디렉토리
// 개발: 프로젝트 루트
function getInstallRoot() {
    if (!app.isPackaged) {
        return path.join(__dirname, '..');
    }
    // Portable 모드: 원본 exe가 있는 디렉토리 (자체 압축 해제 임시 폴더가 아닌 실제 배포 위치)
    if (process.env.PORTABLE_EXECUTABLE_DIR) {
        return process.env.PORTABLE_EXECUTABLE_DIR;
    }
    // Linux AppImage: APPIMAGE 환경변수가 자동 설정됨
    if (process.env.APPIMAGE) {
        return path.dirname(process.env.APPIMAGE);
    }
    return path.dirname(app.getPath('exe'));
}

// ========== 로그 시스템 ==========
let logStream = null;
let logFilePath = null;
let isShuttingDown = false;

function initLogger() {
    const logsDir = path.join(app.getPath('userData'), 'logs');
    if (!fs.existsSync(logsDir)) {
        fs.mkdirSync(logsDir, { recursive: true });
    }

    const timestamp = new Date().toISOString().replace(/[:.]/g, '-').slice(0, -5);
    logFilePath = path.join(logsDir, `saba-chan-${timestamp}.log`);

    logStream = fs.createWriteStream(logFilePath, { flags: 'a' });

    console.log('='.repeat(60));
    console.log(`Log file: ${logFilePath}`);
    console.log('='.repeat(60));

    // console.log, console.error 오버라이드
    const originalLog = console.log;
    const originalError = console.error;

    console.log = (...args) => {
        const message = args
            .map((arg) => (typeof arg === 'object' ? JSON.stringify(arg, null, 2) : String(arg)))
            .join(' ');
        const timestamp = new Date().toISOString();
        const logMessage = `[${timestamp}] [LOG] ${message}\n`;

        if (logStream && !logStream.destroyed && !isShuttingDown) {
            logStream.write(logMessage);
        }
        originalLog.apply(console, args);
    };

    console.error = (...args) => {
        const message = args
            .map((arg) => (typeof arg === 'object' ? JSON.stringify(arg, null, 2) : String(arg)))
            .join(' ');
        const timestamp = new Date().toISOString();
        const logMessage = `[${timestamp}] [ERROR] ${message}\n`;

        if (logStream && !logStream.destroyed && !isShuttingDown) {
            logStream.write(logMessage);
        }
        originalError.apply(console, args);
    };

    // 예외 처리
    process.on('uncaughtException', (error) => {
        console.error('Uncaught Exception:', error);
    });

    process.on('unhandledRejection', (reason, promise) => {
        console.error('Unhandled Rejection at:', promise, 'reason:', reason);
    });
}

function closeLogger() {
    isShuttingDown = true;
    if (logStream && !logStream.destroyed) {
        logStream.end();
    }
}
// ========================================

function getLocalesPath() {
    return path.join(getInstallRoot(), 'locales');
}

// 번역 파일 로드 (메인 프로세스용)
function loadTranslations() {
    const lang = getLanguage();
    const localesPath = getLocalesPath();
    const commonPath = path.join(localesPath, lang, 'common.json');
    try {
        if (fs.existsSync(commonPath)) {
            return JSON.parse(fs.readFileSync(commonPath, 'utf8'));
        }
    } catch (error) {
        console.error('Failed to load translations:', error);
    }
    // Fallback to English
    const fallbackPath = path.join(localesPath, 'en', 'common.json');
    try {
        return JSON.parse(fs.readFileSync(fallbackPath, 'utf8'));
    } catch (error) {
        console.error('Failed to load fallback translations:', error);
    }
    return {};
}

// 번역 함수 (dot notation 지원)
function t(key, variables = {}) {
    const keys = key.split('.');
    let value = translations;
    for (const k of keys) {
        if (value && typeof value === 'object' && k in value) {
            value = value[k];
        } else {
            return key; // 없으면 키 그대로 반환
        }
    }

    if (typeof value === 'string') {
        // 템플릿 보간: {{error}} -> variables.error
        return value.replace(/\{\{(\w+)\}\}/g, (match, varName) => {
            return variables[varName] || match;
        });
    }

    return key;
}

// 상태 업데이트를 렌더러로 전달 (없으면 무시)
function sendStatus(step, message) {
    if (mainWindow && !mainWindow.isDestroyed()) {
        mainWindow.webContents.send('status:update', {
            step,
            message,
            ts: Date.now(),
        });
    }
}

// 짧은 대기 헬퍼
function wait(ms) {
    return new Promise((resolve) => setTimeout(resolve, ms));
}

// Bot Config — 데몬 API 경유 (파일 직접 접근 금지)

async function loadBotConfig() {
    try {
        const resp = await axios.get(`${IPC_BASE}/api/config/bot`);
        return resp.data;
    } catch (error) {
        console.warn('Failed to load bot config from daemon, using defaults:', error.message);
        return { prefix: '!saba', moduleAliases: {}, commandAliases: {} };
    }
}

async function saveBotConfig(config) {
    try {
        await axios.put(`${IPC_BASE}/api/config/bot`, config);
        console.log('Bot config saved via daemon API');
        return true;
    } catch (error) {
        console.error('Failed to save bot config via daemon:', error.message);
        return false;
    }
}

// ── 노드 토큰 관리 — 데몬 API 경유 ──

async function loadNodeToken() {
    try {
        const resp = await axios.get(`${IPC_BASE}/api/config/node-token`);
        return resp.data?.token || '';
    } catch (e) {
        console.warn('[NodeToken] Failed to load from daemon:', e.message);
        return '';
    }
}

async function saveNodeToken(token) {
    try {
        await axios.put(`${IPC_BASE}/api/config/node-token`, { token });
        console.log('[NodeToken] Saved via daemon API');
        return true;
    } catch (e) {
        console.error('[NodeToken] Failed to save via daemon:', e.message);
        return false;
    }
}

// 시스템 언어 가져오기
function getSystemLanguage() {
    try {
        const locale = app.getLocale(); // 예: 'en-US', 'ko-KR', 'ja-JP', 'zh-CN'
        // 정확한 로케일 매칭 (zh-CN, zh-TW, pt-BR 등)
        if (SUPPORTED_LANGUAGES.includes(locale)) {
            return locale;
        }

        // 언어 코드만으로 매칭 (en-US → en, ko-KR → ko 등)
        const baseLang = locale.split('-')[0];
        const matched = SUPPORTED_LANGUAGES.find((lang) => lang === baseLang || lang.startsWith(baseLang + '-'));
        if (matched) {
            return matched;
        }

        return 'en';
    } catch (error) {
        console.error('Failed to get system language:', error);
        return 'en';
    }
}

// 언어 설정 가져오기
function getLanguage() {
    const settings = loadSettings();
    return settings.language || getSystemLanguage();
}

// 언어 설정 저장
function setLanguage(language) {
    const settings = loadSettings();
    settings.language = language;
    return saveSettings(settings);
}

// Settings 관리
// 모든 설정은 <data_dir>/config.json 하나에 네임스페이스별로 들어간다(데몬 config_file).
// GUI 설정은 그중 `gui`. 데몬이 떠 있으면 데몬 API로만 쓰고, 읽기와 데몬 기동 전
// 부트스트랩 쓰기만 파일을 직접 다룬다.
function getSettingsPath() {
    const userDataPath = app.getPath('userData'); // %APPDATA%/saba-chan
    return path.join(userDataPath, 'config.json');
}

function readJsonFile(filePath) {
    if (!fs.existsSync(filePath)) return null;
    let data = fs.readFileSync(filePath, 'utf8');
    if (data.charCodeAt(0) === 0xfeff) data = data.slice(1); // UTF-8 BOM
    return JSON.parse(data);
}

/** config.json 전체 (깨졌으면 .bak, 없으면 빈 객체) */
function readConfigDocument() {
    const configPath = getSettingsPath();
    for (const candidate of [configPath, `${configPath}.bak`]) {
        try {
            const doc = readJsonFile(candidate);
            if (doc && typeof doc === 'object' && !Array.isArray(doc)) return doc;
        } catch (error) {
            console.error(`Failed to read ${candidate}:`, error.message);
        }
    }
    return {};
}

/** 데몬 기동 전 GUI 설정 저장 — 다른 네임스페이스는 보존, tmp → rename */
function writeGuiSection(gui) {
    const configPath = getSettingsPath();
    const dir = path.dirname(configPath);
    if (!fs.existsSync(dir)) fs.mkdirSync(dir, { recursive: true });
    const doc = readConfigDocument();
    doc.gui = gui;
    doc.schemaVersion = doc.schemaVersion || 1;
    const tmp = `${configPath}.tmp`;
    fs.writeFileSync(tmp, JSON.stringify(doc, null, 2), 'utf8');
    if (fs.existsSync(configPath)) fs.copyFileSync(configPath, `${configPath}.bak`);
    fs.renameSync(tmp, configPath);
}

function defaultSettings() {
    return {
        autoRefresh: true,
        refreshInterval: 2000,
        windowBounds: { width: 1200, height: 840 },
        language: getSystemLanguage(),
        ipcPort: IPC_PORT_DEFAULT,
        consoleBufferSize: 2000,
        autoGeneratePasswords: true,
        portConflictCheck: true,
    };
}

function loadSettings() {
    let gui = readConfigDocument().gui;
    if (!gui) {
        // 아직 데몬이 config.json으로 이관하기 전 (예전 settings.json)
        try {
            gui = readJsonFile(path.join(path.dirname(getSettingsPath()), 'settings.json'));
        } catch (error) {
            console.error('Failed to load legacy settings.json:', error.message);
        }
    }
    // 기본값 병합 — 파일에 없는 필드만 채움 (기존 필드 보존)
    return { ...defaultSettings(), ...(gui || {}) };
}

// 데몬이 응답하면 API로만 저장한다(단일 원천). 내가 띄운 데몬인지와 무관하다 —
// 이미 떠 있던 데몬에 붙은 경우에도 파일을 직접 쓰면 데몬 메모리와 어긋난다.
// 데몬이 아예 응답하지 않을 때(부트스트랩)만 파일에 직접 쓴다.
async function saveSettings(settings) {
    try {
        await axios.put(`${IPC_BASE}/api/config/gui`, settings, { timeout: 5000 });
        return true;
    } catch (err) {
        if (err.response) {
            console.error('[saveSettings] Daemon rejected settings:', err.message);
            return false;
        }
    }
    try {
        writeGuiSection(settings);
        console.log('[saveSettings] Daemon not reachable: saved locally');
        return true;
    } catch (e) {
        console.error('[saveSettings] Local save failed:', e.message);
        return false;
    }
}

// Core Daemon 시작
function startDaemon() {
    const isDev = !app.isPackaged;
    const daemonFileName = process.platform === 'win32' ? 'saba-core.exe' : 'saba-core';

    console.log('\n========== CORE DAEMON STARTUP ==========');
    console.log('[Daemon] isDev:', isDev);
    console.log('[Daemon] app.isPackaged:', app.isPackaged);

    // 루트 디렉토리 + 데몬 경로 결정
    let rootDir, daemonPath;

    if (isDev) {
        // 개발: target/release/saba-core.exe
        rootDir = path.join(__dirname, '..');
        daemonPath = path.join(rootDir, 'target', 'release', daemonFileName);
        console.log('[Daemon] [DEV] rootDir:', rootDir);
        console.log('[Daemon] [DEV] daemonPath:', daemonPath);
    } else {
        // 프로덕션: 설치 루트 디렉토리의 saba-core.exe
        rootDir = getInstallRoot();
        daemonPath = path.join(rootDir, daemonFileName);
        console.log('[Daemon] [PROD] exe:', app.getPath('exe'));
        console.log('[Daemon] [PROD] PORTABLE_EXECUTABLE_DIR:', process.env.PORTABLE_EXECUTABLE_DIR || '(not set)');
        console.log('[Daemon] [PROD] rootDir:', rootDir);
        console.log('[Daemon] [PROD] daemonPath:', daemonPath);
    }

    console.log('[Daemon] exists?:', fs.existsSync(daemonPath));

    // 루트 디렉토리 내용 확인
    try {
        const files = fs.readdirSync(rootDir);
        console.log('[Daemon] rootDir contents:', files.slice(0, 20).join(', '));
    } catch (e) {
        console.error('[Daemon] Cannot read rootDir:', e.message);
    }
    console.log('========================================\n');

    if (!fs.existsSync(daemonPath)) {
        console.error('[Daemon] NOT FOUND:', daemonPath);
        return;
    }

    const currentLanguage = getLanguage();

    const ipcPort = (settings && settings.ipcPort) || IPC_PORT_DEFAULT;
    const daemonEnv = {
        ...process.env,
        RUST_LOG: 'info',
        SABA_LANG: currentLanguage,
        SABA_IPC_PORT: String(ipcPort),
        SABA_INSTANCES_PATH: path.join(app.getPath('userData'), 'instances'),
        SABA_MODULES_PATH: getFixedModulesPath(),
    };

    console.log('[Daemon] Environment variables:');
    console.log('[Daemon] SABA_INSTANCES_PATH:', daemonEnv.SABA_INSTANCES_PATH);
    console.log('[Daemon] SABA_MODULES_PATH:', daemonEnv.SABA_MODULES_PATH);

    // detached: Windows에서 Electron은 분리하지 않은 자식을 kill-on-close 잡에 넣어 GUI와 함께 죽인다 (C14).
    // 분리해 두고 데몬은 정상 종료 경로(cleanQuit·will-quit)에서만 끈다. 인터페이스만 종료하면 그대로 둔다.
    // 로그는 GUI가 살아 있는 동안 파이프로 받고, 그 뒤에는 데몬 버퍼(/api/daemon/console)에 남는다.
    daemonProcess = spawn(daemonPath, ['--spawned'], {
        cwd: rootDir,
        env: daemonEnv,
        stdio: ['ignore', 'pipe', 'pipe'],
        detached: true,
        windowsHide: true,
    });

    console.log('[Daemon] spawned with PID:', daemonProcess.pid);

    // stdout/stderr 이벤트 핸들 (stdio가 'pipe'가 아니면 건너뜀)
    if (daemonProcess.stdout) {
        daemonProcess.stdout.on('data', (data) => {
            console.log('[Daemon]', data.toString().trim());
        });
    }

    if (daemonProcess.stderr) {
        daemonProcess.stderr.on('data', (data) => {
            // 데몬의 tracing 출력이 stderr로 옴 (라인 버퍼링 보장)
            const text = data.toString().trim();
            if (!text) return;
            // tracing WARN/ERROR 레벨은 console.error, 나머지는 console.log
            if (/\bWARN\b|\bERROR\b/.test(text)) {
                console.error('[Daemon]', text);
            } else {
                console.log('[Daemon]', text);
            }
        });
    }

    daemonProcess.on('error', (err) => {
        console.error('Failed to start Core Daemon:', err);
        daemonProcess = null;
    });

    daemonProcess.on('exit', (code, signal) => {
        console.log(`Core Daemon exited with code ${code}, signal ${signal}`);
        daemonProcess = null;

        // 트레이 메뉴 업데이트
        if (tray) {
            updateTrayMenu();
        }
    });

    daemonProcess.on('close', (code, signal) => {
        console.log(`Core Daemon closed with code ${code}, signal ${signal}`);
    });
}

// Core Daemon 종료: API로 정상 종료를 요청하고, 시간 안에 안 끝나면 이 GUI가 띄운 PID만 강제 종료한다.
// 이름 기반 kill(taskkill /IM, pkill -f)은 다른 사용자의 데몬까지 죽일 수 있어 쓰지 않는다.
async function stopDaemon({ timeoutMs = 15000 } = {}) {
    const proc = daemonProcess;
    try {
        await axios.post(`${IPC_BASE}/api/daemon/shutdown`, {}, { timeout: 2000 });
        console.log('Daemon shutdown requested via API');
    } catch (e) {
        console.warn('Daemon shutdown API failed:', e.message);
    }
    if (!proc) return;
    if (!(await waitForExit(proc, timeoutMs))) {
        console.warn(`Daemon did not exit in ${timeoutMs}ms, force killing PID ${proc.pid}`);
        forceKillDaemon(proc);
    }
    if (daemonProcess === proc) daemonProcess = null;
    console.log('Daemon stopped');
}

function waitForExit(proc, timeoutMs) {
    if (proc.exitCode !== null || proc.signalCode !== null) return Promise.resolve(true);
    return new Promise((resolve) => {
        const timer = setTimeout(() => resolve(false), timeoutMs);
        proc.once('exit', () => {
            clearTimeout(timer);
            resolve(true);
        });
    });
}

function forceKillDaemon(proc) {
    try {
        if (process.platform === 'win32') {
            // /T 없이: 서버는 saba-runner가 쥐고 있어 데몬만 끊고, 다음 기동 때 다시 붙는다
            execSync(`taskkill /PID ${proc.pid} /F`, { stdio: 'ignore', windowsHide: true });
        } else {
            proc.kill('SIGKILL');
        }
    } catch (e) {
        console.debug('Force kill error (process may already be dead):', e.message);
    }
}

// 안전한 종료 함수
/**
 * 인터페이스만 종료 — GUI 프로세스를 완전히 종료하되 데몬은 그대로 유지한다.
 * 데몬에서 클라이언트 해제(shutdown=false)만 수행하고, 데몬 프로세스는 건드리지 않는다.
 */
async function interfaceOnlyQuit() {
    console.log('Starting interface-only quit sequence...');
    keepDaemonAlive = true;

    try {
        // 데몬에서 클라이언트 해제 (shutdown=false → 데몬은 계속 실행)
        await unregisterFromDaemon({ shutdown: false });

        // 트레이 정리
        if (tray) {
            tray.destroy();
            tray = null;
        }

        // 메인 윈도우 정리 (close 이벤트 가로채기 해제 후 파괴)
        if (mainWindow) {
            mainWindow.removeAllListeners('close');
            mainWindow.destroy();
            mainWindow = null;
        }

        // 데몬 파이프를 놓아 GUI가 기다리지 않고 끝나게 한다 (데몬 쪽 쓰기 실패는 tracing이 무시한다)
        if (daemonProcess) {
            daemonProcess.stdout?.destroy();
            daemonProcess.stderr?.destroy();
            daemonProcess.unref();
        }
        console.log('Interface-only quit sequence completed (daemon kept alive)');
        closeLogger();
        app.quit();
    } catch (error) {
        console.error('Error during interface-only quit:', error);
        app.quit();
    }
}

async function cleanQuit() {
    console.log('Starting clean quit sequence...');

    try {
        // 0. 데몬에서 클라이언트 해제 + 데몬에 종료 요청 (shutdown=true)
        // → 데몬이 종료되면서 관리 중인 봇 프로세스도 함께 정리됨
        await unregisterFromDaemon({ shutdown: true });

        // 2. 데몬 종료 (정상 종료를 기다리고, 안 되면 자기가 띄운 PID만 강제 종료)
        await stopDaemon();

        // 3. 트레이 정리
        if (tray) {
            tray.destroy();
            tray = null;
        }

        // 4. 메인 윈도우 정리
        if (mainWindow) {
            mainWindow.destroy();
            mainWindow = null;
        }

        console.log('Clean quit sequence completed');

        // 로거 종료
        closeLogger();

        app.quit();
    } catch (error) {
        console.error('Error during clean quit:', error);
        app.quit();
    }
}

// 토큰 재로드 후 검증 재시도 (최대 3회, 500ms 간격). 3회 실패해도 일단 진행 (이후 자동 복구에 맡김)
async function reloadTokenUntilValid() {
    for (let retry = 0; retry < 3; retry++) {
        loadIpcToken();
        try {
            const verifyResp = await axios.get(`${IPC_BASE}/health`, { timeout: 1000 });
            // 예전 데몬은 token 칸 없이 200 (토큰이 맞을 때만)
            if (verifyResp.status === 200 && (verifyResp.data?.token ?? 'valid') === 'valid') {
                console.log('✓ Token refreshed and verified');
                return true;
            }
        } catch (verifyErr) {
            console.warn(`[Auth] Token verify attempt ${retry + 1} failed:`, verifyErr.message);
        }
        await wait(500);
    }
    console.warn('[Auth] Token verification failed after 3 retries, proceeding anyway');
    return false;
}

// 이미 떠 있는 데몬이 있으면 재실행하지 않고 재사용
async function ensureDaemon() {
    try {
        // IPC 토큰을 먼저 로드 (이미 데몬이 떠있을 수 있으므로)
        loadIpcToken();
        // /health 엔드포인트로 체크 (lock / 디스크 I/O 없이 즉시 응답)
        sendStatus('daemon', t('daemon.checking'));
        const response = await axios.get(`${IPC_BASE}/health`, { timeout: 1000 });
        // /health는 토큰 없이 답하고 보낸 토큰 상태(token)를 알려 준다 (A6). 예전 데몬은 401을 준다(아래 catch)
        if (response.status === 200 && response.data?.token && response.data.token !== 'valid') {
            console.log('Existing daemon detected (stale token). Reloading token...');
            await reloadTokenUntilValid();
            sendStatus('daemon', t('daemon.existing_running'));
            await syncInstallRoot();
            return;
        }
        if (response.status === 200) {
            console.log('Existing daemon detected on IPC port. Skipping launch.');
            sendStatus('daemon', t('daemon.existing_running'));
            await syncInstallRoot();
            return;
        }
    } catch (err) {
        // 401 = 데몬은 떠있지만 토큰이 맞지 않음 (이전 세션 토큰)
        if (err.response && err.response.status === 401) {
            console.log('Existing daemon detected (auth failed — stale token). Reloading token...');
            await reloadTokenUntilValid();
            sendStatus('daemon', t('daemon.existing_running'));
            await syncInstallRoot();
            return;
        }
        // ECONNREFUSED = 데몬이 안 떠있음, 그 외 에러 = 데몬은 떠있지만 문제 발생
        if (err.code === 'ECONNREFUSED' || err.code === 'ENOTFOUND' || err.message.includes('timeout')) {
            console.log('No daemon detected, attempting to launch new one...');
            sendStatus('daemon', t('daemon.starting'));
            try {
                startDaemon();
                // Daemon 시작 후 대기 및 재시도
                let attempts = 0;
                const maxAttempts = 8; // 최대 4초 대기
                while (attempts < maxAttempts) {
                    await wait(500);
                    // 데몬이 시작되면서 새 토큰을 생성하므로 매 시도마다 재로드
                    loadIpcToken();
                    try {
                        const checkResponse = await axios.get(`${IPC_BASE}/health`, { timeout: 800 });
                        if (checkResponse.status === 200) {
                            console.log('✓ Daemon is now running');
                            sendStatus('daemon', t('daemon.started'));
                            await syncInstallRoot();
                            return;
                        }
                    } catch (_checkErr) {
                        // 아직 준비 안 됨, 계속 재시도
                    }
                    attempts++;
                }
                // 최대 시도 후에도 응답 없음
                console.warn('Daemon did not respond after startup, but continuing...');
                sendStatus('daemon', t('daemon.preparing'));
            } catch (daemonErr) {
                console.error('Failed to start daemon:', daemonErr);
                sendStatus('daemon', t('daemon.failed_to_start'));
            }
            return;
        } else {
            // 다른 에러는 무시하고 계속
            console.warn('Unexpected error checking daemon:', err.message);
            sendStatus('daemon', t('daemon.check_warning', { error: err.message }));
        }
    }
}

// 데몬에 install_root 동기화 (portable 모드에서 임시 폴더가 아닌 실제 배포 경로 전달)
async function syncInstallRoot() {
    const installRoot = getInstallRoot();
    try {
        await axios.put(
            `${IPC_BASE}/api/updates/config`,
            {
                install_root: installRoot,
            },
            { timeout: 3000 },
        );
        console.log(`[InstallRoot] Synced to daemon: ${installRoot}`);
    } catch (e) {
        console.warn(`[InstallRoot] Failed to sync: ${e.message}`);
    }
}

// ── Client Heartbeat (데몬이 GUI 생존 여부를 추적) ────────────
let heartbeatClientId = null;
let heartbeatTimer = null;

async function registerWithDaemon() {
    try {
        // PID·설치 폴더: 인스톨러가 임시 폴더에 풀린 포터블 GUI도 이 설치의 것으로 찾아 끈다
        const res = await axios.post(
            `${IPC_BASE}/api/client/register`,
            { kind: 'gui', pid: process.pid, installRoot: getInstallRoot() },
            { timeout: 3000 },
        );
        heartbeatClientId = res.data.client_id;
        console.log(`[Heartbeat] Registered with daemon as client: ${heartbeatClientId}`);
        return true;
    } catch (e) {
        console.warn('[Heartbeat] Failed to register with daemon:', e.message);
        return false;
    }
}

function startHeartbeat() {
    if (heartbeatTimer) clearInterval(heartbeatTimer);

    heartbeatTimer = setInterval(async () => {
        if (!heartbeatClientId) return;
        try {
            await axios.post(`${IPC_BASE}/api/client/${heartbeatClientId}/heartbeat`, {}, { timeout: 3000 });
        } catch (e) {
            // 데몬이 재시작되었을 수 있으므로 재등록 시도
            if (e.response?.status === 404 || e.code === 'ECONNREFUSED') {
                console.warn('[Heartbeat] Lost registration, re-registering...');
                await registerWithDaemon();
            }
        }
    }, 30000); // 30초마다
}

async function unregisterFromDaemon(opts = {}) {
    if (!heartbeatClientId) return;
    try {
        const qs = opts.shutdown ? '?shutdown=true' : '';
        await axios.delete(`${IPC_BASE}/api/client/${heartbeatClientId}/unregister${qs}`, { timeout: 2000 });
        console.log('[Heartbeat] Unregistered from daemon' + (opts.shutdown ? ' (with shutdown request)' : ''));
    } catch (e) {
        console.warn('[Heartbeat] Failed to unregister:', e.message);
    }
    heartbeatClientId = null;
    if (heartbeatTimer) {
        clearInterval(heartbeatTimer);
        heartbeatTimer = null;
    }
}

async function runBackgroundInit() {
    sendStatus('init', '초기화 시작');
    await ensureDaemon();
    updateTrayMenu();
    // 실제 데이터는 렌더러가 로드할 때 가져온다. 여기서는 로딩 표시만 넘긴다.
    sendStatus('modules', '새 모듈 목록 준비 중...');
    sendStatus('instances', '인스턴스 목록 준비 중...');

    // 데몬에 클라이언트 등록 및 heartbeat 시작
    await registerWithDaemon();
    startHeartbeat();

    // 업데이트 주기적 체크 시작 (기본 3시간 간격)
    startUpdateChecker();

    sendStatus('ready', '백그라운드 초기화 완료');

    // Discord Bot 자동 시작은 React App.js에서 처리
}

// runDeferredTasks 제거됨 - Discord Bot 자동 시작은 React에서 처리

// ── 업데이트 알림 (데몬 HTTP API) ────────────────────────
// 확인 주기는 데몬이 정한다(U6, 설정 check_interval_hours). GUI는 데몬이 캐시한 결과
// (`GET /api/updates/status`)를 가끔 읽어 새 업데이트가 생겼을 때만 알린다.
const UPDATE_STATUS_POLL_MS = 10 * 60 * 1000; // 10분
let updateCheckTimer = null;
// 마지막으로 OS 알림을 보낸 업데이트 목록의 fingerprint (중복 알림 방지)
let lastNotifiedUpdateKey = null;

async function checkForUpdates() {
    try {
        // 데몬이 마지막으로 확인한 결과 (확인 자체는 데몬이 주기적으로 한다)
        const response = await axios.get(`${IPC_BASE}/api/updates/status`, { timeout: 5000 });
        const data = response.data;

        if (!data.ok || data.error) {
            if (data.error) console.warn('[UpdateChecker] Daemon check failed:', data.error);
            return;
        }

        if (data.updates_available > 0) {
            // Locales는 백그라운드 자동 적용 대상이므로 알림에서 제외 (서버에서도 필터링하지만 안전장치)
            const names = (data.components || [])
                .filter((c) => c.update_available && c.display_name !== 'Locales')
                .map((c) => c.display_name);
            const filteredCount = names.length;

            if (filteredCount === 0) {
                console.log('[UpdateChecker] Only Locales updates — skipping notification (auto-applied)');
                return;
            }

            console.log(`[UpdateChecker] ${filteredCount} update(s) available: ${names.join(', ')}`);

            // 중복 알림 방지: 이전과 동일한 업데이트 목록이면 OS 알림 건너뛰기
            const updateKey = [...names].sort().join('\0');
            const isNewUpdate = updateKey !== lastNotifiedUpdateKey;

            // OS 네이티브 알림 (새 업데이트일 때만)
            if (isNewUpdate && Notification.isSupported()) {
                // 아이콘 경로: build(프로덕션) → public(개발) 순서로 탐색
                const iconCandidates = [
                    path.join(__dirname, 'build', 'icon.png'),
                    path.join(__dirname, 'public', 'icon.png'),
                    path.join(__dirname, '..', 'resources', 'icon.png'),
                ];
                const notifIcon = iconCandidates.find((p) => fs.existsSync(p)) || undefined;
                const notif = new Notification({
                    title: 'saba-chan — 업데이트 알림',
                    body: `${filteredCount}개 업데이트: ${names.join(', ')}`,
                    icon: notifIcon,
                });
                notif.on('click', () => {
                    if (mainWindow) {
                        mainWindow.show();
                        mainWindow.focus();
                    }
                });
                notif.show();
                lastNotifiedUpdateKey = updateKey;
            } else if (!isNewUpdate) {
                console.log('[UpdateChecker] Skipping OS notification — same updates already notified');
            }

            // 렌더러 프로세스에 알림 전송 (업데이트 센터 모달에서 수동 처리)
            // Locales를 제외한 컴포넌트만 전달
            if (mainWindow && !mainWindow.isDestroyed()) {
                const filteredComponents = (data.components || []).filter(c => c.display_name !== 'Locales');
                mainWindow.webContents.send('updates:available', {
                    count: filteredCount,
                    names: names,
                    components: filteredComponents,
                });
            }

            // 적용은 사용자가 업데이트 센터에서 고른다 (auto_download이면 데몬이 미리 받아 둔다)
        } else {
            console.log('[UpdateChecker] No updates available');
        }
    } catch (e) {
        console.warn('[UpdateChecker] Check failed:', e.message);
    }
}

function startUpdateChecker() {
    stopUpdateChecker();
    checkForUpdates();
    updateCheckTimer = setInterval(checkForUpdates, UPDATE_STATUS_POLL_MS);
}

function stopUpdateChecker() {
    if (updateCheckTimer) {
        clearInterval(updateCheckTimer);
        updateCheckTimer = null;
    }
}

/** 모든 앱 창의 렌더러 설정: Node 없음, 격리, 샌드박스 */
function rendererWebPreferences() {
    return {
        preload: path.join(__dirname, 'preload.js'),
        nodeIntegration: false,
        contextIsolation: true,
        sandbox: true,
    };
}

/** 바깥 브라우저로 열어도 되는 주소: http(s)만 (file:, 사용자 정의 프로토콜은 실행이 될 수 있다) */
function isWebUrl(url) {
    try {
        const { protocol } = new URL(url);
        return protocol === 'http:' || protocol === 'https:';
    } catch (_) {
        return false;
    }
}

/** 창이 앱 화면 밖으로 이동하거나 새 창을 열지 못하게 한다. 웹 링크는 바깥 브라우저로 */
function lockNavigation(win) {
    const devOrigin = !app.isPackaged ? new URL(process.env.ELECTRON_START_URL || 'http://localhost:5173').origin : null;
    const isAppUrl = (url) => url.startsWith('file://') || (devOrigin !== null && url.startsWith(`${devOrigin}/`));
    const guard = (event, url) => {
        if (isAppUrl(url)) return;
        event.preventDefault();
        if (isWebUrl(url)) shell.openExternal(url).catch(() => {});
    };
    win.webContents.on('will-navigate', guard);
    win.webContents.on('will-redirect', guard);
    win.webContents.setWindowOpenHandler(({ url }) => {
        if (isWebUrl(url)) shell.openExternal(url).catch(() => {});
        return { action: 'deny' };
    });
}

function createWindow() {
    const settings = loadSettings();
    const { width, height, x, y } = settings.windowBounds || { width: 1200, height: 840 };

    mainWindow = new BrowserWindow({
        width,
        height,
        ...(x !== undefined && y !== undefined ? { x, y } : {}),
        minWidth: 780,
        minHeight: 840,
        show: false, // 준비될 때까지 보이지 않음
        backgroundColor: '#f0f0f2', // CSS 로드 전 흰 화면 방지 (라이트 모드 앱 배경 근사값)
        frame: false, // Windows 기본 프레임 제거
        icon: path.join(__dirname, 'build', 'icon.png'),
        webPreferences: rendererWebPreferences(),
    });

    lockNavigation(mainWindow);

    // 창 크기/위치 변경 시 설정 저장
    function saveWindowBounds() {
        if (!mainWindow || mainWindow.isDestroyed() || mainWindow.isMaximized() || mainWindow.isMinimized()) return;
        const bounds = mainWindow.getBounds();
        const settings = loadSettings();
        settings.windowBounds = { width: bounds.width, height: bounds.height, x: bounds.x, y: bounds.y };
        saveSettings(settings);
    }
    mainWindow.on('resized', saveWindowBounds);
    mainWindow.on('moved', saveWindowBounds);

    // 준비 완료 후 표시
    mainWindow.once('ready-to-show', () => {
        mainWindow.show();
    });

    // 윈도우 닫기 이벤트 가로채기 - React QuestionModal로 확인
    mainWindow.on('close', (e) => {
        e.preventDefault(); // 기본 닫기 동작 중단

        // React 앱에 다이얼로그 표시 요청
        mainWindow.webContents.send('app:closeRequest');
    });

    // 개발 모드: http://localhost:5173 (Vite), 프로덕션: build/index.html
    // --after-update로 재기동된 경우 Vite 서버가 없으므로 빌드 파일 사용
    const isDev = !app.isPackaged;
    const isAfterUpdate = process.argv.includes('--after-update');
    if (isDev && !isAfterUpdate) {
        const startURL = process.env.ELECTRON_START_URL || 'http://localhost:5173';
        mainWindow.loadURL(startURL).catch((e) => {
            console.error(`[Window] loadURL failed: ${e.message} — falling back to build file`);
            mainWindow.loadFile(path.join(__dirname, 'build', 'index.html')).catch((e2) => {
                console.error(`[Window] loadFile also failed: ${e2.message}`);
            });
        });
        // 개발 모드에서 DevTools 자동 열기
        mainWindow.webContents.openDevTools();
    } else {
        // 프로덕션 또는 업데이트 후 재기동: 빌드된 파일 로드
        mainWindow.loadFile(path.join(__dirname, 'build', 'index.html')).catch((e) => {
            console.error(`[Window] loadFile failed: ${e.message}`);
        });
    }

    // F12로 DevTools 열기 (프로덕션에서도 디버깅 가능)
    mainWindow.webContents.on('before-input-event', (_event, input) => {
        if (input.key === 'F12') {
            mainWindow.webContents.toggleDevTools();
        }
        // Ctrl+Shift+I (Windows/Linux) 또는 Cmd+Option+I (Mac)
        if ((input.control || input.meta) && input.shift && input.key === 'I') {
            mainWindow.webContents.toggleDevTools();
        }
    });

    // 메뉴바 제거
    mainWindow.removeMenu();
}

// React에서 종료 선택 응답 처리
ipcMain.on('app:closeResponse', (_event, choice) => {
    if (choice === 'exit-interface') {
        // 인터페이스만 종료 — GUI 프로세스 완전 종료, 데몬 유지
        interfaceOnlyQuit();
    } else if (choice === 'quit') {
        // 완전히 종료 - cleanQuit 사용
        mainWindow.removeAllListeners('close'); // close 이벤트 리스너 제거
        mainWindow.close();
        cleanQuit();
    }
    // choice === 'cancel'이면 아무것도 안 함
});

// 시스템 트레이 아이콘 생성
function getTrayIconPath() {
    const isDark = nativeTheme.shouldUseDarkColors;
    const filename = isDark ? 'favicon-dark.png' : 'favicon-light.png';
    const candidates = [
        path.join(__dirname, 'build', filename),
        path.join(__dirname, 'public', filename),
        path.join(__dirname, '..', 'resources', filename),
        path.join(__dirname, 'build', 'favicon.png'),
        path.join(__dirname, 'public', 'favicon.png'),
        path.join(__dirname, '..', 'resources', 'favicon.png'),
        path.join(__dirname, '..', 'resources', 'icon.png'),
    ];
    for (const p of candidates) {
        try {
            if (fs.existsSync(p)) return p;
        } catch (_) {}
    }
    return null;
}

function createTray() {
    let icon;
    const iconPath = getTrayIconPath();
    if (iconPath) {
        icon = nativeImage.createFromPath(iconPath).resize({ width: 16, height: 16 });
    }
    if (!icon) {
        // 폴백: 내장 base64 아이콘
        const iconBase64 =
            'iVBORw0KGgoAAAANSUhEUgAAABAAAAAQCAYAAAAf8/9hAAAABHNCSVQICAgIfAhkiAAAAAlwSFlzAAAAbwAAAG8B8aLcQwAAABl0RVh0U29mdHdhcmUAd3d3Lmlua3NjYXBlLm9yZ5vuPBoAAADfSURBVDiNpZMxDoJAEEV/kNCQWFhYGBIbO2s7j+ARPISdnYfwCHR2djYewMZKEgsLC0NCwiIFMbCwy7rJJJPM7sz/M7MLLEOSJMBERIZABziIyNlaq2+FkiQxwAH4AEPgDZRKqWdTb0VpXQdWQBd4A3MRecRxfGzuGGPKQB+YAgtgKCIDoK61fob+EeBpre/AB1gDU2AlIoM4jk91j8YYA/SAGbAE+iIyAspa62uLwD+11legDWyBhYhMgI7W+tIikOc5EzCZpum9kOD/gZzNs+xQJPC3oSAILl+nEbD5AYoJdEnfF3TzAAAAAElFTkSuQmCC';
        icon = nativeImage.createFromDataURL(`data:image/png;base64,${iconBase64}`);
    }
    tray = new Tray(icon);

    tray.setToolTip('사바쨩 - 게임 서버 관리');
    updateTrayMenu();

    // 시스템 테마 변경 시 트레이 아이콘 업데이트
    nativeTheme.on('updated', () => {
        if (!tray) return;
        const newIconPath = getTrayIconPath();
        if (newIconPath) {
            tray.setImage(nativeImage.createFromPath(newIconPath).resize({ width: 16, height: 16 }));
        }
    });

    // 트레이 아이콘 더블클릭 시 창 열기
    tray.on('double-click', () => {
        if (mainWindow) {
            mainWindow.show();
            mainWindow.focus();
        }
    });
}

// 트레이 메뉴 업데이트
function updateTrayMenu() {
    if (!tray) return;

    const contextMenu = Menu.buildFromTemplate([
        {
            label: '🖥️ 창 열기',
            click: () => {
                if (mainWindow) {
                    mainWindow.show();
                    mainWindow.focus();
                }
            },
        },
        { type: 'separator' },
        {
            label: daemonProcess ? '🟢 데몬 실행 중' : '⚪ 데몬 중지됨',
            enabled: false,
        },
        {
            label: '🛑 데몬 종료',
            enabled: !!daemonProcess,
            click: async () => {
                await stopDaemon();
                updateTrayMenu();
            },
        },
        {
            label: '▶️ 데몬 시작',
            enabled: !daemonProcess,
            click: () => {
                startDaemon();
                setTimeout(updateTrayMenu, 1000);
            },
        },
        { type: 'separator' },
        {
            label: '🔌 인터페이스만 종료',
            click: () => {
                interfaceOnlyQuit();
            },
        },
        {
            label: '❌ 완전히 종료',
            click: () => {
                cleanQuit();
            },
        },
    ]);

    tray.setContextMenu(contextMenu);
}

app.on('ready', () => {
    // Windows에서 OS 알림을 표시하려면 AppUserModelId가 반드시 필요
    app.setAppUserModelId('com.saba-chan.app');

    // 로거 초기화 (가장 먼저)
    initLogger();
    console.log('Saba-chan starting...');
    console.log('App version:', app.getVersion());
    console.log('Electron version:', process.versions.electron);
    console.log('Node version:', process.versions.node);
    console.log('Platform:', process.platform);
    console.log('isPackaged:', app.isPackaged);

    // 설정 미리 로드 (데몬 시작 전에)
    settings = loadSettings();
    refreshIpcBase(); // IPC 포트 설정 반영

    // 번역 초기화
    translations = loadTranslations();

    createTray();
    createWindow();
    updateTrayMenu();

    // UI가 준비된 뒤 백그라운드 초기화를 시작
    if (mainWindow && mainWindow.webContents) {
        mainWindow.webContents.once('did-finish-load', () => {
            sendStatus('ui', 'UI 로드 완료');
            runBackgroundInit();

            // --after-update 플래그 감지 → 업데이트 완료 알림
            if (process.argv.includes('--after-update')) {
                console.log('[Updater] Detected --after-update flag, notifying renderer');
                mainWindow.webContents.send('updates:completed', {
                    message: '업데이트가 완료되었습니다!',
                    timestamp: new Date().toISOString(),
                });
            }
        });
    }
});

app.on('window-all-closed', () => {
    // 더 이상 트레이에 숨겨 상주하지 않음.
    // interfaceOnlyQuit / cleanQuit에서 app.quit()을 명시적으로 호출하므로
    // 여기서는 추가 동작 불필요.
});

app.on('before-quit', () => {
    console.log('App is quitting, cleaning up...');

    // 업데이트 체커 정지
    stopUpdateChecker();

    // Heartbeat 정지 (동기적으로)
    if (heartbeatTimer) {
        clearInterval(heartbeatTimer);
        heartbeatTimer = null;
    }
    // 데몬에 동기적 unregister 시도 (타임아웃 짧게)
    if (heartbeatClientId) {
        try {
            const currentPort = (settings && settings.ipcPort) || IPC_PORT_DEFAULT;
            // 종료 중이라 axios를 기다릴 수 없어 http를 직접 쓴다 — 토큰은 여기서 붙인다
            const token = getIpcToken();
            const req = http.request({
                hostname: '127.0.0.1',
                port: currentPort,
                path: `/api/client/${heartbeatClientId}/unregister`,
                method: 'DELETE',
                timeout: 1000,
                headers: token ? { 'X-Saba-Token': token } : {},
            });
            req.end();
        } catch (_e) {
            /* 무시 */
        }
        heartbeatClientId = null;
    }

    // 비정상 종료 경로(cleanQuit을 거치지 않음): 기다릴 수 없으니 자기가 띄운 데몬만 정리한다
    if (!keepDaemonAlive && daemonProcess) {
        forceKillDaemon(daemonProcess);
        daemonProcess = null;
    }

    // 트레이 제거
    if (tray) {
        tray.destroy();
        tray = null;
    }

    // 메인 윈도우 제거
    if (mainWindow) {
        mainWindow.destroy();
        mainWindow = null;
    }

    console.log('Cleanup completed');
});

// 앱이 완전히 종료되기 전 최후의 보루
process.on('exit', () => {
    console.log('Process exiting');
    // 혹시 남아있을 데몬 프로세스 강제 종료 (인터페이스만 종료한 경우는 남긴다)
    if (!keepDaemonAlive && daemonProcess && !daemonProcess.killed) {
        try {
            console.log('Force killing daemon process at exit');
            daemonProcess.kill('SIGKILL');
        } catch (_e) {
            // 무시
        }
    }
});

// IPC handlers
ipcMain.handle('server:list', async () => {
    try {
        const response = await axios.get(`${IPC_BASE}/api/servers`);
        const data = response.data;

        // 포트 충돌로 강제 정지된 서버가 있으면 OS 네이티브 알림
        if (data.port_conflict_stops && data.port_conflict_stops.length > 0 && Notification.isSupported()) {
            const iconCandidates = [
                path.join(__dirname, 'build', 'icon.png'),
                path.join(__dirname, 'public', 'icon.png'),
                path.join(__dirname, '..', 'resources', 'icon.png'),
            ];
            const notifIcon = iconCandidates.find((p) => fs.existsSync(p)) || undefined;

            for (const evt of data.port_conflict_stops) {
                const notif = new Notification({
                    title: t('port_conflict.force_stop_title', { name: evt.stopped_name }),
                    body: t('port_conflict.force_stop_body', {
                        stopped: evt.stopped_name,
                        existing: evt.existing_name,
                        port: evt.port,
                    }),
                    icon: notifIcon,
                });
                notif.on('click', () => {
                    if (mainWindow) {
                        mainWindow.show();
                        mainWindow.focus();
                    }
                });
                notif.show();
            }
        }

        return data;
    } catch (error) {
        if (error.response) {
            const status = error.response.status;
            const data = error.response.data;
            if (status === 401) {
                // 인증 실패 — 토큰 재로드 후 1회 재시도
                if (loadIpcToken()) {
                    try {
                        const retry = await axios.get(`${IPC_BASE}/api/servers`);
                        return retry.data;
                    } catch (_) {
                        /* 재시도도 실패 */
                    }
                }
                return { error: 'Authentication failed. Daemon token may have changed.' };
            }
            return { error: t('server.list_failed', { status, error: data.error || error.message }) };
        }

        if (error.code === 'ECONNREFUSED') {
            return { error: t('network.connection_refused') };
        }

        return { error: `${t('error')}: ${error.message}` };
    }
});

// 시작 방식(managed 여부)과 포트 충돌 검사는 데몬이 인스턴스·전역 설정으로 정한다
ipcMain.handle('instance:start', async (_event, instanceId) => {
    try {
        const response = await axios.post(`${IPC_BASE}/api/instance/${instanceId}/start`, {}, { timeout: 30000 });
        return response.data;
    } catch (error) {
        if (error.response) {
            const status = error.response.status;
            const data = error.response.data || {};
            const message = data.error_message || data.error || data.message;
            switch (status) {
                case 404:
                    return { ...data, error: t('server.not_found', { name: instanceId }) };
                case 409:
                    return { ...data, error: message || t('server.already_running', { name: instanceId }) };
                default:
                    return { ...data, error: t('server.start_failed', { error: message || error.message }) };
            }
        }

        if (error.code === 'ECONNREFUSED') {
            return { error: t('network.connection_refused') };
        }

        return { error: `${t('error')}: ${error.message}` };
    }
});

ipcMain.handle('server:stop', async (_event, name, options = {}) => {
    try {
        const body = options || {};
        const response = await axios.post(`${IPC_BASE}/api/server/${name}/stop`, body, { timeout: 30000 });
        return response.data;
    } catch (error) {
        if (error.response) {
            const status = error.response.status;
            const data = error.response.data;

            switch (status) {
                case 400:
                    return { error: t('server.stop_failed', { error: data.error || t('info') }) };
                case 404:
                    return { error: t('server.not_found', { name }) };
                case 500:
                    return { error: `${t('error')}: ${data.error || data.message}` };
                default:
                    return { error: t('server.stop_failed', { error: data.error || error.message }) };
            }
        }

        if (error.code === 'ECONNREFUSED') {
            return { error: t('network.connection_refused') };
        }

        return { error: `${t('error')}: ${error.message}` };
    }
});

ipcMain.handle('server:status', async (_event, name) => {
    try {
        const response = await axios.get(`${IPC_BASE}/api/server/${name}/status`);
        return response.data;
    } catch (error) {
        if (error.response) {
            const status = error.response.status;
            const data = error.response.data;

            switch (status) {
                case 404:
                    return { error: t('server.not_found', { name }) };
                case 500:
                    return { error: `${t('error')}: ${data.error || data.message}` };
                default:
                    return { error: t('server.status_check_failed', { status, error: data.error || error.message }) };
            }
        }

        if (error.code === 'ECONNREFUSED') {
            return { error: t('network.connection_refused') };
        }

        return { error: `${t('error')}: ${error.message}` };
    }
});

// ── Module: Server Installation API ──────────────────────────

ipcMain.handle('module:listVersions', async (_event, moduleName, options = {}) => {
    try {
        const params = new URLSearchParams();
        if (options.include_snapshots) params.set('include_snapshots', 'true');
        if (options.page) params.set('page', options.page);
        if (options.per_page) params.set('per_page', options.per_page);
        const response = await axios.get(`${IPC_BASE}/api/module/${moduleName}/versions?${params}`, { timeout: 15000 });
        return response.data;
    } catch (error) {
        return errorBody(error);
    }
});

ipcMain.handle('module:installServer', async (_event, moduleName, installConfig) => {
    try {
        // JAR 다운로드는 수십 MB — 최대 5분 허용
        const response = await axios.post(`${IPC_BASE}/api/module/${moduleName}/install`, installConfig, {
            timeout: 300000,
        });
        return response.data;
    } catch (error) {
        return errorBody(error);
    }
});

ipcMain.handle('instance:getInstalledVersion', async (_event, instanceId) => {
    try {
        const response = await axios.get(`${IPC_BASE}/api/instance/${instanceId}/installed-version`, { timeout: 10000 });
        return response.data;
    } catch (error) {
        return errorBody(error);
    }
});

ipcMain.handle('instance:resetServer', async (_event, instanceId) => {
    try {
        const response = await axios.post(
            `${IPC_BASE}/api/instance/${instanceId}/server/reset`,
            {},
            { timeout: 30000 },
        );
        return response.data;
    } catch (error) {
        return errorBody(error);
    }
});

// ── Managed Process API (stdin/stdout capture) ───────────────

ipcMain.handle('managed:console', async (_event, instanceId, since = 0, count = 200) => {
    try {
        const response = await axios.get(`${IPC_BASE}/api/instance/${instanceId}/console`, {
            params: { since, count },
        });
        return response.data;
    } catch (error) {
        return errorBody(error);
    }
});

ipcMain.handle('managed:stdin', async (_event, instanceId, command) => {
    try {
        const response = await axios.post(`${IPC_BASE}/api/instance/${instanceId}/stdin`, { command });
        return response.data;
    } catch (error) {
        return errorBody(error);
    }
});

// ── Console Popout (PiP) Window ──────────────────────────────
const consolePopoutWindows = new Map(); // instanceId → BrowserWindow

// 열린 팝아웃 목록: 메인 창을 다시 불러오면 renderer가 이것으로 상태를 맞춘다
// 데몬 로그 버퍼 (GET /api/daemon/console): 하단 활동 줄·데몬 콘솔 창용
ipcMain.handle('daemon:console', (_event, opts) => {
    const { since, count } = opts || {};
    const params = new URLSearchParams();
    if (since !== undefined && since !== null) params.set('since', String(since));
    if (count) params.set('count', String(count));
    const qs = params.toString();
    return daemonCall('get', `/api/daemon/console${qs ? `?${qs}` : ''}`, undefined, 10000);
});

ipcMain.handle('console:popoutList', () =>
    [...consolePopoutWindows.entries()].filter(([, win]) => win && !win.isDestroyed()).map(([id]) => id),
);

ipcMain.handle('console:popout', async (_event, instanceId, serverName) => {
    // 이미 열려 있으면 포커스
    if (consolePopoutWindows.has(instanceId)) {
        const existing = consolePopoutWindows.get(instanceId);
        if (!existing.isDestroyed()) {
            existing.focus();
            return { ok: true, message: 'Focused existing window' };
        }
        consolePopoutWindows.delete(instanceId);
    }

    const popout = new BrowserWindow({
        width: 700,
        height: 450,
        minWidth: 400,
        minHeight: 250,
        frame: false,
        alwaysOnTop: true,
        title: `Console — ${serverName}`,
        icon: path.join(__dirname, 'build', 'icon.png'),
        webPreferences: rendererWebPreferences(),
    });

    popout.removeMenu();
    lockNavigation(popout);

    const isDev = !app.isPackaged;
    const isAfterUpdate = process.argv.includes('--after-update');
    const queryParams = `?console-popout=${encodeURIComponent(instanceId)}&name=${encodeURIComponent(serverName)}`;

    if (isDev && !isAfterUpdate) {
        const startURL = process.env.ELECTRON_START_URL || 'http://localhost:5173';
        popout.loadURL(`${startURL}${queryParams}`);
    } else {
        popout.loadFile(path.join(__dirname, 'build', 'index.html'), {
            search: queryParams.slice(1), // loadFile uses 'search' without '?'
        });
    }

    consolePopoutWindows.set(instanceId, popout);

    // 메인 윈도우에 팝아웃 열림/닫힘 알림 → 임베디드 콘솔 숨김 제어
    if (mainWindow && !mainWindow.isDestroyed()) {
        mainWindow.webContents.send('console:popoutOpened', instanceId);
    }

    popout.on('closed', () => {
        consolePopoutWindows.delete(instanceId);
        if (mainWindow && !mainWindow.isDestroyed()) {
            mainWindow.webContents.send('console:popoutClosed', instanceId);
        }
    });

    return { ok: true };
});

// 팝아웃 창 포커스/하이라이트
ipcMain.handle('console:focusPopout', async (_event, instanceId) => {
    if (consolePopoutWindows.has(instanceId)) {
        const win = consolePopoutWindows.get(instanceId);
        if (!win.isDestroyed()) {
            if (win.isMinimized()) win.restore();
            win.focus();
            // 깜빡임 효과로 주의 환기
            win.flashFrame(true);
            setTimeout(() => {
                if (!win.isDestroyed()) win.flashFrame(false);
            }, 2000);
            return { ok: true };
        }
    }
    return { ok: false };
});

// PiP 콘솔을 메인 윈도우로 되돌리기 (popin)
ipcMain.handle('console:popin', async (_event, instanceId) => {
    if (consolePopoutWindows.has(instanceId)) {
        const win = consolePopoutWindows.get(instanceId);
        if (!win.isDestroyed()) {
            // 팝아웃 창 URL에서 서버 이름 추출
            const url = new URL(win.webContents.getURL());
            const serverName = url.searchParams.get('name') || win.getTitle().replace('Console — ', '');
            // 메인 윈도우에 인앱 콘솔 열기를 요청 (창 닫기 전)
            if (mainWindow && !mainWindow.isDestroyed()) {
                mainWindow.webContents.send('console:popinRequest', instanceId, serverName);
            }
            win.close(); // close 이벤트에서 popoutClosed 알림이 전송됨
            return { ok: true };
        }
        consolePopoutWindows.delete(instanceId);
    }
    return { ok: false };
});

// PiP 창 alwaysOnTop 토글
ipcMain.handle('console:toggleAlwaysOnTop', async (_event, instanceId, pinned) => {
    if (consolePopoutWindows.has(instanceId)) {
        const win = consolePopoutWindows.get(instanceId);
        if (!win.isDestroyed()) {
            win.setAlwaysOnTop(pinned, 'floating');
            if (pinned) {
                // 재고정 시 윈도우를 최전면으로 끌어올리기
                if (win.isMinimized()) win.restore();
                win.moveTop();
                win.focus();
            }
            return { ok: true, pinned };
        }
    }
    return { ok: false };
});

ipcMain.handle('module:list', async () => {
    try {
        const response = await axios.get(`${IPC_BASE}/api/modules`);
        return response.data;
    } catch (error) {
        if (error.response) {
            const status = error.response.status;
            const data = error.response.data;
            return { error: t('server.list_failed', { status, error: data.error || error.message }) };
        }

        if (error.code === 'ECONNREFUSED') {
            return { error: t('network.connection_refused') };
        }

        return { error: `${t('error')}: ${error.message}` };
    }
});

ipcMain.handle('module:refresh', async () => {
    try {
        sendStatus('modules', t('modules.refreshing'));
        const response = await axios.post(`${IPC_BASE}/api/modules/refresh`);
        sendStatus('modules', t('modules.refresh_complete'));
        return response.data;
    } catch (error) {
        let errorMsg = t('modules.refreshing') + ': ';

        if (error.response) {
            const status = error.response.status;
            const data = error.response.data;
            errorMsg = t('server.list_failed', { status, error: data.error || error.message });
        } else if (error.code === 'ECONNREFUSED') {
            errorMsg = t('network.connection_refused');
        } else {
            errorMsg += error.message;
        }

        sendStatus('modules', errorMsg);
        return { error: errorMsg };
    }
});

// ── Module Manifest (사바 스토리지 — 모듈 탭) ──────────────────
ipcMain.handle('module:manifest', async () => {
    try {
        const response = await axios.get(`${IPC_BASE}/api/modules/manifest`, { timeout: 15000 });
        return response.data;
    } catch (error) {
        return errorBody(error, { ok: false });
    }
});

ipcMain.handle('module:installFromManifest', async (_event, moduleId) => {
    try {
        const response = await axios.post(
            `${IPC_BASE}/api/modules/manifest/${moduleId}/install`,
            {},
            { timeout: 120000 },
        );
        return response.data;
    } catch (error) {
        return errorBody(error, { ok: false });
    }
});

ipcMain.handle('module:remove', async (_event, moduleId) => {
    try {
        const response = await axios.delete(`${IPC_BASE}/api/modules/${moduleId}`, { timeout: 15000 });
        return response.data;
    } catch (error) {
        return errorBody(error, { ok: false });
    }
});

// 모듈의 locale 파일들을 모두 읽어서 반환 — 데몬 API 경유
ipcMain.handle('module:getLocales', async (_event, moduleName) => {
    try {
        const response = await axios.get(`${IPC_BASE}/api/module/${encodeURIComponent(moduleName)}/locales`);
        return response.data;
    } catch (error) {
        console.error(`Failed to load locales for module ${moduleName}:`, error.message);
        return {};
    }
});

// 모듈 코드가 쓰는 위험한 능력과 module.toml [permissions] 선언 비교 (데몬 응답 그대로)
ipcMain.handle('module:audit', async (_event, moduleName) => {
    try {
        const response = await axios.get(`${IPC_BASE}/api/module/${encodeURIComponent(moduleName)}/audit`);
        return response.data;
    } catch (error) {
        return errorBody(error);
    }
});

// 최근 종료 이력 (사용자 정지, 게임 안 정지, 크래시, 원인 모를 종료)
ipcMain.handle('instance:events', async (_event, id) => {
    try {
        const response = await axios.get(`${IPC_BASE}/api/instance/${encodeURIComponent(id)}/events`);
        return response.data;
    } catch (error) {
        return errorBody(error, { events: [] });
    }
});

ipcMain.handle('module:getMetadata', async (_event, moduleName) => {
    try {
        const response = await axios.get(`${IPC_BASE}/api/module/${moduleName}`);
        return response.data;
    } catch (error) {
        if (error.response) {
            const status = error.response.status;
            const data = error.response.data;

            switch (status) {
                case 404:
                    return { error: t('server.module_not_found', { module: moduleName }) };
                default:
                    return { error: t('server.status_check_failed', { status, error: data.error || error.message }) };
            }
        }

        if (error.code === 'ECONNREFUSED') {
            return { error: t('network.connection_refused') };
        }

        return { error: `${t('error')}: ${error.message}` };
    }
});

ipcMain.handle('instance:create', async (_event, data) => {
    try {
        // 백엔드가 도커 프로비저닝을 백그라운드로 처리하므로 짧은 타임아웃으로 충분
        const response = await axios.post(`${IPC_BASE}/api/instances`, data, { timeout: 30000 });
        return response.data;
    } catch (error) {
        if (error.response) {
            const status = error.response.status;
            const errData = error.response.data;

            const body = errData && typeof errData === 'object' ? errData : {};
            // error_code(working_dir_in_use 등)·instance를 렌더러가 쓸 수 있게 본문을 그대로 넘긴다
            switch (status) {
                case 400:
                    return { ...body, error: body.error || '잘못된 요청: 입력값을 확인해주세요' };
                case 409:
                    return { ...body, error: body.error || '이미 존재하는 인스턴스 이름입니다' };
                case 500:
                    return { ...body, error: `인스턴스 생성 오류: ${body.error || body.message || '내부 오류 발생'}` };
                default:
                    return { ...body, error: `생성 실패 (HTTP ${status}): ${body.error || error.message}` };
            }
        }

        if (error.code === 'ECONNREFUSED') {
            return { error: '데몬에 연결할 수 없습니다. 데몬이 실행중인지 확인해주세요' };
        }

        return { error: `인스턴스 생성 실패: ${error.message}` };
    }
});

// ── Provision progress polling ──
ipcMain.handle('instance:provisionProgress', async (_event, name) => {
    try {
        const response = await axios.get(`${IPC_BASE}/api/provision-progress/${encodeURIComponent(name)}`, {
            timeout: 3000,
        });
        return response.data;
    } catch (_error) {
        return { active: false };
    }
});

ipcMain.handle('instance:dismissProvision', async (_event, name) => {
    try {
        const response = await axios.delete(`${IPC_BASE}/api/provision-progress/${encodeURIComponent(name)}`, {
            timeout: 3000,
        });
        return response.data;
    } catch (_error) {
        return { success: false };
    }
});

ipcMain.handle('instance:checkUpdate', async (_event, id) => {
    try {
        const response = await axios.get(`${IPC_BASE}/api/instance/${id}/check-update`, {
            timeout: 30000, // SteamCMD app_info_print can be slow
        });
        return response.data;
    } catch (error) {
        console.error('instance:checkUpdate error:', error.message);
        return { update_available: false, error: error.message };
    }
});

ipcMain.handle('instance:applyUpdate', async (_event, id) => {
    try {
        const response = await axios.post(`${IPC_BASE}/api/instance/${id}/apply-update`, {}, {
            timeout: 10000,
        });
        return response.data;
    } catch (error) {
        console.error('instance:applyUpdate error:', error.message);
        return errorBody(error, { success: false });
    }
});

ipcMain.handle('instance:delete', async (_event, id) => {
    try {
        const response = await axios.delete(`${IPC_BASE}/api/instance/${id}`);
        return response.data;
    } catch (error) {
        if (error.response) {
            const status = error.response.status;
            const data = error.response.data;

            switch (status) {
                case 404:
                    return { error: `인스턴스를 찾을 수 없습니다` };
                case 409:
                    return { error: `실행중인 인스턴스는 삭제할 수 없습니다. 먼저 정지해주세요` };
                case 500:
                    return { error: `인스턴스 삭제 오류: ${data.error || data.message || '내부 오류 발생'}` };
                default:
                    return { error: `삭제 실패 (HTTP ${status}): ${data.error || error.message}` };
            }
        }

        if (error.code === 'ECONNREFUSED') {
            return { error: '데몬에 연결할 수 없습니다. 데몬이 실행중인지 확인해주세요' };
        }

        return { error: `인스턴스 삭제 실패: ${error.message}` };
    }
});

ipcMain.handle('instance:reorder', async (_event, orderedIds) => {
    try {
        const response = await axios.put(`${IPC_BASE}/api/instances/reorder`, { order: orderedIds });
        return response.data;
    } catch (error) {
        if (error.response) {
            return { error: error.response.data?.error || '순서 변경 실패' };
        }
        return { error: `순서 변경 실패: ${error.message}` };
    }
});

// 실행 인자 미리보기: 저장된 설정 위에 초안·추가 인자를 얹어 데몬이 실행 줄을 계산한다 (서버는 띄우지 않는다).
// extraArgs를 넘기지 않으면 저장된 추가 인자로 본다
ipcMain.handle('instance:launchPreview', (_event, id, settings, extraArgs) => {
    const body = { settings: settings || {} };
    if (extraArgs !== undefined && extraArgs !== null) body.extra_args = extraArgs;
    return daemonCall('post', `/api/instance/${encodeURIComponent(id)}/launch-preview`, body, 15000);
});

ipcMain.handle('instance:updateSettings', async (_event, id, settings) => {
    try {
        console.log(`[Main] Updating settings for instance ${id}:`, settings);
        const url = `${IPC_BASE}/api/instance/${id}`;
        console.log(`[Main] PATCH request to: ${url}`);
        const response = await axios.patch(url, settings);
        console.log(`[Main] Response:`, response.data);
        return response.data;
    } catch (error) {
        console.error(`[Main] Error updating settings:`, error.message);

        if (error.response) {
            const status = error.response.status;
            const data = error.response.data;

            switch (status) {
                case 400:
                    // 데몬의 검증 결과를 그대로: error_code가 invalid_format이면 renderer가 "잘못된 형식입니다"를 띄운다
                    return {
                        status,
                        error: `잘못된 설정값: ${data.message || data.error || '입력값을 확인해주세요'}`,
                        error_code: data.error_code,
                        details: data.details,
                        fields: data.fields,
                    };
                case 404:
                    return { error: `인스턴스를 찾을 수 없습니다` };
                case 500:
                    return { error: `설정 저장 오류: ${data.error || data.message || '내부 오류 발생'}` };
                default:
                    return { error: `설정 저장 실패 (HTTP ${status}): ${data.error || error.message}` };
            }
        }

        if (error.code === 'ECONNREFUSED') {
            return { error: '데몬에 연결할 수 없습니다. 데몬이 실행중인지 확인해주세요' };
        }

        return { error: `설정 저장 실패: ${error.message}` };
    }
});

// ── 모듈-독립적 입력값 검증 헬퍼 ──────────────────────────────
// module.toml의 inputs 스키마에 따라 args를 검증하고 정규화합니다.
// 모듈 이름을 전혀 참조하지 않으므로 어떤 게임 모듈에도 동일하게 동작합니다.
// 게임 명령 — 해석(명령 정의·별명·인자 검증)과 라우팅(stdin·REST·모듈)은 데몬이 한다.
// command는 "say hello"처럼 한 줄이어도 되고, 이름 + args(이름별 객체)여도 된다.
ipcMain.handle('instance:executeCommand', async (_event, id, command) => {
    const payload = {
        command: (command.command || '').trim(),
        args: command.args && Object.keys(command.args).length > 0 ? command.args : null,
    };
    try {
        const response = await axios.post(`${IPC_BASE}/api/instance/${id}/command`, payload, { timeout: 30000 });
        return response.data;
    } catch (error) {
        console.error('[Main] Error executing command:', error.message, error.response?.data || '');
        if (error.response) {
            const data = error.response.data || {};
            // 데몬이 모듈 [errors] 문구(error_message) 또는 원인(error)을 준다
            return {
                error: data.error_message || data.error || `오류 (HTTP ${error.response.status})`,
                error_code: data.error_code,
                field: data.field,
            };
        }
        const networkErrors = {
            ECONNREFUSED: '데몬에 연결할 수 없습니다. 데몬이 실행중인지 확인해주세요',
            ETIMEDOUT: '요청 시간 초과: 서버가 응답하지 않습니다',
        };
        return { error: networkErrors[error.code] || `명령어 실행 실패: ${error.message}` };
    }
});

// 데몬 응답 본문을 그대로 돌려준다. HTTP 오류면 데몬이 준 본문({ok, error, error_code})을 돌려준다.
async function daemonCall(method, path, body, timeout = 30000) {
    try {
        const response = await axios({ method, url: `${IPC_BASE}${path}`, data: body, timeout });
        return response.data;
    } catch (error) {
        if (error.response) {
            return error.response.data || { ok: false, error: `HTTP ${error.response.status}` };
        }
        return { ok: false, error: error.message, error_code: error.code };
    }
}

ipcMain.handle('instance:stop', (_event, id, opts = {}) =>
    daemonCall('post', `/api/instance/${id}/stop`, { force: !!opts.force }));

// 재시작은 데몬이 프로세스 종료를 최대 2분 기다린다
ipcMain.handle('instance:restart', (_event, id, opts = {}) =>
    daemonCall('post', `/api/instance/${id}/restart`, { force: !!opts.force }, 150000));

ipcMain.handle('instance:rename', (_event, id, name) =>
    daemonCall('post', `/api/instance/${id}/rename`, { name }));

ipcMain.handle('instance:commands', (_event, id) =>
    daemonCall('get', `/api/instance/${id}/commands`));

// 설정 화면용: 디스크의 실제 설정 (실패하면 success:false — 그때는 기본값으로 저장하지 말 것)
ipcMain.handle('instance:readLiveSettings', (_event, id) =>
    daemonCall('get', `/api/instance/${id}/settings/live`, undefined, 60000));

// 부팅 직후 Python 준비 상태: {phase, percent, complete, error, components: {python}}
ipcMain.handle('runtime:preflight', () => daemonCall('get', '/api/runtime/preflight', undefined, 5000));

// 시스템 모니터: 데몬이 표본을 뜬다. 처음 한 번은 두 번 떠서 조금 걸린다
// sizes=false면 데몬이 인스턴스 폴더 크기를 새로 재지 않는다 (접힌 요약)
ipcMain.handle('system:metrics', (_event, opts = {}) =>
    daemonCall('get', `/api/system/metrics${opts.sizes === false ? '?sizes=false' : ''}`, undefined, 5000));

ipcMain.handle('config:defaults', (_event, kind) =>
    daemonCall('get', `/api/config/${kind}/defaults`));

// keys를 생략하면 kind 전체를 기본값으로 되돌린다
ipcMain.handle('config:reset', (_event, kind, keys) =>
    daemonCall('post', `/api/config/${kind}/reset`, keys ? { keys } : {}));

// ── Extension IPC 핸들러 ──────────

// 익스텐션 목록 조회
ipcMain.handle('extension:list', async () => {
    try {
        const response = await axios.get(`${IPC_BASE}/api/extensions`);
        return response.data;
    } catch (error) {
        console.warn('[Extension] Failed to list extensions:', error.message);
        return { extensions: [] };
    }
});

// 익스텐션 활성화
// 익스텐션 코드가 쓰는 위험한 능력과 manifest permissions 비교 (데몬 응답 그대로)
ipcMain.handle('extension:audit', async (_event, extId) => {
    try {
        const response = await axios.get(`${IPC_BASE}/api/extensions/${encodeURIComponent(extId)}/audit`);
        return response.data;
    } catch (error) {
        return errorBody(error);
    }
});

ipcMain.handle('extension:enable', async (_event, extId) => {
    try {
        const response = await axios.post(`${IPC_BASE}/api/extensions/${extId}/enable`);
        return response.data;
    } catch (error) {
        const data = error.response?.data;
        console.warn(`[Extension] Failed to enable '${extId}':`, data || error.message);
        return {
            success: false,
            error: data?.error || error.message,
            error_code: data?.error_code || 'network',
            related: data?.related || [],
        };
    }
});

// 익스텐션 비활성화
ipcMain.handle('extension:disable', async (_event, extId) => {
    try {
        const response = await axios.post(`${IPC_BASE}/api/extensions/${extId}/disable`);
        return response.data;
    } catch (error) {
        const data = error.response?.data;
        console.warn(`[Extension] Failed to disable '${extId}':`, data || error.message);
        return {
            success: false,
            error: data?.error || error.message,
            error_code: data?.error_code || 'network',
            related: data?.related || [],
        };
    }
});

// 익스텐션 i18n 번역 로드
ipcMain.handle('extension:i18n', async (_event, extId, locale) => {
    try {
        const response = await axios.get(`${IPC_BASE}/api/extensions/${extId}/i18n/${locale}`);
        return response.data;
    } catch (error) {
        // 404는 해당 로케일이 없는 것이므로 경고 없이 null 반환
        if (error.response?.status === 404) return null;
        console.warn(`[Extension] Failed to load i18n for '${extId}' (${locale}):`, error.message);
        return null;
    }
});

// 익스텐션 GUI 번들 로드 (바이너리 → base64)
ipcMain.handle('extension:guiBundle', async (_event, extId) => {
    try {
        const response = await axios.get(`${IPC_BASE}/api/extensions/${extId}/gui`, {
            responseType: 'arraybuffer',
        });
        // JS 소스를 UTF-8 텍스트로 반환
        return Buffer.from(response.data).toString('utf-8');
    } catch (error) {
        if (error.response?.status === 404) return null;
        console.warn(`[Extension] Failed to load GUI bundle for '${extId}':`, error.message);
        return null;
    }
});

// 익스텐션 GUI 스타일 로드
ipcMain.handle('extension:guiStyles', async (_event, extId) => {
    try {
        const response = await axios.get(`${IPC_BASE}/api/extensions/${extId}/gui/styles`);
        return typeof response.data === 'string' ? response.data : null;
    } catch (error) {
        if (error.response?.status === 404) return null;
        console.warn(`[Extension] Failed to load GUI styles for '${extId}':`, error.message);
        return null;
    }
});

// 익스텐션 아이콘 로드 (PNG → data:image/png;base64)
ipcMain.handle('extension:icon', async (_event, extId) => {
    try {
        const response = await axios.get(`${IPC_BASE}/api/extensions/${extId}/icon`, {
            responseType: 'arraybuffer',
        });
        const base64 = Buffer.from(response.data).toString('base64');
        return `data:image/png;base64,${base64}`;
    } catch (error) {
        if (error.response?.status === 404) return null;
        console.warn(`[Extension] Failed to load icon for '${extId}':`, error.message);
        return null;
    }
});

// ── Extension Manifest & Version Management IPC 핸들러 ──────────

// 원격 매니페스트에서 가용 익스텐션 목록 페치
ipcMain.handle('extension:fetchManifest', async () => {
    try {
        const response = await axios.get(`${IPC_BASE}/api/extensions/manifest`, { timeout: 15000 });
        return response.data;
    } catch (error) {
        console.warn('[Extension] Failed to fetch manifest:', error.message);
        return { success: false, error: error.message, extensions: [], updates: [] };
    }
});

// 익스텐션 설치 (원격 매니페스트에서 다운로드)
ipcMain.handle('extension:install', async (_event, extId, opts = {}) => {
    try {
        const response = await axios.post(`${IPC_BASE}/api/extensions/${extId}/install`, opts || {}, {
            timeout: 60000,
        });
        return response.data;
    } catch (error) {
        const data = error.response?.data;
        console.warn(`[Extension] Failed to install '${extId}':`, data || error.message);
        return {
            success: false,
            error: data?.error || error.message,
            error_code: data?.error_code || 'network',
        };
    }
});

ipcMain.handle('extension:remove', async (_event, extId) => {
    try {
        const response = await axios.delete(`${IPC_BASE}/api/extensions/${extId}`, { timeout: 15000 });
        return response.data;
    } catch (error) {
        const data = error.response?.data;
        console.warn(`[Extension] Failed to remove '${extId}':`, data || error.message);
        return {
            success: false,
            error: data?.error || error.message,
            error_code: data?.error_code || 'network',
        };
    }
});

// 설치된 익스텐션 업데이트 체크
ipcMain.handle('extension:checkUpdates', async () => {
    try {
        const response = await axios.get(`${IPC_BASE}/api/extensions/updates`, { timeout: 15000 });
        return response.data;
    } catch (error) {
        console.warn('[Extension] Failed to check updates:', error.message);
        return { success: false, error: error.message, updates: [], count: 0 };
    }
});

// 익스텐션 디렉토리 재스캔
ipcMain.handle('extension:rescan', async () => {
    try {
        const response = await axios.post(`${IPC_BASE}/api/extensions/rescan`);
        return response.data;
    } catch (error) {
        console.warn('[Extension] Failed to rescan extensions:', error.message);
        return { success: false, error: error.message, newly_found: [] };
    }
});

// 익스텐션 초기화(daemon.startup) 진행 상태 조회
ipcMain.handle('extension:initStatus', async () => {
    try {
        const response = await axios.get(`${IPC_BASE}/api/extensions/init-status`, { timeout: 5000 });
        return response.data;
    } catch (_error) {
        // 데몬 미연결 → 초기화 상태 알 수 없음. initializing: false로 반환하여
        // 스피너가 데몬 미접속 상태에서 무한으로 도는 것을 방지.
        // 데몬 미접속 자체는 로딩 스크린이 처리.
        return { initializing: false, in_progress: {}, completed: [], daemon_unreachable: true };
    }
});

// ── Updater IPC 핸들러 (데몬 HTTP API 방식) ──────────

// 업데이트 상태 확인 — 데몬 API `/api/updates/check`
ipcMain.handle('updater:check', async () => {
    try {
        const response = await axios.post(`${IPC_BASE}/api/updates/check`, {}, { timeout: 30000 });
        const data = response.data;

        // 업데이트 발견 시 렌더러에 알림 이벤트 전송 → UpdateBanner + 알림 모달
        // Locales는 백그라운드 자동 적용 대상이므로 알림에서 제외
        const visibleNames = (data.update_names || []).filter(n => n !== 'Locales');
        const visibleComponents = (data.components || []).filter(c => c.display_name !== 'Locales');
        const visibleCount = visibleNames.length;

        if (data.ok && visibleCount > 0 && mainWindow && !mainWindow.isDestroyed()) {
            mainWindow.webContents.send('updates:available', {
                count: visibleCount,
                updates_available: visibleCount,
                names: visibleNames,
                update_names: visibleNames,
                components: visibleComponents,
            });
        }

        return data;
    } catch (err) {
        return { ok: false, error: err.message };
    }
});

// ── Relay server proxy (cloud mode) — daemon API 경유 ──

ipcMain.handle('relay:checkHostStatus', async (_event, hostId, relayUrl) => {
    try {
        const params = relayUrl ? `?relayUrl=${encodeURIComponent(relayUrl)}` : '';
        const resp = await axios.get(
            `${IPC_BASE}/api/relay/host/${encodeURIComponent(hostId)}/status${params}`,
            { timeout: 8000 },
        );
        return resp.data;
    } catch (err) {
        return { ok: false, error: err.message };
    }
});

ipcMain.handle('relay:listHostNodes', async (_event, hostId, relayUrl) => {
    try {
        const params = relayUrl ? `?relayUrl=${encodeURIComponent(relayUrl)}` : '';
        const resp = await axios.get(
            `${IPC_BASE}/api/relay/host/${encodeURIComponent(hostId)}/nodes${params}`,
            { timeout: 8000 },
        );
        return resp.data;
    } catch (err) {
        return [];
    }
});

ipcMain.handle('relay:listNodeMembers', async (_event, guildId, relayUrl) => {
    try {
        const params = relayUrl ? `?relayUrl=${encodeURIComponent(relayUrl)}` : '';
        const resp = await axios.get(
            `${IPC_BASE}/api/relay/node/${encodeURIComponent(guildId)}/members${params}`,
            { timeout: 8000 },
        );
        return resp.data;
    } catch (err) {
        return [];
    }
});

// 챗봇 채널(여럿)·음악 채널(하나) 배정. 실패하면 데몬 본문({error, channelId?})을 그대로 돌려준다
// 감사 로그 (A10): {limit?, instance?} → {entries: [...]} 최신이 먼저
ipcMain.handle('audit:list', (_event, opts = {}) => {
    const params = new URLSearchParams();
    if (opts.limit) params.set('limit', String(opts.limit));
    if (opts.instance) params.set('instance', String(opts.instance));
    const qs = params.toString();
    return daemonCall('get', `/api/audit${qs ? `?${qs}` : ''}`, undefined, 10000);
});

ipcMain.handle('relay:nodeChannels', (_event, guildId) =>
    daemonCall('get', `/api/relay/node/${encodeURIComponent(guildId)}/channels`, undefined, 15000));

ipcMain.handle('relay:nodeChannelsSave', (_event, guildId, patch) =>
    daemonCall('put', `/api/relay/node/${encodeURIComponent(guildId)}/channels`, patch, 15000));

ipcMain.handle('relay:botInvite', async (_event, relayUrl) => {
    try {
        const params = relayUrl ? `?relayUrl=${encodeURIComponent(relayUrl)}` : '';
        const resp = await axios.get(`${IPC_BASE}/api/relay/bot-invite${params}`, { timeout: 8000 });
        return resp.data;
    } catch (err) {
        return errorBody(err);
    }
});

// 데몬 JSON 호출. 4xx·5xx도 데몬이 준 본문을 그대로 돌려준다 (error_code, details)
async function daemonJson(method, path, data, timeout = 15000) {
    try {
        const resp = await axios({ method, url: `${IPC_BASE}${path}`, data, timeout });
        return resp.data;
    } catch (err) {
        return err.response?.data || { success: false, error: err.message };
    }
}
// 백업 (A2)·예약 작업 (A3)
// 큰 월드는 묶고 푸는 데 몇 분 걸린다
const BACKUP_TIMEOUT = 10 * 60 * 1000;
const instancePath = (id, rest = '') => `/api/instance/${encodeURIComponent(id)}${rest}`;
ipcMain.handle('instance:acceptEula', (_e, id) => daemonJson('post', instancePath(id, '/accept-eula')));
ipcMain.handle('instance:backups', (_e, id) => daemonJson('get', instancePath(id, '/backups')));
ipcMain.handle('instance:backupCreate', (_e, id) => daemonJson('post', instancePath(id, '/backups'), undefined, BACKUP_TIMEOUT));
ipcMain.handle('instance:backupDelete', (_e, id, name) =>
    daemonJson('delete', instancePath(id, `/backups/${encodeURIComponent(name)}`)),
);
ipcMain.handle('instance:backupRestore', (_e, id, name) =>
    daemonJson('post', instancePath(id, `/backups/${encodeURIComponent(name)}/restore`), undefined, BACKUP_TIMEOUT),
);
ipcMain.handle('instance:scheduleGet', (_e, id) => daemonJson('get', instancePath(id, '/schedule')));
ipcMain.handle('instance:scheduleSave', (_e, id, body) => daemonJson('put', instancePath(id, '/schedule'), body));
// AI 챗봇 설정: 키는 데몬에만 있고 응답에는 hasApiKey만 온다
ipcMain.handle('chatbot:get', () => daemonJson('get', '/api/chatbot'));
ipcMain.handle('chatbot:save', (_event, patch) => daemonJson('put', '/api/chatbot', patch));
ipcMain.handle('chatbot:clear', () => daemonJson('delete', '/api/chatbot'));
ipcMain.handle('chatbot:clearMemory', (_event, guildId) =>
    daemonJson('delete', `/api/chatbot/memory${guildId ? `?guildId=${encodeURIComponent(guildId)}` : ''}`),
);

ipcMain.handle('relay:initiatePairing', async (_event, payload) => {
    try {
        const resp = await axios.post(`${IPC_BASE}/api/relay/pair/initiate`, payload, {
            timeout: 8000,
        });
        return resp.data;
    } catch (err) {
        return errorBody(err);
    }
});

ipcMain.handle('relay:pollPairingStatus', async (_event, code, secret, relayUrl) => {
    try {
        const params = new URLSearchParams();
        if (secret) params.set('secret', secret);
        if (relayUrl) params.set('relayUrl', relayUrl);
        const qs = params.toString() ? `?${params.toString()}` : '';
        const resp = await axios.get(
            `${IPC_BASE}/api/relay/pair/${encodeURIComponent(code)}/status${qs}`,
            { timeout: 8000 },
        );
        return resp.data;
    } catch (err) {
        return errorBody(err);
    }
});

// 업데이트 상태 조회 (캐시) — 데몬 API `/api/updates/status`
ipcMain.handle('updater:status', async () => {
    try {
        const response = await axios.get(`${IPC_BASE}/api/updates/status`, { timeout: 5000 });
        return response.data;
    } catch (err) {
        return { ok: false, error: err.message };
    }
});

// 선택 컴포넌트 다운로드 — 데몬 API `/api/updates/download`
// body: { components: ["module-minecraft", "saba-core"] } (비어있으면 전체)
ipcMain.handle('updater:download', async (_event, components) => {
    try {
        const body = { components: Array.isArray(components) ? components : [] };
        const response = await axios.post(`${IPC_BASE}/api/updates/download`, body, { timeout: 600000 });
        return response.data;
    } catch (err) {
        return { ok: false, error: err.message };
    }
});

// 다운로드 진행률 폴링 — 데몬 API `/api/updates/download/progress`
ipcMain.handle('updater:downloadProgress', async () => {
    try {
        const response = await axios.get(`${IPC_BASE}/api/updates/download/progress`, { timeout: 5000 });
        return response.data;
    } catch (err) {
        return { ok: false, error: err.message };
    }
});

// 업데이트 적용 — 데몬 API `/api/updates/apply`
// 모듈은 데몬이 직접 적용, 데몬/GUI/CLI는 needs_updater에 포함
ipcMain.handle('updater:apply', async (_event, components) => {
    try {
        const body = { components: Array.isArray(components) ? components : [] };
        const response = await axios.post(`${IPC_BASE}/api/updates/apply`, body, { timeout: 120000 });
        const data = response.data;

        // 적용 완료 내역이 있으면 렌더러에 알림
        if (data.applied && data.applied.length > 0 && mainWindow && !mainWindow.isDestroyed()) {
            mainWindow.webContents.send('updates:completed', {
                count: data.applied.length,
                names: data.applied,
                requiresUpdater: !!data.requires_updater,
                needsUpdater: data.needs_updater || [],
            });
        }
        return data;
    } catch (err) {
        return { ok: false, error: err.message };
    }
});

// 업데이트 적용 시작 — 업데이터 실행은 데몬이 한다 (U7)
// 데몬이 업데이터를 띄우고 스스로 종료한다. GUI는 다시 띄울 자기 실행 경로만 알려주고 종료한다.
ipcMain.handle('updater:launchApply', async (_event, _targets) => {
    try {
        let relaunch;
        if (!app.isPackaged) {
            relaunch = [process.execPath, path.resolve(__dirname)]; // 개발 모드: electron exe + 앱 경로
        } else if (process.env.PORTABLE_EXECUTABLE_FILE) {
            relaunch = [process.env.PORTABLE_EXECUTABLE_FILE];
        } else {
            relaunch = [app.getPath('exe')];
        }
        const response = await axios.post(`${IPC_BASE}/api/updates/launch-updater`, { relaunch }, { timeout: 10000 });
        if (!response.data?.ok) {
            return { ok: false, error: response.data?.error || 'Failed to launch updater' };
        }
        console.log('[Updater] Daemon launched the updater; quitting GUI');
        setTimeout(() => cleanQuit(), 500);
        return { ok: true };
    } catch (err) {
        return errorBody(err, { ok: false });
    }
});

// 마지막 업데이트 적용 결과 — 데몬 API `/api/updates/last-result` (확인하면 clear)
ipcMain.handle('updater:lastResult', async (_event, { clear = false } = {}) => {
    try {
        const url = `${IPC_BASE}/api/updates/last-result`;
        // lang: 데몬이 이 언어의 패치노트를 `notes`에 골라 준다
        const response = clear
            ? await axios.delete(url, { timeout: 5000 })
            : await axios.get(url, { params: { lang: getLanguage() }, timeout: 5000 });
        return response.data;
    } catch (err) {
        return { ok: false, error: err.message };
    }
});

// 업데이트 설정 조회 — 데몬 API
ipcMain.handle('updater:getConfig', async () => {
    try {
        const response = await axios.get(`${IPC_BASE}/api/updates/config`, { timeout: 5000 });
        return response.data;
    } catch (err) {
        return { ok: false, error: err.message };
    }
});

// 업데이트 설정 변경 — 데몬 API (확인 주기·켜기/끄기는 데몬 스케줄러가 다음 회차에 반영)
ipcMain.handle('updater:setConfig', async (_event, config) => {
    try {
        const response = await axios.put(`${IPC_BASE}/api/updates/config`, config, { timeout: 5000 });
        return response.data;
    } catch (err) {
        return { ok: false, error: err.message };
    }
});

// Daemon 상태 확인 IPC 핸들러
ipcMain.handle('daemon:status', async () => {
    try {
        const _response = await axios.get(`${IPC_BASE}/health`, { timeout: 1000 });
        return { running: true, message: 'Daemon is running' };
    } catch (err) {
        return { running: false, message: `Daemon not responding: ${err.message}` };
    }
});

// Daemon 재시작 IPC 핸들러
ipcMain.handle('daemon:restart', async () => {
    try {
        await stopDaemon();
        // 이 GUI가 띄우지 않은 데몬이면 프로세스를 기다릴 수 없으니, 응답이 멎을 때까지 기다린다
        for (let i = 0; i < 30; i++) {
            try {
                await axios.get(`${IPC_BASE}/health`, { timeout: 500 });
            } catch (_) {
                break;
            }
            await wait(500);
        }
        console.log('Starting daemon...');
        settings = loadSettings();
        refreshIpcBase(); // 포트 변경 시 반영
        startDaemon();
        // 데몬이 시작될 때까지 대기하면서 새 토큰 로드 재시도
        let ready = false;
        for (let i = 0; i < 8; i++) {
            await wait(500);
            // 새 데몬이 새 토큰을 생성하므로 매 시도마다 재로드
            loadIpcToken();
            try {
                const check = await axios.get(`${IPC_BASE}/health`, { timeout: 800 });
                if (check.status === 200) {
                    ready = true;
                    break;
                }
            } catch (_) {
                /* 아직 기동 중 */
            }
        }
        if (!ready) {
            // 마지막 한 번 더 토큰 로드 시도
            loadIpcToken();
        }
        return { success: true, message: 'Daemon restarted successfully' };
    } catch (err) {
        console.error('Failed to restart daemon:', err);
        return { success: false, error: err.message };
    }
});

// Settings IPC handlers — 데몬 가동 중이면 API 경유, 부트스트랩 시 로컬 폴백
ipcMain.handle('settings:load', async () => {
    try {
        const resp = await axios.get(`${IPC_BASE}/api/config/gui`, { timeout: 5000 });
        return resp.data;
    } catch (err) {
        console.warn('[settings:load] Daemon API failed, falling back to local:', err.message);
    }
    return loadSettings();
});

// App version
ipcMain.handle('app:getVersion', () => {
    return app.getVersion();
});

ipcMain.handle('app:getComponentInfo', async () => {
    try {
        const response = await axios.get(`${IPC_BASE}/api/system/components`);
        return response.data;
    } catch (e) {
        console.warn('[App] Failed to get component info from daemon:', e.message);
        return {
            version: app.getVersion(),
            components: {},
            lastUpdated: null,
        };
    }
});

// Uninstall — GUI/데몬 종료 후 인스톨러를 --uninstall로 실행
ipcMain.handle('app:launchUninstaller', async () => {
    try {
        // 인스톨러 경로 찾기
        const installRoot = getInstallRoot();
        const possibleNames = ['saba-chan-installer.exe', 'Saba-chan Installer.exe'];
        let installerPath = null;

        // 1) 설치 루트 및 하위 폴더에서 검색
        const searchDirs = [
            installRoot,
            path.join(installRoot, 'saba-chan-gui'),
            path.join(installRoot, '..'),
            path.dirname(app.getPath('exe')),
        ];

        for (const dir of searchDirs) {
            for (const name of possibleNames) {
                const candidate = path.join(dir, name);
                if (require('fs').existsSync(candidate)) {
                    installerPath = candidate;
                    break;
                }
            }
            if (installerPath) break;
        }

        // 2) 레지스트리에서 InstallLocation 확인 (Windows)
        if (!installerPath && process.platform === 'win32') {
            try {
                const regOutput = require('child_process').execSync(
                    'reg query "HKCU\\Software\\Microsoft\\Windows\\CurrentVersion\\Uninstall\\Saba-chan" /v InstallLocation 2>nul',
                    { encoding: 'utf8', windowsHide: true }
                );
                const match = regOutput.match(/InstallLocation\s+REG_SZ\s+(.+)/);
                if (match) {
                    const regDir = match[1].trim();
                    for (const name of possibleNames) {
                        const candidate = path.join(regDir, name);
                        if (require('fs').existsSync(candidate)) {
                            installerPath = candidate;
                            break;
                        }
                    }
                }
            } catch (_) { /* 레지스트리 접근 실패 — 무시 */ }
        }

        if (!installerPath) {
            return { success: false, error: 'Installer not found' };
        }

        console.log('[Uninstall] Launching installer:', installerPath);

        // 인스톨러를 --uninstall로 실행 (독립 프로세스)
        const child = require('child_process').spawn(installerPath, ['--uninstall'], {
            detached: true,
            stdio: 'ignore',
        });
        child.unref();

        // GUI 종료 (cleanQuit이 데몬도 종료)
        setTimeout(() => cleanQuit(), 500);

        return { success: true };
    } catch (err) {
        console.error('[Uninstall] Failed to launch installer:', err);
        return { success: false, error: err.message };
    }
});

ipcMain.handle('settings:save', async (_event, settings) => {
    // 데몬이 응답하면 merge base도 API에서 읽는다 (단일 원천 원칙)
    let existing;
    try {
        const resp = await axios.get(`${IPC_BASE}/api/config/gui`, { timeout: 5000 });
        existing = resp.data || {};
    } catch (err) {
        if (err.response) {
            console.warn('[settings:save] Daemon API failed:', err.message);
            return false;
        }
        existing = loadSettings(); // 데몬 없음 — 부트스트랩
    }
    const result = await saveSettings({ ...existing, ...settings });
    refreshIpcBase();
    return result;
});

ipcMain.handle('settings:getPath', () => {
    return getSettingsPath();
});

// Language IPC handlers
ipcMain.handle('language:get', () => {
    return getLanguage();
});

ipcMain.handle('language:set', async (_event, language) => {
    const success = await setLanguage(language);

    // 번역 다시 로드
    translations = loadTranslations();

    // 데몬은 재시작하지 않음 — Python 모듈은 호출 시 환경변수로 언어를 결정하므로
    // 데몬을 재시작하면 실행 중인 서버가 모두 종료됨

    return { success, language };
});

// File dialog handlers
ipcMain.handle('dialog:openFile', async (_event, options) => {
    // 플랫폼별 기본 필터 설정
    let defaultFilters;
    if (process.platform === 'win32') {
        defaultFilters = [
            { name: 'Executable Files', extensions: ['exe'] },
            { name: 'All Files', extensions: ['*'] },
        ];
    } else if (process.platform === 'darwin') {
        defaultFilters = [
            { name: 'Applications', extensions: ['app'] },
            { name: 'All Files', extensions: ['*'] },
        ];
    } else {
        // Linux: 일반적으로 확장자 없음
        defaultFilters = [{ name: 'All Files', extensions: ['*'] }];
    }

    const result = await dialog.showOpenDialog({
        properties: ['openFile'],
        filters: options?.filters || defaultFilters,
    });

    if (result.canceled) {
        return null;
    }
    return result.filePaths[0];
});

ipcMain.handle('dialog:openFolder', async () => {
    const result = await dialog.showOpenDialog({
        properties: ['openDirectory'],
    });

    if (result.canceled) {
        return null;
    }
    return result.filePaths[0];
});

// ── Migration: 고른 폴더가 어느 게임 서버 폴더인지 (D2: 아무 폴더나 열람하던 /api/fs/scan-dir 대신
// 데몬의 게임 폴더 판정만 쓴다. 파일 목록은 돌려주지 않는다) ──
ipcMain.handle('migration:scanDir', async (_event, dirPath) => {
    try {
        if (!dirPath || typeof dirPath !== 'string') {
            return { error: 'Invalid directory path' };
        }
        const response = await axios.post(`${IPC_BASE}/api/modules/detect`, { path: dirPath });
        return { ok: true, ...response.data };
    } catch (error) {
        const errData = error.response?.data;
        return { error: errData?.error || error.message };
    }
});

// Discord 릴레이 에이전트 — 데몬 내장. 봇 본체는 클라우드에서 돈다.
ipcMain.handle('discord:status', async () => {
    try {
        const resp = await axios.get(`${IPC_BASE}/api/relay/agent`, { timeout: 3000 });
        return resp.data?.running ? 'running' : 'stopped';
    } catch (_e) {
        return 'stopped';
    }
});

ipcMain.handle('discord:start', async () => {
    try {
        const resp = await axios.post(`${IPC_BASE}/api/relay/agent/start`, {}, { timeout: 10000 });
        return resp.data;
    } catch (e) {
        const errData = e.response?.data;
        if (errData?.error_code === 'node_token_missing') return { error: 'cloud_token_not_found' };
        return { error: errData?.error || e.message };
    }
});

ipcMain.handle('discord:stop', async () => {
    try {
        const resp = await axios.post(`${IPC_BASE}/api/relay/agent/stop`, {}, { timeout: 10000 });
        return resp.data;
    } catch (e) {
        return errorBody(e);
    }
});

// Bot Config API — 데몬 API 경유
ipcMain.handle('botConfig:load', async () => {
    return loadBotConfig();
});

// Node Token API (클라우드 페어링용) — 데몬 API 경유
ipcMain.handle('nodeToken:save', async (_event, token) => {
    return saveNodeToken(token);
});

ipcMain.handle('nodeToken:load', async () => {
    return loadNodeToken();
});

// 연결 해제: 데몬이 릴레이 에이전트를 멈추고 토큰을 지운다
ipcMain.handle('nodeToken:delete', () => daemonCall('delete', '/api/config/node-token'));

// 설정·서버 목록 내보내기/가져오기 (A9). 파일은 데몬이 읽고 쓴다: 여기서는 경로만 고른다.
// 가져오기는 path 없이 부르면 파일을 고르게 하고, 결과에 path를 붙여 준다 (dryRun 미리보기 뒤 같은 path로 다시 부른다)
ipcMain.handle('dataTransfer:export', async (_event, opts = {}) => {
    const result = await dialog.showSaveDialog({
        defaultPath: `saba-chan-export-${new Date().toISOString().slice(0, 10)}.zip`,
        filters: [{ name: 'Zip', extensions: ['zip'] }],
    });
    if (result.canceled || !result.filePath) return { canceled: true };
    return daemonCall('post', '/api/migration/export', { path: result.filePath, secrets: !!opts.secrets }, 120000);
});

ipcMain.handle('dataTransfer:import', async (_event, opts = {}) => {
    let filePath = typeof opts.path === 'string' && opts.path ? opts.path : null;
    if (!filePath) {
        const result = await dialog.showOpenDialog({
            properties: ['openFile'],
            filters: [{ name: 'Zip', extensions: ['zip'] }],
        });
        if (result.canceled || !result.filePaths[0]) return { canceled: true };
        filePath = result.filePaths[0];
    }
    const body = await daemonCall('post', '/api/migration/import', {
        path: filePath, dryRun: !!opts.dryRun, overwrite: !!opts.overwrite,
    }, 120000);
    return { ...body, path: filePath };
});

// 로그 파일 경로 반환
ipcMain.handle('logs:getPath', async () => {
    return logFilePath || '로그 파일 없음';
});

// 외부 브라우저로 URL 열기
ipcMain.handle('shell:openExternal', async (_event, url) => {
    if (!url || typeof url !== 'string') {
        return { error: 'URL이 지정되지 않았습니다' };
    }
    if (!isWebUrl(url)) {
        return { error: 'http(s) 주소만 열 수 있습니다' };
    }
    try {
        await shell.openExternal(url);
        return { success: true };
    } catch (err) {
        return { error: err.message };
    }
});

// 파일 탐색기로 폴더 열기
ipcMain.handle('shell:openPath', async (_event, folderPath) => {
    if (!folderPath || typeof folderPath !== 'string') {
        return { error: '경로가 지정되지 않았습니다' };
    }
    try {
        const result = await shell.openPath(folderPath);
        if (result) {
            return { error: result };
        }
        return { success: true };
    } catch (err) {
        return { error: err.message };
    }
});

// 로그 폴더 열기
ipcMain.handle('logs:openFolder', async () => {
    const logsDir = path.join(app.getPath('userData'), 'logs');
    if (fs.existsSync(logsDir)) {
        shell.openPath(logsDir);
        return { success: true };
    }
    return { error: '로그 폴더가 없습니다' };
});

ipcMain.handle('botConfig:save', async (_event, config) => {
    // 데몬이 bot-config의 SSOT. 최상위 키 단위로 병합되므로 부분 저장도 안전하다.
    // 릴레이 서버로의 동기화는 데몬 에이전트의 하트비트(botConfig)가 담당한다.
    try {
        if (!(await saveBotConfig(config || {}))) return { error: 'Failed to save bot config' };
        return { success: true, message: 'Bot config saved' };
    } catch (error) {
        console.error('Failed to save bot config:', error.message);
        return { error: error.message };
    }
});

// Window Controls (Title Bar)
// event.sender를 통해 요청을 보낸 BrowserWindow를 찾아서 조작
// (메인 윈도우, 콘솔 팝아웃 등 어떤 창에서 보내더라도 올바른 창이 동작)
ipcMain.on('window:minimize', (event) => {
    const win = BrowserWindow.fromWebContents(event.sender);
    if (win && !win.isDestroyed()) {
        win.minimize();
    }
});

ipcMain.on('window:maximize', (event) => {
    const win = BrowserWindow.fromWebContents(event.sender);
    if (win && !win.isDestroyed()) {
        if (win.isMaximized()) {
            win.restore();
        } else {
            win.maximize();
        }
    }
});

ipcMain.on('window:close', (event) => {
    const win = BrowserWindow.fromWebContents(event.sender);
    if (win && !win.isDestroyed()) {
        win.close();
    }
});
