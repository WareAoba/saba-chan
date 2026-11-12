import { act } from '@testing-library/react';
import { beforeEach, describe, expect, it, vi } from 'vitest';
import { setupServerStore } from './serverStore';

const srv = { id: 'a', name: 'A', module: 'game', status: 'stopped' };

const setup = () => setupServerStore({ servers: [srv] });

async function remove(result: { current: { deleteServer: (s: never) => unknown } }, setModal: ReturnType<typeof vi.fn>) {
    await act(async () => result.current.deleteServer(srv as never));
    await act(async () => setModal.mock.calls.at(-1)![0].onConfirm());
    return setModal.mock.calls.at(-1)![0];
}

describe('서버 삭제: 답을 기다리다 시간이 다 됐을 때', () => {
    beforeEach(() => {
        window.showToast = vi.fn();
        window.api = { instanceDelete: vi.fn().mockResolvedValue({ error: 'timed out', error_code: 'ETIMEDOUT' }) } as unknown as typeof window.api;
    });

    it('목록에서 이미 사라졌으면 삭제된 것으로 알린다', async () => {
        window.api.serverList = vi.fn().mockResolvedValue({ servers: [] });
        const { result, setModal } = setup();
        const modal = await remove(result, setModal);
        expect(modal.type).toBe('success');
    });

    it('아직 목록에 있으면 실패가 아니라 아직 지우는 중이라고 알린다', async () => {
        window.api.serverList = vi.fn().mockResolvedValue({ servers: [srv] });
        const { result, setModal } = setup();
        const modal = await remove(result, setModal);
        expect(modal.type).toBe('notification');
        expect(modal.message).toBe("'A' is still being deleted. Check the server list again in a moment.");
    });

    it('목록도 못 받아 오면 지우는 중이라 단정하지 않고 확인하지 못했다고 알린다', async () => {
        window.api.serverList = vi.fn().mockResolvedValue({ error: 'connect ECONNREFUSED' });
        const { result, setModal } = setup();
        const modal = await remove(result, setModal);
        expect(modal.type).toBe('failure');
        expect(modal.message).toBe("Couldn't confirm whether 'A' was deleted. Check the server list once the daemon responds.");
    });
});
