/**
 * 구성요소 두 개를 따로 내려받으면 진행률 폴러 하나를 같이 쓴다.
 * 먼저 끝난 쪽이 폴러를 멈추면 아직 받는 쪽의 진행 링이 사라진다 (G-U8).
 */
import { act, fireEvent, render, waitFor } from '@testing-library/react';
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';

vi.mock('../contexts/ExtensionContext', () => ({
    useExtensions: () => ({ extensions: [], loading: false, refresh: vi.fn() }),
}));

import SabaStorage from '../components/Modals/SabaStorage';

const components = [
    { component: 'cli', display_name: 'CLI', update_available: true, downloaded: false },
    { component: 'discord_bot', display_name: 'Bot', update_available: true, downloaded: false },
];

let finish;
let progress;

beforeEach(() => {
    window.showToast = vi.fn();
    finish = {};
    progress = { ok: true, component: 'cli', active: true, bytes_received: 10, total_bytes: 100 };
    window.api = new Proxy(
        {
            updaterStatus: vi.fn().mockResolvedValue({ ok: true, components }),
            updaterDownload: vi.fn((keys) => new Promise((resolve) => (finish[keys[0]] = resolve))),
            updaterDownloadProgress: vi.fn(async () => progress),
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

describe('구성요소를 따로 내려받을 때', () => {
    it('먼저 끝난 다운로드가 아직 받는 쪽의 진행률을 지우지 않는다', async () => {
        render(<SabaStorage onBack={() => {}} isExiting={false} />);
        await waitFor(() => expect(document.querySelectorAll('.ss-card .ss-icon-btn.accent')).toHaveLength(2));
        const [cli, bot] = document.querySelectorAll('.ss-card .ss-icon-btn.accent');
        await act(async () => fireEvent.click(cli));
        await act(async () => fireEvent.click(bot));
        await waitFor(() => expect(document.querySelector('.download-ring-wrapper')).not.toBeNull());

        // cli가 끝나고 데몬은 이제 discord_bot을 받는다
        progress = { ok: true, component: 'discord_bot', active: true, bytes_received: 40, total_bytes: 100 };
        await act(async () => finish.cli({ ok: true }));
        await waitFor(() => expect(document.querySelector('.download-ring-wrapper')?.dataset.tooltip).toBe('40%'));

        await act(async () => finish.discord_bot({ ok: true }));
        await waitFor(() => expect(document.querySelector('.download-ring-wrapper')).toBeNull());
    });
});
