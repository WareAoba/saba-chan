import { act, render, renderHook } from '@testing-library/react';
import { beforeEach, describe, expect, it, vi } from 'vitest';
import { ServerSettingsModal } from '../components/ServerSettingsModal';
import { useServerSettings } from '../hooks/useServerSettings';

const modules = [
    {
        name: 'dl',
        install: { method: 'download' },
        settings: { fields: [{ name: 'motd', field_type: 'text', group: 'saba-chan' }] },
    },
];
const server = (status) => ({ id: 's1', name: 'S', module: 'dl', status, module_settings: { working_dir: 'C:/s' } });

function setupHook(servers) {
    const setModal = vi.fn();
    const hook = renderHook(() =>
        useServerSettings({ servers, modules, setModal, setProgressBar: vi.fn(), fetchServers: vi.fn() }),
    );
    return { ...hook, setModal };
}

const toasts = (type) => window.showToast.mock.calls.filter((c) => c[1] === type).map((c) => c[0]);

async function open(result, srv) {
    await act(async () => result.current.handleOpenSettings(srv));
}

describe('켜져 있는 서버의 버전 설치·초기화', () => {
    beforeEach(() => {
        window.showToast = vi.fn();
        window.api = {
            serverList: vi.fn().mockResolvedValue(null),
            moduleListVersions: vi.fn().mockResolvedValue({ versions: ['1.0'] }),
            instanceGetInstalledVersion: vi.fn().mockResolvedValue({ success: false }),
        };
    });

    it('버전을 설치한 뒤 설정 저장이 실패하면 성공으로 알리지 않는다', async () => {
        window.api.moduleInstallServer = vi.fn().mockResolvedValue({ success: true, jar_path: 'C:/s/server.jar' });
        window.api.instanceUpdateSettings = vi.fn().mockResolvedValue({ success: false, error: 'disk full' });
        const { result, setModal } = setupHook([server('stopped')]);
        await open(result, server('stopped'));
        act(() => result.current.handleSettingChange('server_version', '1.0'));
        await act(async () => result.current.handleInstallVersion());
        await act(async () => setModal.mock.calls.at(-1)[0].buttons[0].action());
        expect(toasts('success')).toEqual([]);
        expect(toasts('error')).toHaveLength(1);
        expect(toasts('error')[0]).toContain('saving the settings failed');
    });

    it('데몬이 켜져 있어 설치를 거절하면(server_running) 그 이유를 알린다', async () => {
        window.api.moduleInstallServer = vi
            .fn()
            .mockResolvedValue({ success: false, error: 'Server is running', error_code: 'server_running' });
        const { result, setModal } = setupHook([server('stopped')]);
        await open(result, server('stopped'));
        act(() => result.current.handleSettingChange('server_version', '1.0'));
        await act(async () => result.current.handleInstallVersion());
        await act(async () => setModal.mock.calls.at(-1)[0].buttons[0].action());
        expect(toasts('error')).toEqual(['Stop the server before installing a version']);
    });

    it('모듈이 초기화를 {success:false}로 끝내면 성공으로 알리지 않는다', async () => {
        window.api.instanceResetServer = vi.fn().mockResolvedValue({ success: false, message: 'locked file' });
        const { result, setModal } = setupHook([server('stopped')]);
        await open(result, server('stopped'));
        await act(async () => result.current.handleResetServer());
        await act(async () => setModal.mock.calls.at(-1)[0].buttons[1].action());
        expect(toasts('success')).toEqual([]);
        expect(toasts('error')).toHaveLength(1);
        expect(toasts('error')[0]).toContain('Reset failed');
    });

    it('켜져 있거나 백업 중이라 초기화를 거절하면 그 이유를 알린다', async () => {
        window.api.instanceResetServer = vi
            .fn()
            .mockResolvedValueOnce({ success: false, error: 'running', error_code: 'server_running' })
            .mockResolvedValueOnce({ success: false, error: 'busy', error_code: 'instance_busy' });
        const { result, setModal } = setupHook([server('stopped')]);
        await open(result, server('stopped'));
        for (let i = 0; i < 2; i++) {
            await act(async () => result.current.handleResetServer());
            await act(async () => setModal.mock.calls.at(-1)[0].buttons[1].action());
        }
        expect(toasts('error')).toEqual([
            'Reset failed: Stop the server before resetting it',
            'Reset failed: Another task (backup or restore) is running on this server. Try again when it finishes',
        ]);
    });
});

describe('설정 창: 켜져 있는 서버', () => {
    const props = (status) => ({
        settingsServer: server('stopped'),
        settingsValues: { server_version: '1.0' },
        settingsSource: 'live',
        settingsActiveTab: 'general',
        setSettingsActiveTab: vi.fn(),
        modules,
        advancedExpanded: true,
        setAdvancedExpanded: vi.fn(),
        availableVersions: ['1.0'],
        versionsLoading: false,
        versionInstalling: false,
        handleSettingChange: vi.fn(),
        handleInstallVersion: vi.fn(),
        handleSaveSettings: vi.fn(),
        handleResetServer: vi.fn(),
        resettingServer: false,
        onClose: vi.fn(),
        // 창을 연 뒤에 켜졌다: 지금 상태는 servers 목록이 안다
        servers: [server(status)],
    });

    it('켜져 있으면 버전 설치와 초기화 버튼을 막는다', () => {
        render(<ServerSettingsModal {...props('running')} />);
        expect(document.querySelector('.version-install-btn').disabled).toBe(true);
        expect(document.querySelector('.danger-zone-btn').disabled).toBe(true);
    });

    it('꺼져 있으면 두 버튼을 쓸 수 있다', () => {
        render(<ServerSettingsModal {...props('stopped')} />);
        expect(document.querySelector('.version-install-btn').disabled).toBe(false);
        expect(document.querySelector('.danger-zone-btn').disabled).toBe(false);
    });
});
