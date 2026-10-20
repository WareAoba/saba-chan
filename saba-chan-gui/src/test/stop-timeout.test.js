import { act, render, renderHook } from '@testing-library/react';
import { beforeEach, describe, expect, it, vi } from 'vitest';
import { createApi } from '../api/createApi';
import { ServerSettingsModal } from '../components/ServerSettingsModal';
import { useServerSettings } from '../hooks/useServerSettings';

const modules = [{ name: 'game', settings: { fields: [{ name: 'title', field_type: 'text' }] } }];

const setup = () =>
    renderHook(() =>
        useServerSettings({ servers: [], modules, setModal: vi.fn(), setProgressBar: vi.fn(), fetchServers: vi.fn() }),
    );

beforeEach(() => {
    window.showToast = vi.fn();
    window.api = {
        serverList: vi.fn().mockResolvedValue(null),
        instanceReadLiveSettings: vi.fn().mockResolvedValue({ success: true, settings: { title: 'x' } }),
        instanceUpdateSettings: vi.fn().mockResolvedValue({ success: true }),
    };
});

// 데몬 MK14: 인스턴스 module_settings.stop_timeout(초)이 모듈 값보다 앞선다
describe('정지 대기 시간 칸', () => {
    it('저장된 값으로 채우고, 바꾸지 않으면 보내지 않는다', async () => {
        const { result } = setup();
        await act(async () =>
            result.current.handleOpenSettings({
                id: 's',
                name: 'S',
                module: 'game',
                module_settings: { stop_timeout: 300 },
            }),
        );
        expect(result.current.settingsValues.stop_timeout).toBe('300');
        await act(async () => result.current.handleSaveSettings());
        expect(window.api.instanceUpdateSettings.mock.calls[0][1]).not.toHaveProperty('stop_timeout');
    });

    it('바꾸면 그 값을, 비우면 null(모듈 값으로)을 보낸다', async () => {
        const { result } = setup();
        await act(async () => result.current.handleOpenSettings({ id: 's', name: 'S', module: 'game' }));
        expect(result.current.settingsValues.stop_timeout).toBe('');
        act(() => result.current.handleSettingChange('stop_timeout', ' 600 '));
        await act(async () => result.current.handleSaveSettings());
        expect(window.api.instanceUpdateSettings.mock.calls[0][1]).toMatchObject({ stop_timeout: '600' });

        await act(async () =>
            result.current.handleOpenSettings({
                id: 's',
                name: 'S',
                module: 'game',
                module_settings: { stop_timeout: 120 },
            }),
        );
        act(() => result.current.handleSettingChange('stop_timeout', ''));
        await act(async () => result.current.handleSaveSettings());
        expect(window.api.instanceUpdateSettings.mock.calls[1][1]).toMatchObject({ stop_timeout: null });
    });
});

// 데몬은 정지 요청에 stop_timeout(최대 3600초)까지 기다린 뒤 답한다: 요청 시간 제한이 그보다 길어야 한다 (L74)
describe('정지·재시작 요청 시간 제한', () => {
    const longest = 3600 * 1000;
    const timeoutOf = async (call) => {
        const invoke = vi.fn().mockResolvedValue({ status: 200, data: { success: true } });
        const { api } = createApi({ invoke, hub: { dispatch: vi.fn() } });
        await call(api);
        return invoke.mock.calls.find(([cmd]) => cmd === 'daemon_request')[1].timeoutMs;
    };

    it('id로 정지·재시작, 이름으로 정지 모두 가장 긴 정지 대기보다 오래 기다린다', async () => {
        expect(await timeoutOf((api) => api.instanceStop('s'))).toBeGreaterThan(longest);
        expect(await timeoutOf((api) => api.instanceRestart('s'))).toBeGreaterThan(longest);
        expect(await timeoutOf((api) => api.serverStop('S'))).toBeGreaterThan(longest);
    });
});

// 데몬이 직접 띄우지 않는 서버(managed:false)는 모듈 stop()이 정지를 맡아 stop_timeout을 쓰지 않는다
describe('설정 창의 정지 대기 칸', () => {
    const props = (managed) => ({
        settingsServer: { id: 's', name: 'S', module: 'game', status: 'stopped', managed },
        settingsValues: {},
        settingsSource: 'live',
        settingsActiveTab: 'general',
        setSettingsActiveTab: vi.fn(),
        modules,
        advancedExpanded: false,
        setAdvancedExpanded: vi.fn(),
        availableVersions: [],
        versionsLoading: false,
        versionInstalling: false,
        handleSettingChange: vi.fn(),
        handleInstallVersion: vi.fn(),
        handleSaveSettings: vi.fn(),
        handleResetServer: vi.fn(),
        resettingServer: false,
        onClose: vi.fn(),
        servers: [],
    });

    it('데몬이 띄우는 서버에만 보인다', () => {
        const { unmount } = render(<ServerSettingsModal {...props(true)} />);
        expect(document.querySelector('.stop-timeout-section')).not.toBeNull();
        unmount();
        render(<ServerSettingsModal {...props(false)} />);
        expect(document.querySelector('.stop-timeout-section')).toBeNull();
    });
});
