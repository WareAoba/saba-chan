import fs from 'node:fs';
import path from 'node:path';
import vm from 'node:vm';
import { beforeEach, describe, expect, it, vi } from 'vitest';

const guiDir = path.resolve(__dirname, '../..');
const source = fs.readFileSync(path.join(guiDir, 'main.js'), 'utf8');
let context;
let handlers;
let ipcListeners;
let requestInterceptor;
let responseInterceptor;
let daemon;
let createOptions;
let shell;
let logs;
let windows;
let app;
let child;
let fsMock;
let files;

function makeDaemon() {
    const instance = {
        // 기본: 모든 요청 성공 (method별로 테스트에서 덮어쓴다)
        request: vi.fn(async (config) => ({ status: 200, data: { success: true, config } })),
        interceptors: {
            request: { use: (handler) => { requestInterceptor = handler; } },
            response: { use: (_success, error) => { responseInterceptor = error; } },
        },
    };
    for (const method of ['get', 'post', 'put', 'patch', 'delete']) {
        instance[method] = vi.fn((url, ...rest) => {
            const config = ['get', 'delete'].includes(method) ? rest[0] || {} : { ...(rest[1] || {}), data: rest[0] };
            return instance.request({ ...config, method, url });
        });
    }
    return instance;
}

const refused = () => Object.assign(new Error('connect ECONNREFUSED'), { code: 'ECONNREFUSED' });

function run(code) {
    return vm.runInContext(code, context);
}

beforeEach(() => {
    handlers = new Map();
    ipcListeners = new Map();
    windows = [];
    logs = [];
    files = new Map();
    shell = { openExternal: vi.fn().mockResolvedValue(), openPath: vi.fn().mockResolvedValue('') };
    daemon = makeDaemon();
    const axios = { create: vi.fn((options) => { createOptions = options; return daemon; }) };
    class BrowserWindow {
        constructor(options) {
            this.options = options;
            this.listeners = new Map();
            this.webContents = {
                on: vi.fn(), setWindowOpenHandler: vi.fn(), openDevTools: vi.fn(), send: vi.fn(),
                isCrashed: () => false,
            };
            windows.push(this);
        }
        on(event, handler) { this.listeners.set(event, handler); }
        once() {}
        loadURL() { return Promise.resolve(); }
        loadFile() { return Promise.resolve(); }
        removeMenu() {}
        removeAllListeners() {}
        destroy() {}
        hide() {}
        isDestroyed() { return false; }
    }
    BrowserWindow.fromWebContents = () => null;
    app = { setPath: vi.fn(), on: vi.fn(), quit: vi.fn(), isPackaged: false, getPath: () => '/synthetic', getLocale: () => 'en-US' };
    child = { execSync: vi.fn(), spawn: vi.fn() };
    fsMock = {
        existsSync: (p) => files.has(p),
        readFileSync: (p) => (files.has(p) ? files.get(p) : 'synthetic-credential-for-test'),
        writeFileSync: vi.fn((p, data) => files.set(p, data)),
        mkdirSync: vi.fn(),
        statSync: (p) => ({ isDirectory: () => p.startsWith('/instances/') }),
    };
    const dependencies = {
        electron: {
            app,
            ipcMain: {
                handle: (channel, handler) => handlers.set(channel, handler),
                on: (channel, handler) => ipcListeners.set(channel, handler),
            },
            BrowserWindow,
            shell,
            Notification: { isSupported: () => false },
        },
        path,
        url: { pathToFileURL: (file) => new URL(`file:///${file.replaceAll('\\', '/')}`) },
        axios,
        child_process: child,
        string_decoder: { StringDecoder: class {} },
        fs: fsMock,
        '../shared/constants': {
            DEFAULT_IPC_PORT: 57474,
            DEFAULT_DAEMON_URL: 'http://127.0.0.1:57474',
            DEFAULT_RELAY_URL: 'https://relay.example',
            SUPPORTED_LANGUAGES: ['en'],
            getSabaDataDir: () => '/synthetic',
        },
        '../shared/botConfigPayload': { buildBotConfigSavePayload: vi.fn((config, existing) => ({ ...existing, ...config })) },
    };
    context = vm.createContext({
        require(name) {
            name = name.replace(/^node:/, '');
            if (!(name in dependencies)) throw new Error(`Unexpected dependency: ${name}`);
            return dependencies[name];
        },
        __dirname: guiDir,
        process: { env: {}, platform: 'linux', argv: [], on: vi.fn(), execPath: '/electron' },
        console: Object.fromEntries(['log', 'warn', 'error'].map((level) => [level, (...args) => logs.push(args)])),
        URL, URLSearchParams, setTimeout, clearTimeout, setInterval, clearInterval, Promise, Buffer,
    });
    vm.runInContext(source, context);
});

