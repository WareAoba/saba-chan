/**
 * 데몬은 같은 모듈의 설치·제거가 겹치면 {ok:false, error_code:"module_busy"}로 거절한다.
 * GUI는 그것을 실패 표시 하나로 뭉개거나(설치) 아무것도 보이지 않는(제거) 대신 이유를 알린다.
 */
import { act, fireEvent, render, waitFor } from '@testing-library/react';
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import i18n from '../i18n';

vi.mock('../contexts/ExtensionContext', () => ({
    useExtensions: () => ({ extensions: [], loading: false, refresh: vi.fn() }),
}));

import SabaStorage from '../components/Modals/SabaStorage';

const BUSY = { ok: false, error: 'Module minecraft is being installed or removed', error_code: 'module_busy' };

beforeEach(() => {
    window.showToast = vi.fn();
    window.api = new Proxy(
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
    );
});

afterEach(() => {
    delete window.api;
});

async function openModules() {
    render(<SabaStorage onBack={() => {}} isExiting={false} />);
    await act(async () => fireEvent.click(document.querySelectorAll('.saba-storage-tab')[1]));
    await waitFor(() => expect(document.querySelector('.ss-icon-btn.danger')).not.toBeNull());
}

const busyText = () => i18n.t('gui:saba_storage.module_busy');

describe('모듈 설치·제거가 겹쳐 거절될 때', () => {
    it('제거가 거절되면 이유를 알린다', async () => {
        await openModules();
        await act(async () => fireEvent.click(document.querySelector('.ss-icon-btn.danger')));
        await act(async () => fireEvent.click(document.querySelector('.question-confirm')));
        await waitFor(() => expect(window.showToast).toHaveBeenCalledWith(busyText(), 'warning', 4000, undefined));
        expect(busyText()).not.toBe('saba_storage.module_busy');
    });

    it('설치가 거절되면 실패로 굳히지 않고 이유를 알린 뒤 다시 누를 수 있게 둔다', async () => {
        await openModules();
        const headers = document.querySelectorAll('.ss-store-header');
        const header = headers[headers.length - 1].querySelector('.ss-icon-btn');
        await act(async () => fireEvent.click(header));
        await waitFor(() => expect(document.querySelector('.ss-icon-btn.accent')).not.toBeNull());
        await act(async () => fireEvent.click(document.querySelector('.ss-icon-btn.accent')));
        await waitFor(() => expect(window.showToast).toHaveBeenCalledWith(busyText(), 'warning', 4000, undefined));
        expect(document.querySelector('.ss-status-err')).toBeNull();
        expect(document.querySelector('.ss-icon-btn.accent')).not.toBeNull();
    });

    it('다른 이유로 제거하지 못하면 데몬 원문 대신 번역한 이유를 알린다', async () => {
        window.api.moduleRemove.mockResolvedValue({ ok: false, error: 'Permission denied (os error 13)' });
        await openModules();
        await act(async () => fireEvent.click(document.querySelector('.ss-icon-btn.danger')));
        await act(async () => fireEvent.click(document.querySelector('.question-confirm')));
        const expected = i18n.t('gui:saba_storage.remove_failed', { error: i18n.t('gui:errors.permission_denied') });
        await waitFor(() => expect(window.showToast).toHaveBeenCalledWith(expected, 'error', 5000, undefined));
    });
});
