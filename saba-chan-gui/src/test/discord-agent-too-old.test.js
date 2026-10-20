/**
 * 릴레이가 이 PC의 앱 버전을 받지 않으면 데몬이 lastErrorCode "agent_too_old"와 minVersion을 준다 (데몬 #468).
 * GUI는 연결 화면에 올릴 버전을 알리고 업데이트 화면으로 이어 준다.
 */
import { fireEvent, render, screen } from '@testing-library/react';
import { beforeEach, describe, expect, it, vi } from 'vitest';
import { createApi } from '../api/createApi';
import { ConnectionView } from '../components/discord/ConnectionView';
import { useDiscordStore } from '../stores/useDiscordStore';
import { useUIStore } from '../stores/useUIStore';

const viewProps = {
    cloudState: 'connected',
    hostId: 'host-1',
    relayUrl: 'https://relay',
    nodeCount: 1,
    botStatus: 'connecting',
    pairing: { active: false },
    onStartPairing: () => {},
    onCancelPairing: () => {},
    onDisconnect: () => {},
    onStartAgent: () => {},
    onStopAgent: () => {},
};

describe('릴레이가 받지 않는 앱 버전', () => {
    beforeEach(() => {
        useDiscordStore.getState()._resetForTest();
    });

    it('브리지는 세션이 살아 있어도 too_old와 최소 버전을 준다', async () => {
        const invoke = vi.fn().mockResolvedValue({
            status: 200,
            data: { running: true, lastErrorCode: 'agent_too_old', minVersion: '0.1.9' },
        });
        const { api } = createApi({ invoke, hub: { dispatch: vi.fn() } });
        expect(await api.discordBotStatus()).toEqual({ status: 'too_old', running: true, minVersion: '0.1.9' });
    });

    it('스토어가 최소 버전을 기억하고, 다시 붙으면 지운다', async () => {
        window.api = {
            discordBotStatus: vi.fn().mockResolvedValue({ status: 'too_old', running: true, minVersion: '0.1.9' }),
            relayCheckHostStatus: vi.fn().mockResolvedValue({ online: true }),
        };
        useDiscordStore.setState({ discordCloudHostId: 'host-1' });
        await useDiscordStore.getState().checkStatus();
        expect(useDiscordStore.getState().discordAgentMinVersion).toBe('0.1.9');
        expect(useDiscordStore.getState().discordBotStatus).not.toBe('stopped');

        window.api.discordBotStatus.mockResolvedValue('running');
        await useDiscordStore.getState().checkStatus();
        expect(useDiscordStore.getState().discordAgentMinVersion).toBeNull();
    });

    it('연결 화면이 올릴 버전을 알리고 업데이트 화면을 연다', () => {
        const { rerender } = render(<ConnectionView {...viewProps} />);
        expect(screen.queryByText(/0\.1\.9/)).toBeNull();
        rerender(<ConnectionView {...viewProps} agentMinVersion="0.1.9" />);
        expect(screen.getByText(/0\.1\.9/)).toBeInTheDocument();
        fireEvent.click(screen.getByRole('button', { name: /update/i }));
        expect(useUIStore.getState().settingsInitialView).toBe('update');
    });
});
