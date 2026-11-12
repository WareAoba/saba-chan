import type { ComponentProps } from 'react';
import { act, fireEvent, render, renderHook, screen, within } from '@testing-library/react';
import { describe, expect, it, vi } from 'vitest';
import { ServerSettingsModal } from '../components/ServerSettingsModal';
import { useServerSettings } from '../hooks/useServerSettings';

// 저장 막대의 "N개 바뀜" 칩: 누르면 칸마다 이전 → 새 값과 칸 하나 되돌리기를 보인다 (사용자 11-12)
const modules = [
    {
        name: 'dl',
        settings: {
            fields: [
                { name: 'motd', label: 'MOTD', field_type: 'text' },
                { name: 'port', label: 'Port', field_type: 'number', restart_required: true },
                { name: 'secret', label: 'Secret', field_type: 'password' },
            ],
        },
    },
];
const server = { id: 's1', name: 'S', module: 'dl', status: 'stopped', module_settings: { motd: 'a', port: 1 } };
const make = (settingsValues: Record<string, unknown>, extra: Record<string, unknown> = {}) =>
    ({
        section: 'server',
        settingsServer: server,
        settingsValues,
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
        handleSettingLoaded: vi.fn(),
        revertSettings: vi.fn(),
        openedSettings: { motd: 'a', port: '1', secret: 'old' },
        handleInstallVersion: vi.fn(),
        handleSaveSettings: vi.fn(),
        handleResetServer: vi.fn(),
        resettingServer: false,
        servers: [server],
        ...extra,
    }) as unknown as ComponentProps<typeof ServerSettingsModal>;

describe('저장 막대의 바뀐 칸 목록', () => {
    it('칩에 바뀐 칸 수를 보이고, 누르면 이전 값과 새 값·다시 시작 표시를 보인다', () => {
        render(<ServerSettingsModal {...make({ motd: 'b', port: '2', secret: 'new' })} />);
        fireEvent.click(screen.getByRole('button', { name: '3 changed' }));
        const list = screen.getByRole('dialog', { name: 'Changed fields: 3' });
        const rows = within(list).getAllByRole('listitem');
        expect(rows.map((r) => r.textContent)).toEqual([
            expect.stringMatching(/^MOTDa → b/),
            expect.stringMatching(/^PortApplies after restart1 → 2/),
            // 비밀 값은 글자를 보이지 않는다
            expect.stringMatching(/^Secret•••• → ••••/),
        ]);
    });

    it('칸 하나만 되돌린다', () => {
        const revertSetting = vi.fn();
        render(<ServerSettingsModal {...make({ motd: 'b', port: '2', secret: 'old' }, { revertSetting })} />);
        fireEvent.click(screen.getByRole('button', { name: '2 changed' }));
        fireEvent.click(screen.getByRole('button', { name: 'Revert Port' }));
        expect(revertSetting).toHaveBeenCalledWith('port');
    });

    it('되돌린 칸은 기준 값으로 가고 다른 칸은 고친 값 그대로다', async () => {
        window.showToast = vi.fn();
        window.api = { serverList: vi.fn().mockResolvedValue({ servers: [server] }) } as unknown as typeof window.api;
        const { result } = renderHook(() =>
            useServerSettings({
                servers: [server] as never,
                modules: modules as never,
                setModal: vi.fn(),
                setProgressBar: vi.fn(),
                fetchServers: vi.fn(),
            }),
        );
        await act(async () => result.current.handleOpenSettings(server as never));
        act(() => result.current.handleSettingChange('motd', 'b'));
        act(() => result.current.handleSettingChange('port', '2'));
        act(() => result.current.revertSetting('port'));
        expect(result.current.settingsValues).toMatchObject({ motd: 'b', port: '1' });
        expect(result.current.unsavedServers).toEqual(['s1']);
        act(() => result.current.revertSetting('motd'));
        expect(result.current.unsavedServers).toEqual([]);
    });
});
