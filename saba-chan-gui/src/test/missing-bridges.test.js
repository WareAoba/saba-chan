import { describe, expect, it, vi } from 'vitest';
import { createApi } from '../api/createApi';

// 화면(진단 패널·서버 점검·익스텐션 설정)이 찾는 브리지가 없으면 그 화면이 숨겨진다
const setup = (res = { status: 200, data: { ok: true } }) => {
    const invoke = vi.fn().mockResolvedValue(res);
    const { api } = createApi({ invoke, hub: { dispatch: vi.fn() } });
    const sent = () => invoke.mock.calls.at(-1)[1];
    return { api, sent };
};

describe('데몬 점검·설정 브리지', () => {
    it('daemonDiagnostics는 GET /api/diagnostics', async () => {
        const { api, sent } = setup();
        await expect(api.daemonDiagnostics()).resolves.toEqual({ ok: true });
        expect(sent()).toMatchObject({ method: 'GET', path: '/api/diagnostics' });
    });

    it('instanceValidate·instanceDiagnose는 인스턴스의 validate·diagnose를 POST로 부른다', async () => {
        const { api, sent } = setup({ status: 200, data: { success: true, issues: [] } });
        await expect(api.instanceValidate('a b')).resolves.toEqual({ success: true, issues: [] });
        expect(sent()).toMatchObject({ method: 'POST', path: '/api/instance/a%20b/validate' });
        await api.instanceDiagnose('a b');
        expect(sent()).toMatchObject({ method: 'POST', path: '/api/instance/a%20b/diagnose' });
    });

    it('extensionConfigGet·Set은 익스텐션 설정을 읽고, 값 객체를 그대로 PUT한다', async () => {
        const { api, sent } = setup({ status: 200, data: { config: { a: 1 } } });
        await expect(api.extensionConfigGet('docker')).resolves.toEqual({ config: { a: 1 } });
        expect(sent()).toMatchObject({ method: 'GET', path: '/api/extensions/docker/config' });
        await api.extensionConfigSet('docker', { a: 2 });
        expect(sent()).toMatchObject({ method: 'PUT', path: '/api/extensions/docker/config', body: { a: 2 } });
    });

    it('데몬이 거절하면 오류 본문을 그대로 돌려줘 화면이 알릴 수 있다', async () => {
        const { api } = setup({ status: 400, data: { success: false, error: 'Request body must be a JSON object' } });
        await expect(api.extensionConfigSet('docker', { a: 2 })).resolves.toMatchObject({
            error: 'Request body must be a JSON object',
        });
    });
});
