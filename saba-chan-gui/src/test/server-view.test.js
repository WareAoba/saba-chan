import { describe, expect, it } from 'vitest';
import { playersText, reuseUnchanged, viewStatus } from '../utils/serverView';

describe('모듈 없음 표시', () => {
    const server = { id: 'a', module: 'minecraft', status: 'running' };

    it('모듈 목록을 받은 뒤에만 모듈 없음으로 본다', () => {
        expect(viewStatus(server, undefined, true)).toBe('missing');
    });

    it('목록을 받기 전에는 서버 상태를 그대로 보인다', () => {
        expect(viewStatus(server, undefined, false)).toBe('running');
    });
});

describe('폴링 결과 합치기', () => {
    const a = { name: 'a', status: 'running', port: 1 };
    const b = { name: 'b', status: 'stopped', port: 2 };

    it('바뀐 게 없으면 이전 배열을 그대로 돌려준다', () => {
        const prev = [a, b];
        expect(reuseUnchanged(prev, [{ ...a }, { ...b }])).toBe(prev);
    });

    it('바뀐 서버만 새 객체이고 나머지는 이전 객체다', () => {
        const prev = [a, b];
        const next = reuseUnchanged(prev, [{ ...a }, { ...b, status: 'running' }]);
        expect(next).not.toBe(prev);
        expect(next[0]).toBe(a);
        expect(next[1].status).toBe('running');
    });

    it('순서가 바뀌면 새 배열이다', () => {
        const prev = [a, b];
        const next = reuseUnchanged(prev, [{ ...b }, { ...a }]);
        expect(next).not.toBe(prev);
        expect(next).toEqual([b, a]);
    });
});

describe('접속자 수', () => {
    it('실행 중이고 모듈이 알려 줄 때만 online/max로 보인다', () => {
        expect(playersText({ status: 'running', players: { online: 3, max: 20 } })).toBe('3/20');
        expect(playersText({ status: 'running', players: { online: 3 } })).toBe('3');
        expect(playersText({ status: 'running', players: { online: 0, max: 8 } })).toBe('0/8');
    });

    it('키가 없거나 꺼져 있으면 아무것도 보이지 않는다 (모름을 0으로 보이지 않는다)', () => {
        expect(playersText({ status: 'running' })).toBeNull();
        expect(playersText({ status: 'stopped', players: { online: 2, max: 8 } })).toBeNull();
    });
});
