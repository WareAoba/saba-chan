/**
 * 데몬은 같은 모듈의 설치·제거가 겹치면 {ok:false, error_code:"module_busy"}로 거절한다.
 * GUI는 그것을 실패 표시 하나로 뭉개거나(설치) 아무것도 보이지 않는(제거) 대신 이유를 알린다.
 */
import { act, fireEvent, render, waitFor } from '@testing-library/react';
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import i18n from '../i18n';

vi.mock('../contexts/ExtensionContext', () => ({
    useExtensions: () => ({ extensions: [], manifestExtensions: [], availableUpdates: [], installingIds: new Set(), loading: false, refresh: vi.fn() }),
}));

import SabaStorage from '../components/Modals/SabaStorage';

const BUSY = { ok: false, error: 'Module minecraft is being installed or removed', error_code: 'module_busy' };

beforeEach(() => {
    window.showToast = vi.fn();
    window.api = new Proxy<Record<string | symbol, unknown>>(
        {
            moduleList: vi.fn().mockResolvedValue({ modules: [{ name: 'Minecraft', path: '/m/minecraft' }] }),
            moduleManifest: vi
                .fn()
                .mockResolvedValue({ ok: true, manifest: { modules: { palworld: { display_name: 'Palworld' } } } }),
            moduleInstallFromManifest: vi.fn().mockResolvedValue(BUSY),
            moduleRemove: vi.fn().mockResolvedValue(BUSY),
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

async function openModules() {
    render(<SabaStorage />);
    await act(async () => fireEvent.click(document.querySelectorAll('.saba-storage-tab')[1]));
    await waitFor(() => expect(document.querySelector('.ss-icon-btn.danger')).not.toBeNull());
}

const busyText = () => i18n.t('gui:saba_storage.module_busy');

describe('모듈 설치·제거가 겹쳐 거절될 때', () => {
    it('제거가 거절되면 이유를 알린다', async () => {
        await openModules();
        await act(async () => fireEvent.click(document.querySelector('.ss-icon-btn.danger')!));
        await act(async () => fireEvent.click(document.querySelector('.question-confirm')!));
        await waitFor(() => expect(window.showToast).toHaveBeenCalledWith(busyText(), 'warning', 4000, undefined));
        expect(busyText()).not.toBe('saba_storage.module_busy');
    });

    it('설치가 거절되면 실패로 굳히지 않고 이유를 알린 뒤 다시 누를 수 있게 둔다', async () => {
        await openModules();
        await waitFor(() => expect(document.querySelector('.ss-install-btn')).not.toBeNull());
        await act(async () => fireEvent.click(document.querySelector('.ss-install-btn')!));
        await waitFor(() => expect(window.showToast).toHaveBeenCalledWith(busyText(), 'warning', 4000, undefined));
        expect(document.querySelector('.ss-status-err')).toBeNull();
        expect(document.querySelector('.ss-install-btn')).not.toBeNull();
    });

    it('다른 이유로 제거하지 못하면 데몬 원문 대신 번역한 이유를 알린다', async () => {
        vi.mocked(window.api.moduleRemove).mockResolvedValue({ ok: false, error: 'Permission denied (os error 13)' });
        await openModules();
        await act(async () => fireEvent.click(document.querySelector('.ss-icon-btn.danger')!));
        await act(async () => fireEvent.click(document.querySelector('.question-confirm')!));
        const expected = i18n.t('gui:saba_storage.remove_failed', { error: i18n.t('gui:errors.permission_denied') });
        await waitFor(() => expect(window.showToast).toHaveBeenCalledWith(expected, 'error', 5000, undefined));
    });
});
