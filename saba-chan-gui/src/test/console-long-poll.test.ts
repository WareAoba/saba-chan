import { act, renderHook } from '@testing-library/react';
import { afterEach, describe, expect, it, vi } from 'vitest';
import { useMultiConsole } from '../hooks/useMultiConsole';
import { useConsoleStore } from '../stores/useConsoleStore';
import { useServerStore } from '../stores/useServerStore';
import type { Server } from '../stores/useServerStore';

const contents = () => useConsoleStore.getState().consoles.a.lines.map((l: { content: string }) => l.content);

describe('콘솔: 새 줄이 올 때까지 기다리는 데몬', () => {
    afterEach(() => {
        useConsoleStore.setState({ consoles: {} });
        vi.useRealTimers();
    });

    it('빈 응답을 0.5초마다 다시 묻지 않고, 데몬이 붙잡은 요청이 줄을 가져온다', async () => {
        vi.useFakeTimers();
        useServerStore.setState({ servers: [{ id: 'a', name: 'A', pid: 10 }] as Server[] });
        let release: (v: unknown) => void = () => {};
        const managedConsole = vi.fn(async (_id: string, since: number, _count?: number, _opts?: object): Promise<unknown> =>
            since === 0
                ? { epoch: 1, waits: undefined, lines: [{ id: 0, content: 'hello' }] }
                : new Promise((resolve) => {
                      release = resolve;
                  }),
        );
        window.api.managedConsole = managedConsole as never;
        const { result } = renderHook(() =>
            useMultiConsole({ isPopoutMode: false, popoutParams: null, consoleBufferRef: { current: 2000 } }),
        );
        act(() => result.current.openConsole('a', 'A'));
        await act(async () => vi.advanceTimersByTimeAsync(10_000));
        expect(contents()).toEqual(['hello']);
        // 두 번째 요청은 기다려 달라고 하고(세대를 같이 보낸다) 답이 올 때까지 다음 요청을 보내지 않는다
        expect(managedConsole).toHaveBeenCalledTimes(2);
        expect(managedConsole.mock.calls[1][3]).toMatchObject({ wait: expect.any(Number), epoch: 1 });

        await act(async () => {
            release({ epoch: 1, waits: true, lines: [{ id: 1, content: 'world' }] });
            await vi.advanceTimersByTimeAsync(600);
        });
        expect(contents()).toEqual(['hello', 'world']);
        expect(managedConsole).toHaveBeenCalledTimes(3);
        result.current.closeConsole('a');
    });

    it('꺼진 서버의 지난 기록은 한 번 받은 뒤 다시 달라고 하지 않는다', async () => {
        vi.useFakeTimers();
        useServerStore.setState({ servers: [{ id: 'a', name: 'A', pid: null }] as unknown as Server[] });
        const managedConsole = vi.fn(async (_id?: string, _since?: number, _count?: number, _opts?: object) => ({
            source: 'disk',
            waits: true,
            lines: [{ id: 0, content: 'past' }],
        }));
        window.api.managedConsole = managedConsole as never;
        const { result } = renderHook(() =>
            useMultiConsole({ isPopoutMode: false, popoutParams: null, consoleBufferRef: { current: 2000 } }),
        );
        act(() => result.current.openConsole('a', 'A'));
        await act(async () => vi.advanceTimersByTimeAsync(1200));
        expect(contents()).toEqual(['past']);
        expect(managedConsole.mock.calls.at(-1)?.[3]).toMatchObject({ diskSeen: true });
        result.current.closeConsole('a');
    });
});
