/**
 * Discord 사이드 패널 모드 전환 테스트.
 *
 * - 윈도우 크기가 임계값 이상이면 사이드 패널 모드로 렌더링
 * - 작으면 기존 팝업 모드 유지
 */

import { act, render, screen, waitFor } from '@testing-library/react';
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import App from '../App';
import { SIDE_PANEL_MIN_HEIGHT, SIDE_PANEL_MIN_WIDTH } from '../hooks/useWindowSize';
import { useDiscordStore } from '../stores/useDiscordStore';
import { useServerStore } from '../stores/useServerStore';
import { useSettingsStore } from '../stores/useSettingsStore';
import { useUIStore } from '../stores/useUIStore';

// ── shared mock factory ─────────────────────────────────────
function createApiMock(overrides = {}) {
    const base = {
        settingsLoad: vi.fn().mockResolvedValue({
            autoRefresh: false,
            refreshInterval: 2000,
            ipcPort: 57474,
            consoleBufferSize: 2000,
            discordToken: '',
            discordAutoStart: false,
        }),
        settingsSave: vi.fn().mockResolvedValue({ success: true }),
        settingsGetPath: vi.fn().mockResolvedValue('C:\\test\\settings.json'),
        botConfigLoad: vi.fn().mockResolvedValue({
            prefix: '!saba',
            mode: 'local',
            cloud: { relayUrl: '', hostId: '' },
            moduleAliases: {},
            commandAliases: {},
            musicEnabled: false,
        }),
        botConfigSave: vi.fn().mockResolvedValue({ success: true }),
        discordBotStatus: vi.fn().mockResolvedValue('stopped'),
        discordBotStart: vi.fn().mockResolvedValue({ success: true }),
        discordBotStop: vi.fn().mockResolvedValue({ success: true }),
        daemonStatus: vi.fn().mockResolvedValue({ running: true }),
        moduleList: vi.fn().mockResolvedValue({ modules: [] }),
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
        onConsolePopoutOpened: vi.fn(),
        onConsolePopoutClosed: vi.fn(),
        offConsolePopoutOpened: vi.fn(),
        offConsolePopoutClosed: vi.fn(),
        loadNodeToken: vi.fn().mockResolvedValue(null),
        saveNodeToken: vi.fn().mockResolvedValue({ success: true }),
    };

    const merged = { ...base, ...overrides };
    return new Proxy(merged, {
        get(target, prop) {
            if (typeof prop === 'symbol') return target[prop];
            if (!(prop in target)) {
                target[prop] = vi.fn().mockResolvedValue({});
            }
            return target[prop];
        },
    });
}

// ── lifecycle helpers ───────────────────────────────────────
beforeEach(() => {
    vi.clearAllMocks();
    vi.useRealTimers();
    window.showToast = vi.fn();
    window.showStatus = vi.fn();
    useUIStore.getState()._resetForTest();
    useSettingsStore.getState()._resetForTest();
    useDiscordStore.getState()._resetForTest();
    useServerStore.getState()._resetForTest();
});

afterEach(() => {
    vi.restoreAllMocks();
});

