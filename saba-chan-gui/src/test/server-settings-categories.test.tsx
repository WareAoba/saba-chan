import type { ComponentProps } from 'react';
import { fireEvent, render, screen } from '@testing-library/react';
import { describe, expect, it, vi } from 'vitest';
import { ServerSettingsModal } from '../components/ServerSettingsModal';

// 서버 설정 탭: 기본 설정 다음에 앱이 정한 공통 카테고리마다 접는 카드, 모르는 group은 고급 카드로
const modules = [
    {
        name: 'g',
        settings: {
            fields: [
                { name: 'motd', label: 'MOTD', field_type: 'text' },
                { name: 'tick', label: 'Tick', field_type: 'number', group: 'advanced' },
                { name: 'friendly_fire', label: 'Friendly fire', field_type: 'boolean', group: 'pvp' },
                { name: 'max_players_guild', label: 'Guild size', field_type: 'number', group: 'player' },
                { name: 'odd', label: 'Odd', field_type: 'text', group: 'made_up' },
            ],
        },
    },
];
const server = { id: 's1', name: 'S', module: 'g', status: 'stopped' };
const props = (openGroups: string[], setGroupOpen = vi.fn()) =>
    ({
        section: 'server',
        embedded: true,
        settingsServer: server,
        settingsValues: {},
        settingsSource: 'live',
        settingsActiveTab: 'general',
        setSettingsActiveTab: vi.fn(),
        modules,
        openGroups,
        setGroupOpen,
        availableVersions: [],
        handleSettingChange: vi.fn(),
        handleInstallVersion: vi.fn(),
        handleSaveSettings: vi.fn(),
        handleResetServer: vi.fn(),
        onClose: vi.fn(),
        servers: [server],
    }) as unknown as ComponentProps<typeof ServerSettingsModal>;

const titles = () => [...document.querySelectorAll('.ss-basic .ss-card-title, .ss-fold-card .ss-card-title')].map((h) => h.textContent?.replace(/\d+$/, '').trim());

describe('서버 설정 카테고리 카드', () => {
    it('공통 카테고리 순서로 카드를 그리고, 모르는 group은 고급 카드에 넣는다', () => {
        render(<ServerSettingsModal {...props(['advanced'])} />);
        expect(titles()).toEqual(['Server Settings', 'Player', 'PvP', 'Advanced Settings']);
        const adv = document.querySelector('[data-group="advanced"]') as HTMLElement;
        expect(adv.textContent).toContain('Tick');
        expect(adv.textContent).toContain('Odd');
    });

    it('카드는 접혀 있다가 머리를 누르면 그 카테고리만 펼친다', () => {
        const setGroupOpen = vi.fn();
        render(<ServerSettingsModal {...props([], setGroupOpen)} />);
        const pvp = screen.getByRole('button', { name: /PvP/ });
        expect(pvp.getAttribute('aria-expanded')).toBe('false');
        fireEvent.click(pvp);
        expect(setGroupOpen).toHaveBeenCalledWith('pvp', true);
    });
});
