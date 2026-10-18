import { describe, expect, it } from 'vitest';
import { hexToOklch, inGamut, maxChroma, oklchToHex, oklchToRgb } from '../utils/oklch';

describe('OKLCH 변환', () => {
    it('hex → oklch → hex가 같은 색으로 돌아온다', () => {
        for (const hex of ['#7d8ee8', '#3ecf8e', '#f2555a', '#0d0e10', '#ffffff']) {
            const { l, c, h } = hexToOklch(hex);
            expect(oklchToHex(l, c, h ?? 0)).toBe(hex);
        }
    });

    it('무채색은 색상이 없고, 흰색 밝기는 1이다', () => {
        const white = hexToOklch('#fff');
        expect(white.h).toBeNull();
        expect(white.l).toBeCloseTo(1, 3);
    });

    it('sRGB 밖의 채도는 범위 안으로 줄인다', () => {
        const c = maxChroma(0.7, 145);
        expect(inGamut(oklchToRgb(0.7, c, 145))).toBe(true);
        expect(inGamut(oklchToRgb(0.7, c + 0.02, 145))).toBe(false);
        expect(oklchToHex(0.7, 0.37, 145)).toBe(oklchToHex(0.7, c, 145));
    });

    it('잘못된 hex는 null', () => {
        expect(hexToOklch('nope')).toBeNull();
    });
});
