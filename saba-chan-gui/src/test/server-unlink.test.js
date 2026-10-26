import { act, renderHook } from '@testing-library/react';
import { beforeEach, describe, expect, it, vi } from 'vitest';
import { useServerActions } from '../hooks/useServerActions';

const linked = {
    id: 'a',
    name: 'A',
    module: 'game',
    status: 'stopped',
    linked_folder: 'D:\\Games\\pal',
    restorable_settings: true,
};

function setup(servers) {
    const setModal = vi.fn();
    const { result } = renderHook(() =>
        useServerActions({
            servers,
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

describe('가져온 서버 삭제(연결 해제)', () => {
    beforeEach(() => {
        window.showToast = vi.fn();
        window.api = { instanceDelete: vi.fn().mockResolvedValue({ success: true }), serverList: vi.fn() };
    });

    it('서버 폴더는 남는다고 알리고, 설정을 되돌릴지 그대로 둘지 고르게 한다', async () => {
        const { result, setModal } = setup([linked]);
        await act(async () => result.current.handleDeleteServer(linked));
        const modal = setModal.mock.calls.at(-1)[0];
        expect(modal.detail).toContain('D:\\Games\\pal');
        expect(modal.buttons).toHaveLength(3);

        await act(async () => modal.buttons[0].action());
        expect(window.api.instanceDelete).toHaveBeenLastCalledWith('a', { keepSettings: false });

        await act(async () => result.current.handleDeleteServer(linked));
        await act(async () => setModal.mock.calls.at(-1)[0].buttons[1].action());
        expect(window.api.instanceDelete).toHaveBeenLastCalledWith('a', { keepSettings: true });
    });

    it('되돌릴 설정이 없으면 폴더가 남는다는 안내와 확인만 묻는다', async () => {
        const old = { ...linked, restorable_settings: false };
        const { result, setModal } = setup([old]);
        await act(async () => result.current.handleDeleteServer(old));
        const modal = setModal.mock.calls.at(-1)[0];
        expect(modal.detail).toContain('D:\\Games\\pal');
        expect(modal.buttons).toBeUndefined();
        await act(async () => modal.onConfirm());
        expect(window.api.instanceDelete).toHaveBeenLastCalledWith('a', { keepSettings: false });
    });

    it('일부 설정 파일을 되돌리지 못했으면 성공과 함께 알린다', async () => {
        window.api.instanceDelete.mockResolvedValue({ success: true, linked_folder: 'D:\\Games\\pal', settings_restored: 1, settings_restore_failed: 1 });
        const { result, setModal } = setup([linked]);
        await act(async () => result.current.handleDeleteServer(linked));
        await act(async () => setModal.mock.calls.at(-1)[0].buttons[0].action());
        const done = setModal.mock.calls.at(-1)[0];
        expect(done.type).toBe('success');
        expect(done.message).toContain('1');
    });
});
