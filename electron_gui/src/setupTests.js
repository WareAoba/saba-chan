// jest-dom adds custom jest matchers for asserting on DOM nodes.
// allows you to do things like:
// expect(element).toHaveTextContent(/react/i)
// learn more: https://github.com/testing-library/jest-dom
import '@testing-library/jest-dom';

// 전역 테스트 타임아웃 설정 (10초)
jest.setTimeout(10000);

// 테스트 환경에서 디버깅 로그 억제 (에러는 유지)
const originalConsoleLog = console.log;
const originalConsoleWarn = console.warn;

// 특정 패턴의 로그만 억제
console.log = (...args) => {
    const msg = args.join(' ');
    // 중요 테스트 결과는 표시, 디버깅 로그는 억제
    if (!msg.includes('[Settings]') && 
        !msg.includes('[Auto-start]') && 
        !msg.includes('[Init]') &&
        !msg.includes('Fetching modules') &&
        !msg.includes('Module data received') &&
        !msg.includes('App mounted')) {
        originalConsoleLog(...args);
    }
};

console.warn = (...args) => {
    const msg = args.join(' ');
    // retry 로그만 억제
    if (!msg.includes('Attempt') && 
        !msg.includes('failed, retrying') &&
        !msg.includes('Daemon not ready')) {
        originalConsoleWarn(...args);
    }
};

// Mock window.api globally (jsdom 환경에서만)
if (typeof window !== 'undefined') {
    global.window.api = {
        settingsLoad: jest.fn(),
        settingsSave: jest.fn(),
        settingsGetPath: jest.fn(),
        botConfigLoad: jest.fn(),
        botConfigSave: jest.fn(),
        discordBotStatus: jest.fn(),
        discordBotStart: jest.fn(),
        discordBotStop: jest.fn(),
        serverList: jest.fn(),          // 추가
        moduleList: jest.fn(),          // 추가
        getServers: jest.fn(),
        getModules: jest.fn(),
    };
    
    global.window.showToast = jest.fn();
    global.window.showStatus = jest.fn();
}
