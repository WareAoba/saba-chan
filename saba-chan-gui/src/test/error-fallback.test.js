import { render, screen } from '@testing-library/react';
import { describe, expect, it, vi } from 'vitest';
import { ErrorFallback } from '../components/ErrorFallback';

describe('크래시 화면', () => {
    it('창 닫기를 받으면 화면만 닫는다 (데몬은 유지)', () => {
        let onClose = null;
        window.api.onCloseRequest = vi.fn((cb) => {
            onClose = cb;
        });
        window.api.closeResponse = vi.fn();
        render(<ErrorFallback error={new Error('boom')} resetErrorBoundary={vi.fn()} />);

        expect(screen.getByText('Something went wrong')).toBeTruthy();
        expect(screen.getByTitle(/close/i)).toBeTruthy();
        onClose();
        expect(window.api.closeResponse).toHaveBeenCalledWith('exit-interface');
    });
});
