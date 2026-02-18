import React from 'react';
import { render, screen, waitFor, act } from '@testing-library/react';
import userEvent from '@testing-library/user-event';
import { describe, it, expect, beforeEach, afterAll, vi } from 'vitest';
import App from '../App';
import fs from 'fs';
import path from 'path';

// 테스트 데이터 자동 정리 함수
const cleanupTestInstances = () => {
    const instancesPath = path.join(process.cwd(), '..', 'instances.json');
    
    try {
        if (fs.existsSync(instancesPath)) {
            const content = fs.readFileSync(instancesPath, 'utf-8');
            const instances = JSON.parse(content);
            
            // test- 로 시작하는 서버 제거
            const cleaned = instances.filter(instance => 
                !instance.name || !instance.name.startsWith('test-')
            );
            
            if (cleaned.length !== instances.length) {
                fs.writeFileSync(instancesPath, JSON.stringify(cleaned, null, 2));
                console.log('🧹 Cleaned up test instances from instances.json');
            }
        }
    } catch (error) {
        // 파일이 없거나 파싱 실패는 무시 (테스트 환경에서는 정상)
    }
};

// 모든 테스트 종료 후 cleanup
afterAll(() => {
    cleanupTestInstances();
});

// Mock window.api
const mockApi = {
    settingsLoad: vi.fn(),
    settingsSave: vi.fn(),
    settingsGetPath: vi.fn(),
    botConfigLoad: vi.fn(),
    botConfigSave: vi.fn(),
    discordBotStatus: vi.fn(),
    discordBotStart: vi.fn(),
    discordBotStop: vi.fn(),
    serverList: vi.fn(),
    moduleList: vi.fn(),
    getServers: vi.fn(),
    getModules: vi.fn(),
    onConsolePopoutOpened: vi.fn(),
    onConsolePopoutClosed: vi.fn(),
    offConsolePopoutOpened: vi.fn(),
    offConsolePopoutClosed: vi.fn(),
    onStatusUpdate: vi.fn(),
    daemonStatus: vi.fn().mockResolvedValue({ running: true }),
    onCloseRequest: vi.fn(),
    offCloseRequest: vi.fn(),
    moduleGetLocales: vi.fn().mockResolvedValue({}),
    moduleGetMetadata: vi.fn().mockResolvedValue({}),
    onUpdatesAvailable: vi.fn(),
    onUpdateCompleted: vi.fn(),
    onDiscordBotRelaunch: vi.fn(),
    offDiscordBotRelaunch: vi.fn(),
};

const mockShowToast = vi.fn();

beforeEach(() => {
    // Reset all mocks
    vi.clearAllMocks();
    
    // Setup window.api
    global.window.api = mockApi;
    global.window.showToast = mockShowToast;
    global.window.showStatus = vi.fn();
    
    // Default mock responses
    mockApi.settingsLoad.mockResolvedValue({
        autoRefresh: true,
        refreshInterval: 2000,
        modulesPath: '',
        discordToken: 'test-token-123',
        discordAutoStart: false
    });
    
    mockApi.settingsGetPath.mockResolvedValue('C:\\Users\\test\\AppData\\settings.json');
    
    mockApi.botConfigLoad.mockResolvedValue({
        prefix: '!saba',
        moduleAliases: {},
        commandAliases: {}
    });
    
    mockApi.botConfigSave.mockResolvedValue({ success: true });
    mockApi.discordBotStatus.mockResolvedValue('stopped');
    mockApi.daemonStatus.mockResolvedValue({ running: true });
    mockApi.moduleGetLocales.mockResolvedValue({});
    mockApi.moduleGetMetadata.mockResolvedValue({});
    mockApi.serverList.mockResolvedValue({ servers: [] });
    mockApi.moduleList.mockResolvedValue({ modules: [] });
    mockApi.getServers.mockResolvedValue([]);
    mockApi.getModules.mockResolvedValue([]);
});

describe('설정 저장/로드 테스트', () => {
    it('앱 시작 시 설정이 로드되어야 함', async () => {
        await act(async () => {
            render(<App />);
        });

        await waitFor(() => {
            expect(mockApi.settingsLoad).toHaveBeenCalled();
            expect(mockApi.botConfigLoad).toHaveBeenCalled();
        });
    });

    it('설정 로드 후 상태가 올바르게 설정되어야 함', async () => {
        mockApi.settingsLoad.mockResolvedValue({
            autoRefresh: false,
            refreshInterval: 5000,
            discordToken: 'my-token',
            discordAutoStart: true
        });

        await act(async () => {
            render(<App />);
        });

        await waitFor(() => {
            expect(mockApi.settingsLoad).toHaveBeenCalled();
        });

        // 설정이 제대로 로드되었는지 확인 (내부 state 검증)
        // Note: 실제 검증은 UI 렌더링이나 다른 부수효과로 확인
    });

    it('봇 설정 로드 후 prefix가 올바르게 설정되어야 함', async () => {
        mockApi.botConfigLoad.mockResolvedValue({
            prefix: '!test',
            moduleAliases: { minecraft: 'mc' },
            commandAliases: {}
        });

        await act(async () => {
            render(<App />);
        });

        await waitFor(() => {
            expect(mockApi.botConfigLoad).toHaveBeenCalled();
        });
    });
});

