/**
 * 저장된 릴레이 주소가 https(루프백만 http)가 아니면 데몬은 relay 에이전트를 띄우지 않고
 * 상태 lastErrorCode와 시작 409에 "invalid_relay_url"을 준다. GUI는 영문 원문 대신 주소 안내를 보인다.
 */
import { render, screen } from '@testing-library/react';
import { beforeEach, describe, expect, it, vi } from 'vitest';
import { createApi } from '../api/createApi';
import { ConnectionView } from '../components/discord/ConnectionView';
import { useDiscordStore } from '../stores/useDiscordStore';
import { createTranslateError } from '../utils/helpers';

const viewProps = {
    cloudState: 'connected',
    hostId: 'host-1',
    relayUrl: 'http://relay.example',
    nodeCount: 1,
    botStatus: 'stopped',
    pairing: { active: false },
    onStartPairing: () => {},
    onCancelPairing: () => {},
    onDisconnect: () => {},
    onStartAgent: () => {},
    onStopAgent: () => {},
};

describe('거절된 릴레이 주소로 멈춘 에이전트', () => {
    beforeEach(() => {
        useDiscordStore.getState()._resetForTest();
    });

    it('브리지가 상태와 시작 실패를 코드로 넘긴다', async () => {
        const invoke = vi
            .fn()
            .mockResolvedValueOnce({ status: 200, data: { running: false, lastErrorCode: 'invalid_relay_url' } })
            .mockResolvedValueOnce({
                status: 409,
                data: { error: 'relay URL must use https', error_code: 'invalid_relay_url' },
            });
        const { api } = createApi({ invoke, hub: { dispatch: vi.fn() } });
        expect(await api.discordBotStatus()).toBe('invalid_relay_url');
        expect(await api.discordBotStart()).toEqual({ error: 'invalid_relay_url' });
    });

    it('시작 실패 문구는 주소 안내로 바꾼다', () => {
        const t = (key) => key;
        expect(createTranslateError(t)('invalid_relay_url')).toBe('discord.invalid_relay_url');
    });

    it('상태가 invalid_relay_url이면 꺼짐으로 두고 이유를 기억한다', async () => {
        window.api = {
            discordBotStatus: vi.fn().mockResolvedValue('invalid_relay_url'),
            relayCheckHostStatus: vi.fn().mockResolvedValue({ online: true }),
        };
        useDiscordStore.setState({ discordCloudHostId: 'host-1' });
        await useDiscordStore.getState().checkStatus();
        expect(useDiscordStore.getState().discordBotStatus).toBe('stopped');
        expect(useDiscordStore.getState().discordAgentRelayUrlInvalid).toBe(true);

        window.api.discordBotStatus.mockResolvedValue('running');
        await useDiscordStore.getState().checkStatus();
        expect(useDiscordStore.getState().discordAgentRelayUrlInvalid).toBe(false);
    });

    it('연결 화면이 주소 형식을 알려 준다', () => {
        const { rerender } = render(<ConnectionView {...viewProps} />);
        expect(screen.queryByText(/must start with https/)).toBeNull();
        rerender(<ConnectionView {...viewProps} agentRelayUrlInvalid />);
        expect(screen.getByText(/must start with https/)).toBeInTheDocument();
    });
});
