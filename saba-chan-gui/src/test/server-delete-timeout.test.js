import { act, renderHook } from '@testing-library/react';
import { beforeEach, describe, expect, it, vi } from 'vitest';
import { useServerActions } from '../hooks/useServerActions';

const srv = { id: 'a', name: 'A', module: 'game', status: 'stopped' };

function setup() {
    const setModal = vi.fn();
    const { result } = renderHook(() =>
        useServerActions({
            servers: [srv],
            setServers: vi.fn(),
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
    return { result, setModal };
}

async function remove(result, setModal) {
    await act(async () => result.current.handleDeleteServer(srv));
    await act(async () => setModal.mock.calls.at(-1)[0].onConfirm());
    return setModal.mock.calls.at(-1)[0];
}

describe('서버 삭제: 답을 기다리다 시간이 다 됐을 때', () => {
    beforeEach(() => {
        window.showToast = vi.fn();
        window.api = { instanceDelete: vi.fn().mockResolvedValue({ error: 'timed out', error_code: 'ETIMEDOUT' }) };
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
});
