import { afterEach, describe, expect, it, vi } from 'vitest';
import { everyWhileVisible } from '../utils/visibleTimer';

const setHidden = (hidden: boolean) => {
    Object.defineProperty(document, 'hidden', { configurable: true, get: () => hidden });
    document.dispatchEvent(new Event('visibilitychange'));
};

describe('창이 보일 때만 도는 주기 확인', () => {
    afterEach(() => {
        setHidden(false);
        vi.useRealTimers();
    });

    it('창이 숨으면 묻지 않고, 다시 보이면 바로 한 번 묻는다', () => {
        vi.useFakeTimers();
        const fn = vi.fn();
        const stop = everyWhileVisible(fn, 1000);
        vi.advanceTimersByTime(3000);
        expect(fn).toHaveBeenCalledTimes(3);

        setHidden(true);
        vi.advanceTimersByTime(10_000);
        expect(fn).toHaveBeenCalledTimes(3);

        setHidden(false);
        expect(fn).toHaveBeenCalledTimes(4);
        stop();
        vi.advanceTimersByTime(5000);
        setHidden(false);
        expect(fn).toHaveBeenCalledTimes(4);
    });
});
