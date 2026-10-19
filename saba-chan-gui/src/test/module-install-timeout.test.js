/**
 * 서버 설치 요청은 데몬이 install_server에 허용하는 30분보다 먼저 끊지 않는다.
 * 먼저 끊으면 GUI는 실패를 보이는데 데몬은 설치를 계속한다.
 */
import { describe, expect, it, vi } from 'vitest';
import { createApi } from '../api/createApi';

describe('moduleInstallServer 시간 제한', () => {
    it('데몬의 설치 시간 제한(1800초)보다 길게 기다린다', async () => {
        const invoke = vi.fn().mockResolvedValue({ status: 200, data: { success: true } });
        const { api } = createApi({ invoke, hub: { dispatch: vi.fn() } });
        await api.moduleInstallServer('minecraft', { version: '1.21' });
        const args = invoke.mock.calls.find(([cmd]) => cmd === 'daemon_request')[1];
        expect(args.path).toBe('/api/module/minecraft/install');
        expect(args.timeoutMs).toBeGreaterThan(1800 * 1000);
    });
});
