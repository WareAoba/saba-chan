/**
 * G-D7: 에이전트 상태 확인(3초 제한)이 한 번 늦거나 실패했다고 'stopped'로 보이면 안 된다.
 * 브리지는 모름(unknown)을 따로 주고, 스토어는 릴레이 확인처럼 두 번 연달아 실패해야 꺼짐으로 본다.
 */
import { beforeEach, describe, expect, it, vi } from 'vitest';
import { createApi } from '../api/createApi';
import { useDiscordStore } from '../stores/useDiscordStore';

describe('에이전트 상태 확인이 한 번 실패할 때', () => {
    beforeEach(() => {
        useDiscordStore.getState()._resetForTest();
    });

    it('브리지는 응답을 못 받으면 꺼짐 대신 unknown을 준다', async () => {
        const invoke = vi.fn().mockRejectedValueOnce(new Error('timeout of 3000ms exceeded'));
        const { api } = createApi({ invoke, hub: { dispatch: vi.fn() } });
        expect(await api.discordBotStatus()).toBe('unknown');
    });

    it('한 번 모르면 지금 상태를 두고, 두 번 연달아 모르면 꺼짐으로 본다', async () => {
        window.api = {
            discordBotStatus: vi.fn().mockResolvedValue('running'),
            relayCheckHostStatus: vi.fn().mockResolvedValue({ online: true }),
        };
        useDiscordStore.setState({ discordCloudHostId: 'host-1', _botConfigLoaded: true });
        await useDiscordStore.getState().checkStatus();
        expect(useDiscordStore.getState().discordBotStatus).toBe('running');

        window.api.discordBotStatus.mockResolvedValue('unknown');
        await useDiscordStore.getState().checkStatus();
        expect(useDiscordStore.getState().discordBotStatus).toBe('running');

        await useDiscordStore.getState().checkStatus();
        expect(useDiscordStore.getState().discordBotStatus).toBe('stopped');

        // 다시 답하면 세던 실패는 지운다
        window.api.discordBotStatus.mockResolvedValue('running');
        await useDiscordStore.getState().checkStatus();
        window.api.discordBotStatus.mockResolvedValue('unknown');
        await useDiscordStore.getState().checkStatus();
        expect(useDiscordStore.getState().discordBotStatus).toBe('running');
    });
});
