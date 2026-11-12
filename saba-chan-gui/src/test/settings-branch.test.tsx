import type { ComponentProps } from 'react';
import { act, fireEvent, render, renderHook, screen } from '@testing-library/react';
import { beforeEach, describe, expect, it, vi } from 'vitest';

vi.mock('../contexts/ExtensionContext', async (original) => ({
    ...(await original<typeof import('../contexts/ExtensionContext')>()),
    useExtensions: () => ({ extensions: [{ id: 'steamcmd', name: 'SteamCMD' }], slots: {} }),
}));

import { ServerSettingsModal } from '../components/ServerSettingsModal';
import { useServerSettings } from '../hooks/useServerSettings';

// 시안 10: 설치 방식 익스텐션(Steam 등)으로 받는 서버는 버전 칸에서 브랜치를 고른다.
// 내부 동작은 버전 설치와 같다: 단추를 눌러 그 브랜치로 받고, 받은 브랜치를 데몬이 적는다
const branches = [
    { id: 'public', name: 'Stable (default)' },
    { id: 'unstable', name: 'Unstable (unstable)' },
    { id: 'legacy41', name: 'Old build 41 (legacy41)' },
];
const modules = [
    { name: 'pz', install: { method: 'steamcmd' }, functions: ['list_versions', 'install_server'], settings: { fields: [] } },
    { name: 'dl', install: { method: 'download' }, functions: ['list_versions', 'install_server'], settings: { fields: [] } },
];
const steamServer = {
    id: 'z1',
    name: 'Zombies',
    module: 'pz',
    status: 'stopped',
    working_dir: 'C:/pz',
    extension_data: { install_method_steamcmd: true, steamcmd_beta: 'unstable' },
};

const show = (server: Record<string, unknown>, version: string, handleInstallVersion = vi.fn()) =>
    render(
        <ServerSettingsModal
            {...({
                section: 'saba',
                embedded: true,
                settingsServer: server,
                settingsValues: { server_version: version },
                settingsSource: 'live',
                settingsActiveTab: 'general',
                setSettingsActiveTab: vi.fn(),
                modules,
                advancedExpanded: false,
                setAdvancedExpanded: vi.fn(),
                availableVersions: branches,
                versionsLoading: false,
                versionInstalling: false,
                handleSettingChange: vi.fn(),
                handleInstallVersion,
                handleSaveSettings: vi.fn(),
                handleResetServer: vi.fn(),
                resettingServer: false,
                onClose: vi.fn(),
                servers: [server],
            } as unknown as ComponentProps<typeof ServerSettingsModal>)}
        />,
    );

describe('서버마다 고르는 브랜치', () => {
    it('버전 칸이 브랜치 칸으로 보이고, 단추를 눌러야 받는다', () => {
        const install = vi.fn();
        const { container } = show(steamServer, 'unstable', install);
        expect(screen.getByText('SteamCMD branch')).toBeTruthy();
        expect(screen.getByText('Pick the same branch as your friends, or they cannot join.')).toBeTruthy();
        expect(screen.getByText('Unstable (unstable)')).toBeTruthy();
        expect(container.querySelector('.ss-branch-notice')).not.toBeNull();
        fireEvent.click(screen.getByRole('button', { name: /Download this branch now/ }));
        expect(install).toHaveBeenCalled();
    });

    it('버전을 받는 서버의 칸은 그대로다', () => {
        const { container } = show({ id: 'm1', name: 'MC', module: 'dl', status: 'stopped' }, 'public');
        expect(screen.getByText('Server Version')).toBeTruthy();
        expect(container.querySelector('.ss-branch-notice')).toBeNull();
    });
});

describe('브랜치 목록 읽기', () => {
    beforeEach(() => {
        window.showToast = vi.fn();
        window.api = {
            serverList: vi.fn().mockResolvedValue(null),
            moduleListVersions: vi.fn().mockResolvedValue({ versions: branches, latest: { release: 'public' } }),
            instanceGetInstalledVersion: vi.fn().mockResolvedValue({ success: true, version: '42.0' }),
            moduleInstallServer: vi.fn().mockResolvedValue({ success: true, provisioning: true }),
        } as unknown as typeof window.api;
    });
    const open = async (server: Record<string, unknown>) => {
        const setModal = vi.fn();
        const hook = renderHook(() =>
            useServerSettings({ servers: [server] as never, modules: modules as never, setModal, setProgressBar: vi.fn(), fetchServers: vi.fn() }),
        );
        await act(async () => hook.result.current.handleOpenSettings(server as never));
        return { ...hook, setModal };
    };

    it('데몬이 적어 둔 브랜치를 고른 채로 열고, 받을 때 그 브랜치를 버전으로 보낸다', async () => {
        const { result, setModal } = await open(steamServer);
        expect(result.current.availableVersions).toHaveLength(3);
        expect(result.current.settingsValues.server_version).toBe('unstable');
        expect(window.api.instanceGetInstalledVersion).not.toHaveBeenCalled();
        await act(async () => result.current.handleInstallVersion());
        await act(async () => setModal.mock.calls.at(-1)![0].buttons[0].action());
        expect(window.api.moduleInstallServer).toHaveBeenCalledWith(
            'pz',
            expect.objectContaining({ version: 'unstable', instance_id: 'z1', background: true }),
        );
    });

    it('적어 둔 브랜치가 없으면 모듈의 기본 브랜치를 고른 채로 연다', async () => {
        const { result } = await open({ ...steamServer, extension_data: { install_method_steamcmd: true } });
        expect(result.current.settingsValues.server_version).toBe('public');
    });
});
