import { act, render, screen } from '@testing-library/react';
import { afterEach, describe, expect, it, vi } from 'vitest';
import Toast from '../components/Toast/Toast';
import { useSettingsStore } from '../stores/useSettingsStore';

describe('Toast 타이머', () => {
    afterEach(() => {
        vi.useRealTimers();
        useSettingsStore.getState()._resetForTest();
    });

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

    it('알림 표시 시간 설정을 따른다 (길게는 두 배, 짧게는 0.6배)', () => {
        vi.useFakeTimers();
        render(<Toast />);
        useSettingsStore.setState({ toastDuration: 'long' });
        act(() => window.showToast('길게', 'info', 3000));
        act(() => vi.advanceTimersByTime(3500));
        expect(screen.getByText('길게')).toBeTruthy();
        act(() => vi.advanceTimersByTime(2800));
        expect(screen.queryByText('길게')).toBeNull();

        useSettingsStore.setState({ toastDuration: 'short' });
        act(() => window.showToast('짧게', 'info', 3000));
        act(() => vi.advanceTimersByTime(2200));
        expect(screen.queryByText('짧게')).toBeNull();
    });
});