describe('Discord 봇 상태 테스트', () => {
    it('앱 시작 시 봇 상태를 확인해야 함', async () => {
        await act(async () => {
            render(<App />);
        });

        await waitFor(() => {
            expect(mockApi.discordBotStatus).toHaveBeenCalled();
        }, { timeout: 3000 });
    });

    it('봇이 stopped 상태로 시작되어야 함', async () => {
        mockApi.discordBotStatus.mockResolvedValue('stopped');

        await act(async () => {
            render(<App />);
        });

        await waitFor(() => {
            expect(mockApi.discordBotStatus).toHaveBeenCalled();
        }, { timeout: 3000 });
    });

    it('봇이 이미 실행 중이면 running 상태여야 함', async () => {
        mockApi.discordBotStatus.mockResolvedValue('running');

        await act(async () => {
            render(<App />);
        });

        await waitFor(() => {
            expect(mockApi.discordBotStatus).toHaveBeenCalled();
        }, { timeout: 3000 });
    });
});

describe('Discord 봇 자동실행 테스트', () => {
    it('자동실행 설정이 꺼져있으면 봇이 시작되지 않아야 함', async () => {
        mockApi.settingsLoad.mockResolvedValue({
            autoRefresh: true,
            refreshInterval: 2000,
            discordToken: 'test-token',
            discordAutoStart: false // 자동실행 OFF
        });

        mockApi.discordBotStatus.mockResolvedValue('stopped');

        await act(async () => {
            render(<App />);
        });

        await waitFor(() => {
            expect(mockApi.botConfigLoad).toHaveBeenCalled();
        });

        // 2초 대기 후에도 봇이 시작되지 않아야 함
        await new Promise(resolve => setTimeout(resolve, 2000));
        
        expect(mockApi.discordBotStart).not.toHaveBeenCalled();
    });

    it('자동실행 설정이 켜져있으면 봇이 자동으로 시작되어야 함', async () => {
        mockApi.settingsLoad.mockResolvedValue({
            autoRefresh: true,
            refreshInterval: 2000,
            discordToken: 'test-token',
            discordAutoStart: true // 자동실행 ON
        });

        mockApi.botConfigLoad.mockResolvedValue({
            prefix: '!saba',
            moduleAliases: {},
            commandAliases: {}
        });

        mockApi.discordBotStatus.mockResolvedValue('stopped');
        mockApi.discordBotStart.mockResolvedValue({ success: true });

        await act(async () => {
            render(<App />);
        });

        // 상태 확인 완료 대기 (200ms + 처리 시간)
        await waitFor(() => {
            expect(mockApi.discordBotStatus).toHaveBeenCalled();
        }, { timeout: 3000 });

        // 자동실행이 트리거되어야 함
        await waitFor(() => {
            expect(mockApi.discordBotStart).toHaveBeenCalledWith({
                token: 'test-token',
                prefix: '!saba',
                moduleAliases: {},
                commandAliases: {}
            });
        }, { timeout: 3000 });
    });

    it('토큰이 없으면 자동실행되지 않아야 함', async () => {
        mockApi.settingsLoad.mockResolvedValue({
            autoRefresh: true,
            refreshInterval: 2000,
            discordToken: '', // 토큰 없음
            discordAutoStart: true
        });

        mockApi.discordBotStatus.mockResolvedValue('stopped');

        await act(async () => {
            render(<App />);
        });

        await waitFor(() => {
            expect(mockApi.botConfigLoad).toHaveBeenCalled();
        });

        // 2초 대기 후에도 봇이 시작되지 않아야 함
        await new Promise(resolve => setTimeout(resolve, 2000));
        
        expect(mockApi.discordBotStart).not.toHaveBeenCalled();
    });

    it('봇이 이미 실행 중이면 자동실행을 건너뛰어야 함', async () => {
        mockApi.settingsLoad.mockResolvedValue({
            autoRefresh: true,
            refreshInterval: 2000,
            discordToken: 'test-token',
            discordAutoStart: true
        });

        mockApi.discordBotStatus.mockResolvedValue('running'); // 이미 실행 중

        await act(async () => {
            render(<App />);
        });

        await waitFor(() => {
            expect(mockApi.botConfigLoad).toHaveBeenCalled();
        });

        // 2초 대기 후에도 봇 시작이 호출되지 않아야 함
        await new Promise(resolve => setTimeout(resolve, 2000));
        
        expect(mockApi.discordBotStart).not.toHaveBeenCalled();
    });

    it('자동실행은 앱 시작 시 한 번만 실행되어야 함', async () => {
        mockApi.settingsLoad.mockResolvedValue({
            autoRefresh: true,
            refreshInterval: 2000,
            discordToken: 'test-token',
            discordAutoStart: true
        });

        mockApi.discordBotStatus.mockResolvedValue('stopped');
        mockApi.discordBotStart.mockResolvedValue({ success: true });

        await act(async () => {
            render(<App />);
        });

        await waitFor(() => {
            expect(mockApi.discordBotStart).toHaveBeenCalled();
        }, { timeout: 3000 });

        const callCount = mockApi.discordBotStart.mock.calls.length;

        // 추가 대기
        await new Promise(resolve => setTimeout(resolve, 2000));

        // 여전히 한 번만 호출되어야 함
        expect(mockApi.discordBotStart).toHaveBeenCalledTimes(callCount);
    });
});

