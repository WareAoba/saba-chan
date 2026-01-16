const { app, BrowserWindow, Menu, ipcMain, Tray, nativeImage } = require('electron');
const { dialog } = require('electron');
const path = require('path');
const axios = require('axios');
const { spawn } = require('child_process');
const fs = require('fs');

const IPC_BASE = 'http://127.0.0.1:57474'; // localhost 대신 127.0.0.1 명시

let mainWindow;
let daemonProcess = null;
let daemonStartedByApp = false;
let tray = null;

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
    daemonStartedByApp = true;
    
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
    if (daemonProcess && daemonStartedByApp) {
        console.log('Stopping Core Daemon...');
        daemonProcess.kill('SIGTERM');
        daemonProcess = null;
    }
}

// 이미 떠 있는 데몬이 있으면 재실행하지 않고 재사용
async function ensureDaemon() {
    try {
        // 여러 엔드포인트로 체크 (일부 엔드포인트가 500을 반환해도 데몬은 실행 중)
        const response = await axios.get(`${IPC_BASE}/api/modules`, { timeout: 1000 });
        if (response.status === 200) {
            console.log('Existing daemon detected on IPC port. Skipping launch.');
            daemonStartedByApp = false;
            return;
        }
    } catch (err) {
        // ECONNREFUSED = 데몬이 안 떠있음, 그 외 에러 = 데몬은 떠있지만 문제 발생
        if (err.code === 'ECONNREFUSED' || err.code === 'ENOTFOUND') {
            console.log('No daemon detected, launching new one...');
            startDaemon();
        } else {
            console.log('Daemon might be running (got error but not connection refused):', err.message);
            daemonStartedByApp = false;
        }
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

    // 윈도우 닫기 이벤트 가로채기 - React QuestionModal로 확인
    mainWindow.on('close', (e) => {
        e.preventDefault(); // 기본 닫기 동작 중단
        
        // React 앱에 다이얼로그 표시 요청
        mainWindow.webContents.send('app:closeRequest');
    });

    // 개발 모드: http://localhost:3000, 프로덕션: build/index.html
    const startURL = process.env.ELECTRON_START_URL || 'http://localhost:3000';
    mainWindow.loadURL(startURL);

    // Dev tools - 디버깅 활성화
    mainWindow.webContents.openDevTools();
}

// React에서 종료 선택 응답 처리
ipcMain.on('app:closeResponse', (event, choice) => {
    if (choice === 'hide') {
        // GUI만 닫기 - 트레이로 최소화
        mainWindow.hide();
    } else if (choice === 'quit') {
        // 완전히 종료
        mainWindow.removeAllListeners('close'); // close 이벤트 리스너 제거
        mainWindow.close();
        stopDaemon();
        app.quit();
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
            label: '🔄 데몬 상태',
            enabled: false,
            label: daemonProcess ? '🟢 데몬 실행 중' : '⚪ 데몬 중지됨'
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
                stopDaemon();
                if (tray) {
                    tray.destroy();
                    tray = null;
                }
                app.quit();
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
                stopDaemon();
                if (tray) {
                    tray.destroy();
                    tray = null;
                }
                app.quit();
            }
        }
    ]);
    
    tray.setContextMenu(contextMenu);
}

app.on('ready', () => {
    createTray();
    ensureDaemon().then(() => {
        // Daemon이 시작될 시간을 주기 위해 약간 대기
        setTimeout(() => {
            createWindow();
            updateTrayMenu();
        }, 1500);
    });
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
    stopDaemon();
    if (tray) {
        tray.destroy();
        tray = null;
    }
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
        const body = {
            module: options.module || 'minecraft',
            config: options.config || {}
        };
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

ipcMain.handle('module:getMetadata', async (event, moduleName) => {
    try {
        const response = await axios.get(`${IPC_BASE}/api/module/${moduleName}`);
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
        return { error: error.message };
    }
});

ipcMain.handle('instance:executeCommand', async (event, id, command) => {
    try {
        console.log(`[Main] Executing command for instance ${id}:`, command);
        const url = `${IPC_BASE}/api/instance/${id}/command`;
        console.log(`[Main] POST request to: ${url}`);
        const response = await axios.post(url, command);
        console.log(`[Main] Response:`, response.data);
        return response.data;
    } catch (error) {
        console.error(`[Main] Error executing command:`, error.message);
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

// File dialog handlers
ipcMain.handle('dialog:openFile', async (event, options) => {
    const result = await dialog.showOpenDialog({
        properties: ['openFile'],
        filters: options?.filters || [
            { name: 'Executable Files', extensions: ['exe'] },
            { name: 'All Files', extensions: ['*'] }
        ]
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

    // Write bot config to a temp file for the bot to read
    const configPath = path.join(botPath, 'bot-config.json');
    try {
        fs.writeFileSync(configPath, JSON.stringify({
            prefix: config.prefix || '!pal',
            moduleAliases: config.moduleAliases || {},
            commandAliases: config.commandAliases || {}
        }, null, 2), 'utf8');
    } catch (e) {
        return { error: `Failed to write bot config: ${e.message}` };
    }

    try {
        discordBotProcess = spawn('node', [indexPath], {
            cwd: botPath,
            env: { ...process.env, DISCORD_TOKEN: config.token, IPC_BASE: IPC_BASE },
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
    if (discordBotProcess) {
        discordBotProcess.kill('SIGTERM');
        discordBotProcess = null;
        return { success: true };
    }
    return { error: 'Bot is not running' };
});

// Bot Config API
ipcMain.handle('botConfig:load', async () => {
    try {
        const response = await axios.get(`${IPC_BASE}/api/config/bot`);
        return response.data;
    } catch (error) {
        console.error('Failed to load bot config:', error.message);
        return { prefix: '!saba', moduleAliases: {}, commandAliases: {} };
    }
});

ipcMain.handle('botConfig:save', async (event, config) => {
    try {
        const response = await axios.put(`${IPC_BASE}/api/config/bot`, config);
        return { success: true, message: response.data.message };
    } catch (error) {
        console.error('Failed to save bot config:', error.message);
        return { error: error.message };
    }
});