describe('Electron main daemon client', () => {
    it('uses one daemon client that never follows redirects', () => {
        expect(createOptions.maxRedirects).toBe(0);
    });

    it('does not attach the daemon token to an external HTTP request', () => {
        const config = { url: 'https://external.example/api', headers: {} };
        requestInterceptor(config);
        expect(config.headers).not.toHaveProperty('X-Saba-Token');
    });

    it('attaches the token to the daemon without writing token fragments to logs', () => {
        const config = { url: '/api/ping', headers: {} };
        requestInterceptor(config);
        expect(config.baseURL).toBe('http://127.0.0.1:57474');
        expect(config.headers['X-Saba-Token']).toBe('synthetic-credential-for-test');
        expect(JSON.stringify(logs)).not.toContain('synthet');
    });

    it('does not retry an external 401 with the daemon token', async () => {
        const error = { response: { status: 401 }, config: { url: 'https://external.example/api', headers: {} } };
        await expect(responseInterceptor(error)).rejects.toBe(error);
        expect(error.config.headers).not.toHaveProperty('X-Saba-Token');
    });

    it('reloads the token and retries a daemon 401 once without logging the token', async () => {
        const error = { response: { status: 401 }, config: { url: '/api/ping', headers: {} } };
        await responseInterceptor(error);
        expect(daemon.request).toHaveBeenCalledTimes(1);
        expect(error.config._retried).toBe(true);
        await expect(responseInterceptor(error)).rejects.toBe(error);
        expect(JSON.stringify(logs)).not.toContain('syntheti');
    });

    it('does not retry relay-proxy 401s (relay authentication, not the daemon token)', async () => {
        const error = { response: { status: 401 }, config: { url: '/api/relay/host/x/status', headers: {} } };
        await expect(responseInterceptor(error)).rejects.toBe(error);
        expect(daemon.request).not.toHaveBeenCalled();
    });

    it('returns the daemon error message instead of a generic one', async () => {
        daemon.request.mockRejectedValueOnce({ response: { status: 409, data: { error: 'Instance name already exists' } } });
        const result = await handlers.get('instance:create')({}, { name: 'x' });
        expect(result).toEqual({ error: 'Instance name already exists' });
    });

    it('does not log settings payloads or responses containing passwords', async () => {
        daemon.request.mockResolvedValueOnce({ data: { success: true, echoed: 'fake-password-for-regression' } });
        const result = await handlers.get('instance:updateSettings')({}, 'server', { admin_password: 'fake-password-for-regression' });
        expect(result.success).toBe(true);
        expect(JSON.stringify(logs)).not.toContain('fake-password-for-regression');
    });

    it('does not log private command arguments', async () => {
        const result = await handlers.get('instance:executeCommand')({}, 'server', {
            command: 'setpassword fake-password-for-regression', args: {},
            commandMetadata: { method: 'rcon', rcon_template: 'setpassword {message}' },
        });
        expect(result.success).toBe(true);
        expect(daemon.request.mock.calls[0][0].data).toEqual({ command: 'setpassword fake-password-for-regression' });
        expect(JSON.stringify(logs)).not.toContain('fake-password-for-regression');
    });
});

