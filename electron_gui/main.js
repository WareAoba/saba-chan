const { app, BrowserWindow, Menu, ipcMain, Tray, nativeImage } = require('electron');
const { dialog } = require('electron');
const path = require('path');
const axios = require('axios');
const { spawn } = require('child_process');
const fs = require('fs');

const IPC_BASE = process.env.IPC_BASE || 'http://127.0.0.1:57474'; // Core Daemon endpoint

// 네트워크 호출 기본 타임아웃을 짧게 설정해 초기 체감 지연을 줄입니다.
axios.defaults.timeout = 1200;

let mainWindow;
let daemonProcess = null;
let daemonStartedByApp = false;
let tray = null;
let translations = {}; // 번역 객체 캐시

// 번역 파일 로드 (메인 프로세스용)
function loadTranslations() {
    const lang = getLanguage();
    const commonPath = path.join(__dirname, '..', 'locales', lang, 'common.json');
    try {
        if (fs.existsSync(commonPath)) {
            return JSON.parse(fs.readFileSync(commonPath, 'utf8'));
        }
    } catch (error) {
        console.error('Failed to load translations:', error);
    }
    // Fallback to English
    const fallbackPath = path.join(__dirname, '..', 'locales', 'en', 'common.json');
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

// Bot Config 경로 (AppData에 저장)
function getBotConfigPath() {
    const userDataPath = app.getPath('userData');
    return path.join(userDataPath, 'bot-config.json');
}

function loadBotConfig() {
    const configPath = getBotConfigPath();
    try {
        if (fs.existsSync(configPath)) {
            const data = fs.readFileSync(configPath, 'utf8');
            const parsed = JSON.parse(data);
            console.log('Bot config loaded from:', configPath);
            return parsed;
        }
    } catch (error) {
        console.error('Failed to load bot config:', error);
    }
    return { prefix: '!saba', moduleAliases: {}, commandAliases: {} };
}

function saveBotConfig(config) {
    const configPath = getBotConfigPath();
    try {
        const dir = path.dirname(configPath);
        if (!fs.existsSync(dir)) {
            fs.mkdirSync(dir, { recursive: true });
        }
        fs.writeFileSync(configPath, JSON.stringify(config, null, 2), 'utf8');
        console.log('Bot config saved to:', configPath);
        return true;
    } catch (error) {
        console.error('Failed to save bot config:', error);
        return false;
    }
}

// 시스템 언어 가져오기
function getSystemLanguage() {
    try {
        const locale = app.getLocale(); // 예: 'en-US', 'ko-KR', 'ja-JP', 'zh-CN'
        const language = locale.split('-')[0]; // 언어 코드만 추출 (en, ko, ja, etc)
        
        // 지원하는 언어인지 확인 (en, ko, ja만 지원)
        if (['en', 'ko', 'ja'].includes(language)) {
            return language;
        }
        
        // 지원하지 않는 언어면 영어로 기본 설정
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
    const userDataPath = app.getPath('userData'); // %APPDATA%/game-server-gui
    return path.join(userDataPath, 'settings.json');
}

function loadSettings() {
    try {
        const settingsPath = getSettingsPath();
        if (fs.existsSync(settingsPath)) {
            const data = fs.readFileSync(settingsPath, 'utf8');
            return JSON.parse(data);
        }
    } catch (error) {
        console.error('Failed to load settings:', error);
    }
    // 기본 설정 (시스템 언어로 초기화)
    const systemLanguage = getSystemLanguage();
    return {
        modulesPath: path.join(__dirname, '..', 'modules'),
        autoRefresh: true,
        refreshInterval: 2000,
        windowBounds: { width: 1200, height: 800 },
        language: systemLanguage
    };
}

function saveSettings(settings) {
    try {
        const settingsPath = getSettingsPath();
        const dir = path.dirname(settingsPath);
        if (!fs.existsSync(dir)) {
            fs.mkdirSync(dir, { recursive: true });
        }
        fs.writeFileSync(settingsPath, JSON.stringify(settings, null, 2), 'utf8');
        console.log('Settings saved to:', settingsPath);
        return true;
    } catch (error) {
        console.error('Failed to save settings:', error);
        return false;
    }
}

// Core Daemon 시작
function startDaemon() {
    // Electron 포터블 exe 내에서는 bin 폴더에 binary 포함
    const isDev = !app.isPackaged;
    let daemonPath;
    let projectRoot;
    
    // 플랫폼별 실행 파일 이름
    const daemonFileName = process.platform === 'win32' ? 'core_daemon.exe' : 'core_daemon';
    
    if (isDev) {
        // 개발 환경: electron_gui/bin 폴더
        daemonPath = path.join(__dirname, 'bin', daemonFileName);
        projectRoot = path.join(__dirname, '..');
    } else {
        // 패키징된 앱: win-unpacked/bin 폴더
        const appDir = path.dirname(app.getPath('exe'));
        daemonPath = path.join(appDir, 'bin', daemonFileName);
        projectRoot = path.join(appDir, 'resources');  // resources 폴더 (modules 폴더가 여기 있음)
    }
    
    console.log('Starting Core Daemon:', daemonPath);
    console.log('Is Packaged:', !isDev);
    console.log('Project Root:', projectRoot);
    
    if (!fs.existsSync(daemonPath)) {
        console.error('Core Daemon executable not found at:', daemonPath);
        return;
    }
    
    // 언어 설정 가져오기
    const currentLanguage = getLanguage();
    console.log(`Starting daemon with language: ${currentLanguage}`);
    
    daemonProcess = spawn(daemonPath, [], {
        cwd: projectRoot,  // 프로젝트 루트에서 실행하여 "./modules" 경로가 올바르게 작동
        env: { 
            ...process.env, 
            RUST_LOG: 'info',
            SABA_LANG: currentLanguage  // Python 모듈에 언어 설정 전달
        },
        stdio: ['ignore', 'pipe', 'pipe'],
        detached: false  // Electron 프로세스에 연결되어 있으므로 자동으로 종료됨
    });
    daemonStartedByApp = true;
    
    console.log(`Daemon process spawned with PID: ${daemonProcess.pid}`);
    
    // stdout/stderr 이벤트 핸들 (stdio가 'pipe'가 아니면 건너뜀)
    if (daemonProcess.stdout) {
        daemonProcess.stdout.on('data', (data) => {
            console.log('[Daemon]', data.toString().trim());
        });
    }
    
    if (daemonProcess.stderr) {
        daemonProcess.stderr.on('data', (data) => {
            console.error('[Daemon Error]', data.toString().trim());
        });
    }
    
    daemonProcess.on('error', (err) => {
        console.error('Failed to start Core Daemon:', err);
        daemonProcess = null;
        daemonStartedByApp = false;
    });
    
    daemonProcess.on('exit', (code, signal) => {
        console.log(`Core Daemon exited with code ${code}, signal ${signal}`);
        daemonProcess = null;
        daemonStartedByApp = false;
        
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
                    require('child_process').execSync(`taskkill /PID ${daemonProcess.pid} /F /T`, { stdio: 'ignore' });
                    console.log('Daemon terminated via taskkill');
                } catch (e) {
                    console.warn('taskkill failed, trying process.kill:', e.message);
                    daemonProcess.kill('SIGTERM');
                }
            } else {
                // Unix/Linux/macOS: SIGTERM으로 우아하게 종료 시도
                daemonProcess.kill('SIGTERM');
                console.log('Sent SIGTERM to daemon');
                
                // 2초 후에도 살아있으면 SIGKILL
                const killTimeout = setTimeout(() => {
                    if (daemonProcess && !daemonProcess.killed) {
                        console.warn('SIGTERM timeout, sending SIGKILL');
                        try {
                            daemonProcess.kill('SIGKILL');
                        } catch (e) {
                            console.error('SIGKILL failed:', e);
                        }
                    }
                }, 2000);
                
                daemonProcess.once('exit', () => {
                    clearTimeout(killTimeout);
                });
            }
        }
        
        // 프로세스 참조 제거
        daemonProcess = null;
        daemonStartedByApp = false;
        console.log('Daemon stopped');
        
    } catch (error) {
        console.error('Error stopping daemon:', error);
        daemonProcess = null;
    }
}

