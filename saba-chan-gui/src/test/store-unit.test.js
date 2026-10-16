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
            moduleGetMetadata: vi.fn().mockResolvedValue({ metadata: {} }),
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
        expect(state.useGradient).toBe(false);
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
        expect(state.useGradient).toBe(false); // 변경 안 됨
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
        expect(callArgs.useGradient).toBe(false);
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
// 4. useDiscordStore
// ═════════════════════════════════════════════════════════════

describe('useDiscordStore', () => {
    it('초기 상태 검증', () => {
        const state = useDiscordStore.getState();
        expect(state.discordPrefix).toBe('!saba');
        expect(state.discordBotStatus).toBe('stopped');
        expect(state.botStatusReady).toBe(false);
        // 로컬 봇 시절 필드는 더 이상 없다
        expect(state.discordToken).toBeUndefined();
        expect(state.discordBotMode).toBeUndefined();
    });

    it('update() — 부분 업데이트', () => {
        useDiscordStore.getState().update({ discordPrefix: '!s' });
        const state = useDiscordStore.getState();
        expect(state.discordPrefix).toBe('!s');
        expect(state.discordCloudHostId).toBe(''); // 미변경
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
        expect(state._botConfigLoaded).toBe(true);
    });

    it('loadConfig() — 빈 설정이면 기본값 유지', async () => {
        mockApi({
            botConfigLoad: vi.fn().mockResolvedValue({}),
        });

        await useDiscordStore.getState().loadConfig();

        const state = useDiscordStore.getState();
        expect(state.discordPrefix).toBe('!saba');
        expect(state.discordCloudRelayUrl).toBe('');
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
            discordBotStatus: 'running',
            botStatusReady: true,
        });

        useDiscordStore.getState()._resetForTest();

        const state = useDiscordStore.getState();
        expect(state.discordPrefix).toBe('!saba');
        expect(state.discordBotStatus).toBe('stopped');
        expect(state.botStatusReady).toBe(false);
    });
});

// ═════════════════════════════════════════════════════════════
// 5. 크로스 스토어 동기화
// ═════════════════════════════════════════════════════════════

describe('bot-config 저장 계약', () => {
    it('settings save payload에는 디스코드 필드가 없다', async () => {
        useSettingsStore.setState({ settingsPath: '/mock/settings.json' });
        await useSettingsStore.getState().save();

        const payload = window.api.settingsSave.mock.calls[0][0];
        expect(payload.discordToken).toBeUndefined();
        expect(payload.discordAutoStart).toBeUndefined();
    });

    it('bot-config save payload에는 로컬 봇 필드(token/autoStart/mode/music)가 없다', async () => {
        useDiscordStore.setState({ discordPrefix: '!x', discordCloudHostId: 'h1' });

        await useDiscordStore.getState().saveConfig();

        const payload = window.api.botConfigSave.mock.calls[0][0];
        expect(payload.prefix).toBe('!x');
        expect(payload.cloud).toEqual({ relayUrl: '', hostId: 'h1' });
        for (const legacy of ['token', 'autoStart', 'mode', 'musicEnabled', 'musicChannelId', 'musicUISettings']) {
            expect(payload).not.toHaveProperty(legacy);
        }
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

describe('useServerStore 모듈 별명 (파싱된 메타데이터)', () => {
    it('metadata의 module_aliases·command_aliases·commands로 별명 표를 만든다', async () => {
        mockApi({
            moduleList: vi.fn().mockResolvedValue({ modules: [{ name: 'palworld' }] }),
            moduleGetMetadata: vi.fn().mockResolvedValue({
                metadata: {
                    module_aliases: ['팰월드', 'pw'],
                    command_aliases: { players: { aliases: ['접속자'], description: '접속자 목록' } },
                    commands: { fields: [{ name: 'players', label: '접속자' }, { name: 'save', label: '저장' }] },
                },
            }),
        });
        await useServerStore.getState().fetchModules();
        const map = useServerStore.getState().moduleAliasesPerModule.palworld;
        expect(map.module_aliases).toEqual(['팰월드', 'pw']);
        expect(map.commands.players).toEqual({ aliases: ['접속자'], description: '접속자 목록', label: '접속자' });
        expect(map.commands.save.label).toBe('저장');
    });
});