describe('Electron main windows', () => {
    it('keeps renderer navigation within the current application', () => {
        run('createWindow()');
        const listener = windows[0].webContents.on.mock.calls.find(([event]) => event === 'will-navigate')?.[1];
        const event = { preventDefault: vi.fn() };
        listener(event, 'http://localhost:5173/?console-popout=example');
        expect(event.preventDefault).not.toHaveBeenCalled();
    });

    it('prevents external pages from replacing the privileged renderer', () => {
        run('createWindow()');
        const webContents = windows[0].webContents;
        const listener = webContents.on.mock.calls.find(([event]) => event === 'will-navigate')?.[1];
        const event = { preventDefault: vi.fn() };
        listener(event, 'https://external.example/');
        expect(event.preventDefault).toHaveBeenCalled();
        expect(webContents.setWindowOpenHandler).toHaveBeenCalled();
    });

    it('creates sandboxed renderer windows', () => {
        run('createWindow()');
        expect(windows[0].options.webPreferences).toMatchObject({ sandbox: true, contextIsolation: true, nodeIntegration: false });
    });

    it('does not register DevTools shortcuts in packaged builds', () => {
        app.isPackaged = true;
        run('createWindow()');
        const events = windows[0].webContents.on.mock.calls.map(([event]) => event);
        expect(events).not.toContain('before-input-event');
    });

    it('closes directly when the renderer has crashed and cannot show the exit dialog', () => {
        run('createWindow()');
        windows[0].webContents.isCrashed = () => true;
        windows[0].listeners.get('close')({ preventDefault: vi.fn() });
        expect(windows[0].webContents.send).not.toHaveBeenCalledWith('app:closeRequest');
    });

    it.each(['file:///C:/test.exe', 'javascript:alert(1)', 'custom-app://run', 'invalid'])
    ('rejects an unsupported external URL: %s', async (url) => {
        const result = await handlers.get('shell:openExternal')({}, url);
        expect(result.error).toBeTruthy();
        expect(shell.openExternal).not.toHaveBeenCalled();
    });

    it('opens ordinary HTTPS links', async () => {
        const result = await handlers.get('shell:openExternal')({}, 'https://discord.com/oauth2/authorize');
        expect(result.success).toBe(true);
        expect(shell.openExternal).toHaveBeenCalledWith('https://discord.com/oauth2/authorize');
    });
});

describe('Electron main instance folder', () => {
    it('opens only the working directory the daemon reports for the instance', async () => {
        daemon.request.mockResolvedValueOnce({ data: { module_settings: { working_dir: '/instances/alpha' } } });
        const result = await handlers.get('instance:openFolder')({}, 'alpha');
        expect(result.success).toBe(true);
        expect(daemon.request.mock.calls[0][0].url).toBe('/api/instance/alpha');
        expect(shell.openPath).toHaveBeenCalledWith('/instances/alpha');
    });

    it('refuses to open a path that is not a directory', async () => {
        daemon.request.mockResolvedValueOnce({ data: { working_dir: '/tmp/payload.exe' } });
        const result = await handlers.get('instance:openFolder')({}, 'alpha');
        expect(result.error).toBeTruthy();
        expect(shell.openPath).not.toHaveBeenCalled();
    });

    it('does not expose an arbitrary-path shell handler', () => {
        expect(handlers.has('shell:openPath')).toBe(false);
    });
});

describe('Electron main quit and daemon lifecycle', () => {
    it('cleanQuit completes the whole sequence without errors', async () => {
        await run('cleanQuit()');
        expect(JSON.stringify(logs)).not.toContain('Error during clean quit');
        expect(JSON.stringify(logs)).toContain('Clean quit sequence completed');
        expect(child.execSync).toHaveBeenCalledWith(expect.stringContaining('saba-core'), expect.anything());
        expect(app.quit).toHaveBeenCalled();
    });

    it('restarting an external daemon asks it to shut down instead of spawning a duplicate', async () => {
        let healthy = true;
        daemon.request.mockImplementation(async (config) => {
            if (config.url === '/health') {
                if (!healthy) throw refused();
                return { status: 200, data: 'ok' };
            }
            if (config.url === '/api/daemon/shutdown') {
                healthy = false;
                return { data: { ok: true } };
            }
            return { data: {} };
        });
        await handlers.get('daemon:restart')({});
        const urls = daemon.request.mock.calls.map(([config]) => config.url);
        expect(urls).toContain('/api/daemon/shutdown');
    }, 15000);

    it('reports failure when the restarted daemon never answers', async () => {
        daemon.request.mockImplementation(async () => { throw refused(); });
        const result = await handlers.get('daemon:restart')({});
        expect(result.success).toBe(false);
    }, 15000);
});

