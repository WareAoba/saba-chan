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
});