// 안전한 종료 함수
async function cleanQuit() {
    console.log('Starting clean quit sequence...');
    
    try {
        // 1. 데몬 종료
        stopDaemon();
        
        // 2. 데몬이 종료될 때까지 대기 (최대 3초)
        let attempts = 0;
        while (daemonProcess && !daemonProcess.killed && attempts < 6) {
            await new Promise(resolve => setTimeout(resolve, 500));
            attempts++;
        }
        
        if (daemonProcess) {
            console.warn('Daemon still running after waiting, force killing');
            try {
                if (process.platform === 'win32') {
                    // Windows: taskkill로 강제 종료
                    require('child_process').execSync(`taskkill /PID ${daemonProcess.pid} /F /T 2>nul`, { stdio: 'ignore' });
                } else {
                    // Unix/Linux/macOS: SIGKILL로 강제 종료
                    daemonProcess.kill('SIGKILL');
                }
            } catch (e) {
                console.debug('Force kill error (process may already be dead):', e.message);
            }
        }
        
        daemonProcess = null;
        
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
        app.quit();
        
    } catch (error) {
        console.error('Error during clean quit:', error);
        app.quit();
    }
}

// 이미 떠 있는 데몬이 있으면 재실행하지 않고 재사용
async function ensureDaemon() {
    try {
        // 여러 엔드포인트로 체크 (일부 엔드포인트가 500을 반환해도 데몬은 실행 중)
        sendStatus('daemon', t('daemon.checking'));
        const response = await axios.get(`${IPC_BASE}/api/modules`, { timeout: 1000 });
        if (response.status === 200) {
            console.log('Existing daemon detected on IPC port. Skipping launch.');
            daemonStartedByApp = false;
            sendStatus('daemon', t('daemon.existing_running'));
            return;
        }
    } catch (err) {
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
                    try {
                        const checkResponse = await axios.get(`${IPC_BASE}/api/modules`, { timeout: 800 });
                        if (checkResponse.status === 200) {
                            console.log('✓ Daemon is now running');
                            sendStatus('daemon', t('daemon.started'));
                            return;
                        }
                    } catch (checkErr) {
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

async function preloadLightData() {
    const tasks = [
        axios
            .get(`${IPC_BASE}/api/modules`, { timeout: 1200 })
            .then(() => sendStatus('modules', '모듈 목록 준비 완료'))
            .catch((err) => sendStatus('modules', `모듈 로드 실패: ${err.message}`)),
        axios
            .get(`${IPC_BASE}/api/instances`, { timeout: 1200 })
            .then(() => sendStatus('instances', '인스턴스 목록 준비 완료'))
            .catch((err) => sendStatus('instances', `인스턴스 로드 실패: ${err.message}`)),
    ];

    await Promise.allSettled(tasks);
}

async function runBackgroundInit() {
    sendStatus('init', '초기화 시작');
    await ensureDaemon();
    updateTrayMenu();
    await preloadLightData();
    sendStatus('ready', '백그라운드 초기화 완료');
    // Discord Bot 자동 시작은 React App.js에서 처리
}

// runDeferredTasks 제거됨 - Discord Bot 자동 시작은 React에서 처리

function createWindow() {
    const settings = loadSettings();
    const { width, height } = settings.windowBounds || { width: 1200, height: 800 };
    
    mainWindow = new BrowserWindow({
        width,
        height,
        minWidth: 400,
        minHeight: 500,
        show: false,  // 준비될 때까지 보이지 않음
        frame: false,  // Windows 기본 프레임 제거
        icon: path.join(__dirname, '..', 'assets', 'icon.png'),  // 아이콘 (있으면)
        webPreferences: {
            preload: path.join(__dirname, 'preload.js'),
            nodeIntegration: false,
            contextIsolation: true
        }
    });

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
    const isDev = !app.isPackaged;
    if (isDev) {
        const startURL = process.env.ELECTRON_START_URL || 'http://localhost:5173';
        mainWindow.loadURL(startURL);
        // 개발 모드에서 DevTools 자동 열기
        mainWindow.webContents.openDevTools();
    } else {
        // 프로덕션: 빌드된 파일 로드
        mainWindow.loadFile(path.join(__dirname, 'build', 'index.html'));
    }
    
    // 메뉴바 제거
    mainWindow.removeMenu();
}

// React에서 종료 선택 응답 처리
ipcMain.on('app:closeResponse', (event, choice) => {
    if (choice === 'hide') {
        // GUI만 닫기 - 트레이로 최소화
        mainWindow.hide();
    } else if (choice === 'quit') {
        // 완전히 종료 - cleanQuit 사용
        mainWindow.removeAllListeners('close'); // close 이벤트 리스너 제거
        mainWindow.close();
        cleanQuit();
    }
    // choice === 'cancel'이면 아무것도 안 함
});

// 시스템 트레이 아이콘 생성
function createTray() {
    // 16x16 간단한 아이콘 (Base64 PNG - 보라색 원)
    const iconBase64 = 'iVBORw0KGgoAAAANSUhEUgAAABAAAAAQCAYAAAAf8/9hAAAABHNCSVQICAgIfAhkiAAAAAlwSFlzAAAAbwAAAG8B8aLcQwAAABl0RVh0U29mdHdhcmUAd3d3Lmlua3NjYXBlLm9yZ5vuPBoAAADfSURBVDiNpZMxDoJAEEV/kNCQWFhYGBIbO2s7j+ARPISdnYfwCHR2djYewMZKEgsLC0NCwiIFMbCwy7rJJJPM7sz/M7MLLEOSJMBERIZABziIyNlaq2+FkiQxwAH4AEPgDZRKqWdTb0VpXQdWQBd4A3MRecRxfGzuGGPKQB+YAgtgKCIDoK61fob+EeBpre/AB1gDU2AlIoM4jk91j8YYA/SAGbAE+iIyAspa62uLwD+11legDWyBhYhMgI7W+tIikOc5EzCZpum9kOD/gZzNs+xQJPC3oSAILl+nEbD5AYoJdEnfF3TzAAAAAElFTkSuQmCC';
    
    const icon = nativeImage.createFromDataURL(`data:image/png;base64,${iconBase64}`);
    tray = new Tray(icon);
    
    const contextMenu = Menu.buildFromTemplate([
        {
            label: '🖥️ 창 열기',
            click: () => {
                if (mainWindow) {
                    mainWindow.show();
                    mainWindow.focus();
                }
            }
        },
        { type: 'separator' },
        {
            label: daemonProcess ? '🟢 데몬 실행 중' : '⚪ 데몬 중지됨',
            enabled: false
        },
        {
            label: '🛑 데몬 종료',
            click: () => {
                stopDaemon();
                updateTrayMenu();
            }
        },
        {
            label: '▶️ 데몬 시작',
            click: () => {
                startDaemon();
                updateTrayMenu();
            }
        },
        { type: 'separator' },
        {
            label: '❌ 완전히 종료',
            click: () => {
                cleanQuit();
            }
        }
    ]);
    
    tray.setToolTip('사바쨩 - 게임 서버 관리');
    tray.setContextMenu(contextMenu);
    
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
            }
        },
        { type: 'separator' },
        {
            label: daemonProcess ? '🟢 데몬 실행 중' : '⚪ 데몬 중지됨',
            enabled: false
        },
        {
            label: '🛑 데몬 종료',
            enabled: !!daemonProcess,
            click: () => {
                stopDaemon();
                updateTrayMenu();
            }
        },
        {
            label: '▶️ 데몬 시작',
            enabled: !daemonProcess,
            click: () => {
                startDaemon();
                setTimeout(updateTrayMenu, 1000);
            }
        },
        { type: 'separator' },
        {
            label: '❌ 완전히 종료',
            click: () => {
                cleanQuit();
            }
        }
    ]);
    
    tray.setContextMenu(contextMenu);
}

