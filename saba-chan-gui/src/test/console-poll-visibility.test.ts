/**
 * 한 번 연 콘솔은 화면에 없어도 0.5초마다 읽었고, 창이 가려져도, 서버를 지워도 계속 읽었다.
 */
import { act, renderHook } from '@testing-library/react';
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import { useMultiConsole } from '../hooks/useMultiConsole';
import { useConsoleStore } from '../stores/useConsoleStore';
import { useServerStore } from '../stores/useServerStore';
import type { Server } from '../stores/useServerStore';

const setHidden = (hidden: boolean) => {
    Object.defineProperty(document, 'hidden', { configurable: true, get: () => hidden });
    document.dispatchEvent(new Event('visibilitychange'));
};

describe('콘솔 읽기 주기', () => {
    let read: ReturnType<typeof vi.fn>;
    const open = async () => {
        const hook = renderHook(() =>
            useMultiConsole({ isPopoutMode: false, popoutParams: null, consoleBufferRef: { current: 2000 } }),
        );
        act(() => hook.result.current.openConsole('a', 'A'));
        await act(async () => vi.advanceTimersByTimeAsync(10));
        read.mockClear();
        return hook;
    };

    beforeEach(() => {
        vi.useFakeTimers();
        useServerStore.setState({ servers: [{ id: 'a', name: 'A', pid: 10 }] as Server[] });
        read = vi.fn(async () => ({ epoch: 1, lines: [] }));
        window.api.managedConsole = read as never;
    });
    afterEach(() => {
        setHidden(false);
        useConsoleStore.setState({ consoles: {} });
        vi.useRealTimers();
    });

    it('화면에 보이는 콘솔만 빠르게 읽고, 안 보이는 콘솔은 천천히 읽는다', async () => {
        const { result } = await open();
        await act(async () => vi.advanceTimersByTimeAsync(5000));
        expect(read.mock.calls.length).toBeLessThanOrEqual(1);

        // 콘솔이 화면에 뜨면 기다리지 않고 바로 읽는다
        read.mockClear();
        let unwatch = () => {};
        act(() => {
            unwatch = result.current.watchConsole('a');
        });
        await act(async () => vi.advanceTimersByTimeAsync(10));
        expect(read).toHaveBeenCalledTimes(1);
        await act(async () => vi.advanceTimersByTimeAsync(5000));
        expect(read.mock.calls.length).toBeGreaterThanOrEqual(9);

        read.mockClear();
        act(() => unwatch());
        await act(async () => vi.advanceTimersByTimeAsync(5000));
        expect(read.mock.calls.length).toBeLessThanOrEqual(2);
        act(() => result.current.closeConsole('a'));
    });

    it('창이 가려져 있는 동안은 읽지 않고, 다시 보이면 바로 읽는다', async () => {
        const { result } = await open();
        act(() => void result.current.watchConsole('a'));
        await act(async () => vi.advanceTimersByTimeAsync(10));
        act(() => setHidden(true));
        await act(async () => vi.advanceTimersByTimeAsync(600));
        read.mockClear();
        await act(async () => vi.advanceTimersByTimeAsync(30_000));
        expect(read).not.toHaveBeenCalled();

        act(() => setHidden(false));
        await act(async () => vi.advanceTimersByTimeAsync(10));
        expect(read).toHaveBeenCalledTimes(1);
        act(() => result.current.closeConsole('a'));
    });

    it('목록에서 사라진 서버의 콘솔은 닫고 더 읽지 않는다', async () => {
        await open();
        useServerStore.setState({ servers: [] });
        await act(async () => vi.advanceTimersByTimeAsync(6000));
        read.mockClear();
        await act(async () => vi.advanceTimersByTimeAsync(30_000));
        expect(read).not.toHaveBeenCalled();
        expect(useConsoleStore.getState().consoles.a).toBeUndefined();
    });
});
