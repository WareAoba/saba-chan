/**
 * G-S4: 서버 목록 요청 둘이 겹쳐 먼저 보낸 것이 늦게 오면, 옛 상태가 한 폴링 동안 보였다.
 */
import { act } from '@testing-library/react';
import { describe, expect, it, vi } from 'vitest';
import { setupServerStore } from './serverStore';

describe('서버 목록 응답 순서', () => {
    it('늦게 온 옛 응답은 새 응답을 덮지 않는다', async () => {
        const { result } = setupServerStore();
        const srv = (status: string) => ({ servers: [{ id: 's1', name: 'S', module: 'game', status }] });
        let releaseOld!: (value?: unknown) => void;
        window.showToast = vi.fn();
        window.api = {
            serverList: vi
                .fn()
                .mockReturnValueOnce(new Promise((resolve) => (releaseOld = () => resolve(srv('stopped')))))
                .mockResolvedValueOnce(srv('running')),
        } as unknown as typeof window.api;
        let first: Promise<unknown>;
        await act(async () => {
            first = result.current.fetchServers();
            await result.current.fetchServers();
        });
        expect(result.current.servers[0].status).toBe('running');
        await act(async () => {
            releaseOld();
            await first;
        });
        expect(result.current.servers[0].status).toBe('running');
    });
});
