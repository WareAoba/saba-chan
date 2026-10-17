/**
 * 새 화면 골격 테스트 (V1 GUI 리메이크).
 *
 * - 상단 탭으로 서버 · 디스코드 · 설정 페이지를 오간다 (모달이 아니라 페이지)
 * - 서버 페이지는 왼쪽 목록 + 오른쪽 상세, 좁은 창에서는 한 열
 * - 목록 줄의 켜기 버튼은 항상 같은 자리에서 handleStart를 부른다
 * - 서버가 없으면 빈 화면 안내와 "서버 추가" 버튼
 */

import { act, fireEvent, render, waitFor } from '@testing-library/react';
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import App from '../App';
import { SINGLE_COLUMN_MAX_WIDTH } from '../components/servers/ServersPage';
import { useDiscordStore } from '../stores/useDiscordStore';
import { useServerStore } from '../stores/useServerStore';
import { useSettingsStore } from '../stores/useSettingsStore';
import { useUIStore } from '../stores/useUIStore';

Element.prototype.scrollIntoView = vi.fn();

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
        moduleGetMetadata: vi.fn().mockResolvedValue({ metadata: { module_aliases: [], commands: { fields: [] } } }),
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

const twoServers = {
    servers: [
        { id: 'srv-1', name: 'Alpha', module: 'palworld', status: 'running', start_time: Math.floor(Date.now() / 1000) - 60 },
        { id: 'srv-2', name: 'Beta', module: 'palworld', status: 'stopped' },
    ],
};
const palworld = { modules: [{ name: 'palworld', version: '1.0.0', commands: { fields: [] } }] };

async function boot(overrides = {}, width = 1280) {
    Object.defineProperty(window, 'innerWidth', { value: width, writable: true, configurable: true });
    window.api = createApiMock(overrides);
    await act(async () => {
        render(<App />);
    });
    await waitFor(() => expect(document.querySelector('.appbar')).not.toBeNull(), { timeout: 8000 });
}

describe('상단 탭 페이지 전환', () => {
    it('디스코드 · 설정 탭이 같은 창 안의 페이지로 열린다', async () => {
        await boot({ serverList: vi.fn().mockResolvedValue(twoServers), moduleList: vi.fn().mockResolvedValue(palworld) });

        const tabs = document.querySelectorAll('.appbar-tab');
        expect(tabs).toHaveLength(3);

        await act(async () => fireEvent.click(tabs[1]));
        await waitFor(() => expect(document.querySelector('.discord-page')).not.toBeNull());
        // 왼쪽 메뉴 + 오른쪽 화면 (연결 화면이 먼저)
        expect(document.querySelector('.dc-nav')).not.toBeNull();
        expect(document.querySelector('.dc-hero')).not.toBeNull();

        await act(async () => fireEvent.click(tabs[2]));
        await waitFor(() => expect(document.querySelector('.settings-page')).not.toBeNull());
        expect(document.querySelector('.settings-modal-overlay')).toBeNull();
    }, 20000);

    it('openSettings(update) 요청은 설정 페이지로 이동한다', async () => {
        await boot();
        act(() => useUIStore.getState().openSettings('update'));
        await waitFor(() => expect(document.querySelector('.settings-page')).not.toBeNull());
        expect(useUIStore.getState().activeTab).toBe('settings');
    }, 20000);
});

