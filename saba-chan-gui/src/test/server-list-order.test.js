/**
 * G-S4: 서버 목록 요청 둘이 겹쳐 먼저 보낸 것이 늦게 오면, 옛 상태가 한 폴링 동안 보였다.
 */
import { act, renderHook } from '@testing-library/react';
import { describe, expect, it, vi } from 'vitest';
import { useServerActions } from '../hooks/useServerActions';

describe('서버 목록 응답 순서', () => {
    it('늦게 온 옛 응답은 새 응답을 덮지 않는다', async () => {
        let servers = [];
        const setServers = vi.fn((update) => {
            servers = typeof update === 'function' ? update(servers) : update;
        });
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
        const srv = (status) => ({ servers: [{ id: 's1', name: 'S', module: 'game', status }] });
        let releaseOld;
        window.showToast = vi.fn();
        window.api = {
            serverList: vi
                .fn()
                .mockReturnValueOnce(new Promise((resolve) => (releaseOld = () => resolve(srv('stopped')))))
                .mockResolvedValueOnce(srv('running')),
        };
        let first;
        await act(async () => {
            first = result.current.fetchServers();
            await result.current.fetchServers();
        });
        expect(servers[0].status).toBe('running');
        await act(async () => {
            releaseOld();
            await first;
        });
        expect(servers[0].status).toBe('running');
    });
});
