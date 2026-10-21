/**
 * 닫기 요청이 App이 구독하기 전에 오면(로드 중 Alt+F4) 버리지 않고 첫 구독자에게 넘긴다.
 * 그래야 받음 확인(ack)이 가고, 셸이 정상 렌더러를 멈춘 것으로 보지 않는다 (#477 리뷰).
 */
import { describe, expect, it, vi } from 'vitest';
import { createEventHub } from '../api/events';

describe('구독 전에 온 닫기 요청', () => {
    it('첫 구독자에게 한 번 넘긴다', async () => {
        let deliver;
        const hub = createEventHub();
        await hub.start(async (name, cb) => {
            if (name === 'app:closeRequest') deliver = cb;
        });
        deliver(null);
        const first = vi.fn();
        hub.on('app:closeRequest', first);
        await Promise.resolve();
        expect(first).toHaveBeenCalledTimes(1);

        const second = vi.fn();
        hub.on('app:closeRequest', second);
        await Promise.resolve();
        expect(second).not.toHaveBeenCalled();
    });
});
