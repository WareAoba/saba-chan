import { render, screen } from '@testing-library/react';
import { describe, expect, it, vi } from 'vitest';
import { ErrorFallback } from '../components/ErrorFallback';

describe('크래시 화면', () => {
    it('창 닫기를 받으면 화면만 닫는다 (데몬은 유지)', () => {
        let onClose = null;
        window.api.onCloseRequest = vi.fn((cb) => {
            onClose = cb;
        }) as never;
        window.api.closeResponse = vi.fn() as never;
        render(<ErrorFallback error={new Error('boom')} resetErrorBoundary={vi.fn()} />);

        expect(screen.getByText('Display failed')).toBeTruthy();
        expect(screen.getByRole('button', { name: /close/i })).toBeTruthy();
        (onClose as unknown as () => void)();
        expect(window.api.closeResponse).toHaveBeenCalledWith('exit-interface');
    });
});
