const { app, BrowserWindow, Menu, ipcMain } = require('electron');
const path = require('path');
const axios = require('axios');
const { spawn } = require('child_process');
const fs = require('fs');

const IPC_BASE = 'http://127.0.0.1:57474'; // localhost 대신 127.0.0.1 명시

let mainWindow;
let daemonProcess = null;

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
    // 기본 설정
    return {
        modulesPath: path.join(__dirname, '..', 'modules'),
        autoRefresh: true,
        refreshInterval: 2000,
        windowBounds: { width: 1200, height: 800 }
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
    // Release 빌드 우선, 없으면 debug 빌드 사용
    const releasePath = path.join(__dirname, '..', 'target', 'release', 'core_daemon.exe');
    const debugPath = path.join(__dirname, '..', 'target', 'debug', 'core_daemon.exe');
    
    const daemonPath = fs.existsSync(releasePath) ? releasePath : debugPath;
    
    console.log('Starting Core Daemon:', daemonPath);
    
    if (!fs.existsSync(daemonPath)) {
        console.error('Core Daemon executable not found at:', daemonPath);
        return;
    }
    
    daemonProcess = spawn(daemonPath, [], {
        cwd: path.join(__dirname, '..'),
        env: { ...process.env, RUST_LOG: 'info' },
        stdio: ['ignore', 'pipe', 'pipe'] // stdout, stderr를 pipe로 받음
    });
    
    // stdout 로그 출력
    daemonProcess.stdout.on('data', (data) => {
        console.log('[Daemon]', data.toString().trim());
    });
    
    // stderr 로그 출력
    daemonProcess.stderr.on('data', (data) => {
        console.error('[Daemon Error]', data.toString().trim());
    });
    
    daemonProcess.on('error', (err) => {
        console.error('Failed to start Core Daemon:', err);
    });
    
    daemonProcess.on('exit', (code) => {
        console.log(`Core Daemon exited with code ${code}`);
        daemonProcess = null;
    });
}

// Core Daemon 종료
function stopDaemon() {
    if (daemonProcess) {
        console.log('Stopping Core Daemon...');
        daemonProcess.kill('SIGTERM');
        daemonProcess = null;
    }
}

function createWindow() {
    const settings = loadSettings();
    const { width, height } = settings.windowBounds || { width: 1200, height: 800 };
    
    mainWindow = new BrowserWindow({
        width,
        height,
        webPreferences: {
            preload: path.join(__dirname, 'preload.js'),
            nodeIntegration: false,
            contextIsolation: true
        }
    });

    // 윈도우 크기 변경 시 저장
    mainWindow.on('resize', () => {
        const bounds = mainWindow.getBounds();
        const currentSettings = loadSettings();
        currentSettings.windowBounds = { width: bounds.width, height: bounds.height };
        saveSettings(currentSettings);
    });

    // 개발 모드: http://localhost:3000, 프로덕션: build/index.html
    const startURL = process.env.ELECTRON_START_URL || 'http://localhost:3000';
    mainWindow.loadURL(startURL);

    // Dev tools - 디버깅 활성화
    mainWindow.webContents.openDevTools();
}

app.on('ready', () => {
    startDaemon();
    
    // Daemon이 시작될 시간을 주기 위해 약간 대기
    setTimeout(() => {
        createWindow();
    }, 3000); // 2초에서 3초로 증가
});

app.on('window-all-closed', () => {
    stopDaemon();
    if (process.platform !== 'darwin') {
        app.quit();
    }
});

app.on('before-quit', () => {
    stopDaemon();
});

// IPC handlers
ipcMain.handle('server:list', async () => {
    try {
        const response = await axios.get(`${IPC_BASE}/api/servers`);
        return response.data;
    } catch (error) {
        return { error: error.message };
    }
});

ipcMain.handle('server:start', async (event, name, options = {}) => {
    try {
        const body = options.resource || {};
        const response = await axios.post(`${IPC_BASE}/api/server/${name}/start`, body);
        return response.data;
    } catch (error) {
        return { error: error.message };
    }
});

ipcMain.handle('server:stop', async (event, name, options = {}) => {
    try {
        const body = options || {};
        const response = await axios.post(`${IPC_BASE}/api/server/${name}/stop`, body);
        return response.data;
    } catch (error) {
        return { error: error.message };
    }
});

ipcMain.handle('server:status', async (event, name) => {
    try {
        const response = await axios.get(`${IPC_BASE}/api/server/${name}/status`);
        return response.data;
    } catch (error) {
        return { error: error.message };
    }
});

ipcMain.handle('module:list', async () => {
    try {
        const response = await axios.get(`${IPC_BASE}/api/modules`);
        return response.data;
    } catch (error) {
        return { error: error.message };
    }
});

ipcMain.handle('instance:create', async (event, data) => {
    try {
        const response = await axios.post(`${IPC_BASE}/api/instances`, data);
        return response.data;
    } catch (error) {
        return { error: error.message };
    }
});

ipcMain.handle('instance:delete', async (event, id) => {
    try {
        const response = await axios.delete(`${IPC_BASE}/api/instance/${id}`);
        return response.data;
    } catch (error) {
        return { error: error.message };
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
