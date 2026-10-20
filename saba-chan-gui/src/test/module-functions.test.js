/**
 * 데몬은 GET /api/modules의 각 모듈에 구현한 lifecycle 함수 목록 `functions`를 싣는다 (MK1).
 * 배열이면 빠진 선택 함수의 버튼을 숨기고, null이면(옛 모듈) 지금처럼 모두 보인다.
 * 그래도 불리면 데몬이 {success:false, error_code:"not_supported"}로 답하니 그 이유를 알린다.
 */
import { act, fireEvent, render, renderHook, screen } from '@testing-library/react';
import { beforeEach, describe, expect, it, vi } from 'vitest';
import { ServerSettingsModal } from '../components/ServerSettingsModal';
import { ServerCheck } from '../components/servers/ServerCheck';
import { useServerSettings } from '../hooks/useServerSettings';
import { useServerStore } from '../stores/useServerStore';

const NOT_SUPPORTED = {
    success: false,
    error_code: 'not_supported',
    function: 'reset_server',
    message: "This module does not implement 'reset_server'",
};

const moduleWith = (functions) => ({
    name: 'dl',
    install: { method: 'download' },
    settings: { fields: [{ name: 'motd', field_type: 'text', group: 'saba-chan' }] },
    functions,
});
const srv = { id: 's1', name: 'S', module: 'dl', status: 'stopped', module_settings: { working_dir: 'C:/s' } };

const modalProps = (modules) => ({
    settingsServer: srv,
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
    servers: [srv],
});

beforeEach(() => {
    window.showToast = vi.fn();
    useServerStore.setState({ modules: [] });
});

describe('설정 창: 모듈이 구현하지 않은 기능', () => {
    it('목록에 reset_server가 없으면 초기화를, list_versions·install_server가 없으면 버전 설치를 숨긴다', () => {
        render(<ServerSettingsModal {...modalProps([moduleWith(['validate'])])} />);
        expect(document.querySelector('.danger-zone-btn')).toBeNull();
        expect(document.querySelector('.version-install-btn')).toBeNull();
    });

    it('목록에 있으면 보이고, 목록을 모르면(null) 모두 보인다', () => {
        const { unmount } = render(
            <ServerSettingsModal {...modalProps([moduleWith(['reset_server', 'list_versions', 'install_server'])])} />,
        );
        expect(document.querySelector('.danger-zone-btn')).not.toBeNull();
        expect(document.querySelector('.version-install-btn')).not.toBeNull();
        unmount();
        render(<ServerSettingsModal {...modalProps([moduleWith(null)])} />);
        expect(document.querySelector('.danger-zone-btn')).not.toBeNull();
        expect(document.querySelector('.version-install-btn')).not.toBeNull();
    });
});

describe('설정 열기: 모듈이 구현하지 않은 버전 조회', () => {
    it('list_versions·get_installed_version이 없으면 부르지 않는다', async () => {
        window.api = {
            serverList: vi.fn().mockResolvedValue(null),
            moduleListVersions: vi.fn().mockResolvedValue({ versions: ['1.0'] }),
            instanceGetInstalledVersion: vi.fn().mockResolvedValue({ success: false }),
        };
        const modules = [moduleWith(['validate'])];
        const { result } = renderHook(() =>
            useServerSettings({
                servers: [srv],
                modules,
                setModal: vi.fn(),
                setProgressBar: vi.fn(),
                fetchServers: vi.fn(),
            }),
        );
        await act(async () => result.current.handleOpenSettings(srv));
        expect(window.api.moduleListVersions).not.toHaveBeenCalled();
        expect(window.api.instanceGetInstalledVersion).not.toHaveBeenCalled();
    });

    it('데몬이 not_supported로 초기화를 거절하면 그 이유를 알린다', async () => {
        window.api = {
            serverList: vi.fn().mockResolvedValue(null),
            instanceResetServer: vi.fn().mockResolvedValue(NOT_SUPPORTED),
        };
        const setModal = vi.fn();
        const modules = [moduleWith(null)];
        const { result } = renderHook(() =>
            useServerSettings({ servers: [srv], modules, setModal, setProgressBar: vi.fn(), fetchServers: vi.fn() }),
        );
        await act(async () => result.current.handleOpenSettings(srv));
        await act(async () => result.current.handleResetServer());
        await act(async () => setModal.mock.calls.at(-1)[0].buttons[1].action());
        expect(window.showToast.mock.calls.map((c) => c[0])).toEqual([
            "Reset failed: This server's module doesn't support this feature.",
        ]);
    });
});

describe('서버 점검: 모듈이 구현하지 않은 점검', () => {
    beforeEach(() => {
        window.api = { instanceValidate: vi.fn(), instanceDiagnose: vi.fn() };
    });

    it('목록에 없는 점검 버튼은 숨기고, 둘 다 없으면 점검 칸을 그리지 않는다', () => {
        useServerStore.setState({ modules: [moduleWith(['validate'])] });
        const { unmount } = render(<ServerCheck server={srv} />);
        expect(screen.queryByRole('button', { name: /Check server/i })).not.toBeNull();
        expect(screen.queryByRole('button', { name: /Diagnose log/i })).toBeNull();
        unmount();
        useServerStore.setState({ modules: [moduleWith([])] });
        const { container } = render(<ServerCheck server={srv} />);
        expect(container).toBeEmptyDOMElement();
    });

    it('데몬이 not_supported로 답하면 그 이유를 보인다', async () => {
        window.api.instanceValidate = vi.fn().mockResolvedValue({ ...NOT_SUPPORTED, function: 'validate' });
        render(<ServerCheck server={srv} />);
        await act(async () => fireEvent.click(screen.getByRole('button', { name: /Check server/i })));
        expect(screen.getByRole('alert').textContent).toContain("This server's module doesn't support this feature");
    });
});
