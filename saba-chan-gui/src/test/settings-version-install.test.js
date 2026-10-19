import { act, renderHook } from '@testing-library/react';
import { beforeEach, describe, expect, it, vi } from 'vitest';
import { useServerSettings } from '../hooks/useServerSettings';

const modules = [
    {
        name: 'dl',
        install: { method: 'download' },
        settings: { fields: [{ name: 'motd', field_type: 'text', group: 'saba-chan' }] },
    },
];
const srv = {
    id: 'a',
    name: 'A',
    module: 'dl',
    status: 'stopped',
    module_settings: { working_dir: 'C:/a', server_version: '1.0', server_type: 'paper' },
};

function setup() {
    const setModal = vi.fn();
    const hook = renderHook(() =>
        useServerSettings({ servers: [srv], modules, setModal, setProgressBar: vi.fn(), fetchServers: vi.fn() }),
    );
    return { ...hook, setModal };
}

// 버전은 설치한 것만 기록한다. 설치는 그 인스턴스 설정(서버 종류 등)으로 한다
describe('서버 설정의 버전', () => {
    beforeEach(() => {
        window.showToast = vi.fn();
        window.api = {
            serverList: vi.fn().mockResolvedValue(null),
            moduleListVersions: vi.fn().mockResolvedValue({ versions: ['1.0', '3.0'] }),
            instanceGetInstalledVersion: vi.fn().mockResolvedValue({ success: false }),
            instanceUpdateSettings: vi.fn().mockResolvedValue({ success: true }),
            moduleInstallServer: vi.fn().mockResolvedValue({ success: true }),
        };
    });

    it('설치 요청에 인스턴스 id를 담는다', async () => {
        const { result, setModal } = setup();
        await act(async () => result.current.handleOpenSettings(srv));
        act(() => result.current.handleSettingChange('server_version', '3.0'));
        await act(async () => result.current.handleInstallVersion());
        await act(async () => setModal.mock.calls.at(-1)[0].buttons[0].action());
        expect(window.api.moduleInstallServer).toHaveBeenCalledWith(
            'dl',
            expect.objectContaining({ version: '3.0', instance_id: 'a' }),
        );
    });

    it('설치하지 않고 고른 버전은 저장에 실리지 않는다', async () => {
        const { result } = setup();
        await act(async () => result.current.handleOpenSettings(srv));
        act(() => result.current.handleSettingChange('server_version', '3.0'));
        await act(async () => result.current.handleSaveSettings());
        expect(window.api.instanceUpdateSettings).toHaveBeenCalled();
        expect(window.api.instanceUpdateSettings.mock.calls.at(-1)[1]).not.toHaveProperty('server_version');
    });

    it('jar 경로가 없는 설치도 설치한 버전을 기록한다', async () => {
        const { result, setModal } = setup();
        await act(async () => result.current.handleOpenSettings(srv));
        act(() => result.current.handleSettingChange('server_version', '3.0'));
        await act(async () => result.current.handleInstallVersion());
        await act(async () => setModal.mock.calls.at(-1)[0].buttons[0].action());
        expect(window.api.instanceUpdateSettings).toHaveBeenCalledWith(
            'a',
            expect.objectContaining({ server_version: '3.0' }),
        );
    });
});
