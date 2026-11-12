import { act, renderHook } from '@testing-library/react';
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import { useModulesRetry } from '../hooks/useModulesRetry';
import { useServerStore } from '../stores/useServerStore';

// 데몬은 켜져 있는데 모듈 목록 첫 읽기가 실패하면, 데몬 상태가 바뀌지 않는 한 다시 받지 않아
// 게임 이름·설정 칸·명령 목록이 빈 채로 남았다
describe('모듈 목록 다시 받기', () => {
    beforeEach(() => {
        vi.useFakeTimers();
        useServerStore.getState()._resetForTest();
        window.showToast = vi.fn();
    });
    afterEach(() => vi.useRealTimers());

    it('데몬이 켜져 있는 동안 받을 때까지 조용히 다시 받는다', async () => {
        const moduleList = vi.fn().mockResolvedValue({ error: 'timeout' });
        window.api = {
            daemonStatus: vi.fn().mockResolvedValue({ running: true }),
            moduleList,
            moduleGetMetadata: vi.fn().mockResolvedValue({}),
        } as unknown as typeof window.api;
        const { unmount } = renderHook(() => useModulesRetry('running'));
        await act(async () => vi.advanceTimersByTimeAsync(10_000));
        expect(useServerStore.getState().modulesLoaded).toBe(false);
        const toasts = vi.mocked(window.showToast as NonNullable<typeof window.showToast>).mock.calls.length;
        expect(toasts).toBeGreaterThan(0);
        const tries = moduleList.mock.calls.length;
        await act(async () => vi.advanceTimersByTimeAsync(20_000));
        expect(moduleList.mock.calls.length).toBeGreaterThan(tries);

        moduleList.mockResolvedValue({ modules: [{ name: 'game' }] });
        await act(async () => vi.advanceTimersByTimeAsync(20_000));
        expect(useServerStore.getState().modulesLoaded).toBe(true);
        // 다시 받는 동안의 실패는 또 알리지 않는다
        expect(window.showToast).toHaveBeenCalledTimes(toasts);

        const calls = moduleList.mock.calls.length;
        await act(async () => vi.advanceTimersByTimeAsync(60_000));
        expect(moduleList).toHaveBeenCalledTimes(calls);
        unmount();
    });
});
