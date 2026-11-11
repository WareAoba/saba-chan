import { describe, expect, it } from 'vitest';
import { areaToLch, lchToArea } from '../components/ui/OklchPicker';
import { inGamut, maxChroma, oklchToRgb } from '../utils/oklch';

// 색 고르는 판은 삼각형(범위 밖 빈칸) 대신 사각형을 꽉 채운다: 가로 = 그 밝기에서 낼 수 있는 채도의 비율 (사용자 11-11)
describe('OKLCH 판 = 꽉 찬 사각형', () => {
    it('판의 어느 점이든 화면에 낼 수 있는 색이고, 오른쪽 끝은 그 밝기의 최대 채도다', () => {
        for (const h of [0, 90, 200, 270]) {
            for (const y of [0.1, 0.3, 0.5, 0.7, 0.9]) {
                for (const x of [0, 0.25, 0.5, 1]) {
                    const v = areaToLch(x, y, h);
                    expect(inGamut(oklchToRgb(v.l, v.c, h)), `${h} ${x} ${y}`).toBe(true);
                }
                const right = areaToLch(1, y, h);
                expect(right.c).toBeCloseTo(maxChroma(right.l, h), 5);
            }
        }
    });

    it('색에서 판 위 자리로 되돌아간다', () => {
        const v = areaToLch(0.6, 0.4, 150);
        const p = lchToArea(v);
        expect(p.x).toBeCloseTo(0.6, 3);
        expect(p.y).toBeCloseTo(0.4, 3);
        // 채도를 낼 수 없는 밝기(검정·흰색)에서도 자리가 깨지지 않는다
        expect(lchToArea({ l: 1, c: 0, h: 0 })).toEqual({ x: 0, y: 0 });
    });
});
