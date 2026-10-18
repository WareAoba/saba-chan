import { act, render, screen } from '@testing-library/react';
import { afterEach, describe, expect, it } from 'vitest';
import { TaskIndicator } from '../components/shell/TaskIndicator';
import { useUIStore } from '../stores/useUIStore';

describe('TaskIndicator', () => {
    afterEach(() => act(() => useUIStore.getState().clearProgressBar()));

    it('0%로 시작한 다운로드는 회전하지 않고 0%를 보인다', () => {
        render(<TaskIndicator />);
        act(() => useUIStore.getState().setProgressBar({ message: '다운로드 중', percent: 0 }));
        expect(screen.getByText('0%')).toBeTruthy();
        expect(document.querySelector('.preflight-fill.indeterminate')).toBeNull();
    });

    it('indeterminate 작업은 회전하고 백분율을 숨긴다', () => {
        render(<TaskIndicator />);
        act(() => useUIStore.getState().setProgressBar({ message: '시작 중', indeterminate: true }));
        expect(document.querySelector('.preflight-fill.indeterminate')).not.toBeNull();
        expect(screen.queryByText(/%/)).toBeNull();
    });
});
