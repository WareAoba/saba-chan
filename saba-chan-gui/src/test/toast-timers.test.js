import { act, render } from '@testing-library/react';
import { afterEach, describe, expect, it, vi } from 'vitest';
import Toast from '../components/Toast/Toast';

describe('Toast 타이머', () => {
    afterEach(() => vi.useRealTimers());

    it('언마운트하면 사라지기 타이머가 남지 않는다', () => {
        vi.useFakeTimers();
        const { unmount } = render(<Toast />);
        act(() => window.showToast('hello', 'info', 3000));
        expect(vi.getTimerCount()).toBeGreaterThan(0);
        unmount();
        expect(vi.getTimerCount()).toBe(0);
    });
});
