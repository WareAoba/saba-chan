import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import { useDiscordStore } from '../stores/useDiscordStore';

function deferred() {
    let resolve;
    const promise = new Promise((done) => {
        resolve = done;
    });
    return { promise, resolve };
}

beforeEach(() => {
    vi.useFakeTimers();
    useDiscordStore.getState()._resetForTest();
    window.api = {
        discordBotStatus: vi.fn().mockResolvedValue('running'),
        relayCheckHostStatus: vi.fn().mockResolvedValue({ error: 'offline' }),
    };
});

afterEach(() => {
    useDiscordStore.getState().stopStatusPolling();
    vi.clearAllTimers();
    vi.useRealTimers();
});

describe('릴레이 상태 확인 수명', () => {
    it('연결 대상이 바뀐 뒤 도착한 옛 호스트 응답은 버린다', async () => {
        const oldHost = deferred();
        window.api.relayCheckHostStatus.mockReturnValueOnce(oldHost.promise);
        useDiscordStore.setState({ discordCloudHostId: 'old-host' });
        const oldCheck = useDiscordStore.getState().checkStatus();
        await Promise.resolve();

        useDiscordStore.getState().update({ discordCloudHostId: 'new-host' });
        oldHost.resolve({ connected: true });
        await oldCheck;

        const s = useDiscordStore.getState();
        expect(s.discordCloudHostId).toBe('new-host');
        expect(s.relayConnected).toBe(false);
        expect(s.discordBotStatus).toBe('stopped');
    });

    it('폴링을 멈추면 미뤄 둔 첫 확인도 시작하지 않는다', async () => {
        useDiscordStore.setState({ discordCloudHostId: 'host' });
        useDiscordStore.getState().startStatusPolling();
        useDiscordStore.getState().stopStatusPolling();
        await vi.advanceTimersByTimeAsync(6000);
        expect(window.api.discordBotStatus).not.toHaveBeenCalled();
        expect(window.api.relayCheckHostStatus).not.toHaveBeenCalled();
    });
});

describe('데몬보다 앱이 먼저 뜬 경우', () => {
    it('처음 설정 읽기에 실패해도 데몬이 뜬 뒤 상태 확인에서 다시 읽어 연결을 이어 간다', async () => {
        window.api.botConfigLoad = vi.fn().mockRejectedValueOnce(new Error('ECONNREFUSED'));
        await useDiscordStore.getState().loadConfig();
        expect(useDiscordStore.getState().discordCloudHostId).toBe('');

        window.api.botConfigLoad.mockResolvedValue({ cloud: { hostId: 'h1', relayUrl: 'https://r' } });
        window.api.relayCheckHostStatus.mockResolvedValue({ id: 'h1' });
        await useDiscordStore.getState().checkStatus();

        const s = useDiscordStore.getState();
        expect(s.discordCloudHostId).toBe('h1');
        expect(s.relayConnected).toBe(true);
        expect(window.api.relayCheckHostStatus).toHaveBeenCalledWith('h1', 'https://r');
    });
});

describe('늦게 읽은 설정과 그 전에 고친 값', () => {
    it('설정을 읽기 전에 고친 값은 늦게 읽은 값으로 덮지 않고 데몬에 저장한다', async () => {
        window.api.botConfigLoad = vi.fn().mockRejectedValueOnce(new Error('ECONNREFUSED'));
        window.api.botConfigSave = vi.fn().mockResolvedValue({ success: true });
        await useDiscordStore.getState().loadConfig();
        useDiscordStore.getState().update({ discordPrefixes: ['!new'] });

        window.api.botConfigLoad.mockResolvedValue({
            prefixes: ['!old'],
            cloud: { hostId: 'h1', relayUrl: 'https://r' },
        });
        await useDiscordStore.getState().checkStatus();

        const s = useDiscordStore.getState();
        expect(s.discordPrefixes).toEqual(['!new']);
        expect(s.discordCloudHostId).toBe('h1');
        expect(window.api.botConfigSave).toHaveBeenCalledWith({ prefixes: ['!new'] });
    });
});

describe('봇 설정 로드 실패 시 저장', () => {
    it('처음 로드 전에 저장하면 바꾼 키만 보내 디스크의 접두사·호스트를 건드리지 않는다', async () => {
        window.api.botConfigLoad = vi.fn().mockResolvedValue({
            prefix: '!custom',
            cloud: { hostId: 'host', relayUrl: 'https://r' },
            moduleAliases: { minecraft: 'mc' },
        });
        window.api.botConfigSave = vi.fn().mockResolvedValue({ success: true });
        useDiscordStore.setState({ nodeSettings: { g1: { allowedInstances: [] } } });
        expect(await useDiscordStore.getState().saveConfig()).toBe(true);
        expect(window.api.botConfigSave.mock.calls[0][0]).toEqual({ nodeSettings: { g1: { allowedInstances: [] } } });
    });

    it('데몬에서 설정을 못 읽으면 기본값으로 덮어쓰지 않는다', async () => {
        window.api.botConfigLoad = vi.fn().mockResolvedValue(null);
        window.api.botConfigSave = vi.fn();
        expect(await useDiscordStore.getState().saveConfig()).toBe(false);
        expect(window.api.botConfigSave).not.toHaveBeenCalled();
    });

    it('정상 로드된 뒤에는 바꾼 값만 보낸다', async () => {
        window.api.botConfigLoad = vi.fn().mockResolvedValue({ cloud: { hostId: 'h1', relayUrl: 'https://r' } });
        window.api.botConfigSave = vi.fn().mockResolvedValue({ success: true });
        useDiscordStore.setState({ _botConfigLoaded: true, discordCloudHostId: 'h2' });
        await useDiscordStore.getState().saveConfig();
        // relayUrl은 바꾸지 않았으므로 싣지 않는다 (PATCH가 데몬의 지금 값을 둔다)
        expect(window.api.botConfigSave.mock.calls[0][0]).toEqual({ cloud: { hostId: 'h2' } });
    });

    it('노드 캐시만 바뀌면 앱 시작 때 읽은 nodeSettings·접두사를 다시 보내지 않는다 (CLI가 바꾼 값을 되돌리지 않게)', async () => {
        window.api.botConfigLoad = vi.fn().mockResolvedValue({
            prefixes: ['!a'],
            cloud: { hostId: 'h1', relayUrl: 'https://r' },
            nodeSettings: { g1: {} },
            cloudNodes: [{ guildId: 'g1' }],
        });
        window.api.botConfigSave = vi.fn().mockResolvedValue({ success: true });
        await useDiscordStore.getState().loadConfig();
        // 같은 내용의 새 배열은 바뀐 것이 아니다
        useDiscordStore.setState({ cloudNodes: [{ guildId: 'g1' }] });
        await useDiscordStore.getState().saveConfig();
        expect(window.api.botConfigSave).not.toHaveBeenCalled();

        useDiscordStore.setState({ cloudNodes: [{ guildId: 'g1' }, { guildId: 'g2' }] });
        await useDiscordStore.getState().saveConfig();
        expect(window.api.botConfigSave.mock.calls[0][0]).toEqual({
            cloudNodes: [{ guildId: 'g1' }, { guildId: 'g2' }],
        });

        // 저장한 값은 다시 보내지 않는다
        await useDiscordStore.getState().saveConfig();
        expect(window.api.botConfigSave).toHaveBeenCalledTimes(1);
    });
});
