import { describe, expect, it } from 'vitest';
import { DEFAULT_MONITOR_FLOAT, floatFromPoint, placeFloat } from '../components/monitor/floatPosition';

const area = { width: 1000, height: 600 };
const box = { width: 200, height: 100 };

describe('떠 있는 시스템 모니터 자리 (사용자 11-11: 아무 데나 놓되 모서리 가까이면 붙고, 밖으로 밀리면 돌아온다)', () => {
    it('기본은 오른쪽 위 12px', () => {
        expect(placeFloat(DEFAULT_MONITOR_FLOAT, area, box)).toEqual({ left: 788, top: 12 });
    });

    it('모서리 64px 안에 놓으면 그 모서리에 붙는다', () => {
        expect(floatFromPoint(40, 450, area, box)).toEqual({ corner: 'bottom-left', x: 12, y: 12 });
        expect(floatFromPoint(760, 50, area, box)).toEqual({ corner: 'top-right', x: 12, y: 12 });
    });

    it('모서리에서 먼 곳은 놓은 자리 그대로, 가까운 모서리에서의 거리로 기억한다', () => {
        const pos = floatFromPoint(300, 120, area, box);
        expect(pos).toEqual({ corner: 'top-left', x: 300, y: 120 });
        expect(placeFloat(pos, area, box)).toEqual({ left: 300, top: 120 });
        const right = floatFromPoint(600, 380, area, box);
        expect(right).toEqual({ corner: 'bottom-right', x: 200, y: 120 });
    });

    it('창이 줄어도 붙은 모서리를 따라가고, 밖으로 밀리면 안쪽 12px로 돌아온다', () => {
        const pos = { corner: 'bottom-right' as const, x: 200, y: 120 };
        expect(placeFloat(pos, { width: 500, height: 400 }, box)).toEqual({ left: 100, top: 180 });
        const far = { corner: 'top-left' as const, x: 900, y: 550 };
        expect(placeFloat(far, { width: 500, height: 400 }, box)).toEqual({ left: 288, top: 288 });
    });

    it('창이 모니터보다 좁으면 왼쪽 위 끝에 둔다', () => {
        expect(placeFloat(DEFAULT_MONITOR_FLOAT, { width: 150, height: 80 }, box)).toEqual({ left: 0, top: 0 });
    });
});
