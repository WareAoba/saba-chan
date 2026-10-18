import { afterEach, describe, expect, it, vi } from 'vitest';
import { openWithNotice } from '../utils/helpers';

describe('폴더 열기: 실패를 사용자에게 알린다', () => {
    afterEach(() => {
        window.showToast = undefined;
    });

    it('셸이 {error}로 답하면 그 문구를 오류 토스트로 보인다 (폴더가 없을 때)', async () => {
        window.showToast = vi.fn();
        await openWithNotice(() => Promise.resolve({ error: 'The folder does not exist.' }));
        expect(window.showToast).toHaveBeenCalledWith('The folder does not exist.', 'error', 4000, undefined);
    });

    it('호출이 던지면 그 메시지를, 성공하면 아무것도 알리지 않는다', async () => {
        window.showToast = vi.fn();
        await openWithNotice(() => Promise.reject(new Error('ipc down')));
        await openWithNotice(() => Promise.resolve({ success: true }));
        expect(window.showToast.mock.calls).toEqual([['ipc down', 'error', 4000, undefined]]);
    });
});
