/**
 * 받는 진행은 묻지 않고 데몬의 `update.progress` 이벤트로 받는다 (F3). 퍼센트는 데몬이 계산한다.
 * 업데이트 한 번에 데몬이 구성요소를 차례로 받으면, 막대는 지금 받는 것의 이름과 퍼센트를 보인다.
 */
import { act, fireEvent, render, waitFor } from '@testing-library/react';
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';

vi.mock('../contexts/ExtensionContext', () => ({
    useExtensions: () => ({ extensions: [], manifestExtensions: [], availableUpdates: [], installingIds: new Set(), loading: false, refresh: vi.fn() }),
}));

import SabaStorage from '../components/Modals/SabaStorage';

const components = [
    { component: 'cli', kind: 'app', display_name: 'CLI', update_available: true, downloaded: false },
    { component: 'discord_bot', kind: 'app', display_name: 'Bot', update_available: true, downloaded: false },
];

let finish: Record<string, (value?: unknown) => void>;
let listeners: Set<(ev: unknown) => void>;
const progress = (component: string, percent: number | null, active = true) =>
    act(() => {
        for (const l of listeners) l({ type: 'update.progress', data: { component, percent, active, bytesReceived: 1, totalBytes: 2 } });
    });

beforeEach(() => {
    window.showToast = vi.fn();
    finish = {};
    listeners = new Set();
    window.api = new Proxy<Record<string | symbol, unknown>>(
        {
            updaterStatus: vi.fn().mockResolvedValue({ ok: true, components }),
            updaterDownload: vi.fn(() => new Promise((resolve) => (finish.all = resolve))),
            updaterApply: vi.fn().mockResolvedValue({ ok: true, applied: ['cli', 'discord_bot'], needs_updater: [], requires_updater: false }),
            onUpdateEvent: (cb: (ev: unknown) => void) => {
                listeners.add(cb);
                return () => listeners.delete(cb);
            },
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

describe('구성요소를 받는 진행', () => {
    it('데몬이 알린 구성요소와 퍼센트를 막대에 보이고, 다 받으면 막대를 거둔다', async () => {
        render(<SabaStorage />);
        await waitFor(() => expect(document.querySelector('.ss-update-btn')).not.toBeNull());
        await act(async () => fireEvent.click(document.querySelector('.ss-update-btn')!));
        const row = () => document.querySelector('.ss-progress-row')?.textContent;
        progress('cli', 10);
        await waitFor(() => expect(row()).toContain('CLI'));
        expect(row()).toContain('10%');

        // cli가 끝나고 데몬은 이제 discord_bot을 받는다
        progress('cli', null, false);
        progress('discord_bot', 40);
        await waitFor(() => expect(row()).toContain('Bot'));
        expect(row()).toContain('40%');
        // 전체 크기를 모르면 흐르는 막대
        progress('discord_bot', null);
        await waitFor(() => expect(document.querySelector('.ss-progress.indeterminate')).not.toBeNull());

        progress('discord_bot', null, false);
        await act(async () => finish.all({ ok: true }));
        await waitFor(() => expect(window.api.updaterApply).toHaveBeenCalledWith([]));
        await waitFor(() => expect(document.querySelector('.ss-progress')).toBeNull());
    });

    it('상태는 묻지 않고 데몬이 알린 update.status로 바뀐다', async () => {
        vi.useFakeTimers({ shouldAdvanceTime: true });
        try {
            render(<SabaStorage />);
            await waitFor(() => expect(document.querySelector('.ss-update-btn')).not.toBeNull());
            const asked = vi.mocked(window.api.updaterStatus).mock.calls.length;
            act(() => {
                for (const l of listeners)
                    l({ type: 'update.status', data: { ok: true, components: [{ ...components[0], update_available: false, current_version: '9.9.9' }] } });
            });
            await waitFor(() => expect(document.querySelector('.ss-update-btn')).toBeNull());
            expect(document.body.textContent).toContain('v9.9.9');
            await act(async () => {
                await vi.advanceTimersByTimeAsync(60_000);
            });
            expect(vi.mocked(window.api.updaterStatus).mock.calls.length).toBe(asked);
        } finally {
            vi.useRealTimers();
        }
    });
});