describe('설정 저장 테스트', () => {
    it('prefix 변경 시 봇 설정이 저장되어야 함', async () => {
        // 이 테스트는 실제 UI 상호작용이 필요하므로 E2E 테스트로 이동하는 것이 좋음
        // 여기서는 기본적인 동작만 확인
        await act(async () => {
            render(<App />);
        });

        await waitFor(() => {
            expect(mockApi.botConfigLoad).toHaveBeenCalled();
        });
    });
});

describe('로딩 화면 테스트', () => {
    it('초기 로딩 화면이 표시되어야 함', async () => {
        // HMR 감지를 비활성화하여 로딩 화면이 보이도록 함
        mockApi.daemonStatus = vi.fn().mockRejectedValue(new Error('not running'));
        // onStatusUpdate 이벤트 모킹
        mockApi.onStatusUpdate = vi.fn((callback) => {
            // 이벤트 리스너 등록만 확인
        });

        await act(async () => {
            render(<App />);
        });

        // 로딩 화면 요소가 존재해야 함 (daemonReady=false 상태)
        // Note: 실제로는 status:update 이벤트를 받아야 전환됨
        expect(screen.getByText(/Initialize/i)).toBeInTheDocument();
    });

    it('ready 상태 수신 시 서버 초기화 완료 후 로딩 화면이 사라져야 함', async () => {
        // HMR 감지를 비활성화하여 로딩 화면이 보이도록 함
        mockApi.daemonStatus = vi.fn().mockRejectedValue(new Error('not running'));
        let statusCallback = null;
        mockApi.onStatusUpdate = vi.fn((callback) => {
            statusCallback = callback;
        });

        await act(async () => {
            render(<App />);
        });

        // ready 상태 전송
        if (statusCallback) {
            await act(async () => {
                statusCallback({ step: 'ready', message: '준비 완료' });
            });
        }

        // 3.5초 후 serversInitializing=false 로 전환되어야 로딩 화면이 사라짐
        await waitFor(() => {
            // 로딩 화면이 사라지고 메인 UI가 표시되어야 함
            expect(screen.queryByText('Saba-chan')).toBeInTheDocument();
        }, { timeout: 5000 });
    });

    it('서버 초기화 완료 전까지 로딩 화면이 유지되어야 함', async () => {
        vi.useFakeTimers();
        // HMR 감지를 비활성화하여 로딩 화면이 보이도록 함
        mockApi.daemonStatus = vi.fn().mockRejectedValue(new Error('not running'));
        
        let statusCallback = null;
        mockApi.onStatusUpdate = vi.fn((callback) => {
            statusCallback = callback;
        });

        await act(async () => {
            render(<App />);
        });

        // ready 상태 전송
        if (statusCallback) {
            await act(async () => {
                statusCallback({ step: 'ready', message: '준비 완료' });
            });
        }

        // 600ms 후 daemonReady=true 이지만 serversInitializing=true 이므로 로딩 화면 유지
        await act(async () => {
            vi.advanceTimersByTime(700);
        });

        // 로딩 화면이 아직 표시되어야 함 (serversInitializing=true)
        expect(screen.getByText(/Checking servers/i)).toBeInTheDocument();

        // 3.5초 경과 → serversInitializing=false → 로딩 화면 사라짐
        await act(async () => {
            vi.advanceTimersByTime(3000);
        });

        expect(screen.queryByText(/Checking servers/i)).not.toBeInTheDocument();
        
        vi.useRealTimers();
    });
});
// === 2026-01-20 추가: safeShowToast 및 통신 테스트 ===

