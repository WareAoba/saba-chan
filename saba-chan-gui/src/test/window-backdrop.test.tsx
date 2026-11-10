/**
 * 창 바탕 반투명 (사용자 11-08): 설정 → 외관에서 켜고 끄고, 재질·진하기·밝기를 고른다.
 * 셸이 OS 효과를 깔았다고 답한 창만 바탕을 비우고(data-backdrop="on"), 그 위에 테마 바탕을 고른 진하기로 덮는다.
 */
import { act, fireEvent, render, screen } from '@testing-library/react';
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import type { ComponentProps, ComponentType } from 'react';
import SettingsModalImpl from '../components/Modals/SettingsModal';
import i18n from 'i18next';
import { useSettingsStore } from '../stores/useSettingsStore';
import { WINDOW_BACKDROP_DEFAULTS, applyWindowBackdrop, backdropVars } from '../utils/themeManager';

/** 시험에 필요한 props만 넘기려고 느슨하게 받는다 */
const SettingsModal = SettingsModalImpl as ComponentType<Partial<ComponentProps<typeof SettingsModalImpl>>>;

const WIN11 = {
    platform: 'windows',
    available: true,
    choices: ['tabbed', 'mica', 'acrylic'],
    slow: [],
    blur: 'system',
    liveToggle: true,
    minOpacity: 0,
    transparent: true,
};

function mockApi(info: Record<string, unknown>) {
    const windowSetBackdrop = vi.fn(async (config: { enabled: boolean }) => ({
        ...info,
        active: config.enabled && !!info.available,
    }));
    window.api = new Proxy<Record<string | symbol, unknown>>(
        {
            windowSetBackdrop,
            settingsLoad: vi.fn().mockResolvedValue({}),
            settingsSave: vi.fn().mockResolvedValue({ success: true }),
            settingsGetPath: vi.fn().mockResolvedValue('/mock/config.json'),
        },
        {
            get(target, prop) {
                if (typeof prop === 'symbol') return target[prop];
                if (!(prop in target)) target[prop] = vi.fn().mockResolvedValue({});
                return target[prop];
            },
        },
    ) as unknown as typeof window.api;
    return windowSetBackdrop;
}

describe('window backdrop', () => {
    beforeEach(() => {
        useSettingsStore.getState()._resetForTest();
        delete document.documentElement.dataset.backdrop;
    });
    afterEach(() => vi.restoreAllMocks());

    it('defaults to on with tabbed glass at 60% and neutral brightness', () => {
        expect(WINDOW_BACKDROP_DEFAULTS).toEqual({
            enabled: true,
            windows: 'tabbed',
            mac: 'regular',
            opacity: 60,
            brightness: 0,
        });
        expect(useSettingsStore.getState().windowBackdrop).toEqual(WINDOW_BACKDROP_DEFAULTS);
    });

    it('mixes the cover toward white or black and keeps the minimum opacity where nothing blurs', () => {
        expect(backdropVars({ ...WINDOW_BACKDROP_DEFAULTS, brightness: 20 }, 0)).toEqual({
            '--backdrop-opacity': '60%',
            '--backdrop-mix': 'white',
            '--backdrop-mix-amount': '20%',
        });
        expect(backdropVars({ ...WINDOW_BACKDROP_DEFAULTS, opacity: 30, brightness: -15 }, 85)).toEqual({
            '--backdrop-opacity': '85%',
            '--backdrop-mix': 'black',
            '--backdrop-mix-amount': '15%',
        });
    });

    it('clears the page background only when the shell says the effect is on', async () => {
        const call = mockApi(WIN11);
        await applyWindowBackdrop(WINDOW_BACKDROP_DEFAULTS);
        expect(call).toHaveBeenCalledWith(WINDOW_BACKDROP_DEFAULTS);
        expect(document.documentElement.dataset.backdrop).toBe('on');

        await applyWindowBackdrop({ ...WINDOW_BACKDROP_DEFAULTS, enabled: false });
        expect(document.documentElement.dataset.backdrop).toBe('off');

        mockApi({ ...WIN11, available: false, reason: 'windows-old', choices: [] });
        await applyWindowBackdrop(WINDOW_BACKDROP_DEFAULTS);
        expect(document.documentElement.dataset.backdrop).toBe('off');
    });

    it('loads the saved value and fills missing fields with defaults', async () => {
        mockApi(WIN11);
        (window.api.settingsLoad as ReturnType<typeof vi.fn>).mockResolvedValue({
            windowBackdrop: { enabled: false, opacity: 40 },
        });
        await useSettingsStore.getState().load();
        expect(useSettingsStore.getState().windowBackdrop).toEqual({
            ...WINDOW_BACKDROP_DEFAULTS,
            enabled: false,
            opacity: 40,
        });
    });

    it('settings → appearance: the toggle turns it off and the Windows material can be picked', async () => {
        const call = mockApi(WIN11);
        render(<SettingsModal />);
        fireEvent.click(screen.getByRole('tab', { name: i18n.t('gui:settings_modal.appearance') }));
        await act(async () => {});
        const toggle = screen.getByRole('checkbox', { name: i18n.t('gui:settings_modal.backdrop_label') });
        expect(toggle).toBeChecked();
        expect(screen.getByText(i18n.t('gui:settings_modal.backdrop_material_label'))).toBeInTheDocument();
        expect(
            screen.getByRole('slider', { name: i18n.t('gui:settings_modal.backdrop_opacity_label') }),
        ).toBeInTheDocument();
        expect(
            screen.getByRole('slider', { name: i18n.t('gui:settings_modal.backdrop_brightness_label') }),
        ).toBeInTheDocument();

        await act(async () => {
            fireEvent.click(toggle);
        });
        expect(useSettingsStore.getState().windowBackdrop.enabled).toBe(false);
        expect(call).toHaveBeenLastCalledWith(expect.objectContaining({ enabled: false }));
        expect(screen.queryByText(i18n.t('gui:settings_modal.backdrop_material_label'))).not.toBeInTheDocument();
    });
});
