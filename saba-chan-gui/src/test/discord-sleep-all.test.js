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

    it('깨우기가 일부 서버에서 실패하면 깨우기를 다시 누를 수 있다', async () => {
        sleeping = { g1: true, g2: true };
        window.api.discordSettingsSet = vi.fn(async (g, patch) => {
            if (g === 'g2') return { error: 'relay down' };
            sleeping[g] = patch.sleeping;
            return { settings: { sleeping: sleeping[g] } };
        });
        await act(async () => render(<ConnectionView {...viewProps} />));
        await act(async () => fireEvent.click(await screen.findByRole('button', { name: /Wake up/ })));
        expect(sleeping).toEqual({ g1: false, g2: true });
        expect(window.showToast).toHaveBeenCalled();
        await screen.findByRole('button', { name: /Wake up/ });
    });

    it('길드 목록이 바뀐 뒤 늦게 온 옛 목록의 응답은 버린다', async () => {
        let releaseOld;
        window.api.discordSettingsGet = vi.fn((g) =>
            g === 'old'
                ? new Promise((r) => (releaseOld = () => r({ sleeping: true })))
                : Promise.resolve({ sleeping: false }),
        );
        let view;
        await act(async () => {
            view = render(<ConnectionView {...viewProps} nodes={[{ guildId: 'old' }]} />);
        });
        await act(async () => view.rerender(<ConnectionView {...viewProps} nodes={[{ guildId: 'g1' }]} />));
        await screen.findByRole('button', { name: /Put to sleep/ });
        await act(async () => releaseOld());
        expect(screen.queryByRole('button', { name: /Wake up/ })).toBeNull();
        expect(screen.getByRole('button', { name: /Put to sleep/ })).toBeTruthy();
    });
});
