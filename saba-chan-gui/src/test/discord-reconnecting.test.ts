/**
 * 사용자 11-11: 붙어 있던 봇이 릴레이와 끊겨 다시 붙는 중이면 노란 깜빡이(복구 중), 꺼짐에서 켜는 중이면 회색 깜빡이.
 * 스토어가 "다시 붙는 중"을 기억해 화면이 고른다.
 */
import { beforeEach, describe, expect, it, vi } from 'vitest';
import { useDiscordStore } from '../stores/useDiscordStore';

describe('봇 다시 붙는 중', () => {
    beforeEach(() => {
        useDiscordStore.getState()._resetForTest();
        window.api = {
            discordBotStatus: vi.fn().mockResolvedValue({ status: 'running', running: true, link: { connected: false, error: '', heard: true } }),
            relayCheckHostStatus: vi.fn().mockResolvedValue({ online: true }),
        } as unknown as typeof window.api;
        useDiscordStore.setState({ discordCloudHostId: 'host-1' });
    });

    it('처음 켜는 중은 다시 붙는 중이 아니다', async () => {
        vi.mocked(window.api.relayCheckHostStatus).mockResolvedValue({ error: 'down' } as never);
        await useDiscordStore.getState().checkStatus();
        await useDiscordStore.getState().checkStatus();
        expect(useDiscordStore.getState().discordBotStatus).toBe('connecting');
        expect(useDiscordStore.getState().discordReconnecting).toBe(false);
    });

    it('실행 중이던 봇이 끊기면 다시 붙는 중, 다시 붙으면 풀린다', async () => {
        await useDiscordStore.getState().checkStatus();
        expect(useDiscordStore.getState().discordBotStatus).toBe('running');
        vi.mocked(window.api.relayCheckHostStatus).mockResolvedValue({ error: 'down' } as never);
        await useDiscordStore.getState().checkStatus();
        await useDiscordStore.getState().checkStatus();
        expect(useDiscordStore.getState().discordBotStatus).toBe('connecting');
        expect(useDiscordStore.getState().discordReconnecting).toBe(true);
        vi.mocked(window.api.relayCheckHostStatus).mockResolvedValue({ online: true });
        await useDiscordStore.getState().checkStatus();
        expect(useDiscordStore.getState().discordReconnecting).toBe(false);
    });
});
