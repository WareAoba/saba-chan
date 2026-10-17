import { act, fireEvent, render, screen } from '@testing-library/react';
import { afterEach, describe, expect, it, vi } from 'vitest';
import { UnisolatedSection } from '../components/Modals/UnisolatedSection';

const flush = () => act(async () => {});

describe('격리 없이 돌리기 동의 목록', () => {
    afterEach(() => {
        delete window.api.sandboxListUnisolated;
        delete window.api.sandboxRevokeUnisolated;
        window.showToast = undefined;
    });

    it('동의한 구성요소와 파일이 바뀐 동의를 보이고, 지우면 다시 읽는다', async () => {
        window.showToast = vi.fn();
        window.api.sandboxListUnisolated = vi
            .fn()
            .mockResolvedValueOnce({
                components: [
                    { component: 'module-palworld', fingerprint: 'a', current: true },
                    { component: 'ext-music', fingerprint: 'b', current: false },
                ],
            })
            .mockResolvedValue({ components: [{ component: 'ext-music', fingerprint: 'b', current: false }] });
        window.api.sandboxRevokeUnisolated = vi.fn().mockResolvedValue({ success: true, fingerprint: null });
        render(<UnisolatedSection />);
        await flush();
        expect(screen.getByText('module-palworld')).toBeInTheDocument();
        expect(screen.getByText(/Files changed since you agreed/)).toBeInTheDocument();

        fireEvent.click(screen.getByRole('button', { name: 'Remove consent for module-palworld' }));
        await flush();
        expect(window.api.sandboxRevokeUnisolated).toHaveBeenCalledWith('module-palworld');
        expect(window.showToast).toHaveBeenCalledWith(
            'Removed the consent to run module-palworld without isolation.',
            'info',
            4000,
            undefined,
        );
        expect(screen.queryByText('module-palworld')).toBeNull();
    });

    it('동의가 없으면 없다고 쓴다', async () => {
        window.api.sandboxListUnisolated = vi.fn().mockResolvedValue({ components: [] });
        window.api.sandboxRevokeUnisolated = vi.fn();
        render(<UnisolatedSection />);
        await flush();
        expect(screen.getByText('No components run without isolation.')).toBeInTheDocument();
    });
});
