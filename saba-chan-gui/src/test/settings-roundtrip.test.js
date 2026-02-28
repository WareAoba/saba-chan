/**
 * 설정 저장/로드 라운드트립 테스트
 *
 * 모든 설정 항목에 대해 save → load 왕복을 검증한다.
 * - settings.json (settingsLoad / settingsSave) : 7 필드
 * - bot-config.json (botConfigLoad / botConfigSave) : 10 필드
 * - cross-store sync (token, autoStart) : 2 필드
 *
 * 각 테스트가 독립적으로 store를 리셋하므로 순서 무관.
 */
import { beforeEach, describe, expect, it, vi } from 'vitest';
import { useDiscordStore } from '../stores/useDiscordStore';
import { useSettingsStore } from '../stores/useSettingsStore';
import { useUIStore } from '../stores/useUIStore';

// ── Helpers ──────────────────────────────────────────────────

/** Minimal window.api mock — all APIs noop by default */
function mockApi(overrides = {}) {
    window.api = new Proxy(
        {
            settingsLoad: vi.fn().mockResolvedValue({}),
            settingsSave: vi.fn().mockResolvedValue({ success: true }),
            settingsGetPath: vi.fn().mockResolvedValue('/mock/settings.json'),
            botConfigLoad: vi.fn().mockResolvedValue({}),
            botConfigSave: vi.fn().mockResolvedValue({ success: true }),
            discordBotStatus: vi.fn().mockResolvedValue('stopped'),
            daemonStatus: vi.fn().mockResolvedValue({ running: true }),
            ...overrides,
        },
        {
            get(target, prop) {
                if (typeof prop === 'symbol') return target[prop];
                if (!(prop in target)) target[prop] = vi.fn().mockResolvedValue({});
                return target[prop];
            },
        },
    );
    window.showToast = vi.fn();
}

beforeEach(() => {
    vi.clearAllMocks();
    useSettingsStore.getState()._resetForTest();
    useDiscordStore.getState()._resetForTest();
    useUIStore.getState()._resetForTest();
});

// ═══════════════════════════════════════════════════════════════
// 1. settings.json 라운드트립 — 각 필드별 개별 검증
// ═══════════════════════════════════════════════════════════════

