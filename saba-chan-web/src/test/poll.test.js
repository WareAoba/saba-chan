import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import { startPoll } from '../components/poll';

describe('주기 읽기', () => {
    beforeEach(() => vi.useFakeTimers({ toFake: ['setTimeout', 'clearTimeout'] }));
    afterEach(() => {
        vi.useRealTimers();
        Object.defineProperty(document, 'hidden', { value: false, configurable: true });
    });

    it('앞 요청이 끝나기 전에는 다음 요청을 보내지 않는다', async () => {
        let resolve;
        const fn = vi.fn(() => new Promise((r) => (resolve = r)));
        const stop = startPoll(fn, 1000);
        expect(fn).toHaveBeenCalledTimes(1);
        vi.advanceTimersByTime(5000);
        expect(fn).toHaveBeenCalledTimes(1);
        resolve();
        await Promise.resolve();
        await Promise.resolve();
        vi.advanceTimersByTime(1000);
        expect(fn).toHaveBeenCalledTimes(2);
        stop();
    });

    it('탭이 숨으면 쉬고, 다시 보이면 바로 읽는다', async () => {
        const fn = vi.fn(async () => {});
        const stop = startPoll(fn, 1000);
        await Promise.resolve();
        await Promise.resolve();
        Object.defineProperty(document, 'hidden', { value: true, configurable: true });
        vi.advanceTimersByTime(5000);
        expect(fn).toHaveBeenCalledTimes(1);
        Object.defineProperty(document, 'hidden', { value: false, configurable: true });
        document.dispatchEvent(new Event('visibilitychange'));
        expect(fn).toHaveBeenCalledTimes(2);
        stop();
    });

    it('멈춘 뒤에는 읽지 않는다', async () => {
        const fn = vi.fn(async () => {});
        const stop = startPoll(fn, 1000);
        stop();
        vi.advanceTimersByTime(5000);
        document.dispatchEvent(new Event('visibilitychange'));
        expect(fn).toHaveBeenCalledTimes(1);
    });
});
