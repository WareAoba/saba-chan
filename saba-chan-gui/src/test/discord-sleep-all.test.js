/**
 * 연결 화면의 재우기: 연결은 그대로 두고 모든 디스코드 서버의 잠자기(sleeping)를 켠다.
 * 모두 잠들어 있으면 깨우기로 바뀐다.
 */
import { act, fireEvent, render, screen, waitFor } from '@testing-library/react';
import { beforeEach, describe, expect, it, vi } from 'vitest';
import { ConnectionView } from '../components/discord/ConnectionView';

const viewProps = {
    cloudState: 'connected',
    hostId: 'host-1',
    relayUrl: 'https://relay',
    nodes: [{ guildId: 'g1' }, { guildId: 'g2' }],
    nodeCount: 2,
    botStatus: 'running',
    pairing: { active: false },
    onStartPairing: () => {},
    onCancelPairing: () => {},
    onReload: () => {},
    onDisconnect: () => {},
    onStartAgent: () => {},
};

describe('모든 서버 재우기', () => {
    let sleeping;
    beforeEach(() => {
        sleeping = { g1: false, g2: false };
        window.showToast = vi.fn();
        window.api = {
            discordBotStop: vi.fn(),
            discordSettingsGet: vi.fn(async (g) => ({ sleeping: sleeping[g] })),
            discordSettingsSet: vi.fn(async (g, patch) => {
                sleeping[g] = patch.sleeping;
                return { settings: { sleeping: sleeping[g] } };
            }),
        };
    });

    it('재우기는 에이전트를 끄지 않고 모든 서버를 잠재운 뒤 깨우기로 바뀐다', async () => {
        await act(async () => render(<ConnectionView {...viewProps} />));
        const sleep = await screen.findByRole('button', { name: /Put to sleep/ });
        await act(async () => fireEvent.click(sleep));
        expect(window.api.discordSettingsSet).toHaveBeenCalledWith('g1', { sleeping: true });
        expect(window.api.discordSettingsSet).toHaveBeenCalledWith('g2', { sleeping: true });
        expect(window.api.discordBotStop).not.toHaveBeenCalled();
        await screen.findByRole('button', { name: /Wake up/ });
    });

    it('모두 잠들어 있으면 깨우기를 보이고, 누르면 모두 깨운다', async () => {
        sleeping = { g1: true, g2: true };
        await act(async () => render(<ConnectionView {...viewProps} />));
        const wake = await screen.findByRole('button', { name: /Wake up/ });
        await act(async () => fireEvent.click(wake));
        await waitFor(() => expect(sleeping).toEqual({ g1: false, g2: false }));
    });
});
