import { describe, expect, it } from 'vitest';
import i18n from '../i18n';
import { createTranslateError } from '../utils/helpers';

const translateError = createTranslateError(i18n.getFixedT('en', 'gui'));

describe('오류 문구 옮기기', () => {
    it('모르는 오류는 데몬이 보낸 문구를 그대로 보여 준다', () => {
        expect(translateError('Port 25565 is already in use')).toBe('Port 25565 is already in use');
    });

    it('시간 초과는 숨기지 않고 시간 초과라고 알린다', () => {
        for (const msg of [
            'ETIMEDOUT',
            'timeout of 30000ms exceeded',
            'error sending request for url (http://127.0.0.1:57474/api/servers): operation timed out',
        ]) {
            expect(translateError(msg)).toMatch(/timed out/i);
        }
    });

    it('비어 있으면 알 수 없는 오류로 쓴다', () => {
        expect(translateError('')).toMatch(/unknown/i);
    });
});
