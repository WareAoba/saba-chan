/**
 * 모두 업데이트는 컴포넌트 탭에 보이지 않는 모듈·익스텐션 업데이트도 함께 받고 적용한다 (UP1).
 * 0.1.8→0.1.9에서 앱만 바뀌고 모듈·익스텐션은 한 번 더 눌러야 했다.
 */
import { act, fireEvent, render, waitFor } from '@testing-library/react';
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';

vi.mock('../contexts/ExtensionContext', () => ({
    useExtensions: () => ({ extensions: [], loading: false, refresh: vi.fn() }),
}));

import SabaStorage from '../components/Modals/SabaStorage';

const components = [
    { component: 'cli', display_name: 'CLI', update_available: true, downloaded: false },
    { component: 'module-minecraft', display_name: 'Minecraft', update_available: true, downloaded: false },
    { component: 'ext-steamcmd', display_name: 'SteamCMD', update_available: true, downloaded: false },
    { component: 'module-palworld', display_name: 'Palworld', update_available: false, downloaded: false },
];

beforeEach(() => {
    window.showToast = vi.fn();
    window.api = new Proxy(
        {
            updaterStatus: vi.fn().mockResolvedValue({ ok: true, components }),
            updaterDownload: vi.fn().mockResolvedValue({ ok: true }),
            updaterApply: vi.fn().mockResolvedValue({ ok: true, applied: [], needs_updater: [], requires_updater: false }),
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

describe('모두 업데이트', () => {
    it('업데이트가 있는 모듈·익스텐션도 받고 같은 적용 요청에 싣는다', async () => {
        render(<SabaStorage onBack={() => {}} isExiting={false} />);
        await waitFor(() => expect(document.querySelector('.ss-icon-btn.accent')).not.toBeNull());
        await act(async () => fireEvent.click(document.querySelector('.ss-icon-btn.accent')));
        await waitFor(() => expect(window.api.updaterApply).toHaveBeenCalled());
        const downloaded = window.api.updaterDownload.mock.calls.flatMap(([keys]) => keys);
        expect(downloaded).toEqual(expect.arrayContaining(['cli', 'module-minecraft', 'ext-steamcmd']));
        expect(downloaded).not.toContain('module-palworld');
        expect(window.api.updaterApply.mock.calls[0][0]).toEqual(
            expect.arrayContaining(['cli', 'module-minecraft', 'ext-steamcmd']),
        );
    });
});
