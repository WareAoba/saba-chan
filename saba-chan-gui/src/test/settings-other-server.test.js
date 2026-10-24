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
const srvA = {
    id: 'a',
    name: 'A',
    module: 'dl',
    status: 'stopped',
    module_settings: { working_dir: 'C:/a', server_version: '1.0' },
};
const srvB = {
    id: 'b',
    name: 'B',
    module: 'dl',
    status: 'stopped',
    module_settings: { working_dir: 'C:/b', server_version: '2.0' },
};

function setup() {
    const setModal = vi.fn();
    const hook = renderHook(() =>
        useServerSettings({ servers: [srvA, srvB], modules, setModal, setProgressBar: vi.fn(), fetchServers: vi.fn() }),
    );
    return { ...hook, setModal };
}

/** 끝나지 않은 약속과 그것을 끝내는 함수 */
const deferred = () => {
    let resolve;
    const promise = new Promise((r) => {
        resolve = r;
    });
    return { promise, resolve };
};

describe('작업 중에 다른 서버 설정을 열었을 때', () => {
    beforeEach(() => {
        window.showToast = vi.fn();
        window.api = {
            serverList: vi.fn().mockResolvedValue(null),
            moduleListVersions: vi.fn().mockResolvedValue({ versions: ['1.0', '3.0'] }),
            instanceGetInstalledVersion: vi.fn().mockResolvedValue({ success: false }),
            instanceUpdateSettings: vi.fn().mockResolvedValue({ success: true }),
        };
    });

    it('끝난 버전 설치가 나중에 연 서버의 버전 칸을 바꾸지 않는다', async () => {
        const install = deferred();
        window.api.moduleInstallServer = vi.fn(() => install.promise);
        const { result, setModal } = setup();
        await act(async () => result.current.handleOpenSettings(srvA));
        act(() => result.current.handleSettingChange('server_version', '3.0'));
        await act(async () => result.current.handleInstallVersion());
        let running;
        act(() => {
            running = setModal.mock.calls.at(-1)[0].buttons[0].action();
        });
        await act(async () => result.current.handleOpenSettings(srvB));
        await act(async () => {
            install.resolve({ success: true, provisioning: true, id: 'a' });
            await running;
        });
        // A의 설치가 시작돼도 B 창은 닫히지 않고 B의 버전 그대로다
        expect(window.api.instanceUpdateSettings).not.toHaveBeenCalled();
        expect(result.current.showSettingsModal).toBe(true);
        expect(result.current.settingsServer.id).toBe('b');
        expect(result.current.settingsValues.server_version).toBe('2.0');
    });

    it('끝난 초기화가 나중에 연 서버 창을 앞 서버 값으로 바꾸지 않는다', async () => {
        const reset = deferred();
        window.api.instanceResetServer = vi.fn(() => reset.promise);
        const { result, setModal } = setup();
        await act(async () => result.current.handleOpenSettings(srvA));
        await act(async () => result.current.handleResetServer());
        let running;
        act(() => {
            running = setModal.mock.calls.at(-1)[0].buttons[1].action();
        });
        await act(async () => result.current.handleOpenSettings(srvB));
        window.api.serverList = vi.fn().mockResolvedValue({ servers: [srvA, srvB] });
        await act(async () => {
            reset.resolve({ success: true, deleted: [] });
            await running;
        });
        expect(result.current.settingsServer.id).toBe('b');
        expect(result.current.settingsValues.server_version).toBe('2.0');
    });
});
