import { render, screen } from '@testing-library/react';
import { describe, expect, it, vi } from 'vitest';
import { ServerSettingsModal } from '../components/ServerSettingsModal';

// 서버 상세: "설정" 탭은 게임 서버 설정만, 사바쨩이 서버를 다루는 설정은 "사바쨩" 탭에 모은다
const modules = [
    {
        name: 'dl',
        install: { method: 'download' },
        protocols: { supported: ['rcon'] },
        capabilities: ['list_versions', 'install_server', 'reset_server'],
        settings: {
            fields: [
                { name: 'java_path', label: 'Java path', field_type: 'text', group: 'saba-chan' },
                { name: 'motd', label: 'MOTD', field_type: 'text' },
                { name: 'view_distance', label: 'View distance', field_type: 'number', group: 'advanced' },
            ],
        },
    },
];
const server = { id: 's1', name: 'S', module: 'dl', status: 'stopped', game_chat_supported: true };
const props = (section) => ({
    section,
    embedded: true,
    settingsServer: server,
    settingsValues: {},
    settingsSource: 'live',
    settingsActiveTab: 'general',
    setSettingsActiveTab: vi.fn(),
    modules,
    advancedExpanded: true,
    setAdvancedExpanded: vi.fn(),
    availableVersions: ['1.0'],
    versionsLoading: false,
    versionInstalling: false,
    handleSettingChange: vi.fn(),
    handleInstallVersion: vi.fn(),
    handleSaveSettings: vi.fn(),
    handleResetServer: vi.fn(),
    resettingServer: false,
    onClose: vi.fn(),
    servers: [server],
});

const has = (sel) => !!document.querySelector(sel);

describe('서버 설정 / 사바쨩 설정 나누기', () => {
    it('설정 탭에는 게임 서버 설정만 있다', () => {
        render(<ServerSettingsModal {...props('server')} />);
        expect(screen.getByText('MOTD')).toBeTruthy();
        expect(screen.getByText('View distance')).toBeTruthy();
        expect(has('.danger-zone-btn')).toBe(true);
        expect(screen.queryByText('Java path')).toBeNull();
        for (const sel of [
            '.protocol-mode-info',
            '.restart-section',
            '.memory-limit-section',
            '.game-chat-section',
            '.stop-timeout-section',
            '.version-install-btn',
            '.settings-tabs',
        ]) {
            expect(has(sel), sel).toBe(false);
        }
    });

    it('사바쨩 탭에는 사바쨩이 서버를 다루는 설정만 있다', () => {
        render(<ServerSettingsModal {...props('saba')} />);
        expect(screen.getByText('Java path')).toBeTruthy();
        for (const sel of [
            '.protocol-mode-info',
            '.restart-section',
            '.memory-limit-section',
            '.game-chat-section',
            '.stop-timeout-section',
            '.version-install-btn',
        ]) {
            expect(has(sel), sel).toBe(true);
        }
        expect(screen.queryByText('MOTD')).toBeNull();
        expect(screen.queryByText('View distance')).toBeNull();
        expect(has('.danger-zone-btn')).toBe(false);
    });
});
