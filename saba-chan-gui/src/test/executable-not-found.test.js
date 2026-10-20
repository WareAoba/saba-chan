import { act, renderHook } from '@testing-library/react';
import { describe, expect, it, vi } from 'vitest';
import { useServerActions } from '../hooks/useServerActions';

/**
 * 모듈은 서버 실행 파일을 찾지 못하면 범용 코드 executable_not_found를 준다(예전 모듈은 server_jar_not_found).
 * GUI는 둘 다 경로 다시 고르기·새로 설치를 묻고, 파일 고르기 필터는 게임마다 다른 실행 파일 확장자를 따른다
 */
async function start(startResult) {
    const servers = [{ id: 'a', name: 'A', module: 'game', status: 'stopped' }];
    const setModal = vi.fn();
    window.api = {
        instanceStart: vi.fn().mockResolvedValue(startResult),
        openFileDialog: vi.fn().mockResolvedValue(null),
        serverList: vi.fn().mockResolvedValue({ servers }),
    };
    window.showToast = vi.fn();
    const { result } = renderHook(() =>
        useServerActions({
            servers,
            setServers: vi.fn(),
            modules: [{ name: 'game' }],
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
    await act(async () => result.current.handleStart('A', 'game'));
    return setModal.mock.calls[0]?.[0];
}

async function pickPath(modal) {
    await act(async () => modal.buttons[0].action());
    return window.api.openFileDialog.mock.calls[0][0];
}

describe('서버 실행 파일을 찾지 못했을 때', () => {
    it('범용 코드도 예전 코드처럼 경로 다시 고르기·새로 설치를 묻는다', async () => {
        for (const code of ['executable_not_found', 'server_jar_not_found']) {
            const modal = await start({ success: false, action_required: code, configured_path: 'C:/s/x.jar' });
            expect(modal?.type, code).toBe('question');
            expect(modal.message).toContain('C:/s/x.jar');
        }
    });

    it('파일 고르기 필터는 설정된 경로의 확장자를 따른다', async () => {
        const modal = await start({
            success: false,
            action_required: 'executable_not_found',
            configured_path: 'C:/pal/PalServer.exe',
        });
        const opts = await pickPath(modal);
        expect(opts.filters).toEqual([{ name: 'PalServer.exe', extensions: ['exe'] }]);
    });

    it('설정된 경로가 없으면 모든 파일을 고르게 한다', async () => {
        const opts = await pickPath(await start({ success: false, action_required: 'executable_not_found' }));
        expect(opts.filters).toBeUndefined();
    });

    // 모듈 기본 이름과 다른 파일을 골라도 다음 시작에서 그 파일을 쓴다: 폴더와 함께 실행 파일 경로를 저장한다
    it('고른 파일의 경로와 폴더를 함께 저장한다', async () => {
        const modal = await start({
            success: false,
            action_required: 'executable_not_found',
            configured_path: 'C:/s/x.jar',
        });
        window.api.openFileDialog = vi.fn().mockResolvedValue('D:/mc/paper-1.21.jar');
        window.api.instanceUpdateSettings = vi.fn().mockResolvedValue({ success: true });
        await act(async () => modal.buttons[0].action());
        expect(window.api.instanceUpdateSettings).toHaveBeenCalledWith('a', {
            working_dir: 'D:/mc',
            executable_path: 'D:/mc/paper-1.21.jar',
        });
    });
});