describe('Electron main settings', () => {
    it('saves through the daemon when it is reachable even if this GUI did not spawn it', async () => {
        daemon.request.mockImplementation(async (config) => ({ data: config.method === 'get' ? { language: 'ko' } : { ok: true } }));
        const ok = await handlers.get('settings:save')({}, { refreshInterval: 5000 });
        expect(ok).toBe(true);
        const put = daemon.request.mock.calls.find(([config]) => config.method === 'put')[0];
        expect(put.data).toEqual({ language: 'ko', refreshInterval: 5000 });
        expect(fsMock.writeFileSync).not.toHaveBeenCalled();
    });

    it('falls back to the local file only when the daemon is unreachable', async () => {
        daemon.request.mockImplementation(async () => { throw refused(); });
        const ok = await handlers.get('settings:save')({}, { refreshInterval: 5000 });
        expect(ok).toBe(true);
        expect(fsMock.writeFileSync).toHaveBeenCalled();
        expect(JSON.parse(fsMock.writeFileSync.mock.calls[0][1]).refreshInterval).toBe(5000);
    });

    it('debounces window bounds saves while the window is being dragged', async () => {
        run('createWindow()');
        const win = windows[0];
        Object.assign(win, { isMaximized: () => false, isMinimized: () => false, getBounds: () => ({ width: 900, height: 900, x: 1, y: 2 }) });
        for (let i = 0; i < 10; i++) win.listeners.get('moved')();
        await new Promise((resolve) => setTimeout(resolve, 700));
        const puts = daemon.request.mock.calls.filter(([config]) => config.method === 'put');
        expect(puts).toHaveLength(1);
        expect(puts[0][0].data.windowBounds).toEqual({ width: 900, height: 900, x: 1, y: 2 });
    });
});

describe('Electron main bot config', () => {
    it('saves merged bot config through the daemon and never PATCHes the relay itself', async () => {
        daemon.request.mockImplementation(async (config) => ({ data: config.method === 'get' ? { prefix: '!old', cloud: { hostId: 'h', relayUrl: 'https://r' } } : { ok: true } }));
        const result = await handlers.get('botConfig:save')({}, { prefix: '!new', cloud: { hostId: 'h', relayUrl: 'https://r' } });
        expect(result.success).toBe(true);
        const methods = daemon.request.mock.calls.map(([config]) => `${config.method} ${config.url}`);
        expect(methods).toEqual(['get /api/config/bot', 'put /api/config/bot']);
    });

    it('refuses to save when the existing bot config cannot be read', async () => {
        daemon.request.mockImplementation(async () => { throw refused(); });
        const result = await handlers.get('botConfig:save')({}, { prefix: '!new' });
        expect(result.error).toBeTruthy();
        expect(daemon.request.mock.calls.some(([config]) => config.method === 'put')).toBe(false);
    });
});

describe('Electron preload boundary', () => {
    function loadPreload() {
        const exposed = {};
        const ipcRenderer = { on: vi.fn(), removeListener: vi.fn(), invoke: vi.fn(), send: vi.fn() };
        vm.runInNewContext(fs.readFileSync(path.join(guiDir, 'preload.js'), 'utf8'), {
            require: () => ({ contextBridge: { exposeInMainWorld: (key, api) => { exposed[key] = api; } }, ipcRenderer }),
        });
        return { exposed, ipcRenderer };
    }

    it('does not expose the privileged IPC event to a close listener', () => {
        const { exposed, ipcRenderer } = loadPreload();
        const callback = vi.fn();
        exposed.api.onCloseRequest(callback);
        const handler = ipcRenderer.on.mock.calls[0][1];
        handler({ sender: { privileged: true } });
        expect(callback).toHaveBeenCalledWith();
    });

    it('unsubscribes only its own listener', () => {
        const { exposed, ipcRenderer } = loadPreload();
        const off = exposed.api.onStatusUpdate(vi.fn());
        const handler = ipcRenderer.on.mock.calls[0][1];
        off();
        expect(ipcRenderer.removeListener).toHaveBeenCalledWith('status:update', handler);
    });

    it('every invoke channel in preload has a main handler', () => {
        const preload = fs.readFileSync(path.join(guiDir, 'preload.js'), 'utf8');
        const channels = [...preload.matchAll(/(?:invoke|ipcRenderer\.invoke)\('([^']+)'/g)].map((m) => m[1]);
        expect(channels.length).toBeGreaterThan(50);
        for (const channel of channels) expect(handlers.has(channel), channel).toBe(true);
    });
});
