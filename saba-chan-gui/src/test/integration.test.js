import React from 'react';
import { render, waitFor, act } from '@testing-library/react';
import { describe, it, expect, beforeEach, vi } from 'vitest';
import App from '../App';

function createApiMock(overrides = {}) {
    const base = {
        settingsLoad: vi.fn().mockResolvedValue({
            autoRefresh: true,
            refreshInterval: 2000,
            ipcPort: 57474,
            consoleBufferSize: 2000,
            modulesPath: '',
            discordToken: '',
            discordAutoStart: false,
        }),
        settingsSave: vi.fn().mockResolvedValue({ success: true }),
        settingsGetPath: vi.fn().mockResolvedValue('C:/tmp/settings.json'),
        botConfigLoad: vi.fn().mockResolvedValue({
            prefix: '!saba',
            moduleAliases: {},
            commandAliases: {},
        }),
        botConfigSave: vi.fn().mockResolvedValue({ success: true }),
        discordBotStatus: vi.fn().mockResolvedValue('stopped'),
        discordBotStart: vi.fn().mockResolvedValue({ success: true }),
        discordBotStop: vi.fn().mockResolvedValue({ success: true }),
        daemonStatus: vi.fn().mockResolvedValue({ running: true }),
        moduleList: vi.fn().mockResolvedValue({ modules: [{ name: 'palworld' }] }),
        moduleGetLocales: vi.fn().mockResolvedValue({}),
        moduleGetMetadata: vi.fn().mockResolvedValue({ toml: { aliases: {}, commands: { fields: [] } } }),
        serverList: vi.fn().mockResolvedValue({ servers: [] }),
        onStatusUpdate: vi.fn(),
        onUpdatesAvailable: vi.fn(),
        onUpdateCompleted: vi.fn(),
        onCloseRequest: vi.fn(),
        offCloseRequest: vi.fn(),
        onBotRelaunch: vi.fn(),
        offBotRelaunch: vi.fn(),
    };

    const merged = { ...base, ...overrides };
    return new Proxy(merged, {
        get(target, prop) {
            if (!(prop in target)) {
                target[prop] = vi.fn().mockResolvedValue({});
            }
            return target[prop];
        },
    });
}

describe('GUI Integration E2E', () => {
    beforeEach(() => {
        vi.clearAllMocks();
        window.showToast = vi.fn();
        window.showStatus = vi.fn();
    });

    it('앱 부팅 시 핵심 API 플로우를 끝까지 호출해야 함', async () => {
        window.api = createApiMock();

        await act(async () => {
            render(<App />);
        });

        await waitFor(() => {
            expect(window.api.settingsLoad).toHaveBeenCalled();
            expect(window.api.settingsGetPath).toHaveBeenCalled();
            expect(window.api.botConfigLoad).toHaveBeenCalled();
            expect(window.api.moduleList).toHaveBeenCalled();
            expect(window.api.serverList).toHaveBeenCalled();
            expect(window.api.daemonStatus).toHaveBeenCalled();
            expect(window.api.discordBotStatus).toHaveBeenCalled();
        }, { timeout: 6000 });
    });

    it('모듈 조회 실패 시 백오프 재시도 후 성공해야 함', async () => {
        const moduleList = vi.fn()
            .mockRejectedValueOnce(new Error('temporary fail 1'))
            .mockRejectedValueOnce(new Error('temporary fail 2'))
            .mockResolvedValue({ modules: [{ name: 'palworld' }] });

        window.api = createApiMock({ moduleList });

        await act(async () => {
            render(<App />);
        });

        await waitFor(() => {
            expect(window.api.moduleList).toHaveBeenCalledTimes(3);
        }, { timeout: 7000 });
    });
});