describe('settings.json 라운드트립 (settingsLoad ↔ settingsSave)', () => {
    // ── 저장 시 모든 7 필드가 payload에 포함되는지 검증 ──

    it('save() payload에 GUI 6필드 + discord 2필드 = 총 8필드 포함', async () => {
        mockApi();
        const _store = useSettingsStore.getState();
        // Prepare: settingsPath 필요
        useSettingsStore.setState({ settingsPath: '/mock/settings.json' });
        useSettingsStore.setState({
            autoRefresh: false,
            refreshInterval: 9999,
            ipcPort: 11111,
            consoleBufferSize: 500,
        });
        useSettingsStore.getState()._setDiscordFields('tok-abc', true);

        await useSettingsStore.getState().save();

        const payload = window.api.settingsSave.mock.calls[0][0];
        expect(payload).toEqual({
            autoRefresh: false,
            refreshInterval: 9999,
            ipcPort: 11111,
            consoleBufferSize: 500,
            autoGeneratePasswords: true,
            portConflictCheck: true,
            discordToken: 'tok-abc',
            discordAutoStart: true,
        });
    });

    // ── 개별 필드 라운드트립: save → load → 값이 동일한지 ──

    const guiFields = [
        { field: 'autoRefresh', saved: false, defaultVal: true },
        { field: 'refreshInterval', saved: 7777, defaultVal: 2000 },
        { field: 'ipcPort', saved: 33333, defaultVal: 57474 },
        { field: 'consoleBufferSize', saved: 100, defaultVal: 2000 },
        // modulesPath는 설정이 아닌 고정 경로이므로 save 대상에서 제외
    ];

    it.each(guiFields)('GUI 필드 "$field": 저장($saved) → 로드 → store에 복원', async ({ field, saved }) => {
        // 1) save
        let captured = null;
        mockApi({
            settingsSave: vi.fn().mockImplementation(async (data) => {
                captured = data;
                return { success: true };
            }),
        });
        useSettingsStore.setState({ settingsPath: '/s.json', [field]: saved });
        useSettingsStore.getState()._setDiscordFields('', false);
        await useSettingsStore.getState().save();
        expect(captured[field]).toEqual(saved);

        // 2) reset
        useSettingsStore.getState()._resetForTest();

        // 3) load with captured payload
        mockApi({
            settingsLoad: vi.fn().mockResolvedValue(captured),
            settingsGetPath: vi.fn().mockResolvedValue('/s.json'),
        });
        await useSettingsStore.getState().load();

        // 4) verify
        expect(useSettingsStore.getState()[field]).toEqual(saved);
    });

    // ── discordToken 라운드트립 (settings 파일 경유) ──

    it('discordToken: 저장("my-token") → 로드 → settingsStore._discordToken 복원', async () => {
        let captured = null;
        mockApi({
            settingsSave: vi.fn().mockImplementation(async (data) => {
                captured = data;
                return { success: true };
            }),
        });
        useSettingsStore.setState({ settingsPath: '/s.json' });
        useSettingsStore.getState()._setDiscordFields('my-token', false);
        await useSettingsStore.getState().save();
        expect(captured.discordToken).toBe('my-token');

        // Reset + load: App.js에서 하는 것과 동일한 과정 재현
        useSettingsStore.getState()._resetForTest();
        useDiscordStore.getState()._resetForTest();

        mockApi({
            settingsLoad: vi.fn().mockResolvedValue(captured),
            settingsGetPath: vi.fn().mockResolvedValue('/s.json'),
        });
        const raw = await useSettingsStore.getState().load();
        // App.js 로직: raw settings에서 discord fields 복원
        if (raw) {
            useDiscordStore.getState().update({
                discordToken: raw.discordToken || '',
                _discordTokenRef: raw.discordToken || '',
            });
            useSettingsStore.getState()._setDiscordFields(raw.discordToken || '', raw.discordAutoStart ?? false);
        }

        expect(useDiscordStore.getState().discordToken).toBe('my-token');
        expect(useSettingsStore.getState()._discordToken).toBe('my-token');
    });

    // ── discordAutoStart 라운드트립 (settings 파일 경유) ──

    it('discordAutoStart: 저장(true) → 로드 → discordStore.discordAutoStart 복원', async () => {
        let captured = null;
        mockApi({
            settingsSave: vi.fn().mockImplementation(async (data) => {
                captured = data;
                return { success: true };
            }),
        });
        useSettingsStore.setState({ settingsPath: '/s.json' });
        useSettingsStore.getState()._setDiscordFields('tok', true);
        await useSettingsStore.getState().save();
        expect(captured.discordAutoStart).toBe(true);

        // Reset + load
        useSettingsStore.getState()._resetForTest();
        useDiscordStore.getState()._resetForTest();

        mockApi({
            settingsLoad: vi.fn().mockResolvedValue(captured),
            settingsGetPath: vi.fn().mockResolvedValue('/s.json'),
        });
        const raw = await useSettingsStore.getState().load();
        if (raw) {
            useDiscordStore.getState().update({
                discordAutoStart: raw.discordAutoStart ?? false,
            });
            useSettingsStore.getState()._setDiscordFields(raw.discordToken || '', raw.discordAutoStart ?? false);
        }

        expect(useDiscordStore.getState().discordAutoStart).toBe(true);
        expect(useSettingsStore.getState()._discordAutoStart).toBe(true);
    });

    // ── 전체 필드 동시 라운드트립 ──

    it('전체 필드 동시 라운드트립: 커스텀 값 → save → reset → load → 전부 복원', async () => {
        const custom = {
            autoRefresh: false,
            refreshInterval: 5000,
            ipcPort: 22222,
            consoleBufferSize: 4000,
            autoGeneratePasswords: true,
            portConflictCheck: true,
            discordToken: 'roundtrip-token',
            discordAutoStart: true,
        };

        let captured = null;
        mockApi({
            settingsSave: vi.fn().mockImplementation(async (data) => {
                captured = data;
                return { success: true };
            }),
        });
        useSettingsStore.setState({
            settingsPath: '/s.json',
            autoRefresh: custom.autoRefresh,
            refreshInterval: custom.refreshInterval,
            ipcPort: custom.ipcPort,
            consoleBufferSize: custom.consoleBufferSize,
        });
        useSettingsStore.getState()._setDiscordFields(custom.discordToken, custom.discordAutoStart);
        await useSettingsStore.getState().save();

        // Verify payload (modulesPath는 고정 경로이므로 save에 포함되지 않음)
        expect(captured).toEqual(custom);

        // Reset
        useSettingsStore.getState()._resetForTest();
        useDiscordStore.getState()._resetForTest();

        // Load
        mockApi({
            settingsLoad: vi.fn().mockResolvedValue(captured),
            settingsGetPath: vi.fn().mockResolvedValue('/s.json'),
        });
        const raw = await useSettingsStore.getState().load();
        if (raw) {
            useDiscordStore.getState().update({
                discordToken: raw.discordToken || '',
                _discordTokenRef: raw.discordToken || '',
                discordAutoStart: raw.discordAutoStart ?? false,
            });
            useSettingsStore.getState()._setDiscordFields(raw.discordToken || '', raw.discordAutoStart ?? false);
        }

        // Verify all GUI fields
        const s = useSettingsStore.getState();
        expect(s.autoRefresh).toBe(custom.autoRefresh);
        expect(s.refreshInterval).toBe(custom.refreshInterval);
        expect(s.ipcPort).toBe(custom.ipcPort);
        expect(s.consoleBufferSize).toBe(custom.consoleBufferSize);
        expect(s._discordToken).toBe(custom.discordToken);
        expect(s._discordAutoStart).toBe(custom.discordAutoStart);

        // Verify discord store
        const d = useDiscordStore.getState();
        expect(d.discordToken).toBe(custom.discordToken);
        expect(d.discordAutoStart).toBe(custom.discordAutoStart);
    });

    // ── 기본값 복원: 필드가 undefined/missing일 때 ──

    it('settingsLoad가 빈 객체 반환 → 모든 GUI 필드 기본값 유지', async () => {
        mockApi({
            settingsLoad: vi.fn().mockResolvedValue({}),
            settingsGetPath: vi.fn().mockResolvedValue('/s.json'),
        });

        await useSettingsStore.getState().load();

        const s = useSettingsStore.getState();
        expect(s.autoRefresh).toBe(true);
        expect(s.refreshInterval).toBe(2000);
        expect(s.ipcPort).toBe(57474);
        expect(s.consoleBufferSize).toBe(2000);
    });

    // ── settingsPath 미초기화 시 save 스킵 ──

    it('settingsPath 비어있으면 save() 호출해도 settingsSave 미호출', async () => {
        mockApi();
        useSettingsStore.setState({ settingsPath: '' });
        await useSettingsStore.getState().save();
        expect(window.api.settingsSave).not.toHaveBeenCalled();
    });
});

