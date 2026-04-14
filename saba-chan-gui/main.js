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
// 데몬이 시작 시 생성하는 .ipc_token 파일을 읽어서 모든 요청에 X-Saba-Token 헤더로 포함
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

// ═══════════════════════════════════════════════════════════════
// ── http.request 레벨 토큰 주입 (axios AxiosHeaders 우회) ──
// axios 인터셉터/defaults.headers.common 경유로는 Electron 환경에서
// 토큰이 실제 HTTP 요청에 도달하지 않는 문제가 확인됨.
// Node.js http.request() 자체를 패치하여 127.0.0.1:IPC_PORT로 가는
// 모든 요청에 X-Saba-Token 헤더를 강제 주입합니다.
// ═══════════════════════════════════════════════════════════════
const _origHttpRequest = http.request;
http.request = function _patchedRequest(urlOrOptions, optionsOrCallback, _maybeCallback) {
    // http.request(options[, callback]) — 가장 흔한 패턴 (axios 사용)
    // http.request(url[, options][, callback])
    let options;
    if (typeof urlOrOptions === 'object' && !(urlOrOptions instanceof URL)) {
        options = urlOrOptions;
    } else if (typeof optionsOrCallback === 'object' && typeof optionsOrCallback !== 'function') {
        options = optionsOrCallback;
    }

    if (options) {
        const host = options.hostname || options.host || '';
        const port = parseInt(options.port, 10) || 80;
        const ipcPort = parseInt(
            (typeof settings !== 'undefined' && settings && settings.ipcPort) || IPC_PORT_DEFAULT,
            10,
        );

        if ((host === '127.0.0.1' || host === 'localhost') && port === ipcPort) {
            const token = getIpcToken();
            if (token) {
                if (!options.headers) options.headers = {};
                options.headers['X-Saba-Token'] = token;
            }
        }
    }

    return _origHttpRequest.apply(this, arguments);
};

