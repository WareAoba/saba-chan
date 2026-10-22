import { act, renderHook } from '@testing-library/react';
import { describe, expect, it, vi } from 'vitest';
import { useServerActions } from '../hooks/useServerActions';
import i18n from '../i18n';

/** 시작했더니 서버 파일이 없다 → "새로 설치"를 누른다. 설치 응답은 install로 정한다 */
async function openJarModal(modules = [{ name: 'game', game_name: 'Terraria' }], install) {
    const servers = [{ id: 'a', name: 'A', module: 'game', status: 'stopped' }];
    const setModal = vi.fn();
    window.api = {
        instanceStart: vi.fn().mockResolvedValue({ success: false, action_required: 'server_jar_not_found' }),
        openFolderDialog: vi.fn().mockResolvedValue('/srv/a'),
        moduleListVersions: vi.fn().mockResolvedValue({ latest: { release: '9.9' } }),
        moduleInstallServer: vi.fn().mockResolvedValue(install),
        instanceUpdateSettings: vi.fn().mockResolvedValue({ success: true }),
        serverList: vi.fn().mockResolvedValue({ servers }),
    };
    window.showToast = vi.fn();
    const { result } = renderHook(() =>
        useServerActions({
            servers,
            setServers: vi.fn(),
            modules,
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
    return setModal.mock.calls[0][0];
}

async function installNew(install) {
    const modal = await openJarModal(undefined, install);
    await act(async () => modal.buttons[1].action());
    return window.showToast.mock.calls.at(-1)[0];
}

// 서버 파일이 없을 때 "새로 설치"는 최신 버전을 데몬이 그 인스턴스 설정(서버 종류)으로 고르게 맡긴다 (L23)
describe('서버 파일이 없을 때 새로 설치', () => {
    it('버전을 묻지 않고 instance_id를 보내며, 데몬이 고른 버전을 그 게임 이름과 알린다', async () => {
        const toast = await installNew({ success: true, version: '1.21.4', install_path: '/srv/a', java_warning: 'x' });
        expect(window.api.moduleListVersions).not.toHaveBeenCalled();
        expect(window.api.moduleInstallServer).toHaveBeenCalledWith('game', {
            instance_id: 'a',
            install_dir: '/srv/a',
        });
        expect(toast).toContain('Terraria 1.21.4');
    });

    // 데몬의 영어 원문 대신 이유를 현지화해 보인다 (리뷰 343 P3-1)
    it.each([
        ['instance_busy', /Another task/],
        ['server_running', /Stop the server/],
        ['version_unknown', /latest version.*no releases/],
    ])('설치를 거절한 이유(%s)를 현지화해 보인다', async (code, text) => {
        const toast = await installNew({ success: false, error_code: code, error: 'no releases' });
        expect(toast).toMatch(text);
    });

    // 모듈이 install_server를 구현하지 않으면(functions 목록에 없음) "새로 설치"를 보이지 않는다 (MK1)
    it('설치를 구현하지 않은 모듈이면 새로 설치를 보이지 않는다', async () => {
        const modal = await openJarModal([{ name: 'game', game_name: 'Terraria', functions: ['validate'] }]);
        expect(modal.buttons.map((b) => b.label)).not.toContain(i18n.t('gui:servers.jar_action_install_new'));
        expect(modal.buttons).toHaveLength(2);
    });

    // 받은 실행 파일·작업 폴더는 데몬이 설치를 마치며 그 인스턴스에 기록한다 (0.1.10 결정 3, 외부 감사 10-20 #5)
    it('받은 실행 파일 기록은 데몬에 맡긴다', async () => {
        await installNew({
            success: true,
            version: '1.21.4',
            install_path: '/srv/a',
            executable_path: '/srv/a/server.jar',
        });
        expect(window.api.instanceUpdateSettings).not.toHaveBeenCalled();
    });
});
