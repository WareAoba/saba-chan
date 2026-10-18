/**
 * 개발용 가짜 window.api — Electron 없이 `npm run dev`로 브라우저에서 화면을 볼 때만 쓴다.
 * index.js가 DEV 빌드이고 window.api가 없을 때만 설치한다. 프로덕션 번들에는 들어가지 않는다.
 */
const mockChannels = {
    chatbot: ['111'],
    music: null,
    notify: null,
    botAvailable: true,
    channels: [
        { id: '111', name: '잡담', category: '채팅' },
        { id: '112', name: '질문', category: '채팅' },
        { id: '221', name: '음악', category: '놀이' },
        { id: '222', name: '공지', category: '' },
    ],
};
const now = () => Math.floor(Date.now() / 1000);

const modules = [
    {
        name: 'minecraft',
        game_name: 'Minecraft',
        display_name: 'Minecraft: Java Edition',
        version: '1.2.0',
        trust: 'verified',
        description: 'Minecraft Java 서버',
        interaction_mode: 'console',
        protocols: { supported: ['stdin'], default: 'stdin' },
        agreements: [{ id: 'eula', label: 'Minecraft EULA', url: 'https://aka.ms/MinecraftEULA' }],
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
        trust: 'modified',
        description: 'Palworld 전용 서버',
        commands: { fields: [{ name: 'players', label: '접속자', method: 'rest' }] },
        settings: {
            fields: [{ name: 'port', label: '포트', type: 'number', field_type: 'number', default: 8211 }],
        },
    },
    {
        name: 'zomboid',
        game_name: 'Project Zomboid',
        display_name: 'Project Zomboid',
        version: '1.0.0',
        trust: 'unverified',
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
        // 데몬이 판정한 익스텐션 데이터 slot (Q-7)
        slots: {
            'ServerCard.badge': [{ extension: 'backup', text: '💾' }],
            'ServerDetail.status': [{ extension: 'backup', label: '마지막 백업', value: '2시간 전' }],
        },
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

const mockSpans = (text) => {
    const found = [];
    const add = (re, token, group = 0) => {
        for (const m of text.matchAll(re)) {
            const start = m.index + m[0].indexOf(m[group]);
            found.push([start, start + m[group].length, token]);
        }
    };
    add(/^\[\d{2}:\d{2}:\d{2}\]/g, 'timestamp');
    add(/\[([^\]]+\/INFO)\]/g, 'dim', 1);
    add(/\[([^\]]+\/WARN)\]/g, 'warn', 1);
    add(/"[^"]*"/g, 'string');
    add(/\d+(?:\.\d+)?s\b/g, 'number');
    add(/Done/g, 'success');
    add(/(\w+) joined the game/g, 'player', 1);
    found.sort((a, b) => a[0] - b[0]);
    return found.filter((s, i) => i === 0 || s[0] >= found[i - 1][1]);
};

const ok = (v = {}) => Promise.resolve(v);

const mockBotSettings = {
    locale: 'auto',
    sleeping: false,
    music: { enabled: true, ui: true, queueLines: 10, refreshInterval: 5000, normalize: true },
};

const mockPerms = {};
const mockPermissions = (guildId) => {
    mockPerms[guildId] ??= {
        guildId,
        ownerDiscordId: 'u-1',
        relayOwnerDiscordId: guildId === 'guild-1' ? 'u-9' : null,
        defaultLevel: 1,
        preset: 'default',
        chatbotCap: 0,
        members: { 'u-2': 2, 'u-5': -1 },
        commandLevels: {},
        presets: {
            loose: { safe: 0, caution: 0, danger: 1 },
            default: { safe: 0, caution: 1, danger: 2 },
            strict: { safe: 1, caution: 2, danger: 3 },
        },
    };
    return mockPerms[guildId];
};
// 주소 뒤에 ?daemon=stopped&bot=running 처럼 붙이면 상태 표시를 바꿔 볼 수 있다
const params = new URLSearchParams(globalThis.location?.search || '');
const mockBot = params.get('bot') || 'stopped';
const mockDaemonUp = params.get('daemon') !== 'stopped';
const mockUpdate = params.get('update'); // ok | fail
const mockNoServers = params.get('servers') === '0'; // 빈 화면 보기
const mockFailed = params.get('failed') // 불러오지 못한 모듈 보기
    ? [{ path: 'modules/terraria', error: "module.toml: unknown field 'protocl' (did you mean 'protocol'?)" }]
    : [];
const settings = {};

