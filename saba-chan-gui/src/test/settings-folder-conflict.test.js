/**
 * 서버 폴더를 다른 서버가 쓰는 폴더로 바꾸거나 작업 중에 바꾸면 데몬이 409로 거절한다.
 * 브리지는 그 이유(error_code)를 넘겨, 설정 화면이 "HTTP 409" 대신 까닭을 보이게 한다.
 */
import { describe, expect, it, vi } from 'vitest';
import { createApi } from '../api/createApi';

describe('instanceUpdateSettings 409', () => {
    it('working_dir_in_use와 그 서버 이름을 넘긴다', async () => {
        const data = {
            success: false,
            error: "Server 'other' already uses this folder",
            error_code: 'working_dir_in_use',
            instance: 'other',
        };
        const invoke = vi.fn().mockResolvedValue({ status: 409, data });
        const { api } = createApi({ invoke, hub: { dispatch: vi.fn() } });
        const result = await api.instanceUpdateSettings('a', { working_dir: 'C:/theirs' });
        expect(result.error).toBeTruthy();
        expect(result.error_code).toBe('working_dir_in_use');
        expect(result.instance).toBe('other');
    });

    it('instance_busy도 넘긴다', async () => {
        const data = { error: 'A backup, restore or update is running for this instance', error_code: 'instance_busy' };
        const invoke = vi.fn().mockResolvedValue({ status: 409, data });
        const { api } = createApi({ invoke, hub: { dispatch: vi.fn() } });
        const result = await api.instanceUpdateSettings('a', { working_dir: 'C:/elsewhere' });
        expect(result.error_code).toBe('instance_busy');
    });
});
