/**
 * 모두 업데이트에서 데몬이 일부만 적용하면 {ok:false, applied:[..], errors:[..]}로 답한다.
 * GUI는 실패만 보이지 않고, 같은 요청에서 적용된 것도 알린다 (CLI와 같게, G-U9b).
 */
import { act, fireEvent, render, waitFor } from '@testing-library/react';
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import i18n from '../i18n';

vi.mock('../contexts/ExtensionContext', () => ({
    useExtensions: () => ({ extensions: [], loading: false, refresh: vi.fn() }),
}));

import SabaStorage from '../components/Modals/SabaStorage';

const components = [
    { component: 'cli', display_name: 'CLI', update_available: true, downloaded: true },
    { component: 'discord_bot', display_name: 'Bot', update_available: true, downloaded: true },
];

beforeEach(() => {
    window.showToast = vi.fn();
    window.api = new Proxy(
        {
            updaterStatus: vi.fn().mockResolvedValue({ ok: true, components }),
            updaterDownload: vi.fn().mockResolvedValue({ ok: true }),
            updaterApply: vi.fn().mockResolvedValue({
                ok: false,
                applied: ['cli'],
                errors: ['discord_bot: disk full'],
                needs_updater: [],
                requires_updater: false,
            }),
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

describe('모두 업데이트가 일부만 적용됐을 때', () => {
    it('실패와 함께 적용된 것도 알린다', async () => {
        render(<SabaStorage onBack={() => {}} isExiting={false} />);
        await waitFor(() => expect(document.querySelector('.ss-icon-btn.accent')).not.toBeNull());
        await act(async () => fireEvent.click(document.querySelector('.ss-icon-btn.accent')));
        await waitFor(() => expect(document.querySelector('.update-modal-status-bar.error')).not.toBeNull());
        expect(document.querySelector('.update-modal-status-bar.error').textContent).toContain(
            'discord_bot: disk full',
        );
        const done = i18n.t('gui:saba_storage.apply_completed', { names: 'cli' });
        expect(done).not.toBe('saba_storage.apply_completed');
        await waitFor(() =>
            expect(document.querySelector('.update-modal-status-bar.success')?.textContent).toContain(done),
        );
    });
});
