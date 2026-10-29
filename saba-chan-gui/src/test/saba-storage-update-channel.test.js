/**
 * 업데이트 채널(정식·미리보기·베타)을 사바 스토리지 설정에서 고른다.
 * 지금 채널을 보여 주고, 고르면 데몬 PUT /api/updates/config 에 channel 만 보낸다. 실패하면 되돌린다.
 */
import { fireEvent, render, screen, waitFor } from '@testing-library/react';
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';

vi.mock('../contexts/ExtensionContext', () => ({
    useExtensions: () => ({ extensions: [], loading: false, refresh: vi.fn() }),
}));

import SabaStorage from '../components/Modals/SabaStorage';

const setConfigSpy = vi.fn();

beforeEach(() => {
    setConfigSpy.mockReset().mockResolvedValue({ ok: true });
    window.api = new Proxy(
        {
            updaterSetConfig: setConfigSpy,
            updaterGetConfig: vi.fn().mockResolvedValue({ config: { enabled: true, channel: 'rc' } }),
            // 캐시가 있으면 열 때 릴레이에 다시 묻지 않는다
            updaterStatus: vi.fn().mockResolvedValue({ ok: true, components: [{ component: 'saba-core', current_version: '0.1.11' }] }),
            updaterCheck: vi.fn().mockResolvedValue({ ok: true, components: [] }),
            updaterDownloadProgress: vi.fn().mockResolvedValue({ ok: true, active: false }),
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

describe('업데이트 채널', () => {
    it('지금 채널을 보여 주고, 고르면 channel 만 저장한다', async () => {
        render(<SabaStorage onBack={() => {}} isExiting={false} />);
        const box = await screen.findByRole('combobox', { name: 'Update channel' });
        await waitFor(() => expect(box.textContent).toContain('Preview'));

        fireEvent.click(box);
        fireEvent.click(await screen.findByRole('option', { name: /Beta/ }));
        await waitFor(() => expect(setConfigSpy).toHaveBeenCalledWith({ channel: 'beta' }));
        expect(box.textContent).toContain('Beta');
    });

    it('채널을 바꾸면 새 채널로 목록을 다시 받는다', async () => {
        render(<SabaStorage onBack={() => {}} isExiting={false} />);
        const box = await screen.findByRole('combobox', { name: 'Update channel' });
        await waitFor(() => expect(box.textContent).toContain('Preview'));
        expect(window.api.updaterCheck).not.toHaveBeenCalled();

        fireEvent.click(box);
        fireEvent.click(await screen.findByRole('option', { name: /Beta/ }));
        await waitFor(() => expect(window.api.updaterCheck).toHaveBeenCalledTimes(1));
        expect(setConfigSpy.mock.invocationCallOrder[0]).toBeLessThan(
            window.api.updaterCheck.mock.invocationCallOrder[0],
        );
    });

    it('저장에 실패하면 이전 채널로 되돌린다', async () => {
        setConfigSpy.mockResolvedValue({ ok: false, error: 'nope' });
        render(<SabaStorage onBack={() => {}} isExiting={false} />);
        const box = await screen.findByRole('combobox', { name: 'Update channel' });
        await waitFor(() => expect(box.textContent).toContain('Preview'));

        fireEvent.click(box);
        fireEvent.click(await screen.findByRole('option', { name: /Stable/ }));
        await waitFor(() => expect(setConfigSpy).toHaveBeenCalledWith({ channel: 'stable' }));
        await waitFor(() => expect(box.textContent).toContain('Preview'));
        expect(window.api.updaterCheck).not.toHaveBeenCalled();
    });
});
