/**
 * 업데이트를 마치면 결과를 토스트로 알린다. 알림함의 결과 알림은 데몬이 하나만 둔다 (notices.rs)
 */
import { describe, expect, it, vi } from 'vitest';
import { createApi } from '../api/createApi';
import { announceUpdateCompleted } from '../utils/updateNotice';

const t = ((key: string, opts?: Record<string, unknown>) => `${key}:${opts?.count ?? ''}`) as never;

describe('업데이트 완료 알림', () => {
    it('업데이트를 마치면 토스트로 알린다', async () => {
        window.api.updaterLastResult = vi.fn().mockResolvedValue({
            success: true,
            version: '0.3.0',
            result: { success: true, updated_components: ['saba-core'], failed: [] },
        });
        vi.mocked(window.showToast!).mockClear();

        await announceUpdateCompleted({ message: 'done' }, t);

        expect(window.showToast).toHaveBeenCalledWith('done', 'success', 5000);
        delete (window.api as unknown as Record<string, unknown>).updaterLastResult;
    });

    it('적용이 실패했으면 성공으로 알리지 않는다', async () => {
        window.api.updaterLastResult = vi.fn().mockResolvedValue({
            success: true,
            version: '0.3.0',
            result: { success: false, updated_components: [], failed: ['saba-core'] },
        });
        vi.mocked(window.showToast!).mockClear();

        await announceUpdateCompleted({ message: 'done' }, t);

        expect(window.showToast).toHaveBeenCalledTimes(1);
        expect(vi.mocked(window.showToast!).mock.calls[0][1]).toBe('error');
        delete (window.api as unknown as Record<string, unknown>).updaterLastResult;
    });

    it('수동 확인 결과 받을 것이 없으면(Locales만 있어도) 없음을 알린다', async () => {
        for (const data of [
            { success: true, update_names: [], components: [] },
            { success: true, update_names: ['Locales'], components: [{ display_name: 'Locales' }] },
        ]) {
            const hub = { dispatch: vi.fn() };
            const { api } = createApi({ invoke: vi.fn().mockResolvedValue({ status: 200, data }), hub: hub as never });
            await api.updaterCheck();
            const sent = hub.dispatch.mock.calls.filter(([name]) => name === 'updates:available');
            expect(sent).toHaveLength(1);
            expect(sent[0][1].count).toBe(0);
        }
    });

    it('확인이 실패하면 아무것도 알리지 않는다 (알림을 지우지 않는다)', async () => {
        const hub = { dispatch: vi.fn() };
        const { api } = createApi({
            invoke: vi.fn().mockResolvedValue({ status: 200, data: { success: false, error: 'relay down' } }),
            hub: hub as never,
        });
        await api.updaterCheck();
        expect(hub.dispatch.mock.calls.filter(([name]) => name === 'updates:available')).toHaveLength(0);
    });
});