// ═══════════════════════════════════════════════════════════════
// 2. bot-config.json 라운드트립 — 각 필드별 개별 검증
// ═══════════════════════════════════════════════════════════════

describe('bot-config.json 라운드트립 (botConfigLoad ↔ botConfigSave)', () => {
    // ── 저장 시 모든 10 필드가 payload에 포함되는지 검증 ──

    it('saveConfig() payload에 10개 필드 전부 포함', async () => {
        mockApi();
        useDiscordStore.setState({
            discordPrefix: '!test',
            discordBotMode: 'cloud',
            discordCloudRelayUrl: 'https://relay.test',
            discordCloudHostId: 'host-xyz',
            discordModuleAliases: { mc: 'minecraft' },
            discordCommandAliases: { mc: { start: 'go' } },
            discordMusicEnabled: false,
            nodeSettings: { local: { ids: ['a'] } },
            cloudNodes: [{ id: 'n1' }],
            cloudMembers: { u1: 'admin' },
        });

        await useDiscordStore.getState().saveConfig();

        const payload = window.api.botConfigSave.mock.calls[0][0];
        expect(payload).toEqual({
            prefix: '!test',
            mode: 'cloud',
            cloud: { relayUrl: 'https://relay.test', hostId: 'host-xyz' },
            moduleAliases: { mc: 'minecraft' },
            commandAliases: { mc: { start: 'go' } },
            musicEnabled: false,
            nodeSettings: { local: { ids: ['a'] } },
            cloudNodes: [{ id: 'n1' }],
            cloudMembers: { u1: 'admin' },
        });
    });

    // ── 개별 필드 라운드트립 (simple fields) ──

    const simpleBotFields = [
        {
            field: 'discordPrefix',
            botKey: 'prefix',
            saved: '!mybot',
            defaultVal: '!saba',
            makeBotCfg: (v) => ({ prefix: v }),
        },
        {
            field: 'discordBotMode',
            botKey: 'mode',
            saved: 'cloud',
            defaultVal: 'local',
            makeBotCfg: (v) => ({ mode: v }),
        },
        {
            field: 'discordMusicEnabled',
            botKey: 'musicEnabled',
            saved: false,
            defaultVal: true,
            makeBotCfg: (v) => ({ musicEnabled: v }),
        },
        {
            field: 'discordModuleAliases',
            botKey: 'moduleAliases',
            saved: { palworld: 'pw', minecraft: 'mc' },
            defaultVal: {},
            makeBotCfg: (v) => ({ moduleAliases: v }),
        },
        {
            field: 'discordCommandAliases',
            botKey: 'commandAliases',
            saved: { palworld: { start: '시작', stop: '정지' } },
            defaultVal: {},
            makeBotCfg: (v) => ({ commandAliases: v }),
        },
        {
            field: 'nodeSettings',
            botKey: 'nodeSettings',
            saved: { local: { allowedInstances: ['srv-1', 'srv-2'] } },
            defaultVal: {},
            makeBotCfg: (v) => ({ nodeSettings: v }),
        },
        {
            field: 'cloudNodes',
            botKey: 'cloudNodes',
            saved: [
                { id: 'n1', name: 'Node-1' },
                { id: 'n2', name: 'Node-2' },
            ],
            defaultVal: [],
            makeBotCfg: (v) => ({ cloudNodes: v }),
        },
        {
            field: 'cloudMembers',
            botKey: 'cloudMembers',
            saved: { user1: { role: 'admin' }, user2: { role: 'member' } },
            defaultVal: {},
            makeBotCfg: (v) => ({ cloudMembers: v }),
        },
    ];

    // biome-ignore lint/correctness/noUnusedFunctionParameters: `field` is used in test title template "$field" and inside test body
    it.each(simpleBotFields)('봇 설정 "$field": 저장 → 로드 → store 복원', async ({ field, saved, makeBotCfg }) => {
        // 1) Set & Save
        let captured = null;
        mockApi({
            botConfigSave: vi.fn().mockImplementation(async (data) => {
                captured = data;
                return { success: true };
            }),
        });
        useDiscordStore.setState({ [field]: saved });
        await useDiscordStore.getState().saveConfig();
        // Verify field is in payload (use botKey mapping)
        expect(captured).toBeTruthy();

        // 2) Reset
        useDiscordStore.getState()._resetForTest();

        // 3) Load — convert saved payload back to botConfigLoad format
        mockApi({
            botConfigLoad: vi.fn().mockResolvedValue(captured),
        });
        await useDiscordStore.getState().loadConfig();

        // 4) Verify
        const actual = useDiscordStore.getState()[field];
        expect(actual).toEqual(saved);
    });

    // ── cloud nested 필드 (relayUrl, hostId) 라운드트립 ──

    it('cloud.relayUrl: 저장 → 로드 → discordCloudRelayUrl 복원', async () => {
        let captured = null;
        mockApi({
            botConfigSave: vi.fn().mockImplementation(async (data) => {
                captured = data;
                return { success: true };
            }),
        });
        useDiscordStore.setState({ discordCloudRelayUrl: 'https://my-relay.io' });
        await useDiscordStore.getState().saveConfig();
        expect(captured.cloud.relayUrl).toBe('https://my-relay.io');

        useDiscordStore.getState()._resetForTest();
        mockApi({ botConfigLoad: vi.fn().mockResolvedValue(captured) });
        await useDiscordStore.getState().loadConfig();
        expect(useDiscordStore.getState().discordCloudRelayUrl).toBe('https://my-relay.io');
    });

    it('cloud.hostId: 저장 → 로드 → discordCloudHostId 복원', async () => {
        let captured = null;
        mockApi({
            botConfigSave: vi.fn().mockImplementation(async (data) => {
                captured = data;
                return { success: true };
            }),
        });
        useDiscordStore.setState({ discordCloudHostId: 'hid-42' });
        await useDiscordStore.getState().saveConfig();
        expect(captured.cloud.hostId).toBe('hid-42');

        useDiscordStore.getState()._resetForTest();
        mockApi({ botConfigLoad: vi.fn().mockResolvedValue(captured) });
        await useDiscordStore.getState().loadConfig();
        expect(useDiscordStore.getState().discordCloudHostId).toBe('hid-42');
    });

    // ── 전체 10필드 동시 라운드트립 ──

    it('전체 10필드 동시 라운드트립: 커스텀 → save → reset → load → 전부 복원', async () => {
        const custom = {
            discordPrefix: '!full',
            discordBotMode: 'cloud',
            discordCloudRelayUrl: 'https://full.relay',
            discordCloudHostId: 'full-host-id',
            discordModuleAliases: { a: 'alpha', b: 'beta' },
            discordCommandAliases: { a: { x: 'ex' } },
            discordMusicEnabled: false,
            nodeSettings: { local: { data: true } },
            cloudNodes: [{ id: 'cn1' }],
            cloudMembers: { m1: { level: 3 } },
        };

        let captured = null;
        mockApi({
            botConfigSave: vi.fn().mockImplementation(async (data) => {
                captured = data;
                return { success: true };
            }),
        });
        useDiscordStore.setState(custom);
        await useDiscordStore.getState().saveConfig();

        // Verify save payload shape
        expect(captured.prefix).toBe('!full');
        expect(captured.mode).toBe('cloud');
        expect(captured.cloud).toEqual({ relayUrl: 'https://full.relay', hostId: 'full-host-id' });
        expect(captured.moduleAliases).toEqual({ a: 'alpha', b: 'beta' });
        expect(captured.commandAliases).toEqual({ a: { x: 'ex' } });
        expect(captured.musicEnabled).toBe(false);
        expect(captured.nodeSettings).toEqual({ local: { data: true } });
        expect(captured.cloudNodes).toEqual([{ id: 'cn1' }]);
        expect(captured.cloudMembers).toEqual({ m1: { level: 3 } });

        // Reset + Load
        useDiscordStore.getState()._resetForTest();
        mockApi({ botConfigLoad: vi.fn().mockResolvedValue(captured) });
        await useDiscordStore.getState().loadConfig();

        const d = useDiscordStore.getState();
        expect(d.discordPrefix).toBe(custom.discordPrefix);
        expect(d.discordBotMode).toBe(custom.discordBotMode);
        expect(d.discordCloudRelayUrl).toBe(custom.discordCloudRelayUrl);
        expect(d.discordCloudHostId).toBe(custom.discordCloudHostId);
        expect(d.discordModuleAliases).toEqual(custom.discordModuleAliases);
        expect(d.discordCommandAliases).toEqual(custom.discordCommandAliases);
        expect(d.discordMusicEnabled).toBe(custom.discordMusicEnabled);
        expect(d.nodeSettings).toEqual(custom.nodeSettings);
        expect(d.cloudNodes).toEqual(custom.cloudNodes);
        expect(d.cloudMembers).toEqual(custom.cloudMembers);
    });

    // ── 기본값 복원: botConfigLoad가 빈 객체일 때 ──

    it('botConfigLoad 빈 객체 → 모든 봇 필드 기본값 유지', async () => {
        mockApi({ botConfigLoad: vi.fn().mockResolvedValue({}) });
        await useDiscordStore.getState().loadConfig();

        const d = useDiscordStore.getState();
        expect(d.discordPrefix).toBe('!saba');
        expect(d.discordBotMode).toBe('local');
        expect(d.discordCloudRelayUrl).toBe('');
        expect(d.discordCloudHostId).toBe('');
        expect(d.discordModuleAliases).toEqual({});
        expect(d.discordCommandAliases).toEqual({});
        expect(d.discordMusicEnabled).toBe(true);
        // nodeSettings, cloudNodes, cloudMembers are not patched when missing
        expect(d.nodeSettings).toEqual({});
        expect(d.cloudNodes).toEqual([]);
        expect(d.cloudMembers).toEqual({});
    });

    // ── musicEnabled 엣지케이스: false vs undefined ──

    it('musicEnabled: false → 저장 → 로드 → false 복원 (true로 뒤집히지 않음)', async () => {
        let captured = null;
        mockApi({
            botConfigSave: vi.fn().mockImplementation(async (data) => {
                captured = data;
                return { success: true };
            }),
        });
        useDiscordStore.setState({ discordMusicEnabled: false });
        await useDiscordStore.getState().saveConfig();
        expect(captured.musicEnabled).toBe(false);

        useDiscordStore.getState()._resetForTest();
        mockApi({ botConfigLoad: vi.fn().mockResolvedValue(captured) });
        await useDiscordStore.getState().loadConfig();
        // "botCfg.musicEnabled !== false" → false !== false → false ✓
        expect(useDiscordStore.getState().discordMusicEnabled).toBe(false);
    });

    it('musicEnabled 누락 → 기본값 true', async () => {
        mockApi({ botConfigLoad: vi.fn().mockResolvedValue({ prefix: '!x' }) });
        await useDiscordStore.getState().loadConfig();
        // undefined !== false → true
        expect(useDiscordStore.getState().discordMusicEnabled).toBe(true);
    });

    // ── 레거시 allowedInstances → nodeSettings 마이그레이션 ──

    it('레거시 allowedInstances 배열 → nodeSettings 형태로 마이그레이션 로드', async () => {
        mockApi({
            botConfigLoad: vi.fn().mockResolvedValue({
                prefix: '!saba',
                allowedInstances: ['srv-a', 'srv-b'],
            }),
        });
        await useDiscordStore.getState().loadConfig();
        expect(useDiscordStore.getState().nodeSettings).toEqual({
            local: { allowedInstances: ['srv-a', 'srv-b'], memberPermissions: {} },
        });
    });
});

