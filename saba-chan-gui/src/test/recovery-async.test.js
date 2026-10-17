import React from 'react';
import { act, renderHook } from '@testing-library/react';
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import { useMultiConsole } from '../hooks/useMultiConsole';
import { useServerSettings } from '../hooks/useServerSettings';
import { useDiscordStore } from '../stores/useDiscordStore';
import { useServerStore } from '../stores/useServerStore';

function deferred() {
    let resolve;
    const promise = new Promise((done) => { resolve = done; });
    return { promise, resolve };
}

beforeEach(() => {
    vi.useFakeTimers();
    useDiscordStore.getState()._resetForTest();
    useServerStore.getState()._resetForTest();
    window.api = {
        relayCheckHostStatus: vi.fn().mockResolvedValue({ error: 'offline' }),
        managedConsole: vi.fn().mockResolvedValue({ lines: [] }),
    };
});

afterEach(() => {
    useDiscordStore.getState().stopStatusPolling();
    vi.clearAllTimers();
    vi.useRealTimers();
    vi.restoreAllMocks();
});

describe('relay status request lifecycle', () => {
    it('ignores an old host response after the pairing target changes', async () => {
        const oldHost = deferred();
        window.api.relayCheckHostStatus.mockReturnValueOnce(oldHost.promise);
        useDiscordStore.setState({ discordCloudHostId: 'old-host' });
        const oldCheck = useDiscordStore.getState().checkStatus();

        useDiscordStore.getState().update({ discordCloudHostId: 'new-host' });
        await Promise.resolve();
        oldHost.resolve({ connected: true });
        await oldCheck;

        expect(useDiscordStore.getState().relayConnected).toBe(false);
        expect(useDiscordStore.getState().relayStatusChecked).toBe(true);
    });

    it('does not start a delayed initial check after polling stops', async () => {
        useDiscordStore.setState({ discordCloudHostId: 'host' });
        useDiscordStore.getState().startStatusPolling();
        useDiscordStore.getState().stopStatusPolling();
        await vi.advanceTimersByTimeAsync(500);
        expect(window.api.relayCheckHostStatus).not.toHaveBeenCalled();
    });
});

describe('bot config load guard', () => {
    it('preserves the stored prefix when saving before initialization finishes', async () => {
        window.api.botConfigLoad = vi.fn().mockResolvedValue({ prefix: '!custom', cloud: { hostId: 'host' } });
        window.api.botConfigSave = vi.fn().mockResolvedValue({ success: true });
        await useDiscordStore.getState().saveConfig();
        expect(window.api.botConfigSave.mock.calls[0][0].prefix).toBe('!custom');
    });

    it('does not overwrite config with defaults when its initial load is unavailable', async () => {
        window.api.botConfigLoad = vi.fn().mockResolvedValue(null);
        window.api.botConfigSave = vi.fn();
        await useDiscordStore.getState().saveConfig();
        expect(window.api.botConfigSave).not.toHaveBeenCalled();
    });
});

describe('console polling lifecycle', () => {
    const options = { isPopoutMode: true, popoutParams: null, consoleBufferRef: { current: 2000 } };

    it('ignores a closed console response after the same console is reopened', async () => {
        const oldPoll = deferred();
        window.api.managedConsole.mockReturnValueOnce(oldPoll.promise);
        useServerStore.setState({ servers: [{ id: 'server', pid: 42 }] });
        const { result, unmount } = renderHook(() => useMultiConsole(options));

        act(() => result.current.openConsole('server', 'Server'));
        await act(async () => { await vi.advanceTimersByTimeAsync(500); });
        act(() => {
            result.current.closeConsole('server');
            result.current.openConsole('server', 'Server');
        });
        await act(async () => {
            oldPoll.resolve({ lines: [{ id: 1, content: 'old session' }] });
            await oldPoll.promise;
        });

        expect(result.current.consoles.server.lines).toEqual([]);
        await act(async () => { await vi.advanceTimersByTimeAsync(500); });
        expect(window.api.managedConsole).toHaveBeenCalledTimes(2);
        unmount();
    });

    it('restarts popout polling after the StrictMode effect cleanup', async () => {
        useServerStore.setState({ servers: [{ id: 'server', pid: 42 }] });
        const popoutParams = { instanceId: 'server', name: 'Server' };
        const { unmount } = renderHook(
            () => useMultiConsole({ ...options, popoutParams }),
            { wrapper: React.StrictMode },
        );
        await act(async () => { await vi.advanceTimersByTimeAsync(500); });
        expect(window.api.managedConsole).toHaveBeenCalledTimes(1);
        unmount();
    });
});

