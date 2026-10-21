import { act, renderHook, waitFor } from '@testing-library/react';
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import { ExtensionProvider, useExtensions } from '../contexts/ExtensionContext';

const docker = { id: 'docker', enabled: true, version: '1.0.0', gui: { bundle: 'x.js' }, gui_allowed: true };

// 번들을 한 번 받지 못했다고 로드됨으로 기억하면, 목록을 다시 받아도 다시 요청하지 않아 칸이 빈 채로 남는다 (외부 감사 10-21 #6)
describe('익스텐션 GUI 번들을 받지 못했을 때', () => {
    beforeEach(() => {
        window.showToast = vi.fn();
        window.api.extensionList = vi.fn(async () => ({ extensions: [{ ...docker }] }));
        window.api.extensionGuiBundle = vi.fn().mockRejectedValueOnce(new Error('daemon busy')).mockResolvedValue(null);
        window.api.extensionGuiStyles = vi.fn().mockResolvedValue(null);
        window.api.extensionI18n = vi.fn().mockResolvedValue(null);
    });
    afterEach(() => {
        for (const k of ['extensionList', 'extensionGuiBundle', 'extensionGuiStyles', 'extensionI18n']) {
            delete window.api[k];
        }
    });

    it('다음에 목록을 받을 때 번들을 다시 요청한다', async () => {
        const { result } = renderHook(() => useExtensions(), { wrapper: ExtensionProvider });
        await waitFor(() => expect(window.api.extensionGuiBundle).toHaveBeenCalledTimes(1));
        await waitFor(() => expect(result.current.loading).toBe(false));
        await act(async () => result.current.refreshExtensions());
        await waitFor(() => expect(window.api.extensionGuiBundle).toHaveBeenCalledTimes(2));
    });
});
