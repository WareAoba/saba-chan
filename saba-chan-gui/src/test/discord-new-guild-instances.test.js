/**
 * 페어링 코드로 새로 묶인 디스코드 서버에 이 PC의 서버를 바로 열지(auto) 나중에 고를지(manual) — 데몬 봇 설정 `newGuildInstances`.
 * 코드 없이 /사바쨩 연결로 묶인 서버는 이 설정과 상관없이 데몬이 늘 나중에 고르게 둔다.
 */
import { act, fireEvent, render, screen, waitFor } from '@testing-library/react';
import { beforeEach, describe, expect, it, vi } from 'vitest';
import { ConnectionView } from '../components/discord/ConnectionView';
import { useDiscordStore } from '../stores/useDiscordStore';

const viewProps = {
    cloudState: 'connected',
    hostId: 'host-1',
    relayUrl: 'https://relay',
    nodeCount: 1,
    botStatus: 'running',
    pairing: { active: false },
    onStartPairing: () => {},
    onCancelPairing: () => {},
    onDisconnect: () => {},
    onStartAgent: () => {},
    onStopAgent: () => {},
};

describe('새 디스코드 서버에 서버 열기', () => {
    beforeEach(() => {
        useDiscordStore.getState()._resetForTest();
        window.api = {
            ...window.api,
            botConfigLoad: vi.fn().mockResolvedValue({ newGuildInstances: 'manual' }),
            botConfigSave: vi.fn().mockResolvedValue({ success: true }),
        };
    });

    it('데몬 값을 읽어 보이고, 바꾸면 그 키만 저장한다', async () => {
        await act(() => useDiscordStore.getState().loadConfig());
        useDiscordStore.setState({ _settingsReady: true });
        render(<ConnectionView {...viewProps} />);
        const toggle = screen.getByRole('checkbox', { name: 'Open my servers in new Discord servers' });
        expect(toggle).not.toBeChecked();
        fireEvent.click(toggle);
        expect(useDiscordStore.getState().discordNewGuildInstances).toBe('auto');
        await waitFor(() => expect(window.api.botConfigSave).toHaveBeenCalledWith({ newGuildInstances: 'auto' }));
    });
});
