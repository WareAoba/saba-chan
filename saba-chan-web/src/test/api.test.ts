import { describe, expect, it, vi } from 'vitest';
import { createWebApi } from '../api';
import type { PcEvent, SessionReply } from '../types';

function fakeSession(reply: SessionReply) {
    const calls: object[] = [];
    return {
        calls,
        request: async (method: string, path: string, body?: unknown, timeoutMs?: number) => {
            calls.push({ method, path, body, timeoutMs });
            return reply;
        },
    };
}

describe('웹 브리지', () => {
    it('GUI 브리지의 데몬 요청을 세션으로 보낸다', async () => {
        const session = fakeSession({ status: 200, data: { servers: [{ id: 'a' }] } });
        const { api } = createWebApi(session);
        await expect(api.serverList()).resolves.toEqual({ servers: [{ id: 'a' }] });
        expect(session.calls[0]).toMatchObject({ method: 'GET', path: '/api/servers?notices_after=0' });
    });

    it('PC 에서만 하는 일은 문지기의 답을 그대로 돌려준다', async () => {
        const body = { ok: false, success: false, error: 'x', error_code: 'remote_not_allowed' };
        const { api } = createWebApi(fakeSession({ status: 403, data: body }));
        await expect(api.permissionsGet('g')).resolves.toMatchObject({ error_code: 'remote_not_allowed' });
    });

    it('PC 의 창·파일을 여는 명령은 하지 않는다', async () => {
        const { api } = createWebApi(fakeSession({ status: 200, data: {} }));
        await expect(api.openFolderDialog()).rejects.toThrow('not_supported');
    });

    it('PC 이벤트를 GUI 와 같은 채널로 나눠 준다', () => {
        const { api, emit } = createWebApi(fakeSession({ status: 200, data: {} }));
        const got: PcEvent[] = [];
        api.onDaemonEvent((e) => got.push(e));
        emit({ type: 'instance.exit', data: { id: 'a' } });
        expect(got).toEqual([{ type: 'instance.exit', data: { id: 'a' } }]);
    });

    it('콘솔 스트림을 세션의 조각 답으로 받는다', async () => {
        let chunk: (body: unknown) => void = () => {};
        let end: (r: SessionReply) => void = () => {};
        const cancel = vi.fn();
        const session = {
            ...fakeSession({ status: 200, data: {} }),
            stream: vi.fn((_m: string, _p: string, onChunk: (b: unknown) => void, onEnd: (r: SessionReply) => void) => {
                chunk = onChunk;
                end = onEnd;
                return cancel;
            }),
        };
        const { api } = createWebApi(session);
        const got: unknown[] = [];
        const ended = vi.fn();
        const close = await api.daemonConsoleStream({ since: 3 }, (b) => got.push(b), ended);
        expect(session.stream).toHaveBeenCalledWith('GET', '/api/daemon/console/stream?since=3&spans=1', expect.any(Function), expect.any(Function));
        chunk({ lines: ['a'] });
        expect(got).toEqual([{ lines: ['a'] }]);
        end({ status: 200 });
        expect(ended).toHaveBeenCalledTimes(1);
        expect(typeof close).toBe('function');

        await api.daemonConsoleStream({}, () => {}, () => {}).then((c) => c?.());
        expect(cancel).toHaveBeenCalledTimes(1);
    });

    it('스트림을 열 수 없는 세션이면 null: 화면은 주기 조회로 읽는다', async () => {
        const session = { ...fakeSession({ status: 200, data: {} }), stream: () => null };
        const { api } = createWebApi(session);
        await expect(api.daemonConsoleStream({}, () => {}, () => {})).resolves.toBeNull();
    });
});
