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

import SettingsModal from '../components/Modals/SettingsModal';
import SabaStorage from '../components/Modals/SabaStorage';
import { APP_PAGES, AppBar } from '../components/shell/AppBar';

beforeEach(() => {
    toggleExtension.mockClear();
    window.api = new Proxy(
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
    );
});

afterEach(() => {
    delete window.api;
});

describe('설정에서 익스텐션 탭을 뺀다', () => {
    it('설정 탭 줄에 익스텐션 탭이 없다', () => {
        render(<SettingsModal isOpen onClose={vi.fn()} variant="page" />);
        const labels = [...document.querySelectorAll('.settings-tab')].map((b) => b.textContent);
        expect(labels).toEqual(
            ['general', 'appearance', 'advanced_tab'].map((k) => i18n.t(`gui:settings_modal.${k}`)),
        );
    });

    it("'익스텐션'으로 열면 사바 스토리지 익스텐션 탭이 열린다", async () => {
        render(<SettingsModal isOpen onClose={vi.fn()} variant="page" initialView="extensions" />);
        await waitFor(() => expect(document.querySelector('.update-panel')).not.toBeNull());
        const active = document.querySelector('.saba-storage-tab.active');
        expect(active.textContent).toBe(i18n.t('gui:saba_storage.tab_extensions'));
    });
});

describe('사바 스토리지 익스텐션 탭에서 켜고 끈다', () => {
    async function openExtensions() {
        render(<SabaStorage onBack={() => {}} isExiting={false} initialTab="extensions" />);
        await act(async () => {});
    }

    it('익스텐션마다 켜기 스위치가 있고 누르면 켜고 끈다', async () => {
        await openExtensions();
        const toggle = screen.getByRole('checkbox', { name: 'SteamCMD' });
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
    it('설정 > 고급에 작업 기록을 펼쳐 두지 않는다', async () => {
        render(<SettingsModal isOpen onClose={vi.fn()} variant="page" />);
        const advanced = [...document.querySelectorAll('.settings-tab')].find(
            (b) => b.textContent === i18n.t('gui:settings_modal.advanced_tab'),
        );
        await act(async () => fireEvent.click(advanced));
        expect(document.querySelector('.settings-audit')).toBeNull();
        expect(window.api.auditList).not.toHaveBeenCalled();
    });

    it('상단 탭에 작업 기록 페이지가 있다', () => {
        expect(APP_PAGES.map((p) => p.id)).toContain('activity');
        const onNavigate = vi.fn();
        render(<AppBar page="servers" onNavigate={onNavigate} noticeProps={{}} daemonProps={{}} />);
        fireEvent.click(screen.getByRole('button', { name: i18n.t('gui:shell.nav_activity') }));
        expect(onNavigate).toHaveBeenCalledWith('activity');
    });
});
