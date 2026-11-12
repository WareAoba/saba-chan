import { act, renderHook } from '@testing-library/react';
import { beforeEach, describe, expect, it, vi } from 'vitest';
import { useServerActions } from '../hooks/useServerActions';
import i18n from '../i18n';
import type { Server } from '../stores/useServerStore';

// 재시작을 기다리는 동안 다른 쪽에서 정지·시작·삭제해 데몬이 그만둔 것은 실패가 아니다
const restartWith = async (response: Record<string, unknown>) => {
    const list = [{ id: 'a', name: 'A', module: 'game', status: 'running', pid: 10 }] as unknown as Server[];
    window.api = {
        serverList: vi.fn(async () => ({ servers: list })),
        instanceRestart: vi.fn(async () => response),
    } as unknown as typeof window.api;
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
            setShowModuleManager: vi.fn(),
        }),
    );
    await act(async () => result.current.handleRestart('A'));
    await act(async () => setModal.mock.calls.at(-1)![0].onConfirm());
    return vi.mocked(window.showToast!).mock.calls.at(-1)!;
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

    it('서버가 아직 꺼지는 중이라 다시 켜지 못했으면 데몬의 영어 문장이 아니라 번역된 이유를 알린다', async () => {
        const [message, type] = await restartWith({
            success: false,
            stopped: false,
            error: 'The server is still shutting down; start it again once it has stopped',
            error_code: 'still_stopping',
        });
        expect(message).toBe(i18n.t('errors.still_stopping'));
        expect(type).toBe('error');
    });
});
