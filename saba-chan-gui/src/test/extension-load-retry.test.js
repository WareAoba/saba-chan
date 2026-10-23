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

// 번들 실행이 throw해도 브라우저는 window error 뒤 onload를 부른다. 전역이 없거나 실행이 실패한 번들을 로드됨으로 기억하면 같은 버전은 다시 요청하지 않는다
describe('익스텐션 GUI 번들 실행이 실패했을 때', () => {
    const Slot = () => null;
    let runs;
    beforeEach(() => {
        window.showToast = vi.fn();
        window.api.extensionList = vi.fn(async () => ({ extensions: [{ ...docker }] }));
        window.api.extensionGuiBundle = vi.fn().mockResolvedValue('/* bundle */');
        window.api.extensionGuiStyles = vi.fn().mockResolvedValue(null);
        window.api.extensionI18n = vi.fn().mockResolvedValue(null);
        runs = [];
        const realAppend = document.head.appendChild.bind(document.head);
        vi.spyOn(document.head, 'appendChild').mockImplementation((el) => {
            realAppend(el);
            if (el.tagName === 'SCRIPT') (runs.shift() || (() => {}))(el);
            return el;
        });
    });
    afterEach(() => {
        vi.restoreAllMocks();
        delete window.SabaExtDocker;
        for (const k of ['extensionList', 'extensionGuiBundle', 'extensionGuiStyles', 'extensionI18n']) {
            delete window.api[k];
        }
    });
    const register = () => {
        window.SabaExtDocker = { registerSlots: () => ({ 'test.slot': [Slot] }) };
    };
    const ok = (el) => {
        register();
        el.onload();
    };

    async function retriesAfter(firstRun) {
        runs.push(firstRun, ok);
        const { result } = renderHook(() => useExtensions(), { wrapper: ExtensionProvider });
        await waitFor(() => expect(result.current.loading).toBe(false));
        expect(result.current.slots['test.slot']).toBeUndefined();
        await act(async () => result.current.refreshExtensions());
        await waitFor(() => expect(window.api.extensionGuiBundle).toHaveBeenCalledTimes(2));
        await waitFor(() => expect(result.current.slots['test.slot']).toEqual([Slot]));
    }

    it('전역을 만들지 못한 번들은 같은 버전이라도 다음 목록에서 다시 요청한다', async () => {
        await retriesAfter((el) => {
            window.dispatchEvent(new ErrorEvent('error', { filename: el.src, error: new Error('dep missing') }));
            el.onload();
        });
    });

    it('오류 없이 끝났어도 전역이 없으면 다시 요청한다', async () => {
        await retriesAfter((el) => el.onload());
    });

    it('전역을 먼저 등록한 뒤 throw한 번들도 성공으로 기억하지 않는다', async () => {
        await retriesAfter((el) => {
            register();
            window.dispatchEvent(new ErrorEvent('error', { filename: el.src, error: new Error('late throw') }));
            el.onload();
        });
    });

    // 외부 감사: 늦게 실행된 앞 로드의 실패 정리가 겹친 뒤 로드가 성공해 등록한 전역까지 지워, 다시 받지도 않고 칸이 빈 채로 남았다
    it('늦게 실패한 앞 로드는 겹친 로드가 성공해 등록한 전역을 지우지 않는다', async () => {
        let releaseFirst;
        window.api.extensionGuiBundle = vi
            .fn()
            .mockImplementationOnce(() => new Promise((resolve) => (releaseFirst = () => resolve('/* bundle A */'))))
            .mockResolvedValue('/* bundle B */');
        let firstRan = false;
        runs.push(ok, (el) => {
            window.SabaExtDocker = { registerSlots: () => ({}) };
            window.dispatchEvent(new ErrorEvent('error', { filename: el.src, error: new Error('late throw') }));
            el.onload();
            firstRan = true;
        });
        const { result } = renderHook(() => useExtensions(), { wrapper: ExtensionProvider });
        await waitFor(() => expect(window.api.extensionGuiBundle).toHaveBeenCalledTimes(1));
        await act(async () => result.current.refreshExtensions());
        await waitFor(() => expect(result.current.slots['test.slot']).toEqual([Slot]));
        const loaded = window.SabaExtDocker;
        await act(async () => {
            releaseFirst();
            await new Promise((r) => setTimeout(r, 0));
        });
        expect(firstRan).toBe(true);
        expect(window.SabaExtDocker).toBe(loaded);
        await act(async () => result.current.refreshExtensions());
        await waitFor(() => expect(window.api.extensionList).toHaveBeenCalledTimes(3));
        await act(async () => new Promise((r) => setTimeout(r, 0)));
        expect(window.api.extensionGuiBundle).toHaveBeenCalledTimes(2);
        expect(result.current.slots['test.slot']).toEqual([Slot]);
    });

    it('로드됨으로 기억했는데 전역이 사라졌으면 다음 목록에서 다시 읽는다', async () => {
        runs.push(ok, ok);
        const { result } = renderHook(() => useExtensions(), { wrapper: ExtensionProvider });
        await waitFor(() => expect(result.current.slots['test.slot']).toEqual([Slot]));
        delete window.SabaExtDocker;
        await act(async () => result.current.refreshExtensions());
        await waitFor(() => expect(window.api.extensionGuiBundle).toHaveBeenCalledTimes(2));
        await waitFor(() => expect(window.SabaExtDocker).toBeDefined());
    });
});
