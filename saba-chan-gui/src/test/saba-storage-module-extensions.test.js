/**
 * 모듈을 설치하면 데몬이 그 모듈이 쓰는 익스텐션을 받고 켠다 (응답의 extensions).
 * GUI는 켠 것과 켜지 못한 것을 알리고, 익스텐션 목록을 새로 받는다.
 */
import { act, fireEvent, render, waitFor } from '@testing-library/react';
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import i18n from '../i18n';

const refreshExtensions = vi.fn();
vi.mock('../contexts/ExtensionContext', () => ({
    useExtensions: () => ({ extensions: [], loading: false, refresh: vi.fn(), refreshExtensions }),
}));

import SabaStorage from '../components/Modals/SabaStorage';

beforeEach(() => {
    refreshExtensions.mockClear();
    window.showToast = vi.fn();
    window.api = new Proxy(
        {
            moduleList: vi.fn().mockResolvedValue({ modules: [] }),
            moduleManifest: vi
                .fn()
                .mockResolvedValue({ ok: true, manifest: { modules: { palworld: { display_name: 'Palworld' } } } }),
            moduleInstallFromManifest: vi.fn(),
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

async function installPalworld(response) {
    window.api.moduleInstallFromManifest.mockResolvedValue(response);
    render(<SabaStorage onBack={() => {}} isExiting={false} />);
    await act(async () => fireEvent.click(document.querySelectorAll('.saba-storage-tab')[1]));
    const headers = document.querySelectorAll('.ss-store-header');
    await act(async () => fireEvent.click(headers[headers.length - 1].querySelector('.ss-icon-btn')));
    await waitFor(() => expect(document.querySelector('.ss-icon-btn.accent')).not.toBeNull());
    await act(async () => fireEvent.click(document.querySelector('.ss-icon-btn.accent')));
}

describe('모듈 설치 때 켠 익스텐션', () => {
    it('켠 익스텐션을 알리고 익스텐션 목록을 새로 받는다', async () => {
        await installPalworld({
            ok: true,
            module_id: 'palworld',
            extensions: { installed: ['ue4-ini'], enabled: ['steamcmd', 'ue4-ini'], failed: [] },
        });
        const expected = i18n.t('gui:saba_storage.extensions_enabled', { ids: 'steamcmd, ue4-ini' });
        await waitFor(() => expect(window.showToast).toHaveBeenCalledWith(expected, 'info', 4000, undefined));
        expect(expected).not.toBe('saba_storage.extensions_enabled');
        expect(refreshExtensions).toHaveBeenCalled();
    });

    it('켜지 못한 익스텐션은 번역한 이유와 함께 경고한다', async () => {
        await installPalworld({
            ok: true,
            module_id: 'palworld',
            extensions: {
                installed: [],
                enabled: [],
                failed: [{ id: 'steamcmd', error: 'Permission denied (os error 13)', error_code: 'install_failed' }],
            },
        });
        const expected = i18n.t('gui:saba_storage.extension_failed', {
            id: 'steamcmd',
            error: i18n.t('gui:errors.permission_denied'),
        });
        await waitFor(() => expect(window.showToast).toHaveBeenCalledWith(expected, 'warning', 6000, undefined));
        expect(document.querySelector('.ss-status-err')).toBeNull();
    });

    it('익스텐션이 필요 없는 모듈은 아무것도 알리지 않는다', async () => {
        await installPalworld({ ok: true, module_id: 'palworld', extensions: { installed: [], enabled: [], failed: [] } });
        await waitFor(() => expect(window.api.moduleInstallFromManifest).toHaveBeenCalled());
        await act(async () => {});
        expect(window.showToast).not.toHaveBeenCalled();
    });
});
