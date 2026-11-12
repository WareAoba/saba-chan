import { act, renderHook } from '@testing-library/react';
import { beforeAll, beforeEach, describe, expect, it, vi } from 'vitest';
import { useServerControl } from '../components/useServerControl';
import { initI18n } from '../i18n';
import { useApp } from '../store';
import { asServer, mockApi } from './mocks';

const lastToast = () => useApp.getState().toasts.at(-1);

// 콘솔이 없어 저장하고 정지하라고 요청할 길이 없는 서버: 데몬은 죽이지 않고 graceful_stop_unavailable 로 답한다
describe('웹에서 저장 없이 정지', () => {
    const refused = { success: false, error_code: 'graceful_stop_unavailable', error: 'no console' };
    const server = asServer({ id: 'a', name: 'Terraria' });
    beforeAll(() => initI18n('en'));
    beforeEach(() => useApp.setState({ toasts: [], confirm: null }));

    /** 떠 있는 확인 창에 답하고, 다음 창이나 끝을 기다린다 */
    const answer = async (ok: boolean) => {
        await act(async () => {
            useApp.getState().confirm?.done(ok);
            await new Promise((r) => setTimeout(r));
        });
    };

    it('정지가 거절되면 앱과 같은 문구로 묻고, 확인하면 강제로 정지한다', async () => {
        const instanceStop = vi.fn(async (_id: string, opts: { force?: boolean }) => (opts.force ? { success: true } : refused));
        const { result } = renderHook(() => useServerControl(mockApi({ instanceStop })));
        act(() => void result.current.stop(server));
        await answer(true);
        expect(useApp.getState().confirm).toMatchObject({
            title: 'Stop without saving?',
            message: expect.stringContaining('Terraria'),
            confirmLabel: 'Stop without saving',
        });
        expect(useApp.getState().toasts).toEqual([]);
        await answer(true);
        expect(instanceStop.mock.calls).toEqual([
            ['a', {}],
            ['a', { force: true }],
        ]);
        expect(lastToast()).toMatchObject({ tone: 'ok' });
    });

    it('취소하면 서버를 그대로 둔다', async () => {
        const instanceStop = vi.fn().mockResolvedValue(refused);
        const { result } = renderHook(() => useServerControl(mockApi({ instanceStop })));
        act(() => void result.current.stop(server));
        await answer(true);
        await answer(false);
        expect(instanceStop).toHaveBeenCalledTimes(1);
        expect(useApp.getState().toasts).toEqual([]);
    });

    it('재시작이 거절되어도 같이 묻고, 확인하면 강제로 정지한 뒤 다시 시작한다', async () => {
        const instanceRestart = vi.fn(async (_id: string, opts: { force?: boolean }) => (opts.force ? { success: true } : refused));
        const { result } = renderHook(() => useServerControl(mockApi({ instanceRestart })));
        act(() => void result.current.restart(server));
        await answer(true);
        expect(useApp.getState().confirm).toMatchObject({ title: 'Stop without saving?' });
        await answer(true);
        expect(instanceRestart).toHaveBeenLastCalledWith('a', { force: true });
        expect(lastToast()).toMatchObject({ tone: 'ok' });
    });
});