const impl = {
    serverList: () => ok({ servers: mockNoServers ? [] : servers.map((s) => ({ ...s })) }),
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
            guilds: { 'guild-1': { memo: '' } },
            hasApiKey: true,
        }),
    chatbotSave: (patch) =>
        ok({ success: true, settings: patch.settings || {}, guilds: {}, hasApiKey: true, sync: [] }),
    chatbotClearMemory: () => ok({ success: true }),
    instanceValidate: () =>
        ok({
            success: false,
            issues: [
                {
                    code: 'JAVA_VERSION_TOO_OLD',
                    severity: 'critical',
                    message: 'Java 17은 이 버전에 너무 오래됐습니다',
                    solution: 'Java 21을 설치하세요',
                },
                {
                    code: 'LOW_RAM',
                    severity: 'warning',
                    message: '할당 메모리가 적습니다',
                    solution: '2GB 이상을 권장합니다',
                },
            ],
        }),
    instanceDiagnose: () => ok({ success: true, issues: [] }),
    daemonDiagnostics: () =>
        ok({
            ok: true,
            version: '0.1.5',
            pid: 4242,
            uptime_secs: 3725,
            listen_addr: '127.0.0.1:57474',
            data_dir: 'C:/Users/me/AppData/Roaming/saba-chan',
            token_file: { path: 'C:/Users/me/AppData/Roaming/saba-chan/.ipc_token', exists: true },
            extensions: { enabled: ['steamcmd'], enabled_but_missing: [] },
            modules: { count: 3 },
            recent_problems: [
                {
                    id: 1,
                    timestamp: Date.now() - 60000,
                    level: 'warn',
                    target: 'saba',
                    message: '디스크 응답이 느립니다',
                },
            ],
        }),
    migrationScanDir: () => ok({ ok: true, matches: [{ name: 'minecraft', display_name: 'Minecraft: Java Edition' }] }),
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
    // 시스템 모니터: 데몬 /api/system/metrics 모양 (값은 조금씩 흔들린다)
    systemMetrics: () => {
        const jitter = (base, spread) => Math.max(0, base + (Math.random() - 0.5) * spread);
        const GB = 1024 ** 3;
        const MB = 1024 ** 2;
        const running = servers.filter((s) => s.status === 'running' || s.status === 'starting');
        const instances = servers.map((s, i) => {
            const on = running.includes(s);
            return {
                id: s.id,
                name: s.name,
                running: on,
                process_count: on ? 2 : 0,
                cpu_percent: on ? jitter(18 - i * 6, 6) : 0,
                memory_bytes: on ? (4.2 - i) * GB : 0,
                disk_read_bps: on ? jitter(2 * MB, 2 * MB) : 0,
                disk_write_bps: on ? jitter(1.5 * MB, 2 * MB) : 0,
            };
        });
        const instCpu = instances.reduce((a, i) => a + i.cpu_percent, 0);
        return ok({
            sampled_at_ms: Date.now(),
            window_ms: 2000,
            cpu: { cores: 16, total_percent: instCpu + jitter(14, 6), others_percent: jitter(14, 6) },
            memory: { total_bytes: 32 * GB, used_bytes: 19.6 * GB, others_bytes: 19.6 * GB - 6.4 * GB },
            disk_io: {
                read_bps: jitter(0.3 * MB, 0.4 * MB),
                write_bps: jitter(0.2 * MB, 0.3 * MB),
                source: 'physical',
                per_instance: false,
            },
            network: { rx_bps: jitter(3.2 * MB, 2 * MB), tx_bps: jitter(1.1 * MB, 1 * MB), per_instance: false },
            disks: [
                {
                    mount: 'C:\\',
                    total_bytes: 931 * GB,
                    available_bytes: 312 * GB,
                    others_bytes: 931 * GB - 312 * GB - 48 * GB,
                    instances: [
                        { id: 'srv-1', bytes: 21 * GB },
                        { id: 'srv-2', bytes: 9 * GB },
                        { id: 'srv-3', bytes: 14 * GB },
                        { id: 'srv-4', bytes: 4 * GB },
                    ],
                    sizes_at_ms: Date.now() - 30000,
                },
            ],
            instances,
        });
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
    dataExport: ({ secrets } = {}) =>
        ok({
            success: true,
            path: 'C:\\Users\\me\\saba-chan-export.zip',
            secrets: !!secrets,
            instances: [{ id: 'mc-1', name: '마크 서버' }],
            config: ['gui', 'chatbot'],
        }),
    dataImport: ({ path, dryRun, overwrite } = {}) =>
        ok({
            success: true,
            dryRun: !!dryRun,
            appVersion: '0.1.3',
            secrets: true,
            instances: {
                added: [{ id: 'pal-1', name: '팰월드' }],
                replaced: overwrite ? [{ id: 'mc-1', name: '마크 서버' }] : [],
                skipped: overwrite ? [] : [{ id: 'mc-1', name: '마크 서버', reason: 'exists' }],
            },
            missingModules: ['zomboid'],
            config: ['gui', 'chatbot'],
            nodeToken: 'kept',
            path: path || 'C:\\Users\\me\\saba-chan-export.zip',
        }),
    auditList: ({ instance } = {}) =>
        ok({
            entries: [
                {
                    ts: now() - 30,
                    source: 'discord',
                    user: '284019384710293847',
                    userName: '김치',
                    guild: '1029384756102938475',
                    action: 'command',
                    instanceId: 'mc-1',
                    instance: '마크 서버',
                    command: 'say',
                    args: '["안녕"]',
                    ok: true,
                    status: 200,
                },
                {
                    ts: now() - 600,
                    source: 'gui',
                    action: 'restart',
                    instanceId: 'mc-1',
                    instance: '마크 서버',
                    ok: true,
                    status: 200,
                },
                {
                    ts: now() - 3600,
                    source: 'schedule',
                    action: 'stop',
                    instanceId: 'pal-1',
                    instance: '팰월드',
                    ok: false,
                    status: 500,
                },
                {
                    ts: now() - 7200,
                    source: 'cli',
                    action: 'start',
                    instanceId: 'mc-1',
                    instance: '마크 서버',
                    ok: true,
                    status: 200,
                },
            ].filter((e) => !instance || e.instanceId === instance),
        }),
    daemonConsole: ({ since } = {}) => {
        const msgs = [
            ['info', 'IPC server listening on 127.0.0.1:57474'],
            ['info', '[Relay] heartbeat ok (2 guilds)'],
            ['info', '[Audit] discord 김치 → 마크 서버 command say'],
            ['warn', '[Monitor] PDH counter "Disk Bytes/sec" not ready, retrying'],
            ['info', '[Supervisor] 마크 서버 stdin: say 안녕'],
            ['error', '[Schedule] 팰월드 stop failed: process not running'],
        ];
        const all = msgs.map(([level, message], i) => ({
            id: i + 1,
            timestamp: Date.now() - (msgs.length - i) * 4000,
            level,
            target: 'saba_core',
            message,
        }));
        return ok({ lines: since === undefined ? all : all.filter((l) => l.id >= since) });
    },
    instanceLaunchPreview: (id, draft = {}, extra) => {
        if (id !== 'srv-2') return ok({ success: true, has_launch_args: false });
        // undefined·null이면 저장된 추가 인자로 계산한다 (데몬과 같다)
        const text = extra ?? '-useperfthreads -NoAsyncLoadingThread';
        const args = String(text).split(/\s+/).filter(Boolean);
        const portArg = args.find((a) => a.startsWith('-port='));
        const port = portArg ? portArg.slice(6) : draft.port;
        const bad = portArg && !/^\d+$/.test(port);
        const rest = args.filter((a) => a !== portArg);
        return ok({
            success: true,
            has_launch_args: true,
            command_line: ['PalServer.exe', `-port=${port ?? 8211}`, '-adminpassword=***', ...rest].join(' '),
            settings_from_args: portArg && !bad ? { port: Number(port) } : {},
            extra_args: rest,
            extra_args_text: text,
            errors: bad ? [{ arg: portArg, field: 'port', expected: 'port', got: port }] : [],
        });
    },
    relayNodeChannels: (guildId) => ok({ guildId, ...mockChannels }),
    relayNodeChannelsSave: (guildId, patch) => {
        Object.assign(mockChannels, patch);
        return ok({ guildId, ...mockChannels });
    },
    // 터미널로 열기 (I4: POST /api/clients/launch)
    openInTerminal: () => ok({ success: true, pid: 4242 }),
    // 데몬 이벤트 스트림 (P4): 잠시 뒤 디스코드에서 실행한 명령 한 건
    onDaemonEvent: (cb) => {
        const timer = setTimeout(
            () =>
                cb({
                    id: 1,
                    type: 'action.run',
                    data: {
                        at: Math.floor(Date.now() / 1000),
                        source: 'discord',
                        userName: '김치',
                        guildName: '사바 마을',
                        action: 'stop',
                        instanceId: 'srv-1',
                        instance: '야생 서버',
                        ok: true,
                        status: 200,
                    },
                }),
            2500,
        );
        return () => clearTimeout(timer);
    },
    // 디스코드 봇 취향 설정 (I2: GET/PATCH /api/discord/:guild/settings)
    discordSettingsGet: (guildId) => ok({ guildId, ...mockBotSettings, sync: { ok: true } }),
    discordSettingsSet: (guildId, patch) => {
        const { music, ...rest } = patch;
        Object.assign(mockBotSettings, rest);
        if (music) Object.assign(mockBotSettings.music, music);
        return ok({ success: true, settings: { guildId, ...mockBotSettings }, sync: { ok: true } });
    },
    // 데몬 액션 카탈로그 (P2: GET /api/actions, POST …/:id/plan·run)
    actionsList: ({ instance } = {}) =>
        ok({
            actions: [
                { id: 'instance.list', kind: 'query', risk: 'safe', scope: 'none', title: '서버 목록' },
                { id: 'monitor.summary', kind: 'query', risk: 'safe', scope: 'none', title: '시스템 요약' },
                { id: 'server.start', kind: 'command', risk: 'caution', scope: 'instance', title: '서버 시작' },
                { id: 'server.stop', kind: 'command', risk: 'caution', scope: 'instance', title: '서버 정지' },
                { id: 'server.restart', kind: 'command', risk: 'caution', scope: 'instance', title: '서버 재시작' },
                ...(instance
                    ? [
                          {
                              id: 'server.command:say',
                              kind: 'command',
                              risk: 'caution',
                              scope: 'instance',
                              title: '전체 메시지',
                              input: {
                                  type: 'object',
                                  properties: { message: { type: 'string', title: '내용' } },
                                  required: ['message'],
                              },
                          },
                      ]
                    : []),
                { id: 'backup.create', kind: 'command', risk: 'caution', scope: 'instance', title: '백업 만들기' },
                { id: 'instance.delete', kind: 'admin', risk: 'danger', scope: 'instance', title: '서버 삭제' },
            ],
        }),
    actionsPlan: (id, { instance } = {}) =>
        ok({
            planId: `plan-${id}`,
            actionId: id,
            risk: id === 'instance.delete' ? 'danger' : 'caution',
            needsConfirm: id === 'instance.delete',
            summary: [
                { labelKey: 'instance', label: '서버', value: instance || '-' },
                { labelKey: 'effect', label: '하는 일', value: '서버 폴더와 설정을 지웁니다' },
            ],
        }),
    actionsRun: (id) =>
        ok({
            success: true,
            result: id === 'instance.list' ? { servers: ['야생 서버', '팰월드 친구들'] } : { message: 'ok' },
        }),
    // 데몬 P1 권한 (GET/PATCH /api/permissions/:guildId, PUT/DELETE …/members/:id)
    permissionsGet: (guildId) => ok(mockPermissions(guildId)),
    permissionsUpdate: (guildId, patch) => {
        const p = mockPermissions(guildId);
        const { commandLevels, ...rest } = patch || {};
        Object.assign(p, rest);
        for (const [k, v] of Object.entries(commandLevels || {})) {
            if (v === null) delete p.commandLevels[k];
            else p.commandLevels[k] = v;
        }
        if (rest.ownerDiscordId) p.relayOwnerDiscordId = null;
        return ok(p);
    },
    permissionsSetMember: (guildId, id, level) => {
        const p = mockPermissions(guildId);
        if (id === p.ownerDiscordId) return ok({ error: 'owner', error_code: 'owner_fixed' });
        p.members[id] = level;
        return ok(p);
    },
    permissionsRemoveMember: (guildId, id) => {
        delete mockPermissions(guildId).members[id];
        return ok({ success: true });
    },
    relayListNodeMembers: (guildId) =>
        ok(
            guildId === 'guild-1'
                ? [
                      { id: 'u-1', username: 'wareaoba', displayName: '아오바', level: 2 },
                      { id: 'u-2', username: 'mint', displayName: '민트', level: 2 },
                      { id: 'u-3', username: 'newbie01', displayName: '뉴비', level: 0 },
                      { id: 'u-4', username: 'sabafan', displayName: '사바팬', level: 1 },
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
            version: '0.1.3',
            notes: '- 디스코드 서버별 챗봇·음악 채널 배정\n- 업데이트 결과 알림과 패치노트',
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
    // spans를 달라고 하면(하이라이트 켬) 데몬처럼 조각 토큰을 붙인다: 시각·스레드/레벨·따옴표·숫자·플레이어
    managedConsole: (_id, since = 0, _count, opts) =>
        ok({
            lines: consoleLines
                .map((content, i) => ({
                    id: i + 1,
                    content,
                    source: 'stdout',
                    level: content.includes('/WARN]') ? 'warn' : 'info',
                    ...(opts?.spans ? { spans: mockSpans(content) } : {}),
                }))
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
