import { act, renderHook } from '@testing-library/react';
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import { ExtensionProvider, useExtensions } from '../contexts/ExtensionContext';

const ext = { id: 'steamcmd', enabled: true, version: '1.0.0' };

describe('익스텐션 목록을 받지 못했을 때', () => {
    beforeEach(() => {
        vi.useFakeTimers();
        vi.spyOn(console, 'warn').mockImplementation(() => {});
    });
    afterEach(() => {
        vi.useRealTimers();
        vi.restoreAllMocks();
        delete window.api.extensionList;
    });

    it('처음에 실패해도 다시 받아 와서 익스텐션 UI를 되살린다', async () => {
        window.api.extensionList = vi
            .fn()
            .mockResolvedValueOnce({ extensions: [], error: 'ECONNREFUSED' })
            .mockResolvedValue({ extensions: [ext] });
        const { result } = renderHook(() => useExtensions(), { wrapper: ExtensionProvider });
        await act(async () => {});
        expect(result.current.extensions).toEqual([]);
        await act(async () => vi.advanceTimersByTimeAsync(5000));
        expect(result.current.extensions.map((e) => e.id)).toEqual(['steamcmd']);
    });

    it('받아 둔 목록은 다음 읽기가 실패해도 지우지 않는다', async () => {
        window.api.extensionList = vi
            .fn()
            .mockResolvedValueOnce({ extensions: [ext] })
            .mockResolvedValue({ extensions: [], error: 'ECONNREFUSED' });
        const { result } = renderHook(() => useExtensions(), { wrapper: ExtensionProvider });
        await act(async () => {});
        await act(async () => result.current.refreshExtensions());
        expect(result.current.extensions.map((e) => e.id)).toEqual(['steamcmd']);
    });
});
