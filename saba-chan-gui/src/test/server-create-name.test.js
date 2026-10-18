import { act, renderHook } from '@testing-library/react';
import { beforeEach, describe, expect, it, vi } from 'vitest';
import { useServerActions } from '../hooks/useServerActions';

describe('서버 만들기: 같은 이름', () => {
    beforeEach(() => {
        window.api = { serverList: vi.fn().mockResolvedValue({ servers: [] }) };
        window.showToast = vi.fn();
    });

    it('데몬이 같은 이름을 거절하면(name_taken) 그 이름이 이미 있다고 알린다', async () => {
        window.api.instanceCreate = vi
            .fn()
            .mockResolvedValue({ success: false, error: 'Name already in use', error_code: 'name_taken' });
        const setModal = vi.fn();
        const { result } = renderHook(() =>
            useServerActions({
                servers: [{ id: 'a', name: 'A', module: 'game', status: 'stopped' }],
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
        await act(async () => result.current.handleAddServer({ name: 'A', module_name: 'game' }));
        const modal = setModal.mock.calls.at(-1)[0];
        expect(modal.type).toBe('failure');
        expect(modal.message).toBe("'A' is already taken");
    });
});
