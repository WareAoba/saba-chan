import { act, renderHook } from '@testing-library/react';
import { afterEach, describe, expect, it, vi } from 'vitest';
import { usePairing } from '../components/discord/usePairing';

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
