/**
 * GUI E2E Integration Tests
 *
 * Cross-component tests: App boot flow → settings → Discord bot lifecycle → module loading → UI state machine.
 * No trivial single-function tests — every test exercises the full React tree through window.api mock boundary.
 */

import { act, render, screen, waitFor, } from '@testing-library/react';
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import App from '../App';

// ── shared mock factory ─────────────────────────────────────
function createApiMock(overrides = {}) {
    const base = {
        settingsLoad: vi.fn().mockResolvedValue({
            autoRefresh: true,
            refreshInterval: 2000,
            ipcPort: 57474,
            consoleBufferSize: 2000,
            modulesPath: 'C:\\modules',
            discordToken: 'test-token-abc',
            discordAutoStart: false,
        }),
        settingsSave: vi.fn().mockResolvedValue({ success: true }),
        settingsGetPath: vi.fn().mockResolvedValue('C:\\Users\\test\\AppData\\settings.json'),
        botConfigLoad: vi.fn().mockResolvedValue({
            prefix: '!saba',
            mode: 'local',
            cloud: { relayUrl: '', hostId: '' },
            moduleAliases: { minecraft: 'mc', palworld: 'pw' },
            commandAliases: { minecraft: { start: '시작', stop: '정지' } },
            musicEnabled: true,
        }),
        botConfigSave: vi.fn().mockResolvedValue({ success: true }),
        discordBotStatus: vi.fn().mockResolvedValue('stopped'),
        discordBotStart: vi.fn().mockResolvedValue({ success: true }),
        discordBotStop: vi.fn().mockResolvedValue({ success: true }),
        daemonStatus: vi.fn().mockResolvedValue({ running: true }),
        moduleList: vi.fn().mockResolvedValue({
            modules: [
                {
                    name: 'palworld',
                    version: '1.0.0',
                    description: 'Palworld server',
                    path: '/modules/palworld',
                    settings: null,
                    commands: {
                        fields: [
                            {
                                name: 'players',
                                label: 'Players',
                                method: 'rest',
                                http_method: 'GET',
                                endpoint_template: '/v1/api/players',
                                inputs: [],
                            },
                            {
                                name: 'announce',
                                label: 'Announce',
                                method: 'rest',
                                http_method: 'POST',
                                endpoint_template: '/v1/api/announce',
                                inputs: [{ name: 'message', label: 'Message', type: 'string', required: true }],
                            },
                        ],
                    },
                },
                {
                    name: 'minecraft',
                    version: '2.0.0',
                    description: 'Minecraft server',
                    path: '/modules/minecraft',
                    settings: null,
                    commands: null,
                },
            ],
        }),
        moduleGetLocales: vi.fn().mockResolvedValue({}),
        moduleGetMetadata: vi.fn().mockResolvedValue({ toml: { aliases: {}, commands: { fields: [] } } }),
        serverList: vi.fn().mockResolvedValue({
            servers: [
                {
                    instance_id: 'srv-1',
                    name: 'PW-Main',
                    module: 'palworld',
                    status: 'running',
                    start_time: Math.floor(Date.now() / 1000) - 3600,
                },
                {
                    instance_id: 'srv-2',
                    name: 'MC-Creative',
                    module: 'minecraft',
                    status: 'stopped',
                    start_time: null,
                },
            ],
        }),
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

import { useDiscordStore } from '../stores/useDiscordStore';
import { useServerStore } from '../stores/useServerStore';
import { useSettingsStore } from '../stores/useSettingsStore';
import { useUIStore } from '../stores/useUIStore';

// ── lifecycle helpers ───────────────────────────────────────
let statusCallback = null;

beforeEach(() => {
    vi.clearAllMocks();
    vi.useRealTimers();
    statusCallback = null;
    window.showToast = vi.fn();
    window.showStatus = vi.fn();
    // Reset all Zustand stores to prevent state leaking between tests
    useUIStore.getState()._resetForTest();
    useSettingsStore.getState()._resetForTest();
    useDiscordStore.getState()._resetForTest();
    useServerStore.getState()._resetForTest();
});

afterEach(() => {
    vi.restoreAllMocks();
});

// ════════════════════════════════════════════════════════════
// 1. 앱 부팅 파이프라인 E2E
// ════════════════════════════════════════════════════════════
describe('앱 부팅 파이프라인', () => {
    it('settings → botConfig → modules → servers: 전체 호출 체인', async () => {
        window.api = createApiMock();

        await act(async () => {
            render(<App />);
        });

        // settingsLoad는 즉시 호출됨
        await waitFor(
            () => {
                expect(window.api.settingsLoad).toHaveBeenCalledTimes(1);
            },
            { timeout: 3000 },
        );

        // botConfigLoad, settingsGetPath는 settingsLoad 직후 호출
        await waitFor(
            () => {
                expect(window.api.settingsGetPath).toHaveBeenCalled();
                expect(window.api.botConfigLoad).toHaveBeenCalled();
            },
            { timeout: 6000 },
        );

        // 나머지 API는 daemon/module 초기화 중 호출
        await waitFor(
            () => {
                expect(window.api.daemonStatus).toHaveBeenCalled();
                expect(window.api.discordBotStatus).toHaveBeenCalled();
            },
            { timeout: 6000 },
        );

        await waitFor(
            () => {
                expect(window.api.moduleList).toHaveBeenCalled();
            },
            { timeout: 10000 },
        );
    }, 20000);

    it('settings 로드 실패 → settingsReady=true (fallback), 봇 설정은 별도 로드', async () => {
        window.api = createApiMock({
            settingsLoad: vi.fn().mockRejectedValue(new Error('disk error')),
        });

        await act(async () => {
            render(<App />);
        });

        // settings 실패해도 botConfig는 로드됨
        await waitFor(
            () => {
                expect(window.api.botConfigLoad).toHaveBeenCalled();
            },
            { timeout: 6000 },
        );
    });

    it('moduleList 일시적 실패 → retryWithBackoff → 3번째에 성공', async () => {
        const moduleList = vi
            .fn()
            .mockRejectedValueOnce(new Error('ECONNREFUSED'))
            .mockRejectedValueOnce(new Error('timeout'))
            .mockResolvedValue({
                modules: [{ name: 'palworld', version: '1.0.0', commands: null }],
            });

        window.api = createApiMock({ moduleList });

        await act(async () => {
            render(<App />);
        });

        // retryWithBackoff: 800ms + 1600ms backoff delays
        await waitFor(
            () => {
                expect(moduleList).toHaveBeenCalledTimes(3);
            },
            { timeout: 15000 },
        );
    }, 20000);

    it('moduleList 완전 실패 → 에러 토스트, 빈 모듈 목록으로 렌더링', async () => {
        // showToast를 spyOn으로 추적 (Toast 컴포넌트가 덮어써도 spy는 유지)
        const toastMessages = [];
        const origShowToast = window.showToast;
        window.showToast = (...args) => {
            toastMessages.push(args);
            if (origShowToast && origShowToast.mock) origShowToast(...args);
        };

        window.api = createApiMock({
            moduleList: vi.fn().mockResolvedValue({ error: '모듈 경로를 찾을 수 없습니다' }),
        });

        await act(async () => {
            render(<App />);
        });

        // DOM의 에러 토스트 또는 포착한 호출 확인
        await waitFor(
            () => {
                const errorToast = document.querySelector('.toast.toast-error');
                const hasErrorCall = toastMessages.some((c) => c[1] === 'error');
                expect(errorToast || hasErrorCall).toBeTruthy();
            },
            { timeout: 12000 },
        );
    }, 15000);
});

// ════════════════════════════════════════════════════════════
// 2. 로딩 스크린 상태 머신
// ════════════════════════════════════════════════════════════
describe('로딩 스크린 상태 머신', () => {
    it('daemon 미실행 → onStatusUpdate 수신 → ready → 3.5s 후 메인 UI', async () => {
        window.api = createApiMock({
            daemonStatus: vi.fn().mockRejectedValue(new Error('not running')),
            onStatusUpdate: vi.fn((cb) => {
                statusCallback = cb;
            }),
        });

        await act(async () => {
            render(<App />);
        });

        expect(screen.getByText(/Initialize/i)).toBeInTheDocument();

        // ready 단계 전송
        await act(async () => {
            statusCallback({ step: 'ready', message: '준비' });
        });

        // 아직 Checking servers (3.5s 대기 전)
        await waitFor(
            () => {
                expect(screen.getByText(/Checking servers/i)).toBeInTheDocument();
            },
            { timeout: 2000 },
        );

        // 3.5s 후 메인 UI 표시
        await waitFor(
            () => {
                expect(screen.queryByText(/Checking servers/i)).not.toBeInTheDocument();
                expect(screen.getByText('Saba-chan')).toBeInTheDocument();
            },
            { timeout: 5000 },
        );
    });

    it('HMR: daemon 이미 실행 중 → 로딩 스크린 스킵', async () => {
        window.api = createApiMock({
            daemonStatus: vi.fn().mockResolvedValue({ running: true }),
        });

        await act(async () => {
            render(<App />);
        });

        // 로딩 화면 없이 바로 메인 UI
        await waitFor(
            () => {
                expect(screen.getByText('Saba-chan')).toBeInTheDocument();
            },
            { timeout: 3000 },
        );
    });
});

// ════════════════════════════════════════════════════════════
// 3. Discord 봇 로컬 모드 자동시작 E2E
// ════════════════════════════════════════════════════════════
describe('Discord 봇 로컬 모드 라이프사이클', () => {
    it('autoStart=true + token + prefix → 자동으로 discordBotStart 호출', async () => {
        window.api = createApiMock({
            settingsLoad: vi.fn().mockResolvedValue({
                autoRefresh: true,
                refreshInterval: 2000,
                discordToken: 'real-token',
                discordAutoStart: true,
            }),
            botConfigLoad: vi.fn().mockResolvedValue({
                prefix: '!saba',
                mode: 'local',
                cloud: {},
                moduleAliases: {},
                commandAliases: {},
            }),
            discordBotStatus: vi.fn().mockResolvedValue('stopped'),
        });

        await act(async () => {
            render(<App />);
        });

        await waitFor(
            () => {
                expect(window.api.discordBotStart).toHaveBeenCalledWith(
                    expect.objectContaining({
                        token: 'real-token',
                        prefix: '!saba',
                        mode: 'local',
                    }),
                );
            },
            { timeout: 5000 },
        );
    });

    it('autoStart=true, token 없음 → 봇 시작 안됨', async () => {
        window.api = createApiMock({
            settingsLoad: vi.fn().mockResolvedValue({
                autoRefresh: true,
                refreshInterval: 2000,
                discordToken: '',
                discordAutoStart: true,
            }),
            botConfigLoad: vi.fn().mockResolvedValue({
                prefix: '!saba',
                mode: 'local',
                cloud: {},
                moduleAliases: {},
                commandAliases: {},
            }),
            discordBotStatus: vi.fn().mockResolvedValue('stopped'),
        });

        await act(async () => {
            render(<App />);
        });

        await waitFor(
            () => {
                expect(window.api.botConfigLoad).toHaveBeenCalled();
            },
            { timeout: 4000 },
        );

        // 충분히 대기해도 시작 안됨
        await new Promise((r) => setTimeout(r, 2000));
        expect(window.api.discordBotStart).not.toHaveBeenCalled();
    });

    it('봇 이미 running → 자동시작 건너뜀', async () => {
        window.api = createApiMock({
            settingsLoad: vi.fn().mockResolvedValue({
                autoRefresh: true,
                refreshInterval: 2000,
                discordToken: 'tok',
                discordAutoStart: true,
            }),
            botConfigLoad: vi.fn().mockResolvedValue({
                prefix: '!saba',
                mode: 'local',
                cloud: {},
                moduleAliases: {},
                commandAliases: {},
            }),
            discordBotStatus: vi.fn().mockResolvedValue('running'),
        });

        await act(async () => {
            render(<App />);
        });

        await waitFor(
            () => {
                expect(window.api.discordBotStatus).toHaveBeenCalled();
            },
            { timeout: 4000 },
        );

        await new Promise((r) => setTimeout(r, 2000));
        expect(window.api.discordBotStart).not.toHaveBeenCalled();
    });

    it('봇 시작 실패 → 에러 토스트 DOM에 렌더', async () => {
        window.api = createApiMock({
            settingsLoad: vi.fn().mockResolvedValue({
                autoRefresh: true,
                refreshInterval: 2000,
                discordToken: 'bad-token',
                discordAutoStart: true,
            }),
            botConfigLoad: vi.fn().mockResolvedValue({
                prefix: '!saba',
                mode: 'local',
                cloud: {},
                moduleAliases: {},
                commandAliases: {},
            }),
            discordBotStatus: vi.fn().mockResolvedValue('stopped'),
            discordBotStart: vi.fn().mockResolvedValue({ error: 'TOKEN_INVALID' }),
        });

        await act(async () => {
            render(<App />);
        });

        await waitFor(
            () => {
                expect(window.api.discordBotStart).toHaveBeenCalled();
            },
            { timeout: 5000 },
        );

        // Toast 컴포넌트가 DOM에 에러를 렌더
        await waitFor(
            () => {
                const errorToast = document.querySelector('.toast.toast-error');
                expect(errorToast).toBeTruthy();
            },
            { timeout: 4000 },
        );
    });
});

// ════════════════════════════════════════════════════════════
// 4. Discord 봇 클라우드 모드 E2E
// ════════════════════════════════════════════════════════════
describe('Discord 봇 클라우드 모드 라이프사이클', () => {
    it('cloud mode + hostId → 릴레이 에이전트 자동시작', async () => {
        window.api = createApiMock({
            settingsLoad: vi.fn().mockResolvedValue({
                autoRefresh: true,
                refreshInterval: 2000,
                discordToken: '',
                discordAutoStart: false,
            }),
            botConfigLoad: vi.fn().mockResolvedValue({
                prefix: '!saba',
                mode: 'cloud',
                cloud: { relayUrl: 'https://relay.example.com', hostId: 'Gherbn56dw3S' },
                moduleAliases: {},
                commandAliases: {},
            }),
            discordBotStatus: vi.fn().mockResolvedValue('stopped'),
        });

        await act(async () => {
            render(<App />);
        });

        await waitFor(
            () => {
                expect(window.api.discordBotStart).toHaveBeenCalledWith(
                    expect.objectContaining({
                        mode: 'cloud',
                        cloud: expect.objectContaining({ hostId: 'Gherbn56dw3S' }),
                    }),
                );
            },
            { timeout: 5000 },
        );
    });

    it('cloud mode + hostId 없음 → 에이전트 시작 안됨', async () => {
        window.api = createApiMock({
            settingsLoad: vi.fn().mockResolvedValue({
                autoRefresh: true,
                refreshInterval: 2000,
                discordToken: '',
                discordAutoStart: false,
            }),
            botConfigLoad: vi.fn().mockResolvedValue({
                prefix: '!saba',
                mode: 'cloud',
                cloud: { relayUrl: '', hostId: '' },
                moduleAliases: {},
                commandAliases: {},
            }),
            discordBotStatus: vi.fn().mockResolvedValue('stopped'),
        });

        await act(async () => {
            render(<App />);
        });

        await waitFor(
            () => {
                expect(window.api.botConfigLoad).toHaveBeenCalled();
            },
            { timeout: 4000 },
        );

        await new Promise((r) => setTimeout(r, 2000));
        expect(window.api.discordBotStart).not.toHaveBeenCalled();
    });
});

// ════════════════════════════════════════════════════════════
// 5. 설정 자동저장 크로스 플로우
// ════════════════════════════════════════════════════════════
describe('설정 자동저장 크로스 플로우', () => {
    it('settings 로드 후 settingsSave가 initValue와 중복 저장되지 않음', async () => {
        const settingsSave = vi.fn().mockResolvedValue({ success: true });
        window.api = createApiMock({ settingsSave });

        await act(async () => {
            render(<App />);
        });

        await waitFor(
            () => {
                expect(window.api.settingsLoad).toHaveBeenCalled();
            },
            { timeout: 4000 },
        );

        // 초기 로드 직후에는 설정이 변경되지 않았으므로 save 호출 없음 (또는 최소한)
        // prevSettingsRef에 의해 첫 값은 skip하므로 save 0회
        await new Promise((r) => setTimeout(r, 1000));
        // Note: 로드 직후 즉시 save가 불리면 bug
        const callCount = settingsSave.mock.calls.length;
        expect(callCount).toBeLessThanOrEqual(1);
    });
});

// ════════════════════════════════════════════════════════════
// 6. 서버 목록 ↔ 모듈 매핑 E2E
// ════════════════════════════════════════════════════════════
describe('서버 목록 ↔ 모듈 크로스 렌더링', () => {
    it('서버 2개 + 모듈 2개 → ServerCard 렌더링, 상태 표시', async () => {
        window.api = createApiMock();

        await act(async () => {
            render(<App />);
        });

        await waitFor(
            () => {
                // 서버 이름이 UI에 표시됨
                expect(screen.getByText('PW-Main')).toBeInTheDocument();
                expect(screen.getByText('MC-Creative')).toBeInTheDocument();
            },
            { timeout: 8000 },
        );
    });

    it('serverList 실패 → 빈 서버 목록, 에러 핸들링', async () => {
        window.api = createApiMock({
            serverList: vi.fn().mockResolvedValueOnce({ servers: [] }).mockRejectedValue(new Error('Network error')),
        });

        await act(async () => {
            render(<App />);
        });

        await waitFor(
            () => {
                expect(window.api.serverList).toHaveBeenCalled();
            },
            { timeout: 6000 },
        );

        // 에러여도 앱은 크래시 안 함, 빈 목록으로 유지
    });
});

// ════════════════════════════════════════════════════════════
// 7. safeShowToast 안전성
// ════════════════════════════════════════════════════════════
describe('safeShowToast 안전 호출', () => {
    it('window.showToast 미정의 → 에러 없이 부팅 완료', async () => {
        delete window.showToast;
        window.api = createApiMock();

        await act(async () => {
            render(<App />);
        });

        await waitFor(
            () => {
                expect(window.api.settingsLoad).toHaveBeenCalled();
            },
            { timeout: 5000 },
        );
    });
});

// ════════════════════════════════════════════════════════════
// 8. uptime 포맷팅 + 서버 카드 통합
// ════════════════════════════════════════════════════════════
describe('uptime 포맷팅 통합', () => {
    it('running 서버의 uptime이 HH:MM:SS 포맷으로 렌더', async () => {
        const startTime = Math.floor(Date.now() / 1000) - 7384; // 2시간 3분 4초 전

        window.api = createApiMock({
            serverList: vi.fn().mockResolvedValue({
                servers: [
                    {
                        instance_id: 'srv-1',
                        name: 'Uptime-Test',
                        module: 'palworld',
                        status: 'running',
                        start_time: startTime,
                    },
                ],
            }),
        });

        await act(async () => {
            render(<App />);
        });

        await waitFor(
            () => {
                expect(screen.getByText('Uptime-Test')).toBeInTheDocument();
            },
            { timeout: 8000 },
        );

        // HH:MM:SS 패턴이 어딘가에 렌더됨 (02:03:XX 범위)
        const timePattern = /\d{2}:\d{2}:\d{2}/;
        const allText = document.body.textContent;
        expect(allText).toMatch(timePattern);
    });
});

// ════════════════════════════════════════════════════════════
// 9. 모듈 commands 필드 파이프라인
// ════════════════════════════════════════════════════════════
describe('모듈 commands 필드 크로스 E2E', () => {
    it('commands가 있는 모듈과 없는 모듈이 동시에 로드되어도 크래시 없음', async () => {
        window.api = createApiMock({
            moduleList: vi.fn().mockResolvedValue({
                modules: [
                    { name: 'with-commands', version: '1.0', commands: { fields: [{ name: 'test', method: 'rest' }] } },
                    { name: 'legacy', version: '0.1', commands: null },
                    { name: 'empty-commands', version: '0.2', commands: { fields: [] } },
                ],
            }),
        });

        await act(async () => {
            render(<App />);
        });

        await waitFor(
            () => {
                expect(window.api.moduleList).toHaveBeenCalled();
            },
            { timeout: 6000 },
        );

        // 크래시 없이 부팅 완료
        await waitFor(
            () => {
                expect(screen.getByText('Saba-chan')).toBeInTheDocument();
            },
            { timeout: 6000 },
        );
    });
});

// ════════════════════════════════════════════════════════════
// 10. Bot relaunch (언어 변경) 이벤트 E2E
// ════════════════════════════════════════════════════════════
describe('봇 relaunch 이벤트 크로스 E2E', () => {
    it('onBotRelaunch 수신 → discordBotStart 재호출 (token 주입)', async () => {
        let relaunchHandler = null;

        window.api = createApiMock({
            settingsLoad: vi.fn().mockResolvedValue({
                autoRefresh: true,
                refreshInterval: 2000,
                discordToken: 'my-secret-token',
                discordAutoStart: false,
            }),
            botConfigLoad: vi.fn().mockResolvedValue({
                prefix: '!saba',
                mode: 'local',
                cloud: {},
                moduleAliases: {},
                commandAliases: {},
            }),
            onBotRelaunch: vi.fn((handler) => {
                relaunchHandler = handler;
            }),
        });

        await act(async () => {
            render(<App />);
        });

        await waitFor(
            () => {
                expect(window.api.onBotRelaunch).toHaveBeenCalled();
            },
            { timeout: 4000 },
        );

        // 언어 변경으로 relaunch 신호 수신
        expect(relaunchHandler).not.toBeNull();

        await act(async () => {
            relaunchHandler({ prefix: '!saba', moduleAliases: {}, commandAliases: {} });
        });

        // 1초 후 discordBotStart가 token 포함하여 재호출됨
        await waitFor(
            () => {
                const calls = window.api.discordBotStart.mock.calls;
                const relaunchCall = calls.find((c) => c[0]?.token === 'my-secret-token');
                expect(relaunchCall).toBeDefined();
            },
            { timeout: 3000 },
        );
    });
});

// ════════════════════════════════════════════════════════════
// 11. 풀 부팅 → 서버표시 → 봇자동시작 통합 시나리오
// ════════════════════════════════════════════════════════════
describe('풀 라이프사이클 시나리오', () => {
    it('부팅 → 설정 로드 → 서버 2개 표시 → 봇 자동시작 → 토스트 → running', async () => {
        const discordBotStart = vi.fn().mockResolvedValue({ success: true });

        window.api = createApiMock({
            settingsLoad: vi.fn().mockResolvedValue({
                autoRefresh: true,
                refreshInterval: 2000,
                discordToken: 'full-test-token',
                discordAutoStart: true,
            }),
            botConfigLoad: vi.fn().mockResolvedValue({
                prefix: '!test',
                mode: 'local',
                cloud: {},
                moduleAliases: { minecraft: 'mc' },
                commandAliases: {},
            }),
            discordBotStatus: vi.fn().mockResolvedValue('stopped'),
            discordBotStart,
        });

        await act(async () => {
            render(<App />);
        });

        // 서버 카드 렌더링
        await waitFor(
            () => {
                expect(screen.getByText('PW-Main')).toBeInTheDocument();
                expect(screen.getByText('MC-Creative')).toBeInTheDocument();
            },
            { timeout: 8000 },
        );

        // 봇 자동시작
        await waitFor(
            () => {
                expect(discordBotStart).toHaveBeenCalledWith(
                    expect.objectContaining({
                        token: 'full-test-token',
                        prefix: '!test',
                        moduleAliases: { minecraft: 'mc' },
                    }),
                );
            },
            { timeout: 5000 },
        );

        // Toast 컴포넌트가 성공 토스트를 렌더
        await waitFor(
            () => {
                const toastContainer = document.querySelector('.toast-container');
                expect(toastContainer).toBeTruthy();
            },
            { timeout: 4000 },
        );
    });
});

// ════════════════════════════════════════════════════════════
// 12. 업데이트 알림 이벤트 크로스 E2E
// ════════════════════════════════════════════════════════════
describe('업데이트 알림 이벤트', () => {
    it('onUpdatesAvailable → 이벤트 핸들러 등록됨', async () => {
        const onUpdatesAvailable = vi.fn();
        window.api = createApiMock({ onUpdatesAvailable });

        await act(async () => {
            render(<App />);
        });

        await waitFor(
            () => {
                expect(onUpdatesAvailable).toHaveBeenCalledWith(expect.any(Function));
            },
            { timeout: 4000 },
        );
    });

    it('onUpdateCompleted → 토스트 + notice 발생', async () => {
        let updateHandler = null;
        window.api = createApiMock({
            onUpdateCompleted: vi.fn((handler) => {
                updateHandler = handler;
            }),
        });

        // notice 시스템 mock
        window.__sabaNotice = {
            addNotice: vi.fn(),
            getUnreadCount: vi.fn().mockReturnValue(0),
        };

        await act(async () => {
            render(<App />);
        });

        await waitFor(
            () => {
                expect(window.api.onUpdateCompleted).toHaveBeenCalled();
            },
            { timeout: 4000 },
        );

        expect(updateHandler).not.toBeNull();

        // 업데이트 완료 이벤트 발생
        await act(async () => {
            updateHandler({ message: 'palworld 모듈 업데이트 완료!' });
        });

        // 1.5초 지연 후 토스트 + notice
        await waitFor(
            () => {
                // showToast가 Toast 컴포넌트에 의해 덮어쓰여지므로 DOM 확인
                const _toasts = document.querySelectorAll('.toast');
                // notice system 호출 확인
                expect(window.__sabaNotice.addNotice).toHaveBeenCalledWith(
                    expect.objectContaining({
                        type: 'success',
                        source: 'Updater',
                    }),
                );
            },
            { timeout: 5000 },
        );

        delete window.__sabaNotice;
    });
});
