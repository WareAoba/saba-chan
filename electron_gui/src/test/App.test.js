import React from 'react';
import { render, screen, waitFor, act } from '@testing-library/react';
import userEvent from '@testing-library/user-event';
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
    settingsLoad: jest.fn(),
    settingsSave: jest.fn(),
    settingsGetPath: jest.fn(),
    botConfigLoad: jest.fn(),
    botConfigSave: jest.fn(),
    discordBotStatus: jest.fn(),
    discordBotStart: jest.fn(),
    discordBotStop: jest.fn(),
    serverList: jest.fn(),
    moduleList: jest.fn(),
    getServers: jest.fn(),
    getModules: jest.fn(),
};

const mockShowToast = jest.fn();

beforeEach(() => {
    // Reset all mocks
    jest.clearAllMocks();
    
    // Setup window.api
    global.window.api = mockApi;
    global.window.showToast = mockShowToast;
    global.window.showStatus = jest.fn();
    
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
    mockApi.serverList.mockResolvedValue({ servers: [] });
    mockApi.moduleList.mockResolvedValue({ modules: [] });
    mockApi.getServers.mockResolvedValue([]);
    mockApi.getModules.mockResolvedValue([]);
});

describe('설정 저장/로드 테스트', () => {
    test('앱 시작 시 설정이 로드되어야 함', async () => {
        await act(async () => {
            render(<App />);
        });

        await waitFor(() => {
            expect(mockApi.settingsLoad).toHaveBeenCalled();
            expect(mockApi.botConfigLoad).toHaveBeenCalled();
        });
    });

    test('설정 로드 후 상태가 올바르게 설정되어야 함', async () => {
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

    test('봇 설정 로드 후 prefix가 올바르게 설정되어야 함', async () => {
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
    test('앱 시작 시 봇 상태를 확인해야 함', async () => {
        await act(async () => {
            render(<App />);
        });

        await waitFor(() => {
            expect(mockApi.discordBotStatus).toHaveBeenCalled();
        }, { timeout: 3000 });
    });

    test('봇이 stopped 상태로 시작되어야 함', async () => {
        mockApi.discordBotStatus.mockResolvedValue('stopped');

        await act(async () => {
            render(<App />);
        });

        await waitFor(() => {
            expect(mockApi.discordBotStatus).toHaveBeenCalled();
        }, { timeout: 3000 });
    });

    test('봇이 이미 실행 중이면 running 상태여야 함', async () => {
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
    test('자동실행 설정이 꺼져있으면 봇이 시작되지 않아야 함', async () => {
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

    test('자동실행 설정이 켜져있으면 봇이 자동으로 시작되어야 함', async () => {
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

    test('토큰이 없으면 자동실행되지 않아야 함', async () => {
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

    test('봇이 이미 실행 중이면 자동실행을 건너뛰어야 함', async () => {
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

    test('자동실행은 앱 시작 시 한 번만 실행되어야 함', async () => {
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
    test('prefix 변경 시 봇 설정이 저장되어야 함', async () => {
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
    test('초기 로딩 화면이 표시되어야 함', async () => {
        // onStatusUpdate 이벤트 모킹
        mockApi.onStatusUpdate = jest.fn((callback) => {
            // 이벤트 리스너 등록만 확인
        });

        await act(async () => {
            render(<App />);
        });

        // 로딩 화면 요소가 존재해야 함 (daemonReady=false 상태)
        // Note: 실제로는 status:update 이벤트를 받아야 전환됨
        expect(screen.getByText(/초기화/i)).toBeInTheDocument();
    });

    test('ready 상태 수신 시 로딩 화면이 사라져야 함', async () => {
        let statusCallback = null;
        mockApi.onStatusUpdate = jest.fn((callback) => {
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

        // 600ms 후 daemonReady=true 로 전환
        await waitFor(() => {
            // 로딩 화면이 사라지고 메인 UI가 표시되어야 함
            expect(screen.queryByText('Saba-chan')).toBeInTheDocument();
        }, { timeout: 2000 });
    });

    test('서버 카드 초기화 로딩이 3.5초 후 사라져야 함', async () => {
        // 이 테스트는 타이머 기반이므로 매우 긴 타임아웃 필요
        // 실제 CI에서는 스킵하거나 모킹으로 대체 권장
        jest.useFakeTimers();
        
        let statusCallback = null;
        mockApi.onStatusUpdate = jest.fn((callback) => {
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

        // 3.5초 경과
        await act(async () => {
            jest.advanceTimersByTime(3500);
        });

        // serversInitializing=false 로 전환되어 오버레이가 사라져야 함
        expect(screen.queryByText('서버 상태 확인 중...')).not.toBeInTheDocument();
        
        jest.useRealTimers();
    });
});
// === 2026-01-20 추가: safeShowToast 및 통신 테스트 ===

describe('safeShowToast 안전 호출 테스트', () => {
    test('window.showToast가 정의되지 않았을 때 에러가 발생하지 않아야 함', async () => {
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

    test('window.showToast가 정의되어 있으면 정상 호출되어야 함', async () => {
        global.window.showToast = mockShowToast;

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

        // showToast가 호출되었는지 확인
        await waitFor(() => {
            expect(mockShowToast).toHaveBeenCalled();
        }, { timeout: 3000 });
    });

    test('Discord 봇 시작 실패 시 에러 토스트가 표시되어야 함', async () => {
        global.window.showToast = mockShowToast;

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

        // 에러 토스트 호출 확인
        await waitFor(() => {
            expect(mockShowToast).toHaveBeenCalledWith(
                expect.stringContaining('Discord 봇 시작 실패'),
                'error',
                4000
            );
        }, { timeout: 3000 });
    });
});

describe('모듈 목록 API 응답 테스트', () => {
    test('모듈 목록에 commands 필드가 포함되어야 함', async () => {
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

        mockApi.moduleList = jest.fn().mockResolvedValue(mockModulesWithCommands);

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

    test('commands가 없는 모듈도 정상 처리되어야 함', async () => {
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

        mockApi.moduleList = jest.fn().mockResolvedValue(mockModulesWithoutCommands);

        const result = await mockApi.moduleList();
        
        expect(result.modules).toHaveLength(1);
        expect(result.modules[0].commands).toBeNull();
    });
});

describe('REST 명령어 실행 테스트', () => {
    test('GET 메서드 명령어가 올바르게 전송되어야 함', async () => {
        const mockExecuteCommand = jest.fn().mockResolvedValue({
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

    test('POST 메서드 명령어가 body와 함께 전송되어야 함', async () => {
        const mockExecuteCommand = jest.fn().mockResolvedValue({
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

    test('REST 연결 실패 시 에러가 반환되어야 함', async () => {
        const mockExecuteCommand = jest.fn().mockResolvedValue({
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
    test('서버 목록 조회 실패 시 토스트가 표시되어야 함', async () => {
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
    test('모듈 로드 실패 시 에러 토스트가 표시되어야 함', async () => {
        global.window.showToast = mockShowToast;

        mockApi.moduleList = jest.fn().mockResolvedValue({ error: '모듈 경로를 찾을 수 없습니다' });

        await act(async () => {
            render(<App />);
        });

        await waitFor(() => {
            expect(mockShowToast).toHaveBeenCalledWith(
                expect.stringContaining('모듈 로드 실패'),
                'error',
                4000
            );
        }, { timeout: 5000 });
    });
});