/**
 * 설정 저장은 데몬이 잠금을 기다리고 모듈 configure(Python)를 돌린 뒤 답한다: CLI(SLOW)처럼 오래 기다린다.
 * 삭제가 백업·복원·업데이트와 겹쳐 거절되면(409 instance_busy) "먼저 정지" 대신 작업 중이라고 알린다.
 */
import i18n from 'i18next';
import { describe, expect, it, vi } from 'vitest';
import { createApi } from '../api/createApi';

describe('설정 저장·삭제 브리지', () => {
    it('설정 저장은 CLI처럼 60초 이상 기다린다', async () => {
        const invoke = vi.fn().mockResolvedValue({ status: 200, data: { success: true } });
        const { api } = createApi({ invoke, hub: { dispatch: vi.fn() } as never });
        await api.instanceUpdateSettings('a', { motd: 'hi' });
        const call = invoke.mock.calls.find(([, args]) => args?.method === 'PATCH');
        expect(call![1].timeoutMs).toBeGreaterThanOrEqual(60_000);
    });

    it('삭제가 작업 중이라 거절되면 이유를 넘기고 정지하라고 하지 않는다', async () => {
        const data = { error: 'busy', error_code: 'instance_busy' };
        const invoke = vi.fn().mockResolvedValue({ status: 409, data });
        const { api } = createApi({ invoke, hub: { dispatch: vi.fn() } as never });
        const result = await api.instanceDelete('a');
        expect(result.error_code).toBe('instance_busy');
        expect(result.error).toBe(i18n.t('errors.instance_busy', { ns: 'common' }));
        expect(result.error).not.toBe(i18n.t('errors.instance_running', { ns: 'common' }));
    });
});
