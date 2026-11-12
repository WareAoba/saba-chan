/**
 * 응답 없는 서버 감시 (데몬 module_settings: unresponsive_action·unresponsive_minutes, 이벤트 instance.unresponsive).
 * 사바쨩 탭에서 할 일과 시간을 고르고, 판정되면 토스트로 알린다.
 */
import { act, render, renderHook, screen } from '@testing-library/react';
import type { ComponentProps } from 'react';
import { beforeEach, describe, expect, it, vi } from 'vitest';
import { ServerSettingsModal } from '../components/ServerSettingsModal';
import { useServerSettings } from '../hooks/useServerSettings';
import i18n from '../i18n';
import { eventToast } from '../utils/daemonEvents';
import { pickOption } from './dropdown';

const tr = i18n.getFixedT('en', 'gui');
const t = (k: string, o?: Record<string, unknown>) => i18n.t(`gui:${k}`, o) as string;
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

describe('응답 없음 감시 설정', () => {
    it('기본은 알림만·10분이고, 바꾼 칸만 데몬에 보낸다', async () => {
        const { result } = setup();
        await act(async () => result.current.handleOpenSettings({ id: 's', name: 'S', module: 'game' } as never));
        expect(result.current.settingsValues).toMatchObject({ unresponsive_action: 'notify', unresponsive_minutes: '10' });
        await act(async () => result.current.handleSaveSettings());
        const untouched = vi.mocked(window.api.instanceUpdateSettings).mock.calls[0][1];
        expect(Object.keys(untouched).filter((k) => k.startsWith('unresponsive'))).toEqual([]);

        act(() => result.current.handleSettingChange('unresponsive_action', 'restart'));
        await act(async () => result.current.handleSaveSettings());
        const sent = vi.mocked(window.api.instanceUpdateSettings).mock.calls[1][1];
        expect(sent).toMatchObject({ unresponsive_action: 'restart' });
        expect(sent).not.toHaveProperty('unresponsive_minutes');
    });

    it('사바쨩 탭에서 할 일을 고르고, 끄면 시간 칸을 못 고친다', () => {
        const handleSettingChange = vi.fn();
        const props = (values: Record<string, unknown>) => ({
            settingsServer: { id: 's', name: 'S', module: 'game', status: 'stopped' },
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
            handleSettingChange,
            handleInstallVersion: vi.fn(),
            handleSaveSettings: vi.fn(),
            handleResetServer: vi.fn(),
            resettingServer: false,
            onClose: vi.fn(),
            servers: [],
        });
        const { unmount } = render(
            <ServerSettingsModal
                {...(props({ unresponsive_action: 'notify', unresponsive_minutes: '10' }) as unknown as ComponentProps<typeof ServerSettingsModal>)}
            />,
        );
        const minutes = () => screen.getByRole('spinbutton', { name: t('server_settings.unresponsive_minutes') }) as HTMLInputElement;
        expect(minutes().value).toBe('10');
        expect(minutes().disabled).toBe(false);
        pickOption(
            screen.getByRole('combobox', { name: t('server_settings.unresponsive_action') }),
            t('server_settings.unresponsive_action_restart'),
        );
        expect(handleSettingChange).toHaveBeenCalledWith('unresponsive_action', 'restart');
        unmount();
        render(
            <ServerSettingsModal
                {...(props({ unresponsive_action: 'off', unresponsive_minutes: '10' }) as unknown as ComponentProps<typeof ServerSettingsModal>)}
            />,
        );
        expect(minutes().disabled).toBe(true);
    });
});

describe('응답 없음 알림', () => {
    it('판정되면 경고 토스트와 서버 알림을 남기고, 다시 시작할 때는 그렇게 알린다', () => {
        const data = { at: 1700000000, instanceId: 'srv-1', instance: 'Survival', minutes: 10, restarting: false };
        const toast = eventToast({ type: 'instance.unresponsive', data }, tr);
        expect(toast).toMatchObject({ type: 'warning', message: tr('events.instance_unresponsive', { name: 'Survival', minutes: 10 }) });
        expect(toast?.notice).toMatchObject({ instanceId: 'srv-1', type: 'error' });
        const restarting = eventToast({ type: 'instance.unresponsive', data: { ...data, restarting: true } }, tr);
        expect(restarting?.message).toBe(tr('events.instance_unresponsive_restart', { name: 'Survival', minutes: 10 }));
    });
});
