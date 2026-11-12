/**
 * 접속자 없을 때 자동 정지 (데몬 module_settings: idle_stop·idle_stop_minutes·idle_stop_grace_minutes).
 * 사바쨩 탭에서 켜고 끄고, 접속자 수를 모르는 게임에서는 고를 수 없다. 자동으로 정지한 서버는 이유를 알린다.
 */
import { act, render, renderHook, screen } from '@testing-library/react';
import type { ComponentProps } from 'react';
import { beforeEach, describe, expect, it, vi } from 'vitest';
import { ServerSettingsModal } from '../components/ServerSettingsModal';
import { useServerSettings } from '../hooks/useServerSettings';
import i18n from '../i18n';

const t = (k: string, o?: Record<string, unknown>) => i18n.t(`gui:${k}`, o ?? {}) as string;
const modules = [{ name: 'game', settings: { fields: [{ name: 'title', field_type: 'text' }] } }];

const setup = () =>
    renderHook(() =>
        useServerSettings({ servers: [], modules: modules as never, setModal: vi.fn(), setProgressBar: vi.fn(), fetchServers: vi.fn() }),
    );

beforeEach(() => {
    window.showToast = vi.fn();
    window.api = {
        serverList: vi.fn().mockResolvedValue(null),
        instanceReadLiveSettings: vi.fn().mockResolvedValue({ success: true, settings: { title: 'x' } }),
        instanceUpdateSettings: vi.fn().mockResolvedValue({ success: true }),
    } as unknown as typeof window.api;
});

describe('접속자 없을 때 자동 정지 설정', () => {
    it('기본은 꺼짐·30분·15분이고, 바꾼 칸만 데몬에 보낸다', async () => {
        const { result } = setup();
        await act(async () => result.current.handleOpenSettings({ id: 's', name: 'S', module: 'game' } as never));
        expect(result.current.settingsValues).toMatchObject({ idle_stop: false, idle_stop_minutes: '30', idle_stop_grace_minutes: '15' });
        await act(async () => result.current.handleSaveSettings());
        const untouched = vi.mocked(window.api.instanceUpdateSettings).mock.calls[0][1];
        expect(Object.keys(untouched).filter((k) => k.startsWith('idle_stop'))).toEqual([]);

        act(() => result.current.handleSettingChange('idle_stop', true));
        act(() => result.current.handleSettingChange('idle_stop_minutes', '45'));
        await act(async () => result.current.handleSaveSettings());
        const sent = vi.mocked(window.api.instanceUpdateSettings).mock.calls[1][1];
        expect(sent).toMatchObject({ idle_stop: true, idle_stop_minutes: '45' });
        expect(sent).not.toHaveProperty('idle_stop_grace_minutes');
    });

    const props = (supported: boolean, values: Record<string, unknown>) => ({
        settingsServer: { id: 's', name: 'S', module: 'game', status: 'stopped', idle_stop_supported: supported },
        settingsValues: values,
        settingsSource: 'live',
        settingsActiveTab: 'general',
        setSettingsActiveTab: vi.fn(),
        modules,
        advancedExpanded: false,
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
        servers: [],
    });

    it('켜면 기다리는 시간과 켠 직후 기다리기를 고친다', () => {
        const p = props(true, { idle_stop: true, idle_stop_minutes: '30', idle_stop_grace_minutes: '15' });
        render(<ServerSettingsModal {...(p as unknown as ComponentProps<typeof ServerSettingsModal>)} />);
        expect((screen.getByRole('checkbox', { name: t('server_settings.idle_stop_label') }) as HTMLInputElement).checked).toBe(true);
        const wait = screen.getByRole('spinbutton', { name: t('server_settings.idle_stop_minutes') }) as HTMLInputElement;
        expect(wait.value).toBe('30');
        expect(wait.disabled).toBe(false);
        expect((screen.getByRole('spinbutton', { name: t('server_settings.idle_stop_grace') }) as HTMLInputElement).value).toBe('15');
    });

    it('접속자 수를 모르는 게임에서는 고를 수 없고 이유를 알린다', () => {
        const p = props(false, { idle_stop: false, idle_stop_minutes: '30', idle_stop_grace_minutes: '15' });
        render(<ServerSettingsModal {...(p as unknown as ComponentProps<typeof ServerSettingsModal>)} />);
        expect((screen.getByRole('checkbox', { name: t('server_settings.idle_stop_label') }) as HTMLInputElement).disabled).toBe(true);
        expect(screen.getByText(t('server_settings.idle_stop_unsupported'))).toBeTruthy();
        expect(screen.queryByRole('spinbutton', { name: t('server_settings.idle_stop_minutes') })).toBeNull();
    });
});
