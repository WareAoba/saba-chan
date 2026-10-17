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
        protocols: { supported: ['stdin'], default: 'stdin' },
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
                { name: 'stop', label: '서버 정지', method: 'stdin', risk: 'danger' },
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
const mockFailed = params.get('failed') // 불러오지 못한 모듈 보기
    ? [{ path: 'modules/terraria', error: "module.toml: unknown field 'protocl' (did you mean 'protocol'?)" }]
    : [];
const settings = {};

const impl = {
    serverList: () => ok({ servers: servers.map((s) => ({ ...s })) }),
    moduleList: () => ok({ modules, failed: mockFailed }),
    instanceEvents: () =>
        ok({
            events: [
                { at: now() - 7200, kind: 'user_stop', exit_code: 0 },
                {
                    at: now() - 3600,
                    kind: 'crash',
                    exit_code: -1073741819,
                    evidence: 'hs_err_pid4120.log',
                    restart_in_secs: 10,
                    note: 'restarting',
                },
                { at: now() - 600, kind: 'unknown_exit', exit_code: 1, note: 'restart_on_unknown_exit is off' },
            ],
        }),
    instanceScheduleGet: () =>
        ok({
            success: true,
            schedule: [
                { action: 'backup', every_minutes: 120, enabled: true },
                { action: 'restart', at: '04:00', warn_minutes: [10, 1], enabled: true },
                { action: 'announce', every_minutes: 60, message: '디스코드에도 놀러 오세요!', enabled: false },
            ],
            backup_keep: 10,
            backup_supported: true,
            announce_supported: true,
        }),
    instanceScheduleSave: () => ok({ success: true }),
    instanceBackups: () =>
        ok({
            success: true,
            backups: [
                { name: 'mock-scheduled.zip', kind: 'scheduled', created: now() - 1800, size: 184_320_000 },
                { name: 'mock-pre-update.zip', kind: 'pre-update', created: now() - 86400, size: 180_100_000 },
                { name: 'mock-manual.zip', kind: 'manual', created: now() - 3 * 86400, size: 95_000_000 },
            ],
        }),
    instanceBackupCreate: () => ok({ success: true, backup: { name: 'new-manual.zip' } }),
    instanceBackupDelete: () => ok({ success: true }),
    instanceBackupRestore: () => ok({ success: true, restored: true }),
    chatbotGet: () =>
        ok({
            success: true,
            settings: { enabled: true, ambientSeconds: 0 },
            guilds: { 'guild-1': { channelIds: ['1234567890'] } },
            hasApiKey: true,
        }),
    chatbotSave: (patch) => ok({ success: true, settings: patch.settings || {}, guilds: {}, hasApiKey: true, sync: [] }),
    chatbotClearMemory: () => ok({ success: true }),
    moduleDetect: () => ok({ matches: [{ name: 'minecraft', display_name: 'Minecraft: Java Edition' }] }),
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
            moduleAliases: { minecraft: '마크,mc' },
            commandAliases: {},
            instanceAliases: { 'srv-1': '야생' },
            nodeSettings: {
                'guild-1': {
                    allowedInstances: ['srv-1'],
                    memberPermissions: { 'u-3': { 'srv-1': ['stop', 'save-all'] } },
                },
            },
        }),
    moduleGetMetadata: (name) => {
        const mod = modules.find((m) => m.name === name);
        const aliases = { minecraft: ['minecraft', 'mc'], palworld: ['pal', '팰월드'], zomboid: ['좀보이드'] };
        return ok({
            metadata: {
                module_aliases: aliases[name] || [],
                command_aliases: {
                    start: { aliases: ['시작', '켜기'], description: '서버 시작' },
                    stop: { aliases: ['정지', '끄기'], description: '서버 정지' },
                    status: { aliases: ['상태'], description: '상태 보기' },
                },
                commands: mod?.commands || { fields: [] },
            },
        });
    },
    instanceStop: (id) => {
        servers = servers.map((s) => (s.id === id ? { ...s, status: 'stopped', start_time: null } : s));
        return ok({ success: true });
    },
    instanceRestart: (id) => {
        servers = servers.map((s) => (s.id === id ? { ...s, status: 'running', start_time: now() } : s));
        return ok({ success: true, stopped: true });
    },
    instanceRename: (id, name) => {
        if (servers.some((s) => s.name === name && s.id !== id))
            return ok({ success: false, error_code: 'name_taken' });
        servers = servers.map((s) => (s.id === id ? { ...s, name } : s));
        return ok({ success: true, id, name });
    },
    // 부팅 뒤 몇 초 동안 Python 준비 진행을 흉내 낸다
    runtimePreflight: () => {
        const started = (globalThis.__mockPreflightStart ??= Date.now());
        const percent = Math.min(100, Math.round(((Date.now() - started) / 6000) * 100));
        const done = percent >= 100;
        return ok({
            phase: done ? 'preflight-done' : 'preflight-python',
            percent,
            complete: done,
            error: null,
            components: { python: done ? 'ok' : 'running' },
        });
    },
    instanceCommands: (id) => {
        const srv = servers.find((s) => s.id === id);
        const mod = modules.find((m) => m.name === srv?.module);
        return ok({
            commands: [
                ...(mod?.commands?.fields || []).map((f) => ({ ...f, risk: f.risk || 'caution', source: 'module' })),
                {
                    name: 'backup-now',
                    label: '지금 백업',
                    risk: 'caution',
                    source: 'extension',
                    extension: 'world-backup',
                },
            ],
        });
    },
    configReset: () => ok({ ok: true, config: {} }),
    relayListHostNodes: () =>
        ok([
            { guildId: 'guild-1', guildName: '사바 마을' },
            { guildId: 'guild-2', guildName: '주말 게임방' },
        ]),
    relayListNodeMembers: (guildId) =>
        ok(
            guildId === 'guild-1'
                ? [
                      { id: 'u-1', username: 'wareaoba', displayName: '아오바' },
                      { id: 'u-2', username: 'mint', displayName: '민트' },
                      { id: 'u-3', username: 'newbie01', displayName: '뉴비' },
                      { id: 'u-4', username: 'sabafan', displayName: '사바팬' },
                  ]
                : [{ id: 'u-1', username: 'wareaoba', displayName: '아오바' }],
        ),
    relayBotInvite: () => ok({ url: 'https://discord.com/oauth2/authorize?client_id=0&scope=bot' }),
    relayInitiatePairing: () =>
        ok({ code: 'K7Q2-M9', expiresAt: new Date(Date.now() + 297000).toISOString(), pollSecret: 'x' }),
    relayPollPairingStatus: () => ok({ status: 'pending' }),
    discordBotStatus: () => ok(mockBot === 'stopped' ? 'stopped' : 'running'),
    relayCheckHostStatus: () => ok(mockBot === 'running' ? { online: true } : { error: 'offline' }),
    daemonStatus: () => ok({ running: mockDaemonUp }),
    updaterLastResult: ({ clear } = {}) =>
        ok({
            ok: true,
            result:
                !mockUpdate || clear
                    ? null
                    : {
                          timestamp: now(),
                          success: mockUpdate === 'ok',
                          updated_components: mockUpdate === 'ok' ? ['saba-core', 'saba-chan-gui'] : ['saba-chan-gui'],
                          failed: mockUpdate === 'ok' ? [] : ['saba-core'],
                          message: mockUpdate === 'ok' ? '' : '데몬 파일을 교체하지 못했어요 (사용 중)',
                      },
        }),
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
