import { act, renderHook } from '@testing-library/react';
import { beforeEach, describe, expect, it, vi } from 'vitest';
import { useServerActions } from '../hooks/useServerActions';

describe('재시작이 끝난 뒤 상태', () => {
    beforeEach(() => {
        window.showToast = vi.fn();
    });

    it('데몬이 다시 켰다고 하면 끄는 중으로 남지 않는다', async () => {
        let list = [{ id: 'a', name: 'A', module: 'game', status: 'running', pid: 10 }];
        window.api = {
            serverList: vi.fn(async () => ({ servers: list })),
            // 짧은 정지 구간은 폴링이 보지 못하고, 끝났을 때는 이미 새 프로세스가 돈다
            instanceRestart: vi.fn(async () => {
                list = [{ ...list[0], pid: 20 }];
                return { success: true };
            }),
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
        await act(async () => result.current.handleRestart('A'));
        await act(async () => setModal.mock.calls.at(-1)[0].onConfirm());
        await act(async () => result.current.fetchServers());
        expect(servers[0].status).toBe('running');
    });
});
