import { act } from '@testing-library/react';
import { beforeEach, describe, expect, it, vi } from 'vitest';
import { setupServerStore } from './serverStore';

const linked = {
    id: 'a',
    name: 'A',
    module: 'game',
    status: 'stopped',
    linked_folder: 'D:\\Games\\pal',
    restorable_settings: true,
};

function setup(servers: Record<string, unknown>[]) {
    const { result, setModal } = setupServerStore({ servers });
    return { result, setModal };
}

describe('가져온 서버 삭제(연결 해제)', () => {
    beforeEach(() => {
        window.showToast = vi.fn();
        window.api = { instanceDelete: vi.fn().mockResolvedValue({ success: true }), serverList: vi.fn() } as unknown as typeof window.api;
    });

    it('서버 폴더는 남는다고 알리고, 설정을 되돌릴지 그대로 둘지 고르게 한다', async () => {
        const { result, setModal } = setup([linked]);
        await act(async () => result.current.deleteServer(linked as never));
        const modal = setModal.mock.calls.at(-1)![0];
        expect(modal.detail).toContain('D:\\Games\\pal');
        expect(modal.buttons).toHaveLength(3);

        await act(async () => modal.buttons[0].action());
        expect(window.api.instanceDelete).toHaveBeenLastCalledWith('a', { keepSettings: false });

        await act(async () => result.current.deleteServer(linked as never));
        await act(async () => setModal.mock.calls.at(-1)![0].buttons[1].action());
        expect(window.api.instanceDelete).toHaveBeenLastCalledWith('a', { keepSettings: true });
    });

    it('되돌릴 설정이 없으면 폴더가 남는다는 안내와 확인만 묻는다', async () => {
        const old = { ...linked, restorable_settings: false };
        const { result, setModal } = setup([old]);
        await act(async () => result.current.deleteServer(old as never));
        const modal = setModal.mock.calls.at(-1)![0];
        expect(modal.detail).toContain('D:\\Games\\pal');
        expect(modal.buttons).toBeUndefined();
        await act(async () => modal.onConfirm());
        expect(window.api.instanceDelete).toHaveBeenLastCalledWith('a', { keepSettings: false });
    });

    it('옮겨 온 서버도 밖에 남는 설정을 되돌릴지 그대로 둘지 고르게 한다', async () => {
        const moved = { ...linked, linked_folder: undefined };
        const { result, setModal } = setup([moved]);
        await act(async () => result.current.deleteServer(moved as never));
        const modal = setModal.mock.calls.at(-1)![0];
        expect(modal.buttons).toHaveLength(3);
        await act(async () => modal.buttons[1].action());
        expect(window.api.instanceDelete).toHaveBeenLastCalledWith('a', { keepSettings: true });
    });

    it('되돌리지 못한 설정 사본은 데몬이 알려준 보관 위치와 함께 알린다', async () => {
        vi.mocked(window.api.instanceDelete).mockResolvedValue({ success: true, linked_folder: 'D:\\Games\\pal', settings_restore_failed: 2, settings_kept_at: 'D:\\state\\unrestored-settings\\a-1' });
        const { result, setModal } = setup([linked]);
        await act(async () => result.current.deleteServer(linked as never));
        await act(async () => setModal.mock.calls.at(-1)![0].buttons[0].action());
        expect(setModal.mock.calls.at(-1)![0].message).toContain('D:\\state\\unrestored-settings\\a-1');
    });

    it('일부 설정 파일을 되돌리지 못했으면 성공과 함께 알린다', async () => {
        vi.mocked(window.api.instanceDelete).mockResolvedValue({ success: true, linked_folder: 'D:\\Games\\pal', settings_restored: 1, settings_restore_failed: 1 });
        const { result, setModal } = setup([linked]);
        await act(async () => result.current.deleteServer(linked as never));
        await act(async () => setModal.mock.calls.at(-1)![0].buttons[0].action());
        const done = setModal.mock.calls.at(-1)![0];
        expect(done.type).toBe('success');
        expect(done.message).toContain('1');
    });
});

describe('기존 서버 가져오기', () => {
    it('고른 방식(옮기기)을 데몬 생성 요청에 싣는다', async () => {
        window.showToast = vi.fn();
        window.api = { instanceCreate: vi.fn().mockResolvedValue({ success: true, id: 'b' }), serverList: vi.fn().mockResolvedValue({ servers: [] }) } as unknown as typeof window.api;
        const { result } = setup([]);
        await act(async () =>
            result.current.addServer({ name: 'B', module_name: 'game', migration_source: '/srv/old', migration_mode: 'move' }),
        );
        expect(vi.mocked(window.api.instanceCreate).mock.calls[0][0]).toMatchObject({ migration_source: '/srv/old', migration_mode: 'move' });
    });
});

describe('일반 서버 삭제', () => {
    beforeEach(() => {
        window.showToast = vi.fn();
        window.api = { instanceDelete: vi.fn().mockResolvedValue({ success: true }), serverList: vi.fn() } as unknown as typeof window.api;
    });

    it('서버 파일도 함께 지워진다고 알린다 (옮겨 온 서버도 같다)', async () => {
        const plain = { id: 'b', name: 'B', module: 'game', status: 'stopped' };
        const { result, setModal } = setup([plain]);
        await act(async () => result.current.deleteServer(plain as never));
        expect(setModal.mock.calls.at(-1)![0].detail).toMatch(/delete_detail|지워져|deleted/);
    });
});
