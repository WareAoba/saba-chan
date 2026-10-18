import { act, renderHook } from '@testing-library/react';
import { afterEach, describe, expect, it, vi } from 'vitest';
import { persistNodeToken, usePairing } from '../components/discord/usePairing';

describe('짝짓기는 저장된 릴레이 주소로만 (데몬 59a93b8)', () => {
    afterEach(() => {
        delete window.api.relayInitiatePairing;
    });

    it('주소를 먼저 저장하고 나서 짝짓기 코드를 받는다', async () => {
        const order = [];
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
        delete window.api.relayInitiatePairing;
        delete window.api.relayPollPairingStatus;
        delete window.api.saveNodeToken;
    });

    it('한 번 실패하면 한 번 더 저장하고, 두 번 다 실패하면 false', async () => {
        window.api.saveNodeToken = vi.fn().mockResolvedValueOnce(false).mockResolvedValueOnce(true);
        expect(await persistNodeToken('tok')).toBe(true);
        window.api.saveNodeToken = vi.fn().mockResolvedValueOnce(false).mockRejectedValueOnce(new Error('down'));
        expect(await persistNodeToken('tok')).toBe(false);
        expect(window.api.saveNodeToken).toHaveBeenCalledTimes(2);
    });

    it('짝짓기 마무리가 실패하면 성공 대신 오류로 보여 다시 짝짓게 한다', async () => {
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
        expect(result.current.status).toBe('error');
    });
});
