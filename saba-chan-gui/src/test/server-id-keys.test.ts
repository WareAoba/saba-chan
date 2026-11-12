import { act } from '@testing-library/react';
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import { useServerStore } from '../stores/useServerStore';
import { setupServerStore } from './serverStore';

// 화면은 서버를 인스턴스 id로 찾는다: 이름은 표시에만 쓰므로 이름이 같은 서버가 있어도 누른 서버만 움직인다
describe('서버 동작은 id로 서버를 찾는다', () => {
    let servers: Record<string, unknown>[];
    const hook = () => setupServerStore({ servers });
    const statusOf = (id: string) => useServerStore.getState().servers.find((s) => s.id === id)?.status;

    beforeEach(() => {
        vi.useFakeTimers({ shouldAdvanceTime: true });
        const list = [
            { id: 'x', name: 'Same', module: 'game', status: 'stopped' },
            { id: 'y', name: 'Same', module: 'game', status: 'stopped' },
        ];
        servers = list.map((s) => ({ ...s }));
        window.api = {
            serverList: vi.fn().mockResolvedValue({ servers: list }),
            instanceStart: vi.fn().mockResolvedValue({ success: true }),
            instanceStatus: vi.fn().mockResolvedValue({ status: 'starting' }),
        } as unknown as typeof window.api;
        window.showToast = vi.fn();
    });
    afterEach(() => vi.useRealTimers());

    it('이름이 같은 서버 중 누른 서버만 켜고 켜는 중으로 보인다', async () => {
        const { result } = hook();
        await act(async () => result.current.startServer('y'));
        expect(window.api.instanceStart).toHaveBeenCalledWith('y');
        expect(statusOf('y')).toBe('starting');
        expect(statusOf('x')).toBe('stopped');

        // 목록을 다시 받아도 켜는 중 표시는 그 서버에만 남는다
        await act(async () => result.current.fetchServers());
        await act(async () => result.current.fetchServers());
        expect(statusOf('y')).toBe('starting');
        expect(statusOf('x')).toBe('stopped');
    });
});
