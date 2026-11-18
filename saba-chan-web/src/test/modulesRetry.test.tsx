import { act, renderHook } from '@testing-library/react';
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import { useLiveData } from '../App';
import { useData } from '../store';
import { mockApi } from './mocks';

// 휴대폰 망에서 모듈 목록 첫 요청이 시간 초과되면 게임 이름·설정 칸·명령 목록이 비고, 새로고침 말고는 방법이 없었다
describe('웹 모듈 목록', () => {
    beforeEach(() => {
        vi.useFakeTimers({ toFake: ['setTimeout', 'clearTimeout'] });
        useData.setState({ modules: [], modulesLoaded: false });
    });
    afterEach(() => vi.useRealTimers());

    it('첫 읽기에 실패하면 서버 목록을 읽을 때 다시 받는다', async () => {
        const moduleList = vi
            .fn()
            .mockResolvedValueOnce({ error: 'timeout', error_code: 'ETIMEDOUT' })
            .mockResolvedValue({ modules: [{ name: 'game' }] });
        const api = mockApi({
            serverList: vi.fn().mockResolvedValue({ servers: [] }),
            moduleList,
            moduleGetLocales: vi.fn().mockResolvedValue({}),
            onDaemonEvent: () => () => {},
        });
        const { unmount } = renderHook(() => useLiveData(api, true));
        await act(async () => vi.advanceTimersByTimeAsync(4000));
        expect(useData.getState().modules).toEqual([{ name: 'game' }]);

        // 받은 뒤에는 다시 묻지 않는다
        await act(async () => vi.advanceTimersByTimeAsync(8000));
        expect(moduleList).toHaveBeenCalledTimes(2);
        unmount();
    });

    it('다시 읽기 신호(resync)가 오면 모듈 목록도 다시 받는다', async () => {
        let emit: (e: { type: string; data: object }) => void = () => {};
        const moduleList = vi.fn().mockResolvedValue({ modules: [{ name: 'game' }] });
        const api = mockApi({
            serverList: vi.fn().mockResolvedValue({ servers: [] }),
            moduleList,
            moduleGetLocales: vi.fn().mockResolvedValue({}),
            onDaemonEvent: (cb: typeof emit) => {
                emit = cb;
                return () => {};
            },
        });
        const { unmount } = renderHook(() => useLiveData(api, true));
        await act(async () => {});
        expect(moduleList).toHaveBeenCalledTimes(1);
        await act(async () => emit({ type: 'resync', data: {} }));
        expect(moduleList).toHaveBeenCalledTimes(2);
        unmount();
    });
});
