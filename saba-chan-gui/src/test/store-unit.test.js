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
            discordBotStatus: vi.fn().mockResolvedValue('stopped'),
            discordBotStart: vi.fn().mockResolvedValue({ success: true }),
            discordBotStop: vi.fn().mockResolvedValue({ success: true }),
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

    it('_setDiscordFields() — discord 토큰과 autoStart 저장', () => {
        useSettingsStore.getState()._setDiscordFields('my-token-123', true);

        const state = useSettingsStore.getState();
        expect(state._discordToken).toBe('my-token-123');
        expect(state._discordAutoStart).toBe(true);
    });

    it('update() — 부분 업데이트 적용', () => {
        useSettingsStore.getState().update({ autoRefresh: false, ipcPort: 7777 });

        const state = useSettingsStore.getState();
        expect(state.autoRefresh).toBe(false);
        expect(state.ipcPort).toBe(7777);
        expect(state.refreshInterval).toBe(2000); // 변경 안 됨
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
// 4. useDiscordStore
// ═════════════════════════════════════════════════════════════

describe('useDiscordStore', () => {
    it('초기 상태 검증', () => {
        const state = useDiscordStore.getState();
        expect(state.discordToken).toBe('');
        expect(state.discordPrefix).toBe('!saba');
        expect(state.discordAutoStart).toBe(false);
        expect(state.discordMusicEnabled).toBe(true);
        expect(state.discordBotStatus).toBe('stopped');
        expect(state.discordBotMode).toBe('local');
        expect(state.botStatusReady).toBe(false);
    });

    it('setDiscordToken() — 토큰 설정 + ref 동기화', () => {
        useDiscordStore.getState().setDiscordToken('tok-secret-123');
        const state = useDiscordStore.getState();
        expect(state.discordToken).toBe('tok-secret-123');
        expect(state._discordTokenRef).toBe('tok-secret-123');
    });

    it('update() — 부분 업데이트', () => {
        useDiscordStore.getState().update({
            discordPrefix: '!s',
            discordMusicEnabled: false,
        });
        const state = useDiscordStore.getState();
        expect(state.discordPrefix).toBe('!s');
        expect(state.discordMusicEnabled).toBe(false);
        expect(state.discordAutoStart).toBe(false); // 미변경
    });

    it('loadConfig() — bot-config에서 모든 필드 로드', async () => {
        mockApi({
            botConfigLoad: vi.fn().mockResolvedValue({
                prefix: '!bot',
                mode: 'cloud',
                cloud: { relayUrl: 'https://relay.example.com', hostId: 'h123' },
                moduleAliases: { palworld: 'pw' },
                commandAliases: { palworld: { start: '시작' } },
                musicEnabled: false,
                nodeSettings: { guild1: { defaultHostId: 'h1' } },
            }),
        });

        await useDiscordStore.getState().loadConfig();

        const state = useDiscordStore.getState();
        expect(state.discordPrefix).toBe('!bot');
        expect(state.discordBotMode).toBe('cloud');
        expect(state.discordCloudRelayUrl).toBe('https://relay.example.com');
        expect(state.discordCloudHostId).toBe('h123');
        expect(state.discordModuleAliases).toEqual({ palworld: 'pw' });
        expect(state.discordCommandAliases).toEqual({ palworld: { start: '시작' } });
        expect(state.discordMusicEnabled).toBe(false);
        expect(state.nodeSettings).toEqual({ guild1: { defaultHostId: 'h1' } });
    });

    it('loadConfig() — 빈 설정이면 기본값 유지', async () => {
        mockApi({
            botConfigLoad: vi.fn().mockResolvedValue({}),
        });

        await useDiscordStore.getState().loadConfig();

        const state = useDiscordStore.getState();
        expect(state.discordPrefix).toBe('!saba');
        expect(state.discordBotMode).toBe('local');
        expect(state.discordMusicEnabled).toBe(true);
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
            discordToken: 'xxx',
            discordPrefix: '!!',
            discordBotStatus: 'running',
            botStatusReady: true,
        });

        useDiscordStore.getState()._resetForTest();

        const state = useDiscordStore.getState();
        expect(state.discordToken).toBe('');
        expect(state.discordPrefix).toBe('!saba');
        expect(state.discordBotStatus).toBe('stopped');
        expect(state.botStatusReady).toBe(false);
    });
});

// ═════════════════════════════════════════════════════════════
// 5. 크로스 스토어 동기화
// ═════════════════════════════════════════════════════════════

describe('크로스 스토어 동기화', () => {
    it('discord 토큰은 settings 저장 시 함께 직렬화되어야 한다', async () => {
        useSettingsStore.setState({ settingsPath: '/mock/settings.json' });
        useSettingsStore.getState()._setDiscordFields('tok-cross-test', true);

        await useSettingsStore.getState().save();

        const payload = window.api.settingsSave.mock.calls[0][0];
        expect(payload.discordToken).toBe('tok-cross-test');
        expect(payload.discordAutoStart).toBe(true);
    });

    it('settings 리셋은 discord 내부 필드도 초기화해야 한다', () => {
        useSettingsStore.getState()._setDiscordFields('tok-xxx', true);
        expect(useSettingsStore.getState()._discordToken).toBe('tok-xxx');

        useSettingsStore.getState()._resetForTest();
        expect(useSettingsStore.getState()._discordToken).toBe('');
        expect(useSettingsStore.getState()._discordAutoStart).toBe(false);
    });
});

// ═════════════════════════════════════════════════════════════
// 6. 에지 케이스
// ═════════════════════════════════════════════════════════════

describe('에지 케이스', () => {
    it('여러 스토어 동시 리셋 후 서로 독립적이어야 한다', () => {
        useSettingsStore.setState({ ipcPort: 1111 });
        useServerStore.setState({ daemonReady: true });
        useDiscordStore.setState({ discordBotStatus: 'running' });

        useSettingsStore.getState()._resetForTest();
        // 다른 스토어의 상태는 변경되지 않아야 함
        expect(useServerStore.getState().daemonReady).toBe(true);
        expect(useDiscordStore.getState().discordBotStatus).toBe('running');
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
