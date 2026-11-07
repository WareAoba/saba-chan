import { describe, expect, it } from 'vitest';
import { localLogLine } from '../utils/logTime';

// 데몬은 로그 줄 머리에 UTC 시각을 적는다: 화면은 이 PC 시간대로 바꿔 게임이 찍는 시각과 맞춘다
describe('지난 로그 시각', () => {
    it('줄 머리의 UTC 시각을 이 PC 시간대로 바꾸고 나머지는 그대로 둔다', () => {
        const at = new Date(Date.UTC(2026, 10, 7, 16, 40, 57));
        const two = (n: number) => String(n).padStart(2, '0');
        const want = `[${at.getFullYear()}-${two(at.getMonth() + 1)}-${two(at.getDate())} ${two(at.getHours())}:${two(at.getMinutes())}:${two(at.getSeconds())}]`;
        expect(localLogLine('[2026-11-07 16:40:57] [STDOUT/INFO] [01:40:57] Done')).toBe(`${want} [STDOUT/INFO] [01:40:57] Done`);
    });

    it('시각이 없는 줄(이어지는 줄)은 그대로', () => {
        expect(localLogLine('English counters. Error 0x80070057.')).toBe('English counters. Error 0x80070057.');
    });
});
