/**
 * 설정 정리 (사용자 10-26):
 * - 설정의 익스텐션 탭을 없앤다. 켜기·끄기와 익스텐션 설정은 사바 스토리지 익스텐션 탭에서 한다.
 * - 작업 기록은 설정 > 고급에 펼쳐 두지 않고 상단 탭의 별도 페이지로 둔다.
 */
import { act, fireEvent, render, screen, waitFor } from '@testing-library/react';
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import i18n from '../i18n';

const toggleExtension = vi.fn().mockResolvedValue(true);
const extensions = [
    {
        id: 'ue4-ini',
        name: 'UE4 INI',
        version: '0.1.0',
        enabled: true,
        config_fields: { path: { type: 'string' } },
    },
    { id: 'steamcmd', name: 'SteamCMD', version: '0.1.0', enabled: false },
];
vi.mock('../contexts/ExtensionContext', () => ({
    useExtensions: () => ({
        extensions,
        toggleExtension,
        refreshExtensions: vi.fn(),
        manifestExtensions: [],
        availableUpdates: [],
        installingIds: new Set(),
        fetchManifest: vi.fn(),
        installExtension: vi.fn(),
        removeExtension: vi.fn(),
        guiPending: [],
        slots: {},
        approveExtensionGui: vi.fn(),
    }),
}));

import type { ComponentProps, ComponentType } from 'react';
import SettingsModalFull from '../components/Modals/SettingsModal';
import SabaStorage from '../components/Modals/SabaStorage';
import { APP_PAGES, AppBar as AppBarFull } from '../components/shell/AppBar';

// 이 시험이 보는 props만 넘기므로 나머지는 선택으로 느슨하게 받는다
const SettingsModal = SettingsModalFull as unknown as ComponentType<Partial<ComponentProps<typeof SettingsModalFull>>>;
const AppBar = AppBarFull as unknown as ComponentType<Partial<ComponentProps<typeof AppBarFull>>>;

beforeEach(() => {
    toggleExtension.mockClear();
    window.api = new Proxy<Record<string | symbol, unknown>>(
        {
            auditList: vi.fn().mockResolvedValue({ entries: [] }),
            extensionConfigGet: vi.fn().mockResolvedValue({ config: {} }),
            extensionConfigSet: vi.fn().mockResolvedValue({}),
        },
        {
            get(target, prop) {
                if (typeof prop === 'symbol') return target[prop];
                if (!(prop in target)) target[prop] = vi.fn().mockResolvedValue({});
                return target[prop];
            },
        },
    ) as unknown as typeof window.api;
});

afterEach(() => {
    delete (window as { api?: unknown }).api;
});

describe('설정에서 익스텐션 탭을 뺀다', () => {
    it('설정 탭 줄에 익스텐션 탭이 없다', () => {
        render(<SettingsModal isOpen onClose={vi.fn()} variant="page" />);
        const labels = [...document.querySelectorAll('.settings-tab')].map((b) => b.textContent);
        expect(labels).toEqual(
            [
                'general',
                'tab_notify',
                'appearance',
                'tab_console',
                'tab_servers',
                'tab_connection',
                'tab_storage',
                'tab_security',
                'tab_troubleshoot',
            ].map((k) => i18n.t(`gui:settings_modal.${k}`)),
        );
    });

    // 사용자 요청(2026-11-06): 고급에 몰린 설정을 여러 탭으로 나눈다 (옮기기만, 설정 자체는 그대로)
    it('설정을 갈래별 탭에 나눠 둔다', async () => {
        render(<SettingsModal isOpen onClose={vi.fn()} variant="page" />);
        const open = async (key: string) => {
            const tab = screen.getByRole('tab', { name: i18n.t(`gui:settings_modal.${key}`) });
            await act(async () => fireEvent.click(tab));
            expect(tab.getAttribute('aria-selected')).toBe('true');
        };
        const has = (key: string) => expect(screen.queryByText(i18n.t(`gui:settings_modal.${key}`))).not.toBeNull();
        const lacks = (key: string) => expect(screen.queryByText(i18n.t(`gui:settings_modal.${key}`))).toBeNull();
        has('language_label');
        lacks('os_notifications_label');
        await open('tab_notify');
        has('os_notifications_label');
        has('sound_effects_label');
        await open('tab_console');
        has('console_fonts_label');
        has('console_buffer_label');
        lacks('theme_label');
        await open('tab_servers');
        has('auto_generate_passwords_label');
        has('audit_keep_label');
        await open('tab_connection');
        has('ipc_port_label');
        has('relay_url_label');
        await open('tab_troubleshoot');
        has('uninstall_label');
        lacks('ipc_port_label');
    });

    it("'익스텐션'으로 열면 사바 스토리지 익스텐션 탭이 열린다", async () => {
        render(<SettingsModal isOpen onClose={vi.fn()} variant="page" initialView="extensions" />);
        await waitFor(() => expect(document.querySelector('.update-panel')).not.toBeNull());
        const active = document.querySelector('.saba-storage-tab.active');
        expect(active!.textContent).toBe(i18n.t('gui:saba_storage.tab_extensions'));
    });
});

describe('사바 스토리지 익스텐션 탭에서 켜고 끈다', () => {
    async function openExtensions() {
        render(<SabaStorage onBack={() => {}} isExiting={false} initialTab="extensions" />);
        await act(async () => {});
    }

    it('익스텐션마다 켜기 스위치가 있고 누르면 켜고 끈다', async () => {
        await openExtensions();
        const toggle = screen.getByRole<HTMLInputElement>('checkbox', { name: 'SteamCMD' });
        expect(toggle.checked).toBe(false);
        await act(async () => fireEvent.click(toggle));
        expect(toggleExtension).toHaveBeenCalledWith('steamcmd', true);
    });

    it('켜진 익스텐션은 설정 칸을 같은 자리에서 연다', async () => {
        await openExtensions();
        expect(screen.getByText(i18n.t('gui:extension_config.toggle'))).toBeTruthy();
    });
});

describe('작업 기록은 별도 페이지', () => {
    it('설정 > 문제 해결에 작업 기록을 펼쳐 두지 않는다', async () => {
        render(<SettingsModal isOpen onClose={vi.fn()} variant="page" />);
        const tab = [...document.querySelectorAll('.settings-tab')].find(
            (b) => b.textContent === i18n.t('gui:settings_modal.tab_troubleshoot'),
        );
        await act(async () => fireEvent.click(tab!));
        expect(document.querySelector('.settings-audit')).toBeNull();
        expect(window.api.auditList).not.toHaveBeenCalled();
    });

    it('상단 탭에 작업 기록 페이지가 있다', () => {
        expect(APP_PAGES.map((p) => p.id)).toContain('activity');
        const onNavigate = vi.fn();
        render(<AppBar page="servers" onNavigate={onNavigate} noticeProps={{} as never} daemonProps={{} as never} />);
        fireEvent.click(screen.getByRole('button', { name: i18n.t('gui:shell.nav_activity') }));
        expect(onNavigate).toHaveBeenCalledWith('activity');
    });
});
