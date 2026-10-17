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
const path = require('node:path');
const { pathToFileURL } = require('node:url');
const axios = require('axios');
const { spawn, execSync } = require('node:child_process');
const { StringDecoder } = require('node:string_decoder');
const fs = require('node:fs');
const { DEFAULT_IPC_PORT, DEFAULT_DAEMON_URL, SUPPORTED_LANGUAGES, getSabaDataDir } = require('../shared/constants');
const { buildBotConfigSavePayload } = require('../shared/botConfigPayload');

const IPC_PORT_DEFAULT = DEFAULT_IPC_PORT;
let IPC_BASE = process.env.IPC_BASE || DEFAULT_DAEMON_URL; // Core Daemon endpoint — updated from settings after app ready

// ── Windows UTF-8 콘솔 출력 강제 ──────────────────────────────
// 시스템 로캘이 비-UTF-8(CP932 등)이면 console.log 한글이 깨짐
if (process.platform === 'win32') {
    try { execSync('chcp 65001', { stdio: 'pipe' }); } catch {}
}

// ── 고정 경로: shared/constants.js SSOT ──
// Electron userData를 saba-chan으로 통일 — ready 전에 호출해야 Chromium 내부 데이터도 이 경로 사용
app.setPath('userData', getSabaDataDir());

