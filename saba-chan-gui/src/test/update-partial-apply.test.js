/**
 * 업데이트 적용이 일부만 되면(데몬 200 `ok:false`, `errors`) 성공 알림(updates:completed)을 보내지 않는다 (G-U9).
 * 실패 이유는 적용을 부른 화면(SabaStorage)이 오류로 보인다
 */
import { describe, expect, it, vi } from 'vitest';
import { createApi } from '../api/createApi';

async function apply(data) {
    const hub = { dispatch: vi.fn() };
    const { api } = createApi({ invoke: vi.fn().mockResolvedValue({ status: 200, data }), hub });
    const res = await api.updaterApply(['module-a', 'module-b']);
    return { res, completed: hub.dispatch.mock.calls.filter(([name]) => name === 'updates:completed') };
}

describe('업데이트 적용 결과 알림 (G-U9)', () => {
    it('일부가 실패하면 성공 알림을 보내지 않고 결과를 그대로 돌려준다', async () => {
        const data = {
            ok: false,
            applied: ['module-a'],
            errors: ['module-b: boom'],
            needs_updater: [],
            requires_updater: false,
        };
        const { res, completed } = await apply(data);
        expect(completed).toHaveLength(0);
        expect(res).toEqual(data);
    });

    it('모두 적용되면 성공 알림을 보낸다', async () => {
        const { completed } = await apply({
            ok: true,
            applied: ['module-a'],
            errors: [],
            needs_updater: [],
            requires_updater: false,
        });
        expect(completed).toHaveLength(1);
        expect(completed[0][1].names).toEqual(['module-a']);
    });
});