describe('server settings request lifecycle', () => {
    it.each(['install', 'reset'])('ignores an older %s result after switching servers', async (operation) => {
        const finished = deferred();
        const first = { id: 'first', name: 'First', module: 'game', working_dir: '/synthetic' };
        const second = { id: 'second', name: 'Second', module: 'game' };
        window.api.serverList = vi.fn().mockResolvedValue({ servers: [first, second] });
        window.api.instanceReadLiveSettings = vi.fn().mockResolvedValue({ success: true, settings: { title: 'Original' } });
        window.api.instanceUpdateSettings = vi.fn().mockResolvedValue({ success: true });
        window.api.instanceResetServer = vi.fn().mockReturnValue(finished.promise);
        window.api.moduleInstallServer = vi.fn().mockReturnValue(finished.promise);
        const setModal = vi.fn();
        const { result } = renderHook(() => useServerSettings({
            servers: [first, second], modules: [{ name: 'game', settings: { fields: [{ name: 'title', field_type: 'text' }] } }],
            setModal, setProgressBar: vi.fn(), fetchServers: vi.fn(),
        }));
        await act(async () => { await result.current.handleOpenSettings(first); });
        act(() => result.current.handleSettingChange('server_version', '1.0'));
        await act(async () => {
            if (operation === 'install') await result.current.handleInstallVersion();
            else await result.current.handleResetServer();
        });
        const confirm = setModal.mock.calls.at(-1)[0].buttons[operation === 'install' ? 0 : 1].action;
        let oldOperation;
        act(() => { oldOperation = confirm(); });
        await act(async () => { await result.current.handleOpenSettings(second); });
        act(() => result.current.handleSettingChange('title', 'Unsaved second edit'));
        await act(async () => {
            finished.resolve({ success: true, jar_path: '/synthetic/server.jar', install_path: '/synthetic' });
            await oldOperation;
        });
        expect(result.current.settingsServer.id).toBe('second');
        expect(result.current.settingsValues.title).toBe('Unsaved second edit');
        expect(result.current.settingsValues.server_version).toBeUndefined();
    });

    it('keeps the current server form open when an older save finishes', async () => {
        const saved = deferred();
        const first = { id: 'first', name: 'First', module: 'game' };
        const second = { id: 'second', name: 'Second', module: 'game' };
        window.api.serverList = vi.fn().mockResolvedValue({ servers: [first, second] });
        window.api.instanceReadLiveSettings = vi.fn().mockResolvedValue({ success: true, settings: { title: 'Original' } });
        window.api.instanceUpdateSettings = vi.fn().mockReturnValue(saved.promise);
        const { result } = renderHook(() => useServerSettings({
            servers: [first, second], modules: [{ name: 'game', settings: { fields: [{ name: 'title', field_type: 'text' }] } }],
            setModal: vi.fn(), setProgressBar: vi.fn(), fetchServers: vi.fn(),
        }));
        await act(async () => { await result.current.handleOpenSettings(first); });
        let oldSave;
        act(() => { oldSave = result.current.handleSaveSettings(); });
        await act(async () => { await result.current.handleOpenSettings(second); });
        act(() => result.current.handleSettingChange('title', 'Unsaved second edit'));
        await act(async () => { saved.resolve({ success: true }); await oldSave; });
        expect(result.current.showSettingsModal).toBe(true);
        expect(result.current.settingsServer.id).toBe('second');
        expect(result.current.settingsValues.title).toBe('Unsaved second edit');
    });

    it('ignores live settings from a previously selected server', async () => {
        const oldLive = deferred();
        const first = { id: 'first', name: 'First', module: 'game' };
        const second = { id: 'second', name: 'Second', module: 'game' };
        window.api.serverList = vi.fn().mockResolvedValue({ servers: [first, second] });
        window.api.instanceReadLiveSettings = vi.fn()
            .mockReturnValueOnce(oldLive.promise)
            .mockResolvedValue({ success: true, settings: { title: 'Second title' } });
        const { result } = renderHook(() => useServerSettings({
            servers: [first, second],
            modules: [{ name: 'game', settings: { fields: [{ name: 'title', default: 'Default' }] } }],
            setModal: vi.fn(), setProgressBar: vi.fn(), fetchServers: vi.fn(),
        }));

        let firstOpen;
        await act(async () => {
            firstOpen = result.current.handleOpenSettings(first);
            await Promise.resolve();
        });
        await act(async () => { await result.current.handleOpenSettings(second); });
        await act(async () => {
            oldLive.resolve({ success: true, settings: { title: 'First title' } });
            await firstOpen;
        });

        expect(result.current.settingsServer.id).toBe('second');
        expect(result.current.settingsValues.title).toBe('Second title');
    });

    it('does not write password values to the console while loading, editing, or saving', async () => {
        const log = vi.spyOn(console, 'log').mockImplementation(() => {});
        const server = { id: 'server', module: 'game', name: 'Server' };
        window.api.serverList = vi.fn().mockResolvedValue({ servers: [server] });
        window.api.instanceReadLiveSettings = vi.fn().mockResolvedValue({
            success: true, settings: { admin_password: 'fake-password-for-regression' },
        });
        window.api.instanceUpdateSettings = vi.fn().mockResolvedValue({ success: true });
        const { result } = renderHook(() => useServerSettings({
            servers: [server], modules: [{ name: 'game', settings: { fields: [{ name: 'admin_password', field_type: 'text' }] } }],
            setModal: vi.fn(), setProgressBar: vi.fn(), fetchServers: vi.fn(),
        }));
        await act(async () => { await result.current.handleOpenSettings(server); });
        act(() => result.current.handleSettingChange('admin_password', 'fake-password-for-regression'));
        await act(async () => { await result.current.handleSaveSettings(); });
        expect(window.api.instanceUpdateSettings).toHaveBeenCalled();
        expect(JSON.stringify(log.mock.calls)).not.toContain('fake-password-for-regression');
        log.mockRestore();
    });
});