// ── axios 인터셉터 (보조: http.request 패치가 주 메커니즘) ──
axios.interceptors.request.use((config) => {
    // http.request 패치가 토큰을 주입하므로 여기서는 보조적으로만 설정
    const token = getIpcToken();
    if (token && config.headers) {
        if (typeof config.headers.set === 'function') {
            config.headers.set('X-Saba-Token', token);
        } else {
            config.headers['X-Saba-Token'] = token;
        }
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
function getSettingsPath() {
    const userDataPath = app.getPath('userData'); // %APPDATA%/saba-chan
    return path.join(userDataPath, 'settings.json');
}

function loadSettings() {
    const settingsPath = getSettingsPath();
    try {
        if (fs.existsSync(settingsPath)) {
            let data = fs.readFileSync(settingsPath, 'utf8');
            // UTF-8 BOM 제거
            if (data.charCodeAt(0) === 0xfeff) {
                data = data.slice(1);
            }
            const parsed = JSON.parse(data);
            // 기본값 병합 — 파일에 없는 필드만 채움 (기존 필드 보존)
            const systemLanguage = getSystemLanguage();
            return {
                autoRefresh: true,
                refreshInterval: 2000,
                windowBounds: { width: 1200, height: 840 },
                language: systemLanguage,
                ipcPort: IPC_PORT_DEFAULT,
                consoleBufferSize: 2000,
                autoGeneratePasswords: true,
                portConflictCheck: true,
                ...parsed,
            };
        }
    } catch (error) {
        console.error('Failed to load settings:', error);
    }
    // 기본 설정 (시스템 언어로 초기화)
    const systemLanguage = getSystemLanguage();
    return {
        autoRefresh: true,
        refreshInterval: 2000,
        windowBounds: { width: 1200, height: 840 },
        language: systemLanguage,
        ipcPort: IPC_PORT_DEFAULT,
        consoleBufferSize: 2000,
        autoGeneratePasswords: true,
        portConflictCheck: true,
    };
}

function saveSettings(settings) {
    // 데몬이 아직 시작되지 않은 부트스트랩 단계에서는 로컬 파일에 직접 저장
    if (!daemonProcess) {
        try {
            const settingsPath = getSettingsPath();
            const dir = path.dirname(settingsPath);
            if (!fs.existsSync(dir)) fs.mkdirSync(dir, { recursive: true });
            fs.writeFileSync(settingsPath, JSON.stringify(settings, null, 2), 'utf8');
            console.log('[saveSettings] Bootstrap: saved locally (daemon not started)');
        } catch (e) {
            console.error('[saveSettings] Bootstrap local save failed:', e.message);
        }
        return true;
    }

    // 데몬 가동 중 — API 경유로만 설정 저장 (단일 원천 원칙)
    axios.put(`${IPC_BASE}/api/config/gui`, settings, { timeout: 5000 })
        .then(() => console.log('Settings saved via daemon API'))
        .catch((err) => {
            console.error('[saveSettings] Daemon API failed:', err.message);
        });
    return true;
}

// syncGuiConfigToDaemon은 saveSettings에 통합됨 (데몬의 PUT /api/config/gui가 portConflictCheck 동기화도 처리)
async function syncGuiConfigToDaemon(_settings) {
    // no-op: save_gui_config 데몬 핸들러가 portConflictCheck 동기화를 통합 처리
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

    daemonProcess = spawn(daemonPath, ['--spawned'], {
        cwd: rootDir,
        env: daemonEnv,
        stdio: ['ignore', 'pipe', 'pipe'],
        detached: false,
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

// Core Daemon 종료 (크로스 플랫폼)
function stopDaemon() {
    if (!daemonProcess) {
        console.log('Daemon is not running');
        return;
    }

    console.log(`Attempting to stop daemon (PID: ${daemonProcess.pid})`);

    try {
        if (!daemonProcess.killed) {
            if (process.platform === 'win32') {
                // Windows: taskkill로 프로세스 트리 전체 종료
                try {
                    execSync(`taskkill /PID ${daemonProcess.pid} /F /T`, { stdio: 'ignore' });
                    console.log('Daemon terminated via taskkill');
                } catch (e) {
                    console.warn('taskkill failed, trying process.kill:', e.message);
                    daemonProcess.kill('SIGTERM');
                }
                // Windows: 즉시 참조 제거 (taskkill이 동기적으로 종료)
                daemonProcess = null;
                console.log('Daemon stopped');
            } else {
                // Unix/Linux/macOS: SIGTERM으로 우아하게 종료 시도
                daemonProcess.kill('SIGTERM');
                console.log('Sent SIGTERM to daemon');

                // 프로세스 참조를 exit 이벤트에서 정리 (SIGKILL 타이머가 참조 필요)
                const proc = daemonProcess;

                // 2초 후에도 살아있으면 SIGKILL
                const killTimeout = setTimeout(() => {
                    if (proc && !proc.killed) {
                        console.warn('SIGTERM timeout, sending SIGKILL');
                        try {
                            proc.kill('SIGKILL');
                        } catch (e) {
                            console.error('SIGKILL failed:', e);
                        }
                    }
                }, 2000);

                proc.once('exit', () => {
                    clearTimeout(killTimeout);
                    daemonProcess = null;
                    console.log('Daemon stopped');
                });
            }
        }
    } catch (error) {
        console.error('Error stopping daemon:', error);
        daemonProcess = null;
    }
}

// ── Mock Release Server 프로세스 관리 ──────────────────────
let mockServerProcess = null;

ipcMain.handle('mockServer:start', async (_event, options = {}) => {
    if (mockServerProcess && !mockServerProcess.killed) {
        return { ok: true, message: 'Mock server already running', port: 9876 };
    }
    const port = options.port || 9876;
    const version = options.version || '0.2.0';
    const _isDev = !app.isPackaged;
    const rootDir = getInstallRoot();
    const scriptPath = path.join(rootDir, 'scripts', 'mock-release-server.js');

    if (!fs.existsSync(scriptPath)) {
        return { ok: false, error: `Mock server script not found: ${scriptPath}` };
    }

    return new Promise((resolve) => {
        mockServerProcess = spawn('node', [scriptPath, '--port', String(port), '--version', version], {
            cwd: rootDir,
            stdio: ['ignore', 'pipe', 'pipe'],
            detached: false,
        });

        let started = false;
        const timeout = setTimeout(() => {
            if (!started) {
                started = true;
                resolve({ ok: true, message: 'Mock server started (timeout, assumed ready)', port });
            }
        }, 3000);

        mockServerProcess.stdout.on('data', (data) => {
            const line = data.toString();
            console.log('[MockServer]', line.trim());
            // 서버가 listening 시작하면 즉시 resolve
            if (!started && (line.includes('Listening') || line.includes('listen') || line.includes(String(port)))) {
                started = true;
                clearTimeout(timeout);
                resolve({ ok: true, message: `Mock server started on port ${port}`, port });
            }
        });

        mockServerProcess.stderr.on('data', (data) => {
            console.error('[MockServer]', data.toString().trim());
        });

        mockServerProcess.on('error', (err) => {
            console.error('[MockServer] spawn error:', err.message);
            mockServerProcess = null;
            if (!started) {
                started = true;
                clearTimeout(timeout);
                resolve({ ok: false, error: err.message });
            }
        });

        mockServerProcess.on('exit', (code) => {
            console.log('[MockServer] exited with code', code);
            mockServerProcess = null;
        });
    });
});

ipcMain.handle('mockServer:stop', async () => {
    if (!mockServerProcess || mockServerProcess.killed) {
        mockServerProcess = null;
        return { ok: true, message: 'Mock server not running' };
    }
    mockServerProcess.kill('SIGTERM');
    // Windows에서는 SIGTERM이 작동하지 않을 수 있으므로 fallback
    setTimeout(() => {
        if (mockServerProcess && !mockServerProcess.killed) {
            mockServerProcess.kill('SIGKILL');
        }
    }, 1000);
    mockServerProcess = null;
    return { ok: true, message: 'Mock server stopped' };
});

ipcMain.handle('mockServer:status', async () => {
    const running = mockServerProcess != null && !mockServerProcess.killed;
    return { running };
});

// ── 프로세스 완전 분리 스폰 (Chromium Job Object 회피) ──────
// Chromium(Electron)은 프로덕션에서 Job Object로 자식 프로세스를 관리하며,
// app.quit() 시 JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE로 자식을 모두 종료합니다.
// detached: true만으로는 Job Object에서 벗어나지 못하므로,
// Windows에서는 cmd.exe /c start로 스폰하여 완전히 분리합니다.
function spawnDetached(exe, args) {
    if (process.platform === 'win32') {
        // cmd /c start "" /B "exe" args...
        // /B: 새 창 열지 않음, "": 타이틀 빈 문자열
        // shell: true + cmd start 조합으로 Chromium Job Object에서 벗어남
        // 공백이 포함된 인자를 따옴표로 감싸야 cmd.exe가 올바르게 파싱
        const quotedArgs = args.map(a => (a.includes(' ') ? `"${a}"` : a));
        const proc = spawn('cmd.exe', ['/c', 'start', '""', '/B', `"${exe}"`, ...quotedArgs], {
            detached: true,
            stdio: 'ignore',
            shell: true,
            windowsHide: true,
        });
        proc.unref();
    } else {
        const proc = spawn(exe, args, {
            detached: true,
            stdio: 'ignore',
        });
        proc.unref();
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

        // 1.5. Mock 서버 종료
        if (mockServerProcess && !mockServerProcess.killed) {
            console.log('Stopping mock server process...');
            mockServerProcess.kill();
            mockServerProcess = null;
        }

        // 2. 데몬 종료
        stopDaemon();

        // 2. 데몬이 종료될 때까지 대기 (최대 3초)
        let attempts = 0;
        while (daemonProcess && !daemonProcess.killed && attempts < 6) {
            await wait(500);
            attempts++;
        }

        if (daemonProcess) {
            console.warn('Daemon still running after waiting, force killing');
            try {
                if (process.platform === 'win32') {
                    // Windows: taskkill로 강제 종료
                    execSync(`taskkill /PID ${daemonProcess.pid} /F /T 2>nul`, { stdio: 'ignore' });
                } else {
                    // Unix/Linux/macOS: SIGKILL로 강제 종료
                    daemonProcess.kill('SIGKILL');
                }
            } catch (e) {
                console.debug('Force kill error (process may already be dead):', e.message);
            }
        }

        daemonProcess = null;

        // 2.5. Fallback: 프로세스 이름으로 saba-core 종료 (daemonProcess가 null이었던 경우 대비)
        if (process.platform === 'win32') {
            try {
                execSync('taskkill /IM saba-core.exe /F 2>nul', { stdio: 'ignore' });
                console.log('Fallback: killed saba-core.exe by name');
            } catch (_e) {
                // 이미 종료됨 — 무시
            }
        } else {
            try {
                execSync('pkill -f saba-core 2>/dev/null || true', { stdio: 'ignore' });
            } catch (_e) {
                // 무시
            }
        }

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

// 이미 떠 있는 데몬이 있으면 재실행하지 않고 재사용
async function ensureDaemon() {
    try {
        // IPC 토큰을 먼저 로드 (이미 데몬이 떠있을 수 있으므로)
        loadIpcToken();
        // /health 엔드포인트로 체크 (lock / 디스크 I/O 없이 즉시 응답)
        sendStatus('daemon', t('daemon.checking'));
        const response = await axios.get(`${IPC_BASE}/health`, { timeout: 1000 });
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
            // 토큰 재로드 후 검증 재시도 (최대 3회, 500ms 간격)
            for (let retry = 0; retry < 3; retry++) {
                loadIpcToken();
                try {
                    const verifyResp = await axios.get(`${IPC_BASE}/health`, { timeout: 1000 });
                    if (verifyResp.status === 200) {
                        console.log('✓ Token refreshed and verified');
                        sendStatus('daemon', t('daemon.existing_running'));
                        await syncInstallRoot();
                        return;
                    }
                } catch (verifyErr) {
                    console.warn(`[Auth] Token verify attempt ${retry + 1} failed:`, verifyErr.message);
                }
                await wait(500);
            }
            // 3회 실패해도 일단 진행 (GUI는 표시하고 이후 자동 복구에 맡김)
            console.warn('[Auth] Token verification failed after 3 retries, proceeding anyway');
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

async function preloadLightData() {
    // 레거시: 응답을 버리는 워밍업 요청이었으나 Rust 데모닌에 HTTP 캐시가 없으므로
    // supervisor lock만 유발하는 순 오버헤드였음. 렌더러가 이미 로드 시 실제 데이터를 페치하므로 여기서는
    // 로딩 상태 변경만 수행한다.
    sendStatus('modules', '새 모듈 목록 준비 중...');
    sendStatus('instances', '인스턴스 목록 준비 중...');
}

// ── Client Heartbeat (데몬이 GUI 생존 여부를 추적) ────────────
let heartbeatClientId = null;
let heartbeatTimer = null;

async function registerWithDaemon() {
    try {
        const res = await axios.post(`${IPC_BASE}/api/client/register`, { kind: 'gui' }, { timeout: 3000 });
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
            let botPid = null;
            try {
                const statusResp = await axios.get(`${IPC_BASE}/api/ext-process/discord-bot/status`, { timeout: 2000 });
                if (statusResp.data?.status === 'running') {
                    botPid = statusResp.data.pid ?? null;
                }
            } catch (_) { /* 무시 */ }
            await axios.post(
                `${IPC_BASE}/api/client/${heartbeatClientId}/heartbeat`,
                {
                    bot_pid: botPid,
                },
                { timeout: 3000 },
            );
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
    await preloadLightData();

    // 데몬에 클라이언트 등록 및 heartbeat 시작
    await registerWithDaemon();
    startHeartbeat();

    // 업데이트 주기적 체크 시작 (기본 3시간 간격)
    startUpdateChecker();

    sendStatus('ready', '백그라운드 초기화 완료');

    // 데몬에 GUI 설정 초기 동기화 (portConflictCheck 등)
    const currentSettings = loadSettings();
    syncGuiConfigToDaemon(currentSettings).catch(err => {
        console.warn('[Init] Failed to sync initial GUI config to daemon:', err.message);
    });

    // Discord Bot 자동 시작은 React App.js에서 처리
}

// runDeferredTasks 제거됨 - Discord Bot 자동 시작은 React에서 처리

// ── 업데이터 exe 경로 해석 ────────────────────────────────────

/**
 * 업데이터 exe 경로를 찾습니다.
 * 개발: updater/gui/src-tauri/target/{release,debug}/saba-chan-updater.exe
 * 프로덕션: exe와 같은 디렉토리의 saba-chan-updater.exe
 */
function findUpdaterExe() {
    const isDev = !app.isPackaged;
    if (isDev) {
        const rootDir = path.join(__dirname, '..');
        // workspace root target (cargo workspace가 여기에 빌드)
        const wsRelease = path.join(rootDir, 'target', 'release', 'saba-chan-updater.exe');
        const wsDebug = path.join(rootDir, 'target', 'debug', 'saba-chan-updater.exe');
        // crate-local target (fallback)
        const crateRelease = path.join(
            rootDir,
            'updater',
            'gui',
            'src-tauri',
            'target',
            'release',
            'saba-chan-updater.exe',
        );
        const crateDebug = path.join(
            rootDir,
            'updater',
            'gui',
            'src-tauri',
            'target',
            'debug',
            'saba-chan-updater.exe',
        );
        // workspace root 우선, 최신 빌드가 여기 있음
        if (fs.existsSync(wsRelease)) return wsRelease;
        if (fs.existsSync(crateRelease)) return crateRelease;
        if (fs.existsSync(wsDebug)) return wsDebug;
        if (fs.existsSync(crateDebug)) return crateDebug;
        return null;
    } else {
        // 설치 루트에서 찾기 (portable: 원본 exe 디렉토리)
        const rootDir = getInstallRoot();
        const p = path.join(rootDir, 'saba-chan-updater.exe');
        if (fs.existsSync(p)) return p;
        // fallback: 추출 temp 디렉토리
        const tempDir = path.dirname(app.getPath('exe'));
        const tp = path.join(tempDir, 'saba-chan-updater.exe');
        return fs.existsSync(tp) ? tp : null;
    }
}

// ── 업데이트 주기적 체크 (데몬 HTTP API) ────────────────────
const UPDATE_CHECK_INTERVAL_MS = 3 * 60 * 60 * 1000; // 3시간
const UPDATE_INITIAL_DELAY_MS = 0; // 데몬 준비 후 즉시 체크
let updateCheckTimer = null;
// 마지막으로 OS 알림을 보낸 업데이트 목록의 fingerprint (중복 알림 방지)
let lastNotifiedUpdateKey = null;

async function checkForUpdates() {
    try {
        // 데몬 API를 통해 업데이트 확인
        const response = await axios.post(`${IPC_BASE}/api/updates/check`, {}, { timeout: 30000 });
        const data = response.data;

        if (!data.ok) {
            console.warn('[UpdateChecker] Check failed:', data.error);
            return;
        }

        if (data.updates_available > 0) {
            // Locales는 백그라운드 자동 적용 대상이므로 알림에서 제외 (서버에서도 필터링하지만 안전장치)
            const names = (data.update_names || []).filter(n => n !== 'Locales');
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

            // 자동 다운로드/적용은 하지 않음 — 사용자가 업데이트 센터에서 수동 처리
            // auto_download/auto_apply 설정은 향후 구현 예정
        } else {
            console.log('[UpdateChecker] No updates available');
        }
    } catch (e) {
        console.warn('[UpdateChecker] Check failed:', e.message);
    }
}

function startUpdateChecker() {
    // config의 enabled 플래그를 확인하여 비활성화 상태이면 체크하지 않음
    (async () => {
        try {
            const response = await axios.get(`${IPC_BASE}/api/updates/config`, { timeout: 5000 });
            const cfg = response.data?.config || response.data;
            if (cfg?.enabled === false) {
                console.log('[UpdateChecker] Auto-check disabled by config');
                return;
            }
        } catch (_) {
            // config 조회 실패 시 기본 동작(체크 실행)
        }
        _doStartUpdateChecker();
    })();
}

function _doStartUpdateChecker() {
    stopUpdateChecker();
    if (UPDATE_INITIAL_DELAY_MS > 0) {
        setTimeout(() => {
            checkForUpdates();
            updateCheckTimer = setInterval(checkForUpdates, UPDATE_CHECK_INTERVAL_MS);
        }, UPDATE_INITIAL_DELAY_MS);
    } else {
        checkForUpdates();
        updateCheckTimer = setInterval(checkForUpdates, UPDATE_CHECK_INTERVAL_MS);
    }
}

function stopUpdateChecker() {
    if (updateCheckTimer) {
        clearInterval(updateCheckTimer);
        updateCheckTimer = null;
    }
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
        webPreferences: {
            preload: path.join(__dirname, 'preload.js'),
            nodeIntegration: false,
            contextIsolation: true,
        },
    });

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
            click: () => {
                stopDaemon();
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
            // http는 top-level에서 require하고 패치된 버전 사용 — 토큰 자동 주입됨
            const req = http.request({
                hostname: '127.0.0.1',
                port: currentPort,
                path: `/api/client/${heartbeatClientId}/unregister`,
                method: 'DELETE',
                timeout: 1000,
            });
            req.end();
        } catch (_e) {
            /* 무시 */
        }
        heartbeatClientId = null;
    }

    // Discord 봇 종료 요청 (데몬 API 경유 — fire and forget)
    // 인터페이스만 종료 시에는 데몬이 계속 봇을 관리하므로 스킵
    if (!keepDaemonAlive) {
        try {
            const tokenForShutdown = getIpcToken();
            const shutdownReq = http.request({
                hostname: '127.0.0.1',
                port: currentPort,
                path: '/api/ext-process/discord-bot/stop',
                method: 'POST',
                headers: {
                    'Content-Type': 'application/json',
                    'X-Saba-Token': tokenForShutdown,
                },
                timeout: 2000,
            });
            shutdownReq.write('{}');
            shutdownReq.end();
        } catch (_e) { /* 무시 */ }
    }

    // 데몬 프로세스 종료 (인터페이스만 종료 시에는 데몬 유지)
    if (!keepDaemonAlive) {
        stopDaemon();
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
    // 혹시 남아있을 데몬 프로세스 강제 종료
    if (daemonProcess && !daemonProcess.killed) {
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

ipcMain.handle('server:start', async (_event, name, options = {}) => {
    try {
        if (!options.module) {
            return { error: '모듈이 지정되지 않았습니다. 인스턴스 설정을 확인해주세요.' };
        }
        const body = {
            module: options.module,
            config: options.config || {},
        };
        const response = await axios.post(`${IPC_BASE}/api/server/${name}/start`, body, { timeout: 30000 });
        return response.data;
    } catch (error) {
        if (error.response) {
            const status = error.response.status;
            const data = error.response.data;

            switch (status) {
                case 400:
                    return { error: t('server.start_failed', { error: data.error || t('info') }) };
                case 404:
                    return { error: t('server.not_found', { name }) };
                case 409:
                    return { error: t('server.already_running', { name }) };
                case 500:
                    return { error: `${t('error')}: ${data.error || data.message}` };
                default:
                    return { error: t('server.start_failed', { error: data.error || error.message }) };
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
        return { error: error.response?.data?.error || error.message };
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
        return { error: error.response?.data?.error || error.message };
    }
});

ipcMain.handle('instance:getInstalledVersion', async (_event, instanceId) => {
    try {
        const response = await axios.get(`${IPC_BASE}/api/instance/${instanceId}/installed-version`, { timeout: 10000 });
        return response.data;
    } catch (error) {
        return { error: error.response?.data?.error || error.message };
    }
});

ipcMain.handle('instance:resetProperties', async (_event, instanceId) => {
    try {
        const response = await axios.post(
            `${IPC_BASE}/api/instance/${instanceId}/properties/reset`,
            {},
            { timeout: 10000 },
        );
        return response.data;
    } catch (error) {
        return { error: error.response?.data?.error || error.message };
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
        return { error: error.response?.data?.error || error.message };
    }
});

// ── Managed Process API (stdin/stdout capture) ───────────────

ipcMain.handle('managed:start', async (_event, instanceId, options = {}) => {
    try {
        const response = await axios.post(
            `${IPC_BASE}/api/instance/${instanceId}/managed/start`,
            { config: options.config || {} },
            { timeout: 30000 },
        );
        return response.data;
    } catch (error) {
        return { error: error.response?.data?.error || error.message };
    }
});

ipcMain.handle('managed:console', async (_event, instanceId, since = 0, count = 200) => {
    try {
        const response = await axios.get(`${IPC_BASE}/api/instance/${instanceId}/console`, {
            params: { since, count },
        });
        return response.data;
    } catch (error) {
        return { error: error.response?.data?.error || error.message };
    }
});

ipcMain.handle('managed:stdin', async (_event, instanceId, command) => {
    try {
        const response = await axios.post(`${IPC_BASE}/api/instance/${instanceId}/stdin`, { command });
        return response.data;
    } catch (error) {
        return { error: error.response?.data?.error || error.message };
    }
});

// ── Console Popout (PiP) Window ──────────────────────────────
const consolePopoutWindows = new Map(); // instanceId → BrowserWindow

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
        webPreferences: {
            preload: path.join(__dirname, 'preload.js'),
            nodeIntegration: false,
            contextIsolation: true,
        },
    });

    popout.removeMenu();

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
        return { ok: false, error: error.response?.data?.error || error.message };
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
        return { ok: false, error: error.response?.data?.error || error.message };
    }
});

ipcMain.handle('module:remove', async (_event, moduleId) => {
    try {
        const response = await axios.delete(`${IPC_BASE}/api/modules/${moduleId}`, { timeout: 15000 });
        return response.data;
    } catch (error) {
        return { ok: false, error: error.response?.data?.error || error.message };
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

            switch (status) {
                case 400:
                    return { error: `잘못된 요청: ${errData.error || '입력값을 확인해주세요'}` };
                case 409:
                    return { error: `이미 존재하는 인스턴스 이름입니다` };
                case 500:
                    return { error: `인스턴스 생성 오류: ${errData.error || errData.message || '내부 오류 발생'}` };
                default:
                    return { error: `생성 실패 (HTTP ${status}): ${errData.error || error.message}` };
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
        return { success: false, error: error.response?.data?.error || error.message };
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
                    return { error: `잘못된 설정값: ${data.error || '입력값을 확인해주세요'}` };
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
function buildValidatedBody(inputs, args, inlineMessage) {
    const body = {};
    if (inputs && inputs.length > 0) {
        for (const field of inputs) {
            const value = args?.[field.name];

            // 필수 필드 확인
            if (field.required && (value === undefined || value === null || value === '')) {
                throw new Error(`필수 필드 '${field.label || field.name}'이(가) 누락되었습니다`);
            }

            // 값이 있으면 타입 검증 및 추가
            if (value !== undefined && value !== null && value !== '') {
                if (field.type === 'number') {
                    const numValue = Number(value);
                    if (isNaN(numValue)) {
                        throw new Error(`'${field.label || field.name}'은(는) 숫자여야 합니다`);
                    }
                    body[field.name] = numValue;
                } else {
                    body[field.name] = String(value);
                }
            } else if (field.default !== undefined) {
                body[field.name] = field.default;
            }
        }
    }
    // 입력 스키마가 비어 있지만 사용자가 인라인으로 메시지를 입력한 경우
    if (inlineMessage && Object.keys(body).length === 0) {
        body.message = inlineMessage;
    }
    return body;
}

ipcMain.handle('instance:executeCommand', async (_event, id, command) => {
    try {
        console.log(`[Main] Executing command for instance ${id}:`, command);

        // 사용자가 "announce hi" 같이 입력하면 첫 단어는 명령어, 나머지는 메시지로 분리
        const rawCommand = command.command || '';
        const [cmdName, ...restParts] = rawCommand.trim().split(/\s+/);
        const inlineMessage = restParts.join(' ');

        // Step 1: 인스턴스 정보 가져오기
        const instanceUrl = `${IPC_BASE}/api/instance/${id}`;
        const instanceResponse = await axios.get(instanceUrl);
        const instance = instanceResponse.data;

        console.log(`[Main] Instance module: ${instance.module_name}`);

        // Step 2: 명령어 메타데이터 확인 (프론트엔드에서 전달받거나 없으면 null)
        // commandMetadata는 module.toml의 commands.fields 중 하나 — method, rcon_template, endpoint_template 등 포함
        const cmdMeta = command.commandMetadata || null;
        const method = cmdMeta?.method || null;
        const args = command.args || {};

        console.log(`[Main] Command: ${cmdName}, method: ${method || '(none → stdin/command fallback)'}`);

        // Step 3: method에 따라 프로토콜 라우팅 (모듈 이름 참조 없음!)
        //   rcon  → RCON 템플릿 치환 후 /rcon 엔드포인트
        //   rest  → REST endpoint_template + http_method 로 /rest 엔드포인트
        //   dual  → Python lifecycle 모듈이 프로토콜 선택 (/command 엔드포인트)
        //   없음  → 기본 command 엔드포인트 (stdin 기반)
        let protocolUrl;
        let commandPayload;

        if (method === 'rcon') {
            // RCON: rcon_template에서 입력값을 치환하여 명령어 생성
            let rconCmd = cmdMeta?.rcon_template || cmdName;
            for (const [key, value] of Object.entries(args)) {
                if (value !== undefined && value !== null && value !== '') {
                    rconCmd = rconCmd.replace(`{${key}}`, value);
                }
            }
            // 치환되지 않은 선택적 파라미터 제거
            rconCmd = rconCmd.replace(/\s*\{\w+\}/g, '').trim();

            console.log(`[Main] RCON command: ${rconCmd}`);
            protocolUrl = `${IPC_BASE}/api/instance/${id}/rcon`;
            commandPayload = { command: rconCmd };
        } else if (method === 'rest') {
            // REST: endpoint_template과 http_method로 직접 API 호출
            const endpoint = cmdMeta?.endpoint_template || `/v1/api/${cmdName}`;
            const httpMethod = (cmdMeta?.http_method || 'GET').toUpperCase();
            const validatedBody = buildValidatedBody(cmdMeta?.inputs, args, inlineMessage);

            console.log(`[Main] REST ${httpMethod} ${endpoint}`, validatedBody);
            protocolUrl = `${IPC_BASE}/api/instance/${id}/rest`;
            commandPayload = {
                endpoint,
                method: httpMethod,
                body: validatedBody,
                instance_id: id,
                rest_host: instance.rest_host,
                rest_port: instance.rest_port,
            };
        } else if (method === 'dual') {
            // Dual: Python lifecycle 모듈이 REST/RCON을 내부적으로 선택
            // (예: Palworld lifecycle.py가 플레이어 ID 변환 + 프로토콜 라우팅 수행)
            const validatedBody = buildValidatedBody(cmdMeta?.inputs, args, inlineMessage);

            console.log(`[Main] Dual-mode via module lifecycle: ${cmdName}`, validatedBody);
            protocolUrl = `${IPC_BASE}/api/instance/${id}/command`;
            commandPayload = {
                command: cmdName,
                args: validatedBody,
                instance_id: id,
            };
        } else {
            // 메서드 미지정: 기본 command 엔드포인트 (stdin 기반 또는 모듈 lifecycle 처리)
            console.log(`[Main] Generic command endpoint: ${cmdName}`);
            protocolUrl = `${IPC_BASE}/api/instance/${id}/command`;
            commandPayload = {
                command: cmdName,
                args: args,
                instance_id: id,
            };
        }

        // RCON/REST는 빠르지만, /command (Python lifecycle)는 subprocess 스폰 시간이 필요
        const requestTimeout = method === 'dual' || !method ? 30000 : 10000;
        console.log(`[Main] POST ${protocolUrl} (timeout: ${requestTimeout}ms)`);
        const response = await axios.post(protocolUrl, commandPayload, { timeout: requestTimeout });
        console.log(`[Main] Response:`, response.data);

        return response.data;
    } catch (error) {
        console.error(`[Main] Error executing command:`, error.message, error.response?.data || '');

        // HTTP 응답 에러 → 상태 코드 기반 분류 (모듈명 참조 없음)
        if (error.response) {
            const status = error.response.status;
            const data = error.response.data;
            const detail = data?.error || data?.message || '';

            const errorMap = {
                400: `잘못된 요청: ${detail || '입력값을 확인해주세요'}`,
                401: `인증 실패: 서버 설정에서 사용자명/비밀번호를 확인해주세요`,
                403: `접근 거부: 권한이 없습니다`,
                404: `명령어를 찾을 수 없음: 서버가 실행중이지 않거나 명령어가 존재하지 않습니다`,
                500: `서버 내부 오류: ${detail || '서버에서 오류가 발생했습니다'}`,
                503: `서비스 사용 불가: 서버가 응답하지 않습니다. 서버 상태를 확인해주세요`,
            };

            return { error: errorMap[status] || `오류 (HTTP ${status}): ${detail || error.message}` };
        }

        // 네트워크 에러 → 에러 코드 기반 분류
        const networkErrors = {
            ECONNREFUSED: '데몬에 연결할 수 없습니다. 데몬이 실행중인지 확인해주세요',
            ETIMEDOUT: '요청 시간 초과: 서버가 응답하지 않습니다',
            ENOTFOUND: '서버를 찾을 수 없습니다. 네트워크 설정을 확인해주세요',
        };

        return { error: networkErrors[error.code] || `명령어 실행 실패: ${error.message}` };
    }
});

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

ipcMain.handle('relay:initiatePairing', async (_event, payload) => {
    try {
        const resp = await axios.post(`${IPC_BASE}/api/relay/pair/initiate`, payload, {
            timeout: 8000,
        });
        return resp.data;
    } catch (err) {
        return { error: err.message };
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
        return { error: err.message };
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

// 업데이터 exe 스폰 — 업데이트 적용 전용
// 업데이터가 apply-targets.json에서 적용 대상을 읽고, 파일 교체 후 GUI를 재실행합니다.
// install_root와 컴포넌트 목록은 업데이터가 자동 추론하므로 전달하지 않습니다.
ipcMain.handle('updater:launchApply', async (_event, _targets) => {
    try {
        const updaterExe = findUpdaterExe();
        if (!updaterExe) {
            return { ok: false, error: 'Updater exe not found' };
        }

        const args = ['--apply'];

        // GUI exe 경로: 업데이터가 적용 완료 후 재실행할 대상
        let guiExe;
        if (!app.isPackaged) {
            guiExe = process.execPath; // 개발 모드: electron exe
            args.push('--relaunch', guiExe, path.resolve(__dirname));
        } else if (process.env.PORTABLE_EXECUTABLE_FILE) {
            guiExe = process.env.PORTABLE_EXECUTABLE_FILE;
            args.push('--relaunch', guiExe);
        } else {
            // 프로덕션: 업데이터가 같은 폴더의 saba-chan-gui.exe를 자동 추론
            // --relaunch 생략 가능하지만, 명시적으로 전달
            guiExe = app.getPath('exe');
            args.push('--relaunch', guiExe);
        }

        console.log(`[Updater] Launching apply: ${updaterExe} ${args.join(' ')}`);
        spawnDetached(updaterExe, args);

        // 업데이터가 실행되면 GUI는 항상 종료 (업데이터가 재실행 담당)
        setTimeout(() => cleanQuit(), 500);
        return { ok: true };
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

// 업데이트 설정 변경 — 데몬 API + 백그라운드 체커 연동
ipcMain.handle('updater:setConfig', async (_event, config) => {
    try {
        const response = await axios.post(`${IPC_BASE}/api/updates/config`, config, { timeout: 5000 });
        // enabled 플래그가 변경된 경우 백그라운드 체커 시작/중지
        if (config && typeof config.enabled === 'boolean') {
            if (config.enabled) {
                console.log('[UpdateChecker] Auto-check enabled — starting background checker');
                _doStartUpdateChecker();
            } else {
                console.log('[UpdateChecker] Auto-check disabled — stopping background checker');
                stopUpdateChecker();
            }
        }
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
        if (daemonProcess && !daemonProcess.killed) {
            console.log('Killing existing daemon process...');
            daemonProcess.kill();
            await wait(1000);
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
    if (daemonProcess && !daemonProcess.killed) {
        try {
            const resp = await axios.get(`${IPC_BASE}/api/config/gui`, { timeout: 5000 });
            return resp.data;
        } catch (err) {
            console.warn('[settings:load] Daemon API failed, falling back to local:', err.message);
        }
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

ipcMain.handle('guiConfig:sync', async (_event, config) => {
    return syncGuiConfigToDaemon(config);
});

ipcMain.handle('settings:save', async (_event, settings) => {
    // 데몬 가동 중이면 merge base도 API에서 로드 → API로 저장 (단일 원천 원칙)
    if (daemonProcess && !daemonProcess.killed) {
        try {
            const resp = await axios.get(`${IPC_BASE}/api/config/gui`, { timeout: 5000 });
            const existing = resp.data || {};
            const merged = { ...existing, ...settings };
            await axios.put(`${IPC_BASE}/api/config/gui`, merged, { timeout: 5000 });
            refreshIpcBase();
            return true;
        } catch (err) {
            console.warn('[settings:save] Daemon API failed:', err.message);
            return false;
        }
    }
    // 부트스트랩 단계 — 로컬 파일 사용
    const existing = loadSettings();
    const merged = { ...existing, ...settings };
    const result = saveSettings(merged);
    refreshIpcBase();
    return result;
});

ipcMain.handle('settings:getPath', () => {
    return getSettingsPath();
});

ipcMain.handle('paths:getFixed', () => {
    return {
        modulesPath: getFixedModulesPath(),
        extensionsPath: getFixedExtensionsPath(),
    };
});

// Language IPC handlers
ipcMain.handle('language:get', () => {
    return getLanguage();
});

ipcMain.handle('language:set', async (_event, language) => {
    const success = setLanguage(language);

    // 번역 다시 로드
    translations = loadTranslations();

    // 데몬은 재시작하지 않음 — Python 모듈은 호출 시 환경변수로 언어를 결정하므로
    // 데몬을 재시작하면 실행 중인 서버가 모두 종료됨

    // Discord 봇이 실행 중이면 데몬 API로 정지 후 렌더러에 재시작 신호 전송
    try {
        const statusResp = await axios.get(`${IPC_BASE}/api/ext-process/discord-bot/status`);
        if (statusResp.data?.status === 'running') {
            console.log('Stopping Discord bot to apply new language setting...');
            await axios.post(`${IPC_BASE}/api/ext-process/discord-bot/stop`, {}, { timeout: 5000 });

            // 렌더러에 재시작 신호 전송 (봇 설정을 데몬에서 로드)
            setTimeout(async () => {
                try {
                    const botConfig = await loadBotConfig();
                    if (mainWindow && !mainWindow.isDestroyed()) {
                        mainWindow.webContents.send('bot:relaunch', botConfig);
                    }
                } catch (error) {
                    console.error('Failed to relaunch Discord bot:', error);
                }
            }, 500);
        }
    } catch (_e) {
        // 데몬 미연결 — 봇 미실행으로 간주
    }

    return { success, language };
});

ipcMain.handle('language:getSystem', () => {
    return getSystemLanguage();
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

// ── Migration: 디렉토리 스캔 (파일 목록 반환) ──
ipcMain.handle('migration:scanDir', async (_event, dirPath) => {
    try {
        if (!dirPath || typeof dirPath !== 'string') {
            return { error: 'Invalid directory path' };
        }
        const response = await axios.post(`${IPC_BASE}/api/fs/scan-dir`, { path: dirPath });
        return response.data;
    } catch (error) {
        const errData = error.response?.data;
        return { error: errData?.error || error.message };
    }
});

// Discord Bot process management — 데몬 API 경유
// GUI는 봇 프로세스를 직접 관리하지 않음. 모든 봇 수명주기는 데몬이 담당.

// ── 봇 프로세스 IPC 응답 관리 (데몬 경유 stdin으로 전환) ──
const pendingBotIpcRequests = new Map(); // id → { resolve, timer }
let botIpcIdCounter = 0;

async function sendBotIpcRequest(msg, timeoutMs = 15000) {
    const id = String(++botIpcIdCounter);
    // 데몬의 stdin API로 봇에 메시지 전송
    try {
        await axios.post(`${IPC_BASE}/api/ext-process/discord-bot/stdin`, {
            message: JSON.stringify({ ...msg, id }),
        });
    } catch (e) {
        throw new Error('Bot IPC unavailable: ' + e.message);
    }

    // 데몬의 console buffer에서 __IPC__ 응답을 폴링
    const ipcPrefix = '__IPC__:';
    const deadline = Date.now() + timeoutMs;
    const pollInterval = 500;
    let lastLineCount = 0;

    while (Date.now() < deadline) {
        await new Promise(r => setTimeout(r, pollInterval));
        try {
            const consoleResp = await axios.get(
                `${IPC_BASE}/api/ext-process/discord-bot/console`,
                { timeout: 3000 },
            );
            const lines = consoleResp.data?.lines || consoleResp.data?.console || [];
            // 새 줄만 탐색 (이전 폴링 이후 추가된 줄)
            const startIdx = Math.max(0, lastLineCount - 5); // 약간 겹치게 (누락 방지)
            lastLineCount = lines.length;
            for (let i = lines.length - 1; i >= startIdx; i--) {
                const line = typeof lines[i] === 'string' ? lines[i] : lines[i]?.line || '';
                // [stdout] 및 [stderr] 접두사 제거
                const stripped = line.replace(/^\[(stdout|stderr)\]\s*/, '');
                if (!stripped.startsWith(ipcPrefix)) continue;
                try {
                    const payload = JSON.parse(stripped.slice(ipcPrefix.length));
                    if (payload.id === id) return payload;
                } catch (_) {}
            }
        } catch (_) {}
    }

    return { error: 'TIMEOUT', message: 'Bot IPC response timeout' };
}

ipcMain.handle('discord:status', async () => {
    try {
        const resp = await axios.get(`${IPC_BASE}/api/ext-process/discord-bot/status`);
        return resp.data?.status || 'stopped';
    } catch (e) {
        return 'stopped';
    }
});

// ── 봇에 연결된 Discord 길드 멤버 목록 조회 (로컬 모드 전용) ──
ipcMain.handle('discord:guildMembers', async () => {
    try {
        const resp = await sendBotIpcRequest({ type: 'getGuildMembers' }, 15000);
        if (resp.error) {
            return { error: resp.error };
        }
        return { data: resp.data || {} };
    } catch (e) {
        return { error: e.message };
    }
});

ipcMain.handle('discord:guildChannels', async () => {
    try {
        const resp = await sendBotIpcRequest({ type: 'getGuildChannels' }, 15000);
        if (resp.error) {
            return { error: resp.error };
        }
        return { data: resp.data || {} };
    } catch (e) {
        return { error: e.message };
    }
});

ipcMain.handle('discord:start', async (_event, config) => {
    // 봇 설정을 데몬에 저장 (데몬이 SSOT)
    let existingConfig = {};
    try {
        existingConfig = await loadBotConfig();
    } catch (_) {}

    const configToSave = {
        ...existingConfig,
        prefix: config.prefix || '!saba',
        token: config.token || existingConfig.token || '',
        autoStart: config.autoStart ?? existingConfig.autoStart ?? false,
        moduleAliases: config.moduleAliases || {},
        commandAliases: config.commandAliases || {},
        musicEnabled: config.musicEnabled !== false,
        musicChannelId: config.musicChannelId || existingConfig.musicChannelId || '',
        musicUISettings: config.musicUISettings || existingConfig.musicUISettings || {},
        nodeSettings: config.nodeSettings && Object.keys(config.nodeSettings).length > 0
            ? config.nodeSettings
            : (existingConfig.nodeSettings || {}),
        mode: config.mode || existingConfig.mode || 'local',
        cloud: config.cloud || existingConfig.cloud || {},
        cloudNodes: config.cloudNodes || existingConfig.cloudNodes || [],
        cloudMembers: config.cloudMembers || existingConfig.cloudMembers || {},
    };
    await saveBotConfig(configToSave);

    // ★ 클라우드 모드: 릴레이 서버에 botConfig 동기화
    if (config.mode === 'cloud') {
        const relayUrl = config.cloud?.relayUrl;
        const hostId = config.cloud?.hostId;
        if (relayUrl && hostId) {
            try {
                await axios.patch(`${relayUrl}/api/hosts/${hostId}/bot-config`, {
                    prefix: config.prefix || '!saba',
                    moduleAliases: config.moduleAliases || {},
                    commandAliases: config.commandAliases || {},
                });
            } catch (e) {
                console.warn('[Discord Bot] Failed to sync botConfig to relay:', e.message);
            }
        }
    }

    // 범용 프로세스 시작 API를 통해 봇 실행
    try {
        // Node.js 경로 확인 (포터블 → 시스템 폴백)
        let nodePath = 'node';
        try {
            const nodeResp = await axios.post(`${IPC_BASE}/api/node-env/setup`, {}, { timeout: 30000 });
            if (nodeResp.data?.success && nodeResp.data.node_path) {
                nodePath = nodeResp.data.node_path;
            }
        } catch (_) { /* 시스템 node 폴백 */ }

        const botDir = path.join(__dirname, '..', 'discord_bot');
        const envVars = {
            IPC_BASE,
            SABA_LANG: getLanguage(),
            BOT_CONFIG_PATH: path.join(getSabaDataDir(), 'bot-config.json'),
            SABA_EXTENSIONS_DIR: getFixedExtensionsPath(),
        };

        if (config.mode === 'cloud') {
            const nodeToken = await loadNodeToken();
            if (!nodeToken) {
                return { error: 'cloud_token_not_found' };
            }
            const relayUrl = (config.cloud?.relayUrl || 'https://saba-chan.online').replace(/\/$/, '');
            envVars.RELAY_URL = relayUrl;
            envVars.RELAY_NODE_TOKEN = nodeToken;
        } else {
            if (!config.token) {
                return { error: 'Discord token is required for local mode' };
            }
            envVars.DISCORD_TOKEN = config.token;
        }

        const startReq = {
            command: nodePath,
            args: [path.join(botDir, 'index.js')],
            cwd: botDir,
            env: envVars,
            meta: { mode: config.mode || 'local' },
        };

        const resp = await axios.post(`${IPC_BASE}/api/ext-process/discord-bot/start`, startReq, { timeout: 15000 });

        // 봇 시작 후 크래시 감지 — 3초 뒤 상태 확인하여 에러 피드백
        setTimeout(async () => {
            try {
                const statusResp = await axios.get(`${IPC_BASE}/api/ext-process/discord-bot/status`, { timeout: 3000 });
                if (statusResp.data?.status !== 'running') {
                    // 콘솔 버퍼에서 에러 메시지 추출
                    let errorMsg = 'Bot process exited unexpectedly';
                    try {
                        const consoleResp = await axios.get(`${IPC_BASE}/api/ext-process/discord-bot/console`, { timeout: 3000 });
                        const lines = consoleResp.data?.lines || consoleResp.data?.console || [];
                        const errorLines = lines
                            .map(l => typeof l === 'string' ? l : l?.line || '')
                            .filter(l => /\[Bot\].*(?:error|failed|fatal)/i.test(l) || /Error:/i.test(l))
                            .slice(-5);
                        if (errorLines.length > 0) errorMsg = errorLines.join('\n');
                    } catch (_) {}
                    if (mainWindow && !mainWindow.isDestroyed()) {
                        mainWindow.webContents.send('bot:error', { type: 'exit', message: errorMsg });
                    }
                }
            } catch (_) {}
        }, 3000);

        return resp.data;
    } catch (e) {
        const errData = e.response?.data;
        return { error: errData?.error || e.message };
    }
});

ipcMain.handle('discord:stop', async () => {
    try {
        const resp = await axios.post(`${IPC_BASE}/api/ext-process/discord-bot/stop`, {}, { timeout: 10000 });
        return resp.data;
    } catch (e) {
        return { error: e.response?.data?.error || e.message };
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

// 로그 파일 경로 반환
ipcMain.handle('logs:getPath', async () => {
    return logFilePath || '로그 파일 없음';
});

// 외부 브라우저로 URL 열기
ipcMain.handle('shell:openExternal', async (_event, url) => {
    if (!url || typeof url !== 'string') {
        return { error: 'URL이 지정되지 않았습니다' };
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
    try {
        // 기존 파일 내용을 읽어서 병합 — 부분 업데이트 시 token/autoStart 손실 방지
        let existing = {};
        try {
            existing = await loadBotConfig();
        } catch (_) { /* 데몬 미응답 — 빈 base 사용 */ }

        const configToSave = {
            ...existing,
            prefix: config.prefix !== undefined ? config.prefix : (existing.prefix || '!saba'),
            token: config.token !== undefined ? config.token : (existing.token || ''),
            autoStart: config.autoStart !== undefined ? config.autoStart : (existing.autoStart ?? false),
            moduleAliases: config.moduleAliases || existing.moduleAliases || {},
            commandAliases: config.commandAliases || existing.commandAliases || {},
            musicEnabled: config.musicEnabled !== undefined ? config.musicEnabled : (existing.musicEnabled !== false),
            musicChannelId: config.musicChannelId !== undefined ? config.musicChannelId : (existing.musicChannelId || ''),
            musicUISettings: config.musicUISettings || existing.musicUISettings || {},
            mode: config.mode || existing.mode || 'local',
            cloud: config.cloud || existing.cloud || {},
            nodeSettings: config.nodeSettings || existing.nodeSettings || {},
        };

        // AppData에 저장 (단일 원본 — 봇은 BOT_CONFIG_PATH 환경변수로 참조)
        const appDataConfig = {
            ...configToSave,
            cloudNodes: config.cloudNodes || [],
            cloudMembers: config.cloudMembers || {},
        };
        saveBotConfig(appDataConfig);
        console.log('Bot config saved via daemon API');

        // ★ 클라우드 모드: 릴레이 서버에 botConfig 동기화
        if (config.mode === 'cloud') {
            const relayUrl = config.cloud?.relayUrl;
            const hostId = config.cloud?.hostId;
            if (relayUrl && hostId) {
                try {
                    await axios.patch(`${relayUrl}/api/hosts/${hostId}/bot-config`, {
                        prefix: configToSave.prefix,
                        moduleAliases: configToSave.moduleAliases,
                        commandAliases: configToSave.commandAliases,
                    });
                    console.log('[BotConfig] Synced to relay server');
                } catch (relayErr) {
                    console.warn('[BotConfig] Failed to sync to relay:', relayErr.message);
                }
            }
        }

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
