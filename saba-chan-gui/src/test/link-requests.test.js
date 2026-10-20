import { act, fireEvent, render, screen } from '@testing-library/react';
import { beforeEach, describe, expect, it, vi } from 'vitest';
import { PairingCard } from '../components/discord/ConnectionView';
import { LinkRequests, linkRequestsAvailable } from '../components/discord/LinkRequests';
import { useDiscordStore } from '../stores/useDiscordStore';

const later = () => new Date(Date.now() + 600_000).toISOString();

describe('디스코드에서 온 연결 요청 (J1-b)', () => {
    let emit;
    beforeEach(() => {
        window.showToast = vi.fn();
        window.api = {
            onDaemonEvent: vi.fn((cb) => {
                emit = cb;
                return () => {};
            }),
        };
    });

    it('채널이 없으면 그리지 않는다', () => {
        expect(linkRequestsAvailable()).toBe(false);
    });

    it('대기 중인 요청을 보이고, 수락하면 데몬에 accept를 보내고 지운다', async () => {
        window.api.relayLinkRequests = vi.fn().mockResolvedValue({
            requests: [{ requestId: 'r1', guildId: 'g', guildName: '김치', userName: 'aoba', expiresAt: later() }],
        });
        window.api.relayLinkRequestRespond = vi.fn().mockResolvedValue({ ok: true, guildId: 'g', guildName: '김치' });
        render(<LinkRequests />);
        await act(async () => {});
        expect(screen.getByText(/김치/)).toBeInTheDocument();
        await act(async () => fireEvent.click(screen.getByRole('button', { name: /Accept/i })));
        expect(window.api.relayLinkRequestRespond).toHaveBeenCalledWith('r1', true);
        expect(screen.queryByText(/김치/)).toBeNull();
    });

    it('relay.link_request 이벤트가 오면 요청을 더하고, 거절은 accept:false', async () => {
        window.api.relayLinkRequests = vi.fn().mockResolvedValue({ requests: [] });
        window.api.relayLinkRequestRespond = vi.fn().mockResolvedValue({ ok: true, declined: true });
        render(<LinkRequests />);
        await act(async () => {});
        act(() =>
            emit({
                type: 'relay.link_request',
                data: { requestId: 'r2', guildId: 'g2', guildName: 'Pals', expiresAt: later() },
            }),
        );
        expect(screen.getByText(/Pals/)).toBeInTheDocument();
        await act(async () => fireEvent.click(screen.getByRole('button', { name: /Decline/i })));
        expect(window.api.relayLinkRequestRespond).toHaveBeenCalledWith('r2', false);
        expect(screen.queryByText(/Pals/)).toBeNull();
    });

    // G-D4: 릴레이는 404·409로 답할 때도 요청을 이미 꺼냈다. 카드가 남으면 다시 눌러도 계속 실패한다
    it('릴레이가 404·409로 답하면 카드를 지우고, 데몬에 닿지 못했을 때만 남긴다', async () => {
        window.api.relayLinkRequests = vi.fn().mockResolvedValue({
            requests: [
                { requestId: 'r4', guildId: 'g4', guildName: 'Gone', expiresAt: later() },
                { requestId: 'r5', guildId: 'g5', guildName: 'Taken', expiresAt: later() },
                { requestId: 'r6', guildId: 'g6', guildName: 'Offline', expiresAt: later() },
            ],
        });
        window.api.relayLinkRequestRespond = vi.fn(async (id) =>
            id === 'r4'
                ? { ok: false, error: 'NOT_FOUND' }
                : id === 'r5'
                  ? { ok: false, error: 'GUILD_TAKEN' }
                  : { success: false, error: 'connect ECONNREFUSED' },
        );
        render(<LinkRequests />);
        await act(async () => {});
        for (const name of ['Gone', 'Taken', 'Offline']) {
            const card = screen.getByText(new RegExp(name)).closest('.link-request');
            await act(async () => fireEvent.click(card.querySelector('.btn-primary')));
        }
        expect(screen.queryByText(/Gone/)).toBeNull();
        expect(screen.queryByText(/Taken/)).toBeNull();
        expect(screen.getByText(/Offline/)).toBeInTheDocument();
        expect(window.showToast).toHaveBeenCalledTimes(3);
    });

    // G-D5: 수락으로 길드가 묶였으니 디스코드 화면의 길드 목록을 다시 읽게 한다
    it('수락에 성공하면 길드 목록을 다시 읽으라고 알린다', async () => {
        window.api.relayLinkRequests = vi.fn().mockResolvedValue({
            requests: [{ requestId: 'r7', guildId: 'g7', guildName: 'New', expiresAt: later() }],
        });
        window.api.relayLinkRequestRespond = vi.fn().mockResolvedValue({ ok: true, guildId: 'g7', guildName: 'New' });
        const before = useDiscordStore.getState().cloudNodesStale;
        render(<LinkRequests />);
        await act(async () => {});
        await act(async () => fireEvent.click(screen.getByRole('button', { name: /Accept/i })));
        expect(useDiscordStore.getState().cloudNodesStale).toBe(before + 1);
    });

    it('expiresAt이 초 단위 숫자여도 만료 전이면 보인다', async () => {
        window.api.relayLinkRequests = vi.fn().mockResolvedValue({
            requests: [
                { requestId: 'r3', guildId: 'g', guildName: 'Secs', expiresAt: Math.floor(Date.now() / 1000) + 600 },
            ],
        });
        window.api.relayLinkRequestRespond = vi.fn();
        render(<LinkRequests />);
        await act(async () => {});
        expect(screen.getByText(/Secs/)).toBeInTheDocument();
    });

    it('페어링 화면에 같은 일을 하는 CLI 한 줄을 보인다 (J1-c)', () => {
        render(<PairingCard pairing={{ status: 'waiting', code: 'ABC', remaining: 300 }} onCancel={vi.fn()} />);
        expect(screen.getByText('saba-chan-cli bot pair')).toBeInTheDocument();
    });
});
