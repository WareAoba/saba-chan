import { describe, expect, it } from 'vitest';
import { addController, memberChoices, removeController } from '../components/servers/controllers';

// 사용자 지정(2026-11-06): 인스턴스마다 "조작 가능" 명단. 한 줄은 게임 닉네임과 이을 수 있는 디스코드 멤버
describe('조작 가능 명단', () => {
    it('닉네임 앞뒤 공백을 떼고, 고른 디스코드 멤버를 잇는다', () => {
        expect(addController([], '  Steve ', '332007306403250178')).toEqual({
            list: [{ name: 'Steve', discord_id: '332007306403250178' }],
        });
        expect(addController([], 'Alex', '')).toEqual({ list: [{ name: 'Alex' }] });
    });

    it('빈 닉네임·대소문자만 다른 닉네임·64자 넘는 닉네임·200명 넘는 명단은 넣지 않는다 (데몬과 같은 규칙)', () => {
        const list = [{ name: 'Steve' }];
        expect(addController(list, '   ', '')).toEqual({ error: 'empty' });
        expect(addController(list, 'steve', '')).toEqual({ error: 'duplicate' });
        expect(addController(list, 'x'.repeat(65), '')).toEqual({ error: 'too_long' });
        const full = Array.from({ length: 200 }, (_, i) => ({ name: `p${i}` }));
        expect(addController(full, 'one_more', '')).toEqual({ error: 'too_many' });
    });

    it('빼면 그 줄만 빠진다', () => {
        expect(removeController([{ name: 'a' }, { name: 'b' }], 0)).toEqual([{ name: 'b' }]);
    });

    it('여러 디스코드 서버의 멤버를 한 번씩만, 이름 순으로 고를 수 있게 한다', () => {
        const choices = memberChoices({
            g1: [
                { id: '2', displayName: '마요' },
                { id: '1', username: 'aoba' },
            ],
            g2: [{ id: 2, displayName: '마요' }],
        });
        expect(choices.map((c) => [c.id, c.label])).toEqual([
            ['1', 'aoba'],
            ['2', '마요'],
        ]);
    });
});