app.on('ready', () => {
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
        });
    }
});

app.on('window-all-closed', () => {
    // 창이 닫혀도 트레이에서 계속 실행
    // macOS가 아니면 앱을 완전히 종료하지 않음
    if (process.platform === 'darwin') {
        // macOS에서는 기본 동작 유지
    }
    // Windows/Linux에서는 트레이에 남아있음
});

app.on('before-quit', () => {
    console.log('App is quitting, cleaning up...');
    
    // 데몬 프로세스 종료
    stopDaemon();
    
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
        } catch (e) {
            // 무시
        }
    }
});

// IPC handlers
ipcMain.handle('server:list', async () => {
    try {
        const response = await axios.get(`${IPC_BASE}/api/servers`);
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

ipcMain.handle('server:start', async (event, name, options = {}) => {
    try {
        const body = {
            module: options.module || 'minecraft',
            config: options.config || {}
        };
        const response = await axios.post(`${IPC_BASE}/api/server/${name}/start`, body);
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

ipcMain.handle('server:stop', async (event, name, options = {}) => {
    try {
        const body = options || {};
        const response = await axios.post(`${IPC_BASE}/api/server/${name}/stop`, body);
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

ipcMain.handle('server:status', async (event, name) => {
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

ipcMain.handle('module:getMetadata', async (event, moduleName) => {
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

ipcMain.handle('instance:create', async (event, data) => {
    try {
        const response = await axios.post(`${IPC_BASE}/api/instances`, data);
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

ipcMain.handle('instance:delete', async (event, id) => {
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

ipcMain.handle('instance:updateSettings', async (event, id, settings) => {
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

ipcMain.handle('instance:executeCommand', async (event, id, command) => {
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
        console.log(`[Main] Instance data:`, {
            module: instance.module_name,
            rcon_port: instance.rcon_port,
            rcon_password: instance.rcon_password,
            rest_host: instance.rest_host,
            rest_port: instance.rest_port
        });
        
        // Step 2: 모듈에 따라 적절한 프로토콜 선택
        let protocolUrl;
        let commandPayload;
        
        if (instance.module_name === 'minecraft') {
            // Minecraft는 RCON 사용 (권장)
            console.log(`[Main] Using RCON protocol for Minecraft`);
            protocolUrl = `${IPC_BASE}/api/instance/${id}/rcon`;
            commandPayload = {
                command: cmdName,
                args: command.args || {},
                instance_id: id,
                rcon_port: instance.rcon_port,
                rcon_password: instance.rcon_password
            };
        } else if (instance.module_name === 'palworld') {
            // Palworld 명령어 처리
            console.log(`[Main] Processing Palworld command: ${cmdName}`);
            
            // kick, ban, unban은 플레이어 ID 변환이 필요하므로 Python 모듈을 통해 실행
            const playerCommands = ['kick', 'ban', 'unban'];
            if (playerCommands.includes(cmdName.toLowerCase())) {
                console.log(`[Main] Using command endpoint for player command: ${cmdName}`);
                protocolUrl = `${IPC_BASE}/api/instance/${id}/command`;
                commandPayload = {
                    command: cmdName,
                    args: command.args || {},
                    instance_id: id
                };
            } else {
                // 그 외 명령어는 REST API 직접 호출
                console.log(`[Main] Using REST API protocol for Palworld`);
                protocolUrl = `${IPC_BASE}/api/instance/${id}/rest`;
                
                // 명령 메타데이터에서 http_method와 입력 스키마 읽기
                const httpMethod = command.commandMetadata?.http_method || 'POST';
                const inputSchema = command.commandMetadata?.inputs || [];
                
                console.log(`[Main] HTTP Method from metadata: ${httpMethod}`);
                console.log(`[Main] Input schema:`, inputSchema);
                
                // 입력값 검증 및 정규화
                const validatedBody = {};
                for (const field of inputSchema) {
                    const value = command.args?.[field.name];
                    
                    // 필수 필드 확인
                    if (field.required && (value === undefined || value === null || value === '')) {
                        throw new Error(`필수 필드 '${field.label}'이(가) 누락되었습니다`);
                    }
                    
                    // 값이 있으면 타입 검증 및 추가
                    if (value !== undefined && value !== null && value !== '') {
                        if (field.type === 'number') {
                            const numValue = Number(value);
                            if (isNaN(numValue)) {
                                throw new Error(`'${field.label}'은(는) 숫자여야 합니다`);
                            }
                            validatedBody[field.name] = numValue;
                        } else {
                            validatedBody[field.name] = String(value);
                        }
                    } else if (field.default !== undefined) {
                        // 기본값 적용
                        validatedBody[field.name] = field.default;
                    }
                }
                
                console.log(`[Main] Validated body:`, validatedBody);
                
                // REST 요청 구성 - Palworld API 형식: /v1/api/{endpoint}
                commandPayload = {
                    endpoint: `/v1/api/${cmdName}`,
                    method: httpMethod,
                    body: validatedBody,
                    instance_id: id,
                    rest_host: instance.rest_host,
                    rest_port: instance.rest_port,
                    username: instance.rest_username,
                    password: instance.rest_password
                };

                // 사용자가 메시지를 인라인으로 입력한 경우 announce 본문으로 설정
                if (inlineMessage && Object.keys(validatedBody).length === 0) {
                    commandPayload.body = { message: inlineMessage };
                }
            }
        } else {
            // 기타 모듈은 기본 command 엔드포인트 사용
            console.log(`[Main] Using default command protocol for ${instance.module_name}`);
            protocolUrl = `${IPC_BASE}/api/instance/${id}/command`;
            commandPayload = {
                command: cmdName,
                args: command.args || {},
                instance_id: id
            };
        }
        
        console.log(`[Main] POST request to: ${protocolUrl}`);
        console.log(`[Main] Payload:`, commandPayload);
        const response = await axios.post(protocolUrl, commandPayload);
        console.log(`[Main] Response:`, response.data);
        
        return response.data;
    } catch (error) {
        console.error(`[Main] Error executing command:`, error.message);
        
        // HTTP 응답 에러 처리
        if (error.response) {
            const status = error.response.status;
            const data = error.response.data;
            
            let errorMsg = '';
            switch (status) {
                case 400:
                    errorMsg = `잘못된 요청: ${data.error || data.message || '입력값을 확인해주세요'}`;
                    break;
                case 401:
                    errorMsg = `인증 실패: 서버 설정에서 REST 사용자명/비밀번호를 확인해주세요`;
                    break;
                case 403:
                    errorMsg = `접근 거부: 권한이 없습니다`;
                    break;
                case 404:
                    errorMsg = `명령어를 찾을 수 없음: '${cmdName}' 명령어가 존재하지 않거나 서버가 실행중이지 않습니다`;
                    break;
                case 500:
                    errorMsg = `서버 내부 오류: ${data.error || data.message || '서버에서 오류가 발생했습니다'}`;
                    break;
                case 503:
                    errorMsg = `서비스 사용 불가: 서버가 응답하지 않습니다. 서버 상태를 확인해주세요`;
                    break;
                default:
                    errorMsg = `오류 (HTTP ${status}): ${data.error || data.message || error.message}`;
            }
            
            return { error: errorMsg };
        }
        
        // 네트워크 에러 처리
        if (error.code === 'ECONNREFUSED') {
            return { error: '데몬에 연결할 수 없습니다. 데몬이 실행중인지 확인해주세요' };
        }
        if (error.code === 'ETIMEDOUT') {
            return { error: '요청 시간 초과: 서버가 응답하지 않습니다' };
        }
        if (error.code === 'ENOTFOUND') {
            return { error: '서버를 찾을 수 없습니다. 네트워크 설정을 확인해주세요' };
        }
        
        return { error: `명령어 실행 실패: ${error.message}` };
    }
});

// Daemon 상태 확인 IPC 핸들러
ipcMain.handle('daemon:status', async () => {
    try {
        const response = await axios.get(`${IPC_BASE}/api/modules`, { timeout: 1000 });
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
            await new Promise(resolve => setTimeout(resolve, 1000));
        }
        console.log('Starting daemon...');
        startDaemon();
        // 데몬이 시작될 때까지 잠시 대기
        await new Promise(resolve => setTimeout(resolve, 2000));
        return { success: true, message: 'Daemon restarted successfully' };
    } catch (err) {
        console.error('Failed to restart daemon:', err);
        return { success: false, error: err.message };
    }
});

// Settings IPC handlers
ipcMain.handle('settings:load', () => {
    return loadSettings();
});

ipcMain.handle('settings:save', (event, settings) => {
    return saveSettings(settings);
});

ipcMain.handle('settings:getPath', () => {
    return getSettingsPath();
});

// Language IPC handlers
ipcMain.handle('language:get', () => {
    return getLanguage();
});

ipcMain.handle('language:set', (event, language) => {
    const success = setLanguage(language);
    
    // 번역 다시 로드
    translations = loadTranslations();
    
    // 데몬이 실행 중이면 재시작하여 새 언어 설정 적용
    if (daemonStartedByApp && daemonProcess) {
        console.log('Restarting daemon to apply new language setting...');
        stopDaemon();
        setTimeout(() => startDaemon(), 1000);
    }
    
    // Discord 봇이 실행 중이면 재시작하여 새 언어 설정 적용
    const botRunning = discordBotProcess && !discordBotProcess.killed;
    if (botRunning) {
        console.log('Restarting Discord bot to apply new language setting...');
        discordBotProcess.kill('SIGTERM');
        
        // 봇이 종료될 때까지 잠시 대기
        setTimeout(() => {
            // 설정 파일에서 봇 토큰과 설정을 다시 로드하여 재시작
            try {
                const botConfigPath = getBotConfigPath();
                if (fs.existsSync(botConfigPath)) {
                    const botConfig = JSON.parse(fs.readFileSync(botConfigPath, 'utf8'));
                    // 봇 닫기/재시작을 위해 IPC 이벤트 발생 (mainWindow가 있을 때만)
                    if (mainWindow) {
                        mainWindow.webContents.send('bot:relaunch', botConfig);
                    }
                }
            } catch (error) {
                console.error('Failed to relaunch Discord bot:', error);
            }
        }, 500);
    }
    
    return { success, language };
});

ipcMain.handle('language:getSystem', () => {
    return getSystemLanguage();
});

// File dialog handlers
ipcMain.handle('dialog:openFile', async (event, options) => {
    // 플랫폼별 기본 필터 설정
    let defaultFilters;
    if (process.platform === 'win32') {
        defaultFilters = [
            { name: 'Executable Files', extensions: ['exe'] },
            { name: 'All Files', extensions: ['*'] }
        ];
    } else if (process.platform === 'darwin') {
        defaultFilters = [
            { name: 'Applications', extensions: ['app'] },
            { name: 'All Files', extensions: ['*'] }
        ];
    } else {
        // Linux: 일반적으로 확장자 없음
        defaultFilters = [
            { name: 'All Files', extensions: ['*'] }
        ];
    }
    
    const result = await dialog.showOpenDialog({
        properties: ['openFile'],
        filters: options?.filters || defaultFilters
    });
    
    if (result.canceled) {
        return null;
    }
    return result.filePaths[0];
});

ipcMain.handle('dialog:openFolder', async () => {
    const result = await dialog.showOpenDialog({
        properties: ['openDirectory']
    });
    
    if (result.canceled) {
        return null;
    }
    return result.filePaths[0];
});

// Discord Bot process management
let discordBotProcess = null;

ipcMain.handle('discord:status', () => {
    if (discordBotProcess && !discordBotProcess.killed) {
        return 'running';
    }
    return 'stopped';
});

ipcMain.handle('discord:start', async (event, config) => {
    if (discordBotProcess && !discordBotProcess.killed) {
        return { error: 'Bot is already running' };
    }

    const botPath = path.join(__dirname, '..', 'discord_bot');
    const indexPath = path.join(botPath, 'index.js');

    if (!fs.existsSync(indexPath)) {
        return { error: `Bot script not found: ${indexPath}` };
    }

    // 현재 설정을 저장 (AppData와 discord_bot 폴더 모두)
    const configToSave = {
        prefix: config.prefix || '!saba',
        moduleAliases: config.moduleAliases || {},
        commandAliases: config.commandAliases || {}
    };
    
    // AppData에 저장
    saveBotConfig(configToSave);
    
    // discord_bot 폴더에도 저장
    const localConfigPath = path.join(botPath, 'bot-config.json');
    try {
        fs.writeFileSync(localConfigPath, JSON.stringify(configToSave, null, 2), 'utf8');
    } catch (e) {
        return { error: `Failed to write bot config: ${e.message}` };
    }

    try {
        // AppData 설정 경로를 환경 변수로 전달
        const appDataConfigPath = getBotConfigPath();
        const currentLanguage = getLanguage();
        
        discordBotProcess = spawn('node', [indexPath], {
            cwd: botPath,
            env: { 
                ...process.env, 
                DISCORD_TOKEN: config.token, 
                IPC_BASE: IPC_BASE,
                BOT_CONFIG_PATH: appDataConfigPath,
                SABA_LANG: currentLanguage  // Discord bot에 언어 설정 전달
            },
            stdio: ['ignore', 'pipe', 'pipe']
        });

        discordBotProcess.stdout.on('data', (data) => {
            console.log('[Discord Bot]', data.toString().trim());
        });

        discordBotProcess.stderr.on('data', (data) => {
            console.error('[Discord Bot Error]', data.toString().trim());
        });

        discordBotProcess.on('error', (err) => {
            console.error('Failed to start Discord Bot:', err);
            discordBotProcess = null;
        });

        discordBotProcess.on('exit', (code) => {
            console.log(`Discord Bot exited with code ${code}`);
            discordBotProcess = null;
        });

        return { success: true };
    } catch (e) {
        return { error: e.message };
    }
});

ipcMain.handle('discord:stop', () => {
    if (discordBotProcess && !discordBotProcess.killed) {
        console.log('[Discord] Stopping bot process with SIGTERM');
        discordBotProcess.kill('SIGTERM');
        
        // SIGTERM에 응답하지 않으면 5초 후 강제 종료
        const killTimeout = setTimeout(() => {
            if (discordBotProcess && !discordBotProcess.killed) {
                console.log('[Discord] Force killing bot process with SIGKILL');
                discordBotProcess.kill('SIGKILL');
            }
        }, 5000);
        
        discordBotProcess.once('exit', () => {
            clearTimeout(killTimeout);
        });
        
        return { success: true };
    }
    return { error: 'Bot is not running' };
});

// Bot Config API - AppData에 직접 저장/로드
ipcMain.handle('botConfig:load', async () => {
    return loadBotConfig();
});

ipcMain.handle('botConfig:save', async (event, config) => {
    try {
        const configToSave = {
            prefix: config.prefix || '!saba',
            moduleAliases: config.moduleAliases || {},
            commandAliases: config.commandAliases || {}
        };
        
        // 1. AppData에 저장
        const success = saveBotConfig(configToSave);
        if (!success) {
            return { error: 'Failed to save bot config to AppData' };
        }
        
        // 2. discord_bot 폴더에도 복사 (봇이 직접 읽을 수 있도록)
        const botPath = path.join(__dirname, '..', 'discord_bot');
        const botConfigPath = path.join(botPath, 'bot-config.json');
        
        try {
            fs.writeFileSync(botConfigPath, JSON.stringify(configToSave, null, 2), 'utf8');
            console.log('Bot config also saved to:', botConfigPath);
        } catch (fileError) {
            console.warn('Failed to save bot config to discord_bot folder:', fileError.message);
        }
        
        return { success: true, message: 'Bot config saved' };
    } catch (error) {
        console.error('Failed to save bot config:', error.message);
        return { error: error.message };
    }
});

// Window Controls (Title Bar)
ipcMain.on('window:minimize', () => {
    if (mainWindow && !mainWindow.isDestroyed()) {
        mainWindow.minimize();
    }
});

ipcMain.on('window:maximize', () => {
    if (mainWindow && !mainWindow.isDestroyed()) {
        if (mainWindow.isMaximized()) {
            mainWindow.restore();
        } else {
            mainWindow.maximize();
        }
    }
});

ipcMain.on('window:close', () => {
    if (mainWindow && !mainWindow.isDestroyed()) {
        mainWindow.close();
    }
});
