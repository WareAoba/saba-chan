import { act, renderHook } from '@testing-library/react';
import { beforeEach, describe, expect, it, vi } from 'vitest';
import { useServerActions } from '../hooks/useServerActions';
import i18n from '../i18n';

// 데몬이 Windows 예약 범위를 피해 REST 포트를 옮겼으면(port_moved) 저장값이 조용히 바뀌지 않게 알린다 (M13)
const startWith = async (response, action = 'start') => {
    const list = [{ id: 'a', name: 'A', module: 'game', status: 'stopped' }];
    window.api = {
        serverList: vi.fn(async () => ({ servers: list })),
        instanceStart: vi.fn(async () => response),
        instanceRestart: vi.fn(async () => response),
    };
    const setModal = vi.fn();
    const { result } = renderHook(() =>
        useServerActions({
            servers: list,
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
    if (action === 'restart') {
        await act(async () => result.current.handleRestart('A'));
        await act(async () => setModal.mock.calls.at(-1)[0].onConfirm());
    } else {
        await act(async () => result.current.handleStart('A', 'game'));
    }
    return window.showToast.mock.calls.map((c) => c[0]);
};

const moved = [{ field: 'rest_port', from: 8212, to: 8242 }];

describe('옮긴 포트 안내', () => {
    beforeEach(() => {
        window.showToast = vi.fn();
    });

    it('시작에 성공해도 옮긴 포트를 알린다', async () => {
        const toasts = await startWith({ success: true, port_moved: moved });
        expect(toasts).toContain(i18n.t('servers.port_moved', { from: 8212, to: 8242 }));
    });

    it('시작에 실패해도 옮긴 포트를 알린다', async () => {
        const toasts = await startWith({ success: false, error: 'boom', port_moved: moved });
        expect(toasts).toContain(i18n.t('servers.port_moved', { from: 8212, to: 8242 }));
    });

    it('재시작에서도 옮긴 포트를 알린다', async () => {
        const toasts = await startWith({ success: true, port_moved: moved }, 'restart');
        expect(toasts).toContain(i18n.t('servers.port_moved', { from: 8212, to: 8242 }));
    });

    it('옮기지 않았으면 알리지 않는다', async () => {
        const toasts = await startWith({ success: true });
        expect(toasts.some((m) => /8242/.test(m))).toBe(false);
    });
});
