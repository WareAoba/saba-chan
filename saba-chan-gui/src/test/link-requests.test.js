import { act, fireEvent, render, screen } from '@testing-library/react';
import { beforeEach, describe, expect, it, vi } from 'vitest';
import { PairingCard } from '../components/discord/ConnectionView';
import { LinkRequests, linkRequestsAvailable } from '../components/discord/LinkRequests';

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

    it('페어링 화면에 같은 일을 하는 CLI 한 줄을 보인다 (J1-c)', () => {
        render(<PairingCard pairing={{ status: 'waiting', code: 'ABC', remaining: 300 }} onCancel={vi.fn()} />);
        expect(screen.getByText('saba-chan-cli bot pair')).toBeInTheDocument();
    });
});
