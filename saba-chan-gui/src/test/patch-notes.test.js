import { fireEvent, render, screen, waitFor } from '@testing-library/react';
import { afterEach, describe, expect, it, vi } from 'vitest';
import NoticeModal, { PATCH_NOTES_ACTION } from '../components/Modals/NoticeModal';
import { announceLastUpdateResult, PatchNotesModal } from '../components/Modals/PatchNotesModal';

const t = (k, o) => `${k}${o ? JSON.stringify(o) : ''}`;

describe('업데이트 결과 알림 (A5)', () => {
    afterEach(() => {
        delete window.api.updaterLastResult;
        window.__sabaNotice.clearAllNotices();
    });

    it('결과가 있으면 알림 하나를 남기고, 다시 물어도 쌓이지 않는다', async () => {
        window.api.updaterLastResult = vi.fn().mockResolvedValue({
            ok: true,
            version: '0.1.3',
            notes: '- 새 기능',
            result: { success: true, updated_components: ['gui'], failed: [] },
        });
        await announceLastUpdateResult(t);
        await announceLastUpdateResult(t);
        const list = window.__sabaNotice.loadNotices();
        expect(list).toHaveLength(1);
        expect(list[0]).toMatchObject({ action: PATCH_NOTES_ACTION, type: 'success' });
        expect(list[0].data).toMatchObject({ version: '0.1.3', notes: '- 새 기능' });
        expect(list[0].message).toContain('0.1.3');
    });

    it('결과가 없으면 아무것도 남기지 않는다', async () => {
        window.api.updaterLastResult = vi.fn().mockResolvedValue({ ok: true, result: null });
        await announceLastUpdateResult(t);
        expect(window.__sabaNotice.loadNotices()).toHaveLength(0);
    });

    it('알림을 누르면 패치노트가 열리고, 알림을 지우면 확인 처리한다', async () => {
        window.api.updaterLastResult = vi.fn().mockResolvedValue({ ok: true });
        const data = { version: '0.1.3', notes: null, result: { success: true, failed: [] } };
        window.__sabaNotice.addNotice({ message: 'updated', action: PATCH_NOTES_ACTION, source: 'Updater', data });
        const onOpen = vi.fn();
        render(<NoticeModal isOpen onClose={vi.fn()} onOpenPatchNotes={onOpen} />);
        fireEvent.click(screen.getByText('updated'));
        expect(onOpen).toHaveBeenCalledWith(data);
        window.__sabaNotice.clearAllNotices();
        expect(window.api.updaterLastResult).toHaveBeenCalledWith({ clear: true });
    });

    it('패치노트가 없으면 대체 문구, 실패 항목은 목록으로, 열면 확인 처리', async () => {
        window.api.updaterLastResult = vi.fn().mockResolvedValue({ ok: true });
        render(
            <PatchNotesModal
                data={{ version: '0.1.3', notes: null, result: { success: true, failed: ['gui: locked'] } }}
                onClose={vi.fn()}
            />,
        );
        expect(screen.getByText('Saba-chan 0.1.3 patch notes')).toBeTruthy();
        expect(screen.getByText('There are no patch notes for this version.')).toBeTruthy();
        expect(screen.getByText('gui: locked')).toBeTruthy();
        await waitFor(() => expect(window.api.updaterLastResult).toHaveBeenCalledWith({ clear: true }));
    });
});
