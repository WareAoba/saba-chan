/**
 * 얇은 클라이언트 점검 1번 (2026-11-12): 앱이 5초마다 데몬을 시켜 릴레이에 다녀오던 연결 확인을 없앴다.
 * 데몬 에이전트가 하트비트로 아는 연결 상태(`connected`)를 그대로 쓰고, 바뀌면 `relay.status` 이벤트로 바로 다시 읽는다.
 * 그 값을 주지 않는 옛 데몬과 꺼 둔 봇은 예전처럼 릴레이에 묻고, 끊겼을 때도 왜인지(토큰 거절 등) 릴레이에 묻는다.
 */
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import { useDiscordStore } from '../stores/useDiscordStore';

let listeners: ((ev: unknown) => void)[] = [];
const link = (connected: boolean, error = '', heard = true) => ({ status: 'running', running: true, link: { connected, error, heard } });

beforeEach(() => {
    vi.useFakeTimers();
    listeners = [];
    useDiscordStore.getState()._resetForTest();
    window.api = {
        discordBotStatus: vi.fn().mockResolvedValue(link(true)),
        relayCheckHostStatus: vi.fn().mockResolvedValue({ ok: true }),
        onDaemonEvent: vi.fn((cb: (ev: unknown) => void) => {
            listeners.push(cb);
            return () => {
                listeners = listeners.filter((l) => l !== cb);
            };
        }),
    } as unknown as typeof window.api;
    useDiscordStore.setState({ discordCloudHostId: 'host-1', _botConfigLoaded: true });
});

afterEach(() => {
    useDiscordStore.getState().stopStatusPolling();
    vi.clearAllTimers();
    vi.useRealTimers();
});

describe('릴레이 연결은 데몬 에이전트가 안다', () => {
    it('데몬이 연결 상태를 주면 릴레이에 따로 묻지 않는다', async () => {
        await useDiscordStore.getState().checkStatus();
        const s = useDiscordStore.getState();
        expect(s.relayConnected).toBe(true);
        expect(s.relayChecked).toBe(true);
        expect(s.discordBotStatus).toBe('running');
        expect(window.api.relayCheckHostStatus).not.toHaveBeenCalled();
    });

    it('끊겼으면 왜인지 릴레이에 묻는다: 토큰 거절이면 다시 연결해야 하는 실패다 (두 번 연달아일 때만 끊김으로 본다)', async () => {
        await useDiscordStore.getState().checkStatus();
        vi.mocked(window.api.discordBotStatus).mockResolvedValue(link(false, 'heartbeat 401 (node token rejected)') as never);
        vi.mocked(window.api.relayCheckHostStatus).mockResolvedValue({ error: 'INVALID_TOKEN' } as never);
        await useDiscordStore.getState().checkStatus();
        expect(useDiscordStore.getState().relayConnected).toBe(true);
        await useDiscordStore.getState().checkStatus();
        const s = useDiscordStore.getState();
        expect(s.relayConnected).toBe(false);
        expect(s.relayError).toBe('INVALID_TOKEN');
        expect(s.relayErrorKind).toBe('rejected');
        expect(s.discordBotStatus).toBe('connecting');
        expect(s.discordReconnecting).toBe(false);
    });

    it('에이전트가 다시 붙으면 거절을 기다리지 않고 바로 연결로 본다', async () => {
        useDiscordStore.setState({ relayError: 'INVALID_TOKEN', relayErrorKind: 'rejected', _relayCheckedAt: Date.now() });
        await useDiscordStore.getState().checkStatus();
        const s = useDiscordStore.getState();
        expect(s.relayConnected).toBe(true);
        expect(s.relayError).toBe('');
        expect(s.discordBotStatus).toBe('running');
    });

    it('막 켠 봇이 아직 하트비트를 못 했으면 끊김이 아니라 연결 중이다', async () => {
        vi.mocked(window.api.discordBotStatus).mockResolvedValue(link(false, '', false) as never);
        await useDiscordStore.getState().checkStatus();
        await useDiscordStore.getState().checkStatus();
        const s = useDiscordStore.getState();
        expect(s.relayError).toBeFalsy();
        expect(s.discordReconnecting).toBe(false);
        expect(s.discordBotStatus).toBe('connecting');
    });

    it('끊긴 것 같으면 30초를 기다리지 않고 5초 뒤 다시 보고 끊김을 정한다', async () => {
        useDiscordStore.getState().startStatusPolling();
        await vi.advanceTimersByTimeAsync(300);
        vi.mocked(window.api.discordBotStatus).mockResolvedValue(link(false, 'poll 502') as never);
        vi.mocked(window.api.relayCheckHostStatus).mockResolvedValue({ error: 'connect ECONNREFUSED' } as never);
        for (const l of listeners) l({ type: 'relay.status', data: {} });
        await vi.advanceTimersByTimeAsync(10);
        expect(useDiscordStore.getState().relayConnected).toBe(true);
        await vi.advanceTimersByTimeAsync(5_100);
        expect(useDiscordStore.getState().relayConnected).toBe(false);
    });

    it('봇을 꺼 두었으면 에이전트가 모르므로 릴레이에 묻는다', async () => {
        vi.mocked(window.api.discordBotStatus).mockResolvedValue({ status: 'stopped', running: false, link: { connected: false, error: '' } } as never);
        await useDiscordStore.getState().checkStatus();
        expect(window.api.relayCheckHostStatus).toHaveBeenCalledWith('host-1', expect.any(String));
        expect(useDiscordStore.getState().relayConnected).toBe(true);
    });

    it('relay.status 이벤트가 오면 바로 다시 읽고, 평소에는 30초에 한 번만 묻는다', async () => {
        useDiscordStore.getState().startStatusPolling();
        await vi.advanceTimersByTimeAsync(300);
        expect(window.api.discordBotStatus).toHaveBeenCalledTimes(1);

        await vi.advanceTimersByTimeAsync(10_000);
        expect(window.api.discordBotStatus).toHaveBeenCalledTimes(1);

        for (const l of listeners) l({ type: 'relay.status', data: {} });
        await vi.advanceTimersByTimeAsync(0);
        expect(window.api.discordBotStatus).toHaveBeenCalledTimes(2);

        await vi.advanceTimersByTimeAsync(30_000);
        expect(window.api.discordBotStatus).toHaveBeenCalledTimes(3);
        expect(window.api.relayCheckHostStatus).not.toHaveBeenCalled();
    });

    it('연결 상태를 주지 않는 옛 데몬이면 예전처럼 5초마다 릴레이에 묻는다', async () => {
        vi.mocked(window.api.discordBotStatus).mockResolvedValue('running' as never);
        useDiscordStore.getState().startStatusPolling();
        await vi.advanceTimersByTimeAsync(300);
        await vi.advanceTimersByTimeAsync(5_000);
        expect(window.api.relayCheckHostStatus).toHaveBeenCalledTimes(2);
    });
});