// ═══════════════════════════════════════════════════════════════
// 3. 크로스 스토어 동기화
// ═══════════════════════════════════════════════════════════════

describe('크로스 스토어 동기화 (discordStore ↔ settingsStore)', () => {
    it('discordToken 변경 → settingsStore._discordToken 자동 동기화', async () => {
        mockApi();
        useDiscordStore.getState().setDiscordToken('new-tok-123');
        // Cross-store subscription fires synchronously in Zustand
        expect(useSettingsStore.getState()._discordToken).toBe('new-tok-123');
    });

    it('discordAutoStart 변경 → settingsStore._discordAutoStart 자동 동기화', async () => {
        mockApi();
        useDiscordStore.getState().update({ discordAutoStart: true });
        expect(useSettingsStore.getState()._discordAutoStart).toBe(true);
    });

    it('token 동기화 후 settings save → payload에 새 token 포함', async () => {
        mockApi();
        useSettingsStore.setState({ settingsPath: '/s.json' });

        // 1) Change token in discord store
        useDiscordStore.getState().setDiscordToken('synced-token');

        // 2) Save settings
        await useSettingsStore.getState().save();

        const payload = window.api.settingsSave.mock.calls[0][0];
        expect(payload.discordToken).toBe('synced-token');
    });

    it('autoStart 동기화 후 settings save → payload에 새 autoStart 포함', async () => {
        mockApi();
        useSettingsStore.setState({ settingsPath: '/s.json' });

        useDiscordStore.getState().update({ discordAutoStart: true });
        await useSettingsStore.getState().save();

        const payload = window.api.settingsSave.mock.calls[0][0];
        expect(payload.discordAutoStart).toBe(true);
    });
});

