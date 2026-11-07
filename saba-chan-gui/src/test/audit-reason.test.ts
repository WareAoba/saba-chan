import { describe, expect, it } from 'vitest';
import i18n from '../i18n';
import { auditReason, auditResult, auditWhat, isInstanceEntry } from '../utils/audit';

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
        expect(auditReason({ ...failed, code: 'module_timeout' }, t)).toBe(
            i18n.getFixedT('en', 'common')('module_errors.module_timeout'),
        );
        expect(auditReason({ ...failed, code: 'backup_save_rejected' }, t)).toBe(
            t('maintenance.err_backup_save_rejected'),
        );
        // 값이 빠지는 문구는 쓰지 않는다
        expect(auditReason({ ...failed, code: 'backup_file_locked' }, t)).toBe('');
        expect(auditReason({ ...failed, code: 'mystery_code' }, t)).toBe('');
        expect(auditReason({ ...base, code: 'invalid_argument' }, t)).toBe('');
    });
    it('JSON 기호를 보이지 않는다: 겹친 값은 풀어 쓰고, 잘린 JSON은 기호를 걷는다', () => {
        expect(auditWhat({ ...base, command: 'tp', args: '{"to":{"x":1,"y":2},"who":["a","b"]}' }, t)).toBe(
            'Command · tp 1 2 a b',
        );
        expect(auditWhat({ ...base, command: 'say', args: '{"message":"hello wor' }, t)).toBe(
            'Command · say hello wor…',
        );
        expect(auditWhat({ ...base, command: 'say', args: 'plain words' }, t)).toBe('Command · say plain words');
    });

    it('데몬 액션 이름은 액션 표의 제목으로 보인다', () => {
        const bot = i18n.getFixedT('en', 'bot');
        expect(auditWhat({ ...base, action: 'instance.settings.set', args: '{"players":4}' }, t)).toBe(
            `${bot('actions.instance.settings.set.title')} · 4`,
        );
        expect(auditWhat({ ...base, action: 'mystery.thing' }, t)).toBe('mystery.thing');
    });

    it('서버를 조작한 줄과 원격 중계만 작업 기록에 보인다', () => {
        expect(isInstanceEntry({ ...base, instanceId: 'a' })).toBe(true);
        expect(isInstanceEntry({ ...base, action: 'remote', instanceId: '' })).toBe(true);
        expect(isInstanceEntry({ ...base, action: 'discord.settings.set', instanceId: '' })).toBe(false);
        expect(isInstanceEntry({ ...base, action: 'chatbot.settings.set' })).toBe(false);
    });
});
