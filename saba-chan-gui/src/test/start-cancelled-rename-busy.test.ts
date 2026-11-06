import { act, renderHook } from '@testing-library/react';
import { beforeEach, describe, expect, it, vi } from 'vitest';
import { useServerActions } from '../hooks/useServerActions';

function setup() {
    const setModal = vi.fn();
    const { result } = renderHook(() =>
        useServerActions({
            servers: [{ id: 'a', name: 'A', module: 'game', status: 'stopped' } as never],
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
        } as never),
    );
    return { result, setModal };
}

describe('시작 중에 사용자가 정지함 (start_cancelled)', () => {
    beforeEach(() => {
        window.api = { serverList: vi.fn().mockResolvedValue({ servers: [] }) } as unknown as typeof window.api;
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

// 사용자 11-06 #10: 이미 켜진 서버에 시작을 보내면 "시작 완료" 대신 이미 실행 중이라고 알린다
describe('이미 실행 중인 서버 시작 (already_running)', () => {
    beforeEach(() => {
        window.api = { serverList: vi.fn().mockResolvedValue({ servers: [] }) } as unknown as typeof window.api;
        window.showToast = vi.fn();
    });

    it('시작 진행 막대를 띄우지 않고 이미 실행 중이라고만 알린다', async () => {
        window.api.instanceStart = vi.fn().mockResolvedValue({ success: true, already_running: true, pid: 1 });
        window.api.instanceStatus = vi.fn();
        const setProgressBar = vi.fn();
        const { result } = renderHook(() =>
            useServerActions({
                servers: [{ id: 'a', name: 'A', module: 'game', status: 'running' } as never],
                setServers: vi.fn(),
                modules: [],
                loading: false,
                setLoading: vi.fn(),
                setModal: vi.fn(),
                setProgressBar,
                openConsole: vi.fn(),
                closeConsole: vi.fn(),
                setShowModuleManager: vi.fn(),
                formatUptime: vi.fn(),
            } as never),
        );
        await act(async () => result.current.handleStart('A', 'game'));
        expect(setProgressBar).not.toHaveBeenCalled();
        expect(window.api.instanceStatus).not.toHaveBeenCalled();
        expect(vi.mocked(window.showToast!).mock.calls.at(-1)!.slice(0, 3)).toEqual(['ℹ️ “A” is already running', 'info', 3000]);
    });
});

describe('이름 바꾸기: 다른 작업 중 (instance_busy)', () => {
    beforeEach(() => {
        window.api = { serverList: vi.fn().mockResolvedValue({ servers: [] }) } as unknown as typeof window.api;
        window.showToast = vi.fn();
    });

    it('시작·설치·백업 같은 다른 작업 중이라 못 바꾼다고 알린다', async () => {
        window.api.instanceRename = vi
            .fn()
            .mockResolvedValue({ success: false, error: 'busy', error_code: 'instance_busy' });
        const { result } = setup();
        let ok: unknown;
        await act(async () => {
            ok = await result.current.handleRename('a', 'B');
        });
        expect(ok).toBe(false);
        expect(vi.mocked(window.showToast!).mock.calls.at(-1)![0]).toMatch(
            /another task \(starting, installing, updating, backing up or restoring\)/,
        );
    });
});