describe('서버 페이지', () => {
    it('목록 + 상세가 함께 보이고 첫 서버가 선택된다', async () => {
        await boot({ serverList: vi.fn().mockResolvedValue(twoServers), moduleList: vi.fn().mockResolvedValue(palworld) });
        await waitFor(() => expect(document.querySelectorAll('.server-row')).toHaveLength(2), { timeout: 8000 });
        await waitFor(() => expect(document.querySelector('.server-detail h1')?.textContent).toBe('Alpha'));
        expect(document.querySelector('.server-row.selected')?.textContent).toContain('Alpha');
    }, 20000);

    it('다른 줄을 누르면 상세가 그 서버로 바뀐다', async () => {
        await boot({ serverList: vi.fn().mockResolvedValue(twoServers), moduleList: vi.fn().mockResolvedValue(palworld) });
        await waitFor(() => expect(document.querySelectorAll('.server-row')).toHaveLength(2), { timeout: 8000 });
        await act(async () => fireEvent.click(document.querySelectorAll('.server-row')[1]));
        await waitFor(() => expect(document.querySelector('.server-detail h1')?.textContent).toBe('Beta'));
    }, 20000);

    it('꺼진 서버 줄의 켜기 버튼은 데몬 시작 API를 부른다', async () => {
        const instanceStart = vi.fn().mockResolvedValue({ success: true });
        await boot({
            serverList: vi.fn().mockResolvedValue(twoServers),
            moduleList: vi.fn().mockResolvedValue(palworld),
            instanceStart,
        });
        await waitFor(() => expect(document.querySelectorAll('.server-row')).toHaveLength(2), { timeout: 8000 });
        const startBtn = document.querySelectorAll('.server-row')[1].querySelector('.server-row-action.action-start');
        expect(startBtn).not.toBeNull();
        await act(async () => fireEvent.click(startBtn));
        await waitFor(() => expect(instanceStart).toHaveBeenCalledWith('srv-2'));
    }, 20000);

    it('이미 켜는 중이면 실패 대신 안내 토스트를 띄우고 켜는 중 표시를 유지한다', async () => {
        const instanceStart = vi.fn().mockResolvedValue({ success: false, error_code: 'start_in_progress' });
        await boot({
            serverList: vi.fn().mockResolvedValue(twoServers),
            moduleList: vi.fn().mockResolvedValue(palworld),
            instanceStart,
        });
        await waitFor(() => expect(document.querySelectorAll('.server-row')).toHaveLength(2), { timeout: 8000 });
        const toast = vi.spyOn(window, 'showToast');
        const startBtn = document.querySelectorAll('.server-row')[1].querySelector('.server-row-action.action-start');
        await act(async () => fireEvent.click(startBtn));
        await waitFor(() => expect(instanceStart).toHaveBeenCalled());
        await waitFor(() => expect(toast).toHaveBeenCalledWith(expect.stringContaining('Beta'), 'info', 3000, undefined));
        expect(toast).not.toHaveBeenCalledWith(expect.any(String), 'error', expect.anything());
    }, 20000);

    it('좁은 창에서는 목록만 보이고, 줄을 누르면 상세와 돌아가기 버튼이 보인다', async () => {
        await boot(
            { serverList: vi.fn().mockResolvedValue(twoServers), moduleList: vi.fn().mockResolvedValue(palworld) },
            SINGLE_COLUMN_MAX_WIDTH - 100,
        );
        await waitFor(() => expect(document.querySelectorAll('.server-row')).toHaveLength(2), { timeout: 8000 });
        expect(document.querySelector('.server-detail')).toBeNull();
        await act(async () => fireEvent.click(document.querySelectorAll('.server-row')[0]));
        await waitFor(() => expect(document.querySelector('.server-detail')).not.toBeNull());
        expect(document.querySelector('.server-list-pane')).toBeNull();
        await act(async () => fireEvent.click(document.querySelector('.detail-back')));
        await waitFor(() => expect(document.querySelector('.server-list-pane')).not.toBeNull());
    }, 20000);

    it('서버가 없으면 빈 화면 안내와 서버 추가 버튼을 보여준다', async () => {
        await boot();
        await waitFor(() => expect(document.querySelector('.empty-state')).not.toBeNull(), { timeout: 8000 });
        await act(async () => fireEvent.click(document.querySelector('.empty-state .btn-primary')));
        expect(useUIStore.getState().showModuleManager).toBe(true);
    }, 20000);

    it('우클릭 메뉴의 설정은 그 서버 상세의 설정 탭을 연다', async () => {
        await boot({ serverList: vi.fn().mockResolvedValue(twoServers), moduleList: vi.fn().mockResolvedValue(palworld) });
        await waitFor(() => expect(document.querySelectorAll('.server-row')).toHaveLength(2), { timeout: 8000 });
        act(() => useUIStore.getState().showServerTab('srv-2', 'settings'));
        await waitFor(() => expect(document.querySelector('.server-detail h1')?.textContent).toBe('Beta'));
        await waitFor(() =>
            expect(document.querySelector('.segmented-item.active')?.textContent).toMatch(/Settings/),
        );
    }, 20000);
});
