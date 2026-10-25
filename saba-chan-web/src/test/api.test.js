import { describe, expect, it } from 'vitest';
import { createWebApi } from '../api';

function fakeSession(reply) {
    const calls = [];
    return {
        calls,
        request: async (method, path, body, timeoutMs) => {
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
        const got = [];
        api.onDaemonEvent((e) => got.push(e));
        emit({ type: 'instance.exit', data: { id: 'a' } });
        expect(got).toEqual([{ type: 'instance.exit', data: { id: 'a' } }]);
    });
});
