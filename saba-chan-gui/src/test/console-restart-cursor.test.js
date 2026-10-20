import { act, renderHook } from '@testing-library/react';
import { afterEach, describe, expect, it, vi } from 'vitest';
import { useMultiConsole } from '../hooks/useMultiConsole';
import { useConsoleStore } from '../stores/useConsoleStore';
import { useServerStore } from '../stores/useServerStore';

describe('서버를 다시 켠 뒤 콘솔', () => {
    afterEach(() => {
        useConsoleStore.setState({ consoles: {} });
        vi.useRealTimers();
    });

    it('새 프로세스의 줄을 처음부터 받아 보여 준다', async () => {
        vi.useFakeTimers();
        useServerStore.setState({ servers: [{ id: 'a', name: 'A', pid: 10 }] });
        window.api.managedConsole = vi.fn(async (_id, since) =>
            since === 0 ? { lines: [{ id: 100, content: 'old' }] } : { lines: [] },
        );
        const { result } = renderHook(() =>
            useMultiConsole({ isPopoutMode: false, popoutParams: null, consoleBufferRef: { current: 2000 } }),
        );
        act(() => result.current.openConsole('a', 'A'));
        await act(async () => vi.advanceTimersByTimeAsync(600));
        expect(useConsoleStore.getState().consoles.a.lines.map((l) => l.content)).toEqual(['old']);

        // 재시작: 새 프로세스는 줄 번호를 0부터 다시 센다
        useServerStore.setState({ servers: [{ id: 'a', name: 'A', pid: 20 }] });
        window.api.managedConsole = vi.fn(async (_id, since) => ({
            lines: [
                { id: 0, content: 'new 0' },
                { id: 1, content: 'new 1' },
            ].filter((l) => l.id >= since),
        }));
        await act(async () => vi.advanceTimersByTimeAsync(1200));
        expect(useConsoleStore.getState().consoles.a.lines.map((l) => l.content)).toEqual(['old', 'new 0', 'new 1']);
        result.current.closeConsole('a');
    });

    it('데몬이 다시 붙어 버퍼 세대가 바뀌면 PID가 같아도 처음부터 다시 받아 그 버퍼로 바꾼다', async () => {
        vi.useFakeTimers();
        useServerStore.setState({ servers: [{ id: 'a', name: 'A', pid: 10 }] });
        const old = Array.from({ length: 3 }, (_, i) => ({ id: 500 + i, content: `old ${i}` }));
        window.api.managedConsole = vi.fn(async (_id, since) => ({
            epoch: 1,
            lines: old.filter((l) => l.id >= since),
        }));
        const { result } = renderHook(() =>
            useMultiConsole({ isPopoutMode: false, popoutParams: null, consoleBufferRef: { current: 2000 } }),
        );
        act(() => result.current.openConsole('a', 'A'));
        await act(async () => vi.advanceTimersByTimeAsync(600));
        expect(useConsoleStore.getState().consoles.a.lines).toHaveLength(3);

        // 다시 붙은 버퍼: 디스크 꼬리를 0부터 다시 매기고 새 줄이 이어진다
        const rebuilt = [...old.map((l, i) => ({ id: i, content: l.content })), { id: 3, content: 'after' }];
        window.api.managedConsole = vi.fn(async (_id, since) => ({
            epoch: 2,
            lines: rebuilt.filter((l) => l.id >= since),
        }));
        await act(async () => vi.advanceTimersByTimeAsync(2000));
        expect(useConsoleStore.getState().consoles.a.lines.map((l) => l.content)).toEqual([
            'old 0',
            'old 1',
            'old 2',
            'after',
        ]);
        result.current.closeConsole('a');
    });

    it('세대를 주는 데몬에서 서버를 다시 켜면 지난 실행 줄 뒤에 새 줄을 한 번씩만 붙인다', async () => {
        vi.useFakeTimers();
        useServerStore.setState({ servers: [{ id: 'a', name: 'A', pid: 10 }] });
        window.api.managedConsole = vi.fn(async (_id, since) => ({
            epoch: 1,
            lines: [{ id: 100, content: 'old' }].filter((l) => l.id >= since),
        }));
        const { result } = renderHook(() =>
            useMultiConsole({ isPopoutMode: false, popoutParams: null, consoleBufferRef: { current: 2000 } }),
        );
        act(() => result.current.openConsole('a', 'A'));
        await act(async () => vi.advanceTimersByTimeAsync(600));

        useServerStore.setState({ servers: [{ id: 'a', name: 'A', pid: 20 }] });
        window.api.managedConsole = vi.fn(async (_id, since) => ({
            epoch: 2,
            lines: [
                { id: 0, content: 'new 0' },
                { id: 1, content: 'new 1' },
            ].filter((l) => l.id >= since),
        }));
        await act(async () => vi.advanceTimersByTimeAsync(3000));
        expect(useConsoleStore.getState().consoles.a.lines.map((l) => l.content)).toEqual(['old', 'new 0', 'new 1']);
        result.current.closeConsole('a');
    });
});
