import { describe, expect, it, vi } from 'vitest';
import { createDaemonClient } from '../api/daemon';
import { createApi } from '../api/createApi';
import { moduleFailureText } from '../utils/helpers';

describe('모듈 호출 실패 안내', () => {
    it('시작·정지의 기존 message 필드보다 번역된 모듈 오류 안내를 먼저 보인다', () => {
        const t = (key, opts) => key === 'common:module_errors.module_exception' ? 'guidance' : opts.defaultValue;
        expect(moduleFailureText(t, { error_code: 'module_exception', error: 'guidance with details', message: 'raw SDK exception' })).toBe('guidance with details');
        expect(moduleFailureText(t, { error_code: 'game_failed', error: 'raw machine error', message: 'Module guidance' })).toBe('Module guidance');
    });

    it('기존 오류 표시 경로에 번역 안내와 로컬 진단을 전달한다', async () => {
        const body = {
            success: false, error_code: 'module_exception', error: 'ValueError: bad value',
            module: 'plain', function: 'install_server', phase: 'run', stderr_tail: ['traceback line'],
        };
        const client = createDaemonClient(vi.fn().mockResolvedValue({ status: 500, data: body }));
        const result = await client.daemonCall('post', '/api/instance/id/install');
        expect(result.error).toContain('The module raised an exception.');
        expect(result.error).toContain('plain.install_server');
        expect(result.error).toContain('traceback line');
        expect(result.error).toContain('ValueError: bad value');
        expect(result).toMatchObject({ error_code: body.error_code, module: body.module, function: body.function, phase: 'run', stderr_tail: body.stderr_tail });
    });

    it('HTTP 200의 모듈 실패도 안내하고 모듈의 고유 오류는 그대로 둔다', async () => {
        const own = { success: false, error_code: 'game_not_ready', message: 'Wait for the game' };
        const client = createDaemonClient(vi.fn().mockResolvedValue({ status: 200, data: own }));
        expect(await client.daemonJson('post', '/api/instance/id/query')).toEqual(own);
        const timeout = createDaemonClient(vi.fn().mockResolvedValue({ status: 200, data: { success: false, error_code: 'module_timeout', module: 'plain', function: 'query' } }));
        expect((await timeout.daemonJson('post', '/api/instance/id/query')).error).toContain('The module call timed out.');
    });

    it('기존 명령·설정·정지·익스텐션 오류 경로가 진단 필드를 보존한다', async () => {
        const body = { success: false, error_code: 'module_timeout', module: 'plain', function: 'run', phase: 'run', stderr_tail: ['detail'] };
        const { api } = createApi({ invoke: vi.fn().mockResolvedValue({ status: 500, data: body }), hub: { dispatch: vi.fn(), on: vi.fn() } });
        for (const run of [() => api.executeCommand('id', { command: 'query' }), () => api.instanceUpdateSettings('id', {}), () => api.instanceStop('id'), () => api.extensionEnable('ext')]) {
            const result = await run();
            expect(result).toMatchObject(body);
            expect(result.error).toContain('The module call timed out.');
        }
    });

    it('원격 데몬이 PC 전용 칸을 거절하면 막힌 칸 이름과 함께 안내한다', async () => {
        const body = { ok: false, success: false, error_code: 'remote_field_blocked', error: 'These settings can only be changed on the PC', fields: ['executable_path', 'java_path'] };
        const { api } = createApi({ invoke: vi.fn().mockResolvedValue({ status: 403, data: body }), hub: { dispatch: vi.fn(), on: vi.fn() } });
        const result = await api.instanceUpdateSettings('id', { executable_path: '/x' });
        expect(result.error).toContain('These settings can only be changed on that PC itself: executable_path, java_path');
        expect(result.error).not.toContain('{{');
    });
});
