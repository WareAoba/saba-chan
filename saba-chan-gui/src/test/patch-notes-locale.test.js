import { act, render, screen } from '@testing-library/react';
import { afterEach, beforeAll, describe, expect, it, vi } from 'vitest';
import NoticeModal from '../components/Modals/NoticeModal';
import { announceLastUpdateResult, PatchNotesModal } from '../components/Modals/PatchNotesModal';
import i18n from '../i18n';

// L2: 업데이트 결과 안내가 사용자 언어가 아니라 영어로 보였다
describe('업데이트 결과 안내 언어', () => {
    beforeAll(async () => {
        await i18n.loadLanguages('ko');
    });
    afterEach(async () => {
        delete window.api.updaterLastResult;
        window.__sabaNotice.clearAllNotices();
        await act(async () => i18n.changeLanguage('en'));
    });

    it('언어 파일이 오기 전에 알림을 남겼어도 보일 때는 지금 언어로 보인다', async () => {
        window.api.updaterLastResult = vi.fn().mockResolvedValue({
            ok: true,
            version: '0.1.7',
            result: { success: true, updated_components: ['gui'], failed: [] },
        });
        // 첫 언어(캐시)가 영어인 채로 데몬이 먼저 떠서 알림을 만든다
        await announceLastUpdateResult(i18n.getFixedT('en', 'gui'));
        await act(async () => i18n.changeLanguage('ko'));
        render(<NoticeModal isOpen onClose={vi.fn()} />);
        expect(
            screen.getByText(i18n.getFixedT('ko', 'gui')('patch_notes.notice_success', { version: '0.1.7' })),
        ).toBeTruthy();
    });

    it('실패한 업데이트는 업데이터의 진단 문장 대신 번역 문구를 보인다', async () => {
        await act(async () => i18n.changeLanguage('ko'));
        render(
            <PatchNotesModal
                data={{
                    version: '0.1.7',
                    notes: null,
                    result: { success: false, message: 'Failed to apply update: gui locked', failed: ['gui'] },
                }}
                onClose={vi.fn()}
            />,
        );
        expect(screen.getAllByText(new RegExp(i18n.getFixedT('ko', 'gui')('patch_notes.failed'))).length).toBe(1);
        expect(screen.queryByText(/Failed to apply update/)).toBeNull();
    });
});
