import { describe, expect, it } from 'vitest';
import i18n from '../i18n';
import { auditReason, auditResult, auditWhat } from '../utils/audit';

const t = i18n.getFixedT('en', 'gui');
const base = { source: 'discord', action: 'command', ok: true, status: 200 };

// #14: 작업 기록에 "save-on {}", "실패 (400)" 같은 날것이 보였다
describe('작업 기록 한 줄', () => {
    it('빈 인자는 숨기고, 값 하나는 따옴표로 묶는다', () => {
        expect(auditWhat({ ...base, command: 'save-on', args: '{}' }, t)).toBe('Command · save-on');
        expect(auditWhat({ ...base, command: 'save-on', args: '[]' }, t)).toBe('Command · save-on');
        expect(auditWhat({ ...base, command: 'say', args: '{"message":"hi"}' }, t)).toBe('Command · say “hi”');
        expect(auditWhat({ ...base, command: 'give', args: '{"player":"Steve","item":"diamond","count":64}' }, t)).toBe(
            'Command · give Steve diamond 64',
        );
        expect(auditWhat({ ...base, command: 'say', args: '["a b"]' }, t)).toBe('Command · say a b');
        expect(auditWhat({ ...base, action: 'restore' }, t)).toBe('Restore');
    });

    it('실패는 숫자 대신 번역한 이유를 보인다', () => {
        const failed = { ...base, ok: false, status: 400, code: 'invalid_argument' };
        expect(auditResult(failed, t)).toEqual({ tone: 'stop', label: 'Failed' });
        expect(auditReason(failed, t)).toBe("An input value wasn't valid");
        // 모듈 오류와 백업 오류도 그쪽 문구로
        expect(auditReason({ ...failed, code: 'module_timeout' }, t)).toBe(i18n.getFixedT('en', 'common')('module_errors.module_timeout'));
        expect(auditReason({ ...failed, code: 'backup_save_rejected' }, t)).toBe(t('maintenance.err_backup_save_rejected'));
        // 값이 빠지는 문구는 쓰지 않는다
        expect(auditReason({ ...failed, code: 'backup_file_locked' }, t)).toBe('');
        expect(auditReason({ ...failed, code: 'mystery_code' }, t)).toBe('');
        expect(auditReason({ ...base, code: 'invalid_argument' }, t)).toBe('');
    });
});