function getFixedModulesPath() {
    return process.env.SABA_MODULES_PATH || path.join(getSabaDataDir(), 'modules');
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

// ── IPC 토큰 인증 ──────────────────────
// 데몬이 시작 시 생성하는 .ipc_token 파일을 읽어서 모든 데몬 요청의 X-Saba-Token 헤더로 포함
const _tokenPath = process.env.SABA_TOKEN_PATH
    || path.join(getSabaDataDir(), '.ipc_token');

let _cachedToken = '';

function loadToken() {
    try {
        const token = fs.readFileSync(_tokenPath, 'utf-8').trim();
        if (token) {
            const prev = _cachedToken;
            _cachedToken = token;
            if (prev !== token) {
                console.log(prev ? '[Auth] IPC token refreshed' : '[Auth] IPC token loaded');
            }
            return true;
        }
    } catch (err) {
        console.warn('[Auth] IPC token not found, auth may fail:', err.message);
    }
    return false;
}

function getToken() {
    if (!_cachedToken) loadToken();
    return _cachedToken;
}

function isDaemonRequest(url, baseURL = IPC_BASE) {
    try {
        return new URL(url, baseURL).origin === new URL(IPC_BASE).origin;
    } catch {
        return false;
    }
}

function setTokenHeader(config, token) {
    if (!config.headers) config.headers = {};
    if (typeof config.headers.set === 'function') {
        config.headers.set('X-Saba-Token', token);
    } else {
        config.headers['X-Saba-Token'] = token;
    }
}

// ── 데몬 전용 axios 인스턴스 (모든 데몬 호출의 단일 경로) ──
// baseURL은 요청마다 현재 IPC_BASE로 채운다 (설정에서 포트가 바뀔 수 있음).
// 리다이렉트를 따라가지 않으므로 토큰이 다른 출처로 넘어가지 않는다.
const daemon = axios.create({ timeout: 5000, maxRedirects: 0, adapter: 'http' });

daemon.interceptors.request.use((config) => {
    config.baseURL = IPC_BASE;
    if (!isDaemonRequest(config.url, IPC_BASE)) return config;
    const token = getToken();
    if (token) setTokenHeader(config, token);
    return config;
});

// ── 401 응답 시 토큰 재로드 후 1회 재시도 ──
// 데몬 재시작으로 토큰이 바뀐 경우 자동 복구. 동시에 받은 401은 한 번의 재로드를 공유한다.
// ★ 릴레이 프록시 요청(/api/relay/)의 401은 릴레이 서버 인증 실패이므로 제외
let _tokenRefreshPromise = null;
daemon.interceptors.response.use(
    (response) => response,
    async (error) => {
        const original = error.config;
        const retryable =
            error.response?.status === 401 &&
            original &&
            !original._retried &&
            isDaemonRequest(original.url, IPC_BASE) &&
            !(original.url || '').includes('/api/relay/');
        if (!retryable) throw error;

        original._retried = true;
        if (!_tokenRefreshPromise) {
            _tokenRefreshPromise = Promise.resolve(loadToken() ? _cachedToken : null);
            setTimeout(() => {
                _tokenRefreshPromise = null;
            }, 300);
        }
        const refreshed = await _tokenRefreshPromise;
        if (!refreshed) throw error;
        setTokenHeader(original, refreshed);
        return daemon.request(original);
    },
);

// 데몬 오류를 사용자에게 보여줄 문자열로 바꾼다 — 데몬이 준 메시지를 우선한다.
function describeDaemonError(error) {
    const data = error?.response?.data;
    const detail = data?.error || data?.message;
    if (detail) return String(detail);
    if (error?.code === 'ECONNREFUSED') return t('network.connection_refused');
    if (error?.response) return `HTTP ${error.response.status}`;
    return error?.message || t('error');
}

/**
 * 데몬 API 호출 공통 헬퍼.
 * 성공하면 응답 본문을, 실패하면 onError(error) 결과 또는 { error: 메시지 }를 반환한다.
 */
async function daemonCall(method, urlPath, { data, params, timeout, responseType, onError } = {}) {
    try {
        const response = await daemon.request({ method, url: urlPath, data, params, timeout, responseType });
        return response.data;
    } catch (error) {
        if (onError) return onError(error);
        return { error: describeDaemonError(error) };
    }
}

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
let isShuttingDown = false;

function initLogger() {
    const logsDir = path.join(app.getPath('userData'), 'logs');
    if (!fs.existsSync(logsDir)) {
        fs.mkdirSync(logsDir, { recursive: true });
    }

    const timestamp = new Date().toISOString().replace(/[:.]/g, '-').slice(0, -5);
    const logFilePath = path.join(logsDir, `saba-chan-${timestamp}.log`);

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
function sendStatus(step, message, percent) {
    if (mainWindow && !mainWindow.isDestroyed()) {
        mainWindow.webContents.send('status:update', {
            step,
            message,
            percent: typeof percent === 'number' ? percent : undefined,
            ts: Date.now(),
        });
    }
}

// 짧은 대기 헬퍼
function wait(ms) {
    return new Promise((resolve) => setTimeout(resolve, ms));
}

// Bot Config — 데몬 API 경유 (파일 직접 접근 금지)
// ⚠ 실패 시 기본값 객체를 만들어 반환하면 렌더러가 "완전한 설정을 로드했다"고
// 오인하고, 이후 저장 시 빈 hostId/별명으로 디스크 설정을 덮어쓴다. 실패는 null로 전달한다.
function loadBotConfig() {
    return daemonCall('get', '/api/config/bot', {
        onError: (error) => {
            console.warn('Failed to load bot config from daemon:', error.message);
            return null;
        },
    });
}

// 시스템 언어 가져오기
function getSystemLanguage() {
    try {
        const locale = app.getLocale(); // 예: 'en-US', 'ko-KR', 'ja-JP', 'zh-CN'
        if (SUPPORTED_LANGUAGES.includes(locale)) return locale;
        // 언어 코드만으로 매칭 (en-US → en, ko-KR → ko 등)
        const baseLang = locale.split('-')[0];
        return SUPPORTED_LANGUAGES.find((lang) => lang === baseLang || lang.startsWith(`${baseLang}-`)) || 'en';
    } catch (error) {
        console.error('Failed to get system language:', error);
        return 'en';
    }
}

function getLanguage() {
    return loadSettings().language || getSystemLanguage();
}

// ── Settings 관리 ──
// 데몬이 응답하면 데몬 API(GET/PUT /api/config/gui)가 단일 원천이다.
// 데몬에 연결할 수 없을 때(부트스트랩 등)만 로컬 settings.json을 직접 읽고 쓴다.
// 판정 기준은 "이 GUI가 데몬을 띄웠는가"가 아니라 "데몬에 연결되는가"이다.
function getSettingsPath() {
    return path.join(app.getPath('userData'), 'settings.json'); // %APPDATA%/saba-chan
}

function defaultSettings() {
    return {
        autoRefresh: true,
        refreshInterval: 2000,
        windowBounds: { width: 1200, height: 840 },
        language: getSystemLanguage(),
        ipcPort: IPC_PORT_DEFAULT,
        consoleBufferSize: 2000,
        portConflictCheck: true,
    };
}

function loadSettings() {
    const settingsPath = getSettingsPath();
    try {
        if (fs.existsSync(settingsPath)) {
            let data = fs.readFileSync(settingsPath, 'utf8');
            if (data.charCodeAt(0) === 0xfeff) data = data.slice(1); // UTF-8 BOM 제거
            // 기본값 병합 — 파일에 없는 필드만 채움 (기존 필드 보존)
            return { ...defaultSettings(), ...JSON.parse(data) };
        }
    } catch (error) {
        console.error('Failed to load settings:', error);
    }
    return defaultSettings();
}

function writeLocalSettings(next) {
    const settingsPath = getSettingsPath();
    fs.mkdirSync(path.dirname(settingsPath), { recursive: true });
    fs.writeFileSync(settingsPath, JSON.stringify(next, null, 2), 'utf8');
}

// 응답이 없는(연결 거부·타임아웃) 오류만 "데몬 없음"으로 본다.
function isDaemonUnreachable(error) {
    return !error?.response;
}

async function readSettings() {
    try {
        const response = await daemon.get('/api/config/gui');
        return { ...loadSettings(), ...(response.data || {}) };
    } catch (error) {
        if (!isDaemonUnreachable(error)) console.warn('[Settings] Daemon load failed:', error.message);
        return loadSettings();
    }
}

// 저장은 직렬화한다 — 창 크기 저장과 설정 저장의 GET-병합-PUT이 서로 덮어쓰지 않도록.
let _settingsWriteChain = Promise.resolve();

/** 설정 일부를 병합 저장한다. 성공하면 true. */
function saveSettingsPatch(patch) {
    const run = async () => {
        try {
            const response = await daemon.get('/api/config/gui');
            await daemon.put('/api/config/gui', { ...(response.data || {}), ...patch });
            return true;
        } catch (error) {
            if (!isDaemonUnreachable(error)) {
                console.error('[Settings] Daemon save failed:', error.message);
                return false;
            }
        }
        try {
            writeLocalSettings({ ...loadSettings(), ...patch });
            console.log('[Settings] Daemon unreachable — saved locally');
            return true;
        } catch (error) {
            console.error('[Settings] Local save failed:', error.message);
            return false;
        }
    };
    const result = _settingsWriteChain.then(run, run);
    _settingsWriteChain = result;
    return result;
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

    const ipcPort = (settings?.ipcPort) || IPC_PORT_DEFAULT;
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
    // StringDecoder로 UTF-8 멀티바이트 문자가 청크 경계에서 분할되는 문제 방지
    const stdoutDecoder = new StringDecoder('utf8');
    const stderrDecoder = new StringDecoder('utf8');

    if (daemonProcess.stdout) {
        daemonProcess.stdout.on('data', (data) => {
            const text = stdoutDecoder.write(data).trim();
            if (text) console.log('[Daemon]', text);
        });
    }

    if (daemonProcess.stderr) {
        daemonProcess.stderr.on('data', (data) => {
            // 데몬의 tracing 출력이 stderr로 옴 (라인 버퍼링 보장)
            const text = stderrDecoder.write(data).trim();
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
        // 디코더에 남은 불완전한 바이트 플러시
        const remainOut = stdoutDecoder.end().trim();
        const remainErr = stderrDecoder.end().trim();
        if (remainOut) console.log('[Daemon]', remainOut);
        if (remainErr) console.log('[Daemon]', remainErr);
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

// ── 종료 시퀀스 ──
function destroyTrayAndWindow() {
    if (tray) {
        tray.destroy();
        tray = null;
    }
    if (mainWindow) {
        // close 이벤트 가로채기 해제 후 파괴
        mainWindow.removeAllListeners('close');
        mainWindow.destroy();
        mainWindow = null;
    }
}

/**
 * 인터페이스만 종료 — GUI 프로세스를 완전히 종료하되 데몬은 그대로 유지한다.
 * 데몬에서 클라이언트 해제(shutdown=false)만 수행하고, 데몬 프로세스는 건드리지 않는다.
 */
async function interfaceOnlyQuit() {
    console.log('Starting interface-only quit sequence...');
    keepDaemonAlive = true;
    try {
        await unregisterFromDaemon({ shutdown: false });
        destroyTrayAndWindow();
        console.log('Interface-only quit sequence completed (daemon kept alive)');
        closeLogger();
    } catch (error) {
        console.error('Error during interface-only quit:', error);
    }
    app.quit();
}

/** 완전히 종료 — 데몬에 종료를 요청하고, 남아 있으면 강제 종료한 뒤 GUI를 닫는다. */
async function cleanQuit() {
    console.log('Starting clean quit sequence...');
    try {
        // 1. 클라이언트 해제 + 데몬 종료 요청 (shutdown=true)
        await unregisterFromDaemon({ shutdown: true });

        // 2. 이 GUI가 띄운 데몬 종료 후 최대 3초 대기
        stopDaemon();
        for (let attempts = 0; daemonProcess && !daemonProcess.killed && attempts < 6; attempts++) {
            await wait(500);
        }
        if (daemonProcess) {
            console.warn('Daemon still running after waiting, force killing');
            try {
                if (process.platform === 'win32') {
                    execSync(`taskkill /PID ${daemonProcess.pid} /F /T 2>nul`, { stdio: 'ignore' });
                } else {
                    daemonProcess.kill('SIGKILL');
                }
            } catch (e) {
                console.warn('Force kill error (process may already be dead):', e.message);
            }
        }
        daemonProcess = null;

        // 3. Fallback: 프로세스 이름으로 saba-core 종료 (외부에서 띄운 데몬 대비)
        try {
            if (process.platform === 'win32') {
                execSync('taskkill /IM saba-core.exe /F 2>nul', { stdio: 'ignore' });
            } else {
                execSync('pkill -f saba-core 2>/dev/null || true', { stdio: 'ignore' });
            }
        } catch (_e) {
            // 이미 종료됨 — 무시
        }

        destroyTrayAndWindow();
        console.log('Clean quit sequence completed');
        closeLogger();
    } catch (error) {
        console.error('Error during clean quit:', error);
    }
    app.quit();
}

// /health 응답 여부 (토큰 불필요)
async function isDaemonHealthy(timeout = 1000) {
    try {
        const response = await daemon.get('/health', { timeout });
        return response.status === 200;
    } catch {
        return false;
    }
}

// 데몬을 띄우고 /health가 응답할 때까지 최대 4초 대기한다. 데몬이 새 토큰을 만들므로 매번 재로드.
async function startDaemonAndWait() {
    startDaemon();
    for (let attempts = 0; attempts < 8; attempts++) {
        await wait(500);
        loadToken();
        if (await isDaemonHealthy(800)) return true;
    }
    return false;
}

// 이미 떠 있는 데몬이 있으면 재실행하지 않고 재사용
async function ensureDaemon() {
    loadToken();
    sendStatus('daemon', t('daemon.checking'));
    if (await isDaemonHealthy()) {
        console.log('Existing daemon detected on IPC port. Skipping launch.');
        sendStatus('daemon', t('daemon.existing_running'));
        await syncInstallRoot();
        return;
    }

    console.log('No daemon detected, attempting to launch new one...');
    sendStatus('daemon', t('daemon.starting'));
    try {
        if (await startDaemonAndWait()) {
            console.log('Daemon is now running');
            sendStatus('daemon', t('daemon.started'));
            await syncInstallRoot();
        } else {
            console.warn('Daemon did not respond after startup, but continuing...');
            sendStatus('daemon', t('daemon.preparing'));
        }
    } catch (daemonErr) {
        console.error('Failed to start daemon:', daemonErr);
        sendStatus('daemon', t('daemon.failed_to_start'));
    }
}

// 데몬에 install_root 동기화 (portable 모드에서 임시 폴더가 아닌 실제 배포 경로 전달)
async function syncInstallRoot() {
    const installRoot = getInstallRoot();
    try {
        await daemon.put('/api/updates/config', { install_root: installRoot }, { timeout: 3000 });
        console.log(`[InstallRoot] Synced to daemon: ${installRoot}`);
    } catch (e) {
        console.warn(`[InstallRoot] Failed to sync: ${e.message}`);
    }
}

// ★ 데몬의 runtime preflight 완료까지 폴링.
//   데몬은 부팅 시 포터블 Node.js·Python venv를 검사·복구한다. 각 단계 상태를
//   `/api/runtime/preflight`에서 읽어 LoadingScreen 진행 링에 그대로 전달한다.
//   완료 또는 5분 타임아웃까지 대기하며, 타임아웃이어도 GUI는 계속 진행한다.
async function waitForPreflight() {
    const POLL_INTERVAL_MS = 500;
    const MAX_WAIT_MS = 5 * 60 * 1000;
    const start = Date.now();
    let lastPhase = null;

    while (Date.now() - start < MAX_WAIT_MS) {
        try {
            const resp = await daemon.get('/api/runtime/preflight', { timeout: 1500 });
            const snap = resp.data || {};
            const phase = String(snap.phase || 'preflight-idle');
            const message = String(snap.message || '');
            if (phase !== lastPhase) {
                console.log(`[Preflight] ${phase}: ${message} (${snap.percent ?? 0}%)`);
                lastPhase = phase;
            }
            // preflight는 전체 부팅 진행률의 50%~65% 구간을 차지한다.
            const scaled = typeof snap.percent === 'number'
                ? Math.round(50 + Math.min(100, snap.percent) * 0.15)
                : undefined;
            sendStatus(phase, message, scaled);

            if (snap.complete === true) {
                console.log('[Preflight] complete');
                return;
            }
            if (phase === 'preflight-failed') {
                const err = snap.error || 'preflight failed';
                console.warn(`[Preflight] failed: ${err}`);
                sendStatus('preflight-failed', err);
                return;
            }
        } catch (_e) {
            // 데몬 응답 일시 오류 — 다음 폴링까지 대기.
        }
        await wait(POLL_INTERVAL_MS);
    }
    console.warn('[Preflight] timeout waiting for completion; continuing');
    sendStatus('preflight-failed', 'preflight timeout (5m)');
}

// ── Client Heartbeat (데몬이 GUI 생존 여부를 추적) ────────────
let heartbeatClientId = null;
let heartbeatTimer = null;

async function registerWithDaemon() {
    try {
        const res = await daemon.post('/api/client/register', { kind: 'gui' }, { timeout: 3000 });
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
            await daemon.post(`/api/client/${heartbeatClientId}/heartbeat`, {}, { timeout: 3000 });
        } catch (e) {
            // 데몬이 재시작되었을 수 있으므로 재등록 시도
            if (e.response?.status === 404 || e.code === 'ECONNREFUSED') {
                console.warn('[Heartbeat] Lost registration, re-registering...');
                await registerWithDaemon();
            }
        }
    }, 30000);
}

function stopHeartbeat() {
    if (heartbeatTimer) {
        clearInterval(heartbeatTimer);
        heartbeatTimer = null;
    }
}

async function unregisterFromDaemon({ shutdown = false, timeout = 2000 } = {}) {
    stopHeartbeat();
    if (!heartbeatClientId) return;
    const clientId = heartbeatClientId;
    heartbeatClientId = null;
    try {
        await daemon.delete(`/api/client/${clientId}/unregister`, {
            params: shutdown ? { shutdown: true } : undefined,
            timeout,
        });
        console.log(`[Heartbeat] Unregistered from daemon${shutdown ? ' (with shutdown request)' : ''}`);
    } catch (e) {
        console.warn('[Heartbeat] Failed to unregister:', e.message);
    }
}

async function runBackgroundInit() {
    sendStatus('init', 'Initializing');
    await ensureDaemon();
    updateTrayMenu();
    // 데몬은 응답하지만 내부 preflight(포터블 Node/Python)가 아직 진행 중일 수 있다.
    await waitForPreflight();

    await registerWithDaemon();
    startHeartbeat();
    startUpdateChecker(); // 기본 3시간 간격

    sendStatus('ready', 'Background initialization complete');
}

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
let updateCheckTimer = null;
// 마지막으로 OS 알림을 보낸 업데이트 목록의 fingerprint (중복 알림 방지)
let lastNotifiedUpdateKey = null;

function findAppIcon() {
    const candidates = [
        path.join(__dirname, 'build', 'icon.png'),
        path.join(__dirname, 'public', 'icon.png'),
        path.join(__dirname, '..', 'resources', 'icon.png'),
    ];
    return candidates.find((p) => fs.existsSync(p)) || undefined;
}

function showMainWindow() {
    if (mainWindow) {
        mainWindow.show();
        mainWindow.focus();
    }
}

/**
 * 업데이트 확인 응답에서 사용자에게 보일 항목만 고른다.
 * Locales는 백그라운드 자동 적용 대상이므로 알림에서 제외한다.
 * 렌더러로 보내는 `updates:available` 이벤트의 유일한 형태: { count, names, components }
 */
function visibleUpdates(data) {
    const names = (data?.update_names || []).filter((n) => n !== 'Locales');
    const components = (data?.components || []).filter((c) => c.display_name !== 'Locales');
    return { count: names.length, names, components };
}

function notifyRendererOfUpdates(updates) {
    if (updates.count > 0 && mainWindow && !mainWindow.isDestroyed()) {
        mainWindow.webContents.send('updates:available', updates);
    }
}

async function checkForUpdates() {
    try {
        const response = await daemon.post('/api/updates/check', {}, { timeout: 30000 });
        const data = response.data;
        if (!data.ok) {
            console.warn('[UpdateChecker] Check failed:', data.error);
            return;
        }

        const updates = visibleUpdates(data);
        if (updates.count === 0) {
            console.log('[UpdateChecker] No user-visible updates available');
            return;
        }
        console.log(`[UpdateChecker] ${updates.count} update(s) available: ${updates.names.join(', ')}`);

        // OS 네이티브 알림 (이전과 다른 업데이트 목록일 때만)
        const updateKey = [...updates.names].sort().join('\0');
        if (updateKey !== lastNotifiedUpdateKey && Notification.isSupported()) {
            const notif = new Notification({
                title: 'saba-chan — 업데이트 알림',
                body: `${updates.count}개 업데이트: ${updates.names.join(', ')}`,
                icon: findAppIcon(),
            });
            notif.on('click', showMainWindow);
            notif.show();
            lastNotifiedUpdateKey = updateKey;
        }

        // 렌더러에 알림 (자동 다운로드/적용은 하지 않음 — 사용자가 업데이트 센터에서 처리)
        notifyRendererOfUpdates(updates);
    } catch (e) {
        console.warn('[UpdateChecker] Check failed:', e.message);
    }
}

// config의 enabled 플래그가 false면 체크하지 않는다. 조회 실패 시 기본 동작(체크 실행).
async function startUpdateChecker() {
    try {
        const response = await daemon.get('/api/updates/config');
        const cfg = response.data?.config || response.data;
        if (cfg?.enabled === false) {
            console.log('[UpdateChecker] Auto-check disabled by config');
            return;
        }
    } catch (_) {
        // 기본 동작
    }
    runUpdateChecker();
}

function runUpdateChecker() {
    stopUpdateChecker();
    checkForUpdates();
    updateCheckTimer = setInterval(checkForUpdates, UPDATE_CHECK_INTERVAL_MS);
}

function stopUpdateChecker() {
    if (updateCheckTimer) {
        clearInterval(updateCheckTimer);
        updateCheckTimer = null;
    }
}

async function openExternalUrl(url) {
    try {
        const parsed = new URL(url);
        if (!['http:', 'https:'].includes(parsed.protocol)) {
            return { error: 'Unsupported URL protocol' };
        }
        await shell.openExternal(url);
        return { success: true };
    } catch (err) {
        return { error: err.message };
    }
}

function secureWindowNavigation(window) {
    const rendererUrls = [pathToFileURL(path.join(__dirname, 'build', 'index.html'))];
    if (!app.isPackaged && !process.argv.includes('--after-update')) {
        rendererUrls.push(new URL(process.env.ELECTRON_START_URL || 'http://localhost:5173'));
    }
    const preventExternalNavigation = (event, url) => {
        let allowed = false;
        try {
            const target = new URL(url);
            allowed = rendererUrls.some((renderer) => renderer.origin === target.origin && renderer.pathname === target.pathname);
        } catch {}
        if (!allowed) event.preventDefault();
    };
    window.webContents.on('will-navigate', preventExternalNavigation);
    window.webContents.on('will-redirect', preventExternalNavigation);
    window.webContents.setWindowOpenHandler(({ url }) => {
        openExternalUrl(url);
        return { action: 'deny' };
    });
}

// 렌더러 창 공통 webPreferences — preload는 contextBridge/ipcRenderer만 쓰므로 sandbox 가능
function rendererWebPreferences() {
    return {
        preload: path.join(__dirname, 'preload.js'),
        nodeIntegration: false,
        contextIsolation: true,
        sandbox: true,
    };
}

// 개발 모드: Vite 서버, 프로덕션 또는 --after-update 재기동: 빌드 파일
function loadRenderer(win, query = '') {
    const useDevServer = !app.isPackaged && !process.argv.includes('--after-update');
    const buildFile = path.join(__dirname, 'build', 'index.html');
    if (useDevServer) {
        const startURL = process.env.ELECTRON_START_URL || 'http://localhost:5173';
        return win.loadURL(`${startURL}${query ? `?${query}` : ''}`).catch((e) => {
            console.error(`[Window] loadURL failed: ${e.message} — falling back to build file`);
            return win.loadFile(buildFile, { search: query });
        });
    }
    return win.loadFile(buildFile, { search: query }).catch((e) => {
        console.error(`[Window] loadFile failed: ${e.message}`);
    });
}

// DevTools 단축키(F12, Ctrl/Cmd+Shift+I)는 개발 빌드에서만 허용한다.
// 프로덕션에서 열리면 콘솔에서 window.api로 데몬을 토큰 권한으로 호출할 수 있다.
function enableDevToolsShortcuts(win) {
    if (app.isPackaged) return;
    win.webContents.on('before-input-event', (_event, input) => {
        const ctrlShiftI = (input.control || input.meta) && input.shift && input.key === 'I';
        if (input.key === 'F12' || ctrlShiftI) win.webContents.toggleDevTools();
    });
}

const WINDOW_BOUNDS_SAVE_DELAY_MS = 500;

function createWindow() {
    const { width, height, x, y } = loadSettings().windowBounds || { width: 1200, height: 840 };

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
    secureWindowNavigation(mainWindow);

    // 창 크기/위치 변경 시 설정 저장 — 드래그 중 이벤트가 연달아 오므로 디바운스
    let boundsTimer = null;
    const saveWindowBounds = () => {
        clearTimeout(boundsTimer);
        boundsTimer = setTimeout(() => {
            if (!mainWindow || mainWindow.isDestroyed() || mainWindow.isMaximized() || mainWindow.isMinimized()) return;
            const { width, height, x, y } = mainWindow.getBounds();
            saveSettingsPatch({ windowBounds: { width, height, x, y } });
        }, WINDOW_BOUNDS_SAVE_DELAY_MS);
    };
    mainWindow.on('resized', saveWindowBounds);
    mainWindow.on('moved', saveWindowBounds);

    mainWindow.once('ready-to-show', () => {
        mainWindow.show();
    });

    // 윈도우 닫기 이벤트 가로채기 — 렌더러의 종료 다이얼로그로 확인한다.
    // 렌더러가 죽어 응답할 수 없으면 바로 완전 종료한다 (트레이 외에 닫을 방법이 없어지는 문제 방지).
    mainWindow.on('close', (e) => {
        e.preventDefault();
        if (mainWindow.webContents.isCrashed()) {
            cleanQuit();
            return;
        }
        mainWindow.webContents.send('app:closeRequest');
    });

    loadRenderer(mainWindow);
    if (!app.isPackaged && !process.argv.includes('--after-update')) {
        mainWindow.webContents.openDevTools();
    }
    enableDevToolsShortcuts(mainWindow);

    mainWindow.removeMenu();
}

// React에서 종료 선택 응답 처리 ('cancel'이면 아무것도 안 함)
ipcMain.on('app:closeResponse', (_event, choice) => {
    if (choice === 'exit-interface') {
        interfaceOnlyQuit();
    } else if (choice === 'quit') {
        // 종료 대기 동안 창이 남아 보이지 않도록 먼저 숨긴다
        if (mainWindow && !mainWindow.isDestroyed()) mainWindow.hide();
        cleanQuit();
    }
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
    if (mainWindow?.webContents) {
        mainWindow.webContents.once('did-finish-load', () => {
            sendStatus('ui', 'UI loaded');
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
    stopUpdateChecker();

    // 아직 등록이 남아 있으면 해제 요청 (응답은 기다리지 않음, 오류는 unregisterFromDaemon이 처리)
    unregisterFromDaemon({ timeout: 1000 });

    // 데몬 프로세스 종료 (인터페이스만 종료 시에는 데몬 유지)
    if (!keepDaemonAlive) {
        stopDaemon();
    }

    destroyTrayAndWindow();
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

// ═══════════════════════════════════════════════════════════════
// IPC handlers — 대부분 daemonCall()로 데몬 API를 그대로 중계한다.
// ═══════════════════════════════════════════════════════════════

const enc = encodeURIComponent;

// ── Servers ──────────────────────────────────────────────────
ipcMain.handle('server:list', () => daemonCall('get', '/api/servers'));

ipcMain.handle('server:start', (_event, name, options = {}) => {
    if (!options.module) {
        return { error: t('server.start_failed', { error: 'module is not set — check the instance settings' }) };
    }
    return daemonCall('post', `/api/server/${enc(name)}/start`, {
        data: { module: options.module, config: options.config || {} },
        timeout: 30000,
        onError: (error) => ({ error: t('server.start_failed', { error: describeDaemonError(error) }) }),
    });
});

ipcMain.handle('server:stop', (_event, name, options = {}) =>
    daemonCall('post', `/api/server/${enc(name)}/stop`, {
        data: options || {},
        timeout: 30000,
        onError: (error) => ({ error: t('server.stop_failed', { error: describeDaemonError(error) }) }),
    }),
);

ipcMain.handle('server:status', (_event, name) => daemonCall('get', `/api/server/${enc(name)}/status`));

// ── Module: Server Installation API ──────────────────────────
ipcMain.handle('module:listVersions', (_event, moduleName, options = {}) =>
    daemonCall('get', `/api/module/${enc(moduleName)}/versions`, {
        params: {
            include_snapshots: options.include_snapshots ? 'true' : undefined,
            page: options.page,
            per_page: options.per_page,
        },
        timeout: 15000,
    }),
);

// JAR 다운로드는 수십 MB — 최대 5분 허용
ipcMain.handle('module:installServer', (_event, moduleName, installConfig) =>
    daemonCall('post', `/api/module/${enc(moduleName)}/install`, { data: installConfig, timeout: 300000 }),
);

ipcMain.handle('instance:getInstalledVersion', (_event, instanceId) =>
    daemonCall('get', `/api/instance/${enc(instanceId)}/installed-version`, { timeout: 10000 }),
);

ipcMain.handle('instance:readLiveSettings', (_event, instanceId) =>
    daemonCall('get', `/api/instance/${enc(instanceId)}/settings/live`, { timeout: 10000 }),
);

ipcMain.handle('instance:resetServer', (_event, instanceId) =>
    daemonCall('post', `/api/instance/${enc(instanceId)}/server/reset`, { data: {}, timeout: 30000 }),
);

// ── Managed Process API (stdin/stdout capture) ───────────────
ipcMain.handle('managed:start', (_event, instanceId, options = {}) =>
    daemonCall('post', `/api/instance/${enc(instanceId)}/managed/start`, {
        data: { config: options.config || {} },
        timeout: 30000,
    }),
);

ipcMain.handle('managed:console', (_event, instanceId, since = 0, count = 200) =>
    daemonCall('get', `/api/instance/${enc(instanceId)}/console`, { params: { since, count } }),
);

ipcMain.handle('managed:stdin', (_event, instanceId, command) =>
    daemonCall('post', `/api/instance/${enc(instanceId)}/stdin`, { data: { command } }),
);

// ── Console Popout (PiP) Window ──────────────────────────────
const consolePopoutWindows = new Map(); // instanceId → BrowserWindow

function getPopout(instanceId) {
    const win = consolePopoutWindows.get(instanceId);
    if (win && !win.isDestroyed()) return win;
    consolePopoutWindows.delete(instanceId);
    return null;
}

function sendToMainWindow(channel, ...args) {
    if (mainWindow && !mainWindow.isDestroyed()) mainWindow.webContents.send(channel, ...args);
}

ipcMain.handle('console:popout', (_event, instanceId, serverName) => {
    const existing = getPopout(instanceId);
    if (existing) {
        existing.focus();
        return { ok: true, message: 'Focused existing window' };
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
    secureWindowNavigation(popout);
    enableDevToolsShortcuts(popout);
    popout.removeMenu();
    loadRenderer(popout, `console-popout=${enc(instanceId)}&name=${enc(serverName)}`);

    consolePopoutWindows.set(instanceId, popout);
    // 메인 윈도우에 팝아웃 열림/닫힘 알림 → 임베디드 콘솔 숨김 제어
    sendToMainWindow('console:popoutOpened', instanceId);
    popout.on('closed', () => {
        consolePopoutWindows.delete(instanceId);
        sendToMainWindow('console:popoutClosed', instanceId);
    });
    return { ok: true };
});

// 팝아웃 창 포커스 + 깜빡임으로 주의 환기
ipcMain.handle('console:focusPopout', (_event, instanceId) => {
    const win = getPopout(instanceId);
    if (!win) return { ok: false };
    if (win.isMinimized()) win.restore();
    win.focus();
    win.flashFrame(true);
    setTimeout(() => {
        if (!win.isDestroyed()) win.flashFrame(false);
    }, 2000);
    return { ok: true };
});

// PiP 콘솔을 메인 윈도우로 되돌리기 (popin)
ipcMain.handle('console:popin', (_event, instanceId) => {
    const win = getPopout(instanceId);
    if (!win) return { ok: false };
    const url = new URL(win.webContents.getURL());
    const serverName = url.searchParams.get('name') || win.getTitle().replace('Console — ', '');
    // 창을 닫기 전에 메인 윈도우에 인앱 콘솔 열기를 요청 (closed 이벤트에서 popoutClosed 전송)
    sendToMainWindow('console:popinRequest', instanceId, serverName);
    win.close();
    return { ok: true };
});

ipcMain.handle('console:toggleAlwaysOnTop', (_event, instanceId, pinned) => {
    const win = getPopout(instanceId);
    if (!win) return { ok: false };
    win.setAlwaysOnTop(pinned, 'floating');
    if (pinned) {
        // 재고정 시 윈도우를 최전면으로 끌어올리기
        if (win.isMinimized()) win.restore();
        win.moveTop();
        win.focus();
    }
    return { ok: true, pinned };
});

// ── Modules ──────────────────────────────────────────────────
ipcMain.handle('module:list', () => daemonCall('get', '/api/modules'));

ipcMain.handle('module:refresh', () => daemonCall('post', '/api/modules/refresh'));

const okFalse = (error) => ({ ok: false, error: describeDaemonError(error) });

ipcMain.handle('module:manifest', () =>
    daemonCall('get', '/api/modules/manifest', { timeout: 15000, onError: okFalse }),
);

ipcMain.handle('module:installFromManifest', (_event, moduleId) =>
    daemonCall('post', `/api/modules/manifest/${enc(moduleId)}/install`, {
        data: {},
        timeout: 120000,
        onError: okFalse,
    }),
);

ipcMain.handle('module:remove', (_event, moduleId) =>
    daemonCall('delete', `/api/modules/${enc(moduleId)}`, { timeout: 15000, onError: okFalse }),
);

// 모듈의 locale 파일들을 모두 읽어서 반환 — 실패 시 빈 객체
ipcMain.handle('module:getLocales', (_event, moduleName) =>
    daemonCall('get', `/api/module/${enc(moduleName)}/locales`, {
        onError: (error) => {
            console.error(`Failed to load locales for module ${moduleName}:`, error.message);
            return {};
        },
    }),
);

ipcMain.handle('module:getMetadata', (_event, moduleName) => daemonCall('get', `/api/module/${enc(moduleName)}`));

// ── Instances ────────────────────────────────────────────────
// 백엔드가 프로비저닝을 백그라운드로 처리하므로 짧은 타임아웃으로 충분
ipcMain.handle('instance:create', (_event, data) =>
    daemonCall('post', '/api/instances', { data, timeout: 30000 }),
);

ipcMain.handle('instance:provisionProgress', (_event, name) =>
    daemonCall('get', `/api/provision-progress/${enc(name)}`, { timeout: 3000, onError: () => ({ active: false }) }),
);

ipcMain.handle('instance:dismissProvision', (_event, name) =>
    daemonCall('delete', `/api/provision-progress/${enc(name)}`, {
        timeout: 3000,
        onError: () => ({ success: false }),
    }),
);

// SteamCMD app_info_print는 느릴 수 있다
ipcMain.handle('instance:checkUpdate', (_event, id) =>
    daemonCall('get', `/api/instance/${enc(id)}/check-update`, {
        timeout: 30000,
        onError: (error) => ({ update_available: false, error: describeDaemonError(error) }),
    }),
);

ipcMain.handle('instance:applyUpdate', (_event, id) =>
    daemonCall('post', `/api/instance/${enc(id)}/apply-update`, {
        data: {},
        timeout: 10000,
        onError: (error) => ({ success: false, error: describeDaemonError(error) }),
    }),
);

ipcMain.handle('instance:delete', (_event, id) => daemonCall('delete', `/api/instance/${enc(id)}`));

ipcMain.handle('instance:reorder', (_event, orderedIds) =>
    daemonCall('put', '/api/instances/reorder', { data: { order: orderedIds } }),
);

ipcMain.handle('instance:updateSettings', (_event, id, settings) =>
    daemonCall('patch', `/api/instance/${enc(id)}`, { data: settings }),
);

// 인스턴스 폴더 열기 — 렌더러가 준 경로가 아니라 데몬이 아는 인스턴스 작업 폴더만 연다.
// (임의 경로를 받으면 shell.openPath로 실행 파일을 띄울 수 있다)
ipcMain.handle('instance:openFolder', async (_event, instanceId) => {
    if (!instanceId || typeof instanceId !== 'string') return { error: 'Invalid instance id' };
    const instance = await daemonCall('get', `/api/instance/${enc(instanceId)}`);
    if (instance?.error) return instance;
    const dir = instance?.module_settings?.working_dir || instance?.working_dir;
    if (!dir || typeof dir !== 'string') return { error: 'Instance has no working directory' };
    try {
        if (!fs.statSync(dir).isDirectory()) return { error: 'Working directory is not a folder' };
        const result = await shell.openPath(dir);
        return result ? { error: result } : { success: true };
    } catch (err) {
        return { error: err.message };
    }
});

// ── 모듈-독립적 입력값 검증 헬퍼 ──────────────────────────────
// module.toml의 inputs 스키마에 따라 args를 검증하고 정규화합니다.
function buildValidatedBody(inputs, args, inlineMessage) {
    const body = {};
    for (const field of inputs || []) {
        const value = args?.[field.name];
        const label = field.label || field.name;
        const present = value !== undefined && value !== null && value !== '';

        if (field.required && !present) {
            throw new Error(`필수 필드 '${label}'이(가) 누락되었습니다`);
        }
        if (present) {
            if (field.type === 'number') {
                const numValue = Number(value);
                if (Number.isNaN(numValue)) throw new Error(`'${label}'은(는) 숫자여야 합니다`);
                body[field.name] = numValue;
            } else {
                body[field.name] = String(value);
            }
        } else if (field.default !== undefined) {
            body[field.name] = field.default;
        }
    }
    // 입력 스키마가 비어 있지만 사용자가 인라인으로 메시지를 입력한 경우
    if (inlineMessage && Object.keys(body).length === 0) {
        body.message = inlineMessage;
    }
    return body;
}

// commandMetadata(module.toml의 commands.fields 항목)의 method에 따라 데몬 엔드포인트를 고른다.
//   rcon → RCON 템플릿 치환 후 /rcon
//   rest → endpoint_template + http_method로 /rest (데몬이 rest_host/rest_port 참조)
//   dual → Python lifecycle 모듈이 프로토콜 선택 (/command)
//   없음 → 기본 /command (stdin 기반)
function buildCommandRequest(id, command) {
    // "announce hi"처럼 입력하면 첫 단어는 명령어, 나머지는 메시지
    const [cmdName, ...restParts] = (command.command || '').trim().split(/\s+/);
    const inlineMessage = restParts.join(' ');
    const cmdMeta = command.commandMetadata || null;
    const method = cmdMeta?.method || null;
    const args = command.args || {};
    const base = `/api/instance/${enc(id)}`;

    if (method === 'rcon') {
        const validatedArgs = buildValidatedBody(cmdMeta?.inputs, args, inlineMessage);
        let rconCmd = cmdMeta?.rcon_template || cmdName;
        for (const [key, value] of Object.entries(validatedArgs)) {
            if (value !== undefined && value !== null && value !== '') {
                rconCmd = rconCmd.replace(`{${key}}`, value);
            }
        }
        // 치환되지 않은 선택적 파라미터 제거
        rconCmd = rconCmd.replace(/\s*\{\w+\}/g, '').trim();
        return { url: `${base}/rcon`, data: { command: rconCmd }, timeout: 10000 };
    }
    if (method === 'rest') {
        return {
            url: `${base}/rest`,
            data: {
                endpoint: cmdMeta?.endpoint_template || `/v1/api/${cmdName}`,
                method: (cmdMeta?.http_method || 'GET').toUpperCase(),
                body: buildValidatedBody(cmdMeta?.inputs, args, inlineMessage),
                instance_id: id,
            },
            timeout: 10000,
        };
    }
    // dual/기본: /command (Python lifecycle은 subprocess 스폰 시간이 필요)
    const commandArgs = method === 'dual' ? buildValidatedBody(cmdMeta?.inputs, args, inlineMessage) : args;
    return { url: `${base}/command`, data: { command: cmdName, args: commandArgs, instance_id: id }, timeout: 30000 };
}

ipcMain.handle('instance:executeCommand', async (_event, id, command) => {
    let request;
    try {
        request = buildCommandRequest(id, command);
    } catch (validationError) {
        return { error: validationError.message };
    }
    console.log(`[Main] Executing command for instance ${id} via ${request.url}`);
    return daemonCall('post', request.url, { data: request.data, timeout: request.timeout });
});

// ── Extensions ───────────────────────────────────────────────
const extensionFailure = (error) => ({
    success: false,
    error: describeDaemonError(error),
    error_code: error.response?.data?.error_code || 'network',
});
// 활성화/비활성화 실패는 의존 관계(related)도 함께 전달한다
const toggleFailure = (error) => ({ ...extensionFailure(error), related: error.response?.data?.related || [] });
// 선택적 리소스(i18n·번들·스타일·아이콘): 실패하면 null (404는 리소스 없음이므로 조용히)
const optionalResource = (label) => (error) => {
    if (error.response?.status !== 404) console.warn(`[Extension] Failed to load ${label}:`, error.message);
    return null;
};

// 빈 목록을 반환하면 "익스텐션 없음"과 구분할 수 없어 UI가 재시도를 포기한다 → { error }
ipcMain.handle('extension:list', () => daemonCall('get', '/api/extensions'));

ipcMain.handle('extension:enable', (_event, extId) =>
    daemonCall('post', `/api/extensions/${enc(extId)}/enable`, {
        onError: toggleFailure,
    }),
);

ipcMain.handle('extension:disable', (_event, extId) =>
    daemonCall('post', `/api/extensions/${enc(extId)}/disable`, {
        onError: toggleFailure,
    }),
);

ipcMain.handle('extension:i18n', (_event, extId, locale) =>
    daemonCall('get', `/api/extensions/${enc(extId)}/i18n/${enc(locale)}`, {
        onError: optionalResource(`i18n for '${extId}' (${locale})`),
    }),
);

ipcMain.handle('extension:guiBundle', async (_event, extId) => {
    const data = await daemonCall('get', `/api/extensions/${enc(extId)}/gui`, {
        responseType: 'arraybuffer',
        onError: optionalResource(`GUI bundle for '${extId}'`),
    });
    return data ? Buffer.from(data).toString('utf-8') : null;
});

ipcMain.handle('extension:guiStyles', async (_event, extId) => {
    const data = await daemonCall('get', `/api/extensions/${enc(extId)}/gui/styles`, {
        onError: optionalResource(`GUI styles for '${extId}'`),
    });
    return typeof data === 'string' ? data : null;
});

// PNG → data:image/png;base64
ipcMain.handle('extension:icon', async (_event, extId) => {
    const data = await daemonCall('get', `/api/extensions/${enc(extId)}/icon`, {
        responseType: 'arraybuffer',
        onError: optionalResource(`icon for '${extId}'`),
    });
    return data ? `data:image/png;base64,${Buffer.from(data).toString('base64')}` : null;
});

ipcMain.handle('extension:fetchManifest', () =>
    daemonCall('get', '/api/extensions/manifest', {
        timeout: 15000,
        onError: (error) => ({ success: false, error: describeDaemonError(error), extensions: [], updates: [] }),
    }),
);

ipcMain.handle('extension:install', (_event, extId, opts = {}) =>
    daemonCall('post', `/api/extensions/${enc(extId)}/install`, {
        data: opts || {},
        timeout: 60000,
        onError: extensionFailure,
    }),
);

ipcMain.handle('extension:remove', (_event, extId) =>
    daemonCall('delete', `/api/extensions/${enc(extId)}`, {
        timeout: 15000,
        onError: extensionFailure,
    }),
);

ipcMain.handle('extension:checkUpdates', () =>
    daemonCall('get', '/api/extensions/updates', {
        timeout: 15000,
        onError: (error) => ({ success: false, error: describeDaemonError(error), updates: [], count: 0 }),
    }),
);

ipcMain.handle('extension:rescan', () =>
    daemonCall('post', '/api/extensions/rescan', {
        onError: (error) => ({ success: false, error: describeDaemonError(error), newly_found: [] }),
    }),
);

// 데몬 미연결 → initializing: false로 반환해 스피너가 무한히 돌지 않게 한다 (미연결은 로딩 스크린이 처리)
ipcMain.handle('extension:initStatus', () =>
    daemonCall('get', '/api/extensions/init-status', {
        onError: () => ({ initializing: false, in_progress: {}, completed: [], daemon_unreachable: true }),
    }),
);

// ── Relay server proxy (cloud mode) — daemon API 경유 ──
const relayCall = (method, urlPath, relayUrl, { data, params } = {}) =>
    daemonCall(method, urlPath, {
        data,
        params: { ...params, ...(relayUrl ? { relayUrl } : {}) },
        timeout: 8000,
    });

// 실패를 빈 배열로 바꾸면 "0개"와 구분할 수 없어 UI가 기존 목록을 지워버린다 → { error }
ipcMain.handle('relay:checkHostStatus', (_event, hostId, relayUrl) =>
    relayCall('get', `/api/relay/host/${enc(hostId)}/status`, relayUrl),
);
ipcMain.handle('relay:listHostNodes', (_event, hostId, relayUrl) =>
    relayCall('get', `/api/relay/host/${enc(hostId)}/nodes`, relayUrl),
);
ipcMain.handle('relay:listNodeMembers', (_event, guildId, relayUrl) =>
    relayCall('get', `/api/relay/node/${enc(guildId)}/members`, relayUrl),
);
ipcMain.handle('relay:getNodeSettings', (_event, guildId, relayUrl) =>
    relayCall('get', `/api/relay/node/${enc(guildId)}/settings`, relayUrl),
);
ipcMain.handle('relay:updateNodeSettings', (_event, guildId, settings, relayUrl) =>
    relayCall('patch', `/api/relay/node/${enc(guildId)}/settings`, relayUrl, { data: settings }),
);
ipcMain.handle('relay:initiatePairing', (_event, payload) =>
    relayCall('post', '/api/relay/pair/initiate', null, { data: payload }),
);
ipcMain.handle('relay:pollPairingStatus', (_event, code, secret, relayUrl) =>
    relayCall('get', `/api/relay/pair/${enc(code)}/status`, relayUrl, { params: secret ? { secret } : {} }),
);

// ── Updater (데몬 HTTP API) ──────────────────────────────────
const updaterCall = (method, urlPath, opts = {}) => daemonCall(method, urlPath, { ...opts, onError: okFalse });
const componentList = (components) => ({ components: Array.isArray(components) ? components : [] });

// 업데이트 확인 — 발견 시 렌더러에 updates:available 이벤트도 보낸다
ipcMain.handle('updater:check', async () => {
    const data = await updaterCall('post', '/api/updates/check', { data: {}, timeout: 30000 });
    if (data?.ok) notifyRendererOfUpdates(visibleUpdates(data));
    return data;
});

ipcMain.handle('updater:status', () => updaterCall('get', '/api/updates/status'));

// body: { components: ["module-minecraft", "saba-core"] } (비어있으면 전체)
ipcMain.handle('updater:download', (_event, components) =>
    updaterCall('post', '/api/updates/download', { data: componentList(components), timeout: 600000 }),
);

ipcMain.handle('updater:downloadProgress', () => updaterCall('get', '/api/updates/download/progress'));

// 업데이트 적용 — 모듈은 데몬이 직접 적용, 데몬/GUI/CLI는 needs_updater에 포함.
// launch_token은 main 프로세스에만 보관하고 렌더러에는 노출하지 않는다.
let _pendingLaunchToken = null;
ipcMain.handle('updater:apply', async (_event, components) => {
    const data = await updaterCall('post', '/api/updates/apply', { data: componentList(components), timeout: 120000 });
    if (data?.launch_token) {
        _pendingLaunchToken = data.launch_token;
        delete data.launch_token;
    }
    if (data?.applied?.length > 0) {
        sendToMainWindow('updates:completed', {
            count: data.applied.length,
            names: data.applied,
            requiresUpdater: !!data.requires_updater,
            needsUpdater: data.needs_updater || [],
        });
    }
    return data;
});

// 업데이터 exe 스폰 — 업데이터가 apply-targets.json에서 적용 대상을 읽고, 파일 교체 후 GUI를 재실행한다.
ipcMain.handle('updater:launchApply', () => {
    try {
        const updaterExe = findUpdaterExe();
        if (!updaterExe) return { ok: false, error: 'Updater exe not found' };

        // 데몬이 발급한 단일 사용 티켓 — 없으면 업데이터 실행 거부
        const token = _pendingLaunchToken;
        _pendingLaunchToken = null;
        if (!token) return { ok: false, error: 'Launch ticket not available — call updater:apply first' };

        const args = ['--apply', '--spawned-by', 'gui', '--launch-token', token];
        // 적용 완료 후 재실행할 GUI
        if (!app.isPackaged) {
            args.push('--relaunch', process.execPath, path.resolve(__dirname));
        } else {
            args.push('--relaunch', process.env.PORTABLE_EXECUTABLE_FILE || app.getPath('exe'));
        }

        console.log(`[Updater] Launching apply: ${updaterExe} (token hidden)`);
        spawnDetached(updaterExe, args);
        // 업데이터가 재실행을 담당하므로 GUI는 종료
        setTimeout(() => cleanQuit(), 500);
        return { ok: true };
    } catch (err) {
        return { ok: false, error: err.message };
    }
});

ipcMain.handle('updater:getConfig', () => updaterCall('get', '/api/updates/config'));

// 업데이트 설정 변경 — enabled 플래그에 따라 백그라운드 체커 시작/중지
ipcMain.handle('updater:setConfig', async (_event, config) => {
    const data = await updaterCall('put', '/api/updates/config', { data: config });
    if (!data?.error && typeof config?.enabled === 'boolean') {
        if (config.enabled) runUpdateChecker();
        else stopUpdateChecker();
    }
    return data;
});

// ── Daemon ───────────────────────────────────────────────────
// `/api/ping`은 인증을 거치므로 "데몬이 떠 있고 토큰이 유효한지"까지 확인한다.
// (`/health`는 listen만 되면 200이라 토큰 불일치를 놓친다)
ipcMain.handle('daemon:status', async () => {
    try {
        await daemon.get('/api/ping', { timeout: 1500 });
        return { running: true, message: 'Daemon is running' };
    } catch (err) {
        if (err.response?.status === 401) return { running: false, message: 'Daemon auth token mismatch' };
        return { running: false, message: `Daemon not responding: ${err.message}` };
    }
});

// 데몬 재시작 — 이 GUI가 띄운 데몬은 프로세스를 종료하고, 외부 데몬은 종료 API로 내린 뒤 새로 띄운다.
ipcMain.handle('daemon:restart', async () => {
    try {
        settings = loadSettings();
        refreshIpcBase(); // 포트 변경 반영
        if (daemonProcess && !daemonProcess.killed) {
            console.log('Stopping own daemon process...');
            stopDaemon();
            await wait(1000);
        } else if (await isDaemonHealthy()) {
            console.log('Requesting external daemon shutdown...');
            await daemonCall('post', '/api/daemon/shutdown', { onError: () => null });
            for (let i = 0; i < 10 && (await isDaemonHealthy(500)); i++) await wait(500);
            if (await isDaemonHealthy(500)) {
                return { success: false, error: 'The running daemon did not stop' };
            }
        }
        console.log('Starting daemon...');
        if (!(await startDaemonAndWait())) {
            return { success: false, error: 'Daemon did not respond after restart' };
        }
        return { success: true, message: 'Daemon restarted successfully' };
    } catch (err) {
        console.error('Failed to restart daemon:', err);
        return { success: false, error: err.message };
    }
});

// ── Settings ─────────────────────────────────────────────────
ipcMain.handle('settings:load', () => readSettings());

ipcMain.handle('settings:save', async (_event, patch) => {
    const ok = await saveSettingsPatch(patch || {});
    refreshIpcBase();
    return ok;
});

// ── App ──────────────────────────────────────────────────────
ipcMain.handle('app:getVersion', () => app.getVersion());

ipcMain.handle('app:getComponentInfo', () =>
    daemonCall('get', '/api/system/components', {
        onError: (e) => {
            console.warn('[App] Failed to get component info from daemon:', e.message);
            return { version: app.getVersion(), components: {}, lastUpdated: null };
        },
    }),
);

function findInstaller() {
    const installRoot = getInstallRoot();
    const names = ['saba-chan-installer.exe', 'Saba-chan Installer.exe'];
    const searchDirs = [
        installRoot,
        path.join(installRoot, 'saba-chan-gui'),
        path.join(installRoot, '..'),
        path.dirname(app.getPath('exe')),
    ];
    // Windows: 레지스트리의 InstallLocation도 확인
    if (process.platform === 'win32') {
        try {
            const regOutput = execSync(
                'reg query "HKCU\\Software\\Microsoft\\Windows\\CurrentVersion\\Uninstall\\Saba-chan" /v InstallLocation 2>nul',
                { encoding: 'utf8', windowsHide: true },
            );
            const match = regOutput.match(/InstallLocation\s+REG_SZ\s+(.+)/);
            if (match) searchDirs.push(match[1].trim());
        } catch (_) {
            /* 레지스트리 접근 실패 — 무시 */
        }
    }
    for (const dir of searchDirs) {
        for (const name of names) {
            const candidate = path.join(dir, name);
            if (fs.existsSync(candidate)) return candidate;
        }
    }
    return null;
}

// Uninstall — 인스톨러를 --uninstall로 실행한 뒤 GUI/데몬 종료
ipcMain.handle('app:launchUninstaller', () => {
    try {
        const installerPath = findInstaller();
        if (!installerPath) return { success: false, error: 'Installer not found' };
        console.log('[Uninstall] Launching installer:', installerPath);
        spawn(installerPath, ['--uninstall'], { detached: true, stdio: 'ignore' }).unref();
        setTimeout(() => cleanQuit(), 500);
        return { success: true };
    } catch (err) {
        console.error('[Uninstall] Failed to launch installer:', err);
        return { success: false, error: err.message };
    }
});

// ── Language ─────────────────────────────────────────────────
ipcMain.handle('language:get', () => getLanguage());

// 데몬은 재시작하지 않는다 — Python 모듈은 호출 시 환경변수로 언어를 결정하고,
// 재시작하면 실행 중인 서버가 모두 종료된다.
ipcMain.handle('language:set', async (_event, language) => {
    const success = await saveSettingsPatch({ language });
    translations = loadTranslations();
    return { success, language };
});

// ── Dialogs ──────────────────────────────────────────────────
ipcMain.handle('dialog:openFile', async (_event, options) => {
    const platformFilters = {
        win32: [{ name: 'Executable Files', extensions: ['exe'] }],
        darwin: [{ name: 'Applications', extensions: ['app'] }],
    };
    const defaultFilters = [...(platformFilters[process.platform] || []), { name: 'All Files', extensions: ['*'] }];
    const result = await dialog.showOpenDialog({
        properties: ['openFile'],
        filters: options?.filters || defaultFilters,
    });
    return result.canceled ? null : result.filePaths[0];
});

ipcMain.handle('dialog:openFolder', async () => {
    const result = await dialog.showOpenDialog({ properties: ['openDirectory'] });
    return result.canceled ? null : result.filePaths[0];
});

// ── Migration: 디렉토리 스캔 (파일 목록 반환) ──
ipcMain.handle('migration:scanDir', (_event, dirPath) => {
    if (!dirPath || typeof dirPath !== 'string') return { error: 'Invalid directory path' };
    return daemonCall('post', '/api/fs/scan-dir', { data: { path: dirPath } });
});

// ── Bot Config (릴레이 봇 설정) — 데몬 API 경유 ──
// 봇은 릴레이 서버(server-chan)가 호스팅한다. GUI는 bot-config만 데몬에 저장하고,
// 릴레이로의 동기화는 데몬이 병합된 설정으로 직접 push한다 (GUI는 릴레이에 PATCH하지 않는다).
ipcMain.handle('botConfig:load', () => loadBotConfig());

ipcMain.handle('botConfig:save', async (_event, config) => {
    // 부분 업데이트로 hostId·별명 등이 사라지지 않도록 디스크 값과 병합한다 (병합은 여기 한 곳에서만).
    const existing = await loadBotConfig();
    if (!existing) return { error: 'Bot config is unavailable; refusing to overwrite it with partial data' };
    const payload = buildBotConfigSavePayload(config, existing);
    const result = await daemonCall('put', '/api/config/bot', { data: payload });
    if (result?.error) {
        console.error('Failed to save bot config via daemon:', result.error);
        return { error: result.error };
    }
    console.log('Bot config saved via daemon API');
    return { success: true, message: 'Bot config saved' };
});

// ── Node Token (클라우드 페어링용) — 데몬 API 경유 ──
ipcMain.handle('nodeToken:save', async (_event, token) => {
    const result = await daemonCall('put', '/api/config/node-token', { data: { token } });
    return !result?.error;
});

ipcMain.handle('nodeToken:delete', async () => {
    const result = await daemonCall('delete', '/api/config/node-token');
    return !result?.error;
});

// ── Shell ────────────────────────────────────────────────────
ipcMain.handle('shell:openExternal', (_event, url) => {
    if (!url || typeof url !== 'string') return { error: 'URL is required' };
    return openExternalUrl(url);
});

// ── Window Controls (Title Bar) ──────────────────────────────
// event.sender로 요청을 보낸 창을 찾아 조작한다 (메인 윈도우·콘솔 팝아웃 공통)
function senderWindow(event) {
    const win = BrowserWindow.fromWebContents(event.sender);
    return win && !win.isDestroyed() ? win : null;
}

ipcMain.on('window:minimize', (event) => senderWindow(event)?.minimize());

ipcMain.on('window:maximize', (event) => {
    const win = senderWindow(event);
    if (!win) return;
    if (win.isMaximized()) win.restore();
    else win.maximize();
});

ipcMain.on('window:close', (event) => senderWindow(event)?.close());
