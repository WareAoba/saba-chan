import { render, screen, waitFor } from '@testing-library/react';
import { afterEach, describe, expect, it, vi } from 'vitest';
import { PatchNotesModal } from '../components/Modals/PatchNotesModal';

// 업데이트 결과 알림은 데몬이 넣고 누르면 열린다 (notice-groups)
describe('패치노트 (A5)', () => {
    afterEach(() => {
        delete (window.api as Partial<typeof window.api>).updaterLastResult;
    });

    it('패치노트가 없으면 대체 문구, 실패 항목은 목록으로, 열면 확인 처리', async () => {
        window.api.updaterLastResult = vi.fn().mockResolvedValue({ success: true });
        render(
            <PatchNotesModal
                data={{ version: '0.1.3', notes: null, result: { success: true, failed: ['gui: locked'] } }}
                onClose={vi.fn()}
            />,
        );
        expect(screen.getByText('Saba-chan 0.1.3 patch notes')).toBeTruthy();
        expect(screen.getByText('No patch notes for this version')).toBeTruthy();
        expect(screen.getByText('gui: locked')).toBeTruthy();
        await waitFor(() => expect(window.api.updaterLastResult).toHaveBeenCalledWith({ clear: true }));
    });
});
