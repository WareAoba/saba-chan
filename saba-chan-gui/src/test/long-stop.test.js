import { act, renderHook } from '@testing-library/react';
import { afterEach, describe, expect, it, vi } from 'vitest';
import { useServerActions } from '../hooks/useServerActions';

// 데몬은 정지 요청에 stop_timeout(최대 3600초)까지 기다린 뒤 답한다(MK14, #458). 그동안 목록은 running으로 온다.
// 요청이 끝나기 전에는 카드가 '정지 중'을 지키고, 그사이 꺼져도 GUI가 한 정지라 '예기치 않게 종료'로 알리지 않는다
describe('오래 걸리는 정지', () => {
    afterEach(() => vi.useRealTimers());

    it('요청이 60초를 넘겨도 정지 중을 지키고, 꺼지면 예기치 않은 종료로 알리지 않는다', async () => {
        window.showToast = vi.fn();
        vi.useFakeTimers({ toFake: ['setTimeout', 'clearTimeout', 'Date'] });
        let now = 1_000_000_000_000;
        vi.setSystemTime(now);
        const tick = (ms) => {
            now += ms;
            vi.advanceTimersByTime(ms);
        };
        let list = [{ id: 'a', name: 'A', module: 'game', status: 'running', pid: 10, last_api_action: 0 }];
        let release;
        window.api = {
            serverList: vi.fn(async () => ({ servers: list })),
            instanceStop: vi.fn(() => new Promise((r) => (release = r))),
            instanceStatus: vi.fn(async () => ({ status: 'stopped' })),
        };
        let servers = list;
        const setServers = (u) => {
            servers = typeof u === 'function' ? u(servers) : u;
        };
        const setModal = vi.fn();
        const { result } = renderHook(() =>
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
                formatUptime: vi.fn(),
            }),
        );
        await act(async () => result.current.fetchServers());
        tick(40_000); // 데몬이 막 켜진 뒤의 유예(30초)를 넘긴다
        await act(async () => result.current.handleStop('A'));
        act(() => {
            setModal.mock.calls.at(-1)[0].onConfirm();
        });
        tick(61_000);
        await act(async () => result.current.fetchServers());
        expect(servers[0].status).toBe('stopping');

        list = [{ ...list[0], last_api_action: now - 61_000 }];
        tick(11 * 60_000);
        list = [{ ...list[0], status: 'stopped', pid: null }];
        await act(async () => result.current.fetchServers());
        expect(servers[0].status).toBe('stopped');
        const unexpected = window.showToast.mock.calls.filter(([, level]) => level === 'error' || level === 'info');
        expect(unexpected).toEqual([]);
        await act(async () => release({ success: true }));
    });
});
