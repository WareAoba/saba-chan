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

describe('봇 설정 로드 실패 시 저장', () => {
    it('처음 로드 전에 저장하면 디스크의 접두사·호스트를 지키고 바꾼 값만 얹는다', async () => {
        window.api.botConfigLoad = vi.fn().mockResolvedValue({
            prefix: '!custom',
            cloud: { hostId: 'host', relayUrl: 'https://r' },
            moduleAliases: { minecraft: 'mc' },
        });
        window.api.botConfigSave = vi.fn().mockResolvedValue({ success: true });
        useDiscordStore.setState({ nodeSettings: { g1: { allowedInstances: [] } } });
        expect(await useDiscordStore.getState().saveConfig()).toBe(true);
        const payload = window.api.botConfigSave.mock.calls[0][0];
        expect(payload.prefixes).toEqual(['!custom']);
        expect(payload.cloud).toEqual({ relayUrl: 'https://r', hostId: 'host' });
        expect(payload.moduleAliases).toEqual({ minecraft: 'mc' });
        expect(payload.nodeSettings).toEqual({ g1: { allowedInstances: [] } });
    });

    it('데몬에서 설정을 못 읽으면 기본값으로 덮어쓰지 않는다', async () => {
        window.api.botConfigLoad = vi.fn().mockResolvedValue(null);
        window.api.botConfigSave = vi.fn();
        expect(await useDiscordStore.getState().saveConfig()).toBe(false);
        expect(window.api.botConfigSave).not.toHaveBeenCalled();
    });

    it('정상 로드된 뒤에는 다시 읽지 않고 스토어 값을 그대로 보낸다', async () => {
        window.api.botConfigLoad = vi.fn();
        window.api.botConfigSave = vi.fn().mockResolvedValue({ success: true });
        useDiscordStore.setState({ _botConfigLoaded: true, discordCloudHostId: '' });
        await useDiscordStore.getState().saveConfig();
        expect(window.api.botConfigLoad).not.toHaveBeenCalled();
        expect(window.api.botConfigSave.mock.calls[0][0].cloud.hostId).toBe('');
    });
});
