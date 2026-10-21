/**
 * G-C1: 줄 key가 데몬 id라 디스크 꼬리와 새 프로세스 줄이 같은 key를 가졌다.
 * G-C3: 크래시·외부 정지 뒤 이미 본 줄이 담긴 디스크 꼬리를 통째로 다시 붙였다.
 */
import { act, renderHook } from '@testing-library/react';
import { afterEach, describe, expect, it, vi } from 'vitest';
import { useMultiConsole } from '../hooks/useMultiConsole';
import { useConsoleStore } from '../stores/useConsoleStore';
import { useServerStore } from '../stores/useServerStore';

const open = () => {
    const hook = renderHook(() =>
        useMultiConsole({ isPopoutMode: false, popoutParams: null, consoleBufferRef: { current: 2000 } }),
    );
    act(() => hook.result.current.openConsole('a', 'A'));
    return hook;
};
const lines = () => useConsoleStore.getState().consoles.a.lines;
const disk = (contents) => ({ source: 'disk', lines: contents.map((content, id) => ({ id, content })) });

describe('콘솔 디스크 꼬리', () => {
    afterEach(() => {
        useConsoleStore.setState({ consoles: {} });
        vi.useRealTimers();
    });

    it('크래시 뒤에는 이미 본 줄을 빼고 못 본 마지막 줄만 붙인다', async () => {
        vi.useFakeTimers();
        useServerStore.setState({ servers: [{ id: 'a', name: 'A', pid: 10 }] });
        const live = ['boot', 'ready', 'tick'].map((content, id) => ({ id, content }));
        window.api.managedConsole = vi.fn(async (_id, since) => ({
            epoch: 1,
            lines: live.filter((l) => l.id >= since),
        }));
        const { result } = open();
        await act(async () => vi.advanceTimersByTimeAsync(600));
        expect(lines().map((l) => l.content)).toEqual(['boot', 'ready', 'tick']);

        // 프로세스가 죽어 데몬은 디스크 로그 꼬리를 준다 (0부터 다시 매긴 id)
        useServerStore.setState({ servers: [{ id: 'a', name: 'A', pid: 0 }] });
        window.api.managedConsole = vi.fn(async () => disk(['boot', 'ready', 'tick', 'panic!']));
        await act(async () => vi.advanceTimersByTimeAsync(11_000));
        expect(lines().map((l) => l.content)).toEqual(['boot', 'ready', 'tick', 'panic!']);
        result.current.closeConsole('a');
    });

    it('같은 줄이 되풀이돼도 못 본 줄을 버리지 않는다 (시각까지 맞춰 본다)', async () => {
        vi.useFakeTimers();
        useServerStore.setState({ servers: [{ id: 'a', name: 'A', pid: 10 }] });
        const at = (content, timestamp, id) => ({ id, content, timestamp, source: 'Stdout' });
        const live = [at('ready', 100, 0), at('tick', 101, 1)];
        window.api.managedConsole = vi.fn(async (_id, since) => ({
            epoch: 1,
            lines: live.filter((l) => l.id >= since),
        }));
        const { result } = open();
        await act(async () => vi.advanceTimersByTimeAsync(600));

        // 마지막 폴링 뒤에 같은 두 줄이 한 번 더 찍히고 죽었다. 디스크는 앞 공백을 지운다
        useServerStore.setState({ servers: [{ id: 'a', name: 'A', pid: 0 }] });
        const tail = [at('ready', 100), at('tick', 101), at('ready', 102), at('tick', 103), at('panic', 103)];
        window.api.managedConsole = vi.fn(async () => ({ source: 'disk', lines: tail.map((l, id) => ({ ...l, id })) }));
        await act(async () => vi.advanceTimersByTimeAsync(11_000));
        expect(lines().map((l) => l.content)).toEqual(['ready', 'tick', 'ready', 'tick', 'panic']);
        result.current.closeConsole('a');
    });

    it('디스크 꼬리 뒤에 새 프로세스 줄이 와도 줄 key가 겹치지 않는다', async () => {
        vi.useFakeTimers();
        useServerStore.setState({ servers: [{ id: 'a', name: 'A', pid: 0 }] });
        window.api.managedConsole = vi.fn(async () => disk(['old 0', 'old 1']));
        const { result } = open();
        await act(async () => vi.advanceTimersByTimeAsync(5_100));
        expect(lines()).toHaveLength(2);

        useServerStore.setState({ servers: [{ id: 'a', name: 'A', pid: 20 }] });
        const fresh = ['new 0', 'new 1'].map((content, id) => ({ id, content }));
        window.api.managedConsole = vi.fn(async (_id, since) => ({
            epoch: 1,
            lines: fresh.filter((l) => l.id >= since),
        }));
        await act(async () => vi.advanceTimersByTimeAsync(6_000));
        expect(lines().map((l) => l.content)).toEqual(['old 0', 'old 1', 'new 0', 'new 1']);
        const keys = lines().map((l) => l.key);
        expect(keys.every((k) => k != null)).toBe(true);
        expect(new Set(keys).size).toBe(keys.length);
        result.current.closeConsole('a');
    });
});
