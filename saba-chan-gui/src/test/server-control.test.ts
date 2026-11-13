import { describe, expect, it } from 'vitest';
import { controlOutcome } from '../utils/serverControl';

describe('켜기·끄기 답 해석', () => {
    it('데몬이 고른 이유를 하나로 읽는다', () => {
        const agreements = [{ id: 'eula', url: 'https://example.com/eula' }];
        expect(controlOutcome({ success: false, error_code: 'agreement_required', agreements })).toEqual({ kind: 'agreements', agreements });
        expect(controlOutcome({ success: false, error_code: 'agreement_required' })).toEqual({ kind: 'agreements', agreements: [] });
        expect(controlOutcome({ success: false, error_code: 'restart_cancelled' })).toEqual({ kind: 'cancelled', code: 'restart_cancelled' });
        expect(controlOutcome({ success: false, error_code: 'still_stopping' })).toEqual({ kind: 'stillStopping' });
    });

    it('이미 켜져 있던 것은 실패가 아닐 때만이고, 나머지는 실패와 성공으로 나눈다', () => {
        expect(controlOutcome({ success: true, already_running: true })).toEqual({ kind: 'alreadyRunning' });
        expect(controlOutcome({ error: 'boom', already_running: true })).toEqual({ kind: 'failed' });
        expect(controlOutcome(null)).toEqual({ kind: 'failed' });
        expect(controlOutcome({ success: true })).toEqual({ kind: 'ok' });
    });
});
