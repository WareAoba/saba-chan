// vitest-dom adds custom matchers for asserting on DOM nodes.
// allows you to do things like:
// expect(element).toHaveTextContent(/react/i)
// learn more: https://github.com/testing-library/jest-dom
import '@testing-library/jest-dom';
import { expect, afterEach, vi } from 'vitest';
import { cleanup } from '@testing-library/react';
import i18n from 'i18next';
import { initReactI18next } from 'react-i18next';
import enCommon from '../../locales/en/common.json';
import enGui from '../../locales/en/gui.json';
import koCommon from '../../locales/ko/common.json';
import koGui from '../../locales/ko/gui.json';

// Cleanup after each test case (e.g. clearing jsdom)
afterEach(() => {
  cleanup();
});

// i18n 초기화 (테스트 환경용 - 영어로 설정)
const initializeTestI18n = async () => {
  if (!i18n.isInitialized) {
    await i18n
      .use(initReactI18next)
      .init({
        resources: {
          en: {
            common: enCommon,
            gui: enGui,
          },
          ko: {
            common: koCommon,
            gui: koGui,
          },
        },
        lng: 'en', // 테스트 환경은 영어로 설정
        fallbackLng: 'en',
        defaultNS: 'gui',
        interpolation: {
          escapeValue: false,
        },
      });
  }
};

// 동기 초기화 (테스트 시작 전)
initializeTestI18n();

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
        settingsLoad: vi.fn(),
        settingsSave: vi.fn(),
        settingsGetPath: vi.fn(),
        botConfigLoad: vi.fn(),
        botConfigSave: vi.fn(),
        discordBotStatus: vi.fn(),
        discordBotStart: vi.fn(),
        discordBotStop: vi.fn(),
        serverList: vi.fn(),          // 추가
        moduleList: vi.fn(),          // 추가
        getServers: vi.fn(),
        getModules: vi.fn(),
    };
    
    global.window.showToast = vi.fn();
    global.window.showStatus = vi.fn();
}
