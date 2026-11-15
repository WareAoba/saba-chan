import { act, render, screen } from '@testing-library/react';
import { afterEach, beforeAll, describe, expect, it, vi } from 'vitest';
import { PatchNotesModal } from '../components/Modals/PatchNotesModal';
import i18n from '../i18n';

// L2: 업데이트 결과 안내가 사용자 언어가 아니라 영어로 보였다 (알림함 문장은 데몬이 화면 언어로 만든다: notice-groups)
describe('업데이트 결과 안내 언어', () => {
    beforeAll(async () => {
        await i18n.loadLanguages('ko');
    });
    afterEach(async () => {
        delete (window.api as Partial<typeof window.api>).updaterLastResult;
        await act(async () => i18n.changeLanguage('en'));
    });

    it('실패한 업데이트는 업데이터의 진단 문장 대신 번역 문구를 보인다', async () => {
        window.api.updaterLastResult = vi.fn().mockResolvedValue({ success: true });
        await act(async () => i18n.changeLanguage('ko'));
        render(
            <PatchNotesModal
                data={{
                    version: '0.1.7',
                    notes: null,
                    result: { success: false, message: 'Failed to apply update: gui locked', failed: ['gui'] } as never,
                }}
                onClose={vi.fn()}
            />,
        );
        expect(screen.getAllByText(new RegExp(i18n.getFixedT('ko', 'gui')('patch_notes.failed'))).length).toBe(1);
        expect(screen.queryByText(/Failed to apply update/)).toBeNull();
    });
});
