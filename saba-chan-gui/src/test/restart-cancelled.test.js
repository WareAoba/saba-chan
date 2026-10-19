import { act, renderHook } from '@testing-library/react';
import { beforeEach, describe, expect, it, vi } from 'vitest';
import i18n from '../i18n';
import { useServerActions } from '../hooks/useServerActions';

// 재시작을 기다리는 동안 다른 쪽에서 정지·시작·삭제해 데몬이 그만둔 것은 실패가 아니다
const restartWith = async (response) => {
    const list = [{ id: 'a', name: 'A', module: 'game', status: 'running', pid: 10 }];
    window.api = {
        serverList: vi.fn(async () => ({ servers: list })),
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
    await act(async () => result.current.handleRestart('A'));
    await act(async () => setModal.mock.calls.at(-1)[0].onConfirm());
    return window.showToast.mock.calls.at(-1);
};

describe('그만둔 재시작', () => {
    beforeEach(() => {
        window.showToast = vi.fn();
    });

    it('켜는 중에 정지가 들어와 그만뒀으면 재시작 실패가 아니라 그 이유를 알린다', async () => {
        const [message, type] = await restartWith({
            success: false,
            error: 'start_cancelled',
            error_code: 'start_cancelled',
        });
        expect(message).toBe(i18n.t('errors.start_cancelled'));
        expect(type).toBe('info');
    });

    it('기다리는 동안 다른 곳에서 켜거나 지워 그만뒀으면 그 이유를 알린다', async () => {
        const [message, type] = await restartWith({
            success: false,
            error: 'The server was started or removed while waiting',
            error_code: 'restart_cancelled',
        });
        expect(message).toBe(i18n.t('errors.restart_cancelled'));
        expect(message).not.toMatch(/started or removed while waiting/);
        expect(type).toBe('info');
    });
});
