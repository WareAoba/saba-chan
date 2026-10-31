/**
 * 업데이트를 적용해 더 받을 것이 없어지면 알림함의 "업데이트 N개 발견" 알림을 지운다.
 * 알림은 localStorage에 남으므로, 상태가 "없음"으로 바뀐 것을 알리지 않으면 사용자가 지울 때까지 계속 남는다
 */
import { beforeEach, describe, expect, it, vi } from 'vitest';
import { createApi } from '../api/createApi';
import '../components/Modals/NoticeModal';
import { announceUpdateCompleted, syncUpdateNotice } from '../utils/updateNotice';

const t = (key, opts) => `${key}:${opts?.count ?? ''}`;
const updateNotices = () => window.__sabaNotice.loadNotices().filter((n) => n.action === 'openUpdateModal');

describe('업데이트 알림 정리', () => {
    beforeEach(() => {
        localStorage.clear();
    });

    it('업데이트가 없어지면 남아 있던 업데이트 알림을 지우고 다른 알림은 둔다', () => {
        syncUpdateNotice({ count: 2, names: ['Daemon', 'GUI'] }, t);
        window.__sabaNotice.addNotice({ message: 'other', source: 'saba-chan' });
        expect(updateNotices()).toHaveLength(1);

        syncUpdateNotice({ count: 0, names: [] }, t);

        expect(updateNotices()).toHaveLength(0);
        expect(window.__sabaNotice.loadNotices().map((n) => n.message)).toEqual(['other']);
    });

    it('업데이트를 마치면 업데이트 알림을 지우고, 여러 번 마쳐도 완료 알림은 하나만 남는다', async () => {
        window.api.updaterLastResult = vi.fn().mockResolvedValue({
            ok: true,
            version: '0.3.0',
            result: { success: true, updated_components: ['saba-core'], failed: [] },
        });
        syncUpdateNotice({ count: 1, names: ['Saba-Core'] }, t);

        await announceUpdateCompleted({ message: 'done' }, t);
        await announceUpdateCompleted({ count: 1, names: ['Saba-Core'] }, t);
        await announceUpdateCompleted({ message: 'done' }, t);

        expect(updateNotices()).toHaveLength(0);
        expect(window.__sabaNotice.loadNotices()).toHaveLength(1);
        expect(window.showToast).toHaveBeenCalledTimes(3);
        expect(window.showToast.mock.calls.every(([, , , opts]) => !opts?.isNotice)).toBe(true);
        delete window.api.updaterLastResult;
    });

    it('적용이 실패했으면 성공으로 알리지 않고 업데이트 알림도 남긴다', async () => {
        window.api.updaterLastResult = vi.fn().mockResolvedValue({
            ok: true,
            version: '0.3.0',
            result: { success: false, updated_components: [], failed: ['saba-core'] },
        });
        window.showToast.mockClear();
        syncUpdateNotice({ count: 1, names: ['Saba-Core'] }, t);

        await announceUpdateCompleted({ message: 'done' }, t);

        expect(updateNotices()).toHaveLength(1);
        expect(window.showToast).toHaveBeenCalledTimes(1);
        expect(window.showToast.mock.calls[0][1]).toBe('error');
        delete window.api.updaterLastResult;
    });

    it('수동 확인 결과 받을 것이 없으면(Locales만 있어도) 없음을 알린다', async () => {
        for (const data of [
            { ok: true, update_names: [], components: [] },
            { ok: true, update_names: ['Locales'], components: [{ display_name: 'Locales' }] },
        ]) {
            const hub = { dispatch: vi.fn() };
            const { api } = createApi({ invoke: vi.fn().mockResolvedValue({ status: 200, data }), hub });
            await api.updaterCheck();
            const sent = hub.dispatch.mock.calls.filter(([name]) => name === 'updates:available');
            expect(sent).toHaveLength(1);
            expect(sent[0][1].count).toBe(0);
        }
    });

    it('확인이 실패하면 아무것도 알리지 않는다 (알림을 지우지 않는다)', async () => {
        const hub = { dispatch: vi.fn() };
        const { api } = createApi({
            invoke: vi.fn().mockResolvedValue({ status: 200, data: { ok: false, error: 'relay down' } }),
            hub,
        });
        await api.updaterCheck();
        expect(hub.dispatch.mock.calls.filter(([name]) => name === 'updates:available')).toHaveLength(0);
    });
});