// ════════════════════════════════════════════════════════════
// Discord 사이드 패널 모드 전환
// ════════════════════════════════════════════════════════════
describe('Discord 사이드 패널 모드', () => {
    it('큰 윈도우에서 Discord 열면 side-panel 클래스로 렌더링', async () => {
        // 큰 화면 시뮬레이션
        Object.defineProperty(window, 'innerWidth', { value: SIDE_PANEL_MIN_WIDTH + 100, writable: true });
        Object.defineProperty(window, 'innerHeight', { value: SIDE_PANEL_MIN_HEIGHT + 100, writable: true });

        window.api = createApiMock();

        await act(async () => {
            render(<App />);
        });

        // 앱 부팅 대기
        await waitFor(() => {
            expect(window.api.settingsLoad).toHaveBeenCalled();
        }, { timeout: 3000 });

        // Discord 섹션 열기
        act(() => {
            useUIStore.getState().setShowDiscordSection(true);
        });

        // side-panel 클래스가 적용된 컨테이너가 존재해야 함
        await waitFor(() => {
            const sidePanel = document.querySelector('.discord-modal-container.side-panel');
            expect(sidePanel).not.toBeNull();
        });

        // app-body 내에 side-panel이 main 옆에 위치 확인
        const appBody = document.querySelector('.app-body');
        expect(appBody).not.toBeNull();
        const sidePanel = appBody.querySelector('.discord-modal-container.side-panel');
        expect(sidePanel).not.toBeNull();

        // 말풍선 팝업 형태가 아닌 사이드 패널 확인
        // backdrop이 없어야 함
        const backdrop = document.querySelector('.discord-backdrop');
        expect(backdrop).toBeNull();
    }, 15000);

    it('작은 윈도우에서 Discord 열면 기존 팝업 모달로 렌더링', async () => {
        // 작은 화면 시뮬레이션
        Object.defineProperty(window, 'innerWidth', { value: 1200, writable: true });
        Object.defineProperty(window, 'innerHeight', { value: 800, writable: true });

        window.api = createApiMock();

        await act(async () => {
            render(<App />);
        });

        await waitFor(() => {
            expect(window.api.settingsLoad).toHaveBeenCalled();
        }, { timeout: 3000 });

        // Discord 섹션 열기
        act(() => {
            useUIStore.getState().setShowDiscordSection(true);
        });

        // 기존 팝업 컨테이너 (side-panel 아닌)
        await waitFor(() => {
            const popup = document.querySelector('.discord-modal-container');
            expect(popup).not.toBeNull();
            expect(popup.classList.contains('side-panel')).toBe(false);
        });

        // backdrop 존재 확인
        const backdrop = document.querySelector('.discord-backdrop');
        expect(backdrop).not.toBeNull();
    }, 15000);

    it('윈도우 리사이즈 시 모드 전환', async () => {
        // 처음에 작은 화면
        Object.defineProperty(window, 'innerWidth', { value: 1200, writable: true });
        Object.defineProperty(window, 'innerHeight', { value: 800, writable: true });

        window.api = createApiMock();

        await act(async () => {
            render(<App />);
        });

        await waitFor(() => {
            expect(window.api.settingsLoad).toHaveBeenCalled();
        }, { timeout: 3000 });

        // Discord 열기
        act(() => {
            useUIStore.getState().setShowDiscordSection(true);
        });

        // 팝업 모드 확인
        await waitFor(() => {
            const popup = document.querySelector('.discord-modal-container');
            expect(popup).not.toBeNull();
            expect(popup.classList.contains('side-panel')).toBe(false);
        });

        // 윈도우 크기 확대
        act(() => {
            Object.defineProperty(window, 'innerWidth', { value: SIDE_PANEL_MIN_WIDTH + 100, writable: true });
            Object.defineProperty(window, 'innerHeight', { value: SIDE_PANEL_MIN_HEIGHT + 100, writable: true });
            window.dispatchEvent(new Event('resize'));
        });

        // 사이드 패널로 전환 확인
        await waitFor(() => {
            const sidePanel = document.querySelector('.discord-modal-container.side-panel');
            expect(sidePanel).not.toBeNull();
        });
    }, 15000);

    it('높이가 부족하면 큰 너비에서도 팝업 모드 유지', async () => {
        Object.defineProperty(window, 'innerWidth', { value: SIDE_PANEL_MIN_WIDTH + 200, writable: true });
        Object.defineProperty(window, 'innerHeight', { value: SIDE_PANEL_MIN_HEIGHT - 100, writable: true });

        window.api = createApiMock();

        await act(async () => {
            render(<App />);
        });

        await waitFor(() => {
            expect(window.api.settingsLoad).toHaveBeenCalled();
        }, { timeout: 3000 });

        act(() => {
            useUIStore.getState().setShowDiscordSection(true);
        });

        await waitFor(() => {
            const popup = document.querySelector('.discord-modal-container');
            expect(popup).not.toBeNull();
            expect(popup.classList.contains('side-panel')).toBe(false);
        });
    }, 15000);
});
