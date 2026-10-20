/**
 * GUI 설정 자동 저장과 언어 저장이 실패하면 조용히 넘기지 않고 알린다 (G-U6).
 */
import { act, render, screen } from '@testing-library/react';
import i18n from 'i18next';
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import SettingsModal from '../components/Modals/SettingsModal';
import { setSettingsI18n, useSettingsStore } from '../stores/useSettingsStore';
import { pickOption } from './dropdown';

const saveError = () => i18n.t('gui:settings.save_error');

beforeEach(() => {
    window.showToast = vi.fn();
    useSettingsStore.getState()._resetForTest();
});

afterEach(() => {
    vi.useRealTimers();
    delete window.electron;
});

describe('GUI 설정 저장 실패', () => {
    it('자동 저장이 실패하면 알린다', async () => {
        vi.useFakeTimers();
        setSettingsI18n((key) => i18n.t(key, { ns: 'gui' }));
        window.api.settingsSave = vi.fn().mockResolvedValue(false);
        useSettingsStore.setState({ settingsReady: true, settingsPath: '/x/config.json' });
        useSettingsStore.getState().update({ autoRefresh: false });
        await act(async () => {
            await vi.advanceTimersByTimeAsync(600);
        });
        expect(window.api.settingsSave).toHaveBeenCalled();
        expect(saveError()).not.toBe('settings.save_error');
        expect(window.showToast).toHaveBeenCalledWith(saveError(), 'error', undefined, undefined);
    });

    it('언어를 저장하지 못하면 알린다', async () => {
        window.electron = { setLanguage: vi.fn().mockResolvedValue({ success: false, language: 'ko' }) };
        render(<SettingsModal isOpen onClose={vi.fn()} />);
        const trigger = screen.getAllByRole('combobox').find((el) => el.textContent.includes('English'));
        pickOption(trigger, '한국어 (Korean)');
        await act(async () => {});
        expect(window.electron.setLanguage).toHaveBeenCalledWith('ko');
        expect(window.showToast).toHaveBeenCalledWith(expect.any(String), 'error', undefined, undefined);
        await act(async () => i18n.changeLanguage('en'));
    });
});
