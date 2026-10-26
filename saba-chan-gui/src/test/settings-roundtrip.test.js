/**
 * 설정 저장/로드 라운드트립 테스트
 *
 * 모든 설정 항목에 대해 save → load 왕복을 검증한다.
 * - settings.json (settingsLoad / settingsSave) : 17 필드 (GUI 전용 + 테마 모드 + 테마 커스터마이즈 10필드)
 * - bot-config.json (botConfigLoad / botConfigSave) : 데몬 BotConfig 키 (로컬 봇 키 없음)
 *
 * 각 테스트가 독립적으로 store를 리셋하므로 순서 무관.
 */
import { beforeEach, describe, expect, it, vi } from 'vitest';
import { useDiscordStore } from '../stores/useDiscordStore';
import { DEFAULT_SOUNDS, useSettingsStore } from '../stores/useSettingsStore';
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

    it('save() payload에는 바꾼 GUI 칸만 실린다 (token/autoStart는 bot-config, 바꾸지 않은 칸은 CLI 값을 덮지 않게)', async () => {
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

        await useSettingsStore.getState().save();

        const payload = window.api.settingsSave.mock.calls[0][0];
        expect(payload).toEqual({
            autoRefresh: false,
            refreshInterval: 9999,
            ipcPort: 11111,
            consoleBufferSize: 500,
        });
        // discordToken/discordAutoStart가 포함되지 않음을 확인
        expect(payload.discordToken).toBeUndefined();
        expect(payload.discordAutoStart).toBeUndefined();
    });

    // ── 개별 필드 라운드트립: save → load → 값이 동일한지 ──

    const guiFields = [
        { field: 'autoRefresh', saved: false, defaultVal: true },
        { field: 'refreshInterval', saved: 7777, defaultVal: 2000 },
        { field: 'ipcPort', saved: 33333, defaultVal: 57474 },
        { field: 'systemMonitorOpen', saved: false, defaultVal: true },
        { field: 'consoleBufferSize', saved: 100, defaultVal: 2000 },
        // 테마 모드도 config.json이 원천 (예전엔 브라우저 저장소에만 있었다)
        { field: 'theme', saved: 'dark', defaultVal: 'auto' },
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

    // ── 전체 필드 동시 라운드트립 ──

    it('전체 필드 동시 라운드트립: 커스텀 값 → save → reset → load → 전부 복원', async () => {
        const custom = {
            autoRefresh: false,
            refreshInterval: 5000,
            ipcPort: 22222,
            consoleBufferSize: 4000,
            autoGeneratePasswords: true,
            portConflictCheck: true,
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
        await useSettingsStore.getState().save();

        // 바꾼 칸만 실린다 (token/autoStart는 더 이상 포함되지 않음)
        expect(captured).toEqual({
            autoRefresh: custom.autoRefresh,
            refreshInterval: custom.refreshInterval,
            ipcPort: custom.ipcPort,
            consoleBufferSize: custom.consoleBufferSize,
        });

        // Reset
        useSettingsStore.getState()._resetForTest();

        // Load
        mockApi({
            settingsLoad: vi.fn().mockResolvedValue(captured),
            settingsGetPath: vi.fn().mockResolvedValue('/s.json'),
        });
        await useSettingsStore.getState().load();

        // Verify all GUI fields
        const s = useSettingsStore.getState();
        expect(s.autoRefresh).toBe(custom.autoRefresh);
        expect(s.refreshInterval).toBe(custom.refreshInterval);
        expect(s.ipcPort).toBe(custom.ipcPort);
        expect(s.consoleBufferSize).toBe(custom.consoleBufferSize);
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

    it('효과음: 빠진 종류는 켬으로 채우고, 한 종류를 끄면 전체 묶음을 저장한다', async () => {
        mockApi({
            settingsLoad: vi.fn().mockResolvedValue({ sounds: { enabled: true, crash: false } }),
            settingsGetPath: vi.fn().mockResolvedValue('/s.json'),
        });
        await useSettingsStore.getState().load();
        const { sounds } = useSettingsStore.getState();
        expect(sounds).toEqual({ ...DEFAULT_SOUNDS, crash: false });

        useSettingsStore.getState().update({ sounds: { ...sounds, update: false } });
        await useSettingsStore.getState().save();
        expect(window.api.settingsSave).toHaveBeenCalledWith({
            sounds: { ...DEFAULT_SOUNDS, crash: false, update: false },
        });
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

    it('saveConfig() payload = 데몬 BotConfig 키 전부, 로컬 봇 키 없음', async () => {
        mockApi();
        useDiscordStore.setState({
            discordPrefixes: ['!test'],
            discordCloudRelayUrl: 'https://relay.test',
            discordCloudHostId: 'host-xyz',
            discordModuleAliases: { mc: 'minecraft' },
            discordCommandAliases: { mc: { start: 'go' } },
            nodeSettings: { g1: { allowedInstances: ['a'] } },
            cloudNodes: [{ id: 'n1' }],
            cloudMembers: { u1: 'admin' },
        });

        await useDiscordStore.getState().saveConfig();

        const payload = window.api.botConfigSave.mock.calls[0][0];
        expect(payload).toEqual({
            prefixes: ['!test'],
            cloud: { relayUrl: 'https://relay.test', hostId: 'host-xyz' },
            moduleAliases: { mc: 'minecraft' },
            commandAliases: { mc: { start: 'go' } },
            nodeSettings: { g1: { allowedInstances: ['a'] } },
            cloudNodes: [{ id: 'n1' }],
            cloudMembers: { u1: 'admin' },
        });
    });

    // ── 개별 필드 라운드트립 (simple fields) ──

    const simpleBotFields = [
        {
            field: 'discordPrefixes',
            botKey: 'prefixes',
            saved: ['!mybot', '사바'],
            defaultVal: [],
            makeBotCfg: (v) => ({ prefixes: v }),
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

    // ── 전체 필드 동시 라운드트립 (bot-config) ──

    it('전체 필드 동시 라운드트립 (bot-config): 커스텀 → save → reset → load → 전부 복원', async () => {
        const custom = {
            discordPrefixes: ['!full'],
            discordCloudRelayUrl: 'https://full.relay',
            discordCloudHostId: 'full-host-id',
            discordModuleAliases: { a: 'alpha', b: 'beta' },
            discordCommandAliases: { a: { x: 'ex' } },
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
        expect(captured.prefixes).toEqual(['!full']);
        expect(captured.cloud).toEqual({ relayUrl: 'https://full.relay', hostId: 'full-host-id' });
        expect(captured.moduleAliases).toEqual({ a: 'alpha', b: 'beta' });
        expect(captured.commandAliases).toEqual({ a: { x: 'ex' } });
        expect(captured.nodeSettings).toEqual({ local: { data: true } });
        expect(captured.cloudNodes).toEqual([{ id: 'cn1' }]);
        expect(captured.cloudMembers).toEqual({ m1: { level: 3 } });

        // Reset + Load
        useDiscordStore.getState()._resetForTest();
        mockApi({ botConfigLoad: vi.fn().mockResolvedValue(captured) });
        await useDiscordStore.getState().loadConfig();

        const d = useDiscordStore.getState();
        expect(d.discordPrefixes).toEqual(custom.discordPrefixes);
        expect(d.discordCloudRelayUrl).toBe(custom.discordCloudRelayUrl);
        expect(d.discordCloudHostId).toBe(custom.discordCloudHostId);
        expect(d.discordModuleAliases).toEqual(custom.discordModuleAliases);
        expect(d.discordCommandAliases).toEqual(custom.discordCommandAliases);
        expect(d.nodeSettings).toEqual(custom.nodeSettings);
        expect(d.cloudNodes).toEqual(custom.cloudNodes);
        expect(d.cloudMembers).toEqual(custom.cloudMembers);
    });

    // ── 기본값 복원: botConfigLoad가 빈 객체일 때 ──

    it('botConfigLoad 빈 객체 → 모든 봇 필드 기본값 유지', async () => {
        mockApi({ botConfigLoad: vi.fn().mockResolvedValue({}) });
        await useDiscordStore.getState().loadConfig();

        const d = useDiscordStore.getState();
        expect(d.discordPrefixes).toEqual([]);
        expect(d.discordCloudRelayUrl).toBe('');
        expect(d.discordCloudHostId).toBe('');
        expect(d.discordModuleAliases).toEqual({});
        expect(d.discordCommandAliases).toEqual({});
        // nodeSettings, cloudNodes, cloudMembers are not patched when missing
        expect(d.nodeSettings).toEqual({});
        expect(d.cloudNodes).toEqual([]);
        expect(d.cloudMembers).toEqual({});
    });
});

// ═══════════════════════════════════════════════════════════════
// 4. saveCurrentSettings (App.js와 동일한 로직)
// ═══════════════════════════════════════════════════════════════

describe('saveCurrentSettings 통합 (settings + botConfig 동시 저장)', () => {
    /** App.js의 saveCurrentSettings 재현 — token/autoStart는 bot-config SSOT */
    async function saveCurrentSettings() {
        await Promise.all([useSettingsStore.getState().save(), useDiscordStore.getState().saveConfig()]);
    }

    it('모든 필드가 두 파일에 올바르게 분배되어 저장', async () => {
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
            discordPrefixes: ['!go'],
            discordCloudRelayUrl: 'https://r.io',
            discordCloudHostId: 'h1',
            discordModuleAliases: { x: 'y' },
            discordCommandAliases: { x: { a: 'b' } },
            nodeSettings: { n: 1 },
            cloudNodes: [{ id: 'c1' }],
            cloudMembers: { u: 'a' },
        });

        await saveCurrentSettings();

        // Check settings file — token/autoStart는 더 이상 포함되지 않음
        const settingsPayload = window.api.settingsSave.mock.calls[0][0];
        expect(settingsPayload).toEqual({
            autoRefresh: false,
            refreshInterval: 3000,
            ipcPort: 9999,
            consoleBufferSize: 1000,
        });
        expect(settingsPayload.discordToken).toBeUndefined();
        expect(settingsPayload.discordAutoStart).toBeUndefined();

        // Check bot config file
        const botPayload = window.api.botConfigSave.mock.calls[0][0];
        expect(botPayload).toEqual({
            prefixes: ['!go'],
            cloud: { relayUrl: 'https://r.io', hostId: 'h1' },
            moduleAliases: { x: 'y' },
            commandAliases: { x: { a: 'b' } },
            nodeSettings: { n: 1 },
            cloudNodes: [{ id: 'c1' }],
            cloudMembers: { u: 'a' },
        });
    });

    it('saveCurrentSettings → reset → 두 파일 모두 로드 → 전부 복원', async () => {
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
            discordPrefixes: ['!rt'],
            discordCloudRelayUrl: 'https://rt.relay',
            discordCloudHostId: 'rt-host',
            discordModuleAliases: { rt: 'roundtrip' },
            discordCommandAliases: { rt: { save: 'go' } },
            nodeSettings: { local: { rt: true } },
            cloudNodes: [{ id: 'rt-node' }],
            cloudMembers: { rtUser: { role: 'mod' } },
        });

        await saveCurrentSettings();

        // Reset both stores
        useSettingsStore.getState()._resetForTest();
        useDiscordStore.getState()._resetForTest();

        // Reload from captured data (simulating App.js boot sequence)
        mockApi({
            settingsLoad: vi.fn().mockResolvedValue(settingsCaptured),
            settingsGetPath: vi.fn().mockResolvedValue('/s.json'),
            botConfigLoad: vi.fn().mockResolvedValue(botCaptured),
        });

        // Step 1: Load settings (no longer contains token/autoStart)
        await useSettingsStore.getState().load();

        // Step 2: Load bot config (SSOT for token/autoStart)
        await useDiscordStore.getState().loadConfig();

        // Verify settings fields (6 GUI-only)
        const s = useSettingsStore.getState();
        expect(s.autoRefresh).toBe(false);
        expect(s.refreshInterval).toBe(4444);
        expect(s.ipcPort).toBe(8888);
        expect(s.consoleBufferSize).toBe(6000);

        // Verify discord fields (13 — all from bot-config)
        const d = useDiscordStore.getState();
        expect(d.discordPrefixes).toEqual(['!rt']);
        expect(d.discordCloudRelayUrl).toBe('https://rt.relay');
        expect(d.discordCloudHostId).toBe('rt-host');
        expect(d.discordModuleAliases).toEqual({ rt: 'roundtrip' });
        expect(d.discordCommandAliases).toEqual({ rt: { save: 'go' } });
        expect(d.nodeSettings).toEqual({ local: { rt: true } });
        expect(d.cloudNodes).toEqual([{ id: 'rt-node' }]);
        expect(d.cloudMembers).toEqual({ rtUser: { role: 'mod' } });
    });
});

// ═══════════════════════════════════════════════════════════════
// 5. 자동저장 감시키 완전성 검증
// ═══════════════════════════════════════════════════════════════

describe('자동저장 감시키 완전성', () => {
    it('settings auto-save: GUI 필드 변경 감시 (debounced)', async () => {
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
        useDiscordStore.setState({ _settingsReady: true, _botConfigLoaded: true, discordPrefixes: [] });

        useDiscordStore.getState().update({ discordPrefixes: ['!changed'] });

        await vi.advanceTimersByTimeAsync(600);

        expect(window.api.botConfigSave).toHaveBeenCalled();
        const payload = window.api.botConfigSave.mock.calls[0][0];
        expect(payload.prefixes).toEqual(['!changed']);

        vi.useRealTimers();
    });

    it('discord auto-save: moduleAliases 변경 → botConfigSave 호출', async () => {
        vi.useFakeTimers();
        mockApi();
        useDiscordStore.setState({ _settingsReady: true, _botConfigLoaded: true });

        useDiscordStore.getState().update({ discordModuleAliases: { mc: 'minecraft-new' } });

        await vi.advanceTimersByTimeAsync(600);

        expect(window.api.botConfigSave).toHaveBeenCalled();
        vi.useRealTimers();
    });

    it('discord auto-save: _settingsReady=false → 변경해도 저장 안됨', async () => {
        vi.useFakeTimers();
        mockApi();
        useDiscordStore.setState({ _settingsReady: false });

        useDiscordStore.getState().update({ discordPrefixes: ['!blocked'] });

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

        expect(useDiscordStore.getState().discordPrefixes).toEqual([]); // default
    });

    it('settingsSave 실패 → 에러 로그 (크래시 없음)', async () => {
        mockApi({
            settingsSave: vi.fn().mockRejectedValue(new Error('disk full')),
        });
        useSettingsStore.setState({ settingsPath: '/s.json', theme: 'dark' });

        // Should not throw, and tells the caller it failed
        await expect(useSettingsStore.getState().save()).resolves.toBe(false);
    });

    it('botConfigSave 실패 → 에러 토스트 (크래시 없음)', async () => {
        mockApi({
            botConfigSave: vi.fn().mockRejectedValue(new Error('EPERM')),
        });
        useDiscordStore.setState({ discordPrefixes: ['!x'] });

        await expect(useDiscordStore.getState().saveConfig()).resolves.toBe(false);
    });

    it('botConfigSave 에러 응답 → 에러 토스트 (크래시 없음)', async () => {
        mockApi({
            botConfigSave: vi.fn().mockResolvedValue({ error: 'write failed' }),
        });
        useDiscordStore.setState({ discordPrefixes: ['!x'] });

        await expect(useDiscordStore.getState().saveConfig()).resolves.toBe(false);
    });
});
