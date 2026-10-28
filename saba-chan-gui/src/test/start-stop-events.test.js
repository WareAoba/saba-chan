import { act, renderHook } from '@testing-library/react';
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import { useServerActions } from '../hooks/useServerActions';

// 이벤트 스트림이 있으면 시작·정지가 끝났는지를 0.5초마다 묻지 않고, 그 서버의 이벤트가 오면 묻는다
describe('시작·정지 완료 확인 (이벤트)', () => {
    let servers;
    let emit;
    let setModal;
    const setServers = vi.fn((update) => {
        servers = typeof update === 'function' ? update(servers) : update;
    });
    const hook = () =>
        renderHook(() =>
            useServerActions({
                servers,
                setServers,
                modules: [],
                loading: false,
                setLoading: vi.fn(),
                setModal,
                setProgressBar: vi.fn(),
                openConsole: vi.fn(),
                closeConsole: vi.fn(),
                setShowModuleManager: vi.fn(),
            }),
        );
    const tick = async (ms, times = 1) => {
        for (let i = 0; i < times; i++) await act(async () => vi.advanceTimersByTimeAsync(ms));
    };
    const toastTypes = () => window.showToast.mock.calls.map((c) => c[1]);

    beforeEach(() => {
        vi.useFakeTimers({ shouldAdvanceTime: true });
        const listeners = new Set();
        emit = (type, data) =>
            act(async () => {
                for (const cb of listeners) cb({ type, data });
            });
        servers = [{ id: 'a', name: 'A', module: 'game', status: 'stopped' }];
        window.api = {
            serverList: vi.fn().mockResolvedValue({ servers }),
            instanceStart: vi.fn().mockResolvedValue({ success: true }),
            instanceStop: vi.fn().mockResolvedValue({ success: true }),
            instanceStatus: vi.fn().mockResolvedValue({ status: 'starting' }),
            onDaemonEvent: (cb) => {
                listeners.add(cb);
                return () => listeners.delete(cb);
            },
        };
        window.showToast = vi.fn();
        setModal = vi.fn();
    });
    afterEach(() => vi.useRealTimers());

    it('켜짐 이벤트가 오면 그때 묻고 끝낸다 (기다리는 동안 0.5초마다 묻지 않는다)', async () => {
        const { result } = hook();
        await act(async () => result.current.handleStart('A', 'game'));
        await tick(500, 6);
        expect(window.api.instanceStatus.mock.calls.length).toBeLessThanOrEqual(1);
        window.api.instanceStatus.mockResolvedValue({ status: 'running' });
        await emit('instance.started', { instanceId: 'other' });
        expect(toastTypes()).not.toContain('success');
        await emit('instance.started', { instanceId: 'a' });
        expect(window.api.instanceStatus).toHaveBeenLastCalledWith('a', 'A');
        expect(toastTypes()).toContain('success');
    });

    it('켜다가 꺼짐 이벤트가 오면 바로 실패로 알린다', async () => {
        const { result } = hook();
        await act(async () => result.current.handleStart('A', 'game'));
        await tick(500);
        window.api.instanceStatus.mockResolvedValue({ status: 'stopped' });
        await emit('instance.exit', { instanceId: 'a', kind: 'start_failed' });
        expect(toastTypes().at(-1)).toBe('error');
    });

    it('이벤트가 없으면 30초 뒤 시간 초과로 끝난다', async () => {
        const { result } = hook();
        await act(async () => result.current.handleStart('A', 'game'));
        await tick(5000, 7);
        expect(toastTypes().at(-1)).toBe('warning');
        expect(window.api.instanceStatus.mock.calls.length).toBeLessThan(10);
    });

    it('정지는 첫 확인에서 꺼져 있으면 바로 끝난다', async () => {
        servers = [{ id: 'a', name: 'A', module: 'game', status: 'running' }];
        window.api.instanceStatus.mockResolvedValue({ status: 'stopped' });
        const { result } = hook();
        await act(async () => result.current.handleStop('A'));
        await act(async () => setModal.mock.calls[0][0].onConfirm());
        await tick(500, 2);
        expect(window.api.instanceStatus).toHaveBeenCalledTimes(1);
        expect(toastTypes()).toContain('success');
    });
});
