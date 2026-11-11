import { act, renderHook, waitFor } from '@testing-library/react';
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import { createApi } from '../api/createApi';
import { ExtensionProvider, useExtensions } from '../contexts/ExtensionContext';
import { useUIStore } from '../stores/useUIStore';

const docker = (gui_allowed: boolean) => ({
    id: 'docker',
    enabled: true,
    version: '1.0.0',
    gui: { bundle: 'x.js' },
    gui_allowed,
});

function mount() {
    return renderHook(() => useExtensions(), { wrapper: ExtensionProvider });
}

const modal = () => useUIStore.getState().modal!;

describe('확인되지 않은 익스텐션의 GUI 번들 (gui_allowed)', () => {
    beforeEach(() => {
        window.showToast = vi.fn();
        useUIStore.setState({ modal: null });
        window.api.extensionGuiBundle = vi.fn().mockResolvedValue(null);
        window.api.extensionGuiStyles = vi.fn().mockResolvedValue(null);
        window.api.extensionI18n = vi.fn().mockResolvedValue(null);
    });
    afterEach(() => {
        for (const k of [
            'extensionList',
            'extensionGuiBundle',
            'extensionGuiStyles',
            'extensionI18n',
            'sandboxAllowExtensionGui',
            'daemonTarget',
        ]) {
            delete (window.api as unknown as Record<string, unknown>)[k];
        }
        useUIStore.setState({ modal: null });
    });

    it('허락받지 않은 번들은 요청하지 않고 한 번 묻는다. 허락하면 저장하고 다시 받아 로드한다', async () => {
        window.api.extensionList = vi
            .fn()
            .mockResolvedValueOnce({ extensions: [docker(false)] })
            .mockResolvedValue({ extensions: [docker(true)] });
        window.api.sandboxAllowExtensionGui = vi.fn().mockResolvedValue({ success: true, component: 'ext-docker' });
        mount();
        await waitFor(() => expect(modal()?.type).toBe('question'));
        expect(window.api.extensionGuiBundle).not.toHaveBeenCalled();
        expect(modal().message).toContain('docker');

        await act(async () => modal().buttons[0].action());
        expect(window.api.sandboxAllowExtensionGui).toHaveBeenCalledWith('ext-docker');
        await waitFor(() => expect(window.api.extensionGuiBundle).toHaveBeenCalledWith('docker'));
    });

    it('확인 창에서 취소하면 오류를 띄우지 않고 목록을 다시 받지 않는다', async () => {
        window.api.extensionList = vi.fn().mockResolvedValue({ extensions: [docker(false)] });
        window.api.sandboxAllowExtensionGui = vi.fn().mockResolvedValue({ success: false, error_code: 'canceled' });
        mount();
        await waitFor(() => expect(modal()?.type).toBe('question'));
        await act(async () => modal().buttons[0].action());
        expect(window.api.sandboxAllowExtensionGui).toHaveBeenCalledWith('ext-docker');
        expect(vi.mocked(window.showToast!).mock.calls.filter((c) => c[1] === 'error')).toEqual([]);
        expect(window.api.extensionList).toHaveBeenCalledTimes(1);
        expect(window.api.extensionGuiBundle).not.toHaveBeenCalled();
    });

    it('거절하면 저장하지 않고, 목록을 다시 받아도 이번 실행에서는 다시 묻지 않는다', async () => {
        window.api.extensionList = vi.fn().mockResolvedValue({ extensions: [docker(false)] });
        window.api.sandboxAllowExtensionGui = vi.fn();
        const { result } = mount();
        await waitFor(() => expect(modal()?.type).toBe('question'));
        act(() => modal().buttons[1].action());
        expect(modal()).toBeNull();

        await act(async () => result.current.refreshExtensions());
        expect(modal()).toBeNull();
        expect(window.api.sandboxAllowExtensionGui).not.toHaveBeenCalled();
        expect(window.api.extensionGuiBundle).not.toHaveBeenCalled();
        // 설정 화면에서 다시 허락할 수 있게 대기 목록에 남는다
        expect(result.current.guiPending.map((e) => e.id)).toEqual(['docker']);
    });

    it('번들 요청이 extension_gui_unapproved로 막히면(파일이 바뀜) 다시 묻는다', async () => {
        window.api.extensionList = vi.fn().mockResolvedValue({ extensions: [docker(true)] });
        window.api.extensionGuiBundle = vi.fn().mockResolvedValue({ error_code: 'extension_gui_unapproved' });
        mount();
        await waitFor(() => expect(modal()?.type).toBe('question'));
        expect(window.api.extensionGuiStyles).not.toHaveBeenCalled();
    });

    it('원격 PC를 고른 동안에는 이 PC 데몬의 허락으로 번들을 읽는다 (원격 목록의 gui_allowed는 그 PC의 것이다)', async () => {
        window.api.daemonTarget = vi.fn().mockResolvedValue({ target: 'host-b' });
        // 원격 PC는 허락하지 않았고 이 PC는 허락했다
        window.api.extensionList = vi.fn(async (opts?: { local?: boolean }) => ({
            extensions: [docker(Boolean(opts?.local))],
        })) as never;
        const { result } = mount();
        await waitFor(() => expect(window.api.extensionGuiBundle).toHaveBeenCalledWith('docker'));
        expect(modal()).toBeNull();
        expect(result.current.guiPending).toEqual([]);
    });

    it('원격 PC를 고른 동안 이 PC 목록을 잠깐 받지 못하면 원격 값으로 판단하지 않고 다시 받아 로드한다', async () => {
        vi.useFakeTimers({ shouldAdvanceTime: true });
        try {
            window.api.daemonTarget = vi.fn().mockResolvedValue({ target: 'host-b' });
            let localCalls = 0;
            window.api.extensionList = vi.fn(async (opts?: { local?: boolean }) => {
                if (!opts?.local) return { extensions: [docker(false)] };
                // 첫 조회만 실패한다 (브리지는 실패를 빈 목록과 error로 돌려준다)
                return ++localCalls === 1 ? { extensions: [], error: 'timeout' } : { extensions: [docker(true)] };
            }) as never;
            mount();
            await act(async () => {
                await vi.advanceTimersByTimeAsync(100);
            });
            expect(modal()).toBeNull();
            expect(window.api.extensionGuiBundle).not.toHaveBeenCalled();
            await act(async () => {
                await vi.advanceTimersByTimeAsync(2100);
            });
            await waitFor(() => expect(window.api.extensionGuiBundle).toHaveBeenCalledWith('docker'));
            expect(modal()).toBeNull();
        } finally {
            vi.useRealTimers();
        }
    });

    it('원격 PC는 허락했어도 이 PC에서 허락하지 않았으면 번들을 요청하지 않고 묻는다', async () => {
        window.api.daemonTarget = vi.fn().mockResolvedValue({ target: 'host-b' });
        window.api.extensionList = vi.fn(async (opts?: { local?: boolean }) => ({
            extensions: [docker(!opts?.local)],
        })) as never;
        mount();
        await waitFor(() => expect(modal()?.type).toBe('question'));
        expect(window.api.extensionGuiBundle).not.toHaveBeenCalled();
    });

    it('확인된 익스텐션(gui_allowed:true)은 묻지 않고 로드한다', async () => {
        window.api.extensionList = vi.fn().mockResolvedValue({ extensions: [docker(true)] });
        mount();
        await waitFor(() => expect(window.api.extensionGuiBundle).toHaveBeenCalledWith('docker'));
        expect(modal()).toBeNull();
    });
});

describe('번들 응답 해석 (extensionGuiBundle)', () => {
    it('403 extension_gui_unapproved 본문(텍스트)을 error_code로 돌려준다', async () => {
        const invoke = vi.fn().mockResolvedValue({
            status: 403,
            data: JSON.stringify({ error: 'no', error_code: 'extension_gui_unapproved', component: 'ext-docker' }),
        });
        const { api } = createApi({ invoke, hub: { dispatch: vi.fn() } as never });
        await expect(api.extensionGuiBundle('docker')).resolves.toEqual({ error_code: 'extension_gui_unapproved' });
    });
});
