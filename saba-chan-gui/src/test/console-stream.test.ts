import { act, renderHook } from '@testing-library/react';
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import { useMultiConsole } from '../hooks/useMultiConsole';
import { useConsoleStore } from '../stores/useConsoleStore';
import { useServerStore } from '../stores/useServerStore';
import type { Server } from '../stores/useServerStore';

const contents = () => useConsoleStore.getState().consoles.a.lines.map((l: { content: string }) => l.content);

type Update = (body: unknown) => void;

const setup = (managedConsoleStream: unknown) => {
    window.api.managedConsole = vi.fn(async () => ({ epoch: 1, lines: [{ id: 0, content: 'hello' }] })) as never;
    window.api.managedConsoleStream = managedConsoleStream as never;
    return renderHook(() => useMultiConsole({ isPopoutMode: false, popoutParams: null, consoleBufferRef: { current: 2000 } }));
};

describe('콘솔: 화면에 보이는 콘솔은 스트림으로 받는다', () => {
    beforeEach(() => {
        vi.useFakeTimers();
        useServerStore.setState({ servers: [{ id: 'a', name: 'A', pid: 10 }] as Server[] });
    });
    afterEach(() => {
        useConsoleStore.setState({ consoles: {} });
        vi.useRealTimers();
    });

    it('받은 데부터 스트림을 열어 오는 줄을 붙이고, 프로세스에 다시 붙으면 그 버퍼로 바꾸고, 화면에서 사라지면 닫는다', async () => {
        let update: Update = () => {};
        const close = vi.fn();
        const stream = vi.fn(async (_id: string, _opts: object, onUpdate: Update) => {
            update = onUpdate;
            return close;
        });
        const { result } = setup(stream);
        act(() => result.current.openConsole('a', 'A'));
        await act(async () => vi.advanceTimersByTimeAsync(100));
        let unwatch: () => void = () => {};
        act(() => {
            unwatch = result.current.watchConsole('a');
        });
        await act(async () => vi.advanceTimersByTimeAsync(100));
        expect(stream).toHaveBeenCalledTimes(1);
        expect(stream.mock.calls[0][1]).toMatchObject({ since: 1, epoch: 1 });

        const polls = (window.api.managedConsole as ReturnType<typeof vi.fn>).mock.calls.length;
        act(() => update({ epoch: 1, lines: [{ id: 1, content: 'world' }] }));
        expect(contents()).toEqual(['hello', 'world']);
        act(() => update({ epoch: 2, reset: true, lines: [{ id: 0, content: 'again' }] }));
        expect(contents()).toEqual(['again']);
        // 스트림이 열려 있는 동안은 주기 조회를 하지 않는다
        await act(async () => vi.advanceTimersByTimeAsync(10_000));
        expect((window.api.managedConsole as ReturnType<typeof vi.fn>).mock.calls.length).toBe(polls);

        act(() => unwatch());
        expect(close).toHaveBeenCalledTimes(1);
        result.current.closeConsole('a');
    });

    it('콘솔을 닫으면 스트림도 닫는다', async () => {
        const close = vi.fn();
        const { result } = setup(vi.fn(async () => close));
        act(() => result.current.openConsole('a', 'A'));
        act(() => void result.current.watchConsole('a'));
        await act(async () => vi.advanceTimersByTimeAsync(100));
        act(() => result.current.closeConsole('a'));
        expect(close).toHaveBeenCalledTimes(1);
    });

    it('스트림을 열지 못하면(옛 릴레이·연결 실패) 기다리는 조회로 읽고, 한동안 다시 열어 보지 않는다', async () => {
        const stream = vi.fn(async () => null);
        const { result } = setup(stream);
        act(() => result.current.openConsole('a', 'A'));
        act(() => void result.current.watchConsole('a'));
        await act(async () => vi.advanceTimersByTimeAsync(5_000));
        expect(stream).toHaveBeenCalledTimes(1);
        const calls = (window.api.managedConsole as ReturnType<typeof vi.fn>).mock.calls;
        expect(calls.at(-1)?.[3]).toMatchObject({ wait: expect.any(Number) });
        result.current.closeConsole('a');
    });
});
