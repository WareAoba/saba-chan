// jest-dom adds custom jest matchers for asserting on DOM nodes.
// allows you to do things like:
// expect(element).toHaveTextContent(/react/i)
// learn more: https://github.com/testing-library/jest-dom
import '@testing-library/jest-dom';

// Mock window.api globally
global.window.api = {
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

global.window.showToast = jest.fn();
global.window.showStatus = jest.fn();
