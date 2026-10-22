import { describe, expect, it, vi } from 'vitest';
import { retryWithBackoff } from '../utils/helpers';

// 브리지는 데몬 실패를 던지지 않고 `{ error }`로 돌려준다: 그 답도 실패로 보고 다시 묻는다
describe('retryWithBackoff', () => {
    it('{ error } 답을 실패로 보고 다시 시도한다', async () => {
        const fn = vi
            .fn()
            .mockResolvedValueOnce({ ok: false, error: 'connect ECONNREFUSED' })
            .mockResolvedValue({ servers: [] });
        await expect(retryWithBackoff(fn, 3, 1)).resolves.toEqual({ servers: [] });
        expect(fn).toHaveBeenCalledTimes(2);
    });

    it('끝까지 실패하면 마지막 실패 답을 그대로 돌려준다', async () => {
        const fn = vi.fn().mockResolvedValue({ error: 'down' });
        await expect(retryWithBackoff(fn, 3, 1)).resolves.toEqual({ error: 'down' });
        expect(fn).toHaveBeenCalledTimes(3);
    });
});
