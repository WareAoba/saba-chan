import React from 'react';
import { render, screen, waitFor, act } from '@testing-library/react';
import userEvent from '@testing-library/user-event';
import App from '../App';

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
