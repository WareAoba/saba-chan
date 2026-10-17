/**
 * Zustand 스토어 단위 테스트
 *
 * 개별 스토어의 상태 관리, 액션, 리셋, 에지 케이스를 검증합니다.
 * React 렌더링 없이 스토어만 직접 조작하여 빠른 피드백을 제공합니다.
 */
import { beforeEach, describe, expect, it, vi } from 'vitest';
import { useSettingsStore } from '../stores/useSettingsStore';
import { useDiscordStore } from '../stores/useDiscordStore';
import { useServerStore } from '../stores/useServerStore';
import { useUIStore } from '../stores/useUIStore';

// ── 공통 헬퍼 ───────────────────────────────────────────────

function mockApi(overrides = {}) {
    window.api = new Proxy(
        {
            settingsLoad: vi.fn().mockResolvedValue({}),
            settingsSave: vi.fn().mockResolvedValue({ success: true }),
            settingsGetPath: vi.fn().mockResolvedValue('/mock/settings.json'),
            botConfigLoad: vi.fn().mockResolvedValue({}),
            botConfigSave: vi.fn().mockResolvedValue({ success: true }),
            daemonStatus: vi.fn().mockResolvedValue({ running: true }),
            moduleList: vi.fn().mockResolvedValue({ modules: [] }),
            moduleGetLocales: vi.fn().mockResolvedValue({}),
            moduleGetMetadata: vi.fn().mockResolvedValue({ toml: {} }),
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
    window.showStatus = vi.fn();
}

beforeEach(() => {
    vi.clearAllMocks();
    useSettingsStore.getState()._resetForTest();
    useDiscordStore.getState()._resetForTest();
    useServerStore.getState()._resetForTest();
    useUIStore.getState()._resetForTest();
    mockApi();
});

// ═════════════════════════════════════════════════════════════
// 1. useSettingsStore
// ═════════════════════════════════════════════════════════════

describe('useSettingsStore', () => {
    it('초기 상태가 올바른 기본값을 가져야 한다', () => {
        const state = useSettingsStore.getState();
        expect(state.autoRefresh).toBe(true);
        expect(state.refreshInterval).toBe(2000);
        expect(state.ipcPort).toBe(57474);
        expect(state.consoleBufferSize).toBe(2000);
        expect(state.autoGeneratePasswords).toBe(true);
        expect(state.portConflictCheck).toBe(true);
        expect(state.settingsPath).toBe('');
        expect(state.settingsReady).toBe(false);
    });

    it('_resetForTest()는 모든 필드를 기본값으로 복원해야 한다', () => {
        useSettingsStore.setState({
            autoRefresh: false,
            refreshInterval: 9999,
            ipcPort: 11111,
            settingsReady: true,
        });

        useSettingsStore.getState()._resetForTest();

        const state = useSettingsStore.getState();
        expect(state.autoRefresh).toBe(true);
        expect(state.refreshInterval).toBe(2000);
        expect(state.ipcPort).toBe(57474);
        expect(state.settingsReady).toBe(false);
    });

    it('load() — 성공 시 모든 필드 반영 + settingsReady=true', async () => {
        mockApi({
            settingsLoad: vi.fn().mockResolvedValue({
                autoRefresh: false,
                refreshInterval: 5000,
                ipcPort: 12345,
                consoleBufferSize: 500,
                autoGeneratePasswords: false,
                portConflictCheck: false,
            }),
            settingsGetPath: vi.fn().mockResolvedValue('C:\\config\\settings.json'),
        });

        const rawSettings = await useSettingsStore.getState().load();

        const state = useSettingsStore.getState();
        expect(state.autoRefresh).toBe(false);
        expect(state.refreshInterval).toBe(5000);
        expect(state.ipcPort).toBe(12345);
        expect(state.consoleBufferSize).toBe(500);
        expect(state.settingsPath).toBe('C:\\config\\settings.json');
        expect(state.settingsReady).toBe(true);
        expect(rawSettings).toBeTruthy();
    });

    it('load() — 실패 시에도 settingsReady=true (fallback)', async () => {
        mockApi({
            settingsLoad: vi.fn().mockRejectedValue(new Error('disk error')),
        });

        const result = await useSettingsStore.getState().load();

        expect(result).toBeNull();
        expect(useSettingsStore.getState().settingsReady).toBe(true);
    });

    it('load() — 누락 필드는 기본값으로 채워져야 한다', async () => {
        mockApi({
            settingsLoad: vi.fn().mockResolvedValue({ ipcPort: 8080 }),
        });

        await useSettingsStore.getState().load();

        const state = useSettingsStore.getState();
        expect(state.ipcPort).toBe(8080);
        expect(state.autoRefresh).toBe(true); // 기본값
        expect(state.refreshInterval).toBe(2000); // 기본값
    });

    it('save() — settingsPath 미설정 시 저장 스킵', async () => {
        // settingsPath가 빈 문자열
        await useSettingsStore.getState().save();

        expect(window.api.settingsSave).not.toHaveBeenCalled();
    });

    it('save() — settingsPath 설정 시 전체 필드 전송', async () => {
        useSettingsStore.setState({
            settingsPath: '/mock/settings.json',
            autoRefresh: false,
            refreshInterval: 3000,
            ipcPort: 9999,
            consoleBufferSize: 100,
        });

        await useSettingsStore.getState().save();

        expect(window.api.settingsSave).toHaveBeenCalledTimes(1);
        const payload = window.api.settingsSave.mock.calls[0][0];
        expect(payload.autoRefresh).toBe(false);
        expect(payload.refreshInterval).toBe(3000);
        expect(payload.ipcPort).toBe(9999);
        expect(payload.consoleBufferSize).toBe(100);
        expect(payload.modulesPath).toBeUndefined();
    });

    it('update() — 부분 업데이트 적용', () => {
        useSettingsStore.getState().update({ autoRefresh: false, ipcPort: 7777 });

        const state = useSettingsStore.getState();
        expect(state.autoRefresh).toBe(false);
        expect(state.ipcPort).toBe(7777);
        expect(state.refreshInterval).toBe(2000); // 변경 안 됨
    });

    // ── Theme customization ──

    it('초기 상태에 테마 커스터마이즈 필드가 존재해야 한다', () => {
        const state = useSettingsStore.getState();
        expect(state.accentColor).toBe('#667eea');
        expect(state.accentSecondary).toBe('#764ba2');
        expect(state.useGradient).toBe(true);
        expect(state.fontScale).toBe(100);
        expect(state.enableTransitions).toBe(true);
        expect(state.consoleSyntaxHighlight).toBe(true);
        expect(state.consoleBgColor).toBe('#1e1e2e');
        expect(state.consoleTextColor).toBe('#cdd6f4');
        expect(state.sidebarCompact).toBe(false);
        expect(state.consoleFontScale).toBe(100);
    });

    it('_resetForTest()는 테마 커스터마이즈 필드도 초기화해야 한다', () => {
        useSettingsStore.setState({
            accentColor: '#ff0000',
            fontScale: 150,
            consoleBgColor: '#000000',
        });
        useSettingsStore.getState()._resetForTest();

        const state = useSettingsStore.getState();
        expect(state.accentColor).toBe('#667eea');
        expect(state.fontScale).toBe(100);
        expect(state.consoleBgColor).toBe('#1e1e2e');
    });

    it('update() — 테마 필드 부분 업데이트 적용', () => {
        useSettingsStore.getState().update({ accentColor: '#ff5500', fontScale: 120 });

        const state = useSettingsStore.getState();
        expect(state.accentColor).toBe('#ff5500');
        expect(state.fontScale).toBe(120);
        expect(state.useGradient).toBe(true); // 변경 안 됨
    });

    it('load() — 테마 설정이 서버 응답에 포함되면 반영되어야 한다', async () => {
        mockApi({
            settingsLoad: vi.fn().mockResolvedValue({
                accentColor: '#43e97b',
                accentSecondary: '#38f9d7',
                useGradient: false,
                fontScale: 80,
                enableTransitions: false,
                consoleSyntaxHighlight: false,
                consoleBgColor: '#000000',
                consoleTextColor: '#ffffff',
                consoleFontScale: 75,
            }),
            settingsGetPath: vi.fn().mockResolvedValue('/mock/settings.json'),
        });

        await useSettingsStore.getState().load();

        const state = useSettingsStore.getState();
        expect(state.accentColor).toBe('#43e97b');
        expect(state.accentSecondary).toBe('#38f9d7');
        expect(state.useGradient).toBe(false);
        expect(state.fontScale).toBe(80);
        expect(state.enableTransitions).toBe(false);
        expect(state.consoleSyntaxHighlight).toBe(false);
        expect(state.consoleBgColor).toBe('#000000');
        expect(state.consoleTextColor).toBe('#ffffff');
        expect(state.consoleFontScale).toBe(75);
    });

    it('load() — 테마 설정 누락 시 기본값으로 채워져야 한다', async () => {
        mockApi({
            settingsLoad: vi.fn().mockResolvedValue({ autoRefresh: true }),
            settingsGetPath: vi.fn().mockResolvedValue('/mock/settings.json'),
        });

        await useSettingsStore.getState().load();

        const state = useSettingsStore.getState();
        expect(state.accentColor).toBe('#667eea');
        expect(state.fontScale).toBe(100);
        expect(state.enableTransitions).toBe(true);
        expect(state.consoleSyntaxHighlight).toBe(true);
    });

    it('save() — 테마 필드가 payload에 포함되어야 한다', async () => {
        mockApi();
        useSettingsStore.setState({
            settingsPath: '/mock/settings.json',
            accentColor: '#eb3349',
            fontScale: 150,
        });

        await useSettingsStore.getState().save();

        const callArgs = window.api.settingsSave.mock.calls[0][0];
        expect(callArgs.accentColor).toBe('#eb3349');
        expect(callArgs.fontScale).toBe(150);
        expect(callArgs.useGradient).toBe(true);
        expect(callArgs.consoleSyntaxHighlight).toBe(true);
        expect(callArgs.consoleBgColor).toBe('#1e1e2e');
        expect(callArgs.consoleTextColor).toBe('#cdd6f4');
        expect(callArgs.consoleFontScale).toBe(100);
    });
});

// ═════════════════════════════════════════════════════════════
// 2. useUIStore
// ═════════════════════════════════════════════════════════════

describe('useUIStore', () => {
    it('초기 상태 검증', () => {
        const state = useUIStore.getState();
        expect(state.activeServerId).toBeNull();
        expect(state.activeTab).toBe('servers');
        expect(state.modal).toBeNull();
    });

    it('setActiveTab() — 탭 전환', () => {
        useUIStore.getState().setActiveTab('settings');
        expect(useUIStore.getState().activeTab).toBe('settings');
    });

    it('setActiveServerId() — 서버 선택', () => {
        useUIStore.getState().setActiveServerId('srv-42');
        expect(useUIStore.getState().activeServerId).toBe('srv-42');
    });

    it('openModal() / closeModal() — 모달 생명주기', () => {
        useUIStore.getState().openModal({
            type: 'confirm',
            title: '삭제',
            message: '정말 삭제?',
        });
        expect(useUIStore.getState().modal).not.toBeNull();
        expect(useUIStore.getState().modal.type).toBe('confirm');

        useUIStore.getState().closeModal();
        expect(useUIStore.getState().modal).toBeNull();
    });

    it('closeModal() — 모달 없는 상태에서 호출해도 패닉 없음', () => {
        expect(useUIStore.getState().modal).toBeNull();
        useUIStore.getState().closeModal();
        expect(useUIStore.getState().modal).toBeNull();
    });

    it('_resetForTest() — 완전 리셋', () => {
        useUIStore.setState({ activeTab: 'discord', activeServerId: 'x' });
        useUIStore.getState()._resetForTest();

        const state = useUIStore.getState();
        expect(state.activeTab).toBe('servers');
        expect(state.activeServerId).toBeNull();
        expect(state.modal).toBeNull();
    });
});

// ═════════════════════════════════════════════════════════════
// 3. useServerStore
// ═════════════════════════════════════════════════════════════

describe('useServerStore', () => {
    it('초기 상태 — loading=true, servers 비어있음', () => {
        const state = useServerStore.getState();
        expect(state.servers).toEqual([]);
        expect(state.modules).toEqual([]);
        expect(state.loading).toBe(true);
        expect(state.daemonReady).toBe(false);
        expect(state.serversInitializing).toBe(true);
    });

    it('setServers() — 배열 직접 설정', () => {
        const servers = [
            { instance_id: 'a', name: 'A', status: 'running' },
            { instance_id: 'b', name: 'B', status: 'stopped' },
        ];
        useServerStore.getState().setServers(servers);
        expect(useServerStore.getState().servers).toHaveLength(2);
        expect(useServerStore.getState().servers[0].instance_id).toBe('a');
    });

    it('setServers() — updater 함수 지원', () => {
        useServerStore.getState().setServers([
            { instance_id: 'x', name: 'X', status: 'running' },
        ]);

        useServerStore.getState().setServers((prev) => [
            ...prev,
            { instance_id: 'y', name: 'Y', status: 'stopped' },
        ]);

        expect(useServerStore.getState().servers).toHaveLength(2);
    });

    it('setModules() — 모듈 목록 설정', () => {
        useServerStore.getState().setModules([
            { name: 'palworld', version: '1.0.0' },
            { name: 'minecraft', version: '2.0.0' },
        ]);
        expect(useServerStore.getState().modules).toHaveLength(2);
    });

    it('setDaemonReady() / setInitStatus() / setInitProgress()', () => {
        useServerStore.getState().setDaemonReady(true);
        useServerStore.getState().setInitStatus('Loading modules...');
        useServerStore.getState().setInitProgress(50);

        const state = useServerStore.getState();
        expect(state.daemonReady).toBe(true);
        expect(state.initStatus).toBe('Loading modules...');
        expect(state.initProgress).toBe(50);
    });

    it('formatUptime() — 올바른 HH:MM:SS 형식', () => {
        const now = Math.floor(Date.now() / 1000);
        useServerStore.setState({ nowEpoch: now });

        const oneHourAgo = now - 3661; // 1시간 1분 1초 전
        const result = useServerStore.getState().formatUptime(oneHourAgo);
        expect(result).toBe('01:01:01');
    });

    it('formatUptime() — null 입력 시 null 반환', () => {
        expect(useServerStore.getState().formatUptime(null)).toBeNull();
    });

    it('formatUptime() — 미래 시간은 00:00:00', () => {
        const now = Math.floor(Date.now() / 1000);
        useServerStore.setState({ nowEpoch: now });

        const future = now + 1000;
        expect(useServerStore.getState().formatUptime(future)).toBe('00:00:00');
    });

    it('startUptimeClock() — 중복 호출 시 인터벌 중복 생성 방지', () => {
        vi.useFakeTimers();
        try {
            useServerStore.getState().startUptimeClock();
            const interval1 = useServerStore.getState()._uptimeInterval;
            expect(interval1).not.toBeNull();

            useServerStore.getState().startUptimeClock();
            const interval2 = useServerStore.getState()._uptimeInterval;
            expect(interval2).toBe(interval1); // 같은 인터벌

            useServerStore.getState().stopUptimeClock();
        } finally {
            vi.useRealTimers();
        }
    });

    it('stopUptimeClock() — 인터벌 없는 상태에서 호출해도 안전', () => {
        expect(useServerStore.getState()._uptimeInterval).toBeNull();
        useServerStore.getState().stopUptimeClock(); // 패닉 없음
        expect(useServerStore.getState()._uptimeInterval).toBeNull();
    });

    it('_resetForTest() — 완전 리셋', () => {
        useServerStore.setState({
            servers: [{ id: 'x' }],
            modules: [{ name: 'y' }],
            daemonReady: true,
            loading: false,
            initProgress: 100,
        });

        useServerStore.getState()._resetForTest();

        const state = useServerStore.getState();
        expect(state.servers).toEqual([]);
        expect(state.modules).toEqual([]);
        expect(state.daemonReady).toBe(false);
        expect(state.loading).toBe(true);
        expect(state.initProgress).toBe(0);
    });
});

// ═════════════════════════════════════════════════════════════
// 4. useDiscordStore (클라우드 전용)
// ═════════════════════════════════════════════════════════════

describe('useDiscordStore', () => {
    it('초기 상태 검증', () => {
        const state = useDiscordStore.getState();
        expect(state.discordPrefix).toBe('!saba');
        expect(state.discordCloudRelayUrl).toBe('');
        expect(state.discordCloudHostId).toBe('');
        expect(state.relayConnected).toBe(false);
        expect(state.relayStatusChecked).toBe(false);
    });

    it('update() — 부분 업데이트', () => {
        useDiscordStore.getState().update({
            discordPrefix: '!s',
        });
        const state = useDiscordStore.getState();
        expect(state.discordPrefix).toBe('!s');
    });

    it('loadConfig() — bot-config에서 모든 필드 로드', async () => {
        mockApi({
            botConfigLoad: vi.fn().mockResolvedValue({
                prefix: '!bot',
                cloud: { relayUrl: 'https://relay.example.com', hostId: 'h123' },
                moduleAliases: { palworld: 'pw' },
                commandAliases: { palworld: { start: '시작' } },
                nodeSettings: { guild1: { defaultHostId: 'h1' } },
            }),
        });

        await useDiscordStore.getState().loadConfig();

        const state = useDiscordStore.getState();
        expect(state.discordPrefix).toBe('!bot');
        expect(state.discordCloudRelayUrl).toBe('https://relay.example.com');
        expect(state.discordCloudHostId).toBe('h123');
        expect(state.discordModuleAliases).toEqual({ palworld: 'pw' });
        expect(state.discordCommandAliases).toEqual({ palworld: { start: '시작' } });
        expect(state.nodeSettings).toEqual({ guild1: { defaultHostId: 'h1' } });
    });

    it('syncNodeMusicSettings() — relay node settings를 store에 병합', async () => {
        mockApi({
            relayGetNodeSettings: vi.fn().mockResolvedValue({
                guildId: 'guild-1',
                musicEnabled: false,
                musicUIEnabled: true,
                musicSettings: { queueLines: 7, refreshInterval: 5000, normalize: false },
            }),
        });
        useDiscordStore.setState({
            discordCloudRelayUrl: 'https://relay.example.com',
            nodeSettings: { 'guild-1': { allowedInstances: ['srv-1'] } },
        });

        await useDiscordStore.getState().syncNodeMusicSettings('guild-1');

        expect(window.api.relayGetNodeSettings).toHaveBeenCalledWith('guild-1', 'https://relay.example.com');
        expect(useDiscordStore.getState().nodeSettings['guild-1']).toEqual({
            allowedInstances: ['srv-1'],
            musicEnabled: false,
            musicUIEnabled: true,
            musicSettings: { queueLines: 7, refreshInterval: 5000, normalize: false },
        });
    });

    it('updateNodeMusicSettings() — relay에 PATCH 후 store를 갱신', async () => {
        mockApi({
            relayUpdateNodeSettings: vi.fn().mockResolvedValue({ success: true }),
        });
        useDiscordStore.setState({
            discordCloudRelayUrl: 'https://relay.example.com',
            nodeSettings: { 'guild-1': { allowedInstances: [] } },
        });

        const result = await useDiscordStore.getState().updateNodeMusicSettings('guild-1', { musicEnabled: false });

        expect(result.success).toBe(true);
        expect(window.api.relayUpdateNodeSettings).toHaveBeenCalledWith(
            'guild-1',
            { musicEnabled: false },
            'https://relay.example.com',
        );
        expect(useDiscordStore.getState().nodeSettings['guild-1'].musicEnabled).toBe(false);
    });

    it('loadConfig() — 빈 설정이면 기본값 유지', async () => {
        mockApi({
            botConfigLoad: vi.fn().mockResolvedValue({}),
        });

        await useDiscordStore.getState().loadConfig();

        const state = useDiscordStore.getState();
        expect(state.discordPrefix).toBe('!saba');
    });

    it('loadConfig() — 실패 시 에러 모달 없이 조용히 실패', async () => {
        mockApi({
            botConfigLoad: vi.fn().mockRejectedValue(new Error('file not found')),
        });

        // 패닉이나 unhandled rejection 없어야 함
        await useDiscordStore.getState().loadConfig();
        // 상태는 기본값 그대로
        expect(useDiscordStore.getState().discordPrefix).toBe('!saba');
    });

    it('_resetForTest() — 완전 리셋', () => {
        useDiscordStore.setState({
            discordPrefix: '!!',
            discordCloudHostId: 'h-set',
            relayConnected: true,
        });

        useDiscordStore.getState()._resetForTest();

        const state = useDiscordStore.getState();
        expect(state.discordPrefix).toBe('!saba');
        expect(state.discordCloudHostId).toBe('');
        expect(state.relayConnected).toBe(false);
    });

    it('checkStatus() — hostId 없으면 relayConnected=false', async () => {
        useDiscordStore.setState({
            discordCloudHostId: '',
            _settingsReady: true,
            _botConfigLoaded: true,
        });

        await useDiscordStore.getState().checkStatus();

        expect(useDiscordStore.getState().relayConnected).toBe(false);
    });

    it('checkStatus() — hostId 있으면 relay 핑 후 결과 반영', async () => {
        mockApi({
            relayCheckHostStatus: vi.fn().mockResolvedValue({ id: 'host', online: true }),
        });
        useDiscordStore.setState({
            discordCloudHostId: 'activeHost789',
            _settingsReady: true,
            _botConfigLoaded: true,
        });

        await useDiscordStore.getState().checkStatus();

        expect(useDiscordStore.getState().relayConnected).toBe(true);
    });

    it('checkStatus() — 연결 실패 시 relayConnected=false', async () => {
        mockApi({
            relayCheckHostStatus: vi.fn().mockResolvedValue({ error: 'connection failed' }),
        });
        useDiscordStore.setState({
            discordCloudHostId: 'h',
            _settingsReady: true,
            _botConfigLoaded: true,
        });

        await useDiscordStore.getState().checkStatus();

        expect(useDiscordStore.getState().relayConnected).toBe(false);
    });

    // ── 히스테리시스: 일시적 실패(429/타임아웃)로 UI가 깜빡이지 않아야 함 ──

    it('checkStatus() 히스테리시스 — 연결 중 1회 일시 실패 시 relayConnected 유지', async () => {
        mockApi({
            relayCheckHostStatus: vi.fn().mockResolvedValue({ error: 'rate limited' }),
        });
        useDiscordStore.setState({
            discordCloudHostId: 'h',
            relayConnected: true,
            _statusFailStreak: 0,
        });

        await useDiscordStore.getState().checkStatus();

        expect(useDiscordStore.getState().relayConnected).toBe(true);
    });

    it('checkStatus() 히스테리시스 — 연속 3회 실패 시에만 relayConnected=false', async () => {
        mockApi({
            relayCheckHostStatus: vi.fn().mockResolvedValue({ error: 'down' }),
        });
        useDiscordStore.setState({
            discordCloudHostId: 'h',
            relayConnected: true,
            _statusFailStreak: 0,
        });

        await useDiscordStore.getState().checkStatus(); // 1회
        expect(useDiscordStore.getState().relayConnected).toBe(true);
        await useDiscordStore.getState().checkStatus(); // 2회
        expect(useDiscordStore.getState().relayConnected).toBe(true);
        await useDiscordStore.getState().checkStatus(); // 3회
        expect(useDiscordStore.getState().relayConnected).toBe(false);
    });

    it('checkStatus() 히스테리시스 — 실패 후 성공하면 스트릭 리셋', async () => {
        const mock = vi.fn()
            .mockResolvedValueOnce({ error: 'blip' })
            .mockResolvedValueOnce({ id: 'h', online: true })
            .mockResolvedValueOnce({ error: 'blip' })
            .mockResolvedValueOnce({ error: 'blip' });
        mockApi({ relayCheckHostStatus: mock });
        useDiscordStore.setState({
            discordCloudHostId: 'h',
            relayConnected: true,
            _statusFailStreak: 0,
        });

        await useDiscordStore.getState().checkStatus(); // 실패 1
        await useDiscordStore.getState().checkStatus(); // 성공 → 리셋
        await useDiscordStore.getState().checkStatus(); // 실패 1
        await useDiscordStore.getState().checkStatus(); // 실패 2 — 아직 유지
        expect(useDiscordStore.getState().relayConnected).toBe(true);
        expect(useDiscordStore.getState()._statusFailStreak).toBe(2);
    });

    it('checkStatus() — hostId 없으면 히스테리시스 없이 즉시 false', async () => {
        useDiscordStore.setState({
            discordCloudHostId: '',
            relayConnected: true,
            _statusFailStreak: 0,
        });

        await useDiscordStore.getState().checkStatus();

        expect(useDiscordStore.getState().relayConnected).toBe(false);
    });

    // ── relayStatusChecked: 첫 상태 확인 완료 여부 (무한 "연결 중" 스피너 방지) ──

    it('checkStatus() — 성공 시 relayStatusChecked=true', async () => {
        mockApi({
            relayCheckHostStatus: vi.fn().mockResolvedValue({ id: 'h', online: true }),
        });
        useDiscordStore.setState({ discordCloudHostId: 'h' });

        await useDiscordStore.getState().checkStatus();

        expect(useDiscordStore.getState().relayStatusChecked).toBe(true);
    });

    it('checkStatus() — 실패 시에도 relayStatusChecked=true (히스테리시스 유지 중 포함)', async () => {
        mockApi({
            relayCheckHostStatus: vi.fn().mockResolvedValue({ error: 'down' }),
        });
        useDiscordStore.setState({
            discordCloudHostId: 'h',
            relayConnected: true, // 히스테리시스로 연결 유지되는 첫 실패
            _statusFailStreak: 0,
        });

        await useDiscordStore.getState().checkStatus();

        expect(useDiscordStore.getState().relayStatusChecked).toBe(true);
        expect(useDiscordStore.getState().relayConnected).toBe(true); // 히스테리시스 유지
    });

    it('checkStatus() — hostId 없어도 relayStatusChecked=true (확인 완료로 간주)', async () => {
        useDiscordStore.setState({ discordCloudHostId: '' });

        await useDiscordStore.getState().checkStatus();

        expect(useDiscordStore.getState().relayStatusChecked).toBe(true);
    });

    it('_resetForTest() — relayStatusChecked도 초기화', async () => {
        useDiscordStore.setState({ relayStatusChecked: true });

        useDiscordStore.getState()._resetForTest();

        expect(useDiscordStore.getState().relayStatusChecked).toBe(false);
    });

    it('update() — hostId 변경 시 stale 연결 상태 리셋 후 즉시 재확인', async () => {
        const mock = vi.fn().mockResolvedValue({ id: 'newHost', online: true });
        mockApi({ relayCheckHostStatus: mock });
        useDiscordStore.setState({
            discordCloudHostId: 'oldHost',
            relayConnected: false,
            relayStatusChecked: true, // 이전 호스트 기준의 확인 결과 (stale)
            _statusFailStreak: 2,
        });

        useDiscordStore.getState().update({ discordCloudHostId: 'newHost' });

        // 동기적으로 stale 상태가 리셋되어 UI가 "연결 끊김"을 잘못 표시하지 않음
        expect(useDiscordStore.getState().relayStatusChecked).toBe(false);
        expect(useDiscordStore.getState()._statusFailStreak).toBe(0);

        // 폴링을 기다리지 않고 즉시 재확인이 수행됨
        await vi.waitFor(() => {
            expect(useDiscordStore.getState().relayStatusChecked).toBe(true);
        });
        expect(useDiscordStore.getState().relayConnected).toBe(true);
        expect(mock).toHaveBeenCalledTimes(1);
    });

    it('update() — hostId 제거(연결 해제) 시 재확인 없이 리셋만 수행', () => {
        const mock = vi.fn().mockResolvedValue({ id: 'h', online: true });
        mockApi({ relayCheckHostStatus: mock });
        useDiscordStore.setState({
            discordCloudHostId: 'h',
            relayConnected: true,
            relayStatusChecked: true,
        });

        useDiscordStore.getState().update({ discordCloudHostId: '' });

        expect(useDiscordStore.getState().relayConnected).toBe(false);
        expect(mock).not.toHaveBeenCalled();
    });

    it('update() — hostId 외 필드 변경은 연결 상태에 영향 없음', () => {
        useDiscordStore.setState({
            discordCloudHostId: 'h',
            relayConnected: true,
            relayStatusChecked: true,
        });

        useDiscordStore.getState().update({ discordPrefix: '!x' });

        expect(useDiscordStore.getState().relayConnected).toBe(true);
        expect(useDiscordStore.getState().relayStatusChecked).toBe(true);
    });
});

// ═════════════════════════════════════════════════════════════
// 5. (구) 크로스 스토어 동기화 — 로컬 봇 제거로 폐기됨
// ═════════════════════════════════════════════════════════════
// token/autoStart 필드는 로컬 봇과 함께 제거되어 더 이상 cross-store sync가 없음.

// ═════════════════════════════════════════════════════════════
// 6. 에지 케이스
// ═════════════════════════════════════════════════════════════

describe('에지 케이스', () => {
    it('여러 스토어 동시 리셋 후 서로 독립적이어야 한다', () => {
        useSettingsStore.setState({ ipcPort: 1111 });
        useServerStore.setState({ daemonReady: true });
        useDiscordStore.setState({ discordPrefix: '!sentinel' });

        useSettingsStore.getState()._resetForTest();
        // 다른 스토어의 상태는 변경되지 않아야 함
        expect(useServerStore.getState().daemonReady).toBe(true);
        expect(useDiscordStore.getState().discordPrefix).toBe('!sentinel');
    });

    it('settings load() → save() 왕복 — 저장된 값이 로드된 값과 동일', async () => {
        const original = {
            autoRefresh: false,
            refreshInterval: 7777,
            ipcPort: 33333,
            consoleBufferSize: 100,
            autoGeneratePasswords: false,
            portConflictCheck: false,
        };

        // 캡처용
        let savedPayload = null;
        mockApi({
            settingsLoad: vi.fn().mockResolvedValue(original),
            settingsSave: vi.fn().mockImplementation(async (data) => {
                savedPayload = data;
                return { success: true };
            }),
            settingsGetPath: vi.fn().mockResolvedValue('/test'),
        });

        await useSettingsStore.getState().load();
        await useSettingsStore.getState().save();

        expect(savedPayload.autoRefresh).toBe(original.autoRefresh);
        expect(savedPayload.refreshInterval).toBe(original.refreshInterval);
        expect(savedPayload.ipcPort).toBe(original.ipcPort);
        expect(savedPayload.consoleBufferSize).toBe(original.consoleBufferSize);
        expect(savedPayload.modulesPath).toBeUndefined();
    });

    it('serverStore.setServers — 빈 배열 설정 가능', () => {
        useServerStore.getState().setServers([{ id: 'x' }]);
        expect(useServerStore.getState().servers).toHaveLength(1);

        useServerStore.getState().setServers([]);
        expect(useServerStore.getState().servers).toEqual([]);
    });

    it('uiStore — 같은 탭으로 재전환해도 안전', () => {
        useUIStore.getState().setActiveTab('settings');
        useUIStore.getState().setActiveTab('settings');
        expect(useUIStore.getState().activeTab).toBe('settings');
    });
});
