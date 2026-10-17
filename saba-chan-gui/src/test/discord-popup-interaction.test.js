/**
 * Discord 팝업 모달 상호작용 회귀 테스트.
 *
 * 이슈 1: 백드롭이 App 최상위(z-index 1999)에 깔리면 헤더 스태킹 컨텍스트(z-index 10)
 *   안에 있는 팝업 모달(z-index 2000)을 덮어버려 스크롤/클릭이 전부 차단된다.
 *   → 백드롭은 반드시 헤더 내부(버튼 래퍼)에 렌더링되어야 모달 아래에 깔린다.
 *
 * 이슈 2: App이 매 렌더마다 새 인라인 setter를 만들어 넘기면 DiscordBotModal의
 *   노드 로드 effect가 매 렌더마다 재발화 → 릴레이 서버에 노드 목록을 무한 재요청.
 *   → setter가 안정화되면 리렌더가 반복돼도 노드 목록은 1회만 로드된다.
 */

import { act, render, waitFor } from '@testing-library/react';
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import App from '../App';
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
        }),
        settingsSave: vi.fn().mockResolvedValue({ success: true }),
        settingsGetPath: vi.fn().mockResolvedValue('C:\\test\\settings.json'),
        botConfigLoad: vi.fn().mockResolvedValue({
            prefix: '!saba',
            cloud: { relayUrl: '', hostId: '' },
            moduleAliases: {},
            commandAliases: {},
        }),
        botConfigSave: vi.fn().mockResolvedValue({ success: true }),
        daemonStatus: vi.fn().mockResolvedValue({ running: true }),
        moduleList: vi.fn().mockResolvedValue({ modules: [] }),
        moduleGetLocales: vi.fn().mockResolvedValue({}),
        moduleGetMetadata: vi.fn().mockResolvedValue({ toml: { aliases: {}, commands: { fields: [] } } }),
        serverList: vi.fn().mockResolvedValue({ servers: [] }),
        relayListHostNodes: vi.fn().mockResolvedValue([]),
        relayGetNodeSettings: vi.fn().mockResolvedValue({ musicEnabled: true, musicUIEnabled: true }),
        onStatusUpdate: vi.fn(),
        onUpdatesAvailable: vi.fn(),
        onUpdateCompleted: vi.fn(),
        onCloseRequest: vi.fn(),
        offCloseRequest: vi.fn(),
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

async function renderAppSmallWindow() {
    // 작은 화면 → 팝업 모드
    Object.defineProperty(window, 'innerWidth', { value: 1200, writable: true });
    Object.defineProperty(window, 'innerHeight', { value: 800, writable: true });

    await act(async () => {
        render(<App />);
    });
    await waitFor(() => {
        expect(window.api.settingsLoad).toHaveBeenCalled();
    }, { timeout: 3000 });
}

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
// 이슈 1: 백드롭 스태킹 — 팝업 모달을 덮으면 안 된다
// ════════════════════════════════════════════════════════════
describe('팝업 백드롭 스태킹', () => {
    it('Discord 백드롭은 헤더 내부에 렌더링되어야 함 (App 직속이면 모달을 덮음)', async () => {
        window.api = createApiMock();
        await renderAppSmallWindow();

        act(() => {
            useUIStore.getState().setShowDiscordSection(true);
        });

        await waitFor(() => {
            expect(document.querySelector('.discord-modal-container')).not.toBeNull();
        });

        // 백드롭은 존재하되, App 직속(헤더 스태킹 컨텍스트 밖)이면 안 된다
        const backdrops = document.querySelectorAll('.discord-backdrop');
        expect(backdrops.length).toBe(1);
        expect(document.querySelector('.App > .discord-backdrop')).toBeNull();
        // 헤더(z-index 10 컨텍스트) 내부 → 모달(z-2000)이 백드롭(z-1999) 위에 온다
        expect(document.querySelector('.app-header .discord-backdrop')).not.toBeNull();
    }, 15000);

    it('Background 백드롭도 헤더 내부에 1개만 렌더링되어야 함', async () => {
        window.api = createApiMock();
        await renderAppSmallWindow();

        act(() => {
            useUIStore.getState().setShowBackgroundSection(true);
        });

        await waitFor(() => {
            expect(document.querySelector('.background-modal-container')).not.toBeNull();
        });

        const backdrops = document.querySelectorAll('.discord-backdrop');
        expect(backdrops.length).toBe(1);
        expect(document.querySelector('.App > .discord-backdrop')).toBeNull();
        expect(document.querySelector('.app-header .discord-backdrop')).not.toBeNull();
    }, 15000);

    it('Notice 백드롭도 헤더 내부에 1개만 렌더링되어야 함', async () => {
        window.api = createApiMock();
        await renderAppSmallWindow();

        act(() => {
            useUIStore.getState().setShowNoticeSection(true);
        });

        await waitFor(() => {
            expect(document.querySelector('.notice-modal-container')).not.toBeNull();
        });

        const backdrops = document.querySelectorAll('.discord-backdrop');
        expect(backdrops.length).toBe(1);
        expect(document.querySelector('.App > .discord-backdrop')).toBeNull();
        expect(document.querySelector('.app-header .discord-backdrop')).not.toBeNull();
    }, 15000);
});

// ════════════════════════════════════════════════════════════
// 이슈 2: 노드 목록 재요청 루프 — 리렌더가 반복돼도 1회만 로드
// ════════════════════════════════════════════════════════════
describe('클라우드 노드 로드 루프 방지', () => {
    it('App 리렌더(uptime 시계 등)가 반복돼도 노드 목록은 1회만 요청', async () => {
        window.api = createApiMock({
            relayListHostNodes: vi.fn().mockResolvedValue([
                { guildId: 'g1', guildName: 'Guild One' },
            ]),
        });
        await renderAppSmallWindow();

        // 릴레이 연결 상태 시뮬레이션 (update()를 거치면 상태가 리셋되므로 setState 직접 사용)
        act(() => {
            useDiscordStore.setState({
                discordCloudHostId: 'host-test-1',
                relayConnected: true,
                relayConnecting: false,
                relayStatusChecked: true,
            });
        });

        // Discord 팝업 열기 → 노드 목록 1회 로드
        act(() => {
            useUIStore.getState().setShowDiscordSection(true);
        });

        await waitFor(() => {
            expect(window.api.relayListHostNodes).toHaveBeenCalledTimes(1);
        });

        // App 리렌더 유발: uptime 시계 틱 시뮬레이션 (실제 앱에서는 1초마다 발생)
        for (let i = 1; i <= 5; i++) {
            // eslint-disable-next-line no-await-in-loop
            await act(async () => {
                useServerStore.setState({ nowEpoch: Math.floor(Date.now() / 1000) + i });
                // 마이크로태스크 플러시 — effect 재발화 시 fetch가 실행될 시간을 준다
                await new Promise((r) => setTimeout(r, 20));
            });
        }

        // 리렌더가 반복돼도 노드 목록 재요청이 없어야 함
        expect(window.api.relayListHostNodes).toHaveBeenCalledTimes(1);
    }, 15000);
});
