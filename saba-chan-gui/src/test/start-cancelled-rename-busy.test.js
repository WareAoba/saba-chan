import { act, renderHook } from '@testing-library/react';
import { beforeEach, describe, expect, it, vi } from 'vitest';
import { useServerActions } from '../hooks/useServerActions';

function setup() {
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
    return { result, setModal };
}

describe('시작 중에 사용자가 정지함 (start_cancelled)', () => {
    beforeEach(() => {
        window.api = { serverList: vi.fn().mockResolvedValue({ servers: [] }) };
        window.showToast = vi.fn();
    });

    it('실패 창이나 오류 토스트 없이 시작 표시만 되돌린다', async () => {
        window.api.instanceStart = vi.fn().mockResolvedValue({
            success: false,
            error: 'start_cancelled',
            error_code: 'start_cancelled',
            message: "Server 'A' was stopped while it was starting",
        });
        const { result, setModal } = setup();
        await act(async () => result.current.handleStart('A', 'game'));
        expect(setModal).not.toHaveBeenCalled();
        expect(window.showToast).not.toHaveBeenCalled();
        // 낙관적 'starting'을 지우고 데몬 상태를 다시 읽는다
        expect(window.api.serverList).toHaveBeenCalled();
    });
});

describe('이름 바꾸기: 다른 작업 중 (instance_busy)', () => {
    beforeEach(() => {
        window.api = { serverList: vi.fn().mockResolvedValue({ servers: [] }) };
        window.showToast = vi.fn();
    });

    it('설치·업데이트·백업·복원 중이라 못 바꾼다고 알린다', async () => {
        window.api.instanceRename = vi
            .fn()
            .mockResolvedValue({ success: false, error: 'busy', error_code: 'instance_busy' });
        const { result } = setup();
        let ok;
        await act(async () => {
            ok = await result.current.handleRename('a', 'B');
        });
        expect(ok).toBe(false);
        expect(window.showToast.mock.calls.at(-1)[0]).toMatch(/installing, updating, backing up or restoring/);
    });
});
