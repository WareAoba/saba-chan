/**
 * 같은 디스코드 계정이 다른 PC를 페어링하면 데몬이 이 PC의 relay 에이전트를 멈추고
 * lastErrorCode "token_replaced"를 준다. GUI는 이유를 알려 준다.
 */
import { render, screen } from '@testing-library/react';
import { beforeEach, describe, expect, it, vi } from 'vitest';
import { ConnectionView } from '../components/discord/ConnectionView';
import { useDiscordStore } from '../stores/useDiscordStore';

const viewProps = {
    cloudState: 'connected',
    hostId: 'host-1',
    relayUrl: 'https://relay',
    nodeCount: 1,
    botStatus: 'stopped',
    pairing: { active: false },
    onStartPairing: () => {},
    onCancelPairing: () => {},
    onDisconnect: () => {},
    onStartAgent: () => {},
    onStopAgent: () => {},
};

describe('다른 PC로 옮겨진 연결', () => {
    beforeEach(() => {
        useDiscordStore.getState()._resetForTest();
    });

    it('브리지가 replaced를 주면 꺼짐으로 두고 이유를 기억한다', async () => {
        window.api = {
            discordBotStatus: vi.fn().mockResolvedValue('replaced'),
            relayCheckHostStatus: vi.fn().mockResolvedValue({ online: true }),
        };
        useDiscordStore.setState({ discordCloudHostId: 'host-1' });
        await useDiscordStore.getState().checkStatus();
        expect(useDiscordStore.getState().discordBotStatus).toBe('stopped');
        expect(useDiscordStore.getState().discordAgentReplaced).toBe(true);

        window.api.discordBotStatus.mockResolvedValue('running');
        await useDiscordStore.getState().checkStatus();
        expect(useDiscordStore.getState().discordAgentReplaced).toBe(false);
    });

    it('연결 화면이 다시 페어링하라고 알려 준다', () => {
        const { rerender } = render(<ConnectionView {...viewProps} />);
        expect(screen.queryByText(/moved to another PC/)).toBeNull();
        rerender(<ConnectionView {...viewProps} agentReplaced />);
        expect(screen.getByText(/moved to another PC/)).toBeInTheDocument();
    });
});
