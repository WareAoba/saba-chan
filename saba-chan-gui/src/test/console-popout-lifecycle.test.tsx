import { renderHook, waitFor } from '@testing-library/react';
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import { usePopoutLifecycle } from '../hooks/usePopoutLifecycle';

type Srv = { id: string; name: string; status: string };

describe('usePopoutLifecycle (PIP 기억)', () => {
    let popout: ReturnType<typeof vi.fn>;
    let close: ReturnType<typeof vi.fn>;
    let saved: string[];

    beforeEach(() => {
        saved = [];
        popout = vi.fn(async () => ({ ok: true }));
        close = vi.fn(async () => ({ ok: true }));
        Object.assign(window.api, {
            consolePopout: popout,
            consolePopoutClose: close,
            consolePopoutSaved: vi.fn(async () => saved),
        });
    });
    afterEach(() => {
        const api = window.api as unknown as Record<string, unknown>;
        delete api.consolePopout;
        delete api.consolePopoutClose;
        delete api.consolePopoutSaved;
    });

    const run = (servers: Srv[], popoutIds: string[]) =>
        renderHook((p: { servers: Srv[]; popoutIds: string[] }) => usePopoutLifecycle({ enabled: true, ...p }), {
            initialProps: { servers, popoutIds },
        });

    it('PIP가 떠 있던 서버가 꺼지면 PIP를 닫고 다시 띄울 수 있게 남긴다', async () => {
        const h = run([{ id: 'a', name: 'A', status: 'running' }], ['a']);
        h.rerender({ servers: [{ id: 'a', name: 'A', status: 'stopping' }], popoutIds: ['a'] });
        expect(close).not.toHaveBeenCalled();
        h.rerender({ servers: [{ id: 'a', name: 'A', status: 'stopped' }], popoutIds: ['a'] });
        await waitFor(() => expect(close).toHaveBeenCalledWith('a', true));
    });

    it('기억된 서버가 다시 켜지면 PIP를 띄운다', async () => {
        saved = ['a'];
        const h = run([{ id: 'a', name: 'A', status: 'stopped' }], []);
        expect(popout).not.toHaveBeenCalled();
        h.rerender({ servers: [{ id: 'a', name: 'A', status: 'starting' }], popoutIds: [] });
        await waitFor(() => expect(popout).toHaveBeenCalledWith('a', 'A'));
    });

    it('앱을 켰을 때 이미 돌고 있는 기억된 서버는 PIP를 띄우고, 아닌 서버는 두지 않는다', async () => {
        saved = ['a'];
        run(
            [
                { id: 'a', name: 'A', status: 'running' },
                { id: 'b', name: 'B', status: 'running' },
            ],
            [],
        );
        await waitFor(() => expect(popout).toHaveBeenCalledWith('a', 'A'));
        expect(popout).toHaveBeenCalledTimes(1);
    });

    it('이미 떠 있는 PIP는 다시 띄우지 않는다', async () => {
        saved = ['a'];
        run([{ id: 'a', name: 'A', status: 'running' }], ['a']);
        await new Promise((r) => setTimeout(r, 20));
        expect(popout).not.toHaveBeenCalled();
    });

    it('닫는 중에 서버가 다시 켜지면, 닫기가 끝난 뒤 PIP를 다시 띄운다', async () => {
        let finish: (v: unknown) => void = () => {};
        close.mockImplementation(() => new Promise((r) => (finish = r)));
        saved = ['a'];
        const h = run([{ id: 'a', name: 'A', status: 'running' }], ['a']);
        h.rerender({ servers: [{ id: 'a', name: 'A', status: 'stopped' }], popoutIds: ['a'] });
        h.rerender({ servers: [{ id: 'a', name: 'A', status: 'starting' }], popoutIds: ['a'] });
        h.rerender({ servers: [{ id: 'a', name: 'A', status: 'running' }], popoutIds: ['a'] });
        expect(popout).not.toHaveBeenCalled();
        finish({ ok: true });
        await waitFor(() => expect(popout).toHaveBeenCalledWith('a', 'A'));
    });
});