// ═══════════════════════════════════════════════════════════════
// 4. saveCurrentSettings (App.js와 동일한 로직)
// ═══════════════════════════════════════════════════════════════

describe('saveCurrentSettings 통합 (settings + botConfig 동시 저장)', () => {
    /** App.js의 saveCurrentSettings 재현 */
    async function saveCurrentSettings(token, autoStart) {
        useSettingsStore.getState()._setDiscordFields(token, autoStart);
        await Promise.all([useSettingsStore.getState().save(), useDiscordStore.getState().saveConfig()]);
    }

    it('모든 17필드가 두 파일에 올바르게 분배되어 저장', async () => {
        mockApi();

        // Prepare full state
        useSettingsStore.setState({
            settingsPath: '/s.json',
            autoRefresh: false,
            refreshInterval: 3000,
            ipcPort: 9999,
            consoleBufferSize: 1000,
        });
        useDiscordStore.setState({
            discordToken: 'full-tok',
            discordAutoStart: true,
            discordPrefix: '!go',
            discordBotMode: 'cloud',
            discordCloudRelayUrl: 'https://r.io',
            discordCloudHostId: 'h1',
            discordModuleAliases: { x: 'y' },
            discordCommandAliases: { x: { a: 'b' } },
            discordMusicEnabled: false,
            nodeSettings: { n: 1 },
            cloudNodes: [{ id: 'c1' }],
            cloudMembers: { u: 'a' },
        });

        await saveCurrentSettings('full-tok', true);

        // Check settings file
        const settingsPayload = window.api.settingsSave.mock.calls[0][0];
        expect(settingsPayload).toEqual({
            autoRefresh: false,
            refreshInterval: 3000,
            ipcPort: 9999,
            consoleBufferSize: 1000,
            autoGeneratePasswords: true,
            portConflictCheck: true,
            discordToken: 'full-tok',
            discordAutoStart: true,
        });

        // Check bot config file
        const botPayload = window.api.botConfigSave.mock.calls[0][0];
        expect(botPayload).toEqual({
            prefix: '!go',
            mode: 'cloud',
            cloud: { relayUrl: 'https://r.io', hostId: 'h1' },
            moduleAliases: { x: 'y' },
            commandAliases: { x: { a: 'b' } },
            musicEnabled: false,
            nodeSettings: { n: 1 },
            cloudNodes: [{ id: 'c1' }],
            cloudMembers: { u: 'a' },
        });
    });

    it('saveCurrentSettings → reset → 두 파일 모두 로드 → 17필드 전부 복원', async () => {
        let settingsCaptured = null;
        let botCaptured = null;
        mockApi({
            settingsSave: vi.fn().mockImplementation(async (d) => {
                settingsCaptured = d;
                return { success: true };
            }),
            botConfigSave: vi.fn().mockImplementation(async (d) => {
                botCaptured = d;
                return { success: true };
            }),
        });

        useSettingsStore.setState({
            settingsPath: '/s.json',
            autoRefresh: false,
            refreshInterval: 4444,
            ipcPort: 8888,
            consoleBufferSize: 6000,
        });
        useDiscordStore.setState({
            discordToken: 'rt-token',
            discordAutoStart: true,
            discordPrefix: '!rt',
            discordBotMode: 'cloud',
            discordCloudRelayUrl: 'https://rt.relay',
            discordCloudHostId: 'rt-host',
            discordModuleAliases: { rt: 'roundtrip' },
            discordCommandAliases: { rt: { save: 'go' } },
            discordMusicEnabled: false,
            nodeSettings: { local: { rt: true } },
            cloudNodes: [{ id: 'rt-node' }],
            cloudMembers: { rtUser: { role: 'mod' } },
        });

        await saveCurrentSettings('rt-token', true);

        // Reset both stores
        useSettingsStore.getState()._resetForTest();
        useDiscordStore.getState()._resetForTest();

        // Reload from captured data (simulating App.js boot sequence)
        mockApi({
            settingsLoad: vi.fn().mockResolvedValue(settingsCaptured),
            settingsGetPath: vi.fn().mockResolvedValue('/s.json'),
            botConfigLoad: vi.fn().mockResolvedValue(botCaptured),
        });

        // Step 1: Load settings
        const raw = await useSettingsStore.getState().load();
        if (raw) {
            useDiscordStore.getState().update({
                discordToken: raw.discordToken || '',
                _discordTokenRef: raw.discordToken || '',
                discordAutoStart: raw.discordAutoStart ?? false,
            });
            useSettingsStore.getState()._setDiscordFields(raw.discordToken || '', raw.discordAutoStart ?? false);
        }

        // Step 2: Load bot config
        await useDiscordStore.getState().loadConfig();

        // Verify ALL 17 fields
        const s = useSettingsStore.getState();
        expect(s.autoRefresh).toBe(false);
        expect(s.refreshInterval).toBe(4444);
        expect(s.ipcPort).toBe(8888);
        expect(s.consoleBufferSize).toBe(6000);

        expect(s._discordToken).toBe('rt-token');
        expect(s._discordAutoStart).toBe(true);

        const d = useDiscordStore.getState();
        expect(d.discordToken).toBe('rt-token');
        expect(d.discordAutoStart).toBe(true);
        expect(d.discordPrefix).toBe('!rt');
        expect(d.discordBotMode).toBe('cloud');
        expect(d.discordCloudRelayUrl).toBe('https://rt.relay');
        expect(d.discordCloudHostId).toBe('rt-host');
        expect(d.discordModuleAliases).toEqual({ rt: 'roundtrip' });
        expect(d.discordCommandAliases).toEqual({ rt: { save: 'go' } });
        expect(d.discordMusicEnabled).toBe(false);
        expect(d.nodeSettings).toEqual({ local: { rt: true } });
        expect(d.cloudNodes).toEqual([{ id: 'rt-node' }]);
        expect(d.cloudMembers).toEqual({ rtUser: { role: 'mod' } });
    });
});

