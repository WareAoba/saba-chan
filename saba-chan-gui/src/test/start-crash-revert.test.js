import { act, renderHook } from '@testing-library/react';
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import { useServerActions } from '../hooks/useServerActions';

// 시작 요청은 성공했지만 프로세스가 바로 죽으면 '시작 중' 표시를 남기지 않는다
describe('시작 직후 프로세스가 죽은 서버', () => {
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
            instanceStart: vi.fn().mockResolvedValue({ success: true }),
            instanceStatus: vi.fn().mockResolvedValue({ status: 'stopped' }),
        };
        window.showToast = vi.fn();
    });
    afterEach(() => vi.useRealTimers());

    it('크래시 판정 뒤 목록 새로고침은 멈춤 상태를 보여 준다', async () => {
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
        await act(async () => result.current.fetchServers());
        await act(async () => result.current.handleStart('A', 'game'));
        for (let i = 0; i < 6; i++) await act(async () => vi.advanceTimersByTime(500));
        expect(window.showToast.mock.calls.at(-1)[1]).toBe('error');
        await act(async () => result.current.fetchServers());
        expect(servers[0].status).toBe('stopped');
    });
});
