/**
 * 로딩 화면 매트릭스는 데몬 로그에서 낱말을 뽑아 흘린다.
 * 사람들이 로딩 화면을 캡처해 올리므로 비밀값·경로·주소·사람 이름처럼 보이는 것은 빼고
 * 평범한 영어 낱말만 남긴다.
 */
import { describe, expect, it } from 'vitest';
import { logTokens } from '../components/shell/MatrixRain';

const line = (message) => ({ level: 'info', message });

describe('logTokens', () => {
    it('로그 문장에서 평범한 낱말만 뽑는다', () => {
        const tokens = logTokens([line('[Supervisor] starting minecraft server'), line('Loaded 3 modules')]);
        expect(tokens).toEqual(
            expect.arrayContaining(['Supervisor', 'starting', 'minecraft', 'server', 'Loaded', 'modules']),
        );
        expect(tokens).not.toContain('3');
    });

    it('비밀값·경로·주소·숫자 섞인 글자는 뺀다', () => {
        const tokens = logTokens([
            line('token=abcDEF123ghi456 key: deadbeefcafebabe0011 sk_live_AbCdEfGh'),
            line('opened C:\\Users\\jace5\\AppData\\saba and /home/aoba/.config'),
            line('listening on 127.0.0.1:57474 via https://saba-chan.online/api'),
            line('password hunter2 secret Zm9vYmFyYmF6cXV4'),
        ]);
        for (const bad of [
            'abcDEF123ghi456',
            'deadbeefcafebabe0011',
            'sk_live_AbCdEfGh',
            'jace5',
            'Users',
            'aoba',
            'config',
            '127',
            'online',
            'hunter2',
            'Zm9vYmFyYmF6cXV4',
        ]) {
            expect(tokens).not.toContain(bad);
        }
    });

    it('사람 이름이 들어가는 줄(감사 기록·디스코드 사용자)과 한글 낱말은 쓰지 않는다', () => {
        const tokens = logTokens([
            line('[Audit] discord aoba → minecraft command say'),
            line('[Relay] user WareAoba linked guild'),
            line('야생 서버 시작'),
            line('[Relay] heartbeat ok'),
        ]);
        expect(tokens).not.toContain('aoba');
        expect(tokens).not.toContain('WareAoba');
        expect(tokens).not.toContain('야생');
        expect(tokens).not.toContain('linked');
        expect(tokens).toEqual(['Relay', 'heartbeat', 'ok']);
    });

    it('같은 낱말은 한 번만, 비어 있으면 빈 배열', () => {
        expect(logTokens([line('tick tick tick')])).toEqual(['tick']);
        expect(logTokens(undefined)).toEqual([]);
    });
});
