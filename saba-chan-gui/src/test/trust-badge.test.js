import { render, screen } from '@testing-library/react';
import { describe, expect, it } from 'vitest';
import { TrustBadge } from '../components/ui/TrustBadge';

describe('TrustBadge', () => {
    it('trust 값별로 라벨·설명을 붙이고, 값이 없으면 그리지 않는다', () => {
        const { container, rerender } = render(<TrustBadge />);
        expect(container.innerHTML).toBe('');
        rerender(<TrustBadge trust="verified" />);
        expect(screen.getByText('Verified').className).toContain('trust-verified');
        rerender(<TrustBadge trust="modified" />);
        const badge = screen.getByRole('img', { name: /Files changed after installing/ });
        expect(badge.className).toContain('trust-modified');
        rerender(<TrustBadge trust="unverified" />);
        expect(screen.getByText('Unverified').getAttribute('data-tooltip')).toMatch(/signed catalog/);
        rerender(<TrustBadge trust="bogus" />);
        expect(container.innerHTML).toBe('');
    });
});