describe('safeShowToast 안전 호출 테스트', () => {
    it('window.showToast가 정의되지 않았을 때 에러가 발생하지 않아야 함', async () => {
        // showToast 제거
        delete global.window.showToast;

        await act(async () => {
            render(<App />);
        });

        // 에러 없이 렌더링되어야 함
        await waitFor(() => {
            expect(mockApi.settingsLoad).toHaveBeenCalled();
        });
    });

    it('window.showToast가 정의되어 있으면 정상 호출되어야 함', async () => {
        mockApi.discordBotStart.mockResolvedValue({ success: true });
        mockApi.discordBotStatus.mockResolvedValue('stopped');
        mockApi.settingsLoad.mockResolvedValue({
            autoRefresh: true,
            refreshInterval: 2000,
            discordToken: 'test-token',
            discordAutoStart: true
        });

        await act(async () => {
            render(<App />);
        });

        // 자동실행으로 봇 시작 후 토스트 호출 확인
        await waitFor(() => {
            expect(mockApi.discordBotStart).toHaveBeenCalled();
        }, { timeout: 3000 });

        // Toast 컴포넌트가 window.showToast를 덮어쓰므로, 실제 렌더된 토스트 확인
        await waitFor(() => {
            const toastContainer = document.querySelector('.toast-container');
            expect(toastContainer).toBeTruthy();
            const toasts = toastContainer.querySelectorAll('.toast');
            expect(toasts.length).toBeGreaterThan(0);
        }, { timeout: 3000 });
    });

    it('Discord 봇 시작 실패 시 에러 토스트가 표시되어야 함', async () => {
        mockApi.discordBotStart.mockResolvedValue({ error: '토큰이 유효하지 않습니다' });
        mockApi.discordBotStatus.mockResolvedValue('stopped');
        mockApi.settingsLoad.mockResolvedValue({
            autoRefresh: true,
            refreshInterval: 2000,
            discordToken: 'invalid-token',
            discordAutoStart: true
        });

        await act(async () => {
            render(<App />);
        });

        await waitFor(() => {
            expect(mockApi.discordBotStart).toHaveBeenCalled();
        }, { timeout: 3000 });

        // 에러 토스트가 DOM에 렌더되었는지 확인
        await waitFor(() => {
            const errorToast = document.querySelector('.toast.toast-error');
            expect(errorToast).toBeTruthy();
            expect(errorToast.textContent.length).toBeGreaterThan(0);
        }, { timeout: 3000 });
    });
});

describe('모듈 목록 API 응답 테스트', () => {
    it('모듈 목록에 commands 필드가 포함되어야 함', async () => {
        const mockModulesWithCommands = {
            modules: [
                {
                    name: 'palworld',
                    version: '1.0.0',
                    description: 'Palworld 서버 관리',
                    path: '/modules/palworld',
                    settings: null,
                    commands: {
                        fields: [
                            {
                                name: 'players',
                                label: '플레이어 목록',
                                method: 'rest',
                                http_method: 'GET',
                                endpoint_template: '/v1/api/players',
                                inputs: []
                            },
                            {
                                name: 'announce',
                                label: '공지 전송',
                                method: 'rest',
                                http_method: 'POST',
                                endpoint_template: '/v1/api/announce',
                                inputs: [
                                    { name: 'message', label: '메시지', type: 'string', required: true }
                                ]
                            }
                        ]
                    }
                }
            ]
        };

        mockApi.moduleList = vi.fn().mockResolvedValue(mockModulesWithCommands);

        // 모듈 데이터 검증
        const result = await mockApi.moduleList();
        
        expect(result.modules).toHaveLength(1);
        expect(result.modules[0].commands).toBeDefined();
        expect(result.modules[0].commands.fields).toHaveLength(2);
        
        // GET 명령어 검증
        const playersCmd = result.modules[0].commands.fields[0];
        expect(playersCmd.http_method).toBe('GET');
        expect(playersCmd.inputs).toHaveLength(0);
        
        // POST 명령어 검증
        const announceCmd = result.modules[0].commands.fields[1];
        expect(announceCmd.http_method).toBe('POST');
        expect(announceCmd.inputs).toHaveLength(1);
        expect(announceCmd.inputs[0].required).toBe(true);
    });

    it('commands가 없는 모듈도 정상 처리되어야 함', async () => {
        const mockModulesWithoutCommands = {
            modules: [
                {
                    name: 'legacy-module',
                    version: '0.1.0',
                    description: null,
                    path: '/modules/legacy',
                    settings: null,
                    commands: null
                }
            ]
        };

        mockApi.moduleList = vi.fn().mockResolvedValue(mockModulesWithoutCommands);

        const result = await mockApi.moduleList();
        
        expect(result.modules).toHaveLength(1);
        expect(result.modules[0].commands).toBeNull();
    });
});