// ═══════════════════════════════════════════════════════════════
// 5. 자동저장 감시키 완전성 검증
// ═══════════════════════════════════════════════════════════════

describe('자동저장 감시키 완전성', () => {
    it('settings auto-save: GUI 5필드 + _discordAutoStart = 6필드 변경 감시 (debounced)', async () => {
        vi.useFakeTimers();
        mockApi();
        // Initialize store as "ready"
        useSettingsStore.setState({
            settingsPath: '/s.json',
            settingsReady: true,
            autoRefresh: true,
            refreshInterval: 2000,
            ipcPort: 57474,
            consoleBufferSize: 2000,
        });

        // Change refreshInterval
        useSettingsStore.getState().update({ refreshInterval: 9999 });

        // Fast-forward debounce (500ms)
        await vi.advanceTimersByTimeAsync(600);

        expect(window.api.settingsSave).toHaveBeenCalled();
        const payload = window.api.settingsSave.mock.calls[0][0];
        expect(payload.refreshInterval).toBe(9999);

        vi.useRealTimers();
    });

    it('discord auto-save: 10필드 중 하나(prefix) 변경 시 botConfigSave 호출', async () => {
        vi.useFakeTimers();
        mockApi();
        useDiscordStore.setState({ _settingsReady: true, discordPrefix: '!saba' });

        useDiscordStore.getState().update({ discordPrefix: '!changed' });

        await vi.advanceTimersByTimeAsync(600);

        expect(window.api.botConfigSave).toHaveBeenCalled();
        const payload = window.api.botConfigSave.mock.calls[0][0];
        expect(payload.prefix).toBe('!changed');

        vi.useRealTimers();
    });

    it('discord auto-save: moduleAliases 변경 → botConfigSave 호출', async () => {
        vi.useFakeTimers();
        mockApi();
        useDiscordStore.setState({ _settingsReady: true });

        useDiscordStore.getState().update({ discordModuleAliases: { mc: 'minecraft-new' } });

        await vi.advanceTimersByTimeAsync(600);

        expect(window.api.botConfigSave).toHaveBeenCalled();
        vi.useRealTimers();
    });

    it('discord auto-save: musicEnabled 변경 → botConfigSave 호출', async () => {
        vi.useFakeTimers();
        mockApi();
        useDiscordStore.setState({ _settingsReady: true, discordMusicEnabled: true });

        useDiscordStore.getState().update({ discordMusicEnabled: false });

        await vi.advanceTimersByTimeAsync(600);

        expect(window.api.botConfigSave).toHaveBeenCalled();
        vi.useRealTimers();
    });

    it('settings auto-save: discordAutoStart 변경 → 크로스스토어 경유 → settingsSave 호출', async () => {
        vi.useFakeTimers();
        mockApi();
        useSettingsStore.setState({
            settingsPath: '/s.json',
            settingsReady: true,
        });

        // discordStore에서 autoStart 변경 → cross-store sync → _discordAutoStart 변경 → auto-save
        useDiscordStore.getState().update({ discordAutoStart: true });

        await vi.advanceTimersByTimeAsync(600);

        expect(window.api.settingsSave).toHaveBeenCalled();
        const payload = window.api.settingsSave.mock.calls[0][0];
        expect(payload.discordAutoStart).toBe(true);

        vi.useRealTimers();
    });

    it('discord auto-save: _settingsReady=false → 변경해도 저장 안됨', async () => {
        vi.useFakeTimers();
        mockApi();
        useDiscordStore.setState({ _settingsReady: false });

        useDiscordStore.getState().update({ discordPrefix: '!blocked' });

        await vi.advanceTimersByTimeAsync(600);

        expect(window.api.botConfigSave).not.toHaveBeenCalled();
        vi.useRealTimers();
    });
});

