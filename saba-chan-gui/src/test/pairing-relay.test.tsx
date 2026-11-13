import { act, render, renderHook, screen } from '@testing-library/react';
import { afterEach, describe, expect, it, vi } from 'vitest';
import { PairingCard } from '../components/discord/ConnectionView';
import { pairingTokenSaved, usePairing } from '../components/discord/usePairing';

describe('짝짓기는 저장된 릴레이 주소로만 (데몬 59a93b8)', () => {
    afterEach(() => {
        delete (window.api as Partial<typeof window.api>).relayInitiatePairing;
    });

    it('주소를 먼저 저장하고 나서 짝짓기 코드를 받는다', async () => {
        const order: string[] = [];
        window.api.relayInitiatePairing = vi.fn(async () => {
            order.push('initiate');
            return { error: 'stop here' };
        });
        const beforeStart = vi.fn(async () => {
            order.push('save');
            return true;
        });
        const { result } = renderHook(() => usePairing({ relayUrl: 'https://my.relay', beforeStart }));
        await act(async () => result.current.start());
        expect(order).toEqual(['save', 'initiate']);
    });

    it('저장에 실패하면 짝짓기를 시작하지 않는다', async () => {
        window.api.relayInitiatePairing = vi.fn();
        const { result } = renderHook(() =>
            usePairing({ relayUrl: 'https://my.relay', beforeStart: async () => false }),
        );
        await act(async () => result.current.start());
        expect(window.api.relayInitiatePairing).not.toHaveBeenCalled();
        expect(result.current.status).toBe('error');
    });
});

describe('짝지은 뒤 노드 토큰 저장', () => {
    afterEach(() => {
        vi.useRealTimers();
        delete (window.api as Partial<typeof window.api>).relayInitiatePairing;
        delete (window.api as Partial<typeof window.api>).relayPollPairingStatus;
    });

    it('짝짓기 마무리가 실패하면 성공 대신 저장하지 못했다고 보여 다시 짝짓게 한다', async () => {
        vi.useFakeTimers();
        window.api.relayInitiatePairing = vi
            .fn()
            .mockResolvedValue({ code: 'C', pollSecret: 'p', expiresAt: '2999-01-01T00:00:00Z' });
        window.api.relayPollPairingStatus = vi.fn().mockResolvedValue({ status: 'claimed', nodeToken: 'tok' });
        const onClaimed = vi.fn().mockResolvedValue(false);
        const { result } = renderHook(() => usePairing({ relayUrl: 'https://my.relay', onClaimed }));
        await act(async () => result.current.start());
        await act(async () => vi.advanceTimersByTimeAsync(3000));
        expect(onClaimed).toHaveBeenCalled();
        expect(result.current.status).toBe('unsaved');
    });
});

describe('만료 시각과 화면 이탈 (릴레이 2차 점검 R16·R17)', () => {
    afterEach(() => {
        vi.useRealTimers();
        delete (window.api as Partial<typeof window.api>).relayInitiatePairing;
        delete (window.api as Partial<typeof window.api>).relayPollPairingStatus;
    });

    it('만료 직전에 연결됐으면 만료 시각에 한 번 더 확인해 연결됨으로 끝낸다', async () => {
        vi.useFakeTimers();
        vi.setSystemTime(new Date('2030-01-01T00:00:00Z'));
        window.api.relayInitiatePairing = vi
            .fn()
            .mockResolvedValue({ code: 'C', pollSecret: 'p', expiresAt: '2030-01-01T00:00:02Z' });
        // 폴링(3초)보다 만료 시각(2초)이 먼저 온다: 만료 시각 뒤 확인에서 claimed
        window.api.relayPollPairingStatus = vi.fn().mockResolvedValue({ status: 'claimed', nodeToken: 'tok' });
        const onClaimed = vi.fn().mockResolvedValue(true);
        const { result } = renderHook(() => usePairing({ relayUrl: 'https://my.relay', onClaimed }));
        await act(async () => result.current.start());
        await act(async () => vi.advanceTimersByTimeAsync(2000));
        expect(onClaimed).toHaveBeenCalledTimes(1);
        expect(result.current.status).toBe('success');
    });

    it('만료 뒤 확인에서도 연결되지 않았으면 만료로 보인다', async () => {
        vi.useFakeTimers();
        vi.setSystemTime(new Date('2030-01-01T00:00:00Z'));
        window.api.relayInitiatePairing = vi
            .fn()
            .mockResolvedValue({ code: 'C', pollSecret: 'p', expiresAt: '2030-01-01T00:00:02Z' });
        window.api.relayPollPairingStatus = vi.fn().mockResolvedValue({ status: 'pending' });
        const { result } = renderHook(() => usePairing({ relayUrl: 'https://my.relay' }));
        await act(async () => result.current.start());
        await act(async () => vi.advanceTimersByTimeAsync(2000));
        expect(window.api.relayPollPairingStatus).toHaveBeenCalled();
        expect(result.current.status).toBe('expired');
    });

    it('코드 발급을 기다리는 사이 화면을 떠나면 폴링을 시작하지 않는다', async () => {
        vi.useFakeTimers();
        let resolveInit: ((v: unknown) => void) | undefined;
        window.api.relayInitiatePairing = vi.fn(
            () =>
                new Promise((r) => {
                    resolveInit = r;
                }),
        ) as never;
        window.api.relayPollPairingStatus = vi.fn().mockResolvedValue({ status: 'pending' });
        const { result, unmount } = renderHook(() => usePairing({ relayUrl: 'https://my.relay' }));
        let started: unknown;
        act(() => {
            started = result.current.start();
        });
        unmount();
        resolveInit!({ code: 'C', pollSecret: 'p', expiresAt: '2999-01-01T00:00:00Z' });
        await started;
        await vi.advanceTimersByTimeAsync(10000);
        expect(window.api.relayPollPairingStatus).not.toHaveBeenCalled();
    });
});

describe('데몬이 노드 토큰을 저장했는지 (릴레이 2차 점검 R15)', () => {
    it('데몬이 알려 준 tokenSaved를 따른다', () => {
        expect(pairingTokenSaved({ status: 'claimed', tokenSaved: false, nodeToken: null })).toBe(false);
        expect(pairingTokenSaved({ status: 'claimed', tokenSaved: true, nodeToken: 'tok' })).toBe(true);
    });

    it('저장하지 못했으면 연결됨 대신 다시 연결하라고 안내한다', () => {
        render(
            <PairingCard pairing={{ status: 'unsaved', code: 'C', start: () => {} } as never} onCancel={() => {}} />,
        );
        expect(screen.getByText(/saving the connection on this PC failed/)).toBeInTheDocument();
        expect(screen.queryByText(/Connected/)).not.toBeInTheDocument();
    });
});
