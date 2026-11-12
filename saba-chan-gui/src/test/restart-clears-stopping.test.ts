import { act } from '@testing-library/react';
import { beforeEach, describe, expect, it, vi } from 'vitest';
import { setupServerStore } from './serverStore';

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
        } as unknown as typeof window.api;
        const { result, setModal } = setupServerStore({ servers: list });
        await act(async () => result.current.fetchServers());
        await act(async () => result.current.restartServer('a'));
        await act(async () => setModal.mock.calls.at(-1)![0].onConfirm());
        await act(async () => result.current.fetchServers());
        expect(result.current.servers[0].status).toBe('running');
    });

    it('이미 재시작을 고른 곳에서 부르면 다시 묻지 않고 바로 재시작한다', async () => {
        window.api = {
            serverList: vi.fn(async () => ({ servers: [] })),
            instanceRestart: vi.fn(async () => ({ success: true })),
        } as unknown as typeof window.api;
        const { result, setModal } = setupServerStore({ servers: [{ id: 'a', name: 'A', module: 'game', status: 'running' } as never] });
        await act(async () => result.current.restartServer('a', { confirmed: true }));
        expect(setModal).not.toHaveBeenCalledWith(expect.objectContaining({ type: 'question' }));
        expect(window.api.instanceRestart).toHaveBeenCalledWith('a', { force: false });
    });
});
