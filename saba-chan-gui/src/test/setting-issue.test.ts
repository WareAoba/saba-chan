import { describe, expect, it } from 'vitest';
import i18n from '../i18n';
import { checkIssueText } from '../utils/settingIssue';

const t = i18n.getFixedT('en');

// 서버 점검: 데몬이 더한 설정 값 문제(setting_invalid)는 칸 이름과 오류 종류로 번역하고 고치는 곳을 알린다
describe('서버 점검의 설정 값 문제', () => {
    it('칸 이름·범위로 번역하고 해결 방법을 붙인다', () => {
        const text = checkIssueText(
            t,
            { code: 'setting_invalid', severity: 'error', field: 'players', error_type: 'out_of_range', value: 50 },
            'nomod',
            [{ name: 'players', label: 'Players', field_type: 'number', min: 1, max: 10 } as never],
        );
        expect(text.message).toBe(t('gui:validation.out_of_range', { field: 'Players', min: 1, max: 10, value: 50 }));
        expect(text.solution).toBe(t('gui:shell.check_setting_fix'));
    });

    it('모듈이 준 문제는 모듈 문구 그대로', () => {
        expect(checkIssueText(t, { code: 'JAVA', severity: 'critical', message: 'No Java', solution: 'Install' }, 'm')).toEqual({
            message: 'No Java',
            solution: 'Install',
        });
    });
});
