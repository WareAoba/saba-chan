import { act, renderHook } from '@testing-library/react';
import { beforeEach, describe, expect, it, vi } from 'vitest';
import { createApi } from '../api/createApi';
import { useServerActions } from '../hooks/useServerActions';

// 서버 jar를 찾지 못했을 때의 복구(경로 바꾸기, 새로 설치)는 설정 저장이 실패하면 성공이라고 하지 않는다
describe('jar 복구의 설정 저장 실패', () => {
    let setModal;
    const setup = () => {
        setModal = vi.fn();
        return renderHook(() =>
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
    };
    beforeEach(() => {
        window.api = {
            serverList: vi.fn().mockResolvedValue({ servers: [] }),
            instanceStart: vi.fn().mockResolvedValue({ action_required: 'server_jar_not_found' }),
            instanceUpdateSettings: vi.fn().mockResolvedValue({ error: 'disk full' }),
            openFileDialog: vi.fn().mockResolvedValue('C:/srv/server.jar'),
            openFolderDialog: vi.fn().mockResolvedValue('C:/srv'),
            moduleInstallServer: vi
                .fn()
                .mockResolvedValue({ success: true, jar_path: 'C:/srv/server.jar', version: '1.0' }),
        };
        window.showToast = vi.fn();
    });
    const toastTypes = () => window.showToast.mock.calls.map((c) => c[1]);

    it('경로 바꾸기: 저장이 실패하면 오류를 보이고 다시 시작하지 않는다', async () => {
        const { result } = setup();
        await act(async () => result.current.handleStart('A', 'game'));
        await act(async () => setModal.mock.calls.at(-1)[0].buttons[0].action());
        expect(toastTypes()).toEqual(['error']);
        expect(window.api.instanceStart).toHaveBeenCalledTimes(1);
    });

    it('새로 설치: 저장이 실패하면 설치 완료라고 하지 않는다', async () => {
        const { result } = setup();
        await act(async () => result.current.handleStart('A', 'game'));
        await act(async () => setModal.mock.calls.at(-1)[0].buttons[1].action());
        expect(toastTypes()).toEqual(['error']);
    });
});

// 모듈 설치 실패(422)는 모듈이 준 메시지를 보인다
describe('설치 오류 본문', () => {
    it('본문에 error가 없으면 message를 오류로 넘긴다', async () => {
        const invoke = vi
            .fn()
            .mockResolvedValue({ status: 422, data: { success: false, message: '체크섬이 맞지 않아요' } });
        const { api } = createApi({ invoke, hub: { dispatch: vi.fn() } });
        const res = await api.moduleInstallServer('game', { version: '1.0' });
        expect(res.error).toBe('체크섬이 맞지 않아요');
    });
});