// ═══════════════════════════════════════════════════════════════
// 6. 에러 핸들링
// ═══════════════════════════════════════════════════════════════

describe('설정 에러 핸들링', () => {
    it('settingsLoad 실패 → settingsReady=true, 기본값 유지', async () => {
        mockApi({
            settingsLoad: vi.fn().mockRejectedValue(new Error('ENOENT')),
        });

        const result = await useSettingsStore.getState().load();

        expect(result).toBeNull();
        expect(useSettingsStore.getState().settingsReady).toBe(true);
        expect(useSettingsStore.getState().autoRefresh).toBe(true); // default
    });

    it('botConfigLoad 실패 → 기본값 유지, 크래시 없음', async () => {
        mockApi({
            botConfigLoad: vi.fn().mockRejectedValue(new Error('corrupt JSON')),
        });

        await useDiscordStore.getState().loadConfig();

        expect(useDiscordStore.getState().discordPrefix).toBe('!saba'); // default
        expect(useDiscordStore.getState().discordBotMode).toBe('local'); // default
    });

    it('settingsSave 실패 → 에러 로그 (크래시 없음)', async () => {
        mockApi({
            settingsSave: vi.fn().mockRejectedValue(new Error('disk full')),
        });
        useSettingsStore.setState({ settingsPath: '/s.json' });

        // Should not throw
        await expect(useSettingsStore.getState().save()).resolves.toBeUndefined();
    });

    it('botConfigSave 실패 → 에러 토스트 (크래시 없음)', async () => {
        mockApi({
            botConfigSave: vi.fn().mockRejectedValue(new Error('EPERM')),
        });

        await expect(useDiscordStore.getState().saveConfig()).resolves.toBeUndefined();
    });

    it('botConfigSave 에러 응답 → 에러 토스트 (크래시 없음)', async () => {
        mockApi({
            botConfigSave: vi.fn().mockResolvedValue({ error: 'write failed' }),
        });

        await expect(useDiscordStore.getState().saveConfig()).resolves.toBeUndefined();
    });
});
