/**
 * 기존 서버 가져오기는 큰 폴더에 권한을 주고 설정을 읽느라 30초를 넘길 수 있다. 끊으면 GUI는 실패로 보이므로
 * CLI(`post_long`)처럼 오래 기다린다. 새로 설치하는 서버는 데몬이 바로 답한다
 */
import { describe, expect, it, vi } from 'vitest';
import { createApi } from '../api/createApi';

describe('instanceCreate 기다리는 시간', () => {
    const timeoutOf = async (data) => {
        const invoke = vi.fn().mockResolvedValue({ status: 201, data: { success: true, id: 'a' } });
        const { api } = createApi({ invoke, hub: { dispatch: vi.fn() } });
        await api.instanceCreate(data);
        const call = invoke.mock.calls.find(([, args]) => args?.path === '/api/instances');
        return call[1].timeoutMs;
    };

    it('가져오기는 오래 기다린다', async () => {
        expect(await timeoutOf({ name: 'a', module_name: 'game', migration_source: 'D:/server' })).toBeGreaterThanOrEqual(10 * 60_000);
    });

    it('새로 만들기는 짧게 기다린다', async () => {
        expect(await timeoutOf({ name: 'a', module_name: 'game' })).toBe(30000);
    });
});
