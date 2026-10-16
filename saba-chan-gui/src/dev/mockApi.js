/**
 * 개발용 가짜 window.api — Electron 없이 `npm run dev`로 브라우저에서 화면을 볼 때만 쓴다.
 * index.js가 DEV 빌드이고 window.api가 없을 때만 설치한다. 프로덕션 번들에는 들어가지 않는다.
 */
const now = () => Math.floor(Date.now() / 1000);

const modules = [
    {
        name: 'minecraft',
        game_name: 'Minecraft',
        display_name: 'Minecraft: Java Edition',
        version: '1.2.0',
        description: 'Minecraft Java 서버',
        interaction_mode: 'console',
        protocols: { supported: ['stdin', 'rcon'], default: 'rcon' },
        commands: {
            fields: [
                {
                    name: 'say',
                    label: '전체 메시지',
                    method: 'stdin',
                    inputs: [{ name: 'message', type: 'text', required: true }],
                },
                { name: 'list', label: '접속자 목록', method: 'stdin' },
                { name: 'save-all', label: '월드 저장', method: 'stdin' },
            ],
        },
        settings: { fields: [{ name: 'port', label: '포트', type: 'number', default: 25565 }] },
    },
    {
        name: 'palworld',
        game_name: 'Palworld',
        display_name: 'Palworld',
        version: '1.0.0',
        description: 'Palworld 전용 서버',
        commands: { fields: [{ name: 'players', label: '접속자', method: 'rest' }] },
        settings: { fields: [{ name: 'port', label: '포트', type: 'number', default: 8211 }] },
    },
    {
        name: 'zomboid',
        game_name: 'Project Zomboid',
        display_name: 'Project Zomboid',
        version: '1.0.0',
        description: 'Project Zomboid 서버',
        commands: { fields: [] },
        settings: { fields: [] },
    },
];

let servers = [
    {
        id: 'srv-1',
        name: '야생 서버',
        module: 'minecraft',
        status: 'running',
        start_time: now() - 3 * 3600 - 420,
        port: 25565,
        pid: 1234,
        server_version: '1.21.1',
        module_settings: { port: 25565 },
    },
    {
        id: 'srv-2',
        name: '팰월드 친구들',
        module: 'palworld',
        status: 'stopped',
        port: 8211,
        module_settings: { port: 8211 },
    },
    {
        id: 'srv-3',
        name: '좀보이드 하드코어',
        module: 'zomboid',
        status: 'starting',
        start_time: now() - 20,
        port: 16261,
    },
    {
        id: 'srv-4',
        name: '크리에이티브',
        module: 'minecraft',
        status: 'stopped',
        port: 25566,
        server_version: '1.20.4',
    },
];

const consoleLines = [
    '[12:00:01] [Server thread/INFO]: Starting minecraft server version 1.21.1',
    '[12:00:03] [Server thread/INFO]: Preparing level "world"',
    '[12:00:09] [Server thread/INFO]: Done (6.214s)! For help, type "help"',
    '[12:04:51] [Server thread/INFO]: Steve joined the game',
    "[12:05:12] [Server thread/WARN]: Can't keep up! Is the server overloaded?",
];

const ok = (v = {}) => Promise.resolve(v);
// 주소 뒤에 ?daemon=stopped&bot=running 처럼 붙이면 상태 표시를 바꿔 볼 수 있다
const params = new URLSearchParams(globalThis.location?.search || '');
const mockBot = params.get('bot') || 'stopped';
const mockDaemonUp = params.get('daemon') !== 'stopped';
const mockUpdate = params.get('update'); // ok | fail
const settings = {};

const impl = {
    serverList: () => ok({ servers: servers.map((s) => ({ ...s })) }),
    moduleList: () => ok({ modules }),
    settingsLoad: () => ok(settings),
    settingsSave: (s) => {
        Object.assign(settings, s);
        return ok({ success: true });
    },
    settingsGetPath: () => ok('C:/Users/me/AppData/Roaming/saba-chan/config.json'),
    botConfigLoad: () =>
        ok({
            prefix: '!saba',
            cloud: { relayUrl: 'https://relay.example', hostId: mockBot === 'stopped' ? '' : 'mock-host' },
            moduleAliases: {},
            commandAliases: {},
        }),
    discordBotStatus: () => ok(mockBot === 'stopped' ? 'stopped' : 'running'),
    relayCheckHostStatus: () => ok(mockBot === 'running' ? { online: true } : { error: 'offline' }),
    daemonStatus: () => ok({ running: mockDaemonUp }),
    updaterLastResult: ({ clear } = {}) =>
        ok(
            !mockUpdate || clear
                ? null
                : {
                      timestamp: now(),
                      success: mockUpdate === 'ok',
                      updated_components: mockUpdate === 'ok' ? ['saba-core', 'saba-chan-gui'] : ['saba-chan-gui'],
                      failed: mockUpdate === 'ok' ? [] : ['saba-core'],
                      message: mockUpdate === 'ok' ? '' : '데몬 파일을 교체하지 못했어요 (사용 중)',
                  },
        ),
    extensionList: () => ok({ extensions: [] }),
    extensionInitStatus: () => ok({ initializing: false, in_progress: {} }),
    getAppVersion: () => ok('0.1.1'),
    managedConsole: (_id, since = 0) =>
        ok({
            lines: consoleLines
                .map((content, i) => ({ id: i + 1, content, source: 'stdout' }))
                .filter((l) => l.id >= since),
            stdin_available: true,
        }),
    serverStart: (name) => {
        servers = servers.map((s) => (s.name === name ? { ...s, status: 'running', start_time: now() } : s));
        return ok({ success: true });
    },
    instanceStart: (id) => {
        servers = servers.map((s) => (s.id === id ? { ...s, status: 'running', start_time: now() } : s));
        return ok({ success: true });
    },
    managedStart: (id) => impl.instanceStart(id),
    serverStop: (name) => {
        servers = servers.map((s) => (s.name === name ? { ...s, status: 'stopped', start_time: null } : s));
        return ok({ success: true });
    },
    onStatusUpdate: (cb) => setTimeout(() => cb({ step: 'ready', message: 'ready' }), 50),
};

export function installMockApi() {
    const noop = () => ok({});
    window.api = new Proxy(impl, {
        get(target, prop) {
            if (prop in target) return target[prop];
            if (typeof prop === 'string' && (prop.startsWith('on') || prop.startsWith('off'))) return () => {};
            return noop;
        },
    });
    window.electron = {
        minimizeWindow() {},
        maximizeWindow() {},
        closeWindow() {},
        getLanguage: () => ok('ko'),
        setLanguage: () => ok(),
        shell: { openExternal: () => ok() },
    };
}
