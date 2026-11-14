/**
 * 얇은 클라이언트 점검 7번 (2026-11-12): 설치 진행률을 1.2초마다 묻지 않고 데몬의 `provision.progress` 이벤트로 받는다.
 * 이벤트가 없는 셸에서만 예전처럼 자주 묻는다.
 */
import { act, renderHook } from '@testing-library/react';
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import { useInstanceMaintenance, useMaintenanceStore } from '../hooks/useInstanceMaintenance';
import type { Server } from '../stores/useServerStore';

const server = { id: 's1', name: 'pal', module: 'palworld', status: 'stopped', provisioning: true } as Server;
let listeners: ((ev: unknown) => void)[] = [];
const emit = (data: object) => {
    for (const l of listeners) l({ type: 'provision.progress', data: { instance: 'pal', ...data } });
};

beforeEach(() => {
    vi.useFakeTimers();
    listeners = [];
    useMaintenanceStore.setState({ byId: {} });
    window.api = {
        instanceProvisionProgress: vi.fn().mockResolvedValue({ active: true, step: 1, done: false }),
        onDaemonEvent: vi.fn((cb: (ev: unknown) => void) => {
            listeners.push(cb);
            return () => {
                listeners = listeners.filter((l) => l !== cb);
            };
        }),
    } as unknown as typeof window.api;
});
afterEach(() => vi.useRealTimers());

describe('설치 진행률', () => {
    it('이벤트를 받는 동안에는 자주 묻지 않고 이벤트로 진행률과 완성을 보인다', async () => {
        const { unmount } = renderHook(() => useInstanceMaintenance(server));
        await act(async () => vi.advanceTimersByTimeAsync(5000));
        expect(window.api.instanceProvisionProgress).toHaveBeenCalledTimes(1);
        expect(useMaintenanceStore.getState().byId.s1?.provisionProgress).toMatchObject({ step: 1 });

        act(() => emit({ step: 2, done: false }));
        expect(useMaintenanceStore.getState().byId.s1?.provisionProgress).toMatchObject({ step: 2, active: true });
        act(() => emit({ instance: 'other', step: 9 }));
        expect(useMaintenanceStore.getState().byId.s1?.provisionProgress).toMatchObject({ step: 2 });

        act(() => emit({ step: 3, done: true }));
        expect(useMaintenanceStore.getState().byId.s1).toMatchObject({ cooked: true, provisionProgress: { done: true } });
        unmount();
    });

    it('이벤트가 없는 셸에서는 예전처럼 자주 묻는다', async () => {
        (window.api as Partial<typeof window.api>).onDaemonEvent = undefined;
        const { unmount } = renderHook(() => useInstanceMaintenance(server));
        await act(async () => vi.advanceTimersByTimeAsync(5000));
        expect((window.api.instanceProvisionProgress as ReturnType<typeof vi.fn>).mock.calls.length).toBeGreaterThanOrEqual(4);
        unmount();
    });
});
