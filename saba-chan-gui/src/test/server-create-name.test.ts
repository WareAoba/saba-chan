import { act } from '@testing-library/react';
import { beforeEach, describe, expect, it, vi } from 'vitest';
import { setupServerStore } from './serverStore';

describe('서버 만들기: 같은 이름', () => {
    beforeEach(() => {
        window.api = { serverList: vi.fn().mockResolvedValue({ servers: [] }) } as unknown as typeof window.api;
        window.showToast = vi.fn();
    });

    it('데몬이 같은 이름을 거절하면(name_taken) 그 이름이 이미 있다고 알린다', async () => {
        window.api.instanceCreate = vi
            .fn()
            .mockResolvedValue({ success: false, error: 'Name already in use', error_code: 'name_taken' });
        const { result, setModal } = setupServerStore({ servers: [{ id: 'a', name: 'A', module: 'game', status: 'stopped' } as never] });
        let created: unknown;
        await act(async () => {
            created = await result.current.addServer({ name: 'A', module_name: 'game' });
        });
        // 만들기 화면이 입력을 지우지 않도록 실패를 돌려준다
        expect(created).toBe(false);
        const modal = setModal.mock.calls.at(-1)![0];
        expect(modal.type).toBe('failure');
        expect(modal.message).toBe("'A' is already taken");
    });
});

describe('서버 만들기: 마법사에서 고른 값', () => {
    beforeEach(() => {
        window.api = {
            serverList: vi.fn().mockResolvedValue({ servers: [] }),
            instanceCreate: vi.fn().mockResolvedValue({ success: true, id: 'new' }),
        } as unknown as typeof window.api;
        window.showToast = vi.fn();
    });

    it('버전·폴더·실행 방식·포트를 데몬 만들기 요청에 그대로 싣는다', async () => {
        const { result } = setupServerStore();
        await act(async () => {
            await result.current.addServer({
                name: 'A',
                module_name: 'game',
                version: '1.21.4',
                working_dir: 'D:\\Servers\\A',
                managed_start: false,
                port: 25566,
            });
        });
        expect(window.api.instanceCreate).toHaveBeenCalledWith(
            expect.objectContaining({
                version: '1.21.4',
                working_dir: 'D:\\Servers\\A',
                managed_start: false,
                port: 25566,
            }),
        );
    });
});