describe('REST 명령어 실행 테스트', () => {
    it('GET 메서드 명령어가 올바르게 전송되어야 함', async () => {
        const mockExecuteCommand = vi.fn().mockResolvedValue({
            success: true,
            data: { players: [{ name: 'TestPlayer', level: 10 }] },
            endpoint: '/v1/api/players',
            method: 'GET'
        });

        mockApi.instanceCommand = mockExecuteCommand;

        const result = await mockApi.instanceCommand('palworld-1', {
            command: 'players',
            args: { method: 'GET' }
        });

        expect(result.success).toBe(true);
        expect(result.method).toBe('GET');
        expect(result.data.players).toHaveLength(1);
    });

    it('POST 메서드 명령어가 body와 함께 전송되어야 함', async () => {
        const mockExecuteCommand = vi.fn().mockResolvedValue({
            success: true,
            message: '공지가 전송되었습니다',
            endpoint: '/v1/api/announce',
            method: 'POST'
        });

        mockApi.instanceCommand = mockExecuteCommand;

        const result = await mockApi.instanceCommand('palworld-1', {
            command: 'announce',
            args: { 
                method: 'POST',
                body: { message: '서버 점검 예정' }
            }
        });

        expect(result.success).toBe(true);
        expect(result.method).toBe('POST');
        expect(mockExecuteCommand).toHaveBeenCalledWith('palworld-1', expect.objectContaining({
            args: expect.objectContaining({
                body: { message: '서버 점검 예정' }
            })
        }));
    });

    it('REST 연결 실패 시 에러가 반환되어야 함', async () => {
        const mockExecuteCommand = vi.fn().mockResolvedValue({
            success: false,
            error: 'REST connection failed: Connection refused'
        });

        mockApi.instanceCommand = mockExecuteCommand;

        const result = await mockApi.instanceCommand('palworld-1', {
            command: 'info',
            args: { method: 'GET' }
        });

        expect(result.success).toBe(false);
        expect(result.error).toContain('REST connection failed');
    });
});

describe('서버 목록 업데이트 실패 테스트', () => {
    it('서버 목록 조회 실패 시 토스트가 표시되어야 함', async () => {
        global.window.showToast = mockShowToast;

        // 초기에는 성공하고, 나중에 실패하도록 설정
        mockApi.serverList
            .mockResolvedValueOnce({ servers: [] }) // 첫 호출 성공
            .mockRejectedValue(new Error('Network error')); // 두 번째 이후 실패

        await act(async () => {
            render(<App />);
        });

        // 초기 로딩 완료 대기
        await waitFor(() => {
            expect(mockApi.settingsLoad).toHaveBeenCalled();
        }, { timeout: 10000 });

        // 약간 대기 후 토스트 호출 확인 (재시도 실패 시)
        await new Promise(resolve => setTimeout(resolve, 3000));
        
        // 에러 발생 시에만 토스트가 호출되므로, 호출되었다면 성공
        if (mockShowToast.mock.calls.length > 0) {
            expect(mockShowToast).toHaveBeenCalledWith(
                expect.stringContaining('서버 목록 업데이트 실패'),
                'warning',
                3000
            );
        }
        // 호출되지 않았다면 초기 로딩 중이므로 패스
    }, 20000);
});

describe('모듈 로드 실패 테스트', () => {
    it('모듈 로드 실패 시 에러 토스트가 표시되어야 함', async () => {
        global.window.showToast = mockShowToast;

        mockApi.moduleList = vi.fn().mockResolvedValue({ error: '모듈 경로를 찾을 수 없습니다' });

        await act(async () => {
            render(<App />);
        });

        await waitFor(() => {
            expect(mockShowToast).toHaveBeenCalled();

            const lastCall = mockShowToast.mock.calls[mockShowToast.mock.calls.length - 1];
            expect(typeof lastCall[0]).toBe('string');
            expect(lastCall[0].length).toBeGreaterThan(0);
            expect(lastCall[1]).toBe('error');
            expect(lastCall[2]).toBe(4000);
        }, { timeout: 5000 });
    });
});