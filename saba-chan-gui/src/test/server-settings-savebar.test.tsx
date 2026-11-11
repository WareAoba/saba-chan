import type { ComponentProps } from 'react';
import { fireEvent, render, screen } from '@testing-library/react';
import { describe, expect, it, vi } from 'vitest';
import { ServerSettingsModal } from '../components/ServerSettingsModal';

// 서버 설정도 바뀐 게 있을 때만 공통 저장 막대가 올라와 저장·되돌리기를 묻는다 (사용자 11-11)
const modules = [
    {
        name: 'dl',
        install: { method: 'download' },
        settings: { fields: [{ name: 'motd', label: 'MOTD', field_type: 'text' }] },
    },
];
const server = { id: 's1', name: 'S', module: 'dl', status: 'stopped' };
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
        openedSettings: { motd: 'a' },
        handleInstallVersion: vi.fn(),
        handleSaveSettings: vi.fn(),
        handleResetServer: vi.fn(),
        resettingServer: false,
        servers: [server],
        ...extra,
    }) as unknown as ComponentProps<typeof ServerSettingsModal>;

describe('서버 설정 저장 막대', () => {
    it('바뀐 게 없으면 막대가 없다', () => {
        render(<ServerSettingsModal {...make({ motd: 'a' })} />);
        expect(document.querySelector('.saba-savebar')).toBeNull();
        expect(screen.queryByRole('button', { name: /Save Settings/ })).toBeNull();
    });

    it('연 때 값과 다르면 막대가 올라와 저장하고, 되돌리기는 설정 훅에 맡긴다', () => {
        const revertSettings = vi.fn();
        const handleSaveSettings = vi.fn();
        render(<ServerSettingsModal {...make({ motd: 'b' }, { revertSettings, handleSaveSettings })} />);
        expect(document.querySelector('.saba-savebar')).not.toBeNull();
        fireEvent.click(screen.getByRole('button', { name: /Save Settings/ }));
        expect(handleSaveSettings).toHaveBeenCalledTimes(1);
        fireEvent.click(screen.getByRole('button', { name: 'Revert' }));
        expect(revertSettings).toHaveBeenCalledTimes(1);
    });

    it('다시 그려져도(탭 전환) 기준은 그대로라 막대가 남는다', () => {
        const { unmount } = render(<ServerSettingsModal {...make({ motd: 'b' })} />);
        unmount();
        render(<ServerSettingsModal {...make({ motd: 'b' })} />);
        expect(document.querySelector('.saba-savebar')).not.toBeNull();
    });

    it('추가 실행 인자만 바꿔도 막대가 올라온다', () => {
        render(<ServerSettingsModal {...make({ motd: 'a', _launch_extra_args: '-x' })} />);
        expect(document.querySelector('.saba-savebar')).not.toBeNull();
    });
});
