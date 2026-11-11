import { describe, expect, it } from 'vitest';
import { choseong, matchScore } from '../utils/hangul';

describe('한글 검색', () => {
    it('완성형 글자를 초성으로 바꾸고 나머지는 그대로 둔다', () => {
        expect(choseong('백업 만들기')).toBe('ㅂㅇ ㅁㄷㄱ');
        expect(choseong('Steve 3명')).toBe('Steve 3ㅁ');
    });

    it('검색어 조각이 모두 들어 있어야 맞고, 앞에서 맞을수록 점수가 높다', () => {
        expect(matchScore('백업 만들기', '')).toBe(1);
        expect(matchScore('백업 만들기', '백업')).toBeGreaterThan(matchScore('백업 만들기', '만들'));
        expect(matchScore('Restart server', 'serv RE')).toBeGreaterThan(0);
        expect(matchScore('백업 만들기', '백업 지우기')).toBe(0);
    });

    it('초성만 친 조각은 초성으로도 맞는다 (글자로 맞는 것보다 낮게)', () => {
        expect(matchScore('백업 만들기', 'ㅂㅇ')).toBeGreaterThan(0);
        expect(matchScore('백업 만들기', 'ㅂㅇ')).toBeLessThan(matchScore('백업 만들기', '만들'));
        expect(matchScore('정지', 'ㅂㅇ')).toBe(0);
    });
});
