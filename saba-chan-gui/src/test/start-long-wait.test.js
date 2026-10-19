import { act, renderHook } from '@testing-library/react';
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import { createApi } from '../api/createApi';
import { useServerActions } from '../hooks/useServerActions';

// 첫 시작은 데몬이 준비하느라 몇 분이 걸릴 수 있다 (L35: 105초): 그동안 실패로 보이면 안 된다
describe('오래 걸리는 서버 시작', () => {
    it('instanceStart는 CLI처럼 30분까지 기다린다', async () => {
        const invoke = vi.fn().mockResolvedValue({ status: 200, data: { success: true } });
        const { api } = createApi({ invoke, hub: { dispatch: vi.fn() } });
        await api.instanceStart('a');
        expect(invoke.mock.calls.at(-1)[1]).toMatchObject({ path: '/api/instance/a/start', timeoutMs: 30 * 60_000 });
    });

    describe('시작 요청이 끝나기 전', () => {
        let servers;
        const setServers = vi.fn((update) => {
            servers = typeof update === 'function' ? update(servers) : update;
        });
        beforeEach(() => {
            vi.useFakeTimers({ shouldAdvanceTime: true });
            servers = [{ id: 'a', name: 'A', module: 'game', status: 'stopped' }];
            window.api = {
                serverList: vi
                    .fn()
                    .mockResolvedValue({ servers: [{ id: 'a', name: 'A', module: 'game', status: 'stopped' }] }),
                instanceStart: vi.fn(() => new Promise(() => {})),
            };
            window.showToast = vi.fn();
        });
        afterEach(() => vi.useRealTimers());

        it('1분이 넘어도 목록 새로고침이 시작 중 표시를 지우지 않는다', async () => {
            const { result } = renderHook(() =>
                useServerActions({
                    servers,
                    setServers,
                    modules: [],
                    loading: false,
                    setLoading: vi.fn(),
                    setModal: vi.fn(),
                    setProgressBar: vi.fn(),
                    openConsole: vi.fn(),
                    closeConsole: vi.fn(),
                    setShowModuleManager: vi.fn(),
                    formatUptime: vi.fn(),
                }),
            );
            await act(async () => result.current.fetchServers()); // 첫 목록
            act(() => {
                result.current.handleStart('A', 'game');
            });
            expect(servers[0].status).toBe('starting');
            await act(async () => vi.advanceTimersByTime(90_000));
            await act(async () => result.current.fetchServers());
            expect(servers[0].status).toBe('starting');
            expect(window.showToast).not.toHaveBeenCalled();
        });
    });
});
