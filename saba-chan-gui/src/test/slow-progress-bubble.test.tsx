import { act, renderHook } from '@testing-library/react';
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import { useSlowProgressBubble } from '../hooks/useSlowProgressBubble';
import { useSabaBubble } from '../stores/useSabaBubble';
import type { ProgressBarConfig } from '../stores/useUIStore';

// 요리하는 사바쨩은 받기·설치 진행이 멈춰 보일 때만 뜬다.
// 서버 시작·정지처럼 끝을 모르는 진행은 서버 상태가 따로 보여 주니 띄우지 않는다 (사용자 11-12)
describe('느린 진행 말풍선', () => {
    beforeEach(() => {
        vi.useFakeTimers();
        useSabaBubble.setState({ bubble: null });
    });
    afterEach(() => {
        vi.useRealTimers();
    });

    const run = (bar: ProgressBarConfig | null) =>
        renderHook(({ b }) => useSlowProgressBubble(b, 'wait'), { initialProps: { b: bar } });

    it('받기 진행이 5초 넘게 그대로면 띄운다', () => {
        run({ message: 'Downloading', percent: 20 });
        act(() => vi.advanceTimersByTime(6000));
        expect(useSabaBubble.getState().bubble?.kind).toBe('cooking');
    });

    it('서버 시작·정지처럼 끝을 모르는 진행은 오래 걸려도 띄우지 않는다', () => {
        run({ message: 'Starting', indeterminate: true });
        act(() => vi.advanceTimersByTime(20000));
        expect(useSabaBubble.getState().bubble).toBeNull();
    });
});
