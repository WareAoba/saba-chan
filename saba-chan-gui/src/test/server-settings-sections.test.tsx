import type { ComponentProps } from 'react';
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
const props = (section: string) => ({
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

const has = (sel: string) => !!document.querySelector(sel);

describe('서버 설정 / 사바쨩 설정 나누기', () => {
    it('설정 탭에는 게임 서버 설정만 있다', () => {
        render(<ServerSettingsModal {...(props('server') as unknown as ComponentProps<typeof ServerSettingsModal>)} />);
        expect(screen.getByText('MOTD')).toBeTruthy();
        expect(screen.getByText('View distance')).toBeTruthy();
        expect(has('.ss-reset')).toBe(true);
        expect(screen.queryByText('Java path')).toBeNull();
        for (const sel of [
            '.ss-restart',
            '.ss-memory',
            '.ss-game-chat',
            '.ss-stop-timeout',
            '.ss-version-install',
            '.ss-tabs',
        ]) {
            expect(has(sel), sel).toBe(false);
        }
    });

    it('사바쨩 탭에는 사바쨩이 서버를 다루는 설정만 있다', () => {
        render(<ServerSettingsModal {...(props('saba') as unknown as ComponentProps<typeof ServerSettingsModal>)} />);
        expect(screen.getByText('Java path')).toBeTruthy();
        for (const sel of [
            '.ss-restart',
            '.ss-memory',
            '.ss-game-chat',
            '.ss-stop-timeout',
            '.ss-version-install',
        ]) {
            expect(has(sel), sel).toBe(true);
        }
        expect(screen.queryByText('MOTD')).toBeNull();
        expect(screen.queryByText('View distance')).toBeNull();
        expect(has('.ss-reset')).toBe(false);
        // 조작 방식 안내와 "일반 설정" 탭 단추는 없다. 사바쨩 그룹(Java path·버전)은 다른 사바쨩 설정보다 위에 온다
        expect(has('.protocol-mode-info')).toBe(false);
        expect(has('.ss-tabs')).toBe(false);
        const form = document.querySelector('.ss-form') as HTMLElement;
        expect(form.firstElementChild?.classList.contains('ss-saba')).toBe(true);
    });
});
