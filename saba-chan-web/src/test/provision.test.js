import { describe, expect, it } from 'vitest';
import { provisionView } from '../components/useProvision';

describe('설치 진행 보기', () => {
    it('설치 중이 아니면 아무것도 보이지 않는다', () => {
        expect(provisionView(null)).toBe(null);
        expect(provisionView({ active: false })).toBe(null);
    });

    it('끝나지 않았으면 진행률과 메시지', () => {
        expect(provisionView({ active: true, done: false, percent: 40, message: 'jar' })).toEqual({
            state: 'running',
            percent: 40,
            message: 'jar',
        });
    });

    it('실패로 끝나면 오류와, PC 에서 할 일이 있는지', () => {
        const v = provisionView({
            active: true,
            done: true,
            success: false,
            error: 'no',
            action_required: 'sandbox_unavailable',
        });
        expect(v).toEqual({ state: 'failed', error: 'no', pcNeeded: true });
    });

    it('성공으로 끝나면 done', () => {
        expect(provisionView({ active: true, done: true, success: true })).toEqual({ state: 'done' });
    });
});
