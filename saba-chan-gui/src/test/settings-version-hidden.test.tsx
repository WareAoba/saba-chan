import type { ComponentProps } from 'react';
import { fireEvent, render, screen } from '@testing-library/react';
import { describe, expect, it, vi } from 'vitest';
import { ServerSettingsModal } from '../components/ServerSettingsModal';

// 시안 12: 설치된 버전에 없는 설정(데몬이 서버 목록의 version_hidden으로 알린다)은 설정 탭에서 그리지 않고 한 줄로 알린다
const modules = [
    {
        name: 'mc',
        install: { method: 'download' },
        protocols: { supported: ['stdin'] },
        capabilities: [],
        settings: {
            fields: [
                { name: 'pvp', label: 'PVP', field_type: 'boolean', versions: '<1.21.9' },
                { name: 'allow_nether', label: 'Allow Nether', field_type: 'boolean', versions: '<1.21.9' },
                { name: 'difficulty', label: 'Difficulty', field_type: 'text' },
                { name: 'spawn_animals', label: 'Spawn Animals', field_type: 'boolean', group: 'advanced', versions: '<1.21.2' },
            ],
        },
    },
];
const props = (server: Record<string, unknown>) => ({
    section: 'server',
    embedded: true,
    settingsServer: server,
    settingsValues: {},
    settingsSource: 'live',
    settingsActiveTab: 'general',
    setSettingsActiveTab: vi.fn(),
    modules,
    advancedExpanded: true,
    setAdvancedExpanded: vi.fn(),
    availableVersions: [],
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
const show = (server: Record<string, unknown>) =>
    render(<ServerSettingsModal {...(props(server) as unknown as ComponentProps<typeof ServerSettingsModal>)} />);

describe('설치된 버전에 없는 설정', () => {
    it('옛 버전 서버는 지금처럼 모든 설정을 보인다', () => {
        const { container } = show({ id: 's1', name: 'Old', module: 'mc', status: 'stopped', server_version: '1.21.8' });
        for (const label of ['PVP', 'Allow Nether', 'Difficulty', 'Spawn Animals']) expect(screen.getByText(label)).toBeTruthy();
        expect(container.querySelector('.ss-version-hidden')).toBeNull();
    });

    it('숨기고, 몇 개를 왜 숨겼는지 한 줄로 알리며, 목록 보기로 이름을 보인다', () => {
        const { container } = show({
            id: 's2',
            name: 'New',
            module: 'mc',
            status: 'stopped',
            server_version: '1.21.10',
            version_hidden: ['pvp', 'allow_nether', 'spawn_animals'],
        });
        expect(screen.getByText('Difficulty')).toBeTruthy();
        for (const label of ['PVP', 'Allow Nether', 'Spawn Animals']) expect(screen.queryByText(label)).toBeNull();
        const notice = container.querySelector('.ss-version-hidden') as HTMLElement;
        expect(notice.textContent).toContain('3 setting(s) missing in the installed version (1.21.10) are hidden');
        const list = screen.getByRole('button', { name: 'Show list' });
        expect(list.getAttribute('aria-expanded')).toBe('false');
        fireEvent.click(list);
        expect(list.getAttribute('aria-expanded')).toBe('true');
        expect(notice.textContent).toContain('PVP, Allow Nether, Spawn Animals');
    });
});
