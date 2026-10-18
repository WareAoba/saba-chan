import { act, render, screen } from '@testing-library/react';
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

    it('갱신한 토스트는 이전 타이머가 아니라 새 타이머에 맞춰 사라진다', () => {
        vi.useFakeTimers();
        render(<Toast />);
        let id;
        act(() => {
            id = window.showToast('설치 중', 'info', 1000);
        });
        act(() => window.updateToast(id, '설치 완료', 'success', 5000));
        act(() => vi.advanceTimersByTime(2000));
        expect(screen.getByText('설치 완료')).toBeTruthy();
        act(() => vi.advanceTimersByTime(3300));
        expect(screen.queryByText('설치 완료')).toBeNull();
    });
});
