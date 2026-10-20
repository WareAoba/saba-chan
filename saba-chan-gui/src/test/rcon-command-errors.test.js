/**
 * L75: RCON으로만 명령을 받는 서버(데몬 #457)의 실패 코드를 사용자 문구로 보인다.
 * 데몬 영어 원문(error)을 그대로 띄우지 않는다.
 */
import { readFileSync } from 'node:fs';
import { resolve } from 'node:path';
import { describe, expect, it } from 'vitest';
import { commandErrorText } from '../utils/commandReply';

const t = (key) => `t:${key}`;
const CODES = ['rcon_unreachable', 'rcon_auth_failed', 'rcon_timeout', 'rcon_failed'];

describe('명령 실패 코드 문구', () => {
    it('RCON 실패 코드는 각자의 문구로 보인다', () => {
        for (const code of CODES) {
            expect(commandErrorText({ error: 'could not connect', error_code: code }, t)).toBe(`t:errors.${code}`);
        }
    });

    it('콘솔이 없는 서버와 입력이 밀린 서버는 예전 문구 그대로', () => {
        expect(commandErrorText({ error_code: 'no_command_path' }, t)).toBe('t:servers.no_command_path');
        expect(commandErrorText({ error_code: 'console_input_full' }, t)).toBe('t:errors.console_input_full');
    });

    it('모르는 코드는 맡지 않는다 (호출한 쪽이 원래 오류를 보인다)', () => {
        expect(commandErrorText({ error: 'boom', error_code: 'other' }, t)).toBeNull();
    });

    it('10개 언어 모두 RCON 문구가 있다', () => {
        for (const lang of ['ko', 'en', 'ja', 'zh-CN', 'zh-TW', 'de', 'es', 'fr', 'pt-BR', 'ru']) {
            const gui = JSON.parse(readFileSync(resolve(__dirname, `../../../locales/${lang}/gui.json`), 'utf8'));
            for (const code of CODES) expect(gui.errors[code], `${lang} ${code}`).toBeTruthy();
        }
    });
});
