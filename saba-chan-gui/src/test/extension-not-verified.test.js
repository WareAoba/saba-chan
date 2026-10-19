import { act, renderHook } from '@testing-library/react';
import { beforeEach, describe, expect, it, vi } from 'vitest';
import { useServerActions } from '../hooks/useServerActions';
import { useServerSettings } from '../hooks/useServerSettings';

const notVerified = {
    success: false,
    error: 'extension_not_verified',
    error_code: 'extension_not_verified',
    unverified_extensions: ['ue4-ini', 'steamcmd'],
};

function setup() {
    const setModal = vi.fn();
    const { result } = renderHook(() =>
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
            openSettingsToExtensions: vi.fn(),
        }),
    );
    return { result, setModal };
}

/** 사용자에게 보인 글 (모달 메시지와 토스트) */
const shown = (setModal) =>
    [...setModal.mock.calls.map(([m]) => m?.message || ''), ...window.showToast.mock.calls.map(([msg]) => msg)].join(
        '\n',
    );

describe('익스텐션 파일이 바뀌어 확인되지 않았을 때', () => {
    beforeEach(() => {
        window.api = { serverList: vi.fn().mockResolvedValue({ servers: [] }) };
        window.showToast = vi.fn();
    });

    it('시작: 바뀐 익스텐션 이름과 다시 설치·되돌리기 안내를 보여 준다', async () => {
        window.api.instanceStart = vi.fn().mockResolvedValue(notVerified);
        const { result, setModal } = setup();
        await act(async () => result.current.handleStart('A', 'game'));
        const text = shown(setModal);
        expect(text).toMatch(/ue4-ini, steamcmd/);
        expect(text).toMatch(/changed/i);
        expect(text).toMatch(/reinstall/i);
        // "설치하고 켜라"는 안내로 빠지지 않는다
        expect(text).not.toMatch(/not enabled|Enable them/);
    });

    it('action_required로 와도 같은 안내를 보여 준다', async () => {
        window.api.instanceStart = vi
            .fn()
            .mockResolvedValue({ ...notVerified, error_code: undefined, action_required: 'extension_not_verified' });
        const { result, setModal } = setup();
        await act(async () => result.current.handleStart('A', 'game'));
        expect(shown(setModal)).toMatch(/ue4-ini, steamcmd/);
    });

    it('정지: message가 없어도 바뀐 익스텐션을 알린다', async () => {
        window.api.instanceStop = vi.fn().mockResolvedValue(notVerified);
        const { result, setModal } = setup();
        await act(async () => result.current.handleStop('A'));
        await act(async () => setModal.mock.calls.at(-1)[0].onConfirm());
        expect(shown(setModal)).toMatch(/ue4-ini, steamcmd/);
    });

    it('서버 만들기: extension_required 안내 대신 확인되지 않았다고 알린다', async () => {
        window.api.instanceCreate = vi.fn().mockResolvedValue(notVerified);
        const { result, setModal } = setup();
        let returned;
        await act(async () => {
            returned = await result.current.handleAddServer({ name: 'B', module_name: 'game' });
        });
        // 실패로 알려야 만들기 창이 입력한 이름·게임을 지우지 않는다
        expect(returned).toBe(false);
        const text = shown(setModal);
        expect(text).toMatch(/ue4-ini, steamcmd/);
        expect(text).not.toMatch(/requires the following extension/);
    });
});

describe('서버 설정 창에서 익스텐션이 확인되지 않았을 때', () => {
    const modules = [
        {
            name: 'game',
            install: { method: 'download' },
            settings: { fields: [{ name: 'motd', field_type: 'text' }] },
        },
    ];
    const srv = { id: 'a', name: 'A', module: 'game', status: 'stopped', module_settings: { working_dir: 'C:/a' } };
    const open = async () => {
        const setModal = vi.fn();
        const hook = renderHook(() =>
            useServerSettings({ servers: [srv], modules, setModal, setProgressBar: vi.fn(), fetchServers: vi.fn() }),
        );
        await act(async () => hook.result.current.handleOpenSettings(srv));
        return { ...hook, setModal };
    };

    beforeEach(() => {
        window.showToast = vi.fn();
        window.api = {
            serverList: vi.fn().mockResolvedValue(null),
            moduleListVersions: vi.fn().mockResolvedValue({ versions: ['1.0'] }),
            instanceGetInstalledVersion: vi.fn().mockResolvedValue({ success: false }),
            instanceReadLiveSettings: vi.fn().mockResolvedValue({ success: true, settings: {} }),
        };
    });

    it('실제 설정 읽기가 막히면 바뀐 익스텐션을 알린다', async () => {
        window.api.instanceReadLiveSettings = vi
            .fn()
            .mockResolvedValue({ ...notVerified, missing_extensions: ['ue4-ini'] });
        const { setModal } = await open();
        expect(shown(setModal)).toMatch(/ue4-ini, steamcmd/);
    });

    it('버전 설치가 막히면 "설치하고 켜라" 대신 바뀐 익스텐션을 알린다', async () => {
        window.api.moduleInstallServer = vi.fn().mockResolvedValue({ ...notVerified, missing_extensions: ['ue4-ini'] });
        const { result, setModal } = await open();
        act(() => result.current.handleSettingChange('server_version', '1.0'));
        await act(async () => result.current.handleInstallVersion());
        await act(async () => setModal.mock.calls.at(-1)[0].buttons[0].action());
        const text = shown(setModal);
        expect(text).toMatch(/ue4-ini, steamcmd/);
        expect(text).not.toMatch(/Required extension/);
    });
});
